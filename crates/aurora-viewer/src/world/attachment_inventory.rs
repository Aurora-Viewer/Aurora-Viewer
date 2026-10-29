//! Inventory registration of simulator-arrived attachments, following
//! LLVOAvatarSelf::attachObject and LLAttachmentsMgr::linkRecentlyArrivedAttachments
//! (indra/newview/llvoavatarself.cpp, llattachmentsmgr.cpp, LGPL 2.1).

use super::{World, appearance, inventory::FetchState, temp_attachments::is_temp_attachment};
use aurora_net::{
    NetCommand,
    inventory::operations::{Mutation, Operation, Reply},
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

const RETRY_GAP: Duration = Duration::from_secs(5);
const FETCH_ATTEMPTS: u8 = 3;

#[derive(Default)]
pub struct AttachmentInventory {
    fetched: HashMap<Uuid, (Instant, u8)>,
    registered: HashSet<Uuid>,
    detached: HashSet<Uuid>,
    pending: Option<(Uuid, Vec<Uuid>)>,
    last_link: Option<Instant>,
}

impl AttachmentInventory {
    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.as_ref().is_some_and(|(id, _)| *id == request)
    }

    pub fn detaching(&mut self, items: &[Uuid]) {
        self.registered.extend(items.iter().copied());
    }

    pub fn detached(&mut self, item: Uuid) {
        self.registered.remove(&item);
        self.fetched.remove(&item);
        if self.pending.as_ref().is_some_and(|(_, items)| items.contains(&item)) {
            self.detached.insert(item);
        }
    }
}

impl World {
    /// Independent of the inventory/appearance floaters: a script can attach
    /// a newly created inventory item while both windows are closed.
    pub fn update_attachment_inventory(&mut self, now: Instant, allow_links: bool) -> Vec<NetCommand> {
        if self.teleporting || !self.movement_complete || !self.main().is_some_and(|r| r.caps.contains_key("FetchInventory2")) {
            return Vec::new();
        }
        let worn: HashSet<_> = self
            .worn_attachment_items()
            .into_iter()
            .filter(|item| {
                self.worn_attachment(item)
                    .and_then(|i| self.objects.get(i))
                    .is_some_and(|o| o.attachment_point() != 0 && !is_temp_attachment(o.full_id, Some(*item)))
            })
            .collect();
        self.attachment_inventory
            .fetched
            .retain(|id, _| worn.contains(id) && !self.inventory.items.contains_key(id));
        self.attachment_inventory.registered.retain(|id| worn.contains(id));
        if self.attachment_inventory.pending.is_none()
            && !self.attachment_inventory.detached.is_empty()
            && appearance::cof(&self.inventory).is_some_and(|cof| appearance::loaded(&self.inventory, cof))
        {
            // A failed registration can still have created its link. The
            // subsequent COF readback must also honor confirmed detaches.
            for item in std::mem::take(&mut self.attachment_inventory.detached) {
                if !worn.contains(&item)
                    && let Some(change) = appearance::unlink_attachment(&mut self.inventory, item)
                {
                    self.attachment_inventory.registered.remove(&item);
                    self.inventory_edits.push(change);
                }
            }
        }
        let mut missing = Vec::new();
        for id in &worn {
            if self.inventory.items.contains_key(id) {
                continue;
            }
            let attempt = self.attachment_inventory.fetched.entry(*id).or_insert((now, 0));
            if attempt.1 < FETCH_ATTEMPTS && (attempt.1 == 0 || now.saturating_duration_since(attempt.0) >= RETRY_GAP) {
                *attempt = (now, attempt.1 + 1);
                missing.push(*id);
            }
        }
        let mut commands = Vec::new();
        if !missing.is_empty() {
            commands.push(NetCommand::FetchItems {
                items: missing,
                owner: self.agent_id,
            });
        }
        if !allow_links
            || worn.is_empty()
            || worn.iter().all(|id| self.attachment_inventory.registered.contains(id))
            || self.attachment_inventory.pending.is_some()
            || self
                .attachment_inventory
                .last_link
                .is_some_and(|at| now.saturating_duration_since(at) < RETRY_GAP)
            || !self
                .main()
                .is_some_and(|r| r.caps.contains_key("InventoryAPIv3") && r.caps.contains_key("FetchInventoryDescendents2"))
        {
            return commands;
        }
        let Some(cof) = appearance::cof(&self.inventory) else {
            return commands;
        };
        self.inventory.request(cof);
        if !appearance::loaded(&self.inventory, cof) {
            return commands;
        }
        let linked: HashSet<_> = appearance::folder_links(&self.inventory, cof)
            .into_iter()
            .map(|link| link.target)
            .collect();
        // Track arrivals, not every unlinked worn item: an intentional COF
        // removal can precede the simulator's detach confirmation.
        self.attachment_inventory.registered.extend(worn.intersection(&linked).copied());
        let links: Vec<_> = worn
            .iter()
            .filter(|id| !self.attachment_inventory.registered.contains(id))
            .filter_map(|id| self.inventory.items.get(id))
            .filter(|item| item.asset_type == 6 && item.owner == self.agent_id)
            .map(appearance::item_link)
            .collect();
        if !links.is_empty() {
            let request = Uuid::new_v4();
            let targets = links.iter().map(|link| link.target).collect();
            self.attachment_inventory.last_link = Some(now);
            let command = NetCommand::EditInventory {
                request,
                change: Mutation {
                    operations: vec![Operation::Links { parent: cof, links }],
                    refresh: vec![cof],
                    ..Default::default()
                },
            };
            if self.rlva_allows_command(&command, Uuid::nil()) {
                self.attachment_inventory.pending = Some((request, targets));
                commands.push(command);
            }
        }
        commands
    }

