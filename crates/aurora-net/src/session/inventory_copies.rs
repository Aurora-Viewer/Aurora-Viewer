//! copy_inventory_item and copy_inventory_category_content callbacks
//! (Firestorm indra/newview/llviewerinventory.cpp, llinventoryfunctions.cpp,
//! originally LGPL 2.1). Folder creation alone never completes a folder copy.

use super::*;
use crate::inventory::{
    InvItem,
    operations::{CopyItem, Reply},
};

const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Pending {
    batches: HashMap<Uuid, Batch>,
    callbacks: HashMap<u32, (Uuid, CopyItem)>,
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
        new_name: str_field(&copy.name),
    }];
    message
}

impl Session<'_> {
    pub(super) fn start_inventory_copies(&mut self, request: Uuid, reply: Reply) {
        if reply.copies.is_empty() || self.main_cap("FetchInventoryDescendents2").is_none() {
            emit(
                self.sh,
                NetEvent::InventoryEdited {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into()),
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
        for (callback, copy) in copies {
            self.send_main(&message(self.agent_id(), self.session_id(), callback, &copy), true);
        }
    }

    pub(super) fn finish_inventory_copies(&mut self, completion: Completion) {
        for callback in completion.callbacks {
            self.inventory_callbacks.remove(&callback);
        }
        let request = completion.request;
        let result = completion.result;
        let Some(fetch) = self.main_cap("FetchInventoryDescendents2") else {
            emit(
                self.sh,
                NetEvent::InventoryEdited {
                    request,
                    result: Err(aurora_i18n::tr!("inventory-copy-unconfirmed").into()),
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        tokio::spawn(async move {
            let result = match result {
                Ok((reply, items)) => {
                    log::info!("inventory copy: {} item copies acknowledged; reading folders back", items.len());
                    crate::inventory::operations::complete_copies(&http, &fetch, owner, reply, &items).await
                }
                Err(reason) => Err(reason),
            };
            if result.is_err() {
                log::warn!("inventory copy was not fully confirmed");
            }
            let _ = events.send(NetEvent::InventoryEdited { request, result });
        });
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
    }
}
