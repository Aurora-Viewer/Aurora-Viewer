// FSR 3.1 luma and shading change pyramids (ffx_fsr3upscaler_luma_pyramid.h,
// ffx_fsr3upscaler_shading_change_pyramid.h), as one pass per level read
// downstream instead of the single pass downsampler: the farthest depth at
// half size (the luma pyramid's mip 1; its 1x1 average feeds only FSR's own
// automatic exposure, which Aurora does not use), and the first three
// levels of the shading change (the signed luma change against the last
// frame, and its sign, averaged).

// fs_farthest_depth_mip1: the farthest depth in meters, render size
// fs_shading_change_mip0: the current luma, the last frame's luma, the
// dilated motion; fs_downsample: the level above
@group(0) @binding(2) var src0: texture_2d<f32>;
@group(0) @binding(3) var src1: texture_2d<f32>;
@group(0) @binding(4) var src2: texture_2d<f32>;

@fragment
fn fs_farthest_depth_mip1(in: VsOut) -> @location(0) vec4<f32> {
    let p = px_of(in) * 2;
    var sum = 0.0;
    for (var i = 0; i < 4; i++) {
        let q = clamp_load(p, vec2<i32>(i & 1, i >> 1u), fsr.render_size);
        sum += textureLoad(src0, q, 0).x;
    }
    return vec4<f32>(sum * 0.25, 0.0, 0.0, 0.0);
}

const SET_SIZE: i32 = 5;

fn sample_offsets() -> array<vec2<i32>, 5> {
    return array<vec2<i32>, 5>(vec2<i32>(0, 0), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(0, -1), vec2<i32>(0, 1));
}

fn compare_swap(s: ptr<function, array<f32, 5>>, i: i32, j: i32) {
    let lo = min((*s)[i], (*s)[j]);
    (*s)[j] = max((*s)[i], (*s)[j]);
    (*s)[i] = lo;
}

fn sort_set(s: ptr<function, array<f32, 5>>) {
    compare_swap(s, 0, 3);
    compare_swap(s, 1, 4);
    compare_swap(s, 0, 2);
    compare_swap(s, 1, 3);
    compare_swap(s, 0, 1);
    compare_swap(s, 2, 4);
    compare_swap(s, 1, 2);
    compare_swap(s, 3, 4);
    compare_swap(s, 2, 3);
}

fn minimum_difference(set0_in: array<f32, 5>, set1_in: array<f32, 5>) -> f32 {
    var set0 = set0_in;
    var set1 = set1_in;
    var min_diff = FP16_MAX - 1.0;
    var a = 0;
    var b = 0;
    sort_set(&set0);
    sort_set(&set1);
    let fmax = min(set0[SET_SIZE - 1], set1[SET_SIZE - 1]);
    if (fmax > FP32_MIN) {
        for (var i = 0; i < SET_SIZE && min_diff < FP16_MAX; i++) {
            // the SDK can step one past the set: held at its last sample
            let s0 = set0[min(a, SET_SIZE - 1)];
            let s1 = set1[min(b, SET_SIZE - 1)];
            var diff = s0 - s1;
            if (abs(diff) > FP16_MIN) {
                diff = sign(diff) * (1.0 - min_divided_by_max(s0, s1));
                min_diff = select(min_diff, diff, abs(diff) < abs(min_diff));
                a += i32(s0 < s1);
                // with the new `a`, as the SDK
                b += i32(set0[min(a, SET_SIZE - 1)] >= s1);
            } else {
                min_diff = FP16_MAX;
            }
        }
    }
    return min_diff * f32(min_diff < FP16_MAX - 1.0);
}

