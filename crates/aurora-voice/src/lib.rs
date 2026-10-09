//! Second Life voice for Aurora Viewer (WebRTC).
//!
//! Second Life regions announce their voice backend in
//! `SimulatorFeatures["VoiceServerType"]`. For `"webrtc"` regions this crate
//! negotiates a WebRTC peer connection through the region's
//! `ProvisionVoiceAccountRequest` capability (SDP offer/answer), receives the
//! server-mixed Opus audio into an [`aurora_audio::VoiceSink`], tracks the
//! participant list from the "SLData" data channel and sends the listener
//! position on it. The microphone ([`aurora_audio::MicCapture`], attached
//! with [`VoiceSession::set_mic`]) is Opus-encoded and sent while
//! [`VoiceSession::set_transmit`] is on (push-to-talk / toggle are driven by
//! the application); otherwise silence is sent, like a muted track. Vivox
//! regions (proprietary SLVoice daemon) are not supported and report
//! [`VoiceStatus::Unsupported`].
//!
//! The crate takes plain inputs (capability URLs, ids, an HTTP client) and
//! does not depend on the viewer's networking crate.
//!
//! Protocol ported from Firestorm's `indra/newview/llvoicewebrtc.cpp` and
//! `indra/llwebrtc/llwebrtc.cpp`, Copyright (C) Linden Research, Inc. and
//! The Phoenix Firestorm Project, licensed under the GNU originally LGPL 2.1.

pub mod protocol;
mod session;
mod state;
mod transmit;

use std::sync::Arc;
use std::thread;

use aurora_audio::{MicCapture, VoiceSink};
use glam::{DVec3, Quat};
use tokio::sync::mpsc;
use uuid::Uuid;

pub use protocol::{ChannelKind, is_webrtc_voice_server};
pub use state::Participant;

use session::{Cmd, Shared};

/// Connection state of a voice session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceStatus {
    /// Negotiating with the voice server.
    Connecting,
    /// Audio is flowing and the session is joined.
    Connected,
    /// The region / channel uses an unsupported voice backend (Vivox).
    Unsupported(String),
    /// The channel is full (HTTP 409 from provisioning); retrying.
    ChannelFull,
    /// Not allowed in this channel (HTTP 401 from provisioning); retrying.
    ChannelLocked,
    /// Something failed; the session retries automatically.
    Error(String),
    /// The session was closed.
    Disconnected,
}

/// Everything needed to join a voice channel.
#[derive(Debug, Clone)]
pub struct VoiceConnectParams {
    /// The region's `ProvisionVoiceAccountRequest` capability URL.
    pub provision_cap_url: String,
    /// The region's `VoiceSignalingRequest` capability URL (ICE trickle).
    /// Optional: local candidates are also embedded in the offer SDP.
    pub signaling_cap_url: Option<String>,
    /// Our agent id (never removed from the participant list on "leave").
    pub agent_id: Uuid,
    /// `SimulatorFeatures["VoiceServerType"]` of the region ("" = Vivox).
    pub voice_server_type: String,
    /// Which channel to join.
    pub channel: ChannelKind,
    /// Grid id used to pick STUN servers ("agni" = main grid, "aditi" = beta).
    pub grid: String,
    /// STUN server URLs overriding the grid defaults (e.g. OpenSim's
    /// `SimulatorFeatures` STUN list). `None` = Second Life servers for `grid`.
    pub stun_servers: Option<Vec<String>>,
}

/// A running voice session. Dropping it (or calling
/// [`VoiceSession::disconnect`]) logs out of the channel in the background.
pub struct VoiceSession {
    shared: Arc<Shared>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    agent_id: Uuid,
}

/// Largest accepted [`VoiceSession::set_mic_gain`] value.
pub const MAX_MIC_GAIN: f32 = transmit::MAX_MIC_GAIN;

/// `SPEAKING_AUDIO_LEVEL`: [`VoiceSession::own_level`] above which we count
/// as speaking.
pub const SPEAKING_AUDIO_LEVEL: f32 = transmit::SPEAKING_AUDIO_LEVEL;

impl std::fmt::Debug for VoiceSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceSession").field("status", &self.status()).finish()
    }
}

