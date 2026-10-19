//! What can cast a shadow into each cascade.
//!
//! A cascade covers a square around its split of the view, wider than the
//! split so that its texels stay still when the camera turns
//! (`Backend::cascade_matrices`). Only part of the casters inside that
//! square can shadow something the main view samples in the cascade: those
//! that touch the split's volume swept towards the light. The culling
//! (cull.wgsl on the GPU, the renderer's CPU lists) leaves the others out of
//! the cascade; the shadow map differs only in texels no pixel reads.
//!
//! Three things make the path ([`path`]): that culling; opaque casters drawn
//! without a fragment shader; and the cascade of a pixel chosen from its own
//! distance, which gives the far cascades a nearest receiver, so that the
//! culling can leave the casters near the camera out of them.
//! AURORA_SHADOW_LEGACY brings back the path before this module, to compare.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use glam::{Mat4, Vec3};

/// Parts of the shadow path, which AURORA_SHADOW_LEGACY turns off.
pub mod path {
    /// Casters culled to what the view can receive in each cascade
    /// (`caster_planes`).
    pub const TIGHT: u32 = 1;
    /// Opaque casters drawn without a fragment shader.
    pub const DEPTH_ONLY: u32 = 2;
    /// The cascade of a pixel chosen from its own distance to the camera
    /// instead of the one interpolated from its triangle's vertices, which
    /// lets `TIGHT` leave the near casters out of the far cascades.
    pub const PIXEL_SPLITS: u32 = 4;
    /// An opaque caster that the cascade before holds is left out of a
    /// cascade (`merged_from_finer`), which takes its depth from that finer
    /// map instead (shadow_merge.rs): most casters are drawn in one cascade.
    pub const MERGE: u32 = 8;
    pub const LEGACY: u32 = 0;
    pub const NEW: u32 = TIGHT | DEPTH_ONLY | PIXEL_SPLITS | MERGE;
    /// The new path with the cascades chosen as before: the image of
    /// `LEGACY` to the pixel.
    pub const VERTEX_SPLITS: u32 = TIGHT | DEPTH_ONLY;
    /// The path before `MERGE`: each cascade draws all its casters.
    pub const SEPARATE: u32 = TIGHT | DEPTH_ONLY | PIXEL_SPLITS;
}

/// Paths the session goes through, one per profile period.
/// AURORA_SHADOW_LEGACY: `1` the path before this module; `splits` only
/// its choice of the cascades; `separate` every caster drawn in each
/// cascade it shadows, as before `path::MERGE`; `ab` (with AURORA_PROFILE,
/// whose periods drive the cycle) the old path, the new one and `splits`
/// in turn, a second each, to compare them in the same session and under
/// the same outside load; `merge` likewise `separate` one second in three
/// (not two: work that comes back every other second would always fall on
/// the same path).
fn cycle() -> &'static [u32] {
    static CYCLE: OnceLock<Vec<u32>> = OnceLock::new();
    let profile = std::env::var_os("AURORA_PROFILE").is_some();
    CYCLE.get_or_init(|| match std::env::var("AURORA_SHADOW_LEGACY").as_deref() {
        Ok("ab") if profile => vec![path::LEGACY, path::NEW, path::VERTEX_SPLITS],
        Ok("merge") if profile => vec![path::SEPARATE, path::NEW, path::NEW],
        Ok("splits") => vec![path::VERTEX_SPLITS],
        Ok("separate") => vec![path::SEPARATE],
        Ok("" | "0" | "ab" | "merge") | Err(_) => vec![path::NEW],
        Ok(_) => vec![path::LEGACY],
    })
}

/// Alpha-blended faces that are not rigged cast sun and projector shadows,
/// as in LLPipeline::renderShadow (indra/newview/pipeline.cpp, originally
/// LGPL 2.1), whose renderAlphaObjects draws rigged and static blended faces
/// alike: glass with opaque frames, blended foliage. AURORA_BLEND_SHADOWS=0
/// leaves them out as before (only rigged ones cast), to compare.
pub fn blend_casts() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_BLEND_SHADOWS").as_deref() != Ok("0"))
}

static STEP: AtomicUsize = AtomicUsize::new(0);

/// The `path` bits frames are drawn with now.
pub fn current_path() -> u32 {
    let c = cycle();
    c[STEP.load(Ordering::Relaxed) % c.len()]
}

/// End of a profile period: the path its frames were drawn with (the
/// render thread reads it a frame or two late), then the next one of the
/// cycle takes over.
pub fn next_path() -> u32 {
    let c = cycle();
    c[STEP.fetch_add(1, Ordering::Relaxed) % c.len()]
}

