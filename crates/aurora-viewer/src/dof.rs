//! Depth of field in the viewer (LLPipeline::renderDoF, newview/pipeline.cpp,
//! with Firestorm's focus lock and free focus, FIRE-16728, originally LGPL
//! 2.1): the settings of Preferences › Graphics, the point the lens focuses
//! on and its smooth transition, and the lens the renderer gets for a frame
//! (`aurora_render::DepthOfField`).

use serde::{Deserialize, Serialize};

/// Firestorm's focus distance when nothing is focused (renderDoF).
const DEFAULT_DISTANCE: f32 = 16.0;

/// Where the lens focuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusMode {
    /// The camera's focus point (Firestorm's default: the alt-zoom focus).
    Camera,
    /// The point under the pointer (FSFocusPointFollowsPointer); the
    /// crosshair in mouselook.
    Pointer,
    /// The point last clicked in the world.
    Click,
    /// The point focused when it was locked (FSFocusPointLocked).
    Locked,
}

/// Preferences › Graphics › Depth of field (RenderDepthOfField and the
/// Camera* settings, Firestorm's defaults).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// RenderDepthOfField (off).
    pub enabled: bool,
    /// RenderDepthOfFieldInEditMode (off).
    pub in_edit: bool,
    pub focus: FocusMode,
    /// CameraFNumber (9, 1..64).
    pub f_number: f32,
    /// CameraFocalLength (50 mm, 1..1000) at CameraFieldOfView.
    pub focal_length: f32,
    /// CameraFieldOfView (60°, 10..200): the field of view the focal length
    /// is given for; a wider view is a shorter lens.
    pub reference_fov: f32,
    /// CameraMaxCoF (10 pixels at the window's height).
    pub max_coc: f32,
    /// CameraFocusTransitionTime (0.5 s, 0.1..10).
    pub transition: f32,
    /// CameraDoFResScale (0.7, 0.25..1): here the number of samples of the
    /// blur, which is always drawn at half resolution.
    pub quality: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            in_edit: false,
            focus: FocusMode::Camera,
            f_number: 9.0,
            focal_length: 50.0,
            reference_fov: 60.0,
            max_coc: 10.0,
            transition: 0.5,
            quality: 0.7,
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        let fix = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        let d = Settings::default();
        self.f_number = fix(self.f_number, 1.0, 64.0, d.f_number);
        self.focal_length = fix(self.focal_length, 1.0, 1000.0, d.focal_length);
        self.reference_fov = fix(self.reference_fov, 10.0, 200.0, d.reference_fov);
        self.max_coc = fix(self.max_coc, 1.0, 64.0, d.max_coc);
        self.transition = fix(self.transition, 0.1, 10.0, d.transition);
        self.quality = fix(self.quality, 0.25, 1.0, d.quality);
    }

    /// Samples of the blur's gather for the quality.
    pub fn samples(&self) -> u32 {
        (16.0 + 64.0 * self.quality.clamp(0.25, 1.0)).round() as u32
    }
}

/// Where the photo window's lens focuses (the mockup's Click / Pointer /
/// Manual).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhotoFocus {
    /// The point last clicked in the world.
    Click,
    /// The point under the pointer.
    Pointer,
    /// A distance set by hand.
    Manual,
}

/// The photo window's depth of field: its own aperture and focus, applied
/// to the view only while the window is open, so the preview, the photos
/// and the view agree. Its switch off ("All sharp") is sharp: the
/// Preferences' blur does not come back under it (#1162). The Preferences'
/// setting is never changed: the window closed, the view follows it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoDof {
    pub enabled: bool,
    /// f-number (f/2.8, a photographer's portrait aperture).
    pub f_number: f32,
    pub focus: PhotoFocus,
    /// Manual focus distance (m).
    pub distance: f32,
}

impl Default for PhotoDof {
    fn default() -> Self {
        PhotoDof {
            // on: the blur that follows the focus is what makes the photo
            // mode a camera (the Preferences' own stays off)
            enabled: true,
            f_number: 2.8,
            focus: PhotoFocus::Click,
            distance: 5.0,
        }
    }
}

