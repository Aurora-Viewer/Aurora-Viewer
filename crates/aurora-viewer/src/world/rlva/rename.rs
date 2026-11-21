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
    /// The item moved into a new folder `name` made under `parent`.
    Folder {
        item: Uuid,
        parent: Uuid,
        name: String,
    },
    Name {
        item: Uuid,
        name: String,
    },
    /// The item's own "New Folder" renamed.
    Renamed {
        folder: Uuid,
        name: String,
    },
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

impl Target {
    fn confirmed(&self, inv: &Inventory) -> bool {
        match self {
            Self::Folder { item, parent, name } => inv
                .items
                .get(item)
                .and_then(|item| inv.folders.get(&item.parent))
                .is_some_and(|folder| folder.info.parent == *parent && folder.info.name.eq_ignore_ascii_case(name)),
            Self::Name { item, name } => inv.items.get(item).is_some_and(|item| item.name == *name),
            Self::Renamed { folder, name } => inv.folders.get(folder).is_some_and(|folder| folder.info.name == *name),
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
                self.candidates
                    .retain(|candidate| candidate.item != pending.item || candidate.point != pending.point);
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
            let (target, operations, refresh) = if actions::held(inv, item) & actions::MODIFY != 0 {
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
                    vec![Operation::Patch {
                        id: item.id,
                        folder: false,
                        body: llsd_map! { "name" => name },
                    }],
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
                // A parent called "New Folder", not directly under #RLV, that
                // holds exactly one object is renamed; otherwise a new folder
                // is always made and the item moved into it
                // (RlvRenameOnWearObserver::onCategoryCreate). The Legacy
                // Viewer compares with LLViewerFolderType's untranslated
                // "New Folder".
                let objects = parent
                    .items
                    .iter()
                    .filter_map(|id| inv.items.get(id))
                    .filter(|it| it.asset_type == 6)
                    .count();
                if parent.info.name == "New Folder" && parent.info.parent != root && objects == 1 {
                    (
                        Target::Renamed {
                            folder: parent.info.id,
                            name: name.clone(),
                        },
                        vec![Operation::Patch {
                            id: parent.info.id,
                            folder: true,
                            body: llsd_map! { "name" => name },
                        }],
                        vec![parent.info.parent, parent.info.id],
                    )
                } else {
                    let folder = Uuid::new_v4();
                    (
                        Target::Folder {
                            item: item.id,
                            parent: item.parent,
                            name: name.clone(),
                        },
                        vec![
                            Operation::Folder {
                                id: folder,
                                parent: item.parent,
                                name,
                                type_default: -1,
                            },
                            Operation::Patch {
                                id: item.id,
                                folder: false,
                                body: llsd_map! { "parent_id" => folder },
                            },
                        ],
                        vec![item.parent, folder],
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
                    operations,
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

    fn context<'a>(worn: &'a HashMap<Uuid, u8>, root: Uuid, names: &'a HashMap<u8, String>, now: Instant) -> WearContext<'a> {
        WearContext {
            worn,
            root,
            names,
            legacy: true,
            now,
        }
    }

    fn add_folder(inv: &mut Inventory, id: Uuid, parent: Uuid, name: &str) {
        inv.folders.insert(
            id,
            Folder {
                info: InvFolder {
                    id,
                    parent,
                    name: name.into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: vec![],
                items: vec![],
                state: FetchState::Fetched,
                library: false,
            },
        );
        inv.folders.get_mut(&parent).unwrap().children.push(id);
    }

    /// The first update after a new arrival of `item` on point 1.
    fn arrival(inv: &mut Inventory, root: Uuid, item: Uuid, names: &HashMap<u8, String>, now: Instant) -> (RenameOnWear, Uuid, Mutation) {
        let mut planner = RenameOnWear::default();
        let mut worn = HashMap::new();
        planner.update(inv, context(&worn, root, names, now));
        worn.insert(item, 1);
        let Some(NetCommand::EditInventory { request, change }) = planner.update(inv, context(&worn, root, names, now)) else {
            panic!("a naming request")
        };
        (planner, request, change)
    }

    /// RlvRenameOnWearObserver::doneIdle and onCategoryCreate
    /// (rlvinventory.cpp): a no-modify item goes into a new ".(point)"
    /// folder made in its own folder, in one request.
    #[test]
    fn rlva_no_modify_rename_moves_the_item_into_a_new_folded_folder() {
        let (mut inv, root, item, names) = fixture(false);
        let now = Instant::now();
        let (mut planner, request, change) = arrival(&mut inv, root, item, &names, now);
        let [
            Operation::Folder { id, parent, name, .. },
            Operation::Patch {
                id: moved,
                folder: false,
                body,
            },
        ] = &change.operations[..]
        else {
            panic!("a folder and a move: {:?}", change.operations)
        };
        assert_eq!(name, ".(chest)");
        assert_eq!(*moved, item);
        assert_eq!(body["parent_id"].as_uuid(), *id, "the move goes to the folder the request makes");
        let (folder, parent) = (*id, *parent);
        let worn = HashMap::from([(item, 1)]);
        assert!(planner.update(&mut inv, context(&worn, root, &names, now)).is_none());
        add_folder(&mut inv, folder, parent, ".(chest)");
        assert!(
            planner.update(&mut inv, context(&worn, root, &names, now)).is_none(),
            "the move is not seen yet"
        );
        assert!(planner.owns(request));
        inv.items.get_mut(&item).unwrap().parent = folder;
        assert!(planner.update(&mut inv, context(&worn, root, &names, now)).is_none());
        assert!(!planner.owns(request));
    }

    /// RlvRenameOnWearObserver::doneIdle always makes a new folder: a
    /// ".(point)" folder already there is not reused.
    #[test]
    fn rlva_no_modify_rename_does_not_reuse_an_existing_folded_folder() {
        let (mut inv, root, item, names) = fixture(false);
        let existing = Uuid::from_u128(5);
        add_folder(&mut inv, existing, Uuid::from_u128(3), ".(chest)");
        let (_, _, change) = arrival(&mut inv, root, item, &names, Instant::now());
        let [Operation::Folder { id, name, .. }, Operation::Patch { body, .. }] = &change.operations[..] else {
            panic!("a new folder and a move: {:?}", change.operations)
        };
        assert_eq!(name, ".(chest)");
        assert_ne!(*id, existing);
        assert_eq!(body["parent_id"].as_uuid(), *id);
    }

    /// RlvRenameOnWearObserver::doneIdle renames the item's parent when it
    /// is called "New Folder", is not directly under #RLV and holds exactly
    /// one object.
    #[test]
    fn rlva_no_modify_rename_renames_a_new_folder_holding_one_object() {
        let (mut inv, root, item, names) = fixture(false);
        let outfit = Uuid::from_u128(3);
        let fresh = Uuid::from_u128(6);
        add_folder(&mut inv, fresh, outfit, "New Folder");
        inv.folders.get_mut(&outfit).unwrap().items.clear();
        inv.folders.get_mut(&fresh).unwrap().items.push(item);
        inv.items.get_mut(&item).unwrap().parent = fresh;
        let now = Instant::now();
        let (mut planner, request, change) = arrival(&mut inv, root, item, &names, now);
        assert!(
            matches!(&change.operations[..], [Operation::Patch { id, folder: true, body }] if *id == fresh && body["name"].as_str() == ".(chest)"),
            "{:?}",
            change.operations
        );
        inv.folders.get_mut(&fresh).unwrap().info.name = ".(chest)".into();
        let worn = HashMap::from([(item, 1)]);
        assert!(planner.update(&mut inv, context(&worn, root, &names, now)).is_none());
        assert!(!planner.owns(request));

        // A second object in it, or the folder directly under #RLV: a new folder.
        for (second, parent) in [(true, outfit), (false, root)] {
            let (mut inv, root, item, names) = fixture(false);
            add_folder(&mut inv, fresh, parent, "New Folder");
            inv.folders.get_mut(&outfit).unwrap().items.clear();
            inv.folders.get_mut(&fresh).unwrap().items.push(item);
            inv.items.get_mut(&item).unwrap().parent = fresh;
            if second {
                let other = Uuid::from_u128(7);
                inv.items.insert(
                    other,
                    InvItem {
                        id: other,
                        parent: fresh,
                        name: "Ring".into(),
                        asset_type: 6,
                        ..Default::default()
                    },
                );
                inv.folders.get_mut(&fresh).unwrap().items.push(other);
            }
            let (_, _, change) = arrival(&mut inv, root, item, &names, now);
            assert!(
                matches!(&change.operations[..], [Operation::Folder { parent, .. }, Operation::Patch { .. }] if *parent == fresh),
                "second object {second}: {:?}",
                change.operations
            );
        }
    }
}
