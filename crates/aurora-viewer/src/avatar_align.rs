//! Turning our avatar on purpose: the rules of the Legacy Viewer's Face
//! nearest avatar and Compass window. Port of FSAvatarAlignBase
//! (indra/newview/fsfloateravataralign.cpp, originally LGPL 2.1): which
//! avatar is the nearest, the compass bearings and the zones of a click on
//! the compass. The app applies the turn (`app/avatar_align.rs`), the window
//! draws the compass (`ui/avatar_align.rs`).
//!
//! Bearings are the compass's: degrees from north, clockwise. Aurora's
//! `AgentState::yaw` is the world's angle: radians from east (+X),
//! counter-clockwise.

use glam::{Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use uuid::Uuid;

/// FSAvatarAlignBase::MAX_FACE_DISTANCE: the nearest avatar is looked for
/// within 20 m, in any direction.
pub const MAX_FACE_DISTANCE: f32 = 20.0;

/// The compass bearing of `yaw`, in [0, 360).
pub fn bearing(yaw: f32) -> f32 {
    (90.0 - yaw.to_degrees()).rem_euclid(360.0)
}

/// The yaw of the compass bearing `degrees`, in (-π, π].
pub fn yaw_of_bearing(degrees: f32) -> f32 {
    let yaw = (FRAC_PI_2 - degrees.to_radians()).rem_euclid(TAU);
    if yaw > PI { yaw - TAU } else { yaw }
}

/// ll_round: halves round up.
fn ll_round(x: f32) -> f32 {
    (x + 0.5).floor()
}

/// onClickRotate: `delta` degrees from the current heading, negative to
/// the left.
pub fn rotated(yaw: f32, delta: f32) -> f32 {
    yaw_of_bearing((bearing(yaw) + delta).rem_euclid(360.0))
}

/// onClickNearest ("To nearest cardinal"): the heading rounded to the
/// nearest 45°, intercardinals included.
pub fn nearest_cardinal(yaw: f32) -> f32 {
    yaw_of_bearing((ll_round(bearing(yaw) / 45.0) * 45.0).rem_euclid(360.0))
}

/// faceAvatar: the yaw from `from` towards `to`, on the horizontal; `None`
/// when one stands right above the other.
pub fn yaw_towards(from: Vec3, to: Vec3) -> Option<f32> {
    let d = (to - from).truncate();
    (d.length_squared() > 1e-8).then(|| d.y.atan2(d.x))
}

/// The rotation, relative to its seat, that makes a seated avatar face `yaw`
/// in the world and stay upright (snapAvatarBody keeps the world's Z as up,
/// whatever the seat's tilt). `seat` is the seat's world rotation.
pub fn seat_local_rotation(seat: glam::Quat, yaw: f32) -> glam::Quat {
    (seat.inverse() * glam::Quat::from_rotation_z(yaw)).normalize()
}

/// onClickFaceNearestAvatar: the nearest of `avatars` (id, position) within
/// MAX_FACE_DISTANCE of `me`, in 3D; the first one found wins a tie. The
/// caller leaves out ourselves and the animesh (isSelf, isControlAvatar).
pub fn nearest(me: Vec3, avatars: impl IntoIterator<Item = (Uuid, Vec3)>) -> Option<(Uuid, Vec3)> {
    let mut best: Option<(f32, Uuid, Vec3)> = None;
    for (id, position) in avatars {
        let d = position.distance_squared(me);
        if !d.is_finite() || d > MAX_FACE_DISTANCE * MAX_FACE_DISTANCE {
            continue;
        }
        if best.is_none_or(|(b, _, _)| d < b) {
            best = Some((d, id, position));
        }
    }
    best.map(|(_, id, position)| (id, position))
}

/// Where a point falls on the compass (handleMouseDown, handleHover).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// The inner quarter of the radius: face the nearest avatar.
    Centre,
    /// The ring: one of the 8 octants, its index × 45° being its bearing.
    Octant(u8),
}

impl Zone {
    /// The zone of `offset` from the compass's centre (x to the east, y to
    /// the north, in points) on a compass of `radius`; `None` outside it.
    pub fn at(offset: Vec2, radius: f32) -> Option<Zone> {
        let dist = offset.length();
        if radius <= 0.0 || dist > radius {
            return None;
        }
        if dist < radius * 0.25 {
            return Some(Zone::Centre);
        }
        let degrees = offset.x.atan2(offset.y).to_degrees().rem_euclid(360.0);
        Some(Zone::Octant((ll_round(degrees / 45.0) as u32 % 8) as u8))
    }

