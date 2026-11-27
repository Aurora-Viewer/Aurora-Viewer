//! The cache and transcript folders chosen in the preferences (the Legacy
//! Viewer's Set / Reset buttons of Network & Files, onClickSetCache and
//! onClickLogPath in llfloaterpreference.cpp). The folder dialog runs off
//! the frame; the cache folder applies at the next start
//! (cache_location.rs), the transcripts move at once with their files. The
//! interface language file is picked the same way and is read at the next
//! start too (language_file.rs).
use super::*;
use crate::ui::options::{Folder, FolderChoice};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(super) struct Folders {
    /// The folder dialog open, and what it is for.
    pick: Option<(Folder, Receiver<Option<PathBuf>>)>,
    /// The transcripts' folder before a move, put back if the move fails.
    pub(super) chat_log_previous: Option<String>,
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
                    Folder::LegacyTeleports(_) => aurora_i18n::tr!("ui-places-legacy-history-folder"),
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

    /// The folder dialog's answer, once it is closed.
    pub(super) fn poll_folder_pick(&mut self) {
        let Some((folder, rx)) = &self.folders.pick else {
            return;
        };
        let folder = *folder;
        match rx.try_recv() {
            Ok(picked) => {
                self.folders.pick = None;
                if let Some(path) = picked {
                    if let Folder::LegacyTeleports(agent) = folder {
                        if self.world.agent_id == agent {
                            self.open_teleport_history_import(Some(path));
                        }
                    } else {
                        self.set_folder(folder, path.to_string_lossy().into_owned());
                    }
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
            Folder::LegacyTeleports(_) => {}
            Folder::LanguageFile => {
                if self.settings.custom_language_file != location {
                    self.settings.custom_language_file = location;
                    self.folders.settings_changed = true;
                }
            }
            Folder::ChatLogs => {
                if self.settings.chat_log_location == location {
                    return;
                }
                // the files could not move now: keep the folder they are in
                if self.world.chat_log.busy() {
                    self.options_ui.chat_log_status = aurora_i18n::tr!("ui-options-chat-logs-busy").to_owned();
                    return;
                }
                self.options_ui.chat_log_status.clear();
                let previous = std::mem::replace(&mut self.settings.chat_log_location, location);
                if self.world.chat_log.relocate(chat_log_root(&self.settings.chat_log_location)) {
                    self.folders.chat_log_previous = Some(previous);
                }
                self.folders.settings_changed = true;
            }
        }
    }
}
