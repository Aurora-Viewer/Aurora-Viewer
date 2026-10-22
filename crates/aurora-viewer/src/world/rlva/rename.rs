//! RlvRenameOnWearObserver (rlvinventory.cpp, originally LGPL 2.1).
//! Observe actual attachment arrivals, rather than repeatedly undoing a user's
//! later manual rename. Inventory work stays asynchronous and bounded.
use super::{collection, naming};
use crate::world::inventory::{Inventory, actions};
use aurora_llsd::llsd_map;
use aurora_net::{
    NetCommand,
    inventory::operations::{Mutation, Operation},
};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Default)]
pub struct RenameOnWear {
    known: Option<HashMap<Uuid, u8>>,
    candidates: VecDeque<Candidate>,
    pending: Option<Pending>,
}

struct Candidate {
    item: Uuid,
    point: u8,
    since: Instant,
    attempts: u8,
    retry: Instant,
}

enum Target {
    Folder { parent: Uuid, name: String },
    Name { item: Uuid, name: String },
    Parent { item: Uuid, parent: Uuid },
}

struct Pending {
    request: Uuid,
    item: Uuid,
    point: u8,
    since: Instant,
    target: Target,
}

pub struct WearContext<'a> {
    pub worn: &'a HashMap<Uuid, u8>,
    pub root: Uuid,
    pub names: &'a HashMap<u8, String>,
    pub legacy: bool,
    pub now: Instant,
}

fn child(inv: &Inventory, parent: Uuid, name: &str) -> Option<Uuid> {
    inv.folders
        .get(&parent)?
        .children
        .iter()
        .filter_map(|id| inv.folders.get(id))
        .find(|folder| !folder.library && folder.info.name.eq_ignore_ascii_case(name))
        .map(|folder| folder.info.id)
}

impl Target {
    fn confirmed(&self, inv: &Inventory) -> bool {
        match self {
            Self::Folder { parent, name } => child(inv, *parent, name).is_some(),
            Self::Name { item, name } => inv.items.get(item).is_some_and(|item| item.name == *name),
            Self::Parent { item, parent } => inv.items.get(item).is_some_and(|item| item.parent == *parent),
        }
    }
}

impl RenameOnWear {
    pub fn owns(&self, request: Uuid) -> bool {
        self.pending.as_ref().is_some_and(|pending| pending.request == request)
    }

    pub fn failed(&mut self, request: Uuid, now: Instant) {
        if !self.owns(request) {
            return;
        }
        if let Some(pending) = self.pending.take()
            && let Some(candidate) = self.candidates.iter_mut().find(|candidate| candidate.item == pending.item)
        {
            candidate.attempts += 1;
            candidate.retry = now + Duration::from_secs(30);
        }
    }

