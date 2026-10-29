//! Outfit selection and COF changes, port of LLAppearanceMgr::updateCOF,
//! updateIsDirty and makeNewOutfitLinks (indra/newview/llappearancemgr.cpp,
//! originally LGPL 2.1). Required body parts survive an outfit replacement.

use super::inventory::{FetchState, Inventory};
use aurora_net::inventory::{InvFolder, InvItem};
use aurora_net::outfits::{OutfitLink, OutfitMutation};
use aurora_net::{AttachRequest, NetCommand};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const FT_OUTFIT: i32 = 47;
pub const FT_MY_OUTFITS: i32 = 48;
pub fn wearables() -> &'static Vec<&'static str> {
    static TABLE: aurora_i18n::Localized<Vec<&'static str>> = aurora_i18n::Localized::new();
    TABLE.get(|| {
        vec![
            aurora_i18n::tr!("world-appearance-silhouette"),
            aurora_i18n::tr!("world-appearance-skin"),
            aurora_i18n::tr!("world-appearance-hair"),
            aurora_i18n::tr!("world-appearance-eyes"),
            aurora_i18n::tr!("world-appearance-shirt"),
            aurora_i18n::tr!("world-appearance-pants"),
            aurora_i18n::tr!("world-appearance-shoes"),
            aurora_i18n::tr!("world-appearance-socks"),
            aurora_i18n::tr!("world-appearance-jacket"),
            aurora_i18n::tr!("world-appearance-gloves"),
            aurora_i18n::tr!("world-appearance-tank-top"),
            aurora_i18n::tr!("world-appearance-underpants"),
            aurora_i18n::tr!("world-appearance-skirt"),
            aurora_i18n::tr!("common-alpha"),
            aurora_i18n::tr!("world-appearance-tattoo"),
            aurora_i18n::tr!("world-appearance-physics-properties"),
            aurora_i18n::tr!("world-appearance-universal"),
        ]
    })
}

#[derive(Clone, Copy, Debug)]
pub enum WearMode {
    ReplaceOutfit,
    Append,
    ReplaceItems,
}

#[derive(Clone, Debug)]
pub enum Action {
    Wear(Uuid, bool),
    WearOutfits {
        folders: Vec<Uuid>,
        append: bool,
    },
    WearContents {
        folder: Uuid,
        mode: WearMode,
    },
    Save(Option<String>),
    SaveIn(String, Uuid),
    Remove(Uuid),
    RemoveItems(Vec<Uuid>),
    Add(Uuid),
    WearItem {
        item: Uuid,
        replace: bool,
        point: u8,
    },
    WearItems {
        items: Vec<Uuid>,
        replace: bool,
        point: u8,
    },
    ReplaceWearable {
        old: Uuid,
        new: Uuid,
    },
    DeleteFromOutfit {
        folder: Uuid,
        item: Uuid,
    },
    ShowOriginal(Uuid),
    Favorite(Uuid),
    Category(Uuid, aurora_net::outfits::categories::Update),
    Folders(aurora_net::inventory::operations::Mutation),
    SaveTo(Uuid),
    RemoveOutfit(Uuid),
    RemoveOutfits(Vec<Uuid>),
    /// A clothing layer one step closer to the body (true) or further from
    /// it, over the next one (LLAppearanceMgr::moveWearable).
    MoveLayer(Uuid, bool),
    /// A clothing layer dropped at a body index of its wearable type, 0
    /// closest to the body (LLAppearanceMgr::reorderWearable).
    PlaceLayer(Uuid, usize),
    Revert,
}

pub fn system_folder(inv: &Inventory, kind: i32) -> Option<Uuid> {
    inv.folders
        .values()
        .find(|f| f.info.type_default == kind && !f.library)
        .map(|f| f.info.id)
}

pub fn cof(inv: &Inventory) -> Option<Uuid> {
    system_folder(inv, super::outfit::FT_CURRENT_OUTFIT)
}

/// Confirmed clothing only: required body parts never enter Take Off.
pub fn worn_clothing(inv: &Inventory) -> Vec<(Uuid, u8)> {
    let Some(cof) = cof(inv) else { return Vec::new() };
    let mut items: Vec<_> = folder_links(inv, cof)
        .into_iter()
        .filter(|link| !link.folder)
        .filter_map(|link| inv.items.get(&link.target))
        .filter(|item| item.asset_type == 5 && (4..=16).contains(&(item.flags & 0xff)))
        .map(|item| (item.id, (item.flags & 0xff) as u8))
        .collect();
    items.sort_unstable();
    items.dedup();
    items
}

pub fn base(inv: &Inventory) -> Option<Uuid> {
    let folder = inv.folders.get(&cof(inv)?)?;
    folder.items.iter().filter_map(|id| inv.items.get(id)).find_map(|it| {
        (it.asset_type == 25 && inv.folders.get(&it.asset_id).is_some_and(|f| f.info.type_default == FT_OUTFIT)).then_some(it.asset_id)
    })
}

pub fn folder_links(inv: &Inventory, folder: Uuid) -> Vec<OutfitLink> {
    let mut links: Vec<_> = inv
        .folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter_map(|id| inv.items.get(id))
        .filter(|it| it.parent == folder)
        .filter_map(|it| {
            if matches!(it.asset_type, 24 | 25) {
                Some(OutfitLink {
                    target: it.asset_id,
                    name: it.name.clone(),
                    desc: it.desc.clone(),
                    folder: it.asset_type == 25,
                    inv_type: it.inv_type,
                })
            } else if matches!(it.asset_type, 5 | 6 | 13 | 21) {
                Some(item_link(it))
            } else {
                None
            }
        })
        .collect();
    // LLAppearanceMgr::removeDuplicateItems keeps the last link to each
    // original, preserving its layer order. Separate inventory copies stay.
    let mut seen = HashSet::new();
    links.reverse();
    links.retain(|l| !l.target.is_nil() && seen.insert((l.folder, l.target)));
    links.reverse();
    links
}

pub(super) fn item_link(it: &InvItem) -> OutfitLink {
    OutfitLink {
        target: it.id,
        name: it.name.clone(),
        desc: it.desc.clone(),
        folder: false,
        inv_type: it.inv_type,
    }
}

pub fn dirty(inv: &Inventory) -> bool {
    let Some((cof, base)) = cof(inv).zip(base(inv)) else {
        return false;
    };
    let keys = |id| {
        let mut keys: Vec<_> = folder_links(inv, id)
            .into_iter()
            .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| matches!(it.asset_type, 5 | 6 | 13 | 21)))
            .map(|l| (l.target, l.name, l.desc))
            .collect();
        keys.sort();
        keys
    };
    loaded(inv, base) && keys(cof) != keys(base)
}

pub fn loaded(inv: &Inventory, folder: Uuid) -> bool {
    inv.folders.get(&folder).is_some_and(|f| f.state == FetchState::Fetched)
}

pub fn complete(inv: &Inventory, folder: Uuid) -> bool {
    loaded(inv, folder)
        && folder_links(inv, folder)
            .iter()
            .all(|l| l.folder || inv.items.contains_key(&l.target))
}

/// Only the relevant folders and link targets are fetched, including retry.
pub fn prepare(inv: &mut Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> Vec<NetCommand> {
    let folders = prepare_folders(inv);
    let mut missing: HashSet<_> = worn.iter().filter(|id| !inv.items.contains_key(id)).copied().collect();
    for id in &folders {
        inv.request(*id);
        missing.extend(missing_targets(inv, *id));
    }
    fetch_items(missing, agent)
}

/// The folders Appearance keeps loaded: the Current Outfit, My Outfits, the
/// worn outfit and every group and outfit of My Outfits.
pub fn prepare_folders(inv: &Inventory) -> Vec<Uuid> {
    let my = system_folder(inv, FT_MY_OUTFITS);
    let mut folders: Vec<_> = [cof(inv), my, base(inv)].into_iter().flatten().collect();
    folders.extend(super::outfit_folders::folders(inv));
    folders
}

/// The originals of a folder's item links that the inventory does not hold
/// yet: the targets `folder_links` would list without an item, read without
/// copying the links (a wardrobe of thousands of outfits is walked whenever
/// the inventory changes).
pub fn missing_targets(inv: &Inventory, folder: Uuid) -> impl Iterator<Item = Uuid> + '_ {
    inv.folders
        .get(&folder)
        .into_iter()
        .flat_map(|f| &f.items)
        .filter_map(|id| inv.items.get(id))
        .filter(move |it| it.parent == folder && it.asset_type == 24 && !it.asset_id.is_nil())
        .map(|it| it.asset_id)
        .filter(|target| !inv.items.contains_key(target))
}

/// One FetchInventory2 request for these items, none when there are none.
pub fn fetch_items(missing: HashSet<Uuid>, agent: Uuid) -> Vec<NetCommand> {
    if missing.is_empty() {
        Vec::new()
    } else {
        vec![NetCommand::FetchItems {
            items: missing.into_iter().collect(),
            owner: agent,
        }]
    }
}

/// LLRightClickInventoryFetchDescendentsObserver
/// (indra/newview/llinventorybridge.cpp): load only the menu's subtree and
/// its link targets, without opening the Appearance window.
pub fn prepare_contents(inv: &mut Inventory, agent: Uuid, folder: Uuid) -> Vec<NetCommand> {
    let mut pending = vec![folder];
    let mut seen = HashSet::new();
    let mut missing = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = inv.folders.get(&id) else { continue };
        if f.library || matches!(f.info.type_default, 14 | 50) {
            continue;
        }
        pending.extend(f.children.iter().copied());
        for link in folder_links(inv, id) {
            if !link.folder && !link.target.is_nil() && !inv.items.contains_key(&link.target) {
                missing.insert(link.target);
            }
        }
        if inv.queue.len() < 16 {
            inv.request(id);
        }
    }
    if missing.is_empty() {
        Vec::new()
    } else {
        vec![NetCommand::FetchItems {
            items: missing.into_iter().collect(),
            owner: agent,
        }]
    }
}

