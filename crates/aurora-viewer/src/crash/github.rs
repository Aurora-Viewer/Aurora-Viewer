//! Crash reports sent as GitHub issues in the tester's name, through the
//! project's GitHub App and its Device Flow: the tester authorizes once on
//! github.com, and neither a password nor a client secret reaches the viewer.
//! The authorization (an expiring user token and its refresh token, rotated
//! at each refresh) stays in the system credential store, never in the
//! settings, the reports or the logs. No server of our own: only the issue
//! text the tester has seen leaves the PC.

use crossbeam_channel::{Receiver, Sender};
use serde_json::Value;
use std::time::{Duration, Instant};

const API: &str = "https://api.github.com/repos/Aurora-Viewer/Aurora-Viewer/issues";
pub const ISSUES: &str = "https://github.com/Aurora-Viewer/Aurora-Viewer/issues";
pub const VERIFY: &str = "https://github.com/login/device";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
// Public identifier of the project's GitHub App (Device Flow enabled).
const CLIENT_ID: &str = "Iv23liT7PLWClMLztI1W";
const SERVICE: &str = "Aurora Viewer GitHub";
const ACCOUNT: &str = "crash-reports";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Failure {
    Network,
    Denied,
    Expired,
    Configuration,
    Invalid,
    Unauthorized,
    RateLimit,
    Ambiguous,
}

impl Failure {
    pub fn message(self) -> &'static str {
        aurora_i18n::text(match self {
            Self::Network => "bug-github-network",
            Self::Denied => "bug-github-denied",
            Self::Expired => "bug-github-expired",
            Self::Configuration => "bug-github-config",
            Self::Invalid => "bug-github-invalid",
            Self::Unauthorized => "bug-github-unauthorized",
            Self::RateLimit => "bug-github-rate-limit",
            Self::Ambiguous => "bug-github-ambiguous",
        })
    }
}

/// A user token of the GitHub App. Without token expiration (an App
/// setting), GitHub returns neither `expires_in` nor a refresh token.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Authorization {
    access: String,
    expires: u64,
    refresh: Option<String>,
    refresh_expires: u64,
}

impl Authorization {
    fn from_response(value: &Value, now: u64) -> Option<Self> {
        let token = |key: &str| value[key].as_str().filter(|s| !s.is_empty() && s.len() <= 4096).map(str::to_owned);
        let access = token("access_token")?;
        let refresh = token("refresh_token");
        Some(Self {
            access,
            expires: value["expires_in"].as_u64().map_or(u64::MAX, |s| now.saturating_add(s)),
            refresh_expires: match &refresh {
                Some(_) => now.saturating_add(value["refresh_token_expires_in"].as_u64().unwrap_or(0)),
                None => 0,
            },
            refresh,
        })
    }

    /// Five minutes of margin: a token must not expire during the request.
    fn access_valid(&self, now: u64) -> bool {
        now.saturating_add(300) < self.expires
    }

    fn refresh_valid(&self, now: u64) -> bool {
        self.refresh.is_some() && now.saturating_add(60) < self.refresh_expires
    }

    pub fn usable(&self, now: u64) -> bool {
        self.access_valid(now) || self.refresh_valid(now)
    }
}

fn entry() -> Option<keyring::Entry> {
    if aurora_updater::test_environment().is_some() {
        return None;
    }
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| log::warn!("credential store unavailable: {e}"))
        .ok()
}

fn load() -> Option<Authorization> {
    let secret = entry()?.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

fn save(authorization: &Authorization) {
    let Some(entry) = entry() else { return };
    match serde_json::to_string(authorization) {
        Ok(secret) => {
            if let Err(e) = entry.set_password(&secret) {
                log::warn!("crash report: cannot keep the GitHub authorization: {e}");
            }
        }
        Err(e) => log::warn!("crash report: cannot keep the GitHub authorization: {e}"),
    }
}

fn forget() {
    if let Some(entry) = entry()
        && let Err(e) = entry.delete_credential()
        && !matches!(e, keyring::Error::NoEntry)
    {
        log::warn!("crash report: cannot forget the GitHub authorization: {e}");
    }
}

/// Preferences › Privacy: forget it on this PC. The tester can also revoke
/// the app on github.com (Settings › Applications).
pub fn forget_authorization() {
    forget();
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |t| t.as_secs())
}

