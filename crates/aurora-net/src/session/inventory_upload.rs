//! LLViewerAssetStorage::storeAssetData and LLXfer::sendPacket /
//! processConfirmation (Firestorm indra/newview and llmessage, LGPL 2.1).
//! Upload first; CreateInventoryItem is sent only after AssetUploadComplete.

use super::*;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Uploads {
    assets: HashMap<Uuid, Upload>,
    pub(super) wearable_copies: HashMap<u32, super::wearable_save::Pending>,
}

struct Upload {
    permissions: Option<crate::inventory::upload::Permissions>,
    transaction: Uuid,
    parent: Uuid,
    kind: u8,
    name: String,
    data: Vec<u8>,
    started: Instant,
    transfer: Option<Transfer>,
    import: Option<(Uuid, crate::inventory::upload::Request)>,
    wearable: Option<super::wearable_save::Pending>,
    /// An abuse report's screenshot: the report goes once it is stored.
    report: Option<Box<crate::abuse_report::AbuseReport>>,
}
impl Upload {
    fn asset_type(&self) -> i8 {
        if self.report.is_some() {
            return 0;
        }
        // LLXferManager::processFileRequest uses the type declared by
        // LLViewerAssetStorage::storeAssetData, including existing clothing.
        if let Some(pending) = &self.wearable {
            return pending.item.asset_type as i8;
        }
        self.import
            .as_ref()
            .map_or(if self.kind < 4 { 13 } else { 5 }, |(_, request)| request.kind.asset_type() as i8)
    }
}

struct Transfer {
    id: u64,
    chunk: usize,
    packet: u32,
    sent: Instant,
    retries: u8,
}

