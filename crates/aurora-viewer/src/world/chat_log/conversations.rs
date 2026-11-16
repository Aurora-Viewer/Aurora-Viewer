//! The conversation log: who we had an instant message, group chat or
//! conference with, and when last. Port of LLConversationLog and
//! LLConversation (indra/newview/llconversationlog.cpp), the sort of
//! LLConversationLogList (llconversationloglist.cpp: its name and date
//! comparators, the filter of rebuildList) and the `conversation.log` file
//! of the account's transcript folder, in the Legacy Viewer's own format so
//! that the one an import brings works as it is (originally LGPL 2.1).
//!
//! The nearby chat is not a conversation here, as in the Legacy Viewer (its
//! log follows the IM sessions only).
//!
//! The list lives in memory; the file is read once at login and written
//! whole after a change (the Legacy Viewer writes it at exit only, so a
//! crash lost the session's entries), on the transcript thread.

use super::format::Slt;
use std::collections::HashSet;
use uuid::Uuid;

/// Its name in the account's transcript folder (LLConversationLog::getFileName).
pub const FILE: &str = "conversation.log";

/// Days an entry is kept before a login drops it (FSConversationLogLifetime,
/// 120; the Legacy Viewer has no interface for it either).
pub const LIFETIME_DAYS: i64 = 120;

/// The session types of LLIMModel::LLIMSession::SType, as the file numbers
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// P2P_SESSION: an instant message with a resident.
    Private,
    /// GROUP_SESSION.
    Group,
    /// ADHOC_SESSION: a conference.
    Conference,
    /// AVALINE_SESSION and anything newer: kept as read.
    Other(i32),
}

impl Kind {
    pub fn code(self) -> i32 {
        match self {
            Kind::Private => 0,
            Kind::Group => 1,
            Kind::Conference => 2,
            Kind::Other(n) => n,
        }
    }

    pub fn from_code(code: i32) -> Kind {
        match code {
            0 => Kind::Private,
            1 => Kind::Group,
            2 => Kind::Conference,
            n => Kind::Other(n),
        }
    }
}

/// One entry (LLConversation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// Last interaction, in seconds since 1970 (mTime).
    pub time: i64,
    pub kind: Kind,
    /// The resident's complete name, the group's or the conference's.
    pub name: String,
    /// Its transcript, without folder and extension (mHistoryFileName).
    pub history_file: String,
    /// The session: the two residents' ids mixed for an instant message
    /// (LLIMMgr::computeSessionID), the group's id for its chat.
    pub session: Uuid,
    /// The other resident (the group's id for a group chat we opened).
    pub participant: Uuid,
    /// Messages that arrived while we were logged out, not read yet
    /// (mHasOfflineIMs).
    pub offline: bool,
}

/// The session id of an instant message with `other`
/// (LLIMMgr::computeSessionID for IM_NOTHING_SPECIAL): both ids xored.
pub fn private_session(agent: Uuid, other: Uuid) -> Uuid {
    Uuid::from_u128(agent.as_u128() ^ other.as_u128())
}

/// LLURI::escape with its default set: every byte that is not a letter, a
/// digit or `-._~` as `%XX`.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// LLURI::unescape: `%XX` back to its byte; a `%` that starts no escape
/// stays.
pub fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One line of the file, read as LLConversationLog::loadFromFile's sscanf
/// `[%lld] %d %d %d %[^|]| %s %s %[^|]|` does. The third number held the
/// voice state before CHUI-348 and is ignored. None for a line that is not
/// an entry.
pub fn parse_line(line: &str) -> Option<Conversation> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (time, rest) = rest.split_once(']')?;
    let time: i64 = time.trim().parse().ok()?;
    let mut numbers = [0i32; 3];
    let mut rest = rest;
    for n in &mut numbers {
        let t = rest.trim_start();
        let end = t.find(|c: char| c.is_whitespace()).unwrap_or(t.len());
        *n = t[..end].parse().ok()?;
        rest = &t[end..];
    }
    let [kind, _voice, offline] = numbers;
    let kind = Kind::from_code(kind);
    let (name, rest) = rest.trim_start().split_once('|')?;
    // `%s %s`: two words, then ` %[^|]` the file name up to its bar
    let word = |s: &str| -> Option<(String, String)> {
        let s = s.trim_start();
        let end = s.find(char::is_whitespace)?;
        Some((s[..end].to_owned(), s[end..].to_owned()))
    };
    let (participant, rest) = word(rest)?;
    let (session, rest) = word(&rest)?;
    let participant = Uuid::parse_str(&participant).ok()?;
    let session = Uuid::parse_str(&session).ok()?;
    let rest = rest.trim_start();
    let history = rest.split_once('|').map_or(rest, |(h, _)| h);
    // a conference's names are written as they are (they come from the
    // server and carry no bar)
    let plain = kind == Kind::Conference;
    let read = |s: &str| if plain { s.to_owned() } else { unescape(s) };
    Some(Conversation {
        time,
        kind,
        name: read(name),
        history_file: read(history.trim_end()),
        session,
        participant,
        offline: offline != 0,
    })
}

