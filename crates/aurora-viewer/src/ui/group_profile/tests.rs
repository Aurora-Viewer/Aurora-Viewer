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
    let (texts, actions, _) = frames(st, world, classic);
    (texts, actions)
}

/// The names of the second frame's buttons, an icon alone's included (its
/// name for a screen reader).
fn buttons(st: &mut GroupProfileUi, world: &World) -> Vec<String> {
    frames(st, world, false).2
}

fn frames(st: &mut GroupProfileUi, world: &World, classic: bool) -> (Vec<String>, Vec<GroupAction>, Vec<String>) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
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
    let mut buttons = Vec::new();
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
        buttons = output
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter(|(_, n)| n.role() == egui::accesskit::Role::Button)
                    .filter_map(|(_, n)| n.label().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
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
    (out.into_iter().map(|t| t.2).collect(), actions, buttons)
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
fn search_groups_join_confirms_without_opening_a_profile() {
    let mut world = world(false, 0);
    world.balance = Some(1000);
    let mut st = GroupProfileUi::default();
    st.join(&world, DEMO_GROUP1);
    assert!(matches!(
        st.confirm,
        Some(Confirm::Join {
            group: DEMO_GROUP1,
            fee: 250,
            ..
        })
    ));
    assert!(st.shown().is_none());
    let (drawn, actions) = texts(&mut st, &world, false);
    assert!(
        has(&drawn, "ui-group-profile-join-title"),
        "the confirmation remains visible without a profile"
    );
    assert!(
        !actions.iter().any(|a| matches!(a, GroupAction::Join(_))),
        "no request before confirmation"
    );
    world.balance = Some(100);
    st.join(&world, DEMO_GROUP1);
    assert!(matches!(st.confirm, Some(Confirm::CannotAfford { fee: 250, .. })));
    st.confirm = None;
    world.search.groups.get_mut(&DEMO_GROUP1).expect("profile").open_enrollment = false;
    st.join(&world, DEMO_GROUP1);
    st.join(&world, Uuid::nil());
    assert!(st.confirm.is_none(), "no join without a known open enrollment");

    world.search.groups.get_mut(&DEMO_GROUP1).expect("profile").open_enrollment = true;
    world.search.groups.get_mut(&DEMO_GROUP1).expect("profile").membership_fee = 0;
    world.balance = None;
    st.join(&world, DEMO_GROUP1);
    assert!(
        matches!(st.confirm, Some(Confirm::Join { fee: 0, .. })),
        "free enrollment needs no balance"
    );

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
    world.groups.groups.push(aurora_net::GroupMembership {
        id: crate::demo::DEMO_GROUP2,
        name: "Another group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    st.join(&world, DEMO_GROUP1);
    assert!(matches!(st.confirm, Some(Confirm::TooManyGroups)));
    st.confirm = None;
    world.groups.groups[0].id = DEMO_GROUP1;
    st.join(&world, DEMO_GROUP1);
    assert!(st.confirm.is_none(), "members cannot join again");
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
        // named, or their icons alone in a narrow head (#1089)
        let names = buttons(&mut st, &world);
        assert_eq!(has(&names, "ui-group-profile-invite"), invites, "{names:?}");
        assert!(has(&names, "ui-group-profile-call"), "{names:?}");
    }
}

/// Feed the demo server's answers to the commands of the group data.
fn pump(world: &mut World) {
    for _ in 0..3 {
        let mut cmds = world.group_data.take_commands();
        cmds.extend(world.take_social_commands());
        for cmd in cmds {
            for ev in crate::demo::group::reply(&cmd).unwrap_or_default() {
                world.apply(ev);
            }
        }
    }
}

#[test]
fn a_notice_opens_with_its_attachment_and_is_saved_once() {
    let mut world = world(true, MEMBER);
    let first = world
        .group_data
        .get(&DEMO_GROUP1)
        .and_then(|d| d.notices.as_ref())
        .and_then(|l| l.first().map(|n| n.id))
        .expect("notices");
    world.group_data.open_notice(DEMO_GROUP1, first);
    pump(&mut world);
    let n = world
        .group_data
        .get(&DEMO_GROUP1)
        .and_then(|d| d.notice.clone())
        .expect("notice shown");
    assert_eq!(n.id, first);
    assert_eq!(n.subject, aurora_i18n::tr!("demo-group-notice-workshop"));
    assert_eq!(n.attachment.as_ref().map(|a| a.1), Some(6));
    assert!(world.group_data.is_read(&DEMO_GROUP1, &first));
    world.save_notice_attachment(DEMO_GROUP1);
    let cmds = world.group_data.take_commands();
    assert!(
        matches!(&cmds[..], [aurora_net::NetCommand::SendImDialog { dialog: 33, .. }]),
        "{cmds:?}"
    );
    world.save_notice_attachment(DEMO_GROUP1);
    assert!(world.group_data.take_commands().is_empty(), "saved once");
}

#[test]
fn ejects_bans_and_role_ticks_follow_firestorm() {
    use super::admin_members::{self, Me};
    let world = world(true, MEMBER);
    let d = world.group_data.get(&DEMO_GROUP1).expect("data");
    let role = |name: &str| d.roles.values().find(|r| r.name == name).map(|r| r.id).expect("role");
    let (owners, officers, builders) = (role("Owners"), role("Officers"), role("Builders"));
    let plain = d
        .members
        .keys()
        .copied()
        .find(|m| d.member_roles.get(m).is_some_and(|r| r.iter().all(|r| r.is_nil())))
        .expect("an everyone-only member");
    let founder = crate::demo::DEMO_LOUP;
    let officer = Me {
        id: crate::demo::DEMO_TESS,
        powers: powers::MEMBER_EJECT | powers::GROUP_BAN_ACCESS | powers::ROLE_ASSIGN_MEMBER_LIMITED,
        owner_role: owners,
    };
    // an officer ejects and bans a member in no role but "everyone"
    assert_eq!(admin_members::eject_and_ban(d, &officer, &[plain]), (true, true));
    // not someone in another role, never themselves
    assert!(!admin_members::eject_and_ban(d, &officer, &[founder]).0);
    assert_eq!(admin_members::eject_and_ban(d, &officer, &[officer.id]), (false, false));
    // an owner ejects anyone but another owner
    let owner = Me {
        id: DEMO_AGENT,
        powers: u64::MAX,
        owner_role: owners,
    };
    let mut d2 = d.clone();
    d2.member_roles.entry(DEMO_AGENT).or_default().push(owners);
    assert!(admin_members::eject_and_ban(&d2, &owner, &[crate::demo::DEMO_NOVA]).0);
    assert_eq!(admin_members::eject_and_ban(&d2, &owner, &[founder]), (false, false));

    // ticks: the officer adds to the officers' role (theirs), not the owners'
    let checks = admin_members::role_checks(d, &officer, &[plain], &[]);
    let enabled = |r: Uuid| checks.iter().find(|c| c.role == r).map(|c| c.enabled);
    assert_eq!(enabled(Uuid::nil()), Some(false), "nobody leaves everyone");
    assert_eq!(enabled(officers), Some(true));
    assert_eq!(enabled(owners), Some(false));
    assert_eq!(enabled(builders), Some(false), "not one of the officer's roles");
    // a click adds, a second one cancels the change
    let mut staged = Vec::new();
    admin_members::toggle_role(d, &[plain], &mut staged, officers);
    assert_eq!(
        staged,
        vec![aurora_net::groups::RoleChange {
            role: officers,
            member: plain,
            add: true
        }]
    );
    let (some, _) = admin_members::powers_of(d, &[plain], &staged);
    assert!(some & powers::MEMBER_INVITE != 0, "an officer's abilities once saved");
    admin_members::toggle_role(d, &[plain], &mut staged, officers);
    assert!(staged.is_empty());

    // the export: a title line, then one line per member
    let csv = admin_members::export_csv("Aurora Builders", d, &|_| "Name".into());
    assert!(csv.starts_with("Group membership record for Aurora Builders"));
    assert_eq!(csv.trim_end().lines().count(), 1 + d.members.len());
}

#[test]
fn role_changes_follow_set_role_data() {
    use super::administration::{self, MAX_ROLES};
    use aurora_net::groups::role_change;
    let world = world(true, MEMBER);
    let d = world.group_data.get(&DEMO_GROUP1).expect("data");
    let owners = d.roles.values().find(|r| r.name == "Owners").map(|r| r.id).expect("owners");
    let builders = d.roles.values().find(|r| r.name == "Builders").cloned().expect("builders");
    let mut changes = std::collections::BTreeMap::new();
    // a title: a data change; the abilities too: both; back: no change
    let mut r = builders.clone();
    r.title = "Maker".into();
    administration::set_role(d, &mut changes, r.clone());
    assert_eq!(changes[&builders.id].kind, role_change::DATA);
    r.powers |= powers::MEMBER_INVITE;
    administration::set_role(d, &mut changes, r);
    assert_eq!(changes[&builders.id].kind, role_change::ALL);
    administration::set_role(d, &mut changes, builders.clone());
    assert!(changes.is_empty(), "back to the saved role");
    // a new role and its copy, up to MAX_ROLES
    let new = administration::create_role(d, &mut changes, owners, None).expect("room");
    assert_eq!(changes[&new].kind, role_change::CREATE);
    assert_eq!(changes[&new].name, "New Role");
    let copy = administration::create_role(d, &mut changes, owners, Some(&builders)).expect("room");
    assert_eq!(changes[&copy].name, "Builders(Copy)");
    while administration::create_role(d, &mut changes, owners, None).is_some() {}
    assert_eq!(administration::roles_now(d, &changes, owners).len(), MAX_ROLES);
    // an edit of a new role stays a creation; deleting it forgets it
    let mut n = administration::role_now(d, &changes, &new).expect("new");
    n.title = "T".into();
    administration::set_role(d, &mut changes, n);
    assert_eq!(changes[&new].kind, role_change::CREATE);
    assert!(administration::delete_role(d, &mut changes, owners, new));
    assert!(!changes.contains_key(&new));
    // everyone and the owners stay; another is deleted
    assert!(!administration::delete_role(d, &mut changes, owners, Uuid::nil()));
    assert!(!administration::delete_role(d, &mut changes, owners, owners));
    assert!(administration::delete_role(d, &mut changes, owners, builders.id));
    assert_eq!(changes[&builders.id].kind, role_change::DELETE);
    // the ban ability brings eject and remove-from-roles
    let p = administration::set_power(0, powers::GROUP_BAN_ACCESS.trailing_zeros(), true);
    assert!(p & powers::MEMBER_EJECT != 0 && p & powers::ROLE_REMOVE_MEMBER != 0);
}

#[test]
fn bans_by_name_skip_ourselves_the_banned_and_the_full_list() {
    let mut world = world(true, MEMBER);
    world.apply(NetEvent::GroupBanList {
        group: DEMO_GROUP1,
        bans: vec![(Uuid::from_u128(77), 0.0)],
    });
    let done = world
        .group_data
        .ban_residents(DEMO_GROUP1, vec![DEMO_AGENT, Uuid::from_u128(77), Uuid::from_u128(78)], DEMO_AGENT);
    assert!(done.myself);
    assert_eq!(done.already, vec![Uuid::from_u128(77)]);
    assert_eq!(done.banned, vec![Uuid::from_u128(78)]);
    let cmds = world.group_data.take_commands();
    assert!(cmds.iter().any(|c| matches!(
        c,
        aurora_net::NetCommand::GroupBans {
            action: Some(1),
            refresh: true,
            ..
        }
    )));
    assert!(cmds.iter().any(|c| matches!(c, aurora_net::NetCommand::EjectGroupMembers { .. })));
}

/// What the "Land and L$" tab asked, answered by the demo group server.
fn answer_land(world: &mut World, actions: &[GroupAction]) {
    for a in actions {
        match a {
            GroupAction::LoadLand { group, ledger } => world.group_money.open(*group, *ledger),
            GroupAction::LandPage { group, ledger } => world.group_money.request(*group, *ledger),
            _ => {}
        }
    }
    for cmd in world.group_money.take_commands() {
        for ev in crate::demo::group::reply(&cmd).unwrap_or_default() {
            world.apply(ev);
        }
    }
}

#[test]
fn the_land_tab_shows_the_totals_the_parcels_and_this_week() {
    use aurora_net::group_money::Ledger;
    let mut world = world(true, MEMBER);
    world.apply(NetEvent::LandHoldings {
        credit: 4096,
        committed: 3584,
    });
    let mut st = opened(&mut world);
    st.show_land(Ledger::Planning);
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(has(&texts, "common-loading-2"), "{texts:?}");
    let loads = actions.iter().filter(|a| matches!(a, GroupAction::LoadLand { .. })).count();
    assert_eq!(loads, 1, "asked once when the tab shows");
    answer_land(&mut world, &actions);
    let (texts, actions) = self::texts(&mut st, &world, false);
    assert!(
        !actions.iter().any(|a| matches!(a, GroupAction::LoadLand { .. })),
        "not again while it shows"
    );
    let area = |n: i64| aurora_i18n::tr!("ui-group-profile-area", area = aurora_i18n::integer(n));
    for want in [
        area(4608),
        area(5120),
        aurora_i18n::tr!("demo-group-land-sandbox").to_owned(),
        aurora_i18n::tr!("demo-group-land-workshop").to_owned(),
        land::lindens(1250),
        land::lindens(-300),
        // our contribution (0 here) and the 512 m² we hold and give to nobody
        aurora_i18n::tr!("ui-group-land-of-max", max = aurora_i18n::integer(512)),
    ] {
        assert!(texts.iter().any(|t| t.contains(&want)), "{want} in {texts:?}");
    }
    assert!(has(&texts, "ui-group-land-money"));
    assert!(has(&texts, "ui-group-land-balance"));

    // the sales page: both packets of this week
    st.land.ledger = Ledger::Sales;
    answer_land(
        &mut world,
        &[GroupAction::LandPage {
            group: DEMO_GROUP1,
            ledger: Ledger::Sales,
        }],
    );
    let (texts, _) = self::texts(&mut st, &world, false);
    let lune = format!("Lune {}", aurora_i18n::tr!("demo-example"));
    assert!(texts.iter().any(|t| t.contains(&lune)), "the second packet in {texts:?}");
    assert!(texts.iter().any(|t| t.contains(aurora_i18n::tr!("demo-group-notice-workshop-kit"))));
}

#[test]
fn a_guest_has_no_land_tab() {
    let mut world = world(false, 0);
    let mut st = opened(&mut world);
    st.show_land(aurora_net::group_money::Ledger::Planning);
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(!actions.iter().any(|a| matches!(a, GroupAction::LoadLand { .. })));
    assert!(!has(&texts, "ui-group-land-money"));
}

#[test]
fn the_experiences_tab_lists_what_the_group_owns() {
    let mut world = world(true, MEMBER);
    let mut st = opened(&mut world);
    st.show_experiences();
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(has(&texts, "ui-group-experiences-loading"), "{texts:?}");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, GroupAction::LoadExperiences(g) if *g == DEMO_GROUP1))
    );
    // the list, then the details of its experience
    world.experiences.request_group(DEMO_GROUP1);
    for _ in 0..2 {
        for cmd in world.experiences.take_commands() {
            for ev in crate::demo::group::reply(&cmd).unwrap_or_default() {
                world.apply(ev);
            }
        }
    }
    let (texts, actions) = self::texts(&mut st, &world, false);
    assert!(
        !actions.iter().any(|a| matches!(a, GroupAction::LoadExperiences(_))),
        "not again while it shows"
    );
    assert!(texts.iter().any(|t| t == aurora_i18n::tr!("demo-group-experience")), "{texts:?}");
    assert!(has(&texts, "ui-group-experiences-profile"));
    let scope = format!(
        "{} · {}",
        aurora_i18n::tr!("ui-group-experiences-owned"),
        aurora_i18n::tr!("ui-group-experiences-land")
    );
    assert!(texts.contains(&scope), "{texts:?}");
    // the tab's count
    assert!(texts.iter().any(|t| t == "1"));
}

