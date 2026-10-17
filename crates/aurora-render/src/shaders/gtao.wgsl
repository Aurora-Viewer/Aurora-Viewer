// Ground-truth ambient occlusion (GTAO), the alternative to the Legacy
// Viewer's screen-space occlusion (ssao.wgsl) chosen in Preferences ›
// Graphics. Port of Intel's XeGTAO (XeGTAO.hlsli, vaGTAO.hlsl,
// https://github.com/GameTechDev/XeGTAO; see NOTICE.md), itself after
// Jimenez et al., "Practical Real-Time Strategies for Accurate Indirect
// Occlusion" (2016). Its notice:
//
//   Copyright (C) 2016-2021, Intel Corporation
//
//   Permission is hereby granted, free of charge, to any person obtaining a
//   copy of this software and associated documentation files (the
//   "Software"), to deal in the Software without restriction, including
//   without limitation the rights to use, copy, modify, merge, publish,
//   distribute, sublicense, and/or sell copies of the Software, and to
//   permit persons to whom the Software is furnished to do so, subject to
//   the following conditions:
//
//   The above copyright notice and this permission notice shall be included
//   in all copies or substantial portions of the Software.
//
//   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//   OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//   MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
//   IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
//   CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
//   TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
//   SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
//
// Four compute steps, all from the single-sample depth prepass:
//
// 1. `prefilter_depth0`: the view depth (metres in front of the camera) at
//    the resolution the occlusion is computed at (the window's, or half);
// 2. `prefilter_depth`: four more levels, each a weighted mean of the 2 x 2
//    below that keeps the nearest surfaces (XeGTAO_DepthMIPFilter); far
//    samples read a coarse level;
// 3. `main`: per pixel, a few slices of the hemisphere around the view
//    vector; along each, the highest horizon on both sides gives the
//    cosine-weighted visible arc (the paper's lines 5 to 27), averaged over
//    the slices; plus the depth edges with the four neighbours;
// 4. `denoise` (then `denoise_final`): XeGTAO's edge-aware 3 x 3 blur, which
//    replaces its temporal accumulation (we have none): the noise is a
//    Hilbert curve driving the R2 sequence, the same for every frame, so the
//    result does not flicker and the blur removes its pattern.
//
// The result is a visibility (1 = nothing occludes) in `.r` of the last
// target, sampled where the Legacy occlusion is (screen_ao, common.wgsl),
// and the bent normal — the mean direction of the unoccluded part of the
// hemisphere (XE_GTAO_COMPUTE_BENT_NORMALS, the paper's algorithm 2) — in
// view space, `.gba` mapped from -1..1 to 0..1; the shading turns it into a
// specular occlusion (specular_occlusion, common.wgsl). The working
// targets hold the visibility in `.r` and the bent normal unpacked in
// `.gba`, blurred with the same weights.
// View space here is XeGTAO's: x right, y up, z the distance in front of
// the camera; screen pixels go down.

