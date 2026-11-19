// The denoiser of the traced passes that keep a history (rt_shadows.rs
// `Denoise`: rt_gi.wgsl's indirect light, rt_lights.wgsl's share of the
// local light), after SVGF (Schied et al. 2017, "Spatiotemporal
// Variance-Guided Filtering"):
//
// `cs_variance`: each pixel's variance from the luminance moments its
// history kept (the guide's `w`); where the history is short, from its
// neighbours' moments instead, raised the shorter it is. Each 8×8 tile
// with any variance is marked (`tiles`), as AMD's FidelityFX Shadow
// Denoiser and NVIDIA's NRD SIGMA classify theirs.
// `cs_atrous`, `ATROUS_PASSES` times (rt_shadows.rs) with the step
// doubling: an à-trous wavelet (3×3 with holes, which costs half SVGF's 5×5
// for the same reach once the steps double) whose weights stop at a different depth, a
// different normal and, by the variance, a luminance too far apart to be
// noise: the noisy parts blur wide, the edges and the real changes stay.
// A tile whose own pixels and its neighbours' have none (a light's share
// lit through or shadowed through, most of the image) is copied as it is:
// the filter would leave each of its pixels unchanged. The passes write to
// two images in turn, so from the third on the image a pass writes to
// holds such a tile already, from the pass before the last: it is copied
// once to each image and left alone after (`params.mode.z`, `.w`).
//
// The image filtered: rgb a value, a its history's frames (`cs_variance`'s
// input) or its variance (`cs_atrous`'s). The guide (rt_history.wgsl
// `pack_normal`): xy the normal, z the distance from the eye (0 where
// nothing is), w the second moment of the luminance.
//
// `cs_variance_pair` and `cs_atrous_pair` are the same for two numbers
// filtered at once, each by its own variance: rt_lights.wgsl's share of
// the local light (r, its frames in a, its second moment in the guide) and
// the sun's light
// (g, its frames in b; a ray is stopped or is not, so its second moment is
// its mean). Between the passes b and a are their variances. One pass's
// reads serve both, and a shadow's sharp edge is not blurred by the noise
// of the other light's penumbra across it. Beside the pair,
// `cs_atrous_pair` filters the share by colour (rt_lights.wgsl's tint) with
// the share's weights, where `params.mode.x` says there is one.

