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
    /// Changed since the last save.
    dirty: bool,
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
        let Ok(read) = rx.try_recv() else { return };
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
        }
    }

    /// onClearLogResponse: everything (the file is written empty).
    pub fn clear(&mut self) {
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
        if !now_or_never && self.saved_at.is_some_and(|t| t.elapsed() < SAVE_EVERY) {
            return None;
        }
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
pub fn shown(list: &[Conversation], view: View, is_friend: impl Fn(&Uuid) -> bool, blocked: &HashSet<Uuid>) -> Vec<usize> {
    let filter = view.filter.trim_start().to_uppercase();
    let mut out: Vec<usize> = (0..list.len())
        .filter(|&i| {
            let c = &list[i];
            let found = filter.is_empty() || c.name.to_uppercase().contains(&filter) || timestamp(c.time).to_uppercase().contains(&filter);
            found && (view.show_blocked || !blocked.contains(&c.participant))
        })
        .collect();
    let names: Vec<String> = list.iter().map(|c| c.name.to_uppercase()).collect();
    out.sort_by(|&a, &b| {
        let (x, y) = (&list[a], &list[b]);
        let friends = if view.friends_first {
            is_friend(&y.participant).cmp(&is_friend(&x.participant))
        } else {
            std::cmp::Ordering::Equal
        };
        friends.then_with(|| match view.sort {
            Sort::Name => names[a].cmp(&names[b]),
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
        assert_eq!(
            names(shown(&list, view, |_| false, &none)),
            ["Alice", "bob", "Carol's group", "dave"]
        );
        let by_date = View { sort: Sort::Date, ..view };
        assert_eq!(
            names(shown(&list, by_date, |_| false, &none)),
            ["dave", "bob", "Carol's group", "Alice"]
        );
        let friends = View {
            friends_first: true,
            ..view
        };
        assert_eq!(
            names(shown(&list, friends, |id| *id == friend, &none)),
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
        assert_eq!(shown(&list, view("  jane"), |_| false, &none), [0]);
        assert_eq!(shown(&list, view("12/31"), |_| false, &none), [1]);
        assert_eq!(shown(&list, view("nobody"), |_| false, &none).len(), 0);
        let blocked: HashSet<Uuid> = [list[1].participant].into();
        assert_eq!(shown(&list, view(""), |_| false, &blocked), [0]);
        let all = View {
            show_blocked: true,
            ..view("")
        };
        assert_eq!(shown(&list, all, |_| false, &blocked), [0, 1]);
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
}
