//! LLViewerInventory{Item,Category}::updateParentOnServer, Firestorm
//! indra/newview/llviewerinventory.cpp (originally LGPL 2.1).
//! HTTP mutation tasks hand moves to the session owning the reliable circuit.

use super::Session;
use crate::circuit::Circuit;
use crate::inventory::operations::InventoryMove;
use crate::stats::NetStats;
use aurora_msg::msgs::{MoveInventoryFolder, MoveInventoryItem, move_inventory_folder, move_inventory_item};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

pub(super) struct Request {
    movement: InventoryMove,
    reply: oneshot::Sender<Result<(), String>>,
}

pub(super) async fn submit(tx: mpsc::UnboundedSender<Request>, movement: InventoryMove) -> Result<(), String> {
    let (reply, result) = oneshot::channel();
    tx.send(Request { movement, reply })
        .map_err(|_| aurora_i18n::tr!("inventory-move-connection-unavailable").to_owned())?;
    result
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-move-connection-unavailable").to_owned())?
}

impl Session<'_> {
    pub(super) fn inventory_move(&mut self, request: Request) {
        let result = self.queue_inventory_move(request.movement);
        let _ = request.reply.send(result);
    }

    pub(super) fn queue_inventory_move(&mut self, movement: InventoryMove) -> Result<(), String> {
        let agent = self.agent_id();
        let session = self.session_id();
        match self.main.and_then(|addr| self.sims.get_mut(&addr)) {
            Some(sim) if self.logout_started.is_none() => {
                send(&mut sim.circuit, &self.sock, &self.sh.stats, agent, session, movement);
                super::emit(
                    self.sh,
                    super::NetEvent::InventoryMoved {
                        id: movement.id,
                        parent: movement.parent,
                        folder: movement.folder,
                    },
                );
                Ok(())
            }
            _ => Err(aurora_i18n::tr!("inventory-move-connection-unavailable").into()),
        }
    }
}

fn send(circuit: &mut Circuit, socket: &UdpSocket, stats: &NetStats, agent: Uuid, session: Uuid, movement: InventoryMove) -> u32 {
    if movement.folder {
        let mut message = MoveInventoryFolder::default();
        message.agent_data.agent_id = agent;
        message.agent_data.session_id = session;
        message.agent_data.stamp = false;
        message.inventory_data.push(move_inventory_folder::InventoryData {
            folder_id: movement.id,
            parent_id: movement.parent,
        });
        circuit.send(socket, stats, &message, true)
    } else {
        let mut message = MoveInventoryItem::default();
        message.agent_data.agent_id = agent;
        message.agent_data.session_id = session;
        message.agent_data.stamp = false;
        message.inventory_data.push(move_inventory_item::InventoryData {
            item_id: movement.id,
            folder_id: movement.parent,
            new_name: Vec::new(),
        });
        circuit.send(socket, stats, &message, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_msg::packet::PacketFlags;
    use aurora_msg::{Msg, parse_packet};

    #[tokio::test]
    async fn item_and_folder_moves_use_firestorms_reliable_wire_messages() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.expect("local receiver");
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("local sender");
        socket.writable().await.expect("writable socket");
        let mut circuit = Circuit::new(receiver.local_addr().expect("address"));
        let stats = NetStats::default();
        let agent = Uuid::from_u128(1);
        let session = Uuid::from_u128(2);
        let mut wire = [0u8; 512];
        for folder in [false, true] {
            let movement = InventoryMove {
                id: Uuid::from_u128(3),
                parent: Uuid::from_u128(4),
                folder,
            };
            let seq = send(&mut circuit, &socket, &stats, agent, session, movement);
            assert!(circuit.is_unacked(seq), "reliable circuit tracks retransmission");
            let (size, _) = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv_from(&mut wire))
                .await
                .expect("packet timeout")
                .expect("packet");
            let packet = parse_packet(&wire[..size]).expect("packet");
            assert_ne!(packet.flags & PacketFlags::RELIABLE, 0);
            if folder {
                assert_eq!(packet.id, MoveInventoryFolder::ID);
                let message: MoveInventoryFolder = packet.decode().expect("folder move");
                assert_eq!(message.agent_data.agent_id, agent);
                assert_eq!(message.agent_data.session_id, session);
                assert!(!message.agent_data.stamp);
                assert_eq!(message.inventory_data.len(), 1);
                assert_eq!(message.inventory_data[0].folder_id, movement.id);
                assert_eq!(message.inventory_data[0].parent_id, movement.parent);
            } else {
                assert_eq!(packet.id, MoveInventoryItem::ID);
                let message: MoveInventoryItem = packet.decode().expect("item move");
                assert_eq!(message.agent_data.agent_id, agent);
                assert_eq!(message.agent_data.session_id, session);
                assert!(!message.agent_data.stamp);
                assert_eq!(message.inventory_data.len(), 1);
                assert_eq!(message.inventory_data[0].item_id, movement.id);
                assert_eq!(message.inventory_data[0].folder_id, movement.parent);
                assert!(
                    message.inventory_data[0].new_name.is_empty(),
                    "Firestorm sends a zero-length NewName"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_closed_session_does_not_report_the_move_as_sent() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        assert!(
            submit(
                tx,
                InventoryMove {
                    id: Uuid::from_u128(3),
                    parent: Uuid::from_u128(4),
                    folder: false
                }
            )
            .await
            .is_err()
        );
    }
}
