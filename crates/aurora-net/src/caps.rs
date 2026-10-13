//! Capabilities: seed resolution and the `EventQueueGet` long-poll.

use aurora_llsd::{Llsd, from_xml, llsd_map, to_xml};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;

pub const REQUESTED_CAPS: &[&str] = &[
    // offline friendship offers and group invitations are answered through
    // these (session_offline.rs)
    "AcceptFriendship",
    "AcceptGroupInvite",
    "AgentPreferences",
    "AgentProfile",
    "AvatarPickerSearch",
    "AvatarRenderInfo",
    "ChatSessionRequest",
    "CopyInventoryFromNotecard",
    "DeclineFriendship",
    "DeclineGroupInvite",
    "EnvironmentSettings",
    "EventQueueGet",
    "ExtEnvironment",
    "FetchInventory2",
    "FetchInventoryDescendents2",
    "FetchLib2",
    "FetchLibDescendents2",
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
    "InventoryAPIv3",
    "InventoryThumbnailUpload",
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
    "RenderMaterials",
    "RequestTaskInventory",
    "ResourceCostSelected",
    "SetDisplayName",
    "SimulatorFeatures",
    "UpdateAvatarAppearance",
    "UpdateAgentLanguage",
    "UpdateMaterialAgentInventory",
    "UpdateMaterialTaskInventory",
    "UpdateNotecardTaskInventory",
    "UpdateNotecardAgentInventory",
    "UpdateGestureAgentInventory",
    "UpdateScriptAgent",
    "UpdateScriptTask",
    "ViewerAsset",
    "ViewerStats",
    "VoiceSignalingRequest",
];

/// LLAgentLanguage::update: exact LLSD fields and boolean visibility.
pub fn agent_language_body(language: &str, public: bool) -> Llsd {
    llsd_map! { "language" => language, "language_is_public" => public }
}

#[cfg(test)]
mod language_tests {
    use super::*;

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

/// POST, or PUT (RenderMaterials writes, LLMaterialMgr::processPutQueue), of LLSD XML.
pub async fn request_llsd(http: &reqwest::Client, url: &str, body: &Llsd, put: bool) -> Result<Llsd, CapsError> {
    let req = if put { http.put(url) } else { http.post(url) };
    let resp = req
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .body(to_xml(body))
        .send()
        .await
        .map_err(|e| CapsError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| CapsError::Http(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(Llsd::Undef);
    }
    Ok(from_xml(&bytes)?)
}

/// POST the list of wanted capabilities to the seed and return the map.
pub async fn resolve_seed(http: &reqwest::Client, seed: &str) -> Result<HashMap<String, String>, CapsError> {
    let req = Llsd::Array(REQUESTED_CAPS.iter().map(|s| Llsd::from(*s)).collect());
    let mut last_err = None;
    for attempt in 0..4 {
        match post_llsd(http, seed, &req).await {
            Ok(v) => {
                let mut caps = HashMap::new();
                if let Some(m) = v.as_map() {
                    for (k, val) in m {
                        let s = val.to_string_value();
                        if !s.is_empty() {
                            caps.insert(k.clone(), s);
                        }
                    }
                }
                return Ok(caps);
            }
            Err(e) => {
                log::warn!("seed capability request failed (attempt {attempt}): {e}");
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (attempt + 1))).await;
            }
        }
    }
    Err(last_err.unwrap_or(CapsError::Status(0)))
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
    async fn seed_post_advertises_object_animation_without_requiring_a_cap_url() {
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
            let body = to_xml(&llsd_map! { "ViewerAsset" => "http://127.0.0.1/asset" });
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
        let resolved = resolve_seed(&http, &seed).await.expect("seed response");
        assert!(server.await.expect("seed server"), "simulator was not told to send ObjectAnimation");
        assert_eq!(resolved.get("ViewerAsset").map(String::as_str), Some("http://127.0.0.1/asset"));
        assert!(!resolved.contains_key("ObjectAnimation"));
    }
}
