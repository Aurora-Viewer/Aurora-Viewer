//! LLAgentWearables::saveWearable/saveWearableAs and update_inventory_item
//! (Firestorm indra/newview, originally LGPL 2.1). Upload the asset, preserve
//! permissions through CopyInventoryItem, then use the transaction hash_id
//! in AIS and read the item back before reporting a successful save.

use super::*;
use crate::inventory::{InvItem, operations::CopyItem};
use aurora_llsd::llsd_map;

pub(super) struct Pending {
    pub request: Uuid,
    pub item: InvItem,
    pub copy_name: Option<String>,
    pub transaction: Uuid,
    pub asset: Uuid,
    pub started: Instant,
}

/// LLWearableType: the four body parts and the thirteen clothing types.
fn editable(asset_type: i32, flags: u32) -> bool {
    matches!((asset_type, flags & 0xff), (13, 0..=3) | (5, 4..=16))
}

/// Why a save failed, worded for the kind of wearable saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    Denied,
    Rejected,
    Interrupted,
    Unconfirmed,
    Unavailable,
}

pub(super) fn failure(item: &InvItem, why: Failure) -> String {
    if !matches!((item.asset_type, item.flags & 0xff), (13, 0) | (5, 15)) {
        return match why {
            Failure::Denied => aurora_i18n::tr!("wearable-save-denied"),
            Failure::Rejected => aurora_i18n::tr!("wearable-save-rejected"),
            Failure::Interrupted => aurora_i18n::tr!("wearable-save-interrupted"),
            Failure::Unconfirmed => aurora_i18n::tr!("wearable-save-unconfirmed"),
            Failure::Unavailable => aurora_i18n::tr!("wearable-save-unavailable"),
        }
        .into();
    }
    let physics = item.asset_type == 5;
    match (why, physics) {
        (Failure::Denied, false) => aurora_i18n::tr!("shape-save-denied"),
        (Failure::Denied, true) => aurora_i18n::tr!("physics-save-denied"),
        (Failure::Rejected, false) => aurora_i18n::tr!("shape-save-rejected"),
        (Failure::Rejected, true) => aurora_i18n::tr!("physics-save-rejected"),
        (Failure::Interrupted, false) => aurora_i18n::tr!("shape-save-interrupted"),
        (Failure::Interrupted, true) => aurora_i18n::tr!("physics-save-interrupted"),
        (Failure::Unconfirmed, false) => aurora_i18n::tr!("shape-save-unconfirmed"),
        (Failure::Unconfirmed, true) => aurora_i18n::tr!("physics-save-unconfirmed"),
        (Failure::Unavailable, false) => aurora_i18n::tr!("shape-save-unavailable"),
        (Failure::Unavailable, true) => aurora_i18n::tr!("physics-save-unavailable"),
    }
    .into()
}

fn eligible(item: &InvItem, owner: Uuid, copy: bool) -> bool {
    !item.id.is_nil()
        && item.owner == owner
        && editable(item.asset_type, item.flags)
        && item.owner_mask & if copy { 0x8000 } else { 0x4000 } != 0
}

/// The item read back is the saved one: same kind of wearable as before.
fn same_kind(a: &InvItem, b: &InvItem) -> bool {
    a.asset_type == b.asset_type && a.flags & 0xff == b.flags & 0xff
}

async fn commit_asset(http: &reqwest::Client, cap: &str, fetch: &str, owner: Uuid, pending: &Pending) -> Result<InvItem, String> {
    let body = llsd_map! {"hash_id"=>pending.transaction};
    // Only the asset changes; no rewritten ownership, sale or masks.
    crate::outfits::send(http.patch(format!("{}/item/{}", cap.trim_end_matches('/'), pending.item.id)), &body).await?;
    let body = crate::inventory::fetch_items_body(&[pending.item.id], owner);
    let reply = caps::post_llsd(http, fetch, &body)
        .await
        .map_err(|_| failure(&pending.item, Failure::Unconfirmed))?;
    crate::inventory::parse_items_response(&reply)
        .into_iter()
        .find(|item| item.id == pending.item.id && item.owner == owner && item.asset_id == pending.asset && same_kind(item, &pending.item))
        .ok_or_else(|| failure(&pending.item, Failure::Unconfirmed))
}

