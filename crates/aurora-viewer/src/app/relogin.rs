//! Logging in again to an account whose previous session may still be on the
//! grid (#956): the login waits for it on the login screen, then starts by
//! itself (login_wait.rs).
use super::App;
use crate::login_wait::{self, Pending, Reason};
use aurora_net::login::LastExec;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Relogin {
    /// The login held back, with its countdown.
    pending: Option<Pending>,
    /// The next `start_login` goes through: its wait just ended.
    skip: bool,
    /// Automatic attempts after a "presence" refusal in a row.
    retries: u32,
    /// The account of the running session, and its last heartbeat.
    session: Option<((String, String), Instant)>,
}

impl App {
    fn login_key(&self) -> (String, String) {
        (self.settings.login_uri(), self.settings.username.trim().to_lowercase())
    }

    /// The last session of the account about to log in (the demo's made-up
    /// one with `AURORA_DEMO_LOGIN_WAIT`).
    fn previous_session(&self) -> Option<login_wait::Session> {
        if self.demo {
            return demo_session();
        }
        login_wait::load(&self.login_key())
    }

    /// What the login server is told of that session.
    pub(super) fn last_exec(&self) -> LastExec {
        login_wait::last_exec(self.previous_session().as_ref())
    }

    /// `start_login`: true when the login waits for the previous session
    /// first (it starts again by itself, `poll_login_wait`).
    pub(super) fn hold_login(&mut self) -> bool {
        if std::mem::take(&mut self.relogin.skip) {
            return false;
        }
        let Some((wait, reason)) = login_wait::wait(self.previous_session().as_ref(), crate::credentials::now()) else {
            return false;
        };
        log::info!("login waits {} s for the previous session ({reason:?})", wait.as_secs());
        self.begin_login_wait(Pending::new(Instant::now(), wait, reason, None));
        true
    }

    fn begin_login_wait(&mut self, pending: Pending) {
        self.login_form.busy = true;
        self.login_form.error = None;
        self.login_progress = Some(pending.progress(Instant::now()));
        self.relogin.pending = Some(pending);
    }

    /// Each frame of the login screen: the countdown, Escape to cancel it,
    /// and the login once it is over.
    pub(super) fn poll_login_wait(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.relogin.pending else {
            return;
        };
        let now = Instant::now();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            log::info!("login wait cancelled");
            let message = pending.message.clone();
            self.relogin.pending = None;
            self.relogin.retries = 0;
            self.login_form.busy = false;
            self.login_form.transition = None;
            self.login_progress = None;
            self.login_form.error = message;
            return;
        }
        if now >= pending.until {
            self.relogin.pending = None;
            self.relogin.skip = true;
            self.login_progress = None;
            self.start_login();
            return;
        }
        self.login_progress = Some(pending.progress(now));
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// The login server is still logging out a previous session of this
    /// account: tried again after a minute, `MAX_AUTO_RETRIES` times in a
    /// row. False when the refusal is to be shown as it is.
    pub(super) fn retry_after_refusal(&mut self, message: &str) -> bool {
        if self.relogin.retries >= login_wait::MAX_AUTO_RETRIES {
            self.relogin.retries = 0;
            return false;
        }
        self.relogin.retries += 1;
        log::info!(
            "login refused, the grid is still logging out the previous session: try {} in {} s",
            self.relogin.retries,
            login_wait::AFTER_REFUSAL.as_secs()
        );
        let message = (!message.trim().is_empty()).then(|| message.to_owned());
        self.begin_login_wait(Pending::new(Instant::now(), login_wait::AFTER_REFUSAL, Reason::Refused, message));
        true
    }

    /// LoggedIn: the session is recorded for the next login to this account.
    pub(super) fn session_started(&mut self, session_id: uuid::Uuid) {
        self.relogin.retries = 0;
        let key = self.login_key();
        if !self.demo {
            login_wait::started(&key, session_id, crate::credentials::now());
        }
        self.relogin.session = Some((key, Instant::now()));
    }

    /// While logged in: the session's sign of life, every `HEARTBEAT`.
    pub(super) fn session_heartbeat(&mut self, now: Instant) {
        let demo = self.demo;
        let Some((key, last)) = &mut self.relogin.session else {
            return;
        };
        if now.duration_since(*last) < login_wait::HEARTBEAT {
            return;
        }
        *last = now;
        if !demo {
            login_wait::alive(key, crate::credentials::now());
        }
    }

    /// The viewer saw the session end (logout, lost link, kick).
    pub(super) fn session_ended(&mut self) {
        if let Some((key, _)) = self.relogin.session.take()
            && !self.demo
        {
            login_wait::ended(&key, crate::credentials::now());
        }
    }

    /// `AURORA_DEMO_LOGIN_WAIT=refused`: the offline login server answers as
    /// the grid does while it logs out a previous session, once.
    pub(super) fn demo_refusal(&mut self) -> bool {
        if std::env::var("AURORA_DEMO_LOGIN_WAIT").as_deref() != Ok("refused") || self.relogin.retries > 0 {
            return false;
        }
        log::info!("demo login: refused, a previous session is still on the grid");
        self.on_app_event(aurora_net::NetEvent::LoginFailed {
            error: aurora_i18n::tr!("app-demo-login-pending-logout").into(),
            mfa_required: false,
            tos_required: false,
            bad_credentials: false,
            previous_session: true,
        });
        true
    }
}

/// `AURORA_DEMO_LOGIN_WAIT=logout|sudden`: the demo account's previous
/// session logged out, or was never seen ending, when the viewer started.
fn demo_session() -> Option<login_wait::Session> {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let mode = std::env::var("AURORA_DEMO_LOGIN_WAIT").ok()?;
    let at = *START.get_or_init(crate::credentials::now);
    let ended = match mode.as_str() {
        "logout" => Some(at),
        "sudden" => None,
        _ => return None,
    };
    Some(login_wait::Session {
        session_id: uuid::Uuid::from_u128(0xa0e0_a6e1_0000_0000_0000_0000_0000_0956),
        started: at.saturating_sub(600),
        alive: at,
        ended,
    })
}
