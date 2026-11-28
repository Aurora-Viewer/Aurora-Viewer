//! Account binding for conversation transcript folder and Legacy Viewer
//! sharing options. Keeps each logged-in avatar's chosen folder and its yes
//! to sharing isolated across logins, logouts and restarts while allowing
//! one-time migration from pre-split global settings (the folder and the
//! sharing option of PR #3588).
//!
//! An entry belongs to an account on a grid (`account_key`): the same agent
//! id logs in on the main grid and on the beta grid, and the Legacy Viewer
//! keeps InstantMessageLogPath in its `settings_per_account.xml`, in the
//! `<first_last>[.<grid label>]` folder of LLDir::setLindenUserDir
//! (indra/newview/llstartup.cpp, indra/llfilesystem/lldir.cpp), so a folder
//! picked, or a yes to sharing, on one grid is never the other's. An entry
//! keyed by the agent id alone (this file's first form, never released)
//! cannot tell its grid: it stays in the file, so the uninstall still lists
//! its folder, but no login takes it.

use super::Settings;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pub chat_log_location: String,
    /// The yes to sharing the Legacy Viewer's folder (the former global
    /// option's name, so an account that shared keeps sharing).
    pub log_legacy_layout: bool,
}

impl Preferences {
    fn current(settings: &Settings) -> Self {
        Self {
            chat_log_location: settings.chat_log_location.clone(),
            log_legacy_layout: settings.log_legacy_layout,
        }
    }

    fn apply(self, settings: &mut Settings) {
        settings.chat_log_location = self.chat_log_location;
        settings.log_legacy_layout = self.log_legacy_layout;
    }
}

/// The key of an account's entry in `chat_log_accounts`, `<grid>/<agent id>`:
/// `grid` is the grid of its transcript folders (`world::chat_log::grid_name`,
/// as in `store::Identity`), which never holds a `/`.
pub(crate) fn account_key(grid: &str, agent: Uuid) -> String {
    format!("{grid}/{agent}")
}

impl Settings {
    /// Bind before the account's transcripts open at login, the account
    /// `agent` on `grid` (`world::chat_log::grid_name`). Preserves any
    /// pre-split global folder/sharing settings in `chat_log_legacy_default` so
    /// every upgrading account without its own entry inherits them until it
    /// picks its own folder/layout.
    pub(crate) fn select_chat_log_account(&mut self, grid: &str, agent: Uuid) {
        self.remember_chat_log_account();
        if self.chat_log_accounts.is_none() {
            let former = Preferences::current(self);
            if former != Preferences::default() && self.chat_log_legacy_default.is_none() {
                self.chat_log_legacy_default = Some(former);
            }
            self.chat_log_accounts = Some(Default::default());
        }
        let key = account_key(grid, agent);
        let preferences = self
            .chat_log_accounts
            .as_ref()
            .and_then(|accounts| accounts.get(&key))
            .cloned()
            .or_else(|| self.chat_log_legacy_default.clone())
            .unwrap_or_default();
        self.chat_log_account = Some(key);
        preferences.apply(self);
        self.remember_chat_log_account();
    }

    /// A change of folder of an earlier session of `agent` on `grid` failed
    /// after its logout (`ChatLog::take_past_move_failure`): its entry names
    /// the folder and the sharing its files stayed with again. When that
    /// account is logged in again, its entry is the settings' own fields.
    pub(crate) fn restore_chat_log_account(&mut self, grid: &str, agent: Uuid, location: String, share: bool) {
        let key = account_key(grid, agent);
        let preferences = Preferences {
            chat_log_location: location,
            log_legacy_layout: share,
        };
        if self.chat_log_account.as_deref() == Some(key.as_str()) {
            preferences.apply(self);
        } else {
            self.chat_log_accounts.get_or_insert_with(Default::default).insert(key, preferences);
        }
    }

    /// Release the active account at logout so another account starts clean
    /// without inheriting or overwriting the leaving account's settings.
    pub(crate) fn release_chat_log_account(&mut self) {
        if self.chat_log_account.is_none() {
            return;
        }
        self.remember_chat_log_account();
        self.chat_log_account = None;
        Preferences::default().apply(self);
    }

    pub(super) fn remember_chat_log_account(&mut self) {
        if let Some(key) = self.chat_log_account.clone() {
            let preferences = Preferences::current(self);
            self.chat_log_accounts.get_or_insert_with(Default::default).insert(key, preferences);
        }
    }

