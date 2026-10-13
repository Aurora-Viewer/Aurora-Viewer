//! Voice chat glue: one WebRTC session for the agent's region / parcel
//! channel (LLVoiceWebRTC spatial channel), the microphone capture and
//! push-to-talk. Vivox regions are not supported (proprietary daemon).

use aurora_audio::{AudioEngine, MicCapture, OVERDRIVEN_POWER_LEVEL};
use aurora_net::parcel_flags;
use aurora_voice::{ChannelKind, VoiceConnectParams, VoiceSession, VoiceStatus};
use glam::{DVec3, Quat};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Channel the session is joined to.
#[derive(Debug, Clone, PartialEq)]
struct ChannelKey {
    region: u64,
    /// None = estate (region-wide) channel.
    parcel: Option<i32>,
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
    pub transmit: bool,
    /// Global positions (metres) and rotations of the avatar and the camera.
    pub avatar: (DVec3, Quat),
    pub listener: (DVec3, Quat),
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
}

impl Voice {
    /// The channel wanted now, or why there is none.
    fn wanted(v: &VoiceInputs) -> Result<(ChannelKey, String, Option<String>), String> {
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
        let parcel = v.parcel.ok_or_else(|| aurora_i18n::tr!("voice-waiting-for-parcel").to_owned())?;
        if parcel.flags & parcel_flags::ALLOW_VOICE_CHAT == 0 {
            return Err(aurora_i18n::tr!("voice-voice-disabled-on-this-parcel").into());
        }
        let parcel_id = (parcel.flags & parcel_flags::USE_ESTATE_VOICE_CHAN == 0).then_some(parcel.local_id);
        let signaling = caps.get("VoiceSignalingRequest").cloned();
        Ok((ChannelKey { region, parcel: parcel_id }, provision, signaling))
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
            self.mic_level = m.level();
            self.mic_meter = (m.level() / OVERDRIVEN_POWER_LEVEL).clamp(0.0, 1.0);
        } else {
            self.mic_meter = 0.0;
            self.mic_level = 0.0;
        }

        // session for the current channel
        match Self::wanted(&v) {
            Ok((key, provision, signaling)) => {
                if self.session.as_ref().map(|(k, _)| k) != Some(&key) {
                    // changing channel: leave the old one first (drop logs out)
                    self.session = None;
                    if let Some(engine) = v.engine {
                        log::info!("voice: joining region {:x} parcel {:?}", key.region, key.parcel);
                        let params = VoiceConnectParams {
                            provision_cap_url: provision,
                            signaling_cap_url: signaling,
                            agent_id: v.agent_id,
                            voice_server_type: v.voice_server.to_owned(),
                            channel: ChannelKind::Local {
                                parcel_local_id: key.parcel,
                            },
                            grid: v.grid.to_owned(),
                            stun_servers: None,
                        };
                        let s = VoiceSession::connect(v.http.clone(), params, engine.voice_sink());
                        s.set_mic(self.mic.clone());
                        self.session = Some((key, s));
                        self.muted.clear();
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
        s.set_position(v.avatar.0, v.avatar.1, v.listener.0, v.listener.1);
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

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

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
