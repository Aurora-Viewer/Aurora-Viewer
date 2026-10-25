//! Back up the settings to one file and restore them (#683), after the
//! Legacy Viewer's Backup tab (FSPanelPreferenceBackup::doBackupSettings and
//! doRestoreSettings, llfloaterpreference.cpp; panel_preferences_backup.xml).
//!
//! A backup holds the preferences (`settings.json`, with the colours, the
//! shortcuts and the bars), the windows' layout (`ui_layout.ron`), the
//! skin's `theme.json` and `layout.json`, and the preference files of each
//! account (the Legacy Viewer's per-account list, the ones Aurora has).
//! Deliberate differences: one file instead of a folder, to carry to another
//! PC as it is; a restore applies at once instead of quitting the viewer;
//! nothing of the login goes in it (the Legacy Viewer copies bin_conf.dat,
//! the saved credentials), nor what belongs to this PC (window, folders,
//! audio devices, machine id): a restore keeps this PC's values of those.

use super::Settings;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

/// What the file says it is: any other JSON file is refused.
const FORMAT: &str = "aurora-settings-backup";
const VERSION: u32 = 1;

/// Settings of the login and of this PC, left out of a backup and kept by
/// a restore (`keep_this_pc`).
const THIS_PC: &[&str] = &[
    "username",
    "remember_username",
    "remember_password",
    "grid",
    "custom_login_uri",
    "start_location",
    "start_region",
    "window_width",
    "window_height",
    "window_x",
    "window_y",
    "window_maximized",
    "cache_location",
    "chat_log_location",
];
/// The same inside `audio`: the devices of this PC.
const THIS_PC_AUDIO: &[&str] = &["output_device", "input_device"];

/// An account's files that are preferences. Only these names are read from
/// a backup: a name in the file is never taken as a path.
const ACCOUNT_FILES: &[&str] = &[
    crate::world::contact_sets::CONTACT_SETS_FILE,
    crate::world::groups::FAVORITE_GROUPS_FILE,
    crate::world::calls::VOLUMES_FILE,
    crate::world::derender::FILE,
    crate::media::filter::FILE,
    crate::world::favorites::FILE,
    crate::world::hud_scale::FILE,
    crate::world::hud_lock::FILE,
    crate::world::hover_height::FILE,
    crate::world::tphistory::TELEPORT_HISTORY_FILE,
];

/// The skin whose files are kept (`ui/skin.rs`), and those files; its icons
/// are pictures and stay out.
const SKIN: &str = "aurora";
const SKIN_FILES: &[&str] = &["theme.json", "layout.json"];

/// A backup file.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Backup {
    format: String,
    version: u32,
    /// The viewer that wrote it, and when (local time).
    viewer: String,
    created: String,
    settings: Option<Value>,
    /// egui's memory as `ui_layout.ron` holds it.
    layout: Option<String>,
    skin: BTreeMap<String, String>,
    accounts: BTreeMap<Uuid, BTreeMap<String, String>>,
}

/// The parts of a backup, those it holds or those a restore puts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub settings: bool,
    pub layout: bool,
    pub skin: bool,
    pub accounts: bool,
}

impl Backup {
    /// What is taken on the frame: the settings as they are saved, and the
    /// windows' layout. The files follow off the frame (`read_files`).
    pub fn new(settings: &Settings, layout: Option<String>) -> Backup {
        let mut value = serde_json::to_value(settings.to_saved()).ok();
        if let Some(Value::Object(map)) = &mut value {
            for key in THIS_PC {
                map.remove(*key);
            }
            if let Some(Value::Object(audio)) = map.get_mut("audio") {
                for key in THIS_PC_AUDIO {
                    audio.remove(*key);
                }
            }
        }
        Backup {
            format: FORMAT.into(),
            version: VERSION,
            viewer: aurora_version::version().to_string(),
            created: crate::snapshot::local_stamp(),
            settings: value,
            layout,
            ..Backup::default()
        }
    }

