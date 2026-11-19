//! Planet curvature (Aurora's own, off by default): the regions are flat,
//! so open water runs out to eye level. With it, the world is drawn on a
//! planet of radius R: a point at a horizontal distance d from the camera
//! is lowered by the sagitta d² / 2R, and from 10 m up the sea ends at a
//! real horizon ~11 km out, ~0.1° under eye level, with the sky below it.
//!
//! It is a projection, not a change of the world (`apply_curvature` in
//! common.wgsl): the vertices are bent on their way to clip space, and
//! whatever reads a position back from the depth (the shaders'
//! `world_from_depth`, the click picks here) raises it by the same drop.
//! Lighting, shadows, fog, the waterline and the simulator keep the flat
//! region. The drop depends on x and y alone, so the inverse is exact.
//!
//! The bend is centred on the main camera in every pass (reflections,
//! mirrors and probes see the planet it stands on); within the usual draw
//! distance it is millimetres to centimetres, the horizon is where it shows.

use glam::{Vec2, Vec3};

/// The Earth's mean radius (m), the default.
pub const EARTH_RADIUS: f32 = 6_371_000.0;

/// The radii the setting allows (m): from a small moon, whose horizon is
/// still a few kilometres out from a standing avatar, to a gas giant.
pub const RADIUS_MIN: f32 = 100_000.0;
pub const RADIUS_MAX: f32 = 100_000_000.0;

/// The bend of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curvature {
    /// The main camera's horizontal position (region metres).
    pub centre: Vec2,
    /// 1 / 2R; 0 for a flat world.
    pub inv_2r: f32,
}

impl Curvature {
    pub const FLAT: Curvature = Curvature {
        centre: Vec2::ZERO,
        inv_2r: 0.0,
    };

    /// The bend around `camera` for a planet of `radius` metres (clamped to
    /// the allowed radii); None: flat.
    pub fn new(camera: Vec3, radius: Option<f32>) -> Curvature {
        match radius.filter(|r| r.is_finite() && *r > 0.0) {
            Some(r) => Curvature {
                centre: camera.truncate(),
                inv_2r: 0.5 / r.clamp(RADIUS_MIN, RADIUS_MAX),
            },
            None => Curvature::FLAT,
        }
    }

    pub fn is_flat(&self) -> bool {
        self.inv_2r == 0.0
    }

    /// How far a point at `p` (region metres) is drawn under its place.
    pub fn drop_at(&self, p: Vec2) -> f32 {
        (p - self.centre).length_squared() * self.inv_2r
    }

    /// Where a point of the flat region is drawn.
    pub fn bend(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x, p.y, p.z - self.drop_at(p.truncate()))
    }

    /// The point of the flat region drawn at `p` (a position read back
    /// from the depth).
    pub fn unbend(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x, p.y, p.z + self.drop_at(p.truncate()))
    }

    /// `Frame::curvature` and the clouds' (common.wgsl, clouds.wgsl).
    pub fn uniform(&self) -> [f32; 4] {
        [self.centre.x, self.centre.y, self.inv_2r, 0.0]
    }
}

/// How far the horizon is (m) from `height` metres above the surface, on
/// the drawn planet (the tangent to the parabola d² / 2R: √(2Rh)).
pub fn horizon_distance(height: f32, radius: f32) -> f32 {
    (2.0 * radius * height.max(0.0)).sqrt()
}

/// How far under eye level the horizon is (radians) from `height` metres:
/// the slope of the surface where the line of sight touches it, d / R.
pub fn horizon_dip(height: f32, radius: f32) -> f32 {
    (horizon_distance(height, radius) / radius).atan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sea_ends_eleven_kilometres_out_from_ten_metres() {
        let d = horizon_distance(10.0, EARTH_RADIUS);
        assert!((d - 11_288.0).abs() < 5.0, "{d}");
        let dip = horizon_dip(10.0, EARTH_RADIUS).to_degrees();
        assert!((dip - 0.1015).abs() < 0.001, "{dip}");
    }

    #[test]
    fn the_drawn_drop_matches_the_sphere_within_draw_distances() {
        let c = Curvature::new(Vec3::new(128.0, 128.0, 25.0), Some(EARTH_RADIUS));
        for d in [256.0f32, 1024.0, 11_000.0, 40_000.0] {
            let sphere = EARTH_RADIUS as f64 - ((EARTH_RADIUS as f64).powi(2) - (d as f64).powi(2)).sqrt();
            let drawn = c.drop_at(Vec2::new(128.0 + d, 128.0)) as f64;
            assert!((drawn - sphere).abs() < 0.002 + sphere * 1e-5, "{d} m: {drawn} vs {sphere}");
        }
        // the camera's own column is not moved
        assert_eq!(c.drop_at(Vec2::new(128.0, 128.0)), 0.0);
    }

    #[test]
    fn a_point_read_back_from_the_depth_is_the_flat_one() {
        let c = Curvature::new(Vec3::new(40.0, -300.0, 12.0), Some(EARTH_RADIUS));
        for p in [
            Vec3::new(10.0, 20.0, 21.5),
            Vec3::new(9000.0, -4000.0, 20.0),
            Vec3::new(-15_000.0, 0.0, 0.0),
        ] {
            let back = c.unbend(c.bend(p));
            assert!(back.distance(p) < 1e-3 * (1.0 + p.length() / 1000.0), "{p} -> {back}");
        }
    }

    #[test]
    fn off_or_broken_radius_is_flat_and_the_radius_is_clamped() {
        let p = Vec3::new(5000.0, 5000.0, 20.0);
        for r in [None, Some(0.0), Some(-1.0), Some(f32::NAN), Some(f32::INFINITY)] {
            let c = Curvature::new(Vec3::ZERO, r);
            assert!(c.is_flat());
            assert_eq!(c.bend(p), p);
            assert_eq!(c.uniform(), [0.0; 4]);
        }
        assert_eq!(Curvature::new(Vec3::ZERO, Some(1.0)).inv_2r, 0.5 / RADIUS_MIN);
        assert_eq!(Curvature::new(Vec3::ZERO, Some(1e12)).inv_2r, 0.5 / RADIUS_MAX);
    }
}
