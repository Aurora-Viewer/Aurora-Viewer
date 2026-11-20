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
        expires: 1120,
    }
}
pub(super) fn point(x: f32) -> Vec3 {
    Vec3::new(10.0 + x, 10.0, 25.0)
}
pub(super) fn fixture() -> (World, State) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(1);
    world.main_region = Some(256);
    world.agent.position = point(4.0);
    let mut state = State::default();
    state.sync(&world, 1000);
    (world, state)
}

#[test]
fn portal_placement_stays_inside_standard_and_variable_size_regions() {
    assert!(fits_region(Vec3::new(128.0, 128.0, 25.0), [256, 256]));
    assert!(!fits_region(Vec3::new(258.0, 128.0, 25.0), [256, 256]));
    assert!(!fits_region(Vec3::new(128.0, 0.5, 25.0), [256, 256]));
    assert!(fits_region(Vec3::new(500.0, 200.0, 25.0), [512, 256]));
    assert!(!fits_region(Vec3::new(128.0, 300.0, 25.0), [512, 256]));
}

#[test]
fn a_portal_opens_only_where_we_may_build_and_a_refusal_keeps_the_open_one() {
    use aurora_net::parcel_flags as pf;
    let (mut world, mut state) = fixture();
    let center = Vec3::from(portal().center);
    let open = |world: &World, state: &mut State, id: u128| {
        let right = world.build_right_at(256, center, None);
        state.create(
            Portal {
                id: Uuid::from_u128(id),
                ..portal()
            },
            right,
            world.agent.position,
            Instant::now(),
            1000,
        )
    };
    // the parcel there is not known yet
    assert_eq!(open(&world, &mut state, 1), Err(Refusal::LandUnknown));
    // every cell of the region, someone else's, no build
    let mut parcel = aurora_net::ParcelInfo {
        owner_id: Uuid::from_u128(2),
        group_id: Uuid::from_u128(50),
        bitmap: vec![0xFF; 512],
        ..Default::default()
    };
    world.parcel = Some(Arc::new(parcel.clone()));
    assert_eq!(open(&world, &mut state, 1), Err(Refusal::NoBuild));
    assert!(state.own().is_none());
    // building for everyone
    parcel.flags = pf::CREATE_OBJECTS;
    world.parcel = Some(Arc::new(parcel.clone()));
    assert_eq!(open(&world, &mut state, 2), Ok(()));
    // our own land
    parcel.flags = 0;
    parcel.owner_id = world.agent_id;
    world.parcel = Some(Arc::new(parcel.clone()));
    assert_eq!(open(&world, &mut state, 3), Ok(()));
    // group building, as a member
    parcel.owner_id = Uuid::from_u128(2);
    parcel.flags = pf::CREATE_GROUP_OBJECTS;
    world.parcel = Some(Arc::new(parcel.clone()));
    assert_eq!(open(&world, &mut state, 4), Err(Refusal::NoBuild), "not a member");
    world.groups.groups.push(aurora_net::GroupMembership {
        id: Uuid::from_u128(50),
        name: String::new(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    assert_eq!(open(&world, &mut state, 5), Ok(()));
    // the land closes: a new place is refused and the open portal stays
    parcel.flags = 0;
    world.parcel = Some(Arc::new(parcel));
    assert_eq!(open(&world, &mut state, 6), Err(Refusal::NoBuild));
    assert_eq!(state.own().map(|e| e.portal.id), Some(Uuid::from_u128(5)));
    assert_eq!(state.entries.len(), 1, "each new portal replaced the one before");
}

#[test]
fn a_portal_is_refused_outside_the_region_expired_or_to_no_place() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let refused = |state: &mut State, portal: Portal| {
        state.create(portal, BuildRight::Allowed, world.agent.position, now, 1000) == Err(Refusal::Invalid)
    };
    assert!(refused(&mut state, Portal { region: 512, ..portal() }));
    assert!(refused(&mut state, Portal { expires: 1000, ..portal() }));
    assert!(
        refused(&mut state, Portal { expires: 1200, ..portal() }),
        "longer than its lifetime"
    );
    for destination in ["", "  ", "grid.example/Region", "A|B", "Line\nbreak"] {
        assert!(refused(
            &mut state,
            Portal {
                destination: destination.into(),
                ..portal()
            }
        ));
    }
    assert!(refused(
        &mut state,
        Portal {
            destination: "x".repeat(129),
            ..portal()
        }
    ));
    assert!(refused(
        &mut state,
        Portal {
            center: [f32::NAN, 10.0, 25.0],
            ..portal()
        }
    ));
    assert!(state.entries.is_empty());
}

#[test]
fn portal_can_be_entered_immediately_from_nearby_and_after_a_distant_stall() {
    let p = portal();
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
fn portal_opened_on_us_requires_leaving_then_a_voluntary_plane_crossing() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    world.agent.position = point(0.2);
    assert!(state.create(portal(), BuildRight::Allowed, world.agent.position, now, 1000).is_ok());
    assert!(state.own().unwrap().waiting_for_clearance());
    // walking out through the plane from inside is not a crossing
    for (i, x) in [-0.3, -0.6, -0.9].into_iter().enumerate() {
        world.agent.position = point(x);
        assert!(
            state
                .update(&world, Vec3::NEG_X, now + Duration::from_millis(100 * (i as u64 + 1)))
                .is_none()
        );
    }
    world.agent.position = point(-1.2);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(400)).is_none());
    assert!(!state.own().unwrap().waiting_for_clearance());
    world.agent.position = point(-0.4);
    assert!(state.update(&world, Vec3::X, now + Duration::from_millis(500)).is_none());
    world.agent.position = point(0.4);
    assert_eq!(state.update(&world, Vec3::X, now + Duration::from_millis(600)), Some(portal()));
    // the crossing is consumed: no second teleport without leaving again
    world.agent.position = point(-0.4);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(700)).is_none());
}

