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
                Self { quality: super::Values::read(s), $($field: s.$field.clone(),)+ }
            }

            #[allow(clippy::clone_on_copy)]
            fn write(&self, s: &mut Settings) {
                self.quality.write(s);
                $(s.$field = self.$field.clone();)+
            }
        }
    };
}

quality_fields! {
    max_attachment_area: f32,
    avatar_physics: bool,
    shadows: bool,
    exposure: f32,
    water_effects: crate::settings::WaterEffectsSettings,
    specular_aa: bool,
    colored_sky_ambient: bool,
    tonemapper: u8,
    tonemap_mix: f32,
    dof: crate::dof::Settings,
    occlusion_culling: bool,
}

impl Saved {
    /// Photo quality temporarily raises rendering controls. Save the user's
    /// controls, just as Settings::save does, rather than those temporary ones.
    pub fn capture(s: &Settings) -> Self {
        let mut saved = Self::read(s);
        if let Some(held) = &s.photo_held {
            (saved.quality, saved.shadows) = held.user_values(s);
        }
        saved
    }

    pub fn apply(&self, s: &mut Settings) {
        // Treat persisted values as external data. Sanitize in an isolated
        // settings object, then copy only our allowlist into the live one.
        let mut quality = Settings::default();
        self.write(&mut quality);
        quality.shadows = quality.shadow_quality > 0;
        Self::read(&quality.sanitized()).write(s);
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
        saved.quality.draw_distance = 99_999.0;
        saved.quality.shadow_quality = 255;
        saved.quality.antialiasing = 255;
        saved.dof.f_number = f32::NAN;
        let mut s = Settings::default();
        saved.apply(&mut s);
        assert!(s.draw_distance <= 512.0);
        assert!(s.shadow_quality <= 4);
        assert!(s.antialiasing <= 6);
        assert!(s.dof.f_number.is_finite());
        assert_eq!(s.graphics_saved.len(), 0);
        assert!(valid_name(" Photo / nuit 🌙 "));
        assert!(!valid_name("  "));
        assert!(!valid_name("two\nlines"));
        assert!(!valid_name(&"a".repeat(65)));
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
}
