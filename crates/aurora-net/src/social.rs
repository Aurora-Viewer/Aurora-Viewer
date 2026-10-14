//! Groups, mute list and multi-agent chat sessions (group / ad-hoc IM):
//! types shared with the viewer and the LLSD parsers of the ChatterBox
//! event-queue messages (`newview/llimview.cpp`, `llspeakers.cpp`,
//! `llagent.cpp` in Firestorm).

use aurora_llsd::Llsd;
use uuid::Uuid;

/// A group the agent belongs to (AgentGroupDataUpdate, LLGroupData).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMembership {
    pub id: Uuid,
    pub name: String,
    pub insignia: Uuid,
    pub powers: u64,
    pub accept_notices: bool,
    pub list_in_profile: bool,
    pub contribution: i32,
}

/// Where the mute list came from (LLMuteList::EMuteListSource).
#[derive(Debug, Clone, PartialEq)]
pub enum MuteListSource {
    /// The file downloaded from the simulator (Xfer), in the
    /// "type id name|flags" text format.
    File(String),
    /// UseCachedMuteList: our cached copy is current.
    UseCache,
}

/// First message of a group / conference session (ChatterBoxInvitation).
#[derive(Debug, Clone)]
pub struct SessionInvite {
    pub session: Uuid,
    pub from: Uuid,
    pub from_name: String,
    pub message: String,
    /// Group or conference name (binary bucket).
    pub session_name: String,
    pub offline: bool,
    pub timestamp: u32,
}

/// Change of one participant of a chat session (LLIMSpeakerMgr).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionAgent {
    pub agent: Uuid,
    /// Some(true) = entered, Some(false) = left, None = info only.
    pub present: Option<bool>,
    pub moderator: Option<bool>,
    /// Text chat muted by a moderator.
    pub text_muted: Option<bool>,
    /// An "ENTER" transition of "agent_updates": they just joined (the full
    /// list of a session start or "agent_info" is not a transition).
    pub entered: bool,
}

/// A line of the chat server history ("fetch history").
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryLine {
    pub from_id: Uuid,
    pub from: String,
    pub message: String,
    /// Unix time.
    pub time: u32,
}

/// ChatSessionRequest methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMethod {
    AcceptInvitation,
    DeclineInvitation,
    FetchHistory,
}

impl SessionMethod {
    pub fn name(self) -> &'static str {
        match self {
            SessionMethod::AcceptInvitation => "accept invitation",
            SessionMethod::DeclineInvitation => "decline invitation",
            SessionMethod::FetchHistory => "fetch history",
        }
    }
}

/// Participants of a session: the full list (LLIMSpeakerMgr::setSpeakers:
/// "agent_info" map or "agents" array) or changes (updateSpeakers:
/// "agent_updates" map with transitions, or "updates" map of strings).
pub fn parse_session_agents(body: &Llsd) -> Vec<SessionAgent> {
    let mut out = Vec::new();
    let info = |a: &mut SessionAgent, i: &Llsd| {
        if i.as_map().is_some_and(|m| m.contains_key("is_moderator")) {
            a.moderator = Some(i["is_moderator"].as_bool());
        }
        if i.as_map().is_some_and(|m| m.contains_key("mutes")) {
            a.text_muted = Some(i["mutes"]["text"].as_bool());
        }
    };
    if let Some(m) = body["agent_info"].as_map() {
        for (k, v) in m {
            let Ok(agent) = Uuid::parse_str(k) else { continue };
            let mut a = SessionAgent {
                agent,
                present: Some(true),
                ..Default::default()
            };
            info(&mut a, v);
            out.push(a);
        }
    } else if !body["agents"].as_array().is_empty() {
        for v in body["agents"].as_array() {
            let agent = v.as_uuid();
            if !agent.is_nil() {
                out.push(SessionAgent {
                    agent,
                    present: Some(true),
                    ..Default::default()
                });
            }
        }
    }
    if let Some(m) = body["agent_updates"].as_map() {
        for (k, v) in m {
            let Ok(agent) = Uuid::parse_str(k) else { continue };
            let mut a = SessionAgent {
                agent,
                ..Default::default()
            };
            match v["transition"].as_str() {
                "ENTER" => {
                    a.present = Some(true);
                    a.entered = true;
                }
                "LEAVE" => a.present = Some(false),
                _ => {}
            }
            info(&mut a, &v["info"]);
            out.push(a);
        }
    } else if let Some(m) = body["updates"].as_map() {
        for (k, v) in m {
            let Ok(agent) = Uuid::parse_str(k) else { continue };
            let present = match v.as_str() {
                "ENTER" => Some(true),
                "LEAVE" => Some(false),
                _ => None,
            };
            out.push(SessionAgent {
                agent,
                present,
                ..Default::default()
            });
        }
    }
    out
}

