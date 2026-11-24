//! LLFloaterSearchReplace and script preferences (Firestorm, LGPL 2.1).

use super::{
    editing, reference,
    settings::{ColorRole, ScriptSettings},
    syntax::Highlighter,
};
use crate::{
    theme::Palette,
    ui::widgets::{Floater, flat_button, icon_button},
};
use egui::{Id, RichText, Vec2};

#[derive(Default)]
pub(super) struct Panels {
    pub preferences: Preferences,
    pub search: Search,
}

#[derive(Default)]
pub(super) struct Search {
    pub open: bool,
    pub query: String,
    pub replacement: String,
    pub insensitive: bool,
    pub whole_words: bool,
    pub up: bool,
    pub focus: bool,
    pub owner: Option<Id>,
    status: String,
    // Explicitly retain the current find result while Find / Replace owns focus.
    current: Option<std::ops::Range<usize>>,
}

impl Search {
    pub const WINDOW: &str = "script_search";
    const SIZE: Vec2 = Vec2::new(360.0, 216.0);
    const INLINE_FIELD_WIDTH: f32 = 140.0;
    const INLINE_BUTTON_PADDING: f32 = 4.0;

    pub fn inline_width(ui: &egui::Ui) -> f32 {
        let checkbox_width = |label: &str| {
            ui.painter()
                .layout_no_wrap(label.into(), egui::TextStyle::Button.resolve(ui.style()), ui.visuals().text_color())
                .size()
                .x
                + ui.spacing().icon_width
                + ui.spacing().icon_spacing
        };
        let first_row = Self::INLINE_FIELD_WIDTH
            + 8.0
            // Reserve a small result label beside the options.
            + 20.0
            + 3.0 * ui.spacing().item_spacing.x
            + checkbox_width(aurora_i18n::tr!("script-search-insensitive"))
            + checkbox_width(aurora_i18n::tr!("script-search-up"));
        let actions = [
            aurora_i18n::tr!("script-search-find"),
            aurora_i18n::tr!("script-search-replace"),
            aurora_i18n::tr!("script-search-replace-all"),
        ];
        let second_row = Self::INLINE_FIELD_WIDTH
            + 8.0
            + 4.0 * ui.spacing().item_spacing.x
            + actions
                .into_iter()
                .map(|label| {
                    ui.painter()
                        .layout_no_wrap(label.into(), egui::FontId::proportional(12.0), ui.visuals().text_color())
                        .size()
                        .x
                        + 2.0 * Self::INLINE_BUTTON_PADDING
                })
                .sum::<f32>()
            + checkbox_width(aurora_i18n::tr!("script-search-whole-words"));
        first_row.max(second_row).max(440.0)
    }

    pub fn inline_min_height(ui: &egui::Ui) -> f32 {
        2.0 * (ui.text_style_height(&egui::TextStyle::Body) + 4.0)
            .max(ui.text_style_height(&egui::TextStyle::Button))
            .max(20.0)
            + ui.spacing().item_spacing.y
    }

    pub fn owns(&self, id: Id) -> bool {
        self.open && self.owner == Some(id)
    }

    pub fn placement(ctx: &egui::Context, parent: egui::Rect, content_top: f32, prefer_left: bool) -> (egui::Pos2, bool) {
        let room = crate::ui::layout::work_area(ctx);
        let size = ctx
            .memory(|m| m.area_rect(Id::unique(Self::WINDOW)))
            .map_or(Self::SIZE, |r| r.size());
        let fits = |x| x >= room.left() && x + size.x <= room.right();
        let right = parent.right();
        let left = parent.left() - size.x;
        let sides = if prefer_left { [left, right] } else { [right, left] };
        let (x, y, overlap) = if let Some(x) = sides.into_iter().find(|&x| fits(x)) {
            (x, parent.top(), false)
        } else {
            (parent.right() - size.x, content_top, true)
        };
        (
            egui::pos2(
                x.clamp(room.left(), (room.right() - size.x).max(room.left())),
                y.clamp(room.top(), (room.bottom() - size.y).max(room.top())),
            ),
            overlap,
        )
    }

