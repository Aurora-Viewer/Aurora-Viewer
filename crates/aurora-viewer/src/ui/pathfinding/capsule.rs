//! Show physics capsule of the Characters window: the selected character's
//! capsule (CHARACTER_LENGTH, CHARACTER_RADIUS, vertical or horizontal) at
//! its live position and rotation. The Legacy Viewer has Havok build and
//! draw it (LLPathingLib::createPhysicsCapsuleRep /
//! renderSimpleShapeCapsuleID, called from LLFloaterPathfindingCharacters);
//! Aurora draws the same shape as a wire outline, without Havok.
use crate::theme::Palette;
use crate::ui::hud::Projector;
use glam::{Quat, Vec3};

const SEGMENTS: usize = 24;

/// One character's capsule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capsule {
    pub center: Vec3,
    pub rotation: Quat,
    /// The whole length, caps included (CHARACTER_LENGTH).
    pub length: f32,
    pub radius: f32,
    /// Lying along the character's forward axis instead of standing.
    pub horizontal: bool,
}

impl Capsule {
    /// The capsule's axis and the half length of its straight part.
    fn axis(&self) -> (Vec3, f32) {
        let local = if self.horizontal { Vec3::X } else { Vec3::Z };
        let half = ((self.length - 2.0 * self.radius) * 0.5).max(0.0);
        (self.rotation * local, half)
    }

    /// The outline as line segments: a ring at each end of the straight
    /// part, four lines between them, and two half circles over each cap.
    pub fn segments(&self) -> Vec<(Vec3, Vec3)> {
        let (axis, half) = self.axis();
        if !axis.is_finite() || !self.radius.is_finite() || self.radius <= 0.0 {
            return Vec::new();
        }
        let u = axis.any_orthonormal_vector();
        let v = axis.cross(u);
        let r = self.radius;
        let ends = [self.center + axis * half, self.center - axis * half];
        let mut out = Vec::with_capacity(SEGMENTS * 4 + 4);
        let point = |end: Vec3, a: f32| end + (u * a.cos() + v * a.sin()) * r;
        for end in ends {
            for i in 0..SEGMENTS {
                let a = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
                let b = std::f32::consts::TAU * (i + 1) as f32 / SEGMENTS as f32;
                out.push((point(end, a), point(end, b)));
            }
        }
        for side in [u, -u, v, -v] {
            out.push((ends[0] + side * r, ends[1] + side * r));
        }
        let half_circle = SEGMENTS / 2;
        for (end, outward) in [(ends[0], axis), (ends[1], -axis)] {
            for across in [u, v] {
                let arc = |a: f32| end + (across * a.cos() + outward * a.sin()) * r;
                for i in 0..half_circle {
                    let a = std::f32::consts::PI * i as f32 / half_circle as f32;
                    let b = std::f32::consts::PI * (i + 1) as f32 / half_circle as f32;
                    out.push((arc(a), arc(b)));
                }
            }
        }
        out
    }
}

/// The selected character's physics capsule.
pub fn draw(ctx: &egui::Context, p: &Palette, proj: &Projector, capsule: &Capsule) {
    let painter = crate::ui::hud::world_painter(ctx);
    draw_outline(ctx, &painter, proj, capsule, p.violet_light);
}

/// A capsule's outline in the world, each line hidden where the scene stands
/// in front of it (the Legacy capsules are drawn with the depth test).
pub fn draw_outline(ctx: &egui::Context, painter: &egui::Painter, proj: &Projector, capsule: &Capsule, color: egui::Color32) {
    for (a, b) in capsule.segments() {
        if let Some(segment) = crate::ui::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
            aurora_render::world_line_at(ctx, a, b);
            painter.line_segment(segment, egui::Stroke::new(1.5, color));
        }
    }
    aurora_render::world_label_at(ctx, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outline_spans_the_whole_length_on_the_right_axis() {
        let standing = Capsule {
            center: Vec3::new(10.0, 10.0, 20.0),
            rotation: Quat::IDENTITY,
            length: 2.0,
            radius: 0.5,
            horizontal: false,
        };
        let points: Vec<Vec3> = standing.segments().into_iter().flat_map(|(a, b)| [a, b]).collect();
        let top = points.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        let bottom = points.iter().map(|p| p.z).fold(f32::MAX, f32::min);
        assert!((top - 21.0).abs() < 1e-4 && (bottom - 19.0).abs() < 1e-4, "{bottom}..{top}");
        assert!(points.iter().all(|p| (p.x - 10.0).abs() <= 0.5 + 1e-4));

        let lying = Capsule {
            horizontal: true,
            ..standing
        };
        let points: Vec<Vec3> = lying.segments().into_iter().flat_map(|(a, b)| [a, b]).collect();
        let front = points.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!((front - 11.0).abs() < 1e-4);

        let ball = Capsule { length: 0.2, ..standing };
        assert!(!ball.segments().is_empty(), "shorter than its diameter: a sphere");
        assert!(Capsule { radius: 0.0, ..standing }.segments().is_empty());
    }
}
