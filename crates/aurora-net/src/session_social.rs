//! Session side of the mute list, groups and chat sessions: the mute list
//! download (MuteListRequest → MuteListUpdate → Xfer, LLMuteList and
//! LLXferManager), its entry updates, the group list (AgentDataUpdateRequest
//! → AgentGroupDataUpdate) and the ChatterBox event-queue messages and
//! ChatSessionRequest capability (LLIMMgr, llimview.cpp).

use super::{Session, emit};
use crate::social::{self, MuteListSource, SessionMethod};
use crate::types::{NetCommand, NetEvent};
use aurora_llsd::{Llsd, llsd_map};
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, field_str, str_field};
use std::net::SocketAddr;

/// ELLPath LL_PATH_CACHE: where the simulator writes the mute list file.
const LL_PATH_CACHE: u8 = 4;
/// Last packet of an xfer (LLXferManager::encodePacketNum).
const LAST_PACKET: u32 = 0x8000_0000;
/// LL_XFER_LARGE_PAYLOAD + the size prefix of the first packet.
const MAX_XFER_PACKET: usize = 7680 + 4;
/// Largest mute list file accepted (MuteListLimit 1000 entries × ~300 bytes).
const MAX_MUTE_FILE: usize = 1 << 20;

/// File download in progress (LLXfer_File, receive side).
#[derive(Debug)]
pub(super) struct XferDownload {
    id: u64,
    sim: SocketAddr,
    /// Packet expected next.
    next: u32,
    size: usize,
    data: Vec<u8>,
}

#[derive(Debug, Default)]
pub(super) struct SocialNet {
    mute_xfer: Option<XferDownload>,
}

