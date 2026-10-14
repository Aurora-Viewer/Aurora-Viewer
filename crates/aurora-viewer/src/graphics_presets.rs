//! Graphics presets of Preferences › Graphics: five levels, from a very
//! modest machine to everything a strong card can afford. Each level writes
//! the same set of quality settings; when one of them is then changed by
//! hand, no level matches any more and the row shows "Custom", like
//! Firestorm's "Quality and speed" slider once a setting moved
//! (LLFeatureManager::setGraphicsLevel, indra/newview/llfeaturemanager.cpp;
//! the values follow the Low, LowMid, Mid, High and Ultra lists of
//! indra/newview/featuretable.txt, mapped onto Aurora's settings).
//!
//! What a level does not touch: the texture memory (sized from the video
//! memory), the frame rate limits and vertical sync (comfort, not quality),
//! occlusion culling (only a gain), the environment and the image tuning
//! (exposure, tone mapping), and the limits of the worn surface and of the
//! complexity rule, which are choices about other people.

use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Potato,
    Low,
    Medium,
    High,
    Deluxe,
}

impl Preset {
    pub const ALL: [Preset; 5] = [Preset::Potato, Preset::Low, Preset::Medium, Preset::High, Preset::Deluxe];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Potato => aurora_i18n::tr!("ui-options-preset-potato"),
            Preset::Low => aurora_i18n::tr!("ui-options-preset-low"),
            Preset::Medium => aurora_i18n::tr!("ui-options-preset-medium"),
            Preset::High => aurora_i18n::tr!("ui-options-preset-high"),
            Preset::Deluxe => aurora_i18n::tr!("ui-options-preset-deluxe"),
        }
    }

    /// The name accepted by `AURORA_GRAPHICS_PRESET`.
    pub fn from_name(name: &str) -> Option<Preset> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "potato" => Preset::Potato,
            "low" => Preset::Low,
            "medium" => Preset::Medium,
            "high" => Preset::High,
            "deluxe" => Preset::Deluxe,
            _ => return None,
        })
    }

    pub fn values(self) -> Values {
        match self {
            // featuretable "Low": 64 m, LOD 1.5, 3 full avatars at 35 k,
            // 256 particles, no shadows, SSAO, water or probe reflections,
            // no anti-aliasing, anisotropy or sharpening; glow off as well
            // (a full-screen pass the oldest cards feel)
            Preset::Potato => Values {
                draw_distance: 64.0,
                lod_factor: 1.5,
                max_avatars: 3,
                max_complexity: 35_000,
                max_particles: 256,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 0,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 0,
                probe_count: 8,
                antialiasing: 0,
                anisotropy: 1,
                sharpen: 0.0,
                glow: false,
            },
            // "LowMid": 96 m, LOD 2, 5 avatars at 100 k, 2,048 particles,
            // still nothing of the deferred effects
            Preset::Low => Values {
                draw_distance: 96.0,
                lod_factor: 2.0,
                max_avatars: 5,
                max_complexity: 100_000,
                max_particles: 2048,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 0,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 0,
                probe_count: 16,
                antialiasing: 0,
                anisotropy: 1,
                sharpen: 0.0,
                glow: true,
            },
            // "Mid": 128 m, 7 avatars at 200 k, 4,096 particles, FXAA,
            // anisotropy, manual reflection probes; no shadows yet
            Preset::Medium => Values {
                draw_distance: 128.0,
                lod_factor: 2.0,
                max_avatars: 7,
                max_complexity: 200_000,
                max_particles: 4096,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 0,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 1,
                probe_count: 32,
                antialiasing: 1,
                anisotropy: 8,
                sharpen: 0.0,
                glow: true,
            },
            // "High": 11 avatars at 300 k, sun and moon shadows (detail 1:
            // no projector shadows), SSAO, water reflection, probes with
            // the terrain, SMAA and CAS sharpening at 0.4
            Preset::High => Values {
                draw_distance: 128.0,
                lod_factor: 2.0,
                max_avatars: 11,
                max_complexity: 300_000,
                max_particles: 4096,
                shadow_quality: 2,
                projector_shadows: false,
                ssao: 1,
                water_reflections: 1,
                mirrors: 0,
                ssr: false,
                reflection_probes: 2,
                probe_count: 32,
                antialiasing: 6,
                anisotropy: 8,
                sharpen: 0.4,
                glow: true,
            },
            // "Ultra": 256 m, LOD 3, 16 avatars, 8,192 particles, shadows of
            // the projectors too (detail 2), the whole scene in the probes;
            // the rest is "the most that stays reasonable": the complexity
            // of "HighUltra" (Ultra keeps the user's), the highest SSAO and
            // shadow quality, water reflection at half resolution (full
            // costs a second scene), Aurora's mirrors as by default and 16x
            // anisotropy; screen-space reflections stay off, as in every
            // Firestorm level
            Preset::Deluxe => Values {
                draw_distance: 256.0,
                lod_factor: 3.0,
                max_avatars: 16,
                max_complexity: 350_000,
                max_particles: 8192,
                shadow_quality: 4,
                projector_shadows: true,
                ssao: 2,
                water_reflections: 2,
                mirrors: 1,
                ssr: false,
                reflection_probes: 3,
                probe_count: 64,
                antialiasing: 6,
                anisotropy: 16,
                sharpen: 0.4,
                glow: true,
            },
        }
    }

    /// The level of a new profile, from the graphics card, like Firestorm's
    /// first launch (LLFeatureManager::loadGPUClass then
    /// applyRecommendedSettings): Medium on an integrated GPU or a dedicated
    /// one under 2 GB of video memory (the threshold applyBaseMasks tests
    /// for its VRAMLT2GB mask, a list Firestorm's table leaves empty today:
    /// lowering the level there is Aurora's own rule), High otherwise; one
    /// level lower with less than 8 GB of system memory, as Firestorm does on
    /// Windows.
    /// Firestorm ranks the card by a memory-bandwidth benchmark; Aurora asks
    /// wgpu for the kind of adapter and its memory instead. An unknown size
    /// counts as enough.
    pub fn for_machine(integrated: bool, vram_mb: Option<u64>, system_mb: Option<u64>) -> Preset {
        const SMALL_VRAM_MB: u64 = 2048;
        const MIN_SYSTEM_MB: u64 = 8 * 1024;
        let level = if integrated || vram_mb.is_some_and(|v| v < SMALL_VRAM_MB) {
            Preset::Medium
        } else {
            Preset::High
        };
        if system_mb.is_some_and(|m| m < MIN_SYSTEM_MB) {
            level.lower()
        } else {
            level
        }
    }

    fn lower(self) -> Preset {
        match self {
            Preset::Potato | Preset::Low => Preset::Potato,
            Preset::Medium => Preset::Low,
            Preset::High => Preset::Medium,
            Preset::Deluxe => Preset::High,
        }
    }

    /// Writes the level's settings (the shadow switch follows its quality,
    /// like the slider of the page).
    pub fn apply(self, s: &mut Settings) {
        self.values().write(s);
    }

    /// The level whose settings are exactly those of `s`, if any.
    pub fn matching(s: &Settings) -> Option<Preset> {
        let current = Values::read(s).as_shown();
        Preset::ALL.into_iter().find(|p| p.values().as_shown() == current)
    }
}

