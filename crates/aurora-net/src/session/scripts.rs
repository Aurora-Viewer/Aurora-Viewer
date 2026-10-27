//! Inventory source transfers (LLAssetStorage::getInvItemAsset,
//! LLTransferSourceParamsInvItem::packParams, LLTransferManager;
//! Firestorm indra/llmessage, originally LGPL 2.1).

use super::*;
use crate::scripts::{MAX_SOURCE, ScriptOptions, ScriptTarget};
use std::collections::BTreeMap;
use uuid::Uuid;

const ASSET_CHANNEL: i32 = 2;
const INVENTORY_SOURCE: i32 = 3;
const MAX_TRANSFERS: usize = 16;

#[derive(Default)]
pub(super) struct ScriptTransfers {
    pending: HashMap<Uuid, SourceTransfer>,
}

struct SourceTransfer {
    request: Uuid,
    target: ScriptTarget,
    sim: SocketAddr,
    started: Instant,
    data: SourceData,
}

/// UDP packets may arrive before TransferInfo or out of order. Bound both
/// the source and the reordering queue, and never expose a partial document.
#[derive(Default)]
struct SourceData {
    size: Option<usize>,
    next: i32,
    queued: BTreeMap<i32, (bool, Vec<u8>)>,
    received: usize,
    bytes: Vec<u8>,
}

impl SourceData {
    fn info(&mut self, size: i32) -> Result<Option<String>, String> {
        let size = usize::try_from(size)
            .ok()
            .filter(|s| *s <= MAX_SOURCE)
            .ok_or_else(|| aurora_i18n::tr!("script-source-too-large").to_owned())?;
        if self.size.is_some_and(|s| s != size) || self.received > size {
            return Err(aurora_i18n::tr!("script-transfer-invalid").into());
        }
        self.size = Some(size);
        self.drain()
    }

    fn packet(&mut self, index: i32, status: i32, bytes: Vec<u8>) -> Result<Option<String>, String> {
        if index < 0 || !matches!(status, 0 | 1) {
            return Err(transfer_error(status));
        }
        if index < self.next || self.queued.contains_key(&index) {
            return Ok(None);
        }
        if index > self.next.saturating_add(100)
            || self.queued.len() >= 100
            || self.received + bytes.len() > self.size.unwrap_or(MAX_SOURCE)
        {
            return Err(aurora_i18n::tr!("script-transfer-invalid").into());
        }
        self.received += bytes.len();
        self.queued.insert(index, (status == 1, bytes));
        self.drain()
    }

    fn drain(&mut self) -> Result<Option<String>, String> {
        let Some(size) = self.size else { return Ok(None) };
        while let Some((last, bytes)) = self.queued.remove(&self.next) {
            self.bytes.extend_from_slice(&bytes);
            self.next += 1;
            if last {
                if self.bytes.len() != size || !self.queued.is_empty() {
                    return Err(aurora_i18n::tr!("script-transfer-invalid").into());
                }
                // Script assets may include the C string terminator.
                if self.bytes.last() == Some(&0) {
                    self.bytes.pop();
                }
                return String::from_utf8(std::mem::take(&mut self.bytes))
                    .map(Some)
                    .map_err(|_| aurora_i18n::tr!("script-transfer-invalid").into());
            }
        }
        Ok(None)
    }
}

fn transfer_error(status: i32) -> String {
    if status == -3 {
        aurora_i18n::tr!("script-no-permission").into()
    } else {
        aurora_i18n::tr!("script-load-failed").into()
    }
}

fn source_params(agent: Uuid, session: Uuid, owner: Uuid, target: ScriptTarget, asset: Uuid) -> Vec<u8> {
    let mut params = Vec::with_capacity(100);
    for id in [agent, session, owner, target.object, target.item, asset] {
        params.extend_from_slice(id.as_bytes());
    }
    params.extend_from_slice(&10i32.to_le_bytes());
    params
}

