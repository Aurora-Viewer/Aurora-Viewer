//! Graphics presets of Preferences › Graphics: five levels, from a very
//! modest machine to everything a strong card can afford. Each level writes
//! the same set of quality settings; when one of them is then changed by
//! hand, no level matches any more and the row shows "Custom", like
//! Firestorm's "Quality and speed" slider once a setting moved
//! (LLFeatureManager::setGraphicsLevel, indra/newview/llfeaturemanager.cpp;
//! the values follow the Low, LowMid, Mid, High and Ultra lists of
//! indra/newview/featuretable.txt, mapped onto Aurora's settings).
//!
//! From Medium up the levels go beyond the Legacy Viewer's lists with
//! Aurora's own effects, which the Legacy Viewer has no level for (decided by
//! the human on 2026-10-17): the ambient occlusion by GTAO (gtao.rs in
//! aurora-render, its specular occlusion following it) at Low quality in
//! Medium (about 0.1 ms at 2560 x 1369 in the demo crowd), High quality in
//! High and Ultra in Deluxe; the contact shadows of the sun in High and
//! Deluxe; the screen-space indirect light, still marked experimental in the
//! page, in Deluxe only (High is the level a new profile gets on most
//! dedicated cards, and the indirect light is not measured on a mid-range
//! one yet). Deluxe also keeps the draw distance at 192 m, where the Legacy
//! Viewer's Ultra has 256. The bloom, experimental, stays off in every level.
//! Potato and Low have none of these; outside the levels they are all off by
//! default.
//!
//! The avatar physics detail follows the Legacy Viewer's lists too (0 in
//! Potato, 0.75 in Low, 1 above); the physics switch itself is not a level's.
//!
//! What a level does not touch: the texture memory (sized from the video
//! memory), the frame rate limits and vertical sync (comfort, not quality),
//! occlusion culling (only a gain), the environment and the image tuning
//! (exposure, tone mapping), and the limits of the worn surface and of the
//! complexity rule, which are choices about other people.

use crate::settings::Settings;

