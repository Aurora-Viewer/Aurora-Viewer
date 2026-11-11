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

/// One item of a `Action::Force` batch, with its own mode and point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WearRequest {
    pub item: Uuid,
    pub replace: bool,
    pub point: u8,
}

#[derive(Clone, Debug)]
pub enum Action {
    /// The image editor on an outfit (the menu's Image…, Outfit.Thumbnail).
    OutfitImage(Uuid),
    /// A snapshot of the scene, set as the outfit's picture at once.
    OutfitSnapshot(Uuid),
    /// The auto portrait of an outfit (`snapshot::portrait`).
    OutfitPortrait(Uuid),
    /// The photo mode, square, for an outfit's picture.
    OutfitPhotoMode(Uuid),
    /// The batch portraits' window, over these outfits (the showcase's,
    /// in its order).
    OutfitBatch(Vec<Uuid>),
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
    ExportJson(Uuid),
    PickJson,
    ImportJson {
        backup: Box<super::outfit_backup::Backup>,
        name: String,
        partial: bool,
    },
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
    /// The removals and wears of RLVa force commands in one COF update,
    /// each wear with its own mode and point (RlvForceWear::done,
    /// rlvhelper.cpp). Removals apply first, then the wears in order.
    Force {
        remove: Vec<Uuid>,
        wear: Vec<WearRequest>,
    },
    ReplaceWearable {
        old: Uuid,
        new: Uuid,
    },
    DeleteFromOutfit {
        folder: Uuid,
        items: Vec<Uuid>,
    },
    ShowOriginal(Uuid),
    /// Activate (true) or deactivate the gestures among favorite rows.
    Gesture(Vec<Uuid>, bool),
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

/// The picture of an outfit (LLOutfitGallery::refreshOutfit): the folder's
/// thumbnail, else the first texture in it or linked from it, its subfolders
/// included. Nil without one.
pub fn outfit_image(inv: &Inventory, outfit: Uuid) -> Uuid {
    let Some(folder) = inv.folders.get(&outfit) else {
        return Uuid::nil();
    };
    if !folder.info.thumbnail.is_nil() {
        return folder.info.thumbnail;
    }
    let mut todo = vec![outfit];
    let mut seen = HashSet::new();
    while let Some(id) = todo.pop() {
        let Some(folder) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
            continue;
        };
        for item in folder.items.iter().filter_map(|id| inv.items.get(id)) {
            let item = if item.asset_type == 24 {
                inv.items.get(&item.asset_id).unwrap_or(item)
            } else {
                item
            };
            if item.asset_type == 0 && !item.asset_id.is_nil() {
                return item.asset_id;
            }
        }
        todo.extend(&folder.children);
    }
    Uuid::nil()
}

/// The HD pictures this viewer uploaded into an outfit's own folder
/// (`snapshot::portrait::HD_PICTURE_DESC`): our own textures, not links,
/// oldest first. The showcase shows the newest in its large preview; the
/// cards keep the thumbnail, as LLOutfitGallery::refreshOutfit does.
pub fn hd_pictures(inv: &Inventory, outfit: Uuid) -> Vec<&InvItem> {
    let Some(folder) = inv.folders.get(&outfit) else {
        return Vec::new();
    };
    let mut found: Vec<(usize, &InvItem)> = folder
        .items
        .iter()
        .filter_map(|id| inv.items.get(id))
        .filter(|it| {
            it.parent == outfit && it.asset_type == 0 && !it.asset_id.is_nil() && it.desc == crate::snapshot::portrait::HD_PICTURE_DESC
        })
        .enumerate()
        .collect();
    found.sort_by_key(|(at, it)| (it.created_at, *at));
    found.into_iter().map(|(_, it)| it).collect()
}

/// The newest HD picture of an outfit, its texture's asset.
pub fn outfit_hd_picture(inv: &Inventory, outfit: Uuid) -> Option<Uuid> {
    hd_pictures(inv, outfit).last().map(|it| it.asset_id)
}

