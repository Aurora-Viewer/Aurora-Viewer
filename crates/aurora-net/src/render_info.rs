//! The avatar render info of each region (`AvatarRenderInfo` capability):
//! the viewer tells every region the complexity of the avatars it sees there
//! and which ones it draws as silhouettes, and reads back how many of the
//! viewers around draw our own avatar as one. That count is what the
//! "people may not see you fully" notice says, in our viewer and in the
//! others'.
//!
//! Port of LLAvatarRenderInfoAccountant
//! (indra/newview/llavatarrenderinfoaccountant.cpp, originally LGPL 2.1): the
//! same keys, periods and conditions, which the region expects. The pure
//! parts are here; session/render_info_net.rs sends the requests off the
//! session loop.

use aurora_llsd::{Llsd, Map};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::RegionHandle;

const KEY_AGENTS: &str = "agents";
const KEY_WEIGHT: &str = "weight";
const KEY_TOO_COMPLEX: &str = "tooComplex";
const KEY_OVER_COMPLEXITY_LIMIT: &str = "overlimit";
const KEY_REPORTING_COMPLEXITY_LIMIT: &str = "reportinglimit";
const KEY_ERROR: &str = "error";

/// SECS_BETWEEN_REGION_SCANS: the regions are looked at every 5 s.
pub const SCAN_PERIOD: Duration = Duration::from_secs(5);
/// SECS_BETWEEN_REGION_REQUEST: each region is asked every 15 s, and the
/// GET gives up after as long (no retry: the next one comes soon).
pub const REQUEST_PERIOD: Duration = Duration::from_secs(15);
/// SECS_BETWEEN_REGION_REPORTS: each region is told every 60 s, 2 s more
/// for every avatar in the last report.
pub const REPORT_PERIOD: Duration = Duration::from_secs(60);
/// The POST's time limit: llcorehttp's default (HTTP_REQUEST_TIMEOUT_DEFAULT).
pub const REPORT_TIMEOUT: Duration = Duration::from_secs(30);
/// The POST is tried once more on a failure (`setRetries(1)`), after
/// llcorehttp's shortest back-off.
pub const REPORT_RETRY_DELAY: Duration = Duration::from_secs(1);

/// An avatar the viewer sees, as the report needs it. The viewer lists only
/// the avatars LLAvatarRenderInfoAccountant::avatarRenderInfoReportCoro
/// keeps: not an animated object, mostly loaded (`getRezzedStatus() >= 2`:
/// its bakes shown), our own included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeenAvatar {
    pub id: Uuid,
    /// The region its object came from (`getObjectHost()`).
    pub region: RegionHandle,
    /// LLVOAvatar::getVisualComplexity (0 while not computed).
    pub weight: u32,
    /// Drawn as a silhouette (LLVOAvatar::isTooComplex).
    pub too_complex: bool,
}

/// The body posted to `region`'s capability and the number of avatars in
/// it: those of that region with a complexity (`avatar_complexity > 0`).
/// LLSD integers are signed: a weight above `i32::MAX` is sent as it.
pub fn report(avatars: &[SeenAvatar], region: RegionHandle) -> (Llsd, usize) {
    let mut agents = Map::new();
    for a in avatars.iter().filter(|a| a.region == region && a.weight > 0) {
        let mut info = Map::new();
        info.insert(KEY_WEIGHT.into(), Llsd::Integer(i32::try_from(a.weight).unwrap_or(i32::MAX)));
        info.insert(KEY_TOO_COMPLEX.into(), Llsd::Boolean(a.too_complex));
        agents.insert(a.id.to_string(), Llsd::Map(info));
    }
    let count = agents.len();
    let mut body = Map::new();
    body.insert(KEY_AGENTS.into(), Llsd::Map(agents));
    (Llsd::Map(body), count)
}

/// When the next report to a region is due after one listing `avatars`:
/// "moving to longer intervals if there are lots of avatars around".
pub fn report_delay(avatars: usize) -> Duration {
    REPORT_PERIOD + Duration::from_secs(2 * avatars as u64)
}

/// What a region answers about us: how many viewers around report on our
/// avatar, and how many of them draw it as a silhouette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub reporting: u32,
    pub over_limit: u32,
}

