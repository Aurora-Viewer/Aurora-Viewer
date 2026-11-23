use super::*;
use aurora_net::{ChatMessage, ChatSourceType, NetEvent};
use std::sync::Arc;

const ME: Uuid = Uuid::from_u128(1);
const NEIGHBOUR: Uuid = Uuid::from_u128(2);

fn test_world() -> World {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = ME;
    world.notifications.started = true;
    world
}

fn chat(source_id: Uuid, source_type: ChatSourceType, from_name: &str, message: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: from_name.to_owned(),
        source_id,
        owner_id: Uuid::nil(),
        source_type,
        chat_type: ChatType::Normal,
        region: Some(crate::demo::HANDLE),
        position: glam::Vec3::new(128.0, 128.0, 25.0),
        message: message.to_owned(),
    })
}

#[test]
fn extension_input_is_only_accepted_other_resident_chat_and_is_sanitized() {
    let mut world = test_world();
    world.apply(chat(NEIGHBOUR, ChatSourceType::Agent, "Some\u{1}one", "hel\u{1}lo"));
    world.apply(chat(ME, ChatSourceType::Agent, "Me", "my line"));
    world.apply(chat(Uuid::from_u128(3), ChatSourceType::Object, "Object", "object line"));

    let events = world.take_extension_chat_events();
    assert_eq!(events.len(), 1);
    let request = &events[0];
    request.validate().expect("host event satisfies API v1 bounds");
    let ExtensionEvent::NearbyChat { message_id, speaker, text } = &request.event else {
        panic!("nearby chat event expected");
    };
    assert_eq!(*message_id, request.request_id);
    assert_eq!(speaker, "Someone");
    assert_eq!(text, "hello");
    let json = serde_json::to_string(request).expect("request JSON");
    assert!(
        !json.contains(&NEIGHBOUR.to_string()),
        "resident UUID must not cross the extension boundary"
    );
}

#[test]
fn extension_annotation_is_bound_to_its_live_nearby_line() {
    let mut world = test_world();
    world.apply(chat(NEIGHBOUR, ChatSourceType::Agent, "Someone", "hello"));
    let event = world.take_extension_chat_events().pop().expect("accepted nearby event");
    let ExtensionEvent::NearbyChat { message_id, .. } = event.event else {
        panic!("nearby event expected")
    };

    assert!(world.annotate_extension_chat(message_id, "dev.example.helper".into(), "separate note".into()));
    let line = world.chat.back().expect("nearby line retained");
    let annotations = world.extension_annotations(line);
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0], ("dev.example.helper".into(), "separate note".into()));
    assert!(!world.annotate_extension_chat(message_id + 1, "dev.example.helper".into(), "stale".into()));
}

#[test]
fn clearing_nearby_chat_drops_queued_extension_events_and_annotations() {
    let mut world = test_world();
    world.apply(chat(NEIGHBOUR, ChatSourceType::Agent, "Someone", "hello"));
    let event = world.take_extension_chat_events().pop().expect("accepted nearby event");
    let ExtensionEvent::NearbyChat { message_id, .. } = event.event else {
        panic!("nearby event expected")
    };
    assert!(world.annotate_extension_chat(message_id, "dev.example.helper".into(), "note".into()));

    world.clear_nearby_chat();
    assert!(world.take_extension_chat_events().is_empty());
    assert!(!world.annotate_extension_chat(message_id, "dev.example.helper".into(), "late".into()));
}
