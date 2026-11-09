//! The Debug Settings window (Advanced.ShowDebugSettings), every setting by
//! its name, over `settings/registry.rs`.
//!
//! Port of LLFloaterSettingsDebug (indra/newview/llfloatersettingsdebug.cpp,
//! floater_settings_debug.xml, originally LGPL 2.1): the search in the names
//! and the comments, "Show changed settings only" (DebugSettingsHideDefault),
//! the star of a changed setting, the editor of the value's type, Copy name,
//! Reset to default, and the DebugSettingsWarning notification with its
//! "ignore". A setting RLVa holds is greyed, as `updateControl` does.
//!
//! Differences: the names are Aurora's own (`camera.fov`), its settings being
//! typed fields (the Legacy names are in Preferences' code comments); the
//! comment of a setting is what Preferences says of it, in the interface's
//! language, and a setting Preferences does not show has only its name; a
//! setting's bounds are those of a settings file, and a value its type refuses
//! is refused with the reason. No sanity warning (SanityCheck): the bounds
//! already apply.

use super::widgets::{self, ActionKind, Floater};
use crate::settings::Settings;
use crate::settings::registry::{self, Entry, Kind, Locks};
use crate::theme::Palette;
use aurora_i18n::tr;
use egui::{RichText, Vec2};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The window's id (Floater.ToggleOrBringToFront "settings_debug").
pub const WINDOW: &str = "debug_settings";

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 520.0;
const LIST_W: f32 = 340.0;
const GAP: f32 = 12.0;
const BUTTON: Vec2 = Vec2::new(0.0, 28.0);
/// How often the values are read again while the window is open: another
/// window may change them, and reading all of them each frame is wasted work.
const REFRESH: Duration = Duration::from_millis(500);
/// "Copied" stays on the button this long.
const COPIED_FOR: Duration = Duration::from_secs(2);

/// What Preferences says of a setting: its row's label and hint.
type Words = (String, String);

#[derive(Default)]
pub struct DebugSettingsUi {
    filter: String,
    selected: Option<String>,
    /// `registry::defaults`, read once.
    defaults: Option<Value>,
    /// Every setting, as of `read_at`.
    entries: Vec<Entry>,
    read_at: Option<Instant>,
    /// By path, in the language `words_language` (`describe_settings`).
    words: HashMap<String, Words>,
    words_language: String,
    /// The warning was answered since the window opened.
    warned: bool,
    was_open: bool,
    /// The text being typed in the selected setting's field.
    typing: Option<String>,
    /// Why the last value was refused.
    refused: Option<String>,
    copied: Option<Instant>,
}

/// The words of `key`: its own row's, or those of the nearest row that edits
/// a whole section holding it.
fn words_of<'a>(words: &'a HashMap<String, Words>, key: &str) -> Option<&'a Words> {
    let mut at = key;
    loop {
        if let Some(found) = words.get(at) {
            return Some(found);
        }
        at = &at[..at.rfind('.')?];
    }
}

/// `onUpdateFilter`: the name or the comment holds the search, in any case.
fn matches(entry: &Entry, words: Option<&Words>, lower_query: &str) -> bool {
    lower_query.is_empty()
        || entry.key.to_lowercase().contains(lower_query)
        || words.is_some_and(|(label, hint)| label.to_lowercase().contains(lower_query) || hint.to_lowercase().contains(lower_query))
}

/// A value as the list and the default line write it. A float is an `f32`
/// widened on its way through JSON: written back as one, 0.1 stays 0.1.
fn written(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(n) if n.is_f64() => n.as_f64().map_or_else(|| n.to_string(), |f| (f as f32).to_string()),
        Value::Array(items) => items.iter().map(written).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// TYPE_COL4: four whole numbers named a colour are an `Rgba` (`colors.*`,
/// `build.new_prim_color`).
fn is_color(entry: &Entry) -> bool {
    entry.kind == (Kind::Vector { len: 4, whole: true }) && (entry.key.starts_with("colors.") || entry.key.ends_with("_color"))
}

fn kind_name(entry: &Entry) -> &'static str {
    if is_color(entry) {
        return tr!("debug-settings-type-color");
    }
    match entry.kind {
        Kind::Bool => tr!("debug-settings-type-bool"),
        Kind::Int => tr!("debug-settings-type-int"),
        Kind::Float => tr!("debug-settings-type-float"),
        Kind::Text => tr!("debug-settings-type-text"),
        Kind::Vector { .. } => tr!("debug-settings-type-vector"),
    }
}

