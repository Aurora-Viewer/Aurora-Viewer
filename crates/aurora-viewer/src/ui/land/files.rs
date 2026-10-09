//! Save / open dialogs of the access list export and import (FS:PP), run
//! on their own thread so the viewer keeps drawing (LLFilePickerReplyThread).

use std::sync::mpsc;

pub(super) enum Done {
    Saved(String),
    /// The file's text, for the list kind (AL_ACCESS / AL_BAN).
    Opened(u32, String),
    Failed(String),
    Cancelled,
}

pub(super) struct Job(mpsc::Receiver<Done>);

impl Job {
    pub(super) fn try_take(&self) -> Option<Done> {
        self.0.try_recv().ok()
    }
}

/// FFSAVE_CSV with a suggested name, then write `text`.
pub(super) fn save(file_name: &str, text: String) -> Job {
    let (tx, rx) = mpsc::channel();
    let file_name = file_name.to_owned();
    std::thread::spawn(move || {
        let done = match rfd::FileDialog::new()
            .set_file_name(&file_name)
            .add_filter("CSV", &["csv"])
            .save_file()
        {
            None => Done::Cancelled,
            Some(path) => match std::fs::write(&path, text) {
                Ok(()) => Done::Saved(path.display().to_string()),
                Err(e) => {
                    log::warn!("access list export to {}: {e}", path.display());
                    Done::Failed("Impossible d'enregistrer le fichier.".into())
                }
            },
        };
        let _ = tx.send(done);
    });
    Job(rx)
}

/// FFLOAD_ALL, then read the file as text.
pub(super) fn open(kind: u32) -> Job {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let done = match rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .add_filter("Tous les fichiers", &["*"])
            .pick_file()
        {
            None => Done::Cancelled,
            Some(path) => match std::fs::read(&path) {
                Ok(bytes) => Done::Opened(kind, String::from_utf8_lossy(&bytes).into_owned()),
                Err(e) => {
                    log::warn!("access list import from {}: {e}", path.display());
                    Done::Failed("Impossible de lire le fichier.".into())
                }
            },
        };
        let _ = tx.send(done);
    });
    Job(rx)
}
