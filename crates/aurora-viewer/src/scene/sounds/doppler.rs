//! The Doppler shift of world sounds: a sound coming closer plays higher,
//! one going away lower, like a passing car.
//!
//! The Legacy Viewer leaves it to FMOD's 3D Doppler, at a fixed level of 1
//! (`AUDIO_LEVEL_DOPPLER`, audio_update_volume in
//! indra/newview/llvieweraudio.cpp, passed to `System::set3DSettings` by
//! LLListener_FMODSTUDIO::setDopplerFactor, indra/llaudio/lllistener_fmodstudio.cpp,
//! with a distance factor of 1). The listener's velocity is the agent's,
//! wherever the ear is (`gAgent.getVelocity()`, audio_update_listener); a
//! source's is its parent's when it has one (a child prim: the root; an
//! attachment: the avatar), else its own (LLAudioSourceVO::update,
//! indra/newview/llaudiosourcevo.cpp). A triggered sound keeps a zero
//! velocity (LLAudioSource, indra/llaudio/llaudioengine.cpp).
//!
//! Aurora computes the ratio here, once a frame per playing sound, and the
//! mixer plays the clip at that rate (`AudioEngine::world_rate`).

use glam::Vec3;

/// FMOD's speed of sound (m/s) at the Legacy Viewer's distance factor of 1.
const SPEED_OF_SOUND: f32 = 340.0;

/// A rate is sent again once it moved by this much (about 2 cents): the
/// mixer glides between them.
pub const RATE_RESEND: f32 = 0.001;

/// What the shift of this frame needs besides the sound's own place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Doppler {
    /// Preferences › Sound, "Doppler effect": 1 as in the Legacy Viewer,
    /// 0 off, up to 2 for twice the shift.
    pub factor: f32,
    /// The agent's velocity (m/s, region axes).
    pub listener_velocity: Vec3,
}

impl Default for Doppler {
    fn default() -> Self {
        Self {
            factor: 1.0,
            listener_velocity: Vec3::ZERO,
        }
    }
}

impl Doppler {
    /// The playback rate of a sound at `pos` moving at `velocity`; 1 for a
    /// sound played at the listener (a HUD's).
    pub fn rate(&self, listener: Vec3, pos: Vec3, velocity: Vec3, at_listener: bool) -> f32 {
        if at_listener {
            return 1.0;
        }
        ratio(listener, self.listener_velocity, pos, velocity, self.factor)
    }
}

/// The frequency ratio heard at `listener` from a source at `source`
/// (positions in metres, velocities in m/s): (c + v_listener) /
/// (c - v_source), each velocity taken along the line between them,
/// positive toward the other one, and scaled by `factor` (FMOD's Doppler
/// scale). Kept within [`aurora_audio::MIN_RATE`]..[`aurora_audio::MAX_RATE`]
/// (Aurora's own bound: a velocity glitch, an object warped across the
/// region, never sends a sound out of hearing). 1 when off, when the two
/// stand at the same place, or on a value that is not a number.
pub fn ratio(listener: Vec3, listener_velocity: Vec3, source: Vec3, source_velocity: Vec3, factor: f32) -> f32 {
    if factor.is_nan() || factor <= 0.0 || !listener_velocity.is_finite() || !source_velocity.is_finite() {
        return 1.0;
    }
    // from the source to the listener
    let Some(dir) = (listener - source).try_normalize() else {
        return 1.0;
    };
    let factor = factor.min(2.0);
    let closing_listener = -listener_velocity.dot(dir) * factor;
    let closing_source = source_velocity.dot(dir) * factor;
    let num = (SPEED_OF_SOUND + closing_listener).max(0.0);
    let den = SPEED_OF_SOUND - closing_source;
    if den <= 0.0 {
        // faster than sound toward the listener
        return aurora_audio::MAX_RATE;
    }
    (num / den).clamp(aurora_audio::MIN_RATE, aurora_audio::MAX_RATE)
}

/// The velocity a sound of object `o` moves at: its parent's when it has
/// one, else its own (LLAudioSourceVO::update).
pub fn source_velocity(objects: &crate::world::objects::ObjectStore, idx: usize) -> Vec3 {
    let Some(o) = objects.get(idx) else {
        return Vec3::ZERO;
    };
    objects.parent_of(o).and_then(|p| objects.get(p)).map_or(o.velocity, |p| p.velocity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ratio_rises_coming_closer_falls_going_away_and_stays_bounded() {
        let at = Vec3::new(10.0, 0.0, 0.0);
        let still = Vec3::ZERO;
        let r = |lv: Vec3, sv: Vec3, factor: f32| ratio(Vec3::ZERO, lv, at, sv, factor);
        // nothing moves: unshifted, exactly
        assert_eq!(r(still, still, 1.0), 1.0);
        // a source coming at 34 m/s: 340 / 306; going away: 340 / 374
        assert!((r(still, Vec3::new(-34.0, 0.0, 0.0), 1.0) - 340.0 / 306.0).abs() < 1e-5);
        assert!((r(still, Vec3::new(34.0, 0.0, 0.0), 1.0) - 340.0 / 374.0).abs() < 1e-5);
        // the listener walking toward it: 374 / 340
        assert!((r(Vec3::new(34.0, 0.0, 0.0), still, 1.0) - 374.0 / 340.0).abs() < 1e-5);
        // both moving the same way at the same speed: unshifted
        let v = Vec3::new(20.0, 0.0, 0.0);
        assert!((r(v, v, 1.0) - 1.0).abs() < 1e-6);
        // passing across the line of sight: unshifted
        assert_eq!(r(still, Vec3::new(0.0, 30.0, 0.0), 1.0), 1.0);
        // the factor scales the velocities (FMOD's Doppler scale); 0 is off
        assert!((r(still, Vec3::new(-17.0, 0.0, 0.0), 2.0) - 340.0 / 306.0).abs() < 1e-5);
        assert_eq!(r(still, Vec3::new(-34.0, 0.0, 0.0), 0.0), 1.0);
        // within an octave either way, even faster than sound
        assert_eq!(r(still, Vec3::new(-400.0, 0.0, 0.0), 1.0), aurora_audio::MAX_RATE);
        assert_eq!(r(still, Vec3::new(-300.0, 0.0, 0.0), 1.0), aurora_audio::MAX_RATE);
        assert_eq!(r(Vec3::new(-400.0, 0.0, 0.0), still, 1.0), aurora_audio::MIN_RATE);
        assert_eq!(r(still, Vec3::new(1000.0, 0.0, 0.0), 1.0), aurora_audio::MIN_RATE);
        // broken values and a sound at the ear: unshifted
        assert_eq!(r(still, Vec3::splat(f32::NAN), 1.0), 1.0);
        assert_eq!(r(still, still, f32::NAN), 1.0);
        assert_eq!(ratio(at, still, at, Vec3::new(-34.0, 0.0, 0.0), 1.0), 1.0);
        let d = Doppler {
            factor: 1.0,
            listener_velocity: still,
        };
        assert_eq!(
            d.rate(Vec3::ZERO, at, Vec3::new(-34.0, 0.0, 0.0), true),
            1.0,
            "a HUD sound plays at the ear"
        );
    }
}