/// Name of a path in the profile (`shadow_path=`).
pub fn path_name(bits: u32) -> &'static str {
    match bits {
        path::NEW => "new",
        path::LEGACY => "legacy",
        path::SEPARATE => "separate",
        _ => "splits",
    }
}

/// Planes of a cascade's caster volume (`caster_planes`).
pub const SHADOW_PLANES: usize = 12;
/// A plane every point is inside of (padding of the plane lists).
const PLANE_ALL: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Caster planes that keep every caster of the cascade.
pub const CASTER_PLANES_ALL: [[f32; 4]; SHADOW_PLANES] = [PLANE_ALL; SHADOW_PLANES];

/// Corners of a view frustum slice, as `caster_planes` takes them: the four
/// of the near side then the four of the far side, each (-x, -y), (+x, -y),
/// (-x, +y), (+x, +y). `tan_x`, `tan_y`: tangents of the half angles.
pub fn slice_corners(inv_view: Mat4, tan_x: f32, tan_y: f32, near: f32, far: f32) -> [Vec3; 8] {
    let mut out = [Vec3::ZERO; 8];
    for (i, d) in [near, far].into_iter().enumerate() {
        for (j, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
            out[i * 4 + j] = inv_view.transform_point3(Vec3::new(sx * d * tan_x, sy * d * tan_y, -d));
        }
    }
    out
}

/// Planes (inside: positive) of the volume a shadow caster must touch to
/// shadow a point of the convex `receivers` (`slice_corners`) when the light
/// travels along `light`: the receivers swept towards the light without end.
/// Its faces are the receivers' faces that look away from the light and,
/// for each edge between such a face and one that looks at the light, the
/// plane through that edge along the light. A caster whose sphere is
/// outside one of them (`casts_into`) shadows nothing in `receivers`,
/// however long its shadow: Firestorm reaches the same casters by fitting
/// each shadow frustum to the visible part of its split
/// (`LLPipeline::getVisiblePointCloud`), where Aurora keeps a cascade wider
/// than its split so that its texels stay still.
///
/// `margin` moves every plane outwards (filter width and biases of the
/// shadow lookup). Planes that cannot be built (flat receivers, edge along
/// the light) are left out, which only keeps more casters.
pub fn caster_planes(receivers: &[Vec3; 8], light: Vec3, margin: f32) -> [[f32; 4]; SHADOW_PLANES] {
    const FACES: [[usize; 4]; 6] = [[0, 1, 3, 2], [4, 5, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5], [0, 1, 5, 4], [2, 3, 7, 6]];
    // (corner, corner, face, face)
    const EDGES: [(usize, usize, usize, usize); 12] = [
        (0, 1, 0, 4),
        (2, 3, 0, 5),
        (0, 2, 0, 2),
        (1, 3, 0, 3),
        (4, 5, 1, 4),
        (6, 7, 1, 5),
        (4, 6, 1, 2),
        (5, 7, 1, 3),
        (0, 4, 2, 4),
        (1, 5, 3, 4),
        (2, 6, 2, 5),
        (3, 7, 3, 5),
    ];
    let mut out = CASTER_PLANES_ALL;
    let Some(light) = light.try_normalize() else {
        return out;
    };
    let center = receivers.iter().fold(Vec3::ZERO, |a, c| a + *c) / 8.0;
    let size = receivers.iter().map(|c| c.distance(center)).fold(0.0f32, f32::max);
    // a plane the receivers are not all inside of is no face of the volume
    let holds = |n: Vec3, d: f32| receivers.iter().all(|c| n.dot(*c) + d >= -1e-3 * size.max(1.0));
    let mut planes: Vec<(Vec3, f32)> = Vec::with_capacity(SHADOW_PLANES);
    // per face: looks at the light (the light enters the receivers by it)
    let mut lit = [false; 6];
    for (f, q) in FACES.iter().enumerate() {
        let a = receivers[q[0]];
        let Some(mut n) = (receivers[q[1]] - a).cross(receivers[q[3]] - a).try_normalize() else {
            return out;
        };
        if n.dot(center - a) < 0.0 {
            n = -n;
        }
        lit[f] = n.dot(light) > 0.0;
        let d = -n.dot(a);
        if !lit[f] && holds(n, d) {
            planes.push((n, d));
        }
    }
    for (a, b, f1, f2) in EDGES {
        if lit[f1] == lit[f2] {
            continue;
        }
        let (a, b) = (receivers[a], receivers[b]);
        let Some(mut n) = (b - a).cross(light).try_normalize() else {
            continue;
        };
        if n.dot(center - a) < 0.0 {
            n = -n;
        }
        let d = -n.dot(a);
        if holds(n, d) {
            planes.push((n, d));
        }
    }
    for (o, (n, d)) in out.iter_mut().zip(planes) {
        *o = [n.x, n.y, n.z, d + margin];
    }
    out
}

