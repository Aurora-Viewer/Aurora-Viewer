//! What may flash the Conversations button: a resident's message in a
//! conversation (`World::message_flashes_conversations`) and a resident's
//! line of nearby chat (`World::nearby_news`).

use super::*;

const ME: Uuid = Uuid::from_u128(1);
const JANE: Uuid = Uuid::from_u128(2);
const BOX: Uuid = Uuid::from_u128(3);
const GROUP: Uuid = Uuid::from_u128(0x6200);

fn world() -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.social.names.insert(JANE, "Jane Doe".into());
    w
}

fn say(w: &mut World, from: Uuid, source_type: ChatSourceType, chat_type: ChatType, text: &str) {
    w.apply(NetEvent::Chat(aurora_net::ChatMessage {
        from_name: "Someone".into(),
        source_id: from,
        owner_id: from,
        source_type,
        chat_type,
        position: Vec3::ZERO,
        message: text.into(),
    }));
}

/// A resident's line (said, whispered, shouted) is news for the button;
/// ours, an object's (owner say included) and the system's never are.
#[test]
fn only_a_residents_nearby_line_is_news() {
    let mut w = world();
    for (from, source, kind) in [
        (ME, ChatSourceType::Agent, ChatType::Normal),
        (BOX, ChatSourceType::Object, ChatType::Normal),
        (BOX, ChatSourceType::Object, ChatType::OwnerSay),
        (Uuid::nil(), ChatSourceType::System, ChatType::Normal),
    ] {
        say(&mut w, from, source, kind, "hello");
        assert!(!std::mem::take(&mut w.nearby_news), "{source:?} {kind:?}");
    }
    for kind in [ChatType::Normal, ChatType::Whisper, ChatType::Shout] {
        say(&mut w, JANE, ChatSourceType::Agent, kind, "hello");
        assert!(std::mem::take(&mut w.nearby_news), "{kind:?}");
    }
}

/// A message in a 1:1, a group chat or a conference flashes the button
/// (FSNotifyIMFlash), whatever the nearby chat choice; never in do not
/// disturb, nor an object's line.
#[test]
fn residents_messages_flash_the_button_except_in_do_not_disturb() {
    let mut w = world();
    for (kind, session) in [(ImKind::Private, JANE), (ImKind::Group, GROUP), (ImKind::Conference, GROUP)] {
        w.session_lines(session);
        w.conversation_push(
            session,
            ChatLine {
                time: SystemTime::now(),
                from: "Jane Doe".into(),
                text: "hi".into(),
                kind: ChatKind::Im,
                source: JANE,
                origin: Origin::Live,
            },
            true,
        );
        let m = ImMessage {
            kind,
            session,
            new_session: false,
        };
        assert!(w.message_flashes_conversations(&m), "{kind:?}");
        w.status.dnd = true;
        assert!(!w.message_flashes_conversations(&m), "{kind:?} in do not disturb");
        w.status.dnd = false;
    }
    // an object's message in its conversation does not
    w.conversation_push(
        BOX,
        ChatLine {
            time: SystemTime::now(),
            from: "Box".into(),
            text: "ping".into(),
            kind: ChatKind::Object(ChatType::Normal),
            source: BOX,
            origin: Origin::Live,
        },
        true,
    );
    let m = ImMessage {
        kind: ImKind::Private,
        session: BOX,
        new_session: false,
    };
    assert!(!w.message_flashes_conversations(&m));
}