impl Session<'_> {
    pub(super) fn save_wearable(&mut self, request: Uuid, item: InvItem, data: Vec<u8>, copy_name: Option<String>) {
        if !eligible(&item, self.agent_id(), copy_name.is_some())
            || data.is_empty()
            || data.len() > 64 * 1024
            || self.inventory_uploads.wearable_copies.len() >= 8
            || self.inventory_uploads_busy()
            || copy_name
                .as_ref()
                .is_some_and(|n| n.trim().is_empty() || n.contains(['\n', '\r', '\0']) || n.len() > 255)
        {
            emit(
                self.sh,
                NetEvent::WearableSaved {
                    request,
                    result: Err(failure(&item, Failure::Denied)),
                },
            );
            return;
        }
        let pending = Pending {
            request,
            asset: item.asset_id,
            item,
            copy_name,
            transaction: Uuid::nil(),
            started: Instant::now(),
        };
        // A non-modifiable wearable can be copied, but its contents stay original.
        if pending.item.owner_mask & 0x4000 == 0 {
            self.wearable_asset_ready(pending, Uuid::nil(), Uuid::nil());
        } else {
            self.upload_wearable_save(pending, data);
        }
    }

    pub(super) fn wearable_asset_ready(&mut self, mut pending: Pending, transaction: Uuid, asset: Uuid) {
        if !transaction.is_nil() {
            pending.transaction = transaction;
            pending.asset = asset;
        }
        if let Some(name) = &pending.copy_name {
            let callback = self.inventory_callback(false);
            let message = inventory_copies::message(
                self.agent_id(),
                self.session_id(),
                callback,
                &CopyItem {
                    item: pending.item.id,
                    owner: pending.item.owner,
                    parent: pending.item.parent,
                    name: name.clone(),
                },
            );
            pending.started = Instant::now();
            self.inventory_uploads.wearable_copies.insert(callback, pending);
            self.send_main(&message, true);
        } else {
            self.commit_wearable(pending);
        }
    }

    pub(super) fn accept_wearable_copy(&mut self, callback: u32, item: &InvItem) -> bool {
        let Some(mut pending) = self.inventory_uploads.wearable_copies.remove(&callback) else {
            return false;
        };
        if item.id.is_nil()
            || item.id == pending.item.id
            || item.owner != self.agent_id()
            || !same_kind(item, &pending.item)
            || item.asset_id != pending.item.asset_id
            || item.parent != pending.item.parent
        {
            emit(
                self.sh,
                NetEvent::WearableSaved {
                    request: pending.request,
                    result: Err(failure(&pending.item, Failure::Unconfirmed)),
                },
            );
        } else if pending.transaction.is_nil() {
            emit(
                self.sh,
                NetEvent::WearableSaved {
                    request: pending.request,
                    result: Ok(item.clone()),
                },
            );
        } else {
            pending.item = item.clone();
            self.commit_wearable(pending);
        }
        true
    }

    fn commit_wearable(&mut self, pending: Pending) {
        let Some((cap, fetch)) = self.main_cap("InventoryAPIv3").zip(self.main_cap("FetchInventory2")) else {
            emit(
                self.sh,
                NetEvent::WearableSaved {
                    request: pending.request,
                    result: Err(failure(&pending.item, Failure::Unavailable)),
                },
            );
            return;
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(30), commit_asset(&http, &cap, &fetch, owner, &pending))
                .await
                .unwrap_or_else(|_| Err(failure(&pending.item, Failure::Interrupted)));
            let _ = events.send(NetEvent::WearableSaved {
                request: pending.request,
                result,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(replies: Vec<(u16, Llsd)>) -> (String, tokio::task::JoinHandle<Vec<(String, Llsd)>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, value) in replies {
                let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                    .await
                    .expect("request deadline")
                    .expect("request");
                let mut bytes = Vec::new();
                let (start, length) = loop {
                    let mut chunk = [0u8; 4096];
                    let n = socket.read(&mut chunk).await.expect("read");
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let length = header
                            .lines()
                            .filter_map(|l| l.split_once(':'))
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break (end + 4, length);
                        }
                    }
                    assert!(bytes.len() < 1024 * 1024);
                };
                requests.push((
                    String::from_utf8_lossy(&bytes[..start]).into_owned(),
                    aurora_llsd::from_xml(&bytes[start..start + length]).expect("LLSD"),
                ));
                let body = aurora_llsd::to_xml(&value);
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .expect("header");
                socket.write_all(&body).await.expect("body");
                crate::test_http::finish(socket);
            }
            requests
        });
        (base, handle)
    }

    fn pending() -> Pending {
        Pending {
            request: Uuid::from_u128(9),
            transaction: Uuid::from_u128(8),
            asset: Uuid::from_u128(7),
            item: InvItem {
                id: Uuid::from_u128(1),
                owner: Uuid::from_u128(2),
                owner_mask: 0x4000,
                asset_type: 13,
                ..Default::default()
            },
            copy_name: None,
            started: Instant::now(),
        }
    }
    fn saved(asset: Uuid) -> Llsd {
        llsd_map! {"items"=>Llsd::Array(vec![llsd_map! {
            "item_id"=>Uuid::from_u128(1),"asset_id"=>asset,"type"=>"bodypart","inv_type"=>"wearable",
            "permissions"=>llsd_map! {"owner_id"=>Uuid::from_u128(2),"owner_mask"=>0x4000u32,"next_owner_mask"=>0x8000u32},
        }])}
    }

    #[tokio::test]
    async fn save_patches_only_the_transaction_and_requires_the_confirmed_new_asset() {
        let pending = pending();
        let (base, task) = server(vec![(200, Llsd::default()), (200, saved(pending.asset))]).await;
        let item = commit_asset(
            &reqwest::Client::new(),
            &format!("{base}/ais"),
            &format!("{base}/fetch"),
            pending.item.owner,
            &pending,
        )
        .await
        .expect("confirmed save");
        assert_eq!(item.owner_mask, 0x4000);
        assert_eq!(item.next_owner_mask, 0x8000);
        let requests = task.await.expect("requests");
        assert!(requests[0].0.starts_with(&format!("PATCH /ais/item/{} ", pending.item.id)));
        assert_eq!(requests[0].1.as_map().expect("body").len(), 1);
        assert_eq!(requests[0].1["hash_id"].as_uuid(), pending.transaction);
        assert_eq!(requests[1].1["items"].at(0)["item_id"].as_uuid(), pending.item.id);
    }
    #[tokio::test]
    async fn mismatched_readback_and_http_denial_never_report_a_save() {
        let pending = pending();
        let (base, task) = server(vec![(200, Llsd::default()), (200, saved(Uuid::from_u128(6)))]).await;
        assert!(
            commit_asset(
                &reqwest::Client::new(),
                &base,
                &format!("{base}/fetch"),
                pending.item.owner,
                &pending
            )
            .await
            .is_err()
        );
        task.await.expect("readback");
        let (base, task) = server(vec![(403, Llsd::default())]).await;
        assert!(
            commit_asset(
                &reqwest::Client::new(),
                &base,
                &format!("{base}/fetch"),
                pending.item.owner,
                &pending
            )
            .await
            .is_err()
        );
        assert_eq!(task.await.expect("denial").len(), 1);
    }
    #[test]
    fn shape_save_requires_modify_and_copy_requires_copy_without_rewriting_permissions() {
        let mut item = InvItem {
            id: Uuid::from_u128(1),
            owner: Uuid::from_u128(2),
            asset_type: 13,
            owner_mask: 0x4000,
            ..Default::default()
        };
        assert!(eligible(&item, item.owner, false));
        assert!(!eligible(&item, item.owner, true));
        item.owner_mask = 0x8000;
        assert!(!eligible(&item, item.owner, false));
        assert!(eligible(&item, item.owner, true));
        assert!(!eligible(&item, Uuid::nil(), true));
        item.flags = 17;
        assert!(!eligible(&item, item.owner, true));
    }
    #[test]
    fn a_physics_layer_and_system_clothing_save_with_their_original_kind() {
        let mut item = InvItem {
            id: Uuid::from_u128(1),
            owner: Uuid::from_u128(2),
            asset_type: 5,
            flags: 15,
            owner_mask: 0xC000,
            ..Default::default()
        };
        assert!(eligible(&item, item.owner, false) && eligible(&item, item.owner, true));
        assert_eq!(failure(&item, Failure::Denied), aurora_i18n::tr!("physics-save-denied"));
        let mut read_back = item.clone();
        assert!(same_kind(&read_back, &item));
        read_back.flags = 4;
        assert!(!same_kind(&read_back, &item), "a shirt is not the saved physics");
        item.flags = 4;
        assert!(eligible(&item, item.owner, false), "system clothing is editable");
        item.flags = 0x0100_000F;
        assert!(eligible(&item, item.owner, false), "only the low byte is the wearable type");
    }
}