/// LLConversationLog::saveToFile's line, with its end of line.
pub fn format_line(c: &Conversation) -> String {
    let plain = c.kind == Kind::Conference;
    let write = |s: &str| if plain { s.replace('|', " ") } else { escape(s) };
    format!(
        "[{}] {} 0 {} {}| {} {} {}|{}",
        c.time,
        c.kind.code(),
        i32::from(c.offline),
        write(&c.name),
        c.participant.hyphenated(),
        c.session.hyphenated(),
        write(&c.history_file),
        super::store::EOL
    )
}

/// The file's text for a list.
pub fn format_file(list: &[Conversation]) -> String {
    list.iter().map(format_line).collect()
}

/// Read a file: its entries, without those older than `LIFETIME_DAYS` at
/// `now`, and whether some were dropped (then the file is written again,
/// like the purge of loadFromFile).
pub fn parse_file(bytes: &[u8], now: i64) -> (Vec<Conversation>, bool) {
    let text = String::from_utf8_lossy(bytes);
    let mut purged = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(c) = parse_line(line) else {
            continue;
        };
        if is_older_than(&c, now, LIFETIME_DAYS) {
            purged = true;
            continue;
        }
        out.push(c);
    }
    (out, purged)
}

/// LLConversation::isOlderThan.
pub fn is_older_than(c: &Conversation, now: i64, days: i64) -> bool {
    now - c.time > days * 86_400
}

/// The time shown on a row (LLConversation::createTimestamp): the month,
/// day, year, hour and minute in Second Life time, with the 24 hour clock
/// (Use24HourClock, on).
pub fn timestamp(time: i64) -> String {
    let t = Slt::from_unix(time);
    format!("{:02}/{:02}/{:04} {:02}:{:02}", t.month, t.day, t.year, t.hour, t.minute)
}

/// Add the entries of `incoming` to `list`: one with a session already
/// listed updates it when it is as recent or newer (its time, and its name,
/// file and resident when it has them), the others are added. `offline`:
/// keep their unread offline mark (not for an import, whose marks are the
/// other viewer's). True when the list changed.
pub fn merge(list: &mut Vec<Conversation>, incoming: Vec<Conversation>, offline: bool) -> bool {
    let mut changed = false;
    for mut c in incoming {
        c.offline &= offline;
        match list.iter_mut().find(|e| e.session == c.session) {
            Some(e) if c.time >= e.time => {
                let before = e.clone();
                e.time = c.time;
                if !c.name.is_empty() {
                    e.name = c.name;
                }
                if !c.history_file.is_empty() {
                    e.history_file = c.history_file;
                }
                if !c.participant.is_nil() {
                    e.participant = c.participant;
                }
                e.offline |= c.offline;
                changed |= *e != before;
            }
            Some(_) => {}
            None => {
                list.push(c);
                changed = true;
            }
        }
    }
    changed
}

/// Seconds since 1970 now (time_corrected).
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A file being read.
type Reading = crossbeam_channel::Receiver<std::io::Result<Option<Vec<u8>>>>;

/// What the user removed from the log while an import was reading: its
/// entries must not come back with the import's result.
#[derive(Debug, Default)]
pub struct Removals {
    cleared: bool,
    removed: Vec<(Uuid, i64)>,
}

impl Removals {
    /// Drop from an import's entries what was cleared or removed.
    pub fn filter(&self, incoming: &mut Vec<Conversation>) {
        if self.cleared {
            incoming.clear();
        } else {
            incoming.retain(|c| !self.removed.contains(&(c.session, c.time)));
        }
    }
}

/// The answer of a publication: written, or why not. A dropped job (full
/// queue, thread gone) closes the channel without an answer.
pub type Published = crossbeam_channel::Receiver<std::io::Result<()>>;

