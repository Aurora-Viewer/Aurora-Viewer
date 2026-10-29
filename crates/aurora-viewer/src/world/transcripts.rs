//! The conversations and their transcripts: what is written when a line
//! enters a conversation, and the history put back when it opens, after
//! Firestorm (LLIMModel::addMessage / logToFile, LLIMSession::loadHistory /
//! addMessagesFromHistoryCache / addMessagesFromServerHistory in
//! indra/newview/llimview.cpp, FSFloaterNearbyChat::addMessage / loadHistory
//! in fsfloaternearbychat.cpp; originally LGPL 2.1).
//!
//! - a conversation's file is named when its first line arrives (or when it
//!   opens), from the resident's legacy name, the group's name or the
//!   conference's; while a resident's name is unknown its lines wait;
//! - the end of the file is asked for before the first line is written, so
//!   the history never contains the line that opened the conversation;
//! - history lines carry `Origin::Log` (greyed by the view); their sender is
//!   the avatar whose cached user name is the one written (findIdByName on
//!   buildLegacyName), or the other resident of a private conversation, so
//!   that their picture shows as on new lines;
//! - the chat server's recent messages of a group (`Origin::ServerLog`) go
//!   between the file's history and the new lines, without those the file
//!   already has.

use super::chat_log::{Entry, Loaded, format, names};
use super::{ChatKind, ChatLine, MAX_CHAT, Origin, World};
use aurora_net::{ChatType, HistoryLine};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Lines kept for a conversation whose file name is not known yet.
const MAX_PENDING: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryState {
    #[default]
    NotAsked,
    Asked,
    /// Read and shown, or not to be shown.
    Done,
}

/// Transcript state of one conversation.
#[derive(Debug, Clone, Default)]
pub struct SessionLog {
    /// Its file, without folder and extension (`chat_log::names`).
    stem: Option<String>,
    history: HistoryState,
    /// Lines waiting for the file name: when, sender and text as written.
    pending: Vec<(SystemTime, String, String)>,
    /// The chat server's history, held until the end of the file was read.
    server_history: Option<Vec<HistoryLine>>,
}

impl SessionLog {
    pub fn stem(&self) -> Option<&str> {
        self.stem.as_deref()
    }
}

/// What a view needs to follow a list of lines without reading them all
/// every frame: when it changed, and a number for each line that stays the
/// same while lines are added at either end or dropped from the front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineClock {
    /// Number of the first line; line `i` has number `first + i`.
    pub first: u64,
    /// Changes whenever the lines do (unique in the whole run).
    pub revision: u64,
    /// Changes when the numbers no longer name the same lines (a new list,
    /// lines inserted in the middle).
    pub token: u64,
}

fn tick() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for LineClock {
    fn default() -> Self {
        LineClock {
            // room below for the history put before the first line
            first: 1 << 40,
            revision: tick(),
            token: tick(),
        }
    }
}

impl LineClock {
    pub fn pushed_back(&mut self) {
        self.revision = tick();
    }

    pub fn popped_front(&mut self) {
        self.first += 1;
        self.revision = tick();
    }

    pub fn pushed_front(&mut self, count: usize) {
        self.first -= count as u64;
        self.revision = tick();
    }

    /// A line changed in place (its translation): the views lay it out again.
    pub fn touched(&mut self) {
        self.revision = tick();
    }

    /// The lines were replaced or shuffled.
    pub fn renumbered(&mut self) {
        self.revision = tick();
        self.token = tick();
    }
}

/// Drop the oldest lines over the bound.
fn trim(lines: &mut std::collections::VecDeque<ChatLine>, clock: &mut LineClock) {
    while lines.len() > MAX_CHAT {
        lines.pop_front();
        clock.popped_front();
    }
}

/// The minute of a moment (transcript timestamps have no seconds).
fn minute(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() / 60).unwrap_or(0)
}

impl World {
    /// The transcript file of a conversation, from what is known now.
    fn transcript_stem(&self, session: &Uuid) -> Option<String> {
        let group = self.groups.group(session);
        let info = self.groups.sessions.get(session);
        if group.is_some() || info.is_some_and(|i| i.group) {
            let name = group.map(|g| g.name.as_str()).or(info.map(|i| i.name.as_str())).unwrap_or("");
            return (!name.is_empty()).then(|| names::group(name));
        }
        if info.is_some() {
            // a conference someone invited us to: dated when it started
            let name = self.session_title(session);
            return Some(names::conference_invited(&name, SystemTime::now(), session));
        }
        // LLAvatarName::getUserName, without " Resident" (UseLegacyIMLogNames)
        let legacy = match self.social.avatar_names.raw(session) {
            Some(n) if !n.legacy_first.is_empty() => {
                if n.legacy_last.is_empty() || n.legacy_last == "Resident" {
                    n.legacy_first.clone()
                } else {
                    format!("{} {}", n.legacy_first, n.legacy_last)
                }
            }
            // the name an instant message came with
            _ => names::resident(self.social.names.get(session)?),
        };
        (!legacy.trim().is_empty()).then_some(legacy)
    }

