// Photo looks: the Legacy Viewer's snapshot filters on the GPU. Port of
// LLImageFilter (indra/llimage/llimagefilter.cpp, originally LGPL 2.1): a
// filter is a list of operations on the photo's 8-bit gamma-encoded image,
// each blended through the current procedural stencil. The image is worked in
// 0..255 levels truncated after every operation, as the U8 image there, and in
// its coordinates: `i` from the left, `j` from the bottom (LLImageRaw holds
// the rows bottom up). An operation that reads its neighbours (a 3 x 3
// convolution) starts a pass of its own; looks.rs plans the passes.

struct Op {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    d: vec4<f32>,
};

// One pass (looks.rs `PassU`).
struct Pass {
    // the photo's origin in the target (pixels), its size in photo pixels
    image: vec4<f32>,
    // target pixels per photo pixel
    scale: f32,
    start: u32,
    end: u32,
    flags: u32,
    intensity: f32,
    // the original left of this share of the photo's width (< 0: none)
    compare: f32,
    _pad: vec2<f32>,
};

const SRC_SRGB: u32 = 1u;
const FINAL: u32 = 2u;
const DST_SRGB: u32 = 4u;

const OP_STENCIL: f32 = 0.0;
const OP_MATRIX: f32 = 1.0;
const OP_CURVE: f32 = 2.0;
const OP_HISTO: f32 = 3.0;
const OP_SCREEN: f32 = 4.0;
const OP_CONVOLVE: f32 = 5.0;

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var original: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;
@group(0) @binding(3) var<storage, read> ops: array<Op>;
@group(0) @binding(4) var<storage, read> luts: array<u32>;
@group(0) @binding(5) var<uniform> pass_u: Pass;