    /// Adds the skin's and the accounts' files of the profile folder
    /// `config` (`settings::config_dir`).
    pub fn read_files(&mut self, config: &Path) {
        let skin = config.join("skins").join(SKIN);
        for name in SKIN_FILES {
            if let Some(text) = read_text(&skin.join(name)) {
                self.skin.insert((*name).to_owned(), text);
            }
        }
        let Ok(accounts) = std::fs::read_dir(config.join("accounts")) else {
            return;
        };
        for entry in accounts.flatten() {
            let Some(id) = entry.file_name().to_str().and_then(|name| Uuid::parse_str(name).ok()) else {
                continue;
            };
            let files: BTreeMap<String, String> = ACCOUNT_FILES
                .iter()
                .filter_map(|name| Some(((*name).to_owned(), read_text(&entry.path().join(name))?)))
                .collect();
            if !files.is_empty() {
                self.accounts.insert(id, files);
            }
        }
    }

    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
    }

    /// A backup file read back; `None` (logged) when it is not one.
    pub fn parse(bytes: &[u8]) -> Option<Backup> {
        let backup = match serde_json::from_slice::<Backup>(bytes) {
            Ok(backup) => backup,
            Err(e) => {
                log::warn!("settings backup: not readable ({e})");
                return None;
            }
        };
        if backup.format != FORMAT {
            log::warn!("settings backup: not a backup of Aurora's settings");
            return None;
        }
        if backup.version > VERSION {
            // read what this build knows of it, as the settings themselves
            log::info!(
                "settings backup: written by a newer Aurora ({}), read as version {VERSION}",
                backup.viewer
            );
        }
        Some(backup)
    }

    /// The parts the file holds.
    pub fn parts(&self) -> Parts {
        Parts {
            settings: self.settings.is_some(),
            layout: self.layout.is_some(),
            skin: !self.skin.is_empty(),
            accounts: !self.accounts.is_empty(),
        }
    }

    /// The version of the viewer that wrote it.
    pub fn viewer(&self) -> &str {
        &self.viewer
    }

    pub fn accounts(&self) -> impl Iterator<Item = &Uuid> {
        self.accounts.keys()
    }

    /// When it was written, `YYYY-MM-DD HH:MM`.
    pub fn created(&self) -> String {
        let c = &self.created;
        match (c.get(..10), c.get(11..13), c.get(13..15)) {
            (Some(date), Some(h), Some(m)) => format!("{date} {h}:{m}"),
            _ => c.clone(),
        }
    }

    /// The backup's settings, migrated and bounded as a settings file, with
    /// this PC's login, window, folders and devices; `None` when they are
    /// not readable.
    pub fn settings(&self, current: &Settings) -> Option<Settings> {
        let read = crate::tolerant::from_value::<Settings>(self.settings.clone()?, "settings backup")?;
        let mut restored = read.value.sanitized();
        restored.keep_this_pc(current);
        Some(restored)
    }

    pub fn layout(&self) -> Option<&str> {
        self.layout.as_deref()
    }

    /// Writes the skin's files into the profile folder `config`; the skin
    /// reloads them on its own (`Skin::maybe_reload`).
    pub fn write_skin(&self, config: &Path) -> std::io::Result<()> {
        let dir = config.join("skins").join(SKIN);
        std::fs::create_dir_all(&dir)?;
        for (name, text) in known(&self.skin, SKIN_FILES) {
            crate::atomic_file::write(&dir.join(name), text.as_bytes())?;
        }
        Ok(())
    }

    /// Writes the accounts' files into the profile folder `config`, but for
    /// the account logged in (`skip`), whose files the session holds and
    /// saves over; how many accounts were written.
    pub fn write_accounts(&self, config: &Path, skip: Option<Uuid>) -> std::io::Result<usize> {
        let mut written = 0;
        for (id, files) in self.accounts.iter().filter(|(id, _)| Some(**id) != skip) {
            let dir = config.join("accounts").join(id.to_string());
            std::fs::create_dir_all(&dir)?;
            for (name, text) in known(files, ACCOUNT_FILES) {
                crate::atomic_file::write(&dir.join(name), text.as_bytes())?;
            }
            written += 1;
        }
        Ok(written)
    }
}

