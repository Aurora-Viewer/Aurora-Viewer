//! System folders hidden from the inventory (Preferences › Aurora › System
//! folders), and the folders with nothing in them (feedback #533). A hidden
//! folder only leaves the inventory windows' trees; it stays on the server
//! with everything in it, and comes back as soon as it is switched on again.
//!
//! The settings (`InventoryPreferences::hidden_system_folders` and
//! `hide_empty_folders`) are turned into a bit mask once a frame by `sync`,
//! and `ViewCache::children_iter` leaves those folders out.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The folders that can be hidden, in the page's order, with their system
/// folder type (LLFolderType). The name is what the settings keep; a folder
/// is matched by its type, never by its (translated) name. `#Aurora` and
/// `#Firestorm` have no type: each is the folder of that name at the root.
pub const HIDEABLE: &[(&str, Option<i32>)] = &[
    ("Animations", Some(20)),
    ("Body Parts", Some(13)),
    ("Calling Cards", Some(2)),
    ("Clothing", Some(5)),
    ("Current Outfit", Some(46)),
    ("Favorites", Some(23)),
    ("Landmarks", Some(3)),
    ("Marketplace Listings", Some(53)),
    ("My Outfits", Some(48)),
    ("Notecards", Some(7)),
    ("Objects", Some(6)),
    ("Settings", Some(56)),
    ("Textures", Some(0)),
    ("#Aurora", None),
    ("#Firestorm", None),
];

/// The folder's name on the page, in the viewer's language.
pub fn label(name: &str) -> &'static str {
    use aurora_i18n::tr;
    match name {
        "Animations" => tr!("ui-options-system-folder-animations"),
        "Body Parts" => tr!("ui-options-system-folder-body-parts"),
        "Calling Cards" => tr!("ui-options-system-folder-calling-cards"),
        "Clothing" => tr!("ui-options-system-folder-clothing"),
        "Current Outfit" => tr!("ui-options-system-folder-current-outfit"),
        "Favorites" => tr!("ui-options-system-folder-favorites"),
        "Landmarks" => tr!("ui-options-system-folder-landmarks"),
        "Marketplace Listings" => tr!("ui-options-system-folder-marketplace-listings"),
        "My Outfits" => tr!("ui-options-system-folder-my-outfits"),
        "Notecards" => tr!("ui-options-system-folder-notecards"),
        "Objects" => tr!("ui-options-system-folder-objects"),
        "Settings" => tr!("ui-options-system-folder-settings"),
        "Textures" => tr!("ui-options-system-folder-textures"),
        "#Aurora" => tr!("ui-options-system-folder-aurora"),
        _ => tr!("ui-options-system-folder-firestorm"),
    }
}

/// The bits of the folders without a type, by name (the folder types stay
/// below them).
const NAMED: &[(&str, u32)] = &[("#Aurora", 63), ("#Firestorm", 62)];

static MASK: AtomicU64 = AtomicU64::new(0);
static HIDE_EMPTY: AtomicBool = AtomicBool::new(false);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn bit(name: &str) -> Option<u32> {
    HIDEABLE.iter().find(|(n, _)| *n == name).and_then(|(n, kind)| match kind {
        Some(k) => u32::try_from(*k).ok(),
        None => NAMED.iter().find(|(named, _)| named == n).map(|(_, b)| *b),
    })
}

/// Takes the settings (once a frame): the hidden folders' names and Hide
/// empty folders. A change lists the trees again.
pub fn sync(hidden: &[String], hide_empty: bool) {
    let mask = hidden.iter().filter_map(|n| bit(n)).fold(0u64, |m, b| m | (1 << b));
    let changed = MASK.swap(mask, Ordering::Relaxed) != mask;
    if HIDE_EMPTY.swap(hide_empty, Ordering::Relaxed) != hide_empty || changed {
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
}

/// Folders with nothing in them, system folders included, are left out.
pub fn hide_empty() -> bool {
    HIDE_EMPTY.load(Ordering::Relaxed)
}

/// The hidden folders' bits.
pub fn mask() -> u64 {
    MASK.load(Ordering::Relaxed)
}

/// Changes whenever the hidden folders change.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The bit a folder at the root answers to, if it is one that can be hidden.
pub fn folder_bit(parent_is_root: bool, type_default: i32, name: &str) -> Option<u32> {
    if !parent_is_root {
        return None;
    }
    if type_default >= 0 {
        return HIDEABLE
            .iter()
            .any(|(_, k)| *k == Some(type_default))
            .then(|| u32::try_from(type_default).ok())
            .flatten();
    }
    NAMED.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `sync` is not called here: its mask is shared by every test's trees
    #[test]
    fn names_and_folders_become_bits() {
        assert_eq!(bit("Clothing"), Some(5));
        assert_eq!(bit("Textures"), Some(0));
        assert_eq!(bit("#Aurora"), Some(63));
        assert_eq!(bit("Unknown"), None);
        assert_eq!(folder_bit(true, 5, "Clothing"), Some(5));
        assert_eq!(folder_bit(false, 5, "Clothing"), None, "only at the root");
        assert_eq!(folder_bit(true, -1, "#Aurora"), Some(63));
        assert_eq!(folder_bit(true, -1, "#Firestorm"), Some(62));
        assert_eq!(folder_bit(true, 14, "Trash"), None, "the Trash cannot be hidden");
        assert_eq!(folder_bit(true, -1, "My stuff"), None);
        assert!(!hide_empty(), "off until Preferences turns it on");
    }

    #[test]
    fn every_hideable_folder_has_a_label() {
        for (name, _) in HIDEABLE {
            assert!(!label(name).is_empty(), "{name}");
        }
        assert_eq!(label("#Aurora"), "#Aurora");
    }
}
