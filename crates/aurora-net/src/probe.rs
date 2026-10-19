//! Detailed probes of the network side (AURORA_PROFILE): every UDP message
//! type in and out, the kinds of object updates, and the HTTP requests by
//! kind of asset and by host. Off by default: each counting point first
//! reads one flag ([`enabled`]), and costs a few relaxed atomic additions
//! once it is on; nothing allocates or locks per packet. The viewer turns
//! it on and writes what [`take_messages`], [`take_objects`] and
//! [`take_http`] return as its `msg summary` and `http summary` lines, every
//! 10 s (deep_profile.rs; docs/TESTING.md#profiling lists the fields).

use aurora_msg::MsgId;
use parking_lot::Mutex;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turn the probes on (AURORA_PROFILE): from now on every counting point
/// counts.
pub fn enable() {
    ENABLED.store(true, Relaxed);
}

/// Turn the probes on or off (the resource monitor, while it is open).
/// Requests started while they counted still leave the in-flight count.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Relaxed);
}

/// The probes count (one relaxed load: the whole cost when they are off).
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Relaxed)
}

// ------------------------------------------------------------ durations

/// Buckets of a [`Hist`]: a quarter of a power of two each, from 50 µs
/// (bucket 0 holds everything below) to about 50 s.
pub const HIST_BUCKETS: usize = 80;
const HIST_FIRST_MS: f64 = 0.05;

/// Distribution of durations, lock-free: one atomic bucket per quarter of
/// a power of two, so a percentile is known within 19 % (it is read as the
/// top of its bucket, never above the true maximum).
pub struct Hist {
    buckets: [AtomicU32; HIST_BUCKETS],
    max_us: AtomicU64,
}

impl Default for Hist {
    fn default() -> Self {
        Hist::new()
    }
}

impl Hist {
    pub const fn new() -> Self {
        Hist {
            buckets: [const { AtomicU32::new(0) }; HIST_BUCKETS],
            max_us: AtomicU64::new(0),
        }
    }

    fn bucket(ms: f64) -> usize {
        if ms <= HIST_FIRST_MS {
            return 0;
        }
        ((ms / HIST_FIRST_MS).log2() * 4.0).ceil().clamp(0.0, (HIST_BUCKETS - 1) as f64) as usize
    }

    pub fn add(&self, d: Duration) {
        let ms = d.as_secs_f64() * 1000.0;
        self.buckets[Self::bucket(ms)].fetch_add(1, Relaxed);
        self.max_us.fetch_max(d.as_micros().min(u64::MAX as u128) as u64, Relaxed);
    }

    /// What was added since the last take, and start again from nothing.
    pub fn take(&self) -> Durations {
        Durations {
            buckets: std::array::from_fn(|i| self.buckets[i].swap(0, Relaxed)),
            max_ms: self.max_us.swap(0, Relaxed) as f32 / 1000.0,
        }
    }
}

/// A period of a [`Hist`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Durations {
    pub buckets: [u32; HIST_BUCKETS],
    pub max_ms: f32,
}

impl Default for Durations {
    fn default() -> Self {
        Durations {
            buckets: [0; HIST_BUCKETS],
            max_ms: 0.0,
        }
    }
}

impl Durations {
    pub fn count(&self) -> u64 {
        self.buckets.iter().map(|&n| u64::from(n)).sum()
    }

    /// The duration under which `p` percent of them fall (ms; 0 when
    /// there are none): the top of its bucket, at most the maximum.
    pub fn percentile(&self, p: f32) -> f32 {
        let n = self.count();
        if n == 0 {
            return 0.0;
        }
        let rank = ((n as f64 * f64::from(p) / 100.0).ceil() as u64).clamp(1, n);
        let mut seen = 0u64;
        for (i, &c) in self.buckets.iter().enumerate() {
            seen += u64::from(c);
            if seen >= rank {
                let top = HIST_FIRST_MS * 2f64.powf(i as f64 / 4.0);
                return (top as f32).min(self.max_ms);
            }
        }
        self.max_ms
    }

    pub fn merge(&mut self, other: &Durations) {
        for (a, b) in self.buckets.iter_mut().zip(other.buckets) {
            *a += b;
        }
        self.max_ms = self.max_ms.max(other.max_ms);
    }

