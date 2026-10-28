//! Editable projects retain inventory references and fixed-frame layers, while
//! scripted rigs stay small. File dialogs and atomic draft writes are jobs.
use super::*;
use crate::{scene::jobs::Jobs, world::inventory::actions};
use aurora_net::{
    NetCommand,
    inventory::{InvItem, notecard::Notecard},
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::Duration};

mod library;
mod target;

const MAX_BYTES: usize = 65_536;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    aurora_ik_project: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<library::Identity>,
    definition: crate::world::rig_graph::Definition,
    animations: Vec<animations::SavedStack>,
    profile: usize,
    bone: Option<String>,
    time: f64,
    source: Option<Uuid>,
    #[serde(default)]
    source_name: String,
}
impl Document {
    fn clear(&mut self) {
        self.definition = Default::default();
        self.animations.clear();
        self.profile = 0;
        self.bone = None;
        self.time = 0.0;
        self.source = None;
        self.source_name.clear();
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(invalid());
        }
        let document: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if document.aurora_ik_project != 1
            || document.project.as_ref().is_some_and(|p| !p.valid())
            || document.definition.validate().is_err()
            || if document.definition.profiles.is_empty() {
                document.profile != 0 || !document.definition.bindings.is_empty() || !document.definition.nodes.is_empty()
            } else {
                document.profile >= document.definition.profiles.len()
            }
            || document.animations.len() != document.definition.profiles.len()
            || !document.animations.iter().all(animations::SavedStack::valid)
            || document.bone.as_ref().is_some_and(|s| s.len() > 128)
            || document.source_name.len() > 256
            || !document.time.is_finite()
            || document.time < 0.0
        {
            return Err(invalid());
        }
        Ok(document)
    }
    fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        Self::decode(&bytes)?;
        Ok(bytes)
    }
}
fn invalid() -> String {
    aurora_i18n::tr!("ik-project-invalid").into()
}
#[derive(Clone, Copy)]
enum Action {
    Open,
    Restore,
    Keep,
}
enum Output {
    LibraryListed(Vec<library::Identity>),
    LibraryApplied(Vec<u8>),
    LibrarySaved(Vec<u8>),
    LibraryCreated(Vec<u8>),
    LibraryNotecard(Vec<u8>),
    LibraryDeleted(Uuid, Vec<u8>),
    LibraryFailed(String),
    Loaded(Vec<u8>),
    Draft(Option<Vec<u8>>),
    Saved,
    Autosaved(Vec<u8>),
    Cancelled,
    Failed(String),
}
pub(super) struct State {
    target: target::State,
    library: library::Library,
    owner: Option<Uuid>,
    action: Option<Action>,
    out: Vec<NetCommand>,
    saving: Option<(Uuid, Instant)>,
    loading: Option<(Uuid, Instant)>,
    picker: bool,
    filter: String,
    candidates: Vec<Uuid>,
    candidates_at: Option<(u64, String)>,
    draft: Option<Vec<u8>>,
    checked: bool,
    autosave_blocked: bool,
    busy: bool,
    was_open: bool,
    flush: bool,
    next: Option<Instant>,
    last: Vec<u8>,
    message: Option<String>,
    tx: crossbeam_channel::Sender<Output>,
    rx: crossbeam_channel::Receiver<Output>,
}
impl State {
    pub(super) fn picking_notecard(&self) -> bool {
        self.picker
    }
}
impl Default for State {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            target: target::State::default(),
            library: library::Library::default(),
            owner: None,
            action: None,
            out: Vec::new(),
            saving: None,
            loading: None,
            picker: false,
            filter: String::new(),
            candidates: Vec::new(),
            candidates_at: None,
            draft: None,
            checked: false,
            autosave_blocked: false,
            busy: false,
            was_open: false,
            flush: false,
            next: None,
            last: Vec::new(),
            message: None,
            tx,
            rx,
        }
    }
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Document::decode(&bytes)?;
    Ok(bytes)
}
fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
impl Editor {
    pub(super) fn project_error(&mut self, message: String) {
        self.project.message = Some(message);
    }
    pub(super) fn import_object_project(&mut self, world: &World, data: &crate::world::hand_ik::GraphRuntime) -> Result<(), String> {
        let frame = Frame::new(world).ok_or_else(invalid)?;
        let mut imported = Editor::default();
        imported.graph.replace_runtime(data, frame.at);
        imported.ensure_profiles(world, &frame);
        let mut name = self.prims.names.get(&data.source.root).cloned().unwrap_or_default();
        while name.len() > 128 {
            name.pop();
        }
        name = name.trim().to_owned();
        if name.is_empty() || name.chars().any(char::is_control) {
            name = aurora_i18n::tr!("ik-library-default").into();
        }
        let mut document = imported.project_document();
        document.project = Some(library::Identity {
            id: Uuid::new_v4(),
            name: name.clone(),
        });
        document.source_name = name;
        self.project.library.import(document.encode()?)
    }