    /// Title and transcript file of a conversation, for the History window:
    /// the nearby chat (None), an open conversation, or a resident or one
    /// of our groups without one. The file is the one its history is read
    /// from; None when the account keeps no transcripts or the name the
    /// file is made of is unknown.
    pub fn transcript_of(&self, conversation: Option<Uuid>) -> (String, Option<std::path::PathBuf>) {
        let Some(id) = conversation else {
            return (
                aurora_i18n::tr!("common-nearby-chat").to_owned(),
                self.chat_log.history_file_of(names::NEARBY),
            );
        };
        let title = match self.groups.group(&id) {
            Some(g) => g.name.clone(),
            None => self.session_title(&id),
        };
        let open = self
            .social
            .ims
            .iter()
            .find(|s| s.other == id)
            .and_then(|s| s.log.stem().map(str::to_owned));
        let stem = open.or_else(|| {
            // a conference has no file before it starts
            (!self.groups.sessions.contains_key(&id) || self.groups.group(&id).is_some())
                .then(|| self.transcript_stem(&id))
                .flatten()
        });
        (title, stem.and_then(|s| self.chat_log.history_file_of(&s)))
    }

    /// Name the conversation's file if it can be, ask for its history
    /// (once, before anything is written) and write the lines that waited.
    fn ensure_transcript(&mut self, session: Uuid) {
        let Some(i) = self.social.ims.iter().position(|s| s.other == session) else {
            return;
        };
        if self.social.ims[i].log.stem.is_none() {
            self.social.ims[i].log.stem = self.transcript_stem(&session);
        }
        let log = &mut self.social.ims[i].log;
        let Some(stem) = log.stem.clone() else {
            return;
        };
        if log.history == HistoryState::NotAsked {
            log.history = if self.chat_log.request_history(session, &stem) {
                HistoryState::Asked
            } else {
                HistoryState::Done
            };
        }
        for (time, from, text) in std::mem::take(&mut log.pending) {
            self.chat_log.append(&stem, time, &from, &text);
        }
    }

    /// Sender and text of a line as its transcript has them
    /// (LLIMModel::logToFile: the complete name of an avatar).
    fn transcript_text(&self, line: &ChatLine) -> (String, String) {
        let names = &self.social.avatar_names;
        let from = match line.kind {
            ChatKind::System => String::new(),
            ChatKind::Own(_) => names.complete(&self.agent_id).unwrap_or_else(|| line.from.clone()),
            ChatKind::Local(_) | ChatKind::Im if !line.source.is_nil() => names.complete(&line.source).unwrap_or_else(|| line.from.clone()),
            _ => line.from.clone(),
        };
        // the verbs Firestorm puts before what is shouted or whispered
        let verb = match line.kind.volume() {
            Some(ChatType::Shout) => "shouts: ",
            Some(ChatType::Whisper) => "whispers: ",
            _ => "",
        };
        let text = format::encode_mentions(&line.text, |id| self.social.name_of(&id));
        (
            self.rlva_filter_text(&from).into_owned(),
            self.rlva_filter_text(&format!("{verb}{text}")).into_owned(),
        )
    }

    /// Write a live line to its conversation's transcript, when instant
    /// messages are logged.
    fn log_line(&mut self, session: Uuid, line: &ChatLine) {
        if line.origin == Origin::Live && self.chat_log.cfg.log_im && self.chat_log.identity().is_some() {
            let (from, text) = self.transcript_text(line);
            let s = self.session_lines(session);
            match s.log.stem.clone() {
                Some(stem) => self.chat_log.append(&stem, line.time, &from, &text),
                None if s.log.pending.len() < MAX_PENDING => s.log.pending.push((line.time, from, text)),
                None => {}
            }
        }
    }

    /// A line shows its translation: `original` at the end of its text
    /// becomes `translated` (an offline message keeps its "(Saved …)"), and
    /// the transcript gets the translation as a line of its own, after the
    /// original it already holds.
    pub fn replace_line(&mut self, key: &crate::translate::LineKey, original: &str, translated: &str) {
        let s = self.session_lines(key.session);
        let Some(line) = s.lines.iter_mut().rev().find(|l| l.time == key.time) else {
            return;
        };
        let Some(head) = line.text.strip_suffix(original) else {
            return;
        };
        line.text = format!("{head}{translated}");
        s.clock.touched();
        let line = s.lines.iter().rev().find(|l| l.time == key.time).cloned();
        let Some(mut logged) = line else {
            return;
        };
        logged.text = format!("[translation] {translated}");
        self.log_line(key.session, &logged);
    }

    /// Our held message taken back ("Edit" on an untranslated message): its
    /// line leaves the conversation.
    pub fn remove_line(&mut self, key: &crate::translate::LineKey) {
        let s = self.session_lines(key.session);
        if let Some(i) = s.lines.iter().rposition(|l| l.time == key.time) {
            s.lines.remove(i);
            s.clock.renumbered();
        }
    }

