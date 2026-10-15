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
mod folding;
mod indent;
mod preprocessor;
mod reference;
pub mod settings;
mod syntax;
mod view;
pub use view::editor_focused;

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
    pub documents: Vec<Editor>,
    preferences: dialogs::Preferences,
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
            let id = egui::Id::new((&id, "source_text"));
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
            let id = egui::Id::new((&id, "source_text")).with("insert_filter");
            if let Some(field) = ctx.read_response(id) {
                return vec![
                    egui::Event::PointerMoved(field.rect.center_bottom() + Vec2::new(0.0, 90.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: Vec2::new(0.0, -240.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
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
        let text_id = egui::Id::new((&id, "source_text"));
        match mode {
            "search" => {
                e.tools.search.begin(ctx, text_id, &e.source);
                e.tools.search.query = "llSay".into();
                e.tools.search.replacement = "llOwnerSay".into();
                e.tools.search.find(ctx, text_id, &e.source);
            }
            "preferences" => self.preferences.open = true,
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
            let id = egui::Id::new((&id, "source_text")).with("processed");
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
        let text_id = egui::Id::new((&id, "source_text"));
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
        if let Some(editor) = self.documents.iter_mut().find(|e| e.target == target) {
            editor.raise = true;
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
            tools: view::Tools::default(),
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
        let id = egui::Id::new((&format!("script_editor_{}_{}", target.object, target.item), "source_text"));
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
        let colors_before = color_prefs.clone();
        let mut actions = Vec::new();
        if self.preferences.show(ctx, p, settings) {
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
                ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(&id)));
                ctx.data_mut(|data| data.insert_persisted(egui::Id::new((&id, "minimized")), false));
            }
            let mut open = true;
            let title = format!("{}{}", e.name, if e.dirty() { " *" } else { "" });
            Floater::new(
                &id,
                title,
                ctx.content_rect().center() - Vec2::new(330.0, 260.0),
                Vec2::new(660.0, 520.0),
            )
            .cascade(super::widgets::CascadeGroup::Script)
            .show(ctx, p, &mut open, |ui| {
                view::body(
                    ui,
                    p,
                    e,
                    egui::Id::new((&id, "source_text")),
                    settings,
                    &mut self.preferences,
                    &mut actions,
                );
            });
            view::dialogs(
                ctx,
                p,
                e,
                egui::Id::new((&id, "source_text")),
                settings,
                &mut self.preferences,
                &mut actions,
            );
            let source_id = egui::Id::new((&id, "source_text"));
            if let Some(literal) = e.colors.clicked.take() {
                e.colors.open(&mut e.source, ctx, source_id, egui::Id::new(&id), literal);
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
        if *color_prefs != colors_before {
            actions.push(Action::SettingsChanged);
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    m.area_rect(egui::Id::new(format!("script_editor_{}_{}", t.object, t.item)))
                        .expect("script window")
                })
            })
            .collect();
        assert!(((rects[1].min - rects[0].min) - egui::Vec2::splat(16.0)).length() < 1.0);
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
