//! UI icons: Tabler SVGs (MIT, see assets/icons/tabler/LICENSE), rasterized
//! at startup as white alpha masks so they can be tinted with the skin
//! palette. Names follow Tabler: `eye-off` is the outline icon, `eye-filled`
//! the filled one. Legacy Firestorm icon names are mapped onto Tabler ones;
//! skins can override any icon with a PNG or SVG of the same name.

use std::collections::HashMap;

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
    "chevron-left",
    "chevron-right",
    "chevrons-down",
    "chevrons-right",
    "chevrons-up",
    "circle-minus",
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
    "external-link",
    "eye-closed",
    "eye-off",
    "file-export",
    "file-import",
    "focus-2",
    "focus-centered",
    "fingerprint",
    "gavel",
    "gif",
    "grid-3x3",
    "grip-vertical",
    "hammer-off",
    "hand-grab",
    "hand-move",
    "hand-click",
    "hand-finger",
    "hand-stop",
    "heart-handshake",
    "heart-rate-monitor",
    "history",
    "home-check",
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
    "license",
    "link",
    "loader-2",
    "lock-open",
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
    "radio",
    "refresh",
    "refresh-off",
    "reload",
    "repeat",
    "restore",
    "rotate-2",
    "rotate-360",
    "rotate-clockwise",
    "rotate-clockwise-2",
    "route",
    "rss",
    "run",
    "select-all",
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

    "swimming",
    "terminal-2",
    "text-size",
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
    "caret-down",
    "caret-left",
    "caret-right",
    "caret-up",
    "chart-area-line",
    "chart-candle",
    "chart-dots",
    "circle",
    "chart-area-line",
    "circle-check",
    "circle-plus",
    "circle-x",
    "click",
    "clipboard-text",
    "clock",
    "cloud",
    "cloud-data-connection",
    "code-circle",
    "coin",
    "compass",
    "confetti",
    "copy",
    "crown",
    "current-location",
    "dashboard",
    "database",
    "device-desktop",
    "device-floppy",
    "device-gamepad",
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
    "gift",
    "grid-pattern",
    "hanger-2",
    "headphones",
    "headset",
    "heart",
    "help-circle",
    "home",
    "hourglass",
    "id",
    "info-circle",
    "key",
    "keyboard",
    "label",
    "layout",
    "layout-align-bottom",
    "layout-align-center",
    "layout-align-left",
    "layout-align-middle",
    "layout-align-right",
    "layout-align-top",
    "layout-bottombar",
    "layout-dashboard",
    "layout-sidebar-right",
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
    "mountain",
    "mouse",
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
    "player-stop",
    "playlist",
    "point",
    "pointer",
    "quote",
    "radar",
    "receipt",
    "replace",
    "rosette-discount-check",
    "search",
    "send",
    "settings",
    "shield",
    "shield-lock",
    "shirt",
    "shopping-cart",
    "sort-ascending-2",
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
    "square-rounded-chevron-right",
    "square-rounded-chevrons-down",
    "square-rounded-chevrons-up",
    "stack",
    "star",
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

