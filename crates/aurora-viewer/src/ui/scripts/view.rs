//! LLScriptEdCore menus / button bar and LLScriptEditor gutter (Firestorm,
//! LGPL 2.1). Aurora keeps its flat widgets and always-visible experience list.

use super::{
    Action, Editor,
    dialogs::{Help, Panels, Search},
    editing, files, indent, reference,
    settings::{ColorRole, ScriptSettings},
};
use crate::{
    theme::Palette,
    ui::{
        menu,
        widgets::{Floater, flat_button, icon_button},
    },
};
use egui::{Id, RichText, Vec2};

#[derive(Default)]
pub(super) struct Tools {
    pub focus_start: bool,
    pub body_top: f32,
    pub search_slot: Option<egui::Ui>,
    header_width: f32,
    header_height: f32,
    pub help: Help,
    pub insert_filter: String,
    pub insert_button: Option<egui::Rect>,
    insertion_open: bool,
    pub library: bool,
    pub goto: bool,
    pub goto_line: usize,
    pub processed: bool,
    pub processed_source: String,
    pub has_processed: bool,
    pub position: (usize, usize),
    pub scroll_to_cursor: bool,
    pub revert: bool,
    pub file_job: Option<files::Job>,
    pub file_snapshot: String,
    pub external: Option<files::Watch>,
    pub external_snapshot: String,
    pub incoming: Option<String>,
    incoming_external: bool,
    pub diagnostics_processed: bool,
    pub reset_history: bool,
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

#[derive(Clone, Copy, Default)]
enum Command {
    #[default]
    Save,
    Reload,
    Revert,
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
    Help,
    Keyword,
    Library,
    Debug,
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
        Command::Export => e.loaded && (e.can_read || !e.source.is_empty()) && e.tools.file_job.is_none(),
        Command::Cut => editable && selection,
        Command::Copy => e.loaded && selection,
        Command::Paste => editable,
        Command::Undo => editable && e.tools.can_undo,
        Command::Redo => editable && e.tools.can_redo,
        Command::Indent | Command::Outdent | Command::LineComment | Command::BlockComment => editable,
        Command::FoldAll | Command::ExpandAll => e.loaded,
        Command::SelectAll | Command::Deselect | Command::Find | Command::Goto | Command::Keyword => e.loaded,
        _ => true,
    }
}

fn run(
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
        Command::Export => e.tools.file_job = Some(files::save(e.name.clone(), e.source.clone())),
        Command::Preferences => prefs.preferences.open = true,
        Command::External => {
            e.tools.file_snapshot.clone_from(&e.source);
            e.tools.file_job = Some(files::external(
                e.name.clone(),
                e.source.clone(),
                settings.external_editor.clone(),
                settings.convert_tabs,
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
        Command::Help => ctx.open_url(egui::OpenUrl::new_tab(reference::url(""))),
        Command::Keyword => {
            e.tools.help.open = true;
            e.tools.help.visit(editing::word(
                if e.tools.processed { &e.tools.processed_source } else { &e.source },
                editing::cursor(ctx, id).primary.index.0,
            ));
        }
        Command::Library => e.tools.library = true,
        Command::Debug => actions.push(Action::Debug),
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
            e.folding
                .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source });
            e.folding.collapse_all();
            editing::select(ctx, id, 0..0, false);
        }
        Command::ExpandAll => e.folding.expand_all(),
    }
}

