//! Conversation transcripts on disk, like Firestorm's (LLLogChat,
//! LLIMModel::logToFile; indra/newview/lllogchat.cpp, llimview.cpp): every
//! line of an instant message, group chat or conference (and of the nearby
//! chat when asked) is appended to a text file, and the end of that file is
//! shown again, greyed, when the conversation opens.
//!
//! The files of an account on a grid live in their own folder
//! (`store::folder`): nothing is read or written before the login tells
//! which account this is, and after a logout nothing reaches the folder of
//! the account that left. The demo has its own grid name and its own
//! settings folder, so it never touches a real account.
//!
//! When the user said yes to sharing a Legacy Viewer's folder (an Aurora
//! addition, asked when such a folder is picked), the account's files are
//! `<folder>/<first_last>` instead, the Legacy Viewer's own
//! (`legacy_profile::Account::chat_folder`): both viewers then read and add
//! to the same files. That folder is shared, so Aurora only appends lines
//! to it: no import, merge, move or erasing there, and its
//! `conversation.log` is only read (`shared`).
//!
//! - `format`: the text of a line and its reading back;
//! - `names`: the file of each conversation;
//! - `place`: the account's folder, and the checks of a picked folder;
//! - `store`: the folder, the thread that writes and reads;
//! - `transcript`: a whole file for the History window, by pages;
//! - `import`: a Legacy Viewer's transcripts brought into the account's
//!   folder, adding only;
//! - `conversations`: the conversation log (`conversation.log` in the same
//!   folder), who we talked with and when;
//! - `service`: the Chat Service's direct-IM history (the LL viewer 26.5
//!   archive per resident) stitched with the transcript and the live rows.

pub mod conversations;
pub mod format;
pub mod import;
pub mod names;
pub mod place;
pub mod service;
pub mod store;
pub mod transcript;

pub use format::Entry;
pub use place::{FolderCheck, Placement, Refusal};
pub use store::{Identity, Loaded};
pub use transcript::HistorySource;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

/// What the user chose (Preferences › Chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Instant messages, group chats and conferences (Firestorm's
    /// KeepConversationLogTranscripts = 2).
    pub log_im: bool,
    /// The nearby chat (LogNearbyChat). Off by default.
    pub log_nearby: bool,
    /// Show the end of the transcript when a conversation opens
    /// (LogShowHistory).
    pub show_history: bool,
    /// Date in the file names (LogFileNamewithDate).
    pub dated_names: bool,
    /// Import the Legacy Viewer's transcripts at login. On by default.
    pub import_at_login: bool,
    /// Keep the conversation log (KeepConversationLogTranscripts > 0).
    pub log_conversations: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_im: true,
            log_nearby: false,
            show_history: true,
            dated_names: false,
            import_at_login: true,
            log_conversations: true,
        }
    }
}

/// Name of the grid in the transcripts' path. The demo is its own grid: its
/// files can never be those of an account, whatever the folder.
pub fn grid_name(choice: &crate::settings::GridChoice, custom_login_uri: &str, demo: bool) -> String {
    use crate::settings::GridChoice;
    if demo {
        return "demo".to_owned();
    }
    match choice {
        GridChoice::SecondLife => "agni".to_owned(),
        GridChoice::SecondLifeBeta => "aditi".to_owned(),
        GridChoice::Custom => {
            let uri: String = custom_login_uri
                .trim()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
                .collect();
            // the prefix keeps another grid from naming itself like one above
            format!("custom_{}", uri.trim_matches('_'))
        }
    }
}

/// The account in the Legacy layout: the names come from the server, so what
/// a file system refuses becomes '_'.
fn legacy_account(first: &str, last: &str, grid: &str) -> import::Account {
    import::Account {
        first: names::clean_file_name(first),
        last: names::clean_file_name(last),
        grid: grid.to_owned(),
    }
}

/// The transcripts' part of a Chat Service delete (`ChatLog::service_sweep`),
/// run on the transcript thread before the service's own files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceSweep {
    /// Aurora's own folder: its transcripts are erased.
    Erase(PathBuf),
    /// A folder shared with the Legacy Viewer: every transcript stays.
    Shared,
}

impl ServiceSweep {
    /// The number of transcript files removed.
    pub fn run(&self) -> std::io::Result<usize> {
        match self {
            ServiceSweep::Erase(dir) => store::erase(dir),
            ServiceSweep::Shared => Ok(0),
        }
    }
}

struct Open {
    identity: Identity,
    /// The account in the Legacy layout, when the login gave its name.
    account: Option<import::Account>,
    /// Where the files are; None until the login's placement comes back
    /// from the transcript thread: nothing is written before.
    place: Option<Placement>,
}

/// What waits for the login's placement: the lines of the conversations, the
/// histories asked, the conversation log's file and the import at login.
#[derive(Default)]
struct Waiting {
    /// `(peer, stem, text)`, formatted already.
    lines: Vec<(Option<Uuid>, String, String)>,
    /// `(conversation, stem)` of the histories asked.
    tails: Vec<(Uuid, String)>,
    /// The answer channel of the conversation log's read, started at login
    /// (`Log::start`): its file is only known once placed.
    log_read: Option<crossbeam_channel::Sender<std::io::Result<Option<Vec<u8>>>>>,
    /// The import at login, asked before the folder was known.
    import: Option<import::Account>,
}

/// At most this many lines wait for the login's placement (a disk that
/// never answers must not grow the memory without end).
const MAX_WAITING: usize = 4096;

/// A placement asked from the transcript thread, until its answer comes.
struct Placing {
    /// `ChatLog::place_epoch` when it was asked: an older one is dropped.
    epoch: u64,
    answer: crossbeam_channel::Receiver<Placement>,
    /// None for the login; a change of folder and sharing otherwise.
    change: Option<(Option<PathBuf>, bool)>,
}

/// The transcripts of the account logged in.
#[derive(Default)]
pub struct ChatLog {
    pub cfg: Config,
    /// Grid of the next login (given by the app before it logs in); empty
    /// when this run keeps no transcripts.
    grid: String,
    /// Folder holding one folder per account, when it is not the one of the
    /// settings: chosen in the preferences (InstantMessageLogPath), tests.
    accounts: Option<PathBuf>,
    /// The default folder of the accounts when it is not the settings'
    /// (tests): where the default location and a refused folder lead.
    default_accounts: Option<PathBuf>,
    /// The user said yes to sharing the Legacy Viewer's folder of the
    /// chosen folder (`log_legacy_layout`, per account).
    legacy_layout: bool,
    /// A move to another folder asked for, until its answer comes, with the
    /// folder, sharing and place to go back to if it fails.
    moving: Option<Moving>,
    /// A change of folder the transcript thread never took (its placement
    /// or its move dropped: queue full, thread gone): the files stayed in
    /// the folder in use, and the settings that already name the new one
    /// go back (`take_dropped_change`).
    dropped_change: bool,
    open: Option<Open>,
    /// Counts the logins and logouts: what was read for another one is
    /// thrown away.
    epoch: u64,
    /// Generation counter for shared append-path caching in `store::Worker`.
    /// Bumped on login, logout, folder change, or move rollback so the cache
    /// stays session- and folder-scoped.
    append_epoch: u64,
    /// Counts the placements asked: the answer of an older one is dropped.
    place_epoch: u64,
    /// Bumped whenever the active placement (`open.place`) changes or clears,
    /// so open direct views reload plaintext transcripts when placement lands
    /// or the user switches folders.
    place_version: u64,
    placing: Option<Placing>,
    waiting: Waiting,
    worker: Option<store::Worker>,
    /// An erasing asked for, until its answer comes.
    erasing: Option<crossbeam_channel::Receiver<std::io::Result<usize>>>,
    /// The Legacy Viewers' folders, when they are not the system's (tests,
    /// demo).
    legacy_apps: Option<Vec<PathBuf>>,
    /// An import under way.
    importing: Option<import::Running>,
    /// The checks of the folder picked last, until they come (with the
    /// login they were asked in).
    checking: Option<(u64, crossbeam_channel::Receiver<FolderCheck>)>,
    /// The file of a transcript to open outside Aurora, until it is found.
    locating: Option<(u64, crossbeam_channel::Receiver<Option<PathBuf>>)>,
    /// The conversation log of the account logged in.
    pub conversations: conversations::Log,
}

/// A move under way: its answer, and what to go back to if it fails.
struct Moving {
    done: crossbeam_channel::Receiver<std::io::Result<usize>>,
    accounts: Option<PathBuf>,
    legacy_layout: bool,
    place: Placement,
}

impl ChatLog {
    /// Transcripts under another folder than the settings' (tests).
    #[cfg(test)]
    pub fn in_folder(accounts: PathBuf) -> ChatLog {
        ChatLog {
            accounts: Some(accounts),
            ..Default::default()
        }
    }

    /// Another default folder than the settings' (tests): the default
    /// location and the fallback of a refused folder go there.
    #[cfg(test)]
    pub fn set_default_folder(&mut self, accounts: PathBuf) {
        self.default_accounts = Some(accounts);
    }

    /// The grid the next login is made on.
    pub fn set_grid(&mut self, grid: String) {
        if self.open.is_none() {
            self.grid = grid;
        }
    }

    /// The account is known: its folder becomes the one written and read,
    /// placed at once (tests). Without its name, the Legacy layout cannot be
    /// used.
    #[cfg(test)]
    pub fn login(&mut self, agent: Uuid) {
        self.login_named(agent, None);
        assert!(self.settle(Duration::from_secs(10)), "the placement never came");
    }

    /// `login`, with the account's first and last names of the login. The
    /// account's folder is asked from the transcript thread (`place::place`
    /// reads the folders) and taken when it comes (`poll_conversations`);
    /// until then the session's lines and histories wait.
    pub fn login_named(&mut self, agent: Uuid, name: Option<(&str, &str)>) {
        self.epoch += 1;
        self.append_epoch += 1;
        self.place_version += 1;
        self.open = None;
        self.placing = None;
        self.dropped_change = false;
        self.waiting = Waiting::default();
        self.checking = None;
        self.locating = None;
        self.conversations.reset();
        if agent.is_nil() || self.grid.is_empty() {
            log::info!("conversation transcripts: off for this session (no account or no grid)");
            return;
        }
        let identity = Identity {
            grid: self.grid.clone(),
            agent,
        };
        let account = name.map(|(first, last)| legacy_account(first, last, &self.grid));
        let request = self.request(self.accounts.clone(), self.legacy_layout, &identity, account.as_ref());
        self.open = Some(Open {
            identity,
            account,
            place: None,
        });
        self.ask_placement(request, None);
        // LLConversationLog::initLoggingState
        if self.cfg.log_conversations {
            self.start_conversations();
        }
    }

    /// What a placement of the account logged in is asked with.
    fn request(&self, accounts: Option<PathBuf>, share: bool, identity: &Identity, account: Option<&import::Account>) -> place::Request {
        place::Request {
            accounts,
            default_accounts: self.default_accounts.clone(),
            identity: identity.clone(),
            account: account.cloned(),
            share,
        }
    }