enum Reply {
    Code(String),
    Authorized(Authorization),
    Created(u64),
    Failed(Failure),
}

/// One crash window's GitHub state; the network work runs on a dedicated
/// thread, polled by the window between frames.
pub struct Session {
    authorization: Option<Authorization>,
    rx: Option<Receiver<Reply>>,
    cancel: Option<Sender<()>>,
    pub user_code: Option<String>,
    pub failure: Option<Failure>,
    pub created: Option<u64>,
    pub sending: bool,
    /// Offline demo: a simulated GitHub, no network and no credential store.
    demo: bool,
}

impl Session {
    pub fn new(demo: bool) -> Self {
        Self {
            authorization: if demo { None } else { load() },
            rx: None,
            cancel: None,
            user_code: None,
            failure: None,
            created: None,
            sending: false,
            demo,
        }
    }

    /// Demo option `connected`: as if GitHub had been authorized before.
    pub fn demo_authorize(&mut self) {
        if self.demo {
            self.authorization = Some(demo_authorization());
        }
    }

    pub fn connected(&self) -> bool {
        self.authorization.as_ref().is_some_and(|a| a.usable(now()))
    }

    pub fn authorizing(&self) -> bool {
        self.rx.is_some() && !self.sending
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Applies the worker's replies; true when something changed.
    pub fn poll(&mut self) -> bool {
        let replies: Vec<_> = self.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        let changed = !replies.is_empty();
        for reply in replies {
            match reply {
                Reply::Code(code) => self.user_code = Some(code),
                Reply::Authorized(authorization) => {
                    self.authorization = Some(authorization);
                    self.user_code = None;
                    if !self.sending {
                        self.rx = None;
                        self.cancel = None;
                    }
                }
                Reply::Created(number) => {
                    self.created = Some(number);
                    self.sending = false;
                    self.rx = None;
                }
                Reply::Failed(error) => {
                    if error == Failure::Unauthorized {
                        self.authorization = None;
                        if !self.demo {
                            forget();
                        }
                    }
                    self.failure = Some(error);
                    self.sending = false;
                    self.user_code = None;
                    self.rx = None;
                    self.cancel = None;
                }
            }
        }
        changed
    }

    pub fn authorize(&mut self) {
        if self.rx.is_some() {
            return;
        }
        self.failure = None;
        let (tx, rx) = crossbeam_channel::bounded(2);
        let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
        self.cancel = Some(cancel_tx);
        self.rx = Some(rx);
        let demo = self.demo;
        std::thread::spawn(move || {
            let result = if demo {
                simulate_authorization(&tx, &cancel_rx)
            } else {
                authorize(&tx, &cancel_rx)
            };
            if let Err(error) = result {
                let _ = tx.send(Reply::Failed(error));
            }
        });
    }

    pub fn cancel_authorization(&mut self) {
        if self.sending {
            return;
        }
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.try_send(());
        }
        self.rx = None;
        self.user_code = None;
    }

    pub fn forget(&mut self) {
        self.cancel_authorization();
        self.authorization = None;
        if !self.demo {
            forget();
        }
    }

    /// Creates the issue. Never retried automatically: a lost reply can
    /// mean the issue exists.
    pub fn send(&mut self, title: String, body: String) {
        if self.rx.is_some() || self.created.is_some() {
            return;
        }
        let Some(authorization) = self.authorization.clone().filter(|a| a.usable(now())) else {
            return;
        };
        self.failure = None;
        self.sending = true;
        let (tx, rx) = crossbeam_channel::bounded(2);
        self.rx = Some(rx);
        let demo = self.demo;
        std::thread::spawn(move || {
            let reply = if demo {
                demo_send()
            } else {
                match send(authorization, &title, &body, &tx) {
                    Ok(number) => Reply::Created(number),
                    Err(error) => Reply::Failed(error),
                }
            };
            let _ = tx.send(reply);
        });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.try_send(());
        }
    }
}

