//! Inventory mutations following AISAPI::{CreateInventory, UpdateItem,
//! UpdateCategory, RemoveItem, RemoveCategory, PurgeDescendents} and
//! copy_inventory_item (Firestorm indra/newview, originally LGPL 2.1).
//! Parent changes use LLViewerInventory{Item,Category}::updateParentOnServer
//! (llviewerinventory.cpp): reliable UDP, never an AIS parent_id PATCH.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InventoryMove {
    pub id: Uuid,
    pub parent: Uuid,
    pub folder: bool,
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
    /// Interactive inventory commands can display AIS-created folders before
    /// fetching their contents. Dependent construction jobs keep readback.
    pub defer_folder_contents: bool,
}

impl Mutation {
    pub fn move_only(&self) -> bool {
        !self.operations.is_empty() && self.operations.len() <= 2048 && !self.refresh.is_empty() && self.copies.is_empty() && self.operations.iter().all(|op| {
            matches!(op, Operation::Patch { body, .. } if body.as_map().is_some_and(|m| m.len() == 1 && m.contains_key("parent_id")))
        })
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub contents: Vec<FolderContents>,
    /// Empty folders whose creation was acknowledged by AIS.
    pub folders: Vec<super::InvFolder>,
    /// Reliable moves have been queued; server updates reconcile them later.
    pub moves_queued: bool,
    /// Folders to reload if a mutation partially fails, including empty children.
    pub refresh: Vec<Uuid>,
    pub copies: Vec<CopyItem>,
    pub removed: Vec<Uuid>,
    pub created: HashMap<Uuid, Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Updating,
    Copying,
    Verifying,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub completed: usize,
    pub total: usize,
}

pub fn metadata_favorite(on: bool) -> Llsd {
    llsd_map! { "favorite" => if on { llsd_map! { "toggled" => true } } else { Llsd::default() } }
}

pub fn metadata_thumbnail(id: Uuid) -> Llsd {
    llsd_map! { "thumbnail" => if id.is_nil() { Llsd::default() } else { llsd_map! { "asset_id" => id.to_string() } } }
}

/// LLInventoryObject::correctInventoryName (llinventory.cpp): inventory
/// names are ASCII bytes, without pipes, trimmed and limited to 63 bytes.
fn folder_name(name: &str) -> String {
    let ascii: String = name
        .bytes()
        .map(|b| {
            if b == b'|' || ((b as i8) < 32 && b != b'\n') {
                ' '
            } else {
                char::from(b)
            }
        })
        .collect();
    ascii.trim().chars().take(63).collect()
}

/// Sequential operations preserve parent-before-child ordering. Client UUIDs
/// for new folders are replaced with the UUIDs actually assigned by AIS.
pub async fn mutate(http: &reqwest::Client, cap: &str, fetch_cap: &str, owner: Uuid, change: Mutation) -> Result<Reply, String> {
    mutate_with_moves(http, cap, fetch_cap, owner, change, |_| async {
        Err(aurora_i18n::tr!("inventory-move-connection-unavailable").into())
    })
    .await
}

pub(crate) async fn mutate_with_moves<F, Fut>(
    http: &reqwest::Client,
    cap: &str,
    fetch_cap: &str,
    owner: Uuid,
    change: Mutation,
    mut send_move: F,
) -> Result<Reply, String>
where
    F: FnMut(InventoryMove) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    mutate_with_progress(http, cap, fetch_cap, owner, change, &mut send_move, |_| {}).await
}

