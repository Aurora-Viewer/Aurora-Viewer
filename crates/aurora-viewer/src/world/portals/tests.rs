use super::*;
use crate::scene::avatar::AvatarLibrary;
use std::sync::Arc;

pub(super) fn portal() -> Portal {
    Portal {
        id: Uuid::from_u128(7),
        region: 256,
        center: [10.0, 10.0, 25.0],
        yaw: 0.0,
        destination: "Demo destination".into(),
        position: [128.0, 128.0, 25.0],
        access: Access::Everyone,
        expires: 1120,
    }
}
pub(super) fn point(x: f32) -> Vec3 {
    Vec3::new(10.0 + x, 10.0, 25.0)
}

#[test]
fn portal_placement_stays_inside_standard_and_variable_size_regions() {
    assert!(fits_region(Vec3::new(128.0, 128.0, 25.0), [256, 256]));
    assert!(!fits_region(Vec3::new(258.0, 128.0, 25.0), [256, 256]));
    assert!(!fits_region(Vec3::new(128.0, 0.5, 25.0), [256, 256]));
    assert!(fits_region(Vec3::new(500.0, 200.0, 25.0), [512, 256]));
    assert!(!fits_region(Vec3::new(128.0, 300.0, 25.0), [512, 256]));
}
pub(super) fn wire(owner: Uuid, message: Message) -> String {
    format!("<auroraPortal>{owner}|{}</auroraPortal>", message.encode().unwrap())
}
pub(super) fn fixture() -> (World, State) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(1);
    world.main_region = Some(256);
    world.agent.position = point(4.0);
    let mut state = State::default();
    state.sync(&world, None, 1000);
    (world, state)
}

#[test]
fn portal_can_be_entered_immediately_from_nearby_and_after_a_distant_stall() {
    let p = portal().announcement();
    let now = Instant::now();
    for stalled in [false, true] {
        let mut gate = Gate::new(Vec3::from(p.center), point(1.8), now);
        let at = now + Duration::from_millis(if stalled { 2000 } else { 100 });
        if stalled {
            assert!(!gate.sample(&p, point(1.8), true, at));
            assert!(gate.armed);
        }
        assert!(!gate.sample(&p, point(0.4), true, at));
        assert!(gate.sample(&p, point(-0.4), true, at + Duration::from_millis(100)));
    }
}

#[test]
fn portal_return_arrivals_share_one_aperture_and_preserve_the_observers_gate() {
    let (mut observer, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(20);
    let mut announcement = portal().announcement();
    announcement.creator = Some(creator);
    observer.agent.position = point(0.0);
    for peer in [2, 3, 4] {
        state.receive(
            &observer,
            &wire(Uuid::from_u128(peer), Message::Open(announcement.clone())),
            now,
            1000,
        );
    }
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].owner, creator);
    assert_eq!(state.entries[0].relays.len(), 3);
    assert!(state.entries[0].waiting_for_clearance());
    state.receive(
        &observer,
        &wire(
            Uuid::from_u128(2),
            Message::Close {
                region: 256,
                id: announcement.id,
            },
        ),
        now,
        1000,
    );
    assert_eq!(state.entries.len(), 1, "one departing relay does not close the shared return");
    assert!(state.share_return(portal(), creator, observer.agent.position, now, 1000));
    assert_eq!(state.entries.len(), 1);
    assert!(
        state.entries[0].waiting_for_clearance(),
        "a new arrival does not rearm the observer"
    );
    state.receive(&observer, &wire(Uuid::from_u128(5), Message::Discover { region: 256 }), now, 1000);
    assert!(
        state
            .out
            .iter()
            .any(|(peer, m)| *peer == Uuid::from_u128(5) && matches!(m, Message::Open(_)))
    );
}

