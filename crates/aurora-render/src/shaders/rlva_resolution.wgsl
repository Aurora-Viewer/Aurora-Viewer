@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;
override spatial_upscale: bool = false;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_main(@builtin(vertex_index) index: u32) -> Vertex {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var v: Vertex;
    v.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    v.uv = vec2<f32>(p.x, 1.0 - p.y);
    return v;
}
fn cubic_weights(t: f32) -> vec4<f32> {
    return vec4<f32>(
        t * (-0.5 + t * (1.0 - 0.5 * t)),
        1.0 + t * t * (-2.5 + 1.5 * t),
        t * (0.5 + t * (2.0 - 1.5 * t)),
        t * t * (-0.5 + 0.5 * t));
}

@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene));
    if (!spatial_upscale) {
        let p = clamp(vec2<i32>(v.uv * vec2<f32>(size)), vec2<i32>(0), size - 1);
        return textureLoad(scene, p, 0);
    }
    // Catmull-Rom reconstruction: combine the two positive middle weights
    // with hardware bilinear filtering, reducing sixteen taps to nine.
    let p = v.uv * vec2<f32>(size) - 0.5;
    let base = floor(p);
    let wx = cubic_weights(fract(p.x));
    let wy = cubic_weights(fract(p.y));
    let gx = vec3<f32>(wx.x, wx.y + wx.z, wx.w);
    let gy = vec3<f32>(wy.x, wy.y + wy.z, wy.w);
    let xs = (base.x + vec3<f32>(-0.5, 0.5 + wx.z / gx.y, 2.5)) / f32(size.x);
    let ys = (base.y + vec3<f32>(-0.5, 0.5 + wy.z / gy.y, 2.5)) / f32(size.y);
    var color = vec4<f32>(0.0);
    for (var y = 0u; y < 3u; y++) {
        for (var x = 0u; x < 3u; x++) {
            color += textureSampleLevel(scene, linear_clamp, vec2<f32>(xs[x], ys[y]), 0.0) * gx[x] * gy[y];
        }
    }
    // Keep cubic lobes from creating halos or changing a flat edge color.
    let p0 = clamp(vec2<i32>(base), vec2<i32>(0), size - 1);
    let p1 = clamp(vec2<i32>(base) + vec2<i32>(1), vec2<i32>(0), size - 1);
    let a = textureLoad(scene, p0, 0);
    let b = textureLoad(scene, vec2<i32>(p1.x, p0.y), 0);
    let c = textureLoad(scene, vec2<i32>(p0.x, p1.y), 0);
    let d = textureLoad(scene, p1, 0);
    return clamp(color, min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
}
