// Premultiplied sRGB, like egui's vertex colors and native image textures.
struct Vertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex fn vs_mesh(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> Vertex {
    var out: Vertex;
    out.clip = vec4(position, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment fn fs_mesh(v: Vertex) -> @location(0) vec4<f32> { return v.color; }

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> options: vec4<u32>;

@vertex fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(positions[index], 0.0, 1.0);
}

@fragment fn fs_resolve(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    let factor = options.x;
    let origin = vec2<i32>(pixel.xy) * i32(factor);
    var sum = vec4<f32>(0.0);
    for (var y = 0u; y < factor; y++) {
        for (var x = 0u; x < factor; x++) {
            sum += textureLoad(source, origin + vec2<i32>(i32(x), i32(y)), 0);
        }
    }
    // Filter RGB together with alpha. Averaging straight alpha would add
    // dark fringes around the silhouette and the inside of a torus hole.
    return sum / f32(factor * factor);
}
