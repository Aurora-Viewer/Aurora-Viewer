//! Inventory mutations following AISAPI::{CreateInventory, UpdateItem,
//! UpdateCategory, RemoveItem, RemoveCategory, PurgeDescendents} and
//! copy_inventory_item (Firestorm indra/newview, originally LGPL 2.1).

use super::{FolderContents, InvItem};
use aurora_llsd::{Llsd, llsd_map};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CopyItem {
    pub item: Uuid,
    pub owner: Uuid,
    pub parent: Uuid,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Operation {
    Folder {
        id: Uuid,
        parent: Uuid,
        name: String,
        type_default: i32,
    },
    Patch {
        id: Uuid,
        folder: bool,
        body: Llsd,
    },
    Links {
        parent: Uuid,
        links: Vec<crate::outfits::OutfitLink>,
    },
    Delete {
        id: Uuid,
        folder: bool,
    },
    Purge(Uuid),
}

#[derive(Debug, Clone, Default)]
pub struct Mutation {
    pub operations: Vec<Operation>,
    pub copies: Vec<CopyItem>,
    pub refresh: Vec<Uuid>,
    pub removed: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub contents: Vec<FolderContents>,
    pub copies: Vec<CopyItem>,
    pub removed: Vec<Uuid>,
    pub created: HashMap<Uuid, Uuid>,
}

pub fn metadata_favorite(on: bool) -> Llsd {
    llsd_map! { "favorite" => if on { llsd_map! { "toggled" => true } } else { Llsd::default() } }
}

pub fn metadata_thumbnail(id: Uuid) -> Llsd {
    llsd_map! { "thumbnail" => if id.is_nil() { Llsd::default() } else { llsd_map! { "asset_id" => id.to_string() } } }
}

/// Sequential operations preserve parent-before-child ordering. Client UUIDs
/// for new folders are replaced with the UUIDs actually assigned by AIS.
pub async fn mutate(http: &reqwest::Client, cap: &str, fetch_cap: &str, owner: Uuid, change: Mutation) -> Result<Reply, String> {
    if change.operations.len() > 2048 || change.copies.len() > 2048 || change.refresh.is_empty() {
        return Err("La sélection est vide ou trop volumineuse.".into());
    }
    let cap = cap.trim_end_matches('/');
    let mut created = HashMap::new();
    let resolve = |ids: &HashMap<Uuid, Uuid>, id: Uuid| ids.get(&id).copied().unwrap_or(id);
    for op in &change.operations {
        match op {
            Operation::Folder {
                id,
                parent,
                name,
                type_default,
            } => {
                let parent = resolve(&created, *parent);
                let body = llsd_map! { "categories" => Llsd::Array(vec![llsd_map! {
                    "category_id" => Uuid::nil(), "parent_id" => parent, "name" => name.clone(), "type_default" => *type_default,
                }]) };
                let reply = crate::outfits::send(http.post(format!("{cap}/category/{parent}?tid={}", Uuid::new_v4())), &body).await?;
                let actual = reply["_created_categories"].at(0).as_uuid();
                if actual.is_nil() {
                    return Err("Le serveur n’a pas confirmé le nouveau dossier. Rechargez avant de réessayer.".into());
                }
                created.insert(*id, actual);
            }
            Operation::Patch { id, folder, body } => {
                let id = resolve(&created, *id);
                let mut body = body.clone();
                if body.has("parent_id") {
                    body.insert("parent_id", Llsd::Uuid(resolve(&created, body["parent_id"].as_uuid())));
                }
                let kind = if *folder { "category" } else { "item" };
                crate::outfits::send(http.patch(format!("{cap}/{kind}/{id}")), &body).await?;
            }
            Operation::Links { parent, links } => {
                let parent = resolve(&created, *parent);
                let body = llsd_map! { "links" => crate::outfits::links_body(links) };
                let response = crate::outfits::send(http.post(format!("{cap}/category/{parent}?tid={}", Uuid::new_v4())), &body).await?;
                if response["_created_items"].as_array().len() < links.len()
                    || response["_created_items"].as_array().iter().any(|id| id.as_uuid().is_nil())
                {
                    return Err("Le serveur n’a pas confirmé la création des liens. Aucun ancien lien ne sera supprimé.".into());
                }
            }
            Operation::Delete { id, folder } => {
                let kind = if *folder { "category" } else { "item" };
                crate::outfits::send(http.delete(format!("{cap}/{kind}/{id}")), &Llsd::new_map()).await?;
            }
            Operation::Purge(id) => {
                crate::outfits::send(http.delete(format!("{cap}/category/{id}/children")), &Llsd::new_map()).await?;
            }
        }
    }
    let mut refresh: Vec<_> = change.refresh.into_iter().map(|id| resolve(&created, id)).collect();
    refresh.sort();
    refresh.dedup();
    let mut contents = Vec::new();
    for chunk in refresh.chunks(16) {
        let doc = crate::caps::post_llsd(http, fetch_cap, &super::fetch_request_body(chunk, owner))
            .await
            .map_err(|_| "Modification envoyée, mais relecture impossible. Rechargez l’inventaire.".to_owned())?;
        let fetched = super::parse_fetch_response(&doc);
        if chunk.iter().any(|id| !fetched.iter().any(|c| c.folder_id == *id && c.version >= 0)) {
            return Err("Relecture incomplète. Rechargez l’inventaire avant de réessayer.".into());
        }
        contents.extend(fetched);
    }
    if !confirmed(&change.operations, &created, &contents) {
        return Err("Le serveur n’a pas confirmé toutes les modifications. Les dossiers seront rechargés.".into());
    }
    // Parent metadata must precede its descendants (including new folders).
    contents.sort_by_key(|c| created.values().any(|id| *id == c.folder_id));
    let copies = change
        .copies
        .into_iter()
        .map(|mut c| {
            c.parent = resolve(&created, c.parent);
            c
        })
        .collect();
    Ok(Reply {
        contents,
        copies,
        removed: change.removed,
        created,
    })
}

fn confirmed(operations: &[Operation], created: &HashMap<Uuid, Uuid>, contents: &[FolderContents]) -> bool {
    let resolve = |id: Uuid| created.get(&id).copied().unwrap_or(id);
    let item = |id| contents.iter().flat_map(|c| &c.items).find(|it| it.id == id);
    let folder = |id| contents.iter().flat_map(|c| &c.folders).find(|f| f.id == id);
    // An element can move twice in one operation (ungroup / marketplace).
    // Verify the final value of each patched field rather than an intermediate one.
    let mut patches = HashMap::<(Uuid, bool), aurora_llsd::Map>::new();
    for op in operations {
        match op {
            Operation::Patch { id, folder, body } => {
                let Some(fields) = body.as_map() else {
                    return false;
                };
                patches.entry((resolve(*id), *folder)).or_default().extend(fields.clone());
            }
            Operation::Folder {
                id,
                parent,
                name,
                type_default,
            } => {
                if !folder(resolve(*id)).is_some_and(|f| f.parent == resolve(*parent) && f.name == *name && f.type_default == *type_default)
                {
                    return false;
                }
            }
            Operation::Links { parent, links } => {
                if !links.iter().all(|l| {
                    contents
                        .iter()
                        .filter(|c| c.folder_id == resolve(*parent))
                        .flat_map(|c| &c.items)
                        .any(|it| it.asset_id == l.target && it.asset_type == if l.folder { 25 } else { 24 } && it.name == l.name)
                }) {
                    return false;
                }
            }
            Operation::Delete { id, .. } => {
                if item(*id).is_some() || folder(*id).is_some() {
                    return false;
                }
            }
            Operation::Purge(id) => {
                if !contents
                    .iter()
                    .any(|c| c.folder_id == *id && c.items.is_empty() && c.folders.is_empty())
                {
                    return false;
                }
            }
        }
    }
    for ((id, is_folder), fields) in patches {
        let body = Llsd::Map(fields);
        let observed = if is_folder {
            let Some(f) = folder(id) else {
                return false;
            };
            llsd_map! { "name" => f.name.clone(), "parent_id" => f.parent, "favorite" => metadata_favorite(f.favorite)["favorite"].clone(),
            "thumbnail" => metadata_thumbnail(f.thumbnail)["thumbnail"].clone() }
        } else {
            let Some(it) = item(id) else {
                return false;
            };
            llsd_map! { "name" => it.name.clone(), "desc" => it.desc.clone(), "parent_id" => it.parent, "linked_id" => it.asset_id,
            "favorite" => metadata_favorite(it.favorite)["favorite"].clone(), "thumbnail" => metadata_thumbnail(it.thumbnail)["thumbnail"].clone(),
            "permissions" => llsd_map! { "next_owner_mask" => it.next_owner_mask as i32, "group_mask" => it.group_mask as i32, "everyone_mask" => it.everyone_mask as i32 },
            "sale_info" => llsd_map! { "sale_type" => i32::from(it.sale_type), "sale_price" => it.sale_price } }
        };
        for (key, value) in body.as_map().into_iter().flatten() {
            if key == "parent_id" {
                if observed[key.as_str()].as_uuid() != resolve(value.as_uuid()) {
                    return false;
                }
            } else if key == "permissions" {
                if !value
                    .as_map()
                    .into_iter()
                    .flatten()
                    .all(|(key, value)| observed["permissions"][key.as_str()] == *value)
                {
                    return false;
                }
            } else if observed[key.as_str()] != *value {
                return false;
            }
        }
    }
    true
}

/// Names and types of the Firestorm create menus (LLWearableType / LLSettingsType).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewItem {
    Script,
    Note,
    Gesture,
    Material,
    Wearable(u8),
    Settings(u8),
}

