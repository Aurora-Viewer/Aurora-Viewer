//! The Chat Service's direct-IM history on the viewer's side: the
//! per-resident CSV archive and the stitching of that archive with the
//! legacy plaintext transcript and the live session rows. Port of the
//! storage and composition halves of LLChatServiceHistoryCore
//! (indra/newview/llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1);
//! the wire format is `aurora_net::chat_service`.
//!
//! - `csv`: the archive's records, written and scanned from its bytes;
//! - `message`: a message of any source and the capture of a live
//!   message's clock;
//! - `merge`: the seam merge, the duplicate filter against live rows, the
//!   per-view composition and the session model update.

pub mod csv;
pub mod merge;
pub mod message;

#[cfg(test)]
mod tests;
