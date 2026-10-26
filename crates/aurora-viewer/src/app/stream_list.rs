//! Advanced › Media streams backup: the saved music streams to and from the
//! Legacy Viewer's stream list file (`settings/stream_list.rs`), like
//! FSStreamListExportXML / FSStreamListImportXML
//! (indra/newview/llviewermenu.cpp). The file dialogs, the reading and the
//! writing run off the frame.
use super::*;
use crate::settings::stream_list;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

/// What a dialog's thread found.
enum Outcome {
    Exported(PathBuf),
    Imported(Vec<String>),
    NotAList,
    Failed,
    Cancelled,
}

#[derive(Default)]
pub(super) struct Job(Option<Receiver<Outcome>>);

impl App {
    /// Asks where to write the list (`stream_list.xml`), then writes it.
    pub(super) fn export_stream_list(&mut self) {
        if self.stream_list.0.is_some() {
            return;
        }
        let xml = stream_list::to_xml(&self.settings.audio.saved_streams);
        self.stream_list.0 = Some(spawn(move || {
            let Some(path) = file_dialog().set_file_name(stream_list::FILE_NAME).save_file() else {
                return Outcome::Cancelled;
            };
            match crate::atomic_file::write(&path, xml.as_bytes()) {
                Ok(()) => {
                    log::info!("stream list exported to {}", path.display());
                    Outcome::Exported(path)
                }
                Err(e) => {
                    log::warn!("stream list: cannot write {} ({e})", path.display());
                    Outcome::Failed
                }
            }
        }));
    }

    /// Asks for a list file and reads it; it replaces the saved streams.
    pub(super) fn import_stream_list(&mut self) {
        if self.stream_list.0.is_some() {
            return;
        }
        self.stream_list.0 = Some(spawn(|| {
            let Some(path) = file_dialog().pick_file() else {
                return Outcome::Cancelled;
            };
            match std::fs::read(&path) {
                Ok(bytes) => stream_list::from_xml(&bytes).map_or_else(
                    || {
                        log::warn!("stream list: {} is not a stream list", path.display());
                        Outcome::NotAList
                    },
                    Outcome::Imported,
                ),
                Err(e) => {
                    log::warn!("stream list: cannot read {} ({e})", path.display());
                    Outcome::Failed
                }
            }
        }));
    }

    /// The dialog's answer, once it is closed: the list in place and the
    /// notice (StreamListExportSuccess / StreamListImportSuccess).
    pub(super) fn poll_stream_list(&mut self) {
        let Some(rx) = &self.stream_list.0 else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Outcome::Cancelled,
        };
        self.stream_list.0 = None;
        let (kind, body) = match outcome {
            Outcome::Exported(path) => (
                crate::world::notifications::Kind::Info,
                aurora_i18n::tr!("stream-list-exported", file = path.display().to_string()),
            ),
            Outcome::Imported(streams) => {
                log::info!("stream list imported: {} streams", streams.len());
                let count = streams.len();
                self.settings.audio.saved_streams = streams;
                self.settings.save();
                (
                    crate::world::notifications::Kind::Info,
                    aurora_i18n::tr!("stream-list-imported", count = count),
                )
            }
            Outcome::NotAList => (
                crate::world::notifications::Kind::Alert,
                aurora_i18n::tr!("stream-list-not-a-list").to_owned(),
            ),
            Outcome::Failed => (
                crate::world::notifications::Kind::Alert,
                aurora_i18n::tr!("stream-list-failed").to_owned(),
            ),
            Outcome::Cancelled => return,
        };
        self.world.notifications.push(
            kind,
            aurora_i18n::tr!("ui-bars-media-streams-backup"),
            body,
            crate::world::notifications::Data::None,
        );
    }
}

fn file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title(aurora_i18n::tr!("ui-bars-media-streams-backup"))
        .add_filter(aurora_i18n::tr!("stream-list-file-type"), &["xml"])
}

fn spawn(work: impl FnOnce() -> Outcome + Send + 'static) -> Receiver<Outcome> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("aurora-stream-list".into()).spawn(move || {
        let _ = tx.send(work());
    });
    rx
}
