// Depth-based stochastic transparency: the resolve and the composite
// (oit_stochastic.rs; the passes over the faces are in object.wgsl,
// `fs_st_*`). An experiment: Enderton et al., "Stochastic Transparency",
// I3D 2010, section 3.4, steps 5.
//
// 1. cs_resolve: per pixel, the accumulated colour and alpha averaged over
//    the samples of every iteration, and the total alpha averaged over the
//    samples (1 - the product of the transparencies); the paper's "filtered"
//    values (a box filter over the pixel). Optionally blended with the
//    previous frame's (temporal: no reprojection, ghosts under motion).
// 2. cs_filter (optional): the paper's post-process, a cross-bilateral
//    filter of the accumulated colour guided by the total alpha, which is
//    not stochastic: the edges of the transparent layers are kept, the
//    noise inside them blurred.
// 3. fs_composite: per sample of the scene, the opaque colour dimmed by that
//    sample's transparency (the blend's destination factor) plus the
//    filtered accumulated colour times the alpha correction (the ratio of
//    the total alpha to the accumulated alpha; 1 without correction).
// 4. fs_composite_glow: the blended faces' glow, accumulated like their
//    colour (object.wgsl fs_st_glow) and corrected the same way, added to
//    the scene alpha (the glow amount) once the composite has dimmed it.

