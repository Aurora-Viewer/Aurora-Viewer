//! UI icons: Tabler SVGs (MIT, see assets/icons/tabler/LICENSE), rasterized
//! at startup as white alpha masks so they can be tinted with the skin
//! palette. Names follow Tabler: `eye-off` is the outline icon, `eye-filled`
//! the filled one. Legacy Firestorm icon names are mapped onto Tabler ones;
//! skins can override any icon with a PNG or SVG of the same name.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Mutex;

use super::inventory::category;

macro_rules! svg_list {
    ($dir:literal, $suffix:literal: $($name:literal),* $(,)?) => {
        &[$((
            concat!($name, $suffix),
            include_bytes!(concat!("../../assets/icons/tabler/", $dir, "/", $name, ".svg")) as &[u8],
        )),*]
    };
}

/// Embedded Tabler outline icons, by Tabler name.
const OUTLINE: &[(&str, &[u8])] = svg_list!(
    "outline", "":
    "antenna-bars-5",
    "aperture",
    "armchair",
    "arrow-back-up",
    "arrow-bar-to-down",
    "arrow-down",
    "arrow-forward-up",
    "arrow-left",
    "arrow-right",
    "arrow-up",
    "arrows-down-up",
    "arrows-exchange",
    "arrows-horizontal",
    "arrows-maximize",
    "arrows-minimize",
    "arrows-move",
    "arrows-sort",
    "arrows-split",
    "arrows-vertical",
    "at",
    "ban",
    "baseline-density-small",
    "bell-off",
    "box",
    "box-model-2",
    "brand-flickr",
    "brush",
    "building-castle",
    "building-store",
    "bulb-off",
    "cake",
    "camera-cog",
    "camera-off",
    "chart-line",
    "check",
    "chevron-down",
    "chevron-up",
    "chevron-left",
    "chevron-right",
    "chevrons-down",
    "chevrons-right",
    "chevrons-up",
    "circle-minus",
    "cloud-rain",
    "cloud-storm",
    "code",
    "code-asterisk",
    "code-off",
    "color-filter",
    "color-picker",
    "cpu",
    "credit-card-off",
    "crosshair",
    "crown",
    "cube",
    "cube-3d-sphere",
    "cut",
    "database-export",
    "database-import",
    "deselect",
    "device-desktop-analytics",
    "device-desktop-off",
    "dimensions",
    "diaper",
    "door-enter",
    "door-exit",
    "dots",
    "eraser",
    "external-link",
    "eye-closed",
    "eye-off",
    "file-export",
    "file-import",
    "focus-2",
    "focus-centered",
    "fingerprint",
    "flip-horizontal",
    "gavel",
    "gif",
    "grid-3x3",
    "grip-horizontal",
    "grip-vertical",
    "hammer-off",
    "hand-grab",
    "hand-move",
    "hand-click",
    "hand-finger",
    "hand-stop",
    "hanger",
    "heart-handshake",
    "heart-rate-monitor",
    "history",
    "home-check",
    "ice-skating",
    "id-off",
    "inbox",
    "indent-decrease",
    "indent-increase",
    "jacket",
    "label-off",
    "layers-intersect",
    "layers-subtract",
    "layers-union",
    "layout-columns",
    "library-photo",
    "license",
    "link",
    "loader-2",
    "lock-open",
    "lock-cog",
    "mist",
    "pencil-off",
    "login-2",
    "logout",
    "map",
    "map-2",
    "math-not",
    "maximize",
    "message-circle-plus",
    "message-off",
    "microphone-off",
    "minimize",
    "mouse",
    "mouse-2",
    "movie",
    "music",
    "network",
    "needle",
    "packages",
    "note",
    "minus",
    "package",
    "package-import",
    "paperclip",
    "phone-off",
    "plane-off",
    "plus",
    "power",
    "question-mark",
    "radar-2",
    "radio",
    "refresh",
    "refresh-dot",
    "refresh-off",
    "reload",
    "repeat",
    "restore",
    "rotate-2",
    "resize",
    "ripple",
    "rotate",
    "rotate-3d",
    "rotate-360",
    "rotate-clockwise",
    "rotate-clockwise-2",
    "route",
    "rss",
    "run",
    "scan-eye",
    "select-all",
    "separator-horizontal",
    "separator-vertical",
    "scissors",
    "shape",
    "share",
    "shield-off",
    "shovel",
    "shirt-sport",
    "shoe",
    "slash",
    "slashes",
    "snowflake",
    "sock",
    "sort-ascending-letters",
    "sort-descending-letters",
    "speakerphone",
    "sphere",
    "spiral",
    "square",
    "star",
    "stack-2",
    "stopwatch",
    "stretching",
    "sun-moon",

    "swimming",
    "terminal-2",
    "text-size",
    "timeline",
    "tree",
    "typography",
    "unlink",
    "upload",
    "user",
    "user-cog",
    "user-heart",
    "user-check",
    "user-circle",
    "user-minus",
    "user-off",
    "user-plus",
    "user-scan",
    "user-search",
    "user-square",
    "user-x",
    "users",
    "users-group",
    "users-minus",
    "users-plus",
    "vector",
    "volume",
    "volume-2",
    "volume-3",
    "volume-off",
    "walk",
    "wand",
    "wave-sine",
    "wifi-off",
    "wind",
    "world-search",
    "x",
);

