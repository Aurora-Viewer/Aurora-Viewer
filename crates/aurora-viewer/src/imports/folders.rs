//! Editable upload destinations. Folder creation follows createNewCategory /
//! AISAPI::CreateInventory; recursive disk discovery is an Aurora addition.

use super::*;
use crate::world::inventory::{Inventory, actions as rules};
use aurora_net::inventory::operations::{Operation, Reply, folder_name};
use std::{collections::BTreeSet, path::Path};

mod roots;
pub use roots::SourceRoots;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Mirror,
    Type,
    Flat,
}

#[derive(Default)]
pub struct Folders {
    pub enabled: bool,
    pub destination: Uuid,
    pub layout: Layout,
    pub roots: SourceRoots,
    pub collapsed: BTreeSet<Vec<String>>,
    pub editing: Option<Vec<String>>,
    pub edited_path: String,
    resolved_destination: Uuid,
    resolved: HashMap<Vec<String>, Uuid>,
    todo: VecDeque<Vec<String>>,
    creating: bool,
    pending: Option<(Uuid, Uuid, Vec<String>)>,
}

impl Folders {
    pub fn active(&self) -> bool {
        self.enabled
    }

    pub fn busy(&self) -> bool {
        self.creating || self.pending.is_some()
    }

    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.as_ref().is_some_and(|p| p.0 == request)
    }

    pub(super) fn cancel(&mut self) {
        self.creating = false;
        self.todo.clear();
    }

    pub(super) fn resolved(&self, path: &[String], inventory: &Inventory, protected: &HashSet<Uuid>) -> Option<Uuid> {
        if !rules::destination(inventory, self.destination, protected) {
            return None;
        }
        let mut parent = self.destination;
        if !path.is_empty() && self.destination != self.resolved_destination {
            return None;
        }
        for depth in 1..=path.len() {
            let id = *self.resolved.get(&path[..depth])?;
            let folder = inventory.folders.get(&id)?;
            if folder.info.parent != parent || folder.info.name != path[depth - 1] || !rules::destination(inventory, id, protected) {
                return None;
            }
            parent = id;
        }
        Some(parent)
    }
}

pub fn valid_path(path: &[String]) -> bool {
    path.len() <= 64
        && path.iter().all(|part| {
            !part.trim().is_empty()
                && part.trim() == part
                && part.len() <= 63
                && part != "."
                && part != ".."
                && !part.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | '|'))
        })
}

pub fn parse_path(path: &str) -> Option<Vec<String>> {
    let path: Vec<_> = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').map(str::to_owned).collect()
    };
    if !valid_path(&path) {
        return None;
    }
    let path: Vec<_> = path.iter().map(|part| folder_name(part)).collect();
    valid_path(&path).then_some(path)
}

#[cfg(test)]
fn layout_path(entry: &Entry, layout: Layout) -> Vec<String> {
    SourceRoots::default().layout_path(entry, layout)
}

/// Change a complete subtree, retaining separate files with equal names.
pub fn rename(entries: &mut [Entry], old: &[String], new: &[String]) -> bool {
    if old.is_empty()
        || !valid_path(new)
        || entries
            .iter()
            .filter_map(|e| e.upload_path.as_ref())
            .filter(|path| path.starts_with(old))
            .any(|path| path.len() - old.len() + new.len() > 64)
    {
        return false;
    }
    for entry in entries {
        if let Some(path) = &mut entry.upload_path
            && path.starts_with(old)
        {
            let suffix = path[old.len()..].to_vec();
            *path = new.iter().cloned().chain(suffix).collect();
        }
    }
    true
}

impl Imports {
    pub(super) fn batch_folder(world: &crate::world::World, requested: Uuid) -> Uuid {
        if requested.is_nil() { world.inventory.root } else { requested }
    }

