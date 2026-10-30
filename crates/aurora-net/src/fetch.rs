//! Prioritised, concurrency-limited HTTP fetcher for assets (textures,
//! meshes, materials). Results are delivered on a crossbeam channel so the
//! render thread can poll them without blocking.
//!
//! Requests follow Firestorm's llcorehttp GET (indra/llcorehttp/
//! _httpoprequest.cpp, _httppolicy.cpp, httpcommon.cpp, originally LGPL 2.1):
//! `Range: bytes=a-b` (or `bytes=a-` when open), no transparent content
//! decoding, up to 10 redirects, and the same retry policy (5 retries of
//! 499–599 statuses and transport errors, 1 s doubling backoff capped at
//! 20 s, `Retry-After` honoured below 30 s).

use crate::probe::{self, HttpKind};
use crate::stats::NetStats;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc};

/// Requests to the appearance service (the bakes) under way at once at
/// most. It is Linden Lab's own service, not a CDN, and must share the
/// transport bound with the other assets.
const BAKE_SLOTS: usize = 24;

/// A bake from the appearance service: `…/texture/<avatar>/<bake>/<id>`,
/// with no query (the CDN's assets always have one, `?texture_id=…`).
fn is_bake(url: &str) -> bool {
    !url.contains('?') && url.contains("/texture/")
}

/// Byte range of a request (`end` inclusive, None = up to the end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: Option<u64>,
}

impl ByteRange {
    /// `Range` header value, formatted like llcorehttp (`bytes=%lu-%lu` or
    /// `bytes=%lu-`).
    pub fn header(&self) -> String {
        match self.end {
            Some(end) => format!("bytes={}-{end}", self.start),
            None => format!("bytes={}-", self.start),
        }
    }

    /// Number of bytes asked for (None when open-ended).
    pub fn requested_len(&self) -> Option<u64> {
        self.end.map(|end| end.saturating_sub(self.start) + 1)
    }
}

/// Where a request waits. Firestorm gives each kind of asset its own
/// connections (`LLAppCoreHttp::init_data`, llappcorehttp.cpp: 12 for the
/// assets, 8 for the textures, 8 for the meshes), so an animation never
/// waits behind a region's meshes. Here they share one pool and the urgent
/// lane goes first: it waits for the next slot to free up, a few
/// milliseconds with 24 requests under way. Slots kept for it were tried
/// and left out: idle once its few requests were served, they slowed a
/// whole arrival by a sixth (the 4 of 24 the bulk could not take).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Lane {
    /// Meshes and textures, ordered together by their size on screen.
    #[default]
    Bulk,
    /// Small assets something waits on: the sky, animations, sounds,
    /// materials, our own bakes, a window's image or landmark.
    Urgent,
}

#[derive(Debug, Clone)]
pub struct FetchRequest {
    /// Caller-defined key, echoed in the result.
    pub key: u64,
    pub url: String,
    pub range: Option<ByteRange>,
    /// Higher is more urgent, within its lane.
    pub priority: f32,
    pub lane: Lane,
    pub accept: &'static str,
}

#[derive(Debug)]
pub struct FetchResult {
    pub key: u64,
    /// Final HTTP status (0 when no response was received).
    pub status: u16,
    /// Whole resource was returned (no further ranges available).
    pub complete: bool,
    /// Position of the first returned byte in the resource: the
    /// `Content-Range` start of a 206 (the requested start when the header
    /// is missing, as LLTextureFetchWorker::callbackHttpGet assumes), 0 for
    /// a whole-resource 200.
    pub offset: u64,
    /// The body, or a description of the failure: HTTP status, error kind,
    /// host and attempts. It never contains the URL path or query (asset
    /// URLs are capabilities, which are secrets).
    pub data: Result<Vec<u8>, String>,
}

/// Retry policy of llcorehttp (HttpPolicy::retryOp).
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Retries after the first attempt (HTTP_RETRY_COUNT_DEFAULT).
    pub retries: u32,
    /// First delay, doubled at each retry (HTTP_RETRY_BACKOFF_MIN_DEFAULT).
    pub backoff_min: Duration,
    /// Longest delay: HTTP_RETRY_BACKOFF_MAX_DEFAULT clamped to
    /// HTTP_RETRY_BACKOFF_MAX when request options are given, as they are
    /// for every asset fetch.
    pub backoff_max: Duration,
}

impl RetryPolicy {
    pub const FIRESTORM: RetryPolicy = RetryPolicy {
        retries: 5,
        backoff_min: Duration::from_secs(1),
        backoff_max: Duration::from_secs(20),
    };