    // LLFloaterSearchReplace::show rebinds one shared search to its editor.
    pub fn begin(&mut self, ctx: &egui::Context, owner: Id, id: Id, source: &str) {
        if self.owner.is_none() {
            self.insensitive = true;
        }
        let selection = editing::bytes(source, editing::cursor(ctx, id));
        if !selection.is_empty() {
            self.query = source[selection].into();
        }
        self.open = true;
        self.focus = true;
        self.owner = Some(owner);
        self.refresh_selected_match(ctx, id, source);
        crate::ui::widgets::expand_floater(ctx, Self::WINDOW);
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, id: Id, source: &mut String, editable: bool, pos: egui::Pos2) {
        if !self.open {
            return;
        }
        let window = Id::unique(Self::WINDOW);
        // Adopt the narrower default once, retaining later manual resizes.
        let compact = ctx.data_mut(|d| {
            let key = window.with("compact_width_v1");
            let done = d.get_persisted::<bool>(key).unwrap_or(false);
            d.insert_persisted(key, true);
            !done
        });
        if compact {
            let height = ctx.memory(|m| m.area_rect(window)).map_or(Self::SIZE.y, |r| r.height());
            crate::ui::widgets::place_floater(ctx, Self::WINDOW, pos, Vec2::new(Self::SIZE.x, height.max(Self::SIZE.y)));
        }
        crate::ui::widgets::move_floater(ctx, Self::WINDOW, pos);
        if self.focus {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, window));
        }
        let mut open = true;
        Floater::new(Self::WINDOW, aurora_i18n::tr!("script-search-title"), pos, Self::SIZE).show(ctx, p, &mut open, |ui| {
            self.controls(ui, p, id, source, editable, false);
        });
        self.open = open;
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, source: &mut String, editable: bool, inline: bool) {
        let ctx = ui.ctx().clone();
        if let Some(owner) = self.owner {
            super::focus::claim(&ctx, ui.layer_id(), owner);
        }
        let window = Id::unique(Self::WINDOW);
        if inline {
            self.inline_controls(ui, p, id, source, editable);
            return;
        }
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-search-query"));
            self.query_field(ui, p, id, source, 220.0);
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-search-replacement"));
            crate::ui::widgets::text_field(ui, editable, &mut self.replacement, |b, _| {
                egui::TextEdit::singleline(b).id(window.with("replacement")).desired_width(220.0)
            });
        });
        ui.horizontal(|ui| {
            let before = self.insensitive;
            crate::ui::widgets::checkbox(ui, p, &mut self.insensitive, aurora_i18n::tr!("script-search-insensitive"));
            if self.insensitive != before {
                self.refresh_selected_match(ui.ctx(), id, source);
            }
            crate::ui::widgets::checkbox(ui, p, &mut self.up, aurora_i18n::tr!("script-search-up"));
        });
        ui.horizontal(|ui| {
            let before = self.whole_words;
            crate::ui::widgets::checkbox(ui, p, &mut self.whole_words, aurora_i18n::tr!("script-search-whole-words"));
            if self.whole_words != before {
                self.refresh_selected_match(ui.ctx(), id, source);
            }
        });
        ui.horizontal(|ui| {
            if flat_button(ui, p, aurora_i18n::tr!("script-search-find")).clicked() {
                self.find(&ctx, id, source);
            }
            ui.add_enabled_ui(editable && !self.query.is_empty(), |ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-search-replace")).clicked() {
                    self.replace(&ctx, id, source);
                }
                if flat_button(ui, p, aurora_i18n::tr!("script-search-replace-all")).clicked() {
                    self.replace_all(&ctx, id, source);
                }
            });
        });
        ui.label(RichText::new(&self.status).color(p.muted));
    }

    fn query_field(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, source: &str, width: f32) {
        let response = crate::ui::widgets::search_text_edit(ui, p, &mut self.query, |edit| {
            edit.id(Id::unique(Self::WINDOW).with("query"))
                .hint_text(aurora_i18n::tr!("script-search-query"))
                .desired_width(width)
        })
        .on_hover_text(aurora_i18n::tr!("script-search-query"));
        if response.changed() {
            self.refresh_selected_match(ui.ctx(), id, source);
        }
        if !ui.is_sizing_pass() && ui.is_enabled() && std::mem::take(&mut self.focus) {
            editing::select(ui.ctx(), response.id, 0..self.query.chars().count(), true);
        }
        if (response.has_focus() || response.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
            self.find(ui.ctx(), id, source);
            response.request_focus();
        }
    }

    fn inline_controls(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, source: &mut String, editable: bool) {
        let ctx = ui.ctx().clone();
        let row_height = (ui.max_rect().height() - ui.spacing().item_spacing.y) / 2.0;
        // Two toolbar rows share the existing menu/toolbar height. Result
        // feedback stays beside the options, including its full hover text.
        ui.spacing_mut().interact_size.y = row_height;
        ui.spacing_mut().button_padding.y = 1.0;
        ui.spacing_mut().button_padding.x = Self::INLINE_BUTTON_PADDING;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        ui.horizontal(|ui| {
            ui.set_min_height(row_height);
            self.query_field(ui, p, id, source, Self::INLINE_FIELD_WIDTH);
            let before_case = self.insensitive;
            crate::ui::widgets::checkbox(ui, p, &mut self.insensitive, aurora_i18n::tr!("script-search-insensitive"));
            crate::ui::widgets::checkbox(ui, p, &mut self.up, aurora_i18n::tr!("script-search-up"));
            if self.insensitive != before_case {
                self.refresh_selected_match(ui.ctx(), id, source);
            }
            if !self.status.is_empty() {
                ui.add(egui::Label::new(RichText::new(&self.status).small().color(p.muted)).truncate())
                    .on_hover_text(&self.status);
            }
        });
        ui.horizontal(|ui| {
            ui.set_min_height(row_height);
            crate::ui::widgets::text_field(ui, editable, &mut self.replacement, |b, _| {
                egui::TextEdit::singleline(b)
                    .id(Id::unique(Self::WINDOW).with("replacement"))
                    .hint_text(aurora_i18n::tr!("script-search-replacement"))
                    .desired_width(Self::INLINE_FIELD_WIDTH)
            })
            .on_hover_text(aurora_i18n::tr!("script-search-replacement"));
            let before = self.whole_words;
            crate::ui::widgets::checkbox(ui, p, &mut self.whole_words, aurora_i18n::tr!("script-search-whole-words"));
            if self.whole_words != before {
                self.refresh_selected_match(ui.ctx(), id, source);
            }
            if flat_button(ui, p, aurora_i18n::tr!("script-search-find")).clicked() {
                self.find(&ctx, id, source);
            }
            ui.add_enabled_ui(editable && !self.query.is_empty(), |ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-search-replace")).clicked() {
                    self.replace(&ctx, id, source);
                }
                if flat_button(ui, p, aurora_i18n::tr!("script-search-replace-all")).clicked() {
                    self.replace_all(&ctx, id, source);
                }
            });
        });
    }

    /// The last Find result, not merely the editor's native selection (which
    /// can become visually inactive while the Find / Replace floater has focus).
    /// Keep the remembered result only while the editor selection still
    /// agrees with it. egui keeps a selection only in the field with the
    /// keyboard: elsewhere it shrinks to its moving end, the start of the
    /// match when it was selected right to left, else its end.
    fn current_match(&self, ctx: &egui::Context, id: Id, matches: &[std::ops::Range<usize>]) -> Option<std::ops::Range<usize>> {
        let current = self.current.as_ref()?;
        let [start, end] = editing::cursor(ctx, id).sorted_cursors();
        let selected = start.index.0..end.index.0;
        let caret_at_edge = selected.is_empty() && (selected.start == current.start || selected.start == current.end);

        (matches.contains(current) && (selected == *current || caret_at_edge)).then(|| current.clone())
    }

    pub(super) fn active_range_from_matches(
        &self,
        ctx: &egui::Context,
        id: Id,
        matches: &[std::ops::Range<usize>],
    ) -> Option<std::ops::Range<usize>> {
        if !self.open || self.query.is_empty() {
            return None;
        }
        self.current_match(ctx, id, matches)
    }

    /// Test helper: exercise active-result validation independently of the UI.
    #[cfg(test)]
    pub(super) fn active_range(&self, ctx: &egui::Context, id: Id, source: &str) -> Option<std::ops::Range<usize>> {
        let matches = editing::matches_filtered(source, &self.query, self.insensitive, self.whole_words);
        self.active_range_from_matches(ctx, id, &matches)
    }

    /// Refresh the active match without moving the editor's selection.
    /// Opening Find and changing its options only update highlighting.
    fn refresh_selected_match(&mut self, ctx: &egui::Context, id: Id, source: &str) {
        let [start, end] = editing::cursor(ctx, id).sorted_cursors();
        let selected = start.index.0..end.index.0;
        let matches = editing::matches_filtered(source, &self.query, self.insensitive, self.whole_words);
        // A selected match becomes the current one; otherwise the match Find
        // went to stays current while the caret egui left on it has not moved
        // (Ctrl+F again after Find, or an option that keeps it a match).
        self.current = if matches.contains(&selected) {
            Some(selected)
        } else {
            self.current_match(ctx, id, &matches)
        };
        self.status.clear();
        ctx.request_repaint();
    }

    pub fn find(&mut self, ctx: &egui::Context, id: Id, source: &str) {
        let matches = editing::matches_filtered(source, &self.query, self.insensitive, self.whole_words);
        let range = editing::cursor(ctx, id).sorted_cursors();
        let native = range[0].index.0..range[1].index.0;
        let from = self.current_match(ctx, id, &matches).unwrap_or(native);
        if let Some(found) = editing::next_match(&matches, from, self.up) {
            editing::select(ctx, id, found.clone(), false);
            self.current = Some(found);
            self.status.clear();
        } else {
            self.current = None;
            self.status = aurora_i18n::tr!("script-search-not-found").into();
        }
    }

    fn replace(&mut self, ctx: &egui::Context, id: Id, source: &mut String) {
        let range = editing::cursor(ctx, id).sorted_cursors();
        let native = range[0].index.0..range[1].index.0;
        let matches = editing::matches_filtered(source, &self.query, self.insensitive, self.whole_words);
        let selected = if matches.contains(&native) {
            native
        } else {
            self.current_match(ctx, id, &matches).unwrap_or(native)
        };
        if matches.contains(&selected) && !editing::replace(ctx, id, source, selected, &self.replacement) {
            self.status = aurora_i18n::tr!("script-source-too-large").into();
            return;
        }
        self.current = None;
        self.find(ctx, id, source);
    }

    fn replace_all(&mut self, ctx: &egui::Context, id: Id, source: &mut String) {
        let ranges = editing::matches_filtered(source, &self.query, self.insensitive, self.whole_words);
        let count = ranges.len();
        if count == 0 {
            self.status = aurora_i18n::tr!("script-search-not-found").into();
            return;
        }
        let output = editing::replaced_all(source, &ranges, &self.replacement);
        if output.len() > aurora_net::scripts::MAX_SOURCE {
            self.status = aurora_i18n::tr!("script-source-too-large").into();
            return;
        }
        editing::replace_text(ctx, id, source, 0..source.chars().count(), &output);
        self.current = None;
        self.status = aurora_i18n::tr!("script-search-replaced", count = count as i64);
    }
}

