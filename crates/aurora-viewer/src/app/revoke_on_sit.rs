//! Revoke the permissions of the object we sit on, when we sit down, stand
//! up, or both (FSRevokePerms, in Debug Settings).
//!
//! Port of the FSRevokePerms checks in LLVOAvatar::sitOnObject and
//! LLVOAvatar::getOffObject, with LLVOAvatar::revokePermissionsOnObject
//! (indra/newview/llvoavatar.cpp), originally LGPL 2.1: for our own avatar
//! only, an object we do not own (permYouOwner) gets one RevokePermissions
//! with every bit, so that its scripts lose what they were granted, the
//! animation permission among them.
//!
//! Aurora has no event for "sat down" and "got off": the prim our avatar
//! is parented to is compared with the last frame's. A change from one seat
//! to another is a stand from the first and a sit on the second, as the
//! Legacy Viewer gets off the first object before sitting on the next.

use super::App;
use crate::ui::context::flags::OBJECT_YOU_OWNER;
use aurora_net::NetCommand;
use uuid::Uuid;

/// Every permission bit, as revokePermissionsOnObject sends.
const ALL_PERMISSIONS: u32 = u32::MAX;

/// When to revoke (FSRevokePerms: 0 never, 1 on sit, 2 on stand, 3 both).
pub fn on_sit(mode: u8) -> bool {
    mode == 1 || mode == 3
}

pub fn on_stand(mode: u8) -> bool {
    mode == 2 || mode == 3
}

/// The prim we sit on, and whether it is ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub id: Uuid,
    pub ours: bool,
}

/// The objects to revoke for a change of seat from `before` to `now`: the
/// one we got off (as it is now, when it is still there), then the one we
/// sat on.
pub fn revokes(before: Option<Uuid>, now: Option<Seat>, left: Option<Seat>, mode: u8) -> Vec<Uuid> {
    let mut out = Vec::new();
    if before == now.map(|s| s.id) {
        return out;
    }
    if before.is_some()
        && on_stand(mode)
        && let Some(seat) = left
        && !seat.ours
    {
        out.push(seat.id);
    }
    if let Some(seat) = now
        && on_sit(mode)
        && !seat.ours
    {
        out.push(seat.id);
    }
    out
}

impl App {
    /// The prim object `id` is, as a seat: None when it is not known (gone).
    fn seat_of(&self, id: Uuid) -> Option<Seat> {
        let o = self.world.objects.get(self.world.objects.index_of_uuid(&id)?)?;
        Some(Seat {
            id,
            ours: o.update_flags & OBJECT_YOU_OWNER != 0,
        })
    }

    /// Each frame: our seat compared with the last frame's.
    pub(super) fn revoke_on_sit(&mut self) {
        let now = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .and_then(|avatar| self.world.objects.parent_of(avatar))
            .and_then(|i| self.world.objects.get(i))
            .map(|seat| seat.full_id);
        let before = std::mem::replace(&mut self.last_seat, now);
        if before == now {
            return;
        }
        let left = before.and_then(|id| self.seat_of(id));
        let seat = now.and_then(|id| self.seat_of(id));
        for target in revokes(before, seat, left, self.settings.revoke_permissions) {
            log::info!("revoke on sit: the permissions of {target} revoked (seat {before:?} to {now:?})");
            self.send(NetCommand::RevokePermissions {
                target,
                permissions: ALL_PERMISSIONS,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIR: Uuid = Uuid::from_u128(1);
    const SOFA: Uuid = Uuid::from_u128(2);

    fn theirs(id: Uuid) -> Seat {
        Seat { id, ours: false }
    }

    #[test]
    fn each_mode_revokes_when_the_legacy_viewer_does() {
        // (mode, on sit, on stand)
        for (mode, sit, stand) in [(0, false, false), (1, true, false), (2, false, true), (3, true, true)] {
            let sat = revokes(None, Some(theirs(CHAIR)), None, mode);
            assert_eq!(sat == [CHAIR], sit, "mode {mode} sitting");
            assert_eq!(sat.is_empty(), !sit, "mode {mode} sitting");
            let stood = revokes(Some(CHAIR), None, Some(theirs(CHAIR)), mode);
            assert_eq!(stood == [CHAIR], stand, "mode {mode} standing");
            assert_eq!(stood.is_empty(), !stand, "mode {mode} standing");
        }
    }

    #[test]
    fn our_own_objects_and_vanished_seats_are_left_alone() {
        let ours = Seat { id: CHAIR, ours: true };
        assert!(revokes(None, Some(ours), None, 3).is_empty());
        assert!(revokes(Some(CHAIR), None, Some(ours), 3).is_empty());
        // the seat is gone when we stand: nothing to send it to
        assert!(revokes(Some(CHAIR), None, None, 3).is_empty());
    }

    #[test]
    fn staying_seated_sends_nothing_and_a_new_seat_is_a_stand_then_a_sit() {
        assert!(revokes(Some(CHAIR), Some(theirs(CHAIR)), Some(theirs(CHAIR)), 3).is_empty());
        assert!(revokes(None, None, None, 3).is_empty());
        assert_eq!(revokes(Some(CHAIR), Some(theirs(SOFA)), Some(theirs(CHAIR)), 3), [CHAIR, SOFA]);
        assert_eq!(revokes(Some(CHAIR), Some(theirs(SOFA)), Some(theirs(CHAIR)), 1), [SOFA]);
        assert_eq!(revokes(Some(CHAIR), Some(theirs(SOFA)), Some(theirs(CHAIR)), 2), [CHAIR]);
    }
}
