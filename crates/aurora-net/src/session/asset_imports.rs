//! Materials use CreateInventoryItem followed by UpdateMaterialAgentInventory,
//! as LLMaterialEditor::createInventoryItem (Firestorm, originally LGPL 2.1).
//! The callback owns its request; unrelated inventory creations cannot finish it.

use super::*;
use crate::inventory::{
    InvItem,
    operations::{Mutation, Operation},
    upload::{Error, Kind, Receipt, Request},
};

#[derive(Default)]
pub(super) struct Imports {
    creates: HashMap<u32, Pending>,
    meshes: Arc<Mutex<HashMap<Uuid, MeshOffer>>>,
    pub balance: Option<i32>,
}
struct MeshOffer {
    started: Instant,
    upload: crate::inventory::mesh_upload::Request,
    offer: Option<crate::inventory::mesh_upload::Offer>,
}
struct Pending {
    started: Instant,
    request: Uuid,
    upload: Request,
    caps: (String, String, String, String),
    legacy_asset: Option<Uuid>,
    permissions_applied: bool,
}

/// LLViewerInventoryItem::updateServer(false), including the legacy checksum.
fn permission_update(item: &InvItem, rights: crate::inventory::upload::Permissions, callback: u32) -> update_inventory_item::InventoryData {
    let uuid_sum = |id: Uuid| {
        id.as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .fold(0u32, |sum, b| sum.wrapping_add(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    };
    let group = rights.group & item.owner_mask;
    let everyone = rights.everyone & item.owner_mask;
    let crc = [
        item.id,
        item.parent,
        item.creator,
        item.owner,
        item.last_owner,
        item.group_id,
        item.asset_id,
        item.thumbnail,
    ]
    .into_iter()
    .map(uuid_sum)
    .chain([
        item.base_mask,
        item.owner_mask,
        group,
        everyone,
        item.asset_type as u32,
        item.inv_type as u32,
        item.flags,
        item.sale_price as u32,
        u32::from(item.sale_type).wrapping_mul(0x07073096),
        item.created_at as u32,
    ])
    .fold(0u32, u32::wrapping_add);
    update_inventory_item::InventoryData {
        item_id: item.id,
        folder_id: item.parent,
        callback_id: callback,
        creator_id: item.creator,
        owner_id: item.owner,
        group_id: item.group_id,
        base_mask: item.base_mask,
        owner_mask: item.owner_mask,
        group_mask: group,
        everyone_mask: everyone,
        next_owner_mask: rights.next_owner & item.owner_mask,
        group_owned: item.group_owned,
        transaction_id: Uuid::nil(),
        type_: item.asset_type as i8,
        inv_type: item.inv_type as i8,
        flags: item.flags,
        sale_type: item.sale_type,
        sale_price: item.sale_price,
        name: str_field(&item.name),
        description: str_field(&item.desc),
        creation_date: item.created_at as i32,
        crc,
    }
}

impl Session<'_> {
    pub(super) fn legacy_asset_ready(&mut self, request: Uuid, upload: Request, transaction: Uuid, asset: Uuid) {
        if upload.expected_cost > 0 && self.asset_imports.balance.is_none_or(|balance| balance < upload.expected_cost) {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Rejected),
                },
            );
            return;
        }
        // upload_done_callback: the charge precedes inventory creation and
        // carries the price the user confirmed, so the simulator can reject
        // a discrepancy. The transfer itself has already completed.
        let mut charge = MoneyTransferRequest::default();
        charge.agent_data.agent_id = self.agent_id();
        charge.agent_data.session_id = self.session_id();
        charge.money_data.source_id = self.agent_id();
        charge.money_data.amount = upload.expected_cost;
        charge.money_data.transaction_type = 1101;
        self.send_main(&charge, true);
        if let Some(balance) = &mut self.asset_imports.balance {
            *balance -= upload.expected_cost;
        }
        let callback = self.inventory_callback(false);
        let mut message = CreateInventoryItem::default();
        message.agent_data.agent_id = self.agent_id();
        message.agent_data.session_id = self.session_id();
        message.inventory_block.folder_id = upload.folder;
        message.inventory_block.callback_id = callback;
        message.inventory_block.transaction_id = transaction;
        message.inventory_block.next_owner_mask = upload.permissions.next_owner;
        message.inventory_block.type_ = upload.kind.asset_type() as i8;
        message.inventory_block.inv_type = match upload.kind {
            Kind::Texture => 0,
            Kind::Sound => 1,
            Kind::Animation => 19,
            // IT_SNAPSHOT
            Kind::Snapshot => 15,
            _ => return,
        };
        message.inventory_block.wearable_type = 255;
        message.inventory_block.name = str_field(&upload.name);
        message.inventory_block.description = str_field(&upload.description);
        self.asset_imports.creates.insert(
            callback,
            Pending {
                started: Instant::now(),
                request,
                upload,
                caps: Default::default(),
                legacy_asset: Some(asset),
                permissions_applied: false,
            },
        );
        self.send_main(&message, true);
    }
    pub(super) fn quote_mesh(&mut self, request: Uuid, upload: crate::inventory::mesh_upload::Request) {
        let caps = self.main_cap("NewFileAgentInventory").zip(self.main_cap("MeshUploadFlag"));
        let Some((cap, permission)) = caps else {
            emit(
                self.sh,
                NetEvent::MeshQuoted {
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        };
        if upload.validate().is_err() || self.asset_imports.meshes.lock().len() >= 8 {
            emit(
                self.sh,
                NetEvent::MeshQuoted {
                    request,
                    result: Err(Error::Invalid),
                },
            );
            return;
        }
        let token = Uuid::new_v4();
        self.asset_imports.meshes.lock().insert(
            token,
            MeshOffer {
                started: Instant::now(),
                upload: upload.clone(),
                offer: None,
            },
        );
        let offers = self.asset_imports.meshes.clone();
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = crate::inventory::mesh_upload::quote(&http, &cap, &permission, &upload).await;
            let result = match result {
                Ok(offer) => {
                    let quote = crate::inventory::mesh_upload::Quote {
                        token,
                        cost: offer.cost,
                        data: offer.data.clone(),
                    };
                    if let Some(pending) = offers.lock().get_mut(&token) {
                        pending.offer = Some(offer);
                        Ok(quote)
                    } else {
                        Err(Error::PriceChanged(None))
                    }
                }
                Err(error) => {
                    offers.lock().remove(&token);
                    Err(error)
                }
            };
            let _ = events.send(NetEvent::MeshQuoted { request, result });
        });
    }

    pub(super) fn discard_mesh_quote(&mut self, token: Uuid) {
        self.asset_imports.meshes.lock().remove(&token);
    }

    pub(super) fn upload_mesh(&mut self, request: Uuid, token: Uuid) {
        // Consume before spawning. Repeating the command cannot spend twice.
        let pending = self.asset_imports.meshes.lock().remove(&token);
        let caps = self.main_cap("FetchInventory2");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        tokio::spawn(async move {
            let result = if let (Some(pending), Some(fetch)) = (pending, caps) {
                if pending.started.elapsed() > Duration::from_secs(300) {
                    Err(Error::PriceChanged(None))
                } else if let Some(offer) = pending.offer {
                    crate::inventory::mesh_upload::upload(&http, offer.uploader, &fetch, owner, offer.cost, pending.upload).await
                } else {
                    Err(Error::Invalid)
                }
            } else {
                Err(Error::PriceChanged(None))
            };
            let _ = events.send(NetEvent::AssetUploaded { request, result });
        });
    }

    pub(super) fn import_asset(&mut self, request: Uuid, upload: Request) {
        if upload.kind == Kind::Mesh {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Invalid),
                },
            );
            return;
        }
        let invalid = upload.body().is_err();
        if invalid {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Invalid),
                },
            );
            return;
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        if upload.kind != Kind::Material {
            let caps = self.main_cap("NewFileAgentInventory").zip(self.main_cap("FetchInventory2"));
            if self.main_cap("NewFileAgentInventory").is_none() {
                self.legacy_import(request, upload);
                return;
            }
            tokio::spawn(async move {
                let result = if let Some((cap, fetch)) = caps {
                    crate::inventory::upload::upload(&http, &cap, &fetch, owner, upload).await
                } else {
                    Err(Error::Unavailable)
                };
                let _ = events.send(NetEvent::AssetUploaded { request, result });
            });
            return;
        }
        let caps = self
            .main_cap("UpdateMaterialAgentInventory")
            .zip(self.main_cap("FetchInventory2"))
            .zip(self.main_cap("InventoryAPIv3"))
            .zip(self.main_cap("FetchInventoryDescendents2"));
        let Some((((cap, fetch), ais), descendants)) = caps else {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        };
        if self.asset_imports.creates.len() >= 8 || upload.expected_cost != 0 {
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request,
                    result: Err(Error::Invalid),
                },
            );
            return;
        }
        let callback = self.inventory_callback(false);
        let mut message = CreateInventoryItem::default();
        message.agent_data.agent_id = owner;
        message.agent_data.session_id = self.session_id();
        message.inventory_block.folder_id = upload.folder;
        message.inventory_block.callback_id = callback;
        message.inventory_block.next_owner_mask = upload.permissions.next_owner;
        message.inventory_block.type_ = 57;
        message.inventory_block.inv_type = 26;
        message.inventory_block.wearable_type = 255;
        message.inventory_block.transaction_id = Uuid::new_v4();
        message.inventory_block.name = str_field(&upload.name);
        message.inventory_block.description = str_field(&upload.description);
        self.asset_imports.creates.insert(
            callback,
            Pending {
                started: Instant::now(),
                request,
                upload,
                caps: (cap, fetch, ais, descendants),
                legacy_asset: None,
                permissions_applied: false,
            },
        );
        self.send_main(&message, true);
    }

    pub(super) fn asset_created(&mut self, callback: u32, item: &InvItem) {
        let Some(mut pending) = self.asset_imports.creates.remove(&callback) else {
            return;
        };
        if let Some(asset) = pending.legacy_asset {
            let valid = !item.id.is_nil()
                && item.asset_id == asset
                && item.parent == pending.upload.folder
                && item.asset_type == pending.upload.kind.asset_type()
                && item.owner == self.agent_id();
            let rights = pending.upload.permissions;
            let masks_match = item.group_mask == (rights.group & item.owner_mask)
                && item.everyone_mask == (rights.everyone & item.owner_mask)
                && item.next_owner_mask == (rights.next_owner & item.owner_mask);
            if valid && !masks_match && !pending.permissions_applied {
                let callback = self.inventory_callback(false);
                let mut message = UpdateInventoryItem::default();
                message.agent_data.agent_id = self.agent_id();
                message.agent_data.session_id = self.session_id();
                message.agent_data.transaction_id = Uuid::new_v4();
                message.inventory_data.push(permission_update(item, rights, callback));
                pending.permissions_applied = true;
                self.asset_imports.creates.insert(callback, pending);
                self.send_main(&message, true);
                return;
            }
            let result = if item.asset_id == asset
                && item.parent == pending.upload.folder
                && item.asset_type == pending.upload.kind.asset_type()
                && item.owner == self.agent_id()
                && valid
                && masks_match
            {
                Ok(Receipt {
                    item: item.clone(),
                    cost: pending.upload.expected_cost,
                })
            } else {
                Err(Error::Verification)
            };
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request: pending.request,
                    result,
                },
            );
            return;
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        let item = item.clone();
        tokio::spawn(async move {
            let request = pending.request;
            let result = async {
                if item.id.is_nil() || item.parent != pending.upload.folder || item.asset_type != 57 || item.owner != owner {
                    return Err(Error::Response);
                }
                let (cap, fetch, ais, descendants) = pending.caps;
                let rights = pending.upload.permissions;
                let change = Mutation {
                    operations: vec![Operation::Patch {
                        id: item.id,
                        folder: false,
                        body: aurora_llsd::llsd_map! {
                            "name" => pending.upload.name.as_str(), "desc" => pending.upload.description.as_str(),
                            "permissions" => aurora_llsd::llsd_map! { "next_owner_mask" => rights.next_owner as i32,
                                "group_mask" => rights.group as i32, "everyone_mask" => rights.everyone as i32 }
                        },
                    }],
                    refresh: vec![item.parent],
                    ..Default::default()
                };
                crate::inventory::operations::mutate(&http, &ais, &descendants, owner, change)
                    .await
                    .map_err(|_| Error::Verification)?;
                let saved = crate::inventory::operations::save_content(&http, &cap, &fetch, owner, item.id, 57, pending.upload.data)
                    .await
                    .map_err(|_| Error::Unknown)?;
                if saved.id != item.id
                    || saved.parent != item.parent
                    || saved.owner != owner
                    || saved.asset_type != 57
                    || saved.asset_id.is_nil()
                {
                    return Err(Error::Verification);
                }
                Ok(Receipt { item: saved, cost: 0 })
            }
            .await;
            let _ = events.send(NetEvent::AssetUploaded { request, result });
        });
    }

    pub(super) fn expire_asset_imports(&mut self) {
        self.asset_imports
            .meshes
            .lock()
            .retain(|_, pending| pending.started.elapsed() < Duration::from_secs(300));
        self.asset_imports.creates.retain(|_, pending| {
            if pending.started.elapsed() < Duration::from_secs(60) {
                return true;
            }
            emit(
                self.sh,
                NetEvent::AssetUploaded {
                    request: pending.request,
                    result: Err(Error::Unknown),
                },
            );
            false
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_rights_update_preserves_identity_and_clamps_to_owned_permissions() {
        let item = InvItem {
            id: Uuid::from_u128(1),
            parent: Uuid::from_u128(2),
            asset_id: Uuid::from_u128(3),
            creator: Uuid::from_u128(4),
            owner: Uuid::from_u128(5),
            asset_type: 20,
            inv_type: 19,
            name: "Motion".into(),
            desc: "Test".into(),
            owner_mask: 0x0008a000,
            base_mask: 0x7fff_ffff,
            flags: 7,
            created_at: 12,
            ..Default::default()
        };
        let update = permission_update(
            &item,
            crate::inventory::upload::Permissions {
                next_owner: 0x7fff_ffff,
                group: 0x7fff_ffff,
                everyone: 0x7fff_ffff,
            },
            9,
        );
        assert_eq!(update.item_id, item.id);
        assert_eq!(update.folder_id, item.parent);
        assert_eq!(update.callback_id, 9);
        assert_eq!(update.transaction_id, Uuid::nil());
        assert_eq!(update.group_mask, item.owner_mask);
        assert_eq!(update.everyone_mask, item.owner_mask);
        assert_eq!(update.next_owner_mask, item.owner_mask);
        assert_eq!(update.type_, 20);
        assert_eq!(update.inv_type, 19);
        assert_eq!(field_str(&update.name), "Motion");
        assert_eq!(field_str(&update.description), "Test");
        // UUID bytes use little-endian words; the next-owner mask is omitted
        // by the protocol checksum, as LLPermissions::getCRC32 specifies.
        assert_eq!(
            update.crc,
            0x0f00_0000u32
                .wrapping_add(item.base_mask)
                .wrapping_add(item.owner_mask * 3)
                .wrapping_add(20 + 19 + 7 + 12)
        );
    }
}