    /// Delay before retry number `retry` (0-based). A `Retry-After` of 1 to
    /// 29 seconds replaces the backoff, as in HttpPolicy::retryOp.
    pub fn delay(&self, retry: u32, retry_after: Option<u64>) -> Duration {
        if let Some(secs) = retry_after
            && secs > 0
            && secs < 30
        {
            return Duration::from_secs(secs);
        }
        let factor = if retry <= 10 { 1u32 << retry } else { 1024 };
        self.backoff_min.saturating_mul(factor).min(self.backoff_max)
    }
}

/// HTTP statuses llcorehttp retries (HttpStatus::isRetryable: 499 to 599).
pub fn status_retryable(status: u16) -> bool {
    (499..=599).contains(&status)
}

/// What a received status means for an asset request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    /// 2xx: data.
    Success,
    /// 416: the range starts past the end; LLTextureFetchWorker accepts
    /// the data it already has as complete.
    RangeNotSatisfiable,
    /// 499–599: retried.
    Retryable,
    /// Anything else (404 missing asset, 403, unfollowed 3xx…).
    Failed,
}

pub fn classify_status(status: u16) -> StatusClass {
    match status {
        200..=299 => StatusClass::Success,
        416 => StatusClass::RangeNotSatisfiable,
        s if status_retryable(s) => StatusClass::Retryable,
        _ => StatusClass::Failed,
    }
}

/// Parsed `Content-Range: bytes first-last/total` (total may be `*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    pub first: u64,
    pub last: u64,
    pub total: Option<u64>,
}

