//! Opt-in translation of received nearby chat, following the Legacy Viewer
//! behaviour in `indra/newview/lltranslate.cpp` and `llviewermessage.cpp`.

use crossbeam_channel::{Receiver, TryRecvError, bounded};
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::{runtime::Handle, task::JoinHandle};

const AZURE: &str = "https://api.cognitive.microsofttranslator.com/translate";
const MAX_PENDING: usize = 4;
const MAX_RESPONSE: usize = 32 * 1024;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub target: String,
    pub azure_key: String,
    pub azure_region: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            target: "en".into(),
            azure_key: String::new(),
            azure_region: String::new(),
        }
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("enabled", &self.enabled)
            .field("target", &self.target)
            .field("azure_region", &self.azure_region)
            .finish_non_exhaustive()
    }
}

impl Settings {
    pub fn ready(&self) -> bool {
        let target = self.target.trim();
        (2..=24).contains(&target.len())
            && target
                .split('-')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
            && !self.azure_key.trim().is_empty()
            && self.azure_key.trim().len() <= 256
            && self.azure_key.trim().bytes().all(|b| b.is_ascii_graphic())
            && self.azure_region.trim().len() <= 64
            && self.azure_region.trim().bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Translated(String),
    Credentials,
    Quota,
    Unavailable,
}

#[derive(Deserialize)]
struct AzureResult {
    #[serde(rename = "detectedLanguage")]
    detected_language: Option<AzureLanguage>,
    translations: Vec<AzureText>,
}

#[derive(Deserialize)]
struct AzureLanguage {
    language: String,
}

#[derive(Deserialize)]
struct AzureText {
    text: String,
}

fn parse(body: &[u8], original: &str, target: &str) -> Result<Option<Outcome>, Outcome> {
    let results: Vec<AzureResult> = serde_json::from_slice(body).map_err(|_| Outcome::Unavailable)?;
    let row = results.into_iter().next().ok_or(Outcome::Unavailable)?;
    let text = row.translations.into_iter().next().ok_or(Outcome::Unavailable)?.text;
    if row
        .detected_language
        .is_some_and(|language| language.language.eq_ignore_ascii_case(target))
    {
        return Ok(None);
    }
    let text = text.trim();
    Ok((!text.is_empty() && text != original.trim()).then(|| Outcome::Translated(text.to_owned())))
}

fn request(client: &reqwest::Client, settings: &Settings, text: &str) -> reqwest::RequestBuilder {
    let mut url = reqwest::Url::parse(AZURE).expect("fixed Azure endpoint");
    url.query_pairs_mut()
        .append_pair("api-version", "3.0")
        .append_pair("to", settings.target.trim());
    let mut key = HeaderValue::from_str(settings.azure_key.trim()).expect("validated API key header");
    key.set_sensitive(true);
    let mut req = client
        .post(url)
        .header("Ocp-Apim-Subscription-Key", key)
        .header(CONTENT_TYPE, "application/json")
        .body(serde_json::json!([{ "Text": text }]).to_string());
    let region = settings.azure_region.trim();
    if !region.is_empty() && !region.eq_ignore_ascii_case("global") {
        req = req.header("Ocp-Apim-Subscription-Region", region);
    }
    req
}

async fn translate(client: &reqwest::Client, settings: &Settings, text: &str) -> Result<Option<Outcome>, Outcome> {
    let mut response = request(client, settings, text).send().await.map_err(|_| Outcome::Unavailable)?;
    match response.status().as_u16() {
        200 => (),
        401 | 403 => return Err(Outcome::Credentials),
        429 => return Err(Outcome::Quota),
        _ => return Err(Outcome::Unavailable),
    }
    if response.content_length().is_some_and(|len| len > MAX_RESPONSE as u64) {
        return Err(Outcome::Unavailable);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Outcome::Unavailable)? {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err(Outcome::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    parse(&body, text, settings.target.trim())
}

fn eligible_body(text: &str) -> Option<&str> {
    let text = text
        .strip_prefix("/me ")
        .or_else(|| text.starts_with("/me'").then(|| &text[3..]))
        .unwrap_or(text)
        .trim();
    (text.len() <= 4096 && text.chars().any(char::is_alphabetic) && !text.contains("://")).then_some(text)
}

struct Job {
    seq: u64,
    result: Receiver<Result<Option<Outcome>, Outcome>>,
    task: JoinHandle<()>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Default)]
pub struct Nearby {
    settings: Option<Settings>,
    worker: Option<(Handle, reqwest::Client)>,
    token: u64,
    jobs: Vec<Job>,
    outcomes: BTreeMap<u64, Outcome>,
    pause_until: Option<Instant>,
}

impl Nearby {
    /// Called before the frame handles chat events. Demo mode passes no settings.
    pub fn configure(&mut self, settings: Option<&Settings>, handle: &Handle, token: u64) {
        let settings = settings.filter(|s| s.enabled && s.ready());
        if self.token != token || self.settings.as_ref() != settings {
            self.jobs.clear();
            self.outcomes.clear();
            self.pause_until = None;
            self.token = token;
            self.settings = settings.cloned();
        }
        if settings.is_some() && self.worker.is_none() {
            self.worker = reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .referer(false)
                .https_only(true)
                .build()
                .ok()
                .map(|client| (handle.clone(), client));
        }
    }

    pub fn offer(&mut self, token: u64, seq: u64, text: &str) {
        let (Some(settings), Some((handle, client))) = (&self.settings, &self.worker) else {
            return;
        };
        if self.token != token || self.jobs.len() >= MAX_PENDING || self.pause_until.is_some_and(|until| Instant::now() < until) {
            return;
        }
        let Some(text) = eligible_body(text) else {
            return;
        };
        let (send, result) = bounded(1);
        let (settings, client, text) = (settings.clone(), client.clone(), text.to_owned());
        let task = handle.spawn(async move {
            let _ = send.send(translate(&client, &settings, &text).await);
        });
        self.jobs.push(Job { seq, result, task });
    }

    pub fn poll(&mut self, token: u64, first: u64) {
        if token != self.token {
            return;
        }
        while self.outcomes.first_key_value().is_some_and(|(&seq, _)| seq < first) {
            self.outcomes.pop_first();
        }
        self.jobs.retain(|job| {
            if job.seq < first {
                return false;
            }
            match job.result.try_recv() {
                Ok(Ok(result)) => {
                    if let Some(result) = result {
                        self.outcomes.insert(job.seq, result);
                    }
                    false
                }
                Ok(Err(error)) => {
                    if self.pause_until.is_none_or(|until| Instant::now() >= until) {
                        self.outcomes.insert(job.seq, error);
                    }
                    self.pause_until = Some(Instant::now() + Duration::from_secs(30));
                    false
                }
                Err(TryRecvError::Empty) => true,
                Err(TryRecvError::Disconnected) => false,
            }
        });
    }

    pub fn outcome(&self, token: u64, seq: u64) -> Option<&Outcome> {
        (token == self.token).then(|| self.outcomes.get(&seq)).flatten()
    }

    /// This fixture stays entirely offline, even if the user saved a key.
    pub fn demo_line(&mut self, token: u64, seq: u64) {
        self.token = token;
        self.outcomes
            .insert(seq, Outcome::Translated("Hello! Are you coming to the concert tonight?".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azure_request_sends_text_in_json_and_a_sensitive_key_header() {
        let settings = Settings {
            target: "zh-Hans".into(),
            azure_key: "example".into(),
            azure_region: "westus2".into(),
            ..Settings::default()
        };
        assert!(settings.ready());
        let request = request(&reqwest::Client::new(), &settings, "Bonjour \"ami\"").build().unwrap();
        assert_eq!(request.url().query(), Some("api-version=3.0&to=zh-Hans"));
        assert!(request.headers()["Ocp-Apim-Subscription-Key"].is_sensitive());
        assert_eq!(request.headers()["Ocp-Apim-Subscription-Region"], "westus2");
        let body: serde_json::Value = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!([{ "Text": "Bonjour \"ami\"" }]));
    }

    #[test]
    fn azure_response_keeps_only_a_distinct_translation() {
        let body = br#"[{"detectedLanguage":{"language":"es"},"translations":[{"text":"Hello!","to":"en"}]}]"#;
        assert_eq!(parse(body, "Hola!", "en"), Ok(Some(Outcome::Translated("Hello!".into()))));
        assert_eq!(parse(body, " Hello! ", "en"), Ok(None));
        assert_eq!(parse(body, "Hola!", "ES"), Ok(None));
        assert_eq!(parse(b"[]", "Hello!", "en"), Err(Outcome::Unavailable));
    }

    #[test]
    fn only_text_chat_is_submitted() {
        assert_eq!(eligible_body("  你好！  "), Some("你好！"));
        assert_eq!(eligible_body("/me smiles"), Some("smiles"));
        assert_eq!(eligible_body("/me's here"), Some("'s here"));
        assert_eq!(eligible_body("✨ 123"), None);
        assert_eq!(eligible_body("Hello https://example.com"), None);
        assert_eq!(eligible_body(&"a".repeat(4097)), None);
    }
}