pub(super) fn body(
    ui: &mut egui::Ui,
    p: &Palette,
    e: &mut Editor,
    id: Id,
    settings: &ScriptSettings,
    prefs: &mut Panels,
    actions: &mut Vec<Action>,
) {
    let ctx = ui.ctx().clone();
    e.tools.body_top = ui.cursor().top();
    e.tools.search_slot = None;
    let gap = ui.spacing().item_spacing.x;
    let search_width = Search::inline_width(ui);
    let inline = settings.search_inline
        && prefs.search.owns(id)
        && e.tools.header_width > 0.0
        && e.tools.header_height >= Search::inline_min_height(ui)
        && ui.available_width() >= e.tools.header_width + gap + search_width;
    if inline {
        let left = ui.available_width() - gap - search_width;
        ui.horizontal_top(|ui| {
            let header = ui.allocate_ui_with_layout(Vec2::new(left, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                header(ui, p, e, id, settings, prefs, actions);
            });
            e.tools.header_height = header.response.rect.height();
            let (rect, _) = ui.allocate_exact_size(Vec2::new(search_width, e.tools.header_height), egui::Sense::hover());
            if !ui.is_sizing_pass() && ui.is_enabled() {
                // Render the shared controls after all editor commands have
                // selected their owner, using this editor's layer and style.
                e.tools.search_slot = Some(
                    ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt("inline_search")
                            .max_rect(rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    ),
                );
            }
        });
    } else {
        let header = ui.scope(|ui| header(ui, p, e, id, settings, prefs, actions));
        e.tools.header_height = header.response.rect.height();
    }
    if e.loaded && !e.can_edit {
        ui.label(RichText::new(aurora_i18n::tr!("script-read-only")).color(p.warn));
    }
    if settings.preprocessor || e.tools.has_processed || e.tools.processed {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut e.tools.processed, false, aurora_i18n::tr!("script-tab-source"));
            ui.selectable_value(&mut e.tools.processed, true, aurora_i18n::tr!("script-tab-processed"));
        });
    } else {
        e.tools.processed = false;
    }
    // Measure the actual controls, including object-only rows and diagnostics.
    // Repaint after a width/status change, without replaying asynchronous actions.
    let footer_height = if e.tools.footer_height > 0.0 {
        e.tools.footer_height
    } else {
        155.0
    };
    let height = (ui.available_height() - footer_height - ui.spacing().item_spacing.y - 8.0).max(120.0);
    source(ui, p, e, id, settings, height, &prefs.search);
    let footer = ui.scope(|ui| footer(ui, p, e, id, actions));
    if (e.tools.footer_height - footer.response.rect.height()).abs() > 0.5 {
        e.tools.footer_height = footer.response.rect.height();
        ctx.request_repaint();
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
) {
    let ctx = ui.ctx().clone();
    let mut chosen = None;
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button(aurora_i18n::tr!("script-menu-file"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Save, "device-floppy-filled", aurora_i18n::tr!("common-save")),
                (Command::Revert, "restore", aurora_i18n::tr!("script-menu-revert")),
                (Command::Reload, "refresh", aurora_i18n::tr!("build-ui-contents-refresh")),
                (Command::Open, "folder-open-filled", aurora_i18n::tr!("script-menu-open")),
                (Command::Export, "download-filled", aurora_i18n::tr!("script-menu-export")),
                (Command::External, "external-link-filled", aurora_i18n::tr!("script-menu-external")),
                (Command::Preferences, "settings-filled", aurora_i18n::tr!("script-menu-preferences")),
            ] {
                if menu::item_if(ui, p, icon, label, enabled(command, e, &ctx, id)) {
                    chosen = Some(command);
                }
            }
        });
        ui.menu_button(aurora_i18n::tr!("script-menu-edit"), |ui| {
            edit_menu(ui, p, e, id, &mut chosen);
        });
        ui.menu_button(aurora_i18n::tr!("script-menu-help"), |ui| {
            super::focus::claim(&ctx, ui.layer_id(), id);
            for (command, icon, label) in [
                (Command::Help, "help-circle-filled", aurora_i18n::tr!("script-menu-help")),
                (Command::Keyword, "book-filled", aurora_i18n::tr!("script-help-keyword")),
                (Command::Library, "library-filled", aurora_i18n::tr!("script-help-library")),
                (Command::Debug, "bug-filled", aurora_i18n::tr!("script-debug-title")),
            ] {
                if menu::item_if(ui, p, icon, label, enabled(command, e, &ctx, id)) {
                    chosen = Some(command);
                }
            }
        });
    });
    ui.horizontal(|ui| {
        let left = ui.cursor().left();
        for (command, icon, label) in [
            (Command::Save, "device-floppy-filled", aurora_i18n::tr!("common-save")),
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
        ] {
            if icon_button(ui, p, icon, label, enabled(command, e, &ctx, id)) {
                chosen = Some(command);
            }
        }
        if e.saving.is_some() {
            ui.spinner();
        } else {
            crate::ui::widgets::status_dot(
                ui,
                if e.compile_failed {
                    p.danger
                } else if e.running == Some(true) {
                    p.success
                } else {
                    p.amber
                },
            );
        }
        e.tools.header_width = ui.cursor().left() - left;
    });
    if let Some(command) = chosen {
        run(command, e, &ctx, id, settings, prefs, actions);
    }
    if ctx.memory(|m| m.has_focus(visible_id(id, e.tools.processed))) {
        for (key, command) in [
            (egui::Key::S, Command::Save),
            (egui::Key::G, Command::Goto),
            (egui::Key::O, Command::Open),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                run(command, e, &ctx, id, settings, prefs, actions);
            }
        }
    }
}