    /// The translation marks of a conversation's lines changed.
    pub fn touch_conversation(&mut self, session: Uuid) {
        if let Some(s) = self.social.ims.iter_mut().find(|s| s.other == session) {
            s.clock.touched();
        }
    }

    /// A line enters a conversation: written to its transcript (new lines,
    /// when instant messages are logged), kept in memory within `MAX_CHAT`.
    pub(super) fn conversation_push(&mut self, session: Uuid, line: ChatLine, unread: bool) {
        self.session_lines(session);
        self.ensure_transcript(session);
        self.log_line(session, &line);
        let s = self.session_lines(session);
        s.lines.push_back(line);
        s.clock.pushed_back();
        if unread {
            s.unread += 1;
        }
        trim(&mut s.lines, &mut s.clock);
    }

    /// A line for the conversation with an avatar that has none open: only
    /// written to its transcript, as LLHandlerUtil::logToIM does without a
    /// session (llnotificationhandlerutil.cpp). Lost while the avatar's name,
    /// which names the file, is unknown.
    pub(super) fn transcript_only(&mut self, other: Uuid, line: &ChatLine) {
        if !self.chat_log.cfg.log_im || self.chat_log.identity().is_none() {
            return;
        }
        let Some(stem) = self.transcript_stem(&other) else {
            return;
        };
        let (from, text) = self.transcript_text(line);
        self.chat_log.append(&stem, line.time, &from, &text);
    }

    /// A card in a conversation (a teleport invitation): kept in memory and
    /// counted as unread, like LLHandlerUtil::addNotifPanelToIM, but never
    /// written to the transcript (its text is logged by itself).
    pub(super) fn conversation_card(&mut self, session: Uuid, line: ChatLine) {
        self.session_lines(session);
        self.ensure_transcript(session);
        let s = self.session_lines(session);
        s.lines.push_back(line);
        s.clock.pushed_back();
        s.unread += 1;
        trim(&mut s.lines, &mut s.clock);
    }

    /// A line enters the nearby chat (same bound).
    pub(super) fn nearby_push(&mut self, line: ChatLine) {
        self.log_nearby(&line);
        self.chat.push_back(line);
        self.chat_clock.pushed_back();
        trim(&mut self.chat, &mut self.chat_clock);
    }

    /// Empty the nearby chat (the "clear" command of the chat bar).
    pub fn clear_nearby_chat(&mut self) {
        self.chat.clear();
        self.chat_clock.renumbered();
    }

    /// A line of the nearby chat, written when the nearby chat is logged
    /// (LogNearbyChat). Instant messages shown there are not
    /// (FSLogIMInChatHistory is off by default), those of objects are.
    pub(super) fn log_nearby(&mut self, line: &ChatLine) {
        if line.origin != Origin::Live || !self.chat_log.cfg.log_nearby || line.kind == ChatKind::Im || self.chat_log.identity().is_none() {
            return;
        }
        let (from, text) = self.transcript_text(line);
        self.chat_log.append(names::NEARBY, line.time, &from, &text);
    }

    /// Every frame: histories asked for, histories read.
    pub fn poll_transcripts(&mut self) {
        if self.chat_log.identity().is_none() {
            return;
        }
        if self.nearby_history == HistoryState::NotAsked {
            // FSFloaterNearbyChat::loadHistory at login; only when the
            // nearby chat is logged (a deviation: Firestorm shows an old
            // file for as long as it exists)
            let ask = self.chat_log.cfg.log_nearby && self.chat_log.request_history(Uuid::nil(), names::NEARBY);
            self.nearby_history = if ask { HistoryState::Asked } else { HistoryState::Done };
        }
        let waiting: Vec<Uuid> = self
            .social
            .ims
            .iter()
            .filter(|s| s.log.stem.is_none() || s.log.history == HistoryState::NotAsked)
            .map(|s| s.other)
            .collect();
        for session in waiting {
            self.ensure_transcript(session);
        }
        for loaded in self.chat_log.take_loaded() {
            self.apply_history(loaded);
        }
    }

    /// Our names as a transcript may have them.
    fn own_log_names(&self) -> Vec<String> {
        let mut out = vec!["You".to_owned(), self.own_name()];
        if let Some(n) = self.social.avatar_names.complete(&self.agent_id) {
            out.push(n);
        }
        if let Some(l) = &self.login {
            out.push(super::social::format_name(&l.first_name, &l.last_name));
            out.push(format!("{} {}", l.first_name, l.last_name));
        }
        out.retain(|n| !n.trim().is_empty());
        out
    }

