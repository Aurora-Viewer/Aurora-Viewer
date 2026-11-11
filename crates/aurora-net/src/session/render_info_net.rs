//! The requests of LLAvatarRenderInfoAccountant on each region's
//! `AvatarRenderInfo` capability (crate::render_info): every scan, a report
//! of the avatars we see in the region when it is due (POST), and a request
//! for what the region knows of ours (GET). Both run off the session loop;
//! the capability's URL never reaches the log.

use super::Session;
use crate::caps::{self, CapsError};
use crate::render_info::{self, RequestTimer};
use crate::{EventSender, NetEvent, RegionHandle};
use aurora_llsd::Llsd;
use std::collections::HashSet;
use std::time::Instant;

const CAP: &str = "AvatarRenderInfo";

impl Session<'_> {
    /// LLAvatarRenderInfoAccountant::idle: every 5 s, each connected region
    /// with its capabilities (`capabilitiesReceived`) is told and asked,
    /// each on its own timer (sendRenderInfoToRegion, then
    /// getRenderInfoFromRegion).
    pub(super) fn scan_render_info(&mut self, now: Instant) {
        if !self.render_info.scan(now) {
            return;
        }
        let connected: HashSet<RegionHandle> = self.sims.values().map(|sim| sim.handle).collect();
        self.render_info.retain(|handle| connected.contains(&handle));
        let regions: Vec<(RegionHandle, String)> = self
            .sims
            .values()
            .filter_map(|sim| Some((sim.handle, sim.caps.get(CAP)?.clone())))
            .collect();
        for (handle, url) in regions {
            if self.render_info.report_due(handle, now) {
                // the report is built when it leaves (avatarRenderInfoReportCoro)
                let (body, count) = render_info::report(&self.sh.seen_avatars.lock(), handle);
                self.render_info.reported(handle, now, count);
                if count > 0 {
                    log::debug!("avatar render info: reporting {count} avatars to region {handle}");
                    tokio::spawn(post_report(self.sh.caps_http.clone(), url.clone(), body, handle));
                }
            }
            if let Some(timer) = self.render_info.request_due(handle, now) {
                tokio::spawn(get_render_info(
                    self.sh.caps_http.clone(),
                    url,
                    handle,
                    timer,
                    self.sh.events.clone(),
                ));
            }
        }
    }
}

/// The kind of a failure, without the URL reqwest's errors quote.
fn failure(e: &CapsError) -> String {
    match e {
        CapsError::Status(code) => format!("status {code}"),
        CapsError::Http(_) => "request failed".into(),
        CapsError::Llsd(_) => "unreadable answer".into(),
    }
}

/// avatarRenderInfoReportCoro: the POST, tried once more on a failure
/// (`setRetries(1)`), within llcorehttp's 30 s each time. Its answer only
/// goes to the log.
async fn post_report(http: reqwest::Client, url: String, body: Llsd, handle: RegionHandle) {
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(render_info::REPORT_RETRY_DELAY).await;
        }
        match tokio::time::timeout(render_info::REPORT_TIMEOUT, caps::post_llsd(&http, &url, &body)).await {
            Ok(Ok(answer)) => {
                match render_info::post_error(&answer) {
                    Some(error) => log::warn!("avatar render info: region {handle} refused the report: {error}"),
                    None => log::debug!("avatar render info: report accepted by region {handle}"),
                }
                return;
            }
            // a refusal (4xx) is not tried again, as llcorehttp does
            Ok(Err(e @ CapsError::Status(code))) if code < 500 => {
                log::debug!("avatar render info: report to region {handle} not posted: {}", failure(&e));
                return;
            }
            Ok(Err(e)) => last = failure(&e),
            Err(_) => last = "timed out".into(),
        }
    }
    log::debug!("avatar render info: report to region {handle} not posted: {last}");
}

/// avatarRenderInfoGetCoro: one GET (no retry, 15 s at most); whatever
/// comes back, the next one is 15 s after it. An answer with both counts
/// goes to the viewer for the notice.
async fn get_render_info(http: reqwest::Client, url: String, handle: RegionHandle, timer: RequestTimer, events: EventSender) {
    let answer = tokio::time::timeout(render_info::REQUEST_PERIOD, caps::get_llsd(&http, &url)).await;
    render_info::answered(&timer, Instant::now());
    let body = match answer {
        Ok(Ok(body)) => body,
        Ok(Err(e)) => {
            log::debug!("avatar render info: region {handle} not read: {}", failure(&e));
            return;
        }
        Err(_) => {
            log::debug!("avatar render info: region {handle} did not answer");
            return;
        }
    };
    let (visibility, agents) = render_info::parse_answer(&body);
    log::debug!(
        "avatar render info: region {handle} lists {} avatars, visibility {visibility:?}",
        agents.len()
    );
    match visibility {
        Some(visibility) => {
            let _ = events.send(NetEvent::AvatarRenderInfo { handle, visibility });
        }
        None => log::debug!("avatar render info: region {handle} answer without reportinglimit / overlimit"),
    }
}
