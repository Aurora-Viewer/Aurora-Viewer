//! Omnifilter: rules that hide, rewrite or answer what arrives. Port of
//! OmnifilterEngine (indra/newview/omnifilterengine.cpp, originally LGPL
//! 2.1) and of its settings OmnifilterEnabled (off) and
//! OmnifilterRuleSetID:
//! - rules are kept in named rule sets, "Default" first and never removed;
//!   only the chosen set filters;
//! - a message is tried against the enabled rules of that set in their
//!   order, the first one that matches wins (OmnifilterEngine::match);
//! - a rule names any of a sender (exact, substring or full regular
//!   expression match), an owner key, a region (exact), the kinds of
//!   message it applies to (none: every kind) and a content (exact,
//!   substring or regular expression found anywhere); an empty field
//!   matches everything;
//! - what a match does is the caller's: a chat line or an IM is dropped, or
//!   replaced by "Omnifilter Replacement: <text>"; a script dialog is
//!   answered with a button or a text box reply, after a delay of up to
//!   3 s (the callers in `world/omnifilter.rs`);
//! - every match is logged with its time and the rule's name.
//!
//! Regular expressions use the Legacy Viewer's Perl syntax through
//! fancy-regex, compiled once per edit, with a bounded backtracking: a bad
//! expression never matches and is logged, it cannot hang the viewer.
//! Case-insensitive matching folds the whole Unicode range, where the
//! Legacy Viewer lowers ASCII only.

mod file;
#[cfg(test)]
mod tests;

pub use file::{parse, to_xml};

use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// The rule set that always exists, first in the list.
pub const DEFAULT_SET: &str = "Default";

/// A save waits this long after the last change (the engine's 5 s timer).
const SAVE_DELAY: Duration = Duration::from_secs(5);

/// How long the viewer waits, when it closes or logs out, for the writes
/// still under way.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// Longest wait between two tries of a save that keeps failing.
const RETRY_MAX: Duration = Duration::from_secs(300);

/// Matches kept for the editor's log (the Legacy Viewer keeps them all).
const LOG_MAX: usize = 1000;

/// fancy-regex's limit on backtracking: past it, the expression is taken
/// as not matching.
const BACKTRACK_LIMIT: usize = 100_000;

/// The kinds of message a rule can be limited to, with the numbers of
/// OmnifilterEngine::eType that the file keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Source {
    NearbyChat = 0,
    GroupChat = 1,
    GroupNotice = 2,
    GroupInvite = 3,
    InstantMessage = 4,
    ObjectChat = 5,
    ObjectInstantMessage = 6,
    /// The editor offers it, but no message is ever of this kind: debug
    /// chat from scripts is ObjectChat in process_chat_from_simulator.
    ScriptError = 7,
    ScriptDialog = 8,
    FriendshipOffer = 9,
    InventoryOffer = 10,
    Lure = 11,
    TeleportRequest = 12,
    UrlRequest = 13,
}

/// TYPES_MAX: the kind of a message no case of the callers names. Only a
/// rule with no kind (or this one, from a hand-written file) matches it.
const OTHER: u8 = 14;

impl Source {
    /// In the order of the editor's "Match Source" buttons.
    pub const ALL: [Source; 14] = [
        Source::NearbyChat,
        Source::InstantMessage,
        Source::GroupChat,
        Source::GroupNotice,
        Source::GroupInvite,
        Source::ObjectChat,
        Source::ObjectInstantMessage,
        Source::ScriptError,
        Source::ScriptDialog,
        Source::InventoryOffer,
        Source::FriendshipOffer,
        Source::Lure,
        Source::TeleportRequest,
        Source::UrlRequest,
    ];

    fn bit(self) -> u16 {
        1 << self as u8
    }
}

/// OmnifilterEngine::eMatchType.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MatchType {
    Exact,
    #[default]
    Substring,
    Regex,
    /// A number the Legacy Viewer does not know (a hand-written file): it
    /// never matches ("match type unknown!"), and is written back as read.
    Other(i32),
}

impl MatchType {
    pub fn from_i32(v: i32) -> MatchType {
        match v {
            0 => MatchType::Exact,
            1 => MatchType::Substring,
            2 => MatchType::Regex,
            o => MatchType::Other(o),
        }
    }

