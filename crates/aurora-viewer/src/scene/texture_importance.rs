//! How finely the textures of a face are loaded for where it stands from the
//! camera, as the Legacy Viewer sizes them: the face loop of
//! LLViewerTextureList::updateImageDecodePriority
//! (indra/newview/llviewertexturelist.cpp), LLFace::calcPixelArea and
//! LLFace::calcImportanceToCamera (indra/newview/llface.cpp), originally
//! LGPL 2.1.
//!
//! The Legacy Viewer gives a texture the "virtual size" of the faces that
//! use it: the screen area of the circle around the face, measured from its
//! nearest point (distance minus its size), times up to TextureCameraBoost
//! (8) for a face near the camera and in the middle of the view, divided by
//! its repeats squared. Every texture of the face (base color, normal,
//! specular / metallic-roughness, emissive) gets the same size. The discard
//! level then follows from the texels per image over that area
//! (`textures::discard_for`, on its square root).

use glam::Vec3;

use super::CullView;

/// TextureCameraBoost (settings.xml): how much a face important to the
/// camera boosts the resolution of its textures.
const TEXTURE_CAMERA_BOOST: f32 = 8.0;
/// FACE_IMPORTANCE_TO_CAMERA_OVER_DISTANCE: (distance, weight).
const OVER_DISTANCE: [(f32, f32); 4] = [(16.1, 1.0), (32.1, 0.5), (48.1, 0.2), (96.1, 0.05)];
/// FACE_IMPORTANCE_TO_CAMERA_OVER_ANGLE: (cos of the angle to the view
/// direction, weight): 10, 20 and 30 degrees, then the rest of the view.
const OVER_ANGLE: [(f32, f32); 4] = [(0.985, 1.0), (0.94, 0.8), (0.866, 0.64), (0.0, 0.36)];
/// TextureScaleMinAreaFactor and TextureScaleMaxAreaFactor: the bounds of
/// the repeats squared the area is divided by.
const SCALE_MIN_AREA: f32 = 0.0095;
const SCALE_MAX_AREA: f32 = 25.0;
/// LLFace::calcImportanceToCamera ignores the importance while the camera
/// moves faster than this (m/s) or turns faster than this (rad/s).
const FAST_SPEED: f32 = 10.0;
const FAST_TURN: f32 = 1.0;

/// LLFace::calcImportanceToCamera: 0 out of the view or beyond 96 m, else
/// the weight of the distance times that of the angle to the view
/// direction; 0 too while the camera moves or turns fast.
pub fn importance_to_camera(cos_angle: f32, dist: f32, cos_half_fov: f32, camera_fast: bool) -> f32 {
    if camera_fast || cos_angle <= cos_half_fov || dist >= OVER_DISTANCE[3].0 {
        return 0.0;
    }
    let by_distance = OVER_DISTANCE.iter().find(|d| dist <= d.0).unwrap_or(&OVER_DISTANCE[3]).1;
    let by_angle = OVER_ANGLE.iter().find(|a| cos_angle >= a.0).unwrap_or(&OVER_ANGLE[3]).1;
    by_distance * by_angle
}

/// Square root of the virtual size the Legacy Viewer gives the textures of
/// an object of bounding sphere (`c`, `r`), before its repeats
/// ([`per_repeats`]): the circle around it seen from its nearest point
/// (calcPixelArea), boosted by its importance to the camera when it is in
/// view (updateImageDecodePriority). A camera inside the sphere counts as
/// fully important.
pub fn texture_pixels(view: &CullView, c: Vec3, r: f32, in_view: bool) -> f32 {
    if screen_sizing() {
        return view.pixel_size(c, r);
    }
    let to = c - view.eye;
    let centre = to.length();
    let dist = (centre - r).max(0.001);
    let radius_px = (r / dist).atan() * view.pixel_angle;
    let mut area = radius_px * radius_px * std::f32::consts::PI;
    if in_view {
        let importance = if dist < r {
            1.0
        } else {
            let cos_angle = if centre > 1e-6 { to.dot(view.forward) / centre } else { 1.0 };
            importance_to_camera(cos_angle, dist, view.cos_half_fov, view.camera_fast)
        };
        area *= (importance * TEXTURE_CAMERA_BOOST).max(1.0);
    }
    area.sqrt()
}

/// The size of [`texture_pixels`] for a face repeating its texture
/// `repeats` times (the smaller of the texture entry's two scales): the
/// area is divided by the repeats squared, within the Legacy Viewer's
/// bounds. The Legacy Viewer applies the texture entry's scale to every map
/// of the face, glTF transforms included (a TODO there).
pub fn per_repeats(px: f32, repeats: f32) -> f32 {
    if screen_sizing() {
        return px * repeats.clamp(0.1, 16.0);
    }
    px / (repeats * repeats).clamp(SCALE_MIN_AREA, SCALE_MAX_AREA).sqrt()
}

/// `AURORA_TEXTURE_SIZING=screen`: the sizing of before, to compare — the
/// object's diameter on screen times the repeats, no importance boost.
fn screen_sizing() -> bool {
    static SCREEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SCREEN.get_or_init(|| std::env::var("AURORA_TEXTURE_SIZING").is_ok_and(|v| v == "screen"))
}

/// The camera's mean speed and turn rate over the last frames, as
/// LLViewerCamera::updateCameraLocation keeps them (sVelocityStat and
/// sAngularVelocityStat, a mean per second over the recent frames).
#[derive(Debug, Default)]
pub struct CameraMotion {
    last: Option<(Vec3, Vec3)>,
    /// (seconds, metres, radians) of the recent frames.
    frames: std::collections::VecDeque<(f32, f32, f32)>,
}

