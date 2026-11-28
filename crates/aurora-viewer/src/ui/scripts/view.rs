//! The script window's drawing: the menus and the toolbar, the source with its
//! gutter, its sticky line, its minimap and its path, the docks under it, and
//! the status bar -- what the Legacy Viewer's script editor has of them
//! (LLScriptEdCore, LLScriptEditor, indra/newview, originally LGPL 2.1), and
//! Aurora's own for the docks, the split and the documentation pane. Aurora
//! keeps its flat widgets and its always-visible experience list.

use super::Lang;

use super::{
    Action, Editor, complete, crumbs,
    dialogs::{Panels, Search},
    editing, files, indent, reference,
    settings::{ColorRole, ScriptSettings},
};
use crate::{
    theme::Palette,
    ui::{
        menu,
        widgets::{self, Floater, flat_button, icon_button_sized, icon_toggle_sized},
    },
};
use aurora_net::scripts::ScriptTarget;
use egui::text::{LayoutJob, TextFormat};
use egui::{Id, RichText, Vec2};

#[derive(Default)]
pub(super) struct Tools {
    /// The window's analysis of the text in front, with the hash of what it was
    /// made of -- `None` before the first one (`Editor::analysis`).
    pub(super) analysis: (Option<u64>, Analysis),
    /// The hash of the text shown, with the clock of the frame it was made in
    /// (`Editor::text_key`): the caches that key on the text share it.
    pub(super) text_key: Option<(f64, u64)>,
    /// The parameter names of the calls in the text on screen, with the hash of
    /// what they were made of (`update_inlays`): the room they take in the
    /// layout, and the pills painted into it.
    inlays: Option<(u64, Vec<super::inlays::Inlay>)>,
    pub focus_start: bool,
    /// The keyboard to this script, asked once every source of the frame is
    /// drawn: a request made before a text edit draws does not hold (a tab
    /// chosen by a key or a click takes it).
    pub focus_request: bool,
    pub body_top: f32,
    /// The window's menus and toolbar this frame, where Find / Replace hang
    /// when the setting puts them in the window (`settings.search_inline`):
    /// a popup just under them, over the source. None for the panel beside
    /// the window.
    pub search_anchor: Option<egui::Rect>,
    /// The name the window's documentation pane reads next, if a command or
    /// the Insert menu's list asked for one (`Help` > Documentation, the
    /// Insert menu): taken by the pane.
    pub insert_filter: String,
    pub insert_button: Option<egui::Rect>,
    insertion_open: bool,
    /// The Code menu's button this frame, where the demo clicks it.
    pub code_button: Option<egui::Rect>,
    pub goto: bool,
    pub goto_line: usize,
    pub processed: bool,
    pub processed_source: String,
    pub has_processed: bool,
    /// Where the strip (the minimap, or the thin overview) was drawn this
    /// frame.
    pub overview: Option<egui::Rect>,
    /// The minimap of the text as last made, with the text's hash: the map is
    /// made again only when the script changes.
    minimap: Option<(u64, super::minimap::Map)>,
    pub position: (usize, usize),
    /// The prim that holds this script, when its window is a linkset's: the
    /// first word of the path under the source. Empty in a window of its own.
    pub prim: String,
    pub scroll_to_cursor: bool,
    /// The line at the top of the source's viewport, 0-based, as the last
    /// frame showed it: what the sticky line above the text names.
    pub first_visible: usize,
    /// The line at its bottom, for the minimap's window.
    pub last_visible: usize,
    /// A click on the text this frame, without Ctrl: the split's pane that is
    /// not in front takes the front with it.
    pub clicked: bool,
    /// The name Ctrl+click (or the context menu) asked the window to take the
    /// caret to the declaration of.
    pub goto_word: Option<String>,
    /// The word the context menu was opened on, read from the right click
    /// itself: the pointer leaves the text to reach the menu's own rows, and a
    /// word read from the hover would go with it.
    pub menu_word: String,
    /// The word the context menu asked the dock to find in the window's
    /// scripts.
    pub references_word: Option<String>,
    pub revert: bool,
    pub file_job: Option<files::Job>,
    pub file_snapshot: String,
    pub external: Option<files::Watch>,
    /// The external editor is being launched: the script stays followed
    /// by the editor sync while its watch is replaced.
    pub external_starting: bool,
    pub external_snapshot: String,
    pub incoming: Option<String>,
    incoming_external: bool,
    pub diagnostics_processed: bool,
    pub reset_history: bool,
    /// The notice over this source the reader has put away: it comes back when
    /// another one takes its place.
    pub(super) notice_hidden: Option<Notice>,
    /// The dock's Check again, asked: the window reads again the object's
    /// scripts that could not be read.
    /// The snippet the reader is filling, its placeholders walked with Tab.
    pub(super) snippet: Option<complete::Snippet>,
    footer_height: f32,
    can_undo: bool,
    can_redo: bool,
    history_cursor: Option<egui::text::CCursorRange>,
}

impl Tools {
    pub fn opening() -> Self {
        Self {
            focus_start: true,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Command {
    #[default]
    Save,
    SaveAll,
    Reload,
    Revert,
    Properties,
    Open,
    Export,
    Preferences,
    External,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Deselect,
    Find,
    Goto,
    Format,
    Help,
    Shortcuts,
    Indent,
    Outdent,
    LineComment,
    BlockComment,
    FoldAll,
    ExpandAll,
}

fn visible_id(id: Id, processed: bool) -> Id {
    if processed { id.with("processed") } else { id }
}

fn text(e: &Editor) -> &str {
    if e.tools.processed { &e.tools.processed_source } else { &e.source }
}

fn enabled(command: Command, e: &Editor, ctx: &egui::Context, id: Id) -> bool {
    let editable = e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed && !e.colors.active();
    let selection = !editing::cursor(ctx, visible_id(id, e.tools.processed)).is_empty();
    match command {
        Command::Save => e.can_save(),
        Command::Reload => e.can_read && !e.dirty() && e.saving.is_none() && e.loading.is_none(),
        Command::Revert => editable && e.dirty(),
        Command::Open | Command::External => editable && e.tools.file_job.is_none(),
        // a read-only editor saves no file (LLTextEditor::canLoadOrSaveToFile,
        // indra/llui/lltexteditor.cpp: !mReadOnly, set from is_modifiable in
        // LLPreviewLSL::loadAsset, indra/newview/llpreviewscript.cpp)
        Command::Export => e.loaded && e.can_edit && e.tools.file_job.is_none(),
        Command::Cut => editable && selection,
        Command::Copy => e.loaded && selection,
        Command::Paste => editable,
        Command::Undo => editable && e.tools.can_undo,
        Command::Redo => editable && e.tools.can_redo,
        Command::Indent | Command::Outdent | Command::LineComment | Command::BlockComment | Command::Format => editable,
        Command::FoldAll | Command::ExpandAll => e.loaded,
        Command::SelectAll | Command::Deselect | Command::Find | Command::Goto => e.loaded,
        _ => true,
    }
}

pub(super) fn run(
    command: Command,
    e: &mut Editor,
    ctx: &egui::Context,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
) {
    if !enabled(command, e, ctx, id) {
        return;
    }
    let owner = id;
    let id = visible_id(id, e.tools.processed);
    match command {
        Command::Save => {
            if let Some(action) = e.save() {
                actions.push(action);
            }
        }
        Command::SaveAll => actions.push(Action::SaveAll {
            linkset: e.linkset,
            target: e.target,
        }),
        Command::Reload => {
            let request = uuid::Uuid::new_v4();
            e.loading = Some(request);
            e.loaded = false;
            e.status = aurora_i18n::tr!("common-loading-2").into();
            actions.push(Action::Reload { request, target: e.target });
        }
        Command::Revert => e.tools.revert = true,
        Command::Open => {
            e.tools.file_snapshot.clone_from(&e.source);
            e.tools.file_job = Some(files::open(settings.convert_tabs));
        }
        Command::Export => e.tools.file_job = Some(files::save(e.name.clone(), e.source.clone(), e.lang())),
        Command::Properties => actions.push(Action::Properties(e.target)),
        Command::Preferences => {
            // shown again, a minimized window expands (LLFloaterReg::showInstance),
            // and the panel is drawn under the windows: bring it over them, or a
            // large script window hides it entirely
            crate::ui::widgets::open_floater(ctx, super::dialogs::Preferences::WINDOW, &mut prefs.preferences.open);
            crate::ui::widgets::bring_floater_front(ctx, super::dialogs::Preferences::WINDOW);
        }
        Command::External => {
            e.tools.file_snapshot.clone_from(&e.source);
            // the script takes the same file again: the watch of the last
            // launch ends now, not once the new one has written it
            e.tools.external = None;
            e.tools.external_starting = true;
            e.tools.file_job = Some(files::external(
                e.name.clone(),
                aurora_editor_sync::subscriptions::script_id(e.target.object, e.target.item),
                e.source.clone(),
                settings.external_editor.clone(),
                settings.convert_tabs,
                e.lang(),
            ));
        }
        Command::Undo | Command::Redo => {
            editing::undo(ctx, id, &mut e.source, matches!(command, Command::Redo));
            e.tools.scroll_to_cursor = true;
        }
        Command::Cut | Command::Copy => {
            let selected = editing::bytes(text(e), editing::cursor(ctx, id));
            ctx.copy_text(text(e)[selected].into());
            if matches!(command, Command::Cut) {
                let [a, b] = editing::cursor(ctx, id).sorted_cursors();
                editing::replace(ctx, id, &mut e.source, a.index.0..b.index.0, "");
            }
        }
        Command::Paste => actions.push(Action::Paste(e.target)),
        Command::SelectAll => editing::select(ctx, id, 0..text(e).chars().count(), true),
        Command::Deselect => {
            let at = editing::cursor(ctx, id).primary.index.0;
            editing::select(ctx, id, at..at, true);
        }
        Command::Find => prefs.search.begin(ctx, owner, id, text(e)),
        Command::Goto => e.tools.goto = true,
        Command::Format => {
            let formatted = super::format::text(&e.source, e.lang());
            if formatted == e.source {
                e.status = aurora_i18n::tr!("script-format-unchanged").into();
            } else {
                let caret = super::format::caret(&e.source, &formatted, editing::cursor(ctx, id).primary.index.0);
                let range = 0..e.source.chars().count();
                // one undo step, the caret on the same character of the code
                if editing::replace(ctx, id, &mut e.source, range, &formatted) {
                    editing::select(ctx, id, caret..caret, true);
                    e.tools.scroll_to_cursor = true;
                    e.status = aurora_i18n::tr!("script-format-done").into();
                } else {
                    e.status = aurora_i18n::tr!("script-source-too-large").into();
                }
            }
        }
        Command::Shortcuts => {
            prefs.shortcuts = true;
            // the windows are drawn before this one: bring it over them
            crate::ui::widgets::bring_floater_front(ctx, super::dialogs::Shortcuts::WINDOW);
        }
        Command::Help => ctx.open_url(egui::OpenUrl::new_tab(reference::url("", e.lang()))),
        Command::Indent | Command::Outdent | Command::LineComment | Command::BlockComment => {
            let command = match command {
                Command::Indent => editing::SelectionCommand::Indent,
                Command::Outdent => editing::SelectionCommand::Outdent,
                Command::LineComment => editing::SelectionCommand::LineComment,
                Command::BlockComment => editing::SelectionCommand::BlockComment,
                _ => unreachable!("selection command"),
            };
            if !editing::selection_command(ctx, id, &mut e.source, command) {
                e.status = aurora_i18n::tr!("script-selection-edit-failed").into();
            }
        }
        Command::FoldAll => {
            let lang = e.lang();
            // field by field: the folds take a mutable borrow of themselves
            e.folding
                .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source }, lang);
            e.folding.collapse_all();
            editing::select(ctx, id, 0..0, false);
        }
        Command::ExpandAll => e.folding.expand_all(),
    }
}

/// One line of the Problems dock: a diagnostic of a script the window shows.
#[derive(Clone)]
pub(super) struct Problem {
    pub source: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// What the window reads of the script in front every frame -- its blocks and
/// its outline -- made once for the text they were made of
/// (`Editor::analysis`).
#[derive(Default)]
pub(super) struct Analysis {
    pub outline: Vec<super::outline::Item>,
    /// LSL: the text's chars, which of them are code, where its lines start and
    /// its blocks, for the path under the caret.
    chars: Vec<char>,
    code: Vec<bool>,
    starts: Vec<usize>,
    blocks: Vec<crumbs::Block>,
}

impl Analysis {
    /// `text` is what the source shows -- the preprocessed text when the Output
    /// view is on -- and `source` the script's own.
    pub(super) fn make(text: &str, lang: Lang) -> Self {
        let lua = lang.is_lua();
        let outline = super::outline::outline(text, lang);
        let mut analysis = Self {
            outline,
            chars: Vec::new(),
            code: Vec::new(),
            starts: Vec::new(),
            blocks: Vec::new(),
        };
        // SLua has no braces: its path follows the outline
        if !lua {
            analysis.chars = text.chars().collect();
            analysis.code = editing::code_mask(&analysis.chars);
            analysis.starts = crumbs::line_starts(&analysis.chars);
            analysis.blocks = crumbs::blocks(&analysis.chars, &analysis.code);
        }
        analysis
    }

    /// The path of the block that holds `line`, outermost first.
    pub(super) fn path(&self, line: usize, lang: Lang) -> Vec<crumbs::Step> {
        let lua = lang.is_lua();
        if lua {
            crumbs::lua_path_in(&self.outline, line)
        } else {
            crumbs::path_in(&self.chars, &self.code, &self.starts, &self.blocks, line)
        }
    }
}

/// What a script has to reckon with, as the notice over its source says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    /// Its item could not be read: the viewer's reason is in the status.
    Failed,
    /// Saved, but the compiler refused the source.
    CompileFailed,
    /// Longer than the viewer can save.
    TooLarge,
    /// Readable, but the item gives no modify right.
    ReadOnly,
    /// The simulator has not said whether the script runs.
    NotRunning,
    /// A Lua script writes a preprocessor directive, which never runs on Lua.
    LslOnly,
}

/// The notice a script asks for, the most urgent first. A failed compile stays
/// over the source until a save or a load settles it (or the reader puts it
/// away): a save's word goes to the Console, not to `status`, so the banner
/// does not depend on that word.
fn notice(e: &Editor) -> Option<Notice> {
    if !e.loaded && e.loading.is_none() && !e.status.is_empty() {
        Some(Notice::Failed)
    } else if e.compile_failed {
        Some(Notice::CompileFailed)
    } else if e.source.len() > aurora_net::scripts::MAX_SOURCE {
        Some(Notice::TooLarge)
    } else if e.loaded && !e.can_edit {
        Some(Notice::ReadOnly)
    } else if !e.target.object.is_nil() && e.loaded && e.can_read && e.running.is_none() {
        Some(Notice::NotRunning)
    } else if e.lua() && writes_a_directive(&e.source) {
        Some(Notice::LslOnly)
    } else {
        None
    }
}

/// Whether a script writes a preprocessor directive: a line of its own that
/// starts with `#define` or `#include`.
fn writes_a_directive(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("#define") || line.starts_with("#include")
    })
}

/// The notice over the editor: what the script in front has to reckon with,
/// what can be done about it, and a cross that keeps it out of the way until
/// another one comes up (an IDE's notice bar, in one row of Aurora's own).
pub(super) fn notice_bar(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, actions: &mut Vec<Action>) {
    let Some(kind) = notice(e) else {
        e.tools.notice_hidden = None;
        return;
    };
    if e.tools.notice_hidden == Some(kind) {
        return;
    }
    let (icon, tint, text) = match kind {
        Notice::Failed => ("alert-triangle-filled", p.danger, e.status.clone()),
        Notice::CompileFailed => (
            "alert-triangle-filled",
            p.danger,
            aurora_i18n::tr!("script-compile-failed").to_string(),
        ),
        Notice::TooLarge => (
            "alert-triangle-filled",
            p.danger,
            aurora_i18n::tr!("script-source-too-large").to_string(),
        ),
        Notice::ReadOnly => ("lock-filled", p.warn, aurora_i18n::tr!("script-read-only").to_string()),
        Notice::NotRunning => ("hourglass-filled", p.warn, aurora_i18n::tr!("script-waiting-running").to_string()),
        Notice::LslOnly => ("info-circle-filled", p.warn, aurora_i18n::tr!("script-notice-lsl-only").to_string()),
    };
    let height = ui.spacing().interact_size.y.max(24.0);
    egui::Frame::new()
        .fill(p.bar.lerp_to_gamma(tint, 0.18))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                super::linkset::icon(ui, icon, tint);
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), height),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("script-notice-hide"), true) {
                            e.tools.notice_hidden = Some(kind);
                        }
                        if kind == Notice::NotRunning && flat_button(ui, p, aurora_i18n::tr!("script-retry")).clicked() {
                            actions.push(Action::QueryRunning(e.target));
                        }
                        // the words take the room left, on the left
                        let width = ui.available_width();
                        ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&text).size(11.0).color(p.ink))
                                    .truncate()
                                    .halign(egui::Align::LEFT),
                            )
                            .on_hover_text(&text);
                        });
                    },
                );
            });
        });
}

pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &mut ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    own_menus: bool,
    problems: &[Problem],
    stats: Option<super::ScriptStats>,
    bar: bool,
) {
    e.tools.body_top = ui.cursor().top();
    // A script that does not exist yet: the page that asks which language it
    // will be written in takes the source's place, and the choice creates it
    // (`create::page`). The page keeps the tab, the tree and the menus.
    if let Some(draft) = e.draft.clone() {
        let now = ui.input(|input| input.time);
        // an object that never made the script gives the page back, so the
        // reader can choose again instead of waiting on a page that says so
        if draft.chosen
            && now - draft.chosen_at > 8.0
            && let Some(draft) = e.draft.as_mut()
        {
            draft.chosen = false;
        }
        if let Some(lua) = super::create::page(ui, p, &draft)
            && !draft.chosen
        {
            actions.push(Action::NewScript { object: draft.object, lua });
            if let Some(draft) = e.draft.as_mut() {
                draft.chosen = true;
                draft.chosen_at = now;
            }
        }
        return;
    }
    if own_menus {
        // a window of its own: its menus and toolbar are the body's first rows
        e.tools.search_anchor = None;
    }
    // the notice of what the script has to reckon with: a window of its own
    // puts it under its menus; a linkset window puts it over its tree and its
    // sources (`show_linkset`), so that both start level
    if own_menus {
        notice_bar(ui, p, e, actions);
    }
    if own_menus {
        let header = ui.scope(|ui| header(ui, p, e, id, settings, prefs, actions, own_menus));
        // Find / Replace hang in a popup under the menus and the toolbar, over
        // the source: the room the source has does not change with them
        e.tools.search_anchor = settings.search_inline.then_some(header.response.rect);
    }
    let room = ui.available_height();
    let gap = ui.spacing().item_spacing.y;
    let footer = footer_room(e.tools.footer_height, bar);
    let column = column(room, footer, gap);
    let mut column_ui = |ui: &mut egui::Ui, settings: &mut ScriptSettings| {
        let top = ui.cursor().min - Vec2::new(3.0, 0.0);
        filament_edge(ui, p, top, column.source, 0.55);
        source(ui, p, e, id, settings, column.source, &prefs.search, problems);
        if column.crumbs {
            ui.add_space(AIR);
            crumbs_row(ui, p, e, id, settings, true);
        }
    };
    column_ui(ui, settings);
    if let Some(change) = footer_row(ui, p, e, bar, id, settings, stats) {
        actions.push(Action::BarChange(change));
    }
}

/// The quiet light down the source's left edge, fading out well before the
/// dock: the tab in front is this pane, and the light says so without another
/// border. Drawn after the source, which fills its frame over the pane's own
/// edge.
fn filament_edge(ui: &egui::Ui, p: &Palette, top: egui::Pos2, height: f32, alpha: f32) {
    let length = (height * 0.45).clamp(24.0, 220.0);
    let line = egui::Rect::from_min_max(top, egui::pos2(top.x + 2.0, top.y + length));
    crate::ui::loading::gradient_rect(
        ui.painter(),
        line,
        p.violet.gamma_multiply(alpha),
        egui::Color32::TRANSPARENT,
        false,
    );
}

/// Which pane of a split holds the script in front: the window's dock, its
/// readout and its search are that one's.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// Two scripts side by side in one window: the rows above each pane's source
/// on one line (so both sources start level), then both sources, then the
/// window's dock and the status bar of the script in front. Clicking a pane's
/// source brings that script to the front, without moving either pane.
#[allow(clippy::too_many_arguments)]
pub(super) fn split_body(
    ui: &mut egui::Ui,
    p: &Palette,
    left: &mut Editor,
    left_id: Id,
    right: &mut Editor,
    right_id: Id,
    focused: Side,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    problems: &[Problem],
    ratio: &mut f32,
    stats: Option<super::ScriptStats>,
    bar: bool,
) -> Option<ScriptTarget> {
    // Two equal columns: egui splits the width exactly, and each column's own
    // layout keeps the two sources at the same height.
    let room = ui.available_height();
    let gap_y = ui.spacing().item_spacing.y;
    let column = column(room, footer_room(left.tools.footer_height, bar), gap_y);
    // the width the reader dragged the two panes to, the divider between them
    let total = ui.available_width();
    let (left_w, right_w) = split_widths(total, *ratio);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, column.source + SOURCE_PAINT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                left.tools.body_top = ui.cursor().top();
                let top = ui.cursor().min - Vec2::new(3.0, 0.0);
                filament_edge(ui, p, top, column.source, if focused == Side::Left { 0.55 } else { 0.0 });
                source(ui, p, left, left_id, settings, column.source, &prefs.search, problems);
            },
        );
        split_edge(ui, p, total, column.source + SOURCE_PAINT, ratio);
        ui.allocate_ui_with_layout(
            Vec2::new(right_w, column.source + SOURCE_PAINT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                right.tools.body_top = ui.cursor().top();
                let top = ui.cursor().min - Vec2::new(3.0, 0.0);
                filament_edge(ui, p, top, column.source, if focused == Side::Right { 0.55 } else { 0.0 });
                source(ui, p, right, right_id, settings, column.source, &prefs.search, problems);
            },
        );
    });
    if column.crumbs {
        ui.add_space(AIR);
        // the path of each pane keeps its pane's width
        let total = ui.available_width();
        let (left_w, right_w) = split_widths(total, *ratio);
        let crumbs = CRUMBS_HEIGHT;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.allocate_ui_with_layout(Vec2::new(left_w, crumbs), egui::Layout::top_down(egui::Align::Min), |ui| {
                crumbs_row(ui, p, left, left_id, settings, focused == Side::Left)
            });
            ui.add_space(SPLIT_EDGE);
            ui.allocate_ui_with_layout(Vec2::new(right_w, crumbs), egui::Layout::top_down(egui::Align::Min), |ui| {
                crumbs_row(ui, p, right, right_id, settings, focused == Side::Right)
            });
        });
    }
    // a click in the pane that is not in front brings its script to the front
    let focus = match focused {
        Side::Left if right.tools.clicked => Some(right.target),
        Side::Right if left.tools.clicked => Some(left.target),
        _ => None,
    };
    let front = match focused {
        Side::Left => &mut *left,
        Side::Right => &mut *right,
    };
    // a split's bar is the window's, drawn under both panes (`bar` is false
    // here), so its asks never come from this column
    let front_id = if focused == Side::Left { left_id } else { right_id };
    let _ = footer_row(ui, p, front, bar, front_id, settings, stats);
    focus
}

/// The width of the split's divider, and the least a pane may be dragged to.
const SPLIT_EDGE: f32 = 8.0;
const MIN_SPLIT: f32 = 160.0;

/// The two panes' widths, left then right, for a `total` width (divider
/// included): `ratio` of the room for the left pane, neither narrower than
/// `MIN_SPLIT` while both fit.
pub(super) fn split_widths(total: f32, ratio: f32) -> (f32, f32) {
    let room = (total - SPLIT_EDGE).max(1.0);
    let min = MIN_SPLIT.min(room / 2.0);
    let left = (room * ratio).clamp(min, room - min);
    (left, room - left)
}

/// The divider between the split's panes: dragged, it gives one pane more of
/// the window and the other less; a double click puts them back at half. The
/// quiet grip says so, as the tree's edge does.
fn split_edge(ui: &mut egui::Ui, p: &Palette, total: f32, height: f32, ratio: &mut f32) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(SPLIT_EDGE, height), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.double_clicked() {
        *ratio = 0.5;
    } else if response.dragged() {
        let room = (total - SPLIT_EDGE).max(1.0);
        let left = split_widths(total, *ratio).0 + response.drag_delta().x;
        *ratio = (left / room).clamp(0.1, 0.9);
    }
    if response.hovered() || response.dragged() {
        ui.painter()
            .vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0, p.violet_light));
    } else {
        let middle = rect.center();
        for step in -1..=1 {
            ui.painter().circle_filled(
                egui::pos2(middle.x, middle.y + step as f32 * 5.0),
                1.5,
                p.muted_dim.gamma_multiply(0.9),
            );
        }
    }
}

/// The rows of a body `room` points tall: the dock's, the source's under the
/// rows above it, and whether the path of the caret has a row of its own.
struct Column {
    source: f32,
    crumbs: bool,
}

/// Those rows, from the room a body has and the footer it already measured:
/// the dock keeps its room -- with "No problems." when everything compiles --
/// unless the window is too short for it, and the source takes the rest (its
/// frame paints 4 points above and below it, hence the 8). The air between the
/// rows comes out of the source's own room, so a body with the air is no
/// taller than one without.
fn column(room: f32, footer: f32, gap: f32) -> Column {
    let height = (room - footer - gap - SOURCE_PAINT).max(SOURCE_MIN);
    // the path of the caret, under the source: a row of its own, taken from
    // the source's room (a window too short keeps its text instead)
    let crumbs = height > CRUMBS_HEIGHT + SOURCE_MIN + gap;
    // a little air above the path of the caret
    let air = if crumbs { AIR } else { 0.0 };
    let height = if crumbs { height - CRUMBS_HEIGHT - gap } else { height };
    Column {
        source: (height - air).max(SOURCE_MIN),
        crumbs,
    }
}

/// The room a footer takes, before it has been measured: the column of a
/// script's own window draws the status bar and guesses its height; the column
/// of a linkset window draws no bar and keeps none. A measured footer of
/// nothing is 0, so 0 cannot mean "not measured yet" here.
fn footer_room(measured: f32, bar: bool) -> f32 {
    if measured > 0.0 {
        measured
    } else if bar {
        155.0
    } else {
        0.0
    }
}

/// The footer under a column -- the reason a script failed and, in a window of
/// its own, the status bar -- and the room it took, read again next frame.
fn footer_row(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    bar: bool,
    id: Id,
    settings: &ScriptSettings,
    stats: Option<super::ScriptStats>,
) -> Option<BarChange> {
    let footer = ui.scope(|ui| footer(ui, p, e, bar, id, settings, stats));
    let height = footer.response.rect.height();
    if (e.tools.footer_height - height).abs() > 0.5 {
        e.tools.footer_height = height;
        ui.ctx().request_repaint();
    }
    footer.inner
}

/// The least height the source keeps: a short window gives the dock's room to
/// the source instead.
const SOURCE_MIN: f32 = 120.0;
/// What the source's frame paints above and below its text: the column
/// keeps it out of the room, and a split's panes keep it in their height
/// so that the rows under them sit where a single pane's do.
const SOURCE_PAINT: f32 = 8.0;

/// What a click on the status bar asks the app for: the two settings a
/// writer flips while working, and the editor's zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarChange {
    /// The editor's font size, in points.
    FontSize(f32),
    WordWrap,
    Minimap,
    /// One window per script, where the preference keeps it (`ScriptSettings::simple`).
    Simple,
}

/// The bookmark F2 (next) or Shift+F2 (previous) goes to from the caret's
/// line, wrapping at either end.
pub(super) fn bookmark_step(lines: &std::collections::BTreeSet<usize>, at: usize, next: bool) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    if next {
        lines.range(at + 1..).next().copied().or_else(|| lines.iter().next().copied())
    } else {
        lines.range(..at).next_back().copied().or_else(|| lines.iter().next_back().copied())
    }
}

/// Puts the cursor on a line of the text on screen (the outline's rows).
pub(super) fn go_to_line(ctx: &egui::Context, e: &mut Editor, id: Id, line: usize, column: usize) {
    let at = editing::line_cursor(text(e), line, column);
    editing::select(ctx, visible_id(id, e.tools.processed), at..at, true);
    e.tools.scroll_to_cursor = true;
}

const DOCK_TEXT: f32 = 11.0;
/// The caret of a bar dropdown: the mark of a word's own size, not a button's
/// chevron, which beside 11 point words read as a control of its own, big and
/// far from what it opens.
const BAR_CARET: f32 = 8.0;
/// Its air from the word it follows.
const BAR_CARET_AIR: f32 = 2.0;
/// The room the caret takes at the right of a bar dropdown's word: the mark,
/// its air and the button's padding on both sides.
const BAR_CARET_ROOM: f32 = BAR_CARET + BAR_CARET_AIR + 6.0;

/// The width of `text` in the dock's text, and its caret's room.
fn word_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = crate::ui::widgets::control_font(ui, DOCK_TEXT);
    ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER).size().x) + BAR_CARET_ROOM
}

/// The keys of one of the editor's own commands, as the menus write them
/// ("Ctrl+S", or "⌘S" on a Mac): the modifiers of the platform, then the key.
/// A control of the status bar: no face and no border, the bar's own text at
/// the size of the rest, its words brighter under the pointer.
fn plain_face(ui: &mut egui::Ui, p: &Palette) {
    let widgets = &mut ui.visuals_mut().widgets;
    for visuals in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        visuals.bg_fill = egui::Color32::TRANSPARENT;
        visuals.weak_bg_fill = egui::Color32::TRANSPARENT;
        visuals.bg_stroke = egui::Stroke::NONE;
    }
    widgets.inactive.fg_stroke.color = p.muted;
    for visuals in [&mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
        visuals.fg_stroke.color = p.ink;
    }
    ui.visuals_mut().override_text_color = Some(p.muted);
    ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
    // a dragged value keeps its own text style (a value is not a label)
    ui.style_mut().drag_value_text_style = egui::TextStyle::Small;
}

/// A dropdown that reads as its word on the surface behind it -- no face, no
/// border, its text brighter under the pointer -- with the theme's own face
/// returned for the menu it opens. `width` is the room its word takes.
fn plain_dropdown(ui: &mut egui::Ui, p: &Palette, id: &str, width: f32, text: &str, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let popup: egui::Style = ui.style().as_ref().clone();
    ui.scope(|ui| {
        plain_face(ui, p);
        ui.spacing_mut().button_padding.x = 3.0;
        // the theme's caret is a button's: on a bar, the mark belongs to the
        // word (its size and its air, `BAR_CARET`)
        ui.spacing_mut().icon_width = BAR_CARET;
        ui.spacing_mut().icon_spacing = BAR_CARET_AIR;
        egui::ComboBox::from_id_salt(id)
            .width(width)
            .selected_text(text.to_owned())
            .popup_style(egui::style::StyleModifier::from(popup))
            .show_ui(ui, add)
            .response
    })
    .inner
}

fn keys_with(key: &str, ctrl: bool, shift: bool, alt: bool) -> String {
    let prefix = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key(String::new()),
        ctrl,
        shift,
        alt,
    }
    .label();
    format!("{prefix}{key}")
}

/// The keys a menu entry writes at its right end, when its command has one
/// the source listens to (`toolbar` below, `editing::keyboard`, the window's
/// own shortcuts).
fn command_keys(command: Command) -> String {
    match command {
        Command::Save => keys_with("S", true, false, false),
        Command::Open => keys_with("O", true, false, false),
        Command::Undo => keys_with("Z", true, false, false),
        Command::Redo => keys_with("Z", true, true, false),
        Command::Cut => keys_with("X", true, false, false),
        Command::Copy => keys_with("C", true, false, false),
        Command::Paste => keys_with("V", true, false, false),
        Command::SelectAll => keys_with("A", true, false, false),
        Command::Find => keys_with("F", true, false, false),
        Command::Goto => keys_with("G", true, false, false),
        Command::Format => keys_with("F", false, true, true),
        Command::LineComment => keys_with("/", true, false, false),
        Command::BlockComment => keys_with("/", true, true, false),
        Command::Indent => keys_with("Tab", false, false, false),
        Command::Outdent => keys_with("Tab", false, true, false),
        _ => String::new(),
    }
}

/// The File, Edit and Help menus of the editor. A window of the linkset's
/// scripts draws them once, above the tree and the tabs, for the script in
/// front. Insert, which inserts a function at the cursor, is one of them: the
/// Legacy Viewer keeps it in the editor's bottom bar, Aurora in the menus.
pub(super) fn menus(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, chosen: &mut Option<Command>) {
    let ctx = ui.ctx().clone();
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(aurora_i18n::tr!("script-menu-file"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Save, "device-floppy-filled", aurora_i18n::tr!("common-save")),
                (Command::Revert, "restore", aurora_i18n::tr!("script-menu-revert")),
                (Command::Reload, "refresh", aurora_i18n::tr!("build-ui-contents-refresh")),
                (Command::Open, "folder-open-filled", aurora_i18n::tr!("script-menu-open")),
                (Command::Export, "download-filled", aurora_i18n::tr!("script-menu-export")),
                (Command::Properties, "info-circle-filled", aurora_i18n::tr!("common-properties")),
                (Command::External, "external-link-filled", aurora_i18n::tr!("script-menu-external")),
                (Command::Preferences, "settings-filled", aurora_i18n::tr!("script-menu-preferences")),
            ] {
                if menu::item_keys_if(ui, p, icon, label, enabled(command, e, &ctx, id), &command_keys(command)) {
                    *chosen = Some(command);
                }
            }
        });
        ui.menu_button(aurora_i18n::tr!("script-menu-edit"), |ui| {
            edit_menu(ui, p, e, id, chosen);
        });
        ui.add_enabled_ui(
            e.loaded && e.can_edit && e.saving.is_none() && !e.tools.processed && !e.colors.active(),
            |ui| {
                let response = ui.menu_button(aurora_i18n::tr!("script-menu-insert"), |ui| {
                    insertion_menu(ui, p, e, id);
                });
                // the demo presses this button and types into its filter
                e.tools.insert_button = Some(response.response.rect);
                e.tools.insertion_open = response.inner.is_some();
            },
        );
        let code = ui.menu_button(aurora_i18n::tr!("script-menu-code"), |ui| {
            code_menu(ui, p, e, id, chosen);
        });
        // the demo clicks it, as it clicks the Insert menu
        e.tools.code_button = Some(code.response.rect);
        ui.menu_button(aurora_i18n::tr!("script-menu-help"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Help, "help-circle-filled", aurora_i18n::tr!("script-menu-help")),
                (Command::Shortcuts, "keyboard-filled", aurora_i18n::tr!("script-help-shortcuts")),
            ] {
                if menu::item_keys_if(ui, p, icon, label, enabled(command, e, &ctx, id), &command_keys(command)) {
                    *chosen = Some(command);
                }
            }
        });
    });
}

