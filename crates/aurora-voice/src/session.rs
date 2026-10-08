//! WebRTC connection to the Second Life voice server.
//!
//! The connection life cycle follows `LLVoiceWebRTCConnection::
//! connectionStateMachine` in Firestorm's `indra/newview/llvoicewebrtc.cpp`
//! (offer -> `ProvisionVoiceAccountRequest` -> answer -> ICE -> data channel
//! join -> position updates; logout + retry with a growing randomized delay
//! on failure), and the peer connection setup follows
//! `LLWebRTCPeerConnectionImpl::initializeConnection` in
//! `indra/llwebrtc/llwebrtc.cpp`. The microphone is encoded and sent by
//! [`crate::transmit`].
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//! Licensed under the GNU Lesser General Public License, version 2.1.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use aurora_audio::VoiceSink;
use aurora_llsd::Llsd;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_OPUS, MediaEngine};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_init::RTCDataChannelInit;
use webrtc::ice::udp_network::{EphemeralUDP, UDPNetwork};
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType};
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_remote::TrackRemote;

use crate::protocol::{self, AUDIO_TRACK_ID, DATA_CHANNEL_LABEL, LocalCandidate, STREAM_ID};
use crate::state::{LocalPrefs, ParticipantTable, SpatialState};
use crate::transmit::{FRAME_DURATION, OPUS_SILENCE, Payload, Transmitter, TxControl};
use crate::{VoiceConnectParams, VoiceStatus};

/// `UPDATE_THROTTLE_SECONDS`: state machine / position update period.
const UPDATE_PERIOD: Duration = Duration::from_millis(100);
/// `MAX_RETRY_WAIT_SECONDS`.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
/// `DISCONNECT_RENEGOTIATE_DELAY` (llwebrtc.cpp).
const DISCONNECT_RENEGOTIATE_DELAY: Duration = Duration::from_secs(10);
/// How long to wait for ICE gathering before sending the offer anyway.
const GATHER_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the peer connection may take to come up after the answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(5);
/// UDP port range used by llwebrtc (`set_min_port(60000)` / `set_max_port(60100)`).
const UDP_PORT_MIN: u16 = 60_000;
const UDP_PORT_MAX: u16 = 60_100;
/// How often the sender looks for microphone frames.
const TX_POLL: Duration = Duration::from_millis(10);

/// State shared between the API handle and the session thread.
pub(crate) struct Shared {
    pub status: Mutex<VoiceStatus>,
    pub participants: Mutex<ParticipantTable>,
    pub spatial: Mutex<SpatialState>,
    pub prefs: Mutex<LocalPrefs>,
    pub tx: TxControl,
}

impl Shared {
    pub(crate) fn new() -> Self {
        Self {
            status: Mutex::new(VoiceStatus::Connecting),
            participants: Mutex::new(ParticipantTable::default()),
            spatial: Mutex::new(SpatialState::default()),
            prefs: Mutex::new(LocalPrefs::default()),
            tx: TxControl::new(),
        }
    }

    fn set_status(&self, s: VoiceStatus) {
        *self.status.lock() = s;
    }
}

/// Commands from the API handle.
pub(crate) enum Cmd {
    /// Send a raw data-channel message (mute / gain).
    Data(String),
    Shutdown,
}

/// Events from webrtc callbacks.
enum Event {
    State(RTCPeerConnectionState),
    DataOpen(Arc<RTCDataChannel>),
    DataText(String),
}

enum Outcome {
    Shutdown,
    Retry,
}

/// A value in 0.5..1.5, like `(F32)rand() / RAND_MAX + 0.5f`.
fn jitter_secs() -> f32 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    0.5 + (nanos % 1_000_000) as f32 / 1_000_000.0
}

pub(crate) async fn run(
    http: reqwest::Client,
    params: VoiceConnectParams,
    sink: VoiceSink,
    shared: Arc<Shared>,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
) {
    if !protocol::is_webrtc_voice_server(&params.voice_server_type) {
        shared.set_status(VoiceStatus::Unsupported("Vivox regions not supported".to_string()));
        // Stay alive until dropped so the status remains observable.
        while let Some(cmd) = cmd_rx.recv().await {
            if matches!(cmd, Cmd::Shutdown) {
                break;
            }
        }
        return;
    }

    let sink = Arc::new(Mutex::new(sink));
    let mut retry_wait = Duration::from_secs_f32(jitter_secs());
    loop {
        match connect_once(&http, &params, &sink, &shared, &mut cmd_rx).await {
            Outcome::Shutdown => break,
            Outcome::Retry => {}
        }
        shared.participants.lock().clear();
        // Wait before retrying; shutdown interrupts.
        let deadline = tokio::time::Instant::now() + retry_wait;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                cmd = cmd_rx.recv() => match cmd {
                    None | Some(Cmd::Shutdown) => {
                        shared.set_status(VoiceStatus::Disconnected);
                        return;
                    }
                    Some(Cmd::Data(_)) => {}
                },
            }
        }
        if retry_wait < MAX_RETRY_WAIT {
            retry_wait += Duration::from_secs_f32(jitter_secs());
        }
    }
    shared.set_status(VoiceStatus::Disconnected);
}

