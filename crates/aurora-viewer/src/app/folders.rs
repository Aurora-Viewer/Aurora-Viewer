//! The cache and transcript folders chosen in the preferences (the Legacy
//! Viewer's Set / Reset buttons of Network & Files, onClickSetCache and
//! onClickLogPath in llfloaterpreference.cpp). The folder dialog runs off
//! the frame; the cache folder applies at the next start
//! (cache_location.rs), the transcripts move at once with their files. The
//! interface language file is picked the same way and is read at the next
//! start too (language_file.rs).
//!
//! A transcripts' folder picked is first checked on the transcript thread
//! (`ChatLog::check_folder`, answered on a later frame): another account's
//! folder is refused; a Legacy Viewer's folder is shared only once the user
//! answers Share to the question it brings (an Aurora addition, Cancel
//! changes nothing); any other folder takes the account's files.
use super::*;
use crate::ui::options::{Folder, FolderChoice, ShareOffer};
use crate::world::chat_log::{ChatLog, FolderCheck, Refusal};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(super) struct Folders {
    /// The folder dialog open, and what it is for.
    pick: Option<(Folder, Receiver<Option<PathBuf>>)>,
    /// The transcripts' folder and sharing before a move, put back if the
    /// move fails.
    pub(super) chat_log_previous: Option<(String, bool)>,
    /// A folder setting changed outside the preferences' own rows.
    pub(super) settings_changed: bool,
}

/// The folder holding the transcripts' account folders for a setting
/// ("" = the default one).
pub(super) fn chat_log_root(location: &str) -> Option<PathBuf> {
    let location = location.trim();
    (!location.is_empty()).then(|| PathBuf::from(location))
}