impl PhotoDof {
    pub fn sanitize(&mut self) {
        let d = PhotoDof::default();
        self.f_number = if self.f_number.is_finite() {
            self.f_number.clamp(1.0, 22.0)
        } else {
            d.f_number
        };
        self.distance = if self.distance.is_finite() {
            self.distance.clamp(0.3, 200.0)
        } else {
            d.distance
        };
    }

    /// The Preferences' lens with the photo's aperture and focus: on or off
    /// as the window's switch says, also while editing (the window says
    /// so), and a manual distance when asked.
    pub fn applied(&self, global: &Settings) -> (Settings, Option<f32>) {
        let s = Settings {
            enabled: self.enabled,
            in_edit: true,
            f_number: self.f_number,
            focus: match self.focus {
                PhotoFocus::Click => FocusMode::Click,
                PhotoFocus::Pointer => FocusMode::Pointer,
                PhotoFocus::Manual => FocusMode::Locked,
            },
            ..global.clone()
        };
        (s, (self.focus == PhotoFocus::Manual).then_some(self.distance))
    }
}

/// The lens's focal length (mm) at the vertical field of view `fov_y`
/// (radians): renderDoF keeps the image's height of the reference lens
/// (dv = 2 f tan(fov0 / 2)), so a wider view is a shorter lens.
pub fn focal_length_at(s: &Settings, fov_y: f32) -> f32 {
    let reference = (s.reference_fov.to_radians() * 0.5).tan();
    s.focal_length * reference / (fov_y * 0.5).tan().max(1e-4)
}

/// The circle of confusion, in pixels of an image `height` pixels high,
/// for (s - z) / z (cofF.glsl's calc_cof with renderDoF's uniforms: the blur
/// disk f / N |s - z| / z at the focus distance, over a pixel's size there,
/// times the shader's 1.414).
pub fn coc_scale(focal_mm: f32, f_number: f32, focus: f32, fov_y: f32, height: u32) -> f32 {
    let pixel_angle = (fov_y / height.max(1) as f32).tan();
    (focal_mm / 1000.0) / f_number.max(0.1) / (focus.max(0.01) * pixel_angle.max(1e-7)) * 1.414
}

/// The circle at distance `z` (pixels, positive in front of the focus),
/// as the shader works it out.
#[cfg(test)]
pub fn coc(scale: f32, focus: f32, z: f32, max_coc: f32) -> f32 {
    (scale * (focus - z) / z.max(1e-3)).clamp(-max_coc, max_coc)
}

/// The circle at `z` with a zone kept sharp from `focus` to `far` (the
/// group focus), as dof.wgsl's `coc` works it out.
#[cfg(test)]
pub fn coc_in_zone(scale: f32, focus: f32, far: f32, z: f32, max_coc: f32) -> f32 {
    let z = z.max(1e-3);
    let s = z.clamp(focus, far.max(focus));
    (scale * focus * (s - z) / (z * s)).clamp(-max_coc, max_coc)
}

/// The focus distance moving to its target (renderDoF: a change of more
/// than 1 % starts a cosine transition of CameraFocusTransitionTime).
#[derive(Debug, Clone)]
pub struct Focus {
    current: f32,
    start: f32,
    /// 0..=1 through a transition, 1 at rest.
    t: f32,
    /// The point clicked or locked (Click, Locked).
    pub point: Option<glam::Vec3>,
}

impl Default for Focus {
    fn default() -> Self {
        Focus {
            current: DEFAULT_DISTANCE,
            start: DEFAULT_DISTANCE,
            t: 1.0,
            point: None,
        }
    }
}

impl Focus {
    #[cfg(test)]
    pub fn distance(&self) -> f32 {
        self.current
    }

    /// One frame of `dt` seconds towards `target` (m).
    pub fn update(&mut self, target: f32, dt: f32, transition: f32) -> f32 {
        let target = if target.is_finite() && target > 0.01 {
            target
        } else {
            DEFAULT_DISTANCE
        };
        if self.t >= 1.0 && (self.current - target).abs() / self.current.max(1e-3) > 0.01 {
            self.t = 0.0;
            self.start = self.current;
        } else if self.t < 1.0 {
            self.t = (self.t + dt / transition.max(0.01)).min(1.0);
            let k = (self.t * std::f32::consts::PI + std::f32::consts::PI).cos() * 0.5 + 0.5;
            self.current = self.start + (target - self.start) * k;
        } else {
            self.current = target;
        }
        self.current
    }

