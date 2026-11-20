//! The last speaker Look at Last Chatter frames (`World::last_speaker`):
//! LLAgent::heardChat, from process_chat_from_simulator.

use super::*;

const ME: Uuid = Uuid::from_u128(1);
const JANE: Uuid = Uuid::from_u128(2);
const BOX: Uuid = Uuid::from_u128(3);

fn say(w: &mut World, from: Uuid, source_type: ChatSourceType, chat_type: ChatType) {
    w.apply(NetEvent::Chat(aurora_net::ChatMessage {
        from_name: "Someone".into(),
        source_id: from,
        owner_id: from,
        source_type,
        chat_type,
        region: None,
        position: Vec3::ZERO,
        message: "hello".into(),
    }));
}

/// A resident or an object heard becomes the last speaker; ourselves, the
/// system and anyone heard in do not disturb leave it as it was.
#[test]
fn the_last_speaker_is_the_last_resident_or_object_heard() {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    say(&mut w, JANE, ChatSourceType::Agent, ChatType::Normal);
    assert_eq!(w.last_speaker, Some(JANE));
    say(&mut w, ME, ChatSourceType::Agent, ChatType::Normal);
    say(&mut w, Uuid::nil(), ChatSourceType::System, ChatType::Normal);
    assert_eq!(w.last_speaker, Some(JANE));
    say(&mut w, BOX, ChatSourceType::Object, ChatType::Normal);
    assert_eq!(w.last_speaker, Some(BOX));
    w.status.dnd = true;
    say(&mut w, JANE, ChatSourceType::Agent, ChatType::Shout);
    assert_eq!(w.last_speaker, Some(BOX));
}