    /// `n=12 p50=3.4 p95=12.1 max=20.3` (ms), or `n=0` when empty.
    pub fn describe(&self) -> String {
        let n = self.count();
        if n == 0 {
            return "n=0".to_string();
        }
        format!(
            "n={n} p50={:.1} p95={:.1} max={:.1}",
            self.percentile(50.0),
            self.percentile(95.0),
            self.max_ms
        )
    }
}

// ------------------------------------------------------------- messages

/// Slots of the message table: High 1..255, Medium 1..255, Low 1..511 and
/// the Fixed messages (Low 0xFFxx).
const SLOTS: usize = 256 + 256 + 512 + 256;

fn slot(id: MsgId) -> Option<usize> {
    match id {
        MsgId::High(n) => Some(usize::from(n)),
        MsgId::Medium(n) => Some(256 + usize::from(n)),
        MsgId::Low(n) if n >= 0xFF00 => Some(1024 + usize::from(n & 0xFF)),
        MsgId::Low(n) if n < 512 => Some(512 + usize::from(n)),
        MsgId::Low(_) => None,
    }
}

fn id_of(slot: usize) -> MsgId {
    match slot {
        0..256 => MsgId::High(slot as u8),
        256..512 => MsgId::Medium((slot - 256) as u8),
        512..1024 => MsgId::Low((slot - 512) as u16),
        _ => MsgId::Low(0xFF00 | (slot - 1024) as u16),
    }
}

struct MsgTable {
    count: [[AtomicU64; SLOTS]; 2],
    bytes: [[AtomicU64; SLOTS]; 2],
}

static MESSAGES: MsgTable = MsgTable {
    count: [const { [const { AtomicU64::new(0) }; SLOTS] }; 2],
    bytes: [const { [const { AtomicU64::new(0) }; SLOTS] }; 2],
};

fn count_message(dir: usize, id: MsgId, bytes: usize) {
    if let Some(s) = slot(id) {
        MESSAGES.count[dir][s].fetch_add(1, Relaxed);
        MESSAGES.bytes[dir][s].fetch_add(bytes as u64, Relaxed);
    }
}

/// A packet came in carrying message `id`, `bytes` long on the wire.
#[inline]
pub fn message_in(id: MsgId, bytes: usize) {
    if enabled() {
        count_message(0, id, bytes);
    }
}

/// A packet left carrying the message whose body starts `payload` (its
/// number first), `bytes` long on the wire (resends included).
#[inline]
pub fn message_out(payload: &[u8], bytes: usize) {
    if enabled()
        && let Ok((id, _)) = MsgId::decode(payload)
    {
        count_message(1, id, bytes);
    }
}

/// One message type over a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageCount {
    pub name: &'static str,
    pub count: u64,
    pub bytes: u64,
}

/// The messages of the period, in then out, each list from the most bytes
/// down (types not seen left out); the counts start again from nothing.
pub fn take_messages() -> [Vec<MessageCount>; 2] {
    std::array::from_fn(|dir| {
        let mut out: Vec<MessageCount> = (0..SLOTS)
            .filter_map(|s| {
                let count = MESSAGES.count[dir][s].swap(0, Relaxed);
                let bytes = MESSAGES.bytes[dir][s].swap(0, Relaxed);
                (count > 0).then(|| MessageCount {
                    name: aurora_msg::message_info(id_of(s)).map_or("?", |i| i.name),
                    count,
                    bytes,
                })
            })
            .collect();
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.count.cmp(&a.count)));
        out
    })
}

/// Adds the messages of `other` to `acc` (the same lists for a longer
/// period), each list still from the most bytes down.
pub fn merge_messages(acc: &mut [Vec<MessageCount>; 2], other: &[Vec<MessageCount>; 2]) {
    for (list, more) in acc.iter_mut().zip(other) {
        for m in more {
            match list.iter_mut().find(|x| x.name == m.name) {
                Some(x) => {
                    x.count += m.count;
                    x.bytes += m.bytes;
                }
                None => list.push(*m),
            }
        }
        list.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.count.cmp(&a.count)));
    }
}

