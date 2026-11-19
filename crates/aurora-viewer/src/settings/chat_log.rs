//! Account binding for conversation transcript folder and Legacy Viewer
//! sharing options. Keeps each logged-in avatar's chosen folder and its yes
//! to sharing isolated across logins, logouts and restarts while allowing
//! one-time migration from pre-split global settings (the folder and the
//! sharing option of PR #3588).

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

impl Settings {
    /// Bind before the account's transcripts open at login. Preserves any
    /// pre-split global folder/sharing settings in `chat_log_legacy_default` so
    /// every upgrading account without its own entry inherits them until it
    /// picks its own folder/layout.
    pub(crate) fn select_chat_log_account(&mut self, account: Uuid) {
        self.remember_chat_log_account();
        if self.chat_log_accounts.is_none() {
            let former = Preferences::current(self);
            if former != Preferences::default() && self.chat_log_legacy_default.is_none() {
                self.chat_log_legacy_default = Some(former);
            }
            self.chat_log_accounts = Some(Default::default());
        }
        let preferences = self
            .chat_log_accounts
            .as_ref()
            .and_then(|accounts| accounts.get(&account))
            .cloned()
            .or_else(|| self.chat_log_legacy_default.clone())
            .unwrap_or_default();
        self.chat_log_account = Some(account);
        preferences.apply(self);
        self.remember_chat_log_account();
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
        if let Some(account) = self.chat_log_account {
            let preferences = Preferences::current(self);
            self.chat_log_accounts
                .get_or_insert_with(Default::default)
                .insert(account, preferences);
        }
    }

    /// Every non-empty custom transcript root across the active session, saved
    /// accounts (sorted by account ID) and the inherited legacy default,
    /// deduplicated in that order.
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
        settings.select_chat_log_account(first);
        settings.chat_log_location = "D:\\Firestorm".into();
        settings.log_legacy_layout = true;
        let first_prefs = Preferences::current(&settings);

        // Logging out resets active fields to defaults while preserving account 1.
        settings.release_chat_log_account();
        assert_eq!(Preferences::current(&settings), Preferences::default());

        // Account 2 starts clean with default location and without sharing.
        settings.select_chat_log_account(second);
        assert_eq!(Preferences::current(&settings), Preferences::default());
        assert!(!settings.log_legacy_layout);
        settings.chat_log_location = "E:\\OtherLogs".into();
        let second_prefs = Preferences::current(&settings);

        settings.release_chat_log_account();
        let mut reloaded = restart(&settings);
        assert_eq!(reloaded.chat_log_account, None);
        assert_eq!(Preferences::current(&reloaded), Preferences::default());

        reloaded.select_chat_log_account(first);
        assert_eq!(Preferences::current(&reloaded), first_prefs);
        reloaded.select_chat_log_account(second);
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
        settings.select_chat_log_account(first);
        assert_eq!(Preferences::current(&settings), former);

        settings.release_chat_log_account();
        settings = restart(&settings);
        settings.select_chat_log_account(second);
        assert_eq!(Preferences::current(&settings), former);

        // Changing account 2's settings isolates it without touching account 1 or future alts.
        settings.chat_log_location = "E:\\AltTwo".into();
        let second_prefs = Preferences::current(&settings);

        settings.release_chat_log_account();
        settings = restart(&settings);
        settings.select_chat_log_account(third);
        assert_eq!(Preferences::current(&settings), former);

        settings.select_chat_log_account(first);
        assert_eq!(Preferences::current(&settings), former);
        settings.select_chat_log_account(second);
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

        settings.select_chat_log_account(first);
        settings.chat_log_location = "D:\\LogsA".into();
        let first_prefs = Preferences::current(&settings);

        settings.select_chat_log_account(second);
        settings.chat_log_location = "E:\\LogsB".into();

        settings.reset_keeping_account();
        let mut loaded = restart(&settings);
        loaded.select_chat_log_account(second);
        assert_eq!(Preferences::current(&loaded), Preferences::default());
        loaded.select_chat_log_account(first);
        assert_eq!(Preferences::current(&loaded), first_prefs);
        // An alt not yet seen keeps inheriting the pre-split global settings across reset.
        loaded.select_chat_log_account(third);
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
        settings.select_chat_log_account(third);
        settings.chat_log_location = "E:\\Other".into();
        settings.select_chat_log_account(second);
        settings.chat_log_location = "D:\\Shared".into();
        settings.select_chat_log_account(first);
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
        settings.select_chat_log_account(active);
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
}
