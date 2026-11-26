// Screen-space contact shadows of the sun (or the moon), from the
// single-sample depth prepass: a WGSL port of Bend Studio's screen space
// shadows (bend_sss_gpu.h, as shipped in Days Gone), run in the dispatches
// that contact_shadows.rs lays out. The result is the shadow in `.r` (0 lit,
// 1 in contact shadow) that the shading
// combines with the sun's shadow map (`contact_shadow` in common.wgsl).
// Every pixel of the screen is written each frame without emptying the
// target first: a wavefront writes its sky pixels lit, and `clear_light`
// the one pixel no wavefront writes, under the light's point.
//
// Original notice:
//
// Copyright 2023 Sony Interactive Entertainment.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// How it works: each workgroup is one "wavefront" of WAVE_SIZE pixels lined
// up along a ray towards the light's point on screen. Its threads read the
// depth of their pixel and of the pixels up to SAMPLE_COUNT further towards
// the light, divide them by their distance to the light point (in that
// space every ray of light is parallel), share them in workgroup memory,
// and a pixel is in shadow when one of the depths before it along the ray
// comes within a surface thickness of its own. Changes from the original:
// integer texel loads instead of a clamp-to-border sampler (outside the
// screen reads the far depth), the early-out of a whole wavefront through
// workgroup memory (no wave intrinsics in WGSL), the shadow stored rather
// than the light, no debug outputs, occluders taken as deep as the ray went
// under them (up to `tuning.w`) rather than a shell (`depth_delta`), more
// samples in a photo larger than the viewfinder (the four constants below
// replaced in a pipeline of its own, `shader_source` in contact_shadows.rs:
// the shadow keeps its length in the image), Bend's defaults otherwise
// (BilinearSamplingOffsetMode and IgnoreEdgePixels off, no precision
// offset).

struct Params {
    // x, y: the light on screen (pixels, y down), z: its depth, w: 1 when
    // the light is in front of the camera, -1 behind (LightCoordinate)
    light: vec4<f32>,
    // xy: this dispatch's first wave, relative to the light (WaveOffset)
    wave_offset: vec4<i32>,
    // x surface thickness, y bilinear threshold, z shadow contrast, w the
    // occluders' depth in units of surface thickness per unit of reversed
    // depth (0: Bend's shell of one unit)
    tuning: vec4<f32>,
    // x: projected contact search pixels per reversed-Z depth (0: original
    // fixed sample count). y enables candidate metadata and z classifies
    // failed candidates. w is a projected pixel width in baseline shell units.
    reach: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba8unorm, write>;

// Must match WAVE_SIZE in contact_shadows.rs.
const WAVE_SIZE: i32 = 64;
// Shadow samples per pixel: the length of the shadow in pixels. This line
// and the next three are rewritten for a photo (`shader_source`): keep them
// as they are.
const SAMPLE_COUNT: i32 = 60;
// The first samples: one of them alone can shadow the pixel.
const HARD_SHADOW_SAMPLES: i32 = 4;
// The last samples fade out, softening the end of the shadow.
const FADE_OUT_SAMPLES: i32 = 8;
// Depth reads per thread: SAMPLE_COUNT / WAVE_SIZE + 2.
const READ_COUNT: i32 = 2;
// Reversed depth: the far plane at 0, the near plane at 1.
const FAR_DEPTH: f32 = 0.0;
const NEAR_DEPTH: f32 = 1.0;

var<workgroup> depth_data: array<f32, READ_COUNT * WAVE_SIZE>;
// Several live invocations set this concurrently. Atomic writes avoid a
// workgroup data race before the uniform load's barrier (zero at the start).
var<workgroup> any_live: atomic<u32>;

// A pixel lit, when it is on the target.
fn store_lit(p: vec2<i32>) {
    let size = vec2<i32>(textureDimensions(out_tex));
    if (all(p >= vec2<i32>(0)) && all(p < size)) {
        textureStore(out_tex, p, vec4<f32>(0.0));
    }
}

fn load_depth(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return FAR_DEPTH;
    }
    return textureLoad(depth_tex, p, 0);
}

