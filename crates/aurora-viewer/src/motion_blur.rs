//! Motion blur in the viewer, Aurora's own (the Legacy Viewer draws none):
//! the setting of Preferences › Graphics, and the blur the renderer gets
//! for a frame (`aurora_render::MotionBlur`, drawn by the renderer's
//! motion_blur.rs).
//!
//! The strength is a shutter open for a share of a sixtieth of a second,
//! whatever the frame rate: at 50 % the streaks are those of a 180° shutter
//! at 60 frames a second, as long at 30 or 144. A frame's motion only
//! covers that frame's time, so the share of it the blur spreads is the
//! open time over the frame's time.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The time the strength is a share of (s): a frame at 60 frames a second.
const REFERENCE_FRAME: f32 = 1.0 / 60.0;
/// The most of a frame's motion the blur spreads (a very fast frame rate
/// shows small motions, spread over several frames' worth).
const MAX_SCALE: f32 = 16.0;

/// Preferences › Graphics › Motion blur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Off by default.
    pub enabled: bool,
    /// 0..1: the share of a sixtieth of a second the shutter stays open
    /// (0.5, a 180° shutter at 60 frames a second).
    pub strength: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            strength: 0.5,
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        self.strength = if self.strength.is_finite() {
            self.strength.clamp(0.0, 1.0)
        } else {
            Settings::default().strength
        };
    }
}

/// `AURORA_MOTION_BLUR=<0..100>`: the blur at that strength (%) for this
/// launch, whatever the setting says (`0`: none); unset, the setting.
fn forced() -> Option<f32> {
    static FORCED: OnceLock<Option<f32>> = OnceLock::new();
    *FORCED.get_or_init(|| std::env::var("AURORA_MOTION_BLUR").ok().and_then(|v| parse_forced(&v)))
}

fn parse_forced(v: &str) -> Option<f32> {
    v.trim()
        .parse::<f32>()
        .ok()
        .filter(|p| p.is_finite())
        .map(|p| p.clamp(0.0, 100.0) / 100.0)
}

/// The strength the frame draws with: the setting's, or the launch's.
pub fn strength(s: &Settings) -> f32 {
    match forced() {
        Some(f) => f,
        None if s.enabled => s.strength,
        None => 0.0,
    }
}

/// The blur of a frame drawn `dt` seconds after the last one, at
/// `strength` (0..1); None: none, at no cost.
pub fn frame(strength: f32, dt: f32) -> Option<aurora_render::MotionBlur> {
    let strength = if strength.is_finite() { strength.clamp(0.0, 1.0) } else { 0.0 };
    if strength <= 0.0 {
        return None;
    }
    // a stronger blur reaches further and takes more samples
    open_for(
        strength * REFERENCE_FRAME,
        dt,
        0.02 + 0.06 * strength,
        (12.0 + 20.0 * strength).round() as u32,
    )
}

/// The Director's blur: a shutter of `angle` degrees at the film's `fps`,
/// for a frame drawn `dt` seconds after the last one. Past 360° it stays
/// open longer than a frame of the film (up to `SHUTTER_RANGE`'s end).
pub fn shutter(angle: f32, fps: u32, dt: f32) -> Option<aurora_render::MotionBlur> {
    let range = crate::director::SHUTTER_RANGE;
    let angle = if angle.is_finite() {
        angle.clamp(*range.start(), *range.end())
    } else {
        return None;
    };
    let turns = angle / 360.0;
    open_for(
        turns / fps.max(1) as f32,
        dt,
        (0.04 * turns * 2.0).clamp(0.02, 0.25),
        (16.0 + 16.0 * turns).round() as u32,
    )
}

/// The Director's render with a shutter of `angle` degrees at `fps`: the
/// seconds it stays open from each frame's instant, and the sub-frames
/// drawn over that time, more for a longer shutter so that its streaks
/// stay smooth (12 at 180°, 48 at 720°).
pub fn render_shutter(angle: f32, fps: u32) -> (f32, u32) {
    let range = crate::director::SHUTTER_RANGE;
    let angle = if angle.is_finite() {
        angle.clamp(*range.start(), *range.end())
    } else {
        crate::director::DEFAULT_SHUTTER
    };
    let turns = angle / 360.0;
    (turns / fps.max(1) as f32, ((turns * 24.0).ceil() as u32).clamp(4, 48))
}