    /// The lines of a transcript's end as conversation lines
    /// (addMessagesFromHistoryCache). A line without a date takes the time
    /// of the one before. `keep_all`: one line per entry, the empty ones
    /// too (the History window counts them). `peer`: the other resident of
    /// a private conversation, the sender of its lines whose name is not
    /// cached (a group's or the nearby chat's stay unknown then).
    pub fn history_lines(&self, entries: Vec<Entry>, nearby: bool, keep_all: bool, peer: Option<Uuid>) -> Vec<ChatLine> {
        let own = self.own_log_names();
        let names = &self.social.avatar_names;
        let trim = names.options.trim_resident;
        let mut last = UNIX_EPOCH;
        entries
            .into_iter()
            .filter(|e| keep_all || !e.text.is_empty())
            .map(|e| {
                last = e.time.unwrap_or(last);
                let kind = if e.is_system() || e.from.is_empty() {
                    ChatKind::System
                } else if own.contains(&e.from) {
                    ChatKind::OWN
                } else if nearby {
                    ChatKind::Local(ChatType::Normal)
                } else {
                    ChatKind::Im
                };
                // LLIMSession::addMessagesFromHistoryCache: the id by the
                // legacy name of what was written
                let source = match kind {
                    ChatKind::Im | ChatKind::Local(_) => names
                        .find_id_by_name(&super::names::build_legacy_name(&e.from, trim))
                        .or(peer)
                        .unwrap_or_default(),
                    _ => Uuid::nil(),
                };
                ChatLine {
                    time: last,
                    from: if kind == ChatKind::System { String::new() } else { e.from },
                    text: e.text,
                    kind,
                    source,
                    origin: Origin::Log,
                }
            })
            .collect()
    }

    fn apply_history(&mut self, loaded: Loaded) {
        let nearby = loaded.conversation.is_nil();
        // a private conversation is keyed by the other resident
        let peer = (!nearby && !self.is_group_session(&loaded.conversation)).then_some(loaded.conversation);
        let lines = self.history_lines(loaded.entries, nearby, false, peer);
        if nearby {
            self.nearby_history = HistoryState::Done;
            self.chat_clock.pushed_front(lines.len());
            for l in lines.into_iter().rev() {
                self.chat.push_front(l);
            }
            trim(&mut self.chat, &mut self.chat_clock);
            return;
        }
        let Some(s) = self.social.ims.iter_mut().find(|s| s.other == loaded.conversation) else {
            return;
        };
        if s.log.history != HistoryState::Asked {
            return;
        }
        s.log.history = HistoryState::Done;
        s.clock.pushed_front(lines.len());
        for l in lines.into_iter().rev() {
            s.lines.push_front(l);
        }
        trim(&mut s.lines, &mut s.clock);
        if let Some(history) = s.log.server_history.take() {
            self.merge_server_history(loaded.conversation, history);
        }
    }

    /// Recent messages from the chat server of a group, once the end of the
    /// transcript is known (LLIMSession::addMessagesFromServerHistory).
    pub(super) fn add_server_history(&mut self, session: Uuid, history: Vec<HistoryLine>) {
        if history.is_empty() {
            return;
        }
        self.session_lines(session);
        self.ensure_transcript(session);
        let s = self.session_lines(session);
        if s.log.history == HistoryState::Asked {
            s.log.server_history = Some(history);
            return;
        }
        self.merge_server_history(session, history);
    }

    fn merge_server_history(&mut self, session: Uuid, history: Vec<HistoryLine>) {
        let me = self.agent_id;
        let own = self.own_name();
        let Some(s) = self.social.ims.iter().find(|s| s.other == session) else {
            return;
        };
        // what the file's history ends with: older server lines are in it
        // or before it, and in its last minute the texts tell
        let last_minute = s.lines.iter().rev().find(|l| l.origin == Origin::Log).map(|l| minute(l.time));
        let last_texts: Vec<&str> = s
            .lines
            .iter()
            .filter(|l| l.origin == Origin::Log && Some(minute(l.time)) == last_minute)
            .map(|l| l.text.as_str())
            .collect();
        let known: std::collections::HashSet<(Uuid, &str)> = s
            .lines
            .iter()
            .filter(|l| l.origin != Origin::Log)
            .map(|l| (l.source, l.text.as_str()))
            .collect();
        let older: Vec<ChatLine> = history
            .into_iter()
            .filter(|h| !self.mutes.text_muted(&h.from_id, &h.from))
            .filter(|h| match last_minute {
                Some(m) => {
                    let hm = u64::from(h.time) / 60;
                    hm > m || (hm == m && !last_texts.contains(&h.message.as_str()))
                }
                None => true,
            })
            .filter(|h| !known.contains(&(if h.from_id == me { Uuid::nil() } else { h.from_id }, h.message.as_str())))
            .map(|h| {
                let mine = h.from_id == me;
                ChatLine {
                    time: UNIX_EPOCH + Duration::from_secs(u64::from(h.time)),
                    from: if mine { own.clone() } else { h.from },
                    text: h.message,
                    kind: if mine { ChatKind::OWN } else { ChatKind::Im },
                    source: if mine { Uuid::nil() } else { h.from_id },
                    origin: Origin::ServerLog,
                }
            })
            .collect();
        if older.is_empty() {
            return;
        }
        // a chat we opened ourselves: what the server brought is written to
        // the file (target_from and target_message empty in Firestorm)
        let to_log = self.groups.sessions.get(&session).is_some_and(|i| i.started_by_us) && self.chat_log.cfg.log_im;
        let written: Vec<(String, String)> = if to_log {
            older.iter().map(|l| self.transcript_text(l)).collect()
        } else {
            Vec::new()
        };
        let s = self.session_lines(session);
        if let Some(stem) = s.log.stem.clone() {
            for (from, text) in written {
                self.chat_log.append(&stem, SystemTime::now(), &from, &text);
            }
        }
        let s = self.session_lines(session);
        // after the file's history, before the new lines
        let at = s.lines.iter().position(|l| l.origin == Origin::Live).unwrap_or(s.lines.len());
        for (i, l) in older.into_iter().enumerate() {
            s.lines.insert(at + i, l);
        }
        s.clock.renumbered();
        trim(&mut s.lines, &mut s.clock);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use crate::world::chat_log;
    use aurora_net::InstantMessage;
    use std::path::PathBuf;
    use std::sync::Arc;

    const WAIT: Duration = Duration::from_secs(10);

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-transcripts-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn world(root: &std::path::Path, agent: Uuid) -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        w.chat_log = chat_log::ChatLog::in_folder(root.to_path_buf());
        w.chat_log.set_grid("agni".into());
        w.agent_id = agent;
        w.chat_log.login(agent);
        w
    }

