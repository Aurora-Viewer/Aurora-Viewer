//! The Chat Service's direct-IM history, as the LL viewer 26.5 reads it from
//! two region capabilities: the conversation list and the history pages of
//! one conversation. Port of the wire half of LLChatServiceHistoryCore
//! (indra/newview/llchatservicehistorycore.cpp, LL viewer 26.5, LGPL 2.1).
//!
//! Everything here is pure: the validators take the parsed LLSD of a
//! response and either accept all of it or none of it, so a malformed page
//! can never move the paging cursor or reach the archive.
//!
//! - `wire`: message ids (TimeUUID keys), canonical UUIDs, the conversation
//!   id, the text and timestamp checks, both response validators and the
//!   history request body.

pub mod wire;

pub use wire::*;

use uuid::Uuid;

/// The page size of every history request, echoed by the service.
pub const HISTORY_LIMIT: i32 = 100;
/// More entries reject the whole conversation list.
pub const MAX_LIST_ENTRIES: usize = 10_000;
/// Bound on a listed `conversation_id`, direct or not.
pub const MAX_LIST_CONVERSATION_ID_BYTES: usize = 512;
/// Bound on a row's `from_name`, on the wire and in the archive.
pub const MAX_FROM_NAME_BYTES: usize = 256;
/// Bound on a row's `message`, on the wire and in the archive.
pub const MAX_MESSAGE_BYTES: usize = 1024;
/// Rows one resident pass may receive before it gives up without writing
/// (MAX_PASS_ROWS, indra/newview/llchatservicehistory.cpp).
pub const MAX_PASS_ROWS: usize = 50_000;
/// Staged bytes one resident pass may hold before it gives up without
/// writing (MAX_PASS_BYTES), counted with [`HistoryRow::staged_bytes`].
pub const MAX_PASS_BYTES: usize = 64 * 1024 * 1024;
/// UUIDv1 ticks (100 ns since 1582-10-15) at the Unix epoch (UUID_EPOCH).
pub const UUID_EPOCH: u64 = 122_192_928_000_000_000;
/// UUIDv1 timestamps have 60 bits (UUID_TICK_LIMIT).
pub const UUID_TICK_LIMIT: u64 = 1 << 60;

/// The order the service (Cassandra `timeuuid`) gives message ids: the
/// 60-bit timestamp first, then bytes 8-15 compared as signed values.
/// The derived order is exactly that; it differs from the UUID text order
/// at timestamp word rollovers and for tail bytes of 0x80 and up.
/// Port of LLChatServiceHistoryCore::TimeUuidKey.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeUuidKey {
    pub ticks: u64,
    pub tail: [i8; 8],
}

impl TimeUuidKey {
    /// Unix seconds of the id's timestamp, `None` before 1970.
    pub fn unix_seconds(&self) -> Option<f64> {
        (self.ticks >= UUID_EPOCH).then(|| (self.ticks - UUID_EPOCH) as f64 / 10_000_000.0)
    }
}

/// One validated history row, the single representation shared by the
/// wire pages and the archive files (LLChatServiceHistoryCore::Row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    pub conversation_id: String,
    pub msg_id: String,
    pub key: TimeUuidKey,
    pub from_id: Uuid,
    pub from_name: String,
    pub message: String,
    pub dialog: i32,
    /// `created_at` as [`parse_created_at`] normalised it (an offset-less
    /// time gets `Z`): the value written to the archive.
    pub created_at: String,
}

impl HistoryRow {
    /// What the row costs against [`MAX_PASS_BYTES`]: its field bytes, with
    /// 36 for the sender id and the decimal dialog (the estimate of
    /// LLChatServiceHistory::syncResident).
    pub fn staged_bytes(&self) -> usize {
        self.conversation_id.len()
            + self.msg_id.len()
            + 36
            + self.from_name.len()
            + self.message.len()
            + self.dialog.to_string().len()
            + self.created_at.len()
    }
}

/// A direct conversation of the list: the other resident and the id of
/// its latest message (LLChatServiceHistoryCore::ListEntry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub resident_id: Uuid,
    pub last_msg_id: String,
}

/// A validated history page (LLChatServiceHistoryCore::Page): the rows
/// newer than the deletion cutoff, newest first, and the cursor of the
/// next older page while paging must go on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub rows: Vec<HistoryRow>,
    /// The `msg_id` of the page's oldest row, empty once `terminal`.
    pub next_cursor: String,
    /// An empty page, or the cutoff was reached: no older page to ask for.
    pub terminal: bool,
}