/// `AURORA_DEMO_CRASH_SEND=…,fail`: the simulated issue creation fails.
fn demo_send() -> Reply {
    std::thread::sleep(Duration::from_millis(800));
    if std::env::var("AURORA_DEMO_CRASH_SEND").is_ok_and(|v| v.split(',').any(|p| p == "fail")) {
        Reply::Failed(Failure::Network)
    } else {
        Reply::Created(4242)
    }
}

fn demo_authorization() -> Authorization {
    Authorization {
        access: "demo".into(),
        expires: u64::MAX,
        refresh: None,
        refresh_expires: 0,
    }
}

/// The demo's GitHub: a code, then the authorization a few seconds later.
fn simulate_authorization(tx: &Sender<Reply>, cancel: &Receiver<()>) -> Result<(), Failure> {
    tx.send(Reply::Code("DEMO-1234".into())).map_err(|_| Failure::Denied)?;
    if !matches!(
        cancel.recv_timeout(Duration::from_secs(3)),
        Err(crossbeam_channel::RecvTimeoutError::Timeout)
    ) {
        return Err(Failure::Denied);
    }
    tx.send(Reply::Authorized(demo_authorization())).map_err(|_| Failure::Denied)
}

fn runtime() -> Result<tokio::runtime::Runtime, Failure> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Network)
}

fn client() -> Result<reqwest::Client, Failure> {
    reqwest::Client::builder()
        .user_agent(aurora_version::user_agent())
        .https_only(true)
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| Failure::Network)
}

async fn json(mut response: reqwest::Response) -> Result<Value, Failure> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Network)? {
        if bytes.len() + chunk.len() > 256 * 1024 {
            return Err(Failure::Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::Invalid)
}

async fn post_form(client: &reqwest::Client, url: &str, pairs: &[(&str, &str)]) -> Result<Value, Failure> {
    let mut form = reqwest::Url::parse(VERIFY).expect("constant verification URL");
    form.query_pairs_mut().extend_pairs(pairs.iter().copied());
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form.query().unwrap_or_default().to_owned())
        .send()
        .await
        .map_err(|_| Failure::Network)?;
    if !response.status().is_success() {
        return Err(Failure::Network);
    }
    json(response).await
}

#[derive(Debug, PartialEq)]
enum PollState {
    Pending,
    Slower(u64),
    Failed(Failure),
}

fn poll_error(value: &Value) -> PollState {
    match value["error"].as_str() {
        Some("authorization_pending") => PollState::Pending,
        Some("slow_down") => PollState::Slower(value["interval"].as_u64().unwrap_or(0)),
        Some("access_denied") => PollState::Failed(Failure::Denied),
        Some("expired_token") => PollState::Failed(Failure::Expired),
        Some("incorrect_client_credentials" | "device_flow_disabled") => PollState::Failed(Failure::Configuration),
        // A refresh token that was revoked, rotated elsewhere or expired.
        Some("bad_refresh_token" | "unauthorized_client") => PollState::Failed(Failure::Unauthorized),
        _ => PollState::Failed(Failure::Invalid),
    }
}

