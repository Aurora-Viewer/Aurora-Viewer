//! Voice chat glue: one WebRTC session for the agent's region / parcel
//! channel (LLVoiceWebRTC spatial channel) or, during a call, for the call's
//! "multiagent" channel (startAdHocSession); the microphone capture and
//! push-to-talk. Vivox regions are not supported (proprietary daemon).

use crate::world::calls::{CallChannel, CallMember, ChannelStatus};
use aurora_audio::{AudioEngine, MicCapture, OVERDRIVEN_POWER_LEVEL};
use aurora_net::parcel_flags;
use aurora_voice::{ChannelKind, VoiceConnectParams, VoiceSession, VoiceStatus};
use glam::{DVec3, Quat};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Channel the session is joined to.
#[derive(Debug, Clone, PartialEq)]
enum ChannelKey {
    Local {
        region: u64,
        /// None = estate (region-wide) channel.
        parcel: Option<i32>,
    },
    /// A call's channel (its id): the spatial channel is left meanwhile
    /// (LLWebRTCVoiceClient::leaveChannel before startAdHocSession).
    Call(String),
}

/// What the app knows this frame.
pub struct VoiceInputs<'a> {
    pub enabled: bool,
    pub in_world: bool,
    /// Offline demo: microphone only, no voice server.
    pub offline: bool,
    /// Preferences open on "Sound and voice": keep the microphone open for the meter.
    pub tuning: bool,
    pub engine: Option<&'a AudioEngine>,
    pub http: &'a reqwest::Client,
    pub agent_id: Uuid,
    /// Grid id for the STUN servers ("agni" / "aditi").
    pub grid: &'a str,
    pub region: Option<u64>,
    /// Current region permissions, unknown until its handshake arrives.
    pub region_flags: Option<u32>,
    pub caps: Option<&'a HashMap<String, String>>,
    /// SimulatorFeatures VoiceServerType ("" until known).
    pub voice_server: &'a str,
    pub parcel: Option<&'a aurora_net::ParcelInfo>,
    pub input_device: &'a str,
    pub mic_gain: f32,
    pub processing: aurora_audio::MicProcessing,
    pub ear: crate::settings::VoiceEar,
    pub transmit: bool,
    /// Close the microphone while muted (StopMicCaptureWhenMuted; [`MicGate`]).
    pub stop_mic_when_muted: bool,
    /// Global positions (metres) and rotations of the avatar and the camera.
    pub avatar: (DVec3, Quat),
    pub listener: (DVec3, Quat),
    /// The channel of the call in progress, joined instead of the spatial one.
    pub call: Option<&'a CallChannel>,
}

/// Connection light shown in the top bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VoiceLight {
    /// Voice turned off by the user.
    #[default]
    Off,
    Connecting,
    Connected,
    /// Not connected (no voice here, error, offline).
    Down,
}

#[derive(Default)]
pub struct Voice {
    pub light: VoiceLight,
    session: Option<(ChannelKey, VoiceSession)>,
    /// The sessions left that are still logging out: a quit waits for them
    /// too (`close_for_exit`).
    closing: Vec<aurora_voice::Closing>,
    mic: Option<MicCapture>,
    mic_device: String,
    /// Whether the open microphone captures or is held closed.
    mic_gate: MicGate,
    /// Status line for the UI.
    pub status: String,
    pub connected: bool,
    /// Our speaking level (0..1) and the microphone meter (0..1).
    pub own_level: f32,
    pub mic_meter: f32,
    /// Raw microphone level (Firestorm scale, 0..1) while it is open.
    pub mic_level: f32,
    /// Participants muted for us in the current session (block list).
    muted: HashSet<Uuid>,
    /// Volumes sent to the current session, per resident.
    volumes: HashMap<Uuid, f32>,
    /// The viewer's own volume on the received voice given to the current
    /// session; `None` = never set (0.5, heard as sent).
    receive_volume: Option<f32>,
}

