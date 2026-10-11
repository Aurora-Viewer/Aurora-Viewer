//! Protected notecard / script downloads: LLAssetStorage::getInvItemAsset,
//! LLTransferSourceParamsInvItem and LLTransferTarget (Firestorm, LGPL 2.1).
//! ViewerAsset cannot read these assets by UUID alone. The simulator checks
//! the inventory item's owner and permissions before sending its contents.

use super::{Session, emit};
use crate::inventory::InvItem;
use crate::types::NetEvent;
use aurora_msg::msgs::{TransferAbort, TransferInfo, TransferPacket, TransferRequest};
use aurora_msg::{IncomingPacket, Msg};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use uuid::Uuid;

const ASSET_CHANNEL: i32 = 2;
const INVENTORY_SOURCE: i32 = 3;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_PACKETS: i32 = 8192;
const MAX_DOWNLOADS: usize = 32;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Previews {
    pending: HashMap<Uuid, Download>,
}

struct Download {
    sim: SocketAddr,
    item: Uuid,
    asset: Uuid,
    object: Uuid,
    started: Instant,
    data: Assembly,
}

impl Download {
    fn event(&self, result: Result<Vec<u8>, String>) -> NetEvent {
        preview_event(self.object, self.item, self.asset, result)
    }
}

fn preview_event(object: Uuid, item: Uuid, asset: Uuid, result: Result<Vec<u8>, String>) -> NetEvent {
    if object.is_nil() {
        NetEvent::InventoryPreview { item, asset, result }
    } else {
        NetEvent::TaskNotecardPreview {
            object,
            item,
            asset,
            result,
        }
    }
}

enum Reply {
    Info { status: i32, size: i32 },
    Packet { status: i32, packet: i32, data: Vec<u8> },
    Abort,
}

/// Wait for TransferInfo and every numbered packet. A DONE packet can arrive
/// before packet zero or TransferInfo; it never makes a truncated file valid.
#[derive(Default)]
struct Assembly {
    size: Option<usize>,
    packets: BTreeMap<i32, Vec<u8>>,
    received: usize,
    last: Option<i32>,
}

fn interrupted() -> String {
    aurora_i18n::tr!("inventory-operations-read-interrupted").into()
}

fn too_large() -> String {
    aurora_i18n::tr!("inventory-operations-the-contents-exceed-the-maximum-preview-size").into()
}

fn status_error(status: i32) -> String {
    if status == -3 {
        aurora_i18n::tr!("app-inventory-you-are-not-allowed-to-read-this-content").into()
    } else {
        aurora_i18n::tr!("inventory-operations-could-not-load-contents").into()
    }
}

impl Assembly {
    fn accept(&mut self, reply: Reply) -> Result<Option<Vec<u8>>, String> {
        match reply {
            Reply::Info { status, size } => {
                if status != 0 {
                    return Err(status_error(status));
                }
                let size = usize::try_from(size).map_err(|_| interrupted())?;
                if size > MAX_BYTES {
                    return Err(too_large());
                }
                if self.size.is_some_and(|old| old != size) {
                    return Err(interrupted());
                }
                self.size = Some(size);
            }
            Reply::Packet { status, packet, data } => {
                if !matches!(status, 0 | 1) {
                    return Err(status_error(status));
                }
                if !(0..MAX_PACKETS).contains(&packet) || self.last.is_some_and(|last| packet > last) {
                    return Err(interrupted());
                }
                if status == 1 {
                    if self.last.is_some_and(|last| last != packet) || self.packets.keys().any(|p| *p > packet) {
                        return Err(interrupted());
                    }
                    self.last = Some(packet);
                }
                if let Some(old) = self.packets.get(&packet) {
                    if *old != data {
                        return Err(interrupted());
                    }
                } else {
                    if self.received + data.len() > MAX_BYTES {
                        return Err(too_large());
                    }
                    self.received += data.len();
                    self.packets.insert(packet, data);
                }
            }
            Reply::Abort => return Err(interrupted()),
        }
        let Some(size) = self.size else { return Ok(None) };
        if self.received > size {
            return Err(interrupted());
        }
        if self.last.is_some_and(|last| self.packets.len() == last as usize + 1)
            && self.packets.keys().copied().eq(0..self.packets.len() as i32)
        {
            if self.received != size {
                return Err(interrupted());
            }
            return Ok(Some(std::mem::take(&mut self.packets).into_values().flatten().collect()));
        }
        Ok(None)
    }
}