impl VoiceSession {
    /// Starts connecting in the background (dedicated thread with its own
    /// tokio runtime) and returns immediately. Received voice is pushed into
    /// `sink`. Non-WebRTC regions end up in [`VoiceStatus::Unsupported`]
    /// without any network traffic.
    pub fn connect(http: reqwest::Client, params: VoiceConnectParams, sink: VoiceSink) -> VoiceSession {
        // rustls' process-wide crypto provider is ambiguous when several
        // backends are compiled in (ring via webrtc, aws-lc-rs via reqwest);
        // webrtc's DTLS takes its own provider, the other rustls users do
        // not. Install aws-lc-rs unless the application already chose one.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }
        let shared = Arc::new(Shared::new());
        let agent_id = params.agent_id;
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let thread_shared = shared.clone();
        let spawned = thread::Builder::new().name("aurora-voice".into()).spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("aurora-voice-rt")
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    *thread_shared.status.lock() = VoiceStatus::Error(format!("cannot start voice runtime: {e}"));
                    return;
                }
            };
            rt.block_on(session::run(http, params, sink, thread_shared, cmd_rx));
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
        });
        if let Err(e) = spawned {
            *shared.status.lock() = VoiceStatus::Error(format!("cannot start voice thread: {e}"));
        }
        VoiceSession { shared, cmd_tx, agent_id }
    }

    /// Current connection status.
    pub fn status(&self) -> VoiceStatus {
        self.shared.status.lock().clone()
    }

    /// Participants of the channel, sorted by agent id. Positions are not
    /// part of the voice protocol: use the avatar positions from the world.
    /// Our own entry carries the local [`VoiceSession::own_level`] and
    /// speaking state (`predUpdateOwnVolume`).
    pub fn participants(&self) -> Vec<Participant> {
        let mut list = self.shared.participants.lock().list();
        let level = self.own_level();
        for p in list.iter_mut().filter(|p| p.agent_id == self.agent_id) {
            p.level = level;
            p.speaking = level > SPEAKING_AUDIO_LEVEL;
        }
        list
    }

    /// Attaches the microphone to send (`None` detaches it: silence is sent).
    /// It can be changed at any time, also before the session connects, and
    /// is kept across reconnections. The session reads (drains) the capture
    /// even while not transmitting, so it must be the capture's only reader.
    pub fn set_mic(&self, mic: Option<MicCapture>) {
        *self.shared.tx.mic.lock() = mic;
    }

    /// Starts / stops transmitting the microphone (push-to-talk or toggle,
    /// driven by the application; off by default). When off, the track sends
    /// silence frames exactly like libwebrtc's muted track; switching ramps
    /// the gain over 20 ms to avoid clicks.
    pub fn set_transmit(&self, on: bool) {
        self.shared.tx.set_transmit(on);
    }

    /// Whether the microphone is being transmitted.
    pub fn is_transmitting(&self) -> bool {
        self.shared.tx.transmit()
    }

    /// Microphone gain applied to the sent voice (linear, clamped to
    /// 0..=[`MAX_MIC_GAIN`], default 1): Firestorm's `AudioLevelMic`
    /// (`setMicGain`). It multiplies [`MicCapture::set_gain`].
    pub fn set_mic_gain(&self, gain: f32) {
        self.shared.tx.set_gain(gain);
    }

    /// The microphone gain.
    pub fn mic_gain(&self) -> f32 {
        self.shared.tx.gain()
    }

    /// Our own speaking level, 0..1, smoothed over 300 ms, on Firestorm's
    /// participant scale (about 0.6 for a loud voice); 0 while not
    /// transmitting or not connected. For the "I'm speaking" indicator.
    pub fn own_level(&self) -> f32 {
        self.shared.tx.level()
    }

    /// `true` while [`VoiceSession::own_level`] is above
    /// [`SPEAKING_AUDIO_LEVEL`].
    pub fn own_speaking(&self) -> bool {
        self.own_level() > SPEAKING_AUDIO_LEVEL
    }

    /// Updates the spatial coordinates (global metres; rotations in the
    /// region frame). `avatar_pos` is the avatar's position (1 m is added for
    /// head height, as Firestorm does); `listener_*` is the "ear": the camera,
    /// the avatar, or a mix, according to the user's ear-location setting.
    /// Updates are sent at most every 100 ms and only when they changed
    /// (moves > 10 cm, turns > 2 degrees); the listener is kept within 50 m
    /// of the avatar.
    pub fn set_position(&self, avatar_pos: DVec3, avatar_rot: Quat, listener_pos: DVec3, listener_rot: Quat) {
        self.shared
            .spatial
            .lock()
            .update(avatar_pos, avatar_rot, listener_pos, listener_rot);
    }

    /// Sets a participant's volume (0..=1, 0.5 = default) for this viewer.
    pub fn set_user_volume(&self, id: Uuid, volume: f32) {
        let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { 0.5 };
        self.shared.prefs.lock().volumes.insert(id, volume);
        let _ = self.cmd_tx.send(Cmd::Data(protocol::user_gain_message(id, volume)));
    }

    /// Mutes / unmutes a participant for this viewer (mute list).
    pub fn set_user_mute(&self, id: Uuid, mute: bool) {
        {
            let mut prefs = self.shared.prefs.lock();
            if mute {
                prefs.mutes.insert(id);
            } else {
                prefs.mutes.remove(&id);
            }
        }
        let _ = self.cmd_tx.send(Cmd::Data(protocol::user_mute_message(id, mute)));
    }

    /// Leaves the channel (logout + close happen in the background).
    pub fn disconnect(self) {
        drop(self);
    }
}

