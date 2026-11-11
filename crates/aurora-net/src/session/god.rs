//! Admin status (god mode): the request and the region's answer. Port of
//! LLAgent::requestEnterGodMode / requestLeaveGodMode (indra/newview/
//! llagent.cpp) and process_grant_godlike_powers (llviewermessage.cpp,
//! originally LGPL 2.1). The region decides: OpenSim grants it to the
//! region's owner by default, to grid gods (user level 200 or more) and, by
//! its configuration, to estate managers (GodController.cs); Second Life only
//! to its staff. The viewer only asks and shows the level it is given.

use super::build_cmds::{encoded_packet_len, mtu_bounded_ranges};
use super::{Session, emit};
use crate::build::perm_field;
use crate::god::{GodCommand, PERM_ALL};
use crate::types::NetEvent;
use aurora_msg::msgs::*;
use aurora_msg::{IncomingPacket, Msg, str_field};

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
            | GodCommand::SaveState { handle }
            | GodCommand::KickUser { handle, .. } => *handle,
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
                // LLSelectMgr::sendListToRegions: at most MAX_OBJECTS_PER_PACKET,
                // and a new packet when one is full (isSendFull)
                let packet = |chunk: &[u32]| {
                    let mut m = ObjectOwner::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.header_data.override_ = true;
                    m.header_data.owner_id = owner_id;
                    m.header_data.group_id = group_id;
                    m.object_data = chunk.iter().map(|&id| object_owner::ObjectData { object_local_id: id }).collect();
                    m
                };
                for range in mtu_bounded_ranges(&local_ids, |chunk| encoded_packet_len(&packet(chunk))) {
                    self.send(addr, &packet(&local_ids[range]), true);
                }
            }
            GodCommand::Permissive { local_ids, .. } => {
                for field in [perm_field::BASE, perm_field::OWNER] {
                    let packet = |chunk: &[u32]| {
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
                        m
                    };
                    for range in mtu_bounded_ranges(&local_ids, |chunk| encoded_packet_len(&packet(chunk))) {
                        self.send(addr, &packet(&local_ids[range]), true);
                    }
                }
            }
            GodCommand::ForceDelete { local_ids, .. } => {
                let packet = |chunk: &[u32]| {
                    let mut m = ObjectDelete::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.agent_data.force = true;
                    m.object_data = chunk.iter().map(|&id| object_delete::ObjectData { object_local_id: id }).collect();
                    m
                };
                for range in mtu_bounded_ranges(&local_ids, |chunk| encoded_packet_len(&packet(chunk))) {
                    self.send(addr, &packet(&local_ids[range]), true);
                }
            }
            GodCommand::Request {
                method, params, objects, ..
            } => {
                // LLSelectMgr::sendGodlikeRequest: the method and its
                // parameters head every packet (packGodlikeHead), the roots'
                // local ids follow and alone are split (sendListToRegions,
                // packObjectIDAsParam); with no roots, the head alone
                let packet = |chunk: &[u32]| {
                    let mut m = GodlikeMessage::default();
                    m.agent_data.agent_id = agent_id;
                    m.agent_data.session_id = session_id;
                    m.method_data.method = str_field(&method);
                    m.param_list = params
                        .iter()
                        .cloned()
                        .chain(chunk.iter().map(u32::to_string))
                        .map(|p| godlike_message::ParamList { parameter: str_field(&p) })
                        .collect();
                    m
                };
                if objects.is_empty() {
                    self.send(addr, &packet(&[]), true);
                }
                for range in mtu_bounded_ranges(&objects, |chunk| encoded_packet_len(&packet(chunk))) {
                    self.send(addr, &packet(&objects[range]), true);
                }
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
            GodCommand::KickUser {
                agent_id: target,
                flags,
                reason,
                ..
            } => {
                let mut m = GodKickUser::default();
                m.user_info.god_id = agent_id;
                m.user_info.god_session_id = session_id;
                m.user_info.agent_id = target;
                m.user_info.kick_flags = flags;
                m.user_info.reason = str_field(&reason);
                self.send(addr, &m, true);
            }
        }
    }

    pub(super) fn dispatch_god(&mut self, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if pkt.id != GrantGodlikePowers::ID {
            return Ok(false);
        }
        let m: GrantGodlikePowers = pkt.decode()?;
        // only ours: agent and session, as process_grant_godlike_powers
        // (llviewermenu.cpp) checks them (the token is the simulator's,
        // ignored by viewers)
        if m.agent_data.agent_id == self.agent_id() && m.agent_data.session_id == self.session_id() {
            emit(self.sh, NetEvent::GodLevel(m.grant_data.god_level));
        } else {
            log::warn!("admin: a god level granted to another agent or session, ignored");
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::script_dialog_tests::{received, test_session, test_shared};
    use super::super::*;
    use super::*;

    #[tokio::test]
    async fn admin_commands_go_to_the_region_of_the_objects_with_the_override() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let other = UdpSocket::bind("127.0.0.1:0").await.expect("neighbour");
        let (main_addr, other_addr) = (main.local_addr().expect("main"), other.local_addr().expect("neighbour"));
        let (events, _received) = crossbeam_channel::unbounded();
        let sh = test_shared(events);
        let sims = [(main_addr, Sim::new(main_addr, 10)), (other_addr, Sim::new(other_addr, 20))];
        let mut session = test_session(&sh, sock.clone(), sims.into(), main_addr);
        sock.writable().await.expect("socket ready");
        let (me, group) = (Uuid::from_u128(1), Uuid::from_u128(7));

        // 300 objects: two packets, the Legacy Viewer's 254 per packet
        // (MAX_OBJECTS_PER_PACKET), each within the MTU
        let many: Vec<u32> = (1..=300).collect();
        session.on_command(NetCommand::God(GodCommand::ForceOwner {
            handle: 20,
            local_ids: many.clone(),
            owner_id: me,
            group_id: group,
        }));
        let owners: Vec<ObjectOwner> = received(&other, 2).await;
        assert!(
            owners
                .iter()
                .all(|m| m.header_data.override_ && m.header_data.owner_id == me && m.header_data.group_id == group)
        );
        assert!(owners.iter().all(|m| encoded_packet_len(m) <= aurora_msg::packet::MTU));
        let ids: Vec<u32> = owners
            .iter()
            .flat_map(|m| m.object_data.iter().map(|o| o.object_local_id))
            .collect();
        assert_eq!((owners[0].object_data.len(), ids), (254, many.clone()));

        // ObjectPermissions blocks are larger: 300 of them do not fit 254 to
        // a packet under the MTU (isSendFull); every object is still sent, for
        // the base then the owner field
        session.on_command(NetCommand::God(GodCommand::Permissive {
            handle: 10,
            local_ids: many.clone(),
        }));
        let mut sent = Vec::new();
        while sent.len() < 2 * many.len() {
            let m = received::<ObjectPermissions>(&main, 1).await.remove(0);
            assert!(m.header_data.override_);
            assert!(m.object_data.len() <= 254 && encoded_packet_len(&m) <= aurora_msg::packet::MTU);
            sent.extend(m.object_data.iter().map(|o| (o.field, o.object_local_id)));
        }
        let base: Vec<u32> = sent
            .iter()
            .filter(|(f, _)| *f == crate::build::perm_field::BASE)
            .map(|(_, id)| *id)
            .collect();
        let owner: Vec<u32> = sent
            .iter()
            .filter(|(f, _)| *f == crate::build::perm_field::OWNER)
            .map(|(_, id)| *id)
            .collect();
        assert_eq!((base, owner), (many.clone(), many.clone()));

        // Get Asset IDs: one parameter per root, split the same way, and
        // every packet starts with "assetids" (packGodlikeHead)
        session.on_command(NetCommand::God(GodCommand::Request {
            handle: 10,
            method: "objectinfo".into(),
            params: vec!["assetids".into()],
            objects: many.clone(),
        }));
        let mut ids = Vec::new();
        let mut packets = 0;
        while ids.len() < many.len() {
            let m = received::<GodlikeMessage>(&main, 1).await.remove(0);
            packets += 1;
            assert_eq!(field_str(&m.method_data.method), "objectinfo");
            assert!(m.param_list.len() <= 255 && encoded_packet_len(&m) <= aurora_msg::packet::MTU);
            let params: Vec<String> = m.param_list.iter().map(|p| field_str(&p.parameter)).collect();
            assert_eq!(params[0], "assetids", "packet {packets}");
            ids.extend(params[1..].iter().map(|p| p.parse::<u32>().expect("a local id")));
        }
        assert!(packets > 1);
        assert_eq!(ids, many);

        // base then owner, all set, with the override
        session.on_command(NetCommand::God(GodCommand::Permissive {
            handle: 10,
            local_ids: vec![5],
        }));
        let perms: Vec<ObjectPermissions> = received(&main, 2).await;
        let fields: Vec<(u8, u8, u32)> = perms
            .iter()
            .inspect(|m| assert!(m.header_data.override_))
            .flat_map(|m| m.object_data.iter().map(|o| (o.field, o.set, o.mask)))
            .collect();
        assert_eq!(
            fields,
            [
                (crate::build::perm_field::BASE, 1, PERM_ALL),
                (crate::build::perm_field::OWNER, 1, PERM_ALL)
            ]
        );

        session.on_command(NetCommand::God(GodCommand::ForceDelete {
            handle: 10,
            local_ids: vec![5, 6],
        }));
        let delete = received::<ObjectDelete>(&main, 1).await.remove(0);
        assert!(delete.agent_data.force);
        assert_eq!(delete.object_data.len(), 2);

        session.on_command(NetCommand::God(GodCommand::Request {
            handle: 10,
            method: "objectinfo".into(),
            params: vec!["assetids".into()],
            objects: vec![5],
        }));
        let request = received::<GodlikeMessage>(&main, 1).await.remove(0);
        assert_eq!(field_str(&request.method_data.method), "objectinfo");
        let params: Vec<String> = request.param_list.iter().map(|p| field_str(&p.parameter)).collect();
        assert_eq!(params, ["assetids", "5"]);

        session.on_command(NetCommand::God(GodCommand::ParcelForceOwner {
            handle: 10,
            local_id: 3,
            owner_id: me,
        }));
        let parcel = received::<ParcelGodForceOwner>(&main, 1).await.remove(0);
        assert_eq!((parcel.data.local_id, parcel.data.owner_id), (3, me));

        // no circuit to that region: nothing is sent
        session.on_command(NetCommand::God(GodCommand::SaveState { handle: 99 }));
        session.on_command(NetCommand::God(GodCommand::SaveState { handle: 10 }));
        let save = received::<StateSave>(&main, 1).await.remove(0);
        assert_eq!(save.agent_data.agent_id, me);

        session.on_command(NetCommand::God(GodCommand::KickUser {
            handle: 10,
            agent_id: Uuid::from_u128(9),
            flags: crate::god::KICK_FREEZE,
            reason: "Frozen".into(),
        }));
        let kick = received::<GodKickUser>(&main, 1).await.remove(0).user_info;
        assert_eq!(
            (kick.god_id, kick.god_session_id, kick.agent_id, kick.kick_flags),
            (me, Uuid::from_u128(2), Uuid::from_u128(9), 1)
        );
        assert_eq!(field_str(&kick.reason), "Frozen");
        let mut rest = [0; 1500];
        assert!(other.try_recv_from(&mut rest).is_err());
    }

    /// GrantGodlikePowers counts only for our agent and session
    /// (process_grant_godlike_powers).
    #[tokio::test]
    async fn a_god_level_counts_only_for_our_agent_and_session() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let main_addr = main.local_addr().expect("main");
        let (events, received_events) = crossbeam_channel::unbounded();
        let sh = test_shared(events);
        let mut session = test_session(&sh, sock, [(main_addr, Sim::new(main_addr, 10))].into(), main_addr);
        let grant = |agent: u128, session_id: u128, level: u8| {
            let mut m = GrantGodlikePowers::default();
            m.agent_data.agent_id = Uuid::from_u128(agent);
            m.agent_data.session_id = Uuid::from_u128(session_id);
            m.grant_data.god_level = level;
            let mut bytes = Vec::new();
            aurora_msg::build_packet(&m, 1, true, false, &[], &mut bytes);
            bytes
        };
        let levels = |rx: &crossbeam_channel::Receiver<(Instant, NetEvent)>| -> Vec<u8> {
            rx.try_iter()
                .filter_map(|(_, e)| if let NetEvent::GodLevel(l) = e { Some(l) } else { None })
                .collect()
        };
        for (agent, session_id) in [(9, 2), (1, 9)] {
            let bytes = grant(agent, session_id, 200);
            assert!(session.dispatch_god(&parse_packet(&bytes).expect("packet")).expect("decoded"));
        }
        assert!(levels(&received_events).is_empty(), "another agent or session: ignored");
        let bytes = grant(1, 2, 225);
        assert!(session.dispatch_god(&parse_packet(&bytes).expect("packet")).expect("decoded"));
        assert_eq!(levels(&received_events), [225]);
    }
}
