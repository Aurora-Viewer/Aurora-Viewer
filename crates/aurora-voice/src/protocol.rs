//! Wire formats of Second Life WebRTC voice: the `ProvisionVoiceAccountRequest`
//! / `VoiceSignalingRequest` capability bodies (LLSD), SDP munging, and the
//! JSON messages carried on the "SLData" data channel.
//!
//! Ported from Firestorm's `indra/newview/llvoicewebrtc.cpp` and
//! `indra/llwebrtc/llwebrtc.cpp`.
//! Copyright (C) Linden Research, Inc. and The Phoenix Firestorm Project.
//! Licensed under the GNU Lesser General Public License, version 2.1.

use std::fmt::Write as _;

use aurora_llsd::Llsd;
use glam::{DVec3, Quat};
use uuid::Uuid;

/// `voice_server_type` value of WebRTC regions/channels.
pub const WEBRTC_VOICE_SERVER_TYPE: &str = "webrtc";
/// `voice_server_type` value of Vivox regions/channels.
pub const VIVOX_VOICE_SERVER_TYPE: &str = "vivox";

/// Label of the data channel created by the viewer.
pub(crate) const DATA_CHANNEL_LABEL: &str = "SLData";
/// Media stream / track ids used by llwebrtc.
pub(crate) const STREAM_ID: &str = "SLStream";
pub(crate) const AUDIO_TRACK_ID: &str = "SLAudio";

/// Opus fmtp parameters forced into the offer (llwebrtc `OnSuccess`).
pub(crate) const OPUS_FMTP: &str =
    "minptime=10;useinbandfec=1;stereo=1;sprop-stereo=1;maxplaybackrate=48000;sprop-maxplaybackrate=48000;sprop-maxcapturerate=48000";

/// `PEER_GAIN_CONVERSION_FACTOR` (llvoicewebrtc.cpp), used by `setUserVolume`.
pub(crate) const PEER_GAIN_CONVERSION_FACTOR: f32 = 220.0;
/// Factor used when re-applying a stored speaker volume on join
/// (`user_gain[participant_id] = (uint32_t)(volume * 200)` in
/// `OnDataReceivedImpl`). Firestorm uses 200 there and 220 in
/// `setUserVolume`; both are kept for parity.
pub(crate) const JOIN_GAIN_CONVERSION_FACTOR: f32 = 200.0;

/// `MAX_AUDIO_DIST`: the listener is tethered within 50 m of the avatar.
pub(crate) const MAX_AUDIO_DIST: f64 = 50.0;

/// Which voice channel to join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelKind {
    /// Spatial (region) voice. `parcel_local_id` is `Some` for a parcel with
    /// its own voice channel ("use estate voice channel" unchecked), `None`
    /// for the estate channel.
    Local { parcel_local_id: Option<i32> },
    /// Ad-hoc / group / P2P call ("multiagent" channel) with the channel id
    /// and credentials received from the simulator.
    Multiagent { channel_id: String, credentials: String },
}

impl ChannelKind {
    pub(crate) fn is_spatial(&self) -> bool {
        matches!(self, ChannelKind::Local { .. })
    }
}

/// `true` when the region's `SimulatorFeatures["VoiceServerType"]` selects
/// WebRTC. An empty / missing value means Vivox (llvoiceclient.cpp).
pub fn is_webrtc_voice_server(voice_server_type: &str) -> bool {
    voice_server_type.trim().eq_ignore_ascii_case(WEBRTC_VOICE_SERVER_TYPE)
}

/// STUN servers used by Firestorm (`getConnectionOptions`): two per grid,
/// three on the main grid ("agni").
pub fn stun_servers_for_grid(grid: &str) -> Vec<String> {
    let grid = grid.trim().to_ascii_lowercase();
    let grid = if grid.is_empty() { "agni".to_string() } else { grid };
    let count = if grid == "agni" { 3 } else { 2 };
    (1..=count).map(|i| format!("stun:stun{i}.{grid}.secondlife.io:3478")).collect()
}