impl Voice {
    /// Drop capture and the transport; update reconnects to the wanted channel.
    pub fn restart(&mut self) {
        self.leave();
        self.mic = None;
        self.muted.clear();
        self.volumes.clear();
        self.receive_volume = None;
        self.connected = false;
        self.own_level = 0.0;
        self.status.clear();
    }
    /// Leaves the current session, if any: its thread logs out in the
    /// background (the drop detaches the microphone at once), and is kept
    /// while it does, for a quit in the meantime.
    fn leave(&mut self) -> bool {
        self.closing.retain(|session| !session.is_finished());
        let Some((_, session)) = self.session.take() else { return false };
        self.closing.push(session.disconnect());
        true
    }
    /// The viewer quits: the session closes without its logout, the ones
    /// left moments ago give theirs up, and the microphone stops; the app
    /// waits on the returned handle before the process exits
    /// (aurora_voice::VoiceSession::close_for_exit, ExitWait::with).
    pub fn close_for_exit(&mut self) -> aurora_voice::ExitWait {
        self.mic = None;
        self.connected = false;
        let current = self.session.take().map(|(_, session)| session.close_for_exit());
        current.unwrap_or_default().with(self.closing.drain(..))
    }
    /// A channel to join, None to keep the current call, or why to leave.
    fn wanted(v: &VoiceInputs, current: Option<&ChannelKey>) -> Result<Option<(ChannelKey, ChannelKind, String, Option<String>)>, String> {
        if !v.enabled {
            return Err(aurora_i18n::tr!("common-voice-disabled").into());
        }
        // Voice (WebRTC) cannot go through the proxy of Preferences › Network
        // › Proxy and shows the real address to the voice servers and the
        // other speakers: off while a proxy is on, unless the user allowed
        // direct voice after its warning. Stricter than the Legacy Viewer,
        // whose voice always connects directly (llvoicewebrtc.cpp has no
        // proxy).
        if !aurora_proxy::rule().voice_allowed() {
            return Err(aurora_i18n::tr!("voice-off-proxy").into());
        }
        if !v.in_world {
            return Err(String::new());
        }
        if v.offline {
            return Err(aurora_i18n::tr!("voice-demo-mode-no-voice-server-the-microphone-drives-the").into());
        }
        if v.engine.is_none() {
            return Err(aurora_i18n::tr!("voice-no-audio-output").into());
        }
        Self::connection(v, current)
    }

    /// Region data is needed to join, not to maintain a non-spatial call.
    fn connection(
        v: &VoiceInputs,
        current: Option<&ChannelKey>,
    ) -> Result<Option<(ChannelKey, ChannelKind, String, Option<String>)>, String> {
        // LLWebRTCVoiceClient::voiceConnectionCoro (indra/newview/llvoicewebrtc.cpp)
        // changes regional channels only in spatial voice. During a teleport,
        // region/caps/backend can disappear before the destination is ready;
        // leaving here logs out of the independent call on both ends.
        if let Some(ChannelKey::Call(id)) = current
            && v.call.is_some_and(|call| call.id == *id)
        {
            return Ok(None);
        }
        let region = v.region.ok_or_else(String::new)?;
        let caps = v.caps.ok_or_else(|| aurora_i18n::tr!("voice-waiting-for-region").to_owned())?;
        let provision = caps
            .get("ProvisionVoiceAccountRequest")
            .cloned()
            .ok_or_else(|| aurora_i18n::tr!("voice-voice-unavailable-in-this-region").to_owned())?;
        if v.voice_server.is_empty() {
            return Err(aurora_i18n::tr!("voice-waiting-for-region").into());
        }
        if !aurora_voice::is_webrtc_voice_server(v.voice_server) {
            return Err(aurora_i18n::tr!("voice-vivox-region-voice-is-not-supported").into());
        }
        let signaling = caps.get("VoiceSignalingRequest").cloned();
        let (key, channel) = Self::channel(region, v.region_flags, v.parcel, v.call)?;
        Ok(Some((key, channel, provision, signaling)))
    }

    /// LLWebRTCVoiceClient::voiceConnectionCoro (indra/newview/llvoicewebrtc.cpp):
    /// spatial voice needs both region and parcel permission; calls are independent.
    fn channel(
        region: u64,
        region_flags: Option<u32>,
        parcel: Option<&aurora_net::ParcelInfo>,
        call: Option<&CallChannel>,
    ) -> Result<(ChannelKey, ChannelKind), String> {
        if let Some(call) = call {
            let kind = ChannelKind::Multiagent {
                channel_id: call.id.clone(),
                credentials: call.credentials.clone(),
            };
            return Ok((ChannelKey::Call(call.id.clone()), kind));
        }
        if let Some(why) = spatial_voice_denied(region_flags, parcel) {
            return Err(why);
        }
        region_flags.ok_or_else(|| aurora_i18n::tr!("voice-waiting-for-region").to_owned())?;
        let parcel = parcel.ok_or_else(|| aurora_i18n::tr!("voice-waiting-for-parcel").to_owned())?;
        let parcel_id = (parcel.flags & parcel_flags::USE_ESTATE_VOICE_CHAN == 0).then_some(parcel.local_id);
        Ok((
            ChannelKey::Local { region, parcel: parcel_id },
            ChannelKind::Local {
                parcel_local_id: parcel_id,
            },
        ))
    }

    /// Apply a native-window microphone override without waiting for a frame.
    pub fn set_transmit(&self, transmit: bool) {
        if let Some((_, session)) = &self.session {
            session.set_transmit(transmit && self.mic.is_some());
        }
        // a paused microphone reopens now, not at the next frame ([`MicGate`])
        if transmit
            && self.connected
            && let Some(m) = &self.mic
        {
            m.set_paused(false);
        }
    }

