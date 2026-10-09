//! UI icons: Phosphor SVGs (MIT, see assets/icons/phosphor/LICENSE),
//! rasterized at startup as white alpha masks so they can be tinted with
//! the skin palette. Legacy Firestorm icon names are mapped onto Phosphor
//! ones; skins can override any icon with a PNG or SVG of the same name.

use std::collections::HashMap;

macro_rules! svg_list {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../assets/icons/phosphor/", $name, ".svg")) as &[u8])),*]
    };
}

/// Embedded Phosphor icons (regular weight), by Phosphor name.
const SVGS: &[(&str, &[u8])] = svg_list!(
    "chat-circle-dots",
    "terminal-window",
    "microphone",
    "speaker-high",
    "arrows-out-cardinal",
    "eye",
    "users",
    "t-shirt",
    "magnifying-glass",
    "map-trifold",
    "camera",
    "backpack",
    "compass",
    "gauge",
    "gear-six",
    "cube",
    "hand-waving",
    "question",
    "sun-horizon",
    "map-pin",
    "armchair",
    "crosshair",
    "target",
    "chart-bar",
    "chats-circle",
    "house",
    "stop-circle",
    "folder",
    "folder-open",
    "image",
    "note",
    "person",
    "code",
    "speaker-simple-high",
    "person-simple-run",
    "cube-transparent",
    "drop",
    "sliders",
    "identification-card",
    "link",
    "folder-simple-star",
    "trash",
    "user-circle",
    "user-plus",
    "airplane-tilt",
    "gift",
    "tag",
    "phone",
    "clock-counter-clockwise",
    "list",
    "smiley",
    "caret-down",
    "x",
    "arrow-left",
    "arrow-right",
    "info",
    "airplane-takeoff",
    "currency-circle-dollar",
    "paper-plane-tilt",
    "users-three",
    "person-simple-walk",
    "minus",
    "corners-out",
    "corners-in",
    "arrows-out",
    "star",
    "address-book",
    "chat-text",
    "radio",
    "monitor-play",
    "speaker-x",
    "speaker-slash",
    "speaker-none",
    "speaker-low",
    "play",
    "pause",
    "eye-slash",
    "music-notes",
    "bell",
    "bell-ringing",
    "warning",
    "handshake",
    "scroll",
    "key",
    "globe",
    "check",
    "keyboard",
    "microphone-slash",
    "sun",
    "speedometer",
    "layout",
    "palette",
    "text-aa",
    "text-t",
    "monitor",
    "at",
    "mouse",
    "hard-drives",
    "speaker-hifi",
    "waveform",
    "squares-four",
    "github-logo",
    "discord-logo",
    "globe-simple",
    "user",
    "lock-key",
    "eye",
    "download-simple",
    "newspaper",
    "arrow-square-out",
    "power",
    "sign-in",
    "check-circle",
    "sparkle",
    "arrows-clockwise",
    "plus",
    "copy",
    "calendar-dots",
    "broadcast",
    "prohibit",
    "sign-out",
    "chat-teardrop-slash",
    "microphone-fill",
    "airplane-tilt-fill",
    "hand-palm-fill",
    "cube-fill",
    "code-fill",
    "heart-fill",
    "eye-fill",
    "eye-slash-fill",
    "arrow-square-in",
    "plus",
    "push-pin",
);

/// Brand SVGs (white wolf silhouette from Branding/), drawn untinted or tinted.
const BRAND_SVGS: &[(&str, &[u8])] = &[("wolf", include_bytes!("../../assets/icons/wolf.svg"))];

/// Raster images kept as PNG (brand logo).
const PNGS: &[(&str, &[u8])] = &[("phoenix_18", include_bytes!("../../assets/icons/phoenix_18.png"))];

/// Firestorm icon names used by skins / layouts -> Phosphor icon.
const ALIASES: &[(&str, &str)] = &[
    ("Command_Chat_Icon", "chat-circle-dots"),
    ("Command_Speak_Icon", "microphone"),
    ("Command_Voice_Icon", "speaker-high"),
    ("Command_Move_Icon", "arrows-out-cardinal"),
    ("Command_View_Icon", "eye"),
    ("Command_People_Icon", "users"),
    ("Command_Appearance_Icon", "t-shirt"),
    ("Command_Search_Icon", "magnifying-glass"),
    ("Command_Map_Icon", "map-trifold"),
    ("Command_Snapshot_Icon", "camera"),
    ("Command_Inventory_Icon", "backpack"),
    ("Command_MiniMap_Icon", "compass"),
    ("Command_Performance_Icon", "gauge"),
    ("Command_Preferences_Icon", "gear-six"),
    ("Command_Build_Icon", "cube"),
    ("Command_Gestures_Icon", "hand-waving"),
    ("Command_HowTo_Icon", "question"),
    ("Command_Environments_Icon", "sun-horizon"),
    ("Command_Places_Icon", "map-pin"),
    ("groundsit", "armchair"),
    ("mouselook", "crosshair"),
    ("radar", "target"),
    ("statistics", "chart-bar"),
    ("nearbychat_18", "chats-circle"),
    ("Home_Off", "house"),
    ("Stop_Off", "stop-circle"),
    ("Inv_FolderClosed", "folder"),
    ("Inv_FolderOpen", "folder-open"),
    ("Inv_Object", "cube"),
    ("Inv_Texture", "image"),
    ("Inv_Notecard", "note"),
    ("Inv_Landmark", "map-pin"),
    ("Inv_Clothing", "t-shirt"),
    ("Inv_BodyShape", "person"),
    ("Inv_Script", "code"),
    ("Inv_Sound", "speaker-simple-high"),
    ("Inv_Animation", "person-simple-run"),
    ("Inv_Gesture", "hand-waving"),
    ("Inv_Snapshot", "camera"),
    ("Inv_Mesh", "cube-transparent"),
    ("Inv_Material", "drop"),
    ("Inv_Settings", "sliders"),
    ("Inv_CallingCard", "identification-card"),
    ("Inv_LinkItem", "link"),
    ("Inv_LinkFolder", "link"),
    ("Inv_TrashClosed", "trash"),
    ("Inv_SysClosed", "folder-simple-star"),
    ("Inv_Invalid", "question"),
];

/// Raster size of SVG icons (crisp up to ~2x UI scale at 24 px).
const RASTER_PX: u32 = 64;

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
        for (name, bytes) in SVGS {
            match rasterize_svg(bytes, RASTER_PX) {
                Some(ci) => {
                    map.insert((*name).to_owned(), ctx.load_texture(*name, ci, opts));
                }
                None => log::warn!("icon {name}: invalid SVG"),
            }
        }
        for (name, bytes) in BRAND_SVGS {
            if let Some(ci) = rasterize_svg(bytes, 128).map(center_on_content) {
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
        for (name, bytes) in SVGS {
            let ci = rasterize_svg(bytes, 32).unwrap_or_else(|| panic!("{name} failed"));
            assert!(ci.pixels.iter().any(|p| p.a() > 0), "{name} is empty");
        }
    }
}