struct Params {
    // width, height of the image, z: this pass's step (pixels), w: the
    // variance under which a pixel has no noise worth filtering
    size: vec4<f32>,
    // x: 1 when the pair has a tint beside (`input_tint`); y: over 0 when
    // `cs_atrous`'s image is kept as the next frame's history, a number
    // that changes with the frame (`dithered`); z: 1 when `output` holds
    // the tiles with no noise already, w: 1 when `output_tint` does
    mode: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var input: texture_2d<f32>;
@group(0) @binding(2) var guide: texture_2d<f32>;
@group(0) @binding(3) var output: texture_storage_2d<rgba16float, write>;
// one per 8×8 tile, row by row: 1 when a pixel of it has variance
@group(0) @binding(4) var<storage, read_write> tiles: array<u32>;
// `cs_atrous_pair` alone: rgb the share by colour, read and written only
// with `params.mode.x`
@group(0) @binding(5) var input_tint: texture_2d<f32>;
@group(0) @binding(6) var output_tint: texture_storage_2d<rgba16float, write>;

var<workgroup> tile_noisy: atomic<u32>;

// Frames under which the variance comes from the neighbours.
const SHORT_HISTORY: f32 = 4.0;
// How many standard deviations apart two luminances may be and still be
// one surface's noise (SVGF's sigma_l), and how sharp the normals' stop is
// (its sigma_n).
const SIGMA_LUMA: f32 = 4.0;
const NORMAL_POWER: f32 = 64.0;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn unpack_normal(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// A hash of `v` (PCG).
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// `c` as a 16-bit float holds it, each channel rounded up or down by
// chance so that its mean is `c` (rt_gi.wgsl `dithered`, which says why):
// for the image kept as a history, another chance for each pixel `p` at
// each `frame`.
fn dithered(c: vec3<f32>, p: vec2<u32>, frame: u32) -> vec3<f32> {
    let a = pcg(p.x + pcg(p.y + pcg(frame)));
    let b = pcg(a);
    let u = vec3<f32>(f32(a >> 8u), f32(b >> 8u), f32(pcg(b) >> 8u)) / 16777216.0;
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(65504.0));
    let e = max(floor(log2(max(x, vec3<f32>(1.0e-30)))), vec3<f32>(-14.0));
    let step = exp2(e - vec3<f32>(10.0));
    let q = x / step;
    let lo = floor(q);
    return (lo + vec3<f32>(u < (q - lo))) * step;
}

fn inside(q: vec2<i32>) -> bool {
    return all(q >= vec2<i32>(0)) && all(q < vec2<i32>(params.size.xy));
}

// How much the distance changes from one pixel to the next about `p`: the
// smaller one-sided difference on each axis, so an edge beside the pixel
// does not count.
fn depth_slope(p: vec2<i32>, d: f32) -> f32 {
    var slope = 0.0;
    for (var axis = 0; axis < 2; axis++) {
        let o = select(vec2<i32>(0, 1), vec2<i32>(1, 0), axis == 0);
        var best = 1e9;
        for (var s = -1; s <= 1; s += 2) {
            let q = p + o * s;
            if (inside(q)) {
                let dq = textureLoad(guide, q, 0).z;
                if (dq > 0.0) {
                    best = min(best, abs(dq - d));
                }
            }
        }
        if (best < 1e9) {
            slope = max(slope, best);
        }
    }
    return slope;
}

// The edge stop between the pixel (normal `n`, distance `d`, slope `slope`)
// and one `pixels` away with guide `g`.
fn geometry_weight(n: vec3<f32>, d: f32, slope: f32, g: vec4<f32>, pixels: f32) -> f32 {
    if (g.z <= 0.0) {
        return 0.0;
    }
    let w_n = pow(max(dot(n, unpack_normal(g.xy)), 0.0), NORMAL_POWER);
    let w_z = exp(-abs(g.z - d) / (slope * pixels + 0.002 * d + 1e-3));
    return w_n * w_z;
}

fn tiles_wide() -> u32 {
    return (u32(params.size.x) + 7u) / 8u;
}

// The variance of pixel `p`, written with its value to `output`.
fn variance_at(p: vec2<i32>) -> f32 {
    let g = textureLoad(guide, p, 0);
    let c = textureLoad(input, p, 0);
    if (g.z <= 0.0) {
        textureStore(output, p, vec4<f32>(c.rgb, 0.0));
        return 0.0;
    }
    let mean = luma(c.rgb);
    var variance = max(g.w - mean * mean, 0.0);
    if (c.a < SHORT_HISTORY) {
        // SVGF's spatial estimate: the moments of the neighbours on the
        // same surface, 7×7
        let n = unpack_normal(g.xy);
        let slope = depth_slope(p, g.z);
        var m1 = 0.0;
        var m2 = 0.0;
        var w_sum = 0.0;
        for (var y = -3; y <= 3; y++) {
            for (var x = -3; x <= 3; x++) {
                let q = p + vec2<i32>(x, y);
                if (!inside(q)) {
                    continue;
                }
                let gq = textureLoad(guide, q, 0);
                let w = geometry_weight(n, g.z, slope, gq, length(vec2<f32>(f32(x), f32(y))));
                if (w <= 0.0) {
                    continue;
                }
                m1 += luma(textureLoad(input, q, 0).rgb) * w;
                m2 += gq.w * w;
                w_sum += w;
            }
        }
        if (w_sum > 0.0) {
            m1 /= w_sum;
            m2 /= w_sum;
            variance = max(m2 - m1 * m1, 0.0) * SHORT_HISTORY / max(c.a, 1.0);
        }
    }
    textureStore(output, p, vec4<f32>(c.rgb, variance));
    return variance;
}

@compute @workgroup_size(8, 8)
fn cs_variance(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) tile: vec3<u32>,
) {
    if (local == 0u) {
        atomicStore(&tile_noisy, 0u);
    }
    workgroupBarrier();
    let p = vec2<i32>(id.xy);
    if (inside(p) && variance_at(p) > params.size.w) {
        atomicOr(&tile_noisy, 1u);
    }
    workgroupBarrier();
    if (local == 0u) {
        tiles[tile.x + tile.y * tiles_wide()] = atomicLoad(&tile_noisy);
    }
}

