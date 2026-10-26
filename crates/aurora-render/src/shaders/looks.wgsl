// Photo looks: the Legacy Viewer's snapshot filters on the GPU. Port of
// LLImageFilter (indra/llimage/llimagefilter.cpp, originally LGPL 2.1): a
// filter is a list of operations on the photo's 8-bit gamma-encoded image,
// each blended through the current procedural stencil. The image is worked in
// 0..255 levels truncated after every operation, as the U8 image there, and in
// its coordinates: `i` from the left, `j` from the bottom (LLImageRaw holds
// the rows bottom up). An operation that reads its neighbours (a 3 x 3
// convolution) starts a pass of its own; looks.rs plans the passes.
//
// Aurora's own operations (kinds from 6) follow the same rules: worked in
// levels and in the photo's pixels, blended through the stencil, and those
// that read around the pixel (chromatic aberration, distortion, the wide
// gathers) start a pass. Sizes are shares of the photo's height, so that the
// live view, the photo, its tiles and its supersampling show the same. Three
// read the scene's depth (fog, blur by distance, subject), from the focus
// distance of the frame (`focus`, found by cs_focus or given by the app),
// and one a 3D look-up table (a .cube file).

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
    // the layer's first operation: its stencil starts there
    layer_start: u32,
    _pad: f32,
    // the frame's inverse projection (view distance of a depth value)
    inv_proj: mat4x4<f32>,
};

const SRC_SRGB: u32 = 1u;
const FINAL: u32 = 2u;
const DST_SRGB: u32 = 4u;
// the scene's depth is bound (not the tests' blank one)
const DEPTH_OK: u32 = 8u;
const LAYER_SRGB: u32 = 16u;
const LAST: u32 = 32u;
const SCENE_SRGB: u32 = 64u;

const OP_STENCIL: f32 = 0.0;
const OP_MATRIX: f32 = 1.0;
const OP_CURVE: f32 = 2.0;
const OP_HISTO: f32 = 3.0;
const OP_SCREEN: f32 = 4.0;
const OP_CONVOLVE: f32 = 5.0;
const OP_GRAIN: f32 = 6.0;
const OP_CHROMA: f32 = 7.0;
const OP_DISTORT: f32 = 8.0;
const OP_BARS: f32 = 9.0;
const OP_FRAME: f32 = 10.0;
const OP_SELECTIVE: f32 = 11.0;
const OP_GRADIENT_MAP: f32 = 12.0;
const OP_PIXELATE: f32 = 13.0;
const OP_GLITCH: f32 = 14.0;
const OP_PAINT: f32 = 15.0;
const OP_CLARITY: f32 = 16.0;
const OP_GLOW: f32 = 17.0;
const OP_FOG: f32 = 18.0;
const OP_DEPTH_BLUR: f32 = 19.0;
const OP_SUBJECT: f32 = 20.0;
const OP_LUT: f32 = 21.0;
const OP_SPLIT: f32 = 22.0;

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var original: texture_2d<f32>;
@group(0) @binding(2) var src_sampler: sampler;
@group(0) @binding(3) var<storage, read> ops: array<Op>;
@group(0) @binding(4) var<storage, read> luts: array<u32>;
@group(0) @binding(5) var<uniform> pass_u: Pass;
@group(0) @binding(6) var depth_tex: texture_depth_2d;
@group(0) @binding(7) var lut_tex: texture_3d<f32>;
// x: the focus distance along the view axis (m)
@group(0) @binding(8) var<storage, read> focus: vec4<f32>;
// the scene before every layer (comparison, coverage)
@group(0) @binding(9) var scene: texture_2d<f32>;

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

// ---- Aurora's operations

fn luma(p: vec3<f32>) -> f32 {
    return dot(p, vec3<f32>(0.2126, 0.7152, 0.0722)) / 255.0;
}

