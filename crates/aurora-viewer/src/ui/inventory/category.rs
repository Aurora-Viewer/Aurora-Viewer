//! The inventory's categories and their symbols: one table for the icons of
//! items, of system folders and of the type filters, so a kind of content
//! looks the same everywhere.
//!
//! A system folder's icon is Tabler's folder with the category's symbol cut
//! out of it. Those SVGs are generated (scripts/generate-inventory-icons.py,
//! committed in assets/icons/aurora/folders/); a test checks each one names
//! the symbol of this table. Icon names follow the Legacy Viewer's
//! (llviewerfoldertype.cpp `ViewerFolderEntry`, llinventoryicon.cpp), so a
//! skin overrides a folder by the file name it already uses.

/// The kinds of content the inventory gives a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Textures,
    Sounds,
    CallingCards,
    Landmarks,
    Clothing,
    Objects,
    Notecards,
    Scripts,
    BodyParts,
    PhotoAlbum,
    LostAndFound,
    Animations,
    Gestures,
    Favorites,
    CurrentOutfit,
    Outfits,
    Meshes,
    Received,
    Marketplace,
    Settings,
    Materials,
}

/// One category's row: its symbol and the icons made from it.
pub struct Row {
    pub category: Category,
    /// Tabler icon name (`-filled` for the filled set).
    pub symbol: &'static str,
    /// The item icon drawn with the same symbol, when items of the category
    /// have one (`Inv_Texture`); `None` for folders only.
    pub item: Option<&'static str>,
    /// LLFolderType values whose system folder shows the category
    /// (llfoldertype.h); the first is also its asset type for colours.
    pub folder_types: &'static [i32],
    /// Bit of the type filter (LLInventoryType, view::types()).
    pub filter_type: Option<i32>,
    /// Folder icon names: closed, open, and the link to the closed folder.
    pub closed: &'static str,
    pub open: &'static str,
    pub link: &'static str,
    /// Generated SVGs: closed, open, closed with the shortcut corner.
    pub svgs: [&'static [u8]; 3],
}

macro_rules! folder_svgs {
    ($file:literal) => {
        [
            include_bytes!(concat!("../../../../../assets/icons/aurora/folders/", $file, "-closed.svg")),
            include_bytes!(concat!("../../../../../assets/icons/aurora/folders/", $file, "-open.svg")),
            include_bytes!(concat!("../../../../../assets/icons/aurora/folders/", $file, "-closed-link.svg")),
        ]
    };
}

macro_rules! row {
    ($category:ident, $symbol:literal, $item:expr, $types:expr, $filter:expr, $name:literal, $file:literal) => {
        Row {
            category: Category::$category,
            symbol: $symbol,
            item: $item,
            folder_types: $types,
            filter_type: $filter,
            closed: concat!("Inv_", $name, "Closed"),
            open: concat!("Inv_", $name, "Open"),
            link: concat!("Inv_", $name, "Closed_Link"),
            svgs: folder_svgs!($file),
        }
    };
}

/// Every category, in the order of the type filters' labels where they
/// have one. Names after `Inv_` are the Legacy Viewer's, except `CurrentOutfit`
/// and `Marketplace` (the Legacy Viewer draws those as Inv_LookFolder / Inv_Sys).
pub const ROWS: &[Row] = &[
    row!(Textures, "photo-filled", Some("Inv_Texture"), &[0], Some(0), "Textures", "textures"),
    row!(Sounds, "file-music-filled", Some("Inv_Sound"), &[1], Some(1), "Sound", "sounds"),
    row!(
        CallingCards,
        "id-filled",
        Some("Inv_CallingCard"),
        &[2],
        Some(2),
        "CallingCard",
        "calling-cards"
    ),
    row!(
        Landmarks,
        "map-pin-filled",
        Some("Inv_Landmark"),
        &[3],
        Some(3),
        "Landmarks",
        "landmarks"
    ),
    row!(
        Clothing,
        "shirt-filled",
        Some("Inv_Clothing"),
        &[5],
        Some(18),
        "Clothing",
        "clothing"
    ),
    row!(Objects, "box", Some("Inv_Object"), &[6], Some(6), "Objects", "objects"),
    row!(
        Notecards,
        "file-text-filled",
        Some("Inv_Notecard"),
        &[7],
        Some(7),
        "Notecards",
        "notecards"
    ),
    row!(Scripts, "code", Some("Inv_Script"), &[10], Some(10), "Scripts", "scripts"),
    row!(
        BodyParts,
        "man-filled",
        Some("Inv_BodyShape"),
        &[13],
        None,
        "BodyParts",
        "body-parts"
    ),
    row!(PhotoAlbum, "library-photo", None, &[15], Some(15), "Snapshots", "photo-album"),
    row!(LostAndFound, "help-circle-filled", None, &[16], None, "Lost", "lost-and-found"),
    row!(
        Animations,
        "run",
        Some("Inv_Animation"),
        &[20],
        Some(19),
        "Animations",
        "animations"
    ),
    row!(Gestures, "hand-move", Some("Inv_Gesture"), &[21], Some(20), "Gestures", "gestures"),
    row!(Favorites, "star-filled", None, &[23], None, "Favorites", "favorites"),
    row!(
        CurrentOutfit,
        "hanger-2-filled",
        None,
        &[46],
        None,
        "CurrentOutfit",
        "current-outfit"
    ),
    row!(Outfits, "hanger", None, &[47, 48], None, "LookFolder", "outfits"),
    row!(Meshes, "cube-3d-sphere", Some("Inv_Mesh"), &[49], Some(22), "Meshes", "meshes"),
    row!(Received, "inbox", None, &[50], None, "Inbox", "inbox"),
    // listings, their stock and version folders (FT_MARKETPLACE_*)
    row!(
        Marketplace,
        "building-store",
        None,
        &[53, 54, 55],
        None,
        "Marketplace",
        "marketplace"
    ),
    row!(
        Settings,
        "sun-filled",
        Some("Inv_Settings"),
        &[56],
        Some(25),
        "Settings",
        "settings"
    ),
    row!(Materials, "sphere", Some("Inv_Material"), &[57], Some(26), "Materials", "materials"),
];

impl Category {
    pub fn row(self) -> &'static Row {
        ROWS.iter().find(|row| row.category == self).expect("every category has a row")
    }

