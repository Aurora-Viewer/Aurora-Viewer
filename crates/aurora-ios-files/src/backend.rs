//! What shows the dialogs: UIKit on iPadOS (`uikit`), nothing elsewhere.

#[cfg(not(target_os = "ios"))]
use std::path::PathBuf;

#[cfg(not(target_os = "ios"))]
use crate::{FileDialog, MessageDialog, MessageDialogResult};

#[cfg(target_os = "ios")]
mod uikit;

#[cfg(target_os = "ios")]
pub(crate) use uikit::{alert, pick, save};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    File,
    Files,
    Folder,
}

// The three functions below are never called: the viewer and the media host
// take this crate on iOS only, and `rfd` everywhere else. They are there so
// that the workspace builds, lints and runs this crate's tests on every
// system.

#[cfg(not(target_os = "ios"))]
pub(crate) fn pick(dialog: &FileDialog, kind: Pick) -> Option<Vec<PathBuf>> {
    log::warn!("file picker ({kind:?}, {:?}) is not shown on this system", dialog.title);
    None
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn save(dialog: &FileDialog) -> Option<PathBuf> {
    log::warn!(
        "save dialog ({:?}) is not shown on this system",
        crate::proposed_name(dialog.file_name.as_deref(), &dialog.filters)
    );
    None
}

#[cfg(not(target_os = "ios"))]
pub(crate) fn alert(dialog: &MessageDialog) -> MessageDialogResult {
    log::warn!("{:?} {}: {}", dialog.level, dialog.title, dialog.description);
    crate::dismissed(&dialog.buttons)
}
