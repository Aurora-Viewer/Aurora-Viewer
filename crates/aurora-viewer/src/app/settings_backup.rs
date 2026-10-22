//! Back up the settings to a file and restore them (Preferences › Language
//! and updates › Backup; `settings/backup.rs` says what a backup holds).
//! The file dialogs, the reading and the writing run off the frame; the
//! restore's confirmation is the preferences' (`ui/options/backup.rs`).
use super::*;
use crate::settings::backup::Backup;
use crate::ui::options::backup::{Pending, Restored};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// What a dialog's thread found.
enum Outcome {
    Saved(PathBuf),
    Read(Backup),
    NotABackup,
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(super) struct Job(Option<Receiver<Outcome>>);

impl App {
    /// Takes the settings and the windows' layout as they are, then asks
    /// where to write them, with the skin's and the accounts' files.
    pub(super) fn back_up_settings(&mut self, ctx: &egui::Context) {
        if self.settings_backup.0.is_some() {
            return;
        }
        self.options_ui.backup_status.clear();
        let mut backup = Backup::new(&self.settings, ui::layout::text(ctx));
        let date = crate::snapshot::local_stamp();
        let name = format!("aurora-settings-{}.json", date.get(..10).unwrap_or(&date));
        self.settings_backup.0 = Some(spawn(move || {
            let Some(path) = file_dialog().set_file_name(name).save_file() else {
                return Outcome::Cancelled;
            };
            backup.read_files(&crate::settings::config_dir());
            match backup
                .to_bytes()
                .map_err(std::io::Error::other)
                .and_then(|b| crate::atomic_file::write(&path, &b))
            {
                Ok(()) => {
                    log::info!("settings backed up to {}", path.display());
                    Outcome::Saved(path)
                }
                Err(e) => {
                    log::warn!("settings backup: cannot write {} ({e})", path.display());
                    Outcome::Failed
                }
            }
        }));
    }

    /// Asks for a backup and reads it; its restore waits for the
    /// confirmation of the preferences.
    pub(super) fn pick_settings_backup(&mut self) {
        if self.settings_backup.0.is_some() {
            return;
        }
        self.options_ui.restore_status.clear();
        self.settings_backup.0 = Some(spawn(|| {
            let Some(path) = file_dialog().pick_file() else {
                return Outcome::Cancelled;
            };
            match std::fs::read(&path) {
                Ok(bytes) => Backup::parse(&bytes).map_or(Outcome::NotABackup, Outcome::Read),
                Err(e) => {
                    log::warn!("settings backup: cannot read {} ({e})", path.display());
                    Outcome::NotABackup
                }
            }
        }));
    }

    /// The dialog's answer, once it is closed.
    pub(super) fn poll_settings_backup(&mut self) {
        let Some(rx) = &self.settings_backup.0 else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Outcome::Cancelled,
        };
        self.settings_backup.0 = None;
        let ui = &mut self.options_ui;
        match outcome {
            Outcome::Saved(path) => {
                ui.backup_status = aurora_i18n::tr!("ui-options-backup-saved", path = path.display().to_string());
            }
            Outcome::Failed => ui.backup_status = aurora_i18n::tr!("ui-options-backup-save-failed").to_owned(),
            Outcome::Read(backup) => {
                ui.restore = Some(Pending::new(backup, self.world.login.as_ref().map(|l| l.agent_id)));
            }
            Outcome::NotABackup => ui.restore_status = aurora_i18n::tr!("ui-options-backup-not-a-backup").to_owned(),
            Outcome::Cancelled => {}
        }
    }

    /// Puts back what the preferences did not (they swapped the settings):
    /// the windows' layout, the skin, the accounts' files. True when the
    /// layout was replaced.
    pub(super) fn restore_settings(&mut self, ctx: &egui::Context, restored: Restored) -> bool {
        let Restored { backup, parts } = restored;
        let config = crate::settings::config_dir();
        let mut failed = false;
        let mut layout = false;
        if parts.layout
            && let Some(text) = backup.layout()
        {
            match ui::layout::apply(ctx, text) {
                Ok(()) => layout = true,
                Err(e) => {
                    log::warn!("settings backup: layout not restored ({e})");
                    failed = true;
                }
            }
        }
        if parts.skin
            && let Err(e) = backup.write_skin(&config)
        {
            log::warn!("settings backup: skin not restored ({e})");
            failed = true;
        }
        if parts.accounts {
            // the account logged in saves its own files over them
            match backup.write_accounts(&config, self.world.login.as_ref().map(|l| l.agent_id)) {
                Ok(count) => log::info!("settings backup: files of {count} accounts restored"),
                Err(e) => {
                    log::warn!("settings backup: account files not all restored ({e})");
                    failed = true;
                }
            }
        }
        self.settings.save();
        log::info!(
            "settings restored from the backup of {} (settings {}, layout {layout}, skin {}, accounts {})",
            backup.created(),
            parts.settings,
            parts.skin,
            parts.accounts
        );
        self.options_ui.restore_status = if failed {
            aurora_i18n::tr!("ui-options-backup-restore-failed").to_owned()
        } else {
            aurora_i18n::tr!("ui-options-backup-restored").to_owned()
        };
        self.world.system_message(self.options_ui.restore_status.clone());
        layout
    }
}

fn file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title(aurora_i18n::tr!("ui-options-backup"))
        .add_filter(aurora_i18n::tr!("ui-options-backup-file-type"), &["json"])
}

fn spawn(work: impl FnOnce() -> Outcome + Send + 'static) -> Receiver<Outcome> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("aurora-settings-backup".into()).spawn(move || {
        let _ = tx.send(work());
    });
    rx
}