/// Rewrites the local offer like llwebrtc's `OnSuccess`: force the Opus
/// rtpmap to `opus/48000/2` and request 48 kHz stereo with in-band FEC.
///
/// Firestorm appends a second `a=fmtp` line after the original one (joined
/// by a bare `\r`); here the original line is replaced by the merged
/// parameters, which yields the same effective parameters with a valid SDP.
pub fn mangle_offer_sdp(sdp: &str) -> String {
    let mut out = String::with_capacity(sdp.len() + 160);
    let mut opus_payload: Option<String> = None;
    for raw in sdp.split('\n') {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(pt) = parse_opus_rtpmap(line) {
            let _ = write!(out, "a=rtpmap:{pt} opus/48000/2\r\n");
            opus_payload = Some(pt);
            continue;
        }
        if let Some(pt) = &opus_payload {
            let prefix = format!("a=fmtp:{pt} ");
            if line.starts_with(&prefix) || line == prefix.trim_end() {
                let _ = write!(out, "a=fmtp:{pt} {OPUS_FMTP}\r\n");
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out
}

/// Matches `a=rtpmap:<pt> opus/<rate>/2` (case-insensitive codec name) and
/// returns the payload type.
fn parse_opus_rtpmap(line: &str) -> Option<String> {
    let rest = line.strip_prefix("a=rtpmap:")?;
    let (pt, codec) = rest.split_once(' ')?;
    if pt.is_empty() || !pt.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut parts = codec.trim().split('/');
    let name = parts.next()?;
    let _rate = parts.next()?.parse::<u32>().ok()?;
    let channels = parts.next()?;
    (name.eq_ignore_ascii_case("opus") && channels == "2").then(|| pt.to_string())
}

/// Body of the `ProvisionVoiceAccountRequest` call that sends the SDP offer
/// (`requestVoiceConnection` of the spatial and ad-hoc connections).
pub fn build_provision_request(offer_sdp: &str, channel: &ChannelKind) -> Llsd {
    let mut jsep = Llsd::new_map();
    jsep.insert("type", "offer");
    jsep.insert("sdp", offer_sdp);
    let mut body = Llsd::new_map();
    body.insert("jsep", jsep);
    match channel {
        ChannelKind::Local { parcel_local_id } => {
            if let Some(id) = parcel_local_id {
                body.insert("parcel_local_id", *id);
            }
            body.insert("channel_type", "local");
        }
        ChannelKind::Multiagent { channel_id, credentials } => {
            body.insert("credentials", credentials.as_str());
            body.insert("channel", channel_id.as_str());
            body.insert("channel_type", "multiagent");
        }
    }
    body.insert("voice_server_type", WEBRTC_VOICE_SERVER_TYPE);
    body
}

/// Body of the `ProvisionVoiceAccountRequest` call that closes a session
/// (`breakVoiceConnectionCoro`).
pub fn build_logout_request(viewer_session: &Llsd) -> Llsd {
    let mut body = Llsd::new_map();
    body.insert("logout", true);
    body.insert("viewer_session", viewer_session.clone());
    body.insert("voice_server_type", WEBRTC_VOICE_SERVER_TYPE);
    body
}

/// A local ICE candidate, as needed for trickling.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalCandidate {
    pub foundation: String,
    pub component: u16,
    /// "udp" or "tcp".
    pub protocol: String,
    pub priority: u32,
    pub address: String,
    pub port: u16,
    /// "host", "srflx", "prflx" or "relay".
    pub typ: String,
    pub related_address: String,
    pub related_port: u16,
    pub tcp_type: String,
    pub sdp_mid: String,
    pub mline_index: u16,
}

/// Candidate attribute value in the format of `iceCandidateToTrickleString`
/// (no `candidate:` prefix).
pub fn candidate_trickle_string(c: &LocalCandidate) -> String {
    let mut s = format!(
        "{} {} {} {} {} {} typ ",
        c.foundation, c.component, c.protocol, c.priority, c.address, c.port
    );
    match c.typ.as_str() {
        "host" => s.push_str("host"),
        "srflx" | "relay" | "prflx" => {
            let _ = write!(s, "{} raddr {} rport {}", c.typ, c.related_address, c.related_port);
        }
        other => {
            // llwebrtc logs an error and leaves the type empty.
            log::warn!("unknown ICE candidate type {other}");
        }
    }
    if c.protocol == "tcp" {
        let _ = write!(s, " tcptype {}", c.tcp_type);
    }
    s
}

/// Body of a `VoiceSignalingRequest` call (`processIceUpdatesCoro`): either a
/// batch of candidates or, when there are none, the "completed" marker.
pub fn build_ice_request(candidates: &[LocalCandidate], completed: bool, viewer_session: &Llsd) -> Option<Llsd> {
    let mut body = Llsd::new_map();
    if !candidates.is_empty() {
        let mut list = Llsd::new_array();
        for c in candidates {
            let mut item = Llsd::new_map();
            item.insert("sdpMid", c.sdp_mid.as_str());
            item.insert("sdpMLineIndex", i32::from(c.mline_index));
            item.insert("candidate", candidate_trickle_string(c));
            list.push(item);
        }
        body.insert("candidates", list);
    } else if completed {
        let mut item = Llsd::new_map();
        item.insert("completed", true);
        body.insert("candidate", item);
    } else {
        return None;
    }
    body.insert("viewer_session", viewer_session.clone());
    body.insert("voice_server_type", WEBRTC_VOICE_SERVER_TYPE);
    Some(body)
}

/// Successful provisioning answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvisionAnswer {
    /// Remote SDP answer.
    pub sdp: String,
    /// Opaque session token, echoed back on ICE trickle and logout.
    pub viewer_session: Llsd,
}

/// Validates the provisioning response like `OnVoiceConnectionRequestSuccess`.
pub fn parse_provision_response(result: &Llsd) -> Option<ProvisionAnswer> {
    let jsep = result.get("jsep");
    if result.has("viewer_session") && jsep.has("type") && jsep.get("type").as_str() == "answer" && jsep.has("sdp") {
        Some(ProvisionAnswer {
            sdp: jsep.get("sdp").to_string_value(),
            viewer_session: result.get("viewer_session").clone(),
        })
    } else {
        None
    }
}

/// `{"j":{"p":true}}` (primary) or `{"j":{}}` — `sendJoin`.
pub fn join_message(primary: bool) -> String {
    if primary {
        r#"{"j":{"p":true}}"#.to_string()
    } else {
        r#"{"j":{}}"#.to_string()
    }
}

/// Spatial update (`sendPositionUpdate`): positions in centimetres and
/// quaternion components times 100, truncated to integers.
pub fn position_message(avatar_pos: DVec3, avatar_rot: Quat, listener_pos: DVec3, listener_rot: Quat) -> String {
    fn p(v: DVec3) -> String {
        format!(
            r#"{{"x":{},"y":{},"z":{}}}"#,
            (v.x * 100.0) as i64,
            (v.y * 100.0) as i64,
            (v.z * 100.0) as i64
        )
    }
    fn q(r: Quat) -> String {
        format!(
            r#"{{"x":{},"y":{},"z":{},"w":{}}}"#,
            (r.x * 100.0) as i32,
            (r.y * 100.0) as i32,
            (r.z * 100.0) as i32,
            (r.w * 100.0) as i32
        )
    }
    format!(
        r#"{{"sp":{},"sh":{},"lp":{},"lh":{}}}"#,
        p(avatar_pos),
        q(avatar_rot),
        p(listener_pos),
        q(listener_rot)
    )
}

/// `{"ug":{"<id>":<gain>}}` — `setUserVolume` (volume 0..=1, gain = volume * 220).
pub fn user_gain_message(id: Uuid, volume: f32) -> String {
    format!(
        r#"{{"ug":{{"{}":{}}}}}"#,
        id.hyphenated(),
        gain_value(volume, PEER_GAIN_CONVERSION_FACTOR)
    )
}

/// `{"m":{"<id>":true}}` — `setUserMute`.
pub fn user_mute_message(id: Uuid, mute: bool) -> String {
    format!(r#"{{"m":{{"{}":{}}}}}"#, id.hyphenated(), mute)
}

/// `(uint32_t)(volume * factor)`, saturating for out-of-range input.
pub(crate) fn gain_value(volume: f32, factor: f32) -> u32 {
    let v = volume * factor;
    if v.is_finite() && v > 0.0 { v as u32 } else { 0 }
}

/// Builds the mute / gain reply sent after joins (end of `OnDataReceivedImpl`).
pub(crate) fn mute_gain_reply(mutes: &[Uuid], gains: &[(Uuid, u32)]) -> Option<String> {
    if mutes.is_empty() && gains.is_empty() {
        return None;
    }
    let mut s = String::from("{");
    if !mutes.is_empty() {
        s.push_str(r#""m":{"#);
        for (i, id) in mutes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, r#""{}":true"#, id.hyphenated());
        }
        s.push('}');
    }
    if !gains.is_empty() {
        if !mutes.is_empty() {
            s.push(',');
        }
        s.push_str(r#""ug":{"#);
        for (i, (id, g)) in gains.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, r#""{}":{}"#, id.hyphenated(), g);
        }
        s.push('}');
    }
    s.push('}');
    Some(s)
}

/// One participant entry of an incoming data-channel message.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParticipantUpdate {
    pub agent_id: Uuid,
    /// `Some(primary)` when the entry carries a `"j"` (join) object.
    pub joined: Option<bool>,
    /// `"l": true` — the participant left.
    pub left: bool,
    /// `"p"` — audio power, already divided by 128.
    pub level: Option<f32>,
    /// `"v"` — voice activity detected.
    pub speaking: Option<bool>,
    /// `"m"` — muted by a moderator.
    pub moderator_muted: Option<bool>,
}

/// Errors parsing a data-channel message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DataMessageError {
    #[error("binary data channel message")]
    Binary,
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("expected a JSON object")]
    NotObject,
}

