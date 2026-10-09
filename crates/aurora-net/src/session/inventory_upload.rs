//! LLViewerAssetStorage::storeAssetData and LLXfer::sendPacket /
//! processConfirmation (Firestorm indra/newview and llmessage, LGPL 2.1).
//! Upload first; CreateInventoryItem is sent only after AssetUploadComplete.

use super::*;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Uploads {
    assets: HashMap<Uuid, Upload>,
}

struct Upload {
    transaction: Uuid,
    parent: Uuid,
    kind: u8,
    name: String,
    data: Vec<u8>,
    started: Instant,
    transfer: Option<Transfer>,
}

struct Transfer {
    id: u64,
    chunk: usize,
    packet: u32,
    sent: Instant,
    retries: u8,
}

fn packet(data: &[u8], chunk: usize, index: u32) -> Option<(u32, Vec<u8>)> {
    let start = (index as usize).checked_mul(chunk)?;
    if start >= data.len() {
        return None;
    }
    let end = (start + chunk).min(data.len());
    let mut bytes = Vec::new();
    if index == 0 {
        bytes.extend_from_slice(&(data.len() as i32).to_le_bytes());
    }
    bytes.extend_from_slice(&data[start..end]);
    Some((index | if end == data.len() { 0x8000_0000 } else { 0 }, bytes))
}