pub(crate) mod saved;

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
            // no anti-aliasing, anisotropy or sharpening, no HDR and Emissive
            // (RenderDisableVintageMode 0); glow off as well (a full-screen
            // pass the oldest cards feel)
            Preset::Potato => Values {
                draw_distance: 64.0,
                lod_factor: 1.5,
                max_avatars: 3,
                max_complexity: 35_000,
                max_particles: 256,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 0,
                ao_method: 0,
                gtao_quality: 2,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 0,
                probe_count: 8,
                antialiasing: 0,
                anisotropy: 1,
                sharpen: 0.0,
                glow: false,
                avatar_physics_detail: 0.0,
                hdr_emissive: false,
                effects: Effects::OFF,
            },
            // "LowMid": 96 m, LOD 2, 5 avatars at 100 k, 2,048 particles,
            // still nothing of the deferred effects nor HDR and Emissive
            Preset::Low => Values {
                draw_distance: 96.0,
                lod_factor: 2.0,
                max_avatars: 5,
                max_complexity: 100_000,
                max_particles: 2048,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 0,
                ao_method: 0,
                gtao_quality: 2,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 0,
                probe_count: 16,
                antialiasing: 0,
                anisotropy: 1,
                sharpen: 0.0,
                glow: true,
                avatar_physics_detail: 0.75,
                hdr_emissive: false,
                effects: Effects::OFF,
            },
            // "Mid": 128 m, 7 avatars at 200 k, 4,096 particles, FXAA,
            // anisotropy, manual reflection probes; no shadows yet; and
            // Aurora's ambient occlusion by GTAO at Low quality
            Preset::Medium => Values {
                draw_distance: 128.0,
                lod_factor: 2.0,
                max_avatars: 7,
                max_complexity: 200_000,
                max_particles: 4096,
                shadow_quality: 0,
                projector_shadows: false,
                ssao: 1,
                ao_method: 1,
                gtao_quality: 0,
                water_reflections: 0,
                mirrors: 0,
                ssr: false,
                reflection_probes: 1,
                probe_count: 32,
                antialiasing: 1,
                anisotropy: 8,
                sharpen: 0.0,
                glow: true,
                avatar_physics_detail: 1.0,
                hdr_emissive: true,
                effects: Effects::OFF,
            },
            // "High": 11 avatars at 300 k, sun and moon shadows (detail 1:
            // no projector shadows), ambient occlusion (by GTAO at High
            // quality, Aurora's method, where the Legacy Viewer has its SSAO),
            // water reflection, probes with the terrain, SMAA and CAS
            // sharpening at 0.4; and Aurora's contact shadows
            Preset::High => Values {
                draw_distance: 128.0,
                lod_factor: 2.0,
                max_avatars: 11,
                max_complexity: 300_000,
                max_particles: 4096,
                shadow_quality: 2,
                projector_shadows: false,
                ssao: 1,
                ao_method: 1,
                gtao_quality: 2,
                water_reflections: 1,
                mirrors: 0,
                ssr: false,
                reflection_probes: 2,
                probe_count: 32,
                antialiasing: 6,
                anisotropy: 8,
                sharpen: 0.4,
                glow: true,
                avatar_physics_detail: 1.0,
                hdr_emissive: true,
                effects: Effects::HIGH,
            },
            // "Ultra": LOD 3, 16 avatars, 8,192 particles, shadows of the
            // projectors too (detail 2), the whole scene in the probes; the
            // rest is "the most that stays reasonable": 192 m instead of
            // Ultra's 256 (a busy place costs far less), the complexity of
            // "HighUltra" (Ultra keeps the user's), the highest shadow
            // quality, ambient occlusion by GTAO at Ultra quality (Aurora's
            // method, beyond the Legacy Viewer's SSAO of Ultra), water
            // reflection at half resolution (full costs a second scene),
            // Aurora's mirrors as by default and 16x anisotropy;
            // screen-space reflections stay off, as in every Legacy Viewer
            // level; and Aurora's own effects (module comment)
            Preset::Deluxe => Values {
                draw_distance: 192.0,
                lod_factor: 3.0,
                max_avatars: 16,
                max_complexity: 350_000,
                max_particles: 8192,
                shadow_quality: 4,
                projector_shadows: true,
                ssao: 2,
                ao_method: 1,
                gtao_quality: 3,
                water_reflections: 2,
                mirrors: 1,
                ssr: false,
                reflection_probes: 3,
                probe_count: 64,
                antialiasing: 6,
                anisotropy: 16,
                sharpen: 0.4,
                glow: true,
                avatar_physics_detail: 1.0,
                hdr_emissive: true,
                effects: Effects::DELUXE,
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

/// The Deluxe of settings version 20: 256 m, GTAO at High and the contact
/// shadows only. The older Deluxes below are written from it, so that they
/// stay what those files hold whatever the levels of now become.
pub(crate) fn deluxe_v20() -> Values {
    Values {
        draw_distance: 256.0,
        gtao_quality: 2,
        effects: Effects {
            contact_shadows: true,
            ..Effects::OFF
        },
        ..Preset::Deluxe.values()
    }
}

/// The High of settings version 20: GTAO at Medium, none of the effects.
pub(crate) fn high_v20() -> Values {
    Values {
        gtao_quality: 1,
        effects: Effects::OFF,
        ..Preset::High.values()
    }
}

/// The Medium of settings version 20: no ambient occlusion.
pub(crate) fn medium_v20() -> Values {
    Values {
        ssao: 0,
        ao_method: 0,
        gtao_quality: 2,
        ..Preset::Medium.values()
    }
}

/// The Deluxe of settings version 18: GTAO at High, none of the effects
/// added since.
pub(crate) fn deluxe_v18() -> Values {
    Values {
        effects: Effects::OFF,
        ..deluxe_v20()
    }
}

/// Settings files written before Deluxe took GTAO (version 18): a profile
/// still exactly on the former Deluxe (the Legacy occlusion at its high
/// level) moves to the Deluxe of now; any other, a level or custom, keeps
/// its occlusion.
pub(crate) fn upgrade_deluxe(s: &mut Settings) {
    let former = Values {
        ao_method: 0,
        gtao_quality: 2,
        ..deluxe_v18()
    };
    if Values::read(s).as_shown() == former.as_shown() {
        Preset::Deluxe.apply(s);
    }
}

/// The Deluxe of settings version 19: every effect on, the bloom and the
/// indirect light included.
pub(crate) fn deluxe_v19() -> Values {
    Values {
        effects: Effects {
            bloom: true,
            screen_indirect_light: true,
            ..deluxe_v20().effects
        },
        ..deluxe_v20()
    }
}

/// Settings files written before Deluxe took the bloom, the contact shadows
/// and the indirect light (version 19): a profile still exactly on the
/// Deluxe of version 18 moves to the Deluxe of now; any other keeps them as
/// they were.
pub(crate) fn upgrade_deluxe_effects(s: &mut Settings) {
    if Values::read(s).as_shown() == deluxe_v18().as_shown() {
        Preset::Deluxe.apply(s);
    }
}

/// Settings files written while Deluxe turned on the bloom and the indirect
/// light (version 19), both experimental since: a profile still exactly on
/// that Deluxe moves to the Deluxe of now (both off); any other keeps them.
pub(crate) fn upgrade_deluxe_experimental(s: &mut Settings) {
    if Values::read(s).as_shown() == deluxe_v19().as_shown() {
        Preset::Deluxe.apply(s);
    }
}

/// The High of settings version 19: the Legacy occlusion at its low level.
pub(crate) fn high_v19() -> Values {
    Values {
        ao_method: 0,
        gtao_quality: 2,
        ..high_v20()
    }
}

/// Settings files written before High took GTAO at Medium quality (version
/// 20): a profile still exactly on the former High follows it; any other, a
/// level or custom, keeps its occlusion.
pub(crate) fn upgrade_high_gtao(s: &mut Settings) {
    if Values::read(s).as_shown() == high_v19().as_shown() {
        Preset::High.apply(s);
    }
}

/// Settings files written before the levels of 2026-10-17 (version 21):
/// Medium took GTAO at Low quality, High GTAO at High quality and the contact
/// shadows, Deluxe GTAO at Ultra quality, the indirect light and 192 m. A
/// profile still exactly on the Medium, the High or the Deluxe of version 20
/// follows its level; any other, a level or custom, keeps its values.
pub(crate) fn upgrade_levels_v21(s: &mut Settings) {
    let current = Values::read(s).as_shown();
    for (former, preset) in [
        (medium_v20(), Preset::Medium),
        (high_v20(), Preset::High),
        (deluxe_v20(), Preset::Deluxe),
    ] {
        if current == former.as_shown() {
            preset.apply(s);
            return;
        }
    }
}

/// Settings files written before the levels set the avatar physics detail
/// (version 24), all at 1: a profile exactly on Potato or Low apart from it
/// takes the level's detail (0 and 0.75, the Legacy Viewer's Low and
/// LowMid); any other keeps its value.
pub(crate) fn upgrade_physics_detail(s: &mut Settings) {
    for preset in [Preset::Potato, Preset::Low] {
        let level = preset.values();
        // (files of then all have HDR and Emissive on: upgrade_hdr_emissive)
        let current = Values {
            avatar_physics_detail: level.avatar_physics_detail,
            hdr_emissive: level.hdr_emissive,
            ..Values::read(s)
        };
        if current.as_shown() == level.as_shown() {
            s.avatar_physics_detail = level.avatar_physics_detail;
            return;
        }
    }
}

/// Settings files written before HDR and Emissive could be turned off
/// (version 28), all with it on: a profile exactly on Potato or Low apart
/// from it takes the level's (off, as the Legacy Viewer's Low and LowMid);
/// any other keeps it on.
pub(crate) fn upgrade_hdr_emissive(s: &mut Settings) {
    for preset in [Preset::Potato, Preset::Low] {
        let level = preset.values();
        let current = Values {
            hdr_emissive: level.hdr_emissive,
            ..Values::read(s)
        };
        if current.as_shown() == level.as_shown() {
            s.hdr_emissive = level.hdr_emissive;
            return;
        }
    }
}

/// Aurora's effects beyond the Legacy Viewer's levels, which High and
/// Deluxe turn on.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Effects {
    pub bloom: bool,
    pub bloom_strength: f32,
    pub contact_shadows: bool,
    pub screen_indirect_light: bool,
    pub screen_indirect_intensity: f32,
}

