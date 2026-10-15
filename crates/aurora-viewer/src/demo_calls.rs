//! Voice calls in the demo (AURORA_DEMO_CALL): the ChatSessionRequest
//! answers and a stand-in for the voice server, which the offline demo does
//! not have. Every call made from the interface works with it; the variable
//! starts one, or makes someone call:
//!
//! - `p2p`: we call Tess; she answers after 3 s, then the two talk in turn;
//! - `hangup`: the same, and Tess hangs up after 12 s;
//! - `group`: a call in the group Aurora Builders (Nova, Tess, Orion);
//! - `conference`: a voice conference with Tess and Nova ("start
//!   conference", then the call once the session has its id);
//! - `incoming`: Tess calls us (the card with Accept / Decline / Reply by
//!   text);
//! - `incoming-group`: Nova starts a call in Aurora Builders;
//! - `conference-legacy`: the conference of `conference`, refused by the
//!   capability (400): it starts by message (IM_SESSION_CONFERENCE_START).
//!
//! - `conference-invite`: the conference of `conference`, then Orion added
//!   once it is connected: he joins and our "was invited" message follows.
//!
//! Residents invited into a conference ("invite") join it at once.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::world::World;
use crate::world::calls::{CallKind, CallMember, ChannelStatus};
use aurora_net::{CallMethod, NetEvent, VoiceInvite};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    P2p,
    Hangup,
    Group,
    Conference,
    ConferenceLegacy,
    ConferenceInvite,
    Incoming,
    IncomingGroup,
}

pub fn mode() -> Option<Mode> {
    // the calls of AURORA_DEMO_CONV=aurora:call…
    if let Some(call) = super::aurora_scenario().and_then(|s| s.call) {
        use super::AuroraCall;
        return Some(match call {
            AuroraCall::Incoming => Mode::Incoming,
            AuroraCall::InCall | AuroraCall::Mini => Mode::Group,
            AuroraCall::Ongoing | AuroraCall::IncomingGroup => Mode::IncomingGroup,
            AuroraCall::P2p => Mode::P2p,
            AuroraCall::Conference => Mode::Conference,
        });
    }
    match std::env::var("AURORA_DEMO_CALL").ok()?.as_str() {
        "p2p" => Some(Mode::P2p),
        "hangup" => Some(Mode::Hangup),
        "group" => Some(Mode::Group),
        "conference" => Some(Mode::Conference),
        "conference-legacy" => Some(Mode::ConferenceLegacy),
        "conference-invite" => Some(Mode::ConferenceInvite),
        "incoming" => Some(Mode::Incoming),
        "incoming-group" => Some(Mode::IncomingGroup),
        _ => None,
    }
}

/// How long the simulated server takes to connect, and the other resident
/// of a P2P call to answer.
const CONNECT: Duration = Duration::from_millis(1500);
const ANSWER: Duration = Duration::from_secs(3);
const HANGUP: Duration = Duration::from_secs(12);
/// When the scenario starts, once in world.
const SCENARIO_DELAY: Duration = Duration::from_secs(3);

/// The simulated channel of the call in progress.
#[derive(Default)]
pub struct CallDemo {
    /// The channel joined and since when.
    joined: Option<(String, Instant)>,
    /// First frame in world, and the scenario already started.
    since: Option<Instant>,
    started: bool,
    /// `conference-invite`: Orion was added.
    invited: bool,
}

fn present(agent: Uuid) -> aurora_net::SessionAgent {
    aurora_net::SessionAgent {
        agent,
        present: Some(true),
        moderator: None,
        text_muted: None,
        entered: false,
    }
}

/// The start reply of a conference: its real id, and who is in it.
fn conference_started(temp: Uuid, agents: &[Uuid]) -> NetEvent {
    NetEvent::SessionStarted {
        temp_session: temp,
        session: Uuid::from_u128(temp.as_u128() ^ 0xC0F),
        success: true,
        error: String::new(),
        agents: std::iter::once(DEMO_AGENT).chain(agents.iter().copied()).map(present).collect(),
    }
}

/// A conference started by message (IM_SESSION_CONFERENCE_START): the
/// residents are the ids of the binary bucket.
pub fn legacy_conference(temp: Uuid, bucket: &[u8]) -> Vec<NetEvent> {
    let agents: Vec<Uuid> = bucket.as_chunks::<16>().0.iter().map(|c| Uuid::from_bytes(*c)).collect();
    vec![conference_started(temp, &agents)]
}

/// The answers of ChatSessionRequest: a P2P session always starts and every
/// session gets a channel.
pub fn reply(method: CallMethod, session: Uuid) -> Vec<NetEvent> {
    let ok = aurora_llsd::Llsd::Undef;
    match &method {
        // `conference-legacy`: the capability refuses (startConferenceCoro)
        CallMethod::StartConference { .. } if mode() == Some(Mode::ConferenceLegacy) => {
            return vec![NetEvent::CallReply {
                method,
                session,
                result: Err(400),
            }];
        }
        // a conference gets its real id in the session's start reply
        CallMethod::StartConference { agents } => {
            let started = conference_started(session, agents);
            return vec![
                NetEvent::CallReply {
                    method,
                    session,
                    result: Ok(ok),
                },
                started,
            ];
        }
        // the invited join the conference
        CallMethod::Invite { agents } => {
            // ENTER transitions, as the server sends them
            let joined = NetEvent::SessionAgents {
                session,
                agents: agents
                    .iter()
                    .map(|a| aurora_net::SessionAgent {
                        entered: true,
                        ..present(*a)
                    })
                    .collect(),
            };
            return vec![
                NetEvent::CallReply {
                    method,
                    session,
                    result: Ok(ok),
                },
                joined,
            ];
        }
        _ => {}
    }
    let result = match method {
        CallMethod::StartP2p { .. } | CallMethod::DeclineP2p | CallMethod::StartConference { .. } | CallMethod::Invite { .. } => {
            Ok(aurora_llsd::Llsd::Undef)
        }
        CallMethod::Call => Ok(aurora_llsd::llsd_map! {
            "voice_credentials" => aurora_llsd::llsd_map! {
                "channel_uri" => format!("demo-{session}"),
                "channel_credentials" => "demo",
                "voice_server_type" => "webrtc",
            },
        }),
    };
    vec![NetEvent::CallReply { method, session, result }]
}