    /// Every non-empty custom transcript root across the active session, saved
    /// accounts (sorted by key, grid then account ID) and the inherited legacy
    /// default, deduplicated in that order.
    pub fn chat_log_locations(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let active = std::iter::once(self.chat_log_location.as_str());
        let saved = self
            .chat_log_accounts
            .iter()
            .flat_map(|map| map.values().map(|p| p.chat_log_location.as_str()));
        let legacy_default = self.chat_log_legacy_default.iter().map(|p| p.chat_log_location.as_str());
        for raw in active.chain(saved).chain(legacy_default) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                let path = PathBuf::from(trimmed);
                if seen.insert(path.clone()) {
                    out.push(path);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restart(settings: &Settings) -> Settings {
        serde_json::from_slice(&serde_json::to_vec(&settings.to_saved()).unwrap()).unwrap()
    }

    #[test]
    fn chat_log_accounts_stay_isolated_across_switches_logout_and_restart() {
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut settings = Settings::default();
        settings.select_chat_log_account("agni", first);
        settings.chat_log_location = "D:\\Firestorm".into();
        settings.log_legacy_layout = true;
        let first_prefs = Preferences::current(&settings);

        // Logging out resets active fields to defaults while preserving account 1.
        settings.release_chat_log_account();
        assert_eq!(Preferences::current(&settings), Preferences::default());

        // Account 2 starts clean with default location and without sharing.
        settings.select_chat_log_account("agni", second);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        assert!(!settings.log_legacy_layout);
        settings.chat_log_location = "E:\\OtherLogs".into();
        let second_prefs = Preferences::current(&settings);

        settings.release_chat_log_account();
        let mut reloaded = restart(&settings);
        assert_eq!(reloaded.chat_log_account, None);
        assert_eq!(Preferences::current(&reloaded), Preferences::default());

        reloaded.select_chat_log_account("agni", first);
        assert_eq!(Preferences::current(&reloaded), first_prefs);
        reloaded.select_chat_log_account("agni", second);
        assert_eq!(Preferences::current(&reloaded), second_prefs);
    }

    #[test]
    fn chat_log_accounts_migrate_former_global_settings_for_every_account() {
        let (first, second, third) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "chat_log_location": "D:\\LegacyRoot",
            "log_legacy_layout": true,
        }))
        .unwrap();
        let former = Preferences::current(&settings);
        // an account that shared with PR #3588's option keeps sharing
        assert!(former.log_legacy_layout);

        // Releasing before any login never consumes the pending migration.
        settings.release_chat_log_account();
        assert_eq!(Preferences::current(&settings), former);

        settings = restart(&settings);
        settings.select_chat_log_account("agni", first);
        assert_eq!(Preferences::current(&settings), former);

        settings.release_chat_log_account();
        settings = restart(&settings);
        settings.select_chat_log_account("agni", second);
        assert_eq!(Preferences::current(&settings), former);

        // Changing account 2's settings isolates it without touching account 1 or future alts.
        settings.chat_log_location = "E:\\AltTwo".into();
        let second_prefs = Preferences::current(&settings);

        settings.release_chat_log_account();
        settings = restart(&settings);
        settings.select_chat_log_account("agni", third);
        assert_eq!(Preferences::current(&settings), former);