/// LLAppearanceMgr::wearInventoryCategory: ordinary folders are recursive;
/// unrelated inventory types are ignored, incomplete or broken links are refused.
pub fn wearable_contents(inv: &Inventory, folder: Uuid) -> Result<Vec<OutfitLink>, String> {
    let mut pending = vec![folder];
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id) {
            return Err(aurora_i18n::tr!("world-appearance-cycle-in-outfit-folders").into());
        }
        let f = inv.folders.get(&id).ok_or(aurora_i18n::tr!("common-folder-not-found"))?;
        require_complete(inv, id)?;
        for link in folder_links(inv, id).into_iter().filter(|l| !l.folder) {
            let item = inv
                .items
                .get(&link.target)
                .ok_or(aurora_i18n::tr!("world-appearance-an-outfit-link-is-broken-or-still-loading"))?;
            if matches!(item.asset_type, 5 | 6 | 13 | 21) {
                links.push(link);
            }
        }
        pending.extend(f.children.iter().copied());
    }
    let mut targets = HashSet::new();
    links.retain(|l| targets.insert(l.target));
    Ok(links)
}

fn require_complete(inv: &Inventory, folder: Uuid) -> Result<(), String> {
    if complete(inv, folder) {
        Ok(())
    } else {
        Err(aurora_i18n::tr!("world-appearance-this-outfit-s-inventory-is-still-loading").into())
    }
}

fn base_link(inv: &Inventory, id: Uuid) -> OutfitLink {
    OutfitLink {
        target: id,
        name: inv.folders.get(&id).map(|f| f.info.name.clone()).unwrap_or_default(),
        desc: String::new(),
        folder: true,
        inv_type: 8,
    }
}

/// Produces one acknowledged AIS operation. No optimistic inventory mutation.
pub fn plan(inv: &Inventory, action: Action, worn: &HashMap<Uuid, u8>) -> Result<(OutfitMutation, bool), String> {
    // DeleteFromOutfit removes the saved link, never the original or the worn item.
    if let Action::DeleteFromOutfit { folder, item } = action {
        if !inv
            .folders
            .get(&folder)
            .is_some_and(|f| f.info.type_default == FT_OUTFIT && !f.library)
            || !loaded(inv, folder)
        {
            return Err(aurora_i18n::tr!("world-appearance-the-outfit-folder-is-unavailable").into());
        }
        return Ok((
            OutfitMutation {
                folder,
                create: None,
                links: folder_links(inv, folder).into_iter().filter(|l| l.target != item).collect(),
                cof: None,
            },
            false,
        ));
    }
    let cof = cof(inv).ok_or(aurora_i18n::tr!("world-appearance-the-current-outfit-folder-is-unavailable"))?;
    require_complete(inv, cof)?;
    let mut current = folder_links(inv, cof);
    if matches!(
        action,
        Action::Add(_)
            | Action::WearItem { .. }
            | Action::WearItems { .. }
            | Action::Remove(_)
            | Action::RemoveItems(_)
            | Action::RemoveOutfit(_)
            | Action::RemoveOutfits(_)
            | Action::WearOutfits { append: true, .. }
            | Action::MoveLayer(..)
            | Action::PlaceLayer(..)
    ) {
        for id in worn.keys() {
            if !current.iter().any(|l| l.target == *id) {
                let it = inv
                    .items
                    .get(id)
                    .ok_or(aurora_i18n::tr!("world-appearance-a-worn-object-is-still-loading"))?;
                current.push(item_link(it));
            }
        }
    }
    let mut change = OutfitMutation {
        folder: cof,
        create: None,
        links: Vec::new(),
        cof: None,
    };
    let original_current = current.clone();
    let mut sync = true;
    match action {
        Action::Save(_) | Action::SaveIn(..) | Action::SaveTo(_) => {
            let my = system_folder(inv, FT_MY_OUTFITS);
            let (name, destination, parent) = match action {
                Action::Save(name) => (name, base(inv), my),
                Action::SaveIn(name, parent) => {
                    if !super::outfit_folders::container(inv, parent) {
                        return Err(aurora_i18n::tr!("world-outfit-folders-cannot-move-here").into());
                    }
                    (Some(name), None, Some(parent))
                }
                Action::SaveTo(id) => {
                    require_outfit(inv, id)?;
                    (None, Some(id), my)
                }
                _ => unreachable!("save action"),
            };
            // Include attachments that arrived before their COF link, as Firestorm does.
            for id in worn.keys() {
                if !current.iter().any(|l| l.target == *id) {
                    let it = inv
                        .items
                        .get(id)
                        .ok_or(aurora_i18n::tr!("world-appearance-a-worn-object-is-not-yet-loaded-in-inventory"))?;
                    current.push(item_link(it));
                }
            }
            let links: Vec<_> = current.iter().filter(|l| !l.folder).cloned().collect();
            if let Some(name) = name {
                let name = name.trim();
                if name.is_empty() || name.chars().count() > 63 {
                    return Err(aurora_i18n::tr!("world-appearance-choose-a-name-of-1-to-63-characters").into());
                }
                let parent = parent.ok_or(aurora_i18n::tr!("world-appearance-the-my-outfits-folder-is-unavailable"))?;
                let id = Uuid::new_v4();
                change.folder = id;
                change.create = Some(InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: FT_OUTFIT,
                    version: 1,
                    ..Default::default()
                });
                change.links = links.clone();
                let mut cof_links = links;
                cof_links.push(OutfitLink {
                    target: id,
                    name: name.into(),
                    desc: String::new(),
                    folder: true,
                    inv_type: 8,
                });
                change.cof = Some((cof, cof_links));
            } else {
                change.folder = destination.ok_or(aurora_i18n::tr!("world-appearance-save-the-outfit-with-save-as-first"))?;
                require_complete(inv, change.folder)?;
                change.links = links;
                change.links.extend(
                    folder_links(inv, change.folder)
                        .into_iter()
                        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 0)),
                );
                current.retain(|l| !l.folder);
                current.push(base_link(inv, change.folder));
                change.cof = Some((cof, current.clone()));
                sync = false;
            }
        }
        Action::Wear(..) | Action::WearOutfits { .. } | Action::WearContents { .. } => {
            let (folders, append, replace_items, contents) = match action {
                Action::Wear(id, append) => (vec![id], append, false, false),
                Action::WearOutfits { folders, append } => {
                    if folders.is_empty() {
                        return Err(aurora_i18n::tr!("world-appearance-the-outfit-folder-is-unavailable").into());
                    }
                    for id in &folders {
                        require_outfit(inv, *id)?;
                    }
                    (folders, append, false, false)
                }
                Action::WearContents { folder, mode } => (
                    vec![folder],
                    !matches!(mode, WearMode::ReplaceOutfit),
                    matches!(mode, WearMode::ReplaceItems),
                    true,
                ),
                _ => unreachable!(),
            };
            // Collect every selected folder before one COF mutation. Sending
            // separate wears loses later selections while the first is pending.
            let mut links = Vec::new();
            let mut seen = HashSet::new();
            for id in &folders {
                if !seen.insert(*id) {
                    continue;
                }
                require_complete(inv, *id)?;
                links.extend(if contents {
                    wearable_contents(inv, *id)?
                } else {
                    folder_links(inv, *id)
                });
            }
            let mut next: Vec<_> = links
                .into_iter()
                .filter(|l| !l.folder && inv.items.get(&l.target).is_none_or(|it| it.asset_type != 0))
                .collect();
            if next
                .iter()
                .any(|l| inv.items.get(&l.target).is_none_or(|it| !matches!(it.asset_type, 5 | 6 | 13 | 21)))
            {
                return Err(aurora_i18n::tr!("world-appearance-this-outfit-contains-a-link-that-cannot-be-worn").into());
            }
            if seen.len() > 1 {
                // LLAppearanceMgr::removeDuplicateItems / filterWearableItems
                // (indra/newview/llappearancemgr.cpp) keep the last original
                // and body part of each type. Combine in displayed order.
                let mut targets = HashSet::new();
                let mut body = HashSet::new();
                next.reverse();
                next.retain(|link| {
                    targets.insert(link.target)
                        && inv
                            .items
                            .get(&link.target)
                            .is_none_or(|item| item.asset_type != 13 || body.insert(item.flags & 0xff))
                });
                next.reverse();
            }
            for old in current.iter().filter(|l| !l.folder) {
                let preserve = (append
                    && (!replace_items
                        || inv.items.get(&old.target).is_none_or(|it| {
                            it.asset_type != 5
                                || !next.iter().any(|n| {
                                    inv.items
                                        .get(&n.target)
                                        .is_some_and(|n| n.asset_type == 5 && n.flags & 0xff == it.flags & 0xff)
                                })
                        })))
                    || inv.items.get(&old.target).is_some_and(|it| {
                        it.asset_type == 13
                            && !next.iter().any(|l| {
                                inv.items
                                    .get(&l.target)
                                    .is_some_and(|n| n.asset_type == 13 && n.flags & 0xff == it.flags & 0xff)
                            })
                    });
                if preserve && !next.iter().any(|l| l.target == old.target) {
                    next.push(old.clone());
                }
            }
            // One body part per type: new body parts replace existing ones even when adding.
            let mut body = HashSet::new();
            let mut targets = HashSet::new();
            next.retain(|l| {
                targets.insert(l.target)
                    && inv
                        .items
                        .get(&l.target)
                        .is_none_or(|it| it.asset_type != 13 || body.insert(it.flags & 0xff))
            });
            if let Some(base) = if append {
                base(inv)
            } else {
                // A combination has no single saved outfit to overwrite.
                (seen.len() == 1)
                    .then_some(folders[0])
                    .filter(|id| inv.folders.get(id).is_some_and(|f| f.info.type_default == FT_OUTFIT))
            } {
                next.push(base_link(inv, base));
            }
            change.links = next;
        }
        Action::Revert => {
            return plan(
                inv,
                Action::Wear(
                    base(inv).ok_or(aurora_i18n::tr!("world-appearance-no-saved-outfit-to-restore"))?,
                    false,
                ),
                worn,
            );
        }
        Action::Remove(_) | Action::RemoveItems(_) => {
            let ids = match action {
                Action::Remove(id) => vec![id],
                Action::RemoveItems(ids) => ids,
                _ => unreachable!(),
            };
            let mut removable = HashSet::new();
            for id in ids {
                let id = super::inventory::actions::original(inv, id)
                    .ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
                let it = inv
                    .items
                    .get(&id)
                    .ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
                if it.asset_type == 13 {
                    return Err(aurora_i18n::tr!("world-appearance-a-body-part-must-be-replaced-it-cannot-be").into());
                }
                if !matches!(it.asset_type, 5 | 6 | 21) {
                    return Err(aurora_i18n::tr!("world-appearance-this-item-cannot-be-taken-off").into());
                }
                removable.insert(id);
            }
            current.retain(|l| l.folder || !removable.contains(&l.target));
            change.links = current;
        }
        Action::RemoveOutfit(_) | Action::RemoveOutfits(_) => {
            let folders = match action {
                Action::RemoveOutfit(id) => vec![id],
                Action::RemoveOutfits(ids) => ids,
                _ => unreachable!(),
            };
            // takeOffOutfit never removes the four required body parts.
            let mut removable = HashSet::new();
            for id in folders {
                require_outfit(inv, id)?;
                require_complete(inv, id)?;
                removable.extend(
                    wearable_contents(inv, id)?
                        .iter()
                        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type != 13))
                        .map(|l| l.target),
                );
            }
            current.retain(|l| l.folder || !removable.contains(&l.target));
            change.links = current;
        }
        Action::Add(_) | Action::WearItem { .. } | Action::WearItems { .. } => {
            // LLInventoryAction::doToSelected handles wear_multiple in bulk:
            // one COF update must include the entire selection before syncing.
            let (ids, replace, point) = match action {
                Action::WearItem { item, replace, point } => (vec![item], replace, point),
                Action::WearItems { items, replace, point } => (items, replace, point),
                Action::Add(id) => (vec![id], false, 0),
                _ => unreachable!(),
            };
            let mut seen = HashSet::new();
            let mut replaced_clothing = HashSet::new();
            for id in ids {
                let id = super::inventory::actions::original(inv, id)
                    .ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
                if !seen.insert(id) {
                    continue;
                }
                let it = inv
                    .items
                    .get(&id)
                    .ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
                if !matches!(it.asset_type, 5 | 6 | 13 | 21)
                    || super::inventory::actions::library(inv, id)
                    || super::inventory::actions::in_type(inv, id, 14)
                {
                    return Err(aurora_i18n::tr!("world-appearance-this-item-cannot-be-worn").into());
                }
                if it.asset_type == 13 || (replace && it.asset_type == 5 && replaced_clothing.insert(it.flags & 0xff)) {
                    current.retain(|l| {
                        l.folder
                            || inv
                                .items
                                .get(&l.target)
                                .is_none_or(|old| old.asset_type != it.asset_type || old.flags & 0xff != it.flags & 0xff)
                    });
                }
                if replace && it.asset_type == 6 {
                    // Point zero uses the last saved point (LLObjectBridge::mAttachPt).
                    let point = if point == 0 { (it.flags & 0xff) as u8 } else { point };
                    if point != 0 {
                        current.retain(|l| {
                            l.folder
                                || l.target == id
                                || inv.items.get(&l.target).is_none_or(|old| {
                                    old.asset_type != 6 || worn.get(&l.target).copied().unwrap_or((old.flags & 0xff) as u8) != point
                                })
                        });
                    }
                }
                if !current.iter().any(|l| l.target == id) {
                    current.push(item_link(it));
                }
            }
            change.links = current;
        }
        Action::ReplaceWearable { old, new } => {
            let original = inv.items.get(&old).ok_or(aurora_i18n::tr!("common-item-not-found"))?;
            let replacement = inv.items.get(&new).ok_or(aurora_i18n::tr!("common-item-not-found"))?;
            if original.asset_type != replacement.asset_type
                || original.flags & 0xff != replacement.flags & 0xff
                || !matches!(original.asset_type, 5 | 13)
                || !current.iter().any(|link| !link.folder && link.target == old)
                || super::inventory::actions::library(inv, new)
                || super::inventory::actions::in_type(inv, new, 14)
            {
                return Err(aurora_i18n::tr!("wearable-save-denied").into());
            }
            for link in current.iter_mut().filter(|link| !link.folder && link.target == old) {
                link.target = new;
                link.name.clone_from(&replacement.name);
            }
            change.links = current;
        }
        Action::MoveLayer(id, _) | Action::PlaceLayer(id, _) => {
            let it = inv
                .items
                .get(&id)
                .ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
            if it.asset_type != 5 {
                return Err(aurora_i18n::tr!("world-appearance-only-clothing-layers-can-be-reordered").into());
            }
            let mut layers: Vec<_> = current
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    inv.items
                        .get(&l.target)
                        .is_some_and(|n| n.asset_type == 5 && n.flags & 0xff == it.flags & 0xff)
                })
                .map(|(i, l)| (i, l.desc.clone(), l.target))
                .collect();
            layers.sort_by(|a, b| a.1.cmp(&b.1));
            if let Some(pos) = layers.iter().position(|l| l.2 == id) {
                let to = match action {
                    Action::MoveLayer(_, true) => pos.checked_sub(1),
                    Action::MoveLayer(_, false) => (pos + 1 < layers.len()).then_some(pos + 1),
                    Action::PlaceLayer(_, index) => Some(index.min(layers.len() - 1)),
                    _ => None,
                };
                if let Some(to) = to {
                    let layer = layers.remove(pos);
                    layers.insert(to, layer);
                }
                for (order, (i, _, _)) in layers.iter().enumerate() {
                    current[*i].desc = format!("@{}", (it.flags & 0xff) * 100 + order as u32);
                }
            }
            change.links = current;
        }
        Action::ShowOriginal(_) | Action::Favorite(_) | Action::Category(..) | Action::Folders(_) | Action::DeleteFromOutfit { .. } => {
            return Err(aurora_i18n::tr!("world-appearance-this-action-does-not-change-the-current-outfit").into());
        }
    }
    normalize_layers(inv, &mut change.links);
    if let Some((_, links)) = &mut change.cof {
        normalize_layers(inv, links);
    }
    if change.create.is_none()
        && change.folder != cof
        && let Some((_, links)) = &change.cof
    {
        if *links == original_current {
            change.cof = None;
        } else {
            sync = true;
        }
    }
    let count = change
        .links
        .iter()
        .filter(|l| !l.folder && inv.items.get(&l.target).is_some_and(|it| it.asset_type == 6))
        .count();
    if sync && count > 38 {
        return Err(aurora_i18n::tr!("world-appearance-the-outfit-exceeds-the-38-attachment-limit").into());
    }
    for kind in 4..=16 {
        if sync
            && change
                .links
                .iter()
                .filter(|l| {
                    !l.folder
                        && inv
                            .items
                            .get(&l.target)
                            .is_some_and(|it| it.asset_type == 5 && it.flags & 0xff == kind)
                })
                .count()
                > 60
        {
            return Err(aurora_i18n::tr!("world-appearance-the-outfit-exceeds-the-limit-of-60-layers-per").into());
        }
    }
    Ok((change, sync))
}

