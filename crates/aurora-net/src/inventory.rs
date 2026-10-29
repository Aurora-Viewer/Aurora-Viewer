//! Inventory data model and AIS / legacy inventory clients
//! (see `newview/llinventorymodelbackgroundfetch.cpp`).

use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;
pub mod ais;
#[cfg(test)]
mod gesture_tests;
pub mod mesh_upload;
pub mod notecard;
pub mod operations;
pub mod rez;
pub mod thumbnail;
pub mod upload;

pub fn created_date(seconds: i64) -> String {
    let (year, month, day) = crate::profile::civil_from_days(seconds.div_euclid(86400));
    format!("{day:02}/{month:02}/{year}")
}

#[derive(Debug, Clone, Default)]
pub struct InvFolder {
    pub id: Uuid,
    pub parent: Uuid,
    pub name: String,
    /// Preferred (system) folder type, -1 for normal folders.
    pub type_default: i32,
    pub version: i32,
    pub favorite: bool,
    /// A complete AIS record, or explicit metadata, can replace this flag.
    pub favorite_known: bool,
    pub thumbnail: Uuid,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InvItem {
    pub id: Uuid,
    pub parent: Uuid,
    pub name: String,
    pub desc: String,
    pub asset_type: i32,
    pub inv_type: i32,
    pub asset_id: Uuid,
    pub flags: u32,
    pub favorite: bool,
    /// UDP and older fetch responses may omit this metadata entirely.
    pub favorite_known: bool,
    pub thumbnail: Uuid,
    pub base_mask: u32,
    pub owner_mask: u32,
    pub last_owner: Uuid,
    pub group_id: Uuid,
    pub group_owned: bool,
    pub sale_type: u8,
    pub sale_price: i32,
    pub creator: Uuid,
    pub created_at: i64,
    /// Owner and permission masks (needed to attach an object).
    pub owner: Uuid,
    pub group_mask: u32,
    pub everyone_mask: u32,
    pub next_owner_mask: u32,
}

#[derive(Debug, Clone)]
pub struct FolderContents {
    pub folder_id: Uuid,
    pub owner_id: Uuid,
    pub version: i32,
    pub folders: Vec<InvFolder>,
    pub items: Vec<InvItem>,
}

/// LLAssetType names (llassettype.cpp) -> numeric type.
pub fn asset_type_from_name(s: &str) -> i32 {
    match s {
        "texture" => 0,
        "sound" => 1,
        "callcard" => 2,
        "landmark" => 3,
        "clothing" => 5,
        "object" => 6,
        "notecard" => 7,
        "category" => 8,
        "lsltext" => 10,
        "lslbyte" => 11,
        "txtr_tga" => 12,
        "bodypart" => 13,
        "snapshot" => 15,
        "jpeg" => 19,
        "animatn" => 20,
        "gesture" => 21,
        "simstate" => 22,
        "link" => 24,
        "link_f" => 25,
        "mesh" => 49,
        "settings" => 56,
        "material" => 57,
        _ => -1,
    }
}

/// LLInventoryType names -> numeric type.
pub fn inv_type_from_name(s: &str) -> i32 {
    match s {
        "texture" => 0,
        "sound" => 1,
        "callcard" => 2,
        "landmark" => 3,
        "object" => 6,
        "notecard" => 7,
        "category" => 8,
        "root" => 9,
        "script" => 10,
        "snapshot" => 15,
        "attach" => 17,
        "wearable" => 18,
        "animation" => 19,
        "gesture" => 20,
        "mesh" => 22,
        "settings" => 25,
        "material" => 26,
        _ => -1,
    }
}

fn int_or_name(v: &Llsd, f: fn(&str) -> i32) -> i32 {
    match v {
        Llsd::String(s) => f(s),
        other => other.as_i32(),
    }
}

pub fn folder_from_llsd(v: &Llsd) -> Option<InvFolder> {
    let id = if v.has("category_id") {
        v["category_id"].as_uuid()
    } else {
        v["folder_id"].as_uuid()
    };
    if id.is_nil() {
        return None;
    }
    let type_default = if v.has("type_default") {
        v["type_default"].as_i32()
    } else if v.has("preferred_type") {
        int_or_name(&v["preferred_type"], asset_type_from_name)
    } else {
        -1
    };
    Some(InvFolder {
        id,
        parent: v["parent_id"].as_uuid(),
        name: v["name"].to_string_value(),
        type_default,
        version: v["version"].as_i32(),
        favorite: v["favorite"]["toggled"].as_bool(),
        favorite_known: v.has("favorite"),
        thumbnail: v["thumbnail"]["asset_id"].as_uuid(),
    })
}

pub fn item_from_llsd(v: &Llsd) -> Option<InvItem> {
    let id = v["item_id"].as_uuid();
    if id.is_nil() {
        return None;
    }
    let asset_id = if v.has("linked_id") {
        v["linked_id"].as_uuid()
    } else if v.has("asset_id") {
        v["asset_id"].as_uuid()
    } else {
        // LLInventoryItem::fromLLSD (llinventory.cpp): fixed XOR key, not a secret.
        let shadow = v["shadow_id"].as_uuid();
        if shadow.is_nil() {
            Uuid::nil()
        } else {
            Uuid::from_u128(shadow.as_u128() ^ 0x3c115e51_04f4_523c_9fa6_98aff1034730)
        }
    };
    let asset_type = int_or_name(&v["type"], asset_type_from_name);
    let inv_type = if v.has("inv_type") {
        int_or_name(&v["inv_type"], inv_type_from_name)
    } else {
        -1
    };
    // LLInventoryItem::fromLLSD / LLInventoryType::defaultForAssetType:
    // Recover an omitted inventory type from its asset type, as the reader
    // of a newly received inventory record does in the reference.
    let inv_type = if inv_type >= 0 {
        inv_type
    } else {
        default_inventory_type(asset_type)
    };
    Some(InvItem {
        id,
        parent: v["parent_id"].as_uuid(),
        name: v["name"].to_string_value(),
        desc: v["desc"].to_string_value(),
        asset_type,
        inv_type,
        asset_id,
        flags: v["flags"].as_u32(),
        favorite: v["favorite"]["toggled"].as_bool(),
        favorite_known: v.has("favorite"),
        thumbnail: v["thumbnail"]["asset_id"].as_uuid(),
        base_mask: v["permissions"]["base_mask"].as_u32(),
        owner_mask: v["permissions"]["owner_mask"].as_u32(),
        last_owner: v["permissions"]["last_owner_id"].as_uuid(),
        group_id: v["permissions"]["group_id"].as_uuid(),
        group_owned: v["permissions"]["is_owner_group"].as_bool(),
        sale_type: match v["sale_info"]["sale_type"].as_str() {
            "orig" => 1,
            "copy" => 2,
            "cntn" => 3,
            _ => v["sale_info"]["sale_type"].as_u32() as u8,
        },
        sale_price: v["sale_info"]["sale_price"].as_i32(),
        creator: v["permissions"]["creator_id"].as_uuid(),
        created_at: v["created_at"].as_i32() as i64,
        owner: v["permissions"]["owner_id"].as_uuid(),
        group_mask: v["permissions"]["group_mask"].as_u32(),
        everyone_mask: v["permissions"]["everyone_mask"].as_u32(),
        next_owner_mask: v["permissions"]["next_owner_mask"].as_u32(),
    })
}

fn default_inventory_type(asset_type: i32) -> i32 {
    match asset_type {
        0 | 12 => 0,
        1 | 17 => 1,
        2 | 3 | 6 | 7 => asset_type,
        4 | 10 | 11 => 10,
        5 | 13 => 18,
        8 | 14..=16 => 8,
        20 => 19,
        21 => 20,
        40 => 23,
        45 => 24,
        49 => 22,
        56 => 25,
        57 => 26,
        58 => 27,
        59 => 28,
        _ => -1,
    }
}

/// Parse the login "inventory-skeleton" / "inventory-skel-lib" arrays.
pub fn parse_skeleton(v: &Llsd) -> Vec<InvFolder> {
    v.as_array().iter().filter_map(folder_from_llsd).collect()
}

pub fn fetch_request_body(folders: &[Uuid], owner: Uuid) -> Llsd {
    let list: Vec<Llsd> = folders
        .iter()
        .map(|f| {
            llsd_map! {
                "folder_id" => *f,
                "owner_id" => owner,
                "sort_order" => 1,
                "fetch_folders" => true,
                "fetch_items" => true,
            }
        })
        .collect();
    llsd_map! { "folders" => Llsd::Array(list) }
}

/// `FetchInventory2` request (LLInventoryModel::fetchItemHttp): items by id.
pub fn fetch_items_body(items: &[Uuid], owner: Uuid) -> Llsd {
    let list: Vec<Llsd> = items.iter().map(|i| llsd_map! { "owner_id" => owner, "item_id" => *i }).collect();
    llsd_map! { "agent_id" => owner, "items" => Llsd::Array(list) }
}

pub fn parse_items_response(v: &Llsd) -> Vec<InvItem> {
    v["items"].as_array().iter().filter_map(legacy_item).collect()
}

/// A `FetchInventory2` answer to `asked`: an item absent from a valid answer
/// does not exist; a 200-with-error answer
/// (LLInventoryModel::FetchItemHttpHandler::onCompleted) proves nothing.
pub fn item_reads(v: &Llsd, asked: &[Uuid]) -> ais::ItemReads {
    if !v.is_map() || v.has("error") {
        return ais::ItemReads {
            failed: asked.to_vec(),
            ..Default::default()
        };
    }
    let answered: std::collections::HashSet<_> = v["items"].as_array().iter().map(|it| it["item_id"].as_uuid()).collect();
    let items = parse_items_response(v);
    let read: std::collections::HashSet<_> = items.iter().map(|it| it.id).collect();
    let mut reads = ais::ItemReads {
        items,
        ..Default::default()
    };
    for id in asked {
        if !answered.contains(id) {
            reads.missing.push(*id);
        } else if !read.contains(id) {
            reads.failed.push(*id);
        }
    }
    reads
}

fn legacy_item(v: &Llsd) -> Option<InvItem> {
    let mut item = item_from_llsd(v)?;
    item.favorite_known = item.favorite;
    Some(item)
}

fn legacy_folder(v: &Llsd) -> Option<InvFolder> {
    let mut folder = folder_from_llsd(v)?;
    folder.favorite_known = folder.favorite;
    Some(folder)
}

pub fn parse_fetch_response(v: &Llsd) -> Vec<FolderContents> {
    v["folders"]
        .as_array()
        .iter()
        .map(|f| FolderContents {
            folder_id: f["folder_id"].as_uuid(),
            owner_id: f["owner_id"].as_uuid(),
            version: f["version"].as_i32(),
            folders: f["categories"].as_array().iter().filter_map(legacy_folder).collect(),
            items: f["items"].as_array().iter().filter_map(legacy_item).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_inventory_types_follow_asset_defaults_and_explicit_attachment_types_stay() {
        for (asset, inventory) in [(6, 6), (5, 18), (13, 18), (20, 19), (21, 20), (56, 25), (57, 26)] {
            let doc = llsd_map! { "item_id" => Uuid::from_u128(1), "type" => asset };
            assert_eq!(item_from_llsd(&doc).expect("item").inv_type, inventory);
        }
        let doc = llsd_map! { "item_id" => Uuid::from_u128(1), "type" => 6, "inv_type" => 17 };
        assert_eq!(item_from_llsd(&doc).expect("attachment").inv_type, 17);
    }

    #[test]
    fn parses_fetch_response() {
        let doc = br#"<llsd><map><key>folders</key><array><map>
            <key>folder_id</key><uuid>00000000-0000-0000-0000-000000000001</uuid>
            <key>owner_id</key><uuid>00000000-0000-0000-0000-000000000002</uuid>
            <key>version</key><integer>7</integer>
            <key>categories</key><array><map>
                <key>category_id</key><uuid>00000000-0000-0000-0000-000000000003</uuid>
                <key>parent_id</key><uuid>00000000-0000-0000-0000-000000000001</uuid>
                <key>name</key><string>Objects</string>
                <key>type_default</key><integer>6</integer>
                <key>version</key><integer>1</integer></map></array>
            <key>items</key><array><map>
                <key>item_id</key><uuid>00000000-0000-0000-0000-000000000004</uuid>
                <key>parent_id</key><uuid>00000000-0000-0000-0000-000000000001</uuid>
                <key>name</key><string>Home</string>
                <key>type</key><string>landmark</string>
                <key>inv_type</key><string>landmark</string>
                <key>asset_id</key><uuid>00000000-0000-0000-0000-000000000005</uuid>
            </map></array></map></array></map></llsd>"#;
        let v = aurora_llsd::from_xml(doc).unwrap();
        let r = parse_fetch_response(&v);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].version, 7);
        assert_eq!(r[0].folders[0].name, "Objects");
        assert_eq!(r[0].items[0].asset_type, 3);
        assert_eq!(r[0].items[0].asset_id, Uuid::from_u128(5));
    }

    #[test]
    fn fetch_inventory2_answers_tell_missing_items_from_failures() {
        let asked = [Uuid::from_u128(4), Uuid::from_u128(6)];
        let found = llsd_map! {
            "item_id" => Uuid::from_u128(4), "parent_id" => Uuid::from_u128(1), "name" => "Shirt", "type" => 5, "inv_type" => 18,
        };
        let reply = llsd_map! { "agent_id" => Uuid::from_u128(2), "items" => Llsd::Array(vec![found]) };
        let reads = item_reads(&reply, &asked);
        assert_eq!(reads.items.iter().map(|it| it.id).collect::<Vec<_>>(), [asked[0]]);
        assert_eq!(reads.missing, [asked[1]], "absent from a valid answer: deleted");
        assert!(reads.failed.is_empty());
        // 200-with-error and malformed answers prove nothing
        for bad in [llsd_map! { "error" => llsd_map! { "message" => "busy" } }, Llsd::Undef] {
            let reads = item_reads(&bad, &asked);
            assert_eq!(reads.failed, asked);
            assert!(reads.items.is_empty() && reads.missing.is_empty());
        }
    }
}
