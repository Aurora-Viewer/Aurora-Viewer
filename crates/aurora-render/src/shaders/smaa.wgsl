// SMAA 1x (Jimenez et al., MIT licence, see src/smaa/LICENSE-SMAA.txt), a
// WGSL port of SMAA.hlsl as shipped in Firestorm (class1/deferred/SMAA.glsl,
// SMAAEdgeDetect / SMAABlendWeights / SMAANeighborhoodBlend), preset HIGH,
// color edge detection. wgpu textures have a top-left origin: this is the
// SMAA_FLIP_Y 0 (Direct3D) variant, sampling the lookup textures as stored.

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var edges_tex: texture_2d<f32>;
@group(0) @binding(2) var blend_tex: texture_2d<f32>;
@group(0) @binding(3) var area_tex: texture_2d<f32>;
@group(0) @binding(4) var search_tex: texture_2d<f32>;
@group(0) @binding(5) var lin: sampler;

struct SmaaParams {
    rt: vec4<f32>,      // SMAA_RT_METRICS: 1 / width, 1 / height, width, height
    output_srgb: f32,   // 1: the target encodes sRGB itself (write linear)
    _p0: f32,
    _p1: f32,
    _p2: f32,
};
@group(0) @binding(6) var<uniform> smaa: SmaaParams;

// SMAA_PRESET_HIGH
const THRESHOLD: f32 = 0.1;
const MAX_SEARCH_STEPS: f32 = 16.0;
const MAX_SEARCH_STEPS_DIAG: f32 = 8.0;
const CORNER_ROUNDING_NORM: f32 = 0.25;
const LOCAL_CONTRAST_ADAPTATION_FACTOR: f32 = 2.0;

