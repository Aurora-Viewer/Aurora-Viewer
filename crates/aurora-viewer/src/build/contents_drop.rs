//! Inventory dropped into an object's contents: one entry, or a whole
//! selection with its folders, from the world (dragged over an object) or
//! onto the Contents tab. Port of LLToolDragAndDrop::{willObjectAcceptInventory,
//! dad3dUpdateInventory, dad3dUpdateInventoryCategory, dad3dRezScript,
//! dad3dRezObjectOnObject, dad3dApplyToObject, dropInventory, dropScript}
//! (indra/newview/lltooldraganddrop.cpp), LLDroppableItem and
//! LLTaskCategoryBridge::dragOrDrop (indra/newview/llpanelobjectinventory.cpp),
//! originally LGPL 2.1.
//!
//! A drop of several entries is all or nothing, like the Legacy Viewer's cargo
//! loop: a single refused entry refuses the whole drop.

use crate::build::task_inventory;
use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
use crate::world::World;
use crate::world::inventory::{FetchState, actions as rules};
use aurora_net::build::BuildCmd;
use aurora_net::inventory::InvItem;
use std::collections::HashSet;
use uuid::Uuid;

const AT_TEXTURE: i32 = 0;
const AT_CALLINGCARD: i32 = 2;
const AT_OBJECT: i32 = 6;
const AT_CLOTHING: i32 = 5;
const AT_BODYPART: i32 = 13;
const AT_LSL_TEXT: i32 = 10;
const AT_LINK: i32 = 24;
const AT_LINK_FOLDER: i32 = 25;
const AT_MATERIAL: i32 = 57;
const FT_TRASH: i32 = 14;
const FT_MARKETPLACE: i32 = 53;
/// FLAGS_ALLOW_INVENTORY_DROP (llprimitive.h).
const ALLOW_INVENTORY_DROP: u32 = 1 << 16;
/// PERM_ITEM_UNRESTRICTED (llpermissionsflags.h).
const UNRESTRICTED: u32 = rules::MODIFY | rules::COPY | rules::TRANSFER;

/// One item sent into an object's contents.
#[derive(Debug)]
pub struct ContentsDrop {
    pub command: BuildCmd,
    /// A no-copy item leaves the inventory (dropInventory deletes it locally).
    pub consume: Option<Uuid>,
}

/// Where the cargo is released: the rules differ for the keys held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Over a prim in the world (LLToolDragAndDrop::dragOrDrop3D).
    World,
    /// Over the Contents tab of the edit window (LLTaskCategoryBridge).
    Panel,
}

pub fn reason_text(reason: &str) -> &'static str {
    match reason {
        "rez-marketplace" => aurora_i18n::tr!("rez-marketplace"),
        "rez-incomplete" => aurora_i18n::tr!("rez-incomplete"),
        "rez-object-only" => aurora_i18n::tr!("rez-object-only"),
        "rez-worn" => aurora_i18n::tr!("rez-worn"),
        "rez-worn-contents" => aurora_i18n::tr!("rez-worn-contents"),
        "rez-contents-denied" => aurora_i18n::tr!("rez-contents-denied"),
        "rez-transfer-denied" => aurora_i18n::tr!("rez-transfer-denied"),
        "rez-hold-ctrl" => aurora_i18n::tr!("rez-hold-ctrl"),
        _ => aurora_i18n::tr!("rez-no-surface"),
    }
}

/// allowCopyBy(agent), without an active-group override, like the Legacy Viewer.
pub fn copyable(item: &InvItem, agent: Uuid) -> bool {
    agent_mask(item, agent) & rules::COPY != 0
}

fn agent_mask(item: &InvItem, agent: Uuid) -> u32 {
    if item.owner == agent && !item.group_owned {
        item.owner_mask
    } else {
        item.everyone_mask
    }
}

/// The root of the prim `idx`'s linkset, and whether it is worn (its root
/// sits on an avatar). `None` for an avatar or a broken parent chain.
fn linkset(world: &World, idx: usize) -> Option<(usize, bool)> {
    let mut current = idx;
    for _ in 0..64 {
        let o = world.objects.get(current)?;
        if o.is_avatar() {
            return None;
        }
        if o.parent_id == 0 {
            return Some((current, false));
        }
        let parent = world.objects.parent_of(o)?;
        if world.objects.get(parent)?.is_avatar() {
            return Some((current, true));
        }
        current = parent;
    }
    None
}

/// The world rules of an inventory drag over the prim `idx`: never an avatar,
/// a tree or what an avatar wears (they are avatar targets in the Legacy
/// Viewer). With Ctrl the item goes into the contents and the object must
/// accept it.
pub fn target_allowed(world: &World, idx: usize, item: &InvItem, ctrl: bool) -> Result<(), &'static str> {
    surface_allowed(world, idx)?;
    if ctrl {
        accepts(world, idx, item)?;
    }
    Ok(())
}

