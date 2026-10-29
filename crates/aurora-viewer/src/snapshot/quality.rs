//! Photo quality: while the photo window is open with its "Photo quality"
//! switch on, the graphics settings are raised to the most Aurora renders
//! (draw distance, levels of detail, every avatar in full, shadows,
//! ambient occlusion, reflections at full resolution, anisotropy), the
//! textures in view are loaded at full resolution and the photo itself is
//! drawn at twice its size (`app::snapshot`), the user's own values set
//! aside.
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
    /// The user's current values without cloning the entire preferences or
    /// their saved profile library for the FPS hover card's active marker.
    pub(crate) fn user_values(&self, s: &Settings) -> (Values, bool) {
        let mut user = self.clone();
        user.sync(s);
        (user.values, user.shadows)
    }

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
            ao_method,
            gtao_quality,
            water_reflections,
            mirrors,
            ssr,
            reflection_probes,
            probe_count,
            antialiasing,
            anisotropy,
            sharpen,
            glow,
            effects
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

/// The most each setting goes to (the top of its range in Aurora; Firestorm
/// allows a draw distance of 1,024 m and a LOD factor of 8 by hand, Aurora
/// draws up to 512 m and 4). Every avatar is drawn in full (no impostor up
/// to Aurora's 100, no complexity limit): a photo shows people as they are.
fn photo_maxima() -> Values {
    Values {
        draw_distance: 512.0,
        lod_factor: 4.0,
        max_avatars: 100,
        max_complexity: 0,
        shadow_quality: 4,
        projector_shadows: true,
        ssao: 2,
        water_reflections: 4,
        mirrors: 4,
        reflection_probes: 3,
        probe_count: 64,
        anisotropy: 16,
        ..Preset::Deluxe.values()
    }
}

/// `Settings::antialiasing` for MSAA 8x, the photo's anti-aliasing on top
/// of the supersampling (asked for by the user; the renderer takes the most
/// samples the GPU supports up to 8).
pub const MSAA_8X: u8 = 4;

/// The boosted values: each one at Aurora's most, never lower than the
/// user's (an unlimited complexity stays unlimited), the anti-aliasing at
/// MSAA 8x. The particles, the screen-space reflections, the sharpening, the
/// glow and Aurora's effects (bloom, contact shadows, indirect light) stay
/// the user's own choices.
pub fn boosted(user: Values) -> Values {
    let top = photo_maxima();
    Values {
        antialiasing: MSAA_8X,
        draw_distance: user.draw_distance.max(top.draw_distance),
        lod_factor: user.lod_factor.max(top.lod_factor),
        max_avatars: user.max_avatars.max(top.max_avatars),
        max_complexity: top.max_complexity,
        shadow_quality: user.shadow_quality.max(top.shadow_quality),
        projector_shadows: user.projector_shadows || top.projector_shadows,
        ssao: user.ssao.max(top.ssao),
        water_reflections: user.water_reflections.max(top.water_reflections),
        mirrors: user.mirrors.max(top.mirrors),
        reflection_probes: user.reflection_probes.max(top.reflection_probes),
        probe_count: user.probe_count.max(top.probe_count),
        anisotropy: user.anisotropy.max(top.anisotropy),
        ..user
    }
}

/// The photo is drawn at this many times its size, then reduced (a box
/// filter over whole pixels), while its larger side stays within
/// `snapshot::MAX_SIDE`: an 8K photo is drawn once, a 4K one at 8K.
pub fn supersample(size: (u32, u32)) -> u32 {
    if size.0.max(size.1) * 2 <= super::MAX_SIDE { 2 } else { 1 }
}

/// The longest wait for the textures in view to reach full resolution
/// before the photo is taken anyway.
pub const DETAIL_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

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
        assert_eq!((s.draw_distance, s.lod_factor, s.max_avatars), (512.0, 4.0, 100));
        assert_eq!((s.shadow_quality, s.water_reflections, s.mirrors), (4, 4, 4));
        assert!(s.shadows);
        // no complexity limit during the photo, MSAA 8x
        assert_eq!((s.antialiasing, s.max_complexity), (MSAA_8X, 0));
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
    fn photos_up_to_4k_are_drawn_at_twice_their_size() {
        assert_eq!(supersample((3840, 2160)), 2);
        assert_eq!(supersample((1600, 900)), 2);
        assert_eq!(supersample((5120, 2880)), 1);
        assert_eq!(supersample((7680, 4320)), 1);
    }

    #[test]
    fn a_boosted_profile_saves_the_users_values() {
        let mut s = Settings {
            draw_distance: 96.0,
            ..Settings::default()
        };
        s.photo_held = Some(boost(&mut s));
        assert_eq!(s.draw_distance, 512.0);
        let saved = s.to_saved();
        assert_eq!(saved.draw_distance, 96.0);
        assert_eq!(saved.photo_held, None);
        // and the held values never reach the file
        let json = serde_json::to_string(&s).expect("serialize");
        assert!(!json.contains("photo_held"));
    }
}
