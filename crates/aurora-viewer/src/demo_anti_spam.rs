//! Synthetic floods through the same World/App paths as incoming traffic.
//! `burst`: twelve objects flood nearby chat at once, as a rezzer's
//! temporary prims do, then a resident alone 15 s later: one notice with
//! the count, then one naming the resident (`world::anti_spam::burst_window`).

use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent, SoundPreload};
use glam::Vec3;
use uuid::Uuid;

pub const DIALOG_SOURCE: Uuid = Uuid::from_u128(0xa6780001_0000_0000_0000_000000000001);
pub const INVENTORY_SOURCE: Uuid = Uuid::from_u128(0xa6780002_0000_0000_0000_000000000002);
pub const TELEPORT_SOURCE: Uuid = Uuid::from_u128(0xa6780003_0000_0000_0000_000000000003);
pub const SOUND_SOURCE: Uuid = Uuid::from_u128(0xa6780004_0000_0000_0000_000000000004);
pub const SOUND_OWNER: Uuid = Uuid::from_u128(0xa6780005_0000_0000_0000_000000000005);

/// A script dialog on its own channel: an object's dialogs replace each
/// other on the same channel by default (`world/script_dialog_limit.rs`),
/// so a flood that has to stack takes a channel per prompt.
pub fn dialog_on(source: Uuid, channel: i32) -> NetEvent {
    NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id: source,
        object_name: aurora_i18n::tr!("anti-spam-demo-object").into(),
        owner_name: aurora_i18n::tr!("anti-spam-demo-resident").into(),
        owner_id: Uuid::nil(),
        message: aurora_i18n::tr!("anti-spam-demo-prompt").into(),
        channel,
        buttons: vec![aurora_i18n::tr!("common-ok").into()],
    }
}

pub fn offer(agent: Uuid, source: Uuid, dialog: u8, number: u128) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: source,
        from_name: aurora_i18n::tr!("anti-spam-demo-resident").into(),
        to_agent_id: agent,
        dialog,
        session_id: Uuid::from_u128(0xa6780010_0000_0000_0000_000000000000 | number),
        message: aurora_i18n::tr!("anti-spam-demo-offer").into(),
        offline: false,
        from_group: false,
        binary_bucket: vec![7],
        timestamp: 0,
    })
}

pub fn sound() -> NetEvent {
    NetEvent::SoundTrigger {
        sound: super::u(0x50_0001),
        owner: SOUND_OWNER,
        object: SOUND_SOURCE,
        parent: Uuid::nil(),
        handle: super::HANDLE,
        position: Vec3::new(128.0, 128.0, 26.0),
        gain: 0.1,
    }
}

pub fn preload(object: Uuid, owner: Uuid) -> SoundPreload {
    SoundPreload {
        sound: super::u(0x50_0001),
        object,
        owner,
    }
}

/// The `burst` mode's flooding objects.
const BURST_SOURCES: u128 = 12;

/// When the `burst` mode's first wave was sent.
static BURST_AT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// A line of nearby chat from an object of the burst, or from a resident.
fn flood_line(source: Uuid, source_type: ChatSourceType, name: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: name.into(),
        source_id: source,
        owner_id: if source_type == ChatSourceType::Agent {
            source
        } else {
            Uuid::nil()
        },
        source_type,
        chat_type: ChatType::Normal,
        region: Some(crate::demo::HANDLE),
        position: Vec3::new(128.0, 128.0, 26.0),
        message: aurora_i18n::tr!("anti-spam-demo-chat").into(),
    })
}

/// The `burst` mode's first wave: each object says one line more than the
/// mode's allowance of two.
fn burst_events() -> Vec<NetEvent> {
    BURST_AT.get_or_init(std::time::Instant::now);
    let name = aurora_i18n::tr!("anti-spam-demo-object");
    (0..3)
        .flat_map(|_| {
            (0..BURST_SOURCES).map(|n| {
                flood_line(
                    Uuid::from_u128(0xa6780100_0000_0000_0000_000000000000 | n),
                    ChatSourceType::Object,
                    name,
                )
            })
        })
        .collect()
}

/// The `burst` mode's second wave, once, 15 s after the first (past the
/// 10 s window): Nova alone.
pub fn lone_events() -> Option<Vec<NetEvent>> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SENT: AtomicBool = AtomicBool::new(false);
    let at = BURST_AT.get()?;
    if at.elapsed() < std::time::Duration::from_secs(15) || SENT.swap(true, Ordering::Relaxed) {
        return None;
    }
    let name = aurora_i18n::tr!("demo-nova-example");
    Some(
        (0..3)
            .map(|_| flood_line(crate::demo::DEMO_NOVA, ChatSourceType::Agent, name))
            .collect(),
    )
}

pub fn events(agent: Uuid) -> Vec<NetEvent> {
    if std::env::var("AURORA_DEMO_ANTI_SPAM").as_deref() == Ok("burst") {
        return burst_events();
    }
    if std::env::var("AURORA_DEMO_ANTI_SPAM").as_deref() == Ok("text") {
        let mut out: Vec<_> = (0..4).map(|n| offer(agent, agent, 9, n)).collect();
        let text = format!("{}\n", aurora_i18n::tr!("anti-spam-demo-offer")).repeat(6);
        out.push(NetEvent::Chat(ChatMessage {
            from_name: aurora_i18n::tr!("anti-spam-demo-object").into(),
            source_id: DIALOG_SOURCE,
            owner_id: agent,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Normal,
            region: Some(crate::demo::HANDLE),
            position: Vec3::ZERO,
            message: text.clone(),
        }));
        if let NetEvent::InstantMessage(mut im) = offer(agent, INVENTORY_SOURCE, 0, 100) {
            im.message = text;
            out.push(NetEvent::InstantMessage(im));
        }
        return out;
    }
    let mut out = Vec::new();
    for n in 0..65 {
        if n < 15 {
            out.push(dialog_on(DIALOG_SOURCE, -678 - n as i32));
            out.push(offer(agent, INVENTORY_SOURCE, 9, n));
            out.push(offer(agent, TELEPORT_SOURCE, 22, n));
        }
        out.push(sound());
        out.push(NetEvent::PreloadSounds(vec![preload(SOUND_SOURCE, SOUND_OWNER)]));
    }
    out
}
