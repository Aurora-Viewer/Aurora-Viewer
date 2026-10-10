//! Screen-space attachment projection and picking. Port of get_hud_matrices
//! and LLViewerWindow::mousePointHUD (Firestorm, originally LGPL 2.1).
use glam::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy)]
pub struct HudView {
    pub width: f32,
    pub height: f32,
    pub zoom: f32,
    pub min_x: f32,
    pub max_x: f32,
}

impl HudView {
    pub fn new(size: [u32; 2], zoom: f32, min_x: f32, max_x: f32) -> Self {
        Self {
            width: size[0].max(1) as f32,
            height: size[1].max(1) as f32,
            zoom: if zoom.is_finite() { zoom.clamp(0.1, 1.0) } else { 1.0 },
            min_x,
            max_x: max_x.max(min_x + 1.0),
        }
    }

    /// SL's HUD axes: +X into the screen, +Y left, +Z up. One unit spans
    /// the viewport height; attachment anchors use the viewport aspect.
    pub fn matrix(self) -> Mat4 {
        let depth = self.max_x - self.min_x;
        Mat4::from_cols(
            Vec4::new(0.0, 0.0, -1.0 / depth, 0.0),
            Vec4::new(-2.0 * self.zoom * self.height / self.width, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 * self.zoom, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0 + self.min_x / depth, 1.0),
        )
    }

    pub fn ray(self, x: f32, y: f32) -> (Vec3, Vec3) {
        (
            Vec3::new(
                self.min_x,
                (self.width * 0.5 - x) / (self.height * self.zoom),
                (self.height * 0.5 - y) / (self.height * self.zoom),
            ),
            Vec3::X,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_and_picking_agree_after_resize_and_zoom() {
        for size in [[1280, 720], [720, 1280], [2560, 1440]] {
            for zoom in [0.1, 0.5, 1.0] {
                let view = HudView::new(size, zoom, -4.0, 7.0);
                for [x, y] in [[0.0, 0.0], [120.0, 280.0], [size[0] as f32, size[1] as f32]] {
                    let (origin, dir) = view.ray(x, y);
                    let clip = view.matrix() * (origin + dir * 2.0).extend(1.0);
                    assert!((clip.x - (x / view.width * 2.0 - 1.0)).abs() < 1e-5);
                    assert!((clip.y - (1.0 - y / view.height * 2.0)).abs() < 1e-5);
                    assert!((0.0..=1.0).contains(&clip.z));
                }
            }
        }
    }
}
