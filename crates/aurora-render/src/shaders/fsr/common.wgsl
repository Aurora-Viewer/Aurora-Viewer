// AMD FidelityFX Super Resolution 3.1 (the upscaler only), ported to WGSL
// from the FidelityFX SDK 1.1.4 (sdk/include/FidelityFX/gpu/fsr3upscaler/,
// gpu/fsr1/ffx_fsr1.h for RCAS; Copyright (C) 2024 Advanced Micro Devices,
// Inc., MIT licence, see NOTICE.md). This file holds what every pass
// shares (ffx_fsr3upscaler_common.h, the callbacks' constant buffer and
// ffx_fsr3upscaler_sample.h); each pass file is appended to it.
//
// Differences from the SDK, all forced by WGSL or by Aurora's frame:
// - every pass is a full-screen fragment pass (one pixel per invocation,
//   as the SDK's compute passes are), so every intermediate format stays
//   renderable and filterable on every backend; the two scattered writes
//   (the reconstructed previous depth and the new locks) go to storage
//   buffers, the depth's InterlockedMax to an atomicMax on its bits;
// - the luma and shading change pyramids (single pass downsampler, whose
//   cross-workgroup counter needs globallycoherent textures WGSL has not)
//   become one pass per level that is read: the farthest depth at half
//   size and the shading change's three first levels;
// - f32 everywhere (no FP16 permutation);
// - textures are read in bounds only: where D3D returns 0 for a load
//   outside the texture, `load_or_zero` does the same;
// - ffxSaturate maps a NaN to 0 as HLSL saturate does (`sat`).
//
// Options of this port (the SDK's permutation flags): HDR colour input,
// inverted (reverse-Z) depth, motion vectors at render size and without
// jitter, Lanczos reference for the history and the FSR1 approximation for
// the upsampling (the SDK's defaults).

struct FsrParams {
    // this frame's jittered view projection, inverted, and last frame's
    // unjittered one: the camera's motion where the prepass drew none
    inv_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
    render_size: vec2<i32>,
    prev_render_size: vec2<i32>,
    upscale_size: vec2<i32>,
    prev_upscale_size: vec2<i32>,
    max_render_size: vec2<i32>,
    max_upscale_size: vec2<i32>,
    // device depth to view depth: z = y / (depth - x)
    device_to_view: vec4<f32>,
    // in pixels, y down: where the jitter moved the image
    jitter: vec2<f32>,
    prev_jitter: vec2<f32>,
    downscale: vec2<f32>,
    jitter_sequence_length: f32,
    delta_pre_exposure: f32,
    frame_index: f32,
    velocity_factor: f32,
    accumulation_added_per_frame: f32,
    min_disocclusion_accumulation: f32,
    exposure: f32,
    // RCAS sharpness, linear (FsrRcasCon: exp2(-stops))
    rcas: f32,
    view_to_meters: f32,
    // 1 when the prepass's motion vectors are used (AURORA_TAA_MOTION)
    motion_on: f32,
    // 0 without the generated reactive mask (AURORA_FSR_REACTIVE=0)
    reactive_on: f32,
    // 0 without the transparency and composition mask (AURORA_FSR_COMPOSITION=0)
    composition_on: f32,
    pad1: f32,
    pad2: f32,
};

@group(0) @binding(0) var<uniform> fsr: FsrParams;
@group(0) @binding(1) var lin: sampler;

const FP16_MIN: f32 = 6.10e-05;
const FP16_MAX: f32 = 65504.0;
const EPSILON: f32 = FP16_MIN;
const TONEMAP_EPSILON: f32 = FP16_MIN;
const FP32_MAX: f32 = 3.402823466e+38;
const FP32_MIN: f32 = 1.175494351e-38;
const RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD: f32 = FP16_MIN * 10.0;
const UPSAMPLE_LANCZOS_WEIGHT_SCALE: f32 = 1.0 / 16.0;
const AVERAGE_LANCZOS_WEIGHT_PER_FRAME: f32 = 0.74 * UPSAMPLE_LANCZOS_WEIGHT_SCALE;
const LOCK_THRESHOLD: f32 = 1.0;
const LOCK_MAX: f32 = 2.0;

// the channels of the dilated reactive masks
const REACTIVE: i32 = 0;
const DISOCCLUSION: i32 = 1;
const SHADING_CHANGE: i32 = 2;
const ACCUMULATION: i32 = 3;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_fsr(@builtin(vertex_index) vi: u32) -> VsOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: VsOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn px_of(in: VsOut) -> vec2<i32> {
    return vec2<i32>(in.clip.xy);
}

// HLSL saturate: a NaN gives 0
fn sat(x: f32) -> f32 {
    return select(clamp(x, 0.0, 1.0), 0.0, x != x);
}

fn sat3(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(sat(v.x), sat(v.y), sat(v.z));
}

fn render_size() -> vec2<f32> {
    return vec2<f32>(fsr.render_size);
}

fn shading_change_render_size() -> vec2<i32> {
    return vec2<i32>(vec2<f32>(fsr.render_size) * 0.5);
}

fn shading_change_max_render_size() -> vec2<i32> {
    return vec2<i32>(vec2<f32>(fsr.max_render_size) * 0.5);
}

fn reconstructed_depth_mv_px_threshold(nearest_depth_in_meters: f32) -> f32 {
    return mix(0.25, 0.75, sat(nearest_depth_in_meters / 100.0));
}

