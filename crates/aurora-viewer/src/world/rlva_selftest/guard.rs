//! What a case or the cleanup may ask to send: only commands that name the
//! test's own inventory objects or, in phase 2, the prims it rezzed itself.
//! Anything else is refused here, before the send path, whatever the case
//! wanted. What changes an object (moving, duplicating, unlinking, taking,
//! its contents, a touch, a sit request) passes only as a probe that a
//! restriction must refuse, which the runner sends only once the boundary's
//! own rule says it will be refused.
use super::{Ctx, Prim, World};
use aurora_net::NetCommand;
use aurora_net::build::{BuildCmd, derez};

/// At most two prims are ever rezzed by a run.
const MAX_PRIMS: u8 = 2;

/// The prim is still the one the run rezzed: same key, same id, ours.
pub(super) fn still_own_prim(world: &World, ctx: &Ctx, prim: &Prim) -> bool {
    use crate::ui::context::flags::OBJECT_YOU_OWNER;
    world
        .objects
        .index_of_uuid(&prim.id)
        .and_then(|i| world.objects.get(i))
        .is_some_and(|o| {
            o.key.region == prim.handle
                && o.key.local_id == prim.local_id
                && o.update_flags & OBJECT_YOU_OWNER != 0
                && !o.is_avatar()
                && o.attachment_point() == 0
                && (o.owner_id.is_nil() || o.owner_id == ctx.agent)
        })
}

fn own(world: &World, ctx: &Ctx, handle: aurora_net::RegionHandle, local_ids: &[u32]) -> bool {
    !local_ids.is_empty()
        && local_ids.iter().all(|id| {
            ctx.prims
                .iter()
                .any(|p| p.handle == handle && p.local_id == *id && still_own_prim(world, ctx, p))
        })
}

/// `refused`: the command is a probe a restriction must refuse, sent only
/// when the boundary's own rule refuses it.
pub(super) fn allows(command: &NetCommand, ctx: &Ctx, world: &World, refused: bool) -> bool {
    match command {
        NetCommand::RezAttachments(items) => !items.is_empty() && items.iter().all(|i| ctx.own_item(&i.item_id)),
        NetCommand::DetachAttachments(items) => !items.is_empty() && items.iter().all(|i| ctx.own_item(i)),
        // standing up, and only from the test's own prim
        NetCommand::OneShotControl(flags) => {
            *flags == aurora_net::control::STAND_UP && (!world.agent.seated || ctx.prims.iter().any(|p| p.id == world.rlva_sit_id()))
        }
        // (the one sit of a run is RLVa's own, by force, on the test's prim)
        NetCommand::RequestSit { handle, target, .. } => {
            refused
                && ctx
                    .prims
                    .iter()
                    .any(|p| p.id == *target && p.handle == *handle && still_own_prim(world, ctx, p))
        }
        NetCommand::ObjectGrab { handle, local_id, .. } => refused && own(world, ctx, *handle, &[*local_id]),
        NetCommand::Build(build) => match build {
            // a small plain prim, close to the avatar, twice at most
            BuildCmd::Add(prim) => {
                (refused || ctx.adds < MAX_PRIMS)
                    && prim.pcode == aurora_prim::params::LL_PCODE_VOLUME
                    && prim.scale.max_element() <= 0.5
                    && prim.ray_end.distance(world.agent.position) <= 3.0
                    && Some(prim.handle) == world.main_region
            }
            BuildCmd::Deselect { handle, local_ids }
            | BuildCmd::Link { handle, local_ids }
            | BuildCmd::SetName { handle, local_ids, .. } => own(world, ctx, *handle, local_ids),
            // to the Trash: the cleanup's removal of the run's own prims
            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                ..
            } => (refused || *destination == derez::TRASH) && own(world, ctx, *handle, local_ids),
            BuildCmd::Duplicate { handle, local_ids, .. } | BuildCmd::Delink { handle, local_ids } => {
                refused && own(world, ctx, *handle, local_ids)
            }
            BuildCmd::Transform { handle, updates } => {
                refused && own(world, ctx, *handle, &updates.iter().map(|u| u.local_id).collect::<Vec<_>>())
            }
            BuildCmd::RezScript { handle, local_id, .. }
            | BuildCmd::RemoveTaskInventory { handle, local_id, .. }
            | BuildCmd::UpdateTaskInventory { handle, local_id, .. } => refused && own(world, ctx, *handle, &[*local_id]),
            _ => false,
        },
        _ => false,
    }
}
