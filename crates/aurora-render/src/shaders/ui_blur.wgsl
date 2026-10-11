// Separable Gaussian with paired linear samples. No tint or dimming.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
@group(0) @binding(2) var<uniform> direction: vec4<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs(@builtin(vertex_index) i: u32) -> Vertex {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Vertex;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment fn fs(v: Vertex) -> @location(0) vec4<f32> {
    let d = direction.xy;
    var color = textureSample(source, linear, v.uv) * 0.2270270270;
    color += textureSample(source, linear, v.uv + d * 1.3846153846) * 0.3162162162;
    color += textureSample(source, linear, v.uv - d * 1.3846153846) * 0.3162162162;
    color += textureSample(source, linear, v.uv + d * 3.2307692308) * 0.0702702703;
    color += textureSample(source, linear, v.uv - d * 3.2307692308) * 0.0702702703;
    return color;
}
