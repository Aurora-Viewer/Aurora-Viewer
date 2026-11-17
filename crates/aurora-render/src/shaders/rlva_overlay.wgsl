enable wgpu_binding_array;
// `texture`: x the slot; w 0 for the whole frame (RLVa's overlays), 1 for a
// rectangle whose corners y and z hold, as 16-bit fractions of the frame
// from its top left (the Director's title cards, film_readback.rs).
struct Overlay { color: vec4<f32>, texture: vec4<u32> };
@group(0) @binding(0) var<storage, read> overlays: array<Overlay>;
@group(1) @binding(0) var pages: binding_array<texture_2d_array<f32>>;
@group(1) @binding(1) var image_sampler: sampler;
// The scene's texture table (common.wgsl): a page-count header, then the
// slots' locations; a page past the bound falls back to page 0.
struct TextureLocations {
    page_count: u32,
    values: array<u32>,
}
@group(1) @binding(2) var<storage, read> locations: TextureLocations;
fn page_of(loc: u32) -> u32 {
    let page = loc >> 16u;
    let last_bound_page = max(locations.page_count, 1u) - 1u;
    return select(page, 0u, page > last_bound_page);
}
struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) slot: u32,
};
// Six vertices an overlay: the whole frame is the first three's triangle
// (the last three fall on one point), a rectangle two triangles.
@vertex fn vs_main(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> Vertex {
    let o = overlays[instance];
    var v: Vertex;
    v.color = o.color;
    v.slot = o.texture.x;
    if (o.texture.w == 0u) {
        let i = min(index, 3u);
        let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
        v.position = select(vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0), vec4<f32>(0.0, 0.0, 0.0, 1.0), index >= 3u);
        v.uv = vec2<f32>(p.x, 1.0 - p.y);
        return v;
    }
    let corner = vec2<f32>(
        select(0.0, 1.0, index == 1u || index == 4u || index == 5u),
        select(0.0, 1.0, index == 2u || index == 3u || index == 5u),
    );
    let at = mix(unpack2x16unorm(o.texture.y), unpack2x16unorm(o.texture.z), corner);
    v.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    v.uv = corner;
    return v;
}
@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let loc = locations.values[v.slot];
    return textureSample(pages[page_of(loc)], image_sampler, v.uv, i32(loc & 65535u)) * v.color;
}
