// The veil of a preview: its bottom band blurred more and more toward the
// bottom edge (a progressive blur), under what the interface lays there.
// The picture's copy holds its smaller sizes; deeper rows read smaller ones,
// spread over a few taps so that no block of a small size shows.
struct Veil { band: vec4<f32> }  // x: where it starts (0 top, 1 bottom), y: deepest size, zw: one pixel
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: Veil;
struct Corner { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Corner {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Corner(vec4(p*2.0 - 1.0, 0.0, 1.0), vec2(p.x, 1.0 - p.y));
}
@fragment fn fs(v: Corner) -> @location(0) vec4<f32> {
    let t = clamp((v.uv.y - u.band.x)/max(1.0 - u.band.x, 0.0001), 0.0, 1.0);
    let lod = t*t*(3.0 - 2.0*t)*u.band.y;
    let r = exp2(lod)*u.band.zw;
    var c = textureSampleLevel(picture, samp, v.uv, lod)*0.25;
    c += textureSampleLevel(picture, samp, v.uv + vec2(r.x, 0.0), lod)*0.125;
    c += textureSampleLevel(picture, samp, v.uv - vec2(r.x, 0.0), lod)*0.125;
    c += textureSampleLevel(picture, samp, v.uv + vec2(0.0, r.y), lod)*0.125;
    c += textureSampleLevel(picture, samp, v.uv - vec2(0.0, r.y), lod)*0.125;
    c += textureSampleLevel(picture, samp, v.uv + r, lod)*0.0625;
    c += textureSampleLevel(picture, samp, v.uv - r, lod)*0.0625;
    c += textureSampleLevel(picture, samp, v.uv + vec2(r.x, -r.y), lod)*0.0625;
    c += textureSampleLevel(picture, samp, v.uv + vec2(-r.x, r.y), lod)*0.0625;
    return c;
}
