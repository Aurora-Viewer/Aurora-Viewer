//! Inventory registration of simulator-arrived attachments, following
//! LLVOAvatarSelf::attachObject and LLAttachmentsMgr::linkRecentlyArrivedAttachments
//! (indra/newview/llvoavatarself.cpp, llattachmentsmgr.cpp, LGPL 2.1).

use super::{World, appearance, inventory::FetchState};
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
    /// LLSelectMgr::sendDetach (indra/newview/llselectmgr.cpp): an object
    /// actually worn by us can be detached by its simulator-local root even
    /// when its inventory record or the rest of the outfit cannot be fetched.
    fn attachment_detach_command(&self, item: Uuid) -> Option<NetCommand> {
        let item = super::inventory::actions::original(&self.inventory, item)
            .or_else(|| {
                self.inventory
                    .items
                    .get(&item)
                    .filter(|it| it.asset_type == 24)
                    .map(|it| it.asset_id)
            })
            .unwrap_or(item);
        let index = self.worn_attachment(&item).or_else(|| self.objects.index_of_uuid(&item))?;
        let object = self.objects.get(index)?;
        let parent = self.objects.parent_of(object).and_then(|i| self.objects.get(i))?;
        if object.is_avatar() || object.attachment_point() == 0 || !parent.is_avatar() || parent.full_id != self.agent_id {
            return None;
        }
        Some(NetCommand::DetachObject {
            handle: object.key.region,
            local_id: object.key.local_id,
        })
    }

    /// Keep healthy inventory removals on the normal outfit path. Recover
    /// worn roots independently when a stale item (including another worn
    /// object's missing record) would otherwise block the whole COF plan.
    pub fn recover_attachment_removal(
        &self,
        action: appearance::Action,
        allow_recovery: bool,
    ) -> (Vec<NetCommand>, Option<appearance::Action>) {
        let (items, multiple) = match action {
            appearance::Action::Remove(item) => (vec![item], false),
            appearance::Action::RemoveItems(items) => (items, true),
            other => return (Vec::new(), Some(other)),
        };
        let incomplete = self
            .outfit_attachment_items()
            .iter()
            .any(|id| !self.inventory.items.contains_key(id))
            || appearance::cof(&self.inventory).is_none_or(|cof| !appearance::complete(&self.inventory, cof));
        let mut commands = Vec::new();
        let mut remaining = Vec::new();
        let mut seen = HashSet::new();
        for item in items {
            let temporary = self.rlva_temporary_attachment(item).is_some();
            if (temporary || allow_recovery && (incomplete || !self.inventory.items.contains_key(&item)))
                && let Some(command) = self.attachment_detach_command(item)
            {
                if let NetCommand::DetachObject { handle, local_id } = &command
                    && seen.insert((*handle, *local_id))
                {
                    commands.push(command);
                }
            } else {
                remaining.push(item);
            }
        }
        let action = if remaining.is_empty() {
            None
        } else if multiple {
            Some(appearance::Action::RemoveItems(remaining))
        } else {
            Some(appearance::Action::Remove(remaining[0]))
        };
        (commands, action)
    }

    /// Independent of the inventory/appearance floaters: a script can attach
    /// a newly created inventory item while both windows are closed.
    pub fn update_attachment_inventory(&mut self, now: Instant, allow_links: bool) -> Vec<NetCommand> {
        if self.teleporting || !self.movement_complete || !self.main().is_some_and(|r| r.caps.contains_key("FetchInventory2")) {
            return Vec::new();
        }
        let worn: HashSet<_> = self
            .outfit_attachment_points()
            .into_iter()
            .filter(|(_, point)| *point != 0)
            .map(|(item, _)| item)
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
            commands.push(NetCommand::FetchAttachmentItems { items: missing });
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
            [NetCommand::FetchAttachmentItems { items }] if items == &[item.id]));
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
                [NetCommand::FetchAttachmentItems { items }] if items == &[item.id]));
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

    fn add_temporary(world: &mut World, ordinary: &ObjectUpdate) -> ObjectUpdate {
        let mut temporary = ordinary.clone();
        temporary.local_id += 1000;
        temporary.full_id = Uuid::from_u128(90_100);
        temporary.name_values = format!("AttachItemID STRING RW SV {}", temporary.full_id);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![temporary.clone()],
        });
        temporary
    }

    #[test]
    fn temporary_attachments_do_not_block_add_remove_or_save_outfits() {
        let (mut world, object, item) = fixture();
        world.inventory.add_items(vec![item.clone()]);
        let temporary = add_temporary(&mut world, &object);
        assert_eq!(world.worn_attachment_items().len(), 2, "temporary objects still count as attached");
        assert!(world.worn_attachment_points().contains_key(&temporary.full_id));
        assert_eq!(world.temp_attachments().len(), 1);
        let worn = world.outfit_attachment_items();
        assert!(appearance::prepare(&mut world.inventory, world.agent_id, &worn).is_empty());
        let points = world.outfit_attachment_points();
        for action in [
            appearance::Action::Add(Uuid::from_u128(723)),
            appearance::Action::WearItem {
                item: Uuid::from_u128(723),
                point: 5,
                replace: false,
            },
            appearance::Action::Remove(item.id),
            appearance::Action::Remove(Uuid::from_u128(715)),
            appearance::Action::Save(Some("Temporary attachment regression".to_owned())),
        ] {
            assert!(
                appearance::plan(&world.inventory, action.clone(), &world.worn_attachment_points()).is_err(),
                "the raw temporary task ID reproduces #1663: {action:?}"
            );
            let (change, _) = appearance::plan(&world.inventory, action, &points).expect("temporary object must not block the outfit");
            assert!(change.links.iter().all(|link| link.target != temporary.full_id));
            assert!(
                change
                    .cof
                    .iter()
                    .flat_map(|(_, links)| links)
                    .all(|link| link.target != temporary.full_id)
            );
            let mut inventory = super::super::inventory::Inventory {
                root: world.inventory.root,
                owner: world.agent_id,
                items: world.inventory.items.clone(),
                folders: world.inventory.folders.clone(),
                ..Default::default()
            };
            appearance::demo_mutate(&mut inventory, world.agent_id, &change).expect("acknowledged outfit change");
            let sync = appearance::sync_plan(&inventory, world.agent_id, &worn);
            assert!(sync.unknown.is_empty());
            for command in sync.commands {
                if let NetCommand::DetachAttachments(items) = command {
                    assert!(!items.contains(&temporary.full_id), "outfit sync must leave temporary objects worn");
                }
            }
        }
    }

    #[test]
    fn temporary_attachments_do_not_hide_real_items_still_loading() {
        let (mut world, object, item) = fixture();
        let temporary = add_temporary(&mut world, &object);
        assert!(world.outfit_attachment_points().contains_key(&item.id));
        assert!(!world.outfit_attachment_points().contains_key(&temporary.full_id));
        assert!(
            appearance::plan(
                &world.inventory,
                appearance::Action::Add(Uuid::from_u128(723)),
                &world.outfit_attachment_points()
            )
            .is_err(),
            "an unresolved ordinary attachment must still protect the outfit"
        );
        let worn = world.outfit_attachment_items();
        assert!(matches!(&appearance::prepare(&mut world.inventory, world.agent_id, &worn)[..],
            [NetCommand::FetchItems { items, .. }] if items == &[item.id]));
        assert!(matches!(&world.update_attachment_inventory(Instant::now(), true)[..],
            [NetCommand::FetchAttachmentItems { items }] if items == &[item.id]));
    }

    #[test]
    fn temporary_attachments_preserve_healthy_removals_explicit_detach_and_rlva_locks() {
        let (mut world, object, item) = fixture();
        world.inventory.add_items(vec![item.clone()]);
        let temporary = add_temporary(&mut world, &object);
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(item.id), true);
        assert!(commands.is_empty(), "a temporary object is not a missing inventory record");
        assert!(matches!(remaining, Some(appearance::Action::Remove(id)) if id == item.id));
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(temporary.full_id), true);
        assert!(remaining.is_none());
        assert!(matches!(&commands[..], [NetCommand::DetachObject { local_id, .. }] if *local_id == temporary.local_id));
        world.rlva_configure(true);
        restriction(&mut world, temporary.full_id, "@detach=n");
        assert!(!world.rlva_can_remove(temporary.full_id, Uuid::nil()));
        assert!(!world.rlva_allows_command(&commands[0], Uuid::nil()));
        assert!(world.rlva_can_remove(item.id, Uuid::nil()));
    }

    #[test]
    fn temporary_attachment_marker_with_a_known_inventory_item_stays_usable_in_demo() {
        let (mut world, mut object, mut item) = fixture();
        item.id = object.full_id;
        world.inventory.add_items(vec![item.clone()]);
        object.name_values = format!("AttachItemID STRING RW SV {}", item.id);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object.clone()],
        });
        assert_eq!(world.outfit_attachment_items(), HashSet::from([item.id]));
        assert_eq!(world.outfit_attachment_points().get(&item.id), Some(&object.attachment_point()));
        assert!(world.temp_attachments().is_empty());
        assert!(
            appearance::plan(
                &world.inventory,
                appearance::Action::Add(Uuid::from_u128(723)),
                &world.outfit_attachment_points()
            )
            .is_ok()
        );
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

    #[test]
    fn attachment_recovery_detaches_an_old_object_after_inventory_fetches_are_exhausted() {
        let (mut world, object, item) = fixture();
        let now = Instant::now();
        for attempt in 0..u32::from(FETCH_ATTEMPTS) {
            world.update_attachment_inventory(now + RETRY_GAP * attempt, false);
            world.apply(NetEvent::InventoryItemsFailed { items: vec![item.id] });
        }
        assert!(
            appearance::plan(
                &world.inventory,
                appearance::Action::Remove(item.id),
                &world.worn_attachment_points()
            )
            .is_err()
        );
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(item.id), true);
        assert!(remaining.is_none());
        assert!(matches!(&commands[..], [NetCommand::DetachObject { handle: 1, local_id }] if *local_id == object.local_id));
        assert!(world.worn_attachment(&item.id).is_some(), "wait for simulator confirmation");
        assert!(world.take_inventory_edits().is_empty());
        world.note_outfit_command(&commands[0]);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![object.local_id],
        });
        assert!(world.worn_attachment(&item.id).is_none());
        assert!(!world.inventory.items.contains_key(&item.id), "never fabricate an inventory record");
    }

    #[test]
    fn attachment_recovery_removes_only_the_selected_known_root_despite_another_missing_item() {
        let (mut world, object, missing) = fixture();
        let selected = Uuid::from_u128(717);
        let mut other = object.clone();
        other.full_id = Uuid::from_u128(90_040);
        other.local_id += 1;
        other.name_values = format!("AttachItemID STRING RW SV {selected}");
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![other.clone()],
        });
        let cof = appearance::cof(&world.inventory).expect("COF");
        let before = appearance::folder_links(&world.inventory, cof);
        let expected: Vec<_> = before.iter().filter(|link| link.target != selected).cloned().collect();
        assert!(
            appearance::plan(
                &world.inventory,
                appearance::Action::Remove(selected),
                &world.worn_attachment_points()
            )
            .is_err()
        );
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(selected), true);
        assert!(remaining.is_none());
        assert!(matches!(&commands[..], [NetCommand::DetachObject { handle: 1, local_id }] if *local_id == other.local_id));
        assert_eq!(appearance::folder_links(&world.inventory, cof), before);
        world.note_outfit_command(&commands[0]);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![other.local_id],
        });
        assert_eq!(appearance::folder_links(&world.inventory, cof), expected);
        assert!(world.worn_attachment(&missing.id).is_some());
        assert!(world.inventory.items.contains_key(&selected));
        assert!(!world.take_inventory_edits().is_empty());
    }

    #[test]
    fn attachment_recovery_resolves_a_broken_link_deduplicates_roots_and_retains_clothing_actions() {
        let (mut world, object, item) = fixture();
        let link = InvItem {
            id: Uuid::from_u128(90_050),
            asset_id: item.id,
            asset_type: 24,
            ..item.clone()
        };
        let link_id = link.id;
        world.inventory.add_items(vec![link]);
        let clothing = Uuid::from_u128(715);
        let (commands, remaining) = world.recover_attachment_removal(
            appearance::Action::RemoveItems(vec![item.id, link_id, object.full_id, clothing]),
            true,
        );
        assert_eq!(commands.len(), 1);
        assert!(matches!(&commands[0], NetCommand::DetachObject { local_id, .. } if *local_id == object.local_id));
        assert!(matches!(remaining, Some(appearance::Action::RemoveItems(ids)) if ids == vec![clothing]));
    }

    #[test]
    fn attachment_recovery_preserves_healthy_outfit_removal_and_temporary_object_detachment() {
        let (mut world, mut object, item) = fixture();
        world.inventory.add_items(vec![item.clone()]);
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(item.id), true);
        assert!(commands.is_empty());
        assert!(matches!(remaining, Some(appearance::Action::Remove(id)) if id == item.id));
        object.name_values = format!("AttachItemID STRING RW SV {}", object.full_id);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object.clone()],
        });
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(object.full_id), true);
        assert!(remaining.is_none());
        assert!(matches!(&commands[..], [NetCommand::DetachObject { local_id, .. }] if *local_id == object.local_id));
    }

    #[test]
    fn attachment_recovery_never_addresses_a_foreign_attachment_child_or_world_object() {
        for kind in 0..3 {
            let (mut world, mut object, _) = fixture();
            match kind {
                0 => {
                    let mut avatar = crate::demo::action_avatar(false);
                    avatar.full_id = Uuid::from_u128(90_060);
                    avatar.local_id += 1;
                    object.parent_id = avatar.local_id;
                    world.apply(NetEvent::ObjectUpdates {
                        handle: 1,
                        objects: vec![avatar],
                    });
                }
                1 => {
                    object.parent_id = object.local_id;
                    object.local_id += 1;
                    object.full_id = Uuid::from_u128(90_061);
                }
                _ => object.parent_id = 0,
            }
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![object.clone()],
            });
            let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(object.full_id), true);
            assert!(commands.is_empty(), "kind {kind}");
            assert!(remaining.is_some());
        }
    }

    fn restriction(world: &mut World, source: Uuid, message: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Controller".to_owned(),
            source_id: source,
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: message.to_owned(),
        }));
    }

    #[test]
    fn attachment_recovery_honors_rlva_point_and_source_locks_with_known_or_missing_records() {
        for known in [false, true] {
            for (rule, allowed) in [("", true), ("@addattach=n", true), ("@remattach=n", false), ("@detach=n", false)] {
                let (mut world, object, item) = fixture();
                if known {
                    world.inventory.add_items(vec![item.clone()]);
                    let cof = appearance::cof(&world.inventory).expect("COF");
                    world.inventory.folders.get_mut(&cof).expect("COF").state = FetchState::Unknown;
                }
                world.rlva_configure(true);
                if !rule.is_empty() {
                    restriction(&mut world, object.full_id, rule);
                }
                let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(item.id), true);
                assert!(remaining.is_none());
                assert_eq!(
                    world.rlva_allows_command(&commands[0], Uuid::nil()),
                    allowed,
                    "known {known}, rule {rule}"
                );
                if allowed {
                    world.rlva_note_attachment_command(&commands[0], Uuid::nil(), Instant::now());
                    assert!(world.rlva.watchdog.expected_detach.contains_key(&item.id));
                    assert!(!world.rlva.watchdog.expected_detach.contains_key(&object.full_id));
                }
            }
        }
    }

    #[test]
    fn attachment_recovery_keeps_unknown_folder_and_scripted_nostrip_checks_conservative() {
        for rule in ["", "@sharedunwear=n", "@unsharedunwear=n", "@detachthis=n", "@detachallthis=n"] {
            let (mut world, object, item) = fixture();
            world.rlva_configure(true);
            if !rule.is_empty() {
                restriction(&mut world, object.full_id, rule);
            }
            let (commands, _) = world.recover_attachment_removal(appearance::Action::Remove(item.id), true);
            assert_eq!(world.rlva_allows_command(&commands[0], Uuid::nil()), rule.is_empty(), "{rule}");
            assert!(
                !world.rlva_allows_command(&commands[0], Uuid::from_u128(90_070)),
                "unknown scripted nostrip: {rule}"
            );
        }
    }

    #[test]
    fn attachment_recovery_waits_for_interactive_outfit_changes_and_preserves_temporary_detaches() {
        let (mut world, mut object, item) = fixture();
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(item.id), false);
        assert!(commands.is_empty());
        assert!(matches!(remaining, Some(appearance::Action::Remove(id)) if id == item.id));
        object.name_values = format!("AttachItemID STRING RW SV {}", object.full_id);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object.clone()],
        });
        let (commands, remaining) = world.recover_attachment_removal(appearance::Action::Remove(object.full_id), false);
        assert!(remaining.is_none());
        assert!(matches!(&commands[..], [NetCommand::DetachObject { local_id, .. }] if *local_id == object.local_id));
    }

    fn with_lost_folder(world: &mut World, item: &InvItem) -> Uuid {
        world.inventory.owner = world.agent_id;
        let id = Uuid::from_u128(90_080);
        let mut folder = world.inventory.folders[&item.parent].clone();
        folder.info.id = id;
        folder.info.type_default = 16;
        folder.info.name = "Lost And Found".to_owned();
        folder.items.clear();
        world.inventory.folders.insert(id, folder);
        id
    }

    #[test]
    fn a_fetched_parentless_attachment_becomes_visible_without_replacing_its_original() {
        let (mut world, object, mut item) = fixture();
        let objects = item.parent;
        let lost = with_lost_folder(&mut world, &item);
        item.parent = Uuid::nil();
        world.apply(NetEvent::AttachmentInventoryItems(vec![item.clone()]));
        assert_eq!(world.inventory.items[&item.id].parent, objects);
        assert!(world.inventory.folders[&objects].items.contains(&item.id));
        assert!(!world.inventory.folders[&lost].items.contains(&item.id));
        assert!(
            world.objects.index_of_uuid(&object.full_id).is_some(),
            "do not detach or replace the object to file it"
        );
        let change = world.inventory.take_parent_repairs().expect("reliable parent repair");
        assert!(change.move_only());
        assert!(
            matches!(&change.operations[..], [Operation::Patch { id, body, .. }] if *id == item.id && body["parent_id"].as_uuid() == objects)
        );
        let commands = world.update_attachment_inventory(Instant::now(), true);
        assert!(matches!(&commands[..], [NetCommand::EditInventory { change, .. }]
            if matches!(&change.operations[..], [Operation::Links { links, .. }] if links.len() == 1 && links[0].target == item.id)));
        assert!(
            !world.inventory.items.contains_key(&object.full_id),
            "the original inventory UUID stays unchanged"
        );
    }

    #[test]
    fn parent_repairs_use_normal_rlva_move_rules_without_unlocking_other_folders() {
        let (mut world, object, mut item) = fixture();
        let old = item.parent;
        let lost = with_lost_folder(&mut world, &item);
        item.parent = Uuid::nil();
        world.apply(NetEvent::AttachmentInventoryItems(vec![item.clone()]));
        assert_eq!(world.inventory.items[&item.id].parent, old);
        assert!(world.inventory.folders[&lost].items.is_empty());
        let locked = Uuid::from_u128(90_083);
        let mut shared = world.inventory.folders[&old].info.clone();
        shared.parent = world.inventory.root;
        shared.name = "#RLV".to_owned();
        shared.type_default = -1;
        world.inventory.server_updated(
            vec![
                shared,
                aurora_net::inventory::InvFolder {
                    id: locked,
                    parent: old,
                    name: "Locked".to_owned(),
                    type_default: -1,
                    ..Default::default()
                },
            ],
            Vec::new(),
        );
        world.rlva_configure(true);
        restriction(&mut world, object.full_id, "@attachthis:Locked=n");
        let change = world.inventory.take_parent_repairs().expect("repair");
        let command = NetCommand::EditInventory {
            request: Uuid::from_u128(90_081),
            change: change.clone(),
        };
        assert!(world.rlva_allows_command(&command, Uuid::nil()));
        assert!(
            world.rlva_allows_command(&command, object.full_id),
            "confirming the current parent is an ordinary allowed move"
        );
        let other = NetCommand::EditInventory {
            request: Uuid::from_u128(90_082),
            change: Mutation {
                operations: vec![Operation::Patch {
                    id: item.id,
                    folder: false,
                    body: aurora_llsd::llsd_map! { "parent_id" => locked },
                }],
                refresh: vec![locked],
                ..Default::default()
            },
        };
        assert!(!world.rlva_allows_command(&other, Uuid::nil()));
        assert!(!world.rlva_allows_command(&other, Uuid::from_u128(90_084)));
        let version = world.inventory.folders[&old].info.version;
        world.apply(NetEvent::InventoryContents(vec![FolderContents {
            folder_id: old,
            owner_id: world.agent_id,
            version: version + 1,
            folders: vec![world.inventory.folders[&locked].info.clone()],
            items: vec![world.inventory.items[&item.id].clone()],
        }]));
        assert!(
            world.rlva_allows_command(&command, Uuid::nil()),
            "parent confirmation retains normal move policy after readback"
        );
        assert!(!world.rlva_allows_command(&other, Uuid::nil()));
    }

    #[test]
    fn script_attachment_creation_and_readback_use_objects_in_both_arrival_orders() {
        for notification_first in [false, true] {
            let (mut world, _, mut item) = fixture();
            let objects = item.parent;
            let lost = with_lost_folder(&mut world, &item);
            item.parent = Uuid::nil();
            let notification = NetEvent::InventoryUpdated {
                folders: Vec::new(),
                items: vec![item.clone()],
            };
            let readback = NetEvent::AttachmentInventoryItems(vec![item.clone()]);
            for event in if notification_first {
                [notification, readback]
            } else {
                [readback, notification]
            } {
                world.apply(event);
                assert_eq!(world.inventory.items[&item.id].parent, objects);
                assert!(!world.inventory.folders[&lost].items.contains(&item.id));
            }
            let mut expected = item.clone();
            expected.parent = objects;
            assert_eq!(
                world.inventory.items[&item.id], expected,
                "preserve the original record and permissions"
            );
            assert_eq!(
                world.inventory.take_parent_repairs().expect("parent assignment").operations.len(),
                1
            );
            assert!(world.inventory.take_parent_repairs().is_none());
            // An older generic metadata read cannot reclassify a pending
            // creation as an AIS orphan while its reliable move is in flight.
            world.apply(NetEvent::InventoryItems(vec![item.clone()]));
            assert_eq!(world.inventory.items[&item.id].parent, objects);
            assert!(world.inventory.take_parent_repairs().is_none());
        }
    }

    #[test]
    fn late_attachment_metadata_keeps_its_creation_context_after_detach_or_avatar_loss() {
        for teleport in [false, true] {
            let (mut world, object, mut item) = fixture();
            let objects = item.parent;
            let lost = with_lost_folder(&mut world, &item);
            assert!(matches!(
                &world.update_attachment_inventory(Instant::now(), true)[..],
                [NetCommand::FetchAttachmentItems { .. }]
            ));
            if teleport {
                world.teleporting = true;
                let avatar = world.objects.index_of_uuid(&world.agent_id).expect("avatar");
                world.objects.remove_idx(avatar);
            } else {
                world.apply(NetEvent::ObjectsKilled {
                    handle: 1,
                    local_ids: vec![object.local_id],
                });
            }
            assert!(world.worn_attachment(&item.id).is_none());
            item.parent = Uuid::nil();
            world.apply(NetEvent::AttachmentInventoryItems(vec![item.clone()]));
            assert_eq!(world.inventory.items[&item.id].parent, objects);
            assert!(!world.inventory.folders[&lost].items.contains(&item.id));
            assert!(
                world.update_attachment_inventory(Instant::now(), true).is_empty(),
                "never rewear a late original"
            );
        }
    }

    #[test]
    fn orphan_discovery_is_account_scoped_and_preserves_live_attachment_creation() {
        let (mut world, object, mut attached) = fixture();
        let objects = attached.parent;
        let lost = with_lost_folder(&mut world, &attached);
        let root = Uuid::from_u128(90_090);
        world.inventory.root = root;
        let mut folder = world.inventory.folders[&objects].clone();
        folder.info.id = root;
        folder.info.parent = Uuid::nil();
        folder.info.type_default = 8;
        world.inventory.folders.insert(root, folder);
        let commands = world.pump_inventory_background();
        let request = commands
            .iter()
            .find_map(|cmd| match cmd {
                NetCommand::FetchInventoryOrphans { request } => Some(*request),
                _ => None,
            })
            .expect("startup orphan scan");
        attached.parent = Uuid::nil();
        let mut orphan = attached.clone();
        orphan.id = Uuid::from_u128(90_095);
        let contents = aurora_net::inventory::orphans::Contents {
            folders: Vec::new(),
            items: vec![attached.clone(), orphan.clone()],
        };
        world.apply(NetEvent::InventoryOrphansFetched {
            request,
            owner: Uuid::from_u128(90_099),
            result: Ok(contents.clone()),
        });
        assert!(
            !world.inventory.items.contains_key(&orphan.id),
            "a previous account's response cannot apply"
        );
        world.apply(NetEvent::InventoryOrphansFetched {
            request,
            owner: world.agent_id,
            result: Ok(contents.clone()),
        });
        assert_eq!(world.inventory.items[&attached.id].parent, objects);
        assert_eq!(world.inventory.items[&orphan.id].parent, lost);
        assert!(world.objects.index_of_uuid(&object.full_id).is_some());
        assert_eq!(world.worn_attachment_items().len(), 1, "recovery never wears the lost original");
        let generation = world.inventory.generation;
        world.apply(NetEvent::InventoryOrphansFetched {
            request,
            owner: world.agent_id,
            result: Ok(contents),
        });
        assert_eq!(world.inventory.generation, generation, "duplicate results are obsolete");
    }
}
