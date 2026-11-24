// The outline of what is pointed at (red of the mask) and of what is selected
// (green): a line around the silhouette, a soft glow, a light wash inside.
struct Outline { colors: array<vec4<f32>,2>, pixel: vec4<f32> }
@group(0) @binding(0) var mask: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> u: Outline;
struct Corner { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Corner {
    let p = vec2(f32((i << 1u) & 2u), f32(i & 2u));
    return Corner(vec4(p*2.0 - 1.0, 0.0, 1.0), vec2(p.x, 1.0 - p.y));
}
@fragment fn fs(v: Corner) -> @location(0) vec4<f32> {
    let inside = textureSampleLevel(mask, samp, v.uv, 0.0).rg;
    var line = vec2(0.0);
    var glow = vec2(0.0);
    for (var i = 0; i < 20; i++) {
        let a = f32(i)*0.31415927;
        let d = vec2(cos(a), sin(a))*u.pixel.xy*u.pixel.z;
        line = max(line, textureSampleLevel(mask, samp, v.uv + d, 0.0).rg);
        line = max(line, textureSampleLevel(mask, samp, v.uv + d*0.5, 0.0).rg);
        glow += textureSampleLevel(mask, samp, v.uv + d*2.6, 0.0).rg;
        glow += textureSampleLevel(mask, samp, v.uv + d*4.4, 0.0).rg*0.6;
    }
    glow /= 32.0;
    var out = vec4(0.0);
    for (var c = 0; c < 2; c++) {
        let color = u.colors[c];
        let edge = line[c]*(1.0 - inside[c]);
        let around = glow[c]*(1.0 - inside[c])*0.55;
        let strength = max(max(edge, around), inside[c]*0.10)*color.a;
        out = vec4(mix(out.rgb, color.rgb, strength/max(strength + out.a*(1.0 - strength), 0.0001)), strength + out.a*(1.0 - strength));
    }
    return out;
}