/// LLInventoryModel::findCategoryUUIDForTypeInRoot (llinventorymodel.cpp),
/// indexed across inventory and Appearance rules until the inventory changes.
pub fn system_folder(inv: &Inventory, kind: i32) -> Option<Uuid> {
    inv.system_folder(kind)
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

/// Loaded, and every link resolved: to its original, or known broken. A
/// broken link never holds an outfit back (LLAppearanceMgr::
/// updateAppearanceFromCOF waits only while the inventory is still fetched).
pub fn complete(inv: &Inventory, folder: Uuid) -> bool {
    loaded(inv, folder)
        && folder_links(inv, folder)
            .iter()
            .all(|l| l.folder || inv.items.contains_key(&l.target) || inv.broken(&l.target))
}

/// `folder_links` without the links to a broken original, which are skipped
/// and dropped from the rewritten folder (LLAppearanceMgr::
/// wearInventoryCategoryOnAvatar: "Attempt to wear a broken link").
pub fn usable_links(inv: &Inventory, folder: Uuid) -> Vec<OutfitLink> {
    let mut links = folder_links(inv, folder);
    links.retain(|l| l.folder || !inv.broken(&l.target));
    links
}

/// Logs the broken links an outfit change skips, once per change (menus
/// read `usable_links` at every frame).
fn warn_broken(inv: &Inventory, folder: Uuid) {
    for l in folder_links(inv, folder).iter().filter(|l| !l.folder && inv.broken(&l.target)) {
        log::warn!("outfit link \"{}\" in folder {folder} is broken: skipped", l.name);
    }
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
        .filter(|target| !inv.items.contains_key(target) && !inv.missing_items.contains(target))
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
            if !link.folder && !link.target.is_nil() && !inv.items.contains_key(&link.target) && !inv.broken(&link.target) {
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
/// unrelated inventory types and broken links are skipped, links still
/// loading are refused.
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
        for link in usable_links(inv, id).into_iter().filter(|l| !l.folder) {
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

/// Take items off the planned COF: clothing and attachments only, a body
/// part is replaced, never removed.
fn remove_links(inv: &Inventory, current: &mut Vec<OutfitLink>, ids: Vec<Uuid>) -> Result<(), String> {
    let mut removable = HashSet::new();
    for id in ids {
        let id = super::inventory::actions::original(inv, id).ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
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
    Ok(())
}

/// Put one item on the planned COF. A body part always replaces its type;
/// `replace_layer` clears the item's clothing layer first; `replace` sends
/// an attachment to its point instead of adding it there.
fn wear_link(
    inv: &Inventory,
    current: &mut Vec<OutfitLink>,
    worn: &HashMap<Uuid, u8>,
    seen: &mut HashSet<Uuid>,
    request: WearRequest,
    replace_layer: bool,
) -> Result<(), String> {
    let id =
        super::inventory::actions::original(inv, request.item).ok_or(aurora_i18n::tr!("world-appearance-this-item-has-not-loaded-yet"))?;
    if !seen.insert(id) {
        return Ok(());
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
    if it.asset_type == 13 || (replace_layer && it.asset_type == 5) {
        current.retain(|l| {
            l.folder
                || inv
                    .items
                    .get(&l.target)
                    .is_none_or(|old| old.asset_type != it.asset_type || old.flags & 0xff != it.flags & 0xff)
        });
    }
    if request.replace && it.asset_type == 6 {
        // Point zero uses the last saved point (LLObjectBridge::mAttachPt).
        let point = if request.point == 0 {
            (it.flags & 0xff) as u8
        } else {
            request.point
        };
        if point != 0 {
            current.retain(|l| {
                l.folder
                    || l.target == id
                    || inv
                        .items
                        .get(&l.target)
                        .is_none_or(|old| old.asset_type != 6 || worn.get(&l.target).copied().unwrap_or((old.flags & 0xff) as u8) != point)
            });
        }
    }
    if !current.iter().any(|l| l.target == id) {
        current.push(item_link(it));
    }
    Ok(())
}

/// Produces one acknowledged AIS operation. No optimistic inventory mutation.
pub fn plan(inv: &Inventory, action: Action, worn: &HashMap<Uuid, u8>) -> Result<(OutfitMutation, bool), String> {
    // DeleteFromOutfit removes the saved links, never the originals or the worn items.
    if let Action::DeleteFromOutfit { folder, items } = action {
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
                links: usable_links(inv, folder)
                    .into_iter()
                    .filter(|l| !items.contains(&l.target))
                    .collect(),
                cof: None,
            },
            false,
        ));
    }
    let cof = cof(inv).ok_or(aurora_i18n::tr!("world-appearance-the-current-outfit-folder-is-unavailable"))?;
    require_complete(inv, cof)?;
    // the rewritten COF drops its broken links
    warn_broken(inv, cof);
    let mut current = usable_links(inv, cof);
    if matches!(
        action,
        Action::Add(_)
            | Action::WearItem { .. }
            | Action::WearItems { .. }
            | Action::Force { .. }
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
        Action::ImportJson { backup, name, partial } => {
            if !super::outfit_backup::valid_name(&name) {
                return Err(aurora_i18n::tr!("world-appearance-choose-a-name-of-1-to-63-characters").into());
            }
            let resolved = backup.resolve(inv, inv.owner);
            if !partial && !resolved.missing.is_empty() {
                return Err(aurora_i18n::tr!("outfit-json-missing-changed").into());
            }
            // Wearing zero links would take off every clothing item and
            // attachment, so an empty backup is refused like an unavailable one.
            if resolved.links.is_empty() {
                return Err(if resolved.missing.is_empty() {
                    aurora_i18n::tr!("outfit-json-empty")
                } else {
                    aurora_i18n::tr!("outfit-json-none-found")
                }
                .into());
            }
            let mut links = resolved.links;
            // As updateCOF does, keep the current required body part whenever
            // the selected (possibly partial) outfit does not supply its type.
            for old in current.iter().filter(|link| !link.folder) {
                if let Some(item) = inv.items.get(&old.target)
                    && item.asset_type == 13
                    && !links.iter().any(|link| {
                        inv.items
                            .get(&link.target)
                            .is_some_and(|it| it.asset_type == 13 && it.flags & 0xff == item.flags & 0xff)
                    })
                {
                    links.push(old.clone());
                }
            }
            let parent =
                system_folder(inv, FT_MY_OUTFITS).ok_or(aurora_i18n::tr!("world-appearance-the-my-outfits-folder-is-unavailable"))?;
            let id = Uuid::new_v4();
            let name = name.trim().to_owned();
            change.folder = id;
            change.create = Some(InvFolder {
                id,
                parent,
                name: name.clone(),
                type_default: FT_OUTFIT,
                version: 1,
                ..Default::default()
            });
            change.links = links.clone();
            links.push(OutfitLink {
                target: id,
                name,
                desc: String::new(),
                folder: true,
                inv_type: 8,
            });
            change.cof = Some((cof, links));
        }
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
                crate::text_validation::require(crate::text_validation::Policy::Ascii, &name)?;
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
                    usable_links(inv, change.folder)
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
                warn_broken(inv, *id);
                links.extend(if contents {
                    wearable_contents(inv, *id)?
                } else {
                    usable_links(inv, *id)
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
            remove_links(inv, &mut current, ids)?;
            change.links = current;
        }
        Action::Force { remove, wear } => {
            remove_links(inv, &mut current, remove)?;
            let mut seen = HashSet::new();
            for request in wear {
                // RLVa already chose replace or add per item (the first
                // clothing of a layer replaces, the next ones add).
                wear_link(inv, &mut current, worn, &mut seen, request, request.replace)?;
            }
            change.links = current;
        }
        Action::RemoveOutfit(_) | Action::RemoveOutfits(_) => {
            let folders = match action {
                Action::RemoveOutfit(id) => vec![id],
                Action::RemoveOutfits(ids) => ids,
                _ => unreachable!(),
            };
            // takeOffOutfit never removes the four required body parts, and
            // takes any folder: a demo or a delivery, not only a saved outfit
            // (#1849).
            let mut removable = HashSet::new();
            for id in folders {
                require_complete(inv, id)?;
                warn_broken(inv, id);
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
                let layer = super::inventory::actions::original(inv, id)
                    .and_then(|id| inv.items.get(&id))
                    .map(|it| (it.asset_type, it.flags & 0xff));
                // The first clothing of a layer replaces, the next ones add.
                let replace_layer = replace && layer.is_some_and(|(kind, layer)| kind != 5 || replaced_clothing.insert(layer));
                wear_link(
                    inv,
                    &mut current,
                    worn,
                    &mut seen,
                    WearRequest { item: id, replace, point },
                    replace_layer,
                )?;
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
        Action::ShowOriginal(_)
        | Action::Gesture(..)
        | Action::Favorite(_)
        | Action::OutfitImage(_)
        | Action::OutfitSnapshot(_)
        | Action::OutfitPortrait(_)
        | Action::OutfitPhotoMode(_)
        | Action::OutfitBatch(_)
        | Action::Category(..)
        | Action::Folders(_)
        | Action::DeleteFromOutfit { .. }
        | Action::ExportJson(_)
        | Action::PickJson => {
            return Err(aurora_i18n::tr!("world-appearance-this-action-does-not-change-the-current-outfit").into());
        }
    }
    // Every new-outfit entry point (including imports) must satisfy the
    // destination-name policy before any inventory requests are prepared.
    if let Some(created) = &change.create {
        crate::text_validation::require(crate::text_validation::Policy::Ascii, &created.name)?;
    }
    if change.folder == cof {
        drop_coalesced(inv, &mut change.links);
    }
    normalize_layers(inv, &mut change.links);
    if let Some((_, links)) = &mut change.cof {
        drop_coalesced(inv, links);
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
            crate::text_validation::require_edit(crate::text_validation::Policy::Ascii, &inv.folders[&id].info.name, name)?;
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
        type_default: None,
    })
}

/// A coalesced object (II_FLAGS_OBJECT_HAS_MULTIPLE_ITEMS): the simulator
/// refuses to attach it ("You can't attach multiple objects to one spot").
pub fn coalesced(it: &InvItem) -> bool {
    it.asset_type == 6 && it.flags & crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS != 0
}

/// The Current Outfit never links a coalesced object. The Legacy Viewer links
/// an object only once it arrives worn (LLAppearanceMgr::wearItemsOnAvatar
/// sends rez_attachment alone, registerAttachment links), so a refused one
/// leaves nothing behind; here the COF is written first, and a link left to a
/// coalesced object would be asked for again at every outfit sync and login,
/// and count toward the 38 attachments (#1668). A rewrite drops such links.
fn drop_coalesced(inv: &Inventory, links: &mut Vec<OutfitLink>) {
    links.retain(|l| l.folder || !inv.items.get(&l.target).is_some_and(coalesced));
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

/// LLAppearanceMgr::wearItemsOnAvatar / LLAttachmentsMgr::linkRecentlyArrivedAttachments
/// (indra/newview/llappearancemgr.cpp, llattachmentsmgr.cpp): request objects
/// first, and link only those that arrive worn. Inventory flags do not say
/// whether an object is Animesh; the simulator decides its account limit.
/// A refused request must not leave a COF link that retries at every login.
/// Saved outfits retain their requested objects; only the COF write changes.
pub fn defer_attachment_links(
    inv: &Inventory,
    change: &mut OutfitMutation,
    worn: &HashSet<Uuid>,
    attachments: &mut Vec<AttachRequest>,
    agent: Uuid,
) {
    let Some(cof) = cof(inv) else { return };
    let before = folder_links(inv, cof);
    let links = if change.folder == cof {
        &mut change.links
    } else if let Some((folder, links)) = &mut change.cof {
        if *folder != cof {
            return;
        }
        links
    } else {
        return;
    };
    links.retain(|link| {
        let Some(item) = inv.items.get(&link.target).filter(|item| !link.folder && item.asset_type == 6) else {
            return true;
        };
        if worn.contains(&item.id) {
            return true;
        }
        // An existing link may still be restoring at login. Do not remove
        // it just because the region has not sent the object yet, unless
        // this action explicitly retries that item.
        if before.iter().any(|old| old.target == item.id) && !attachments.iter().any(|request| request.item_id == item.id) {
            return true;
        }
        if !attachments.iter().any(|request| request.item_id == item.id) {
            attachments.push(super::outfit::attach_request(item, agent));
        }
        false
    });
}

/// An object can arrive while the COF write is in flight. Keep explicit
/// requests and objects that arrived during the write out of the detach
/// list until their arrival is linked. Keep the chosen point and mode.
pub fn sync_with_attachments(
    inv: &Inventory,
    agent: Uuid,
    worn: &HashSet<Uuid>,
    worn_before: &HashSet<Uuid>,
    keep: Uuid,
    attachments: Vec<AttachRequest>,
) -> CofSync {
    let mut sync = sync_plan_keeping(inv, agent, worn, keep);
    for command in &mut sync.commands {
        if let NetCommand::DetachAttachments(items) = command {
            items.retain(|id| worn_before.contains(id) && !attachments.iter().any(|request| request.item_id == *id));
        }
    }
    sync.commands
        .retain(|command| !matches!(command, NetCommand::DetachAttachments(items) if items.is_empty()));
    for attachment in attachments {
        if worn.contains(&attachment.item_id) && attachment.point == 0 && attachment.add {
            continue;
        }
        apply_attachment_override(&mut sync.commands, attachment);
    }
    sync
}

pub fn sync_plan(inv: &Inventory, agent: Uuid, worn: &HashSet<Uuid>) -> CofSync {
    sync_plan_keeping(inv, agent, worn, Uuid::nil())
}

/// `sync_plan` that never takes `keep` off, linked or not: the Aurora bridge,
/// which the Legacy Viewer puts back into every replaced outfit (FSLSLBridge
/// in LLAppearanceMgr::updateCOF, llappearancemgr.cpp). It stays in `worn`,
/// or its link would ask to wear it again at each sync.
pub fn sync_plan_keeping(inv: &Inventory, agent: Uuid, worn: &HashSet<Uuid>, keep: Uuid) -> CofSync {
    let Some(cof) = cof(inv) else {
        return CofSync::default();
    };
    // the links to objects, with those whose item is not known yet: a worn
    // object linked in the COF stays on even before its item is fetched
    let objects: HashSet<_> = super::outfit::cof_objects(inv, cof).into_iter().collect();
    let detach: Vec<_> = worn.difference(&objects).copied().filter(|id| *id != keep).collect();
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

/// A forced batch with nothing left to do: its links would not change and the
/// avatar already matches. RlvForceWear::done (rlvhelper.cpp) returns early
/// here; an outfit write would still raise the COF version and ask for a new
/// appearance. Read when the batch runs: a run of `@detach` queues while the
/// first object is leaving, and has nothing to do once it has left. An object
/// on the avatar or on its way needs no new request (`requested`:
/// LLAttachmentsMgr::wasRequestedRecently) and is linked when it arrives
/// (`update_attachment_inventory`). Gestures are always applied, their state
/// is not in the links.
pub fn force_changes_nothing(inv: &Inventory, action: &Action, worn: &HashMap<Uuid, u8>, requested: &HashSet<Uuid>) -> bool {
    let Action::Force { remove, wear } = action else {
        return false;
    };
    let original = |id: Uuid| super::inventory::actions::original(inv, id).unwrap_or(id);
    let gesture = |id: Uuid| inv.items.get(&id).is_some_and(|it| it.asset_type == 21);
    let settled_off = |id: &Uuid| {
        let id = original(*id);
        !gesture(id) && !worn.contains_key(&id)
    };
    let settled_on = |request: &WearRequest| {
        let id = original(request.item);
        !gesture(id)
            && match inv.items.get(&id).map(|it| it.asset_type) {
                Some(6) => match worn.get(&id) {
                    Some(point) => request.point == 0 || request.point == *point,
                    None => requested.contains(&id),
                },
                Some(_) => true,
                None => false,
            }
    };
    if !remove.iter().all(settled_off) || !wear.iter().all(settled_on) {
        return false;
    }
    let (Some(cof), Ok((change, _))) = (cof(inv), plan(inv, action.clone(), worn)) else {
        return false;
    };
    let arriving: HashSet<_> = wear
        .iter()
        .map(|request| original(request.item))
        .filter(|id| inv.items.get(id).is_some_and(|it| it.asset_type == 6))
        .collect();
    let links = |links: &[OutfitLink]| {
        links
            .iter()
            .filter(|l| !arriving.contains(&l.target))
            .map(|l| (l.target, l.folder))
            .collect::<HashSet<_>>()
    };
    links(&folder_links(inv, cof)) == links(&change.links)
}

/// Explicit attachment requests retain the chosen point and replace/add mode.
/// They are sent only after the COF update has been acknowledged. Objects
/// are linked on arrival, so "Add" also needs its explicit rez request.
pub fn attachments_for_action(inv: &Inventory, agent: Uuid, action: &Action) -> Vec<AttachRequest> {
    let requests: Vec<WearRequest> = match action {
        Action::WearItem { item, point, replace } => vec![WearRequest {
            item: *item,
            replace: *replace,
            point: *point,
        }],
        Action::WearItems { items, point, replace } => items
            .iter()
            .map(|item| WearRequest {
                item: *item,
                replace: *replace,
                point: *point,
            })
            .collect(),
        Action::Force { wear, .. } => wear.clone(),
        Action::ImportJson { backup, .. } => {
            let available: HashSet<_> = backup.resolve(inv, agent).links.into_iter().map(|link| link.target).collect();
            backup
                .points()
                .into_iter()
                .filter(|request| available.contains(&request.item))
                .collect()
        }
        Action::Add(item) => vec![WearRequest {
            item: *item,
            replace: false,
            point: 0,
        }],
        _ => return Vec::new(),
    };
    let mut seen = HashSet::new();
    requests
        .into_iter()
        .filter_map(|request| super::inventory::actions::original(inv, request.item).map(|id| (id, request)))
        .filter(|(id, _)| seen.insert(*id))
        .filter_map(|(id, request)| inv.items.get(&id).filter(|it| it.asset_type == 6).map(|it| (it, request)))
        .map(|(it, request)| AttachRequest {
            item_id: it.id,
            owner_id: if it.owner.is_nil() { agent } else { it.owner },
            point: request.point,
            add: !request.replace,
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

/// `AURORA_DEMO_APPEARANCE=broken`: the third outfit and the current
/// outfit hold a link to a deleted original, answered missing as the grid
/// would (issue #1441): both still wear, without it.
pub fn seed_demo_broken(inv: &mut Inventory, agent: Uuid) {
    let gone = OutfitLink {
        target: Uuid::from_u128(729),
        name: aurora_i18n::tr!("world-appearance-deleted-scarf").into(),
        desc: String::new(),
        folder: false,
        inv_type: 18,
    };
    for folder in [Uuid::from_u128(704)].into_iter().chain(cof(inv)) {
        let mut links = folder_links(inv, folder);
        links.push(gone.clone());
        demo_replace(inv, agent, folder, &links);
    }
    inv.items_missing(vec![gone.target]);
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

/// `AURORA_DEMO_APPEARANCE=take-off-folder` (#1849): an ordinary folder,
/// not a saved outfit, as a demo or a delivery worn from Recent. It holds
/// the worn jacket and pants originals and, in a subfolder, a link to the
/// worn mesh hair; frame 600 takes it off as the inventory's Remove from
/// current outfit does.
pub const DEMO_TAKE_OFF_FOLDER: Uuid = Uuid::from_u128(0x1849_0001);

/// Seeds the [`DEMO_TAKE_OFF_FOLDER`] folder and its subfolder.
pub fn seed_demo_take_off(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::Folder;
    let sub = Uuid::from_u128(0x1849_0002);
    let folder = |id, parent, name: &str, children| Folder {
        info: InvFolder {
            id,
            parent,
            name: name.into(),
            type_default: -1,
            version: 1,
            ..Default::default()
        },
        children,
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    };
    let root = inv.root;
    inv.folders
        .insert(DEMO_TAKE_OFF_FOLDER, folder(DEMO_TAKE_OFF_FOLDER, root, "Demo delivery", vec![sub]));
    inv.folders
        .insert(sub, folder(sub, DEMO_TAKE_OFF_FOLDER, "Accessories", Vec::new()));
    if let Some(r) = inv.folders.get_mut(&root) {
        r.children.push(DEMO_TAKE_OFF_FOLDER);
    }
    let items = [715, 716]
        .into_iter()
        .filter_map(|n| inv.items.get(&Uuid::from_u128(n)).cloned())
        .map(|mut it| {
            it.parent = DEMO_TAKE_OFF_FOLDER;
            it
        })
        .collect();
    inv.apply(vec![aurora_net::inventory::FolderContents {
        folder_id: DEMO_TAKE_OFF_FOLDER,
        owner_id: agent,
        version: 2,
        folders: Vec::new(),
        items,
    }]);
    let hair: Vec<_> = inv.items.get(&Uuid::from_u128(717)).map(item_link).into_iter().collect();
    demo_replace(inv, agent, sub, &hair);
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

    /// Issue #1441: a link to a deleted original held every outfit change
    /// on "still loading" forever.
    #[test]
    fn an_outfit_link_to_a_deleted_original_waits_for_the_server_then_is_skipped() {
        let mut inv = fixture();
        let outfit = Uuid::from_u128(704);
        let gone = Uuid::from_u128(723);
        let original = inv.items.remove(&gone).expect("linked original");
        assert!(!complete(&inv, outfit), "not read yet: still loading");
        assert!(plan(&inv, Action::Wear(outfit, false), &HashMap::new()).is_err());
        assert_eq!(missing_targets(&inv, outfit).collect::<Vec<_>>(), [gone]);
        inv.items_missing(vec![gone]);
        assert!(complete(&inv, outfit));
        assert!(missing_targets(&inv, outfit).next().is_none(), "never asked for again");
        for action in [
            Action::Wear(outfit, false),
            Action::Wear(outfit, true),
            Action::WearContents {
                folder: outfit,
                mode: WearMode::ReplaceOutfit,
            },
            Action::RemoveOutfit(outfit),
        ] {
            let (change, _) = plan(&inv, action.clone(), &HashMap::new()).unwrap_or_else(|e| panic!("{action:?}: {e}"));
            assert!(!change.links.iter().any(|l| l.target == gone), "{action:?} skips the broken link");
        }
        let (change, _) = plan(&inv, Action::Wear(outfit, false), &HashMap::new()).expect("wear");
        assert!(
            change.links.iter().any(|l| l.target == Uuid::from_u128(710)),
            "the rest of the outfit is worn"
        );
        // the original comes back (restored from the trash): no longer broken
        inv.add_items(vec![original]);
        assert!(!inv.broken(&gone));
        let (change, _) = plan(&inv, Action::Wear(outfit, false), &HashMap::new()).expect("wear");
        assert!(change.links.iter().any(|l| l.target == gone));
    }

    #[test]
    fn a_broken_link_in_the_current_outfit_no_longer_blocks_outfit_changes() {
        let mut inv = fixture();
        let cof = cof(&inv).expect("COF");
        let gone = Uuid::from_u128(950);
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(951),
            parent: cof,
            name: "Deleted shirt".into(),
            asset_type: 24,
            inv_type: 18,
            asset_id: gone,
            ..Default::default()
        }]);
        assert!(!complete(&inv, cof));
        assert!(plan(&inv, Action::Wear(Uuid::from_u128(704), false), &HashMap::new()).is_err());
        inv.items_missing(vec![gone]);
        assert!(complete(&inv, cof));
        let clothing: Vec<_> = worn_clothing(&inv).into_iter().map(|(id, _)| id).collect();
        for action in [
            Action::Wear(Uuid::from_u128(704), false),
            Action::Wear(Uuid::from_u128(703), true),
            Action::RemoveItems(clothing.clone()),
            Action::Save(Some("Without the shirt".into())),
        ] {
            let (change, _) = plan(&inv, action.clone(), &HashMap::new()).unwrap_or_else(|e| panic!("{action:?}: {e}"));
            assert!(
                !change.links.iter().any(|l| l.target == gone)
                    && change.cof.iter().all(|(_, links)| !links.iter().any(|l| l.target == gone)),
                "{action:?}: the rewritten folders drop the broken link"
            );
        }
    }

    #[test]
    fn failed_item_reads_are_retried_then_count_as_broken() {
        let mut inv = fixture();
        let id = Uuid::from_u128(723);
        inv.items.remove(&id);
        for _ in 1..super::super::inventory::ITEM_FETCH_ATTEMPTS {
            inv.items_failed(&[id]);
            assert!(!inv.broken(&id), "one failed read proves nothing");
            assert_eq!(inv.take_item_retries(), HashSet::from([id]));
            assert!(!complete(&inv, Uuid::from_u128(704)));
        }
        inv.items_failed(&[id]);
        assert!(inv.broken(&id));
        assert!(inv.take_item_retries().is_empty());
        assert!(complete(&inv, Uuid::from_u128(704)));
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

    /// An ordinary folder (no outfit type) holding `items`, under `parent`.
    fn plain_folder(inv: &mut Inventory, id: u128, parent: Uuid, items: &[u128]) -> Uuid {
        let id = Uuid::from_u128(id);
        let mut folder = inv.folders[&Uuid::from_u128(703)].clone();
        folder.info.id = id;
        folder.info.parent = parent;
        folder.info.type_default = -1;
        folder.info.name = format!("Folder {id}");
        folder.children.clear();
        folder.items = items.iter().map(|n| Uuid::from_u128(*n)).collect();
        inv.folders.insert(id, folder);
        if let Some(p) = inv.folders.get_mut(&parent) {
            p.children.push(id);
        }
        for item in &folder_items(inv, id) {
            inv.items.get_mut(item).expect("folder item").parent = id;
        }
        id
    }

    fn folder_items(inv: &Inventory, id: Uuid) -> Vec<Uuid> {
        inv.folders[&id].items.clone()
    }

    /// Issue #1849: "Remove from current outfit" on an ordinary folder (a
    /// demo worn from Recent) refused it as an outfit it cannot modify. The
    /// Legacy Viewer (takeOffOutfit) takes off the worn clothing and
    /// attachments of any folder and its subfolders, originals or links,
    /// never a body part, and leaves the rest of the outfit on.
    #[test]
    fn remove_outfit_takes_off_an_ordinary_folder() {
        let mut inv = fixture();
        let (skin, jacket, pants, hair, earrings, spare) = (711, 715, 716, 717, 718, 722);
        let worn = |change: &OutfitMutation, n: u128| change.links.iter().any(|l| l.target == Uuid::from_u128(n));
        let current = usable_links(&inv, cof(&inv).expect("COF"));
        let before = |n: u128| current.iter().any(|l| l.target == Uuid::from_u128(n));
        assert!([skin, jacket, pants, hair, earrings].into_iter().all(before), "worn in the demo");
        assert!(!before(spare), "the spare shirt is not");
        // the demo: worn originals, an unworn one, a body part, and a
        // subfolder holding a link to a worn attachment
        let root = inv.root;
        let demo = plain_folder(&mut inv, 1100, root, &[jacket, spare, skin]);
        let sub = plain_folder(&mut inv, 1101, demo, &[]);
        let link = Uuid::from_u128(1102);
        let mut hair_link = inv.items[&Uuid::from_u128(hair)].clone();
        hair_link.id = link;
        hair_link.parent = sub;
        hair_link.asset_type = 24;
        hair_link.asset_id = Uuid::from_u128(hair);
        inv.items.insert(link, hair_link);
        inv.folders.get_mut(&sub).expect("subfolder").items.push(link);
        let (change, sync) = plan(&inv, Action::RemoveOutfit(demo), &HashMap::new()).expect("take off a demo folder");
        assert!(sync);
        assert!(!worn(&change, jacket), "its clothing is taken off");
        assert!(!worn(&change, hair), "the attachment linked in its subfolder too");
        assert!(!worn(&change, spare), "nothing is put on");
        assert!(worn(&change, skin), "a body part in the folder stays");
        for n in 710..714 {
            assert!(worn(&change, n), "body part {n}");
        }
        for n in [pants, earrings, 719, 720, 721] {
            assert!(worn(&change, n), "{n}, outside the folder, stays worn");
        }
        // two ordinary folders at once: one outfit change for both
        let other = plain_folder(&mut inv, 1103, root, &[pants]);
        let (change, _) = plan(&inv, Action::RemoveOutfits(vec![demo, other]), &HashMap::new()).expect("take off two folders");
        assert!(!worn(&change, jacket) && !worn(&change, pants) && !worn(&change, hair));
        assert!(worn(&change, earrings));
        // an unloaded folder waits instead of taking off part of it
        inv.folders.get_mut(&sub).expect("subfolder").state = FetchState::Unknown;
        assert!(plan(&inv, Action::RemoveOutfit(demo), &HashMap::new()).is_err());
    }

    #[test]
    fn the_newest_hd_picture_is_found_and_never_worn() {
        let mut inv = fixture();
        let outfit = Uuid::from_u128(702);
        let before = folder_links(&inv, outfit);
        assert_eq!(outfit_hd_picture(&inv, outfit), None);
        let texture = |n: u128, desc: &str, created_at: i64| InvItem {
            id: Uuid::from_u128(0x4d00 + n),
            parent: outfit,
            name: format!("Photo {n}"),
            desc: desc.to_owned(),
            asset_type: 0,
            asset_id: Uuid::from_u128(0x4e00 + n),
            created_at,
            ..Default::default()
        };
        let ours = crate::snapshot::portrait::HD_PICTURE_DESC;
        inv.add_items(vec![
            texture(1, ours, 10),
            texture(2, ours, 20),
            // a texture the user keeps in the folder is not ours
            texture(3, "a merchant's picture", 30),
        ]);
        assert_eq!(outfit_hd_picture(&inv, outfit), Some(Uuid::from_u128(0x4e02)));
        assert_eq!(
            hd_pictures(&inv, outfit).iter().map(|it| it.id).collect::<Vec<_>>(),
            [Uuid::from_u128(0x4d01), Uuid::from_u128(0x4d02)]
        );
        // a texture in the folder is never part of what the outfit wears
        assert_eq!(folder_links(&inv, outfit).len(), before.len());
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
        // a multiple selection goes in one update: another link of the outfit with it
        let other = folder_links(&inv, folder)
            .into_iter()
            .find(|l| !l.folder && l.target != item)
            .expect("a second link")
            .target;
        let kept = folder_links(&inv, folder).len() - 2;
        let (change, sync) = plan(
            &inv,
            Action::DeleteFromOutfit {
                folder,
                items: vec![item, other],
            },
            &HashMap::new(),
        )
        .expect("delete links");
        assert!(!sync, "saved-outfit removal must not detach worn objects");
        demo_mutate(&mut inv, Uuid::from_u128(1), &change).expect("mutation");
        let links = folder_links(&inv, folder);
        assert!(!links.iter().any(|l| l.target == item || l.target == other));
        assert_eq!(links.len(), kept, "the other links stay");
        assert!(
            inv.items.contains_key(&item) && inv.items.contains_key(&other),
            "originals retained"
        );
        assert_eq!(folder_links(&inv, cof(&inv).expect("COF")), current);
    }

    #[test]
    fn a_coalesced_object_is_asked_once_and_never_linked_in_the_cof() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let (item, linked) = (Uuid::from_u128(723), Uuid::from_u128(717));
        inv.items.get_mut(&item).expect("object").flags |= crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
        let action = Action::Add(item);
        let (change, _) = plan(&inv, action.clone(), &HashMap::new()).expect("add");
        assert!(!change.links.iter().any(|l| l.target == item), "not linked");
        assert!(change.links.iter().any(|l| l.target == linked), "the outfit stays");
        // the request still leaves once, for the simulator's refusal
        let asked = attachments_for_action(&inv, agent, &action);
        assert_eq!(
            asked.iter().map(|a| (a.item_id, a.point, a.add)).collect::<Vec<_>>(),
            [(item, 0, true)]
        );
        // Ordinary Add also requests the object explicitly: its COF link
        // is deferred until the simulator confirms the attachment.
        assert_eq!(attachments_for_action(&inv, agent, &Action::Add(linked))[0].item_id, linked);
        demo_mutate(&mut inv, agent, &change).expect("mutation");
        let asks_for = |inv: &Inventory, id: Uuid| {
            sync_commands(inv, agent, &HashSet::new())
                .iter()
                .any(|c| matches!(c, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == id)))
        };
        assert!(!asks_for(&inv, item));
        assert!(asks_for(&inv, linked));
    }

    #[test]
    fn a_link_left_to_a_coalesced_object_is_not_worn_again_and_is_dropped() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let stale = Uuid::from_u128(717);
        let cof = cof(&inv).expect("COF");
        assert!(folder_links(&inv, cof).iter().any(|l| l.target == stale));
        inv.items.get_mut(&stale).expect("object").flags |= crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
        assert!(!super::super::outfit::cof_objects(&inv, cof).contains(&stale), "not re-requested");
        assert!(
            !sync_commands(&inv, agent, &HashSet::new())
                .iter()
                .any(|c| matches!(c, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == stale)))
        );
        // the next COF rewrite drops the link, freeing its attachment slot
        let (change, _) = plan(&inv, Action::Add(Uuid::from_u128(723)), &HashMap::new()).expect("add");
        assert!(!change.links.iter().any(|l| l.target == stale));
    }

    #[test]
    fn refused_attachments_leave_no_cof_link_for_a_later_sync_or_login() {
        let agent = Uuid::from_u128(1);
        for limit in [2, 3] {
            let mut inv = fixture();
            let cof = cof(&inv).expect("COF");
            let worn: HashMap<_, _> = (717..717 + limit).map(|id| (Uuid::from_u128(id), 5)).collect();
            let rejected = Uuid::from_u128(723);
            // Also recover a phantom link left by an earlier viewer when
            // the user explicitly attempts that object again.
            let mut links = folder_links(&inv, cof);
            links.push(item_link(&inv.items[&rejected]));
            demo_replace(&mut inv, agent, cof, &links);
            let action = Action::Add(rejected);
            let (mut change, _) = plan(&inv, action.clone(), &worn).expect("add at account limit");
            let mut attachments = attachments_for_action(&inv, agent, &action);
            defer_attachment_links(&inv, &mut change, &worn.keys().copied().collect(), &mut attachments, agent);
            assert!(!change.links.iter().any(|link| link.target == rejected));
            assert!(worn.keys().all(|id| change.links.iter().any(|link| link.target == *id)));
            demo_mutate(&mut inv, agent, &change).expect("COF write");
            let sync = sync_with_attachments(
                &inv,
                agent,
                &worn.keys().copied().collect(),
                &worn.keys().copied().collect(),
                Uuid::nil(),
                attachments,
            );
            assert_eq!(
                sync.commands
                    .iter()
                    .flat_map(|cmd| match cmd {
                        NetCommand::RezAttachments(items) => items.iter().filter(|a| a.item_id == rejected).collect(),
                        _ => Vec::new(),
                    })
                    .count(),
                1,
                "one request, whose refusal does not link the object"
            );
            assert!(
                !super::super::outfit::cof_objects(&inv, cof).contains(&rejected),
                "login cannot re-request it"
            );
            let subsequent = sync_plan(&inv, agent, &worn.keys().copied().collect());
            assert!(
                !subsequent
                    .commands
                    .iter()
                    .any(|cmd| matches!(cmd, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == rejected)))
            );
            assert!(inv.items.contains_key(&rejected), "the inventory original remains");
        }
    }

    #[test]
    fn deferred_bulk_requests_keep_the_point_mode_and_deduplicate_aliases() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let original = Uuid::from_u128(723);
        let alias = Uuid::from_u128(9001);
        inv.items.insert(
            alias,
            InvItem {
                id: alias,
                asset_type: 24,
                asset_id: original,
                ..Default::default()
            },
        );
        for replace in [false, true] {
            let action = Action::WearItems {
                items: vec![original, alias],
                replace,
                point: 35,
            };
            let (mut change, _) = plan(&inv, action.clone(), &HashMap::new()).expect("bulk request");
            let mut attachments = attachments_for_action(&inv, agent, &action);
            defer_attachment_links(&inv, &mut change, &HashSet::new(), &mut attachments, agent);
            let requests: Vec<_> = attachments.iter().filter(|a| a.item_id == original).collect();
            assert_eq!(requests.len(), 1);
            assert_eq!((requests[0].point, requests[0].add), (35, !replace));
            assert!(!change.links.iter().any(|link| link.target == original));
        }
    }

    #[test]
    fn an_attachment_arriving_during_the_cof_write_is_not_detached() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let item = Uuid::from_u128(723);
        let action = Action::Add(item);
        let (mut change, _) = plan(&inv, action.clone(), &HashMap::new()).expect("add");
        let mut attachments = attachments_for_action(&inv, agent, &action);
        defer_attachment_links(&inv, &mut change, &HashSet::new(), &mut attachments, agent);
        demo_mutate(&mut inv, agent, &change).expect("COF readback");
        let another = Uuid::from_u128(9999);
        let removed = Uuid::from_u128(9998);
        let sync = sync_with_attachments(
            &inv,
            agent,
            &HashSet::from([item, another, removed]),
            &HashSet::from([removed]),
            Uuid::nil(),
            attachments,
        );
        assert!(
            !sync
                .commands
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::DetachAttachments(items) if items.contains(&another)))
        );
        assert!(
            sync.commands
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::DetachAttachments(items) if items.contains(&removed)))
        );
        assert!(
            !sync
                .commands
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::DetachAttachments(items) if items.contains(&item)))
        );
        assert!(
            !sync
                .commands
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::RezAttachments(items) if items.iter().any(|a| a.item_id == item)))
        );
    }

    #[test]
    fn saved_outfit_objects_stay_saved_while_their_cof_links_wait_for_arrival() {
        let inv = fixture();
        let agent = Uuid::from_u128(1);
        let saved = Uuid::from_u128(704);
        let before = folder_links(&inv, saved);
        let (mut change, _) = plan(&inv, Action::Wear(saved, false), &HashMap::new()).expect("wear outfit");
        let mut attachments = Vec::new();
        defer_attachment_links(&inv, &mut change, &HashSet::new(), &mut attachments, agent);
        assert!(!attachments.is_empty());
        assert!(
            attachments
                .iter()
                .all(|request| !change.links.iter().any(|link| link.target == request.item_id))
        );
        assert_eq!(folder_links(&inv, saved), before, "the saved outfit is not rewritten");
        let (mut save, _) = plan(&inv, Action::Save(Some("Saved outfit".into())), &HashMap::new()).expect("save");
        let saved_objects: Vec<_> = save
            .links
            .iter()
            .filter(|link| inv.items[&link.target].asset_type == 6)
            .cloned()
            .collect();
        defer_attachment_links(&inv, &mut save, &HashSet::new(), &mut Vec::new(), agent);
        assert!(
            saved_objects.iter().all(|link| save.links.contains(link)),
            "only the COF side of a save is deferred"
        );
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

    /// RlvForceWear::done: a run of forced commands costs no outfit write once
    /// the avatar and the links are as asked, and anything still to do does.
    #[test]
    fn a_forced_batch_with_nothing_to_do_changes_nothing() {
        let mut inv = fixture();
        let cof = cof(&inv).expect("COF");
        let (linked, other) = (Uuid::from_u128(970), Uuid::from_u128(971));
        let shirt = Uuid::from_u128(972);
        inv.add_items(
            [(linked, 6, 6), (other, 6, 6), (shirt, 5, 18)]
                .into_iter()
                .map(|(id, asset_type, inv_type)| InvItem {
                    id,
                    parent: inv.root,
                    asset_type,
                    inv_type,
                    flags: 4,
                    name: "item".into(),
                    ..Default::default()
                })
                .collect(),
        );
        inv.add_items(
            [(980, linked, 6), (981, shirt, 18)]
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
        let remove = |items: &[Uuid]| Action::Force {
            remove: items.to_vec(),
            wear: Vec::new(),
        };
        let wear = |item, point| Action::Force {
            remove: Vec::new(),
            wear: vec![WearRequest {
                item,
                replace: false,
                point,
            }],
        };
        let on = HashMap::from([(linked, 5)]);
        let none = HashMap::new();
        let nothing = |action: &Action, worn: &HashMap<Uuid, u8>| force_changes_nothing(&inv, action, worn, &HashSet::new());
        assert!(nothing(&remove(&[other]), &on), "off already, no link");
        assert!(!nothing(&remove(&[other]), &HashMap::from([(other, 5)])), "still on");
        assert!(!nothing(&remove(&[linked]), &none), "its link is still there");
        assert!(nothing(&wear(linked, 0), &on), "on already, linked");
        assert!(nothing(&wear(linked, 5), &on), "on its point");
        assert!(!nothing(&wear(linked, 6), &on), "to be moved");
        assert!(!nothing(&wear(linked, 0), &none), "linked, not on the avatar");
        assert!(nothing(&wear(other, 0), &HashMap::from([(other, 5)])), "on, linked when it arrived");
        assert!(!nothing(&wear(other, 0), &none), "off, no request");
        let requested = HashSet::from([other]);
        assert!(force_changes_nothing(&inv, &wear(other, 0), &none, &requested), "on its way");
        assert!(
            !force_changes_nothing(&inv, &wear(other, 6), &HashMap::from([(other, 5)]), &requested),
            "still to be moved"
        );
        assert!(nothing(&wear(shirt, 0), &none), "clothing already linked");
        assert!(!nothing(&remove(&[shirt]), &none), "clothing to take off");
        assert!(
            !nothing(&remove(&[Uuid::from_u128(999)]), &on),
            "an unknown item is an error to report"
        );
        assert!(!nothing(&Action::Remove(other), &on), "only forced batches");
    }

    /// LLAttachmentsMgr::wasRequestedRecently: the first wear goes out, its
    /// repeats do not, and one is sent again once the request is old.
    #[test]
    fn repeats_of_a_forced_wear_wait_for_the_first_request() {
        let mut world = super::super::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory = fixture();
        let item = Uuid::from_u128(990);
        world.inventory.add_items(vec![InvItem {
            id: item,
            parent: world.inventory.root,
            asset_type: 6,
            inv_type: 6,
            flags: 4,
            name: "item".into(),
            ..Default::default()
        }]);
        let action = Action::Force {
            remove: Vec::new(),
            wear: vec![WearRequest {
                item,
                replace: true,
                point: 0,
            }],
        };
        let start = std::time::Instant::now();
        assert!(!world.rlva_force_settled(&action, start), "the first one goes out");
        assert!(
            world.rlva_force_settled(&action, start + std::time::Duration::from_secs(1)),
            "a repeat does not"
        );
        assert!(
            !world.rlva_force_settled(&action, start + std::time::Duration::from_secs(6)),
            "sent again once old"
        );
    }

    /// The bridge item: on while it is worn, whether the COF links it or not,
    /// and only worn again when a link finds it really absent.
    #[test]
    fn cof_sync_neither_wears_again_nor_takes_off_the_kept_bridge() {
        let mut inv = fixture();
        let agent = Uuid::from_u128(1);
        let cof = cof(&inv).expect("COF");
        let bridge = Uuid::from_u128(960);
        let touched = |sync: &CofSync| {
            sync.commands.iter().fold((false, false), |(worn, off), command| match command {
                NetCommand::RezAttachments(items) => (worn || items.iter().any(|a| a.item_id == bridge), off),
                NetCommand::DetachAttachments(items) => (worn, off || items.contains(&bridge)),
                _ => (worn, off),
            })
        };
        inv.add_items(vec![InvItem {
            id: bridge,
            parent: inv.root,
            asset_type: 6,
            inv_type: 6,
            name: "bridge".into(),
            ..Default::default()
        }]);
        let on = HashSet::from([bridge]);
        let unlinked = sync_plan_keeping(&inv, agent, &on, bridge);
        assert_eq!(touched(&unlinked), (false, false), "worn and not linked: it stays on");
        assert_eq!(touched(&sync_plan(&inv, agent, &on)), (false, true), "any other object comes off");
        inv.add_items(vec![InvItem {
            id: Uuid::from_u128(961),
            parent: cof,
            asset_type: 24,
            inv_type: 6,
            asset_id: bridge,
            ..Default::default()
        }]);
        let linked = sync_plan_keeping(&inv, agent, &on, bridge);
        assert_eq!(touched(&linked), (false, false), "worn and linked: nothing to do");
        let absent = sync_plan_keeping(&inv, agent, &HashSet::new(), bridge);
        assert_eq!(touched(&absent), (true, false), "linked and absent: it is worn");
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

    /// LLOutfitGallery::refreshOutfit: the thumbnail, else a texture in the
    /// outfit or linked from it, its subfolders included.
    #[test]
    fn an_outfit_s_picture_falls_back_on_a_texture_inside() {
        let id = Uuid::from_u128;
        let folder = |n, parent, thumbnail| InvFolder {
            id: id(n),
            parent: id(parent),
            type_default: 47,
            thumbnail: id(thumbnail),
            ..Default::default()
        };
        let item = |n, parent, asset_type, asset_id| InvItem {
            id: id(n),
            parent: id(parent),
            asset_type,
            asset_id: id(asset_id),
            ..Default::default()
        };
        let mut inv = Inventory::default();
        inv.add_folders(vec![folder(1, 48, 0), folder(2, 1, 0)], false);
        assert!(outfit_image(&inv, id(1)).is_nil(), "nothing to show");
        assert!(outfit_image(&inv, id(99)).is_nil(), "unknown outfit");
        // a wearable is no picture
        inv.items.insert(id(10), item(10, 1, 13, 500));
        inv.folders.get_mut(&id(1)).expect("outfit").items.push(id(10));
        assert!(outfit_image(&inv, id(1)).is_nil());
        // in a subfolder
        inv.items.insert(id(11), item(11, 2, 0, 501));
        inv.folders.get_mut(&id(2)).expect("subfolder").items.push(id(11));
        assert_eq!(outfit_image(&inv, id(1)), id(501));
        // a link to a texture elsewhere, in the outfit itself
        inv.items.insert(id(12), item(12, 7, 0, 502));
        inv.items.insert(id(13), item(13, 1, 24, 12));
        inv.folders.get_mut(&id(1)).expect("outfit").items.push(id(13));
        assert_eq!(outfit_image(&inv, id(1)), id(502));
        // the folder's own thumbnail comes first
        inv.folders.get_mut(&id(1)).expect("outfit").info.thumbnail = id(503);
        assert_eq!(outfit_image(&inv, id(1)), id(503));
    }
}