fn opus_capability() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
        rtcp_feedback: vec![],
    }
}

async fn new_peer_connection(params: &VoiceConnectParams) -> Result<Arc<RTCPeerConnection>, String> {
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                capability: opus_capability(),
                payload_type: 111,
                ..Default::default()
            },
            RTPCodecType::Audio,
        )
        .map_err(|e| e.to_string())?;
    let registry = register_default_interceptors(Registry::new(), &mut media).map_err(|e| e.to_string())?;
    let mut settings = SettingEngine::default();
    match EphemeralUDP::new(UDP_PORT_MIN, UDP_PORT_MAX) {
        Ok(udp) => settings.set_udp_network(UDPNetwork::Ephemeral(udp)),
        Err(e) => log::warn!("voice: cannot restrict UDP ports: {e}"),
    }
    #[cfg(test)]
    if tests::LOOPBACK_ONLY.load(std::sync::atomic::Ordering::Relaxed) {
        tests::restrict_to_loopback(&mut settings);
    }
    let api = APIBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_setting_engine(settings)
        .build();
    let urls = match &params.stun_servers {
        Some(list) => list.clone(),
        None => protocol::stun_servers_for_grid(&params.grid),
    };
    let ice_servers = if urls.is_empty() {
        vec![]
    } else {
        vec![RTCIceServer {
            urls,
            ..Default::default()
        }]
    };
    let config = RTCConfiguration {
        ice_servers,
        ..Default::default()
    };
    api.new_peer_connection(config).await.map(Arc::new).map_err(|e| e.to_string())
}

fn to_local_candidate(c: &RTCIceCandidate) -> LocalCandidate {
    LocalCandidate {
        foundation: c.foundation.clone(),
        component: c.component,
        protocol: c.protocol.to_string(),
        priority: c.priority,
        address: c.address.clone(),
        port: c.port,
        typ: c.typ.to_string(),
        related_address: c.related_address.clone(),
        related_port: c.related_port,
        tcp_type: c.tcp_type.clone(),
        // Audio is the first m-line (bundled), as with llwebrtc.
        sdp_mid: "0".to_string(),
        mline_index: 0,
    }
}

async fn post_llsd(http: &reqwest::Client, url: &str, body: &Llsd, timeout: Duration) -> Result<Llsd, PostError> {
    let resp = http
        .post(url)
        .header("Content-Type", "application/llsd+xml")
        .header("Accept", "application/llsd+xml")
        .timeout(timeout)
        .body(aurora_llsd::to_xml_string(body))
        .send()
        .await
        .map_err(|e| PostError::Transport(e.to_string()))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(PostError::Status(status));
    }
    let bytes = resp.bytes().await.map_err(|e| PostError::Transport(e.to_string()))?;
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(Llsd::Undef);
    }
    aurora_llsd::from_xml(&bytes).map_err(|e| PostError::Transport(format!("bad LLSD: {e}")))
}

#[derive(Debug)]
enum PostError {
    Status(u16),
    Transport(String),
}

fn status_for_provision_error(e: &PostError) -> VoiceStatus {
    match e {
        // HTTP_CONFLICT -> ERROR_CHANNEL_FULL, HTTP_UNAUTHORIZED -> ERROR_CHANNEL_LOCKED.
        PostError::Status(409) => VoiceStatus::ChannelFull,
        PostError::Status(401) => VoiceStatus::ChannelLocked,
        PostError::Status(s) => VoiceStatus::Error(format!("voice provisioning failed: HTTP {s}")),
        PostError::Transport(t) => VoiceStatus::Error(format!("voice provisioning failed: {t}")),
    }
}

/// 120 ms at 48 kHz: the longest Opus packet, per channel.
const MAX_OPUS_FRAME: usize = 5760;
/// Samples per channel concealed for one lost packet (the server sends 20 ms).
const PLC_FRAME: usize = 960;

