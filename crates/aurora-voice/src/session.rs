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

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use aurora_audio::VoiceSink;
use aurora_llsd::Llsd;
use parking_lot::Mutex;
use rtc::ice::mdns::MulticastDnsMode;
use rtc::media::Sample;
use rtc::media_stream::MediaStreamTrack;
use rtc::peer_connection::configuration::media_engine::MIME_TYPE_OPUS;
use rtc::rtp_transceiver::rtp_sender::{
    RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
};
use tokio::sync::{Notify, mpsc, watch};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelInit};
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder, RTCIceCandidate,
    RTCIceGatheringState, RTCIceServer, RTCPeerConnectionIceEvent, RTCPeerConnectionState, RTCSessionDescription, Registry,
    SettingEngineBuilder, register_default_interceptors,
};

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
    DataOpen(Arc<dyn DataChannel>),
    DataText(String),
}

enum Outcome {
    Shutdown,
    Retry,
}

/// Sub-second clock noise, the only randomness the retry jitter needs.
fn clock_nanos() -> u32 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

/// A value in 0.5..1.5, like `(F32)rand() / RAND_MAX + 0.5f`.
fn jitter_secs() -> f32 {
    0.5 + (clock_nanos() % 1_000_000) as f32 / 1_000_000.0
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

fn opus_capability() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: MIME_TYPE_OPUS.to_owned(),
        clock_rate: 48_000,
        channels: 2,
        sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
        rtcp_feedback: vec![],
    }
}

/// SSRC of a local track. webrtc 0.21 no longer draws it when the track is
/// bound: the application names it in the track's encoding.
fn random_ssrc() -> u32 {
    (uuid::Uuid::new_v4().as_u128() & u128::from(u32::MAX)) as u32
}

/// A free port of llwebrtc's range, probed from a random start like the
/// ephemeral port-range allocator of webrtc 0.17 (`EphemeralUDP`), which
/// 0.21 no longer has.
fn free_udp_port() -> Option<u16> {
    let span = UDP_PORT_MAX - UDP_PORT_MIN + 1;
    let start = (clock_nanos() % u32::from(span)) as u16;
    (0..span)
        .map(|i| UDP_PORT_MIN + (start + i) % span)
        .find(|&port| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok())
}

/// Whether an interface has an IPv6 address a peer can reach. Without one a
/// `[::]` bind is kept unexpanded by webrtc and would advertise `::`.
fn has_routable_ipv6() -> bool {
    rtc::shared::ifaces::ifaces().is_ok_and(|list| {
        list.iter().filter_map(|i| i.addr).any(|a| match a.ip() {
            IpAddr::V6(ip) => !ip.is_loopback() && !ip.is_unspecified() && !ip.is_unicast_link_local(),
            IpAddr::V4(_) => false,
        })
    })
}

/// Local UDP sockets for ICE: every interface, IPv4 and IPv6 (webrtc 0.17's
/// default network types), on one port of the llwebrtc range. webrtc 0.21
/// expands each wildcard into one socket per routable interface address.
fn udp_bind_addrs() -> Vec<String> {
    let port = free_udp_port().unwrap_or_else(|| {
        log::warn!("voice: cannot restrict UDP ports: no free port in {UDP_PORT_MIN}..={UDP_PORT_MAX}");
        0
    });
    #[cfg(test)]
    if tests::LOOPBACK_ONLY.load(Ordering::Relaxed) {
        return vec![format!("127.0.0.1:{port}")];
    }
    let mut addrs = vec![format!("0.0.0.0:{port}")];
    if has_routable_ipv6() {
        addrs.push(format!("[::]:{port}"));
    }
    addrs
}

async fn new_peer_connection(params: &VoiceConnectParams, handler: Arc<Handler>) -> Result<Arc<dyn PeerConnection>, String> {
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
    let udp_addrs = udp_bind_addrs();
    match build_peer_connection(&ice_servers, &udp_addrs, MulticastDnsMode::QueryOnly, handler.clone()).await {
        Ok(pc) => Ok(pc),
        // mDNS (resolving `.local` candidates) is opportunistic, as it was in
        // webrtc 0.17; 0.21 fails the whole build when its multicast socket
        // cannot be opened.
        Err(e) => {
            log::warn!("voice: WebRTC setup with mDNS failed ({e}), retrying without mDNS");
            build_peer_connection(&ice_servers, &udp_addrs, MulticastDnsMode::Disabled, handler).await
        }
    }
}

