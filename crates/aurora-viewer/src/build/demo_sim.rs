//! Demo mode: what a simulator answers to the build tools, so that every
//! tab of the build floater can be tried offline (properties of selected
//! objects, contents, land impact, weights, physics data, parcel).

use crate::world::World;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::NetEvent;
use aurora_net::build::{BuildCmd, ObjectProps, PhysicsParams, TaskItem, perm, perm_field};
use std::collections::HashMap;
use uuid::Uuid;

/// Creator of the demo objects (a demo resident).
const DEMO_CREATOR: Uuid = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
const PERM_ALL: u32 = 0x7fff_ffff;

#[derive(Default)]
pub struct DemoSim {
    props: HashMap<Uuid, ObjectProps>,
    contents: HashMap<Uuid, (i16, Vec<TaskItem>)>,
    running: HashMap<(Uuid, Uuid), (bool, bool)>,
    pending_running: HashMap<(Uuid, Uuid), bool>,
    physics: HashMap<Uuid, PhysicsParams>,
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

fn script_copy_demo() -> bool {
    std::env::var("AURORA_DEMO_REPLACE_PRIM").is_ok_and(|mode| mode == "script")
}

impl DemoSim {
    pub fn save_script(&mut self, target: aurora_net::scripts::ScriptTarget, asset: Uuid, running: bool, mono: bool) {
        if let Some(entry) = self.contents.get_mut(&target.object)
            && let Some(item) = entry.1.iter_mut().find(|i| i.item_id == target.item)
        {
            item.asset_id = asset;
            entry.0 = entry.0.wrapping_add(1);
        }
        self.running.insert((target.object, target.item), (running, mono));
    }
    fn props_of(&mut self, world: &World, id: Uuid) -> &mut ObjectProps {
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
            ObjectProps {
                object_id: id,
                full: true,
                creator_id: DEMO_CREATOR,
                owner_id: crate::demo::DEMO_AGENT,
                group_id: crate::demo::DEMO_GROUP1,
                last_owner_id: DEMO_CREATOR,
                creation_date: 1_700_000_000_000_000,
                base_mask: PERM_ALL,
                owner_mask: PERM_ALL,
                group_mask: 0,
                everyone_mask: 0,
                next_owner_mask: perm::MOVE | perm::TRANSFER | perm::COPY | perm::MODIFY,
                sale_type: 0,
                sale_price: 10,
                category: 0,
                inventory_serial: 1,
                name,
                description: String::new(),
                touch_name: String::new(),
                sit_name: String::new(),
            }
        })
    }

    fn contents_of(&mut self, id: Uuid) -> &mut (i16, Vec<TaskItem>) {
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
            (
                1,
                vec![
                    demo_item(id, 1, aurora_i18n::tr!("build-demo-sim-door-script"), 10, 10),
                    demo_item(id, 2, aurora_i18n::tr!("build-demo-sim-instructions"), 7, 7),
                    demo_item(id, 3, aurora_i18n::tr!("build-demo-sim-light-wood"), 0, 0),
                ],
            )
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
    pub fn reply(&mut self, world: &World, c: &BuildCmd) -> Vec<NetEvent> {
        match c {
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
                if task.owner_mask & perm::COPY == 0 {
                    return Vec::new();
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
                vec![NetEvent::InventoryContents(vec![aurora_net::inventory::FolderContents {
                    folder_id: *folder_id,
                    owner_id: world.agent_id,
                    version: 1,
                    folders: Vec::new(),
                    items,
                }])]
            }
            BuildCmd::RezInventory(rez) => vec![NetEvent::ObjectUpdates {
                handle: rez.handle,
                objects: vec![crate::demo::inventory_rez(rez)],
            }],
            BuildCmd::Select { handle, local_ids } => {
                let ids = Self::uuids(world, *handle, local_ids);
                self.reply_props(world, &ids)
            }
            BuildCmd::SetName { handle, local_id, name } => {
                let ids = Self::uuids(world, *handle, &[*local_id]);
                for id in &ids {
                    self.props_of(world, *id).name = name.clone();
                }
                self.reply_props(world, &ids)
            }
            BuildCmd::SetDescription {
                handle,
                local_id,
                description,
            } => {
                let ids = Self::uuids(world, *handle, &[*local_id]);
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
                    self.running.insert((id, script_id), (*enabled, true));
                }
                self.props_of(world, id).inventory_serial = serial;
                self.reply_props(world, &ids)
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
                    self.running.entry((*object_id, *item_id)).or_insert((true, true)).0 = *running;
                }
                Vec::new()
            }
            BuildCmd::GetScriptRunning { object_id, item_id, .. } => {
                let initial = *object_id != super::replace::demo::id(super::replace::demo::ROOT + 6);
                let (running, mono) = self.running.get(&(*object_id, *item_id)).copied().unwrap_or((initial, true));
                // First reply observes the previous state; the next query
                // confirms the asynchronous script-engine operation.
                if let Some(next) = self.pending_running.remove(&(*object_id, *item_id)) {
                    self.running.insert((*object_id, *item_id), (next, mono));
                }
                vec![NetEvent::ScriptRunning {
                    object_id: *object_id,
                    item_id: *item_id,
                    running,
                    mono: Some(mono),
                }]
            }
            BuildCmd::ParcelRequest { handle, sequence, .. } => match world.parcel.clone() {
                Some(p) => vec![NetEvent::SelectedParcel {
                    handle: *handle,
                    sequence: *sequence,
                    parcel: p,
                }],
                None => Vec::new(),
            },
            BuildCmd::Cap { cap, body, tag, .. } => vec![NetEvent::CapReply {
                tag: *tag,
                result: Ok(self.cap_reply(world, cap, body)),
            }],
            _ => Vec::new(),
        }
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
                    let n = prims_of(&id);
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
