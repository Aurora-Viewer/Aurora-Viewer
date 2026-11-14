// Depth-based stochastic transparency: the resolve and the composite
// (oit_stochastic.rs; the passes over the faces are in object.wgsl,
// `fs_st_*`). Enderton et al., "Stochastic Transparency", I3D 2010,
// section 3.4, step 5.
//
// 1. cs_resolve: per pixel, the accumulated colour and alpha averaged over
//    the passes' samples (the paper's "filtered" values: a box filter over
//    the pixel), the total alpha averaged over the samples (1 - the product
//    of the transparencies), the colour times the alpha correction (the
//    ratio of the total alpha to the accumulated alpha), and how uncertain
//    that colour is: it is the total alpha times the mean, weighted by their
//    alpha, of the samples' colours (each its accumulated colour over its
//    accumulated alpha), so its standard error is their spread over the
//    root of their count. Where one layer covers the pixel every sample
//    holds the same colour, the spread is 0 and the colour is exact.
// 2. cs_temporal: that colour blended with the previous frame's, reprojected
//    through the camera's motion, only as much as its uncertainty asks for:
//    an exact pixel is this frame's alone, so one layer neither softens nor
//    trails. The previous frame's colour is left out where its total alpha
//    (no noise in it) is not this pixel's (something else was there) and
//    where the camera slid the pixel's layers past one another.
// 3. cs_smooth: what grain the blend leaves (all of this frame's where the
//    previous one was left out), averaged with the neighbours whose colour
//    is within that grain of the pixel's (and of a like total alpha): broad
//    glass comes out even, while an edge or a texture stronger than the
//    grain stays as it is. Its result is drawn, not kept: the next frame
//    blends with cs_temporal's.
// 4. fs_composite: per sample of the scene, the opaque colour dimmed by that
//    sample's transparency (the blend's destination factor) plus the
//    resolved colour.
// 5. fs_composite_glow: the blended faces' glow, accumulated like their
//    colour (object.wgsl fs_st_glow) and corrected the same way, added to
//    the scene alpha (the glow amount) once the composite has dimmed it.
//
// No value that is not a finite number goes through: the passes sum what the
// faces give them and the resolve divides those sums, where a face's NaN or
// two infinite sums would take the pixel, and the previous frame's colour
// would keep it and hand it to its neighbours frame after frame (#1380, as
// the exact order's `blend_finite`). A sample holding one counts for
// nothing (`st_sample`), a transparency that is one hides nothing
// (`st_through`), and the previous frame's colour is left out where it is
// one.

struct ResolveParams {
    width: u32,
    height: u32,
    // samples per pixel of the passes' targets
    samples_hw: u32,
    // bit 0 temporal blend, bit 1 glowing faces accumulated (glow_ms),
    // bit 2 the scene single-sampled: the composite dims each pixel by its
    // total alpha (trans_one),
    // bit 3 the history holds an earlier frame of this view
    flags: u32,
    // the temporal blend: the least weight of this frame
    temporal_weight: f32,
    // samples per pixel of the transmittance's target: the scene's
    trans_samples: u32,
    _pad0: u32,
    _pad1: u32,
    // this frame's clip space to the last frame's (frame.motion_reproj)
    reproj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> rp: ResolveParams;
// the accumulation and the transmittance, multisampled
@group(0) @binding(1) var accum_ms: texture_multisampled_2d<f32>;
@group(0) @binding(2) var trans_ms: texture_multisampled_2d<f32>;
// cs_resolve writes this frame's corrected colour with its total alpha,
// cs_temporal the blended colour with that total alpha, cs_smooth the
// colour drawn
@group(0) @binding(3) var resolved_w: texture_storage_2d<rgba16float, write>;
// cs_resolve writes: the farthest stochastic depth, the blended faces'
// corrected glow, the colour's uncertainty (its standard error) and the
// nearest stochastic depth
@group(0) @binding(4) var aux_w: texture_storage_2d<rgba16float, write>;
// the previous frame's blended colour and total alpha (cs_temporal,
// cs_smooth)
@group(0) @binding(5) var history: texture_2d<f32>;
// cs_temporal reads this frame's colour, cs_smooth the blended one,
// fs_composite the smoothed one
@group(0) @binding(6) var resolved_r: texture_2d<f32>;
@group(0) @binding(7) var aux_r: texture_2d<f32>;
// the stochastic depth (cs_resolve): the nearest transparent winner of the
// pixel, which the temporal blend reprojects
@group(0) @binding(10) var st_depth_ms: texture_depth_multisampled_2d;
// the glow accumulated over the stochastic passes' samples (red)
@group(0) @binding(11) var glow_ms: texture_multisampled_2d<f32>;
// the transmittance of a single-sampled scene (trans_ms is then unused)
@group(0) @binding(12) var trans_one: texture_2d<f32>;

// The largest value an Rgba16Float target holds (common.wgsl `HDR_MAX`).
const HDR_MAX: f32 = 65504.0;

// Whether `x` is not a number. Tested on its bits (object.wgsl
// `blend_nan`): a driver may fold `x != x` to false, and what min, max and
// clamp make of a NaN is left to the card.
fn st_nan(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7fffffffu) > 0x7f800000u;
}

