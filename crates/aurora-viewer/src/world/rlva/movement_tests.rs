//! Movement, sitting and teleport parity with the Legacy Viewer's RLVa
//! (rlvactions.cpp, rlvhandler.cpp, rlvui.cpp, llagent.cpp,
//! llimprocessing.cpp, llviewermessage.cpp).
use super::{Boundary, Command};
use crate::world::World;
use aurora_net::{InstantMessage, NetCommand, NetEvent, control};
use glam::Vec3;
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0x6d6f_7665_0001);
const B: Uuid = Uuid::from_u128(0x6d6f_7665_0002);
const FRIEND: Uuid = Uuid::from_u128(0x6d6f_7665_0003);
const SEAT: Uuid = Uuid::from_u128(0x6d6f_7665_0004);
const OTHER_SEAT: Uuid = Uuid::from_u128(0x6d6f_7665_0005);

fn world() -> World {
    let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        w.apply(event);
    }
    for (id, local) in [(SEAT, 33001), (OTHER_SEAT, 33002)] {
        let mut o = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: id,
            owner_id: w.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        });
        o.local_id = local;
        o.parent_id = 0;
        o.state = 0;
        o.name_values.clear();
        w.objects.upsert(w.main_region.expect("demo region"), o);
    }
    w.rlva.configure(true);
    w.rlva_sitting_tick();
    w
}

fn say(w: &mut World, actor: Uuid, text: &str) {
    for part in text.split(',') {
        w.rlva.process(actor, Command::parse(part).expect("a command"));
    }
}

/// Our avatar on `seat` (its local id), or standing.
fn seat_on(w: &mut World, seat: Option<u32>) {
    w.agent.seated = seat.is_some();
    w.agent.ground_sit = false;
    let own = w.objects.index_of_uuid(&w.agent_id).expect("own avatar");
    w.objects.get_mut(own).expect("own avatar").parent_id = seat.unwrap_or(0);
}

fn local_teleport(w: &World, distance: f32) -> NetCommand {
    NetCommand::TeleportTo {
        handle: w.main_region.expect("demo region"),
        position: w.agent.position + Vec3::X * distance,
        look_at: Vec3::X,
    }
}

fn teleport_im(w: &World, dialog: u8, from: Uuid, message: &str) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: from,
        from_name: "Test Resident".into(),
        to_agent_id: w.agent_id,
        dialog,
        session_id: Uuid::from_u128(0x6d6f_7665_0100 + u128::from(dialog)),
        message: message.into(),
        offline: false,
        from_group: false,
        binary_bucket: vec![],
        timestamp: 0,
    })
}

/// G1. RlvActions::canTeleportToLocal: whether @sittp applies leaves out
/// the issuer's own restriction, but the distance is the global modifier,
/// the issuer's own value included.
#[test]
fn rlva_tpto_local_distance_keeps_the_issuers_own_sittp_value() {
    let mut w = world();
    let tp = local_teleport(&w, 1.0);
    say(&mut w, A, "sittp:0.5=n");
    assert!(w.rlva_allows_command(&tp, A), "only its own restriction: exempt");
    say(&mut w, B, "sittp=n");
    assert!(!w.rlva_allows_command(&tp, A), "B's @sittp applies, at A's 0.5 m");
    assert!(w.rlva_allows_command(&local_teleport(&w, 0.4), A));
    say(&mut w, A, "clear");
    assert!(w.rlva_allows_command(&tp, A), "B's 1.5 m alone");
    say(&mut w, A, "tplocal:0.5=n");
    say(&mut w, B, "tplocal=n");
    assert!(!w.rlva_allows_command(&tp, A), "the same for @tplocal");
}

