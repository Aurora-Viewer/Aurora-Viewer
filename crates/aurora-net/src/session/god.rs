//! Admin status (god mode): the request and the region's answer. Port of
//! LLAgent::requestEnterGodMode / requestLeaveGodMode (indra/newview/
//! llagent.cpp) and process_grant_godlike_powers (llviewermessage.cpp,
//! originally LGPL 2.1). The region decides: OpenSim grants it to the
//! region's owner by default, to grid gods (user level 200 or more) and, by
//! its configuration, to estate managers (GodController.cs); Second Life only
//! to its staff. The viewer only asks and shows the level it is given.

use super::{Session, emit};
use crate::build::perm_field;
use crate::god::{GodCommand, PERM_ALL};
use crate::types::NetEvent;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, str_field};

/// Objects in one packet (the Legacy Viewer's MAX_OBJECTS_PER_PACKET).
const PER_PACKET: usize = 255;

impl Session<'_> {
    /// RequestGodlikePowers: `godlike` asks for the region's god level,
    /// false gives it back. The token is null from a viewer.
    pub(super) fn request_godlike(&mut self, godlike: bool) {
        let mut m = RequestGodlikePowers::default();
        m.agent_data.agent_id = self.agent_id();
        m.agent_data.session_id = self.session_id();
        m.request_block.godlike = godlike;
        self.send_main(&m, true);
    }

    /// Advanced › Admin's requests, to the region of the objects or parcel.
    pub(super) fn on_god_command(&mut self, c: GodCommand) {
        let handle = match &c {
            GodCommand::ForceOwner { handle, .. }
            | GodCommand::Permissive { handle, .. }
            | GodCommand::ForceDelete { handle, .. }
            | GodCommand::Request { handle, .. }
            | GodCommand::ParcelForceOwner { handle, .. }
            | GodCommand::SaveState { handle } => *handle,
        };
        let Some(addr) = self.sim_for_handle(handle) else {
            log::warn!("admin: no circuit to region {handle} for {c:?}");
            return;
        };
        log::info!("admin: {c:?}");
        let (agent_id, session_id) = (self.agent_id(), self.session_id());
        match c {
            GodCommand::ForceOwner {
                local_ids,
                owner_id,
                group_id,
                ..
            } => {
                for chunk in local_ids.chunks(PER_PACKET) {
                    let mut m = ObjectOwner::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.header_data.override_ = true;
                    m.header_data.owner_id = owner_id;
                    m.header_data.group_id = group_id;
                    m.object_data = chunk.iter().map(|&id| object_owner::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            GodCommand::Permissive { local_ids, .. } => {
                for field in [perm_field::BASE, perm_field::OWNER] {
                    for chunk in local_ids.chunks(PER_PACKET) {
                        let mut m = ObjectPermissions::default();
                        m.agent_data.agent_id = agent_id;
                        m.agent_data.session_id = session_id;
                        m.header_data.override_ = true;
                        m.object_data = chunk
                            .iter()
                            .map(|&id| object_permissions::ObjectData {
                                object_local_id: id,
                                field,
                                set: 1,
                                mask: PERM_ALL,
                            })
                            .collect();
                        self.send(addr, &m, true);
                    }
                }
            }
            GodCommand::ForceDelete { local_ids, .. } => {
                for chunk in local_ids.chunks(PER_PACKET) {
                    let mut m = ObjectDelete::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.agent_data.force = true;
                    m.object_data = chunk.iter().map(|&id| object_delete::ObjectData { object_local_id: id }).collect();
                    self.send(addr, &m, true);
                }
            }
            GodCommand::Request { method, params, .. } => {
                let mut m = GodlikeMessage::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.method_data.method = str_field(&method);
                m.param_list = params
                    .iter()
                    .map(|p| godlike_message::ParamList { parameter: str_field(p) })
                    .collect();
                self.send(addr, &m, true);
            }
            GodCommand::ParcelForceOwner { local_id, owner_id, .. } => {
                let mut m = ParcelGodForceOwner::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                m.data.owner_id = owner_id;
                m.data.local_id = local_id;
                self.send(addr, &m, true);
            }
            GodCommand::SaveState { .. } => {
                let mut m = StateSave::default();
                m.agent_data.agent_id = agent_id;
                m.agent_data.session_id = session_id;
                self.send(addr, &m, true);
            }
        }
    }

    pub(super) fn dispatch_god(&mut self, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id != GrantGodlikePowers::ID {
            return Ok(false);
        }
        let m: GrantGodlikePowers = pkt.decode()?;
        // only ours (the token is the simulator's, ignored by viewers)
        if m.agent_data.agent_id == self.agent_id() {
            emit(self.sh, NetEvent::GodLevel(m.grant_data.god_level));
        }
        Ok(true)
    }
}