/// Embedded Tabler filled icons, named `<Tabler name>-filled`.
const FILLED: &[(&str, &[u8])] = svg_list!(
    "filled", "-filled":
    "accessible",
    "adjustments-horizontal",
    "archive",
    "adjustments",
    "alarm-snooze",
    "alert-triangle",
    "arrow-big-up",
    "arrow-big-up-lines",
    "app-window",
    "apple",
    "arrow-autofit-width",
    "article",
    "aspect-ratio",
    "backspace",
    "ball-bowling",
    "barrier-block",
    "bell",
    "bell-ringing",
    "bell-z",
    "bolt",
    "bone",
    "book",
    "bookmark",
    "bookmarks",
    "bounce-right",
    "bounce-right",
    "box-align-bottom",
    "brand-discord",
    "brand-github",
    "brand-youtube",
    "briefcase",
    "briefcase-2",
    "brightness",
    "bug",
    "building-broadcast-tower",
    "bulb",
    "calendar",
    "calendar-event",
    "camera",
    "car",
    "caret-down",
    "caret-left",
    "caret-right",
    "caret-up",
    "carousel-horizontal",
    "chart-area-line",
    "chart-candle",
    "chart-dots",
    "circle",
    "chart-area-line",
    "circle-check",
    "circle-plus",
    "circle-x",
    "click",
    "clipboard-data",
    "clipboard-text",
    "clock",
    "cloud",
    "cloud-data-connection",
    "code-circle",
    "coin",
    "columns-2",
    "columns-3",
    "compass",
    "contrast",
    "confetti",
    "copy",
    "crown",
    "current-location",
    "dashboard",
    "database",
    "device-cctv",
    "device-desktop",
    "device-floppy",
    "device-gamepad",
    "device-gamepad-2",
    "device-vision-pro",
    "credit-card",
    "device-speaker",
    "device-tv",
    "dots",
    "download",
    "droplet",
    "edit",
    "external-link",
    "eye",
    "file-description",
    "file-download",
    "file-info",
    "file-music",
    "file-text",
    "file-typography",
    "file-upload",
    "files",
    "filter",
    "filters",
    "flag",
    "flask",
    "folder",
    "folder-open",
    "forbid",
    "gauge",
    "ghost",
    "ghost-2",
    "gift",
    "grid-pattern",
    "hanger-2",
    "headphones",
    "headset",
    "heart",
    "help-circle",
    "home",
    "home-2",
    "hourglass",
    "id",
    "info-circle",
    "key",
    "keyboard",
    "keyframe",
    "label",
    "layout",
    "layout-align-bottom",
    "layout-align-center",
    "layout-align-left",
    "layout-align-middle",
    "layout-align-right",
    "layout-align-top",
    "layout-board-split",
    "layout-bottombar",
    "layout-cards",
    "layout-dashboard",
    "layout-sidebar-left-collapse",
    "layout-sidebar-left-expand",
    "layout-sidebar-right",
    "layout-sidebar-right-collapse",
    "layout-sidebar-right-expand",
    "layout-grid",
    "layout-navbar-collapse",
    "layout-navbar-expand",
    "leaf",
    "library",
    "link",
    "list",
    "list-details",
    "location",
    "lock",
    "lock-square-rounded",
    "man",
    "woman",
    "mail",
    "map-pin",
    "medical-cross",
    "menu-2",
    "message",
    "message-chatbot",
    "message-circle",
    "messages",
    "microphone",
    "mood-smile",
    "moon",
    // Aurora-drawn Luau mark for SLua scripts, after the Legacy Viewer's
    // Inv_Script_Luau: Tabler has no such icon (decision on PR #2802, docs/BRANDING.md)
    "luau",
    "mountain",
    "mouse",
    "navigation",
    "paint",
    "palette",
    "panorama-horizontal",
    "paw",
    "pencil",
    "phone",
    "photo",
    "picture-in-picture",
    "pin",
    "pinned",
    "plane-arrival",
    "plane-departure",
    "plane-tilt",
    "player-eject",
    "player-pause",
    "player-play",
    "player-record",
    "player-skip-back",
    "player-skip-forward",
    "player-stop",
    "player-track-next",
    "player-track-prev",
    "playlist",
    "point",
    "pointer",
    "quote",
    "radar",
    "receipt",
    "replace",
    "rosette-discount-check",
    "search",
    "seedling",
    "send",
    "settings",
    "shield",
    "shield-lock",
    "shirt",
    "shopping-cart",
    "sort-ascending-2",
    "tilt-shift",
    "sparkles",
    "square-arrow-down",
    "square-arrow-left",
    "square-arrow-right",
    "square-arrow-up",
    "square-check",
    "square-f1",
    "square-f2",
    "square-f3",
    "square-f4",
    "square-f5",
    "square-f6",
    "square-f7",
    "square-f8",
    "square-f9",
    "square-letter-a",
    "square-letter-b",
    "square-letter-c",
    "square-letter-d",
    "square-letter-e",
    "square-letter-f",
    "square-letter-g",
    "square-letter-h",
    "square-letter-i",
    "square-letter-j",
    "square-letter-k",
    "square-letter-l",
    "square-letter-m",
    "square-letter-n",
    "square-letter-o",
    "square-letter-p",
    "square-letter-q",
    "square-letter-r",
    "square-letter-s",
    "square-letter-t",
    "square-letter-u",
    "square-letter-v",
    "square-letter-w",
    "square-letter-x",
    "square-letter-y",
    "square-letter-z",
    "square-minus",
    "square-number-0",
    "square-number-1",
    "square-number-2",
    "square-number-3",
    "square-number-4",
    "square-number-5",
    "square-number-6",
    "square-number-7",
    "square-number-8",
    "square-number-9",
    "square-rounded-minus",
    "square-rounded-chevron-right",
    "square-rounded-chevrons-down",
    "square-rounded-chevrons-up",
    "square-rounded-plus",
    "stack",
    "star",
    "steering-wheel",
    "stack-2",
    "sun",
    "sun-high",
    "sunrise",
    "sunset-2",
    "tag",
    "tags",
    "thumb-down",
    "ticket",
    "toggle-right",
    "trash",
    "trash-x",
    "umbrella",
    "user",
    "video",
    "world",
    "zoom",
    "zoom-in",
    "zoom-out",
    "zoom-scan",
);

