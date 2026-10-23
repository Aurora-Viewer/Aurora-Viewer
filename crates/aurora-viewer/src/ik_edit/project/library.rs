//! Account-local named projects. Stable IDs are filenames; editable titles
//! never become paths. All catalogue reads and writes run on Jobs.
use super::*;
use std::path::PathBuf;

mod browser;
mod management;
use management::Management;

const LIMIT: usize = 128;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Identity {
    pub id: Uuid,
    pub name: String,
}
impl Identity {
    pub(super) fn valid(&self) -> bool {
        !self.id.is_nil() && valid_name(&self.name)
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.trim() == name && name.len() <= 128 && !name.chars().any(char::is_control)
}
pub(super) fn path(root: &Path, id: Uuid) -> PathBuf {
    root.join("projects").join(format!("{id}.aurora-ik.json"))
}
fn load(root: &Path, id: Uuid) -> Result<Vec<u8>, String> {
    let bytes = read(&path(root, id))?;
    if Document::decode(&bytes)?.project.as_ref().is_none_or(|p| p.id != id) {
        return Err(invalid());
    }
    Ok(bytes)
}
fn list(root: &Path) -> Result<Vec<Identity>, String> {
    let dir = root.join("projects");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.take(512) {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = file.strip_suffix(".aurora-ik.json").and_then(|s| Uuid::parse_str(s).ok()) else {
            continue;
        };
        if let Ok(bytes) = read(&entry.path())
            && let Ok(doc) = Document::decode(&bytes)
            && let Some(identity) = doc.project
            && identity.id == id
        {
            entries.push(identity);
            if entries.len() >= LIMIT {
                break;
            }
        }
    }
    sort(&mut entries);
    Ok(entries)
}
fn sort(entries: &mut [Identity]) {
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
}
#[derive(Clone, Copy)]
enum EditKind {
    New,
    Duplicate,
    Rename,
}
struct Edit {
    kind: EditKind,
    target: Option<Uuid>,
    name: String,
    focus: bool,
}
pub(super) enum Command {
    New(String),
    Duplicate(String),
    Rename(String),
    Save,
    Load(Uuid),
    Reload,
    Import(Vec<u8>),
    Delete(Uuid),
    Manage(Uuid, Management),
}
#[derive(Default)]
pub(super) struct Library {
    pub active: Option<Identity>,
    pub entries: Vec<Identity>,
    pub ready: bool,
    pub switching: bool,
    edit: Option<Edit>,
    command: Option<Command>,
    reveal_new: bool,
    pub(super) delete: Option<Identity>,
    filter: String,
    pub(super) open_bindings: Option<Uuid>,
}
impl Library {
    pub(super) fn import(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        let mut document = Document::decode(&bytes)?;
        if let Some(identity) = &mut document.project {
            identity.id = Uuid::new_v4();
        }
        self.command = Some(Command::Import(document.encode()?));
        Ok(())
    }
    pub(super) fn title(&self) -> String {
        self.active
            .as_ref()
            .map_or_else(|| aurora_i18n::tr!("ik-library-default").into(), |i| i.name.clone())
    }
    fn upsert(&mut self, identity: Identity) {
        self.entries.retain(|i| i.id != identity.id);
        self.entries.push(identity);
        sort(&mut self.entries);
    }
    fn unique(&self, base: &str) -> String {
        let mut name = base.trim().to_owned();
        while name.len() > 110 {
            name.pop();
        }
        if !self.entries.iter().any(|e| e.name == name) {
            return name;
        }
        for number in 2..=LIMIT + 2 {
            let next = aurora_i18n::tr!("ik-profile-numbered", name = &name, number = number).to_string();
            if !self.entries.iter().any(|e| e.name == next) {
                return next;
            }
        }
        name
    }
    fn begin(&mut self, kind: EditKind) {
        let base = if matches!(kind, EditKind::New) {
            aurora_i18n::tr!("ik-library-default").to_string()
        } else {
            self.title()
        };
        let name = if matches!(kind, EditKind::Rename) {
            base
        } else {
            self.unique(&base)
        };
        self.edit = Some(Edit {
            kind,
            target: None,
            name,
            focus: true,
        });
    }
}
fn has_work(doc: &Document, default_profile: &str) -> bool {
    doc.project.is_some()
        || !doc.definition.bindings.is_empty()
        || !doc.definition.nodes.is_empty()
        || doc.definition.profiles.len() > 1
        || doc
            .definition
            .profiles
            .first()
            .is_some_and(|p| p.name != default_profile || p.smoothing != Some([0.2, 0.2]))
        || doc.animations.iter().any(|s| !s.rows.is_empty())
}
pub(super) fn save(root: &Path, doc: &Document) -> Result<Vec<u8>, String> {
    let identity = doc.project.as_ref().ok_or_else(invalid)?;
    let bytes = doc.encode()?;
    if !path(root, identity.id).exists() && list(root)?.len() >= LIMIT {
        return Err(aurora_i18n::tr!("ik-library-limit").into());
    }
    write(&path(root, identity.id), &bytes)?;
    Ok(bytes)
}
fn execute(root: &Path, mut current: Document, command: Command, default_name: String, default_profile: String) -> Result<Output, String> {
    if let Command::Manage(id, action) = command {
        return management::execute(root, current, id, action, default_name, default_profile);
    }
    if let Command::Delete(id) = &command {
        if current.project.as_ref().is_none_or(|p| p.id != *id) {
            return Err(invalid());
        }
        // Validate the exact UUID-named file and empty replacement before
        // removal; this transition must never autosave the deleted project.
        load(root, *id)?;
        current.clear();
        current.project = None;
        let empty = current.encode()?;
        std::fs::remove_file(path(root, *id)).map_err(|e| e.to_string())?;
        return Ok(Output::LibraryDeleted(*id, empty));
    }
    if matches!(command, Command::Reload) {
        let id = current.project.as_ref().ok_or_else(invalid)?.id;
        return Ok(Output::LibraryApplied(load(root, id)?));
    }
    // Validate a destination before saving/changing the current project.
    let loaded = if let Command::Load(id) = &command {
        Some(load(root, *id)?)
    } else {
        None
    };
    let imported = if let Command::Import(bytes) = &command {
        Some(Document::decode(bytes)?)
    } else {
        None
    };
    let next_name = match &command {
        Command::New(name) | Command::Duplicate(name) | Command::Rename(name) => Some(name.clone()),
        _ => None,
    };
    if next_name.as_ref().is_some_and(|n| !valid_name(n)) {
        return Err(invalid());
    }
    let entries = list(root)?;
    if let Some(name) = &next_name
        && entries
            .iter()
            .any(|e| &e.name == name && (!matches!(command, Command::Rename(_)) || current.project.as_ref().is_none_or(|p| p.id != e.id)))
    {
        return Err(aurora_i18n::tr!("ik-library-duplicate-name").into());
    }
    if matches!(command, Command::Save | Command::Rename(_)) {
        if let Some(name) = next_name {
            if let Some(identity) = &mut current.project {
                identity.name = name;
            } else {
                current.project = Some(Identity { id: Uuid::new_v4(), name });
            }
        }
        if current.project.is_none() {
            current.project = Some(Identity {
                id: Uuid::new_v4(),
                name: default_name,
            });
        }
        return Ok(Output::LibrarySaved(save(root, &current)?));
    }
    if has_work(&current, &default_profile) {
        if current.project.is_none() {
            let mut name = default_name.clone();
            let mut number = 2;
            while entries.iter().any(|e| e.name == name) || next_name.as_ref() == Some(&name) {
                name = aurora_i18n::tr!("ik-profile-numbered", name = &default_name, number = number).to_string();
                number += 1;
            }
            current.project = Some(Identity { id: Uuid::new_v4(), name });
        }
        save(root, &current)?;
    }
    if let Some(bytes) = loaded {
        return Ok(Output::LibraryApplied(bytes));
    }
    if let Some(mut document) = imported {
        let mut identity = document.project.take().unwrap_or_else(|| Identity {
            id: Uuid::new_v4(),
            name: default_name,
        });
        let library = Library {
            entries: list(root)?.into_iter().filter(|e| e.id != identity.id).collect(),
            ..Default::default()
        };
        identity.name = library.unique(&identity.name);
        document.project = Some(identity);
        return Ok(Output::LibraryApplied(save(root, &document)?));
    }
    if matches!(command, Command::New(_)) {
        current.clear();
    }
    current.project = Some(Identity {
        id: Uuid::new_v4(),
        name: next_name.ok_or_else(invalid)?,
    });
    Ok(Output::LibraryApplied(save(root, &current)?))
}
impl Editor {
    #[cfg(test)]
    pub(in crate::ik_edit) fn take_project_import_for_test(&mut self) -> Option<Vec<u8>> {
        if let Some(Command::Import(bytes)) = self.project.library.command.take() {
            Some(bytes)
        } else {
            None
        }
    }
    #[cfg(test)]
    pub(in crate::ik_edit) fn apply_pending_project_import_for_test(&mut self, world: &World) {
        let bytes = self.take_project_import_for_test().expect("pending project import");
        self.apply_project(world, &bytes).expect("valid project import");
    }
    fn new_project_menu(&mut self, ui: &mut egui::Ui, p: &Palette, compact: bool) {
        use crate::ui::menu;
        let response = if compact {
            widgets::action_button(ui, p, "plus", "", widgets::ActionKind::Flat, egui::Vec2::splat(20.0))
                .on_hover_text(aurora_i18n::tr!("ik-library-new"))
        } else {
            ui.button(aurora_i18n::tr!("ik-library-new"))
        };
        if self.project.library.reveal_new {
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
        }
        menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            if menu::item(ui, p, "file-description-filled", aurora_i18n::tr!("ik-project-new-blank")) {
                self.begin_project_creation();
                self.project.library.begin(EditKind::New);
            }
            if menu::item(ui, p, "box", aurora_i18n::tr!("ik-project-new-object")) {
                self.begin_project_creation();
                self.project.library.edit = None;
                self.begin_object_import();
            }
            if menu::item(ui, p, "folder-open-filled", aurora_i18n::tr!("ik-project-new-file")) {
                self.begin_project_creation();
                self.project.library.edit = None;
                self.project.action = Some(Action::Open);
            }
            if menu::item(ui, p, "note", aurora_i18n::tr!("ik-project-new-notecard")) {
                self.begin_project_creation();
                self.project.library.edit = None;
                self.project.picker = true;
            }
        });
    }
    pub(super) fn library_tick(&mut self, jobs: &Jobs, ctx: &egui::Context, root: &Path) -> bool {
        if !self.project.library.ready {
            self.project.busy = true;
            let root = root.to_owned();
            let tx = self.project.tx.clone();
            let ctx = ctx.clone();
            jobs.execute(move || {
                let output = list(&root).map(Output::LibraryListed).unwrap_or_else(Output::LibraryFailed);
                let _ = tx.send(output);
                ctx.request_repaint();
            });
            return true;
        }
        let Some(command) = self.project.library.command.take() else {
            return false;
        };
        let document = self.project_document();
        if document.encode().is_err() {
            self.project.message = Some(invalid());
            return true;
        }
        self.project.library.switching = !matches!(command, Command::Save | Command::Rename(_) | Command::Manage(_, _));
        self.project.busy = true;
        let root = root.to_owned();
        let tx = self.project.tx.clone();
        let ctx = ctx.clone();
        let default_name = self.project.library.unique(aurora_i18n::text("ik-library-default"));
        let default_profile = aurora_i18n::tr!("ik-profile-default").to_string();
        jobs.execute(move || {
            let output = execute(&root, document, command, default_name, default_profile).unwrap_or_else(Output::LibraryFailed);
            let _ = tx.send(output);
            ctx.request_repaint();
        });
        true
    }
    pub(super) fn library_output(&mut self, world: &World, output: Output) {
        let was_switching = self.project.library.switching;
        self.project.library.switching = false;
        match output {
            Output::LibraryListed(entries) => {
                self.project.library.ready = true;
                self.project.library.entries = entries;
            }
            Output::LibraryApplied(bytes) => {
                if let Err(e) = self.apply_project(world, &bytes) {
                    self.project.message = Some(e);
                } else if self.project.library.open_bindings == self.project.library.active.as_ref().map(|p| p.id)
                    && self.project.library.open_bindings.is_some()
                {
                    self.tab = 1;
                }
                self.project.library.open_bindings = None;
                // The previous project was saved by the same job; refresh its
                // catalogue entry as well, including a formerly unnamed draft.
                self.project.library.ready = false;
            }
            Output::LibrarySaved(bytes) => {
                if let Ok(document) = Document::decode(&bytes)
                    && let Some(identity) = document.project
                {
                    if self.project.library.active.as_ref().is_some_and(|p| p.id == identity.id) {
                        self.project.library.active = Some(identity.clone());
                        self.project.last = bytes;
                    }
                    self.project.library.upsert(identity);
                    self.project.message = Some(aurora_i18n::tr!("ik-project-saved").into());
                }
            }
            Output::LibraryCreated(bytes) => {
                if let Ok(document) = Document::decode(&bytes)
                    && let Some(identity) = document.project
                {
                    self.project.library.upsert(identity);
                    self.project.message = Some(aurora_i18n::tr!("ik-project-saved").into());
                }
            }
            Output::LibraryNotecard(bytes) => self.export_project_notecard(world, bytes),
            Output::LibraryDeleted(id, bytes) => {
                if self.project.library.active.as_ref().is_some_and(|p| p.id == id) {
                    if let Err(e) = self.apply_project(world, &bytes) {
                        self.project.message = Some(e);
                        return;
                    }
                    self.tab = 5;
                }
                self.project.library.entries.retain(|p| p.id != id);
                self.project.library.delete = None;
                self.project.message = Some(aurora_i18n::tr!("ik-library-deleted").into());
            }
            Output::LibraryFailed(e) => {
                self.project.library.open_bindings = None;
                self.project.library.ready = !was_switching;
                self.project.message = Some(e);
            }
            _ => {}
        }
    }
    pub(super) fn library_autosaved(&mut self, bytes: &[u8]) {
        if let Ok(doc) = Document::decode(bytes)
            && let Some(identity) = doc.project
        {
            self.project.library.upsert(identity);
        }
    }
    pub(in crate::ik_edit) fn library_title_ui(&self, ui: &mut egui::Ui) {
        if let Some(identity) = &self.project.library.active {
            ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&identity.name).strong()).truncate())
                    .on_hover_text(&identity.name);
            });
        }
    }
    fn library_controls_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let title = self.project.library.title();
        let existing = self.project.library.active.is_some();
        ui.horizontal(|ui| {
            ui.strong(aurora_i18n::tr!("ik-library-title"));
            let enabled = self.library_idle();
            ui.add_enabled_ui(enabled, |ui| {
                let mut selected = self.project.library.active.as_ref().map(|i| i.id);
                let width = (ui.available_width() - if existing { 94.0 } else { 240.0 }).max(80.0);
                egui::ComboBox::from_id_salt("ik-project-library")
                    .selected_text(if existing {
                        title.clone()
                    } else {
                        aurora_i18n::tr!("ik-library-open").to_string()
                    })
                    .width(width)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for entry in &self.project.library.entries {
                            ui.selectable_value(&mut selected, Some(entry.id), &entry.name);
                        }
                    });
                if selected != self.project.library.active.as_ref().map(|i| i.id)
                    && let Some(id) = selected
                {
                    self.select_library_project(id);
                }
                self.new_project_menu(ui, p, existing);
                if existing {
                    if widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("ik-library-save"), true) {
                        self.project.library.command = Some(Command::Save);
                    }
                    if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("ik-library-reload"), true) {
                        self.begin_project_creation();
                        self.project.library.command = Some(Command::Reload);
                    }
                }
            });
        });
        if self.project.library.active.is_some() {
            self.project_target_ui(ui, p, world);
        }
        self.project_recovery_ui(ui, p);
        self.object_data_ui(ui, p, world);
        self.project_import_ui(ui, world);
        if let Some(message) = &self.project.message {
            ui.add(egui::Label::new(message).truncate()).on_hover_text(message);
        }
    }
    fn library_name_ui(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if let Some(edit) = &mut self.project.library.edit {
            let mut confirmed = false;
            let mut cancelled = false;
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut edit.name)
                        .id(egui::Id::unique("ik-library-name"))
                        .char_limit(128)
                        .desired_width((ui.available_width() - 140.0).max(80.0)),
                );
                if edit.focus {
                    response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), response.id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(edit.name.chars().count()),
                        )));
                        state.store(ui.ctx(), response.id);
                    }
                    edit.focus = false;
                }
                let valid = valid_name(edit.name.trim());
                let label = aurora_i18n::text(match edit.kind {
                    EditKind::New => "ik-library-create",
                    EditKind::Duplicate => "ik-library-copy",
                    EditKind::Rename => "common-save",
                });
                confirmed = ui.add_enabled(valid, egui::Button::new(label)).clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && valid);
                cancelled = widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), true)
                    || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
            if confirmed {
                let name = edit.name.trim().to_owned();
                if matches!(edit.kind, EditKind::New) {
                    self.tab = 1;
                }
                let command = match edit.kind {
                    EditKind::New => Command::New(name),
                    EditKind::Duplicate => Command::Duplicate(name),
                    EditKind::Rename => Command::Rename(name),
                };
                self.project.library.command = Some(match (edit.target, command) {
                    (Some(id), Command::Duplicate(name)) => Command::Manage(id, Management::Duplicate(name)),
                    (Some(id), Command::Rename(name)) => Command::Manage(id, Management::Rename(name)),
                    (_, command) => command,
                });
            }
            if confirmed || cancelled {
                self.project.library.edit = None;
            }
        }
    }
    pub(in crate::ik_edit) fn library_start_ui(&mut self, ctx: &egui::Context, p: &Palette, world: &World) -> bool {
        if self.clip.active || (self.project.library.active.is_some() && !self.graph.definition.profiles.is_empty()) {
            return false;
        }
        if self.project.library.active.is_none() {
            self.tab = 5;
        }
        self.drag = None;
        self.gizmo.cancel();
        if !self.project_target_picking() && !self.object_import_picking() {
            self.cancel_link_pick();
        }
        let mut open = self.open;
        Floater::new(
            "ik-edit",
            aurora_i18n::tr!("ik-edit-title"),
            egui::pos2(24.0, 85.0),
            egui::vec2(530.0, 785.0),
        )
        .show(ctx, p, &mut open, |ui| {
            self.library_title_ui(ui);
            self.project_tabs_ui(ui, p);
            if self.tab == 5 {
                self.library_browser_ui(ui, p, world);
            } else if self.project.library.active.is_some()
                && let Some(frame) = Frame::new(world)
            {
                ui.add_space(ui.spacing().item_spacing.y);
                self.profiles_ui(ui, p, world, &frame);
            }
        });
        self.open = open;
        true
    }
    pub(in crate::ik_edit) fn library_creation_demo(&mut self, world: &World, mode: &str) {
        self.library_demo(world, "");
        self.project.library.entries.clear();
        if matches!(mode, "library-empty" | "library-new") {
            self.project.library.active = None;
        }
        if mode == "library-new" {
            self.project.library.begin(EditKind::New);
        } else if mode == "library-profile-name" {
            self.begin_profile_creation();
        }
    }
    pub(in crate::ik_edit) fn library_demo(&mut self, world: &World, mode: &str) {
        // Seed the demo's account before tick_project performs its first-open
        // initialization; keep synthetic catalogue entries off disk.
        self.project.owner = Some(world.agent_id);
        self.project.checked = true;
        self.project.next = Some(Instant::now() + Duration::from_secs(60));
        let moto = Identity {
            id: Uuid::from_u128(0x1b0001),
            name: "Example project".into(),
        };
        self.project.library.ready = true;
        self.project.library.active = Some(moto.clone());
        self.project.library.entries = vec![
            moto,
            Identity {
                id: Uuid::from_u128(0x1b0002),
                name: "Second project".into(),
            },
        ];
        if mode.starts_with("library") {
            self.tab = 5;
        }
        if mode == "library-name" {
            self.project.library.begin(EditKind::Duplicate);
        } else if matches!(mode, "library-projects" | "library-delete") {
            sort(&mut self.project.library.entries);
            self.tab = 5;
            if mode == "library-delete" {
                self.project.library.delete = self.project.library.active.clone();
            }
        } else if mode == "library-menu" {
            self.project.library.reveal_new = true;
        } else if mode == "library-object" {
            self.begin_object_import();
        } else if mode == "library-notecard" {
            self.project.picker = true;
        }
    }
    pub(in crate::ik_edit) fn library_loading_ui(&mut self, ctx: &egui::Context, p: &Palette) -> bool {
        if !self.project.library.switching {
            return false;
        }
        self.drag = None;
        self.gizmo.cancel();
        self.cancel_link_pick();
        let mut open = self.open;
        Floater::new(
            "ik-edit",
            aurora_i18n::tr!("ik-edit-title"),
            egui::pos2(24.0, 85.0),
            egui::vec2(530.0, 785.0),
        )
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("ik-library-loading"));
        });
        self.open = open;
        true
    }
}

#[cfg(test)]
mod tests;
