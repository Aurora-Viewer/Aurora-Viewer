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
    "door-enter",
    "door-exit",
    "external-link",
    "eye-closed",
    "eye-off",
    "file-export",
    "file-import",
    "focus-2",
    "focus-centered",
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
    "label-off",
    "language",
    "language-off",
    "layers-intersect",
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
    "maximize",
    "message-circle-plus",
    "message-off",
    "microphone-off",
    "minimize",
    "movie",
    "music",
    "network",
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
    "shape",
    "share",
    "shield-off",
    "shovel",
    "slash",
    "slashes",
    "snowflake",
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
    "man",
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

/// Whether `name` is an embedded icon or one of the skin's aliases.
pub(crate) fn embedded(name: &str) -> bool {
    OUTLINE.iter().chain(FILLED).any(|(n, _)| *n == name) || ALIASES.iter().any(|(alias, _)| *alias == name)
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
        .flat_map(|p| [255, 255, 255, p[3]])
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
    fn aliases_point_to_embedded_icons() {
        let names: Vec<&str> = OUTLINE.iter().chain(FILLED).map(|(name, _)| *name).collect();
        for (alias, target) in ALIASES {
            assert!(names.contains(target), "{alias} -> {target} is not embedded");
        }
    }
}
