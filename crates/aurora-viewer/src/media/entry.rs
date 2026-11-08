//! Media on a prim: the per-face media entries (LLMediaEntry), the media
//! version carried by object updates ("x-mv:0000000003/<agent id>",
//! LLTextureEntry::getVersionFromMediaVersionString) and the ObjectMedia
//! capability requests (LLMediaDataClient: GET, one object per second).
//!
//! Derived from indra/llprimitive/llmediaentry.cpp and
//! indra/newview/llmediadataclient.cpp, Copyright (C) Linden Research, Inc.,
//! originally LGPL 2.1.

use aurora_llsd::{Llsd, Map};
use std::collections::{HashSet, VecDeque};
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

    /// LLVOVolume::hasMediaPermission(MEDIA_PERM_INTERACT): may we navigate
    /// this face to another page? Only navigation depends on it: clicks,
    /// hover and wheel reach the page whatever it says (handleMediaClick).
    /// The group bit is not checked yet.
    pub fn may_navigate(&self, owner: bool) -> bool {
        let bits = if owner { PERM_OWNER | PERM_ANYONE } else { PERM_ANYONE };
        self.perms_interact & bits != 0
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

/// One GET waiting for its turn, in flight, or waiting to be retried.
#[derive(Debug)]
struct Pending {
    object: Uuid,
    cap: String,
    /// Attempts already made.
    attempts: u32,
    /// A refresh of data already received (the round-robin queue), rather
    /// than the first load of an object (the sorted queue).
    update: bool,
}

/// Why a GET failed: `retry` when the simulator was busy (503, its answer
/// to the rate limit) or the request never reached it.
struct GetError {
    message: String,
    retry: bool,
}

/// ObjectMedia GET queue, as LLObjectMediaDataClient: the simulator allows
/// about 5 requests every 5 s per agent and answers 503 beyond, so one
/// request leaves per PrimMediaRequestQueueDelay (1 s). First loads wait in
/// a queue sorted by interest (the face nearest the eye first), refreshes
/// after a media change in a round-robin queue, and the two take turns so
/// that a script changing a screen is not stuck behind a region's worth of
/// first loads. A busy answer is retried after PrimMediaRetryTimerDelay
/// (5 s), PrimMediaMaxRetries (4) attempts in all; the caller then waits for
/// the next media change of the object before asking again.
pub struct MediaDataClient {
    sorted: Vec<Pending>,
    round_robin: VecDeque<Pending>,
    /// Busy answers, re-queued at their instant (LLMediaDataClient::RetryTimer).
    retrying: Vec<(Instant, Pending)>,
    /// Objects queued, retrying or in flight.
    queued: HashSet<Uuid>,
    /// The sorted queue's turn (LLObjectMediaDataClient::swapCurrentQueue).
    sorted_turn: bool,
    tx: crossbeam_channel::Sender<(Pending, Result<ObjectMediaData, GetError>)>,
    rx: crossbeam_channel::Receiver<(Pending, Result<ObjectMediaData, GetError>)>,
    last_send: Option<Instant>,
}

const QUEUE_DELAY: Duration = Duration::from_millis(1000);
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// PrimMediaMaxRetries: LLMediaDataClient::Handler::onFailure counts the
/// first attempt, so 4 attempts in all.
const MAX_ATTEMPTS: u32 = 4;

impl Default for MediaDataClient {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        MediaDataClient {
            sorted: Vec::new(),
            round_robin: VecDeque::new(),
            retrying: Vec::new(),
            queued: HashSet::new(),
            sorted_turn: true,
            tx,
            rx,
            last_send: None,
        }
    }
}

impl MediaDataClient {
    pub fn is_queued(&self, object: &Uuid) -> bool {
        self.queued.contains(object)
    }

    /// Queue a GET for an object (no-op if already queued, retrying or in
    /// flight). `update`: the object's data was received before.
    pub fn request(&mut self, object: Uuid, cap: &str, update: bool) {
        if !self.queued.insert(object) {
            return;
        }
        let p = Pending {
            object,
            cap: cap.to_string(),
            attempts: 0,
            update,
        };
        self.enqueue(p);
    }

    fn enqueue(&mut self, p: Pending) {
        if p.update {
            self.round_robin.push_back(p);
        } else {
            self.sorted.push(p);
        }
    }