    /// The routing policy is shared by the file inspector and result preview.
    pub fn sync_destinations(
        &mut self,
        prefs: &Preferences,
        world: &crate::world::World,
        inventory: &crate::ui::inventory::InventoryPreferences,
    ) {
        self.load_preferences = prefs.clone();
        if self.busy() || self.confirm {
            return;
        }
        let enabled = !prefs.use_default_folders;
        let layout = prefs.folder_layout.unwrap_or(if enabled { Layout::Flat } else { Layout::Mirror });
        let destination = Self::batch_folder(world, prefs.batch_folder);
        let layout_changed = enabled && self.folders.layout != layout;
        let changed = self.folders.enabled != enabled || enabled && (layout_changed || self.folders.destination != destination);
        self.folders.enabled = enabled;
        self.folders.layout = layout;
        self.folders.destination = destination;
        let mut roots = self.folders.roots.clone();
        roots.include = prefs.include_source_folder;
        self.update_source_roots(roots);
        if layout_changed {
            self.folders.editing = None;
            self.folders.collapsed.clear();
            for entry in self.entries.iter_mut().filter(|entry| !entry.local) {
                entry.upload_path = Some(self.folders.roots.layout_path(entry, layout));
            }
        }
        for entry in self.entries.iter_mut().filter(|entry| !entry.local) {
            if changed && let Some(Source::Mesh(draft)) = &mut entry.source {
                draft.invalidate_quote();
                entry.preview_generation += 1;
            }
            if !enabled && let Some(source) = &entry.source {
                let folder = Self::default_folder(world, inventory, source.kind());
                if entry.folder != folder {
                    entry.folder = folder;
                    if let Some(Source::Mesh(draft)) = &mut entry.source {
                        draft.invalidate_quote();
                        entry.preview_generation += 1;
                    }
                }
            }
        }
        self.sync_folder_plan(world, inventory);
    }

    /// A fee query may use the existing root before confirmation creates children.
    pub fn folder_plan_valid(&self, world: &crate::world::World, prefs: &crate::ui::inventory::InventoryPreferences) -> bool {
        if !self.folders.active() || self.choice == Choice::Local {
            return self.folder_plan_ready(world, prefs);
        }
        rules::destination(&world.inventory, self.folders.destination, &prefs.protected)
            && self
                .entries
                .iter()
                .filter(|entry| self.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Ready))
                .all(|entry| entry.upload_path.as_ref().is_some_and(|path| valid_path(path)))
    }

    pub fn sync_folder_plan(&mut self, world: &crate::world::World, prefs: &crate::ui::inventory::InventoryPreferences) {
        if self.folders.destination.is_nil() {
            self.folders.destination = world.inventory.root;
        }
        let mut mode_changed = false;
        for entry in self.entries.iter_mut().filter(|e| !e.local) {
            let previous_folder = entry.folder;
            let was_managed = entry.upload_path.is_some();
            if self.folders.active() && entry.upload_path.is_none() {
                entry.upload_path = Some(self.folders.roots.layout_path(entry, self.folders.layout));
            }
            let folder = if self.folders.active() {
                entry
                    .upload_path
                    .as_ref()
                    .and_then(|path| self.folders.resolved(path, &world.inventory, &prefs.protected))
            } else {
                None
            };
            if let Some(folder) = folder {
                entry.folder = folder;
            }
            mode_changed |= was_managed != entry.upload_path.is_some();
            // Quotes also freeze the mesh's texture destination, which changes
            // when organization is toggled even if the mesh folder stays put.
            if (entry.folder != previous_folder || was_managed != entry.upload_path.is_some())
                && let Some(Source::Mesh(draft)) = &mut entry.source
            {
                draft.invalidate_quote();
                entry.preview_generation += 1;
            }
        }
        if mode_changed {
            self.folders.collapsed.clear();
            self.folders.editing = None;
        }
    }

    pub fn folder_plan_ready(&self, world: &crate::world::World, prefs: &crate::ui::inventory::InventoryPreferences) -> bool {
        if !self.folders.active() && !self.load_preferences.use_default_folders && self.choice != Choice::Local {
            return rules::destination(
                &world.inventory,
                Self::batch_folder(world, self.load_preferences.batch_folder),
                &prefs.protected,
            );
        }
        !self.folders.active()
            || self.choice == Choice::Local
            || (!self.folders.busy()
                && self
                    .entries
                    .iter()
                    .filter(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready))
                    .all(|e| {
                        e.upload_path.as_ref().is_some_and(|path| {
                            valid_path(path)
                                && self.folders.resolved(path, &world.inventory, &prefs.protected) == Some(e.folder)
                                && rules::destination(&world.inventory, e.folder, &prefs.protected)
                        })
                    }))
    }

    pub fn prepare_import_folders(&mut self, world: &crate::world::World, prefs: &crate::ui::inventory::InventoryPreferences) {
        if self.busy() || self.pending_files.is_some() || !self.folders.active() {
            return;
        }
        self.sync_folder_plan(world, prefs);
        if !rules::destination(&world.inventory, self.folders.destination, &prefs.protected) {
            self.message = aurora_i18n::tr!("import-invalid-request").into();
            return;
        }
        let mut paths = BTreeSet::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready))
        {
            let Some(path) = &entry.upload_path else { return };
            if !valid_path(path) {
                self.message = aurora_i18n::tr!("import-folders-invalid-path").into();
                return;
            }
            for depth in 1..=path.len() {
                paths.insert(path[..depth].to_vec());
            }
        }
        if self.folders.resolved_destination != self.folders.destination {
            self.folders.resolved.clear();
            self.folders.resolved_destination = self.folders.destination;
        }
        let stale: Vec<_> = self
            .folders
            .resolved
            .keys()
            .filter(|path| self.folders.resolved(path, &world.inventory, &prefs.protected).is_none())
            .cloned()
            .collect();
        for path in stale {
            self.folders.resolved.remove(&path);
        }
        self.folders.todo = paths.into_iter().collect();
        self.folders.creating = true;
        self.message.clear();
    }

    pub fn next_import_folder(
        &mut self,
        world: &crate::world::World,
        prefs: &crate::ui::inventory::InventoryPreferences,
    ) -> Option<aurora_net::NetCommand> {
        if !self.folders.creating || self.folders.pending.is_some() {
            return None;
        }
        while let Some(path) = self.folders.todo.pop_front() {
            if self.folders.resolved(&path, &world.inventory, &prefs.protected).is_some() {
                continue;
            }
            let Some(parent) = self.folders.resolved(&path[..path.len() - 1], &world.inventory, &prefs.protected) else {
                self.folders.cancel();
                self.message = aurora_i18n::tr!("import-invalid-request").into();
                return None;
            };
            if !rules::destination(&world.inventory, parent, &prefs.protected) {
                self.folders.cancel();
                self.message = aurora_i18n::tr!("import-invalid-request").into();
                return None;
            }
            let change = match rules::create_folder(parent, &path[path.len() - 1]) {
                Ok(change) => change,
                Err(reason) => {
                    self.folders.cancel();
                    self.message = reason;
                    return None;
                }
            };
            let Some(Operation::Folder { id, .. }) = change.operations.first() else {
                return None;
            };
            let request = Uuid::new_v4();
            self.folders.pending = Some((request, *id, path));
            return Some(aurora_net::NetCommand::EditInventory { request, change });
        }
        self.folders.creating = false;
        self.sync_folder_plan(world, prefs);
        None
    }

    pub fn import_folder_result(&mut self, request: Uuid, result: Result<Reply, String>, inventory: &mut Inventory) {
        if !self.folders.owns(request) {
            return;
        }
        let Some((_, asked, path)) = self.folders.pending.take() else {
            return;
        };
        match result {
            Ok(reply) => {
                let id = reply.created.get(&asked).copied().unwrap_or(asked);
                inventory.remove(&reply.removed);
                inventory.apply(reply.contents);
                inventory.created_folders(reply.folders);
                if inventory.folders.contains_key(&id) {
                    self.folders.resolved.insert(path, id);
                } else {
                    self.folders.cancel();
                    self.message = aurora_i18n::tr!("import-folders-failed").into();
                }
            }
            Err(_) => {
                self.folders.cancel();
                self.message = aurora_i18n::tr!("import-folders-failed").into();
            }
        }
    }
}

