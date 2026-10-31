// Screen-space contact shadows of the sun (or the moon), from the
// single-sample depth prepass: a WGSL port of Bend Studio's screen space
// shadows (bend_sss_gpu.h, as shipped in Days Gone), run in the dispatches
// that contact_shadows.rs lays out. The result is the shadow in `.r` (0 lit,
// 1 in contact shadow: `clear` empties the target first each frame, so a
// pixel no wavefront writes, the light's own one, stays lit) that the shading
// combines with the sun's shadow map (`contact_shadow` in common.wgsl).
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
// set by any thread whose pixel needs a shadow (zero at the start)
var<workgroup> any_live: u32;

fn load_depth(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_tex));
    if (any(p < vec2<i32>(0)) || any(p >= size)) {
        return FAR_DEPTH;
    }
    return textureLoad(depth_tex, p, 0);
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
fn depth_delta(w: ShadowWindow, sample_depth: f32, k: i32) -> f32 {
    let v = (sample_depth * w.scale - w.start) * -w.z_sign;
    // the pixel's depth seen from the sample, in front of the ray by
    // start × (distance / distance_k - 1)
    let distance_k = max(w.distance + f32(k) * w.direction, 1.0);
    let receding = w.start * f32(k) * w.direction * w.z_sign / distance_k;
    return max(1.0 - v, v - clamp(receding, 1.0, w.depth));
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

    // ---- early out of a wavefront with nothing but sky (DepthBounds 0, 1)
    let skip_pixel = sampling_depth[0] >= NEAR_DEPTH || sampling_depth[0] <= FAR_DEPTH;
    if (!skip_pixel) {
        any_live = 1u;
    }
    if (workgroupUniformLoad(&any_live) == 0u) {
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
        return;
    }

    let start_depth = (sampling_depth[0] - params.light.z) / sample_distance[0];
    let sample_index = i32(tid) + 1;
    var shadow_value = vec4<f32>(1.0);
    var hard_shadow = 1.0;
    // the shadowing window, in units of SurfaceThickness of the depth left
    // to the far plane (at least one pixel near the light point)
    let depth_scale = min(sample_distance[0] + direction, 1.0 / params.tuning.x) * sample_distance[0] / depth_thickness_scale[0];
    let window = ShadowWindow(
        start_depth * depth_scale,
        depth_scale,
        z_sign,
        sample_distance[0],
        direction,
        max(1.0, params.tuning.w * sampling_depth[0]),
    );
    // sample i lies i + 1 pixels along the ray
    for (var i = 0; i < HARD_SHADOW_SAMPLES; i++) {
        hard_shadow = min(hard_shadow, depth_delta(window, depth_data[sample_index + i], i + 1));
    }
    // the bulk, in four groups averaged at the end
    for (var i = HARD_SHADOW_SAMPLES; i < SAMPLE_COUNT - FADE_OUT_SAMPLES; i++) {
        shadow_value[i & 3] = min(shadow_value[i & 3], depth_delta(window, depth_data[sample_index + i], i + 1));
    }
    for (var i = SAMPLE_COUNT - FADE_OUT_SAMPLES; i < SAMPLE_COUNT; i++) {
        let fade_out = f32(i + 1 - (SAMPLE_COUNT - FADE_OUT_SAMPLES)) / f32(FADE_OUT_SAMPLES + 1) * 0.75;
        shadow_value[i & 3] = min(shadow_value[i & 3], depth_delta(window, depth_data[sample_index + i], i + 1) + fade_out);
    }
    // a sample on the reference depth is full shadow; the contrast widens it
    let contrast = params.tuning.z;
    shadow_value = saturate(shadow_value * contrast + (1.0 - contrast));
    hard_shadow = saturate(hard_shadow * contrast + (1.0 - contrast));
    let result = min(hard_shadow, dot(shadow_value, vec4<f32>(0.25)));
    let size = vec2<i32>(textureDimensions(out_tex));
    if (all(write_xy >= vec2<i32>(0)) && all(write_xy < size)) {
        textureStore(out_tex, write_xy, vec4<f32>(1.0 - result, 0.0, 0.0, 0.0));
    }
}

// Every pixel lit, before the wavefronts (wgpu's clear_texture needs a
// device feature).
@compute @workgroup_size(8, 8, 1)
fn clear(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(out_tex);
    if (id.x < size.x && id.y < size.y) {
        textureStore(out_tex, vec2<i32>(id.xy), vec4<f32>(0.0));
    }
}
