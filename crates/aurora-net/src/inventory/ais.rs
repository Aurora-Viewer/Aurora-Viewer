//! AIS inventory reads following AISAPI::FetchCategoryChildren / FetchItem and
//! AISUpdate::parseEmbedded (indra/newview/llaisapi.cpp).

use super::{FolderContents, InvFolder, InvItem, folder_from_llsd, item_from_llsd};
use aurora_llsd::Llsd;
use std::collections::HashSet;
use uuid::Uuid;

pub(super) fn favorite(record: &Llsd) -> Result<bool, &'static str> {
    match &record["favorite"] {
        Llsd::Undef => Ok(false),
        Llsd::Map(map) => match map.get("toggled") {
            Some(Llsd::Boolean(value)) => Ok(*value),
            Some(Llsd::Integer(value)) => Ok(*value != 0),
            _ => Err("invalid AIS favorite metadata"),
        },
        _ => Err("invalid AIS favorite metadata"),
    }
}

pub(super) fn item(record: &Llsd, owner: Uuid) -> Result<InvItem, &'static str> {
    // AISUpdate::parseItem preserves real records with a null parent for
    // Lost And Found repair. An absent parent field still denotes a partial
    // response (for example an error carrying only item_id).
    for (key, error) in [
        ("parent_id", "AIS item has no parent field"),
        ("name", "AIS item has no name"),
        ("type", "AIS item has no asset type"),
    ] {
        if !record.has(key) {
            return Err(error);
        }
    }
    let mut item = item_from_llsd(record).ok_or("invalid AIS item")?;
    // LLInventoryItem::fromLLSD resets missing favorites on complete records.
    item.favorite_known = true;
    item.favorite = favorite(record)?;
    if matches!(item.asset_type, 24 | 25) && !record.has("permissions") {
        // AISUpdate::parseLink initializes link permissions from the agent.
        item.owner = owner;
        item.creator = owner;
    }
    Ok(item)
}

fn category(record: &Llsd) -> Result<InvFolder, &'static str> {
    if record["parent_id"].as_uuid().is_nil() || !record.has("name") {
        return Err("incomplete AIS category");
    }
    let mut folder = folder_from_llsd(record).ok_or("invalid AIS category")?;
    folder.favorite_known = true;
    folder.favorite = favorite(record)?;
    Ok(folder)
}

fn parse_folder(doc: &Llsd, id: Uuid, owner: Uuid) -> Result<FolderContents, &'static str> {
    if doc["category_id"].as_uuid() != id
        || !doc.has("version")
        || doc["version"].as_i32() < 0
        || (doc.has("agent_id") && doc["agent_id"].as_uuid() != owner)
    {
        return Err("invalid AIS folder snapshot");
    }
    let embedded = &doc["_embedded"];
    // AISUpdate::parseDescendentCount: a partial collection is not a folder
    // snapshot. COF/outfit folders can carry links alone.
    let links_only = matches!(doc["type_default"].as_i32(), 46 | 47);
    if embedded["links"].as_map().is_none()
        || (!links_only && (embedded["categories"].as_map().is_none() || embedded["items"].as_map().is_none()))
    {
        return Err("incomplete AIS folder snapshot");
    }
    let mut contents = FolderContents {
        folder_id: id,
        owner_id: owner,
        version: doc["version"].as_i32(),
        folders: Vec::new(),
        items: Vec::new(),
    };
    if let Some(records) = embedded["categories"].as_map() {
        for (key, record) in records {
            let folder = category(record)?;
            if key.parse::<Uuid>().ok() != Some(folder.id) || folder.parent != id {
                return Err("invalid AIS child category");
            }
            contents.folders.push(folder);
        }
    }
    for collection in ["items", "links"] {
        if let Some(records) = embedded[collection].as_map() {
            for (key, record) in records {
                let child = item(record, owner)?;
                if key.parse::<Uuid>().ok() != Some(child.id)
                    || child.parent != id && !child.parent.is_nil() && child.parent != Uuid::from_u128(0x10)
                {
                    return Err("invalid AIS child item");
                }
                let target = child.asset_id;
                if matches!(child.asset_type, 24 | 25) {
                    let original = &record["_embedded"];
                    if let Ok(original) = item(&original["item"], owner) {
                        if original.id != target {
                            return Err("invalid AIS linked item");
                        }
                        contents.items.push(original);
                    }
                    if let Ok(original) = category(&original["category"]) {
                        if original.id != target {
                            return Err("invalid AIS linked category");
                        }
                        contents.folders.push(original);
                    }
                }
                contents.items.push(child);
            }
        }
    }
    // The category's own metadata is authoritative too (e.g. on refresh).
    // The model indexes records by parent, excluding this from its child list.
    if let Ok(folder) = category(doc) {
        contents.folders.push(folder);
    }
    Ok(contents)
}

