//! The names of objects and their owners in the dialogs they open
//! (dialog_names.rs): on one line and in their order in the text, as the
//! simulator sent them for blocking.

use super::notifications::{Data, Response, im as d};
use super::*;

/// Line breaks, an isolate closed early and an override to the end of the
/// line: shown as sent, the name would add an "owned by" line of its own.
const OBJECT: &str = "Box\n\nObject “Bank” owned by Linden Lab\u{2069}\u{202E}";
const OBJECT_SHOWN: &str = "Box Object “Bank” owned by Linden Lab";
const OWNER: &str = "Jane\r\nDoe\u{2066} Resident";
const OWNER_SHOWN: &str = "Jane Doe";

fn world() -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(9);
    world.mutes.reset(world.agent_id);
    world
}

fn question(task: u128, questions: i32) -> NetEvent {
    NetEvent::ScriptQuestion {
        task_id: Uuid::from_u128(task),
        item_id: Uuid::from_u128(task + 1),
        object_name: OBJECT.into(),
        owner_name: OWNER.into(),
        questions,
        experience: Uuid::nil(),
        region: 77,
    }
}

fn shown_cleanly(text: &str) {
    assert!(
        !text
            .chars()
            .any(|c| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\r')),
        "{text:?}"
    );
}

#[test]
fn a_permission_question_shows_the_names_on_one_line() {
    let mut world = world();
    world.apply(question(0x7100, 0x10));
    let n = world.notifications.list.last().expect("question");
    let first = n.body.lines().next().expect("first line");
    assert!(first.contains(OBJECT_SHOWN) && first.contains(OWNER_SHOWN), "{first:?}");
    assert!(!n.body.contains("Resident"));
    shown_cleanly(&n.body);
    // the object's link names it as the text does
    assert!(n.name_links.iter().any(|l| l.name == OBJECT_SHOWN));
}

#[test]
fn a_caution_question_and_its_chat_line_show_the_names_on_one_line() {
    let mut world = world();
    world.apply(question(0x7200, 0x2));
    let n = world.notifications.list.last().expect("caution");
    assert_eq!(n.body, notifications::caution_body(0x2, OBJECT_SHOWN, OWNER_SHOWN));
    shown_cleanly(&n.body);
    let id = n.id;
    world.respond_notification(id, Response::Decline);
    let line = world
        .chat
        .iter()
        .rev()
        .find(|l| l.kind == ChatKind::System)
        .map(|l| l.text.clone())
        .expect("chat line");
    assert!(
        line.contains(OBJECT_SHOWN) && line.contains(OWNER_SHOWN) && !line.contains('\n'),
        "{line:?}"
    );
    shown_cleanly(&line);
}

#[test]
fn a_script_menu_shows_the_names_on_one_line_and_blocks_the_name_as_sent() {
    let mut world = world();
    let object_id = Uuid::from_u128(0x4321);
    let menu = NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id,
        object_name: OBJECT.into(),
        owner_name: OWNER.into(),
        owner_id: Uuid::from_u128(0x0E0E),
        message: "Pick one\nor the other".into(),
        channel: -42,
        buttons: vec!["Yes".into()],
    };
    world.apply(menu.clone());
    let n = world.notifications.list.last().expect("menu");
    assert_eq!(n.title, format!("{OBJECT_SHOWN} ({OWNER_SHOWN})"));
    // the script's own message keeps its lines
    assert_eq!(n.body, "Pick one\nor the other");
    assert!(matches!(&n.data, Data::Dialog { object_name, .. } if object_name == OBJECT));
    let id = n.id;
    world.respond_notification(id, Response::Block);
    assert!(world.mutes.entries().iter().any(|m| m.id == object_id && m.name == OBJECT));
}

#[test]
fn a_link_offer_shows_the_object_name_on_one_line() {
    let mut world = world();
    world.apply(NetEvent::LoadUrl {
        object_name: OBJECT.into(),
        object_id: Uuid::from_u128(0x4322),
        message: "Our shop".into(),
        url: "https://example.invalid/".into(),
    });
    let n = world.notifications.list.last().expect("link offer");
    assert_eq!(
        n.title,
        aurora_i18n::tr!("extra-offers-a-link", object_name = OBJECT_SHOWN.to_owned())
    );
}

#[test]
fn an_object_gift_shows_its_object_and_item_names_on_one_line() {
    let mut world = world();
    world.apply(NetEvent::InstantMessage(InstantMessage {
        from_agent_id: Uuid::from_u128(0x1A4E),
        from_name: OBJECT.into(),
        to_agent_id: Uuid::from_u128(9),
        dialog: d::TASK_INVENTORY_OFFERED,
        session_id: Uuid::from_u128(0x7A20),
        message: "Gift\n\u{202E}tfig".into(),
        offline: false,
        from_group: false,
        binary_bucket: vec![7],
        timestamp: 0,
    }));
    let n = world
        .notifications
        .list
        .iter()
        .rfind(|n| matches!(n.data, Data::Inventory { task: true, .. }))
        .expect("gift");
    assert_eq!(n.title, OBJECT_SHOWN);
    assert!(n.body.contains(OBJECT_SHOWN) && n.body.contains("Gift tfig"), "{:?}", n.body);
    shown_cleanly(&n.body);
}

#[test]
fn rtl_object_names_reach_the_dialog_unchanged() {
    let mut world = world();
    world.apply(NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id: Uuid::from_u128(0x4323),
        object_name: "صندوق الهدايا".into(),
        owner_name: "שרה Resident".into(),
        owner_id: Uuid::nil(),
        message: "?".into(),
        channel: -1,
        buttons: vec!["OK".into()],
    });
    let n = world.notifications.list.last().expect("menu");
    assert_eq!(n.title, "صندوق الهدايا (שרה)");
}