fn authorize(tx: &Sender<Reply>, cancel: &Receiver<()>) -> Result<(), Failure> {
    runtime()?.block_on(async {
        let client = client()?;
        let start = post_form(&client, "https://github.com/login/device/code", &[("client_id", CLIENT_ID)]).await?;
        let device = start["device_code"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or(Failure::Configuration)?;
        let code = start["user_code"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .ok_or(Failure::Invalid)?;
        let lifetime = start["expires_in"]
            .as_u64()
            .filter(|&n| n > 0 && n <= 3600)
            .ok_or(Failure::Invalid)?;
        let deadline = Instant::now() + Duration::from_secs(lifetime);
        let mut interval = start["interval"].as_u64().unwrap_or(5).max(5);
        tx.send(Reply::Code(code.into())).map_err(|_| Failure::Denied)?;
        loop {
            // This dedicated worker sleeps; the window keeps drawing.
            let delay = Duration::from_secs(interval).min(deadline.saturating_duration_since(Instant::now()));
            if !matches!(cancel.recv_timeout(delay), Err(crossbeam_channel::RecvTimeoutError::Timeout)) {
                return Err(Failure::Denied);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Expired);
            }
            let value = post_form(
                &client,
                TOKEN,
                &[
                    ("client_id", CLIENT_ID),
                    ("device_code", device),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ],
            )
            .await?;
            if let Some(authorization) = Authorization::from_response(&value, now()) {
                save(&authorization);
                let _ = tx.send(Reply::Authorized(authorization));
                return Ok(());
            }
            match poll_error(&value) {
                PollState::Pending => {}
                PollState::Slower(server) => interval = server.max(interval.saturating_add(5)),
                PollState::Failed(error) => return Err(error),
            }
        }
    })
}

/// A Device Flow token is refreshed without the client secret. Each refresh
/// replaces both tokens: the new pair is stored before the issue is sent.
fn send(mut authorization: Authorization, title: &str, body: &str, tx: &Sender<Reply>) -> Result<u64, Failure> {
    runtime()?.block_on(async {
        let client = client()?;
        if !authorization.access_valid(now()) {
            let refresh = authorization.refresh.clone().ok_or(Failure::Unauthorized)?;
            let value = post_form(
                &client,
                TOKEN,
                &[
                    ("client_id", CLIENT_ID),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", &refresh),
                ],
            )
            .await?;
            authorization = match Authorization::from_response(&value, now()) {
                Some(renewed) => renewed,
                None => match poll_error(&value) {
                    PollState::Failed(error) => return Err(error),
                    _ => return Err(Failure::Invalid),
                },
            };
            save(&authorization);
            let _ = tx.send(Reply::Authorized(authorization.clone()));
        }
        create_request(&client, API, &authorization.access, title, body).await
    })
}

async fn create_request(client: &reqwest::Client, url: &str, token: &str, title: &str, body: &str) -> Result<u64, Failure> {
    // No labels: a tester without triage rights cannot set them, and the
    // repository's crash triage sets the type.
    let body = serde_json::to_vec(&serde_json::json!({ "title": title, "body": body })).map_err(|_| Failure::Invalid)?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|_| Failure::Ambiguous)?;
    match response.status().as_u16() {
        201 => json(response).await.map_err(|_| Failure::Ambiguous)?["number"]
            .as_u64()
            .filter(|&n| n > 0)
            .ok_or(Failure::Ambiguous),
        401 => Err(Failure::Unauthorized),
        403 | 404 => Err(Failure::Denied),
        429 => Err(Failure::RateLimit),
        422 => Err(Failure::Invalid),
        _ => Err(Failure::Ambiguous),
    }
}

