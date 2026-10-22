//! Shared inventory / object script editors (LLPreviewLSL,
//! LLLiveLSLEditor and LLScriptEdCore, Firestorm indra/newview,
//! originally LGPL 2.1). Each document owns its pending requests and
//! saved snapshot, so late replies cannot replace another editor's text.

use super::widgets::{Floater, flat_button};
use crate::theme::Palette;
use aurora_net::scripts::{MAX_SOURCE, ScriptOptions, ScriptSaved, ScriptTarget};
use egui::Vec2;
use uuid::Uuid;

mod colors;
mod debug;
mod dialogs;
mod editing;
mod experiences;
mod files;
mod focus;
mod folding;
mod indent;
mod preprocessor;
mod reference;
pub mod settings;
mod syntax;
mod view;
pub use focus::editor_focused;
pub(crate) use indent::edit_scoped as edit_code;
pub(crate) use syntax::highlight as highlight_lsl;

pub enum Action {
    Debug,
    Include {
        request: Uuid,
        name: String,
    },
    Paste(ScriptTarget),
    SettingsChanged,
    Save {
        request: Uuid,
        target: ScriptTarget,
        options: ScriptOptions,
        source: String,
    },
    Reload {
        request: Uuid,
        target: ScriptTarget,
    },
    Experiences {
        request: Uuid,
        target: ScriptTarget,
    },
    Running {
        target: ScriptTarget,
        running: bool,
    },
    Reset(ScriptTarget),
    QueryRunning(ScriptTarget),
}

struct PendingSave {
    request: Uuid,
    source: String,
    mono: bool,
    experience: Option<Uuid>,
    running: bool,
}

pub struct Editor {
    pub target: ScriptTarget,
    pub name: String,
    pub source: String,
    highlighter: syntax::Highlighter,
    colors: colors::State,
    folding: folding::State,
    baseline: String,
    loaded: bool,
    loading: Option<Uuid>,
    saving: Option<PendingSave>,
    pub can_read: bool,
    pub can_edit: bool,
    pub mono: bool,
    baseline_mono: bool,
    experience: experiences::State,
    running: Option<bool>,
    status: String,
    errors: Vec<String>,
    compile_failed: bool,
    close_prompt: bool,
    close_after_save: bool,
    closed: bool,
    raise: bool,
    tools: view::Tools,
    recompile_allowed: bool,
    processor_settings: settings::ScriptSettings,
    processor_environment: preprocessor::Environment,
    processing: Option<preprocessor::Job>,
}

impl Editor {
    fn dirty(&self) -> bool {
        self.loaded && (self.source != self.baseline || self.mono != self.baseline_mono || self.experience.dirty())
    }

    fn can_save(&self) -> bool {
        self.loaded
            && self.can_edit
            && !self.colors.active()
            && self.saving.is_none()
            && self.loading.is_none()
            && (self.target.object.is_nil()
                || (self.running.is_some() && self.experience.selected.is_some() && self.experience.loading.is_none()))
            && (self.dirty() || self.compile_failed || self.recompile_allowed)
            && self.source.len() <= MAX_SOURCE
    }

    fn save(&mut self) -> Option<Action> {
        if !self.can_save() {
            return None;
        }
        let request = Uuid::new_v4();
        self.saving = Some(PendingSave {
            request,
            source: self.source.clone(),
            mono: self.mono,
            experience: self.experience.selected,
            running: self.running.unwrap_or(false),
        });
        self.status = aurora_i18n::tr!("script-saving").into();
        self.errors.clear();
        self.tools.diagnostics_processed = false;
        if self.processor_settings.preprocessor {
            self.status = aurora_i18n::tr!("script-preproc-processing").into();
            self.processing = Some(preprocessor::start(
                self.source.clone(),
                self.name.clone(),
                self.processor_settings.clone(),
                self.processor_environment.clone(),
            ));
            return None;
        }
        Some(Action::Save {
            request,
            target: self.target,
            options: ScriptOptions {
                mono: self.mono,
                running: self.running.unwrap_or(false),
                experience: self.experience.selected.unwrap_or_default(),
            },
            source: self.source.clone(),
        })
    }
}

#[derive(Default)]
pub struct Editors {
    pub rlva_hidden: bool,
    pub documents: Vec<Editor>,
    panels: dialogs::Panels,
    debug: debug::Debug,
    includes: std::collections::HashMap<Uuid, PendingInclude>,
    pub agent: Uuid,
    pub agent_name: String,
}

struct PendingInclude {
    owner: ScriptTarget,
    asset: Uuid,
    reply: std::sync::mpsc::Sender<Result<preprocessor::Include, String>>,
}

impl Editors {
    pub fn show_debug(&mut self, ctx: &egui::Context, p: &Palette, messages: &mut crate::world::script_debug::Messages) {
        self.debug.show(ctx, p, messages);
    }
    pub fn open_debug(&mut self) {
        self.debug.open = true;
    }

    pub fn disconnected(&mut self) {
        self.includes.clear();
        self.documents.retain(|e| e.dirty() || e.saving.is_some());
        for e in &mut self.documents {
            e.colors.cancel(&mut e.source);
            e.loading = None;
            e.experience.loading = None;
            e.saving = None;
            e.processing = None;
            e.tools.external = None;
            e.can_read = false;
            e.can_edit = false;
            e.running = None;
            e.close_prompt = false;
            e.close_after_save = false;
            e.status = aurora_i18n::tr!("script-disconnected").into();
        }
    }

    pub fn prepare_exit(&mut self) -> bool {
        let mut ready = true;
        for e in &mut self.documents {
            e.colors.cancel(&mut e.source);
            if e.dirty() || e.saving.is_some() {
                e.close_prompt = true;
                ready = false;
            }
        }
        ready
    }

    pub fn demo_edit(&mut self, target: ScriptTarget, source: String, save: bool) -> Option<Action> {
        let e = self.documents.iter_mut().find(|e| e.target == target)?;
        e.source = source;
        if save { e.save() } else { None }
    }

    pub fn demo_experience(&mut self, target: ScriptTarget, experience: Uuid) -> Option<Action> {
        let e = self.documents.iter_mut().find(|e| e.target == target)?;
        e.experience.selected = Some(experience);
        e.save()
    }