/// Body of a ChatterBoxInvitation carrying an instant message.
pub fn parse_invitation(body: &Llsd) -> Option<SessionInvite> {
    let p = &body["instantmessage"]["message_params"];
    p.as_map()?;
    let bucket = p["data"]["binary_bucket"].as_binary();
    let end = bucket.iter().position(|&c| c == 0).unwrap_or(bucket.len());
    let mut session_name = String::from_utf8_lossy(&bucket[..end]).into_owned();
    if session_name.is_empty() {
        session_name = body["session_name"].as_str().to_owned();
    }
    Some(SessionInvite {
        session: p["id"].as_uuid(),
        from: p["from_id"].as_uuid(),
        from_name: p["from_name"].as_str().to_owned(),
        message: p["message"].as_str().to_owned(),
        session_name,
        offline: p["offline"].as_i32() != 0,
        timestamp: p["timestamp"].as_u32(),
    })
}

/// `invitation_type` of a voice invitation (EMultiAgentChatSessionType, llimview.cpp).
const P2P_CHAT_SESSION: i32 = 2;

/// Voice call invitation: a ChatterBoxInvitation carrying "voice" instead of
/// an instant message (LLViewerChatterBoxInvitation::post).
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceInvite {
    pub session: Uuid,
    pub session_name: String,
    pub from: Uuid,
    pub from_name: String,
    /// A call between two residents (IM_SESSION_P2P_INVITE), else a group or
    /// ad-hoc call (IM_SESSION_INVITE).
    pub p2p: bool,
    /// The voice channel info ("voice" map: channel_uri,
    /// channel_credentials, voice_server_type…), handed back on accept.
    pub voice: Llsd,
}

/// Body of a ChatterBoxInvitation carrying a voice invitation.
pub fn parse_voice_invitation(body: &Llsd) -> Option<VoiceInvite> {
    let voice = &body["voice"];
    voice.as_map()?;
    Some(VoiceInvite {
        session: body["session_id"].as_uuid(),
        session_name: body["session_name"].as_str().to_owned(),
        from: body["from_id"].as_uuid(),
        from_name: body["from_name"].as_str().to_owned(),
        p2p: voice["invitation_type"].as_i32() == P2P_CHAT_SESSION,
        voice: voice.clone(),
    })
}

/// ChatSessionRequest methods of the voice calls (llvoicechannel.cpp,
/// llimview.cpp).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallMethod {
    /// Ask the channel credentials of a group / ad-hoc / P2P session
    /// (LLVoiceChannelGroup::voiceCallCapCoro).
    Call,
    /// Start a P2P voice session with a resident (startP2PVoiceCoro).
    StartP2p { other: Uuid },
    /// Decline a P2P call (LLIncomingCallDialog::processCallResponse).
    DeclineP2p,
    /// Start an ad-hoc conference with these residents, under a temporary
    /// session id the server replaces (startConferenceCoro).
    StartConference { agents: Vec<Uuid> },
    /// Invite residents into a conference (LLFloaterIMSession::
    /// inviteToSession, FSFloaterIM::inviteToSession).
    Invite { agents: Vec<Uuid> },
}