#[test]
fn private_return_preserves_travelers_rights_without_granting_the_relays_friends() {
    let (mut traveler, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(20);
    let stranger = Uuid::from_u128(3);
    traveler.social.form_friendship(stranger);
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    assert!(state.share_return(p.clone(), creator, traveler.agent.position, now, 1000));
    assert!(state.entries[0].allowed(&traveler), "the traveler retains a personal return");
    assert!(state.entries[0].portal.destination.is_none());
    for peer in [creator, stranger] {
        state.receive(
            &traveler,
            &wire(
                peer,
                Message::Request {
                    region: 256,
                    id: p.id,
                    nonce: peer,
                },
            ),
            now,
            1000,
        );
    }
    state.update(&traveler, Vec3::ZERO, now);
    let grants: Vec<_> = state
        .out
        .iter()
        .filter_map(|(peer, m)| matches!(m, Message::Details { .. }).then_some(*peer))
        .collect();
    assert_eq!(grants, [creator]);
    assert!(
        state
            .out
            .iter()
            .all(|(peer, m)| !peer.is_nil() || !matches!(m, Message::Details { .. }))
    );
}

#[test]
fn private_return_details_require_the_original_creator_and_can_arrive_after_other_travelers() {
    let (mut observer, mut state) = fixture();
    let now = Instant::now();
    let creator = Uuid::from_u128(20);
    let relay = Uuid::from_u128(2);
    observer.social.form_friendship(creator);
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    let mut announcement = p.announcement();
    announcement.creator = Some(creator);
    state.receive(&observer, &wire(relay, Message::Open(announcement.clone())), now, 1000);
    assert_eq!(state.entries[0].label_owner(), relay, "a creator claim cannot impersonate its name");
    state.update(&observer, Vec3::ZERO, now);
    let nonce = state.fetches[&(relay, p.id)].nonce;
    state.receive(
        &observer,
        &wire(
            relay,
            Message::Details {
                region: 256,
                id: p.id,
                nonce,
                details: p.details(),
            },
        ),
        now,
        1000,
    );
    assert!(
        state.entries[0].details.is_none(),
        "a relayed creator claim does not prove friendship"
    );
    state.receive(&observer, &wire(creator, Message::Open(announcement)), now, 1000);
    assert_eq!(state.entries[0].label_owner(), creator);
    state.update(&observer, Vec3::ZERO, now + Duration::from_millis(250));
    let nonce = state.fetches[&(creator, p.id)].nonce;
    state.receive(
        &observer,
        &wire(
            creator,
            Message::Details {
                region: 256,
                id: p.id,
                nonce,
                details: p.details(),
            },
        ),
        now,
        1000,
    );
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].allowed(&observer));
    assert_eq!(state.entries[0].resolved(), Some(p));
}

#[test]
fn photo_isolation_hides_portal_surfaces_without_retiring_invitations_or_clearance() {
    use crate::snapshot::isolate::Mode;
    let (mut world, mut state) = fixture();
    world.agent.position = point(0.0);
    assert!(state.create(portal(), world.agent.position, Instant::now(), 1000));
    assert!(state.own().unwrap().waiting_for_clearance());
    assert_eq!(state.visible(&world).count(), 1);
    world.photo_isolation.open = true;
    for mode in [Mode::Own, Mode::Chosen] {
        world.photo_isolation.mode = mode;
        for transparent in [false, true] {
            world.photo_isolation.transparent = transparent;
            assert_eq!(state.visible(&world).count(), 0);
            // Clipping still has the same aperture, even in an isolated photo.
            assert_eq!(state.entries[0].portal.id, portal().id);
            assert!(state.own().unwrap().allowed(&world));
            assert!(state.own().unwrap().waiting_for_clearance());
        }
        world.photo_isolation.picking = true;
        assert_eq!(state.visible(&world).count(), 1, "choosing shows the world");
        world.photo_isolation.picking = false;
    }
    world.photo_isolation.mode = Mode::Off;
    assert_eq!(state.visible(&world).count(), 1);
    world.photo_isolation.mode = Mode::Own;
    world.photo_isolation.open = false;
    assert_eq!(state.visible(&world).count(), 1, "closing photo mode restores the surface");
    world.photo_isolation.open = true;
    state.close(1000);
    world.photo_isolation.open = false;
    assert_eq!(state.visible(&world).count(), 0, "closing an invitation while hidden is final");
}

#[test]
fn portal_dropped_on_someone_requires_leaving_then_a_voluntary_plane_crossing() {
    let p = portal();
    let now = Instant::now();
    let mut gate = Gate::observer(Vec3::from(p.center), point(0.0), now);
    let later = now + Duration::from_secs(1);
    assert!(!gate.sample(&p.announcement(), point(0.0), true, later));
    for x in [0.5, -0.5, 1.0, 2.0, 3.0] {
        assert!(!gate.sample(&p.announcement(), point(x), true, later));
    }
    assert!(!gate.armed);
    assert!(!gate.sample(&p.announcement(), point(3.5), true, later));
    assert!(gate.armed);
    assert!(!gate.sample(&p.announcement(), point(1.0), true, later));
    assert!(gate.sample(&p.announcement(), point(-0.5), true, later));
    assert!(!gate.sample(&p.announcement(), point(0.5), true, later));
    assert!(!gate.sample(&p.announcement(), point(-0.5), true, later));
}