impl Session<'_> {
    pub(super) fn create_wearable(&mut self, parent: Uuid, kind: u8, name: String, data: Vec<u8>) {
        if kind >= 17 || data.is_empty() || data.len() > 64 * 1024 || self.inventory_uploads.assets.len() >= 8 {
            emit(
                self.sh,
                NetEvent::InventoryOperationFailed("Création refusée : type, taille ou nombre de chargements invalide.".into()),
            );
            return;
        }
        let transaction = Uuid::new_v4();
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(transaction.as_bytes());
        key[16..].copy_from_slice(self.login.secure_session_id.as_bytes());
        let asset = Uuid::from_bytes(md5::compute(key).0);
        let mut m = AssetUploadRequest::default();
        m.asset_block.transaction_id = transaction;
        m.asset_block.type_ = if kind < 4 { 13 } else { 5 };
        // MTUBYTES = 1200, reserve the message/circuit overhead like Firestorm.
        if data.len() + 100 < 1200 {
            m.asset_block.asset_data.clone_from(&data);
        }
        self.inventory_uploads.assets.insert(
            asset,
            Upload {
                transaction,
                parent,
                kind,
                name,
                data,
                started: Instant::now(),
                transfer: None,
            },
        );
        self.send_main(&m, true);
    }

    fn send_inventory_packet(&mut self, asset: Uuid) {
        let Some(upload) = self.inventory_uploads.assets.get(&asset) else {
            return;
        };
        let Some(transfer) = &upload.transfer else {
            return;
        };
        let Some((index, data)) = packet(&upload.data, transfer.chunk, transfer.packet) else {
            return;
        };
        let mut m = SendXferPacket::default();
        m.xfer_id.id = transfer.id;
        m.xfer_id.packet = index;
        m.data_packet.data = data;
        self.send_main(&m, false);
    }

    pub(super) fn dispatch_inventory_upload(&mut self, from: SocketAddr, pkt: &IncomingPacket) -> Result<bool, aurora_msg::DecodeError> {
        if Some(from) != self.main {
            return Ok(false);
        }
        if pkt.id == RequestXfer::ID {
            let m: RequestXfer = pkt.decode()?;
            let Some(upload) = self.inventory_uploads.assets.get_mut(&m.xfer_id.v_file_id) else {
                return Ok(false);
            };
            let asset_type = if upload.kind < 4 { 13 } else { 5 };
            if m.xfer_id.v_file_type != asset_type || !field_str(&m.xfer_id.filename).is_empty() {
                return Ok(false);
            }
            upload.transfer = Some(Transfer {
                id: m.xfer_id.id,
                chunk: if m.xfer_id.use_big_packets { 7680 } else { 1000 },
                packet: 0,
                sent: Instant::now(),
                retries: 0,
            });
            self.send_inventory_packet(m.xfer_id.v_file_id);
            return Ok(true);
        }
        if pkt.id == ConfirmXferPacket::ID {
            let m: ConfirmXferPacket = pkt.decode()?;
            let Some(asset) = self
                .inventory_uploads
                .assets
                .iter()
                .find_map(|(asset, u)| u.transfer.as_ref().filter(|t| t.id == m.xfer_id.id).map(|_| *asset))
            else {
                return Ok(false);
            };
            if let Some(u) = self.inventory_uploads.assets.get_mut(&asset)
                && let Some(t) = &mut u.transfer
            {
                if m.xfer_id.packet & 0x7fff_ffff != t.packet {
                    return Ok(true);
                }
                t.packet += 1;
                t.sent = Instant::now();
                t.retries = 0;
            }
            self.send_inventory_packet(asset);
            return Ok(true);
        }
        if pkt.id == AssetUploadComplete::ID {
            let m: AssetUploadComplete = pkt.decode()?;
            let Some(upload) = self.inventory_uploads.assets.remove(&m.asset_block.uuid) else {
                return Ok(false);
            };
            if !m.asset_block.success {
                emit(
                    self.sh,
                    NetEvent::InventoryOperationFailed("Le serveur a refusé le nouvel asset de vêtement.".into()),
                );
                return Ok(true);
            }
            let mut m = CreateInventoryItem::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.inventory_block.folder_id = upload.parent;
            m.inventory_block.callback_id = self.inventory_callback(true);
            m.inventory_block.transaction_id = upload.transaction;
            m.inventory_block.next_owner_mask = 0x7fff_ffff;
            m.inventory_block.type_ = if upload.kind < 4 { 13 } else { 5 };
            m.inventory_block.inv_type = 18;
            m.inventory_block.wearable_type = upload.kind;
            m.inventory_block.name = str_field(&upload.name);
            self.send_main(&m, true);
            return Ok(true);
        }
        if pkt.id == AbortXfer::ID {
            let m: AbortXfer = pkt.decode()?;
            let asset = self
                .inventory_uploads
                .assets
                .iter()
                .find_map(|(id, u)| u.transfer.as_ref().filter(|t| t.id == m.xfer_id.id).map(|_| *id));
            if let Some(asset) = asset {
                self.inventory_uploads.assets.remove(&asset);
                emit(
                    self.sh,
                    NetEvent::InventoryOperationFailed("Le chargement du vêtement a été interrompu.".into()),
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn expire_inventory_uploads(&mut self) {
        let mut retry = Vec::new();
        let mut failed = false;
        self.inventory_uploads.assets.retain(|id, u| {
            if u.started.elapsed() > Duration::from_secs(60) {
                failed = true;
                return false;
            }
            if let Some(t) = &mut u.transfer
                && t.sent.elapsed() > Duration::from_secs(3)
                && (t.packet as usize) * t.chunk < u.data.len()
            {
                if t.retries >= 3 {
                    failed = true;
                    return false;
                }
                t.retries += 1;
                t.sent = Instant::now();
                retry.push(*id);
            }
            true
        });
        for asset in retry {
            self.send_inventory_packet(asset);
        }
        if failed {
            emit(
                self.sh,
                NetEvent::InventoryOperationFailed("Le chargement du vêtement a expiré. Rechargez l’inventaire avant de réessayer.".into()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xfer_prefix_and_final_bit_follow_llxfer() {
        let bytes: Vec<_> = (0..2500).map(|i| (i % 251) as u8).collect();
        let (i, first) = packet(&bytes, 1000, 0).expect("packet zero");
        assert_eq!(i, 0);
        assert_eq!(&first[..4], &2500i32.to_le_bytes());
        assert_eq!(&first[4..], &bytes[..1000]);
        let (i, middle) = packet(&bytes, 1000, 1).expect("packet one");
        assert_eq!(i, 1);
        assert_eq!(middle, bytes[1000..2000]);
        let (i, last) = packet(&bytes, 1000, 2).expect("packet two");
        assert_eq!(i, 0x80000002);
        assert_eq!(last, bytes[2000..]);
        assert!(packet(&bytes, 1000, 3).is_none());
    }
}
