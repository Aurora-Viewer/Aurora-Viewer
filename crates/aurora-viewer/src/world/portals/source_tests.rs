//! Whose portals are shown (`Sources`), the first-crossing question, and the
//! bounds on what one relay or one claimed creator can hold.
use super::{
    tests::{fixture, point, portal, wire},
    *,
};
use crate::world::mutes::{Mute, MuteType};

fn open(state: &mut State, world: &World, relay: Uuid, announcement: Announcement) {
    state.receive(world, &wire(relay, Message::Open(announcement)), Instant::now(), 1000);
}

fn claimed(id: u128, creator: Uuid) -> Announcement {
    let mut announcement = Portal {
        id: Uuid::from_u128(id),
        ..portal()
    }
    .announcement();
    announcement.creator = Some(creator);
    announcement
}

fn mute(world: &mut World, id: Uuid) {
    let mute = Mute {
        id,
        name: "Muted Resident".into(),
        kind: MuteType::Agent,
        flags: 0,
    };
    world.mutes.add(mute, 0).expect("muted");
}

fn my_group(world: &mut World, group: Uuid) {
    world.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Demo group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
}

#[test]
fn portals_show_friends_by_default_and_our_own_whatever_the_setting() {
    let (mut world, mut state) = fixture();
    world.portal_sources = Sources::default();
    assert_eq!(Sources::default(), Sources::Friends);
    let stranger = Uuid::from_u128(2);
    let friend = Uuid::from_u128(3);
    world.social.form_friendship(friend);
    state.sync(&world, None, 1000);
    open(&mut state, &world, stranger, portal().announcement());
    assert!(state.entries.is_empty(), "a non-friend's portal is not kept");
    open(
        &mut state,
        &world,
        friend,
        Portal {
            id: Uuid::from_u128(8),
            ..portal()
        }
        .announcement(),
    );
    assert_eq!(state.visible(&world).count(), 1);
    assert!(state.entries[0].allowed(&world));
    assert!(
        !state.ask_before_crossing(&world, friend, Uuid::from_u128(8)),
        "a friend's portal asks nothing"
    );

    // Nobody: other residents' portals go, ours and our returns stay.
    assert!(
        state
            .create(
                Portal {
                    id: Uuid::from_u128(9),
                    ..portal()
                },
                BuildRight::Allowed,
                world.agent.position,
                Instant::now(),
                1000
            )
            .is_ok()
    );
    assert!(state.share_return(
        Portal {
            id: Uuid::from_u128(10),
            ..portal()
        },
        stranger,
        BuildRight::Allowed,
        world.agent.position,
        Instant::now(),
        1000
    ));
    world.portal_sources = Sources::Nobody;
    state.sync(&world, None, 1000);
    let ids: Vec<_> = state.visible(&world).map(|entry| entry.portal.id.as_u128()).collect();
    assert_eq!(ids, [9, 10]);
    assert!(state.entries.iter().all(|entry| !entry.opened_by_non_friend(&world)));
    open(
        &mut state,
        &world,
        friend,
        Portal {
            id: Uuid::from_u128(11),
            ..portal()
        }
        .announcement(),
    );
    assert_eq!(state.entries.len(), 2, "nothing new is kept");

    // A wider choice asks the region's bridges to announce again.
    state.discovered = true;
    world.portal_sources = Sources::Everyone;
    state.sync(&world, None, 1000);
    assert!(!state.discovered);
    open(&mut state, &world, stranger, portal().announcement());
    assert_eq!(state.visible(&world).count(), 3);
}

#[test]
fn friends_and_groups_shows_a_resident_once_the_grid_lists_a_shared_group() {
    let (mut world, mut state) = fixture();
    world.portal_sources = Sources::FriendsAndGroups;
    let group = Uuid::from_u128(40);
    my_group(&mut world, group);
    let resident = Uuid::from_u128(2);
    let private = Portal {
        access: Access::Friends,
        ..portal()
    };
    state.sync(&world, None, 1000);
    open(&mut state, &world, resident, private.announcement());
    assert_eq!(state.entries.len(), 1, "kept while their groups are unknown");
    assert_eq!(state.visible(&world).count(), 0);
    assert_eq!(state.shown(&world).count(), 0, "a hidden portal cuts no body");
    assert_eq!(state.take_profile_want(), Some(resident));
    assert_eq!(state.take_profile_want(), None);
    state.update(&world, Vec3::ZERO, Instant::now());
    assert!(
        !state.out.iter().any(|(_, message)| matches!(message, Message::Request { .. })),
        "no request reaches a resident the setting leaves out"
    );

    // Their profile lists one of our groups: shown, asks before crossing.
    world.profiles.merge_groups(
        resident,
        vec![aurora_net::profile::ProfileGroup {
            id: group,
            name: "Demo group".into(),
            insignia: Uuid::nil(),
        }],
    );
    assert_eq!(state.visible(&world).count(), 1);
    assert!(state.ask_before_crossing(&world, resident, private.id));
    state.update(&world, Vec3::ZERO, Instant::now() + SEND_INTERVAL);
    assert!(
        state
            .out
            .iter()
            .any(|(target, message)| *target == resident && matches!(message, Message::Request { .. }))
    );

    // A profile without a shared group hides it; Friends drops it.
    let other = Uuid::from_u128(3);
    world.profiles.merge_groups(other, Vec::new());
    open(
        &mut state,
        &world,
        other,
        Portal {
            id: Uuid::from_u128(8),
            ..portal()
        }
        .announcement(),
    );
    assert_eq!(state.visible(&world).count(), 1);
    world.portal_sources = Sources::Friends;
    state.sync(&world, None, 1000);
    assert!(state.entries.is_empty());
}