impl DebugSettingsUi {
    /// The demo scenario: `warning` opens on the warning; otherwise no
    /// warning, `search` typed, the first match chosen.
    pub fn demo(&mut self, search: &str) {
        self.was_open = true;
        if search != "warning" {
            self.warned = true;
            self.filter = search.to_owned();
        }
    }

    /// The window; true when a setting changed.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, settings: &mut Settings, locks: Locks) -> bool {
        if !*open {
            // the list and the words are not worth keeping while closed
            if self.was_open {
                self.was_open = false;
                self.warned = false;
                self.entries = Vec::new();
                self.read_at = None;
                self.typing = None;
                self.refused = None;
            }
            return false;
        }
        self.was_open = true;
        self.refresh(settings);

        let mut changed = false;
        Floater::new(
            WINDOW,
            tr!("toolbar-command-debug-settings"),
            egui::Pos2::new(120.0, 80.0),
            Vec2::new(WIDTH, HEIGHT),
        )
        .show(ctx, p, open, |ui| {
            // Fixed areas, not rows sized by what they hold: a window grows
            // with its content, so content that fills its room and a little
            // more would widen the window at every frame.
            let width = ui.available_width();
            widgets::search_field(ui, &mut self.filter, tr!("debug-settings-search"), width);
            changed |= widgets::checkbox(ui, p, &mut settings.debug_settings_changed_only, tr!("debug-settings-changed-only")).changed();
            ui.add_space(6.0);
            let shown = self.shown(settings.debug_settings_changed_only);
            // `onUpdateFilter` selects the first match of a search
            if !self.filter.is_empty() && !shown.iter().any(|i| Some(&self.entries[*i].key) == self.selected.as_ref()) {
                self.select(shown.first().map(|i| self.entries[*i].key.clone()));
            }
            let (room, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let list = egui::Rect::from_min_size(room.min, Vec2::new(LIST_W.min(room.width() * 0.5), room.height()));
            let details = egui::Rect::from_min_max(egui::Pos2::new(list.right() + GAP, room.top()), room.max);
            let list_layout = egui::Layout::top_down_justified(egui::Align::LEFT);
            ui.scope_builder(egui::UiBuilder::new().max_rect(list).layout(list_layout), |ui| {
                self.list(ui, p, &shown);
            });
            let details_layout = egui::Layout::top_down(egui::Align::LEFT);
            ui.scope_builder(egui::UiBuilder::new().max_rect(details).layout(details_layout), |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(details));
                changed |= self.details(ui, p, settings, locks);
            });
        });
        changed |= self.warning(ctx, p, settings, open);
        changed
    }

    /// Reads the values when they may have moved, and the words when the
    /// interface changed language.
    fn refresh(&mut self, settings: &Settings) {
        let language = aurora_i18n::language().to_string();
        if self.words.is_empty() || self.words_language != language {
            self.words = crate::ui::options::describe_settings()
                .into_iter()
                .map(|(path, label, hint)| (path, (label, hint)))
                .collect();
            self.words_language = language;
        }
        if self.read_at.is_none_or(|at| at.elapsed() >= REFRESH) {
            self.read_entries(settings);
        }
    }

    fn read_entries(&mut self, settings: &Settings) {
        let defaults = self.defaults.get_or_insert_with(registry::defaults);
        self.entries = registry::entries(settings, defaults);
        self.read_at = Some(Instant::now());
    }

    /// The indices of the listed settings.
    fn shown(&self, changed_only: bool) -> Vec<usize> {
        let query = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !changed_only || e.changed())
            .filter(|(_, e)| matches(e, words_of(&self.words, &e.key), &query))
            .map(|(i, _)| i)
            .collect()
    }

    fn select(&mut self, key: Option<String>) {
        if self.selected != key {
            self.selected = key;
            self.typing = None;
            self.refused = None;
            self.copied = None;
        }
    }

    fn list(&mut self, ui: &mut egui::Ui, p: &Palette, shown: &[usize]) {
        let mut chosen = None;
        // what a selectable row takes, for the scroll bar to be true
        let row_h = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .id_salt("debug_settings_list")
            .auto_shrink([false, false])
            .show_rows(ui, row_h, shown.len(), |ui, rows| {
                // a long name is cut, with the whole of it under the pointer
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for index in rows {
                    let entry = &self.entries[shown[index]];
                    // the star of a changed setting (FS: changed_setting)
                    let mark = if entry.changed() { "* " } else { "  " };
                    let selected = self.selected.as_ref() == Some(&entry.key);
                    let text = RichText::new(format!("{mark}{}", entry.key)).monospace().size(11.5).color(p.ink);
                    if ui.selectable_label(selected, text).on_hover_text(&entry.key).clicked() {
                        chosen = Some(entry.key.clone());
                    }
                }
            });
        if chosen.is_some() {
            self.select(chosen);
        }
    }

    /// The selected setting; true when it changed.
    fn details(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut Settings, locks: Locks) -> bool {
        let Some(entry) = self
            .selected
            .as_ref()
            .and_then(|key| self.entries.iter().find(|e| &e.key == key))
            .cloned()
        else {
            ui.label(RichText::new(tr!("debug-settings-none-selected")).color(p.muted));
            return false;
        };
        ui.label(RichText::new(&entry.key).strong().color(p.ink));
        ui.label(RichText::new(kind_name(&entry)).size(11.5).color(p.muted));
        ui.add_space(4.0);
        match words_of(&self.words, &entry.key) {
            Some((label, hint)) => {
                ui.label(RichText::new(label).color(p.ink));
                if !hint.is_empty() {
                    ui.label(RichText::new(hint).size(11.5).color(p.muted));
                }
            }
            None => {
                ui.label(RichText::new(tr!("debug-settings-not-in-preferences")).size(11.5).color(p.muted));
            }
        }
        ui.add_space(8.0);

        let locked = locks.locked(&entry.key);
        let edited = ui.add_enabled_ui(!locked, |ui| self.editor(ui, p, &entry)).inner;
        let mut changed = false;
        if let Some(value) = edited {
            changed |= self.apply(settings, &entry.key, value);
        }
        if locked {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("debug-settings-rlva-locked")).size(11.5).color(p.muted));
        }
        if let Some(reason) = &self.refused {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("debug-settings-refused")).size(11.5).color(p.ink));
            ui.label(RichText::new(reason).size(11.5).color(p.muted));
        }
        ui.add_space(8.0);
        let default = written(&entry.default);
        ui.label(
            RichText::new(tr!("debug-settings-default", value = default.as_str()))
                .size(11.5)
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let copied = self.copied.is_some_and(|at| at.elapsed() < COPIED_FOR);
            let label = if copied {
                tr!("debug-settings-copied")
            } else {
                tr!("debug-settings-copy-name")
            };
            // the name only, as `onCopyToClipboard`: never a value
            if widgets::action_button(ui, p, "copy-filled", label, ActionKind::Flat, BUTTON).clicked() {
                ui.ctx().copy_text(entry.key.clone());
                self.copied = Some(Instant::now());
            }
            let can_reset = !locked && entry.changed();
            if widgets::action_button_if(ui, p, "restore", tr!("debug-settings-reset"), ActionKind::Flat, BUTTON, can_reset).clicked() {
                self.typing = None;
                changed |= self.apply(settings, &entry.key, entry.default.clone());
            }
        });
        changed
    }

    /// The editor of the value's kind; the new value when one was given.
    fn editor(&mut self, ui: &mut egui::Ui, p: &Palette, entry: &Entry) -> Option<Value> {
        match entry.kind {
            Kind::Bool => {
                let mut on = entry.value.as_bool().unwrap_or(false);
                widgets::switch(ui, p, &mut on).changed().then(|| json!(on))
            }
            Kind::Int => number(ui, &entry.value, true).map(|n| json!(n as i64)),
            Kind::Float => number(ui, &entry.value, false).map(|n| json!(n as f32)),
            Kind::Text => {
                let current = entry.value.as_str().unwrap_or_default();
                let text = self.typing.get_or_insert_with(|| current.to_owned());
                let response = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                // given when typed in full, not at each letter: a half-typed
                // word is rarely a value of its type
                if response.lost_focus() && text.as_str() != current {
                    Some(json!(text.clone()))
                } else {
                    if !response.has_focus() {
                        current.clone_into(text);
                    }
                    None
                }
            }
            _ if is_color(entry) => {
                let mut color: crate::settings::Rgba = serde_json::from_value(entry.value.clone()).ok()?;
                widgets::color_button(ui, p, ("debug_settings_color", &entry.key), &mut color, true)
                    .changed()
                    .then(|| json!(color))
            }
            Kind::Vector { whole, .. } => {
                let mut items = entry.value.as_array().cloned().unwrap_or_default();
                let mut edited = false;
                ui.horizontal_wrapped(|ui| {
                    for item in &mut items {
                        if let Some(n) = number(ui, item, whole) {
                            *item = if whole { json!(n as i64) } else { json!(n as f32) };
                            edited = true;
                        }
                    }
                });
                edited.then_some(Value::Array(items))
            }
        }
    }

    /// Gives the value; a refusal stays on screen with its reason.
    fn apply(&mut self, settings: &mut Settings, key: &str, value: Value) -> bool {
        match registry::set(settings, key, value) {
            Ok(()) => {
                self.refused = None;
                self.typing = None;
                self.read_entries(settings);
                true
            }
            Err(reason) => {
                self.refused = Some(reason);
                false
            }
        }
    }

    /// DebugSettingsWarning: an alert with "do not show again".
    fn warning(&mut self, ctx: &egui::Context, p: &Palette, settings: &mut Settings, open: &mut bool) -> bool {
        if self.warned || settings.debug_settings_warning_ignored || !*open {
            return false;
        }
        let mut changed = false;
        widgets::Modal::new(egui::Id::unique("debug_settings_warning")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new(tr!("debug-settings-warning-title")).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(RichText::new(tr!("debug-settings-warning-text")).size(12.5).color(p.muted));
            ui.add_space(8.0);
            let mut ignore = settings.debug_settings_warning_ignored;
            if widgets::checkbox(ui, p, &mut ignore, tr!("debug-settings-warning-ignore")).changed() {
                settings.debug_settings_warning_ignored = ignore;
                changed = true;
            }
            ui.add_space(10.0);
            if widgets::primary_button_sized(ui, p, tr!("common-ok"), Vec2::new(96.0, 28.0)).clicked() {
                self.warned = true;
            }
        });
        changed
    }
}

