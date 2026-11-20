//! The active flare and user presets are local viewer preferences.

use std::collections::BTreeMap;

use aurora_render::lens_flare::{Frame, LensFlare, Quality};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(remote = "Quality", rename_all = "snake_case")]
enum StoredQuality {
    Performance,
    High,
    #[serde(other)]
    Balanced,
}

#[derive(Deserialize, Serialize)]
#[serde(remote = "LensFlare")]
struct StoredValues {
    intensity: f32,
    threshold: f32,
    knee: f32,
    #[serde(default)]
    blur: f32,
    #[serde(with = "StoredQuality")]
    quality: Quality,
    tint: [f32; 3],
    ghosts: f32,
    ghost_count: u32,
    ghost_spread: f32,
    ghost_softness: f32,
    halo: f32,
    halo_radius: f32,
    halo_softness: f32,
    streaks: f32,
    streak_length: f32,
    streak_angle: f32,
    starburst: f32,
    star_length: f32,
    star_angle: f32,
    star_rays: u32,
    chromatic: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SavedPreset(#[serde(with = "StoredValues")] LensFlare);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltIn {
    #[default]
    SubtleOptical,
    GoldenHour,
    BlueAnamorphic,
    StarFilter,
}

impl BuiltIn {
    pub const ALL: [Self; 4] = [Self::SubtleOptical, Self::GoldenHour, Self::BlueAnamorphic, Self::StarFilter];

    pub fn label(self) -> &'static str {
        match self {
            Self::SubtleOptical => aurora_i18n::tr!("lens-flare-subtle"),
            Self::GoldenHour => aurora_i18n::tr!("lens-flare-golden"),
            Self::BlueAnamorphic => aurora_i18n::tr!("lens-flare-blue"),
            Self::StarFilter => aurora_i18n::tr!("lens-flare-star"),
        }
    }