/// Firestorm icon names used by skins / layouts -> Tabler icon.
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
    ("Command_HowTo_Icon", "help-circle-filled"),
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
    ("Inv_Object", "box"),
    ("Inv_Object_Multi", "packages"),
    ("Inv_Texture", "photo-filled"),
    ("Inv_Notecard", "file-text-filled"),
    ("Inv_Landmark", "map-pin-filled"),
    ("Inv_Clothing", "shirt-filled"),
    ("Inv_BodyShape", "man-filled"),
    ("Inv_Skin", "fingerprint"),
    ("Inv_Hair", "scissors"),
    ("Inv_Eye", "eye-filled"),
    ("Inv_Shirt", "shirt-filled"),
    ("Inv_Pants", "walk"),
    ("Inv_Shoe", "shoe"),
    ("Inv_Socks", "sock"),
    ("Inv_Jacket", "jacket"),
    ("Inv_Gloves", "hand-stop"),
    ("Inv_Undershirt", "shirt-sport"),
    ("Inv_Underpants", "diaper"),
    ("Inv_Skirt", "woman-filled"),
    ("Inv_Alpha", "layers-subtract"),
    ("Inv_Tattoo", "needle"),
    ("Inv_Physics", "wave-sine"),
    ("Inv_Universal", "layers-union"),
    ("Inv_Script", "code"),
    ("Inv_Sound", "file-music-filled"),
    ("Inv_Animation", "run"),
    ("Inv_Gesture", "hand-move"),
    ("Inv_Snapshot", "camera-filled"),
    ("Inv_Mesh", "cube-3d-sphere"),
    ("Inv_Material", "droplet-filled"),
    ("Inv_Settings", "sun-filled"),
    ("Inv_CallingCard", "id-filled"),
    ("Inv_LinkItem", "link-filled"),
    ("Inv_LinkFolder", "link-filled"),
    ("Inv_TrashClosed", "trash-filled"),
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
        include_bytes!("../../../../assets/tabler-icons/outline/walk-link.svg"),
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
        include_bytes!("../../../../assets/tabler-icons/outline/shirt-sport-link.svg"),
    ),
    (
        "Inv_Underpants_Link",
        include_bytes!("../../../../assets/tabler-icons/outline/diaper-link.svg"),
    ),
    (
        "Inv_Skirt_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/woman-link.svg"),
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
        include_bytes!("../../../../assets/tabler-icons/filled/droplet-link.svg"),
    ),
    (
        "Inv_Settings_Link",
        include_bytes!("../../../../assets/tabler-icons/filled/sun-link.svg"),
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
        "Inv_Sound" => "Inv_Sound_Link",
        "Inv_Animation" => "Inv_Animation_Link",
        "Inv_Gesture" => "Inv_Gesture_Link",
        "Inv_Snapshot" => "Inv_Snapshot_Link",
        "Inv_Mesh" => "Inv_Mesh_Link",
        "Inv_Material" => "Inv_Material_Link",
        "Inv_Settings" => "Inv_Settings_Link",
        "Inv_CallingCard" => "Inv_CallingCard_Link",
        "Inv_TrashClosed" => "Inv_TrashClosed_Link",
        "Inv_SysClosed" => "Inv_SysClosed_Link",
        "Inv_Invalid" => "Inv_Invalid_Link",
        _ => "Inv_LinkItem",
    }
}

/// Whether `name` is an embedded icon or one of the skin's aliases.
pub(crate) fn embedded(name: &str) -> bool {
    OUTLINE.iter().chain(FILLED).chain(INVENTORY_LINKS).any(|(n, _)| *n == name) || ALIASES.iter().any(|(alias, _)| *alias == name)
}

/// Raster size of SVG icons (crisp up to ~2x UI scale at 24 px).
const RASTER_PX: u32 = 64;

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

#[derive(Default)]
pub struct Icons {
    map: HashMap<String, egui::TextureHandle>,
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
        for (name, bytes) in OUTLINE.iter().chain(FILLED) {
            match rasterize_svg(bytes, RASTER_PX) {
                Some(ci) => {
                    map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
                }
                None => log::warn!("icon {name}: invalid SVG"),
            }
        }
        for (name, bytes) in INVENTORY_LINKS {
            if let Some(ci) = rasterize_mask(bytes, RASTER_PX, true) {
                map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
            } else {
                log::warn!("icon {name}: invalid shortcut SVG");
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
        for (alias, target) in ALIASES {
            if let Some(t) = map.get(*target).cloned() {
                map.insert((*alias).to_owned(), t);
            }
        }
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
                    map.insert(stem.to_owned(), ctx.load_texture(stem, ci, opts));
                }
            }
        }
        if let Ok(mut g) = GLOBAL.write() {
            *g = Some(map.clone());
        }
        Icons { map }
    }

    pub fn get(&self, name: &str) -> Option<&egui::TextureHandle> {
        self.map.get(name)
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
    fn aliases_point_to_embedded_icons() {
        let names: Vec<&str> = OUTLINE.iter().chain(FILLED).map(|(name, _)| *name).collect();
        for (alias, target) in ALIASES {
            assert!(names.contains(target), "{alias} -> {target} is not embedded");
        }
    }

    #[test]
    fn wearable_types_have_distinct_embedded_icons() {
        let mut targets = std::collections::HashSet::new();
        for kind in 0..=16 {
            let asset_type = if kind < 4 { 13 } else { 5 };
            let name = crate::ui::inventory::item_icon(asset_type, 18, kind);
            let target = ALIASES.iter().find(|(alias, _)| *alias == name).expect("wearable icon alias").1;
            assert!(targets.insert(target), "wearable type {kind} repeats {target}");
            assert!(OUTLINE.iter().chain(FILLED).any(|(name, _)| *name == target));
            assert_eq!(crate::ui::inventory::item_icon(asset_type, 18, kind | 0xffff_ff00), name);
        }
        assert_eq!(crate::ui::inventory::item_icon(5, 18, 255), "Inv_Clothing");
        assert_eq!(crate::ui::inventory::item_icon(13, 18, 255), "Inv_BodyShape");
        assert_eq!(crate::ui::inventory::item_icon(24, 24, 14), "Inv_LinkItem");
    }
}