impl App {
    pub(super) fn choose_folder(&mut self, folder: Folder, choice: FolderChoice) {
        match choice {
            FolderChoice::Pick => {
                if self.folders.pick.is_some() {
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel();
                self.folders.pick = Some((folder, rx));
                let title = match folder {
                    Folder::LegacyLogs => aurora_i18n::tr!("ui-options-chat-logs-import-choose"),
                    Folder::LanguageFile => aurora_i18n::tr!("language-file-choose"),
                    _ => aurora_i18n::tr!("ui-options-change-folder"),
                };
                let file = (folder == Folder::LanguageFile).then(|| {
                    let current = std::path::Path::new(self.settings.custom_language_file.trim());
                    (
                        aurora_i18n::tr!("language-file-filter"),
                        current.parent().map(std::path::Path::to_path_buf),
                    )
                });
                let _ = std::thread::Builder::new().name("aurora-folder-dialog".into()).spawn(move || {
                    let dialog = rfd::FileDialog::new().set_title(title);
                    let picked = match file {
                        Some((filter, folder)) => {
                            let dialog = dialog.add_filter(filter, &["ftl"]);
                            match folder.filter(|folder| folder.is_dir()) {
                                Some(folder) => dialog.set_directory(folder),
                                None => dialog,
                            }
                            .pick_file()
                        }
                        None => dialog.pick_folder(),
                    };
                    let _ = tx.send(picked);
                });
            }
            FolderChoice::Default => self.set_folder(folder, String::new()),
        }
    }

    /// The folder dialog's answer, once it is closed, and the checks of a
    /// transcripts' folder picked, once they come.
    pub(super) fn poll_folder_pick(&mut self) {
        poll_chat_log_folder_check(
            &mut self.world.chat_log,
            &mut self.settings,
            &mut self.options_ui,
            &mut self.folders,
        );
        let Some((folder, rx)) = &self.folders.pick else {
            return;
        };
        let folder = *folder;
        match rx.try_recv() {
            Ok(picked) => {
                self.folders.pick = None;
                if let Some(path) = picked {
                    self.set_folder(folder, path.to_string_lossy().into_owned());
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.folders.pick = None,
        }
    }

    fn set_folder(&mut self, folder: Folder, location: String) {
        match folder {
            Folder::Cache => {
                if self.settings.cache_location != location {
                    self.settings.cache_location = location;
                    self.folders.settings_changed = true;
                }
            }
            Folder::LegacyLogs => self.chat_import_chosen(PathBuf::from(location)),
            Folder::LanguageFile => {
                if self.settings.custom_language_file != location {
                    self.settings.custom_language_file = location;
                    self.folders.settings_changed = true;
                }
            }
            Folder::ChatLogs => {
                set_chat_log_folder(
                    &mut self.world.chat_log,
                    &mut self.settings,
                    &mut self.options_ui,
                    &mut self.folders,
                    location,
                );
            }
        }
    }

    pub(super) fn accept_chat_log_switch(&mut self) {
        accept_chat_log_switch_in(&mut self.world.chat_log, &mut self.options_ui);
    }

    /// Share answered to the question of a Legacy Viewer's folder. The demo
    /// shares its synthetic folder for the run only: its settings are every
    /// worktree's.
    pub(super) fn share_chat_log(&mut self, offer: ShareOffer) {
        if self.demo {
            if self.world.chat_log.identity().is_some() && !self.world.chat_log.busy() {
                self.world.chat_log.change_folder(Some(offer.root), true);
            }
            return;
        }
        share_chat_log_in(
            &mut self.world.chat_log,
            &mut self.settings,
            &mut self.options_ui,
            &mut self.folders,
            offer,
        );
    }

    pub(super) fn sync_chat_log_options_ui(&mut self) {
        sync_chat_log_options_ui(&mut self.options_ui, &self.settings, &self.world.chat_log);
    }

    pub(super) fn poll_chat_log_change(&mut self) {
        poll_chat_log_change(
            &mut self.world.chat_log,
            &mut self.settings,
            &mut self.options_ui,
            &mut self.folders,
        );
        // the demo's settings are every worktree's: an earlier session's
        // move changes none of them
        if self.demo {
            while self.world.chat_log.take_past_move_failure().is_some() {}
        } else {
            poll_past_chat_log_moves(&mut self.world.chat_log, &mut self.settings, &mut self.folders);
        }
    }
}

/// The settings back to the folder in use, after a change of folder that
/// did not happen. The demo's changes saved nothing to put back.
pub(super) fn restore_chat_log_previous(settings: &mut crate::settings::Settings, folders: &mut Folders) {
    if let Some((location, share)) = folders.chat_log_previous.take() {
        settings.chat_log_location = location;
        settings.log_legacy_layout = share;
        folders.settings_changed = true;
    }
}

/// The transcripts' folder and sharing the next login is placed with: the
/// settings', which are the account's once it is selected
/// (`Settings::select_chat_log_account`). Logged in, they only change
/// together with the files (`ChatLog::change_folder`).
pub(super) fn bind_chat_log_folder(chat_log: &mut ChatLog, settings: &crate::settings::Settings) {
    if chat_log.identity().is_none() {
        chat_log.set_folder(chat_log_root(&settings.chat_log_location));
        chat_log.set_sharing(settings.log_legacy_layout);
    }
}

/// The account's transcript settings at its logout, before its transcripts
/// close: a folder change still being placed, or one the transcript thread
/// dropped, puts back the folder in use; then they are saved for the
/// account, and the next one starts from its own. The folder to go back to
/// goes with the session: a move still under way keeps its own
/// (`ChatLog::take_past_move_failure`), for this account's entry only.
pub(super) fn chat_log_logout(chat_log: &mut ChatLog, settings: &mut crate::settings::Settings, folders: &mut Folders) {
    if chat_log.changing() || chat_log.take_dropped_change() {
        restore_chat_log_previous(settings, folders);
    }
    settings.release_chat_log_account();
    folders.chat_log_previous = None;
}

/// The moves of earlier sessions answered since: one that failed puts the
/// folder and the sharing its files stayed with back in its own account's
/// entry (its grid and agent), never in the settings of the account logged
/// in now, and says nothing to it.
fn poll_past_chat_log_moves(chat_log: &mut ChatLog, settings: &mut crate::settings::Settings, folders: &mut Folders) {
    while let Some(failed) = chat_log.take_past_move_failure() {
        let location = failed.accounts.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        settings.restore_chat_log_account(&failed.identity.grid, failed.identity.agent, location, failed.legacy_layout);
        folders.settings_changed = true;
    }
}

/// The end of a change of folder, taken before a new folder is (so the
/// folder to go back to is still the one in use): a move's answer (moved,
/// or failed, the transcripts staying where they were), or a change the
/// transcript thread dropped (nothing switched or moved). On a failure the
/// settings, which already name the new folder, go back.
fn poll_chat_log_change(
    chat_log: &mut ChatLog,
    settings: &mut crate::settings::Settings,
    options_ui: &mut crate::ui::options::OptionsUi,
    folders: &mut Folders,
) {
    if let Some(done) = chat_log.take_relocated() {
        match done {
            Ok(count) => {
                // the settings name the folder in use: nothing to go back to
                folders.chat_log_previous = None;
                options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-moved", count = count), false);
            }
            Err(e) => {
                log::warn!("conversation transcripts: moving failed ({e})");
                restore_chat_log_previous(settings, folders);
                options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-move-failed"), true);
            }
        }
    }
    if chat_log.take_dropped_change() {
        restore_chat_log_previous(settings, folders);
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-change-failed"), true);
    }
}

pub(super) fn sync_chat_log_options_ui(
    options_ui: &mut crate::ui::options::OptionsUi,
    settings: &crate::settings::Settings,
    chat_log: &ChatLog,
) {
    options_ui.chat_log_dir = chat_log.dir().map(std::path::Path::to_path_buf);
    options_ui.conversation_log_entries = chat_log.conversations.list.len();
    options_ui.chat_log_custom = !settings.chat_log_location.is_empty();
    options_ui.chat_log_shared = chat_log.shared();
    if options_ui.chat_log_dir.is_none() {
        options_ui.chat_log_switch_offer = None;
        options_ui.chat_log_share_offer = None;
    }
}

/// A transcripts' folder picked ("" = Default folder, which also stops
/// sharing): the default folder at once, another one once checked.
fn set_chat_log_folder(
    chat_log: &mut ChatLog,
    settings: &mut crate::settings::Settings,
    options_ui: &mut crate::ui::options::OptionsUi,
    folders: &mut Folders,
    location: String,
) {
    if chat_log.identity().is_none() {
        return;
    }
    // the files could not move now: keep the folder they are in
    if chat_log.busy() {
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-busy"), true);
        return;
    }
    options_ui.chat_log_switch_offer = None;
    options_ui.chat_log_share_offer = None;
    match chat_log_root(&location) {
        None => apply_chat_log_folder(chat_log, settings, options_ui, folders, String::new(), false),
        Some(picked) => {
            options_ui.clear_chat_log_status();
            chat_log.check_folder(picked);
        }
    }
}

/// The current folder is that Legacy Viewer root, shared already.
fn shares_already(chat_log: &ChatLog, settings: &crate::settings::Settings, root: &std::path::Path) -> bool {
    chat_log.shared() && chat_log_root(&settings.chat_log_location).is_some_and(|current| current == root)
}

/// The checks of the transcripts' folder picked last, applied when they
/// come: a refusal (with the account's own folder beside it to switch to),
/// the question whether to share a Legacy Viewer's folder, or the folder
/// taken. False while there is none.
fn poll_chat_log_folder_check(
    chat_log: &mut ChatLog,
    settings: &mut crate::settings::Settings,
    options_ui: &mut crate::ui::options::OptionsUi,
    folders: &mut Folders,
) -> bool {
    let Some(check) = chat_log.take_folder_check() else {
        return false;
    };
    match check {
        FolderCheck::Refused { why, folder, offer } => {
            let status = match why {
                Refusal::MainGrid => aurora_i18n::tr!("ui-options-chat-logs-main-grid-folder", folder = folder),
                Refusal::OtherAccount => aurora_i18n::tr!("ui-options-chat-logs-other-account", folder = folder),
            };
            options_ui.set_chat_log_status(status, true);
            options_ui.chat_log_switch_offer = offer.filter(|(viewer_root, _)| !shares_already(chat_log, settings, viewer_root));
        }
        // shared already: nothing to ask
        FolderCheck::Legacy { root, .. } if shares_already(chat_log, settings, &root) => {}
        FolderCheck::Legacy { root, folder, count } => {
            options_ui.chat_log_share_offer = Some(ShareOffer { root, folder, count });
        }
        FolderCheck::Plain { root } => {
            apply_chat_log_folder(chat_log, settings, options_ui, folders, root.to_string_lossy().into_owned(), false);
        }
    }
    true
}

/// "Switch to {folder}": the account's own folder in a Legacy Viewer's
/// root, checked and asked about like a folder picked.
fn accept_chat_log_switch_in(chat_log: &mut ChatLog, options_ui: &mut crate::ui::options::OptionsUi) {
    if chat_log.identity().is_none() {
        return;
    }
    if chat_log.busy() {
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-busy"), true);
        return;
    }
    let Some((viewer_root, _)) = options_ui.chat_log_switch_offer.take() else {
        return;
    };
    options_ui.clear_chat_log_status();
    chat_log.check_folder(viewer_root);
}

/// Share answered: the Legacy Viewer's folder taken, shared, and saved for
/// the account's next logins.
fn share_chat_log_in(
    chat_log: &mut ChatLog,
    settings: &mut crate::settings::Settings,
    options_ui: &mut crate::ui::options::OptionsUi,
    folders: &mut Folders,
    offer: ShareOffer,
) {
    if chat_log.identity().is_none() {
        return;
    }
    if chat_log.busy() {
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-busy"), true);
        return;
    }
    apply_chat_log_folder(
        chat_log,
        settings,
        options_ui,
        folders,
        offer.root.to_string_lossy().into_owned(),
        true,
    );
}

/// The folder and the sharing taken, saved and applied together.
fn apply_chat_log_folder(
    chat_log: &mut ChatLog,
    settings: &mut crate::settings::Settings,
    options_ui: &mut crate::ui::options::OptionsUi,
    folders: &mut Folders,
    location: String,
    share: bool,
) {
    options_ui.chat_log_switch_offer = None;
    options_ui.chat_log_share_offer = None;
    if settings.chat_log_location == location && settings.log_legacy_layout == share {
        return;
    }
    // an import, a move or an erasing started since the pick: the files
    // stay where they are, and so do the settings
    if chat_log.busy() {
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-busy"), true);
        return;
    }
    options_ui.clear_chat_log_status();
    let previous = (
        std::mem::replace(&mut settings.chat_log_location, location),
        std::mem::replace(&mut settings.log_legacy_layout, share),
    );
    if chat_log.change_folder(chat_log_root(&settings.chat_log_location), share) {
        folders.chat_log_previous = Some(previous);
    } else if chat_log.identity().is_some() {
        // refused: the settings keep naming the folder in use
        (settings.chat_log_location, settings.log_legacy_layout) = previous;
        options_ui.set_chat_log_status(aurora_i18n::tr!("ui-options-chat-logs-busy"), true);
        return;
    }
    sync_chat_log_options_ui(options_ui, settings, chat_log);
    folders.settings_changed = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);

    struct Prefs {
        chat_log: ChatLog,
        settings: crate::settings::Settings,
        options_ui: crate::ui::options::OptionsUi,
        folders: Folders,
    }

    impl Prefs {
        fn pick(&mut self, location: &std::path::Path) {
            set_chat_log_folder(
                &mut self.chat_log,
                &mut self.settings,
                &mut self.options_ui,
                &mut self.folders,
                location.to_string_lossy().into_owned(),
            );
            self.answered();
        }

        fn default_folder(&mut self) {
            set_chat_log_folder(
                &mut self.chat_log,
                &mut self.settings,
                &mut self.options_ui,
                &mut self.folders,
                String::new(),
            );
            self.moved();
        }

        /// The checks of the folder picked, applied as the frames do.
        fn answered(&mut self) {
            let start = std::time::Instant::now();
            while !poll_chat_log_folder_check(&mut self.chat_log, &mut self.settings, &mut self.options_ui, &mut self.folders) {
                assert!(start.elapsed() < WAIT, "the folder check never came");
                std::thread::yield_now();
            }
            self.moved();
        }

        /// The folder taken, and its files moved when they move.
        fn moved(&mut self) {
            assert!(self.chat_log.settle(WAIT));
            assert!(self.chat_log.flush(WAIT));
            while self.chat_log.busy() {
                if let Some(res) = self.chat_log.take_relocated() {
                    res.unwrap();
                }
                std::thread::yield_now();
            }
            sync_chat_log_options_ui(&mut self.options_ui, &self.settings, &self.chat_log);
        }

        fn share(&mut self) {
            let offer = self.options_ui.chat_log_share_offer.take().expect("the question");
            share_chat_log_in(
                &mut self.chat_log,
                &mut self.settings,
                &mut self.options_ui,
                &mut self.folders,
                offer,
            );
            self.moved();
        }
    }

    #[test]
    fn a_picked_legacy_folder_is_shared_only_once_share_is_answered() {
        let dir = std::env::temp_dir().join(format!("aurora_folders_share_{}", uuid::Uuid::new_v4()));
        let default_accounts = dir.join("aurora_default");
        let legacy_root = dir.join("Firestorm_x64");
        let own_legacy = legacy_root.join("jane_doe");
        let other_legacy = legacy_root.join("other_resident");
        let neutral_custom = dir.join("custom_aurora_logs");
        std::fs::create_dir_all(&default_accounts).unwrap();
        std::fs::create_dir_all(&own_legacy).unwrap();
        std::fs::create_dir_all(&other_legacy).unwrap();
        std::fs::create_dir_all(&neutral_custom).unwrap();
        let legacy_line = "[2026/10/12 14:00]  Bob Smith: from the legacy viewer\n";
        std::fs::write(own_legacy.join("Bob Smith.txt"), legacy_line).unwrap();
        std::fs::write(other_legacy.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();

        let mut chat_log = ChatLog::default();
        chat_log.set_default_folder(default_accounts.clone());
        chat_log.set_grid("agni".into());
        let me = uuid::Uuid::from_u128(0xA11CE);
        chat_log.login_named(me, Some(("Jane", "Doe")));
        assert!(chat_log.settle(WAIT));
        let initial_dir = chat_log.dir().unwrap().to_path_buf();
        let mut prefs = Prefs {
            chat_log,
            settings: crate::settings::Settings::default(),
            options_ui: crate::ui::options::OptionsUi::default(),
            folders: Folders::default(),
        };
        sync_chat_log_options_ui(&mut prefs.options_ui, &prefs.settings, &prefs.chat_log);

        // A neutral folder keeps Aurora's per-account layout, asked nothing.
        prefs.pick(&neutral_custom);
        assert!(!prefs.chat_log.shared());
        assert!(prefs.options_ui.chat_log_share_offer.is_none());
        assert!(
            prefs
                .options_ui
                .chat_log_dir
                .as_deref()
                .is_some_and(|d| d.starts_with(&neutral_custom))
        );
        prefs.default_folder();
        assert_eq!(prefs.options_ui.chat_log_dir.as_deref(), Some(initial_dir.as_path()));
        prefs.folders.settings_changed = false;

        // The Legacy Viewer's folder: a question, nothing changed yet.
        prefs.pick(&legacy_root);
        let question = crate::ui::options::ShareOffer {
            root: legacy_root.clone(),
            folder: "jane_doe".into(),
            count: 1,
        };
        assert_eq!(prefs.options_ui.chat_log_share_offer, Some(question.clone()));
        // Cancel (the notice drops the question): the folder and the sharing
        // as they were, the Legacy Viewer's files too.
        prefs.options_ui.chat_log_share_offer = None;
        prefs.moved();
        assert!(prefs.settings.chat_log_location.is_empty() && !prefs.settings.log_legacy_layout);
        assert!(!prefs.folders.settings_changed);
        assert!(!prefs.chat_log.shared());
        assert_eq!(prefs.chat_log.dir(), Some(initial_dir.as_path()));
        assert_eq!(std::fs::read_to_string(own_legacy.join("Bob Smith.txt")).unwrap(), legacy_line);
        assert_eq!(std::fs::read_dir(&own_legacy).unwrap().count(), 1);
        assert_eq!(std::fs::read_dir(&legacy_root).unwrap().count(), 2);

        // Picked again and Share answered: shared, and saved for the account.
        prefs.pick(&legacy_root);
        assert_eq!(prefs.options_ui.chat_log_share_offer, Some(question.clone()));
        prefs.share();
        assert_eq!(prefs.settings.chat_log_location, legacy_root.to_string_lossy().as_ref());
        assert!(prefs.settings.log_legacy_layout);
        assert!(prefs.folders.settings_changed);
        assert!(prefs.chat_log.shared() && prefs.options_ui.chat_log_shared);
        assert_eq!(prefs.options_ui.chat_log_dir.as_deref(), Some(own_legacy.as_path()));
        // The same folder picked again while sharing it: nothing to ask.
        prefs.pick(&legacy_root);
        assert!(prefs.options_ui.chat_log_share_offer.is_none());
        assert!(prefs.chat_log.shared());
        // "Default folder" stops sharing.
        prefs.default_folder();
        assert!(!prefs.chat_log.shared() && !prefs.settings.log_legacy_layout);
        assert_eq!(prefs.options_ui.chat_log_dir.as_deref(), Some(initial_dir.as_path()));

        // Another account's folder is refused, the account's own one beside it
        // offered; switching to it asks the same question.
        prefs.pick(&other_legacy);
        assert!(prefs.options_ui.chat_log_status_warn);
        assert_eq!(prefs.options_ui.chat_log_dir.as_deref(), Some(initial_dir.as_path()));
        assert_eq!(
            prefs.options_ui.chat_log_switch_offer,
            Some((legacy_root.clone(), "jane_doe".into()))
        );
        accept_chat_log_switch_in(&mut prefs.chat_log, &mut prefs.options_ui);
        prefs.answered();
        assert_eq!(prefs.options_ui.chat_log_switch_offer, None);
        assert_eq!(prefs.options_ui.chat_log_share_offer, Some(question));
        assert!(!prefs.chat_log.shared());
        prefs.share();
        assert!(prefs.chat_log.shared());
        assert_eq!(prefs.chat_log.dir(), Some(own_legacy.as_path()));
        // nothing of Aurora's was made in the Legacy Viewer's folders
        assert_eq!(std::fs::read_dir(&own_legacy).unwrap().count(), 1);
        assert_eq!(std::fs::read_dir(&legacy_root).unwrap().count(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_the_transcript_thread_never_takes_puts_the_settings_back() {
        use crate::world::chat_log::store::Worker;
        let dir = std::env::temp_dir().join(format!("aurora_folders_dropped_{}", uuid::Uuid::new_v4()));
        let default_accounts = dir.join("aurora_default");
        let custom = dir.join("custom_aurora_logs");
        std::fs::create_dir_all(&default_accounts).unwrap();
        std::fs::create_dir_all(&custom).unwrap();
        let mut chat_log = ChatLog::default();
        chat_log.set_default_folder(default_accounts.clone());
        chat_log.set_grid("agni".into());
        chat_log.login_named(uuid::Uuid::from_u128(0xA11CE), Some(("Jane", "Doe")));
        assert!(chat_log.settle(WAIT));
        let mut prefs = Prefs {
            chat_log,
            settings: crate::settings::Settings::default(),
            options_ui: crate::ui::options::OptionsUi::default(),
            folders: Folders::default(),
        };
        prefs.pick(&custom);
        let custom_setting = custom.to_string_lossy().into_owned();
        assert_eq!(prefs.settings.chat_log_location, custom_setting);
        let in_use = prefs.chat_log.dir().unwrap().to_path_buf();
        assert!(in_use.starts_with(&custom));
        let default_folder = |prefs: &mut Prefs| {
            set_chat_log_folder(
                &mut prefs.chat_log,
                &mut prefs.settings,
                &mut prefs.options_ui,
                &mut prefs.folders,
                String::new(),
            );
        };
        let poll = |prefs: &mut Prefs| {
            poll_chat_log_change(&mut prefs.chat_log, &mut prefs.settings, &mut prefs.options_ui, &mut prefs.folders);
        };
        let kept = |prefs: &Prefs| {
            assert_eq!(prefs.settings.chat_log_location, custom_setting);
            assert!(!prefs.chat_log.busy());
            assert_eq!(prefs.chat_log.dir(), Some(in_use.as_path()));
            assert!(prefs.options_ui.chat_log_status_warn);
        };

        // the thread gone: the placement refused at once, the settings
        // never name the default folder
        let (worker, queue) = Worker::stalled();
        drop(queue);
        prefs.chat_log.set_worker(worker);
        default_folder(&mut prefs);
        kept(&prefs);
        assert_eq!(prefs.options_ui.chat_log_status, aurora_i18n::tr!("ui-options-chat-logs-busy"));

        // the placement taken, then dropped unanswered: the settings, which
        // already name the default folder, go back once the frames see it
        let (worker, queue) = Worker::stalled();
        prefs.chat_log.set_worker(worker);
        default_folder(&mut prefs);
        assert!(prefs.settings.chat_log_location.is_empty());
        assert!(prefs.chat_log.busy() && prefs.chat_log.changing());
        drop(queue);
        assert!(prefs.chat_log.settle(WAIT));
        poll(&mut prefs);
        kept(&prefs);
        assert_eq!(
            prefs.options_ui.chat_log_status,
            aurora_i18n::tr!("ui-options-chat-logs-change-failed")
        );

        // placed, but the move refused (the thread gone after the answer)
        prefs.chat_log.set_worker(Worker::start().unwrap());
        default_folder(&mut prefs);
        assert!(prefs.chat_log.flush(WAIT));
        let (worker, queue) = Worker::stalled();
        drop(queue);
        prefs.chat_log.set_worker(worker);
        assert!(prefs.chat_log.settle(WAIT));
        poll(&mut prefs);
        kept(&prefs);
        assert_eq!(
            prefs.options_ui.chat_log_status,
            aurora_i18n::tr!("ui-options-chat-logs-change-failed")
        );

        // placed, the move taken, then dropped unanswered: it fails like a
        // move that ran, and the next lines go to the folder in use again
        prefs.chat_log.set_worker(Worker::start().unwrap());
        default_folder(&mut prefs);
        assert!(prefs.chat_log.flush(WAIT));
        let (worker, queue) = Worker::stalled();
        prefs.chat_log.set_worker(worker);
        assert!(prefs.chat_log.settle(WAIT));
        assert!(prefs.chat_log.busy());
        poll(&mut prefs);
        assert!(prefs.settings.chat_log_location.is_empty(), "the move has not answered yet");
        drop(queue);
        poll(&mut prefs);
        kept(&prefs);
        assert_eq!(
            prefs.options_ui.chat_log_status,
            aurora_i18n::tr!("ui-options-chat-logs-move-failed")
        );

        // a working thread again: the default folder taken, its files
        // moved, and nothing left to go back to
        prefs.chat_log.set_worker(Worker::start().unwrap());
        default_folder(&mut prefs);
        assert!(prefs.folders.chat_log_previous.is_some());
        assert!(prefs.chat_log.settle(WAIT));
        assert!(prefs.chat_log.flush(WAIT));
        let start = std::time::Instant::now();
        while prefs.chat_log.busy() {
            assert!(start.elapsed() < WAIT, "the move never answered");
            poll(&mut prefs);
            std::thread::yield_now();
        }
        assert!(prefs.settings.chat_log_location.is_empty());
        assert!(prefs.chat_log.dir().unwrap().starts_with(&default_accounts));
        assert!(!prefs.options_ui.chat_log_status_warn);
        assert_eq!(prefs.folders.chat_log_previous, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A move still under way at the logout (a slow folder) that fails in
    /// the next account's session: that account keeps its folder and
    /// settings, and the first account's own entry names the folder its
    /// files stayed in again.
    #[test]
    fn a_move_failing_after_the_logout_puts_back_only_its_own_account_s_folder() {
        use crate::world::chat_log::store::Worker;
        let dir = std::env::temp_dir().join(format!("aurora_folders_past_move_{}", uuid::Uuid::new_v4()));
        let default_accounts = dir.join("aurora_default");
        let (a_saved, b_saved) = (dir.join("a_saved"), dir.join("b_saved"));
        for folder in [&default_accounts, &a_saved, &b_saved] {
            std::fs::create_dir_all(folder).unwrap();
        }
        let (a, b) = (uuid::Uuid::from_u128(0xA11CE), uuid::Uuid::from_u128(0xB0B));
        let (a_setting, b_setting) = (a_saved.to_string_lossy().into_owned(), b_saved.to_string_lossy().into_owned());
        let mut settings = crate::settings::Settings::default();
        // each account's own folder, saved at earlier logins
        settings.select_chat_log_account("agni", b);
        settings.chat_log_location = b_setting.clone();
        settings.select_chat_log_account("agni", a);
        settings.chat_log_location = a_setting.clone();

        // A logs in, in its own folder
        let mut chat_log = ChatLog::default();
        chat_log.set_default_folder(default_accounts.clone());
        chat_log.set_grid("agni".into());
        bind_chat_log_folder(&mut chat_log, &settings);
        chat_log.login_named(a, Some(("Jane", "Doe")));
        assert!(chat_log.settle(WAIT));
        assert!(chat_log.dir().unwrap().starts_with(&a_saved));
        let mut prefs = Prefs {
            chat_log,
            settings,
            options_ui: crate::ui::options::OptionsUi::default(),
            folders: Folders::default(),
        };
        let poll = |prefs: &mut Prefs| {
            poll_chat_log_change(&mut prefs.chat_log, &mut prefs.settings, &mut prefs.options_ui, &mut prefs.folders);
            poll_past_chat_log_moves(&mut prefs.chat_log, &mut prefs.settings, &mut prefs.folders);
        };

        // A picks Default folder: placed by a working thread, its files
        // moved by one that never gets to it (a slow folder)
        set_chat_log_folder(
            &mut prefs.chat_log,
            &mut prefs.settings,
            &mut prefs.options_ui,
            &mut prefs.folders,
            String::new(),
        );
        assert!(prefs.chat_log.flush(WAIT));
        let (worker, queue) = Worker::stalled();
        prefs.chat_log.set_worker(worker);
        assert!(prefs.chat_log.settle(WAIT));
        poll(&mut prefs);
        assert!(prefs.chat_log.busy());
        assert!(prefs.settings.chat_log_location.is_empty());

        // A logs out (the app's steps, then the World's), B logs in
        chat_log_logout(&mut prefs.chat_log, &mut prefs.settings, &mut prefs.folders);
        prefs.chat_log.logout();
        assert_eq!(prefs.folders.chat_log_previous, None);
        prefs.settings.select_chat_log_account("agni", b);
        bind_chat_log_folder(&mut prefs.chat_log, &prefs.settings);
        prefs.chat_log.set_worker(Worker::start().unwrap());
        prefs.chat_log.login_named(b, Some(("Bob", "Smith")));
        assert!(prefs.chat_log.settle(WAIT));
        let b_dir = prefs.chat_log.dir().unwrap().to_path_buf();
        // B's own folder, not the one A picked
        assert!(b_dir.starts_with(&b_saved), "{b_dir:?}");
        assert!(!prefs.chat_log.busy());
        prefs.folders.settings_changed = false;
        poll(&mut prefs);
        assert!(!prefs.folders.settings_changed);

        // A's move fails now
        drop(queue);
        poll(&mut prefs);
        assert_eq!(prefs.chat_log.dir(), Some(b_dir.as_path()));
        assert_eq!(prefs.settings.chat_log_location, b_setting);
        assert!(!prefs.settings.log_legacy_layout);
        assert!(prefs.options_ui.chat_log_status.is_empty(), "nothing said to B");
        assert!(prefs.folders.settings_changed);
        assert_eq!(prefs.folders.chat_log_previous, None);
        // A's own entry names its files' folder again
        prefs.settings.select_chat_log_account("agni", a);
        assert_eq!(prefs.settings.chat_log_location, a_setting);
        prefs.settings.select_chat_log_account("agni", b);
        assert_eq!(prefs.settings.chat_log_location, b_setting);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
