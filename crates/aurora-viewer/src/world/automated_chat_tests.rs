//! #741: with the option on, the lines no person sent (objects, their IMs,
//! the viewer's notices) neither count as unread nor ask for attention;
//! people's lines still do. Off, everything counts as before.

use super::*;
use aurora_net::{ChatMessage, ChatSourceType, InstantMessage, NetEvent};

const ME: Uuid = Uuid::from_u128(1);
const TOM: Uuid = Uuid::from_u128(3);
const VENDOR: Uuid = Uuid::from_u128(0x0B1E);
const OWNER: Uuid = Uuid::from_u128(0x0B1F);

fn world(quiet: bool) -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.notifications.started = true;
    w.quiet_automated_chat = quiet;
    w
}

fn said(from: Uuid, source_type: ChatSourceType, chat_type: ChatType) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: "Someone".into(),
        source_id: from,
        owner_id: OWNER,
        source_type,
        chat_type,
        position: glam::Vec3::new(128.0, 128.0, 25.0),
        message: "hello".into(),
    })
}

fn object_im() -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: VENDOR,
        from_name: "Plaza Vendor".into(),
        to_agent_id: ME,
        dialog: 19,
        session_id: Uuid::from_u128(0x0B20),
        message: "Your gift is ready".into(),
        offline: false,
        from_group: false,
        binary_bucket: b"Aurora Demo/128/128/25\0".to_vec(),
        timestamp: 0,
    })
}

/// Everything the nearby chat receives in the scenario: a person, an
/// object's line, owner-say and llRegionSayTo, an object's IM, a notice.
fn receive_all(w: &mut World) {
    w.apply(said(TOM, ChatSourceType::Agent, ChatType::Normal));
    w.apply(said(VENDOR, ChatSourceType::Object, ChatType::Normal));
    w.apply(said(VENDOR, ChatSourceType::Object, ChatType::OwnerSay));
    w.apply(said(VENDOR, ChatSourceType::Object, ChatType::Other(9)));
    w.apply(object_im());
    w.system_notice("Teleporting…", notice_topics::Topic::Teleport);
}

#[test]
fn with_the_option_only_peoples_lines_count_and_alert() {
    let mut w = world(true);
    receive_all(&mut w);
    // every line is in the chat, one person's counts
    assert_eq!(w.chat.len(), 6);
    assert_eq!(w.chat_unread, 6, "the New line still finds where reading stopped");
    assert_eq!(w.chat_badge, 1);
    // the object's IM asked for nothing (no flash, no system notification)
    assert!(
        !w.attention.iter().any(|a| a.cause == attention::Cause::ObjectIm),
        "{:?}",
        w.attention
    );
    assert!(w.alerts(ChatKind::Local(ChatType::Normal)) && w.alerts(ChatKind::Im) && w.alerts(ChatKind::Invite(1)));
    for kind in [
        ChatKind::Object(ChatType::Normal),
        ChatKind::Object(ChatType::OwnerSay),
        ChatKind::ObjectIm,
        ChatKind::System,
    ] {
        assert!(!w.alerts(kind), "{kind:?}");
    }
    // read: nothing left
    assert_eq!(w.nearby_read(), 6);
    assert_eq!((w.chat_unread, w.chat_badge), (0, 0));
}

#[test]
fn without_the_option_every_line_counts_and_an_objects_im_flashes() {
    let mut w = world(false);
    receive_all(&mut w);
    assert_eq!(w.chat_unread, 6);
    assert_eq!(w.chat_badge, 6);
    assert!(w.attention.iter().any(|a| a.cause == attention::Cause::ObjectIm));
    assert!(w.alerts(ChatKind::Object(ChatType::Normal)) && w.alerts(ChatKind::System));
}

/// The notice keeps the topic it was said with, for its icon.
#[test]
fn a_notice_is_told_with_its_topic() {
    let mut w = world(true);
    w.system_notice("Teleporting…", notice_topics::Topic::Teleport);
    w.system_message("Settings reset");
    let topics: Vec<_> = w.chat.iter().map(|l| w.notice_topics.of(l)).collect();
    assert_eq!(topics, [notice_topics::Topic::Teleport, notice_topics::Topic::Info]);
}
