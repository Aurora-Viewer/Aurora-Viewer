//! Capabilities: seed resolution and the `EventQueueGet` long-poll.

use crate::probe::{self, HttpKind};
use aurora_llsd::{Llsd, from_xml, llsd_map, to_xml};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub const REQUESTED_CAPS: &[&str] = &[
    // the categories of an abuse report, in the viewer's language (LLFloaterReporter)
    "AbuseCategories",
    // offline friendship offers and group invitations are answered through
    // these (session_offline.rs)
    "AcceptFriendship",
    "AcceptGroupInvite",
    "AgentPreferences",
    "AgentProfile",
    // the scripts of my attachments (Script info)
    "AttachmentResources",
    "AvatarPickerSearch",
    "AvatarRenderInfo",
    "ChatSessionRequest",
    "CharacterProperties",
    "CopyInventoryFromNotecard",
    "DeclineFriendship",
    "DeclineGroupInvite",
    // the Region / Estate window (llviewerregion.cpp buildCapabilityNames):
    // the General panel's "Apply", the estate lists, the Estate panel's
    // "Apply", the region's experiences and its restart schedule
    "DispatchRegionInfo",
    "EnvironmentSettings",
    "EstateAccess",
    "EstateChangeInfo",
    "EventQueueGet",
    // allow, block and forget an experience (LLExperienceCache::setExperiencePermission)
    "ExperiencePreferences",
    "ExtEnvironment",
    "FetchInventory2",
    "FetchInventoryDescendents2",
    "FetchLib2",
    "FetchLibDescendents2",
    // the experience picker of About Land (LLExperienceCache::findExperienceByName)
    "FindExperienceByName",
    "GetCreatorExperiences",
    "GetDisplayNames",
    "GetExperienceInfo",
    "GetMesh",
    "GetMesh2",
    "GetMetadata",
    // build floater: land impact, object weights, physics shape costs
    "GetObjectCost",
    "GetObjectPhysicsData",
    "GetTexture",
    // the group profile (llviewerregion.cpp buildCapabilityNames): the ban
    // list (LLGroupMgr::sendGroupBanRequest), the experiences a group owns
    // (LLExperienceCache::getGroupExperiences), the member list
    // (LLGroupMgr::sendCapGroupMembersRequest)
    "GroupAPIv1",
    "GroupExperiences",
    "GroupMemberData",
    // World › Set Home to Here (LLAgent::setStartPosition)
    "HomeLocation",
    // rebake (LLAppearanceMgr::syncCofVersionAndRefresh, Firestorm's
    // buildCapabilityNames)
    "IncrementCOFVersion",
    "InventoryAPIv3",
    "InventoryThumbnailUpload",
    // the scripts of a parcel (LLFloaterScriptLimits)
    "LandResources",
    // the library's background fetch (AISAPI::LIBRARY_CAP_NAME)
    "LibraryAPIv3",
    // the server's LSL keywords, fetched when SimulatorFeatures' LSLSyntaxId
    // changes (LLSyntaxIdLSL)
    "LSLSyntax",
    "NewFileAgentInventory",
    "MeshUploadFlag",
    "DirectDelivery",
    "ModifyMaterialParams",
    // LLViewerRegionImpl::buildCapabilityNames (indra/newview/llviewerregion.cpp,
    // originally LGPL 2.1): advertise support so the simulator sends the
    // ObjectAnimation UDP messages. This capability needs no HTTP request.
    "ObjectAnimation",
    "ObjectMedia",
    "ObjectMediaNavigate",
    "ParcelVoiceInfoRequest",
    "ProvisionVoiceAccountRequest",
    // instant messages kept while offline (LLIMProcessing::requestOfflineMessages)
    "ReadOfflineMsgs",
    "RegionExperiences",
    "RegionObjects",
    "RegionSchedule",
    // LLViewerRegionImpl::buildCapabilityNames: parcel UUID lookup before
    // About Land's script resources, and remote place profiles.
    "RemoteParcelRequest",
    "RenderMaterials",
    "RequestTaskInventory",
    "ResourceCostSelected",
    // the clicks of a classified (LLPanelProfileClassified)
    "SearchStatRequest",
    "SearchStatTracking",
    // a photo sent by e-mail (LLPanelSnapshotPostcard::sendPostcard)
    "SendPostcard",
    // abuse reports, with or without their screenshot (sendReportViaCaps)
    "SendUserReport",
    "SendUserReportWithScreenshot",
    // Help › Server release notes (LLFloaterAbout::startFetchServerReleaseNotes)
    "ServerReleaseNotes",
    "SetDisplayName",
    // the region console of estate managers (LLFloaterRegionDebugConsole)
    "SimConsoleAsync",
    "SimulatorFeatures",
    // nearby voice moderation by estate managers and parcel owners
    // (LLNearbyVoiceModeration)
    "SpatialVoiceModerationRequest",
    "UpdateAvatarAppearance",
    // the maturity preference (LLAgent::sendMaturityPreferenceToServer)
    "UpdateAgentInformation",
    "UpdateAgentLanguage",
    "UpdateMaterialAgentInventory",
    "UpdateMaterialTaskInventory",
    "UpdateNotecardTaskInventory",
    "UpdateNotecardAgentInventory",
    "UpdateGestureAgentInventory",
    "UpdateScriptAgent",
    "UpdateScriptTask",
    // environment settings items (LLEnvironment::isInventoryEnabled needs both)
    "UpdateSettingsAgentInventory",
    "UpdateSettingsTaskInventory",
    // Preferences › Privacy: online visibility, offline IMs by e-mail
    // (LLAgent::sendAgentUserInfoRequest / sendAgentUpdateUserInfo)
    "UserInfo",
    "ViewerAsset",
    "ViewerStats",
    "VoiceSignalingRequest",
];