    pub fn clear(&mut self) {
        self.sorted.clear();
        self.round_robin.clear();
        self.retrying.clear();
        self.queued.clear();
    }

    /// Send the next request when its turn has come, return the answers
    /// that arrived. `interest` ranks the first loads (pixels on screen,
    /// then nearness); `None` drops the request of an object that is gone.
    pub fn update(
        &mut self,
        rt: &tokio::runtime::Handle,
        http: &reqwest::Client,
        now: Instant,
        interest: impl Fn(&Uuid) -> Option<(f32, f32)>,
    ) -> Vec<Fetched> {
        let mut out = Vec::new();
        while let Ok((p, result)) = self.rx.try_recv() {
            if let Some(f) = self.answer(p, result, now) {
                out.push(f);
            }
        }
        if let Some(p) = self.next(now, interest) {
            let (tx, http) = (self.tx.clone(), http.clone());
            rt.spawn(async move {
                let data = get(&http, &p.cap, p.object).await;
                let _ = tx.send((p, data));
            });
        }
        out
    }

    /// An answer: kept for a retry when the simulator was busy, else handed
    /// to the caller.
    fn answer(&mut self, mut p: Pending, result: Result<ObjectMediaData, GetError>, now: Instant) -> Option<Fetched> {
        p.attempts += 1;
        let data = match result {
            Ok(d) => Ok(d),
            Err(e) if e.retry && p.attempts < MAX_ATTEMPTS && self.queued.contains(&p.object) => {
                log::debug!("ObjectMedia GET {} failed ({}), retrying in 5 s", p.object, e.message);
                self.retrying.push((now + RETRY_DELAY, p));
                return None;
            }
            Err(e) => Err(e.message),
        };
        self.queued.remove(&p.object);
        Some(Fetched { object: p.object, data })
    }

    /// The request to send now, if any: one per QUEUE_DELAY, from the
    /// queue whose turn it is (LLObjectMediaDataClient::processQueueTimer).
    fn next(&mut self, now: Instant, interest: impl Fn(&Uuid) -> Option<(f32, f32)>) -> Option<Pending> {
        let mut i = 0;
        while i < self.retrying.len() {
            if self.retrying[i].0 <= now {
                let (_, p) = self.retrying.swap_remove(i);
                self.enqueue(p);
            } else {
                i += 1;
            }
        }
        if self.last_send.is_some_and(|t| now.duration_since(t) < QUEUE_DELAY) {
            return None;
        }
        // objects gone since they were queued (Request::isDead)
        let queued = &mut self.queued;
        let mut alive = |p: &Pending| {
            let keep = interest(&p.object).is_some();
            if !keep {
                queued.remove(&p.object);
            }
            keep
        };
        self.sorted.retain(&mut alive);
        self.round_robin.retain(&mut alive);
        let use_sorted = if self.sorted_turn {
            !self.sorted.is_empty()
        } else {
            self.round_robin.is_empty()
        };
        let p = if use_sorted {
            // LLObjectMediaDataClient::sortQueue: the highest interest first
            let best = self
                .sorted
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    let (ia, ib) = (interest(&a.object).unwrap_or_default(), interest(&b.object).unwrap_or_default());
                    ia.0.total_cmp(&ib.0).then(ib.1.total_cmp(&ia.1))
                })
                .map(|(i, _)| i)?;
            self.sorted.swap_remove(best)
        } else {
            self.round_robin.pop_front()?
        };
        // the other queue's turn next (swapCurrentQueue)
        self.sorted_turn = !use_sorted;
        self.last_send = Some(now);
        Some(p)
    }
}