    pub fn update(&mut self, v: VoiceInputs) {
        // microphone: open while voice is on in world, or for the settings meter
        let want_mic = v.enabled && (v.in_world || v.tuning);
        // with "Better in-world audio", the device itself is closed while
        // muted; this frame's connection state is last frame's
        let capturing = self.mic_gate.update(
            Instant::now(),
            capture_wanted(v.stop_mic_when_muted, v.tuning, v.transmit, self.connected || v.offline),
        );
        if want_mic && self.mic.is_none() {
            match MicCapture::open_with(non_empty(v.input_device), !capturing) {
                Ok(m) => {
                    self.mic_device = v.input_device.to_owned();
                    self.mic = Some(m);
                }
                Err(e) => {
                    // keep listening without a microphone; retry when the device changes
                    if self.mic_device != v.input_device || self.status.is_empty() {
                        log::warn!("microphone unavailable: {e}");
                    }
                    self.mic_device = v.input_device.to_owned();
                }
            }
        } else if !want_mic && self.mic.is_some() {
            // closing it removes Windows' "microphone in use" indicator
            self.mic = None;
            self.mic_gate = MicGate::default();
            if let Some((_, s)) = &self.session {
                s.set_mic(None);
            }
        }
        if let Some(m) = &self.mic {
            m.set_paused(!capturing);
            if self.mic_device != v.input_device {
                m.set_device(non_empty(v.input_device));
                self.mic_device = v.input_device.to_owned();
            }
            // one gain only (they multiply): on the capture, so the meter shows it
            m.set_gain(v.mic_gain);
            m.set_processing(v.processing, v.engine.map(AudioEngine::echo_reference));
            self.mic_level = m.level();
            self.mic_meter = (m.level() / OVERDRIVEN_POWER_LEVEL).clamp(0.0, 1.0);
        } else {
            self.mic_meter = 0.0;
            self.mic_level = 0.0;
        }

        // session for the current channel
        match Self::wanted(&v, self.session.as_ref().map(|(key, _)| key)) {
            Ok(Some((key, channel, provision, signaling))) => {
                if self.session.as_ref().map(|(k, _)| k) != Some(&key) {
                    // changing channel: leave the old one first (the drop
                    // detaches the microphone at once, then logs out)
                    self.leave();
                    if let Some(engine) = v.engine {
                        match &key {
                            ChannelKey::Local { region, parcel } => log::info!("voice: joining region {region:x} parcel {parcel:?}"),
                            ChannelKey::Call(_) => log::info!("voice: joining the call's channel"),
                        }
                        let params = VoiceConnectParams {
                            provision_cap_url: provision,
                            signaling_cap_url: signaling,
                            agent_id: v.agent_id,
                            voice_server_type: v.voice_server.to_owned(),
                            channel,
                            grid: v.grid.to_owned(),
                            stun_servers: None,
                        };
                        let s = VoiceSession::connect(v.http.clone(), params, engine.voice_sink());
                        s.set_mic(self.mic.clone());
                        self.session = Some((key, s));
                        self.muted.clear();
                        self.volumes.clear();
                        self.receive_volume = None;
                    }
                }
            }
            Ok(None) => {}
            Err(why) => {
                if self.leave() {
                    log::info!("voice: leaving ({why})");
                }
                self.light = if v.enabled { VoiceLight::Down } else { VoiceLight::Off };
                self.status = if why.is_empty() {
                    aurora_i18n::tr!("common-offline").into()
                } else {
                    why
                };
                self.connected = false;
                self.own_level = 0.0;
                return;
            }
        }
        let Some((_, s)) = &self.session else {
            return;
        };
        s.set_transmit(v.transmit && self.mic.is_some());
        let (avatar, listener) = listening_positions(v.avatar, v.listener, v.ear);
        s.set_position(avatar.0, avatar.1, listener.0, listener.1);
        let status = s.status();
        self.connected = matches!(status, VoiceStatus::Connected);
        self.light = match status {
            VoiceStatus::Connecting => VoiceLight::Connecting,
            VoiceStatus::Connected => VoiceLight::Connected,
            _ => VoiceLight::Down,
        };
        self.own_level = s.own_level();
        self.status = match status {
            VoiceStatus::Connecting => aurora_i18n::tr!("common-connecting-to-voice").into(),
            VoiceStatus::Connected => aurora_i18n::tr!("common-voice-connected").into(),
            VoiceStatus::Unsupported(m) => aurora_i18n::tr!("voice-voice-not-supported", m = m.to_string()),
            VoiceStatus::ChannelFull => aurora_i18n::tr!("voice-voice-channel-full").into(),
            VoiceStatus::ChannelLocked => aurora_i18n::tr!("voice-voice-channel-locked").into(),
            VoiceStatus::Error(e) => aurora_i18n::tr!("voice-voice-error", e = e.to_string()),
            VoiceStatus::Disconnected => aurora_i18n::tr!("voice-voice-disconnected").into(),
        };
    }

    /// The moderator mute of each participant of the nearby channel
    /// (LLWebRTCVoiceClient::getIsModeratorMuted), us included; None off
    /// the nearby channel or before it connects.
    pub fn nearby_moderation(&self) -> Option<HashMap<Uuid, bool>> {
        let (ChannelKey::Local { .. }, s) = self.session.as_ref()? else {
            return None;
        };
        self.connected
            .then(|| s.participants().into_iter().map(|p| (p.agent_id, p.moderator_muted)).collect())
    }

