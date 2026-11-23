//! Named quality snapshots, following LLPresetsManager::savePreset/loadPreset
//! (indra/newview/llpresetsmanager.cpp). Only an explicit set of rendering
//! controls is stored: never account data, environment, hardware selection,
//! frame pacing or other preferences. The library lives in settings.json.

use crate::settings::Settings;

macro_rules! quality_fields {
    ($($field:ident: $ty:ty),+ $(,)?) => {
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(default)]
        pub struct Saved {
            pub quality: super::Values,
            // Missing in older profiles: import their original shadow tier when applied.
            #[serde(default)]
            pub three_level_shadow_distance: u16,
            // Old named profiles omitted the Bend controls. Loading one preserves
            // the current values; new named profiles round-trip them together.
            #[serde(default)]
            bend: Option<BendSettings>,
            #[serde(default)]
            pub shadow_profile: Option<crate::settings::shadow_profile::Profile>,
            $(pub $field: $ty,)+
        }

        impl Default for Saved {
            fn default() -> Self {
                Self::read(&Settings::default())
            }
        }

        impl Saved {
            #[allow(clippy::clone_on_copy)] // The allowlist mixes scalar and structured settings.
            pub fn read(s: &Settings) -> Self {
                Self::with_quality(s, super::Values::read(s), s.shadows)
            }

            fn with_quality(s: &Settings, quality: super::Values, shadows: bool) -> Self {
                let mut saved = Self {
                    shadow_profile: Some(crate::settings::shadow_profile::captured(s, &quality, shadows)),
                    quality,
                    three_level_shadow_distance: s.three_level_shadow_reach(),
                    bend: Some(BendSettings::read(s)),
                    $($field: s.$field.clone(),)+
                };
                saved.shadows = shadows;
                if s.three_level_shadow_distance == 0 {
                    let tier = if quality.shadow_quality == 0 { quality.shadow_resume_quality } else { quality.shadow_quality };
                    saved.three_level_shadow_distance = Settings::shadow_params_of(tier.clamp(1, 4)).1 as u16;
                }
                saved
            }

            #[allow(clippy::clone_on_copy)]
            fn write(&self, s: &mut Settings) {
                self.quality.write(s);
                s.three_level_shadow_distance = self.three_level_shadow_distance;
                if let Some(bend) = &self.bend { bend.write(s); }
                s.shadow_profile = self.shadow_profile.clone();
                $(s.$field = self.$field.clone();)+
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct BendSettings {
    mode: u8,
    reach_metres: f32,
    occluder_depth_metres: f32,
}

impl BendSettings {
    fn read(s: &Settings) -> Self {
        Self {
            mode: s.experimental_contact_reach_mode,
            reach_metres: s.experimental_contact_reach_metres,
            occluder_depth_metres: s.experimental_contact_occluder_depth_metres,
        }
    }

    fn write(&self, s: &mut Settings) {
        s.experimental_contact_reach_mode = self.mode;
        s.experimental_contact_reach_metres = self.reach_metres;
        s.experimental_contact_occluder_depth_metres = self.occluder_depth_metres;
    }
}

quality_fields! {
    max_attachment_area: f32,
    avatar_physics: bool,
    shadows: bool,
    contact_ray_method: u8,
    contact_view_reach: f32,
    contact_view_thickness: f32,
    contact_view_budget: u32,
    contact_view_debug: bool,
    shadow_visibility_debug: u8,
    experimental_contact_mask_debug: bool,
    experimental_contact_hit_debug: bool,
    experimental_contact_failure_debug: bool,
    graded_sun_shadows: bool,
    graded_shadow_edge_smoothing: bool,
    sun_shadow_contact_hardening: bool,
    sun_shadow_angular_diameter_deg: f32,
    graded_shadow_detail_metres: u8,
    graded_shadow_distance: u16,
    near_shadow_detail: bool,
    near_shadow_detail_metres: u8,
    near_shadow_precision: bool,
    near_shadow_cascade_percent: u8,
    near_shadow_debug: bool,
    contact_integrity_depth: bool,
    contact_integrity_hole_repair: bool,
    contact_integrity_composite: bool,
    exposure: f32,
    water_effects: crate::settings::WaterEffectsSettings,
    specular_aa: bool,
    legacy_light_harmonization: bool,
    harmonize_sunlight: bool,
    harmonize_ambient: bool,
    harmonize_local_lights: bool,
    colored_sky_ambient: bool,
    tonemapper: u8,
    tonemap_mix: f32,
    dof: crate::dof::Settings,
    occlusion_culling: bool,
    // FSR's mode and sharpening, with the anti-aliasing of `quality`: the
    // levels never choose FSR, a saved profile can
    fsr_quality: u8,
    fsr_sharpness: u32,
}

impl Saved {
    /// Photo quality temporarily raises rendering controls. Save the user's
    /// controls, just as Settings::save does, rather than those temporary ones.
    pub fn capture(s: &Settings) -> Self {
        if let Some(held) = &s.photo_held {
            let (quality, shadows) = held.user_values(s);
            return Self::with_quality(s, quality, shadows);
        }
        Self::read(s)
    }

    /// Old profiles have no editing history and may omit derived shadow values.
    /// Compare their effective inputs without rewriting their stored contents.
    pub fn matches_current(&self, s: &Settings) -> bool {
        let mut current = Self::capture(s);
        if self
            .shadow_profile
            .as_ref()
            .is_some_and(|p| p.saved_needs_upgrade(&self.quality, self.sun_shadow_contact_hardening))
        {
            return self.resolved(s) == current;
        }
        if self.shadow_profile.is_none() {
            let imported = crate::settings::shadow_profile::imported_profile(s, &current.quality, current.shadows);
            if current.shadow_profile.as_ref() == Some(&imported) {
                current.shadow_profile = None;
            }
            if self.bend.is_none() {
                current.bend = None;
            }
            if self.quality.shadow_map_resolution == 0 {
                current.quality.shadow_map_resolution = 0;
            }
            if self.quality.shadow_resume_quality == 0 {
                current.quality.shadow_resume_quality = 0;
            }
            if self.three_level_shadow_distance == 0 {
                current.three_level_shadow_distance = 0;
            }
        }
        self == &current
    }

    pub(crate) fn import_legacy_sun(&mut self, beta: &serde_json::Value, stock_schema: bool) {
        let mut s = Settings::default();
        self.write(&mut s);
        if crate::settings::shadow_profile::import_upstream_controls(&mut s, beta, stock_schema) {
            *self = Self::read(&s);
        } else {
            self.sun_shadow_angular_diameter_deg = crate::settings::shadow_profile::imported_sun_diameter(&s);
        }
    }

    pub(crate) fn upgrade_shadow_history(&mut self, current: &Settings) {
        if self
            .shadow_profile
            .as_ref()
            .is_some_and(|p| p.saved_needs_upgrade(&self.quality, self.sun_shadow_contact_hardening))
        {
            *self = self.resolved(current);
        }
    }

    fn resolved(&self, s: &Settings) -> Self {
        // Treat persisted values as external data. Sanitize in an isolated
        // settings object, then copy only our allowlist into the live one.
        let mut quality = Settings::default();
        BendSettings::read(s).write(&mut quality);
        // Resolve a historical profile's zero resolution in its own sun mode.
        quality.graded_sun_shadows = self.graded_sun_shadows;
        self.write(&mut quality);
        quality.shadows &= quality.shadow_quality > 0;
        let mut quality = quality.sanitized();
        crate::settings::shadow_profile::loaded(&mut quality);
        Self::read(&quality)
    }

    pub fn apply(&self, s: &mut Settings) {
        self.resolved(s).write(s);
        s.shadow_advanced_open = false;
        crate::snapshot::quality::adopt_shadow_selection(s);
        crate::settings::autotune::Hold::chosen(s);
    }
}

/// Names are labels, never file paths. Keep them compact and single-line.
pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}

/// Selection is an identity, not a comparison: identical profiles stay distinct.
pub fn selected(s: &Settings) -> Option<&str> {
    s.graphics_saved_active
        .as_deref()
        .filter(|name| s.graphics_saved.contains_key(*name))
}

pub fn select(s: &mut Settings, name: &str) -> bool {
    let Some(saved) = s.graphics_saved.get(name).cloned() else {
        return false;
    };
    saved.apply(s);
    s.graphics_saved_active = Some(name.to_owned());
    true
}

pub fn save_selected(s: &mut Settings) -> bool {
    let Some(name) = selected(s).map(str::to_owned) else {
        return false;
    };
    s.graphics_saved.insert(name, Saved::capture(s));
    true
}

pub fn choices(s: &Settings) -> Vec<(String, bool)> {
    let selected = selected(s);
    s.graphics_saved
        .keys()
        .map(|name| (name.clone(), selected == Some(name.as_str())))
        .collect()
}

pub fn demo(s: &mut Settings) {
    use super::Preset;
    let mut sample = Settings::default();
    Preset::Potato.apply(&mut sample);
    s.graphics_saved.insert("My fast profile".into(), Saved::read(&sample));
    Preset::High.apply(&mut sample);
    sample.draw_distance = 160.0;
    let photo = Saved::read(&sample);
    s.graphics_saved.insert("My photo profile".into(), photo.clone());
    s.graphics_saved.insert("My photo profile (copy)".into(), photo);
    select(s, "My photo profile");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_presets::Preset;

    #[test]
    fn graphics_saved_roundtrip_restores_quality_without_other_preferences() {
        let mut s = Settings::default();
        Preset::Deluxe.apply(&mut s);
        s.draw_distance = 224.0;
        s.dof.enabled = true;
        s.water_effects.enabled = true;
        let saved = Saved::capture(&s);
        s.graphics_saved.insert("Photo / nuit 🌙".into(), saved.clone());
        let json = serde_json::to_string(&s).expect("serialize settings");
        let mut loaded: Settings = serde_json::from_str(&json).expect("load settings");
        Preset::Potato.apply(&mut loaded);
        loaded.fps_cap = false;
        loaded.fps_limit = 144;
        loaded.vsync = false;
        loaded.background_fps_limit = 7;
        loaded.time_of_day = 2;
        loaded.ui_scale = 1.5;
        loaded.graphics_saved["Photo / nuit 🌙"].clone().apply(&mut loaded);
        assert_eq!(Saved::read(&loaded), saved);
        assert_eq!((loaded.fps_cap, loaded.fps_limit, loaded.vsync), (false, 144, false));
        assert_eq!((loaded.background_fps_limit, loaded.time_of_day, loaded.ui_scale), (7, 2, 1.5));
        let snapshot = serde_json::to_value(&saved).expect("serialize quality snapshot");
        for unrelated in ["username", "camera", "time_of_day", "fps_cap", "ui_scale", "graphics_saved"] {
            assert!(snapshot.get(unrelated).is_none(), "profile excludes {unrelated}");
        }
        select(&mut loaded, "Photo / nuit 🌙");
        assert_eq!(choices(&loaded), vec![("Photo / nuit 🌙".into(), true)]);
        loaded.draw_distance += 1.0;
        assert!(choices(&loaded)[0].1, "editing keeps the selected identity");
        assert_eq!(loaded.graphics_saved.len(), 1);
        loaded.reset_keeping_account();
        assert_eq!(loaded.graphics_saved.len(), 1, "resetting preferences keeps the saved library");
        assert!(selected(&loaded).is_none(), "resetting preferences clears the selected identity");
    }

    #[test]
    fn graphics_saved_selection_distinguishes_identical_profiles_and_save_updates_only_the_selected_name() {
        let mut s = Settings::default();
        let original = Saved::capture(&s);
        s.graphics_saved.insert("Test".into(), original.clone());
        s.graphics_saved.insert("Test 2".into(), original.clone());
        assert!(
            choices(&s).iter().all(|(_, active)| !active),
            "equal values do not select a profile"
        );
        assert!(!save_selected(&mut s), "no implicit overwrite target");
        for name in ["Test", "Test 2", "Test"] {
            assert!(select(&mut s, name));
            assert_eq!(choices(&s).iter().filter(|(_, active)| *active).count(), 1);
            assert_eq!(selected(&s), Some(name));
        }
        s.draw_distance = 224.0;
        assert_eq!(selected(&s), Some("Test"));
        assert!(save_selected(&mut s));
        assert_eq!(s.graphics_saved["Test"].quality.draw_distance, 224.0);
        assert_eq!(s.graphics_saved["Test 2"], original);
        let json = serde_json::to_string(&s).expect("serialize selected identity");
        let mut restored: Settings = serde_json::from_str(&json).expect("restore selected identity");
        assert_eq!(selected(&restored), Some("Test"));
        assert!(!select(&mut restored, "Missing"));
        assert_eq!(selected(&restored), Some("Test"));
        Preset::High.apply(&mut restored);
        assert!(selected(&restored).is_none(), "choosing a built-in level leaves the saved profile");
        restored.graphics_saved_active = Some("Missing".into());
        assert!(restored.sanitized().graphics_saved_active.is_none());
    }

    #[test]
    fn graphics_saved_sanitizes_external_values_and_supports_older_entries() {
        let mut saved: Saved = serde_json::from_str("{}").expect("partial saved quality");
        saved.quality.draw_distance = crate::settings::DRAW_DISTANCE_MAX;
        let mut s = Settings::default();
        saved.apply(&mut s);
        assert_eq!(s.draw_distance, crate::settings::DRAW_DISTANCE_MAX);
        saved.quality.draw_distance = 99_999.0;
        saved.quality.shadow_quality = 255;
        saved.quality.antialiasing = 255;
        saved.dof.f_number = f32::NAN;
        saved.apply(&mut s);
        assert_eq!(s.draw_distance, crate::settings::DRAW_DISTANCE_MAX);
        assert!(s.shadow_quality <= 4);
        assert!(s.antialiasing <= 7);
        assert!(s.dof.f_number.is_finite());
        assert_eq!(s.graphics_saved.len(), 0);
        assert!(valid_name(" Photo / nuit 🌙 "));
        assert!(!valid_name("  "));
        assert!(!valid_name("two\nlines"));
        assert!(!valid_name(&"a".repeat(65)));
    }

    #[test]
    fn graphics_saved_keeps_the_fsr_mode_and_older_entries_take_the_defaults() {
        let mut s = Settings {
            antialiasing: 7,
            fsr_quality: 3,
            fsr_sharpness: 25,
            ..Settings::default()
        };
        let saved = Saved::capture(&s);
        let json = serde_json::to_value(&saved).expect("serialize the profile");
        s.antialiasing = 6;
        s.fsr_quality = 0;
        s.fsr_sharpness = 100;
        saved.apply(&mut s);
        assert_eq!((s.antialiasing, s.fsr_quality, s.fsr_sharpness), (7, 3, 25));
        let mut old = json;
        let fields = old.as_object_mut().expect("profile object");
        fields.remove("fsr_quality");
        fields.remove("fsr_sharpness");
        let old: Saved = serde_json::from_value(old).expect("profile saved before FSR");
        old.apply(&mut s);
        let d = Settings::default();
        assert_eq!(
            (s.antialiasing, s.fsr_quality, s.fsr_sharpness),
            (7, d.fsr_quality, d.fsr_sharpness)
        );
        let mut bad: Saved = serde_json::from_str("{}").expect("partial profile");
        bad.fsr_quality = 200;
        bad.fsr_sharpness = 9_999;
        bad.apply(&mut s);
        assert!(s.fsr_quality <= 4 && s.fsr_sharpness <= 100);
    }

    /// LLPresetsManager saves RenderGlow with a graphics preset
    /// (app_settings/graphic_preset_controls.xml) and loading it puts it back,
    /// where a level leaves it alone (#2388).
    #[test]
    fn graphics_saved_restores_the_glow_it_was_saved_with() {
        let mut s = Settings::default();
        Preset::High.apply(&mut s);
        s.glow = false;
        s.graphics_saved.insert("No glow".into(), Saved::capture(&s));
        s.glow = true;
        s.graphics_saved.insert("Glow".into(), Saved::capture(&s));
        assert!(select(&mut s, "No glow"));
        assert!(!s.glow);
        Preset::Deluxe.apply(&mut s);
        assert!(!s.glow, "a level after the profile keeps its glow");
        assert!(select(&mut s, "Glow"));
        assert!(s.glow);
    }

    #[test]
    fn graphics_saved_captures_user_quality_while_photo_boost_is_active() {
        let mut s = Settings::default();
        Preset::Low.apply(&mut s);
        let user = Saved::read(&s);
        s.photo_held = Some(crate::snapshot::quality::boost(&mut s));
        assert_ne!(Saved::read(&s), user);
        assert_eq!(Saved::capture(&s), user);
        s.graphics_saved.insert("My quality".into(), user);
        s.graphics_saved_active = Some("My quality".into());
        assert!(choices(&s)[0].1);
    }

    #[test]
    fn each_named_profile_keeps_its_own_dormant_shadow_edits() {
        use crate::settings::shadow_profile as shadow;
        let mut s = Settings::default();
        shadow::reset_all(&mut s);
        shadow::set_custom_enabled(&mut s, true);
        shadow::set_advanced_enabled(&mut s, true);
        s.graded_shadow_distance = 151;
        shadow::commit_edits(&mut s);
        super::super::Preset::Deluxe.apply(&mut s);
        assert_eq!(shadow::tier(&s), shadow::Tier::Ultra);
        assert!(!shadow::custom_enabled(&s));
        let first = Saved::capture(&s);
        shadow::set_custom_enabled(&mut s, true);
        assert_eq!(shadow::tier(&s), shadow::Tier::Ultra);
        assert_ne!(s.graded_shadow_distance, 151);
        s.graded_shadow_distance = 207;
        shadow::commit_edits(&mut s);
        let second = Saved::capture(&s);
        let first: Saved = serde_json::from_value(serde_json::to_value(first).unwrap()).unwrap();
        first.apply(&mut s);
        assert!(!shadow::custom_enabled(&s));
        shadow::set_custom_enabled(&mut s, true);
        assert!(!shadow::can_reset(&s));
        shadow::select_tier(&mut s, shadow::Tier::High);
        assert_eq!(s.graded_shadow_distance, 151);
        second.apply(&mut s);
        assert!(shadow::custom_enabled(&s));
        assert_eq!(s.graded_shadow_distance, 207);
        shadow::set_advanced_enabled(&mut s, true);
        assert!(second.matches_current(&s));
        second.apply(&mut s);
        assert!(!shadow::advanced_enabled(&s));
        shadow::select_tier(&mut s, shadow::Tier::High);
        assert_eq!(s.graded_shadow_distance, 151);
    }

    #[test]
    fn v236_named_preset_migrates_fixed_and_custom_history_without_dirtying_its_selection() {
        use crate::settings::shadow_profile as shadow;
        let mut s = Settings::default();
        shadow::select_fixed_tier(&mut s, shadow::Tier::High);
        shadow::set_custom_enabled(&mut s, true);
        s.graded_shadow_distance = 157;
        shadow::commit_edits(&mut s);
        let custom = serde_json::to_value(Saved::capture(&s)).unwrap();
        for enabled in [false, true] {
            let mut wire = if enabled {
                custom.clone()
            } else {
                shadow::select_fixed_tier(&mut s, shadow::Tier::Low);
                serde_json::to_value(Saved::capture(&s)).unwrap()
            };
            wire["shadow_profile"] = serde_json::json!({
                "selected": "Low", "use_custom": enabled, "custom": {
                    "base": "High", "coverage": 1, "advanced": true, "override_three": false,
                    "override_four": true, "tuning": custom["shadow_profile"]["slots"]["High"]["tuning"],
                }
            });
            let mut old: Saved = serde_json::from_value(wire).unwrap();
            old.apply(&mut s);
            assert!(old.matches_current(&s));
            old.upgrade_shadow_history(&s);
            assert!(old.matches_current(&s));
            let repeat: Saved = serde_json::from_value(serde_json::to_value(&old).unwrap()).unwrap();
            repeat.apply(&mut s);
            assert!(repeat.matches_current(&s) && !shadow::advanced_enabled(&s));
            assert_eq!(shadow::custom_enabled(&s), enabled);
            if !enabled {
                assert_eq!(shadow::tier(&s), shadow::Tier::Low);
                shadow::set_custom_enabled(&mut s, true);
                assert_eq!(shadow::tier(&s), shadow::Tier::Low);
                shadow::select_tier(&mut s, shadow::Tier::High);
            }
            assert_eq!((shadow::tier(&s), s.graded_shadow_distance), (shadow::Tier::High, 157));
        }
    }

    #[test]
    fn v41_rewrite_keeps_global_and_named_custom_history_and_imports_known_edits() {
        use crate::settings::shadow_profile as shadow;
        let mut s = Settings::default();
        shadow::reset_all(&mut s);
        shadow::set_custom_enabled(&mut s, true);
        shadow::set_advanced_enabled(&mut s, true);
        s.graded_shadow_distance = 173;
        s.experimental_contact_occluder_depth_metres = 0.13;
        shadow::commit_edits(&mut s);
        let named = Saved::capture(&s);
        shadow::select_fixed_tier(&mut s, shadow::Tier::Medium);
        let mut newer = serde_json::to_value(&s).unwrap();
        newer["settings_version"] = 43.into();
        newer["graphics_saved"] = serde_json::json!({"portrait": named});

        // A v41 run knows each flat field, neither shadow_profile object.
        let mut at_load = newer.clone();
        at_load["settings_version"] = 41.into();
        at_load.as_object_mut().unwrap().remove("shadow_profile");
        at_load["graphics_saved"]["portrait"]
            .as_object_mut()
            .unwrap()
            .remove("shadow_profile");
        let mut edits = at_load.clone();
        edits["graded_shadow_distance"] = 197.into();
        edits["graphics_saved"]["portrait"]["graded_shadow_distance"] = 163.into();
        let merged = crate::tolerant::merge_over_newer(&newer, edits, &at_load, &["settings_version".into()]);
        assert_eq!(merged["settings_version"], 43);
        assert_eq!(merged["shadow_profile"], newer["shadow_profile"]);
        assert_eq!(
            merged["graphics_saved"]["portrait"]["shadow_profile"],
            newer["graphics_saved"]["portrait"]["shadow_profile"]
        );

        let mut reopened: Settings = serde_json::from_value(merged.clone()).unwrap();
        shadow::loaded(&mut reopened);
        assert_eq!(shadow::tier(&reopened), shadow::Tier::Medium);
        assert!(shadow::custom_enabled(&reopened));
        assert_eq!(reopened.graded_shadow_distance, 197);
        shadow::select_tier(&mut reopened, shadow::Tier::High);
        assert_eq!(
            (reopened.graded_shadow_distance, reopened.experimental_contact_occluder_depth_metres),
            (173, 0.13)
        );
        let edited: Saved = serde_json::from_value(merged["graphics_saved"]["portrait"].clone()).unwrap();
        edited.apply(&mut reopened);
        assert!(shadow::custom_enabled(&reopened));
        assert_eq!(shadow::base_tier(&reopened), shadow::Tier::High);
        assert_eq!(reopened.graded_shadow_distance, 163);
    }

    #[test]
    fn old_named_profile_selection_stays_clean_until_custom_history_changes() {
        use crate::settings::shadow_profile as shadow;
        let mut s = Settings::default();
        let mut old = serde_json::to_value(Saved::capture(&s)).unwrap();
        let fields = old.as_object_mut().unwrap();
        for key in ["shadow_profile", "three_level_shadow_distance", "bend"] {
            fields.remove(key);
        }
        let quality = fields.get_mut("quality").unwrap().as_object_mut().unwrap();
        quality.remove("shadow_map_resolution");
        quality.remove("shadow_resume_quality");
        let old: Saved = serde_json::from_value(old).unwrap();
        old.apply(&mut s);
        assert!(old.matches_current(&s));
        shadow::select_tier(&mut s, shadow::Tier::Low);
        assert!(!old.matches_current(&s));
        let newer = Saved::capture(&s);
        assert!(newer.matches_current(&s));
        let enabled = shadow::custom_enabled(&s);
        shadow::set_custom_enabled(&mut s, !enabled);
        assert!(!newer.matches_current(&s));
    }

    #[test]
    fn shadow_visibility_graphics_profiles_round_trip_and_accept_legacy_fields() {
        use aurora_render::render_controls::{ContactDiagnostic, ContactDiagnosticSwitches};
        let mut settings = Settings::default();
        for mode in [
            ContactDiagnostic::Off,
            ContactDiagnostic::ShadowMapVisibility,
            ContactDiagnostic::ContactVisibility,
            ContactDiagnostic::CombinedVisibility,
            ContactDiagnostic::Cascades,
            ContactDiagnostic::Mask,
            ContactDiagnostic::BendCandidates,
            ContactDiagnostic::BendFailures,
            ContactDiagnostic::ViewRayOutcomes,
        ] {
            let switches = mode.switches();
            settings.shadow_visibility_debug = mode.visibility_setting();
            settings.experimental_contact_mask_debug = switches.mask;
            settings.experimental_contact_hit_debug = switches.candidates;
            settings.experimental_contact_failure_debug = switches.failures;
            settings.contact_view_debug = switches.view_ray;
            let saved = Saved::capture(&settings);
            let json = serde_json::to_value(&saved).expect("save shadow diagnostic");
            let saved: Saved = serde_json::from_value(json).expect("load shadow diagnostic");
            settings.shadow_visibility_debug = 0;
            settings.experimental_contact_mask_debug = true;
            settings.experimental_contact_hit_debug = true;
            settings.experimental_contact_failure_debug = true;
            settings.contact_view_debug = true;
            settings.near_shadow_debug = true;
            saved.apply(&mut settings);
            assert_eq!(
                ContactDiagnostic::from_saved(
                    settings.shadow_visibility_debug,
                    ContactDiagnosticSwitches {
                        mask: settings.experimental_contact_mask_debug,
                        candidates: settings.experimental_contact_hit_debug,
                        failures: settings.experimental_contact_failure_debug,
                        view_ray: settings.contact_view_debug,
                    },
                    settings.near_shadow_debug
                ),
                mode
            );
        }
        let old: Saved = serde_json::from_str(r#"{"contact_view_debug":true,"contact_integrity_composite":true}"#)
            .expect("profile without new selector");
        old.apply(&mut settings);
        assert_eq!(settings.shadow_visibility_debug, 0);
        assert!(settings.contact_view_debug && settings.contact_integrity_composite);
        let invalid = Saved {
            shadow_visibility_debug: 255,
            ..old
        };
        invalid.apply(&mut settings);
        assert_eq!(settings.shadow_visibility_debug, 0);
    }

    #[test]
    fn shadow_profile_capture_canonicalizes_old_defaults_and_preserves_off_tier() {
        let s = Settings {
            shadow_quality: 4,
            ..Settings::default()
        };
        let saved = Saved::capture(&s);
        let mut restored = Settings::default();
        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        saved.apply(&mut restored);
        assert_eq!(Saved::read(&restored), saved);
        assert_eq!(restored.shadow_params(), (4096, 256.0));

        let external: Settings = serde_json::from_value(serde_json::json!({"shadows":false,"shadow_quality":2})).unwrap();
        let low = external.sanitized();
        assert!(!low.shadows);
        assert_eq!(low.shadow_params(), (0, 0.0));
        assert_eq!(low.shadow_map_size(), 2048);
        assert_eq!(low.three_level_shadow_reach(), 96);
        let off = Saved::capture(&low);
        off.apply(&mut restored);
        restored.set_shadow_enabled(true);
        assert_eq!(restored.shadow_quality, 2);
        assert_eq!(restored.shadow_params(), (2048, 96.0));
    }

    #[test]
    fn historic_shadow_preferences_import_effective_sizes_and_reach() {
        for graded in [false, true] {
            for tier in 0..=4 {
                let json = serde_json::json!({
                    "graded_sun_shadows": graded,
                    "shadow_quality": tier,
                    "graded_shadow_distance": 144
                });
                let mut s: Settings = serde_json::from_value(json).unwrap();
                s = s.sanitized();
                let old_size = if graded {
                    Settings::graded_shadow_resolution(tier)
                } else {
                    Settings::shadow_params_of(tier).0
                };
                if tier == 0 {
                    assert!(!s.shadows);
                    assert_eq!(s.shadow_params(), (0, 0.0));
                    s.set_shadow_enabled(true);
                    assert_eq!(s.shadow_map_size(), 2048);
                } else {
                    let old_reach = Settings::shadow_params_of(tier).1;
                    assert_eq!(s.shadow_params(), (old_size, if graded { 144.0 } else { old_reach }));
                    s.graded_sun_shadows = !graded;
                    assert_eq!(s.shadow_params(), (old_size, if graded { old_reach } else { 144.0 }));
                    s.set_shadow_map_size(512);
                    assert_eq!(s.three_level_shadow_reach(), old_reach as u16);
                    assert_eq!(s.graded_shadow_distance, 144);
                    assert_eq!(s.shadow_map_size(), 512);
                }
            }
        }
    }

    #[test]
    fn shadow_master_and_named_profile_preserve_manual_controls() {
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "shadow_quality": 4, "graded_sun_shadows": true,
            "near_shadow_detail": true, "near_shadow_detail_metres": 12,
            "near_shadow_precision": true, "near_shadow_cascade_percent": 3,
            "projector_shadows": true, "projector_shadow_limit": 16, "contact_shadows": true,
            "contact_ray_method": 0, "experimental_contact_reach_mode": 3,
            "experimental_contact_occluder_depth_metres": 0.05
        }))
        .unwrap();
        s = s.sanitized();
        assert_eq!(s.shadow_params(), (4096, 192.0));
        assert_eq!(s.three_level_shadow_reach(), 256);
        assert!(!s.graded_shadow_edge_smoothing);
        s.three_level_shadow_distance = 80;
        s.graded_shadow_distance = 128;
        s.set_shadow_map_size(1024);
        s.graded_shadow_edge_smoothing = true;
        s.set_shadow_enabled(false);
        let saved: Saved = serde_json::from_slice(&serde_json::to_vec(&Saved::capture(&s)).unwrap()).unwrap();
        let mut restored = Settings::default();
        saved.apply(&mut restored);
        let mut restored: Settings = serde_json::from_slice(&serde_json::to_vec(&restored).unwrap()).unwrap();
        restored = restored.sanitized();
        assert_eq!(restored.shadow_params(), (0, 0.0));
        assert_eq!(restored.shadow_map_size(), 1024);
        restored.set_shadow_enabled(true);
        assert_eq!(restored.shadow_params(), (1024, 128.0));
        assert_eq!(restored.shadow_quality, 2);
        restored.graded_sun_shadows = false;
        assert_eq!(restored.shadow_params(), (1024, 80.0));
        assert!(restored.graded_shadow_edge_smoothing && restored.contact_shadows && restored.projector_shadows);
        assert_eq!(
            (
                restored.projector_shadow_limit,
                restored.contact_ray_method,
                restored.experimental_contact_reach_mode
            ),
            (16, 0, 3)
        );
        assert_eq!(restored.experimental_contact_occluder_depth_metres, 0.05);
        assert_eq!((restored.near_shadow_detail_metres, restored.near_shadow_cascade_percent), (12, 3));

        // A profile saved before Bend settings were captured must keep the user's live Bend values.
        let mut old = serde_json::to_value(&saved).unwrap();
        let entry = old.as_object_mut().unwrap();
        entry.remove("bend");
        entry.remove("three_level_shadow_distance");
        entry.remove("graded_shadow_edge_smoothing");
        let mut quality = entry["quality"].as_object().unwrap().clone();
        quality.remove("shadow_map_resolution");
        quality.remove("shadow_resume_quality");
        entry.insert("quality".into(), serde_json::Value::Object(quality));
        let old: Saved = serde_json::from_value(old).unwrap();
        restored.experimental_contact_reach_mode = 1;
        restored.experimental_contact_occluder_depth_metres = 0.11;
        old.apply(&mut restored);
        assert_eq!(restored.experimental_contact_reach_mode, 1);
        assert_eq!(restored.experimental_contact_occluder_depth_metres, 0.11);
    }

    #[test]
    fn graded_shadow_profiles_preserve_legacy_and_decouple_reach_from_resolution() {
        let mut s = Settings::default();
        let old: Saved = serde_json::from_str("{}").unwrap();
        old.apply(&mut s);
        assert!(!s.graded_sun_shadows);
        assert_eq!(s.graded_shadow_detail_metres, 2);
        assert_eq!(s.graded_shadow_distance, 192);
        s.graded_sun_shadows = true;
        s.graded_shadow_distance = 96;
        s.near_shadow_cascade_percent = 3;
        for size in [512u32, 1024, 2048, 4096] {
            s.set_shadow_map_size(size as u16);
            assert_eq!(s.shadow_params(), (size, 96.0));
            assert_eq!(s.render_settings().shadow_resolution, size);
        }
        let mut p: Saved = serde_json::from_slice(&serde_json::to_vec(&Saved::capture(&s)).unwrap()).unwrap();
        p.graded_shadow_distance = u16::MAX;
        p.graded_shadow_detail_metres = u8::MAX;
        p.apply(&mut s);
        assert!(s.render_settings().graded_sun_shadows);
        assert_eq!(s.shadow_params(), (4096, 512.0));
        assert_eq!(s.render_settings().graded_shadow_detail_metres, 12);
        assert_eq!(s.near_shadow_cascade_percent, 3);
        s.shadow_quality = 0;
        assert_eq!(s.shadow_params(), (0, 0.0));
        s.set_shadow_enabled(true);
        s.set_shadow_map_size(2048);
        s.graded_sun_shadows = false;
        assert_eq!(s.shadow_params(), Settings::shadow_params_of(3));
    }

    #[test]
    fn near_shadow_detail_presets_round_trip_and_keep_old_profiles_off() {
        let old: Saved = serde_json::from_str(r#"{"contact_view_debug":true}"#).expect("profile predating near detail");
        assert!(!old.near_shadow_detail);
        assert_eq!(old.near_shadow_detail_metres, 4);
        let mut settings = Settings {
            near_shadow_detail: true,
            near_shadow_detail_metres: 6,
            contact_shadows: true,
            contact_ray_method: 1,
            ..Settings::default()
        };
        let saved = Saved::capture(&settings);
        let json = serde_json::to_string(&saved).expect("save near detail");
        let saved: Saved = serde_json::from_str(&json).expect("load near detail");
        settings.near_shadow_detail = false;
        settings.near_shadow_detail_metres = 4;
        saved.apply(&mut settings);
        assert!(settings.near_shadow_detail);
        assert_eq!(settings.near_shadow_detail_metres, 6);
        assert!(settings.contact_shadows);
        assert_eq!(settings.contact_ray_method, 1);
        assert!(settings.render_settings().near_shadow_detail);
        assert_eq!(settings.render_settings().near_shadow_detail_metres, 6);

        for (external, expected) in [(0, 2), (1, 2), (2, 2), (12, 12), (255, 12)] {
            let invalid = Saved {
                near_shadow_detail_metres: external,
                ..saved.clone()
            };
            invalid.apply(&mut settings);
            assert_eq!(settings.near_shadow_detail_metres, expected);
            assert_eq!(settings.render_settings().near_shadow_detail_metres, expected);
        }
        old.apply(&mut settings);
        assert!(!settings.near_shadow_detail);
        assert_eq!(settings.near_shadow_detail_metres, 4);
        assert!(settings.contact_view_debug);
    }
}
