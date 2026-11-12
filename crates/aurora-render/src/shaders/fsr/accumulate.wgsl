// FSR 3.1 luma instability (ffx_fsr3upscaler_luma_instability.h), the
// accumulation (ffx_fsr3upscaler_accumulate.h, _upsample.h, _reproject.h)
// and RCAS (ffx_fsr3upscaler_rcas.h, fsr1/ffx_fsr1.h FsrRcasF).

// fs_luma_instability: 2 dilated motion, 3 dilated reactive masks,
// 4 current luma, 5 last frame's luma history, 6 farthest depth (half)
// fs_accumulate: 2 dilated motion, 3 dilated reactive masks, 4 input
// colour, 5 last frame's internal colour, 6 farthest depth (half),
// 7 luma instability, 8 new locks
// fs_rcas: 4 input colour (its glow alpha), 5 this frame's internal colour
@group(0) @binding(2) var dilated_motion_tex: texture_2d<f32>;
@group(0) @binding(3) var masks_tex: texture_2d<f32>;
@group(0) @binding(4) var t4: texture_2d<f32>;
@group(0) @binding(5) var t5: texture_2d<f32>;
@group(0) @binding(6) var farthest_mip1_tex: texture_2d<f32>;
@group(0) @binding(7) var luma_instability_tex: texture_2d<f32>;
@group(0) @binding(8) var<storage, read_write> new_locks: array<f32>;

fn farthest_depth_mip1(uv: vec2<f32>) -> f32 {
    let fu = clamp_uv(uv, fsr.render_size / 2, vec2<i32>(textureDimensions(farthest_mip1_tex)));
    return textureSampleLevel(farthest_mip1_tex, lin, fu, 0.0).x;
}

// ---- luma instability

struct InstabilityOut {
    @location(0) luma_history: vec4<f32>,
    @location(1) instability: vec4<f32>,
};

@fragment
fn fs_luma_instability(in: VsOut) -> InstabilityOut {
    let p = px_of(in);
    var history = vec4<f32>(0.0);
    var instability = 0.0;
    let mv = textureLoad(dilated_motion_tex, p, 0).xy;
    let uv = (vec2<f32>(p) + 0.5) / render_size();
    let uv_cur_jittered = uv + fsr.jitter / render_size();
    let uv_prev_jittered = uv + fsr.prev_jitter / vec2<f32>(fsr.prev_render_size);
    let reprojected = uv_prev_jittered + mv;
    if (is_uv_inside(reprojected)) {
        let masks = textureSampleLevel(masks_tex, lin, clamp_uv(uv_cur_jittered, fsr.render_size, fsr.max_render_size), 0.0);
        let reactive = sat(masks[REACTIVE]);
        let disocclusion = sat(masks[DISOCCLUSION]);
        let shading_change = sat(masks[SHADING_CHANGE]);
        let accumulation = sat(masks[ACCUMULATION]);
        if (accumulation > 0.9) {
            let current = textureSampleLevel(t4, lin, clamp_uv(uv_cur_jittered, fsr.render_size, fsr.max_render_size), 0.0).x
                * fsr.exposure;
            history = textureSampleLevel(t5, lin, clamp_uv(reprojected, fsr.prev_render_size, fsr.max_render_size), 0.0)
                * fsr.delta_pre_exposure * fsr.exposure;
            // ComputeLumaInstabilityFactor: N-1 to N-4 in x to w
            let diffs0 = current - history.x;
            let similarity0 = min_divided_by_max_or(current, history.x, 1.0);
            var max_similarity = similarity0;
            var unstable = 0.0;
            if (similarity0 < 1.0) {
                for (var i = 1; i <= 3; i++) {
                    let diffs1 = current - history[i];
                    let similarity1 = min_divided_by_max(current, history[i]);
                    if (sign(diffs0) == sign(diffs1)) {
                        max_similarity = max(max_similarity, similarity1);
                    }
                }
                unstable = f32(max_similarity > similarity0);
            }
            history = vec4<f32>(current, history.x, history.y, history.z) / fsr.exposure;
            instability = unstable * f32(history.w != 0.0);
            let velocity_weight = 1.0 - sat(get_4k_velocity(mv) / 20.0);
            instability *= velocity_weight * (1.0 - disocclusion) * (1.0 - reactive) * (1.0 - shading_change);
        }
    }
    var o: InstabilityOut;
    o.luma_history = history;
    o.instability = vec4<f32>(instability, 0.0, 0.0, 0.0);
    return o;
}