fn footer(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        insertion(ui, p, e, id);
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "script-position",
                line = e.tools.position.0 as i64,
                column = e.tools.position.1 as i64
            ))
            .small()
            .color(p.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled_ui(e.can_save(), |ui| flat_button(ui, p, aurora_i18n::tr!("common-save")))
                .inner
                .clicked()
            {
                actions.extend(e.save());
            }
        });
    });
    ui.horizontal(|ui| {
        if !e.target.object.is_nil() {
            ui.add_enabled_ui(e.can_edit && e.running.is_some() && e.saving.is_none(), |ui| {
                if flat_button(ui, p, aurora_i18n::tr!("script-reset")).clicked() {
                    actions.push(Action::Reset(e.target));
                }
                let mut running = e.running.unwrap_or(false);
                if crate::ui::widgets::checkbox(ui, p, &mut running, aurora_i18n::tr!("script-running")).changed() {
                    actions.push(Action::Running { target: e.target, running });
                }
            });
        }
        ui.add_enabled_ui(
            e.can_edit && e.loaded && e.saving.is_none() && (e.target.object.is_nil() || e.running.is_some()),
            |ui| {
                crate::ui::widgets::checkbox(ui, p, &mut e.mono, "Mono").on_hover_text(aurora_i18n::tr!("script-mono-hint"));
            },
        );
    });
    if !e.target.object.is_nil()
        && e.experience.show(ui, p, e.can_edit && e.loaded && e.saving.is_none())
        && let Some(request) = e.experience.begin()
    {
        actions.push(Action::Experiences { request, target: e.target });
    }
    if !e.target.object.is_nil() && e.loaded && e.can_read && e.running.is_none() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("script-waiting-running")).color(p.warn));
            if flat_button(ui, p, aurora_i18n::tr!("script-retry")).clicked() {
                actions.push(Action::QueryRunning(e.target));
            }
        });
    }
    if e.source.len() > aurora_net::scripts::MAX_SOURCE {
        ui.label(RichText::new(aurora_i18n::tr!("script-source-too-large")).color(p.danger));
    }
    ui.label(RichText::new(&e.status).color(if e.compile_failed { p.danger } else { p.muted }));
    egui::ScrollArea::vertical()
        .id_salt((id, "errors"))
        .max_height(65.0)
        .show(ui, |ui| {
            for error in &e.errors {
                let location = editing::error_location(error);
                if ui
                    .selectable_label(false, RichText::new(error).monospace().color(p.danger))
                    .clicked()
                    && let Some((line, column)) = location
                {
                    e.tools.processed = e.tools.diagnostics_processed && !e.tools.processed_source.is_empty();
                    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
                    let at = editing::line_cursor(text, line, column);
                    editing::select(&ctx, visible_id(id, e.tools.processed), at..at, true);
                    e.tools.scroll_to_cursor = true;
                }
            }
        });
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
        if menu::item_if(ui, p, icon, label, enabled(command, e, ui.ctx(), id)) {
            *chosen = Some(command);
        }
    }
}