pub fn parse_content_range(value: &str) -> Option<ContentRange> {
    let rest = value.trim().strip_prefix("bytes")?.trim_start();
    let (span, total) = rest.split_once('/')?;
    let (first, last) = span.trim().split_once('-')?;
    let first = first.trim().parse().ok()?;
    let last = last.trim().parse().ok()?;
    if last < first {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some(ContentRange { first, last, total })
}

/// Offset and completeness of a successful body
/// (LLTextureFetchWorker::callbackHttpGet): a 200 is the whole resource
/// whatever range was asked; a 206 is complete when it ends at the total
/// size, or when it is shorter than what was asked.
pub fn body_extent(status: u16, range: Option<ByteRange>, content_range: Option<ContentRange>, body_len: u64) -> (u64, bool) {
    let Some(range) = range.filter(|_| status == 206) else {
        return (0, true);
    };
    let offset = content_range.map_or(range.start, |c| c.first);
    let complete = match (content_range.and_then(|c| c.total), range.requested_len()) {
        (Some(total), _) => offset + body_len >= total,
        (None, Some(asked)) => body_len < asked,
        // open range without a total: everything up to the end was sent
        (None, None) => true,
    };
    (offset, complete)
}

/// Replace every `scheme://host/path?query` in `text` by `scheme://host`,
/// so that error messages never carry a capability.
pub fn redact_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find("://") {
        let (before, after) = rest.split_at(pos);
        out.push_str(before);
        out.push_str("://");
        let after = &after[3..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | '>'))
            .unwrap_or(after.len());
        let url = &after[..end];
        let host_end = url.find(['/', '?', '#']).unwrap_or(url.len());
        out.push_str(&url[..host_end]);
        if host_end < url.len() {
            out.push('…');
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Kind of a transport error, without the URL that reqwest's own message
/// includes, followed by the underlying causes.
fn describe_transport_error(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        aurora_i18n::tr!("fetch-connection-failed")
    } else if e.is_redirect() {
        aurora_i18n::tr!("fetch-redirect-failed")
    } else if e.is_body() || e.is_decode() {
        // content decoding is off: reqwest reports short or broken bodies
        // as decode errors
        "body read failed"
    } else if e.is_request() {
        aurora_i18n::tr!("fetch-request-failed")
    } else {
        aurora_i18n::tr!("fetch-transport-error")
    };
    let mut text = kind.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        text.push_str(": ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    redact_urls(&text)
}

/// llcorehttp retries these transport failures (connect, timeouts,
/// send/receive errors, short bodies); not redirect loops.
fn transport_retryable(e: &reqwest::Error) -> bool {
    !e.is_redirect() && !e.is_builder()
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".into())
}

/// Log the HTTP version an asset host answers with, once per host and
/// version: how many requests a connection carries at a time depends on it.
fn note_protocol(host: &str, version: reqwest::Version) {
    static SEEN: parking_lot::Mutex<Vec<(String, reqwest::Version)>> = parking_lot::Mutex::new(Vec::new());
    let mut seen = SEEN.lock();
    if !seen.iter().any(|(h, v)| h == host && *v == version) {
        seen.push((host.to_string(), version));
        log::info!("asset host {host} answers in {version:?}");
    }
}

enum Msg {
    Request(FetchRequest),
    Priority(u64, f32),
    Cancel(u64),
}

/// A place in the heap: stale once its request was cancelled, sent, or
/// given another priority (a newer generation).
struct Node {
    lane: Lane,
    priority: f32,
    generation: u64,
    key: u64,
}

impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == CmpOrdering::Equal
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    /// Lane first, then priority, then the oldest.
    fn cmp(&self, o: &Self) -> CmpOrdering {
        self.lane
            .cmp(&o.lane)
            .then_with(|| self.priority.total_cmp(&o.priority))
            .then_with(|| o.generation.cmp(&self.generation))
    }
}

/// The requests waiting for a slot. Their priority can change while they
/// wait (Firestorm scores its pending mesh requests again before sending
/// the best ones, `LLMeshRepository::notifyLoadedMeshes`, and its texture
/// fetch follows each texture's current priority), and a cancelled one
/// leaves. Once sent, a request runs to its end: like Firestorm, nothing
/// in flight is cancelled, its caller simply ignores the result.
#[derive(Default)]
struct FetchQueue {
    heap: BinaryHeap<Node>,
    /// key -> the request and the generation of its live heap node
    waiting: HashMap<u64, (FetchRequest, u64)>,
    generation: u64,
    /// When each waiting request was first queued (AURORA_PROFILE only:
    /// the wait for a slot, `http summary`).
    queued_at: HashMap<u64, Instant>,
}

impl FetchQueue {
    fn len(&self) -> usize {
        self.waiting.len()
    }

    fn insert_node(&mut self, key: u64, lane: Lane, priority: f32) -> u64 {
        self.generation += 1;
        self.heap.push(Node {
            lane,
            priority,
            generation: self.generation,
            key,
        });
        // (the stale nodes left by new priorities, swept now and then)
        if self.heap.len() > 2 * self.waiting.len() + 64 {
            let waiting = &self.waiting;
            self.heap
                .retain(|n| waiting.get(&n.key).is_some_and(|(_, g)| *g == n.generation) || n.generation == self.generation);
        }
        self.generation
    }

    /// Queue a request; one already waiting under its key is replaced.
    fn push(&mut self, r: FetchRequest) {
        let (key, lane, priority) = (r.key, r.lane, r.priority);
        self.waiting.remove(&key);
        let generation = self.insert_node(key, lane, priority);
        self.waiting.insert(key, (r, generation));
        if probe::enabled() {
            self.queued_at.entry(key).or_insert_with(Instant::now);
        }
    }

    /// New priority for a request still waiting (false when it is not).
    fn reprioritise(&mut self, key: u64, priority: f32) -> bool {
        let Some(lane) = self.waiting.get(&key).map(|(r, _)| r.lane) else {
            return false;
        };
        let generation = self.insert_node(key, lane, priority);
        if let Some((r, g)) = self.waiting.get_mut(&key) {
            r.priority = priority;
            *g = generation;
        }
        true
    }

    fn cancel(&mut self, key: u64) -> bool {
        self.queued_at.remove(&key);
        self.waiting.remove(&key).is_some()
    }

    /// How long a request just popped waited (AURORA_PROFILE only).
    fn waited(&mut self, key: u64) -> Option<Duration> {
        self.queued_at.remove(&key).map(|at| at.elapsed())
    }

    /// Lane of the most urgent request waiting.
    fn next_lane(&mut self) -> Option<Lane> {
        while let Some(n) = self.heap.peek() {
            if self.waiting.get(&n.key).is_some_and(|(_, g)| *g == n.generation) {
                return Some(n.lane);
            }
            self.heap.pop();
        }
        None
    }

    fn pop(&mut self) -> Option<FetchRequest> {
        self.next_lane()?;
        let n = self.heap.pop()?;
        self.waiting.remove(&n.key).map(|(r, _)| r)
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Request(r) => self.push(r),
            Msg::Priority(k, p) => {
                self.reprioritise(k, p);
            }
            Msg::Cancel(k) => {
                self.cancel(k);
            }
        }
    }
}

#[derive(Clone)]
pub struct Fetcher {
    tx: mpsc::UnboundedSender<Msg>,
}

impl Fetcher {
    /// `http` must not decode content transparently (gzip off): Firestorm
    /// hands asset bodies over as received.
    pub fn new(
        rt: &tokio::runtime::Handle,
        http: reqwest::Client,
        max_concurrent: usize,
        stats: Arc<NetStats>,
    ) -> (Fetcher, crossbeam_channel::Receiver<FetchResult>) {
        Self::with_policy(rt, http, max_concurrent, stats, RetryPolicy::FIRESTORM)
    }

