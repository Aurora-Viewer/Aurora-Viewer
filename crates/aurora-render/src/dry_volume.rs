//! Dry volumes: invisible "occlude water" prims the camera stands in below
//! the water line (an underwater room, a "reverse aquarium").
//!
//! Aurora's own (the Legacy Viewer decides it is under water from the
//! camera's height alone, LLViewerCamera::cameraUnderWater). Inside a dry
//! volume the view is drawn as in air; what lies outside the volume keeps
//! the water's fog for the part of the line of sight that is in the water.
//! The shaders mirror `span` (common.wgsl, `dry_span` and `dry_exit`).

use glam::{Mat4, Quat, Vec3, Vec4};

/// Most dry volumes sent to the shaders in one frame.
pub const MAX_DRY_VOLUMES: usize = 8;

/// The prim's shape, as far as the test goes: hollows, cuts, twists and
/// tapers are ignored (the volume is the solid shape around them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DryShape {
    Box,
    /// Round in the prim's local X/Y, straight along Z.
    Cylinder,
    Sphere,
}

impl DryShape {
    fn code(self) -> f32 {
        match self {
            DryShape::Box => 0.0,
            DryShape::Cylinder => 1.0,
            DryShape::Sphere => 2.0,
        }
    }
}

/// One volume in render space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DryVolume {
    /// Render space to the unit shape (-1..1 on each axis): the first three
    /// rows of the inverse of the prim's transform scaled to half its size.
    pub rows: [Vec4; 3],
    pub shape: DryShape,
}

impl DryVolume {
    /// A prim at `pos`, turned by `rot`, of full size `scale`. None for a
    /// flat or degenerate prim (it encloses nothing).
    pub fn new(pos: Vec3, rot: Quat, scale: Vec3, shape: DryShape) -> Option<Self> {
        let half = scale * 0.5;
        // (NaN fails the first test too)
        if !half.cmpgt(Vec3::splat(0.001)).all() || !pos.is_finite() || !rot.is_finite() {
            return None;
        }
        let inv = Mat4::from_scale_rotation_translation(half, rot.normalize(), pos).inverse();
        let rows = [inv.row(0), inv.row(1), inv.row(2)];
        rows.iter().all(|r| r.is_finite()).then_some(Self { rows, shape })
    }

    fn local(&self, p: Vec3) -> Vec3 {
        let p = p.extend(1.0);
        Vec3::new(self.rows[0].dot(p), self.rows[1].dot(p), self.rows[2].dot(p))
    }

    fn local_dir(&self, d: Vec3) -> Vec3 {
        let d = d.extend(0.0);
        Vec3::new(self.rows[0].dot(d), self.rows[1].dot(d), self.rows[2].dot(d))
    }

    pub fn contains(&self, p: Vec3) -> bool {
        let l = self.local(p);
        match self.shape {
            DryShape::Box => l.abs().max_element() <= 1.0,
            DryShape::Cylinder => l.z.abs() <= 1.0 && l.x * l.x + l.y * l.y <= 1.0,
            DryShape::Sphere => l.length_squared() <= 1.0,
        }
    }

    /// The part of the segment `origin + t * delta` inside the volume, as
    /// (t in, t out); in > out when the segment misses it.
    pub fn span(&self, origin: Vec3, delta: Vec3) -> (f32, f32) {
        let o = self.local(origin);
        let d = self.local_dir(delta);
        let all = (-1e30, 1e30);
        match self.shape {
            DryShape::Sphere => round(o, d, all),
            DryShape::Cylinder => round(o.with_z(0.0), d.with_z(0.0), slab(o.z, d.z, all)),
            DryShape::Box => slab(o.y, d.y, slab(o.x, d.x, slab(o.z, d.z, all))),
        }
    }

    /// The four vec4 the shaders read (`Frame::dry`).
    pub fn uniform(&self) -> [[f32; 4]; 4] {
        [
            self.rows[0].to_array(),
            self.rows[1].to_array(),
            self.rows[2].to_array(),
            [self.shape.code(), 0.0, 0.0, 0.0],
        ]
    }
}

const MISS: (f32, f32) = (1.0, -1.0);

fn slab(o: f32, d: f32, (lo, hi): (f32, f32)) -> (f32, f32) {
    if d.abs() < 1e-8 {
        return if o.abs() <= 1.0 { (lo, hi) } else { MISS };
    }
    let (a, b) = ((-1.0 - o) / d, (1.0 - o) / d);
    (lo.max(a.min(b)), hi.min(a.max(b)))
}