// Whether tile `t` and the eight around it have no variance.
fn calm(t: vec2<i32>) -> bool {
    let wide = i32(tiles_wide());
    let high = (i32(params.size.y) + 7) / 8;
    for (var y = max(t.y - 1, 0); y <= min(t.y + 1, high - 1); y++) {
        for (var x = max(t.x - 1, 0); x <= min(t.x + 1, wide - 1); x++) {
            if (tiles[x + y * wide] != 0u) {
                return false;
            }
        }
    }
    return true;
}

@compute @workgroup_size(8, 8)
fn cs_atrous(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) tile: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p)) {
        return;
    }
    if (calm(vec2<i32>(tile.xy))) {
        if (params.mode.z < 0.5) {
            textureStore(output, p, textureLoad(input, p, 0));
        }
        return;
    }
    let c = textureLoad(input, p, 0);
    let g = textureLoad(guide, p, 0);
    if (g.z <= 0.0) {
        textureStore(output, p, c);
        return;
    }
    // the variance blurred 3×3 (SVGF): one pixel's is noisy itself
    var v = 0.0;
    var v_w = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = p + vec2<i32>(x, y);
            if (inside(q)) {
                let k = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
                v += textureLoad(input, q, 0).a * k;
                v_w += k;
            }
        }
    }
    // no noise about it (lit through, or shadowed through, for long
    // enough): nothing to filter, which is most of the image
    if (v <= params.size.w * v_w) {
        textureStore(output, p, c);
        return;
    }
    let sigma = SIGMA_LUMA * sqrt(max(v / v_w, 0.0)) + 1e-4;
    let step = i32(params.size.z);
    let n = unpack_normal(g.xy);
    let slope = depth_slope(p, g.z);
    let l = luma(c.rgb);

    var sum = vec3<f32>(0.0);
    var variance = 0.0;
    var w_sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = p + vec2<i32>(x, y) * step;
            if (!inside(q)) {
                continue;
            }
            let cq = textureLoad(input, q, 0);
            let gq = textureLoad(guide, q, 0);
            let pixels = length(vec2<f32>(f32(x), f32(y))) * f32(step);
            let k = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
            let w = k * geometry_weight(n, g.z, slope, gq, pixels)
                * exp(-abs(luma(cq.rgb) - l) / sigma);
            sum += cq.rgb * w;
            variance += cq.a * w * w;
            w_sum += w;
        }
    }
    if (w_sum <= 1e-6) {
        textureStore(output, p, c);
        return;
    }
    var value = sum / w_sum;
    if (params.mode.y > 0.5) {
        value = dithered(value, vec2<u32>(id.xy), u32(params.mode.y));
    }
    textureStore(output, p, vec4<f32>(value, variance / (w_sum * w_sum)));
}

// The share's variance (x) and the sun's (y) at pixel `p`, written with
// their values to `output`.
fn variance_pair_at(p: vec2<i32>) -> vec2<f32> {
    let g = textureLoad(guide, p, 0);
    let c = textureLoad(input, p, 0);
    if (g.z <= 0.0) {
        textureStore(output, p, vec4<f32>(c.rg, 0.0, 0.0));
        return vec2<f32>(0.0);
    }
    var variance = vec2<f32>(max(g.w - c.r * c.r, 0.0), c.g * (1.0 - c.g));
    if (min(c.a, c.b) < SHORT_HISTORY) {
        let n = unpack_normal(g.xy);
        let slope = depth_slope(p, g.z);
        var m1 = vec2<f32>(0.0);
        var m2 = 0.0;
        var w_sum = 0.0;
        for (var y = -3; y <= 3; y++) {
            for (var x = -3; x <= 3; x++) {
                let q = p + vec2<i32>(x, y);
                if (!inside(q)) {
                    continue;
                }
                let gq = textureLoad(guide, q, 0);
                let w = geometry_weight(n, g.z, slope, gq, length(vec2<f32>(f32(x), f32(y))));
                if (w <= 0.0) {
                    continue;
                }
                m1 += textureLoad(input, q, 0).rg * w;
                m2 += gq.w * w;
                w_sum += w;
            }
        }
        if (w_sum > 0.0) {
            m1 /= w_sum;
            m2 /= w_sum;
            if (c.a < SHORT_HISTORY) {
                variance.x = max(m2 - m1.x * m1.x, 0.0) * SHORT_HISTORY / max(c.a, 1.0);
            }
            if (c.b < SHORT_HISTORY) {
                variance.y = max(variance.y, m1.y * (1.0 - m1.y)) * SHORT_HISTORY / max(c.b, 1.0);
            }
        }
    }
    textureStore(output, p, vec4<f32>(c.rg, variance));
    return variance;
}

