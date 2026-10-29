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
    legacy_gamma: f32, // sky gamma on skies without probe ambiance, else 1
    // Aurora's bloom (bloom.wgsl): the share of the light above the
    // threshold spread around (0 = off), the threshold and its soft knee in
    // exposed units (1 = white), the gain / the number of bloom levels
    bloom: f32,
    bloom_threshold: f32,
    bloom_knee: f32,
    bloom_norm: f32,
    // RenderTonemapMix: share of the tone mapped color over the exposed
    // linear one (tonemapUtilF.glsl), 0 on classic skies
    tonemap_mix: f32,
    // the automatic exposure (exposure.wgsl), copied here on the GPU before
    // the glow, the bloom and this pass
    auto_exposure: f32,
    // a cut-out photo (FrameParams::cutout): the scene's alpha holds what
    // shows through each pixel; 1: the photo, straight color and coverage,
    // 2: the live view, over a checkerboard
    cutout: f32,
    _pad: f32,
    // the part of the photo this frame draws, in fractions of the photo
    // (left, top, width, height; 0, 0, 1, 1 outside a tiled photo): the
    // glow's 512 x 512 texels are the whole photo's, as in one pass
    photo_part: vec4<f32>,
};
@group(0) @binding(2) var<uniform> post: PostParams;
// Blurred glow (512x512, display space), see glow.wgsl.
@group(0) @binding(3) var glow_tex: texture_2d<f32>;
// The bloom levels added up (half resolution, scene units), see bloom.wgsl.
@group(1) @binding(0) var bloom_tex: texture_2d<f32>;

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

// ACES filmic, Stephen Hill's fit (toneMapACES_Hill, tonemapUtilF.glsl,
// from MJP's BakingLab): sRGB to the RRT's space, the RRT and ODT curve,
// and back. WGSL matrices are column-major like GLSL's.
const ACES_INPUT = mat3x3<f32>(
    vec3<f32>(0.59719, 0.07600, 0.02840),
    vec3<f32>(0.35458, 0.90834, 0.13383),
    vec3<f32>(0.04823, 0.01566, 0.83777),
);
const ACES_OUTPUT = mat3x3<f32>(
    vec3<f32>(1.60475, -0.10208, -0.00327),
    vec3<f32>(-0.53108, 1.10813, -0.07276),
    vec3<f32>(-0.07367, -0.00605, 1.07602),
);