    pub fn as_i32(self) -> i32 {
        match self {
            MatchType::Exact => 0,
            MatchType::Substring => 1,
            MatchType::Regex => 2,
            MatchType::Other(o) => o,
        }
    }
}

/// One rule (OmnifilterEngine::Needle).
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// Unique in its rule set.
    pub name: String,
    pub enabled: bool,
    pub sender_name: String,
    pub sender_match: MatchType,
    pub sender_case_insensitive: bool,
    pub content: String,
    pub content_match: MatchType,
    pub content_case_insensitive: bool,
    /// Compared exactly, case included.
    pub region_name: String,
    /// Nil: any.
    pub owner_id: Uuid,
    /// The kinds of message (bits of `Source`, bit 14 for TYPES_MAX);
    /// none: every kind.
    pub sources: u16,
    /// Empty: a matched line or IM is dropped; otherwise it is replaced.
    pub chat_replace: String,
    /// The button "clicked" on a matched script dialog.
    pub button_reply: String,
    /// The text sent to a matched llTextBox.
    pub textbox_reply: String,
    /// Seconds before a dialog's answer is sent (0 to 3).
    pub reply_delay: f32,
}

impl Default for Rule {
    /// OmnifilterEngine::Needle's defaults, which a file's missing fields
    /// keep.
    fn default() -> Self {
        Rule {
            name: String::new(),
            enabled: false,
            sender_name: String::new(),
            sender_match: MatchType::Substring,
            sender_case_insensitive: false,
            content: String::new(),
            content_match: MatchType::Substring,
            content_case_insensitive: false,
            region_name: String::new(),
            owner_id: Uuid::nil(),
            sources: 0,
            chat_replace: String::new(),
            button_reply: String::new(),
            textbox_reply: String::new(),
            reply_delay: 0.0,
        }
    }
}

impl Rule {
    /// OmnifilterEngine::newNeedle: disabled, both texts matched as
    /// case-insensitive substrings.
    pub fn new(name: impl Into<String>) -> Rule {
        Rule {
            name: name.into(),
            sender_case_insensitive: true,
            content_case_insensitive: true,
            ..Rule::default()
        }
    }

    pub fn has_source(&self, s: Source) -> bool {
        self.sources & s.bit() != 0
    }

    pub fn set_source(&mut self, s: Source, on: bool) {
        if on {
            self.sources |= s.bit();
        } else {
            self.sources &= !s.bit();
        }
    }

    /// The kinds as the file's numbers, in increasing order (a std::set).
    pub fn source_numbers(&self) -> impl Iterator<Item = i32> + '_ {
        (0..=OTHER).filter(|b| self.sources & (1 << b) != 0).map(i32::from)
    }

    pub fn add_source_number(&mut self, n: i32) {
        if let Ok(b) = u8::try_from(n)
            && b <= OTHER
        {
            self.sources |= 1 << b;
        }
    }
}

/// A named, ordered list of rules.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RuleSet {
    pub name: String,
    pub rules: Vec<Rule>,
}

/// What arrived, as the callers fill it (OmnifilterEngine::Haystack).
#[derive(Clone, Copy, Debug, Default)]
pub struct Haystack<'a> {
    /// None: TYPES_MAX, a kind no caller names.
    pub source: Option<Source>,
    pub sender_name: &'a str,
    pub content: &'a str,
    pub region_name: &'a str,
    pub owner_id: Uuid,
}

/// The rule that matched: what the caller needs of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub name: String,
    pub chat_replace: String,
    pub button_reply: String,
    pub textbox_reply: String,
    pub reply_delay: f32,
}

/// Why a rule set could not be added (the OmniFilter*Blank / *Duplicate
/// notifications).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameError {
    Blank,
    Duplicate,
}

/// A text field of a rule, made ready to compare.
#[derive(Debug)]
enum Matcher {
    /// An empty field: everything passes.
    Any,
    Exact(String, bool),
    Substring(String, bool),
    /// None: the expression does not compile and never matches.
    Regex(Option<fancy_regex::Regex>),
    Never,
}