/// Kinds of object updates, as the summary names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// ObjectUpdate: objects in full, uncompressed.
    Full,
    /// ObjectUpdateCompressed: objects in full, compressed.
    Compressed,
    /// ImprovedTerseObjectUpdate: moves.
    Terse,
    /// ObjectUpdateCached: objects announced by their cache entry.
    Cached,
    /// Of those, found in the object cache with the same CRC…
    CacheHit,
    /// …or not found.
    CacheMiss,
    /// Objects asked for in full (RequestMultipleObjects).
    Requested,
    /// KillObject: objects removed.
    Killed,
}

const OBJECT_KINDS: [(ObjectKind, &str); 8] = [
    (ObjectKind::Full, "full"),
    (ObjectKind::Compressed, "compressed"),
    (ObjectKind::Terse, "terse"),
    (ObjectKind::Cached, "cached"),
    (ObjectKind::CacheHit, "cache_hit"),
    (ObjectKind::CacheMiss, "cache_miss"),
    (ObjectKind::Requested, "requested"),
    (ObjectKind::Killed, "killed"),
];

static OBJECTS: [AtomicU64; 8] = [const { AtomicU64::new(0) }; 8];

/// `n` objects of that kind of update.
#[inline]
pub fn objects(kind: ObjectKind, n: usize) {
    if enabled() {
        OBJECTS[kind as usize].fetch_add(n as u64, Relaxed);
    }
}

/// Object updates of the period by kind ([`ObjectKind`] order).
pub fn take_objects() -> [u64; 8] {
    std::array::from_fn(|i| OBJECTS[i].swap(0, Relaxed))
}

/// The `msg summary` line of a period of `secs`: the `top` message types
/// in and out by bytes (count and kilobytes over the period), then the
/// object updates by kind.
pub fn message_line(messages: &[Vec<MessageCount>; 2], objects: &[u64; 8], secs: f32, top: usize) -> String {
    let mut out = String::new();
    for (dir, list) in ["in", "out"].iter().zip(messages) {
        let (n, bytes) = list.iter().fold((0, 0), |(n, b), m| (n + m.count, b + m.bytes));
        let _ = write!(
            out,
            "{}{dir} {:.0} msg/s {:.1} kb/s:",
            if out.is_empty() { "" } else { " | " },
            n as f32 / secs.max(1e-3),
            bytes as f32 * 8.0 / 1000.0 / secs.max(1e-3)
        );
        if list.is_empty() {
            out.push_str(" -");
        }
        for m in list.iter().take(top) {
            let _ = write!(out, " {}={}/{:.1}k", m.name, m.count, m.bytes as f32 / 1024.0);
        }
        if list.len() > top {
            let rest = &list[top..];
            let _ = write!(
                out,
                " others={}/{:.1}k",
                rest.iter().map(|m| m.count).sum::<u64>(),
                rest.iter().map(|m| m.bytes).sum::<u64>() as f32 / 1024.0
            );
        }
    }
    out.push_str(" | objects");
    for ((_, key), n) in OBJECT_KINDS.iter().zip(objects) {
        let _ = write!(out, " {key}={n}");
    }
    out
}

// ----------------------------------------------------------------- HTTP

/// What an HTTP request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpKind {
    Texture,
    Mesh,
    Animation,
    Sound,
    Material,
    Settings,
    Landmark,
    /// Another asset of the fetcher (a bake, a kind not listed above).
    Asset,
    /// Inventory folder fetches (FetchInventoryDescendents2, FetchLib…).
    Inventory,
    /// Other LLSD capability calls (caps.rs).
    Cap,
}

pub const HTTP_KINDS: [HttpKind; 10] = [
    HttpKind::Texture,
    HttpKind::Mesh,
    HttpKind::Animation,
    HttpKind::Sound,
    HttpKind::Material,
    HttpKind::Settings,
    HttpKind::Landmark,
    HttpKind::Asset,
    HttpKind::Inventory,
    HttpKind::Cap,
];