struct ResolveParams {
    width: u32,
    height: u32,
    // samples per pixel of the targets, and in all (over the iterations)
    samples_hw: u32,
    samples: u32,
    // bit 0 alpha correction, bit 1 filtered colour read by the composite,
    // bit 2 temporal blend, bit 3 glowing faces accumulated (glow_ms),
    // bit 4 the scene single-sampled: the composite dims each pixel by its
    // resolved total alpha (the transmittance at the passes' samples)
    flags: u32,
    // the filter's radius in pixels (1 to 4) and its alpha sigma
    radius: u32,
    sigma_alpha: f32,
    // the temporal blend: weight of this frame
    temporal_weight: f32,
    // samples per pixel of the transmittance's target: the scene's with
    // MSAA, the stochastic passes' (samples_hw) without
    trans_samples: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    // this frame's clip space to the last frame's (frame.motion_reproj)
    reproj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> rp: ResolveParams;
// the accumulation and the transmittance, multisampled
@group(0) @binding(1) var accum_ms: texture_multisampled_2d<f32>;
@group(0) @binding(2) var trans_ms: texture_multisampled_2d<f32>;
// cs_resolve writes: the averaged accumulation, and the total alpha (red)
// with the blended faces' corrected glow (green)
@group(0) @binding(3) var resolved_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var total_w: texture_storage_2d<rg32float, write>;
// the previous frame's resolved colour (temporal)
@group(0) @binding(5) var history: texture_2d<f32>;
// cs_filter and fs_composite read
@group(0) @binding(6) var resolved_r: texture_2d<f32>;
@group(0) @binding(7) var total_r: texture_2d<f32>;
@group(0) @binding(8) var filtered_w: texture_storage_2d<rgba16float, write>;
@group(0) @binding(9) var filtered_r: texture_2d<f32>;
// the stochastic depth (cs_resolve, temporal): the nearest transparent
// winner of the pixel, reprojected
@group(0) @binding(10) var st_depth_ms: texture_depth_multisampled_2d;
// the glow accumulated over the stochastic passes' samples (red)
@group(0) @binding(11) var glow_ms: texture_multisampled_2d<f32>;

// The averaged accumulation of a pixel: each iteration added its own
// samples' worth, so the average is over all of them.
fn resolve_pixel(px: vec2<i32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    let n = min(rp.samples_hw, 32u);
    for (var s = 0u; s < n; s += 1u) {
        sum += textureLoad(accum_ms, px, i32(s));
    }
    return sum / f32(max(rp.samples, 1u));
}

// Temporal (an option, labelled as such): the last frame's resolved colour
// where this pixel's nearest transparent surface was, through the camera's
// motion only (the hair's own sway is not reprojected), clamped to the 3 x 3
// neighbourhood of this frame as TAA does, then blended. Ghosts and smears
// where the clamp does not catch a change.
fn reprojected(px: vec2<i32>, cur: vec4<f32>) -> vec4<f32> {
    var z = 0.0;
    let n = min(rp.samples_hw, 32u);
    for (var s = 0u; s < n; s += 1u) {
        z = max(z, textureLoad(st_depth_ms, px, i32(s)));
    }
    if (z <= 0.0) {
        return cur;
    }
    let size = vec2<f32>(f32(rp.width), f32(rp.height));
    let ndc = vec2<f32>((f32(px.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / size.y * 2.0);
    let pp = rp.reproj * vec4<f32>(ndc, z, 1.0);
    if (pp.w <= 0.0) {
        return cur;
    }
    let pn = pp.xy / pp.w;
    let puv = vec2<f32>(pn.x * 0.5 + 0.5, 0.5 - pn.y * 0.5);
    if (puv.x < 0.0 || puv.y < 0.0 || puv.x >= 1.0 || puv.y >= 1.0) {
        return cur;
    }
    let old = textureLoad(history, vec2<i32>(puv * size), 0);
    var mn = cur;
    var mx = cur;
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            let q = clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
            let c = resolve_pixel(q);
            mn = min(mn, c);
            mx = max(mx, c);
        }
    }
    return mix(clamp(old, mn, mx), cur, rp.temporal_weight);
}

@compute @workgroup_size(8, 8)
fn cs_resolve(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= rp.width || g.y >= rp.height) {
        return;
    }
    let px = vec2<i32>(g.xy);
    var trans = 0.0;
    let n = min(rp.trans_samples, 32u);
    for (var s = 0u; s < n; s += 1u) {
        trans += textureLoad(trans_ms, px, i32(s)).x;
    }
    var resolved = resolve_pixel(px);
    let total = 1.0 - trans / f32(max(n, 1u));
    // the glow, averaged as the colour and corrected by the same ratio
    var glow = 0.0;
    if ((rp.flags & 8u) != 0u) {
        let m = min(rp.samples_hw, 32u);
        for (var s = 0u; s < m; s += 1u) {
            glow += textureLoad(glow_ms, px, i32(s)).x;
        }
        glow /= f32(max(rp.samples, 1u));
        if ((rp.flags & 1u) != 0u && resolved.a > 1e-5) {
            glow *= total / resolved.a;
        }
    }
    if ((rp.flags & 4u) != 0u) {
        resolved = reprojected(px, resolved);
    }
    textureStore(resolved_w, px, resolved);
    textureStore(total_w, px, vec4<f32>(total, glow, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn cs_filter(@builtin(global_invocation_id) g: vec3<u32>) {
    if (g.x >= rp.width || g.y >= rp.height) {
        return;
    }
    let px = vec2<i32>(g.xy);
    let centre = textureLoad(total_r, px, 0).x;
    if (centre <= 0.0) {
        textureStore(filtered_w, px, textureLoad(resolved_r, px, 0));
        return;
    }
    let r = i32(clamp(rp.radius, 1u, 4u));
    let sigma_s = f32(r) * 0.5;
    var sum = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -4; dy <= 4; dy += 1) {
        for (var dx = -4; dx <= 4; dx += 1) {
            if (abs(dx) > r || abs(dy) > r) {
                continue;
            }
            let q = clamp(px + vec2<i32>(dx, dy), vec2<i32>(0), vec2<i32>(i32(rp.width) - 1, i32(rp.height) - 1));
            let t = textureLoad(total_r, q, 0).x;
            let da = t - centre;
            let d2 = f32(dx * dx + dy * dy);
            let w = exp(-d2 / (2.0 * sigma_s * sigma_s) - da * da / (2.0 * rp.sigma_alpha * rp.sigma_alpha));
            sum += textureLoad(resolved_r, q, 0) * w;
            wsum += w;
        }
    }
    textureStore(filtered_w, px, sum / max(wsum, 1e-6));
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
    let total = textureLoad(total_r, px, 0).x;
    var trans: f32;
    if ((rp.flags & 16u) != 0u) {
        trans = 1.0 - total;
    } else {
        trans = textureLoad(trans_ms, px, i32(s)).x;
    }
    var c: vec4<f32>;
    if ((rp.flags & 2u) != 0u) {
        c = textureLoad(filtered_r, px, 0);
    } else {
        c = textureLoad(resolved_r, px, 0);
    }
    var corr = 1.0;
    if ((rp.flags & 1u) != 0u && c.a > 1e-5) {
        corr = total / c.a;
    }
    if (total <= 0.0 && trans >= 1.0) {
        discard;
    }
    return vec4<f32>(c.rgb * corr, 1.0 - trans);
}

// The blended faces' glow added to the scene alpha (alpha only, additive).
@fragment
fn fs_composite_glow(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let glow = textureLoad(total_r, vec2<i32>(p.xy), 0).y;
    if (glow <= 0.0) {
        discard;
    }
    return vec4<f32>(0.0, 0.0, 0.0, glow);
}
