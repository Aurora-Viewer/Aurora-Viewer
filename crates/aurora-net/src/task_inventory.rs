//! Object inventories (task inventories), kept apart from the resident's
//! inventory: the capability answer and the legacy text file the simulator
//! sends after RequestTaskInventory, and the CRC sent back with
//! UpdateTaskInventory / RezScript.
//!
//! Port of LLViewerObject::loadTaskInvLLSD / loadTaskInvFile,
//! LLInventoryObject / LLInventoryItem::importLegacyStream / fromLLSD,
//! LLPermissions / LLSaleInfo::importLegacyStream and
//! LLInventoryItem::getCRC32 (indra/newview/llviewerobject.cpp,
//! indra/llinventory/, Firestorm, originally LGPL 2.1).

use crate::build::TaskItem;
use crate::inventory::{asset_type_from_name, inv_type_from_name};
use aurora_llsd::Llsd;
use uuid::Uuid;

/// MAGIC_ID: asset ids of no-copy items travel XORed with it ("shadow_id").
const MAGIC_ID: Uuid = Uuid::from_u128(0x3c115e51_04f4_523c_9fa6_98aff1034730);

/// LLInventoryType::IT_NONE.
const IT_NONE: i8 = -1;

/// LLSaleInfo sale type names.
fn sale_type_from_name(s: &str) -> u8 {
    match s {
        "orig" => 1,
        "copy" => 2,
        "cntn" => 3,
        _ => 0,
    }
}

fn hex_u32(s: &str) -> u32 {
    u32::from_str_radix(s.trim(), 16).unwrap_or(0)
}

fn xor_magic(id: Uuid) -> Uuid {
    let mut b = *id.as_bytes();
    for (x, m) in b.iter_mut().zip(MAGIC_ID.as_bytes()) {
        *x ^= m;
    }
    Uuid::from_bytes(b)
}

/// "name\tValue with spaces|": the text between the tab after the keyword
/// and the '|' (scanf "%s%[\t]%[^|]").
fn bar_value(line: &str) -> String {
    let t = line.trim_start();
    let rest = t.split_once(char::is_whitespace).map(|(_, r)| r).unwrap_or("");
    let rest = rest.trim_start_matches(['\t', ' ']);
    let v = rest.split('|').next().unwrap_or("");
    // replaceNonstandardASCII + no '|' (it ends the field)
    v.chars().map(|c| if (c as u32) < 0x20 && c != '\t' { ' ' } else { c }).collect()
}

fn keyword_value(line: &str) -> (&str, &str) {
    let mut it = line.split_whitespace();
    (it.next().unwrap_or(""), it.next().unwrap_or(""))
}

/// Parse a task inventory file. The "Contents" root folder comes back as
/// an item with `is_folder` set (and no other folder exists in objects).
pub fn parse_task_inventory(text: &str) -> Vec<TaskItem> {
    let mut lines = text.lines();
    let mut out = Vec::new();
    while let Some(line) = lines.next() {
        match keyword_value(line).0 {
            "inv_object" => {
                if let Some(f) = parse_object(&mut lines) {
                    out.push(f);
                }
            }
            "inv_item" => {
                if let Some(i) = parse_item(&mut lines) {
                    out.push(i);
                }
            }
            _ => {}
        }
        if out.len() > 10_000 {
            break;
        }
    }
    out
}

fn parse_object<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Option<TaskItem> {
    let mut f = TaskItem {
        is_folder: true,
        asset_type: 8,
        inv_type: 8,
        ..Default::default()
    };
    for line in lines.by_ref() {
        let (k, v) = keyword_value(line);
        match k {
            "{" => {}
            "}" => return Some(f),
            "obj_id" => f.item_id = v.parse().unwrap_or_default(),
            "parent_id" => f.parent_id = v.parse().unwrap_or_default(),
            "type" => f.asset_type = asset_type_from_name(v) as i8,
            "name" => f.name = bar_value(line),
            _ => {}
        }
    }
    None
}

