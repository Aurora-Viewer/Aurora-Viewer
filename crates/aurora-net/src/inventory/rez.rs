//! Wire contract of LLToolDragAndDrop::dropObject / pack_permissions_slam
//! (Firestorm indra/newview/lltooldraganddrop.cpp, originally LGPL 2.1).
//! Placement and permission decisions are made by Aurora before queueing.

use super::InvItem;
use crate::types::RegionHandle;
use aurora_msg::msgs::{RezObject, RezObjectFromNotecard, rez_object, rez_object_from_notecard};
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
    /// The object whose contents hold the item, nil for the agent's
    /// inventory: the simulator copies it out of that object (FromTaskID).
    pub from_task: Uuid,
    /// An object embedded in a notecard: RezObjectFromNotecard instead of
    /// RezObject (dropObject with SOURCE_NOTECARD).
    pub notecard: Option<FromNotecard>,
}

/// Where an embedded object comes from: the notecard's item, and the
/// object whose contents hold that notecard (nil for the agent's
/// inventory), as LLViewerTextEditor's drag gives them (mPreviewID,
/// mObjectID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FromNotecard {
    pub notecard: Uuid,
    pub object: Uuid,
}

impl RezInventory {
    /// RezObjectFromNotecard (LLToolDragAndDrop::dropObject, SOURCE_NOTECARD):
    /// the same RezData, then the notecard and the object that holds it,
    /// and the embedded item named by its id only.
    pub(crate) fn notecard_message(&self, agent: Uuid, session: Uuid, from: FromNotecard) -> RezObjectFromNotecard {
        let r = self.message(agent, session);
        let d = r.rez_data;
        RezObjectFromNotecard {
            agent_data: rez_object_from_notecard::AgentData {
                agent_id: agent,
                session_id: session,
                group_id: self.group,
            },
            rez_data: rez_object_from_notecard::RezData {
                from_task_id: d.from_task_id,
                bypass_raycast: d.bypass_raycast,
                ray_start: d.ray_start,
                ray_end: d.ray_end,
                ray_target_id: d.ray_target_id,
                ray_end_is_intersection: d.ray_end_is_intersection,
                rez_selected: d.rez_selected,
                remove_item: d.remove_item,
                item_flags: d.item_flags,
                group_mask: d.group_mask,
                everyone_mask: d.everyone_mask,
                next_owner_mask: d.next_owner_mask,
            },
            notecard_data: rez_object_from_notecard::NotecardData {
                notecard_item_id: from.notecard,
                object_id: from.object,
            },
            inventory_data: vec![rez_object_from_notecard::InventoryData { item_id: self.item.id }],
        }
    }

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
                from_task_id: self.from_task,
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
                group_id: Uuid::from_u128(99),
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
            from_task: Uuid::nil(),
            notecard: None,
        };
        let land = r.message(Uuid::from_u128(5), Uuid::nil());
        assert_eq!(land.agent_data.group_id, r.group);
        assert_eq!(land.inventory_data.group_id, r.item.group_id);
        assert_ne!(land.agent_data.group_id, land.inventory_data.group_id);
        assert_eq!(land.rez_data.bypass_raycast, 1);
        assert_eq!(land.rez_data.from_task_id, Uuid::nil());
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
        assert_eq!(object.agent_data.group_id, r.group);
        assert_eq!(object.rez_data.bypass_raycast, 0);
        assert_eq!(object.rez_data.ray_target_id, r.target);
        assert_eq!(object.rez_data.ray_end, Vec3::new(18.0, 10.0, -20.0));
        assert!(!object.rez_data.ray_end_is_intersection);
        r.from_task = Uuid::from_u128(7);
        assert_eq!(r.message(Uuid::nil(), Uuid::nil()).rez_data.from_task_id, r.from_task);
    }

    #[test]
    fn an_object_of_a_notecard_names_the_notecard_its_object_and_the_item() {
        let r = RezInventory {
            handle: 1,
            group: Uuid::from_u128(2),
            item: InvItem {
                id: Uuid::from_u128(3),
                asset_type: 6,
                inv_type: 6,
                everyone_mask: 0x8000,
                next_owner_mask: 0x6000,
                flags: 0x100,
                ..Default::default()
            },
            ray_start: Vec3::new(1.0, 2.0, 30.0),
            ray_end: Vec3::new(5.0, 6.0, 20.0),
            target: Uuid::nil(),
            selected: false,
            remove: true,
            source_folder: Uuid::nil(),
            from_task: Uuid::nil(),
            notecard: Some(FromNotecard {
                notecard: Uuid::from_u128(7),
                object: Uuid::from_u128(8),
            }),
        };
        let from = r.notecard.unwrap();
        let m = r.notecard_message(Uuid::from_u128(10), Uuid::from_u128(11), from);
        assert_eq!(m.agent_data.group_id, Uuid::from_u128(2));
        assert_eq!(m.notecard_data.notecard_item_id, Uuid::from_u128(7));
        assert_eq!(m.notecard_data.object_id, Uuid::from_u128(8));
        assert_eq!(m.inventory_data.len(), 1);
        assert_eq!(m.inventory_data[0].item_id, Uuid::from_u128(3));
        assert!(m.rez_data.from_task_id.is_nil(), "dropObject sends no FromTaskID for a notecard");
        assert_eq!(m.rez_data.bypass_raycast, 1);
        assert!(m.rez_data.remove_item);
        assert_eq!(
            (m.rez_data.everyone_mask, m.rez_data.next_owner_mask, m.rez_data.item_flags),
            (0x8000, 0x6000, 0x100)
        );
    }
}
