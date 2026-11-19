// Ray-traced reflections of the main view (rt_shadows.rs `Reflect`): one
// mirror ray per glossy pixel (roughness up to 0.5, as ssr.wgsl), through
// the scene's and the posed faces' acceleration structures, nearest hit.
// A hit the view sees takes its lit colour from the scene copy; one it does
// not see is shaded here (`hit_view`), and the two are mixed where the view
// half sees it: at the screen's edges and where the depth there leaves the
// hit, so that no line runs between them. ssr.wgsl's `fs_ssr_rt` blends the
// result with SSR's weights; a miss (alpha 0) leaves the probes' reflection.
// The hit shaded here takes the point lights too (rt_light.wgsl `hit_light`:
// a shadow ray to each that counts, where the view traces their shadows),
// and where it is itself a mirror the ray goes on from it once (`hit_sent`,
// `BOUNCES`).
// Not yet: the projectors and the materials' maps past the base colour.

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl and wgpu_ray_query's
// enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection, with the jitter the depth prepass was drawn
    // with, and its inverse.
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    // xyz the eye.
    camera: vec4<f32>,
    lit: HitLight,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the rays' reach (m), y: 1 when the avatars' top level is traced,
    // z: the point lights (`lights`), w: 1 when their shadows are traced.
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
@group(0) @binding(7) var<storage, read> lights: array<Light>;

fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The scene copy's colour where the view sees `p`, a point of a surface
// facing `n` (rgb), and how surely it sees it (a): 0 off screen or behind
// something else, 1 on it, between the two over the screen's last twentieth
// and where the depth leaves `p`. The four pixels around `p` are weighed as
// a texture filter would, each by whether its depth is `p`'s: the eye's
// distance to within 5 cm and 1 %, more on a face seen at a slant, where
// a pixel's step along the face is as much deeper. The nearest of them
// alone makes the view sure of `p`: beside an outline, the pixels of the
// face in front leave the colour and take nothing from the sureness (the
// rim of a glossy face mirrors what is right behind it, and came in specks
// of the hit's own shading).
fn seen(p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let clip = params.clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let ndc = clip.xyz / clip.w;
    let edge = 1.0 - max(abs(ndc.x), abs(ndc.y));
    if (edge <= 0.0) {
        return vec4<f32>(0.0);
    }
    let size = vec2<i32>(params.screen.xy);
    let at = (ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy - vec2<f32>(0.5);
    let corner = vec2<i32>(floor(at));
    let f = at - floor(at);
    let to_p = p - params.camera.xyz;
    let dist = length(to_p);
    let facing = abs(dot(n, to_p / dist));
    // a pixel's width at `p`, and how much farther the face is a pixel on
    let across = distance(unproject(corner, ndc.z), unproject(corner + vec2<i32>(1, 0), ndc.z));
    let slope = across * sqrt(max(1.0 - facing * facing, 0.0)) / max(facing, 0.05);
    let near = 0.05 + 0.01 * dist + slope;
    var sum = vec4<f32>(0.0);
    var sure = 0.0;
    for (var i = 0u; i < 4u; i++) {
        let o = vec2<i32>(i32(i & 1u), i32(i >> 1u));
        let px = corner + o;
        if (any(px < vec2<i32>(0)) || any(px >= size)) {
            continue;
        }
        let d = textureLoad(depth_tex, px, 0);
        if (d <= 0.0) {
            continue;
        }
        let off = abs(distance(unproject(px, d), params.camera.xyz) - dist);
        let share = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        let same = 1.0 - smoothstep(near, 1.5 * near, off);
        sum += vec4<f32>(textureLoad(scene_tex, px, 0).rgb, 1.0) * (share * same);
        // the nearest pixel has a quarter of the filter at least
        sure = max(sure, same * min(4.0 * share, 1.0));
    }
    if (sum.a <= 1e-4) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sum.rgb / sum.a, sure * smoothstep(0.0, 0.1, edge));
}

// What a hit the view does not show sends back along the ray that met it
// (`dir`): its own light, and the share of what it mirrors, which the ray
// goes on to find along `reflect(dir, normal)`.
struct Sent {
    body: vec3<f32>,
    mirror: vec3<f32>,
};

// types.rs `DrawRecord`: `legacy` and `spec_color`; common.wgsl's flag
const RECORD_LEGACY: u32 = 12u;
const RECORD_SPEC_COLOR: u32 = 13u;
const FLAG_LEGACY_MAT: u32 = 256u;
// The hits of a ray: a mirror met off the screen sends it on once.
const BOUNCES: u32 = 2u;
// The share of a pixel's reflection under which what a hit mirrors is the
// sky's hemisphere, no ray.
const MIRROR_MIN: f32 = 0.02;

