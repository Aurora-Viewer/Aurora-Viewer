// Depth of field. The circle of confusion follows Firestorm's
// (LLPipeline::renderDoF and deferred/cofF.glsl, LGPL 2.1, Copyright (C)
// Linden Research, Inc. and the Firestorm project): a thin lens of the
// camera's focal length and f-number focused at a distance, its blur disk
// measured in pixels of the image. The blur itself is Aurora's: at half
// resolution, the far field and the near field gathered apart (a gather
// that only takes samples whose own disk reaches the pixel, so a sharp
// subject never smears into the blurred background), the near field
// spread over what is behind it from a dilated tile map of its blur, then
// both laid over the sharp image at full resolution.

@group(0) @binding(0) var t_a: texture_2d<f32>;
@group(0) @binding(1) var t_depth: texture_depth_2d;
@group(0) @binding(2) var t_b: texture_2d<f32>;
@group(0) @binding(3) var t_c: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;

struct Dof {
    inv_proj: mat4x4<f32>,
    // coc scale (pixels for (s - z) / z), focus distance (m), largest
    // blur (full-resolution pixels), samples of the gather
    lens: vec4<f32>,
    // full width, height, half width, height
    sizes: vec4<f32>,
    // tile width, height, debug view, a cut-out photo (cutout.rs: the
    // alpha holds what shows through)
    tiles: vec4<f32>,
    // focus peaking: colour, w 1 when on
    peak: vec4<f32>,
    // x: the far end of the sharp zone (m), lens.y its near end
    band: vec4<f32>,
};
@group(0) @binding(5) var<uniform> dof: Dof;

const TILE: i32 = 16;
const GOLDEN_ANGLE: f32 = 2.39996323;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_dof(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

// Distance along the view axis of a depth value (reverse-Z, any projection).
fn view_depth(d: f32, uv: vec2<f32>) -> f32 {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    let p = dof.inv_proj * ndc;
    if (abs(p.w) < 1e-9) {
        return 1e9;
    }
    return max(-p.z / p.w, 1e-3);
}

// Signed circle of confusion in full-resolution pixels: positive in front of
// the focus plane (cofF.glsl's calc_cof: |s - z| / z * f^2 / (N (s - f)),
// divided by the magnification and by a pixel's size at the focus distance).
// That is k (1/z - 1/s), k = scale * s the same at any focus: with a sharp
// zone from s to band.x (group focus), s is the zone's nearer end for a
// point in front of it, its farther end for one behind, z itself inside.
fn coc(z: f32) -> f32 {
    let s = clamp(z, dof.lens.y, max(dof.band.x, dof.lens.y));
    let c = dof.lens.x * dof.lens.y * (s - z) / (z * s);
    return clamp(c, -dof.lens.z, dof.lens.z);
}

fn coc_at(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t_depth));
    let p = clamp(px, vec2<i32>(0), size - vec2<i32>(1));
    let d = textureLoad(t_depth, p, 0);
    if (dof.tiles.w > 0.5 && d <= 0.0) {
        // a cut-out photo's empty pixel: sharp, so that its black never
        // blurs into a subject behind the focus
        return 0.0;
    }
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size);
    return coc(view_depth(d, uv));
}

// Half resolution: the circle of the nearest of four pixels (in
// full-resolution pixels: an edge belongs to what is in front, whose
// antialiased colour it holds), and the colour of those on its side of the
// focus, the blurrier counting more: a sharp edge never tints the blur
// around it.
@fragment
fn fs_setup(in: Out) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.clip.xy) * 2;
    let size = vec2<i32>(textureDimensions(t_a));
    var cols: array<vec3<f32>, 4>;
    var cocs: array<f32, 4>;
    var c = 0.0;
    for (var i = 0; i < 4; i++) {
        let px = clamp(base + vec2<i32>(i & 1, i >> 1u), vec2<i32>(0), size - vec2<i32>(1));
        cols[i] = textureLoad(t_a, px, 0).rgb;
        cocs[i] = coc_at(px);
        if (i == 0 || cocs[i] > c) {
            c = cocs[i];
        }
    }
    var rgb = vec3<f32>(0.0);
    var w = 0.0;
    for (var i = 0; i < 4; i++) {
        let side = select(max(-cocs[i], 0.0), max(cocs[i], 0.0), c > 0.0);
        let wk = side + 1e-3;
        rgb += cols[i] * wk;
        w += wk;
    }
    return vec4<f32>(rgb / w, c);
}