// Reversed-Z depth is affine across a projected triangle. Estimate its
// two screen-space derivatives once at the receiver. Opposing neighbours
// must agree on both axes so sky, silhouettes and creases keep their
// original contact test.
struct ReceiverPlane {
    origin: f32,
    major_slope: f32,
    minor_slope: f32,
    minor_phase: f32,
    minor_step: f32,
    enabled: bool,
};

fn front_receiver_plane(p: vec2<i32>, d: f32, step_px: vec2<f32>, x_major: bool,
                        light_slope: f32, origin_xy: vec2<f32>) -> ReceiverPlane {
    let disabled = ReceiverPlane(d, 0.0, 0.0, 0.0, 0.0, false);
    let neighbours = vec4<f32>(
        load_depth(p - vec2<i32>(1, 0)), load_depth(p - vec2<i32>(0, 1)),
        load_depth(p + vec2<i32>(1, 0)), load_depth(p + vec2<i32>(0, 1)));
    if (any(neighbours <= vec4<f32>(FAR_DEPTH))) { return disabled; }
    let back = vec2<f32>(d) - neighbours.xy;
    let forward = neighbours.zw - vec2<f32>(d);
    let gradient = (back + forward) * 0.5;
    if (any(abs(forward - back) > 0.125 * abs(gradient) + 1e-5 * d)) { return disabled; }
    let slope = dot(gradient, step_px);
    // At positive incidence the light ray leaves this plane on its near
    // side. A ray entering the solid must still self-shadow it.
    if (slope >= light_slope) { return disabled; }
    let major_slope = select(gradient.y * step_px.y, gradient.x * step_px.x, x_major);
    let minor_slope = select(gradient.x, gradient.y, x_major);
    let minor_phase = fract(select(origin_xy.x, origin_xy.y, x_major));
    let minor_step = select(step_px.x, step_px.y, x_major);
    return ReceiverPlane(d, major_slope, minor_slope, minor_phase, minor_step, true);
}

// The pixel's side of the comparison: its own depth along the ray, made
// perspective-correct and scaled to units of surface thickness.
struct ShadowWindow {
    start: f32,
    scale: f32,
    z_sign: f32,
    // the pixel's distance to the light point, and -1 or 1 per sample
    // along the ray (sample_distance[0], direction)
    distance: f32,
    direction: f32,
    // the deepest an occluder is taken to be, in units (1: Bend's shell)
    depth: f32,
    adaptive: bool,
    pixel_width: f32,
    shell_width: f32,
    receiver_plane: ReceiverPlane,
};