#[test]
fn portal_observer_uses_the_wide_clearance_only_before_its_first_approach() {
    let p = portal().announcement();
    let now = Instant::now();
    for nearby in [false, true] {
        let mut gate = Gate::observer(Vec3::from(p.center), point(if nearby { 0.0 } else { 4.0 }), now);
        let later = now + Duration::from_millis(700);
        if nearby {
            for x in [0.5, 2.0, 3.0] {
                assert!(!gate.sample(&p, point(x), true, later));
                assert!(!gate.armed, "initial anti-drop clearance remains mandatory");
            }
            assert!(!gate.sample(&p, point(3.5), true, later));
        }
        assert!(gate.armed);
        assert!(!gate.sample(&p, point(1.0), true, later));
        assert!(!gate.sample(&p, point(-0.5), false, later));
        assert!(!gate.armed, "a refused crossing is consumed");
        assert!(!gate.sample(&p, point(-0.5), true, later));
        assert!(!gate.armed, "standing inside does not cause a delayed teleport");
        assert!(!gate.sample(&p, point(-1.2), true, later));
        assert!(gate.armed, "a deliberate retry needs only the ordinary clearance");
        assert!(gate.sample(&p, point(0.5), true, later));
    }
}

#[test]
fn portal_teleport_start_preserves_invitations_but_arrival_cannot_be_a_crossing() {
    let (mut world, mut state) = fixture();
    let owner = Uuid::from_u128(2);
    let now = Instant::now();
    let p = portal();
    state.receive(&world, &wire(owner, Message::Open(p.announcement())), now, 1000);
    world.agent.position = point(1.0);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(700)).is_none());
    world.teleporting = true;
    state.sync(&world, None, 1000);
    assert_eq!(state.entries.len(), 1, "the source bridge can still serve grants");
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(800)).is_none());
    world.teleporting = false;
    world.agent.position = point(-0.5);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(900)).is_none());
    world.agent.position = point(-1.2);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(1000)).is_none());
    world.agent.position = point(0.5);
    assert_eq!(state.update(&world, Vec3::X, now + Duration::from_millis(1100)), Some((owner, p)));
}

#[test]
fn portal_pushes_corrections_standing_inside_and_walking_around_do_not_teleport() {
    let p = portal();
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p.announcement(), point(1.0), false, later));
    assert!(!gate.sample(&p.announcement(), point(-0.5), false, later));
    assert!(!gate.sample(&p.announcement(), point(-0.5), true, later));
    assert!(!gate.armed);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p.announcement(), point(-0.5), true, later));
    assert!(!gate.armed);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    for x in [2.0, 0.5, -0.5] {
        assert!(!gate.sample(&p.announcement(), point(x) + Vec3::Y * 1.2, true, later));
    }
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p.announcement(), point(1.0), true, now));
    assert!(gate.sample(&p.announcement(), point(-0.5), true, now));
}

#[test]
fn portal_crossing_is_two_sided_bounded_by_the_aperture_and_rotates_with_its_plane() {
    let p = portal();
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut gate = Gate::new(Vec3::from(p.center), point(-4.0), now);
    for x in [-2.0, -0.5] {
        assert!(!gate.sample(&p.announcement(), point(x), true, later));
    }
    assert!(gate.sample(&p.announcement(), point(0.5), true, later));
    let p = Portal {
        yaw: std::f32::consts::FRAC_PI_2,
        ..p
    };
    let center = Vec3::from(p.center);
    let mut gate = Gate::new(center, center + Vec3::Y * 4.0, now);
    assert!(!gate.sample(&p.announcement(), center + Vec3::Y, true, later));
    assert!(gate.sample(&p.announcement(), center - Vec3::Y * 0.5, true, later));
    let mut gate = Gate::new(center, center + Vec3::Y * 4.0 + Vec3::Z * 2.0, now);
    assert!(!gate.sample(&p.announcement(), center + Vec3::Y + Vec3::Z * 2.0, true, later));
    assert!(!gate.sample(&p.announcement(), center - Vec3::Y * 0.5 + Vec3::Z * 2.0, true, later));
}

