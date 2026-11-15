use super::*;
use crate::world::portals::tests::{fixture, point, portal, wire};

#[test]
fn recreated_sources_reuse_the_first_route_geometry_lifetime_and_gate_clearance() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    assert!(state.share_return(portal(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    let original = state.entries[0].portal.clone();
    let deadline = state.entries[0].expires_at;
    for i in 1..20 {
        let mut recreated = portal();
        recreated.id = Uuid::from_u128(100 + i);
        recreated.center[0] += 0.2;
        recreated.position[0] += 0.2;
        recreated.destination = recreated.destination.to_uppercase();
        recreated.expires += i as u64;
        assert!(state.share_return(
            recreated,
            creator,
            BuildRight::Allowed,
            point(0.2),
            now + Duration::from_millis(i as u64),
            1000
        ));
        assert_eq!(state.entries.len(), 1);
        let entry = &state.entries[0];
        assert_eq!(entry.portal, original);
        assert_eq!(entry.expires_at, deadline);
        assert_eq!(entry.details, Some(portal().details()));
        assert!(!entry.ready_to_cross(now));
        assert!(entry.reveals_arrival());
        assert!(entry.allowed(&world));
    }
    assert_eq!(state.out.len(), 1, "reuse does not flood the bridge");
}

#[test]
fn conflicting_provenances_creators_and_private_routes_cannot_stack() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    assert!(state.share_return(portal(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    for i in 0..20 {
        let mut conflicting = portal();
        conflicting.id = Uuid::from_u128(100 + i);
        conflicting.position[0] += 10.0;
        assert!(!state.share_return(conflicting.clone(), creator, BuildRight::Allowed, point(0.15), now, 1000));
        let mut announcement = conflicting.announcement();
        announcement.creator = Some(Uuid::from_u128(200 + i));
        state.receive(&world, &wire(Uuid::from_u128(300 + i), Message::Open(announcement)), now, 1000);
    }
    let mut private = portal();
    private.id = Uuid::from_u128(8);
    private.access = Access::Friends;
    assert!(!state.share_return(private, creator, BuildRight::Allowed, point(0.15), now, 1000));
    assert_eq!(state.entries.len(), 1);
    let mut distant = portal();
    distant.id = Uuid::from_u128(9);
    distant.center[0] += 4.0;
    assert!(state.share_return(distant, creator, BuildRight::Allowed, point(4.15), now, 1000));
    assert_eq!(state.visible(&world).count(), 2);
}

#[test]
fn an_arrival_must_clear_the_return_before_voluntarily_crossing_back() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    assert!(state.share_return(portal(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    for (frame, offset) in [0.0, -0.3, 0.15, 1.2, 0.3].into_iter().enumerate() {
        let previous = world.agent.position;
        world.agent.position = point(offset);
        assert!(
            state
                .update(
                    &world,
                    world.agent.position - previous,
                    now + Duration::from_millis(frame as u64 * 16)
                )
                .is_none()
        );
    }
    world.agent.position = point(-0.3);
    assert_eq!(
        state.update(&world, -Vec3::X, now + Duration::from_millis(96)),
        Some((creator, portal()))
    );
    assert!(
        !state.entries[0].reveals_arrival(),
        "a later crossing must not restart the arrival reveal"
    );
}

#[test]
fn a_later_normal_source_has_priority_for_drawing_and_crossing() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    assert!(state.share_return(portal(), Uuid::from_u128(2), BuildRight::Allowed, point(0.15), now, 1000));
    let mut source = portal();
    source.id = Uuid::from_u128(8);
    let owner = Uuid::from_u128(3);
    state.receive(&world, &wire(owner, Message::Open(source.announcement())), now, 1000);
    assert_eq!(state.entries.len(), 2);
    assert_eq!(state.visible(&world).map(|e| e.portal.id).collect::<Vec<_>>(), vec![source.id]);
    assert_eq!(state.apertures().count(), 1);
    world.agent.position = point(1.5);
    assert!(state.update(&world, Vec3::X, now + Duration::from_millis(700)).is_none());
    world.agent.position = point(-0.1);
    assert_eq!(
        state.update(&world, -Vec3::X, now + Duration::from_millis(716)),
        Some((owner, source))
    );
}

#[test]
fn a_hidden_source_does_not_suppress_the_travelers_own_return() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    assert!(state.share_return(portal(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    let mut source = portal();
    source.id = Uuid::from_u128(8);
    let stranger = Uuid::from_u128(3);
    state.receive(&world, &wire(stranger, Message::Open(source.announcement())), now, 1000);
    world.portal_sources = Sources::FriendsAndGroups;
    state.sync(&world, None, 1000);
    assert_eq!(state.entries.len(), 2, "unknown group membership keeps the source hidden");
    assert_eq!(state.shown(&world).map(|e| e.portal.id).collect::<Vec<_>>(), vec![portal().id]);
    assert!(state.crossable(&world, stranger, source.id).is_none());
    assert!(state.crossable(&world, creator, portal().id).is_some());
    world.social.form_friendship(stranger);
    assert_eq!(state.shown(&world).map(|e| e.portal.id).collect::<Vec<_>>(), vec![source.id]);
    assert!(state.crossable(&world, creator, portal().id).is_none());
}

#[test]
fn reusing_a_normal_source_keeps_its_priority_and_relays_the_correct_creator() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    assert!(state.share_return(portal(), Uuid::from_u128(2), BuildRight::Allowed, point(0.15), now, 1000));
    let mut source = portal();
    source.id = Uuid::from_u128(8);
    let creator = Uuid::from_u128(3);
    state.receive(&world, &wire(creator, Message::Open(source.announcement())), now, 1000);
    assert!(state.share_return(source.clone(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    assert_eq!(state.apertures().map(|e| e.portal.id).collect::<Vec<_>>(), vec![source.id]);
    let shared = state.entries.iter().find(|e| e.portal.id == source.id).unwrap();
    assert!(shared.portal.creator.is_none(), "the stored source keeps priority over old returns");
    assert_eq!(shared.shared_announcement().creator, Some(creator));
    assert!(
        state
            .out
            .iter()
            .any(|(_, message)| matches!(message, Message::Open(p) if p.id == source.id && p.creator == Some(creator)))
    );
}

#[test]
fn a_return_pass_cannot_grant_the_relays_friends_private_access() {
    let (mut relay, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    let stranger = Uuid::from_u128(3);
    relay.social.form_friendship(stranger);
    let mut private = portal();
    private.access = Access::Friends;
    assert!(state.share_return(private.clone(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    assert!(state.entries[0].allowed(&relay));
    state.receive(
        &relay,
        &wire(
            stranger,
            Message::Request {
                region: 256,
                id: private.id,
                nonce: Uuid::from_u128(9),
            },
        ),
        now,
        1000,
    );
    state.update(&relay, Vec3::ZERO, now);
    assert!(state.out.iter().all(|(_, m)| !matches!(m, Message::Details { .. })));
    let (mut observer, mut client) = fixture();
    observer.agent_id = stranger;
    observer.social.form_friendship(relay.agent_id);
    client.sync(&observer, None, 1000);
    let mut announcement = private.announcement();
    announcement.creator = Some(creator);
    client.receive(&observer, &wire(relay.agent_id, Message::Open(announcement)), now, 1000);
    assert!(!client.entries[0].allowed(&observer));
    assert!(client.entries[0].details.is_none());
    assert_eq!(client.entries[0].label_owner(), relay.agent_id);
}

#[test]
fn expired_returns_make_room_without_extending_the_old_invitation() {
    let (_world, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(2);
    assert!(state.share_return(portal(), creator, BuildRight::Allowed, point(0.15), now, 1000));
    let mut next = portal();
    next.id = Uuid::from_u128(8);
    next.expires = 1241;
    assert!(state.share_return(
        next.clone(),
        creator,
        BuildRight::Allowed,
        point(0.15),
        now + Duration::from_secs(121),
        1121
    ));
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].portal.id, next.id);
    assert_eq!(state.entries[0].portal.expires, 1241);
}