/// LLDataPackerBinaryBuffer: six UUIDs, then the S32 asset type little-endian.
fn request(agent: Uuid, session: Uuid, object: Uuid, item: &InvItem) -> TransferRequest {
    let mut m = TransferRequest::default();
    let t = &mut m.transfer_info;
    t.transfer_id = Uuid::new_v4();
    t.channel_type = ASSET_CHANNEL;
    t.source_type = INVENTORY_SOURCE;
    // Both document previews request priority in getInvItemAsset: 100 + 1.
    t.priority = 101.0;
    for id in [agent, session, item.owner, object, item.id, item.asset_id] {
        t.params.extend_from_slice(id.as_bytes());
    }
    t.params.extend_from_slice(&item.asset_type.to_le_bytes());
    m
}

impl Previews {
    fn receive(&mut self, from: SocketAddr, transfer: Uuid, channel: i32, reply: Reply) -> (bool, Option<NetEvent>) {
        let Some(d) = self
            .pending
            .get_mut(&transfer)
            .filter(|d| d.sim == from && channel == ASSET_CHANNEL)
        else {
            return (false, None);
        };
        let result = match d.data.accept(reply) {
            Ok(None) => return (true, None),
            Ok(Some(data)) => Ok(data),
            Err(reason) => Err(reason),
        };
        let event = d.event(result);
        self.pending.remove(&transfer);
        (true, Some(event))
    }
}

