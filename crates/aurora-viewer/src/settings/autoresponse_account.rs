//! The automatic responses of each account (#5024). The Legacy Viewer keeps
//! the autorespond modes, the rejections and every response text in
//! settings_per_account.xml (FSAutorespondMode, FSAutorespondNonFriendsMode,
//! FSRejectTeleportOffersMode, FSRejectAllGroupInvitesMode,
//! FSRejectFriendshipRequestsMode, DoNotDisturbModeResponse,
//! FSAutorespondModeResponse, FSSendAwayAvatarResponse,
//! FSSendMutedAvatarResponse, VoiceCallsFriendsOnly...), and the ad hoc and
//! away timing options in settings.xml (FSIgnoreAdHocSessions, AFKTimeout,
//! AvatarSitOnAway, QuitAfterSecondsOfAFK). Aurora kept all of them in
//! settings.json, so every account of the PC shared one auto-reply.
//!
//! Each account's part is a file of its own folder, written only by the
//! viewer logged in to it: two viewers open on two accounts never write
//! each other's choice. While an account is logged in, `Settings::autoresponse`
//! holds its values (the Preferences and the Online status menu change them
//! there) and settings.json keeps the values it had before the login.
//!
//! An account with no file yet takes those values once: the first account
//! gets all of them, as the shared settings were before; the next ones get
//! the texts only, with every mode off, like an account the Legacy Viewer
//! has never seen.
use super::Settings;
use crate::world::status::AutoResponse;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The file of an account's automatic responses, in its folder.
pub const FILE: &str = "autoresponse.json";

/// The per-account part of `AutoResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Account {
    autorespond: bool,
    autorespond_nonfriends: bool,
    reject_teleports: bool,
    reject_group_invites: bool,
    reject_friendship: bool,
    send_away: bool,
    send_muted: bool,
    dont_reject_friends_teleports: bool,
    friends_only: bool,
    dnd_text: String,
    autorespond_text: String,
    nonfriends_text: String,
    away_text: String,
    muted_text: String,
    reject_teleports_text: String,
    reject_friendship_text: String,
}

impl Default for Account {
    fn default() -> Self {
        Account::current(&AutoResponse::default())
    }
}

impl Account {
    fn current(a: &AutoResponse) -> Self {
        Account {
            autorespond: a.autorespond,
            autorespond_nonfriends: a.autorespond_nonfriends,
            reject_teleports: a.reject_teleports,
            reject_group_invites: a.reject_group_invites,
            reject_friendship: a.reject_friendship,
            send_away: a.send_away,
            send_muted: a.send_muted,
            dont_reject_friends_teleports: a.dont_reject_friends_teleports,
            friends_only: a.friends_only,
            dnd_text: a.dnd_text.clone(),
            autorespond_text: a.autorespond_text.clone(),
            nonfriends_text: a.nonfriends_text.clone(),
            away_text: a.away_text.clone(),
            muted_text: a.muted_text.clone(),
            reject_teleports_text: a.reject_teleports_text.clone(),
            reject_friendship_text: a.reject_friendship_text.clone(),
        }
    }

    fn apply(&self, a: &mut AutoResponse) {
        a.autorespond = self.autorespond;
        a.autorespond_nonfriends = self.autorespond_nonfriends;
        a.reject_teleports = self.reject_teleports;
        a.reject_group_invites = self.reject_group_invites;
        a.reject_friendship = self.reject_friendship;
        a.send_away = self.send_away;
        a.send_muted = self.send_muted;
        a.dont_reject_friends_teleports = self.dont_reject_friends_teleports;
        a.friends_only = self.friends_only;
        a.dnd_text.clone_from(&self.dnd_text);
        a.autorespond_text.clone_from(&self.autorespond_text);
        a.nonfriends_text.clone_from(&self.nonfriends_text);
        a.away_text.clone_from(&self.away_text);
        a.muted_text.clone_from(&self.muted_text);
        a.reject_teleports_text.clone_from(&self.reject_teleports_text);
        a.reject_friendship_text.clone_from(&self.reject_friendship_text);
    }

    /// The texts, every mode and option back to its default (off).
    fn texts_only(self) -> Self {
        let off = Account::default();
        Account {
            autorespond: off.autorespond,
            autorespond_nonfriends: off.autorespond_nonfriends,
            reject_teleports: off.reject_teleports,
            reject_group_invites: off.reject_group_invites,
            reject_friendship: off.reject_friendship,
            send_away: off.send_away,
            send_muted: off.send_muted,
            dont_reject_friends_teleports: off.dont_reject_friends_teleports,
            friends_only: off.friends_only,
            ..self
        }
    }
}