/// LLAgentLanguage::update: exact LLSD fields and boolean visibility.
pub fn agent_language_body(language: &str, public: bool) -> Llsd {
    llsd_map! { "language" => language, "language_is_public" => public }
}

/// LLAgent::sendMaturityPreferenceToServer: `{"access_prefs": {"max": "M"}}`
/// (LLViewerRegion::accessToShortString).
pub fn maturity_preference_body(maturity: u8) -> Llsd {
    llsd_map! { "access_prefs" => llsd_map! { "max" => crate::search::access_short_string(maturity) } }
}

/// LLAgent::processMaturityPreferenceFromServer: the preference the server
/// kept, `access_prefs.max` ("PG", "M", "A"); SIM_ACCESS_MIN when missing.
pub fn maturity_preference_reply(reply: &Llsd) -> u8 {
    crate::search::short_string_to_access(reply["access_prefs"]["max"].as_str().trim())
}

/// LLVOAvatarSelf::sendHoverHeight, including MIN/MAX_HOVER_Z (-3..3 m).
pub fn hover_height_body(value: f32) -> Option<Llsd> {
    value.is_finite().then(|| llsd_map! { "hover_height" => value.clamp(-3.0, 3.0) })
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn maturity_preference_is_sent_and_read_as_short_strings() {
        assert!(REQUESTED_CAPS.contains(&"UpdateAgentInformation"));
        let body = maturity_preference_body(crate::search::SIM_ACCESS_MATURE);
        let decoded = aurora_llsd::from_xml(&aurora_llsd::to_xml(&body)).unwrap();
        assert_eq!(decoded["access_prefs"]["max"].as_str(), "M");
        assert_eq!(maturity_preference_reply(&decoded), crate::search::SIM_ACCESS_MATURE);
        let adult = llsd_map! { "access_prefs" => llsd_map! { "max" => " A " } };
        assert_eq!(maturity_preference_reply(&adult), crate::search::SIM_ACCESS_ADULT);
        assert_eq!(maturity_preference_reply(&Llsd::Undef), 0);
    }

    #[test]
    fn hover_height_payload_is_one_finite_bounded_real() {
        assert!(REQUESTED_CAPS.contains(&"AgentPreferences"));
        for value in [-4.0_f32, -0.125, 0.0, 0.75, 4.0] {
            let body = hover_height_body(value).unwrap();
            let decoded = aurora_llsd::from_xml(&aurora_llsd::to_xml(&body)).unwrap();
            assert_eq!(decoded["hover_height"].as_f32(), value.clamp(-3.0, 3.0));
            assert_eq!(decoded.as_map().unwrap().len(), 1);
        }
        assert!(hover_height_body(f32::NAN).is_none());
        assert!(hover_height_body(f32::INFINITY).is_none());
    }

    /// The Region / Estate window's capabilities: without them the lists,
    /// the schedule and the experiences are never read.
    #[test]
    fn the_region_estate_capabilities_are_requested() {
        for cap in [
            "DispatchRegionInfo",
            "EstateAccess",
            "EstateChangeInfo",
            "RegionExperiences",
            "RegionSchedule",
        ] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }

    #[test]
    fn the_home_and_user_info_capabilities_are_requested() {
        for cap in ["HomeLocation", "UserInfo"] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }

    /// The minor capabilities of llviewerregion.cpp buildCapabilityNames
    /// (#660): without them the console, the postcard, the voice moderation,
    /// the server keywords and the release notes have no URL.
    #[test]
    fn the_minor_simulator_capabilities_are_requested() {
        for cap in [
            "LSLSyntax",
            "SendPostcard",
            "ServerReleaseNotes",
            "SimConsoleAsync",
            "SpatialVoiceModerationRequest",
        ] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }

    #[test]
    fn the_group_profile_capabilities_are_requested() {
        for cap in ["GroupAPIv1", "GroupExperiences", "GroupMemberData"] {
            assert!(REQUESTED_CAPS.contains(&cap), "{cap}");
        }
    }

    #[test]
    fn language_capability_is_requested_and_payload_keeps_protocol_fields() {
        assert!(REQUESTED_CAPS.contains(&"UpdateAgentLanguage"));
        for language in ["en", "fr", "pt-BR"] {
            for public in [false, true] {
                let body = agent_language_body(language, public);
                let bytes = aurora_llsd::to_xml(&body);
                let decoded = aurora_llsd::from_xml(&bytes).expect("synthetic language payload");
                assert_eq!(decoded["language"].as_str(), language);
                assert_eq!(decoded["language_is_public"].as_bool(), public);
                assert_eq!(decoded.as_map().expect("language map").len(), 2);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("http: {0}")]
    Http(String),
    #[error("status {0}")]
    Status(u16),
    #[error("llsd: {0}")]
    Llsd(#[from] aurora_llsd::LlsdError),
}

pub async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd) -> Result<Llsd, CapsError> {
    request_llsd(http, url, body, false).await
}

/// [`post_llsd`] counted under another kind than `cap` by the HTTP probe
/// (AURORA_PROFILE: inventory fetches).
pub async fn post_llsd_as(http: &reqwest::Client, url: &str, body: &Llsd, kind: HttpKind) -> Result<Llsd, CapsError> {
    let req = http.post(url).header("Content-Type", "application/llsd+xml").body(to_xml(body));
    send_llsd(req, kind).await
}

/// GET of an LLSD XML document (getAndSuspend).
pub async fn get_llsd(http: &reqwest::Client, url: &str) -> Result<Llsd, CapsError> {
    send_llsd(http.get(url), HttpKind::Cap).await
}

pub async fn get_llsd_as(http: &reqwest::Client, url: &str, kind: HttpKind) -> Result<Llsd, CapsError> {
    send_llsd(http.get(url), kind).await
}

/// POST, or PUT (RenderMaterials writes, LLMaterialMgr::processPutQueue), of LLSD XML.
pub async fn request_llsd(http: &reqwest::Client, url: &str, body: &Llsd, put: bool) -> Result<Llsd, CapsError> {
    let req = if put { http.put(url) } else { http.post(url) };
    send_llsd(req.header("Content-Type", "application/llsd+xml").body(to_xml(body)), HttpKind::Cap).await
}

/// Send a capability request that answers in LLSD XML, timed by the HTTP
/// probe under `kind` (AURORA_PROFILE).
async fn send_llsd(req: reqwest::RequestBuilder, kind: HttpKind) -> Result<Llsd, CapsError> {
    let timed = probe::HttpRequest::start(kind);
    let finish = |timed: Option<probe::HttpRequest>, status: u16, bytes: usize, ok: bool| {
        if let Some(t) = timed {
            t.finish(status, bytes, ok);
        }
    };
    let resp = match req.header("Accept", "application/llsd+xml").send().await {
        Ok(resp) => resp,
        Err(e) => {
            finish(timed, 0, 0, false);
            return Err(CapsError::Http(e.to_string()));
        }
    };
    let status = resp.status();
    if !status.is_success() {
        finish(timed, status.as_u16(), 0, false);
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            finish(timed, 0, 0, false);
            return Err(CapsError::Http(e.to_string()));
        }
    };
    finish(timed, status.as_u16(), bytes.len(), true);
    if bytes.is_empty() {
        return Ok(Llsd::Undef);
    }
    Ok(from_xml(&bytes)?)
}

/// How long the seed capability is asked for before the region is given up.
///
/// LLViewerRegionImpl::requestBaseCapabilitiesCoro (indra/newview/
/// llviewerregion.cpp, originally LGPL 2.1) asks again until more than
/// MAX_CAP_REQUEST_ATTEMPTS (30) requests have failed, each one given the
/// 30 s of llcorehttp (HTTP_REQUEST_TIMEOUT_DEFAULT) and its retries 1 to
/// 5 s apart (HTTP_RETRY_BACKOFF_MIN / MAX_DEFAULT). A simulator that lags
/// at the wrong moment, at login or on arrival, is not a region without
/// textures, inventory nor teleports for the rest of the session.
#[derive(Debug, Clone, Copy)]
pub struct SeedRetry {
    pub attempts: u32,
    pub timeout: Duration,
    pub min_wait: Duration,
    pub max_wait: Duration,
}

impl SeedRetry {
    /// LLViewerRegionImpl::requestSimulatorFeatureCoro stops after 30 attempts.
    pub const SIMULATOR_FEATURES: SeedRetry = SeedRetry {
        attempts: 30,
        ..Self::FIRESTORM
    };

    pub const FIRESTORM: SeedRetry = SeedRetry {
        attempts: 31,
        timeout: Duration::from_secs(30),
        min_wait: Duration::from_secs(1),
        max_wait: Duration::from_secs(5),
    };

    /// The wait after failed attempt number `failed` (from 1): doubled each
    /// time, up to the longest.
    pub fn wait(&self, failed: u32) -> Duration {
        let doubled = self.min_wait.saturating_mul(1u32 << failed.saturating_sub(1).min(16));
        doubled.min(self.max_wait)
    }
}

/// The wait with up to a fifth more, so that the viewers a restarting
/// simulator threw out do not all come back at the same instant.
fn jittered(wait: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    // (a fresh hasher is seeded at random: no generator to carry around)
    let roll = std::collections::hash_map::RandomState::new().build_hasher().finish() % 1000;
    wait + wait.mul_f32(roll as f32 / 5000.0)
}

/// POST the list of wanted capabilities to the seed and return the map.
/// `still_wanted` is asked before each new attempt: the session ended, or
/// the region has another seed by now.
pub async fn resolve_seed(
    http: &reqwest::Client,
    seed: &str,
    retry: SeedRetry,
    still_wanted: impl Fn() -> bool,
) -> Result<HashMap<String, String>, CapsError> {
    let req = Llsd::Array(REQUESTED_CAPS.iter().map(|s| Llsd::from(*s)).collect());
    let mut last_err = CapsError::Status(0);
    for attempt in 1..=retry.attempts.max(1) {
        let reply = match tokio::time::timeout(retry.timeout, post_llsd(http, seed, &req)).await {
            Ok(reply) => reply,
            Err(_) => Err(CapsError::Http(format!("no reply in {} s", retry.timeout.as_secs()))),
        };
        match reply {
            Ok(v) => match v.as_map() {
                Some(m) => {
                    let mut caps = HashMap::new();
                    for (k, val) in m {
                        let s = val.to_string_value();
                        if !s.is_empty() {
                            caps.insert(k.clone(), s);
                        }
                    }
                    if attempt > 1 {
                        log::info!("seed capability answered at attempt {attempt}");
                    }
                    return Ok(caps);
                }
                // (Firestorm's "malformed response": asked again)
                None => last_err = CapsError::Http("malformed response".into()),
            },
            Err(e) => last_err = e,
        }
        log::warn!(
            "seed capability request failed (attempt {attempt} of {}): {last_err}",
            retry.attempts
        );
        if attempt == retry.attempts || !still_wanted() {
            break;
        }
        tokio::time::sleep(jittered(retry.wait(attempt))).await;
        if !still_wanted() {
            break;
        }
    }
    Err(last_err)
}

/// LLViewerRegionImpl::requestSimulatorFeatureCoro (llviewerregion.cpp):
/// retry failed GETs rather than disabling region-dependent tools for the
/// rest of a session. The owning simulator aborts this future on logout or
/// seed replacement; a valid map with hover explicitly disabled is final.
pub async fn simulator_features(http: &reqwest::Client, url: &str, retry: SeedRetry) -> Result<Llsd, CapsError> {
    let mut last = CapsError::Http("no SimulatorFeatures request attempted".into());
    for attempt in 1..=retry.attempts {
        let reply = tokio::time::timeout(retry.timeout, get_llsd(http, url)).await;
        last = match reply {
            Ok(Ok(features)) if features.as_map().is_some() => return Ok(features),
            Ok(Ok(_)) => CapsError::Http("malformed SimulatorFeatures response".into()),
            Ok(Err(error)) => error,
            Err(_) => CapsError::Http("SimulatorFeatures request timed out".into()),
        };
        // No capability URL or server response body in diagnostics.
        let reason = match &last {
            CapsError::Status(status) => format!("HTTP {status}"),
            CapsError::Http(_) => "transport failure or malformed response".into(),
            CapsError::Llsd(_) => "malformed LLSD".into(),
        };
        log::warn!(
            "SimulatorFeatures request failed (attempt {attempt} of {}): {reason}",
            retry.attempts
        );
        if attempt < retry.attempts {
            tokio::time::sleep(jittered(retry.wait(attempt))).await;
        }
    }
    Err(last)
}

#[derive(Debug, Clone)]
pub struct EqEvent {
    pub sim: std::net::SocketAddr,
    pub message: String,
    pub body: Llsd,
}

/// What an event queue hands to the session.
#[derive(Debug, Clone)]
pub enum EqMessage {
    Event(EqEvent),
    /// The poll of this simulator stopped after too many errors in a row:
    /// nothing it sends through the queue will arrive any more.
    Lost(std::net::SocketAddr),
}

// LLEventPollImpl::eventPollCoro (indra/newview/lleventpoll.cpp, originally
// LGPL 2.1): wait 1 s + 3 s per error already counted, give up after 15
// (about 5 minutes); the simulator holds a request 20 to 30 s, so one that
// lasted 10 s was held and brought no event.
const ERROR_RETRY: Duration = Duration::from_secs(1);
const ERROR_RETRY_INC: Duration = Duration::from_secs(3);
const MAX_ERRORS: u32 = 15;
const MIN_HELD: Duration = Duration::from_secs(10);

/// How a poll ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PollEnd {
    /// A 200, with or without events.
    Reply,
    Status(u16),
    /// Our own timeout.
    Timeout,
    /// The connection failed or was cut before a complete reply.
    Transport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// Poll again at once.
    Again,
    /// An error: poll again after this wait.
    Wait(Duration),
    /// The simulator closed the queue.
    Closed,
    GiveUp,
}