fn parse_permissions<'a>(lines: &mut impl Iterator<Item = &'a str>, i: &mut TaskItem) {
    for line in lines.by_ref() {
        let (k, v) = keyword_value(line);
        match k {
            "{" => {}
            "}" => return,
            "creator_id" => i.creator_id = v.parse().unwrap_or_default(),
            "owner_id" => i.owner_id = v.parse().unwrap_or_default(),
            "last_owner_id" => i.last_owner_id = v.parse().unwrap_or_default(),
            "group_id" => i.group_id = v.parse().unwrap_or_default(),
            "group_owned" => i.group_owned = v.trim() == "1",
            "base_mask" => i.base_mask = hex_u32(v),
            "owner_mask" => i.owner_mask = hex_u32(v),
            "group_mask" => i.group_mask = hex_u32(v),
            "everyone_mask" => i.everyone_mask = hex_u32(v),
            "next_owner_mask" => i.next_owner_mask = hex_u32(v),
            _ => {}
        }
    }
}

fn parse_item<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Option<TaskItem> {
    let mut i = TaskItem {
        inv_type: IT_NONE,
        ..Default::default()
    };
    while let Some(line) = lines.next() {
        let (k, v) = keyword_value(line);
        match k {
            "{" => {}
            "}" => {
                if i.inv_type == IT_NONE {
                    i.inv_type = default_inv_type(i.asset_type);
                }
                return Some(i);
            }
            "item_id" => i.item_id = v.parse().unwrap_or_default(),
            "parent_id" => i.parent_id = v.parse().unwrap_or_default(),
            "permissions" => parse_permissions(lines, &mut i),
            "sale_info" => {
                // old files keep the next owner mask here (perm_mask)
                for line in lines.by_ref() {
                    let (k, v) = keyword_value(line);
                    match k {
                        "{" => {}
                        "}" => break,
                        "sale_type" => i.sale_type = sale_type_from_name(v),
                        "sale_price" => i.sale_price = v.trim().parse().unwrap_or(0),
                        "perm_mask" => {
                            let mut m = hex_u32(v);
                            if m == 0 {
                                m = i.owner_mask;
                            }
                            if m & crate::build::perm::COPY == 0 {
                                m |= crate::build::perm::TRANSFER;
                            }
                            i.next_owner_mask = m;
                        }
                        _ => {}
                    }
                }
            }
            "shadow_id" => i.asset_id = xor_magic(v.parse().unwrap_or_default()),
            "asset_id" => i.asset_id = v.parse().unwrap_or_default(),
            "type" => i.asset_type = asset_type_from_name(v) as i8,
            "inv_type" => i.inv_type = inv_type_from_name(v) as i8,
            "flags" => i.flags = hex_u32(v),
            "name" => i.name = bar_value(line).replace('|', " "),
            "desc" => i.description = bar_value(line),
            "creation_date" => i.creation_date = v.trim().parse().unwrap_or(0),
            _ => {}
        }
    }
    None
}

/// LLInventoryType::defaultForAssetType (the types objects can hold).
fn default_inv_type(asset_type: i8) -> i8 {
    match asset_type {
        0 | 12 | 19 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        5 | 13 => 18,
        6 => 6,
        7 => 7,
        8 => 8,
        10 | 11 => 10,
        15 => 15,
        20 => 19,
        21 => 20,
        49 => 22,
        56 => 25,
        57 => 26,
        _ => IT_NONE,
    }
}

fn uuid_crc(id: &Uuid) -> u32 {
    id.as_bytes()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .fold(0u32, u32::wrapping_add)
}

/// LLInventoryItem::getCRC32 (a checksum, not a real CRC), as sent with
/// UpdateTaskInventory / RezScript. The thumbnail (none here) adds 0.
pub fn item_crc(i: &TaskItem) -> u32 {
    let perms = uuid_crc(&i.creator_id)
        .wrapping_add(uuid_crc(&i.owner_id))
        .wrapping_add(uuid_crc(&i.last_owner_id))
        .wrapping_add(uuid_crc(&i.group_id))
        .wrapping_add(i.base_mask)
        .wrapping_add(i.owner_mask)
        .wrapping_add(i.everyone_mask)
        .wrapping_add(i.group_mask);
    let sale = (i.sale_price as u32).wrapping_add((i.sale_type as u32).wrapping_mul(0x0707_3096));
    uuid_crc(&i.item_id)
        .wrapping_add(uuid_crc(&i.parent_id))
        .wrapping_add(perms)
        .wrapping_add(uuid_crc(&i.asset_id))
        .wrapping_add(i.asset_type as i32 as u32)
        .wrapping_add(i.inv_type as i32 as u32)
        .wrapping_add(i.flags)
        .wrapping_add(sale)
        .wrapping_add(i.creation_date as u32)
}

