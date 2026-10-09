//! Inventory data model and `FetchInventoryDescendents2` client
//! (see `newview/llinventorymodelbackgroundfetch.cpp`).

use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct InvFolder {
    pub id: Uuid,
    pub parent: Uuid,
    pub name: String,
    /// Preferred (system) folder type, -1 for normal folders.
    pub type_default: i32,
    pub version: i32,
    pub favorite: bool,
    pub thumbnail: Uuid,
}

#[derive(Debug, Clone)]
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
        thumbnail: v["thumbnail"]["asset_id"].as_uuid(),
    })
}

pub fn item_from_llsd(v: &Llsd) -> Option<InvItem> {
    let id = v["item_id"].as_uuid();
    if id.is_nil() {
        return None;
    }
    let asset_id = if v.has("asset_id") {
        v["asset_id"].as_uuid()
    } else {
        // shadow_id is the asset id obfuscated with a well known key; not needed for display
        Uuid::nil()
    };
    Some(InvItem {
        id,
        parent: v["parent_id"].as_uuid(),
        name: v["name"].to_string_value(),
        desc: v["desc"].to_string_value(),
        asset_type: int_or_name(&v["type"], asset_type_from_name),
        inv_type: int_or_name(&v["inv_type"], inv_type_from_name),
        asset_id,
        flags: v["flags"].as_u32(),
        favorite: v["favorite"]["toggled"].as_bool(),
        creator: v["permissions"]["creator_id"].as_uuid(),
        created_at: v["created_at"].as_i32() as i64,
        owner: v["permissions"]["owner_id"].as_uuid(),
        group_mask: v["permissions"]["group_mask"].as_u32(),
        everyone_mask: v["permissions"]["everyone_mask"].as_u32(),
        next_owner_mask: v["permissions"]["next_owner_mask"].as_u32(),
    })
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
    v["items"].as_array().iter().filter_map(item_from_llsd).collect()
}

pub fn parse_fetch_response(v: &Llsd) -> Vec<FolderContents> {
    v["folders"]
        .as_array()
        .iter()
        .map(|f| FolderContents {
            folder_id: f["folder_id"].as_uuid(),
            owner_id: f["owner_id"].as_uuid(),
            version: f["version"].as_i32(),
            folders: f["categories"].as_array().iter().filter_map(folder_from_llsd).collect(),
            items: f["items"].as_array().iter().filter_map(item_from_llsd).collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