/// What to do after a poll that lasted `held`, with `errors` counted in a
/// row before it (LLEventPollImpl::eventPollCoro). A poll the simulator held
/// and that then ended without events is the normal "no event": a timeout,
/// one of its gateway statuses, or the held connection being cut, which is
/// how its end shows through our HTTP client about once a minute in a quiet
/// place. Counting those as errors, as before, stopped the queue for good
/// after twenty minutes without an event.
fn after_poll(end: PollEnd, held: Duration, errors: &mut u32) -> Next {
    let no_event = match end {
        PollEnd::Reply => true,
        PollEnd::Status(404 | 410) => return Next::Closed,
        PollEnd::Status(499 | 500 | 502 | 503 | 504) | PollEnd::Timeout | PollEnd::Transport => held >= MIN_HELD,
        PollEnd::Status(_) => false,
    };
    if no_event {
        *errors = 0;
        return Next::Again;
    }
    if *errors >= MAX_ERRORS {
        return Next::GiveUp;
    }
    let wait = ERROR_RETRY + ERROR_RETRY_INC * *errors;
    *errors += 1;
    Next::Wait(wait)
}

/// Long-poll `EventQueueGet` until the receiver is dropped, the simulator
/// closes the queue (404) or too many polls in a row fail.
pub async fn event_queue_loop(
    http: reqwest::Client,
    url: String,
    sim: std::net::SocketAddr,
    tx: mpsc::UnboundedSender<EqMessage>,
    stats: std::sync::Arc<crate::NetStats>,
) {
    use std::sync::atomic::Ordering;
    let mut ack: Llsd = Llsd::Undef;
    let mut errors = 0u32;
    loop {
        if tx.is_closed() {
            return;
        }
        let body = llsd_map! { "ack" => ack.clone(), "done" => false };
        let started = std::time::Instant::now();
        let fut = http
            .post(&url)
            .header("Content-Type", "application/llsd+xml")
            .header("Accept", "application/llsd+xml")
            .timeout(Duration::from_secs(75))
            .body(to_xml(&body))
            .send();
        // (errors are described without their URL: it is a capability)
        let (end, why) = match fut.await {
            Ok(resp) => match resp.status().as_u16() {
                200 => match resp.bytes().await {
                    Ok(bytes) => {
                        match from_xml(&bytes) {
                            // like Firestorm, a reply without an id keeps the previous ack
                            Ok(v) if !v["id"].is_undef() => {
                                ack = v["id"].clone();
                                for ev in v["events"].as_array() {
                                    let e = EqEvent {
                                        sim,
                                        message: ev["message"].as_str().to_owned(),
                                        body: ev["body"].clone(),
                                    };
                                    if tx.send(EqMessage::Event(e)).is_err() {
                                        return;
                                    }
                                }
                            }
                            Ok(_) => log::warn!("event queue for {sim}: reply without an id"),
                            Err(_) => log::warn!("event queue for {sim}: malformed LLSD"),
                        }
                        (PollEnd::Reply, String::new())
                    }
                    Err(e) => (PollEnd::Transport, e.without_url().to_string()),
                },
                status => (PollEnd::Status(status), format!("status {status}")),
            },
            Err(e) if e.is_timeout() => (PollEnd::Timeout, String::new()),
            Err(e) => (PollEnd::Transport, e.without_url().to_string()),
        };
        let held = started.elapsed();
        match after_poll(end, held, &mut errors) {
            Next::Again => {
                if end != PollEnd::Reply {
                    stats.eq_idle_polls.fetch_add(1, Ordering::Relaxed);
                }
            }
            Next::Closed => {
                log::info!("event queue for {sim} closed ({why})");
                return;
            }
            Next::Wait(wait) => {
                stats.eq_errors.fetch_add(1, Ordering::Relaxed);
                log::warn!(
                    "event queue for {sim}: {why} after {:.1} s, error {errors} of {MAX_ERRORS} in a row, next poll in {} s",
                    held.as_secs_f32(),
                    wait.as_secs()
                );
                tokio::time::sleep(wait).await;
            }
            Next::GiveUp => {
                stats.eq_errors.fetch_add(1, Ordering::Relaxed);
                log::warn!("event queue for {sim}: {why}, giving up after {MAX_ERRORS} errors in a row");
                let _ = tx.send(EqMessage::Lost(sim));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const HELD: Duration = Duration::from_secs(30);
    const FAST: Duration = Duration::from_millis(200);

    #[test]
    fn a_quiet_place_never_stops_the_event_queue() {
        // every way a held poll ends without an event, for hours
        let mut errors = 0;
        for _ in 0..500 {
            for end in [PollEnd::Transport, PollEnd::Status(502), PollEnd::Timeout, PollEnd::Status(499)] {
                assert_eq!(after_poll(end, HELD, &mut errors), Next::Again);
            }
        }
        assert_eq!(errors, 0);
        // the threshold is the 10 s the simulator holds a request at least
        assert_eq!(after_poll(PollEnd::Transport, MIN_HELD, &mut errors), Next::Again);
        assert_eq!(
            after_poll(PollEnd::Transport, MIN_HELD - FAST, &mut errors),
            Next::Wait(ERROR_RETRY)
        );
    }

    #[test]
    fn fast_failures_back_off_like_firestorm_then_give_up() {
        let mut errors = 0;
        let mut total = Duration::ZERO;
        for n in 0..MAX_ERRORS {
            // a refused connection, a gateway error at once, a forbidden cap
            let end = [PollEnd::Transport, PollEnd::Status(503), PollEnd::Status(403)][n as usize % 3];
            let Next::Wait(wait) = after_poll(end, FAST, &mut errors) else {
                panic!("error {n} should be retried");
            };
            assert_eq!(wait, Duration::from_secs(1 + 3 * u64::from(n)));
            total += wait;
        }
        // "~5 minutes, by the above rules" (lleventpoll.cpp)
        assert_eq!(total, Duration::from_secs(330));
        assert_eq!(after_poll(PollEnd::Transport, FAST, &mut errors), Next::GiveUp);
        // a status that is not a gateway one is an error however long it took
        let mut errors = 0;
        assert_eq!(after_poll(PollEnd::Status(403), HELD, &mut errors), Next::Wait(ERROR_RETRY));
    }

    #[test]
    fn a_reply_or_a_held_poll_forgets_the_errors() {
        for recovery in [(PollEnd::Reply, FAST), (PollEnd::Status(502), HELD), (PollEnd::Transport, HELD)] {
            let mut errors = 0;
            for _ in 0..MAX_ERRORS {
                after_poll(PollEnd::Transport, FAST, &mut errors);
            }
            assert_eq!(after_poll(recovery.0, recovery.1, &mut errors), Next::Again, "{recovery:?}");
            // back to the first wait, not to the give-up
            assert_eq!(after_poll(PollEnd::Transport, FAST, &mut errors), Next::Wait(ERROR_RETRY));
        }
    }

    #[test]
    fn a_missing_queue_is_closed_at_once() {
        let mut errors = 3;
        assert_eq!(after_poll(PollEnd::Status(404), FAST, &mut errors), Next::Closed);
        assert_eq!(after_poll(PollEnd::Status(410), HELD, &mut errors), Next::Closed);
        assert_eq!(errors, 3);
    }

    /// Serve canned HTTP replies, one connection each; `None` cuts the
    /// connection without a reply.
    async fn poll_server(replies: Vec<Option<Vec<u8>>>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local simulator");
        let url = format!("http://{}/cap/queue", listener.local_addr().expect("local address"));
        let handle = tokio::spawn(async move {
            let mut bodies = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.expect("poll");
                let mut request = Vec::new();
                let body = loop {
                    let mut buf = [0; 2048];
                    let n = socket.read(&mut buf).await.expect("read poll");
                    assert_ne!(n, 0, "incomplete poll");
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((_, body)) = text.split_once("\r\n\r\n")
                        && body.contains("</llsd>")
                    {
                        break body.to_string();
                    }
                };
                bodies.push(body);
                if let Some(reply) = reply {
                    socket.write_all(&reply).await.expect("write reply");
                }
                let _ = socket.shutdown().await;
            }
            bodies
        });
        (url, handle)
    }

    fn reply(status: &str, body: &[u8]) -> Option<Vec<u8>> {
        let mut r = format!("HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        Some(r)
    }

    #[tokio::test]
    async fn events_are_delivered_and_acknowledged_across_an_error() {
        let events = to_xml(&llsd_map! {
            "id" => 7,
            "events" => Llsd::Array(vec![llsd_map! { "message" => "TeleportFinish", "body" => llsd_map! { "n" => 1 } }]),
        });
        // events, then a cut connection (a counted error: 1 s wait), a reply
        // without an id, and the queue closes
        let (url, server) = poll_server(vec![
            reply("200 OK", &events),
            None,
            reply("200 OK", &to_xml(&llsd_map! { "events" => Llsd::Array(vec![]) })),
            reply("404 Not Found", b""),
        ])
        .await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sim: std::net::SocketAddr = "127.0.0.1:9".parse().expect("address");
        let http = reqwest::Client::builder().no_proxy().build().expect("HTTP client");
        let stats = std::sync::Arc::new(crate::NetStats::default());
        event_queue_loop(http, url, sim, tx, stats.clone()).await;
        let snap = stats.snapshot();
        assert_eq!((snap.eq_errors, snap.eq_idle_polls), (1, 0));
        let Some(EqMessage::Event(e)) = rx.recv().await else {
            panic!("event");
        };
        assert_eq!((e.sim, e.message.as_str(), e.body["n"].as_i32()), (sim, "TeleportFinish", 1));
        assert!(rx.recv().await.is_none(), "a closed queue is not a lost one");
        let bodies = server.await.expect("server");
        assert_eq!(bodies.len(), 4);
        let acks: Vec<i32> = bodies
            .iter()
            .map(|b| from_xml(b.as_bytes()).expect("poll LLSD")["ack"].as_i32())
            .collect();
        // the ack of the events is repeated until a reply brings a new id
        assert_eq!(acks, [0, 7, 7, 7]);
    }

    #[tokio::test]
    async fn seed_post_negotiates_parcel_resources_and_advertises_object_animation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local seed server");
        let seed = format!("http://{}/seed", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("seed client");
            let mut request = Vec::new();
            let body_start = loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.expect("read seed request");
                assert_ne!(n, 0, "incomplete HTTP headers");
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..body_start]).expect("HTTP headers");
            assert!(headers.starts_with("POST /seed HTTP/1.1\r\n"));
            let body_len: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("body length"))
                })
                .expect("content-length");
            while request.len() < body_start + body_len {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.expect("read seed body");
                assert_ne!(n, 0, "incomplete LLSD request");
                request.extend_from_slice(&buf[..n]);
            }
            let requested = from_xml(&request[body_start..body_start + body_len]).expect("seed LLSD");
            let supports_animesh = requested.as_array().iter().any(|v| v.as_str() == "ObjectAnimation");
            // Support markers may not return a URL. Asset caps must still resolve.
            let mut caps = llsd_map! { "ViewerAsset" => "http://127.0.0.1/asset" };
            // The simulator only supplies services the viewer requested. A
            // missing parcel lookup must fail here, before querying resources.
            for name in ["RemoteParcelRequest", "LandResources", "AttachmentResources"] {
                if requested.as_array().iter().any(|v| v.as_str() == name) {
                    caps.insert(name, format!("http://127.0.0.1/{name}"));
                }
            }
            let body = to_xml(&caps);
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.expect("write headers");
            socket.write_all(&body).await.expect("write LLSD");
            supports_animesh
        });
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("HTTP client");
        let resolved = resolve_seed(&http, &seed, SeedRetry::FIRESTORM, || true)
            .await
            .expect("seed response");
        assert!(server.await.expect("seed server"), "simulator was not told to send ObjectAnimation");
        assert_eq!(resolved.get("ViewerAsset").map(String::as_str), Some("http://127.0.0.1/asset"));
        assert!(!resolved.contains_key("ObjectAnimation"));
        for name in ["RemoteParcelRequest", "LandResources", "AttachmentResources"] {
            assert_eq!(
                resolved.get(name),
                Some(&format!("http://127.0.0.1/{name}")),
                "the seed must supply {name} before About Land can query script resources"
            );
        }
    }

    /// A simulator that answers each seed request in turn with one of
    /// `replies` (an HTTP status; 200 comes with a capability map, 0 with a
    /// body that is not a map), then refuses. Returns its seed and the
    /// count of requests it got.
    async fn seed_server(replies: Vec<u16>) -> (String, std::sync::Arc<std::sync::atomic::AtomicU32>) {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let seed = format!("http://{}/seed", listener.local_addr().expect("address"));
        let count = Arc::new(AtomicU32::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let n = seen.fetch_add(1, Ordering::SeqCst) as usize;
                // (the request is small: its headers and body come together
                // or in the next read)
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                while !request.windows(7).any(|w| w == b"</llsd>") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let (status, body) = match replies.get(n).copied() {
                    Some(200) => ("200 OK", to_xml(&llsd_map! { "ViewerAsset" => "http://127.0.0.1/asset" })),
                    Some(0) => ("200 OK", to_xml(&Llsd::from("not a map"))),
                    Some(503) | None => ("503 Service Unavailable", Vec::new()),
                    Some(other) => panic!("no reply written for {other}"),
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        (seed, count)
    }

    fn quick(attempts: u32) -> SeedRetry {
        SeedRetry {
            attempts,
            timeout: Duration::from_secs(5),
            min_wait: Duration::from_millis(2),
            max_wait: Duration::from_millis(8),
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().expect("HTTP client")
    }

    /// Each request must be a GET; unlike a seed POST, it has no LLSD body.
    async fn features_server(replies: Vec<(u16, Vec<u8>)>) -> (String, tokio::task::JoinHandle<usize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local feature server");
        let url = format!("http://{}/features", listener.local_addr().expect("local address"));
        let task = tokio::spawn(async move {
            let count = replies.len();
            for (status, body) in replies {
                let (mut socket, _) = listener.accept().await.expect("feature request");
                let mut request = Vec::new();
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut bytes = [0; 2048];
                    let size = socket.read(&mut bytes).await.expect("GET headers");
                    assert_ne!(size, 0);
                    request.extend_from_slice(&bytes[..size]);
                }
                let headers = String::from_utf8(request).expect("HTTP headers");
                assert!(headers.starts_with("GET /features HTTP/1.1\r\n"));
                assert!(headers.to_ascii_lowercase().contains("accept: application/llsd+xml"));
                assert!(headers.ends_with("\r\n\r\n"));
                let head = format!(
                    "HTTP/1.1 {status} Reply\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.expect("response headers");
                socket.write_all(&body).await.expect("feature response");
            }
            count
        });
        (url, task)
    }

    #[tokio::test]
    async fn simulator_features_retry_status_xml_and_non_map_failures_before_enabling_hover() {
        let (url, server) = features_server(vec![
            (503, Vec::new()),
            (200, b"not XML".to_vec()),
            (200, to_xml(&Llsd::from("not a map"))),
            (
                200,
                to_xml(&llsd_map! { "AvatarHoverHeightEnabled" => true, "VoiceServerType" => "webrtc" }),
            ),
        ])
        .await;
        let features = simulator_features(&client(), &url, quick(6)).await.expect("recovered features");
        assert!(features["AvatarHoverHeightEnabled"].as_bool());
        assert_eq!(features["VoiceServerType"].as_str(), "webrtc");
        assert_eq!(server.await.expect("feature server"), 4);
    }

    #[tokio::test]
    async fn simulator_features_do_not_retry_a_region_that_explicitly_disables_hover() {
        let (url, server) = features_server(vec![(200, to_xml(&llsd_map! { "AvatarHoverHeightEnabled" => false }))]).await;
        let features = simulator_features(&client(), &url, quick(6)).await.expect("unsupported region");
        assert!(!features["AvatarHoverHeightEnabled"].as_bool());
        assert_eq!(server.await.expect("feature server"), 1);
    }

    #[tokio::test]
    async fn simulator_features_stop_after_the_retry_limit() {
        let (url, server) = features_server(vec![(503, Vec::new()); 3]).await;
        let error = simulator_features(&client(), &url, quick(3)).await.expect_err("bounded retries");
        assert!(matches!(error, CapsError::Status(503)));
        assert_eq!(server.await.expect("feature server"), 3);
        assert_eq!(SeedRetry::SIMULATOR_FEATURES.attempts, 30);
    }

    #[tokio::test]
    async fn simulator_features_retry_a_request_that_never_answers() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("local feature server");
        let url = format!("http://{}/features", listener.local_addr().expect("local address"));
        let count = std::sync::Arc::new(AtomicU32::new(0));
        let seen = count.clone();
        let server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                seen.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });
        let retry = SeedRetry {
            timeout: Duration::from_millis(60),
            ..quick(3)
        };
        assert!(simulator_features(&client(), &url, retry).await.is_err());
        assert_eq!(count.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[test]
    fn seed_requests_are_spaced_like_firestorms_http_retries() {
        let f = SeedRetry::FIRESTORM;
        // more than MAX_CAP_REQUEST_ATTEMPTS (30) failures give the region up
        assert_eq!(f.attempts, 31);
        assert_eq!(f.timeout, Duration::from_secs(30));
        let waits: Vec<u64> = (1..=6).map(|failed| f.wait(failed).as_secs()).collect();
        assert_eq!(waits, [1, 2, 4, 5, 5, 5]);
        assert_eq!(f.wait(10_000), Duration::from_secs(5));
        // a wait is never shortened, and lengthened by a fifth at most
        for _ in 0..200 {
            let j = jittered(Duration::from_secs(5));
            assert!(j >= Duration::from_secs(5) && j <= Duration::from_secs(6), "{j:?}");
        }
    }

    #[tokio::test]
    async fn a_seed_that_fails_at_first_is_asked_again_until_it_answers() {
        use std::sync::atomic::Ordering;
        // two refusals and a reply that is not a map, then the capabilities
        let (seed, count) = seed_server(vec![503, 0, 503, 200]).await;
        let caps = resolve_seed(&client(), &seed, quick(31), || true).await.expect("capabilities");
        assert_eq!(caps.get("ViewerAsset").map(String::as_str), Some("http://127.0.0.1/asset"));
        assert_eq!(count.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn a_seed_that_never_answers_is_given_up_after_every_attempt() {
        use std::sync::atomic::Ordering;
        let (seed, count) = seed_server(Vec::new()).await;
        let err = resolve_seed(&client(), &seed, quick(6), || true).await.expect_err("given up");
        assert!(matches!(err, CapsError::Status(503)), "{err}");
        assert_eq!(count.load(Ordering::SeqCst), 6);
        // the session ended, or the region changed its seed: no more requests
        let (seed, count) = seed_server(Vec::new()).await;
        let wanted = std::sync::atomic::AtomicBool::new(true);
        let stop_after_two = || count.load(Ordering::SeqCst) < 2 && wanted.load(Ordering::SeqCst);
        resolve_seed(&client(), &seed, quick(31), stop_after_two)
            .await
            .expect_err("stopped");
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_seed_request_left_without_reply_times_out_and_is_asked_again() {
        use std::sync::atomic::Ordering;
        // a simulator that accepts the connection and says nothing
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let seed = format!("http://{}/seed", listener.local_addr().expect("address"));
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                seen.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });
        let retry = SeedRetry {
            timeout: Duration::from_millis(60),
            ..quick(3)
        };
        let err = resolve_seed(&client(), &seed, retry, || true).await.expect_err("no reply");
        assert!(matches!(err, CapsError::Http(_)), "{err}");
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }
}