impl HttpKind {
    pub fn key(self) -> &'static str {
        match self {
            HttpKind::Texture => "texture",
            HttpKind::Mesh => "mesh",
            HttpKind::Animation => "anim",
            HttpKind::Sound => "sound",
            HttpKind::Material => "material",
            HttpKind::Settings => "settings",
            HttpKind::Landmark => "landmark",
            HttpKind::Asset => "asset",
            HttpKind::Inventory => "inventory",
            HttpKind::Cap => "cap",
        }
    }

    /// The kind of an asset request of the fetcher, from the parameter of
    /// its ViewerAsset URL (`?texture_id=`…) or what it accepts.
    pub fn of_asset(url: &str, accept: &str) -> HttpKind {
        let param = url
            .rsplit_once('?')
            .and_then(|(_, q)| q.split_once('='))
            .map_or("", |(name, _)| name);
        match param {
            "texture_id" => HttpKind::Texture,
            "mesh_id" => HttpKind::Mesh,
            "animatn_id" => HttpKind::Animation,
            "sound_id" => HttpKind::Sound,
            "material_id" => HttpKind::Material,
            "settings_id" => HttpKind::Settings,
            "landmark_id" => HttpKind::Landmark,
            _ if accept == "image/x-j2c" => HttpKind::Texture,
            _ if accept == "application/vnd.ll.mesh" => HttpKind::Mesh,
            _ => HttpKind::Asset,
        }
    }
}

/// Failure classes of a finished request: no response, 4xx, 5xx, other.
const FAIL_CLASSES: [&str; 4] = ["none", "4xx", "5xx", "other"];

struct HttpCounters {
    started: AtomicU64,
    done: AtomicU64,
    failed: [AtomicU64; 4],
    retries: AtomicU64,
    bytes: AtomicU64,
    in_flight: AtomicU32,
    in_flight_peak: AtomicU32,
    /// From the start of the request (its slot in the fetcher) to its
    /// result, retries included.
    latency: Hist,
    /// Fetcher: time waited in the queue for a connection slot.
    wait: Hist,
}

impl HttpCounters {
    const fn new() -> Self {
        HttpCounters {
            started: AtomicU64::new(0),
            done: AtomicU64::new(0),
            failed: [const { AtomicU64::new(0) }; 4],
            retries: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            in_flight: AtomicU32::new(0),
            in_flight_peak: AtomicU32::new(0),
            latency: Hist::new(),
            wait: Hist::new(),
        }
    }
}

static HTTP: [HttpCounters; HTTP_KINDS.len()] = [const { HttpCounters::new() }; HTTP_KINDS.len()];
/// The most requests the fetcher had waiting for a slot.
static QUEUE_PEAK: AtomicU32 = AtomicU32::new(0);

/// A request under way, counted from [`HttpRequest::start`] to
/// [`HttpRequest::finish`] (or its drop, for a cancelled one: it then
/// leaves the in-flight count without a result).
pub struct HttpRequest {
    kind: HttpKind,
    at: Instant,
}

impl HttpRequest {
    /// None while the probes are off.
    pub fn start(kind: HttpKind) -> Option<HttpRequest> {
        if !enabled() {
            return None;
        }
        let c = &HTTP[kind as usize];
        c.started.fetch_add(1, Relaxed);
        let n = c.in_flight.fetch_add(1, Relaxed) + 1;
        c.in_flight_peak.fetch_max(n, Relaxed);
        Some(HttpRequest { kind, at: Instant::now() })
    }

    /// The request ended with `status` (0: no response) and `bytes` of
    /// body; `ok` when its data is usable.
    pub fn finish(self, status: u16, bytes: usize, ok: bool) {
        let c = &HTTP[self.kind as usize];
        c.latency.add(self.at.elapsed());
        c.bytes.fetch_add(bytes as u64, Relaxed);
        if ok {
            c.done.fetch_add(1, Relaxed);
        } else {
            let class = match status {
                0 => 0,
                400..=499 => 1,
                500..=599 => 2,
                _ => 3,
            };
            c.failed[class].fetch_add(1, Relaxed);
        }
    }
}

impl Drop for HttpRequest {
    fn drop(&mut self) {
        HTTP[self.kind as usize].in_flight.fetch_sub(1, Relaxed);
    }
}

/// A fetcher request of that kind waited `waited` for its slot.
pub fn http_waited(kind: HttpKind, waited: Duration) {
    if enabled() {
        HTTP[kind as usize].wait.add(waited);
    }
}