impl NewItem {
    pub fn types(self) -> (i8, i8, u8) {
        match self {
            Self::Script => (10, 10, 255),
            Self::Note => (7, 7, 255),
            Self::Gesture => (21, 20, 255),
            Self::Material => (57, 26, 255),
            Self::Wearable(kind) => (if kind < 4 { 13 } else { 5 }, 18, kind),
            Self::Settings(kind) => (56, 25, kind),
        }
    }
}

pub fn item_from_udp(it: &aurora_msg::msgs::update_create_inventory_item::InventoryData) -> InvItem {
    InvItem {
        id: it.item_id,
        parent: it.folder_id,
        asset_id: it.asset_id,
        name: aurora_msg::field_str(&it.name),
        desc: aurora_msg::field_str(&it.description),
        asset_type: i32::from(it.type_),
        inv_type: i32::from(it.inv_type),
        flags: it.flags,
        creator: it.creator_id,
        owner: it.owner_id,
        created_at: i64::from(it.creation_date),
        base_mask: it.base_mask,
        owner_mask: it.owner_mask,
        group_mask: it.group_mask,
        everyone_mask: it.everyone_mask,
        next_owner_mask: it.next_owner_mask,
        group_id: it.group_id,
        group_owned: it.group_owned,
        sale_type: it.sale_type,
        sale_price: it.sale_price,
        ..Default::default()
    }
}