#[derive(Default)]
pub(super) struct Preferences {
    pub open: bool,
    preview: Highlighter,
    directory: Option<std::sync::mpsc::Receiver<Option<String>>>,
}

impl Preferences {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, settings: &mut ScriptSettings) -> bool {
        if !self.open {
            return false;
        }
        let before = settings.clone();
        if let Some(result) = self.directory.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.directory = None;
            if let Some(path) = result {
                settings.include_directory = path;
            }
        }
        Floater::new("script_preferences", aurora_i18n::tr!("script-preferences-title"),
            ctx.content_rect().center() - Vec2::new(255.0, 330.0), Vec2::new(510.0, 660.0))
            .show(ctx, p, &mut self.open, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label(aurora_i18n::tr!("script-preferences-colors"));
                    egui::Grid::new("script_colors").num_columns(4).show(ui, |ui| {
                        for (i, role) in ColorRole::ALL.into_iter().enumerate() {
                            let mut color = settings.color(role, p).to_srgba_unmultiplied();
                            if crate::ui::widgets::color_button(ui, p, ("script_color", i), &mut color, true).changed() { settings.colors.insert(role, color); }
                            ui.label(role.label());
                            if i % 2 == 1 { ui.end_row(); }
                        }
                    });
                    if flat_button(ui, p, aurora_i18n::tr!("script-preferences-reset-colors")).clicked() { settings.colors.clear(); }
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("script-preferences-font-size"));
                        ui.add(egui::DragValue::new(&mut settings.font_size).range(8.0..=32.0));
                    });
                    let preview = "/* LSL */\ninteger counter = 0;\ndefault\n{\n    state_entry()\n    {\n        string greeting = \"Hello\";\n        llSay(PUBLIC_CHANNEL, greeting);\n    }\n}";
                    egui::Frame::new().fill(settings.color(ColorRole::Background, p)).inner_margin(6.0).show(ui, |ui| {
                        ui.add(egui::Label::new(self.preview.custom(preview, p, egui::FontId::monospace(settings.font_size.clamp(8.0,32.0)), settings, "", false, false, false)));
                    });
                    crate::ui::widgets::divider(ui, p);
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.word_wrap, aurora_i18n::tr!("script-preferences-word-wrap"));
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.search_prefer_left, aurora_i18n::tr!("script-preferences-search-left"));
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.search_inline, aurora_i18n::tr!("script-preferences-search-inline"));
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.convert_tabs, aurora_i18n::tr!("script-preferences-convert-tabs"));
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.preprocessor, aurora_i18n::tr!("script-preferences-preprocessor"));
                    ui.add_enabled_ui(settings.preprocessor, |ui| {
                        crate::ui::widgets::switch_labeled(ui, p, &mut settings.optimize, aurora_i18n::tr!("script-preferences-optimize"));
                        crate::ui::widgets::switch_labeled(ui, p, &mut settings.switches, aurora_i18n::tr!("script-preferences-switches"));
                        crate::ui::widgets::switch_labeled(ui, p, &mut settings.lazy_lists, aurora_i18n::tr!("script-preferences-lazy-lists"));
                        crate::ui::widgets::switch_labeled(ui, p, &mut settings.disk_includes, aurora_i18n::tr!("script-preferences-disk-includes"));
                        ui.label(aurora_i18n::tr!("script-preferences-include-path"));
                        ui.add_enabled_ui(settings.disk_includes, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut settings.include_directory).desired_width(370.0));
                                let choose = aurora_i18n::tr!("script-preferences-choose-directory");
                                if crate::ui::widgets::icon_button(ui, p, "folder-open-filled", choose, self.directory.is_none()) {
                                    let (tx,rx) = std::sync::mpsc::channel();
                                    let directory = settings.include_directory.clone(); self.directory = Some(rx);
                                    std::thread::spawn(move || {
                                        let path = rfd::FileDialog::new().set_directory(directory).pick_folder().map(|p| p.display().to_string());
                                        let _ = tx.send(path);
                                    });
                                }
                            });
                        });
                    });
                    crate::ui::widgets::divider(ui, p);
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.owner_say_debug, aurora_i18n::tr!("script-preferences-owner-say"));
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.recompile, aurora_i18n::tr!("script-preferences-recompile"));
                    ui.label(aurora_i18n::tr!("script-preferences-external"));
                    ui.text_edit_singleline(&mut settings.external_editor);
                    ui.label(RichText::new(aurora_i18n::tr!("script-preferences-external-hint")).small().color(p.muted));
                    // LLScriptEditorWSServer's settings (floater_scripting_settings.xml)
                    crate::ui::widgets::switch_labeled(ui, p, &mut settings.websocket_sync, aurora_i18n::tr!("script-preferences-websocket"));
                    ui.add_enabled_ui(settings.websocket_sync, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(aurora_i18n::tr!("script-preferences-websocket-port"));
                            ui.add(egui::DragValue::new(&mut settings.websocket_port).range(1024..=u16::MAX));
                        });
                        crate::ui::widgets::switch_labeled(ui, p, &mut settings.websocket_forward_debug, aurora_i18n::tr!("script-preferences-websocket-debug"));
                    });
                    ui.label(RichText::new(aurora_i18n::tr!("script-preferences-websocket-hint")).small().color(p.muted));
                });
            });
        *settings != before
    }
}

