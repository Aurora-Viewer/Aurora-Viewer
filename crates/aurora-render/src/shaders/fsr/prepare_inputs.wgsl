// FSR 3.1 prepare inputs (ffx_fsr3upscaler_prepare_inputs.h): nearest depth
// of the 3x3 and its motion, the previous frame's depth reconstructed by
// pushing the nearest depth to where it was, the farthest depth in meters
// and the luma of the pixel.

@group(0) @binding(2) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var depth_tex: texture_depth_2d;
// zw: the pixel's screen motion since the last frame in uv, without the
// jitter (object.wgsl depth_normal); 0 where the prepass drew nothing
@group(0) @binding(4) var motion_tex: texture_2d<f32>;
// one u32 a pixel: the bits of the nearest depth (max: reverse-Z)
@group(0) @binding(5) var<storage, read_write> reconstructed: array<atomic<u32>>;

struct PrepareOut {
    @location(0) dilated_motion: vec4<f32>,
    @location(1) dilated_depth: vec4<f32>,
    @location(2) farthest_depth: vec4<f32>,
    @location(3) luma: vec4<f32>,
};

fn load_depth(p: vec2<i32>) -> f32 {
    return textureLoad(depth_tex, clamp_load(p, vec2<i32>(0), fsr.render_size), 0);
}

// The motion to the last frame, in uv (FSR's sign: previous = uv + motion).
// Where the prepass wrote none (the sky, water and what did not move) the
// camera alone moves the pixel, through its depth, as taa.wgsl does.
fn load_input_motion_vector(p_in: vec2<i32>) -> vec2<f32> {
    let p = clamp_load(p_in, vec2<i32>(0), fsr.render_size);
    let motion = textureLoad(motion_tex, p, 0).zw;
    let depth = textureLoad(depth_tex, p, 0);
    if (fsr.motion_on > 0.5 && depth > 0.0 && any(motion != vec2<f32>(0.0))) {
        return -motion;
    }
    let uv = (vec2<f32>(p) + 0.5) / render_size();
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let wp = fsr.inv_view_proj * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
    let pp = fsr.prev_view_proj * vec4<f32>(wp.xyz / wp.w, 1.0);
    if (pp.w <= 0.0) {
        // behind the last camera: no history there
        return vec2<f32>(2.0);
    }
    let prev_uv = vec2<f32>(pp.x / pp.w * 0.5 + 0.5, 0.5 - pp.y / pp.w * 0.5);
    // the pixel shows the unjittered point `uv - jitter`
    return prev_uv - (uv - fsr.jitter / render_size());
}

fn reconstruct_prev_depth(p: vec2<i32>, depth: f32, mv_in: vec2<f32>) {
    let nearest_m = min(view_space_depth_in_meters(depth), FP16_MAX);
    let threshold = reconstructed_depth_mv_px_threshold(nearest_m);
    // discard small motions
    let mv = mv_in * f32(get_4k_velocity(mv_in) > threshold);
    let uv = (vec2<f32>(p) + 0.5) / render_size();
    let b = bilinear_data(uv + mv, fsr.render_size);
    var offsets = BILINEAR_OFFSETS;
    for (var i = 0; i < 4; i++) {
        if (b.weights[i] > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
            let q = b.base + offsets[i];
            if (is_on_screen(q, fsr.render_size)) {
                atomicMax(&reconstructed[q.y * fsr.render_size.x + q.x], bitcast<u32>(depth));
            }
        }
    }
}

@fragment
fn fs_prepare_inputs(in: VsOut) -> PrepareOut {
    let p = px_of(in);
    // the 3x3's nearest (largest, reverse-Z) and farthest depths
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1), vec2<i32>(-1, 0),
        vec2<i32>(-1, 1), vec2<i32>(1, 1), vec2<i32>(-1, -1), vec2<i32>(1, -1),
    );
    var nearest = load_depth(p);
    var nearest_at = p;
    var farthest = nearest;
    for (var i = 1; i < 9; i++) {
        let q = p + offsets[i];
        if (is_on_screen(q, fsr.render_size)) {
            let d = load_depth(q);
            if (d > nearest) {
                farthest = min(farthest, d);
                nearest_at = q;
                nearest = d;
            }
        }
    }
    let mv = load_input_motion_vector(nearest_at);
    reconstruct_prev_depth(p, nearest, mv);
    let rgb = max(vec3<f32>(0.0), safe_color(textureLoad(color_tex, clamp_load(p, vec2<i32>(0), fsr.render_size), 0).rgb));
    var o: PrepareOut;
    o.dilated_motion = vec4<f32>(mv, 0.0, 0.0);
    o.dilated_depth = vec4<f32>(nearest, 0.0, 0.0, 0.0);
    o.farthest_depth = vec4<f32>(min(view_space_depth_in_meters(farthest), FP16_MAX), 0.0, 0.0, 0.0);
    o.luma = vec4<f32>(rgb_to_luma(rgb), 0.0, 0.0, 0.0);
    return o;
}

// The scene's depth at the display size, for the passes after FSR (the
// selection outline, the depth of field, RLVa's spheres, the photo's
// looks): the render pixel under each display pixel.
@fragment
fn fs_display_depth(in: VsOut) -> @builtin(frag_depth) f32 {
    return load_depth(vec2<i32>(in.clip.xy * fsr.downscale));
}