impl CallMethod {
    pub fn name(&self) -> &'static str {
        match self {
            CallMethod::Call => "call",
            CallMethod::StartP2p { .. } => "start p2p voice",
            CallMethod::DeclineP2p => "decline p2p voice",
            CallMethod::StartConference { .. } => "start conference",
            CallMethod::Invite { .. } => "invite",
        }
    }

    /// The POST body, as Firestorm builds it. `server_type` is the region's
    /// voice server type (VoiceServerType unset: getVersion's).
    pub fn body(&self, session: Uuid, server_type: &str) -> Llsd {
        match self {
            CallMethod::Invite { agents } => aurora_llsd::llsd_map! {
                "params" => Llsd::Array(agents.iter().map(|a| Llsd::from(*a)).collect()),
                "method" => self.name(),
                "session-id" => session,
            },
            CallMethod::StartConference { agents } => aurora_llsd::llsd_map! {
                "method" => self.name(),
                "session-id" => session,
                "params" => Llsd::Array(agents.iter().map(|a| Llsd::from(*a)).collect()),
                "alt_params" => aurora_llsd::llsd_map! { "voice_server_type" => server_type },
            },
            CallMethod::Call => aurora_llsd::llsd_map! {
                "method" => self.name(),
                "session-id" => session,
                "alt_params" => aurora_llsd::llsd_map! { "preferred_voice_server_type" => server_type },
            },
            CallMethod::StartP2p { other } => aurora_llsd::llsd_map! {
                "method" => self.name(),
                "session-id" => session,
                "params" => *other,
                "alt_params" => aurora_llsd::llsd_map! { "voice_server_type" => server_type },
            },
            CallMethod::DeclineP2p => aurora_llsd::llsd_map! {
                "method" => self.name(),
                "session-id" => session,
            },
        }
    }
}

/// AgentGroupDataUpdate from the event queue (LLAgentGroupDataUpdateViewerNode).
pub fn parse_group_data(body: &Llsd) -> Vec<GroupMembership> {
    let body = if body.as_map().is_some_and(|m| m.contains_key("body")) {
        &body["body"]
    } else {
        body
    };
    body["GroupData"]
        .as_array()
        .iter()
        .enumerate()
        .filter(|(_, g)| !g["GroupID"].as_uuid().is_nil())
        .map(|(i, g)| GroupMembership {
            id: g["GroupID"].as_uuid(),
            name: g["GroupName"].as_str().to_owned(),
            insignia: g["GroupInsigniaID"].as_uuid(),
            powers: g["GroupPowers"].as_u64(),
            accept_notices: g["AcceptNotices"].as_bool(),
            list_in_profile: body["NewGroupData"][i]["ListInProfile"].as_bool(),
            contribution: g["Contribution"].as_i32(),
        })
        .collect()
}

/// Reply of "fetch history" (array oldest to newest).
pub fn parse_history(v: &Llsd) -> Vec<HistoryLine> {
    v.as_array()
        .iter()
        .filter(|l| l.as_map().is_some())
        .map(|l| HistoryLine {
            from_id: l["from_id"].as_uuid(),
            from: l["from"].as_str().to_owned(),
            message: l["message"].as_str().to_owned(),
            time: l["time"].as_f64().max(0.0) as u32,
        })
        .collect()
}