/// The account logged in: its file, and what settings.json keeps meanwhile.
#[derive(Debug, Clone)]
pub(crate) struct Binding {
    path: PathBuf,
    shared: Account,
    /// The values last given to the save thread: a settings save that did
    /// not change them does not write the file again.
    written: Arc<Mutex<Option<Account>>>,
}

/// The saved values of an account; `Ok(None)` when it has none yet.
fn read(path: &Path) -> Result<Option<Account>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn write(path: &Path, account: &Account) {
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("autoresponse: cannot create {}: {e}", dir.display());
        return;
    }
    match serde_json::to_vec_pretty(account) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(path, bytes) {
                log::warn!("autoresponse: cannot write {}: {e}", path.display());
            }
        }
        Err(e) => log::warn!("autoresponse: cannot encode the responses: {e}"),
    }
}

impl Binding {
    /// Hands the account's values to the save thread, when they changed
    /// since the last time.
    fn save(&self, account: Account) {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if written.as_ref() == Some(&account) {
            return;
        }
        *written = Some(account.clone());
        let path = self.path.clone();
        crate::saves::replace(path.clone(), move || write(&path, &account));
    }
}

impl Settings {
    /// Gives the account whose responses are kept in `file` its own values,
    /// before the first world update. Called at each login.
    pub(crate) fn select_autoresponse_account(&mut self, file: PathBuf) {
        self.release_autoresponse_account();
        let shared = Account::current(&self.autoresponse);
        let account = match read(&file) {
            Ok(Some(saved)) => saved,
            Ok(None) if !self.autoresponse_imported => {
                self.autoresponse_imported = true;
                shared.clone()
            }
            Ok(None) => shared.clone().texts_only(),
            Err(e) => {
                // never turn a reply on from a file that cannot be read
                log::warn!("autoresponse: {} cannot be read ({e}), its modes start off", file.display());
                shared.clone().texts_only()
            }
        };
        self.bind_autoresponse(file, shared, account);
    }

    /// Binds `account` to `file`, `shared` being what settings.json keeps.
    pub(super) fn bind_autoresponse(&mut self, file: PathBuf, shared: Account, account: Account) {
        account.apply(&mut self.autoresponse);
        self.autoresponse_account = Some(Binding {
            path: file,
            shared,
            written: Default::default(),
        });
    }

    /// At logout: writes the account's values and gives the login screen
    /// back the shared ones, so that an edit there changes no account.
    pub(crate) fn release_autoresponse_account(&mut self) {
        let Some(binding) = self.autoresponse_account.take() else {
            return;
        };
        binding.save(Account::current(&self.autoresponse));
        binding.shared.apply(&mut self.autoresponse);
    }

    /// The file and values of the account logged in, for a change that
    /// rebuilds the settings (a backup restored, a reset).
    pub(super) fn autoresponse_binding(&self) -> Option<(PathBuf, Account)> {
        let b = self.autoresponse_account.as_ref()?;
        Some((b.path.clone(), Account::current(&self.autoresponse)))
    }

    /// Every settings save writes the account's file too (when it changed).
    pub(super) fn save_autoresponse_account(&self) {
        if let Some(binding) = &self.autoresponse_account {
            binding.save(Account::current(&self.autoresponse));
        }
    }

    /// settings.json takes the shared values, never the account's.
    pub(super) fn shared_autoresponse(&mut self) {
        if let Some(binding) = self.autoresponse_account.take() {
            binding.shared.apply(&mut self.autoresponse);
        }
    }

