// The rays of the traced global illumination's probes (gi.rs, gi/trace.rs),
// after rt_common.wgsl, rt_light.wgsl and gi.rs `bound_sample_source` (the
// volume's uniforms, its read and its bindings at group 3). One invocation
// per ray of the frame's update list: from the probe, nearest hit among the
// scene's and the posed faces (SOLID: the glass is not seen yet), and what
// it sends back towards the probe, shaded as the indirect light shades the
// hits its view does not see (rt_gi.wgsl, rt_light.wgsl `hit_light`): the
// sun and the lamps with their shadow rays, the face's emission, and for
// its ambient the volume itself as last updated, so that the light bounces
// once more at each update (the paper's infinite bounces, with a loss at
// each: GI_ENERGY), the sky's hemisphere where the volume has no light
// yet. A miss brings the sky's hemisphere along the ray (w GI_MISS); a hit
// on a face's back side (the probe is in a wall or a closed object) brings
// nothing, its w minus a fifth of its distance, so that the visibility
// test hides that probe from what lies beyond the wall and the relocation
// moves it out (gi_update.wgsl `cs_settle`). The update's fixed rays only
// tell where the faces are.

// The volume's light feeds itself back at each update: with a white face
// (an albedo of 1, common in the grid) nothing was lost at a bounce and a
// closed room's light grew without end, to white. A probe's hit keeps at
// most this albedo, and this share of the volume's light (RTXGI's energy
// conservation), so that the bounces converge: a closed room of the
// brightest faces settles at about 4 times its direct light.
const GI_ALBEDO_MAX: f32 = 0.9;
const GI_ENERGY: f32 = 0.85;
// A ray's light at most (luminance; rt_gi.wgsl's SAMPLE_MAX, its colour
// kept): a lamp right beside a hit would light a whole probe.
const GI_RAY_MAX: f32 = 8.0;

struct TraceU {
    lit: HitLight,
    // x: lamps, y: 1 when their shadows are traced, z: the rays' reach (m),
    // w: 1 when there are posed faces
    lamps: vec4<f32>,
    // x: the frame, for the lamps' draw
    frame: vec4<u32>,
}

@group(0) @binding(0) var<uniform> trace: TraceU;
@group(0) @binding(1) var scene_tlas: acceleration_structure;
@group(0) @binding(2) var posed_tlas: acceleration_structure;
@group(0) @binding(3) var<storage, read> lights: array<Light>;
@group(0) @binding(4) var<storage, read_write> gi_rays: array<vec4<f32>>;

// (pass 2 of the counts of AURORA_RT_GI_DIAG=count, rt_gi_counts.rs; gi.rs
// `PLAIN_HOOKS` count nothing)
@compute @workgroup_size(64)
fn cs_gi_trace(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    gi_count_start(2u, group);
    gi_trace_ray(id);
    gi_count_flush();
}

fn gi_trace_ray(id: vec3<u32>) {
    let k = id.x / GI_RAYS;
    let probe = gi_update_probe(k);
    if (probe.x == GI_NONE) {
        return;
    }
    let origin = gi_probe_pos(probe.x, probe.y);
    let dir = gi_ray_dir(id.x % GI_RAYS);
    let reach = trace.lamps.z;
    let posed = trace.lamps.w > 0.5;

    var t = -1.0;
    var record = 0u;
    var primitive = 0u;
    var bary = vec2<f32>(0.0);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, SOLID, 0.0, reach, origin, dir));
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    let hs = rayQueryGetCommittedIntersection(&rq);
    if (hs.kind != RAY_QUERY_INTERSECTION_NONE) {
        t = hs.t;
        let h = hit_of(false, hs.instance_custom_data, hs.geometry_index, hs.primitive_index);
        record = h.record;
        primitive = h.primitive;
        bary = hs.barycentrics;
    }
    if (posed) {
        var rp: ray_query;
        rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, SOLID, 0.0, select(reach, t, t >= 0.0), origin, dir));
        while (rayQueryProceed(&rp)) {
            if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
                rayQueryConfirmIntersection(&rp);
            }
        }
        let hp = rayQueryGetCommittedIntersection(&rp);
        if (hp.kind != RAY_QUERY_INTERSECTION_NONE) {
            t = hp.t;
            let h = hit_of(true, hp.instance_custom_data, hp.geometry_index, hp.primitive_index);
            record = h.record;
            primitive = h.primitive;
            bary = hp.barycentrics;
        }
    }
    if (t < 0.0) {
        gi_rays[id.x] = vec4<f32>(hemisphere(trace.lit, dir), GI_MISS);
        return;
    }
    let s = surface_at(record, primitive, bary, dir);
    if (s.back) {
        // (gi.rs BACK_SCALE: negative, never 0)
        gi_rays[id.x] = vec4<f32>(0.0, 0.0, 0.0, min(-GI_BACK_SCALE * t, -1e-6));
        return;
    }
    // the fixed rays tell where the faces are, not their light
    if (id.x % GI_RAYS < GI_FIXED_RAYS) {
        gi_rays[id.x] = vec4<f32>(0.0, 0.0, 0.0, t);
        return;
    }
    let hit = origin + dir * t;
    let lamps = Lamps(u32(trace.lamps.x), trace.lamps.y > 0.5, random2(vec2<u32>(id.x, 7u), trace.frame.x).x);
    let base = min(base_color(s.record, s.uv, 2.0 + log2(1.0 + t)).rgb, vec3<f32>(GI_ALBEDO_MAX));
    let g = gi_irradiance(hit, s.normal, -dir * t);
    let ambient = mix(hemisphere(trace.lit, s.normal), g.rgb * GI_ENERGY, g.a);
    let light = base * hit_light(trace.lit, s, hit, 0.01 + 0.002 * t, reach, posed, lamps, ambient) + emissive_color(s.record);
    let bounded = min(light, vec3<f32>(1.0e4));
    let capped = bounded * min(1.0, GI_RAY_MAX / max(dot(bounded, LUMA), 1e-6));
    gi_rays[id.x] = vec4<f32>(capped, t);
}
