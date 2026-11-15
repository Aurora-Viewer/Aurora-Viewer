//! Group invitations with a membership fee (send_join_group_response),
//! caution permission questions (ScriptQuestionCaution), script menus of a
//! blocked owner (process_script_dialog) and offers from a resident blocked
//! for some properties only (inventory_offer_handler), like the Legacy
//! Viewer.

use super::mutes::{MuteType, flag};
use super::notifications::{Data, Response, im as d};
use super::*;
use aurora_net::NetCommand;

const GROUP: Uuid = Uuid::from_u128(0x6E20);
const TX: Uuid = Uuid::from_u128(0x7A20);
const JANE: Uuid = Uuid::from_u128(0x1A4E);

fn world() -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(9);
    world.mutes.reset(world.agent_id);
    world
}

/// IM_GROUP_INVITATION with its invite_bucket_t: the fee (big-endian S32)
/// then the role id.
fn invitation(fee: Option<i32>, tx: Uuid, offline: bool) -> NetEvent {
    let bucket = fee.map_or_else(Vec::new, |fee| {
        let mut bucket = fee.to_be_bytes().to_vec();
        bucket.extend_from_slice(Uuid::from_u128(0x201E).as_bytes());
        bucket
    });
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: GROUP,
        from_name: "Lagoon Sailing Club".into(),
        to_agent_id: Uuid::from_u128(9),
        dialog: d::GROUP_INVITATION,
        session_id: tx,
        message: "Tess Touch has invited you to join Lagoon Sailing Club.".into(),
        offline,
        from_group: false,
        binary_bucket: bucket,
        timestamp: 0,
    })
}

fn group_card(world: &World) -> &notifications::Notification {
    world
        .notifications
        .list
        .iter()
        .rfind(|n| matches!(n.data, Data::Group { .. }))
        .expect("group card")
}

fn question(task: u128, questions: i32) -> NetEvent {
    NetEvent::ScriptQuestion {
        task_id: Uuid::from_u128(task),
        item_id: Uuid::from_u128(task + 1),
        object_name: "Tip jar".into(),
        owner_name: "Tess Touch".into(),
        questions,
        experience: Uuid::nil(),
        region: 77,
    }
}

fn last_system_line(world: &World) -> Option<&str> {
    world
        .chat
        .iter()
        .rev()
        .find(|l| l.kind == ChatKind::System)
        .map(|l| l.text.as_str())
}

#[test]
fn the_fee_is_read_from_the_invitation_bucket() {
    let mut bucket = 250i32.to_be_bytes().to_vec();
    bucket.extend_from_slice(&[0; 16]);
    assert_eq!(notifications::group_invite_fee(&bucket), Some(250));
    assert_eq!(notifications::group_invite_fee(&bucket[..19]), None);
    assert_eq!(notifications::group_invite_fee(&[]), None);
}

#[test]
fn an_invitation_with_a_fee_names_it_and_asks_again_before_joining() {
    let mut world = world();
    world.balance = Some(250);
    world.apply(invitation(Some(50), TX, false));
    let card = group_card(&world);
    let invitation_id = card.id;
    assert!(card.body.starts_with("Tess Touch has invited you"));
    assert!(card.body.ends_with(&notifications::group_fee_line(50)));
    assert!(matches!(
        card.data,
        Data::Group {
            fee: 50,
            confirming: false,
            ..
        }
    ));
    // Accept sends nothing yet: the cost is asked (JoinGroupCanAfford)
    let (cmds, _) = world.respond_notification(invitation_id, Response::Accept);
    assert!(cmds.is_empty());
    assert!(world.notifications.list.iter().all(|n| n.id != invitation_id));
    let card = group_card(&world);
    let confirm = card.id;
    assert_eq!(card.title, aurora_i18n::tr!("ui-group-profile-join-title"));
    let cost = aurora_i18n::tr!("ui-group-profile-join-cost", cost = aurora_i18n::integer(50));
    assert!(card.body.starts_with(&cost));
    assert!(card.body.contains("Tess Touch has invited you"));
    assert!(!card.body.contains(&notifications::group_fee_line(50)));
    assert!(matches!(card.data, Data::Group { confirming: true, .. }));
    // the group's name stays a link
    assert!(card.name_links.iter().any(|l| l.name == "Lagoon Sailing Club"));
    // Join: the answer goes, once
    let (cmds, _) = world.respond_notification(confirm, Response::Accept);
    assert!(matches!(
        &cmds[..],
        [NetCommand::SendImDialog { to, dialog: d::GROUP_INVITATION_ACCEPT, id, .. }] if *to == GROUP && *id == TX
    ));
    assert!(world.notifications.list.iter().all(|n| !matches!(n.data, Data::Group { .. })));

    // Cancel on the question declines the invitation
    world.apply(invitation(Some(50), TX, false));
    let id = group_card(&world).id;
    world.respond_notification(id, Response::Accept);
    let confirm = group_card(&world).id;
    let (cmds, _) = world.respond_notification(confirm, Response::Decline);
    assert!(matches!(
        &cmds[..],
        [NetCommand::SendImDialog {
            dialog: d::GROUP_INVITATION_DECLINE,
            ..
        }]
    ));
}