pub(crate) async fn mutate_with_progress<F, Fut, P>(
    http: &reqwest::Client,
    cap: &str,
    fetch_cap: &str,
    owner: Uuid,
    change: Mutation,
    mut send_move: F,
    mut progress: P,
) -> Result<Reply, String>
where
    F: FnMut(InventoryMove) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
    P: FnMut(Progress),
{
    if change.operations.len() > 2048 || change.copies.len() > 2048 || change.refresh.is_empty() {
        return Err(aurora_i18n::tr!("inventory-operations-the-selection-is-empty-or-too-large").into());
    }
    let cap = cap.trim_end_matches('/');
    let mut created = HashMap::new();
    let mut folders = Vec::new();
    let mut moved = false;
    let resolve = |ids: &HashMap<Uuid, Uuid>, id: Uuid| ids.get(&id).copied().unwrap_or(id);
    for (index, op) in change.operations.iter().enumerate() {
        progress(Progress {
            phase: Phase::Updating,
            completed: index,
            total: change.operations.len(),
        });
        match op {
            Operation::Folder {
                id,
                parent,
                name,
                type_default,
            } => {
                let parent = resolve(&created, *parent);
                let body = llsd_map! { "categories" => Llsd::Array(vec![llsd_map! {
                    "category_id" => Uuid::nil(), "parent_id" => parent, "name" => folder_name(name), "type_default" => *type_default,
                }]) };
                let reply = crate::outfits::send(http.post(format!("{cap}/category/{parent}?tid={}", Uuid::new_v4())), &body).await?;
                let actual = reply["_created_categories"].at(0).as_uuid();
                if actual.is_nil() {
                    return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-new-folder-reload").into());
                }
                created.insert(*id, actual);
                folders.push(super::InvFolder {
                    id: actual,
                    parent,
                    name: folder_name(name),
                    type_default: *type_default,
                    version: 1,
                    ..Default::default()
                });
            }
            Operation::Patch { id, folder, body } => {
                let id = resolve(&created, *id);
                let Some(mut fields) = body.as_map().cloned() else {
                    return Err(aurora_i18n::tr!("inventory-operations-the-selection-is-empty-or-too-large").into());
                };
                if let Some(parent) = fields.remove("parent_id") {
                    let parent = resolve(&created, parent.as_uuid());
                    if id.is_nil() || parent.is_nil() {
                        return Err(aurora_i18n::tr!("inventory-move-invalid-target").into());
                    }
                    send_move(InventoryMove {
                        id,
                        parent,
                        folder: *folder,
                    })
                    .await?;
                    moved = true;
                }
                if !fields.is_empty() {
                    let kind = if *folder { "category" } else { "item" };
                    crate::outfits::send(http.patch(format!("{cap}/{kind}/{id}")), &Llsd::Map(fields)).await?;
                }
            }
            Operation::Links { parent, links } => {
                let parent = resolve(&created, *parent);
                let body = llsd_map! { "links" => crate::outfits::links_body(links) };
                let response = crate::outfits::send(http.post(format!("{cap}/category/{parent}?tid={}", Uuid::new_v4())), &body).await?;
                if response["_created_items"].as_array().len() < links.len()
                    || response["_created_items"].as_array().iter().any(|id| id.as_uuid().is_nil())
                {
                    return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-link-creation-no-old").into());
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
    let copies: Vec<_> = change
        .copies
        .into_iter()
        .map(|mut c| {
            c.parent = resolve(&created, c.parent);
            c
        })
        .collect();
    if (change.defer_folder_contents || !copies.is_empty())
        && (!copies.is_empty() || !change.operations.is_empty())
        && change.operations.iter().all(|op| matches!(op, Operation::Folder { .. }))
    {
        // AIS already returned every new folder ID. Item callbacks populate
        // the folders; neither an initial nor a final snapshot gates the copy.
        return Ok(Reply {
            contents: Vec::new(),
            folders,
            moves_queued: false,
            refresh,
            copies,
            removed: change.removed,
            created,
        });
    }
    progress(Progress {
        phase: Phase::Verifying,
        completed: 0,
        total: refresh.len(),
    });
    let mut contents = Vec::new();
    let favorite_changed = change
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Patch { body, .. } if body.has("favorite")));
    let retries = if moved || !created.is_empty() || favorite_changed { 5 } else { 0 };
    for attempt in 0..=retries {
        contents.clear();
        let mut complete = true;
        for chunk in refresh.chunks(16) {
            let fetched = if favorite_changed {
                // Every sibling's metadata must come from AIS. Overlaying only
                // this mutation onto a legacy listing erases earlier favorites.
                super::ais::fetch_folders(http, cap, owner, chunk)
                    .await
                    .map_err(|_| aurora_i18n::tr!("inventory-operations-change-sent-but-could-not-read-it-back-reload").to_owned())?
            } else {
                let doc = crate::caps::post_llsd(http, fetch_cap, &super::fetch_request_body(chunk, owner))
                    .await
                    .map_err(|_| aurora_i18n::tr!("inventory-operations-change-sent-but-could-not-read-it-back-reload").to_owned())?;
                super::parse_fetch_response(&doc)
            };
            complete &= chunk.iter().all(|id| fetched.iter().any(|c| c.folder_id == *id && c.version >= 0));
            contents.extend(fetched);
        }
        if complete && confirmed(&change.operations, &created, &contents) {
            break;
        }
        if attempt == retries {
            for operation in &change.operations {
                if let Operation::Patch { id, folder, body } = operation
                    && body.has("favorite")
                {
                    let id = resolve(&created, *id);
                    let observed = if *folder {
                        contents.iter().flat_map(|c| &c.folders).find(|f| f.id == id).map(|f| f.favorite)
                    } else {
                        contents.iter().flat_map(|c| &c.items).find(|it| it.id == id).map(|it| it.favorite)
                    };
                    log::warn!(
                        "inventory favorite confirmation failed: folder={folder}, expected={}, observed={observed:?}",
                        body["favorite"]["toggled"].as_bool()
                    );
                }
            }
            return Err(if complete {
                aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-every-change-the-folders").into()
            } else {
                aurora_i18n::tr!("inventory-operations-read-back-incomplete-reload-inventory-before-trying-again").into()
            });
        }
        // AIS creation/favorites and reliable UDP moves can become visible to
        // the fetch service after the first HTTP read. Retry reads, never writes.
        tokio::time::sleep(std::time::Duration::from_millis(200 << attempt.min(3))).await;
    }
    // Parent metadata must precede its descendants (including new folders).
    sort_contents_parent_first(&mut contents);
    Ok(Reply {
        contents,
        folders,
        moves_queued: false,
        refresh,
        copies,
        removed: change.removed,
        created,
    })
}

fn sort_contents_parent_first(contents: &mut [FolderContents]) {
    // HTTP responses (including separate chunks) can list a deep descendant
    // before the snapshot that introduces its folder to the inventory model.
    let parents: HashMap<_, _> = contents
        .iter()
        .flat_map(|c| c.folders.iter().filter(|f| f.id != c.folder_id).map(|f| (f.id, f.parent)))
        .collect();
    contents.sort_by_cached_key(|c| {
        let mut id = c.folder_id;
        let mut depth = 0;
        // Bound traversal so malformed network parent cycles cannot hang it.
        for _ in 0..=parents.len() {
            let Some(parent) = parents.get(&id) else { break };
            depth += 1;
            id = *parent;
        }
        depth
    });
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
                if !folder(resolve(*id))
                    .is_some_and(|f| f.parent == resolve(*parent) && f.name == folder_name(name) && f.type_default == *type_default)
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
    /// AT_LANDMARK / IT_LANDMARK: the server writes the asset from the
    /// agent's position (LLLandmarkActions::createLandmarkHere).
    Landmark,
}

impl NewItem {
    pub fn types(self) -> (i8, i8, u8) {
        match self {
            Self::Script => (10, 10, 255),
            Self::Landmark => (3, 3, 255),
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
    checksum_in_folder(it, it.parent)
}

/// Rez can override the wire folder without mutating inventory metadata.
pub(crate) fn checksum_in_folder(it: &InvItem, folder: Uuid) -> u32 {
    fn uuid_sum(id: Uuid) -> u32 {
        id.as_bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .fold(0u32, |sum, b| sum.wrapping_add(u32::from_le_bytes([b[0], b[1], b[2], b[3]])))
    }
    [
        it.id,
        folder,
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
        return Err(aurora_i18n::tr!("inventory-operations-this-item-has-no-viewable-contents").into());
    }
    let url = format!("{cap}{}{kind}={asset}", if cap.contains('?') { "&" } else { "?" });
    let mut response = http
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-operations-could-not-load-contents").to_owned())?;
    if !response.status().is_success() {
        return Err(aurora_i18n::tr!(
            "inventory-operations-contents-unavailable-http",
            value0 = format!("{}", response.status().as_u16())
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-operations-read-interrupted").to_owned())?
    {
        if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
            return Err(aurora_i18n::tr!("inventory-operations-the-contents-exceed-the-maximum-preview-size").into());
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
        return Err(aurora_i18n::tr!("inventory-operations-the-document-is-too-large").into());
    }
    if kind == 7 {
        super::notecard::Notecard::decode(&data)?;
    }
    if kind == 21 {
        aurora_assets::gesture::Gesture::decode(&data).map_err(|_| aurora_i18n::tr!("gesture-invalid").to_owned())?;
    }
    if kind == 57 {
        aurora_assets::parse_material_asset(&data).map_err(|e| e.to_string())?;
    }
    let mut body = llsd_map! { "item_id" => item };
    if kind == 10 {
        body.insert("target", Llsd::String("mono".into()));
    }
    let ack = upload_content(http, cap, &body, data).await?;
    if kind == 10 && !ack["compiled"].as_bool() {
        return Err(aurora_i18n::tr!("inventory-operations-the-script-contains-compilation-errors-check-the-code-and").into());
    }
    let asset = ack["new_asset"].as_uuid();
    if matches!(kind, 7 | 21 | 56 | 57) && asset.is_nil() {
        return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save").into());
    }
    let saved_item = if ack["new_inventory_item"].as_uuid().is_nil() {
        item
    } else {
        ack["new_inventory_item"].as_uuid()
    };
    let doc = crate::caps::post_llsd(http, fetch, &super::fetch_items_body(&[saved_item], owner))
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-operations-saved-but-could-not-read-it-back-reload-inventory").to_owned())?;
    let saved = super::parse_items_response(&doc)
        .into_iter()
        .find(|it| it.id == saved_item)
        .ok_or_else(|| aurora_i18n::tr!("inventory-operations-the-saved-item-could-not-be-found").to_owned())?;
    if matches!(kind, 7 | 21 | 56 | 57) && (saved.asset_id != asset || saved.asset_type != kind || kind == 57 && saved.owner != owner) {
        return Err(if kind == 21 {
            aurora_i18n::tr!("gesture-save-unconfirmed")
        } else if matches!(kind, 56 | 57) {
            aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save")
        } else {
            aurora_i18n::tr!("notecard-save-unconfirmed")
        }
        .into());
    }
    Ok(saved)
}

/// LLBufferedAssetUploadInfo::generatePostBody / finishTaskUpload:
/// task_id and item_id identify the note in the object, not agent inventory.
pub async fn save_task_notecard(http: &reqwest::Client, cap: &str, object: Uuid, item: Uuid, data: Vec<u8>) -> Result<Uuid, String> {
    if object.is_nil() || item.is_nil() {
        return Err(aurora_i18n::tr!("notecard-edit-denied").into());
    }
    if data.len() > 1024 * 1024 {
        return Err(aurora_i18n::tr!("inventory-operations-the-document-is-too-large").into());
    }
    super::notecard::Notecard::decode(&data)?;
    let body = llsd_map! { "task_id" => object, "item_id" => item };
    let ack = upload_content(http, cap, &body, data).await?;
    let asset = ack["new_asset"].as_uuid();
    if asset.is_nil() {
        return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save").into());
    }
    Ok(asset)
}

async fn upload_content(http: &reqwest::Client, cap: &str, body: &Llsd, data: Vec<u8>) -> Result<Llsd, String> {
    let answer = crate::caps::post_llsd(http, cap, body)
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-operations-the-save-was-rejected").to_owned())?;
    if answer["state"].as_str() != "upload" {
        return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-offer-an-upload").into());
    }
    let url = answer["uploader"].to_string_value();
    let response = http
        .post(url)
        .header("Content-Type", "application/octet-stream")
        .body(data)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("inventory-operations-save-interrupted").to_owned())?;
    if !response.status().is_success() {
        return Err(aurora_i18n::tr!(
            "inventory-operations-save-rejected-http",
            value0 = format!("{}", response.status().as_u16())
        ));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| aurora_i18n::tr!("common-interrupted-response").to_owned())?;
    let ack = aurora_llsd::from_xml(&bytes).map_err(|_| aurora_i18n::tr!("inventory-operations-unreadable-response").to_owned())?;
    if ack["state"].as_str() != "complete" {
        return Err(aurora_i18n::tr!("inventory-operations-the-server-did-not-confirm-the-save").into());
    }
    Ok(ack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outfits::tests::server;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    async fn notecard_server(
        stale: bool,
        reject: bool,
        replace: bool,
        task_save: bool,
    ) -> (String, tokio::task::JoinHandle<Vec<(String, Vec<u8>)>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("test server");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let returned_item = if replace { id(30) } else { id(3) };
        let replies = vec![
            (200, llsd_map! { "state" => "upload", "uploader" => format!("{url}/upload") }),
            (
                if reject { 503 } else { 200 },
                llsd_map! { "state" => "complete", "new_asset" => if task_save && stale { Uuid::nil() } else { id(40) }, "new_inventory_item" => returned_item },
            ),
            (
                200,
                llsd_map! { "items" => Llsd::Array(vec![llsd_map! {
                    "item_id" => returned_item, "parent_id" => id(1), "type" => 7, "inv_type" => 7,
                    "asset_id" => if stale { id(4) } else { id(40) }, "name" => "Note",
                    "permissions" => llsd_map! { "owner_id" => id(2), "owner_mask" => 0x7fffffff }
                }]) },
            ),
        ];
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (mut status, reply) in replies.into_iter().take(if reject || task_save { 2 } else { 3 }) {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .expect("request timeout")
                    .expect("request");
                let mut bytes = Vec::new();
                let (start, size) = loop {
                    let mut chunk = [0u8; 4096];
                    let n = socket.read(&mut chunk).await.expect("read");
                    assert!(n > 0, "complete request body");
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]);
                        let size = header
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + size {
                            break (end + 4, size);
                        }
                    }
                };
                requests.push((
                    String::from_utf8_lossy(&bytes[..start]).into_owned(),
                    bytes[start..start + size].to_vec(),
                ));
                if requests
                    .last()
                    .is_some_and(|(headers, body)| headers.starts_with("POST /upload ") && !firestorm_embedded_headers(body))
                {
                    status = 400;
                }
                let body = aurora_llsd::to_xml(&reply);
                let header = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).await.expect("header");
                socket.write_all(&body).await.expect("body");
                crate::test_http::finish(socket);
            }
            requests
        });
        (url, task)
    }

    fn firestorm_embedded_headers(data: &[u8]) -> bool {
        // LLNotecard::importEmbeddedItemsStream uses the exact literal
        // "inv_item\t0\n", not tokenized whitespace (llstreamtools.cpp).
        let Ok(text) = std::str::from_utf8(data) else { return false };
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            if line.starts_with("Text length ") {
                break;
            }
            if line.trim_start().starts_with("ext char index ")
                && lines.find(|line| !line.trim().is_empty()).map(str::trim_start) != Some("inv_item\t0")
            {
                return false;
            }
        }
        true
    }

    #[tokio::test]
    async fn notecard_with_inventory_item_saves_through_the_strict_uploader() {
        let mut note = super::super::notecard::Notecard::default();
        let mut text = "Photo : ".to_owned();
        let embedded = InvItem {
            id: id(50),
            asset_id: id(60),
            parent: id(1),
            owner: id(2),
            creator: id(2),
            asset_type: 0,
            inv_type: 0,
            name: "Image de test".into(),
            desc: "Description avec espaces".into(),
            base_mask: 0x7fffffff,
            owner_mask: 0x7fffffff,
            next_owner_mask: 0xe000,
            ..Default::default()
        };
        note.insert_attachment(&mut text, 8, &embedded).expect("image drop");
        let data = note.encode(&text).expect("notecard asset");
        for task_save in [false, true] {
            let (url, task) = notecard_server(false, false, false, task_save).await;
            if task_save {
                assert_eq!(
                    save_task_notecard(&reqwest::Client::new(), &url, id(90), id(3), data.clone())
                        .await
                        .expect("task note save"),
                    id(40)
                );
            } else {
                let saved = save_content(
                    &reqwest::Client::new(),
                    &url,
                    &format!("{url}/fetch"),
                    id(2),
                    id(3),
                    7,
                    data.clone(),
                )
                .await
                .expect("inventory note save");
                assert_eq!(saved.asset_id, id(40));
            }
            let requests = task.await.expect("requests");
            assert_eq!(requests[1].1, data);
            assert!(firestorm_embedded_headers(&requests[1].1));
        }
    }

    #[tokio::test]
    async fn notecard_save_posts_exact_asset_bytes_then_confirms_the_new_asset() {
        let data = super::super::notecard::Notecard::default().encode("Été 🐺\n}").expect("asset");
        for replace in [false, true] {
            let (url, task) = notecard_server(false, false, replace, false).await;
            let saved = save_content(
                &reqwest::Client::new(),
                &url,
                &format!("{url}/fetch"),
                id(2),
                id(3),
                7,
                data.clone(),
            )
            .await
            .expect("save");
            assert_eq!(saved.asset_id, id(40));
            assert_eq!(saved.id, if replace { id(30) } else { id(3) });
            let requests = task.await.expect("requests");
            let negotiate = aurora_llsd::from_xml(&requests[0].1).expect("negotiation");
            assert_eq!(negotiate["item_id"].as_uuid(), id(3));
            assert!(!negotiate.has("target"));
            assert!(requests[1].0.starts_with("POST /upload "));
            assert!(requests[1].0.to_lowercase().contains("content-type: application/octet-stream"));
            assert_eq!(requests[1].1, data);
            let fetch = aurora_llsd::from_xml(&requests[2].1).expect("fetch");
            assert_eq!(fetch["items"].at(0)["item_id"].as_uuid(), saved.id);
        }
    }

    #[tokio::test]
    async fn task_notecard_upload_uses_object_and_item_ids_and_requires_an_asset_ack() {
        let data = super::super::notecard::Notecard::decode(b"")
            .expect("empty")
            .encode("Note dans un objet 🐺")
            .expect("asset");
        for (missing_asset, reject) in [(false, false), (false, true), (true, false)] {
            let (url, task) = notecard_server(missing_asset, reject, false, true).await;
            let result = save_task_notecard(&reqwest::Client::new(), &url, id(90), id(3), data.clone()).await;
            assert_eq!(result.is_ok(), !missing_asset && !reject);
            if let Ok(asset) = result {
                assert_eq!(asset, id(40));
            }
            let requests = task.await.expect("requests");
            assert_eq!(requests.len(), 2, "task notes must not be fetched from agent inventory");
            let body = aurora_llsd::from_xml(&requests[0].1).expect("negotiation");
            assert_eq!(body["task_id"].as_uuid(), id(90));
            assert_eq!(body["item_id"].as_uuid(), id(3));
            assert!(!body.has("target"));
            assert_eq!(requests[1].1, data);
        }
        assert!(
            save_task_notecard(&reqwest::Client::new(), "http://127.0.0.1:1", Uuid::nil(), id(3), data)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn notecard_upload_failure_and_stale_readback_do_not_confirm_a_save() {
        let data = super::super::notecard::Notecard::default().encode("Brouillon").expect("asset");
        for (stale, reject) in [(true, false), (false, true)] {
            let (url, task) = notecard_server(stale, reject, false, false).await;
            assert!(
                save_content(&reqwest::Client::new(), &url, &url, id(2), id(3), 7, data.clone())
                    .await
                    .is_err()
            );
            let requests = task.await.expect("requests");
            assert_eq!(requests.len(), if reject { 2 } else { 3 });
        }
    }

    #[tokio::test]
    async fn malformed_or_oversized_notecards_are_rejected_before_http() {
        for data in [
            b"Linden text version 3\n".to_vec(),
            vec![b'a'; super::super::notecard::MAX_TEXT_BYTES + 1],
        ] {
            assert!(
                save_content(&reqwest::Client::new(), "invalid", "invalid", id(2), id(3), 7, data)
                    .await
                    .is_err()
            );
        }
    }
    async fn move_server(replies: Vec<Llsd>) -> (String, tokio::task::JoinHandle<Vec<(String, Llsd)>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .expect("request timeout")
                    .expect("request");
                let mut bytes = Vec::new();
                let (start, len) = loop {
                    let mut buffer = [0u8; 4096];
                    let n = socket.read(&mut buffer).await.expect("read");
                    assert!(n > 0, "complete request");
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
                let body = if len == 0 {
                    Llsd::Undef
                } else {
                    aurora_llsd::from_xml(&bytes[start..start + len]).expect("LLSD")
                };
                let rejected = header.starts_with("PATCH ") && body.has("parent_id");
                requests.push((header, body));
                let body = aurora_llsd::to_xml(&reply);
                let response = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    if rejected { 400 } else { 200 },
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.expect("header");
                socket.write_all(&body).await.expect("body");
                crate::test_http::finish(socket);
                if rejected {
                    break;
                }
            }
            requests
        });
        (url, task)
    }

    fn move_readback(moved: bool) -> Llsd {
        let parent = if moved { id(4) } else { id(1) };
        let item = llsd_map! { "item_id" => id(3), "parent_id" => parent, "type" => 6, "inv_type" => 6, "name" => "Moved" };
        let folder = llsd_map! { "category_id" => id(5), "parent_id" => parent, "type_default" => -1, "name" => "Folder" };
        llsd_map! { "folders" => Llsd::Array([id(1), id(4)].into_iter().map(|id| llsd_map! {
            "folder_id" => id, "version" => 2,
            "items" => Llsd::Array(if id == parent { vec![item.clone()] } else { Vec::new() }),
            "categories" => Llsd::Array(if id == parent { vec![folder.clone()] } else { Vec::new() }),
        }).collect()) }
    }

    fn move_change() -> Mutation {
        Mutation {
            operations: [(id(3), false), (id(5), true)]
                .into_iter()
                .map(|(id, folder)| Operation::Patch {
                    id,
                    folder,
                    body: llsd_map! { "parent_id" => super::tests::id(4) },
                })
                .collect(),
            refresh: vec![id(1), id(4)],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn parent_changes_use_udp_instead_of_the_ais_patch_rejected_with_http_400() {
        let http = reqwest::Client::new();
        let (url, task) = move_server(vec![Llsd::new_map()]).await;
        let error = crate::outfits::send(http.patch(format!("{url}/item/{}", id(3))), &llsd_map! { "parent_id" => id(4) })
            .await
            .expect_err("old transport refused");
        assert!(error.contains("400"));
        task.await.expect("old request");

        let (url, task) = move_server(vec![move_readback(true)]).await;
        let mut moves = Vec::new();
        let reply = mutate_with_moves(&http, &url, &format!("{url}/fetch"), id(2), move_change(), |movement| {
            moves.push(movement);
            std::future::ready(Ok(()))
        })
        .await
        .expect("confirmed move");
        assert_eq!(
            moves,
            vec![
                InventoryMove {
                    id: id(3),
                    parent: id(4),
                    folder: false
                },
                InventoryMove {
                    id: id(5),
                    parent: id(4),
                    folder: true
                }
            ]
        );
        assert_eq!(reply.contents.len(), 2);
        let requests = task.await.expect("new requests");
        assert_eq!(requests.len(), 1, "moves must not issue any AIS PATCH");
        assert!(requests[0].0.starts_with("POST /fetch "));
    }

    #[tokio::test]
    async fn move_readback_waits_for_storage_without_sending_the_move_twice() {
        let (url, task) = move_server(vec![move_readback(false), move_readback(true)]).await;
        let mut moves = Vec::new();
        let reply = mutate_with_moves(&reqwest::Client::new(), &url, &url, id(2), move_change(), |movement| {
            moves.push(movement);
            std::future::ready(Ok(()))
        })
        .await
        .expect("eventual confirmation");
        assert_eq!(moves.len(), 2, "one send per element");
        assert_eq!(reply.contents[1].items[0].parent, id(4));
        assert_eq!(task.await.expect("reads").len(), 2);
    }

    #[tokio::test]
    async fn an_unconfirmed_move_never_succeeds_and_stops_retrying() {
        let (url, task) = move_server(vec![move_readback(false); 6]).await;
        let mut sent = 0;
        let result = mutate_with_moves(&reqwest::Client::new(), &url, &url, id(2), move_change(), |_| {
            sent += 1;
            std::future::ready(Ok(()))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(sent, 2);
        assert_eq!(task.await.expect("bounded reads").len(), 6);
    }

    #[tokio::test]
    async fn a_move_without_a_region_connection_fails_before_http_requests() {
        let mut sent = 0;
        let result = mutate_with_moves(&reqwest::Client::new(), "invalid", "invalid", id(2), move_change(), |_| {
            sent += 1;
            std::future::ready(Err("closed session".into()))
        })
        .await;
        assert_eq!(result.expect_err("not sent"), "closed session");
        assert_eq!(sent, 1, "stop at the first failed move");
    }

    fn folder_change(name: &str) -> Mutation {
        Mutation {
            operations: vec![Operation::Folder {
                id: id(100),
                parent: id(1),
                name: name.into(),
                type_default: -1,
            }],
            refresh: vec![id(1), id(100)],
            ..Default::default()
        }
    }

    #[test]
    fn new_folder_names_follow_the_protocol_length_and_character_rules() {
        assert_eq!(folder_name("  Copies | temporaires\t "), "Copies   temporaires");
        assert_eq!(folder_name(&"a".repeat(70)), "a".repeat(63));
        assert_eq!(folder_name("Étape"), "tape");
    }

    fn folder_readback(name: &str, present: bool) -> Llsd {
        llsd_map! { "folders" => Llsd::Array(vec![
            llsd_map! { "folder_id" => id(1), "version" => 2, "categories" => Llsd::Array(if present { vec![
                llsd_map! { "category_id" => id(200), "parent_id" => id(1), "type_default" => -1, "name" => name }
            ] } else { Vec::new() }) },
            llsd_map! { "folder_id" => id(200), "version" => 1 }
        ]) }
    }

    #[tokio::test]
    async fn interactive_folder_trees_use_ais_acknowledgments_without_fetching_even_when_empty() {
        for copy_item in [false, true] {
            let mut change = folder_change("  Copies | temporaires  ");
            change.defer_folder_contents = true;
            for (client, parent) in [(101, 100), (102, 101)] {
                change.operations.push(Operation::Folder {
                    id: id(client),
                    parent: id(parent),
                    name: "Child".into(),
                    type_default: -1,
                });
                change.refresh.push(id(client));
            }
            if copy_item {
                change.copies.push(CopyItem {
                    item: id(300),
                    owner: id(2),
                    parent: id(102),
                    name: String::new(),
                });
            }
            let (url, task) = move_server(
                [200, 201, 202]
                    .into_iter()
                    .map(|actual| llsd_map! { "_created_categories" => Llsd::Array(vec![id(actual).into()]) })
                    .collect(),
            )
            .await;
            let reply = mutate_with_progress(
                &reqwest::Client::new(),
                &url,
                "invalid fetch capability must not be used",
                id(2),
                change,
                |_| std::future::ready(Err("unexpected move".into())),
                |_| {},
            )
            .await
            .expect("AIS alone confirms the interactive tree");
            assert!(reply.contents.is_empty());
            assert_eq!(reply.folders.len(), 3);
            assert_eq!(reply.folders[0].name, "Copies   temporaires");
            assert_eq!(reply.folders[1].parent, id(200));
            assert_eq!(reply.folders[2].parent, id(201));
            if copy_item {
                assert_eq!(reply.copies[0].parent, id(202));
            }
            let requests = task.await.expect("AIS requests");
            assert_eq!(requests.len(), 3);
            assert!(requests[0].0.starts_with("POST /category/00000000-0000-0000-0000-000000000001?"));
            assert!(requests[1].0.contains(&id(200).to_string()));
            assert!(requests[2].0.contains(&id(201).to_string()));
        }
    }

    #[tokio::test]
    async fn folder_creation_uses_the_inventory_name_corrected_by_firestorm() {
        let name = "Remplacement de prim     copies temporaires";
        let (url, task) = move_server(vec![
            llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) },
            folder_readback(name, true),
        ])
        .await;
        let reply = mutate(
            &reqwest::Client::new(),
            &url,
            &url,
            id(2),
            folder_change("Remplacement de prim — copies temporaires"),
        )
        .await
        .expect("confirmed normalized name");
        assert_eq!(reply.created[&id(100)], id(200));
        let requests = task.await.expect("requests");
        assert_eq!(requests[0].1["categories"].at(0)["name"].as_str(), name);
    }

    #[tokio::test]
    async fn folder_creation_waits_for_readback_without_creating_a_second_folder() {
        let name = "Copies temporaires";
        let (url, task) = move_server(vec![
            llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) },
            folder_readback(name, false),
            folder_readback(name, true),
        ])
        .await;
        let reply = mutate(&reqwest::Client::new(), &url, &format!("{url}/fetch"), id(2), folder_change(name))
            .await
            .expect("eventual creation confirmation");
        assert_eq!(reply.created[&id(100)], id(200));
        let requests = task.await.expect("requests");
        assert_eq!(requests.len(), 3);
        assert!(requests[0].0.starts_with("POST /category/"));
        assert!(requests[1..].iter().all(|r| r.0.starts_with("POST /fetch ")));
    }

    #[tokio::test]
    async fn a_created_folder_with_permanently_wrong_metadata_still_fails() {
        let mut replies = vec![llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) }];
        replies.extend(vec![folder_readback("Wrong folder", true); 6]);
        let (url, task) = move_server(replies).await;
        assert!(
            mutate(&reqwest::Client::new(), &url, &url, id(2), folder_change("Expected folder"))
                .await
                .is_err()
        );
        assert_eq!(task.await.expect("bounded requests").len(), 7);
    }

    #[tokio::test]
    async fn moves_into_new_folders_use_the_server_assigned_parent_id() {
        let doc = llsd_map! { "folders" => Llsd::Array(vec![
            llsd_map! { "folder_id" => id(1), "version" => 2, "categories" => Llsd::Array(vec![
                llsd_map! { "category_id" => id(200), "parent_id" => id(1), "type_default" => -1, "name" => "Destination" }
            ]) },
            llsd_map! { "folder_id" => id(200), "version" => 1, "items" => Llsd::Array(vec![
                llsd_map! { "item_id" => id(3), "parent_id" => id(200), "type" => 6, "inv_type" => 6, "name" => "Moved" }
            ]) }
        ]) };
        let (url, task) = move_server(vec![llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) }, doc]).await;
        let change = Mutation {
            operations: vec![
                Operation::Folder {
                    id: id(100),
                    parent: id(1),
                    name: "Destination".into(),
                    type_default: -1,
                },
                Operation::Patch {
                    id: id(3),
                    folder: false,
                    body: llsd_map! { "parent_id" => id(100) },
                },
            ],
            refresh: vec![id(1), id(100)],
            ..Default::default()
        };
        let mut moves = Vec::new();
        mutate_with_moves(&reqwest::Client::new(), &url, &url, id(2), change, |movement| {
            moves.push(movement);
            std::future::ready(Ok(()))
        })
        .await
        .expect("created parent confirmed");
        assert_eq!(
            moves,
            vec![InventoryMove {
                id: id(3),
                parent: id(200),
                folder: false
            }]
        );
        assert_eq!(task.await.expect("requests").len(), 2);
    }

    #[tokio::test]
    async fn metadata_updates_keep_ais_but_remove_the_udp_parent_change() {
        let (url, task) = move_server(vec![Llsd::new_map(), move_readback(true)]).await;
        let change = Mutation {
            operations: vec![Operation::Patch {
                id: id(3),
                folder: false,
                body: llsd_map! { "parent_id" => id(4), "name" => "Moved" },
            }],
            refresh: vec![id(1), id(4)],
            ..Default::default()
        };
        let mut moves = Vec::new();
        mutate_with_moves(&reqwest::Client::new(), &url, &url, id(2), change, |movement| {
            moves.push(movement);
            std::future::ready(Ok(()))
        })
        .await
        .expect("move and rename confirmed");
        let requests = task.await.expect("requests");
        assert!(requests[0].0.starts_with("PATCH /item/"));
        assert_eq!(requests[0].1, llsd_map! { "name" => "Moved" });
        assert_eq!(moves.len(), 1);
    }

    #[tokio::test]
    async fn nested_folders_and_deferred_copies_use_the_server_assigned_ids() {
        for with_items in [false, true] {
            let (url, task) = server(vec![
                (200, llsd_map! { "_created_categories" => Llsd::Array(vec![id(200).into()]) }),
                (200, llsd_map! { "_created_categories" => Llsd::Array(vec![id(201).into()]) }),
            ])
            .await;
            let result = mutate(
                &reqwest::Client::new(),
                &url,
                "invalid",
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
                    copies: if with_items {
                        vec![CopyItem {
                            item: id(3),
                            owner: id(2),
                            parent: id(101),
                            name: String::new(),
                        }]
                    } else {
                        Vec::new()
                    },
                    refresh: vec![id(1), id(100), id(101)],
                    defer_folder_contents: true,
                    ..Default::default()
                },
            )
            .await
            .expect("confirmed hierarchy");
            if with_items {
                assert_eq!(result.copies[0].parent, id(201));
            }
            assert_eq!(result.folders[1].parent, id(200));
            assert!(result.contents.is_empty(), "no empty snapshot is published before item copies");
            assert_eq!(result.refresh, vec![id(1), id(200), id(201)]);
            let requests = task.await.expect("requests");
            assert_eq!(requests.len(), 2, "only folder creations, without redundant reads");
            assert!(requests[1].0.contains(&format!("/category/{}?", id(200))));
            assert_eq!(requests[1].1["categories"].at(0)["parent_id"].as_uuid(), id(200));
        }
    }

    #[tokio::test]
    async fn item_copies_are_sent_without_waiting_for_an_empty_http_snapshot() {
        let reply = mutate(
            &reqwest::Client::new(),
            "invalid",
            "invalid",
            id(2),
            Mutation {
                copies: vec![CopyItem {
                    item: id(3),
                    owner: id(2),
                    parent: id(4),
                    name: String::new(),
                }],
                refresh: vec![id(4)],
                ..Default::default()
            },
        )
        .await
        .expect("no HTTP request needed before UDP copies");
        assert_eq!(reply.copies.len(), 1);
        assert_eq!(reply.refresh, vec![id(4)]);
        assert!(reply.contents.is_empty());
    }

    #[test]
    fn copied_folder_snapshots_introduce_parents_before_every_nested_content() {
        let snapshot = |folder_id, child: Option<u128>| FolderContents {
            folder_id: id(folder_id),
            owner_id: id(2),
            version: 1,
            folders: child
                .map(|child| super::super::InvFolder {
                    id: id(child),
                    parent: id(folder_id),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            items: Vec::new(),
        };
        // UUID ordering is intentionally opposite to ancestry, as on a server.
        let mut contents = vec![
            snapshot(100, None),
            snapshot(200, Some(100)),
            snapshot(300, Some(200)),
            snapshot(900, Some(300)),
        ];
        sort_contents_parent_first(&mut contents);
        assert_eq!(
            contents.iter().map(|c| c.folder_id).collect::<Vec<_>>(),
            vec![id(900), id(300), id(200), id(100)]
        );
    }

    #[tokio::test]
    async fn inventory_favorites_do_not_hide_unconfirmed_name_changes() {
        let current = favorite_readback(false, true);
        let record = current["_embedded"]["items"][id(3).to_string().as_str()].clone();
        let mut replies = vec![record];
        replies.extend(vec![favorite_readback(false, true); 6]);
        let (url, task) = move_server(replies).await;
        let mut change = favorite_change(false, true);
        let Operation::Patch { body, .. } = &mut change.operations[0] else {
            panic!("favorite patch");
        };
        body.insert("name", "New sound");
        assert!(mutate(&reqwest::Client::new(), &url, &url, id(2), change).await.is_err());
        assert_eq!(task.await.expect("bounded readback").len(), 7);
    }

    #[tokio::test]
    async fn inventory_favorites_wait_for_readback_without_repeating_the_patch() {
        for folder in [false, true] {
            for favorite in [false, true] {
                let (url, task) = move_server(vec![
                    Llsd::new_map(),
                    favorite_readback(folder, !favorite),
                    favorite_readback(folder, favorite),
                ])
                .await;
                let reply = mutate(
                    &reqwest::Client::new(),
                    &url,
                    &format!("{url}/fetch"),
                    id(2),
                    favorite_change(folder, favorite),
                )
                .await
                .expect("eventual favorite confirmation");
                let contents = &reply.contents[0];
                assert_eq!(
                    if folder {
                        contents.folders[0].favorite
                    } else {
                        contents.items[0].favorite
                    },
                    favorite
                );
                let requests = task.await.expect("requests");
                assert_eq!(requests.len(), 3);
                let kind = if folder { "category" } else { "item" };
                assert!(requests[0].0.starts_with(&format!("PATCH /{kind}/{} ", id(3))));
                assert_eq!(requests[0].1, metadata_favorite(favorite));
                assert!(requests[1..].iter().all(|r| {
                    r.0.starts_with("GET /category/00000000-0000-0000-0000-000000000001/children?depth=0 ")
                }));
            }
        }
    }

    fn favorite_change(folder: bool, favorite: bool) -> Mutation {
        Mutation {
            operations: vec![Operation::Patch {
                id: id(3),
                folder,
                body: metadata_favorite(favorite),
            }],
            refresh: vec![id(1)],
            ..Default::default()
        }
    }

    fn favorite_readback(folder: bool, favorite: bool) -> Llsd {
        let favorite = if favorite {
            llsd_map! { "toggled" => true }
        } else {
            Llsd::Undef
        };
        let record = if folder {
            llsd_map! { "category_id" => id(3), "parent_id" => id(1), "name" => "Folder", "type_default" => -1, "favorite" => favorite }
        } else {
            llsd_map! { "item_id" => id(3), "parent_id" => id(1), "name" => "Sound", "type" => 1, "inv_type" => 1, "favorite" => favorite }
        };
        let mut records = Llsd::new_map();
        records.insert(id(3).to_string(), record);
        let mut embedded = llsd_map! { "categories" => Llsd::new_map(), "items" => Llsd::new_map(), "links" => Llsd::new_map() };
        embedded.insert(if folder { "categories" } else { "items" }, records);
        llsd_map! { "category_id" => id(1), "agent_id" => id(2), "version" => 2, "_embedded" => embedded }
    }

    #[tokio::test]
    async fn inventory_favorites_still_fail_when_readback_never_confirms_them() {
        let mut responses = vec![Llsd::new_map()];
        responses.extend(vec![favorite_readback(false, false); 6]);
        let (url, task) = move_server(responses).await;
        assert!(
            mutate(
                &reqwest::Client::new(),
                &url,
                &format!("{url}/fetch"),
                id(2),
                favorite_change(false, true)
            )
            .await
            .is_err()
        );
        let requests = task.await.expect("bounded requests");
        assert_eq!(requests.len(), 7);
        assert!(requests[0].0.starts_with("PATCH /item/"));
        assert!(requests[1..].iter().all(|r| {
            r.0.starts_with("GET /category/00000000-0000-0000-0000-000000000001/children?depth=0 ")
        }));
    }

    #[tokio::test]
    async fn inventory_favorites_stop_immediately_after_a_server_refusal() {
        let (url, task) = server(vec![(403, Llsd::new_map())]).await;
        assert!(
            mutate(
                &reqwest::Client::new(),
                &url,
                "invalid fetch capability",
                id(2),
                favorite_change(false, true)
            )
            .await
            .is_err()
        );
        assert_eq!(task.await.expect("requests").len(), 1);
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