#[test]
fn a_non_friends_portal_asks_once_on_its_first_crossing() {
    let (mut world, mut state) = fixture();
    let stranger = Uuid::from_u128(2);
    let friend = Uuid::from_u128(3);
    world.social.form_friendship(friend);
    let p = portal();
    open(&mut state, &world, stranger, p.announcement());
    world.agent.position = point(1.0);
    assert!(
        state
            .update(&world, Vec3::NEG_X, Instant::now() + Duration::from_millis(700))
            .is_none()
    );
    world.agent.position = point(-0.5);
    let crossed = state.update(&world, Vec3::NEG_X, Instant::now() + Duration::from_millis(800));
    assert_eq!(crossed, Some((stranger, p.clone())), "the crossing is reported");
    assert!(state.ask_before_crossing(&world, stranger, p.id), "but waits for the question");
    assert!(state.crossable(&world, stranger, p.id).is_some());
    state.confirm(stranger, p.id);
    assert!(!state.ask_before_crossing(&world, stranger, p.id), "one question per portal");
    assert!(
        state.ask_before_crossing(&world, stranger, Uuid::from_u128(8)),
        "another portal asks again"
    );

    // A friend relaying a return that names a non-friend creator still asks.
    open(&mut state, &world, friend, claimed(12, Uuid::from_u128(20)));
    assert!(state.ask_before_crossing(&world, Uuid::from_u128(20), Uuid::from_u128(12)));
    // Muted meanwhile: nothing left to answer.
    mute(&mut world, stranger);
    state.sync(&world, None, 1000);
    assert!(state.crossable(&world, stranger, p.id).is_none());
}

#[test]
fn one_relay_or_one_claimed_creator_cannot_fill_the_portal_table() {
    let (world, mut state) = fixture();
    let relay = Uuid::from_u128(2);
    for n in 0..20 {
        open(&mut state, &world, relay, claimed(100 + n, Uuid::from_u128(1000 + n)));
    }
    assert_eq!(state.entries.len(), PER_RELAY);
    // A relay at its bound cannot join further returns either.
    let (world, mut state) = fixture();
    let creator = Uuid::from_u128(50);
    for n in 0..20 {
        open(&mut state, &world, Uuid::from_u128(200 + n), claimed(300 + n, creator));
    }
    assert_eq!(state.entries.len(), PER_CREATOR);
    let full = Uuid::from_u128(3);
    for n in 0..PER_RELAY as u128 {
        open(&mut state, &world, full, claimed(400 + n, Uuid::from_u128(500 + n)));
    }
    let relays = state.entries[0].relays.len();
    open(&mut state, &world, full, claimed(300, creator));
    assert_eq!(state.entries[0].relays.len(), relays);
}

#[test]
fn a_relayed_creator_claim_never_names_us_unless_it_joins_our_portal() {
    let (world, mut state) = fixture();
    let relay = Uuid::from_u128(2);
    open(&mut state, &world, relay, claimed(8, world.agent_id));
    assert!(state.entries.is_empty(), "nobody else can announce a portal as ours");
    // A traveler sharing our own same-region portal joins it.
    assert!(
        state
            .create(portal(), BuildRight::Allowed, world.agent.position, Instant::now(), 1000)
            .is_ok()
    );
    let mut ours = portal().announcement();
    ours.creator = Some(world.agent_id);
    open(&mut state, &world, relay, ours);
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].relays, [relay]);
}

#[test]
fn the_relay_passes_the_setting_and_its_mute_with_the_claimed_creator() {
    let (mut world, mut state) = fixture();
    world.portal_sources = Sources::Friends;
    let friend = Uuid::from_u128(20);
    let stranger = Uuid::from_u128(2);
    world.social.form_friendship(friend);
    state.sync(&world, None, 1000);
    open(&mut state, &world, stranger, claimed(8, friend));
    assert!(state.entries.is_empty(), "naming a friend does not carry a stranger's portal");

    world.portal_sources = Sources::Everyone;
    state.sync(&world, None, 1000);
    let (a, b) = (Uuid::from_u128(3), Uuid::from_u128(4));
    open(&mut state, &world, a, claimed(9, friend));
    open(&mut state, &world, b, claimed(9, friend));
    open(&mut state, &world, a, claimed(10, Uuid::from_u128(30)));
    assert_eq!(state.entries.len(), 2);
    mute(&mut world, a);
    state.sync(&world, None, 1000);
    assert_eq!(state.entries.len(), 1, "every entry only the muted relay announced goes");
    assert_eq!(state.entries[0].relays, [b]);
    assert_eq!(state.entries[0].label_owner(), b);
    open(&mut state, &world, a, claimed(11, friend));
    assert_eq!(state.entries.len(), 1, "a muted relay announces nothing new");
}