    /// Mute the participants we blocked (voice), unmute the others
    /// (LLVoiceClient::muteListChanged); only changes are sent.
    pub fn apply_blocks(&mut self, blocked: impl Fn(&Uuid) -> bool) {
        let Some((_, s)) = &self.session else {
            return;
        };
        if !self.connected {
            return;
        }
        for p in s.participants() {
            let want = blocked(&p.agent_id);
            if want != self.muted.contains(&p.agent_id) {
                s.set_user_mute(p.agent_id, want);
                if want {
                    self.muted.insert(p.agent_id);
                } else {
                    self.muted.remove(&p.agent_id);
                }
            }
        }
    }

    /// Each resident's volume (0..1, 0.5 as sent) for the participants of
    /// the session (LLSpeakerVolumeStorage, setUserVolume); only changes are
    /// sent, a resident without a saved volume is heard at 0.5.
    ///
    /// `p2p_peer` is the other resident of the private call in progress. The
    /// server ignores `setUserVolume` on a private call's channel, in the
    /// Legacy Viewer too (issue #2874), so the viewer also scales what it
    /// receives there, which is that resident's voice alone: better than the
    /// Legacy Viewer, where the slider does nothing. The request is still
    /// sent as the Legacy Viewer sends it.
    pub fn apply_volumes(&mut self, saved: &std::collections::BTreeMap<Uuid, f32>, p2p_peer: Option<Uuid>) {
        let Some((key, s)) = &self.session else {
            return;
        };
        let receive = receive_volume(key, p2p_peer, saved);
        if (receive - self.receive_volume.unwrap_or(0.5)).abs() > f32::EPSILON {
            s.set_receive_volume(receive);
            self.receive_volume = Some(receive);
        }
        if !self.connected {
            return;
        }
        for p in s.participants() {
            let want = saved.get(&p.agent_id).copied().unwrap_or(0.5);
            let sent = self.volumes.get(&p.agent_id).copied().unwrap_or(0.5);
            if (want - sent).abs() > f32::EPSILON {
                s.set_user_volume(p.agent_id, want);
                self.volumes.insert(p.agent_id, want);
            }
        }
    }

    /// The call's channel this frame: its state and participants (us
    /// included), `None` while the session is not on a call's channel.
    pub fn call_report(&self) -> Option<(ChannelStatus, Vec<CallMember>)> {
        let (ChannelKey::Call(_), s) = self.session.as_ref()? else {
            return None;
        };
        let status = match s.status() {
            VoiceStatus::Connecting | VoiceStatus::Error(_) | VoiceStatus::Disconnected => ChannelStatus::Connecting,
            VoiceStatus::Connected => ChannelStatus::Connected,
            VoiceStatus::ChannelFull | VoiceStatus::ChannelLocked => ChannelStatus::Full,
            VoiceStatus::Unsupported(_) => ChannelStatus::Failed,
        };
        let members = s
            .participants()
            .into_iter()
            .map(|p| CallMember {
                id: p.agent_id,
                level: p.level,
                speaking: p.speaking,
                muted: p.moderator_muted,
            })
            .collect();
        Some((status, members))
    }

    /// Voice participants: level (0..1) and speaking, including our local mic.
    pub fn levels(&self, agent_id: Uuid) -> HashMap<Uuid, (f32, bool)> {
        match &self.session {
            Some((_, s)) if self.connected => tag_levels(s.participants(), agent_id, s.own_level()),
            _ => HashMap::new(),
        }
    }
}

/// The viewer's own volume on what `key`'s channel sends: the private
/// call's other resident's saved volume on that call's channel, 0.5 (as
/// sent) on a channel that mixes several voices (nearby, conferences).
fn receive_volume(key: &ChannelKey, p2p_peer: Option<Uuid>, saved: &std::collections::BTreeMap<Uuid, f32>) -> f32 {
    match (key, p2p_peer) {
        (ChannelKey::Call(_), Some(peer)) => saved.get(&peer).copied().unwrap_or(0.5),
        _ => 0.5,
    }
}

/// Why the region or the parcel forbids nearby voice, `None` when it allows
/// it or is not known yet (LLViewerParcelMgr::allowAgentVoice,
/// indra/newview/llviewerparcelmgr.cpp, originally LGPL 2.1).
pub fn spatial_voice_denied(region_flags: Option<u32>, parcel: Option<&aurora_net::ParcelInfo>) -> Option<String> {
    if region_flags? & aurora_net::region_flags::ALLOW_VOICE == 0 {
        return Some(aurora_i18n::tr!("voice-voice-disabled-in-this-region").into());
    }
    (parcel?.flags & parcel_flags::ALLOW_VOICE_CHAT == 0).then(|| aurora_i18n::tr!("voice-voice-disabled-on-this-parcel").into())
}