impl Effects {
    /// None of them, at their default strengths.
    pub const OFF: Effects = Effects {
        bloom: false,
        bloom_strength: crate::settings::BLOOM_STRENGTH,
        contact_shadows: false,
        screen_indirect_light: false,
        screen_indirect_intensity: crate::settings::SCREEN_INDIRECT_INTENSITY,
    };
    /// High's: the contact shadows.
    pub const HIGH: Effects = Effects {
        contact_shadows: true,
        ..Effects::OFF
    };
    /// Deluxe's: the contact shadows and the indirect light at its default
    /// intensity; the bloom, experimental, stays off.
    pub const DELUXE: Effects = Effects {
        contact_shadows: true,
        screen_indirect_light: true,
        ..Effects::OFF
    };
}

/// The settings a level writes. Saved as they are by the safe graphics
/// (`app/safe_graphics.rs`), to be put back.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Values {
    pub draw_distance: f32,
    pub lod_factor: f32,
    pub max_avatars: u32,
    pub max_complexity: u32,
    pub max_particles: u32,
    pub shadow_quality: u8,
    pub projector_shadows: bool,
    pub ssao: u8,
    /// 0 Legacy, 1 GTAO (`Settings::ao_method`).
    pub ao_method: u8,
    pub gtao_quality: u8,
    pub water_reflections: u8,
    pub mirrors: u8,
    pub ssr: bool,
    pub reflection_probes: u8,
    pub probe_count: u32,
    pub antialiasing: u8,
    pub anisotropy: u16,
    pub sharpen: f32,
    pub glow: bool,
    /// RenderAvatarPhysicsLODFactor: 0 in Low, 0.75 in LowMid, 1 above.
    pub avatar_physics_detail: f32,
    /// RenderDisableVintageMode: off in Low and LowMid, on above (a backup
    /// written before it holds none: on, the default then).
    #[serde(default = "hdr_on")]
    pub hdr_emissive: bool,
    pub effects: Effects,
}

