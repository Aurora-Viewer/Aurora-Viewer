//! The named inventory path is resolved only after its parents are fetched.
//! Folder creation uses LLInventoryModel::createNewCategory
//! (indra/newview/llinventorymodel.cpp) through the shared
//! inventory mutation service; dependent uploads wait for its acknowledgement.
use crate::world::inventory::{FetchState, Inventory, actions};
use aurora_net::inventory::operations::{Mutation, Operation};
use uuid::Uuid;

pub(super) const BRAND: &str = "#Aurora";
pub(super) const FOLDER: &str = "Aurora Interfaces";

pub(super) enum Path {
    Ready(Uuid),
    Loading,
    Missing { parent: Uuid, brand: bool },
}

pub(super) fn unavailable() -> String {
    aurora_i18n::tr!("toolbar-notecard-folder-error").into()
}

pub(super) fn resolve(inv: &mut Inventory) -> Result<Path, String> {
    if inv.root.is_nil() {
        return Err(unavailable());
    }
    let mut parent = inv.root;
    for (index, name) in [BRAND, FOLDER].into_iter().enumerate() {
        if !ready(inv, parent)? {
            return Ok(Path::Loading);
        }
        let next = child(inv, parent, name);
        let Some(next) = next else {
            return Ok(Path::Missing { parent, brand: index == 0 });
        };
        if !actions::destination(inv, next) {
            return Err(unavailable());
        }
        parent = next;
    }
    Ok(if ready(inv, parent)? { Path::Ready(parent) } else { Path::Loading })
}

fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&parent)?
        .children
        .iter()
        .copied()
        .filter(|id| {
            inv.folders
                .get(id)
                .is_some_and(|child| !child.library && child.info.parent == parent && child.info.name == name)
        })
        .min()
}

fn ready(inv: &mut Inventory, folder: Uuid) -> Result<bool, String> {
    if !actions::destination(inv, folder) {
        return Err(unavailable());
    }
    if inv.held.is_some() {
        inv.request(folder);
        return Ok(false);
    }
    match inv.folders.get(&folder).map(|folder| folder.state) {
        Some(FetchState::Fetched) => Ok(true),
        Some(FetchState::Failed) | None => Err(unavailable()),
        _ => {
            inv.request(folder);
            Ok(false)
        }
    }
}

pub(super) fn retry(inv: &mut Inventory) {
    let mut parent = inv.root;
    let mut names = [BRAND, FOLDER].into_iter();
    loop {
        if inv.folders.get(&parent).is_some_and(|folder| folder.state == FetchState::Failed) {
            inv.request(parent);
        }
        let Some(name) = names.next() else { break };
        let next = child(inv, parent, name);
        let Some(next) = next else { break };
        parent = next;
    }
}

pub(super) fn create(parent: Uuid, brand: bool) -> Mutation {
    let mut change = Mutation::default();
    change.refresh.push(parent);
    let mut parent = parent;
    for name in [BRAND, FOLDER].into_iter().skip(usize::from(!brand)) {
        let id = Uuid::new_v4();
        change.operations.push(Operation::Folder {
            id,
            parent,
            name: name.into(),
            type_default: -1,
        });
        change.refresh.push(id);
        parent = id;
    }
    change
}

pub(super) struct Entry {
    pub id: Uuid,
    pub name: String,
    pub available: bool,
    pub writable: bool,
    pub removable: bool,
    pub asset: Uuid,
}

#[derive(Default)]
pub(super) struct Catalog {
    pub entries: Vec<Entry>,
    pub loading: bool,
    pub can_preview: bool,
    pub owner: Uuid,
    pub can_save: bool,
    generation: Option<u64>,
    folder: Option<Uuid>,
}

impl Catalog {
    pub fn sync(&mut self, inv: &mut Inventory) {
        let path = resolve(inv);
        self.owner = inv.owner;
        self.can_preview = inv.can_preview(7);
        self.can_save = matches!(path, Ok(Path::Ready(_) | Path::Missing { .. }));
        self.loading = matches!(path, Ok(Path::Loading));
        let folder = match path {
            Ok(Path::Ready(folder)) => Some(folder),
            _ => None,
        };
        if self.generation == Some(inv.generation) && self.folder == folder {
            return;
        }
        self.generation = Some(inv.generation);
        self.folder = folder;
        self.entries = folder
            .into_iter()
            .filter_map(|id| inv.folders.get(&id))
            .flat_map(|folder| &folder.items)
            .filter_map(|id| inv.items.get(id))
            .filter(|item| Some(item.parent) == folder && item.asset_type == 7)
            .map(|item| Entry {
                id: item.id,
                name: item.name.clone(),
                available: super::context_notecards::available(inv, item.id).is_some(),
                writable: replaceable(inv, item.id, item.parent, item.asset_id),
                removable: actions::removable(inv, item.id, &std::collections::HashSet::new()),
                asset: item.asset_id,
            })
            .collect();
        self.entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.id));
    }
}

// LLPreview::canModify and LLPreviewNotecard::saveIfNeeded (indra/newview/
// llpreview.cpp, llpreviewnotecard.cpp). Reuse COPY/MODIFY and Library checks;
// no elevated god permissions, and only a live item in the dedicated folder.
pub(super) fn replaceable(inv: &Inventory, item: Uuid, parent: Uuid, asset: Uuid) -> bool {
    super::context_notecards::available(inv, item).is_some_and(|item| item.parent == parent && item.asset_id == asset)
        && crate::world::inventory::preview::can_edit(inv, item, 0)
}

impl Catalog {
    pub fn target(&self, name: &str, selected: Option<Uuid>) -> Result<Option<&Entry>, String> {
        let name = name.trim();
        let invalid = || aurora_i18n::tr!("toolbar-preset-save-unavailable").to_owned();
        if name.is_empty() || name.chars().count() > 63 || name.chars().any(char::is_control) || !self.can_save || self.loading {
            return Err(invalid());
        }
        let selected = selected.and_then(|id| self.entries.iter().find(|entry| entry.id == id && entry.name == name));
        let target = if selected.is_some() {
            selected
        } else {
            let mut matches = self.entries.iter().filter(|entry| entry.name == name);
            let first = matches.next();
            if matches.next().is_some() {
                return Err(invalid());
            }
            first
        };
        if target.is_some_and(|entry| !entry.writable) {
            return Err(invalid());
        }
        Ok(target)
    }
}