fn tag_levels(participants: Vec<aurora_voice::Participant>, agent_id: Uuid, own_level: f32) -> HashMap<Uuid, (f32, bool)> {
    let mut levels: HashMap<_, _> = participants.into_iter().map(|p| (p.agent_id, (p.level, p.speaking))).collect();
    // Servers need not echo our own join. Local capture is authoritative,
    // as in LLWebRTCVoiceClient::predUpdateOwnVolume (originally LGPL 2.1).
    levels.insert(agent_id, (own_level, own_level > aurora_voice::SPEAKING_AUDIO_LEVEL));
    levels
}

/// LLVoiceWebRTC spatial ears: avatar head offset and a 50 m camera tether.
fn listening_positions(mut avatar: (DVec3, Quat), camera: (DVec3, Quat), ear: crate::settings::VoiceEar) -> ((DVec3, Quat), (DVec3, Quat)) {
    avatar.0.z += 1.0;
    let listener = match ear {
        crate::settings::VoiceEar::Avatar => avatar,
        crate::settings::VoiceEar::Camera => (avatar.0 + (camera.0 - avatar.0).clamp_length_max(50.0), camera.1),
    };
    (avatar, listener)
}

/// `MUTE_FADE_DELAY_MS` (indra/newview/llvoicewebrtc.cpp): with
/// StopMicCaptureWhenMuted, the delay between muting and the end of the
/// capture, so that the 20 ms fade and the silence after it are sent first.
const MIC_STOP_DELAY: Duration = Duration::from_millis(500);

/// Whether the microphone should capture this frame. With
/// `stop_when_muted` (StopMicCaptureWhenMuted, the LL viewer 26.5's
/// "Bluetooth only: Better in-world audio", on), only while unmuted with the
/// voice audio up, or for the meter of the preferences (tuning); without
/// it, the whole time voice is on, as before 26.5. Port of
/// LLWebRTCImpl::workerStartRecording and intSetMute
/// (indra/llwebrtc/llwebrtc.cpp, originally LGPL 2.1).
fn capture_wanted(stop_when_muted: bool, tuning: bool, transmit: bool, audio_up: bool) -> bool {
    !stop_when_muted || tuning || (transmit && audio_up)
}

/// Turns [`capture_wanted`] into the microphone's state: it starts at once
/// and stops [`MIC_STOP_DELAY`] after it stopped being wanted. Unmuting
/// within that delay keeps it open: the LL viewer's `mute_sequence` cancels
/// the delayed stop the same way (LLWebRTCImpl::intSetMute). Closed until
/// first wanted, as the LL viewer holds capture off before the first unmute.
#[derive(Debug, Default)]
struct MicGate {
    active: bool,
    idle_since: Option<Instant>,
}

impl MicGate {
    /// Whether the microphone captures at `now`.
    fn update(&mut self, now: Instant, wanted: bool) -> bool {
        if wanted {
            self.active = true;
            self.idle_since = None;
        } else if self.active {
            let since = *self.idle_since.get_or_insert(now);
            if now.duration_since(since) >= MIC_STOP_DELAY {
                self.active = false;
                self.idle_since = None;
            }
        }
        self.active
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

/// Firestorm's LLCoordFrame::getQuaternion (llcoordframe.cpp) uses forward,
/// left, up axes. The inverse render view contains right, up, backward axes.
/// Sending that render rotation directly turns the voice listener sideways.
pub(crate) fn listener_rotation(view: glam::Mat4) -> Quat {
    let world = view.inverse();
    Quat::from_mat3(&glam::Mat3::from_cols(
        -world.z_axis.truncate(),
        -world.x_axis.truncate(),
        world.y_axis.truncate(),
    ))
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_call_takes_its_peer_volume() {
        let (peer, other) = (Uuid::from_u128(2), Uuid::from_u128(3));
        let saved = std::collections::BTreeMap::from([(peer, 0.1), (other, 0.9)]);
        let call = ChannelKey::Call("call".into());
        let nearby = ChannelKey::Local { region: 7, parcel: None };
        assert_eq!(receive_volume(&call, Some(peer), &saved), 0.1);
        // no saved volume: heard as sent
        assert_eq!(receive_volume(&call, Some(Uuid::from_u128(4)), &saved), 0.5);
        // a conference mixes several voices: the server's gain only
        assert_eq!(receive_volume(&call, None, &saved), 0.5);
        // still on nearby voice while the call connects
        assert_eq!(receive_volume(&nearby, Some(peer), &saved), 0.5);
    }

    #[test]
    fn the_microphone_captures_while_unmuted_with_better_in_world_audio() {
        // (stop when muted, tuning, transmit, audio up)
        assert!(capture_wanted(true, false, true, true));
        assert!(!capture_wanted(true, false, false, true), "muted");
        assert!(!capture_wanted(true, false, true, false), "voice audio not up yet");
        assert!(capture_wanted(true, true, false, false), "the preferences' meter");
        // "Better voice": open the whole time voice is on
        assert!(capture_wanted(false, false, false, false));
        assert!(capture_wanted(false, false, true, true));
    }

    #[test]
    fn the_microphone_stops_half_a_second_after_muting_and_starts_at_once() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut gate = MicGate::default();
        assert!(!gate.update(t0, false), "closed until first wanted");
        assert!(gate.update(ms(10), true), "unmuting opens it at once");
        assert!(gate.update(ms(20), false), "muted: still open for the fade");
        assert!(gate.update(ms(519), false));
        assert!(!gate.update(ms(520), false), "closed 500 ms after muting");
        assert!(!gate.update(ms(2000), false));
        assert!(gate.update(ms(2001), true));
    }

    #[test]
    fn unmuting_within_the_delay_cancels_the_stop() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut gate = MicGate::default();
        gate.update(t0, true);
        gate.update(ms(100), false);
        assert!(gate.update(ms(400), true), "unmuted before the stop");
        // the next mute waits its own 500 ms, not what was left of the first
        assert!(gate.update(ms(500), false));
        assert!(gate.update(ms(999), false));
        assert!(!gate.update(ms(1000), false));
    }

