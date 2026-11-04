//! The wait before logging in again to an account whose previous session may
//! still be on the grid (#956).
//!
//! The Legacy Viewer keeps an exec marker (llappviewer.cpp,
//! `MARKER_FILE_NAME`): created at start, holding the agent's session id once
//! logged in, deleted on a clean exit. A marker found at the next start means
//! the previous run did not end cleanly: its session id goes to the login
//! server as `last_exec_session_id`, with a `last_exec_event`
//! (lllogininstance.cpp). Aurora keeps the same facts per account, plus when
//! the session was last seen alive and when it ended. The login screen then
//! waits a moment for the grid instead of landing in a region that is still
//! logging the previous session out, where the avatar stays frozen until the
//! region's time-out. When the login server itself answers that it is still
//! logging the previous session out ("presence", "Please wait a minute
//! before you try logging in again"), the login is tried again after a
//! minute instead of leaving the user to do it.
use aurora_net::login::LastExec;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// After a session the viewer saw end (logout, lost link, kick). The region
/// answered or gave up on the circuit, then the grid still needs a few
/// seconds to drop the account's presence: the human measured a quick login
/// after a logout hanging, and about ten seconds is their figure.
pub const AFTER_END: Duration = Duration::from_secs(10);
/// After a session that ended without the viewer seeing it (a crash, a killed
/// viewer), counted from the last time it was seen alive: the region only
/// notices the dead circuit after its own time-out. The report's screenshot
/// shows a login region silent for 46 s and more, and the login after the
/// viewer's 90 s time-out going through: a minute from the last sign of life
/// (at most `HEARTBEAT` before the crash) covers that span.
pub const AFTER_SUDDEN_CLOSE: Duration = Duration::from_secs(60);
/// How often a logged-in session records that it is alive.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// The login server asked to wait (LoginFailedPendingLogout: "Please wait a
/// minute before you try logging in again").
pub const AFTER_REFUSAL: Duration = Duration::from_secs(60);
/// Automatic attempts after such refusals, before the user is left to try.
pub const MAX_AUTO_RETRIES: u32 = 3;

const FILE: &str = "last_sessions.json";

/// The last session of one account, as this PC saw it (Unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub session_id: uuid::Uuid,
    pub started: u64,
    pub alive: u64,
    /// When the viewer saw it end; `None` when it never did.
    #[serde(default)]
    pub ended: Option<u64>,
}

/// Why the login waits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The previous session ended a moment ago.
    Ended,
    /// The previous session was never seen ending.
    SuddenClose,
    /// The login server is still logging it out.
    Refused,
}

/// A login held back until `until`.
#[derive(Clone, Debug)]
pub struct Pending {
    pub until: Instant,
    pub total: Duration,
    pub reason: Reason,
    /// The grid's words, shown if the user cancels the wait.
    pub message: Option<String>,
}

impl Pending {
    pub fn new(now: Instant, wait: Duration, reason: Reason, message: Option<String>) -> Self {
        Self {
            until: now + wait,
            total: wait,
            reason,
            message,
        }
    }

    /// Whole seconds left, rounded up (the countdown never shows 0 while
    /// it still waits).
    pub fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.until.saturating_duration_since(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }

    /// The text and the fraction of the login progress while it waits.
    pub fn progress(&self, now: Instant) -> (String, f32) {
        let seconds = self.seconds_left(now) as i64;
        let text = match self.reason {
            Reason::Ended | Reason::SuddenClose => aurora_i18n::tr!("ui-login-previous-session-ending", seconds = seconds),
            Reason::Refused => aurora_i18n::tr!("ui-login-previous-session-retry", seconds = seconds),
        };
        let done = 1.0 - self.until.saturating_duration_since(now).as_secs_f32() / self.total.as_secs_f32().max(0.001);
        (text, 0.02 + 0.06 * done.clamp(0.0, 1.0))
    }
}

/// How long a login to this account still waits, and why.
pub fn wait(session: Option<&Session>, now: u64) -> Option<(Duration, Reason)> {
    let session = session?;
    let (at, span, reason) = match session.ended {
        Some(ended) => (ended, AFTER_END, Reason::Ended),
        None => (session.alive, AFTER_SUDDEN_CLOSE, Reason::SuddenClose),
    };
    // A clock set back puts the record in the future: nothing tells how
    // long ago it really was, and waiting would repeat at every login.
    let elapsed = Duration::from_secs(now.checked_sub(at)?);
    span.checked_sub(elapsed).filter(|left| !left.is_zero()).map(|left| (left, reason))
}