async fn build_peer_connection(
    ice_servers: &[RTCIceServer],
    udp_addrs: &[String],
    mdns: MulticastDnsMode,
    handler: Arc<Handler>,
) -> Result<Arc<dyn PeerConnection>, String> {
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: opus_capability(),
                payload_type: 111,
            },
            RtpCodecKind::Audio,
        )
        .map_err(|e| e.to_string())?;
    let registry = register_default_interceptors(Registry::new(), &mut media).map_err(|e| e.to_string())?;
    let settings = SettingEngineBuilder::new().with_multicast_dns_mode(mdns);
    #[cfg(test)]
    let settings = if tests::LOOPBACK_ONLY.load(Ordering::Relaxed) {
        tests::restrict_to_loopback(settings)
    } else {
        settings
    };
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().with_ice_servers(ice_servers.to_vec()).build())
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_setting_engine(settings.build())
        .with_handler(handler)
        .with_udp_addrs(udp_addrs.to_vec())
        .build()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(pc))
}

/// Peer connection callbacks. webrtc 0.21 awaits them inline on its driver,
/// so each one only records or forwards.
struct Handler {
    events: mpsc::UnboundedSender<Event>,
    candidates: Mutex<Vec<LocalCandidate>>,
    gathered: Notify,
    /// The DTLS/SRTP path has come up: from then on the local track sends
    /// (webrtc 0.17 bound it there and kept it bound until the close).
    connected: Arc<AtomicBool>,
    sink: Arc<Mutex<VoiceSink>>,
    stop: watch::Receiver<bool>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        self.candidates.lock().push(to_local_candidate(&event.candidate));
    }

    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.notify_one();
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if state == RTCPeerConnectionState::Connected {
            self.connected.store(true, Ordering::Relaxed);
        }
        let _ = self.events.send(Event::State(state));
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        // The server may also open its own channel (`OnDataChannel`).
        watch_data_channel(dc, self.events.clone(), self.stop.clone());
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        tokio::spawn(receive_audio(track, self.sink.clone(), self.stop.clone()));
    }
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
        tcp_type: c.tcp_type.to_ice().to_string(),
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
async fn receive_audio(track: Arc<dyn TrackRemote>, sink: Arc<Mutex<VoiceSink>>, mut stop: watch::Receiver<bool>) {
    let mut decoder = match rusty_opus::OpusDecoder::new(48_000, 2) {
        Ok(d) => d,
        Err(e) => {
            log::error!("voice: cannot create Opus decoder: {e:?}");
            return;
        }
    };
    let mut pcm = vec![0.0f32; MAX_OPUS_FRAME * 2];
    let mut last_seq: Option<u16> = None;
    loop {
        // webrtc 0.21 keeps a closed connection's track queues open, so the
        // session's stop signal also ends the loop.
        let event = tokio::select! {
            event = track.poll() => event,
            _ = stop.changed() => break,
        };
        let packet = match event {
            Some(TrackRemoteEvent::OnRtpPacket(packet)) => packet,
            None | Some(TrackRemoteEvent::OnEnded) => break,
            Some(_) => continue,
        };
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
/// so the media path stays established in both directions. Frames produced
/// before the connection first comes up are dropped, as webrtc 0.17 did for
/// a track not yet bound to its DTLS transport (0.21 would log each one as
/// a send error).
async fn send_audio(
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    connected: Arc<AtomicBool>,
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
) {
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
                    if !connected.load(Ordering::Relaxed) {
                        continue;
                    }
                    let data = match payload {
                        Payload::Silence => bytes::Bytes::from_static(&OPUS_SILENCE),
                        Payload::Opus(v) => bytes::Bytes::from(v),
                    };
                    let sample = Sample {
                        data,
                        duration: FRAME_DURATION,
                        ..Sample::new(Instant::now())
                    };
                    if track.write_sample(ssrc, payload_type, &sample, &[]).await.is_err() {
                        break 'run;
                    }
                }
            }
            _ = stop.changed() => break,
        }
    }
    shared.tx.set_level(0.0);
}

