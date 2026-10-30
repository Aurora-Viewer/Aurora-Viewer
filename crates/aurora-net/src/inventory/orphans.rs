//! LLInventoryModelBackgroundFetch::findLostItems / bulkFetch and
//! AISAPI::FetchOrphans (llinventorymodelbackgroundfetch.cpp, llaisapi.cpp).
use super::{InvFolder, InvItem, folder_from_llsd, item_from_llsd};
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct Contents {
    pub folders: Vec<InvFolder>,
    pub items: Vec<InvItem>,
}

pub async fn fetch(http: &reqwest::Client, ais: Option<&str>, legacy: Option<&str>, owner: Uuid) -> Result<Contents, &'static str> {
    if owner.is_nil() {
        return Err("missing orphan inventory owner");
    }
    if let Some(cap) = ais {
        let doc = crate::caps::get_llsd_as(
            http,
            &format!("{}/orphans", cap.trim_end_matches('/')),
            crate::probe::HttpKind::Inventory,
        )
        .await
        .map_err(|_| "AIS orphan inventory read failed")?;
        parse_ais(&doc, owner)
    } else if let Some(cap) = legacy {
        let body = llsd_map! { "folders" => Llsd::Array(vec![llsd_map! {
            "folder_id" => Uuid::nil().to_string(), "owner_id" => owner, "sort_order" => 1,
            "fetch_folders" => false, "fetch_items" => true
        }]) };
        let doc = crate::caps::post_llsd_as(http, cap, &body, crate::probe::HttpKind::Inventory)
            .await
            .map_err(|_| "legacy orphan inventory read failed")?;
        parse_legacy(&doc, owner)
    } else {
        Err("no orphan inventory capability")
    }
}

fn owned(item: InvItem, owner: Uuid) -> Result<InvItem, &'static str> {
    if item.owner != owner {
        return Err("foreign orphan inventory record");
    }
    Ok(item)
}

fn category(record: &Llsd, owner: Uuid) -> Result<InvFolder, &'static str> {
    if !record.has("parent_id")
        || !record.has("name")
        || record.has("agent_id") && record["agent_id"].as_uuid() != owner
        || record.has("owner_id") && record["owner_id"].as_uuid() != owner
    {
        return Err("invalid orphan inventory folder");
    }
    let mut folder = folder_from_llsd(record).ok_or("invalid orphan inventory folder")?;
    folder.favorite_known = true;
    folder.favorite = super::ais::favorite(record)?;
    Ok(folder)
}

fn parse_ais(doc: &Llsd, owner: Uuid) -> Result<Contents, &'static str> {
    if doc.has("error") || doc.has("agent_id") && doc["agent_id"].as_uuid() != owner {
        return Err("invalid AIS orphan inventory reply");
    }
    let mut out = Contents::default();
    if doc.as_map().is_some_and(|m| m.is_empty()) {
        return Ok(out);
    }
    let mut queue = vec![(doc, 0)];
    let mut saw_collection = false;
    while let Some((record, depth)) = queue.pop() {
        if depth > super::ais::MAX_FOLDER_DEPTH || out.items.len() + out.folders.len() > 100_000 {
            return Err("oversized AIS orphan inventory reply");
        }
        let embedded = &record["_embedded"];
        for collection in ["items", "links"] {
            if embedded.has(collection) {
                let records = embedded[collection].as_map().ok_or("invalid AIS orphan collection")?;
                saw_collection = true;
                for (key, value) in records {
                    let item = owned(super::ais::item(value, owner)?, owner)?;
                    if key.parse::<Uuid>().ok() != Some(item.id) {
                        return Err("mismatched AIS orphan identity");
                    }
                    out.items.push(item);
                    queue.push((value, depth + 1));
                }
            }
        }
        if embedded.has("item") {
            out.items.push(owned(super::ais::item(&embedded["item"], owner)?, owner)?);
            saw_collection = true;
        }
        if embedded.has("categories") {
            let records = embedded["categories"].as_map().ok_or("invalid AIS orphan folders")?;
            saw_collection = true;
            for (key, value) in records {
                let folder = category(value, owner)?;
                if key.parse::<Uuid>().ok() != Some(folder.id) {
                    return Err("mismatched AIS orphan folder");
                }
                out.folders.push(folder);
                queue.push((value, depth + 1));
            }
        }
        if embedded.has("category") {
            let value = &embedded["category"];
            out.folders.push(category(value, owner)?);
            queue.push((value, depth + 1));
            saw_collection = true;
        }
    }
    if !saw_collection {
        return Err("missing AIS orphan collection");
    }
    out.items.sort_by_key(|it| it.id);
    out.items.dedup_by_key(|it| it.id);
    Ok(out)
}

