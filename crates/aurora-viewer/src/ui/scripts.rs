//! Shared inventory / object script editors (LLPreviewLSL,
//! LLLiveLSLEditor and LLScriptEdCore, Firestorm indra/newview,
//! originally LGPL 2.1). Each document owns its pending requests and
//! saved snapshot, so late replies cannot replace another editor's text.

use super::widgets::{Floater, flat_button};
use crate::theme::Palette;
use aurora_net::scripts::{MAX_SOURCE, ScriptOptions, ScriptSaved, ScriptTarget};
use egui::{RichText, Vec2};
use uuid::Uuid;

mod experiences;
mod indent;
mod syntax;

pub enum Action {
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
}

pub struct Editor {
    pub target: ScriptTarget,
    pub name: String,
    pub source: String,
    highlighter: syntax::Highlighter,
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
}

impl Editor {
    fn dirty(&self) -> bool {
        self.loaded && (self.source != self.baseline || self.mono != self.baseline_mono || self.experience.dirty())
    }

    fn can_save(&self) -> bool {
        self.loaded
            && self.can_edit
            && self.saving.is_none()
            && self.loading.is_none()
            && (self.target.object.is_nil()
                || (self.running.is_some() && self.experience.selected.is_some() && self.experience.loading.is_none()))
            && (self.dirty() || self.compile_failed)
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
        });
        self.status = aurora_i18n::tr!("script-saving").into();
        self.errors.clear();
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
}

impl Editors {
    pub fn disconnected(&mut self) {
        self.documents.retain(|e| e.dirty() || e.saving.is_some());
        for e in &mut self.documents {
            e.loading = None;
            e.experience.loading = None;
            e.saving = None;
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
        let cursor = egui::text::CCursor::new(e.source[..line].chars().count());
        let id = format!("script_editor_{}_{}", target.object, target.item);
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
        });
        request
    }

    pub fn loaded(&mut self, request: Uuid, target: ScriptTarget, result: Result<String, String>) {
        let Some(e) = self.documents.iter_mut().find(|e| e.target == target && e.loading == Some(request)) else {
            return;
        };
        e.loading = None;
        match result {
            Ok(source) => {
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

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> Vec<Action> {
        let mut actions = Vec::new();
        for e in &mut self.documents {
            if e.closed {
                continue;
            }
            let id = format!("script_editor_{}_{}", e.target.object, e.target.item);
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
            .show(ctx, p, &mut open, |ui| {
                ui.horizontal(|ui| {
                    let save = ui
                        .add_enabled_ui(e.can_save(), |ui| flat_button(ui, p, aurora_i18n::tr!("common-save")))
                        .inner;
                    if save.on_hover_text(aurora_i18n::tr!("script-save-hint")).clicked()
                        && let Some(action) = e.save()
                    {
                        actions.push(action);
                    }
                    let reload = e.can_read && !e.dirty() && e.saving.is_none() && e.loading.is_none();
                    if ui
                        .add_enabled_ui(reload, |ui| flat_button(ui, p, aurora_i18n::tr!("build-ui-contents-refresh")))
                        .inner
                        .clicked()
                    {
                        let request = Uuid::new_v4();
                        e.loading = Some(request);
                        e.loaded = false;
                        e.status = aurora_i18n::tr!("common-loading-2").into();
                        actions.push(Action::Reload { request, target: e.target });
                    }
                    ui.add_enabled_ui(
                        e.can_edit && e.loaded && e.saving.is_none() && (e.target.object.is_nil() || e.running.is_some()),
                        |ui| {
                            ui.checkbox(&mut e.mono, "Mono").on_hover_text(aurora_i18n::tr!("script-mono-hint"));
                        },
                    );
                    if !e.target.object.is_nil() {
                        ui.add_enabled_ui(e.can_edit && e.running.is_some() && e.saving.is_none(), |ui| {
                            let mut running = e.running.unwrap_or(false);
                            if ui.checkbox(&mut running, aurora_i18n::tr!("script-running")).changed() {
                                actions.push(Action::Running { target: e.target, running });
                            }
                            if flat_button(ui, p, aurora_i18n::tr!("script-reset")).clicked() {
                                actions.push(Action::Reset(e.target));
                            }
                        });
                    }
                });
                if !e.target.object.is_nil()
                    && e.experience.show(ui, p, e.can_edit && e.loaded && e.saving.is_none())
                    && let Some(request) = e.experience.begin()
                {
                    actions.push(Action::Experiences { request, target: e.target });
                }
                if !e.can_edit && e.loaded {
                    ui.label(RichText::new(aurora_i18n::tr!("script-read-only")).color(p.warn));
                }
                if !e.target.object.is_nil() && e.loaded && e.can_read && e.running.is_none() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("script-waiting-running")).color(p.warn));
                        if flat_button(ui, p, aurora_i18n::tr!("script-retry")).clicked() {
                            actions.push(Action::QueryRunning(e.target));
                        }
                    });
                }
                ui.separator();
                let height = (ui.available_height() - 110.0).max(120.0);
                let mut shortcut_save = false;
                egui::ScrollArea::both().id_salt((&id, "source")).max_height(height).show(ui, |ui| {
                    let font = egui::TextStyle::Monospace.resolve(ui.style());
                    if e.loaded && !e.can_edit {
                        let job = e.highlighter.layout(&e.source, p, font);
                        ui.add(egui::Label::new(job).selectable(true).wrap_mode(egui::TextWrapMode::Extend));
                        return;
                    }
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                        let mut job = e.highlighter.layout(text.as_str(), p, font.clone());
                        job.wrap.max_width = wrap_width;
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    let text_id = egui::Id::new((&id, "source_text"));
                    let response = indent::edit(ui, text_id, &mut e.source, |ui, source| {
                        ui.add(
                            egui::TextEdit::multiline(source)
                                .id(text_id)
                                .code_editor()
                                .layouter(&mut layouter)
                                .desired_width(f32::INFINITY)
                                .desired_rows(18)
                                .interactive(e.loaded && e.can_edit && e.saving.is_none() && e.loading.is_none()),
                        )
                    });
                    if response.has_focus() {
                        shortcut_save = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
                    }
                });
                if shortcut_save && let Some(action) = e.save() {
                    actions.push(action);
                }
                ui.separator();
                if e.source.len() > MAX_SOURCE {
                    ui.label(RichText::new(aurora_i18n::tr!("script-source-too-large")).color(p.danger));
                }
                ui.label(RichText::new(&e.status).color(if e.compile_failed { p.danger } else { p.muted }));
                egui::ScrollArea::vertical()
                    .id_salt((&id, "errors"))
                    .max_height(80.0)
                    .show(ui, |ui| {
                        for error in &e.errors {
                            ui.add(egui::Label::new(RichText::new(error).monospace().color(p.danger)).selectable(true));
                        }
                    });
            });
            if !open {
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
                            && let Some(action) = e.save()
                        {
                            e.close_after_save = true;
                            e.close_prompt = false;
                            actions.push(action);
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