    pub fn update(&mut self, inv: &mut Inventory, context: WearContext<'_>) -> Option<NetCommand> {
        let WearContext {
            worn,
            root,
            names,
            legacy,
            now,
        } = context;
        let Some(known) = self.known.as_mut() else {
            self.known = Some(worn.clone());
            return None;
        };
        self.candidates.retain(|candidate| {
            worn.contains_key(&candidate.item)
                && candidate.attempts < 3
                && now.saturating_duration_since(candidate.since) < Duration::from_secs(120)
        });
        for (item, point) in worn {
            if known.get(item) == Some(point) {
                continue;
            }
            if let Some(candidate) = self.candidates.iter_mut().find(|candidate| candidate.item == *item) {
                candidate.point = *point;
            } else if self.candidates.len() < 128 {
                self.candidates.push_back(Candidate {
                    item: *item,
                    point: *point,
                    since: now,
                    attempts: 0,
                    retry: now,
                });
            }
        }
        known.clone_from(worn);
        if let Some(pending) = self.pending.as_ref() {
            if pending.target.confirmed(inv) {
                if !matches!(pending.target, Target::Folder { .. }) {
                    self.candidates
                        .retain(|candidate| candidate.item != pending.item || candidate.point != pending.point);
                }
                self.pending = None;
            } else if now.saturating_duration_since(pending.since) < Duration::from_secs(30) {
                return None;
            } else {
                self.failed(pending.request, now);
            }
        }
        while let Some(candidate) = self.candidates.front() {
            if now < candidate.retry {
                return None;
            }
            let item = inv.items.get(&candidate.item)?;
            if item.asset_type != 6 || !actions::under(inv, item.id, root) || collection::attachment_point(inv, item.id, legacy).is_some() {
                self.candidates.pop_front();
                continue;
            }
            let Some(name) = names.get(&candidate.point).map(|name| name.to_ascii_lowercase()) else {
                self.candidates.pop_front();
                continue;
            };
            let (target, operation, refresh) = if item.owner_mask & actions::MODIFY != 0 {
                let suffix = format!(" ({name})");
                let mut end = item.name.len().min(63usize.saturating_sub(suffix.len()));
                while !item.name.is_char_boundary(end) {
                    end -= 1;
                }
                let name = format!("{}{suffix}", &item.name[..end]);
                (
                    Target::Name {
                        item: item.id,
                        name: name.clone(),
                    },
                    Operation::Patch {
                        id: item.id,
                        folder: false,
                        body: llsd_map! { "name" => name },
                    },
                    vec![item.parent],
                )
            } else {
                let parent = inv.folders.get(&item.parent)?;
                if parent.info.id == root || naming::folded(&parent.info.name, legacy) {
                    self.candidates.pop_front();
                    continue;
                }
                if !crate::world::appearance::loaded(inv, parent.info.id) {
                    inv.request(parent.info.id);
                    return None;
                }
                let name = format!(".({name})");
                if let Some(parent) = child(inv, item.parent, &name) {
                    (
                        Target::Parent { item: item.id, parent },
                        Operation::Patch {
                            id: item.id,
                            folder: false,
                            body: llsd_map! { "parent_id" => parent },
                        },
                        vec![item.parent, parent],
                    )
                } else {
                    (
                        Target::Folder {
                            parent: item.parent,
                            name: name.clone(),
                        },
                        Operation::Folder {
                            id: Uuid::new_v4(),
                            parent: item.parent,
                            name,
                            type_default: -1,
                        },
                        vec![item.parent],
                    )
                }
            };
            let request = Uuid::new_v4();
            self.pending = Some(Pending {
                request,
                item: item.id,
                point: candidate.point,
                since: now,
                target,
            });
            return Some(NetCommand::EditInventory {
                request,
                change: Mutation {
                    operations: vec![operation],
                    refresh,
                    defer_folder_contents: true,
                    ..Default::default()
                },
            });
        }
        None
    }
}

impl crate::world::World {
    pub(crate) fn rlva_demo_naming_seed(&mut self) {
        use aurora_net::inventory::{InvFolder, InvItem};
        let root = Uuid::from_u128(0x726c766172656e01);
        let parent = Uuid::from_u128(0x726c766172656e02);
        self.inventory.created_folders(vec![
            InvFolder {
                id: root,
                parent: self.inventory.root,
                name: "#RLV".into(),
                type_default: -1,
                ..Default::default()
            },
            InvFolder {
                id: parent,
                parent: root,
                name: "Accessories".into(),
                type_default: -1,
                ..Default::default()
            },
        ]);
        self.inventory.add_items(
            [true, false]
                .into_iter()
                .map(|modify| InvItem {
                    id: Uuid::from_u128(if modify { 0x726c766172656e03 } else { 0x726c766172656e04 }),
                    parent,
                    owner: self.agent_id,
                    name: if modify { "Modifiable bracelet" } else { "No-modify bracelet" }.into(),
                    asset_type: 6,
                    inv_type: 6,
                    owner_mask: if modify { actions::MODIFY } else { 0 },
                    ..Default::default()
                })
                .collect(),
        );
    }