/// The files of a backup whose name is one of `names`.
fn known<'a>(files: &'a BTreeMap<String, String>, names: &'a [&str]) -> impl Iterator<Item = (&'a str, &'a String)> {
    names.iter().filter_map(|name| Some((*name, files.get(*name)?)))
}

/// A text file; nothing when it is missing or not text.
fn read_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(bytes)
        .inspect_err(|_| log::warn!("settings backup: {} is not text, left out", path.display()))
        .ok()
}

impl Settings {
    /// Takes this PC's values of what a backup leaves out (`THIS_PC`), and
    /// gives the account logged in its own height options from the backup.
    fn keep_this_pc(&mut self, current: &Settings) {
        self.username = current.username.clone();
        self.remember_username = current.remember_username;
        self.remember_password = current.remember_password;
        self.grid = current.grid.clone();
        self.custom_login_uri = current.custom_login_uri.clone();
        self.start_location = current.start_location.clone();
        self.start_region = current.start_region.clone();
        self.window_width = current.window_width;
        self.window_height = current.window_height;
        self.window_x = current.window_x;
        self.window_y = current.window_y;
        self.window_maximized = current.window_maximized;
        self.cache_location = current.cache_location.clone();
        self.chat_log_location = current.chat_log_location.clone();
        self.audio.output_device = current.audio.output_device.clone();
        self.audio.input_device = current.audio.input_device.clone();
        self.mfa_hash = current.mfa_hash.clone();
        self.debug = current.debug;
        // a newer build's own values are not the backup's to drop
        self.newer_file = current.newer_file.clone();
        if let Some(account) = current.hover_height_account {
            self.select_hover_account(account);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-settings-backup-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Settings whose login, window, folders and devices are this PC's.
    fn this_pc() -> Settings {
        let mut s = Settings {
            username: "someone".into(),
            remember_username: true,
            remember_password: true,
            grid: crate::settings::GridChoice::Custom,
            custom_login_uri: "http://grid.test/".into(),
            start_location: "last".into(),
            start_region: "Somewhere".into(),
            window_width: 1234,
            window_height: 777,
            window_x: Some(12),
            window_y: Some(34),
            window_maximized: true,
            cache_location: "D:\\cache".into(),
            chat_log_location: "D:\\logs".into(),
            ..Settings::default()
        };
        s.audio.output_device = "Speakers".into();
        s.audio.input_device = "Microphone".into();
        s
    }

    #[test]
    fn a_backup_holds_nothing_of_the_login_nor_of_this_pc() {
        let backup = Backup::new(&this_pc(), None);
        let text = String::from_utf8(backup.to_bytes().unwrap()).unwrap();
        for value in [
            "someone",
            "grid.test",
            "Somewhere",
            "D:\\\\cache",
            "D:\\\\logs",
            "Speakers",
            "Microphone",
        ] {
            assert!(!text.contains(value), "{value} written in the backup");
        }
        let settings = backup.settings.as_ref().unwrap();
        for key in THIS_PC {
            assert!(settings.get(*key).is_none(), "{key} written in the backup");
        }
    }

    #[test]
    fn every_key_left_out_is_a_setting() {
        // a renamed setting would quietly go into the backups again
        let saved = serde_json::to_value(this_pc().to_saved()).unwrap();
        for key in THIS_PC {
            assert!(saved.get(*key).is_some(), "{key} is not a setting");
        }
        for key in THIS_PC_AUDIO {
            assert!(saved["audio"].get(*key).is_some(), "audio.{key} is not a setting");
        }
    }

    #[test]
    fn a_restore_takes_the_backup_and_keeps_this_pc() {
        let mut other = Settings::default();
        other.draw_distance = 256.0;
        other.vsync = !other.vsync;
        other.colors.chat_direct = [1, 2, 3, 255];
        other.window_width = 640;
        other.audio.output_device = "Elsewhere".into();
        let backup = Backup::parse(&Backup::new(&other, Some("layout".into())).to_bytes().unwrap()).unwrap();
        let current = this_pc();
        let restored = backup.settings(&current).unwrap();
        assert_eq!(restored.draw_distance, 256.0);
        assert_eq!(restored.vsync, other.vsync);
        assert_eq!(restored.colors.chat_direct, [1, 2, 3, 255]);
        let (restored, current) = (serde_json::to_value(&restored).unwrap(), serde_json::to_value(&current).unwrap());
        for key in THIS_PC {
            assert_eq!(restored[key], current[key], "{key} not kept");
        }
        for key in THIS_PC_AUDIO {
            assert_eq!(restored["audio"][key], current["audio"][key], "audio.{key} not kept");
        }
        assert_eq!(backup.layout(), Some("layout"));
    }

    #[test]
    fn a_restore_gives_the_account_logged_in_its_saved_heights() {
        let account = Uuid::from_u128(7);
        let mut saved = Settings::default();
        saved.select_hover_account(account);
        saved.auto_hover_height = true;
        saved.auto_hover_offset = -0.12;
        let backup = Backup::new(&saved, None);
        let mut current = Settings::default();
        current.select_hover_account(account);
        let restored = backup.settings(&current).unwrap();
        assert_eq!(restored.hover_height_account, Some(account));
        assert!(restored.auto_hover_height);
        assert_eq!(restored.auto_hover_offset, -0.12);
    }

    #[test]
    fn another_json_file_is_not_a_backup() {
        assert!(Backup::parse(b"{\"draw_distance\": 128}").is_none());
        assert!(Backup::parse(b"not json").is_none());
        assert!(Backup::parse(&serde_json::to_vec(&Settings::default()).unwrap()).is_none());
    }

    #[test]
    fn a_newer_backup_is_read_for_what_this_build_knows() {
        let mut value = serde_json::to_value(Backup::new(&Settings::default(), None)).unwrap();
        value["version"] = 99.into();
        value["a_newer_part"] = "anything".into();
        value["settings"]["a_newer_setting"] = 1.into();
        let backup = Backup::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(backup.settings(&Settings::default()).is_some());
    }

    #[test]
    fn files_go_back_where_they_came_from_and_nowhere_else() {
        let (from, to) = (scratch(), scratch());
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        std::fs::create_dir_all(from.join("skins/aurora")).unwrap();
        std::fs::write(from.join("skins/aurora/theme.json"), "{\"violet\":\"#123456\"}").unwrap();
        for id in [first, second] {
            let dir = from.join("accounts").join(id.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(crate::world::contact_sets::CONTACT_SETS_FILE), format!("sets of {id}")).unwrap();
            // a transcript is not a preference
            std::fs::write(dir.join("notification_history.json"), "[]").unwrap();
        }
        // a folder that is not an account
        std::fs::create_dir_all(from.join("accounts/not-an-account")).unwrap();
        std::fs::write(from.join("accounts/not-an-account/hud_scales.json"), "{}").unwrap();
        let mut backup = Backup::new(&Settings::default(), None);
        backup.read_files(&from);
        assert_eq!(backup.accounts().count(), 2);
        let mut backup = Backup::parse(&backup.to_bytes().unwrap()).unwrap();
        // names in the file are never paths
        backup.skin.insert("../escape.json".into(), "x".into());
        if let Some(files) = backup.accounts.get_mut(&first) {
            files.insert("..\\..\\escape.txt".into(), "x".into());
        }
        backup.write_skin(&to).unwrap();
        // the account logged in keeps its own files
        assert_eq!(backup.write_accounts(&to, Some(second)).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(to.join("skins/aurora/theme.json")).unwrap(),
            "{\"violet\":\"#123456\"}"
        );
        let first_dir = to.join("accounts").join(first.to_string());
        assert_eq!(
            std::fs::read_to_string(first_dir.join(crate::world::contact_sets::CONTACT_SETS_FILE)).unwrap(),
            format!("sets of {first}")
        );
        assert!(!first_dir.join("notification_history.json").exists());
        assert!(!to.join("accounts").join(second.to_string()).exists());
        assert!(!to.join("skins/escape.json").exists() && !to.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(from);
        let _ = std::fs::remove_dir_all(to);
    }

    #[test]
    fn the_creation_time_reads_as_a_date_and_an_hour() {
        let backup = Backup {
            created: "2026-10-22_143005".into(),
            ..Backup::default()
        };
        assert_eq!(backup.created(), "2026-10-22 14:30");
    }
}
