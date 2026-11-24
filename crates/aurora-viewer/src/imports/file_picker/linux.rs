//! The same desktop portal as rfd, with the Request handle retained for Close.
//! A private bus connection and token keep simultaneous pickers independent.
use super::{Cancellation, PathBuf, Request};

mod portal;
#[cfg(target_os = "linux")]
mod zenity;

#[cfg(target_os = "linux")]
pub(super) fn select(request: Request, cancellation: &Cancellation) -> Vec<PathBuf> {
    match futures_lite::future::block_on(portal::select(&request, cancellation)) {
        Ok(paths) => paths,
        Err(error) if !cancellation.cancelled() => {
            // Preserve rfd's fallback on desktops without a file chooser portal,
            // but retain the child so importer cancellation closes it too.
            log::warn!("import file portal failed ({error}); trying zenity");
            zenity::select(&request, cancellation).unwrap_or_else(|error| {
                log::warn!("import file picker failed: {error}");
                Vec::new()
            })
        }
        Err(_) => Vec::new(),
    }
}