struct GtaoParams {
    // the projection: P00, P11 (scales), P20, P21 (off-centre terms of a
    // photo tile)
    proj: vec4<f32>,
    // P32 (near: the depth buffer holds near / view z), P22, the size of
    // the occlusion targets in pixels
    depth: vec4<f32>,
    // effect radius (metres), falloff range (fraction of the radius),
    // sample distribution power, thin occluder compensation
    radius: vec4<f32>,
    // final value power, depth MIP sampling offset, slices, steps per slice
    quality: vec4<f32>,
    // denoise blur beta, full-resolution pixels per occlusion pixel (1 or
    // 2), unused, unused
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: GtaoParams;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var src_mip: texture_2d<f32>;
@group(0) @binding(3) var dst_mip: texture_storage_2d<r32float, write>;
@group(0) @binding(4) var depth_mips: texture_2d<f32>;
@group(0) @binding(5) var ao_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var edges_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(7) var ao_in: texture_2d<f32>;
@group(0) @binding(8) var edges_in: texture_2d<f32>;
@group(0) @binding(9) var final_out: texture_storage_2d<rgba8unorm, write>;

const PI: f32 = 3.1415926535897932;
const PI_HALF: f32 = 1.5707963267948966;
// XE_GTAO_DEPTH_MIP_LEVELS
const MIP_LEVELS: f32 = 5.0;
// XE_GTAO_OCCLUSION_TERM_SCALE: the main pass stores visibility / 1.5 so
// that the blur has headroom; the last pass scales it back
const OCCLUSION_TERM_SCALE: f32 = 1.5;
// the sky (depth 0) and anything this far count as no surface
const FAR: f32 = 100000.0;

fn target_size() -> vec2<f32> {
    return g.depth.zw;
}

// View depth of a depth buffer value (reversed, infinite far plane).
fn view_depth(d: f32) -> f32 {
    if (d <= 0.0) {
        return FAR;
    }
    return min(g.depth.x / (d + g.depth.y), FAR);
}

// XeGTAO_ComputeViewspacePosition: the point at `uv` (0..1, y down) and
// view depth `z`, through the projection (a photo tile's included).
fn view_position(uv: vec2<f32>, z: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return vec3<f32>((ndc + g.proj.zw) / g.proj.xy * z, z);
}

// ---- 1, 2: depth levels

@compute @workgroup_size(8, 8)
fn prefilter_depth0(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<u32>(target_size());
    if (any(id.xy >= size)) {
        return;
    }
    // at half resolution, the top left sample of each 2 x 2 (as the Legacy
    // pass reads its pixels)
    let full = vec2<i32>(textureDimensions(depth_tex));
    let src = min(vec2<i32>(id.xy) * i32(g.misc.y), full - vec2<i32>(1));
    let z = view_depth(textureLoad(depth_tex, src, 0));
    textureStore(dst_mip, vec2<i32>(id.xy), vec4<f32>(z, 0.0, 0.0, 0.0));
}

// XeGTAO_DepthMIPFilter: the four depths weighted by how far they are
// behind the nearest, so a coarse level keeps the occluders.
fn depth_mip_filter(d0: f32, d1: f32, d2: f32, d3: f32) -> f32 {
    let max_depth = max(max(d0, d1), max(d2, d3));
    // found empirically by XeGTAO
    let depth_range_scale = 0.75;
    let effect_radius = depth_range_scale * g.radius.x;
    let falloff_range = g.radius.y * effect_radius;
    let falloff_from = effect_radius * (1.0 - g.radius.y);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;
    let w0 = saturate((max_depth - d0) * falloff_mul + falloff_add);
    let w1 = saturate((max_depth - d1) * falloff_mul + falloff_add);
    let w2 = saturate((max_depth - d2) * falloff_mul + falloff_add);
    let w3 = saturate((max_depth - d3) * falloff_mul + falloff_add);
    let sum = w0 + w1 + w2 + w3;
    return (w0 * d0 + w1 * d1 + w2 * d2 + w3 * d3) / max(sum, 1e-6);
}

@compute @workgroup_size(8, 8)
fn prefilter_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dst_mip);
    if (any(id.xy >= size)) {
        return;
    }
    let src = vec2<i32>(textureDimensions(src_mip));
    let base = vec2<i32>(id.xy) * 2;
    let d0 = textureLoad(src_mip, min(base, src - vec2<i32>(1)), 0).x;
    let d1 = textureLoad(src_mip, min(base + vec2<i32>(1, 0), src - vec2<i32>(1)), 0).x;
    let d2 = textureLoad(src_mip, min(base + vec2<i32>(0, 1), src - vec2<i32>(1)), 0).x;
    let d3 = textureLoad(src_mip, min(base + vec2<i32>(1, 1), src - vec2<i32>(1)), 0).x;
    textureStore(dst_mip, vec2<i32>(id.xy), vec4<f32>(depth_mip_filter(d0, d1, d2, d3), 0.0, 0.0, 0.0));
}

// ---- 3: the occlusion

