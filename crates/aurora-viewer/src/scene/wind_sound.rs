//! Ambient wind noise while flying: port of audio_update_wind
//! (indra/newview/llvieweraudio.cpp, originally LGPL 2.1, Copyright (C)
//! Linden Research, Inc.), with the wind sampled at our avatar as
//! LLAppViewer::idle does (gWindVec = mWind.getVelocity). The sound itself
//! is `aurora_audio`'s port of LLWindGen, on the Ambient channel.
//!
//! The Legacy Viewer mutes the wind when not flying and, since FIRE-12819,
//! under water; it fades in and out over two seconds.

use aurora_audio::{AudioEngine, WindTarget};
use glam::{Quat, Vec3};

/// AudioLevelWind: share of the region's wind heard, so that the world does
/// not feel desolate when hovering still (a debug setting with no
/// preference in the Legacy Viewer).
const AUDIO_LEVEL_WIND: f32 = 0.5;
/// WIND_SOUND_TRANSITION_TIME (s).
const TRANSITION_SECS: f32 = 2.0;

/// AURORA_DEMO_WIND=<x>,<y>[,fly]: a uniform region wind in the demo (m/s,
/// before the ×2 of LLWind::getVelocity), and with `fly` the avatar arrives
/// flying.
pub fn demo_spec() -> Option<(Vec3, bool)> {
    std::env::var("AURORA_DEMO_WIND").ok().map(|spec| parse_demo_spec(&spec))
}

fn parse_demo_spec(spec: &str) -> (Vec3, bool) {
    let parts = || spec.split(',').map(str::trim);
    let mut numbers = parts().filter_map(|v| v.parse::<f32>().ok().filter(|v| v.is_finite()));
    let x = numbers.next().unwrap_or(3.0);
    let y = numbers.next().unwrap_or(2.0);
    (Vec3::new(x, y, 0.0), parts().any(|p| p.eq_ignore_ascii_case("fly")))
}

/// What the wind sound depends on, this frame.
#[derive(Debug, Clone, Copy)]
pub struct WindInput {
    /// The region's wind at our avatar (m/s).
    pub region_wind: Vec3,
    /// Our avatar's velocity (m/s).
    pub velocity: Vec3,
    /// Our avatar's facing (the agent frame).
    pub rotation: Quat,
    /// Flying, above the water, in the world (not behind a loading or
    /// teleport screen, where the Legacy Viewer mutes all audio).
    pub audible: bool,
}

/// The wind felt by the avatar, in its own frame: the scaled region wind
/// minus the avatar's motion (walking north, the apparent wind blows south).
pub fn relative_wind(input: &WindInput) -> Vec3 {
    let felt = input.region_wind * AUDIO_LEVEL_WIND - input.velocity;
    let local = input.rotation.inverse() * felt;
    if local.is_finite() { local } else { Vec3::ZERO }
}

/// The fade of the wind sound (mMaxWindGain without the volume channels,
/// which the mixer applies).
#[derive(Debug, Default)]
pub struct WindSound {
    fade: f32,
    audible: bool,
}

impl WindSound {
    /// One frame of `dt` seconds.
    pub fn update(&mut self, engine: Option<&AudioEngine>, input: &WindInput, dt: f32) {
        if input.audible != self.audible {
            self.audible = input.audible;
            log::info!(
                "wind sound: {} (region wind {:.1} m/s)",
                if input.audible { "fading in" } else { "fading out" },
                input.region_wind.length()
            );
        }
        let step = if dt.is_finite() { dt.max(0.0) / TRANSITION_SECS } else { 0.0 };
        self.fade = if input.audible {
            (self.fade + step).min(1.0)
        } else {
            (self.fade - step).max(0.0)
        };
        if let Some(engine) = engine {
            engine.set_wind(self.target(input));
        }
    }

    fn target(&self, input: &WindInput) -> WindTarget {
        WindTarget::from_wind(relative_wind(input).to_array(), self.fade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(velocity: Vec3, audible: bool) -> WindInput {
        WindInput {
            region_wind: Vec3::new(4.0, 0.0, 0.0),
            velocity,
            rotation: Quat::IDENTITY,
            audible,
        }
    }

    #[test]
    fn the_wind_felt_is_half_the_region_wind_against_the_motion() {
        // Flying north at 10 m/s in a 4 m/s north wind: 8 m/s from the front.
        let w = relative_wind(&input(Vec3::new(10.0, 0.0, 0.0), true));
        assert!((w - Vec3::new(-8.0, 0.0, 0.0)).length() < 1e-5, "{w}");
        // Facing west (+Y), the same wind comes from the right side.
        let turned = WindInput {
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            ..input(Vec3::ZERO, true)
        };
        let w = relative_wind(&turned);
        assert!((w - Vec3::new(0.0, -2.0, 0.0)).length() < 1e-5, "{w}");
    }

    #[test]
    fn fades_over_two_seconds_and_only_while_audible() {
        let mut s = WindSound::default();
        let flying = input(Vec3::new(10.0, 0.0, 0.0), true);
        s.update(None, &flying, 1.0);
        assert!((s.fade - 0.5).abs() < 1e-6);
        let half = s.target(&flying).gain;
        s.update(None, &flying, 1.5);
        assert_eq!(s.fade, 1.0);
        assert!((s.target(&flying).gain - 2.0 * half).abs() < 1e-5);
        let landed = input(Vec3::new(10.0, 0.0, 0.0), false);
        s.update(None, &landed, 0.5);
        assert!((s.fade - 0.75).abs() < 1e-6);
        s.update(None, &landed, 10.0);
        assert_eq!(s.fade, 0.0);
        assert_eq!(s.target(&landed).gain, 0.0);
        s.update(None, &flying, f32::NAN);
        assert_eq!(s.fade, 0.0);
    }

    #[test]
    fn demo_spec_reads_wind_and_flight() {
        assert_eq!(parse_demo_spec("5, -1, fly"), (Vec3::new(5.0, -1.0, 0.0), true));
        assert_eq!(parse_demo_spec("1"), (Vec3::new(1.0, 2.0, 0.0), false));
        assert_eq!(parse_demo_spec("fly"), (Vec3::new(3.0, 2.0, 0.0), true));
    }
}
