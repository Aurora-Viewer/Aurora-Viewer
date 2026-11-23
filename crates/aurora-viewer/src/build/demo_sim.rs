//! Demo mode: what a simulator answers to the build tools, so that every
//! tab of the build floater can be tried offline (properties of selected
//! objects, contents, land impact, weights, physics data, parcel).

use crate::world::World;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use aurora_net::build::{BuildCmd, ObjectProps, PhysicsParams, TaskItem, perm, perm_field};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Creator of the demo objects (a demo resident).
const DEMO_CREATOR: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
const PERM_ALL: u32 = 0x7fff_ffff;

#[derive(Default)]
pub struct DemoSim {
    props: HashMap<Uuid, ObjectProps>,
    contents: HashMap<Uuid, (i16, Vec<TaskItem>)>,
    running: HashMap<(Uuid, Uuid), (bool, aurora_net::scripts::CompileTarget)>,
    pending_running: HashMap<(Uuid, Uuid), bool>,
    physics: HashMap<Uuid, PhysicsParams>,
    /// The reports of transform edits held for a round trip
    /// (`AURORA_DEMO_BUILD_ECHO`), with when they arrive.
    late_echoes: VecDeque<(Instant, NetEvent)>,
    /// The simulator's undo history of the edited objects (Ctrl+Z /
    /// Ctrl+Y): their placements before each edit, and those undone.
    history: HashMap<Uuid, History>,
    /// The inventory folder each object rezzed from inventory came from,
    /// sent back in its ObjectProperties (FolderID) as a simulator does.
    rezzed_from: HashMap<Uuid, Uuid>,
    /// The inventory item each object rezzed from inventory came from,
    /// sent back in its ObjectProperties (ItemID) as a simulator does.
    rezzed_item: HashMap<Uuid, Uuid>,
    /// The object each object rezzed from an object's contents came from,
    /// sent back in its ObjectProperties (FromTaskID).
    rezzed_from_task: HashMap<Uuid, Uuid>,
    /// Groups supplied by creation requests, including an explicitly empty group.
    rezzed_groups: HashMap<Uuid, Uuid>,
    /// What an object item holds (`AURORA_DEMO_DEEP_PERMS`), by item id: a
    /// rez from contents gets it, a save back keeps the copy's contents.
    stored: HashMap<Uuid, Vec<TaskItem>>,
    /// What the child prims of a stored object item hold, in link order.
    stored_links: HashMap<Uuid, Vec<Vec<TaskItem>>>,
    /// The item each object rezzed from a stored item came from.
    stored_copies: HashMap<Uuid, Uuid>,
    /// The child prims of such a copy.
    copy_children: HashMap<Uuid, Vec<Uuid>>,
    /// The parts rezzed together from a coalesced item, by part.
    copy_parts: HashMap<Uuid, Vec<Uuid>>,
    /// What each part of a stored coalesced item holds (several objects
    /// saved as one), in order.
    stored_group: HashMap<Uuid, Vec<Vec<TaskItem>>>,
    /// `AURORA_DEMO_DEEP_PERMS=run-replace` / `run-refuse`, or set by a test.
    pub(crate) deep_in_place: bool,
    pub(crate) deep_refuse: bool,
    /// A coalesced item saved back as a single object (its first part).
    pub(crate) deep_split: bool,
}

/// An object's position, rotation and scale.
type Placement = (glam::Vec3, glam::Quat, glam::Vec3);

/// Updates of one object closer than this are one edit (a drag sends one
/// a frame), as a simulator keeps one undo step for a drag.
const EDIT_GAP: Duration = Duration::from_millis(400);

#[derive(Default)]
struct History {
    undo: Vec<Placement>,
    redo: Vec<Placement>,
    last: Option<Instant>,
}

/// `AURORA_DEMO_BUILD_ECHO=<ms>`: the demo answers a transform edit that
/// long after it was sent, like a simulator a round trip away (issue #1193).
fn echo_delay() -> Option<Duration> {
    static DELAY: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();
    *DELAY.get_or_init(|| {
        std::env::var("AURORA_DEMO_BUILD_ECHO")
            .ok()
            .and_then(|ms| ms.trim().parse::<u64>().ok())
            .filter(|&ms| ms > 0)
            .map(Duration::from_millis)
    })
}

fn demo_item(object: Uuid, n: u128, name: &str, asset_type: i8, inv_type: i8) -> TaskItem {
    TaskItem {
        item_id: Uuid::from_u128(object.as_u128() ^ (0x17E0 << 64) ^ n),
        parent_id: object,
        creator_id: DEMO_CREATOR,
        owner_id: crate::demo::DEMO_AGENT,
        last_owner_id: DEMO_CREATOR,
        base_mask: PERM_ALL,
        owner_mask: PERM_ALL
            & if object == super::replace::demo::id(super::replace::demo::TARGET) {
                u32::MAX
            } else {
                match n {
                    2 => !perm::MODIFY,
                    3 => !perm::COPY,
                    _ => u32::MAX,
                }
            },
        next_owner_mask: perm::MOVE | perm::TRANSFER,
        asset_id: Uuid::from_u128(n),
        asset_type,
        inv_type,
        sale_price: 10,
        name: name.into(),
        description: String::new(),
        creation_date: 1_700_000_000,
        ..Default::default()
    }
}