/// The browser alternative: a new issue with the title and, when the URL
/// stays short enough, the body; otherwise the window copies the body.
pub fn browser_url(title: &str, body: &str) -> (String, bool) {
    let mut url = reqwest::Url::parse(&format!("{ISSUES}/new")).expect("constant project URL");
    url.query_pairs_mut().append_pair("title", title).append_pair("body", body);
    if url.as_str().len() <= 7000 {
        return (url.into(), true);
    }
    url.set_query(None);
    url.query_pairs_mut().append_pair("title", title);
    (url.into(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Real HTTP on loopback only: serialization, status handling and lost
    // replies without a GitHub account or a public issue.
    fn fixture(status: u16, response: &str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/issues", listener.local_addr().unwrap());
        let response = response.to_owned();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            if status != 0 {
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                )
                .unwrap();
            }
            String::from_utf8(request).unwrap()
        });
        (url, worker)
    }

    #[test]
    fn the_issue_is_exactly_the_previewed_text_and_replies_are_classified() {
        let rt = runtime().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let (title, preview) = (
            "Crash: Rust panic at crates/a.rs:1:2 (0.8.0)",
            "<!-- aurora-crash v1 -->\nÉchec & exemple",
        );
        for (status, reply, expected) in [
            (201, r#"{"number":123}"#, Ok(123)),
            (201, "broken response", Err(Failure::Ambiguous)),
            (401, "{}", Err(Failure::Unauthorized)),
            (403, "{}", Err(Failure::Denied)),
            (404, "{}", Err(Failure::Denied)),
            (429, "{}", Err(Failure::RateLimit)),
            (422, "{}", Err(Failure::Invalid)),
            (500, "{}", Err(Failure::Ambiguous)),
            (0, "", Err(Failure::Ambiguous)),
        ] {
            let (url, server) = fixture(status, reply);
            assert_eq!(rt.block_on(create_request(&client, &url, "test-only", title, preview)), expected);
            let request = server.join().unwrap();
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("POST /issues "));
            assert!(headers.to_ascii_lowercase().contains("authorization: bearer test-only"));
            let sent: Value = serde_json::from_str(body).unwrap();
            assert_eq!(sent, serde_json::json!({ "title": title, "body": preview }));
        }
    }

    #[test]
    fn token_responses_keep_their_lifetimes_and_refresh_rules() {
        let now = 1_000_000;
        let expiring = Authorization::from_response(
            &serde_json::json!({"access_token": "a", "expires_in": 28800, "refresh_token": "r", "refresh_token_expires_in": 15897600}),
            now,
        )
        .unwrap();
        assert!(expiring.access_valid(now));
        assert!(!expiring.access_valid(now + 28800 - 299));
        // Past the access token, the refresh token keeps it usable.
        assert!(expiring.usable(now + 30000) && expiring.refresh_valid(now + 30000));
        assert!(!expiring.usable(now + 15897600));
        // An App without token expiration: no refresh token, no end.
        let lasting = Authorization::from_response(&serde_json::json!({"access_token": "a", "token_type": "bearer"}), now).unwrap();
        assert!(lasting.usable(u64::MAX - 1000) && lasting.refresh.is_none());
        for refused in [
            serde_json::json!({"error": "authorization_pending"}),
            serde_json::json!({"access_token": ""}),
            serde_json::json!({"access_token": "x".repeat(5000)}),
        ] {
            assert!(Authorization::from_response(&refused, now).is_none());
        }
        // What the credential store keeps reads back the same.
        let stored = serde_json::to_string(&expiring).unwrap();
        assert_eq!(serde_json::from_str::<Authorization>(&stored).unwrap(), expiring);
    }

    #[test]
    fn polling_and_refresh_errors_are_classified() {
        for (error, expected) in [
            (serde_json::json!({"error":"authorization_pending"}), PollState::Pending),
            (serde_json::json!({"error":"slow_down", "interval": 15}), PollState::Slower(15)),
            (serde_json::json!({"error":"access_denied"}), PollState::Failed(Failure::Denied)),
            (serde_json::json!({"error":"expired_token"}), PollState::Failed(Failure::Expired)),
            (
                serde_json::json!({"error":"device_flow_disabled"}),
                PollState::Failed(Failure::Configuration),
            ),
            (
                serde_json::json!({"error":"bad_refresh_token"}),
                PollState::Failed(Failure::Unauthorized),
            ),
            (serde_json::json!({"unexpected": true}), PollState::Failed(Failure::Invalid)),
        ] {
            assert_eq!(poll_error(&error), expected);
        }
    }

    #[test]
    fn a_revoked_authorization_is_dropped_and_sending_needs_one() {
        let (tx, rx) = crossbeam_channel::bounded(2);
        let mut session = Session::new(true);
        assert!(!session.connected());
        // Without an authorization, sending does nothing.
        session.send("t".into(), "b".into());
        assert!(!session.sending && !session.busy());
        session.rx = Some(rx);
        tx.send(Reply::Authorized(Authorization {
            access: "test-only".into(),
            expires: u64::MAX,
            refresh: None,
            refresh_expires: 0,
        }))
        .unwrap();
        assert!(session.poll());
        assert!(session.connected() && !session.busy());
        let (tx, rx) = crossbeam_channel::bounded(2);
        session.rx = Some(rx);
        session.sending = true;
        tx.send(Reply::Failed(Failure::Unauthorized)).unwrap();
        session.poll();
        assert!(!session.connected() && !session.sending);
        assert_eq!(session.failure, Some(Failure::Unauthorized));
    }

    #[test]
    fn the_browser_link_carries_the_body_while_it_fits() {
        let (url, with_body) = browser_url("Crash: é & ?", "Rapport : ça & ceci");
        let fields: std::collections::HashMap<_, _> = reqwest::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
        assert!(with_body && fields["title"] == "Crash: é & ?" && fields["body"] == "Rapport : ça & ceci");
        let (url, with_body) = browser_url("Crash", &"é".repeat(6000));
        assert!(!with_body && url.len() < 7000 && !url.contains("body="));
    }
}
