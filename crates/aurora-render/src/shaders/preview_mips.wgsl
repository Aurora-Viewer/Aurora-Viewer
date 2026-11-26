// The next smaller size of a texture: the mean of four texels, the colours
// weighted by their opacity (what a cut-out leaves out does not stain its edge).
@group(0) @binding(0) var from_level: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u))*2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let last = vec2<i32>(textureDimensions(from_level)) - 1;
    let p = vec2<i32>(at.xy)*2;
    var sum = vec4(0.0);
    var plain = vec3(0.0);
    for (var i = 0; i < 4; i++) {
        let texel = textureLoad(from_level, min(p + vec2(i & 1, i >> 1u), last), 0);
        sum += vec4(texel.rgb*texel.a, texel.a);
        plain += texel.rgb;
    }
    let color = select(plain*0.25, sum.rgb/max(sum.a, 0.0001), sum.a > 0.001);
    return vec4(color, sum.a*0.25);
}
