//! LLPreviewNotecard, LLSettingsVOBase and LLMaterialEditor asset decoding.
//! Firestorm indra/newview, originally LGPL 2.1. Presentation stays in French.

pub fn can_edit(inv: &super::Inventory, item: uuid::Uuid) -> bool {
    !super::actions::library(inv, item)
        && inv.items.get(&item).is_some_and(|it| {
            matches!(it.asset_type, 7 | 10)
                && it.owner_mask & (super::actions::COPY | super::actions::MODIFY) == super::actions::COPY | super::actions::MODIFY
        })
}

pub fn text(kind: i32, data: &[u8]) -> Result<(String, bool), String> {
    if kind == 7 {
        return aurora_net::inventory::notecard::Notecard::decode(data).map(|card| (card.text, true));
    }
    if kind == 56 {
        use crate::world::eep::Settings;
        let settings = Settings::from_asset(data).ok_or(aurora_i18n::tr!("world-inventory-preview-the-settings-are-unreadable"))?;
        return Ok((
            match settings {
                Settings::Sky(s) => aurora_i18n::tr!(
                    "world-inventory-preview-sky-haze-density-cloud-coverage-cloud-scale-star-brightness",
                    value0 = aurora_i18n::number_text(&format!("{:.3}", s.haze_density)),
                    value1 = aurora_i18n::number_text(&format!("{:.3}", s.cloud_shadow)),
                    value2 = aurora_i18n::number_text(&format!("{:.3}", s.cloud_scale)),
                    value3 = aurora_i18n::number_text(&format!("{:.3}", s.star_brightness)),
                    value4 = aurora_i18n::number_text(&format!("{:.3}", s.moon_brightness))
                ),
                Settings::Water(w) => aurora_i18n::tr!(
                    "world-inventory-preview-water-haze-density-refraction-wave-scale-normal-texture",
                    value0 = aurora_i18n::number_text(&format!("{:.3}", w.fog_density)),
                    value1 = aurora_i18n::number_text(&format!("{:.3}", w.fresnel_scale)),
                    value2 = format!("{:?}", w.normal_scale.to_array()),
                    value3 = format!("{}", w.normal_map)
                ),
                Settings::Day(_) => aurora_i18n::tr!("world-inventory-preview-day-cycle-this-cycle-animates-sky-and-water-settings").into(),
            },
            false,
        ));
    }
    if kind == 57 {
        let material = aurora_assets::material::parse_material_asset(data)
            .map_err(|_| aurora_i18n::tr!("world-inventory-preview-the-material-is-unreadable"))?;
        return Ok((
            aurora_i18n::tr!(
                "world-inventory-preview-pbr-material-color-metallic-roughness-double-sided-base-color",
                value0 = format!("{:?}", material.base_color_factor),
                value1 = aurora_i18n::number_text(&format!("{:.3}", material.metallic_factor)),
                value2 = aurora_i18n::number_text(&format!("{:.3}", material.roughness_factor)),
                value3 = (if material.double_sided {
                    aurora_i18n::tr!("common-yes")
                } else {
                    aurora_i18n::tr!("common-no")
                })
                .to_string(),
                value4 = material
                    .base_color_texture
                    .map_or_else(|| aurora_i18n::tr!("common-none-2").into(), |id| id.to_string())
                    .to_string(),
                value5 = material
                    .normal_texture
                    .map_or_else(|| aurora_i18n::tr!("common-none-2").into(), |id| id.to_string())
                    .to_string(),
                value6 = material
                    .metallic_roughness_texture
                    .map_or_else(|| aurora_i18n::tr!("common-none-2").into(), |id| id.to_string())
                    .to_string(),
                value7 = material
                    .emissive_texture
                    .map_or_else(|| aurora_i18n::tr!("common-none-2").into(), |id| id.to_string())
                    .to_string()
            ),
            false,
        ));
    }
    let text = std::str::from_utf8(data).map_err(|_| aurora_i18n::tr!("world-inventory-preview-this-asset-is-not-a-text-document"))?;
    Ok((text.into(), matches!(kind, 7 | 10)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_permissions_require_copy_and_modify_and_exclude_library() {
        let mut inv = super::super::Inventory::default();
        crate::world::inventory::demo::seed(&mut inv, uuid::Uuid::from_u128(2));
        let id = uuid::Uuid::from_u128(8103);
        assert!(can_edit(&inv, id));
        for mask in [super::super::actions::COPY, super::super::actions::MODIFY, 0] {
            inv.items.get_mut(&id).expect("note").owner_mask = mask;
            assert!(!can_edit(&inv, id));
        }
        let item = inv.items.get_mut(&id).expect("note");
        item.owner_mask = super::super::actions::COPY | super::super::actions::MODIFY;
        item.parent = inv.lib_root;
        assert!(!can_edit(&inv, id));
    }
    #[test]
    fn notecard_lengths_are_utf8_bytes_and_invalid_sizes_never_panic() {
        let note = "Linden text version 2\n{\nLLEmbeddedItems version 1\n{\ncount 0\n}\nText length 2\né}\n";
        assert_eq!(text(7, note.as_bytes()).expect("note"), ("é".into(), true));
        assert!(text(7, note.replace("length 2", "length 1").as_bytes()).is_err());
        assert!(text(7, note.replace("length 2", &format!("length {}", usize::MAX)).as_bytes()).is_err());
        assert!(text(7, note.replace("count 0", "count 1").as_bytes()).is_err());
    }
    #[test]
    fn settings_preview_decodes_the_same_notation_header_as_the_environment() {
        let data = b"<?llsd/notation?>\n{'type':'sky'}";
        let (description, editable) = text(56, data).expect("sky");
        assert!(description.starts_with(&format!("{}\n", aurora_i18n::tr!("ui-environment-sky"))));
        assert!(!editable);
    }
}
