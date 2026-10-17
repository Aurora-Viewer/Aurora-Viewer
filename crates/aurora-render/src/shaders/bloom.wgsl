// Bloom of the bright parts of the image, Aurora's own (the Legacy Viewer
// only has the glow of faces with a glow value, glow.wgsl, which stays as
// it is): the mip chain of Jimenez, "Next Generation Post Processing in
// Call of Duty: Advanced Warfare" (SIGGRAPH 2014). The first level is
// extracted from the scene in post.wgsl (fs_bloom_prefilter, with the
// Karis average against fireflies); these passes make the smaller levels
// (13-tap downsample) and add each one, blurred by a 3 x 3 tent, to the
// level above it. post.wgsl spreads the result over the image.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_bloom(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

fn at(uv: vec2<f32>, o: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, src_sampler, uv + o * texel, 0.0).rgb;
}

// The 13 bilinear taps of the downsample: four overlapping 2 x 2 boxes
// around the centre and one inside them, weighted 0.125 each and 0.5.
@fragment
fn fs_bloom_down(in: Out) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    let a = at(in.uv, vec2<f32>(-2.0, -2.0), texel);
    let b = at(in.uv, vec2<f32>(0.0, -2.0), texel);
    let c = at(in.uv, vec2<f32>(2.0, -2.0), texel);
    let d = at(in.uv, vec2<f32>(-2.0, 0.0), texel);
    let e = at(in.uv, vec2<f32>(0.0, 0.0), texel);
    let f = at(in.uv, vec2<f32>(2.0, 0.0), texel);
    let g = at(in.uv, vec2<f32>(-2.0, 2.0), texel);
    let h = at(in.uv, vec2<f32>(0.0, 2.0), texel);
    let i = at(in.uv, vec2<f32>(2.0, 2.0), texel);
    let j = at(in.uv, vec2<f32>(-1.0, -1.0), texel);
    let k = at(in.uv, vec2<f32>(1.0, -1.0), texel);
    let l = at(in.uv, vec2<f32>(-1.0, 1.0), texel);
    let m = at(in.uv, vec2<f32>(1.0, 1.0), texel);
    var col = e * 0.125;
    col += (a + c + g + i) * 0.03125;
    col += (b + d + f + h) * 0.0625;
    col += (j + k + l + m) * 0.125;
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}

// The 3 x 3 tent of the smaller level, added (blending One, One) to the
// level above it, one texel of the smaller level wide.
@fragment
fn fs_bloom_up(in: Out) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src));
    var col = at(in.uv, vec2<f32>(0.0, 0.0), texel) * 4.0;
    col += (at(in.uv, vec2<f32>(0.0, -1.0), texel) + at(in.uv, vec2<f32>(-1.0, 0.0), texel)
        + at(in.uv, vec2<f32>(1.0, 0.0), texel) + at(in.uv, vec2<f32>(0.0, 1.0), texel)) * 2.0;
    col += at(in.uv, vec2<f32>(-1.0, -1.0), texel) + at(in.uv, vec2<f32>(1.0, -1.0), texel)
        + at(in.uv, vec2<f32>(-1.0, 1.0), texel) + at(in.uv, vec2<f32>(1.0, 1.0), texel);
    return vec4<f32>(col * (1.0 / 16.0), 0.0);
}