    /// Ask the transcript thread where the files go; a placement asked
    /// before is dropped. False when a change of folder could not be asked
    /// (queue full, no thread): nothing is placed, the folder in use stays.
    fn ask_placement(&mut self, request: place::Request, change: Option<(Option<PathBuf>, bool)>) -> bool {
        self.place_epoch += 1;
        let answer = match self.worker().and_then(|w| w.place(request)) {
            Some(answer) => answer,
            None if change.is_some() => return false,
            // the login's, refused or without a thread: the answer channel
            // closes at once, and the fallback of `poll_placement` applies
            None => crossbeam_channel::bounded(1).1,
        };
        self.placing = Some(Placing {
            epoch: self.place_epoch,
            answer,
            change,
        });
        true
    }

    /// The placement's answer, when it came: applied (the login's folder,
    /// or a change of folder).
    fn poll_placement(&mut self) {
        let Some(placing) = &self.placing else {
            return;
        };
        let answer = match placing.answer.try_recv() {
            Ok(place) => Some(place),
            Err(crossbeam_channel::TryRecvError::Empty) => return,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        if let Some(placing) = self.placing.take() {
            self.placed(placing, answer);
        }
    }

    fn placed(&mut self, placing: Placing, answer: Option<Placement>) {
        // an older placement's answer (another login or folder) is dropped
        if placing.epoch != self.place_epoch {
            return;
        }
        let Some(open) = &self.open else {
            return;
        };
        match (placing.change, answer) {
            (None, place) => {
                // the login's: without an answer (the thread is gone, its
                // queue full), the default folder, which needs no reading
                let place = place.unwrap_or_else(|| {
                    log::warn!("conversation transcripts: the account's folder could not be worked out; the default one is used");
                    let root = self.default_accounts.clone().unwrap_or_else(crate::settings::accounts_dir);
                    Placement {
                        dir: store::folder(&root, &open.identity),
                        shared: false,
                        scheme: names::NamingScheme::Legacy,
                        legacy_count: 0,
                    }
                });
                if let Some(open) = &mut self.open {
                    open.place = Some(place);
                }
                self.place_version += 1;
                self.release_waiting();
            }
            // the placement's job dropped unanswered (the thread gone with
            // it queued, or dead while working it out): nothing switched or
            // moved, and the settings go back to the folder in use
            (Some(_), None) => {
                log::warn!("conversation transcripts: the new folder could not be worked out; the current one is kept");
                self.dropped_change = true;
            }
            (Some((accounts, share)), Some(to)) => self.change_place(accounts, share, to),
        }
    }

    /// The login's folder is known: what waited for it goes there.
    fn release_waiting(&mut self) {
        let waiting = std::mem::take(&mut self.waiting);
        for (peer, stem, text) in waiting.lines {
            self.queue_line(peer, &stem, text);
        }
        for (conversation, stem) in waiting.tails {
            self.request_history(conversation, &stem);
        }
        if let Some(done) = waiting.log_read {
            match self.conversations_file() {
                Some(path) => {
                    if let Some(w) = self.worker() {
                        w.read_into(path, done);
                    }
                }
                // (`done` dropped: the read ends without an answer, like a
                // failed one, and nothing is written over the file)
                None => drop(done),
            }
        }
        if let Some(account) = waiting.import {
            self.import_found(account);
        }
    }

    /// A change of folder asked (`change_folder`) is still being placed: a
    /// logout now drops it, the files staying where they are.
    pub fn changing(&self) -> bool {
        self.placing.as_ref().is_some_and(|p| p.change.is_some())
    }

    /// Wait (at most `timeout`) for the placement asked last and apply it:
    /// the exit's last lines, the demo's scenarios, the tests. Not on a
    /// frame: the frames take the answer when it is there.
    pub fn settle(&mut self, timeout: Duration) -> bool {
        let Some(placing) = &self.placing else {
            return true;
        };
        let answer = match placing.answer.recv_timeout(timeout) {
            Ok(place) => Some(place),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => return false,
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => None,
        };
        if let Some(placing) = self.placing.take() {
            self.placed(placing, answer);
        }
        true
    }

    /// Check a folder picked in the preferences for the account logged in,
    /// on the transcript thread; the answer comes through
    /// `take_folder_check`. False while logged out.
    pub fn check_folder(&mut self, picked: PathBuf) -> bool {
        let Some(open) = &self.open else {
            return false;
        };
        let (agent, account) = (open.identity.agent, open.account.clone());
        let epoch = self.epoch;
        match self.worker().map(|w| w.check(picked, agent, account)) {
            Some(answer) => {
                self.checking = Some((epoch, answer));
                true
            }
            None => false,
        }
    }

    /// The checks of the folder picked last, once they come; those of
    /// another login are dropped.
    pub fn take_folder_check(&mut self) -> Option<FolderCheck> {
        let (epoch, answer) = self.checking.as_ref()?;
        let check = match answer.try_recv() {
            Ok(check) => Some(check),
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        let current = *epoch == self.epoch;
        self.checking = None;
        check.filter(|_| current)
    }

    /// The account's files in another folder, nothing moved: the
    /// conversation log of the old place saved (unless shared), then the new
    /// place's read.
    fn apply_dir_switch(&mut self, place: Placement) {
        let changed = self
            .open
            .as_ref()
            .is_some_and(|o| o.place.as_ref().is_none_or(|p| p.dir != place.dir));
        if changed {
            self.save_conversations(true);
        }
        if let Some(open) = &mut self.open {
            open.place = Some(place);
        }
        self.place_version += 1;
        if changed {
            if self.cfg.log_conversations {
                self.start_conversations();
            } else {
                self.conversations.reset();
            }
        }
    }

    /// The account's files are in the Legacy layout, in a folder the Legacy
    /// Viewer reads and writes too: Aurora only appends lines there.
    pub fn shared(&self) -> bool {
        self.place().is_some_and(|p| p.shared)
    }

    fn place(&self) -> Option<&Placement> {
        self.open.as_ref()?.place.as_ref()
    }

    fn conversations_file(&self) -> Option<PathBuf> {
        Some(self.dir()?.join(conversations::FILE))
    }

    /// enableLogging: the file read again, the session's entries after it.
    /// In a shared Legacy Viewer folder, its `conversation.log` is only read.
    /// Before the login's folder is known, the read waits for it
    /// (`release_waiting`), the session's entries buffered meanwhile.
    fn start_conversations(&mut self) {
        let reading = match self.conversations_file() {
            Some(path) => self.worker().map(|w| w.read(path)),
            None if self.open.is_some() => {
                let (done, reading) = crossbeam_channel::bounded(1);
                self.waiting.log_read = Some(done);
                Some(reading)
            }
            None => None,
        };
        self.conversations.start(reading);
    }

    /// Write the conversation log if it changed: at most every few seconds,
    /// at once with `now` (logout, exit, the log cleared). Not while the
    /// transcripts move (except the flush right before logout or relocate),
    /// and never in a shared Legacy Viewer folder: its `conversation.log` is
    /// the Legacy Viewer's, so the entries of this session are listed until
    /// the logout only.
    pub fn save_conversations(&mut self, now: bool) {
        if self.shared() || (self.moving.is_some() && !now) {
            return;
        }
        let Some(path) = self.conversations_file() else {
            return;
        };
        if now {
            // the entries logged while the file was being read are merged
            // before the last save, not lost with the session
            self.conversations.finish_read(conversations::now());
        }
        if let Some(text) = self.conversations.take_save(now) {
            match self.worker().map(|w| w.publish(path, text.into_bytes())) {
                Some(answer) => self.conversations.saving(answer),
                None => self.conversations.save_failed(),
            }
        }
    }

    /// Every frame: the placement taken when it came, the file read, the
    /// setting followed (enableLogging on a change of
    /// KeepConversationLogTranscripts), the changes written.
    pub fn poll_conversations(&mut self) {
        if self.open.is_none() {
            return;
        }
        self.poll_placement();
        self.conversations.poll_read(conversations::now());
        self.conversations.poll_saves();
        match (self.cfg.log_conversations, self.conversations.logging) {
            (true, false) => self.start_conversations(),
            // (a file still being read is waited for on the next frames, not
            // on this one: the last save merges the session's entries into it)
            (false, true) if !self.conversations.is_reading() => {
                self.save_conversations(true);
                self.conversations.stop();
            }
            _ => {}
        }
        self.save_conversations(false);
    }

    /// onClearLogResponse: the log emptied, its file written empty and its
    /// backups removed. Refused in a shared Legacy Viewer folder: the file
    /// and its backups are the Legacy Viewer's.
    pub fn clear_conversations(&mut self) {
        if self.shared() {
            return;
        }
        self.conversations.clear();
        self.save_conversations(true);
        if let Some(dir) = self.dir().map(Path::to_path_buf)
            && let Some(w) = self.worker()
        {
            w.remove_backups(dir);
        }
    }

    /// Detected 1:1 naming scheme of the open shared folder (`Legacy` when
    /// not shared or logged out).
    #[cfg(test)]
    pub fn naming_scheme(&self) -> names::NamingScheme {
        self.place().map_or(names::NamingScheme::Legacy, |p| p.scheme)
    }

    /// Logged out: nothing more is written or read, and what was being read
    /// for that account is dropped. The lines already queued keep the path
    /// they were queued with; those still waiting for the login's folder
    /// are dropped with the session.
    pub fn logout(&mut self) {
        self.epoch += 1;
        self.append_epoch += 1;
        self.place_version += 1;
        // a read still waiting for the folder ends now, so the last save
        // below does not wait for it
        self.waiting = Waiting::default();
        self.placing = None;
        // (the app put the settings back before: `changing`)
        self.dropped_change = false;
        self.checking = None;
        self.locating = None;
        // LLConversationLog::cache at the session's end
        self.save_conversations(true);
        self.conversations.reset();
        self.open = None;
        if let Some(running) = self.importing.take() {
            running.cancel();
        }
    }

    pub fn identity(&self) -> Option<&Identity> {
        self.open.as_ref().map(|o| &o.identity)
    }

    /// Folder of the account logged in, once known.
    pub fn dir(&self) -> Option<&Path> {
        self.place().map(|p| p.dir.as_path())
    }

    fn today(&self) -> Option<(i64, i64, i64)> {
        self.cfg.dated_names.then(names::local_date)
    }

    /// File a conversation is written to now.
    #[cfg(test)]
    pub fn file_of(&self, stem: &str) -> Option<PathBuf> {
        let place = self.place()?;
        if place.shared {
            names::shared_file_path(&place.dir, None, stem, names::local_date(), place.scheme)
        } else {
            names::file_path(&place.dir, stem, self.today())
        }
    }

    /// Descriptor a History window opens on its background thread without
    /// touching disk on the main thread.
    pub fn history_source(&self, stem: &str) -> Option<HistorySource> {
        self.history_source_peer(None, stem)
    }

    /// `history_source`, with the other party's UUID when the conversation
    /// is a 1:1 IM.
    pub fn history_source_peer(&self, peer: Option<Uuid>, stem: &str) -> Option<HistorySource> {
        let place = self.place()?;
        Some(HistorySource::Conversation {
            dir: place.dir.clone(),
            peer,
            stem: stem.to_owned(),
            shared: place.shared,
        })
    }

    /// File stem to record in the conversation log for a 1:1 conversation:
    /// in a shared Legacy Viewer folder, the stem an entry of the session
    /// already records (the Legacy Viewer's), else the folder's naming
    /// scheme; elsewhere `stem` itself.
    pub fn conversation_history_file(&self, session: Uuid, stem: &str) -> String {
        let Some(place) = self.place().filter(|p| p.shared) else {
            return stem.to_owned();
        };
        if let Some(existing) = self
            .conversations
            .list
            .iter()
            .find(|e| e.session == session && !e.history_file.trim().is_empty())
        {
            return existing.history_file.clone();
        }
        place.scheme.stem(stem)
    }

    /// Look for the best existing `.txt` file of a conversation, to open it
    /// outside Aurora (Conversation Log › Open transcript externally), on
    /// the transcript thread; the answer comes through `take_located`.
    pub fn locate(&mut self, source: HistorySource) -> bool {
        let today = self.today();
        let epoch = self.epoch;
        let answer = match source {
            #[cfg(test)]
            HistorySource::File(path) => {
                let (done, answer) = crossbeam_channel::bounded(1);
                let _ = done.send(path.is_file().then_some(path));
                Some(answer)
            }
            HistorySource::Conversation { dir, peer, stem, .. } => self.worker().map(|w| w.locate(dir, peer, stem, today)),
        };
        self.locating = answer.map(|answer| (epoch, answer));
        self.locating.is_some()
    }

    /// The file `locate` found, once: `Some(None)` when there is none.
    pub fn take_located(&mut self) -> Option<Option<PathBuf>> {
        let (epoch, answer) = self.locating.as_ref()?;
        let found = match answer.try_recv() {
            Ok(found) => found,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => None,
        };
        let current = *epoch == self.epoch;
        self.locating = None;
        current.then_some(found)
    }

    fn worker(&mut self) -> Option<&store::Worker> {
        if self.worker.is_none() {
            self.worker = store::Worker::start();
        }
        self.worker.as_ref()
    }

    /// Run a job on the transcript thread, in turn with the lines queued
    /// before it (the Chat Service's storage). False when it was refused:
    /// it will not run.
    pub fn run_job(&mut self, job: impl FnOnce() + Send + 'static) -> bool {
        self.worker().is_some_and(|w| w.run(job))
    }

    /// Monotonic generation of the active folder placement (`open.place`).
    pub fn place_version(&self) -> u64 {
        self.place_version
    }

    /// The legacy-transcript inputs of a direct conversation's stitched
    /// read: the folder, cleaned stem, peer id and shared-folder flag
    /// (LLLogChat::getTranscriptFamily). None while logged out or before the
    /// account's folder is placed.
    pub fn family_of(&self, peer: Option<Uuid>, stem: &str) -> Option<super::chat_service::stitch::TranscriptFamily> {
        let clean = names::clean_file_name(stem.trim());
        if clean.is_empty() {
            return None;
        }
        let place = self.place()?;
        Some(super::chat_service::stitch::TranscriptFamily {
            dir: place.dir.clone(),
            peer,
            stem: clean,
            shared: place.shared,
        })
    }

    /// Append a line to a conversation's file (LLLogChat::saveHistory). An
    /// empty `from` is a system line. Nothing happens while logged out.
    pub fn append(&mut self, stem: &str, time: SystemTime, from: &str, text: &str) {
        self.append_peer(None, stem, time, from, text);
    }

    /// `append`, with the other party's UUID when the conversation is a 1:1
    /// IM (for `conversation.log` lookup in a shared folder). Shared append-path
    /// resolution and session caching happen on the `aurora-chatlog` worker
    /// thread so outgoing messages never perform synchronous disk I/O on the
    /// main frame. Before the login's folder is known, the line waits for it.
    pub fn append_peer(&mut self, peer: Option<Uuid>, stem: &str, time: SystemTime, from: &str, text: &str) {
        let Some(open) = &self.open else {
            return;
        };
        let from = if from.trim().is_empty() { format::SYSTEM_FROM } else { from };
        // stamped with the grid's time, as LLLogChat::saveHistory stamps
        // time_corrected(): a computer behind the grid writes the minute
        // the Chat Service rows have too (#4233)
        let stamp = format::stamp(crate::ui::bars::clock::grid_time(time), true);
        let line = format::format_line(Some(&stamp), from, text);
        let mut text = if store::EOL == "\n" { line } else { line.replace('\n', store::EOL) };
        text.push_str(store::EOL);
        if open.place.is_none() {
            if self.waiting.lines.len() < MAX_WAITING {
                self.waiting.lines.push((peer, stem.to_owned(), text));
            }
            return;
        }
        self.queue_line(peer, stem, text);
    }

    /// A formatted line to the transcript thread, for the folder placed.
    fn queue_line(&mut self, peer: Option<Uuid>, stem: &str, text: String) {
        let today = self.today();
        let epoch = self.append_epoch;
        let Some(place) = self.place() else {
            return;
        };
        // in a shared folder the Legacy Viewer's files decide, dated or not
        let target = if place.shared {
            Ok(store::SharedAppendTarget {
                epoch,
                dir: place.dir.clone(),
                peer,
                stem: stem.to_owned(),
                scheme: place.scheme,
            })
        } else {
            match names::file_path(&place.dir, stem, today) {
                Some(path) => Err(path),
                None => return,
            }
        };
        if let Some(w) = self.worker() {
            match target {
                Ok(shared) => w.append_shared(shared, text),
                Err(path) => w.append(path, text),
            }
        }
    }

    /// Ask for the end of a conversation's file (LLIMSession::loadHistory);
    /// it comes back through `take_loaded`. False when nothing was asked
    /// (logged out, or the history is not shown). Before the login's folder
    /// is known, the request waits for it.
    pub fn request_history(&mut self, conversation: Uuid, stem: &str) -> bool {
        if !self.cfg.show_history {
            return false;
        }
        let Some(open) = &self.open else {
            return false;
        };
        let Some(place) = &open.place else {
            if self.waiting.tails.len() < MAX_WAITING {
                self.waiting.tails.push((conversation, stem.to_owned()));
            }
            return true;
        };
        let (dir, shared) = (place.dir.clone(), place.shared);
        let (today, epoch) = (self.today(), self.epoch);
        match self.worker() {
            Some(w) => {
                w.tail(dir, stem.to_owned(), today, conversation, epoch, shared);
                true
            }
            None => false,
        }
    }

    /// Histories read since the last call, for the account logged in now.
    pub fn take_loaded(&mut self) -> Vec<Loaded> {
        let epoch = self.epoch;
        match &self.worker {
            Some(w) => w.loaded().filter(|l| l.epoch == epoch).collect(),
            None => Vec::new(),
        }
    }

    /// An import, a move, an erasing or a placement is under way: another
    /// one would be refused.
    pub fn busy(&self) -> bool {
        self.importing.is_some() || self.moving.is_some() || self.erasing.is_some() || self.placing.is_some()
    }

    /// Erase the transcripts of the account logged in (on this grid), after
    /// the lines already queued. False when there is no account. The answer
    /// comes through `take_erased`, a failure when the transcript thread
    /// refused or dropped the erasing. Refused in the Legacy layout: the
    /// files are the Legacy Viewer's too.
    pub fn erase(&mut self) -> bool {
        if self.importing.is_some() || self.placing.is_some() || self.shared() {
            return false;
        }
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return false;
        };
        let done = self.worker().and_then(|w| w.erase(dir)).unwrap_or_else(|| {
            // refused (queue full, no thread): nothing is erased, and the
            // answer, a closed channel, reads as a failure
            log::warn!("conversation transcripts: the erasing could not be queued");
            crossbeam_channel::bounded(1).1
        });
        self.erasing = Some(done);
        true
    }

    /// The transcripts' part of a Chat Service delete, asked now or resumed
    /// at login (deleteTranscriptContent in runDelete,
    /// indra/newview/llchatservicehistory.cpp, LL viewer 26.5): the folder
    /// placed now. None while none is placed, or while a placement is still
    /// coming back from the transcript thread (the login's, or a change of
    /// folder's), as with `erase`: the sweep fails and the delete stays
    /// pending rather than guess the folder. A folder shared with the Legacy
    /// Viewer is never erased, as with `erase`: the delete then finishes on
    /// the service's own files and every transcript stays (an Aurora
    /// difference: the LL viewer has no shared folder). The path is taken
    /// here, on the main thread: a layout switched before the job runs
    /// cannot turn the erase onto the shared folder.
    pub fn service_sweep(&self) -> Option<ServiceSweep> {
        if self.placing.is_some() {
            return None;
        }
        let dir = self.dir()?;
        Some(if self.shared() {
            ServiceSweep::Shared
        } else {
            ServiceSweep::Erase(dir.to_path_buf())
        })
    }

    /// The folder chosen in the preferences (None: the default one), for the
    /// next login.
    pub fn set_folder(&mut self, accounts: Option<PathBuf>) {
        if self.moving.is_none() {
            self.accounts = accounts;
        }
    }

    /// The user's yes to sharing the Legacy Viewer's folder, for the next
    /// login (logged in, `change_folder` changes it with the folder).
    pub fn set_sharing(&mut self, on: bool) {
        if self.open.is_none() && self.moving.is_none() {
            self.legacy_layout = on;
        }
    }

    /// Another folder for the transcripts while logged in, sharing as it
    /// is: see `change_folder`.
    #[cfg(test)]
    pub fn relocate(&mut self, accounts: Option<PathBuf>) -> bool {
        self.change_folder(accounts, self.legacy_layout)
    }

    /// Another folder and sharing together (Preferences › Chat, a folder
    /// picked, or Share answered), placed once: set one after the other,
    /// sharing would first be placed under the old folder and read that
    /// folder's conversation log. The new place is asked from the transcript
    /// thread; until it comes, the lines still go to the current folder.
    /// With a shared folder on either side nothing moves and the new
    /// folder's conversation log is read; otherwise the account's files
    /// follow, after the lines already queued, and the next lines go to the
    /// new folder (moveTranscriptsAndLog). True when the change is asked
    /// (false while busy or logged out, or when the transcript thread
    /// refused it); a move's answer comes through `take_relocated`, a change
    /// the thread dropped after taking it through `take_dropped_change`.
    pub fn change_folder(&mut self, accounts: Option<PathBuf>, share: bool) -> bool {
        if self.busy() {
            return false;
        }
        let Some(open) = &self.open else {
            self.accounts = accounts;
            self.legacy_layout = share;
            return false;
        };
        let request = self.request(accounts.clone(), share, &open.identity, open.account.as_ref());
        self.ask_placement(request, Some((accounts, share)))
    }

    /// The new place of a change of folder, come back from the transcript
    /// thread: switched to, or the files moved there.
    fn change_place(&mut self, accounts: Option<PathBuf>, share: bool, to: Placement) {
        let Some(from) = self.place().cloned() else {
            return;
        };
        self.append_epoch += 1;
        // a folder in the Legacy layout is the Legacy Viewer's too: its
        // files never move, in or out; the next lines go to the new folder
        if to.dir == from.dir || to.shared || from.shared {
            self.accounts = accounts;
            self.legacy_layout = share;
            self.apply_dir_switch(to);
            return;
        }
        // the conversation log's last changes go with the transcripts
        self.save_conversations(true);
        let Some(done) = self.worker().and_then(|w| w.relocate(from.dir.clone(), to.dir.clone())) else {
            // the move refused (queue full, thread gone): nothing moves, the
            // lines still go to the folder in use, the settings go back
            log::warn!("conversation transcripts: the files could not be moved; the current folder is kept");
            self.dropped_change = true;
            return;
        };
        let accounts = std::mem::replace(&mut self.accounts, accounts);
        let legacy_layout = std::mem::replace(&mut self.legacy_layout, share);
        if let Some(open) = &mut self.open {
            open.place = Some(to);
        }
        self.place_version += 1;
        self.moving = Some(Moving {
            done,
            accounts,
            legacy_layout,
            place: from,
        });
    }

    /// The answer of the last move, once: the number of files moved. On a
    /// failure the transcripts stay in (and go back to) the folder used
    /// before, and the next lines go there too: the place it had, not one
    /// worked out again from the folder chosen before, which a refused
    /// folder would have sent elsewhere.
    pub fn take_relocated(&mut self) -> Option<std::io::Result<usize>> {
        let done = match self.moving.as_ref()?.done.try_recv() {
            Ok(done) => done,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            // the move's job dropped unanswered (the thread gone with it
            // queued): nothing moved, and the move fails like one that ran
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(std::io::Error::other("the transcript thread dropped the move")),
        };
        let moving = self.moving.take()?;
        if done.is_err() {
            self.append_epoch += 1;
            self.place_version += 1;
            if let Some(open) = &mut self.open {
                open.place = Some(moving.place);
            }
            self.accounts = moving.accounts;
            self.legacy_layout = moving.legacy_layout;
        }
        Some(done)
    }

    /// A change of folder the transcript thread never took, once: its
    /// placement or its move was dropped, nothing switched or moved, and
    /// the folder in use stays; the settings, which already name the new
    /// folder, go back to it.
    pub fn take_dropped_change(&mut self) -> bool {
        std::mem::take(&mut self.dropped_change)
    }

    /// Another transcript thread than the one started on demand (tests).
    #[cfg(test)]
    pub fn set_worker(&mut self, worker: store::Worker) {
        self.worker = Some(worker);
    }

    /// The Legacy Viewers' folders to look in.
    pub fn legacy_apps(&self) -> Vec<PathBuf> {
        self.legacy_apps.clone().unwrap_or_else(crate::legacy_profile::app_folders)
    }

    /// Look in other folders than the system's (tests, demo).
    pub fn set_legacy_apps(&mut self, apps: Vec<PathBuf>) {
        self.legacy_apps = Some(apps);
    }

    /// Import from these folders, every file read again (the import window).
    pub fn import(&mut self, folders: Vec<PathBuf>) -> bool {
        self.start_import(import::Sources::Folders(folders), import::Kinds::ALL, false)
    }

    /// Import from everything found for the account, without the files that
    /// did not change since the last import (at login), and without what
    /// the user chose not to save: unlike the window, nobody asked for this
    /// import. Not on a grid whose folders cannot be told apart: nobody
    /// would see which were read. Asked before the login's folder is known,
    /// it starts once it is.
    pub fn import_found(&mut self, account: import::Account) -> bool {
        let kinds = import::Kinds {
            im: self.cfg.log_im,
            nearby: self.cfg.log_nearby,
        };
        if !account.grid_known() || !kinds.any() {
            return false;
        }
        if self.open.as_ref().is_some_and(|o| o.place.is_none()) {
            self.waiting.import = Some(account);
            return true;
        }
        let apps = self.legacy_apps();
        self.start_import(import::Sources::Find { apps, account }, kinds, true)
    }

    /// Never into the Legacy layout's folder: a merge rewrites the files,
    /// and these are the Legacy Viewer's too.
    fn start_import(&mut self, sources: import::Sources, kinds: import::Kinds, skip_unchanged: bool) -> bool {
        if self.busy() || self.shared() {
            return false;
        }
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return false;
        };
        let request = import::Request {
            sources,
            kinds,
            // beside the account's folder, not in it: never a transcript
            state: dir.with_extension("legacy-import.json"),
            dated: self.cfg.dated_names,
            skip_unchanged,
            // the import at login leaves the conversation log alone when
            // the user keeps none
            conversations: !skip_unchanged || self.cfg.log_conversations,
            dir,
        };
        self.conversations.watch_removals();
        self.importing = self.worker().and_then(|w| import::start(w.merge_port(), request));
        self.importing.is_some()
    }

