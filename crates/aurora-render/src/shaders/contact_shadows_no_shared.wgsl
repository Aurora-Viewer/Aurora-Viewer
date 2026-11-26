// Appended to contact_shadows.wgsl for `AURORA_CONTACT_VARIANT=no-shared`
// (contact_shadows.rs, `variant_source`), a diagnostic of the GPU hangs on
// some AMD cards: no workgroup memory, no barrier. Each thread recomputes
// from the depth texture what the thread `index % WAVE_SIZE` of its
// wavefront would have shared for its read `index / WAVE_SIZE`, with the
// same arithmetic, so the mask stays the same.

struct Wave {
    start_xy: vec2<f32>,
    end_xy: vec2<f32>,
    xy_delta: vec2<f32>,
    x_axis_major: bool,
    reverse_direction: bool,
    major_axis_start: f32,
    ma_light_frac: f32,
    direction: f32,
    z_sign: f32,
};

fn stored_depth(w: Wave, index: i32) -> f32 {
    let i = index / WAVE_SIZE;
    let tid = u32(index % WAVE_SIZE);
    let thread_step = f32(tid ^ select(u32(WAVE_SIZE - 1), 0u, w.reverse_direction));
    var pixel_xy = mix(w.start_xy, w.end_xy, thread_step / f32(WAVE_SIZE));
    for (var k = 0; k < i; k++) {
        pixel_xy += w.xy_delta * w.direction;
    }
    let pixel_distance = w.major_axis_start - thread_step + w.ma_light_frac;
    let read_xy = vec2<i32>(floor(pixel_xy));
    let minor_axis = select(pixel_xy.x, pixel_xy.y, w.x_axis_major);
    let bilinear = fract(minor_axis) - 0.5;
    let bias = select(-1, 1, bilinear > 0.0);
    let offset_xy = select(vec2<i32>(bias, 0), vec2<i32>(0, bias), w.x_axis_major);
    let d0 = load_depth(read_xy);
    let d1 = load_depth(read_xy + offset_xy);
    let use_point_filter = abs(d0 - d1) > abs(FAR_DEPTH - d0) * params.tuning.y;
    let shadow_depth = d0 + abs(d0 - d1) * w.z_sign;
    let shadowing_depth = select(shadow_depth, d0, use_point_filter);
    let sample_distance = pixel_distance + f32(WAVE_SIZE * i) * w.direction;
    var stored = (shadowing_depth - params.light.z) / sample_distance;
    if (i != 0) {
        stored = select(1e10, stored, sample_distance > 0.0);
    }
    return stored;
}