/// A caster outside one of the cascade's `caster_planes` casts no shadow
/// the main view samples in that cascade (as `casts_into` in cull.wgsl).
pub fn casts_into(planes: &[[f32; 4]; SHADOW_PLANES], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| Vec3::new(p[0], p[1], p[2]).dot(c) + p[3] >= -r)
}

/// The view whose pixels read the shadow map.
#[derive(Debug, Clone, Copy)]
pub struct ReceiverView {
    pub inv_view: Mat4,
    /// Tangents of the half angles of its projection.
    pub tan_x: f32,
    pub tan_y: f32,
    pub near: f32,
}

/// Distances from the camera between which `shadow_factor` (common.wgsl)
/// reads each cascade, from the ends of the cascades (`splits`): a cascade
/// is read from the start of the blend band that ends the one before, the
/// last 15 % of it. With `pixel_splits` these are the distances of the
/// pixels themselves; without, the cascade comes from the distance
/// interpolated from the triangle's vertices (`view_depth`, object.wgsl),
/// longer than the pixel's own on a triangle that spans a range of
/// distances: a near pixel of a large triangle reads a far cascade, and no
/// cascade has a nearest receiver.
pub fn cascade_reach(splits: [f32; 3], pixel_splits: bool) -> ([f32; 3], [f32; 3]) {
    let near = if pixel_splits {
        [
            0.0,
            splits[0] * (1.0 - BLEND_BAND),
            splits[1] - (splits[1] - splits[0]) * BLEND_BAND,
        ]
    } else {
        [0.0; 3]
    };
    (near, splits)
}

/// Part of a cascade, at its far end, blended into the next one
/// (`shadow_factor`).
const BLEND_BAND: f32 = 0.15;

/// Texels around a lookup that `cascade_shadow` (common.wgsl) may read:
/// taps over three spacings of at most two texels, each a bilinear
/// footprint.
const FILTER_TEXELS: f32 = 6.0;

/// Taps per axis of the shadow filter for a span of `span_texels`
/// (`shadow_taps` in common.wgsl): never more than a texel apart, since a
/// bilinear compare tap only blends over one texel and wider gaps draw
/// parallel lines along the shadow edges; 4 when they already are not.
pub fn shadow_taps(span_texels: f32) -> u32 {
    ((span_texels - 1e-3).ceil().max(0.0) as u32 + 1).clamp(4, 7)
}

/// Caster planes of the three cascades. `near[k]`, `far[k]`: the distances
/// from the camera between which a pixel may read cascade `k`
/// (`cascade_reach`); `texel[k]`: its texel in metres.
///
/// The receivers of a cascade are the view's pixels between those
/// distances. The volume is taken a little wider than the view (TAA
/// jitter, pixels extrapolated at triangle edges with MSAA) and the planes
/// are moved out by what a lookup reads around its point (filter taps,
/// normal offset).
pub fn cascade_caster_planes(
    view: &ReceiverView,
    light: Vec3,
    near: [f32; 3],
    far: [f32; 3],
    texel: [f32; 3],
) -> [[[f32; 4]; SHADOW_PLANES]; 3] {
    let (tan_x, tan_y) = (view.tan_x * 1.02, view.tan_y * 1.02);
    // a distance from the camera is the shortest depth along the view's
    // corner rays
    let corner_cos = 1.0 / (1.0 + tan_x * tan_x + tan_y * tan_y).sqrt();
    std::array::from_fn(|k| {
        let corners = slice_corners(
            view.inv_view,
            tan_x,
            tan_y,
            (near[k] * corner_cos * 0.98 - 0.05).max(view.near * 0.5),
            far[k] * 1.05 + 1.0,
        );
        caster_planes(&corners, light, lookup_margin(k, texel[k]))
    })
}

/// What a lookup of cascade `k` reads around its point, in metres: the
/// filter's taps and the normal offset (0.02 + 0.04 k) of `cascade_shadow`
/// (common.wgsl), and a little more.
pub fn lookup_margin(k: usize, texel: f32) -> f32 {
    FILTER_TEXELS * texel + 0.02 + 0.04 * k as f32 + 0.05
}

/// Depth a cascade keeps on each side of its square, along the light
/// (`Backend::cascade_matrices`).
pub const CASCADE_DEPTH_PAD: f32 = 400.0;