/// The GET's answer: our visibility when both counts are there (the Legacy
/// Viewer ignores an answer missing either), and the complexities the
/// region has for the avatars it lists (`setReportedVisualComplexity`).
pub fn parse_answer(body: &Llsd) -> (Option<Visibility>, Vec<(Uuid, u32)>) {
    let agents = body[KEY_AGENTS]
        .as_map()
        .map(|agents| {
            agents
                .iter()
                .filter_map(|(id, info)| {
                    let id = Uuid::parse_str(id).ok()?;
                    info.has(KEY_WEIGHT).then(|| (id, info[KEY_WEIGHT].as_i32() as u32))
                })
                .collect()
        })
        .unwrap_or_default();
    let visibility = (body.has(KEY_REPORTING_COMPLEXITY_LIMIT) && body.has(KEY_OVER_COMPLEXITY_LIMIT)).then(|| Visibility {
        reporting: body[KEY_REPORTING_COMPLEXITY_LIMIT].as_i32() as u32,
        over_limit: body[KEY_OVER_COMPLEXITY_LIMIT].as_i32() as u32,
    });
    (visibility, agents)
}

/// The POST's answer: the region's error, when it gives one (identifier and
/// message, for the log).
pub fn post_error(body: &Llsd) -> Option<String> {
    body.has(KEY_ERROR).then(|| {
        let e = &body[KEY_ERROR];
        format!("{}: {}", e["identifier"].to_string_value(), e["message"].to_string_value())
    })
}

/// When the next GET of a region may leave: moved on by the request when it
/// leaves and again when its answer comes (`getRenderInfoRequestTimer`).
pub type RequestTimer = Arc<Mutex<Instant>>;

/// LLViewerRegion's two timers.
#[derive(Debug, Clone)]
struct RegionTimers {
    report_at: Instant,
    request_at: RequestTimer,
}

impl RegionTimers {
    fn expired(now: Instant) -> Self {
        RegionTimers {
            report_at: now,
            request_at: Arc::new(Mutex::new(now)),
        }
    }
}

/// LLAvatarRenderInfoAccountant's schedule: the scan timer and each
/// region's report and request timers.
#[derive(Debug)]
pub struct Schedule {
    scan_at: Instant,
    regions: HashMap<RegionHandle, RegionTimers>,
}

impl Schedule {
    /// A timer starts expired (LLFrameTimer).
    pub fn new(now: Instant) -> Self {
        Schedule {
            scan_at: now,
            regions: HashMap::new(),
        }
    }

    /// scanNewRegion, once a region's capabilities came: the regions are
    /// scanned at once, this one asked at once and told on the next scan.
    pub fn new_region(&mut self, region: RegionHandle, now: Instant) {
        self.scan_at = now;
        self.regions.insert(
            region,
            RegionTimers {
                report_at: now + SCAN_PERIOD,
                request_at: Arc::new(Mutex::new(now)),
            },
        );
    }

    /// LLAvatarRenderInfoAccountant::idle: whether the regions are scanned
    /// now; the next scan is then [`SCAN_PERIOD`] away.
    pub fn scan(&mut self, now: Instant) -> bool {
        if now < self.scan_at {
            return false;
        }
        self.scan_at = now + SCAN_PERIOD;
        true
    }

    /// sendRenderInfoToRegion: a report to `region` is due; its timer then
    /// moves [`REPORT_PERIOD`] on until [`Schedule::reported`] says more.
    pub fn report_due(&mut self, region: RegionHandle, now: Instant) -> bool {
        let timers = self.regions.entry(region).or_insert_with(|| RegionTimers::expired(now));
        if now < timers.report_at {
            return false;
        }
        timers.report_at = now + REPORT_PERIOD;
        true
    }

    /// The report was built with `avatars` in it (sent or not: with none,
    /// nothing is posted and the timer moves on all the same).
    pub fn reported(&mut self, region: RegionHandle, now: Instant, avatars: usize) {
        if let Some(timers) = self.regions.get_mut(&region) {
            timers.report_at = now + report_delay(avatars);
        }
    }

    /// getRenderInfoFromRegion: a GET of `region` is due. Its timer moves
    /// [`REQUEST_PERIOD`] on, and the request moves it again when its answer
    /// comes ([`answered`]).
    pub fn request_due(&mut self, region: RegionHandle, now: Instant) -> Option<RequestTimer> {
        let timers = self.regions.entry(region).or_insert_with(|| RegionTimers::expired(now));
        let mut at = timers.request_at.lock();
        if now < *at {
            return None;
        }
        *at = now + REQUEST_PERIOD;
        drop(at);
        Some(timers.request_at.clone())
    }

    /// The timers of the regions no longer connected are dropped.
    pub fn retain(&mut self, mut alive: impl FnMut(RegionHandle) -> bool) {
        self.regions.retain(|handle, _| alive(*handle));
    }
}

/// avatarRenderInfoGetCoro: whatever its outcome, the next GET is
/// [`REQUEST_PERIOD`] after the answer (`resetWithExpiry` on return).
pub fn answered(timer: &RequestTimer, now: Instant) {
    *timer.lock() = now + REQUEST_PERIOD;
}

#[cfg(test)]
mod tests;