    /// The category a system folder shows, by its preferred type.
    pub fn of_folder(type_default: i32) -> Option<Self> {
        ROWS.iter()
            .find(|row| row.folder_types.contains(&type_default))
            .map(|row| row.category)
    }

    /// The category of a type filter (a bit of `Filters::types`).
    pub fn of_filter(filter_type: i32) -> Option<Self> {
        ROWS.iter().find(|row| row.filter_type == Some(filter_type)).map(|row| row.category)
    }
}

/// The symbol of a type filter and its tint, the colour of the items it
/// keeps (`item_icon_color`), so a filter looks like the rows it selects.
pub fn filter_glyph(filter_type: i32, p: &crate::theme::Palette) -> Option<(&'static str, egui::Color32)> {
    let row = Category::of_filter(filter_type)?.row();
    let asset_type = row.folder_types.first().copied().unwrap_or(-1);
    Some((row.symbol, super::item_icon_color(asset_type, filter_type, p)))
}

/// Whether `name` is the icon of a folder (drawn in the folders' tint).
pub fn is_folder_icon(name: &str) -> bool {
    name.starts_with("Inv_Folder") || ROWS.iter().any(|row| row.closed == name || row.open == name)
}

/// The link icon of a system folder's closed icon, if `name` is one.
pub fn folder_link(name: &str) -> Option<&'static str> {
    ROWS.iter().find(|row| row.closed == name).map(|row| row.link)
}