/// |o + t d| <= 1: a sphere, or a circle when the z parts are zero.
fn round(o: Vec3, d: Vec3, (lo, hi): (f32, f32)) -> (f32, f32) {
    let a = d.length_squared();
    let b = o.dot(d);
    let c = o.length_squared() - 1.0;
    if a < 1e-12 {
        return if c <= 0.0 { (lo, hi) } else { MISS };
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return MISS;
    }
    let s = disc.sqrt();
    (lo.max((-b - s) / a), hi.min((-b + s) / a))
}

/// How far along `delta` (0..1, 1 or more: all of it) the view from
/// `origin` stays dry, going from volume to volume while they touch
/// (`dry_exit` in common.wgsl).
pub fn exit(volumes: &[DryVolume], origin: Vec3, delta: Vec3) -> f32 {
    let mut t = 0.0f32;
    for _ in 0..volumes.len() {
        let next = volumes
            .iter()
            .map(|v| v.span(origin, delta))
            .filter(|&(a, b)| a <= t + 1e-5 && b > t)
            .fold(t, |m, (_, b)| m.max(b));
        if next <= t {
            break;
        }
        t = next;
        if t >= 1.0 {
            break;
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> DryVolume {
        DryVolume::new(Vec3::new(10.0, 10.0, 5.0), Quat::IDENTITY, Vec3::new(8.0, 6.0, 4.0), DryShape::Box).unwrap()
    }

    #[test]
    fn box_contains_its_inside_only() {
        let r = room();
        assert!(r.contains(Vec3::new(10.0, 10.0, 5.0)));
        assert!(r.contains(Vec3::new(13.9, 12.9, 6.9)));
        assert!(!r.contains(Vec3::new(14.1, 10.0, 5.0)));
        assert!(!r.contains(Vec3::new(10.0, 10.0, 7.1)));
    }

    #[test]
    fn rotation_is_followed() {
        let r = DryVolume::new(
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 2.0, 2.0),
            DryShape::Box,
        )
        .unwrap();
        // the long side now lies along Y
        assert!(r.contains(Vec3::new(0.0, 4.5, 0.0)));
        assert!(!r.contains(Vec3::new(4.5, 0.0, 0.0)));
    }

    #[test]
    fn cylinder_and_sphere_are_round() {
        let c = DryVolume::new(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), DryShape::Cylinder).unwrap();
        assert!(c.contains(Vec3::new(0.0, 0.0, 0.95)));
        assert!(!c.contains(Vec3::new(0.8, 0.8, 0.0)));
        let s = DryVolume::new(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0), DryShape::Sphere).unwrap();
        assert!(!s.contains(Vec3::new(0.0, 0.7, 0.75)));
        assert!(s.contains(Vec3::new(0.0, 0.0, 0.99)));
    }

    #[test]
    fn flat_prims_enclose_nothing() {
        assert!(DryVolume::new(Vec3::ZERO, Quat::IDENTITY, Vec3::new(4.0, 4.0, 0.0), DryShape::Box).is_none());
    }

    #[test]
    fn the_view_leaves_the_room_at_its_wall() {
        let r = room();
        let eye = Vec3::new(10.0, 10.0, 5.0);
        // 4 m to the wall at x = 14, looking 20 m along X
        let t = exit(&[r], eye, Vec3::new(20.0, 0.0, 0.0));
        assert!((t - 0.2).abs() < 1e-5, "{t}");
        // a point inside stays dry
        assert!(exit(&[r], eye, Vec3::new(2.0, 0.0, 0.0)) >= 1.0);
    }

    #[test]
    fn touching_rooms_stay_dry_across_the_doorway() {
        let a = room();
        let b = DryVolume::new(Vec3::new(18.0, 10.0, 5.0), Quat::IDENTITY, Vec3::new(8.0, 6.0, 4.0), DryShape::Box).unwrap();
        let eye = Vec3::new(10.0, 10.0, 5.0);
        let t = exit(&[a, b], eye, Vec3::new(20.0, 0.0, 0.0));
        assert!((t - 0.6).abs() < 1e-5, "{t}");
        // a room apart (a gap of water between) does not join
        let c = DryVolume::new(Vec3::new(20.0, 10.0, 5.0), Quat::IDENTITY, Vec3::new(2.0, 6.0, 4.0), DryShape::Box).unwrap();
        let t = exit(&[a, c], eye, Vec3::new(20.0, 0.0, 0.0));
        assert!((t - 0.2).abs() < 1e-5, "{t}");
    }

    #[test]
    fn uniform_rows_map_to_the_unit_box() {
        let r = room();
        let u = r.uniform();
        let p = Vec4::new(14.0, 13.0, 7.0, 1.0);
        let l = Vec3::new(Vec4::from(u[0]).dot(p), Vec4::from(u[1]).dot(p), Vec4::from(u[2]).dot(p));
        assert!((l - Vec3::ONE).abs().max_element() < 1e-5, "{l}");
        assert_eq!(u[3][0], 0.0);
    }
}
