//! AURORA_DEMO_PRESENCE=<place>: the friends' online / offline toasts of
//! Aurora's notification center at one of their six places (`top-left`,
//! `top-center`, `top-right`, `bottom-left`, `bottom-center`,
//! `bottom-right`), for this run only. Three nearby chat lines, then three
//! friends' toasts close together; then an IM card under the bell with a
//! friend arriving during it (it waits at the top right and shows once the
//! card has folded) and another one too early to wait that long (dropped);
//! then the bell's list open while a friend arrives (shown when it closes),
//! and open again for too long (dropped).

use aurora_net::NetEvent;
use uuid::Uuid;

/// The place asked, if any.
pub fn position() -> Option<crate::settings::PresencePosition> {
    let mode = std::env::var("AURORA_DEMO_PRESENCE").ok()?;
    crate::settings::PresencePosition::from_word(mode.split(':').next().unwrap_or_default().trim())
}

/// Frames at which the bell's list opens and closes: a friend arrives in
/// between, shown at the close; then a longer opening that outlasts the
/// wait of the next one.
pub const LIST: [(u64, u64); 2] = [(1500, 1600), (1700, 1990)];

fn friend(n: u128) -> Uuid {
    Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | n)
}

fn status(n: u128, online: bool) -> Vec<NetEvent> {
    vec![NetEvent::FriendsOnline {
        ids: vec![friend(n)],
        online,
    }]
}

fn say(name: &str, source: Uuid, text: &str) -> NetEvent {
    NetEvent::Chat(aurora_net::ChatMessage {
        from_name: name.into(),
        source_id: source,
        owner_id: Uuid::nil(),
        source_type: aurora_net::ChatSourceType::Agent,
        chat_type: aurora_net::ChatType::Normal,
        position: glam::Vec3::ZERO,
        message: text.into(),
    })
}

/// The events of `frame`, at 60 frames a second.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_NOVA, DEMO_TESS};
    match frame {
        // two friends go offline during the loading screen (no toast then),
        // to come back online later
        300 => [status(101, false), status(103, false)].concat(),
        // the nearby chat lines over the chat bar (the toasts at the bottom
        // left stack above them)
        640 => vec![
            say("Nova Exemple", DEMO_NOVA, aurora_i18n::tr!("demo-toast-hello")),
            say("Tess Touch", DEMO_TESS, aurora_i18n::tr!("demo-toast-are-you-there")),
            say("Nova Exemple", DEMO_NOVA, aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset")),
        ],
        700 => status(101, true),
        730 => status(103, true),
        760 => status(102, false),
        // an IM card under the bell (6 s, folded by frame 1380)
        1000 => vec![NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_NOVA,
            from_name: "Nova Exemple".into(),
            to_agent_id: agent,
            dialog: 0,
            session_id: Uuid::nil(),
            message: aurora_i18n::tr!("demo-toast-are-you-there").into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        })],
        // at the top right: waits 3.5 s, the card still out, dropped
        1060 => status(100, false),
        // at the top right: waits, shown once the card has folded
        1200 => status(102, true),
        // during the first opening of the list: shown when it closes
        1540 => status(103, false),
        // during the second, longer one: dropped
        1720 => status(101, false),
        _ => Vec::new(),
    }
}