// A fullbright face sends its colour, unlit (object.wgsl `fs_main`); the
// others rt_light.wgsl's diffuse and what they mirror, by their material's
// factors without its maps. A PBR face: less a metal's diffuse, which has
// none, and the Fresnel share of common.wgsl `shade_pbr`, a mirror's as far
// as ssr.wgsl `reflection_weight` makes it one (to a roughness of 0.5), the
// rest of it the sky's hemisphere. A legacy material: object.wgsl
// `legacy_light`'s glossy environment and its environment (applyGlossEnv,
// applyLegacyEnv), which dims the face as it covers it; a face with neither
// mirrors nothing, as it is drawn.
fn hit_sent(s: Surface, hit: vec3<f32>, dir: vec3<f32>, lod: f32, offset: f32) -> Sent {
    let flags = record_vec(s.record, RECORD_FLAGS).x;
    let base = base_color(s.record, s.uv, lod).rgb;
    let glow = emissive_color(s.record);
    if ((flags & FLAG_FULLBRIGHT) != 0u) {
        return Sent(base + glow, vec3<f32>(0.0));
    }
    let lamps = Lamps(u32(params.mode.z), params.mode.w > 0.5, -1.0);
    let light = hit_light(params.lit, s, hit, offset, params.mode.x, params.mode.y > 0.5, lamps);
    let grazing = 1.0 - max(dot(s.normal, -dir), 0.0);
    if ((flags & FLAG_PBR) != 0u) {
        let factors = record_vecf(s.record, RECORD_PARAMS);
        let metallic = clamp(factors.y, 0.0, 1.0);
        let f0 = mix(vec3<f32>(0.04), base, metallic);
        let fresnel = f0 + (vec3<f32>(1.0) - f0) * pow(grazing, 5.0);
        let gloss = clamp(1.0 - 2.0 * factors.z, 0.0, 1.0);
        let sharp = gloss * gloss;
        let blurred = fresnel * (1.0 - sharp) * hemisphere(params.lit, reflect(dir, s.normal));
        return Sent(base * (1.0 - metallic) * light + blurred + glow, fresnel * sharp);
    }
    var lit = base * light;
    var mirror = vec3<f32>(0.0);
    if ((flags & FLAG_LEGACY_MAT) != 0u) {
        let legacy = record_vecf(s.record, RECORD_LEGACY);
        let env = clamp(legacy.w, 0.0, 1.0);
        if (legacy.z > 0.0) {
            let f = clamp(grazing, 0.3, 1.0);
            mirror = record_vecf(s.record, RECORD_SPEC_COLOR).rgb * (0.25 * f * f * legacy.z) * (vec3<f32>(1.0) - lit);
        }
        if (env > 0.0) {
            let f = min(grazing * grazing + env, 1.0);
            mirror = mirror * (1.0 - env) + vec3<f32>(env * env * f * 0.5);
            lit *= 1.0 - env;
        }
    }
    return Sent(lit + glow, mirror);
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
    let world = unproject(pixel, depth);
    let n = oct_decode(g.xy);
    let v = normalize(params.camera.xyz - world);
    let r = reflect(-v, n);
    let dist = distance(world, params.camera.xyz);
    // The ray, and what its light is worth to the pixel: all of it at
    // first, then what each hit mirrors of it. A hit the view sees ends it
    // with the scene copy's colour; one it does not is shaded here and, where
    // it is a mirror, sends the ray on: a chrome ball behind the camera
    // shows the room in a mirror, where it showed the sky's hemisphere.
    var origin = world + n * (0.01 + 0.002 * dist);
    var dir = r;
    var along = 0.0;
    var worth = vec3<f32>(1.0);
    var color = vec3<f32>(0.0);
    for (var bounce = 0u; bounce < BOUNCES; bounce++) {
        let ray = RayDesc(0u, 0xFFu, 0.0, params.mode.x, origin, dir);

        // the nearest hit of the two top levels: the scene's, then the posed
        // faces' up to it
        var t = -1.0;
        var record = 0u;
        var primitive = 0u;
        var bary = vec2<f32>(0.0);
        var rq: ray_query;
        rayQueryInitialize(&rq, scene_tlas, ray);
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
        if (params.mode.y > 0.5) {
            let reach = select(params.mode.x, t, t >= 0.0);
            var rp: ray_query;
            rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, 0xFFu, 0.0, reach, origin, dir));
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
            // the first ray's miss leaves the probes' reflection; a later
            // one's is the sky
            if (bounce == 0u) {
                textureStore(out_tex, pixel, vec4<f32>(0.0));
                return;
            }
            color += worth * hemisphere(params.lit, dir);
            break;
        }
        let hit = origin + dir * t;
        let s = surface_at(record, primitive, bary, dir);
        let on_screen = seen(hit, s.normal);
        color += worth * on_screen.rgb * on_screen.a;
        if (on_screen.a >= 0.999) {
            break;
        }
        along += t;
        // a coarser mip further along the ray, where a texel covers more
        let offset = 0.01 + 0.002 * (dist + along);
        let sent = hit_sent(s, hit, dir, log2(1.0 + along), offset);
        worth *= 1.0 - on_screen.a;
        color += worth * sent.body;
        worth *= sent.mirror;
        let onward = reflect(dir, s.normal);
        if (bounce + 1u == BOUNCES || max(max(abs(worth.r), abs(worth.g)), abs(worth.b)) < MIRROR_MIN) {
            color += worth * hemisphere(params.lit, onward);
            break;
        }
        origin = hit + s.normal * offset;
        dir = onward;
    }
    textureStore(out_tex, pixel, vec4<f32>(color, 1.0));
}
