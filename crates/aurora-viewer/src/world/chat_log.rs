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
//! - `format`: the text of a line and its reading back;
//! - `names`: the file of each conversation;
//! - `store`: the folder, the thread that writes and reads;
//! - `transcript`: a whole file for the History window, by pages.

pub mod format;
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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            log_im: true,
            log_nearby: false,
            show_history: true,
            dated_names: false,
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

struct Open {
    identity: Identity,
    dir: PathBuf,
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
    pub fn login(&mut self, agent: Uuid) {
        self.epoch += 1;
        self.open = None;
        if agent.is_nil() || self.grid.is_empty() {
            log::info!("conversation transcripts: off for this session (no account or no grid)");
            return;
        }
        let identity = Identity {
            grid: self.grid.clone(),
            agent,
        };
        let accounts = self.accounts.clone().unwrap_or_else(crate::settings::accounts_dir);
        let dir = store::folder(&accounts, &identity);
        self.open = Some(Open { identity, dir });
    }

    /// Logged out: nothing more is written or read, and what was being read
    /// for that account is dropped. The lines already queued keep the path
    /// they were queued with.
    pub fn logout(&mut self) {
        self.epoch += 1;
        self.open = None;
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
    pub fn erase(&mut self) -> bool {
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
        if self.moving.is_some() {
            return false;
        }
        let Some(open) = &self.open else {
            self.accounts = accounts;
            return false;
        };
        let root = accounts.clone().unwrap_or_else(crate::settings::accounts_dir);
        let to = store::folder(&root, &open.identity);
        if to == open.dir {
            self.accounts = accounts;
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