/// The settings a level writes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Values {
    pub draw_distance: f32,
    pub lod_factor: f32,
    pub max_avatars: u32,
    pub max_complexity: u32,
    pub max_particles: u32,
    pub shadow_quality: u8,
    pub projector_shadows: bool,
    pub ssao: u8,
    pub water_reflections: u8,
    pub mirrors: u8,
    pub ssr: bool,
    pub reflection_probes: u8,
    pub probe_count: u32,
    pub antialiasing: u8,
    pub anisotropy: u16,
    pub sharpen: f32,
    pub glow: bool,
}

impl Values {
    /// The values as the page's sliders show them: a setting moved back by
    /// hand to what its level shows reads as that level again, not custom
    /// (the complexity slider only stops at its own steps, 302 k for the
    /// 300,000 of "High", like Firestorm's).
    fn as_shown(self) -> Values {
        use crate::scene::complexity::slider;
        let round = |v: f32, step: f32| (v / step).round() * step;
        Values {
            draw_distance: round(self.draw_distance, 1.0),
            lod_factor: round(self.lod_factor, 0.125),
            max_complexity: slider::to_limit(slider::from_limit(self.max_complexity)),
            sharpen: round(self.sharpen, 0.05),
            ..self
        }
    }

    pub(crate) fn read(s: &Settings) -> Values {
        Values {
            draw_distance: s.draw_distance,
            lod_factor: s.lod_factor,
            max_avatars: s.max_avatars,
            max_complexity: s.max_complexity,
            max_particles: s.max_particles,
            shadow_quality: s.shadow_quality,
            projector_shadows: s.projector_shadows,
            ssao: s.ssao,
            water_reflections: s.water_reflections,
            mirrors: s.mirrors,
            ssr: s.ssr,
            reflection_probes: s.reflection_probes,
            probe_count: s.probe_count,
            antialiasing: s.antialiasing,
            anisotropy: s.anisotropy,
            sharpen: s.sharpen,
            glow: s.glow,
        }
    }