#[test]
fn portal_replays_never_move_the_portal_or_rearm_its_proximity_gate_and_close_is_owner_scoped() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    let owner = Uuid::from_u128(2);
    world.agent.position = point(0.0);
    let p = portal();
    state.receive(&world, &wire(owner, Message::Open(p.announcement())), now, 1000);
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].waiting_for_clearance());
    let mut changed = p.clone();
    changed.center[0] += 10.0;
    state.receive(&world, &wire(owner, Message::Open(changed.announcement())), now, 1000);
    assert_eq!(state.entries[0].portal, p.announcement());
    assert!(state.entries[0].waiting_for_clearance());
    state.receive(
        &world,
        &wire(Uuid::from_u128(3), Message::Close { region: 256, id: p.id }),
        now,
        1000,
    );
    assert_eq!(state.entries.len(), 1);
    state.receive(&world, &wire(owner, Message::Close { region: 256, id: p.id }), now, 1000);
    state.receive(&world, &wire(owner, Message::Open(p.announcement())), now, 1000);
    assert!(state.entries.is_empty());
}

#[test]
fn portal_permissions_are_checked_at_crossing_and_expiry_region_and_session_clear_state() {
    let (mut world, mut state) = fixture();
    let owner = Uuid::from_u128(2);
    let group = Uuid::from_u128(3);
    assert!(allowed(&world, owner, Access::Everyone));
    assert!(!allowed(&world, owner, Access::Friends));
    world.social.form_friendship(owner);
    assert!(allowed(&world, owner, Access::Friends));
    assert!(!allowed(&world, owner, Access::Group(group)));
    world.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Demo group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    assert!(allowed(&world, owner, Access::Group(group)));
    let now = Instant::now();
    state.receive(
        &world,
        &wire(
            owner,
            Message::Open(
                Portal {
                    access: Access::Friends,
                    ..portal()
                }
                .announcement(),
            ),
        ),
        now,
        1000,
    );
    world.social.end_friendship(&owner);
    world.agent.position = point(1.0);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_secs(1)).is_none());
    world.agent.position = point(-0.5);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_secs(1)).is_none());
    state.sync(&world, None, 1120);
    assert!(state.entries.is_empty());
    assert!(state.create(Portal { expires: 1240, ..portal() }, world.agent.position, now, 1120));
    world.main_region = Some(512);
    state.sync(&world, None, 1120);
    assert!(state.entries.is_empty());
    assert!(state.out.is_empty());
}

#[test]
fn portal_discovery_is_targeted_once_per_peer_and_outgoing_traffic_is_bounded() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    assert!(state.create(portal(), world.agent.position, now, 1000));
    let owner = Uuid::from_u128(2);
    let discover = wire(owner, Message::Discover { region: 256 });
    state.receive(&world, &discover, now, 1000);
    let count = state.out.len();
    state.receive(&world, &discover, now, 1000);
    assert_eq!(state.out.len(), count);
    assert!(
        state
            .out
            .iter()
            .any(|(target, message)| *target == owner && matches!(message, Message::Open(_)))
    );
    let endpoint = Endpoint {
        url: String::new(),
        token: Uuid::from_u128(4),
    };
    assert!(state.command(Some(&endpoint), now).is_some());
    assert!(state.command(Some(&endpoint), now).is_none());
    assert!(state.command(Some(&endpoint), now + Duration::from_millis(250)).is_some());
    state.sync(&world, Some(Uuid::from_u128(5)), 1000);
    assert_eq!(state.entries.len(), 1, "HTTP-in renewal does not close a portal");
    assert!(!state.discovered);
}

#[test]
fn portal_late_sharing_failure_never_removes_or_retries_a_replaced_portal() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let endpoint = Endpoint {
        url: String::new(),
        token: Uuid::from_u128(4),
    };
    assert!(state.create(portal(), world.agent.position, now, 1000));
    let Some(NetCommand::BridgeQuery { request, query, .. }) = state.command(Some(&endpoint), now) else {
        panic!("portal command");
    };
    assert!(matches!(
        query,
        Query::Portal {
            message: Message::Open(_),
            ..
        }
    ));
    assert!(state.owns(request));
    let replacement = Portal {
        id: Uuid::from_u128(8),
        ..portal()
    };
    assert!(state.create(replacement.clone(), world.agent.position, now, 1000));
    state.reply(request, &Err("unavailable".into()), now);
    assert_eq!(state.own().unwrap().portal.id, replacement.id);
    assert!(!state.failed && state.retries.is_empty());
    let Some(NetCommand::BridgeQuery { request, .. }) = state.command(Some(&endpoint), now + Duration::from_secs(1)) else {
        panic!("discovery command");
    };
    state.reply(request, &Ok(Reply::Portal), now + Duration::from_secs(1));
    assert!(!state.owns(request));
}