fn require_outfit(inv: &Inventory, id: Uuid) -> Result<(), String> {
    if inv
        .folders
        .get(&id)
        .is_some_and(|f| f.info.type_default == FT_OUTFIT && !f.library && super::outfit_folders::container(inv, f.info.parent))
    {
        Ok(())
    } else {
        Err(aurora_i18n::tr!("world-appearance-this-outfit-folder-cannot-be-modified").into())
    }
}

pub fn category_plan(
    inv: &Inventory,
    id: Uuid,
    mut update: aurora_net::outfits::categories::Update,
) -> Result<aurora_net::outfits::categories::Mutation, String> {
    use aurora_net::outfits::categories::Update;
    require_outfit(inv, id)?;
    match &mut update {
        Update::Rename(name) => {
            *name = name.trim().into();
            if name.is_empty() || name.chars().count() > 63 || name.chars().any(char::is_control) {
                return Err(aurora_i18n::tr!("world-appearance-choose-a-name-of-1-to-63-characters").into());
            }
        }
        Update::Trash(parent) => {
            let cof = cof(inv).ok_or(aurora_i18n::tr!("world-appearance-the-current-outfit-folder-is-unavailable"))?;
            require_complete(inv, cof)?;
            if base(inv) == Some(id) {
                return Err(aurora_i18n::tr!("world-appearance-the-current-outfit-cannot-be-deleted").into());
            }
            if Some(*parent) != system_folder(inv, 14) {
                return Err(aurora_i18n::tr!("world-appearance-the-trash-folder-is-unavailable").into());
            }
        }
        _ => {}
    }
    Ok(aurora_net::outfits::categories::Mutation {
        folder: id,
        parent: inv.folders.get(&id).map(|f| f.info.parent).unwrap_or_default(),
        update,
    })
}

fn normalize_layers(inv: &Inventory, links: &mut [OutfitLink]) {
    for kind in 4..=16 {
        let mut indices: Vec<_> = links
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                !l.folder
                    && inv
                        .items
                        .get(&l.target)
                        .is_some_and(|it| it.asset_type == 5 && it.flags & 0xff == kind)
            })
            .map(|(i, l)| {
                let order = l
                    .desc
                    .strip_prefix('@')
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|order| order / 100 == kind)
                    .unwrap_or(u32::MAX);
                (i, order)
            })
            .collect();
        indices.sort_by_key(|(_, order)| *order);
        for (order, (i, _)) in indices.iter().enumerate() {
            links[*i].desc = format!("@{}", kind * 100 + order as u32);
        }
    }
}