/// A request of that kind is sent again after a retryable failure.
pub fn http_retry(kind: HttpKind) {
    if enabled() {
        HTTP[kind as usize].retries.fetch_add(1, Relaxed);
    }
}

/// The fetcher has `len` requests waiting for a slot.
#[inline]
pub fn http_queue(len: usize) {
    if enabled() {
        QUEUE_PEAK.fetch_max(len.min(u32::MAX as usize) as u32, Relaxed);
    }
}

/// One host over a period.
#[derive(Debug, Clone, PartialEq)]
pub struct HostCount {
    pub host: String,
    pub responses: u64,
    /// Responses with a status of 400 or more, and requests that got none.
    pub errors: u64,
    pub bytes: u64,
    /// Time from sending to the response headers, summed and longest (ms).
    pub ttfb_ms: f64,
    pub ttfb_max_ms: f32,
}

static HOSTS: Mutex<Vec<HostCount>> = Mutex::new(Vec::new());

/// An asset host answered (`status` 0: it did not) after `ttfb`, with
/// `bytes` of body. One short lock per response, only while the probes
/// are on.
pub fn host_response(host: &str, status: u16, ttfb: Duration, bytes: usize) {
    if !enabled() {
        return;
    }
    let ms = ttfb.as_secs_f64() * 1000.0;
    let mut hosts = HOSTS.lock();
    let row = match hosts.iter().position(|h| h.host == host) {
        Some(i) => &mut hosts[i],
        None => {
            hosts.push(HostCount {
                host: host.to_string(),
                responses: 0,
                errors: 0,
                bytes: 0,
                ttfb_ms: 0.0,
                ttfb_max_ms: 0.0,
            });
            let last = hosts.len() - 1;
            &mut hosts[last]
        }
    };
    row.responses += 1;
    row.errors += u64::from(status == 0 || status >= 400);
    row.bytes += bytes as u64;
    row.ttfb_ms += ms;
    row.ttfb_max_ms = row.ttfb_max_ms.max(ms as f32);
}

/// One kind of request over a period.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HttpPeriod {
    pub started: u64,
    pub done: u64,
    pub failed: [u64; 4],
    pub retries: u64,
    pub bytes: u64,
    pub in_flight: u32,
    pub in_flight_peak: u32,
    pub latency: Durations,
    pub wait: Durations,
}

/// Everything HTTP over a period.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HttpTotals {
    pub kinds: [HttpPeriod; HTTP_KINDS.len()],
    pub queue_peak: u32,
    /// From the most bytes down.
    pub hosts: Vec<HostCount>,
}

impl HttpTotals {
    /// Adds a later period: sums, the latest in-flight counts, the peaks.
    pub fn merge(&mut self, later: &HttpTotals) {
        for (a, b) in self.kinds.iter_mut().zip(&later.kinds) {
            a.started += b.started;
            a.done += b.done;
            for (x, y) in a.failed.iter_mut().zip(b.failed) {
                *x += y;
            }
            a.retries += b.retries;
            a.bytes += b.bytes;
            a.in_flight = b.in_flight;
            a.in_flight_peak = a.in_flight_peak.max(b.in_flight_peak);
            a.latency.merge(&b.latency);
            a.wait.merge(&b.wait);
        }
        self.queue_peak = self.queue_peak.max(later.queue_peak);
        for h in &later.hosts {
            match self.hosts.iter_mut().find(|x| x.host == h.host) {
                Some(x) => {
                    x.responses += h.responses;
                    x.errors += h.errors;
                    x.bytes += h.bytes;
                    x.ttfb_ms += h.ttfb_ms;
                    x.ttfb_max_ms = x.ttfb_max_ms.max(h.ttfb_max_ms);
                }
                None => self.hosts.push(h.clone()),
            }
        }
        self.hosts.sort_by_key(|h| std::cmp::Reverse(h.bytes));
    }
}

