//! Building and land editing: the commands of the build tools (selection,
//! moves, new prims, links, terraforming) and the object properties the
//! simulator sends back for selected objects.
//!
//! Message use follows LLSelectMgr, LLToolPlacer and LLToolBrushLand
//! (Firestorm sources, originally LGPL 2.1).

use crate::types::RegionHandle;
use aurora_prim::params::RawShape;
use glam::{Quat, Vec3};
use uuid::Uuid;

/// MultipleObjectUpdate `Type` bits (llprimitive/llprimitive.h UPD_*).
pub mod upd {
    pub const POSITION: u8 = 0x01;
    pub const ROTATION: u8 = 0x02;
    pub const SCALE: u8 = 0x04;
    /// Move the whole linkset (root) instead of one child.
    pub const LINKED_SETS: u8 = 0x08;
    /// Scale around the center (both sides).
    pub const UNIFORM: u8 = 0x10;
}

/// DeRezObject destinations (llinventory/lldestinationtypes / DeRezDestination).
pub mod derez {
    pub const SAVE_INTO_AGENT_INVENTORY: u8 = 0;
    pub const ACQUIRE_TO_AGENT_INVENTORY: u8 = 1;
    pub const SAVE_INTO_TASK_INVENTORY: u8 = 2;
    pub const ATTACHMENT: u8 = 3;
    pub const TAKE_INTO_AGENT_INVENTORY: u8 = 4;
    pub const FORCE_TO_GOD_INVENTORY: u8 = 5;
    pub const TRASH: u8 = 6;
    pub const ATTACHMENT_TO_INV: u8 = 7;
    pub const ATTACHMENT_EXISTS: u8 = 8;
    pub const RETURN_TO_OWNER: u8 = 9;
    pub const RETURN_TO_LAST_OWNER: u8 = 10;
}

/// ModifyLand actions (LLToolBrushLand E_LAND_LEVEL...).
pub mod land_action {
    pub const FLATTEN: u8 = 0;
    pub const RAISE: u8 = 1;
    pub const LOWER: u8 = 2;
    pub const SMOOTH: u8 = 3;
    pub const ROUGHEN: u8 = 4;
    pub const REVERT: u8 = 5;
}

/// One object transform change (packed like LLSelectMgr::packMultipleUpdate:
/// position, rotation, scale in that order; children relative to their root).
#[derive(Debug, Clone, Copy)]
pub struct TransformUpdate {
    pub local_id: u32,
    pub position: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    /// UPD_LINKED_SETS: the root moves its whole linkset.
    pub linked: bool,
    /// UPD_UNIFORM: scaled around its center.
    pub uniform: bool,
}

impl TransformUpdate {
    pub fn kind(&self) -> u8 {
        let mut t = 0;
        if self.position.is_some() {
            t |= upd::POSITION;
        }
        if self.rotation.is_some() {
            t |= upd::ROTATION;
        }
        if self.scale.is_some() {
            t |= upd::SCALE;
        }
        if self.linked {
            t |= upd::LINKED_SETS;
        }
        if self.uniform {
            t |= upd::UNIFORM;
        }
        t
    }

    /// The `Data` bytes.
    pub fn data(&self) -> Vec<u8> {
        let mut d = Vec::with_capacity(36);
        let v3 = |d: &mut Vec<u8>, v: Vec3| {
            for c in v.to_array() {
                d.extend_from_slice(&c.to_le_bytes());
            }
        };
        if let Some(p) = self.position {
            v3(&mut d, p);
        }
        if let Some(r) = self.rotation {
            // LLQuaternion::packToVector3: w made positive, then dropped
            let r = r.normalize();
            let r = if r.w < 0.0 { -r } else { r };
            v3(&mut d, Vec3::new(r.x, r.y, r.z));
        }
        if let Some(s) = self.scale {
            v3(&mut d, s);
        }
        d
    }
}

/// A new prim (ObjectAdd, as LLToolPlacer::addObject sends it).
#[derive(Debug, Clone)]
pub struct NewPrim {
    pub handle: RegionHandle,
    pub group_id: Uuid,
    pub pcode: u8,
    pub material: u8,
    pub add_flags: u32,
    pub shape: RawShape,
    /// Region-local ray from the camera to the clicked surface.
    pub ray_start: Vec3,
    pub ray_end: Vec3,
    pub ray_target: Uuid,
    pub ray_end_is_intersection: bool,
    pub bypass_raycast: bool,
    pub scale: Vec3,
    pub rotation: Quat,
    pub state: u8,
}