/// The Insert menu: a filter and the LSL / Lua functions it leaves, clicking
/// one writes its name at the cursor (LLScriptEdCore's insert list).
fn insertion_menu(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id) {
    super::focus::claim(ui.ctx(), ui.layer_id(), id);
    let field = crate::ui::widgets::search_text_edit(ui, p, &mut e.tools.insert_filter, |edit| {
        edit.id(id.with("insert_filter"))
            .desired_width(260.0)
            .hint_text(aurora_i18n::tr!("script-insert-filter"))
    });
    if !e.tools.insertion_open {
        field.request_focus();
    }
    crate::ui::widgets::divider(ui, p);
    // Only the results scroll, keeping the field in view.
    let mut results = egui::ScrollArea::vertical().id_salt((id, "insert_results")).max_height(220.0);
    if field.changed() {
        results = results.vertical_scroll_offset(0.0);
    }
    let lang = e.lang();
    let mut chosen = None;
    let filter = e.tools.insert_filter.trim().to_lowercase();
    results.show(ui, |ui| {
        let mut found = false;
        for entry in reference::all(lang)
            .iter()
            .filter(|entry| entry.group == "functions" && entry.name.to_lowercase().contains(&filter))
        {
            found = true;
            if ui.selectable_label(false, &entry.name).on_hover_text(&entry.signature).clicked() {
                chosen = Some(entry.name.clone());
                ui.close();
            }
        }
        if !found {
            ui.label(RichText::new(aurora_i18n::tr!("script-insert-empty")).color(p.muted));
        }
    });
    if let Some(name) = chosen {
        let [a, b] = editing::cursor(ui.ctx(), id).sorted_cursors();
        if editing::replace(ui.ctx(), id, &mut e.source, a.index.0..b.index.0, &name) {
            ui.ctx().memory_mut(|m| m.request_focus(id));
            e.tools.scroll_to_cursor = true;
        } else {
            e.status = aurora_i18n::tr!("script-source-too-large").into();
        }
    }
}

/// The toolbar of the editor: Save, Open, the clipboard, Undo / Redo, Find,
/// its preferences, and the state of the script. A window of the linkset's
/// scripts draws it once, under the menus, for the script in front.
/// The width the experience selector takes at the right of the toolbar.
/// The side of a toolbar's buttons, the script's own commands included:
/// Save is the only one wider, its name at the right of its icon.
const TOOLBAR_BUTTON: f32 = 24.0;

const EXPERIENCE_WIDTH: f32 = 180.0;
pub(super) const RUN_WIDTH: f32 = 160.0;

pub(super) fn toolbar(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    save_all: Option<bool>,
) {
    // a draft (the page that asks which language): the toolbar keeps the row,
    // greyed, so that the window's frame does not change when the page closes
    if e.draft.is_some() {
        ui.scope(|ui| {
            ui.disable();
            let mut ignored = Vec::new();
            toolbar_body(ui, p, e, id, settings, prefs, &mut ignored, save_all);
        });
        return;
    }
    toolbar_body(ui, p, e, id, settings, prefs, actions, save_all);
}

fn toolbar_body(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    save_all: Option<bool>,
) {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    let row = ui.horizontal(|ui| {
        // the same height whether the script's own commands fit at the right
        // or not: the source below must not move with them
        ui.set_min_height(26.0);
        // Save sits above the source, with the script's other commands
        // (`run_buttons`), not among the tools here
        let mut buttons = Vec::new();
        if let Some(all) = save_all {
            buttons.push((Command::SaveAll, "files-filled", aurora_i18n::tr!("script-save-all")));
            let _ = all;
        }
        buttons.extend([
            (Command::Export, "download-filled", aurora_i18n::tr!("script-menu-export")),
            (Command::Open, "folder-open-filled", aurora_i18n::tr!("script-menu-open")),
            (Command::External, "external-link-filled", aurora_i18n::tr!("script-menu-external")),
            (Command::Cut, "cut", aurora_i18n::tr!("script-edit-cut")),
            (Command::Copy, "copy-filled", aurora_i18n::tr!("script-edit-copy")),
            (Command::Paste, "clipboard-text-filled", aurora_i18n::tr!("script-edit-paste")),
            (Command::Undo, "arrow-back-up", aurora_i18n::tr!("script-edit-undo")),
            (Command::Redo, "arrow-forward-up", aurora_i18n::tr!("script-edit-redo")),
            (Command::Find, "search-filled", aurora_i18n::tr!("script-search-title")),
            (Command::Preferences, "settings-filled", aurora_i18n::tr!("script-menu-preferences")),
        ]);
        for (command, icon, label) in buttons {
            let enabled = if command == Command::SaveAll {
                save_all.unwrap_or(false)
            } else {
                enabled(command, e, &ctx, id)
            };
            // one size for every command of the window's toolbar, Save's
            // name making it the only wider one
            if icon_button_sized(ui, p, icon, label, enabled, TOOLBAR_BUTTON) {
                chosen = Some(command);
            }
        }
        // the minimap, a toggle at the toolbar's own size: its face lit while
        // the strip is drawn (the documentation pane's toggle stands on the bar,
        // where the minimap's was)
        if icon_toggle_sized(
            ui,
            p,
            "map",
            aurora_i18n::tr!("script-bar-minimap"),
            settings.minimap,
            TOOLBAR_BUTTON,
        ) {
            actions.push(Action::BarChange(BarChange::Minimap));
        }
        // At the right of this row, right to left: the script's own commands
        // (save, reset, running), the experience of the script -- the room of
        // its name alone when it is in none -- and the compile target.
        let experience = (!e.target.object.is_nil()).then_some(EXPERIENCE_WIDTH);
        let right = experience.unwrap_or(0.0) + RUN_WIDTH;
        let room = ui.available_width();
        // a row too short for the name of the experience keeps the script's
        // own commands at its right and gives the experience one button; a
        // row too short even for that keeps the commands alone, never losing
        // Save, Reset and the running state
        let narrow = experience.is_some() && room < right;
        let taken = if narrow { RUN_WIDTH + TOOLBAR_BUTTON + 6.0 } else { right };
        if room >= taken {
            ui.add_space(room - taken);
            ui.allocate_ui_with_layout(Vec2::new(taken, 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                run_buttons(ui, p, e, actions, TOOLBAR_BUTTON);
                // the experience of the script, in the room of its name, or
                // behind one button where that room is missing
                match experience {
                    Some(_) if narrow => experience_overflow(ui, p, e, TOOLBAR_BUTTON),
                    Some(width) => selectors(ui, width, |ui| {
                        let editable = e.can_edit && e.loaded && e.saving.is_none();
                        if e.experience.combo(ui, p, editable)
                            && let Some(request) = e.experience.begin()
                        {
                            actions.push(Action::Experiences { request, target: e.target });
                        }
                    }),
                    None => {}
                }
            });
        } else if room >= RUN_WIDTH {
            ui.add_space(room - RUN_WIDTH);
            ui.allocate_ui_with_layout(Vec2::new(RUN_WIDTH, 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                run_buttons(ui, p, e, actions, TOOLBAR_BUTTON);
            });
        }
    });
    let _ = row;
    if let Some(command) = chosen {
        run(command, e, &ctx, id, settings, prefs, actions);
    }
    if ctx.memory(|m| m.has_focus(visible_id(id, e.tools.processed))) {
        // F2 and Shift+F2 walk the reader's bookmarks; Ctrl+F2 marks the
        // caret's line (or unmarks it)
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::F2)) {
            if !e.bookmarks.remove(&e.tools.position.0) {
                e.bookmarks.insert(e.tools.position.0);
            }
        } else {
            let step = if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F2)) {
                Some(false)
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
                Some(true)
            } else {
                None
            };
            if let Some(next) = step
                && let Some(line) = bookmark_step(&e.bookmarks, e.tools.position.0, next)
            {
                go_to_line(&ctx, e, id, line, 0);
            }
        }
        // Ctrl+Shift+S saves the window's scripts (Save All, as its button)
        if save_all == Some(true) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S)) {
            run(Command::SaveAll, e, &ctx, id, settings, prefs, actions);
        }
        for (key, command) in [
            (egui::Key::S, Command::Save),
            (egui::Key::G, Command::Goto),
            (egui::Key::O, Command::Open),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                run(command, e, &ctx, id, settings, prefs, actions);
            }
        }
        // Shift+Alt+F formats the source, as the Code menu's first command
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT | egui::Modifiers::SHIFT, egui::Key::F)) {
            run(Command::Format, e, &ctx, id, settings, prefs, actions);
        }
    }
}

fn header(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
    own_menus: bool,
) {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    if own_menus {
        menus(ui, p, e, id, &mut chosen);
    }
    if let Some(command) = chosen {
        run(command, e, &ctx, id, settings, prefs, actions);
    }
    toolbar(ui, p, e, id, settings, prefs, actions, None);
}

/// The compile target picker (panel_script_ed.xml `compile_target`, in its
/// order). The Lua targets need a region with SLua, as
/// LLScriptEdCore::postBuild enables them from LuaScriptsEnabled.
/// Which of a script's commands its window offers: Save when the source has
/// something to save, Reset when the script may be reset (`LLLiveLSLEditor::
/// onReset`, indra/newview/llpreviewscript.cpp: neither access to the source
/// nor a running reply is needed), and the running state when it has one to
/// flip -- a window of the inventory has neither Reset nor a running state.
/// A save in flight keeps Reset and the running state on the row, greyed by
/// `run_buttons`, rather than hiding them.
fn run_state(e: &Editor) -> (bool, bool, Option<bool>) {
    let object = !e.target.object.is_nil();
    let reset = object && e.can_reset;
    let running = (object && e.can_edit && e.running.is_some()).then(|| e.running.unwrap_or(false));
    (e.can_save(), reset, running)
}

/// The commands of the script, as icons above the source: save at the right,
/// then reset, then pause or resume, which says what a click does.
pub(super) fn run_buttons(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, actions: &mut Vec<Action>, side: f32) {
    let (save, reset, running) = run_state(e);
    // a save in flight greys the other commands rather than taking them away:
    // the row keeps every button where the reader knows it
    let idle = e.saving.is_none();
    // Save is the one with its name, at the right of its icon, on the face
    // of the two beside it; while a save is in flight, the spinner takes the
    // icon's place, so that the row does not move and the dot keeps saying
    // what the script is
    if e.saving.is_some() {
        crate::ui::widgets::spinner_label_button_sized(
            ui,
            p,
            aurora_i18n::tr!("common-save").as_ref(),
            aurora_i18n::tr!("script-saving").as_ref(),
            side,
        );
    } else if crate::ui::widgets::icon_label_button_sized(
        ui,
        p,
        "device-floppy-filled",
        aurora_i18n::tr!("common-save").as_ref(),
        aurora_i18n::tr!("script-save-tip").as_ref(),
        save,
        side,
    ) {
        actions.extend(e.save());
    }
    if reset && crate::ui::widgets::icon_button_sized(ui, p, "reload", aurora_i18n::tr!("script-reset"), idle, side) {
        actions.push(Action::Reset(e.target));
    }
    if let Some(running) = running {
        // the icon says what pressing it does
        let (icon, label) = if running {
            ("player-pause-filled", aurora_i18n::tr!("script-pause"))
        } else {
            ("player-play-filled", aurora_i18n::tr!("script-resume"))
        };
        if crate::ui::widgets::icon_button_sized(ui, p, icon, label, idle, side) {
            actions.push(Action::Running {
                target: e.target,
                running: !running,
            });
        }
    }
}

/// The experience behind one button, where the row has no room for its name:
/// the same choices, in the menu of a crowded toolbar.
fn experience_overflow(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, side: f32) {
    let editable = e.can_edit && e.loaded && e.saving.is_none();
    let tip = aurora_i18n::tr!("script-experience");
    let button = ui
        .add_enabled_ui(editable, |ui| {
            crate::ui::widgets::icon_button_sized_response(ui, p, "dots", tip, true, side)
        })
        .inner;
    // the choices as they last came, so that the menu may change the choice
    let choices: Vec<(uuid::Uuid, String, bool)> = e
        .experience
        .choices()
        .iter()
        .map(|choice| (choice.id, choice.name.clone(), choice.selectable))
        .collect();
    menu::dropdown(&button, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        let none = e.experience.selected == Some(uuid::Uuid::nil());
        if menu::check(
            ui,
            p,
            "circle-x",
            aurora_i18n::tr!("script-experience-none").as_ref(),
            none,
            editable,
        ) {
            e.experience.selected = Some(uuid::Uuid::nil());
        }
        for (id, name, selectable) in &choices {
            let chosen = e.experience.selected == Some(*id);
            if menu::check(ui, p, "point", name.as_str(), chosen, editable && *selectable) {
                e.experience.selected = Some(*id);
            }
        }
    });
}

/// The room a selector takes in the toolbar, and the height of the buttons
/// beside it: a combo box follows `interact_size.y`, which the theme sets for
/// a text field.
fn selectors(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, TOOLBAR_BUTTON),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().interact_size.y = TOOLBAR_BUTTON;
            // the theme pads a button by more than the row leaves: 15 points of
            // text and 2 x 4 make exactly the height of the buttons beside it
            ui.spacing_mut().button_padding.y = 4.0;
            // a combo box draws the theme's border around its face, which makes
            // it a point taller than the flat buttons beside it: the selectors
            // keep their face alone, as those buttons do
            let visuals = &mut ui.visuals_mut().widgets;
            visuals.inactive.bg_stroke = egui::Stroke::NONE;
            visuals.hovered.bg_stroke = egui::Stroke::NONE;
            visuals.active.bg_stroke = egui::Stroke::NONE;
            visuals.open.bg_stroke = egui::Stroke::NONE;
            add(ui);
        },
    );
}

/// The engines a script can compile for, in the order the menu lists them.
const ENGINES: [aurora_net::scripts::CompileTarget; 4] = [
    aurora_net::scripts::CompileTarget::Mono,
    aurora_net::scripts::CompileTarget::Lsl2,
    aurora_net::scripts::CompileTarget::LslLuau,
    aurora_net::scripts::CompileTarget::Luau,
];

/// The engine a script compiles for, as the bar names it.
fn engine_label(target: aurora_net::scripts::CompileTarget) -> String {
    use aurora_net::scripts::CompileTarget;
    match target {
        CompileTarget::Mono => aurora_i18n::tr!("script-target-mono"),
        CompileTarget::Lsl2 => aurora_i18n::tr!("script-target-lsl2"),
        CompileTarget::LslLuau => aurora_i18n::tr!("script-target-lsl-luau"),
        CompileTarget::Luau => aurora_i18n::tr!("script-target-luau"),
    }
    .to_string()
}

fn footer(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    bar: bool,
    id: Id,
    settings: &ScriptSettings,
    stats: Option<super::ScriptStats>,
) -> Option<BarChange> {
    // What a script says of itself (saving, preprocessing, what it just did or
    // why it could not) is a line of the window's console, not a word here:
    // the bar keeps only what is true of the script in front all the time.
    if bar { status_bar(ui, p, e, id, settings, stats) } else { None }
}

/// The side of the bar's toggles, smaller than a command's.
const BAR_BUTTON: f32 = 18.0;

/// The height of `status_bar`: its row and the room its frame keeps around it.
pub(super) const STATUS_BAR: f32 = 30.0;
/// The rooms the status bar's right end keeps for what changes there: the
/// source's weight, the zoom and the running state. The engine takes the
/// widest of its names, so choosing another target moves nothing.
const BAR_SIZE_SLOT: f32 = 100.0;
/// The number the zoom is dragged on, between its two steps.
const BAR_DRAG_SLOT: f32 = 44.0;
/// The zoom's unit: the step down, the number, the step up, and no air after
/// the last of them.
const BAR_ZOOM_SLOT: f32 = BAR_BUTTON * 2.0 + BAR_DRAG_SLOT + 4.0;
/// The minimap and word wrap toggles at the right end, as one unit.
const BAR_TOGGLES_SLOT: f32 = BAR_BUTTON * 2.0 + 2.0;
const BAR_STATE_SLOT: f32 = 74.0;

/// The bar under the source: the caret's place at the left, the script's
/// weight at the right end. A window of a linkset draws it across the whole
/// window, under its tree too, so that it reads as the window's own footer;
/// the commands above the source (save, reset, running, the compile target)
/// are the icons beside its tabs.
/// A hairline between two cells of the status bar, the same everywhere.
fn bar_separator(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(7.0, 14.0), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        egui::Rangef::new(rect.center().y - 6.0, rect.center().y + 6.0),
        egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.45)),
    );
}

