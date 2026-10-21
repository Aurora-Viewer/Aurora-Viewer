//! Synthetic floods through the same World/App paths as incoming traffic.

use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent, SoundPreload};
use glam::Vec3;
use uuid::Uuid;

pub const DIALOG_SOURCE: Uuid = Uuid::from_u128(0xa6780001_0000_0000_0000_000000000001);
pub const INVENTORY_SOURCE: Uuid = Uuid::from_u128(0xa6780002_0000_0000_0000_000000000002);
pub const TELEPORT_SOURCE: Uuid = Uuid::from_u128(0xa6780003_0000_0000_0000_000000000003);
pub const SOUND_SOURCE: Uuid = Uuid::from_u128(0xa6780004_0000_0000_0000_000000000004);
pub const SOUND_OWNER: Uuid = Uuid::from_u128(0xa6780005_0000_0000_0000_000000000005);

pub fn dialog(source: Uuid) -> NetEvent {
    NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id: source,
        object_name: aurora_i18n::tr!("anti-spam-demo-object").into(),
        owner_name: aurora_i18n::tr!("anti-spam-demo-resident").into(),
        message: aurora_i18n::tr!("anti-spam-demo-prompt").into(),
        channel: -678,
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

pub fn events(agent: Uuid) -> Vec<NetEvent> {
    if std::env::var("AURORA_DEMO_ANTI_SPAM").as_deref() == Ok("text") {
        let mut out: Vec<_> = (0..4).map(|n| offer(agent, agent, 9, n)).collect();
        let text = format!("{}\n", aurora_i18n::tr!("anti-spam-demo-offer")).repeat(6);
        out.push(NetEvent::Chat(ChatMessage {
            from_name: aurora_i18n::tr!("anti-spam-demo-object").into(),
            source_id: DIALOG_SOURCE,
            owner_id: agent,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Normal,
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
            out.push(dialog(DIALOG_SOURCE));
            out.push(offer(agent, INVENTORY_SOURCE, 9, n));
            out.push(offer(agent, TELEPORT_SOURCE, 22, n));
        }
        out.push(sound());
        out.push(NetEvent::PreloadSounds(vec![preload(SOUND_SOURCE, SOUND_OWNER)]));
    }
    out
}