/// Decodes the incoming Opus track into the Voice channel.
async fn receive_audio(track: Arc<TrackRemote>, sink: Arc<Mutex<VoiceSink>>) {
    let mut decoder = match rusty_opus::OpusDecoder::new(48_000, 2) {
        Ok(d) => d,
        Err(e) => {
            log::error!("voice: cannot create Opus decoder: {e:?}");
            return;
        }
    };
    let mut pcm = vec![0.0f32; MAX_OPUS_FRAME * 2];
    let mut last_seq: Option<u16> = None;
    while let Ok((packet, _)) = track.read_rtp().await {
        let seq = packet.header.sequence_number;
        if let Some(prev) = last_seq {
            let gap = seq.wrapping_sub(prev).wrapping_sub(1);
            if gap > 0x8000 {
                // Late / duplicate packet: drop it.
                continue;
            }
            // Conceal short losses (packet loss concealment).
            for _ in 0..gap.min(5) {
                if let Ok(n) = decoder.decode(&[], PLC_FRAME, &mut pcm) {
                    sink.lock().push(&pcm[..(n * 2).min(pcm.len())], 2);
                }
            }
        }
        last_seq = Some(seq);
        if packet.payload.is_empty() {
            continue;
        }
        match decoder.decode(&packet.payload, MAX_OPUS_FRAME, &mut pcm) {
            Ok(n) => {
                sink.lock().push(&pcm[..(n * 2).min(pcm.len())], 2);
            }
            Err(e) => log::debug!("voice: undecodable Opus packet: {e:?}"),
        }
    }
    log::debug!("voice: remote audio track ended");
}

/// Sends the microphone on the local track: Opus while transmitting, the
/// muted-track silence frame otherwise (and while no microphone delivers),
/// so the media path stays established in both directions.
async fn send_audio(track: Arc<TrackLocalStaticSample>, shared: Arc<Shared>, mut stop: tokio::sync::watch::Receiver<bool>) {
    let mut tx = Transmitter::new();
    let mut payloads: Vec<Payload> = Vec::with_capacity(4);
    let mut tick = tokio::time::interval(TX_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    'run: loop {
        tokio::select! {
            _ = tick.tick() => {
                let mic = shared.tx.mic.lock().clone();
                let transmit = shared.tx.transmit();
                tx.poll(Instant::now(), mic.as_ref(), transmit, shared.tx.gain(), &mut payloads);
                shared.tx.set_level(tx.level(transmit));
                for payload in payloads.drain(..) {
                    let data = match payload {
                        Payload::Silence => bytes::Bytes::from_static(&OPUS_SILENCE),
                        Payload::Opus(v) => bytes::Bytes::from(v),
                    };
                    let sample = webrtc::media::Sample {
                        data,
                        duration: FRAME_DURATION,
                        ..Default::default()
                    };
                    if track.write_sample(&sample).await.is_err() {
                        break 'run;
                    }
                }
            }
            _ = stop.changed() => break,
        }
    }
    shared.tx.set_level(0.0);
}

fn hook_data_channel(dc: &Arc<RTCDataChannel>, ev_tx: &mpsc::UnboundedSender<Event>) {
    let tx = ev_tx.clone();
    let dc_open = dc.clone();
    dc.on_open(Box::new(move || {
        let _ = tx.send(Event::DataOpen(dc_open));
        Box::pin(async {})
    }));
    let tx = ev_tx.clone();
    dc.on_message(Box::new(move |msg| {
        if msg.is_string {
            let text = String::from_utf8_lossy(&msg.data).to_string();
            let _ = tx.send(Event::DataText(text));
        } else {
            log::warn!("voice: binary data received from data channel");
        }
        Box::pin(async {})
    }));
}

async fn logout(http: &reqwest::Client, url: &str, viewer_session: &Llsd) {
    let body = protocol::build_logout_request(viewer_session);
    if let Err(e) = post_llsd(http, url, &body, LOGOUT_TIMEOUT).await {
        log::debug!("voice logout failed: {e:?}");
    }
}