/// What the login server is told of the previous session (LastExec).
pub fn last_exec(session: Option<&Session>) -> LastExec {
    let Some(session) = session else {
        return LastExec::default();
    };
    let duration = |end: u64| i32::try_from(end.saturating_sub(session.started)).unwrap_or(i32::MAX);
    match session.ended {
        Some(ended) => LastExec {
            event: LastExec::NORMAL,
            duration: duration(ended),
            session_id: uuid::Uuid::nil(),
        },
        None => LastExec {
            event: LastExec::UNKNOWN,
            duration: duration(session.alive),
            session_id: session.session_id,
        },
    }
}

/// The key of an account: its grid and its user name, as the catalogue of
/// the login screen (`login_accounts::Account::key`).
fn entry(key: &(String, String)) -> String {
    format!("{}\n{}", key.0.trim(), key.1.trim().to_lowercase())
}

fn read(path: &Path) -> BTreeMap<String, Session> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 256 * 1024 => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            log::warn!("{FILE} unreadable, ignored: {error}");
            BTreeMap::new()
        }),
        Ok(_) => {
            log::warn!("{FILE} too large, ignored");
            BTreeMap::new()
        }
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("{FILE} unreadable, ignored: {error}");
            }
            BTreeMap::new()
        }
    }
}

fn change_at(path: &Path, key: &(String, String), update: impl FnOnce(&mut Option<Session>)) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // several viewers can run at once, on other accounts
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut all = read(path);
    let name = entry(key);
    let mut session = all.remove(&name);
    update(&mut session);
    if let Some(session) = session {
        all.insert(name, session);
    }
    let bytes = serde_json::to_vec_pretty(&all).map_err(std::io::Error::other)?;
    crate::atomic_file::write(path, &bytes)
}

fn path() -> PathBuf {
    crate::settings::config_dir().join(FILE)
}

fn change(key: &(String, String), update: impl FnOnce(&mut Option<Session>)) {
    if let Err(error) = change_at(&path(), key, update) {
        log::warn!("{FILE} not saved: {error}");
    }
}

/// The last session of this account.
pub fn load(key: &(String, String)) -> Option<Session> {
    read(&path()).remove(&entry(key))
}

/// A session of this account began.
pub fn started(key: &(String, String), session_id: uuid::Uuid, now: u64) {
    change(key, |session| {
        *session = Some(Session {
            session_id,
            started: now,
            alive: now,
            ended: None,
        })
    });
}

/// The session is still alive (`HEARTBEAT`), written on a thread of its own:
/// the heartbeat comes from the frame, and the write waits for the disk
/// (atomic_file.rs, #1809).
pub fn alive(key: &(String, String), now: u64) {
    let key = key.clone();
    let spawned = std::thread::Builder::new()
        .name("aurora-login-alive".into())
        .spawn(move || change(&key, |session| touch(session, now)));
    if let Err(e) = spawned {
        log::warn!("{FILE}: no thread for the sign of life: {e}");
    }
}

/// A sign of life written late never moves `alive` back (a session that
/// started since) nor reopens one that ended.
fn touch(session: &mut Option<Session>, now: u64) {
    if let Some(session) = session.as_mut().filter(|session| session.ended.is_none()) {
        session.alive = session.alive.max(now);
    }
}

