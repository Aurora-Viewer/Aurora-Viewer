//! A native picker belongs to the import window that opened it. Closing that
//! window retires its request before asking the OS dialog to close, so a late
//! selection can never populate a newly opened importer.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

#[derive(Clone, Default)]
pub(super) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
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

/// Each native modal loop gets its own thread: cancellation can only address
/// that picker's windows, and an OS dialog never occupies a scene worker.
pub(super) fn spawn(request: Uuid, tx: Sender<Worker>, wake: egui::Context, work: impl FnOnce() -> Vec<Entry> + Send + 'static) {
    let (reply, repaint) = (tx.clone(), wake.clone());
    if let Err(error) = std::thread::Builder::new()
        .name("aurora-import-picker".into())
        .spawn(move || {
            crate::scene::jobs::below_normal_priority();
            complete(request, reply, repaint, work);
        })
    {
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