async fn connect_once(
    http: &reqwest::Client,
    params: &VoiceConnectParams,
    sink: &Arc<Mutex<VoiceSink>>,
    shared: &Arc<Shared>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
) -> Outcome {
    let pc = match new_peer_connection(params).await {
        Ok(pc) => pc,
        Err(e) => {
            shared.set_status(VoiceStatus::Error(format!("WebRTC setup failed: {e}")));
            return Outcome::Retry;
        }
    };
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let outcome = negotiate_and_run(http, params, sink, shared, cmd_rx, &pc, stop_rx).await;
    let _ = stop_tx.send(true);
    if let Err(e) = pc.close().await {
        log::debug!("voice: closing peer connection: {e}");
    }
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn negotiate_and_run(
    http: &reqwest::Client,
    params: &VoiceConnectParams,
    sink: &Arc<Mutex<VoiceSink>>,
    shared: &Arc<Shared>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
    pc: &Arc<RTCPeerConnection>,
    stop_rx: tokio::sync::watch::Receiver<bool>,
) -> Outcome {
    let fail = |msg: String| {
        log::warn!("voice: {msg}");
        shared.set_status(VoiceStatus::Error(msg));
        Outcome::Retry
    };

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Event>();

    // Data channel "SLData", ordered (llwebrtc `initializeConnection`).
    let dc = match pc
        .create_data_channel(
            DATA_CHANNEL_LABEL,
            Some(RTCDataChannelInit {
                ordered: Some(true),
                ..Default::default()
            }),
        )
        .await
    {
        Ok(dc) => dc,
        Err(e) => return fail(format!("cannot create data channel: {e}")),
    };
    hook_data_channel(&dc, &ev_tx);
    {
        // The server may also open its own channel (`OnDataChannel`).
        let tx = ev_tx.clone();
        pc.on_data_channel(Box::new(move |dc| {
            hook_data_channel(&dc, &tx);
            Box::pin(async {})
        }));
    }

    // Local audio track ("SLAudio" in stream "SLStream"): the microphone,
    // silent until transmission is enabled.
    let local_track = Arc::new(TrackLocalStaticSample::new(
        opus_capability(),
        AUDIO_TRACK_ID.to_owned(),
        STREAM_ID.to_owned(),
    ));
    if let Err(e) = pc.add_track(local_track.clone()).await {
        return fail(format!("cannot add audio track: {e}"));
    }

    {
        let sink = sink.clone();
        pc.on_track(Box::new(move |track, _receiver, _transceiver| {
            let sink = sink.clone();
            tokio::spawn(receive_audio(track, sink));
            Box::pin(async {})
        }));
    }
    {
        let tx = ev_tx.clone();
        pc.on_peer_connection_state_change(Box::new(move |s| {
            let _ = tx.send(Event::State(s));
            Box::pin(async {})
        }));
    }
    let candidates: Arc<Mutex<Vec<LocalCandidate>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let candidates = candidates.clone();
        pc.on_ice_candidate(Box::new(move |c| {
            if let Some(c) = c {
                candidates.lock().push(to_local_candidate(&c));
            }
            Box::pin(async {})
        }));
    }

    // Offer; wait for ICE gathering so the offer carries our candidates.
    // webrtc-rs refuses a modified local description, so the offer is set
    // as generated and only the copy sent to the server is munged (the
    // munging only expresses what we want to *receive*: 48 kHz stereo).
    let offer = match pc.create_offer(None).await {
        Ok(o) => o,
        Err(e) => return fail(format!("cannot create offer: {e}")),
    };
    let mut gathered = pc.gathering_complete_promise().await;
    if let Err(e) = pc.set_local_description(offer).await {
        return fail(format!("cannot set local description: {e}"));
    }
    if tokio::time::timeout(GATHER_TIMEOUT, gathered.recv()).await.is_err() {
        log::debug!("voice: ICE gathering not complete after {GATHER_TIMEOUT:?}, sending offer anyway");
    }
    let offer_sdp = match pc.local_description().await {
        Some(d) => protocol::mangle_offer_sdp(&d.sdp),
        None => return fail("no local description".into()),
    };

    // ProvisionVoiceAccountRequest with the offer.
    let body = protocol::build_provision_request(&offer_sdp, &params.channel);
    let result = tokio::select! {
        r = post_llsd(http, &params.provision_cap_url, &body, HTTP_TIMEOUT) => r,
        _ = wait_shutdown(cmd_rx) => return Outcome::Shutdown,
    };
    let answer = match result {
        Ok(llsd) => match protocol::parse_provision_response(&llsd) {
            Some(a) => a,
            None => return fail("invalid voice provision response".into()),
        },
        Err(e) => {
            let status = status_for_provision_error(&e);
            log::warn!("voice: provisioning failed: {e:?}");
            shared.set_status(status);
            return Outcome::Retry;
        }
    };
    let viewer_session = answer.viewer_session.clone();

    let outcome = async {
        let remote = match RTCSessionDescription::answer(answer.sdp) {
            Ok(d) => d,
            Err(e) => return fail(format!("invalid answer SDP: {e}")),
        };
        if let Err(e) = pc.set_remote_description(remote).await {
            return fail(format!("cannot set remote description: {e}"));
        }

        // Trickle our candidates through VoiceSignalingRequest, then mark
        // gathering complete (processIceUpdatesCoro). They are also in the
        // offer SDP, so failures here are not fatal.
        if let Some(url) = params.signaling_cap_url.as_deref().filter(|u| !u.is_empty()) {
            let list = std::mem::take(&mut *candidates.lock());
            if let Some(body) = protocol::build_ice_request(&list, false, &viewer_session)
                && let Err(e) = post_llsd(http, url, &body, HTTP_TIMEOUT).await
            {
                log::debug!("voice: ICE trickle failed: {e:?}");
            }
            if let Some(body) = protocol::build_ice_request(&[], true, &viewer_session)
                && let Err(e) = post_llsd(http, url, &body, HTTP_TIMEOUT).await
            {
                log::debug!("voice: ICE completion failed: {e:?}");
            }
        }

        tokio::spawn(send_audio(local_track.clone(), shared.clone(), stop_rx.clone()));
        session_loop(params, shared, cmd_rx, &mut ev_rx).await
    }
    .await;

    // breakVoiceConnectionCoro: tell the server we are leaving.
    logout(http, &params.provision_cap_url, &viewer_session).await;
    outcome
}

