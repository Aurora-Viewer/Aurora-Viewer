//! Media on a prim: the per-face media entries (LLMediaEntry), the media
//! version carried by object updates ("x-mv:0000000003/<agent id>",
//! LLTextureEntry::getVersionFromMediaVersionString) and the ObjectMedia
//! capability requests (LLMediaDataClient: GET, one object at a time).
//!
//! Derived from indra/llprimitive/llmediaentry.cpp and
//! indra/newview/llmediadataclient.cpp, Copyright (C) Linden Research, Inc.,
//! originally LGPL 2.1.

use aurora_llsd::{Llsd, Map};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const MEDIA_VERSION_PREFIX: &str = "x-mv:";

/// Permission bits of perms_interact / perms_control.
pub const PERM_OWNER: u8 = 1;
#[allow(dead_code)] // group membership is not checked yet
pub const PERM_GROUP: u8 = 2;
pub const PERM_ANYONE: u8 = 4;

/// One face's media settings (LLMediaEntry, defaults of its constructor).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaEntry {
    pub alt_image_enable: bool,
    /// 0 standard, 1 mini.
    pub controls: u8,
    pub current_url: String,
    pub home_url: String,
    pub auto_loop: bool,
    pub auto_play: bool,
    pub auto_scale: bool,
    pub auto_zoom: bool,
    pub first_click_interact: bool,
    pub width_pixels: u16,
    pub height_pixels: u16,
    pub whitelist_enable: bool,
    pub whitelist: Vec<String>,
    pub perms_interact: u8,
    pub perms_control: u8,
}

impl Default for MediaEntry {
    fn default() -> Self {
        MediaEntry {
            alt_image_enable: false,
            controls: 0,
            current_url: String::new(),
            home_url: String::new(),
            auto_loop: false,
            auto_play: false,
            auto_scale: false,
            auto_zoom: false,
            first_click_interact: false,
            width_pixels: 0,
            height_pixels: 0,
            whitelist_enable: false,
            whitelist: Vec::new(),
            perms_interact: 7,
            perms_control: 7,
        }
    }
}

impl MediaEntry {
    /// LLMediaEntry::fromLLSD (missing keys keep their defaults).
    pub fn from_llsd(v: &Llsd) -> MediaEntry {
        let mut e = MediaEntry::default();
        let b = |k: &str, d: bool| if v.has(k) { v.get(k).as_bool() } else { d };
        let i = |k: &str, d: i32| if v.has(k) { v.get(k).as_i32() } else { d };
        let s = |k: &str| v.get(k).to_string_value();
        e.alt_image_enable = b("alt_image_enable", e.alt_image_enable);
        e.controls = i("controls", 0).clamp(0, 1) as u8;
        e.current_url = s("current_url");
        e.home_url = s("home_url");
        e.auto_loop = b("auto_loop", false);
        e.auto_play = b("auto_play", false);
        e.auto_scale = b("auto_scale", false);
        e.auto_zoom = b("auto_zoom", false);
        e.first_click_interact = b("first_click_interact", false);
        e.width_pixels = i("width_pixels", 0).clamp(0, 2048) as u16;
        e.height_pixels = i("height_pixels", 0).clamp(0, 2048) as u16;
        e.whitelist_enable = b("whitelist_enable", false);
        e.whitelist = v.get("whitelist").as_array().iter().map(|w| w.to_string_value()).take(64).collect();
        e.perms_interact = i("perms_interact", 7) as u8;
        e.perms_control = i("perms_control", 7) as u8;
        e
    }

    /// LLMediaEntry::asLLSD (ObjectMedia UPDATE, one per face).
    pub fn to_llsd(&self) -> Llsd {
        let mut m = Llsd::new_map();
        m.insert("alt_image_enable", self.alt_image_enable);
        m.insert("controls", self.controls as i32);
        m.insert("current_url", self.current_url.clone());
        m.insert("home_url", self.home_url.clone());
        m.insert("auto_loop", self.auto_loop);
        m.insert("auto_play", self.auto_play);
        m.insert("auto_scale", self.auto_scale);
        m.insert("auto_zoom", self.auto_zoom);
        m.insert("first_click_interact", self.first_click_interact);
        m.insert("width_pixels", self.width_pixels as i32);
        m.insert("height_pixels", self.height_pixels as i32);
        m.insert("whitelist_enable", self.whitelist_enable);
        m.insert(
            "whitelist",
            Llsd::Array(self.whitelist.iter().map(|w| Llsd::from(w.clone())).collect()),
        );
        m.insert("perms_interact", self.perms_interact as i32);
        m.insert("perms_control", self.perms_control as i32);
        m
    }

    /// URL to show: the current one, else home (LLViewerMediaImpl).
    pub fn url(&self) -> &str {
        if self.current_url.trim().is_empty() {
            self.home_url.trim()
        } else {
            self.current_url.trim()
        }
    }

