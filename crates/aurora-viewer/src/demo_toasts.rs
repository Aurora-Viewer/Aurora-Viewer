//! AURORA_DEMO_TOASTS: a friend going offline then online, two messages of
//! one resident, a group chat message, then the list under the bell (`1`:
//! the messages toast only, `bell`: they stay listed) and, with `click` /
//! `minimized`, a click on the resident's entry (the Conversations window
//! closed / minimized before it). The Conversations button flashes while
//! the window is closed, its bar while it is minimized.

use aurora_net::NetEvent;
use uuid::Uuid;

/// The scenario, if asked: `1`, `bell`, `click` or `minimized`.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_TOASTS").ok()
}

/// Frame at which the list under the bell opens.
pub const LIST_AT: u64 = 1100;
/// Frame of the click on the resident's entry (`click`, `minimized`).
pub const CLICK_AT: u64 = 1250;

/// The friend whose status changes (Orion in the demo).
fn friend() -> Uuid {
    Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | 102)
}

fn im(agent: Uuid, from: Uuid, name: &str, dialog: u8, session: Uuid, text: &str, bucket: &[u8]) -> NetEvent {
    NetEvent::InstantMessage(aurora_net::InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: agent,
        dialog,
        session_id: session,
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket.to_vec(),
        timestamp: 0,
    })
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_GROUP1, DEMO_NOVA, DEMO_TESS};
    match frame {
        700 => vec![NetEvent::FriendsOnline {
            ids: vec![friend()],
            online: false,
        }],
        760 => vec![NetEvent::FriendsOnline {
            ids: vec![friend()],
            online: true,
        }],
        820 => vec![im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-toast-hello"),
            b"",
        )],
        880 => vec![im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-toast-are-you-there"),
            b"",
        )],
        940 => vec![im(
            agent,
            DEMO_TESS,
            "Tess Touch",
            17,
            DEMO_GROUP1,
            aurora_i18n::tr!("demo-toast-group-message"),
            b"Aurora Builders\0",
        )],
        _ => Vec::new(),
    }
}