/// Resolves when a shutdown is requested (or the handle is gone).
async fn wait_shutdown(cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>) {
    loop {
        match cmd_rx.recv().await {
            None | Some(Cmd::Shutdown) => return,
            Some(Cmd::Data(_)) => {}
        }
    }
}

async fn session_loop(
    params: &VoiceConnectParams,
    shared: &Arc<Shared>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
    ev_rx: &mut mpsc::UnboundedReceiver<Event>,
) -> Outcome {
    let spatial = params.channel.is_spatial();
    let started = Instant::now();
    let mut connected = false;
    let mut disconnected_since: Option<Instant> = None;
    let mut data: Option<Arc<RTCDataChannel>> = None;
    let mut joined = false;
    let mut tick = tokio::time::interval(UPDATE_PERIOD);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                None | Some(Cmd::Shutdown) => return Outcome::Shutdown,
                Some(Cmd::Data(msg)) => {
                    if let (Some(dc), true) = (&data, joined) {
                        let _ = dc.send_text(msg).await;
                    }
                }
            },
            ev = ev_rx.recv() => match ev {
                None => return Outcome::Retry,
                Some(Event::State(s)) => {
                    log::debug!("voice: peer connection state {s}");
                    match s {
                        RTCPeerConnectionState::Connected => {
                            connected = true;
                            disconnected_since = None;
                        }
                        RTCPeerConnectionState::Disconnected => {
                            disconnected_since.get_or_insert_with(Instant::now);
                        }
                        RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                            shared.set_status(VoiceStatus::Error("voice connection lost".into()));
                            return Outcome::Retry;
                        }
                        _ => {}
                    }
                }
                Some(Event::DataOpen(dc)) => {
                    // VOICE_STATE_WAIT_FOR_DATA_CHANNEL: join, then position.
                    let _ = dc.send_text(protocol::join_message(true)).await;
                    if spatial {
                        let msg = shared.spatial.lock().take_message(true);
                        if let Some(msg) = msg {
                            let _ = dc.send_text(msg).await;
                        }
                    }
                    data = Some(dc);
                    joined = true;
                }
                Some(Event::DataText(text)) => {
                    match protocol::parse_data_message(&text) {
                        Ok(updates) => {
                            let reply = {
                                let prefs = shared.prefs.lock();
                                shared.participants.lock().apply(&updates, &prefs, spatial, params.agent_id)
                            };
                            if let (Some(reply), Some(dc)) = (reply, &data) {
                                let _ = dc.send_text(reply).await;
                            }
                        }
                        Err(e) => log::warn!("voice: bad data channel message: {e}"),
                    }
                }
            },
            _ = tick.tick() => {
                if connected && joined {
                    if !matches!(*shared.status.lock(), VoiceStatus::Connected) {
                        shared.set_status(VoiceStatus::Connected);
                    }
                    if spatial {
                        let msg = shared.spatial.lock().take_message(false);
                        if let (Some(msg), Some(dc)) = (msg, &data) {
                            let _ = dc.send_text(msg).await;
                        }
                    }
                }
                if let Some(since) = disconnected_since
                    && since.elapsed() > DISCONNECT_RENEGOTIATE_DELAY
                {
                    shared.set_status(VoiceStatus::Error("voice connection lost".into()));
                    return Outcome::Retry;
                }
                if !connected && started.elapsed() > CONNECT_TIMEOUT {
                    shared.set_status(VoiceStatus::Error("voice connection timed out".into()));
                    return Outcome::Retry;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
pub(crate) mod tests;
