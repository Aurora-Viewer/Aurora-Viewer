//! The cache and transcript folders chosen in the preferences (the Legacy
//! Viewer's Set / Reset buttons of Network & Files, onClickSetCache and
//! onClickLogPath in llfloaterpreference.cpp). The folder dialog runs off
//! the frame; the cache folder applies at the next start
//! (cache_location.rs), the transcripts move at once with their files.
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
                let title = aurora_i18n::tr!("ui-options-change-folder");
                let _ = std::thread::Builder::new().name("aurora-folder-dialog".into()).spawn(move || {
                    let _ = tx.send(rfd::FileDialog::new().set_title(title).pick_folder());
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
            Folder::ChatLogs => {
                if self.settings.chat_log_location == location {
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