        settings.select_chat_log_account("agni", first);
        assert_eq!(Preferences::current(&settings), former);
        settings.select_chat_log_account("agni", second);
        assert_eq!(Preferences::current(&settings), second_prefs);
    }

    #[test]
    fn reset_keeping_account_clears_only_active_account_chat_log_prefs() {
        let (first, second, third) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "chat_log_location": "D:\\LegacyDefault",
            "log_legacy_layout": true,
        }))
        .unwrap();
        let legacy_default = Preferences::current(&settings);

        settings.select_chat_log_account("agni", first);
        settings.chat_log_location = "D:\\LogsA".into();
        let first_prefs = Preferences::current(&settings);

        settings.select_chat_log_account("agni", second);
        settings.chat_log_location = "E:\\LogsB".into();

        settings.reset_keeping_account();
        let mut loaded = restart(&settings);
        loaded.select_chat_log_account("agni", second);
        assert_eq!(Preferences::current(&loaded), Preferences::default());
        loaded.select_chat_log_account("agni", first);
        assert_eq!(Preferences::current(&loaded), first_prefs);
        // An alt not yet seen keeps inheriting the pre-split global settings across reset.
        loaded.select_chat_log_account("agni", third);
        assert_eq!(Preferences::current(&loaded), legacy_default);
    }

    #[test]
    fn chat_log_locations_collects_and_deduplicates_across_accounts() {
        let (first, second, third, active) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "chat_log_location": "F:\\LegacyDefault",
            "log_legacy_layout": true,
        }))
        .unwrap();
        // Insert out of UUID key order so deterministic sorting is exercised.
        settings.select_chat_log_account("agni", third);
        settings.chat_log_location = "E:\\Other".into();
        settings.select_chat_log_account("agni", second);
        settings.chat_log_location = "D:\\Shared".into();
        settings.select_chat_log_account("agni", first);
        settings.chat_log_location = "D:\\Shared".into();
        settings.release_chat_log_account();

        assert_eq!(
            settings.chat_log_locations(),
            vec![
                PathBuf::from("D:\\Shared"),
                PathBuf::from("E:\\Other"),
                PathBuf::from("F:\\LegacyDefault"),
            ]
        );

        // Active account's folder is listed first, before saved accounts sorted by ID and inherited default.
        settings.select_chat_log_account("agni", active);
        settings.chat_log_location = "C:\\Active".into();
        assert_eq!(
            settings.chat_log_locations(),
            vec![
                PathBuf::from("C:\\Active"),
                PathBuf::from("D:\\Shared"),
                PathBuf::from("E:\\Other"),
                PathBuf::from("F:\\LegacyDefault"),
            ]
        );
    }

    #[test]
    fn chat_log_accounts_are_kept_per_grid() {
        let me = Uuid::from_u128(1);
        let mut settings = Settings::default();
        // Share answered on the main grid
        settings.select_chat_log_account("agni", me);
        settings.chat_log_location = "D:\\LegacyRoot".into();
        settings.log_legacy_layout = true;
        let agni = Preferences::current(&settings);
        settings.release_chat_log_account();

        // the same agent id on the beta grid: nothing shared, no folder
        settings.select_chat_log_account("aditi", me);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.chat_log_location = "E:\\BetaLogs".into();
        let aditi = Preferences::current(&settings);
        // nor on another grid
        settings.select_chat_log_account("custom_grid_example_org", me);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.release_chat_log_account();

        let mut reloaded = restart(&settings);
        let saved = serde_json::to_value(reloaded.to_saved()).unwrap();
        assert!(saved["chat_log_accounts"].get(account_key("agni", me)).is_some());
        assert!(saved["chat_log_accounts"].get(account_key("aditi", me)).is_some());
        assert!(saved["chat_log_accounts"].get(me.to_string()).is_none());
        reloaded.select_chat_log_account("aditi", me);
        assert_eq!(Preferences::current(&reloaded), aditi);
        reloaded.select_chat_log_account("agni", me);
        assert_eq!(Preferences::current(&reloaded), agni);
    }

    #[test]
    fn an_entry_of_the_agent_id_alone_is_kept_but_taken_by_no_grid() {
        let me = Uuid::from_u128(1);
        // the first form of the entries, saved while that account was
        // logged in (its values are the top-level ones too)
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "chat_log_location": "D:\\LegacyRoot",
            "log_legacy_layout": true,
            "chat_log_accounts": {
                me.to_string(): { "chat_log_location": "D:\\LegacyRoot", "log_legacy_layout": true },
            },
        }))
        .unwrap();
        for grid in ["agni", "aditi"] {
            settings.select_chat_log_account(grid, me);
            assert_eq!(Preferences::current(&settings), Preferences::default(), "{grid}");
        }
        // the top-level values were one account's, not a former global
        // setting every account would inherit
        assert_eq!(settings.chat_log_legacy_default, None);
        settings.release_chat_log_account();
        let reloaded = restart(&settings);
        assert!(reloaded.chat_log_accounts.as_ref().unwrap().contains_key(&me.to_string()));
        // its folder is still one the uninstall lists
        assert_eq!(reloaded.chat_log_locations(), vec![PathBuf::from("D:\\LegacyRoot")]);
    }

    #[test]
    fn a_move_that_failed_after_the_logout_goes_back_to_its_own_account() {
        let (first, second) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut settings = Settings::default();
        settings.select_chat_log_account("agni", second);
        settings.chat_log_location = "E:\\Second".into();
        // the first account picked a new folder, then logged out while its
        // files were still moving
        settings.select_chat_log_account("agni", first);
        settings.chat_log_location = "D:\\New".into();
        settings.release_chat_log_account();
        settings.select_chat_log_account("agni", second);

        // the move fails in the second account's session: the first one's
        // entry goes back, the session's settings stay
        settings.restore_chat_log_account("agni", first, "D:\\Old".into(), false);
        assert_eq!(settings.chat_log_location, "E:\\Second");
        assert!(!settings.log_legacy_layout);
        // the same account on another grid is another entry
        settings.select_chat_log_account("aditi", first);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        settings.select_chat_log_account("agni", first);
        assert_eq!(settings.chat_log_location, "D:\\Old");

        // logged in again before the answer: its own fields go back
        settings.chat_log_location = "D:\\New".into();
        settings.restore_chat_log_account("agni", first, "D:\\Old".into(), true);
        assert_eq!(settings.chat_log_location, "D:\\Old");
        assert!(settings.log_legacy_layout);
        settings.release_chat_log_account();
        settings.select_chat_log_account("agni", first);
        assert_eq!(settings.chat_log_location, "D:\\Old");
    }
}
