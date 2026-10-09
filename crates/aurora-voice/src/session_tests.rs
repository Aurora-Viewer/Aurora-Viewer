//! End-to-end test against an in-process fake voice server on loopback:
//! LLSD provisioning over HTTP, a webrtc-rs answerer sending Opus and
//! data-channel messages and recording the client's Opus (microphone).
//! No external network is used (loopback-only ICE, no STUN, no mDNS).

use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

use aurora_audio::{MicCapture, PcmSink};
use glam::{DVec3, Quat};
use rtc::ice::network_type::NetworkType;
use uuid::Uuid;

use crate::{ChannelKind, VoiceConnectParams, VoiceSession};

pub(crate) static LOOPBACK_ONLY: AtomicBool = AtomicBool::new(false);

/// Loopback candidates only; the sockets themselves are bound on 127.0.0.1
/// (`udp_bind_addrs`), webrtc 0.21 having no IP filter.
pub(crate) fn restrict_to_loopback(settings: SettingEngineBuilder) -> SettingEngineBuilder {
    settings
        .with_include_loopback_candidate(true)
        .with_network_types(vec![NetworkType::Udp4])
        .with_multicast_dns_mode(MulticastDnsMode::Disabled)
}

#[derive(Default)]
struct ServerState {
    provision_bodies: Mutex<Vec<Llsd>>,
    logouts: Mutex<Vec<Llsd>>,
    data_messages: Mutex<Vec<String>>,
    peers: Mutex<Vec<Arc<dyn PeerConnection>>>,
    /// Opus payloads received from the client, in arrival order.
    rx_packets: Mutex<Vec<Vec<u8>>>,
}

/// Callbacks of the fake server's peer connection.
struct ServerHandler {
    state: Arc<ServerState>,
    other: Uuid,
    gathered: Notify,
    connected: Notify,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for ServerHandler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            self.gathered.notify_one();
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        if state == RTCPeerConnectionState::Connected {
            self.connected.notify_one();
        }
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let (state, other) = (self.state.clone(), self.other);
        tokio::spawn(async move {
            while let Some(event) = dc.poll().await {
                let DataChannelEvent::OnMessage(msg) = event else { continue };
                let text = String::from_utf8_lossy(&msg.data).to_string();
                let is_join = text.contains(r#""j""#);
                state.data_messages.lock().push(text);
                if is_join {
                    let roster = format!(r#"{{"{other}":{{"j":{{"p":true}},"v":true,"p":64}}}}"#);
                    let _ = dc.send_text(&roster).await;
                }
            }
        });
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        // Record what the client transmits.
        let state = self.state.clone();
        tokio::spawn(async move {
            while let Some(event) = track.poll().await {
                if let TrackRemoteEvent::OnRtpPacket(packet) = event {
                    state.rx_packets.lock().push(packet.payload.to_vec());
                }
            }
        });
    }
}

async fn make_answer(offer: String, state: Arc<ServerState>, other: Uuid) -> String {
    let mut media = MediaEngine::default();
    media.register_default_codecs().unwrap();
    let registry = register_default_interceptors(Registry::new(), &mut media).unwrap();
    let handler = Arc::new(ServerHandler {
        state: state.clone(),
        other,
        gathered: Notify::new(),
        connected: Notify::new(),
    });
    let pc: Arc<dyn PeerConnection> = Arc::new(
        PeerConnectionBuilder::new()
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_setting_engine(restrict_to_loopback(SettingEngineBuilder::new()).build())
            .with_handler(handler.clone())
            .with_udp_addrs(vec!["127.0.0.1:0"])
            .build()
            .await
            .unwrap(),
    );
    // Add the sending track first so the offered audio m-line is matched
    // with a sendrecv transceiver.
    let ssrc = random_ssrc();
    let track = Arc::new(
        TrackLocalStaticSample::new(
            Instant::now(),
            MediaStreamTrack::new(
                "srv-stream".into(),
                "srv-audio".into(),
                "srv-audio".into(),
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
        )
        .unwrap(),
    );
    let sender = pc.add_track(track.clone() as Arc<dyn TrackLocal>).await.unwrap();
    pc.set_remote_description(RTCSessionDescription::offer(offer).unwrap())
        .await
        .unwrap();
    let payload_type = sender.get_parameters().await.unwrap().rtp_parameters.codecs[0].payload_type;
    let answer = pc.create_answer(None).await.unwrap();
    pc.set_local_description(answer).await.unwrap();
    handler.gathered.notified().await;
    tokio::spawn(async move {
        handler.connected.notified().await;
        loop {
            let sample = Sample {
                data: bytes::Bytes::from_static(&OPUS_SILENCE),
                duration: Duration::from_millis(20),
                ..Sample::new(Instant::now())
            };
            if track.write_sample(ssrc, payload_type, &sample, &[]).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let sdp = pc.local_description().await.unwrap().sdp;
    state.peers.lock().push(pc);
    sdp
}

/// Minimal HTTP/1.1 server answering the provisioning capability.
fn start_fake_server(rt: tokio::runtime::Handle, state: Arc<ServerState>, other: Uuid) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut sock) = conn else { return };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, content_len) = loop {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (pos + 4, len);
                }
            };
            if head_end == 0 {
                continue;
            }
            while buf.len() < head_end + content_len {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = aurora_llsd::from_xml(&buf[head_end..]).unwrap_or_default();
            let reply = if body.has("logout") {
                state.logouts.lock().push(body);
                "<llsd><undef /></llsd>".to_string()
            } else {
                let offer = body.get("jsep").get("sdp").to_string_value();
                state.provision_bodies.lock().push(body);
                let sdp = rt.block_on(make_answer(offer, state.clone(), other));
                let mut jsep = Llsd::new_map();
                jsep.insert("type", "answer");
                jsep.insert("sdp", sdp);
                let mut resp = Llsd::new_map();
                resp.insert("jsep", jsep);
                resp.insert("viewer_session", "test-viewer-session");
                aurora_llsd::to_xml_string(&resp)
            };
            let _ = write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/llsd+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.len(),
                reply
            );
        }
    });
    format!("http://{addr}/cap/provision")
}