/// A world support, independent of any inventory cargo.
pub fn surface_allowed(world: &World, idx: usize) -> Result<(), &'static str> {
    let o = world.objects.get(idx).ok_or("rez-no-surface")?;
    if o.is_avatar() || o.is_tree() {
        return Err("rez-no-surface");
    }
    match linkset(world, idx) {
        Some((_, false)) => {}
        _ => return Err("rez-no-surface"),
    }
    Ok(())
}

/// willObjectAcceptInventory for the prim `idx`: a volume whose linkset you
/// may modify, or that allows inventory drops (scripts always need Modify),
/// and an item its owner may receive.
pub fn accepts(world: &World, idx: usize, item: &InvItem) -> Result<(), &'static str> {
    let o = world.objects.get(idx).ok_or("rez-no-surface")?;
    let (root, attached) = linkset(world, idx).ok_or("rez-no-surface")?;
    let root = world.objects.get(root).ok_or("rez-no-surface")?;
    // Calling cards in objects are disabled (STORM-1117); links are never given.
    if matches!(item.asset_type, AT_CALLINGCARD | AT_LINK | AT_LINK_FOLDER) {
        return Err("rez-contents-denied");
    }
    if o.pcode != aurora_prim::params::LL_PCODE_VOLUME
        || root.update_flags & OBJECT_MODIFY == 0 && o.update_flags & ALLOW_INVENTORY_DROP == 0
        || item.asset_type == AT_LSL_TEXT && root.update_flags & OBJECT_MODIFY == 0
        // the server does not support an item in the world and in inventory at once
        || attached && item.base_mask & UNRESTRICTED != UNRESTRICTED
    {
        return Err("rez-contents-denied");
    }
    let owned = root.update_flags & OBJECT_YOU_OWNER != 0;
    if !(owned && item.owner == world.agent_id) && agent_mask(item, world.agent_id) & rules::TRANSFER == 0 {
        return Err("rez-transfer-denied");
    }
    Ok(())
}

/// What the agent wears now: attached objects and the wearables the Current
/// Outfit links to (get_is_item_worn).
struct Worn(HashSet<Uuid>);

impl Worn {
    fn of(world: &World) -> Self {
        let mut worn = HashSet::new();
        if let Some(avatar) = world.objects.index_of_uuid(&world.agent_id).and_then(|idx| world.objects.get(idx)) {
            worn.extend(
                world
                    .objects
                    .children_of(&avatar.key)
                    .iter()
                    .filter_map(|&idx| world.objects.get(idx)?.attachment_item_id()),
            );
        }
        let inv = &world.inventory;
        if let Some(cof) = crate::world::appearance::cof(inv).and_then(|id| inv.folders.get(&id)) {
            worn.extend(
                cof.items
                    .iter()
                    .filter_map(|id| inv.items.get(id))
                    .filter(|link| link.asset_type == AT_LINK)
                    .map(|link| link.asset_id),
            );
        }
        Self(worn)
    }

    fn has(&self, item: &InvItem) -> bool {
        matches!(item.asset_type, AT_OBJECT | AT_CLOTHING | AT_BODYPART) && self.0.contains(&item.id)
    }
}

/// An entry carried, as the drop sees it.
struct Carried<'a> {
    item: &'a InvItem,
    /// Brought by a dragged folder (dad3dUpdateInventoryCategory).
    from_folder: bool,
}

/// The items carried: entries as they are, folders with every item below them
/// (trash excluded). A folder leaves out what its owner could not give
/// (LLDroppableItem: worn items, no-transfer items for an object you do not
/// own), and, unlike the Legacy Viewer which then refuses the whole folder,
/// the links and calling cards no object accepts.
fn carried<'a>(world: &'a World, ids: &[Uuid], owned: bool, worn: &Worn) -> Result<Vec<Carried<'a>>, &'static str> {
    let inv = &world.inventory;
    let trash = rules::system(inv, FT_TRASH);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &id in ids {
        if inv.in_folder_type(id, FT_MARKETPLACE) {
            return Err("rez-marketplace");
        }
        if inv.folders.contains_key(&id) {
            let mut stack = vec![id];
            while let Some(folder) = stack.pop() {
                let Some(f) = inv.folders.get(&folder) else { continue };
                if f.state != FetchState::Fetched && !f.library {
                    return Err("rez-incomplete");
                }
                stack.extend(f.children.iter().copied().filter(|c| Some(*c) != trash));
                for item in f.items.iter().filter_map(|id| inv.items.get(id)) {
                    let giveable = !matches!(item.asset_type, AT_CALLINGCARD | AT_LINK | AT_LINK_FOLDER)
                        && !worn.has(item)
                        && (owned || agent_mask(item, world.agent_id) & rules::TRANSFER != 0);
                    if giveable && seen.insert(item.id) {
                        out.push(Carried { item, from_folder: true });
                    }
                }
            }
            continue;
        }
        let item = inv.items.get(&id).ok_or("rez-incomplete")?;
        if worn.has(item) {
            return Err("rez-worn-contents");
        }
        if seen.insert(item.id) {
            out.push(Carried { item, from_folder: false });
        }
    }
    Ok(out)
}

