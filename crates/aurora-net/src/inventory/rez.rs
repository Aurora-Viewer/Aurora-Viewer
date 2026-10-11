//! Wire contract of LLToolDragAndDrop::dropObject / pack_permissions_slam
//! (Firestorm indra/newview/lltooldraganddrop.cpp, originally LGPL 2.1).
//! Placement and permission decisions are made by Aurora before queueing.

use super::InvItem;
use crate::types::RegionHandle;
use aurora_msg::msgs::{RezObject, rez_object};
use glam::Vec3;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RezInventory {
    pub handle: RegionHandle,
    pub group: Uuid,
    pub item: InvItem,
    /// All coordinates are local to the destination region.
    pub ray_start: Vec3,
    pub ray_end: Vec3,
    pub target: Uuid,
    pub selected: bool,
    pub remove: bool,
    /// Library and trash items must not be taken back into their source folder.
    pub source_folder: Uuid,
}

impl RezInventory {
    pub(crate) fn message(&self, agent: Uuid, session: Uuid) -> RezObject {
        let i = &self.item;
        let land = self.target.is_nil();
        RezObject {
            agent_data: rez_object::AgentData {
                agent_id: agent,
                session_id: session,
                group_id: self.group,
            },
            rez_data: rez_object::RezData {
                from_task_id: Uuid::nil(),
                bypass_raycast: u8::from(land),
                ray_start: self.ray_start,
                // The simulator traces only the picked object. Extend the ray
                // through its surface, as required by dropObject's contract.
                ray_end: if land { self.ray_end } else { self.ray_end * 2.0 - self.ray_start },
                ray_target_id: self.target,
                ray_end_is_intersection: false,
                rez_selected: self.selected,
                remove_item: self.remove,
                item_flags: i.flags,
                group_mask: i.group_mask,
                everyone_mask: i.everyone_mask,
                next_owner_mask: i.next_owner_mask,
            },
            inventory_data: rez_object::InventoryData {
                item_id: i.id,
                folder_id: self.source_folder,
                creator_id: i.creator,
                owner_id: i.owner,
                group_id: i.group_id,
                base_mask: i.base_mask,
                owner_mask: i.owner_mask,
                group_mask: i.group_mask,
                everyone_mask: i.everyone_mask,
                next_owner_mask: i.next_owner_mask,
                group_owned: i.group_owned,
                transaction_id: Uuid::nil(),
                type_: i.asset_type as i8,
                inv_type: i.inv_type as i8,
                flags: i.flags,
                sale_type: i.sale_type,
                sale_price: i.sale_price,
                name: aurora_msg::str_field(&i.name),
                description: aurora_msg::str_field(&i.desc),
                creation_date: i.created_at as i32,
                crc: super::operations::checksum_in_folder(i, self.source_folder),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn land_object_rays_and_inventory_permissions() {
        let mut r = RezInventory {
            handle: 1,
            group: Uuid::from_u128(2),
            item: InvItem {
                id: Uuid::from_u128(3),
                parent: Uuid::from_u128(4),
                asset_type: 6,
                inv_type: 6,
                base_mask: 0x7fff_ffff,
                owner_mask: 0xa000,
                group_mask: 0x2000,
                everyone_mask: 0x8000,
                next_owner_mask: 0x6000,
                flags: 0x100,
                name: "Cube".into(),
                desc: "Test".into(),
                ..Default::default()
            },
            ray_start: Vec3::new(-12.0, 30.0, 40.0),
            ray_end: Vec3::new(3.0, 20.0, 10.0),
            target: Uuid::nil(),
            selected: true,
            remove: true,
            source_folder: Uuid::nil(),
        };
        let land = r.message(Uuid::from_u128(5), Uuid::nil());
        assert_eq!(land.rez_data.bypass_raycast, 1);
        assert_eq!(land.rez_data.ray_end, Vec3::new(3.0, 20.0, 10.0));
        assert!(land.rez_data.remove_item && land.rez_data.rez_selected);
        assert_eq!(land.inventory_data.folder_id, Uuid::nil());
        assert_eq!(r.item.parent, Uuid::from_u128(4));
        assert_eq!(land.inventory_data.base_mask, r.item.base_mask);
        assert_eq!(land.inventory_data.owner_mask, r.item.owner_mask);
        assert_eq!(land.rez_data.next_owner_mask, land.inventory_data.next_owner_mask);
        assert_eq!(land.inventory_data.name, b"Cube\0");
        let mut wire_item = r.item.clone();
        wire_item.parent = r.source_folder;
        assert_eq!(land.inventory_data.crc, super::super::operations::checksum(&wire_item));
        assert_ne!(land.inventory_data.crc, super::super::operations::checksum(&r.item));
        r.target = Uuid::from_u128(6);
        let object = r.message(Uuid::nil(), Uuid::nil());
        assert_eq!(object.rez_data.bypass_raycast, 0);
        assert_eq!(object.rez_data.ray_target_id, r.target);
        assert_eq!(object.rez_data.ray_end, Vec3::new(18.0, 10.0, -20.0));
        assert!(!object.rez_data.ray_end_is_intersection);
    }
}
