//! Voice calls in the demo (AURORA_DEMO_CALL): the ChatSessionRequest
//! answers and a stand-in for the voice server, which the offline demo does
//! not have. Every call made from the interface works with it; the variable
//! starts one, or makes someone call:
//!
//! - `p2p`: we call Tess; she answers after 3 s, then the two talk in turn;
//! - `hangup`: the same, and Tess hangs up after 12 s;
//! - `group`: a call in the group Aurora Builders (Nova, Tess, Orion);
//! - `incoming`: Tess calls us (the card with Accept / Decline / Reply by
//!   text);
//! - `incoming-group`: Nova starts a call in Aurora Builders.

use super::{DEMO_AGENT, DEMO_GROUP1, DEMO_LOUP, DEMO_NOVA, DEMO_TESS};
use crate::world::World;
use crate::world::calls::{CallMember, ChannelStatus};
use aurora_net::{CallMethod, NetEvent, VoiceInvite};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    P2p,
    Hangup,
    Group,
    Incoming,
    IncomingGroup,
}

pub fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_CALL").ok()?.as_str() {
        "p2p" => Some(Mode::P2p),
        "hangup" => Some(Mode::Hangup),
        "group" => Some(Mode::Group),
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
}

/// The answers of ChatSessionRequest: a P2P session always starts and every
/// session gets a channel.
pub fn reply(method: CallMethod, session: Uuid) -> Vec<NetEvent> {
    let result = match method {
        CallMethod::StartP2p { .. } | CallMethod::DeclineP2p => Ok(aurora_llsd::Llsd::Undef),
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
        };
        let mut members = vec![me];
        if call.p2p() {
            let gone = mode() == Some(Mode::Hangup) && t > HANGUP;
            if (call.incoming || t > ANSWER) && !gone {
                members.push(member(call.conversation(), 0, 1));
            }
        } else {
            for (i, id) in [DEMO_TESS, DEMO_NOVA, DEMO_LOUP].into_iter().enumerate() {
                members.push(member(id, i, 3));
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