/// The capability answer `{inventory_serial, contents: [item LLSD]}`
/// (LLViewerObject::loadTaskInvLLSD); the "Contents" root folder is added
/// like the viewer does.
pub fn parse_cap(v: &Llsd) -> Result<Vec<TaskItem>, String> {
    if !matches!(&v["contents"], Llsd::Array(_)) {
        return Err("Réponse d'inventaire de l'objet invalide.".into());
    }
    Ok(v["contents"].as_array().iter().filter_map(item_from_llsd).take(10_000).collect())
}

/// Inventory serial of a capability answer.
pub fn cap_serial(v: &Llsd) -> Option<i16> {
    v.has("inventory_serial").then(|| v["inventory_serial"].as_i32() as i16)
}

/// An item (or folder) of the capability answer (LLInventoryItem::fromLLSD).
fn item_from_llsd(v: &Llsd) -> Option<TaskItem> {
    let id = v["item_id"].as_uuid();
    let folder_id = v["category_id"].as_uuid();
    if id.is_nil() && folder_id.is_nil() {
        return None;
    }
    let name_or_num = |x: &Llsd, f: fn(&str) -> i32| match x {
        Llsd::String(s) => f(s) as i8,
        other => other.as_i32() as i8,
    };
    if id.is_nil() {
        return Some(TaskItem {
            item_id: folder_id,
            parent_id: v["parent_id"].as_uuid(),
            name: v["name"].to_string_value(),
            asset_type: 8,
            inv_type: 8,
            is_folder: true,
            ..Default::default()
        });
    }
    let p = &v["permissions"];
    let s = &v["sale_info"];
    let asset_id = if v.has("asset_id") {
        v["asset_id"].as_uuid()
    } else {
        xor_magic(v["shadow_id"].as_uuid())
    };
    let mut item = TaskItem {
        item_id: id,
        parent_id: v["parent_id"].as_uuid(),
        creator_id: p["creator_id"].as_uuid(),
        owner_id: p["owner_id"].as_uuid(),
        last_owner_id: p["last_owner_id"].as_uuid(),
        group_id: p["group_id"].as_uuid(),
        group_owned: p["is_owner_group"].as_bool(),
        base_mask: p["base_mask"].as_u32(),
        owner_mask: p["owner_mask"].as_u32(),
        group_mask: p["group_mask"].as_u32(),
        everyone_mask: p["everyone_mask"].as_u32(),
        next_owner_mask: p["next_owner_mask"].as_u32(),
        asset_id,
        asset_type: name_or_num(&v["type"], asset_type_from_name),
        inv_type: name_or_num(&v["inv_type"], inv_type_from_name),
        flags: v["flags"].as_u32(),
        sale_type: match &s["sale_type"] {
            Llsd::String(t) => sale_type_from_name(t),
            other => other.as_i32() as u8,
        },
        sale_price: s["sale_price"].as_i32(),
        name: v["name"].to_string_value(),
        description: v["desc"].to_string_value(),
        creation_date: v["created_at"].as_i32(),
        is_folder: false,
    };
    if item.inv_type == IT_NONE {
        item.inv_type = default_inv_type(item.asset_type);
    }
    Some(item)
}

