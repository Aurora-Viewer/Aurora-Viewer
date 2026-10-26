//! The photo's look (issue #969): one of the Legacy Viewer's photo filters
//! (LLImageFiltersManager, the Filter list of LLFloaterSnapshot,
//! indra/newview, originally LGPL 2.1), drawn on the GPU by
//! `aurora_render::looks` on the live view of the photo mode and on the
//! photo, at a strength the Legacy Viewer does not have, with a before /
//! after comparison of the live view. This names the looks and orders them
//! like the Legacy Viewer's list, by their translated name.

use aurora_render::looks::{LookOp, PhotoLook, legacy_look, legacy_looks};
use std::sync::Arc;

/// The translated name of a look (the Legacy Viewer's strings.xml).
pub fn name(id: &str) -> Option<&'static str> {
    Some(match id {
        "Antique" => aurora_i18n::tr!("photo-look-antique"),
        "Autocontrast" => aurora_i18n::tr!("photo-look-autocontrast"),
        "Badtrip" => aurora_i18n::tr!("photo-look-badtrip"),
        "BlackAndWhite" => aurora_i18n::tr!("photo-look-black-and-white"),
        "Blownhighlights" => aurora_i18n::tr!("photo-look-blown-highlights"),
        "Brighten" => aurora_i18n::tr!("photo-look-brighten"),
        "Cartoon" => aurora_i18n::tr!("photo-look-cartoon"),
        "Colors1970" => aurora_i18n::tr!("photo-look-colors-1970"),
        "Darken" => aurora_i18n::tr!("photo-look-darken"),
        "Edges" => aurora_i18n::tr!("photo-look-edges"),
        "Focus" => aurora_i18n::tr!("photo-look-focus"),
        "Heatwave" => aurora_i18n::tr!("photo-look-heatwave"),
        "Intense" => aurora_i18n::tr!("photo-look-intense"),
        "Julesverne" => aurora_i18n::tr!("photo-look-jules-verne"),
        "LensFlare" => aurora_i18n::tr!("photo-look-lens-flare"),
        "Lightleak" => aurora_i18n::tr!("photo-look-light-leak"),
        "Linearize" => aurora_i18n::tr!("photo-look-linearize"),
        "Miniature" => aurora_i18n::tr!("photo-look-miniature"),
        "Negative" => aurora_i18n::tr!("photo-look-negative"),
        "Newspaper" => aurora_i18n::tr!("photo-look-newspaper"),
        "Overcast" => aurora_i18n::tr!("photo-look-overcast"),
        "Posterize" => aurora_i18n::tr!("photo-look-posterize"),
        "Rotatecolors180" => aurora_i18n::tr!("photo-look-color-shift"),
        "Sepia" => aurora_i18n::tr!("photo-look-sepia"),
        "Sharpen" => aurora_i18n::tr!("photo-look-sharpen"),
        "Softfocus" => aurora_i18n::tr!("photo-look-soft-focus"),
        "Spotlight" => aurora_i18n::tr!("photo-look-spotlight"),
        "Thematrix" => aurora_i18n::tr!("photo-look-the-matrix"),
        "Toycamera" => aurora_i18n::tr!("photo-look-toy-camera"),
        "Video" => aurora_i18n::tr!("photo-look-video"),
        _ => return None,
    })
}

/// The name of the chosen look, "None" without one or with an unknown one.
pub fn label(id: Option<&str>) -> &'static str {
    id.and_then(name).unwrap_or(aurora_i18n::tr!("photo-look-none"))
}

/// The key a name sorts by: letters without their accents, any case (the
/// Legacy Viewer's map sorts the bytes, which puts É after Z).
fn sort_key(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// The looks offered, (id, name), in the order of their names.
pub fn list() -> Vec<(&'static str, &'static str)> {
    let mut looks: Vec<_> = legacy_looks().iter().filter_map(|(id, _)| Some((*id, name(id)?))).collect();
    looks.sort_by_cached_key(|(_, name)| sort_key(name));
    looks
}

/// The operations of a look.
pub fn ops(id: &str) -> Option<Arc<[LookOp]>> {
    legacy_look(id)
}

/// Where the photo is in a frame drawn and how big it is there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// The photo's top left corner, in pixels of the frame.
    pub origin: [f32; 2],
    /// The photo's size, in its own pixels.
    pub photo: (u32, u32),
    /// Pixels of the frame per pixel of the photo.
    pub scale: f32,
}

/// The look of a frame: `ops` at `strength` percent.
pub fn frame_look(ops: Arc<[LookOp]>, strength: u8, place: Placement, compare: Option<f32>, hold_histogram: bool) -> PhotoLook {
    PhotoLook {
        ops,
        intensity: f32::from(strength.min(100)) / 100.0,
        compare: compare.map(|c| c.clamp(0.0, 1.0)),
        image: [place.origin[0], place.origin[1], place.photo.0 as f32, place.photo.1 as f32],
        scale: place.scale,
        hold_histogram,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_look_has_a_name() {
        // a missing message gives its key back (the French catalogue has
        // every English key, aurora-i18n's tests)
        for (id, _) in legacy_looks() {
            let name = name(id).unwrap_or_else(|| panic!("{id} has no name"));
            assert!(!name.starts_with("photo-look-"), "{id}: {name}");
        }
        assert_eq!(list().len(), 30);
    }

    #[test]
    fn the_list_sorts_names_without_their_accents() {
        assert!(sort_key("Éclaircir") < sort_key("Fuite"));
        assert!(sort_key("Négatif") > sort_key("Matrix"));
    }

    #[test]
    fn a_frame_look_takes_its_strength_and_place() {
        let place = Placement {
            origin: [10.0, -4.0],
            photo: (3840, 2160),
            scale: 0.5,
        };
        let look = frame_look(ops("Sepia").expect("Sepia"), 140, place, Some(1.4), true);
        assert_eq!(look.intensity, 1.0);
        assert_eq!(look.compare, Some(1.0));
        assert_eq!(look.image, [10.0, -4.0, 3840.0, 2160.0]);
        assert!(look.hold_histogram);
        assert!(ops("Nothing").is_none());
        assert_eq!(label(Some("Nothing")), aurora_i18n::tr!("photo-look-none"));
    }
}
