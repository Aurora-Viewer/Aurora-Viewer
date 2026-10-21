//! Ordered per-account unread state, outside the disposable inventory cache.

use super::Saved;
use crossbeam_channel::{Receiver, Sender};
use std::path::PathBuf;
use std::thread::JoinHandle;

pub(super) struct Storage {
    pub(super) loaded: Receiver<Saved>,
    writer: Option<Sender<Saved>>,
    worker: Option<JoinHandle<()>>,
}

impl Storage {
    pub(super) fn start(path: PathBuf) -> Option<Self> {
        let (loaded_tx, loaded) = crossbeam_channel::unbounded();
        let (writer, writes) = crossbeam_channel::unbounded::<Saved>();
        let worker = std::thread::Builder::new().name("aurora-received-read".into()).spawn(move || {
            let saved = match std::fs::read(&path) {
                Ok(bytes) => {
                    crate::disk_io::read(crate::disk_io::Origin::Other, bytes.len());
                    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                        log::warn!("Cannot read received inventory state: {error}");
                        Saved::default()
                    })
                }
                Err(error) => {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("Cannot read received inventory state: {error}");
                    }
                    Saved::default()
                }
            };
            let _ = loaded_tx.send(saved);
            while let Ok(mut saved) = writes.recv() {
                while let Ok(newer) = writes.try_recv() {
                    saved = newer;
                }
                let save = || -> std::io::Result<()> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let bytes = serde_json::to_vec(&saved).map_err(std::io::Error::other)?;
                    crate::atomic_file::write(&path, &bytes)
                };
                if let Err(error) = save() {
                    log::warn!("Cannot save received inventory state: {error}");
                }
            }
        });
        match worker {
            Ok(worker) => Some(Self {
                loaded,
                writer: Some(writer),
                worker: Some(worker),
            }),
            Err(error) => {
                log::warn!("Cannot start received inventory storage: {error}");
                None
            }
        }
    }

    pub(super) fn save(&self, saved: Saved) {
        if let Some(writer) = &self.writer {
            let _ = writer.send(saved);
        }
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