/// The log of the account logged in.
#[derive(Default)]
pub struct Log {
    /// The entries, in the file's order (oldest first as they were added).
    pub list: Vec<Conversation>,
    /// Logging on for this session (KeepConversationLogTranscripts > 0 at
    /// login, or turned on since; mLoggingEnabled).
    pub logging: bool,
    reading: Option<Reading>,
    /// What this session logged while the file was being read.
    early: Vec<Conversation>,
    /// Changed since the last save the worker acknowledged.
    dirty: bool,
    /// Removals since an import started (None: no import under way).
    removals: Option<Removals>,
    /// Publications the worker has not answered yet.
    publishing: Vec<Published>,
    /// Wait before the next try after a failed save (doubles, up to a
    /// minute); the plain interval otherwise.
    retry: std::time::Duration,
    /// The file could not be read: it is never written over this session.
    unreadable: bool,
    saved_at: Option<std::time::Instant>,
    /// Changes whenever the list does (the window caches its order).
    pub revision: u64,
    /// The conversations open at the last look: one that appears is a new
    /// session (sessionAdded).
    pub(crate) known: HashSet<Uuid>,
    /// Residents whose entry waits for their complete name.
    pub(crate) unnamed: HashSet<Uuid>,
}

/// Time between two saves of a changing log.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
/// Longest wait between two tries of a save that keeps failing.
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(60);
/// How long a logout or exit waits for a file still being read, so that the
/// session's entries are merged into it and not lost.
const READ_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

impl Log {
    /// Start a session's log (enableLogging): `reading` brings the file.
    pub(super) fn start(&mut self, reading: Option<Reading>) {
        *self = Log {
            logging: true,
            reading,
            revision: self.revision + 1,
            ..Default::default()
        };
    }

    /// No more logging (enableLogging(0)): the list stays shown.
    pub(super) fn stop(&mut self) {
        self.logging = false;
        self.known.clear();
        self.unnamed.clear();
        self.revision += 1;
    }

    /// Back to nothing (logout).
    pub(super) fn reset(&mut self) {
        *self = Log {
            revision: self.revision + 1,
            ..Default::default()
        };
    }

    /// The file read, if it came: its entries, then what the session logged
    /// meanwhile. An unreadable file leaves the session's entries alone and
    /// is not written over.
    pub(super) fn poll_read(&mut self, now: i64) {
        let Some(rx) = &self.reading else { return };
        let read = match rx.try_recv() {
            Ok(read) => read,
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            // the job was dropped without an answer: as an unreadable file
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(std::io::Error::other("the read job ended without an answer")),
        };
        self.apply_read(read, now);
    }

    /// The file is still being read.
    pub(super) fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// Before a logout or an exit: wait (briefly) for the file still being
    /// read and merge the session's entries into it, so that the last save
    /// does not lose them. Gives up, and says so, when the disk is too slow.
    pub(super) fn finish_read(&mut self, now: i64) {
        let Some(rx) = &self.reading else { return };
        match rx.recv_timeout(READ_WAIT) {
            Ok(read) => self.apply_read(read, now),
            Err(_) => log::warn!("conversation log: the file was still being read; this session's entries are not saved"),
        }
    }

    fn apply_read(&mut self, read: std::io::Result<Option<Vec<u8>>>, now: i64) {
        self.reading = None;
        let (mut list, purged) = match read {
            Ok(Some(bytes)) => parse_file(&bytes, now),
            Ok(None) => (Vec::new(), false),
            Err(e) => {
                log::warn!("conversation log: cannot be read ({e}); it is left as it is this session");
                self.unreadable = true;
                self.list = std::mem::take(&mut self.early);
                self.revision += 1;
                return;
            }
        };
        let early = std::mem::take(&mut self.early);
        let added = !early.is_empty();
        merge(&mut list, early, true);
        self.list = list;
        self.dirty |= purged || added;
        self.revision += 1;
    }

    /// Collect the answers of the publications: a failed one leaves the log
    /// dirty again, to be tried again after a longer wait; a written one
    /// resets the wait.
    pub(super) fn poll_saves(&mut self) {
        let mut failed = false;
        let mut written = false;
        self.publishing.retain(|rx| match rx.try_recv() {
            Ok(Ok(())) => {
                written = true;
                false
            }
            Ok(Err(_)) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                failed = true;
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
        });
        if failed {
            self.save_failed();
        } else if written && self.publishing.is_empty() {
            self.retry = SAVE_EVERY;
        }
    }

