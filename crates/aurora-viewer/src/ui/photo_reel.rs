//! The photo reel on screen (`snapshot::reel`): the stack of the session's
//! photos beside the viewfinder's shutter and in the photo window, and the
//! quick view that opens from it, to scroll them, send one again or delete
//! it. Aurora's own: the Legacy Viewer only shows the last photo.

use crate::snapshot::reel::Photo;
use crate::ui::snapshot::Destination;
use std::collections::HashMap;

/// What the reel asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum ReelAction {
    /// The quick view opened on this photo, or moved to it.
    Show(u64),
    Close,
    /// Sent again: held, then the destination's own flow.
    Send(u64, Destination),
    /// Its file in the file manager, its item in the inventory, the profile.
    Reveal(u64),
    ShowItem(u64),
    ShowProfile,
    /// Deleted, confirmed.
    Delete(u64),
}

/// What the app gives the reel to draw.
pub struct ReelView<'a> {
    /// Newest first.
    pub photos: &'a [Photo],
    pub thumbs: &'a HashMap<u64, egui::TextureHandle>,
    /// The quick view's pictures, at the screen's size, read on a worker.
    pub full: &'a HashMap<u64, egui::TextureHandle>,
    /// The destinations that can take a photo now (logged in, the region's
    /// postcards…), and none while one is sent.
    pub can_send: &'a [Destination],
    /// The time a photo was taken, as the clock writes it.
    pub time: &'a dyn Fn(i64) -> String,
}