    fn inputs(http: &reqwest::Client) -> VoiceInputs<'_> {
        VoiceInputs {
            enabled: true,
            in_world: true,
            offline: false,
            tuning: false,
            engine: None,
            http,
            agent_id: Uuid::from_u128(1),
            grid: "test",
            region: Some(7),
            region_flags: Some(aurora_net::region_flags::ALLOW_VOICE),
            caps: None,
            voice_server: "webrtc",
            parcel: None,
            input_device: "",
            mic_gain: 1.0,
            processing: Default::default(),
            ear: crate::settings::VoiceEar::Avatar,
            transmit: true,
            stop_mic_when_muted: true,
            avatar: (DVec3::ZERO, Quat::IDENTITY),
            listener: (DVec3::ZERO, Quat::IDENTITY),
            call: None,
        }
    }

    #[test]
    fn the_current_call_survives_teleport_region_data_gaps() {
        let http = reqwest::Client::new();
        let call = CallChannel {
            id: "test-call".into(),
            credentials: "test-credentials".into(),
        };
        let current = ChannelKey::Call(call.id.clone());
        let caps = HashMap::from([("ProvisionVoiceAccountRequest".into(), "https://voice.invalid/provision".into())]);
        let empty_caps = HashMap::new();
        let forbidden_parcel = aurora_net::ParcelInfo::default();
        let mut v = inputs(&http);
        v.call = Some(&call);

        // Departure, no main region, arrival before seed caps and simulator
        // features, and a destination with no spatial voice. None means keep
        // the exact transport, rather than leave and provision the call again.
        for (region, caps, backend) in [
            (Some(7), Some(&caps), "webrtc"),
            (None, None, ""),
            (Some(8), None, ""),
            (Some(8), Some(&empty_caps), ""),
            (Some(8), Some(&empty_caps), "webrtc"),
            (Some(8), Some(&caps), ""),
            (Some(8), Some(&caps), "vivox"),
            (Some(8), Some(&caps), "webrtc"),
        ] {
            v.region = region;
            v.caps = caps;
            v.voice_server = backend;
            v.region_flags = Some(0);
            v.parcel = Some(&forbidden_parcel);
            assert!(
                Voice::connection(&v, Some(&current))
                    .expect("the call is independent of the destination")
                    .is_none()
            );
        }
    }

    #[test]
    fn call_hangup_and_channel_changes_still_require_the_destination() {
        let http = reqwest::Client::new();
        let call = CallChannel {
            id: "test-call".into(),
            credentials: "test-credentials".into(),
        };
        let current = ChannelKey::Call(call.id.clone());
        let other_call = ChannelKey::Call("other-call".into());
        let local = ChannelKey::Local {
            region: 7,
            parcel: Some(42),
        };
        let caps = HashMap::from([("ProvisionVoiceAccountRequest".into(), "https://voice.invalid/provision".into())]);
        let parcel = aurora_net::ParcelInfo {
            local_id: 42,
            flags: parcel_flags::ALLOW_VOICE_CHAT,
            ..Default::default()
        };
        let mut v = inputs(&http);
        v.call = Some(&call);
        for current in [None, Some(&other_call), Some(&local)] {
            assert!(Voice::connection(&v, current).is_err(), "a new call still needs provisioning");
        }
        v.caps = Some(&caps);
        let (key, kind, _, _) = Voice::connection(&v, Some(&other_call))
            .expect("the new call can join")
            .expect("join, not keep");
        assert_eq!(key, current);
        assert_eq!(
            kind,
            ChannelKind::Multiagent {
                channel_id: call.id.clone(),
                credentials: call.credentials.clone()
            }
        );

        v.call = None;
        assert!(
            Voice::connection(&v, Some(&current)).is_err(),
            "hangup waits for the destination parcel"
        );
        v.parcel = Some(&parcel);
        v.region = Some(8);
        let (key, kind, _, _) = Voice::connection(&v, Some(&current))
            .expect("nearby voice is ready")
            .expect("leave the call");
        assert_eq!(
            key,
            ChannelKey::Local {
                region: 8,
                parcel: Some(42)
            }
        );
        assert_eq!(kind, ChannelKind::Local { parcel_local_id: Some(42) });
        v.region_flags = Some(0);
        assert!(Voice::connection(&v, Some(&current)).is_err(), "hangup respects land restrictions");
        v.region = None;
        assert!(
            Voice::connection(&v, Some(&local)).is_err(),
            "spatial voice must not stay in the old region"
        );
    }

    #[test]
    fn keeping_a_call_never_bypasses_voice_disabled_logout_or_missing_audio() {
        let http = reqwest::Client::new();
        let call = CallChannel {
            id: "test-call".into(),
            credentials: String::new(),
        };
        let current = ChannelKey::Call(call.id.clone());
        let mut v = inputs(&http);
        v.call = Some(&call);
        v.enabled = false;
        assert_eq!(
            Voice::wanted(&v, Some(&current)).unwrap_err(),
            aurora_i18n::tr!("common-voice-disabled")
        );
        v.enabled = true;
        v.in_world = false;
        assert_eq!(Voice::wanted(&v, Some(&current)).unwrap_err(), "");
        v.in_world = true;
        v.offline = true;
        assert_eq!(
            Voice::wanted(&v, Some(&current)).unwrap_err(),
            aurora_i18n::tr!("voice-demo-mode-no-voice-server-the-microphone-drives-the")
        );
        v.offline = false;
        assert_eq!(
            Voice::wanted(&v, Some(&current)).unwrap_err(),
            aurora_i18n::tr!("voice-no-audio-output")
        );
    }

    #[test]
    fn nearby_voice_requires_both_region_and_parcel_permission() {
        for estate in [false, true] {
            for region_voice in [false, true] {
                for parcel_voice in [false, true] {
                    let parcel = aurora_net::ParcelInfo {
                        local_id: 42,
                        flags: if parcel_voice { parcel_flags::ALLOW_VOICE_CHAT } else { 0 }
                            | if estate { parcel_flags::USE_ESTATE_VOICE_CHAN } else { 0 },
                        ..Default::default()
                    };
                    let flags =
                        aurora_net::region_flags::ALLOW_DAMAGE | if region_voice { aurora_net::region_flags::ALLOW_VOICE } else { 0 };
                    let channel = Voice::channel(7, Some(flags), Some(&parcel), None);
                    if region_voice && parcel_voice {
                        let parcel_id = (!estate).then_some(42);
                        let (key, kind) = channel.expect("region and parcel allow voice");
                        assert_eq!(
                            key,
                            ChannelKey::Local {
                                region: 7,
                                parcel: parcel_id
                            }
                        );
                        assert!(matches!(kind, ChannelKind::Local { parcel_local_id } if parcel_local_id == parcel_id));
                    } else {
                        let expected = if region_voice {
                            aurora_i18n::tr!("voice-voice-disabled-on-this-parcel")
                        } else {
                            aurora_i18n::tr!("voice-voice-disabled-in-this-region")
                        };
                        assert_eq!(channel.err().as_deref(), Some(expected));
                    }
                }
            }
        }
    }

    #[test]
    fn nearby_voice_waits_for_current_region_and_parcel_permissions() {
        let parcel = aurora_net::ParcelInfo {
            flags: parcel_flags::ALLOW_VOICE_CHAT,
            ..Default::default()
        };
        assert_eq!(
            Voice::channel(7, None, Some(&parcel), None).err(),
            Some(aurora_i18n::tr!("voice-waiting-for-region").to_owned())
        );
        assert_eq!(
            Voice::channel(7, Some(aurora_net::region_flags::ALLOW_VOICE), None, None).err(),
            Some(aurora_i18n::tr!("voice-waiting-for-parcel").to_owned())
        );
    }

    #[test]
    fn speaking_is_denied_only_once_the_land_forbids_it() {
        let open = aurora_net::ParcelInfo {
            flags: parcel_flags::ALLOW_VOICE_CHAT,
            ..Default::default()
        };
        let closed = aurora_net::ParcelInfo::default();
        let voice = Some(aurora_net::region_flags::ALLOW_VOICE);
        // not known yet: the microphone is left as it is
        assert_eq!(spatial_voice_denied(None, Some(&closed)), None);
        assert_eq!(spatial_voice_denied(voice, None), None);
        assert_eq!(spatial_voice_denied(voice, Some(&open)), None);
        assert_eq!(
            spatial_voice_denied(voice, Some(&closed)).as_deref(),
            Some(aurora_i18n::tr!("voice-voice-disabled-on-this-parcel"))
        );
        assert_eq!(
            spatial_voice_denied(Some(0), None).as_deref(),
            Some(aurora_i18n::tr!("voice-voice-disabled-in-this-region"))
        );
    }

    #[test]
    fn calls_ignore_spatial_permissions_and_hangup_checks_them_again() {
        let parcel = aurora_net::ParcelInfo::default();
        let call = CallChannel {
            id: "test-call".into(),
            credentials: String::new(),
        };
        for flags in [None, Some(0), Some(aurora_net::region_flags::ALLOW_VOICE)] {
            for parcel in [None, Some(&parcel)] {
                let (key, kind) = Voice::channel(7, flags, parcel, Some(&call)).expect("calls are independent of land");
                assert_eq!(key, ChannelKey::Call(call.id.clone()));
                assert!(matches!(kind, ChannelKind::Multiagent { channel_id, .. } if channel_id == call.id));
                assert!(
                    Voice::channel(7, flags, parcel, None).is_err(),
                    "hangup must not join forbidden nearby voice"
                );
            }
        }
    }

    #[test]
    fn no_moderation_off_the_nearby_channel() {
        assert_eq!(Voice::default().nearby_moderation(), None);
    }

    #[test]
    fn avatar_ears_follow_the_head_and_camera_ears_are_tethered() {
        use crate::settings::VoiceEar;
        let body = (DVec3::new(10.0, 20.0, 30.0), Quat::from_rotation_z(0.6));
        let camera = (DVec3::new(1010.0, 20.0, 31.0), Quat::IDENTITY);
        let (head, ear) = listening_positions(body, camera, VoiceEar::Avatar);
        assert_eq!(head.0, DVec3::new(10.0, 20.0, 31.0));
        assert_eq!(ear, head);
        let (_, ear) = listening_positions(body, camera, VoiceEar::Camera);
        assert!((ear.0.distance(head.0) - 50.0).abs() < 1e-8);
        assert_eq!(ear.1, camera.1);
    }

    #[test]
    fn voice_preferences_keep_defaults_and_survive_round_trip() {
        use crate::settings::{AudioSettings, VoiceEar};
        let mut a: AudioSettings = serde_json::from_str("{\"mic_gain\":0.75}").expect("old preferences");
        assert_eq!(a.voice_ear, VoiceEar::Camera);
        assert_eq!(a.media_sounds_ear, VoiceEar::Camera, "MediaSoundsEarLocation 0");
        assert_eq!(a.mic_processing(), aurora_audio::MicProcessing::default());
        assert!(
            a.lip_sync
                && a.voice_visualizer
                && a.voice_visualizer_self
                && a.voice_visualizer_dot
                && a.voice_visualizers_in_calls
                && a.conversation_voice_indicator
        );
        a.voice_ear = VoiceEar::Avatar;
        a.media_sounds_ear = VoiceEar::Avatar;
        a.voice_noise_suppression = 2;
        a.voice_echo_cancellation = false;
        a.lip_sync = false;
        a.voice_visualizer_self = false;
        a.conversation_voice_indicator = false;
        let restored: AudioSettings = serde_json::from_str(&serde_json::to_string(&a).expect("encode")).expect("decode");
        assert_eq!(restored, a);
        assert!(!a.world_voice_visualizer(true, false, false));
        assert!(a.world_voice_visualizer(false, true, false));
        a.voice_visualizers_in_calls = false;
        assert!(!a.world_voice_visualizer(false, true, false));
        assert!(a.world_voice_visualizer(false, false, false));
        a.voice_visualizer = false;
        assert!(!a.world_voice_visualizer(false, false, false));
    }

    #[test]
    fn voice_listener_uses_forward_left_up_axes() {
        use glam::Vec3;

        for forward in [
            Vec3::X,
            Vec3::Y,
            -Vec3::X,
            -Vec3::Y,
            Vec3::new(1.0, 2.0, 3.0).normalize(),
            Vec3::Z,
            -Vec3::Z,
        ] {
            let up = if forward.cross(Vec3::Z).length_squared() < 1e-6 {
                Vec3::Y
            } else {
                Vec3::Z
            };
            let view = glam::camera::rh::view::look_to_mat4(Vec3::new(8.0, 9.0, 10.0), forward, up);
            let rotation = listener_rotation(view);
            let left = up.cross(forward).normalize();
            let above = forward.cross(left);
            assert!((rotation * Vec3::X).abs_diff_eq(forward, 1e-5));
            assert!((rotation * Vec3::Y).abs_diff_eq(left, 1e-5));
            assert!((rotation * Vec3::Z).abs_diff_eq(above, 1e-5));
        }
    }

    #[test]
    fn own_tag_uses_local_mic_even_without_a_server_entry() {
        let me = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let levels = tag_levels(
            vec![aurora_voice::Participant {
                agent_id: other,
                level: 0.4,
                speaking: true,
                moderator_muted: false,
            }],
            me,
            0.5,
        );
        assert_eq!(levels[&me], (0.5, true));
        assert_eq!(levels[&other], (0.4, true));
        assert!(!levels.contains_key(&Uuid::from_u128(3)));
    }

    #[test]
    fn closed_local_mic_overrides_stale_server_speech() {
        let me = Uuid::from_u128(1);
        let levels = tag_levels(
            vec![aurora_voice::Participant {
                agent_id: me,
                level: 0.8,
                speaking: true,
                moderator_muted: false,
            }],
            me,
            0.0,
        );
        assert_eq!(levels[&me], (0.0, false));
        assert!(Voice::default().levels(me).is_empty());
    }
}