impl Drop for VoiceSession {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Cmd::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn params(server_type: &str) -> VoiceConnectParams {
        VoiceConnectParams {
            // Never contacted: Vivox regions stop before any request.
            provision_cap_url: "http://127.0.0.1:9/cap/provision".into(),
            signaling_cap_url: None,
            agent_id: Uuid::from_u128(7),
            voice_server_type: server_type.into(),
            channel: ChannelKind::Local { parcel_local_id: None },
            grid: "agni".into(),
            stun_servers: None,
        }
    }

    #[test]
    fn vivox_regions_are_reported_unsupported() {
        for kind in ["vivox", ""] {
            let s = VoiceSession::connect(reqwest::Client::new(), params(kind), VoiceSink::null());
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let VoiceStatus::Unsupported(msg) = s.status() {
                    assert_eq!(msg, "Vivox regions not supported");
                    break;
                }
                assert!(Instant::now() < deadline, "{:?}", s.status());
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(s.participants().is_empty());
            s.disconnect();
        }
    }

    #[test]
    fn user_prefs_are_recorded() {
        let s = VoiceSession::connect(reqwest::Client::new(), params("vivox"), VoiceSink::null());
        let id = Uuid::from_u128(9);
        s.set_user_volume(id, 2.0);
        s.set_user_mute(id, true);
        {
            let prefs = s.shared.prefs.lock();
            assert_eq!(prefs.volumes.get(&id), Some(&1.0));
            assert!(prefs.mutes.contains(&id));
        }
        s.set_user_mute(id, false);
        assert!(!s.shared.prefs.lock().mutes.contains(&id));
    }

    #[test]
    fn transmit_controls() {
        let s = VoiceSession::connect(reqwest::Client::new(), params("vivox"), VoiceSink::null());
        assert!(!s.is_transmitting());
        assert_eq!(s.mic_gain(), 1.0);
        s.set_mic_gain(3.0);
        assert_eq!(s.mic_gain(), 3.0);
        s.set_mic_gain(100.0);
        assert_eq!(s.mic_gain(), MAX_MIC_GAIN);
        s.set_transmit(true);
        assert!(s.is_transmitting());
        let (_feed, mic) = aurora_audio::MicCapture::virtual_input();
        s.set_mic(Some(mic));
        assert!(s.shared.tx.mic.lock().is_some());
        s.set_mic(None);
        assert_eq!(s.own_level(), 0.0);
        assert!(!s.own_speaking());

        // Our own participant entry reflects the local level.
        let me = Uuid::from_u128(7);
        let msg = format!(r#"{{"{me}":{{"j":{{"p":true}},"p":10}}}}"#);
        let updates = protocol::parse_data_message(&msg).unwrap();
        s.shared
            .participants
            .lock()
            .apply(&updates, &state::LocalPrefs::default(), true, me);
        s.shared.tx.set_level(0.5);
        let p = &s.participants()[0];
        assert_eq!(p.agent_id, me);
        assert_eq!(p.level, 0.5);
        assert!(p.speaking);
    }
}