/// LLAppearanceMgr::unregisterAttachment / removeCOFItemLinks
/// (indra/newview/llappearancemgr.cpp): a simulator-confirmed detach removes
/// only the COF links, without triggering another appearance synchronization.
pub fn unlink_attachment(inv: &mut Inventory, item: Uuid) -> Option<aurora_net::inventory::operations::Mutation> {
    use aurora_net::inventory::operations::{Mutation, Operation};
    if item.is_nil() {
        return None;
    }
    let cof = cof(inv)?;
    let removed: Vec<_> = inv
        .folders
        .get(&cof)?
        .items
        .iter()
        .filter_map(|id| inv.items.get(id))
        .filter(|link| link.parent == cof && link.asset_type == 24 && link.asset_id == item)
        .map(|link| link.id)
        .collect();
    if removed.is_empty() {
        return None;
    }
    // Update the local model before the next inventory action can plan a COF
    // replacement. The original and saved outfits remain in the inventory.
    for id in &removed {
        inv.consume_item(*id);
    }
    if let Some(folder) = inv.folders.get_mut(&cof)
        && folder.info.version >= 0
    {
        // LLInventoryModel::accountForUpdate: each AIS removal advances the
        // parent version. Older in-flight folder snapshots must not undo it.
        folder.info.version = folder.info.version.saturating_add(removed.len().min(i32::MAX as usize) as i32);
    }
    Some(Mutation {
        operations: removed.iter().map(|id| Operation::Delete { id: *id, folder: false }).collect(),
        refresh: vec![cof],
        removed,
        ..Default::default()
    })
}

/// What a COF change asks of the region, as
/// LLAppearanceMgr::updateAppearanceFromCOF: the commands to send now, and
/// the objects the COF links to whose items the inventory does not know yet
/// (a cold inventory cache), to wear once looked up
/// (`OutfitRestore::wear_when_known`).
#[derive(Debug, Default)]
pub struct CofSync {
    pub commands: Vec<NetCommand>,
    pub unknown: Vec<Uuid>,
}