    pub(crate) fn write(self, s: &mut Settings) {
        s.draw_distance = self.draw_distance;
        s.lod_factor = self.lod_factor;
        s.max_avatars = self.max_avatars;
        s.max_complexity = self.max_complexity;
        s.max_particles = self.max_particles;
        s.shadow_quality = self.shadow_quality;
        s.shadows = self.shadow_quality > 0;
        s.projector_shadows = self.projector_shadows;
        s.ssao = self.ssao;
        s.water_reflections = self.water_reflections;
        s.mirrors = self.mirrors;
        s.ssr = self.ssr;
        s.reflection_probes = self.reflection_probes;
        s.probe_count = self.probe_count;
        s.antialiasing = self.antialiasing;
        s.anisotropy = self.anisotropy;
        s.sharpen = self.sharpen;
        s.glow = self.glow;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_matches_itself_after_applying() {
        for preset in Preset::ALL {
            let mut s = Settings::default();
            preset.apply(&mut s);
            assert_eq!(Preset::matching(&s), Some(preset), "{preset:?}");
            assert_eq!(s.shadows, s.shadow_quality > 0);
        }
    }

    #[test]
    fn a_setting_changed_by_hand_reads_as_custom() {
        let mut s = Settings::default();
        Preset::Medium.apply(&mut s);
        s.draw_distance = 200.0;
        assert_eq!(Preset::matching(&s), None);
        // moved back to the level's value: the level again
        s.draw_distance = 128.0;
        assert_eq!(Preset::matching(&s), Some(Preset::Medium));
        s.ssao = 2;
        assert_eq!(Preset::matching(&s), None);
    }

    #[test]
    fn the_complexity_slider_step_of_a_level_still_reads_as_that_level() {
        use crate::scene::complexity::slider;
        let mut s = Settings::default();
        Preset::High.apply(&mut s);
        // what the slider stores once moved onto the step it shows
        s.max_complexity = slider::to_limit(slider::from_limit(s.max_complexity));
        assert_ne!(s.max_complexity, 300_000);
        assert_eq!(Preset::matching(&s), Some(Preset::High));
    }

    #[test]
    fn levels_never_touch_what_is_not_quality() {
        let mut s = Settings {
            texture_budget_auto: false,
            texture_budget_mb: 4321,
            fps_limit: 77,
            vsync: true,
            occlusion_culling: false,
            exposure: 1.7,
            max_attachment_area: 123.0,
            ..Settings::default()
        };
        for preset in Preset::ALL {
            preset.apply(&mut s);
            assert!(!s.texture_budget_auto);
            assert_eq!(s.texture_budget_mb, 4321);
            assert_eq!(s.fps_limit, 77);
            assert!(s.vsync);
            assert!(!s.occlusion_culling);
            assert_eq!(s.exposure, 1.7);
            assert_eq!(s.max_attachment_area, 123.0);
        }
    }

    #[test]
    fn levels_rise_from_potato_to_deluxe() {
        let v: Vec<Values> = Preset::ALL.iter().map(|p| p.values()).collect();
        for w in v.windows(2) {
            let (a, b) = (w[0], w[1]);
            assert!(a.draw_distance <= b.draw_distance);
            assert!(a.lod_factor <= b.lod_factor);
            assert!(a.max_avatars <= b.max_avatars);
            assert!(a.max_complexity <= b.max_complexity);
            assert!(a.max_particles <= b.max_particles);
            assert!(a.shadow_quality <= b.shadow_quality);
            assert!(a.ssao <= b.ssao);
            assert!(a.reflection_probes <= b.reflection_probes);
            assert!(a.probe_count <= b.probe_count);
            assert!(a.anisotropy <= b.anisotropy);
        }
    }

    #[test]
    fn values_stay_inside_what_the_page_offers() {
        for preset in Preset::ALL {
            let v = preset.values();
            assert!((32.0..=512.0).contains(&v.draw_distance));
            assert!((0.5..=4.0).contains(&v.lod_factor));
            assert!((1..=100).contains(&v.max_avatars));
            assert!(v.max_particles <= 8192);
            assert!(v.shadow_quality <= 4 && v.ssao <= 2 && v.reflection_probes <= 3);
            assert!(v.water_reflections <= 4 && v.mirrors <= 4);
            assert!((8..=64).contains(&v.probe_count) && v.probe_count % 8 == 0);
            assert!(v.antialiasing <= 6);
            assert!([1, 2, 4, 8, 16].contains(&v.anisotropy));
            assert!((0.0..=1.0).contains(&v.sharpen));
        }
    }

    #[test]
    fn a_new_profile_takes_its_level_from_the_graphics_card() {
        const GB: u64 = 1024;
        // a dedicated card with enough of everything
        assert_eq!(Preset::for_machine(false, Some(24 * GB), Some(32 * GB)), Preset::High);
        assert_eq!(Preset::for_machine(false, Some(2 * GB), Some(16 * GB)), Preset::High);
        // integrated, or a small dedicated card
        assert_eq!(Preset::for_machine(true, Some(4 * GB), Some(16 * GB)), Preset::Medium);
        assert_eq!(Preset::for_machine(false, Some(1536), Some(16 * GB)), Preset::Medium);
        // less than 8 GB of system memory: one level lower
        assert_eq!(Preset::for_machine(false, Some(8 * GB), Some(7900)), Preset::Medium);
        assert_eq!(Preset::for_machine(true, None, Some(4 * GB)), Preset::Low);
        // what could not be read does not lower the level
        assert_eq!(Preset::for_machine(false, None, None), Preset::High);
        assert_eq!(Preset::for_machine(true, None, None), Preset::Medium);
    }

    #[test]
    fn demo_names() {
        assert_eq!(Preset::from_name("Potato"), Some(Preset::Potato));
        assert_eq!(Preset::from_name(" deluxe "), Some(Preset::Deluxe));
        assert_eq!(Preset::from_name("ultra"), None);
    }
}
