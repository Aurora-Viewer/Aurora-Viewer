//! AIS inventory reads following AISAPI::FetchCategoryChildren / FetchItem and
//! AISUpdate::parseEmbedded (indra/newview/llaisapi.cpp).

use super::{FolderContents, InvFolder, InvItem, folder_from_llsd, item_from_llsd};
use aurora_llsd::Llsd;
use uuid::Uuid;

fn favorite(record: &Llsd) -> Result<bool, &'static str> {
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

fn item(record: &Llsd, owner: Uuid) -> Result<InvItem, &'static str> {
    if record["parent_id"].as_uuid().is_nil() || !record.has("name") || !record.has("type") || !record.has("inv_type") {
        return Err("incomplete AIS item");
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
                if key.parse::<Uuid>().ok() != Some(child.id) || child.parent != id {
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

async fn get(http: &reqwest::Client, url: String) -> Result<Llsd, &'static str> {
    crate::caps::get_llsd_as(http, &url, crate::probe::HttpKind::Inventory)
        .await
        .map_err(|_| "AIS inventory read failed")
}

/// Bounded parallel reads, outside the viewer thread. Never substitute a
/// legacy listing: it may omit the favorites of every sibling in this folder.
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

pub async fn fetch_items(http: &reqwest::Client, cap: &str, owner: Uuid, ids: &[Uuid]) -> Result<Vec<InvItem>, &'static str> {
    let mut items = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(16) {
        let mut tasks = tokio::task::JoinSet::new();
        for id in chunk.iter().copied() {
            let http = http.clone();
            let url = format!("{}/item/{id}", cap.trim_end_matches('/'));
            tasks.spawn(async move {
                let item = item(&get(&http, url).await?, owner)?;
                if item.id != id {
                    return Err("mismatched AIS item");
                }
                Ok(item)
            });
        }
        while let Some(result) = tasks.join_next().await {
            items.push(result.map_err(|_| "AIS inventory task failed")??);
        }
    }
    Ok(items)
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
}