impl Matcher {
    /// `whole`: the sender's expression must match the whole name
    /// (ll_regex_match), the content's anywhere (ll_regex_search).
    fn new(rule: &str, field: &str, kind: MatchType, case_insensitive: bool, whole: bool) -> Matcher {
        if field.is_empty() {
            return Matcher::Any;
        }
        let folded = || if case_insensitive { fold(field) } else { field.to_owned() };
        match kind {
            MatchType::Exact => Matcher::Exact(folded(), case_insensitive),
            MatchType::Substring => Matcher::Substring(folded(), case_insensitive),
            MatchType::Regex => {
                let pattern = if whole { format!("\\A(?:{field})\\z") } else { field.to_owned() };
                match fancy_regex::RegexBuilder::new(&pattern)
                    .case_insensitive(case_insensitive)
                    .backtrack_limit(BACKTRACK_LIMIT)
                    .build()
                {
                    Ok(re) => Matcher::Regex(Some(re)),
                    Err(e) => {
                        log::warn!("omnifilter: rule \"{rule}\": bad regular expression: {e}");
                        Matcher::Regex(None)
                    }
                }
            }
            MatchType::Other(n) => {
                log::debug!("omnifilter: rule \"{rule}\": match type {n} unknown");
                Matcher::Never
            }
        }
    }

    fn test(&self, text: &str, folded: &mut Option<String>) -> bool {
        match self {
            Matcher::Any => true,
            Matcher::Exact(n, false) => text == n,
            Matcher::Exact(n, true) => lowered(text, folded) == n,
            Matcher::Substring(n, false) => text.contains(n.as_str()),
            Matcher::Substring(n, true) => lowered(text, folded).contains(n.as_str()),
            // a backtracking limit reached counts as no match
            Matcher::Regex(Some(re)) => re.is_match(text).unwrap_or(false),
            Matcher::Regex(None) | Matcher::Never => false,
        }
    }
}

fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// `text` folded, once for all the rules tried on a message.
fn lowered<'a>(text: &str, folded: &'a mut Option<String>) -> &'a str {
    folded.get_or_insert_with(|| fold(text))
}

#[derive(Debug)]
struct Compiled {
    sender: Matcher,
    content: Matcher,
}

/// The rule sets, the one in use, the log of matches and the file.
pub struct Engine {
    sets: Vec<RuleSet>,
    current: usize,
    /// OmnifilterEnabled.
    pub enabled: bool,
    /// The current set's rules made ready, rebuilt after an edit.
    compiled: Vec<Compiled>,
    stale: bool,
    /// (when, rule name) of each match, the oldest first.
    pub log: VecDeque<(SystemTime, String)>,
    /// Since when changes wait to be saved.
    dirty_since: Option<Instant>,
    /// The thread that writes, one write at a time and in the order asked,
    /// so that an older rule set never lands over a newer one.
    saver: Option<Saver>,
    /// Writes queued whose answer has not come yet.
    in_flight: usize,
    /// Wait before the next try after a failed write (doubles up to
    /// `RETRY_MAX`), and the time before which none is made.
    backoff: Duration,
    retry_after: Option<Instant>,
    /// The file being read at startup.
    loading: Option<parking_lot::Mutex<std::sync::mpsc::Receiver<Vec<RuleSet>>>>,
    /// Where the rules are saved; None (tests, demo): nowhere.
    path: Option<std::path::PathBuf>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            sets: vec![default_set()],
            current: 0,
            enabled: false,
            compiled: Vec::new(),
            stale: true,
            log: VecDeque::new(),
            dirty_since: None,
            saver: None,
            in_flight: 0,
            backoff: Duration::ZERO,
            retry_after: None,
            loading: None,
            path: None,
        }
    }
}

/// What a first start has: the Default set with one disabled rule named
/// "Default" (loadNeedles without a file).
pub fn default_set() -> RuleSet {
    RuleSet {
        name: DEFAULT_SET.to_owned(),
        rules: vec![Rule::new(aurora_i18n::tr!("omnifilter-default-name"))],
    }
}

impl Engine {
    /// Rules from these sets, saved nowhere.
    pub fn with_sets(sets: Vec<RuleSet>) -> Engine {
        let mut e = Engine::default();
        e.install(sets);
        e
    }