pub(super) fn status_bar(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    stats: Option<super::ScriptStats>,
) -> Option<BarChange> {
    // a draft has no caret, size or state to name: the bar keeps its room,
    // empty, so that the frame does not shrink under the page
    if e.draft.is_some() {
        egui::Frame::new()
            .fill(p.bar)
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.allocate_space(Vec2::new(ui.available_width(), (STATUS_BAR - 4.0).max(20.0)));
            });
        return None;
    }
    let mut change = None;
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            // the row fills the bar between its two points of margin, so that
            // its ink sits on the bar's middle, not a point or two under it
            // (the theme's 26 for a text field would lift the engine's
            // dropdown over it and grow the window a point per frame)
            ui.spacing_mut().interact_size.y = 20.0;
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), (STATUS_BAR - 4.0).max(20.0)),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    // the layout of the windows, where the two toggles stood: the
                    // shared window, or one per script
                    if icon_toggle_sized(
                        ui,
                        p,
                        "app-window-filled",
                        aurora_i18n::tr!("script-bar-one-window"),
                        settings.simple,
                        BAR_BUTTON,
                    ) {
                        change = Some(BarChange::Simple);
                    }
                    bar_separator(ui, p);
                    let position = aurora_i18n::tr!(
                        "script-position",
                        line = (e.tools.position.0 + e.first_line()) as i64,
                        column = (e.tools.position.1 + e.first_line()) as i64
                    );
                    let spaces = aurora_i18n::tr!("script-spaces", count = settings.tab_size.max(1) as i64);
                    let size = aurora_i18n::tr!(
                        "script-source-size",
                        used = size_label(e.source.len()),
                        limit = size_label(aurora_net::scripts::MAX_SOURCE)
                    );
                    // one size and one colour for everything the bar says
                    let words = |text: String| RichText::new(text).small().color(p.muted);
                    // what the object's scripts take, as the region reports it
                    // through the bridge (`OBJECT_SCRIPT_MEMORY`): the memory
                    // they are allowed, and how many of them run. It stands
                    // with the readout, where its coming and going moves no
                    // data on the right.
                    let stats_said = stats.filter(|stats| stats.total > 0).map(|stats| {
                        let memory = size_label(stats.memory as usize);
                        let memory = aurora_i18n::tr!("script-bar-scripts", memory = memory).to_string();
                        let running = (stats.running != stats.total).then(|| {
                            aurora_i18n::tr!(
                                "script-bar-scripts-running",
                                running = stats.running as i64,
                                total = stats.total as i64
                            )
                            .to_string()
                        });
                        (memory, running)
                    });
                    // the data at the left, a hairline between each pair
                    ui.add(egui::Label::new(words(position.to_string())).truncate());
                    bar_separator(ui, p);
                    ui.add(egui::Label::new(words(spaces.to_string())).truncate());
                    // what a copy would take, when something is selected
                    let range = editing::cursor(ui.ctx(), id).as_sorted_char_range();
                    let selected = range.end.0.saturating_sub(range.start.0);
                    if selected > 0 {
                        bar_separator(ui, p);
                        let said = aurora_i18n::tr!("script-selected-chars", count = selected as i64);
                        ui.add(egui::Label::new(words(said.to_string())).truncate());
                    }
                    // when it was last saved here, or that it changed since
                    let saved = if e.dirty() {
                        Some(aurora_i18n::tr!("script-unsaved").to_string())
                    } else {
                        e.saved_at
                            .map(|at| aurora_i18n::tr!("script-saved-at", time = crate::ui::chat::hhmm(at)).to_string())
                    };
                    if let Some(saved) = saved {
                        bar_separator(ui, p);
                        ui.add(egui::Label::new(words(saved)).truncate());
                    }
                    if let Some((memory, running)) = stats_said {
                        bar_separator(ui, p);
                        ui.add(egui::Label::new(words(memory)).truncate())
                            .on_hover_text(aurora_i18n::tr!("script-bar-scripts-tip"));
                        if let Some(running) = running {
                            bar_separator(ui, p);
                            ui.add(egui::Label::new(words(running)).truncate())
                                .on_hover_text(aurora_i18n::tr!("script-bar-scripts-tip"));
                        }
                    }
                    // at the right end, from the right: the script's weight,
                    // the zoom with its two buttons, the running state and the
                    // engine; a hairline between each pair
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let row = (STATUS_BAR - 4.0).max(20.0);
                        // the two settings a writer flips, at the right end: a
                        // unit of their own, so the words beside them never
                        // move them
                        ui.allocate_ui_with_layout(
                            Vec2::new(BAR_TOGGLES_SLOT, row),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if icon_toggle_sized(
                                    ui,
                                    p,
                                    "arrow-autofit-width-filled",
                                    aurora_i18n::tr!("script-bar-wrap"),
                                    settings.word_wrap,
                                    BAR_BUTTON,
                                ) {
                                    change = Some(BarChange::WordWrap);
                                }
                            },
                        );
                        bar_separator(ui, p);
                        // the weight of the source, in a room of its own
                        ui.allocate_ui_with_layout(
                            Vec2::new(BAR_SIZE_SLOT, row),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add(egui::Label::new(RichText::new(size).small().color(p.muted)).truncate());
                            },
                        );
                        bar_separator(ui, p);
                        // the zoom, a number to drag on the bar's own face, with
                        // a step down and a step up beside it
                        let mut zoom = settings.font_size;
                        let (minus, drag, plus) = ui
                            .allocate_ui_with_layout(
                                Vec2::new(BAR_ZOOM_SLOT, row),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    let minus = crate::ui::widgets::icon_button_sized(
                                        ui,
                                        p,
                                        "minus",
                                        aurora_i18n::tr!("script-bar-zoom-out"),
                                        true,
                                        BAR_BUTTON,
                                    );
                                    let drag = ui
                                        .scope(|ui| {
                                            plain_face(ui, p);
                                            // the unit says what the number is, on the bar
                                            let unit = format!(" {}", aurora_i18n::tr!("script-bar-points"));
                                            ui.add_sized(
                                                Vec2::new(BAR_DRAG_SLOT, row),
                                                egui::DragValue::new(&mut zoom)
                                                    .range(8.0..=32.0)
                                                    .speed(0.2)
                                                    .max_decimals(0)
                                                    .suffix(unit),
                                            )
                                        })
                                        .inner;
                                    let plus = crate::ui::widgets::icon_button_sized(
                                        ui,
                                        p,
                                        "plus",
                                        aurora_i18n::tr!("script-bar-zoom-in"),
                                        true,
                                        BAR_BUTTON,
                                    );
                                    (minus, drag, plus)
                                },
                            )
                            .inner;
                        if minus {
                            zoom = (zoom - 1.0).max(8.0);
                        }
                        if plus {
                            zoom = (zoom + 1.0).min(32.0);
                        }
                        if drag.changed() || minus || plus {
                            change = Some(BarChange::FontSize(zoom));
                        }
                        drag.on_hover_text(aurora_i18n::tr!("script-bar-zoom"));
                        bar_separator(ui, p);
                        // whether it runs, before the engine from the right, so
                        // that it reads after it and after its caret; its room
                        // stays whether the state is known or not
                        let state = match e.running {
                            Some(true) => Some(aurora_i18n::tr!("script-state-running")),
                            Some(false) => Some(aurora_i18n::tr!("script-state-stopped")),
                            None => None,
                        };
                        // the light of that state, beside its word: red for a
                        // failed compile, green while it runs, amber stopped
                        let light = if e.compile_failed {
                            Some(p.danger)
                        } else {
                            match e.running {
                                Some(true) => Some(p.success),
                                Some(false) => Some(p.amber),
                                None => None,
                            }
                        };
                        ui.allocate_ui_with_layout(
                            Vec2::new(BAR_STATE_SLOT, row),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if let Some(state) = state {
                                    ui.add(egui::Label::new(words(state.to_string())).truncate());
                                }
                                if let Some(light) = light {
                                    crate::ui::widgets::status_dot(ui, light);
                                }
                            },
                        );
                        bar_separator(ui, p);
                        // the engine is chosen here, where it is named: a dropdown
                        // that reads as its word on the bar, its caret on the name.
                        // Its room is the widest name, so choosing another engine
                        // moves nothing.
                        let said = engine_label(e.compile);
                        let width = ENGINES
                            .iter()
                            .map(|engine| {
                                let label = engine_label(*engine);
                                word_width(ui, &label)
                            })
                            .fold(0.0, f32::max);
                        let editable = e.can_edit && e.loaded && e.saving.is_none() && (e.target.object.is_nil() || e.running.is_some());
                        ui.add_enabled_ui(editable, |ui| {
                            plain_dropdown(ui, p, "bar_runtime", width, &said, |ui| {
                                for engine in ENGINES {
                                    let lua = matches!(
                                        engine,
                                        aurora_net::scripts::CompileTarget::LslLuau | aurora_net::scripts::CompileTarget::Luau
                                    );
                                    ui.add_enabled_ui(!lua || e.lua_scripts, |ui| {
                                        ui.selectable_value(&mut e.compile, engine, engine_label(engine));
                                    });
                                }
                            });
                        })
                        .response
                        .on_hover_text(aurora_i18n::tr!("script-target-hint"));
                    });
                },
            );
        });
    change
}

/// The width of the overview strip at the right of the source.
const OVERVIEW_WIDTH: f32 = 9.0;

/// The row the sticky state/event line takes above the source.
const STICKY_HEIGHT: f32 = 18.0;

/// The room an icon takes in a chip of the path, before its words.
const ICON_CRUMB: f32 = 14.0;

/// The row the path of the caret takes under the source.
const CRUMBS_HEIGHT: f32 = 22.0;

/// The text of the bar: the steps of the path, the view switch and the
/// signature, all at one size, smaller than the source's.
const CRUMB_TEXT: f32 = 9.0;

/// The chevron between two steps of the path, and the air around it.
const CRUMB_SEP: f32 = 10.0;

/// The room of the chip that stands for the steps a narrow bar cannot show.
const CRUMB_FOLD: f32 = 22.0;

/// Under this width, a pane's bar gives the source / processed switch icons
/// instead of its two words.
const COMPACT_BAR: f32 = 380.0;

/// The air the rows around the source keep: above its path, and between that
/// path and the dock under it. Taken from the source's own room, so a window
/// with the air is no taller than one without.
const AIR: f32 = 4.0;

/// The overview strip at the right of the source: the whole script in the
/// height of the window, with the part on screen (a thumb), the lines whose
/// text differs from the saved script (amber) and the diagnostics (the danger
/// colour). A click puts the caret on the line there, as an IDE's overview
/// ruler does. Painted over the scroll area's clip, at screen coordinates: it
/// stays put while the text scrolls.
fn overview(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    strip: egui::Rect,
    viewport: egui::Rect,
    rows: &[egui::epaint::text::PlacedRow],
    problems: &[Problem],
    change_bars: bool,
) {
    let text = text(e);
    let content = output_height(rows, text);
    let at = |y: f32| strip.top() + (y / content).clamp(0.0, 1.0) * strip.height();
    let painter = ui.painter().with_clip_rect(strip);
    // the part of the script the window shows, when it does not show it all
    if viewport.height() < content {
        let top = at(viewport.min.y.max(0.0));
        let bottom = at(viewport.max.y.min(content));
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(strip.left(), top), egui::pos2(strip.right(), bottom.max(top + 2.0))),
            ui.visuals().window_corner_radius,
            p.raised.gamma_multiply(0.35),
        );
    }
    // the changed lines and the diagnostics, as thin marks at the right edge
    let changed = if change_bars && e.loaded && !e.tools.processed {
        changed_lines(text, &e.baseline)
    } else {
        None
    };
    let mut line = e.first_line();
    let mut line_start = true;
    for row in rows {
        if line_start {
            let index = line.saturating_sub(e.first_line());
            let y = at(row.pos.y);
            let tick = egui::Rect::from_min_max(egui::pos2(strip.right() - 4.0, y), egui::pos2(strip.right() - 1.0, y + 2.0));
            if let Some((first, last)) = changed
                && (first..=last).contains(&index)
            {
                painter.rect_filled(tick, 0.0, p.amber);
            }
            if problems.iter().any(|problem| problem.line == line && problem.source == e.name) {
                painter.rect_filled(tick.expand2(egui::Vec2::new(1.0, 0.0)), 0.0, p.danger);
            }
            if e.bookmarks.contains(&index) {
                painter.rect_filled(tick.expand2(egui::Vec2::new(1.0, 0.0)), 0.0, p.indigo_light);
            }
        }
        if row.ends_with_newline {
            line += 1;
        }
        line_start = row.ends_with_newline;
    }
    let response = ui.interact(strip, egui::Id::unique((id, "overview")), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let y = ((pos.y - strip.top()) / strip.height()).clamp(0.0, 1.0) * content;
        let mut target = None;
        let mut line = e.first_line();
        for row in rows {
            if (row.pos.y..row.pos.y + row.height()).contains(&y) {
                target = Some(line.saturating_sub(e.first_line()));
                break;
            }
            if row.ends_with_newline {
                line += 1;
            }
        }
        let target = target.unwrap_or_else(|| text.lines().count().saturating_sub(1));
        let at = editing::line_cursor(text, target, 0);
        editing::select(ui.ctx(), id, at..at, true);
        e.tools.scroll_to_cursor = true;
    }
    e.tools.overview = Some(strip);
}

/// The minimap: the script in a column at the right of the source, a bar per
/// line from its indentation to its last character, in the colour of what the
/// line starts with (Aurora's own; the thin overview strip of [`overview`]
/// stands here when the setting is off). The part of the script on screen is
/// framed, the changed lines, the diagnostics and the bookmarks hold the right
/// edge, a click puts the caret on the line there and a drag moves the view
/// alone.
#[allow(clippy::too_many_arguments)]
fn minimap(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, settings: &ScriptSettings, strip: egui::Rect, problems: &[Problem]) {
    let tab = usize::from(settings.tab_size.max(1));
    // the map is made again only when the script, its tabs or the setting that
    // colours it change
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        e.text_key(Some(ui.input(|i| i.time))).hash(&mut hasher);
        tab.hash(&mut hasher);
        hasher.finish()
    };
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    if e.tools.minimap.as_ref().is_none_or(|(known, _)| *known != hash) {
        e.tools.minimap = Some((hash, super::minimap::map(text, e.lang(), tab)));
    }
    let Some((_, map)) = &e.tools.minimap else {
        return;
    };
    let total = map.total.max(1);
    // a line is a few points tall: a short script does not stretch to the
    // window, and a long one keeps a point per line
    let step = (strip.height() / total as f32).clamp(1.0, 3.0);
    // the marks keep the right edge, the bars the rest
    let band = 4.0;
    let bars = egui::Rect::from_min_max(strip.min, egui::pos2(strip.right() - band, strip.bottom()));
    let scale = bars.width() / super::minimap::COLUMNS;
    let opacity = f32::from(settings.minimap_opacity.clamp(10, 100)) / 100.0;
    let painter = ui.painter().with_clip_rect(strip);
    // the part of the script on screen, behind the bars
    let first = e.tools.first_visible.min(total - 1);
    let last = e.tools.last_visible.max(first).min(total - 1);
    if last.saturating_sub(first) + 1 < total {
        let top = strip.top() + first as f32 * step;
        let bottom = strip.top() + (last + 1) as f32 * step;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(strip.left(), top), egui::pos2(strip.right(), bottom.max(top + 2.0))),
            ui.visuals().window_corner_radius,
            p.raised.gamma_multiply(0.35 * opacity),
        );
    }
    let changed = if settings.change_bars && e.loaded && !e.tools.processed {
        changed_lines(text, &e.baseline)
    } else {
        None
    };
    for line in &map.lines {
        let y = strip.top() + line.number as f32 * step;
        // the lines the strip cannot show are not drawn: a long script's map
        // would paint a mark for every line of it, tens of thousands clipped
        if y > strip.bottom() {
            break;
        }
        if let Some((from, to)) = line.bar {
            let x0 = bars.left() + from * scale;
            let x1 = (bars.left() + to * scale).min(bars.right()).max(x0 + 1.0);
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, y), egui::pos2(x1, y + step)),
                0.0,
                settings.color(line.role, p).gamma_multiply(0.92 * opacity),
            );
        }
        let tick = egui::Rect::from_min_max(egui::pos2(bars.right() + 1.0, y), egui::pos2(strip.right(), y + step.max(2.0)));
        if let Some((first, last)) = changed
            && (first..=last).contains(&line.number)
        {
            painter.rect_filled(tick, 0.0, p.amber);
        }
        if problems
            .iter()
            .any(|problem| problem.line == line.number && problem.source == e.name)
        {
            painter.rect_filled(tick, 0.0, p.danger);
        }
        if e.bookmarks.contains(&line.number) {
            painter.rect_filled(tick, 0.0, p.indigo_light);
        }
    }
    let response = ui.interact(strip, egui::Id::unique((id, "minimap")), egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    e.tools.overview = Some(strip);
    // a click or a drag through the map takes the caret with it, and the source
    // scrolls to it at its next show
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let line = (((pos.y - strip.top()) / step).max(0.0) as usize).min(total - 1);
        let at = editing::line_cursor(text, line, 0);
        editing::select(ui.ctx(), id, at..at, true);
        e.tools.scroll_to_cursor = true;
    }
}