/// CRC-32 of a file's bytes (LLCRC: polynomial 0xedb88320, start ~0, final
/// inversion); 0 for an empty or missing file, as LLCRC::update(filename).
pub fn ll_crc(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn voice_invitations_are_told_apart_from_messages() {
        let session = Uuid::from_u128(5);
        let from = Uuid::from_u128(6);
        let body = llsd_map! {
            "session_id" => session,
            "session_name" => "Nova",
            "from_id" => from,
            "from_name" => "Nova Resident",
            "voice" => llsd_map! { "invitation_type" => 2, "channel_uri" => "ch", "channel_credentials" => "cr" },
        };
        let inv = parse_voice_invitation(&body).expect("voice invitation");
        assert_eq!((inv.session, inv.from, inv.p2p), (session, from, true));
        assert_eq!(inv.voice["channel_uri"].as_str(), "ch");
        assert!(parse_invitation(&body).is_none());
        let group = llsd_map! { "session_id" => session, "voice" => llsd_map! { "invitation_type" => 0 } };
        assert!(!parse_voice_invitation(&group).expect("group call").p2p);
        assert!(parse_voice_invitation(&llsd_map! { "session_id" => session }).is_none());
    }

    #[test]
    fn call_bodies_follow_firestorm() {
        let s = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let call = CallMethod::Call.body(s, "webrtc");
        assert_eq!(call["method"].as_str(), "call");
        assert_eq!(call["session-id"].as_uuid(), s);
        assert_eq!(call["alt_params"]["preferred_voice_server_type"].as_str(), "webrtc");
        let start = CallMethod::StartP2p { other }.body(s, "webrtc");
        assert_eq!(start["method"].as_str(), "start p2p voice");
        assert_eq!(start["params"].as_uuid(), other);
        assert_eq!(start["alt_params"]["voice_server_type"].as_str(), "webrtc");
        let conference = CallMethod::StartConference { agents: vec![other, s] }.body(s, "webrtc");
        assert_eq!(conference["method"].as_str(), "start conference");
        assert_eq!(conference["params"].as_array().len(), 2);
        assert_eq!(conference["params"][0].as_uuid(), other);
        assert_eq!(conference["alt_params"]["voice_server_type"].as_str(), "webrtc");
        let invite = CallMethod::Invite { agents: vec![other] }.body(s, "webrtc");
        assert_eq!(invite["method"].as_str(), "invite");
        assert_eq!(invite["params"][0].as_uuid(), other);
        assert!(invite.as_map().is_some_and(|m| !m.contains_key("alt_params")));
        let decline = CallMethod::DeclineP2p.body(s, "webrtc");
        assert_eq!(decline["method"].as_str(), "decline p2p voice");
        assert!(decline.as_map().is_some_and(|m| !m.contains_key("alt_params")));
    }

    #[test]
    fn crc_matches_crc32() {
        assert_eq!(ll_crc(b""), 0);
        assert_eq!(ll_crc(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn agents_full_and_updates() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let body = llsd_map! {
            "agent_info" => llsd_map! {
                a.to_string() => llsd_map! { "is_moderator" => true, "mutes" => llsd_map! { "text" => false } },
            },
        };
        let v = parse_session_agents(&body);
        assert_eq!(
            v,
            vec![SessionAgent {
                agent: a,
                present: Some(true),
                moderator: Some(true),
                text_muted: Some(false),
                entered: false,
            }]
        );
        let body = llsd_map! {
            "agent_updates" => llsd_map! {
                a.to_string() => llsd_map! { "transition" => "LEAVE" },
                b.to_string() => llsd_map! { "transition" => "ENTER", "info" => llsd_map! { "is_moderator" => false } },
            },
        };
        let v = parse_session_agents(&body);
        assert_eq!(v.len(), 2);
        assert_eq!(v.iter().find(|x| x.agent == a).unwrap().present, Some(false));
        assert_eq!(v.iter().find(|x| x.agent == b).unwrap().moderator, Some(false));
        // only an ENTER transition is an arrival
        assert!(v.iter().find(|x| x.agent == b).unwrap().entered);
        assert!(!v.iter().find(|x| x.agent == a).unwrap().entered);
        let body = llsd_map! { "agents" => Llsd::Array(vec![a.into(), b.into()]) };
        assert_eq!(parse_session_agents(&body).len(), 2);
    }

    #[test]
    fn invitation() {
        let s = Uuid::from_u128(7);
        let body = llsd_map! {
            "instantmessage" => llsd_map! {
                "message_params" => llsd_map! {
                    "id" => s,
                    "from_id" => Uuid::from_u128(3),
                    "from_name" => "Jane Doe",
                    "message" => "hello",
                    "offline" => 0,
                    "timestamp" => 0,
                    "data" => llsd_map! { "binary_bucket" => b"My Group\0".to_vec() },
                },
            },
        };
        let i = parse_invitation(&body).unwrap();
        assert_eq!(i.session, s);
        assert_eq!(i.session_name, "My Group");
        assert_eq!(i.message, "hello");
    }

    #[test]
    fn groups() {
        let g = Uuid::from_u128(9);
        let body = llsd_map! {
            "AgentData" => Llsd::Array(vec![llsd_map! { "AgentID" => Uuid::from_u128(1) }]),
            "GroupData" => Llsd::Array(vec![llsd_map! {
                "GroupID" => g, "GroupName" => "Builders", "AcceptNotices" => true,
                "GroupPowers" => vec![0u8, 0, 0, 0, 0, 0, 0, 5],
            }]),
            "NewGroupData" => Llsd::Array(vec![llsd_map! { "ListInProfile" => true }]),
        };
        let v = parse_group_data(&body);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Builders");
        assert_eq!(v[0].powers, 5);
        assert!(v[0].accept_notices && v[0].list_in_profile);
    }
}