    /// LLMediaEntry::checkCandidateUrl: allowed by the whitelist (if on).
    /// Patterns are `[scheme://]host[/path]` with `*` wildcards; the host
    /// part matches the end of the URL's host (www.example.com ~ example.com).
    pub fn check_url(&self, url: &str) -> bool {
        if !self.whitelist_enable || self.whitelist.is_empty() {
            return true;
        }
        let (host, path) = split_host_path(url);
        self.whitelist.iter().any(|pat| {
            let pat = pat.trim();
            let pat = pat.split_once("://").map(|(_, r)| r).unwrap_or(pat);
            let (ph, pp) = match pat.find('/') {
                Some(i) => (&pat[..i], &pat[i..]),
                None => (pat, ""),
            };
            let host_ok = if ph.starts_with('*') {
                wildcard(&ph.to_ascii_lowercase(), &host)
            } else {
                let ph = ph.to_ascii_lowercase();
                host == ph || host.ends_with(&format!(".{ph}")) || wildcard(&ph, &host)
            };
            host_ok && (pp.is_empty() || pp == "/" || wildcard(&format!("{pp}*"), &path))
        })
    }
}

fn split_host_path(url: &str) -> (String, String) {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (auth, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let host = auth.rsplit('@').next().unwrap_or(auth);
    let host = host.split(':').next().unwrap_or(host);
    (host.to_ascii_lowercase(), path.to_string())
}

/// `*` matches any run of characters.
fn wildcard(pat: &str, text: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == text;
    }
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !text.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if i == parts.len() - 1 {
            return text.len() >= pos + part.len() && text[pos..].ends_with(part);
        } else {
            match text[pos..].find(part) {
                Some(f) => pos += f + part.len(),
                None => return false,
            }
        }
    }
    true
}

/// Version number of an "x-mv:" media URL (None for other URLs).
pub fn media_version(url: &str) -> Option<u32> {
    let rest = url.strip_prefix(MEDIA_VERSION_PREFIX)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Agent who last changed the media ("x-mv:<version>/<agent>").
pub fn media_version_agent(url: &str) -> Uuid {
    url.strip_prefix(MEDIA_VERSION_PREFIX)
        .and_then(|r| r.split_once('/'))
        .and_then(|(_, a)| Uuid::parse_str(a.trim()).ok())
        .unwrap_or_default()
}

/// Media data of an object, as returned by ObjectMedia.
#[derive(Debug, Clone, Default)]
pub struct ObjectMediaData {
    pub version: u32,
    /// Per face (None: no media on that face).
    pub faces: Vec<Option<MediaEntry>>,
}

/// Result of a GET.
pub struct Fetched {
    pub object: Uuid,
    pub data: Result<ObjectMediaData, String>,
}

struct Pending {
    object: Uuid,
    cap: String,
    retries: u32,
    not_before: Instant,
}

/// ObjectMedia GET queue (LLObjectMediaDataClient): requests are spaced by
/// PrimMediaRequestQueueDelay (1 s) per batch, failures retried after
/// PrimMediaRetryTimerDelay (5 s) up to PrimMediaMaxRetries (4) times.
pub struct MediaDataClient {
    queue: VecDeque<Pending>,
    queued: HashMap<Uuid, u32>,
    in_flight: usize,
    tx: crossbeam_channel::Sender<(Fetched, String, u32)>,
    rx: crossbeam_channel::Receiver<(Fetched, String, u32)>,
    last_batch: Instant,
}

const QUEUE_DELAY: Duration = Duration::from_millis(1000);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_RETRIES: u32 = 4;
const MAX_IN_FLIGHT: usize = 4;
const BATCH: usize = 8;

impl Default for MediaDataClient {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        MediaDataClient {
            queue: VecDeque::new(),
            queued: HashMap::new(),
            in_flight: 0,
            tx,
            rx,
            last_batch: Instant::now() - QUEUE_DELAY,
        }
    }
}

impl MediaDataClient {
    pub fn is_queued(&self, object: &Uuid) -> bool {
        self.queued.contains_key(object)
    }

    /// Queue a GET for an object (no-op if already queued).
    pub fn request(&mut self, object: Uuid, cap: &str) {
        if self.queued.contains_key(&object) {
            return;
        }
        self.queued.insert(object, 0);
        self.queue.push_back(Pending {
            object,
            cap: cap.to_string(),
            retries: 0,
            not_before: Instant::now(),
        });
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.queued.clear();
    }

