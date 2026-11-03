//! AURORA_DEMO_IM_ANNOUNCE: from frame 700, every 600 frames, Nova then Tess
//! in turn start typing an IM to us, and send it 300 frames later. `1` turns
//! on "Announce incoming IMs as soon as the sender starts typing" and "Show
//! the number of unread IMs in the window title" for the run (never saved);
//! `prefs` leaves them to Preferences › Chat. A conversation closed is
//! announced again at its resident's next turn.

use aurora_net::{InstantMessage, NetEvent};
use uuid::Uuid;

/// The first typing start.
const START: u64 = 700;
/// One resident's turn: typing, then the message.
const TURN: u64 = 600;
/// From the typing start to the message.
const MESSAGE_AFTER: u64 = 300;

fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_IM_ANNOUNCE").ok()
}

/// The scenario runs.
pub fn active() -> bool {
    mode().is_some()
}

/// Both settings on for the run (every mode but `prefs`).
pub fn forced() -> bool {
    mode().is_some_and(|m| m != "prefs")
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_NOVA, DEMO_TESS};
    let Some(since) = frame.checked_sub(START) else {
        return Vec::new();
    };
    let (from, name) = if (since / TURN).is_multiple_of(2) {
        (DEMO_NOVA, "Nova Exemple")
    } else {
        (DEMO_TESS, "Tess Touch")
    };
    let (dialog, message) = match since % TURN {
        0 => (crate::world::IM_TYPING_START, "typing".to_owned()),
        MESSAGE_AFTER => (0, aurora_i18n::tr!("demo-im-announce-message").to_owned()),
        _ => return Vec::new(),
    };
    vec![NetEvent::InstantMessage(InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: agent,
        dialog,
        session_id: Uuid::from_u128(agent.as_u128() ^ from.as_u128()),
        message,
        offline: false,
        from_group: false,
        binary_bucket: vec![0],
        timestamp: 0,
    })]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogs(frame: u64) -> Vec<(Uuid, u8)> {
        events(frame, Uuid::from_u128(1))
            .into_iter()
            .filter_map(|e| match e {
                NetEvent::InstantMessage(im) => Some((im.from_agent_id, im.dialog)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nova_then_tess_type_then_send() {
        use crate::demo::{DEMO_NOVA, DEMO_TESS};
        use crate::world::IM_TYPING_START;
        assert!(dialogs(START - 1).is_empty());
        assert_eq!(dialogs(START), vec![(DEMO_NOVA, IM_TYPING_START)]);
        assert_eq!(dialogs(START + MESSAGE_AFTER), vec![(DEMO_NOVA, 0)]);
        assert_eq!(dialogs(START + TURN), vec![(DEMO_TESS, IM_TYPING_START)]);
        assert_eq!(dialogs(START + TURN + MESSAGE_AFTER), vec![(DEMO_TESS, 0)]);
        assert_eq!(dialogs(START + 2 * TURN), vec![(DEMO_NOVA, IM_TYPING_START)]);
        assert!(dialogs(START + 1).is_empty());
    }
}