/// A number's field (Legacy spinner: 0.1 per step, 1 for a whole number); the
/// new value when it moved.
fn number(ui: &mut egui::Ui, value: &Value, whole: bool) -> Option<f64> {
    let mut n = value.as_f64().unwrap_or(0.0);
    let field = egui::DragValue::new(&mut n).speed(if whole { 1.0 } else { 0.1 });
    let field = if whole { field.fixed_decimals(0) } else { field.max_decimals(6) };
    ui.add(field).changed().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, value: Value, default: Value) -> Entry {
        Entry {
            key: key.to_owned(),
            kind: Kind::Text,
            value,
            default,
        }
    }

    #[test]
    fn search_reads_the_name_and_the_words_in_any_case() {
        let e = entry("camera.fov", json!(""), json!(""));
        let words = ("Field of view".to_owned(), "The angle the camera sees".to_owned());
        assert!(matches(&e, None, ""));
        assert!(matches(&e, None, "fov"));
        assert!(matches(&e, None, "camera.f"));
        assert!(!matches(&e, None, "angle"));
        assert!(matches(&e, Some(&words), "angle"));
        assert!(matches(&e, Some(&words), "field"));
        assert!(!matches(&e, Some(&words), "brightness"));
    }

    #[test]
    fn a_setting_takes_the_words_of_its_section_row() {
        let words = HashMap::from([("camera".to_owned(), ("Camera".to_owned(), String::new()))]);
        assert_eq!(words_of(&words, "camera.fov").map(|w| w.0.as_str()), Some("Camera"));
        assert_eq!(words_of(&words, "camera").map(|w| w.0.as_str()), Some("Camera"));
        assert!(words_of(&words, "cameras").is_none());
        assert!(words_of(&words, "audio.volume").is_none());
    }

    #[test]
    fn floats_are_written_as_the_f32_they_are() {
        assert_eq!(written(&json!(0.1_f32)), "0.1");
        assert_eq!(written(&json!(2)), "2");
        assert_eq!(written(&json!(true)), "true");
        assert_eq!(written(&json!("text")), "text");
        assert_eq!(written(&json!([0.5_f32, 1, 2.25_f32])), "0.5, 1, 2.25");
    }

    #[test]
    fn preferences_describe_settings_of_the_list() {
        let words: HashMap<String, Words> = crate::ui::options::describe_settings()
            .into_iter()
            .map(|(path, label, hint)| (path, (label, hint)))
            .collect();
        assert!(words_of(&words, "camera.fov").is_some());
        let defaults = registry::defaults();
        let list = registry::entries(&Settings::default(), &defaults);
        let described = list.iter().filter(|e| words_of(&words, &e.key).is_some()).count();
        // most of the list has words of its own; print the share for the PR
        println!("debug settings: {described} of {} have Preferences' words", list.len());
        assert!(described * 2 > list.len(), "{described} of {}", list.len());
    }

    /// A window whose content asks for a little more than it has grows at
    /// every frame: its size stays the same whichever setting is chosen.
    #[test]
    fn no_setting_makes_the_window_grow() {
        let p = crate::theme::Theme::default().palette();
        let mut s = Settings {
            debug_settings_warning_ignored: true,
            ..Settings::default()
        };
        let mut st = DebugSettingsUi::default();
        st.read_entries(&s);
        let keys: Vec<String> = st.entries.iter().map(|e| e.key.clone()).collect();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(4000.0, 2000.0));
        let mut grew = Vec::new();
        for key in keys {
            let ctx = egui::Context::default();
            let raw = || egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            st.selected = Some(key.clone());
            st.typing = None;
            let width = |ctx: &egui::Context, st: &mut DebugSettingsUi, s: &mut Settings| {
                let mut open = true;
                crate::ui::options::run_ui(ctx, raw(), |ui| {
                    st.show(ui.ctx(), &p, &mut open, s, Locks::default());
                });
                ctx.globally_used_rect().width()
            };
            for _ in 0..3 {
                width(&ctx, &mut st, &mut s);
            }
            let settled = width(&ctx, &mut st, &mut s);
            for _ in 0..6 {
                width(&ctx, &mut st, &mut s);
            }
            let last = width(&ctx, &mut st, &mut s);
            if last > settled + 0.5 {
                grew.push(format!("{key}: {settled} -> {last}"));
            }
        }
        assert!(grew.is_empty(), "{} settings: {}", grew.len(), grew.join("\n"));
    }

    #[test]
    fn changed_only_and_the_search_filter_the_list() {
        let mut st = DebugSettingsUi::default();
        let mut s = Settings::default();
        s.show_huds = !s.show_huds;
        st.read_entries(&s);
        assert_eq!(st.shown(true).len(), 1);
        assert_eq!(st.entries[st.shown(true)[0]].key, "show_huds");
        assert!(st.shown(false).len() > 1);
        st.filter = "NO SUCH SETTING".to_owned();
        assert!(st.shown(false).is_empty());
        st.filter = "SHOW_HUDS".to_owned();
        assert_eq!(st.shown(false).len(), 1);
    }
}
