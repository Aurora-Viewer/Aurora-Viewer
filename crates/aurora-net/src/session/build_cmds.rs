//! Sending the build tool commands (see `crate::build`), each to the
//! simulator of the region the objects / land belong to.

use super::Session;
use crate::build::{BuildCmd, ObjectProps, brush_index};
use aurora_msg::msgs::{self, *};
use aurora_msg::{field_str, str_field};

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
        }
    }
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