    pub fn values(self) -> LensFlare {
        let base = LensFlare::default();
        match self {
            Self::SubtleOptical => base,
            Self::GoldenHour => LensFlare {
                intensity: 0.7,
                threshold: 1.15,
                tint: [1.0, 0.78, 0.50],
                ghosts: 0.48,
                ghost_count: 4,
                ghost_spread: 0.8,
                halo: 0.44,
                halo_radius: 0.35,
                halo_softness: 0.65,
                chromatic: 0.24,
                ..base
            },
            Self::BlueAnamorphic => LensFlare {
                intensity: 1.15,
                threshold: 1.2,
                tint: [0.45, 0.71, 1.0],
                ghosts: 0.20,
                ghost_count: 2,
                halo: 0.0,
                streaks: 1.0,
                streak_length: 0.86,
                chromatic: 0.18,
                ..base
            },
            Self::StarFilter => LensFlare {
                intensity: 1.0,
                threshold: 1.25,
                ghosts: 0.0,
                halo: 0.06,
                starburst: 1.0,
                star_length: 0.52,
                star_angle: 15.0,
                chromatic: 0.0,
                ..base
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selection {
    BuiltIn(BuiltIn),
    Named(String),
    Custom,
}

impl Default for Selection {
    fn default() -> Self {
        Self::BuiltIn(BuiltIn::default())
    }
}

impl Selection {
    pub fn label(&self) -> &str {
        match self {
            Self::BuiltIn(built_in) => built_in.label(),
            Self::Named(name) => name,
            Self::Custom => aurora_i18n::tr!("lens-flare-custom"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    #[serde(with = "StoredValues")]
    pub current: LensFlare,
    pub selection: Selection,
    pub named: BTreeMap<String, SavedPreset>,
}

impl Settings {
    pub fn sanitize(&mut self) {
        self.current = self.current.sanitized();
        self.named.retain(|name, values| {
            values.0 = values.0.sanitized();
            crate::graphics_presets::saved::valid_name(name)
        });
        if matches!(&self.selection, Selection::Named(name) if !self.named.contains_key(name)) {
            self.selection = Selection::Custom;
        }
    }

    pub fn frame(&self, photo_size: Option<(u32, u32)>) -> Option<Frame> {
        let settings = self.enabled.then(|| self.current.sanitized())?;
        settings.active().then_some(Frame {
            settings,
            photo_size,
            last_context_tile: false,
        })
    }

    fn stored(&self, selection: &Selection) -> Option<LensFlare> {
        match selection {
            Selection::BuiltIn(preset) => Some(preset.values()),
            Selection::Named(name) => self.named.get(name).map(|values| values.0),
            Selection::Custom => None,
        }
    }

    pub fn select(&mut self, selection: Selection) {
        if let Some(values) = self.stored(&selection) {
            self.current = values;
            self.selection = selection;
        }
    }

    pub fn modified(&self) -> bool {
        self.stored(&self.selection).is_some_and(|saved| saved != self.current)
    }

    pub fn revert(&mut self) {
        if let Some(values) = self.stored(&self.selection) {
            self.current = values;
        }
    }

    pub fn available_name(&self, name: &str) -> bool {
        crate::graphics_presets::saved::valid_name(name) && !self.named.contains_key(name.trim())
    }

    pub fn save_as(&mut self, name: &str) {
        if self.available_name(name) {
            let name = name.trim().to_owned();
            self.named.insert(name.clone(), SavedPreset(self.current));
            self.selection = Selection::Named(name);
        }
    }

    pub fn update(&mut self) {
        if let Selection::Named(name) = &self.selection
            && let Some(saved) = self.named.get_mut(name)
        {
            saved.0 = self.current;
        }
    }

    pub fn rename(&mut self, new_name: &str) {
        if self.available_name(new_name)
            && let Selection::Named(name) = &self.selection
            && let Some(saved) = self.named.remove(name)
        {
            let new_name = new_name.trim().to_owned();
            self.named.insert(new_name.clone(), saved);
            self.selection = Selection::Named(new_name);
        }
    }

    pub fn delete(&mut self) {
        if let Selection::Named(name) = &self.selection {
            self.named.remove(name);
            self.selection = Selection::Custom;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_default_off_and_presets_are_independent() {
        let old: crate::settings::Settings = serde_json::from_str("{}").expect("old settings");
        assert!(!old.lens_flare.enabled);
        assert!(old.lens_flare.frame(None).is_none());
        for (i, preset) in BuiltIn::ALL.iter().enumerate() {
            assert_eq!(preset.values().quality, Quality::Balanced);
            assert!(BuiltIn::ALL[..i].iter().all(|other| other.values() != preset.values()));
        }
    }

    #[test]
    fn named_presets_and_modified_values_survive_restart() {
        let mut flare = Settings {
            enabled: true,
            ..Settings::default()
        };
        flare.current.quality = Quality::High;
        flare.current.blur = 0.6;
        flare.save_as("  Portrait  ");
        flare.current.threshold = 3.0;
        flare.save_as("Portrait");
        flare.rename("Studio");
        assert!(flare.modified());
        let file = serde_json::to_string(&flare).expect("serialize flare");
        let mut read: Settings = serde_json::from_str(&file).expect("reload flare");
        assert_eq!(read, flare);
        read.revert();
        assert_eq!(read.current.threshold, LensFlare::default().threshold);
        read.current.threshold = 4.0;
        read.update();
        assert!(!read.modified());
        read.select(Selection::BuiltIn(BuiltIn::GoldenHour));
        read.select(Selection::Named("Studio".into()));
        assert_eq!(read.current.quality, Quality::High);
        assert_eq!(read.current.blur, 0.6);
        assert_eq!(read.current.threshold, 4.0);
        read.delete();
        assert_eq!(read.selection, Selection::Custom);
        assert_eq!(read.current.threshold, 4.0);
        assert!(read.named.is_empty());
    }

    #[test]
    fn lens_flare_presets_without_blur_keep_their_values() {
        let mut flare = Settings::default();
        flare.current.threshold = 6.0;
        flare.save_as("Older preset");
        let mut saved = serde_json::to_value(&flare).expect("serialize flare");
        saved["current"].as_object_mut().expect("current").remove("blur");
        saved["named"]["Older preset"].as_object_mut().expect("preset").remove("blur");
        let read: Settings = serde_json::from_value(saved).expect("read older settings");
        assert_eq!(read, flare);
    }

    #[test]
    fn bad_numbers_are_sanitized_in_active_and_saved_values() {
        let mut flare = Settings::default();
        flare.current.threshold = f32::NAN;
        flare.current.tint = [-1.0, f32::INFINITY, 0.2];
        flare.current.ghost_count = u32::MAX;
        flare.current.star_rays = 5;
        flare.save_as("Broken");
        flare.sanitize();
        assert_eq!(flare.current.threshold, LensFlare::default().threshold);
        assert_eq!(flare.current.tint, [0.0, 1.0, 0.2]);
        assert_eq!(flare.current.ghost_count, 6);
        assert_eq!(flare.current.star_rays, 6);
        assert_eq!(flare.named.get("Broken"), Some(&SavedPreset(flare.current)));
    }
}