/// Frames the means are taken over (getPeriodMeanPerSec(sVelocityStat, 50)).
const MOTION_FRAMES: usize = 50;

impl CameraMotion {
    /// Note the camera of this frame (`dt` seconds after the last one);
    /// true while it moves or turns too fast for the importance boost.
    pub fn update(&mut self, position: Vec3, forward: Vec3, dt: f32) -> bool {
        if let Some((p, f)) = self.last
            && dt > 0.0
        {
            let turn = f.normalize_or_zero().dot(forward.normalize_or_zero()).clamp(-1.0, 1.0).acos();
            self.frames.push_back((dt, (position - p).length(), turn));
            while self.frames.len() > MOTION_FRAMES {
                self.frames.pop_front();
            }
        }
        self.last = Some((position, forward));
        self.fast()
    }

    pub fn fast(&self) -> bool {
        let (t, d, a) = self.frames.iter().fold((0.0, 0.0, 0.0), |s, f| (s.0 + f.0, s.1 + f.1, s.2 + f.2));
        t > 0.0 && (d / t > FAST_SPEED || a / t > FAST_TURN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn importance_follows_the_legacy_tables() {
        let half_fov = 0.5f32.cos();
        // near and in the middle: full weight
        assert_eq!(importance_to_camera(1.0, 5.0, half_fov, false), 1.0);
        // 16.1 m is still the first distance step, past it the second
        assert_eq!(importance_to_camera(1.0, 16.1, half_fov, false), 1.0);
        assert_eq!(importance_to_camera(1.0, 20.0, half_fov, false), 0.5);
        assert_eq!(importance_to_camera(1.0, 40.0, half_fov, false), 0.2);
        assert_eq!(importance_to_camera(1.0, 90.0, half_fov, false), 0.05);
        // beyond 96.1 m, out of the view, or a fast camera: nothing
        assert_eq!(importance_to_camera(1.0, 97.0, half_fov, false), 0.0);
        assert_eq!(importance_to_camera(0.5, 5.0, half_fov, false), 0.0);
        assert_eq!(importance_to_camera(1.0, 5.0, half_fov, true), 0.0);
        // angle steps: 10, 20, 30 degrees, then the rest of the view
        assert_eq!(importance_to_camera(15f32.to_radians().cos(), 5.0, half_fov, false), 0.8);
        assert_eq!(importance_to_camera(25f32.to_radians().cos(), 5.0, half_fov, false), 0.64);
        assert!((importance_to_camera(1.0, 20.0, 0.0, false) * TEXTURE_CAMERA_BOOST - 4.0).abs() < 1e-6);
        assert_eq!(
            importance_to_camera(26f32.to_radians().cos(), 5.0, 25f32.to_radians().cos(), false),
            0.0
        );
    }

    fn view() -> CullView {
        let fov = 60f32.to_radians();
        // a wide window: half its horizontal field is 60 degrees
        CullView::new(Mat4::IDENTITY, Vec3::ZERO, 1000.0, fov).with_texture_camera(Vec3::X, 2.0, false)
    }

    #[test]
    fn a_near_central_face_asks_for_more_than_its_screen_size() {
        let v = view();
        let (c, r) = (Vec3::new(5.0, 0.0, 0.0), 0.5);
        let plain = texture_pixels(&v, c, r, false);
        let boosted = texture_pixels(&v, c, r, true);
        // sqrt(8): one and a half discard levels finer
        assert!((boosted / plain - 8f32.sqrt()).abs() < 1e-3);
        // seen from its nearest point: larger than the centre's diameter
        assert!(plain > v.pixel_size(c, r));
        // the same object off to the side, beyond 30 degrees: 0.36 x 8
        let side = Vec3::new(5.0, 3.5, 0.0);
        let s = texture_pixels(&v, side, r, true) / texture_pixels(&v, side, r, false);
        assert!((s - (0.36f32 * 8.0).sqrt()).abs() < 1e-3);
        // far away: the screen size alone
        let far = Vec3::new(120.0, 0.0, 0.0);
        assert_eq!(texture_pixels(&v, far, r, true), texture_pixels(&v, far, r, false));
    }

    #[test]
    fn repeats_lower_the_size_within_the_legacy_bounds() {
        assert_eq!(per_repeats(400.0, 1.0), 400.0);
        assert_eq!(per_repeats(400.0, 4.0), 100.0);
        // 25 at most: five repeats and more ask for a fifth
        assert_eq!(per_repeats(400.0, 10.0), 80.0);
        // a texture shown a tenth of a time: about ten times more
        assert!((per_repeats(400.0, 0.01) - 400.0 / 0.0095f32.sqrt()).abs() < 1e-2);
    }

    #[test]
    fn a_fast_camera_drops_the_boost() {
        let mut m = CameraMotion::default();
        assert!(!m.update(Vec3::ZERO, Vec3::X, 1.0 / 60.0));
        assert!(!m.update(Vec3::new(0.05, 0.0, 0.0), Vec3::X, 1.0 / 60.0));
        // 0.5 m a frame at 60 fps: 30 m/s
        for k in 0..10 {
            m.update(Vec3::new(0.05 + 0.5 * (k + 1) as f32, 0.0, 0.0), Vec3::X, 1.0 / 60.0);
        }
        assert!(m.fast());
        // a camera at rest again is slow once the fast frames have gone
        for _ in 0..MOTION_FRAMES {
            m.update(Vec3::new(5.05, 0.0, 0.0), Vec3::X, 1.0 / 60.0);
        }
        assert!(!m.fast());
    }
}