// ---- accumulation

struct Params {
    hr_pos: vec2<i32>,
    hr_uv: vec2<f32>,
    lr_uv_jittered: vec2<f32>,
    lr_uv_hw: vec2<f32>,
    mv: vec2<f32>,
    reprojected_hr_uv: vec2<f32>,
    velocity_4k: f32,
    disocclusion: f32,
    reactive: f32,
    shading_change: f32,
    accumulation: f32,
    luma_instability: f32,
    farthest_depth_m: f32,
    existing_sample: bool,
    new_sample: bool,
};

struct Data {
    clip: RectificationBox,
    upsampled_color: vec3<f32>,
    upsampled_weight: f32,
    history_color: vec3<f32>,
    history_weight: f32,
    lock: f32,
    lock_contribution: f32,
};

// the history's texel, its coordinates held one texel inside the edges
// (ffx_fsr3upscaler_sample.h ClampCoord)
fn history_texel(p: vec2<i32>, offset: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let q = clamp(p + offset, vec2<i32>(1), size - vec2<i32>(2));
    return textureLoad(t5, q, 0);
}

fn lanczos2_4(c0: vec4<f32>, c1: vec4<f32>, c2: vec4<f32>, c3: vec4<f32>, t: f32) -> vec4<f32> {
    let w0 = lanczos2(-1.0 - t);
    let w1 = lanczos2(-0.0 - t);
    let w2 = lanczos2(1.0 - t);
    let w3 = lanczos2(2.0 - t);
    return (w0 * c0 + w1 * c1 + w2 * c2 + w3 * c3) / (w0 + w1 + w2 + w3);
}

// HistorySample: 4 x 4 Lanczos 2, deringed by the 2 x 2 in the middle
fn history_sample(uv: vec2<f32>, size: vec2<i32>) -> vec4<f32> {
    var pxs = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let frac = fract(pxs);
    pxs = clamp(pxs, vec2<f32>(0.0), vec2<f32>(size - vec2<i32>(1)));
    let base = vec2<i32>(floor(pxs));
    var rows: array<vec4<f32>, 4>;
    for (var y = 0; y < 4; y++) {
        rows[y] = lanczos2_4(
            history_texel(base, vec2<i32>(-1, y - 1), size),
            history_texel(base, vec2<i32>(0, y - 1), size),
            history_texel(base, vec2<i32>(1, y - 1), size),
            history_texel(base, vec2<i32>(2, y - 1), size),
            frac.x,
        );
    }
    let c = lanczos2_4(rows[0], rows[1], rows[2], rows[3], frac.y);
    let d11 = history_texel(base, vec2<i32>(0, 0), size);
    let d21 = history_texel(base, vec2<i32>(1, 0), size);
    let d12 = history_texel(base, vec2<i32>(0, 1), size);
    let d22 = history_texel(base, vec2<i32>(1, 1), size);
    let mn = min(min(d11, d21), min(d12, d22));
    let mx = max(max(d11, d21), max(d12, d22));
    return clamp(c, mn, mx);
}

fn reproject_history_color(params: Params, data: ptr<function, Data>) {
    let h = history_sample(params.reprojected_hr_uv, fsr.prev_upscale_size);
    (*data).history_color = rgb_to_ycocg(h.rgb * fsr.delta_pre_exposure * fsr.exposure);
    (*data).lock = h.w;
}

fn update_lock_status(params: Params, data: ptr<function, Data>) {
    (*data).lock *= f32(!params.new_sample);
    let lifetime_decrease_factor = max(sat(params.shading_change), max(params.reactive, params.disocclusion));
    (*data).lock = max(0.0, (*data).lock - lifetime_decrease_factor * LOCK_MAX);
    (*data).lock_contribution = sat(sat((*data).lock - LOCK_THRESHOLD) * (LOCK_MAX - LOCK_THRESHOLD));
    let i = params.hr_pos.y * fsr.upscale_size.x + params.hr_pos.x;
    let new_lock = new_locks[i] * (1.0 - max(params.shading_change * 0.0, params.reactive));
    (*data).lock = max(0.0, min((*data).lock + new_lock, LOCK_MAX));
    // ready for the next frame
    let lifetime_decrease = (0.1 / fsr.jitter_sequence_length) * (1.0 - lifetime_decrease_factor);
    (*data).lock = max(0.0, (*data).lock - lifetime_decrease);
    // a lock leaving the screen would hold on its edge
    (*data).lock *= f32(is_uv_inside(params.hr_uv - params.mv));
}