    fn im(from: Uuid, name: &str, text: &str) -> InstantMessage {
        InstantMessage {
            from_agent_id: from,
            from_name: name.into(),
            to_agent_id: Uuid::from_u128(1),
            dialog: 0,
            session_id: from,
            message: text.into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        }
    }

    fn settle(w: &mut World) {
        assert!(w.chat_log.flush(WAIT));
        w.poll_transcripts();
    }

    /// The end of a test: its folder goes once the writer has nothing left
    /// to write in it (a line still on its way made the removal fail).
    fn clean(w: &World, root: &std::path::Path) {
        assert!(w.chat_log.flush(WAIT));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn file_texts(w: &World, stem: &str) -> Vec<(String, String)> {
        let path = w.chat_log.file_of(stem).unwrap();
        chat_log::store::read_tail(&path)
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.from, e.text))
            .collect()
    }

    #[test]
    fn instant_messages_are_written_and_come_back_as_history() {
        let root = temp();
        let me = Uuid::from_u128(1);
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, me);
        w.on_im(im(jane, "Jane Doe", "hello"));
        w.own_im(jane, "hi Jane");
        settle(&mut w);
        // first conversation: no history, both lines live
        let s = &w.social.ims[0];
        assert_eq!(s.log.stem(), Some("Jane Doe"));
        assert!(s.lines.iter().all(|l| l.origin == Origin::Live));
        assert_eq!(file_texts(&w, "Jane Doe").len(), 2);
        assert_eq!(file_texts(&w, "Jane Doe")[0], ("Jane Doe".to_owned(), "hello".to_owned()));
        // closed, then a new message opens it again: the two lines are
        // history, the new one is not, and it is not in the history
        w.leave_session(jane);
        assert!(w.social.ims.is_empty());
        w.on_im(im(jane, "Jane Doe", "are you there?"));
        settle(&mut w);
        let s = &w.social.ims[0];
        let shown: Vec<(&str, Origin)> = s.lines.iter().map(|l| (l.text.as_str(), l.origin)).collect();
        assert_eq!(
            shown,
            [("hello", Origin::Log), ("hi Jane", Origin::Log), ("are you there?", Origin::Live)]
        );
        // history lines are not counted as unread; Jane's is hers (the
        // other resident of the conversation), ours names nobody, as live
        let sources: Vec<Uuid> = s.lines.iter().take(2).map(|l| l.source).collect();
        assert_eq!(sources, [jane, Uuid::nil()]);
        assert_eq!(s.unread, 1);
        assert_eq!(file_texts(&w, "Jane Doe").len(), 3);
        clean(&w, &root);
    }

    #[test]
    fn logging_off_writes_nothing_and_history_off_shows_nothing() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, Uuid::from_u128(1));
        w.chat_log.cfg.log_im = false;
        w.on_im(im(jane, "Jane Doe", "not kept"));
        settle(&mut w);
        assert!(!w.chat_log.file_of("Jane Doe").unwrap().exists());
        w.chat_log.cfg.log_im = true;
        w.on_im(im(jane, "Jane Doe", "kept"));
        settle(&mut w);
        w.leave_session(jane);
        w.chat_log.cfg.show_history = false;
        w.on_im(im(jane, "Jane Doe", "again"));
        settle(&mut w);
        let shown: Vec<&str> = w.social.ims[0].lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(shown, ["again"]);
        clean(&w, &root);
    }

    #[test]
    fn the_nearby_chat_is_logged_only_when_asked() {
        let root = temp();
        let mut w = world(&root, Uuid::from_u128(1));
        w.system_message("not kept");
        settle(&mut w);
        assert!(!w.chat_log.file_of(names::NEARBY).unwrap().exists());
        w.chat_log.cfg.log_nearby = true;
        w.system_message("teleport completed");
        w.push_chat(ChatLine {
            time: SystemTime::now(),
            from: "Jane Doe".into(),
            text: "HELLO".into(),
            kind: ChatKind::Local(ChatType::Shout),
            source: Uuid::from_u128(2),
            origin: Origin::Live,
        });
        // ours, as the simulator echoes it, keeps its volume and its verb
        // (#1323)
        w.apply(aurora_net::NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Me Resident".into(),
            source_id: w.agent_id,
            owner_id: w.agent_id,
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: ChatType::Whisper,
            position: glam::Vec3::ZERO,
            message: "psst".into(),
        }));
        assert_eq!(w.chat.back().map(|l| l.kind), Some(ChatKind::Own(ChatType::Whisper)));
        settle(&mut w);
        assert_eq!(
            file_texts(&w, names::NEARBY),
            [
                (format::SYSTEM_FROM.to_owned(), "teleport completed".to_owned()),
                ("Jane Doe".to_owned(), "shouts: HELLO".to_owned()),
                ("Me Resident".to_owned(), "whispers: psst".to_owned())
            ]
        );
        clean(&w, &root);
    }

    #[test]
    fn another_account_sees_nothing_of_the_first() {
        let root = temp();
        let alice = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, alice);
        w.chat_log.cfg.log_nearby = true;
        w.on_im(im(jane, "Jane Doe", "for alice only"));
        w.system_message("alice's nearby line");
        settle(&mut w);
        // alice logs out, bob logs in with the same viewer
        w.reset();
        assert!(w.social.ims.is_empty());
        assert_eq!(w.chat_log.identity(), None);
        w.agent_id = bob;
        w.chat_log.login(bob);
        w.on_im(im(jane, "Jane Doe", "for bob only"));
        settle(&mut w);
        settle(&mut w);
        let shown: Vec<&str> = w.social.ims[0].lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(shown, ["for bob only"]);
        assert!(w.chat.iter().all(|l| l.origin == Origin::Live));
        assert_eq!(file_texts(&w, "Jane Doe"), [("Jane Doe".to_owned(), "for bob only".to_owned())]);
        // and alice finds hers, without bob's
        w.reset();
        w.agent_id = alice;
        w.chat_log.login(alice);
        w.on_im(im(jane, "Jane Doe", "back"));
        settle(&mut w);
        settle(&mut w);
        let shown: Vec<&str> = w.social.ims[0].lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(shown, ["for alice only", "back"]);
        let nearby: Vec<&str> = w.chat.iter().filter(|l| l.origin == Origin::Log).map(|l| l.text.as_str()).collect();
        assert_eq!(nearby, ["alice's nearby line"]);
        clean(&w, &root);
    }

    #[test]
    fn the_servers_history_skips_what_the_file_has() {
        let root = temp();
        let me = Uuid::from_u128(1);
        let group = Uuid::from_u128(0x6200);
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, me);
        w.groups.groups.push(aurora_net::GroupMembership {
            id: group,
            name: "Les Loups".into(),
            insignia: Uuid::nil(),
            powers: 0,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        });
        // a transcript from an earlier session
        let t0 = format::parse_stamp("2026/10/12 14:05").unwrap();
        let stem = names::group("Les Loups");
        w.chat_log.append(&stem, t0 - Duration::from_secs(3600), "Jane Doe", "old news");
        w.chat_log.append(&stem, t0, "Jane Doe", "same minute, in the file");
        assert!(w.chat_log.flush(WAIT));
        w.start_group_chat(group);
        let unix = |t: SystemTime| t.duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        let line = |time: u32, text: &str| HistoryLine {
            from_id: jane,
            from: "Jane Doe".into(),
            message: text.into(),
            time,
        };
        // the server's answer arrives before the file was read: it waits
        w.add_server_history(
            group,
            vec![
                line(unix(t0) - 3600, "old news"),
                line(unix(t0) - 60, "before the end of the file"),
                line(unix(t0) + 20, "same minute, in the file"),
                line(unix(t0) + 30, "same minute, new"),
                line(unix(t0) + 600, "later"),
            ],
        );
        assert!(w.social.ims[0].lines.is_empty());
        settle(&mut w);
        let shown: Vec<(&str, Origin)> = w.social.ims[0].lines.iter().map(|l| (l.text.as_str(), l.origin)).collect();
        assert_eq!(
            shown,
            [
                ("old news", Origin::Log),
                ("same minute, in the file", Origin::Log),
                ("same minute, new", Origin::ServerLog),
                ("later", Origin::ServerLog),
            ]
        );
        // opened by us: the server's lines are now in the file too
        assert!(w.chat_log.flush(WAIT));
        assert_eq!(file_texts(&w, &stem).len(), 4);
        clean(&w, &root);
    }

    #[test]
    fn a_message_kept_while_offline_says_when_it_was_sent() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, Uuid::from_u128(1));
        let sent = format::parse_stamp("2026/10/11 09:30").unwrap();
        let mut m = im(jane, "Jane Doe", "see you tomorrow");
        m.offline = true;
        m.timestamp = sent.duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        w.on_im(m);
        settle(&mut w);
        let line = &w.social.ims[0].lines[0];
        // "(Saved <date>) " then the text; shown and written as received now
        assert!(line.text.ends_with(") see you tomorrow"), "{}", line.text);
        assert!(line.text.starts_with('(') && line.text.contains("09:30") && line.text.contains("2026"));
        assert!(line.time > sent + Duration::from_secs(3600));
        assert_eq!(line.origin, Origin::Live);
        assert_eq!(w.social.ims[0].unread, 1);
        let written = file_texts(&w, "Jane Doe");
        assert_eq!(written, [("Jane Doe".to_owned(), line.text.clone())]);
        // a message that is not from storage has no such prefix
        w.on_im(im(jane, "Jane Doe", "hello"));
        assert_eq!(w.social.ims[0].lines[1].text, "hello");
        clean(&w, &root);
    }

    #[test]
    fn offline_messages_are_asked_for_once_when_the_session_is_ready() {
        use aurora_net::NetCommand;
        let root = temp();
        // the demo's avatar object is the agent's
        let me = crate::demo::DEMO_AGENT;
        let mut w = world(&root, me);
        let asked = |w: &mut World| {
            w.tick_social();
            w.groups
                .take_commands()
                .iter()
                .filter(|c| matches!(c, NetCommand::RequestOfflineMessages))
                .count()
        };
        // not before the arrival, the capabilities, the block list and our
        // avatar
        assert_eq!(asked(&mut w), 0);
        w.movement_complete = true;
        assert_eq!(asked(&mut w), 0);
        let handle = 0x0003_E800_0003_E800;
        w.main_region = Some(handle);
        w.apply(aurora_net::NetEvent::Capabilities {
            handle,
            caps: Arc::new([("ReadOfflineMsgs".to_owned(), "x".to_owned())].into_iter().collect()),
        });
        assert_eq!(asked(&mut w), 0);
        w.on_mute_list(aurora_net::MuteListSource::UseCache);
        assert!(w.mutes.is_loaded());
        assert_eq!(asked(&mut w), 0);
        w.apply(aurora_net::NetEvent::ObjectUpdates {
            handle,
            objects: vec![crate::demo::action_avatar(false)],
        });
        assert!(w.objects.index_of_uuid(&me).is_some());
        // once, and never again in this session
        assert_eq!(asked(&mut w), 1);
        assert_eq!(asked(&mut w), 0);
        // the capability failed: the legacy message is asked for
        w.apply(aurora_net::NetEvent::OfflineMessagesFailed);
        assert!(
            w.groups
                .take_commands()
                .iter()
                .any(|c| matches!(c, NetCommand::RequestOfflineMessagesLegacy))
        );
        clean(&w, &root);
    }

    #[test]
    fn a_conversation_we_open_shows_its_history_before_any_line() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, Uuid::from_u128(1));
        w.social.names.insert(jane, "Jane Doe".into());
        w.chat_log.append(
            "Jane Doe",
            format::parse_stamp("2026/10/11 09:30").unwrap(),
            "Jane Doe",
            "yesterday",
        );
        assert!(w.chat_log.flush(WAIT));
        // opened from the contacts: no line yet
        w.social.session_mut(jane);
        settle(&mut w);
        settle(&mut w);
        let shown: Vec<(&str, Origin)> = w.social.ims[0].lines.iter().map(|l| (l.text.as_str(), l.origin)).collect();
        assert_eq!(shown, [("yesterday", Origin::Log)]);
        assert_eq!(w.social.ims[0].unread, 0);
        // the transcript the History window opens for it, and for the
        // nearby chat (none was written)
        let (title, file) = w.transcript_of(Some(jane));
        assert_eq!(title, "Jane Doe");
        assert_eq!(file, w.chat_log.file_of("Jane Doe"));
        assert_eq!(w.transcript_of(None).1, None);
        // a resident never talked to has none
        w.social.names.insert(Uuid::from_u128(3), "Bob Roe".into());
        assert_eq!(w.transcript_of(Some(Uuid::from_u128(3))).1, None);
        clean(&w, &root);
    }

    #[test]
    fn lines_wait_for_the_name_of_their_file() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, Uuid::from_u128(1));
        // we write first, to someone whose name is not known yet
        w.own_im(jane, "hello?");
        settle(&mut w);
        assert_eq!(w.social.ims[0].log.stem(), None);
        w.social.names.insert(jane, "jane Resident".into());
        settle(&mut w);
        assert_eq!(w.social.ims[0].log.stem(), Some("jane"));
        settle(&mut w);
        assert_eq!(file_texts(&w, "jane").len(), 1);
        // read after the line was written? no: the history was asked first
        assert!(w.social.ims[0].lines.iter().all(|l| l.origin == Origin::Live));
        clean(&w, &root);
    }

    #[test]
    fn history_lines_name_their_sender_when_it_is_known() {
        let root = temp();
        let (me, jane, bob) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mut w = world(&root, me);
        w.social.avatar_names.on_legacy(bob, "Bob", "Smith");
        let entry = |from: &str, text: &str| Entry {
            stamp: String::new(),
            time: None,
            from: from.into(),
            text: text.into(),
        };
        let entries = || {
            vec![
                entry("Bob (bob.smith)", "from Bob"),
                entry("Alice Liddell", "from someone not cached"),
                entry(chat_log::format::SYSTEM_FROM, "a notice"),
            ]
        };
        // a group or the nearby chat: the cached name only
        let sources: Vec<Uuid> = w.history_lines(entries(), false, false, None).iter().map(|l| l.source).collect();
        assert_eq!(sources, [bob, Uuid::nil(), Uuid::nil()]);
        // a private conversation: its other resident when the name is unknown
        let sources: Vec<Uuid> = w
            .history_lines(entries(), false, false, Some(jane))
            .iter()
            .map(|l| l.source)
            .collect();
        assert_eq!(sources, [bob, jane, Uuid::nil()]);
        clean(&w, &root);
    }

    #[test]
    fn a_conversation_is_bounded() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let mut w = world(&root, Uuid::from_u128(1));
        w.chat_log.cfg.log_im = false;
        w.social.names.insert(jane, "Jane Doe".into());
        for i in 0..MAX_CHAT + 50 {
            w.own_im(jane, &format!("line {i}"));
        }
        let s = &w.social.ims[0];
        assert_eq!(s.lines.len(), MAX_CHAT);
        assert_eq!(s.lines[0].text, "line 50");
        clean(&w, &root);
    }

    #[test]
    fn a_teleport_offer_is_noted_with_the_avatar_not_in_the_nearby_chat() {
        let root = temp();
        let jane = Uuid::from_u128(2);
        let bob = Uuid::from_u128(3);
        let mut w = world(&root, Uuid::from_u128(1));
        let link = |id: Uuid| format!("secondlife:///app/agent/{id}/completename");
        // a conversation with Jane is open: the notice goes in it, not unread
        w.on_im(im(jane, "Jane Doe", "hello"));
        let unread = w.social.ims[0].unread;
        w.teleport_offer_sent(jane);
        let s = &w.social.ims[0];
        let notice = s.lines.back().unwrap();
        assert!(notice.kind == ChatKind::System);
        assert!(notice.text.ends_with(&link(jane)), "{}", notice.text);
        assert_eq!(s.unread, unread);
        // none with Bob: none opens, the notice is only in his transcript
        w.social.names.insert(bob, "Bob Builder".into());
        w.teleport_offer_sent(bob);
        assert_eq!(w.social.ims.len(), 1);
        assert!(w.chat.is_empty());
        settle(&mut w);
        let bob_file = file_texts(&w, "Bob Builder");
        assert_eq!(bob_file.len(), 1);
        assert_eq!(bob_file[0].0, format::SYSTEM_FROM);
        assert!(bob_file[0].1.ends_with(&link(bob)), "{}", bob_file[0].1);
        assert_eq!(file_texts(&w, "Jane Doe").len(), 2);
        clean(&w, &root);
    }

    #[test]
    fn a_teleport_offer_carries_where_we_stand_to_every_invitee() {
        let root = temp();
        let (jane, bob) = (Uuid::from_u128(2), Uuid::from_u128(3));
        let mut w = world(&root, Uuid::from_u128(1));
        w.agent.position = glam::Vec3::new(128.4, 64.6, 22.0);
        w.on_im(im(jane, "Jane Doe", "hello"));
        let cmd = w.offer_teleport(vec![jane, bob], "Join me");
        // send_lures: the invitation, then the SLURL of our position
        let aurora_net::NetCommand::OfferTeleport { to, message } = cmd else {
            panic!("not a teleport offer: {cmd:?}");
        };
        assert_eq!(to, [jane, bob]);
        let (text, slurl) = message.split_once("\r\n").unwrap();
        assert_eq!(text, "Join me");
        assert!(slurl.starts_with("http://maps.secondlife.com/secondlife/"), "{slurl}");
        assert!(slurl.ends_with("/128/65/22"), "{slurl}");
        // the notice of each invitee: in Jane's conversation, none opened for Bob
        assert!(w.social.ims[0].lines.back().unwrap().kind == ChatKind::System);
        assert_eq!(w.social.ims.len(), 1);
        clean(&w, &root);
    }
}
