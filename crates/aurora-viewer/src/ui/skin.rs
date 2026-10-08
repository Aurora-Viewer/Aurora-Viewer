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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
                tb("Command_HowTo_Icon", "help", "Aide"),
                tb("Command_Chat_Icon", "conversations", "Conversations"),
                tb("Command_Speak_Icon", "speak", "Parler"),
                tb("Command_Move_Icon", "fly", "Voler / atterrir (F)"),
                tb("Command_View_Icon", "mouselook", "Vue souris (M)"),
                tb("Command_People_Icon", "people", "Personnes"),
                tb("Command_Inventory_Icon", "inventory", "Inventaire"),
                tb("Command_Appearance_Icon", "appearance", "Apparence (à venir)"),
                tb("Command_Search_Icon", "search", "Recherche (à venir)"),
                tb("Command_Map_Icon", "worldmap", "Carte du monde"),
                tb("Command_MiniMap_Icon", "minimap", "Mini-carte"),
                tb("Command_Performance_Icon", "performance", "Performances"),
                tb("groundsit", "sit", "S'asseoir par terre / se lever"),
                tb("Home_Off", "home", "Rentrer chez moi"),
                tb("Command_Preferences_Icon", "preferences", "Préférences"),
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
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            log::warn!("{}: {e} (using defaults)", path.display());
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
                d.join("LISEZ-MOI.txt"),
                "Skin Aurora Viewer\n\n\
                 theme.json  : couleurs (#RRGGBB), arrondis, taille des polices\n\
                 layout.json : boutons de la barre d'outils (icone + commande), options du chat\n\
                 (copiez les fichiers *.example.json pour partir des valeurs par defaut)\n\
                 icons/      : PNG qui remplacent les icones integrees (meme nom de fichier)\n\n\
                 Les modifications sont rechargees automatiquement en jeu.\n\
                 Commandes : help, conversations, speak, fly, mouselook, people, friends, inventory,\n\
                 appearance, search, worldmap, minimap, performance, sit, home, preferences.\n",
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
