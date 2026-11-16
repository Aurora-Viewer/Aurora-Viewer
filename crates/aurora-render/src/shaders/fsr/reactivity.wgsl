// FSR 3.1 prepare reactivity (ffx_fsr3upscaler_prepare_reactivity.h):
// disocclusion against the reconstructed previous depth, shading change,
// motion divergence, the accumulation count of each pixel, and the new
// locks of thin features. The reactive mask is generated
// (fs_autogen_reactive); the transparency and composition mask comes from
// the prepass (`composition_mask`).

@group(0) @binding(2) var dilated_motion_tex: texture_2d<f32>;
@group(0) @binding(3) var dilated_depth_tex: texture_2d<f32>;
@group(0) @binding(4) var shading_change_tex: texture_2d<f32>;
@group(0) @binding(5) var prev_accumulation_tex: texture_2d<f32>;
@group(0) @binding(6) var luma_tex: texture_2d<f32>;
@group(0) @binding(7) var<storage, read> reconstructed: array<u32>;
// one f32 a pixel of the upscaled image, zeroed by the accumulation
@group(0) @binding(8) var<storage, read_write> new_locks: array<f32>;
@group(0) @binding(9) var reactive_tex: texture_2d<f32>;
// the prepass's encoded normal and motion (prepare_inputs.wgsl motion_tex):
// its x is over 1 on the faces whose texture moves over them
// (NORMAL_COMPOSITION, object.wgsl normal_composition)
@group(0) @binding(10) var normal_tex: texture_2d<f32>;

struct ReactivityOut {
    @location(0) masks: vec4<f32>,
    @location(1) accumulation: vec4<f32>,
};

// D3D's load: 0 outside the texture
fn load_dilated_depth_or_zero(p: vec2<i32>) -> f32 {
    if (!is_on_screen(p, vec2<i32>(textureDimensions(dilated_depth_tex)))) {
        return 0.0;
    }
    return textureLoad(dilated_depth_tex, p, 0).x;
}

fn load_dilated_motion_or_zero(p: vec2<i32>) -> vec2<f32> {
    if (!is_on_screen(p, vec2<i32>(textureDimensions(dilated_motion_tex)))) {
        return vec2<f32>(0.0);
    }
    return textureLoad(dilated_motion_tex, p, 0).xy;
}

fn compute_disocclusions(uv: vec2<f32>, mv_in: vec2<f32>, current_depth_view: f32) -> f32 {
    let nearest_m = min(current_depth_view * fsr.view_to_meters, FP16_MAX);
    let threshold = reconstructed_depth_mv_px_threshold(nearest_m);
    let mv = mv_in * f32(get_4k_velocity(mv_in) > threshold);
    let b = bilinear_data(uv + mv, fsr.render_size);
    var offsets = BILINEAR_OFFSETS;
    var disocclusion = 0.0;
    var weight_sum = 0.0;
    var potential = true;
    for (var i = 0; i < 4 && potential; i++) {
        let q = clamp_load(b.base, offsets[i], fsr.render_size);
        let w = b.weights[i];
        if (w > RECONSTRUCTED_DEPTH_BILINEAR_WEIGHT_THRESHOLD) {
            let prev_depth = bitcast<f32>(reconstructed[q.y * fsr.render_size.x + q.x]);
            let prev_view = view_space_depth(prev_depth);
            let difference = current_depth_view - prev_view;
            potential = potential && difference > FP32_MIN;
            if (potential) {
                let half_viewport = length(render_size() * 0.5);
                let depth_threshold = max(current_depth_view, prev_view);
                let ksep = 1.37e-05;
                let separation = ksep * half_viewport * depth_threshold;
                disocclusion += sat(separation / difference) * w;
                weight_sum += w;
            }
        }
    }
    return select(0.0, sat(1.0 - disocclusion / weight_sum), potential && weight_sum > 0.0);
}

fn compute_motion_divergence(uv: vec2<f32>, mv: vec2<f32>, current_depth: f32) -> f32 {
    let q = vec2<i32>((uv + mv) * render_size());
    let reprojected_depth = load_dilated_depth_or_zero(q);
    let reprojected_mv = load_dilated_motion_or_zero(q);
    let reprojected_velocity = get_4k_velocity(reprojected_mv);
    let velocity = get_4k_velocity(mv);
    let nucleus_m = view_space_depth_in_meters(reprojected_depth);
    let current_m = view_space_depth_in_meters(current_depth);
    let distance_factor = min_divided_by_max(nucleus_m, current_m);
    let velocity_factor = sat(velocity / 10.0);
    return (1.0 - sat(reprojected_velocity / velocity)) * distance_factor * velocity_factor;
}