// How far the sample `k` pixels along the ray is from shadowing the pixel:
// 0.75 or less is full shadow (with the contrast of 4), 1 or more lit.
// Bend takes |v - 1|, v being how far the sample's surface lies in front of
// the ray, in units: only a shell from 0.25 to 1.75 units in front shadows.
// A ray that goes under a hoof or a shoe and through its body, behind the
// visible faces, was then stopped only where it grazed their edges: the
// ground took the outline of the caster instead of its shadow (issue 988).
// The window now reaches back to the pixel's own depth when the ray recedes
// from the camera (a sun ahead), up to `depth`: a surface between the pixel
// and the ray is a body the ray went under. One nearer than the pixel keeps
// the shell, or each caster would darken the ground just behind it with a
// copy of its silhouette.
// Return the v2.3.4 distance, the current distance, and the distance for a
// resolved front surface (1 when absent). Smaller values are better hits. The history stores
// the prior signed ray gap, prior camera depth and the number of continuous
// steps since the visible surface crossed the light ray.
fn depth_deltas(w: ShadowWindow, sample_depth: f32, k: i32, history: ptr<function, vec3<f32>>) -> vec3<f32> {
    let v = (sample_depth * w.scale - w.start) * -w.z_sign;
    // the pixel's depth seen from the sample, in front of the ray by
    // start × (distance / distance_k - 1)
    let distance_k = max(w.distance + f32(k) * w.direction, 1.0);
    let receding = w.start * f32(k) * w.direction * w.z_sign / distance_k;
    // Preserve the exact Aurora/Bend comparison when disabled. On, boost
    // only receding intersections after a few search steps; limit to 35%
    // over the user-selected occluder depth. This is a depth estimate,
    // not missing-geometry reconstruction.
    var allowed_depth = w.depth;
    if (w.adaptive && w.depth > 1.01 && receding > 1.0) {
        let gradual = clamp(f32(k) / 18.0, 0.0, 1.0);
        allowed_depth *= 1.0 + gradual * 0.35;
    }
    let full_slab = clamp(receding, 1.0, allowed_depth);
    let original = max(1.0 - v, v - full_slab);
    // Undo the perspective normalization for a camera-depth continuity test.
    // A depth discontinuity resets the surface history at a silhouette.
    let surface = sample_depth * distance_k + params.light.z;
    let prev = *history;
    let continuous = prev.y > 0.0 && surface > 0.0
        && abs(surface - prev.y) <= params.tuning.y * min(surface, prev.y);
    let crossed = continuous && (prev.x <= 0.0) != (v <= 0.0);
    let since_crossing = select(1e4, select(prev.z + 1.0, 0.0, crossed), continuous);
    *history = vec3<f32>(v, surface, since_crossing);
    let plane = w.receiver_plane;
    if (plane.enabled && (original < 1.3 || (receding < 0.0 && v > -1.0 && v < 2.3))) {
        // Other wave lanes sampled the integer texel crossed at k, then
        // subtracted the minor-axis depth difference unless they detected
        // an edge (use_point_filter below). Predict that same stored depth.
        let minor_pixels = floor(plane.minor_phase + f32(k) * plane.minor_step);
        let point = plane.origin + f32(k) * plane.major_slope + minor_pixels * plane.minor_slope;
        let minor = abs(plane.minor_slope);
        let expected = select(point - minor, point, minor > point * params.tuning.y);
        // The light ray leaves the tangent toward the camera. A surface on
        // or behind that tangent cannot intercept it, including a convex
        // face curving away. Allow f32 slope roundoff to grow with distance.
        let tolerance = 4e-7 * plane.origin * f32(k + 1);
        if (surface <= expected + tolerance) {
            return vec3<f32>(original, 1.0, 1.0);
        }
    }
    var corrected = original;
    // Update history for every tap, including the hard prefix and distant
    // misses, but only evaluate the interior correction within its window.
    if (k > HARD_SHADOW_SAMPLES && original < 1.3) {
        // Keep the interior transition and crossing evidence proportional to the
        // photo pipeline's hard-sample scale, which is a compile-time constant.
        let scale_px = max(1.0, f32(HARD_SHADOW_SAMPLES) * 0.25);
        let interior = smoothstep(10.0 * scale_px, 56.0 * scale_px, f32(k));
        // A smooth sloping front can cover a larger depth interval between
        // adjacent samples. A discontinuity offers no such evidence.
        let front_step = select(0.0, abs(surface-prev.y) / max(1e-8,
            params.tuning.x * max(surface, prev.y)), continuous);
        let light_step = abs(receding) / f32(k);
        let pixel = w.pixel_width;
        let depth_budget = 1.0 + 3.0 * pixel + min(front_step, 4.0 * pixel) + 3.0 * light_step;
        let thickness = mix(1.0, w.shell_width, interior);
        let extra = mix(full_slab, min(full_slab, depth_budget), interior) - 1.0;
        let compact = max(1.0 - v / thickness, (v - extra) / thickness - 1.0);
        // When adjacent texels on the same continuous surface straddle the ray,
        // retain more of the old slab briefly while that surface remains visible.
        let weight = 1.0 - smoothstep(0.0, 12.0 * scale_px, since_crossing);
        corrected = mix(compact, original, weight);
    }
    // On rays approaching the eye, a detached front only establishes a surface
    // at the sampled depth. Treating a full depth-relative shell behind that
    // surface as solid moves a thin frame's shadow toward the frame on pullback.
    // The half width covers half a ray depth step plus half a pixel footprint,
    // in the same units as v. Keep a small inward bias for grazing front edges.
    let separation = v - receding;
    if (receding < 0.0 && separation > w.pixel_width && v > -1.0 && v < 2.3) {
        let advance = -receding / f32(k);
        let confidence = smoothstep(w.pixel_width, 2.0 * w.pixel_width, separation)
            * smoothstep(0.0, 0.5 * w.pixel_width, advance);
        let half_width = clamp(0.5 * (advance + w.pixel_width), 0.0625, 1.0);
        let front = abs(v - 0.25 * half_width) / half_width;
        let resolved = max(front, 1.0 - confidence / params.tuning.z);
        // A resolved intersection is a single ray hit even when the thin
        // caster occupies fewer than four consecutive taps. Keep the old
        // acceptance for attached surfaces and uncertain view angles.
        return vec3<f32>(original, mix(corrected, front, confidence), resolved);
    }
    return vec3<f32>(original, corrected, 1.0);
}

