// Ray-traced indirect light of the main view (rt_shadows.rs `Gi`): one
// bounce off the surfaces around, in place of GTAO's screen-space indirect
// light (frame binding 14, common.wgsl `screen_indirect`), in its units:
// irradiance over pi, which the shading multiplies by the diffuse colour.
// The sky's own light stays the hemisphere's and the probes'.
//
// `cs_gi`, at half resolution: one cosine-weighted ray per pixel and frame
// from the depth prepass (a pixel of each 2×2 in turn), nearest hit. A hit
// the view sees takes last frame's lit colour from the scene copy (so the
// local lights and the light already bounced come back too); one it does
// not see is shaded here (rt_light.wgsl `hit_radiance`); a miss brings
// nothing. The result is blended with last frame's, reprojected, until
// `HISTORY` frames; a pixel whose history is another surface starts again.
// Its guide keeps the luminance's second moment beside, for the denoiser
// (rt_denoise.wgsl), which then filters the half resolution.
// `cs_gi_filter`, at full resolution: the denoised half resolution brought
// up, through the nearest half resolution pixels at the same depth.

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl, rt_history.wgsl and
// wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection (with the prepass's jitter), its inverse, and
    // last frame's.
    clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    prev_clip_from_world: mat4x4<f32>,
    // xyz the eye, this frame's and last frame's.
    camera: vec4<f32>,
    prev_camera: vec4<f32>,
    lit: HitLight,
    // full resolution: width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // half resolution: width, height; z: the strength (Indirect light
    // intensity, as GTAO's)
    half: vec4<f32>,
    // x: the rays' reach (m), y: 1 when the avatars' top level is traced,
    // z: the frame's number, w: 1 when last frame's history is valid.
    mode: vec4<f32>,
};

// Frames the history averages at most; on a surface that moved on its own,
// whose light changes as it goes, fewer.
const HISTORY: f32 = 32.0;
const HISTORY_MOVING: f32 = 8.0;
// Rays a pixel traces on a frame with no history.
const RESET_RAYS: u32 = 16u;
// The brightest a sample counts (in the lit colour's units): a rare ray
// that meets a lamp would leave a bright speck for the history's length.
const SAMPLE_MAX: f32 = 8.0;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;

// cs_gi
@group(0) @binding(2) var prev_color: texture_2d<f32>;
// the guide (rt_history.wgsl `pack_normal`): z the distance, w the
// luminance's second moment
@group(0) @binding(3) var prev_guide: texture_2d<f32>;
@group(0) @binding(4) var scene_tlas: acceleration_structure;
@group(0) @binding(5) var posed_tlas: acceleration_structure;
@group(0) @binding(6) var out_color: texture_storage_2d<rgba16float, write>;
@group(0) @binding(7) var out_guide: texture_storage_2d<rgba16float, write>;
// last frame's lit scene (the scene copy, not yet made this frame)
@group(0) @binding(8) var scene_tex: texture_2d<f32>;
// the depth prepass's motion (rt_history.wgsl), or black without it
@group(0) @binding(12) var motion_tex: texture_2d<f32>;

// cs_gi_filter (its own bind group: it reads what the denoiser wrote)
@group(0) @binding(9) var gi_color: texture_2d<f32>;
@group(0) @binding(10) var gi_guide: texture_2d<f32>;
@group(0) @binding(11) var out_gi: texture_storage_2d<rgba16float, write>;

