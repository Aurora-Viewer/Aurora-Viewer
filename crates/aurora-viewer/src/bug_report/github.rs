//! Browser links and optional GitHub App device authorization. Passwords
//! stay in the user's browser. Access tokens live only in this module's
//! memory for the current viewer session, never in reports/settings/logs.

use crossbeam_channel::{Receiver, Sender};
use serde_json::Value;
use std::time::{Duration, Instant};

pub const ISSUES: &str = "https://github.com/Aurora-Viewer/Aurora-Viewer/issues";
const API: &str = "https://api.github.com/repos/Aurora-Viewer/Aurora-Viewer/issues";
pub const VERIFY: &str = "https://github.com/login/device";
// Public identifier, supplied by the project's GitHub App registration.
pub const CLIENT_ID: &str = "Iv23liT7PLWClMLztI1W";

pub struct BrowserDraft {
    pub url: String,
    pub paste_body: bool,
}

pub fn browser_draft(title: &str, body: &str) -> BrowserDraft {
    let mut url = reqwest::Url::parse(&format!("{ISSUES}/new")).expect("constant project URL");
    url.query_pairs_mut()
        .append_pair("template", "bug_report.md")
        .append_pair("labels", "bug,alpha")
        .append_pair("title", title)
        .append_pair("body", body);
    // Percent encoding grows non-ASCII text substantially. Preserve the full
    // body in the clipboard/local file rather than silently truncating it.
    let paste_body = url.as_str().len() > 7000;
    if paste_body {
        url.set_query(None);
        url.query_pairs_mut()
            .append_pair("template", "bug_report.md")
            .append_pair("labels", "bug,alpha")
            .append_pair("title", title);
    }
    BrowserDraft {
        url: url.into(),
        paste_body,
    }
}

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

enum Reply {
    Code(String),
    Authorized(String, Instant),
    Created(u64),
    Failed(Failure),
}

#[derive(Default)]
pub struct Github {
    token: Option<(String, Instant)>,
    rx: Option<Receiver<Reply>>,
    cancel: Option<Sender<()>>,
    pub user_code: Option<String>,
    pub failure: Option<Failure>,
    pub created: Option<u64>,
    pub sending: bool,
}

impl Github {
    pub fn connected(&self) -> bool {
        self.token.as_ref().is_some_and(|(_, expires)| *expires > Instant::now())
    }
    pub fn authorizing(&self) -> bool {
        self.rx.is_some() && !self.sending
    }

    pub fn poll(&mut self) {
        let replies: Vec<_> = self.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        for reply in replies {
            match reply {
                Reply::Code(code) => self.user_code = Some(code),
                Reply::Authorized(token, expires) => {
                    self.token = Some((token, expires));
                    self.user_code = None;
                    self.rx = None;
                    self.cancel = None;
                }
                Reply::Created(number) => {
                    self.created = Some(number);
                    self.sending = false;
                    self.rx = None;
                }
                Reply::Failed(error) => {
                    if error == Failure::Unauthorized {
                        self.token = None;
                    }
                    self.failure = Some(error);
                    self.sending = false;
                    self.user_code = None;
                    self.rx = None;
                    self.cancel = None;
                }
            }
        }
        if self.token.as_ref().is_some_and(|(_, until)| *until <= Instant::now()) {
            self.token = None;
        }
    }

    pub fn authorize(&mut self) {
        if self.rx.is_some() {
            return;
        }
        self.failure = None;
        if CLIENT_ID.is_empty() {
            self.failure = Some(Failure::Configuration);
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(2);
        let (cancel_tx, cancel_rx) = crossbeam_channel::bounded(1);
        self.cancel = Some(cancel_tx);
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let result = authorize(&tx, &cancel_rx);
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

    pub fn disconnect(&mut self) {
        self.cancel_authorization();
        self.token = None;
    }

    pub fn submit(&mut self, title: String, body: String) {
        if self.rx.is_some() || !self.connected() {
            return;
        }
        let Some((token, _)) = &self.token else {
            return;
        };
        let token = token.clone();
        self.failure = None;
        self.created = None;
        self.sending = true;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let reply = match create_issue(&token, title, body) {
                Ok(n) => Reply::Created(n),
                Err(e) => Reply::Failed(e),
            };
            let _ = tx.send(reply);
        });
    }
}

impl Drop for Github {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.try_send(());
        }
    }
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
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(Failure::Invalid);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Failure::Invalid)
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut url = reqwest::Url::parse(VERIFY).expect("constant verification URL");
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url.query().unwrap_or_default().to_owned()
}

async fn post_form(client: &reqwest::Client, url: &str, pairs: &[(&str, &str)]) -> Result<Value, Failure> {
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(pairs))
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
        _ => PollState::Failed(Failure::Invalid),
    }
}

