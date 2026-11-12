//! How avatars turn and how our own looks (Preferences › Avatars).
//!
//! - The body's turn thresholds: AvatarRotateThresholdSlow / Fast (60° and
//!   2°, read by LLVOAvatar::updateOrientation, indra/newview/llvoavatar.cpp,
//!   originally LGPL 2.1), and a mouselook threshold, 0 for the Legacy
//!   Viewer's (half the walking one, MOUSELOOK_PELVIS_FOLLOW_FACTOR).
//! - The limits of our own head and eyes when they follow a look-at target:
//!   HEAD_ROTATION_CONSTRAINT (72°) and EYE_ROT_LIMIT_ANGLE (27°) of
//!   llheadrotmotion.cpp, as settings. The other avatars keep the constants.
//!
//! The defaults are the constants used so far, so nothing changes until the
//! user moves a slider. The turn thresholds apply to every avatar drawn, as
//! the Legacy Viewer's settings do; the mouselook one only to our own.
use serde::{Deserialize, Serialize};

/// AvatarRotateThresholdSlow / Fast (degrees).
pub const TURN_SLOW: f32 = 60.0;
pub const TURN_FAST: f32 = 2.0;
/// HEAD_ROTATION_CONSTRAINT and EYE_ROT_LIMIT_ANGLE (degrees).
pub const HEAD_LIMIT: f32 = 72.0;
pub const EYE_LIMIT: f32 = 27.0;
/// The sliders' ranges (degrees).
pub const TURN_RANGE: std::ops::RangeInclusive<f32> = 0.0..=180.0;
pub const HEAD_RANGE: std::ops::RangeInclusive<f32> = 0.0..=90.0;
pub const EYE_RANGE: std::ops::RangeInclusive<f32> = 0.0..=60.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AvatarMotion {
    /// The angle between the body and where it should face at which a
    /// standing or slow avatar turns.
    pub turn_slow: f32,
    /// The same, moving fast.
    pub turn_fast: f32,
    /// In mouselook; 0 keeps the Legacy Viewer's (half of the above).
    pub turn_mouselook: f32,
    /// How far our head turns toward what it looks at.
    pub head_limit: f32,
    /// How far our eyes turn within the head.
    pub eye_limit: f32,
}

impl Default for AvatarMotion {
    fn default() -> Self {
        Self {
            turn_slow: TURN_SLOW,
            turn_fast: TURN_FAST,
            turn_mouselook: 0.0,
            head_limit: HEAD_LIMIT,
            eye_limit: EYE_LIMIT,
        }
    }
}

fn bounded(value: f32, range: &std::ops::RangeInclusive<f32>, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(*range.start(), *range.end())
    } else {
        default
    }
}

impl AvatarMotion {
    /// Values from an edited settings file brought back into their ranges.
    pub fn sanitized(self) -> Self {
        Self {
            turn_slow: bounded(self.turn_slow, &TURN_RANGE, TURN_SLOW),
            turn_fast: bounded(self.turn_fast, &TURN_RANGE, TURN_FAST),
            turn_mouselook: bounded(self.turn_mouselook, &TURN_RANGE, 0.0),
            head_limit: bounded(self.head_limit, &HEAD_RANGE, HEAD_LIMIT),
            eye_limit: bounded(self.eye_limit, &EYE_RANGE, EYE_LIMIT),
        }
    }

    /// The body's turning for `world::body`.
    pub fn turning(&self) -> crate::world::body::Thresholds {
        crate::world::body::Thresholds {
            slow: self.turn_slow,
            fast: self.turn_fast,
            mouselook: self.turn_mouselook,
        }
    }

    /// Our head and eye limits (radians) for the look-at motions.
    pub fn look_limits(&self) -> (f32, f32) {
        (self.head_limit.to_radians(), self.eye_limit.to_radians())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_former_constants_and_old_files_still_load() {
        let d = AvatarMotion::default();
        assert_eq!((d.turn_slow, d.turn_fast, d.turn_mouselook), (60.0, 2.0, 0.0));
        let (head, eye) = d.look_limits();
        assert!((head - 0.8 * std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((eye - 0.3 * std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let old: AvatarMotion = serde_json::from_str("{}").expect("empty");
        assert_eq!(old, d);
        let partial: AvatarMotion = serde_json::from_str(r#"{"turn_slow": 90}"#).expect("partial");
        assert_eq!((partial.turn_slow, partial.turn_fast), (90.0, 2.0));
    }

    #[test]
    fn out_of_range_values_are_brought_back() {
        let s = AvatarMotion {
            turn_slow: f32::NAN,
            turn_fast: -5.0,
            turn_mouselook: 500.0,
            head_limit: 200.0,
            eye_limit: f32::INFINITY,
        }
        .sanitized();
        assert_eq!((s.turn_slow, s.turn_fast, s.turn_mouselook), (60.0, 0.0, 180.0));
        assert_eq!((s.head_limit, s.eye_limit), (90.0, 27.0));
    }
}