/// Parses an incoming "SLData" message (`OnDataReceivedImpl`): a JSON object
/// keyed by agent id. Null / invalid ids ("probably a test client") and
/// non-object values are skipped.
pub fn parse_data_message(text: &str) -> Result<Vec<ParticipantUpdate>, DataMessageError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| DataMessageError::Json(e.to_string()))?;
    let obj = value.as_object().ok_or(DataMessageError::NotObject)?;
    let mut out = Vec::new();
    for (key, entry) in obj {
        let Ok(agent_id) = Uuid::parse_str(key) else {
            continue;
        };
        if agent_id.is_nil() {
            continue;
        }
        let Some(entry) = entry.as_object() else {
            continue;
        };
        let mut up = ParticipantUpdate {
            agent_id,
            ..Default::default()
        };
        if let Some(j) = entry.get("j").and_then(|j| j.as_object()) {
            up.joined = Some(j.get("p").and_then(|p| p.as_bool()).unwrap_or(false));
        }
        up.left = entry.get("l").and_then(|l| l.as_bool()).unwrap_or(false);
        up.level = entry.get("p").and_then(|p| {
            p.as_i64()
                .map(|v| v as f32)
                .or_else(|| p.as_f64().map(|v| v.trunc() as f32))
                .map(|v| v / 128.0)
        });
        up.speaking = entry.get("v").and_then(|v| v.as_bool());
        up.moderator_muted = entry.get("m").and_then(|m| m.as_bool());
        out.push(up);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "0f9d9a3c-6d5e-4a8b-9c1d-2e3f4a5b6c7d";

    /// Offer as produced by libwebrtc / webrtc-rs before munging.
    const OFFER: &str = "v=0\r\n\
o=- 4215775240449105457 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE 0 1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=mid:0\r\n\
a=rtpmap:111 opus/48000/2\r\n\
a=fmtp:111 minptime=10;useinbandfec=1\r\n\
a=sendrecv\r\n\
m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n\
a=mid:1\r\n\
a=sctp-port:5000\r\n";

    #[test]
    fn mangles_opus_lines() {
        let out = mangle_offer_sdp(OFFER);
        assert!(out.contains("a=rtpmap:111 opus/48000/2\r\n"));
        assert!(out.contains(&format!("a=fmtp:111 {OPUS_FMTP}\r\n")));
        assert_eq!(out.matches("a=fmtp:111").count(), 1);
        assert!(out.contains("m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n"));
        // Idempotent.
        assert_eq!(mangle_offer_sdp(&out), out);
        // Only opus/…/2 is touched.
        let other = "a=rtpmap:0 PCMU/8000\r\na=fmtp:0 x=1\r\n";
        assert_eq!(mangle_offer_sdp(other), other);
    }

    #[test]
    fn provision_request_local_parcel() {
        let body = build_provision_request("SDP", &ChannelKind::Local { parcel_local_id: Some(42) });
        assert_eq!(body.get("jsep").get("type").as_str(), "offer");
        assert_eq!(body.get("jsep").get("sdp").as_str(), "SDP");
        assert_eq!(body.get("parcel_local_id"), &Llsd::Integer(42));
        assert_eq!(body.get("channel_type").as_str(), "local");
        assert_eq!(body.get("voice_server_type").as_str(), "webrtc");
        assert!(!body.has("credentials"));

        let xml = aurora_llsd::to_xml_string(&body);
        let back = aurora_llsd::from_xml(xml.as_bytes()).unwrap();
        assert_eq!(back, body);
    }

    #[test]
    fn provision_request_estate_and_multiagent() {
        let estate = build_provision_request("SDP", &ChannelKind::Local { parcel_local_id: None });
        assert!(!estate.has("parcel_local_id"));
        let adhoc = build_provision_request(
            "SDP",
            &ChannelKind::Multiagent {
                channel_id: "chan".into(),
                credentials: "creds".into(),
            },
        );
        assert_eq!(adhoc.get("channel_type").as_str(), "multiagent");
        assert_eq!(adhoc.get("channel").as_str(), "chan");
        assert_eq!(adhoc.get("credentials").as_str(), "creds");
    }

    #[test]
    fn provision_response_parsing() {
        let xml = r#"<?xml version="1.0" ?><llsd><map>
            <key>jsep</key><map><key>type</key><string>answer</string><key>sdp</key><string>v=0
o=- 1 2 IN IP4 127.0.0.1
</string></map>
            <key>viewer_session</key><string>abc-123</string>
            </map></llsd>"#;
        let llsd = aurora_llsd::from_xml(xml.as_bytes()).unwrap();
        let ans = parse_provision_response(&llsd).unwrap();
        assert!(ans.sdp.starts_with("v=0"));
        assert_eq!(ans.viewer_session, Llsd::String("abc-123".into()));

        let mut bad = Llsd::new_map();
        let mut jsep = Llsd::new_map();
        jsep.insert("type", "offer");
        jsep.insert("sdp", "x");
        bad.insert("jsep", jsep);
        bad.insert("viewer_session", "s");
        assert!(parse_provision_response(&bad).is_none());
        assert!(parse_provision_response(&Llsd::Undef).is_none());
    }

    #[test]
    fn logout_and_ice_bodies() {
        let session = Llsd::String("sess".into());
        let logout = build_logout_request(&session);
        assert_eq!(logout.get("logout"), &Llsd::Boolean(true));
        assert_eq!(logout.get("viewer_session"), &session);
        assert_eq!(logout.get("voice_server_type").as_str(), "webrtc");

        let c = LocalCandidate {
            foundation: "1234".into(),
            component: 1,
            protocol: "udp".into(),
            priority: 2_130_706_431,
            address: "192.168.1.10".into(),
            port: 60_001,
            typ: "host".into(),
            sdp_mid: "0".into(),
            mline_index: 0,
            ..Default::default()
        };
        assert_eq!(candidate_trickle_string(&c), "1234 1 udp 2130706431 192.168.1.10 60001 typ host");
        let srflx = LocalCandidate {
            typ: "srflx".into(),
            address: "203.0.113.5".into(),
            related_address: "192.168.1.10".into(),
            related_port: 60_001,
            ..c.clone()
        };
        assert_eq!(
            candidate_trickle_string(&srflx),
            "1234 1 udp 2130706431 203.0.113.5 60001 typ srflx raddr 192.168.1.10 rport 60001"
        );
        let tcp = LocalCandidate {
            protocol: "tcp".into(),
            tcp_type: "passive".into(),
            ..c.clone()
        };
        assert!(candidate_trickle_string(&tcp).ends_with("typ host tcptype passive"));

        let body = build_ice_request(&[c], false, &session).unwrap();
        let list = body.get("candidates").as_array();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].get("sdpMid").as_str(), "0");
        assert_eq!(list[0].get("sdpMLineIndex"), &Llsd::Integer(0));
        assert!(!body.has("candidate"));

        let done = build_ice_request(&[], true, &session).unwrap();
        assert_eq!(done.get("candidate").get("completed"), &Llsd::Boolean(true));
        assert_eq!(done.get("viewer_session"), &session);
        assert!(build_ice_request(&[], false, &session).is_none());
    }

    #[test]
    fn data_channel_outgoing_messages() {
        assert_eq!(join_message(true), r#"{"j":{"p":true}}"#);
        assert_eq!(join_message(false), r#"{"j":{}}"#);
        let id = Uuid::parse_str(AGENT).unwrap();
        assert_eq!(user_gain_message(id, 0.5), format!(r#"{{"ug":{{"{AGENT}":110}}}}"#));
        assert_eq!(user_mute_message(id, true), format!(r#"{{"m":{{"{AGENT}":true}}}}"#));
        let msg = position_message(
            DVec3::new(256_128.25, 255_872.5, 23.999),
            Quat::from_xyzw(0.0, 0.0, 0.70710677, 0.70710677),
            DVec3::new(256_120.0, 255_870.0, -1.5),
            Quat::IDENTITY,
        );
        assert_eq!(
            msg,
            r#"{"sp":{"x":25612825,"y":25587250,"z":2399},"sh":{"x":0,"y":0,"z":70,"w":70},"lp":{"x":25612000,"y":25587000,"z":-150},"lh":{"x":0,"y":0,"z":0,"w":100}}"#
        );
        // Must be valid JSON.
        assert!(serde_json::from_str::<serde_json::Value>(&msg).is_ok());
    }

    #[test]
    fn mute_gain_reply_format() {
        let a = Uuid::parse_str(AGENT).unwrap();
        assert_eq!(mute_gain_reply(&[], &[]), None);
        assert_eq!(
            mute_gain_reply(&[a], &[(a, 100)]).unwrap(),
            format!(r#"{{"m":{{"{AGENT}":true}},"ug":{{"{AGENT}":100}}}}"#)
        );
        assert_eq!(mute_gain_reply(&[], &[(a, 0)]).unwrap(), format!(r#"{{"ug":{{"{AGENT}":0}}}}"#));
        assert_eq!(gain_value(-1.0, 220.0), 0);
        assert_eq!(gain_value(f32::NAN, 220.0), 0);
    }

    #[test]
    fn data_channel_incoming_messages() {
        let msg = format!(
            r#"{{
                "{AGENT}": {{"j": {{"p": true}}, "p": 64, "v": true}},
                "11111111-2222-3333-4444-555555555555": {{"l": true}},
                "00000000-0000-0000-0000-000000000000": {{"j": {{}}}},
                "not-a-uuid": {{"v": true}},
                "66666666-7777-8888-9999-000000000000": 5,
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee": {{"m": true, "p": 12.7}}
            }}"#
        );
        let ups = parse_data_message(&msg).unwrap();
        assert_eq!(ups.len(), 3);
        let a = ups.iter().find(|u| u.agent_id == Uuid::parse_str(AGENT).unwrap()).unwrap();
        assert_eq!(a.joined, Some(true));
        assert_eq!(a.level, Some(0.5));
        assert_eq!(a.speaking, Some(true));
        assert!(!a.left);
        let l = ups.iter().find(|u| u.left).unwrap();
        assert_eq!(l.joined, None);
        let m = ups.iter().find(|u| u.moderator_muted == Some(true)).unwrap();
        assert_eq!(m.level, Some(12.0 / 128.0));

        assert_eq!(parse_data_message("[1,2]"), Err(DataMessageError::NotObject));
        assert!(matches!(parse_data_message("{oops"), Err(DataMessageError::Json(_))));
        assert_eq!(parse_data_message("{}").unwrap(), vec![]);
    }

    #[test]
    fn server_type_and_stun() {
        assert!(is_webrtc_voice_server("webrtc"));
        assert!(!is_webrtc_voice_server(""));
        assert!(!is_webrtc_voice_server("vivox"));
        assert_eq!(
            stun_servers_for_grid("Agni"),
            vec![
                "stun:stun1.agni.secondlife.io:3478",
                "stun:stun2.agni.secondlife.io:3478",
                "stun:stun3.agni.secondlife.io:3478"
            ]
        );
        assert_eq!(stun_servers_for_grid("aditi").len(), 2);
    }
}