    /// A save did not reach the disk: the log stays dirty, the next try waits
    /// longer.
    pub(super) fn save_failed(&mut self) {
        self.dirty = true;
        self.retry = (self.retry * 2).clamp(SAVE_EVERY, RETRY_MAX);
        self.saved_at = Some(std::time::Instant::now());
    }

    /// The publication `take_save` asked for was queued.
    pub(super) fn saving(&mut self, answer: Published) {
        self.publishing.push(answer);
    }

    fn entries(&mut self) -> &mut Vec<Conversation> {
        if self.reading.is_some() { &mut self.early } else { &mut self.list }
    }

    /// logConversation: a session opened or a message came. An entry of
    /// the same session takes the new time (and an offline mark when the
    /// session opened on offline messages); else it is added.
    pub fn log(&mut self, c: Conversation) {
        if !self.logging {
            return;
        }
        if merge(self.entries(), vec![c], true) {
            self.dirty = true;
            self.revision += 1;
        }
    }

    /// A conference's session id changed (the server's replaces the one we
    /// made, processSessionInitializedReply): its entry, also one still
    /// buffered while the file is read, and the open conversations' memory
    /// follow, merged with an entry of the new id if there is one.
    pub fn rekey(&mut self, from: Uuid, to: Uuid) {
        if from == to {
            return;
        }
        if self.known.remove(&from) {
            self.known.insert(to);
        }
        let mut changed = false;
        for list in [&mut self.list, &mut self.early] {
            if let Some(i) = list.iter().position(|c| c.session == from) {
                let mut c = list.remove(i);
                c.session = to;
                merge(list, vec![c], true);
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
            self.revision += 1;
        }
    }

    /// The entry of `session` was read (onIMFloaterShown): no unread
    /// offline mark any more.
    pub fn read(&mut self, session: Uuid) {
        let mut changed = false;
        for c in self.entries().iter_mut().filter(|c| c.session == session && c.offline) {
            c.offline = false;
            changed = true;
        }
        if changed {
            self.dirty = true;
            self.revision += 1;
        }
    }

    /// The name of the entry of `session`, once it is known
    /// (onAvatarNameCache → updateConversationName).
    pub fn rename(&mut self, session: Uuid, name: &str) {
        if name.is_empty() {
            return;
        }
        let mut changed = false;
        for c in self.entries().iter_mut().filter(|c| c.session == session && c.name != name) {
            c.name = name.to_owned();
            changed = true;
        }
        if changed {
            self.dirty = true;
            self.revision += 1;
        }
    }

    /// removeConversation: the entry of that session and time.
    pub fn remove(&mut self, session: Uuid, time: i64) {
        let list = self.entries();
        if let Some(i) = list.iter().position(|c| c.session == session && c.time == time) {
            list.remove(i);
            self.dirty = true;
            self.revision += 1;
            if let Some(r) = &mut self.removals {
                r.removed.push((session, time));
            }
        }
    }

    /// An import starts reading: note what is removed until its result.
    pub fn watch_removals(&mut self) {
        self.removals = Some(Removals::default());
    }

    /// The import's result is here: what was removed since it started.
    pub fn take_removals(&mut self) -> Removals {
        self.removals.take().unwrap_or_default()
    }

    /// onClearLogResponse: everything (the file is written empty).
    pub fn clear(&mut self) {
        if let Some(r) = &mut self.removals {
            r.cleared = true;
        }
        // a file still being read holds what is cleared: its answer is
        // dropped (the receiver goes, the worker's send finds nobody), and
        // the empty log is written over it
        self.reading = None;
        self.list.clear();
        self.early.clear();
        self.dirty = true;
        self.revision += 1;
    }

    /// Entries an import brought (the Legacy Viewer's own log), without
    /// their unread marks.
    pub fn imported(&mut self, theirs: Vec<Conversation>) {
        if merge(self.entries(), theirs, false) {
            self.dirty = true;
            self.revision += 1;
        }
    }

    /// The text to write now, if the log changed: at most every few
    /// seconds, or at once when `now_or_never`. Never while the file is
    /// being read (it would be lost).
    pub(super) fn take_save(&mut self, now_or_never: bool) -> Option<String> {
        if !self.dirty || self.reading.is_some() || self.unreadable {
            return None;
        }
        if !now_or_never && self.saved_at.is_some_and(|t| t.elapsed() < self.retry.max(SAVE_EVERY)) {
            return None;
        }
        // clean only until the worker answers: a failure sets it again
        self.dirty = false;
        self.saved_at = Some(std::time::Instant::now());
        Some(format_file(&self.list))
    }

    /// The demo's entries, without a file.
    pub fn demo(&mut self, list: Vec<Conversation>) {
        *self = Log {
            list,
            logging: true,
            revision: self.revision + 1,
            ..Default::default()
        };
    }
}

/// The order of the window (CallLogSortOrder: 0 by name, 1 by date).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Name,
    Date,
}