fn compute_base_accumulation_weight(params: Params, data: ptr<function, Data>) {
    var base = params.accumulation;
    base = min(base, mix(base, 0.15, sat(max(0.0, (params.velocity_4k * fsr.velocity_factor) / 0.5))));
    (*data).history_weight = base;
}

fn prepared_color(p: vec2<i32>) -> vec3<f32> {
    let c = max(vec3<f32>(0.0), safe_color(textureLoad(t4, p, 0).rgb)) * fsr.exposure;
    return rgb_to_ycocg(c);
}

fn upsample_lanczos_weight(offset: vec2<f32>, kernel_weight: f32) -> f32 {
    let biased = offset * kernel_weight;
    return lanczos2_approx_sq(dot(biased, biased));
}

fn compute_upsampled_color_and_weight(params: Params, data: ptr<function, Data>) {
    // a sliced 2-lobe Lanczos (the other slices come over time)
    let dst = vec2<f32>(params.hr_pos) + 0.5;
    let src = dst * fsr.downscale;
    let src_input = vec2<i32>(floor(src));
    let src_unjittered = vec2<f32>(src_input) + 0.5 - fsr.jitter;
    let base_offset = src_unjittered - src;
    let flip_col = src_unjittered.x > src.x;
    let flip_row = src_unjittered.y > src.y;
    let offset_tl = vec2<i32>(select(-1, -2, flip_col), select(-1, -2, flip_row));
    let initial = params.accumulation == 0.0;
    var samples: array<vec3<f32>, 9>;
    var idx = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let cr = vec2<i32>(select(col, 3 - col, flip_col), select(row, 3 - row, flip_row));
            let q = clamp_load(src_input + offset_tl + cr, vec2<i32>(0), fsr.render_size);
            var s = prepared_color(q);
            if (initial) {
                s = rgb_to_ycocg(tonemap(ycocg_to_rgb(s)));
            }
            samples[idx] = s;
            idx++;
        }
    }
    let kernel_bias_max = min(1.99, 1.0 + (1.0 / fsr.downscale - 1.0).x);
    let kernel_bias_min = max(1.0, (1.0 + kernel_bias_max) * 0.3);
    let kernel_bias_weight = min(1.0 - params.disocclusion * 0.5, min(1.0 - params.shading_change, sat((*data).history_weight * 5.0)));
    let kernel_bias = mix(kernel_bias_min, kernel_bias_max, kernel_bias_weight);
    // the box in a local: naga 30's SPIR-V writer loses a pointer to a
    // member of a pointed-to struct passed on to a call
    var clip: RectificationBox;
    idx = 0;
    for (var row = 0; row < 3; row++) {
        for (var col = 0; col < 3; col++) {
            let cr = vec2<i32>(select(col, 3 - col, flip_col), select(row, 3 - row, flip_row));
            let offset = vec2<f32>(offset_tl) + vec2<f32>(cr);
            let sample_offset = base_offset + offset;
            let q = src_input + offset_tl + cr;
            let on_screen = f32(is_on_screen(q, fsr.render_size));
            if (!initial) {
                let w = on_screen * upsample_lanczos_weight(sample_offset, kernel_bias);
                (*data).upsampled_color += samples[idx] * w;
                (*data).upsampled_weight += w;
            }
            let rectification_curve_bias = -2.3;
            let box_weight = exp(rectification_curve_bias * dot(sample_offset, sample_offset)) * on_screen;
            box_add_sample(row == 0 && col == 0, &clip, samples[idx], box_weight);
            idx++;
        }
    }
    box_compute_variance(&clip);
    (*data).clip = clip;
    (*data).upsampled_weight *= f32((*data).upsampled_weight > EPSILON);
    if ((*data).upsampled_weight > EPSILON) {
        // normalized for the deringing (colours are compared)
        (*data).upsampled_color /= (*data).upsampled_weight;
        (*data).upsampled_weight *= AVERAGE_LANCZOS_WEIGHT_PER_FRAME;
        (*data).upsampled_color = clamp((*data).upsampled_color, (*data).clip.aabb_min, (*data).clip.aabb_max);
    }
    // the first samples from the tone mapped upsampling
    if (initial) {
        (*data).upsampled_color = rgb_to_ycocg(inverse_tonemap(ycocg_to_rgb((*data).clip.center)));
        (*data).upsampled_weight = 1.0;
        (*data).history_weight = 0.0;
    }
}