    pub(super) fn begin_project_creation(&mut self) {
        self.cancel_link_pick();
        self.object_data = object_data::State::default();
        self.project.action = None;
        self.project.loading = None;
        self.project.picker = false;
        self.project.library.delete = None;
        self.project.library.open_bindings = None;
    }
    fn project_document(&self) -> Document {
        let mut animations = self.profiles.animations.clone();
        animations.resize_with(self.graph.definition.profiles.len(), Default::default);
        let profile = self.graph.profile.unwrap_or(0);
        if let Some(stack) = animations.get_mut(profile) {
            *stack = self.animations.save();
        }
        Document {
            aurora_ik_project: 1,
            project: self.project.library.active.clone(),
            definition: self.stored_definition(),
            animations,
            profile,
            bone: self.selected.and_then(|b| self.canonical_names.get(b).cloned()),
            time: self.graph.time,
            source: self.graph.source,
            source_name: if self.project.target.root == self.graph.source {
                self.project.target.name.clone()
            } else {
                String::new()
            },
        }
    }
    fn apply_project(&mut self, world: &World, bytes: &[u8]) -> Result<(), String> {
        let document = Document::decode(bytes)?;
        let frame = Frame::new(world);
        if frame.is_none() && !document.definition.profiles.is_empty() {
            return Err(invalid());
        }
        let rig = &world.avatar_lib.rig;
        if document.definition.bindings.iter().any(|b| !rig.names.contains_key(&b.bone)) {
            return Err(invalid());
        }
        self.project.library.active = document.project;
        self.leave_clip();
        self.cancel_link_pick();
        self.graph = graph::State::default();
        self.graph.definition = document.definition;
        self.graph.source = document.source;
        self.project.target = target::State {
            root: document.source,
            name: document.source_name,
            ..Default::default()
        };
        self.object_data = object_data::State::default();
        self.graph.time = document.time;
        self.bindings.clear();
        self.goals.clear();
        self.authoring = authoring::State::default();
        self.disabled.clear();
        self.profiles = profiles::State::default();
        self.profiles.animations = document.animations;
        self.animations = animations::Stack::default();
        self.selected = document.bone.and_then(|b| rig.names.get(&b).copied());
        if let Some(frame) = frame {
            self.switch_profile(document.profile, world, &frame);
        }
        self.project.draft = None;
        self.project.autosave_blocked = false;
        self.project.last.clear();
        self.project.message = Some(aurora_i18n::tr!("ik-project-loaded").into());
        Ok(())
    }
    pub fn take_project_commands(&mut self, owner: Uuid) -> Vec<NetCommand> {
        let mut out = std::mem::take(&mut self.project.out);
        if self.open {
            out.extend(self.animations.take_requests(owner));
        }
        out
    }
    pub fn project_saving(&self, request: Uuid) -> bool {
        self.project.saving.is_some_and(|(id, _)| id == request)
    }
    pub fn project_saved(&mut self, result: &Result<InvItem, String>) {
        self.project.saving = None;
        self.project.message = Some(match result {
            Ok(_) => aurora_i18n::tr!("ik-project-notecard-saved").into(),
            Err(e) => e.clone(),
        });
    }
    pub fn project_preview(&mut self, _world: &World, item: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        if self.project.loading.is_none_or(|(id, _)| id != item) {
            return false;
        }
        self.project.loading = None;
        let loaded = result
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| Notecard::decode(bytes))
            .and_then(|card| self.project.library.import(card.text.into_bytes()));
        if let Err(e) = loaded {
            self.project.message = Some(e);
        }
        true
    }
    pub fn tick_project(&mut self, jobs: &Jobs, ctx: &egui::Context, world: &World) {
        if self.poser.active {
            if world.teleporting || world.agent_id.is_nil() {
                self.poser.active = false;
                self.open = false;
            }
            return;
        }
        if self.project.owner != Some(world.agent_id) && !world.agent_id.is_nil() {
            self.project = State::default();
            self.project.owner = Some(world.agent_id);
        }
        self.project.flush |= self.project.was_open && !self.open;
        self.project.was_open = self.open;
        while let Ok(output) = self.project.rx.try_recv() {
            self.project.busy = false;
            match output {
                output @ (Output::LibraryListed(_)
                | Output::LibraryApplied(_)
                | Output::LibrarySaved(_)
                | Output::LibraryCreated(_)
                | Output::LibraryNotecard(_)
                | Output::LibraryDeleted(_, _)
                | Output::LibraryFailed(_)) => self.library_output(world, output),
                Output::Loaded(bytes) => {
                    if let Err(e) = self.project.library.import(bytes) {
                        self.project.message = Some(e);
                    }
                }
                Output::Draft(bytes) => {
                    self.project.checked = true;
                    self.project.autosave_blocked = false;
                    self.project.draft = bytes;
                }
                Output::Autosaved(bytes) => {
                    self.library_autosaved(&bytes);
                    self.project.last = bytes;
                }
                Output::Saved => self.project.message = Some(aurora_i18n::tr!("ik-project-saved").into()),
                Output::Failed(e) => self.project.message = Some(e),
                Output::Cancelled => {}
            }
        }
        let now = Instant::now();
        if self
            .project
            .saving
            .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_secs(90))
        {
            self.project.saving = None;
            self.project.message = Some(aurora_i18n::tr!("ik-project-timeout").into());
        }
        if self
            .project
            .loading
            .is_some_and(|(_, at)| now.duration_since(at) > Duration::from_secs(30))
        {
            self.project.loading = None;
            self.project.message = Some(aurora_i18n::tr!("ik-project-timeout").into());
        }
        if (!self.open && !self.project.flush) || self.clip.active || world.agent_id.is_nil() || self.project.busy {
            return;
        }
        let path = crate::settings::account_dir(&world.agent_id)
            .join("ik-projects")
            .join("draft.aurora-ik.json");
        if !self.project.checked {
            self.project.busy = true;
            self.project.autosave_blocked = true;
            let tx = self.project.tx.clone();
            let ctx = ctx.clone();
            jobs.execute(move || {
                let output = if path.exists() {
                    read(&path).map(|b| Output::Draft(Some(b))).unwrap_or_else(Output::Failed)
                } else {
                    Output::Draft(None)
                };
                let _ = tx.send(output);
                ctx.request_repaint();
            });
            // Failed drafts must not be retried each frame or overwritten.
            self.project.checked = true;
            return;
        }
        if let Some(Action::Keep) = self.project.action {
            self.project.action = None;
            self.project.draft = None;
            self.project.autosave_blocked = false;
            self.project.last.clear();
        }
        if let Some(Action::Restore) = self.project.action {
            self.project.action = None;
            if let Some(bytes) = self.project.draft.clone()
                && let Err(e) = self.apply_project(world, &bytes)
            {
                self.project.message = Some(e);
            }
            return;
        }
        if let Some(frame) = Frame::new(world) {
            self.ensure_profiles(world, &frame);
        } else {
            return;
        }
        let root = crate::settings::account_dir(&world.agent_id).join("ik-projects");
        if self.library_tick(jobs, ctx, &root) {
            return;
        }
        let path = self.project.library.active.as_ref().map_or(path, |p| library::path(&root, p.id));
        let action = self.project.action.take();
        if action.is_none() && self.project.library.active.is_none() && self.graph.definition.profiles.is_empty() {
            return;
        }
        let due = self.project.next.is_none_or(|at| now >= at);
        if action.is_none()
            && ((!due && !self.project.flush)
                || (self.project.library.active.is_none() && (self.project.draft.is_some() || self.project.autosave_blocked)))
        {
            return;
        }
        self.project.next = Some(now + Duration::from_secs(1));
        let bytes = match if matches!(action, Some(Action::Open)) {
            Ok(Vec::new())
        } else {
            self.project_document().encode()
        } {
            Ok(b) => b,
            Err(e) => {
                if action.is_some() {
                    self.project.message = Some(e);
                }
                return;
            }
        };
        if action.is_none() && bytes == self.project.last {
            self.project.flush = false;
            return;
        }
        self.project.flush = false;
        self.project.busy = true;
        let tx = self.project.tx.clone();
        let ctx = ctx.clone();
        jobs.execute(move || {
            let work = || -> Result<Output, String> {
                match action {
                    Some(Action::Open) => {
                        let Some(path) = rfd::FileDialog::new().add_filter("Aurora IK", &["json"]).pick_file() else {
                            return Ok(Output::Cancelled);
                        };
                        Ok(Output::Loaded(read(&path)?))
                    }
                    _ => {
                        let document = Document::decode(&bytes)?;
                        if document.project.is_some() {
                            library::save(&root, &document)?;
                        } else {
                            write(&path, &bytes)?;
                        }
                        Ok(Output::Autosaved(bytes))
                    }
                }
            };
            let _ = tx.send(work().unwrap_or_else(Output::Failed));
            ctx.request_repaint();
        });
    }
    fn export_project_notecard(&mut self, world: &World, bytes: Vec<u8>) {
        let result = Document::decode(&bytes).and_then(|document| {
            let identity = document.project.ok_or_else(invalid)?;
            let text = String::from_utf8(bytes).map_err(|_| invalid())?;
            Ok((identity.name, Notecard::default().encode(&text)?))
        });
        match result {
            Ok((title, data)) if self.project.saving.is_none() => {
                let request = Uuid::new_v4();
                let parent = actions::system(&world.inventory, 7).unwrap_or(world.inventory.root);
                self.project.out.push(NetCommand::CreateNotecardItem {
                    request,
                    parent,
                    name: format!("Aurora IK - {title}"),
                    data,
                    permissions: None,
                });
                self.project.saving = Some((request, Instant::now()));
            }
            Err(e) => self.project.message = Some(e),
            _ => {}
        }
    }
    fn project_recovery_ui(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if self.project.draft.is_none() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            if self.project.draft.is_some() && ui.button(aurora_i18n::tr!("ik-project-restore")).clicked() {
                self.project.action = Some(Action::Restore);
            }
            if self.project.draft.is_some() && widgets::icon_button(ui, p, "x", aurora_i18n::tr!("ik-project-keep"), true) {
                self.project.action = Some(Action::Keep);
            }
        });
    }
    pub(super) fn project_import_ui(&mut self, ui: &mut egui::Ui, world: &World) {
        if self.project.picker {
            ui.add(egui::TextEdit::singleline(&mut self.project.filter).hint_text(aurora_i18n::tr!("ik-project-notecard-search")));
            let signature = (world.inventory.generation, self.project.filter.clone());
            if self.project.candidates_at.as_ref() != Some(&signature) {
                let filter = self.project.filter.to_lowercase();
                let mut candidates: Vec<_> = world
                    .inventory
                    .items
                    .values()
                    .filter(|it| {
                        it.asset_type == 7 && it.name.to_lowercase().contains(&filter) && !world.inventory.in_folder_type(it.id, 14)
                    })
                    .collect();
                candidates.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                self.project.candidates = candidates.iter().map(|it| it.id).collect();
                self.project.candidates_at = Some(signature);
            }
            egui::ScrollArea::vertical()
                .id_salt("ik-project-notecards")
                .max_height(100.0)
                .show(ui, |ui| {
                    for id in &self.project.candidates {
                        let Some(it) = world.inventory.items.get(id) else { continue };
                        if ui.selectable_label(false, &it.name).clicked() {
                            self.project.out.push(NetCommand::PreviewInventoryItem(it.clone()));
                            self.project.loading = Some((it.id, Instant::now()));
                            self.project.picker = false;
                        }
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests;