#[test]
fn photo_isolation_hides_portal_surfaces_without_retiring_portals_or_clearance() {
    use crate::snapshot::isolate::Mode;
    let (mut world, mut state) = fixture();
    world.agent.position = point(0.0);
    assert!(
        state
            .create(portal(), BuildRight::Allowed, world.agent.position, Instant::now(), 1000)
            .is_ok()
    );
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
    state.close();
    world.photo_isolation.open = false;
    assert_eq!(state.visible(&world).count(), 0, "closing a portal while hidden is final");
}

#[test]
fn a_teleport_disarms_every_portal_and_its_arrival_is_no_crossing() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    let p = portal();
    assert!(
        state
            .create(p.clone(), BuildRight::Allowed, world.agent.position, now, 1000)
            .is_ok()
    );
    world.agent.position = point(1.0);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(100)).is_none());
    world.teleporting = true;
    state.sync(&world, 1000);
    assert_eq!(state.entries.len(), 1, "kept until the region changes");
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(200)).is_none());
    world.teleporting = false;
    world.agent.position = point(-0.5);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(300)).is_none());
    world.agent.position = point(-1.2);
    assert!(state.update(&world, Vec3::NEG_X, now + Duration::from_millis(400)).is_none());
    world.agent.position = point(0.5);
    assert_eq!(state.update(&world, Vec3::X, now + Duration::from_millis(500)), Some(p));
}

#[test]
fn portal_pushes_corrections_standing_inside_and_walking_around_do_not_teleport() {
    let p = portal();
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p, point(1.0), false, later));
    assert!(!gate.sample(&p, point(-0.5), false, later));
    assert!(!gate.sample(&p, point(-0.5), true, later));
    assert!(!gate.armed);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p, point(-0.5), true, later));
    assert!(!gate.armed);
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    for x in [2.0, 0.5, -0.5] {
        assert!(!gate.sample(&p, point(x) + Vec3::Y * 1.2, true, later));
    }
    let mut gate = Gate::new(Vec3::from(p.center), point(4.0), now);
    assert!(!gate.sample(&p, point(1.0), true, now));
    assert!(gate.sample(&p, point(-0.5), true, now));
}

