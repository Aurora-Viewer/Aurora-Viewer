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
    pub const LEGACY: u32 = 0;
    pub const NEW: u32 = TIGHT | DEPTH_ONLY | PIXEL_SPLITS;
    /// The new path with the cascades chosen as before: the image of
    /// `LEGACY` to the pixel.
    pub const VERTEX_SPLITS: u32 = TIGHT | DEPTH_ONLY;
}

/// Paths the session goes through, one per profile period.
/// AURORA_SHADOW_LEGACY: `1` the path before this module; `splits` only
/// its choice of the cascades; `ab` (with AURORA_PROFILE, whose periods
/// drive the cycle) the old path, the new one and `splits` in turn, a
/// second each, to compare them in the same session and under the same
/// outside load.
fn cycle() -> &'static [u32] {
    static CYCLE: OnceLock<Vec<u32>> = OnceLock::new();
    CYCLE.get_or_init(|| match std::env::var("AURORA_SHADOW_LEGACY").as_deref() {
        Ok("ab") if std::env::var_os("AURORA_PROFILE").is_some() => vec![path::LEGACY, path::NEW, path::VERTEX_SPLITS],
        Ok("splits") => vec![path::VERTEX_SPLITS],
        Ok("" | "0" | "ab") | Err(_) => vec![path::NEW],
        Ok(_) => vec![path::LEGACY],
    })
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
/// 4x4 taps at most two texels apart, each a bilinear footprint.
const FILTER_TEXELS: f32 = 6.0;

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
        // (normal offset of the lookup: 0.02 + 0.04 k)
        let margin = FILTER_TEXELS * texel[k] + 0.02 + 0.04 * k as f32 + 0.05;
        caster_planes(&corners, light, margin)
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
}