// The view depth at `uv` from the level a sample `mip` levels coarse reads
// (XeGTAO samples the level with a point sampler: the nearest level, the
// texel under the point).
fn depth_at(uv: vec2<f32>, mip: f32) -> f32 {
    let lvl = i32(clamp(floor(mip + 0.5), 0.0, MIP_LEVELS - 1.0));
    let size = vec2<i32>(textureDimensions(depth_mips, lvl));
    let c = clamp(vec2<i32>(floor(uv * vec2<f32>(size))), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(depth_mips, c, lvl).x;
}

fn depth0(px: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_mips, 0));
    return textureLoad(depth_mips, clamp(px, vec2<i32>(0), size - vec2<i32>(1)), 0).x;
}

// XE_HILBERT_LEVEL 6: the index of a pixel along a Hilbert curve over a
// 64 x 64 tile (HilbertIndex).
const HILBERT_WIDTH: u32 = 64u;

fn hilbert_index(px: u32, py: u32) -> u32 {
    var x = px;
    var y = py;
    var index = 0u;
    var level = HILBERT_WIDTH / 2u;
    loop {
        if (level == 0u) {
            break;
        }
        let rx = select(0u, 1u, (x & level) > 0u);
        let ry = select(0u, 1u, (y & level) > 0u);
        index += level * level * ((3u * rx) ^ ry);
        if (ry == 0u) {
            if (rx == 1u) {
                x = HILBERT_WIDTH - 1u - x;
                y = HILBERT_WIDTH - 1u - y;
            }
            let t = x;
            x = y;
            y = t;
        }
        level = level / 2u;
    }
    return index;
}

// SpatioTemporalNoise with its temporal index at 0: the Hilbert index
// drives the R2 sequence, so neighbouring pixels get well spread slice and
// sample offsets (a low-discrepancy pattern the blur takes out).
fn pixel_noise(px: vec2<u32>) -> vec2<f32> {
    let index = hilbert_index(px.x % HILBERT_WIDTH, px.y % HILBERT_WIDTH);
    return fract(vec2<f32>(0.5) + f32(index) * vec2<f32>(0.75487766624669276005, 0.5698402909980532659114));
}

// XeGTAO_CalculateEdges: how much each neighbour (left, right, top,
// bottom) lies on the same surface, 1 for yes; a neighbour on the slope the
// opposite one continues counts as the same surface.
fn calculate_edges(c: f32, l: f32, r: f32, t: f32, b: f32) -> vec4<f32> {
    var e = vec4<f32>(l, r, t, b) - vec4<f32>(c);
    let slope_lr = (e.y - e.x) * 0.5;
    let slope_tb = (e.w - e.z) * 0.5;
    let adjusted = e + vec4<f32>(slope_lr, -slope_lr, slope_tb, -slope_tb);
    e = min(abs(e), abs(adjusted));
    return saturate(vec4<f32>(1.25) - e / (c * 0.011));
}

// XeGTAO_CalculateNormal: the normal from the four neighbours, each pair
// counted as much as both are on the same surface.
fn calculate_normal(edges: vec4<f32>, c: vec3<f32>, l: vec3<f32>, r: vec3<f32>, t: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let accepted = saturate(
        vec4<f32>(edges.x * edges.z, edges.z * edges.y, edges.y * edges.w, edges.w * edges.x) + vec4<f32>(0.01)
    );
    let dl = normalize(l - c);
    let dr = normalize(r - c);
    let dt = normalize(t - c);
    let db = normalize(b - c);
    let n = accepted.x * cross(dl, dt) + accepted.y * cross(dt, dr) + accepted.z * cross(dr, db)
        + accepted.w * cross(db, dl);
    return normalize(n);
}

// XeGTAO_FastACos
fn fast_acos(x_in: f32) -> f32 {
    let x = abs(x_in);
    var res = -0.156583 * x + PI_HALF;
    res *= sqrt(max(1.0 - x, 0.0));
    return select(PI - res, res, x_in >= 0.0);
}

