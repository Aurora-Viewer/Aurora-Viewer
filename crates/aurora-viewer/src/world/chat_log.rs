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
//! With the Legacy layout (an Aurora option, off by default) and a chosen
//! folder, the account's files are `<folder>/<first_last>` instead, the
//! Legacy Viewer's own (`legacy_profile::Account::chat_folder`): both
//! viewers then read and add to the same files. That folder is shared, so
//! Aurora only appends lines to it: no import, merge, move or erasing
//! there (`shared`).
//!
//! - `format`: the text of a line and its reading back;
//! - `names`: the file of each conversation;
//! - `store`: the folder, the thread that writes and reads;
//! - `transcript`: a whole file for the History window, by pages;
//! - `import`: a Legacy Viewer's transcripts brought into the account's
//!   folder, adding only;
//! - `conversations`: the conversation log (`conversation.log` in the same
//!   folder), who we talked with and when.

pub mod conversations;
pub mod format;
pub mod import;
pub mod names;
pub mod store;
pub mod transcript;

pub use format::Entry;
pub use store::{Identity, Loaded};

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

/// The account's folder in the Legacy layout, None on a grid without one.
/// The names come from the server: what a file system refuses becomes '_'.
fn legacy_folder(first: &str, last: &str, grid: &str) -> Option<String> {
    import::Account {
        first: names::clean_file_name(first),
        last: names::clean_file_name(last),
        grid: grid.to_owned(),
    }
    .chat_folder()
}

struct Open {
    identity: Identity,
    dir: PathBuf,
    /// The account's folder in the Legacy layout, when the login gave its
    /// name and the grid has one.
    legacy: Option<String>,
    /// `dir` is the Legacy layout's folder.
    shared: bool,
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
    /// The files of the account in the Legacy Viewer's layout under the
    /// chosen folder (Preferences › Chat, off by default).
    legacy_layout: bool,
    /// A move to another folder asked for, until its answer comes, with the
    /// folder to go back to if it fails.
    moving: Option<(crossbeam_channel::Receiver<std::io::Result<usize>>, Option<PathBuf>)>,
    open: Option<Open>,
    /// Counts the logins and logouts: what was read for another one is
    /// thrown away.
    epoch: u64,
    worker: Option<store::Worker>,
    /// An erasing asked for, until its answer comes.
    erasing: Option<crossbeam_channel::Receiver<std::io::Result<usize>>>,
    /// The Legacy Viewers' folders, when they are not the system's (tests,
    /// demo).
    legacy_apps: Option<Vec<PathBuf>>,
    /// An import under way.
    importing: Option<import::Running>,
    /// The conversation log of the account logged in.
    pub conversations: conversations::Log,
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

    /// The grid the next login is made on.
    pub fn set_grid(&mut self, grid: String) {
        if self.open.is_none() {
            self.grid = grid;
        }
    }

    /// The account is known: its folder becomes the one written and read.
    /// Without its name, the Legacy layout cannot be used (tests).
    #[cfg(test)]
    pub fn login(&mut self, agent: Uuid) {
        self.login_named(agent, None);
    }

    /// `login`, with the account's first and last names of the login.
    pub fn login_named(&mut self, agent: Uuid, name: Option<(&str, &str)>) {
        self.epoch += 1;
        self.open = None;
        self.conversations.reset();
        if agent.is_nil() || self.grid.is_empty() {
            log::info!("conversation transcripts: off for this session (no account or no grid)");
            return;
        }
        let identity = Identity {
            grid: self.grid.clone(),
            agent,
        };
        let legacy = name.and_then(|(first, last)| legacy_folder(first, last, &self.grid));
        let (dir, shared) = self.place(self.accounts.as_deref(), &identity, legacy.as_deref());
        self.open = Some(Open {
            identity,
            dir,
            legacy,
            shared,
        });
        // LLConversationLog::initLoggingState
        if self.cfg.log_conversations {
            self.start_conversations();
        }
    }

    /// The folder of an account under the folder of the accounts (None: the
    /// default one), and whether it is the Legacy layout's.
    fn place(&self, accounts: Option<&Path>, identity: &Identity, legacy: Option<&str>) -> (PathBuf, bool) {
        if self.legacy_layout
            && let (Some(root), Some(name)) = (accounts, legacy)
        {
            return (root.join(name), true);
        }
        let root = accounts.map(Path::to_path_buf).unwrap_or_else(crate::settings::accounts_dir);
        (store::folder(&root, identity), false)
    }

    /// The Legacy layout on or off (Preferences › Chat). Logged in, the next
    /// lines go to the other folder and nothing moves: the account's Aurora
    /// files stay where they are, and come back with the option off. Waits
    /// while an import, a move or an erasing is under way (asked again by
    /// the next frame).
    pub fn set_legacy_layout(&mut self, on: bool) {
        if on == self.legacy_layout || self.busy() {
            return;
        }
        self.legacy_layout = on;
        let Some(open) = &self.open else {
            return;
        };
        let (dir, shared) = self.place(self.accounts.as_deref(), &open.identity, open.legacy.as_deref());
        if let Some(open) = &mut self.open {
            (open.dir, open.shared) = (dir, shared);
        }
    }

