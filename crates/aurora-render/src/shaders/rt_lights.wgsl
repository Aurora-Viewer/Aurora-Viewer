// Ray-traced shadows of the local point lights (rt_shadows.rs `Lights`):
//
// `cs_lights`, a pixel of the depth prepass at a time: the light it traces,
// by resampled importance sampling (Talbot 2005) over the point lights that
// reach it, each weighed by the diffuse light it would give unshadowed
// (every light once when no more than `CANDIDATES` reach it), then with
// ReSTIR (Bitterli et al. 2020) merged with the pixel's choice last frame;
// then one shadow ray toward it. The light is drawn in proportion to its
// unshadowed light, so its visibility averages to the share of the local
// light that arrives (the ratio estimator, Heitz et al. 2018): that share,
// blended with last frame's reprojected as rt_gi.wgsl's history, is what
// the shading multiplies its point lights by (common.wgsl `local_share`),
// whose light stays the Legacy Viewer's where nothing is in the way.
// `cs_lights_filter`: a depth-aware 5×5 gather of the share, written with
// the sun's mask (rt_shadows.wgsl) into the mask the frame binds, `.b`.
//
// A light's rays stop short of its own prim (`source.x`), so its bulb or its
// lamp's body does not shadow it. Projectors keep their shadow maps.

// After rt_common.wgsl, rt_view.wgsl, rt_light.wgsl, rt_history.wgsl and
// wgpu_ray_query's enable (rt_shadows.rs `source`).

struct Params {
    // This frame's projection inverted (with the prepass's jitter), and
    // last frame's projection.
    world_from_clip: mat4x4<f32>,
    prev_clip_from_world: mat4x4<f32>,
    // xyz the eye, this frame's and last frame's.
    camera: vec4<f32>,
    prev_camera: vec4<f32>,
    // width, height, 1 / width, 1 / height
    screen: vec4<f32>,
    // x: the lights, y: 1 when the avatars' top level is traced, z: the
    // frame's number, w: 1 when last frame's history is valid, 3 when its
    // choices are of the same lights too.
    mode: vec4<f32>,
};

// A light of the frame's list (the frame uniform's `lights`, in order).
struct Light {
    pos_radius: vec4<f32>,
    color_falloff: vec4<f32>,
    // x: the radius of the prim it shines from, y: 1 for a projector.
    source: vec4<f32>,
};

// Candidates a pixel draws among the lights reaching it.
const CANDIDATES: u32 = 8u;
// The most a choice kept from last frame counts for, against this frame's.
const CONFIDENCE_MAX: f32 = 20.0;
// Frames the share's history averages at most; on a surface that moved on
// its own, whose shadows sweep over it, fewer.
const HISTORY: f32 = 12.0;
const HISTORY_MOVING: f32 = 4.0;
const MAX_LIGHTS: u32 = 64u;
// Rays a pixel traces on a frame with no history.
const RESET_RAYS: u32 = 8u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var<storage, read> lights: array<Light>;

// cs_lights
// A choice: the light's index, its weight, its confidence.
@group(0) @binding(3) var prev_choice: texture_2d<f32>;
@group(0) @binding(4) var out_choice: texture_storage_2d<rgba32float, write>;
@group(0) @binding(5) var scene_tlas: acceleration_structure;
@group(0) @binding(6) var posed_tlas: acceleration_structure;
// The share: r the share, g the frames it averages, b the pixel's distance.
@group(0) @binding(7) var prev_share: texture_2d<f32>;
@group(0) @binding(8) var out_share: texture_storage_2d<rgba16float, write>;
// the depth prepass's motion (rt_history.wgsl), or black without it
@group(0) @binding(12) var motion_tex: texture_2d<f32>;

// cs_lights_filter (its own bind group: it reads what cs_lights wrote)
@group(0) @binding(9) var share_tex: texture_2d<f32>;
@group(0) @binding(10) var sun_tex: texture_2d<f32>;
@group(0) @binding(11) var out_mask: texture_storage_2d<rgba8unorm, write>;

// common.wgsl `ll_light_atten`.
fn light_atten(d: f32, falloff: f32) -> f32 {
    let f = max(falloff, 0.0);
    let x = 1.0 - clamp((d + f) / (1.0 + f), 0.0, 1.0);
    return x * x * 2.0;
}