/// How the window shows the list.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    pub sort: Sort,
    /// SortFriendsFirst.
    pub friends_first: bool,
    /// ShowBlockedConvHistory.
    pub show_blocked: bool,
    /// The filter as typed (its leading spaces are dropped, onFilterEdit).
    pub filter: &'a str,
}

/// The entries the window shows, in its order, as indexes into `list`
/// (rebuildList then sort): those whose name or time contain the filter
/// (any case), without those of blocked residents unless asked; friends
/// first when asked, then by name (A to Z) or by date (newest first).
/// `names` are the names the window shows, one per entry of `list`.
pub fn shown(list: &[Conversation], names: &[String], view: View, friends: &HashSet<Uuid>, blocked: &HashSet<Uuid>) -> Vec<usize> {
    debug_assert_eq!(list.len(), names.len());
    let filter = view.filter.trim_start().to_uppercase();
    let upper: Vec<String> = names.iter().map(|n| n.to_uppercase()).collect();
    let mut out: Vec<usize> = (0..list.len())
        .filter(|&i| {
            let c = &list[i];
            let found = filter.is_empty() || upper[i].contains(&filter) || timestamp(c.time).to_uppercase().contains(&filter);
            found && (view.show_blocked || !blocked.contains(&c.participant))
        })
        .collect();
    out.sort_by(|&a, &b| {
        let (x, y) = (&list[a], &list[b]);
        let friends = if view.friends_first {
            friends.contains(&y.participant).cmp(&friends.contains(&x.participant))
        } else {
            std::cmp::Ordering::Equal
        };
        friends.then_with(|| match view.sort {
            Sort::Name => upper[a].cmp(&upper[b]),
            Sort::Date => y.time.cmp(&x.time),
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const JANE: Uuid = Uuid::from_u128(0x7e4ec5be_783f_49f5_71d2_16c58c64c145);
    const SESSION: Uuid = Uuid::from_u128(0x4ec62a74_c246_0d25_2af6_846beac2aa55);

    fn entry(time: i64, kind: Kind, name: &str, session: u128) -> Conversation {
        Conversation {
            time,
            kind,
            name: name.into(),
            history_file: name.into(),
            session: Uuid::from_u128(session),
            participant: Uuid::from_u128(session + 1000),
            offline: false,
        }
    }

    /// `shown` with the names the entries carry.
    fn show(list: &[Conversation], view: View, friends: &HashSet<Uuid>, blocked: &HashSet<Uuid>) -> Vec<usize> {
        let names: Vec<String> = list.iter().map(|c| c.name.clone()).collect();
        shown(list, &names, view, friends, blocked)
    }

    #[test]
    fn the_filter_and_the_sort_use_the_names_the_window_shows() {
        // an RLVa privacy name replaces the logged one in the filter too
        let list = vec![entry(1, Kind::Private, "Jane Doe", 1), entry(2, Kind::Private, "Bob", 2)];
        let shown_names = vec!["Anonymous 7".to_owned(), "Bob".to_owned()];
        let view = |filter| View {
            sort: Sort::Name,
            friends_first: false,
            show_blocked: true,
            filter,
        };
        let none = HashSet::new();
        assert!(shown(&list, &shown_names, view("jane"), &none, &none).is_empty());
        assert_eq!(shown(&list, &shown_names, view("anonymous"), &none, &none), [0]);
        assert_eq!(shown(&list, &shown_names, view(""), &none, &none), [0, 1]);
    }

    #[test]
    fn reads_the_legacy_viewers_lines() {
        // the two examples of LLConversationLog::saveToFile, with valid ids
        let p2p = parse_line(
            "[1343221177] 0 1 0 John%20Doe| 7e4ec5be-783f-49f5-71d2-16c58c64c145 4ec62a74-c246-0d25-2af6-846beac2aa55 john.doe|",
        )
        .unwrap();
        assert_eq!(p2p.time, 1343221177);
        assert_eq!(p2p.kind, Kind::Private);
        assert_eq!(p2p.name, "John Doe");
        assert_eq!(p2p.participant, JANE);
        assert_eq!(p2p.session, SESSION);
        assert_eq!(p2p.history_file, "john.doe");
        assert!(!p2p.offline);
        let adhoc = parse_line(
            "[1343222639] 2 0 1 Ad-hoc Conference| c3a67c89-c479-4c97-b21d-32869bcfe8ac 68f1c33e-4135-3e3e-a897-8c9b23115c09 Ad-hoc Conference hash597394a0-9982-766d-27b8-c75560213b9a|\r\n",
        )
        .unwrap();
        assert_eq!(adhoc.kind, Kind::Conference);
        assert_eq!(adhoc.name, "Ad-hoc Conference");
        assert_eq!(adhoc.history_file, "Ad-hoc Conference hash597394a0-9982-766d-27b8-c75560213b9a");
        assert!(adhoc.offline);
        // a group, with an escaped name and file
        let group = parse_line(
            "[1700000000] 1 0 0 Les%20Loups%20%28fr%29| 00000000-0000-0000-0000-000000000001 00000000-0000-0000-0000-000000000001 Les%20Loups%20%28fr%29%20%28group%29|",
        )
        .unwrap();
        assert_eq!(group.name, "Les Loups (fr)");
        assert_eq!(group.history_file, "Les Loups (fr) (group)");
        // not entries
        for bad in [
            "",
            "garbage",
            "[x] 0 0 0 a| b c d|",
            "[1] 0 0 0 Jane| not-an-id 4ec62a74-c246-0d25-2af6-846beac2aa55 j|",
        ] {
            assert_eq!(parse_line(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn writes_what_it_reads_and_what_the_legacy_viewer_writes() {
        let c = Conversation {
            time: 1343221177,
            kind: Kind::Private,
            name: "Jane Doe (jane.doe)".into(),
            history_file: "Jane Doe".into(),
            session: SESSION,
            participant: JANE,
            offline: true,
        };
        let line = format_line(&c);
        assert_eq!(
            line,
            format!(
                "[1343221177] 0 0 1 Jane%20Doe%20%28jane.doe%29| 7e4ec5be-783f-49f5-71d2-16c58c64c145 4ec62a74-c246-0d25-2af6-846beac2aa55 Jane%20Doe|{}",
                super::super::store::EOL
            )
        );
        assert_eq!(parse_line(&line), Some(c.clone()));
        // names of every script come back as they were
        for name in ["Jürgen Ölmann", "山田 太郎", "100% | pipes", "a%zz"] {
            let c = Conversation {
                name: name.into(),
                history_file: name.into(),
                ..c.clone()
            };
            assert_eq!(parse_line(&format_line(&c)), Some(c));
        }
        // a conference keeps its names plain, without a bar
        let conf = entry(5, Kind::Conference, "Ad-hoc Conference", 9);
        let back = parse_line(&format_line(&conf)).unwrap();
        assert_eq!(back, conf);
    }

    #[test]
    fn escape_is_lluris() {
        assert_eq!(escape("Jane Doe-._~"), "Jane%20Doe-._~");
        assert_eq!(escape("é"), "%C3%A9");
        assert_eq!(unescape("%C3%A9%2"), "é%2");
        assert_eq!(unescape("100%"), "100%");
    }

    #[test]
    fn the_old_entries_are_dropped_at_login() {
        let now = 1_800_000_000;
        let day = 86_400;
        let text = format_file(&[
            entry(now - day, Kind::Private, "recent", 1),
            entry(now - (LIFETIME_DAYS + 1) * day, Kind::Group, "old", 2),
            entry(now - LIFETIME_DAYS * day, Kind::Private, "just in time", 3),
        ]);
        let (list, purged) = parse_file(text.as_bytes(), now);
        assert!(purged);
        assert_eq!(list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["recent", "just in time"]);
        let (_, purged) = parse_file(format_file(&list).as_bytes(), now);
        assert!(!purged);
    }

    #[test]
    fn the_timestamp_is_second_life_time() {
        // 2026-10-12 21:05 UTC is 14:05 in Pacific daylight time
        assert_eq!(timestamp(1_791_839_100), "10/12/2026 14:05");
        // and January is standard time (UTC-8)
        assert_eq!(timestamp(1_767_225_600), "12/31/2025 16:00");
    }

    #[test]
    fn sorts_by_name_or_date_with_friends_first() {
        let list = vec![
            entry(300, Kind::Private, "bob", 1),
            entry(100, Kind::Private, "Alice", 2),
            entry(200, Kind::Group, "Carol's group", 3),
            entry(400, Kind::Private, "dave", 4),
        ];
        let friend = list[3].participant;
        let none = HashSet::new();
        let names = |order: Vec<usize>| order.into_iter().map(|i| list[i].name.clone()).collect::<Vec<_>>();
        let view = View {
            sort: Sort::Name,
            friends_first: false,
            show_blocked: false,
            filter: "",
        };
        assert_eq!(names(show(&list, view, &none, &none)), ["Alice", "bob", "Carol's group", "dave"]);
        let by_date = View { sort: Sort::Date, ..view };
        assert_eq!(names(show(&list, by_date, &none, &none)), ["dave", "bob", "Carol's group", "Alice"]);
        let friends = View {
            friends_first: true,
            ..view
        };
        assert_eq!(
            names(show(&list, friends, &HashSet::from([friend]), &none)),
            ["dave", "Alice", "bob", "Carol's group"]
        );
    }

    #[test]
    fn filters_names_and_times_and_hides_the_blocked() {
        let list = vec![
            entry(1_791_839_100, Kind::Private, "Jane Doe", 1),
            entry(1_767_225_600, Kind::Private, "John Smith", 2),
        ];
        let none = HashSet::new();
        let view = |filter| View {
            sort: Sort::Date,
            friends_first: false,
            show_blocked: false,
            filter,
        };
        assert_eq!(show(&list, view("  jane"), &none, &none), [0]);
        assert_eq!(show(&list, view("12/31"), &none, &none), [1]);
        assert_eq!(show(&list, view("nobody"), &none, &none).len(), 0);
        let blocked: HashSet<Uuid> = [list[1].participant].into();
        assert_eq!(show(&list, view(""), &none, &blocked), [0]);
        let all = View {
            show_blocked: true,
            ..view("")
        };
        assert_eq!(show(&list, all, &none, &blocked), [0, 1]);
    }

    #[test]
    fn merging_keeps_one_entry_per_session_and_the_newest_time() {
        let mut list = vec![entry(100, Kind::Private, "Jane", 1)];
        let mut newer = entry(200, Kind::Private, "Jane Doe", 1);
        newer.offline = true;
        assert!(merge(&mut list, vec![newer.clone(), entry(50, Kind::Group, "Group", 2)], true));
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].time, list[0].name.as_str(), list[0].offline), (200, "Jane Doe", true));
        // an older one changes nothing
        assert!(!merge(&mut list, vec![entry(150, Kind::Private, "Old", 1)], true));
        // an import never brings an unread mark
        let mut theirs = entry(10, Kind::Private, "Other", 3);
        theirs.offline = true;
        assert!(merge(&mut list, vec![theirs], false));
        assert!(!list[2].offline);
        // the same again: nothing new
        assert!(!merge(&mut list, vec![newer], true));
    }

    #[test]
    fn a_private_session_mixes_both_ids() {
        let a = Uuid::from_u128(0xF0);
        let b = Uuid::from_u128(0x0F);
        assert_eq!(private_session(a, b), Uuid::from_u128(0xFF));
        assert_eq!(private_session(a, b), private_session(b, a));
    }

    fn logging() -> Log {
        Log {
            logging: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_save_stays_dirty_until_the_worker_acknowledges_it() {
        let mut log = logging();
        log.log(entry(10, Kind::Private, "Jane", 1));
        assert!(log.take_save(true).is_some());
        // asked and not answered yet: nothing more to write
        assert!(log.take_save(true).is_none());
        // the worker could not write it: the next try writes it again
        let (done, answer) = crossbeam_channel::bounded(1);
        done.send(Err(std::io::Error::other("disk full"))).unwrap();
        log.saving(answer);
        log.poll_saves();
        assert!(log.take_save(true).is_some());
        // a job the queue dropped closes its channel: the same
        let (done, answer) = crossbeam_channel::bounded::<std::io::Result<()>>(1);
        drop(done);
        log.saving(answer);
        log.poll_saves();
        assert!(log.take_save(true).is_some());
        // an acknowledged one is done, and a failed save is not retried at once
        let (done, answer) = crossbeam_channel::bounded(1);
        done.send(Ok(())).unwrap();
        log.saving(answer);
        log.poll_saves();
        assert!(log.take_save(true).is_none());
        log.save_failed();
        assert!(log.take_save(false).is_none(), "backs off");
        assert!(log.take_save(true).is_some(), "but a logout tries at once");
    }

    #[test]
    fn the_entries_logged_during_the_read_are_merged_before_the_last_save() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let mut log = Log::default();
        log.start(Some(rx));
        log.log(entry(30, Kind::Private, "Session", 2));
        assert!(log.take_save(true).is_none(), "never over a file not read yet");
        let file = format_file(&[entry(10, Kind::Private, "Before", 1)]);
        tx.send(Ok(Some(file.into_bytes()))).unwrap();
        log.finish_read(40);
        let text = log.take_save(true).expect("the merged log is written");
        assert_eq!(parse_file(text.as_bytes(), 40).0.len(), 2);
    }

    #[test]
    fn a_dropped_read_job_ends_the_wait_and_keeps_the_session_entries() {
        let (tx, rx) = crossbeam_channel::bounded::<std::io::Result<Option<Vec<u8>>>>(1);
        let mut log = Log::default();
        log.start(Some(rx));
        log.log(entry(30, Kind::Private, "Session", 2));
        drop(tx);
        log.poll_read(40);
        assert!(!log.is_reading());
        assert_eq!(log.list.len(), 1);
        assert!(log.take_save(true).is_none(), "never over a file that could not be read");
    }

    #[test]
    fn what_the_user_removes_during_an_import_does_not_come_back_with_it() {
        let mut log = logging();
        log.log(entry(10, Kind::Private, "Jane", 1));
        log.log(entry(20, Kind::Private, "Bob", 2));
        let theirs = || {
            vec![
                entry(10, Kind::Private, "Jane", 1),
                entry(20, Kind::Private, "Bob", 2),
                entry(30, Kind::Group, "New", 3),
            ]
        };
        // a removal: only that entry is held back
        log.watch_removals();
        log.remove(Uuid::from_u128(1), 10);
        let mut incoming = theirs();
        log.take_removals().filter(&mut incoming);
        assert_eq!(incoming.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Bob", "New"]);
        // the log cleared: nothing comes back
        log.watch_removals();
        log.clear();
        let mut incoming = theirs();
        log.take_removals().filter(&mut incoming);
        assert!(incoming.is_empty());
        // no import under way: nothing is held back
        let mut incoming = theirs();
        log.take_removals().filter(&mut incoming);
        assert_eq!(incoming.len(), 3);
    }

    #[test]
    fn a_conference_whose_session_id_changes_keeps_one_entry() {
        let mut log = logging();
        let (temp, server) = (Uuid::from_u128(1), Uuid::from_u128(2));
        log.log(entry(10, Kind::Conference, "Conference", 1));
        log.known.insert(temp);
        log.rekey(temp, server);
        assert_eq!(log.list.len(), 1);
        assert_eq!(log.list[0].session, server);
        assert!(log.known.contains(&server) && !log.known.contains(&temp));
        // the poll logged the new id first: the two entries become one
        log.log(entry(20, Kind::Conference, "Conference", 3));
        log.list[1].session = temp;
        log.rekey(temp, server);
        assert_eq!(log.list.len(), 1);
        assert_eq!(log.list[0].time, 20);
        // an entry buffered while the file is read follows too
        let (_tx, rx) = crossbeam_channel::bounded(1);
        let mut log = Log::default();
        log.start(Some(rx));
        log.log(entry(10, Kind::Conference, "Conference", 1));
        log.rekey(temp, server);
        assert_eq!(log.early[0].session, server);
    }

    #[test]
    fn clearing_while_the_file_is_still_being_read_drops_the_old_list() {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let mut log = Log::default();
        log.start(Some(rx));
        log.log(entry(100, Kind::Private, "Session", 1));
        log.clear();
        assert!(!log.is_reading(), "the read is no longer waited for");
        // the worker answers anyway, with the old file
        let old = format_file(&[entry(50, Kind::Private, "Old", 2)]);
        let _ = tx.send(Ok(Some(old.into_bytes())));
        log.poll_read(1_000);
        assert!(log.list.is_empty(), "the old list does not come back");
        // the empty log goes to the disk at once, over the old file
        let saved = log.take_save(true).expect("written");
        assert_eq!(parse_file(saved.as_bytes(), 1_000).0.len(), 0);
        // what is logged afterwards is kept
        log.log(entry(200, Kind::Private, "New", 3));
        assert_eq!(log.list.len(), 1);
    }
}