fn source(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id, settings: &ScriptSettings, height: f32, search: &Search) {
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
    e.folding
        .sync(if e.tools.processed { &e.tools.processed_source } else { &e.source });
    e.folding.reveal(editing::cursor(ui.ctx(), id).primary);
    egui::Frame::new()
        .fill(settings.color(ColorRole::Background, p))
        .inner_margin(4.0)
        .show(ui, |ui| {
            ui.scope(|ui| {
                ui.visuals_mut().selection.bg_fill = settings.color(ColorRole::Selection, p);
                ui.visuals_mut().text_cursor.stroke.color = settings.color(ColorRole::Cursor, p);
                let font = egui::FontId::monospace(settings.font_size.clamp(8.0, 32.0));
                let query = if search.owns(base_id) {
                    search.query.clone()
                } else {
                    String::new()
                };
                let insensitive = search.insensitive;
                let color_editable = e.loaded && e.can_edit && e.saving.is_none() && e.loading.is_none() && !e.tools.processed;
                let editable = color_editable && !e.colors.active();
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
                            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                                let mut job = e.highlighter.custom(text.as_str(), p, font.clone(), settings, &query, insensitive);
                                // panel_script_ed.xml enables wrapping; only the visual rows change.
                                job.wrap.max_width = if settings.word_wrap { wrap } else { f32::INFINITY };
                                e.folding.sync(text.as_str());
                                e.folding.layout(&mut job);
                                ui.fonts_mut(|f| f.layout_job(job))
                            };
                            let mut show = |ui: &mut egui::Ui, buffer: &mut dyn egui::TextBuffer| {
                                egui::TextEdit::multiline(buffer)
                                    .id(id)
                                    .code_editor()
                                    .layouter(&mut layouter)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(1)
                                    .min_size(Vec2::new(0.0, viewport.height()))
                                    .frame(egui::Frame::NONE)
                                    .interactive(interactive)
                                    .show(ui)
                            };
                            let output = if editable {
                                indent::edit(ui, id, &mut e.source, show)
                            } else {
                                let mut text = if e.tools.processed {
                                    e.tools.processed_source.as_str()
                                } else {
                                    e.source.as_str()
                                };
                                show(ui, &mut text)
                            };
                            let mut chosen = None;
                            menu::context_menu(&output.response, p, |ui| edit_menu(ui, p, e, base_id, &mut chosen));
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
                            let left = gutter.left() + viewport.min.x;
                            let clip = egui::Rect::from_min_max(
                                egui::pos2(left, ui.clip_rect().top()),
                                egui::pos2(left + 56.0, ui.clip_rect().bottom()),
                            );
                            let painter = ui.painter().with_clip_rect(clip);
                            painter.rect_filled(clip, 0.0, settings.color(ColorRole::Background, p));
                            let mut line = 0;
                            let mut line_start = true;
                            for row in &output.galley.rows {
                                let y = output.galley_pos.y + row.pos.y;
                                if y > clip.bottom() {
                                    break;
                                }
                                if line_start && row.height() >= 1.0 && y + row.height() >= clip.top() {
                                    painter.text(
                                        egui::pos2(left + 50.0, y),
                                        egui::Align2::RIGHT_TOP,
                                        line.to_string(),
                                        font.clone(),
                                        if line == e.tools.position.0 {
                                            settings.color(ColorRole::Cursor, p)
                                        } else {
                                            p.muted
                                        },
                                    );
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
                            if e.folding.reveal(output.cursor_range.unwrap_or_default().primary) {
                                ui.ctx().request_repaint();
                            }
                            e.folding.draw(ui, p, id, &output, left);
                            if let Some(pos) = output.response.hover_pos() {
                                let at = output.galley.cursor_from_pos(pos - output.galley_pos).index.0;
                                if let Some(entry) = reference::get(editing::word(text, at)) {
                                    (*output.response)
                                        .clone()
                                        .on_hover_text(format!("{}\n{}", entry.signature, entry.tooltip));
                                }
                            }
                        });
                    });
            });
        });
}