// A direction about `n`, more of them where the cosine is larger.
fn cosine_dir(n: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let r = sqrt(u.x);
    let phi = 2.0 * PI * u.y;
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3<f32>(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3<f32>(b, s + n.y * n.y * a, -n.y);
    return normalize(t * (r * cos(phi)) + bt * (r * sin(phi)) + n * sqrt(max(1.0 - u.x, 0.0)));
}

// Last frame's lit colour where the view sees `p` (the view's depth this
// frame finds it in front), or alpha 0.
fn seen_before(p: vec3<f32>) -> vec4<f32> {
    let clip = params.clip_from_world * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0);
    }
    let ndc = clip.xy / clip.w;
    if (any(abs(ndc) >= vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let px = vec2<i32>((ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5)) * params.screen.xy);
    let d = textureLoad(depth_tex, px, 0);
    if (d <= 0.0 || distance(unproject(px, d), p) > 0.05 + 0.01 * distance(p, params.camera.xyz)) {
        return vec4<f32>(0.0);
    }
    let before = prev_pixel(p);
    if (before.x < 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(textureLoad(scene_tex, vec2<i32>(before), 0).rgb, 1.0);
}

@compute @workgroup_size(8, 8)
fn cs_gi(@builtin(global_invocation_id) id: vec3<u32>) {
    let hp = vec2<i32>(id.xy);
    if (any(hp >= vec2<i32>(params.half.xy))) {
        return;
    }
    let frame = u32(params.mode.z);
    let sub = vec2<i32>(i32(frame & 1u), i32((frame >> 1u) & 1u));
    let size = vec2<i32>(params.screen.xy);
    let pixel = min(hp * 2 + sub, size - vec2<i32>(1));
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0) {
        textureStore(out_color, hp, vec4<f32>(0.0));
        textureStore(out_guide, hp, vec4<f32>(0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let n = normal_at(pixel, world, depth);
    let dist = distance(world, params.camera.xyz);
    let origin = ray_origin(world, n, dist);
    let posed = params.mode.y > 0.5;
    // with no history (a photo, a new size), more rays at once
    let rays = select(1u, RESET_RAYS, params.mode.w < 0.5);
    var sample = vec3<f32>(0.0);
    var moment = 0.0;
    for (var ray = 0u; ray < rays; ray++) {
        let dir = cosine_dir(n, random2(vec2<u32>(id.xy), frame * RESET_RAYS + ray));

        var t = -1.0;
        var record = 0u;
        var primitive = 0u;
        var bary = vec2<f32>(0.0);
        var rq: ray_query;
        rayQueryInitialize(&rq, scene_tlas, RayDesc(0u, 0xFFu, 0.0, params.mode.x, origin, dir));
        while (rayQueryProceed(&rq)) {
            if (stops(rayQueryGetCandidateIntersection(&rq), false, dir)) {
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
        if (posed) {
            let reach = select(params.mode.x, t, t >= 0.0);
            var rp: ray_query;
            rayQueryInitialize(&rp, posed_tlas, RayDesc(0u, 0xFFu, 0.0, reach, origin, dir));
            while (rayQueryProceed(&rp)) {
                if (stops(rayQueryGetCandidateIntersection(&rp), true, dir)) {
                    rayQueryConfirmIntersection(&rp);
                }
            }
            let hp2 = rayQueryGetCommittedIntersection(&rp);
            if (hp2.kind != RAY_QUERY_INTERSECTION_NONE) {
                t = hp2.t;
                record = hit_record(true, hp2.instance_custom_data, hp2.geometry_index);
                primitive = hp2.primitive_index;
                bary = hp2.barycentrics;
            }
        }
        var one = vec3<f32>(0.0);
        if (t >= 0.0) {
            let hit = origin + dir * t;
            let before = seen_before(hit);
            if (before.a > 0.0) {
                one = before.rgb;
            } else {
                let s = surface_at(record, primitive, bary, dir);
                one = hit_radiance(params.lit, s, hit, 2.0 + log2(1.0 + t), 0.01 + 0.002 * (dist + t), params.mode.x, posed);
            }
        }
        let kept = min(one, vec3<f32>(SAMPLE_MAX));
        sample += kept;
        moment += luma(kept) * luma(kept);
    }
    sample = sample / f32(rays);
    moment = moment / f32(rays);

    // last frame's at the same surface
    var color = sample;
    var count = min(f32(rays), HISTORY);
    let back = back_of(pixel, world, dist);
    if (params.mode.w > 0.5 && back.at.x >= 0.0) {
        let ph = clamp(vec2<i32>(back.at * 0.5), vec2<i32>(0), vec2<i32>(params.half.xy) - vec2<i32>(1));
        let was = textureLoad(prev_guide, ph, 0);
        if (was.z > 0.0 && abs(was.z - back.expected) < back.tolerance) {
            let h = textureLoad(prev_color, ph, 0);
            count = min(h.a + 1.0, select(HISTORY, HISTORY_MOVING, back.moving));
            color = mix(h.rgb, sample, 1.0 / count);
            moment = mix(was.w, moment, 1.0 / count);
        }
    }
    textureStore(out_color, hp, vec4<f32>(color, count));
    textureStore(out_guide, hp, vec4<f32>(pack_normal(n), dist, moment));
}

@compute @workgroup_size(8, 8)
fn cs_gi_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    if (depth <= 0.0) {
        textureStore(out_gi, pixel, vec4<f32>(0.0));
        return;
    }
    let dist = distance(unproject(pixel, depth), params.camera.xyz);
    let centre = (vec2<f32>(pixel) + vec2<f32>(0.5)) * 0.5 - vec2<f32>(0.5);
    let base = vec2<i32>(floor(centre));
    let last = vec2<i32>(params.half.xy) - vec2<i32>(1);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let q = clamp(base + vec2<i32>(x, y), vec2<i32>(0), last);
            let qd = textureLoad(gi_guide, q, 0).z;
            if (qd <= 0.0) {
                continue;
            }
            let c = textureLoad(gi_color, q, 0);
            let off = vec2<f32>(q) - centre;
            // nearer and of the same depth weigh more
            let w_space = exp(-dot(off, off));
            let w_depth = exp(-abs(qd - dist) / (0.01 + 0.02 * dist));
            let w = w_space * w_depth;
            sum += c.rgb * w;
            weight += w;
        }
    }
    let gi = select(vec3<f32>(0.0), sum / max(weight, 1e-6), weight > 1e-4);
    textureStore(out_gi, pixel, vec4<f32>(gi * params.half.z, 1.0));
}
