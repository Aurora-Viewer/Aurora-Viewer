// Ray-traced reflections of the main view (rt_shadows.rs `Reflect`): one
// mirror ray per glossy pixel (roughness up to 0.5, as ssr.wgsl), through
// the scene's and the posed faces' acceleration structures, nearest hit.
// A hit the view sees takes its lit colour from the scene copy; one it does
// not see is shaded here: its base colour lit by the sun (a shadow ray) and
// the sky's hemisphere, plus its emission. ssr.wgsl's `fs_ssr_rt` blends the
// result with SSR's weights; a miss (alpha 0) leaves the probes' reflection.
// Not yet: the local lights and the materials' maps past the base colour.

// After rt_common.wgsl and wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection, with the jitter the depth prepass was drawn
    // with, and its inverse.
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // xyz the eye.
    camera: vec4<f32>,
    // xyz towards the sun or moon (unit), w 1 when it is up.
    light: vec4<f32>,
    // The frame's lights (common.wgsl `FrameU`): the sun's radiance and the
    // hemisphere's ambient.
    sun_color: vec4<f32>,
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    ground_color: vec4<f32>,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the rays' reach (m), y: 1 when the avatars' top level is traced.
    mode: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
// oct normal xy, roughness, metallic (the G-buffer ssr.wgsl reads)
@group(0) @binding(2) var gbuf_tex: texture_2d<f32>;
// the lit scene before its blended faces (the scene copy)
@group(0) @binding(3) var scene_tex: texture_2d<f32>;
@group(0) @binding(4) var scene_tlas: acceleration_structure;
@group(0) @binding(5) var posed_tlas: acceleration_structure;
@group(0) @binding(6) var out_tex: texture_storage_2d<rgba16float, write>;

const PI: f32 = 3.14159265;

fn unproject(pixel: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = pixel * params.screen.zw * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    let world = params.world_from_clip * vec4<f32>(ndc, depth, 1.0);
    return world.xyz / world.w;
}

fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The sky's ambient along `n` (common.wgsl `shade_pbr`'s hemisphere).
fn hemisphere(n: vec3<f32>) -> vec3<f32> {
    let up = n.z * 0.5 + 0.5;
    return mix(mix(params.ground_color.rgb, params.sky_horizon.rgb, clamp(up * 2.0, 0.0, 1.0)),
               params.sky_zenith.rgb, clamp(up * 2.0 - 1.0, 0.0, 1.0));
}

// The scene copy's colour where the view sees `p`, or alpha 0 where it
// does not (off screen, or something else in front).
fn seen(p: vec3<f32>) -> vec4<f32> {
    let clip = params.clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let ndc = clip.xyz / clip.w;
    if (any(abs(ndc.xy) >= vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let pixel = (ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy;
    let px = vec2<i32>(pixel);
    let d = textureLoad(depth_tex, px, 0);
    if (d <= 0.0) {
        return vec4<f32>(0.0);
    }
    let there = unproject(vec2<f32>(px) + vec2<f32>(0.5), d);
    let dist = distance(p, params.camera.xyz);
    if (distance(there, p) > 0.05 + 0.01 * dist) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(textureLoad(scene_tex, px, 0).rgb, 1.0);
}

@compute @workgroup_size(8, 8)
fn cs_reflect(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    let g = textureLoad(gbuf_tex, pixel, 0);
    // reversed depth: 0 is the sky; rough surfaces keep the probes'
    if (depth <= 0.0 || g.z > 0.5) {
        textureStore(out_tex, pixel, vec4<f32>(0.0));
        return;
    }
    let world = unproject(vec2<f32>(pixel) + vec2<f32>(0.5), depth);
    let n = oct_decode(g.xy);
    let v = normalize(params.camera.xyz - world);
    let r = reflect(-v, n);
    let dist = distance(world, params.camera.xyz);
    let origin = world + n * (0.01 + 0.002 * dist);
    let ray = RayDesc(0u, 0xFFu, 0.0, params.mode.x, origin, r);

    // the nearest hit of the two top levels: the scene's, then the posed
    // faces' up to it
    var t = -1.0;
    var record = 0u;
    var primitive = 0u;
    var bary = vec2<f32>(0.0);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, ray);
    while (rayQueryProceed(&rq)) {
        if (stops(rayQueryGetCandidateIntersection(&rq), false, r)) {
            rayQueryConfirmIntersection(&rq);
        }
    }
    let hs = rayQueryGetCommittedIntersection(&rq);
    if (hs.kind != RAY_QUERY_INTERSECTION_NONE) {
        t = hs.t;
        record = hit_record(false, hs.instance_custom_data, hs.geometry_index);
        primitive = hs.primitive_index;
        bary = hs.barycentrics;
    }
    if (params.mode.y > 0.5) {
        let reach = select(params.mode.x, t, t >= 0.0);
        var rp: ray_query;
        rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, 0xFFu, 0.0, reach, origin, r));
        while (rayQueryProceed(&rp)) {
            if (stops(rayQueryGetCandidateIntersection(&rp), true, r)) {
                rayQueryConfirmIntersection(&rp);
            }
        }
        let hp = rayQueryGetCommittedIntersection(&rp);
        if (hp.kind != RAY_QUERY_INTERSECTION_NONE) {
            t = hp.t;
            record = hit_record(true, hp.instance_custom_data, hp.geometry_index);
            primitive = hp.primitive_index;
            bary = hp.barycentrics;
        }
    }
    if (t < 0.0) {
        textureStore(out_tex, pixel, vec4<f32>(0.0));
        return;
    }
    let hit = origin + r * t;
    let on_screen = seen(hit);
    if (on_screen.a > 0.0) {
        textureStore(out_tex, pixel, on_screen);
        return;
    }

    let s = surface_at(record, primitive, bary, r);
    // a coarser mip further along the ray, where a texel covers more
    let base = base_color(record, s.uv, log2(1.0 + t)).rgb;
    var light = hemisphere(s.normal);
    let l = params.light.xyz;
    let nl = dot(s.normal, l);
    if (params.light.w > 0.5 && nl > 0.0) {
        let start = hit + s.normal * (0.01 + 0.002 * (dist + t));
        let shadow_ray = RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.0, params.mode.x, start, l);
        var sq: ray_query;
        rayQueryInitialize(&sq, scene_tlas, shadow_ray);
        while (rayQueryProceed(&sq)) {
            if (stops(rayQueryGetCandidateIntersection(&sq), false, l)) {
                rayQueryConfirmIntersection(&sq);
            }
        }
        var lit = rayQueryGetCommittedIntersection(&sq).kind == RAY_QUERY_INTERSECTION_NONE;
        if (lit && params.mode.y > 0.5) {
            var sp: ray_query;
            rayQueryInitialize(&sp, posed_tlas, shadow_ray);
            while (rayQueryProceed(&sp)) {
                if (stops(rayQueryGetCandidateIntersection(&sp), true, l)) {
                    rayQueryConfirmIntersection(&sp);
                }
            }
            lit = rayQueryGetCommittedIntersection(&sp).kind == RAY_QUERY_INTERSECTION_NONE;
        }
        if (lit) {
            // shade_pbr's diffuse: over pi for a PBR face, a third of the
            // sunlight for a legacy one
            let pbr = (record_vec(record, RECORD_FLAGS).x & FLAG_PBR) != 0u;
            light += params.sun_color.rgb * nl / select(3.0, PI, pbr);
        }
    }
    textureStore(out_tex, pixel, vec4<f32>(base * light + emissive_color(record), 1.0));
}