/// LLInventoryItem::getCRC32 (a wrapping checksum, not a polynomial CRC).
pub fn checksum(it: &InvItem) -> u32 {
    fn uuid_sum(id: Uuid) -> u32 {
        id.as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .fold(0u32, |sum, b| sum.wrapping_add(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    }
    [
        it.id,
        it.parent,
        it.creator,
        it.owner,
        it.last_owner,
        it.group_id,
        it.asset_id,
        it.thumbnail,
    ]
    .into_iter()
    .map(uuid_sum)
    .chain([
        it.base_mask,
        it.owner_mask,
        it.everyone_mask,
        it.group_mask,
        it.asset_type as u32,
        it.inv_type as u32,
        it.flags,
        it.sale_price as u32,
        u32::from(it.sale_type).wrapping_mul(0x07073096),
        it.created_at as u32,
    ])
    .fold(0u32, u32::wrapping_add)
}

pub async fn preview(http: &reqwest::Client, cap: &str, kind: &str, asset: Uuid) -> Result<Vec<u8>, String> {
    if kind.is_empty() || asset.is_nil() {
        return Err("Cet élément n’a pas de contenu consultable.".into());
    }
    let url = format!("{cap}{}{kind}={asset}", if cap.contains('?') { "&" } else { "?" });
    let mut response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Le contenu n’a pas pu être chargé.".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("Le contenu est indisponible (HTTP {}).", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Lecture interrompue.".to_owned())? {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err("Le contenu dépasse la taille maximale de prévisualisation.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// LLPreviewNotecard::saveIfNeeded / LLPreviewLSL::saveIfNeeded:
/// capability negotiation, binary POST to the returned uploader, then re-read.
pub async fn save_content(
    http: &reqwest::Client,
    cap: &str,
    fetch: &str,
    owner: Uuid,
    item: Uuid,
    kind: i32,
    data: Vec<u8>,
) -> Result<InvItem, String> {
    if data.len() > 1024 * 1024 {
        return Err("Le document est trop volumineux.".into());
    }
    let mut body = llsd_map! { "item_id" => item };
    if kind == 10 {
        body.insert("target", Llsd::String("mono".into()));
    }
    let answer = crate::caps::post_llsd(http, cap, &body)
        .await
        .map_err(|_| "L’enregistrement a été refusé.".to_owned())?;
    if answer["state"].as_str() != "upload" {
        return Err("Le serveur n’a pas proposé de chargement.".into());
    }
    let url = answer["uploader"].to_string_value();
    let response = http
        .post(url)
        .header("Content-Type", "application/octet-stream")
        .body(data)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Enregistrement interrompu.".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("Enregistrement refusé (HTTP {}).", response.status().as_u16()));
    }
    let bytes = response.bytes().await.map_err(|_| "Réponse interrompue.".to_owned())?;
    let ack = aurora_llsd::from_xml(&bytes).map_err(|_| "Réponse illisible.".to_owned())?;
    if ack["state"].as_str() != "complete" {
        return Err("Le serveur n’a pas confirmé l’enregistrement.".into());
    }
    if kind == 10 && !ack["compiled"].as_bool() {
        return Err("Le script contient des erreurs de compilation. Vérifiez le code et réessayez.".into());
    }
    let doc = crate::caps::post_llsd(http, fetch, &super::fetch_items_body(&[item], owner))
        .await
        .map_err(|_| "Enregistré, mais relecture impossible. Rechargez l’inventaire.".to_owned())?;
    super::parse_items_response(&doc)
        .into_iter()
        .find(|it| it.id == item)
        .ok_or_else(|| "L’élément enregistré n’a pas été retrouvé.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outfits::tests::server;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    #[tokio::test]
    async fn nested_folders_and_deferred_copies_use_the_server_assigned_ids() {
        let doc = llsd_map! { "folders" => Llsd::Array(vec![
            llsd_map! { "folder_id" => id(1), "version" => 2, "categories" => Llsd::Array(vec![llsd_map! { "category_id" => id(200), "parent_id" => id(1), "name" => "Dossier", "type_default" => -1 }]) },
            llsd_map! { "folder_id" => id(200), "version" => 1, "categories" => Llsd::Array(vec![llsd_map! { "category_id" => id(201), "parent_id" => id(200), "name" => "Enfant", "type_default" => -1 }]) },
            llsd_map! { "folder_id" => id(201), "version" => 1 }
        ]) };
        let (url, task) = server(vec![
            (200, llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) }),
            (200, llsd_map! { "_created_categories" => Llsd::Array(vec![id(201).into()]) }),
            (200, doc),
        ])
        .await;
        let result = mutate(
            &reqwest::Client::new(),
            &url,
            &format!("{url}/fetch"),
            id(2),
            Mutation {
                operations: vec![
                    Operation::Folder {
                        id: id(100),
                        parent: id(1),
                        name: "Dossier".into(),
                        type_default: -1,
                    },
                    Operation::Folder {
                        id: id(101),
                        parent: id(100),
                        name: "Enfant".into(),
                        type_default: -1,
                    },
                ],
                copies: vec![CopyItem {
                    item: id(3),
                    owner: id(2),
                    parent: id(101),
                    name: String::new(),
                }],
                refresh: vec![id(1), id(100), id(101)],
                ..Default::default()
            },
        )
        .await
        .expect("confirmed hierarchy");
        assert_eq!(result.copies[0].parent, id(201));
        let requests = task.await.expect("requests");
        assert!(requests[1].0.contains(&format!("/category/{}?", id(200))));
        assert_eq!(requests[1].1["categories"].at(0)["parent_id"].as_uuid(), id(200));
    }
    #[tokio::test]
    async fn a_successful_patch_with_stale_readback_is_not_reported_as_success() {
        let doc = llsd_map! { "folders" => Llsd::Array(vec![llsd_map! { "folder_id" => id(1), "version" => 2, "items" => Llsd::Array(vec![llsd_map! { "item_id" => id(3), "parent_id" => id(1), "name" => "Ancien" }]) }]) };
        let (url, task) = server(vec![(200, Llsd::new_map()), (200, doc)]).await;
        let result = mutate(
            &reqwest::Client::new(),
            &url,
            &url,
            id(2),
            Mutation {
                operations: vec![Operation::Patch {
                    id: id(3),
                    folder: false,
                    body: llsd_map! { "name" => "Nouveau" },
                }],
                refresh: vec![id(1)],
                ..Default::default()
            },
        )
        .await;
        assert!(result.is_err());
        task.await.expect("requests");
    }
    #[tokio::test]
    async fn a_server_refusal_never_sends_deferred_copies() {
        let (url, task) = server(vec![(403, Llsd::new_map())]).await;
        assert!(
            mutate(
                &reqwest::Client::new(),
                &url,
                &url,
                id(2),
                Mutation {
                    operations: vec![Operation::Delete { id: id(3), folder: false }],
                    refresh: vec![id(1)],
                    copies: vec![CopyItem {
                        item: id(4),
                        owner: id(2),
                        parent: id(1),
                        name: String::new()
                    }],
                    ..Default::default()
                }
            )
            .await
            .is_err()
        );
        assert_eq!(task.await.expect("requests").len(), 1);
    }
    #[test]
    fn checksum_wraps_and_includes_sale_permissions_asset_and_thumbnail() {
        let item = InvItem {
            id: id(1),
            parent: id(2),
            asset_id: id(3),
            thumbnail: id(4),
            owner_mask: u32::MAX,
            asset_type: 6,
            inv_type: 6,
            sale_type: 2,
            sale_price: 10,
            created_at: 20,
            ..Default::default()
        };
        assert_eq!(
            checksum(&item),
            0x01000000u32
                .wrapping_mul(10)
                .wrapping_add(u32::MAX)
                .wrapping_add(42)
                .wrapping_add(2 * 0x07073096)
        );
    }
    #[tokio::test]
    async fn replacing_a_link_creates_it_before_deleting_the_old_one() {
        let item = llsd_map! { "item_id" => id(20), "parent_id" => id(1), "type" => 24, "linked_id" => id(5), "asset_id" => id(5), "inv_type" => 18, "name" => "Nouveau" };
        let doc = llsd_map! { "folders" => Llsd::Array(vec![llsd_map! { "folder_id" => id(1), "version" => 2, "items" => Llsd::Array(vec![item]) }]) };
        let (url, task) = server(vec![
            (200, llsd_map! { "_created_items" => Llsd::Array(vec![id(20).into()]) }),
            (200, Llsd::new_map()),
            (200, doc),
        ])
        .await;
        let change = Mutation {
            operations: vec![
                Operation::Links {
                    parent: id(1),
                    links: vec![crate::outfits::OutfitLink {
                        target: id(5),
                        name: "Nouveau".into(),
                        desc: "@400".into(),
                        folder: false,
                        inv_type: 18,
                    }],
                },
                Operation::Delete { id: id(10), folder: false },
            ],
            refresh: vec![id(1)],
            removed: vec![id(10)],
            ..Default::default()
        };
        mutate(&reqwest::Client::new(), &url, &url, id(2), change)
            .await
            .expect("replaced link");
        let requests = task.await.expect("requests");
        assert!(requests[0].0.starts_with("POST "));
        assert!(requests[1].0.starts_with("DELETE "));
        assert_eq!(requests[0].1["links"].at(0)["desc"].as_str(), "@400");
    }
    #[tokio::test]
    async fn an_unconfirmed_new_link_never_deletes_the_existing_link() {
        let (url, task) = server(vec![(200, Llsd::new_map())]).await;
        let change = Mutation {
            operations: vec![
                Operation::Links {
                    parent: id(1),
                    links: vec![crate::outfits::OutfitLink {
                        target: id(5),
                        name: "Nouveau".into(),
                        desc: String::new(),
                        folder: false,
                        inv_type: 18,
                    }],
                },
                Operation::Delete { id: id(10), folder: false },
            ],
            refresh: vec![id(1)],
            ..Default::default()
        };
        assert!(mutate(&reqwest::Client::new(), &url, &url, id(2), change).await.is_err());
        assert_eq!(task.await.expect("requests").len(), 1);
    }
}
