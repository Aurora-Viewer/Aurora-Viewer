//! LLInvFVBridge clipboard, LLFolderBridge and LLItemBridge operations
//! (Firestorm indra/newview/llinventorybridge.cpp, originally LGPL 2.1).

use super::{FetchState, Inventory};
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::inventory::operations::{CopyItem, Mutation, Operation};
use aurora_net::outfits::OutfitLink;
use std::collections::HashSet;
use uuid::Uuid;

pub const COPY: u32 = 0x8000;
pub const MODIFY: u32 = 0x4000;
pub const TRANSFER: u32 = 0x2000;
pub const MOVE: u32 = 0x80000;

pub fn parent(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    inv.folders
        .get(&id)
        .map(|f| f.info.parent)
        .or_else(|| inv.items.get(&id).map(|it| it.parent))
}
pub fn under(inv: &Inventory, mut id: Uuid, ancestor: Uuid) -> bool {
    let mut seen = HashSet::new();
    while seen.insert(id) {
        if id == ancestor {
            return true;
        }
        let Some(p) = parent(inv, id) else {
            return false;
        };
        id = p;
    }
    false
}
pub fn system(inv: &Inventory, kind: i32) -> Option<Uuid> {
    super::super::appearance::system_folder(inv, kind)
}
pub fn in_type(inv: &Inventory, id: Uuid, kind: i32) -> bool {
    system(inv, kind).is_some_and(|folder| under(inv, id, folder))
}
pub fn library(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| f.library) || parent(inv, id).and_then(|p| inv.folders.get(&p)).is_some_and(|f| f.library)
}
pub fn mutable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>) -> bool {
    (inv.items.contains_key(&id) || inv.folders.contains_key(&id))
        && !library(inv, id)
        && !in_type(inv, id, 46)
        && !in_type(inv, id, 50)
        && !protected.iter().any(|p| under(inv, id, *p))
}
pub fn movable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>) -> bool {
    mutable(inv, id, protected) && inv.folders.get(&id).is_none_or(|f| matches!(f.info.type_default, -1 | 47))
}
pub fn renameable(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>) -> bool {
    mutable(inv, id, protected)
        && inv.folders.get(&id).map_or_else(
            || {
                inv.items
                    .get(&id)
                    .is_some_and(|it| it.owner_mask & MODIFY != 0 && !matches!(it.asset_type, 2 | 24 | 25))
            },
            |f| matches!(f.info.type_default, -1 | 47),
        )
}
pub fn destination(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>) -> bool {
    inv.folders.contains_key(&id) && mutable(inv, id, protected) && !in_type(inv, id, 14) && !in_type(inv, id, 48) && !in_type(inv, id, 53)
}
pub fn original(inv: &Inventory, mut id: Uuid) -> Option<Uuid> {
    let mut seen = HashSet::new();
    while seen.insert(id) {
        if inv.folders.contains_key(&id) {
            return Some(id);
        }
        let it = inv.items.get(&id)?;
        if !matches!(it.asset_type, 24 | 25) {
            return Some(id);
        }
        id = it.asset_id;
    }
    None
}
pub fn default_folder(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    let kind = original(inv, id).and_then(|id| inv.items.get(&id)).map(|it| it.asset_type);
    kind.and_then(|kind| system(inv, kind)).or(Some(inv.root).filter(|id| !id.is_nil()))
}
pub fn patch(inv: &Inventory, id: Uuid, body: Llsd) -> Result<Mutation, String> {
    let old = parent(inv, id).ok_or("Élément introuvable.")?;
    let mut refresh = vec![old];
    if body.has("parent_id") {
        refresh.push(body["parent_id"].as_uuid());
    }
    Ok(Mutation {
        operations: vec![Operation::Patch {
            id,
            folder: inv.folders.contains_key(&id),
            body,
        }],
        refresh,
        ..Default::default()
    })
}
pub fn name(inv: &Inventory, id: Uuid) -> String {
    inv.items
        .get(&id)
        .map(|it| it.name.clone())
        .or_else(|| inv.folders.get(&id).map(|f| f.info.name.clone()))
        .unwrap_or_default()
}
pub fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().any(|c| c.is_control() || c == '|') {
        return Err("Saisissez un nom valide.".into());
    }
    Ok(name.chars().take(63).collect())
}
pub fn create_folder(parent: Uuid, name: &str) -> Result<Mutation, String> {
    let id = Uuid::new_v4();
    Ok(Mutation {
        operations: vec![Operation::Folder {
            id,
            parent,
            name: clean_name(name)?,
            type_default: -1,
        }],
        refresh: vec![parent, id],
        ..Default::default()
    })
}
fn push_move(inv: &Inventory, id: Uuid, to: Uuid, change: &mut Mutation) -> Result<(), String> {
    if id == to || under(inv, to, id) {
        return Err("Un dossier ne peut pas être déplacé dans lui-même.".into());
    }
    let from = parent(inv, id).ok_or("Élément introuvable.")?;
    if from == to {
        return Ok(());
    }
    change.operations.push(Operation::Patch {
        id,
        folder: inv.folders.contains_key(&id),
        body: llsd_map! { "parent_id" => to },
    });
    change.refresh.extend([from, to]);
    Ok(())
}
pub fn roots(inv: &Inventory, ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    ids.iter()
        .copied()
        .filter(|id| seen.insert(*id) && !ids.iter().any(|p| p != id && under(inv, *id, *p)))
        .collect()
}
pub fn move_selection(
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> Result<Mutation, String> {
    if !destination(inv, to, protected) && Some(to) != system(inv, 14) {
        return Err("Ce dossier ne peut pas recevoir la sélection.".into());
    }
    let mut change = Mutation::default();
    for id in roots(inv, ids) {
        if !movable(inv, id, protected) || (Some(to) == system(inv, 14) && worn.iter().any(|w| under(inv, *w, id))) {
            return Err("La sélection contient un élément protégé, système ou porté.".into());
        }
        push_move(inv, id, to, &mut change)?;
    }
    Ok(change)
}
fn copy_one(inv: &Inventory, id: Uuid, to: Uuid, change: &mut Mutation, visited: &mut HashSet<Uuid>) -> Result<(), String> {
    if !visited.insert(id) {
        return Err("Cycle dans les dossiers de la sélection.".into());
    }
    if let Some(f) = inv.folders.get(&id) {
        if f.state != FetchState::Fetched {
            return Err("Chargez tous les sous-dossiers avant de copier ce dossier.".into());
        }
        let new = Uuid::new_v4();
        change.operations.push(Operation::Folder {
            id: new,
            parent: to,
            name: f.info.name.clone(),
            type_default: -1,
        });
        change.refresh.extend([to, new]);
        for child in f.children.iter().chain(&f.items) {
            copy_one(inv, *child, new, change, visited)?;
        }
    } else if let Some(it) = inv.items.get(&id) {
        if matches!(it.asset_type, 24 | 25) {
            let target = original(inv, id).ok_or("Le lien est cassé.")?;
            change.operations.push(Operation::Links {
                parent: to,
                links: vec![OutfitLink {
                    target,
                    name: it.name.clone(),
                    desc: it.desc.clone(),
                    folder: it.asset_type == 25,
                    inv_type: it.inv_type,
                }],
            });
        } else {
            if it.owner_mask & COPY == 0 && !library(inv, id) {
                return Err("La sélection contient un élément non copiable.".into());
            }
            change.copies.push(CopyItem {
                item: id,
                owner: if library(inv, id) { inv.lib_owner } else { it.owner },
                parent: to,
                name: String::new(),
            });
        }
        change.refresh.push(to);
    } else {
        return Err("Élément introuvable.".into());
    }
    Ok(())
}
pub fn paste(
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    cut: bool,
    links: bool,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> Result<Mutation, String> {
    if ids.is_empty() || !destination(inv, to, protected) {
        return Err("Le presse-papiers est vide ou le dossier est protégé.".into());
    }
    if cut {
        return move_selection(inv, ids, to, protected, worn);
    }
    let mut change = Mutation::default();
    for id in roots(inv, ids) {
        if id == to || under(inv, to, id) {
            return Err("Un dossier ne peut pas être copié dans lui-même.".into());
        }
        if links {
            let id = original(inv, id).ok_or("Le lien est cassé.")?;
            if library(inv, id) || in_type(inv, id, 14) {
                return Err("Copiez d’abord cet élément dans votre inventaire.".into());
            }
            let folder = inv.folders.contains_key(&id);
            let it = inv.items.get(&id);
            if it.is_some_and(|it| matches!(it.asset_type, 2 | 49)) {
                return Err("Ce type d’élément ne peut pas être lié.".into());
            }
            change.operations.push(Operation::Links {
                parent: to,
                links: vec![OutfitLink {
                    target: id,
                    name: name(inv, id),
                    desc: it.map(|it| it.desc.clone()).unwrap_or_default(),
                    folder,
                    inv_type: it.map_or(8, |it| it.inv_type),
                }],
            });
            change.refresh.push(to);
        } else {
            copy_one(inv, id, to, &mut change, &mut HashSet::new())?;
        }
    }
    Ok(change)
}
pub fn group(inv: &Inventory, ids: &[Uuid], protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Result<Mutation, String> {
    let ids = roots(inv, ids);
    let to = ids.first().and_then(|id| parent(inv, *id)).ok_or("Sélection vide.")?;
    if !destination(inv, to, protected) || ids.iter().any(|id| parent(inv, *id) != Some(to)) {
        return Err("Sélectionnez des éléments du même dossier.".into());
    }
    let mut change = create_folder(to, "Nouveau dossier")?;
    let new = match &change.operations[0] {
        Operation::Folder { id, .. } => *id,
        _ => return Err("Dossier introuvable.".into()),
    };
    for id in ids {
        if !movable(inv, id, protected) || worn.iter().any(|w| under(inv, *w, id)) {
            return Err("La sélection contient un élément protégé ou porté.".into());
        }
        push_move(inv, id, new, &mut change)?;
    }
    Ok(change)
}
pub fn ungroup(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Result<Mutation, String> {
    let f = inv.folders.get(&id).ok_or("Dossier introuvable.")?;
    if f.state != FetchState::Fetched || !movable(inv, id, protected) {
        return Err("Chargez un dossier non protégé avant de le dégrouper.".into());
    }
    let ids: Vec<_> = f.children.iter().chain(&f.items).copied().collect();
    let mut change = move_selection(inv, &ids, f.info.parent, protected, worn)?;
    let trash = system(inv, 14).ok_or("Corbeille introuvable.")?;
    push_move(inv, id, trash, &mut change)?;
    Ok(change)
}

pub fn demo_mutate(inv: &mut Inventory, agent: Uuid, change: Mutation) -> Result<aurora_net::inventory::operations::Reply, String> {
    let mut created = std::collections::HashMap::new();
    let mut removed = change.removed;
    for op in change.operations {
        match op {
            Operation::Folder {
                id,
                parent,
                name,
                type_default,
            } => {
                inv.folders.insert(
                    id,
                    super::Folder {
                        info: aurora_net::inventory::InvFolder {
                            id,
                            parent,
                            name,
                            type_default,
                            version: 1,
                            ..Default::default()
                        },
                        children: Vec::new(),
                        items: Vec::new(),
                        state: FetchState::Fetched,
                        library: false,
                    },
                );
                created.insert(id, id);
            }
            Operation::Patch { id, folder, body } => {
                if folder {
                    let f = inv.folders.get_mut(&id).ok_or("Dossier introuvable.")?;
                    if body.has("name") {
                        f.info.name = body["name"].to_string_value();
                    }
                    if body.has("parent_id") {
                        f.info.parent = body["parent_id"].as_uuid();
                    }
                    if body.has("favorite") {
                        f.info.favorite = body["favorite"]["toggled"].as_bool();
                    }
                    if body.has("thumbnail") {
                        f.info.thumbnail = body["thumbnail"]["asset_id"].as_uuid();
                    }
                } else {
                    let it = inv.items.get_mut(&id).ok_or("Élément introuvable.")?;
                    if body.has("name") {
                        it.name = body["name"].to_string_value();
                    }
                    if body.has("parent_id") {
                        it.parent = body["parent_id"].as_uuid();
                    }
                    if body.has("favorite") {
                        it.favorite = body["favorite"]["toggled"].as_bool();
                    }
                    if body.has("thumbnail") {
                        it.thumbnail = body["thumbnail"]["asset_id"].as_uuid();
                    }
                    if body.has("desc") {
                        it.desc = body["desc"].to_string_value();
                    }
                    if body.has("permissions") {
                        for (key, value) in [
                            ("next_owner_mask", &mut it.next_owner_mask),
                            ("group_mask", &mut it.group_mask),
                            ("everyone_mask", &mut it.everyone_mask),
                        ] {
                            if body["permissions"].has(key) {
                                *value = body["permissions"][key].as_u32();
                            }
                        }
                    }
                    if body.has("sale_info") {
                        it.sale_type = body["sale_info"]["sale_type"].as_u32() as u8;
                        it.sale_price = body["sale_info"]["sale_price"].as_i32();
                    }
                    if body.has("linked_id") {
                        it.asset_id = body["linked_id"].as_uuid();
                    }
                }
            }
            Operation::Links { parent, links } => {
                for l in links {
                    let id = Uuid::new_v4();
                    inv.items.insert(
                        id,
                        aurora_net::inventory::InvItem {
                            id,
                            parent,
                            name: l.name,
                            desc: l.desc,
                            asset_id: l.target,
                            asset_type: if l.folder { 25 } else { 24 },
                            inv_type: l.inv_type,
                            owner: agent,
                            ..Default::default()
                        },
                    );
                }
            }
            Operation::Delete { id, .. } => removed.push(id),
            Operation::Purge(id) => {
                if let Some(f) = inv.folders.get(&id) {
                    removed.extend(f.items.iter().chain(&f.children).copied());
                }
            }
        }
    }
    inv.remove(&removed);
    for f in inv.folders.values_mut() {
        f.children.clear();
        f.items.clear();
    }
    let parents: Vec<_> = inv.folders.values().map(|f| (f.info.id, f.info.parent)).collect();
    for (id, parent) in parents {
        if let Some(f) = inv.folders.get_mut(&parent) {
            f.children.push(id);
        }
    }
    for it in inv.items.values() {
        if let Some(f) = inv.folders.get_mut(&it.parent) {
            f.items.push(it.id);
        }
    }
    inv.sort_all();
    let contents = change
        .refresh
        .into_iter()
        .filter_map(|id| inv.folders.get(&id))
        .map(|f| aurora_net::inventory::FolderContents {
            folder_id: f.info.id,
            owner_id: agent,
            version: f.info.version + 1,
            folders: f
                .children
                .iter()
                .filter_map(|id| inv.folders.get(id).map(|f| f.info.clone()))
                .collect(),
            items: f.items.iter().filter_map(|id| inv.items.get(id).cloned()).collect(),
        })
        .collect();
    Ok(aurora_net::inventory::operations::Reply {
        contents,
        copies: change.copies,
        removed,
        created,
    })
}

/// LLMarketplaceData::getMerchantStatus + move_*_to_marketplacelistings:
/// only stage inventory; no listing is published by this action.
pub fn marketplace(inv: &Inventory, ids: &[Uuid], copy: bool, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Result<Mutation, String> {
    fn validate(inv: &Inventory, id: Uuid, copy: bool, seen: &mut HashSet<Uuid>, depth: usize) -> Result<(), String> {
        if !seen.insert(id) || depth > 8 || seen.len() > 500 {
            return Err("La sélection dépasse les limites de la Place du marché.".into());
        }
        if let Some(f) = inv.folders.get(&id) {
            if f.state != FetchState::Fetched || !matches!(f.info.type_default, -1 | 47) {
                return Err("Chargez les sous-dossiers et choisissez un dossier personnel.".into());
            }
            for child in f.children.iter().chain(&f.items) {
                validate(inv, *child, copy, seen, depth + 1)?;
            }
        } else {
            let it = inv.items.get(&id).ok_or("Élément introuvable.")?;
            if it.owner_mask & TRANSFER == 0 || matches!(it.asset_type, 2 | 24 | 25) || (copy && it.owner_mask & COPY == 0) {
                return Err(
                    "Chaque élément doit être transférable et, pour une copie, copiable. Les liens et cartes de visite sont exclus.".into(),
                );
            }
        }
        Ok(())
    }
    if ids.is_empty() {
        return Err("Sélection vide.".into());
    }
    if inv.root.is_nil() {
        return Err("Inventaire indisponible.".into());
    }
    let mut change = Mutation::default();
    let root = system(inv, 53).unwrap_or_else(|| {
        let id = Uuid::new_v4();
        change.operations.push(Operation::Folder {
            id,
            parent: inv.root,
            name: "Annonces Place du marché".into(),
            type_default: 53,
        });
        change.refresh.extend([inv.root, id]);
        id
    });
    for id in roots(inv, ids) {
        if !mutable(inv, id, protected) || in_type(inv, id, 14) || in_type(inv, id, 53) || worn.iter().any(|w| under(inv, *w, id)) {
            return Err("Un élément protégé, porté, dans la corbeille ou déjà en annonce ne peut pas être envoyé.".into());
        }
        let mut seen = HashSet::new();
        validate(inv, id, copy, &mut seen, 0)?;
        let listing = Uuid::new_v4();
        change.operations.push(Operation::Folder {
            id: listing,
            parent: root,
            name: name(inv, id),
            type_default: -1,
        });
        change.refresh.extend([root, listing]);
        if inv.folders.contains_key(&id) {
            if copy {
                copy_one(inv, id, listing, &mut change, &mut HashSet::new())?;
            } else {
                push_move(inv, id, listing, &mut change)?;
                for item in seen
                    .into_iter()
                    .filter_map(|id| inv.items.get(&id))
                    .filter(|it| it.owner_mask & COPY == 0)
                {
                    let stock = Uuid::new_v4();
                    change.operations.push(Operation::Folder {
                        id: stock,
                        parent: item.parent,
                        name: item.name.clone(),
                        type_default: 54,
                    });
                    change.refresh.extend([item.parent, stock]);
                    push_move(inv, item.id, stock, &mut change)?;
                }
            }
        } else {
            let version = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id: version,
                parent: listing,
                name: name(inv, id),
                type_default: -1,
            });
            change.refresh.push(version);
            let item = &inv.items[&id];
            let to = if item.owner_mask & COPY == 0 {
                let stock = Uuid::new_v4();
                change.operations.push(Operation::Folder {
                    id: stock,
                    parent: version,
                    name: item.name.clone(),
                    type_default: 54,
                });
                change.refresh.push(stock);
                stock
            } else {
                version
            };
            if copy {
                copy_one(inv, id, to, &mut change, &mut HashSet::new())?;
            } else {
                push_move(inv, id, to, &mut change)?;
            }
        }
    }
    Ok(change)
}

/// Check without allocating provisional folders on every menu frame.
pub fn paste_allowed(
    inv: &Inventory,
    ids: &[Uuid],
    to: Uuid,
    cut: bool,
    links: bool,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> bool {
    fn copyable(inv: &Inventory, id: Uuid, seen: &mut HashSet<Uuid>) -> bool {
        if !seen.insert(id) {
            return false;
        }
        if let Some(f) = inv.folders.get(&id) {
            return f.state == FetchState::Fetched && f.children.iter().chain(&f.items).all(|id| copyable(inv, *id, seen));
        }
        inv.items.get(&id).is_some_and(|it| {
            if matches!(it.asset_type, 24 | 25) {
                original(inv, id).is_some()
            } else {
                it.owner_mask & COPY != 0 || library(inv, id)
            }
        })
    }
    !ids.is_empty()
        && destination(inv, to, protected)
        && !(links && cut)
        && ids.iter().all(|id| {
            if *id == to || under(inv, to, *id) {
                return false;
            }
            if cut {
                movable(inv, *id, protected) && (!in_type(inv, to, 14) || !worn.iter().any(|w| under(inv, *w, *id)))
            } else if links {
                original(inv, *id).is_some_and(|id| {
                    !library(inv, id) && !in_type(inv, id, 14) && inv.items.get(&id).is_none_or(|it| !matches!(it.asset_type, 2 | 49))
                })
            } else {
                copyable(inv, *id, &mut HashSet::new())
            }
        })
}

/// LLGiveInventory::giveInventoryCategory: type/UUID tuples, bounded below
/// the simulator's UDP MTU. Refuse a partially loaded folder rather than offer
/// an incomplete selection. Copy-protected contents need UI confirmation.
pub fn offer(inv: &Inventory, id: Uuid, protected: &HashSet<Uuid>, worn: &HashSet<Uuid>) -> Result<Vec<u8>, String> {
    let mut todo = vec![id];
    let mut seen = HashSet::new();
    let mut bucket = Vec::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) || seen.len() > 42 {
            return Err("Le partage est limité à 42 éléments par dossier.".into());
        }
        if !mutable(inv, id, protected) || in_type(inv, id, 14) || worn.contains(&id) {
            return Err("Un élément protégé, dans la corbeille ou porté ne peut pas être partagé.".into());
        }
        if let Some(f) = inv.folders.get(&id) {
            if f.state != FetchState::Fetched || f.info.type_default != -1 {
                return Err("Choisissez un dossier personnel entièrement chargé.".into());
            }
            bucket.push(8);
            bucket.extend_from_slice(id.as_bytes());
            todo.extend(f.items.iter().chain(&f.children).copied());
        } else {
            let it = inv.items.get(&id).ok_or("Élément introuvable.")?;
            if it.owner_mask & TRANSFER == 0 || matches!(it.asset_type, 24 | 25) {
                return Err("La sélection contient un élément non transférable ou un lien.".into());
            }
            bucket.push(it.asset_type as u8);
            bucket.extend_from_slice(id.as_bytes());
        }
    }
    if bucket.len() == 17 && inv.folders.contains_key(&id) {
        return Err("Le dossier est vide.".into());
    }
    Ok(bucket)
}

/// Inspect the exact offer, including copy-protected descendants of a folder.
pub fn offer_has_no_copy(inv: &Inventory, bucket: &[u8]) -> bool {
    bucket.as_chunks::<17>().0.iter().any(|tuple| {
        Uuid::from_slice(&tuple[1..])
            .ok()
            .and_then(|id| inv.items.get(&id))
            .is_some_and(|it| it.owner_mask & COPY == 0)
    })
}

/// Queue one bounded batch; descendants become eligible as their parents arrive.
pub fn request_tree(inv: &mut Inventory, root: Uuid) -> bool {
    let mut todo = vec![root];
    let mut seen = HashSet::new();
    let mut requests = Vec::new();
    let mut complete = true;
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else {
            continue;
        };
        if matches!(f.info.type_default, 14 | 50) {
            continue;
        }
        if f.state != FetchState::Fetched {
            complete = false;
        }
        if f.state == FetchState::Unknown && requests.len() + inv.queue.len() < 16 {
            requests.push(id);
        }
        todo.extend(f.children.iter().copied());
    }
    for id in requests {
        inv.request(id);
    }
    complete
}