/// Texels of a cascade kept between a caster it leaves to the cascade
/// before and the edge of that cascade's square (`merged_from_finer`).
pub const MERGE_TEXELS: f32 = 2.0;
/// Slope-scaled depth bias of the shadow pipelines (renderer.rs), which the
/// merge makes up for the size of the coarser texels.
pub const SLOPE_BIAS: f32 = 2.0;
/// Most the merge adds to a depth for the slope, in metres: at a
/// silhouette the slope read from the map is not that of a surface.
const MERGE_BIAS_MAX: f32 = 0.3;

/// Scale of a cascade, in clip units per metre across the light:
/// orthographic, so the first row of its matrix is the light's right axis
/// over the half extent of its square.
pub fn cascade_scale(m: &Mat4) -> f32 {
    m.row(0).truncate().length()
}

/// With `path::MERGE`: cascade `k` need not draw the opaque caster
/// (`c`, `r`), its depth comes from the map of the cascade before
/// (`merged_from_finer` in cull.wgsl, kept line for line). The cascades
/// look along the same light, so a caster covers the same place in each of
/// them: when the cascade before holds it — it draws it, or leaves it in
/// turn to the one before — and the caster is inside that cascade's square
/// with `MERGE_TEXELS` to spare, every texel of `k` it would have covered
/// is filled from the finer map (shadow_merge.rs). `shadow_size`: side of a
/// cascade in texels (0: no merge).
pub fn merged_from_finer(
    cascades: &[Mat4; 3],
    planes: &[[[f32; 4]; SHADOW_PLANES]; 3],
    shadow_size: f32,
    k: usize,
    c: Vec3,
    r: f32,
) -> bool {
    if shadow_size <= 0.0 || k == 0 {
        return false;
    }
    let fine = &cascades[k - 1];
    if !crate::gpu_cull::in_cascade(fine, c, r) || !casts_into(&planes[k - 1], c, r) {
        return false;
    }
    let reach = r + MERGE_TEXELS * (2.0 / (cascade_scale(&cascades[k]) * shadow_size));
    let q = *fine * c.extend(1.0);
    let rho = reach * cascade_scale(fine);
    q.x.abs() + rho <= 1.0 && q.y.abs() + rho <= 1.0
}

/// How the map of cascade `k - 1` fills its part of cascade `k`
/// (`MergeParams` in shadow_merge.wgsl).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Merge {
    /// First texel of cascade `k` that is filled, and how many each way:
    /// the texels whose centre is inside the square of the cascade before.
    pub origin: [u32; 2],
    pub size: [u32; 2],
    /// Clip position in the cascade before from the one in `k`: scale x,
    /// y, offset x, y.
    pub to_fine: [f32; 4],
    /// Depth in `k` from the depth in the cascade before: scale, offset;
    /// what the slope of the finer map adds per unit of depth between two
    /// of its texels; the most it adds.
    pub depth: [f32; 4],
}

