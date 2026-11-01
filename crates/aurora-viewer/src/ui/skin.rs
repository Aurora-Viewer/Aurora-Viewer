//! Skins: a folder `skins/<name>/` containing
//! - `theme.json`  colors, radii, font sizes (see `theme.rs`)
//! - `layout.json` toolbar buttons, chat options
//! - `icons/*.png` icon overrides (same file names as the built-in icons,
//!   plus any new icon referenced from layout.json)
//!
//! Skins are loaded once into memory and hot-reloaded when a file changes
//! (checked once per second), so they cost nothing per frame.

use super::icons::Icons;
use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolButton {
    /// Icon file name without extension.
    pub icon: String,
    /// Command identifier (see `Command`).
    pub command: String,
    pub tooltip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub toolbar: Vec<ToolButton>,
    /// Fraction of the bottom bar used by the chat input.
    pub chat_bar_fraction: f32,
    pub chat_toasts: bool,
    pub chat_toast_seconds: f32,
    pub name_tag_distance: f32,
    pub top_bar_height: f32,
    pub nav_bar_height: f32,
    pub bottom_bar_height: f32,
    pub button_width: f32,
}

fn tb(icon: &str, command: &str, tooltip: &str) -> ToolButton {
    ToolButton {
        icon: icon.into(),
        command: command.into(),
        tooltip: tooltip.into(),
    }
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            toolbar: vec![
                tb("Command_Chat_Icon", "conversations", aurora_i18n::tr!("common-conversations")),
                tb("Command_Speak_Icon", "speak", aurora_i18n::tr!("ui-skin-speak")),
                tb("Command_Move_Icon", "move", aurora_i18n::tr!("controls-movement-title")),
                tb("video-filled", "view", aurora_i18n::tr!("controls-camera-title")),
                #[cfg(target_os = "linux")]
                tb("device-vision-pro-filled", "vr_view", aurora_i18n::tr!("vr-view")),
                tb("Command_View_Icon", "mouselook", aurora_i18n::tr!("common-mouselook-m")),
                tb("Command_People_Icon", "people", aurora_i18n::tr!("common-people")),
                tb("Command_Inventory_Icon", "inventory", aurora_i18n::tr!("common-inventory")),
                tb("stack-2-filled", "texture_library", aurora_i18n::tr!("texture-library-title")),
                tb("Command_Gestures_Icon", "gestures", aurora_i18n::tr!("gesture-title")),
                tb("Command_Appearance_Icon", "appearance", aurora_i18n::tr!("common-appearance")),
                tb("Command_Search_Icon", "search", aurora_i18n::tr!("search-title")),
                tb("Command_Places_Icon", "places", aurora_i18n::tr!("common-places")),
                tb("Command_Snapshot_Icon", "snapshot", aurora_i18n::tr!("photo-title")),
                tb("Command_Map_Icon", "worldmap", aurora_i18n::tr!("common-world-map")),
                tb("Command_MiniMap_Icon", "minimap", aurora_i18n::tr!("common-mini-map")),
                tb("Command_Performance_Icon", "performance", aurora_i18n::tr!("common-performance")),
                tb("groundsit", "sit", aurora_i18n::tr!("extra-sit-on-ground-stand-up")),
                tb("lock-filled", "movelock", aurora_i18n::tr!("movelock-title")),
                tb("AO", "ao", aurora_i18n::tr!("ao-title")),
                tb("Command_Preferences_Icon", "preferences", aurora_i18n::tr!("common-preferences")),
            ],
            chat_bar_fraction: 0.22,
            chat_toasts: true,
            chat_toast_seconds: 20.0,
            name_tag_distance: 64.0,
            top_bar_height: 22.0,
            nav_bar_height: 24.0,
            bottom_bar_height: 30.0,
            button_width: 40.0,
        }
    }
}

pub struct Skin {
    pub name: String,
    pub dir: Option<PathBuf>,
    pub theme: Theme,
    pub layout: Layout,
    pub icons: Icons,
    stamp: Option<SystemTime>,
    last_check: Instant,
}

