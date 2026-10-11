//! Inventory source transfers (LLAssetStorage::getInvItemAsset,
//! LLTransferSourceParamsInvItem::packParams, LLTransferManager;
//! Firestorm indra/llmessage, originally LGPL 2.1).

use super::*;
use crate::scripts::{MAX_SOURCE, ScriptTarget};
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
        if asset.is_nil() {
            emit(
                self.sh,
                NetEvent::ScriptLoaded {
                    request,
                    target,
                    result: Ok(crate::scripts::DEFAULT_SOURCE.into()),
                },
            );
            return;
        }
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

    pub(super) fn save_script(
        &mut self,
        request: Uuid,
        target: ScriptTarget,
        handle: RegionHandle,
        mono: bool,
        running: bool,
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
        let metadata = self
            .sim_for_handle(handle)
            .and_then(|addr| self.sims.get(&addr))
            .and_then(|sim| sim.caps.get("GetMetadata"))
            .cloned();
        tokio::spawn(async move {
            let result = crate::scripts::save(&http, &cap, metadata.as_deref(), target, mono, running, source).await;
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
