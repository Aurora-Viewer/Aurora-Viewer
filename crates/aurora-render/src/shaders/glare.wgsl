// Directional glare from the unblurred bloom prefilter. The stretched
// halo uses per-axis filtering, as in Bevy's anamorphic bloom preset.
// Radii are provided by bloom.rs, in half-resolution texels.
fn glare_sample(uv: vec2<f32>) -> vec3<f32> {
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(src, src_sampler, uv, 0.0).rgb;
}

fn directional(uv: vec2<f32>, radius: vec2<f32>, steps: i32) -> vec3<f32> {
    let delta = radius / vec2<f32>(textureDimensions(src)) / f32(steps);
    let total = f32((steps + 1) * (steps + 1));
    var sum = vec3<f32>(0.0);
    for (var i = -steps; i <= steps; i++) {
        let weight = f32(steps + 1 - abs(i)) / total;
        sum += glare_sample(uv + f32(i) * delta) * weight;
    }
    return sum;
}

@fragment
fn fs_anamorphic_x(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(directional(in.uv, vec2<f32>(HALO_RADIUS, 0.0), 8), 0.0);
}

@fragment
fn fs_anamorphic_y(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(directional(in.uv, vec2<f32>(0.0, 3.0), 3), 0.0);
}

// Wider taps are progressively band-limited horizontally to hide gaps at
// isolated highlights while keeping the streak narrow vertically.
fn streak(uv: vec2<f32>, direction: vec2<f32>, steps: i32) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    let step = direction / f32(steps);
    let total = f32((steps + 1) * (steps + 1));
    var sum = vec3<f32>(0.0);
    for (var i = -steps; i <= steps; i++) {
        let tap = uv + f32(i) * step * texel;
        let wing = step * texel * (1.0 / 3.0);
        let color = glare_sample(tap) * 0.5 + (glare_sample(tap - wing) + glare_sample(tap + wing)) * 0.25;
        sum += color * (f32(steps + 1 - abs(i)) / total);
    }
    return sum;
}

@fragment
fn fs_streaks(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(streak(in.uv, vec2<f32>(STREAK_RADIUS, 0.0), 14), 0.0);
}

@fragment
fn fs_starburst(in: Out) -> @location(0) vec4<f32> {
    let first = streak(in.uv, vec2<f32>(STAR_RADIUS, STAR_RADIUS), 12);
    let second = streak(in.uv, vec2<f32>(STAR_RADIUS, -STAR_RADIUS), 12);
    return vec4<f32>(first * 0.5 + second * 0.5, 0.0);
}

@fragment
fn fs_ghost_source(in: Out) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    var sum = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            sum += glare_sample(in.uv + vec2<f32>(f32(x), f32(y)) * texel) / 9.0;
        }
    }
    return vec4<f32>(sum, 0.0);
}
