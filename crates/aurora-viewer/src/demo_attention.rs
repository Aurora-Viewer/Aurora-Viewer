//! AURORA_DEMO_ATTENTION: the window goes to the taskbar, then a message, a
//! mention in nearby chat, a mention in a group chat, an object's message,
//! a teleport offer and a script dialog arrive one second apart: each
//! flashes the taskbar button and, with the system notifications (turned on
//! for the scenario), shows a Windows notification. `front` keeps the window
//! in front (no flash, no notification unless "Even when Aurora is in front");
//! `capture` brings the window back at frame 1150, for a screenshot.
//! `badge` feeds the Linux launcher badge and KDE flash (turned on for the
//! run, the settings untouched): two residents' IMs and a teleport offer
//! arrive, then the conversations and the notices are read one after the
//! other. The window is left alone: put it behind another to see the flash.

use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent};
use uuid::Uuid;

/// The scenario, if asked: `1`, `front`, `capture` or `badge`.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_ATTENTION").ok()
}

/// Frame at which the window goes to the taskbar (`1`).
pub const MINIMIZE_AT: u64 = 700;
/// Frame at which `capture` brings the window back (the arrivals are at
/// 760, 820, 880, 940, 1000 and 1060).
pub const RESTORE_AT: u64 = 1150;

const OBJECT: Uuid = Uuid::from_u128(0x0A77_E000_0000_0000_0000_0000_0000_0001);

fn im(agent: Uuid, from: Uuid, name: &str, dialog: u8, session: Uuid, text: &str, bucket: &[u8]) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
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

fn mention(agent: Uuid, text: &str) -> String {
    format!("secondlife:///app/agent/{agent}/mention {text}")
}

/// `badge`: frame at which the first conversation is read, and the one at
/// which the rest is (the arrivals are at 760, 820, 880 and 940).
pub const BADGE_READ_FIRST: u64 = 1100;
pub const BADGE_READ_ALL: u64 = 1160;

/// The events of `frame` for `badge`: the count rises to 4 (three unread
/// messages, one offer).
pub fn badge_events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_NOVA, DEMO_TESS};
    let session = |other: Uuid| Uuid::from_u128(agent.as_u128() ^ other.as_u128());
    let event = match frame {
        760 | 820 => im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            session(DEMO_NOVA),
            aurora_i18n::tr!("demo-attention-im"),
            b"",
        ),
        880 => im(
            agent,
            DEMO_TESS,
            "Tess Touch",
            0,
            session(DEMO_TESS),
            aurora_i18n::tr!("demo-attention-im"),
            b"",
        ),
        940 => im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            crate::world::notifications::im::LURE_USER,
            Uuid::from_u128(0x0A77_E000_0000_0000_0000_0000_0000_0002),
            aurora_i18n::tr!("demo-attention-teleport"),
            b"",
        ),
        _ => return Vec::new(),
    };
    vec![event]
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_GROUP1, DEMO_NOVA, DEMO_TESS};
    let event = match frame {
        760 => im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-attention-im"),
            b"",
        ),
        820 => NetEvent::Chat(ChatMessage {
            from_name: "Tess Touch".into(),
            source_id: DEMO_TESS,
            owner_id: DEMO_TESS,
            source_type: ChatSourceType::Agent,
            chat_type: ChatType::Normal,
            region: Some(crate::demo::HANDLE),
            position: glam::Vec3::new(130.0, 130.0, 22.0),
            message: mention(agent, aurora_i18n::tr!("demo-attention-mention")),
        }),
        880 => im(
            agent,
            DEMO_TESS,
            "Tess Touch",
            17,
            DEMO_GROUP1,
            &mention(agent, aurora_i18n::tr!("demo-attention-group")),
            b"Aurora Builders\0",
        ),
        940 => {
            let location = format!("{}/128/128/25\0", aurora_i18n::tr!("common-aurora-demo"));
            im(
                agent,
                agent,
                aurora_i18n::tr!("demo-attention-object-name"),
                19,
                OBJECT,
                aurora_i18n::tr!("demo-attention-object"),
                location.as_bytes(),
            )
        }
        1000 => im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            crate::world::notifications::im::LURE_USER,
            Uuid::from_u128(0x0A77_E000_0000_0000_0000_0000_0000_0002),
            aurora_i18n::tr!("demo-attention-teleport"),
            b"",
        ),
        1060 => NetEvent::ScriptDialog {
            sender: ([127, 0, 0, 1], 13000).into(),
            object_id: OBJECT,
            object_name: aurora_i18n::tr!("demo-attention-object-name").into(),
            owner_name: "Nova Exemple".into(),
            owner_id: Uuid::nil(),
            message: aurora_i18n::tr!("demo-attention-dialog").into(),
            channel: -4242,
            buttons: vec!["OK".into()],
        },
        _ => return Vec::new(),
    };
    vec![event]
}
