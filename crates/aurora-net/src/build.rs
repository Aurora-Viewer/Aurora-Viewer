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