    /// Send due requests, return the answers that arrived.
    pub fn update(&mut self, rt: &tokio::runtime::Handle, http: &reqwest::Client) -> Vec<Fetched> {
        let mut out = Vec::new();
        while let Ok((f, cap, retries)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            match &f.data {
                Err(e) if retries < MAX_RETRIES && !e.starts_with("error 8002") => {
                    log::debug!("ObjectMedia GET {} failed ({e}), retrying", f.object);
                    self.queue.push_back(Pending {
                        object: f.object,
                        cap,
                        retries: retries + 1,
                        not_before: Instant::now() + RETRY_DELAY,
                    });
                }
                _ => {
                    self.queued.remove(&f.object);
                    out.push(f);
                }
            }
        }
        if self.last_batch.elapsed() < QUEUE_DELAY {
            return out;
        }
        let now = Instant::now();
        let mut sent = 0;
        let mut i = 0;
        while i < self.queue.len() && sent < BATCH && self.in_flight < MAX_IN_FLIGHT {
            if self.queue[i].not_before > now {
                i += 1;
                continue;
            }
            let Some(p) = self.queue.remove(i) else {
                break;
            };
            sent += 1;
            self.in_flight += 1;
            let (tx, http) = (self.tx.clone(), http.clone());
            rt.spawn(async move {
                let data = get(&http, &p.cap, p.object).await;
                let _ = tx.send((Fetched { object: p.object, data }, p.cap, p.retries));
            });
        }
        if sent > 0 {
            self.last_batch = now;
        }
        out
    }
}

/// POST {verb: GET, object_id} → {object_id, object_media_version,
/// object_media_data: [entry | undef per face]} or {error: {code, message}}.
async fn get(http: &reqwest::Client, cap: &str, object: Uuid) -> Result<ObjectMediaData, String> {
    let mut m = Map::new();
    m.insert("verb".into(), Llsd::String("GET".into()));
    m.insert("object_id".into(), Llsd::Uuid(object));
    let resp = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(aurora_llsd::to_xml(&Llsd::Map(m)))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let v = aurora_llsd::from_xml(&bytes).map_err(|e| e.to_string())?;
    parse_get_response(&v, object)
}

pub fn parse_get_response(v: &Llsd, object: Uuid) -> Result<ObjectMediaData, String> {
    if v.has("error") {
        let e = v.get("error");
        return Err(format!("error {} {}", e.get("code").as_i32(), e.get("message").to_string_value()));
    }
    let id = v.get("object_id").as_uuid();
    if !id.is_nil() && id != object {
        return Err("answer for another object".into());
    }
    let version = media_version(&v.get("object_media_version").to_string_value()).unwrap_or(0);
    let faces = v
        .get("object_media_data")
        .as_array()
        .iter()
        .map(|f| if f.is_map() { Some(MediaEntry::from_llsd(f)) } else { None })
        .collect();
    Ok(ObjectMediaData { version, faces })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_round_trips() {
        let e = MediaEntry {
            home_url: "https://example.com".into(),
            auto_play: true,
            width_pixels: 512,
            whitelist_enable: true,
            whitelist: vec!["example.com".into()],
            perms_interact: 3,
            ..Default::default()
        };
        assert_eq!(MediaEntry::from_llsd(&e.to_llsd()), e);
    }

    #[test]
    fn version_strings() {
        let s = "x-mv:0000000012/a2e76fcd-9360-4f6d-a924-000000000003";
        assert_eq!(media_version(s), Some(12));
        assert_eq!(media_version_agent(s).to_string(), "a2e76fcd-9360-4f6d-a924-000000000003");
        assert_eq!(media_version("http://example.com"), None);
    }

    #[test]
    fn whitelist() {
        let mut e = MediaEntry {
            whitelist_enable: true,
            whitelist: vec!["example.com".into(), "*.foo.org/videos".into()],
            ..Default::default()
        };
        assert!(e.check_url("https://www.example.com/page"));
        assert!(e.check_url("http://example.com"));
        assert!(!e.check_url("http://badexample.com"));
        assert!(e.check_url("https://a.foo.org/videos/1"));
        assert!(!e.check_url("https://a.foo.org/other"));
        e.whitelist_enable = false;
        assert!(e.check_url("http://anything"));
    }

    #[test]
    fn parse_response() {
        let mut entry = Map::new();
        entry.insert("home_url".into(), Llsd::String("https://example.com".into()));
        entry.insert("auto_play".into(), Llsd::Boolean(true));
        entry.insert("width_pixels".into(), Llsd::Integer(800));
        let mut m = Map::new();
        m.insert(
            "object_media_version".into(),
            Llsd::String("x-mv:0000000003/00000000-0000-0000-0000-000000000000".into()),
        );
        m.insert("object_media_data".into(), Llsd::Array(vec![Llsd::Undef, Llsd::Map(entry)]));
        let d = parse_get_response(&Llsd::Map(m), Uuid::nil()).unwrap();
        assert_eq!(d.version, 3);
        assert!(d.faces[0].is_none());
        let f = d.faces[1].as_ref().unwrap();
        assert!(f.auto_play);
        assert_eq!(f.width_pixels, 800);
        assert_eq!(f.url(), "https://example.com");
        assert_eq!(f.perms_interact, 7);
    }
}