    pub fn with_policy(
        rt: &tokio::runtime::Handle,
        http: reqwest::Client,
        max_concurrent: usize,
        stats: Arc<NetStats>,
        policy: RetryPolicy,
    ) -> (Fetcher, crossbeam_channel::Receiver<FetchResult>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (res_tx, res_rx) = crossbeam_channel::unbounded();
        rt.spawn(dispatcher(rx, res_tx, http, max_concurrent, stats, policy));
        (Fetcher { tx }, res_rx)
    }

    /// Queue a request (one still waiting under the same key is replaced).
    pub fn request(&self, r: FetchRequest) {
        let _ = self.tx.send(Msg::Request(r));
    }

    /// Give a request still waiting for a slot another priority; nothing
    /// happens once it is sent.
    pub fn reprioritise(&self, key: u64, priority: f32) {
        let _ = self.tx.send(Msg::Priority(key, priority));
    }

    /// Drop a request still waiting for a slot. One already sent runs to
    /// its end and its result is still delivered: the caller forgets the
    /// key to ignore it.
    pub fn cancel(&self, key: u64) {
        let _ = self.tx.send(Msg::Cancel(key));
    }
}

async fn dispatcher(
    mut rx: mpsc::UnboundedReceiver<Msg>,
    res_tx: crossbeam_channel::Sender<FetchResult>,
    http: reqwest::Client,
    max_concurrent: usize,
    stats: Arc<NetStats>,
    policy: RetryPolicy,
) {
    let sem = Arc::new(Semaphore::new(max_concurrent));
    let bakes = Arc::new(Semaphore::new(BAKE_SLOTS));
    let mut queue = FetchQueue::default();
    loop {
        // Drain incoming messages without blocking when we have work.
        loop {
            let msg = if queue.next_lane().is_none() {
                stats.http_queued.store(0, Ordering::Relaxed);
                match rx.recv().await {
                    Some(m) => m,
                    None => return,
                }
            } else {
                match rx.try_recv() {
                    Ok(m) => m,
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => return,
                }
            };
            queue.apply(msg);
        }
        stats.http_queued.store(queue.len() as u32, Ordering::Relaxed);
        probe::http_queue(queue.len());
        if queue.next_lane().is_none() {
            continue;
        }

        // Wait for a free slot, but keep accepting messages meanwhile: one
        // may bring a more urgent request, or rank the waiting ones anew.
        let permit = tokio::select! {
            biased;
            p = sem.clone().acquire_owned() => match p {
                Ok(p) => p,
                Err(_) => return,
            },
            m = rx.recv() => {
                match m {
                    Some(m) => queue.apply(m),
                    None => return,
                }
                continue;
            }
        };
        let Some(req) = queue.pop() else {
            drop(permit);
            continue;
        };
        // (AURORA_PROFILE: the kind of the request, its wait and its time)
        let asset_kind = HttpKind::of_asset(&req.url, req.accept);
        let kind = probe::enabled().then_some(asset_kind);
        if let (Some(kind), Some(waited)) = (kind, queue.waited(req.key)) {
            probe::http_waited(kind, waited);
        }
        let http = http.clone();
        let res_tx = res_tx.clone();
        let stats = stats.clone();
        let sem = sem.clone();
        let bakes = bakes.clone();
        tokio::spawn(async move {
            stats.http_in_flight.fetch_add(1, Ordering::Relaxed);
            let timed = kind.and_then(probe::HttpRequest::start);
            let _bake = if is_bake(&req.url) {
                bakes.acquire_owned().await.ok()
            } else {
                None
            };
            let result = do_fetch(&http, &req, policy, &sem, permit, &stats).await;
            stats.http_in_flight.fetch_sub(1, Ordering::Relaxed);
            if let Some(timed) = timed {
                let bytes = result.data.as_ref().map_or(0, Vec::len);
                timed.finish(result.status, bytes, result.data.is_ok());
            }
            match &result.data {
                Ok(d) => {
                    stats.http_bytes.fetch_add(d.len() as u64, Ordering::Relaxed);
                    stats.http_completed.fetch_add(1, Ordering::Relaxed);
                    // (the viewer statistics' texture and mesh downloads)
                    match asset_kind {
                        HttpKind::Texture => stats.texture_bytes.fetch_add(d.len() as u64, Ordering::Relaxed),
                        HttpKind::Mesh => stats.mesh_bytes.fetch_add(d.len() as u64, Ordering::Relaxed),
                        _ => 0,
                    };
                }
                Err(_) => {
                    stats.http_failed.fetch_add(1, Ordering::Relaxed);
                }
            }
            let _ = res_tx.send(result);
        });
    }
}

/// Outcome of one attempt: final, or worth a retry (with the server's
/// `Retry-After` delay when it gave one).
enum Attempt {
    Done(FetchResult),
    Retry {
        status: u16,
        why: String,
        retry_after: Option<u64>,
    },
}