fn rectify_history(params: Params, data: ptr<function, Data>) {
    let velocity_factor = sat(params.velocity_4k / 20.0);
    let distance_factor = sat(0.75 - params.farthest_depth_m / 20.0);
    let accumulation_factor = 1.0 - params.accumulation;
    let reactive_factor = pow(params.reactive, 1.0 / 2.0);
    let t = max(velocity_factor, max(distance_factor, max(accumulation_factor, max(reactive_factor, params.shading_change))));
    let box_scale = mix(3.0, 1.0, t);
    let scaled = (*data).clip.vec * vec3<f32>(1.7, 1.0, 1.0) * box_scale;
    let clamped_scaled = max(scaled, vec3<f32>(1.193e-7));
    let transformed = ((*data).history_color - (*data).clip.center) / clamped_scaled;
    if (length(transformed) > 1.0) {
        let final_clamped = normalize(transformed) * scaled + (*data).clip.center;
        // the accumulation guards against invalid history colours
        let contribution = max(params.luma_instability, (*data).lock_contribution) * params.accumulation * (1.0 - params.disocclusion);
        (*data).history_color = mix(final_clamped, (*data).history_color, sat(contribution));
    }
}

fn accumulate(data: ptr<function, Data>) {
    // no invalid values when the accumulation and the upsampled weight are 0
    (*data).history_weight *= f32((*data).history_weight > FP16_MIN);
    (*data).history_weight = max(EPSILON, (*data).history_weight + (*data).upsampled_weight);
    // YCoCg -> RGB -> tone map -> YCoCg: the RGB tone mapper keeps the colour
    (*data).upsampled_color = rgb_to_ycocg(tonemap(ycocg_to_rgb((*data).upsampled_color)));
    (*data).history_color = rgb_to_ycocg(tonemap(ycocg_to_rgb((*data).history_color)));
    let alpha = sat((*data).upsampled_weight / (*data).history_weight);
    (*data).history_color = ycocg_to_rgb(mix((*data).history_color, (*data).upsampled_color, alpha));
    (*data).history_color = inverse_tonemap((*data).history_color);
}

struct AccumulateOut {
    // the next frame's history: colour and lock
    @location(0) internal: vec4<f32>,
    // the image, with the glow amount of this frame in alpha (taa.wgsl)
    @location(1) color: vec4<f32>,
};