/// G3. RlvUIEnabler::onToggleMovement lands a flight only when @fly,
/// @alwaysrun or @temprun comes or goes: a takeoff forced by the issuer
/// of @fly=n stays in the air.
#[test]
fn rlva_fly_lock_lands_only_when_the_movement_rules_change() {
    let mut w = world();
    w.fly_override = true;
    say(&mut w, A, "fly=n");
    w.rlva_movement_tick();
    say(&mut w, A, "fly=force");
    w.rlva_pending();
    assert!(w.agent.flying, "its own @fly=n lets its force through");
    w.rlva_movement_tick();
    w.rlva_movement_tick();
    assert!(w.agent.flying, "nothing changed: the flight goes on");
    say(&mut w, B, "alwaysrun=n");
    w.rlva_movement_tick();
    assert!(!w.agent.flying, "a movement rule came: @fly lands it");
    w.agent.flying = true;
    say(&mut w, A, "fly=y");
    w.rlva_movement_tick();
    assert!(w.agent.flying, "@fly went: nothing to land");
}

/// G4. LLAgent::setFlying refuses a forced takeoff seated or where the land
/// allows no flight; a landing always goes.
#[test]
fn rlva_forced_takeoff_obeys_sitting_and_the_land() {
    let mut w = world();
    w.fly_override = true;
    seat_on(&mut w, Some(33001));
    say(&mut w, A, "fly:true=force");
    w.rlva_pending();
    assert!(!w.agent.flying, "no takeoff seated");
    seat_on(&mut w, None);
    w.fly_override = false;
    w.parcel = None;
    say(&mut w, A, "fly:true=force");
    w.rlva_pending();
    assert!(!w.agent.flying, "no parcel known: LLAgent::canFly says no");
    w.fly_override = true;
    say(&mut w, A, "fly:true=force");
    w.rlva_pending();
    assert!(w.agent.flying);
    w.fly_override = false;
    say(&mut w, A, "fly:false=force");
    w.rlva_pending();
    assert!(!w.agent.flying, "a landing needs no permission");
}

/// G5. @getsitid on the ground: the avatar is its own root.
#[test]
fn rlva_getsitid_on_the_ground_is_our_own_id() {
    let mut w = world();
    assert!(w.rlva_sit_id().is_nil());
    w.agent.ground_sit = true;
    assert_eq!(w.rlva_sit_id(), w.agent_id);
}

/// G6. m_posSitSource stays until a stand under @standtp uses it: a
/// release and a new lock while seated keep it.
#[test]
fn rlva_standtp_origin_survives_a_release_and_a_new_lock_while_seated() {
    let mut w = world();
    say(&mut w, A, "standtp=n");
    let origin = (w.main_region.unwrap(), w.agent.position);
    let mut sit = NetCommand::RequestSit {
        handle: origin.0,
        target: SEAT,
        offset: Vec3::ZERO,
    };
    assert_eq!(w.rlva_send_boundary(&mut sit, Uuid::nil()), Boundary::Send);
    w.rlva_note_sent(&mut sit, Uuid::nil(), std::time::Instant::now());
    seat_on(&mut w, Some(33001));
    w.rlva_sitting_tick();
    say(&mut w, A, "clear");
    w.rlva_sitting_tick();
    say(&mut w, A, "standtp=n");
    seat_on(&mut w, None);
    w.rlva_sitting_tick();
    assert_eq!(w.rlva.stand_return, Some(origin));
    // a stand without @standtp leaves the place for a later one
    let mut w = world();
    w.rlva.stand_origin = Some(origin);
    seat_on(&mut w, Some(33001));
    w.rlva_sitting_tick();
    seat_on(&mut w, None);
    w.rlva_sitting_tick();
    assert!(w.rlva.stand_return.is_none());
    assert_eq!(w.rlva.stand_origin, Some(origin));
}

