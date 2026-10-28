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
    creation_permissions: HashMap<u32, CreationPermissions>,
    creates: HashMap<u32, Pending>,
    /// New environment settings items waiting for their inventory callback.
    settings: HashMap<u32, SettingsCreate>,
    meshes: Arc<Mutex<HashMap<Uuid, MeshOffer>>>,
    pub balance: Option<i32>,
}

struct CreationPermissions {
    started: Instant,
    parent: Uuid,
    asset_type: i32,
    rights: crate::inventory::upload::Permissions,
}

impl CreationPermissions {
    fn update(&self, item: &InvItem, owner: Uuid) -> Option<update_inventory_item::InventoryData> {
        if item.id.is_nil() || item.parent != self.parent || item.asset_type != self.asset_type || item.owner != owner {
            return None;
        }
        let update = permission_update(item, self.rights, 0);
        ((item.next_owner_mask, item.group_mask, item.everyone_mask) != (update.next_owner_mask, update.group_mask, update.everyone_mask))
            .then_some(update)
    }
}
struct MeshOffer {
    started: Instant,
    upload: crate::inventory::mesh_upload::Request,
    offer: Option<crate::inventory::mesh_upload::Offer>,
}

impl MeshOffer {
    fn expired(&self) -> bool {
        let seconds = if self.offer.is_some() {
            300
        } else {
            u64::from(self.upload.timeout_secs) + 40
        };
        self.started.elapsed() > Duration::from_secs(seconds)
    }

    fn quoted(&mut self, offer: crate::inventory::mesh_upload::Offer) {
        // A slow fee transfer must not consume the user's confirmation time.
        self.started = Instant::now();
        self.offer = Some(offer);
    }
}
/// LLSettingsVOBase::createInventoryItem: the item first, its asset once the
/// callback names it (onInventoryItemCreated → updateInventoryItem).
struct SettingsCreate {
    started: Instant,
    request: Uuid,
    folder: Uuid,
    data: Vec<u8>,
    /// UpdateSettingsAgentInventory and FetchInventory2.
    caps: (String, String),
    asset_type: i32,
}