async fn attempt(http: &reqwest::Client, req: &FetchRequest, attempts: u32) -> Attempt {
    // Like llcorehttp's GET: Accept from the caller, Range when asked, and
    // no Accept-Encoding (the client does not decode content).
    let mut rb = http
        .get(&req.url)
        .header(reqwest::header::ACCEPT, req.accept)
        .timeout(Duration::from_secs(60));
    if let Some(range) = req.range {
        rb = rb.header(reqwest::header::RANGE, range.header());
    }
    let fail = |status: u16, why: String| FetchResult {
        key: req.key,
        status,
        complete: true,
        offset: 0,
        data: Err(why),
    };
    let sent = Instant::now();
    let resp = match rb.send().await {
        Ok(resp) => resp,
        Err(e) => {
            let host = host_of(&req.url);
            probe::host_response(&host, 0, sent.elapsed(), 0);
            let why = format!("status none, {}, host {host}", describe_transport_error(&e));
            if transport_retryable(&e) {
                return Attempt::Retry {
                    status: 0,
                    why,
                    retry_after: None,
                };
            }
            return Attempt::Done(fail(0, format!("{why}, {attempts} attempt(s)")));
        }
    };
    let status = resp.status().as_u16();
    // after redirects: tells whether a CDN answered
    let host = resp.url().host_str().unwrap_or("?").to_string();
    note_protocol(&host, resp.version());
    let ttfb = sent.elapsed();
    if classify_status(status) != StatusClass::Success {
        probe::host_response(&host, status, ttfb, 0);
    }
    let reason = resp.status().canonical_reason().unwrap_or("");
    match classify_status(status) {
        StatusClass::Success => {}
        StatusClass::RangeNotSatisfiable => {
            // Firestorm keeps the data it has as complete; with nothing
            // yet, the load fails ("fail harder").
            if req.range.is_some_and(|r| r.start > 0) {
                return Attempt::Done(FetchResult {
                    key: req.key,
                    status,
                    complete: true,
                    offset: req.range.map_or(0, |r| r.start),
                    data: Ok(Vec::new()),
                });
            }
            return Attempt::Done(fail(
                status,
                format!("status {status} {reason} with no data, host {host}, {attempts} attempt(s)"),
            ));
        }
        StatusClass::Retryable => {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            return Attempt::Retry {
                status,
                why: format!("status {status} {reason}, host {host}"),
                retry_after,
            };
        }
        StatusClass::Failed => {
            return Attempt::Done(fail(
                status,
                format!("status {status} {reason}, host {host}, {attempts} attempt(s)"),
            ));
        }
    }
    if let Some(enc) = resp.headers().get(reqwest::header::CONTENT_ENCODING) {
        // Seen on some assets (Firestorm notes "binary/octet-stream"
        // from AWS); the body is used as is, like Firestorm does.
        static SEEN: AtomicBool = AtomicBool::new(false);
        if !SEEN.swap(true, Ordering::Relaxed) {
            log::info!(
                "asset response with Content-Encoding {:?} (host {host}): body used as received, like Firestorm",
                enc.to_str().unwrap_or("?")
            );
        }
    }
    let content_range = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_range);
    match resp.bytes().await {
        Ok(body) => {
            probe::host_response(&host, status, ttfb, body.len());
            let (offset, complete) = body_extent(status, req.range, content_range, body.len() as u64);
            Attempt::Done(FetchResult {
                key: req.key,
                status,
                complete,
                offset,
                data: Ok(body.to_vec()),
            })
        }
        Err(e) => {
            probe::host_response(&host, 0, ttfb, 0);
            let why = format!("status {status}, {}, host {host}", describe_transport_error(&e));
            if transport_retryable(&e) {
                Attempt::Retry {
                    status,
                    why,
                    retry_after: None,
                }
            } else {
                Attempt::Done(fail(status, format!("{why}, {attempts} attempt(s)")))
            }
        }
    }
}

