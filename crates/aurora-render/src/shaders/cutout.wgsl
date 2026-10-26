// A cut-out photo (cutout.rs): after the opaque and masked faces, the
// scene's alpha takes what shows through each sample, 1 where nothing was
// drawn (reverse Z: the depth is still its clear value) and 0 elsewhere.
// The blended faces then multiply it by what they let through (their
// alpha factors, renderer.rs `suppress`), so that after them 1 - alpha is
// the coverage, partial at a hair strand or a veil.

@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var depth_ms: texture_depth_multisampled_2d;

@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn through(d: f32) -> vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, select(0.0, 1.0, d <= 0.0));
}

@fragment
fn fs_single(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return through(textureLoad(depth, vec2<i32>(p.xy), 0));
}

// per sample (the sample index): the resolve averages what shows through
@fragment
fn fs_msaa(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    return through(textureLoad(depth_ms, vec2<i32>(p.xy), i32(s)));
}
