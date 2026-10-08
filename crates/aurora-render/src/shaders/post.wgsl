// Tonemapping (Khronos PBR Neutral or ACES), optional FXAA, output encoding.

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;

struct PostParams {
    exposure: f32,
    output_srgb: f32,
    tonemapper: f32, // 0 Khronos PBR Neutral, 1 ACES, 2 none (classic skies)
    fxaa: f32,
    sharpen: f32, // AMD CAS sharpness (0 = off)
    glow: f32,    // 1: add the blurred glow (glowcombineF.glsl)
    debug_glow: f32, // AURORA_DEBUG_GLOW: show the glow amount of each pixel
    _p2: f32,
};
@group(0) @binding(2) var<uniform> post: PostParams;
// Blurred glow (512x512, display space), see glow.wgsl.
@group(0) @binding(3) var glow_tex: texture_2d<f32>;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_post(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

// Khronos PBR Neutral tone mapper (the one Second Life uses).
fn neutral(color_in: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desat = 0.15;
    let x = min(color_in.r, min(color_in.g, color_in.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var color = color_in - vec3<f32>(offset);
    let peak = max(color.r, max(color.g, color.b));
    if (peak < start) {
        return color;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    color = color * (new_peak / peak);
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(color, vec3<f32>(new_peak), g);
}

// ACES filmic (Narkowicz fit).
fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return (x * (a * x + b)) / (x * (c * x + d) + e);
}

fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let e = max(c * post.exposure, vec3<f32>(0.0));
    var m: vec3<f32>;
    if (post.tonemapper > 1.5) {
        // classic skies (tonemapUtilF NO_POST): exposure, then clamp
        m = e;
    } else if (post.tonemapper > 0.5) {
        m = aces(e * 0.8);
    } else {
        m = neutral(e);
    }
    return clamp(m, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(sqrt(c), vec3<f32>(0.299, 0.587, 0.114));
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return tonemap(textureSampleLevel(hdr, hdr_sampler, uv, 0.0).rgb);
}

// FXAA (Lottes, "FXAA 2" PC variant) on tonemapped color.
fn fxaa(uv: vec2<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(hdr));
    let rgb_m = tap(uv);
    let l_nw = luma(tap(uv + vec2<f32>(-1.0, -1.0) * texel));
    let l_ne = luma(tap(uv + vec2<f32>(1.0, -1.0) * texel));
    let l_sw = luma(tap(uv + vec2<f32>(-1.0, 1.0) * texel));
    let l_se = luma(tap(uv + vec2<f32>(1.0, 1.0) * texel));
    let l_m = luma(rgb_m);
    let l_min = min(l_m, min(min(l_nw, l_ne), min(l_sw, l_se)));
    let l_max = max(l_m, max(max(l_nw, l_ne), max(l_sw, l_se)));
    if (l_max - l_min < max(0.0312, l_max * 0.125)) {
        return rgb_m;
    }
    var dir = vec2<f32>(-((l_nw + l_ne) - (l_sw + l_se)), (l_nw + l_sw) - (l_ne + l_se));
    let reduce = max((l_nw + l_ne + l_sw + l_se) * (0.25 / 8.0), 1.0 / 128.0);
    let rcp_min = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp_min, vec2<f32>(-8.0), vec2<f32>(8.0)) * texel;
    let rgb_a = 0.5 * (tap(uv + dir * (1.0 / 3.0 - 0.5)) + tap(uv + dir * (2.0 / 3.0 - 0.5)));
    let rgb_b = rgb_a * 0.5 + 0.25 * (tap(uv + dir * -0.5) + tap(uv + dir * 0.5));
    let l_b = luma(rgb_b);
    if (l_b < l_min || l_b > l_max) {
        return rgb_a;
    }
    return rgb_b;
}

// AMD FidelityFX CAS, no-scaling path with the diagonals (CASF.glsl, as
// Firestorm's RenderCASSharpness), in display (sRGB) space.
fn cas_tap(uv: vec2<f32>, o: vec2<f32>, texel: vec2<f32>) -> vec3<f32> {
    return linear_to_srgb(tap(uv + o * texel));
}

fn cas(uv: vec2<f32>) -> vec3<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(hdr));
    let a = cas_tap(uv, vec2<f32>(-1.0, -1.0), texel);
    let b = cas_tap(uv, vec2<f32>(0.0, -1.0), texel);
    let c = cas_tap(uv, vec2<f32>(1.0, -1.0), texel);
    let d = cas_tap(uv, vec2<f32>(-1.0, 0.0), texel);
    let e = cas_tap(uv, vec2<f32>(0.0, 0.0), texel);
    let f = cas_tap(uv, vec2<f32>(1.0, 0.0), texel);
    let g = cas_tap(uv, vec2<f32>(-1.0, 1.0), texel);
    let h = cas_tap(uv, vec2<f32>(0.0, 1.0), texel);
    let i = cas_tap(uv, vec2<f32>(1.0, 1.0), texel);
    // soft min / max over the cross, plus the diagonals
    var mn = min(min(min(d, e), min(f, b)), h);
    let mn2 = min(mn, min(min(a, c), min(g, i)));
    mn = mn + mn2;
    var mx = max(max(max(d, e), max(f, b)), h);
    let mx2 = max(mx, max(max(a, c), max(g, i)));
    mx = mx + mx2;
    // smooth minimum distance to the signal limit divided by the maximum
    var amp = clamp(min(mn, vec3<f32>(2.0) - mx) / max(mx, vec3<f32>(1e-5)), vec3<f32>(0.0), vec3<f32>(1.0));
    amp = sqrt(amp);
    // CasSetup: peak = -1 / lerp(8, 5, sharpness)
    let peak = -1.0 / mix(8.0, 5.0, clamp(post.sharpen, 0.0, 1.0));
    let w = amp * peak;
    let rcp_w = 1.0 / (vec3<f32>(1.0) + 4.0 * w);
    return clamp((b * w + d * w + f * w + h * w + e) * rcp_w, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn srgb_to_linear_post(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// glowExtractF.glsl (RenderGlowMinLuminance 9999: only the glow amount
// accumulated in the scene alpha counts) on the gamma-corrected image,
// blended with BT_ADD_WITH_ALPHA: rgb x alpha. LL takes one tap per glow
// texel; four bilinear taps over the footprint keep it from shimmering.
@fragment
fn fs_glow_extract(in: Out) -> @location(0) vec4<f32> {
    // one glow texel in uv
    let foot = vec2<f32>(1.0 / 512.0);
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2<f32>(f32(i & 1) - 0.5, f32(i >> 1u) - 0.5) * 0.5 * foot;
        let c = textureSampleLevel(hdr, hdr_sampler, in.uv + o, 0.0);
        let a = clamp(c.a, 0.0, 1.0);
        acc += vec4<f32>(linear_to_srgb(tonemap(c.rgb)) * a, a * a);
    }
    return acc * 0.25;
}

@fragment
fn fs_post(in: Out) -> @location(0) vec4<f32> {
    var m: vec3<f32>;
    if (post.fxaa > 0.5) {
        m = fxaa(in.uv);
    } else if (post.sharpen > 0.001) {
        m = srgb_to_linear_post(cas(in.uv));
    } else {
        m = tap(in.uv);
    }
    if (post.glow > 0.5) {
        // glowcombineF.glsl: added in display space, then clamped by the 8-bit target
        let g = textureSampleLevel(glow_tex, hdr_sampler, in.uv, 0.0).rgb;
        m = srgb_to_linear_post(min(linear_to_srgb(m) + g, vec3<f32>(1.0)));
    }
    if (post.debug_glow > 0.5) {
        // red: glow x 10 (0.1 saturates), green: glow x 2, blue: dimmed scene
        let a = textureSampleLevel(hdr, hdr_sampler, in.uv, 0.0).a;
        let l = dot(m, vec3<f32>(0.2126, 0.7152, 0.0722));
        m = vec3<f32>(clamp(a * 10.0, 0.0, 1.0), clamp(a * 2.0, 0.0, 1.0), l * 0.5);
    }
    if (post.output_srgb < 0.5) {
        // target is not an sRGB format: encode manually
        m = linear_to_srgb(m);
    }
    return vec4<f32>(m, 1.0);
}