fn luma_difference(p: vec2<i32>, uv: vec2<f32>, mv: vec2<f32>) -> f32 {
    var offsets = sample_offsets();
    var current: array<f32, 5>;
    let cur_base = vec2<i32>(floor((uv + fsr.jitter / render_size()) * render_size()));
    for (var i = 0; i < SET_SIZE; i++) {
        let q = clamp_load(cur_base, offsets[i], fsr.render_size);
        current[i] = max(textureLoad(src0, q, 0).x * fsr.exposure, EPSILON);
    }
    let prev_size = vec2<f32>(fsr.prev_render_size);
    let reprojected = uv + fsr.prev_jitter / prev_size + mv;
    if (!is_uv_inside(reprojected)) {
        return 0.0;
    }
    var previous: array<f32, 5>;
    let prev_base = vec2<i32>(floor(reprojected * prev_size));
    for (var i = 0; i < SET_SIZE; i++) {
        let q = clamp_load(prev_base, offsets[i], fsr.prev_render_size);
        previous[i] = max(textureLoad(src1, q, 0).x * fsr.delta_pre_exposure * fsr.exposure, EPSILON);
    }
    return minimum_difference(current, previous);
}

@fragment
fn fs_shading_change_mip0(in: VsOut) -> @location(0) vec4<f32> {
    let p = px_of(in) * 2;
    var sum = vec2<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let q = clamp_load(p, vec2<i32>(i & 1, i >> 1u), fsr.render_size);
        let mv = textureLoad(src2, q, 0).xy;
        let uv = (vec2<f32>(q) + 0.5) / render_size();
        let diff = luma_difference(q, uv, mv);
        sum += vec2<f32>(diff, select(0.0, sign(diff), diff != 0.0));
    }
    return vec4<f32>(sum * 0.25, 0.0, 0.0);
}

// the next level of the shading change: the average of 2 x 2
@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(src0));
    let p = px_of(in) * 2;
    var sum = vec2<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        sum += textureLoad(src0, clamp_load(p, vec2<i32>(i & 1, i >> 1u), size), 0).xy;
    }
    return vec4<f32>(sum * 0.25, 0.0, 0.0);
}

// FSR 3.1 shading change (ffx_fsr3upscaler_shading_change.h), half size:
// the strongest of the first three levels, src0 being the whole pyramid
const SHADING_CHANGE_LEVELS: i32 = 3;

@fragment
fn fs_shading_change(in: VsOut) -> @location(0) vec4<f32> {
    let p = px_of(in);
    let size = shading_change_render_size();
    if (!is_on_screen(p, size)) {
        return vec4<f32>(0.0);
    }
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size) + fsr.jitter / render_size();
    let mip_uv = clamp_uv(uv, size, vec2<i32>(textureDimensions(src0, 0)));
    var change = 0.0;
    for (var level = 0; level < SHADING_CHANGE_LEVELS; level++) {
        let s = textureSampleLevel(src0, lin, mip_uv, f32(level)).xy;
        let v = abs(s.x * s.y);
        if (v > 0.0) {
            change = max(change, v);
        }
    }
    return vec4<f32>(sat(change), 0.0, 0.0, 0.0);
}

// FSR 3.1 reactive mask, generated (ffx_fsr3upscaler_autogen_reactive_pass):
// how much the blended faces, the particles and the water changed the
// opaque image (src0: the scene copy before them, src1: the final colour),
// with the SDK sample's flags: tone mapped, largest channel, threshold.
const REACTIVE_SCALE: f32 = 1.0;
const REACTIVE_THRESHOLD: f32 = 0.2;
const REACTIVE_BINARY_VALUE: f32 = 0.9;

@fragment
fn fs_autogen_reactive(in: VsOut) -> @location(0) vec4<f32> {
    let p = clamp_load(px_of(in), vec2<i32>(0), fsr.render_size);
    let pre = tonemap(max(vec3<f32>(0.0), safe_color(textureLoad(src0, p, 0).rgb)));
    let post = tonemap(max(vec3<f32>(0.0), safe_color(textureLoad(src1, p, 0).rgb)));
    let delta = abs(post - pre);
    let v = max(delta.x, max(delta.y, delta.z)) * REACTIVE_SCALE * fsr.reactive_on;
    return vec4<f32>(select(REACTIVE_BINARY_VALUE, 0.0, v < REACTIVE_THRESHOLD), 0.0, 0.0, 0.0);
}
