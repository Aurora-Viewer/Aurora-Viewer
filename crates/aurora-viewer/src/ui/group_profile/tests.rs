use super::*;
use crate::demo::{DEMO_AGENT, DEMO_GROUP1};
use aurora_net::NetEvent;
use aurora_net::groups::powers;
use aurora_net::search::GroupProfile;
use egui::{Pos2, Rect, Shape};

/// The demo's group as a member sees it (a builder), its data answered by
/// the demo group server.
fn world(member: bool, powers: u64) -> World {
    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = DEMO_AGENT;
    if member {
        world.apply(NetEvent::Groups(vec![aurora_net::GroupMembership {
            id: DEMO_GROUP1,
            name: "Aurora Builders".into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }]));
    }
    world.apply(NetEvent::GroupProfile(Box::new(GroupProfile {
        id: DEMO_GROUP1,
        name: "Aurora Builders".into(),
        charter: "Hello".into(),
        founder: crate::demo::DEMO_LOUP,
        open_enrollment: true,
        membership_fee: 250,
        members: 1248,
        ..Default::default()
    })));
    world.group_data.open(DEMO_GROUP1, member);
    // the pairs go out once the members and roles are in
    for _ in 0..3 {
        for cmd in world.group_data.take_commands() {
            for ev in crate::demo::group::reply(&cmd).unwrap_or_default() {
                world.apply(ev);
            }
        }
    }
    world
}