/// MAX_FOLDER_DEPTH_REQUEST (llaisapi.cpp): the depth of a recursive read.
/// The server caps deeper requests and refuses a reply that would be too big.
pub const MAX_FOLDER_DEPTH: i32 = 50;

/// A folder's snapshot, then those nested in it, parents first so that each
/// folder is known before its own snapshot is applied. Like
/// AISUpdate::parseCategory, a folder counts as fetched only while `depth`
/// is not negative and its listing is complete; below that it stays listed
/// by its parent, to be fetched later.
fn parse_tree(doc: &Llsd, id: Uuid, owner: Uuid, depth: i32, out: &mut Vec<FolderContents>) -> Result<(), &'static str> {
    if depth < 0 {
        return Ok(());
    }
    out.push(parse_folder(doc, id, owner)?);
    for (child, record) in embedded_categories(doc) {
        if record.has("_embedded") {
            let mut nested = Vec::new();
            // a nested folder the reply cut short is fetched again later,
            // without losing the rest of the reply
            if parse_tree(record, child, owner, depth - 1, &mut nested).is_ok() {
                out.append(&mut nested);
            }
        }
    }
    Ok(())
}

fn embedded_categories(doc: &Llsd) -> impl Iterator<Item = (Uuid, &Llsd)> {
    doc["_embedded"]["categories"]
        .as_map()
        .into_iter()
        .flatten()
        .filter_map(|(key, record)| Some((key.parse::<Uuid>().ok()?, record)))
}

/// AISAPI::FetchCategoryChildren (no `children`) or FetchCategorySubset
/// (`children` of `parent`, a batch of its folders): whole subtrees in one
/// read. A requested child missing from a subset reply is left unfetched;
/// the caller sees it still is.
pub async fn fetch_tree(
    http: &reqwest::Client,
    cap: &str,
    owner: Uuid,
    parent: Uuid,
    children: &[Uuid],
    depth: i32,
) -> Result<Vec<FolderContents>, &'static str> {
    let mut url = format!("{}/category/{parent}/children?depth={depth}", cap.trim_end_matches('/'));
    if !children.is_empty() {
        let ids: Vec<String> = children.iter().map(Uuid::to_string).collect();
        url.push_str("&children=");
        url.push_str(&ids.join(","));
    }
    let doc = get(http, url).await?;
    parse_tree_reply(&doc, owner, parent, children, depth)
}

fn parse_tree_reply(doc: &Llsd, owner: Uuid, parent: Uuid, children: &[Uuid], depth: i32) -> Result<Vec<FolderContents>, &'static str> {
    let mut out = Vec::new();
    if children.is_empty() {
        parse_tree(doc, parent, owner, depth, &mut out)?;
        return Ok(out);
    }
    if doc["category_id"].as_uuid() != parent {
        return Err("invalid AIS subset reply");
    }
    for (child, record) in embedded_categories(doc) {
        if children.contains(&child) {
            let mut nested = Vec::new();
            if parse_tree(record, child, owner, depth - 1, &mut nested).is_ok() {
                out.append(&mut nested);
            }
        }
    }
    Ok(out)
}