fn get_4k_velocity(mv: vec2<f32>) -> f32 {
    return length(mv * vec2<f32>(3840.0, 2160.0));
}

fn min_divided_by_max_or(v0: f32, v1: f32, on_zero: f32) -> f32 {
    let m = max(v0, v1);
    return select(on_zero, min(v0, v1) / m, m != 0.0);
}

fn min_divided_by_max(v0: f32, v1: f32) -> f32 {
    return min_divided_by_max_or(v0, v1, 0.0);
}

fn ycocg_to_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(c.x + c.y - c.z, c.x + c.z, c.x - c.y - c.z);
}

fn rgb_to_ycocg(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        0.25 * c.r + 0.5 * c.g + 0.25 * c.b,
        0.5 * c.r - 0.5 * c.b,
        -0.25 * c.r + 0.5 * c.g - 0.25 * c.b,
    );
}

fn rgb_to_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    return c / (max(max(0.0, c.r), max(c.g, c.b)) + 1.0);
}

fn inverse_tonemap(c: vec3<f32>) -> vec3<f32> {
    return c / max(TONEMAP_EPSILON, 1.0 - max(c.r, max(c.g, c.b)));
}

fn is_uv_inside(uv: vec2<f32>) -> bool {
    return uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0;
}

fn clamp_load(p: vec2<i32>, offset: vec2<i32>, size: vec2<i32>) -> vec2<i32> {
    return clamp(p + offset, vec2<i32>(0), size - vec2<i32>(1));
}

fn clamp_uv(uv: vec2<f32>, texture_size: vec2<i32>, resource_size: vec2<i32>) -> vec2<f32> {
    let location = uv * vec2<f32>(texture_size);
    let clamped = max(vec2<f32>(0.5), min(location, vec2<f32>(texture_size) - vec2<f32>(0.5)));
    return clamped / vec2<f32>(resource_size);
}

fn is_on_screen(p: vec2<i32>, size: vec2<i32>) -> bool {
    return all(vec2<u32>(p) < vec2<u32>(size));
}

fn compute_hr_pos_from_lr_pos(lr: vec2<i32>) -> vec2<i32> {
    let jittered = vec2<f32>(lr) + 0.5 - fsr.jitter;
    let in_hr = (jittered / render_size()) * vec2<f32>(fsr.upscale_size);
    return vec2<i32>(floor(in_hr));
}

fn view_space_depth(device_depth: f32) -> f32 {
    return fsr.device_to_view.y / (device_depth - fsr.device_to_view.x);
}

fn view_space_depth_in_meters(device_depth: f32) -> f32 {
    return view_space_depth(device_depth) * fsr.view_to_meters;
}

// the four texels of a bilinear fetch (GetBilinearSamplingData)
struct Bilinear {
    base: vec2<i32>,
    weights: vec4<f32>,
};

const BILINEAR_OFFSETS = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));

fn bilinear_data(uv: vec2<f32>, size: vec2<i32>) -> Bilinear {
    let p = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let f = fract(p);
    var b: Bilinear;
    b.base = vec2<i32>(floor(p));
    b.weights = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    return b;
}

fn lanczos2_no_clamp(x: f32) -> f32 {
    let pi = 3.141592653589793;
    if (abs(x) < EPSILON) {
        return 1.0;
    }
    return (sin(pi * x) / (pi * x)) * (sin(0.5 * pi * x) / (0.5 * pi * x));
}

fn lanczos2(x: f32) -> f32 {
    return lanczos2_no_clamp(min(abs(x), 2.0));
}

// FSR1's approximation; x2 = x * x, at most 4
fn lanczos2_approx_sq(x2_in: f32) -> f32 {
    let x2 = min(x2_in, 4.0);
    let a = (2.0 / 5.0) * x2 - 1.0;
    let b = (1.0 / 4.0) * x2 - 1.0;
    return ((25.0 / 16.0) * a * a - (25.0 / 16.0 - 1.0)) * (b * b);
}

// the colour box of the upsampling, in YCoCg
struct RectificationBox {
    center: vec3<f32>,
    vec: vec3<f32>,
    aabb_min: vec3<f32>,
    aabb_max: vec3<f32>,
    center_weight: f32,
};

fn box_add_sample(initial: bool, b: ptr<function, RectificationBox>, c: vec3<f32>, w: f32) {
    let weighted = c * w;
    if (initial) {
        (*b).aabb_min = c;
        (*b).aabb_max = c;
        (*b).center = weighted;
        (*b).vec = c * weighted;
        (*b).center_weight = w;
    } else {
        (*b).aabb_min = min((*b).aabb_min, c);
        (*b).aabb_max = max((*b).aabb_max, c);
        (*b).center += weighted;
        (*b).vec += c * weighted;
        (*b).center_weight += w;
    }
}

fn box_compute_variance(b: ptr<function, RectificationBox>) {
    let w = select(1.0, (*b).center_weight, abs((*b).center_weight) > FP32_MIN);
    (*b).center_weight = w;
    (*b).center /= w;
    (*b).vec /= w;
    (*b).vec = sqrt(abs((*b).vec - (*b).center * (*b).center));
}

// HDR input made safe (taa.wgsl `tm`, #1380): an infinite value held at
// the format's largest, a NaN black, so neither stays in the history
fn safe_color(c: vec3<f32>) -> vec3<f32> {
    return select(min(c, vec3<f32>(FP16_MAX)), vec3<f32>(0.0), c != c);
}
