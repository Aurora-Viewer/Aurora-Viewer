//! Inventory changes over the circuit on a grid without AIS: the UDP paths of
//! llviewerinventory.cpp (`[UDP-Msg]`, the Legacy Viewer's indra/newview, originally
//! LGPL 2.1). OpenSim has no `InventoryAPIv3`, and takes these messages in
//! LLClientView.cs. As with moves, the HTTP tasks hand them to the session
//! that owns the reliable circuit.

use super::Session;
use crate::inventory::operations::UdpEdit;
use aurora_msg::msgs::{
    CreateInventoryFolder, LinkInventoryItem, PurgeInventoryDescendents, RemoveInventoryFolder, RemoveInventoryItem, UpdateInventoryFolder,
    UpdateInventoryItem, remove_inventory_folder, remove_inventory_item, update_inventory_folder,
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// LLAssetType::AT_LINK and AT_LINK_FOLDER.
const LINK: i8 = 24;
const LINK_FOLDER: i8 = 25;

pub(super) struct Request {
    edit: UdpEdit,
    reply: oneshot::Sender<Result<(), String>>,
}

pub(super) async fn submit(tx: mpsc::UnboundedSender<Request>, edit: UdpEdit) -> Result<(), String> {
    let (reply, result) = oneshot::channel();
    tx.send(Request { edit, reply })
        .map_err(|_| aurora_i18n::tr!("inventory-move-connection-unavailable").to_owned())?;
    result
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-move-connection-unavailable").to_owned())?
}

impl Session<'_> {
    pub(super) fn inventory_udp(&mut self, request: Request) {
        let result = self.send_inventory_edit(request.edit);
        let _ = request.reply.send(result);
    }

    fn send_inventory_edit(&mut self, edit: UdpEdit) -> Result<(), String> {
        if self.main.is_none() || self.logout_started.is_some() {
            return Err(aurora_i18n::tr!("inventory-move-connection-unavailable").into());
        }
        let agent = self.agent_id();
        let session = self.session_id();
        match edit {
            UdpEdit::CreateFolder {
                id,
                parent,
                name,
                type_default,
            } => {
                let mut message = CreateInventoryFolder::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.folder_data.folder_id = id;
                message.folder_data.parent_id = parent;
                message.folder_data.type_ = type_default;
                message.folder_data.name = aurora_msg::str_field(&name);
                self.send_main(&message, true);
            }
            UdpEdit::UpdateFolder {
                id,
                parent,
                name,
                type_default,
            } => {
                let mut message = UpdateInventoryFolder::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.folder_data.push(update_inventory_folder::FolderData {
                    folder_id: id,
                    parent_id: parent,
                    type_: type_default,
                    name: aurora_msg::str_field(&name),
                });
                self.send_main(&message, true);
            }
            UdpEdit::UpdateItem(item) => {
                let callback = self.inventory_callback(false);
                let mut message = UpdateInventoryItem::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.agent_data.transaction_id = Uuid::new_v4();
                let rights = crate::inventory::upload::Permissions {
                    next_owner: item.next_owner_mask,
                    group: item.group_mask,
                    everyone: item.everyone_mask,
                };
                message
                    .inventory_data
                    .push(super::asset_imports::permission_update(&item, rights, callback));
                self.send_main(&message, true);
            }
            UdpEdit::Link {
                parent,
                target,
                name,
                desc,
                folder,
                inv_type,
            } => {
                let mut message = LinkInventoryItem::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.inventory_block.callback_id = self.inventory_callback(false);
                message.inventory_block.folder_id = parent;
                message.inventory_block.transaction_id = Uuid::new_v4();
                message.inventory_block.old_item_id = target;
                message.inventory_block.type_ = if folder { LINK_FOLDER } else { LINK };
                message.inventory_block.inv_type = inv_type;
                message.inventory_block.name = aurora_msg::str_field(&name);
                message.inventory_block.description = aurora_msg::str_field(&desc);
                self.send_main(&message, true);
            }
            UdpEdit::RemoveItem(id) => {
                let mut message = RemoveInventoryItem::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.inventory_data.push(remove_inventory_item::InventoryData { item_id: id });
                self.send_main(&message, true);
            }
            UdpEdit::RemoveFolder(id) => {
                let mut message = RemoveInventoryFolder::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.folder_data.push(remove_inventory_folder::FolderData { folder_id: id });
                self.send_main(&message, true);
            }
            UdpEdit::Purge(id) => {
                let mut message = PurgeInventoryDescendents::default();
                message.agent_data.agent_id = agent;
                message.agent_data.session_id = session;
                message.inventory_data.folder_id = id;
                self.send_main(&message, true);
            }
        }
        Ok(())
    }
}