/// The viewer saw the session end.
pub fn ended(key: &(String, String), now: u64) {
    change(key, |session| {
        if let Some(session) = session.as_mut().filter(|session| session.ended.is_none()) {
            session.alive = now;
            session.ended = Some(now);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(alive: u64, ended: Option<u64>) -> Session {
        Session {
            session_id: uuid::Uuid::from_u128(5),
            started: 100,
            alive,
            ended,
        }
    }

    #[test]
    fn a_clean_logout_waits_ten_seconds_from_its_end() {
        let s = session(1000, Some(1000));
        assert_eq!(wait(Some(&s), 1000), Some((AFTER_END, Reason::Ended)));
        assert_eq!(wait(Some(&s), 1003), Some((Duration::from_secs(7), Reason::Ended)));
        assert_eq!(wait(Some(&s), 1010), None);
        assert_eq!(wait(Some(&s), 5000), None);
    }

    #[test]
    fn a_sudden_close_waits_a_minute_from_the_last_sign_of_life() {
        let s = session(1000, None);
        assert_eq!(wait(Some(&s), 1005), Some((Duration::from_secs(55), Reason::SuddenClose)));
        assert_eq!(wait(Some(&s), 1059), Some((Duration::from_secs(1), Reason::SuddenClose)));
        assert_eq!(wait(Some(&s), 1060), None);
    }

    #[test]
    fn no_record_or_a_clock_set_back_never_waits() {
        assert_eq!(wait(None, 1000), None);
        assert_eq!(wait(Some(&session(1000, None)), 900), None);
        assert_eq!(wait(Some(&session(1000, Some(1000))), 999), None);
    }

    #[test]
    fn the_session_id_goes_to_the_login_server_only_after_an_unclean_end() {
        assert_eq!(last_exec(None), LastExec::default());
        let crashed = last_exec(Some(&session(160, None)));
        assert_eq!(crashed.event, LastExec::UNKNOWN);
        assert_eq!(crashed.duration, 60);
        assert_eq!(crashed.session_id, uuid::Uuid::from_u128(5));
        let clean = last_exec(Some(&session(400, Some(400))));
        assert_eq!(clean.event, LastExec::NORMAL);
        assert_eq!(clean.duration, 300);
        assert!(clean.session_id.is_nil());
    }

    #[test]
    fn each_account_keeps_its_own_session_on_disk_across_a_restart() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/unit-test-user-data")
            .join(format!("login-wait-{}", uuid::Uuid::new_v4()));
        let path = dir.join(FILE);
        let alpha = ("https://grid.example/login".to_owned(), "Alpha Tester".to_owned());
        let beta = ("https://grid.example/login".to_owned(), "beta".to_owned());
        let id = uuid::Uuid::from_u128(9);
        change_at(&path, &alpha, |s| {
            *s = Some(Session {
                session_id: id,
                started: 10,
                alive: 10,
                ended: None,
            })
        })
        .unwrap();
        change_at(&path, &alpha, |s| s.as_mut().unwrap().alive = 25).unwrap();
        // a new run reads what the previous one left: the crash of alpha
        let after_restart = read(&path);
        let alpha_session = after_restart.get(&entry(&("https://grid.example/login ".into(), "alpha tester".into())));
        assert_eq!(alpha_session.map(|s| (s.alive, s.ended)), Some((25, None)));
        assert_eq!(wait(alpha_session, 30), Some((Duration::from_secs(55), Reason::SuddenClose)));
        // the other account is not held back
        assert!(!after_restart.contains_key(&entry(&beta)));
        assert_eq!(wait(after_restart.get(&entry(&beta)), 30), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_countdown_rounds_up_and_fills_its_bar() {
        let start = Instant::now();
        let pending = Pending::new(start, Duration::from_secs(10), Reason::Ended, None);
        assert_eq!(pending.seconds_left(start), 10);
        assert_eq!(pending.seconds_left(start + Duration::from_millis(9_100)), 1);
        assert_eq!(pending.seconds_left(start + Duration::from_secs(10)), 0);
        let (_, first) = pending.progress(start);
        let (_, last) = pending.progress(start + Duration::from_secs(10));
        assert!(first < last);
    }

    #[test]
    fn a_late_sign_of_life_never_moves_back_nor_reopens_a_session() {
        let mut current = Some(session(200, None));
        touch(&mut current, 215);
        assert_eq!(current, Some(session(215, None)));
        // written after the next session started (alive 300)
        let mut next = Some(session(300, None));
        touch(&mut next, 230);
        assert_eq!(next, Some(session(300, None)));
        // written after the session ended
        let mut ended = Some(session(250, Some(250)));
        touch(&mut ended, 260);
        assert_eq!(ended, Some(session(250, Some(250))));
        let mut none = None;
        touch(&mut none, 260);
        assert_eq!(none, None);
    }
}