#[test]
fn a_guest_has_no_experiences_tab() {
    let mut world = world(false, 0);
    let mut st = opened(&mut world);
    st.show_experiences();
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(!actions.iter().any(|a| matches!(a, GroupAction::LoadExperiences(_))));
    assert!(!has(&texts, "ui-group-experiences-loading"));
}

#[test]
fn the_creation_window_shows_the_identity_cards_and_the_fees() {
    let world = world(true, MEMBER);
    let mut st = GroupProfileUi::default();
    st.create.open(&world);
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(actions.is_empty());
    for key in [
        "ui-group-create-title",
        "ui-group-create-create",
        "ui-group-profile-charter",
        "ui-group-admin-settings",
        "ui-group-create-rating",
        "ui-group-create-rating-to-choose",
        "ui-group-create-fee-title",
        "ui-group-create-members-title",
    ] {
        assert!(has(&texts, key), "{key} in {texts:?}");
    }
    // the insignia is chosen on the header, not in a card
    assert!(!has(&texts, "ui-group-admin-insignia-change"));
}

#[test]
fn the_classic_creation_window_is_firestorm_s_panel() {
    let world = world(true, MEMBER);
    let mut st = GroupProfileUi::default();
    st.create.open(&world);
    let (texts, _) = texts(&mut st, &world, true);
    for key in [
        "ui-group-create-classic-title",
        "ui-group-create-classic-name",
        "ui-group-create-anyone-can-join",
        "ui-group-create-cost-to-join",
        "ui-group-create-select-rating",
        "ui-group-create-classic-note",
    ] {
        assert!(has(&texts, key), "{key} in {texts:?}");
    }
}