pub(super) fn packet(data: &[u8], chunk: usize, index: u32) -> Option<(u32, Vec<u8>)> {
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
    pub(super) fn inventory_uploads_busy(&self) -> bool {
        self.inventory_uploads.assets.len() >= 8
    }

    pub(super) fn upload_wearable_save(&mut self, pending: super::wearable_save::Pending, data: Vec<u8>) {
        let transaction = Uuid::new_v4();
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(transaction.as_bytes());
        key[16..].copy_from_slice(self.login.secure_session_id.as_bytes());
        let asset = Uuid::from_bytes(md5::compute(key).0);
        let mut message = AssetUploadRequest::default();
        message.asset_block.transaction_id = transaction;
        // Keep the inventory asset type for every body part or clothing layer.
        message.asset_block.type_ = pending.item.asset_type as i8;
        if data.len() + 100 < 1200 {
            message.asset_block.asset_data.clone_from(&data);
        }
        self.inventory_uploads.assets.insert(
            asset,
            Upload {
                transaction,
                parent: pending.item.parent,
                kind: 0,
                name: pending.item.name.clone(),
                data,
                started: Instant::now(),
                transfer: None,
                import: None,
                wearable: Some(pending),
                report: None,
                permissions: None,
            },
        );
        self.send_main(&message, true);
    }
    pub(super) fn legacy_import(&mut self, request: Uuid, mut upload: crate::inventory::upload::Request) {
        use crate::inventory::upload::{Error, Kind};
        if !matches!(upload.kind, Kind::Texture | Kind::Snapshot | Kind::Sound | Kind::Animation)
            || self.inventory_uploads.assets.len() >= 8
        {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Invalid),
                },
            );
            return;
        }
        let transaction = Uuid::new_v4();
        let mut key = [0u8; 32];
        key[..16].copy_from_slice(transaction.as_bytes());
        key[16..].copy_from_slice(self.login.secure_session_id.as_bytes());
        let asset = Uuid::from_bytes(md5::compute(key).0);
        let mut message = AssetUploadRequest::default();
        message.asset_block.transaction_id = transaction;
        message.asset_block.type_ = upload.kind.asset_type() as i8;
        let data = std::mem::take(&mut upload.data);
        if data.len() + 100 < 1200 {
            message.asset_block.asset_data.clone_from(&data);
        }
        self.inventory_uploads.assets.insert(
            asset,
            Upload {
                transaction,
                parent: upload.folder,
                kind: 255,
                name: upload.name.clone(),
                data,
                started: Instant::now(),
                transfer: None,
                import: Some((request, upload)),
                wearable: None,
                report: None,
                permissions: None,
            },
        );
        self.send_main(&message, true);
    }
    pub(super) fn create_wearable(
        &mut self,
        parent: Uuid,
        kind: u8,
        name: String,
        data: Vec<u8>,
        permissions: Option<crate::inventory::upload::Permissions>,
    ) {
        if kind >= 17 || data.is_empty() || data.len() > 64 * 1024 || self.inventory_uploads.assets.len() >= 8 {
            emit(
                self.sh,
                NetEvent::InventoryOperationFailed(
                    aurora_i18n::tr!("session-inventory-upload-creation-denied-invalid-type-size-or-upload-count").into(),
                ),
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
                import: None,
                wearable: None,
                report: None,
                permissions,
            },
        );
        self.send_main(&m, true);
    }

    /// LLFloaterReporter::uploadImage: the screenshot as a temporary texture
    /// (storeAssetData with temp_file), the report after it is stored.
    pub(super) fn upload_report_screenshot(
        &mut self,
        transaction: Uuid,
        asset: Uuid,
        data: Vec<u8>,
        report: crate::abuse_report::AbuseReport,
    ) {
        let mut m = AssetUploadRequest::default();
        m.asset_block.transaction_id = transaction;
        m.asset_block.type_ = 0;
        m.asset_block.tempfile = true;
        if data.len() + 100 < 1200 {
            m.asset_block.asset_data.clone_from(&data);
        }
        self.inventory_uploads.assets.insert(
            asset,
            Upload {
                transaction,
                parent: Uuid::nil(),
                kind: 255,
                name: String::new(),
                data,
                started: Instant::now(),
                transfer: None,
                import: None,
                wearable: None,
                report: Some(Box::new(report)),
                permissions: None,
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
            let asset_type = upload.asset_type();
            if m.xfer_id.v_file_type != i16::from(asset_type) || !field_str(&m.xfer_id.filename).is_empty() {
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
            if let Some(report) = upload.report {
                // uploadDoneCallback: no report without its screenshot
                let result = if m.asset_block.success {
                    self.send_user_report(&report, m.asset_block.uuid);
                    Ok(())
                } else {
                    Err(aurora_i18n::tr!("report-abuse-screenshot-failed").to_owned())
                };
                emit(self.sh, NetEvent::AbuseReportSent(result));
                return Ok(true);
            }
            if let Some(pending) = upload.wearable {
                if m.asset_block.success && i32::from(m.asset_block.type_) == pending.item.asset_type {
                    self.wearable_asset_ready(pending, upload.transaction, m.asset_block.uuid);
                } else {
                    emit(
                        self.sh,
                        NetEvent::WearableSaved {
                            request: pending.request,
                            result: Err(super::wearable_save::failure(
                                &pending.item,
                                super::wearable_save::Failure::Rejected,
                            )),
                        },
                    );
                }
                return Ok(true);
            }
            if let Some((request, metadata)) = upload.import {
                if m.asset_block.success && m.asset_block.type_ == metadata.kind.asset_type() as i8 {
                    self.legacy_asset_ready(request, metadata, upload.transaction, m.asset_block.uuid);
                } else {
                    emit(
                        self.sh,
                        NetEvent::AssetUploaded {
                            request,
                            result: Err(crate::inventory::upload::Error::Rejected),
                        },
                    );
                }
                return Ok(true);
            }
            if !m.asset_block.success {
                emit(
                    self.sh,
                    NetEvent::InventoryOperationFailed(
                        aurora_i18n::tr!("session-inventory-upload-the-server-rejected-the-new-clothing-asset").into(),
                    ),
                );
                return Ok(true);
            }
            let mut m = CreateInventoryItem::default();
            m.agent_data.agent_id = self.agent_id();
            m.agent_data.session_id = self.session_id();
            m.inventory_block.folder_id = upload.parent;
            m.inventory_block.callback_id = self.inventory_callback(true);
            m.inventory_block.transaction_id = upload.transaction;
            m.inventory_block.next_owner_mask = upload.permissions.map_or(0x7fff_ffff, |rights| rights.next_owner);
            if let Some(rights) = upload.permissions {
                self.remember_creation_permissions(
                    m.inventory_block.callback_id,
                    upload.parent,
                    if upload.kind < 4 { 13 } else { 5 },
                    rights,
                );
            }
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
                let removed = self.inventory_uploads.assets.remove(&asset);
                if removed.as_ref().is_some_and(|u| u.report.is_some()) {
                    emit(
                        self.sh,
                        NetEvent::AbuseReportSent(Err(aurora_i18n::tr!("report-abuse-screenshot-failed").to_owned())),
                    );
                    return Ok(true);
                }
                if let Some(pending) = removed.as_ref().and_then(|u| u.wearable.as_ref()) {
                    emit(
                        self.sh,
                        NetEvent::WearableSaved {
                            request: pending.request,
                            result: Err(super::wearable_save::failure(
                                &pending.item,
                                super::wearable_save::Failure::Interrupted,
                            )),
                        },
                    );
                    return Ok(true);
                }
                if let Some((request, _)) = removed.and_then(|u| u.import) {
                    emit(
                        self.sh,
                        NetEvent::AssetUploaded {
                            request,
                            result: Err(crate::inventory::upload::Error::Unknown),
                        },
                    );
                    return Ok(true);
                }
                emit(
                    self.sh,
                    NetEvent::InventoryOperationFailed(
                        aurora_i18n::tr!("session-inventory-upload-the-clothing-upload-was-interrupted").into(),
                    ),
                );
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn expire_inventory_uploads(&mut self) {
        let mut retry = Vec::new();
        let mut failed = false;
        let mut imports = Vec::new();
        let mut wearables = Vec::new();
        let mut reports = 0;
        self.inventory_uploads.assets.retain(|id, u| {
            if u.started.elapsed() > Duration::from_secs(60) {
                if let Some(pending) = &u.wearable {
                    wearables.push((
                        pending.request,
                        super::wearable_save::failure(&pending.item, super::wearable_save::Failure::Interrupted),
                    ));
                }
                if u.report.is_some() {
                    reports += 1;
                } else if let Some((request, _)) = &u.import {
                    imports.push(*request);
                } else if u.wearable.is_none() {
                    failed = true;
                }
                return false;
            }
            if let Some(t) = &mut u.transfer
                && t.sent.elapsed() > Duration::from_secs(3)
                && (t.packet as usize) * t.chunk < u.data.len()
            {
                if t.retries >= 3 {
                    if let Some(pending) = &u.wearable {
                        wearables.push((
                            pending.request,
                            super::wearable_save::failure(&pending.item, super::wearable_save::Failure::Interrupted),
                        ));
                    }
                    if u.report.is_some() {
                        reports += 1;
                    } else if let Some((request, _)) = &u.import {
                        imports.push(*request);
                    } else if u.wearable.is_none() {
                        failed = true;
                    }
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
        for _ in 0..reports {
            emit(
                self.sh,
                NetEvent::AbuseReportSent(Err(aurora_i18n::tr!("report-abuse-screenshot-failed").to_owned())),
            );
        }
        self.inventory_uploads.wearable_copies.retain(|_, pending| {
            if pending.started.elapsed() > Duration::from_secs(60) {
                wearables.push((
                    pending.request,
                    super::wearable_save::failure(&pending.item, super::wearable_save::Failure::Interrupted),
                ));
                false
            } else {
                true
            }
        });
        for (request, reason) in wearables {
            emit(
                self.sh,
                NetEvent::WearableSaved {
                    request,
                    result: Err(reason),
                },
            );
        }
        for request in imports {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(crate::inventory::upload::Error::Unknown),
                },
            );
        }
        if failed {
            emit(
                self.sh,
                NetEvent::InventoryOperationFailed(
                    aurora_i18n::tr!("session-inventory-upload-the-clothing-upload-timed-out-reload-inventory-before-trying").into(),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn receive<M: Msg>(socket: &UdpSocket) -> M {
        let mut bytes = [0; 8192];
        let (size, _) = tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut bytes))
            .await
            .expect("offline simulator receives a packet")
            .expect("datagram");
        parse_packet(&bytes[..size]).expect("packet").decode().expect("message")
    }

    fn dispatch<M: Msg>(session: &mut Session<'_>, from: SocketAddr, message: &M) -> bool {
        let mut bytes = Vec::new();
        aurora_msg::build_packet(message, 1, false, false, &[], &mut bytes);
        session
            .dispatch_inventory_upload(from, &parse_packet(&bytes).expect("packet"))
            .expect("upload dispatch")
    }

    #[tokio::test]
    async fn saved_wearables_transfer_their_original_asset_type_before_copying() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.expect("viewer socket"));
        sock.writable().await.expect("viewer socket ready");
        let simulator = UdpSocket::bind("127.0.0.1:0").await.expect("offline simulator");
        let main = simulator.local_addr().expect("simulator address");
        let (events, received) = crossbeam_channel::unbounded();
        let sh = super::super::event_queue_tests::shared(crate::EventSender(events));
        let mut session = super::super::event_queue_tests::two_regions(&sh, sock, main, "127.0.0.1:9".parse().expect("neighbour"));
        // Universal first: a third-party asset large enough to require Xfer.
        for kind in std::iter::once(16).chain(0..16) {
            for big in [false, true] {
                let request = Uuid::new_v4();
                let item = crate::inventory::InvItem {
                    id: Uuid::new_v4(),
                    asset_id: Uuid::new_v4(),
                    owner: session.agent_id(),
                    creator: Uuid::from_u128(99),
                    owner_mask: 0x7fff_ffff,
                    asset_type: if kind < 4 { 13 } else { 5 },
                    flags: kind,
                    ..Default::default()
                };
                let data: Vec<_> = (0..2500).map(|i| (i % 251) as u8).collect();
                session.save_wearable(request, item.clone(), data.clone(), Some("Edited copy".into()));
                let upload: AssetUploadRequest = receive(&simulator).await;
                assert_eq!(i32::from(upload.asset_block.type_), item.asset_type);
                assert!(upload.asset_block.asset_data.is_empty(), "large assets use Xfer");
                let asset = *session.inventory_uploads.assets.keys().next().expect("pending upload");
                let mut xfer = RequestXfer::default();
                xfer.xfer_id.id = 42;
                xfer.xfer_id.v_file_id = asset;
                xfer.xfer_id.v_file_type = if item.asset_type == 5 { 13 } else { 5 };
                assert!(!dispatch(&mut session, main, &xfer), "wrong asset types remain rejected");
                xfer.xfer_id.v_file_type = item.asset_type as i16;
                xfer.xfer_id.use_big_packets = big;
                assert!(dispatch(&mut session, main, &xfer), "wearable {kind} must accept its own Xfer type");
                let mut transferred = Vec::new();
                let mut index = 0;
                loop {
                    let packet: SendXferPacket = receive(&simulator).await;
                    assert_eq!(packet.xfer_id.id, 42);
                    assert_eq!(packet.xfer_id.packet & 0x7fff_ffff, index);
                    let bytes = if index == 0 {
                        assert_eq!(&packet.data_packet.data[..4], &(data.len() as i32).to_le_bytes());
                        &packet.data_packet.data[4..]
                    } else {
                        &packet.data_packet.data[..]
                    };
                    transferred.extend_from_slice(bytes);
                    let mut ack = ConfirmXferPacket::default();
                    ack.xfer_id.id = 42;
                    ack.xfer_id.packet = index;
                    assert!(dispatch(&mut session, main, &ack));
                    if packet.xfer_id.packet & 0x8000_0000 != 0 {
                        break;
                    }
                    index += 1;
                }
                assert_eq!(transferred, data);
                assert!(
                    session.inventory_uploads.wearable_copies.is_empty(),
                    "no copy before upload acknowledgement"
                );
                assert!(received.is_empty(), "no premature save result");
                let mut complete = AssetUploadComplete::default();
                complete.asset_block.uuid = asset;
                complete.asset_block.type_ = item.asset_type as i8;
                complete.asset_block.success = true;
                assert!(dispatch(&mut session, main, &complete));
                let copy: CopyInventoryItem = receive(&simulator).await;
                assert_eq!(copy.inventory_data[0].old_item_id, item.id);
                let pending = session
                    .inventory_uploads
                    .wearable_copies
                    .remove(&copy.inventory_data[0].callback_id)
                    .expect("copy waits for confirmation");
                assert_eq!(pending.request, request);
                assert_eq!(pending.item.creator, item.creator);
                assert_eq!(pending.item.owner_mask, item.owner_mask);
                assert_eq!(pending.asset, asset);
                assert_eq!(pending.transaction, upload.asset_block.transaction_id);
                assert!(session.inventory_uploads.assets.is_empty());
            }
        }
    }

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
