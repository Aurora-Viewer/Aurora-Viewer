// Image-space reflections and directional optics. Sizes are in fractions of
// the short side of the full frame, independent of texture size or tiling.
struct Optics {
    tint: vec4<f32>,
    ghost: vec4<f32>,
    halo: vec4<f32>,
    streak: vec4<f32>,
    star: vec4<f32>,
    geometry: vec4<f32>,
};
@group(0) @binding(0) var highlights: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;
@group(0) @binding(2) var<uniform> optics: Optics;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) ray: u32,
};

@vertex
fn vs_flare(@builtin(vertex_index) index: u32, @builtin(instance_index) ray: u32) -> VertexOut {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return VertexOut(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(p.x, 1.0 - p.y), ray);
}

fn source(uv: vec2<f32>) -> vec3<f32> {
    let to_edge = min(uv, 1.0 - uv);
    let edge = smoothstep(0.0, 0.045, min(to_edge.x, to_edge.y));
    return textureSampleLevel(highlights, linear_clamp, uv, 0.0).rgb * edge;
}

fn color_sample(uv: vec2<f32>, offset: vec2<f32>) -> vec3<f32> {
    let c = source(uv);
    if (optics.halo.w == 0.0) {
        return c;
    }
    let split = offset * optics.halo.w;
    return vec3<f32>(source(uv - split).r, c.g, source(uv + split).b);
}

fn soft_source(uv: vec2<f32>, separation: vec2<f32>, softness: f32) -> vec3<f32> {
    let c = color_sample(uv, separation);
    if (softness == 0.0) {
        return c;
    }
    let offset = optics.geometry.xy * (softness * 0.018);
    let x = vec2<f32>(offset.x, 0.0);
    let y = vec2<f32>(0.0, offset.y);
    return c * 0.40 + 0.15 * (source(uv + x) + source(uv - x) + source(uv + y) + source(uv - y));
}

fn ray_source(uv: vec2<f32>, first: bool) -> vec3<f32> {
    if (first) {
        return source(uv);
    }
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(highlights, linear_clamp, uv, 0.0).rgb;
}

fn ray_filter(uv: vec2<f32>, step: vec2<f32>, first: bool, weights: vec4<f32>) -> vec3<f32> {
    let a = ray_source(uv - step, first) + ray_source(uv + step, first);
    let b = ray_source(uv - 2.0 * step, first) + ray_source(uv + 2.0 * step, first);
    let c = ray_source(uv - 3.0 * step, first) + ray_source(uv + 3.0 * step, first);
    return weights.x * ray_source(uv, first) + weights.y * a + weights.z * b + weights.w * c;
}

@fragment
fn fs_flare_ray(in: VertexOut) -> @location(0) vec4<f32> {
    let axis = in.ray >> 8u;
    let levels = (in.ray >> 4u) & 15u;
    let stage = in.ray & 15u;
    let axes = optics.star.w * 0.5;
    let is_streak = axis == 0u;
    let angle = select(optics.star.z + (f32(axis) - 1.0) * 3.14159265 / axes, optics.streak.z, is_streak);
    let radius = select(optics.star.y, optics.streak.y, is_streak);
    let direction = vec2<f32>(cos(angle), sin(angle));
    let stride = f32(1u << (2u * stage)) / f32((1u << (2u * levels)) - 1u);
    let step = direction * optics.geometry.xy * (radius * stride);
    let first = stage == 0u;
    // The midpoint and starburst use the original 4:3:2:1 filter.
    // Normalize the symmetric taps so taper redistributes the streak's light.
    // Keep the inner stages triangular so neighbouring coarse taps blend smoothly.
    let taper = select(0.5, optics.streak.w, is_streak && stage + 1u == levels);
    let original = vec4<f32>(4.0, 3.0, 2.0, 1.0);
    var weights = mix(vec4<f32>(4.0), original, min(2.0 * taper, 1.0));
    weights = mix(weights, vec4<f32>(4.0, 2.0, 0.65, 0.15), max(2.0 * taper - 1.0, 0.0));
    weights /= dot(weights, vec4<f32>(1.0, 2.0, 2.0, 2.0));
    var color = ray_filter(in.uv, step, first, weights);
    if (stage + 1u == levels) {
        if (optics.halo.w > 0.0) {
            let split = vec2<f32>(-direction.y, direction.x) * optics.geometry.xy * optics.halo.w * 0.002;
            color = vec3<f32>(ray_filter(in.uv - split, step, first, weights).r, color.g, ray_filter(in.uv + split, step, first, weights).b);
        }
        let gain = select(optics.star.x * 1.8 / sqrt(axes), optics.streak.x * 2.2, is_streak);
        color *= optics.tint.rgb * optics.tint.a * gain;
    }
    // Leave range for all five rays and the ghosts to add in the HDR map.
    return vec4<f32>(min(color, vec3<f32>(8192.0)), 0.0);
}

@fragment
fn fs_flare_blur(in: VertexOut) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(highlights));
    // The blur measures the same fraction of the frame at each quality.
    let sigma = max(0.2, 0.005 * optics.geometry.z * min(size.x, size.y));
    let step = select(vec2<f32>(1.0 / size.x, 0.0), vec2<f32>(0.0, 1.0 / size.y), in.ray == 1u);
    let radius = u32(ceil(3.0 * sigma));
    var sum = textureSampleLevel(highlights, linear_clamp, in.uv, 0.0).rgb;
    var total = 1.0;
    // Consecutive Gaussian weights need only multiplications after the
    // initial exponential. Every texel of the kernel is sampled.
    let initial = exp(-0.5 / (sigma * sigma));
    let ratio_step = initial * initial;
    var weight = 1.0;
    var ratio = initial;
    for (var i = 1u; i <= radius; i++) {
        weight *= ratio;
        ratio *= ratio_step;
        let offset = step * f32(i);
        let a = textureSampleLevel(highlights, linear_clamp, in.uv - offset, 0.0).rgb;
        let b = textureSampleLevel(highlights, linear_clamp, in.uv + offset, 0.0).rgb;
        sum += (a + b) * weight;
        total += 2.0 * weight;
    }
    return vec4<f32>(sum / total, 0.0);
}

@fragment
fn fs_flare(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let center = vec2<f32>(0.5);
    let centered = (uv - center) / optics.geometry.xy;
    let distance = length(centered);
    let radial = centered / max(distance, 1e-4) * optics.geometry.xy;
    let separation = (uv - center) * 0.045;
    var light = vec3<f32>(0.0);
    if (optics.ghost.x > 0.0) {
        for (var i = 0u; i < u32(optics.ghost.y); i++) {
            let n = f32(i);
            let magnification = mix(0.30, 0.90, optics.ghost.z) * (0.72 + 0.39 * n);
            let reflected = center - (uv - center) / magnification;
            light += soft_source(reflected, separation, optics.ghost.w) * optics.ghost.x / (1.0 + 0.7 * n);
        }
    }
    if (optics.halo.x > 0.0) {
        let radius = mix(0.07, 0.68, optics.halo.y);
        let band = mix(0.10, 0.32, optics.halo.z);
        let ring = (1.0 - smoothstep(band * 0.25, band, abs(distance - radius))) * smoothstep(0.0, 0.04, distance);
        light += soft_source(center + radial * (radius - distance), separation, optics.halo.z) * ring * optics.halo.x;
    }
    return vec4<f32>(min(light * optics.tint.rgb * optics.tint.a, vec3<f32>(8192.0)), 0.0);
}
