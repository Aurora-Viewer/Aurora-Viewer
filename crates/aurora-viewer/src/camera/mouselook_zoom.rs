//! Mouselook zoom while the right button is held: port of LLToolCompGun's
//! handleRightMouseDown / handleRightMouseUp / handleScrollWheel
//! (indra/newview/lltoolcomp.cpp, originally LGPL 2.1), whose angles live in
//! the `_NACL_MLFovValues` setting.
//!
//! The press swaps the viewing angle for the remembered zoomed one, at once
//! (no easing, as the Legacy Viewer); each wheel click while held widens it
//! by 0.1 rad (towards you) or narrows it (away), within the camera's view
//! limits; the release remembers the zoomed angle and gives the normal one
//! back. The remembered angle starts equal to the normal angle, so the first
//! press does not zoom until the wheel turned once while holding. The normal
//! angle itself (`CameraAngle`) is never written: a session that ends while
//! the button is held keeps its viewing angle.

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

/// The remembered zoomed angle of a settings file: finite and within the
/// widest limits (the window's own apply when it is used).
pub fn sanitize(angle: f32, default: f32) -> f32 {
    if angle.is_finite() {
        angle.clamp(MIN_FIELD_OF_VIEW, MAX_FIELD_OF_VIEW)
    } else {
        default
    }
}

/// The zoom of one held right button.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MouselookZoom {
    /// The zoomed angle while the button is held.
    held: Option<f32>,
}

impl MouselookZoom {
    /// The button goes down: the remembered angle takes over.
    pub fn press(&mut self, remembered: f32, aspect: f32) {
        self.held = Some(remembered.clamp(min_view(aspect), max_view(aspect)));
    }

    /// Wheel clicks while held (SL sign: positive is towards you, a wider
    /// angle). False when the button is not held: the wheel does its usual
    /// mouselook job.
    pub fn scroll(&mut self, clicks: f32, aspect: f32) -> bool {
        let Some(angle) = &mut self.held else {
            return false;
        };
        *angle = (*angle + clicks * STEP).clamp(min_view(aspect), max_view(aspect));
        true
    }

    /// The button goes up: the angle to remember, if it was held.
    pub fn release(&mut self) -> Option<f32> {
        self.held.take()
    }

    /// The angle shown instead of the normal one, while held.
    pub fn angle(&self) -> Option<f32> {
        self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::DEFAULT_FOV;

    const WIDE: f32 = 16.0 / 9.0;

    #[test]
    fn the_first_press_keeps_the_default_angle_until_the_wheel_turns() {
        let mut z = MouselookZoom::default();
        z.press(DEFAULT_FOV, WIDE);
        assert_eq!(z.angle(), Some(DEFAULT_FOV));
        assert!(z.scroll(-3.0, WIDE));
        assert!((z.angle().unwrap() - (DEFAULT_FOV - 0.3)).abs() < 1e-6);
        let remembered = z.release().unwrap();
        assert_eq!(z.angle(), None);
        // pressed again: straight to the remembered zoom
        z.press(remembered, WIDE);
        assert!((z.angle().unwrap() - (DEFAULT_FOV - 0.3)).abs() < 1e-6);
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
        z.press(DEFAULT_FOV, WIDE);
        z.scroll(-100.0, WIDE);
        assert_eq!(z.angle(), Some(MIN_FIELD_OF_VIEW));
        z.scroll(100.0, WIDE);
        assert!((z.angle().unwrap() - MAX_FIELD_OF_VIEW / WIDE).abs() < 1e-6);
        // a narrow window: the horizontal angle keeps its 5°
        assert!((min_view(0.5) - MIN_FIELD_OF_VIEW * 2.0).abs() < 1e-6);
        assert_eq!(max_view(0.5), MAX_FIELD_OF_VIEW);
        // a remembered angle out of this window's limits is brought in
        z.release();
        z.press(3.0, WIDE);
        assert!((z.angle().unwrap() - MAX_FIELD_OF_VIEW / WIDE).abs() < 1e-6);
    }

    #[test]
    fn a_settings_file_gives_a_finite_angle() {
        assert_eq!(sanitize(f32::NAN, DEFAULT_FOV), DEFAULT_FOV);
        assert_eq!(sanitize(0.0, DEFAULT_FOV), MIN_FIELD_OF_VIEW);
        assert_eq!(sanitize(0.5, DEFAULT_FOV), 0.5);
    }
}