#[derive(Debug, Clone)]
pub enum BuildCmd {
    /// ObjectSelect (the simulator answers with ObjectProperties).
    Select {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    Deselect {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    /// MultipleObjectUpdate.
    Transform {
        handle: RegionHandle,
        updates: Vec<TransformUpdate>,
    },
    Add(Box<NewPrim>),
    /// ObjectDuplicate (offset in region meters; flags FLAGS_CREATE_SELECTED...).
    Duplicate {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        offset: Vec3,
        flags: u32,
        group_id: Uuid,
    },
    /// DeRezObject: delete (trash), take, take a copy, return.
    DeRez {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        destination: u8,
        destination_id: Uuid,
        group_id: Uuid,
    },
    /// ObjectLink (the first id becomes the root).
    Link {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    Delink {
        handle: RegionHandle,
        local_ids: Vec<u32>,
    },
    SetName {
        handle: RegionHandle,
        local_id: u32,
        name: String,
    },
    SetDescription {
        handle: RegionHandle,
        local_id: u32,
        description: String,
    },
    /// ObjectFlagUpdate (physical, temporary, phantom).
    SetFlags {
        handle: RegionHandle,
        local_id: u32,
        physics: bool,
        temporary: bool,
        phantom: bool,
    },
    /// ObjectShape (path / profile parameters).
    SetShape {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        shape: RawShape,
    },
    /// ObjectMaterial (LL_MCODE_*).
    SetMaterial {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        material: u8,
    },
    /// Undo / Redo of the simulator's per-object history.
    Undo {
        handle: RegionHandle,
        ids: Vec<Uuid>,
        group_id: Uuid,
    },
    Redo {
        handle: RegionHandle,
        ids: Vec<Uuid>,
        group_id: Uuid,
    },
    /// ModifyLand. Region-local rectangle (a point for the brush: west = east).
    ModifyLand {
        handle: RegionHandle,
        action: u8,
        /// Real brush size (ModifyBlockExtended) and its legacy index.
        brush_size: f32,
        seconds: f32,
        height: f32,
        parcel_local_id: i32,
        west: f32,
        south: f32,
        east: f32,
        north: f32,
    },
    /// UndoLand: undo the last land change of a region.
    UndoLand {
        handle: RegionHandle,
    },
    /// ObjectFlagUpdate with its ExtraPhysics block (physics shape type,
    /// density, friction, restitution, gravity: LLSelectMgr::selectionUpdatePhysicsParam).
    SetPhysicsParams {
        handle: RegionHandle,
        local_id: u32,
        physics: bool,
        temporary: bool,
        phantom: bool,
        params: PhysicsParams,
    },
    /// ObjectPermissions (field `perm_field::*`, set or clear `mask`).
    SetPermissions {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        field: u8,
        set: bool,
        mask: u32,
    },
    /// ObjectGroup.
    SetGroup {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        group_id: Uuid,
    },
    /// ObjectOwner: deed to the group (LLSelectMgr::sendOwner with override false).
    SetOwner {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        owner_id: Uuid,
        group_id: Uuid,
    },
    /// ObjectSaleInfo (sale type 0 = not for sale).
    SetSaleInfo {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        sale_type: u8,
        price: i32,
    },
    /// ObjectClickAction (CLICK_ACTION_*).
    SetClickAction {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        action: u8,
    },
    /// ObjectIncludeInSearch.
    SetIncludeInSearch {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        include: bool,
    },
    /// ObjectImage: the whole packed TextureEntry and the media URL
    /// (LLPrimitive::packTEMessage, LLViewerObject::sendTEUpdate).
    SetTextures {
        handle: RegionHandle,
        local_id: u32,
        media_url: String,
        texture_entry: Vec<u8>,
    },
    /// ObjectExtraParams: one parameter block (aurora_prim::extra PARAMS_*).
    SetExtraParam {
        handle: RegionHandle,
        local_id: u32,
        param_type: u16,
        in_use: bool,
        data: Vec<u8>,
    },
    SpinStart {
        handle: RegionHandle,
        object_id: Uuid,
    },
    /// ObjectSpinUpdate: absolute rotation.
    SpinUpdate {
        handle: RegionHandle,
        object_id: Uuid,
        rotation: Quat,
    },
    SpinStop {
        handle: RegionHandle,
        object_id: Uuid,
    },
    /// ObjectDuplicateOnRay: copies of the selection where the ray hits
    /// (LLSelectMgr::selectDuplicateOnRay).
    DuplicateOnRay {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        group_id: Uuid,
        ray_start: Vec3,
        ray_end: Vec3,
        ray_target: Uuid,
        bypass_raycast: bool,
        copy_centers: bool,
        copy_rotates: bool,
    },
    /// ParcelPropertiesRequest for a region-local rectangle (the simulator
    /// answers with ParcelProperties: prim counts, owner...).
    ParcelRequest {
        handle: RegionHandle,
        sequence: i32,
        /// The simulator widens the rectangle to the whole parcel.
        snap: bool,
        west: f32,
        south: f32,
        east: f32,
        north: f32,
    },
    /// ParcelDivide / ParcelJoin of a region-local rectangle.
    ParcelDivide {
        handle: RegionHandle,
        west: f32,
        south: f32,
        east: f32,
        north: f32,
    },
    ParcelJoin {
        handle: RegionHandle,
        west: f32,
        south: f32,
        east: f32,
        north: f32,
    },
    /// ParcelRelease (abandon to the estate owner).
    ParcelRelease {
        handle: RegionHandle,
        local_id: i32,
    },
    /// RequestTaskInventory: the simulator answers with ReplyTaskInventory
    /// (the name of a file to download with Xfer).
    RequestTaskInventory {
        handle: RegionHandle,
        local_id: u32,
        object_id: Uuid,
    },
    RemoveTaskInventory {
        handle: RegionHandle,
        local_id: u32,
        item_id: Uuid,
    },
    /// MoveTaskInventory: copy an object's item into the agent's inventory.
    MoveTaskInventory {
        handle: RegionHandle,
        local_id: u32,
        item_id: Uuid,
        folder_id: Uuid,
    },
    /// UpdateTaskInventory (key 0 = inventory item): put / rename an item.
    UpdateTaskInventory {
        handle: RegionHandle,
        local_id: u32,
        item: Box<TaskItem>,
    },
    /// RezScript: a script into an object, running or not.
    RezScript {
        handle: RegionHandle,
        local_id: u32,
        enabled: bool,
        group_id: Uuid,
        item: Box<TaskItem>,
    },
    ScriptReset {
        handle: RegionHandle,
        object_id: Uuid,
        item_id: Uuid,
    },
    SetScriptRunning {
        handle: RegionHandle,
        object_id: Uuid,
        item_id: Uuid,
        running: bool,
    },
    GetScriptRunning {
        handle: RegionHandle,
        object_id: Uuid,
        item_id: Uuid,
    },
    /// LLSD request to a region capability (GetObjectCost, ResourceCostSelected,
    /// GetObjectPhysicsData, ObjectMedia, ModifyMaterialParams, RenderMaterials):
    /// the reply comes back as NetEvent::CapReply with the same `tag`.
    Cap {
        handle: RegionHandle,
        cap: String,
        put: bool,
        body: aurora_llsd::Llsd,
        tag: u64,
    },
}

/// ParcelPropertiesRequest sequence ids of the build tools start here
/// (the agent's own parcel updates come with 0): their replies come back
/// as NetEvent::SelectedParcel.
pub const BUILD_PARCEL_SEQ: i32 = 1000;

/// ObjectPermissions `Field` (llpermissionsflags.h PERM_BASE...).
pub mod perm_field {
    pub const BASE: u8 = 0x01;
    pub const OWNER: u8 = 0x02;
    pub const GROUP: u8 = 0x04;
    pub const EVERYONE: u8 = 0x08;
    pub const NEXT_OWNER: u8 = 0x10;
}

/// ExtraPhysics of ObjectFlagUpdate (LLPhysicsShapeType and material values).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysicsParams {
    /// 0 prim, 1 none, 2 convex hull (LLViewerObject::PHYSICS_SHAPE_*).
    pub shape_type: u8,
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub gravity_multiplier: f32,
}

impl Default for PhysicsParams {
    /// DEFAULT_DENSITY etc. (llprimitive.h).
    fn default() -> Self {
        PhysicsParams {
            shape_type: 0,
            density: 1000.0,
            friction: 0.6,
            restitution: 0.5,
            gravity_multiplier: 1.0,
        }
    }
}

/// An item of an object's inventory (task inventory), as listed in the
/// file the simulator sends (LLInventoryItem::importLegacyStream) and as
/// sent with UpdateTaskInventory / RezScript (packMessage).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskItem {
    pub item_id: Uuid,
    pub parent_id: Uuid,
    pub creator_id: Uuid,
    pub owner_id: Uuid,
    pub last_owner_id: Uuid,
    pub group_id: Uuid,
    pub group_owned: bool,
    pub base_mask: u32,
    pub owner_mask: u32,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
    pub asset_id: Uuid,
    /// LLAssetType (as a number).
    pub asset_type: i8,
    /// LLInventoryType.
    pub inv_type: i8,
    pub flags: u32,
    pub sale_type: u8,
    pub sale_price: i32,
    pub name: String,
    pub description: String,
    pub creation_date: i32,
    /// A folder (the inventory's root "Contents" category) rather than an item.
    pub is_folder: bool,
}

impl TaskItem {
    /// The item as an inventory item (object contents window of the click
    /// actions); None for the folder.
    pub fn to_inv_item(&self) -> Option<crate::inventory::InvItem> {
        (!self.is_folder).then(|| crate::inventory::InvItem {
            id: self.item_id,
            parent: self.parent_id,
            name: self.name.clone(),
            desc: self.description.clone(),
            asset_type: self.asset_type as i32,
            inv_type: self.inv_type as i32,
            asset_id: self.asset_id,
            flags: self.flags,
            favorite: false,
            creator: self.creator_id,
            created_at: self.creation_date as i64,
            owner: self.owner_id,
            group_mask: self.group_mask,
            everyone_mask: self.everyone_mask,
            next_owner_mask: self.next_owner_mask,
        })
    }
}

/// ObjectProperties (selected objects) or ObjectPropertiesFamily (hover /
/// pie menu), as LLSelectMgr::processObjectProperties reads them.
#[derive(Debug, Clone, Default)]
pub struct ObjectProps {
    pub object_id: Uuid,
    /// Full properties (selection) rather than the family subset.
    pub full: bool,
    pub creator_id: Uuid,
    pub owner_id: Uuid,
    pub group_id: Uuid,
    pub last_owner_id: Uuid,
    /// Microseconds since the epoch.
    pub creation_date: u64,
    pub base_mask: u32,
    pub owner_mask: u32,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
    pub sale_type: u8,
    pub sale_price: i32,
    pub category: u32,
    pub inventory_serial: i16,
    pub name: String,
    pub description: String,
    pub touch_name: String,
    pub sit_name: String,
}

/// Permission bits (llinventory/llpermissionsflags.h).
pub mod perm {
    pub const TRANSFER: u32 = 1 << 13;
    pub const MODIFY: u32 = 1 << 14;
    pub const COPY: u32 = 1 << 15;
    pub const MOVE: u32 = 1 << 19;
}

/// Legacy brush size index (LLToolBrushLand::getBrushIndex, deprecated:
/// the real size goes in ModifyBlockExtended).
pub fn brush_index(size: f32) -> u8 {
    const SIZES: [f32; 3] = [1.0, 2.0, 4.0];
    let mut idx = 0u8;
    for (i, s) in SIZES.iter().enumerate() {
        if size > *s {
            idx = i as u8;
        }
    }
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_position_rotation_scale_in_order() {
        let u = TransformUpdate {
            local_id: 1,
            position: Some(Vec3::new(1.0, 2.0, 3.0)),
            rotation: Some(Quat::from_xyzw(0.0, 0.0, 0.0, -1.0)),
            scale: Some(Vec3::splat(0.5)),
            linked: true,
            uniform: false,
        };
        assert_eq!(u.kind(), upd::POSITION | upd::ROTATION | upd::SCALE | upd::LINKED_SETS);
        let d = u.data();
        assert_eq!(d.len(), 36);
        assert_eq!(f32::from_le_bytes(d[8..12].try_into().unwrap()), 3.0);
        assert_eq!(f32::from_le_bytes(d[24..28].try_into().unwrap()), 0.5);
    }

    #[test]
    fn brush_indices() {
        assert_eq!(brush_index(1.0), 0);
        assert_eq!(brush_index(2.0), 0);
        assert_eq!(brush_index(3.0), 1);
        assert_eq!(brush_index(8.0), 2);
    }
}
