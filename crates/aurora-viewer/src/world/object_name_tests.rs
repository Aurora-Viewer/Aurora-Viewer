//! FSMarkObjects: an object with an empty name speaks as "(no name)" when
//! the option is on, and with no name at all when it is off, as in the
//! Legacy Viewer's process_chat_from_simulator.

use super::*;
use aurora_net::{ChatMessage, ChatSourceType, NetEvent};

const ME: Uuid = Uuid::from_u128(1);
const TOM: Uuid = Uuid::from_u128(3);
const BOX: Uuid = Uuid::from_u128(0x0B1E);

fn said(from: Uuid, name: &str, source_type: ChatSourceType) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: name.into(),
        source_id: from,
        owner_id: TOM,
        source_type,
        chat_type: ChatType::Normal,
        region: Some(crate::demo::HANDLE),
        position: glam::Vec3::new(128.0, 128.0, 25.0),
        message: "Tom Resident: bye all".into(),
    })
}

fn from_of(mark: bool, event: NetEvent) -> String {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.notifications.started = true;
    w.mark_unnamed_objects = mark;
    w.apply(event);
    w.chat.iter().last().expect("a line").from.clone()
}

#[test]
fn an_unnamed_object_is_marked_only_with_the_option() {
    let marked = aurora_i18n::tr!("world-chat-unnamed-object");
    for name in ["", "   ", "\t"] {
        assert_eq!(from_of(true, said(BOX, name, ChatSourceType::Object)), marked, "{name:?} marked");
        assert_eq!(from_of(false, said(BOX, name, ChatSourceType::Object)), "", "{name:?} left empty");
    }
    // a named object keeps its name, spaces included
    assert_eq!(from_of(true, said(BOX, " Door ", ChatSourceType::Object)), " Door ");
    assert_eq!(object_speaker_name("Door".into(), true), "Door");
    // a resident's name never becomes "(no name)"
    assert_ne!(from_of(true, said(TOM, "", ChatSourceType::Agent)), marked);
}
