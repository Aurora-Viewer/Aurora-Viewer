// Depth-based stochastic transparency (oit_stochastic.rs): the opaque depth
// of the scene's samples spread over the stochastic passes' own, more of
// them per pixel (8 per pass, the scene at 4x). Each pass sample
// takes the depth of the scene sample nearest to it in the pixel (the
// standard sample positions, `expand_map`), so that a transparent fragment
// is culled by the opaque surface that covers that part of the pixel. A
// scene without MSAA (one sample) gives its depth to every pass sample
// (`fs_expand_ss`).

struct ExpandParams {
    // the scene sample of each pass sample, four to a vector
    map: array<vec4<u32>, 2>,
    // the target's size in pixels
    size: vec2<u32>,
    // 1: one triangle over the whole view, not the listed tiles
    // (oit_stochastic.rs `Stochastic::whole`)
    whole: u32,
};
@group(0) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(0) @binding(1) var<uniform> ep: ExpandParams;
@group(0) @binding(2) var scene_depth_ss: texture_depth_2d;
// the tiles of 8 x 8 pixels that hold a blended face (oit_stochastic.wgsl
// `cs_tiles`); the first 8 words are the indirect arguments
@group(0) @binding(3) var<storage, read> tile_list: array<u32>;

// The listed tiles, a quad each (two triangles, 6 vertices an instance): the
// passes after draw nowhere else.
@vertex
fn vs_tile(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> @builtin(position) vec4<f32> {
    if (ep.whole != 0u) {
        let uv = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
        return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    }
    let t = tile_list[8u + i];
    var corners = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let c = corners[v];
    let at = vec2<f32>(f32(((t & 0xffffu) + (c & 1u)) * 8u), f32(((t >> 16u) + (c >> 1u)) * 8u));
    let size = vec2<f32>(ep.size);
    return vec4<f32>(at.x / size.x * 2.0 - 1.0, 1.0 - at.y / size.y * 2.0, 0.0, 1.0);
}

// One invocation per pass sample (`sample_index`); s < 8 (the pipeline's
// count is at most 8, `MAX_PASS_SAMPLES`).
@fragment
fn fs_expand(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @builtin(frag_depth) f32 {
    let i = min(s, 7u);
    let k = ep.map[i / 4u][i % 4u];
    return textureLoad(scene_depth, vec2<i32>(p.xy), i32(k));
}

// The single-sampled scene depth on every pass sample.
@fragment
fn fs_expand_ss(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(scene_depth_ss, vec2<i32>(p.xy), 0);
}