/// The texts of the window's second frame, top to bottom, and the actions
/// of both frames.
fn texts(st: &mut GroupProfileUi, world: &World, classic: bool) -> (Vec<String>, Vec<GroupAction>) {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    let p = crate::theme::Theme::default().palette();
    let images = HashMap::new();
    let pics = HashMap::new();
    let mut emoji = crate::ui::emoji::Emoji::load();
    let (mut names, mut wanted, mut groups) = (HashSet::new(), HashSet::new(), HashSet::new());
    let env = Env {
        avatar_pics: &pics,
        classic,
        now: 0.0,
        group_calls: false,
    };
    let mut actions = Vec::new();
    let mut shapes = Vec::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::Vec2::new(1400.0, 1300.0))),
                ..Default::default()
            },
            |ui| {
                let mut c = Ctx {
                    p: &p,
                    world,
                    images: &images,
                    emoji: &mut emoji,
                    want_names: &mut names,
                    wanted_images: &mut wanted,
                    want_groups: &mut groups,
                };
                actions.extend(st.show(ui.ctx(), &mut c, &env));
            },
        );
        output.textures_delta.clear();
        shapes = output.shapes;
    }
    fn collect(shape: &Shape, out: &mut Vec<(f32, f32, String)>) {
        match shape {
            Shape::Text(t) => out.push((t.pos.y, t.pos.x, t.galley.text().to_owned())),
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    shapes.iter().for_each(|s| collect(&s.shape, &mut out));
    out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    (out.into_iter().map(|t| t.2).collect(), actions)
}

/// A text of the window, or a card title (in capitals).
fn has(texts: &[String], key: &'static str) -> bool {
    let want = aurora_i18n::text(key);
    texts.iter().any(|t| *t == want || *t == want.to_uppercase())
}

fn opened(world: &mut World) -> GroupProfileUi {
    let mut st = GroupProfileUi::default();
    st.open(world, DEMO_GROUP1);
    st
}

const MEMBER: u64 = powers::NOTICES_RECEIVE | powers::SESSION_JOIN;

#[test]
fn a_member_gets_chat_leave_and_their_settings() {
    let mut world = world(true, MEMBER);
    let mut st = opened(&mut world);
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(texts.iter().any(|t| t == "Aurora Builders"));
    for key in [
        "common-chat",
        "ui-group-profile-leave",
        "ui-group-profile-home",
        "ui-group-profile-charter",
        "ui-group-profile-latest-notices",
        "ui-group-profile-me",
        "ui-group-profile-active",
        "ui-group-profile-title-shown",
        "ui-group-profile-receive-notices",
        "ui-group-profile-receive-chat",
        "ui-group-profile-show-in-profile",
        "ui-group-profile-open-enrollment",
    ] {
        assert!(has(&texts, key), "{key} in {texts:?}");
    }
    assert!(!has(&texts, "ui-group-profile-join"), "no join for a member");
    assert!(!has(&texts, "ui-group-profile-invite"), "inviting needs the ability");
    // the data is asked once, as a member
    let loads: Vec<_> = actions.iter().filter(|a| matches!(a, GroupAction::Load { .. })).collect();
    assert!(matches!(loads[..], [GroupAction::Load { member: true, .. }]));
    // the three latest notices of four, and the member count
    let notices = aurora_i18n::tr!("demo-group-notice-photos");
    assert!(!texts.iter().any(|t| t == notices), "the fourth notice waits for the Notices tab");
    assert!(texts.iter().any(|t| t == aurora_i18n::tr!("demo-group-notice-workshop")));
}

#[test]
fn a_guest_can_join_an_open_group_and_has_no_settings() {
    let mut world = world(false, 0);
    let mut st = opened(&mut world);
    let (texts, actions) = texts(&mut st, &world, false);
    let join = aurora_i18n::tr!("ui-group-profile-join-fee", fee = aurora_i18n::integer(250));
    assert!(texts.contains(&join), "{join} in {texts:?}");
    for key in [
        "common-chat",
        "ui-group-profile-leave",
        "ui-group-profile-me",
        "ui-group-profile-latest-notices",
    ] {
        assert!(!has(&texts, key), "{key} is for members");
    }
    assert!(actions.iter().any(|a| matches!(a, GroupAction::Load { member: false, .. })));

    // invitation only: no join button (LLPanelGroup::update)
    let mut closed = world;
    if let Some(g) = closed.search.groups.get_mut(&DEMO_GROUP1) {
        g.open_enrollment = false;
    }
    let (texts, _) = self::texts(&mut st, &closed, false);
    assert!(!texts.contains(&join));
    assert!(has(&texts, "ui-group-profile-invitation-only"));
}

#[test]
fn managing_members_shows_invite_roles_and_donations() {
    let officer = MEMBER | powers::MEMBER_INVITE | powers::ROLE_ASSIGN_MEMBER_LIMITED;
    for (abilities, manages) in [(MEMBER, false), (officer, true)] {
        let mut world = world(true, abilities);
        let mut st = opened(&mut world);
        st.tab = Tab::Members;
        let (texts, _) = texts(&mut st, &world, false);
        let column = |key: &'static str| {
            texts
                .iter()
                .filter(|t| t.starts_with(&aurora_i18n::text(key).to_uppercase()))
                .count()
        };
        assert!(
            column("ui-group-profile-col-member") > 0
                && column("ui-group-profile-col-title") > 0
                && column("ui-group-profile-col-status") > 0
        );
        // "ROLES" is also the title of the card beside the list
        assert_eq!(column("ui-group-profile-col-roles") > 1, manages, "{texts:?}");
        assert_eq!(column("ui-group-profile-col-donation") > 0, manages);
        assert_eq!(has(&texts, "ui-group-profile-invite"), manages);
    }
}

#[test]
fn the_classic_window_keeps_the_general_pane_and_adds_the_members() {
    let mut world = world(true, MEMBER);
    let mut st = opened(&mut world);
    let (texts, _) = texts(&mut st, &world, true);
    for key in [
        "search-group-founder",
        "search-group-charter",
        "search-group-enrollment",
        "ui-group-profile-members-and-roles",
    ] {
        assert!(has(&texts, key), "{key} in {texts:?}");
    }
    assert!(!has(&texts, "ui-group-profile-me"), "the cards are the Aurora look's");
}

#[test]
fn joining_checks_the_group_limit_and_the_balance() {
    let mut world = world(false, 0);
    let mut st = opened(&mut world);
    let v = view::View::of(&world, DEMO_GROUP1);
    // L$ 250 with L$ 100: cannot afford
    let mut poor = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    poor.balance = Some(100);
    st.ask_join(&poor, &v, DEMO_GROUP1);
    assert!(matches!(st.confirm, Some(Confirm::CannotAfford { fee: 250, .. })));
    poor.balance = Some(1000);
    st.ask_join(&poor, &v, DEMO_GROUP1);
    assert!(matches!(st.confirm, Some(Confirm::Join { fee: 250, .. })));
}