/// The legacy file (loadTaskInvFile); an item block left open is an error.
pub fn parse_legacy(bytes: &[u8]) -> Result<Vec<TaskItem>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Inventaire de l'objet illisible.".to_string())?;
    let opened = text.lines().filter(|l| l.trim() == "{").count();
    let closed = text.lines().filter(|l| l.trim() == "}").count();
    if opened != closed {
        return Err("Inventaire de l'objet incomplet.".into());
    }
    Ok(parse_task_inventory(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_nested_item_and_cap_use_the_same_model() {
        let bytes = b"inv_object 0\n{\nname Contents|\n}\ninv_item 0\n{\nitem_id 00000000-0000-0000-0000-000000000001\npermissions 0\n{\ngroup_mask 00008000\n}\ntype notecard\ninv_type notecard\nname Carte de bienvenue|\ndesc Test|\n}\n";
        let items: Vec<TaskItem> = parse_legacy(bytes).unwrap().into_iter().filter(|i| !i.is_folder).collect();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Carte de bienvenue");
        assert_eq!(items[0].asset_type, 7);
        assert_eq!(items[0].group_mask, 0x8000);
        assert!(parse_legacy(&bytes[..bytes.len() - 3]).is_err());
        assert!(parse_cap(&Llsd::Undef).is_err());
        let mut reply = Llsd::new_map();
        reply.insert("contents", Llsd::Array(Vec::new()));
        assert!(parse_cap(&reply).unwrap().is_empty());
    }

    #[test]
    fn cap_items_keep_permissions_and_sale() {
        let item = aurora_llsd::llsd_map! {
            "item_id" => Uuid::from_u128(5),
            "parent_id" => Uuid::from_u128(1),
            "type" => "lsltext",
            "inv_type" => "script",
            "name" => "Porte",
            "permissions" => aurora_llsd::llsd_map! { "owner_mask" => 0x7fff_ffffu32, "base_mask" => 0x7fff_ffffu32 },
            "sale_info" => aurora_llsd::llsd_map! { "sale_type" => "copy", "sale_price" => 25 },
        };
        let mut reply = Llsd::new_map();
        reply.insert("contents", Llsd::Array(vec![item]));
        reply.insert("inventory_serial", 7);
        let items = parse_cap(&reply).unwrap();
        assert_eq!(items[0].asset_type, 10);
        assert_eq!(items[0].owner_mask, 0x7fff_ffff);
        assert_eq!(items[0].sale_type, 2);
        assert_eq!(cap_serial(&reply), Some(7));
    }

    const FILE: &str = "\tinv_object\t0\n\t{\n\t\tobj_id\t11111111-1111-1111-1111-111111111111\n\t\tparent_id\t00000000-0000-0000-0000-000000000000\n\t\ttype\tcategory\n\t\tname\tContents|\n\t}\n\tinv_item\t0\n\t{\n\t\titem_id\t22222222-2222-2222-2222-222222222222\n\t\tparent_id\t11111111-1111-1111-1111-111111111111\n\tpermissions 0\n\t{\n\t\tbase_mask\t7fffffff\n\t\towner_mask\t7fffffff\n\t\tgroup_mask\t00000000\n\t\teveryone_mask\t00000000\n\t\tnext_owner_mask\t00082000\n\t\tcreator_id\t33333333-3333-3333-3333-333333333333\n\t\towner_id\t33333333-3333-3333-3333-333333333333\n\t\tlast_owner_id\t33333333-3333-3333-3333-333333333333\n\t\tgroup_id\t00000000-0000-0000-0000-000000000000\n\t}\n\t\tasset_id\t44444444-4444-4444-4444-444444444444\n\t\ttype\tlsltext\n\t\tinv_type\tscript\n\t\tflags\t00000000\n\tsale_info\t0\n\t{\n\t\tsale_type\tnot\n\t\tsale_price\t10\n\t}\n\t\tname\tNew Script|\n\t\tdesc\t2024-01-02 10:00:00 script|\n\t\tcreation_date\t1700000000\n\t}\n";

    #[test]
    fn parses_folder_and_script() {
        let items = parse_task_inventory(FILE);
        assert_eq!(items.len(), 2);
        assert!(items[0].is_folder);
        assert_eq!(items[0].name, "Contents");
        let s = &items[1];
        assert!(!s.is_folder);
        assert_eq!(s.name, "New Script");
        assert_eq!(s.description, "2024-01-02 10:00:00 script");
        assert_eq!(s.asset_type, 10);
        assert_eq!(s.inv_type, 10);
        assert_eq!(s.base_mask, 0x7fff_ffff);
        assert_eq!(s.next_owner_mask, 0x0008_2000);
        assert_eq!(s.sale_price, 10);
        assert_eq!(s.creation_date, 1_700_000_000);
        assert_eq!(s.creator_id, Uuid::from_u128(0x33333333_3333_3333_3333_333333333333));
    }

    #[test]
    fn shadow_ids_are_unmasked() {
        let a = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
        let text = format!("inv_item\t0\n{{\n\tshadow_id\t{}\n\ttype\ttexture\n}}\n", xor_magic(a));
        let items = parse_task_inventory(&text);
        assert_eq!(items[0].asset_id, a);
        assert_eq!(items[0].inv_type, 0);
    }

    #[test]
    fn crc_sums_like_ll() {
        let mut i = TaskItem::default();
        assert_eq!(item_crc(&i), 0);
        i.sale_type = 1;
        i.sale_price = 5;
        i.flags = 2;
        assert_eq!(item_crc(&i), 5 + 0x0707_3096 + 2);
    }

    #[test]
    fn empty_names_and_garbage() {
        let items = parse_task_inventory("inv_item\t0\n{\n\tname\t|\n}\n");
        assert_eq!(items[0].name, "");
        let _ = parse_task_inventory("inv_item\n{\npermissions\n{\n");
    }
}