/// Iterative, bounded discovery; links/reparse points are never traversed.
/// Decoding is left to the existing loader, which retains visible error rows.
pub(super) fn discover(root: &Path, limit: usize) -> Result<Vec<(PathBuf, Vec<String>)>, String> {
    let name = root
        .file_name()
        .ok_or_else(|| aurora_i18n::tr!("import-folders-invalid-path").to_owned())?
        .to_string_lossy()
        .into_owned();
    let mut pending = vec![(root.to_owned(), vec![name])];
    let mut files = Vec::new();
    let mut visited = 0;
    while let Some((directory, relative)) = pending.pop() {
        if relative.len() > 64 {
            return Err(aurora_i18n::tr!("import-folders-scan-limit").into());
        }
        for child in std::fs::read_dir(directory).map_err(|_| aurora_i18n::tr!("import-folders-read-failed").to_owned())? {
            visited += 1;
            if visited > 100_000 {
                return Err(aurora_i18n::tr!("import-folders-scan-limit").into());
            }
            let child = child.map_err(|_| aurora_i18n::tr!("import-folders-read-failed").to_owned())?;
            let meta = child
                .path()
                .symlink_metadata()
                .map_err(|_| aurora_i18n::tr!("import-folders-read-failed").to_owned())?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    continue;
                }
            }
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let mut path = relative.clone();
                path.push(child.file_name().to_string_lossy().into_owned());
                pending.push((child.path(), path));
            } else if meta.is_file()
                && child.path().extension().is_some_and(|e| {
                    [
                        "tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2", "wav", "bvh", "anim", "dae", "gltf", "glb",
                    ]
                    .iter()
                    .any(|ext| e.eq_ignore_ascii_case(ext))
                })
            {
                files.push((child.path(), relative.clone()));
                if files.len() > limit {
                    return Err(aurora_i18n::tr!("import-batch-limit").into());
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

#[cfg(test)]
mod tests;