/// The height the overview strip maps: the text's rows, or the text's lines
/// when the galley has not laid them out yet.
fn output_height(rows: &[egui::epaint::text::PlacedRow], text: &str) -> f32 {
    rows.last()
        .map(|row| row.pos.y + row.height())
        .unwrap_or_else(|| text.lines().count().max(1) as f32)
        .max(1.0)
}

/// F8 (`Some(true)`) or Shift+F8 (`Some(false)`), for the diagnostics in walk:
/// the keys belong to the script window while its source has the keyboard.
pub(super) fn problem_key(ctx: &egui::Context) -> Option<bool> {
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::SHIFT, egui::Key::F8) {
            Some(false)
        } else if i.consume_key(egui::Modifiers::NONE, egui::Key::F8) {
            Some(true)
        } else {
            None
        }
    })
}

/// The row F8 or Shift+F8 goes to, from the caret: the first diagnostic of the
/// dock's order after it (or the last one when it is past them), the last one
/// before it (or the first one when it is before them). `rows` holds each
/// row's script and line, `at` the caret's.
pub(super) fn problem_row(rows: &[(usize, usize)], at: (usize, usize), next: bool) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    if next {
        rows.iter().position(|row| *row > at).or(Some(0))
    } else {
        rows.iter().rposition(|row| *row < at).or(Some(rows.len() - 1))
    }
}

/// Puts the cursor on a diagnostic's line and column, in the processed source
/// when the diagnostics shown are its own (LLScriptEdCore::onErrorList).
pub(super) fn go_to_error(ctx: &egui::Context, e: &mut Editor, id: Id, line: usize, column: usize) {
    if line == 0 {
        return;
    }
    e.tools.processed = e.tools.diagnostics_processed && !e.tools.processed_source.is_empty();
    // LLScriptEdCore::onErrorList: a Lua error's line counts from 1
    let at = editing::line_cursor(text(e), line.saturating_sub(e.first_line()), column);
    editing::select(ctx, visible_id(id, e.tools.processed), at..at, true);
    e.tools.scroll_to_cursor = true;
}

/// A byte count for the status line: "512 B", "1.8 KB", "1 MB".
pub(super) fn size_label(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.0} MB", bytes as f32 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f32 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// The columns of indentation of a line: a tab reaches the next stop.
fn indent_columns(line: &str, tab_size: usize) -> usize {
    let tab_size = tab_size.max(1);
    let mut columns = 0;
    for ch in line.chars() {
        match ch {
            ' ' => columns += 1,
            '\t' => columns += tab_size - columns % tab_size,
            _ => break,
        }
    }
    columns
}

/// The lines of `text` that differ from `baseline`, as an inclusive range:
/// the region the two differ in, without marking the lines they share around
/// it (a line inserted in the middle marks that region, not the rest).
fn changed_lines(text: &str, baseline: &str) -> Option<(usize, usize)> {
    if text == baseline {
        return None;
    }
    let edited: Vec<&str> = text.lines().collect();
    let saved: Vec<&str> = baseline.lines().collect();
    let mut first = 0;
    while first < edited.len() && first < saved.len() && edited[first] == saved[first] {
        first += 1;
    }
    let (mut last_edited, mut last_saved) = (edited.len(), saved.len());
    while last_edited > first && last_saved > first && edited[last_edited - 1] == saved[last_saved - 1] {
        last_edited -= 1;
        last_saved -= 1;
    }
    let last = if last_edited > first { last_edited - 1 } else { first }.min(edited.len().saturating_sub(1));
    Some((first, last))
}

fn edit_menu(ui: &mut egui::Ui, p: &Palette, e: &Editor, id: Id, chosen: &mut Option<Command>) {
    super::focus::claim(ui.ctx(), ui.layer_id(), id);
    for (command, icon, label) in [
        (Command::Undo, "arrow-back-up", aurora_i18n::tr!("script-edit-undo")),
        (Command::Redo, "arrow-forward-up", aurora_i18n::tr!("script-edit-redo")),
        (Command::Cut, "cut", aurora_i18n::tr!("script-edit-cut")),
        (Command::Copy, "copy-filled", aurora_i18n::tr!("script-edit-copy")),
        (Command::Paste, "clipboard-text-filled", aurora_i18n::tr!("script-edit-paste")),
        (Command::SelectAll, "select-all", aurora_i18n::tr!("script-edit-select-all")),
        (Command::Deselect, "deselect", aurora_i18n::tr!("script-edit-deselect")),
        (Command::Find, "search-filled", aurora_i18n::tr!("script-search-title")),
        (Command::Goto, "arrow-bar-to-down", aurora_i18n::tr!("script-goto")),
    ] {
        if menu::item_keys_if(ui, p, icon, label, enabled(command, e, ui.ctx(), id), &command_keys(command)) {
            *chosen = Some(command);
        }
    }
}

/// The Code menu: what rewrites the source rather than the caret's selection --
/// its canonical form, its comments, its indentation, its folds. The commands
/// keep the keys they had in Edit (`editing::selection_command` reads them
/// where they are pressed), a menu giving them a home of their own.
fn code_menu(ui: &mut egui::Ui, p: &Palette, e: &Editor, id: Id, chosen: &mut Option<Command>) {
    super::focus::claim(ui.ctx(), ui.layer_id(), id);
    for (command, icon, label) in [
        (Command::Format, "sparkles-filled", aurora_i18n::tr!("script-format")),
        (Command::LineComment, "slashes", aurora_i18n::tr!("script-edit-line-comment")),
        (
            Command::BlockComment,
            "code-asterisk",
            aurora_i18n::tr!("script-edit-block-comment"),
        ),
        (Command::Indent, "indent-increase", aurora_i18n::tr!("script-edit-indent")),
        (Command::Outdent, "indent-decrease", aurora_i18n::tr!("script-edit-outdent")),
        (Command::FoldAll, "caret-right-filled", aurora_i18n::tr!("script-fold-all")),
        (Command::ExpandAll, "caret-down-filled", aurora_i18n::tr!("script-expand-all")),
    ] {
        if menu::item_keys_if(ui, p, icon, label, enabled(command, e, ui.ctx(), id), &command_keys(command)) {
            *chosen = Some(command);
        }
    }
}

/// The rows of `galley` its `clip` shows: the rows' places are monotonic, so
/// a galley of seventy thousand rows is not walked from its first row on every
/// frame to find them.
fn visible_rows(galley: &egui::Galley, at: egui::Pos2, clip: egui::Rect) -> std::ops::Range<usize> {
    let rows = &galley.rows;
    let first = rows.partition_point(|row| at.y + row.pos.y + row.height() <= clip.top());
    let end = rows.partition_point(|row| at.y + row.pos.y < clip.bottom());
    first..end.max(first)
}

/// The line of the text `row` of `galley` begins on: the line its first
/// character is on, read once for a place, never once per row of the script.
fn line_of_row(galley: &egui::Galley, text: &str, row: usize) -> usize {
    let Some(row) = galley.rows.get(row) else { return 0 };
    let at = egui::vec2(1.0, row.pos.y + row.height() / 2.0);
    let index = galley.cursor_from_pos(at).index.0;
    let byte = text.char_indices().nth(index).map_or(text.len(), |(at, _)| at);
    text[..byte].bytes().filter(|byte| *byte == b'\n').count()
}