// The diffuse light point light `i` gives `world` facing `n`, unshadowed
// (its luminance): what a choice is weighed by. 0 for a projector.
fn light_target(i: u32, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let lt = lights[i];
    if (lt.source.y > 0.5) {
        return 0.0;
    }
    let to_l = lt.pos_radius.xyz - world;
    let dist = length(to_l);
    let radius = lt.pos_radius.w;
    if (dist >= radius) {
        return 0.0;
    }
    let nl = dot(n, to_l / max(dist, 1e-4));
    if (nl <= 0.0) {
        return 0.0;
    }
    let lum = dot(lt.color_falloff.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return lum * light_atten(dist / radius, lt.color_falloff.w) * nl;
}

fn rand(seed: ptr<function, u32>) -> f32 {
    *seed = pcg(*seed);
    return f32(*seed >> 8u) / 16777216.0;
}

@compute @workgroup_size(8, 8)
fn cs_lights(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let depth = textureLoad(depth_tex, pixel, 0);
    // reversed depth: 0 is the sky
    if (depth <= 0.0) {
        textureStore(out_choice, pixel, vec4<f32>(0.0));
        textureStore(out_share, pixel, vec4<f32>(1.0, 0.0, 0.0, 0.0));
        return;
    }
    let world = unproject(pixel, depth);
    let n = normal_at(pixel, world, depth);
    let dist = distance(world, params.camera.xyz);
    var seed = pcg(id.x + pcg(id.y + pcg(u32(params.mode.z))));

    // the lights reaching the pixel, and their unshadowed light in all
    var count = 0u;
    var total = 0.0;
    let listed = min(u32(params.mode.x), MAX_LIGHTS);
    for (var i = 0u; i < listed; i++) {
        let p = light_target(i, world, n);
        if (p > 0.0) {
            count += 1u;
            total += p;
        }
    }

    // last frame's at the same surface
    let came = back_of(pixel, world, dist);
    var back = vec2<i32>(-1);
    if (params.mode.w > 0.5 && came.at.x >= 0.0) {
        let b = clamp(vec2<i32>(came.at), vec2<i32>(0), vec2<i32>(params.screen.xy) - vec2<i32>(1));
        let was = textureLoad(prev_share, b, 0).b;
        if (was > 0.0 && abs(was - came.expected) < came.tolerance) {
            back = b;
        }
    }
    // with no history (a photo, a new size), more rays at once
    let rays = select(1u, RESET_RAYS, params.mode.w < 0.5);
    let origin = ray_origin(world, n, dist);
    var seen_sum = 0.0;
    var chosen = 0u;
    var w = 0.0;
    var confidence = 0.0;
    for (var ray = 0u; ray < rays; ray++) {
        // this frame's candidates (Solari's `initial_and_temporal`): every
        // light reaching the pixel once when they are few, else drawn
        // evenly among them
        var chosen_target = 0.0;
        var sum = 0.0;
        confidence = 0.0;
        if (count > 0u) {
            if (count <= CANDIDATES) {
                for (var i = 0u; i < listed; i++) {
                    let p = light_target(i, world, n);
                    sum += p;
                    if (p > 0.0 && rand(&seed) * sum < p) {
                        chosen = i;
                        chosen_target = p;
                    }
                }
            } else {
                for (var k = 0u; k < CANDIDATES; k++) {
                    let entry = min(u32(rand(&seed) * f32(count)), count - 1u);
                    // the entry-th light reaching the pixel
                    var seen_so_far = 0u;
                    for (var i = 0u; i < listed; i++) {
                        let p = light_target(i, world, n);
                        if (p <= 0.0) {
                            continue;
                        }
                        if (seen_so_far == entry) {
                            let weight = p * f32(count) / f32(CANDIDATES);
                            sum += weight;
                            if (rand(&seed) * sum < weight) {
                                chosen = i;
                                chosen_target = p;
                            }
                            break;
                        }
                        seen_so_far += 1u;
                    }
                }
            }
            confidence = 1.0;
        }
        // last frame's choice, of the same lights (ReSTIR's temporal reuse)
        if (params.mode.w > 2.5 && back.x >= 0) {
            let prev = textureLoad(prev_choice, back, 0);
            let row = u32(prev.x);
            if (prev.z > 0.0 && row < listed) {
                let conf = min(prev.z, CONFIDENCE_MAX);
                let p = light_target(row, world, n);
                let weight = p * prev.y * conf;
                sum += weight;
                confidence += conf;
                if (weight > 0.0 && rand(&seed) * sum < weight) {
                    chosen = row;
                    chosen_target = p;
                }
            }
        }
        w = 0.0;
        if (chosen_target > 0.0 && confidence > 0.0) {
            w = sum / (confidence * chosen_target);
        }
        // the chosen light's visibility, as the share of the light arriving
        var sample = 1.0;
        if (w > 0.0 && total > 0.0) {
            let lt = lights[chosen];
            let toward = lt.pos_radius.xyz - origin;
            let span = length(toward);
            let reach = span - max(lt.source.x, min(0.05, span * 0.5));
            var seen = 1.0;
            if (reach > 0.0 && !unblocked(origin, toward / max(span, 1e-6), reach, params.mode.y > 0.5)) {
                seen = 0.0;
            }
            sample = clamp(chosen_target * w * seen / total, 0.0, 1.0);
        }
        seen_sum += sample;
    }
    textureStore(out_choice, pixel, vec4<f32>(f32(chosen), w, confidence, 0.0));
    let sample = seen_sum / f32(rays);

    var share = sample;
    var frames = min(f32(rays), HISTORY);
    if (back.x >= 0) {
        let h = textureLoad(prev_share, back, 0);
        frames = min(h.g + 1.0, select(HISTORY, HISTORY_MOVING, came.moving));
        share = mix(h.r, sample, 1.0 / frames);
    }
    textureStore(out_share, pixel, vec4<f32>(share, frames, dist, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_lights_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if (any(pixel >= vec2<i32>(params.screen.xy))) {
        return;
    }
    let sun = textureLoad(sun_tex, pixel, 0);
    let here = textureLoad(share_tex, pixel, 0);
    if (here.b <= 0.0) {
        textureStore(out_mask, pixel, vec4<f32>(sun.rg, 1.0, sun.a));
        return;
    }
    let last = vec2<i32>(params.screen.xy) - vec2<i32>(1);
    var sum = 0.0;
    var weight = 0.0;
    for (var y = -2; y <= 2; y++) {
        for (var x = -2; x <= 2; x++) {
            let q = clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), last);
            let s = textureLoad(share_tex, q, 0);
            if (s.b <= 0.0) {
                continue;
            }
            let off = vec2<f32>(f32(x), f32(y));
            let w = exp(-dot(off, off) * 0.25) * exp(-abs(s.b - here.b) / (0.01 + 0.02 * here.b)) * s.g;
            sum += s.r * w;
            weight += w;
        }
    }
    let share = select(here.r, sum / max(weight, 1e-6), weight > 1e-4);
    textureStore(out_mask, pixel, vec4<f32>(sun.rg, share, sun.a));
}