// XeGTAO_RotFromToMatrix applied to `v`, for its one use: `src` is
// (0, 0, -1), the slice frame's direction towards the camera, `dst` the
// view vector of the pixel.
fn rot_from_to(src: vec3<f32>, dst: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    let e = dot(src, dst);
    if (abs(e) > 1.0 - 0.0003) {
        return v;
    }
    let c = cross(src, dst);
    let h = 1.0 / (1.0 + e);
    let hvx = h * c.x;
    let hvz = h * c.z;
    let hvxy = hvx * c.y;
    let hvxz = hvx * c.z;
    let hvyz = hvz * c.y;
    // the rows of XeGTAO's matrix
    let r0 = vec3<f32>(e + hvx * c.x, hvxy - c.z, hvxz + c.y);
    let r1 = vec3<f32>(hvxy + c.z, e + h * c.y * c.y, hvyz - c.x);
    let r2 = vec3<f32>(hvxz - c.y, hvyz + c.x, e + hvz * c.z);
    return vec3<f32>(dot(r0, v), dot(r1, v), dot(r2, v));
}

// The view's bent normal into the working target: (visibility / scale,
// bent normal).
fn working(visibility: f32, bent: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(saturate(visibility / OCCLUSION_TERM_SCALE), bent);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = target_size();
    if (f32(id.x) >= size.x || f32(id.y) >= size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let pixel = vec2<f32>(1.0) / size;
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) * pixel;

    let zc = depth0(px);
    let zl = depth0(px + vec2<i32>(-1, 0));
    let zr = depth0(px + vec2<i32>(1, 0));
    let zt = depth0(px + vec2<i32>(0, -1));
    let zb = depth0(px + vec2<i32>(0, 1));
    let edges = calculate_edges(zc, zl, zr, zt, zb);
    textureStore(edges_out, px, edges);
    if (zc >= FAR * 0.5) {
        textureStore(ao_out, px, working(1.0, vec3<f32>(0.0, 0.0, -1.0)));
        return;
    }

    // the centre moved slightly towards the camera against the depth
    // buffer's imprecision (XeGTAO's value for 32-bit depths)
    let z = zc * 0.99999;
    let centre = view_position(uv, z);
    var n = calculate_normal(
        edges,
        centre,
        view_position(uv + vec2<f32>(-pixel.x, 0.0), zl),
        view_position(uv + vec2<f32>(pixel.x, 0.0), zr),
        view_position(uv + vec2<f32>(0.0, -pixel.y), zt),
        view_position(uv + vec2<f32>(0.0, pixel.y), zb),
    );
    let view_vec = normalize(-centre);
    if (dot(n, view_vec) < 0.0) {
        n = -n;
    }

    let effect_radius = g.radius.x;
    let sample_distribution_power = g.radius.z;
    let thin_occluder_compensation = g.radius.w;
    let falloff_range = g.radius.y * effect_radius;
    let falloff_from = effect_radius * (1.0 - g.radius.y);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;
    let slice_count = g.quality.z;
    let steps_per_slice = g.quality.w;

    let noise = pixel_noise(id.xy);
    // view-space size of a pixel at the centre's depth, and the radius in
    // pixels
    let pixel_view_size = z * 2.0 / (g.proj.x * size.x);
    let screen_radius = effect_radius / pixel_view_size;
    // fade out where the radius covers only a few pixels
    var visibility = saturate((10.0 - screen_radius) / 100.0) * 0.5;
    // the first sample stays clear of the centre pixel
    let min_s = 1.3 / screen_radius;
    var bent = vec3<f32>(0.0);

    for (var si = 0; si < i32(slice_count); si++) {
        let slice = f32(si);
        let slice_k = (slice + noise.x) / slice_count;
        // lines 5, 6 of the paper
        let phi = slice_k * PI;
        let cos_phi = cos(phi);
        let sin_phi = sin(phi);
        // in pixels (screen y goes down)
        let omega = vec2<f32>(cos_phi, -sin_phi) * screen_radius;
        // lines 8 to 10
        let direction = vec3<f32>(cos_phi, sin_phi, 0.0);
        let ortho_direction = direction - dot(direction, view_vec) * view_vec;
        let axis = normalize(cross(ortho_direction, view_vec));
        let projected_normal = n - axis * dot(n, axis);
        // lines 13 to 15
        let sign_norm = sign(dot(ortho_direction, projected_normal));
        var projected_length = length(projected_normal);
        let cos_norm = saturate(dot(projected_normal, view_vec) / max(projected_length, 1e-6));
        let nn = sign_norm * fast_acos(cos_norm);
        // the lowest horizons: the tangent plane on each side
        let low_horizon0 = cos(nn + PI_HALF);
        let low_horizon1 = cos(nn - PI_HALF);
        var horizon0 = low_horizon0;
        var horizon1 = low_horizon1;

        for (var st = 0; st < i32(steps_per_slice); st++) {
            // R1 sequence over the samples
            let step_base_noise = f32(si + st * i32(steps_per_slice)) * 0.6180339887498948482;
            let step_noise = fract(noise.y + step_base_noise);
            // line 20, with the noise, a denser distribution near the
            // centre, and away from the centre pixel
            var s = (f32(st) + step_noise) / steps_per_slice;
            s = pow(s, sample_distribution_power);
            s += min_s;
            var offset = s * omega;
            let offset_length = length(offset);
            let mip = clamp(log2(offset_length) - g.quality.y, 0.0, MIP_LEVELS);
            // snapped to pixel centres (the direction follows the texel)
            offset = round(offset) * pixel;

            let uv0 = uv + offset;
            let p0 = view_position(uv0, depth_at(uv0, mip));
            let uv1 = uv - offset;
            let p1 = view_position(uv1, depth_at(uv1, mip));
            let delta0 = p0 - centre;
            let delta1 = p1 - centre;
            let dist0 = length(delta0);
            let dist1 = length(delta1);
            // lines 23, 24
            let horizon_vec0 = delta0 / max(dist0, 1e-6);
            let horizon_vec1 = delta1 / max(dist1, 1e-6);
            // samples beyond the radius fade out; the thin occluder
            // heuristic lets a sample behind the centre go sooner
            let falloff_base0 = length(vec3<f32>(delta0.xy, delta0.z * (1.0 + thin_occluder_compensation)));
            let falloff_base1 = length(vec3<f32>(delta1.xy, delta1.z * (1.0 + thin_occluder_compensation)));
            let weight0 = saturate(falloff_base0 * falloff_mul + falloff_add);
            let weight1 = saturate(falloff_base1 * falloff_mul + falloff_add);
            var shc0 = dot(horizon_vec0, view_vec);
            var shc1 = dot(horizon_vec1, view_vec);
            shc0 = mix(low_horizon0, shc0, weight0);
            shc1 = mix(low_horizon1, shc1, weight1);
            horizon0 = max(horizon0, shc0);
            horizon1 = max(horizon1, shc1);
        }

        // the fudge XeGTAO adds against a slight over-darkening on steep
        // slopes
        projected_length = mix(projected_length, 1.0, 0.05);
        // line 27: the visible arc between the two horizons, cosine weighted
        let h0 = -fast_acos(horizon1);
        let h1 = fast_acos(horizon0);
        let iarc0 = (cos_norm + 2.0 * h0 * sin(nn) - cos(2.0 * h0 - nn)) / 4.0;
        let iarc1 = (cos_norm + 2.0 * h1 * sin(nn) - cos(2.0 * h1 - nn)) / 4.0;
        visibility += projected_length * (iarc0 + iarc1);
        // the paper's algorithm 2: the slice's bent normal, in a frame whose
        // -z looks at the camera, turned to the view vector
        let t0 = (6.0 * sin(h0 - nn) - sin(3.0 * h0 - nn) + 6.0 * sin(h1 - nn) - sin(3.0 * h1 - nn) + 16.0 * sin(nn)
            - 3.0 * (sin(h0 + nn) + sin(h1 + nn))) / 12.0;
        let t1 = (-cos(3.0 * h0 - nn) - cos(3.0 * h1 - nn) + 8.0 * cos(nn) - 3.0 * (cos(h0 + nn) + cos(h1 + nn))) / 12.0;
        let local_bent = vec3<f32>(direction.x * t0, direction.y * t0, -t1);
        bent += rot_from_to(vec3<f32>(0.0, 0.0, -1.0), view_vec, local_bent) * projected_length;
    }
    visibility /= slice_count;
    visibility = pow(max(visibility, 0.0), g.quality.x);
    // a visible pixel is never fully occluded
    visibility = max(0.03, visibility);
    // (no slice: the surface's own normal)
    let bent_n = select(n, normalize(bent), dot(bent, bent) > 1e-8);
    textureStore(ao_out, px, working(visibility, bent_n));
}

