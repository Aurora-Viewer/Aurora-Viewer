//! LLWearable::exportStream, LLViewerWearable::{setParamsToDefaults,
//! setTexturesToDefaults} (Firestorm indra/llappearance and newview, LGPL 2.1).

use std::collections::BTreeMap;
use uuid::Uuid;

const TYPES: [&str; 17] = [
    "shape",
    "skin",
    "hair",
    "eyes",
    "shirt",
    "pants",
    "shoes",
    "socks",
    "jacket",
    "gloves",
    "undershirt",
    "underpants",
    "skirt",
    "alpha",
    "tattoo",
    "physics",
    "universal",
];

pub fn create(kind: u8, name: &str, owner: Uuid) -> Result<Vec<u8>, String> {
    let Some(kind_name) = TYPES.get(kind as usize) else {
        return Err("Type de vêtement inconnu.".into());
    };
    let data = std::fs::read(crate::scene::avatar::AvatarLibrary::data_dir().join("avatar_lad.xml"))
        .map_err(|_| "Les définitions de l’avatar sont indisponibles.".to_owned())?;
    let xml = aurora_llsd::xml::parse(&data).map_err(|_| "Les définitions de l’avatar sont illisibles.".to_owned())?;
    let mut params = BTreeMap::<i32, f32>::new();
    let mut stack = vec![&xml];
    while let Some(el) = stack.pop() {
        if el.name == "param"
            && el.attr("wearable") == Some(kind_name)
            && matches!(el.attr("group"), None | Some("0") | Some("3"))
            && let Some(id) = el.attr("id").and_then(|s| s.parse().ok())
        {
            params
                .entry(id)
                .or_insert_with(|| el.attr("value_default").and_then(|s| s.parse().ok()).unwrap_or(0.0));
        }
        stack.extend(el.elements());
    }
    let version = xml
        .attr("wearable_definition_version")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(22);
    let mut out = format!(
        "LLWearable version {version}\n{name}\n\npermissions 0\n{{\nbase_mask 7fffffff\nowner_mask 7fffffff\ngroup_mask 00000000\neveryone_mask 00000000\nnext_owner_mask 7fffffff\ncreator_id {owner}\nowner_id {owner}\nlast_owner_id {owner}\ngroup_id {}\n}}\nsale_info 0\n{{\nsale_type not\nsale_price 10\n}}\ntype {kind}\nparameters {}\n",
        Uuid::nil(),
        params.len()
    );
    for (id, value) in params {
        out.push_str(&format!("{id} {value}\n"));
    }
    let indices: &[u8] = match kind {
        0 | 15 => &[],
        1 => &[0, 5, 6],
        2 => &[4],
        3 => &[3],
        4 => &[1],
        5 => &[2],
        6 => &[7],
        7 => &[12],
        8 => &[13, 14],
        9 => &[15],
        10 => &[16],
        11 => &[17],
        12 => &[18],
        13 => &[21, 22, 23, 24, 25],
        14 => &[26, 27, 28],
        16 => &[29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39],
        _ => &[],
    };
    let texture = match kind {
        2 => "7ca39b4c-bd19-4699-aff7-f93fd03d3e7b",
        3 => "6522e74d-1660-4e7f-b601-6f48c1659a77",
        1 | 14 | 16 => "c228d1cf-4b5d-4ba8-84f4-899a0796aa97",
        _ => "5748decc-f629-461c-9a36-a35a221fe21f",
    };
    out.push_str(&format!("textures {}\n", indices.len()));
    for index in indices {
        out.push_str(&format!("{index} {texture}\n"));
    }
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_create_submenu_type_has_a_valid_default_asset() {
        for kind in 0..17 {
            let bytes = create(kind, "Nouveau vêtement", Uuid::from_u128(1)).expect("default wearable");
            let text = String::from_utf8(bytes).expect("UTF-8");
            assert!(text.starts_with("LLWearable version 22\n"));
            assert!(text.contains(&format!("\ntype {kind}\n")));
            assert!(text.contains("\nparameters "));
            assert!(text.contains("\ntextures "));
            assert!(text.len() < 64 * 1024);
        }
        assert!(create(17, "Invalide", Uuid::nil()).is_err());
    }
}
