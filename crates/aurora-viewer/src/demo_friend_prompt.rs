//! AURORA_DEMO_FRIEND_PROMPT=1: the friendship offer in the middle of the
//! screen (#465), turned on for the run without saving it. Nova offers
//! friendship with a message, then Tess without one: Nova's card shows
//! that one more offer waits, Tess's comes once Nova's is answered or put
//! off (`later`: put off at `LATER_AT`).

use aurora_net::NetEvent;
use uuid::Uuid;

/// The scenario, if asked.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_FRIEND_PROMPT")
        .ok()
        .filter(|m| !m.is_empty() && m != "0")
}

/// Frame at which `later` puts Nova's offer off.
pub const LATER_AT: u64 = 900;

/// IM_FRIENDSHIP_OFFERED (llinstantmessage.h): its session id is the
/// transaction the answer carries.
fn offer(agent: Uuid, from: Uuid, name: &str, tx: u128, text: &str) -> NetEvent {
    NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: agent,
        dialog: crate::world::notifications::im::FRIENDSHIP_OFFERED,
        session_id: Uuid::from_u128(0xF41E_0000_0000_0000_0000_0000_0000_0000 | tx),
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    })
}

/// Residents who are not friends yet. The demo's display names follow the
/// last byte of the id: 4 is "Nova ♪", 7 "Tess ✨".
const NOVA: Uuid = Uuid::from_u128(0xF41E_0000_0000_0000_0000_0000_0000_0004);
const TESS: Uuid = Uuid::from_u128(0xF41E_0000_0000_0000_0000_0000_0000_0007);

/// The events of `frame`, at 60 frames a second.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    match frame {
        700 => vec![offer(
            agent,
            NOVA,
            "Nova Example",
            1,
            aurora_i18n::tr!("demo-friend-prompt-message"),
        )],
        760 => vec![offer(agent, TESS, "Tess Touch", 2, "")],
        _ => Vec::new(),
    }
}
