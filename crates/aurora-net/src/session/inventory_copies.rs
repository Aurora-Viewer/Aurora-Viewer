//! copy_inventory_item and copy_inventory_category_content callbacks
//! (Firestorm indra/newview/llviewerinventory.cpp, llinventoryfunctions.cpp,
//! originally LGPL 2.1). Folder creation alone never completes a folder copy.

use super::*;
use crate::inventory::{
    InvItem,
    operations::{CopyItem, Phase, Progress, Reply},
};

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Pending {
    batches: HashMap<Uuid, Batch>,
    callbacks: HashMap<u32, (Uuid, CopyItem)>,
    /// Single copies by callback: their request and when they were sent.
    singles: HashMap<u32, (Uuid, Instant)>,
}

struct Batch {
    reply: Reply,
    callbacks: Vec<u32>,
    received: Vec<InvItem>,
    started: Instant,
}

pub(super) struct Completion {
    request: Uuid,
    callbacks: Vec<u32>,
    result: Result<(Reply, Vec<InvItem>), String>,
}

impl Pending {
    pub(super) fn request_for(&self, callback: u32) -> Option<Uuid> {
        self.callbacks.get(&callback).map(|(request, _)| *request)
    }

    fn progress(&self, request: Uuid) -> Option<Progress> {
        let batch = self.batches.get(&request)?;
        Some(Progress {
            phase: Phase::Copying,
            completed: batch.received.len(),
            total: batch.callbacks.len(),
        })
    }
    fn begin(&mut self, request: Uuid, mut reply: Reply, copies: Vec<(u32, CopyItem)>) {
        reply.copies.clear();
        let callbacks = copies.iter().map(|(cb, _)| *cb).collect();
        self.batches.insert(
            request,
            Batch {
                reply,
                callbacks,
                received: Vec::new(),
                started: Instant::now(),
            },
        );
        for (callback, copy) in copies {
            self.callbacks.insert(callback, (request, copy));
        }
    }

    pub(super) fn accept(&mut self, callback: u32, item: &InvItem) -> Option<Completion> {
        let (request, copy) = self.callbacks.remove(&callback)?;
        let batch = self.batches.get_mut(&request)?;
        if item.id.is_nil() || item.id == copy.item || item.parent != copy.parent || batch.received.iter().any(|old| old.id == item.id) {
            return self.complete(request, Some(aurora_i18n::tr!("inventory-copy-unconfirmed").into()));
        }
        batch.received.push(item.clone());
        if batch.received.len() == batch.callbacks.len() {
            self.complete(request, None)
        } else {
            None
        }
    }

    fn complete(&mut self, request: Uuid, error: Option<String>) -> Option<Completion> {
        let batch = self.batches.remove(&request)?;
        for callback in &batch.callbacks {
            self.callbacks.remove(callback);
        }
        Some(Completion {
            request,
            callbacks: batch.callbacks,
            result: error.map_or_else(|| Ok((batch.reply, batch.received)), Err),
        })
    }

    /// The new item of a single copy (`NetCommand::CopyInventoryItem`).
    fn accept_single(&mut self, callback: u32, item: &InvItem) -> Option<(Uuid, Result<InvItem, String>)> {
        let (request, _) = self.singles.remove(&callback)?;
        Some((
            request,
            if item.id.is_nil() {
                Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into())
            } else {
                Ok(item.clone())
            },
        ))
    }

    /// Single copies the server never answered within the timeout.
    pub(super) fn expire_singles(&mut self, now: Instant) -> Vec<Uuid> {
        self.singles
            .extract_if(|_, (_, sent)| now.duration_since(*sent) >= TIMEOUT)
            .map(|(_, (request, _))| request)
            .collect()
    }

    pub(super) fn expire(&mut self, now: Instant) -> Vec<Completion> {
        let expired: Vec<_> = self
            .batches
            .iter()
            .filter(|(_, batch)| now.duration_since(batch.started) >= TIMEOUT)
            .map(|(request, _)| *request)
            .collect();
        expired
            .into_iter()
            .filter_map(|request| self.complete(request, Some(aurora_i18n::tr!("inventory-copy-unconfirmed").into())))
            .collect()
    }
}