fn st_nan4(v: vec4<f32>) -> bool {
    return st_nan(v.r) || st_nan(v.g) || st_nan(v.b) || st_nan(v.a);
}

// A sample of the accumulation as the resolve takes it: nothing where a
// part of it is not a number, no part under 0 nor over what the target
// holds (the sum of many bright layers may be infinite).
fn st_sample(a: vec4<f32>) -> vec4<f32> {
    return select(clamp(a, vec4<f32>(0.0), vec4<f32>(HDR_MAX)), vec4<f32>(0.0), st_nan4(a));
}

// A summed amount (the glow) the same way.
fn st_amount(x: f32) -> f32 {
    return select(clamp(x, 0.0, HDR_MAX), 0.0, st_nan(x));
}

// A sample of the transmittance: between nothing and all of the light, and
// all of it where it is not a number.
fn st_through(x: f32) -> f32 {
    return select(clamp(x, 0.0, 1.0), 1.0, st_nan(x));
}

// The uncertainty left in a colour drawn, as a part of that colour (of a
// dim one, as a part of QUIET_DIM): a pixel under it is this frame's alone,
// one over it takes as much of the previous frames as brings it there.
const QUIET = 0.004;
const QUIET_DIM = 0.05;
// The difference of total alpha from which the previous frame's colour is
// another surface's, and the span over which it is left.
const OTHER_FROM = 0.004;
const OTHER_SPAN = 0.012;
// The pixels the camera's motion has slid the farthest of a pixel's layers
// past its nearest from which the previous frame's colour, reprojected as
// the nearest, is no longer what lies behind it, and the span over which it
// is left.
const SLID_FROM = 0.25;
const SLID_SPAN = 0.5;
// How far from a pixel's colour, in the uncertainty the blend leaves in it,
// a neighbour's still is the same colour under the grain (cs_smooth).
const GRAIN_REACH = 2.0;

// This frame's weight in the temporal blend for a colour `c` of uncertainty
// `unsure` to come out quiet: all of it where the colour is sure, the least
// where it is far from it (a blend of weight w leaves w / (2 - w) of the
// variance).
fn frame_weight(c: vec3<f32>, unsure: f32) -> f32 {
    let quiet = QUIET * max(max(c.r, c.g), max(c.b, QUIET_DIM));
    let q2 = quiet * quiet;
    return clamp(2.0 * q2 / (unsure * unsure + q2), rp.temporal_weight, 1.0);
}

@compute @workgroup_size(8, 8)
fn cs_resolve(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= rp.width || g.y >= rp.height) {
        return;
    }
    let px = vec2<i32>(g.xy);
    let n = min(rp.samples_hw, 8u);
    let tn = min(rp.trans_samples, 16u);
    var trans = 0.0;
    if (tn <= 1u) {
        trans = st_through(textureLoad(trans_one, px, 0).x);
    } else {
        for (var s = 0u; s < tn; s += 1u) {
            trans += st_through(textureLoad(trans_ms, px, i32(s)).x);
        }
        trans /= f32(tn);
    }
    var sum = vec4<f32>(0.0);
    // the sums of alpha x colour squared and of alpha squared, for the
    // spread of the samples' colours
    var squares = vec3<f32>(0.0);
    var alphas = 0.0;
    var z = 0.0;
    var z_far = 1.0;
    for (var s = 0u; s < n; s += 1u) {
        let a = st_sample(textureLoad(accum_ms, px, i32(s)));
        sum += a;
        if (a.a > 1e-5) {
            squares += a.rgb * a.rgb / a.a;
            alphas += a.a * a.a;
        }
        let depth = textureLoad(st_depth_ms, px, i32(s));
        z = max(z, depth);
        z_far = min(z_far, depth);
    }
    let nf = f32(max(n, 1u));
    let resolved = sum / nf;
    // (the total alpha is at the scene's samples, the accumulation at the
    // passes': a sliver that covers the first and none of the second has no
    // colour to show, and is left out rather than drawn black)
    let total = select(0.0, 1.0 - trans, resolved.a > 0.0);
    // the glow, averaged as the colour and corrected by the same ratio
    var glow = 0.0;
    if ((rp.flags & 2u) != 0u) {
        for (var s = 0u; s < n; s += 1u) {
            glow += st_amount(textureLoad(glow_ms, px, i32(s)).x);
        }
        glow /= nf;
    }
    // nothing blended here (a glowing face of alpha 0 has glow alone)
    if (resolved.a <= 0.0 && total <= 0.0 && glow <= 0.0) {
        textureStore(resolved_w, px, vec4<f32>(0.0));
        textureStore(aux_w, px, vec4<f32>(0.0));
        return;
    }
    var corr = 1.0;
    var unsure = 0.0;
    if (resolved.a > 1e-5) {
        corr = total / resolved.a;
        // the standard error of the weighted mean of the samples' colours,
        // times the total alpha
        let mean = sum.rgb / sum.a;
        let spread = max(squares / sum.a - mean * mean, vec3<f32>(0.0));
        let count = sum.a * sum.a / max(alphas, 1e-10);
        unsure = total * sqrt(max(spread.r, max(spread.g, spread.b)) / max(count - 1.0, 1.0));
    }
    // (the correction of a faint accumulation is large: nothing over what
    // the targets hold, which they would keep as infinite)
    let colour = min(resolved.rgb * corr, vec3<f32>(HDR_MAX));
    textureStore(resolved_w, px, vec4<f32>(colour, total));
    textureStore(aux_w, px, vec4<f32>(z_far, min(glow * corr, HDR_MAX), min(unsure, HDR_MAX), z));
}