/// A clothing icon Tabler has no symbol for, drawn by hand (Zakka, for
/// Aurora; assets/icons/aurora/clothing/, a BRANDING.md exception).
pub struct ClothingDrawing {
    pub item: &'static str,
    /// The Tabler icon shown when `drawing` is `None` (a new kind waiting
    /// for its drawing).
    pub stand_in: &'static str,
    /// The hand-drawn icon: it replaces the stand-in.
    pub drawing: Option<&'static [u8]>,
}

/// Pants, Skirt and Underpants have no Tabler symbol of their own: their
/// icons are drawn by hand. Their link icons are made by
/// scripts/generate-inventory-icons.py (`LINKS`).
pub const CLOTHING_DRAWINGS: &[ClothingDrawing] = &[
    ClothingDrawing {
        item: "Inv_Pants",
        stand_in: "walk",
        drawing: Some(include_bytes!("../../../../../assets/icons/aurora/clothing/pants.svg")),
    },
    ClothingDrawing {
        item: "Inv_Skirt",
        stand_in: "woman-filled",
        drawing: Some(include_bytes!("../../../../../assets/icons/aurora/clothing/skirt.svg")),
    },
    ClothingDrawing {
        item: "Inv_Underpants",
        stand_in: "diaper",
        drawing: Some(include_bytes!("../../../../../assets/icons/aurora/clothing/underpants.svg")),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_folders_follow_legacy_folder_types() {
        // llfoldertype.h values, checked against the Legacy Viewer's sources
        for (folder_type, category) in [
            (0, Category::Textures),
            (1, Category::Sounds),
            (2, Category::CallingCards),
            (3, Category::Landmarks),
            (5, Category::Clothing),
            (6, Category::Objects),
            (7, Category::Notecards),
            (10, Category::Scripts),
            (13, Category::BodyParts),
            (15, Category::PhotoAlbum),
            (16, Category::LostAndFound),
            (20, Category::Animations),
            (21, Category::Gestures),
            (23, Category::Favorites),
            (46, Category::CurrentOutfit),
            (47, Category::Outfits),
            (48, Category::Outfits),
            (49, Category::Meshes),
            (50, Category::Received),
            (53, Category::Marketplace),
            (54, Category::Marketplace),
            (55, Category::Marketplace),
            (56, Category::Settings),
            (57, Category::Materials),
        ] {
            assert_eq!(Category::of_folder(folder_type), Some(category), "folder type {folder_type}");
        }
        // root, trash, plain folders, My Suitcase and the Legacy Viewer's own (58-60) keep
        // their icons of tree::folder_icon
        for folder_type in [-1, 8, 14, 51, 52, 58, 59, 60, 100] {
            assert_eq!(Category::of_folder(folder_type), None, "folder type {folder_type}");
        }
    }

    #[test]
    fn every_type_filter_has_a_symbol() {
        for (kind, label) in super::super::view::types() {
            assert!(Category::of_filter(*kind).is_some(), "filter {label} ({kind})");
        }
    }

    #[test]
    fn rows_are_unique() {
        let mut names: Vec<&str> = ROWS.iter().flat_map(|row| [row.closed, row.open, row.link]).collect();
        names.extend(ROWS.iter().filter_map(|row| row.item));
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "an icon name is used twice");
        let mut types: Vec<i32> = ROWS.iter().flat_map(|row| row.folder_types.iter().copied()).collect();
        let count = types.len();
        types.sort_unstable();
        types.dedup();
        assert_eq!(types.len(), count, "a folder type has two categories");
    }

    #[test]
    fn generated_folders_cut_out_their_category_symbol() {
        // the generator writes `aurora-generated: <folder> + <symbol>`
        for row in ROWS {
            for (svg, folder) in row.svgs.iter().zip(["folder-filled", "folder-open-filled", "folder-filled"]) {
                let text = std::str::from_utf8(svg).expect("utf-8 SVG");
                let mark = format!("aurora-generated: {folder} + {} ", row.symbol);
                assert!(
                    text.contains(&mark),
                    "{:?}: expected `{mark}`, regenerate with scripts/generate-inventory-icons.py",
                    row.category
                );
            }
        }
    }
}