@fragment
fn fs_accumulate(in: VsOut) -> AccumulateOut {
    let p = px_of(in);
    var params: Params;
    params.hr_pos = p;
    params.hr_uv = (vec2<f32>(p) + 0.5) / vec2<f32>(fsr.upscale_size);
    params.lr_uv_jittered = params.hr_uv + fsr.jitter / render_size();
    params.lr_uv_hw = clamp_uv(params.lr_uv_jittered, fsr.render_size, fsr.max_render_size);
    let mv_at = clamp_load(vec2<i32>(params.hr_uv * render_size()), vec2<i32>(0), fsr.render_size);
    params.mv = textureLoad(dilated_motion_tex, mv_at, 0).xy;
    params.velocity_4k = get_4k_velocity(params.mv);
    params.reprojected_hr_uv = params.hr_uv + params.mv;
    params.existing_sample = is_uv_inside(params.reprojected_hr_uv);
    params.luma_instability = textureSampleLevel(luma_instability_tex, lin, clamp_uv(params.hr_uv, fsr.render_size, fsr.max_render_size), 0.0).x;
    params.farthest_depth_m = farthest_depth_mip1(params.lr_uv_jittered);
    params.new_sample = !params.existing_sample || fsr.frame_index == 0.0;
    let masks = textureSampleLevel(masks_tex, lin, params.lr_uv_hw, 0.0);
    params.reactive = sat(masks[REACTIVE]);
    params.disocclusion = sat(masks[DISOCCLUSION]);
    params.shading_change = sat(masks[SHADING_CHANGE]);
    params.accumulation = sat(masks[ACCUMULATION]);
    params.accumulation *= f32(round(params.accumulation * 100.0) > 1.0);

    var data: Data;
    data.upsampled_color = vec3<f32>(0.0);
    data.history_color = vec3<f32>(0.0);
    data.history_weight = 1.0;
    data.upsampled_weight = 0.0;
    data.lock = 0.0;
    data.lock_contribution = 0.0;

    if (params.existing_sample && !params.new_sample) {
        reproject_history_color(params, &data);
    }
    update_lock_status(params, &data);
    compute_base_accumulation_weight(params, &data);
    compute_upsampled_color_and_weight(params, &data);
    rectify_history(params, &data);
    accumulate(&data);
    let color = max(data.history_color / fsr.exposure, vec3<f32>(0.0));
    // the locks are new again next frame
    new_locks[p.y * fsr.upscale_size.x + p.x] = 0.0;
    let glow = textureLoad(t4, clamp_load(vec2<i32>(vec2<f32>(p) * fsr.downscale), vec2<i32>(0), fsr.render_size), 0).a;
    var o: AccumulateOut;
    o.internal = vec4<f32>(color, data.lock);
    o.color = vec4<f32>(color, glow);
    return o;
}

// ---- RCAS: robust contrast adaptive sharpening of the internal colour.
// Its neighbours are held inside the image, where the SDK's loads give 0
// (a darker border).

const RCAS_LIMIT: f32 = 0.25 - 1.0 / 16.0;

fn rcas_load(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(t5, clamp_load(p, vec2<i32>(0), fsr.upscale_size), 0).rgb * fsr.exposure;
}

fn max3(a: f32, b: f32, c: f32) -> f32 {
    return max(a, max(b, c));
}

fn min3(a: f32, b: f32, c: f32) -> f32 {
    return min(a, min(b, c));
}

@fragment
fn fs_rcas(in: VsOut) -> @location(0) vec4<f32> {
    let p = px_of(in);
    //    b
    //  d e f
    //    h
    let b = rcas_load(p + vec2<i32>(0, -1));
    let d = rcas_load(p + vec2<i32>(-1, 0));
    let e = rcas_load(p);
    let f = rcas_load(p + vec2<i32>(1, 0));
    let h = rcas_load(p + vec2<i32>(0, 1));
    // luma times 2
    let bl = b.b * 0.5 + (b.r * 0.5 + b.g);
    let dl = d.b * 0.5 + (d.r * 0.5 + d.g);
    let el = e.b * 0.5 + (e.r * 0.5 + e.g);
    let fl = f.b * 0.5 + (f.r * 0.5 + f.g);
    let hl = h.b * 0.5 + (h.r * 0.5 + h.g);
    // noise detection
    var nz = 0.25 * bl + 0.25 * dl + 0.25 * fl + 0.25 * hl - el;
    nz = sat(abs(nz) / (max3(max3(bl, dl, el), fl, hl) - min3(min3(bl, dl, el), fl, hl)));
    nz = -0.5 * nz + 1.0;
    // min and max of the ring
    let mn4 = min(min(min(b, d), f), h);
    let mx4 = max(max(max(b, d), f), h);
    let peak = vec2<f32>(1.0, -4.0);
    let hit_min = mn4 / (4.0 * mx4);
    let hit_max = (peak.x - mx4) / (4.0 * mn4 + peak.y);
    let lobe3 = max(-hit_min, hit_max);
    var lobe = max(-RCAS_LIMIT, min(max3(lobe3.r, lobe3.g, lobe3.b), 0.0)) * fsr.rcas;
    lobe *= nz;
    let rcp_l = 1.0 / (4.0 * lobe + 1.0);
    let c = (lobe * b + lobe * d + lobe * h + lobe * f + e) * rcp_l / fsr.exposure;
    let glow = textureLoad(t4, clamp_load(vec2<i32>(vec2<f32>(p) * fsr.downscale), vec2<i32>(0), fsr.render_size), 0).a;
    return vec4<f32>(c, glow);
}