impl Session<'_> {
    pub(super) fn preview_inventory_document(&mut self, item: InvItem) {
        self.preview_document(self.main, Uuid::nil(), item);
    }

    pub(super) fn preview_task_notecard(&mut self, handle: u64, object: Uuid, item: InvItem) {
        self.preview_document(self.sim_for_handle(handle), object, item);
    }

    fn preview_document(&mut self, sim: Option<SocketAddr>, object: Uuid, item: InvItem) {
        let Some(sim) = sim.filter(|addr| self.sims.contains_key(addr)) else {
            emit(self.sh, preview_event(object, item.id, item.asset_id, Err(interrupted())));
            return;
        };
        // Several editors may request the same document on the same frame.
        // The resulting event updates them all, without duplicate transfers.
        if self
            .inventory_previews
            .pending
            .values()
            .any(|d| d.sim == sim && d.object == object && d.item == item.id && d.asset == item.asset_id)
        {
            return;
        }
        if self.inventory_previews.pending.len() >= MAX_DOWNLOADS {
            emit(
                self.sh,
                preview_event(
                    object,
                    item.id,
                    item.asset_id,
                    Err(aurora_i18n::tr!("inventory-preview-busy").into()),
                ),
            );
            return;
        }
        let m = request(self.agent_id(), self.session_id(), object, &item);
        self.inventory_previews.pending.insert(
            m.transfer_info.transfer_id,
            Download {
                sim,
                item: item.id,
                asset: item.asset_id,
                object,
                started: Instant::now(),
                data: Assembly::default(),
            },
        );
        self.send(sim, &m, true);
    }

    pub(super) fn dispatch_inventory_preview(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        let (transfer, channel, reply) = if pkt.id == TransferInfo::ID {
            let m: TransferInfo = pkt.decode()?;
            let t = m.transfer_info;
            (
                t.transfer_id,
                t.channel_type,
                Reply::Info {
                    status: t.status,
                    size: t.size,
                },
            )
        } else if pkt.id == TransferPacket::ID {
            let m: TransferPacket = pkt.decode()?;
            let t = m.transfer_data;
            (
                t.transfer_id,
                t.channel_type,
                Reply::Packet {
                    status: t.status,
                    packet: t.packet,
                    data: t.data,
                },
            )
        } else if pkt.id == TransferAbort::ID {
            let m: TransferAbort = pkt.decode()?;
            (m.transfer_info.transfer_id, m.transfer_info.channel_type, Reply::Abort)
        } else {
            return Ok(false);
        };
        let (handled, event) = self.inventory_previews.receive(from, transfer, channel, reply);
        if let Some(event) = event {
            if matches!(
                &event,
                NetEvent::InventoryPreview { result: Err(_), .. } | NetEvent::TaskNotecardPreview { result: Err(_), .. }
            ) {
                self.abort_inventory_preview(from, transfer);
            }
            emit(self.sh, event);
        }
        Ok(handled)
    }

    fn abort_inventory_preview(&mut self, sim: SocketAddr, transfer: Uuid) {
        let mut m = TransferAbort::default();
        m.transfer_info.transfer_id = transfer;
        m.transfer_info.channel_type = ASSET_CHANNEL;
        self.send(sim, &m, true);
    }

    pub(super) fn expire_inventory_previews(&mut self) {
        let expired: Vec<_> = self
            .inventory_previews
            .pending
            .iter()
            .filter(|(_, d)| d.started.elapsed() >= TIMEOUT || !self.sims.contains_key(&d.sim))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(d) = self.inventory_previews.pending.remove(&id) {
                self.abort_inventory_preview(d.sim, id);
                emit(self.sh, d.event(Err(aurora_i18n::tr!("inventory-preview-timeout").into())));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(packet: i32, data: &[u8], last: bool) -> Reply {
        Reply::Packet {
            status: i32::from(last),
            packet,
            data: data.to_vec(),
        }
    }

    #[test]
    fn inventory_request_carries_authorization_and_uses_the_inventory_source() {
        let item = InvItem {
            owner: Uuid::from_u128(3),
            id: Uuid::from_u128(4),
            asset_id: Uuid::from_u128(5),
            asset_type: 7,
            ..Default::default()
        };
        let m = request(Uuid::from_u128(1), Uuid::from_u128(2), Uuid::nil(), &item);
        let t = &m.transfer_info;
        assert_eq!((t.channel_type, t.source_type), (2, 3));
        assert_eq!(t.priority, 101.0);
        let ids: Vec<_> = t.params[..96]
            .as_chunks::<16>()
            .0
            .iter()
            .map(|b| Uuid::from_slice(b).expect("UUID"))
            .collect();
        assert_eq!(
            ids,
            [
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                item.owner,
                Uuid::nil(),
                item.id,
                item.asset_id
            ]
        );
        assert_eq!(&t.params[96..], &7i32.to_le_bytes());
        // Includes the Variable2 framing and UUID/S32 wire serialization.
        let mut wire = Vec::new();
        aurora_msg::build_packet(&m, 1, true, false, &[], &mut wire);
        let decoded: TransferRequest = aurora_msg::parse_packet(&wire).expect("packet").decode().expect("request");
        assert_eq!(decoded.transfer_info.params, t.params);
    }

    #[test]
    fn out_of_order_done_and_duplicates_wait_for_the_exact_size_and_all_packets() {
        let mut a = Assembly::default();
        assert!(a.accept(packet(2, b"ghi", true)).expect("last").is_none());
        assert!(a.accept(packet(0, b"abc", false)).expect("first").is_none());
        assert!(a.accept(packet(0, b"abc", false)).expect("duplicate").is_none());
        assert_eq!(a.received, 6);
        assert!(a.accept(packet(1, b"def", false)).expect("middle").is_none());
        assert_eq!(
            a.accept(Reply::Info { status: 0, size: 9 }).expect("info"),
            Some(b"abcdefghi".to_vec())
        );
    }

    #[test]
    fn task_requests_and_results_remain_scoped_to_the_containing_object() {
        let object = Uuid::from_u128(9);
        let item = InvItem {
            id: Uuid::from_u128(4),
            owner: Uuid::from_u128(3),
            asset_id: Uuid::from_u128(5),
            asset_type: 7,
            ..Default::default()
        };
        let m = request(Uuid::from_u128(1), Uuid::from_u128(2), object, &item);
        assert_eq!(&m.transfer_info.params[48..64], object.as_bytes());
        let transfer = m.transfer_info.transfer_id;
        let sim = "127.0.0.1:13000".parse().expect("sim");
        let mut previews = Previews::default();
        previews.pending.insert(
            transfer,
            Download {
                sim,
                item: item.id,
                asset: item.asset_id,
                object,
                started: Instant::now(),
                data: Assembly::default(),
            },
        );
        previews.receive(sim, transfer, 2, Reply::Info { status: 0, size: 3 });
        let (_, event) = previews.receive(sim, transfer, 2, packet(0, b"abc", true));
        assert!(
            matches!(event, Some(NetEvent::TaskNotecardPreview { object: o, item: i, asset: a, result: Ok(data) }) if o == object && i == item.id && a == item.asset_id && data == b"abc")
        );
    }

    #[test]
    fn malformed_and_oversized_transfers_never_become_valid_documents() {
        let mut a = Assembly::default();
        a.accept(Reply::Info { status: 0, size: 7 }).expect("info");
        assert!(a.accept(packet(0, b"short", true)).is_err());
        assert!(Assembly::default().accept(Reply::Info { status: 0, size: -1 }).is_err());
        assert!(
            Assembly::default()
                .accept(Reply::Info {
                    status: 0,
                    size: MAX_BYTES as i32 + 1
                })
                .is_err()
        );
        assert!(Assembly::default().accept(packet(-1, b"bad", false)).is_err());
        assert!(Assembly::default().accept(packet(MAX_PACKETS, b"bad", false)).is_err());
        let mut a = Assembly::default();
        a.accept(packet(0, b"first", false)).expect("first");
        assert!(a.accept(packet(0, b"other", false)).is_err());
        let mut a = Assembly::default();
        a.accept(packet(0, &vec![0; MAX_BYTES], false)).expect("maximum before info");
        assert!(a.accept(packet(1, b"overflow", true)).is_err());
    }

    #[test]
    fn replies_are_correlated_to_transfer_simulator_and_channel_and_failures_finish() {
        let sim: SocketAddr = "127.0.0.1:13000".parse().expect("sim");
        let other: SocketAddr = "127.0.0.1:13001".parse().expect("other");
        let transfer = Uuid::from_u128(6);
        let item = Uuid::from_u128(4);
        let asset = Uuid::from_u128(5);
        for (reply, success) in [
            (Reply::Info { status: -3, size: 0 }, false),
            (packet(0, b"abc", true), true),
            (Reply::Abort, false),
        ] {
            let mut previews = Previews::default();
            previews.pending.insert(
                transfer,
                Download {
                    sim,
                    item,
                    asset,
                    object: Uuid::nil(),
                    started: Instant::now(),
                    data: Assembly::default(),
                },
            );
            assert!(!previews.receive(other, transfer, 2, Reply::Abort).0);
            assert!(!previews.receive(sim, Uuid::nil(), 2, Reply::Abort).0);
            assert!(!previews.receive(sim, transfer, 1, Reply::Abort).0);
            assert_eq!(previews.pending.len(), 1);
            previews.receive(sim, transfer, 2, Reply::Info { status: 0, size: 3 });
            let (handled, event) = previews.receive(sim, transfer, 2, reply);
            assert!(handled && previews.pending.is_empty());
            let Some(NetEvent::InventoryPreview { item: i, asset: a, result }) = event else {
                panic!("preview event")
            };
            assert_eq!((i, a), (item, asset));
            assert_eq!(result.is_ok(), success);
            if success {
                assert_eq!(result.expect("contents"), b"abc");
            }
            assert!(!previews.receive(sim, transfer, 2, Reply::Abort).0);
        }
    }
}