/// PERM_COPY (llpermissionsflags.h).
const PERM_COPY: u32 = 0x8000;
/// LLFloaterPerms::getNextOwnerPerms("Settings") with Firestorm's defaults:
/// PERM_MOVE, plus SettingsNextOwnerModify and SettingsNextOwnerTransfer
/// (on); SettingsNextOwnerCopy is not defined, so it reads as off.
const SETTINGS_NEXT_OWNER: u32 = 0x0008_0000 | 0x4000 | 0x2000;

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
    /// LLInventoryCallback creation callbacks apply group/everyone rights only
    /// after the server has named the new, owned inventory item.
    pub(super) fn remember_creation_permissions(
        &mut self,
        callback: u32,
        parent: Uuid,
        asset_type: i32,
        rights: crate::inventory::upload::Permissions,
    ) {
        self.asset_imports.creation_permissions.insert(
            callback,
            CreationPermissions {
                started: Instant::now(),
                parent,
                asset_type,
                rights,
            },
        );
    }

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
                        pending.quoted(offer);
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
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        tokio::spawn(async move {
            let result = if let Some(pending) = pending {
                if pending.expired() {
                    Err(Error::PriceChanged(None))
                } else if let Some(offer) = pending.offer {
                    crate::inventory::mesh_upload::upload(&http, offer.uploader, owner, offer.cost, pending.upload).await
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

    /// LLSettingsVOBase::createInventoryItem (create_inventory_settings): a
    /// new sky, water or day cycle item in `parent`, its asset uploaded once
    /// the simulator has created it.
    pub(super) fn create_settings_item(
        &mut self,
        request: Uuid,
        parent: Uuid,
        name: String,
        subtype: u8,
        next_owner_mask: Option<u32>,
        data: Vec<u8>,
    ) {
        let fail = |sh: &Shared, reason: &str| {
            emit(
                sh,
                NetEvent::InventoryContentSaved {
                    request,
                    item: Uuid::nil(),
                    result: Err(reason.to_owned()),
                },
            );
        };
        // isInventoryEnabled: both settings capabilities ("SettingsUnsuported")
        let caps = self
            .main_cap("UpdateSettingsAgentInventory")
            .filter(|_| self.main_cap("UpdateSettingsTaskInventory").is_some())
            .zip(self.main_cap("FetchInventory2"));
        let Some(caps) = caps else {
            fail(self.sh, aurora_i18n::tr!("session-saving-is-unavailable-in-this-region"));
            return;
        };
        if self.asset_imports.settings.len() >= 8 || subtype > 2 {
            fail(self.sh, aurora_i18n::tr!("session-saving-is-unavailable-in-this-region"));
            return;
        }
        let callback = self.inventory_callback(false);
        let mut message = CreateInventoryItem::default();
        message.agent_data.agent_id = self.agent_id();
        message.agent_data.session_id = self.session_id();
        message.inventory_block.folder_id = parent;
        message.inventory_block.callback_id = callback;
        message.inventory_block.transaction_id = Uuid::new_v4();
        message.inventory_block.next_owner_mask = next_owner_mask.unwrap_or(SETTINGS_NEXT_OWNER);
        // AT_SETTINGS, IT_SETTINGS; the settings type rides in the
        // wearable type field (create_inventory_settings)
        message.inventory_block.type_ = 56;
        message.inventory_block.inv_type = 25;
        message.inventory_block.wearable_type = subtype;
        message.inventory_block.name = str_field(&name);
        message.inventory_block.description = str_field("");
        self.asset_imports.settings.insert(
            callback,
            SettingsCreate {
                started: Instant::now(),
                request,
                folder: parent,
                data,
                caps,
                asset_type: 56,
            },
        );
        self.send_main(&message, true);
    }

    /// LLPreviewNotecard::onSaveComplete / create_inventory_item: use the
    /// server's callback id before uploading, exactly as for settings items.
    pub(super) fn create_notecard_item(
        &mut self,
        request: Uuid,
        parent: Uuid,
        name: String,
        data: Vec<u8>,
        permissions: Option<crate::inventory::upload::Permissions>,
    ) {
        let caps = self.main_cap("UpdateNotecardAgentInventory").zip(self.main_cap("FetchInventory2"));
        let valid =
            !parent.is_nil() && !name.trim().is_empty() && name.len() <= 255 && crate::inventory::notecard::Notecard::decode(&data).is_ok();
        let Some(caps) = caps.filter(|_| valid && self.asset_imports.settings.len() < 8) else {
            emit(
                self.sh,
                NetEvent::InventoryContentSaved {
                    request,
                    item: Uuid::nil(),
                    result: Err(aurora_i18n::tr!("session-saving-is-unavailable-in-this-region").into()),
                },
            );
            return;
        };
        let callback = self.inventory_callback(false);
        let mut message = CreateInventoryItem::default();
        message.agent_data.agent_id = self.agent_id();
        message.agent_data.session_id = self.session_id();
        message.inventory_block.folder_id = parent;
        message.inventory_block.callback_id = callback;
        // create_new_item (indra/newview/llviewerinventory.cpp) leaves the
        // transaction null: this new note has no previously uploaded asset.
        message.inventory_block.next_owner_mask = permissions.map_or(0x7fff_ffff, |rights| rights.next_owner);
        if let Some(rights) = permissions {
            self.remember_creation_permissions(callback, parent, 7, rights);
        }
        message.inventory_block.type_ = 7;
        message.inventory_block.inv_type = 7;
        message.inventory_block.wearable_type = 255;
        message.inventory_block.name = str_field(&name);
        self.asset_imports.settings.insert(
            callback,
            SettingsCreate {
                started: Instant::now(),
                request,
                folder: parent,
                data,
                caps,
                asset_type: 7,
            },
        );
        self.send_main(&message, true);
    }

    /// onInventoryItemCreated: everyone may copy the new settings item, then its asset
    /// goes up (updateInventoryItem).
    fn settings_created(&mut self, pending: SettingsCreate, item: &InvItem) {
        let owner = self.agent_id();
        if item.id.is_nil() || item.asset_type != pending.asset_type || item.parent != pending.folder || item.owner != owner {
            emit(
                self.sh,
                NetEvent::InventoryContentSaved {
                    request: pending.request,
                    item: item.id,
                    result: Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save").into()),
                },
            );
            return;
        }
        if pending.asset_type == 56 && item.everyone_mask != PERM_COPY {
            let callback = self.inventory_callback(false);
            let mut message = UpdateInventoryItem::default();
            message.agent_data.agent_id = owner;
            message.agent_data.session_id = self.session_id();
            message.agent_data.transaction_id = Uuid::new_v4();
            let rights = crate::inventory::upload::Permissions {
                next_owner: item.next_owner_mask,
                group: item.group_mask,
                everyone: PERM_COPY,
            };
            message.inventory_data.push(permission_update(item, rights, callback));
            self.send_main(&message, true);
        }
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let id = item.id;
        tokio::spawn(async move {
            let (cap, fetch) = pending.caps;
            let result = crate::inventory::operations::save_content(&http, &cap, &fetch, owner, id, pending.asset_type, pending.data).await;
            let _ = events.send(NetEvent::InventoryContentSaved {
                request: pending.request,
                item: id,
                result,
            });
        });
    }

    pub(super) fn asset_created(&mut self, callback: u32, item: &InvItem) {
        if let Some(pending) = self.asset_imports.creation_permissions.remove(&callback)
            && let Some(update) = pending.update(item, self.agent_id())
        {
            let mut message = UpdateInventoryItem::default();
            message.agent_data.agent_id = self.agent_id();
            message.agent_data.session_id = self.session_id();
            message.agent_data.transaction_id = Uuid::new_v4();
            message.inventory_data.push(update);
            self.send_main(&message, true);
        }
        if let Some(pending) = self.asset_imports.settings.remove(&callback) {
            self.settings_created(pending, item);
            return;
        }
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
            .creation_permissions
            .retain(|_, pending| pending.started.elapsed() < Duration::from_secs(60));
        self.asset_imports.meshes.lock().retain(|_, pending| !pending.expired());
        self.asset_imports.settings.retain(|_, pending| {
            if pending.started.elapsed() < Duration::from_secs(60) {
                return true;
            }
            emit(
                self.sh,
                NetEvent::InventoryContentSaved {
                    request: pending.request,
                    item: Uuid::nil(),
                    result: Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save").into()),
                },
            );
            false
        });
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
    fn creation_defaults_only_update_the_expected_owned_item_and_preserve_owner_rights() {
        let owner = Uuid::from_u128(1);
        let mut item = InvItem {
            id: Uuid::from_u128(2),
            parent: Uuid::from_u128(3),
            owner,
            asset_type: 7,
            owner_mask: 0x7fff_ffff,
            base_mask: 0x7fff_ffff,
            next_owner_mask: 0x7fff_ffff,
            ..Default::default()
        };
        let pending = CreationPermissions {
            started: Instant::now(),
            parent: item.parent,
            asset_type: 7,
            rights: crate::inventory::upload::Permissions {
                next_owner: 0x8_2000,
                group: 0x8_c000,
                everyone: 0x8000,
            },
        };
        let update = pending.update(&item, owner).unwrap();
        assert_eq!(
            (update.next_owner_mask, update.group_mask, update.everyone_mask),
            (0x8_2000, 0x8_c000, 0x8000)
        );
        assert_eq!((update.owner_mask, update.base_mask), (item.owner_mask, item.base_mask));
        for (parent, asset_type, item_owner) in [
            (Uuid::nil(), 7, owner),
            (item.parent, 10, owner),
            (item.parent, 7, Uuid::from_u128(4)),
        ] {
            assert!(
                pending
                    .update(
                        &InvItem {
                            parent,
                            asset_type,
                            owner: item_owner,
                            ..item.clone()
                        },
                        owner
                    )
                    .is_none()
            );
        }
        item.owner_mask = 0x2000;
        let update = pending.update(&item, owner).unwrap();
        assert_eq!((update.next_owner_mask, update.group_mask, update.everyone_mask), (0x2000, 0, 0));
    }

    #[test]
    fn mesh_import_parity_slow_quote_keeps_the_full_transfer_and_confirmation_windows() {
        let mut pending = MeshOffer {
            started: Instant::now() - Duration::from_secs(400),
            upload: crate::inventory::mesh_upload::Request {
                folder: Uuid::from_u128(1),
                texture_folder: Uuid::from_u128(2),
                name: "Mesh".into(),
                description: String::new(),
                query: Arc::new(vec![1]),
                resources: Arc::new(vec![1]),
                timeout_secs: 600,
            },
            offer: None,
        };
        assert!(!pending.expired(), "a fee request can take the configured 600 seconds");
        pending.quoted(crate::inventory::mesh_upload::Offer {
            uploader: String::new(),
            cost: 0,
            data: Llsd::Undef,
        });
        assert!(!pending.expired(), "confirmation time starts when the quote arrives");
        pending.started = Instant::now() - Duration::from_secs(301);
        assert!(pending.expired());
    }
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