/// The item put into the prim `idx`: scripts with RezScript, running unless
/// asked otherwise (dropScript), the rest with UpdateTaskInventory
/// (dropInventory), with their actual permissions, dated at drop time.
/// RezScript carries the active group separately from the item's group
/// (LLViewerObject::saveScript, indra/newview/llviewerobject.cpp).
pub fn item_drop(world: &World, idx: usize, item: &InvItem, running: bool) -> Option<ContentsDrop> {
    let o = world.objects.get(idx)?;
    let mut task = task_inventory::from_inventory(o.full_id, item);
    task.creation_date = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(task.creation_date, |d| d.as_secs() as i32);
    let command = if item.asset_type == AT_LSL_TEXT {
        BuildCmd::RezScript {
            handle: o.key.region,
            local_id: o.key.local_id,
            enabled: running,
            group_id: world.groups.active,
            item: Box::new(task),
        }
    } else {
        BuildCmd::UpdateTaskInventory {
            handle: o.key.region,
            local_id: o.key.local_id,
            item: Box::new(task),
        }
    };
    Some(ContentsDrop {
        command,
        consume: (!copyable(item, world.agent_id)).then_some(item.id),
    })
}

/// Whether, in the world, this entry needs Ctrl to go into the contents:
/// without it an object rezzes, a texture or material is applied to the
/// face, and a folder is refused. Everything else goes in anyway
/// (dad3dUpdateInventory, dad3dRezScript).
fn needs_ctrl(world: &World, id: Uuid) -> bool {
    world.inventory.folders.contains_key(&id)
        || world
            .inventory
            .items
            .get(&id)
            .is_some_and(|it| matches!(it.asset_type, AT_OBJECT | AT_TEXTURE | AT_MATERIAL))
}

/// The drop of the inventory entries `ids` (items and folders) into the prim
/// `idx`, every item checked first. Scripts run unless Ctrl is held.
///
/// In the world, the items dad3dUpdateInventory and dad3dRezScript handle go
/// to the linkset root, like the Legacy Viewer; objects, textures, materials
/// and folders (which need Ctrl) go into the prim under the pointer. On the
/// Contents tab everything goes into the object it lists.
pub fn plan(world: &World, ids: &[Uuid], idx: usize, ctrl: bool, target: Target) -> Result<Vec<ContentsDrop>, &'static str> {
    let o = world.objects.get(idx).ok_or("rez-no-surface")?;
    let (root, attached) = linkset(world, idx).ok_or("rez-no-surface")?;
    if target == Target::World {
        if attached || o.is_tree() {
            return Err("rez-no-surface");
        }
        if !ctrl && ids.iter().any(|&id| needs_ctrl(world, id)) {
            return Err("rez-hold-ctrl");
        }
    }
    let owned = world.objects.get(root).is_some_and(|r| r.update_flags & OBJECT_YOU_OWNER != 0);
    let worn = Worn::of(world);
    let cargo = carried(world, ids, owned, &worn)?;
    let mut drops = Vec::with_capacity(cargo.len());
    for Carried { item, from_folder } in cargo {
        let into = if target == Target::World && !from_folder && !matches!(item.asset_type, AT_OBJECT | AT_TEXTURE | AT_MATERIAL) {
            root
        } else {
            idx
        };
        accepts(world, into, item)?;
        drops.extend(item_drop(world, into, item, !ctrl));
    }
    Ok(drops)
}

/// The answer of `plan` while the cargo is held still: asked again only
/// when the cargo, the target, Ctrl, the inventory or the object's rights
/// change, not every frame.
#[derive(Clone, Default)]
pub struct PlanCheck {
    /// cargo, prim, its flags and its root's, Ctrl, inventory generation
    key: Option<(Vec<Uuid>, Uuid, (u32, u32), bool, u64)>,
    /// the number of items the drop would send
    result: Option<Result<usize, &'static str>>,
}

impl PlanCheck {
    pub fn check(&mut self, world: &World, ids: &[Uuid], idx: usize, ctrl: bool, target: Target) -> Result<usize, &'static str> {
        let Some(o) = world.objects.get(idx) else {
            return Err("rez-no-surface");
        };
        let root = linkset(world, idx)
            .and_then(|(root, _)| world.objects.get(root))
            .map_or(0, |r| r.update_flags);
        let key = (ids.to_vec(), o.full_id, (o.update_flags, root), ctrl, world.inventory.generation);
        match self.result {
            Some(result) if self.key.as_ref() == Some(&key) => result,
            _ => {
                let result = plan(world, ids, idx, ctrl, target).map(|drops| drops.len());
                self.key = Some(key);
                self.result = Some(result);
                result
            }
        }
    }
}

#[cfg(test)]
#[path = "contents_drop_tests.rs"]
mod tests;