fn authorize(tx: &Sender<Reply>, cancel: &Receiver<()>) -> Result<(), Failure> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Network)?;
    rt.block_on(async {
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
            if Instant::now() >= deadline {
                return Err(Failure::Expired);
            }
            // This dedicated worker sleeps; the frame and runtime of SL do not.
            let delay = Duration::from_secs(interval).min(deadline.saturating_duration_since(Instant::now()));
            if !matches!(cancel.recv_timeout(delay), Err(crossbeam_channel::RecvTimeoutError::Timeout)) {
                return Err(Failure::Denied);
            }
            if Instant::now() >= deadline {
                return Err(Failure::Expired);
            }
            let value = post_form(
                &client,
                "https://github.com/login/oauth/access_token",
                &[
                    ("client_id", CLIENT_ID),
                    ("device_code", device),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ],
            )
            .await?;
            if let Some(token) = value["access_token"].as_str().filter(|s| !s.is_empty() && s.len() <= 4096) {
                let lifetime = value["expires_in"].as_u64().unwrap_or(28800).min(28800);
                let _ = tx.send(Reply::Authorized(token.into(), Instant::now() + Duration::from_secs(lifetime)));
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

fn create_issue(token: &str, title: String, body: String) -> Result<u64, Failure> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Failure::Network)?;
    rt.block_on(create_request(&client()?, API, token, &title, &body))
}

async fn create_request(client: &reqwest::Client, url: &str, token: &str, title: &str, body: &str) -> Result<u64, Failure> {
    let body = serde_json::to_vec(&serde_json::json!({ "title": title, "body": body, "labels": ["bug", "alpha"] }))
        .map_err(|_| Failure::Invalid)?;
    // A create request is never retried automatically: a timeout can mean
    // the server created the issue but its reply was lost.
    let response = client
        .post(url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2026-03-10")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Real HTTP on loopback only: exercise serialization, status handling and
    // lost replies without a GitHub account or a public issue.
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
    fn direct_request_matches_preview_and_handles_lost_or_denied_replies() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let report = crate::bug_report::Report {
            title: "Échec & exemple".into(),
            actual: "Description reviewed by the tester".into(),
            fields: vec![
                crate::bug_report::Field::new("test", "Location", "UNCHECKED_LOCATION", false),
                crate::bug_report::Field::new("test", "Version", "1.2.3", true),
            ],
            excerpt: "UNCHECKED_LOG".into(),
            ..Default::default()
        };
        let preview = report.markdown();
        for (status, body, expected) in [
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
            let (url, server) = fixture(status, body);
            assert_eq!(
                rt.block_on(create_request(&client, &url, "test-only", &report.title, &preview)),
                expected
            );
            let request = server.join().unwrap();
            let (headers, body) = request.split_once("\r\n\r\n").unwrap();
            assert!(headers.starts_with("POST /issues "));
            let sent: Value = serde_json::from_str(body).unwrap();
            assert_eq!(sent["title"], report.title);
            assert_eq!(sent["body"], preview);
            assert_eq!(sent["labels"], serde_json::json!(["bug", "alpha"]));
            assert!(!body.contains("UNCHECKED_"));
            assert!(!body.contains("test-only"));
        }
    }

    #[test]
    fn authorization_does_not_send_and_disconnect_forgets_session() {
        let (tx, rx) = crossbeam_channel::bounded(2);
        let mut state = Github {
            token: None,
            rx: Some(rx),
            cancel: None,
            user_code: None,
            failure: None,
            created: None,
            sending: false,
        };
        tx.send(Reply::Authorized("test-only".into(), Instant::now() + Duration::from_secs(60)))
            .unwrap();
        state.poll();
        assert!(state.connected());
        assert!(!state.sending);
        assert!(state.created.is_none());
        state.disconnect();
        assert!(!state.connected());
        let (cancel, cancelled) = crossbeam_channel::bounded(1);
        state.cancel = Some(cancel);
        state.user_code = Some("TEST-CODE".into());
        state.cancel_authorization();
        assert!(cancelled.try_recv().is_ok());
        assert!(state.user_code.is_none());
    }

    #[test]
    fn browser_roundtrip_and_overlong_body_fallback() {
        let title = "Échec & panne ? #1";
        let body = "Texte privé sélectionné : ça & ceci\nhttps://example.invalid/";
        let draft = browser_draft(title, body);
        let url = reqwest::Url::parse(&draft.url).unwrap();
        let fields: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(fields["title"], title);
        assert_eq!(fields["body"], body);
        assert!(!draft.paste_body);
        let draft = browser_draft(title, &"é".repeat(6000));
        assert!(draft.paste_body);
        assert!(draft.url.len() < 7000);
        assert!(!draft.url.contains("body="));
    }
    #[test]
    fn polling_respects_pending_slowdown_expiry_and_denial() {
        assert_eq!(
            poll_error(&serde_json::json!({"error":"authorization_pending"})),
            PollState::Pending
        );
        assert_eq!(
            poll_error(&serde_json::json!({"error":"slow_down", "interval": 15})),
            PollState::Slower(15)
        );
        assert_eq!(
            poll_error(&serde_json::json!({"error":"access_denied"})),
            PollState::Failed(Failure::Denied)
        );
        assert_eq!(
            poll_error(&serde_json::json!({"error":"expired_token"})),
            PollState::Failed(Failure::Expired)
        );
        assert_eq!(
            poll_error(&serde_json::json!({"error":"device_flow_disabled"})),
            PollState::Failed(Failure::Configuration)
        );
    }
}