fn insertion(ui: &mut egui::Ui, p: &Palette, e: &mut Editor, id: Id) {
    let mut chosen = None;
    ui.add_enabled_ui(
        e.loaded && e.can_edit && e.saving.is_none() && !e.tools.processed && !e.colors.active(),
        |ui| {
            let popup = egui::ComboBox::from_id_salt((id, "insert"))
                .selected_text(aurora_i18n::tr!("script-insert"))
                .width(155.0)
                .height(280.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    super::focus::claim(ui.ctx(), ui.layer_id(), id);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut e.tools.insert_filter)
                            .id(id.with("insert_filter"))
                            .desired_width(260.0)
                            .hint_text(aurora_i18n::tr!("script-insert-filter")),
                    );
                    if !e.tools.insertion_open {
                        field.request_focus();
                    }
                    crate::ui::widgets::divider(ui, p);
                    // Only the results scroll, keeping the search field visible.
                    let mut results = egui::ScrollArea::vertical().id_salt((id, "insert_results")).max_height(220.0);
                    if field.changed() {
                        results = results.vertical_scroll_offset(0.0);
                    }
                    let filter = e.tools.insert_filter.trim().to_lowercase();
                    results.show(ui, |ui| {
                        let mut found = false;
                        for entry in reference::all()
                            .iter()
                            .filter(|e| e.group == "functions" && e.name.to_lowercase().contains(&filter))
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
                });
            e.tools.insert_button = Some(popup.response.rect);
            e.tools.insertion_open = popup.inner.is_some();
        },
    );
    if let Some(name) = chosen {
        let [a, b] = editing::cursor(ui.ctx(), id).sorted_cursors();
        if editing::replace(ui.ctx(), id, &mut e.source, a.index.0..b.index.0, &name) {
            ui.ctx().memory_mut(|m| m.request_focus(id));
            e.tools.scroll_to_cursor = true;
            e.tools.help.visit(&name);
        } else {
            e.status = aurora_i18n::tr!("script-source-too-large").into();
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
    poll_files(ctx, e, id, settings, actions);
    if let Some(command) = ctx.data_mut(|d| d.remove_temp::<Command>(Id::unique((id, "command")))) {
        run(command, e, ctx, id, settings, prefs, actions);
    }
    let original_id = id;
    let id = visible_id(id, e.tools.processed);
    if e.tools.help.show(
        ctx,
        p,
        id,
        if e.tools.processed { &e.tools.processed_source } else { &e.source },
        settings,
    ) {
        actions.push(Action::SettingsChanged);
    }
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
                ui.label(aurora_i18n::tr!("script-goto-line"));
                let input = ui.add(egui::DragValue::new(&mut e.tools.goto_line).range(0..=usize::MAX));
                if flat_button(ui, p, aurora_i18n::tr!("script-goto-go")).clicked()
                    || (input.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
                {
                    let text = if e.tools.processed { &e.tools.processed_source } else { &e.source };
                    let at = editing::line_cursor(text, e.tools.goto_line, 0);
                    editing::select(ctx, id, at..at, true);
                    e.tools.goto = false;
                    e.tools.scroll_to_cursor = true;
                }
            });
        });
        e.tools.goto &= open;
    }
    if e.tools.library {
        Floater::new(
            &format!("{id:?}_library"),
            aurora_i18n::tr!("script-help-library"),
            ctx.content_rect().center() - Vec2::new(280.0, 240.0),
            Vec2::new(560.0, 480.0),
        )
        .show(ctx, p, &mut e.tools.library, |ui| {
            ui.text_edit_singleline(&mut e.tools.insert_filter);
            let filter = e.tools.insert_filter.to_lowercase();
            let mut keyword = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in reference::all().iter().filter(|e| e.name.to_lowercase().contains(&filter)) {
                    if ui
                        .selectable_label(false, RichText::new(&entry.signature).monospace())
                        .on_hover_text(&entry.tooltip)
                        .clicked()
                    {
                        keyword = Some(entry.name.clone());
                    }
                }
            });
            if let Some(name) = keyword {
                e.tools.help.keyword = name;
                e.tools.help.open = true;
            }
        });
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
                    e.mono = e.baseline_mono;
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
    if let Some(mut ui) = e.tools.search_slot.take() {
        if search.focus {
            ctx.move_to_top(ui.layer_id());
        }
        search.controls(
            &mut ui,
            p,
            id,
            if e.tools.processed {
                &mut e.tools.processed_source
            } else {
                &mut e.source
            },
            editable,
            true,
        );
        e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
        return;
    }
    let (pos, overlap) = Search::placement(ctx, parent, e.tools.body_top, prefer_left);
    if overlap {
        let owner = egui::LayerId::new(egui::Order::Middle, parent_id);
        let child = egui::LayerId::new(egui::Order::Middle, Id::unique(Search::WINDOW));
        // Dependent ordering, as LLFloater::addDependentFloater: clicking
        // the script keeps its search above it without covering other windows.
        ctx.set_sublayer(owner, child);
        let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
        if search.focus || pressed.is_some_and(|at| ctx.layer_id_at(at) == Some(child)) || ctx.is_being_dragged(child.id.with("move")) {
            ctx.move_to_top(owner);
        }
    }
    search.show(
        ctx,
        p,
        id,
        if e.tools.processed {
            &mut e.tools.processed_source
        } else {
            &mut e.source
        },
        editable,
        pos,
    );
    e.tools.scroll_to_cursor |= cursor != editing::cursor(ctx, id);
}