fn is_silence(p: &[u8]) -> bool {
    p == OPUS_SILENCE
}

/// Decodes `packets` with an independent Opus decoder; returns the RMS.
fn decoded_rms(packets: &[Vec<u8>]) -> f32 {
    let mut dec = opus_decoder::OpusDecoder::new(48_000, 2).unwrap();
    let mut pcm = vec![0.0f32; 5760 * 2];
    let (mut sum, mut count) = (0.0f64, 0usize);
    for p in packets {
        let n = dec.decode_float(p, &mut pcm, false).expect("decodable Opus");
        for v in &pcm[..n * 2] {
            sum += f64::from(*v) * f64::from(*v);
        }
        count += n * 2;
    }
    (sum / count.max(1) as f64).sqrt() as f32
}

/// Microphone -> server: silence while muted, real Opus while transmitting.
fn check_transmission(session: &VoiceSession, state: &ServerState) {
    // Before anything is attached the muted track sends silence frames.
    state.rx_packets.lock().clear();
    wait_for("muted frames", Duration::from_secs(5), || state.rx_packets.lock().len() >= 10);
    assert!(state.rx_packets.lock().iter().all(|p| is_silence(p)));

    // A virtual microphone fed in real time with a 440 Hz tone.
    let (mut feed, mic) = MicCapture::virtual_input();
    let feeding = Arc::new(AtomicBool::new(true));
    let feeder = {
        let feeding = feeding.clone();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut pushed = 0usize;
            while feeding.load(std::sync::atomic::Ordering::Relaxed) {
                let due = (start.elapsed().as_secs_f64() * 48_000.0) as usize;
                let chunk: Vec<f32> = (pushed..due)
                    .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin())
                    .collect();
                feed.push(&chunk);
                pushed = due;
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    };
    session.set_mic(Some(mic));
    // Attached but not transmitting: still silence.
    std::thread::sleep(Duration::from_millis(100));
    state.rx_packets.lock().clear();
    wait_for("muted frames with a mic", Duration::from_secs(5), || {
        state.rx_packets.lock().len() >= 10
    });
    assert!(state.rx_packets.lock().iter().all(|p| is_silence(p)));
    assert_eq!(session.own_level(), 0.0);

    // Push-to-talk pressed: decodable, non-silent Opus.
    session.set_transmit(true);
    assert!(session.is_transmitting());
    std::thread::sleep(Duration::from_millis(100));
    state.rx_packets.lock().clear();
    wait_for("voice packets", Duration::from_secs(10), || state.rx_packets.lock().len() >= 25);
    let voice: Vec<Vec<u8>> = state.rx_packets.lock().clone();
    assert!(voice.iter().all(|p| !is_silence(p)), "silence while transmitting");
    let rms = decoded_rms(&voice[..25]);
    assert!(rms > 0.2, "decoded RMS {rms}");
    assert!(session.own_level() > crate::SPEAKING_AUDIO_LEVEL);
    assert!(session.own_speaking());

    // Released: back to silence frames.
    session.set_transmit(false);
    std::thread::sleep(Duration::from_millis(100));
    state.rx_packets.lock().clear();
    wait_for("muted frames after release", Duration::from_secs(5), || {
        state.rx_packets.lock().len() >= 10
    });
    assert!(state.rx_packets.lock().iter().all(|p| is_silence(p)));
    assert_eq!(session.own_level(), 0.0);

    session.set_mic(None);
    feeding.store(false, std::sync::atomic::Ordering::Relaxed);
    let _ = feeder.join();
}