fn thin_feature_confidence(p: vec2<i32>) -> f32 {
    //  1 2 3
    //  4 0 5
    //  6 7 8
    var offsets = array<vec2<i32>, 9>(
        vec2<i32>(0, 0), vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0),
        vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1),
    );
    var samples: array<f32, 9>;
    var luma_min = FP32_MAX;
    var luma_max = FP32_MIN;
    for (var i = 0; i < 9; i++) {
        samples[i] = textureLoad(luma_tex, clamp_load(p, offsets[i], fsr.render_size), 0).x * fsr.exposure;
        luma_min = min(luma_min, samples[i]);
        luma_max = max(luma_max, samples[i]);
    }
    let threshold = 0.9;
    var dissimilar_min = FP32_MAX;
    var dissimilar_max = 0.0;
    // the nucleus counts as similar
    var pattern = 1u;
    var rejection = array<u32, 4>(
        (1u << 1u) | (1u << 2u) | (1u << 4u) | 1u,
        (1u << 2u) | (1u << 3u) | (1u << 5u) | 1u,
        (1u << 4u) | (1u << 6u) | (1u << 7u) | 1u,
        (1u << 5u) | (1u << 7u) | (1u << 8u) | 1u,
    );
    for (var i = 1; i < 9; i++) {
        let difference = abs(samples[i] - samples[0]) / (luma_max - luma_min);
        if (difference < threshold) {
            pattern |= 1u << u32(i);
        } else {
            dissimilar_min = min(dissimilar_min, samples[i]);
            dissimilar_max = max(dissimilar_max, samples[i]);
        }
    }
    let ridge = samples[0] > dissimilar_max || samples[0] < dissimilar_min;
    if (!ridge) {
        return 0.0;
    }
    for (var i = 0; i < 4; i++) {
        if ((pattern & rejection[i]) == rejection[i]) {
            return 0.0;
        }
    }
    return 1.0 - luma_min / luma_max;
}

fn update_accumulation(p: vec2<i32>, uv: vec2<f32>, mv: vec2<f32>, disocclusion: f32, shading_change: f32) -> vec2<f32> {
    let reprojected = uv + mv;
    var accumulation = 0.0;
    if (is_uv_inside(reprojected)) {
        let hw = clamp_uv(reprojected, fsr.prev_render_size, fsr.max_render_size);
        accumulation = sat(textureSampleLevel(prev_accumulation_tex, lin, hw, 0.0).x);
    }
    // a shading change starts the count again, a disocclusion one frame
    // below it (the SDK's comments count the frames)
    accumulation = mix(accumulation, 0.0, shading_change);
    accumulation = mix(accumulation, min(fsr.min_disocclusion_accumulation, accumulation), disocclusion);
    accumulation *= f32(round(accumulation * 100.0) > 1.0);
    // x: this frame's, y: stored for the next
    return vec2<f32>(accumulation, sat(accumulation + fsr.accumulation_added_per_frame));
}

fn dilate_reactive_masks(p: vec2<i32>) -> f32 {
    var m = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            m = max(m, textureLoad(reactive_tex, clamp_load(p, vec2<i32>(x, y), fsr.render_size), 0).x);
        }
    }
    return m;
}

fn compute_shading_change(uv: vec2<f32>) -> f32 {
    // the jitter applied again, removed when the accumulation samples it
    let su = clamp_uv(uv - fsr.jitter / render_size(), shading_change_render_size(), shading_change_max_render_size());
    return sat(textureSampleLevel(shading_change_tex, lin, su, 0.0).x);
}

// DilateTransparencyAndCompositionMasks: 1 on a running texture animation,
// whose texture slides, turns or flips over a face the motion says is
// still. The accumulation then keeps the history within this frame's
// colours and drops the locks: the thin lines of a turning texture showed
// twice, the history's locked one and this frame's.
fn composition_mask(p: vec2<i32>) -> f32 {
    let x = textureLoad(normal_tex, clamp_load(p, vec2<i32>(0), fsr.render_size), 0).x;
    return select(0.0, fsr.composition_on, x > 1.5);
}

@fragment
fn fs_prepare_reactivity(in: VsOut) -> ReactivityOut {
    let p = px_of(in);
    let uv = (vec2<f32>(p) + 0.5) / render_size();
    let mv = textureLoad(dilated_motion_tex, p, 0).xy;
    let depth = textureLoad(dilated_depth_tex, p, 0).x;
    let disocclusion = compute_disocclusions(uv, mv, view_space_depth(depth));
    let shading_change = max(dilate_reactive_masks(p), compute_shading_change(uv));
    let reactive = max(compute_motion_divergence(uv, mv, depth), composition_mask(p));
    let accumulation = update_accumulation(p, uv, mv, disocclusion, shading_change);
    let lock = thin_feature_confidence(p);
    if (lock > 1.0 / 100.0) {
        let hr = compute_hr_pos_from_lr_pos(p);
        if (is_on_screen(hr, fsr.upscale_size)) {
            new_locks[hr.y * fsr.upscale_size.x + hr.x] = lock;
        }
    }
    var o: ReactivityOut;
    o.masks = vec4<f32>(reactive, disocclusion, shading_change, accumulation.x);
    o.accumulation = vec4<f32>(accumulation.y, 0.0, 0.0, 0.0);
    return o;
}