fn source(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    height: f32,
    search: &Search,
    problems: &[Problem],
) {
    if std::mem::take(&mut e.tools.reset_history) {
        egui::text_edit::TextEditState::default().store(ui.ctx(), id);
        egui::text_edit::TextEditState::default().store(ui.ctx(), visible_id(id, true));
        e.tools.can_undo = false;
        e.tools.can_redo = false;
        e.tools.history_cursor = None;
    }
    let base_id = id;
    let id = visible_id(id, e.tools.processed);
    // A new floater first lays out an invisible, disabled sizing pass.
    // Keep the request until the source can actually take the keyboard.
    if e.loaded && !ui.is_sizing_pass() && ui.is_enabled() && std::mem::take(&mut e.tools.focus_start) {
        editing::select(ui.ctx(), id, 0..0, true);
        e.tools.scroll_to_cursor = true;
    }
    let lang = e.lang();
    // field by field: the folds take a mutable borrow of themselves
    e.folding
        .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source }, lang);
    e.folding.reveal(editing::cursor(ui.ctx(), id).primary);
    // The sticky line above the text: the state and the event or function the
    // top of the source is in, from the outline (an IDE's sticky scroll). It
    // takes a row of its own, so the text starts under it; the line is the
    // last frame's, one frame behind the scroll.
    // (a window too short for its row keeps its whole source instead)
    let sticky = if settings.sticky_line && e.loaded && height > STICKY_HEIGHT + SOURCE_MIN {
        let first = e.tools.first_visible;
        let items = &e.analysis(Some(ui.input(|i| i.time))).outline;
        super::outline::enclosing(items, first).map(|(state, inner)| match state {
            Some(state) => format!("{} › {}", state.name, inner.name),
            None => inner.name.clone(),
        })
    } else {
        None
    };
    let mut drop = 0.0;
    if let Some(label) = &sticky {
        // on the editor's own background, as the source below it
        let band = egui::Frame::new()
            .fill(settings.color(ColorRole::Background, p))
            .inner_margin(egui::Margin {
                left: 6,
                right: 4,
                top: 2,
                bottom: 0,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(label).size(11.0).color(p.muted)).truncate());
            });
        // its row and the spacing under it come out of the source's room
        drop = band.response.rect.height() + ui.spacing().item_spacing.y;
    }
    let height = (height - drop).max(60.0);
    egui::Frame::new()
        .fill(settings.color(ColorRole::Background, p))
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.scope(|ui| {
                // the pane's own edge: in a split, a column's clip is the
                // window's, and the rail would land on the right pane's
                let pane_right = ui.max_rect().right();
                ui.visuals_mut().selection.bg_fill = settings.color(ColorRole::Selection, p);
                ui.visuals_mut().text_cursor.stroke.color = settings.color(ColorRole::Cursor, p);
                let font = settings.font_id(ui.ctx());
                let query = if search.owns(base_id) {
                    search.query.clone()
                } else {
                    String::new()
                };
                let insensitive = search.insensitive;
                let whole_words = search.whole_words;
                let color_editable = e.loaded && e.can_edit && e.saving.is_none() && e.loading.is_none() && !e.tools.processed;
                let editable = color_editable && !e.colors.active();
                // the completion list's keys, taken before the text sees them,
                // then a snippet's, which only walk while it is being filled
                let nav = if editable && settings.completion && !ui.is_sizing_pass() {
                    complete::keys(ui, id)
                } else {
                    None
                };
                let step = if editable && nav.is_none() && e.tools.snippet.is_some() && !ui.is_sizing_pass() {
                    complete::snippet_keys(ui)
                } else {
                    None
                };
                let mut scroll = egui::ScrollArea::new([!settings.word_wrap, true]);
                if settings.word_wrap {
                    scroll = scroll.horizontal_scroll_offset(0.0);
                }
                scroll
                    .id_salt((id, "source"))
                    .auto_shrink([false, false])
                    .max_height(height)
                    .show_viewport(ui, |ui, viewport| {
                        ui.horizontal_top(|ui| {
                            let (gutter, _) = ui.allocate_exact_size(Vec2::new(56.0, 1.0), egui::Sense::hover());
                            let interactive = e.loaded && (e.can_read || !e.source.is_empty());
                            let lang = e.lang();
                            // the parameter names of the calls in the text on
                            // screen: their room goes into the job below, their
                            // pills are painted over it
                            if settings.inlays && !e.tools.processed && e.loaded {
                                update_inlays(e, ui, &font, lang);
                            } else {
                                e.tools.inlays = None;
                            }
                            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                                let mut job =
                                    e.highlighter
                                        .custom(text.as_str(), p, font.clone(), settings, &query, insensitive, whole_words, lang);
                                // panel_script_ed.xml enables wrapping; only the visual rows change.
                                job.wrap.max_width = if settings.word_wrap { wrap } else { f32::INFINITY };
                                e.folding.sync(text.as_str(), lang);
                                e.folding.layout(&mut job);
                                // after the folds, which split the sections their own way
                                if let Some((_, inlays)) = &e.tools.inlays {
                                    super::inlays::space(&mut job, inlays);
                                }
                                ui.fonts_mut(|f| f.layout_job(job))
                            };
                            let mut show = |ui: &mut egui::Ui, buffer: &mut dyn egui::TextBuffer| {
                                // a slot under the text for the caret's row and
                                // the indent guides
                                let beneath = ui.painter().add(egui::Shape::Noop);
                                // the rail (the minimap, or the thin overview
                                // strip) and the scroll bar keep their room at
                                // the right of the text
                                let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
                                let rail = if settings.minimap {
                                    f32::from(settings.minimap_width.clamp(40, 100))
                                } else {
                                    OVERVIEW_WIDTH
                                };
                                let width = (ui.available_width() - rail - bar).max(0.0);
                                let output = egui::TextEdit::multiline(buffer)
                                    .id(id)
                                    .code_editor()
                                    .layouter(&mut layouter)
                                    .desired_width(width)
                                    .desired_rows(1)
                                    .min_size(Vec2::new(0.0, viewport.height()))
                                    .frame(egui::Frame::NONE)
                                    .interactive(interactive)
                                    .show(ui);
                                (output, beneath)
                            };
                            let (mut output, beneath) = if editable {
                                indent::edit(ui, id, &mut e.source, lang, usize::from(settings.tab_size.max(1)), show)
                            } else {
                                // field by field: the layouter beside it takes
                                // a mutable borrow of the highlighter
                                let mut text = if e.tools.processed {
                                    e.tools.processed_source.as_str()
                                } else {
                                    e.source.as_str()
                                };
                                show(ui, &mut text)
                            };
                            // the parameter names, painted into the room
                            // their hint takes: over the text, the gap after
                            // each one keeping its argument's caret in sight.
                            // Their meanings, where the definitions have them,
                            // take the pointer's hover instead of the word's.
                            let mut pill_help: Vec<(egui::Rect, String, String)> = Vec::new();
                            if let Some((_, inlays)) = &e.tools.inlays
                                && !inlays.is_empty()
                                && !ui.is_sizing_pass()
                            {
                                let hint = super::inlays::hint_font(&font);
                                let height = hint.size + 5.0;
                                let painter = ui.painter().with_clip_rect(output.text_clip_rect);
                                for inlay in inlays {
                                    // the glyph is in the galley's own
                                    // coordinates: the pill is on the screen
                                    let Some(rect) = glyph_rect(&output.galley, inlay.at) else {
                                        continue;
                                    };
                                    // an argument the hint's own room wrapped onto
                                    // the next row keeps that room at the end of
                                    // the row before: the pill stands there, just
                                    // before the line breaks
                                    let at = if rect.left() < inlay.space + inlay.before {
                                        output.galley.pos_from_cursor(egui::text::CCursor::new(inlay.at))
                                    } else {
                                        rect
                                    };
                                    if at.height() < 6.0 {
                                        continue;
                                    }
                                    let at = at.translate(output.galley_pos.to_vec2());
                                    if !output.text_clip_rect.intersects(at) {
                                        continue;
                                    }
                                    let pill = inlay.pill(at.left(), at.center().y, height);
                                    painter.rect_filled(
                                        pill,
                                        ui.visuals().widgets.inactive.corner_radius,
                                        p.raised.gamma_multiply(super::inlays::PILL_ALPHA),
                                    );
                                    let words = painter.layout_no_wrap(inlay.text.clone(), hint.clone(), p.muted);
                                    painter.galley(
                                        egui::pos2(pill.left() + super::inlays::PAD, pill.center().y - words.size().y / 2.0),
                                        words,
                                        p.muted,
                                    );
                                    if let Some(help) = &inlay.help {
                                        pill_help.push((pill, inlay.text.clone(), help.clone()));
                                    }
                                }
                            }
                            // the name being typed, completed under the caret,
                            // and the snippet's next placeholder
                            if editable && settings.completion && !ui.is_sizing_pass() {
                                if let Some(step) = step {
                                    complete::snippet_step(ui, id, e, step);
                                }
                                complete::after(ui, p, lang, id, e, &mut output, nav);
                            }
                            let mut chosen = None;
                            // the word the context menu was opened on, for its
                            // definition and references rows: read from the
                            // right click itself, since reaching for a row takes
                            // the pointer off the text (the words under it are
                            // the menu's own by then)
                            if output.response.secondary_clicked()
                                && let Some(pos) = output.response.interact_pointer_pos().or(output.response.hover_pos())
                            {
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                e.tools.menu_word = editing::word(e.source.as_str(), at, lang).to_owned();
                            }
                            let pointed = e.tools.menu_word.clone();
                            let (mut goto, mut references) = (None, None);
                            menu::context_menu(&output.response, p, |ui| {
                                edit_menu(ui, p, e, base_id, &mut chosen);
                                if !pointed.is_empty() && !e.linkset.is_nil() {
                                    menu::separator(ui, p);
                                    if menu::item(ui, p, "arrow-narrow-right", aurora_i18n::tr!("script-goto-definition")) {
                                        goto = Some(pointed.clone());
                                    }
                                    if menu::item(ui, p, "search-filled", aurora_i18n::tr!("script-find-references")) {
                                        references = Some(pointed.clone());
                                    }
                                }
                            });
                            if let Some(word) = goto {
                                e.tools.goto_word = Some(word);
                            }
                            if let Some(word) = references {
                                e.tools.references_word = Some(word);
                            }
                            if let Some(command) = chosen {
                                // Context commands use the same dispatcher as the menus.
                                ui.ctx().data_mut(|d| d.insert_temp(Id::unique((base_id, "command")), command));
                            }
                            let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
                            if ui
                                .ctx()
                                .data_mut(|d| d.remove_temp::<bool>(id.with("selection_error")))
                                .unwrap_or(false)
                            {
                                e.status = aurora_i18n::tr!("script-selection-edit-failed").into();
                            }
                            e.colors.draw(ui, p, text, &output, &e.highlighter.colors, color_editable);
                            // The script highlighter already paints every result a subtle amber.
                            // Make only the active Find result prominent, even while the Find
                            // floater owns the keyboard focus. Match notecard search styling.
                            if search.owns(base_id)
                                && !ui.memory(|m| m.has_focus(id))
                                && let Some(matches) = e.highlighter.matches_for(text, &query, insensitive, whole_words)
                                && let Some(range) = search.active_range_from_matches(ui.ctx(), id, matches)
                            {
                                let mut shapes = Vec::new();
                                for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, range) {
                                    if !rect.intersects(output.text_clip_rect) {
                                        continue;
                                    }
                                    let rect = rect.intersect(output.text_clip_rect);
                                    shapes.push(egui::Shape::rect_stroke(
                                        rect,
                                        ui.visuals().widgets.inactive.corner_radius,
                                        egui::Stroke::new(1.5, p.amber),
                                        egui::StrokeKind::Outside,
                                    ));
                                }
                                ui.painter().with_clip_rect(output.text_clip_rect).add(egui::Shape::Vec(shapes));
                            }
                            if editable
                                && (output.response.changed()
                                    || e.tools.history_cursor != output.cursor_range
                                    || ui
                                        .ctx()
                                        .data_mut(|d| d.remove_temp::<bool>(Id::unique((id, "history_dirty"))))
                                        .unwrap_or(false))
                            {
                                let range = output.cursor_range.unwrap_or_default();
                                let current = (range, e.source.clone());
                                let undo = output.state.undoer();
                                e.tools.can_undo = undo.has_undo(&current);
                                e.tools.can_redo = undo.has_redo(&current);
                                e.tools.history_cursor = output.cursor_range;
                            }
                            if let Some(range) = output.cursor_range {
                                e.tools.position = editing::position(text, range.primary.index.0);
                            }
                            if std::mem::take(&mut e.tools.scroll_to_cursor) {
                                let rect = output
                                    .galley
                                    .pos_from_cursor(editing::cursor(ui.ctx(), id).primary)
                                    .translate(output.galley_pos.to_vec2());
                                ui.scroll_to_rect(rect, Some(egui::Align::Center));
                            }
                            // The other places the word under the caret appears,
                            // dimmed behind the text (the box of a reference),
                            // then the caret's row and the indentation guides.
                            let mut beneath_shapes = Vec::new();
                            if e.loaded && !ui.is_sizing_pass() {
                                let caret_at = editing::cursor(ui.ctx(), id).primary.index.0;
                                if settings.occurrences
                                    && let Some((mine, found)) = editing::occurrences(text, caret_at, lang)
                                {
                                    for range in found {
                                        if range == mine {
                                            continue;
                                        }
                                        for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, range) {
                                            if !rect.intersects(output.text_clip_rect) {
                                                continue;
                                            }
                                            beneath_shapes.push(egui::Shape::rect_filled(
                                                rect.intersect(output.text_clip_rect).expand2(Vec2::new(0.5, 0.0)),
                                                ui.visuals().widgets.inactive.corner_radius,
                                                p.raised.gamma_multiply(0.38),
                                            ));
                                        }
                                    }
                                }
                                let caret = output
                                    .galley
                                    .pos_from_cursor(editing::cursor(ui.ctx(), id).primary)
                                    .translate(output.galley_pos.to_vec2());
                                let right = output.text_clip_rect.right();
                                if settings.current_line && caret.height() > 1.0 && caret.top() < output.text_clip_rect.bottom() {
                                    let band = egui::Rect::from_min_max(
                                        egui::pos2(output.galley_pos.x, caret.top()),
                                        egui::pos2(right, caret.bottom()),
                                    )
                                    .intersect(output.text_clip_rect);
                                    if band.height() > 1.0 {
                                        beneath_shapes.push(egui::Shape::rect_filled(band, 0.0, p.raised.gamma_multiply(0.30)));
                                    }
                                }
                                let space = ui.fonts_mut(|f| f.glyph_width(&font, ' '));
                                let lines: Vec<&str> = text.lines().collect();
                                let shown = visible_rows(&output.galley, output.galley_pos, output.text_clip_rect);
                                let mut line = e.first_line() + line_of_row(&output.galley, text, shown.start);
                                let mut line_start = shown.start == 0 || output.galley.rows[shown.start - 1].ends_with_newline;
                                for row in &output.galley.rows[shown] {
                                    let y = output.galley_pos.y + row.pos.y;
                                    if settings.indent_guides
                                        && line_start
                                        && row.height() >= 1.0
                                        && y + row.height() >= output.text_clip_rect.top()
                                        && let Some(prefix) = lines.get(line.saturating_sub(e.first_line()))
                                    {
                                        let tab = usize::from(settings.tab_size.max(1));
                                        for level in 1..=indent_columns(prefix, tab) / tab {
                                            let x = output.galley_pos.x + (level * tab) as f32 * space;
                                            if x >= output.text_clip_rect.right() - 2.0 {
                                                break;
                                            }
                                            beneath_shapes.push(egui::Shape::line_segment(
                                                [egui::pos2(x, y), egui::pos2(x, y + row.height())],
                                                egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6)),
                                            ));
                                        }
                                    }
                                    if row.ends_with_newline {
                                        line += 1;
                                    }
                                    line_start = row.ends_with_newline;
                                }
                                // the bracket at the caret and its match, lit
                                // behind the text; in Lua, the words of the
                                // block that holds the caret
                                let marks: Vec<std::ops::Range<usize>> = if !settings.bracket_match {
                                    Vec::new()
                                } else if lang.is_lua() {
                                    editing::keyword_pair(text, caret_at).map_or_else(Vec::new, |(open, close)| vec![open, close])
                                } else {
                                    Vec::new()
                                };
                                let marks: Vec<std::ops::Range<usize>> = if !settings.bracket_match {
                                    Vec::new()
                                } else if marks.is_empty() {
                                    editing::bracket_match(text, caret_at)
                                        .map_or_else(Vec::new, |(first, second)| vec![first..first + 1, second..second + 1])
                                } else {
                                    marks
                                };
                                for mark in marks {
                                    let start = output
                                        .galley
                                        .pos_from_cursor(egui::text::CCursor::new(mark.start))
                                        .translate(output.galley_pos.to_vec2());
                                    let end = output
                                        .galley
                                        .pos_from_cursor(egui::text::CCursor::new(mark.end))
                                        .translate(output.galley_pos.to_vec2());
                                    // a word split by the wrapping marks nothing
                                    if (start.top() - end.top()).abs() > 1.0 {
                                        continue;
                                    }
                                    let rect = egui::Rect::from_min_max(start.min, end.max);
                                    if rect.intersects(output.text_clip_rect) {
                                        beneath_shapes.push(egui::Shape::rect_filled(
                                            rect.expand2(Vec2::new(1.0, 0.0)),
                                            ui.visuals().widgets.inactive.corner_radius,
                                            p.raised.gamma_multiply(0.55),
                                        ));
                                    }
                                }
                            }
                            ui.painter().set(beneath, egui::Shape::Vec(beneath_shapes));
                            let left = gutter.left() + viewport.min.x;
                            let visible = ui.clip_rect().intersect(egui::Rect::everything_right_of(left + 56.0));
                            let content = egui::Rect::from_min_size(output.galley_pos, output.galley.size());
                            editing::drag_autoscroll(ui, &output.response, visible, content, !settings.word_wrap);
                            let clip = egui::Rect::from_min_max(
                                egui::pos2(left, ui.clip_rect().top()),
                                egui::pos2(left + 56.0, ui.clip_rect().bottom()),
                            );
                            let painter = ui.painter().with_clip_rect(clip);
                            painter.rect_filled(clip, 0.0, settings.color(ColorRole::Background, p));
                            // the lines whose text differs from the saved script
                            let changed = if settings.change_bars && e.loaded && !e.tools.processed {
                                changed_lines(text, &e.baseline)
                            } else {
                                None
                            };
                            let shown = visible_rows(&output.galley, output.galley_pos, clip);
                            let mut line = e.first_line() + line_of_row(&output.galley, text, shown.start);
                            let mut line_start = shown.start == 0 || output.galley.rows[shown.start - 1].ends_with_newline;
                            for row in &output.galley.rows[shown] {
                                let y = output.galley_pos.y + row.pos.y;
                                if line_start && row.height() >= 1.0 && y + row.height() >= clip.top() {
                                    painter.text(
                                        egui::pos2(left + 50.0, y),
                                        egui::Align2::RIGHT_TOP,
                                        line.to_string(),
                                        font.clone(),
                                        if line == e.tools.position.0 + e.first_line() {
                                            settings.color(ColorRole::Cursor, p)
                                        } else {
                                            p.muted
                                        },
                                    );
                                    if let Some((first, last)) = changed
                                        && (first..=last).contains(&line.saturating_sub(e.first_line()))
                                    {
                                        painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(left + 52.0, y + 1.0),
                                                egui::pos2(left + 55.0, y + row.height() - 1.0),
                                            ),
                                            0.0,
                                            p.amber,
                                        );
                                    }
                                    // the lines a diagnostic is reported on (the
                                    // dock may list every script of the window)
                                    if problems.iter().any(|problem| problem.line == line && problem.source == e.name) {
                                        painter.circle_filled(egui::pos2(left + 4.0, y + row.height() / 2.0), 2.5, p.danger);
                                    }
                                    // a reader's bookmark: a mark in the gutter, and a
                                    // click on the line's place in it toggles it
                                    let index = line.saturating_sub(e.first_line());
                                    if e.bookmarks.contains(&index) {
                                        painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(left + 8.0, y + 3.0),
                                                egui::pos2(left + 12.0, y + row.height() - 3.0),
                                            ),
                                            0.0,
                                            p.indigo_light,
                                        );
                                    }
                                    let hit = ui.interact(
                                        egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(left + 16.0, y + row.height())),
                                        egui::Id::unique((id, "gutter", index)),
                                        egui::Sense::click(),
                                    );
                                    if hit.clicked() && !e.bookmarks.remove(&index) {
                                        e.bookmarks.insert(index);
                                    }
                                }
                                if row.ends_with_newline {
                                    line += 1;
                                }
                                line_start = row.ends_with_newline;
                            }
                            painter.line_segment(
                                [egui::pos2(left + 55.0, clip.top()), egui::pos2(left + 55.0, clip.bottom())],
                                egui::Stroke::new(1.0, p.muted_dim),
                            );
                            // The diagnostics of this script in the code: a
                            // squiggle under the span the compile points at
                            // (LSL gives a place, not a length), so that a row
                            // of the dock is visible where it matters.
                            let squiggle = ui.painter().with_clip_rect(output.text_clip_rect);
                            for problem in problems.iter().filter(|problem| problem.source == e.name) {
                                if problem.line == 0 {
                                    continue;
                                }
                                let line = problem.line.saturating_sub(e.first_line());
                                let span = editing::error_span(text, line, problem.column);
                                let warning = problem.message.to_ascii_uppercase().contains("WARNING");
                                let color = if warning { p.amber } else { p.danger };
                                for rect in crate::ui::chat::range_rects(&output.galley, output.galley_pos, span) {
                                    if !rect.intersects(output.text_clip_rect) {
                                        continue;
                                    }
                                    let y = rect.bottom() - 2.0;
                                    let mut points = Vec::new();
                                    let mut x = rect.left();
                                    let mut up = true;
                                    while x <= rect.right() {
                                        points.push(egui::pos2(x, if up { y } else { y - 1.8 }));
                                        x += 2.0;
                                        up = !up;
                                    }
                                    squiggle.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
                                }
                            }
                            if e.folding.reveal(output.cursor_range.unwrap_or_default().primary) {
                                ui.ctx().request_repaint();
                            }
                            e.folding.draw(ui, p, id, &output, left);
                            // the help of the word under the pointer, over code
                            // only: a name in a comment or a string is not the
                            // keyword (LLScriptEdCore::updateDynamicHelp)
                            // no help while a menu stands at the pointer: a
                            // tooltip is drawn above every menu
                            // (`Order::Tooltip` over `Order::Foreground`), and
                            // the help would hide the menu's first entries
                            let rest = id.with("help_rest");
                            if !egui::Popup::is_any_open(ui.ctx())
                                && let Some(pos) = output.response.hover_pos()
                            {
                                // a pill says what its argument means; the word
                                // under the pointer is no part of it
                                let pill = pill_help.iter().position(|(pill, _, _)| pill.contains(pos));
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                // what stands under the pointer, as the wait's
                                // key: a name is the word's own place in the
                                // source, so moving within it keeps the wait
                                let on = match pill {
                                    Some(index) => (index, true),
                                    None => (editing::word(text, at, lang).as_ptr() as usize, false),
                                };
                                // the help waits a moment, so that crossing a
                                // line of code shows nothing
                                if rested(ui, rest, on) {
                                    if let Some(index) = pill {
                                        let (_, name, help) = &pill_help[index];
                                        let response = (*output.response).clone();
                                        egui::Tooltip::for_widget(&response).at_pointer().show(|ui| {
                                            ui.set_max_width(300.0);
                                            ui.spacing_mut().item_spacing.y = 3.0;
                                            ui.label(RichText::new(name).font(egui::FontId::monospace(11.0)).color(p.ink));
                                            ui.add(egui::Label::new(RichText::new(help).size(11.0).color(p.muted)).wrap());
                                        });
                                    } else if let Some(entry) = reference::hover(text, at, lang) {
                                        // at the pointer, not under the whole source
                                        let response = (*output.response).clone();
                                        egui::Tooltip::for_widget(&response)
                                            .at_pointer()
                                            .show(|ui| hover_docs(ui, p, entry, settings.deprecations, settings.arguments));
                                    }
                                }
                            } else {
                                // the pointer left the source: the next name
                                // waits its moment again
                                ui.ctx().data_mut(|data| data.remove::<Resting>(rest));
                            }
                            // a click on the text brings this pane to the front
                            // (a split's other pane, from `split_body`)
                            e.tools.clicked = output.response.clicked() && !ui.ctx().input(|i| i.modifiers.command || i.modifiers.ctrl);
                            // Ctrl (Cmd) on a name and a click: the window takes
                            // the caret to where the name is declared
                            if output.response.clicked()
                                && ui.ctx().input(|i| i.modifiers.command || i.modifiers.ctrl)
                                && let Some(pos) = output.response.interact_pointer_pos()
                            {
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                let word = editing::word(text, at, lang).to_owned();
                                if !word.is_empty() {
                                    e.tools.goto_word = Some(word);
                                }
                            }
                            // the line at the top of the viewport: the sticky
                            // line of the next frame names what holds it
                            let last_row = output.galley.rows.len().saturating_sub(1);
                            let shown = visible_rows(&output.galley, output.galley_pos, output.text_clip_rect);
                            let first = line_of_row(&output.galley, text, shown.start.min(last_row));
                            let last = line_of_row(&output.galley, text, shown.end.saturating_sub(1).min(last_row));
                            e.tools.first_visible = first;
                            e.tools.last_visible = last;
                            // The overview strip at the right of the source: the
                            // whole script in the height of the window, with
                            // what is visible, the changed lines and the
                            // diagnostics; a click goes there. Last, so that it
                            // paints over the text it maps.
                            e.tools.overview = None;
                            if e.loaded && !ui.is_sizing_pass() && !output.galley.rows.is_empty() {
                                let bar = ui.spacing().scroll.bar_width + ui.spacing().scroll.bar_inner_margin;
                                let right = pane_right - bar - 1.0;
                                let rail = if settings.minimap {
                                    f32::from(settings.minimap_width.clamp(40, 100))
                                } else {
                                    OVERVIEW_WIDTH
                                };
                                let strip = egui::Rect::from_min_max(
                                    egui::pos2(right - rail, ui.clip_rect().top() + 1.0),
                                    egui::pos2(right, ui.clip_rect().bottom() - 1.0),
                                );
                                if strip.width() > 1.0 && strip.height() > 1.0 {
                                    if settings.minimap {
                                        minimap(ui, p, e, id, settings, strip, problems);
                                    } else {
                                        overview(ui, p, e, id, strip, viewport, &output.galley.rows, problems, settings.change_bars);
                                    }
                                }
                            }
                        });
                    });
            });
        });
}

/// Where the character `at` is drawn: its glyph's row, and the x it starts at.
/// `Galley::pos_from_cursor` reports a character that starts a wrapped row at
/// the end of the row before it, which would put a pill on that row's words.
fn glyph_rect(galley: &egui::Galley, at: usize) -> Option<egui::Rect> {
    let mut index = 0;
    for row in &galley.rows {
        let count = row.row.glyphs.len();
        if at < index + count {
            let glyph = &row.row.glyphs[at - index];
            let x = row.pos.x + glyph.pos.x;
            return Some(egui::Rect::from_min_max(
                egui::pos2(x, row.pos.y),
                egui::pos2(x, row.pos.y + row.row.size.y),
            ));
        }
        index += count + usize::from(row.ends_with_newline);
    }
    None
}

