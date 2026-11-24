//! The enable rules of a group link's menu (`GroupLinkItems`, ported from
//! the group cases of FSCommon::checkIsActionEnabled, fscommon.cpp).

use super::*;
use crate::demo::{DEMO_GROUP1, DEMO_GROUP2};
use aurora_net::NetEvent;
use aurora_net::search::GroupProfile;

fn membership(id: Uuid) -> aurora_net::GroupMembership {
    aurora_net::GroupMembership {
        id,
        name: "Aurora Builders".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    }
}

fn profile(id: Uuid, open_enrollment: bool) -> NetEvent {
    NetEvent::GroupProfile(Box::new(GroupProfile {
        id,
        name: "Aurora Builders".into(),
        open_enrollment,
        ..Default::default()
    }))
}

/// A world where we belong to `ours`, with the data of `known` groups.
fn world(ours: &[Uuid], known: &[(Uuid, bool)]) -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = crate::demo::DEMO_AGENT;
    world.apply(NetEvent::Groups(ours.iter().copied().map(membership).collect()));
    for (id, open) in known {
        world.apply(profile(*id, *open));
    }
    world
}

fn setgroup(world: &mut World) {
    world.rlva.configure(true);
    assert!(world.rlva.receive(&aurora_net::ChatMessage {
        from_name: "Test lock".into(),
        source_id: Uuid::from_u128(99),
        owner_id: Uuid::nil(),
        source_type: aurora_net::ChatSourceType::Object,
        chat_type: aurora_net::ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: "@setgroup=n".into(),
    }));
}

#[test]
fn group_link_member_activates_and_leaves_but_does_not_join() {
    let world = world(&[DEMO_GROUP1], &[(DEMO_GROUP1, true)]);
    let items = GroupLinkItems::of(&world, DEMO_GROUP1, true);
    assert_eq!(
        items,
        GroupLinkItems {
            activate: true,
            join: false,
            leave: true,
            copy_name: true,
        }
    );
}

#[test]
fn group_link_active_group_is_not_activated_again() {
    let mut world = world(&[DEMO_GROUP1], &[]);
    world.groups.active = DEMO_GROUP1;
    let items = GroupLinkItems::of(&world, DEMO_GROUP1, true);
    assert!(!items.activate);
    assert!(items.leave);
}

#[test]
fn group_link_non_member_joins_only_once_the_open_group_is_known() {
    let mut world = world(&[], &[]);
    assert!(!GroupLinkItems::of(&world, DEMO_GROUP1, true).join, "no data yet");
    world.apply(profile(DEMO_GROUP1, true));
    let items = GroupLinkItems::of(&world, DEMO_GROUP1, true);
    assert!(items.join);
    assert!(!items.activate && !items.leave, "not one of ours");
}

#[test]
fn group_link_closed_enrollment_greys_join() {
    let world = world(&[], &[(DEMO_GROUP1, false)]);
    assert!(!GroupLinkItems::of(&world, DEMO_GROUP1, true).join);
}

#[test]
fn group_link_setgroup_greys_activate_join_and_leaving_the_active_group() {
    let mut world = world(&[DEMO_GROUP1, DEMO_GROUP2], &[(Uuid::from_u128(5), true)]);
    world.groups.active = DEMO_GROUP1;
    setgroup(&mut world);
    let active = GroupLinkItems::of(&world, DEMO_GROUP1, true);
    assert!(!active.activate && !active.leave);
    let other = GroupLinkItems::of(&world, DEMO_GROUP2, true);
    assert!(!other.activate, "canChangeActiveGroup");
    assert!(other.leave, "only the active group is held");
    assert!(!GroupLinkItems::of(&world, Uuid::from_u128(5), true).join);
}

#[test]
fn group_link_group_limit_greys_join() {
    let mut world = world(&[DEMO_GROUP2], &[(DEMO_GROUP1, true)]);
    let mut login = crate::demo::events()
        .into_iter()
        .find_map(|ev| match ev {
            NetEvent::LoggedIn(login) => Some(login),
            _ => None,
        })
        .expect("demo login");
    std::sync::Arc::make_mut(&mut login)
        .raw
        .insert("max-agent-groups", aurora_llsd::Llsd::from(1));
    world.login = Some(login);
    assert!(!GroupLinkItems::of(&world, DEMO_GROUP1, true).join, "LLAgent::canJoinGroups");
}

#[test]
fn group_link_copy_name_waits_for_the_name() {
    let world = world(&[DEMO_GROUP1], &[]);
    assert!(!GroupLinkItems::of(&world, DEMO_GROUP1, false).copy_name);
    assert!(GroupLinkItems::of(&world, DEMO_GROUP1, true).copy_name);
}

/// FSCommon::requestGroupData: opening the menu asks for missing data only.
#[test]
fn group_link_menu_asks_for_missing_group_data_when_it_opens() {
    use crate::ui::widgets::elide_tests::{Frames, painted};
    let p = crate::theme::Theme::default().palette();
    for (known, asked) in [(false, true), (true, false)] {
        let data: &[(Uuid, bool)] = if known { &[(DEMO_GROUP1, true)] } else { &[] };
        let world = world(&[], data);
        let mut frames = Frames::new();
        let body = |ui: &mut egui::Ui| {
            let resp = ui.add(egui::Label::new("Aurora Builders").sense(egui::Sense::click()));
            group_link_menu(&resp, &p, &world, DEMO_GROUP1, Some("Aurora Builders"));
        };
        let texts = frames.run(Vec::new(), body);
        let point = painted(&texts, "Aurora Builders").rect.center();
        for pressed in [true, false] {
            frames.run(
                vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
                body,
            );
        }
        let requested = take_requests(&frames.ctx)
            .iter()
            .any(|a| matches!(a, CtxAction::RequestGroupData(id) if *id == DEMO_GROUP1));
        assert_eq!(requested, asked, "data known: {known}");
    }
}
