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
//    the slices; plus the depth edges with the four neighbours; the surface
//    normal is the mesh's, drawn by the depth prepass (`surface_normal`);
//    With the screen-space indirect light on, `main_bitmask` runs after it:
//    one bounce of the previous frame's lit colour gathered on a visibility
//    bitmask (Therrien et al. 2023), into `il_out` alone, blurred by
//    `denoise_il` (then `denoise_il_final`) with `main`'s edges; the
//    occlusion stays `main`'s, the same with the light on or off;
//    `temporal` then accumulates it over the frames, as XeGTAO expects of
//    the TAA it runs under: the noise (a Hilbert curve driving the R2
//    sequence) moves every frame (SpatioTemporalNoise), and each pixel blends
//    with the history where its point was in the previous frame, refused
//    where the depth there does not match (a surface uncovered) and clipped
//    to its 3 x 3 neighbourhood (moving avatars and objects: the history
//    follows the camera only);
// 4. `denoise` (then `denoise_final`): XeGTAO's edge-aware 3 x 3 blur. Without
//    the accumulation (`AURORA_GTAO_TEMPORAL=0`) the noise stays the same
//    every frame, so the result does not flicker, and the blur alone removes
//    its pattern, leaving a fine grain in the penumbra.
//
// The result is a visibility (1 = nothing occludes) in `.r` of the last
// target, sampled where the Legacy occlusion is (screen_ao, common.wgsl),
// and, for the specular occlusion (specular_occlusion, common.wgsl), a
// visibility blurred more in `.g` and the bent normal — the mean direction
// of the unoccluded part of the hemisphere (XE_GTAO_COMPUTE_BENT_NORMALS,
// the paper's algorithm 2) — in view space, its x and y in `.ba` mapped
// from -1..1 to 0..1 (it faces the camera: z is the negative root). The
// working targets hold the same, unpacked (`working`); `denoise_bent`
// blurs all but `.r` a few more times.
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
    // 2), slices of `main` (`quality.z`, or more where the accumulation has
    // no history), 1 when the surface normal is the mesh's (`normal_tex`)
    // rather than the depth's
    misc: vec4<f32>,
    // screen-space indirect light (main_bitmask): this frame's view space
    // (z in front) to the clip space of the previous frame, whose lit
    // colour `scene_tex` holds
    reproj: mat4x4<f32>,
    // its thickness of a surface (metres), radius (metres), strength,
    // unused
    il: vec4<f32>,
    // the accumulation (`temporal`): the noise's frame index (0 without
    // it), 1 when there is no history to read (first frame, new targets, a
    // frame without GTAO before), the current frame's least weight, the
    // half width of the neighbourhood clip in standard deviations
    temporal: vec4<f32>,
    // world directions to this view space, for the mesh normals
    normal_view: mat3x3<f32>,
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
@group(0) @binding(10) var il_in: texture_2d<f32>;
// the mesh normals the depth prepass draws at the window's resolution
// (world, octahedral 0..1, as in the G-buffer), with `misc.w`
@group(0) @binding(11) var normal_tex: texture_2d<f32>;
@group(0) @binding(12) var il_final: texture_storage_2d<rgba16float, write>;
@group(0) @binding(13) var scene_tex: texture_2d<f32>;
@group(0) @binding(14) var il_out: texture_storage_2d<rgba16float, write>;
// the accumulation's history, read from the previous frame and written for
// the next: the working values (`working`), and the view depth with the
// number of frames gathered (pack2x16float)
@group(0) @binding(15) var hist_in: texture_2d<f32>;
@group(0) @binding(16) var meta_in: texture_2d<u32>;
@group(0) @binding(17) var hist_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(18) var meta_out: texture_storage_2d<r32uint, write>;

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
// 64 x 64 tile (HilbertIndex). Written with `select` rather than XeGTAO's
// nested `if` that swaps the coordinates: on Linux with the NVIDIA driver
// (615) that form came out wrong for whole quadrants of the tile, the noise
// turned into rows and the occlusion into crawling horizontal stripes.
const HILBERT_WIDTH: u32 = 64u;

fn hilbert_index(px: u32, py: u32) -> u32 {
    var x = px;
    var y = py;
    var index = 0u;
    for (var shift = 5u; shift < 6u; shift--) {
        let level = 1u << shift;
        let rx = (x >> shift) & 1u;
        let ry = (y >> shift) & 1u;
        index += level * level * ((3u * rx) ^ ry);
        // the quadrant's frame: mirrored, then the axes swapped, when ry is 0
        let flip = ry == 0u && rx == 1u;
        let mx = select(x, HILBERT_WIDTH - 1u - x, flip);
        let my = select(y, HILBERT_WIDTH - 1u - y, flip);
        x = select(mx, my, ry == 0u);
        y = select(my, mx, ry == 0u);
    }
    return index;
}