// ---- 4: the blur

fn edges_at(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(edges_in));
    return textureLoad(edges_in, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

fn ao_at(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(ao_in));
    return textureLoad(ao_in, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// XeGTAO_Denoise for one pixel: its own value and its eight neighbours,
// each weighted by the edges between them; the last pass weights the
// centre more (DenoiseBlurBeta) so the image stays sharp. The visibility
// and the bent normal take the same weights.
fn denoised(px: vec2<i32>, final_apply: bool) -> vec4<f32> {
    let beta = g.misc.x;
    let blur_amount = select(beta / 5.0, beta, final_apply);
    let diag = 0.85 * 0.5;
    let el = edges_at(px + vec2<i32>(-1, 0));
    let er = edges_at(px + vec2<i32>(1, 0));
    let et = edges_at(px + vec2<i32>(0, -1));
    let eb = edges_at(px + vec2<i32>(0, 1));
    // the edges are not symmetric (slope adjusted): make them so
    var ec = edges_at(px) * vec4<f32>(el.y, er.x, et.w, eb.z);
    // a little leak where three or four edges close the pixel in
    let leak_threshold = 2.5;
    let leak_strength = 0.5;
    let edginess = (saturate(4.0 - leak_threshold - dot(ec, vec4<f32>(1.0))) / (4.0 - leak_threshold)) * leak_strength;
    ec = saturate(ec + vec4<f32>(edginess));
    let w_tl = diag * (ec.x * el.z + ec.z * et.x);
    let w_tr = diag * (ec.z * et.y + ec.y * er.z);
    let w_bl = diag * (ec.w * eb.x + ec.x * el.w);
    let w_br = diag * (ec.y * er.w + ec.w * eb.y);
    var sum_w = blur_amount;
    var sum = ao_at(px) * sum_w;
    sum += ao_at(px + vec2<i32>(-1, 0)) * ec.x;
    sum += ao_at(px + vec2<i32>(1, 0)) * ec.y;
    sum += ao_at(px + vec2<i32>(0, -1)) * ec.z;
    sum += ao_at(px + vec2<i32>(0, 1)) * ec.w;
    sum += ao_at(px + vec2<i32>(-1, -1)) * w_tl;
    sum += ao_at(px + vec2<i32>(1, -1)) * w_tr;
    sum += ao_at(px + vec2<i32>(-1, 1)) * w_bl;
    sum += ao_at(px + vec2<i32>(1, 1)) * w_br;
    sum_w += ec.x + ec.y + ec.z + ec.w + w_tl + w_tr + w_bl + w_br;
    return sum / sum_w;
}

@compute @workgroup_size(8, 8)
fn denoise(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(ao_out))) {
        return;
    }
    let px = vec2<i32>(id.xy);
    textureStore(ao_out, px, denoised(px, false));
}

@compute @workgroup_size(8, 8)
fn denoise_final(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(final_out))) {
        return;
    }
    let px = vec2<i32>(id.xy);
    let d = denoised(px, true);
    let v = saturate(d.r * OCCLUSION_TERM_SCALE);
    let b = select(vec3<f32>(0.0, 0.0, -1.0), normalize(d.gba), dot(d.gba, d.gba) > 1e-8);
    textureStore(final_out, px, vec4<f32>(v, b * 0.5 + vec3<f32>(0.5)));
}