#[test]
fn an_offline_invitation_with_a_fee_is_confirmed_then_answered_by_capability() {
    let mut world = world();
    world.balance = Some(250);
    world.apply(invitation(Some(50), Uuid::nil(), true));
    let id = group_card(&world).id;
    assert!(world.respond_notification(id, Response::Accept).0.is_empty());
    let confirm = group_card(&world).id;
    let (cmds, _) = world.respond_notification(confirm, Response::Accept);
    assert!(matches!(
        &cmds[..],
        [NetCommand::AnswerOfflineGroupInvite { group, accept: true }] if *group == GROUP
    ));
}

#[test]
fn a_free_or_unreadable_invitation_joins_at_once_and_a_fee_too_high_waits() {
    let mut world = world();
    world.balance = Some(10);
    for fee in [Some(0), None] {
        world.apply(invitation(fee, TX, false));
        let card = group_card(&world);
        assert_eq!(card.body, "Tess Touch has invited you to join Lagoon Sailing Club.");
        let id = card.id;
        let (cmds, _) = world.respond_notification(id, Response::Accept);
        assert!(matches!(
            &cmds[..],
            [NetCommand::SendImDialog {
                dialog: d::GROUP_INVITATION_ACCEPT,
                ..
            }]
        ));
    }
    // short of L$ (JoinGroupCannotAfford): told, the invitation stays
    world.apply(invitation(Some(50), TX, false));
    let id = group_card(&world).id;
    assert!(world.respond_notification(id, Response::Accept).0.is_empty());
    assert!(world.notifications.list.iter().any(|n| n.id == id));
    let cannot = aurora_i18n::tr!("ui-group-profile-join-cannot-afford", cost = aurora_i18n::integer(50));
    assert!(world.notifications.list.iter().any(|n| n.body == cannot));
    // closing it still declines, as before
    let (cmds, _) = world.respond_notification(id, Response::Dismiss);
    assert!(matches!(
        &cmds[..],
        [NetCommand::SendImDialog {
            dialog: d::GROUP_INVITATION_DECLINE,
            ..
        }]
    ));
}

#[test]
fn a_question_to_take_money_warns_and_its_answer_is_told_in_the_chat() {
    let mut world = world();
    world.apply(question(0x7100, 0x2 | 0x10));
    let n = world.notifications.list.last().expect("question");
    assert_eq!(n.title, aurora_i18n::tr!("world-notifications-caution-title"));
    assert_eq!(n.body, notifications::caution_body(0x2 | 0x10, "Tip jar", "Tess Touch"));
    assert!(n.body.contains("Tip jar") && n.body.contains(aurora_i18n::tr!("world-notifications-caution-advice")));
    // the other permission is listed after the warning
    assert!(n.body.ends_with(aurora_i18n::tr!("world-notifications-animate-your-avatar")));
    assert!(matches!(&n.data, Data::Permissions { caution: Some(c), .. } if c.object_name == "Tip jar"));
    let id = n.id;
    let (cmds, _) = world.respond_notification(id, Response::Decline);
    assert!(matches!(
        &cmds[..],
        [NetCommand::ScriptAnswer {
            questions: 0,
            region: 77,
            ..
        }]
    ));
    let denied = notifications::caution_chat_line(
        0x2 | 0x10,
        &notifications::CautionPrompt {
            object_name: "Tip jar".into(),
            owner_name: "Tess Touch".into(),
        },
        None,
        false,
    );
    assert_eq!(last_system_line(&world), Some(denied.as_str()));
    // only the caution permission is named, with an unknown place
    assert!(denied.contains(aurora_i18n::tr!("world-notifications-take-l-from-your-account")));
    assert!(!denied.contains(aurora_i18n::tr!("world-notifications-animate-your-avatar")));
    assert!(denied.contains(aurora_i18n::tr!("world-notifications-caution-unknown-region")));

    world.apply(question(0x7200, 0x2));
    let id = world.notifications.list.last().expect("question").id;
    let (cmds, _) = world.respond_notification(id, Response::Accept);
    assert!(matches!(&cmds[..], [NetCommand::ScriptAnswer { questions: 2, .. }]));
    let granted = last_system_line(&world).expect("granted line");
    assert!(granted.contains("Tip jar") && granted.contains("Tess Touch"));
    assert_ne!(granted, denied);
}

#[test]
fn an_ordinary_question_keeps_its_wording_and_tells_nothing() {
    let mut world = world();
    world.apply(question(0x7300, 0x10));
    let n = world.notifications.list.last().expect("question");
    assert_eq!(n.title, aurora_i18n::tr!("world-mod-requested-permissions"));
    assert!(matches!(n.data, Data::Permissions { caution: None, .. }));
    let id = n.id;
    let lines = world.chat.len();
    world.respond_notification(id, Response::Accept);
    assert_eq!(world.chat.len(), lines);
    // privileged land access is a caution permission too, even from an
    // experience (it keeps the ordinary dialog, process_script_question)
    let mut land = question(0x7400, 0x80000);
    if let NetEvent::ScriptQuestion { experience, .. } = &mut land {
        *experience = Uuid::from_u128(0xE5);
    }
    world.apply(land);
    let n = world.notifications.list.last().expect("question");
    assert!(matches!(
        &n.data,
        Data::Permissions {
            experience: None,
            caution: Some(_),
            ..
        }
    ));
    assert!(n.body.contains("Tip jar"));
}