fn hash(v: vec2<f32>) -> f32 {
    let h = dot(v, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

// The source at a point of the photo (its pixels, from the top left), the
// photo's edge repeated beyond it.
fn at_photo(q: vec2<f32>, srgb: bool) -> vec3<f32> {
    let qc = clamp(q, vec2<f32>(0.0), pass_u.image.zw - vec2<f32>(1.0));
    let t = pass_u.image.xy + (qc + vec2<f32>(0.5)) * pass_u.scale;
    return levels(textureSampleLevel(src, src_sampler, t / vec2<f32>(textureDimensions(src)), 0.0).rgb, srgb);
}

// The k-th of n points of a disc of radius 1 (a Vogel spiral).
fn disc(k: u32, n: u32) -> vec2<f32> {
    let r = sqrt((f32(k) + 0.5) / f32(n));
    let a = f32(k) * 2.39996323;
    return r * vec2<f32>(cos(a), sin(a));
}

// Distance along the view axis of the scene at a pixel of the frame.
fn view_depth(frag: vec2<f32>) -> f32 {
    if ((pass_u.flags & DEPTH_OK) == 0u) {
        return 1e9;
    }
    let dims = vec2<f32>(textureDimensions(depth_tex));
    let px = clamp(vec2<i32>(frag), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    let d = textureLoad(depth_tex, px, 0);
    let uv = frag / dims;
    let p = pass_u.inv_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    if (abs(p.w) < 1e-9) {
        return 1e9;
    }
    return max(-p.z / p.w, 1e-3);
}

// The hue (degrees) and saturation of a colour in levels.
fn hue(p: vec3<f32>) -> vec2<f32> {
    let c = p / 255.0;
    let mx = max(c.r, max(c.g, c.b));
    let mn = min(c.r, min(c.g, c.b));
    let d = mx - mn;
    var h = 0.0;
    if (d > 1e-5) {
        if (mx == c.r) {
            h = (c.g - c.b) / d;
        } else if (mx == c.g) {
            h = 2.0 + (c.b - c.r) / d;
        } else {
            h = 4.0 + (c.r - c.g) / d;
        }
    }
    h = fract(h / 6.0 + 1.0) * 360.0;
    return vec2<f32>(h, select(0.0, d / mx, mx > 0.0));
}

// One of Aurora's operations at photo point `q` of a photo `size`, `frag`
// the pixel of the frame: its colour before the stencil.
fn aurora(op: Op, p: vec3<f32>, q: vec2<f32>, size: vec2<f32>, frag: vec2<f32>, srgb: bool) -> vec3<f32> {
    let kind = op.a.x;
    let h = size.y;
    let center = size * 0.5;
    var c = p;
    if (kind == OP_GRAIN) {
        // film grain, strongest in the middle tones
        let cell = floor(q / max(op.a.z, 1.0));
        let l = luma(p);
        let w = 0.35 + 2.6 * l * (1.0 - l);
        if (op.a.w > 0.0) {
            let n = hash(cell) + hash(cell + vec2<f32>(17.3, 5.1)) - 1.0;
            c = p + vec3<f32>(op.a.y * 255.0 * w * n);
        } else {
            let n = vec3<f32>(
                hash(cell) + hash(cell + vec2<f32>(17.3, 5.1)),
                hash(cell + vec2<f32>(3.7, 41.2)) + hash(cell + vec2<f32>(9.1, 2.8)),
                hash(cell + vec2<f32>(23.9, 7.7)) + hash(cell + vec2<f32>(1.3, 33.3)),
            ) - vec3<f32>(1.0);
            c = p + op.a.y * 255.0 * w * n;
        }
    } else if (kind == OP_CHROMA) {
        // red and blue spread apart towards the edges
        let v = q - center;
        let k = op.a.y;
        c = vec3<f32>(at_photo(center + v * (1.0 + k), srgb).r, p.g, at_photo(center + v * (1.0 - k), srgb).b);
    } else if (kind == OP_DISTORT) {
        // barrel (k > 0) or pincushion, the corners kept
        let v = q - center;
        let n = v / length(center);
        let r2 = dot(n, n);
        c = at_photo(center + v * (1.0 + op.a.y * (r2 - 1.0)), srgb);
    } else if (kind == OP_BARS) {
        // cinema bars for a frame `ratio` wide
        let ratio = op.a.y;
        let qc = q + vec2<f32>(0.5);
        var bar = false;
        if (size.x / h < ratio) {
            bar = abs(qc.y - center.y) > size.x / ratio * 0.5;
        } else {
            bar = abs(qc.x - center.x) > h * ratio * 0.5;
        }
        c = select(p, op.b.rgb * 255.0, bar);
    } else if (kind == OP_FRAME) {
        // a border, wider at the bottom (an instant photo)
        let side = op.a.y * min(size.x, h);
        let bottom = op.a.z * min(size.x, h);
        let qc = q + vec2<f32>(0.5);
        let inside = qc.x >= side && qc.x <= size.x - side && qc.y >= side && qc.y <= h - bottom;
        c = select(op.b.rgb * 255.0, p, inside);
    } else if (kind == OP_SELECTIVE) {
        // one hue kept, the rest in grey
        let hs = hue(p);
        let d = abs(fract((hs.x - op.a.y) / 360.0 + 0.5) - 0.5) * 360.0;
        let keep = (1.0 - smoothstep(op.a.z, op.a.z + op.a.w, d)) * smoothstep(0.08, 0.25, hs.y);
        c = mix(vec3<f32>(luma(p) * 255.0), p, keep);
    } else if (kind == OP_GRADIENT_MAP) {
        // the brightness through three colours
        let l = luma(p);
        let g = select(mix(op.c.rgb, op.d.rgb, l * 2.0 - 1.0), mix(op.b.rgb, op.c.rgb, l * 2.0), l < 0.5);
        c = g * 255.0;
    } else if (kind == OP_SPLIT) {
        // split toning: a colour in the shadows, another in the lights
        let l = luma(p);
        let shift = (1.0 - l) * (op.b.rgb - vec3<f32>(0.5)) + l * (op.c.rgb - vec3<f32>(0.5));
        c = p + op.a.y * 255.0 * shift;
    } else if (kind == OP_PIXELATE) {
        let block = max(op.a.y * h, 1.0);
        c = at_photo((floor(q / block) + vec2<f32>(0.5)) * block, srgb);
    } else if (kind == OP_GLITCH) {
        // bands torn sideways, the channels apart
        let band = floor(q.y / max(op.a.z * h, 1.0));
        let r = hash(vec2<f32>(band, op.a.w));
        let tear = select(0.0, (hash(vec2<f32>(op.a.w, band)) - 0.5) * 2.0 * op.a.y * h, r > 0.65);
        let split = op.a.y * h * 0.35;
        c = vec3<f32>(
            at_photo(q + vec2<f32>(tear + split, 0.0), srgb).r,
            at_photo(q + vec2<f32>(tear, 0.0), srgb).g,
            at_photo(q + vec2<f32>(tear - split, 0.0), srgb).b,
        );
    } else if (kind == OP_PAINT) {
        // Kuwahara: the mean of the calmest of four quadrants (oil paint)
        let radius = i32(clamp(round(op.a.y * h), 1.0, 5.0));
        var best = p;
        var lowest = 1e9;
        for (var quad = 0; quad < 4; quad++) {
            let sx = select(-1.0, 1.0, (quad & 1) == 1);
            let sy = select(-1.0, 1.0, (quad & 2) == 2);
            var sum = vec3<f32>(0.0);
            var sum2 = 0.0;
            var count = 0.0;
            for (var y = 0; y <= radius; y++) {
                for (var x = 0; x <= radius; x++) {
                    let s = at_photo(q + vec2<f32>(sx * f32(x), sy * f32(y)), srgb);
                    let l = luma(s);
                    sum += s;
                    sum2 += l * l;
                    count += 1.0;
                }
            }
            let mean = sum / count;
            let lm = luma(mean);
            let variance = sum2 / count - lm * lm;
            if (variance < lowest) {
                lowest = variance;
                best = mean;
            }
        }
        c = best;
    } else if (kind == OP_CLARITY) {
        // local contrast: the pixel away from the mean around it
        let radius = op.a.z * h;
        var mean = vec3<f32>(0.0);
        for (var k = 0u; k < 16u; k++) {
            mean += at_photo(q + disc(k, 16u) * radius, srgb);
        }
        mean /= 16.0;
        let l = luma(p);
        c = p + op.a.y * (p - mean) * (0.4 + 2.4 * l * (1.0 - l));
    } else if (kind == OP_GLOW) {
        // the light above a threshold spread around and tinted (bloom,
        // halation)
        let radius = op.a.z * h;
        var light = vec3<f32>(0.0);
        for (var k = 0u; k < 24u; k++) {
            let s = at_photo(q + disc(k, 24u) * radius, srgb);
            light += max(s - vec3<f32>(op.a.w * 255.0), vec3<f32>(0.0));
        }
        c = p + op.a.y * op.b.rgb * luma(light / 24.0) * 255.0 / max(1.0 - op.a.w, 0.05);
    } else if (kind == OP_FOG) {
        // haze growing with the distance
        let z = view_depth(frag);
        let f = clamp((z - op.a.y) / max(op.a.z - op.a.y, 1e-3), 0.0, 1.0) * op.a.w;
        c = mix(p, op.b.rgb * 255.0, f);
    } else if (kind == OP_DEPTH_BLUR) {
        // blurred away from the focus distance (behind it only when asked)
        let z = view_depth(frag);
        let zf = focus.x;
        var t = clamp(abs(z - zf) / max(zf * op.a.z, 1e-3), 0.0, 1.0);
        if (op.a.w > 0.0 && z < zf) {
            t = 0.0;
        }
        let radius = t * op.a.y * h;
        if (radius > 0.5) {
            var sum = vec3<f32>(0.0);
            for (var k = 0u; k < 24u; k++) {
                sum += at_photo(q + disc(k, 24u) * radius, srgb);
            }
            c = sum / 24.0;
        }
    } else if (kind == OP_SUBJECT) {
        // what is far from the focus distance greyed and darkened
        let z = view_depth(frag);
        let zf = focus.x;
        let away = smoothstep(op.a.w, op.a.w * 2.0, abs(z - zf) / zf);
        let dim = mix(p, vec3<f32>(luma(p) * 255.0), op.a.y) * (1.0 - op.a.z);
        c = mix(p, dim, away);
    } else if (kind == OP_LUT) {
        let n = f32(textureDimensions(lut_tex).x);
        let coord = p / 255.0 * (n - 1.0) / n + vec3<f32>(0.5 / n);
        c = textureSampleLevel(lut_tex, src_sampler, coord, 0.0).rgb * 255.0;
    }
    return floor(clamp(c, vec3<f32>(0.0), vec3<f32>(255.0)));
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
        for (var k = pass_u.layer_start; k < pass_u.end; k++) {
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
            } else if (kind != OP_CONVOLVE) {
                c = aurora(op, p, q, size, in.clip.xy, src_srgb);
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
    // the layer's last pass: the look at its intensity over the image the
    // layers below left; the last layer's: the scene as it is left of the
    // comparison line and outside the photo, and the scene's coverage
    let o = textureLoad(original, px, 0);
    var v = mix(levels(o.rgb, (pass_u.flags & LAYER_SRGB) != 0u), p, clamp(pass_u.intensity, 0.0, 1.0));
    var alpha = 1.0;
    if ((pass_u.flags & LAST) != 0u) {
        let sc = textureLoad(scene, px, 0);
        alpha = sc.a;
        if (!inside || (pass_u.compare >= 0.0 && q.x < pass_u.compare * size.x)) {
            v = levels(sc.rgb, (pass_u.flags & SCENE_SRGB) != 0u);
        }
    }
    var rgb = v / 255.0;
    if ((pass_u.flags & DST_SRGB) != 0u) {
        rgb = to_linear(rgb);
    }
    return vec4<f32>(rgb, alpha);
}

// ---- the brightness histogram of filterLinearize / filterEqualize (its own
// bindings: the compute pipelines have a layout of their own)

@group(0) @binding(16) var hist_src: texture_2d<f32>;
@group(0) @binding(17) var<storage, read_write> bins: array<atomic<u32>, 256>;

struct HistU {
    image: vec4<f32>,
    scale: f32,
    flags: u32,
    count: u32,
    _pad: u32,
    // per curve: x its operation, y linearize 0 / posterize 1
    curves: array<vec4<u32>, 8>,
    // the inverse projection, and the focus distance given (x > 0) or the
    // point of the frame whose depth gives it (yz)
    inv_proj: mat4x4<f32>,
    focus: vec4<f32>,
};

@group(0) @binding(18) var<uniform> hist_u: HistU;
@group(0) @binding(19) var<storage, read> hist_ops: array<Op>;
@group(0) @binding(20) var<storage, read_write> hist_luts: array<u32>;

@group(0) @binding(21) var focus_depth: texture_depth_2d;
@group(0) @binding(22) var<storage, read_write> focus_out: vec4<f32>;

var<workgroup> local_bins: array<atomic<u32>, 256>;

// The focus distance of the depth operations: the one given, or the scene's
// at the middle of the photo.
@compute @workgroup_size(1)
fn cs_focus() {
    if (hist_u.focus.x > 0.0) {
        focus_out = vec4<f32>(hist_u.focus.x, 0.0, 0.0, 0.0);
        return;
    }
    if ((hist_u.flags & DEPTH_OK) == 0u) {
        focus_out = vec4<f32>(1e9, 0.0, 0.0, 0.0);
        return;
    }
    let dims = vec2<f32>(textureDimensions(focus_depth));
    let frag = hist_u.focus.yz;
    let px = clamp(vec2<i32>(frag), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
    let d = textureLoad(focus_depth, px, 0);
    let uv = frag / dims;
    let p = hist_u.inv_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    focus_out = vec4<f32>(select(max(-p.z / p.w, 1e-3), 1e9, abs(p.w) < 1e-9), 0.0, 0.0, 0.0);
}

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
    let base = curve.z * 256u;
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