struct Out {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_look(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return o;
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// a / b rounded like the CPU's division: the GPU's may be an ulp off, and a
// level truncated from 69.99999 instead of 70 is one lower (a blur's mean,
// the stretch of filterLinearize). One correction step with the exact
// remainder.
fn div(a: f32, b: f32) -> f32 {
    let q = a / b;
    return q + fma(-q, b, a) / b;
}

// A texel in levels: gamma-encoded, rounded to the 8-bit level it holds.
fn levels(c: vec3<f32>, srgb: bool) -> vec3<f32> {
    var e = c;
    if (srgb) {
        e = to_srgb(c);
    }
    return round(clamp(e, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0);
}

// The current stencil (setStencil): its shape, blend mode, range and the
// values its parameters give for the photo's size.
struct Stencil {
    shape: f32,
    mode: f32,
    lo: f32,
    hi: f32,
    p: vec4<f32>,
};

// getStencilAlpha at photo pixel (i, j) of a photo `size` wide and high.
fn stencil_alpha(s: Stencil, i: f32, j: f32, size: vec2<f32>) -> f32 {
    let w = size.x;
    let h = size.y;
    var alpha = 1.0;
    if (s.shape == 2.0) {
        // vignette: integer centre and width, as the S32 there
        let cx = trunc(trunc(w + s.p.x * h) / 2.0);
        let cy = trunc(trunc(h + s.p.y * h) / 2.0);
        let width = trunc(trunc(s.p.z * h) / 2.0);
        let gamma = select(s.p.w, 1.0, s.p.w <= 0.0);
        let d2 = (i - cx) * (i - cx) + (j - cy) * (j - cy);
        alpha = exp(-(pow(d2 / (width * width), gamma) / 2.0));
    } else if (s.shape == 3.0) {
        // scan lines
        let wavelength = select(s.p.x * h / 2.0, 10.0, s.p.x <= 0.0);
        let angle = radians(s.p.y);
        let d = sin(angle) * i - cos(angle) * j;
        alpha = select(0.0, 1.0, sin(2.0 * 3.14159265 * d / wavelength) > 0.0);
    } else if (s.shape == 1.0) {
        // gradient
        let start = vec2<f32>((w + s.p.x * h) / 2.0, (h + s.p.y * h) / 2.0);
        let end = vec2<f32>((w + s.p.z * h) / 2.0, (h + s.p.w * h) / 2.0);
        let g = end - start;
        alpha = clamp(div((i - start.x) * g.x + (j - start.y) * g.y, g.x * g.x + g.y * g.y), 0.0, 1.0);
    }
    return s.lo + alpha * (s.hi - s.lo);
}

// blendStencil: an operation's colour `c` laid on the pixel `p`.
fn blend(mode: f32, alpha: f32, p: vec3<f32>, c: vec3<f32>) -> vec3<f32> {
    var v: vec3<f32>;
    if (mode == 1.0) {
        v = p + alpha * c; // add
    } else if (mode == 2.0) {
        v = (1.0 - alpha) * p + c; // add back
    } else if (mode == 3.0) {
        v = alpha * c; // fade
    } else {
        v = (1.0 - alpha) * p + alpha * c;
    }
    return floor(clamp(v, vec3<f32>(0.0), vec3<f32>(255.0)));
}

// The look-up tables of colorCorrect, worked out for one channel's level.
fn curve(kind: f32, param: f32, color: f32, alpha: f32, i: f32) -> f32 {
    if (kind == 1.0) {
        // filterColorize
        return clamp(trunc((1.0 - alpha) * i + 255.0 * alpha * color), 0.0, 255.0);
    }
    var value: f32;
    if (kind == 0.0) {
        // filterGamma
        value = 255.0 * clamp(pow(div(i, 255.0), div(1.0, param)), 0.0, 1.0);
    } else if (kind == 2.0) {
        // filterContrast
        value = clamp(trunc(param * i + 128.0 * (1.0 - param)), 0.0, 255.0);
    } else {
        // filterBrightness (darken: a negative step)
        value = clamp(i + trunc(param * 255.0), 0.0, 255.0);
    }
    return floor((1.0 - alpha) * i + alpha * value);
}

fn histo_curve(lut: u32, alpha: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let base = lut * 256u;
    let v = vec3<f32>(
        f32(luts[base + u32(p.r)]),
        f32(luts[base + u32(p.g)]),
        f32(luts[base + u32(p.b)]),
    );
    return floor((vec3<f32>(1.0) - alpha) * p + alpha * v);
}

// filterScreen on the red level of the pixel: a grey.
fn screen(op: Op, red: f32, i: f32, j: f32, h: f32) -> f32 {
    let wavelength = op.a.z * h / 2.0;
    let angle = radians(op.a.w);
    let s = sin(angle);
    let c = cos(angle);
    var value: f32;
    if (op.a.y == 0.0) {
        let di = c * i + s * j;
        let dj = -s * i + c * j;
        value = (sin(2.0 * 3.14159265 * di / wavelength) * sin(2.0 * 3.14159265 * dj / wavelength) + 1.0) * 255.0 / 2.0;
    } else {
        let dj = s * i - c * j;
        value = (sin(2.0 * 3.14159265 * dj / wavelength) + 1.0) * 255.0 / 2.0;
    }
    let cut = floor(value);
    if (red < cut) {
        return 0.0;
    }
    // the gamma table that softens the screen's edges
    return floor(255.0 * clamp(pow(div(red - cut, 255.0), 0.25), 0.0, 1.0));
}

// The neighbour (di, dj) photo pixels away (dj up), in levels.
fn neighbour(frag: vec2<f32>, di: f32, dj: f32, srgb: bool) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let at = frag + vec2<f32>(di, -dj) * pass_u.scale;
    return levels(textureSampleLevel(src, src_sampler, at / size, 0.0).rgb, srgb);
}

@fragment
fn fs_look(in: Out) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let src_srgb = (pass_u.flags & SRC_SRGB) != 0u;
    let size = pass_u.image.zw;
    // the photo pixel under this one; outside the photo, the scene as it is
    let q = (in.clip.xy - pass_u.image.xy) / pass_u.scale - vec2<f32>(0.5);
    let inside = all(q > vec2<f32>(-0.5)) && all(q < size - vec2<f32>(0.5));
    let i = q.x;
    let j = size.y - 1.0 - q.y;
    let pi = round(i);
    let pj = round(j);

    var p = levels(textureLoad(src, px, 0).rgb, src_srgb);
    if (inside) {
        var st = Stencil(0.0, 0.0, 0.0, 1.0, vec4<f32>(0.0));
        for (var k = 0u; k < pass_u.end; k++) {
            let op = ops[k];
            let kind = op.a.x;
            if (kind == OP_STENCIL) {
                st = Stencil(op.a.y, op.a.z, op.a.w, op.b.x, op.c);
                continue;
            }
            if (k < pass_u.start) {
                continue;
            }
            var c: vec3<f32>;
            if (kind == OP_MATRIX) {
                c = floor(clamp(vec3<f32>(dot(op.b.xyz, p), dot(op.c.xyz, p), dot(op.d.xyz, p)), vec3<f32>(0.0), vec3<f32>(255.0)));
            } else if (kind == OP_CURVE) {
                let alpha = op.c.xyz;
                c = vec3<f32>(
                    curve(op.a.y, op.a.z, op.b.x, alpha.x, p.r),
                    curve(op.a.y, op.a.z, op.b.y, alpha.y, p.g),
                    curve(op.a.y, op.a.z, op.b.z, alpha.z, p.b),
                );
            } else if (kind == OP_HISTO) {
                c = histo_curve(u32(op.a.y), op.b.xyz, p);
            } else if (kind == OP_SCREEN) {
                c = vec3<f32>(screen(op, p.r, i, j, size.y));
            } else {
                // a convolution: always the first operation of its pass, read
                // from the pass's source; the border of the photo goes black
                let last_row = pj >= size.y - 1.0;
                let border = pi <= 0.0 || pj <= 0.0 || pi >= size.x - 1.0 || last_row;
                if (border) {
                    // the last line takes the stencil of the first one
                    let aj = select(j, 0.0, last_row);
                    p = blend(st.mode, stencil_alpha(st, i, aj, size), p, vec3<f32>(0.0));
                    continue;
                }
                var sum = vec3<f32>(0.0);
                var kernel = array<f32, 9>(op.b.x, op.b.y, op.b.z, op.b.w, op.c.x, op.c.y, op.c.z, op.c.w, op.d.x);
                for (var r = 0; r < 3; r++) {
                    for (var col = 0; col < 3; col++) {
                        let n = neighbour(in.clip.xy, f32(col - 1), f32(r - 1), src_srgb);
                        sum += kernel[r * 3 + col] * n;
                    }
                }
                if (op.a.z > 0.0) {
                    sum = abs(sum);
                }
                if (op.a.y > 0.0) {
                    let n = sum - op.a.w;
                    sum = vec3<f32>(div(n.x, op.d.y), div(n.y, op.d.y), div(n.z, op.d.y));
                }
                c = floor(clamp(sum, vec3<f32>(0.0), vec3<f32>(255.0)));
            }
            p = blend(st.mode, stencil_alpha(st, i, j, size), p, c);
        }
    }

    if ((pass_u.flags & FINAL) == 0u) {
        return vec4<f32>(p / 255.0, 1.0);
    }
    // the last pass: the look at its intensity over the original, the
    // original kept left of the comparison line, and its coverage
    let o = textureLoad(original, px, 0);
    // the original has the target's format
    let before = levels(o.rgb, (pass_u.flags & DST_SRGB) != 0u);
    var v = mix(before, p, clamp(pass_u.intensity, 0.0, 1.0));
    if (!inside || (pass_u.compare >= 0.0 && q.x < pass_u.compare * size.x)) {
        v = before;
    }
    var rgb = v / 255.0;
    if ((pass_u.flags & DST_SRGB) != 0u) {
        rgb = to_linear(rgb);
    }
    return vec4<f32>(rgb, o.a);
}

// ---- the brightness histogram of filterLinearize / filterEqualize (its own
// bindings: the compute pipelines have a layout of their own)

@group(0) @binding(8) var hist_src: texture_2d<f32>;
@group(0) @binding(9) var<storage, read_write> bins: array<atomic<u32>, 256>;

struct HistU {
    image: vec4<f32>,
    scale: f32,
    flags: u32,
    count: u32,
    _pad: u32,
    // per curve: x its operation, y linearize 0 / posterize 1
    curves: array<vec4<u32>, 8>,
};

@group(0) @binding(10) var<uniform> hist_u: HistU;
@group(0) @binding(11) var<storage, read> hist_ops: array<Op>;
@group(0) @binding(12) var<storage, read_write> hist_luts: array<u32>;

var<workgroup> local_bins: array<atomic<u32>, 256>;

// Counts (r + g + b) / 3 of the photo's pixels (computeHistograms).
@compute @workgroup_size(16, 16)
fn cs_histogram(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    atomicStore(&local_bins[li], 0u);
    workgroupBarrier();
    let dims = textureDimensions(hist_src);
    if (gid.x < dims.x && gid.y < dims.y) {
        let q = (vec2<f32>(gid.xy) + vec2<f32>(0.5) - hist_u.image.xy) / hist_u.scale - vec2<f32>(0.5);
        if (all(q > vec2<f32>(-0.5)) && all(q < hist_u.image.zw - vec2<f32>(0.5))) {
            let p = levels(textureLoad(hist_src, vec2<i32>(gid.xy), 0).rgb, (hist_u.flags & SRC_SRGB) != 0u);
            let brightness = (u32(p.r) + u32(p.g) + u32(p.b)) / 3u;
            atomicAdd(&local_bins[brightness], 1u);
        }
    }
    workgroupBarrier();
    let n = atomicLoad(&local_bins[li]);
    if (n > 0u) {
        atomicAdd(&bins[li], n);
    }
}

// One look-up table per histogram curve: filterLinearize's stretch or
// filterEqualize's classes, from the cumulated histogram.
@compute @workgroup_size(8)
fn cs_lut(@builtin(local_invocation_index) t: u32) {
    if (t >= hist_u.count) {
        return;
    }
    let curve = hist_u.curves[t];
    let op = hist_ops[curve.x];
    var cumulated: array<u32, 256>;
    var sum = 0u;
    for (var i = 0u; i < 256u; i++) {
        sum += atomicLoad(&bins[i]);
        cumulated[i] = sum;
    }
    let total = cumulated[255];
    let base = t * 256u;
    if (curve.y == 0u) {
        let tail = clamp(op.a.z, 0.0, 1.0);
        let min_c = u32(f32(total) * tail);
        let max_c = u32(f32(total) * (1.0 - tail));
        var min_v = 0u;
        while (min_v < 255u && cumulated[min_v] < min_c) {
            min_v++;
        }
        var max_v = 255u;
        while (max_v > 0u && cumulated[max_v] > max_c) {
            max_v--;
        }
        if (max_v == min_v) {
            for (var i = 0u; i < 256u; i++) {
                hist_luts[base + i] = select(255u, 0u, i < min_v);
            }
        } else {
            let slope = div(255.0, f32(max_v) - f32(min_v));
            let translate = -f32(min_v) * slope;
            for (var i = 0u; i < 256u; i++) {
                hist_luts[base + i] = u32(clamp(trunc(slope * f32(i) + translate), 0.0, 255.0));
            }
        }
    } else {
        let classes = u32(clamp(op.a.z, 2.0, 255.0));
        let delta_count = total / classes;
        var current_count = delta_count;
        let delta_value = 256u / (classes - 1u);
        var current_value = 0u;
        for (var i = 0u; i < 256u; i++) {
            hist_luts[base + i] = current_value;
            if (cumulated[i] >= current_count) {
                current_count += delta_count;
                current_value = min(current_value + delta_value, 255u);
            }
        }
    }
}
