//! AURORA_DEMO_REGION_MESSAGE=1: messages to the whole region in the middle
//! of the screen (#1404), the card turned on for the run without saving
//! it. Orion (the estate manager) announces a restart, then Nova a party:
//! Orion's card shows that one more message waits, Nova's comes once it is
//! closed. Each is written in nearby chat too.

use aurora_net::NetEvent;
use uuid::Uuid;

pub fn enabled() -> bool {
    std::env::var("AURORA_DEMO_REGION_MESSAGE").is_ok_and(|m| !m.is_empty() && m != "0")
}

/// An estate manager's "Send message to region": IM_NOTHING_SPECIAL to
/// nobody (`world::region_messages`).
fn message(from: Uuid, name: &str, text: &str) -> NetEvent {
    NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: Uuid::nil(),
        dialog: 0,
        session_id: Uuid::nil(),
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    })
}

/// The demo's names follow the last byte of the id: 2 is "Orion Example",
/// 4 "Nova ♪".
const ORION: Uuid = Uuid::from_u128(0x2E61_0000_0000_0000_0000_0000_0000_0002);
const NOVA: Uuid = Uuid::from_u128(0x2E61_0000_0000_0000_0000_0000_0000_0004);

/// The events of `frame`, at 60 frames a second.
pub fn events(frame: u64) -> Vec<NetEvent> {
    match frame {
        700 => vec![message(ORION, "Orion Example", aurora_i18n::tr!("demo-region-message-restart"))],
        760 => vec![message(NOVA, "Nova Example", aurora_i18n::tr!("demo-region-message-event"))],
        _ => Vec::new(),
    }
}