/// The parameter names of the calls on screen, with the room each takes: made
/// again only when the text, its language or the editor's font size changes.
fn update_inlays(e: &mut Editor, ui: &egui::Ui, font: &egui::FontId, lang: Lang) {
    let lua = lang.is_lua();
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    e.text_key(Some(ui.input(|i| i.time))).hash(&mut hasher);
    lua.hash(&mut hasher);
    font.size.to_bits().hash(&mut hasher);
    let key = hasher.finish();
    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
    if e.tools.inlays.as_ref().is_some_and(|(made, _)| *made == key) {
        return;
    }
    let hint = super::inlays::hint_font(font);
    // the colour literals the syntax keeps a preview's room for: their swatch
    // is drawn there, and a pill stands left of it
    let colors: Vec<usize> = if lua {
        Vec::new()
    } else {
        super::syntax::colors(text).into_iter().map(|literal| literal.bytes.start).collect()
    };
    let list = super::inlays::hints(text, lang, &colors, |words| {
        ui.fonts_mut(|fonts| fonts.layout_no_wrap(words.to_owned(), hint.clone(), egui::Color32::PLACEHOLDER))
            .size()
            .x
    });
    e.tools.inlays = Some((key, list));
}

/// The bar under the source: the path of the caret -- the script, then the
/// text that opens each block it is in, outermost first -- each step a click
/// away from its line, and at the right what the script counts and whether it
/// can be changed. An IDE puts the same bar under its editor, with a longer
/// trailer; the dock's grip lies over the middle of it, the bar fading under it.
#[allow(clippy::too_many_arguments)]
pub(super) fn crumbs_row(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, settings: &ScriptSettings, trailer: bool) {
    // the source or what the preprocessor made of it, when it runs: the choice
    // is the pane's, so it sits in the pane's own bar under the source. A Lua
    // script is never preprocessed (the preprocessor is LSL's), so it has no
    // second view, whatever the setting says.
    let either = !e.lua() && (settings.preprocessor || e.tools.has_processed || e.tools.processed);
    if !either {
        e.tools.processed = false;
    }
    let lang = e.lang();
    let line = e.tools.position.0;
    let path = e.analysis(Some(ui.input(|i| i.time))).path(line, lang);
    let name = e.name.clone();
    let read_only = e.loaded && !e.can_edit;
    let mut jump = None;
    // The bar of the path, under the source: a quiet face, and a chip per
    // block that holds the caret, the one it is in lit.
    // the bar keeps its row, with no face of its own: the path and the view's
    // words read on the window between the source and the dock, centred in it
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), CRUMBS_HEIGHT), egui::Sense::hover());
    let inner = egui::Rect::from_min_max(bar.min + Vec2::new(8.0, 0.0), bar.max - Vec2::new(6.0, 0.0));
    let inner_width = inner.width();
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    // the trailer: the view switch, and whether the text can be changed.
    // Each takes its own room first, so that the path is planned against what
    // is really left.
    let processed = e.tools.processed;
    let processed_word = aurora_i18n::tr!("script-tab-processed").to_string();
    let source_word = aurora_i18n::tr!("script-tab-source").to_string();
    if either {
        if inner_width < COMPACT_BAR {
            // too narrow for both words: one dropdown reading the view shown,
            // no face of its own until the pointer is on it
            let shown = if processed { &processed_word } else { &source_word };
            let room = word_room(&ui, shown) + 12.0;
            let mut pick = processed;
            let _ = bar_dropdown(&mut ui, p, shown, room, |ui| {
                if menu::check(ui, p, "code", &source_word, !processed, true) {
                    pick = false;
                }
                if menu::check(ui, p, "file-upload-filled", &processed_word, processed, true) {
                    pick = true;
                }
            });
            if pick != processed {
                e.tools.processed = pick;
            }
        } else {
            // the two views, as words in their own room: the one shown in ink,
            // the other dimmed, and neither ever cut
            let processed_room = word_room(&ui, &processed_word);
            let source_room = word_room(&ui, &source_word);
            if switch_word(&mut ui, p, &processed_word, processed, processed_room) {
                e.tools.processed = true;
            }
            // the two words apart, as they were read as one in a narrow pane
            ui.add_space(6.0);
            if switch_word(&mut ui, p, &source_word, !processed, source_room) {
                e.tools.processed = false;
            }
        }
    }
    if trailer
        && read_only
        && let Some(t) = crate::ui::icons::global("lock-filled")
    {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(14.0)).tint(p.warn))
            .on_hover_text(aurora_i18n::tr!("script-read-only"));
    }
    // the path, on the left, in the panes' order as the source is: the prim
    // and the script always, its blocks from the innermost out while they fit,
    // the outer ones folded into one chip. The script's name is cut last.
    let width = ui.available_width();
    let prim = e.tools.prim.clone();
    let script_icon = crate::ui::inventory::item_icon(10, 10, u32::from(e.lua()));
    let script_tint = if e.running == Some(true) { p.teal } else { p.muted };
    let prim_width = if prim.is_empty() { 0.0 } else { crumb_width(&ui, &prim, true) };
    let script_width = crumb_width(&ui, &name, true);
    let blocks: Vec<f32> = path.iter().map(|step| crumb_width(&ui, &step.text, false)).collect();
    let plan = crumbs_plan(width, prim_width, script_width, &blocks);
    // what stands after the script: the folded chip, then the kept blocks
    let tail = (if plan.from > 0 { CRUMB_FOLD + CRUMB_SEP } else { 0.0 }) + blocks[plan.from..].iter().map(|w| w + CRUMB_SEP).sum::<f32>();
    let head = if plan.prim { prim_width + CRUMB_SEP } else { 0.0 };
    let script_room = (width - head - tail).max(48.0);
    // the inner rect's height, not CRUMBS_HEIGHT again: a row taller than the
    // room it is given would hang below the middle of it
    ui.allocate_ui_with_layout(
        Vec2::new(width, inner.height()),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            // where in the object: the prim that holds the script, when the
            // window is a linkset's, then the script, then its blocks -- each
            // with the icon its row has in the tree, so that what the words
            // name is read at a glance
            if plan.prim {
                crumb(ui, p, &prim, Some(("Inv_Object", p.violet_light)), false, false, f32::INFINITY);
                chevron(ui, p);
            }
            crumb(ui, p, &name, Some((script_icon, script_tint)), false, false, script_room);
            if plan.from > 0 {
                chevron(ui, p);
                let dropped = &path[..plan.from];
                let tip = dropped.iter().map(|step| step.text.as_str()).collect::<Vec<_>>().join(" › ");
                let mut folded = None;
                let _ = bar_dropdown(ui, p, "…", CRUMB_FOLD, |ui| {
                    for step in dropped {
                        if menu::item(ui, p, "corner-down-right", &step.text) {
                            folded = Some(step.line);
                        }
                    }
                })
                .on_hover_text(tip);
                if let Some(line) = folded {
                    jump = Some(line);
                }
            }
            for (index, step) in path.iter().enumerate().skip(plan.from) {
                chevron(ui, p);
                let last = index + 1 == path.len();
                if crumb(ui, p, &step.text, None, last, true, f32::INFINITY).clicked() {
                    jump = Some(step.line);
                }
            }
        },
    );
    if let Some(line) = jump {
        go_to_line(ui.ctx(), e, id, line, 0);
    }
}

/// One word of the source / preprocessed switch, in its own room so that it is
/// never cut: the view shown in ink, the other muted.
fn switch_word(ui: &mut egui::Ui, p: &Palette, text: &str, on: bool, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 18.0), egui::Sense::click());
    let font = crate::ui::widgets::control_font(ui, CRUMB_TEXT);
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        if on { p.ink } else { p.muted },
    );
    !on && response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The room a word of the switch takes, and the point that keeps it whole.
fn word_room(ui: &egui::Ui, text: &str) -> f32 {
    let font = crate::ui::widgets::control_font(ui, CRUMB_TEXT);
    ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER))
        .size()
        .x
        + 2.0
}

/// A control of the bar that opens a menu: its word and a small chevron, at
/// the bar's own height and text size, on no face of its own until the pointer
/// is on it -- the theme's combo box would stand too tall here, and its
/// chevron too big beside the bar's text.
fn bar_dropdown(ui: &mut egui::Ui, p: &Palette, text: &str, room: f32, add: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(room, 16.0), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.25));
    }
    let font = crate::ui::widgets::control_font(ui, CRUMB_TEXT);
    ui.painter().text(
        egui::pos2(rect.left() + 2.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font,
        if hovered { p.ink } else { p.muted },
    );
    crate::ui::widgets::paint_glyph(
        ui.painter(),
        "chevron-down",
        egui::pos2(rect.right() - 6.0, rect.center().y),
        8.0,
        p.muted,
    );
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, add);
    response
}

/// The chevron between two steps of the path: one `CRUMB_SEP` with its air.
fn chevron(ui: &mut egui::Ui, p: &Palette) {
    if let Some(t) = crate::ui::icons::global("chevron-right") {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(9.0)).tint(p.muted_dim));
    }
}

/// The room a step of the path takes, without drawing it: its words and their
/// air, which the plan spends before it knows what fits.
fn crumb_width(ui: &egui::Ui, text: &str, icon: bool) -> f32 {
    let font = crate::ui::widgets::control_font(ui, CRUMB_TEXT);
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), f32::INFINITY, font);
    galley.size().x + 10.0 + if icon { ICON_CRUMB } else { 0.0 }
}

/// What the path's row keeps when the bar is narrow: the prim's chip (unless
/// even it does not fit with the script), and the blocks from the innermost
/// out, the outer ones folded into one chip. `from` is the first block kept.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CrumbsPlan {
    prim: bool,
    from: usize,
}

fn crumbs_plan(room: f32, prim: f32, script: f32, blocks: &[f32]) -> CrumbsPlan {
    // the identity first: the prim gives way before the script's name is cut
    let mut show_prim = prim > 0.0;
    let mut identity = if show_prim { prim + CRUMB_SEP } else { 0.0 } + script;
    if identity > room {
        show_prim = false;
        identity = script;
    }
    // the blocks from the innermost out, one more folded at a time
    let mut from = 0;
    loop {
        let folded = if from > 0 { CRUMB_FOLD + CRUMB_SEP } else { 0.0 };
        let cost = identity + folded + blocks[from..].iter().map(|w| w + CRUMB_SEP).sum::<f32>();
        if cost <= room || from == blocks.len() {
            return CrumbsPlan { prim: show_prim, from };
        }
        from += 1;
    }
}

/// One word of the path: a chip, lit when it is the block the caret is in, and
/// lit again under the pointer when a click goes to it.
fn crumb(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &str,
    icon: Option<(&str, egui::Color32)>,
    current: bool,
    jumps: bool,
    room: f32,
) -> egui::Response {
    let font = crate::ui::widgets::control_font(ui, CRUMB_TEXT);
    // the icon of what the words name, ahead of them: the object the script is
    // in, the script itself; a block of it has none
    let icon_room = if icon.is_some() { ICON_CRUMB } else { 0.0 };
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), room, font);
    let pad = Vec2::new(5.0, 1.0);
    let (rect, response) = ui.allocate_exact_size(
        galley.size() + pad * 2.0 + Vec2::new(icon_room, 0.0),
        if jumps { egui::Sense::click() } else { egui::Sense::hover() },
    );
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if current {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if jumps && response.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.30));
    }
    let color = if current { p.ink } else { p.muted };
    let mut left = rect.left() + pad.x;
    if let Some((name, tint)) = icon
        && let Some(t) = crate::ui::icons::global(name)
    {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(left + 5.0, rect.center().y), Vec2::splat(12.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
        left += icon_room;
    }
    ui.painter()
        .galley(egui::pos2(left, rect.center().y - galley.size().y / 2.0), galley, color);
    if jumps {
        response
            .on_hover_text(aurora_i18n::tr!("script-crumbs-jump"))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// The help shows once the pointer has rested on a name this long, in seconds:
/// crossing a line of code shows nothing, and the reader who stops reads.
const HOVER_DELAY: f64 = 0.45;

/// What the pointer rests on, and since when: the key of the name under it, and
/// the second the wait began.
#[derive(Clone, Copy)]
struct Resting {
    on: (usize, bool),
    since: f64,
}

/// How long the pointer has rested on `on`, in seconds: another key starts the
/// wait again, and the first look at a key starts it at `now`.
fn rested_for(resting: &mut Resting, on: (usize, bool), now: f64) -> f64 {
    if resting.on != on {
        *resting = Resting { on, since: now };
    }
    now - resting.since
}

/// Whether the pointer has rested on `on` long enough for its help. While it
/// has not, the frame asks for the one repaint that will show it, so that the
/// help comes without the pointer moving.
fn rested(ui: &egui::Ui, id: egui::Id, on: (usize, bool)) -> bool {
    let now = ui.input(|i| i.time);
    let mut resting = ui
        .ctx()
        .data(|data| data.get_temp::<Resting>(id))
        .unwrap_or(Resting { on, since: now });
    let waited = rested_for(&mut resting, on, now);
    ui.ctx().data_mut(|data| data.insert_temp(id, resting));
    if waited >= HOVER_DELAY {
        true
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY - waited));
        false
    }
}

/// The help of a word, over its name: the signature as the catalog writes it,
/// the language's deprecation when it has one, then what it does
/// (LLScriptEdCore::updateDynamicHelp).
pub(super) fn hover_docs(ui: &mut egui::Ui, p: &Palette, entry: &reference::Entry, show_deprecations: bool, show_arguments: bool) {
    ui.set_max_width(360.0);
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(RichText::new(&entry.signature).font(egui::FontId::monospace(12.0)).color(p.ink));
    if show_deprecations && entry.deprecated {
        let text = match &entry.replaced_by {
            Some(name) => aurora_i18n::tr!("script-deprecated-use", name = name.as_str()),
            None => aurora_i18n::tr!("script-deprecated").to_string(),
        };
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(p.warn)).wrap());
    }
    if !entry.tooltip.is_empty() {
        ui.add(egui::Label::new(RichText::new(&entry.tooltip).size(11.0).color(p.muted)).wrap());
    }
    if show_arguments {
        let name = egui::FontId::monospace(11.0);
        let body = egui::FontId::proportional(11.0);
        for argument in &entry.arguments {
            if argument.help.is_empty() {
                continue;
            }
            let mut line = LayoutJob::default();
            line.append(
                &argument.name,
                0.0,
                TextFormat {
                    font_id: name.clone(),
                    color: p.ink,
                    ..Default::default()
                },
            );
            line.append(
                "  ",
                0.0,
                TextFormat {
                    font_id: body.clone(),
                    color: p.muted,
                    ..Default::default()
                },
            );
            line.append(
                &argument.help,
                0.0,
                TextFormat {
                    font_id: body.clone(),
                    color: p.muted,
                    ..Default::default()
                },
            );
            ui.add(egui::Label::new(line).wrap());
        }
    }
}

pub(super) fn dialogs(
    ctx: &egui::Context,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &mut ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
) {
    if let Some(command) = ctx.data_mut(|d| d.remove_temp::<Command>(Id::unique((id, "command")))) {
        run(command, e, ctx, id, settings, prefs, actions);
    }
    let original_id = id;
    let id = visible_id(id, e.tools.processed);
    if e.tools.goto {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_goto"),
            aurora_i18n::tr!("script-goto"),
            ctx.content_rect().center(),
            Vec2::new(285.0, 115.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.horizontal(|ui| {
                ui.label(if e.lua() {
                    aurora_i18n::tr!("script-goto-line-lua")
                } else {
                    aurora_i18n::tr!("script-goto-line")
                });
                let input = ui.add(crate::ui::widgets::restricted_number(
                    egui::DragValue::new(&mut e.tools.goto_line).range(0..=usize::MAX),
                    crate::ui::widgets::TextPolicy::Digits,
                ));
                if flat_button(ui, p, aurora_i18n::tr!("script-goto-go")).clicked()
                    || (input.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
                {
                    let at = editing::line_cursor(text(e), e.tools.goto_line.saturating_sub(e.first_line()), 0);
                    editing::select(ctx, id, at..at, true);
                    e.tools.goto = false;
                    e.tools.scroll_to_cursor = true;
                }
            });
        });
        e.tools.goto &= open;
    }
    if e.tools.revert {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_revert"),
            aurora_i18n::tr!("script-menu-revert"),
            ctx.content_rect().center(),
            Vec2::new(405.0, 130.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("script-revert-question"));
            ui.horizontal(|ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-menu-revert")).clicked() {
                    let end = e.source.chars().count();
                    editing::replace(ctx, original_id, &mut e.source, 0..end, &e.baseline);
                    e.compile = e.baseline_compile;
                    e.experience.selected = e.experience.baseline;
                    e.tools.revert = false;
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    e.tools.revert = false;
                }
            });
        });
        e.tools.revert &= open;
    }
    if e.tools.incoming.is_some() {
        let mut open = true;
        Floater::new(
            &format!("{id:?}_file_conflict"),
            aurora_i18n::tr!("script-menu-open"),
            ctx.content_rect().center(),
            Vec2::new(420.0, 150.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("script-file-conflict"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled_ui(e.can_edit && e.saving.is_none(), |ui| {
                        flat_button(ui, p, aurora_i18n::tr!("script-file-use"))
                    })
                    .inner
                    .clicked()
                    && let Some(source) = e.tools.incoming.take()
                {
                    let end = e.source.chars().count();
                    editing::replace(ctx, original_id, &mut e.source, 0..end, &source);
                    e.tools.external_snapshot.clone_from(&e.source);
                    if e.tools.incoming_external {
                        actions.extend(e.save());
                    }
                }
                if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    e.tools.incoming = None;
                }
            });
        });
        if !open {
            e.tools.incoming = None;
        }
    }
}

