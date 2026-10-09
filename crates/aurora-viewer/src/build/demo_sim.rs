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
}

fn demo_item(object: Uuid, n: u128, name: &str, asset_type: i8, inv_type: i8) -> TaskItem {
    TaskItem {
        item_id: Uuid::from_u128(object.as_u128() ^ (0x17E0 << 64) ^ n),
        parent_id: object,
        creator_id: DEMO_CREATOR,
        owner_id: crate::demo::DEMO_AGENT,
        base_mask: PERM_ALL,
        owner_mask: PERM_ALL,
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

impl DemoSim {
    fn props_of(&mut self, world: &World, id: Uuid) -> &mut ObjectProps {
        self.props.entry(id).or_insert_with(|| {
            let name = world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| world.objects.get(i))
                .map(|o| {
                    if o.text.is_empty() {
                        format!("Objet {}", o.key.local_id)
                    } else {
                        o.text.lines().next().unwrap_or("").to_owned()
                    }
                })
                .unwrap_or_else(|| "Objet".into());
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
        self.contents.entry(id).or_insert_with(|| {
            (
                1,
                vec![
                    demo_item(id, 1, "Porte (script)", 10, 10),
                    demo_item(id, 2, "Mode d'emploi", 7, 7),
                    demo_item(id, 3, "Bois clair", 0, 0),
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
                if it.item_id.is_nil() {
                    it.item_id = Uuid::new_v4();
                }
                match entry.1.iter_mut().find(|i| i.item_id == it.item_id) {
                    Some(old) => *old = it,
                    None => entry.1.push(it),
                }
                entry.0 = entry.0.wrapping_add(1);
                let serial = entry.0;
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
            BuildCmd::GetScriptRunning { object_id, item_id, .. } => vec![NetEvent::ScriptRunning {
                object_id: *object_id,
                item_id: *item_id,
                running: true,
                mono: Some(true),
            }],
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
                let d = PhysicsParams::default();
                let mut m = Llsd::new_map();
                for v in body["object_ids"].as_array() {
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
