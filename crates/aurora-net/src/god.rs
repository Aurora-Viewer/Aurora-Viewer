//! What an administrator (a god level the region granted, `session/god.rs`)
//! sends from Advanced › Admin: the Legacy Viewer's admin menu
//! (indra/newview/llviewermenu.cpp, llselectmgr.cpp, llviewerparcelmgr.cpp,
//! originally LGPL 2.1). The region checks the god level of each.

use crate::types::RegionHandle;
use uuid::Uuid;

/// PERM_ALL (llpermissionsflags.h).
pub const PERM_ALL: u32 = 0x7FFF_FFFF;

#[derive(Debug, Clone, PartialEq)]
pub enum GodCommand {
    /// Object › Force Owner To Me: ObjectOwner with Override
    /// (LLSelectMgr::sendOwner, handle_object_owner_self).
    ForceOwner {
        handle: RegionHandle,
        local_ids: Vec<u32>,
        owner_id: Uuid,
        group_id: Uuid,
    },
    /// Object › Force Owner Permissive: ObjectPermissions with Override, all
    /// of the base then of the owner's rights (handle_object_owner_permissive).
    Permissive { handle: RegionHandle, local_ids: Vec<u32> },
    /// Object › Delete: ObjectDelete with Force (selectForceDelete).
    ForceDelete { handle: RegionHandle, local_ids: Vec<u32> },
    /// GodlikeMessage (LLSelectMgr::sendGodlikeRequest, handle_claim_public_land):
    /// a method and its parameters, the selected roots' local ids after them.
    Request {
        handle: RegionHandle,
        method: String,
        params: Vec<String>,
    },
    /// Parcel › Force Owner To Me: ParcelGodForceOwner
    /// (LLViewerParcelMgr::sendParcelGodForceOwner).
    ParcelForceOwner {
        handle: RegionHandle,
        local_id: i32,
        owner_id: Uuid,
    },
    /// Region › Save Region State: StateSave (LLFloaterGodTools / Admin.OnSaveState).
    SaveState { handle: RegionHandle },
}