// Tiles of TILE x TILE half pixels: the largest near blur in each.
@fragment
fn fs_tiles(in: Out) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.clip.xy) * TILE;
    let size = vec2<i32>(textureDimensions(t_a));
    var m = 0.0;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            let px = base + vec2<i32>(x, y);
            if (px.x < size.x && px.y < size.y) {
                m = max(m, textureLoad(t_a, px, 0).a);
            }
        }
    }
    return vec4<f32>(m, 0.0, 0.0, 1.0);
}

// The near blur of the tile and its neighbours: how far a blurred
// foreground reaches over what surrounds it.
@fragment
fn fs_dilate(in: Out) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let size = vec2<i32>(textureDimensions(t_a));
    var m = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(p + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
            m = max(m, textureLoad(t_a, q, 0).r);
        }
    }
    return vec4<f32>(m, 0.0, 0.0, 1.0);
}

fn noise(p: vec2<f32>) -> f32 {
    // interleaved gradient noise: the spiral turns from pixel to pixel
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct Fields {
    @location(0) far: vec4<f32>,
    @location(1) near: vec4<f32>,
};

// Half resolution: the far and near fields gathered over a disk (a Vogel
// spiral) as large as the pixel's own far blur or the near blur around it.
@fragment
fn fs_blur(in: Out) -> Fields {
    let half = dof.sizes.zw;
    let center = textureLoad(t_a, vec2<i32>(in.clip.xy), 0);
    let c0 = center.a * 0.5;
    let tile = textureSampleLevel(t_b, samp, in.uv, 0.0).r * 0.5;
    let radius = min(max(max(-c0, 0.0), tile), dof.lens.z * 0.5);
    var o: Fields;
    // alpha of the far field: how blurred it is here (the composite leaves
    // out what is sharp, so no sharp edge spreads with the upsampling)
    let far_a = smoothstep(0.5, 1.5, max(-c0, 0.0));
    if (radius < 0.5) {
        o.far = vec4<f32>(center.rgb, far_a);
        o.near = vec4<f32>(0.0);
        return o;
    }
    let n = i32(dof.lens.w);
    let turn = noise(in.clip.xy) * 6.2831853;
    var far = vec3<f32>(0.0);
    var far_w = 0.0;
    var near = vec3<f32>(0.0);
    var near_w = 0.0;
    var cover = 0.0;
    let area = radius * radius / f32(n);
    for (var i = 0; i < n; i++) {
        let r = radius * sqrt((f32(i) + 0.5) / f32(n));
        let a = f32(i) * GOLDEN_ANGLE + turn;
        let off = vec2<f32>(cos(a), sin(a)) * r;
        // whole texels: a filtered sample would blend a sharp edge's circle
        // with the background's and carry its colour into the blur
        let q = clamp(vec2<i32>(floor(in.clip.xy + off)), vec2<i32>(0), vec2<i32>(half) - vec2<i32>(1));
        let s = textureLoad(t_a, q, 0);
        let sc = s.a * 0.5;
        // a sample counts when its own disk reaches this pixel
        let wf = clamp(max(-sc, 0.0) - r + 1.0, 0.0, 1.0);
        far += s.rgb * wf;
        far_w += wf;
        let sn = max(sc, 0.0);
        let wn = clamp(sn - r + 1.0, 0.0, 1.0);
        // its light spread over its disk: the share that falls here (only
        // what is really out of focus covers what is behind it)
        let e = wn / max(sn * sn, 1.0) * smoothstep(1.0, 2.0, sn);
        near += s.rgb * e;
        near_w += e;
        cover += e * area;
    }
    o.far = vec4<f32>(select(center.rgb, far / far_w, far_w > 0.0), far_a);
    // premultiplied: the bilinear upsampling then never darkens its edge.
    // A blurred foreground covers itself up to its silhouette (what is
    // behind it is not known), decided here at half resolution so that the
    // upsampling smooths the (not antialiased) depth's staircase.
    // The covering follows the gathered coverage, not the silhouette.
    let alpha = max(clamp(cover, 0.0, 1.0), smoothstep(1.0, 2.0, c0 * 2.0) * smoothstep(0.25, 0.6, cover));
    o.near = vec4<f32>(select(vec3<f32>(0.0), near / near_w, near_w > 0.0) * alpha, alpha);
    return o;
}

// Focus peaking (the photo viewfinder's manual focus): where the image is
// sharp, its edges painted in the peaking colour, by the contrast of the
// sharp image around the pixel.
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn peaking(col: vec3<f32>, sharp: vec3<f32>, px: vec2<i32>, c: f32) -> vec3<f32> {
    if (dof.peak.w < 0.5 || abs(c) >= 1.0) {
        return col;
    }
    let last = vec2<i32>(textureDimensions(t_a)) - vec2<i32>(1);
    let l = luma(textureLoad(t_a, clamp(px - vec2<i32>(1, 0), vec2<i32>(0), last), 0).rgb);
    let r = luma(textureLoad(t_a, clamp(px + vec2<i32>(1, 0), vec2<i32>(0), last), 0).rgb);
    let u = luma(textureLoad(t_a, clamp(px - vec2<i32>(0, 1), vec2<i32>(0), last), 0).rgb);
    let d = luma(textureLoad(t_a, clamp(px + vec2<i32>(0, 1), vec2<i32>(0), last), 0).rgb);
    let g = length(vec2<f32>(r - l, d - u)) / (luma(sharp) + 0.05);
    let edge = smoothstep(0.3, 0.7, g) * (1.0 - smoothstep(0.5, 1.0, abs(c)));
    return mix(col, dof.peak.rgb * max(1.5, luma(col) * 1.5), edge);
}

// Full resolution: the sharp image, the far field where the pixel is
// behind the focus, then the near field over both.
@fragment
fn fs_composite(in: Out) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let sharp = textureLoad(t_a, px, 0);
    let c = coc_at(px);
    // the far field around: a bilinear filter that leaves out its sharp texels
    let half_size = vec2<i32>(textureDimensions(t_b));
    let hp = in.uv * vec2<f32>(half_size) - 0.5;
    let h0 = vec2<i32>(floor(hp));
    let fr = hp - floor(hp);
    var far = vec3<f32>(0.0);
    var far_w = 0.0;
    for (var i = 0; i < 4; i++) {
        let o = vec2<i32>(i & 1, i >> 1u);
        let q = clamp(h0 + o, vec2<i32>(0), half_size - vec2<i32>(1));
        let s = textureLoad(t_b, q, 0);
        let bw = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
        far += s.rgb * bw * s.a;
        far_w += bw * s.a;
    }
    // four bilinear taps around: the gather's grain smoothed out
    let texel = 1.0 / vec2<f32>(half_size);
    var near = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let o = vec2<f32>(select(-0.75, 0.75, (i & 1) == 1), select(-0.75, 0.75, (i >> 1u) == 1));
        near += 0.25 * textureSampleLevel(t_c, samp, in.uv + o * texel, 0.0);
    }
    var col = sharp.rgb;
    if (far_w > 1e-4) {
        col = mix(sharp.rgb, far / far_w, smoothstep(1.0, 3.0, -c));
    }
    // the near field over both, spread over its surroundings
    let near_rgb = near.rgb / max(near.a, 1e-4);
    col = mix(col, near_rgb, near.a);
    col = peaking(col, sharp.rgb, px, c);
    if (dof.tiles.z > 0.5) {
        // debug: near in red, far in blue, sharp in grey
        let k = clamp(abs(c) / max(dof.lens.z, 1.0), 0.0, 1.0);
        let g = dot(sharp.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        col = select(vec3<f32>(g * (1.0 - k), g * (1.0 - k), g * (1.0 - k) + k), vec3<f32>(g * (1.0 - k) + k, g * (1.0 - k), g * (1.0 - k)), c > 0.0);
    }
    // a cut-out photo: a blurred foreground covers the empty pixels around
    // it as much as it hides what would be behind them
    let alpha = select(sharp.a, sharp.a * (1.0 - near.a), dof.tiles.w > 0.5);
    return vec4<f32>(col, alpha);
}
