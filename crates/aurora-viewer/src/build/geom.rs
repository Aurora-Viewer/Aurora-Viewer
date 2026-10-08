//! Camera and geometry helpers of the build tools (LLManip / LLViewerCamera
//! math: screen-size scaling, mouse rays, line / plane intersections,
//! grid subdivision levels).

use egui::Pos2;
use glam::{Mat4, Vec3, Vec4};

/// What the manipulators need to know about the camera this frame.
#[derive(Debug, Clone, Copy)]
pub struct Cam {
    pub eye: Vec3,
    pub at: Vec3,
    pub left: Vec3,
    pub up: Vec3,
    pub view_proj: Mat4,
    pub inv_view_proj: Mat4,
    /// Vertical field of view (radians).
    pub fov_y: f32,
    /// Viewport in physical pixels and egui points per pixel.
    pub width: f32,
    pub height: f32,
    pub ppp: f32,
}

impl Default for Cam {
    fn default() -> Self {
        Cam {
            eye: Vec3::ZERO,
            at: Vec3::X,
            left: Vec3::Y,
            up: Vec3::Z,
            view_proj: Mat4::IDENTITY,
            inv_view_proj: Mat4::IDENTITY,
            fov_y: 1.0,
            width: 1.0,
            height: 1.0,
            ppp: 1.0,
        }
    }
}

impl Cam {
    pub fn new(eye: Vec3, target: Vec3, view_proj: Mat4, fov_y: f32, width: f32, height: f32, ppp: f32) -> Cam {
        let at = (target - eye).try_normalize().unwrap_or(Vec3::X);
        let left = Vec3::Z.cross(at).try_normalize().unwrap_or(Vec3::Y);
        let up = at.cross(left);
        Cam {
            eye,
            at,
            left,
            up,
            view_proj,
            inv_view_proj: view_proj.inverse(),
            fov_y,
            width: width.max(1.0),
            height: height.max(1.0),
            ppp: ppp.max(0.1),
        }
    }

    /// LLViewerCamera::getPixelMeterRatio: pixels per meter at 1 m.
    pub fn pixel_meter_ratio(&self) -> f32 {
        self.height / (2.0 * (self.fov_y * 0.5).tan())
    }

    /// World size of `px` screen pixels at `p` (LLManip: range * tan(px / view
    /// height * fov)).
    pub fn meters_for_pixels(&self, p: Vec3, px: f32) -> f32 {
        let range = (p - self.eye).length();
        if range < 0.001 {
            return 1.0;
        }
        range * (px / self.height * self.fov_y).tan()
    }

    /// Distance along the view direction (z depth).
    pub fn depth(&self, p: Vec3) -> f32 {
        (p - self.eye).dot(self.at)
    }

    /// Mouse ray through a cursor position in physical pixels.
    pub fn ray(&self, x: f32, y: f32) -> (Vec3, Vec3) {
        let ndc = |z: f32| Vec4::new(x / self.width * 2.0 - 1.0, 1.0 - y / self.height * 2.0, z, 1.0);
        let un = |v: Vec4| {
            let p = self.inv_view_proj * v;
            p.truncate() / p.w
        };
        // reverse-Z: 1 is the near plane
        let near = un(ndc(1.0));
        let far = un(ndc(0.001));
        let dir = (far - near).try_normalize().unwrap_or(self.at);
        (self.eye, dir)
    }

    /// Clip-space position.
    pub fn clip(&self, p: Vec3) -> Vec4 {
        self.view_proj * p.extend(1.0)
    }

    /// Clip space -> egui points.
    pub fn clip_to_screen(&self, c: Vec4) -> Pos2 {
        let ndc = c.truncate() / c.w;
        Pos2::new(
            (ndc.x * 0.5 + 0.5) * self.width / self.ppp,
            (0.5 - ndc.y * 0.5) * self.height / self.ppp,
        )
    }

    /// World -> egui points (None behind the camera).
    pub fn project(&self, p: Vec3) -> Option<Pos2> {
        let c = self.clip(p);
        (c.w > 0.01).then(|| self.clip_to_screen(c))
    }

    /// World -> physical pixels (None behind the camera).
    pub fn project_px(&self, p: Vec3) -> Option<(f32, f32)> {
        self.project(p).map(|q| (q.x * self.ppp, q.y * self.ppp))
    }
}

/// Intersection of a ray with a plane.
pub fn ray_plane(o: Vec3, d: Vec3, p0: Vec3, n: Vec3) -> Option<Vec3> {
    let den = d.dot(n);
    if den.abs() < 1e-6 {
        return None;
    }
    let t = (p0 - o).dot(n) / den;
    (t > 0.0).then(|| o + d * t)
}

/// LLManip::nearestPointOnLineFromMouse: parameter along the line `a + s (b -
/// a)` of its point nearest to the mouse ray (None when parallel).
pub fn nearest_on_line(o: Vec3, d: Vec3, a: Vec3, b: Vec3) -> Option<f32> {
    let u = b - a;
    let w = o - a;
    let (aa, bb, cc, dd, ee) = (d.dot(d), d.dot(u), u.dot(u), d.dot(w), u.dot(w));
    let den = aa * cc - bb * bb;
    if den.abs() < 1e-9 {
        return None;
    }
    Some((aa * ee - bb * dd) / den)
}

/// LLManip::getSubdivisionLevel: how finely to snap along `axis` at `p` so
/// that a step is at least `min_px` pixels on screen (powers of two, from
/// 1/32 up to `max_sub`).
pub fn subdivision_level(cam: &Cam, p: Vec3, axis: Vec3, grid_scale: f32, min_px: f32, max_sub: f32) -> f32 {
    let d = p - cam.eye;
    let range = d.length();
    if range < 1e-4 {
        return max_sub;
    }
    let dn = d / range;
    let sub = axis.cross(dn).length() * grid_scale / (range / cam.pixel_meter_ratio() * min_px.max(1.0));
    if sub.is_nan() || sub <= 0.0 {
        return 1.0 / 32.0;
    }
    2f32.powf(sub.log2().floor()).clamp(1.0 / 32.0, max_sub)
}

/// Round `v` to the nearest multiple of `unit`.
pub fn round_to(v: f32, unit: f32) -> f32 {
    if unit <= 0.0 { v } else { (v / unit).round() * unit }
}

/// Distance in pixels from point `m` to segment `a`-`b`, and the parameter.
pub fn seg_distance(m: Pos2, a: Pos2, b: Pos2) -> (f32, f32) {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 > 1e-6 {
        ((m - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((a + ab * t).distance(m), t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_point_on_axis() {
        // ray straight down at x = 3 onto the x axis
        let t = nearest_on_line(Vec3::new(3.0, 0.0, 10.0), -Vec3::Z, Vec3::ZERO, Vec3::X).unwrap();
        assert!((t - 3.0).abs() < 1e-5);
    }

    #[test]
    fn subdivision_is_power_of_two() {
        let cam = Cam::new(Vec3::ZERO, Vec3::X, Mat4::IDENTITY, 1.0, 1000.0, 1000.0, 1.0);
        let s = subdivision_level(&cam, Vec3::new(10.0, 0.0, 0.0), Vec3::Y, 0.5, 3.0, 32.0);
        assert_eq!(s.log2().fract(), 0.0);
        assert!(s <= 32.0);
    }
}