#[derive(Default)]
pub(super) struct Help {
    pub open: bool,
    pub keyword: String,
    /// The script is SLua: its words and help are Lua's.
    pub lua: bool,
    history: Vec<String>,
    at: usize,
    filter: String,
}

impl Help {
    pub fn visit(&mut self, name: &str) {
        if reference::get(name, self.lua).is_none() || self.keyword == name {
            return;
        }
        self.history.truncate(self.at + 1);
        self.history.push(name.into());
        self.at = self.history.len() - 1;
        self.keyword = name.into();
    }
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, id: Id, source: &str, settings: &mut ScriptSettings) -> bool {
        if !self.open {
            return false;
        }
        if settings.help_follows_cursor {
            self.visit(editing::word(source, editing::cursor(ctx, id).primary.index.0, self.lua));
        }
        let before = settings.help_follows_cursor;
        Floater::new(
            &format!("{id:?}_help"),
            aurora_i18n::tr!("script-help-keyword"),
            ctx.content_rect().center() + Vec2::new(100.0, -220.0),
            Vec2::new(510.0, 410.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.horizontal(|ui| {
                if icon_button(ui, p, "arrow-left", aurora_i18n::tr!("common-previous"), self.at > 0) {
                    self.at -= 1;
                    self.keyword.clone_from(&self.history[self.at]);
                }
                if icon_button(
                    ui,
                    p,
                    "arrow-right",
                    aurora_i18n::tr!("script-help-forward"),
                    self.at + 1 < self.history.len(),
                ) {
                    self.at += 1;
                    self.keyword.clone_from(&self.history[self.at]);
                }
                crate::ui::widgets::checkbox(ui, p, &mut settings.help_follows_cursor, aurora_i18n::tr!("script-help-follow"));
            });
            crate::ui::widgets::search_text_edit(ui, p, &mut self.filter, |edit| edit);
            let mut selected = None;
            egui::ComboBox::from_id_salt((id, "help_word"))
                .selected_text(&self.keyword)
                .height(220.0)
                .show_ui(ui, |ui| {
                    for entry in reference::all(self.lua)
                        .iter()
                        .filter(|e| self.filter.is_empty() || e.name.to_lowercase().contains(&self.filter.to_lowercase()))
                    {
                        if crate::ui::widgets::search_list_item(ui, entry.name == self.keyword, &entry.name).clicked() {
                            selected = Some(entry.name.clone());
                        }
                    }
                });
            if let Some(name) = selected
                && self.keyword != name
            {
                self.history.truncate(self.at + 1);
                self.history.push(name.clone());
                self.at = self.history.len() - 1;
                self.keyword = name;
            }
            if let Some(entry) = reference::get(&self.keyword, self.lua) {
                ui.label(RichText::new(&entry.signature).monospace().color(p.teal));
                egui::ScrollArea::vertical().max_height(225.0).show(ui, |ui| {
                    ui.label(&entry.tooltip);
                });
                ui.hyperlink_to(aurora_i18n::tr!("script-help-wiki"), reference::url(&entry.name, self.lua));
            } else {
                ui.label(aurora_i18n::tr!("script-help-select"));
            }
        });
        settings.help_follows_cursor != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opening_find_preserves_the_selected_match_and_replaces_that_one() {
        let ctx = egui::Context::default();
        let id = Id::unique("script_preserve_selection");
        let mut source = "hi high hi_there hi".to_owned();
        let last = source.chars().count() - 2;

        editing::select(&ctx, id, last..last + 2, true);
        let before = editing::cursor(&ctx, id).as_sorted_char_range();

        let mut search = Search {
            replacement: "HELLO".into(),
            whole_words: true,
            ..Default::default()
        };

        search.begin(&ctx, id, id, &source);

        assert_eq!(search.query, "hi");
        assert_eq!(search.active_range(&ctx, id, &source), Some(last..last + 2));
        assert_eq!(editing::cursor(&ctx, id).as_sorted_char_range(), before);

        search.replace(&ctx, id, &mut source);
        assert_eq!(source, "hi high hi_there HELLO");
    }

    #[test]
    fn refreshing_the_query_does_not_move_the_selection() {
        let ctx = egui::Context::default();
        let id = Id::unique("script_query_preserves_selection");
        let source = "hi high hi".to_owned();

        editing::select(&ctx, id, 8..10, true);

        let mut search = Search {
            open: true,
            query: "hi".into(),
            whole_words: true,
            ..Default::default()
        };

        search.refresh_selected_match(&ctx, id, &source);
        assert_eq!(search.active_range(&ctx, id, &source), Some(8..10));

        search.query = "high".into();
        search.refresh_selected_match(&ctx, id, &source);

        assert_eq!(search.active_range(&ctx, id, &source), None);
        assert_eq!(
            editing::cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(8)..egui::text::CharIndex(10)
        );
    }

    #[test]
    fn active_result_survives_find_window_focus_and_clears_for_a_different_query() {
        let ctx = egui::Context::default();
        let id = Id::unique("script_active_search");
        let source = "\u{C9}t\u{E9} \u{E9}t\u{E9} \u{C9}T\u{C9}";
        let mut search = Search {
            open: true,
            query: "\u{E9}t\u{E9}".into(),
            insensitive: true,
            ..Default::default()
        };
        search.find(&ctx, id, source);
        assert_eq!(search.active_range(&ctx, id, source), Some(0..3));
        // The Find field owns keyboard focus; active navigation must still
        // advance and the active result must remain available for painting.
        ctx.memory_mut(|m| m.request_focus(Id::unique("script_search_field")));
        search.find(&ctx, id, source);
        assert_eq!(search.active_range(&ctx, id, source), Some(4..7));
        search.up = true;
        search.find(&ctx, id, source);
        assert_eq!(search.active_range(&ctx, id, source), Some(0..3));
        search.query = "missing".into();
        assert_eq!(search.active_range(&ctx, id, source), None);
        search.open = false;
        assert_eq!(search.active_range(&ctx, id, source), None);
    }

    #[test]
    fn moving_the_caret_changes_where_find_continues() {
        let ctx = egui::Context::default();
        let id = Id::unique("script_moved_caret");
        let source = "hit gap hit gap hit";

        let mut search = Search {
            open: true,
            query: "hit".into(),
            ..Default::default()
        };

        search.find(&ctx, id, source);
        assert_eq!(search.active_range(&ctx, id, source), Some(0..3));

        // Move the caret beyond the second hit. The next result must
        // come after the caret, not after the remembered first hit.
        editing::select(&ctx, id, 12..12, true);

        assert_eq!(search.active_range(&ctx, id, source), None);

        search.find(&ctx, id, source);
        assert_eq!(search.active_range(&ctx, id, source), Some(16..19));
    }

    #[test]
    fn replace_does_not_change_a_stale_result_after_caret_movement() {
        let ctx = egui::Context::default();
        let id = Id::unique("script_stale_replace");
        let mut source = "hit gap hit gap hit".to_owned();

        let mut search = Search {
            open: true,
            query: "hit".into(),
            replacement: "DONE".into(),
            ..Default::default()
        };

        search.find(&ctx, id, &source);
        assert_eq!(search.active_range(&ctx, id, &source), Some(0..3));

        // The caret is now between the first and second matches.
        // Replace must not change the old remembered result.
        editing::select(&ctx, id, 5..5, true);
        search.replace(&ctx, id, &mut source);

        assert_eq!(source, "hit gap hit gap hit");
        assert_eq!(search.active_range(&ctx, id, &source), Some(8..11));
    }

    #[test]
    fn whole_words_only_applies_to_script_replace_all() {
        let ctx = egui::Context::default();
        let id = Id::unique("whole_script_replace");
        let mut source = "This hi high hi_there hi-there".to_string();
        let mut search = Search {
            query: "hi".into(),
            whole_words: true,
            replacement: "hello".into(),
            ..Default::default()
        };
        search.replace_all(&ctx, id, &mut source);
        assert_eq!(source, "This hello high hi_there hello-there");
        editing::undo(&ctx, id, &mut source, false);
        assert_eq!(source, "This hi high hi_there hi-there");
        // A closed search does not expose an active result.
        search.open = false;
        assert_eq!(search.active_range(&ctx, id, &source), None);
    }

    #[test]
    fn replace_all_is_one_undoable_unicode_edit_and_search_wraps_in_both_directions() {
        let ctx = egui::Context::default();
        let id = Id::unique("search_test");
        let mut source = "Été été ÉTÉ".to_owned();
        let mut search = Search {
            query: "été".into(),
            replacement: "あ".into(),
            insensitive: true,
            ..Default::default()
        };
        search.find(&ctx, id, &source);
        assert_eq!(
            editing::cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(0)..egui::text::CharIndex(3)
        );
        search.up = true;
        search.find(&ctx, id, &source);
        assert_eq!(
            editing::cursor(&ctx, id).as_sorted_char_range(),
            egui::text::CharIndex(8)..egui::text::CharIndex(11)
        );
        search.replace_all(&ctx, id, &mut source);
        assert_eq!(source, "あ あ あ");
        editing::undo(&ctx, id, &mut source, false);
        assert_eq!(source, "Été été ÉTÉ");
        editing::undo(&ctx, id, &mut source, true);
        assert_eq!(source, "あ あ あ");
    }
}