// Where, in the last frame's pixels, what this pixel shows at the depth `z`
// was, through the camera's motion only (the hair's own sway is not
// reprojected). z < 0: behind the last frame's camera.
fn reprojected(px: vec2<i32>, z: f32) -> vec3<f32> {
    let size = vec2<f32>(f32(rp.width), f32(rp.height));
    let ndc = vec2<f32>((f32(px.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / size.y * 2.0);
    let pp = rp.reproj * vec4<f32>(ndc, z, 1.0);
    if (pp.w <= 0.0) {
        return vec3<f32>(-1.0);
    }
    let pn = pp.xy / pp.w;
    return vec3<f32>((pn.x * 0.5 + 0.5) * size.x, (0.5 - pn.y * 0.5) * size.y, 1.0);
}

// The last frame's colour for a pixel of total alpha `total`, and in alpha
// how much of it is left out (1: all of it): where the nearest layer was,
// unless that total alpha is not the pixel's (something else was there) or
// the farthest layer slid past the nearest since (the camera moved sideways:
// what shows through the nearest is no longer what was behind it).
fn last_frame(px: vec2<i32>, aux: vec4<f32>, total: f32) -> vec4<f32> {
    let last = vec2<i32>(i32(rp.width), i32(rp.height)) - vec2<i32>(1);
    // the farthest depth of the neighbourhood: which of a pixel's layers win
    // its few samples is chance, and so would be what is left out
    var z_far = aux.x;
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            z_far = min(z_far, textureLoad(aux_r, clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), last), 0).x);
        }
    }
    let at = reprojected(px, aux.w);
    let back = reprojected(px, z_far);
    let inside = at.x >= 0.0 && at.y >= 0.0 && at.x < f32(rp.width) && at.y < f32(rp.height);
    if ((rp.flags & 8u) == 0u || aux.w <= 0.0 || at.z <= 0.0 || back.z <= 0.0 || !inside) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    // (between its four texels: the nearest alone would put every edge up
    // to half a pixel off while the camera turns)
    let corner = floor(at.xy - 0.5);
    let part = at.xy - 0.5 - corner;
    let c00 = vec2<i32>(corner);
    let h00 = textureLoad(history, clamp(c00, vec2<i32>(0), last), 0);
    let h10 = textureLoad(history, clamp(c00 + vec2<i32>(1, 0), vec2<i32>(0), last), 0);
    let h01 = textureLoad(history, clamp(c00 + vec2<i32>(0, 1), vec2<i32>(0), last), 0);
    let h11 = textureLoad(history, clamp(c00 + vec2<i32>(1, 1), vec2<i32>(0), last), 0);
    let h = mix(mix(h00, h10, part.x), mix(h01, h11, part.x), part.y);
    // (a NaN in the history would stay there and reach the four pixels that
    // read it: none is let in)
    let apart = length(back.xy - at.xy);
    if (st_nan4(h) || st_nan(apart)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let slid = clamp((apart - SLID_FROM) / SLID_SPAN, 0.0, 1.0);
    let other = clamp((abs(h.a - total) - OTHER_FROM) / OTHER_SPAN, 0.0, 1.0);
    return vec4<f32>(h.rgb, max(other, slid));
}

