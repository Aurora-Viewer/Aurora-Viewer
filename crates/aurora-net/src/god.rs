//! What an administrator (a god level the region granted, `session/god.rs`)
//! sends from Advanced › Admin: the Legacy Viewer's admin menu
//! (indra/newview/llviewermenu.cpp, llselectmgr.cpp, llviewerparcelmgr.cpp,
//! originally LGPL 2.1). The region checks the god level of each.

use crate::types::RegionHandle;
use uuid::Uuid;

/// PERM_ALL (llpermissionsflags.h).
pub const PERM_ALL: u32 = 0x7FFF_FFFF;

/// GodKickUser's KickFlags (KICK_FLAGS_*, llavataractions.cpp):
/// logged off, frozen, unfrozen.
pub const KICK_DEFAULT: u32 = 0;
pub const KICK_FREEZE: u32 = 1;
pub const KICK_UNFREEZE: u32 = 2;

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
    /// `params` head every packet (packGodlikeHead); only `objects` is split
    /// across packets (sendListToRegions, packObjectIDAsParam).
    Request {
        handle: RegionHandle,
        method: String,
        params: Vec<String>,
        objects: Vec<u32>,
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
    /// A resident's Admin › Kick / Freeze / Unfreeze: GodKickUser with
    /// `KICK_*` and the message they read (LLAvatarActions::handleKick,
    /// handleFreeze, handleUnfreeze); the region finds them on the grid.
    KickUser {
        handle: RegionHandle,
        agent_id: Uuid,
        flags: u32,
        reason: String,
    },
}
