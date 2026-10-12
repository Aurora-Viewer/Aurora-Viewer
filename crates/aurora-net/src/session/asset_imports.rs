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
}

impl Session<'_> {
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
            },
        );
        self.send_main(&message, true);
    }

    pub(super) fn asset_created(&mut self, callback: u32, item: &InvItem) {
        let Some(pending) = self.asset_imports.creates.remove(&callback) else {
            return;
        };
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
                if saved.asset_type != 57 || saved.asset_id.is_nil() {
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