/// Brand SVGs (white wolf silhouette of assets/branding/), drawn untinted or tinted.
const BRAND_SVGS: &[(&str, &[u8])] = &[("wolf", include_bytes!("../../../../assets/branding/aurora-logo-white.svg"))];

/// Raster images kept as PNG (brand logo).
const PNGS: &[(&str, &[u8])] = &[("phoenix_18", include_bytes!("../../assets/icons/phoenix_18.png"))];

/// Legacy Viewer icon names used by skins / layouts -> Tabler icon. The
/// inventory's category symbols (`Inv_Texture`…) and the hand-drawn clothing
/// icons are in `inventory::category`; see `alias_target`.
const ALIASES: &[(&str, &str)] = &[
    ("Command_Chat_Icon", "messages-filled"),
    ("Command_Speak_Icon", "microphone-filled"),
    ("Command_Voice_Icon", "headphones-filled"),
    ("Command_Move_Icon", "walk"),
    ("Command_View_Icon", "eye-filled"),
    ("Command_People_Icon", "users"),
    ("Command_Appearance_Icon", "shirt-filled"),
    ("Command_Search_Icon", "search-filled"),
    ("Command_Map_Icon", "map"),
    ("Command_Snapshot_Icon", "camera-filled"),
    ("Command_Inventory_Icon", "briefcase-filled"),
    ("Command_MiniMap_Icon", "compass-filled"),
    ("Command_Performance_Icon", "gauge-filled"),
    ("Command_Preferences_Icon", "settings-filled"),
    ("Command_Build_Icon", "box"),
    ("Command_Gestures_Icon", "hand-move"),
    ("Command_Environments_Icon", "sunset-2-filled"),
    ("Command_Places_Icon", "map-pin-filled"),
    ("groundsit", "armchair"),
    ("mouselook", "crosshair"),
    ("radar", "radar-filled"),
    ("statistics", "chart-area-line-filled"),
    ("nearbychat_18", "message-filled"),
    ("Home_Off", "home-filled"),
    ("Stop_Off", "player-stop-filled"),
    ("Inv_FolderClosed", "folder-filled"),
    ("Inv_FolderOpen", "folder-open-filled"),
    ("Inv_Object_Multi", "packages"),
    ("Inv_Skin", "fingerprint"),
    ("Inv_Hair", "scissors"),
    ("Inv_Eye", "eye-filled"),
    ("Inv_Shirt", "shirt-filled"),
    ("Inv_Shoe", "shoe"),
    ("Inv_Socks", "sock"),
    ("Inv_Jacket", "jacket"),
    ("Inv_Gloves", "hand-stop"),
    ("Inv_Alpha", "layers-subtract"),
    ("Inv_Tattoo", "needle"),
    ("Inv_Physics", "wave-sine"),
    ("Inv_Universal", "layers-union"),
    ("Inv_Script_Luau", "luau-filled"),
    ("Inv_Snapshot", "camera-filled"),
    // settings items by kind (LLSettingsType, llinventoryicon.cpp)
    ("Inv_SettingsSky", "cloud-filled"),
    ("Inv_SettingsWater", "ripple"),
    ("Inv_SettingsDay", "sun-moon"),
    ("Inv_LinkItem", "link-filled"),
    ("Inv_LinkFolder", "link-filled"),
    ("Inv_TrashClosed", "trash-filled"),
    ("Inv_SuitcaseClosed", "briefcase-filled"),
    ("Inv_SuitcaseOpen", "briefcase-filled"),
    ("Inv_SysClosed", "folder-filled"),
    ("Inv_Invalid", "help-circle-filled"),
];