fn parse_legacy(doc: &Llsd, owner: Uuid) -> Result<Contents, &'static str> {
    if doc.has("error") || doc.has("errors") || !doc["bad_folders"].as_array().is_empty() || !matches!(&doc["folders"], Llsd::Array(_)) {
        return Err("invalid legacy orphan reply");
    }
    let mut out = Contents::default();
    for folder in doc["folders"].as_array() {
        if !folder.has("folder_id")
            || !folder["folder_id"].as_uuid().is_nil()
            || folder.has("owner_id") && folder["owner_id"].as_uuid() != owner
            || !matches!(&folder["items"], Llsd::Array(_))
        {
            return Err("invalid legacy null-folder reply");
        }
        for record in folder["items"].as_array() {
            if !record.has("name") || !record.has("type") {
                return Err("incomplete legacy orphan");
            }
            let mut item = owned(item_from_llsd(record).ok_or("invalid legacy orphan")?, owner)?;
            // BGFolderHttpHandler files every null-folder result in Lost And
            // Found, then sends its actual parent through reliable UDP.
            item.parent = Uuid::nil();
            item.favorite_known = item.favorite;
            out.items.push(item);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn record(n: u128) -> Llsd {
        llsd_map! {
            "item_id" => id(n), "parent_id" => Uuid::nil(), "name" => "Recovered object", "type" => 6,
            "permissions" => llsd_map! { "owner_id" => id(2), "owner_mask" => 0x7fffffff }
        }
    }
    fn ais(items: Vec<Llsd>) -> Llsd {
        let records = items.into_iter().map(|it| (it["item_id"].as_uuid().to_string(), it)).collect();
        llsd_map! { "_embedded" => llsd_map! { "items" => Llsd::Map(records), "links" => llsd_map! {} } }
    }
    fn legacy(items: Vec<Llsd>) -> Llsd {
        llsd_map! { "folders" => Llsd::Array(vec![llsd_map! {
            "folder_id" => Uuid::nil(), "owner_id" => id(2), "items" => Llsd::Array(items)
        }]) }
    }

    #[test]
    fn orphan_reads_keep_id_permissions_links_and_empty_success_separate_from_errors() {
        let parsed = parse_ais(&ais(vec![record(3)]), id(2)).expect("orphan");
        assert_eq!(parsed.items[0].id, id(3));
        assert_eq!(parsed.items[0].owner_mask, 0x7fffffff);
        assert!(parse_ais(&ais(Vec::new()), id(2)).expect("empty").items.is_empty());
        assert!(parse_legacy(&legacy(Vec::new()), id(2)).expect("empty").items.is_empty());
        let link =
            llsd_map! { "item_id" => id(4), "parent_id" => Uuid::nil(), "name" => "Recovered link", "type" => 24, "linked_id" => id(3) };
        let doc = llsd_map! { "_embedded" => llsd_map! { "links" => llsd_map! { id(4).to_string() => link } } };
        assert_eq!(parse_ais(&doc, id(2)).expect("link").items[0].asset_id, id(3));
        for bad in [
            llsd_map! { "error" => "not found" },
            llsd_map! { "item_id" => id(3) },
            llsd_map! { "_embedded" => llsd_map! { "items" => Llsd::Array(Vec::new()) } },
        ] {
            assert!(parse_ais(&bad, id(2)).is_err());
        }
        let mut foreign = record(3);
        foreign.insert("permissions", llsd_map! { "owner_id" => id(99) });
        assert!(parse_ais(&ais(vec![foreign.clone()]), id(2)).is_err());
        assert!(parse_legacy(&legacy(vec![foreign]), id(2)).is_err());
        let mut doc = ais(vec![record(3)]);
        doc.insert("agent_id", id(99));
        assert!(parse_ais(&doc, id(2)).is_err());
    }

    #[tokio::test]
    async fn orphan_requests_use_ais_get_or_the_exact_legacy_null_folder_post() {
        for use_ais in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("server");
            let url = format!("http://{}", listener.local_addr().expect("address"));
            let service = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.expect("connection");
                let mut bytes = Vec::new();
                loop {
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            if use_ais {
                                assert!(headers.starts_with("GET /orphans "));
                            } else {
                                assert!(headers.starts_with("POST / "));
                                let doc = aurora_llsd::from_xml(&bytes[end + 4..]).expect("request LLSD");
                                let folder = &doc["folders"].at(0);
                                assert_eq!(folder["folder_id"].as_str(), "00000000-0000-0000-0000-000000000000");
                                assert_eq!(folder["owner_id"].as_uuid(), id(2));
                                assert!(!folder["fetch_folders"].as_bool() && folder["fetch_items"].as_bool());
                            }
                            break;
                        }
                    }
                    let mut chunk = [0; 2048];
                    let n = socket.read(&mut chunk).await.expect("request");
                    assert!(n > 0);
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let body = aurora_llsd::to_xml(&if use_ais { ais(vec![record(3)]) } else { legacy(vec![record(3)]) });
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(headers.as_bytes()).await.expect("headers");
                socket.write_all(&body).await.expect("body");
                crate::test_http::finish(socket);
            });
            let parsed = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                fetch(
                    &reqwest::Client::new(),
                    use_ais.then_some(url.as_str()),
                    (!use_ais).then_some(url.as_str()),
                    id(2),
                ),
            )
            .await
            .expect("read timeout")
            .expect("orphans");
            assert_eq!(parsed.items[0].id, id(3));
            assert!(parsed.items[0].parent.is_nil());
            service.await.expect("service");
        }
    }
}