#[test]
fn a_menu_of_a_blocked_owner_is_dropped_even_when_its_object_is_unknown() {
    let mut world = world();
    world.block(JANE, "Jane Doe", MuteType::Agent).expect("blocked");
    let menu = |object: u128, owner: Uuid| NetEvent::ScriptDialog {
        sender: ([127, 0, 0, 1], 13000).into(),
        object_id: Uuid::from_u128(object),
        object_name: "Vendor".into(),
        owner_name: "Jane Doe".into(),
        owner_id: owner,
        message: "Choose".into(),
        channel: -42,
        buttons: vec!["Yes".into()],
    };
    let count = world.notifications.list.len();
    world.apply(menu(0x4D01, JANE));
    assert_eq!(world.notifications.list.len(), count, "blocked owner named by the simulator");
    // no OwnerData block (older simulators): the scene cannot tell either
    world.apply(menu(0x4D02, Uuid::nil()));
    assert_eq!(world.notifications.list.len(), count + 1);
    world.apply(menu(0x4D03, Uuid::from_u128(0x0BE)));
    assert_eq!(world.notifications.list.len(), count + 2);
}

/// A permission question follows the block rule of script dialogs: the
/// object blocked by id or by name, or its owner blocked when the scene
/// knows the object.
#[test]
fn a_question_follows_the_block_rule_of_script_dialogs() {
    let mut world = world();
    let (handle, mut object) = crate::demo::events()
        .into_iter()
        .find_map(|event| match event {
            NetEvent::ObjectUpdates { handle, mut objects } => objects.pop().map(|o| (handle, o)),
            _ => None,
        })
        .expect("a demo object");
    object.owner_id = JANE;
    object.full_id = Uuid::from_u128(0x5100);
    world.objects.upsert(handle, object);
    let count = world.notifications.list.len();
    // nothing blocked: the question shows
    world.apply(question(0x5100, 0x10));
    assert_eq!(world.notifications.list.len(), count + 1);
    // its owner blocked: the question of her object in the scene is dropped
    world.block(JANE, "Jane Doe", MuteType::Agent).expect("blocked");
    world.apply(question(0x5100, 0x10));
    assert_eq!(world.notifications.list.len(), count + 1, "blocked owner");
    // an object the scene does not know keeps its question (its owner is
    // not in the message)
    world.apply(question(0x5200, 0x10));
    assert_eq!(world.notifications.list.len(), count + 2);
    // the object blocked by id, then by name
    world.block(Uuid::from_u128(0x5300), "Vendor", MuteType::Object).expect("blocked");
    world.apply(question(0x5300, 0x10));
    assert_eq!(world.notifications.list.len(), count + 2, "blocked object");
    world.block(Uuid::nil(), "Tip jar", MuteType::ByName).expect("blocked");
    world.apply(question(0x5400, 0x10));
    assert_eq!(world.notifications.list.len(), count + 2, "blocked by name");
}

#[test]
fn an_offer_from_a_resident_blocked_for_particles_only_is_declined() {
    let mut world = world();
    world
        .block_flags(JANE, "Jane Doe", MuteType::Agent, flag::PARTICLES)
        .expect("blocked");
    world.take_social_commands();
    let im = |dialog: u8, bucket: Vec<u8>| InstantMessage {
        from_agent_id: JANE,
        from_name: "Jane Doe".into(),
        to_agent_id: Uuid::from_u128(9),
        dialog,
        session_id: TX,
        message: "Lantern".into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket,
        timestamp: 0,
    };
    // her words still show: only an offer looks at every entry
    assert!(!world.im_blocked(&im(0, Vec::new())));
    let mut bucket = vec![6];
    bucket.extend_from_slice(Uuid::from_u128(0x17E).as_bytes());
    let count = world.notifications.list.len();
    for (dialog, declined) in [
        (d::INVENTORY_OFFERED, d::INVENTORY_DECLINED),
        (d::TASK_INVENTORY_OFFERED, d::TASK_INVENTORY_DECLINED),
    ] {
        world.apply(NetEvent::InstantMessage(im(dialog, bucket.clone())));
        assert_eq!(world.notifications.list.len(), count);
        assert!(
            world.take_social_commands().iter().any(
                |c| matches!(c, NetCommand::SendImDialog { to, dialog: sent, id, .. } if *to == JANE && *sent == declined && *id == TX)
            )
        );
    }
    // a resident not blocked at all is offered as before
    world.unblock(JANE, "Jane Doe", flag::ALL);
    assert!(world.mutes.get(&JANE).is_none());
    world.apply(NetEvent::InstantMessage(im(d::INVENTORY_OFFERED, bucket)));
    assert_eq!(world.notifications.list.len(), count + 1);
}