/// LLVOAvatar::setParent: moving from seat to seat leaves the first one
/// (getOffObject → sitDown(false)), a stand for @standtp and @notify.
#[test]
fn rlva_seat_to_seat_is_a_stand_then_a_sit() {
    let mut w = world();
    say(&mut w, A, "notify:123=n,standtp=n");
    w.rlva.out.clear();
    let origin = (w.main_region.unwrap(), w.agent.position);
    w.rlva.stand_origin = Some(origin);
    seat_on(&mut w, Some(33001));
    w.rlva_sitting_tick();
    seat_on(&mut w, Some(33002));
    w.rlva_sitting_tick();
    assert_eq!(w.rlva.stand_return, Some(origin));
    let notices: Vec<_> = std::mem::take(&mut w.rlva.out)
        .into_iter()
        .filter_map(|c| match c {
            NetCommand::ChatReply { message, .. } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        notices,
        [
            format!("/sat object legally {SEAT}"),
            format!("/unsat object legally {SEAT}"),
            format!("/sat object legally {OTHER_SEAT}"),
        ]
    );
}

/// G18. RlvUtil::forceTp → teleportViaLocationLookAt with no look-at: the
/// camera's direction.
#[test]
fn rlva_standtp_return_faces_where_the_camera_looks() {
    let mut w = world();
    assert!(w.rlva_stand_return_command(Vec3::Y).is_none());
    w.rlva.stand_return = Some((7, Vec3::ONE));
    assert!(matches!(
        w.rlva_stand_return_command(Vec3::Y),
        Some(NetCommand::TeleportTo { handle: 7, position, look_at }) if position == Vec3::ONE && look_at == Vec3::Y
    ));
}

/// G13. `/siton` and the area search's Sit send AgentRequestSit without
/// RlvHandler::setSitSource.
#[test]
fn rlva_sits_without_a_sit_source_record_no_standtp_origin() {
    let mut w = world();
    say(&mut w, A, "standtp=n");
    let mut sit = NetCommand::RequestSit {
        handle: w.main_region.unwrap(),
        target: SEAT,
        offset: Vec3::ZERO,
    };
    w.rlva.sitting.no_sit_source = true;
    assert_eq!(w.rlva_send_boundary(&mut sit, Uuid::nil()), Boundary::Send);
    w.rlva_note_sent(&mut sit, Uuid::nil(), std::time::Instant::now());
    w.rlva.sitting.no_sit_source = false;
    assert!(w.rlva.stand_origin.is_none());
    w.rlva_note_sent(&mut sit, Uuid::nil(), std::time::Instant::now());
    assert!(w.rlva.stand_origin.is_some(), "the menu's sit records it");
}

/// G8. teleport_request_callback → send_lures: the offer answering an
/// accepted request goes under @showloc, the automatic one included.
#[test]
fn rlva_accepted_teleport_request_is_answered_under_showloc() {
    let mut w = world();
    say(&mut w, A, "showloc=n");
    w.apply(teleport_im(
        &w,
        crate::world::notifications::im::TELEPORT_REQUEST,
        FRIEND,
        "Bring me",
    ));
    let invite = w.teleport_invites.list.last().expect("the request").id;
    let mut answer = w.answer_teleport_invite(invite, true);
    assert!(matches!(&answer[..], [NetCommand::OfferTeleport { to, .. }] if to == &vec![FRIEND]));
    assert_eq!(w.rlva_send_boundary(&mut answer[0], Uuid::nil()), Boundary::Send);
    assert!(matches!(&answer[0], NetCommand::OfferTeleport { to, .. } if to == &vec![FRIEND]));
    w.rlva_note_sent(&mut answer[0], Uuid::nil(), std::time::Instant::now());
    // an offer of our own to the same resident still loses that recipient
    // (LLAvatarActions::canOfferTeleport)
    let mut own = NetCommand::OfferTeleport {
        to: vec![FRIEND],
        message: String::new(),
    };
    w.rlva_send_boundary(&mut own, Uuid::nil());
    assert!(matches!(&own, NetCommand::OfferTeleport { to, .. } if to.is_empty()), "{own:?}");
    // @accepttprequest answers at once, the same way
    say(&mut w, A, "accepttprequest=n");
    w.apply(teleport_im(&w, crate::world::notifications::im::TELEPORT_REQUEST, FRIEND, "Again"));
    let mut sent = w.teleport_invites.take_commands();
    assert!(matches!(&sent[..], [NetCommand::OfferTeleport { .. }]), "{sent:?}");
    assert_eq!(w.rlva_send_boundary(&mut sent[0], Uuid::nil()), Boundary::Send);
}

/// G10. lure_callback asks RLVa nothing: a lock set after the offer came
/// does not stop its acceptance.
#[test]
fn rlva_offer_accepted_after_a_new_tplure_lock_still_goes() {
    let mut w = world();
    w.apply(teleport_im(&w, crate::world::notifications::im::LURE_USER, FRIEND, "Join me"));
    let invite = w.teleport_invites.list.last().expect("the offer").id;
    say(&mut w, A, "tplure=n");
    let answer = w.answer_teleport_invite(invite, true);
    assert!(matches!(&answer[..], [NetCommand::AcceptLure { godlike: false, .. }]), "{answer:?}");
    assert!(w.rlva_allows_command(&answer[0], Uuid::nil()));
    let stranger = NetCommand::AcceptLure {
        lure_id: Uuid::from_u128(99),
        godlike: false,
    };
    assert!(!w.rlva_allows_command(&stranger, Uuid::nil()), "only an offer received");
}

/// G16. A god's lure is never censored.
#[test]
fn rlva_godlike_lure_message_is_never_censored() {
    let mut w = world();
    say(&mut w, A, "recvim=n,showloc=n");
    assert_eq!(w.rlva_teleport_message("Hello", FRIEND, true), "Hello");
    assert_eq!(
        w.rlva_teleport_message("Hello", FRIEND, false),
        aurora_i18n::tr!("rlva-hidden-generic")
    );
}

/// G11. The pending ground sit is released for its issuer alone.
#[test]
fn rlva_released_ground_sit_passes_only_for_its_issuer() {
    let mut w = world();
    seat_on(&mut w, Some(33001));
    w.rlva_sitting_tick();
    assert!(w.rlva_force_ground_sit(A).is_some());
    say(&mut w, B, "sit=n");
    seat_on(&mut w, None);
    let ground = w.rlva_sitting_tick();
    assert!(matches!(&ground[..], [(actor, _)] if *actor == A));
    assert!(w.rlva_allows_command(&NetCommand::OneShotControl(control::SIT_ON_GROUND), A));
    assert!(
        !w.rlva_allows_command(&NetCommand::OneShotControl(control::SIT_ON_GROUND), FRIEND),
        "another object is refused by B's @sit"
    );
    w.rlva_sitting_tick();
    assert!(
        !w.rlva_allows_command(&NetCommand::OneShotControl(control::SIT_ON_GROUND), A),
        "for the frame of the stand only"
    );
}

/// G12. RlvForceHandler<RLV_BHVR_TPTO>: a region name is asked from the
/// map first and nothing is decided before it answers; the teleport it
/// resolves to can be cancelled.
#[test]
fn rlva_tpto_to_a_region_name_waits_for_the_map_and_stays_cancellable() {
    let mut w = world();
    let unknown = NetCommand::TeleportToRegion {
        request: Uuid::from_u128(5),
        name: "A region nobody knows".into(),
        position: Vec3::new(128.0, 128.0, 30.0),
        look_at: Vec3::X,
    };
    say(&mut w, B, "tploc=n");
    assert!(w.rlva_allows_command(&unknown, A), "decided once the map answers");
    say(&mut w, B, "clear");
    say(&mut w, A, "tpto:A region nobody knows/128/128/30=force");
    let sent = w.rlva_pending();
    assert!(matches!(&sent[..], [(_, NetCommand::TeleportToRegion { .. })]), "{sent:?}");
    w.tp_cancel.on_simulator_flags(0);
    assert!(w.tp_cancel.offered(false), "a named @tpto can be cancelled");
    let (x, y) = w.main_origin().unwrap();
    let here = w.agent.position + Vec3::X;
    say(
        &mut w,
        A,
        &format!("tpto:{}/{}/{}=force", x as f32 + here.x, y as f32 + here.y, here.z),
    );
    assert!(matches!(&w.rlva_pending()[..], [(_, NetCommand::TeleportTo { .. })]));
    w.tp_cancel.on_simulator_flags(0);
    assert!(!w.tp_cancel.offered(false), "a @tpto to a position cannot");
}

/// G19. LLAgent::teleportViaLocation: a refused @tpto says so.
#[test]
fn rlva_refused_tpto_shows_the_blocked_notice() {
    let mut w = world();
    say(&mut w, B, "tplocal:0.5=n");
    let (x, y) = w.main_origin().unwrap();
    let to = w.agent.position + Vec3::X * 5.0;
    let lines = w.chat.len();
    say(&mut w, A, &format!("tpto:{}/{}/{}=force", x as f32 + to.x, y as f32 + to.y, to.z));
    assert!(w.rlva_pending().is_empty());
    assert_eq!(w.chat.len(), lines + 1);
    assert_eq!(w.chat.back().unwrap().text, aurora_i18n::tr!("rlva-blocked-teleport"));
}

/// G9. The place windows grey Teleport by canTeleportToLocation whatever
/// the distance, a landmark's by @tplm, Home by @tplm and @tploc together.
#[test]
fn rlva_place_buttons_grey_as_the_legacy_windows_do() {
    let mut w = world();
    assert!(w.rlva_can_teleport_location() && w.rlva_home_button_enabled() && w.rlva_landmark_button_enabled());
    say(&mut w, A, "sittp=n");
    assert!(w.rlva_can_teleport_location(), "@sittp does not grey a place");
    say(&mut w, A, "clear,tploc=n");
    assert!(!w.rlva_can_teleport_location() && w.rlva_home_button_enabled() && w.rlva_landmark_button_enabled());
    say(&mut w, A, "clear,unsit=n");
    assert!(w.rlva_can_teleport_location(), "standing");
    seat_on(&mut w, Some(33001));
    assert!(!w.rlva_can_teleport_location());
    assert!(
        w.rlva_home_button_enabled() && w.rlva_landmark_button_enabled(),
        "@unsit greys neither"
    );
    say(&mut w, A, "clear,tplm=n");
    assert!(!w.rlva_landmark_button_enabled() && w.rlva_home_button_enabled());
    say(&mut w, A, "tploc=n");
    assert!(!w.rlva_home_button_enabled());
}

/// LLAgent::startTeleportRequest: a teleport that waited for the maturity
/// preference goes as RLVa let it through, whatever was locked since.
#[test]
fn rlva_teleport_held_for_maturity_is_not_asked_again() {
    let mut w = world();
    let mut home = NetCommand::TeleportHome;
    say(&mut w, A, "tplm=n,tploc=n");
    assert_eq!(w.rlva_send_boundary(&mut home, Uuid::nil()), Boundary::Refused);
    w.rlva.teleport_already_checked = true;
    assert_eq!(w.rlva_send_boundary(&mut home, Uuid::nil()), Boundary::Send);
}

/// LLToolPie::walkToClickedLocation and teleportToClickedLocation: under
/// @interact the pick is gone and nothing happens; otherwise only
/// RlvActions::canTeleportToLocal is asked, whatever the distance.
#[test]
fn rlva_double_click_drops_its_pick_under_interact_and_asks_the_local_rules() {
    use super::DoubleClickRlva::{Blocked, Go, Ignored};
    let mut w = world();
    let here = w.agent_global().expect("the demo avatar's place");
    let near = Some(here + glam::DVec3::new(10.0, 0.0, 0.0));
    let far = Some(here + glam::DVec3::new(300.0, 0.0, 0.0));
    assert_eq!(w.rlva_double_click(near), Go);
    say(&mut w, A, "interact=n");
    assert_eq!(w.rlva_double_click(near), Ignored, "@interact leaves no pick, land included");
    say(&mut w, A, "clear,tploc=n");
    assert_eq!(w.rlva_double_click(far), Go, "@tploc is not asked of a double click");
    say(&mut w, A, "clear,tplocal:5=n");
    assert_eq!(w.rlva_double_click(near), Blocked);
    assert_eq!(w.rlva_double_click(far), Blocked, "a far point is still asked the local rules");
    assert_eq!(w.rlva_double_click(Some(here + glam::DVec3::new(2.0, 0.0, 0.0))), Go);
}

fn script_question(task: u128, questions: i32) -> NetEvent {
    NetEvent::ScriptQuestion {
        task_id: Uuid::from_u128(task),
        item_id: Uuid::from_u128(task + 1),
        object_name: "Teleporter".into(),
        owner_name: "Test Resident".into(),
        questions,
        experience: Uuid::nil(),
        region: 77,
    }
}

fn permission_question_id(w: &World, task: u128) -> u64 {
    w.notifications
        .list
        .iter()
        .rev()
        .find(|n| matches!(&n.data, crate::world::notifications::Data::Permissions { task: t, .. } if *t == Uuid::from_u128(task)))
        .expect("the question is shown")
        .id
}

/// process_script_question and script_question_cb (llviewermessage.cpp):
/// what RLVa stripped from a question (RlvUtil::filterScriptQuestions) is
/// told once the question is answered yes, at once when nothing is left to
/// ask; a refusal says nothing.
#[test]
fn rlva_stripped_script_permissions_are_told_when_the_question_is_answered_yes() {
    use crate::world::notifications::Response;
    const ANIMATE: i32 = 1 << 4;
    const ATTACH: i32 = 1 << 5;
    const TELEPORT: i32 = 1 << 12;
    let teleport = aurora_i18n::tr!("rlva-blocked-permission-teleport", object = "Teleporter".to_owned());
    let attach = aurora_i18n::tr!("rlva-blocked-permission-attach", object = "Teleporter".to_owned());
    let told = |w: &World, text: &str| w.chat.iter().filter(|line| line.text == text).count();
    let mut w = world();
    say(&mut w, A, "tploc=n");
    // the dialog stays for what is left; a yes tells what was stripped
    w.apply(script_question(0x7a00, ANIMATE | TELEPORT));
    assert_eq!(told(&w, &teleport), 0, "nothing before the answer");
    let id = permission_question_id(&w, 0x7a00);
    let (cmds, _) = w.respond_notification(id, Response::Accept);
    assert!(
        matches!(&cmds[..], [NetCommand::ScriptAnswer { questions: ANIMATE, .. }]),
        "{cmds:?}"
    );
    assert_eq!(told(&w, &teleport), 1);
    // a refusal says nothing
    w.apply(script_question(0x7b00, ANIMATE | TELEPORT));
    let id = permission_question_id(&w, 0x7b00);
    w.respond_notification(id, Response::Decline);
    assert_eq!(told(&w, &teleport), 1);
    // nothing left to ask: answered yes at once (forceResponse)
    w.apply(script_question(0x7c00, TELEPORT));
    assert!(matches!(&w.rlva.out[..], [.., NetCommand::ScriptAnswer { questions: 0, .. }]));
    assert_eq!(told(&w, &teleport), 2);
    // the attach lock, and the teleport notice when both are stripped
    say(&mut w, A, "clear,addattach=n");
    w.apply(script_question(0x7d00, ANIMATE | ATTACH));
    let id = permission_question_id(&w, 0x7d00);
    w.respond_notification(id, Response::Accept);
    assert_eq!(told(&w, &attach), 1);
    say(&mut w, A, "tploc=n");
    w.apply(script_question(0x7e00, ANIMATE | ATTACH | TELEPORT));
    let id = permission_question_id(&w, 0x7e00);
    w.respond_notification(id, Response::Accept);
    assert_eq!((told(&w, &attach), told(&w, &teleport)), (1, 3));
}
