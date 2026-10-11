//! Keep a dragged object visible at the viewport edges. LLManipTranslate::
//! handleHover (indra/newview/llmaniptranslate.cpp) rotates at the outer 5%.
//! Aurora deliberately pans in both screen axes with eased velocity instead,
//! preserving the viewing angle and avoiding an abrupt edge transition.

use super::geom::Cam;
use glam::{Vec2, Vec3};

const MARGIN: f32 = 0.05;
const HALF_LIFE: f32 = 0.08;
const SCREENS_PER_SECOND: f32 = 0.35;
const MAX_SPEED: f32 = 10.0;

#[derive(Debug, Clone, Default)]
pub struct EdgePan {
    velocity: Vec2,
}

fn pressure(pixel: f32, extent: f32) -> f32 {
    let margin = extent * MARGIN;
    if pixel < margin {
        -(1.0 - pixel / margin).clamp(0.0, 1.0)
    } else if pixel > extent - margin {
        (1.0 - (extent - pixel) / margin).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Keep the manipulator inside the visible border while the physical cursor
/// drives continued travel. Otherwise the grab offset can hide the object
/// even though the camera pans under the stationary edge cursor.
pub fn drag_cursor(cam: &Cam, cursor: (f32, f32), start: (f32, f32), grab_offset: (f32, f32)) -> (f32, f32) {
    // A handle already inside the margin must not jump on the first move.
    (
        cursor.0.clamp(
            (cam.width * MARGIN + grab_offset.0).min(start.0),
            (cam.width * (1.0 - MARGIN) + grab_offset.0).max(start.0),
        ),
        cursor.1.clamp(
            (cam.height * MARGIN + grab_offset.1).min(start.1),
            (cam.height * (1.0 - MARGIN) + grab_offset.1).max(start.1),
        ),
    )
}

impl EdgePan {
    pub fn update(&mut self, cam: &Cam, cursor: (f32, f32), point: Vec3, dt: f32, active: bool) -> Vec3 {
        if !active || cam.hud.is_some() || !dt.is_finite() || !Vec2::from(cursor).is_finite() || cam.depth(point) <= 0.01 {
            self.velocity = Vec2::ZERO;
            return Vec3::ZERO;
        }
        let Some((x, y)) = cam.project_px(point) else { return Vec3::ZERO };
        let strongest = |a: f32, b: f32| if a.abs() > b.abs() { a } else { b };
        let target = Vec2::new(
            strongest(pressure(cursor.0, cam.width), pressure(x, cam.width)),
            strongest(pressure(cursor.1, cam.height), pressure(y, cam.height)),
        );
        let dt = dt.clamp(0.0, 0.05);
        let rate = std::f32::consts::LN_2 / HALF_LIFE;
        let decay = (-dt * rate).exp();
        // Integrate the easing exactly: motion is independent of frame rate.
        let travel = target * dt + (self.velocity - target) * ((1.0 - decay) / rate);
        self.velocity = target + (self.velocity - target) * decay;
        if target == Vec2::ZERO && self.velocity.length_squared() < 1e-6 {
            self.velocity = Vec2::ZERO;
            return Vec3::ZERO;
        }
        let meters_per_pixel = cam.depth(point) / cam.pixel_meter_ratio();
        let screen = travel * Vec2::new(cam.width, cam.height) * (SCREENS_PER_SECOND * meters_per_pixel);
        (-cam.left * screen.x - cam.up * screen.y).clamp_length_max(MAX_SPEED * dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Cam {
        let eye = Vec3::new(-10.0, 0.0, 0.0);
        let fov = 60f32.to_radians();
        let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::ZERO, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 1600.0 / 900.0, 0.1);
        Cam::new(eye, Vec3::ZERO, proj * view, fov, 1600.0, 900.0, 1.0)
    }

    #[test]
    fn middle_is_still_and_all_edges_pan_toward_the_cursor() {
        let cam = camera();
        assert_eq!(drag_cursor(&cam, (1600.0, 450.0), (800.0, 450.0), (0.0, 0.0)), (1520.0, 450.0));
        assert_eq!(drag_cursor(&cam, (40.0, 450.0), (40.0, 450.0), (0.0, 0.0)), (40.0, 450.0));
        assert_eq!(drag_cursor(&cam, (1600.0, 450.0), (800.0, 450.0), (-90.0, 0.0)), (1430.0, 450.0));
        assert_eq!(EdgePan::default().update(&cam, (800.0, 450.0), Vec3::ZERO, 0.016, true), Vec3::ZERO);
        for (cursor, axis) in [
            ((0.0, 450.0), cam.left),
            ((1600.0, 450.0), -cam.left),
            ((800.0, 0.0), cam.up),
            ((800.0, 900.0), -cam.up),
        ] {
            let delta = EdgePan::default().update(&cam, cursor, Vec3::ZERO, 0.016, true);
            assert!(delta.dot(axis) > 0.0);
            assert!(delta.cross(axis).length() < 1e-5);
        }
    }

    #[test]
    fn offscreen_object_triggers_pan_and_huds_or_inactive_drags_do_not() {
        let cam = camera();
        let outside = -cam.left * 12.0;
        assert!(EdgePan::default().update(&cam, (800.0, 450.0), outside, 0.016, true).dot(-cam.left) > 0.0);
        let mut pan = EdgePan::default();
        pan.update(&cam, (1600.0, 450.0), Vec3::ZERO, 0.016, true);
        assert_eq!(pan.update(&cam, (1600.0, 450.0), Vec3::ZERO, 0.016, false), Vec3::ZERO);
        let hud = Cam::for_hud(aurora_render::HudView::new([1600, 900], 1.0, -1.0, 1.0), 1.0);
        assert_eq!(pan.update(&hud, (1600.0, 450.0), Vec3::ZERO, 0.016, true), Vec3::ZERO);
    }

    #[test]
    fn starts_and_stops_gradually_and_is_independent_of_frame_rate() {
        let cam = camera();
        let run = |fps: usize| {
            let mut pan = EdgePan::default();
            let mut distance = Vec3::ZERO;
            for _ in 0..fps {
                distance += pan.update(&cam, (1600.0, 450.0), Vec3::ZERO, 1.0 / fps as f32, true);
            }
            distance
        };
        assert!(run(60).abs_diff_eq(run(120), 1e-4));
        let mut pan = EdgePan::default();
        let first = pan.update(&cam, (1600.0, 450.0), Vec3::ZERO, 1.0 / 60.0, true);
        let mut last = first;
        for _ in 0..60 {
            last = pan.update(&cam, (1600.0, 450.0), Vec3::ZERO, 1.0 / 60.0, true);
        }
        assert!(last.length() > first.length() * 5.0);
        let stopping = pan.update(&cam, (800.0, 450.0), Vec3::ZERO, 1.0 / 60.0, true);
        assert!(stopping.length() < last.length());
        for _ in 0..120 {
            last = pan.update(&cam, (800.0, 450.0), Vec3::ZERO, 1.0 / 60.0, true);
        }
        assert_eq!(last, Vec3::ZERO);
    }
}