    /// Jump to `distance` (a photo, a demo), without transition.
    pub fn set(&mut self, distance: f32) {
        self.current = distance;
        self.start = distance;
        self.t = 1.0;
    }
}

/// The distance of `point` along the camera's view axis
/// (LLViewerCamera::getAtAxis * (focus_point - eye)).
pub fn distance_along(eye: glam::Vec3, at: glam::Vec3, point: glam::Vec3) -> f32 {
    at.normalize_or_zero().dot(point - eye)
}

/// The lens of a frame.
pub fn lens(s: &Settings, focus: f32, fov_y: f32, height: u32, window_height: u32, debug: bool) -> aurora_render::DepthOfField {
    let focal = focal_length_at(s, fov_y);
    aurora_render::DepthOfField {
        coc_scale: coc_scale(focal, s.f_number, focus, fov_y, height),
        focus,
        focus_far: focus,
        // FIRE-13989: the same blur in an image of any size
        max_coc: s.max_coc * height as f32 / window_height.max(1) as f32,
        samples: s.samples(),
        debug,
        peaking: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focal_length_follows_the_field_of_view_like_firestorm() {
        let s = Settings::default();
        // at the reference view, the reference lens
        assert!((focal_length_at(&s, 60f32.to_radians()) - 50.0).abs() < 1e-3);
        // zooming in to half the tangent doubles the focal length
        let narrow = 2.0 * (30f32.to_radians().tan() * 0.5).atan();
        assert!((focal_length_at(&s, narrow) - 100.0).abs() < 1e-2);
    }

    #[test]
    fn the_circle_grows_with_the_aperture_and_away_from_the_focus() {
        let fov = 60f32.to_radians();
        let k = coc_scale(50.0, 2.8, 5.0, fov, 1080);
        // in focus: no blur; in front: positive, behind: negative
        assert_eq!(coc(k, 5.0, 5.0, 100.0), 0.0);
        assert!(coc(k, 5.0, 2.0, 100.0) > 0.0);
        assert!(coc(k, 5.0, 20.0, 100.0) < 0.0);
        // f/1.4 blurs twice as much as f/2.8, f/8 less
        let wide = coc_scale(50.0, 1.4, 5.0, fov, 1080);
        let closed = coc_scale(50.0, 8.0, 5.0, fov, 1080);
        assert!((wide / k - 2.0).abs() < 1e-4);
        assert!(closed < k);
        // a far background tends to f / N over a pixel's size at the focus
        let pixel = (fov / 1080.0).tan() * 5.0;
        let far = coc(k, 5.0, 1e6, 1e9);
        assert!((far.abs() - 0.05 / 2.8 / pixel * 1.414).abs() / far.abs() < 1e-3);
        // limited to the largest circle
        assert_eq!(coc(wide, 5.0, 0.5, 10.0), 10.0);
        // the same blur, in pixels per image height, at any resolution
        let k4 = coc_scale(50.0, 2.8, 5.0, fov, 2160);
        assert!((k4 / k - 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_sharp_zone_blurs_beyond_each_end_like_a_lens_focused_there() {
        let fov = 60f32.to_radians();
        let (near, far) = (3.0, 7.0);
        let k = coc_scale(50.0, 2.8, near, fov, 1080);
        let k_far = coc_scale(50.0, 2.8, far, fov, 1080);
        // a zone of one plane is the plane
        for z in [1.0, 3.0, 5.0, 40.0] {
            assert!((coc_in_zone(k, near, near, z, 100.0) - coc(k, near, z, 100.0)).abs() < 1e-4);
        }
        // sharp all through the zone
        for z in [3.0, 4.2, 6.9, 7.0] {
            assert_eq!(coc_in_zone(k, near, far, z, 100.0), 0.0);
        }
        // in front: as focused at its near end; behind: as at its far end
        assert!((coc_in_zone(k, near, far, 1.5, 100.0) - coc(k, near, 1.5, 100.0)).abs() < 1e-4);
        assert!((coc_in_zone(k, near, far, 30.0, 100.0) - coc(k_far, far, 30.0, 100.0)).abs() < 1e-4);
        assert!(coc_in_zone(k, near, far, 30.0, 100.0) < 0.0);
    }

    #[test]
    fn the_largest_circle_scales_with_the_image() {
        let s = Settings::default();
        let fov = 60f32.to_radians();
        assert!((lens(&s, 5.0, fov, 1080, 1080, false).max_coc - 10.0).abs() < 1e-4);
        assert!((lens(&s, 5.0, fov, 4320, 1080, false).max_coc - 40.0).abs() < 1e-4);
        assert_eq!(lens(&s, 5.0, fov, 1080, 1080, false).samples, 61);
    }

    #[test]
    fn the_focus_moves_smoothly_and_snaps_on_small_changes() {
        let mut f = Focus::default();
        f.set(4.0);
        // a small change snaps
        assert_eq!(f.update(4.02, 0.016, 0.5), 4.02);
        // a large one starts a transition: first frame stays, then eases
        assert_eq!(f.update(10.0, 0.016, 0.5), 4.02);
        let mid = (0..15).map(|_| f.update(10.0, 0.016, 0.5)).last().unwrap();
        assert!(mid > 4.02 && mid < 10.0);
        for _ in 0..40 {
            f.update(10.0, 0.016, 0.5);
        }
        assert!((f.distance() - 10.0).abs() < 1e-4);
        // nothing focused: Firestorm's 16 m
        let mut g = Focus::default();
        g.set(16.0);
        assert_eq!(g.update(f32::NAN, 0.016, 0.5), 16.0);
    }

    #[test]
    fn distances_are_along_the_view_axis() {
        let eye = glam::Vec3::ZERO;
        let at = glam::Vec3::new(2.0, 0.0, 0.0);
        assert!((distance_along(eye, at, glam::Vec3::new(5.0, 3.0, 1.0)) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn the_photo_lens_overrides_the_view_without_changing_the_preferences() {
        let global = Settings::default();
        let photo = PhotoDof {
            enabled: true,
            f_number: 1.4,
            focus: PhotoFocus::Manual,
            distance: 3.0,
        };
        let (s, distance) = photo.applied(&global);
        assert!(s.enabled && s.in_edit);
        assert_eq!((s.f_number, distance), (1.4, Some(3.0)));
        // the rest of the lens is the Preferences'
        assert_eq!(
            (s.focal_length, s.max_coc, s.quality),
            (global.focal_length, global.max_coc, global.quality)
        );
        assert!(!global.enabled);
        // the window at All sharp keeps the view sharp, even with the
        // Preferences' own on: one depth of field, never two (#1162)
        let prefs_on = Settings {
            enabled: true,
            ..Settings::default()
        };
        let sharp = PhotoDof {
            enabled: false,
            ..photo.clone()
        };
        assert!(!sharp.applied(&prefs_on).0.enabled);
        assert!(photo.applied(&prefs_on).0.enabled);
        let click = PhotoDof {
            focus: PhotoFocus::Click,
            ..photo.clone()
        };
        assert_eq!(
            click.applied(&global),
            (
                Settings {
                    focus: FocusMode::Click,
                    ..s.clone()
                },
                None
            )
        );
        let mut wild = PhotoDof {
            f_number: 99.0,
            distance: f32::NAN,
            ..photo
        };
        wild.sanitize();
        assert_eq!((wild.f_number, wild.distance), (22.0, 5.0));
    }

    #[test]
    fn loaded_settings_are_brought_back_into_range() {
        let mut s = Settings {
            f_number: 0.0,
            quality: f32::NAN,
            transition: 100.0,
            ..Default::default()
        };
        s.sanitize();
        assert_eq!((s.f_number, s.quality, s.transition), (1.0, 0.7, 10.0));
    }
}