pub(super) fn message(agent: Uuid, session: Uuid, callback: u32, copy: &CopyItem) -> CopyInventoryItem {
    let mut message = CopyInventoryItem::default();
    message.agent_data.agent_id = agent;
    message.agent_data.session_id = session;
    message.inventory_data = vec![copy_inventory_item::InventoryData {
        callback_id: callback,
        old_agent_id: copy.owner,
        old_item_id: copy.item,
        new_folder_id: copy.parent,
        // LLTemplateMessageBuilder::addString(std::string): an empty name
        // has zero length, unlike addString(const char*) which adds a NUL.
        new_name: if copy.name.is_empty() { Vec::new() } else { str_field(&copy.name) },
    }];
    message
}

impl Session<'_> {
    pub(super) fn accept_inventory_copy(&mut self, callback: u32, item: &InvItem) {
        if self.accept_wearable_copy(callback, item) {
            return;
        }
        if let Some((request, result)) = self.inventory_copies.accept_single(callback, item) {
            emit(self.sh, NetEvent::InventoryItemCopied { request, result });
            return;
        }
        let request = self.inventory_copies.request_for(callback);
        if let Some(completion) = self.inventory_copies.accept(callback, item) {
            self.finish_inventory_copies(completion);
        } else if let Some(request) = request
            && let Some(progress) = self.inventory_copies.progress(request)
        {
            emit(self.sh, NetEvent::InventoryProgress { request, progress });
        }
    }
    /// copy_inventory_item with a callback (llviewerinventory.cpp): one
    /// CopyInventoryItem, its new item reported by `InventoryItemCopied`.
    pub(super) fn copy_inventory_item(&mut self, request: Uuid, copy: CopyItem) {
        if self.main.is_none() || copy.item.is_nil() || copy.owner.is_nil() {
            emit(
                self.sh,
                NetEvent::InventoryItemCopied {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into()),
                },
            );
            return;
        }
        let callback = self.inventory_callback(false);
        self.inventory_copies.singles.insert(callback, (request, Instant::now()));
        self.send_main(&message(self.agent_id(), self.session_id(), callback, &copy), true);
    }

    pub(super) fn start_inventory_copies(&mut self, request: Uuid, reply: Reply) {
        if reply.copies.is_empty() || self.main.is_none() || self.logout_started.is_some() {
            emit(
                self.sh,
                NetEvent::InventoryEdited {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into()),
                },
            );
            return;
        }
        // Invalid references cannot become valid by waiting for a callback.
        if reply
            .copies
            .iter()
            .any(|c| c.item.is_nil() || c.owner.is_nil() || c.parent.is_nil())
        {
            log::warn!("inventory copy rejected before sending: incomplete item references");
            emit(
                self.sh,
                NetEvent::InventoryEdited {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-owner-unknown").into()),
                },
            );
            return;
        }
        let copies: Vec<_> = reply
            .copies
            .iter()
            .map(|copy| (self.inventory_callback(false), copy.clone()))
            .collect();
        log::info!("inventory copy: sending {} item copies", copies.len());
        self.inventory_copies.begin(request, reply, copies.clone());
        emit(
            self.sh,
            NetEvent::InventoryProgress {
                request,
                progress: Progress {
                    phase: Phase::Copying,
                    completed: 0,
                    total: copies.len(),
                },
            },
        );
        for (callback, copy) in copies {
            self.send_main(&message(self.agent_id(), self.session_id(), callback, &copy), true);
        }
    }

    pub(super) fn finish_inventory_copies(&mut self, completion: Completion) {
        for callback in completion.callbacks {
            self.inventory_callbacks.remove(&callback);
        }
        let request = completion.request;
        let result = completion.result.map(|(mut reply, items)| {
            // Firestorm updates the model from the server's items and notifies
            // observers. A second folder snapshot is not a completion barrier.
            log::info!("inventory copy: {} items confirmed by server updates", items.len());
            emit(
                self.sh,
                NetEvent::InventoryUpdated {
                    folders: Vec::new(),
                    items,
                },
            );
            reply.contents.clear();
            reply.folders.clear();
            reply
        });
        if result.is_err() {
            log::warn!("inventory copy was not fully confirmed");
        }
        emit(self.sh, NetEvent::InventoryEdited { request, result });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn reply() -> Reply {
        Reply {
            contents: Vec::new(),
            folders: Vec::new(),
            moves_queued: false,
            refresh: Vec::new(),
            copies: Vec::new(),
            removed: Vec::new(),
            created: HashMap::from([(id(10), id(100))]),
        }
    }
    fn copy(item: u128) -> CopyItem {
        CopyItem {
            item: id(item),
            owner: id(2),
            parent: id(100),
            name: String::new(),
        }
    }
    fn copied(n: u128) -> InvItem {
        InvItem {
            id: id(n),
            parent: id(100),
            ..Default::default()
        }
    }

    #[test]
    fn completion_waits_for_every_copy_and_ignores_duplicate_or_foreign_callbacks() {
        let mut pending = Pending::default();
        pending.begin(id(1), reply(), vec![(41, copy(3)), (42, copy(4))]);
        assert!(pending.accept(42, &copied(104)).is_none());
        assert!(pending.accept(42, &copied(104)).is_none());
        assert!(pending.accept(99, &copied(999)).is_none());
        let done = pending.accept(41, &copied(103)).expect("all copies acknowledged");
        let (reply, items) = done.result.expect("confirmed callbacks");
        assert_eq!(items.len(), 2);
        assert_eq!(reply.created[&id(10)], id(100));
        assert!(reply.copies.is_empty());
        assert!(pending.callbacks.is_empty() && pending.batches.is_empty());
    }

    #[test]
    fn missing_or_invalid_copy_never_completes_successfully() {
        for invalid in [
            InvItem {
                parent: id(99),
                ..copied(103)
            },
            copied(3),
        ] {
            let mut pending = Pending::default();
            pending.begin(id(1), reply(), vec![(41, copy(3)), (42, copy(4))]);
            let done = pending.accept(41, &invalid).expect("invalid copy reported");
            assert!(done.result.is_err());
            assert_eq!(done.callbacks.len(), 2);
            assert!(pending.callbacks.is_empty() && pending.batches.is_empty());
        }
        let mut pending = Pending::default();
        pending.begin(id(1), reply(), vec![(41, copy(3)), (42, copy(4))]);
        pending.accept(41, &copied(103));
        let expired = pending.expire(Instant::now() + TIMEOUT);
        assert_eq!(expired.len(), 1);
        assert!(expired[0].result.is_err());
        assert!(pending.callbacks.is_empty() && pending.batches.is_empty());
    }

    #[test]
    fn a_single_copy_reports_its_new_item_once_or_expires() {
        let mut pending = Pending::default();
        pending.singles.insert(41, (id(1), Instant::now()));
        assert!(pending.accept_single(42, &copied(104)).is_none(), "foreign callback");
        let (request, result) = pending.accept_single(41, &copied(103)).expect("copied");
        assert_eq!(request, id(1));
        assert_eq!(result.expect("new item").id, id(103));
        assert!(pending.accept_single(41, &copied(103)).is_none(), "reported once");
        pending.singles.insert(43, (id(2), Instant::now()));
        let (_, result) = pending.accept_single(43, &InvItem::default()).expect("answered");
        assert!(result.is_err(), "a nil item is no copy");
        pending.singles.insert(44, (id(3), Instant::now()));
        assert!(pending.expire_singles(Instant::now()).is_empty());
        assert_eq!(pending.expire_singles(Instant::now() + TIMEOUT), vec![id(3)]);
        assert!(pending.singles.is_empty());
    }

    #[tokio::test]
    async fn copies_use_firestorms_reliable_udp_message_and_server_assigned_folder() {
        use aurora_msg::packet::PacketFlags;
        let receiver = UdpSocket::bind("127.0.0.1:0").await.expect("receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("sender");
        socket.writable().await.expect("writable");
        let mut circuit = Circuit::new(receiver.local_addr().expect("address"));
        circuit.send(&socket, &NetStats::default(), &message(id(2), id(5), 41, &copy(3)), true);
        let mut bytes = [0; 2048];
        let (length, _) = receiver.recv_from(&mut bytes).await.expect("wire packet");
        let packet = parse_packet(&bytes[..length]).expect("packet");
        assert_ne!(packet.flags & PacketFlags::RELIABLE, 0);
        let message: CopyInventoryItem = packet.decode().expect("copy message");
        assert_eq!(message.agent_data.agent_id, id(2));
        assert_eq!(message.agent_data.session_id, id(5));
        assert_eq!(message.inventory_data[0].callback_id, 41);
        assert_eq!(message.inventory_data[0].old_agent_id, id(2));
        assert_eq!(message.inventory_data[0].old_item_id, id(3));
        assert_eq!(message.inventory_data[0].new_folder_id, id(100));
        assert_eq!(field_str(&message.inventory_data[0].new_name), "");
        assert!(
            message.inventory_data[0].new_name.is_empty(),
            "Firestorm's empty std::string has zero wire length"
        );
    }
}
