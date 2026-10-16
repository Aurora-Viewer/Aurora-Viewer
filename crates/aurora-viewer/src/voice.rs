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
    pub caps: Option<&'a HashMap<String, String>>,
    /// SimulatorFeatures VoiceServerType ("" until known).
    pub voice_server: &'a str,
    pub parcel: Option<&'a aurora_net::ParcelInfo>,
    pub input_device: &'a str,
    pub mic_gain: f32,
    pub processing: aurora_audio::MicProcessing,
    pub ear: crate::settings::VoiceEar,
    pub transmit: bool,
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
    mic: Option<MicCapture>,
    mic_device: String,
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
}

impl Voice {
    /// Drop capture and the transport; update reconnects to the wanted channel.
    pub fn restart(&mut self) {
        self.session = None;
        self.mic = None;
        self.muted.clear();
        self.volumes.clear();
        self.connected = false;
        self.own_level = 0.0;
        self.status.clear();
    }
    /// The channel wanted now, its kind for the server, or why there is none.
    fn wanted(v: &VoiceInputs) -> Result<(ChannelKey, ChannelKind, String, Option<String>), String> {
        if !v.enabled {
            return Err(aurora_i18n::tr!("common-voice-disabled").into());
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
        // a call does not depend on the parcel's voice settings
        if let Some(call) = v.call {
            let kind = ChannelKind::Multiagent {
                channel_id: call.id.clone(),
                credentials: call.credentials.clone(),
            };
            return Ok((ChannelKey::Call(call.id.clone()), kind, provision, signaling));
        }
        let parcel = v.parcel.ok_or_else(|| aurora_i18n::tr!("voice-waiting-for-parcel").to_owned())?;
        if parcel.flags & parcel_flags::ALLOW_VOICE_CHAT == 0 {
            return Err(aurora_i18n::tr!("voice-voice-disabled-on-this-parcel").into());
        }
        let parcel_id = (parcel.flags & parcel_flags::USE_ESTATE_VOICE_CHAN == 0).then_some(parcel.local_id);
        Ok((
            ChannelKey::Local { region, parcel: parcel_id },
            ChannelKind::Local {
                parcel_local_id: parcel_id,
            },
            provision,
            signaling,
        ))
    }

    pub fn update(&mut self, v: VoiceInputs) {
        // microphone: open while voice is on in world, or for the settings meter
        let want_mic = v.enabled && (v.in_world || v.tuning);
        if want_mic && self.mic.is_none() {
            match MicCapture::open(non_empty(v.input_device)) {
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
            if let Some((_, s)) = &self.session {
                s.set_mic(None);
            }
        }
        if let Some(m) = &self.mic {
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
        match Self::wanted(&v) {
            Ok((key, channel, provision, signaling)) => {
                if self.session.as_ref().map(|(k, _)| k) != Some(&key) {
                    // changing channel: leave the old one first (drop logs out)
                    self.session = None;
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
                    }
                }
            }
            Err(why) => {
                if self.session.take().is_some() {
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
    pub fn apply_volumes(&mut self, saved: &std::collections::BTreeMap<Uuid, f32>) {
        let Some((_, s)) = &self.session else {
            return;
        };
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