    pub fn demo_indent(&mut self, target: ScriptTarget, ctx: &egui::Context, tabs: bool) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        if tabs {
            e.source = e.source.replace("    ", "\t");
        }
        let Some(line) = e
            .source
            .find("llSay(")
            .and_then(|start| e.source[start..].find(';').map(|end| start + end + 1))
        else {
            return;
        };
        Self::focus_source(e, ctx, line);
    }

    pub fn demo_tabstops(&mut self, target: ScriptTarget, ctx: &egui::Context) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        e.source = e.source.replacen("        llSay(", "       llSay(", 1);
        if let Some(position) = e.source.find("llSay(") {
            Self::focus_source(e, ctx, position);
        }
    }

    pub fn demo_close_brace(&mut self, target: ScriptTarget, ctx: &egui::Context, tabs: bool) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            e.source = "default\n{\n    state_entry()\n    {\n        llSetColor(<0.2, 0.8, 1.0>, ALL_SIDES);\n        ".into();
            if tabs {
                e.source = e.source.replace("    ", "\t");
            }
            Self::focus_source(e, ctx, e.source.len());
        }
    }

    pub fn demo_color_events(&self, frame: u64, mode: &str) -> Vec<egui::Event> {
        use super::color_picker::Control;
        let Some(e) = self.documents.first() else { return Vec::new() };
        let rect = match frame {
            323 | 324 => e.colors.swatches.first().copied(),
            343 | 344 => e.colors.picker.controls.get(&Control::Slot(20)).copied(),
            363 | 364 if mode == "color-cancel" => e.colors.picker.controls.get(&Control::Cancel).copied(),
            363 | 364 if mode == "color-accept" => e.colors.picker.controls.get(&Control::Accept).copied(),
            _ => None,
        };
        let Some(rect) = rect else { return Vec::new() };
        let pos = rect.center();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame % 2 == 1,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    pub fn demo_selection(&mut self, ctx: &egui::Context, outdent: bool) {
        if let Some(e) = self.documents.first_mut() {
            let Some(start) = e.source.find("integer count") else { return };
            let Some(end) = e.source.find("string greet") else { return };
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text"));
            editing::select(ctx, id, e.source[..start].chars().count()..e.source[..end].chars().count(), true);
            if outdent {
                editing::selection_command(ctx, id, &mut e.source, editing::SelectionCommand::Indent);
            }
        }
    }

    pub fn demo_insertion_events(&self, ctx: &egui::Context, frame: u64, mode: &str) -> Vec<egui::Event> {
        let Some(e) = self.documents.first() else { return Vec::new() };
        if matches!(frame, 323 | 324) {
            let Some(rect) = e.tools.insert_button else { return Vec::new() };
            let pos = rect.center();
            return vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 323,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
        }
        if (frame == 400 && mode == "insert-filter") || (frame == 326 && mode == "insert-empty") {
            return vec![egui::Event::Text(
                if mode == "insert-empty" { "no_matching_function" } else { "color" }.into(),
            )];
        }
        if frame == 350 && mode == "insert-filter" {
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text")).with("insert_filter");
            if let Some(field) = ctx.read_response(id) {
                return vec![
                    egui::Event::PointerMoved(field.rect.center_bottom() + Vec2::new(0.0, 90.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -240.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                    },
                ];
            }
        }
        Vec::new()
    }

    pub fn demo_folding_events(&self, frame: u64) -> Vec<egui::Event> {
        if !matches!(frame, 323 | 324) {
            return Vec::new();
        }
        let Some(e) = self.documents.first() else { return Vec::new() };
        let Some((_, rect)) = e.folding.controls.get(1) else {
            return Vec::new();
        };
        let pos = rect.center();
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: frame == 323,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    pub fn demo_tools(&mut self, target: ScriptTarget, ctx: &egui::Context, mode: &str, settings: &mut settings::ScriptSettings) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target) else {
            return;
        };
        let id = format!("script_editor_{}_{}", target.object, target.item);
        let text_id = egui::Id::unique((&id, "source_text"));
        match mode {
            "search" | "search-left" | "search-inline" | "search-overlay" | "search-outside" => {
                settings.search_prefer_left = mode == "search-left";
                settings.search_inline = mode == "search-inline";
                let geometry = match mode {
                    "search-left" => Some((egui::pos2(450.0, 130.0), Vec2::new(660.0, 650.0))),
                    "search-inline" => Some((egui::pos2(170.0, 130.0), Vec2::new(800.0, 700.0))),
                    "search-overlay" => Some((egui::pos2(140.0, 130.0), Vec2::new(1300.0, 700.0))),
                    "search-outside" => Some((egui::pos2(-380.0, 130.0), Vec2::new(660.0, 650.0))),
                    _ => None,
                };
                if let Some((pos, size)) = geometry {
                    super::widgets::place_floater(ctx, &id, pos, size);
                }
                self.panels.search.begin(ctx, text_id, text_id, &e.source);
                self.panels.search.query = "llSay".into();
                self.panels.search.replacement = "llOwnerSay".into();
                self.panels.search.find(ctx, text_id, &e.source);
            }
            "preferences" | "word-wrap-preferences" => self.panels.preferences.open = true,
            "help" => {
                e.tools.help.open = true;
                e.tools.help.visit("llSay");
            }
            "preprocessor" | "processed" | "object-preprocessor" | "preprocessor-error" => {
                settings.preprocessor = true;
                e.processor_settings.clone_from(settings);
                e.source = if mode == "preprocessor-error" {
                    "#if 1\n#error Demo preprocessing error\n#endif\n".into()
                } else {
                    "#define CHANNEL 0\n#include \"library.lsl\"\nlist values = [1, 2];\ndefault\n{\n    state_entry()\n    {\n        values[1] = 42;\n        switch ((integer)values[1])\n        {\n            case 42: greet(CHANNEL); break;\n            default: llOwnerSay(\"Unexpected value\");\n        }\n    }\n}\n".into()
                };
                e.save();
            }
            _ => {}
        }
    }

    pub fn demo_processed(&mut self, ctx: &egui::Context) {
        if let Some(e) = self.documents.first_mut() {
            e.tools.processed = true;
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&id, "source_text")).with("processed");
            let at = e
                .tools
                .processed_source
                .find("\n//nfo_preprocessor_version 0\n")
                .map(|i| i + "\n//nfo_preprocessor_version 0\n".len())
                .unwrap_or(0);
            let at = e.tools.processed_source[..at].chars().count();
            editing::select(ctx, id, at..at, false);
            e.tools.scroll_to_cursor = true;
        }
    }

    fn focus_source(e: &Editor, ctx: &egui::Context, byte: usize) {
        let cursor = egui::text::CCursor::new(e.source[..byte].chars().count());
        let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
        let text_id = egui::Id::unique((&id, "source_text"));
        if let Some(mut state) = egui::TextEdit::load_state(ctx, text_id) {
            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(cursor)));
            state.store(ctx, text_id);
            ctx.memory_mut(|m| m.request_focus(text_id));
        }
    }

    pub fn demo_close_clean(&mut self, target: ScriptTarget) -> bool {
        let Some(index) = self
            .documents
            .iter()
            .position(|e| e.target == target && e.loaded && !e.dirty() && e.saving.is_none())
        else {
            return false;
        };
        self.documents.remove(index);
        true
    }

    pub fn open(&mut self, target: ScriptTarget, name: String, can_read: bool, can_edit: bool) -> Option<Uuid> {
        for editor in &mut self.documents {
            editor.tools.focus_start = false;
        }
        if let Some(editor) = self.documents.iter_mut().find(|e| e.target == target) {
            editor.raise = true;
            editor.tools.focus_start = true;
            return None;
        }
        let request = can_read.then(Uuid::new_v4);
        self.documents.push(Editor {
            target,
            name,
            source: String::new(),
            highlighter: syntax::Highlighter::default(),
            colors: colors::State::new(format!("script_color_picker_{}_{}", target.object, target.item)),
            folding: folding::State::default(),
            baseline: String::new(),
            loaded: false,
            loading: request,
            saving: None,
            can_read,
            can_edit,
            mono: true,
            baseline_mono: true,
            experience: experiences::State::default(),
            running: None,
            status: if can_read {
                aurora_i18n::tr!("common-loading-2")
            } else {
                aurora_i18n::tr!("script-no-permission")
            }
            .into(),
            errors: Vec::new(),
            compile_failed: false,
            close_prompt: false,
            close_after_save: false,
            closed: false,
            raise: true,
            tools: view::Tools::opening(),
            recompile_allowed: false,
            processor_settings: settings::ScriptSettings::default(),
            processor_environment: preprocessor::Environment::default(),
            processing: None,
        });
        request
    }

    pub fn loaded(&mut self, request: Uuid, target: ScriptTarget, result: Result<String, String>) {
        if let Some(include) = self.includes.remove(&request) {
            let _ = include.reply.send(result.map(|source| preprocessor::Include {
                source,
                asset: include.asset,
            }));
            return;
        }
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target && e.loading == Some(request)) else {
            return;
        };
        e.loading = None;
        match result {
            Ok(source) => {
                e.tools.reset_history = true;
                e.tools.processed_source.clone_from(&source);
                let source = preprocessor::decode(&source);
                e.tools.has_processed = source != e.tools.processed_source;
                e.tools.diagnostics_processed = false;
                e.baseline.clone_from(&source);
                e.source = source;
                e.loaded = true;
                e.compile_failed = false;
                e.errors.clear();
                e.status.clear();
            }
            Err(reason) => e.status = reason,
        }
    }

    pub fn load_experiences(&mut self, target: ScriptTarget) -> Option<Action> {
        let e = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.can_read && !target.object.is_nil())?;
        let request = e.experience.begin()?;
        Some(Action::Experiences { request, target })
    }

    pub fn include_asset(&mut self, request: Uuid, asset: Uuid) {
        if let Some(include) = self.includes.get_mut(&request) {
            include.asset = asset;
        }
    }

    pub fn experiences_loaded(&mut self, request: Uuid, target: ScriptTarget, result: aurora_net::scripts::experiences::ScriptExperiences) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            e.experience.loaded(request, result);
        }
    }

    pub fn saved(&mut self, request: Uuid, target: ScriptTarget, result: &Result<ScriptSaved, String>) -> bool {
        let Some(e) = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.saving.as_ref().is_some_and(|s| s.request == request))
        else {
            return false;
        };
        let Some(pending) = e.saving.take() else { return false };
        match result {
            Ok(saved) => {
                e.baseline = pending.source;
                e.baseline_mono = pending.mono;
                e.experience.baseline = pending.experience;
                e.compile_failed = !saved.compiled;
                e.errors.clone_from(&saved.errors);
                e.status = if saved.compiled {
                    aurora_i18n::tr!("script-saved")
                } else {
                    aurora_i18n::tr!("script-compile-failed")
                }
                .into();
                if e.close_after_save && saved.compiled && !e.dirty() {
                    e.closed = true;
                }
            }
            Err(reason) => e.status.clone_from(reason),
        }
        e.close_after_save = false;
        true
    }

    pub fn running(&mut self, target: ScriptTarget, running: bool, mono: Option<bool>) {
        if let Some(e) = self.documents.iter_mut().find(|e| e.target == target) {
            if e.running.is_none()
                && let Some(mono) = mono
            {
                e.mono = mono;
                e.baseline_mono = mono;
            }
            e.running = Some(running);
        }
    }

    pub fn paste(&mut self, ctx: &egui::Context, target: ScriptTarget, text: &str) {
        let Some(e) = self
            .documents
            .iter_mut()
            .find(|e| e.target == target && e.can_edit && e.loaded && e.saving.is_none() && !e.tools.processed)
        else {
            return;
        };
        let id = egui::Id::unique((&format!("script_editor_{}_{}", target.object, target.item), "source_text"));
        let [a, b] = editing::cursor(ctx, id).sorted_cursors();
        if !editing::replace(ctx, id, &mut e.source, a.index.0..b.index.0, text) {
            e.status = aurora_i18n::tr!("script-source-too-large").into();
        }
    }

    pub fn color_picker(&mut self) -> Option<&mut super::color_picker::ColorPicker> {
        let index = self
            .documents
            .iter()
            .position(|e| e.colors.picker.pipette)
            .or_else(|| self.documents.iter().position(|e| e.colors.picker.is_open()))?;
        Some(&mut self.documents[index].colors.picker)
    }

    pub fn color_pipette(&self) -> bool {
        self.documents.iter().any(|e| e.colors.picker.pipette)
    }

    pub fn color_open(&self) -> bool {
        self.documents.iter().any(|e| e.colors.picker.is_open())
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        settings: &mut settings::ScriptSettings,
        color_prefs: &mut super::color_picker::Preferences,
    ) -> Vec<Action> {
        if self.rlva_hidden {
            return Vec::new();
        }
        let colors_before = color_prefs.clone();
        let mut actions = Vec::new();
        if self.panels.preferences.show(ctx, p, settings) {
            actions.push(Action::SettingsChanged);
        }
        for e in &mut self.documents {
            if e.closed {
                continue;
            }
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
            e.recompile_allowed = settings.preprocessor && settings.recompile;
            if e.processor_settings != *settings {
                e.processor_settings.clone_from(settings);
            }
            e.processor_environment.agent = self.agent;
            if e.processor_environment.agent_name != self.agent_name {
                e.processor_environment.agent_name.clone_from(&self.agent_name);
            }
            if let Some(job) = e.processing.as_ref() {
                for request in job.includes.try_iter() {
                    self.includes.insert(
                        request.id,
                        PendingInclude {
                            owner: e.target,
                            asset: Uuid::nil(),
                            reply: request.reply,
                        },
                    );
                    actions.push(Action::Include {
                        request: request.id,
                        name: request.name,
                    });
                }
            }
            if let Some(result) = e.processing.as_ref().and_then(|job| job.done.try_recv().ok()) {
                e.processing = None;
                self.includes.retain(|_, include| include.owner != e.target);
                match result {
                    Ok(output) => {
                        e.tools.has_processed = true;
                        e.tools.diagnostics_processed = true;
                        e.tools.processed_source = output.uploaded.clone();
                        if let Some(mono) = output.mono {
                            e.mono = mono;
                            if let Some(pending) = e.saving.as_mut() {
                                pending.mono = mono;
                            }
                        }
                        e.errors = output.warnings;
                        if let Some(pending) = e.saving.as_ref() {
                            actions.push(Action::Save {
                                request: pending.request,
                                target: e.target,
                                options: ScriptOptions {
                                    mono: pending.mono,
                                    running: pending.running,
                                    experience: pending.experience.unwrap_or_default(),
                                },
                                source: output.uploaded,
                            });
                        }
                    }
                    Err(reason) => {
                        e.saving = None;
                        e.close_after_save = false;
                        e.compile_failed = true;
                        e.errors = vec![reason];
                        e.status = aurora_i18n::tr!("script-preproc-failed").into();
                    }
                }
            }
            if std::mem::take(&mut e.raise) {
                super::widgets::expand_floater(ctx, &id);
            }
            let mut open = true;
            let title = format!("{}{}", e.name, if e.dirty() { " *" } else { "" });
            focus::claim(
                ctx,
                egui::LayerId::new(egui::Order::Middle, egui::Id::unique(&id)),
                egui::Id::unique((&id, "source_text")),
            );
            Floater::new(
                &id,
                title,
                ctx.content_rect().center() - Vec2::new(330.0, 260.0),
                Vec2::new(660.0, 520.0),
            )
            .cascade(super::widgets::CascadeGroup::Script)
            .allow_partial_outside()
            .show(ctx, p, &mut open, |ui| {
                view::body(
                    ui,
                    p,
                    e,
                    egui::Id::unique((&id, "source_text")),
                    settings,
                    &mut self.panels,
                    &mut actions,
                );
            });
            view::dialogs(
                ctx,
                p,
                e,
                egui::Id::unique((&id, "source_text")),
                settings,
                &mut self.panels,
                &mut actions,
            );
            let source_id = egui::Id::unique((&id, "source_text"));
            if let Some(literal) = e.colors.clicked.take() {
                e.colors.open(&mut e.source, ctx, source_id, egui::Id::unique(&id), literal);
            }
            e.colors.show(
                ctx,
                p,
                color_prefs,
                &mut e.source,
                e.can_edit && e.loaded && e.saving.is_none() && e.loading.is_none() && !e.tools.processed,
            );
            if !open {
                e.colors.cancel(&mut e.source);
                if e.dirty() || e.saving.is_some() {
                    e.close_prompt = true;
                } else {
                    e.closed = true;
                }
            }
            if e.close_prompt {
                let mut prompt_open = true;
                Floater::new(
                    &format!("{id}_close"),
                    aurora_i18n::tr!("ui-inventory-dialogs-save-changes"),
                    ctx.content_rect().center() - Vec2::new(220.0, 90.0),
                    Vec2::new(440.0, 180.0),
                )
                .fixed()
                .show(ctx, p, &mut prompt_open, |ui| {
                    ui.label(aurora_i18n::tr!("ui-inventory-dialogs-this-document-has-unsaved-changes"));
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled_ui(e.can_save(), |ui| flat_button(ui, p, aurora_i18n::tr!("common-save")))
                            .inner
                            .clicked()
                        {
                            let action = e.save();
                            if e.saving.is_some() {
                                e.close_after_save = true;
                                e.close_prompt = false;
                                actions.extend(action);
                            }
                        }
                        if ui
                            .add_enabled_ui(e.saving.is_none(), |ui| flat_button(ui, p, aurora_i18n::tr!("script-discard")))
                            .inner
                            .clicked()
                        {
                            e.closed = true;
                        }
                        if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                            e.close_prompt = false;
                            e.close_after_save = false;
                        }
                    });
                });
                if !prompt_open {
                    e.close_prompt = false;
                    e.close_after_save = false;
                }
            }
        }
        self.documents.retain(|e| !e.closed);
        focus::find(ctx, &mut self.documents, &mut self.panels.search);
        // Draw the singleton after every editor has handled its commands. A
        // search opened in a later document must not also draw on the old one.
        let search = &mut self.panels.search;
        let mut bound = false;
        for e in &mut self.documents {
            let window = format!("script_editor_{}_{}", e.target.object, e.target.item);
            let id = egui::Id::unique((&window, "source_text"));
            if search.owns(id) {
                bound = true;
                let minimized = ctx
                    .data_mut(|d| d.get_persisted::<bool>(egui::Id::unique((&window, "minimized"))))
                    .unwrap_or(false);
                if !minimized && let Some(parent) = ctx.memory(|m| m.area_rect(egui::Id::unique(&window))) {
                    view::search(
                        ctx,
                        p,
                        e,
                        id,
                        search,
                        (egui::Id::unique(&window), parent),
                        settings.search_prefer_left,
                    );
                }
                break;
            }
        }
        if !bound {
            search.open = false;
        }
        // An owner can change while a later editor handles Ctrl+F. Drop its
        // unused reserved UI within this pass, without drawing another search.
        for e in &mut self.documents {
            e.tools.search_slot = None;
        }
        if *color_prefs != colors_before {
            actions.push(Action::SettingsChanged);
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(target: ScriptTarget) -> String {
        format!("script_editor_{}_{}", target.object, target.item)
    }

    fn source_id(target: ScriptTarget) -> egui::Id {
        egui::Id::unique((window(target), "source_text"))
    }

    fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<egui::Event>) -> egui::FullOutput {
        frame_settings(ctx, editors, &mut settings::ScriptSettings::default(), events)
    }

    fn frame_settings(
        ctx: &egui::Context,
        editors: &mut Editors,
        settings: &mut settings::ScriptSettings,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        frame_settings_extra(ctx, editors, settings, events, |_, _| {})
    }

    fn frame_settings_extra(
        ctx: &egui::Context,
        editors: &mut Editors,
        settings: &mut settings::ScriptSettings,
        events: Vec<egui::Event>,
        mut extra: impl FnMut(&egui::Context, &crate::theme::Palette),
    ) -> egui::FullOutput {
        let theme = crate::theme::Theme::default();
        theme.apply(ctx, 1.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| {
                super::super::layout::frame(ui.ctx(), &theme.palette(), false);
                editors.show(
                    ui.ctx(),
                    &theme.palette(),
                    settings,
                    &mut super::super::color_picker::Preferences::default(),
                );
                extra(ui.ctx(), &theme.palette());
            },
        );
        // These headless frames inspect shapes and responses without a renderer.
        output.textures_delta.clear();
        output
    }

    fn settle(ctx: &egui::Context, editors: &mut Editors) {
        for _ in 0..3 {
            frame(ctx, editors, vec![]).textures_delta.clear();
        }
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn click_label(ctx: &egui::Context, editors: &mut Editors, label: &str) {
        click_label_settings(ctx, editors, &mut settings::ScriptSettings::default(), label);
    }

    fn click_label_settings(ctx: &egui::Context, editors: &mut Editors, settings: &mut settings::ScriptSettings, label: &str) {
        let output = frame_settings(ctx, editors, settings, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .expect("visible button");
        for pressed in [true, false] {
            frame_settings(
                ctx,
                editors,
                settings,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn opening_scripts_focuses_character_zero_and_late_loads_do_not_steal_focus() {
        for object in [Uuid::nil(), Uuid::from_u128(22)] {
            let ctx = egui::Context::default();
            let mut editors = Editors::default();
            let first = ScriptTarget {
                object,
                item: Uuid::from_u128(1),
            };
            let second = ScriptTarget {
                object,
                item: Uuid::from_u128(2),
            };
            let pending = editors.open(first, "First".into(), true, true).unwrap();
            settle(&ctx, &mut editors);
            let load = editors.open(second, "Second".into(), true, true).unwrap();
            editors.loaded(load, second, Ok("é source".into()));
            settle(&ctx, &mut editors);
            assert!(ctx.memory(|m| m.has_focus(source_id(second))));
            assert_eq!(editing::cursor(&ctx, source_id(second)).as_sorted_char_range(), 0.into()..0.into());
            frame(&ctx, &mut editors, vec![egui::Event::Text("//".into())]);
            assert_eq!(editors.documents[1].source, "//é source");
            editors.loaded(pending, first, Ok("late source".into()));
            settle(&ctx, &mut editors);
            assert!(ctx.memory(|m| m.has_focus(source_id(second))));

            editing::select(&ctx, source_id(first), 3..7, false);
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((window(first), "minimized")), true));
            settle(&ctx, &mut editors);
            editors.open(first, "First".into(), true, true);
            settle(&ctx, &mut editors);
            assert!(!super::super::widgets::floater_minimized(&ctx, &window(first)));
            assert!(ctx.memory(|m| m.has_focus(source_id(first))));
            assert_eq!(editing::cursor(&ctx, source_id(first)).as_sorted_char_range(), 0.into()..0.into());
            assert_eq!(editors.documents[0].source, "late source");
        }
    }

    #[test]
    fn script_search_reuses_its_window_and_options_and_finds_in_the_new_owner() {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let first = target(1);
        let second = ScriptTarget {
            object: Uuid::from_u128(22),
            item: Uuid::from_u128(2),
        };
        for target in [first, second] {
            let load = editors.open(target, "Search owner".into(), true, true).unwrap();
            editors.loaded(load, target, Ok("first needle\nsecond needle".into()));
        }
        settle(&ctx, &mut editors);
        editors.open(first, "Search owner".into(), true, true);
        settle(&ctx, &mut editors);
        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        settle(&ctx, &mut editors);
        assert!(editors.panels.search.owns(source_id(first)));
        editors.panels.search.query = "needle".into();
        editors.panels.search.replacement = "replacement".into();
        editors.panels.search.insensitive = false;
        editors.panels.search.up = true;
        // Rebind the still-open search, then reopen it after closing/minimizing.
        for closed in [false, true] {
            if closed {
                editors.panels.search.open = false;
                ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((dialogs::Search::WINDOW, "minimized")), true));
            }
            editors.open(second, "Search owner".into(), true, true);
            settle(&ctx, &mut editors);
            frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            settle(&ctx, &mut editors);
            assert!(editors.panels.search.owns(source_id(second)));
            assert!(!editors.panels.search.owns(source_id(first)));
            assert_eq!(editors.panels.search.query, "needle");
            assert_eq!(editors.panels.search.replacement, "replacement");
            assert!(!editors.panels.search.insensitive);
            assert!(editors.panels.search.up);
            assert!(!super::super::widgets::floater_minimized(&ctx, dialogs::Search::WINDOW));
            assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));
            let output = frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            assert!(editor_focused(&ctx));
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .filter(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title")))
                    .count(),
                1
            );
            frame(&ctx, &mut editors, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
            assert_eq!(
                editing::cursor(&ctx, source_id(second)).as_sorted_char_range(),
                20.into()..26.into()
            );
            assert!(editing::cursor(&ctx, source_id(first)).is_empty());
        }
        click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
        assert_eq!(editors.documents[0].source, "first needle\nsecond needle");
        assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
        editors.documents[1].can_edit = false;
        editors.panels.search.query = "replacement".into();
        editors.panels.search.replacement = "blocked".into();
        click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
        assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
        editors.documents[1].can_edit = true;
        editors.documents[1].tools.processed = true;
        editors.documents[1].tools.processed_source = "replacement".into();
        click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
        assert_eq!(editors.documents[1].tools.processed_source, "replacement");
        assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
    }

    #[test]
    fn ctrl_f_selects_the_whole_existing_query_in_floating_and_inline_search() {
        for inline in [false, true] {
            let ctx = egui::Context::default();
            let mut editors = Editors::default();
            let target = target(1);
            let load = editors.open(target, "Select search query".into(), true, true).unwrap();
            editors.loaded(load, target, Ok("source".into()));
            let mut settings = settings::ScriptSettings {
                search_inline: inline,
                ..Default::default()
            };
            super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(100.0, 100.0), Vec2::new(900.0, 650.0));
            for _ in 0..4 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            for _ in 0..3 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            let search_id = egui::Id::unique(dialogs::Search::WINDOW);
            let query_id = search_id.with("query");
            let replacement_id = search_id.with("replacement");
            assert!(ctx.memory(|m| m.has_focus(query_id)));
            assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 0.into()..0.into());
            assert_eq!(
                ctx.read_response(query_id).unwrap().layer_id.id == egui::Id::unique(window(target)),
                inline
            );

            for field in [source_id(target), query_id, replacement_id] {
                editors.panels.search.query = "étoile 🌟".into();
                editors.panels.search.replacement = "replacement".into();
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
                let at = if field == source_id(target) { 0 } else { 2 };
                editing::select(&ctx, field, at..at, true);
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
                frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
                for _ in 0..3 {
                    frame_settings(&ctx, &mut editors, &mut settings, vec![]);
                }
                assert!(ctx.memory(|m| m.has_focus(query_id)));
                assert_eq!(editors.panels.search.query, "étoile 🌟");
                assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 0.into()..8.into());
                frame_settings(&ctx, &mut editors, &mut settings, vec![egui::Event::Text("new".into())]);
                assert_eq!(editors.panels.search.query, "new");
                assert_eq!(editors.panels.search.replacement, "replacement");
                assert_eq!(editors.documents[0].source, "source");
            }
            // Finding with Enter must not reset a manually placed query caret.
            editing::select(&ctx, query_id, 1..1, true);
            frame_settings(
                &ctx,
                &mut editors,
                &mut settings,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            );
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 1.into()..1.into());
        }
    }

    #[test]
    fn script_search_tries_both_sides_then_overlays_its_owner_and_hides_with_it() {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let target = target(1);
        let load = editors.open(target, "Anchored".into(), true, true).unwrap();
        editors.loaded(load, target, Ok("source".into()));
        settle(&ctx, &mut editors);
        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        for (pos, size) in [
            (egui::pos2(100.0, 70.0), Vec2::new(660.0, 520.0)),
            (egui::pos2(430.0, 190.0), Vec2::new(660.0, 520.0)),
            (egui::pos2(260.0, 120.0), Vec2::new(900.0, 650.0)),
            (egui::pos2(850.0, 120.0), Vec2::new(660.0, 520.0)),
        ] {
            super::super::widgets::place_floater(&ctx, &window(target), pos, size);
            settle(&ctx, &mut editors);
            let parent = ctx.memory(|m| m.area_rect(egui::Id::unique(window(target)))).unwrap();
            let search = ctx.memory(|m| m.area_rect(egui::Id::unique(dialogs::Search::WINDOW))).unwrap();
            assert!(search.width() < 420.0, "compact default: {search:?}");
            let expected = if parent.right() + search.width() <= 1600.0 {
                parent.right_top()
            } else if parent.left() >= search.width() {
                egui::pos2(parent.left() - search.width(), parent.top())
            } else {
                egui::pos2(parent.right() - search.width(), editors.documents[0].tools.body_top)
            };
            assert!((search.left_top() - expected).length() < 1.5, "{parent:?} / {search:?}");
        }
        ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((window(target), "minimized")), true));
        settle(&ctx, &mut editors);
        let output = frame(&ctx, &mut editors, vec![]);
        assert!(
            !output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title")))
        );
        editors.open(target, "Anchored".into(), true, true);
        settle(&ctx, &mut editors);
        assert!(editors.panels.search.open);
        editors.demo_close_clean(target);
        settle(&ctx, &mut editors);
        assert!(!editors.panels.search.open);
    }

    #[test]
    fn search_moves_between_left_floating_and_inline_without_losing_fields_or_focus() {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let target = target(1);
        let load = editors.open(target, "Responsive search".into(), true, true).unwrap();
        editors.loaded(load, target, Ok("needle needle".into()));
        let mut settings = settings::ScriptSettings {
            search_prefer_left: true,
            search_inline: true,
            ..Default::default()
        };
        let parent_id = egui::Id::unique(window(target));
        let search_id = egui::Id::unique(dialogs::Search::WINDOW);
        let query_id = search_id.with("query");
        super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(470.0, 120.0), Vec2::new(660.0, 650.0));
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        editors.panels.search.query = "needle".into();
        editors.panels.search.replacement = "thread".into();
        editors.panels.search.insensitive = false;
        editors.panels.search.up = true;
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        let parent = ctx.memory(|m| m.area_rect(parent_id)).unwrap();
        let search = ctx.memory(|m| m.area_rect(search_id)).unwrap();
        assert!((search.right() - parent.left()).abs() < 1.5, "{parent:?} / {search:?}");
        assert_eq!(ctx.read_response(query_id).unwrap().layer_id.id, search_id);
        editing::select(&ctx, query_id, 2..2, true);
        for width in [840.0, 1250.0, 660.0, 840.0] {
            super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(100.0, 120.0), Vec2::new(width, 650.0));
            settings.search_inline = false;
            for _ in 0..4 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            let source_before = ctx.read_response(source_id(target)).unwrap().rect;
            settings.search_inline = true;
            for _ in 0..4 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            let output = frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            let query = ctx.read_response(query_id).unwrap();
            assert_eq!(query.layer_id.id, if width > 800.0 { parent_id } else { search_id });
            if width > 800.0 {
                let parent = ctx.memory(|m| m.area_rect(parent_id)).unwrap();
                let source_after = ctx.read_response(source_id(target)).unwrap().rect;
                assert!(parent.contains_rect(query.rect) && query.rect.left() > parent.left() + parent.width() / 3.0);
                assert!(ctx.read_response(source_id(target)).unwrap().rect.top() > query.rect.bottom());
                assert!(
                    (source_after.top() - source_before.top()).abs() < 1.5,
                    "inline changed header height: {source_before:?} / {source_after:?}"
                );
                assert!((source_after.height() - source_before.height()).abs() < 1.5);
                let replacement = ctx.read_response(search_id.with("replacement")).unwrap().rect;
                assert!(replacement.top() > query.rect.bottom() && replacement.bottom() < source_after.top());
                assert!(
                    !output
                        .shapes
                        .iter()
                        .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title")))
                );
            }
            assert_eq!(editing::cursor(&ctx, query_id).primary.index.0, 2);
            assert!(ctx.memory(|m| m.has_focus(query_id)));
            assert_eq!(editors.panels.search.query, "needle");
            assert_eq!(editors.panels.search.replacement, "thread");
            assert!(!editors.panels.search.insensitive && editors.panels.search.up);
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .filter(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-query")))
                    .count(),
                if width > 800.0 { 1 } else { 2 } // Text Find button; floating mode also has a query label.
            );
            if width > 800.0 {
                let label_pos = |label| {
                    output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos),
                            _ => None,
                        })
                        .unwrap()
                };
                let find = label_pos(aurora_i18n::tr!("script-search-find"));
                let replace = label_pos(aurora_i18n::tr!("script-search-replace"));
                assert!(
                    find.x < replace.x && (find.y - replace.y).abs() < 1.5,
                    "Find before Replace on the same row"
                );
            }
        }
        editing::select(&ctx, source_id(target), 0..0, false);
        click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-find"));
        assert_eq!(editing::cursor(&ctx, source_id(target)).as_sorted_char_range(), 7.into()..13.into());
        editors.panels.search.query = "missing".into();
        editors.panels.search.find(&ctx, source_id(target), &editors.documents[0].source);
        let source_before_status = ctx.read_response(source_id(target)).unwrap().rect;
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        assert_eq!(
            ctx.read_response(source_id(target)).unwrap().rect,
            source_before_status,
            "status must stay within the two toolbar rows"
        );
        editors.panels.search.query = "needle".into();
        click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-replace-all"));
        assert_eq!(editors.documents[0].source, "thread thread");
        editors.documents[0].can_edit = false;
        editors.panels.search.query = "thread".into();
        editors.panels.search.replacement = "blocked".into();
        click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-replace-all"));
        assert_eq!(editors.documents[0].source, "thread thread");
    }

    #[test]
    fn overlapping_search_stays_above_its_owner_without_covering_unrelated_windows() {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let first = target(1);
        let second = target(2);
        for target in [first, second] {
            let load = editors.open(target, "Layer ordering".into(), true, true).unwrap();
            editors.loaded(load, target, Ok("source".into()));
        }
        settle(&ctx, &mut editors);
        super::super::widgets::place_floater(&ctx, &window(first), egui::pos2(260.0, 120.0), Vec2::new(1100.0, 650.0));
        editors.panels.search.begin(&ctx, source_id(first), source_id(first), "source");
        settle(&ctx, &mut editors);
        let owner = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window(first)));
        let other = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window(second)));
        let search = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(dialogs::Search::WINDOW));
        for top in [owner, other] {
            ctx.move_to_top(top);
            settle(&ctx, &mut editors);
            let layers = ctx.memory(|m| m.layer_ids().collect::<Vec<_>>());
            let at = |layer| layers.iter().position(|&l| l == layer).unwrap();
            assert!(at(search) > at(owner));
            if top == other {
                assert!(at(other) > at(search));
            }
        }
    }

    #[test]
    fn dragging_scripts_can_cross_closed_edges_but_keeps_the_title_reachable() {
        for delta in [
            egui::vec2(-2000.0, 0.0),
            egui::vec2(2000.0, 0.0),
            egui::vec2(0.0, 2000.0),
            egui::vec2(0.0, -2000.0),
        ] {
            let ctx = egui::Context::default();
            let mut editors = Editors::default();
            let target = target(1);
            let load = editors.open(target, "Reachable".into(), true, true).unwrap();
            editors.loaded(load, target, Ok("source".into()));
            settle(&ctx, &mut editors);
            let id = egui::Id::unique(window(target));
            let start = ctx.read_response(id.with("title_double_click")).unwrap().rect.center();
            frame(&ctx, &mut editors, vec![egui::Event::PointerMoved(start)]);
            frame(
                &ctx,
                &mut editors,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let end = start + delta;
            frame(&ctx, &mut editors, vec![egui::Event::PointerMoved(end)]);
            frame(
                &ctx,
                &mut editors,
                vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            settle(&ctx, &mut editors);
            let rect = ctx.memory(|m| m.area_rect(id)).unwrap();
            let room = super::super::layout::move_area(&ctx);
            let visible = rect.intersect(room);
            assert!(visible.width() >= super::super::layout::TITLE_GRIP - 1.5, "{delta:?}: {rect:?}");
            assert!(visible.height() >= super::super::layout::TITLE_BAR_H - 1.5, "{delta:?}: {rect:?}");
            if delta.x < 0.0 {
                assert!(rect.left() < room.left());
            }
            if delta.x > 0.0 {
                assert!(rect.right() > room.right());
            }
            if delta.y > 0.0 {
                assert!(rect.bottom() > room.bottom());
            }
            assert!(rect.top() >= room.top());
            assert_eq!(
                super::super::layout::edges_for(&ctx, egui::Id::unique("ordinary_window")),
                super::super::layout::Edges::default()
            );
        }
    }

    #[test]
    fn ctrl_f_follows_script_window_activation_without_needing_source_field_focus() {
        fn show(ctx: &egui::Context, editors: &mut Editors, events: Vec<egui::Event>) -> egui::FullOutput {
            frame_settings_extra(ctx, editors, &mut settings::ScriptSettings::default(), events, |ctx, p| {
                let mut other = String::from("Another window");
                super::super::widgets::Floater::new("script_shortcut_other", "Other", egui::pos2(1200.0, 720.0), Vec2::new(260.0, 200.0))
                    .show(ctx, p, &mut true, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut other).id(egui::Id::unique("script_shortcut_other_field")));
                    });
            })
        }
        fn click(ctx: &egui::Context, editors: &mut Editors, pos: egui::Pos2) {
            show(ctx, editors, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                show(
                    ctx,
                    editors,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        }
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let first = target(1);
        let second = target(2);
        for (target, x) in [(first, 50.0), (second, 780.0)] {
            let load = editors.open(target, "Activation".into(), true, true).unwrap();
            editors.loaded(load, target, Ok("source".into()));
            super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(x, 100.0), Vec2::new(660.0, 520.0));
        }
        for _ in 0..4 {
            show(&ctx, &mut editors, vec![]);
        }
        for (target, area) in [
            (first, "title"),
            (second, "header"),
            (first, "control"),
            (second, "footer"),
            (first, "menu"),
        ] {
            editors.panels.search.open = false;
            for _ in 0..2 {
                show(&ctx, &mut editors, vec![]);
            }
            let parent = ctx.memory(|m| m.area_rect(egui::Id::unique(window(target)))).unwrap();
            let editor = editors.documents.iter().find(|e| e.target == target).unwrap();
            let pos = match area {
                "title" => ctx
                    .read_response(egui::Id::unique(window(target)).with("title_double_click"))
                    .unwrap()
                    .rect
                    .center(),
                "header" => egui::pos2(parent.right() - 25.0, editor.tools.body_top + 8.0),
                "footer" => egui::pos2(parent.center().x, parent.bottom() - 25.0),
                _ => {
                    let label = if area == "menu" {
                        aurora_i18n::tr!("script-menu-file")
                    } else {
                        "Mono"
                    };
                    show(&ctx, &mut editors, vec![])
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.job.text == label && parent.contains(t.pos) => {
                                Some(t.pos + t.galley.size() * 0.5)
                            }
                            _ => None,
                        })
                        .unwrap()
                }
            };
            click(&ctx, &mut editors, pos);
            assert!(
                super::super::find_shortcut::window_takes_it(&ctx),
                "{area}: app must yield Ctrl+F to the script"
            );
            // Hovering another window does not move activation there.
            show(&ctx, &mut editors, vec![egui::Event::PointerMoved(egui::pos2(1330.0, 800.0))]);
            assert!(
                super::super::find_shortcut::window_takes_it(&ctx),
                "{area}: pointer hover must not steal it"
            );
            show(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            for _ in 0..3 {
                show(&ctx, &mut editors, vec![]);
            }
            assert!(
                editors.panels.search.owns(source_id(target)),
                "{area}: find must bind the clicked script"
            );
            assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));
        }
        for outside in [
            ctx.read_response(egui::Id::unique("script_shortcut_other").with("title_double_click"))
                .unwrap()
                .rect
                .center(),
            ctx.read_response(egui::Id::unique("script_shortcut_other_field"))
                .unwrap()
                .rect
                .center(),
            egui::pos2(1560.0, 950.0),
        ] {
            editors.panels.search.open = false;
            for _ in 0..2 {
                show(&ctx, &mut editors, vec![]);
            }
            let title = ctx
                .read_response(egui::Id::unique(window(first)).with("title_double_click"))
                .unwrap()
                .rect
                .center();
            click(&ctx, &mut editors, title);
            assert!(super::super::find_shortcut::window_takes_it(&ctx));
            click(&ctx, &mut editors, outside);
            assert!(
                !super::super::find_shortcut::window_takes_it(&ctx),
                "outside click must release script Ctrl+F"
            );
            show(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            assert!(!editors.panels.search.open);
        }
        editors.documents.clear();
        show(&ctx, &mut editors, vec![]);
        assert!(!super::super::find_shortcut::window_takes_it(&ctx));
    }

    fn target(n: u128) -> ScriptTarget {
        ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(n),
        }
    }

    #[test]
    fn asynchronous_preprocessing_loads_includes_and_saves_a_snapshot_that_reopens_as_original_source() {
        let target = target(70);
        let mut editors = Editors::default();
        let load = editors.open(target, "Main".into(), true, true).unwrap();
        editors.loaded(load, target, Ok("default {}".into()));
        let original = "#include \"library.lsl\"\ndefault { state_entry() { greet(); } }";
        let settings = settings::ScriptSettings {
            preprocessor: true,
            ..Default::default()
        };
        let e = &mut editors.documents[0];
        e.processor_settings = settings.clone();
        e.source = original.into();
        assert!(e.save().is_none());
        assert!(e.saving.is_some());
        e.close_after_save = true;
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let mut settings = settings;
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut uploaded = None;
        while std::time::Instant::now() < until && uploaded.is_none() {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                actions = editors.show(ui.ctx(), &p, &mut settings, &mut crate::ui::color_picker::Preferences::default())
            });
            // No renderer consumes texture updates in this headless test.
            output.textures_delta.clear();
            for action in actions {
                match action {
                    Action::Include { request, name } => {
                        assert_eq!(name, "library.lsl");
                        editors.include_asset(request, Uuid::from_u128(71));
                        editors.loaded(request, target, Ok("greet() { llOwnerSay(__ASSETID__); }".into()));
                    }
                    Action::Save { request, source, .. } => uploaded = Some((request, source)),
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (request, uploaded) = uploaded.expect("worker produced upload");
        assert!(uploaded.contains(&Uuid::from_u128(71).to_string()));
        assert_eq!(preprocessor::decode(&uploaded), original);
        assert_eq!(editors.documents[0].source, original);
        editors.saved(
            request,
            target,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(72),
                compiled: true,
                errors: vec![],
            }),
        );
        assert!(editors.documents[0].closed);
        editors.documents.clear();
        let load = editors.open(target, "Main".into(), true, true).unwrap();
        editors.loaded(load, target, Ok(uploaded));
        assert_eq!(editors.documents[0].source, original);
        assert!(!editors.documents[0].dirty());
    }

    #[test]
    fn failed_preprocessing_keeps_the_draft_and_emits_no_network_save() {
        let target = target(80);
        let mut editors = Editors::default();
        let load = editors.open(target, "Main".into(), true, true).unwrap();
        editors.loaded(load, target, Ok("default {}".into()));
        let mut settings = settings::ScriptSettings {
            preprocessor: true,
            ..Default::default()
        };
        editors.documents[0].processor_settings = settings.clone();
        editors.documents[0].source = "#error test".into();
        assert!(editors.documents[0].save().is_none());
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while editors.documents[0].processing.is_some() && std::time::Instant::now() < until {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(
                    !editors
                        .show(ui.ctx(), &p, &mut settings, &mut crate::ui::color_picker::Preferences::default())
                        .iter()
                        .any(|a| matches!(a, Action::Save { .. }))
                )
            });
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let e = &editors.documents[0];
        assert!(e.processing.is_none());
        assert!(e.saving.is_none());
        assert!(e.compile_failed);
        assert!(e.dirty());
        assert_eq!(e.source, "#error test");
        assert_eq!(e.baseline, "default {}");
        assert!(!e.tools.diagnostics_processed);
    }

    #[test]
    fn experience_only_changes_and_removal_are_saved_with_their_snapshot() {
        use aurora_net::scripts::experiences::ScriptExperiences;
        let mut editors = Editors::default();
        let target = ScriptTarget {
            object: Uuid::from_u128(20),
            item: Uuid::from_u128(1),
        };
        let request = editors.open(target, "Test".into(), true, true).expect("load");
        editors.loaded(request, target, Ok("original".into()));
        editors.running(target, true, Some(true));
        let Action::Experiences { request, .. } = editors.load_experiences(target).expect("experiences") else {
            panic!("load experiences")
        };
        editors.experiences_loaded(
            request,
            target,
            ScriptExperiences {
                associated: Ok(Uuid::nil()),
                available: Ok(vec![]),
            },
        );
        assert!(!editors.documents[0].dirty());
        for id in [Uuid::from_u128(45), Uuid::nil()] {
            editors.documents[0].experience.selected = Some(id);
            assert!(!editors.prepare_exit());
            let Action::Save {
                request, options, source, ..
            } = editors.documents[0].save().expect("save association")
            else {
                panic!("save")
            };
            assert_eq!(options.experience, id);
            assert_eq!(source, "original");
            editors.saved(
                request,
                target,
                &Ok(ScriptSaved {
                    asset: Uuid::from_u128(99),
                    compiled: true,
                    errors: vec![],
                }),
            );
            assert!(!editors.documents[0].dirty());
        }
    }

    #[test]
    fn unknown_association_requires_a_choice_and_disconnect_rejects_metadata() {
        use aurora_net::scripts::experiences::ScriptExperiences;
        let mut editors = Editors::default();
        let target = ScriptTarget {
            object: Uuid::from_u128(20),
            item: Uuid::from_u128(1),
        };
        let request = editors.open(target, "Test".into(), true, true).expect("load");
        editors.loaded(request, target, Ok("original".into()));
        editors.running(target, true, Some(true));
        editors.documents[0].source.push_str(" // edit");
        let Action::Experiences { request, .. } = editors.load_experiences(target).expect("experiences") else {
            panic!("load")
        };
        assert!(!editors.documents[0].can_save());
        editors.experiences_loaded(
            request,
            target,
            ScriptExperiences {
                associated: Err("unavailable".into()),
                available: Ok(vec![]),
            },
        );
        assert!(!editors.documents[0].can_save());
        editors.documents[0].experience.selected = Some(Uuid::nil());
        assert!(editors.documents[0].can_save());
        let Action::Experiences { request, .. } = editors.load_experiences(target).expect("retry") else {
            panic!("load")
        };
        editors.disconnected();
        editors.experiences_loaded(
            request,
            target,
            ScriptExperiences {
                associated: Ok(Uuid::from_u128(45)),
                available: Ok(vec![]),
            },
        );
        assert_eq!(editors.documents[0].experience.selected, Some(Uuid::nil()));
        assert!(!editors.documents[0].can_save());
    }

    #[test]
    fn loaded_sources_are_clean_and_reopening_waits_for_the_server() {
        let mut editors = Editors::default();
        for object in [Uuid::nil(), Uuid::from_u128(10)] {
            let target = ScriptTarget {
                object,
                item: Uuid::from_u128(1),
            };
            let source = "default { state_entry() { llSay(0, \"Saved source\"); } }";
            let request = editors.open(target, "Test".into(), true, true).expect("request existing script");
            assert!(editors.documents[0].source.is_empty());
            assert!(!editors.documents[0].can_save());
            editors.loaded(request, target, Ok(source.into()));
            assert_eq!(editors.documents[0].source, source);
            assert!(!editors.documents[0].dirty());
            assert!(editors.prepare_exit());
            assert!(editors.demo_close_clean(target));
            let next = editors.open(target, "Test".into(), true, true).expect("request again");
            assert_ne!(request, next);
            assert!(editors.documents[0].source.is_empty());
            editors.loaded(request, target, Ok("stale".into()));
            assert!(editors.documents[0].source.is_empty());
            editors.loaded(next, target, Ok(source.into()));
            assert_eq!(editors.documents[0].source, source);
            assert!(!editors.documents[0].dirty());
            editors.documents[0].source.push_str("\n// Unsaved edit");
            assert!(!editors.demo_close_clean(target));
            assert!(!editors.prepare_exit());
            editors.documents.clear();
        }
    }

    #[test]
    fn cascade_inventory_and_object_scripts_share_one_group_without_replacing_drafts() {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let mut settings = settings::ScriptSettings::default();
        let mut colors = super::super::color_picker::Preferences::default();
        let targets = [
            ScriptTarget {
                object: Uuid::nil(),
                item: Uuid::from_u128(1),
            },
            ScriptTarget {
                object: Uuid::from_u128(10),
                item: Uuid::from_u128(1),
            },
        ];
        let mut frame = 0;
        for target in targets {
            let request = editors.open(target, "Script".into(), true, true).expect("load");
            editors.loaded(request, target, Ok("default { state_entry() {} }".into()));
            for _ in 0..8 {
                frame += 1;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        editors.show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut settings, &mut colors);
                    },
                );
                output.textures_delta.clear();
            }
        }
        let rects: Vec<_> = targets
            .iter()
            .map(|t| {
                ctx.memory(|m| {
                    m.area_rect(egui::Id::unique(format!("script_editor_{}_{}", t.object, t.item)))
                        .expect("script window")
                })
            })
            .collect();
        assert!(((rects[1].min - rects[0].min) - egui::Vec2::splat(32.0)).length() < 1.0);
        editors.documents[0].source.push_str("\n// Unsaved draft");
        assert!(editors.open(targets[0], "Script".into(), true, true).is_none());
        assert_eq!(editors.documents.len(), 2);
        assert!(editors.documents[0].dirty());
        assert!(editors.documents[0].source.ends_with("// Unsaved draft"));
    }

    #[test]
    fn load_and_save_replies_are_scoped_to_the_request_and_document() {
        let mut editors = Editors::default();
        let target = target(1);
        let request = editors.open(target, "Test".into(), true, true).expect("load");
        editors.loaded(Uuid::new_v4(), target, Ok("stale".into()));
        assert!(editors.documents[0].source.is_empty());
        editors.loaded(request, target, Ok("original".into()));
        editors.documents[0].source = "edited".into();
        let Action::Save { request, .. } = editors.documents[0].save().expect("save") else {
            panic!("save action")
        };
        assert!(!editors.saved(
            Uuid::new_v4(),
            target,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(3),
                compiled: true,
                errors: vec![]
            })
        ));
        assert!(editors.documents[0].dirty());
        editors.saved(request, target, &Err("offline".into()));
        assert!(editors.documents[0].dirty());
        assert!(editors.documents[0].can_save());
    }

    #[test]
    fn compiler_errors_keep_the_editor_open_but_track_the_saved_source() {
        let mut editors = Editors::default();
        let target = target(1);
        let request = editors.open(target, "Test".into(), true, true).expect("load");
        editors.loaded(request, target, Ok("original".into()));
        editors.documents[0].source = "invalid source".into();
        let Action::Save { request, .. } = editors.documents[0].save().expect("save script") else {
            panic!("save action")
        };
        editors.documents[0].close_after_save = true;
        editors.saved(
            request,
            target,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(3),
                compiled: false,
                errors: vec!["(1, 1) Syntax error".into()],
            }),
        );
        let e = &editors.documents[0];
        assert!(!e.dirty());
        assert!(!e.closed);
        assert_eq!(e.errors, ["(1, 1) Syntax error"]);
        assert!(e.can_save());
    }

    #[test]
    fn same_item_uuid_in_inventory_and_object_has_independent_editors_and_exit_guard() {
        let mut editors = Editors::default();
        let inventory = target(1);
        let object = ScriptTarget {
            object: Uuid::from_u128(2),
            ..inventory
        };
        for target in [inventory, object] {
            let request = editors.open(target, "Test".into(), true, true).expect("load");
            editors.loaded(request, target, Ok("original".into()));
        }
        editors.running(object, false, Some(false));
        editors.documents[0].source = "inventory edit".into();
        editors.documents[1].source = "object edit".into();
        assert!(editors.open(inventory, "Again".into(), true, true).is_none());
        assert_eq!(editors.documents.len(), 2);
        assert!(!editors.prepare_exit());
        assert!(editors.documents.iter().all(|e| e.close_prompt));
        let Action::Save { request, .. } = editors.documents[0].save().expect("save inventory") else {
            panic!("save action")
        };
        editors.saved(
            request,
            inventory,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(4),
                compiled: true,
                errors: vec![],
            }),
        );
        assert!(!editors.documents[0].dirty());
        assert!(editors.documents[1].dirty());
        assert!(!editors.documents[1].mono);
    }

    #[test]
    fn disconnect_keeps_unsaved_source_and_rejects_the_previous_sessions_reply() {
        let mut editors = Editors::default();
        let target = target(1);
        let request = editors.open(target, "Test".into(), true, true).expect("load");
        editors.loaded(request, target, Ok("original".into()));
        editors.documents[0].source = "unsaved".into();
        let Action::Save { request, .. } = editors.documents[0].save().expect("save") else {
            panic!("save action")
        };
        editors.disconnected();
        assert_eq!(editors.documents[0].source, "unsaved");
        assert!(editors.documents[0].dirty());
        assert!(!editors.documents[0].can_save());
        assert!(!editors.saved(
            request,
            target,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(3),
                compiled: true,
                errors: vec![]
            })
        ));
    }
}
