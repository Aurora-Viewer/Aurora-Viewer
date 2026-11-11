// Depth-based stochastic transparency (oit_stochastic.rs): the opaque depth
// of the scene's samples spread over the stochastic passes' own, more of
// them per pixel (the paper's 8 per pass, the scene at 4x). Each pass sample
// takes the depth of the scene sample nearest to it in the pixel (the
// standard sample positions, `expand_map`), so that a transparent fragment
// is culled by the opaque surface that covers that part of the pixel. A
// scene without MSAA (one sample) gives its depth to every pass sample
// (`fs_expand_ss`).

struct ExpandParams {
    // the scene sample of each pass sample, four to a vector
    map: array<vec4<u32>, 4>,
};
@group(0) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(0) @binding(1) var<uniform> ep: ExpandParams;
@group(0) @binding(2) var scene_depth_ss: texture_depth_2d;

@vertex
fn vs_full(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// One invocation per pass sample (`sample_index`); s < 16 (the pipeline's
// count is at most 16, `MAX_PASS_SAMPLES`).
@fragment
fn fs_expand(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @builtin(frag_depth) f32 {
    let i = min(s, 15u);
    let k = ep.map[i / 4u][i % 4u];
    return textureLoad(scene_depth, vec2<i32>(p.xy), i32(k));
}

// The single-sampled scene depth on every pass sample.
@fragment
fn fs_expand_ss(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    return textureLoad(scene_depth_ss, vec2<i32>(p.xy), 0);
}