@compute @workgroup_size(8, 8)
fn cs_variance_pair(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) tile: vec3<u32>,
) {
    if (local == 0u) {
        atomicStore(&tile_noisy, 0u);
    }
    workgroupBarrier();
    let p = vec2<i32>(id.xy);
    if (inside(p)) {
        let v = variance_pair_at(p);
        if (max(v.x, v.y) > params.size.w) {
            atomicOr(&tile_noisy, 1u);
        }
    }
    workgroupBarrier();
    if (local == 0u) {
        tiles[tile.x + tile.y * tiles_wide()] = atomicLoad(&tile_noisy);
    }
}

// A pixel of the pair's image, and of its tint's where there is one.
fn store_pair(p: vec2<i32>, c: vec4<f32>, tint: vec4<f32>) {
    textureStore(output, p, c);
    if (params.mode.x > 0.5) {
        textureStore(output_tint, p, tint);
    }
}

@compute @workgroup_size(8, 8)
fn cs_atrous_pair(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) tile: vec3<u32>) {
    let p = vec2<i32>(id.xy);
    if (!inside(p)) {
        return;
    }
    let tinted = params.mode.x > 0.5;
    if (calm(vec2<i32>(tile.xy))) {
        if (params.mode.z < 0.5) {
            textureStore(output, p, textureLoad(input, p, 0));
        }
        if (tinted && params.mode.w < 0.5) {
            textureStore(output_tint, p, textureLoad(input_tint, p, 0));
        }
        return;
    }
    let c = textureLoad(input, p, 0);
    var t = vec4<f32>(0.0);
    if (tinted) {
        t = textureLoad(input_tint, p, 0);
    }
    let g = textureLoad(guide, p, 0);
    if (g.z <= 0.0) {
        store_pair(p, c, t);
        return;
    }
    var v = vec2<f32>(0.0);
    var v_w = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = p + vec2<i32>(x, y);
            if (inside(q)) {
                let k = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
                v += textureLoad(input, q, 0).ba * k;
                v_w += k;
            }
        }
    }
    let noisy = v > vec2<f32>(params.size.w * v_w);
    if (!any(noisy)) {
        store_pair(p, c, t);
        return;
    }
    let sigma = SIGMA_LUMA * sqrt(max(v / v_w, vec2<f32>(0.0))) + vec2<f32>(1e-4);
    let step = i32(params.size.z);
    let n = unpack_normal(g.xy);
    let slope = depth_slope(p, g.z);

    var sum = vec2<f32>(0.0);
    var variance = vec2<f32>(0.0);
    var w_sum = vec2<f32>(0.0);
    var tint = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = p + vec2<i32>(x, y) * step;
            if (!inside(q)) {
                continue;
            }
            let cq = textureLoad(input, q, 0);
            let gq = textureLoad(guide, q, 0);
            let pixels = length(vec2<f32>(f32(x), f32(y))) * f32(step);
            let k = select(0.5, 1.0, x == 0) * select(0.5, 1.0, y == 0);
            let w = k * geometry_weight(n, g.z, slope, gq, pixels) * exp(-abs(cq.rg - c.rg) / sigma);
            sum += cq.rg * w;
            variance += cq.ba * w * w;
            w_sum += w;
            // the colours are the share's own samples: its weights are theirs
            if (tinted && noisy.x) {
                tint += textureLoad(input_tint, q, 0).rgb * w.x;
            }
        }
    }
    let filtered = vec2<bool>(noisy.x && w_sum.x > 1e-6, noisy.y && w_sum.y > 1e-6);
    let safe = max(w_sum, vec2<f32>(1e-6));
    store_pair(
        p,
        vec4<f32>(select(c.rg, sum / safe, filtered), select(c.ba, variance / (safe * safe), filtered)),
        vec4<f32>(select(t.rgb, tint / safe.x, filtered.x), t.a),
    );
}
