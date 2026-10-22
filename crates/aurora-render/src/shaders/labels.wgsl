// What the interface paints for the world (avatar name tags, floating
// texts), hidden where the scene is in front of it (label_depth.rs).
//
// The colour is egui-wgpu's (egui.wgsl of egui-wgpu 0.36, MIT OR
// Apache-2.0): vertex colour times texture in gamma space, dithered. Added
// here: the depth each mesh stands at, and its comparison with the scene's.

struct Locals {
    // the surface in points, then in pixels
    screen_size: vec2<f32>,
    screen_px: vec2<f32>,
    // from a pixel of the surface to where the scene's depth has it (the
    // jitter of temporal anti-aliasing), in pixels
    jitter: vec2<f32>,
    dithering: u32,
    pad: u32,
}

@group(0) @binding(0) var<uniform> locals: Locals;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(1) @binding(0) var tex_color: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VertexOutput {
    @location(0) tex_coord: vec2<f32>,
    // gamma 0-1
    @location(1) color: vec4<f32>,
    // Labels keep a constant depth; world guides interpolate depth along
    // their projected segment. Their screen-space vertices all have w=1.
    @location(2) @interpolate(linear) depth: f32,
    @builtin(position) position: vec4<f32>,
}

// Jimenez 2014, "Next Generation Post-Processing in Call of Duty"
fn interleaved_gradient_noise(n: vec2<f32>) -> f32 {
    let f = 0.06711056 * n.x + 0.00583715 * n.y;
    return fract(52.9829189 * fract(f));
}

fn dither_interleaved(rgb: vec3<f32>, levels: f32, frag_coord: vec4<f32>) -> vec3<f32> {
    var noise = interleaved_gradient_noise(frag_coord.xy);
    // scaled down slightly so that flat colours are not dithered
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

// 0-1 linear from 0-1 sRGB gamma
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

// [u8; 4] sRGB as u32 -> [r, g, b, a] in 0-1
fn unpack_color(color: u32) -> vec4<f32> {
    return vec4<f32>(
        f32(color & 255u),
        f32((color >> 8u) & 255u),
        f32((color >> 16u) & 255u),
        f32((color >> 24u) & 255u),
    ) / 255.0;
}

@vertex
fn vs_main(
    @location(0) a_pos: vec2<f32>,
    @location(1) a_tex_coord: vec2<f32>,
    @location(2) a_color: u32,
    @location(3) a_depth: f32,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coord = a_tex_coord;
    out.color = unpack_color(a_color);
    out.depth = a_depth;
    out.position = vec4<f32>(
        2.0 * a_pos.x / locals.screen_size.x - 1.0,
        1.0 - 2.0 * a_pos.y / locals.screen_size.y,
        0.0,
        1.0,
    );
    return out;
}

// Part of the pixel at `frag` where nothing of the scene is in front of
// `depth` (0 to 1): the four texels of the scene's depth around it, weighted
// like a bilinear lookup, so that a wall cuts a label along a smooth edge.
fn shown(frag: vec2<f32>, depth: f32) -> f32 {
    // a label that stands nowhere in the world is never hidden
    if depth > 1.5 {
        return 1.0;
    }
    let size = vec2<f32>(textureDimensions(scene_depth));
    let at = (frag + locals.jitter) / locals.screen_px * size - 0.5;
    let base = vec2<i32>(floor(at));
    let f = fract(at);
    let last = vec2<i32>(size) - 1;
    var part = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1u);
        let weight = mix(1.0 - f.x, f.x, f32(o.x)) * mix(1.0 - f.y, f.y, f32(o.y));
        // reverse-Z: a surface in front of the label has a larger depth
        if textureLoad(scene_depth, clamp(base + o, vec2<i32>(0), last), 0) <= depth {
            part += weight;
        }
    }
    return part;
}

fn color_gamma(in: VertexOutput) -> vec4<f32> {
    var out = in.color * textureSample(tex_color, tex_sampler, in.tex_coord);
    if locals.dithering == 1u {
        out = vec4<f32>(dither_interleaved(out.rgb, 256.0, in.position), out.a);
    }
    return out;
}

@fragment
fn fs_main_linear_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let gamma = color_gamma(in);
    let part = shown(in.position.xy, in.depth);
    if part <= 0.0 {
        discard;
    }
    // premultiplied: the hidden part of the pixel takes its share of both
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a) * part;
}

@fragment
fn fs_main_gamma_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    let gamma = color_gamma(in);
    let part = shown(in.position.xy, in.depth);
    if part <= 0.0 {
        discard;
    }
    return gamma * part;
}