fn skin_dirs(name: &str) -> Vec<PathBuf> {
    let mut v = vec![crate::settings::config_dir().join("skins").join(name)];
    if let Some(exe) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        v.push(exe.join("skins").join(name));
    }
    v.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skins").join(name));
    v
}

/// Read the theme before graphics startup so early crashes use the same skin.
pub fn recovery_theme() -> Theme {
    skin_dirs("aurora")
        .into_iter()
        .find(|dir| dir.is_dir())
        .map(|dir| read_json(&dir.join("theme.json")))
        .unwrap_or_default()
}

/// Latest modification time of any file in the skin (for hot reload).
fn stamp(dir: &Path) -> Option<SystemTime> {
    let mut latest: Option<SystemTime> = None;
    let mut visit = |p: &Path| {
        if let Ok(m) = std::fs::metadata(p).and_then(|m| m.modified()) {
            latest = Some(latest.map_or(m, |l: SystemTime| l.max(m)));
        }
    };
    visit(&dir.join("theme.json"));
    visit(&dir.join("layout.json"));
    if let Ok(rd) = std::fs::read_dir(dir.join("icons")) {
        for e in rd.flatten() {
            visit(&e.path());
        }
    }
    latest
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match std::fs::read(path) {
        // a value this build does not read costs that value only
        Ok(b) => crate::tolerant::from_slice(&b, &name).map(|read| read.value).unwrap_or_else(|| {
            log::warn!("{}: unreadable (using defaults)", path.display());
            T::default()
        }),
        Err(_) => T::default(),
    }
}

impl Skin {
    pub fn load(ctx: &egui::Context, name: &str, font_scale: f32) -> Skin {
        let dir = skin_dirs(name).into_iter().find(|d| d.is_dir());
        // First run: write the default skin as an editable template.
        let dir = dir.or_else(|| {
            let d = crate::settings::config_dir().join("skins").join(name);
            std::fs::create_dir_all(d.join("icons")).ok()?;
            let theme = serde_json::to_vec_pretty(&Theme::default()).ok()?;
            let layout = serde_json::to_vec_pretty(&Layout::default()).ok()?;
            // Templates only: copy to theme.json / layout.json to customise.
            std::fs::write(d.join("theme.example.json"), theme).ok()?;
            std::fs::write(d.join("layout.example.json"), layout).ok()?;
            let _ = std::fs::write(
                d.join(aurora_i18n::tr!("status-readme-txt")),
                aurora_i18n::tr!("status-aurora-viewer-skin-theme-json-colors"),
            );
            Some(d)
        });
        let theme = dir.as_ref().map(|d| read_json::<Theme>(&d.join("theme.json"))).unwrap_or_default();
        let layout = dir
            .as_ref()
            .map(|d| read_json::<Layout>(&d.join("layout.json")))
            .unwrap_or_default();
        theme.apply(ctx, font_scale);
        let icons = Icons::load(ctx, dir.as_deref().map(|d| d.join("icons")).as_deref());
        log::info!(
            "skin '{name}' loaded from {}",
            dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "(intégré)".into())
        );
        Skin {
            name: name.to_owned(),
            stamp: dir.as_deref().and_then(stamp),
            dir,
            theme,
            layout,
            icons,
            last_check: Instant::now(),
        }
    }

    /// Reload when files changed. Returns true if reloaded.
    pub fn maybe_reload(&mut self, ctx: &egui::Context, font_scale: f32) -> bool {
        if self.last_check.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.last_check = Instant::now();
        let Some(dir) = &self.dir else {
            return false;
        };
        let s = stamp(dir);
        if s != self.stamp {
            *self = Skin::load(ctx, &self.name.clone(), font_scale);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_value_this_build_cannot_read_costs_only_that_value() {
        let dir = std::env::temp_dir().join(format!("aurora-skin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("theme.json");
        let file = serde_json::json!({ "violet": "#123456", "panel_alpha": "a newer kind of value", "a_newer_key": 1 });
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let theme: Theme = read_json(&path);
        assert_eq!(theme.violet, "#123456");
        assert_eq!(theme.panel_alpha, Theme::default().panel_alpha);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