    pub fn attachment_inventory_result(&mut self, request: Uuid, result: Result<Reply, String>) {
        if !self.attachment_inventory.owns(request) {
            return;
        }
        let Some((_, items)) = self.attachment_inventory.pending.take() else {
            return;
        };
        let detached = std::mem::take(&mut self.attachment_inventory.detached);
        match result {
            Ok(reply) => {
                self.inventory.remove(&reply.removed);
                self.inventory.apply(reply.contents);
                self.inventory.created_folders(reply.folders);
                let confirmed: HashSet<_> = appearance::cof(&self.inventory)
                    .into_iter()
                    .flat_map(|cof| appearance::folder_links(&self.inventory, cof))
                    .map(|link| link.target)
                    .collect();
                self.attachment_inventory
                    .registered
                    .extend(items.iter().filter(|id| confirmed.contains(id)).copied());
                // onRegisterAttachmentComplete: remove a late link only for
                // a confirmed detach. Missing objects during a teleport are
                // not evidence that an attachment was taken off.
                for item in items {
                    if detached.contains(&item)
                        && self.worn_attachment(&item).is_none()
                        && let Some(change) = appearance::unlink_attachment(&mut self.inventory, item)
                    {
                        self.attachment_inventory.registered.remove(&item);
                        self.inventory_edits.push(change);
                    }
                }
            }
            Err(reason) => {
                log::warn!("attachment inventory registration failed: {reason}");
                self.attachment_inventory.detached.extend(detached);
                // The write may have succeeded before readback failed. Read
                // the COF again before attempting another additive link.
                if let Some(cof) = appearance::cof(&self.inventory) {
                    if let Some(folder) = self.inventory.folders.get_mut(&cof) {
                        folder.state = FetchState::Unknown;
                    }
                    self.inventory.request(cof);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::{
        NetEvent,
        inventory::{FolderContents, InvItem},
        objects::ObjectUpdate,
    };
    use std::sync::Arc;

    fn fixture() -> (World, ObjectUpdate, InvItem) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let avatar = crate::demo::action_avatar(false);
        world.agent_id = avatar.full_id;
        world.main_region = Some(1);
        world.movement_complete = true;
        world.ensure_region(1, (256, 256)).caps = Arc::new(
            ["FetchInventory2", "FetchInventoryDescendents2", "InventoryAPIv3"]
                .into_iter()
                .map(|name| (name.to_owned(), "x".to_owned()))
                .collect(),
        );
        appearance::seed_demo(&mut world.inventory, world.agent_id);
        let parent = Uuid::from_u128(90_010);
        world.inventory.folders.insert(
            parent,
            super::super::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: parent,
                    name: "Objects".to_owned(),
                    type_default: 6,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        let item = InvItem {
            id: Uuid::from_u128(90_001),
            parent,
            name: "Script-attached object".to_owned(),
            ..world.inventory.items[&Uuid::from_u128(717)].clone()
        };
        let mut object = crate::demo::hud::object(31, false);
        object.parent_id = avatar.local_id;
        object.name_values = format!("AttachItemID STRING RW SV {}", item.id);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![avatar, object.clone()],
        });
        (world, object, item)
    }

    fn linked_reply(world: &World, item: &InvItem) -> Reply {
        let cof = appearance::cof(&world.inventory).expect("COF");
        let folder = &world.inventory.folders[&cof];
        let mut items: Vec<_> = folder.items.iter().map(|id| world.inventory.items[id].clone()).collect();
        items.push(InvItem {
            id: Uuid::from_u128(90_002),
            parent: cof,
            asset_id: item.id,
            asset_type: 24,
            ..item.clone()
        });
        Reply {
            contents: vec![FolderContents {
                folder_id: cof,
                owner_id: world.agent_id,
                version: folder.info.version + 1,
                folders: Vec::new(),
                items,
            }],
            folders: Vec::new(),
            moves_queued: false,
            refresh: vec![cof],
            copies: Vec::new(),
            removed: Vec::new(),
            created: HashMap::new(),
        }
    }

    #[test]
    fn attachment_inventory_recovers_an_unexpected_item_and_registers_it_once() {
        let (mut world, object, item) = fixture();
        let now = Instant::now();
        let cof = appearance::cof(&world.inventory).expect("COF");
        let before = appearance::folder_links(&world.inventory, cof);
        assert!(matches!(&world.update_attachment_inventory(now, true)[..],
            [NetCommand::FetchItems { items, owner }] if items == &[item.id] && *owner == world.agent_id));
        assert!(world.update_attachment_inventory(now, true).is_empty());
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        assert!(world.inventory.folders[&item.parent].items.contains(&item.id));
        let commands = world.update_attachment_inventory(now, true);
        let [NetCommand::EditInventory { request, change }] = &commands[..] else {
            panic!("register arrival");
        };
        assert!(matches!(&change.operations[..], [Operation::Links { parent, links }]
            if *parent == cof && links.len() == 1 && links[0].target == item.id));
        assert_eq!(
            appearance::folder_links(&world.inventory, cof),
            before,
            "wait for server confirmation"
        );
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object],
        });
        assert!(world.update_attachment_inventory(now + RETRY_GAP, true).is_empty());
        world.attachment_inventory_result(*request, Ok(linked_reply(&world, &item)));
        assert!(world.update_attachment_inventory(now + RETRY_GAP * 2, true).is_empty());
        assert!(
            appearance::plan(
                &world.inventory,
                appearance::Action::Remove(item.id),
                &world.worn_attachment_points()
            )
            .is_ok()
        );
        appearance::unlink_attachment(&mut world.inventory, item.id);
        assert!(
            world.update_attachment_inventory(now + RETRY_GAP * 3, true).is_empty(),
            "a requested detach must not be undone"
        );
    }

    #[test]
    fn attachment_inventory_retries_a_delayed_item_without_requesting_each_tick() {
        let (mut world, _, item) = fixture();
        let now = Instant::now();
        for attempt in 0..u32::from(FETCH_ATTEMPTS) {
            assert!(matches!(&world.update_attachment_inventory(now + RETRY_GAP * attempt, true)[..],
                [NetCommand::FetchItems { items, .. }] if items == &[item.id]));
            world.apply(NetEvent::InventoryItemsFailed { items: vec![item.id] });
            assert!(
                world
                    .update_attachment_inventory(now + RETRY_GAP * attempt + Duration::from_millis(500), true)
                    .is_empty()
            );
        }
        assert!(world.update_attachment_inventory(now + RETRY_GAP * 10, true).is_empty());
        world.apply(NetEvent::InventoryUpdated {
            folders: Vec::new(),
            items: vec![item],
        });
        assert!(matches!(
            &world.update_attachment_inventory(now + RETRY_GAP * 11, true)[..],
            [NetCommand::EditInventory { .. }]
        ));
    }

    #[test]
    fn attachment_inventory_ignores_temporary_other_avatar_and_child_objects() {
        for kind in 0..4 {
            let (mut world, mut object, _) = fixture();
            match kind {
                0 => object.name_values = format!("AttachItemID STRING RW SV {}", object.full_id),
                1 => {
                    let mut other = crate::demo::action_avatar(false);
                    other.full_id = Uuid::from_u128(90_003);
                    other.local_id += 1;
                    object.parent_id = other.local_id;
                    world.apply(NetEvent::ObjectUpdates {
                        handle: 1,
                        objects: vec![other],
                    });
                }
                2 => object.parent_id = 999,
                _ => object.name_values.clear(),
            }
            // A new root makes this a complete update, including absent name-values.
            let old = world.objects.index_of_uuid(&object.full_id).expect("object");
            world.objects.remove_idx(old);
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![object],
            });
            assert!(world.update_attachment_inventory(Instant::now(), true).is_empty(), "kind {kind}");
        }
    }

    #[test]
    fn attachment_inventory_waits_for_the_cof_and_interactive_outfit_changes() {
        let (mut world, _, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item]));
        assert!(world.update_attachment_inventory(now, false).is_empty());
        let cof = appearance::cof(&world.inventory).expect("COF");
        world.inventory.folders.get_mut(&cof).expect("COF").state = FetchState::Unknown;
        assert!(world.update_attachment_inventory(now, true).is_empty());
        assert!(world.inventory.queue.contains(&(cof, false)));
        world.inventory.folders.get_mut(&cof).expect("COF").state = FetchState::Fetched;
        assert!(matches!(
            &world.update_attachment_inventory(now, true)[..],
            [NetCommand::EditInventory { .. }]
        ));
    }

    #[test]
    fn attachment_inventory_removes_a_link_confirmed_after_detachment() {
        for killed in [true, false] {
            let (mut world, mut object, item) = fixture();
            let now = Instant::now();
            world.apply(NetEvent::InventoryItems(vec![item.clone()]));
            let commands = world.update_attachment_inventory(now, true);
            let [NetCommand::EditInventory { request, .. }] = &commands[..] else {
                panic!("registration");
            };
            let reply = linked_reply(&world, &item);
            let mut replacement = object.clone();
            replacement.full_id = Uuid::from_u128(90_004);
            replacement.local_id += 1;
            if killed {
                world.apply(NetEvent::ObjectsKilled {
                    handle: 1,
                    local_ids: vec![object.local_id],
                });
            } else {
                object.parent_id = 0;
                world.apply(NetEvent::ObjectUpdates {
                    handle: 1,
                    objects: vec![object],
                });
            }
            world.attachment_inventory_result(*request, Ok(reply));
            let cof = appearance::cof(&world.inventory).expect("COF");
            assert!(
                appearance::folder_links(&world.inventory, cof)
                    .iter()
                    .all(|link| link.target != item.id)
            );
            assert!(world.inventory.items.contains_key(&item.id), "keep original");
            assert!(matches!(&world.take_inventory_edits()[..], [Mutation { operations, .. }]
                if matches!(&operations[..], [Operation::Delete { folder: false, .. }])));
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![replacement],
            });
            assert!(matches!(
                &world.update_attachment_inventory(now + RETRY_GAP, true)[..],
                [NetCommand::EditInventory { .. }]
            ));
        }
    }

    #[test]
    fn attachment_inventory_reads_back_before_retrying_an_uncertain_link_write() {
        let (mut world, _, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        let commands = world.update_attachment_inventory(now, true);
        let [NetCommand::EditInventory { request, .. }] = &commands[..] else {
            panic!("registration");
        };
        world.attachment_inventory_result(*request, Err("readback failed".to_owned()));
        assert!(world.update_attachment_inventory(now + RETRY_GAP, true).is_empty());
        // The server did accept the link, so readback prevents a duplicate.
        world.inventory.apply(linked_reply(&world, &item).contents);
        assert!(world.update_attachment_inventory(now + RETRY_GAP * 2, true).is_empty());
    }

    #[test]
    fn attachment_inventory_respects_a_requested_detach_then_registers_a_new_arrival() {
        let (mut world, mut object, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        world.note_outfit_command(&NetCommand::DetachAttachments(vec![item.id]));
        assert!(world.update_attachment_inventory(now, true).is_empty());
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![object.local_id],
        });
        object.full_id = Uuid::from_u128(90_004);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object],
        });
        assert!(matches!(
            &world.update_attachment_inventory(now, true)[..],
            [NetCommand::EditInventory { .. }]
        ));
    }

    #[test]
    fn attachment_inventory_does_not_register_an_item_fetched_after_it_detaches() {
        let (mut world, object, item) = fixture();
        let now = Instant::now();
        world.update_attachment_inventory(now, true);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![object.local_id],
        });
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        assert!(world.update_attachment_inventory(now + RETRY_GAP, true).is_empty());
        assert!(world.inventory.items.contains_key(&item.id));
    }

    #[test]
    fn attachment_inventory_does_not_keep_a_pending_write_rejected_by_rlva() {
        let (mut world, _, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item]));
        world.rlva_configure(true);
        let mut chat = aurora_net::ChatMessage {
            from_name: "Controller".to_owned(),
            source_id: Uuid::from_u128(90_020),
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "@addattach=n".to_owned(),
        };
        assert!(world.rlva.receive(&chat));
        assert!(world.update_attachment_inventory(now, true).is_empty());
        assert!(world.attachment_inventory.pending.is_none());
        chat.message = "@addattach=y".to_owned();
        assert!(world.rlva.receive(&chat));
        assert!(matches!(
            &world.update_attachment_inventory(now + RETRY_GAP, true)[..],
            [NetCommand::EditInventory { .. }]
        ));
    }

    #[test]
    fn attachment_inventory_preserves_a_late_link_while_our_avatar_is_missing_during_teleport() {
        let (mut world, _, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        let commands = world.update_attachment_inventory(now, true);
        let [NetCommand::EditInventory { request, .. }] = &commands[..] else {
            panic!("registration");
        };
        let reply = linked_reply(&world, &item);
        world.teleporting = true;
        let avatar = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
        world.objects.remove_idx(avatar);
        world.attachment_inventory_result(*request, Ok(reply));
        let cof = appearance::cof(&world.inventory).expect("COF");
        assert!(
            appearance::folder_links(&world.inventory, cof)
                .iter()
                .any(|link| link.target == item.id)
        );
        assert!(world.take_inventory_edits().is_empty());
    }

    #[test]
    fn attachment_inventory_cleans_up_a_detach_after_uncertain_creation_and_readback() {
        let (mut world, object, item) = fixture();
        let now = Instant::now();
        world.apply(NetEvent::InventoryItems(vec![item.clone()]));
        let commands = world.update_attachment_inventory(now, true);
        let [NetCommand::EditInventory { request, .. }] = &commands[..] else {
            panic!("registration");
        };
        let reply = linked_reply(&world, &item);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![object.local_id],
        });
        world.attachment_inventory_result(*request, Err("readback failed".to_owned()));
        world.apply(NetEvent::InventoryContents(reply.contents));
        assert!(world.update_attachment_inventory(now + RETRY_GAP, true).is_empty());
        let cof = appearance::cof(&world.inventory).expect("COF");
        assert!(
            appearance::folder_links(&world.inventory, cof)
                .iter()
                .all(|link| link.target != item.id)
        );
        assert_eq!(world.take_inventory_edits().len(), 1);
        assert!(world.inventory.items.contains_key(&item.id));
    }
}