// SpatioTemporalNoise: the Hilbert index drives the R2 sequence, so
// neighbouring pixels get well spread slice and sample offsets (a
// low-discrepancy pattern the blur takes out), and the frame index `t`
// moves it along by XeGTAO's 288 per frame, over 64 frames, for the
// accumulation to gather different samples (0: the same every frame).
fn pixel_noise(px: vec2<u32>, t: u32) -> vec2<f32> {
    let index = hilbert_index(px.x % HILBERT_WIDTH, px.y % HILBERT_WIDTH) + 288u * (t % 64u);
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

// The octahedral normal of the G-buffer (ssr.wgsl).
fn oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// The surface normal (view space) the occlusion of pixel `px`, whose point
// is `centre`, is computed around. With `misc.w`, the mesh's smooth normal,
// from the full-resolution pixel the depth of `prefilter_depth0` came from,
// as XeGTAO takes the normals of a G-buffer when there is one; it stays as
// it is (near a silhouette it may face slightly away, and the slices clamp
// its angle). Without, the flat triangle's, XeGTAO_CalculateNormal from the
// depths around it (`z`: left, right, top, bottom) turned towards the camera.
fn surface_normal(px: vec2<i32>, uv: vec2<f32>, pixel: vec2<f32>, centre: vec3<f32>, edges: vec4<f32>, z: vec4<f32>) -> vec3<f32> {
    if (g.misc.w > 0.5) {
        let full = vec2<i32>(textureDimensions(normal_tex));
        let src = min(px * i32(g.misc.y), full - vec2<i32>(1));
        return normalize(g.normal_view * oct_decode(textureLoad(normal_tex, src, 0).xy));
    }
    var n = calculate_normal(
        edges,
        centre,
        view_position(uv + vec2<f32>(-pixel.x, 0.0), z.x),
        view_position(uv + vec2<f32>(pixel.x, 0.0), z.y),
        view_position(uv + vec2<f32>(0.0, -pixel.y), z.z),
        view_position(uv + vec2<f32>(0.0, pixel.y), z.w),
    );
    if (dot(n, -centre) < 0.0) {
        n = -n;
    }
    return n;
}

// Under the smooth normal of a low-poly mesh, the flat triangle rises above
// the smooth tangent plane on one side, by up to the angle between the two,
// and its own samples occluded it there (a ring along the silhouette of
// round prims, a band at the avatar's neck); across a concave crease the
// neighbouring triangle rises by about half the crease angle, and drew the
// mesh's edges as lines. How much a sample in direction `dir` (unit, from
// the pixel's point) counts as an occluder: nothing within SURFACE_BIAS_LO
// of the smooth tangent plane (the sine of 20°, beyond the facets and
// creases of the usual low-poly prims and bodies), fully from
// SURFACE_BIAS_HI (the sine of 44°), smoothly between. Measured from the
// smooth normal `n`, which does not jump from one triangle to the next, so
// neither does the occlusion: measured from the flat triangle's plane, as
// #1556 did, an occluder in that band (a chin, the chest between the front
// legs) counted more on one triangle than on the next, and showed the
// facets again in concave folds (#993). What it gives up is the occlusion
// of the shallowest folds, slightly lighter than before.
const SURFACE_BIAS_LO: f32 = 0.34;
const SURFACE_BIAS_HI: f32 = 0.7;

fn above_surface(dir: vec3<f32>, n: vec3<f32>) -> f32 {
    return smoothstep(SURFACE_BIAS_LO, SURFACE_BIAS_HI, dot(dir, n));
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

// The working target: the visibility / scale twice — `.r` blurred as the
// occlusion is, `.g` blurred more for the specular occlusion — and the
// bent normal's x and y in `.ba` (it faces the camera: its z is the
// negative root, `bent_of`).
fn working(visibility: f32, bent: vec3<f32>) -> vec4<f32> {
    let v = saturate(visibility / OCCLUSION_TERM_SCALE);
    return vec4<f32>(v, v, bent.xy);
}

// The bent normal of a working texel's `.ba`.
fn bent_of(xy: vec2<f32>) -> vec3<f32> {
    let l = length(xy);
    let flat = select(xy, xy / l, l > 1.0);
    return vec3<f32>(flat, -sqrt(max(1.0 - dot(flat, flat), 0.0)));
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
    let n = surface_normal(px, uv, pixel, centre, edges, vec4<f32>(zl, zr, zt, zb));
    let view_vec = normalize(-centre);

    let effect_radius = g.radius.x;
    let sample_distribution_power = g.radius.z;
    let thin_occluder_compensation = g.radius.w;
    let falloff_range = g.radius.y * effect_radius;
    let falloff_from = effect_radius * (1.0 - g.radius.y);
    let falloff_mul = -1.0 / falloff_range;
    let falloff_add = falloff_from / falloff_range + 1.0;
    // (more on a frame without history: a photo, new targets)
    let slice_count = g.misc.z;
    let steps_per_slice = g.quality.w;

    let noise = pixel_noise(id.xy, u32(g.temporal.x));
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
            var weight0 = saturate(falloff_base0 * falloff_mul + falloff_add);
            var weight1 = saturate(falloff_base1 * falloff_mul + falloff_add);
            if (g.misc.w > 0.5) {
                // (the mesh normal: the facets' own samples left out)
                weight0 *= above_surface(horizon_vec0, n);
                weight1 *= above_surface(horizon_vec1, n);
            }
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
        // (XeGTAO's clamp: fast_acos is approximate, and the arc must stay
        // within the hemisphere around the normal)
        let h0 = nn + clamp(-fast_acos(horizon1) - nn, -PI_HALF, PI_HALF);
        let h1 = nn + clamp(fast_acos(horizon0) - nn, -PI_HALF, PI_HALF);
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

// ---- 3 bis: the indirect light, on a visibility bitmask

// The sectors of a slice, 32 bits from one side of the hemisphere around
// the normal to the other, each covering the same share of the cosine-
// weighted half disc (uniform in sin of the angle from the normal): their
// count is the slice's cosine-weighted visibility as is.
fn sector(angle: f32, nn: f32) -> f32 {
    return (sin(clamp(angle - nn, -PI_HALF, PI_HALF)) + 1.0) * 0.5;
}

// The sectors between `t0` and `t1` (0..1, either order).
fn sectors_between(t0: f32, t1: f32) -> u32 {
    let lo = u32(clamp(floor(min(t0, t1) * 32.0), 0.0, 32.0));
    let hi = u32(clamp(ceil(max(t0, t1) * 32.0), 0.0, 32.0));
    if (hi <= lo) {
        return 0u;
    }
    let count = hi - lo;
    let ones = select((1u << count) - 1u, 0xffffffffu, count >= 32u);
    return ones << lo;
}

// The lit colour of the previous frame where the view-space point `p` was.
fn previous_radiance(p: vec3<f32>) -> vec4<f32> {
    let clip = g.reproj * vec4<f32>(p, 1.0);
    if (clip.w <= 1e-4) {
        return vec4<f32>(0.0);
    }
    let ndc = clip.xy / clip.w;
    if (any(abs(ndc) > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let size = vec2<f32>(textureDimensions(scene_tex));
    let c = vec2<i32>(clamp((vec2<f32>(ndc.x, -ndc.y) * 0.5 + vec2<f32>(0.5)) * size, vec2<f32>(0.0), size - vec2<f32>(1.0)));
    let l = textureLoad(scene_tex, c, 0).rgb;
    // (no light from an unlit or broken texel)
    return vec4<f32>(clamp(l, vec3<f32>(0.0), vec3<f32>(16.0)), 1.0);
}

// Therrien, Levesque and Gilet, "Screen Space Indirect Lighting with
// Visibility Bitmask" (2023), on GTAO's slices: each sample is a surface of
// some thickness (its front and its back, pushed away from the camera),
// which covers the sectors between their two angles, and lights the pixel
// with the lit colour of the previous frame there, for the sectors it is
// the first to cover (one bounce). Only the light comes from here, into
// `il_out`: the occlusion and its bent normal stay `main`'s, which runs
// first, so turning the light on does not change them (the bitmask's own
// visibility, with thin things occluding only what they cover, was softer
// in the corners and contacts than GTAO's).
@compute @workgroup_size(8, 8)
fn main_bitmask(@builtin(global_invocation_id) id: vec3<u32>) {
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
    if (zc >= FAR * 0.5) {
        textureStore(il_out, px, vec4<f32>(0.0));
        return;
    }
    let z = zc * 0.99999;
    let centre = view_position(uv, z);
    let n = surface_normal(px, uv, pixel, centre, edges, vec4<f32>(zl, zr, zt, zb));
    let view_vec = normalize(-centre);

    let thickness = g.il.x;
    let radius = g.il.y;
    let sample_distribution_power = g.radius.z;
    let slice_count = g.quality.z;
    let steps_per_slice = g.quality.w;
    // (the light is not accumulated: its noise stays the same every frame)
    let noise = pixel_noise(id.xy, 0u);
    let pixel_view_size = z * 2.0 / (g.proj.x * size.x);
    let screen_radius = radius / pixel_view_size;
    let min_s = 1.3 / screen_radius;
    var weight_sum = 0.0;
    var light = vec3<f32>(0.0);

    for (var si = 0; si < i32(slice_count); si++) {
        let slice_k = (f32(si) + noise.x) / slice_count;
        let phi = slice_k * PI;
        let cos_phi = cos(phi);
        let sin_phi = sin(phi);
        let omega = vec2<f32>(cos_phi, -sin_phi) * screen_radius;
        let direction = vec3<f32>(cos_phi, sin_phi, 0.0);
        let ortho_direction = direction - dot(direction, view_vec) * view_vec;
        let axis = normalize(cross(ortho_direction, view_vec));
        let projected_normal = n - axis * dot(n, axis);
        let sign_norm = sign(dot(ortho_direction, projected_normal));
        let projected_length = length(projected_normal);
        let cos_norm = saturate(dot(projected_normal, view_vec) / max(projected_length, 1e-6));
        let nn = sign_norm * fast_acos(cos_norm);
        var occluded = 0u;
        var slice_light = vec3<f32>(0.0);

        for (var st = 0; st < i32(steps_per_slice); st++) {
            let step_noise = fract(noise.y + f32(si + st * i32(steps_per_slice)) * 0.6180339887498948482);
            var s = (f32(st) + step_noise) / steps_per_slice;
            s = pow(s, sample_distribution_power) + min_s;
            var offset = s * omega;
            let mip = clamp(log2(length(offset)) - g.quality.y, 0.0, MIP_LEVELS);
            offset = round(offset) * pixel;
            // side 0 along the slice's direction (positive angles), side 1
            // against it
            for (var side = 0; side < 2; side++) {
                let sgn = select(-1.0, 1.0, side == 0);
                let suv = uv + offset * sgn;
                let zs = depth_at(suv, mip);
                if (zs >= FAR * 0.5) {
                    continue;
                }
                let front = view_position(suv, zs);
                if (distance(front, centre) > radius) {
                    continue;
                }
                let dir = normalize(front - centre);
                // the facets' own samples left out, as in `main`: they
                // would light it with its own colour
                var facet = 1.0;
                if (g.misc.w > 0.5) {
                    facet = above_surface(dir, n);
                    if (facet <= 0.0) {
                        continue;
                    }
                }
                let back = front + normalize(front) * thickness;
                let af = sgn * fast_acos(clamp(dot(dir, view_vec), -1.0, 1.0));
                // a sample on the pixel's own plane (or below it) hides
                // nothing: without this margin a flat floor occluded itself
                // by a sector on each side
                if (sgn * (af - nn) > PI_HALF - 0.1) {
                    continue;
                }
                let ab = sgn * fast_acos(clamp(dot(normalize(back - centre), view_vec), -1.0, 1.0));
                let mask = sectors_between(sector(af, nn), sector(ab, nn));
                let fresh = mask & ~occluded;
                if (fresh != 0u) {
                    let l = previous_radiance(front);
                    slice_light += l.rgb * (facet * f32(countOneBits(fresh)) / 32.0);
                }
                occluded |= mask;
            }
        }

        light += projected_length * slice_light;
        weight_sum += projected_length;
    }
    light /= max(weight_sum, 1e-4);
    textureStore(il_out, px, vec4<f32>(light * g.il.z, 1.0));
}

// ---- 3 ter: the temporal accumulation

// `v`, or `fallback` for its components that are NaN.
fn finite_or(v: vec4<f32>, fallback: vec4<f32>) -> vec4<f32> {
    return select(v, fallback, v != v);
}

// The history's depth (metres in front) and frame count at `q`.
fn history_meta(q: vec2<i32>) -> vec2<f32> {
    return unpack2x16float(textureLoad(meta_in, q, 0).x);
}

// One pixel of `main`'s result (`ao_in`) blended with the history where its
// point was in the previous frame (`g.reproj`, the camera's motion: there
// are no motion vectors), into `hist_out`, which the blur then reads and
// the next frame takes as its history. The four history texels around the
// point count by their bilinear weight, each only where its depth is the
// point's (an uncovered surface starts again from the current frame), else
// the pixel's own texel if its depth is the same (what the camera follows); the
// history is clipped to the mean and deviation of the current 3 x 3
// neighbourhood, so an occlusion that moved with an avatar or an object
// does not trail behind it. The current frame weighs 1 / (frames + 1),
// never less than `g.temporal.z`: a new pixel converges in a few frames.
@compute @workgroup_size(8, 8)
fn temporal(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = target_size();
    if (f32(id.x) >= size.x || f32(id.y) >= size.y) {
        return;
    }
    let px = vec2<i32>(id.xy);
    // (a NaN in the history would stay there: none is let in)
    let cur = finite_or(ao_at(px), working(1.0, vec3<f32>(0.0, 0.0, -1.0)));
    let zc = depth0(px);
    if (zc >= FAR * 0.5) {
        textureStore(hist_out, px, cur);
        textureStore(meta_out, px, vec4<u32>(pack2x16float(vec2<f32>(0.0)), 0u, 0u, 0u));
        return;
    }
    var m1 = vec4<f32>(0.0);
    var m2 = vec4<f32>(0.0);
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let s = finite_or(ao_at(px + vec2<i32>(dx, dy)), cur);
            m1 += s;
            m2 += s * s;
        }
    }
    m1 /= 9.0;
    let sigma = sqrt(max(m2 / 9.0 - m1 * m1, vec4<f32>(0.0)));

    var count = 0.0;
    var hist = cur;
    let isize = vec2<i32>(size);
    let uv = (vec2<f32>(px) + vec2<f32>(0.5)) / size;
    let clip = g.reproj * vec4<f32>(view_position(uv, zc), 1.0);
    if (g.temporal.y < 0.5 && clip.w > 1e-4) {
        let ndc = clip.xy / clip.w;
        let pp = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * size - vec2<f32>(0.5);
        let base = vec2<i32>(floor(pp));
        let f = pp - floor(pp);
        // the history texels lie within a pixel of the point: allow the
        // surface's own slope over that pixel (the smaller step on each
        // axis, so a depth edge beside the pixel does not count), and a
        // little more for the depths' precision
        let gx = min(abs(depth0(px + vec2<i32>(-1, 0)) - zc), abs(depth0(px + vec2<i32>(1, 0)) - zc));
        let gy = min(abs(depth0(px + vec2<i32>(0, -1)) - zc), abs(depth0(px + vec2<i32>(0, 1)) - zc));
        let tolerance = 0.01 * clip.w + 1.5 * (gx + gy);
        var weight = 0.0;
        var sum = vec4<f32>(0.0);
        for (var k = 0; k < 4; k++) {
            let o = vec2<i32>(k & 1, k >> 1u);
            let q = base + o;
            if (any(q < vec2<i32>(0)) || any(q >= isize)) {
                continue;
            }
            let m = history_meta(q);
            let h = textureLoad(hist_in, q, 0);
            if (m.y < 0.5 || abs(m.x - clip.w) > tolerance || any(h != h)) {
                continue;
            }
            let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
            weight += bw;
            sum += h * bw;
            count += m.y * bw;
        }
        // what moves with the camera (our own avatar, which the camera
        // follows as it walks) is where it was on screen, at its depth: the
        // same pixel, when the camera's motion found nothing
        let m = history_meta(px);
        let h = textureLoad(hist_in, px, 0);
        if (weight <= 1e-3 && m.y >= 0.5 && abs(m.x - zc) <= 0.01 * zc + 1.5 * (gx + gy) && all(h == h)) {
            weight = 1.0;
            sum = h;
            count = m.y;
        }
        if (weight > 1e-3) {
            let gamma = g.temporal.w;
            hist = clamp(sum / weight, m1 - gamma * sigma, m1 + gamma * sigma);
        } else {
            count = 0.0;
        }
    }
    let alpha = max(1.0 / (count + 1.0), g.temporal.z);
    textureStore(hist_out, px, mix(hist, cur, alpha));
    let frames = min(count + 1.0, 255.0);
    textureStore(meta_out, px, vec4<u32>(pack2x16float(vec2<f32>(min(zc, 60000.0), frames)), 0u, 0u, 0u));
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

fn il_at(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(il_in));
    return textureLoad(il_in, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// XeGTAO_Denoise's weights for one pixel: its own (`centre`), its four
// neighbours (left, right, top, bottom) and its four diagonals (top left,
// top right, bottom left, bottom right), from the edges between them; the
// last pass weights the centre more (DenoiseBlurBeta) so the image stays
// sharp.
struct BlurWeights {
    centre: f32,
    sides: vec4<f32>,
    diagonals: vec4<f32>,
};

fn blur_weights(px: vec2<i32>, final_apply: bool) -> BlurWeights {
    let beta = g.misc.x;
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
    var w: BlurWeights;
    w.centre = select(beta / 5.0, beta, final_apply);
    w.sides = ec;
    w.diagonals = diag * vec4<f32>(
        ec.x * el.z + ec.z * et.x,
        ec.z * et.y + ec.y * er.z,
        ec.w * eb.x + ec.x * el.w,
        ec.y * er.w + ec.w * eb.y,
    );
    return w;
}

// The light of the indirect pass blurred with the same weights as the
// occlusion.
fn denoised_il(px: vec2<i32>, final_apply: bool) -> vec4<f32> {
    let w = blur_weights(px, final_apply);
    var sum = il_at(px) * w.centre;
    sum += il_at(px + vec2<i32>(-1, 0)) * w.sides.x;
    sum += il_at(px + vec2<i32>(1, 0)) * w.sides.y;
    sum += il_at(px + vec2<i32>(0, -1)) * w.sides.z;
    sum += il_at(px + vec2<i32>(0, 1)) * w.sides.w;
    sum += il_at(px + vec2<i32>(-1, -1)) * w.diagonals.x;
    sum += il_at(px + vec2<i32>(1, -1)) * w.diagonals.y;
    sum += il_at(px + vec2<i32>(-1, 1)) * w.diagonals.z;
    sum += il_at(px + vec2<i32>(1, 1)) * w.diagonals.w;
    return sum / (w.centre + dot(w.sides, vec4<f32>(1.0)) + dot(w.diagonals, vec4<f32>(1.0)));
}

@compute @workgroup_size(8, 8)
fn denoise_il(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(il_out))) {
        return;
    }
    let px = vec2<i32>(id.xy);
    textureStore(il_out, px, denoised_il(px, false));
}

@compute @workgroup_size(8, 8)
fn denoise_il_final(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(il_final))) {
        return;
    }
    let px = vec2<i32>(id.xy);
    textureStore(il_final, px, denoised_il(px, true));
}

// XeGTAO_Denoise for one pixel: the visibility and the bent normal.
fn denoised(px: vec2<i32>, final_apply: bool) -> vec4<f32> {
    let w = blur_weights(px, final_apply);
    var sum = ao_at(px) * w.centre;
    sum += ao_at(px + vec2<i32>(-1, 0)) * w.sides.x;
    sum += ao_at(px + vec2<i32>(1, 0)) * w.sides.y;
    sum += ao_at(px + vec2<i32>(0, -1)) * w.sides.z;
    sum += ao_at(px + vec2<i32>(0, 1)) * w.sides.w;
    sum += ao_at(px + vec2<i32>(-1, -1)) * w.diagonals.x;
    sum += ao_at(px + vec2<i32>(1, -1)) * w.diagonals.y;
    sum += ao_at(px + vec2<i32>(-1, 1)) * w.diagonals.z;
    sum += ao_at(px + vec2<i32>(1, 1)) * w.diagonals.w;
    return sum / (w.centre + dot(w.sides, vec4<f32>(1.0)) + dot(w.diagonals, vec4<f32>(1.0)));
}

// The specular occlusion's visibility and the bent normal blurred once
// more, the occlusion's own visibility kept: GTAO's few slices leave
// patches in both, which the specular occlusion (a cone test, sharp at its
// edge, on metal that has no diffuse to hide them) showed as dark blotches;
// the occlusion itself keeps its detail.
@compute @workgroup_size(8, 8)
fn denoise_bent(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(ao_out))) {
        return;
    }
    let px = vec2<i32>(id.xy);
    textureStore(ao_out, px, vec4<f32>(ao_at(px).r, denoised(px, false).gba));
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
    let v = saturate(d.rg * OCCLUSION_TERM_SCALE);
    let b = bent_of(d.ba);
    textureStore(final_out, px, vec4<f32>(v, b.xy * 0.5 + vec2<f32>(0.5)));
}