    /// The shared part of `autoresponse` as it is now.
    pub(super) fn autoresponse_shared_part(&self) -> Account {
        Account::current(&self.autoresponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-autoresponse-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// settings.json written and read back, as at a restart.
    fn restart(settings: &Settings) -> Settings {
        serde_json::from_slice(&serde_json::to_vec(&settings.to_saved()).unwrap()).unwrap()
    }

    fn flush() {
        assert!(crate::saves::wait(std::time::Duration::from_secs(10)));
    }

    #[test]
    fn each_account_keeps_its_own_reply_through_saves_and_restarts() {
        let dir = scratch();
        let (store, alt) = (dir.join("store").join(FILE), dir.join("alt").join(FILE));
        let mut s = Settings::default();
        s.select_autoresponse_account(store.clone());
        s.autoresponse.autorespond = true;
        s.autoresponse.autorespond_text = "Back soon, the store is open".into();
        s.save_autoresponse_account();
        s.release_autoresponse_account();
        flush();
        let mut s = restart(&s);
        s.select_autoresponse_account(alt.clone());
        assert!(!s.autoresponse.autorespond, "the store's reply does not reach the alt");
        s.autoresponse.reject_teleports = true;
        s.release_autoresponse_account();
        flush();
        let mut s = restart(&s);
        s.select_autoresponse_account(store.clone());
        assert!(s.autoresponse.autorespond && !s.autoresponse.reject_teleports);
        assert_eq!(s.autoresponse.autorespond_text, "Back soon, the store is open");
        s.select_autoresponse_account(alt);
        assert!(!s.autoresponse.autorespond && s.autoresponse.reject_teleports);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn settings_json_never_takes_the_account_values() {
        let dir = scratch();
        let mut s = Settings::default();
        s.select_autoresponse_account(dir.join(FILE));
        s.autoresponse.autorespond = true;
        s.autoresponse.dnd_text = "mine".into();
        // the shared options stay in settings.json
        s.autoresponse.away_after_minutes = 12;
        let saved = restart(&s);
        assert!(!saved.autoresponse.autorespond);
        assert_eq!(saved.autoresponse.dnd_text, AutoResponse::default().dnd_text);
        assert_eq!(saved.autoresponse.away_after_minutes, 12);
        // the live values are untouched by the save
        assert!(s.autoresponse.autorespond);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn two_viewers_on_two_accounts_do_not_overwrite_each_other() {
        let dir = scratch();
        let (store, alt) = (dir.join("store").join(FILE), dir.join("alt").join(FILE));
        let mut first = Settings::default();
        let mut second = Settings::default();
        first.select_autoresponse_account(store.clone());
        second.select_autoresponse_account(alt.clone());
        first.autoresponse.autorespond = true;
        first.save_autoresponse_account();
        second.autoresponse.autorespond = false;
        second.autoresponse.reject_friendship = true;
        second.save_autoresponse_account();
        // the store's viewer saves again, after the alt turned it off
        first.autoresponse.away_text = "later".into();
        first.save_autoresponse_account();
        flush();
        let mut s = Settings::default();
        s.select_autoresponse_account(alt);
        assert!(!s.autoresponse.autorespond && s.autoresponse.reject_friendship);
        s.select_autoresponse_account(store);
        assert!(s.autoresponse.autorespond && !s.autoresponse.reject_friendship);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_former_shared_values_go_once_to_the_first_account_and_texts_to_all() {
        let dir = scratch();
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "autoresponse": {"autorespond": true, "reject_teleports": true, "friends_only": true,
                "autorespond_text": "shared text", "dnd_text": "busy"}
        }))
        .unwrap();
        s.select_autoresponse_account(dir.join("first").join(FILE));
        assert!(s.autoresponse.autorespond && s.autoresponse.reject_teleports && s.autoresponse.friends_only);
        assert_eq!(s.autoresponse.autorespond_text, "shared text");
        s.release_autoresponse_account();
        flush();
        // a restart keeps the import done
        let mut s = restart(&s);
        assert!(s.autoresponse.autorespond, "the login screen shows the former values");
        s.select_autoresponse_account(dir.join("second").join(FILE));
        assert!(!s.autoresponse.autorespond && !s.autoresponse.reject_teleports && !s.autoresponse.friends_only);
        assert_eq!(s.autoresponse.autorespond_text, "shared text");
        assert_eq!(s.autoresponse.dnd_text, "busy");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn an_unreadable_file_starts_with_every_mode_off() {
        let dir = scratch();
        let file = dir.join(FILE);
        std::fs::write(&file, b"{ not json").unwrap();
        let mut s = Settings::default();
        s.autoresponse.autorespond = true;
        s.autoresponse.autorespond_text = "kept".into();
        s.select_autoresponse_account(file);
        assert!(!s.autoresponse.autorespond);
        assert_eq!(s.autoresponse.autorespond_text, "kept");
        assert!(!s.autoresponse_imported, "a broken file does not use up the import");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reset_everything_resets_the_account_logged_in_only() {
        let dir = scratch();
        let (store, alt) = (dir.join("store").join(FILE), dir.join("alt").join(FILE));
        let mut s = Settings::default();
        s.select_autoresponse_account(store.clone());
        s.autoresponse.autorespond = true;
        s.select_autoresponse_account(alt.clone());
        s.autoresponse.reject_teleports = true;
        s.reset_keeping_account();
        assert!(!s.autoresponse.reject_teleports);
        s.save_autoresponse_account();
        flush();
        s.select_autoresponse_account(alt);
        assert!(!s.autoresponse.reject_teleports, "the reset reached the alt's file");
        s.select_autoresponse_account(store);
        assert!(s.autoresponse.autorespond, "the store keeps its reply");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_save_that_changes_nothing_does_not_write_again() {
        let dir = scratch();
        let file = dir.join(FILE);
        let mut s = Settings::default();
        s.select_autoresponse_account(file.clone());
        s.save_autoresponse_account();
        flush();
        std::fs::remove_file(&file).unwrap();
        s.save_autoresponse_account();
        flush();
        assert!(!file.exists());
        s.autoresponse.send_away = true;
        s.save_autoresponse_account();
        flush();
        assert!(file.exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
