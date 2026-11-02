// Spatial filter derived from AMD FidelityFX CAS 1.20190610, CasFilter's
// scaling path (ffx-cas/ffx_cas.h). Copyright (c) 2017-2019 AMD, Inc.
// MIT license and attribution in NOTICE.md.
@group(0) @binding(0) var scene: texture_2d<f32>;
struct Upscale { sharpness: f32 };
@group(0) @binding(1) var<uniform> upscale: Upscale;
override spatial_upscale: bool = false;
override encoded_input: bool = false;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var v: Vertex;
    v.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    v.uv = vec2<f32>(p.x, 1.0 - p.y);
    return v;
}
fn decode(c: vec3<f32>) -> vec3<f32> {
    if (!encoded_input) { return c; }
    return select(pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}
fn encode(c: vec3<f32>) -> vec3<f32> {
    if (!encoded_input) { return c; }
    return select(1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}
fn load(p: vec2<i32>) -> vec4<f32> {
    let last = vec2<i32>(textureDimensions(scene)) - 1;
    let c = textureLoad(scene, clamp(p, vec2<i32>(0), last), 0);
    return vec4<f32>(decode(c.rgb), c.a);
}
struct Cross { center: vec3<f32>, neighbors: vec3<f32>, lo: vec3<f32>, hi: vec3<f32> };
fn cross(n: vec3<f32>, w: vec3<f32>, c: vec3<f32>, e: vec3<f32>, s: vec3<f32>) -> Cross {
    return Cross(c, n + w + e + s, min(min(min(n, w), min(e, s)), c), max(max(max(n, w), max(e, s)), c));
}
@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene));
    if (!spatial_upscale) {
        let p = clamp(vec2<i32>(v.uv * vec2<f32>(size)), vec2<i32>(0), size - 1);
        return textureLoad(scene, p, 0);
    }
    let pos = v.uv * vec2<f32>(size) - 0.5;
    let p = vec2<i32>(floor(pos));
    let phase = fract(pos);
    let f = load(p);
    let g = load(p + vec2<i32>(1, 0));
    let j = load(p + vec2<i32>(0, 1));
    let k = load(p + vec2<i32>(1, 1));
    let bilinear = mix(mix(f, g, phase.x), mix(j, k, phase.x), phase.y);
    if (upscale.sharpness <= 0.001) {
        return vec4<f32>(encode(bilinear.rgb), bilinear.a);
    }
    // Four overlapping five-tap CAS kernels share twelve source texels.
    // Adaptive interpolation keeps thin edges; sharpening happens only here.
    let kernels = array<Cross, 4>(
        cross(load(p + vec2<i32>(0, -1)).rgb, load(p + vec2<i32>(-1, 0)).rgb, f.rgb, g.rgb, j.rgb),
        cross(load(p + vec2<i32>(1, -1)).rgb, f.rgb, g.rgb, load(p + vec2<i32>(2, 0)).rgb, k.rgb),
        cross(f.rgb, load(p + vec2<i32>(-1, 1)).rgb, j.rgb, k.rgb, load(p + vec2<i32>(0, 2)).rgb),
        cross(g.rgb, j.rgb, k.rgb, load(p + vec2<i32>(2, 1)).rgb, load(p + vec2<i32>(1, 2)).rgb));
    let blend = vec4<f32>((1.0 - phase.x) * (1.0 - phase.y), phase.x * (1.0 - phase.y), (1.0 - phase.x) * phase.y, phase.x * phase.y);
    let peak = -1.0 / mix(8.0, 5.0, clamp(upscale.sharpness, 0.0, 1.0));
    var sum = vec3<f32>(0.0);
    var weight = vec3<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let c = kernels[i];
        let amplitude = sqrt(clamp(min(c.lo, vec3<f32>(1.0) - c.hi) / max(c.hi, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0)));
        let w = amplitude * peak;
        let b = blend[i] / (1.0 / 32.0 + c.hi.g - c.lo.g);
        sum += (c.center + c.neighbors * w) * b;
        weight += (vec3<f32>(1.0) + 4.0 * w) * b;
    }
    return vec4<f32>(encode(clamp(sum / max(weight, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0))), bilinear.a);
}