#[test]
fn portal_sharing_retries_the_same_redacted_action_and_reports_only_exhausted_failures() {
    for recovers in [true, false] {
        let (world, mut state) = fixture();
        let now = Instant::now();
        let endpoint = Endpoint {
            url: String::new(),
            token: Uuid::from_u128(4),
        };
        let mut invitation = portal();
        invitation.access = Access::Friends;
        assert!(state.create(invitation.clone(), world.agent.position, now, 1000));
        state.discovered = true;
        let mut at = now;
        for attempt in 1..=3 {
            let Some(NetCommand::BridgeQuery {
                request,
                query: Query::Portal { target, message },
                ..
            }) = state.command(Some(&endpoint), at)
            else {
                panic!("announcement attempt {attempt}");
            };
            assert!(target.is_nil());
            assert_eq!(message, Message::Open(invitation.announcement()));
            assert!(matches!(&message, Message::Open(portal) if portal.destination.is_none()));
            let result = if recovers && attempt == 3 {
                Ok(Reply::Portal)
            } else {
                Err("timeout".into())
            };
            state.reply(request, &result, at);
            if attempt < 3 {
                assert!(state.own().is_some() && !state.failed);
                assert!(state.command(Some(&endpoint), at + Duration::from_millis(250)).is_none());
            }
            at += Duration::from_secs(attempt);
        }
        assert_eq!(state.own().is_some(), recovers);
        assert_eq!(state.failed, !recovers);
        assert!(state.retries.is_empty() && state.pending.is_empty());
        assert!(state.command(Some(&endpoint), at + Duration::from_secs(10)).is_none());
    }
}

#[test]
fn portal_query_renewal_preserves_observer_gate_and_transport_retries_end_on_close_or_region_change() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    let endpoint = Endpoint {
        url: String::new(),
        token: Uuid::from_u128(4),
    };
    let owner = Uuid::from_u128(2);
    world.agent.position = Vec3::from(portal().center);
    state.receive(&world, &wire(owner, Message::Open(portal().announcement())), now, 1000);
    state.sync(&world, Some(endpoint.token), 1000);
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].waiting_for_clearance());
    state.sync(&world, None, 1000);
    state.sync(&world, Some(Uuid::from_u128(5)), 1000);
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries[0].waiting_for_clearance());
    state.discovered = true;
    assert!(state.create(portal(), world.agent.position, now, 1000));
    let Some(NetCommand::BridgeQuery { request, .. }) = state.command(Some(&endpoint), now) else {
        panic!("open");
    };
    state.reply(request, &Err("timeout".into()), now);
    assert_eq!(state.retries.len(), 1);
    state.close(1000);
    assert!(state.retries.is_empty());
    let Some(NetCommand::BridgeQuery {
        request,
        query: Query::Portal {
            message: Message::Close { .. },
            ..
        },
        ..
    }) = state.command(Some(&endpoint), now + SEND_INTERVAL)
    else {
        panic!("close");
    };
    state.reply(request, &Err("timeout".into()), now + SEND_INTERVAL);
    assert_eq!(state.retries.len(), 1);
    world.main_region = Some(512);
    state.sync(&world, Some(endpoint.token), 1000);
    assert!(state.entries.is_empty() && state.retries.is_empty() && state.pending.is_empty());
}

#[test]
fn portal_messages_from_wrong_regions_expired_and_close_before_open_are_rejected() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let owner = Uuid::from_u128(2);
    for p in [Portal { region: 512, ..portal() }, Portal { expires: 1000, ..portal() }] {
        state.receive(&world, &wire(owner, Message::Open(p.announcement())), now, 1000);
    }
    state.receive(
        &world,
        &wire(
            owner,
            Message::Close {
                region: 256,
                id: portal().id,
            },
        ),
        now,
        1000,
    );
    state.receive(&world, &wire(owner, Message::Open(portal().announcement())), now, 1000);
    assert!(state.entries.is_empty());
    assert!(state.out.is_empty());
}