fn poll_files(ctx: &egui::Context, e: &mut Editor, id: Id, settings: &ScriptSettings, actions: &mut Vec<Action>) {
    if let Some(done) = e.tools.file_job.as_ref().and_then(|j| j.0.try_recv().ok()) {
        e.tools.file_job = None;
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

    #[test]
    fn word_wrap_reflows_without_changing_source_selection_or_line_numbers() {
        use crate::theme::Theme;
        use aurora_net::scripts::ScriptTarget;
        use uuid::Uuid;

        let original = format!(
            "// {}\nstring value = \"{}\";\nllOwnerSay(value);",
            "é long comment ".repeat(12),
            "x".repeat(140)
        );
        for (object, editable, processed) in [(false, true, false), (true, true, false), (false, false, false), (true, true, true)] {
            let ctx = egui::Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let target = ScriptTarget {
                object: if object { Uuid::from_u128(9) } else { Uuid::nil() },
                item: Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            let request = editors.open(target, "Wrapping".into(), true, editable).expect("load");
            editors.loaded(request, target, Ok(original.clone()));
            let e = &mut editors.documents[0];
            e.tools.reset_history = false;
            e.tools.focus_start = false;
            e.tools.processed = processed;
            e.tools.processed_source.clone_from(&original);
            let id = Id::unique("wrapping_source");
            let text_id = visible_id(id, processed);
            editing::select(&ctx, text_id, 3..12, true);
            let selection = editing::cursor(&ctx, text_id);
            let mut narrow_rows = 0;
            for (wrap, width) in [(false, 320.0), (true, 320.0), (true, 540.0), (false, 320.0), (true, 320.0)] {
                let settings = ScriptSettings {
                    word_wrap: wrap,
                    ..Default::default()
                };
                let mut frame = || {
                    ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(width, 900.0))),
                            focused: true,
                            ..Default::default()
                        },
                        |ui| source(ui, &p, e, id, &settings, 800.0, &Search::default()),
                    )
                };
                for _ in 0..2 {
                    frame().textures_delta.clear();
                }
                let mut output = frame();
                let texts: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) => Some(text),
                        _ => None,
                    })
                    .collect();
                let galley = &texts
                    .iter()
                    .find(|text| text.galley.job.text == original)
                    .expect("source galley")
                    .galley;
                if wrap {
                    assert!(galley.size().x <= width - 64.0, "wrapped width: {:?} / {width}", galley.size());
                    assert!(galley.rows.len() > 3);
                    if width == 320.0 {
                        narrow_rows = galley.rows.len();
                    } else {
                        assert!(galley.rows.len() < narrow_rows, "resizing must reflow the source");
                    }
                } else {
                    assert_eq!(galley.rows.len(), 3);
                    assert!(galley.size().x > width);
                }
                let numbers: Vec<_> = texts.iter().filter_map(|text| text.galley.job.text.parse::<usize>().ok()).collect();
                assert_eq!(numbers, [0, 1, 2], "one number per source line, never on a continuation");
                assert_eq!(
                    editing::cursor(&ctx, text_id),
                    selection,
                    "wrap={wrap}, width={width}, editable={editable}, processed={processed}"
                );
                assert_eq!(e.source, original);
                assert!(!e.dirty());
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn resizing_fills_the_editor_without_painting_the_gutter_over_the_footer() {
        use crate::theme::Theme;
        for object in [false, true] {
            let ctx = egui::Context::default();
            let theme = Theme::default();
            theme.apply(&ctx, 1.0);
            let p = theme.palette();
            let target = aurora_net::scripts::ScriptTarget {
                object: if object { uuid::Uuid::from_u128(9) } else { uuid::Uuid::nil() },
                item: uuid::Uuid::from_u128(10),
            };
            let mut editors = super::super::Editors::default();
            let request = editors.open(target, "Resize".into(), true, true).unwrap();
            editors.loaded(request, target, Ok("default {\n    event() {\n        code;\n    }\n}".into()));
            let e = &mut editors.documents[0];
            let settings = ScriptSettings::default();
            let mut prefs = Panels::default();
            let short_source = "default {\n    event() {\n        code;\n    }\n}";
            let long_source = format!("// {}\n{short_source}", "long line ".repeat(100));
            for size in [Vec2::new(660.0, 440.0), Vec2::new(1100.0, 880.0), Vec2::new(520.0, 380.0)] {
                for (source, folded) in [(short_source, false), (short_source, true), (long_source.as_str(), false)] {
                    e.source = source.into();
                    e.folding.sync(&e.source);
                    if folded {
                        e.folding.collapse_all();
                    } else {
                        e.folding.expand_all();
                    }
                    let mut bounds = egui::Rect::NOTHING;
                    let mut bottom = 0.0;
                    let mut frame = || {
                        ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                                ..Default::default()
                            },
                            |ui| {
                                bounds = ui.available_rect_before_wrap();
                                body(ui, &p, e, Id::unique("resize_source"), &settings, &mut prefs, &mut vec![]);
                                bottom = ui.min_rect().bottom();
                            },
                        )
                    };
                    // read_response exposes the previous frame's widget geometry.
                    for _ in 0..2 {
                        let mut warmup = frame();
                        warmup.textures_delta.clear();
                    }
                    let mut output = frame();
                    let background = settings.color(ColorRole::Background, &p);
                    let rectangles: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Rect(rect) if rect.fill == background => Some(rect.rect),
                            _ => None,
                        })
                        .collect();
                    let editor = rectangles
                        .iter()
                        .max_by(|a, b| a.area().total_cmp(&b.area()))
                        .expect("editor background");
                    let gutter = rectangles.iter().find(|rect| (rect.width() - 56.0).abs() < 0.5).expect("gutter");
                    let input = ctx.read_response(Id::unique("resize_source")).expect("source input");
                    assert!(
                        input.rect.height() >= gutter.height() - 0.5,
                        "clickable editor: {:?} / {editor:?}",
                        input.rect
                    );
                    assert!(editor.height() > if size.y > 800.0 { 600.0 } else { 120.0 }, "{editor:?}");
                    assert!(
                        (4.0..25.0).contains(&(editor.bottom() - gutter.bottom())),
                        "{editor:?} / {gutter:?}"
                    );
                    assert!(
                        (bounds.bottom() - bottom).abs() < 1.0,
                        "unused/overflowing space: {bounds:?} / {bottom}"
                    );
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn commands_enforce_permissions_and_processed_selection_does_not_replace_the_source_cursor() {
        let target = aurora_net::scripts::ScriptTarget {
            object: uuid::Uuid::nil(),
            item: uuid::Uuid::from_u128(10),
        };
        let mut editors = super::super::Editors::default();
        let request = editors.open(target, "Main".into(), true, false).unwrap();
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
        run(Command::Cut, e, &ctx, id, &settings, &mut prefs, &mut actions);
        run(Command::Paste, e, &ctx, id, &settings, &mut prefs, &mut actions);
        assert_eq!(e.source, "éあ original");
        assert!(actions.is_empty());
        e.can_edit = true;
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
}