impl Session<'_> {
    pub(super) fn load_script(&mut self, request: Uuid, target: ScriptTarget, handle: RegionHandle, owner: Uuid, asset: Uuid) {
        // LLPreviewLSL::loadAsset / LLLiveLSLEditor::loadAsset request an
        // existing inventory item's source even when its asset UUID is hidden.
        // The simulator resolves it from OwnerID / TaskID / ItemID.
        let sim = if target.object.is_nil() {
            self.main
        } else {
            self.sim_for_handle(handle)
        };
        let Some(sim) = sim.filter(|_| self.script_transfers.pending.len() < MAX_TRANSFERS) else {
            emit(
                self.sh,
                NetEvent::ScriptLoaded {
                    request,
                    target,
                    result: Err(aurora_i18n::tr!("script-load-failed").into()),
                },
            );
            return;
        };
        let transfer = Uuid::new_v4();
        self.script_transfers.pending.insert(
            transfer,
            SourceTransfer {
                request,
                target,
                sim,
                started: Instant::now(),
                data: SourceData::default(),
            },
        );
        let mut m = TransferRequest::default();
        m.transfer_info.transfer_id = transfer;
        m.transfer_info.channel_type = ASSET_CHANNEL;
        m.transfer_info.source_type = INVENTORY_SOURCE;
        m.transfer_info.priority = 101.0;
        m.transfer_info.params = source_params(self.agent_id(), self.session_id(), owner, target, asset);
        self.send(sim, &m, true);
    }

    pub(super) fn load_script_experiences(&mut self, request: Uuid, target: ScriptTarget, handle: RegionHandle) {
        let metadata = self
            .sim_for_handle(handle)
            .and_then(|addr| self.sims.get(&addr))
            .and_then(|sim| sim.caps.get("GetMetadata"))
            .cloned();
        let creators = self.main_cap("GetCreatorExperiences");
        let info = self.main_cap("GetExperienceInfo");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = crate::scripts::experiences::load(&http, target, metadata.as_deref(), creators.as_deref(), info.as_deref()).await;
            let _ = events.send(NetEvent::ScriptExperiences { request, target, result });
        });
    }

    pub(super) fn save_script(
        &mut self,
        request: Uuid,
        target: ScriptTarget,
        handle: RegionHandle,
        options: ScriptOptions,
        source: String,
    ) {
        let name = if target.object.is_nil() {
            "UpdateScriptAgent"
        } else {
            "UpdateScriptTask"
        };
        let cap = if target.object.is_nil() {
            self.main_cap(name)
        } else {
            self.sim_for_handle(handle)
                .and_then(|addr| self.sims.get(&addr))
                .and_then(|sim| sim.caps.get(name))
                .cloned()
        };
        let Some(cap) = cap else {
            emit(
                self.sh,
                NetEvent::ScriptSaved {
                    request,
                    target,
                    result: Err(aurora_i18n::tr!("script-save-unavailable").into()),
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = crate::scripts::save(&http, &cap, target, options, source).await;
            let _ = events.send(NetEvent::ScriptSaved { request, target, result });
        });
    }

    fn finish_script_transfer(&mut self, id: Uuid, result: Result<String, String>) {
        if let Some(t) = self.script_transfers.pending.remove(&id) {
            if result.is_err() {
                let mut abort = TransferAbort::default();
                abort.transfer_info.transfer_id = id;
                abort.transfer_info.channel_type = ASSET_CHANNEL;
                self.send(t.sim, &abort, true);
            }
            emit(
                self.sh,
                NetEvent::ScriptLoaded {
                    request: t.request,
                    target: t.target,
                    result,
                },
            );
        }
    }

    pub(super) fn expire_script_transfers(&mut self) {
        let expired: Vec<_> = self
            .script_transfers
            .pending
            .iter()
            .filter(|(_, t)| t.started.elapsed() > Duration::from_secs(60))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.finish_script_transfer(id, Err(aurora_i18n::tr!("script-timeout").into()));
        }
    }

    pub(super) fn dispatch_script_transfer(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let (id, channel, result) = if pkt.id == TransferInfo::ID {
            let m: TransferInfo = pkt.decode()?;
            let b = m.transfer_info;
            let Some(t) = self.script_transfers.pending.get_mut(&b.transfer_id).filter(|t| t.sim == from) else {
                return Ok(false);
            };
            let result = if b.status == 0 {
                t.data.info(b.size)
            } else {
                Err(transfer_error(b.status))
            };
            (b.transfer_id, b.channel_type, result)
        } else if pkt.id == TransferPacket::ID {
            let m: TransferPacket = pkt.decode()?;
            let b = m.transfer_data;
            let Some(t) = self.script_transfers.pending.get_mut(&b.transfer_id).filter(|t| t.sim == from) else {
                return Ok(false);
            };
            (b.transfer_id, b.channel_type, t.data.packet(b.packet, b.status, b.data))
        } else if pkt.id == TransferAbort::ID {
            let m: TransferAbort = pkt.decode()?;
            let b = m.transfer_info;
            if !self.script_transfers.pending.get(&b.transfer_id).is_some_and(|t| t.sim == from) {
                return Ok(false);
            }
            (b.transfer_id, b.channel_type, Err(aurora_i18n::tr!("script-load-failed").into()))
        } else {
            return Ok(false);
        };
        if channel != ASSET_CHANNEL {
            self.finish_script_transfer(id, Err(aurora_i18n::tr!("script-transfer-invalid").into()));
        } else {
            match result {
                Ok(Some(source)) => self.finish_script_transfer(id, Ok(source)),
                Err(reason) => self.finish_script_transfer(id, Err(reason)),
                Ok(None) => {}
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hidden_asset_ids_load_the_servers_source_in_inventory_and_objects() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        let main = UdpSocket::bind("127.0.0.1:0").await.expect("main simulator");
        let neighbour = UdpSocket::bind("127.0.0.1:0").await.expect("object simulator");
        let main_addr = main.local_addr().expect("main address");
        let neighbour_addr = neighbour.local_addr().expect("object address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = Shared {
            http: reqwest::Client::new(),
            caps_http: reqwest::Client::new(),
            events: crate::EventSender(events),
            controls: Arc::new(Mutex::new(AgentControls::default())),
            stats: Arc::new(NetStats::default()),
            object_cache_dir: Arc::new(Mutex::new(None)),
            agent_language: Arc::new(Mutex::new(("en".into(), false))),
            bandwidth_kbps: Default::default(),
            viewer_stats: Default::default(),
            viewer_stats_posts: Default::default(),
        };
        let login = Arc::new(LoginResponse {
            agent_id: Uuid::from_u128(1),
            session_id: Uuid::from_u128(2),
            secure_session_id: Uuid::nil(),
            first_name: String::new(),
            last_name: String::new(),
            sim_ip: Ipv4Addr::LOCALHOST,
            sim_port: main_addr.port(),
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: Vec3::ZERO,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: Uuid::nil(),
            mfa_hash: None,
            raw: Llsd::Undef,
        });
        let (eq_tx, _) = mpsc::unbounded_channel();
        let (caps_tx, _) = mpsc::unbounded_channel();
        let (notecard_copy_tx, _) = mpsc::unbounded_channel();
        let (inventory_move_tx, _) = mpsc::unbounded_channel();
        let mut session = Session {
            sh: &sh,
            sock: sock.clone(),
            login,
            sims: [(main_addr, Sim::new(main_addr, 10)), (neighbour_addr, Sim::new(neighbour_addr, 20))].into(),
            main: Some(main_addr),
            eq_tx,
            caps_tx,
            notecard_copy_tx,
            inventory_move_tx,
            notecard_copies: HashMap::new(),
            agent_update: AgentUpdateRule::new(Instant::now()),
            control_flags: 0,
            one_shot: 0,
            cam_warned: false,
            far: 128.0,
            refuse_neighbours: false,
            fov: DEFAULT_FOV,
            logout_started: None,
            finished: false,
            agent_moved: false,
            viewer_stats: Default::default(),
            regions_visited: Default::default(),
            cam_requested_at: None,
            agent_parcel: None,
            pending_named_tp: None,
            object_cache: super::object_cache::ObjectCaches::new(None).0,
            throttle: crate::throttle::Throttle::new(crate::throttle::DEFAULT_KBPS, Instant::now()),
            social: Default::default(),
            tasks: Default::default(),
            land: Default::default(),
            estate_invoice: uuid::Uuid::nil(),
            terrain_raw: Default::default(),
            decode_failures: HashMap::new(),
            inventory_uploads: Default::default(),
            asset_imports: Default::default(),
            inventory_previews: Default::default(),
            script_transfers: Default::default(),
            inventory_callbacks: HashMap::new(),
            inventory_copies: super::inventory_copies::Pending::default(),
            attach_queue: Default::default(),
            inventory_merchant_pending: false,
            next_inventory_callback: 0,
        };
        for (object, sim) in [(Uuid::nil(), &main), (Uuid::from_u128(3), &neighbour)] {
            let target = ScriptTarget {
                object,
                item: Uuid::from_u128(4),
            };
            let request = Uuid::new_v4();
            sock.writable().await.expect("socket ready");
            // Existing scripts can hide their asset UUID. The containing
            // inventory item, not that UUID, identifies the requested source.
            session.load_script(request, target, 20, Uuid::from_u128(1), Uuid::nil());
            assert!(received.is_empty(), "do not substitute a local template for the server's source");
            let mut datagram = [0; 1200];
            let (size, _) = tokio::time::timeout(Duration::from_secs(2), sim.recv_from(&mut datagram))
                .await
                .expect("source requested from simulator")
                .expect("request packet");
            let packet = parse_packet(&datagram[..size]).expect("request datagram");
            assert_eq!(packet.id, TransferRequest::ID);
            let query: TransferRequest = packet.decode().expect("source request");
            let transfer = query.transfer_info.transfer_id;
            assert_eq!(
                query.transfer_info.params,
                source_params(Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(1), target, Uuid::nil())
            );
            let source = "default { state_entry() { llSay(0, \"Source déjà enregistrée !\"); } }";
            let mut info = TransferInfo::default();
            info.transfer_info.transfer_id = transfer;
            info.transfer_info.channel_type = ASSET_CHANNEL;
            info.transfer_info.size = source.len() as i32;
            let mut bytes = Vec::new();
            aurora_msg::build_packet(&info, 1, true, false, &[], &mut bytes);
            let from = sim.local_addr().expect("simulator address");
            assert!(
                session
                    .dispatch_script_transfer(from, &parse_packet(&bytes).expect("info datagram"))
                    .expect("info")
            );
            assert!(received.is_empty());
            let mut data = TransferPacket::default();
            data.transfer_data.transfer_id = transfer;
            data.transfer_data.channel_type = ASSET_CHANNEL;
            data.transfer_data.status = 1;
            data.transfer_data.data = source.as_bytes().to_vec();
            aurora_msg::build_packet(&data, 2, true, false, &[], &mut bytes);
            assert!(
                session
                    .dispatch_script_transfer(from, &parse_packet(&bytes).expect("source datagram"))
                    .expect("source")
            );
            let NetEvent::ScriptLoaded {
                request: reply,
                target: loaded_target,
                result,
            } = received.try_recv().expect("loaded source").1
            else {
                panic!("script reply")
            };
            assert_eq!((reply, loaded_target), (request, target));
            assert_eq!(result.expect("source"), source);
            assert!(session.script_transfers.pending.is_empty());
        }
    }

    #[test]
    fn source_request_carries_the_inventory_permission_context() {
        let ids: Vec<_> = (1..=6).map(Uuid::from_u128).collect();
        let params = source_params(
            ids[0],
            ids[1],
            ids[2],
            ScriptTarget {
                object: ids[3],
                item: ids[4],
            },
            ids[5],
        );
        assert_eq!(params.len(), 100);
        for (n, id) in ids.iter().enumerate() {
            assert_eq!(&params[n * 16..(n + 1) * 16], id.as_bytes());
        }
        assert_eq!(&params[96..], &10i32.to_le_bytes());
    }

    #[test]
    fn source_packets_can_precede_info_and_arrive_out_of_order() {
        let mut data = SourceData::default();
        assert!(data.packet(1, 1, b"world\0".to_vec()).expect("queued last").is_none());
        assert!(data.packet(0, 0, b"hello ".to_vec()).expect("queued first").is_none());
        assert!(data.packet(0, 0, b"hello ".to_vec()).expect("duplicate").is_none());
        assert_eq!(data.info(12).expect("complete").as_deref(), Some("hello world"));
    }

    #[test]
    fn truncated_oversized_and_denied_sources_never_become_documents() {
        let mut data = SourceData::default();
        assert!(data.info(MAX_SOURCE as i32 + 1).is_err());
        data.info(3).expect("valid size");
        assert!(data.packet(0, 1, b"xx".to_vec()).is_err());
        let mut data = SourceData::default();
        assert!(data.packet(101, 0, vec![]).is_err());
        assert!(data.packet(0, -3, vec![]).is_err());
    }
}
