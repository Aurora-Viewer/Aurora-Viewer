struct Params {
    color: vec4<f32>,
    pulse: f32,
    output_srgb: f32,
    scale: f32,
    pad: f32,
};
@group(0) @binding(0) var coverage: texture_2d<f32>;
@group(0) @binding(1) var horizontal: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn mask_at(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(coverage));
    if (any(p < vec2<i32>(0)) || any(p >= size)) { return 0.0; }
    return textureLoad(coverage, p, 0).r;
}

// A separable weighted maximum yields a smooth exterior band. Sample each
// physical pixel even at high DPI: scaled offsets would skip thin islands.
@fragment
fn fs_spread(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    var halo = 0.0;
    let radius = i32(ceil(5.0 * params.scale));
    for (var i = -radius; i <= radius; i++) {
        let distance = f32(i) / params.scale;
        halo = max(halo, mask_at(p + vec2<i32>(i, 0)) * exp(-0.10 * distance * distance));
    }
    return vec4<f32>(halo, 0.0, 0.0, 1.0);
}

fn output_color(linear: vec3<f32>) -> vec3<f32> {
    if (params.output_srgb > 0.5) { return linear; }
    let low = linear * 12.92;
    let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

@fragment
fn fs_composite(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pixel.xy);
    let inside = mask_at(p);
    if (inside > 0.5) {
        if (params.pulse <= 0.0) { discard; }
        return vec4<f32>(output_color(params.color.rgb), params.pulse * params.color.a);
    }
    let size = vec2<i32>(textureDimensions(horizontal));
    var halo = 0.0;
    let radius = i32(ceil(5.0 * params.scale));
    for (var i = -radius; i <= radius; i++) {
        let distance = f32(i) / params.scale;
        let q = p + vec2<i32>(0, i);
        if (all(q >= vec2<i32>(0)) && all(q < size)) {
            halo = max(halo, textureLoad(horizontal, q, 0).r * exp(-0.10 * distance * distance));
        }
    }
    if (halo <= 0.01) { discard; }
    // A narrow dark separator keeps the violet band visible against light
    // or violet materials; all these pixels are OUTSIDE selected coverage.
    let separator = smoothstep(0.78, 0.90, halo);
    let color = params.color.rgb * mix(1.0, 0.12, separator);
    return vec4<f32>(output_color(color), min(halo * 1.35, 0.92) * params.color.a);
}