fn invite(group: bool) -> VoiceInvite {
    let (session, from, from_name, session_name) = if group {
        (DEMO_GROUP1, DEMO_TESS, "Nova Example", "Aurora Builders")
    } else {
        // (the demo's names call DEMO_NOVA "Tess" and DEMO_TESS "Nova")
        (
            Uuid::from_u128(DEMO_AGENT.as_u128() ^ DEMO_NOVA.as_u128()),
            DEMO_NOVA,
            "Tess Touch",
            "",
        )
    };
    VoiceInvite {
        session,
        session_name: session_name.into(),
        from,
        from_name: from_name.into(),
        p2p: !group,
        voice: aurora_llsd::llsd_map! {
            "channel_uri" => format!("demo-{session}"),
            "channel_credentials" => "demo",
            "invitation_type" => if group { 0 } else { 2 },
        },
    }
}

impl CallDemo {
    /// Starts the scenario of AURORA_DEMO_CALL once in world: a call made
    /// in `world`, or invitation events to apply.
    pub fn scenario(&mut self, world: &mut World, in_world: bool) -> Vec<NetEvent> {
        // `conference-invite`: Orion added once the conference is connected
        if self.started && !self.invited && mode() == Some(Mode::ConferenceInvite) {
            if let Some(c) = world
                .calls
                .active
                .as_ref()
                .filter(|c| c.state == crate::world::calls::CallState::Connected)
            {
                self.invited = true;
                let session = c.session;
                world.invite_to_conversation(session, vec![DEMO_LOUP]);
            }
            return Vec::new();
        }
        if self.started || !in_world {
            return Vec::new();
        }
        let Some(mode) = mode() else {
            self.started = true;
            return Vec::new();
        };
        let since = *self.since.get_or_insert_with(Instant::now);
        if since.elapsed() < SCENARIO_DELAY {
            return Vec::new();
        }
        self.started = true;
        match mode {
            Mode::P2p | Mode::Hangup => world.start_call(DEMO_NOVA),
            Mode::Group => world.start_call(DEMO_GROUP1),
            Mode::Conference | Mode::ConferenceLegacy | Mode::ConferenceInvite => world.start_conference(vec![DEMO_NOVA, DEMO_TESS]),
            Mode::Incoming => return vec![NetEvent::VoiceInvite(invite(false))],
            Mode::IncomingGroup => return vec![NetEvent::VoiceInvite(invite(true))],
        }
        Vec::new()
    }

    /// The channel's state and participants this frame, as the voice
    /// session reports them; `transmit` is our microphone.
    pub fn report(&mut self, world: &World, transmit: bool) -> Option<(ChannelStatus, Vec<CallMember>)> {
        let Some(call) = world.calls.active.as_ref() else {
            self.joined = None;
            return None;
        };
        let channel = call.channel.as_ref()?;
        let since = match &self.joined {
            Some((id, t)) if *id == channel.id => *t,
            _ => {
                let now = Instant::now();
                self.joined = Some((channel.id.clone(), now));
                now
            }
        };
        let t = since.elapsed();
        let me = CallMember {
            id: world.agent_id,
            level: if transmit { 0.4 } else { 0.0 },
            speaking: transmit,
            muted: false,
        };
        if t < CONNECT {
            return Some((ChannelStatus::Connecting, vec![me]));
        }
        // the others talk in turn, two seconds each
        let turn = (t.as_secs() / 2) as usize;
        let talking = |i: usize, n: usize| turn % (n + 1) == i;
        let member = |id: Uuid, i: usize, n: usize| CallMember {
            id,
            level: if talking(i, n) { 0.5 } else { 0.0 },
            speaking: talking(i, n),
            // Tess muted by the group's moderator: her badge on the stage
            muted: id == DEMO_TESS,
        };
        let mut members = vec![me];
        if call.p2p() {
            let gone = mode() == Some(Mode::Hangup) && t > HANGUP;
            if (call.incoming || t > ANSWER) && !gone {
                members.push(member(call.conversation(), 0, 1));
            }
        } else {
            // a conference: its participants; a group: three of its members
            let mut ids: Vec<Uuid> = match call.kind {
                CallKind::AdHoc => world
                    .groups
                    .sessions
                    .get(&call.session)
                    .map(|s| s.participants.iter().copied().filter(|id| *id != world.agent_id).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            if ids.is_empty() {
                ids = vec![DEMO_TESS, DEMO_NOVA, DEMO_LOUP];
            }
            let n = ids.len();
            for (i, id) in ids.into_iter().enumerate() {
                members.push(member(id, i, n));
            }
        }
        Some((ChannelStatus::Connected, members))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_gets_a_channel() {
        let s = Uuid::from_u128(9);
        let ev = reply(CallMethod::Call, s);
        let NetEvent::CallReply { result: Ok(body), .. } = &ev[0] else {
            panic!("a reply");
        };
        assert_eq!(body["voice_credentials"]["channel_uri"].as_str(), format!("demo-{s}"));
        assert!(invite(false).p2p && !invite(true).p2p);
    }
}
