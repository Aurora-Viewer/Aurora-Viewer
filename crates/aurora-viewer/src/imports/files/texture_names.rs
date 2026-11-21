//! Inventory names of the textures a glTF material import creates.
//!
//! Port of LLMaterialEditor::getImageNameFromUri and the texture naming of
//! setFromGltfMetaData / saveTextures (indra/newview/llmaterialeditor.cpp,
//! originally LGPL 2.1): each map takes the base name of its image file and
//! the map type ("bricks (Base Color)", or "bricks_normal" when the creator
//! already named the type), and the upload prefixes the material name
//! ("chair (Wood): bricks (Base Color)").
//!
//! Difference, better for the user: an embedded image (GLB buffer view or
//! data URI) has no file name, and the Legacy Viewer then names the map by
//! its type alone ("Base Color"). The image's own glTF `name` (what Blender
//! and most exporters write) is used instead when there is one.

use aurora_assets::import_gltf::Document;

/// Map types in material slot order (base color, normal, metallic-roughness,
/// emissive). Inventory names, not interface text: English like the Legacy
/// Viewer's `MATERIAL_*_DEFAULT_NAME`, so names match across viewers.
pub(super) const SLOT_TYPES: [&str; 4] = ["Base Color", "Normal", "Metallic Roughness", "Emissive"];

/// LLMaterialEditor's 64-character cap on the base name, before the type.
const MAX_BASE: usize = 64;
/// DB_INV_ITEM_NAME_STR_LEN, applied by LLInventoryObject::correctInventoryName.
const MAX_ITEM: usize = 63;

/// The label of image `index`: its file's base name without extension, or
/// its `name` when it is embedded. Empty when it has neither.
pub(super) fn image_label(doc: &Document, index: usize) -> String {
    let image = &doc.json["images"][index];
    let uri = image["uri"].as_str().filter(|uri| !uri.starts_with("data:"));
    match uri {
        Some(uri) => base_name(&super::decode_uri(uri).unwrap_or_else(|_| uri.to_owned())),
        None => image["name"].as_str().unwrap_or_default().trim().to_owned(),
    }
}

/// LLDir::getBaseFileName(uri, true) with both delimiters unified.
fn base_name(uri: &str) -> String {
    let file = uri.rsplit(['/', '\\']).next().unwrap_or(uri);
    match file.rfind('.') {
        Some(dot) if dot > 0 => file[..dot].to_owned(),
        _ => file.to_owned(),
    }
}

/// LLMaterialEditor::getImageNameFromUri on an image label.
pub(super) fn texture_name(label: &str, kind: &str) -> String {
    let label: String = if label.chars().count() > MAX_BASE {
        label.chars().take(MAX_BASE - 1).collect()
    } else {
        label.to_owned()
    };
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_lowercase();
    let name = if squash(&label).contains(&squash(kind)) {
        label
    } else if label.is_empty() {
        kind.to_owned()
    } else {
        format!("{label} ({kind})")
    };
    crate::text_validation::generated_name(&name, MAX_ITEM)
}

/// LLMaterialEditor::saveTextures: "<material>: <texture>", for ease of
/// inventory management.
pub(crate) fn upload_name(material: &str, texture: &str) -> String {
    crate::text_validation::generated_name(&format!("{material}: {texture}"), MAX_ITEM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_the_type_unless_the_creator_named_it() {
        assert_eq!(texture_name("bricks", "Base Color"), "bricks (Base Color)");
        assert_eq!(texture_name("Bricks_BaseColor", "Base Color"), "Bricks_BaseColor");
        assert_eq!(texture_name("wall normal", "Normal"), "wall normal");
        // The Legacy Viewer's known miss: "metallic_roughness" keeps the type.
        assert_eq!(
            texture_name("metal_roughness", "Metallic Roughness"),
            "metal_roughness (Metallic Roughness)"
        );
        assert_eq!(texture_name("", "Emissive"), "Emissive");
    }

    #[test]
    fn caps_inventory_names() {
        let long = "x".repeat(100);
        assert_eq!(texture_name(&long, "Normal").len(), MAX_ITEM);
        assert_eq!(upload_name(&"m".repeat(60), "bricks (Base Color)").len(), MAX_ITEM);
        assert_eq!(
            upload_name("chair (Wood)", "bricks (Base Color)"),
            "chair (Wood): bricks (Base Color)"
        );
        assert_eq!(texture_name("a|b", "Normal"), "a?b (Normal)");
    }

    #[test]
    fn labels_from_file_uri_or_embedded_name() {
        let data = br#"{"asset":{"version":"2.0"},"images":[
            {"uri":"textures/Wood%20Planks.v2.png"},
            {"uri":"..\\maps\\steel.jpg","name":"ignored"},
            {"bufferView":0,"name":"Leather_Normal"},
            {"uri":"data:image/png;base64,AAAA","name":"Embedded"},
            {"bufferView":0},
            {"uri":".hidden"}
        ]}"#;
        let doc = Document::parse(data, |_| panic!("no resource reads")).expect("glTF");
        let labels: Vec<_> = (0..6).map(|i| image_label(&doc, i)).collect();
        assert_eq!(labels, ["Wood Planks.v2", "steel", "Leather_Normal", "Embedded", "", ".hidden"]);
    }
}
