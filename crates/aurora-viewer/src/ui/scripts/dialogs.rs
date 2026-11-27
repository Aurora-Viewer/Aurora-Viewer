//! LLFloaterSearchReplace and script preferences (Firestorm, LGPL 2.1).

use super::{
    Lang, editing,
    settings::{ColorRole, ScriptSettings},
    syntax::Highlighter,
};
use crate::{
    theme::Palette,
    ui::widgets::{Floater, flat_button, icon_button, keycap},
};
use egui::{Id, RichText, Vec2};

#[derive(Default)]
pub(super) struct Panels {
    pub preferences: Preferences,
    pub search: Search,
    /// The Shortcuts window, as the Help menu opens it.
    pub shortcuts: bool,
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
    /// Whether the keyboard was in this panel on the last frame: egui
    /// surrenders text focus on Escape before a frame's UI runs, so the panel
    /// cannot read the focus at the moment the key arrives.
    keyboard: bool,
    status: String,
    // Explicitly retain the current find result while Find / Replace owns focus.
    current: Option<std::ops::Range<usize>>,
}

impl Search {
    pub const WINDOW: &str = "script_search";
    const SIZE: Vec2 = Vec2::new(360.0, 216.0);
    /// The query and the replacement of the bar under a toolbar: narrow
    /// enough for the whole of Find / Replace across one row.
    const POPUP_FIELD_WIDTH: f32 = 140.0;
    /// The room what the last command said takes at its end.
    const POPUP_STATUS_WIDTH: f32 = 150.0;

    pub fn close(&mut self) {
        self.open = false;
        self.focus = false;
        self.keyboard = false;
        self.current = None;
    }

    pub fn owns(&self, id: Id) -> bool {
        self.open && self.owner == Some(id)
    }

