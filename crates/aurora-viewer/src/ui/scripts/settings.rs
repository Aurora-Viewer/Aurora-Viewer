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
    /// The minimap at the right of the source, in place of the thin overview
    /// strip (Aurora's own; on, as an IDE's is).
    pub minimap: bool,
    /// The object in the tree's Properties, in three dimensions: the whole
    /// object from its root prim, or the child prim the script in front is in
    /// alone (Aurora's own, as the area search's view; on).
    #[serde(default = "on_by_default")]
    pub preview: bool,
    /// The old layout: one window per script, with neither the tree beside the
    /// source nor its tab strip -- a script of an object, or of the inventory,
    /// in a window of its own (off; the shared windows are the default).
    #[serde(default)]
    pub simple: bool,
    /// The minimap's width, in points (60), and how strongly it draws, in
    /// percent (100): a narrow or faint map leaves the text more room.
    #[serde(default = "default_minimap_width")]
    pub minimap_width: u16,
    #[serde(default = "default_minimap_opacity")]
    pub minimap_opacity: u8,
    /// The indent step of Tab, Shift+Tab, Enter and Backspace, in spaces (4).
    #[serde(default = "default_tab_size")]
    pub tab_size: u8,
    /// The editor's font family, by name; empty for the built-in monospace, or
    /// a monospace family this PC has (`ui::fonts::system_fonts`).
    #[serde(default)]
    pub font: String,
    /// A new script is Lua when the region runs SLua (on) -- and LSL when the
    /// region does not, or when this is off.
    #[serde(default = "on_by_default")]
    pub new_script_lua: bool,
    /// The reading aids, each on: the state/event line stuck above the source,
    /// the indent guides, the bars of the lines changed since the save, the
    /// word under the caret and its other occurrences, the bracket at the
    /// caret and its match, the caret's line, and the completion popup.
    #[serde(default = "on_by_default")]
    pub sticky_line: bool,
    #[serde(default = "on_by_default")]
    pub indent_guides: bool,
    #[serde(default = "on_by_default")]
    pub change_bars: bool,
    #[serde(default = "on_by_default")]
    pub occurrences: bool,
    #[serde(default = "on_by_default")]
    pub bracket_match: bool,
    #[serde(default = "on_by_default")]
    pub current_line: bool,
    #[serde(default = "on_by_default")]
    pub completion: bool,
    /// The help of a word says when the language deprecated it, and the name
    /// to use instead (`assets/lsl/definitions.json`; Aurora's own, on).
    #[serde(default = "on_by_default")]
    pub deprecations: bool,
    /// The help of a call lists its parameters and what each one means
    /// (`assets/lsl/definitions.json`; Aurora's own, on).
    #[serde(default = "on_by_default")]
    pub arguments: bool,
    /// The room of the tree beside the source, as the reader dragged its edge:
    /// 0 for the default.
    pub tree_width: u16,
    /// The parameter names in the gap before each argument of a call the
    /// catalog knows (Aurora's own; on, as an IDE's are).
    #[serde(default = "on_by_default")]
    pub inlays: bool,
    pub search_prefer_left: bool,
    /// Find / Replace in the window rather than in a panel beside it: a bar
    /// under its menus and toolbar, over the source (it takes no row of the
    /// text; off, the panel opens beside the window).
    pub search_inline: bool,
    pub convert_tabs: bool,
    pub preprocessor: bool,
    /// Unused code goes from the upload, as the Legacy Viewer's preprocessor
    /// does it (`preprocessor::lsl::optimize`).
    pub optimize: bool,
    pub switches: bool,
    pub lazy_lists: bool,
    pub disk_includes: bool,
    pub include_directory: String,
    pub owner_say_debug: bool,
    pub recompile: bool,
    pub external_editor: String,
    /// ExternalWebsocketSyncEnable: the local server an outside editor
    /// connects to (`crate::editor_sync`). Off here where the Legacy Viewer
    /// has it on: a server open on the computer only when asked for (the
    /// human's decision of 2026-11-17).
    pub websocket_sync: bool,
    /// ExternalWebsocketSyncPort.
    pub websocket_port: u16,
    /// ExternalWebsocketForwardDebug: a followed script's run-time errors
    /// go to the editor.
    pub websocket_forward_debug: bool,
}

