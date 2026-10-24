//! AURORA_DEMO_NOTIFS: the scenario of the notification center's mockup —
//! a message, a friend coming online (a toast outside the center), a
//! teleport offer and an inventory offer close together, then a burst of
//! five. `dnd` plays it with do not disturb on (the paused bell), `legacy`
//! with Firestorm's toasts (Preferences › Aurora › Notifications); `list`
//! opens the list once the burst has folded in, `history` shows its
//! History tab after answering a few cards.

use aurora_net::NetEvent;
use uuid::Uuid;

/// The scenario, if asked: `1`, `dnd`, `legacy`, `list` or `history`.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_NOTIFS").ok()
}

/// Frame at which do not disturb goes on (`dnd`).
pub const DND_AT: u64 = 640;
/// Frame at which the list under the bell opens (`list`, `history`).
pub const LIST_AT: u64 = 2700;
/// Frame at which a few cards are answered (`history`): they leave "To do".
pub const ANSWER_AT: u64 = 2650;

/// The friend who comes online (Orion in the demo).
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

fn lure(agent: Uuid) -> NetEvent {
    use crate::world::notifications::im as d;
    let invitation = format!(
        "{}\r\n{}",
        aurora_i18n::tr!("demo-come-see-the-lagoon-at-sunset"),
        crate::slurl::make(aurora_i18n::tr!("common-aurora-demo"), glam::Vec3::new(128.0, 130.0, 25.0))
    );
    im(
        agent,
        super::DEMO_TESS,
        "Tess Touch",
        d::LURE_USER,
        Uuid::from_u128(0x40C1_0000_0000_0000_0000_0000_0000_0001),
        &invitation,
        b"256000|256000|128|130|25|0|1|0|PG",
    )
}

fn gift(agent: Uuid, n: u128) -> NetEvent {
    use crate::world::notifications::im as d;
    // a resident's offer: the asset type then the item already delivered
    let mut bucket = vec![6u8];
    bucket.extend_from_slice(Uuid::from_u128(0x6170_0000_0000_0000_0000_0000_0000_0000 | n).as_bytes());
    im(
        agent,
        friend(),
        "Orion Exemple",
        d::INVENTORY_OFFERED,
        Uuid::from_u128(0x6170_0000_0000_0000_0000_0000_0000_1000 | n),
        aurora_i18n::tr!("demo-notifs-lantern"),
        &bucket,
    )
}

fn notice(agent: Uuid, text: &str) -> NetEvent {
    use crate::world::notifications::im as d;
    im(agent, Uuid::nil(), "Second Life", d::MESSAGEBOX, Uuid::nil(), text, b"")
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_NOVA, DEMO_TESS};
    if let Some(mode) = mode().filter(|m| {
        matches!(
            m.as_str(),
            "object" | "object-own" | "legacy-object" | "object-group" | "object-history"
        )
    }) {
        return if frame == 700 {
            let mut event = im(
                agent,
                if mode == "object-own" {
                    agent
                } else if mode == "object-group" {
                    super::DEMO_GROUP1
                } else {
                    friend()
                },
                "Aurora gift box",
                crate::world::notifications::im::TASK_INVENTORY_OFFERED,
                Uuid::from_u128(0x6170_0000_0000_0000_0000_0000_0000_2000),
                aurora_i18n::tr!("demo-notifs-lantern"),
                &[6],
            );
            if mode == "object-group"
                && let NetEvent::InstantMessage(im) = &mut event
            {
                im.from_group = true;
            }
            vec![event]
        } else {
            Vec::new()
        };
    }
    // the mockup's times from frame 700, at 60 frames a second
    match frame {
        // Orion goes offline during the loading screen (no toast then), to
        // come back online later
        300 => vec![NetEvent::FriendsOnline {
            ids: vec![friend()],
            online: false,
        }],
        700 => vec![im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            // AURORA_DEMO_APPLINKS=preview: its links on the card (#947)
            &if super::applinks_preview_scene() {
                super::applinks_text(None)
            } else {
                aurora_i18n::tr!("demo-toast-hello").to_owned()
            },
            b"",
        )],
        1190 => vec![NetEvent::FriendsOnline {
            ids: vec![friend()],
            online: true,
        }],
        1270 => vec![lure(agent)],
        1325 => vec![gift(agent, 1)],
        // the burst of five, 0.45 s apart
        2110 => vec![im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-toast-are-you-there"),
            b"",
        )],
        2137 => vec![notice(agent, aurora_i18n::tr!("demo-notifs-group-notice"))],
        2164 => vec![notice(agent, aurora_i18n::tr!("demo-notifs-paid"))],
        2191 => vec![gift(agent, 2)],
        2218 => vec![im(
            agent,
            DEMO_TESS,
            "Tess Touch",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-toast-hello"),
            b"",
        )],
        _ => Vec::new(),
    }
}
