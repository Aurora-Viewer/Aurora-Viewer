//! LLFloaterScriptEdPrefs / app_settings/settings.xml (Firestorm, LGPL 2.1).
//! Defaults follow Firestorm's options; color defaults follow Aurora's skin.

use crate::theme::Palette;
use egui::Color32;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ColorRole {
    Text,
    Cursor,
    Background,
    Selection,
    Section,
    Type,
    Event,
    Comment,
    Function,
    String,
    Constant,
    Integer,
    Float,
    StringConstant,
    Compound,
    Control,
    Preprocessor,
    Variable,
}

impl ColorRole {
    pub const ALL: [Self; 18] = [
        Self::Text,
        Self::Cursor,
        Self::Background,
        Self::Selection,
        Self::Section,
        Self::Type,
        Self::Event,
        Self::Comment,
        Self::Function,
        Self::String,
        Self::Constant,
        Self::Integer,
        Self::Float,
        Self::StringConstant,
        Self::Compound,
        Self::Control,
        Self::Preprocessor,
        Self::Variable,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Text => aurora_i18n::tr!("script-color-text"),
            Self::Cursor => aurora_i18n::tr!("script-color-cursor"),
            Self::Background => aurora_i18n::tr!("script-color-background"),
            Self::Selection => aurora_i18n::tr!("script-color-selection"),
            Self::Section => aurora_i18n::tr!("script-color-section"),
            Self::Type => aurora_i18n::tr!("script-color-type"),
            Self::Event => aurora_i18n::tr!("script-color-event"),
            Self::Comment => aurora_i18n::tr!("script-color-comment"),
            Self::Function => aurora_i18n::tr!("script-color-function"),
            Self::String => aurora_i18n::tr!("script-color-string"),
            Self::Constant => aurora_i18n::tr!("script-color-constant"),
            Self::Integer => aurora_i18n::tr!("script-color-integer"),
            Self::Float => aurora_i18n::tr!("script-color-float"),
            Self::StringConstant => aurora_i18n::tr!("script-color-string-constant"),
            Self::Compound => aurora_i18n::tr!("script-color-compound"),
            Self::Control => aurora_i18n::tr!("script-color-control"),
            Self::Preprocessor => aurora_i18n::tr!("script-color-preprocessor"),
            Self::Variable => aurora_i18n::tr!("script-color-variable"),
        }
    }

    fn default_color(self, p: &Palette) -> Color32 {
        match self {
            Self::Text => p.ink,
            Self::Cursor | Self::Section | Self::Control | Self::Preprocessor => p.violet_light,
            Self::Background => p.field,
            Self::Selection => p.violet,
            Self::Type => p.rose,
            Self::Event => p.violet_pale,
            Self::Comment => p.muted,
            Self::Function => p.teal,
            Self::String | Self::StringConstant => p.success,
            Self::Constant | Self::Compound => p.amber,
            Self::Integer | Self::Float => p.warn,
            Self::Variable => p.indigo_light,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScriptSettings {
    pub colors: BTreeMap<ColorRole, [u8; 4]>,
    pub font_size: f32,
    pub word_wrap: bool,
    pub search_prefer_left: bool,
    pub search_inline: bool,
    pub convert_tabs: bool,
    pub preprocessor: bool,
    pub optimize: bool,
    pub switches: bool,
    pub lazy_lists: bool,
    pub disk_includes: bool,
    pub include_directory: String,
    pub owner_say_debug: bool,
    pub recompile: bool,
    pub external_editor: String,
    pub help_follows_cursor: bool,
}

impl Default for ScriptSettings {
    fn default() -> Self {
        Self {
            colors: BTreeMap::new(),
            font_size: 13.0,
            word_wrap: true,
            search_prefer_left: false,
            search_inline: false,
            convert_tabs: true,
            preprocessor: false,
            optimize: true,
            switches: true,
            lazy_lists: true,
            disk_includes: false,
            include_directory: String::new(),
            owner_say_debug: false,
            recompile: false,
            external_editor: String::new(),
            help_follows_cursor: false,
        }
    }
}

impl ScriptSettings {
    pub fn color(&self, role: ColorRole, p: &Palette) -> Color32 {
        self.colors.get(&role).map_or_else(
            || role.default_color(p),
            |c| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_preferences_keep_firestorm_defaults_and_colors_roundtrip() {
        let mut settings: ScriptSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, ScriptSettings::default());
        assert!(settings.word_wrap);
        assert!(!settings.search_prefer_left && !settings.search_inline);
        assert!(settings.convert_tabs && settings.optimize && settings.switches && settings.lazy_lists);
        assert!(!settings.preprocessor && !settings.disk_includes && !settings.owner_say_debug && !settings.recompile);
        let p = crate::theme::Theme::default().palette();
        settings.colors.insert(ColorRole::Function, p.teal.to_srgba_unmultiplied());
        settings.preprocessor = true;
        settings.word_wrap = false;
        settings.search_prefer_left = true;
        settings.search_inline = true;
        let restored: ScriptSettings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert_eq!(restored.color(ColorRole::Function, &p), p.teal);
    }
}