    /// Escape closes the find panel while it has the keyboard, and gives the
    /// keyboard back to the script: the close of an IDE's find bar.
    fn escape_closes(&mut self, ctx: &egui::Context, layer: egui::LayerId) -> bool {
        if !self.open {
            return false;
        }
        let window = Id::unique(Self::WINDOW);
        let inside = ctx.memory(|m| m.focused()).is_some_and(|field| {
            field == window.with("query")
                || field == window.with("replacement")
                || ctx.read_response(field).is_some_and(|r| r.layer_id == layer)
        });
        // read the frame before the key: egui surrendered the text focus
        let was_inside = std::mem::replace(&mut self.keyboard, inside);
        if was_inside && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close();
            if let Some(owner) = self.owner {
                super::focus::text(ctx, owner);
            }
            return true;
        }
        false
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
        self.keyboard = false;
        self.owner = Some(owner);
        self.refresh_selected_match(ctx, id, source);
        crate::ui::widgets::expand_floater(ctx, Self::WINDOW);
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, id: Id, source: &mut String, editable: bool, pos: egui::Pos2) {
        if !self.open {
            return;
        }
        let window = Id::unique(Self::WINDOW);
        if self.escape_closes(ctx, egui::LayerId::new(egui::Order::Middle, window)) {
            return;
        }
        // Adopt the narrower default once, retaining later manual resizes.
        let compact = ctx.data_mut(|d| {
            let key = window.with("whole_words_window_size_v1");
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
            self.controls(ui, p, id, source, editable);
        });
        self.open = open;
    }

    /// Find / Replace as a bar under the window's menus and toolbar, over the
    /// source: the whole of it across one row, and no row of the layout taken
    /// from the text. It belongs to its window -- directly above it, under the
    /// other windows -- as a dependent floater of the Legacy Viewer does.
    pub fn popup(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        id: Id,
        source: &mut String,
        editable: bool,
        anchor: egui::Rect,
        owner: egui::LayerId,
    ) {
        if !self.open {
            return;
        }
        let area = Id::unique(Self::WINDOW).with("popup");
        if self.escape_closes(ctx, egui::LayerId::new(egui::Order::Middle, area)) {
            return;
        }
        ctx.set_sublayer(owner, egui::LayerId::new(egui::Order::Middle, area));
        egui::Area::new(area)
            .order(egui::Order::Middle)
            .fade_in(false)
            .fixed_pos(egui::pos2(anchor.left(), anchor.bottom() + 2.0))
            .show(ctx, |ui| {
                // as wide as the window's own row allows, never past it: a
                // narrow window wraps the end of the bar onto a second line
                let width = (anchor.width() - 2.0 * 8.0).max(320.0);
                crate::ui::widgets::overlay(ui, p, egui::Margin::symmetric(8, 4), |ui| {
                    ui.set_max_width(width);
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.horizontal_wrapped(|ui| {
                        let layer = ui.layer_id();
                        if let Some(owner) = self.owner {
                            super::focus::claim(ui.ctx(), layer, owner);
                        }
                        self.row(ui, p, id, source, editable);
                    });
                });
            });
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, source: &mut String, editable: bool) {
        let ctx = ui.ctx().clone();
        if let Some(owner) = self.owner {
            super::focus::claim(&ctx, ui.layer_id(), owner);
        }
        let window = Id::unique(Self::WINDOW);
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

    /// The bar's one row: the query, the replacement, the options, the
    /// commands and what the last one said, in that order from the left, as an
    /// IDE's find bar reads.
    fn row(&mut self, ui: &mut egui::Ui, p: &Palette, id: Id, source: &mut String, editable: bool) {
        let ctx = ui.ctx().clone();
        self.query_field(ui, p, id, source, Self::POPUP_FIELD_WIDTH);
        crate::ui::widgets::text_field(ui, editable, &mut self.replacement, |b, _| {
            egui::TextEdit::singleline(b)
                .id(Id::unique(Self::WINDOW).with("replacement"))
                .hint_text(aurora_i18n::tr!("script-search-replacement"))
                .desired_width(Self::POPUP_FIELD_WIDTH)
        })
        .on_hover_text(aurora_i18n::tr!("script-search-replacement"));
        let before_case = self.insensitive;
        let before_words = self.whole_words;
        crate::ui::widgets::checkbox(ui, p, &mut self.insensitive, aurora_i18n::tr!("script-search-insensitive"));
        crate::ui::widgets::checkbox(ui, p, &mut self.up, aurora_i18n::tr!("script-search-up"));
        crate::ui::widgets::checkbox(ui, p, &mut self.whole_words, aurora_i18n::tr!("script-search-whole-words"));
        if (self.insensitive, self.whole_words) != (before_case, before_words) {
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
        if !self.status.is_empty() {
            ui.add_sized(
                Vec2::new(Self::POPUP_STATUS_WIDTH, ui.spacing().interact_size.y),
                egui::Label::new(RichText::new(&self.status).small().color(p.muted)).truncate(),
            )
            .on_hover_text(&self.status);
        }
        // the bar's own cross, as the panel's title carries one
        if crate::ui::widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-close"), true) {
            self.close();
        }
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

/// The editor's shortcuts, under Help: what each key does in a script's window.
pub(super) struct Shortcuts;

impl Shortcuts {
    pub const WINDOW: &'static str = "script_shortcuts";

    pub fn show(open: &mut bool, ctx: &egui::Context, p: &Palette) {
        if !*open {
            return;
        }
        crate::ui::widgets::Floater::new(
            Self::WINDOW,
            aurora_i18n::tr!("script-help-shortcuts"),
            ctx.content_rect().center() - Vec2::new(260.0, 365.0),
            Vec2::new(520.0, 730.0),
        )
        // the table starts under the title's line, as the editor's menus do
        .compact_header()
        .show(ctx, p, open, |ui| {
            // the keys of every row first: one column for all the groups, so
            // that what they do lines up from one group to the next
            let groups = groups();
            let caps: Vec<Vec<Vec<keycap::Keycap>>> = groups
                .iter()
                .map(|(_, _, rows)| rows.iter().map(|(binding, _)| keycap::caps(ui, binding, p.panel)).collect())
                .collect();
            let column = caps.iter().flatten().map(|caps| keycap::row_width(caps)).fold(0.0, f32::max);
            egui::ScrollArea::vertical().show(ui, |ui| {
                // a row is one line tall whatever its font, as `keycap::legend`
                // draws them, so that the three groups fit the window
                ui.spacing_mut().item_spacing.y = 0.0;
                for (at, ((icon, heading, rows), caps)) in groups.iter().zip(&caps).enumerate() {
                    ui.add_space(if at == 0 { 2.0 } else { 10.0 });
                    crate::ui::widgets::section_header(ui, p, icon, heading);
                    for (caps, (_, label)) in caps.iter().zip(rows) {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::hover());
                        keycap::paint_row(ui, p, caps, rect.left_center(), p.panel);
                        ui.painter().text(
                            egui::pos2(rect.left() + column + 10.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            label,
                            crate::ui::widgets::control_font(ui, 13.0),
                            p.ink,
                        );
                    }
                }
            });
        });
    }
}

/// One key of the table: a physical key code, and whether Ctrl and Shift come
/// with it.
fn key(code: &str, ctrl: bool, shift: bool) -> crate::keybinds::Binding {
    crate::keybinds::Binding {
        input: crate::keybinds::Input::Key(code.into()),
        ctrl,
        shift,
        alt: false,
    }
}

/// A mouse button: the left one, under Ctrl, is "Ctrl+click".
fn click(button: &str, ctrl: bool) -> crate::keybinds::Binding {
    crate::keybinds::Binding {
        input: crate::keybinds::Input::Mouse(button.into()),
        ctrl,
        shift: false,
        alt: false,
    }
}

/// A group of the table: its icon, its heading, and one row per key.
type Group = (&'static str, String, Vec<(crate::keybinds::Binding, String)>);

/// The table, grouped: what a script's window does with its files, how to find
/// one's way in a script, and what helps write it.
fn groups() -> Vec<Group> {
    vec![
        (
            "files-filled",
            aurora_i18n::tr!("script-shortcuts-files").to_string(),
            vec![
                (key("S", true, false), aurora_i18n::tr!("script-shortcuts-save").to_string()),
                (key("S", true, true), aurora_i18n::tr!("script-shortcuts-save-all").to_string()),
                (key("O", true, false), aurora_i18n::tr!("script-shortcuts-open").to_string()),
                (key("W", true, false), aurora_i18n::tr!("script-shortcuts-close").to_string()),
                (key("Tab", true, false), aurora_i18n::tr!("script-shortcuts-next").to_string()),
                (key("Tab", true, true), aurora_i18n::tr!("script-shortcuts-previous").to_string()),
                (key("1…6", true, false), aurora_i18n::tr!("script-shortcuts-dock-tabs").to_string()),
                (key("\\", true, false), aurora_i18n::tr!("script-shortcuts-split").to_string()),
            ],
        ),
        (
            "search-filled",
            aurora_i18n::tr!("script-shortcuts-find").to_string(),
            vec![
                (key("F", true, false), aurora_i18n::tr!("script-shortcuts-find-key").to_string()),
                (key("G", true, false), aurora_i18n::tr!("script-shortcuts-goto").to_string()),
                (
                    key("F8", false, false),
                    aurora_i18n::tr!("script-shortcuts-problem-next").to_string(),
                ),
                (
                    key("F8", false, true),
                    aurora_i18n::tr!("script-shortcuts-problem-previous").to_string(),
                ),
                (
                    key("F2", false, false),
                    aurora_i18n::tr!("script-shortcuts-bookmark-next").to_string(),
                ),
                (
                    key("F2", false, true),
                    aurora_i18n::tr!("script-shortcuts-bookmark-previous").to_string(),
                ),
                (
                    key("F2", true, false),
                    aurora_i18n::tr!("script-shortcuts-bookmark-mark").to_string(),
                ),
                (click("Left", true), aurora_i18n::tr!("script-shortcuts-definition").to_string()),
            ],
        ),
        (
            "pencil-filled",
            aurora_i18n::tr!("script-shortcuts-code").to_string(),
            vec![
                (key("K", true, false), aurora_i18n::tr!("script-shortcuts-comment").to_string()),
                (
                    key("Slash", true, false),
                    aurora_i18n::tr!("script-shortcuts-line-comment").to_string(),
                ),
                (key("K", true, true), aurora_i18n::tr!("script-shortcuts-block-comment").to_string()),
                (key("Tab", false, false), aurora_i18n::tr!("script-shortcuts-indent").to_string()),
                (key("Tab", false, true), aurora_i18n::tr!("script-shortcuts-outdent").to_string()),
                (key("Z", true, false), aurora_i18n::tr!("script-shortcuts-undo").to_string()),
                (key("Y", true, false), aurora_i18n::tr!("script-shortcuts-redo").to_string()),
                (
                    key("ArrowDown", false, false),
                    aurora_i18n::tr!("script-shortcuts-list").to_string(),
                ),
                (
                    key("Enter", false, false),
                    aurora_i18n::tr!("script-shortcuts-list-take").to_string(),
                ),
                (
                    key("Escape", false, false),
                    aurora_i18n::tr!("script-shortcuts-dismiss").to_string(),
                ),
            ],
        ),
    ]
}

/// One page of the preferences: its tab in the row names it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Look,
    Editing,
    Preprocessor,
    External,
    Layout,
}

#[derive(Default)]
pub(super) struct Preferences {
    pub open: bool,
    page: Page,
    preview: Highlighter,
    directory: Option<std::sync::mpsc::Receiver<Option<String>>>,
}

impl Preferences {
    /// The window of the preferences, as `Floater` is given it.
    pub const WINDOW: &'static str = "script_preferences";

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
        let mut open = self.open;
        Floater::new(
            Self::WINDOW,
            aurora_i18n::tr!("script-preferences-title"),
            ctx.content_rect().center() - Vec2::new(380.0, 320.0),
            Vec2::new(760.0, 640.0),
        )
        .show(ctx, p, &mut open, |ui| {
            // the pages, as the other windows row their tabs: pills along the
            // top, each with its icon, the page itself under them (one scroll
            // area, so that a page longer than the window keeps its room)
            let tabs = [
                (
                    Page::Look,
                    "palette-filled",
                    aurora_i18n::tr!("script-preferences-look").to_string(),
                ),
                (
                    Page::Editing,
                    "keyboard-filled",
                    aurora_i18n::tr!("script-preferences-editing").to_string(),
                ),
                (
                    Page::Preprocessor,
                    "replace-filled",
                    aurora_i18n::tr!("script-preferences-preprocessing").to_string(),
                ),
                (
                    Page::External,
                    "external-link-filled",
                    aurora_i18n::tr!("script-preferences-external-page").to_string(),
                ),
                (
                    Page::Layout,
                    "layout-filled",
                    aurora_i18n::tr!("script-preferences-layout").to_string(),
                ),
            ];
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (page, icon, label) in &tabs {
                    if crate::ui::widgets::pill_tab(ui, p, icon, label, None, false, self.page == *page, true).clicked() {
                        self.page = *page;
                    }
                }
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("script_preferences_page")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match self.page {
                        Page::Look => self.look(ui, p, settings),
                        Page::Editing => self.editing(ui, p, settings),
                        Page::Preprocessor => self.preprocessor(ui, p, settings),
                        Page::External => self.external(ui, p, settings),
                        Page::Layout => self.layout(ui, p, settings),
                    }
                });
        });
        self.open = open;
        *settings != before
    }

    /// The colours, the font and the preview: what a script looks like.
    fn look(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut ScriptSettings) {
        crate::ui::widgets::section_header(ui, p, "palette-filled", aurora_i18n::tr!("script-preferences-look").as_ref());
        egui::Grid::new("script_colors").num_columns(4).show(ui, |ui| {
            for (i, role) in ColorRole::ALL.into_iter().enumerate() {
                let mut color = settings.color(role, p).to_srgba_unmultiplied();
                if crate::ui::widgets::color_button(ui, p, ("script_color", i), &mut color, true).changed() {
                    settings.colors.insert(role, color);
                }
                ui.label(role.label());
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
        if flat_button(ui, p, aurora_i18n::tr!("script-preferences-reset-colors")).clicked() {
            settings.colors.clear();
        }
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-preferences-font-size"));
            ui.add(egui::DragValue::new(&mut settings.font_size).range(8.0..=32.0));
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-preferences-font"));
            let builtin = aurora_i18n::tr!("script-font-monospace").to_string();
            let choices = crate::ui::fonts::editor_font_choices();
            let said = if settings.font.is_empty() || !choices.contains(&settings.font) {
                builtin.clone()
            } else {
                settings.font.clone()
            };
            egui::ComboBox::from_id_salt("script_editor_font")
                .selected_text(said)
                .width(220.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut settings.font, String::new(), &builtin);
                    for family in choices {
                        ui.selectable_value(&mut settings.font, family.clone(), family);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-preferences-tab-size"));
            ui.add(egui::DragValue::new(&mut settings.tab_size).range(2..=8));
        });
        let preview = "/* LSL */\ninteger counter = 0;\ndefault\n{\n    state_entry()\n    {\n        string greeting = \"Hello\";\n        llSay(PUBLIC_CHANNEL, greeting);\n    }\n}";
        egui::Frame::new()
            .fill(settings.color(ColorRole::Background, p))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.add(egui::Label::new(self.preview.custom(
                    preview,
                    p,
                    settings.font_id(ui.ctx()),
                    settings,
                    "",
                    false,
                    false,
                    Lang::Lsl,
                )));
            });
    }

    /// What the editor does as it is written: the layout, the minimap, the
    /// aids, the completion and the linter.
    fn editing(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut ScriptSettings) {
        crate::ui::widgets::section_header(ui, p, "keyboard-filled", aurora_i18n::tr!("script-preferences-editing").as_ref());
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.simple, aurora_i18n::tr!("script-preferences-simple"))
            .on_hover_text(aurora_i18n::tr!("script-preferences-simple-hint"));
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.word_wrap, aurora_i18n::tr!("script-preferences-word-wrap"));
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.minimap, aurora_i18n::tr!("script-preferences-minimap"));
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.preview, aurora_i18n::tr!("script-preferences-preview"))
            .on_hover_text(aurora_i18n::tr!("script-preferences-preview-hint"));
        ui.add_enabled_ui(settings.minimap, |ui| {
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("script-preferences-minimap-width"));
                ui.add(egui::DragValue::new(&mut settings.minimap_width).range(40..=100).suffix(" pt"));
                ui.add_space(10.0);
                ui.label(aurora_i18n::tr!("script-preferences-minimap-opacity"));
                ui.add(egui::DragValue::new(&mut settings.minimap_opacity).range(10..=100).suffix(" %"));
            });
        });
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.inlays, aurora_i18n::tr!("script-preferences-inlays"))
            .on_hover_text(aurora_i18n::tr!("script-preferences-inlays-hint"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.search_prefer_left,
            aurora_i18n::tr!("script-preferences-search-left"),
        )
        .on_hover_text(aurora_i18n::tr!("script-preferences-search-left-hint"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.search_inline,
            aurora_i18n::tr!("script-preferences-search-popup"),
        )
        .on_hover_text(aurora_i18n::tr!("script-preferences-search-popup-hint"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.convert_tabs,
            aurora_i18n::tr!("script-preferences-convert-tabs"),
        );
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.new_script_lua,
            aurora_i18n::tr!("script-preferences-new-script-lua"),
        )
        .on_hover_text(aurora_i18n::tr!("script-preferences-new-script-lua-hint"));
        crate::ui::widgets::divider(ui, p);
        crate::ui::widgets::section_header(ui, p, "eye-filled", aurora_i18n::tr!("script-preferences-aids").as_ref());
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.sticky_line, aurora_i18n::tr!("script-preferences-sticky-line"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.indent_guides,
            aurora_i18n::tr!("script-preferences-indent-guides"),
        );
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.change_bars, aurora_i18n::tr!("script-preferences-change-bars"));
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.occurrences, aurora_i18n::tr!("script-preferences-occurrences"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.bracket_match,
            aurora_i18n::tr!("script-preferences-bracket-match"),
        );
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.current_line,
            aurora_i18n::tr!("script-preferences-current-line"),
        );
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.completion, aurora_i18n::tr!("script-preferences-completion"));
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.deprecations,
            aurora_i18n::tr!("script-preferences-deprecations"),
        )
        .on_hover_text(aurora_i18n::tr!("script-preferences-deprecations-hint"));
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.arguments, aurora_i18n::tr!("script-preferences-arguments"))
            .on_hover_text(aurora_i18n::tr!("script-preferences-arguments-hint"));
    }

    /// The preprocessor and the dialect it lowers to LSL.
    fn preprocessor(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut ScriptSettings) {
        crate::ui::widgets::section_header(
            ui,
            p,
            "replace-filled",
            aurora_i18n::tr!("script-preferences-preprocessing").as_ref(),
        );
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.preprocessor,
            aurora_i18n::tr!("script-preferences-preprocessor"),
        );
        ui.add_enabled_ui(settings.preprocessor, |ui| {
            crate::ui::widgets::switch_labeled(
                ui,
                p,
                &mut settings.disk_includes,
                aurora_i18n::tr!("script-preferences-disk-includes"),
            );
            ui.label(aurora_i18n::tr!("script-preferences-include-path"));
            ui.add_enabled_ui(settings.disk_includes, |ui| {
                ui.horizontal(|ui| {
                    let field = (ui.available_width() - 40.0).max(60.0);
                    ui.add(egui::TextEdit::singleline(&mut settings.include_directory).desired_width(field));
                    let choose = aurora_i18n::tr!("script-preferences-choose-directory");
                    if icon_button(ui, p, "folder-open-filled", choose, self.directory.is_none()) {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let directory = settings.include_directory.clone();
                        self.directory = Some(rx);
                        std::thread::spawn(move || {
                            let path = rfd::FileDialog::new()
                                .set_directory(directory)
                                .pick_folder()
                                .map(|p| p.display().to_string());
                            let _ = tx.send(path);
                        });
                    }
                });
            });
            crate::ui::widgets::divider(ui, p);
            crate::ui::widgets::section_header(
                ui,
                p,
                "code-circle-filled",
                aurora_i18n::tr!("script-preferences-extensions").as_ref(),
            );
            crate::ui::widgets::switch_labeled(ui, p, &mut settings.switches, aurora_i18n::tr!("script-preferences-switches"));
            crate::ui::widgets::switch_labeled(ui, p, &mut settings.lazy_lists, aurora_i18n::tr!("script-preferences-lazy-lists"));
        });
    }

    /// The outside editor: the program, and the WebSocket an editor follows
    /// the scripts through.
    fn external(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut ScriptSettings) {
        crate::ui::widgets::section_header(
            ui,
            p,
            "external-link-filled",
            aurora_i18n::tr!("script-preferences-external-sync").as_ref(),
        );
        crate::ui::widgets::switch_labeled(ui, p, &mut settings.recompile, aurora_i18n::tr!("script-preferences-recompile"));
        ui.label(aurora_i18n::tr!("script-preferences-external"));
        ui.add(egui::TextEdit::singleline(&mut settings.external_editor).desired_width(f32::INFINITY));
        ui.label(
            RichText::new(aurora_i18n::tr!("script-preferences-external-hint"))
                .small()
                .color(p.muted),
        );
        crate::ui::widgets::divider(ui, p);
        crate::ui::widgets::switch_labeled(
            ui,
            p,
            &mut settings.websocket_sync,
            aurora_i18n::tr!("script-preferences-websocket"),
        );
        ui.add_enabled_ui(settings.websocket_sync, |ui| {
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("script-preferences-websocket-port"));
                ui.add(egui::DragValue::new(&mut settings.websocket_port).range(1024..=u16::MAX));
            });
            crate::ui::widgets::switch_labeled(
                ui,
                p,
                &mut settings.websocket_forward_debug,
                aurora_i18n::tr!("script-preferences-websocket-debug"),
            );
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("script-preferences-websocket-hint"))
                .small()
                .color(p.muted),
        );
    }

    /// The room the reader dragged: the tree's width, the dock's height and
    /// its tabs' order, and the way back from them.
    fn layout(&mut self, ui: &mut egui::Ui, p: &Palette, settings: &mut ScriptSettings) {
        crate::ui::widgets::section_header(ui, p, "layout-filled", aurora_i18n::tr!("script-preferences-layout").as_ref());
        if flat_button(ui, p, aurora_i18n::tr!("script-preferences-reset-layout")).clicked() {
            settings.tree_width = 0;
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("script-preferences-reset-layout-hint"))
                .small()
                .color(p.muted),
        );
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