/// The HTTP requests of the period; the counts start again from nothing
/// (the in-flight peaks from what is in flight now).
pub fn take_http() -> HttpTotals {
    let kinds = std::array::from_fn(|i| {
        let c = &HTTP[i];
        let in_flight = c.in_flight.load(Relaxed);
        HttpPeriod {
            started: c.started.swap(0, Relaxed),
            done: c.done.swap(0, Relaxed),
            failed: std::array::from_fn(|k| c.failed[k].swap(0, Relaxed)),
            retries: c.retries.swap(0, Relaxed),
            bytes: c.bytes.swap(0, Relaxed),
            in_flight,
            in_flight_peak: c.in_flight_peak.swap(in_flight, Relaxed),
            latency: c.latency.take(),
            wait: c.wait.take(),
        }
    });
    let mut hosts = std::mem::take(&mut *HOSTS.lock());
    hosts.sort_by_key(|h| std::cmp::Reverse(h.bytes));
    HttpTotals {
        kinds,
        queue_peak: QUEUE_PEAK.swap(0, Relaxed),
        hosts,
    }
}

/// The `http summary` line of a period of `secs`, `queued` requests
/// waiting at its end: one segment per kind that saw a request (latency
/// and queue wait in ms, kilobytes per second), then the `hosts` busiest
/// hosts.
pub fn http_line(t: &HttpTotals, queued: u32, secs: f32, hosts: usize) -> String {
    let secs = secs.max(1e-3);
    let mut out = format!("queued={queued} queue_peak={}", t.queue_peak);
    let mut any = false;
    for (kind, p) in HTTP_KINDS.iter().zip(&t.kinds) {
        if p.started == 0 && p.in_flight_peak == 0 && p.done == 0 && p.failed.iter().all(|&f| f == 0) {
            continue;
        }
        any = true;
        let _ = write!(
            out,
            " | {} started={} done={} failed={}",
            kind.key(),
            p.started,
            p.done,
            p.failed.iter().sum::<u64>()
        );
        for (class, n) in FAIL_CLASSES.iter().zip(p.failed) {
            if n > 0 {
                let _ = write!(out, " failed_{class}={n}");
            }
        }
        let _ = write!(
            out,
            " retries={} {:.0} kb/s in_flight={} peak={} latency {} wait {}",
            p.retries,
            p.bytes as f32 * 8.0 / 1000.0 / secs,
            p.in_flight,
            p.in_flight_peak,
            p.latency.describe(),
            p.wait.describe(),
        );
    }
    if !any {
        out.push_str(" | -");
    }
    out.push_str(" | hosts:");
    if t.hosts.is_empty() {
        out.push_str(" -");
    }
    for h in t.hosts.iter().take(hosts) {
        let _ = write!(
            out,
            " {}={}/{:.0}k errors={} ttfb_avg={:.0} ttfb_max={:.0}",
            h.host,
            h.responses,
            h.bytes as f32 / 1024.0,
            h.errors,
            h.ttfb_ms / h.responses.max(1) as f64,
            h.ttfb_max_ms
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn percentiles_are_read_within_a_bucket_and_never_above_the_maximum() {
        let h = Hist::new();
        for _ in 0..90 {
            h.add(ms(10));
        }
        for _ in 0..10 {
            h.add(ms(200));
        }
        let d = h.take();
        assert_eq!(d.count(), 100);
        let p50 = d.percentile(50.0);
        assert!((10.0..=11.9).contains(&p50), "{p50}");
        let p95 = d.percentile(95.0);
        assert!((200.0..=200.0 * 1.19).contains(&p95) && p95 <= d.max_ms, "{p95}");
        assert_eq!(d.max_ms, 200.0);
        assert_eq!(d.describe(), format!("n=100 p50={p50:.1} p95={p95:.1} max=200.0"));
        // taken: the next period starts from nothing
        assert_eq!(h.take(), Durations::default());
        assert_eq!(Durations::default().describe(), "n=0");
        // tiny and huge durations land in the end buckets
        h.add(Duration::from_micros(1));
        h.add(Duration::from_secs(3600));
        let d = h.take();
        assert_eq!((d.buckets[0], d.buckets[HIST_BUCKETS - 1]), (1, 1));
    }

    #[test]
    fn every_message_number_has_a_slot_of_its_own() {
        for id in [
            MsgId::High(1),
            MsgId::High(255),
            MsgId::Medium(7),
            MsgId::Low(1),
            MsgId::Low(431),
            MsgId::Low(0xFFFB),
        ] {
            let s = slot(id).expect("a slot");
            assert!(s < SLOTS);
            assert_eq!(id_of(s), id);
        }
        assert_eq!(slot(MsgId::Low(5000)), None);
    }

    #[test]
    fn the_message_line_lists_the_heaviest_types_first() {
        let m = |name, count, bytes| MessageCount { name, count, bytes };
        let messages = [
            vec![m("ObjectUpdate", 100, 102_400), m("PacketAck", 50, 1024), m("LayerData", 2, 512)],
            vec![],
        ];
        let line = message_line(&messages, &[5, 6, 7, 8, 9, 1, 1, 2], 10.0, 2);
        assert_eq!(
            line,
            "in 15 msg/s 83.1 kb/s: ObjectUpdate=100/100.0k PacketAck=50/1.0k others=2/0.5k | out 0 msg/s 0.0 kb/s: - \
             | objects full=5 compressed=6 terse=7 cached=8 cache_hit=9 cache_miss=1 requested=1 killed=2"
        );
    }

    #[test]
    fn merged_periods_add_up() {
        let m = |name, count, bytes| MessageCount { name, count, bytes };
        let mut acc = [vec![m("ObjectUpdate", 10, 1000)], vec![]];
        merge_messages(
            &mut acc,
            &[
                vec![m("PacketAck", 50, 2000), m("ObjectUpdate", 5, 500)],
                vec![m("AgentUpdate", 1, 60)],
            ],
        );
        assert_eq!(acc[0], [m("PacketAck", 50, 2000), m("ObjectUpdate", 15, 1500)]);
        assert_eq!(acc[1], [m("AgentUpdate", 1, 60)]);

        let mut a = HttpTotals::default();
        a.kinds[0].started = 3;
        a.kinds[0].in_flight = 2;
        a.kinds[0].in_flight_peak = 5;
        let mut b = HttpTotals::default();
        b.kinds[0].started = 4;
        b.kinds[0].in_flight = 1;
        b.kinds[0].in_flight_peak = 3;
        b.queue_peak = 9;
        a.merge(&b);
        assert_eq!(
            (a.kinds[0].started, a.kinds[0].in_flight, a.kinds[0].in_flight_peak, a.queue_peak),
            (7, 1, 5, 9)
        );
    }

    #[test]
    fn asset_kinds_come_from_the_url_parameter() {
        assert_eq!(HttpKind::of_asset("https://h/cap/x?texture_id=1", "image/x-j2c"), HttpKind::Texture);
        assert_eq!(HttpKind::of_asset("https://h/cap/x?mesh_id=1", "*/*"), HttpKind::Mesh);
        assert_eq!(HttpKind::of_asset("https://h/cap/x?animatn_id=1", "*/*"), HttpKind::Animation);
        // a bake: no parameter, a texture by what it accepts
        assert_eq!(HttpKind::of_asset("https://bake/texture/a/b/c", "image/x-j2c"), HttpKind::Texture);
        assert_eq!(HttpKind::of_asset("https://h/x", "*/*"), HttpKind::Asset);
    }

    #[test]
    fn the_http_line_has_a_segment_per_kind_seen_and_the_busiest_hosts() {
        let mut t = HttpTotals::default();
        let tex = &mut t.kinds[HttpKind::Texture as usize];
        tex.started = 40;
        tex.done = 38;
        tex.failed = [1, 0, 1, 0];
        tex.retries = 2;
        tex.bytes = 1_250_000;
        tex.in_flight = 3;
        tex.in_flight_peak = 24;
        t.queue_peak = 300;
        t.hosts.push(HostCount {
            host: "asset-cdn".into(),
            responses: 39,
            errors: 1,
            bytes: 1_250_000,
            ttfb_ms: 3900.0,
            ttfb_max_ms: 450.0,
        });
        let line = http_line(&t, 120, 10.0, 3);
        assert_eq!(
            line,
            "queued=120 queue_peak=300 | texture started=40 done=38 failed=2 failed_none=1 failed_5xx=1 retries=2 1000 kb/s \
             in_flight=3 peak=24 latency n=0 wait n=0 | hosts: asset-cdn=39/1221k errors=1 ttfb_avg=100 ttfb_max=450"
        );
        assert_eq!(
            http_line(&HttpTotals::default(), 0, 10.0, 3),
            "queued=0 queue_peak=0 | - | hosts: -"
        );
    }
}