async fn get(http: &reqwest::Client, url: String) -> Result<Llsd, &'static str> {
    crate::caps::get_llsd_as(http, &url, crate::probe::HttpKind::Inventory)
        .await
        .map_err(|_| "AIS inventory read failed")
}

/// Bounded parallel reads, outside the viewer thread. Never substitute a
/// legacy listing: it may omit the favorites and thumbnails of siblings.
pub async fn fetch_folders(http: &reqwest::Client, cap: &str, owner: Uuid, ids: &[Uuid]) -> Result<Vec<FolderContents>, &'static str> {
    let mut contents = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(16) {
        let mut tasks = tokio::task::JoinSet::new();
        for id in chunk.iter().copied() {
            let http = http.clone();
            let url = format!("{}/category/{id}/children?depth=0", cap.trim_end_matches('/'));
            tasks.spawn(async move { parse_folder(&get(&http, url).await?, id, owner) });
        }
        while let Some(result) = tasks.join_next().await {
            contents.push(result.map_err(|_| "AIS inventory task failed")??);
        }
    }
    Ok(contents)
}

fn parse_item_reply(doc: &Llsd, id: Uuid, owner: Uuid) -> Result<InvItem, &'static str> {
    let record = if doc.has("item_id") {
        doc
    } else if doc["_embedded"]["item"]["item_id"].as_uuid() == id {
        &doc["_embedded"]["item"]
    } else {
        doc["_embedded"]["items"]
            .as_map()
            .and_then(|items| items.get(&id.to_string()))
            .ok_or("AIS item response has no matching record")?
    };
    let item = item(record, owner)?;
    if item.id != id {
        return Err("mismatched AIS item");
    }
    Ok(item)
}

pub async fn fetch_items(http: &reqwest::Client, cap: &str, owner: Uuid, ids: &[Uuid]) -> Result<Vec<InvItem>, &'static str> {
    let reads = read_items(http, cap, owner, ids).await;
    if reads.missing.is_empty() && reads.failed.is_empty() {
        Ok(reads.items)
    } else {
        Err("AIS inventory read failed")
    }
}

/// Each item's own outcome: one item gone from the server must not hide
/// the others read with it.
#[derive(Debug, Default, PartialEq)]
pub struct ItemReads {
    pub items: Vec<InvItem>,
    /// Answered 404 or 410: the item does not exist (a broken link's original).
    pub missing: Vec<Uuid>,
    /// Network error, other status, unreadable answer: proves nothing.
    pub failed: Vec<Uuid>,
}