pub fn sync_plan(inv: &Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> CofSync {
    let Some(cof) = cof(inv) else {
        return CofSync::default();
    };
    // the links to objects, with those whose item is not known yet: a worn
    // object linked in the COF stays on even before its item is fetched
    let objects: HashSet<_> = super::outfit::cof_objects(inv, cof).into_iter().collect();
    let detach: Vec<_> = worn.difference(&objects).copied().collect();
    let mut unknown = Vec::new();
    let mut attach = Vec::new();
    for id in objects.difference(worn) {
        match inv.items.get(id) {
            Some(it) => attach.push(super::outfit::attach_request(it, agent)),
            None => unknown.push(*id),
        }
    }
    if !detach.is_empty() || !attach.is_empty() || !unknown.is_empty() {
        let wearing: Vec<Uuid> = attach.iter().map(|a| a.item_id).collect();
        log::info!(
            "outfit: COF changed: wearing {} [{}], taking off {} [{}], {} to look up first",
            attach.len(),
            super::outfit::names(inv, &wearing),
            detach.len(),
            super::outfit::names(inv, &detach),
            unknown.len()
        );
    }
    let mut commands = Vec::new();
    if !detach.is_empty() {
        commands.push(NetCommand::DetachAttachments(detach));
    }
    if !attach.is_empty() {
        commands.push(NetCommand::RezAttachments(attach));
    }
    if let Some(folder) = inv.folders.get(&cof) {
        commands.push(NetCommand::RequestServerAppearance {
            cof_version: folder.info.version,
        });
    }
    CofSync { commands, unknown }
}

/// The commands of `sync_plan` alone (demo and tests: everything is known).
pub fn sync_commands(inv: &Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> Vec<NetCommand> {
    sync_plan(inv, agent, worn).commands
}

/// Explicit attachment requests retain the chosen point and replace/add mode.
/// They are sent only after the COF update has been acknowledged.
pub fn attachments_for_action(inv: &Inventory, agent: Uuid, action: &Action) -> Vec<AttachRequest> {
    let (items, point, replace) = match action {
        Action::WearItem { item, point, replace } => (std::slice::from_ref(item), *point, *replace),
        Action::WearItems { items, point, replace } => (items.as_slice(), *point, *replace),
        _ => return Vec::new(),
    };
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(|id| super::inventory::actions::original(inv, *id))
        .filter(|id| seen.insert(*id))
        .filter_map(|id| inv.items.get(&id).filter(|it| it.asset_type == 6))
        .map(|it| AttachRequest {
            item_id: it.id,
            owner_id: if it.owner.is_nil() { agent } else { it.owner },
            point,
            add: !replace,
            flags: it.flags,
            group_mask: it.group_mask,
            everyone_mask: it.everyone_mask,
            next_owner_mask: it.next_owner_mask,
            name: it.name.clone(),
            desc: it.desc.clone(),
        })
        .collect()
}

pub fn apply_attachment_override(commands: &mut Vec<NetCommand>, attachment: AttachRequest) {
    let mut attachment = Some(attachment);
    for cmd in commands.iter_mut() {
        if let NetCommand::RezAttachments(items) = cmd
            && let Some(attachment) = attachment.take()
        {
            items.retain(|it| it.item_id != attachment.item_id);
            items.push(attachment);
        }
    }
    if let Some(attachment) = attachment {
        let pos = commands
            .iter()
            .position(|cmd| matches!(cmd, NetCommand::RequestServerAppearance { .. }))
            .unwrap_or(commands.len());
        commands.insert(pos, NetCommand::RezAttachments(vec![attachment]));
    }
}

/// Offline fixture; no grid, assets or real inventory are read.
pub fn seed_demo(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::Folder;
    let my = system_folder(inv, FT_MY_OUTFITS).unwrap_or_else(|| Uuid::from_u128(700));
    let cof = Uuid::from_u128(701);
    let outfits: Vec<_> = (702..706).map(Uuid::from_u128).collect();
    let folder = |id, parent, name: &str, kind| Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: kind,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    };
    inv.folders
        .insert(my, folder(my, inv.root, aurora_i18n::tr!("common-my-outfits"), FT_MY_OUTFITS));
    inv.folders
        .insert(cof, folder(cof, inv.root, aurora_i18n::tr!("common-current-outfit"), 46));
    let trash = system_folder(inv, 14).unwrap_or_else(|| Uuid::from_u128(706));
    inv.folders
        .insert(trash, folder(trash, inv.root, aurora_i18n::tr!("common-trash"), 14));
    for (id, name) in outfits.iter().zip([
        aurora_i18n::tr!("world-appearance-empty"),
        aurora_i18n::tr!("world-appearance-cop"),
        "Neurolab",
        aurora_i18n::tr!("world-appearance-leather-jacket"),
    ]) {
        inv.folders.insert(*id, folder(*id, my, name, FT_OUTFIT));
    }
    if let Some(f) = inv.folders.get_mut(&my) {
        f.children = outfits.clone();
    }
    let mut items = Vec::new();
    for (i, (name, kind, flags)) in [
        (aurora_i18n::tr!("world-appearance-aurora-silhouette"), 13, 0),
        (aurora_i18n::tr!("world-appearance-aurora-skin"), 13, 1),
        (aurora_i18n::tr!("world-appearance-aurora-hair"), 13, 2),
        (aurora_i18n::tr!("world-appearance-aurora-eyes"), 13, 3),
        (aurora_i18n::tr!("world-appearance-star-tattoo"), 5, 14),
        (aurora_i18n::tr!("world-appearance-leather-jacket-2"), 5, 8),
        (aurora_i18n::tr!("world-appearance-black-pants"), 5, 5),
        (aurora_i18n::tr!("world-appearance-mesh-hair"), 6, 2),
        (aurora_i18n::tr!("world-appearance-earrings"), 6, 13),
        (aurora_i18n::tr!("world-appearance-silver-ring"), 6, 41),
        (aurora_i18n::tr!("world-appearance-boots"), 6, 7),
        (aurora_i18n::tr!("common-demo-hud"), 6, 32),
        (aurora_i18n::tr!("world-appearance-spare-shirt"), 5, 4),
        (aurora_i18n::tr!("world-appearance-spare-attachment"), 6, 5),
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::from_u128(710 + i as u128);
        items.push(InvItem {
            id,
            parent: inv.root,
            name: name.into(),
            desc: String::new(),
            asset_type: kind,
            inv_type: if kind == 6 { 6 } else { 18 },
            asset_id: Uuid::nil(),
            flags,
            favorite: false,
            favorite_known: false,
            creator: agent,
            created_at: 0,
            owner: agent,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            thumbnail: Uuid::nil(),
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            last_owner: uuid::Uuid::nil(),
            group_id: uuid::Uuid::nil(),
            group_owned: false,
            sale_type: 0,
            sale_price: 0,
        });
    }
    inv.add_items(items.clone());
    let all: Vec<_> = items.iter().take(12).map(item_link).collect();
    for (i, id) in outfits.iter().enumerate() {
        let mut selected = match i {
            0 => Vec::new(),
            1 => all.iter().take(7).cloned().collect(),
            2 => all.iter().take(9).cloned().collect(),
            _ => all.iter().take(11).cloned().collect(),
        };
        if i == 2
            && let Some(item) = items.last()
        {
            selected.push(item_link(item));
        }
        demo_replace(inv, agent, *id, &selected);
    }
    let mut current = all;
    current.push(base_link(inv, outfits[3]));
    demo_replace(inv, agent, cof, &current);
}

/// `AURORA_DEMO_APPEARANCE=layers…`: two more tattoos worn over the star
/// tattoo of `seed_demo`, for the layer order of the outfit editor.
pub fn seed_demo_layers(inv: &mut Inventory, agent: Uuid) {
    let cof = Uuid::from_u128(701);
    let tattoos: Vec<_> = [(724, "Freckles"), (725, "Lipstick")]
        .into_iter()
        .map(|(n, name)| InvItem {
            id: Uuid::from_u128(n),
            parent: inv.root,
            name: name.into(),
            asset_type: 5,
            inv_type: 18,
            flags: 14,
            creator: agent,
            owner: agent,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        })
        .collect();
    inv.add_items(tattoos.clone());
    let mut links = folder_links(inv, cof);
    links.extend(tattoos.iter().map(item_link));
    // the star tattoo closest to the body, then Freckles, then Lipstick
    for (order, id) in [714, 724, 725].into_iter().enumerate() {
        if let Some(link) = links.iter_mut().find(|l| l.target == Uuid::from_u128(id)) {
            link.desc = format!("@{}", 1400 + order);
        }
    }
    demo_replace(inv, agent, cof, &links);
}

/// All wearable icons and ordering in Appearance, with three tattoo layers
/// and attachments: `AURORA_DEMO_APPEARANCE=wearables[-outfits|-edit]`.
pub fn seed_demo_wearables(inv: &mut Inventory, agent: Uuid) {
    seed_demo_layers(inv, agent);
    let Some(cof) = cof(inv) else { return };
    let mut links = folder_links(inv, cof);
    for (kind, name) in wearables().iter().enumerate() {
        if links.iter().any(|link| {
            inv.items
                .get(&link.target)
                .is_some_and(|it| matches!(it.asset_type, 5 | 13) && it.flags & 0xff == kind as u32)
        }) {
            continue;
        }
        let item = InvItem {
            id: Uuid::from_u128(730 + kind as u128),
            parent: inv.root,
            name: (*name).into(),
            asset_type: if kind < 4 { 13 } else { 5 },
            inv_type: 18,
            flags: kind as u32,
            creator: agent,
            owner: agent,
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            ..Default::default()
        };
        links.push(item_link(&item));
        inv.add_items(vec![item]);
    }
    if let Some(outfit) = base(inv) {
        let saved: Vec<_> = links.iter().filter(|link| !link.folder).cloned().collect();
        demo_replace(inv, agent, outfit, &saved);
    }
    demo_replace(inv, agent, cof, &links);
}

/// `AURORA_DEMO_OUTFITS=<count>[,closed][,scroll][,locate]`: the number of generated
/// outfits added to the Appearance demo (a large wardrobe), 0 without it.
pub fn demo_outfit_count() -> usize {
    std::env::var("AURORA_DEMO_OUTFITS")
        .ok()
        .and_then(|v| v.split(',').next().and_then(|n| n.trim().parse().ok()))
        .unwrap_or(0)
}

/// A mode of `AURORA_DEMO_OUTFITS` (`closed`, `scroll`, `locate`).
pub fn demo_outfit_mode(mode: &str) -> bool {
    std::env::var("AURORA_DEMO_OUTFITS").is_ok_and(|v| v.split(',').skip(1).any(|m| m.trim() == mode))
}

/// A large wardrobe for the Appearance demo: `count` outfits with generated
/// names, 20 to 60 links each to a pool of generated clothes, objects and
/// body parts, and a thumbnail (never loaded offline) on one in four.
pub fn seed_demo_many(inv: &mut Inventory, agent: Uuid, count: usize) {
    use super::inventory::Folder;
    let Some(my) = system_folder(inv, FT_MY_OUTFITS) else {
        return;
    };
    const STYLES: [&str; 10] = [
        "Casual", "Formal", "Beach", "Winter", "Club", "Gothic", "Office", "Sport", "Party", "Lounge",
    ];
    const KINDS: [&str; 6] = ["Night", "Day", "Look", "Style", "Fit", "Set"];
    let pool: Vec<InvItem> = (0..480u128)
        .map(|i| {
            let (asset_type, inv_type, name) = match i % 8 {
                0 => (13, 18, "Body part"),
                1..=4 => (5, 18, "Clothing"),
                _ => (6, 6, "Attachment"),
            };
            InvItem {
                id: Uuid::from_u128(1_900_000 + i),
                parent: inv.root,
                name: format!("{name} {i:03}"),
                asset_type,
                inv_type,
                creator: agent,
                owner: agent,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                ..Default::default()
            }
        })
        .collect();
    inv.add_items(pool.clone());
    let mut contents = Vec::with_capacity(count);
    let mut outfits = Vec::with_capacity(count);
    for i in 0..count {
        let id = Uuid::from_u128(2_000_000 + i as u128);
        let name = format!("{} {} {:04}", STYLES[i % STYLES.len()], KINDS[(i / 7) % KINDS.len()], i);
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent: my,
                    name,
                    type_default: FT_OUTFIT,
                    version: 1,
                    thumbnail: if i % 4 == 0 {
                        Uuid::from_u128(3_000_000 + i as u128)
                    } else {
                        Uuid::nil()
                    },
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        outfits.push(id);
        let links = 20 + (i * 7) % 41;
        let items = (0..links)
            .map(|k| {
                let target = &pool[(i * 31 + k * 13) % pool.len()];
                InvItem {
                    id: Uuid::from_u128(4_000_000 + (i as u128) * 64 + k as u128),
                    parent: id,
                    name: target.name.clone(),
                    asset_type: 24,
                    inv_type: target.inv_type,
                    asset_id: target.id,
                    creator: agent,
                    owner: agent,
                    base_mask: 0x7fffffff,
                    owner_mask: 0x7fffffff,
                    ..Default::default()
                }
            })
            .collect();
        contents.push(aurora_net::inventory::FolderContents {
            folder_id: id,
            owner_id: agent,
            version: 2,
            folders: Vec::new(),
            items,
        });
    }
    if let Some(f) = inv.folders.get_mut(&my) {
        f.children.extend(outfits);
    }
    inv.apply(contents);
}

/// Replay a folder response with repeated links and originals from other parents.
pub fn seed_duplicate_response(inv: &mut Inventory, agent: Uuid) {
    for folder in [Uuid::from_u128(704), Uuid::from_u128(701)] {
        let links: Vec<_> = inv
            .folders
            .get(&folder)
            .into_iter()
            .flat_map(|f| &f.items)
            .filter_map(|id| inv.items.get(id))
            .cloned()
            .collect();
        let mut items = links.clone();
        for mut link in links {
            if let Some(original) = inv.items.get(&link.asset_id) {
                items.push(original.clone());
            }
            link.id = Uuid::new_v4();
            items.push(link);
        }
        inv.apply(vec![aurora_net::inventory::FolderContents {
            folder_id: folder,
            owner_id: agent,
            version: 3,
            folders: Vec::new(),
            items,
        }]);
    }
}

pub(super) fn demo_replace(inv: &mut Inventory, agent: Uuid, id: Uuid, links: &[OutfitLink]) {
    let old = inv.folders.get(&id).map(|f| f.items.clone()).unwrap_or_default();
    for id in old {
        if inv.items.get(&id).is_some_and(|it| matches!(it.asset_type, 24 | 25)) {
            inv.items.remove(&id);
        }
    }
    let items: Vec<_> = links
        .iter()
        .map(|l| InvItem {
            id: Uuid::new_v4(),
            parent: id,
            name: l.name.clone(),
            desc: l.desc.clone(),
            asset_type: if l.folder { 25 } else { 24 },
            inv_type: l.inv_type,
            asset_id: l.target,
            flags: 0,
            favorite: false,
            favorite_known: false,
            creator: agent,
            created_at: 0,
            owner: agent,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            thumbnail: Uuid::nil(),
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            last_owner: uuid::Uuid::nil(),
            group_id: uuid::Uuid::nil(),
            group_owned: false,
            sale_type: 0,
            sale_price: 0,
        })
        .collect();
    inv.apply(vec![aurora_net::inventory::FolderContents {
        folder_id: id,
        owner_id: agent,
        version: inv.folders.get(&id).map(|f| f.info.version + 1).unwrap_or(1),
        folders: Vec::new(),
        items,
    }]);
}

pub fn demo_mutate(
    inv: &mut Inventory,
    agent: Uuid,
    change: &OutfitMutation,
) -> Result<Vec<aurora_net::inventory::FolderContents>, String> {
    if let Some(cat) = &change.create {
        inv.folders.insert(
            cat.id,
            super::inventory::Folder {
                info: cat.clone(),
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(parent) = inv.folders.get_mut(&cat.parent) {
            parent.children.push(cat.id);
        }
    }
    demo_replace(inv, agent, change.folder, &change.links);
    if let Some((cof, links)) = &change.cof {
        demo_replace(inv, agent, *cof, links);
    }
    Ok(Vec::new())
}

pub fn demo_category(
    inv: &mut Inventory,
    agent: Uuid,
    change: &aurora_net::outfits::categories::Mutation,
) -> Result<Vec<aurora_net::inventory::FolderContents>, String> {
    let folder = inv
        .folders
        .get_mut(&change.folder)
        .ok_or(aurora_i18n::tr!("world-appearance-outfit-not-found"))?;
    change.update.apply(&mut folder.info);
    let mut parents = vec![change.parent];
    if folder.info.parent != change.parent {
        parents.push(folder.info.parent);
    }
    Ok(parents
        .into_iter()
        .map(|parent| aurora_net::inventory::FolderContents {
            folder_id: parent,
            owner_id: agent,
            version: inv.folders.get(&parent).map_or(1, |f| f.info.version + 1),
            folders: inv
                .folders
                .values()
                .filter(|f| f.info.parent == parent)
                .map(|f| f.info.clone())
                .collect(),
            items: inv.items.values().filter(|it| it.parent == parent).cloned().collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Inventory {
        let mut inv = Inventory::default();
        seed_demo(&mut inv, Uuid::from_u128(1));
        inv
    }

    #[test]
    fn tattoo_layers_move_one_step_or_to_their_drop() {
        let mut inv = fixture();
        seed_demo_layers(&mut inv, Uuid::from_u128(1));
        let (star, freckles, lipstick) = (Uuid::from_u128(714), Uuid::from_u128(724), Uuid::from_u128(725));
        // the tattoos closest to the body first
        let order = |action| {
            let (change, _) = plan(&inv, action, &HashMap::new()).expect("reorder");
            let mut tattoos: Vec<_> = change
                .links
                .iter()
                .filter(|l| inv.items.get(&l.target).is_some_and(|it| it.flags & 0xff == 14))
                .map(|l| (l.desc.clone(), l.target))
                .collect();
            tattoos.sort();
            tattoos.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
        };
        assert_eq!(order(Action::MoveLayer(lipstick, true)), [star, lipstick, freckles]);
        assert_eq!(order(Action::MoveLayer(star, false)), [freckles, star, lipstick]);
        // already at the edge: nothing moves
        assert_eq!(order(Action::MoveLayer(star, true)), [star, freckles, lipstick]);
        assert_eq!(order(Action::PlaceLayer(lipstick, 0)), [lipstick, star, freckles]);
        assert_eq!(order(Action::PlaceLayer(star, 9)), [freckles, lipstick, star]);
        // a body part is never reordered
        assert!(plan(&inv, Action::PlaceLayer(Uuid::from_u128(711), 0), &HashMap::new()).is_err());
    }

    #[test]
    fn context_take_off_clothing_uses_confirmed_links_and_never_body_parts() {
        let mut inv = fixture();
        let confirmed = worn_clothing(&inv);
        assert!(!confirmed.is_empty());
        assert!(
            confirmed
                .iter()
                .all(|(id, kind)| inv.items[id].asset_type == 5 && (4..=16).contains(kind))
        );
        let mut unworn = inv.items[&confirmed[0].0].clone();
        unworn.id = Uuid::from_u128(999);
        inv.items.insert(unworn.id, unworn);
        assert_eq!(worn_clothing(&inv), confirmed, "unworn clothing is excluded");
        let (change, _) = plan(
            &inv,
            Action::RemoveItems(confirmed.iter().map(|(id, _)| *id).collect()),
            &HashMap::new(),
        )
        .expect("remove clothing");
        let remaining: Vec<_> = change.links.iter().filter_map(|link| inv.items.get(&link.target)).collect();
        assert!(remaining.iter().any(|item| item.asset_type == 13), "required body parts survive");
        assert!(remaining.iter().all(|item| item.asset_type != 5));
    }

    #[test]
    fn combined_outfits_wear_once_deduplicate_and_preserve_required_parts() {
        let inv = fixture();
        let folders = vec![Uuid::from_u128(705), Uuid::from_u128(704), Uuid::from_u128(705)];
        let (change, sync) = plan(&inv, Action::WearOutfits { folders, append: false }, &HashMap::new()).expect("combined wear");
        assert!(sync);
        assert_eq!(change.folder, cof(&inv).expect("COF"));
        assert!(change.cof.is_none());
        let targets: HashSet<_> = change.links.iter().map(|link| link.target).collect();
        assert_eq!(targets.len(), change.links.len());
        for id in (710..721).chain(std::iter::once(723)) {
            assert!(targets.contains(&Uuid::from_u128(id)), "missing combined item {id}");
        }
        assert!(!targets.contains(&Uuid::from_u128(721)), "replace removes unrelated HUD");
        assert!(
            !change.links.iter().any(|link| link.folder),
            "a combination has no single saved base"
        );
        for kind in 0..4 {
            assert_eq!(
                change
                    .links
                    .iter()
                    .filter(|link| inv.items[&link.target].asset_type == 13 && inv.items[&link.target].flags & 0xff == kind)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn combined_outfits_add_preserves_current_base_and_unlinked_attachments() {
        let mut inv = fixture();
        let mut attachment = inv.items[&Uuid::from_u128(723)].clone();
        attachment.id = Uuid::from_u128(724);
        inv.add_items(vec![attachment]);
        let worn = HashMap::from([(Uuid::from_u128(724), 5)]);
        let (change, sync) = plan(
            &inv,
            Action::WearOutfits {
                folders: vec![Uuid::from_u128(703), Uuid::from_u128(704)],
                append: true,
            },
            &worn,
        )
        .expect("combined add");
        assert!(sync);
        for id in (710..722).chain([723, 724]) {
            assert!(
                change.links.iter().any(|link| link.target == Uuid::from_u128(id)),
                "missing item {id}"
            );
        }
        assert!(change.links.iter().any(|link| link.folder && Some(link.target) == base(&inv)));
    }

    #[test]
    fn combined_outfits_bodypart_conflicts_follow_last_in_display_order() {
        let mut inv = fixture();
        let mut shape = inv.items[&Uuid::from_u128(710)].clone();
        shape.id = Uuid::from_u128(724);
        inv.add_items(vec![shape.clone()]);
        let folder = Uuid::from_u128(704);
        let mut links = folder_links(&inv, folder);
        links.retain(|link| link.target != Uuid::from_u128(710));
        links.push(item_link(&shape));
        demo_replace(&mut inv, Uuid::from_u128(1), folder, &links);
        for (folders, expected) in [
            (vec![Uuid::from_u128(703), folder], shape.id),
            (vec![folder, Uuid::from_u128(703)], Uuid::from_u128(710)),
        ] {
            for append in [false, true] {
                let (change, _) = plan(
                    &inv,
                    Action::WearOutfits {
                        folders: folders.clone(),
                        append,
                    },
                    &HashMap::new(),
                )
                .expect("combined wear");
                let shapes: Vec<_> = change
                    .links
                    .iter()
                    .filter(|link| !link.folder && inv.items[&link.target].asset_type == 13 && inv.items[&link.target].flags & 0xff == 0)
                    .map(|link| link.target)
                    .collect();
                assert_eq!(shapes, vec![expected]);
            }
        }
    }

    #[test]
    fn combined_outfits_enforce_attachment_limit_on_the_complete_selection() {
        let mut inv = fixture();
        let template = inv.items[&Uuid::from_u128(723)].clone();
        let objects: Vec<_> = (8000..8039)
            .map(|id| InvItem {
                id: Uuid::from_u128(id),
                ..template.clone()
            })
            .collect();
        inv.add_items(objects.clone());
        for (id, items) in [Uuid::from_u128(703), Uuid::from_u128(704)].into_iter().zip(objects.chunks(20)) {
            demo_replace(&mut inv, Uuid::from_u128(1), id, &items.iter().map(item_link).collect::<Vec<_>>());
            assert!(plan(&inv, Action::Wear(id, false), &HashMap::new()).is_ok());
        }
        assert!(
            plan(
                &inv,
                Action::WearOutfits {
                    folders: vec![Uuid::from_u128(703), Uuid::from_u128(704)],
                    append: false,
                },
                &HashMap::new()
            )
            .is_err()
        );
    }

    #[test]
    fn combined_outfits_fail_atomically_on_unloaded_missing_or_invalid_folders() {
        for failure in 0..4 {
            let mut inv = fixture();
            let id = Uuid::from_u128(704);
            match failure {
                0 => inv.folders.get_mut(&id).expect("folder").state = FetchState::Fetching,
                1 => {
                    inv.items.remove(&Uuid::from_u128(723));
                }
                2 => inv.folders.get_mut(&id).expect("folder").library = true,
                _ => {
                    inv.folders.remove(&id);
                }
            }
            for append in [false, true] {
                assert!(
                    plan(
                        &inv,
                        Action::WearOutfits {
                            folders: vec![Uuid::from_u128(703), id],
                            append,
                        },
                        &HashMap::new()
                    )
                    .is_err()
                );
            }
        }
        assert!(
            plan(
                &fixture(),
                Action::WearOutfits {
                    folders: vec![],
                    append: false
                },
                &HashMap::new()
            )
            .is_err()
        );
    }

    #[test]
    fn combined_outfits_remove_preserves_body_parts_and_unrelated_items() {
        let inv = fixture();
        let (change, sync) = plan(
            &inv,
            Action::RemoveOutfits(vec![Uuid::from_u128(703), Uuid::from_u128(704)]),
            &HashMap::new(),
        )
        .expect("combined take off");
        assert!(sync);
        for id in [710, 711, 712, 713, 719, 720, 721] {
            assert!(
                change.links.iter().any(|link| link.target == Uuid::from_u128(id)),
                "missing preserved item {id}"
            );
        }
        for id in 714..719 {
            assert!(!change.links.iter().any(|link| link.target == Uuid::from_u128(id)));
        }
    }

    #[test]
    fn outfit_menu_fetches_only_its_subtree_and_retries_failed_folders() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let root = Uuid::from_u128(704);
        let child = Uuid::from_u128(900);
        let untouched = Uuid::from_u128(703);
        let mut nested = inv.folders[&root].clone();
        nested.info.id = child;
        nested.info.parent = root;
        nested.info.type_default = -1;
        nested.children = vec![root]; // malformed cycles must not loop
        nested.state = FetchState::Failed;
        inv.folders.insert(child, nested);
        inv.folders.get_mut(&root).expect("root").children.push(child);
        inv.folders.get_mut(&root).expect("root").state = FetchState::Unknown;
        inv.folders.get_mut(&untouched).expect("other outfit").state = FetchState::Unknown;
        inv.queue.clear();
        let original = inv.items.remove(&Uuid::from_u128(723)).expect("linked original");
        let mut link = inv
            .items
            .values()
            .find(|it| it.parent == root && it.asset_id == original.id)
            .expect("link")
            .clone();
        link.id = Uuid::from_u128(901);
        link.parent = child;
        inv.add_items(vec![link]);
        let commands = prepare_contents(&mut inv, agent, root);
        assert_eq!(inv.queue.len(), 2);
        assert!(inv.queue.contains(&(root, false)));
        assert!(inv.queue.contains(&(child, false)));
        assert_eq!(inv.folders[&untouched].state, FetchState::Unknown);
        assert!(
            matches!(commands.as_slice(), [NetCommand::FetchItems { items, owner }]
            if items == &[original.id] && *owner == agent),
            "shared original is fetched once"
        );
        inv.failed(&[child]);
        inv.queue.clear();
        prepare_contents(&mut inv, agent, root);
        assert_eq!(inv.queue, vec![(child, false)]);
        // A large ordinary folder cannot fill the entire network queue at once.
        for n in 1000..1040 {
            let id = Uuid::from_u128(n);
            let mut folder = inv.folders[&child].clone();
            folder.info.id = id;
            folder.children.clear();
            folder.items.clear();
            folder.state = FetchState::Unknown;
            inv.folders.insert(id, folder);
            inv.folders.get_mut(&root).expect("root").children.push(id);
        }
        prepare_contents(&mut inv, agent, root);
        assert_eq!(inv.queue.len(), 16);
    }

    #[test]
    fn remove_outfit_preserves_body_parts_and_other_outfit_items() {
        let mut inv = fixture();
        let outfit = Uuid::from_u128(704);
        let extra = Uuid::from_u128(723);
        let (add, _) = plan(&inv, Action::Add(extra), &HashMap::new()).expect("extra attachment");
        demo_mutate(&mut inv, Uuid::from_u128(1), &add).expect("add");
        // Removing only Cop keeps Neurolab's extra and all required body parts.
        let selected = Uuid::from_u128(703);
        let (change, sync) = plan(&inv, Action::RemoveOutfit(selected), &HashMap::new()).expect("take off");
        assert!(sync);
        assert!(change.links.iter().any(|l| l.target == extra), "other outfit attachment remains");
        for id in 710..714 {
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(id)), "body part {id}");
        }
        let removable: HashSet<_> = folder_links(&inv, selected)
            .iter()
            .filter(|l| inv.items.get(&l.target).is_some_and(|it| it.asset_type != 13))
            .map(|l| l.target)
            .collect();
        assert!(!removable.is_empty());
        assert!(!change.links.iter().any(|l| !l.folder && removable.contains(&l.target)));
        assert_eq!(base(&inv), Some(Uuid::from_u128(705)));
        assert!(inv.folders.contains_key(&outfit), "saved outfits are retained");
    }

    #[test]
    fn save_to_selected_outfit_updates_its_links_and_the_cof_base() {
        let mut inv = fixture();
        let selected = Uuid::from_u128(702);
        let originals: HashSet<_> = inv
            .items
            .values()
            .filter(|it| !matches!(it.asset_type, 24 | 25))
            .map(|it| it.id)
            .collect();
        let previous = folder_links(&inv, Uuid::from_u128(705));
        let current = folder_links(&inv, cof(&inv).expect("COF"));
        let (change, sync) = plan(&inv, Action::SaveTo(selected), &HashMap::new()).expect("save to selected");
        assert_eq!(change.folder, selected);
        assert!(sync);
        assert!(change.create.is_none());
        assert_eq!(
            change.links.iter().map(|l| l.target).collect::<HashSet<_>>(),
            current.iter().filter(|l| !l.folder).map(|l| l.target).collect::<HashSet<_>>()
        );
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("save");
        assert_eq!(base(&inv), Some(selected));
        assert_eq!(
            folder_links(&inv, Uuid::from_u128(705)),
            previous,
            "previous outfit is not overwritten"
        );
        assert!(originals.iter().all(|id| inv.items.contains_key(id)));
    }

    #[test]
    fn category_changes_protect_current_outfit_and_preserve_originals_in_trash() {
        use aurora_net::outfits::categories::Update;
        let mut inv = fixture();
        let trash = system_folder(&inv, 14).expect("Trash");
        let current = base(&inv).expect("base");
        assert!(category_plan(&inv, current, Update::Trash(trash)).is_err());
        assert!(
            category_plan(
                &inv,
                cof(&inv).expect("COF"),
                Update::Rename(aurora_i18n::tr!("common-name").into())
            )
            .is_err()
        );
        assert!(category_plan(&inv, Uuid::from_u128(704), Update::Rename("  ".into())).is_err());
        let selected = Uuid::from_u128(704);
        let links = folder_links(&inv, selected);
        let change = category_plan(&inv, selected, Update::Trash(trash)).expect("trash selected");
        let contents = demo_category(&mut inv, Uuid::from_u128(1), &change).expect("server response");
        inv.apply(contents);
        assert_eq!(inv.folders[&selected].info.parent, trash);
        assert!(inv.folders[&trash].children.contains(&selected));
        assert!(
            !inv.folders[&system_folder(&inv, FT_MY_OUTFITS).expect("My Outfits")]
                .children
                .contains(&selected)
        );
        assert_eq!(folder_links(&inv, selected), links);
        assert!(links.iter().all(|l| inv.items.contains_key(&l.target)));
        assert_eq!(base(&inv), Some(current));
        inv.folders.get_mut(&cof(&inv).expect("COF")).expect("COF").state = FetchState::Unknown;
        assert!(
            category_plan(&inv, Uuid::from_u128(703), Update::Trash(trash)).is_err(),
            "wait for the current outfit before allowing deletion"
        );
    }

    #[test]
    fn adding_objects_or_clothes_counts_each_original_once() {
        let mut inv = fixture();
        let cof = cof(&inv).expect("COF");
        let mut links = folder_links(&inv, cof);
        for i in 0..20 {
            let mut item = inv.items[&Uuid::from_u128(717)].clone();
            item.id = Uuid::from_u128(1000 + i);
            inv.items.insert(item.id, item.clone());
            links.extend([item_link(&item), item_link(&item)]);
        }
        demo_replace(&mut inv, Uuid::from_u128(1), cof, &links);
        for item in [Uuid::from_u128(722), Uuid::from_u128(723)] {
            let (change, sync) = plan(
                &inv,
                Action::WearItem {
                    item,
                    replace: false,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("duplicate links must not exhaust the attachment limit");
            assert!(sync);
            assert_eq!(change.links.iter().filter(|l| l.target == item).count(), 1);
            let mut seen = HashSet::new();
            assert!(change.links.iter().all(|l| seen.insert((l.folder, l.target))));
            for i in 0..20 {
                assert_eq!(
                    change.links.iter().filter(|l| l.target == Uuid::from_u128(1000 + i)).count(),
                    1,
                    "distinct inventory copies must remain"
                );
            }
        }
    }

    #[test]
    fn duplicate_links_keep_the_last_layer_and_ignore_foreign_originals() {
        let mut inv = fixture();
        let folder = Uuid::from_u128(704);
        let original = inv.items[&Uuid::from_u128(715)].clone();
        let mut children = vec![original.id]; // index left over from an older cache
        for (id, desc) in [(990, "@800"), (991, "@801")] {
            let mut link = original.clone();
            link.id = Uuid::from_u128(id);
            link.parent = folder;
            link.asset_type = 24;
            link.asset_id = original.id;
            link.desc = desc.into();
            children.push(link.id);
            inv.items.insert(link.id, link);
        }
        let mut copy = original.clone();
        copy.id = Uuid::from_u128(992);
        copy.parent = folder;
        children.push(copy.id);
        inv.items.insert(copy.id, copy);
        inv.folders.get_mut(&folder).expect("outfit").items = children;
        let links = folder_links(&inv, folder);
        assert_eq!(links.len(), 2, "a separate inventory copy stays");
        assert_eq!(links.iter().find(|l| l.target == original.id).expect("link").desc, "@801");
    }
    #[test]
    fn replace_empty_outfit_preserves_required_body_parts() {
        let inv = fixture();
        let (change, sync) = plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashMap::new()).expect("plan");
        assert!(sync);
        assert_eq!(change.links.iter().filter(|l| !l.folder).count(), 4);
        assert!(
            change
                .links
                .iter()
                .filter(|l| !l.folder)
                .all(|l| inv.items[&l.target].asset_type == 13)
        );
        assert_eq!(change.links.iter().find(|l| l.folder).map(|l| l.target), Some(Uuid::from_u128(702)));
    }
    #[test]
    fn save_as_uses_links_and_replaces_the_base_outfit() {
        let mut inv = fixture();
        let (change, _) = plan(&inv, Action::Save(Some("  Nouvelle tenue  ".into())), &HashMap::new()).expect("save");
        assert_eq!(change.create.as_ref().expect("category").name, "Nouvelle tenue");
        assert!(change.links.iter().all(|l| !l.folder));
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert_eq!(base(&inv), Some(change.folder));
        assert!(!dirty(&inv));
    }
    #[test]
    fn body_parts_cannot_be_removed_and_unloaded_outfits_cannot_be_worn() {
        let mut inv = fixture();
        assert!(plan(&inv, Action::Remove(Uuid::from_u128(710)), &HashMap::new()).is_err());
        inv.folders.get_mut(&Uuid::from_u128(702)).expect("outfit").state = FetchState::Unknown;
        assert!(plan(&inv, Action::Wear(Uuid::from_u128(702), false), &HashMap::new()).is_err());
    }
    #[test]
    fn adding_a_body_part_replaces_the_existing_type() {
        let mut inv = fixture();
        let mut shape = inv.items[&Uuid::from_u128(710)].clone();
        shape.id = Uuid::from_u128(999);
        inv.items.insert(shape.id, shape);
        let (change, _) = plan(&inv, Action::Add(Uuid::from_u128(999)), &HashMap::new()).expect("add");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(999)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(710)));
    }

    #[test]
    fn saving_updates_clothing_order_in_both_cof_and_base() {
        let mut inv = fixture();
        let (change, sync) = plan(&inv, Action::Save(None), &HashMap::new()).expect("save");
        assert!(sync, "normalizing COF layer order requires a new bake");
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert!(!dirty(&inv), "Save must clear the unsaved-changes indicator");
        let links = folder_links(&inv, cof(&inv).expect("COF"));
        assert_eq!(
            links.iter().find(|l| l.target == Uuid::from_u128(715)).expect("jacket").desc,
            "@800"
        );
    }

    #[test]
    fn removing_one_clothing_layer_keeps_unlinked_worn_attachments() {
        let mut inv = fixture();
        let mut extra = inv.items[&Uuid::from_u128(717)].clone();
        extra.id = Uuid::from_u128(1000);
        inv.items.insert(extra.id, extra);
        let (change, _) = plan(
            &inv,
            Action::Remove(Uuid::from_u128(715)),
            &HashMap::from([(Uuid::from_u128(1000), 5)]),
        )
        .expect("remove");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(1000)));
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(715)));
    }

    #[test]
    fn wear_replaces_same_clothing_type_while_add_preserves_layers() {
        let mut inv = fixture();
        let mut jacket = inv.items[&Uuid::from_u128(715)].clone();
        jacket.id = Uuid::from_u128(999);
        inv.items.insert(jacket.id, jacket);
        for replace in [false, true] {
            let (change, _) = plan(
                &inv,
                Action::WearItem {
                    item: Uuid::from_u128(999),
                    replace,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("wear");
            assert_eq!(change.links.iter().any(|l| l.target == Uuid::from_u128(715)), !replace);
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(999)));
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(710)), "body preserved");
        }
    }

    #[test]
    fn deleting_saved_link_preserves_original_and_current_outfit() {
        let mut inv = fixture();
        let current = folder_links(&inv, cof(&inv).expect("COF"));
        let item = Uuid::from_u128(717);
        let folder = Uuid::from_u128(704);
        let (change, sync) = plan(&inv, Action::DeleteFromOutfit { folder, item }, &HashMap::new()).expect("delete link");
        assert!(!sync, "saved-outfit removal must not detach worn objects");
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        assert!(!folder_links(&inv, folder).iter().any(|l| l.target == item));
        assert!(inv.items.contains_key(&item), "original retained");
        assert_eq!(folder_links(&inv, cof(&inv).expect("COF")), current);
    }

    #[test]
    fn attach_to_hud_keeps_other_objects_and_passes_the_chosen_point() {
        let inv = fixture();
        let agent = Uuid::from_u128(1);
        let item = Uuid::from_u128(723);
        let action = Action::WearItem {
            item,
            replace: false,
            point: 35,
        };
        let (change, _) = plan(&inv, action.clone(), &HashMap::new()).expect("add to HUD");
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(717)));
        let attachment = attachments_for_action(&inv, agent, &action).pop().expect("attachment");
        assert_eq!(attachment.point, 35);
        assert!(attachment.add);
        let duplicate = attachment.clone();
        let mut commands = vec![
            NetCommand::DetachAttachments(vec![Uuid::from_u128(999)]),
            NetCommand::RezAttachments(vec![duplicate]),
            NetCommand::RequestServerAppearance { cof_version: 1 },
        ];
        apply_attachment_override(&mut commands, attachment);
        assert!(matches!(&commands[0], NetCommand::DetachAttachments(_)));
        assert!(
            matches!(&commands[1], NetCommand::RezAttachments(items) if items.len() == 1 && items[0].item_id == item && items[0].point == 35)
        );
        assert!(matches!(commands.last(), Some(NetCommand::RequestServerAppearance { .. })));
    }

    #[test]
    fn replace_attachment_uses_actual_point_when_inventory_flags_are_stale() {
        let inv = fixture();
        let worn = HashMap::from([(Uuid::from_u128(717), 5), (Uuid::from_u128(718), 6)]);
        let (change, _) = plan(
            &inv,
            Action::WearItem {
                item: Uuid::from_u128(723),
                replace: true,
                point: 0,
            },
            &worn,
        )
        .expect("replace point 5");
        assert!(!change.links.iter().any(|l| l.target == Uuid::from_u128(717)));
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(718)));
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(723)));
    }

    #[test]
    fn multiple_objects_are_added_and_attached_together_after_the_cof_reply() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let first = Uuid::from_u128(723);
        let second = Uuid::from_u128(999);
        let alias = Uuid::from_u128(1000);
        let mut extra = inv.items[&first].clone();
        extra.id = second;
        inv.add_items(vec![
            extra,
            InvItem {
                id: alias,
                asset_type: 24,
                asset_id: first,
                ..Default::default()
            },
        ]);
        let worn = HashMap::from([(Uuid::from_u128(717), 5), (Uuid::from_u128(718), 6)]);
        let action = Action::WearItems {
            items: vec![first, second, alias],
            replace: false,
            point: 35,
        };
        let (change, sync) = plan(&inv, action.clone(), &worn).expect("bulk add");
        assert!(sync);
        for id in [first, second] {
            assert_eq!(change.links.iter().filter(|l| l.target == id).count(), 1);
        }
        for id in 710..714 {
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(id)));
        }
        assert!(worn.keys().all(|id| change.links.iter().any(|l| l.target == *id)));
        let overrides = attachments_for_action(&inv, agent, &action);
        assert_eq!(overrides.len(), 2);
        demo_mutate(&mut inv, agent, &change).expect("COF reply");
        let mut commands = sync_commands(&inv, agent, &worn.keys().copied().collect());
        for attachment in overrides {
            apply_attachment_override(&mut commands, attachment);
        }
        let batches: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                NetCommand::RezAttachments(items) => Some(items),
                _ => None,
            })
            .collect();
        assert_eq!(batches.len(), 1);
        for id in [first, second] {
            assert!(batches[0].iter().any(|a| a.item_id == id && a.point == 35 && a.add));
        }
        assert!(matches!(commands.last(), Some(NetCommand::RequestServerAppearance { .. })));
    }

    #[test]
    fn bulk_replace_keeps_every_new_layer_of_each_selected_clothing_type() {
        for kind in [4, 14] {
            let mut inv = fixture();
            let root = inv.root;
            let template = inv.items[&Uuid::from_u128(714)].clone();
            let ids: [Uuid; 3] = [9901, 9902, 9903].map(Uuid::from_u128);
            let items = ids
                .into_iter()
                .map(|id| InvItem {
                    id,
                    parent: root,
                    flags: kind,
                    asset_type: 5,
                    ..template.clone()
                })
                .collect();
            inv.add_items(items);
            let (old, _) = plan(
                &inv,
                Action::WearItem {
                    item: ids[0],
                    replace: true,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("old layer");
            demo_mutate(&mut inv, Uuid::from_u128(1), &old).expect("old COF");
            let (change, _) = plan(
                &inv,
                Action::WearItems {
                    items: ids[1..].to_vec(),
                    replace: true,
                    point: 0,
                },
                &HashMap::new(),
            )
            .expect("bulk replace");
            assert!(!change.links.iter().any(|link| link.target == ids[0]));
            for id in &ids[1..] {
                assert_eq!(change.links.iter().filter(|link| link.target == *id).count(), 1);
            }
        }
    }

    #[test]
    fn multiple_attachments_are_detached_together_including_an_object_missing_from_cof() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let ids: Vec<_> = [717, 718, 723].into_iter().map(Uuid::from_u128).collect();
        let worn = ids.iter().map(|id| (*id, 35)).collect();
        let (change, sync) = plan(&inv, Action::RemoveItems(ids.clone()), &worn).expect("bulk detach");
        assert!(sync);
        assert!(!change.links.iter().any(|l| ids.contains(&l.target)));
        assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(719)));
        for id in 710..714 {
            assert!(change.links.iter().any(|l| l.target == Uuid::from_u128(id)));
        }
        demo_mutate(&mut inv, agent, &change).expect("COF reply");
        let commands = sync_commands(&inv, agent, &ids.iter().copied().collect());
        let batches: Vec<_> = commands
            .iter()
            .filter_map(|c| match c {
                NetCommand::DetachAttachments(items) => Some(items),
                _ => None,
            })
            .collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].iter().copied().collect::<HashSet<_>>(),
            ids.iter().copied().collect::<HashSet<_>>()
        );
        assert!(ids.iter().all(|id| inv.items.contains_key(id)), "originals remain in inventory");
    }

    #[test]
    fn confirmed_detach_removes_duplicate_cof_links_and_rejects_an_older_snapshot() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        seed_duplicate_response(&mut inv, agent);
        let cof = cof(&inv).expect("COF");
        let before = aurora_net::inventory::FolderContents {
            folder_id: cof,
            owner_id: agent,
            version: inv.folders[&cof].info.version,
            folders: Vec::new(),
            items: inv.folders[&cof].items.iter().filter_map(|id| inv.items.get(id).cloned()).collect(),
        };
        let change = unlink_attachment(&mut inv, Uuid::from_u128(717)).expect("unlink");
        assert_eq!(change.removed.len(), 2);
        assert_eq!(inv.folders[&cof].info.version, before.version + 2);
        inv.apply(vec![before]);
        assert!(!folder_links(&inv, cof).iter().any(|link| link.target == Uuid::from_u128(717)));
        assert!(unlink_attachment(&mut inv, Uuid::from_u128(717)).is_none());
        assert!(unlink_attachment(&mut inv, Uuid::nil()).is_none());
        let reply = super::super::inventory::actions::demo_mutate(&mut inv, agent, change).expect("AIS reply");
        inv.remove(&reply.removed);
        inv.apply(reply.contents);
        assert!(!folder_links(&inv, cof).iter().any(|link| link.target == Uuid::from_u128(717)));
    }

    #[test]
    fn a_body_part_in_a_multiple_removal_rejects_the_entire_selection() {
        let inv = fixture();
        let before = folder_links(&inv, cof(&inv).expect("COF"));
        assert!(
            plan(
                &inv,
                Action::RemoveItems(vec![Uuid::from_u128(717), Uuid::from_u128(710)]),
                &HashMap::new()
            )
            .is_err()
        );
        assert_eq!(folder_links(&inv, cof(&inv).expect("COF")), before);
    }

    /// `missing_targets` reads the links in place; it finds what the copied
    /// `folder_links` would: item links whose original is not there.
    /// Replace outfit with a cold inventory cache: the COF links to objects
    /// whose items are not fetched yet. A worn one stays on (it used to be
    /// taken off), the others are left to look up, then worn.
    #[test]
    fn cof_sync_keeps_worn_objects_whose_items_are_not_known_yet() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let cof = cof(&inv).expect("COF");
        let (worn_unknown, unknown, foreign) = (Uuid::from_u128(950), Uuid::from_u128(951), Uuid::from_u128(952));
        inv.add_items(
            [(900, worn_unknown, 6), (901, unknown, 17)]
                .into_iter()
                .map(|(id, target, inv_type)| InvItem {
                    id: Uuid::from_u128(id),
                    parent: cof,
                    asset_type: 24,
                    inv_type,
                    asset_id: target,
                    ..Default::default()
                })
                .collect(),
        );
        let sync = sync_plan(&inv, agent, &[worn_unknown, foreign].into_iter().collect());
        assert_eq!(sync.unknown, vec![unknown]);
        let detached: Vec<_> = sync
            .commands
            .iter()
            .filter_map(|c| match c {
                NetCommand::DetachAttachments(items) => Some(items.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(detached, vec![foreign], "only what the COF does not link comes off");
        assert!(!sync.commands.iter().any(
            |c| matches!(c, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == unknown || a.item_id == worn_unknown))
        ));
    }

    #[test]
    fn missing_targets_are_the_item_links_without_their_original() {
        let mut inv = Inventory::default();
        let agent = Uuid::from_u128(1);
        seed_demo(&mut inv, agent);
        seed_demo_many(&mut inv, agent, 20);
        for i in (0..480u128).step_by(37) {
            inv.items.remove(&Uuid::from_u128(1_900_000 + i));
        }
        let mut any = false;
        for id in prepare_folders(&inv) {
            let expected: HashSet<_> = folder_links(&inv, id)
                .into_iter()
                .filter(|l| !l.folder && !inv.items.contains_key(&l.target))
                .map(|l| l.target)
                .collect();
            let found: HashSet<_> = missing_targets(&inv, id).collect();
            any |= !found.is_empty();
            assert_eq!(found, expected, "folder {id}");
        }
        assert!(any, "some originals are missing");
    }
}