// Target-type artwork with the human-authored corner shortcut layer.
const INVENTORY_LINKS: &[(&str, &[u8])] = &[
    (
        "Inv_FolderClosed_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/folder-link.svg"),
    ),
    (
        "Inv_FolderOpen_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/folder-open-link.svg"),
    ),
    (
        "Inv_Object_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/box-link.svg"),
    ),
    (
        "Inv_Object_Multi_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/packages-link.svg"),
    ),
    (
        "Inv_Texture_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/photo-link.svg"),
    ),
    (
        "Inv_Notecard_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/file-text-link.svg"),
    ),
    (
        "Inv_Landmark_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/map-pin-link.svg"),
    ),
    (
        "Inv_Clothing_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/shirt-link.svg"),
    ),
    (
        "Inv_BodyShape_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/man-link.svg"),
    ),
    (
        "Inv_Skin_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/fingerprint-link.svg"),
    ),
    (
        "Inv_Hair_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/scissors-link.svg"),
    ),
    (
        "Inv_Eye_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/eye-link.svg"),
    ),
    (
        "Inv_Shirt_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/shirt-link.svg"),
    ),
    (
        "Inv_Pants_Link",
        include_bytes!("../../../../assets/icons/aurora/links/pants-link.svg"),
    ),
    (
        "Inv_Shoe_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/shoe-link.svg"),
    ),
    (
        "Inv_Socks_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/sock-link.svg"),
    ),
    (
        "Inv_Jacket_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/jacket-link.svg"),
    ),
    (
        "Inv_Gloves_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/hand-stop-link.svg"),
    ),
    (
        "Inv_Undershirt_Link",
        include_bytes!("../../../../assets/icons/aurora/links/undershirt-link.svg"),
    ),
    (
        "Inv_Underpants_Link",
        include_bytes!("../../../../assets/icons/aurora/links/underpants-link.svg"),
    ),
    (
        "Inv_Skirt_Link",
        include_bytes!("../../../../assets/icons/aurora/links/skirt-link.svg"),
    ),
    (
        "Inv_Alpha_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/layers-subtract-link.svg"),
    ),
    (
        "Inv_Tattoo_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/needle-link.svg"),
    ),
    (
        "Inv_Physics_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/wave-sine-link.svg"),
    ),
    (
        "Inv_Universal_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/layers-union-link.svg"),
    ),
    (
        "Inv_Script_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/code-link.svg"),
    ),
    (
        "Inv_Script_Luau_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/luau-link.svg"),
    ),
    (
        "Inv_Sound_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/file-music-link.svg"),
    ),
    (
        "Inv_Animation_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/run-link.svg"),
    ),
    (
        "Inv_Gesture_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/hand-move-link.svg"),
    ),
    (
        "Inv_Snapshot_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/camera-link.svg"),
    ),
    (
        "Inv_Mesh_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/cube-3d-sphere-link.svg"),
    ),
    (
        "Inv_Material_Link",
        include_bytes!("../../../../assets/icons/aurora/links/sphere-link.svg"),
    ),
    (
        "Inv_Settings_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/sun-link.svg"),
    ),
    (
        "Inv_SettingsSky_Link",
        include_bytes!("../../../../assets/icons/aurora/links/cloud-filled-link.svg"),
    ),
    (
        "Inv_SettingsWater_Link",
        include_bytes!("../../../../assets/icons/aurora/links/ripple-link.svg"),
    ),
    (
        "Inv_SettingsDay_Link",
        include_bytes!("../../../../assets/icons/aurora/links/sun-moon-link.svg"),
    ),
    (
        "Inv_CallingCard_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/id-link.svg"),
    ),
    (
        "Inv_TrashClosed_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/trash-link.svg"),
    ),
    (
        "Inv_SysClosed_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/folder-link.svg"),
    ),
    (
        "Inv_Invalid_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/help-circle-link.svg"),
    ),
];

pub(crate) fn inventory_link_name(base: &str) -> &'static str {
    match base {
        "Inv_FolderClosed" => "Inv_FolderClosed_Link",
        "Inv_FolderOpen" => "Inv_FolderOpen_Link",
        "Inv_Object" => "Inv_Object_Link",
        "Inv_Object_Multi" => "Inv_Object_Multi_Link",
        "Inv_Texture" => "Inv_Texture_Link",
        "Inv_Notecard" => "Inv_Notecard_Link",
        "Inv_Landmark" => "Inv_Landmark_Link",
        "Inv_Clothing" => "Inv_Clothing_Link",
        "Inv_BodyShape" => "Inv_BodyShape_Link",
        "Inv_Skin" => "Inv_Skin_Link",
        "Inv_Hair" => "Inv_Hair_Link",
        "Inv_Eye" => "Inv_Eye_Link",
        "Inv_Shirt" => "Inv_Shirt_Link",
        "Inv_Pants" => "Inv_Pants_Link",
        "Inv_Shoe" => "Inv_Shoe_Link",
        "Inv_Socks" => "Inv_Socks_Link",
        "Inv_Jacket" => "Inv_Jacket_Link",
        "Inv_Gloves" => "Inv_Gloves_Link",
        "Inv_Undershirt" => "Inv_Undershirt_Link",
        "Inv_Underpants" => "Inv_Underpants_Link",
        "Inv_Skirt" => "Inv_Skirt_Link",
        "Inv_Alpha" => "Inv_Alpha_Link",
        "Inv_Tattoo" => "Inv_Tattoo_Link",
        "Inv_Physics" => "Inv_Physics_Link",
        "Inv_Universal" => "Inv_Universal_Link",
        "Inv_Script" => "Inv_Script_Link",
        "Inv_Script_Luau" => "Inv_Script_Luau_Link",
        "Inv_Sound" => "Inv_Sound_Link",
        "Inv_Animation" => "Inv_Animation_Link",
        "Inv_Gesture" => "Inv_Gesture_Link",
        "Inv_Snapshot" => "Inv_Snapshot_Link",
        "Inv_Mesh" => "Inv_Mesh_Link",
        "Inv_Material" => "Inv_Material_Link",
        "Inv_Settings" => "Inv_Settings_Link",
        "Inv_SettingsSky" => "Inv_SettingsSky_Link",
        "Inv_SettingsWater" => "Inv_SettingsWater_Link",
        "Inv_SettingsDay" => "Inv_SettingsDay_Link",
        "Inv_CallingCard" => "Inv_CallingCard_Link",
        "Inv_TrashClosed" => "Inv_TrashClosed_Link",
        "Inv_SysClosed" => "Inv_SysClosed_Link",
        "Inv_Invalid" => "Inv_Invalid_Link",
        _ => category::folder_link(base).unwrap_or("Inv_LinkItem"),
    }
}