// The search ends at its last allotted step or at the viewport, whichever
// comes first. The step vector uses Bend's major-axis sample units. A
// component parallel to a screen edge imposes no limit in that dimension.
fn contact_taper(origin: vec2<f32>, step_px: vec2<f32>, size: vec2<f32>, cutoff: i32) -> vec2<f32> {
    let budget = f32(cutoff);
    let edge = select(origin, size - origin, step_px >= vec2<f32>(0.0));
    let rate = abs(step_px);
    let to_edge = select(vec2<f32>(budget), edge / max(rate, vec2<f32>(1e-6)), rate > vec2<f32>(1e-6));
    let end = min(budget, min(to_edge.x, to_edge.y));
    let start = max(f32(HARD_SHADOW_SAMPLES), 0.90 * end);
    return vec2<f32>(start, 1.0 / max(end - start, 1.0));
}

// Bend applies contrast after its soft lanes find their minimum. An additive
// pre-contrast fade can leave a strong sample fully dark even on the final
// step. Bound the final occlusion contributed by THIS sample. Earlier hits
// in the same lane still win, including when the rest of a ray goes offscreen.
fn bounded_delta(delta: f32, k: i32, taper: vec2<f32>, inverse_contrast: f32) -> f32 {
    let t = saturate((f32(k) - taper.x) * taper.y);
    let weight = 1.0 - t * t * (3.0 - 2.0 * t);
    return max(delta, 1.0 - weight * inverse_contrast);
}

fn isign(v: vec2<i32>) -> vec2<i32> {
    return vec2<i32>(select(select(0, -1, v.x < 0), 1, v.x > 0), select(select(0, -1, v.y < 0), 1, v.y > 0));
}

