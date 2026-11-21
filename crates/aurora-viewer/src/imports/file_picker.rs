//! A native picker belongs to the import window that opened it. Closing that
//! window retires its request before asking the OS dialog to close, so a late
//! selection can never populate a newly opened importer.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    File,
    Files,
    Folder,
}

struct Request {
    kind: Kind,
    title: String,
    #[cfg(any(not(target_os = "macos"), test))]
    label: String,
    extensions: Vec<String>,
}

#[derive(Clone, Default)]
pub(super) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn select(&self, kind: Kind, title: impl Into<String>, _label: impl Into<String>, extensions: &[&str]) -> Vec<PathBuf> {
        let request = Request {
            kind,
            title: title.into(),
            #[cfg(any(not(target_os = "macos"), test))]
            label: _label.into(),
            extensions: extensions.iter().map(|ext| (*ext).into()).collect(),
        };
        self.pick(|| {
            #[cfg(target_os = "macos")]
            {
                macos::select(request, self.clone())
            }
            #[cfg(target_os = "linux")]
            {
                linux::select(request, self)
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                let mut dialog = rfd::FileDialog::new().set_title(request.title);
                if !request.extensions.is_empty() {
                    dialog = dialog.add_filter(request.label, &request.extensions);
                }
                match request.kind {
                    Kind::File => dialog.pick_file().into_iter().collect(),
                    Kind::Files => dialog.pick_files().unwrap_or_default(),
                    Kind::Folder => dialog.pick_folder().into_iter().collect(),
                }
            }
        })
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn pick<T: Default>(&self, pick: impl FnOnce() -> T) -> T {
        if self.cancelled() {
            return T::default();
        }
        // The timer lives only on this dedicated picker thread, and Windows'
        // native modal loop dispatches it while rfd waits for a selection.
        #[cfg(windows)]
        let Some(_guard) = windows::Guard::new(self.clone()) else {
            return T::default();
        };
        let result = pick();
        if self.cancelled() { T::default() } else { result }
    }
}

/// Wait on a picker worker, never the frame thread. Native callbacks can win
/// the race with cancellation; the caller also discards results after cancel.
#[cfg(any(target_os = "macos", test))]
fn wait_for_selection<T>(rx: &Receiver<T>, cancellation: &Cancellation, close: impl FnOnce()) -> Option<T> {
    loop {
        if cancellation.cancelled() {
            close();
            return None;
        }
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(result) => return Some(result),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                close();
                return None;
            }
        }
    }
}

pub(super) struct Pending {
    pub owner: Option<Choice>,
    pub cancellation: Cancellation,
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Imports {
    pub(super) fn cancel_file_picker(&mut self, owner: Choice) {
        if self.pending_picker.as_ref().is_some_and(|picker| picker.owner == Some(owner)) {
            self.pending_picker = None;
            self.pending_files = None;
            self.pending_file_choice = None;
            self.picker_files = false;
        }
    }
}

/// Each picker gets its own worker: native UI is dispatched to the platform's
/// event loop, and waiting for a selection never occupies a scene worker.
pub(super) fn spawn(request: Uuid, tx: Sender<Worker>, wake: egui::Context, work: impl FnOnce() -> Vec<Entry> + Send + 'static) {
    let (reply, repaint) = (tx.clone(), wake.clone());
    if let Err(error) = std::thread::Builder::new().name("aurora-import-picker".into()).spawn(move || {
        crate::scene::jobs::below_normal_priority();
        complete(request, reply, repaint, work);
    }) {
        log::warn!("import picker could not start: {error}");
        let _ = tx.send(Worker::Loaded(request, Vec::new()));
        wake.request_repaint();
    }
}

pub(super) fn complete(request: Uuid, tx: Sender<Worker>, wake: egui::Context, work: impl FnOnce() -> Vec<Entry>) {
    // The pool's generic panic handler cannot finish this UI request for us.
    let entries = crate::scene::jobs::caught(work).unwrap_or_default();
    let _ = tx.send(Worker::Loaded(request, entries));
    wake.request_repaint();
}