const AREATEX_MAX_DISTANCE: f32 = 16.0;
const AREATEX_MAX_DISTANCE_DIAG: f32 = 20.0;
const AREATEX_PIXEL_SIZE: vec2<f32> = vec2<f32>(1.0 / 160.0, 1.0 / 560.0);
const AREATEX_SUBTEX_SIZE: f32 = 1.0 / 7.0;
const SEARCHTEX_SIZE: vec2<f32> = vec2<f32>(66.0, 33.0);
const SEARCHTEX_PACKED_SIZE: vec2<f32> = vec2<f32>(64.0, 16.0);

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_smaa(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

fn edges_at(c: vec2<f32>) -> vec2<f32> {
    return textureSampleLevel(edges_tex, lin, c, 0.0).rg;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// ------------------------------------------------------- 1: edge detection

fn color_at(c: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(color_tex, lin, c, 0.0).rgb;
}

fn max3(t: vec3<f32>) -> f32 {
    return max(max(t.r, t.g), t.b);
}

// SMAAColorEdgeDetectionPS (gamma-encoded colors).
@fragment
fn fs_smaa_edge(in: Out) -> @location(0) vec4<f32> {
    let tc = in.uv;
    let m = smaa.rt;
    let o0 = m.xyxy * vec4<f32>(-1.0, 0.0, 0.0, -1.0) + tc.xyxy;
    let o1 = m.xyxy * vec4<f32>(1.0, 0.0, 0.0, 1.0) + tc.xyxy;
    let o2 = m.xyxy * vec4<f32>(-2.0, 0.0, 0.0, -2.0) + tc.xyxy;
    let c = color_at(tc);
    var delta = vec4<f32>(0.0);
    delta.x = max3(abs(c - color_at(o0.xy)));
    delta.y = max3(abs(c - color_at(o0.zw)));
    var edges = step(vec2<f32>(THRESHOLD), delta.xy);
    if (dot(edges, vec2<f32>(1.0)) == 0.0) {
        discard;
    }
    delta.z = max3(abs(c - color_at(o1.xy)));
    delta.w = max3(abs(c - color_at(o1.zw)));
    var max_delta = max(delta.xy, delta.zw);
    delta.z = max3(abs(c - color_at(o2.xy)));
    delta.w = max3(abs(c - color_at(o2.zw)));
    max_delta = max(max_delta, delta.zw);
    let final_delta = max(max_delta.x, max_delta.y);
    // local contrast adaptation
    edges *= step(vec2<f32>(final_delta), LOCAL_CONTRAST_ADAPTATION_FACTOR * delta.xy);
    return vec4<f32>(edges, 0.0, 0.0);
}

// ------------------------------------------- 2: blending weight calculation

fn decode_diag2(e_in: vec2<f32>) -> vec2<f32> {
    var e = e_in;
    e.x = e.x * abs(5.0 * e.x - 5.0 * 0.75);
    return round(e);
}

fn decode_diag4(e_in: vec4<f32>) -> vec4<f32> {
    var e = e_in;
    e.x = e.x * abs(5.0 * e.x - 5.0 * 0.75);
    e.z = e.z * abs(5.0 * e.z - 5.0 * 0.75);
    return round(e);
}

// SMAASearchDiag1: xy = (steps, end found), zw = last edges.
fn search_diag1(tc: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    var coord = vec4<f32>(tc, -1.0, 1.0);
    let t = vec3<f32>(smaa.rt.xy, 1.0);
    var e = vec2<f32>(0.0);
    loop {
        if (!(coord.z < MAX_SEARCH_STEPS_DIAG - 1.0 && coord.w > 0.9)) {
            break;
        }
        let xyz = t * vec3<f32>(dir, 1.0) + coord.xyz;
        coord = vec4<f32>(xyz, coord.w);
        e = edges_at(coord.xy);
        coord.w = dot(e, vec2<f32>(0.5));
    }
    return vec4<f32>(coord.zw, e);
}

// SMAASearchDiag2 (both edges fetched at once with bilinear filtering).
fn search_diag2(tc: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    var coord = vec4<f32>(tc, -1.0, 1.0);
    coord.x += 0.25 * smaa.rt.x;
    let t = vec3<f32>(smaa.rt.xy, 1.0);
    var e = vec2<f32>(0.0);
    loop {
        if (!(coord.z < MAX_SEARCH_STEPS_DIAG - 1.0 && coord.w > 0.9)) {
            break;
        }
        let xyz = t * vec3<f32>(dir, 1.0) + coord.xyz;
        coord = vec4<f32>(xyz, coord.w);
        e = decode_diag2(edges_at(coord.xy));
        coord.w = dot(e, vec2<f32>(0.5));
    }
    return vec4<f32>(coord.zw, e);
}

fn area_diag(dist: vec2<f32>, e: vec2<f32>, offset: f32) -> vec2<f32> {
    var tc = vec2<f32>(AREATEX_MAX_DISTANCE_DIAG) * e + dist;
    tc = AREATEX_PIXEL_SIZE * tc + 0.5 * AREATEX_PIXEL_SIZE;
    // diagonal areas are on the second half of the texture
    tc.x += 0.5;
    tc.y += AREATEX_SUBTEX_SIZE * offset;
    return textureSampleLevel(area_tex, lin, tc, 0.0).rg;
}

// SMAAMovc(bool2(step(0.9, d)), cc, 0)
fn drop_unended(cc: vec2<f32>, d: vec2<f32>) -> vec2<f32> {
    return select(cc, vec2<f32>(0.0), d >= vec2<f32>(0.9));
}

fn calc_diag_weights(tc: vec2<f32>, e: vec2<f32>) -> vec2<f32> {
    var weights = vec2<f32>(0.0);
    var d = vec4<f32>(0.0);
    if (e.r > 0.0) {
        let r = search_diag1(tc, vec2<f32>(-1.0, 1.0));
        d.x = r.x + select(0.0, 1.0, r.w > 0.9);
        d.z = r.y;
    }
    let r2 = search_diag1(tc, vec2<f32>(1.0, -1.0));
    d.y = r2.x;
    d.w = r2.y;
    if (d.x + d.y > 2.0) {
        let coords = vec4<f32>(-d.x + 0.25, d.x, d.y, -d.y - 0.25) * smaa.rt.xyxy + tc.xyxy;
        let c0 = vec4<f32>(
            textureSampleLevel(edges_tex, lin, coords.xy, 0.0, vec2<i32>(-1, 0)).rg,
            textureSampleLevel(edges_tex, lin, coords.zw, 0.0, vec2<i32>(1, 0)).rg,
        );
        let dc = decode_diag4(c0);
        // c.yxwz = decode(c.xyzw)
        let c = vec4<f32>(dc.y, dc.x, dc.w, dc.z);
        let cc = drop_unended(vec2<f32>(2.0) * c.xz + c.yw, d.zw);
        weights += area_diag(d.xy, cc, 0.0);
    }
    let r3 = search_diag2(tc, vec2<f32>(-1.0, -1.0));
    d.x = r3.x;
    d.z = r3.y;
    if (textureSampleLevel(edges_tex, lin, tc, 0.0, vec2<i32>(1, 0)).r > 0.0) {
        let r4 = search_diag2(tc, vec2<f32>(1.0, 1.0));
        d.y = r4.x + select(0.0, 1.0, r4.w > 0.9);
        d.w = r4.y;
    } else {
        d.y = 0.0;
        d.w = 0.0;
    }
    if (d.x + d.y > 2.0) {
        let coords = vec4<f32>(-d.x, -d.x, d.y, d.y) * smaa.rt.xyxy + tc.xyxy;
        let cx = textureSampleLevel(edges_tex, lin, coords.xy, 0.0, vec2<i32>(-1, 0)).g;
        let cy = textureSampleLevel(edges_tex, lin, coords.xy, 0.0, vec2<i32>(0, -1)).r;
        let czw = textureSampleLevel(edges_tex, lin, coords.zw, 0.0, vec2<i32>(1, 0)).gr;
        let c = vec4<f32>(cx, cy, czw);
        let cc = drop_unended(vec2<f32>(2.0) * c.xz + c.yw, d.zw);
        weights += area_diag(d.xy, cc, 0.0).yx;
    }
    return weights;
}

fn search_length(e: vec2<f32>, offset: f32) -> f32 {
    var scale = SEARCHTEX_SIZE * vec2<f32>(0.5, -1.0);
    var bias = SEARCHTEX_SIZE * vec2<f32>(offset, 1.0);
    scale += vec2<f32>(-1.0, 1.0);
    bias += vec2<f32>(0.5, -0.5);
    scale *= 1.0 / SEARCHTEX_PACKED_SIZE;
    bias *= 1.0 / SEARCHTEX_PACKED_SIZE;
    return textureSampleLevel(search_tex, lin, scale * e + bias, 0.0).r;
}

fn search_x_left(tc_in: vec2<f32>, end: f32) -> f32 {
    var tc = tc_in;
    var e = vec2<f32>(0.0, 1.0);
    loop {
        if (!(tc.x > end && e.g > 0.8281 && e.r == 0.0)) {
            break;
        }
        e = edges_at(tc);
        tc = -vec2<f32>(2.0, 0.0) * smaa.rt.xy + tc;
    }
    let offset = -(255.0 / 127.0) * search_length(e, 0.0) + 3.25;
    return smaa.rt.x * offset + tc.x;
}

fn search_x_right(tc_in: vec2<f32>, end: f32) -> f32 {
    var tc = tc_in;
    var e = vec2<f32>(0.0, 1.0);
    loop {
        if (!(tc.x < end && e.g > 0.8281 && e.r == 0.0)) {
            break;
        }
        e = edges_at(tc);
        tc = vec2<f32>(2.0, 0.0) * smaa.rt.xy + tc;
    }
    let offset = -(255.0 / 127.0) * search_length(e, 0.5) + 3.25;
    return -smaa.rt.x * offset + tc.x;
}

fn search_y_up(tc_in: vec2<f32>, end: f32) -> f32 {
    var tc = tc_in;
    var e = vec2<f32>(1.0, 0.0);
    loop {
        if (!(tc.y > end && e.r > 0.8281 && e.g == 0.0)) {
            break;
        }
        e = edges_at(tc);
        tc = -vec2<f32>(0.0, 2.0) * smaa.rt.xy + tc;
    }
    let offset = -(255.0 / 127.0) * search_length(e.yx, 0.0) + 3.25;
    return smaa.rt.y * offset + tc.y;
}

fn search_y_down(tc_in: vec2<f32>, end: f32) -> f32 {
    var tc = tc_in;
    var e = vec2<f32>(1.0, 0.0);
    loop {
        if (!(tc.y < end && e.r > 0.8281 && e.g == 0.0)) {
            break;
        }
        e = edges_at(tc);
        tc = vec2<f32>(0.0, 2.0) * smaa.rt.xy + tc;
    }
    let offset = -(255.0 / 127.0) * search_length(e.yx, 0.5) + 3.25;
    return -smaa.rt.y * offset + tc.y;
}

fn area(dist: vec2<f32>, e1: f32, e2: f32, offset: f32) -> vec2<f32> {
    // rounding prevents precision errors of bilinear filtering
    var tc = vec2<f32>(AREATEX_MAX_DISTANCE) * round(4.0 * vec2<f32>(e1, e2)) + dist;
    tc = AREATEX_PIXEL_SIZE * tc + 0.5 * AREATEX_PIXEL_SIZE;
    tc.y = AREATEX_SUBTEX_SIZE * offset + tc.y;
    return textureSampleLevel(area_tex, lin, tc, 0.0).rg;
}

fn corner_rounding(d: vec2<f32>) -> vec2<f32> {
    let left_right = step(d.xy, d.yx);
    // less blending for pixels in the center of a line
    return (1.0 - CORNER_ROUNDING_NORM) * left_right / (left_right.x + left_right.y);
}

fn horizontal_corners(weights: vec2<f32>, tc: vec4<f32>, d: vec2<f32>) -> vec2<f32> {
    let rounding = corner_rounding(d);
    var factor = vec2<f32>(1.0);
    factor.x -= rounding.x * textureSampleLevel(edges_tex, lin, tc.xy, 0.0, vec2<i32>(0, 1)).r;
    factor.x -= rounding.y * textureSampleLevel(edges_tex, lin, tc.zw, 0.0, vec2<i32>(1, 1)).r;
    factor.y -= rounding.x * textureSampleLevel(edges_tex, lin, tc.xy, 0.0, vec2<i32>(0, -2)).r;
    factor.y -= rounding.y * textureSampleLevel(edges_tex, lin, tc.zw, 0.0, vec2<i32>(1, -2)).r;
    return weights * clamp(factor, vec2<f32>(0.0), vec2<f32>(1.0));
}

fn vertical_corners(weights: vec2<f32>, tc: vec4<f32>, d: vec2<f32>) -> vec2<f32> {
    let rounding = corner_rounding(d);
    var factor = vec2<f32>(1.0);
    factor.x -= rounding.x * textureSampleLevel(edges_tex, lin, tc.xy, 0.0, vec2<i32>(1, 0)).g;
    factor.x -= rounding.y * textureSampleLevel(edges_tex, lin, tc.zw, 0.0, vec2<i32>(1, 1)).g;
    factor.y -= rounding.x * textureSampleLevel(edges_tex, lin, tc.xy, 0.0, vec2<i32>(-2, 0)).g;
    factor.y -= rounding.y * textureSampleLevel(edges_tex, lin, tc.zw, 0.0, vec2<i32>(-2, 1)).g;
    return weights * clamp(factor, vec2<f32>(0.0), vec2<f32>(1.0));
}

// SMAABlendingWeightCalculationPS (SMAA 1x: subsample indices 0).
@fragment
fn fs_smaa_weights(in: Out) -> @location(0) vec4<f32> {
    let tc = in.uv;
    let rt = smaa.rt;
    let pix = tc * rt.zw;
    // @PSEUDO_GATHER4 offsets and the ends of the searches
    let o0 = rt.xyxy * vec4<f32>(-0.25, -0.125, 1.25, -0.125) + tc.xyxy;
    let o1 = rt.xyxy * vec4<f32>(-0.125, -0.25, -0.125, 1.25) + tc.xyxy;
    let o2 = rt.xxyy * vec4<f32>(-2.0, 2.0, -2.0, 2.0) * MAX_SEARCH_STEPS + vec4<f32>(o0.xz, o1.yw);
    var weights = vec4<f32>(0.0);
    var e = textureSampleLevel(edges_tex, lin, tc, 0.0).rg;
    if (e.g > 0.0) {
        // edge at north: diagonals first (they have north and west edges)
        let dw = calc_diag_weights(tc, e);
        weights = vec4<f32>(dw, weights.zw);
        if (weights.r == -weights.g) {
            var coords = vec3<f32>(0.0);
            coords.x = search_x_left(o0.xy, o2.x);
            coords.y = o1.y;
            var d = vec2<f32>(coords.x, 0.0);
            let e1 = textureSampleLevel(edges_tex, lin, coords.xy, 0.0).r;
            coords.z = search_x_right(o0.zw, o2.y);
            d.y = coords.z;
            d = abs(round(rt.zz * d - pix.xx));
            let sqrt_d = sqrt(d);
            let e2 = textureSampleLevel(edges_tex, lin, coords.zy, 0.0, vec2<i32>(1, 0)).r;
            var w = area(sqrt_d, e1, e2, 0.0);
            coords.y = tc.y;
            w = horizontal_corners(w, coords.xyzy, d);
            weights = vec4<f32>(w, weights.zw);
        } else {
            // skip vertical processing
            e.r = 0.0;
        }
    }
    if (e.r > 0.0) {
        // edge at west
        var coords = vec3<f32>(0.0);
        coords.y = search_y_up(o1.xy, o2.z);
        coords.x = o0.x;
        var d = vec2<f32>(coords.y, 0.0);
        let e1 = textureSampleLevel(edges_tex, lin, coords.xy, 0.0).g;
        coords.z = search_y_down(o1.zw, o2.w);
        d.y = coords.z;
        d = abs(round(rt.ww * d - pix.yy));
        let sqrt_d = sqrt(d);
        let e2 = textureSampleLevel(edges_tex, lin, coords.xz, 0.0, vec2<i32>(0, 1)).g;
        var w = area(sqrt_d, e1, e2, 0.0);
        coords.x = tc.x;
        w = vertical_corners(w, coords.xyxz, d);
        weights = vec4<f32>(weights.xy, w);
    }
    return weights;
}

// ---------------------------------------------- 3: neighborhood blending

fn lin_color(c: vec2<f32>) -> vec3<f32> {
    return srgb_to_linear(textureSampleLevel(color_tex, lin, c, 0.0).rgb);
}

// SMAANeighborhoodBlendingPS (Firestorm mixes in linear space).
@fragment
fn fs_smaa_blend(in: Out) -> @location(0) vec4<f32> {
    let tc = in.uv;
    let rt = smaa.rt;
    let off = rt.xyxy * vec4<f32>(1.0, 0.0, 0.0, 1.0) + tc.xyxy;
    let here = textureSampleLevel(blend_tex, lin, tc, 0.0);
    // right, top (bottom of the pixel above in top-left space), bottom, left
    let a = vec4<f32>(
        textureSampleLevel(blend_tex, lin, off.xy, 0.0).a,
        textureSampleLevel(blend_tex, lin, off.zw, 0.0).g,
        here.z,
        here.x,
    );
    var col: vec3<f32>;
    if (dot(a, vec4<f32>(1.0)) < 1e-5) {
        col = lin_color(tc);
    } else {
        let h = max(a.x, a.z) > max(a.y, a.w);
        var offset = vec4<f32>(0.0, a.y, 0.0, a.w);
        var weight = a.yw;
        if (h) {
            offset = vec4<f32>(a.x, 0.0, a.z, 0.0);
            weight = a.xz;
        }
        weight /= dot(weight, vec2<f32>(1.0));
        let coord = offset * vec4<f32>(rt.xy, -rt.xy) + tc.xyxy;
        col = weight.x * lin_color(coord.xy) + weight.y * lin_color(coord.zw);
    }
    if (smaa.output_srgb > 0.5) {
        return vec4<f32>(col, 1.0);
    }
    return vec4<f32>(linear_to_srgb(col), 1.0);
}