fn wait_for(what: &str, timeout: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn loopback_webrtc_session() {
    LOOPBACK_ONLY.store(true, std::sync::atomic::Ordering::Relaxed);
    let server_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let state = Arc::new(ServerState::default());
    let me = Uuid::from_u128(0x42);
    let other = Uuid::from_u128(0x43);
    let url = start_fake_server(server_rt.handle().clone(), state.clone(), other);

    let (sink, mut tap) = PcmSink::loopback(48_000);
    let params = VoiceConnectParams {
        provision_cap_url: url,
        signaling_cap_url: None,
        agent_id: me,
        voice_server_type: "webrtc".into(),
        channel: ChannelKind::Local { parcel_local_id: Some(5) },
        grid: "agni".into(),
        stun_servers: Some(vec![]),
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let session = VoiceSession::connect(http, params, sink);
    session.set_position(
        DVec3::new(256_100.0, 256_200.0, 22.0),
        Quat::IDENTITY,
        DVec3::new(256_101.0, 256_200.0, 24.0),
        Quat::IDENTITY,
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while session.status() != crate::VoiceStatus::Connected {
        assert!(
            Instant::now() < deadline,
            "not connected: {:?}; provision requests: {}; data messages: {:?}",
            session.status(),
            state.provision_bodies.lock().len(),
            state.data_messages.lock()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    wait_for("participant", Duration::from_secs(10), || {
        session.participants().iter().any(|p| p.agent_id == other && p.speaking)
    });
    wait_for("decoded audio", Duration::from_secs(10), || tap.available() >= 960 * 2);
    let mut pcm = vec![0.0f32; 960 * 2];
    assert_eq!(tap.read(&mut pcm), 960 * 2);
    assert!(pcm.iter().all(|v| v.abs() < 0.01), "silence expected");

    {
        let bodies = state.provision_bodies.lock();
        let body = &bodies[0];
        assert_eq!(body.get("channel_type").as_str(), "local");
        assert_eq!(body.get("parcel_local_id"), &Llsd::Integer(5));
        assert_eq!(body.get("voice_server_type").as_str(), "webrtc");
        assert!(body.get("jsep").get("sdp").as_str().contains(protocol::OPUS_FMTP));
    }
    wait_for("position update", Duration::from_secs(5), || {
        state.data_messages.lock().iter().any(|m| m.starts_with(r#"{"sp":"#))
    });
    {
        let msgs = state.data_messages.lock();
        assert_eq!(msgs[0], r#"{"j":{"p":true}}"#);
        let pos = msgs.iter().find(|m| m.starts_with(r#"{"sp":"#)).unwrap();
        assert!(pos.contains(r#""sp":{"x":25610000,"y":25620000,"z":2300}"#), "{pos}");
    }

    // Mute / volume are forwarded on the data channel.
    session.set_user_mute(other, true);
    wait_for("mute message", Duration::from_secs(5), || {
        state.data_messages.lock().iter().any(|m| m.starts_with(r#"{"m":"#))
    });

    check_transmission(&session, &state);

    drop(session);
    wait_for("logout", Duration::from_secs(10), || !state.logouts.lock().is_empty());
    let logout = state.logouts.lock()[0].clone();
    assert_eq!(logout.get("viewer_session").as_str(), "test-viewer-session");
    assert_eq!(logout.get("logout"), &Llsd::Boolean(true));
    LOOPBACK_ONLY.store(false, std::sync::atomic::Ordering::Relaxed);
    let peers: Vec<_> = state.peers.lock().drain(..).collect();
    for pc in peers {
        let _ = server_rt.block_on(pc.close());
    }
}