    /// The account's files are in the Legacy layout, in a folder the Legacy
    /// Viewer reads and writes too: Aurora only appends lines there.
    pub fn shared(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.shared)
    }

    fn conversations_file(&self) -> Option<PathBuf> {
        Some(self.dir()?.join(conversations::FILE))
    }

    /// enableLogging: the file read again, the session's entries after it.
    fn start_conversations(&mut self) {
        let reading = match self.conversations_file() {
            Some(path) => self.worker().map(|w| w.read(path)),
            None => None,
        };
        self.conversations.start(reading);
    }

    /// Write the conversation log if it changed: at most every few seconds,
    /// at once with `now` (logout, exit, the log cleared). Not while the
    /// transcripts move, but for the last save of a logout.
    pub fn save_conversations(&mut self, now: bool) {
        if self.moving.is_some() && !now {
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

    /// Every frame: the file read, the setting followed (enableLogging on a
    /// change of KeepConversationLogTranscripts), the changes written.
    pub fn poll_conversations(&mut self) {
        if self.open.is_none() {
            return;
        }
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
    /// backups removed.
    pub fn clear_conversations(&mut self) {
        self.conversations.clear();
        self.save_conversations(true);
        if let Some(dir) = self.dir().map(Path::to_path_buf)
            && let Some(w) = self.worker()
        {
            w.remove_backups(dir);
        }
    }

    /// Logged out: nothing more is written or read, and what was being read
    /// for that account is dropped. The lines already queued keep the path
    /// they were queued with.
    pub fn logout(&mut self) {
        self.epoch += 1;
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

    /// Folder of the account logged in.
    pub fn dir(&self) -> Option<&Path> {
        self.open.as_ref().map(|o| o.dir.as_path())
    }

    fn today(&self) -> Option<(i64, i64, i64)> {
        self.cfg.dated_names.then(names::local_date)
    }

    /// File a conversation is written to now.
    pub fn file_of(&self, stem: &str) -> Option<PathBuf> {
        names::file_path(self.dir()?, stem, self.today())
    }

    /// File a conversation's history is read from: the one written now, or
    /// with dated names the latest that exists. Looks at the disk: not for
    /// every frame.
    pub fn history_file_of(&self, stem: &str) -> Option<PathBuf> {
        names::existing_file(self.dir()?, stem, self.today())
    }

    fn worker(&mut self) -> Option<&store::Worker> {
        if self.worker.is_none() {
            self.worker = store::Worker::start();
        }
        self.worker.as_ref()
    }

    /// Append a line to a conversation's file (LLLogChat::saveHistory). An
    /// empty `from` is a system line. Nothing happens while logged out.
    pub fn append(&mut self, stem: &str, time: SystemTime, from: &str, text: &str) {
        let Some(path) = self.file_of(stem) else {
            return;
        };
        let from = if from.trim().is_empty() { format::SYSTEM_FROM } else { from };
        let line = format::format_line(Some(&format::stamp(time, true)), from, text);
        let mut text = if store::EOL == "\n" { line } else { line.replace('\n', store::EOL) };
        text.push_str(store::EOL);
        if let Some(w) = self.worker() {
            w.append(path, text);
        }
    }

    /// Ask for the end of a conversation's file (LLIMSession::loadHistory);
    /// it comes back through `take_loaded`. False when nothing was asked
    /// (logged out, or the history is not shown).
    pub fn request_history(&mut self, conversation: Uuid, stem: &str) -> bool {
        if !self.cfg.show_history {
            return false;
        }
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return false;
        };
        let (today, epoch) = (self.today(), self.epoch);
        match self.worker() {
            Some(w) => {
                w.tail(dir, stem.to_owned(), today, conversation, epoch);
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

    /// Erase the transcripts of the account logged in (on this grid), after
    /// the lines already queued. False when there is no account. The answer
    /// comes through `take_erased`.
    /// An import, a move or an erasing is under way: another one would be
    /// refused.
    pub fn busy(&self) -> bool {
        self.importing.is_some() || self.moving.is_some() || self.erasing.is_some()
    }

    /// Refused in the Legacy layout: the files are the Legacy Viewer's too.
    pub fn erase(&mut self) -> bool {
        if self.importing.is_some() || self.shared() {
            return false;
        }
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return false;
        };
        self.erasing = self.worker().map(|w| w.erase(dir));
        self.erasing.is_some()
    }

    /// The folder chosen in the preferences (None: the default one), for the
    /// next login.
    pub fn set_folder(&mut self, accounts: Option<PathBuf>) {
        if self.moving.is_none() {
            self.accounts = accounts;
        }
    }

    /// Another folder for the transcripts while logged in: the account's
    /// files follow, after the lines already queued, and the next lines go
    /// to the new folder (moveTranscriptsAndLog). False when nothing moves.
    /// The answer comes through `take_relocated`.
    pub fn relocate(&mut self, accounts: Option<PathBuf>) -> bool {
        if self.moving.is_some() || self.importing.is_some() {
            return false;
        }
        let Some(open) = &self.open else {
            self.accounts = accounts;
            return false;
        };
        let (to, shared) = self.place(accounts.as_deref(), &open.identity, open.legacy.as_deref());
        // a folder in the Legacy layout is the Legacy Viewer's too: its
        // files never move, in or out; the next lines go to the new folder
        if to == open.dir || shared || open.shared {
            self.accounts = accounts;
            if let Some(open) = &mut self.open {
                (open.dir, open.shared) = (to, shared);
            }
            return false;
        }
        let from = open.dir.clone();
        let Some(done) = self.worker().map(|w| w.relocate(from, to.clone())) else {
            return false;
        };
        let previous = std::mem::replace(&mut self.accounts, accounts);
        if let Some(open) = &mut self.open {
            open.dir = to;
        }
        self.moving = Some((done, previous));
        true
    }

    /// The answer of the last move, once: the number of files moved. On a
    /// failure the transcripts stay in (and go back to) the previous folder.
    pub fn take_relocated(&mut self) -> Option<std::io::Result<usize>> {
        let done = self.moving.as_ref()?.0.try_recv().ok()?;
        let (_, previous) = self.moving.take()?;
        if done.is_err() {
            let root = previous.clone().unwrap_or_else(crate::settings::accounts_dir);
            if let Some(open) = &mut self.open {
                open.dir = store::folder(&root, &open.identity);
            }
            self.accounts = previous;
        }
        Some(done)
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
    /// would see which were read.
    pub fn import_found(&mut self, account: import::Account) -> bool {
        let kinds = import::Kinds {
            im: self.cfg.log_im,
            nearby: self.cfg.log_nearby,
        };
        if !account.grid_known() || !kinds.any() {
            return false;
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
        let done = self.erasing.as_ref()?.try_recv().ok()?;
        self.erasing = None;
        Some(done)
    }

    /// Wait for the queued lines to be on disk (exit, tests).
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
        log.set_grid("agni".into());
        log.set_legacy_layout(true);
        log.login_named(me, JANE);
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
    fn turning_the_legacy_layout_on_and_off_moves_nothing() {
        let root = temp();
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_grid("agni".into());
        log.login_named(me, JANE);
        assert!(!log.shared());
        let aurora = log.dir().unwrap().to_path_buf();
        log.append("Bob Smith", now(), "Bob Smith", "before");
        log.set_legacy_layout(true);
        assert!(log.shared());
        log.append("Bob Smith", now(), "Bob Smith", "shared");
        log.set_legacy_layout(false);
        assert_eq!(log.dir(), Some(aurora.as_path()));
        log.append("Bob Smith", now(), "Bob Smith", "after");
        assert!(log.flush(WAIT));
        assert_eq!(texts(&aurora.join("Bob Smith.txt")), ["before", "after"]);
        assert_eq!(texts(&root.join("jane_doe").join("Bob Smith.txt")), ["shared"]);
        // another folder chosen: the shared files stay with the Legacy Viewer
        log.set_legacy_layout(true);
        let other = root.join("other");
        assert!(!log.relocate(Some(other.clone())));
        assert_eq!(log.dir(), Some(other.join("jane_doe").as_path()));
        assert!(log.flush(WAIT));
        assert_eq!(texts(&root.join("jane_doe").join("Bob Smith.txt")), ["shared"]);
        // back to the default folder: Aurora's layout, still no move
        assert!(!log.relocate(None));
        assert!(!log.shared());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_legacy_layout_needs_a_name_and_a_known_grid() {
        let root = temp();
        let me = Uuid::from_u128(0xA11CE);
        let mut log = ChatLog::in_folder(root.clone());
        log.set_legacy_layout(true);
        log.set_grid("aditi".into());
        log.login_named(me, JANE);
        assert_eq!(log.dir(), Some(root.join("jane_doe.second_life_beta").as_path()));
        log.logout();
        // another grid's label is the Legacy Viewer's own: Aurora's layout
        log.set_grid("custom_osgrid_org".into());
        log.login_named(me, JANE);
        assert!(!log.shared());
        log.logout();
        log.set_grid("agni".into());
        log.login(me);
        assert!(!log.shared());
        // a name never leaves the chosen folder
        log.logout();
        log.login_named(me, Some(("..", "x/y")));
        let dir = log.dir().unwrap();
        assert_eq!(dir.parent(), Some(root.as_path()));
        std::fs::remove_dir_all(&root).unwrap();
        // without a chosen folder: Aurora's own layout (nothing is written)
        let mut default = ChatLog::default();
        default.set_grid("agni".into());
        default.set_legacy_layout(true);
        default.login_named(me, JANE);
        assert!(!default.shared());
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
}
