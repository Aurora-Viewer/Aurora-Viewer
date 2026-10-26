//! Task-to-agent inventory transfers: LLTaskInvFVBridge::startDrag,
//! LLFolderBridge::dragItemIntoFolder and warn_move_inventory (Firestorm
//! indra/newview/llpanelobjectinventory.cpp / llinventorybridge.cpp, LGPL 2.1).

use super::BuildTool;
use crate::world::{
    World,
    inventory::{Inventory, actions},
};
use aurora_net::build::{ObjectProps, TaskItem, perm};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Copy,
    Move { scripted: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransferRequest {
    pub object: Uuid,
    pub item: TaskItem,
    pub folder: Uuid,
    /// The strength of the warning the user accepted (true = scripted).
    pub approved: Option<bool>,
    pub ignore_warning: bool,
}

#[cfg(test)]
#[path = "task_inventory_tests.rs"]
mod tests;

/// A stronger warning is required if the object became scripted meanwhile.
pub fn approved(accepted: Option<bool>, scripted: bool) -> bool {
    accepted == Some(true) || accepted == Some(scripted)
}

pub fn owned(world: &World, object: Uuid, props: Option<&ObjectProps>) -> bool {
    world
        .objects
        .index_of_uuid(&object)
        .and_then(|i| world.objects.get(i))
        .is_some_and(|o| {
            o.update_flags & crate::ui::context::flags::OBJECT_YOU_OWNER != 0
                || o.owner_id == world.agent_id
                || props.is_some_and(|p| p.owner_id == world.agent_id)
        })
}

pub fn kind(world: &World, object: Uuid, item: &TaskItem, object_owned: bool) -> Result<TransferKind, &'static str> {
    let idx = world
        .objects
        .index_of_uuid(&object)
        .ok_or(aurora_i18n::tr!("task-transfer-source-gone"))?;
    let o = world.objects.get(idx).ok_or(aurora_i18n::tr!("task-transfer-source-gone"))?;
    if item.is_folder || item.item_id.is_nil() || item.parent_id != object {
        return Err(aurora_i18n::tr!("task-transfer-unavailable"));
    }
    // LLPermissions::allowCopyBy(agent, active group), allowTransferTo(agent).
    let mask = if !item.group_owned && item.owner_id == world.agent_id {
        item.owner_mask
    } else if !item.group_id.is_nil() && item.group_id == world.groups.active {
        item.group_mask | item.everyone_mask
    } else {
        item.everyone_mask
    };
    let copy = mask & perm::COPY != 0;
    let transfer = !item.group_owned && item.owner_id == world.agent_id || item.owner_mask & perm::TRANSFER != 0;
    let attached = world
        .objects
        .get(super::root_of(world, idx))
        .is_some_and(|root| root.parent_id != 0 && root.attachment_point() != 0);
    if !copy && attached {
        return Err(aurora_i18n::tr!("task-transfer-attachment-no-copy"));
    }
    if copy && transfer {
        return Ok(TransferKind::Copy);
    }
    if object_owned {
        let scripted = o.update_flags & (1 << 6) != 0;
        return Ok(TransferKind::Move { scripted });
    }
    Err(aurora_i18n::tr!("task-transfer-no-permission"))
}

pub fn destination(inv: &Inventory, folder: Uuid, item: &TaskItem, protected: &HashSet<Uuid>) -> bool {
    if !inv.folders.contains_key(&folder) || !actions::mutable(inv, folder, protected) {
        return false;
    }
    let mut at = folder;
    let mut seen = HashSet::new();
    while seen.insert(at) {
        let Some(f) = inv.folders.get(&at) else { return false };
        if f.library || matches!(f.info.type_default, 46..=48 | 50 | 53 | 54) {
            return false;
        }
        if f.info.type_default == 23 && !(item.asset_type == 3 || item.asset_type == 24 && item.inv_type == 3) {
            return false;
        }
        if at == inv.root {
            return true;
        }
        at = f.info.parent;
    }
    false
}

/// "Copy to inventory" of a whole object's contents into one new folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentsTransfer {
    /// The no-copy items, moved out of the object instead of copied.
    pub moved: Vec<Uuid>,
    /// The object holds scripts (MoveInventoryFromScriptedObject).
    pub scripted: bool,
}

/// move_inv_category_world_to_agent (indra/newview/llinventorybridge.cpp):
/// each item copyable and transferable, or the object ours and the no-copy
/// ones moved; one refused item refuses the whole copy.
pub fn contents_transfer(world: &World, object: Uuid, items: &[TaskItem], object_owned: bool) -> Result<ContentsTransfer, String> {
    let items: Vec<_> = items.iter().filter(|item| !item.is_folder).collect();
    if items.is_empty() {
        return Err(aurora_i18n::tr!("contents-copy-nothing").into());
    }
    let mut transfer = ContentsTransfer {
        moved: Vec::new(),
        scripted: items.iter().any(|item| item.asset_type == 10),
    };
    for item in items {
        match kind(world, object, item, object_owned) {
            Ok(TransferKind::Copy) => {}
            Ok(TransferKind::Move { scripted }) => {
                transfer.scripted |= scripted;
                transfer.moved.push(item.item_id);
            }
            Err(reason) => {
                return Err(aurora_i18n::tr!(
                    "contents-copy-item-refused",
                    name = item.name.as_str(),
                    reason = reason
                ));
            }
        }
    }
    Ok(transfer)
}

/// The new folder's name: the object's (LLFloaterOpenObject::moveToInventory),
/// without what an inventory name cannot hold.
pub fn contents_folder_name(object_name: &str) -> String {
    let name: String = object_name.chars().filter(|c| !c.is_control() && *c != '|').collect();
    let name = name.trim();
    if name.is_empty() {
        aurora_i18n::tr!("common-object").into()
    } else {
        name.chars().take(63).collect()
    }
}

impl BuildTool {
    pub fn task_transfer_kind(&self, world: &World, object: Uuid, item: &TaskItem) -> Result<TransferKind, &'static str> {
        let mut result = kind(world, object, item, owned(world, object, self.props.get(&object)))?;
        if let TransferKind::Move { scripted } = &mut result {
            *scripted |= self
                .contents
                .objects
                .get(&object)
                .is_some_and(|inv| inv.items.iter().any(|item| item.asset_type == 10));
        }
        Ok(result)
    }
}

pub fn from_inventory(object: Uuid, item: &aurora_net::inventory::InvItem) -> TaskItem {
    TaskItem {
        item_id: item.id,
        parent_id: object,
        creator_id: item.creator,
        owner_id: item.owner,
        last_owner_id: item.last_owner,
        group_id: item.group_id,
        group_owned: item.group_owned,
        base_mask: item.base_mask,
        owner_mask: item.owner_mask,
        group_mask: item.group_mask,
        everyone_mask: item.everyone_mask,
        next_owner_mask: item.next_owner_mask,
        asset_id: item.asset_id,
        asset_type: item.asset_type as i8,
        inv_type: item.inv_type as i8,
        flags: item.flags,
        name: item.name.clone(),
        description: item.desc.clone(),
        creation_date: item.created_at as i32,
        sale_type: item.sale_type,
        sale_price: item.sale_price,
        ..Default::default()
    }
}