    pub(crate) fn rlva_demo_naming_attach(&mut self) {
        let Some(handle) = self.main_region else {
            return;
        };
        for (id, point, local_id) in [(0x726c766172656e03, 1, 161001), (0x726c766172656e04, 2, 161002)] {
            let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
                item_id: Uuid::from_u128(id),
                owner_id: self.agent_id,
                point,
                add: true,
                flags: 0,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: 0,
                name: String::new(),
                desc: String::new(),
            });
            object.local_id = local_id;
            self.apply(aurora_net::NetEvent::ObjectUpdates {
                handle,
                objects: vec![object],
            });
        }
    }

    pub(crate) fn rlva_demo_naming_report(&self) {
        let name = self
            .inventory
            .items
            .get(&Uuid::from_u128(0x726c766172656e03))
            .map_or("missing", |item| item.name.as_str());
        let folder = self
            .inventory
            .items
            .get(&Uuid::from_u128(0x726c766172656e04))
            .and_then(|item| self.inventory.folders.get(&item.parent))
            .map_or("missing", |folder| folder.info.name.as_str());
        log::info!("RLVa naming demo: modified={name}, no_modify_folder={folder}");
    }

    pub(crate) fn rlva_rename_tick(&mut self, now: Instant) -> Option<NetCommand> {
        if !self.rlva.enabled() || self.rlva.preferences.shared_wear || !self.rlva.preferences.auto_rename || !self.movement_complete {
            self.rlva.rename = Default::default();
            return None;
        }
        let root = super::inventory::shared_root(&self.inventory)?;
        let worn = self.rlva_worn_attachment_points();
        for item in worn
            .keys()
            .filter(|item| !self.inventory.items.contains_key(item))
            .copied()
            .collect::<Vec<_>>()
        {
            if self.rlva_inventory_fetch_due(item) {
                self.rlva.out.push(NetCommand::FetchItems {
                    items: vec![item],
                    owner: self.agent_id,
                });
            }
        }
        let mut planner = std::mem::take(&mut self.rlva.rename);
        let command = planner.update(
            &mut self.inventory,
            WearContext {
                worn: &worn,
                root,
                names: &self.avatar_lib.attach_names,
                legacy: self.rlva.preferences.legacy_naming,
                now,
            },
        );
        self.rlva.rename = planner;
        if let Some(command @ NetCommand::EditInventory { request, .. }) = command.as_ref()
            && !self.rlva_allows_command(command, Uuid::nil())
        {
            self.rlva.rename.failed(*request, now);
            return None;
        }
        command
    }

    pub(crate) fn rlva_rename_event(&mut self, event: &aurora_net::NetEvent, now: Instant) -> bool {
        let aurora_net::NetEvent::InventoryEdited { request, result } = event else {
            return false;
        };
        if !self.rlva.rename.owns(*request) {
            return false;
        }
        match result {
            Ok(reply) => {
                self.inventory.apply(reply.contents.clone());
                self.inventory.created_folders(reply.folders.clone());
            }
            Err(_) => {
                self.rlva.rename.failed(*request, now);
                log::warn!("RLVa automatic attachment naming failed");
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn fixture(modify: bool) -> (Inventory, Uuid, Uuid, HashMap<u8, String>) {
        let mut inv = Inventory {
            root: Uuid::from_u128(1),
            ..Default::default()
        };
        let shared = Uuid::from_u128(2);
        let parent = Uuid::from_u128(3);
        let item = Uuid::from_u128(4);
        for (id, container, name, children, items) in [
            (inv.root, Uuid::nil(), "Inventory", vec![shared], vec![]),
            (shared, inv.root, "#RLV", vec![parent], vec![]),
            (parent, shared, "Outfit", vec![], vec![item]),
        ] {
            inv.folders.insert(
                id,
                Folder {
                    info: InvFolder {
                        id,
                        parent: container,
                        name: name.into(),
                        type_default: -1,
                        ..Default::default()
                    },
                    children,
                    items,
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        inv.items.insert(
            item,
            InvItem {
                id: item,
                parent,
                name: "Bracelet".into(),
                asset_type: 6,
                owner_mask: if modify { actions::MODIFY } else { 0 },
                ..Default::default()
            },
        );
        (inv, shared, item, HashMap::from([(1, "Chest".into())]))
    }

    #[test]
    fn rlva_rename_on_wear_waits_for_new_arrivals_and_preserves_later_manual_names() {
        let (mut inv, root, item, names) = fixture(true);
        let now = Instant::now();
        let mut planner = RenameOnWear::default();
        let mut worn = HashMap::new();
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        worn.insert(item, 1);
        let Some(NetCommand::EditInventory { request, change }) = planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        ) else {
            panic!("rename")
        };
        assert!(planner.owns(request));
        assert!(
            matches!(&change.operations[..], [Operation::Patch { id, folder: false, body }] if *id == item && body["name"].as_str() == "Bracelet (chest)")
        );
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        inv.items.get_mut(&item).unwrap().name = "Bracelet (chest)".into();
        planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        );
        inv.items.get_mut(&item).unwrap().name = "My manual name".into();
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        assert!(!planner.owns(request));
    }

    #[test]
    fn rlva_rename_on_wear_bounds_failed_requests_and_does_not_rename_the_login_baseline() {
        let (mut inv, root, item, names) = fixture(true);
        let mut now = Instant::now();
        let mut planner = RenameOnWear::default();
        let mut worn = HashMap::from([(item, 1)]);
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        worn.clear();
        planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        );
        worn.insert(item, 1);
        for _ in 0..3 {
            let Some(NetCommand::EditInventory { request, .. }) = planner.update(
                &mut inv,
                WearContext {
                    worn: &worn,
                    root,
                    names: &names,
                    legacy: true,
                    now,
                },
            ) else {
                panic!("rename")
            };
            planner.failed(request, now);
            assert!(
                planner
                    .update(
                        &mut inv,
                        WearContext {
                            worn: &worn,
                            root,
                            names: &names,
                            legacy: true,
                            now
                        }
                    )
                    .is_none()
            );
            now += Duration::from_secs(31);
        }
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        worn.clear();
        planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        );
        worn.insert(item, 1);
        let Some(NetCommand::EditInventory { request, .. }) = planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        ) else {
            panic!("reattachment starts a new naming attempt")
        };
        let mut rlva = super::super::Rlva::default();
        rlva.configure(true);
        rlva.rename = planner;
        assert!(rlva.rename.owns(request));
        rlva.configure(false);
        assert!(!rlva.rename.owns(request), "global disable cancels a naming request");
    }

    #[test]
    fn rlva_no_modify_rename_creates_a_folded_folder_then_waits_for_the_real_move() {
        let (mut inv, root, item, names) = fixture(false);
        let now = Instant::now();
        let mut planner = RenameOnWear::default();
        let mut worn = HashMap::new();
        planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        );
        worn.insert(item, 1);
        let Some(NetCommand::EditInventory { change, .. }) = planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        ) else {
            panic!("folder")
        };
        let [Operation::Folder { id, parent, name, .. }] = &change.operations[..] else {
            panic!("folder")
        };
        let folder = *id;
        let parent = *parent;
        assert_eq!(name, ".(chest)");
        inv.folders.insert(
            folder,
            Folder {
                info: InvFolder {
                    id: folder,
                    parent,
                    name: name.clone(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&parent).unwrap().children.push(folder);
        let Some(NetCommand::EditInventory { request, change }) = planner.update(
            &mut inv,
            WearContext {
                worn: &worn,
                root,
                names: &names,
                legacy: true,
                now,
            },
        ) else {
            panic!("move")
        };
        assert!(change.move_only());
        assert!(planner.owns(request));
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        inv.items.get_mut(&item).unwrap().parent = folder;
        assert!(
            planner
                .update(
                    &mut inv,
                    WearContext {
                        worn: &worn,
                        root,
                        names: &names,
                        legacy: true,
                        now
                    }
                )
                .is_none()
        );
        assert!(!planner.owns(request));
    }
}