fn aces(x: vec3<f32>) -> vec3<f32> {
    let c = ACES_INPUT * x;
    let a = c * (c + 0.0245786) - 0.000090537;
    let b = c * (0.983729 * c + 0.4329510) + 0.238081;
    return clamp(ACES_OUTPUT * (a / b), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Preferences' exposure times the automatic one (final_exposure).
fn exposure() -> f32 {
    return post.exposure * post.auto_exposure;
}

// toneMap (tonemapUtilF.glsl): exposed, tone mapped, mixed back with the
// exposed linear color by RenderTonemapMix, clamped.
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let e = max(c * exposure(), vec3<f32>(0.0));
    var m: vec3<f32>;
    if (post.tonemapper > 1.5) {
        // classic skies (tonemapUtilF NO_POST): exposure, then clamp
        m = e;
    } else {
        var t: vec3<f32>;
        if (post.tonemapper > 0.5) {
            t = aces(e);
        } else {
            t = neutral(e);
        }
        m = mix(e, t, post.tonemap_mix);
    }
    return clamp(m, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear_post(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// legacyGamma (postDeferredTonemap.glsl, postDeferredGammaCorrect.glsl,
// CASF.glsl with LEGACY_GAMMA): 1 - (1 - c)^gamma on the sRGB-encoded
// color, after linear_to_srgb, with the sky's raw gamma. Our pipeline
// stays linear until the output, hence the round trip.
fn legacy_gamma(m: vec3<f32>) -> vec3<f32> {
    if (abs(post.legacy_gamma - 1.0) < 1e-4) {
        return m;
    }
    let c = vec3<f32>(1.0) - clamp(linear_to_srgb(m), vec3<f32>(0.0), vec3<f32>(1.0));
    return srgb_to_linear_post(vec3<f32>(1.0) - pow(c, vec3<f32>(post.legacy_gamma)));
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(sqrt(c), vec3<f32>(0.299, 0.587, 0.114));
}

// The part of a scene color the bloom spreads: what goes over the
// threshold once exposed, with a quadratic knee below it (scene units).
fn bloom_extract(c: vec3<f32>) -> vec3<f32> {
    let lit = max(c, vec3<f32>(0.0));
    let b = max(lit.r, max(lit.g, lit.b)) * exposure();
    let knee = max(post.bloom_knee, 1e-4);
    var soft = clamp(b - post.bloom_threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    return lit * (max(soft, b - post.bloom_threshold) / max(b, 1e-5));
}

// The scene color with the bloom: the share `bloom` of the light over the
// threshold leaves the pixel and comes back spread by the levels (brighter
// by bloom.rs GAIN, folded into bloom_norm): bright spots keep their
// shape, gain a soft halo, and nothing far under the threshold moves.
fn scene(uv: vec2<f32>) -> vec3<f32> {
    let px = textureSampleLevel(hdr, hdr_sampler, uv, 0.0);
    let c = px.rgb;
    var lit = c;
    if (post.bloom > 0.0) {
        let spread = textureSampleLevel(bloom_tex, hdr_sampler, uv, 0.0).rgb * post.bloom_norm;
        lit = c + post.bloom * (spread - bloom_extract(c));
    }
    if (post.cutout > 0.5) {
        // drawn over black: premultiplied by the coverage, tone mapped as
        // the surface's own color
        let cover = 1.0 - px.a;
        return select(vec3<f32>(0.0), lit / max(cover, 1e-4), cover > 1.0 / 1024.0);
    }
    return lit;
}

// What a cut-out photo covers at `uv` (0 where nothing is drawn).
fn cutout_cover(uv: vec2<f32>) -> f32 {
    return clamp(1.0 - textureSampleLevel(hdr, hdr_sampler, uv, 0.0).a, 0.0, 1.0);
}

// Tone mapped color, before legacyGamma (CAS input).
fn tap_linear(uv: vec2<f32>) -> vec3<f32> {
    return tonemap(scene(uv));
}

// Final color as LLPipeline::tonemap writes it (legacyGamma included): what
// the glow, FXAA and SMAA work on.
fn tap(uv: vec2<f32>) -> vec3<f32> {
    return legacy_gamma(tap_linear(uv));
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
    return linear_to_srgb(tap_linear(uv + o * texel));
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

// glowExtractF.glsl (RenderGlowMinLuminance 9999: only the glow amount
// accumulated in the scene alpha counts) on the gamma-corrected image,
// blended with BT_ADD_WITH_ALPHA: rgb x alpha. LL takes one tap per glow
// texel; four bilinear taps over the footprint keep it from shimmering.
@fragment
fn fs_glow_extract(in: Out) -> @location(0) vec4<f32> {
    // in.uv is a place in the whole photo; one glow texel in the image's uv
    // (texels off the part drawn clamp to its edge: the guard band, cropped)
    let part = post.photo_part;
    let uv = (in.uv - part.xy) / part.zw;
    let foot = vec2<f32>(1.0 / 512.0) / part.zw;
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2<f32>(f32(i & 1) - 0.5, f32(i >> 1u) - 0.5) * 0.5 * foot;
        let c = textureSampleLevel(hdr, hdr_sampler, uv + o, 0.0);
        let a = clamp(c.a, 0.0, 1.0);
        acc += vec4<f32>(linear_to_srgb(legacy_gamma(tonemap(c.rgb))) * a, a * a);
    }
    return acc * 0.25;
}

fn bloom_box(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>) -> vec4<f32> {
    let m = (a + b + c + d) * 0.25;
    // Karis average: each box weighs 1 / (1 + its luma), so that one very
    // bright pixel (a specular on a moving mesh) does not flicker
    let w = 1.0 / (1.0 + dot(m, vec3<f32>(0.2126, 0.7152, 0.0722)) * exposure());
    return vec4<f32>(m * w, w);
}

// First bloom level (half resolution): the light over the threshold,
// downsampled with the 13 taps of bloom.wgsl and the Karis average.
@fragment
fn fs_bloom_prefilter(in: Out) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(hdr));
    var t: array<vec3<f32>, 13>;
    let offs = array<vec2<f32>, 13>(
        vec2<f32>(-2.0, -2.0), vec2<f32>(0.0, -2.0), vec2<f32>(2.0, -2.0),
        vec2<f32>(-2.0, 0.0), vec2<f32>(0.0, 0.0), vec2<f32>(2.0, 0.0),
        vec2<f32>(-2.0, 2.0), vec2<f32>(0.0, 2.0), vec2<f32>(2.0, 2.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    for (var i = 0; i < 13; i++) {
        // an infinite pixel held at the largest value of the format, a NaN
        // (also what filtering gives next to an infinite one) as black: the
        // bloom chain would spread either over the whole image (#1380)
        let s = textureSampleLevel(hdr, hdr_sampler, in.uv + offs[i] * texel, 0.0).rgb;
        t[i] = bloom_extract(select(min(s, vec3<f32>(65504.0)), vec3<f32>(0.0), s != s));
    }
    var acc = bloom_box(t[9], t[10], t[11], t[12]) * 0.5;
    acc += bloom_box(t[0], t[1], t[3], t[4]) * 0.125;
    acc += bloom_box(t[1], t[2], t[4], t[5]) * 0.125;
    acc += bloom_box(t[3], t[4], t[6], t[7]) * 0.125;
    acc += bloom_box(t[4], t[5], t[7], t[8]) * 0.125;
    return vec4<f32>(acc.rgb / max(acc.a, 1e-5), 1.0);
}

@fragment
fn fs_post(in: Out) -> @location(0) vec4<f32> {
    var m: vec3<f32>;
    if (post.fxaa > 0.5) {
        m = fxaa(in.uv);
    } else if (post.sharpen > 0.001) {
        // CASF.glsl: sharpening, then linear_to_srgb and legacyGamma
        m = legacy_gamma(srgb_to_linear_post(cas(in.uv)));
    } else {
        m = tap(in.uv);
    }
    if (post.glow > 0.5) {
        // glowcombineF.glsl: added in display space, then clamped by the 8-bit target
        let g = textureSampleLevel(glow_tex, hdr_sampler, post.photo_part.xy + in.uv * post.photo_part.zw, 0.0).rgb;
        m = srgb_to_linear_post(min(linear_to_srgb(m) + g, vec3<f32>(1.0)));
    }
    if (post.debug_glow > 0.5) {
        // red: glow x 10 (0.1 saturates), green: glow x 2, blue: dimmed scene
        let a = textureSampleLevel(hdr, hdr_sampler, in.uv, 0.0).a;
        let l = dot(m, vec3<f32>(0.2126, 0.7152, 0.0722));
        m = vec3<f32>(clamp(a * 10.0, 0.0, 1.0), clamp(a * 2.0, 0.0, 1.0), l * 0.5);
    }
    var alpha = 1.0;
    if (post.cutout > 1.5) {
        // the live view: what the photo leaves empty shows as a checkerboard
        let cell = vec2<u32>(in.clip.xy / 12.0);
        let light = ((cell.x + cell.y) & 1u) == 0u;
        let checker = select(vec3<f32>(0.16), vec3<f32>(0.32), light);
        m = mix(checker, m, cutout_cover(in.uv));
    } else if (post.cutout > 0.5) {
        alpha = cutout_cover(in.uv);
    }
    if (post.output_srgb < 0.5) {
        // target is not an sRGB format: encode manually
        m = linear_to_srgb(m);
    }
    return vec4<f32>(m, alpha);
}