#[test]
fn members_are_filtered_and_sorted_online_first() {
    let world = world(true, MEMBER);
    let v = view::View::of(&world, DEMO_GROUP1);
    let names = |id: &Uuid| world.social.display_name_of(id);
    let all = members::rows(&v, &names, "", members::Show::All, members::Sort::Status, false);
    assert_eq!(all.len(), 1248);
    let online = members::rows(&v, &names, "", members::Show::Online, members::Sort::Status, false);
    assert_eq!(online.len(), 5);
    let data = v.data.expect("data");
    assert!(
        all[..5]
            .iter()
            .all(|(id, _)| data.members[id].last_seen == aurora_net::groups::LastSeen::Online)
    );
    // by title: the founder's title
    let founders = members::rows(
        &v,
        &names,
        aurora_i18n::tr!("demo-group-founder"),
        members::Show::All,
        members::Sort::Name,
        false,
    );
    assert!(founders.iter().any(|(id, _)| *id == crate::demo::DEMO_LOUP));
    // by role
    let owners_role = data.roles.values().find(|r| r.name == "Owners").map(|r| r.id).expect("owners");
    let owners = members::rows(&v, &names, "", members::Show::Role(owners_role), members::Sort::Name, false);
    assert_eq!(owners.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [crate::demo::DEMO_LOUP]);
}

#[test]
fn status_reads_online_recent_days_and_dates() {
    use aurora_net::groups::LastSeen;
    let today = (2026, 10, 14);
    assert_eq!(
        members::status_text(&LastSeen::Online, today),
        aurora_i18n::tr!("ui-group-profile-status-online")
    );
    assert_eq!(
        members::status_text(&LastSeen::Date(2026, 10, 13), today),
        aurora_i18n::tr!("ui-chat-yesterday").to_lowercase()
    );
    assert_eq!(
        members::status_text(&LastSeen::Date(2026, 10, 11), today),
        aurora_i18n::tr!("ui-group-profile-days-ago", count = 3)
    );
    assert_eq!(
        members::status_text(&LastSeen::Date(2026, 9, 12), today),
        aurora_i18n::tr!("profile-date", month = "09", day = "12", year = "2026")
    );
}

#[test]
fn invitations_skip_members_and_ask_before_making_owners() {
    let officer = MEMBER | powers::MEMBER_INVITE | powers::ROLE_ASSIGN_MEMBER_LIMITED;
    let mut world = world(true, officer);
    // the demo group's owners' role
    let owners = world
        .group_data
        .get(&DEMO_GROUP1)
        .and_then(|d| d.roles.values().find(|r| r.name == "Owners").map(|r| r.id));
    if let (Some(g), Some(o)) = (world.search.groups.get_mut(&DEMO_GROUP1), owners) {
        g.owner_role = o;
    }
    let v = view::View::of(&world, DEMO_GROUP1);
    let outsider = Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0064);
    let mut st = invite::InviteUi::default();
    st.open(DEMO_GROUP1, vec![outsider, crate::demo::DEMO_LOUP]);
    // everyone's role: the member is left out and said so
    let pairs = st.submit(&v).expect("sent");
    assert_eq!(pairs, vec![(outsider, Uuid::nil())]);
    assert_eq!(st.notice, Some(invite::Notice::AlreadyInGroup));

    // the owners' role waits for a yes
    st.open(DEMO_GROUP1, vec![outsider]);
    st.role = owners.expect("owners");
    assert!(st.submit(&v).is_none(), "asks first");
    assert_eq!(st.submit(&v).expect("confirmed"), vec![(outsider, st.role)]);

    // more than 100 at once: nothing goes
    st.open(DEMO_GROUP1, (0..101u128).map(|i| Uuid::from_u128(0xABC0_0000 + i)).collect());
    st.role = Uuid::nil();
    assert_eq!(st.submit(&v), Some(Vec::new()));
    assert_eq!(st.notice, Some(invite::Notice::TooMany));
}

#[test]
fn invite_shows_with_the_ability_and_call_waits_for_group_voice() {
    for (abilities, invites) in [(MEMBER, false), (MEMBER | powers::MEMBER_INVITE, true)] {
        let mut world = world(true, abilities);
        let mut st = opened(&mut world);
        let (texts, _) = texts(&mut st, &world, false);
        assert_eq!(has(&texts, "ui-group-profile-invite"), invites);
        assert!(has(&texts, "ui-group-profile-call"));
    }
}
