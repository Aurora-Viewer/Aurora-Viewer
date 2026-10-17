//! Mouselook zoom while the right button is held: port of LLToolCompGun's
//! handleRightMouseDown / handleRightMouseUp / handleScrollWheel
//! (indra/newview/lltoolcomp.cpp, originally LGPL 2.1), whose angles live in
//! the `_NACL_MLFovValues` setting.
//!
//! The press swaps the viewing angle for the remembered zoomed one, at once
//! (no easing, as the Legacy Viewer); each wheel click while held widens it
//! by 0.1 rad (towards you) or narrows it (away), within the camera's view
//! limits; the release gives the normal angle back and remembers the zoomed
//! one when the wheel set it. Until the wheel has set a zoom, the press
//! keeps the normal angle exactly: the Legacy Viewer starts the zoomed angle
//! at 60° (`_NACL_MLFovValues` (1.047, 1.047, 0)), which already changes the
//! view of anyone whose viewing angle is not 60°. A press and release
//! without the wheel never changes anything. The normal angle itself
//! (`CameraAngle`) is never written: a session that ends while the button
//! is held keeps its viewing angle.

use std::f32::consts::PI;

/// MIN_FIELD_OF_VIEW (llcamera.h): 5°.
const MIN_FIELD_OF_VIEW: f32 = 5.0 * PI / 180.0;
/// MAX_FIELD_OF_VIEW (llcamera.h): 175°.
const MAX_FIELD_OF_VIEW: f32 = 175.0 * PI / 180.0;
/// Radians per wheel click (handleScrollWheel: `clicks * 0.1f`).
const STEP: f32 = 0.1;

/// LLCamera::getMinView: the narrowest vertical angle, wider in a narrow
/// window so the horizontal angle keeps at least 5°.
pub fn min_view(aspect: f32) -> f32 {
    let aspect = aspect.max(0.01);
    if aspect > 1.0 {
        MIN_FIELD_OF_VIEW
    } else {
        MIN_FIELD_OF_VIEW / aspect
    }
}

/// LLCamera::getMaxView: the widest vertical angle, narrower in a wide
/// window so the horizontal angle stays under 175°.
pub fn max_view(aspect: f32) -> f32 {
    let aspect = aspect.max(0.01);
    if aspect > 1.0 {
        MAX_FIELD_OF_VIEW / aspect
    } else {
        MAX_FIELD_OF_VIEW
    }
}

/// The remembered zoomed angle of a settings file: none, or finite and
/// within the widest limits (the window's own apply when it is used).
pub fn sanitize(angle: Option<f32>) -> Option<f32> {
    angle
        .filter(|a| a.is_finite())
        .map(|a| a.clamp(MIN_FIELD_OF_VIEW, MAX_FIELD_OF_VIEW))
}

/// One hold of the right button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    /// The angle shown instead of the normal one.
    pub angle: f32,
    /// The wheel turned during this hold: the angle is to be remembered.
    pub scrolled: bool,
}

/// The zoom of the right button while it is held.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouselookZoom {
    held: Option<Hold>,
}

impl MouselookZoom {
    /// The button goes down: the remembered zoom takes over, or the normal
    /// angle stays as it is when the wheel never set one.
    pub fn press(&mut self, remembered: Option<f32>, normal: f32, aspect: f32) {
        let angle = remembered.map_or(normal, |a| a.clamp(min_view(aspect), max_view(aspect)));
        self.held = Some(Hold { angle, scrolled: false });
    }

    /// Wheel clicks while held (SL sign: positive is towards you, a wider
    /// angle). False when the button is not held: the wheel does its usual
    /// mouselook job.
    pub fn scroll(&mut self, clicks: f32, aspect: f32) -> bool {
        let Some(hold) = &mut self.held else {
            return false;
        };
        hold.angle = (hold.angle + clicks * STEP).clamp(min_view(aspect), max_view(aspect));
        hold.scrolled = true;
        true
    }

    /// The button goes up: the hold it ends, if any.
    pub fn release(&mut self) -> Option<Hold> {
        self.held.take()
    }

    /// The angle shown instead of the normal one, while held.
    pub fn angle(&self) -> Option<f32> {
        self.held.map(|h| h.angle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::DEFAULT_FOV;

    const WIDE: f32 = 16.0 / 9.0;

    #[test]
    fn a_hold_without_the_wheel_keeps_any_normal_angle() {
        // a viewing angle that is not the default 60°
        let normal = 59.5f32.to_radians();
        let mut z = MouselookZoom::default();
        z.press(None, normal, WIDE);
        assert_eq!(z.angle(), Some(normal));
        let hold = z.release().expect("held");
        assert!(!hold.scrolled, "nothing to remember");
        assert_eq!(z.angle(), None);
        // even in a narrow window, whose limits would move it
        z.press(None, 0.1, 0.5);
        assert_eq!(z.angle(), Some(0.1));
    }

    #[test]
    fn the_wheel_sets_the_zoom_that_the_next_hold_brings_back() {
        let mut z = MouselookZoom::default();
        z.press(None, DEFAULT_FOV, WIDE);
        assert!(z.scroll(-3.0, WIDE));
        assert!((z.angle().unwrap() - (DEFAULT_FOV - 0.3)).abs() < 1e-6);
        let hold = z.release().unwrap();
        assert!(hold.scrolled);
        z.press(Some(hold.angle), DEFAULT_FOV, WIDE);
        assert!((z.angle().unwrap() - (DEFAULT_FOV - 0.3)).abs() < 1e-6);
        // held again without the wheel: the zoom is not remembered anew
        assert!(!z.release().unwrap().scrolled);
    }

    #[test]
    fn the_wheel_only_zooms_while_held() {
        let mut z = MouselookZoom::default();
        assert!(!z.scroll(2.0, WIDE));
        assert_eq!(z.release(), None);
    }

    #[test]
    fn the_zoom_stays_within_the_view_limits() {
        let mut z = MouselookZoom::default();
        z.press(None, DEFAULT_FOV, WIDE);
        z.scroll(-100.0, WIDE);
        assert_eq!(z.angle(), Some(MIN_FIELD_OF_VIEW));
        z.scroll(100.0, WIDE);
        assert!((z.angle().unwrap() - MAX_FIELD_OF_VIEW / WIDE).abs() < 1e-6);
        // a narrow window: the horizontal angle keeps its 5°
        assert!((min_view(0.5) - MIN_FIELD_OF_VIEW * 2.0).abs() < 1e-6);
        assert_eq!(max_view(0.5), MAX_FIELD_OF_VIEW);
        // a remembered angle out of this window's limits is brought in
        z.release();
        z.press(Some(3.0), DEFAULT_FOV, WIDE);
        assert!((z.angle().unwrap() - MAX_FIELD_OF_VIEW / WIDE).abs() < 1e-6);
    }

    #[test]
    fn a_settings_file_gives_none_or_a_finite_angle() {
        assert_eq!(sanitize(None), None);
        assert_eq!(sanitize(Some(f32::NAN)), None);
        assert_eq!(sanitize(Some(0.0)), Some(MIN_FIELD_OF_VIEW));
        assert_eq!(sanitize(Some(0.5)), Some(0.5));
    }
}