/// The merge of cascade `k - 1` into cascade `k` (1 or 2), or `None` when
/// the two do not look along the same axes or the part to fill does not
/// fit `scratch` texels a side. `shadow_size`: side of a cascade in texels.
pub fn merge_of(cascades: &[Mat4; 3], k: usize, shadow_size: u32, scratch: u32) -> Option<Merge> {
    let coarse = cascades[k].as_dmat4();
    let fine = cascades[k - 1].as_dmat4();
    if coarse.determinant().abs() < 1e-30 || fine.determinant().abs() < 1e-30 {
        return None;
    }
    // (clip of `k` to clip of `k - 1`, and back)
    let down = fine * coarse.inverse();
    let up = coarse * fine.inverse();
    // the same axes: x to x, y to y, depth to depth, nothing else
    let straight = |m: &glam::DMat4| {
        let off = [m.x_axis.y, m.x_axis.z, m.y_axis.x, m.y_axis.z, m.z_axis.x, m.z_axis.y];
        off.iter().all(|v| v.abs() < 1e-4) && m.x_axis.x > 0.0 && m.y_axis.y > 0.0 && m.z_axis.z > 0.0
    };
    if !straight(&down) || !straight(&up) {
        return None;
    }
    let size = shadow_size as f64;
    // the finer square in the texels of `k`: those whose centre is inside
    let (x0, x1) = (up.w_axis.x - up.x_axis.x, up.w_axis.x + up.x_axis.x);
    let (y0, y1) = (up.w_axis.y - up.y_axis.y, up.w_axis.y + up.y_axis.y);
    let first = |edge: f64| (edge * size - 0.5).ceil().max(0.0);
    let last = |edge: f64| (edge * size - 0.5).floor().min(size - 1.0);
    let (tx0, tx1) = (first(x0 * 0.5 + 0.5), last(x1 * 0.5 + 0.5));
    // (texel rows go down while clip y goes up)
    let (ty0, ty1) = (first(0.5 - y1 * 0.5), last(0.5 - y0 * 0.5));
    if tx1 < tx0 || ty1 < ty0 {
        return None;
    }
    let (w, h) = ((tx1 - tx0 + 1.0) as u32, (ty1 - ty0 + 1.0) as u32);
    if w > scratch || h > scratch {
        return None;
    }
    // a texel of `k` is `ratio` texels of the cascade before
    let ratio = down.x_axis.x;
    let depth_scale = up.z_axis.z;
    // (orthographic: a unit of depth is the same length everywhere)
    let metres_per_depth = 1.0 / coarse.row(2).truncate().length().max(1e-12);
    Some(Merge {
        origin: [tx0 as u32, ty0 as u32],
        size: [w, h],
        to_fine: [
            down.x_axis.x as f32,
            down.y_axis.y as f32,
            down.w_axis.x as f32,
            down.w_axis.y as f32,
        ],
        depth: [
            depth_scale as f32,
            up.w_axis.z as f32,
            (SLOPE_BIAS as f64 * depth_scale * (ratio - 1.0).max(0.0)) as f32,
            (MERGE_BIAS_MAX as f64 / metres_per_depth) as f32,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the caster planes leave out cannot shadow the receivers: no
    /// point of the slice is in the shadow of a rejected sphere, whatever
    /// the light; and casters on the light's side of the slice are kept.
    #[test]
    fn caster_planes_keep_every_shadow() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        let (mut rejected, mut kept) = (0, 0);
        for round in 0..300 {
            let eye = Vec3::new(rand() * 200.0, rand() * 200.0, 20.0 + rand() * 30.0);
            let dir = Vec3::new(rand() - 0.5, rand() - 0.5, (rand() - 0.5) * 0.8).normalize();
            let inv_view = glam::camera::rh::view::look_at_mat4(eye, eye + dir, Vec3::Z).inverse();
            let (tx, ty) = (0.3 + rand(), 0.3 + rand() * 0.6);
            let (near, far) = if round % 3 == 0 {
                (0.1, 8.0)
            } else {
                (5.0 + rand() * 20.0, 30.0 + rand() * 70.0)
            };
            let corners = slice_corners(inv_view, tx, ty, near, far);
            // from straight down to nearly level, sometimes exactly along an axis
            let light = match round % 7 {
                0 => -Vec3::Z,
                1 => Vec3::new(1.0, 0.0, 0.0),
                _ => Vec3::new(rand() - 0.5, rand() - 0.5, -0.02 - rand()).normalize(),
            };
            let planes = caster_planes(&corners, light, 0.0);
            // receivers: random points of the slice
            let points: Vec<Vec3> = (0..40)
                .map(|_| {
                    let d = near + (far - near) * rand();
                    inv_view.transform_point3(Vec3::new((rand() * 2.0 - 1.0) * d * tx, (rand() * 2.0 - 1.0) * d * ty, -d))
                })
                .collect();
            for p in &points {
                assert!(casts_into(&planes, *p, 0.0), "a receiver is inside its own volume");
                // a caster up the light from a receiver shadows it
                let up = *p - light * (1.0 + rand() * 300.0);
                assert!(casts_into(&planes, up, 0.01), "round {round}: caster towards the light rejected");
            }
            for _ in 0..200 {
                let c = eye + Vec3::new(rand() - 0.5, rand() - 0.5, (rand() - 0.5) * 0.5) * 300.0;
                let r = 0.05 + rand() * rand() * 20.0;
                if casts_into(&planes, c, r) {
                    kept += 1;
                    continue;
                }
                rejected += 1;
                for p in &points {
                    // distance from the receiver to the ray the caster's
                    // center sweeps along the light
                    let t = (*p - c).dot(light).max(0.0);
                    let gap = (c + light * t).distance(*p);
                    assert!(gap > r, "round {round}: sphere {c} r {r} shadows {p} (gap {gap})");
                }
            }
        }
        assert!(rejected > 10_000 && kept > 2_000, "{rejected} rejected, {kept} kept");
    }

    /// Every point the view can shade with a cascade is inside that
    /// cascade's planes, the lookup's filter and offsets included; near
    /// casters are out of the far cascades when the pixels choose theirs.
    #[test]
    fn cascade_planes_hold_what_reads_each_cascade() {
        let src = include_str!("shaders/common.wgsl");
        assert!(src.contains(&format!("let band = (end - start) * {BLEND_BAND:?};")));
        let eye = Vec3::new(120.0, 130.0, 25.0);
        let view = ReceiverView {
            inv_view: glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::new(1.0, 0.2, -0.1), Vec3::Z).inverse(),
            tan_x: 1.0,
            tan_y: 0.6,
            near: 0.1,
        };
        let splits = [7.68, 28.8, 96.0];
        let texel = [0.007, 0.025, 0.08];
        let light = Vec3::new(0.3, -0.2, -0.8).normalize();
        for pixel_splits in [false, true] {
            let (near, far) = cascade_reach(splits, pixel_splits);
            let planes = cascade_caster_planes(&view, light, near, far, texel);
            for k in 0..3 {
                for i in 0..2000 {
                    // a pixel: a direction of the view, a distance the cascade is read at
                    let (u, v) = ((i % 41) as f32 / 20.0 - 1.0, (i / 41 % 41) as f32 / 20.0 - 1.0);
                    let dir = Vec3::new(u * view.tan_x, v * view.tan_y, -1.0).normalize();
                    let d = near[k].max(view.near) + (far[k] - near[k].max(view.near)) * (i % 7) as f32 / 6.0;
                    let p = view.inv_view.transform_point3(dir * d);
                    // the lookup's own offsets: six texels and the normal offset
                    let off = Vec3::new(1.0, -1.0, 1.0).normalize() * (6.0 * texel[k] + 0.02 + 0.04 * k as f32);
                    assert!(
                        casts_into(&planes[k], p + off, 0.0),
                        "cascade {k}, pixel_splits {pixel_splits}: {p}"
                    );
                    assert!(casts_into(&planes[k], p - off, 0.0));
                }
            }
            // a caster 3 m ahead of the camera, light straight down: in the
            // first cascade, in the others only while they have no nearest
            // receiver
            let planes = cascade_caster_planes(&view, -Vec3::Z, near, far, texel);
            let caster = eye + Vec3::new(1.0, 0.2, -0.1).normalize() * 3.0;
            assert!(casts_into(&planes[0], caster, 0.2));
            assert_eq!(casts_into(&planes[1], caster, 0.2), !pixel_splits);
            assert_eq!(casts_into(&planes[2], caster, 0.2), !pixel_splits);
        }
    }

    /// Cascades as the renderer builds them: squares of `radius` around
    /// `center`, seen along `light`.
    fn cascades_along(light: Vec3, squares: [(Vec3, f32); 3]) -> [Mat4; 3] {
        squares.map(|(center, radius)| {
            let up = if light.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
            let eye = center - light * (radius + CASCADE_DEPTH_PAD);
            let view = glam::camera::rh::view::look_at_mat4(eye, center, up);
            let depth = radius * 2.0 + 2.0 * CASCADE_DEPTH_PAD;
            glam::camera::rh::proj::directx::orthographic(-radius, radius, -radius, radius, 0.1, depth) * view
        })
    }

    /// The part of a cascade filled from the one before is the finer
    /// square, texel for texel; a point keeps its place and its depth from
    /// one map to the other; and a caster left to the finer cascade only
    /// covers texels of that part.
    #[test]
    fn merge_fills_what_the_left_out_casters_cover() {
        let src = include_str!("shaders/cull.wgsl");
        assert!(src.contains(&format!("const MERGE_TEXELS: f32 = {MERGE_TEXELS:?};")));
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f32 / (1u64 << 53) as f32
        };
        let size = 2048u32;
        let planes = [CASTER_PLANES_ALL; 3];
        let (mut left_out, mut kept) = (0usize, 0usize);
        for round in 0..300 {
            let light = match round % 5 {
                0 => -Vec3::Z,
                1 => Vec3::new(1.0, 0.0, -0.08).normalize(),
                _ => Vec3::new(rand() - 0.5, rand() - 0.5, -0.02 - rand()).normalize(),
            };
            let eye = Vec3::new(rand() * 200.0, rand() * 200.0, 20.0 + rand() * 30.0);
            let dir = Vec3::new(rand() - 0.5, rand() - 0.5, (rand() - 0.5) * 0.6).normalize();
            let far = 48.0 + rand() * 208.0;
            let cascades = cascades_along(
                light,
                [
                    (eye + dir * far * 0.04, far * (0.08 + rand() * 0.03)),
                    (eye + dir * far * 0.19, far * (0.3 + rand() * 0.1)),
                    (eye + dir * far * 0.65, far * (1.0 + rand() * 0.3)),
                ],
            );
            for k in 1..3 {
                let m = merge_of(&cascades, k, size, size / 2).expect("the finer square fits");
                let (coarse, fine) = (cascades[k], cascades[k - 1]);
                // the corners of the part filled are inside the finer
                // square, and the texels just outside it are not
                for (step, inside) in [(0.0, true), (1.0, false)] {
                    for corner in [0.0, 1.0] {
                        let out = step * (2.0 * corner - 1.0);
                        let tx = m.origin[0] as f32 + corner * (m.size[0] - 1) as f32 + out;
                        let ty = m.origin[1] as f32 + corner * (m.size[1] - 1) as f32 + out;
                        let (x, y) = ((tx + 0.5) / size as f32 * 2.0 - 1.0, 1.0 - (ty + 0.5) / size as f32 * 2.0);
                        let (fx, fy) = (x * m.to_fine[0] + m.to_fine[2], y * m.to_fine[1] + m.to_fine[3]);
                        // (to a quarter of a texel of `k`)
                        let ok = if inside {
                            fx.abs() <= 1.001 && fy.abs() <= 1.001
                        } else {
                            fx.abs() > 0.999 || fy.abs() > 0.999
                        };
                        assert!(ok, "round {round}, cascade {k}: texel {tx} {ty} at {fx} {fy}");
                    }
                }
                // a point: the same place and the same depth through the merge
                for _ in 0..20 {
                    let p = fine
                        .inverse()
                        .project_point3(Vec3::new(rand() * 1.8 - 0.9, rand() * 1.8 - 0.9, rand()));
                    let (qc, qf) = (coarse * p.extend(1.0), fine * p.extend(1.0));
                    assert!((qc.x * m.to_fine[0] + m.to_fine[2] - qf.x).abs() < 2e-3, "round {round}");
                    assert!((qc.y * m.to_fine[1] + m.to_fine[3] - qf.y).abs() < 2e-3);
                    assert!((qf.z * m.depth[0] + m.depth[1] - qc.z).abs() < 1e-4);
                }
                assert!(m.depth[2] > 0.0 && m.depth[3] > 0.0);
                // casters: left out only when every texel they cover is filled
                for _ in 0..60 {
                    let across = Vec3::new(rand() - 0.5, rand() - 0.5, rand() - 0.5) * far * 0.8;
                    let c = eye + dir * far * 0.3 * rand() + across * rand();
                    let r = 0.05 + rand() * rand() * 4.0;
                    if !merged_from_finer(&cascades, &planes, size as f32, k, c, r) {
                        kept += 1;
                        continue;
                    }
                    left_out += 1;
                    let q = coarse * c.extend(1.0);
                    let (tx, ty) = ((q.x * 0.5 + 0.5) * size as f32 - 0.5, (0.5 - q.y * 0.5) * size as f32 - 0.5);
                    let reach = r * cascade_scale(&coarse) * size as f32 * 0.5 + 1.0;
                    let (x0, y0) = (m.origin[0] as f32, m.origin[1] as f32);
                    let (x1, y1) = (x0 + m.size[0] as f32 - 1.0, y0 + m.size[1] as f32 - 1.0);
                    // (a caster partly off the map covers no texel there)
                    let max = size as f32 - 1.0;
                    assert!(
                        (tx - reach).max(0.0) >= x0
                            && (tx + reach).min(max) <= x1
                            && (ty - reach).max(0.0) >= y0
                            && (ty + reach).min(max) <= y1,
                        "round {round}, cascade {k}: caster {c} r {r} beyond the part filled"
                    );
                }
            }
            assert!(
                !merged_from_finer(&cascades, &planes, 0.0, 2, eye, 0.1),
                "no merge: nothing is left out"
            );
            assert!(
                !merged_from_finer(&cascades, &planes, size as f32, 0, eye, 0.1),
                "the first cascade draws its casters"
            );
        }
        assert!(left_out > 2_000 && kept > 2_000, "{left_out} left out, {kept} kept");
        // a caster the cascade before does not hold stays
        let cascades = cascades_along(-Vec3::Z, [(Vec3::ZERO, 10.0), (Vec3::ZERO, 40.0), (Vec3::ZERO, 120.0)]);
        assert!(merged_from_finer(&cascades, &planes, 2048.0, 1, Vec3::new(7.0, 0.0, 1.0), 1.0));
        assert!(
            !merged_from_finer(&cascades, &planes, 2048.0, 1, Vec3::new(9.5, 0.0, 1.0), 1.0),
            "at the edge of the square"
        );
        let mut out = planes;
        out[0][0] = [1.0, 0.0, 0.0, -100.0];
        assert!(
            !merged_from_finer(&cascades, &out, 2048.0, 1, Vec3::new(7.0, 0.0, 1.0), 1.0),
            "not a caster of the cascade before"
        );
        assert!(merged_from_finer(&cascades, &out, 2048.0, 2, Vec3::new(7.0, 0.0, 1.0), 1.0));
        // squares turned differently cannot be merged, nor one too large for the scratch
        let mut turned = cascades;
        turned[1] *= Mat4::from_rotation_z(0.3);
        assert_eq!(merge_of(&turned, 1, 2048, 1024), None);
        assert_eq!(merge_of(&cascades, 1, 2048, 256), None);
        assert_eq!(merge_of(&cascades, 1, 2048, 1024).map(|m| m.size), Some([512, 512]));
    }

    /// The margin moves the planes outwards; without a light, or with flat
    /// receivers, every caster is kept.
    #[test]
    fn caster_planes_margin_and_degenerate_input() {
        let inv_view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z).inverse();
        let corners = slice_corners(inv_view, 0.5, 0.5, 1.0, 10.0);
        // light straight down: a caster under the slice is out, by 1 m here
        let below = Vec3::new(8.0, 0.0, -5.2);
        assert!(!casts_into(&caster_planes(&corners, -Vec3::Z, 0.0), below, 0.2));
        assert!(casts_into(&caster_planes(&corners, -Vec3::Z, 1.0), below, 0.2));
        // above it, however high
        assert!(casts_into(&caster_planes(&corners, -Vec3::Z, 0.0), Vec3::new(8.0, 0.0, 900.0), 0.2));
        // beside it
        assert!(!casts_into(&caster_planes(&corners, -Vec3::Z, 0.0), Vec3::new(8.0, 9.0, 50.0), 0.2));
        assert_eq!(caster_planes(&corners, Vec3::ZERO, 0.0), CASTER_PLANES_ALL);
        assert_eq!(caster_planes(&[Vec3::ONE; 8], -Vec3::Z, 0.0), CASTER_PLANES_ALL);
    }

    /// One axis of `cascade_shadow`'s filter across a straight shadow edge:
    /// texels below `edge` are in shadow, each tap a bilinear compare
    /// (linear between the two texel centres around it).
    fn filtered_edge(p: f32, span: f32, taps: u32, edge: i32) -> f32 {
        let lit = |i: i32| if i < edge { 0.0 } else { 1.0 };
        let step = span / (taps - 1) as f32;
        let center = 0.5 * (taps - 1) as f32;
        (0..taps)
            .map(|k| {
                let s = p + (k as f32 - center) * step - 0.5;
                let i = s.floor();
                let f = s - i;
                lit(i as i32) * (1.0 - f) + lit(i as i32 + 1) * f
            })
            .sum::<f32>()
            / taps as f32
    }

    /// The shadow filter's taps are never more than a texel apart, and its
    /// result rises steadily across a shadow edge: no flat step, which drew
    /// parallel lines along the edges when 4 taps covered up to six texels.
    #[test]
    fn shadow_filter_has_no_steps_across_an_edge() {
        for span in [0.5f32, 1.5, 3.0, 3.01, 4.0, 4.5, 5.0, 6.0] {
            let taps = shadow_taps(span);
            assert!(span / (taps - 1) as f32 <= 1.0 + 1e-4, "span {span}: {taps} taps");
            if span <= 3.0 {
                assert_eq!(taps, 4, "span {span}: the filter of before when it had no gap");
            }
            // across the edge, the result must rise at every sub-texel move
            let mut prev = filtered_edge(-span, span, taps, 0);
            let mut x = -span + 0.05;
            while x < span {
                let v = filtered_edge(x, span, taps, 0);
                assert!(v > prev - 1e-6, "span {span}: {v} after {prev} at {x}");
                if x > -0.5 * span + 0.6 && x < 0.5 * span - 0.6 {
                    assert!(v > prev + 1e-3, "span {span}: flat step at {x} ({v})");
                }
                prev = v;
                x += 0.05;
            }
        }
        // what 4 taps two texels apart did: flat between the ramps
        let flat = (0..20)
            .map(|i| filtered_edge(-1.9 + 0.05 * i as f32, 6.0, 4, 0))
            .collect::<Vec<_>>();
        assert!(flat.windows(2).any(|w| (w[1] - w[0]).abs() < 1e-6));
    }
}
