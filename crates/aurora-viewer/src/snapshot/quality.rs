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
    /// What the boost wrote; a setting found otherwise was changed by the
    /// user (Preferences) and is theirs from then on.
    boosted: Values,
    boosted_shadows: bool,
}

/// Every field of `Values`, for `Held::sync`.
macro_rules! adopt {
    ($held:expr, $now:expr, $($f:ident),*) => {
        $(
            if $now.$f != $held.boosted.$f {
                $held.values.$f = $now.$f;
                $held.boosted.$f = $now.$f;
            }
        )*
    };
}

impl Held {
    /// A setting the user changed while the boost is on (in Preferences)
    /// becomes their value: it is kept when the boost ends, and no longer
    /// raised while it lasts.
    pub fn sync(&mut self, s: &Settings) {
        let now = Values::read(s);
        adopt!(
            self,
            now,
            draw_distance,
            lod_factor,
            max_avatars,
            max_complexity,
            max_particles,
            shadow_quality,
            projector_shadows,
            ssao,
            water_reflections,
            mirrors,
            ssr,
            reflection_probes,
            probe_count,
            antialiasing,
            anisotropy,
            sharpen,
            glow
        );
        if s.shadows != self.boosted_shadows {
            self.shadows = s.shadows;
            self.boosted_shadows = s.shadows;
        }
    }

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
    let values = Values::read(s);
    let shadows = s.shadows;
    boosted(values).write(s);
    Held {
        values,
        shadows,
        boosted: Values::read(s),
        boosted_shadows: s.shadows,
    }
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
    fn a_setting_changed_during_the_boost_becomes_the_users() {
        let mut s = Settings {
            draw_distance: 96.0,
            shadow_quality: 1,
            antialiasing: 3,
            ..Settings::default()
        };
        let mut held = boost(&mut s);
        // the user lowers the draw distance and changes the anti-aliasing
        s.draw_distance = 128.0;
        s.antialiasing = 6;
        held.sync(&s);
        // a second sync with nothing new changes nothing
        held.sync(&s);
        // their values stay while the boost lasts, the others stay raised
        assert_eq!((s.draw_distance, s.shadow_quality), (128.0, 4));
        held.restore(&mut s);
        // and are theirs afterwards; the untouched ones come back
        assert_eq!((s.draw_distance, s.antialiasing, s.shadow_quality), (128.0, 6, 1));
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