    pub fn sets(&self) -> &[RuleSet] {
        &self.sets
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn current_set(&self) -> &RuleSet {
        &self.sets[self.current]
    }

    /// The rules of the set in use, for an edit: they are compiled again
    /// and saved a little later.
    pub fn rules_mut(&mut self) -> &mut Vec<Rule> {
        self.edited();
        &mut self.sets[self.current].rules
    }

    /// A change made: compile again, save soon.
    pub fn edited(&mut self) {
        self.stale = true;
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    /// OmnifilterRuleSetID: the set in use (the first for an index out of
    /// range, as assignRuleSetNameFromSettings leaves it).
    pub fn select(&mut self, index: usize) {
        let index = if index < self.sets.len() { index } else { 0 };
        if index != self.current {
            self.current = index;
            self.stale = true;
        }
    }

    /// Try `h` against the enabled rules of the set in use, in their order;
    /// the first that matches is logged and returned.
    pub fn matches(&mut self, h: &Haystack<'_>) -> Option<Match> {
        if !self.enabled {
            return None;
        }
        self.compile();
        let kind_bit = 1u16 << h.source.map_or(OTHER, |s| s as u8);
        let (mut sender_folded, mut content_folded) = (None, None);
        let set = &self.sets[self.current];
        let hit = set.rules.iter().zip(&self.compiled).find(|(rule, c)| {
            rule.enabled
                && c.sender.test(h.sender_name, &mut sender_folded)
                && (rule.owner_id.is_nil() || rule.owner_id == h.owner_id)
                && (rule.region_name.is_empty() || rule.region_name == h.region_name)
                && (rule.sources == 0 || rule.sources & kind_bit != 0)
                && c.content.test(h.content, &mut content_folded)
        });
        let (rule, _) = hit?;
        let found = Match {
            name: rule.name.clone(),
            chat_replace: rule.chat_replace.clone(),
            button_reply: rule.button_reply.clone(),
            textbox_reply: rule.textbox_reply.clone(),
            reply_delay: rule.reply_delay.clamp(0.0, 3.0),
        };
        if self.log.len() >= LOG_MAX {
            self.log.pop_front();
        }
        self.log.push_back((SystemTime::now(), found.name.clone()));
        log::info!("omnifilter: rule \"{}\" matched", found.name);
        Some(found)
    }

    fn compile(&mut self) {
        if !self.stale {
            return;
        }
        self.stale = false;
        self.compiled = self.sets[self.current]
            .rules
            .iter()
            .map(|r| Compiled {
                sender: Matcher::new(&r.name, &r.sender_name, r.sender_match, r.sender_case_insensitive, true),
                content: Matcher::new(&r.name, &r.content, r.content_match, r.content_case_insensitive, false),
            })
            .collect();
    }

    /// Whether a rule of the set in use has this name.
    #[cfg(test)]
    pub fn has_rule(&self, name: &str) -> bool {
        self.current_set().rules.iter().any(|r| r.name == name)
    }

    /// onAddNeedleClicked: a rule named `name` (the "NEW RULE" of the
    /// editor), or the one that already has that name. Returns its index.
    pub fn add_rule(&mut self, name: &str) -> usize {
        if let Some(i) = self.current_set().rules.iter().position(|r| r.name == name) {
            return i;
        }
        let rules = self.rules_mut();
        rules.push(Rule::new(name));
        rules.len() - 1
    }

    pub fn remove_rule(&mut self, index: usize) {
        if index < self.current_set().rules.len() {
            self.rules_mut().remove(index);
        }
    }

    /// onNeedleNameChanged: a name left blank or taken by another rule is
    /// refused. Returns whether the rule was renamed.
    pub fn rename_rule(&mut self, index: usize, name: &str) -> bool {
        let rules = &self.current_set().rules;
        if index >= rules.len() || name.trim().is_empty() {
            return false;
        }
        if rules.iter().enumerate().any(|(i, r)| i != index && r.name == name) {
            return false;
        }
        if rules[index].name != name {
            self.rules_mut()[index].name = name.to_owned();
        }
        true
    }

    /// swapNeedles: the rule moved one place up (`up`) or down. Returns its
    /// new index.
    pub fn move_rule(&mut self, index: usize, up: bool) -> usize {
        let len = self.current_set().rules.len();
        let other = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|o| *o < len)
        };
        match other {
            Some(o) if index < len => {
                self.rules_mut().swap(index, o);
                o
            }
            _ => index,
        }
    }