fn hdr_on() -> bool {
    true
}

impl Values {
    /// The values as the page's sliders show them: a setting moved back by
    /// hand to what its level shows reads as that level again, not custom
    /// (the complexity slider only stops at its own steps, 302 k for the
    /// 300,000 of "High", like Firestorm's).
    /// The method and quality of the occlusion only count while it is on,
    /// and GTAO's quality only under GTAO (the page greys them out); the
    /// same for the effects: the contact shadows only with the shadows, the
    /// indirect light only with GTAO, a strength only while its effect is on.
    fn as_shown(self) -> Values {
        use crate::scene::complexity::slider;
        let round = |v: f32, step: f32| (v / step).round() * step;
        let ao_method = if self.ssao == 0 { 0 } else { self.ao_method };
        let e = self.effects;
        let indirect = ao_method == 1 && e.screen_indirect_light;
        let effects = Effects {
            bloom: e.bloom,
            bloom_strength: if e.bloom {
                round(e.bloom_strength, 0.05)
            } else {
                Effects::OFF.bloom_strength
            },
            contact_shadows: self.shadow_quality > 0 && e.contact_shadows,
            screen_indirect_light: indirect,
            screen_indirect_intensity: if indirect {
                round(e.screen_indirect_intensity, 0.1)
            } else {
                Effects::OFF.screen_indirect_intensity
            },
        };
        Values {
            draw_distance: round(self.draw_distance, 1.0),
            lod_factor: round(self.lod_factor, 0.125),
            max_complexity: slider::to_limit(slider::from_limit(self.max_complexity)),
            sharpen: round(self.sharpen, 0.05),
            avatar_physics_detail: round(self.avatar_physics_detail, 0.05),
            ao_method,
            gtao_quality: if ao_method == 1 { self.gtao_quality } else { 2 },
            effects,
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
            ao_method: s.ao_method,
            gtao_quality: s.gtao_quality,
            water_reflections: s.water_reflections,
            mirrors: s.mirrors,
            ssr: s.ssr,
            reflection_probes: s.reflection_probes,
            probe_count: s.probe_count,
            antialiasing: s.antialiasing,
            anisotropy: s.anisotropy,
            sharpen: s.sharpen,
            glow: s.glow,
            avatar_physics_detail: s.avatar_physics_detail,
            hdr_emissive: s.hdr_emissive,
            effects: Effects {
                bloom: s.bloom,
                bloom_strength: s.bloom_strength,
                contact_shadows: s.contact_shadows,
                screen_indirect_light: s.screen_indirect_light,
                screen_indirect_intensity: s.screen_indirect_intensity,
            },
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
        s.ao_method = self.ao_method;
        s.gtao_quality = self.gtao_quality;
        s.water_reflections = self.water_reflections;
        s.mirrors = self.mirrors;
        s.ssr = self.ssr;
        s.reflection_probes = self.reflection_probes;
        s.probe_count = self.probe_count;
        s.antialiasing = self.antialiasing;
        s.anisotropy = self.anisotropy;
        s.sharpen = self.sharpen;
        s.glow = self.glow;
        s.avatar_physics_detail = self.avatar_physics_detail;
        s.hdr_emissive = self.hdr_emissive;
        s.bloom = self.effects.bloom;
        s.bloom_strength = self.effects.bloom_strength;
        s.contact_shadows = self.effects.contact_shadows;
        s.screen_indirect_light = self.effects.screen_indirect_light;
        s.screen_indirect_intensity = self.effects.screen_indirect_intensity;
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
    fn the_low_levels_lower_the_avatar_physics_like_the_legacy_viewer() {
        let detail = |p: Preset| p.values().avatar_physics_detail;
        assert_eq!(detail(Preset::Potato), 0.0);
        assert_eq!(detail(Preset::Low), 0.75);
        for p in [Preset::Medium, Preset::High, Preset::Deluxe] {
            assert_eq!(detail(p), 1.0, "{p:?}");
        }
        // a hand-moved detail reads as custom
        let mut s = Settings::default();
        Preset::Low.apply(&mut s);
        s.avatar_physics_detail = 0.5;
        assert_eq!(Preset::matching(&s), None);
        // an older file exactly on Potato or Low (detail 1 then) takes the
        // level's detail; a custom profile keeps 1
        for preset in [Preset::Potato, Preset::Low] {
            let mut s = Settings::default();
            preset.apply(&mut s);
            s.avatar_physics_detail = 1.0;
            upgrade_physics_detail(&mut s);
            assert_eq!(Preset::matching(&s), Some(preset));
        }
        let mut s = Settings::default();
        Preset::Low.apply(&mut s);
        s.avatar_physics_detail = 1.0;
        s.draw_distance = 300.0;
        upgrade_physics_detail(&mut s);
        assert_eq!(s.avatar_physics_detail, 1.0);
        Preset::High.apply(&mut s);
        upgrade_physics_detail(&mut s);
        assert_eq!(s.avatar_physics_detail, 1.0);
    }

    /// RenderDisableVintageMode 0 in the Legacy Viewer's Low and LowMid
    /// (featuretable.txt), 1 above.
    #[test]
    fn the_low_levels_turn_hdr_off_like_the_legacy_viewer() {
        let hdr = |p: Preset| p.values().hdr_emissive;
        assert!(!hdr(Preset::Potato) && !hdr(Preset::Low));
        for p in [Preset::Medium, Preset::High, Preset::Deluxe] {
            assert!(hdr(p), "{p:?}");
        }
        // an older file exactly on Potato or Low (HDR on then, and before
        // version 24 the physics detail at 1 too) takes the level's
        for preset in [Preset::Potato, Preset::Low] {
            let mut s = Settings::default();
            preset.apply(&mut s);
            s.hdr_emissive = true;
            s.avatar_physics_detail = 1.0;
            upgrade_physics_detail(&mut s);
            upgrade_hdr_emissive(&mut s);
            assert_eq!(Preset::matching(&s), Some(preset));
            assert!(!s.hdr_emissive);
        }
        // a custom profile and the higher levels keep it on
        let mut s = Settings::default();
        Preset::Potato.apply(&mut s);
        s.hdr_emissive = true;
        s.draw_distance = 300.0;
        upgrade_hdr_emissive(&mut s);
        assert!(s.hdr_emissive);
        Preset::Medium.apply(&mut s);
        upgrade_hdr_emissive(&mut s);
        assert!(s.hdr_emissive);
        // a safe graphics backup written before it holds none: on
        let mut json = serde_json::to_value(Preset::Low.values()).expect("values serialize");
        json.as_object_mut().expect("an object").remove("hdr_emissive");
        let read: Values = serde_json::from_value(json).expect("backup without the field");
        assert!(read.hdr_emissive);
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
            assert!(a.avatar_physics_detail <= b.avatar_physics_detail);
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
            assert!(v.ao_method <= 1 && v.gtao_quality <= 3);
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
    fn gtao_from_medium_up_and_the_levels_below_none() {
        let ao = |p: Preset| (p.values().ssao, p.values().ao_method, p.values().gtao_quality);
        assert_eq!(ao(Preset::Deluxe), (2, 1, 3));
        assert_eq!(ao(Preset::High), (1, 1, 2));
        assert_eq!(ao(Preset::Medium), (1, 1, 0));
        for p in [Preset::Potato, Preset::Low] {
            assert_eq!(p.values().ssao, 0, "{p:?}");
        }
        assert_eq!(Preset::Deluxe.values().draw_distance, 192.0);
        // the default method outside the levels stays Legacy
        assert_eq!(Settings::default().ao_method, 0);
    }

    #[test]
    fn the_occlusion_method_only_counts_while_it_is_on() {
        let mut s = Settings::default();
        Preset::Low.apply(&mut s);
        // occlusion off: a method chosen earlier does not make it custom
        s.ao_method = 1;
        s.gtao_quality = 0;
        assert_eq!(Preset::matching(&s), Some(Preset::Low));
        // Legacy method: the quality of GTAO does not count either
        high_v19().write(&mut s);
        let shown = Values::read(&s).as_shown();
        s.gtao_quality = 3;
        assert_eq!(Values::read(&s).as_shown(), shown);
        // Medium or High with another GTAO quality is custom
        Preset::Medium.apply(&mut s);
        s.gtao_quality = 1;
        assert_eq!(Preset::matching(&s), None);
        Preset::High.apply(&mut s);
        s.gtao_quality = 1;
        assert_eq!(Preset::matching(&s), None);
        // Deluxe with another GTAO quality, or the Legacy method, is custom
        Preset::Deluxe.apply(&mut s);
        s.gtao_quality = 2;
        assert_eq!(Preset::matching(&s), None);
        Preset::Deluxe.apply(&mut s);
        s.ao_method = 0;
        assert_eq!(Preset::matching(&s), None);
    }

    #[test]
    fn a_profile_on_the_former_deluxe_moves_to_gtao_and_no_other() {
        // the former Deluxe: the Legacy occlusion at its high level, none of
        // the effects added later
        let mut s = Settings::default();
        deluxe_v18().write(&mut s);
        s.ao_method = 0;
        upgrade_deluxe(&mut s);
        assert_eq!((s.ao_method, s.gtao_quality), (1, 3));
        assert_eq!(Preset::matching(&s), Some(Preset::Deluxe));
        // a custom profile close to it keeps its occlusion
        let mut custom = Settings::default();
        deluxe_v18().write(&mut custom);
        custom.ao_method = 0;
        custom.draw_distance = 300.0;
        upgrade_deluxe(&mut custom);
        assert_eq!(custom.ao_method, 0);
        // the other levels are untouched
        for p in [Preset::Potato, Preset::Low, Preset::Medium, Preset::High] {
            let mut s = Settings::default();
            p.apply(&mut s);
            let before = Values::read(&s);
            upgrade_deluxe(&mut s);
            assert_eq!(Values::read(&s), before, "{p:?}");
        }
    }

    #[test]
    fn high_and_deluxe_turn_on_their_effects_and_no_other_level_does() {
        let mut s = Settings::default();
        Preset::Deluxe.apply(&mut s);
        // contact shadows and indirect light; the experimental bloom off
        assert!(s.contact_shadows && !s.bloom && s.screen_indirect_light);
        assert_eq!(s.bloom_strength, crate::settings::BLOOM_STRENGTH);
        assert_eq!(s.screen_indirect_intensity, crate::settings::SCREEN_INDIRECT_INTENSITY);
        Preset::High.apply(&mut s);
        assert!(s.contact_shadows && !s.bloom && !s.screen_indirect_light);
        for p in [Preset::Potato, Preset::Low, Preset::Medium] {
            p.apply(&mut s);
            assert_eq!(p.values().effects, Effects::OFF, "{p:?}");
            assert!(!s.bloom && !s.contact_shadows && !s.screen_indirect_light, "{p:?}");
        }
        // outside the levels, all off
        let d = Settings::default();
        assert!(!d.bloom && !d.contact_shadows && !d.screen_indirect_light);
    }

    #[test]
    fn an_effect_only_counts_while_what_it_needs_is_on() {
        let mut s = Settings::default();
        // Low has no shadows and no occlusion: contact shadows, indirect
        // light and their strengths left from before do not make it custom
        Preset::Low.apply(&mut s);
        s.contact_shadows = true;
        s.screen_indirect_light = true;
        s.screen_indirect_intensity = 3.0;
        s.bloom_strength = 0.9;
        assert_eq!(Preset::matching(&s), Some(Preset::Low));
        // Medium has GTAO but no shadows: the indirect light counts, the
        // contact shadows do not
        Preset::Medium.apply(&mut s);
        s.contact_shadows = true;
        assert_eq!(Preset::matching(&s), Some(Preset::Medium));
        s.screen_indirect_light = true;
        assert_eq!(Preset::matching(&s), None);
        // High has shadows and GTAO: its contact shadows turned off, or the
        // indirect light turned on, is custom; the strength of the indirect
        // light, off there, does not count
        Preset::High.apply(&mut s);
        s.screen_indirect_light = true;
        assert_eq!(Preset::matching(&s), None);
        Preset::High.apply(&mut s);
        s.contact_shadows = false;
        assert_eq!(Preset::matching(&s), None);
        Preset::High.apply(&mut s);
        s.screen_indirect_intensity = 2.0;
        assert_eq!(Preset::matching(&s), Some(Preset::High));
        // the former High (Legacy occlusion): the indirect light, GTAO only,
        // does not count there
        let mut legacy = Settings::default();
        high_v19().write(&mut legacy);
        let shown = Values::read(&legacy).as_shown();
        legacy.screen_indirect_light = true;
        assert_eq!(Values::read(&legacy).as_shown(), shown);
        // the bloom counts whenever it is on
        Preset::High.apply(&mut s);
        s.bloom = true;
        assert_eq!(Preset::matching(&s), None);
        // Deluxe with its contact shadows or indirect light off, or the
        // bloom on, or another indirect intensity, is custom; the strength of
        // the bloom, off, does not count
        for change in [
            |s: &mut Settings| s.contact_shadows = false,
            |s: &mut Settings| s.screen_indirect_light = false,
            |s: &mut Settings| s.bloom = true,
            |s: &mut Settings| s.screen_indirect_intensity = 2.0,
        ] {
            Preset::Deluxe.apply(&mut s);
            change(&mut s);
            assert_eq!(Preset::matching(&s), None);
        }
        Preset::Deluxe.apply(&mut s);
        s.bloom_strength = 0.8;
        assert_eq!(Preset::matching(&s), Some(Preset::Deluxe));
        // on, the strength the slider shows reads as its own value
        s.screen_indirect_intensity = 1.5000001;
        assert_eq!(Preset::matching(&s), Some(Preset::Deluxe));
    }

    #[test]
    fn a_profile_on_the_deluxe_of_version_18_takes_the_effects_and_no_other() {
        let mut s = Settings::default();
        deluxe_v18().write(&mut s);
        assert_eq!(Preset::matching(&s), None);
        upgrade_deluxe_effects(&mut s);
        assert_eq!(Preset::matching(&s), Some(Preset::Deluxe));
        // straight to the Deluxe of now: the experimental bloom stays off
        assert!(s.contact_shadows && !s.bloom && s.screen_indirect_light);
        // a custom profile close to it keeps its effects off
        let mut custom = Settings::default();
        deluxe_v18().write(&mut custom);
        custom.max_particles = 4096;
        upgrade_deluxe_effects(&mut custom);
        assert!(!custom.bloom && !custom.contact_shadows && !custom.screen_indirect_light);
        // one effect chosen by hand on the old Deluxe: custom, kept as is
        let mut chosen = Settings::default();
        deluxe_v18().write(&mut chosen);
        chosen.bloom = true;
        upgrade_deluxe_effects(&mut chosen);
        assert!(chosen.bloom && !chosen.contact_shadows && !chosen.screen_indirect_light);
        // the other levels are untouched
        for p in [Preset::Potato, Preset::Low, Preset::Medium, Preset::High] {
            let mut s = Settings::default();
            p.apply(&mut s);
            let before = Values::read(&s);
            upgrade_deluxe_effects(&mut s);
            assert_eq!(Values::read(&s), before, "{p:?}");
        }
    }

    #[test]
    fn a_profile_on_the_deluxe_of_version_19_turns_the_experimental_effects_off() {
        let mut s = Settings::default();
        deluxe_v19().write(&mut s);
        assert!(s.bloom && s.contact_shadows && s.screen_indirect_light);
        assert_eq!(Preset::matching(&s), None);
        upgrade_deluxe_experimental(&mut s);
        assert_eq!(Preset::matching(&s), Some(Preset::Deluxe));
        assert!(s.contact_shadows && !s.bloom && s.screen_indirect_light);
        // a custom profile close to it keeps its bloom and indirect light
        let mut custom = Settings::default();
        deluxe_v19().write(&mut custom);
        custom.max_particles = 4096;
        upgrade_deluxe_experimental(&mut custom);
        assert!(custom.bloom && custom.screen_indirect_light);
        // that Deluxe with another bloom strength chosen by hand: kept
        let mut chosen = Settings::default();
        deluxe_v19().write(&mut chosen);
        chosen.bloom_strength = 0.8;
        upgrade_deluxe_experimental(&mut chosen);
        assert!(chosen.bloom && chosen.screen_indirect_light);
        // the levels of now are untouched
        for p in Preset::ALL {
            let mut s = Settings::default();
            p.apply(&mut s);
            let before = Values::read(&s);
            upgrade_deluxe_experimental(&mut s);
            assert_eq!(Values::read(&s), before, "{p:?}");
        }
    }

    #[test]
    fn a_profile_on_the_former_high_follows_the_high_of_now_once() {
        // exactly on the High of version 19 (Legacy occlusion): to the High
        // of now
        let mut old = Settings::default();
        high_v19().write(&mut old);
        assert_eq!(Preset::matching(&old), None);
        upgrade_high_gtao(&mut old);
        assert_eq!(Preset::matching(&old), Some(Preset::High));
        assert_eq!((old.ao_method, old.gtao_quality), (1, 2));
        assert!(old.contact_shadows);
        // a High customized by hand keeps its Legacy occlusion
        let mut custom = Settings::default();
        high_v19().write(&mut custom);
        custom.draw_distance = 192.0;
        upgrade_high_gtao(&mut custom);
        assert_eq!(custom.ao_method, 0);
        // the levels of now are untouched
        for p in Preset::ALL {
            let mut s = Settings::default();
            p.apply(&mut s);
            let before = Values::read(&s);
            upgrade_high_gtao(&mut s);
            assert_eq!(Values::read(&s), before, "{p:?}");
        }
    }

    #[test]
    fn profiles_on_the_levels_of_version_20_follow_the_levels_of_now_once() {
        for (former, preset) in [
            (medium_v20(), Preset::Medium),
            (high_v20(), Preset::High),
            (deluxe_v20(), Preset::Deluxe),
        ] {
            let mut s = Settings::default();
            former.write(&mut s);
            assert_eq!(Preset::matching(&s), None, "{preset:?}");
            upgrade_levels_v21(&mut s);
            assert_eq!(Preset::matching(&s), Some(preset));
            // customized on that level: kept
            let mut custom = Settings::default();
            former.write(&mut custom);
            custom.max_particles = 1024;
            let before = Values::read(&custom);
            upgrade_levels_v21(&mut custom);
            assert_eq!(Values::read(&custom), before, "{preset:?}");
        }
        // Potato and Low, and the levels of now, are untouched
        for p in Preset::ALL {
            let mut s = Settings::default();
            p.apply(&mut s);
            let before = Values::read(&s);
            upgrade_levels_v21(&mut s);
            assert_eq!(Values::read(&s), before, "{p:?}");
        }
    }

    #[test]
    fn demo_names() {
        assert_eq!(Preset::from_name("Potato"), Some(Preset::Potato));
        assert_eq!(Preset::from_name(" deluxe "), Some(Preset::Deluxe));
        assert_eq!(Preset::from_name("ultra"), None);
    }
}