@compute @workgroup_size(8, 8)
fn cs_temporal(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= rp.width || g.y >= rp.height) {
        return;
    }
    let px = vec2<i32>(g.xy);
    let cur = textureLoad(resolved_r, px, 0);
    let aux = textureLoad(aux_r, px, 0);
    let weight = frame_weight(cur.rgb, aux.z);
    if ((rp.flags & 1u) == 0u || weight >= 1.0) {
        textureStore(resolved_w, px, cur);
        return;
    }
    let h = last_frame(px, aux, cur.a);
    if (h.a >= 1.0) {
        textureStore(resolved_w, px, cur);
        return;
    }
    // the last frame's colour within the extent of the neighbourhood, as
    // TAA takes it
    let last = vec2<i32>(i32(rp.width), i32(rp.height)) - vec2<i32>(1);
    var mn = cur.rgb;
    var mx = cur.rgb;
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            let c = textureLoad(resolved_r, clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), last), 0).rgb;
            mn = min(mn, c);
            mx = max(mx, c);
        }
    }
    textureStore(resolved_w, px, vec4<f32>(mix(clamp(h.rgb, mn, mx), cur.rgb, mix(weight, 1.0, h.a)), cur.a));
}

@compute @workgroup_size(8, 8)
fn cs_smooth(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= rp.width || g.y >= rp.height) {
        return;
    }
    let px = vec2<i32>(g.xy);
    let c = textureLoad(resolved_r, px, 0);
    let aux = textureLoad(aux_r, px, 0);
    let unsure = aux.z;
    // a sure colour stays as it is
    let weight = frame_weight(c.rgb, unsure);
    if (weight >= 1.0) {
        textureStore(resolved_w, px, c);
        return;
    }
    // what the blend left of the uncertainty, more of it where it went
    // without the last frame
    let left = mix(sqrt(weight / (2.0 - weight)), 1.0, last_frame(px, aux, c.a).a);
    let reach = max(GRAIN_REACH * unsure * left, 1e-6);
    let last = vec2<i32>(i32(rp.width), i32(rp.height)) - vec2<i32>(1);
    // twice: the nearest neighbours around the pixel's own colour first,
    // which is off by its grain, then the wider ones around their mean
    var mid = c.rgb;
    for (var radius = 1; radius <= 2; radius += 1) {
        // (the mean so far weighs a little, should no neighbour be like it)
        var sum = vec4<f32>(mid, 1.0) * 0.01;
        for (var dy = -radius; dy <= radius; dy += 1) {
            for (var dx = -radius; dx <= radius; dx += 1) {
                let q = textureLoad(resolved_r, clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), last), 0);
                let d = abs(q.rgb - mid);
                let apart = max(d.r, max(d.g, d.b)) / reach;
                let like = max(1.0 - abs(q.a - c.a) / OTHER_FROM, 0.0);
                sum += vec4<f32>(q.rgb, 1.0) * like * exp(-apart * apart);
            }
        }
        mid = sum.rgb / sum.a;
    }
    textureStore(resolved_w, px, vec4<f32>(mid, c.a));
}

@vertex
fn vs_full(@builtin(vertex_index) v: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((v << 1u) & 2u), f32(v & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// Per sample: the colour added over the scene, and in alpha this sample's
// total alpha, the blend's destination factor dimming the scene by its
// transparency (One / OneMinusSrcAlpha; the scene alpha, the glow, dimmed
// the same way). A single-sampled scene: the pixel's total alpha, averaged
// over the passes' samples.
@fragment
fn fs_composite(@builtin(position) p: vec4<f32>, @builtin(sample_index) s: u32) -> @location(0) vec4<f32> {
    let px = vec2<i32>(p.xy);
    let c = textureLoad(resolved_r, px, 0);
    var trans: f32;
    if ((rp.flags & 4u) != 0u) {
        trans = 1.0 - c.a;
    } else {
        trans = st_through(textureLoad(trans_ms, px, i32(s)).x);
    }
    // (no colour resolved here: nothing is drawn, rather than the scene
    // dimmed by a face that left none, as cs_resolve's total alpha)
    if (c.a <= 0.0) {
        discard;
    }
    return vec4<f32>(c.rgb, 1.0 - trans);
}

// The blended faces' glow added to the scene alpha (alpha only, additive).
@fragment
fn fs_composite_glow(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let glow = st_amount(textureLoad(aux_r, vec2<i32>(p.xy), 0).y);
    if (glow <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, glow);
}