#[test]
fn create_asks_the_rating_then_sends_the_trimmed_name() {
    use create::{DemoStart, DemoThen, Draft};
    let world = world(true, MEMBER);
    let mut st = GroupProfileUi::default();
    let draft = Draft {
        name: "  Lagoon Painters ".into(),
        ..Default::default()
    };
    st.create.demo(DemoStart {
        draft: Some(draft.clone()),
        then: DemoThen::Press,
    });
    let (texts, actions) = texts(&mut st, &world, false);
    assert!(has(&texts, "ui-group-create-moderate-question"), "{texts:?}");
    assert!(actions.is_empty(), "nothing leaves before the rating");

    let mut st = GroupProfileUi::default();
    st.create.demo(DemoStart {
        draft: Some(Draft {
            rating: Some(true),
            ..draft
        }),
        then: DemoThen::Send,
    });
    let (texts, actions) = self::texts(&mut st, &world, false);
    let sent: Vec<_> = actions
        .iter()
        .filter_map(|a| match a {
            GroupAction::Create { name, info } => Some((name.clone(), info.mature_publish, info.show_in_list)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![("Lagoon Painters".to_owned(), true, true)], "sent once");
    assert!(has(&texts, "ui-group-create-creating"), "Create waits for the reply");
}

/// Draw the group windows with what `Ctx` needs, for the frames tests.
fn show_group(st: &mut GroupProfileUi, world: &World, ctx: &egui::Context, p: &crate::theme::Palette) {
    let images = HashMap::new();
    let pics = HashMap::new();
    let mut emoji = crate::ui::emoji::Emoji::load();
    let (mut names, mut wanted, mut groups) = (HashSet::new(), HashSet::new(), HashSet::new());
    let env = Env {
        avatar_pics: &pics,
        classic: false,
        now: 0.0,
        group_calls: false,
    };
    let mut c = Ctx {
        p,
        world,
        images: &images,
        emoji: &mut emoji,
        want_names: &mut names,
        wanted_images: &mut wanted,
        want_groups: &mut groups,
    };
    st.show(ctx, &mut c, &env);
}

/// The invitation keeps its size with residents to invite and its role.
#[test]
fn the_invitation_keeps_its_size() {
    let world = world(true, MEMBER | powers::MEMBER_INVITE);
    crate::ui::widgets::steady_tests::assert_window_steady(
        invite::WINDOW,
        &[egui::Vec2::new(380.0, 460.0), egui::Vec2::new(320.0, 360.0)],
        aurora_i18n::tr!("ui-group-invite-send"),
        || {
            let mut st = GroupProfileUi::default();
            st.invite.open(DEMO_GROUP1, (1..6).map(|i| Uuid::from_u128(0x7000 + i)).collect());
            st
        },
        |st, ctx, p| show_group(st, &world, ctx, p),
    );
}

/// The group picker of "Invite to group" keeps its size.
#[test]
fn the_group_picker_keeps_its_size() {
    let world = world(true, MEMBER | powers::MEMBER_INVITE);
    crate::ui::widgets::steady_tests::assert_window_steady(
        "group_invite_choice",
        &[egui::Vec2::new(300.0, 320.0), egui::Vec2::new(240.0, 220.0)],
        aurora_i18n::tr!("common-ok"),
        || {
            let mut st = GroupProfileUi::default();
            st.invite.choose_group(vec![Uuid::from_u128(0x7001)]);
            st
        },
        |st, ctx, p| show_group(st, &world, ctx, p),
    );
}
