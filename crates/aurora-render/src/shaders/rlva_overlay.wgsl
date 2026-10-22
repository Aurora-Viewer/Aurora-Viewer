enable wgpu_binding_array;
struct Overlay { color: vec4<f32>, texture: vec4<u32> };
@group(0) @binding(0) var<storage, read> overlays: array<Overlay>;
@group(1) @binding(0) var pages: binding_array<texture_2d_array<f32>>;
@group(1) @binding(1) var image_sampler: sampler;
@group(1) @binding(2) var<storage, read> locations: array<u32>;
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) slot: u32,
};
@vertex fn vs_main(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> Vertex {
    let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var v: Vertex;
    v.position = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    v.uv = vec2<f32>(p.x, 1.0 - p.y);
    v.color = overlays[instance].color;
    v.slot = overlays[instance].texture.x;
    return v;
}
@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let loc = locations[v.slot];
    return textureSample(pages[loc >> 16u], image_sampler, v.uv, i32(loc & 65535u)) * v.color;
}