    /// Files done and files to do of the import under way.
    pub fn import_progress(&self) -> Option<(usize, usize)> {
        use std::sync::atomic::Ordering;
        let running = self.importing.as_ref()?;
        Some((running.done.load(Ordering::Relaxed), running.total.load(Ordering::Relaxed)))
    }

    /// The report of the last import, once.
    pub fn take_imported(&mut self) -> Option<import::Report> {
        let mut report = self.importing.as_ref()?.report.try_recv().ok()?;
        self.importing = None;
        // what the user did while the import ran wins: a cleared log gets
        // nothing back, a removed entry stays removed
        let removals = self.conversations.take_removals();
        if report.at_login && !self.cfg.log_conversations {
            // the log was switched off meanwhile: nothing taken in, nothing
            // marked brought
            report.conversations.clear();
        } else {
            if report.at_login {
                // taken in, or removed by the user on purpose: not brought again
                import::mark_brought_in_background(&report.state, &report.conversations);
            }
            removals.filter(&mut report.conversations);
        }
        Some(report)
    }

    /// The answer of the last erasing, once: the number of files removed.
    pub fn take_erased(&mut self) -> Option<std::io::Result<usize>> {
        let done = match self.erasing.as_ref()?.try_recv() {
            Ok(done) => done,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            // the erasing's job refused or dropped unanswered (the thread
            // gone with it queued): nothing was erased, and it fails
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(std::io::Error::other("the transcript thread dropped the erasing")),
        };
        self.erasing = None;
        Some(done)
    }

