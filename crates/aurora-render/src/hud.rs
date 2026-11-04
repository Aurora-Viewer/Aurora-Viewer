//! Screen-space attachment projection and picking. Port of get_hud_matrices
//! and LLViewerWindow::mousePointHUD (Firestorm, originally LGPL 2.1).
use glam::{Mat4, Vec3, Vec4};

#[derive(Debug, Clone, Copy)]
pub struct HudView {
    /// The window, in pixels: what the matrix maps to.
    pub width: f32,
    pub height: f32,
    /// Where the HUDs are laid out in it (left, top, right, bottom pixels):
    /// the room the bars leave, as the Legacy Viewer draws them in its world
    /// view rectangle (LLViewerWindow::updateWorldViewRect, setup3DViewport,
    /// indra/newview/llviewerwindow.cpp), or the whole window.
    pub area: [f32; 4],
    pub zoom: f32,
    pub min_x: f32,
    pub max_x: f32,
}

impl HudView {
    /// HUDs laid out on the whole window.
    pub fn new(size: [u32; 2], zoom: f32, min_x: f32, max_x: f32) -> Self {
        let (width, height) = (size[0].max(1) as f32, size[1].max(1) as f32);
        Self {
            width,
            height,
            area: [0.0, 0.0, width, height],
            zoom: if zoom.is_finite() { zoom.clamp(0.1, 1.0) } else { 1.0 },
            min_x,
            max_x: max_x.max(min_x + 1.0),
        }
    }

    /// HUDs laid out in `area` (left, top, right, bottom pixels), kept in
    /// the window and at least a pixel wide; the whole window when it is
    /// not a usable rectangle.
    pub fn in_area(mut self, area: [f32; 4]) -> Self {
        if area.iter().all(|v| v.is_finite()) {
            let left = area[0].clamp(0.0, self.width - 1.0);
            let top = area[1].clamp(0.0, self.height - 1.0);
            self.area = [
                left,
                top,
                area[2].clamp(left + 1.0, self.width),
                area[3].clamp(top + 1.0, self.height),
            ];
        }
        self
    }

    /// Width and height of the area, in pixels.
    pub fn area_size(self) -> [f32; 2] {
        [self.area[2] - self.area[0], self.area[3] - self.area[1]]
    }

    /// Pixels per HUD unit: one unit spans the area's height at zoom 1.
    pub fn unit(self) -> f32 {
        self.area_size()[1] * self.zoom
    }

    /// The window pixel of a HUD position (its centre is the area's).
    pub fn pixel(self, p: Vec3) -> [f32; 2] {
        let unit = self.unit();
        [
            (self.area[0] + self.area[2]) * 0.5 - p.y * unit,
            (self.area[1] + self.area[3]) * 0.5 - p.z * unit,
        ]
    }

    /// SL's HUD axes: +X into the screen, +Y left, +Z up. One unit spans
    /// the area's height; attachment anchors use the area's aspect.
    pub fn matrix(self) -> Mat4 {
        let depth = self.max_x - self.min_x;
        let unit = self.unit();
        let center = self.pixel(Vec3::ZERO);
        Mat4::from_cols(
            Vec4::new(0.0, 0.0, -1.0 / depth, 0.0),
            Vec4::new(-2.0 * unit / self.width, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 * unit / self.height, 0.0, 0.0),
            Vec4::new(
                2.0 * center[0] / self.width - 1.0,
                1.0 - 2.0 * center[1] / self.height,
                1.0 + self.min_x / depth,
                1.0,
            ),
        )
    }

    pub fn ray(self, x: f32, y: f32) -> (Vec3, Vec3) {
        let unit = self.unit();
        let center = self.pixel(Vec3::ZERO);
        (Vec3::new(self.min_x, (center[0] - x) / unit, (center[1] - y) / unit), Vec3::X)
    }

    /// The 100% area rectangle projected into the zoomed view, in pixels.
    pub fn screen_bounds(self) -> [f32; 4] {
        let [w, h] = self.area_size();
        let center = self.pixel(Vec3::ZERO);
        let (half_w, half_h) = (w * self.zoom * 0.5, h * self.zoom * 0.5);
        [center[0] - half_w, center[1] - half_h, center[0] + half_w, center[1] + half_h]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_bounds_follow_the_projected_normal_screen_after_resize_and_zoom() {
        for (size, area) in [
            ([1600, 900], None),
            ([900, 1600], None),
            ([3200, 1800], None),
            ([1600, 900], Some([8.0, 52.0, 1592.0, 840.0])),
            ([900, 1600], Some([0.0, 24.0, 900.0, 1570.0])),
        ] {
            for zoom in [0.1, 0.5, 0.8, 1.0] {
                let view = HudView::new(size, zoom, -1.0, 1.0);
                let view = area.map_or(view, |a| view.in_area(a));
                let bounds = view.screen_bounds();
                let [w, h] = view.area_size();
                let aspect = w / h;
                for (point, expected) in [
                    (Vec3::new(0.0, aspect * 0.5, 0.5), [bounds[0], bounds[1]]),
                    (Vec3::new(0.0, -aspect * 0.5, -0.5), [bounds[2], bounds[3]]),
                ] {
                    let clip = view.matrix() * point.extend(1.0);
                    let pixel = [(clip.x + 1.0) * view.width * 0.5, (1.0 - clip.y) * view.height * 0.5];
                    for axis in 0..2 {
                        assert!((pixel[axis] - expected[axis]).abs() < 0.001);
                    }
                }
            }
        }
    }

    #[test]
    fn projection_and_picking_agree_after_resize_and_zoom() {
        for (size, area) in [
            ([1280, 720], None),
            ([720, 1280], None),
            ([2560, 1440], None),
            ([1280, 720], Some([8.0, 52.0, 1272.0, 664.0])),
        ] {
            for zoom in [0.1, 0.5, 1.0] {
                let view = HudView::new(size, zoom, -4.0, 7.0);
                let view = area.map_or(view, |a| view.in_area(a));
                for [x, y] in [[0.0, 0.0], [120.0, 280.0], [size[0] as f32, size[1] as f32]] {
                    let (origin, dir) = view.ray(x, y);
                    let clip = view.matrix() * (origin + dir * 2.0).extend(1.0);
                    assert!((clip.x - (x / view.width * 2.0 - 1.0)).abs() < 1e-4);
                    assert!((clip.y - (1.0 - y / view.height * 2.0)).abs() < 1e-4);
                    assert!((0.0..=1.0).contains(&clip.z));
                    let back = view.pixel(origin);
                    assert!((back[0] - x).abs() < 0.01 && (back[1] - y).abs() < 0.01);
                }
            }
        }
    }

    /// The Legacy Viewer's HUDs fill the world view rectangle, not the
    /// window: one unit spans its height, the centre is its centre.
    #[test]
    fn huds_in_an_area_scale_with_its_height_and_centre_on_it() {
        let full = HudView::new([1600, 900], 1.0, -1.0, 1.0);
        let view = full.in_area([0.0, 60.0, 1600.0, 860.0]);
        assert_eq!(full.unit(), 900.0);
        assert_eq!(view.unit(), 800.0);
        assert_eq!(view.pixel(Vec3::ZERO), [800.0, 460.0]);
        // the top left corner of the area, with its aspect
        let corner = view.pixel(Vec3::new(0.0, 1.0, 0.5));
        assert_eq!(corner, [0.0, 60.0]);
        // a rectangle outside the window keeps the whole window
        let odd = full.in_area([f32::NAN, 0.0, 10.0, 10.0]);
        assert_eq!(odd.area, [0.0, 0.0, 1600.0, 900.0]);
    }
}