/// Forwards a data channel's open and text messages to the session loop
/// (webrtc 0.21 delivers them by polling the channel), until the channel or
/// the session ends.
fn watch_data_channel(dc: Arc<dyn DataChannel>, ev_tx: mpsc::UnboundedSender<Event>, mut stop: watch::Receiver<bool>) {
    tokio::spawn(async move {
        loop {
            let event = tokio::select! {
                event = dc.poll() => event,
                _ = stop.changed() => break,
            };
            match event {
                None => break,
                Some(DataChannelEvent::OnOpen) => {
                    let _ = ev_tx.send(Event::DataOpen(dc.clone()));
                }
                Some(DataChannelEvent::OnMessage(msg)) => {
                    if msg.is_string {
                        let text = String::from_utf8_lossy(&msg.data).to_string();
                        let _ = ev_tx.send(Event::DataText(text));
                    } else {
                        log::warn!("voice: binary data received from data channel");
                    }
                }
                Some(_) => {}
            }
        }
    });
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
    let (stop_tx, stop_rx) = watch::channel(false);
    let (ev_tx, ev_rx) = mpsc::unbounded_channel::<Event>();
    let handler = Arc::new(Handler {
        events: ev_tx,
        candidates: Mutex::new(Vec::new()),
        gathered: Notify::new(),
        connected: Arc::new(AtomicBool::new(false)),
        sink: sink.clone(),
        stop: stop_rx.clone(),
    });
    let pc = match new_peer_connection(params, handler.clone()).await {
        Ok(pc) => pc,
        Err(e) => {
            shared.set_status(VoiceStatus::Error(format!("WebRTC setup failed: {e}")));
            return Outcome::Retry;
        }
    };
    let outcome = negotiate_and_run(http, params, shared, cmd_rx, &pc, &handler, ev_rx, stop_rx).await;
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
    shared: &Arc<Shared>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
    pc: &Arc<dyn PeerConnection>,
    handler: &Arc<Handler>,
    mut ev_rx: mpsc::UnboundedReceiver<Event>,
    stop_rx: watch::Receiver<bool>,
) -> Outcome {
    let fail = |msg: String| {
        log::warn!("voice: {msg}");
        shared.set_status(VoiceStatus::Error(msg));
        Outcome::Retry
    };

    // Data channel "SLData", ordered (llwebrtc `initializeConnection`).
    let dc = match pc
        .create_data_channel(
            DATA_CHANNEL_LABEL,
            Some(RTCDataChannelInit {
                ordered: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(dc) => dc,
        Err(e) => return fail(format!("cannot create data channel: {e}")),
    };
    watch_data_channel(dc, handler.events.clone(), stop_rx.clone());

    // Local audio track ("SLAudio" in stream "SLStream"): the microphone,
    // silent until transmission is enabled.
    let ssrc = random_ssrc();
    let local_track = match TrackLocalStaticSample::new(
        Instant::now(),
        MediaStreamTrack::new(
            STREAM_ID.to_owned(),
            AUDIO_TRACK_ID.to_owned(),
            AUDIO_TRACK_ID.to_owned(),
            RtpCodecKind::Audio,
            vec![RTCRtpEncodingParameters {
                rtp_coding_parameters: RTCRtpCodingParameters {
                    ssrc: Some(ssrc),
                    ..Default::default()
                },
                codec: opus_capability(),
                ..Default::default()
            }],
        ),
    ) {
        Ok(track) => Arc::new(track),
        Err(e) => return fail(format!("cannot create audio track: {e}")),
    };
    let sender = match pc.add_track(local_track.clone() as Arc<dyn TrackLocal>).await {
        Ok(sender) => sender,
        Err(e) => return fail(format!("cannot add audio track: {e}")),
    };

    // Offer; wait for ICE gathering so the offer carries our candidates.
    // webrtc-rs refuses a modified local description, so the offer is set
    // as generated and only the copy sent to the server is munged (the
    // munging only expresses what we want to *receive*: 48 kHz stereo).
    let offer = match pc.create_offer(None).await {
        Ok(o) => o,
        Err(e) => return fail(format!("cannot create offer: {e}")),
    };
    if let Err(e) = pc.set_local_description(offer).await {
        return fail(format!("cannot set local description: {e}"));
    }
    if tokio::time::timeout(GATHER_TIMEOUT, handler.gathered.notified()).await.is_err() {
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
        // webrtc 0.21 stamps each sample with the payload type it is given;
        // use the one negotiated for the sender, as 0.17 did when binding.
        let payload_type = match sender.get_parameters().await {
            Ok(p) => match p.rtp_parameters.codecs.first() {
                Some(codec) => codec.payload_type,
                None => return fail("no audio codec negotiated".into()),
            },
            Err(e) => return fail(format!("cannot read audio sender parameters: {e}")),
        };

        // Trickle our candidates through VoiceSignalingRequest, then mark
        // gathering complete (processIceUpdatesCoro). They are also in the
        // offer SDP, so failures here are not fatal.
        if let Some(url) = params.signaling_cap_url.as_deref().filter(|u| !u.is_empty()) {
            let list = std::mem::take(&mut *handler.candidates.lock());
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

        tokio::spawn(send_audio(
            local_track.clone(),
            ssrc,
            payload_type,
            handler.connected.clone(),
            shared.clone(),
            stop_rx.clone(),
        ));
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
    let mut data: Option<Arc<dyn DataChannel>> = None;
    let mut joined = false;
    let mut tick = tokio::time::interval(UPDATE_PERIOD);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                None | Some(Cmd::Shutdown) => return Outcome::Shutdown,
                Some(Cmd::Data(msg)) => {
                    if let (Some(dc), true) = (&data, joined) {
                        let _ = dc.send_text(&msg).await;
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
                    let _ = dc.send_text(&protocol::join_message(true)).await;
                    if spatial {
                        let msg = shared.spatial.lock().take_message(true);
                        if let Some(msg) = msg {
                            let _ = dc.send_text(&msg).await;
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
                                let _ = dc.send_text(&reply).await;
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
                            let _ = dc.send_text(&msg).await;
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
