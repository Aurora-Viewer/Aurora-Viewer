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

/// The location kept with the newest chat line of object `id`.
fn origin(w: &World, id: Uuid) -> String {
    let line = w.chat.iter().rev().find(|l| l.source == id).expect("a line of the object");
    w.chat_object(line).expect("the line's origin").slurl.clone()
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

/// IM_FROM_TASK (LLIMProcessing::processNewMessage): an object's IM line
/// belongs to the object, its IM's session id, not to its owner; the
/// inspector of its name gets the owner and the bucket's location. With the
/// owner's id, Inspect showed no object and Block muted the owner.
#[test]
fn an_object_ims_line_belongs_to_the_object_and_names_its_owner() {
    let mut w = world(false);
    w.apply(object_im());
    let object = Uuid::from_u128(0x0B20);
    let line = w.chat.back().expect("the object's IM line");
    assert_eq!((line.kind, line.source), (ChatKind::ObjectIm, object));
    let o = w.chat_object(line).expect("the line's origin");
    assert_eq!((o.id, o.owner, o.group_owned), (object, VENDOR, false));
    assert_eq!(o.name, "Plaza Vendor");
    assert_eq!(o.slurl, format!("{}/128/128/25", crate::slurl::encode("Aurora Demo")));
    // a region without the task id: no object to inspect or block
    let NetEvent::InstantMessage(mut older) = object_im() else {
        unreachable!()
    };
    older.session_id = Uuid::nil();
    w.apply(NetEvent::InstantMessage(older));
    assert!(w.chat.back().unwrap().source.is_nil());
    assert_eq!(w.chat_objects.len(), 1);
}

/// RLVa @shownames: the inspector of an object's name shows a resident
/// owner anonymous, with no profile link; a group owner stays named.
#[test]
fn an_objects_owner_is_anonymous_in_its_inspector_under_shownames() {
    let mut w = world(false);
    w.rlva_configure(true);
    let mut o = crate::slapp::RemoteObject {
        id: Uuid::from_u128(0x0B20),
        name: "Plaza Vendor".into(),
        owner: VENDOR,
        group_owned: false,
        slurl: String::new(),
    };
    assert_eq!(crate::ui::remote_object::hidden_owner(&w, &o), None);
    let restriction = ChatMessage {
        from_name: "RLVa test object".into(),
        source_id: Uuid::from_u128(0x0C01),
        owner_id: ME,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        position: glam::Vec3::ZERO,
        message: "@shownames=n".into(),
    };
    assert!(w.rlva.receive(&restriction));
    assert_eq!(
        crate::ui::remote_object::hidden_owner(&w, &o).as_deref(),
        Some(rlva::privacy::anonymous(VENDOR).display_name.as_str())
    );
    o.group_owned = true;
    assert_eq!(crate::ui::remote_object::hidden_owner(&w, &o), None);
}

/// RLVa @showloc: the location of an object's IM or nearby chat line from
/// one of our regions is masked when it arrives, as in the Legacy Viewer.
/// The decision stays with the kept origin: neither a region change nor the
/// restriction's end reveals it; a line from elsewhere keeps its location.
#[test]
fn an_objects_location_masked_under_showloc_stays_masked_after_a_region_change() {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    for event in crate::demo::events() {
        w.apply(event);
    }
    w.rlva_configure(true);
    let here = w.region_name();
    assert!(!here.is_empty());
    let owner_say = |w: &World, message: &str| {
        NetEvent::Chat(ChatMessage {
            from_name: "RLVa test object".into(),
            source_id: Uuid::from_u128(0x0C01),
            owner_id: w.agent_id,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: message.into(),
        })
    };
    w.apply(owner_say(&w, "@showloc=n"));
    assert!(w.rlva_private_region(&here));
    let im = |session: u128, region: &str| {
        NetEvent::InstantMessage(InstantMessage {
            from_agent_id: VENDOR,
            from_name: "Plaza Vendor".into(),
            to_agent_id: w.agent_id,
            dialog: 19,
            session_id: Uuid::from_u128(session),
            message: "Your gift is ready".into(),
            offline: false,
            from_group: false,
            binary_bucket: format!("{region}/128/128/25\0").into_bytes(),
            timestamp: 0,
        })
    };
    let (nearby_im, far_im, speaker) = (Uuid::from_u128(0x0B21), Uuid::from_u128(0x0B22), Uuid::from_u128(0x0B23));
    let (a, b) = (im(0x0B21, &here), im(0x0B22, "Far Away"));
    w.apply(a);
    w.apply(b);
    w.apply(said(speaker, ChatSourceType::Object, ChatType::Normal));
    assert_eq!(origin(&w, nearby_im), "", "an object IM from our region");
    assert_eq!(origin(&w, speaker), "", "an object's nearby chat");
    assert_eq!(
        origin(&w, far_im),
        format!("{}/128/128/25", crate::slurl::encode("Far Away")),
        "a region we are not in stays shown"
    );
    // the old region leaves the list while the restriction holds
    w.regions.clear();
    assert!(!w.rlva_private_region(&here));
    assert_eq!(origin(&w, nearby_im), "");
    assert_eq!(origin(&w, speaker), "");
    // nor does the end of the restriction bring it back
    w.apply(owner_say(&w, "@clear"));
    assert!(!w.rlva.has("showloc"));
    assert_eq!(origin(&w, nearby_im), "");
    assert_eq!(origin(&w, speaker), "");
}

/// LLWorld::getRegionFromPosAgent: an object's nearby chat line takes the
/// region under its position, a neighbour's included, with the position
/// within it; outside every region we know, no location rather than ours.
#[test]
fn an_objects_chat_origin_is_the_region_under_its_position() {
    let mut w = world(false);
    let here: RegionHandle = (1000 * 256) << 32 | (1000 * 256);
    let east: RegionHandle = (1001 * 256) << 32 | (1000 * 256);
    w.set_main_region(here);
    w.ensure_region(here, (256, 256)).name = "Home Region".into();
    w.ensure_region(east, (256, 256)).name = "East Region".into();
    let at = |id: u128, x: f32| {
        NetEvent::Chat(ChatMessage {
            from_name: "Speaker".into(),
            source_id: Uuid::from_u128(id),
            owner_id: OWNER,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Normal,
            position: glam::Vec3::new(x, 40.0, 22.0),
            message: "hello".into(),
        })
    };
    w.apply(at(1, 100.0));
    w.apply(at(2, 270.0));
    w.apply(at(3, 600.0));
    let slurl = |id: u128| origin(&w, Uuid::from_u128(id));
    assert_eq!(slurl(1), format!("{}/100/40/22", crate::slurl::encode("Home Region")));
    assert_eq!(slurl(2), format!("{}/14/40/22", crate::slurl::encode("East Region")));
    assert_eq!(slurl(3), "", "no region known there");
}

/// Each line keeps its own origin, as each Legacy Viewer line carries its
/// own `objectim` SLURL: an object that moves and speaks again does not
/// move its earlier line's location.
#[test]
fn each_object_chat_line_keeps_its_own_origin() {
    let mut w = world(false);
    let here: RegionHandle = (1000 * 256) << 32 | (1000 * 256);
    w.set_main_region(here);
    w.ensure_region(here, (256, 256)).name = "Home Region".into();
    for x in [10.0, 200.0] {
        w.apply(NetEvent::Chat(ChatMessage {
            from_name: "Walker".into(),
            source_id: VENDOR,
            owner_id: OWNER,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Normal,
            position: glam::Vec3::new(x, 50.0, 21.0),
            message: "step".into(),
        }));
    }
    let lines: Vec<_> = w.chat.iter().filter(|l| l.source == VENDOR).collect();
    assert_eq!(lines.len(), 2);
    assert_ne!(lines[0].time, lines[1].time);
    let at = |line: &ChatLine| w.chat_object(line).unwrap().slurl.clone();
    assert_eq!(at(lines[0]), format!("{}/10/50/21", crate::slurl::encode("Home Region")));
    assert_eq!(at(lines[1]), format!("{}/200/50/21", crate::slurl::encode("Home Region")));
}