/// POST {verb: GET, object_id} → {object_id, object_media_version,
/// object_media_data: [entry | undef per face]} or {error: {code, message}}.
async fn get(http: &reqwest::Client, cap: &str, object: Uuid) -> Result<ObjectMediaData, GetError> {
    let unsent = |e: reqwest::Error| GetError {
        message: e.without_url().to_string(),
        retry: true,
    };
    let fail = |message: String| GetError { message, retry: false };
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
        .map_err(unsent)?;
    let status = resp.status();
    if !status.is_success() {
        // LLMediaDataClient::Handler::onFailure retries HTTP_SERVICE_UNAVAILABLE only
        return Err(GetError {
            message: format!("HTTP {status}"),
            retry: status == reqwest::StatusCode::SERVICE_UNAVAILABLE,
        });
    }
    let bytes = resp.bytes().await.map_err(|e| fail(e.without_url().to_string()))?;
    let v = aurora_llsd::from_xml(&bytes).map_err(|e| fail(e.to_string()))?;
    parse_get_response(&v, object).map_err(fail)
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
    fn interaction_permissions_gate_navigation() {
        let mut e = MediaEntry::default();
        assert!(e.may_navigate(true) && e.may_navigate(false));
        e.perms_interact = PERM_OWNER;
        assert!(e.may_navigate(true) && !e.may_navigate(false));
        // a HUD page locked to its home page: nobody navigates it
        e.perms_interact = 0;
        assert!(!e.may_navigate(true) && !e.may_navigate(false));
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn busy() -> Result<ObjectMediaData, GetError> {
        Err(GetError {
            message: "HTTP 503 Service Unavailable".into(),
            retry: true,
        })
    }

    #[test]
    fn one_request_per_second_whatever_the_queue() {
        let mut c = MediaDataClient::default();
        for n in 1..=40 {
            c.request(id(n), "cap", false);
        }
        let t0 = Instant::now();
        let any = |_: &Uuid| Some((0.0, 1.0));
        let mut sent = 0;
        // 10 s of frames at 100 Hz: 10 requests, never more than 1 a second
        for frame in 0..1000 {
            if c.next(t0 + Duration::from_millis(frame * 10), any).is_some() {
                sent += 1;
            }
        }
        assert_eq!(sent, 10);
    }

    #[test]
    fn first_loads_go_by_interest_and_take_turns_with_refreshes() {
        let mut c = MediaDataClient::default();
        // far, near and on-screen first loads, then a refresh
        c.request(id(1), "cap", false);
        c.request(id(2), "cap", false);
        c.request(id(3), "cap", false);
        c.request(id(9), "cap", true);
        let interest = |o: &Uuid| {
            Some(match o.as_u128() {
                1 => (0.0, 90.0),
                2 => (0.0, 10.0),
                3 => (5000.0, 30.0),
                _ => (0.0, 50.0),
            })
        };
        let t0 = Instant::now();
        let order: Vec<u128> = (0..4)
            .filter_map(|s| c.next(t0 + Duration::from_secs(s), interest))
            .map(|p| p.object.as_u128())
            .collect();
        assert_eq!(order, [3, 9, 2, 1]);
    }

    #[test]
    fn busy_answers_are_retried_four_attempts_in_all() {
        let mut c = MediaDataClient::default();
        c.request(id(1), "cap", false);
        let any = |_: &Uuid| Some((0.0, 1.0));
        let mut t = Instant::now();
        for attempt in 1..=MAX_ATTEMPTS {
            let p = c.next(t, any).expect("request sent");
            assert_eq!(p.attempts, attempt - 1);
            let got = c.answer(p, busy(), t);
            if attempt < MAX_ATTEMPTS {
                assert!(got.is_none() && c.is_queued(&id(1)));
                // not before the retry delay
                assert!(c.next(t + Duration::from_secs(2), any).is_none());
                t += RETRY_DELAY;
            } else {
                assert!(got.expect("given up").data.is_err());
                assert!(!c.is_queued(&id(1)));
            }
        }
        assert!(c.next(t + Duration::from_secs(60), any).is_none(), "nothing left to send");
    }

    #[test]
    fn other_failures_are_not_retried() {
        let mut c = MediaDataClient::default();
        c.request(id(1), "cap", false);
        let t = Instant::now();
        let p = c.next(t, |_| Some((0.0, 1.0))).expect("request sent");
        let refused = Err(GetError {
            message: "HTTP 404 Not Found".into(),
            retry: false,
        });
        assert!(c.answer(p, refused, t).expect("answer").data.is_err());
        assert!(!c.is_queued(&id(1)));
    }

    #[test]
    fn requests_of_objects_gone_are_dropped() {
        let mut c = MediaDataClient::default();
        c.request(id(1), "cap", false);
        c.request(id(2), "cap", true);
        assert!(c.next(Instant::now(), |_| None).is_none());
        assert!(!c.is_queued(&id(1)) && !c.is_queued(&id(2)));
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
