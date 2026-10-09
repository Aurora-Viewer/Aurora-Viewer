//! LLPreviewNotecard, LLSettingsVOBase and LLMaterialEditor asset decoding.
//! Firestorm indra/newview, originally LGPL 2.1. Presentation stays in French.

pub fn text(kind: i32, data: &[u8]) -> Result<(String, bool), String> {
    if kind == 56 {
        use crate::world::eep::Settings;
        let settings = Settings::from_asset(data).ok_or("Les paramètres sont illisibles.")?;
        return Ok((match settings {
            Settings::Sky(s) => format!("Ciel\n\nDensité de brume : {:.3}\nCouverture nuageuse : {:.3}\nÉchelle des nuages : {:.3}\nLuminosité des étoiles : {:.3}\nLuminosité de la lune : {:.3}", s.haze_density, s.cloud_shadow, s.cloud_scale, s.star_brightness, s.moon_brightness),
            Settings::Water(w) => format!("Eau\n\nDensité de brume : {:.3}\nRéfraction : {:.3}\nÉchelle des vagues : {:?}\nTexture des normales : {}", w.fog_density, w.fresnel_scale, w.normal_scale.to_array(), w.normal_map),
            Settings::Day(_) => "Cycle du jour\n\nCe cycle anime les paramètres du ciel et de l’eau. Utilisez « Appliquer uniquement à moi-même » pour le prévisualiser dans le monde.".into(),
        }, false));
    }
    if kind == 57 {
        let material = aurora_assets::material::parse_material_asset(data).map_err(|_| "Le matériau est illisible.")?;
        return Ok((
            format!(
                "Matériau PBR\n\nCouleur : {:?}\nMétal : {:.3}\nRugosité : {:.3}\nDouble face : {}\nTexture de couleur : {}\nTexture des normales : {}\nTexture métal / rugosité : {}\nTexture émissive : {}",
                material.base_color_factor,
                material.metallic_factor,
                material.roughness_factor,
                if material.double_sided { "Oui" } else { "Non" },
                material.base_color_texture.map_or_else(|| "Aucune".into(), |id| id.to_string()),
                material.normal_texture.map_or_else(|| "Aucune".into(), |id| id.to_string()),
                material
                    .metallic_roughness_texture
                    .map_or_else(|| "Aucune".into(), |id| id.to_string()),
                material.emissive_texture.map_or_else(|| "Aucune".into(), |id| id.to_string())
            ),
            false,
        ));
    }
    let text = std::str::from_utf8(data).map_err(|_| "Cet asset n’est pas un document texte.")?;
    if kind == 7 && text.starts_with("Linden text version") {
        let pos = text.find("\nText length ").map(|p| p + 1).ok_or("Note invalide.")?;
        let line_end = text[pos..].find('\n').map(|n| n + pos).ok_or("Note invalide.")?;
        let len: usize = text[pos + 12..line_end].trim().parse().map_err(|_| "Taille de note invalide.")?;
        let end = (line_end + 1).checked_add(len).ok_or("Taille de note invalide.")?;
        let content = text.get(line_end + 1..end).ok_or("Note tronquée.")?;
        return Ok((content.into(), text.contains("\ncount 0\n")));
    }
    Ok((text.into(), matches!(kind, 7 | 10)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notecard_lengths_are_utf8_bytes_and_invalid_sizes_never_panic() {
        let note = "Linden text version 2\n{\nLLEmbeddedItems version 1\n{\ncount 0\n}\nText length 2\né}\n";
        assert_eq!(text(7, note.as_bytes()).expect("note"), ("é".into(), true));
        assert!(text(7, note.replace("length 2", "length 1").as_bytes()).is_err());
        assert!(text(7, note.replace("length 2", &format!("length {}", usize::MAX)).as_bytes()).is_err());
        assert!(!text(7, note.replace("count 0", "count 1").as_bytes()).expect("embedded note").1);
    }
    #[test]
    fn settings_preview_decodes_the_same_notation_header_as_the_environment() {
        let data = b"<?llsd/notation?>\n{'type':'sky'}";
        let (description, editable) = text(56, data).expect("sky");
        assert!(description.starts_with("Ciel\n"));
        assert!(!editable);
    }
}