    fn check_name(&self, name: &str) -> Result<(), NameError> {
        if name.is_empty() {
            return Err(NameError::Blank);
        }
        if self.sets.iter().any(|s| s.name == name) {
            return Err(NameError::Duplicate);
        }
        Ok(())
    }

    /// addNewRuleSet: an empty set, chosen; the editor then adds its first
    /// rule. Returns its index.
    pub fn add_set(&mut self, name: &str) -> Result<usize, NameError> {
        self.check_name(name)?;
        self.sets.push(RuleSet {
            name: name.to_owned(),
            rules: Vec::new(),
        });
        self.chose_new_set();
        Ok(self.current)
    }

    /// addClonedRuleSet: a copy of the set in use, chosen.
    pub fn clone_set(&mut self, name: &str) -> Result<usize, NameError> {
        self.check_name(name)?;
        let rules = self.current_set().rules.clone();
        self.sets.push(RuleSet {
            name: name.to_owned(),
            rules,
        });
        self.chose_new_set();
        Ok(self.current)
    }

    fn chose_new_set(&mut self) {
        self.current = self.sets.len() - 1;
        self.edited();
    }

    /// removeCurrentRuleSet: never the Default set; the Default set is in
    /// use after. Returns whether a set was removed.
    pub fn remove_current_set(&mut self) -> bool {
        if self.current == 0 {
            return false;
        }
        self.sets.remove(self.current);
        self.current = 0;
        self.edited();
        true
    }

    /// Sets read from a file replace these (a read at startup); a list
    /// without the Default set first gets one.
    fn install(&mut self, mut sets: Vec<RuleSet>) {
        if let Some(i) = sets.iter().position(|s| s.name == DEFAULT_SET) {
            let default = sets.remove(i);
            sets.insert(0, default);
        } else {
            sets.insert(0, default_set());
        }
        self.sets = sets;
        if self.current >= self.sets.len() {
            self.current = 0;
        }
        self.stale = true;
    }

    /// Adds sets from a file read after edits were made: a name already
    /// used takes " (2)", " (3)"…; an empty Default set is left out.
    /// Returns how many sets were added.
    pub fn import(&mut self, sets: Vec<RuleSet>) -> usize {
        let mut added = 0;
        for mut set in sets {
            if set.rules.is_empty() && set.name == DEFAULT_SET {
                continue;
            }
            let base = set.name.clone();
            let mut n = 2;
            while self.sets.iter().any(|s| s.name == set.name) {
                set.name = format!("{base} ({n})");
                n += 1;
            }
            self.sets.push(set);
            added += 1;
        }
        if added > 0 {
            self.edited();
        }
        added
    }