/// Run the attempts of one request. The connection slot is released while
/// waiting for a retry (llcorehttp parks such requests in a retry queue).
async fn do_fetch(
    http: &reqwest::Client,
    req: &FetchRequest,
    policy: RetryPolicy,
    sem: &Arc<Semaphore>,
    permit: tokio::sync::OwnedSemaphorePermit,
    stats: &NetStats,
) -> FetchResult {
    let mut first = Some(permit);
    let mut retry = 0u32;
    loop {
        let slot = match first.take() {
            Some(p) => Some(p),
            None => sem.clone().acquire_owned().await.ok(),
        };
        match attempt(http, req, retry + 1).await {
            Attempt::Done(r) => return r,
            Attempt::Retry { status, why, retry_after } => {
                if retry >= policy.retries {
                    return FetchResult {
                        key: req.key,
                        status,
                        complete: true,
                        offset: 0,
                        data: Err(format!("{why}, {} attempt(s)", retry + 1)),
                    };
                }
                log::debug!("asset fetch {}: {why}, retry {}", req.key, retry + 1);
                stats.http_retries.fetch_add(1, Ordering::Relaxed);
                if probe::enabled() {
                    probe::http_retry(HttpKind::of_asset(&req.url, req.accept));
                }
                drop(slot);
                tokio::time::sleep(policy.delay(retry, retry_after)).await;
                retry += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn bakes_are_told_from_cdn_assets() {
        assert!(is_bake(
            "https://bake-texture.example/texture/0d7b2c3e-0000-0000-0000-000000000001/head/9a1c0000-0000-0000-0000-000000000002"
        ));
        assert!(!is_bake(
            "https://asset-cdn.example/?texture_id=9a1c0000-0000-0000-0000-000000000002"
        ));
        assert!(!is_bake("https://asset-cdn.example/?mesh_id=9a1c0000-0000-0000-0000-000000000002"));
    }

    #[test]
    fn range_header_matches_llcorehttp() {
        assert_eq!(ByteRange { start: 0, end: Some(599) }.header(), "bytes=0-599");
        assert_eq!(ByteRange { start: 4095, end: None }.header(), "bytes=4095-");
        assert_eq!(ByteRange { start: 10, end: Some(19) }.requested_len(), Some(10));
        assert_eq!(ByteRange { start: 10, end: None }.requested_len(), None);
    }

    fn request(key: u64, priority: f32, lane: Lane) -> FetchRequest {
        FetchRequest {
            key,
            url: String::new(),
            range: None,
            priority,
            lane,
            accept: "*/*",
        }
    }

    fn drain(q: &mut FetchQueue) -> Vec<u64> {
        std::iter::from_fn(|| q.pop().map(|r| r.key)).collect()
    }

    #[test]
    fn the_urgent_lane_goes_first_then_the_highest_rank_then_the_oldest() {
        let mut q = FetchQueue::default();
        q.push(request(1, 5000.0, Lane::Bulk));
        q.push(request(2, 1.0, Lane::Urgent));
        q.push(request(3, 9000.0, Lane::Bulk));
        q.push(request(4, 5000.0, Lane::Bulk));
        q.push(request(5, 3.0, Lane::Urgent));
        assert_eq!(q.next_lane(), Some(Lane::Urgent));
        assert_eq!(drain(&mut q), [5, 2, 3, 1, 4]);
        assert_eq!(q.next_lane(), None);
    }

    #[test]
    fn a_waiting_request_is_ranked_again_or_dropped() {
        let mut q = FetchQueue::default();
        for key in 1..=4 {
            q.push(request(key, key as f32, Lane::Bulk));
        }
        assert!(q.reprioritise(1, 100.0));
        assert!(q.cancel(3));
        assert!(!q.cancel(3));
        // the same key again replaces the waiting request
        q.push(request(2, 50.0, Lane::Bulk));
        assert_eq!(q.len(), 3);
        assert_eq!(q.pop().map(|r| (r.key, r.priority)), Some((1, 100.0)));
        // once sent, nothing more happens to it
        assert!(!q.reprioritise(1, 1.0));
        assert_eq!(drain(&mut q), [2, 4]);
    }

    #[test]
    fn places_left_by_new_ranks_are_swept() {
        let mut q = FetchQueue::default();
        q.push(request(1, 1.0, Lane::Bulk));
        q.push(request(2, 2.0, Lane::Bulk));
        for i in 0..1000 {
            q.reprioritise(1, i as f32);
        }
        assert!(q.heap.len() <= 2 * q.len() + 65, "{}", q.heap.len());
        assert_eq!(drain(&mut q), [1, 2]);
    }

    #[test]
    fn statuses_are_classified_like_firestorm() {
        assert_eq!(classify_status(200), StatusClass::Success);
        assert_eq!(classify_status(206), StatusClass::Success);
        assert_eq!(classify_status(416), StatusClass::RangeNotSatisfiable);
        assert_eq!(classify_status(404), StatusClass::Failed);
        assert_eq!(classify_status(403), StatusClass::Failed);
        assert_eq!(classify_status(429), StatusClass::Failed);
        for s in [499, 500, 502, 503, 504, 599] {
            assert_eq!(classify_status(s), StatusClass::Retryable, "{s}");
        }
    }

    #[test]
    fn retry_delays_follow_http_policy() {
        let p = RetryPolicy::FIRESTORM;
        let secs: Vec<u64> = (0..6).map(|r| p.delay(r, None).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 20]);
        assert_eq!(p.delay(0, Some(7)), Duration::from_secs(7));
        // out of range Retry-After values are ignored
        assert_eq!(p.delay(1, Some(0)), Duration::from_secs(2));
        assert_eq!(p.delay(1, Some(30)), Duration::from_secs(2));
        assert_eq!(p.delay(40, None), Duration::from_secs(20));
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 0-599/12345"),
            Some(ContentRange {
                first: 0,
                last: 599,
                total: Some(12345)
            })
        );
        assert_eq!(
            parse_content_range("bytes 100-199/*"),
            Some(ContentRange {
                first: 100,
                last: 199,
                total: None
            })
        );
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("bytes 9-1/10"), None);
        assert_eq!(parse_content_range("items 0-1/2"), None);
    }

    #[test]
    fn body_extent_handles_200_206_and_short_bodies() {
        let r = Some(ByteRange {
            start: 599,
            end: Some(4999),
        });
        // 200: the whole asset whatever the range
        assert_eq!(body_extent(200, r, None, 3000), (0, true));
        // 206 with Content-Range
        let cr = parse_content_range("bytes 599-4999/9000");
        assert_eq!(body_extent(206, r, cr, 4401), (599, false));
        let cr = parse_content_range("bytes 599-2999/3000");
        assert_eq!(body_extent(206, r, cr, 2401), (599, true));
        // 206 without Content-Range: assume what was asked, short = complete
        assert_eq!(body_extent(206, r, None, 4401), (599, false));
        assert_eq!(body_extent(206, r, None, 100), (599, true));
        // open range
        let open = Some(ByteRange { start: 99, end: None });
        assert_eq!(body_extent(206, open, None, 10), (99, true));
        let cr = parse_content_range("bytes 99-199/500");
        assert_eq!(body_extent(206, open, cr, 101), (99, false));
    }

    #[test]
    fn urls_are_reduced_to_their_host() {
        let msg = "error sending request for url (https://asset.example.org:443/cap/0123-secret/?texture_id=abc) after 3 tries";
        let out = redact_urls(msg);
        assert_eq!(out, "error sending request for url (https://asset.example.org:443…) after 3 tries");
        assert!(!out.contains("secret") && !out.contains("texture_id"));
        assert_eq!(redact_urls("no url here"), "no url here");
        assert_eq!(redact_urls("http://host"), "http://host");
    }

    /// Minimal HTTP/1.1 server: answers each connection with the next
    /// canned response and records the request heads.
    async fn serve(responses: Vec<Vec<u8>>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local server");
        let base = format!("http://{}", listener.local_addr().expect("local address"));
        let handle = tokio::spawn(async move {
            let mut heads = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("client");
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = socket.read(&mut buf).await.expect("read request");
                    assert_ne!(n, 0, "incomplete request");
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                heads.push(String::from_utf8_lossy(&request).into_owned());
                socket.write_all(&response).await.expect("write response");
                crate::test_http::finish(socket);
            }
            heads
        });
        (base, handle)
    }

    fn response(status: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
        let mut r = format!("HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for h in headers {
            r.push_str(h);
            r.push_str("\r\n");
        }
        r.push_str("\r\n");
        let mut r = r.into_bytes();
        r.extend_from_slice(body);
        r
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().gzip(false).build().expect("HTTP client")
    }

    const FAST: RetryPolicy = RetryPolicy {
        retries: 5,
        backoff_min: Duration::from_millis(1),
        backoff_max: Duration::from_millis(5),
    };

    async fn fetch(url: String, range: Option<ByteRange>, policy: RetryPolicy) -> FetchResult {
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.expect("permit");
        let req = FetchRequest {
            key: 7,
            url,
            range,
            priority: 1.0,
            lane: Lane::Bulk,
            accept: "image/x-j2c",
        };
        do_fetch(&client(), &req, policy, &sem, permit, &NetStats::default()).await
    }

    #[tokio::test]
    async fn partial_content_is_returned_with_its_offset() {
        let body = vec![7u8; 100];
        let (base, server) = serve(vec![response("206 Partial Content", &["Content-Range: bytes 599-698/699"], &body)]).await;
        let r = fetch(
            format!("{base}/cap/secret/?texture_id=1"),
            Some(ByteRange { start: 599, end: None }),
            FAST,
        )
        .await;
        assert_eq!(r.status, 206);
        assert_eq!(r.offset, 599);
        assert!(r.complete);
        assert_eq!(r.data.as_deref().ok(), Some(&body[..]));
        let heads = server.await.expect("server");
        let head = heads[0].to_ascii_lowercase();
        assert!(head.contains("range: bytes=599-\r\n"), "{head}");
        assert!(head.contains("accept: image/x-j2c\r\n"), "{head}");
        // like libcurl without CURLOPT_ENCODING
        assert!(!head.contains("accept-encoding"), "{head}");
    }

    #[tokio::test]
    async fn content_encoding_is_not_decoded() {
        // raw J2C announced as gzip: Firestorm uses the bytes as they are
        let body = b"\xff\x4f\xff\x51raw-j2c".to_vec();
        let (base, server) = serve(vec![response(
            "206 Partial Content",
            &["Content-Encoding: gzip", "Content-Range: bytes 0-10/11"],
            &body,
        )])
        .await;
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 0, end: Some(599) }), FAST).await;
        assert_eq!(r.data.as_deref().ok(), Some(&body[..]));
        assert!(r.complete);
        server.await.expect("server");
    }

    #[tokio::test]
    async fn service_unavailable_is_retried_then_succeeds() {
        let (base, server) = serve(vec![
            response("503 Service Unavailable", &["Retry-After: 0"], b""),
            response("502 Bad Gateway", &[], b""),
            response("200 OK", &[], b"whole"),
        ])
        .await;
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 0, end: Some(599) }), FAST).await;
        assert_eq!(r.status, 200);
        assert!(r.complete);
        assert_eq!(r.offset, 0);
        assert_eq!(r.data.as_deref().ok(), Some(&b"whole"[..]));
        assert_eq!(server.await.expect("server").len(), 3);
    }

    #[tokio::test]
    async fn retries_stop_after_the_policy_limit() {
        let policy = RetryPolicy { retries: 2, ..FAST };
        let (base, server) = serve(vec![response("503 Service Unavailable", &[], b""); 3]).await;
        let r = fetch(format!("{base}/cap/secret/?texture_id=1"), None, policy).await;
        assert_eq!(r.status, 503);
        let err = r.data.expect_err("failure");
        assert!(err.starts_with("status 503 Service Unavailable"), "{err}");
        assert!(err.contains("3 attempt(s)"), "{err}");
        assert!(err.contains("host 127.0.0.1"), "{err}");
        assert!(!err.contains("secret") && !err.contains("texture_id"), "{err}");
        assert_eq!(server.await.expect("server").len(), 3);
    }

    #[tokio::test]
    async fn not_found_is_not_retried() {
        let (base, server) = serve(vec![response("404 Not Found", &[], b"")]).await;
        let r = fetch(format!("{base}/a"), None, FAST).await;
        assert_eq!(r.status, 404);
        assert!(r.data.expect_err("missing").starts_with("status 404 Not Found"));
        assert_eq!(server.await.expect("server").len(), 1);
    }

    #[tokio::test]
    async fn range_not_satisfiable_completes_what_we_have() {
        let (base, server) = serve(vec![
            response("416 Range Not Satisfiable", &[], b""),
            response("416 Range Not Satisfiable", &[], b""),
        ])
        .await;
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 4999, end: None }), FAST).await;
        assert_eq!(r.status, 416);
        assert!(r.complete);
        assert_eq!(r.data.as_deref().ok(), Some(&b""[..]));
        // nothing at all: a failure, not an empty complete texture
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 0, end: Some(599) }), FAST).await;
        assert!(r.data.is_err());
        server.await.expect("server");
    }

    #[tokio::test]
    async fn redirects_are_followed_with_the_range() {
        let (target, target_server) = serve(vec![response("206 Partial Content", &["Content-Range: bytes 0-3/4"], b"data")]).await;
        let location = format!("Location: {target}/cdn/object");
        let (base, server) = serve(vec![response("302 Found", &[&location], b"")]).await;
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 0, end: Some(599) }), FAST).await;
        assert_eq!(r.status, 206);
        assert_eq!(r.data.as_deref().ok(), Some(&b"data"[..]));
        server.await.expect("server");
        let heads = target_server.await.expect("target");
        assert!(heads[0].to_ascii_lowercase().contains("range: bytes=0-599\r\n"), "{}", heads[0]);
    }

    #[tokio::test]
    async fn truncated_body_is_a_retried_transport_error() {
        let mut short = b"HTTP/1.1 206 Partial Content\r\nConnection: close\r\nContent-Length: 100\r\n\r\n".to_vec();
        short.extend_from_slice(b"only ten b");
        let policy = RetryPolicy { retries: 1, ..FAST };
        let (base, server) = serve(vec![short.clone(), short]).await;
        let r = fetch(format!("{base}/a"), Some(ByteRange { start: 0, end: Some(99) }), policy).await;
        let err = r.data.expect_err("short body");
        assert!(err.starts_with("status 206, body"), "{err}");
        assert!(err.contains("2 attempt(s)"), "{err}");
        server.await.expect("server");
    }
}