impl Session<'_> {
    /// Commands of this module; false when `c` is not one of them.
    pub(super) fn on_social_command(&mut self, c: &NetCommand) -> bool {
        match c {
            NetCommand::RequestMuteList { crc } => {
                // LLMuteList::requestFromServer
                let mut m = MuteListRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.mute_data.mute_crc = *crc;
                self.send_main(&m, true);
                log::info!("mute list requested (cache crc {crc:#010x})");
            }
            NetCommand::UpdateMute { id, name, kind, flags } => {
                let mut m = UpdateMuteListEntry::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.mute_data.mute_id = *id;
                m.mute_data.mute_name = str_field(name);
                m.mute_data.mute_type = *kind;
                m.mute_data.mute_flags = *flags;
                self.send_main(&m, true);
            }
            NetCommand::RemoveMute { id, name } => {
                let mut m = RemoveMuteListEntry::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.mute_data.mute_id = *id;
                m.mute_data.mute_name = str_field(name);
                self.send_main(&m, true);
            }
            NetCommand::RequestGroups => {
                // LLAgent::sendAgentDataUpdateRequest: AgentDataUpdate + AgentGroupDataUpdate
                let mut m = AgentDataUpdateRequest::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                self.send_main(&m, true);
            }
            NetCommand::ChatSession { method, session } => self.chat_session_request(*method, *session),
            NetCommand::AgentAnimation { anim, start } => {
                // LLAgent::sendAnimationRequest
                let mut m = AgentAnimation::default();
                m.agent_data.agent_id = self.agent_id();
                m.agent_data.session_id = self.session_id();
                m.animation_list = vec![agent_animation::AnimationList {
                    anim_id: *anim,
                    start_anim: *start,
                }];
                m.physical_avatar_event_list = Vec::new();
                self.send_main(&m, true);
            }
            _ => return false,
        }
        true
    }

    /// POST to ChatSessionRequest (chatterBoxInvitationCoro, chatterBoxHistoryCoro).
    fn chat_session_request(&mut self, method: SessionMethod, session: uuid::Uuid) {
        let Some(url) = self.main_cap("ChatSessionRequest") else {
            log::warn!("ChatSessionRequest: no capability ({})", method.name());
            emit(
                self.sh,
                NetEvent::ChatSessionReply {
                    method,
                    session,
                    result: None,
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let body = llsd_map! { "method" => method.name(), "session-id" => session };
            let result = match crate::caps::post_llsd(&http, &url, &body).await {
                Ok(v) => Some(v),
                Err(e) => {
                    log::warn!("ChatSessionRequest {} for {session}: {e}", method.name());
                    None
                }
            };
            let _ = events.send(NetEvent::ChatSessionReply { method, session, result });
        });
    }

    /// UDP messages of this module; Ok(false) when not one of them.
    pub(super) fn dispatch_social(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let id = pkt.id;
        if id == MuteListUpdate::ID {
            let m: MuteListUpdate = pkt.decode()?;
            if m.mute_data.agent_id != self.agent_id() {
                log::warn!("mute list update for another agent");
                return Ok(true);
            }
            let filename = field_str(&m.mute_data.filename);
            // the file is fetched by xfer from the simulator that announced it
            let xfer = rand_u64();
            let mut r = RequestXfer::default();
            r.xfer_id.id = xfer;
            r.xfer_id.filename = str_field(&filename);
            r.xfer_id.file_path = LL_PATH_CACHE;
            r.xfer_id.delete_on_completion = true;
            r.xfer_id.use_big_packets = false;
            r.xfer_id.v_file_id = uuid::Uuid::nil();
            r.xfer_id.v_file_type = -1;
            self.send(from, &r, true);
            self.social.mute_xfer = Some(XferDownload {
                id: xfer,
                sim: from,
                next: 0,
                size: 0,
                data: Vec::new(),
            });
            log::info!("mute list: downloading {filename}");
        } else if id == UseCachedMuteList::ID {
            log::info!("mute list: server says the cached copy is current");
            emit(self.sh, NetEvent::MuteList(MuteListSource::UseCache));
        } else if id == SendXferPacket::ID {
            let m: SendXferPacket = pkt.decode()?;
            self.on_xfer_packet(from, m);
        } else if id == AbortXfer::ID {
            let m: AbortXfer = pkt.decode()?;
            if self.social.mute_xfer.as_ref().is_some_and(|x| x.id == m.xfer_id.id) {
                log::warn!("mute list xfer aborted by the simulator ({})", m.xfer_id.result);
                self.social.mute_xfer = None;
            }
        } else if id == AgentGroupDataUpdate::ID {
            let m: AgentGroupDataUpdate = pkt.decode()?;
            let groups = m
                .group_data
                .iter()
                .filter(|g| !g.group_id.is_nil())
                .map(|g| social::GroupMembership {
                    id: g.group_id,
                    name: field_str(&g.group_name),
                    insignia: g.group_insignia_id,
                    powers: g.group_powers,
                    accept_notices: g.accept_notices,
                    list_in_profile: true,
                    contribution: g.contribution,
                })
                .collect();
            emit(self.sh, NetEvent::Groups(groups));
        } else if id == AgentDropGroup::ID {
            let m: AgentDropGroup = pkt.decode()?;
            emit(self.sh, NetEvent::GroupDropped(m.agent_data.group_id));
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    /// LLXferManager::processReceiveData for the mute list file.
    fn on_xfer_packet(&mut self, from: SocketAddr, m: SendXferPacket) {
        let Some(x) = self.social.mute_xfer.as_mut() else {
            return;
        };
        if x.id != m.xfer_id.id || x.sim != from {
            return;
        }
        let num = m.xfer_id.packet & !LAST_PACKET;
        let confirm = |s: &mut Session, id: u64, packet: u32| {
            let mut c = ConfirmXferPacket::default();
            c.xfer_id.id = id;
            c.xfer_id.packet = packet;
            s.send(from, &c, false);
        };
        if num != x.next {
            // resend of the last packet: its confirmation was lost
            if num + 1 == x.next {
                let id = x.id;
                confirm(self, id, num);
            }
            return;
        }
        let data = &m.data_packet.data[..m.data_packet.data.len().min(MAX_XFER_PACKET)];
        if num == 0 {
            // first packet: S32 total size first (little endian on the wire)
            if data.len() < 4 {
                self.social.mute_xfer = None;
                return;
            }
            x.size = i32::from_le_bytes([data[0], data[1], data[2], data[3]]).max(0) as usize;
            x.data.extend_from_slice(&data[4..]);
        } else {
            x.data.extend_from_slice(data);
        }
        x.next += 1;
        let id = x.id;
        let too_big = x.data.len() > MAX_MUTE_FILE;
        confirm(self, id, num);
        if too_big {
            log::warn!("mute list file too large, dropped");
            self.social.mute_xfer = None;
        } else if m.xfer_id.packet & LAST_PACKET != 0 {
            let x = self.social.mute_xfer.take().unwrap();
            if x.size != x.data.len() {
                log::warn!("mute list: {} bytes received, {} announced", x.data.len(), x.size);
            }
            let text = String::from_utf8_lossy(&x.data).into_owned();
            log::info!("mute list downloaded ({} bytes)", x.data.len());
            emit(self.sh, NetEvent::MuteList(MuteListSource::File(text)));
        }
    }

    /// Event-queue messages of this module; false when not one of them.
    pub(super) fn on_social_eq(&mut self, message: &str, b: &Llsd) -> bool {
        match message {
            "AgentGroupDataUpdate" => {
                emit(self.sh, NetEvent::Groups(social::parse_group_data(b)));
            }
            "AgentDropGroup" => {
                let g = b["AgentData"][0]["GroupID"].as_uuid();
                if !g.is_nil() {
                    emit(self.sh, NetEvent::GroupDropped(g));
                }
            }
            "ChatterBoxInvitation" => {
                if let Some(inv) = social::parse_invitation(b) {
                    emit(self.sh, NetEvent::SessionInvite(inv));
                } else {
                    // voice / immediate invitations: no text session
                    log::info!("ChatterBoxInvitation without an instant message ignored");
                }
            }
            "ChatterBoxSessionStartReply" => {
                emit(
                    self.sh,
                    NetEvent::SessionStarted {
                        temp_session: b["temp_session_id"].as_uuid(),
                        session: b["session_id"].as_uuid(),
                        success: b["success"].as_bool(),
                        error: b["error"].as_str().to_owned(),
                        agents: social::parse_session_agents(b),
                    },
                );
            }
            "ChatterBoxSessionAgentListUpdates" => {
                emit(
                    self.sh,
                    NetEvent::SessionAgents {
                        session: b["session_id"].as_uuid(),
                        agents: social::parse_session_agents(b),
                    },
                );
            }
            "ChatterBoxSessionEventReply" => {
                if !b["success"].as_bool() {
                    emit(
                        self.sh,
                        NetEvent::SessionError {
                            session: b["session_id"].as_uuid(),
                            event: b["event"].as_str().to_owned(),
                            error: b["error"].as_str().to_owned(),
                        },
                    );
                }
            }
            "ForceCloseChatterBoxSession" => {
                emit(
                    self.sh,
                    NetEvent::SessionClosed {
                        session: b["session_id"].as_uuid(),
                        reason: b["reason"].as_str().to_owned(),
                    },
                );
            }
            // moderation / voice channel info of a session: nothing to show yet
            "ChatterBoxSessionUpdate" => {}
            _ => return false,
        }
        true
    }
}

/// Xfer ids only need to be unique among our transfers.
fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.finish()
}