    /// Reads the user's file on a thread (OmnifilterEngine::init); until it
    /// is read, the Default set filters (nothing: its rule is disabled).
    pub fn load(&mut self, path: std::path::PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.path = Some(path.clone());
        self.loading = Some(parking_lot::Mutex::new(rx));
        let spawned = std::thread::Builder::new().name("aurora-omnifilter".into()).spawn(move || {
            let sets = match std::fs::read(&path) {
                Ok(bytes) if bytes.is_empty() => {
                    log::warn!("omnifilter: {} is empty", path.display());
                    return;
                }
                Ok(bytes) => match parse(&bytes) {
                    Ok(sets) => sets,
                    Err(e) => {
                        log::warn!("omnifilter: {}: {e}", path.display());
                        return;
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(e) => {
                    log::warn!("omnifilter: {}: {e}", path.display());
                    return;
                }
            };
            let _ = tx.send(sets);
        });
        if let Err(e) = spawned {
            log::warn!("omnifilter: no thread: {e}");
            self.loading = None;
        }
    }

    /// Once a frame: takes the file read, saves what changed 5 s ago
    /// (OmnifilterEngine::tick).
    pub fn update(&mut self, now: Instant) {
        if let Some(rx) = &self.loading {
            let got = rx.lock().try_recv();
            match got {
                Ok(sets) => {
                    self.loading = None;
                    let edits = self.dirty_since.is_some();
                    if !edits {
                        self.install(sets);
                    } else {
                        // edited before the file came: keep the edits, add
                        // what the file had
                        self.import(sets);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        self.collect_writes(now);
        if self.dirty_since.is_some_and(|t| now.duration_since(t) >= SAVE_DELAY) && self.retry_after.is_none_or(|t| now >= t) {
            self.save();
        }
    }

    /// Writes the rules now, on the writer thread (saveNeedles). The change
    /// counts as saved only once the thread says the file is on disk.
    pub fn save(&mut self) {
        if let Some((path, xml)) = self.pending_save() {
            self.queue(path, xml);
        }
    }

    /// Writes what is left to save before the viewer closes or the account
    /// logs out (~OmnifilterEngine: a change made just before closing is
    /// kept), and waits for the writes under way, a few seconds at most.
    pub fn save_before_exit(&mut self) {
        self.save();
        let deadline = Instant::now() + EXIT_WAIT;
        while self.in_flight > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.saver.as_ref().map(|s| s.answers.recv_timeout(left)) {
                Some(Ok(answer)) => self.answered(answer, Instant::now()),
                _ => break,
            }
        }
        // a write that failed, or that was never queued, is tried once more
        // here, on this thread
        if let Some((path, xml)) = self.pending_save()
            && let Err(e) = write(&path, &xml)
        {
            log::warn!("omnifilter: {}: {e}", path.display());
        }
    }

    fn queue(&mut self, path: std::path::PathBuf, xml: String) {
        if self.saver.is_none() {
            self.saver = Saver::start();
        }
        match &self.saver {
            Some(saver) if saver.jobs.send((path, xml)).is_ok() => self.in_flight += 1,
            _ => self.write_failed(Instant::now()),
        }
    }

    /// The answers of the writer thread, once a frame.
    fn collect_writes(&mut self, now: Instant) {
        while let Some(answer) = self.saver.as_ref().and_then(|s| s.answers.try_recv().ok()) {
            self.answered(answer, now);
        }
    }

    fn answered(&mut self, answer: std::io::Result<()>, now: Instant) {
        self.in_flight = self.in_flight.saturating_sub(1);
        match answer {
            Ok(()) => {
                self.backoff = Duration::ZERO;
                self.retry_after = None;
            }
            Err(e) => {
                log::warn!("omnifilter: not saved ({e}); it will be tried again");
                self.write_failed(now);
            }
        }
    }

    /// A write did not reach the disk: the rules are unsaved again, the
    /// next try waits longer.
    fn write_failed(&mut self, now: Instant) {
        self.dirty_since.get_or_insert(now);
        self.backoff = (self.backoff * 2).clamp(SAVE_DELAY, RETRY_MAX);
        self.retry_after = Some(now + self.backoff);
    }

    fn pending_save(&mut self) -> Option<(std::path::PathBuf, String)> {
        self.dirty_since.take()?;
        let path = self.path.clone()?;
        Some((path, to_xml(&self.sets)))
    }
}

/// The writer thread: it takes the files to write in order and answers each
/// one when it is on disk, or why it is not.
struct Saver {
    jobs: crossbeam_channel::Sender<(std::path::PathBuf, String)>,
    answers: crossbeam_channel::Receiver<std::io::Result<()>>,
}

impl Saver {
    fn start() -> Option<Saver> {
        let (jobs, queue) = crossbeam_channel::unbounded::<(std::path::PathBuf, String)>();
        let (done, answers) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("aurora-omnifilter-save".into())
            .spawn(move || {
                for (path, xml) in queue {
                    let _ = done.send(write(&path, &xml));
                }
            })
            .map_err(|e| log::warn!("omnifilter: no thread: {e}"))
            .ok()?;
        Some(Saver { jobs, answers })
    }
}

fn write(path: &std::path::Path, xml: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomic_file::write(path, xml.as_bytes())
}

/// The user's file: `<config>/omnifilter.xml`, the Legacy Viewer's name and
/// format.
pub fn user_file() -> std::path::PathBuf {
    crate::settings::config_dir().join("omnifilter.xml")
}

/// The Settings of the Omnifilter, kept with the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// OmnifilterEnabled: off.
    pub enabled: bool,
    /// OmnifilterRuleSetID: the set in use.
    pub rule_set: usize,
}