@compute @workgroup_size(64, 1, 1)
fn main(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) tid: u32) {
    // ---- ComputeWavefrontExtents
    var xy = vec2<i32>(group.yz) * WAVE_SIZE + params.wave_offset.xy;
    let light_xy = floor(params.light.xy) + 0.5;
    let light_xy_fraction = params.light.xy - light_xy;
    let reverse_direction = params.light.w > 0.0;
    let sign_xy = isign(xy);
    let horizontal = abs(xy.x + sign_xy.y) < abs(xy.y - sign_xy.x);
    let axis = vec2<i32>(select(0, sign_xy.y, horizontal), select(-sign_xy.x, 0, horizontal));
    xy = axis * i32(group.x) + xy;
    let xy_f = vec2<f32>(xy);
    let x_axis_major = abs(xy_f.x) > abs(xy_f.y);
    let major_axis = select(xy_f.y, xy_f.x, x_axis_major);
    let major_axis_start = abs(major_axis);
    let major_axis_end = abs(major_axis) - f32(WAVE_SIZE);
    var ma_light_frac = select(light_xy_fraction.y, light_xy_fraction.x, x_axis_major);
    ma_light_frac = select(ma_light_frac, -ma_light_frac, major_axis > 0.0);
    let start_xy = xy_f + light_xy;
    let end_xy = mix(params.light.xy, start_xy, (major_axis_end + ma_light_frac) / (major_axis_start + ma_light_frac));
    let xy_delta = start_xy - end_xy;
    let thread_step = f32(tid ^ select(u32(WAVE_SIZE - 1), 0u, reverse_direction));
    var pixel_xy = mix(start_xy, end_xy, thread_step / f32(WAVE_SIZE));
    let pixel_distance = major_axis_start - thread_step + ma_light_frac;

    // ---- the depths along the ray
    let direction = -params.light.w;
    let z_sign = select(1.0, -1.0, NEAR_DEPTH > FAR_DEPTH);
    var sampling_depth: array<f32, READ_COUNT>;
    var shadowing_depth: array<f32, READ_COUNT>;
    var depth_thickness_scale: array<f32, READ_COUNT>;
    var sample_distance: array<f32, READ_COUNT>;
    let ray_origin = pixel_xy;
    let write_xy = vec2<i32>(floor(pixel_xy));
    for (var i = 0; i < READ_COUNT; i++) {
        // two reads per sample, blended on the minor axis of the ray unless
        // they straddle an edge; the major axis stays on pixel centres
        let read_xy = vec2<i32>(floor(pixel_xy));
        let minor_axis = select(pixel_xy.x, pixel_xy.y, x_axis_major);
        let bilinear = fract(minor_axis) - 0.5;
        let bias = select(-1, 1, bilinear > 0.0);
        let offset_xy = select(vec2<i32>(bias, 0), vec2<i32>(0, bias), x_axis_major);
        let d0 = load_depth(read_xy);
        let d1 = load_depth(read_xy + offset_xy);
        // thresholds are fractions of what is left to the far plane
        depth_thickness_scale[i] = abs(FAR_DEPTH - d0);
        let use_point_filter = abs(d0 - d1) > depth_thickness_scale[i] * params.tuning.y;
        // the pixel starts at its own depth and casts from the further of
        // its two reads (BilinearSamplingOffsetMode off)
        sampling_depth[i] = d0;
        let shadow_depth = d0 + abs(d0 - d1) * z_sign;
        shadowing_depth[i] = select(shadow_depth, d0, use_point_filter);
        sample_distance[i] = pixel_distance + f32(WAVE_SIZE * i) * direction;
        pixel_xy += xy_delta * direction;
    }

    // ---- early out of a wavefront with nothing but sky (DepthBounds 0, 1),
    // its pixels lit
    let skip_pixel = sampling_depth[0] >= NEAR_DEPTH || sampling_depth[0] <= FAR_DEPTH;
    if (!skip_pixel) {
        atomicOr(&any_live, 1u);
    }
    if (workgroupUniformLoad(&any_live) == 0u) {
        store_lit(write_xy);
        return;
    }

    // ---- share the depths, made perspective-correct towards the light
    for (var i = 0; i < READ_COUNT; i++) {
        var stored_depth = (shadowing_depth[i] - params.light.z) / sample_distance[i];
        if (i != 0) {
            // reads that overshoot the light point are ignored
            stored_depth = select(1e10, stored_depth, sample_distance[i] > 0.0);
        }
        depth_data[i * WAVE_SIZE + i32(tid)] = stored_depth;
    }
    workgroupBarrier();
    if (skip_pixel) {
        store_lit(write_xy);
        return;
    }

    let start_depth = (sampling_depth[0] - params.light.z) / sample_distance[0];
    let sample_index = i32(tid) + 1;
    var shadow_value = vec4<f32>(1.0);
    var hard_shadow = 1.0;
    var front_delta = 1.0;
    // Compare the corrected result with v2.3.4 using the same depth reads.
    // It uses these four reference minima without taking extra depth reads.
    let compare_checkpoint = params.reach.y > 0.5 && params.reach.z < 0.5;
    var checkpoint_lanes = vec4<f32>(1.0);
    var checkpoint_hard = 1.0;
    // First candidate from the depth comparison. This is not a confirmed
    // physical intersection; it is an algorithm-local ray hit diagnostic.
    var first_candidate = SAMPLE_COUNT + 1;
    // Diagnostic only: did any sampled depth approach the acceptance window
    // without becoming a contact-shadow candidate?
    var near_depth_window = false;
    // Adaptive: depth decreases as view distance increases (infinite
    // reversed-Z). A near pixel may search hundreds of pixels, while distant
    // receivers use 60. The long kernel is cached once; the per-fragment
    // loop exits as soon as it has covered this receiver's reach.
    let adaptive = params.reach.x > 0.0;
    let cutoff = select(
        SAMPLE_COUNT,
        clamp(i32(ceil(params.reach.x * sampling_depth[0])), min(60, SAMPLE_COUNT), SAMPLE_COUNT),
        adaptive,
    );
    let step_px = xy_delta * direction / f32(WAVE_SIZE);
    let taper = contact_taper(ray_origin, step_px, vec2<f32>(textureDimensions(depth_tex)), cutoff);
    let inverse_contrast = 1.0 / params.tuning.z;
    // the shadowing window, in units of SurfaceThickness of the depth left
    // to the far plane (at least one pixel near the light point)
    let depth_scale = min(sample_distance[0] + direction, 1.0 / params.tuning.x) * sample_distance[0] / depth_thickness_scale[0];
    // One screen pixel spans 2 / (projection_y * height) of its
    // view-space distance. Express that in Bend's original shell units.
    let fallback = 1.1547005 / (f32(textureDimensions(depth_tex).y) * params.tuning.x);
    let pixel = select(fallback, params.reach.w, params.reach.w > 0.0);
    let shell_width = clamp(pixel * 1.5, 0.0625, 1.0);
    let window = ShadowWindow(
        start_depth * depth_scale,
        depth_scale,
        z_sign,
        sample_distance[0],
        direction,
        max(1.0, abs(params.tuning.w) * sampling_depth[0]),
        params.tuning.w < 0.0,
        pixel, shell_width,
        front_receiver_plane(write_xy, sampling_depth[0], step_px, x_axis_major, start_depth * direction, ray_origin),
    );
    var history = vec3<f32>(0.0, 0.0, 1e4);
    // sample i lies i + 1 pixels along the ray
    for (var i = 0; i < HARD_SHADOW_SAMPLES; i++) {
        let hits = depth_deltas(window, depth_data[sample_index + i], i + 1, &history);
        let delta = hits.y;
        hard_shadow = min(hard_shadow, delta);
        front_delta = min(front_delta, hits.z);
        if (compare_checkpoint) { checkpoint_hard = min(checkpoint_hard, hits.x); }
        if (params.reach.y > 0.5 && min(delta, hits.z) < 1.0) {
            first_candidate = min(first_candidate, i + 1);
        }
        if (params.reach.z > 0.5 && min(delta, hits.z) >= 1.0 && delta < 1.3) {
            near_depth_window = true;
        }
    }
    // For fixed modes, preserve Bend's existing loops and 8-sample tail.
    // In Adaptive mode the fade follows this pixel's cutoff, not the end of
    // the 480-sample kernel: prevents a hard truncation ring during zoom.
    for (var i = HARD_SHADOW_SAMPLES; i < SAMPLE_COUNT - FADE_OUT_SAMPLES; i++) {
        if (i >= cutoff) { break; }
        let adaptive_tail = max(i - (cutoff - FADE_OUT_SAMPLES) + 1, 0);
        let fade_out = select(0.0, f32(adaptive_tail) / f32(FADE_OUT_SAMPLES + 1) * 0.75, adaptive);
        let hits = depth_deltas(window, depth_data[sample_index + i], i + 1, &history);
        let delta = hits.y;
        if (compare_checkpoint) {
            checkpoint_lanes[i & 3] = min(checkpoint_lanes[i & 3], bounded_delta(hits.x + fade_out, i + 1, taper, inverse_contrast));
        }
        let edge = bounded_delta(0.0, i + 1, taper, inverse_contrast);
        let accepted = max(delta + fade_out, edge);
        let resolved = max(hits.z + fade_out, edge);
        front_delta = min(front_delta, resolved);
        shadow_value[i & 3] = min(shadow_value[i & 3], accepted);
        if (params.reach.y > 0.5 && min(accepted, resolved) < 1.0) {
            first_candidate = min(first_candidate, i + 1);
        }
        if (params.reach.z > 0.5 && min(delta, hits.z) + fade_out >= 1.0 && delta + fade_out < 1.3) {
            near_depth_window = true;
        }
    }
    for (var i = SAMPLE_COUNT - FADE_OUT_SAMPLES; i < SAMPLE_COUNT; i++) {
        if (i >= cutoff) { break; }
        let tail_end = select(SAMPLE_COUNT, cutoff, adaptive);
        let fade_out = f32(max(i + 1 - (tail_end - FADE_OUT_SAMPLES), 0)) / f32(FADE_OUT_SAMPLES + 1) * 0.75;
        let hits = depth_deltas(window, depth_data[sample_index + i], i + 1, &history);
        let delta = hits.y;
        if (compare_checkpoint) {
            checkpoint_lanes[i & 3] = min(checkpoint_lanes[i & 3], bounded_delta(hits.x + fade_out, i + 1, taper, inverse_contrast));
        }
        let edge = bounded_delta(0.0, i + 1, taper, inverse_contrast);
        let accepted = max(delta + fade_out, edge);
        let resolved = max(hits.z + fade_out, edge);
        front_delta = min(front_delta, resolved);
        shadow_value[i & 3] = min(shadow_value[i & 3], accepted);
        if (params.reach.y > 0.5 && min(accepted, resolved) < 1.0) {
            first_candidate = min(first_candidate, i + 1);
        }
        if (params.reach.z > 0.5 && min(delta, hits.z) + fade_out >= 1.0 && delta + fade_out < 1.3) {
            near_depth_window = true;
        }
    }
    // a sample on the reference depth is full shadow; the contrast widens it
    let contrast = params.tuning.z;
    shadow_value = saturate(shadow_value * contrast + (1.0 - contrast));
    hard_shadow = saturate(hard_shadow * contrast + (1.0 - contrast));
    let front_occlusion = saturate((1.0 - front_delta) * contrast);
    let result = min(1.0 - front_occlusion, min(hard_shadow, dot(shadow_value, vec4<f32>(0.25))));
    // B carries the resolved fraction for the spatial filter. Its high bit
    // keeps the existing offscreen diagnostic; 0 and 255 encode no fraction.
    let front_code = front_occlusion * (127.0 / 255.0);
    let size = vec2<i32>(textureDimensions(out_tex));
    if (all(write_xy >= vec2<i32>(0)) && all(write_xy < size)) {
        if (params.reach.y > 0.5) {
            // Ray direction is expressed in screen pixels per sample.
            let end_px = ray_origin + step_px * f32(cutoff);
            let exits = any(end_px < vec2<f32>(0.0)) || any(end_px >= vec2<f32>(size));
            let candidate = first_candidate <= cutoff;
            // No-hit g=1 means the ray ended at the 480-sample shader cap;
            // it does not prove that a longer search would hit geometry.
            let at_cap = !candidate && SAMPLE_COUNT >= 480 && cutoff >= SAMPLE_COUNT;
            let fraction = select(select(0.0, 1.0, at_cap),
                f32(first_candidate) / max(f32(cutoff), 1.0), candidate);
            // a: 1 accepted candidate, 0.5 near depth window without hit,
            // 0 neither. r is unchanged from the original shadow computation.
            let state = select(select(0.0, 0.5, near_depth_window), 1.0, candidate);
            // Failure analysis keeps its three states. In the candidate view
            // bit 0 marks an accepted sample. Bits 1..7 carry the change in
            // raw occlusion removed by the interior test, before spatial filtering.
            var metadata = state;
            if (compare_checkpoint) {
                let reference = saturate(checkpoint_lanes * contrast + (1.0 - contrast));
                let old_visibility = min(saturate((checkpoint_hard - 1.0) * contrast + 1.0), dot(reference, vec4<f32>(0.25)));
                let removed = saturate(result - old_visibility);
                let packed = 2u * u32(round(127.0 * removed)) + select(0u, 1u, candidate);
                metadata = f32(packed) / 255.0;
            }
            textureStore(out_tex, write_xy, vec4<f32>(1.0 - result, fraction,
                select(front_code, 1.0 - front_code, exits), metadata));
        } else {
            textureStore(out_tex, write_xy, vec4<f32>(1.0 - result, 0.0, front_code, 0.0));
        }
    }
}

// The pixel under the light's point, which no wavefront writes, lit (one
// invocation). Emptying the whole target first instead took 0.02 ms of the
// pass's 0.25 at 1920 x 1080 (demo plaza, RTX 4090).
@compute @workgroup_size(1, 1, 1)
fn clear_light() {
    let p = floor(params.light.xy);
    if (all(p >= vec2<f32>(0.0)) && all(p < vec2<f32>(textureDimensions(out_tex)))) {
        store_lit(vec2<i32>(p));
    }
}

// Every pixel lit, for `AURORA_CONTACT_VARIANT=skip`, which dispatches no
// wavefront (wgpu's clear_texture needs a device feature).
@compute @workgroup_size(8, 8, 1)
fn clear(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(out_tex);
    if (id.x < size.x && id.y < size.y) {
        textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(0.0));
    }
}
