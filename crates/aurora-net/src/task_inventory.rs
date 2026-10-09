//! LLViewerObject::loadTaskInvLLSD / loadTaskInvFile (Firestorm, LGPL 2.1).
//! Task inventory remains separate from the resident's inventory.
use crate::inventory::{InvItem, item_from_llsd};
use aurora_llsd::Llsd;
use uuid::Uuid;

pub fn parse_cap(v: &Llsd) -> Result<Vec<InvItem>, String> {
    if !matches!(&v["contents"], Llsd::Array(_)) {
        return Err("Réponse d'inventaire de l'objet invalide.".into());
    }
    Ok(v["contents"].as_array().iter().filter_map(item_from_llsd).collect())
}

/// Legacy inv_item blocks: nested permissions/sale_info, pipe-terminated names.
pub fn parse_legacy(bytes: &[u8]) -> Result<Vec<InvItem>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Inventaire de l'objet illisible.")?;
    let mut items = Vec::new();
    let mut entry = None;
    let mut depth = 0i32;
    for line in text.lines().map(str::trim) {
        if line.starts_with("inv_item") && entry.is_none() {
            entry = Some(Llsd::new_map());
            depth = 0;
            continue;
        }
        let Some(v) = entry.as_mut() else { continue };
        match line {
            "{" => depth += 1,
            "}" => {
                depth -= 1;
                if depth == 0 {
                    if let Some(item) = item_from_llsd(v) {
                        items.push(item);
                    }
                    entry = None;
                }
            }
            _ => {
                let Some((key, value)) = line.split_once(char::is_whitespace) else {
                    continue;
                };
                let value = value.trim().trim_end_matches('|');
                match key {
                    "item_id" | "parent_id" | "asset_id" => {
                        if let Ok(id) = value.parse::<Uuid>() {
                            v.insert(key, id);
                        }
                    }
                    "creator_id" | "owner_id" => {
                        let mut perms = v["permissions"].clone();
                        if perms.as_map().is_none() {
                            perms = Llsd::new_map();
                        }
                        if let Ok(id) = value.parse::<Uuid>() {
                            perms.insert(key, id);
                        }
                        v.insert("permissions", perms);
                    }
                    "group_mask" | "everyone_mask" | "next_owner_mask" => {
                        let mut perms = v["permissions"].clone();
                        if perms.as_map().is_none() {
                            perms = Llsd::new_map();
                        }
                        if let Ok(mask) = u32::from_str_radix(value, 16) {
                            perms.insert(key, mask);
                        }
                        v.insert("permissions", perms);
                    }
                    "flags" => {
                        if let Ok(flags) = u32::from_str_radix(value, 16) {
                            v.insert(key, flags);
                        }
                    }
                    "creation_date" => {
                        if let Ok(date) = value.parse::<i32>() {
                            v.insert("created_at", date);
                        }
                    }
                    "name" | "desc" | "type" | "inv_type" => {
                        v.insert(key, value);
                    }
                    _ => {}
                }
            }
        }
    }
    if entry.is_some() {
        return Err("Inventaire de l'objet incomplet.".into());
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_nested_item_and_cap_use_the_same_model() {
        let bytes = b"inv_object 0\n{\nname Contents|\n}\ninv_item 0\n{\nitem_id 00000000-0000-0000-0000-000000000001\npermissions 0\n{\ngroup_mask 00008000\n}\ntype notecard\ninv_type notecard\nname Carte de bienvenue|\ndesc Test|\n}\n";
        let items = parse_legacy(bytes).unwrap();
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
}