    /// The bearing an octant turns to.
    pub fn bearing(self) -> Option<f32> {
        match self {
            Zone::Centre => None,
            Zone::Octant(i) => Some(f32::from(i) * 45.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn bearings_and_yaws_agree_on_the_compass_points() {
        // east is yaw 0, north yaw π/2
        assert!(close(bearing(0.0), 90.0));
        assert!(close(bearing(FRAC_PI_2), 0.0));
        assert!(close(bearing(PI), 270.0));
        assert!(close(bearing(-FRAC_PI_2), 180.0));
        for degrees in [0.0, 1.0, 45.0, 90.0, 179.0, 180.0, 225.0, 359.0] {
            assert!(close(bearing(yaw_of_bearing(degrees)), degrees), "{degrees}");
            let yaw = yaw_of_bearing(degrees);
            assert!(yaw > -PI - 1e-6 && yaw <= PI + 1e-6);
        }
    }

    #[test]
    fn rotations_turn_left_with_negative_degrees() {
        let north = yaw_of_bearing(0.0);
        // -90° from north is west, +90° east, +1° and -1° across north
        assert!(close(bearing(rotated(north, -90.0)), 270.0));
        assert!(close(bearing(rotated(north, 90.0)), 90.0));
        assert!(close(bearing(rotated(north, -1.0)), 359.0));
        assert!(close(bearing(rotated(yaw_of_bearing(359.5), 1.0)), 0.5));
        // a left turn is counter-clockwise seen from above: the yaw grows
        assert!(rotated(north, -10.0) > north);
    }

    #[test]
    fn nearest_cardinal_rounds_to_45_degrees_halves_up() {
        assert!(close(bearing(nearest_cardinal(yaw_of_bearing(10.0))), 0.0));
        assert!(close(bearing(nearest_cardinal(yaw_of_bearing(30.0))), 45.0));
        assert!(close(bearing(nearest_cardinal(yaw_of_bearing(22.5))), 45.0));
        assert!(close(bearing(nearest_cardinal(yaw_of_bearing(350.0))), 0.0));
        assert!(close(bearing(nearest_cardinal(yaw_of_bearing(200.0))), 180.0));
    }

    #[test]
    fn facing_an_avatar_looks_along_the_ground() {
        let me = Vec3::new(10.0, 10.0, 20.0);
        let north_above = Vec3::new(10.0, 15.0, 30.0);
        assert!(close(bearing(yaw_towards(me, north_above).unwrap()), 0.0));
        assert!(close(bearing(yaw_towards(me, Vec3::new(5.0, 10.0, 20.0)).unwrap()), 270.0));
        assert!(yaw_towards(me, me + Vec3::Z * 3.0).is_none());
    }

    #[test]
    fn the_nearest_avatar_is_within_20_metres_in_3d() {
        let me = Vec3::new(100.0, 100.0, 20.0);
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let c = Uuid::from_u128(3);
        // none in range: 19 m up and 10 m away is past 20 m in 3D
        assert_eq!(nearest(me, [(a, me + Vec3::new(10.0, 0.0, 19.0))]), None);
        assert_eq!(nearest(me, [(a, me + Vec3::X * 20.5)]), None);
        // exactly 20 m counts
        assert_eq!(nearest(me, [(a, me + Vec3::X * 20.0)]).map(|n| n.0), Some(a));
        let found = nearest(
            me,
            [
                (a, me + Vec3::X * 12.0),
                (b, me + Vec3::new(0.0, -3.0, 4.0)),
                (c, me + Vec3::Y * 30.0),
            ],
        );
        assert_eq!(found.map(|n| n.0), Some(b));
        // a tie keeps the first found
        assert_eq!(
            nearest(me, [(a, me + Vec3::X * 5.0), (b, me - Vec3::X * 5.0)]).map(|n| n.0),
            Some(a)
        );
        // a lost position never wins
        assert_eq!(nearest(me, [(a, Vec3::NAN)]), None);
    }

    #[test]
    fn compass_zones_follow_the_legacy_hit_test() {
        let r = 100.0;
        assert_eq!(Zone::at(Vec2::new(0.0, 0.0), r), Some(Zone::Centre));
        assert_eq!(Zone::at(Vec2::new(24.0, 0.0), r), Some(Zone::Centre));
        assert_eq!(Zone::at(Vec2::new(0.0, 80.0), r), Some(Zone::Octant(0)));
        assert_eq!(Zone::at(Vec2::new(80.0, 0.0), r), Some(Zone::Octant(2)));
        assert_eq!(Zone::at(Vec2::new(-60.0, -60.0), r), Some(Zone::Octant(5)));
        // a hair west of north rounds back to north
        assert_eq!(Zone::at(Vec2::new(-5.0, 80.0), r), Some(Zone::Octant(0)));
        assert_eq!(Zone::at(Vec2::new(0.0, 101.0), r), None);
        assert_eq!(Zone::Octant(5).bearing(), Some(225.0));
        assert_eq!(Zone::Centre.bearing(), None);
    }
}