impl ScriptSettings {}

/// A setting that is on unless the file says otherwise.
fn on_by_default() -> bool {
    true
}

/// The indent step of a new profile, and of a file that does not say.
pub const DEFAULT_TAB_SIZE: u8 = 4;
/// The minimap's width of a new profile, as `minimap::WIDTH`.
pub const DEFAULT_MINIMAP_WIDTH: u16 = 60;

fn default_tab_size() -> u8 {
    DEFAULT_TAB_SIZE
}

fn default_minimap_width() -> u16 {
    DEFAULT_MINIMAP_WIDTH
}

fn default_minimap_opacity() -> u8 {
    100
}

/// ExternalWebsocketSyncPort's default.
pub const WEBSOCKET_PORT: u16 = 9020;

impl Default for ScriptSettings {
    fn default() -> Self {
        Self {
            colors: BTreeMap::new(),
            font_size: 13.0,
            word_wrap: true,
            minimap: true,
            preview: true,
            simple: false,
            minimap_width: DEFAULT_MINIMAP_WIDTH,
            minimap_opacity: 100,
            tab_size: DEFAULT_TAB_SIZE,
            font: String::new(),
            new_script_lua: true,
            sticky_line: true,
            indent_guides: true,
            change_bars: true,
            occurrences: true,
            bracket_match: true,
            current_line: true,
            completion: true,
            deprecations: true,
            arguments: true,
            tree_width: 0,
            inlays: true,
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
            websocket_sync: false,
            websocket_port: WEBSOCKET_PORT,
            websocket_forward_debug: true,
        }
    }
}

impl ScriptSettings {
    /// The editor's font: the family the reader chose when this PC has it, the
    /// built-in monospace otherwise, at the size they set.
    pub fn font_id(&self, ctx: &egui::Context) -> egui::FontId {
        egui::FontId::new(self.font_size.clamp(8.0, 32.0), crate::ui::fonts::editor_family(ctx, &self.font))
    }

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
        assert!(settings.word_wrap && settings.minimap);
        assert!(!settings.simple, "the shared windows are the default");
        assert_eq!(settings.tab_size, 4);
        assert_eq!(settings.minimap_width, 60);
        assert_eq!(settings.minimap_opacity, 100);
        assert!(settings.font.is_empty(), "the built-in monospace");
        assert!(settings.new_script_lua, "Lua where the region runs it");
        assert!(
            settings.sticky_line
                && settings.indent_guides
                && settings.change_bars
                && settings.occurrences
                && settings.bracket_match
                && settings.current_line
                && settings.completion
        );
        assert!(!settings.search_prefer_left && !settings.search_inline);
        assert!(settings.convert_tabs && settings.optimize && settings.switches && settings.lazy_lists);
        // short names are off: the uploaded script stays readable in another
        // viewer, and the gain is small
        assert!(!settings.preprocessor && !settings.disk_includes && !settings.owner_say_debug && !settings.recompile);
        // the editor sync: off (stricter than the Legacy Viewer), port 9020,
        // run-time errors forwarded
        assert!(!settings.websocket_sync && settings.websocket_forward_debug);
        assert_eq!(settings.websocket_port, 9020);
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

    /// A settings file written before the object's 3D view existed shows it:
    /// its serde default is on, as the preview of the area search's objects.
    #[test]
    fn a_settings_file_written_before_the_view_shows_it() {
        let value = serde_json::to_value(ScriptSettings::default()).expect("a settings value");
        let mut object = value.as_object().cloned().expect("an object");
        object.remove("preview");
        let read: ScriptSettings = serde_json::from_value(serde_json::Value::Object(object)).expect("a settings file");
        assert!(read.preview, "the view is on");
    }
}
