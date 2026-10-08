// Glow blur pass, ported from the Second Life / Firestorm viewer sources
// (LGPL 2.1, Copyright (C) 2007-2024 Linden Research, Inc. and the
// Firestorm project): effects/glowV.glsl and effects/glowF.glsl, driven by
// LLPipeline::generateGlow (RenderGlowIterations x 2 separable passes,
// alternating horizontal / vertical, on 512x512 8-bit targets).

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct GlowParams {
    delta: vec2<f32>,  // glowDelta: RenderGlowWidth / 512 on one axis
    strength: f32,     // RenderGlowStrength
    _pad: f32,
};
@group(0) @binding(2) var<uniform> glow: GlowParams;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_glow(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

fn tap(uv: vec2<f32>, k: f32) -> vec4<f32> {
    return k * textureSampleLevel(src, src_sampler, uv, 0.0);
}

@fragment
fn fs_glow_blur(in: Out) -> @location(0) vec4<f32> {
    let d = glow.delta;
    var col = tap(in.uv + d * -3.5, 0.25);
    col += tap(in.uv + d * -2.5, 0.5);
    col += tap(in.uv + d * -1.5, 0.8);
    col += tap(in.uv + d * -0.5, 1.0);
    col += tap(in.uv + d * 0.5, 1.0);
    col += tap(in.uv + d * 1.5, 0.8);
    col += tap(in.uv + d * 2.5, 0.5);
    col += tap(in.uv + d * 3.5, 0.25);
    return max(vec4<f32>(col.rgb * glow.strength, col.a), vec4<f32>(0.0));
}