/// The demo region makes new contents items like a Second Life region
/// (CreateTaskInventoryItem); `AURORA_DEMO_CONTENTS_CAP=0` withdraws the
/// capability, for the RezScript fallback and a disabled "New notecard".
pub fn caps() -> Vec<(String, String)> {
    if std::env::var("AURORA_DEMO_CONTENTS_CAP").is_ok_and(|v| v.trim() == "0") {
        return Vec::new();
    }
    vec![(
        super::contents::CREATE_TASK_ITEM_CAP.to_owned(),
        "demo://create-task-item".to_owned(),
    )]
}

fn script_copy_demo() -> bool {
    std::env::var("AURORA_DEMO_REPLACE_PRIM").is_ok_and(|mode| mode == "script")
}

impl DemoSim {
    pub fn save_script(
        &mut self,
        target: aurora_net::scripts::ScriptTarget,
        asset: Uuid,
        running: bool,
        compile: aurora_net::scripts::CompileTarget,
    ) {
        if let Some(entry) = self.contents.get_mut(&target.object)
            && let Some(item) = entry.1.iter_mut().find(|i| i.item_id == target.item)
        {
            item.asset_id = asset;
            entry.0 = entry.0.wrapping_add(1);
        }
        self.running.insert((target.object, target.item), (running, compile));
    }
    fn props_of(&mut self, world: &World, id: Uuid) -> &mut ObjectProps {
        let folder_id = self.rezzed_from.get(&id).copied().unwrap_or_default();
        let item_id = self.rezzed_item.get(&id).copied().unwrap_or_default();
        let from_task_id = self.rezzed_from_task.get(&id).copied().unwrap_or_default();
        let group = self.rezzed_groups.get(&id).copied();
        self.props.entry(id).or_insert_with(|| {
            let name = world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| world.objects.get(i))
                .map(|o| {
                    if o.text.is_empty() {
                        aurora_i18n::tr!("common-object-2", value0 = format!("{}", o.key.local_id))
                    } else {
                        o.text.lines().next().unwrap_or("").to_owned()
                    }
                })
                .unwrap_or_else(|| aurora_i18n::tr!("common-object").into());
            let local_id = world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| world.objects.get(i))
                .map(|o| o.key.local_id);
            let (touch_name, sit_name) = local_id.and_then(crate::demo::object_menu::texts).unwrap_or_default();
            // an attachment belongs to its wearer, as on a grid
            let owner_id = world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| world.objects.get(i))
                .and_then(|o| {
                    Some(o.owner_id).filter(|owner| !owner.is_nil()).or_else(|| {
                        let parent = world.objects.get(world.objects.parent_of(o)?)?;
                        parent.is_avatar().then_some(parent.full_id)
                    })
                })
                .unwrap_or(crate::demo::DEMO_AGENT);
            // the lamp of AURORA_DEMO_OBJECT_EXPORT: mostly our own work
            let creator_id = local_id.and_then(crate::demo::object_export::creator).unwrap_or(DEMO_CREATOR);
            // Violet Wolf's outfit of AURORA_DEMO_INSPECT: named and credited
            let outfit = local_id.and_then(crate::demo::inspect::props);
            let creator_id = outfit.map_or(creator_id, |o| o.2);
            let name = outfit.map_or(name, |o| o.0.to_owned());
            // the market of AURORA_DEMO_AREA_SEARCH: several owners and creators
            let market = local_id.and_then(crate::demo::market::props);
            let (name, owner_id, creator_id, group) = match market {
                Some((name, _, creator, Some(group))) => (name, group, creator, Some(group)),
                Some((name, owner, creator, None)) => (name, owner, creator, group),
                None => (name, owner_id, creator_id, group),
            };
            let description = outfit.map_or_else(String::new, |o| o.1.to_owned());
            ObjectProps {
                object_id: id,
                full: true,
                creator_id,
                owner_id,
                group_id: group.unwrap_or_else(|| {
                    if owner_id == crate::demo::DEMO_AGENT {
                        crate::demo::DEMO_GROUP1
                    } else {
                        Uuid::nil()
                    }
                }),
                last_owner_id: DEMO_CREATOR,
                creation_date: 1_700_000_000_000_000,
                base_mask: PERM_ALL,
                owner_mask: PERM_ALL,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: perm::MOVE | perm::TRANSFER | perm::COPY | perm::MODIFY,
                sale_type: crate::demo::action_sale_type(id),
                sale_price: 10,
                category: 0,
                inventory_serial: 1,
                item_id,
                folder_id,
                from_task_id,
                name,
                description,
                touch_name: touch_name.into(),
                sit_name: sit_name.into(),
            }
        })
    }

    /// Contents listed by another demo (`demo.rs`'s click-action cubes):
    /// MoveTaskInventory then finds their items.
    pub fn adopt_contents(&mut self, id: Uuid, items: &[TaskItem]) {
        self.contents.entry(id).or_insert_with(|| (1, items.to_vec()));
    }

    fn contents_of(&mut self, id: Uuid) -> &mut (i16, Vec<TaskItem>) {
        if id == super::deep_perms::demo::id(super::deep_perms::demo::CHEST) && !self.contents.contains_key(&id) {
            let (items, stored, links, groups) = super::deep_perms::demo::contents(id);
            self.stored.extend(stored);
            self.stored_links.extend(links);
            self.stored_group.extend(groups);
            self.contents.insert(id, (1, items));
        }
        if id == super::replace::demo::id(super::replace::demo::SOURCE) {
            return self.contents.entry(id).or_insert((1, Vec::new()));
        }
        self.contents.entry(id).or_insert_with(|| {
            if script_copy_demo()
                && (super::replace::demo::id(super::replace::demo::ROOT)..=super::replace::demo::id(super::replace::demo::ROOT + 14))
                    .contains(&id)
            {
                return (
                    1,
                    if id == super::replace::demo::id(super::replace::demo::TARGET) {
                        vec![demo_item(id, 1, aurora_i18n::tr!("build-demo-sim-door-script"), 10, 10)]
                    } else {
                        Vec::new()
                    },
                );
            }
            let mut items = vec![
                demo_item(id, 1, aurora_i18n::tr!("build-demo-sim-door-script"), 10, 10),
                demo_item(id, 2, aurora_i18n::tr!("build-demo-sim-instructions"), 7, 7),
                demo_item(id, 3, aurora_i18n::tr!("build-demo-sim-light-wood"), 0, 0),
                // an object to drag out onto the land (rez from contents)
                demo_item(id, 4, aurora_i18n::tr!("build-demo-sim-chair"), 6, 6),
            ];
            if std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("contents-move-plain,")) {
                items.retain(|i| i.asset_type != 10);
            }
            // a second no-copy item, for the shared move warning
            if std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("contents-move-many,")) {
                items[1].owner_mask &= !perm::COPY;
            }
            (1, items)
        })
    }

    fn uuids(world: &World, handle: u64, ids: &[u32]) -> Vec<Uuid> {
        ids.iter()
            .filter_map(|&l| {
                world
                    .objects
                    .index_of(&crate::world::objects::ObjKey {
                        region: handle,
                        local_id: l,
                    })
                    .and_then(|i| world.objects.get(i))
                    .map(|o| o.full_id)
            })
            .collect()
    }

    fn reply_props(&mut self, world: &World, ids: &[Uuid]) -> Vec<NetEvent> {
        let list: Vec<ObjectProps> = ids.iter().map(|id| self.props_of(world, *id).clone()).collect();
        if list.is_empty() {
            Vec::new()
        } else {
            vec![NetEvent::ObjectProperties(list)]
        }
    }

    /// Events a simulator would send back for a build command.
    /// The held reports whose round trip is over at `now`, in order.
    pub fn arrived(&mut self, now: Instant) -> Vec<NetEvent> {
        let mut out = Vec::new();
        while self.late_echoes.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((_, ev)) = self.late_echoes.pop_front() {
                out.push(ev);
            }
        }
        out
    }

    pub fn reply(&mut self, world: &World, c: &BuildCmd) -> Vec<NetEvent> {
        match c {
            BuildCmd::Add(p) => {
                let object = crate::demo::build_rez(p);
                self.rezzed_groups.insert(object.full_id, p.group_id);
                vec![NetEvent::ObjectUpdates {
                    handle: p.handle,
                    objects: vec![object],
                }]
            }

            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                ..
            } if *destination == aurora_net::build::derez::TRASH || *destination == aurora_net::build::derez::RETURN_TO_OWNER => {
                vec![NetEvent::ObjectsKilled {
                    handle: *handle,
                    local_ids: local_ids.clone(),
                }]
            }
            // Take / Take Copy: one object item per DeRez in the folder asked
            // for, coalesced when it holds several objects, as the simulator
            // does (take separate sends one DeRez per object); a take leaves
            // the world.
            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                ..
            } if *destination == aurora_net::build::derez::TAKE_INTO_AGENT_INVENTORY
                || *destination == aurora_net::build::derez::ACQUIRE_TO_AGENT_INVENTORY =>
            {
                let ids = Self::uuids(world, *handle, local_ids);
                let items = ids
                    .first()
                    .map(|id| aurora_net::inventory::InvItem {
                        id: Uuid::new_v4(),
                        parent: *destination_id,
                        name: self.props_of(world, *id).name.clone(),
                        asset_type: 6,
                        inv_type: 6,
                        flags: if ids.len() > 1 {
                            crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS
                        } else {
                            0
                        },
                        owner: world.agent_id,
                        base_mask: PERM_ALL,
                        owner_mask: PERM_ALL,
                        ..Default::default()
                    })
                    .into_iter()
                    .collect();
                let mut events = vec![NetEvent::InventoryUpdated {
                    bulk: false,
                    folders: Vec::new(),
                    items,
                }];
                if *destination == aurora_net::build::derez::TAKE_INTO_AGENT_INVENTORY {
                    events.push(NetEvent::ObjectsKilled {
                        handle: *handle,
                        local_ids: local_ids.clone(),
                    });
                }
                events
            }
            // Save back to object contents: a copy of the object, as an
            // object item of the destination's contents, which the
            // simulator lists again; the object stays in the world.
            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                ..
            } if *destination == aurora_net::build::derez::SAVE_INTO_TASK_INVENTORY => {
                let ids = Self::uuids(world, *handle, local_ids);
                let Some(id) = ids.first().copied() else {
                    return Vec::new();
                };
                if let Some(events) = self.save_stored(world, &ids, *destination_id) {
                    return events;
                }
                let name = self.props_of(world, id).name.clone();
                let entry = self.contents_of(*destination_id);
                entry.0 = entry.0.wrapping_add(1);
                entry.1.push(TaskItem {
                    item_id: Uuid::new_v4(),
                    parent_id: *destination_id,
                    creator_id: DEMO_CREATOR,
                    owner_id: crate::demo::DEMO_AGENT,
                    last_owner_id: DEMO_CREATOR,
                    base_mask: PERM_ALL,
                    owner_mask: PERM_ALL,
                    next_owner_mask: perm::MOVE | perm::TRANSFER,
                    asset_type: 6,
                    inv_type: 6,
                    name,
                    creation_date: 1_700_000_000,
                    ..Default::default()
                });
                let (serial, items) = entry.clone();
                vec![NetEvent::TaskInventory {
                    object: *destination_id,
                    serial: Some(serial),
                    result: Ok(items),
                }]
            }
            // worn: it leaves the world (the demo has no inventory item to
            // wear it from)
            BuildCmd::Attach { handle, objects, .. } => vec![NetEvent::ObjectsKilled {
                handle: *handle,
                local_ids: objects.iter().map(|(id, _)| *id).collect(),
            }],
            BuildCmd::SetPhysicsParams {
                handle, local_id, params, ..
            } => {
                if let Some(id) = Self::uuids(world, *handle, &[*local_id]).first() {
                    self.physics.insert(*id, *params);
                }
                Vec::new()
            }
            BuildCmd::Transform { handle, updates } => {
                let mut objects = Vec::new();
                for update in updates {
                    let Some(id) = Self::uuids(world, *handle, &[update.local_id]).first().copied() else {
                        continue;
                    };
                    let Some(o) = world.objects.index_of_uuid(&id).and_then(|i| world.objects.get(i)) else {
                        continue;
                    };
                    let history = self.history.entry(id).or_default();
                    let now = Instant::now();
                    if history.last.is_none_or(|at| now - at > EDIT_GAP) {
                        history.undo.push((o.position, o.rotation, o.scale));
                        history.redo.clear();
                    }
                    history.last = Some(now);
                    let mut o = super::replace::demo_update(o);
                    if let Some(p) = update.position {
                        o.position = p;
                    }
                    if let Some(r) = update.rotation {
                        o.rotation = r;
                    }
                    if let Some(s) = update.scale {
                        o.scale = s;
                    }
                    objects.push(o);
                }
                let echo = NetEvent::ObjectUpdates { handle: *handle, objects };
                match echo_delay() {
                    Some(delay) => {
                        self.late_echoes.push_back((Instant::now() + delay, echo));
                        Vec::new()
                    }
                    None => vec![echo],
                }
            }
            BuildCmd::Undo { handle, ids, .. } | BuildCmd::Redo { handle, ids, .. } => {
                let redo = matches!(c, BuildCmd::Redo { .. });
                let mut objects = Vec::new();
                for id in ids {
                    let Some(o) = world.objects.index_of_uuid(id).and_then(|i| world.objects.get(i)) else {
                        continue;
                    };
                    let history = self.history.entry(*id).or_default();
                    let (from, to) = if redo {
                        (&mut history.redo, &mut history.undo)
                    } else {
                        (&mut history.undo, &mut history.redo)
                    };
                    let Some((position, rotation, scale)) = from.pop() else {
                        continue;
                    };
                    to.push((o.position, o.rotation, o.scale));
                    history.last = None;
                    let mut o = super::replace::demo_update(o);
                    (o.position, o.rotation, o.scale) = (position, rotation, scale);
                    objects.push(o);
                }
                vec![NetEvent::ObjectUpdates { handle: *handle, objects }]
            }
            BuildCmd::Delink { handle, local_ids } => {
                let mut objects = Vec::new();
                for id in Self::uuids(world, *handle, local_ids) {
                    let Some(idx) = world.objects.index_of_uuid(&id) else { continue };
                    let Some(o) = world.objects.get(idx) else { continue };
                    let Some((p, r)) = super::replace::region_transform(world, idx) else {
                        continue;
                    };
                    let mut o = super::replace::demo_update(o);
                    o.parent_id = 0;
                    o.position = p;
                    o.rotation = r;
                    objects.push(o);
                }
                vec![NetEvent::ObjectUpdates { handle: *handle, objects }]
            }
            BuildCmd::Link { handle, local_ids } => {
                let ids = Self::uuids(world, *handle, local_ids);
                let Some(root) = ids.first().and_then(|id| world.objects.index_of_uuid(id)) else {
                    return Vec::new();
                };
                let Some(ro) = world.objects.get(root) else { return Vec::new() };
                let Some((p, r)) = super::replace::region_transform(world, root) else {
                    return Vec::new();
                };
                let mut objects = Vec::new();
                for id in &ids {
                    let Some(idx) = world.objects.index_of_uuid(id) else { continue };
                    let Some(o) = world.objects.get(idx) else { continue };
                    let Some((op, or)) = super::replace::region_transform(world, idx) else {
                        continue;
                    };
                    let mut o = super::replace::demo_update(o);
                    o.parent_id = if idx == root { 0 } else { ro.key.local_id };
                    o.position = if idx == root { op } else { r.inverse() * (op - p) };
                    o.rotation = if idx == root { or } else { r.inverse() * or };
                    objects.push(o);
                }
                vec![NetEvent::ObjectUpdates { handle: *handle, objects }]
            }
            BuildCmd::MoveTaskInventory {
                handle,
                local_id,
                item_id,
                folder_id,
            } => {
                let Some(id) = Self::uuids(world, *handle, &[*local_id]).first().copied() else {
                    return Vec::new();
                };
                let Some(task) = self.contents_of(id).1.iter().find(|i| i.item_id == *item_id).cloned() else {
                    return Vec::new();
                };
                // Offline copy-feedback fixture: retain the no-copy cube so
                // the real observer sees a partial result and times out.
                if task.asset_type == 6 && std::env::var("AURORA_DEMO_ACTIONS").as_deref() == Ok("open-copy-timeout-yes") {
                    return Vec::new();
                }
                let Ok(kind) = super::task_inventory::kind(world, id, &task, true) else {
                    return Vec::new();
                };
                let moving = matches!(kind, super::task_inventory::TransferKind::Move { .. });
                if moving {
                    let inv = self.contents_of(id);
                    inv.1.retain(|i| i.item_id != *item_id);
                    inv.0 = inv.0.wrapping_add(1);
                    let serial = inv.0;
                    self.props_of(world, id).inventory_serial = serial;
                }
                let mut items: Vec<_> = world.inventory.items.values().filter(|i| i.parent == *folder_id).cloned().collect();
                items.push(aurora_net::inventory::InvItem {
                    id: Uuid::new_v4(),
                    parent: *folder_id,
                    name: task.name,
                    desc: task.description,
                    asset_type: task.asset_type as i32,
                    inv_type: task.inv_type as i32,
                    asset_id: task.asset_id,
                    creator: task.creator_id,
                    owner: task.owner_id,
                    last_owner: task.last_owner_id,
                    group_id: task.group_id,
                    group_owned: task.group_owned,
                    base_mask: task.base_mask,
                    owner_mask: task.owner_mask,
                    group_mask: task.group_mask,
                    everyone_mask: task.everyone_mask,
                    next_owner_mask: task.next_owner_mask,
                    created_at: task.creation_date as i64,
                    ..Default::default()
                });
                let mut events = vec![NetEvent::InventoryContents(vec![aurora_net::inventory::FolderContents {
                    folder_id: *folder_id,
                    owner_id: world.agent_id,
                    // A new item advances storage's snapshot version too.
                    version: world
                        .inventory
                        .folders
                        .get(folder_id)
                        .map_or(1, |f| f.info.version.max(0).saturating_add(1)),
                    folders: world
                        .inventory
                        .folders
                        .get(folder_id)
                        .map(|folder| {
                            folder
                                .children
                                .iter()
                                .filter_map(|id| world.inventory.folders.get(id).map(|folder| folder.info.clone()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    items,
                }])];
                if moving {
                    let (serial, items) = self.contents_of(id).clone();
                    events.push(NetEvent::TaskInventory {
                        object: id,
                        serial: Some(serial),
                        result: Ok(items),
                    });
                    events.push(NetEvent::ObjectProperties(vec![self.props_of(world, id).clone()]));
                }
                events
            }
            BuildCmd::RezInventory(rez)
                if !rez.from_task.is_nil() && (self.stored.contains_key(&rez.item.id) || self.stored_group.contains_key(&rez.item.id)) =>
            {
                self.rez_stored(world, rez)
            }
            BuildCmd::RezInventory(rez) => {
                let object = crate::demo::inventory_rez(rez);
                if rez.from_task.is_nil() {
                    self.rezzed_from.insert(object.full_id, rez.item.parent);
                    self.rezzed_item.insert(object.full_id, rez.item.id);
                } else {
                    self.rezzed_from_task.insert(object.full_id, rez.from_task);
                }
                self.rezzed_groups.insert(object.full_id, rez.group);
                vec![NetEvent::ObjectUpdates {
                    handle: rez.handle,
                    objects: vec![object],
                }]
            }
            BuildCmd::Select { handle, local_ids } => {
                let ids = Self::uuids(world, *handle, local_ids);
                self.reply_props(world, &ids)
            }
            BuildCmd::SetName { handle, local_ids, name } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    self.props_of(world, *id).name = name.clone();
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetDescription {
                handle,
                local_ids,
                description,
            } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    self.props_of(world, *id).description = description.clone();
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetPermissions {
                handle,
                local_ids,
                field,
                set,
                mask,
            } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    let p = self.props_of(world, *id);
                    let m = match *field {
                        perm_field::OWNER => &mut p.owner_mask,
                        perm_field::GROUP => &mut p.group_mask,
                        perm_field::EVERYONE => &mut p.everyone_mask,
                        perm_field::NEXT_OWNER => &mut p.next_owner_mask,
                        _ => &mut p.base_mask,
                    };
                    if *set {
                        *m |= mask;
                    } else {
                        *m &= !mask;
                    }
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetGroup {
                handle,
                local_ids,
                group_id,
            } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    self.props_of(world, *id).group_id = *group_id;
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetOwner {
                handle,
                local_ids,
                group_id,
                ..
            } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    let p = self.props_of(world, *id);
                    p.owner_id = *group_id;
                    p.group_id = *group_id;
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetSaleInfo {
                handle,
                local_ids,
                sale_type,
                price,
            } => {
                let ids = Self::uuids(world, *handle, local_ids);
                for id in &ids {
                    let p = self.props_of(world, *id);
                    p.sale_type = *sale_type;
                    p.sale_price = *price;
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::RequestTaskInventory { object_id, .. } => {
                let (serial, items) = self.contents_of(*object_id).clone();
                let root = TaskItem {
                    item_id: *object_id,
                    is_folder: true,
                    asset_type: 8,
                    inv_type: 8,
                    name: "Contents".into(),
                    ..Default::default()
                };
                let mut all = vec![root];
                all.extend(items);
                vec![NetEvent::TaskInventory {
                    object: *object_id,
                    serial: Some(serial),
                    result: Ok(all),
                }]
            }
            BuildCmd::RezScript {
                handle, local_id, item, ..
            }
            | BuildCmd::UpdateTaskInventory { handle, local_id, item } => {
                let ids = Self::uuids(world, *handle, &[*local_id]);
                let Some(id) = ids.first().copied() else { return Vec::new() };
                let entry = self.contents_of(id);
                let mut it = (**item).clone();
                if it.item_id.is_nil() || world.inventory.items.contains_key(&it.item_id) {
                    it.item_id = Uuid::new_v4();
                }
                let script_id = it.item_id;
                match entry.1.iter_mut().find(|i| i.item_id == it.item_id) {
                    Some(old) => *old = it,
                    None => entry.1.push(it),
                }
                entry.0 = entry.0.wrapping_add(1);
                let serial = entry.0;
                if let BuildCmd::RezScript { enabled, .. } = c {
                    self.running
                        .insert((id, script_id), (*enabled, aurora_net::scripts::CompileTarget::Mono));
                }
                self.props_of(world, id).inventory_serial = serial;
                self.reply_props(world, &ids)
            }
            // CreateTaskInventoryItem: the region writes the new item, a
            // running Mono or Luau script or an empty notecard
            BuildCmd::CreateTaskItem { item, tag, .. } => {
                let id = item.parent_id;
                if world.objects.index_of_uuid(&id).is_none() {
                    return vec![NetEvent::CapReply {
                        tag: *tag,
                        result: Ok(llsd_map! { "success" => false, "message" => "Object not found" }),
                    }];
                }
                let mut it = (**item).clone();
                it.item_id = Uuid::new_v4();
                it.asset_id = Uuid::new_v4();
                let item_id = it.item_id;
                let lua = aurora_net::scripts::is_slua_item(it.flags);
                let script = it.asset_type == super::contents::AT_LSL_TEXT;
                let entry = self.contents_of(id);
                entry.1.push(it);
                entry.0 = entry.0.wrapping_add(1);
                let serial = entry.0;
                if script {
                    let vm = if lua {
                        aurora_net::scripts::CompileTarget::Luau
                    } else {
                        aurora_net::scripts::CompileTarget::Mono
                    };
                    self.running.insert((id, item_id), (true, vm));
                }
                self.props_of(world, id).inventory_serial = serial;
                let mut events = self.reply_props(world, &[id]);
                events.push(NetEvent::CapReply {
                    tag: *tag,
                    result: Ok(llsd_map! { "success" => true, "item_id" => item_id }),
                });
                events
            }
            BuildCmd::RemoveTaskInventory { handle, local_id, item_id } => {
                let ids = Self::uuids(world, *handle, &[*local_id]);
                let Some(id) = ids.first().copied() else { return Vec::new() };
                let entry = self.contents_of(id);
                entry.1.retain(|i| i.item_id != *item_id);
                entry.0 = entry.0.wrapping_add(1);
                let serial = entry.0;
                self.props_of(world, id).inventory_serial = serial;
                self.reply_props(world, &ids)
            }
            BuildCmd::SetScriptRunning {
                object_id,
                item_id,
                running,
                ..
            } => {
                if script_copy_demo() {
                    self.pending_running.insert((*object_id, *item_id), *running);
                } else {
                    self.running
                        .entry((*object_id, *item_id))
                        .or_insert((true, aurora_net::scripts::CompileTarget::Mono))
                        .0 = *running;
                }
                Vec::new()
            }
            BuildCmd::GetScriptRunning { object_id, item_id, .. } => {
                let initial = *object_id != super::replace::demo::id(super::replace::demo::ROOT + 6);
                let (running, compile) = self
                    .running
                    .get(&(*object_id, *item_id))
                    .copied()
                    .unwrap_or((initial, aurora_net::scripts::CompileTarget::Mono));
                // First reply observes the previous state; the next query
                // confirms the asynchronous script-engine operation.
                if let Some(next) = self.pending_running.remove(&(*object_id, *item_id)) {
                    self.running.insert((*object_id, *item_id), (next, compile));
                }
                vec![NetEvent::ScriptRunning {
                    object_id: *object_id,
                    item_id: *item_id,
                    running,
                    compile: Some(compile),
                }]
            }
            // the hover tips' parcel under the pointer
            BuildCmd::ParcelRequest {
                handle,
                sequence,
                west,
                south,
                ..
            } if *sequence == aurora_net::land::HOVERED_PARCEL_SEQ_ID => {
                crate::demo::hover_tips::parcel_at(*west, *south, world.parcel.clone())
                    .map(|parcel| NetEvent::HoverParcel { handle: *handle, parcel })
                    .into_iter()
                    .collect()
            }
            BuildCmd::ParcelRequest {
                handle,
                sequence,
                west,
                south,
                ..
            } if *sequence >= 1_000_000 && std::env::var_os("AURORA_DEMO_PARCEL_VISIBILITY").is_some() => {
                vec![NetEvent::SelectedParcel {
                    handle: *handle,
                    sequence: *sequence,
                    parcel: crate::demo::parcel_visibility::parcel_at(*west, *south),
                }]
            }
            BuildCmd::ParcelRequestById {
                handle,
                sequence,
                local_id,
            } if std::env::var_os("AURORA_DEMO_PARCEL_VISIBILITY").is_some() => crate::demo::parcel_visibility::parcel_by_id(*local_id)
                .map(|parcel| NetEvent::SelectedParcel {
                    handle: *handle,
                    sequence: *sequence,
                    parcel,
                })
                .into_iter()
                .collect(),
            BuildCmd::ParcelRequest {
                handle,
                sequence,
                west,
                south,
                east,
                north,
                ..
            } => match world.parcel.clone() {
                Some(mut p) => {
                    // Explicit offline aggregate fixture: no unique owner, and
                    // an area matching the rectangle must not disable Join.
                    if *sequence >= aurora_net::build::BUILD_PARCEL_SEQ + 1000
                        && std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("select-join,"))
                        && (east - west) * (north - south) > 16.0
                    {
                        let p = std::sync::Arc::make_mut(&mut p);
                        p.request_result = aurora_net::land::PARCEL_RESULT_MULTIPLE;
                        p.owner_id = Uuid::nil();
                        p.area = ((east - west) * (north - south)) as i32;
                    }
                    vec![NetEvent::SelectedParcel {
                        handle: *handle,
                        sequence: *sequence,
                        parcel: p,
                    }]
                }
                None => Vec::new(),
            },
            BuildCmd::Cap { cap, body, tag, .. } => vec![NetEvent::CapReply {
                tag: *tag,
                result: Ok(self.cap_reply(world, cap, body)),
            }],
            _ => Vec::new(),
        }
    }

    /// RezObject of a stored object item (`AURORA_DEMO_DEEP_PERMS`): a copy
    /// holding what the item holds, under new item ids, with the item's
    /// name and masks, and its child prims with theirs; `run-refuse`
    /// answers with an alert instead.
    fn rez_stored(&mut self, world: &World, rez: &aurora_net::inventory::rez::RezInventory) -> Vec<NetEvent> {
        if self.deep_refuse || super::deep_perms::demo::refuse() {
            return vec![NetEvent::Alert {
                message: format!(
                    "Can't rez object '{}' at {{ {:.1}, {:.1}, {:.1} }} because the owner of this land does not allow it.",
                    rez.item.name, rez.ray_end.x, rez.ray_end.y, rez.ray_end.z
                ),
            }];
        }
        if let Some(parts) = self.stored_group.get(&rez.item.id).cloned() {
            return self.rez_group(world, rez, parts);
        }
        let object = crate::demo::inventory_rez(rez);
        let id = object.full_id;
        self.rezzed_from_task.insert(id, rez.from_task);
        self.rezzed_groups.insert(id, rez.group);
        self.stored_copies.insert(id, rez.item.id);
        let held = self.stored.get(&rez.item.id).cloned().unwrap_or_default();
        let items = self.adopt(held, id);
        self.contents.insert(id, (1, items));
        let mut objects = vec![object.clone()];
        let mut children = Vec::new();
        for (n, held) in self
            .stored_links
            .get(&rez.item.id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .enumerate()
        {
            let child_id = Uuid::new_v4();
            let mut child = object.clone();
            child.full_id = child_id;
            child.local_id = crate::demo::next_rez_local_id();
            child.parent_id = object.local_id;
            // (a child's position is its parent's frame)
            child.position = glam::Vec3::new(0.6 * (n as f32 + 1.0), 0.0, 0.0);
            child.scale = glam::Vec3::splat(0.5);
            let items = self.adopt(held, child_id);
            self.contents.insert(child_id, (1, items));
            children.push(child_id);
            objects.push(child);
        }
        self.copy_children.insert(id, children);
        let item = &rez.item;
        let p = self.props_of(world, id);
        p.name.clone_from(&item.name);
        p.base_mask = item.base_mask;
        p.owner_mask = item.owner_mask;
        p.group_mask = item.group_mask;
        p.everyone_mask = item.everyone_mask;
        p.next_owner_mask = item.next_owner_mask;
        vec![NetEvent::ObjectUpdates {
            handle: rez.handle,
            objects,
        }]
    }

    /// A coalesced item rezzed: one root per part, side by side, each with
    /// its own contents and the item's masks.
    fn rez_group(&mut self, world: &World, rez: &aurora_net::inventory::rez::RezInventory, parts: Vec<Vec<TaskItem>>) -> Vec<NetEvent> {
        let mut objects = Vec::new();
        let mut ids = Vec::new();
        for (n, held) in parts.into_iter().enumerate() {
            let mut object = crate::demo::inventory_rez(rez);
            object.position.x += 1.5 * n as f32;
            let id = object.full_id;
            self.rezzed_from_task.insert(id, rez.from_task);
            self.rezzed_groups.insert(id, rez.group);
            self.stored_copies.insert(id, rez.item.id);
            let items = self.adopt(held, id);
            self.contents.insert(id, (1, items));
            let item = &rez.item;
            let p = self.props_of(world, id);
            p.name.clone_from(&item.name);
            p.base_mask = item.base_mask;
            p.owner_mask = item.owner_mask;
            p.group_mask = item.group_mask;
            p.everyone_mask = item.everyone_mask;
            p.next_owner_mask = item.next_owner_mask;
            ids.push(id);
            objects.push(object);
        }
        for id in &ids {
            self.copy_parts.insert(*id, ids.clone());
        }
        vec![NetEvent::ObjectUpdates {
            handle: rez.handle,
            objects,
        }]
    }

    /// Items of a stored listing in a rezzed prim: new ids, what their
    /// object items hold following them.
    fn adopt(&mut self, held: Vec<TaskItem>, prim: Uuid) -> Vec<TaskItem> {
        let mut items = Vec::new();
        for mut it in held {
            let old = it.item_id;
            it.item_id = Uuid::new_v4();
            it.parent_id = prim;
            if let Some(inner) = self.stored.get(&old).cloned() {
                self.stored.insert(it.item_id, inner);
            }
            if let Some(links) = self.stored_links.get(&old).cloned() {
                self.stored_links.insert(it.item_id, links);
            }
            if let Some(parts) = self.stored_group.get(&old).cloned() {
                self.stored_group.insert(it.item_id, parts);
            }
            items.push(it);
        }
        items
    }

    /// Save back of a copy rezzed from a stored item: a new object item
    /// holding the copy's contents, its child prims' and its masks, next
    /// to the original (as the demo always did), or in its place with
    /// `run-replace`.
    fn save_stored(&mut self, world: &World, ids: &[Uuid], destination: Uuid) -> Option<Vec<NetEvent>> {
        let copy = *ids.first()?;
        let original = *self.stored_copies.get(&copy)?;
        // several objects saved together: one coalesced item (only the
        // first with `deep_split`, as a simulator that kept one)
        let group: Option<Vec<Vec<TaskItem>>> = (ids.len() > 1 && !self.deep_split).then(|| {
            ids.iter()
                .map(|id| self.contents.get(id).map(|c| c.1.clone()).unwrap_or_default())
                .collect()
        });
        let held = self.contents.get(&copy).map(|c| c.1.clone()).unwrap_or_default();
        let links: Vec<Vec<TaskItem>> = self
            .copy_children
            .get(&copy)
            .map(|children| {
                children
                    .iter()
                    .map(|c| self.contents.get(c).map(|c| c.1.clone()).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        let p = self.props_of(world, copy).clone();
        let in_place = self.deep_in_place || super::deep_perms::demo::replace_in_place();
        let entry = self.contents_of(destination);
        entry.0 = entry.0.wrapping_add(1);
        let existing = entry.1.iter().position(|i| i.item_id == original);
        let mut item = existing.and_then(|i| entry.1.get(i).cloned()).unwrap_or_else(|| TaskItem {
            parent_id: destination,
            creator_id: DEMO_CREATOR,
            owner_id: crate::demo::DEMO_AGENT,
            last_owner_id: DEMO_CREATOR,
            asset_type: 6,
            inv_type: 6,
            creation_date: 1_700_000_000,
            ..Default::default()
        });
        if group.is_some() {
            // a coalesced item keeps its name (the parts each have theirs)
            if existing.is_none() {
                item.name.clone_from(&p.name);
            }
            item.flags |= crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
        } else {
            item.name.clone_from(&p.name);
            item.flags &= !crate::ui::inventory::OBJECT_HAS_MULTIPLE_ITEMS;
        }
        item.asset_id = Uuid::new_v4();
        item.base_mask = p.base_mask;
        item.owner_mask = p.owner_mask;
        item.group_mask = p.group_mask;
        item.everyone_mask = p.everyone_mask;
        item.next_owner_mask = p.next_owner_mask;
        match existing {
            Some(i) if in_place => entry.1[i] = item.clone(),
            _ => {
                item.item_id = Uuid::new_v4();
                entry.1.push(item.clone());
            }
        }
        let (serial, items) = entry.clone();
        if let Some(parts) = group {
            self.stored.remove(&item.item_id);
            self.stored_group.insert(item.item_id, parts);
        } else {
            self.stored_group.remove(&item.item_id);
            self.stored.insert(item.item_id, held);
        }
        if !links.is_empty() {
            self.stored_links.insert(item.item_id, links);
        }
        Some(vec![NetEvent::TaskInventory {
            object: destination,
            serial: Some(serial),
            result: Ok(items),
        }])
    }

    /// What a stored object item holds (tests).
    #[cfg(test)]
    pub(crate) fn stored(&self, item: Uuid) -> Option<&Vec<TaskItem>> {
        self.stored.get(&item)
    }

    /// What each part of a stored coalesced item holds (tests).
    #[cfg(test)]
    pub(crate) fn stored_group(&self, item: Uuid) -> Option<&Vec<Vec<TaskItem>>> {
        self.stored_group.get(&item)
    }

    /// What the child prims of a stored object item hold (tests).
    #[cfg(test)]
    pub(crate) fn stored_links(&self, item: Uuid) -> Option<&Vec<Vec<TaskItem>>> {
        self.stored_links.get(&item)
    }

    /// Capability answers: costs from the prim counts, default physics.
    fn cap_reply(&mut self, world: &World, cap: &str, body: &Llsd) -> Llsd {
        let prims_of = |id: &Uuid| {
            world
                .objects
                .index_of_uuid(id)
                .map(|i| super::family(world, i).len() as f64)
                .unwrap_or(1.0)
        };
        match cap {
            "GetObjectCost" => {
                let mut m = Llsd::new_map();
                for v in body["object_ids"].as_array() {
                    let id = v.as_uuid();
                    let n = world
                        .objects
                        .index_of_uuid(&id)
                        .and_then(|i| world.objects.get(i))
                        .and_then(|o| crate::demo::market::land_impact(o.key.local_id))
                        .unwrap_or_else(|| prims_of(&id));
                    m.insert(
                        id.to_string(),
                        llsd_map! {
                            "resource_cost" => 1.0,
                            "linked_set_resource_cost" => n,
                            "physics_cost" => 0.1,
                            "linked_set_physics_cost" => 0.1 * n,
                        },
                    );
                }
                m
            }
            "ResourceCostSelected" => {
                let n: f64 = body["selected_roots"].as_array().iter().map(|v| prims_of(&v.as_uuid())).sum();
                llsd_map! { "selected" => llsd_map! { "streaming" => 0.1 * n, "physics" => 0.1 * n, "simulation" => 0.5 * n } }
            }
            "GetObjectPhysicsData" => {
                let mut m = Llsd::new_map();
                for v in body["object_ids"].as_array() {
                    let d = self.physics.get(&v.as_uuid()).copied().unwrap_or_default();
                    m.insert(
                        v.as_uuid().to_string(),
                        llsd_map! {
                            "PhysicsShapeType" => d.shape_type as i32,
                            "Density" => d.density as f64,
                            "Friction" => d.friction as f64,
                            "Restitution" => d.restitution as f64,
                            "GravityMultiplier" => d.gravity_multiplier as f64,
                        },
                    );
                }
                m
            }
            "ModifyMaterialParams" => llsd_map! { "success" => true },
            _ => Llsd::Undef,
        }
    }
}