    /// Wait for the queued lines to be on disk (exit, tests). Lines still
    /// waiting for the login's folder are queued by `settle` first.
    pub fn flush(&self, timeout: Duration) -> bool {
        self.worker.as_ref().is_none_or(|w| w.flush(timeout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aurora-chatlog-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct TempGuard(PathBuf);

    impl TempGuard {
        fn new() -> Self {
            Self(temp())
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn now() -> SystemTime {
        format::parse_stamp("2026/10/12 14:05").unwrap()
    }

    fn texts(path: &Path) -> Vec<String> {
        store::read_tail(path).unwrap().into_iter().map(|e| e.text).collect()
    }

    fn wait_loaded(log: &mut ChatLog) -> Vec<Loaded> {
        assert!(log.flush(WAIT));
        log.take_loaded()
    }

    #[test]
    fn two_accounts_never_share_a_file() {
        let root = temp();
        let alice = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        // both talk to a "Jane Doe"; the writes alternate, with logins and
        // logouts in between and without waiting for the disk
        for round in 0..20 {
            log.login(alice);
            log.append("Jane Doe", now(), "Jane Doe", &format!("to alice {round}"));
            log.append(names::NEARBY, now(), "Jane Doe", &format!("near alice {round}"));
            log.logout();
            log.login(bob);
            log.append("Jane Doe", now(), "Jane Doe", &format!("to bob {round}"));
            log.append(names::NEARBY, now(), "Jane Doe", &format!("near bob {round}"));
            log.logout();
        }
        assert!(log.flush(WAIT));
        let dir = |agent| {
            store::folder(
                &root,
                &Identity {
                    grid: "agni".into(),
                    agent,
                },
            )
        };
        assert_ne!(dir(alice), dir(bob));
        for (agent, who) in [(alice, "alice"), (bob, "bob")] {
            let im = texts(&dir(agent).join("Jane Doe.txt"));
            assert_eq!(im, (0..20).map(|r| format!("to {who} {r}")).collect::<Vec<_>>());
            let near = texts(&dir(agent).join("chat.txt"));
            assert_eq!(near, (0..20).map(|r| format!("near {who} {r}")).collect::<Vec<_>>());
            // exactly its two files, nothing else
            assert_eq!(std::fs::read_dir(dir(agent)).unwrap().count(), 2);
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_same_account_id_on_two_grids_is_two_folders() {
        let root = temp();
        let agent = Uuid::from_u128(7);
        let mut log = ChatLog::in_folder(root.clone());
        for (grid, text) in [("agni", "on the main grid"), ("aditi", "on the beta grid"), ("demo", "in the demo")] {
            log.set_grid(grid.into());
            log.login(agent);
            assert_eq!(log.identity().unwrap().grid, grid);
            log.append("Jane Doe", now(), "Jane Doe", text);
            log.logout();
        }
        assert!(log.flush(WAIT));
        for (grid, text) in [("agni", "on the main grid"), ("aditi", "on the beta grid"), ("demo", "in the demo")] {
            let dir = store::folder(&root, &Identity { grid: grid.into(), agent });
            assert_eq!(texts(&dir.join("Jane Doe.txt")), [text]);
        }
        // the grid cannot change under an open session
        log.set_grid("agni".into());
        log.login(agent);
        log.set_grid("aditi".into());
        assert_eq!(log.identity().unwrap().grid, "agni");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nothing_is_written_or_read_before_the_login_or_after_the_logout() {
        let root = temp();
        let agent = Uuid::from_u128(9);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.append("Jane Doe", now(), "Jane Doe", "too early");
        assert!(!log.request_history(agent, "Jane Doe"));
        assert_eq!(log.dir(), None);
        assert!(!log.erase());
        // a login without a grid, or without an account, opens nothing
        let mut blind = ChatLog::in_folder(root.clone());
        blind.login(agent);
        blind.append("Jane Doe", now(), "Jane Doe", "no grid");
        log.login(Uuid::nil());
        log.append("Jane Doe", now(), "Jane Doe", "no account");
        log.login(agent);
        log.append("Jane Doe", now(), "Jane Doe", "in time");
        log.logout();
        log.append("Jane Doe", now(), "Jane Doe", "too late");
        assert!(log.flush(WAIT) && blind.flush(WAIT));
        let dir = store::folder(
            &root,
            &Identity {
                grid: "agni".into(),
                agent,
            },
        );
        assert_eq!(texts(&dir.join("Jane Doe.txt")), ["in time"]);
        // and that is the only file under the root
        fn files(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                if e.path().is_dir() {
                    files(&e.path(), out);
                } else {
                    out.push(e.path());
                }
            }
        }
        let mut all = Vec::new();
        files(&root, &mut all);
        assert_eq!(all, [dir.join("Jane Doe.txt")]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_history_read_for_one_account_is_dropped_after_a_switch() {
        let root = temp();
        let alice = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let peer = Uuid::from_u128(0x1A4E);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(alice);
        log.append("Jane Doe", now(), "Jane Doe", "alice's secret");
        assert!(log.request_history(peer, "Jane Doe"));
        // the answer arrives after alice left and bob came
        log.logout();
        log.login(bob);
        assert!(wait_loaded(&mut log).is_empty());
        // bob has no such file: his history of the same peer is empty
        assert!(log.request_history(peer, "Jane Doe"));
        let loaded = wait_loaded(&mut log);
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].entries.is_empty());
        // alice again: hers
        log.logout();
        log.login(alice);
        assert!(log.request_history(peer, "Jane Doe"));
        let loaded = wait_loaded(&mut log);
        assert_eq!(loaded[0].conversation, peer);
        assert_eq!(loaded[0].entries[0].text, "alice's secret");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_history_is_read_before_the_line_that_opened_the_conversation() {
        let root = temp();
        let agent = Uuid::from_u128(3);
        let peer = Uuid::from_u128(4);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(agent);
        log.append("Jane Doe", now(), "Jane Doe", "yesterday");
        assert!(log.flush(WAIT));
        // a new message opens the conversation: history asked, then logged
        assert!(log.request_history(peer, "Jane Doe"));
        log.append("Jane Doe", now(), "Jane Doe", "today");
        let loaded = wait_loaded(&mut log);
        assert_eq!(loaded[0].entries.len(), 1);
        assert_eq!(loaded[0].entries[0].text, "yesterday");
        // not shown when the setting is off
        log.cfg.show_history = false;
        assert!(!log.request_history(peer, "Jane Doe"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lines_are_firestorms_and_multi_line_messages_come_back() {
        let root = temp();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(Uuid::from_u128(5));
        log.append("Jane Doe", now(), "Jane Doe (jane.doe)", "one\ntwo");
        log.append("Jane Doe", now(), "", "Jane Doe is online.");
        assert!(log.flush(WAIT));
        let path = log.file_of("Jane Doe").unwrap();
        let raw = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            raw,
            format!(
                "[2026/10/12 14:05]  Jane Doe (jane.doe): one{eol} two{eol}[2026/10/12 14:05]  Second Life: Jane Doe is online.{eol}",
                eol = store::EOL
            )
        );
        let entries = store::read_tail(&path).unwrap();
        assert_eq!(entries[0].text, "one\ntwo");
        assert!(entries[1].is_system());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn erasing_is_for_the_account_logged_in_only() {
        let root = temp();
        let alice = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        for agent in [alice, bob] {
            log.login(agent);
            log.append("Jane Doe", now(), "Jane Doe", "hello");
            log.append(names::NEARBY, now(), "Jane Doe", "hello");
        }
        // bob is logged in
        assert!(log.erase());
        assert!(log.flush(WAIT));
        assert_eq!(log.take_erased().unwrap().unwrap(), 2);
        assert!(log.take_erased().is_none());
        let dir = |agent| {
            store::folder(
                &root,
                &Identity {
                    grid: "agni".into(),
                    agent,
                },
            )
        };
        assert!(!dir(bob).exists());
        assert_eq!(texts(&dir(alice).join("Jane Doe.txt")), ["hello"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_erasing_the_transcript_thread_never_takes_fails_and_frees_the_history() {
        let guard = TempGuard::new();
        let mut log = ChatLog::in_folder(guard.path().to_path_buf());
        log.set_grid("agni".into());
        log.login(Uuid::from_u128(0xA11CE));
        log.append("Jane Doe", now(), "Jane Doe", "kept");
        assert!(log.flush(WAIT));
        let file = log.dir().unwrap().join("Jane Doe.txt");
        // the thread gone: refused at once, a failure the next frame reads
        let (worker, queue) = store::Worker::stalled();
        drop(queue);
        log.set_worker(worker);
        assert!(log.erase());
        assert!(log.take_erased().unwrap().is_err());
        assert!(!log.busy());
        // taken, then dropped unanswered (the thread gone with it queued):
        // busy until then, a failure after
        let (worker, queue) = store::Worker::stalled();
        log.set_worker(worker);
        assert!(log.erase());
        assert!(log.busy());
        assert!(log.take_erased().is_none(), "no answer yet");
        drop(queue);
        assert!(log.take_erased().unwrap().is_err());
        assert!(!log.busy());
        assert!(log.take_erased().is_none());
        assert_eq!(texts(&file), ["kept"]);
    }

    fn conversation(name: &str, session: u128, time: i64) -> conversations::Conversation {
        conversations::Conversation {
            time,
            kind: conversations::Kind::Private,
            name: name.into(),
            history_file: name.into(),
            session: Uuid::from_u128(session),
            participant: Uuid::from_u128(session + 1),
            offline: false,
        }
    }

    /// Poll until the conversation log's file is read.
    fn read_log(log: &mut ChatLog) {
        assert!(log.flush(WAIT));
        log.poll_conversations();
    }

    #[test]
    fn the_conversation_log_is_read_at_login_and_written_after_a_change() {
        let root = temp();
        let agent = Uuid::from_u128(11);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(agent);
        let file = log.dir().unwrap().join(conversations::FILE);
        let now = conversations::now();
        // what the Legacy Viewer left there, with a line of another kind
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let old = conversation("Jane Doe", 100, now - 3600);
        std::fs::write(
            &file,
            format!(
                "{}not an entry
",
                conversations::format_line(&old)
            ),
        )
        .unwrap();
        log.logout();
        log.login(agent);
        // a session logged while the file is read comes after it
        log.conversations.log(conversation("John Smith", 200, now));
        read_log(&mut log);
        let names: Vec<_> = log.conversations.list.iter().map(|c| c.name.clone()).collect();
        assert_eq!(names, ["Jane Doe", "John Smith"]);
        // written at the logout, read back at the next login
        log.logout();
        assert!(log.flush(WAIT));
        let (back, _) = conversations::parse_file(&std::fs::read(&file).unwrap(), now);
        assert_eq!(back.len(), 2);
        log.login(agent);
        read_log(&mut log);
        assert_eq!(log.conversations.list, back);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn turning_the_conversation_log_off_keeps_the_list_and_stops_logging() {
        let root = temp();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(Uuid::from_u128(12));
        read_log(&mut log);
        let now = conversations::now();
        log.conversations.log(conversation("Jane Doe", 100, now));
        log.cfg.log_conversations = false;
        log.poll_conversations();
        assert!(!log.conversations.logging);
        log.conversations.log(conversation("John Smith", 200, now));
        assert_eq!(log.conversations.list.len(), 1);
        // on again: the file is read again, with what was saved
        log.cfg.log_conversations = true;
        log.poll_conversations();
        read_log(&mut log);
        assert!(log.conversations.logging);
        assert_eq!(log.conversations.list.len(), 1);
        // off at login: nothing is read
        log.logout();
        log.cfg.log_conversations = false;
        log.login(Uuid::from_u128(12));
        read_log(&mut log);
        assert!(log.conversations.list.is_empty() && !log.conversations.logging);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clearing_the_conversation_log_empties_it_and_its_backups() {
        let root = temp();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login(Uuid::from_u128(13));
        read_log(&mut log);
        let dir = log.dir().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        for backup in ["conversation.log.backup", "conversation.log.backup1"] {
            std::fs::write(dir.join(backup), "x").unwrap();
        }
        log.conversations.log(conversation("Jane Doe", 100, conversations::now()));
        log.clear_conversations();
        assert!(log.flush(WAIT));
        assert!(log.conversations.list.is_empty());
        assert_eq!(std::fs::read(dir.join(conversations::FILE)).unwrap(), b"");
        assert!(!dir.join("conversation.log.backup").exists() && !dir.join("conversation.log.backup1").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_conversation_log_moves_with_the_transcripts() {
        let root = temp();
        let other = temp();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        let agent = Uuid::from_u128(14);
        log.login(agent);
        read_log(&mut log);
        log.append("Jane Doe", now(), "Jane Doe", "hello");
        log.conversations.log(conversation("Jane Doe", 100, conversations::now()));
        log.save_conversations(true);
        // a log already in the new folder is kept as a backup
        let to = store::folder(&other, log.identity().unwrap());
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join(conversations::FILE), "older").unwrap();
        assert!(log.relocate(Some(other.clone())));
        settled(&mut log);
        assert!(log.flush(WAIT));
        assert_eq!(log.take_relocated().unwrap().unwrap(), 1);
        let (moved, _) = conversations::parse_file(&std::fs::read(to.join(conversations::FILE)).unwrap(), conversations::now());
        assert_eq!(moved.len(), 1);
        assert_eq!(std::fs::read(to.join("conversation.log.backup")).unwrap(), b"older");
        assert!(
            !store::folder(
                &root,
                &Identity {
                    grid: "agni".into(),
                    agent
                }
            )
            .join(conversations::FILE)
            .exists()
        );
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
    }

    const JANE: Option<(&str, &str)> = Some(("Jane", "Doe"));

    /// The placement asked last, applied (it comes from the transcript
    /// thread on a later frame).
    fn settled(log: &mut ChatLog) {
        assert!(log.settle(WAIT), "the placement never came");
    }

    /// The checks of a picked folder, once they come.
    fn checked(log: &mut ChatLog, picked: &Path) -> FolderCheck {
        assert!(log.check_folder(picked.to_path_buf()));
        let start = std::time::Instant::now();
        loop {
            if let Some(check) = log.take_folder_check() {
                return check;
            }
            assert!(start.elapsed() < WAIT, "the folder check never came");
            std::thread::yield_now();
        }
    }

    /// A Legacy Viewer account folder with its conversation log, a backup
    /// and a transcript: the files and their exact bytes.
    fn legacy_files_in(dir: &Path, now: i64) -> Vec<(&'static str, String)> {
        std::fs::create_dir_all(dir).unwrap();
        let files = vec![
            (
                conversations::FILE,
                conversations::format_line(&conversation("Bob Smith", 101, now - 120)),
            ),
            ("conversation.log.backup", "legacy backup".to_owned()),
            ("bob_smith.txt", "[2026/10/11 09:00]  Bob Smith: legacy line\r\n".to_owned()),
        ];
        for (name, text) in &files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        files
    }

    /// The Legacy Viewer's folder holds exactly its files, byte for byte, and
    /// its root nothing of Aurora's.
    fn assert_legacy_untouched(root: &Path, dir: &Path, files: &[(&str, String)]) {
        for (name, text) in files {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), text.as_bytes(), "{name}");
        }
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), files.len());
        assert_eq!(std::fs::read_dir(root).unwrap().count(), 1);
    }

    fn names_of(log: &ChatLog) -> Vec<String> {
        let mut names: Vec<_> = log.conversations.list.iter().map(|c| c.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_legacy_layout_adds_to_the_legacy_viewers_own_files() {
        let root = temp();
        let me = Uuid::from_u128(0xA11CE);
        // what the Legacy Viewer wrote, and a file of its own beside it
        let legacy = root.join("jane_doe");
        std::fs::create_dir_all(&legacy).unwrap();
        let written = "[2026/10/11 09:00]  Bob Smith: from the legacy viewer\r\n";
        std::fs::write(legacy.join("Bob Smith.txt"), written).unwrap();
        std::fs::write(legacy.join("conversation.log"), "kept").unwrap();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(root.join("default"));
        log.set_grid("agni".into());
        log.set_sharing(true);
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(log.shared());
        assert_eq!(log.dir(), Some(legacy.as_path()));
        log.append("Bob Smith", now(), "Bob Smith", "from aurora");
        assert!(log.flush(WAIT));
        // appended, the Legacy Viewer's line left as it was written
        let text = std::fs::read_to_string(legacy.join("Bob Smith.txt")).unwrap();
        assert!(text.starts_with(written), "{text}");
        assert_eq!(texts(&legacy.join("Bob Smith.txt")), ["from the legacy viewer", "from aurora"]);
        // and read back as the conversation's history
        assert!(log.request_history(Uuid::from_u128(1), "Bob Smith"));
        let loaded = wait_loaded(&mut log);
        assert_eq!(loaded[0].entries.len(), 2);
        // nothing that rewrites or removes these files
        assert!(!log.erase());
        assert!(!log.import(vec![root.join("elsewhere")]));
        assert_eq!(std::fs::read_to_string(legacy.join("conversation.log")).unwrap(), "kept");
        // nothing of Aurora's layout was made
        let aurora = store::folder(
            &root,
            &Identity {
                grid: "agni".into(),
                agent: me,
            },
        );
        assert!(!aurora.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_legacy_folder_is_shared_only_with_the_user_s_yes() {
        let guard = TempGuard::new();
        let legacy_root = guard.path().join("Firestorm_x64");
        let legacy_dir = legacy_root.join("jane_doe");
        let files = legacy_files_in(&legacy_dir, conversations::now());
        let me = Uuid::from_u128(0xA11CE);
        // the Legacy root chosen without a yes (PR #3588 with its option
        // off): Aurora's own layout under it, and no question at login
        let mut log = ChatLog::in_folder(legacy_root.clone());
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(!log.shared());
        assert_eq!(log.dir(), Some(store::folder(&legacy_root, log.identity().unwrap()).as_path()));
        assert!(log.take_folder_check().is_none());
        // picked, the folder is a question, nothing is changed by it
        assert_eq!(
            checked(&mut log, &legacy_root),
            FolderCheck::Legacy {
                root: legacy_root.clone(),
                folder: "jane_doe".into(),
                count: 1,
            }
        );
        assert!(!log.shared());
        // Share: the same folder, now in the Legacy layout, nothing moved
        log.change_folder(Some(legacy_root.clone()), true);
        settled(&mut log);
        assert!(log.shared());
        assert_eq!(log.dir(), Some(legacy_dir.as_path()));
        // the yes saved for the account: shared again at the next login
        log.logout();
        log.set_sharing(true);
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(log.shared());
        assert_eq!(log.dir(), Some(legacy_dir.as_path()));
        // "Default folder" stops sharing
        log.change_folder(None, false);
        settled(&mut log);
        assert!(!log.shared());
        assert!(log.dir().unwrap().starts_with(guard.path().join("default")));
        log.logout();
        assert!(log.flush(WAIT));
        for (name, text) in &files {
            assert_eq!(std::fs::read(legacy_dir.join(name)).unwrap(), text.as_bytes(), "{name}");
        }
    }

    #[test]
    fn switching_to_and_from_a_shared_folder_moves_nothing() {
        let root = temp();
        let aurora_root = root.join("aurora");
        let legacy_root = root.join("Firestorm_x64");
        let other_root = root.join("Firestorm");
        std::fs::create_dir_all(&aurora_root).unwrap();
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::create_dir_all(&other_root).unwrap();
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(aurora_root.clone());
        log.set_default_folder(root.join("default"));
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(!log.shared());
        let aurora = log.dir().unwrap().to_path_buf();
        log.append("Bob Smith", now(), "Bob Smith", "before");
        log.change_folder(Some(legacy_root.clone()), true);
        settled(&mut log);
        assert!(log.shared());
        log.append("Bob Smith", now(), "Bob Smith", "shared");
        log.change_folder(Some(aurora_root), false);
        settled(&mut log);
        assert!(!log.shared());
        assert_eq!(log.dir(), Some(aurora.as_path()));
        log.append("Bob Smith", now(), "Bob Smith", "after");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&aurora.join("Bob Smith.txt")), ["before", "after"]);
        assert_eq!(texts(&legacy_root.join("jane_doe").join("Bob Smith.txt")), ["shared"]);
        // another viewer root shared: the shared files stay where they were written
        log.change_folder(Some(legacy_root.clone()), true);
        settled(&mut log);
        log.change_folder(Some(other_root.clone()), true);
        settled(&mut log);
        assert_eq!(log.dir(), Some(other_root.join("jane_doe").as_path()));
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy_root.join("jane_doe").join("Bob Smith.txt")), ["shared"]);
        // back to the default folder (the test's own): Aurora's layout, still no move
        log.change_folder(None, false);
        settled(&mut log);
        assert!(!log.shared());
        assert!(log.dir().unwrap().starts_with(root.join("default")));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_legacy_layout_needs_a_name_and_a_known_grid() {
        let base = temp();
        let root = base.join("Firestorm_x64");
        std::fs::create_dir_all(&root).unwrap();
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(base.join("default"));
        log.set_sharing(true);
        log.set_grid("aditi".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert_eq!(log.dir(), Some(root.join("jane_doe.second_life_beta").as_path()));
        log.logout();
        // another grid's label is the Legacy Viewer's own: Aurora's layout
        log.set_grid("custom_osgrid_org".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(!log.shared());
        log.logout();
        log.set_grid("agni".into());
        log.login(me);
        assert!(!log.shared());
        // a name never leaves the chosen folder
        log.logout();
        log.login_named(me, Some(("..", "x/y")));
        settled(&mut log);
        let dir = log.dir().unwrap();
        assert_eq!(dir.parent(), Some(root.as_path()));
        // without a chosen folder: Aurora's own layout (nothing is written)
        log.logout();
        let mut default = ChatLog::default();
        default.set_default_folder(base.join("default"));
        default.set_sharing(true);
        default.set_grid("agni".into());
        default.login_named(me, JANE);
        settled(&mut default);
        assert!(!default.shared());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_service_sweep_never_erases_a_shared_folder() {
        let root = temp();
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(root.join("default"));
        log.set_grid("agni".into());
        assert_eq!(log.service_sweep(), None, "no folder placed");
        log.login_named(me, JANE);
        assert_eq!(log.service_sweep(), None, "the login's placement is not known yet");
        settled(&mut log);
        let own = log.dir().unwrap().to_path_buf();
        assert_eq!(log.service_sweep(), Some(ServiceSweep::Erase(own)));
        let shared = root.join("jane_doe");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("Bob Smith.txt"), "kept").unwrap();
        assert!(log.change_folder(Some(root.clone()), true));
        assert_eq!(log.service_sweep(), None, "the change of folder is not placed yet");
        settled(&mut log);
        assert!(log.shared());
        let sweep = log.service_sweep().unwrap();
        assert_eq!(sweep, ServiceSweep::Shared);
        assert_eq!(sweep.run().unwrap(), 0);
        assert_eq!(std::fs::read(shared.join("Bob Smith.txt")).unwrap(), b"kept");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn grids_have_distinct_names() {
        use crate::settings::GridChoice;
        assert_eq!(grid_name(&GridChoice::SecondLife, "", false), "agni");
        assert_eq!(grid_name(&GridChoice::SecondLifeBeta, "", false), "aditi");
        assert_eq!(
            grid_name(&GridChoice::Custom, "http://127.0.0.1:9000/", false),
            "custom_http___127_0_0_1_9000"
        );
        // a custom grid cannot pass for the main one, nor for the demo
        assert_eq!(grid_name(&GridChoice::Custom, "agni", false), "custom_agni");
        assert_eq!(grid_name(&GridChoice::Custom, "demo", false), "custom_demo");
        for choice in [GridChoice::SecondLife, GridChoice::SecondLifeBeta, GridChoice::Custom] {
            assert_eq!(grid_name(&choice, "http://example.invalid/", true), "demo");
        }
    }

    #[test]
    fn shared_folder_adopts_username_files_and_combines_split_history() {
        let guard = TempGuard::new();
        let root = guard.path().join("SecondLife");
        let me = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let dana = Uuid::from_u128(0xDA4A);
        let legacy = root.join("jane_doe");
        std::fs::create_dir_all(&legacy).unwrap();

        // Bob already has a username-scheme file in the shared folder
        std::fs::write(legacy.join("bob_smith.txt"), "[2026/10/12 14:00]  Bob Smith: from username file\n").unwrap();
        // Dana has split files in both schemes
        std::fs::write(legacy.join("Dana Sato.txt"), "[2026/10/12 14:01]  Dana Sato: legacy line\n").unwrap();
        std::fs::write(legacy.join("dana_sato.txt"), "[2026/10/12 14:02]  Dana Sato: username line\n").unwrap();

        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        log.set_sharing(true);
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(log.shared());
        assert_eq!(log.naming_scheme(), names::NamingScheme::Username);

        // Appending to Bob uses the existing `bob_smith.txt` and never creates `Bob Smith.txt`
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "reply to bob");
        // Appending to a brand-new resident uses the folder's detected scheme (`alex_roe.txt`)
        log.append("Alex Roe", now(), "Jane Doe", "hello alex");
        assert!(log.flush(WAIT));
        assert!(!legacy.join("Bob Smith.txt").exists());
        assert_eq!(texts(&legacy.join("bob_smith.txt")), ["from username file", "reply to bob"]);
        assert!(legacy.join("alex_roe.txt").is_file());
        assert!(!legacy.join("Alex Roe.txt").exists());

        // Requesting Dana's history combines both candidate files in time order
        assert!(log.request_history(dana, "Dana Sato"));
        let loaded = wait_loaded(&mut log);
        let dana_texts: Vec<&str> = loaded[0].entries.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(dana_texts, vec!["legacy line", "username line"]);
    }

    #[test]
    fn the_folder_checks_refuse_other_accounts_and_ask_about_legacy_folders() {
        let guard = TempGuard::new();
        let root = guard.path().join("SecondLife");
        let me = Uuid::from_u128(0xA11CE);
        let legacy = root.join("jane_doe");
        std::fs::create_dir_all(&legacy).unwrap();
        for name in ["bob_smith.txt", "Dana Sato.txt", "dana_sato.txt"] {
            std::fs::write(legacy.join(name), "[2026/10/12 14:00]  Bob Smith: hi\n").unwrap();
        }
        std::fs::write(legacy.join("search_history.txt"), "<llsd/>").unwrap();
        let mut log = ChatLog::default();
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        // logged out: nothing is checked
        assert!(!log.check_folder(root.clone()));
        log.login_named(me, JANE);
        settled(&mut log);

        // the viewer root or the account folder: the same question, with
        // the transcripts found
        let jane = FolderCheck::Legacy {
            root: root.clone(),
            folder: "jane_doe".into(),
            count: 3,
        };
        assert_eq!(checked(&mut log, &root), jane);
        assert_eq!(checked(&mut log, &legacy), jane);
        // a known viewer's root without the account's folder yet asks too
        let empty_root = guard.path().join("Firestorm_x64");
        std::fs::create_dir_all(&empty_root).unwrap();
        assert_eq!(
            checked(&mut log, &empty_root),
            FolderCheck::Legacy {
                root: empty_root.clone(),
                folder: "jane_doe".into(),
                count: 0,
            }
        );
        // another folder is Aurora's own layout, asked nothing
        let plain = guard.path().join("logs");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(checked(&mut log, &plain), FolderCheck::Plain { root: plain.clone() });

        // Foreign account folders (Legacy subfolder or Aurora UUID dir) are refused, and the
        // account's own folder in the parent viewer root is offered instead
        let other = root.join("alice_resident");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();
        assert_eq!(
            checked(&mut log, &other),
            FolderCheck::Refused {
                why: Refusal::OtherAccount,
                folder: "alice_resident".into(),
                offer: Some((root.clone(), "jane_doe".into())),
            }
        );
        let foreign_id = Uuid::from_u128(0xDEAD);
        let foreign_aurora = root.join(foreign_id.to_string()).join("chat_logs").join("agni");
        std::fs::create_dir_all(&foreign_aurora).unwrap();
        for picked in [foreign_aurora.parent().unwrap().parent().unwrap(), foreign_aurora.as_path()] {
            assert!(
                matches!(
                    checked(&mut log, picked),
                    FolderCheck::Refused { why: Refusal::OtherAccount, ref folder, .. } if *folder == foreign_id.to_string()
                ),
                "{picked:?}"
            );
        }
        // nothing changed by any of these checks
        assert!(!log.shared());
        assert!(log.dir().unwrap().starts_with(guard.path().join("default")));
    }

    #[test]
    fn append_caches_shared_path_until_folder_move_date_change_or_missing_file() {
        let guard = TempGuard::new();
        let root = guard.path().join("Firestorm");
        let me = Uuid::from_u128(0xA11CE);
        let bob = Uuid::from_u128(0xB0B);
        let session = Uuid::from_u128(0x9999);
        let legacy = root.join("jane_doe");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("bob_smith.txt"), "[2026/10/12 14:00]  Bob Smith: line 0\n").unwrap();

        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        log.set_sharing(true);
        log.login_named(me, JANE);
        settled(&mut log);

        // First append resolves and caches `bob_smith.txt`.
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 1");
        assert!(log.flush(WAIT));

        // Even if `conversation.log` and a target override file appear mid-session,
        // repeated appends reuse the cached path without re-scanning the folder.
        std::fs::write(legacy.join("bob_override.txt"), "[2026/10/12 14:00]  Bob Smith: base\n").unwrap();
        std::fs::write(
            legacy.join("conversation.log"),
            format!("[1700000500] 0 0 0 Bob%20Smith| {bob} {session} bob_override|\n"),
        )
        .unwrap();
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 2");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy.join("bob_smith.txt")), ["line 0", "line 1", "line 2"]);
        assert_eq!(texts(&legacy.join("bob_override.txt")), ["base"]);

        // Aurora's Dated files option does not pick the file in a shared
        // folder (the Legacy Viewer's files do): the cached file is kept,
        // and no dated file is started beside it
        log.cfg.dated_names = true;
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 3");
        assert!(log.flush(WAIT));
        log.cfg.dated_names = false;
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 4");
        assert!(log.flush(WAIT));
        assert_eq!(
            texts(&legacy.join("bob_smith.txt")),
            ["line 0", "line 1", "line 2", "line 3", "line 4"]
        );
        assert_eq!(std::fs::read_dir(&legacy).unwrap().count(), 3);

        log.change_folder(None, false);
        settled(&mut log);
        log.change_folder(Some(root.clone()), true);
        settled(&mut log);
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 5");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy.join("bob_override.txt")), ["base", "line 5"]);

        // the file written to goes away (the Legacy Viewer renamed it): the
        // next line follows the file there is now, without a new epoch
        std::fs::rename(legacy.join("bob_override.txt"), legacy.join("Bob Override.txt")).unwrap();
        std::fs::write(
            legacy.join("conversation.log"),
            format!("[1700000600] 0 0 0 Bob%20Smith| {bob} {session} Bob Override|\n"),
        )
        .unwrap();
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 6");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy.join("Bob Override.txt")), ["base", "line 5", "line 6"]);
        assert!(!legacy.join("bob_override.txt").exists());

        // Moving to another shared root bumps `append_epoch` and clears cached paths as well.
        let other_guard = TempGuard::new();
        let other_root = other_guard.path().join("OtherViewer");
        let other_legacy = other_root.join("jane_doe");
        std::fs::create_dir_all(&other_legacy).unwrap();
        log.change_folder(Some(other_root.clone()), true);
        settled(&mut log);
        log.append_peer(Some(bob), "Bob Smith", now(), "Jane Doe", "line 7");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&other_legacy.join("Bob Smith.txt")), ["line 7"]);
        assert_eq!(texts(&legacy.join("Bob Override.txt")), ["base", "line 5", "line 6"]);
    }

    /// Dated files on while sharing: the lines go to the file the Legacy
    /// Viewer reads back (LLLogChat::loadChatHistory), never to a dated
    /// file of Aurora's beside it; in Aurora's own folder dated names stay.
    #[test]
    fn dated_files_while_sharing_follow_the_legacy_viewer_s_files() {
        let guard = TempGuard::new();
        let root = guard.path().join("Firestorm");
        let legacy = root.join("jane_doe");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("Jane Doe.txt"), "[2026/10/12 14:00]  Jane Doe: legacy\n").unwrap();
        let today = names::local_date();
        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        log.set_sharing(true);
        log.cfg.dated_names = true;
        log.login_named(Uuid::from_u128(0xA11CE), JANE);
        settled(&mut log);
        assert!(log.shared());
        let jane = Some(Uuid::from_u128(0x7A7E));

        // `Jane Doe.txt` and no file for this month: appended to it
        log.append_peer(jane, "Jane Doe", now(), "Jane Doe", "first");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy.join("Jane Doe.txt")), ["legacy", "first"]);
        let this_month = names::file_path(&legacy, "Jane Doe", Some(today)).unwrap();
        assert!(!this_month.exists());
        // the Legacy Viewer (dated names on) starts this month's file: the
        // next line follows it the same day
        std::fs::write(&this_month, "[2026/10/12 14:00]  Jane Doe: dated\n").unwrap();
        log.append_peer(jane, "Jane Doe", now(), "Jane Doe", "second");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&this_month), ["dated", "second"]);
        assert_eq!(texts(&legacy.join("Jane Doe.txt")), ["legacy", "first"]);
        // a new conversation and the nearby chat: the undated names, which
        // the Legacy Viewer reads with either setting
        log.append_peer(Some(Uuid::from_u128(0xB0B)), "Bob Smith", now(), "Bob Smith", "new");
        log.append(names::NEARBY, now(), "Bob Smith", "nearby");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&legacy.join("Bob Smith.txt")), ["new"]);
        assert_eq!(texts(&legacy.join("chat.txt")), ["nearby"]);
        assert_eq!(std::fs::read_dir(&legacy).unwrap().count(), 4);

        // Aurora's own folder: dated names as before
        assert!(log.change_folder(None, false));
        settled(&mut log);
        assert!(!log.shared());
        log.append_peer(jane, "Jane Doe", now(), "Jane Doe", "own");
        assert!(log.flush(WAIT));
        let own = log.dir().unwrap().to_path_buf();
        assert_eq!(texts(&names::file_path(&own, "Jane Doe", Some(today)).unwrap()), ["own"]);
        assert!(!own.join("Jane Doe.txt").exists());
    }

    #[test]
    fn aditi_rejects_own_main_grid_folder_and_empty_custom_root_stays_root() {
        let guard = TempGuard::new();
        let base = guard.path();
        let legacy_root = base.join("Firestorm");
        let main_jane = legacy_root.join("jane_doe");
        let beta_jane = legacy_root.join("jane_doe.second_life_beta");
        std::fs::create_dir_all(&main_jane).unwrap();
        std::fs::create_dir_all(&beta_jane).unwrap();
        std::fs::write(main_jane.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();

        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(legacy_root.clone());
        log.set_default_folder(base.join("default"));
        log.set_sharing(true);
        log.set_grid("aditi".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert_eq!(log.dir().unwrap(), beta_jane);
        assert_eq!(
            checked(&mut log, &main_jane),
            FolderCheck::Refused {
                why: Refusal::MainGrid,
                folder: "jane_doe".into(),
                offer: Some((legacy_root.clone(), "jane_doe.second_life_beta".into())),
            }
        );
        // Never offer a parent root that the placement itself would refuse
        std::fs::write(legacy_root.join("settings_per_account.xml"), "<llsd/>").unwrap();
        assert!(matches!(
            checked(&mut log, &main_jane),
            FolderCheck::Refused {
                why: Refusal::MainGrid,
                offer: None,
                ..
            }
        ));
        std::fs::remove_file(legacy_root.join("settings_per_account.xml")).unwrap();

        // Pre-saved main-grid account folder on Aditi refuses to nest inside `main_jane`.
        let default_root = base.join("default");
        let mut saved_main = ChatLog::in_folder(main_jane.clone());
        saved_main.set_default_folder(default_root.clone());
        saved_main.set_sharing(true);
        saved_main.set_grid("aditi".into());
        saved_main.login_named(me, JANE);
        settled(&mut saved_main);
        assert!(!saved_main.dir().unwrap().starts_with(&main_jane));
        assert!(saved_main.dir().unwrap().starts_with(&default_root));
        assert!(!saved_main.shared());

        // An empty custom folder named `jane_doe` outside any viewer root stays the chosen root
        // until transcript files make it recognizable as an existing account subfolder.
        let custom_parent = base.join("archive");
        let custom_empty_jane = custom_parent.join("jane_doe");
        std::fs::create_dir_all(&custom_empty_jane).unwrap();
        let mut agni = ChatLog::in_folder(custom_empty_jane.clone());
        agni.set_default_folder(base.join("default"));
        agni.set_grid("agni".into());
        agni.login_named(me, JANE);
        settled(&mut agni);
        assert!(!agni.shared());
        assert_eq!(
            checked(&mut agni, &custom_empty_jane),
            FolderCheck::Plain {
                root: custom_empty_jane.clone()
            }
        );
        std::fs::write(custom_empty_jane.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();
        assert_eq!(
            checked(&mut agni, &custom_empty_jane),
            FolderCheck::Legacy {
                root: custom_parent.clone(),
                folder: "jane_doe".into(),
                count: 1,
            }
        );
    }

    #[test]
    fn pre_saved_foreign_account_root_falls_back_to_default() {
        let guard = TempGuard::new();
        let root = guard.path().join("Firestorm");
        let me = Uuid::from_u128(0xA11CE);
        let foreign_legacy = root.join("alice_resident");
        std::fs::create_dir_all(&foreign_legacy).unwrap();
        std::fs::write(foreign_legacy.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();

        let foreign_id = Uuid::from_u128(0xDEAD);
        let foreign_aurora = root.join(foreign_id.to_string()).join("chat_logs").join("agni");
        std::fs::create_dir_all(&foreign_aurora).unwrap();

        let default_root = guard.path().join("default");
        for bad_root in [&foreign_legacy, foreign_aurora.parent().unwrap().parent().unwrap(), &foreign_aurora] {
            for share in [false, true] {
                let mut log = ChatLog::in_folder(bad_root.to_path_buf());
                log.set_default_folder(default_root.clone());
                log.set_sharing(share);
                log.set_grid("agni".into());
                log.login_named(me, JANE);
                settled(&mut log);
                let dir = log.dir().unwrap();
                assert!(!dir.starts_with(bad_root), "must not nest inside foreign folder {}", dir.display());
                assert!(
                    dir.starts_with(&default_root),
                    "expected fallback to {}, got {}",
                    default_root.display(),
                    dir.display()
                );
                assert!(!log.shared());
            }
        }

        // Relocating an active shared session to a foreign folder also refuses to nest inside it.
        let mut log = ChatLog::in_folder(root.clone());
        log.set_default_folder(default_root.clone());
        log.set_sharing(true);
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        assert!(log.shared());
        let _ = log.change_folder(Some(foreign_legacy.clone()), true);
        settled(&mut log);
        assert!(!log.dir().unwrap().starts_with(&foreign_legacy));
        assert!(log.dir().unwrap().starts_with(&default_root));
        assert!(!log.shared());
    }

    #[test]
    fn a_failed_move_keeps_the_folder_used_before() {
        let guard = TempGuard::new();
        let root = guard.path().join("Firestorm");
        let default_root = guard.path().join("default");
        let me = Uuid::from_u128(0xA11CE);
        // the saved folder is another account's: refused, the default one used
        let foreign = root.join("alice_resident");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("Bob.txt"), "[2026/10/12 14:00]  Bob: hi\n").unwrap();
        let mut log = ChatLog::in_folder(foreign.clone());
        log.set_default_folder(default_root.clone());
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        let before = log.dir().unwrap().to_path_buf();
        assert!(before.starts_with(&default_root));
        log.append("Bob Smith", now(), "Bob Smith", "before the pick");
        // X picked, but a transcript of the same name is there: the move fails
        let x = guard.path().join("x");
        let x_dir = store::folder(&x, log.identity().unwrap());
        std::fs::create_dir_all(&x_dir).unwrap();
        std::fs::write(x_dir.join("Bob Smith.txt"), "[2026/10/12 14:00]  Bob Smith: x\n").unwrap();
        log.change_folder(Some(x.clone()), false);
        settled(&mut log);
        assert!(log.flush(WAIT));
        assert!(log.take_relocated().unwrap().is_err());
        // the next lines go where they went before the pick, not to the
        // refused folder worked out again
        assert_eq!(log.dir(), Some(before.as_path()));
        log.append("Bob Smith", now(), "Bob Smith", "after the failed move");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&before.join("Bob Smith.txt")), ["before the pick", "after the failed move"]);
        assert_eq!(texts(&x_dir.join("Bob Smith.txt")), ["x"]);
        assert!(!foreign.join(me.to_string()).exists());
        assert_eq!(std::fs::read_dir(&foreign).unwrap().count(), 1);
    }

    #[test]
    fn a_move_the_transcript_thread_drops_keeps_the_folder_used_before() {
        let guard = TempGuard::new();
        let mut log = ChatLog::in_folder(guard.path().join("a"));
        log.set_grid("agni".into());
        log.login_named(Uuid::from_u128(0xA11CE), JANE);
        settled(&mut log);
        let before = log.dir().unwrap().to_path_buf();
        log.append("Bob Smith", now(), "Bob Smith", "before the pick");
        // placed by a working thread, then the move taken by one that dies
        // with it queued
        assert!(log.change_folder(Some(guard.path().join("b")), false));
        assert!(log.flush(WAIT));
        let (worker, queue) = store::Worker::stalled();
        log.set_worker(worker);
        settled(&mut log);
        assert!(log.busy());
        assert!(log.take_relocated().is_none(), "no answer yet");
        drop(queue);
        // the closed answer reads as a failed move, not as "not yet": the
        // rollback applies and the history is free again
        assert!(log.take_relocated().unwrap().is_err());
        assert!(!log.busy());
        assert!(!log.take_dropped_change());
        assert_eq!(log.dir(), Some(before.as_path()));
        log.set_worker(store::Worker::start().unwrap());
        log.append("Bob Smith", now(), "Bob Smith", "after the dropped move");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&before.join("Bob Smith.txt")), ["before the pick", "after the dropped move"]);
        // a folder can be changed again
        assert!(log.change_folder(Some(guard.path().join("b")), false));
        settled(&mut log);
        assert!(log.flush(WAIT));
        assert_eq!(log.take_relocated().unwrap().unwrap(), 1);
    }

    #[test]
    fn a_shared_folder_s_conversation_log_is_only_read() {
        let guard = TempGuard::new();
        let default = guard.path().join("default");
        let legacy_root = guard.path().join("LegacyViewer");
        let legacy_dir = legacy_root.join("jane_doe");
        let now = conversations::now();
        let legacy_files = legacy_files_in(&legacy_dir, now);
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(legacy_root.clone());
        log.set_default_folder(default.clone());
        log.set_sharing(true);
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        read_log(&mut log);
        assert!(log.shared());
        // the Legacy Viewer's entries are listed
        assert_eq!(names_of(&log), ["Bob Smith"]);
        // an entry of this session is listed until the logout, never saved;
        // Clear is refused: the file and its backups are the Legacy Viewer's
        log.conversations.log(conversation("Carol New", 202, now - 60));
        assert_eq!(names_of(&log), ["Bob Smith", "Carol New"]);
        log.save_conversations(true);
        log.clear_conversations();
        assert_eq!(names_of(&log), ["Bob Smith", "Carol New"]);
        // the files of the session are the existing ones: the stem an entry
        // records, else the folder's naming scheme
        let bob_session = Uuid::from_u128(101);
        assert_eq!(log.conversation_history_file(bob_session, "Bob Smith"), "Bob Smith");
        assert_eq!(log.conversation_history_file(Uuid::from_u128(303), "Dana Test"), "dana_test");
        log.logout();
        assert!(log.flush(WAIT));
        assert_legacy_untouched(&legacy_root, &legacy_dir, &legacy_files);
        assert!(!default.exists());
        // the next login lists the Legacy Viewer's entries only
        log.login_named(me, JANE);
        settled(&mut log);
        read_log(&mut log);
        assert_eq!(names_of(&log), ["Bob Smith"]);
        log.logout();
        assert!(log.flush(WAIT));
        assert_legacy_untouched(&legacy_root, &legacy_dir, &legacy_files);
    }

    #[test]
    fn leaving_a_shared_folder_reads_aurora_conversation_log_again_and_keeps_legacy_files() {
        let guard = TempGuard::new();
        let aurora_root = guard.path().join("aurora");
        let legacy_root = guard.path().join("LegacyViewer");
        let legacy_dir = legacy_root.join("jane_doe");
        let now = conversations::now();
        let legacy_files = legacy_files_in(&legacy_dir, now);
        let me = Uuid::from_u128(0xA11CE);
        // Aurora's own conversation log, in its own folder
        let aurora_dir = store::folder(
            &aurora_root,
            &Identity {
                grid: "agni".into(),
                agent: me,
            },
        );
        std::fs::create_dir_all(&aurora_dir).unwrap();
        let alice = conversations::format_line(&conversation("Alice Resident", 202, now - 60));
        std::fs::write(aurora_dir.join(conversations::FILE), alice).unwrap();

        let mut log = ChatLog::in_folder(aurora_root.clone());
        log.set_default_folder(guard.path().join("default"));
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        settled(&mut log);
        read_log(&mut log);
        assert!(!log.shared());
        assert_eq!(log.dir(), Some(aurora_dir.as_path()));
        assert_eq!(names_of(&log), ["Alice Resident"]);

        // the Legacy root shared: nothing moves, its log is read
        log.change_folder(Some(legacy_root.clone()), true);
        settled(&mut log);
        read_log(&mut log);
        assert!(log.shared());
        assert_eq!(log.dir(), Some(legacy_dir.as_path()));
        assert_eq!(names_of(&log), ["Bob Smith"]);
        log.conversations.log(conversation("Transient Entry", 303, now - 30));
        log.save_conversations(true);

        // back to Aurora's folder, sharing off, in one call: placed under the
        // Aurora root at once, so its own log is read again, not an empty one
        log.change_folder(Some(aurora_root.clone()), false);
        settled(&mut log);
        read_log(&mut log);
        assert!(!log.shared());
        assert_eq!(log.dir(), Some(aurora_dir.as_path()));
        assert_eq!(names_of(&log), ["Alice Resident"]);

        // a new entry and a save keep the earlier one in Aurora's file
        log.conversations.log(conversation("Carol New", 404, now));
        log.save_conversations(true);
        assert!(log.flush(WAIT));
        let (saved, _) = conversations::parse_file(&std::fs::read(aurora_dir.join(conversations::FILE)).unwrap(), now);
        let mut saved: Vec<_> = saved.iter().map(|c| c.name.as_str()).collect();
        saved.sort_unstable();
        assert_eq!(saved, ["Alice Resident", "Carol New"]);

        // the Legacy Viewer's files as they were, and no Aurora folder in its root
        assert_legacy_untouched(&legacy_root, &legacy_dir, &legacy_files);
        assert!(!legacy_root.join(me.to_string()).exists());
    }
}
