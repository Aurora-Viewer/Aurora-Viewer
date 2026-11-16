//! What the viewer's own scripted objects are made of: a copy of a Library
//! object, worn, with a script written into it. Shared by the bridge and by
//! the RLVa self-test's controller, so that both follow FSLSLBridge
//! (indra/newview/fslslbridge.cpp) the same way.
use super::super::inventory::{FetchState, Inventory};
use aurora_net::build::{BuildCmd, TaskItem, perm};
use aurora_net::inventory::InvItem;
use aurora_net::scripts::{ScriptOptions, ScriptTarget};
use aurora_net::{AttachRequest, NetCommand, RegionHandle};
use uuid::Uuid;

/// The Library object both are copied from: small, full permissions, and in
/// every Library (FSLSLBridge's rock).
pub(crate) const ROCK: &str = "Rock - medium, round";

/// Where the search of the Library stands.
pub(crate) enum LibraryObject {
    Found(Box<InvItem>),
    /// Every Library folder is read and none holds it.
    Missing,
    /// Folders are still being read; more were asked for.
    Waiting,
}

/// Looks for the copiable, modifiable Library object named `name`. Library
/// layouts vary: its metadata only is searched, with no more than eight
/// folder requests in flight, and never a folder of the user.
pub(crate) fn library_object(inv: &mut Inventory, name: &str) -> LibraryObject {
    let library: Vec<_> = inv.folders.values().filter(|f| f.library).map(|f| f.info.id).collect();
    let found = library
        .iter()
        .filter_map(|id| inv.folders.get(id))
        .flat_map(|f| f.items.iter())
        .filter_map(|id| inv.items.get(id))
        .find(|i| i.name == name && i.asset_type == 6 && i.owner_mask & (perm::COPY | perm::MODIFY) == (perm::COPY | perm::MODIFY))
        .cloned();
    if let Some(item) = found {
        return LibraryObject::Found(Box::new(item));
    }
    if !library.is_empty()
        && library
            .iter()
            .all(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetched))
    {
        return LibraryObject::Missing;
    }
    let in_flight = library
        .iter()
        .filter(|id| inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetching))
        .count();
    let mut waiting: Vec<_> = library
        .iter()
        .filter_map(|id| inv.folders.get(id))
        .filter(|f| matches!(f.state, FetchState::Unknown | FetchState::Failed))
        .map(|f| (f.info.name != "Objects" && !f.info.name.contains("Rock"), f.info.id))
        .collect();
    waiting.sort_unstable();
    for (_, id) in waiting.into_iter().take(8usize.saturating_sub(in_flight)) {
        inv.request(id);
    }
    LibraryObject::Waiting
}

/// Wearing `item` on `point`, added beside what the point holds.
pub(crate) fn attach_request(item: &InvItem, agent: Uuid, point: u8) -> AttachRequest {
    AttachRequest {
        item_id: item.id,
        owner_id: agent,
        point,
        add: true,
        flags: item.flags,
        group_mask: item.group_mask,
        everyone_mask: item.everyone_mask,
        next_owner_mask: item.next_owner_mask,
        name: item.name.clone(),
        desc: item.desc.clone(),
    }
}

/// A new, stopped script named `name` in the contents of the object
/// `object` (local id `local_id` in `handle`). RezScript uses the active
/// group (LLViewerObject::saveScript, indra/newview/llviewerobject.cpp),
/// even for attachments.
pub(crate) fn rez_script(
    handle: RegionHandle,
    local_id: u32,
    object: Uuid,
    agent: Uuid,
    group: Uuid,
    name: &str,
    description: &str,
) -> NetCommand {
    NetCommand::Build(BuildCmd::RezScript {
        handle,
        local_id,
        enabled: false,
        group_id: group,
        item: Box::new(TaskItem {
            parent_id: object,
            creator_id: agent,
            owner_id: agent,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            next_owner_mask: perm::COPY | perm::MODIFY | perm::MOVE,
            asset_type: 10,
            inv_type: 10,
            name: name.into(),
            description: description.into(),
            ..Default::default()
        }),
    })
}

/// Writes `source` into the script `item` of `object` and starts it (Mono).
pub(crate) fn save_script(request: Uuid, handle: RegionHandle, object: Uuid, item: Uuid, source: String) -> NetCommand {
    NetCommand::SaveScript {
        request,
        target: ScriptTarget { object, item },
        handle,
        options: ScriptOptions {
            compile: aurora_net::scripts::CompileTarget::Mono,
            running: true,
            experience: Uuid::nil(),
        },
        source,
    }
}
