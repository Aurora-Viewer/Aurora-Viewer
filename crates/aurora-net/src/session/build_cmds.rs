//! Sending the build tool commands (see `crate::build`), each to the
//! simulator of the region the objects / land belong to.

use super::{Session, emit};
use crate::build::{BuildCmd, ObjectProps, PhysicsParams, brush_index};
use crate::types::NetEvent;
use aurora_llsd::Llsd;
use aurora_msg::msgs::{self, *};
use aurora_msg::{IncomingPacket, Msg, field_str, str_field};

/// Most objects per message (LLSelectMgr::sendListToRegions MAX_OBJECTS_PER_PACKET).
const MAX_PER_PACKET: usize = 254;

impl Session<'_> {
    pub(super) fn on_build(&mut self, c: BuildCmd) {
        let (agent_id, session_id) = (self.agent_id(), self.session_id());
        match c {
            BuildCmd::Select { handle, local_ids } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectSelect::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk.iter().map(|&id| object_select::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::Deselect { handle, local_ids } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectDeselect::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_deselect::ObjectData { object_local_id: id })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::Transform { handle, updates } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                for chunk in updates.chunks(MAX_PER_PACKET / 4) {
                    let mut m = MultipleObjectUpdate::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk
                        .iter()
                        .map(|u| multiple_object_update::ObjectData {
                            object_local_id: u.local_id,
                            type_: u.kind(),
                            data: u.data(),
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::Add(p) => {
                let Some(addr) = self.sim_for_handle(p.handle) else { return };
                let mut m = ObjectAdd::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.group_id = p.group_id;
                let s = &p.shape;
                let o = &mut m.object_data;
                o.p_code = p.pcode;
                o.material = p.material;
                o.add_flags = p.add_flags;
                o.path_curve = s.path_curve;
                o.profile_curve = s.profile_curve;
                o.path_begin = s.path_begin;
                o.path_end = s.path_end;
                o.path_scale_x = s.path_scale_x;
                o.path_scale_y = s.path_scale_y;
                o.path_shear_x = s.path_shear_x;
                o.path_shear_y = s.path_shear_y;
                o.path_twist = s.path_twist;
                o.path_twist_begin = s.path_twist_begin;
                o.path_radius_offset = s.path_radius_offset;
                o.path_taper_x = s.path_taper_x;
                o.path_taper_y = s.path_taper_y;
                o.path_revolutions = s.path_revolutions;
                o.path_skew = s.path_skew;
                o.profile_begin = s.profile_begin;
                o.profile_end = s.profile_end;
                o.profile_hollow = s.profile_hollow;
                o.bypass_raycast = p.bypass_raycast as u8;
                o.ray_start = p.ray_start;
                o.ray_end = p.ray_end;
                o.ray_target_id = p.ray_target;
                o.ray_end_is_intersection = p.ray_end_is_intersection as u8;
                o.scale = p.scale;
                o.rotation = p.rotation;
                o.state = p.state;
                self.send(addr, &m, true);
            }
            BuildCmd::Duplicate {
                handle,
                local_ids,
                offset,
                flags,
                group_id,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectDuplicate::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.agent_data.group_id = group_id;
                    m.shared_data.offset = offset;
                    m.shared_data.duplicate_flags = flags;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_duplicate::ObjectData { object_local_id: id })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::DeRez {
                handle,
                local_ids,
                destination,
                destination_id,
                group_id,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                // take / return: one transaction in numbered packets of 250
                // (derez_objects); delete: each packet its own transaction,
                // count 1 number 1 (LLSelectMgr::packDeRezHeader)
                let trash = destination == crate::build::derez::TRASH;
                let transaction = uuid::Uuid::new_v4();
                let chunks: Vec<_> = local_ids.chunks(if trash { MAX_PER_PACKET } else { 250 }).collect();
                let count = chunks.len() as u8;
                for (i, chunk) in chunks.into_iter().enumerate() {
                    let (transaction, count, i) = if trash {
                        (uuid::Uuid::new_v4(), 1, 1)
                    } else {
                        (transaction, count, i)
                    };
                    let mut m = DeRezObject::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.agent_block.group_id = group_id;
                    m.agent_block.destination = destination;
                    m.agent_block.destination_id = destination_id;
                    m.agent_block.transaction_id = transaction;
                    m.agent_block.packet_count = count;
                    m.agent_block.packet_number = i as u8;
                    m.object_data = chunk.iter().map(|&id| de_rez_object::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::Link { handle, local_ids } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectLink::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = local_ids
                    .iter()
                    .map(|&id| object_link::ObjectData { object_local_id: id })
                    .collect();
                self.send(addr, &m, true);
            }
            BuildCmd::Delink { handle, local_ids } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectDelink::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = local_ids
                    .iter()
                    .map(|&id| object_delink::ObjectData { object_local_id: id })
                    .collect();
                self.send(addr, &m, true);
            }
            BuildCmd::SetName { handle, local_id, name } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectName::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = vec![object_name::ObjectData {
                    local_id,
                    name: str_field(&name),
                }];
                self.send(addr, &m, true);
            }
            BuildCmd::SetDescription {
                handle,
                local_id,
                description,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectDescription::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = vec![object_description::ObjectData {
                    local_id,
                    description: str_field(&description),
                }];
                self.send(addr, &m, true);
            }
            BuildCmd::SetFlags {
                handle,
                local_id,
                physics,
                temporary,
                phantom,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectFlagUpdate::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.object_local_id = local_id;
                m.agent_data.use_physics = physics;
                m.agent_data.is_temporary = temporary;
                m.agent_data.is_phantom = phantom;
                m.agent_data.casts_shadows = false;
                self.send(addr, &m, true);
            }
            BuildCmd::SetShape {
                handle,
                local_ids,
                shape: s,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectShape::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = local_ids
                    .iter()
                    .map(|&id| object_shape::ObjectData {
                        object_local_id: id,
                        path_curve: s.path_curve,
                        profile_curve: s.profile_curve,
                        path_begin: s.path_begin,
                        path_end: s.path_end,
                        path_scale_x: s.path_scale_x,
                        path_scale_y: s.path_scale_y,
                        path_shear_x: s.path_shear_x,
                        path_shear_y: s.path_shear_y,
                        path_twist: s.path_twist,
                        path_twist_begin: s.path_twist_begin,
                        path_radius_offset: s.path_radius_offset,
                        path_taper_x: s.path_taper_x,
                        path_taper_y: s.path_taper_y,
                        path_revolutions: s.path_revolutions,
                        path_skew: s.path_skew,
                        profile_begin: s.profile_begin,
                        profile_end: s.profile_end,
                        profile_hollow: s.profile_hollow,
                    })
                    .collect();
                self.send(addr, &m, true);
            }
            BuildCmd::SetMaterial {
                handle,
                local_ids,
                material,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ObjectMaterial::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = local_ids
                    .iter()
                    .map(|&id| object_material::ObjectData {
                        object_local_id: id,
                        material,
                    })
                    .collect();
                self.send(addr, &m, true);
            }
            BuildCmd::Undo { handle, ids, group_id } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = Undo::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.group_id = group_id;
                m.object_data = ids.iter().map(|&id| undo::ObjectData { object_id: id }).collect();
                self.send(addr, &m, true);
            }
            BuildCmd::Redo { handle, ids, group_id } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = Redo::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.group_id = group_id;
                m.object_data = ids.iter().map(|&id| redo::ObjectData { object_id: id }).collect();
                self.send(addr, &m, true);
            }
            BuildCmd::ModifyLand {
                handle,
                action,
                brush_size,
                seconds,
                height,
                parcel_local_id,
                west,
                south,
                east,
                north,
            } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = ModifyLand::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.modify_block.action = action;
                m.modify_block.brush_size = brush_index(brush_size);
                m.modify_block.seconds = seconds;
                m.modify_block.height = height;
                m.parcel_data = vec![modify_land::ParcelData {
                    local_id: parcel_local_id,
                    west,
                    south,
                    east,
                    north,
                }];
                m.modify_block_extended = vec![modify_land::ModifyBlockExtended { brush_size }];
                // brush strokes go out every frame: unreliable like LL's sendMessage
                self.send(addr, &m, false);
            }
            BuildCmd::UndoLand { handle } => {
                let Some(addr) = self.sim_for_handle(handle) else { return };
                let mut m = UndoLand::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                self.send(addr, &m, true);
            }
            other => self.on_build_more(other),
        }
    }

    /// The commands of the build floater's tabs and of the other tools.
    fn on_build_more(&mut self, c: BuildCmd) {
        let (agent_id, session_id) = (self.agent_id(), self.session_id());
        let Some(addr) = build_handle(&c).and_then(|h| self.sim_for_handle(h)) else {
            if let BuildCmd::Cap { tag, cap, .. } = c {
                emit(
                    self.sh,
                    NetEvent::CapReply {
                        tag,
                        result: Err(format!("{cap}: no region")),
                    },
                );
            }
            return;
        };
        match c {
            BuildCmd::SetPhysicsParams {
                local_id,
                physics,
                temporary,
                phantom,
                params,
                ..
            } => {
                let mut m = ObjectFlagUpdate::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.object_local_id = local_id;
                m.agent_data.use_physics = physics;
                m.agent_data.is_temporary = temporary;
                m.agent_data.is_phantom = phantom;
                m.extra_physics = vec![object_flag_update::ExtraPhysics {
                    physics_shape_type: params.shape_type,
                    density: params.density,
                    friction: params.friction,
                    restitution: params.restitution,
                    gravity_multiplier: params.gravity_multiplier,
                }];
                self.send(addr, &m, true);
            }
            BuildCmd::SetPermissions {
                local_ids,
                field,
                set,
                mask,
                ..
            } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectPermissions::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.header_data.override_ = false;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_permissions::ObjectData {
                            object_local_id: id,
                            field,
                            set: set as u8,
                            mask,
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetGroup { local_ids, group_id, .. } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectGroup::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.agent_data.group_id = group_id;
                    m.object_data = chunk.iter().map(|&id| object_group::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetOwner {
                local_ids,
                owner_id,
                group_id,
                ..
            } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectOwner::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.header_data.override_ = false;
                    m.header_data.owner_id = owner_id;
                    m.header_data.group_id = group_id;
                    m.object_data = chunk.iter().map(|&id| object_owner::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetSaleInfo {
                local_ids,
                sale_type,
                price,
                ..
            } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectSaleInfo::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_sale_info::ObjectData {
                            local_id: id,
                            sale_type,
                            sale_price: price,
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetClickAction { local_ids, action, .. } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectClickAction::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_click_action::ObjectData {
                            object_local_id: id,
                            click_action: action,
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetIncludeInSearch { local_ids, include, .. } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectIncludeInSearch::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_include_in_search::ObjectData {
                            object_local_id: id,
                            include_in_search: include,
                        })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::SetTextures {
                local_id,
                media_url,
                texture_entry,
                ..
            } => {
                let mut m = ObjectImage::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = vec![object_image::ObjectData {
                    object_local_id: local_id,
                    media_url: str_field(&media_url),
                    texture_entry,
                }];
                self.send(addr, &m, true);
            }
            BuildCmd::SetExtraParam {
                local_id,
                param_type,
                in_use,
                data,
                ..
            } => {
                let mut m = ObjectExtraParams::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data = vec![object_extra_params::ObjectData {
                    object_local_id: local_id,
                    param_type,
                    param_in_use: in_use,
                    param_size: data.len() as u32,
                    param_data: data,
                }];
                self.send(addr, &m, true);
            }
            BuildCmd::SpinStart { object_id, .. } => {
                let mut m = ObjectSpinStart::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data.object_id = object_id;
                self.send(addr, &m, true);
            }
            BuildCmd::SpinUpdate { object_id, rotation, .. } => {
                let mut m = ObjectSpinUpdate::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data.object_id = object_id;
                m.object_data.rotation = rotation.normalize();
                self.send(addr, &m, false);
            }
            BuildCmd::SpinStop { object_id, .. } => {
                let mut m = ObjectSpinStop::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.object_data.object_id = object_id;
                self.send(addr, &m, true);
            }
            BuildCmd::DuplicateOnRay {
                local_ids,
                group_id,
                ray_start,
                ray_end,
                ray_target,
                bypass_raycast,
                copy_centers,
                copy_rotates,
                ..
            } => {
                for chunk in local_ids.chunks(MAX_PER_PACKET) {
                    let mut m = ObjectDuplicateOnRay::default();
                    let a = &mut m.agent_data;
                    a.agent_id = agent_id;
                    a.session_id = session_id;
                    a.group_id = group_id;
                    a.ray_start = ray_start;
                    a.ray_end = ray_end;
                    a.bypass_raycast = bypass_raycast;
                    a.ray_end_is_intersection = false;
                    a.copy_centers = copy_centers;
                    a.copy_rotates = copy_rotates;
                    a.ray_target_id = ray_target;
                    a.duplicate_flags = 0;
                    m.object_data = chunk
                        .iter()
                        .map(|&id| object_duplicate_on_ray::ObjectData { object_local_id: id })
                        .collect();
                    self.send(addr, &m, true);
                }
            }
            BuildCmd::ParcelRequest {
                sequence,
                snap,
                west,
                south,
                east,
                north,
                ..
            } => {
                let mut m = ParcelPropertiesRequest::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                let d = &mut m.parcel_data;
                d.sequence_id = sequence;
                d.west = west;
                d.south = south;
                d.east = east;
                d.north = north;
                d.snap_selection = snap;
                self.send(addr, &m, true);
            }
            BuildCmd::ParcelDivide {
                west, south, east, north, ..
            } => {
                let mut m = ParcelDivide::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.parcel_data = parcel_divide::ParcelData { west, south, east, north };
                self.send(addr, &m, true);
            }
            BuildCmd::ParcelJoin {
                west, south, east, north, ..
            } => {
                let mut m = ParcelJoin::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.parcel_data = parcel_join::ParcelData { west, south, east, north };
                self.send(addr, &m, true);
            }
            BuildCmd::ParcelRelease { local_id, .. } => {
                let mut m = ParcelRelease::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            BuildCmd::RequestTaskInventory {
                handle,
                local_id,
                object_id,
            } => {
                // the capability first, else UDP + Xfer (object_actions.rs)
                self.request_task_inventory(handle, local_id, object_id);
            }
            BuildCmd::RemoveTaskInventory { local_id, item_id, .. } => {
                let mut m = RemoveTaskInventory::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.inventory_data.local_id = local_id;
                m.inventory_data.item_id = item_id;
                self.send(addr, &m, true);
            }
            BuildCmd::MoveTaskInventory {
                local_id,
                item_id,
                folder_id,
                ..
            } => {
                let mut m = MoveTaskInventory::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.folder_id = folder_id;
                m.inventory_data.local_id = local_id;
                m.inventory_data.item_id = item_id;
                self.send(addr, &m, true);
            }
            BuildCmd::UpdateTaskInventory { local_id, item, .. } => {
                let mut m = UpdateTaskInventory::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.update_data.local_id = local_id;
                m.update_data.key = 0; // TASK_INVENTORY_ITEM_KEY
                let i = &item;
                m.inventory_data = update_task_inventory::InventoryData {
                    item_id: i.item_id,
                    folder_id: i.parent_id,
                    creator_id: i.creator_id,
                    owner_id: i.owner_id,
                    group_id: i.group_id,
                    base_mask: i.base_mask,
                    owner_mask: i.owner_mask,
                    group_mask: i.group_mask,
                    everyone_mask: i.everyone_mask,
                    next_owner_mask: i.next_owner_mask,
                    group_owned: i.group_owned,
                    transaction_id: uuid::Uuid::nil(),
                    type_: i.asset_type,
                    inv_type: i.inv_type,
                    flags: i.flags,
                    sale_type: i.sale_type,
                    sale_price: i.sale_price,
                    name: str_field(&i.name),
                    description: str_field(&i.description),
                    creation_date: i.creation_date,
                    crc: crate::task_inventory::item_crc(i),
                };
                self.send(addr, &m, true);
            }
            BuildCmd::RezScript {
                local_id,
                enabled,
                group_id,
                item,
                ..
            } => {
                let mut m = RezScript::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.agent_data.group_id = group_id;
                m.update_block.object_local_id = local_id;
                m.update_block.enabled = enabled;
                let i = &item;
                m.inventory_block = rez_script::InventoryBlock {
                    item_id: i.item_id,
                    folder_id: i.parent_id,
                    creator_id: i.creator_id,
                    owner_id: i.owner_id,
                    group_id: i.group_id,
                    base_mask: i.base_mask,
                    owner_mask: i.owner_mask,
                    group_mask: i.group_mask,
                    everyone_mask: i.everyone_mask,
                    next_owner_mask: i.next_owner_mask,
                    group_owned: i.group_owned,
                    transaction_id: uuid::Uuid::nil(),
                    type_: i.asset_type,
                    inv_type: i.inv_type,
                    flags: i.flags,
                    sale_type: i.sale_type,
                    sale_price: i.sale_price,
                    name: str_field(&i.name),
                    description: str_field(&i.description),
                    creation_date: i.creation_date,
                    crc: crate::task_inventory::item_crc(i),
                };
                self.send(addr, &m, true);
            }
            BuildCmd::ScriptReset { object_id, item_id, .. } => {
                let mut m = ScriptReset::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.script.object_id = object_id;
                m.script.item_id = item_id;
                self.send(addr, &m, true);
            }
            BuildCmd::SetScriptRunning {
                object_id,
                item_id,
                running,
                ..
            } => {
                let mut m = SetScriptRunning::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.script.object_id = object_id;
                m.script.item_id = item_id;
                m.script.running = running;
                self.send(addr, &m, true);
            }
            BuildCmd::GetScriptRunning { object_id, item_id, .. } => {
                let mut m = GetScriptRunning::default();
                m.script.object_id = object_id;
                m.script.item_id = item_id;
                self.send(addr, &m, true);
            }
            BuildCmd::Cap { cap, put, body, tag, .. } => {
                let url = self.sims.get(&addr).and_then(|s| s.caps.get(&cap).cloned());
                let Some(url) = url else {
                    emit(
                        self.sh,
                        NetEvent::CapReply {
                            tag,
                            result: Err(format!("{cap}: capability not granted")),
                        },
                    );
                    return;
                };
                let http = self.sh.caps_http.clone();
                let events = self.sh.events.clone();
                tokio::spawn(async move {
                    let result = crate::caps::request_llsd(&http, &url, &body, put)
                        .await
                        .map_err(|e| format!("{cap}: {e}"));
                    if let Err(e) = &result {
                        log::warn!("{e}");
                    }
                    let _ = events.send(NetEvent::CapReply { tag, result });
                });
            }
            _ => {}
        }
    }

    /// Build-related UDP messages; Ok(false) when not one of them.
    pub(super) fn dispatch_build(&mut self, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let id = pkt.id;
        if id == ScriptRunningReply::ID {
            let m: ScriptRunningReply = pkt.decode()?;
            emit(
                self.sh,
                NetEvent::ScriptRunning {
                    object_id: m.script.object_id,
                    item_id: m.script.item_id,
                    running: m.script.running,
                    mono: None,
                },
            );
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// Build-related event queue messages; false when not one of them.
    pub(super) fn on_build_eq(&mut self, message: &str, b: &Llsd) -> bool {
        match message {
            // LLSelectMgr::processObjectPhysicsProperties
            "ObjectPhysicsProperties" => {
                let list = b["ObjectData"]
                    .as_array()
                    .iter()
                    .map(|o| {
                        (
                            o["LocalID"].as_u32(),
                            PhysicsParams {
                                shape_type: o["PhysicsShapeType"].as_i32() as u8,
                                density: o["Density"].as_f32(),
                                friction: o["Friction"].as_f32(),
                                restitution: o["Restitution"].as_f32(),
                                gravity_multiplier: o["GravityMultiplier"].as_f32(),
                            },
                        )
                    })
                    .collect();
                emit(self.sh, NetEvent::PhysicsProperties(list));
            }
            // the event queue form carries the Mono flag (LLLiveLSLEditor)
            "ScriptRunningReply" => {
                let s = &b["Script"][0];
                emit(
                    self.sh,
                    NetEvent::ScriptRunning {
                        object_id: s["ObjectID"].as_uuid(),
                        item_id: s["ItemID"].as_uuid(),
                        running: s["Running"].as_bool(),
                        mono: s.has("Mono").then(|| s["Mono"].as_bool()),
                    },
                );
            }
            _ => return false,
        }
        true
    }
}

/// The region a command of `on_build_more` goes to.
fn build_handle(c: &BuildCmd) -> Option<crate::types::RegionHandle> {
    Some(match c {
        BuildCmd::SetPhysicsParams { handle, .. }
        | BuildCmd::SetPermissions { handle, .. }
        | BuildCmd::SetGroup { handle, .. }
        | BuildCmd::SetOwner { handle, .. }
        | BuildCmd::SetSaleInfo { handle, .. }
        | BuildCmd::SetClickAction { handle, .. }
        | BuildCmd::SetIncludeInSearch { handle, .. }
        | BuildCmd::SetTextures { handle, .. }
        | BuildCmd::SetExtraParam { handle, .. }
        | BuildCmd::SpinStart { handle, .. }
        | BuildCmd::SpinUpdate { handle, .. }
        | BuildCmd::SpinStop { handle, .. }
        | BuildCmd::DuplicateOnRay { handle, .. }
        | BuildCmd::ParcelRequest { handle, .. }
        | BuildCmd::ParcelDivide { handle, .. }
        | BuildCmd::ParcelJoin { handle, .. }
        | BuildCmd::ParcelRelease { handle, .. }
        | BuildCmd::RequestTaskInventory { handle, .. }
        | BuildCmd::RemoveTaskInventory { handle, .. }
        | BuildCmd::MoveTaskInventory { handle, .. }
        | BuildCmd::UpdateTaskInventory { handle, .. }
        | BuildCmd::RezScript { handle, .. }
        | BuildCmd::ScriptReset { handle, .. }
        | BuildCmd::SetScriptRunning { handle, .. }
        | BuildCmd::GetScriptRunning { handle, .. }
        | BuildCmd::Cap { handle, .. } => *handle,
        _ => return None,
    })
}

/// ObjectProperties blocks (selected objects).
pub(super) fn parse_properties(m: &msgs::ObjectProperties) -> Vec<ObjectProps> {
    m.object_data
        .iter()
        .map(|b| ObjectProps {
            object_id: b.object_id,
            full: true,
            creator_id: b.creator_id,
            owner_id: b.owner_id,
            group_id: b.group_id,
            last_owner_id: b.last_owner_id,
            creation_date: b.creation_date,
            base_mask: b.base_mask,
            owner_mask: b.owner_mask,
            group_mask: b.group_mask,
            everyone_mask: b.everyone_mask,
            next_owner_mask: b.next_owner_mask,
            sale_type: b.sale_type,
            sale_price: b.sale_price,
            category: b.category,
            inventory_serial: b.inventory_serial,
            name: field_str(&b.name),
            description: field_str(&b.description),
            touch_name: field_str(&b.touch_name),
            sit_name: field_str(&b.sit_name),
        })
        .collect()
}

/// ObjectPropertiesFamily (one object).
pub(super) fn parse_family(m: &msgs::ObjectPropertiesFamily) -> ObjectProps {
    let b = &m.object_data;
    ObjectProps {
        object_id: b.object_id,
        full: false,
        owner_id: b.owner_id,
        group_id: b.group_id,
        last_owner_id: b.last_owner_id,
        base_mask: b.base_mask,
        owner_mask: b.owner_mask,
        group_mask: b.group_mask,
        everyone_mask: b.everyone_mask,
        next_owner_mask: b.next_owner_mask,
        sale_type: b.sale_type,
        sale_price: b.sale_price,
        category: b.category,
        name: field_str(&b.name),
        description: field_str(&b.description),
        ..Default::default()
    }
}