/// Whether `name` is an embedded icon or one of the skin's aliases.
pub(crate) fn embedded(name: &str) -> bool {
    OUTLINE
        .iter()
        .chain(FILLED)
        .chain(INVENTORY_LINKS)
        .chain(BRAND_SVGS)
        .chain(PNGS)
        .any(|(n, _)| *n == name)
        || alias_target(name).is_some()
        || generated_folder(name).is_some()
}

/// The embedded icon an alias stands for: the Legacy Viewer's names of `ALIASES`,
/// the inventory's item icons by category and the clothing icons' stand-ins.
pub(crate) fn alias_target(name: &str) -> Option<&'static str> {
    ALIASES
        .iter()
        .copied()
        .chain(
            category::CLOTHING_DRAWINGS
                .iter()
                .map(|clothing| (clothing.item, clothing.stand_in)),
        )
        .find(|(alias, _)| *alias == name)
        .map(|(_, target)| target)
        .or_else(|| category::ROWS.iter().find(|row| row.item == Some(name)).map(|row| row.symbol))
}

/// A system folder's generated SVG and whether it carries the shortcut
/// corner (a link to the folder), by icon name.
fn generated_folder(name: &str) -> Option<(&'static [u8], bool)> {
    category::drawn_folders()
        .find(|(n, _, _)| *n == name)
        .map(|(_, svg, shortcut)| (svg, shortcut))
}

/// Raster size of SVG icons (crisp up to ~2x UI scale at 24 px).
const RASTER_PX: u32 = 64;

/// Raster size of the large copies of inventory icons (`Icons::large`):
/// the gallery draws a tile's icon up to 92 points, 184 pixels at 200%.
/// The UI renderer has no mipmaps, so the small icons keep `RASTER_PX`
/// rather than one raster for both sizes.
const LARGE_RASTER_PX: u32 = 192;

/// Raster size of the brand wolf, drawn at 18 px (twice that at most with the
/// UI scale). Sampled down from far larger without mipmaps, its thin aurora
/// ribbons broke into jagged dots; resvg draws them clean at this size.
const BRAND_RASTER_PX: u32 = 40;

/// Last loaded icon set, for widgets drawn without access to the skin
/// (floater title buttons).
static GLOBAL: std::sync::RwLock<Option<HashMap<String, egui::TextureHandle>>> = std::sync::RwLock::new(None);

/// Icon from the current skin, by name.
pub fn global(name: &str) -> Option<egui::TextureHandle> {
    GLOBAL.read().ok()?.as_ref()?.get(name).cloned()
}

/// Available bundled and skin icons for the button appearance picker.
pub fn names() -> Vec<String> {
    let mut names: Vec<_> = GLOBAL
        .read()
        .ok()
        .and_then(|icons| icons.as_ref().map(|icons| icons.keys().cloned().collect()))
        .unwrap_or_default();
    names.sort_unstable();
    names
}

#[derive(Default)]
pub struct Icons {
    map: HashMap<String, egui::TextureHandle>,
    /// SVG of each icon by name (aliases included, a skin's files over the
    /// embedded ones), for `large`.
    sources: HashMap<String, Source>,
    /// Large copies made so far (`None`: the icon has no SVG).
    large: Mutex<HashMap<String, Option<egui::TextureHandle>>>,
}

#[derive(Clone)]
enum Source {
    Svg(Cow<'static, [u8]>),
    Shortcut(&'static [u8]),
    /// A skin's PNG: no larger copy.
    Png,
}

fn rasterize_svg(bytes: &[u8], px: u32) -> Option<egui::ColorImage> {
    rasterize_mask(bytes, px, false)
}

fn rasterize_mask(bytes: &[u8], px: u32, shortcut: bool) -> Option<egui::ColorImage> {
    // A black shortcut outline knocks out the base glyph. Reading the rendered
    // premultiplied white channel retains that gap and antialiasing in our tinted mask.
    let source = shortcut.then(|| {
        String::from_utf8_lossy(bytes)
            .replace("currentColor", "#ffffff")
            .replace("#202222", "#000000")
    });
    let bytes = source.as_ref().map_or(bytes, |s| s.as_bytes());
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).ok()?;
    let size = tree.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    let scale = px as f32 / size.width().max(size.height()).max(1.0);
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // white mask with the icon's coverage as alpha (tinted when drawn)
    let rgba: Vec<u8> = pixmap
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [255, 255, 255, if shortcut { p[0] } else { p[3] }])
        .collect();
    Some(egui::ColorImage::from_rgba_unmultiplied([px as usize, px as usize], &rgba))
}