#[test]
fn portal_crossing_is_two_sided_bounded_by_the_aperture_and_rotates_with_its_plane() {
    let p = portal();
    let now = Instant::now();
    let later = now + Duration::from_secs(1);
    let mut gate = Gate::new(Vec3::from(p.center), point(-4.0), now);
    for x in [-2.0, -0.5] {
        assert!(!gate.sample(&p, point(x), true, later));
    }
    assert!(gate.sample(&p, point(0.5), true, later));
    let p = Portal {
        yaw: std::f32::consts::FRAC_PI_2,
        ..p
    };
    let center = Vec3::from(p.center);
    let mut gate = Gate::new(center, center + Vec3::Y * 4.0, now);
    assert!(!gate.sample(&p, center + Vec3::Y, true, later));
    assert!(gate.sample(&p, center - Vec3::Y * 0.5, true, later));
    let mut gate = Gate::new(center, center + Vec3::Y * 4.0 + Vec3::Z * 2.0, now);
    assert!(!gate.sample(&p, center + Vec3::Y + Vec3::Z * 2.0, true, later));
    assert!(!gate.sample(&p, center - Vec3::Y * 0.5 + Vec3::Z * 2.0, true, later));
}

#[test]
fn returns_stay_ours_beside_our_portal_and_an_arrival_never_moves_or_rearms_one() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    world.agent.position = point(0.0);
    assert!(state.add_return(portal(), world.agent.position, now, 1000));
    assert!(state.own().is_none(), "a return is not the portal opened here");
    assert!(state.entries[0].waiting_for_clearance());
    // the same return again, from elsewhere: kept as it was
    let moved = Portal {
        center: [40.0, 40.0, 25.0],
        ..portal()
    };
    assert!(state.add_return(moved, point(30.0), now, 1000));
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].portal, portal());
    assert!(state.entries[0].waiting_for_clearance());
    // opening and closing our portal leaves the return
    let own = Portal {
        id: Uuid::from_u128(8),
        ..portal()
    };
    assert!(state.create(own, BuildRight::Allowed, world.agent.position, now, 1000).is_ok());
    assert_eq!(state.entries.len(), 2);
    state.close();
    assert_eq!(state.entries.len(), 1);
    assert!(state.own().is_none());
    // a return in another region, or expired, is not kept
    assert!(!state.add_return(
        Portal {
            id: Uuid::from_u128(9),
            region: 512,
            ..portal()
        },
        world.agent.position,
        now,
        1000
    ));
    assert!(!state.add_return(
        Portal {
            id: Uuid::from_u128(9),
            ..portal()
        },
        world.agent.position,
        now,
        1120
    ));
}

#[test]
fn expiry_region_and_session_changes_clear_every_portal() {
    let (mut world, mut state) = fixture();
    let now = Instant::now();
    assert!(state.create(portal(), BuildRight::Allowed, world.agent.position, now, 1000).is_ok());
    state.sync(&world, 1119);
    assert_eq!(state.entries.len(), 1);
    state.sync(&world, 1120);
    assert!(state.entries.is_empty(), "expired");
    let fresh = || Portal { expires: 1240, ..portal() };
    assert!(state.create(fresh(), BuildRight::Allowed, world.agent.position, now, 1120).is_ok());
    world.main_region = Some(512);
    state.sync(&world, 1120);
    assert!(state.entries.is_empty(), "another region");
    world.main_region = Some(256);
    state.sync(&world, 1120);
    assert!(state.create(fresh(), BuildRight::Allowed, world.agent.position, now, 1120).is_ok());
    world.agent_id = Uuid::from_u128(2);
    state.sync(&world, 1120);
    assert!(state.entries.is_empty(), "another session");
}