/// A shutter open `exposure` seconds over a frame drawn `dt` seconds after
/// the last one, its streaks held to `max_share` of the image's height.
fn open_for(exposure: f32, dt: f32, max_share: f32, samples: u32) -> Option<aurora_render::MotionBlur> {
    if !(exposure > 0.0 && dt > 0.0 && dt.is_finite()) {
        return None;
    }
    Some(aurora_render::MotionBlur {
        scale: (exposure / dt).min(MAX_SCALE),
        max_share,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_by_default_and_at_no_strength() {
        assert!(!Settings::default().enabled);
        assert_eq!(frame(0.0, 1.0 / 60.0), None);
        assert_eq!(frame(f32::NAN, 1.0 / 60.0), None);
    }

    #[test]
    fn the_streak_is_the_same_length_at_any_frame_rate() {
        // a motion of one frame times the share it spreads: the same open
        // time whatever the frame's time
        for fps in [30.0f32, 60.0, 144.0] {
            let m = frame(0.5, 1.0 / fps).expect("on");
            let streak_per_second = m.scale / fps;
            assert!((streak_per_second - 0.5 / 60.0).abs() < 1e-6, "{fps}: {}", m.scale);
        }
        assert!((frame(0.5, 1.0 / 60.0).expect("on").scale - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_frame_without_time_draws_none_and_a_tiny_one_is_held() {
        assert_eq!(frame(0.5, 0.0), None);
        assert_eq!(frame(0.5, -1.0), None);
        assert_eq!(frame(0.5, f32::INFINITY), None);
        assert_eq!(frame(1.0, 1e-6).expect("on").scale, MAX_SCALE);
    }

    #[test]
    fn a_stronger_blur_reaches_further_with_more_samples() {
        let (weak, strong) = (frame(0.1, 0.02).expect("on"), frame(1.0, 0.02).expect("on"));
        assert!(strong.max_share > weak.max_share);
        assert!(strong.samples > weak.samples);
        assert_eq!(strong.samples, 32);
        assert_eq!(frame(2.0, 0.02), Some(strong));
    }

    #[test]
    fn the_directors_shutter_follows_the_films_rate_and_goes_past_a_frame() {
        // 180° at 30 frames a second: open a sixtieth of a second
        let m = shutter(180.0, 30, 1.0 / 60.0).expect("on");
        assert!((m.scale - 1.0).abs() < 1e-6, "{}", m.scale);
        // 720°: two frames of the film, a longer streak, more samples
        let wild = shutter(720.0, 30, 1.0 / 60.0).expect("on");
        assert!((wild.scale - 4.0).abs() < 1e-5, "{}", wild.scale);
        assert!(wild.max_share > m.max_share && wild.samples > m.samples);
        assert_eq!(wild.max_share, 0.16);
        assert_eq!(wild.samples, 48);
        // held in its range
        assert_eq!(shutter(5000.0, 30, 1.0 / 60.0), Some(wild));
        assert_eq!(shutter(f32::NAN, 30, 1.0 / 60.0), None);
        assert_eq!(shutter(180.0, 30, 0.0), None);
        // a rate of 0 from a broken file is one frame a second, not a crash
        assert!(shutter(180.0, 0, 1.0 / 60.0).is_some());
    }

    #[test]
    fn the_render_shutter_stays_open_its_angle_and_takes_more_sub_frames_when_longer() {
        let (open, count) = render_shutter(180.0, 30);
        assert!((open - 1.0 / 60.0).abs() < 1e-7);
        assert_eq!(count, 12);
        let (open, count) = render_shutter(720.0, 24);
        assert!((open - 2.0 / 24.0).abs() < 1e-7);
        assert_eq!(count, 48);
        assert_eq!(render_shutter(1.0, 30).1, 4);
        assert_eq!(render_shutter(9000.0, 30), render_shutter(720.0, 30));
        assert_eq!(render_shutter(f32::NAN, 30), render_shutter(180.0, 30));
        assert!(render_shutter(180.0, 0).0.is_finite());
    }

    #[test]
    fn the_settings_strength_is_held_in_range() {
        let mut s = Settings {
            enabled: true,
            strength: 3.0,
        };
        s.sanitize();
        assert_eq!(s.strength, 1.0);
        s.strength = f32::NAN;
        s.sanitize();
        assert_eq!(s.strength, 0.5);
    }

    #[test]
    fn the_launch_strength_is_a_percentage() {
        assert_eq!(parse_forced("50"), Some(0.5));
        assert_eq!(parse_forced(" 250 "), Some(1.0));
        assert_eq!(parse_forced("0"), Some(0.0));
        assert_eq!(parse_forced("on"), None);
        assert_eq!(parse_forced("NaN"), None);
    }
}