/// Re-center an icon on its visible content (brand artwork does not sit in
/// the middle of its canvas): crop to the alpha bounds and pad to a square.
fn center_on_content(ci: egui::ColorImage) -> egui::ColorImage {
    let [w, h] = ci.size;
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if ci.pixels[y * w + x].a() > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 || y0 > y1 {
        return ci;
    }
    let (cw, ch) = (x1 - x0 + 1, y1 - y0 + 1);
    let side = cw.max(ch);
    let (ox, oy) = ((side - cw) / 2, (side - ch) / 2);
    let mut out = egui::ColorImage::filled([side, side], egui::Color32::TRANSPARENT);
    for y in 0..ch {
        for x in 0..cw {
            out.pixels[(y + oy) * side + x + ox] = ci.pixels[(y + y0) * w + x + x0];
        }
    }
    out
}

fn decode_png(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
}

impl Icons {
    /// Built-in icons, overridden/extended by PNG or SVG files in `override_dir`.
    pub fn load(ctx: &egui::Context, override_dir: Option<&std::path::Path>) -> Icons {
        let mut map: HashMap<String, egui::TextureHandle> = HashMap::new();
        let opts = egui::TextureOptions::LINEAR;
        let mut sources: HashMap<String, Source> = HashMap::new();
        for (name, bytes) in OUTLINE.iter().chain(FILLED) {
            sources.insert((*name).to_owned(), Source::Svg(Cow::Borrowed(*bytes)));
            match rasterize_svg(bytes, RASTER_PX) {
                Some(ci) => {
                    map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
                }
                None => log::warn!("icon {name}: invalid SVG"),
            }
        }
        let folders = category::drawn_folders();
        for (name, bytes, shortcut) in INVENTORY_LINKS.iter().map(|(name, bytes)| (*name, *bytes, true)).chain(folders) {
            sources.insert(
                name.to_owned(),
                if shortcut {
                    Source::Shortcut(bytes)
                } else {
                    Source::Svg(Cow::Borrowed(bytes))
                },
            );
            if let Some(ci) = rasterize_mask(bytes, RASTER_PX, shortcut) {
                map.insert(name.to_owned(), ctx.load_texture(name, ci, opts));
            } else {
                log::warn!("icon {name}: invalid SVG");
            }
        }
        for (name, bytes) in BRAND_SVGS {
            if let Some(ci) = rasterize_svg(bytes, BRAND_RASTER_PX).map(center_on_content) {
                map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
            }
        }
        for (name, bytes) in PNGS {
            if let Some(ci) = decode_png(bytes) {
                map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
            }
        }
        let items = category::ROWS.iter().filter_map(|row| row.item.map(|item| (item, row.symbol)));
        let clothing = category::CLOTHING_DRAWINGS
            .iter()
            .map(|clothing| (clothing.item, clothing.stand_in));
        for (alias, target) in ALIASES.iter().copied().chain(clothing).chain(items) {
            if let Some(t) = map.get(target).cloned() {
                map.insert(alias.to_owned(), t);
            }
            if let Some(source) = sources.get(target).cloned() {
                sources.insert(alias.to_owned(), source);
            }
        }
        // hand-drawn clothing icons replace their stand-ins
        for clothing in category::CLOTHING_DRAWINGS {
            if let Some(bytes) = clothing.drawing
                && let Some(ci) = rasterize_svg(bytes, RASTER_PX)
            {
                map.insert(clothing.item.to_owned(), ctx.load_texture(clothing.item, ci, opts));
                sources.insert(clothing.item.to_owned(), Source::Svg(Cow::Borrowed(bytes)));
            }
        }
        // the names the skin gives an image of its own
        let mut skinned = std::collections::HashSet::new();
        if let Some(dir) = override_dir
            && let Ok(rd) = std::fs::read_dir(dir)
        {
            for e in rd.flatten() {
                let path = e.path();
                let ext = path.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase());
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let ci = match ext.as_deref() {
                    Some("png") => decode_png(&bytes),
                    Some("svg") => rasterize_svg(&bytes, RASTER_PX),
                    _ => None,
                };
                if let Some(ci) = ci {
                    skinned.insert(stem.to_owned());
                    map.insert(stem.to_owned(), ctx.load_texture(stem, ci, opts));
                    let source = if ext.as_deref() == Some("svg") {
                        Source::Svg(Cow::Owned(bytes))
                    } else {
                        Source::Png
                    };
                    sources.insert(stem.to_owned(), source);
                }
            }
        }
        // a skin made for the Legacy Viewer: its images of a system folder's
        // Legacy names (Inv_LookFolder*, Inv_Sys*, Inv_Folder*) win over
        // Aurora's own icon for that folder (`category::skin_image`)
        for (name, _, _) in category::drawn_folders() {
            if let Some(image) = category::skin_image(name, |n| skinned.contains(n)).filter(|image| *image != name)
                && let (Some(t), Some(source)) = (map.get(image).cloned(), sources.get(image).cloned())
            {
                map.insert(name.to_owned(), t);
                sources.insert(name.to_owned(), source);
            }
        }
        if let Ok(mut g) = GLOBAL.write() {
            *g = Some(map.clone());
        }
        Icons {
            map,
            sources,
            large: Mutex::default(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&egui::TextureHandle> {
        self.map.get(name)
    }

    /// The icon for drawing `points` wide: well past the usual raster's
    /// pixels, its large raster (`large`); below, the usual one. The UI
    /// renderer has no mipmaps, so the switch sits at 1.5x: neither raster is
    /// shrunk more than 2x nor stretched more than 1.5x up to 288 pixels.
    pub fn sized(&self, ctx: &egui::Context, name: &str, points: f32) -> Option<egui::TextureHandle> {
        if points * ctx.pixels_per_point() > RASTER_PX as f32 * 1.5 {
            self.large(ctx, name)
        } else {
            self.get(name).cloned()
        }
    }

    /// The icon rasterized for drawing large (the gallery's tiles), made the
    /// first time it is asked for; the usual one when it has no SVG.
    pub fn large(&self, ctx: &egui::Context, name: &str) -> Option<egui::TextureHandle> {
        let made = self.large.lock().ok().and_then(|large| large.get(name).cloned());
        let texture = match made {
            Some(texture) => texture,
            None => {
                let image = match self.sources.get(name) {
                    Some(Source::Svg(bytes)) => rasterize_svg(bytes, LARGE_RASTER_PX),
                    Some(Source::Shortcut(bytes)) => rasterize_mask(bytes, LARGE_RASTER_PX, true),
                    Some(Source::Png) | None => None,
                };
                let texture = image.map(|ci| ctx.load_texture(format!("{name}@large"), ci, egui::TextureOptions::LINEAR));
                if let Ok(mut large) = self.large.lock() {
                    large.insert(name.to_owned(), texture.clone());
                }
                texture
            }
        };
        texture.or_else(|| self.get(name).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_svg_rasterizes() {
        for (name, bytes) in OUTLINE.iter().chain(FILLED) {
            let ci = rasterize_svg(bytes, 32).unwrap_or_else(|| panic!("{name} failed"));
            assert!(ci.pixels.iter().any(|p| p.a() > 0), "{name} is empty");
        }
    }

    #[test]
    fn inventory_shortcuts_rasterize_and_keep_the_outline_as_a_gap() {
        let ctx = egui::Context::default();
        let icons = Icons::load(&ctx, None);
        for (name, bytes) in INVENTORY_LINKS {
            assert!(icons.get(name).is_some(), "missing {name}");
            let image = rasterize_mask(bytes, 240, true).expect("shortcut SVG");
            assert!(image.pixels.iter().any(|p| p.a() > 0), "empty {name}");
            // The template's upright shortcut stem occupies x=0.408..1.888.
            // Its dark outside edge must stay transparent, not become a solid blob.
            assert_eq!(image.pixels[230 * 240 + 3].a(), 0, "outline {name}");
            assert_eq!(image.pixels[230 * 240 + 10].a(), 255, "arrow {name}");
        }
    }

    #[test]
    fn the_rendering_types_menu_icons_are_embedded() {
        let names: Vec<&str> = OUTLINE.iter().chain(FILLED).map(|(name, _)| *name).collect();
        for icon in crate::ui::bars::RENDER_TYPE_ICONS {
            assert!(names.contains(&icon), "{icon} is not embedded");
        }
    }

    #[test]
    fn aliases_point_to_embedded_icons() {
        let names: Vec<&str> = OUTLINE.iter().chain(FILLED).map(|(name, _)| *name).collect();
        for (alias, target) in ALIASES {
            assert!(names.contains(target), "{alias} -> {target} is not embedded");
        }
        for clothing in category::CLOTHING_DRAWINGS {
            assert!(
                names.contains(&clothing.stand_in),
                "{} -> {} is not embedded",
                clothing.item,
                clothing.stand_in
            );
        }
        for row in category::ROWS {
            assert!(names.contains(&row.symbol), "{:?} -> {} is not embedded", row.category, row.symbol);
        }
    }

    #[test]
    fn category_icons_and_legacy_aliases_do_not_overlap() {
        // one name, one source: `alias_target` and `Icons::load` would
        // otherwise disagree on which wins
        let rows: Vec<&str> = category::ROWS
            .iter()
            .flat_map(|row| [Some(row.closed), Some(row.open), Some(row.link), row.item])
            .flatten()
            .chain(category::CLOTHING_DRAWINGS.iter().map(|clothing| clothing.item))
            .collect();
        for (alias, _) in ALIASES {
            assert!(!rows.contains(alias), "{alias} is both an alias and a category icon");
        }
        for (name, _) in INVENTORY_LINKS {
            assert!(!rows.contains(name), "{name} is both a link and a category icon");
        }
    }

    #[test]
    fn hand_drawn_clothing_icons_replace_their_stand_ins() {
        let ctx = egui::Context::default();
        let icons = Icons::load(&ctx, None);
        for clothing in category::CLOTHING_DRAWINGS {
            let drawing = clothing.drawing.expect("drawn");
            let image = rasterize_svg(drawing, RASTER_PX).expect("clothing SVG");
            assert!(image.pixels.iter().any(|p| p.a() > 0), "{} is empty", clothing.item);
            let item = icons.get(clothing.item).expect("loaded");
            let stand_in = icons.get(clothing.stand_in).expect("stand-in");
            assert_ne!(item.id(), stand_in.id(), "{} still shows its stand-in", clothing.item);
            assert!(icons.large(&ctx, clothing.item).is_some_and(|t| t.size() == [192, 192]));
        }
    }

    #[test]
    fn generated_folder_links_keep_the_shortcut_outline_as_a_gap() {
        // the same corner as the hand-made links (see the test above)
        for row in category::ROWS {
            let image = rasterize_mask(row.svgs[2], 240, true).expect("link SVG");
            assert_eq!(image.pixels[230 * 240 + 3].a(), 0, "outline {}", row.link);
            assert_eq!(image.pixels[230 * 240 + 10].a(), 255, "arrow {}", row.link);
        }
    }

    #[test]
    fn a_skins_legacy_folder_images_reach_auroras_system_folders() {
        let dir = std::env::temp_dir().join(format!("aurora-skin-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("skin dir");
        let square = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24"/></svg>"#;
        for name in ["Inv_LookFolderClosed", "Inv_StockFolderOpen", "Inv_SysClosed"] {
            std::fs::write(dir.join(format!("{name}.svg")), square).expect("skin icon");
        }
        let ctx = egui::Context::default();
        let skinned = Icons::load(&ctx, Some(&dir));
        let plain = Icons::load(&ctx, None);
        let _ = std::fs::remove_dir_all(&dir);
        let id = |icons: &Icons, name: &str| icons.get(name).map(|t| t.id());
        // Current outfit takes the skin's look folder, Marketplace listings its
        // system folder, the stock folder its own image
        assert_eq!(id(&skinned, "Inv_CurrentOutfitClosed"), id(&skinned, "Inv_LookFolderClosed"));
        assert_eq!(id(&skinned, "Inv_MarketplaceClosed"), id(&skinned, "Inv_SysClosed"));
        assert!(id(&skinned, "Inv_StockFolderOpen").is_some());
        // what the skin does not draw keeps Aurora's own icon
        assert_ne!(id(&skinned, "Inv_CurrentOutfitOpen"), id(&skinned, "Inv_LookFolderClosed"));
        assert_ne!(id(&skinned, "Inv_VersionFolderClosed"), id(&skinned, "Inv_SysClosed"));
        // with no skin, the stock and version folders are the Marketplace's
        for name in [
            "Inv_StockFolderClosed",
            "Inv_StockFolderOpen",
            "Inv_VersionFolderClosed",
            "Inv_VersionFolderOpen",
        ] {
            assert!(plain.get(name).is_some() && embedded(name), "{name}");
        }
        assert_eq!(inventory_link_name("Inv_StockFolderClosed"), "Inv_MarketplaceClosed_Link");
    }

    #[test]
    fn generated_folder_icons_load_with_a_hole_and_a_large_copy() {
        let ctx = egui::Context::default();
        let icons = Icons::load(&ctx, None);
        for row in category::ROWS {
            for name in [row.closed, row.open, row.link] {
                assert!(icons.get(name).is_some(), "missing {name}");
                assert!(icons.large(&ctx, name).is_some_and(|t| t.size() == [192, 192]), "no large {name}");
            }
            // the symbol is cut out of the folder: the middle of the
            // folder's body is see-through somewhere, unlike a plain folder
            let (svg, _) = generated_folder(row.closed).expect("generated");
            let image = rasterize_svg(svg, 96).expect("folder SVG");
            let plain = rasterize_svg(include_bytes!("../../assets/icons/tabler/filled/folder.svg"), 96).expect("folder");
            let holes = |ci: &egui::ColorImage| {
                (36..68)
                    .flat_map(|y| (30..66).map(move |x| (x, y)))
                    .filter(|(x, y)| ci.pixels[y * 96 + x].a() < 128)
                    .count()
            };
            assert_eq!(holes(&plain), 0);
            assert!(holes(&image) > 20, "{:?}: no symbol cut out", row.category);
        }
        // drawn small, the usual raster; past its pixels, the large one
        assert_eq!(icons.sized(&ctx, "Inv_TexturesClosed", 16.0).map(|t| t.size()), Some([64, 64]));
        assert_eq!(icons.sized(&ctx, "Inv_TexturesClosed", 90.0).map(|t| t.size()), Some([64, 64]));
        assert_eq!(icons.sized(&ctx, "Inv_TexturesClosed", 100.0).map(|t| t.size()), Some([192, 192]));
        // aliases share their target's SVG
        assert_eq!(icons.large(&ctx, "Inv_Texture").map(|t| t.size()), Some([192, 192]));
        // icons with no SVG of their own fall back on the usual raster
        assert!(icons.large(&ctx, "phoenix_18").is_some());
        assert!(icons.large(&ctx, "no-such-icon").is_none());
    }

    #[test]
    fn wearable_types_have_distinct_embedded_icons() {
        let mut targets = std::collections::HashSet::new();
        for kind in 0..=16 {
            let asset_type = if kind < 4 { 13 } else { 5 };
            let name = crate::ui::inventory::item_icon(asset_type, 18, kind);
            let target = alias_target(name).expect("wearable icon alias");
            assert!(targets.insert(target), "wearable type {kind} repeats {target}");
            assert!(OUTLINE.iter().chain(FILLED).any(|(name, _)| *name == target));
            assert_eq!(crate::ui::inventory::item_icon(asset_type, 18, kind | 0xffff_ff00), name);
        }
        assert_eq!(crate::ui::inventory::item_icon(5, 18, 255), "Inv_Clothing");
        assert_eq!(crate::ui::inventory::item_icon(13, 18, 255), "Inv_BodyShape");
        assert_eq!(crate::ui::inventory::item_icon(24, 24, 14), "Inv_LinkItem");
        assert_eq!(crate::ui::inventory::item_icon(10, 10, 0), "Inv_Script");
        assert_eq!(crate::ui::inventory::item_icon(10, 10, 1), "Inv_Script_Luau");
        assert_eq!(inventory_link_name("Inv_Script_Luau"), "Inv_Script_Luau_Link");
    }
}