pub(super) fn search(
    ctx: &egui::Context,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    search: &mut Search,
    (parent_id, parent): (Id, egui::Rect),
    prefer_left: bool,
) {
    let id = visible_id(id, e.tools.processed);
    let cursor = editing::cursor(ctx, id);
    let editable = e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed && !e.colors.active();
    let owner_layer = egui::LayerId::new(egui::Order::Middle, parent_id);
    let source = if e.tools.processed {
        &mut e.tools.processed_source
    } else {
        &mut e.source
    };
    if let Some(anchor) = e.tools.search_anchor.take() {
        // In the window: a bar under its menus and toolbar, over the source,
        // which keeps every row it had (a script's own window or a linkset's)
        search.popup(ctx, p, id, source, editable, anchor, owner_layer);
        e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
        return;
    }
    let (pos, _overlap) = Search::placement(ctx, parent, e.tools.body_top, prefer_left);
    // Dependent Find follows its active script parent whether overlapping or
    // beside it. Do not force Find over unrelated windows when the editor is inactive.
    let owner = owner_layer;
    let child = egui::LayerId::new(egui::Order::Middle, Id::unique(Search::WINDOW));
    ctx.set_sublayer(owner, child);
    // A keyboard-focused Find field can remain focused while another window
    // is explicitly raised. Only an actual interaction with this editor/find
    // pair (or the initial Find opening) should raise the pair again.
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let pressed_here = pressed.is_some_and(|at| matches!(ctx.layer_id_at(at), Some(layer) if layer == owner || layer == child));
    if (search.focus && pressed.is_none()) || pressed_here || ctx.is_being_dragged(child.id.with("move")) {
        ctx.move_to_top(owner);
    }
    search.show(ctx, p, id, source, editable, pos);
    e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
}

/// The file job of one document (opened, exported, sent to the external
/// editor): its result reaches the document even while its tab is not the one
/// in front.
pub(super) fn poll_files(ctx: &egui::Context, e: &mut Editor, id: Id, settings: &ScriptSettings, actions: &mut Vec<Action>) {
    if let Some(done) = e.tools.file_job.as_ref().and_then(|j| j.0.try_recv().ok()) {
        e.tools.file_job = None;
        e.tools.external_starting = false;
        match done {
            files::Done::Opened(source) if e.can_edit && e.loaded && e.saving.is_none() && e.source == e.tools.file_snapshot => {
                let end = e.source.chars().count();
                editing::replace(ctx, id, &mut e.source, 0..end, &source);
                e.status = aurora_i18n::tr!("script-file-opened").into();
            }
            files::Done::Opened(source) => {
                e.tools.incoming = Some(source);
                e.tools.incoming_external = false;
            }
            files::Done::Saved => e.status = aurora_i18n::tr!("script-file-saved").into(),
            files::Done::External(watch) => {
                e.tools.external = Some(watch);
                e.tools.external_snapshot.clone_from(&e.tools.file_snapshot);
                e.status = aurora_i18n::tr!("script-external-started").into();
            }
            files::Done::Failed(reason) => e.status = reason,
            files::Done::Cancelled => {}
        }
    }
    let mut latest = None;
    if let Some(watch) = e.tools.external.as_ref() {
        for source in watch.changes.try_iter() {
            latest = Some(source);
        }
    }
    if let Some(result) = latest {
        match result {
            Ok(source) if e.can_edit && e.loaded && e.saving.is_none() && e.source == e.tools.external_snapshot => {
                let end = e.source.chars().count();
                editing::replace(ctx, id, &mut e.source, 0..end, &source);
                e.tools.external_snapshot.clone_from(&e.source);
                if let Some(action) = e.save() {
                    actions.push(action);
                }
            }
            Ok(source) => {
                e.tools.incoming = Some(source);
                e.tools.incoming_external = true;
            }
            Err(reason) => e.status = reason,
        }
    }
    let _ = settings;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help's wait starts on a name and starts again on another: moving
    /// within one name keeps the wait, crossing a line does not.
    #[test]
    fn the_help_waits_on_a_name_and_starts_again_on_the_next() {
        let mut resting = Resting {
            on: (7, false),
            since: 10.0,
        };
        assert_eq!(rested_for(&mut resting, (7, false), 10.25), 0.25, "the wait goes on");
        assert_eq!(rested_for(&mut resting, (7, false), 10.5), 0.5, "and ends");
        assert_eq!(rested_for(&mut resting, (9, false), 11.0), 0.0, "another name starts again");
        assert_eq!(rested_for(&mut resting, (9, false), 11.25), 0.25);
        assert_eq!(rested_for(&mut resting, (0, true), 11.5), 0.0, "a pill is no word");
        assert_eq!(rested_for(&mut resting, (0, true), 12.0), 0.5);
    }

    /// The help under a word says when the language deprecated it, and what
    /// to use instead, only while the setting asks for it.
    #[test]
    fn the_help_marks_a_deprecated_call_only_when_asked() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let entry = super::reference::get("llSetPrimitiveParams", Lang::Lsl).expect("a deprecated call");
        let want = aurora_i18n::tr!("script-deprecated-use", name = "llSetLinkPrimitiveParamsFast");
        let drawn = |show: bool| {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                hover_docs(ui, &p, entry, show, false);
            });
            output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == want))
        };
        assert!(drawn(true), "the line, with the setting on");
        assert!(!drawn(false), "and none with it off");
        assert!(
            crate::ui::scripts::settings::ScriptSettings::default().deprecations,
            "on, as the aids are"
        );
    }

    /// The help lists each parameter of a call and what it means, only while
    /// the setting asks for it; the signature itself names them either way.
    #[test]
    fn the_help_lists_the_arguments_and_their_meaning_only_when_asked() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let entry = super::reference::get("llListen", Lang::Lsl).expect("a call");
        let want = "Specific prim name or agent legacy name to filter by";
        let drawn = |show: bool| {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                hover_docs(ui, &p, entry, false, show);
            });
            output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(want)))
        };
        assert!(drawn(true), "the meaning of the argument, with the setting on");
        assert!(!drawn(false), "and none with it off");
    }

    /// A narrow bar folds the outer steps of the path into one chip and keeps
    /// the prim and the script; the script's name gives way last.
    #[test]
    fn a_narrow_bar_folds_the_outer_blocks_and_keeps_the_script() {
        let blocks = [100.0, 90.0, 80.0];
        // a wide bar keeps every step, the prim included
        assert_eq!(crumbs_plan(900.0, 60.0, 70.0, &blocks), CrumbsPlan { prim: true, from: 0 });
        // half a window: the outer blocks fold, the identity stays whole
        assert_eq!(crumbs_plan(300.0, 60.0, 70.0, &blocks), CrumbsPlan { prim: true, from: 2 });
        // a sliver: the prim goes before the script's name is cut
        assert_eq!(crumbs_plan(120.0, 60.0, 70.0, &blocks), CrumbsPlan { prim: false, from: 3 });
        // no block at all: nothing to fold
        assert_eq!(crumbs_plan(120.0, 60.0, 70.0, &[]), CrumbsPlan { prim: false, from: 0 });
    }

    /// A dock tab's plate keeps the well's air around its controls, and the
    /// list's scroll bar its room: a control never lies on the plate's edge,
    /// across the top of its list (Search, References) or its bottom (Console,
    /// Outline).
    #[test]
    fn a_crumb_with_an_icon_keeps_it_before_its_words() {
        let ctx = egui::Context::default();
        // the icons the chip draws are the skin's set, loaded as the viewer does
        let _icons = crate::ui::icons::Icons::load(&ctx, None);
        let p = crate::theme::Theme::default().palette();
        let mut plain = egui::Rect::NOTHING;
        let mut icon = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            plain = crumb(ui, &p, "Lamp", None, false, false, f32::INFINITY).rect;
            icon = crumb(ui, &p, "Lamp", Some(("Inv_Object", p.violet_light)), false, false, f32::INFINITY).rect;
        });
        output.textures_delta.clear();
        assert!(icon.width() >= plain.width() + ICON_CRUMB - 1.0, "{plain:?} / {icon:?}");
        // the icon is drawn in the chip, and the words start past it
        let mut starts: Vec<f32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.job.text == "Lamp" => Some(t.pos.x),
                _ => None,
            })
            .collect();
        starts.sort_by(f32::total_cmp);
        assert_eq!(starts.len(), 2, "both chips' words");
        assert!(
            starts[1] >= starts[0] + ICON_CRUMB - 1.0,
            "the icon'd chip's words start past its icon: {starts:?}"
        );
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if !mesh.vertices.is_empty())),
            "the icon's mesh"
        );
    }

    /// The dock keeps the height its edge was dragged to, within its least and
    /// the room the window leaves once the text has its own; the default is a
    /// third of the window at most, and folding gives it its header alone.

    #[test]
    fn the_changed_lines_are_the_region_between_the_saved_and_the_edited_text() {
        // no change: no bars
        assert_eq!(changed_lines("a\nb\n", "a\nb\n"), None);
        // one line edited in the middle
        assert_eq!(changed_lines("a\nB\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // a line added in the middle marks it alone
        assert_eq!(changed_lines("a\nnew\nb\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // a line removed marks the line the rest moved onto
        assert_eq!(changed_lines("a\nc\n", "a\nb\nc\n"), Some((1, 1)));
        // everything differs
        assert_eq!(changed_lines("x\n", "a\nb\n"), Some((0, 0)));
        // the end of the file
        assert_eq!(changed_lines("a\nb\nc\n", "a\nb\n"), Some((2, 2)));
    }

    #[test]
    fn indentation_is_counted_in_columns_and_a_tab_reaches_the_next_stop() {
        assert_eq!(indent_columns("    x", indent::TAB_SIZE), 4);
        assert_eq!(indent_columns("\tx", indent::TAB_SIZE), 4);
        assert_eq!(indent_columns("  \tx", indent::TAB_SIZE), 4);
        assert_eq!(indent_columns("\t\tx", indent::TAB_SIZE), 8);
        assert_eq!(indent_columns("x", indent::TAB_SIZE), 0);
        assert_eq!(indent_columns("", indent::TAB_SIZE), 0);
        // the spaces of a comment that ends the line are not indentation
        assert_eq!(indent_columns("//      note", indent::TAB_SIZE), 0);
    }

    #[test]
    fn the_status_line_sizes_read_as_a_person_expects() {
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(2048), "2.0 KB");
        assert_eq!(size_label(1024 * 1024), "1 MB");
    }

    #[test]
    fn script_reset_click_does_not_require_source_permissions_or_running_info() {
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;

        for (in_object, readable, editable, available, saving, expected) in [
            (true, false, false, true, false, true),
            (true, true, false, true, false, true),
            (true, true, true, true, false, true),
            (true, false, false, false, false, false),
            // a save in flight keeps the button on the row (greyed): what it
            // offers is unchanged
            (true, true, true, true, true, true),
            (false, true, true, false, false, false),
        ] {
            let target = ScriptTarget {
                object: if in_object { Uuid::from_u128(9) } else { Uuid::nil() },
                item: Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            editors.open(target, "Script".into(), readable, editable, 0);
            let e = &mut editors.documents[0];
            e.can_reset = available;
            if saving {
                e.saving = Some(super::super::PendingSave {
                    request: Uuid::from_u128(11),
                    source: String::new(),
                    compile: aurora_net::scripts::CompileTarget::Mono,
                    experience: None,
                    running: false,
                });
            }
            assert!(e.running.is_none(), "the reply has not come");
            let (save, reset, running) = run_state(e);
            assert_eq!(
                reset, expected,
                "readable={readable}, editable={editable}, available={available}, saving={saving}"
            );
            assert_eq!(running, None, "no running state to flip without the reply");
            assert!(!save, "nothing to save");
            assert!(e.source.is_empty());
        }
    }

    #[test]
    fn commands_enforce_permissions_and_processed_selection_does_not_replace_the_source_cursor() {
        let target = aurora_net::scripts::ScriptTarget {
            object: uuid::Uuid::nil(),
            item: uuid::Uuid::from_u128(10),
        };
        let mut editors = super::super::Editors::default();
        let request = editors.open(target, "Main".into(), true, false, 0).unwrap();
        editors.loaded(request, target, Ok("éあ original".into()));
        let e = &mut editors.documents[0];
        let ctx = egui::Context::default();
        let id = Id::unique("commands");
        let settings = ScriptSettings::default();
        let mut prefs = Panels::default();
        let mut actions = vec![];
        editing::select(&ctx, id, 0..2, false);
        assert!(enabled(Command::Copy, e, &ctx, id));
        assert!(!enabled(Command::Cut, e, &ctx, id));
        assert!(!enabled(Command::Export, e, &ctx, id), "a read-only script saves no file");
        run(Command::Cut, e, &ctx, id, &settings, &mut prefs, &mut actions);
        run(Command::Paste, e, &ctx, id, &settings, &mut prefs, &mut actions);
        assert_eq!(e.source, "éあ original");
        assert!(actions.is_empty());
        e.can_edit = true;
        assert!(enabled(Command::Export, e, &ctx, id), "an editable script saves to a file");
        e.tools.processed = true;
        e.tools.processed_source = "generated source with different length".into();
        run(Command::SelectAll, e, &ctx, id, &settings, &mut prefs, &mut actions);
        assert_eq!(
            editing::cursor(&ctx, visible_id(id, true)).as_sorted_char_range().end.0,
            e.tools.processed_source.chars().count()
        );
        assert_eq!(editing::cursor(&ctx, id).as_sorted_char_range().end.0, 2);
        assert!(!enabled(Command::Paste, e, &ctx, id));
        assert!(!enabled(Command::Cut, e, &ctx, id));
        assert_eq!(e.source, "éあ original");
    }

    #[test]
    fn a_drag_selection_held_past_the_view_scrolls_the_source_like_the_legacy_viewer() {
        use crate::theme::Theme;
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;

        let original: String = (0..300).map(|i| format!("llOwnerSay(\"line {i}\");\n")).collect();
        let ctx = egui::Context::default();
        let theme = Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let target = ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(10),
        };
        let mut editors = super::super::Editors::default();
        let request = editors.open(target, "Drag".into(), true, true, 0).expect("load");
        editors.loaded(request, target, Ok(original.clone()));
        let e = &mut editors.documents[0];
        e.tools.reset_history = false;
        e.tools.focus_start = false;
        let id = Id::unique("drag_autoscroll_source");
        let settings = ScriptSettings::default();
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>, e: &mut Editor| {
            time += 1.0 / 60.0;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 400.0))),
                    focused: true,
                    time: Some(time),
                    predicted_dt: 1.0 / 60.0,
                    events,
                    ..Default::default()
                },
                |ui| source(ui, &p, e, id, &settings, 200.0, &Search::default(), &[]),
            )
            .textures_delta
            .clear();
        };
        let line = |ctx: &egui::Context| editing::position(&original, editing::cursor(ctx, id).primary.index.0).0;
        for _ in 0..3 {
            frame(Vec::new(), e);
        }
        let start = egui::pos2(150.0, 20.0);
        frame(vec![egui::Event::PointerMoved(start)], e);
        let press = |pressed| egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![press(true)], e);
        let anchor = editing::cursor(&ctx, id).primary.index;
        // Inside the view, the selection follows the pointer and nothing scrolls.
        frame(vec![egui::Event::PointerMoved(egui::pos2(150.0, 100.0))], e);
        let inside = line(&ctx);
        for _ in 0..10 {
            frame(Vec::new(), e);
        }
        assert_eq!(line(&ctx), inside, "no scroll away from the edges");
        // Below the view, held still: the source scrolls and the selection grows.
        frame(vec![egui::Event::PointerMoved(egui::pos2(150.0, 300.0))], e);
        let past = line(&ctx);
        for _ in 0..60 {
            frame(Vec::new(), e);
        }
        let after_one_second = line(&ctx);
        assert!(after_one_second > past + 3, "{past} -> {after_one_second}");
        for _ in 0..600 {
            frame(Vec::new(), e);
        }
        assert_eq!(line(&ctx), 300, "the selection reaches the end of the script");
        // Back above the view: it scrolls up again, still selecting from the start.
        frame(vec![egui::Event::PointerMoved(egui::pos2(150.0, -20.0))], e);
        let top = line(&ctx);
        for _ in 0..60 {
            frame(Vec::new(), e);
        }
        assert!(line(&ctx) < top, "{top} -> {}", line(&ctx));
        frame(vec![press(false)], e);
        let selection = editing::cursor(&ctx, id);
        assert_eq!(selection.secondary.index, anchor, "the selection still starts where the drag began");
        assert_eq!(e.source, original);
    }
}
