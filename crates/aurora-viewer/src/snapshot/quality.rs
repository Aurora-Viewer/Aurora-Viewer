//! Photo quality: while the photo window is open with its "Photo quality"
//! switch on, the graphics settings are raised to at least the Deluxe
//! level (shadows, draw distance, levels of detail, full avatars, ambient
//! occlusion, reflections, anisotropy), the user's own values set aside.
//! Firestorm has no such switch: its Phototools change the real settings
//! for good. Here the user's values are what is saved while the boost is
//! on (`Settings::save`), and they come back when the window closes.

use crate::graphics_presets::{Preset, Values};
use crate::settings::Settings;

/// The user's values while the boost is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    values: Values,
    /// The shadows switch, which the level's values do not carry.
    shadows: bool,
}

impl Held {
    /// Put the user's values back in `s` (the boost's end, or the copy that
    /// is saved).
    pub fn restore(&self, s: &mut Settings) {
        self.values.write(s);
        s.shadows = self.shadows;
    }
}

/// The boosted values: each one at least the Deluxe level's, never lower
/// than the user's. The anti-aliasing, the complexity limit, the particles,
/// the sharpening and the glow stay the user's own choices.
pub fn boosted(user: Values) -> Values {
    let top = Preset::Deluxe.values();
    Values {
        draw_distance: user.draw_distance.max(top.draw_distance),
        lod_factor: user.lod_factor.max(top.lod_factor),
        max_avatars: user.max_avatars.max(top.max_avatars),
        shadow_quality: user.shadow_quality.max(top.shadow_quality),
        projector_shadows: user.projector_shadows || top.projector_shadows,
        ssao: user.ssao.max(top.ssao),
        water_reflections: user.water_reflections.max(top.water_reflections),
        reflection_probes: user.reflection_probes.max(top.reflection_probes),
        probe_count: user.probe_count.max(top.probe_count),
        anisotropy: user.anisotropy.max(top.anisotropy),
        ..user
    }
}

/// Raise `s`, returning what to restore.
pub fn boost(s: &mut Settings) -> Held {
    let held = Held {
        values: Values::read(s),
        shadows: s.shadows,
    };
    boosted(held.values).write(s);
    held
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boost_raises_without_lowering_and_restores_exactly() {
        let mut s = Settings {
            draw_distance: 64.0,
            lod_factor: 4.0,
            shadow_quality: 0,
            shadows: false,
            antialiasing: 3,
            max_complexity: 80_000,
            ..Settings::default()
        };
        let before = s.clone();
        let held = boost(&mut s);
        assert_eq!(s.draw_distance, 256.0);
        // already higher than Deluxe: kept
        assert_eq!(s.lod_factor, 4.0);
        assert_eq!(s.shadow_quality, 4);
        assert!(s.shadows);
        // the user's own choices stay
        assert_eq!((s.antialiasing, s.max_complexity), (3, 80_000));
        held.restore(&mut s);
        assert_eq!(Values::read(&s), Values::read(&before));
        assert_eq!(s.shadows, before.shadows);
    }

    #[test]
    fn a_boosted_profile_saves_the_users_values() {
        let mut s = Settings {
            draw_distance: 96.0,
            ..Settings::default()
        };
        s.photo_held = Some(boost(&mut s));
        assert_eq!(s.draw_distance, 256.0);
        let saved = s.to_saved();
        assert_eq!(saved.draw_distance, 96.0);
        assert_eq!(saved.photo_held, None);
        // and the held values never reach the file
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(!json.contains("photo_held"));
    }
}