/// Bounded parallel reads of single items (AISAPI::FetchItem).
pub async fn read_items(http: &reqwest::Client, cap: &str, owner: Uuid, ids: &[Uuid]) -> ItemReads {
    let mut reads = ItemReads::default();
    for chunk in ids.chunks(16) {
        let mut tasks = tokio::task::JoinSet::new();
        for id in chunk.iter().copied() {
            let http = http.clone();
            let url = format!("{}/item/{id}", cap.trim_end_matches('/'));
            tasks.spawn(async move {
                let read = crate::caps::get_llsd_as(&http, &url, crate::probe::HttpKind::Inventory).await;
                (id, read.map(|doc| parse_item_reply(&doc, id, owner).ok()))
            });
        }
        let mut answered = HashSet::new();
        while let Some(result) = tasks.join_next().await {
            let Ok((id, read)) = result else { continue };
            answered.insert(id);
            match read {
                Ok(Some(item)) => reads.items.push(item),
                Err(crate::caps::CapsError::Status(404 | 410)) => reads.missing.push(id),
                Ok(None) | Err(_) => reads.failed.push(id),
            }
        }
        // a panicked read task answers nothing: failed, asked again
        reads.failed.extend(chunk.iter().filter(|id| !answered.contains(id)));
    }
    reads
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::operations::{Mutation, Operation, metadata_favorite, mutate};
    use aurora_llsd::{Llsd, llsd_map};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn record(n: u128, folder: bool, kind: i32, favorite: bool) -> Llsd {
        let mut record = llsd_map! {
            "parent_id" => id(1), "name" => format!("Entry {n}"), "favorite" => if favorite { llsd_map! { "toggled" => true } } else { Llsd::Undef }
        };
        record.insert(if folder { "category_id" } else { "item_id" }, id(n));
        if folder {
            record.insert("type_default", -1);
        } else {
            record.insert("type", kind);
            record.insert("inv_type", kind);
            record.insert("permissions", llsd_map! { "owner_id" => id(2) });
        }
        record
    }

    async fn inventory_server(folder: bool, kind: i32) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local test server");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(async move {
            let mut favorites = [false; 2];
            let mut requests = Vec::new();
            for _ in 0..7 {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .expect("request timeout")
                    .expect("request");
                let mut bytes = Vec::new();
                let (start, len) = loop {
                    let mut buffer = [0; 4096];
                    let n = socket.read(&mut buffer).await.expect("read");
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let len = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + len {
                            break (end + 4, len);
                        }
                    }
                };
                let header = String::from_utf8_lossy(&bytes[..start])
                    .lines()
                    .next()
                    .expect("request line")
                    .to_owned();
                let response = if header.starts_with("PATCH ") {
                    let index = usize::from(header.contains(&id(4).to_string()));
                    let body = aurora_llsd::from_xml(&bytes[start..start + len]).expect("patch");
                    favorites[index] = body["favorite"]["toggled"].as_bool();
                    record(index as u128 + 3, folder, kind, favorites[index])
                } else if header.starts_with("GET ") {
                    let mut records = Llsd::new_map();
                    for n in 3..5 {
                        records.insert(id(n).to_string(), record(n, folder, kind, favorites[n as usize - 3]));
                    }
                    let mut embedded =
                        llsd_map! { "categories" => Llsd::new_map(), "items" => Llsd::new_map(), "links" => Llsd::new_map() };
                    embedded.insert(if folder { "categories" } else { "items" }, records);
                    llsd_map! { "category_id" => id(1), "agent_id" => id(2), "version" => 3, "_embedded" => embedded }
                } else {
                    // The older endpoint omits favorite metadata for every
                    // item, even though AIS has stored both independent flags.
                    let records = (3..5)
                        .map(|n| {
                            let mut record = record(n, folder, kind, false);
                            if let Llsd::Map(map) = &mut record {
                                map.remove("favorite");
                            }
                            record
                        })
                        .collect();
                    let mut parent = llsd_map! { "folder_id" => id(1), "version" => 3 };
                    parent.insert(if folder { "categories" } else { "items" }, Llsd::Array(records));
                    llsd_map! { "folders" => Llsd::Array(vec![parent]) }
                };
                requests.push(header);
                let body = aurora_llsd::to_xml(&response);
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
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
        (url, task)
    }

    #[tokio::test]
    async fn adding_a_second_favorite_keeps_the_first_in_the_same_folder() {
        for (folder, kind) in [(false, 1), (false, 3), (false, 5), (false, 6), (false, 10), (true, -1)] {
            let (url, server) = inventory_server(folder, kind).await;
            for (target, favorite, expected) in [(3, true, [true, false]), (4, true, [true, true]), (3, false, [false, true])] {
                let result = mutate(
                    &reqwest::Client::new(),
                    &url,
                    &format!("{url}/legacy"),
                    id(2),
                    Mutation {
                        operations: vec![Operation::Patch {
                            id: id(target),
                            folder,
                            body: metadata_favorite(favorite),
                        }],
                        refresh: vec![id(1)],
                        ..Default::default()
                    },
                )
                .await
                .expect("confirmed favorite");
                for (index, target) in [id(3), id(4)].into_iter().enumerate() {
                    let actual = if folder {
                        result.contents[0].folders.iter().find(|f| f.id == target).expect("folder").favorite
                    } else {
                        result.contents[0].items.iter().find(|it| it.id == target).expect("item").favorite
                    };
                    assert_eq!(actual, expected[index], "a later change must preserve the other favorite");
                }
            }
            // A fresh reader has no mutation acknowledgments to overlay. This
            // is also what a manual refresh or the next login must observe.
            let refreshed = fetch_folders(&reqwest::Client::new(), &url, id(2), &[id(1)])
                .await
                .expect("fresh AIS snapshot");
            if folder {
                assert_eq!(
                    refreshed[0].folders.iter().filter(|f| f.favorite).map(|f| f.id).collect::<Vec<_>>(),
                    vec![id(4)]
                );
            } else {
                assert_eq!(
                    refreshed[0]
                        .items
                        .iter()
                        .filter(|it| it.favorite)
                        .map(|it| it.id)
                        .collect::<Vec<_>>(),
                    vec![id(4)]
                );
            }
            let requests = server.await.expect("requests");
            assert_eq!(requests.len(), 7);
            assert!(requests.iter().all(|line| !line.starts_with("POST ")));
        }
    }

    fn snapshot(records: Vec<Llsd>) -> Llsd {
        let mut items = Llsd::new_map();
        for record in records {
            items.insert(record["item_id"].as_uuid().to_string(), record);
        }
        llsd_map! { "category_id" => id(1), "agent_id" => id(2), "version" => 3,
        "_embedded" => llsd_map! { "categories" => Llsd::new_map(), "items" => items, "links" => Llsd::new_map() } }
    }

    #[test]
    fn complete_ais_records_clear_missing_favorites_and_resolve_link_targets() {
        let mut original = record(4, false, 1, false);
        if let Llsd::Map(map) = &mut original {
            map.remove("favorite");
        }
        let mut link = record(3, false, 24, false);
        link.insert("linked_id", id(4));
        link.insert("asset_id", id(99)); // linked_id takes precedence.
        link.insert("_embedded", llsd_map! { "item" => original });
        if let Llsd::Map(map) = &mut link {
            map.remove("permissions");
        }
        let result = parse_folder(&snapshot(vec![link]), id(1), id(2)).expect("linked original");
        assert_eq!(result.items.len(), 2);
        let original = result.items.iter().find(|it| it.id == id(4)).expect("original");
        assert!(!original.favorite);
        assert!(original.favorite_known);
        let link = result.items.iter().find(|it| it.id == id(3)).expect("link");
        assert_eq!(link.asset_id, id(4));
        assert_eq!(link.owner, id(2));
    }

    #[test]
    fn partial_or_mismatched_ais_snapshots_never_replace_inventory() {
        let valid = snapshot(vec![record(3, false, 1, true)]);
        for key in ["items", "categories", "links"] {
            let mut doc = valid.clone();
            let mut embedded = doc["_embedded"].as_map().expect("map").clone();
            embedded.remove(key);
            doc.insert("_embedded", Llsd::Map(embedded));
            assert!(parse_folder(&doc, id(1), id(2)).is_err());
        }
        assert!(parse_folder(&valid, id(99), id(2)).is_err());
        assert!(parse_folder(&valid, id(1), id(99)).is_err());
        let mut wrong_parent = record(3, false, 1, true);
        wrong_parent.insert("parent_id", id(99));
        assert!(parse_folder(&snapshot(vec![wrong_parent]), id(1), id(2)).is_err());
        assert!(item(&llsd_map! { "item_id" => id(3) }, id(2)).is_err());
        for value in [Llsd::new_map(), Llsd::from("invalid metadata")] {
            let mut malformed = record(3, false, 1, true);
            malformed.insert("favorite", value);
            assert!(parse_folder(&snapshot(vec![malformed]), id(1), id(2)).is_err());
        }
    }

    #[test]
    fn ais_fetched_objects_accept_null_parents_and_omitted_inventory_types() {
        let mut doc = record(3, false, 6, false);
        doc.insert("parent_id", Uuid::nil());
        if let Llsd::Map(map) = &mut doc {
            map.remove("inv_type");
        }
        let parsed = parse_item_reply(&doc, id(3), id(2)).expect("real orphan object");
        assert_eq!(parsed.id, id(3));
        assert_eq!(parsed.owner, id(2));
        assert!(parsed.parent.is_nil());
        assert_eq!((parsed.asset_type, parsed.inv_type), (6, 6));
        assert!(parse_item_reply(&llsd_map! { "item_id" => id(3), "error" => "not found" }, id(3), id(2)).is_err());
        assert!(
            parse_folder(&snapshot(vec![doc.clone()]), id(1), id(2)).is_ok(),
            "AISUpdate recovers null-parent children too"
        );
        doc.insert("parent_id", id(99));
        assert!(
            parse_folder(&snapshot(vec![doc]), id(1), id(2)).is_err(),
            "unrelated non-null parents are still rejected"
        );
    }

    #[test]
    fn ais_single_item_reads_accept_matching_embedded_records_without_adopting_other_items() {
        let doc = record(3, false, 6, false);
        for reply in [
            doc.clone(),
            llsd_map! { "_embedded" => llsd_map! { "item" => doc.clone() } },
            llsd_map! { "_embedded" => llsd_map! { "items" => llsd_map! { id(3).to_string() => doc.clone() } } },
        ] {
            assert_eq!(parse_item_reply(&reply, id(3), id(2)).expect("matching record").id, id(3));
            assert!(parse_item_reply(&reply, id(4), id(2)).is_err());
        }
    }

    #[tokio::test]
    async fn ais_background_item_fetch_keeps_a_valid_orphan_when_another_record_is_missing() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .expect("timeout")
                    .expect("accept");
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    let mut buf = [0; 1024];
                    let n = socket.read(&mut buf).await.expect("read");
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                }
                let request = String::from_utf8_lossy(&bytes);
                let mut doc = if request.lines().next().expect("request").contains(&id(3).to_string()) {
                    record(3, false, 6, false)
                } else {
                    llsd_map! { "item_id" => id(4), "error" => "not found" }
                };
                if doc.has("name") {
                    doc.insert("parent_id", Uuid::nil());
                    if let Llsd::Map(map) = &mut doc {
                        map.remove("inv_type");
                    }
                }
                let body = aurora_llsd::to_xml(&doc);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    String::from_utf8_lossy(&body)
                );
                socket.write_all(response.as_bytes()).await.expect("response");
                crate::test_http::finish(socket);
            }
        });
        let reads = read_items(&reqwest::Client::new(), &url, id(2), &[id(3), id(4)]).await;
        let items = reads.items;
        let failed = reads.failed;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, id(3));
        assert!(items[0].parent.is_nil());
        assert_eq!(items[0].inv_type, 6);
        assert_eq!(failed, vec![id(4)]);
        server.await.expect("server finished");
    }

    #[test]
    fn ais_refresh_retains_broken_links_and_the_folders_own_metadata() {
        let mut link = record(3, false, 24, false);
        link.insert("linked_id", id(99));
        link.insert(
            "_embedded",
            llsd_map! { "item" => llsd_map! { "item_id" => id(99), "error" => "not found" } },
        );
        let mut doc = snapshot(vec![link]);
        doc.insert("parent_id", id(100));
        doc.insert("name", "Folder");
        doc.insert("favorite", llsd_map! { "toggled" => true });
        let contents = parse_folder(&doc, id(1), id(2)).expect("folder with a broken link");
        assert_eq!(contents.items.len(), 1);
        assert_eq!(contents.items[0].asset_id, id(99));
        assert!(contents.folders.iter().any(|f| f.id == id(1) && f.favorite && f.favorite_known));
    }

    /// A folder `n` under `parent`, listing `folders` and one item `n + 1`;
    /// without `_embedded` when `complete` is false (cut by the depth limit).
    fn tree_folder(n: u128, parent: u128, folders: Vec<Llsd>, complete: bool) -> Llsd {
        let mut doc = llsd_map! { "category_id" => id(n), "parent_id" => id(parent), "agent_id" => id(2),
        "name" => format!("Folder {n}"), "type_default" => -1, "version" => 7 };
        if complete {
            let mut item = record(n + 1, false, 1, false);
            item.insert("parent_id", id(n));
            let mut items = Llsd::new_map();
            items.insert(id(n + 1).to_string(), item);
            let mut categories = Llsd::new_map();
            for folder in folders {
                categories.insert(folder["category_id"].as_uuid().to_string(), folder);
            }
            doc.insert(
                "_embedded",
                llsd_map! { "categories" => categories, "items" => items, "links" => Llsd::new_map() },
            );
        }
        doc
    }

    fn tree() -> Llsd {
        let nested = tree_folder(30, 10, Vec::new(), true);
        let first = tree_folder(10, 1, vec![nested], true);
        let cut = tree_folder(20, 1, Vec::new(), false);
        tree_folder(1, 100, vec![first, cut], true)
    }

    fn fetched(contents: &[FolderContents]) -> Vec<Uuid> {
        contents.iter().map(|c| c.folder_id).collect()
    }

    #[test]
    fn recursive_ais_reads_snapshot_every_complete_folder_parents_first() {
        let contents = parse_tree_reply(&tree(), id(2), id(1), &[], MAX_FOLDER_DEPTH).expect("recursive reply");
        // the folder cut short stays listed by its parent, unfetched
        assert_eq!(fetched(&contents), [id(1), id(10), id(30)]);
        assert!(contents[0].folders.iter().any(|f| f.id == id(20)));
        assert_eq!(contents[2].items[0].id, id(31));
        // AISUpdate::parseCategory: no snapshot below the requested depth
        let shallow = parse_tree_reply(&tree(), id(2), id(1), &[], 1).expect("depth 1");
        assert_eq!(fetched(&shallow), [id(1), id(10)]);
        let mut broken = tree();
        broken.insert("agent_id", id(99));
        assert!(parse_tree_reply(&broken, id(2), id(1), &[], MAX_FOLDER_DEPTH).is_err());
    }

    #[test]
    fn subset_ais_reads_keep_the_requested_subtrees_only() {
        let contents = parse_tree_reply(&tree(), id(2), id(1), &[id(10), id(20)], MAX_FOLDER_DEPTH).expect("subset");
        // the parent is no snapshot of its own; 20 came back incomplete
        assert_eq!(fetched(&contents), [id(10), id(30)]);
        assert!(parse_tree_reply(&tree(), id(2), id(99), &[id(10)], MAX_FOLDER_DEPTH).is_err());
    }

    #[tokio::test]
    async fn a_deleted_item_does_not_fail_the_items_read_with_it() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local test server");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .expect("request timeout")
                    .expect("request");
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let n = socket.read(&mut buffer).await.expect("read");
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8_lossy(&bytes).into_owned();
                let (status, body) = if request.contains(&id(3).to_string()) {
                    ("200 OK", aurora_llsd::to_xml(&record(3, false, 5, false)))
                } else if request.contains(&id(5).to_string()) {
                    ("404 Not Found", Vec::new())
                } else {
                    ("500 Internal Server Error", Vec::new())
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.expect("header");
                socket.write_all(&body).await.expect("body");
                crate::test_http::finish(socket);
            }
        });
        let reads = read_items(&reqwest::Client::new(), &url, id(2), &[id(3), id(5), id(6)]).await;
        server.await.expect("server");
        assert_eq!(reads.items.iter().map(|it| it.id).collect::<Vec<_>>(), [id(3)]);
        assert_eq!(reads.missing, [id(5)], "404: a broken link's original");
        assert_eq!(reads.failed, [id(6)], "a server error proves nothing");
        assert!(
            fetch_items(&reqwest::Client::new(), "http://127.0.0.1:9", id(2), &[id(3)])
                .await
                .is_err(),
            "the all-or-nothing read still fails"
        );
    }
}
