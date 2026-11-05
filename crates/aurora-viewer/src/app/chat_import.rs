//! The import of a Legacy Viewer's chat history, from Preferences › Chat
//! (`world::chat_log::import`): looking for its folders off the frame, what
//! the import window shows, and what follows a finished import.
use super::*;
use crate::world::chat_log::import::{self, Account, Found};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

#[derive(Default)]
pub(super) struct ChatImport {
    /// The searches under way: a folder chosen does not stop the first one.
    /// Their folders, and whether to tick them.
    looking: Vec<Receiver<(Vec<Found>, bool)>>,
    /// AURORA_DEMO_CHATLOG=import:again: its second run started.
    again: bool,
}

impl App {
    pub(super) fn chat_import_account(&self) -> Option<(Account, PathBuf)> {
        let login = self.world.login.as_ref()?;
        let log = &self.world.chat_log;
        let account = Account {
            first: login.first_name.clone(),
            last: login.last_name.clone(),
            grid: log.identity()?.grid.clone(),
        };
        Some((account, log.dir()?.to_path_buf()))
    }

    fn look_for_chat_logs(&mut self, chosen: Option<PathBuf>) {
        let Some((account, target)) = self.chat_import_account() else {
            return;
        };
        let apps = self.world.chat_log.legacy_apps();
        let (tx, rx) = channel();
        self.chat_import.looking.push(rx);
        let _ = std::thread::Builder::new().name("aurora-chatlog-find".into()).spawn(move || {
            let found = match chosen {
                Some(dir) => (vec![import::inspect_chosen(&dir, &account, &target)], true),
                None => (import::find(&apps, &account, &target), account.grid_known()),
            };
            let _ = tx.send(found);
        });
    }

    /// The import window opens: look for the viewers' folders.
    pub(super) fn chat_import_opened(&mut self) {
        self.options_ui.chat_import = ui::options::ChatImportUi { open: true, found: None };
        // a folder chosen in an earlier opening must not join this list
        self.chat_import.looking.clear();
        self.look_for_chat_logs(None);
    }

    /// A folder chosen by hand joins the list.
    pub(super) fn chat_import_chosen(&mut self, dir: PathBuf) {
        if self.options_ui.chat_import.open {
            self.look_for_chat_logs(Some(dir));
        }
    }

    /// The account is gone: what was found was found for it.
    pub(super) fn chat_import_closed(&mut self) {
        self.options_ui.chat_import = Default::default();
        self.chat_import.looking.clear();
    }

    pub(super) fn chat_import_start(&mut self, folders: Vec<PathBuf>) {
        // refused now (the import at login, a move, an erasing): the window
        // stays, to try again
        if self.world.chat_log.busy() {
            self.options_ui.chat_log_status = aurora_i18n::tr!("ui-options-chat-logs-busy").to_owned();
            return;
        }
        self.options_ui.chat_log_status.clear();
        self.options_ui.chat_import.open = false;
        self.world.chat_log.import(folders);
    }

    /// Every frame: what was found, how far the import is, how it ended.
    pub(super) fn poll_chat_import(&mut self) {
        let mut answers = Vec::new();
        self.chat_import.looking.retain(|rx| match rx.try_recv() {
            Ok(found) => {
                answers.push(found);
                false
            }
            Err(e) => e == std::sync::mpsc::TryRecvError::Empty,
        });
        for (found, tick) in answers {
            let list = self.options_ui.chat_import.found.get_or_insert_default();
            for f in found {
                if !list.iter().any(|(known, _)| known.dir == f.dir) {
                    let ticked = tick && f.files > 0 && !f.same;
                    list.push((f, ticked));
                }
            }
            self.demo_chat_import_found();
        }
        self.options_ui.chat_log_importing = self.world.chat_log.import_progress();
        let Some(report) = self.world.import_report.take() else {
            return;
        };
        // the import at login runs every session: its line in the
        // preferences only when something failed, the notice says the rest
        if !report.at_login || report.failed > 0 {
            self.options_ui.chat_log_status = import_status(&report);
        }
        if report.at_login && report.added > 0 {
            // on by default: say what came in, and where to turn it off
            self.world.system_notice(
                aurora_i18n::tr!(
                    "ui-options-chat-logs-imported-at-login",
                    count = report.added,
                    files = report.changed.len()
                ),
                crate::world::notice_topics::Topic::Info,
            );
        }
        self.history_ui.imported(&report.changed);
        self.demo_chat_import_done();
    }

    /// AURORA_DEMO_CHATLOG=import:<stage>: the window's Import button.
    fn demo_chat_import_found(&mut self) {
        use crate::demo::chatlog::{ImportStage, Mode, mode};
        if self.demo
            && matches!(mode(), Some(Mode::Import(ImportStage::Run | ImportStage::Again)))
            && let Some(found) = self.options_ui.chat_import.found.clone()
        {
            self.chat_import_start(found.into_iter().map(|(f, _)| f.dir).collect());
        }
    }

    /// AURORA_DEMO_CHATLOG=import:<stage>: a second run, then the History
    /// window on the merged file.
    fn demo_chat_import_done(&mut self) {
        use crate::demo::chatlog::{ImportStage, Mode, mode, peer_name};
        match mode() {
            Some(Mode::Import(ImportStage::Run | ImportStage::Login)) if self.demo => {
                self.panels.settings = false;
                // by file: the import can end before the peer's name is known
                let name = peer_name();
                let file = self.world.chat_log.history_file_of(&name);
                self.history_ui.show_file(name, file);
            }
            Some(Mode::Import(ImportStage::Again)) if self.demo && !self.chat_import.again => {
                self.chat_import.again = true;
                self.chat_import_opened();
            }
            _ => {}
        }
    }
}

/// The result as the preferences say it.
fn import_status(report: &import::Report) -> String {
    use aurora_i18n::tr;
    let mut text = if report.added == 0 {
        tr!("ui-options-chat-logs-import-nothing").to_owned()
    } else {
        format!(
            "{} {}",
            tr!("ui-options-chat-logs-imported", count = report.added, files = report.changed.len()),
            tr!("ui-options-chat-logs-import-reopen")
        )
    };
    if report.failed > 0 {
        text.push(' ');
        text.push_str(&tr!("ui-options-chat-logs-import-failed", count = report.failed));
    }
    if report.left_out > 0 {
        text.push(' ');
        text.push_str(&tr!("ui-options-chat-logs-import-left-out", count = report.left_out));
    }
    text
}
