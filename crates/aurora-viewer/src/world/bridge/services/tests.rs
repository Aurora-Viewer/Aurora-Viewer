use super::*;
use aurora_net::{NetEvent, bridge::query::ScriptInfo};
use glam::Vec3;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn endpoint() -> Endpoint {
    Endpoint {
        url: "https://synthetic.agni.lindenlab.com/cap/11111111-1111-4111-8111-111111111111".into(),
        token: id(100),
    }
}
fn sighting() -> Sighting {
    Sighting {
        id: id(1),
        region: 1,
        position: Vec3::new(20.0, 0.0, 1020.0),
        altitude_known: false,
        loaded: false,
        sitting: false,
        typing: false,
    }
}
fn command_request(command: Option<NetCommand>) -> Uuid {
    match command.expect("query") {
        NetCommand::BridgeQuery { request, .. } => request,
        _ => panic!("read-only query"),
    }
}
fn setup() -> (Services, Endpoint, Instant) {
    let mut services = Services::default();
    let endpoint = endpoint();
    let _ = services.sync(id(10), Some(1), Some(&endpoint));
    (services, endpoint, Instant::now())
}
#[test]
fn radar_height_updates_unknown_distance_but_never_overwrites_precise_or_other_region_positions() {
    let (mut s, endpoint, now) = setup();
    let request = command_request(s.radar(&mut [sighting()], true, Some(&endpoint), now));
    let _ = s.reply(request, Ok(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2048.0 }])), now);
    let mut seen = [sighting()];
    assert!(s.radar(&mut seen, true, Some(&endpoint), now).is_none());
    assert!(seen[0].altitude_known && seen[0].position.z == 2048.0);
    seen[0] = Sighting { region: 2, ..sighting() };
    let _ = s.radar(&mut seen, true, Some(&endpoint), now);
    assert!(!seen[0].altitude_known);
    seen[0] = Sighting {
        loaded: true,
        altitude_known: true,
        position: Vec3::Z * 3000.0,
        ..sighting()
    };
    let _ = s.radar(&mut seen, true, Some(&endpoint), now);
    assert_eq!(seen[0].position.z, 3000.0);
    assert!(s.heights.is_empty());
}
#[test]
fn radar_batches_throttles_refreshes_and_expires_without_fabricating_an_altitude() {
    let (mut s, endpoint, now) = setup();
    let mut crowd: Vec<_> = (1..=80).map(|n| Sighting { id: id(n), ..sighting() }).collect();
    let request = command_request(s.radar(&mut crowd, true, Some(&endpoint), now));
    assert_eq!(s.radar.as_ref().expect("pending").avatars.len(), MAX_AVATARS);
    assert!(s.radar(&mut crowd, true, Some(&endpoint), now).is_none());
    let _ = s.reply(request, Ok(Reply::Altitudes(vec![])), now);
    assert!(s.radar(&mut crowd, true, Some(&endpoint), now).is_none());
    assert!(s.radar(&mut crowd, true, Some(&endpoint), now + Duration::from_secs(1)).is_some());
    assert_eq!(s.attempts.len(), 2 * MAX_AVATARS);
    let (mut s, endpoint, now) = setup();
    let request = command_request(s.radar(&mut [sighting()], true, Some(&endpoint), now));
    let _ = s.reply(request, Ok(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2048.0 }])), now);
    let mut seen = [sighting()];
    assert!(s.radar(&mut seen, true, Some(&endpoint), now + REFRESH).is_some());
    assert_eq!(seen[0].position.z, 2048.0);
    seen[0] = sighting();
    let _ = s.radar(&mut seen, true, Some(&endpoint), now + CACHE_AGE);
    assert!(!seen[0].altitude_known);
}

#[test]
fn large_crowds_service_unqueried_avatars_before_refreshing_earlier_batches() {
    let (mut s, endpoint, now) = setup();
    let mut crowd: Vec<_> = (1..=320).map(|n| Sighting { id: id(n), ..sighting() }).collect();
    let mut queried = HashSet::new();
    for second in 0..(crowd.len() / MAX_AVATARS) as u64 {
        let at = now + Duration::from_secs(second);
        let request = command_request(s.radar(&mut crowd, true, Some(&endpoint), at));
        queried.extend(s.radar.as_ref().expect("pending batch").avatars.keys().copied());
        let _ = s.reply(request, Ok(Reply::Altitudes(vec![])), at);
    }
    assert_eq!(
        queried.len(),
        crowd.len(),
        "unresolved neighbours must not starve avatars later in the list"
    );
}

#[test]
fn unavailable_or_disabled_queries_stay_quiet_and_stale_region_session_replies_are_ignored() {
    let (mut s, endpoint, now) = setup();
    let request = command_request(s.radar(&mut [sighting()], true, Some(&endpoint), now));
    assert!(s.owns(request));
    let _ = s.sync(id(10), Some(2), Some(&endpoint));
    assert!(!s.owns(request));
    assert!(
        s.reply(request, Ok(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2048.0 }])), now)
            .is_none()
    );
    assert!(s.heights.is_empty());
    assert!(s.radar(&mut [sighting()], false, Some(&endpoint), now).is_none());
    assert!(s.radar(&mut [sighting()], true, None, now).is_none());
    let request = command_request(s.radar(&mut [sighting()], true, Some(&endpoint), now));
    assert!(s.reply(request, Err("synthetic failure".into()), now).is_none());
    assert!(s.heights.is_empty());
}
#[test]
fn inspection_waits_for_initial_provisioning_and_reports_allocated_memory_and_milliseconds_locally() {
    let mut s = Services::default();
    let now = Instant::now();
    let _ = s.sync(id(10), Some(1), None);
    assert!(s.inspect(id(1), now));
    assert!(!s.inspect(id(1), now));
    assert!(s.update(None, now).0.is_none());
    let endpoint = endpoint();
    assert!(s.sync(id(10), Some(1), Some(&endpoint)).is_none());
    let request = command_request(s.update(Some(&endpoint), now).0);
    let message = s
        .reply(
            request,
            Ok(Reply::ScriptInfo(ScriptInfo {
                object: id(1),
                name: "Synthetic object".into(),
                running: 2,
                total: 3,
                memory: 196608,
                cpu_seconds: 0.00125,
                character_seconds: 0.0002,
            })),
            now,
        )
        .expect("local report");
    assert!(message.contains("Synthetic object") && message.contains("2/3"));
    assert!(message.contains(&aurora_i18n::number(192.0, 2)));
    assert!(message.contains(&aurora_i18n::number(1.25, 3)));
    assert!(message.contains(&aurora_i18n::number(0.2, 3)));
    assert!(!message.contains(&endpoint.url) && !message.contains(&endpoint.token.to_string()));
}
#[test]
fn newer_inspections_session_changes_and_timeouts_do_not_publish_old_object_results() {
    let (mut s, endpoint, now) = setup();
    s.inspect(id(1), now);
    let old = command_request(s.update(Some(&endpoint), now).0);
    assert!(s.owns(old));
    s.inspect(id(2), now);
    assert!(!s.owns(old));
    let current = command_request(s.update(Some(&endpoint), now).0);
    assert!(s.owns(current));
    assert!(s.reply(old, Err("old failure".into()), now).is_none());
    let rotated = Endpoint {
        token: id(101),
        ..endpoint.clone()
    };
    assert!(s.sync(id(10), Some(1), Some(&rotated)).is_some());
    assert!(!s.owns(current));
    assert!(s.reply(current, Err("stale failure".into()), now).is_none());
    s.inspect(id(1), now);
    command_request(s.update(Some(&rotated), now).0);
    assert!(s.update(Some(&rotated), now + QUERY_TIMEOUT).1.is_some());
    s.inspect(id(1), now);
    assert!(s.sync(id(11), Some(1), Some(&rotated)).is_some());
}

fn object_info(object: Uuid) -> Box<ObjectInfo> {
    Box::new(ObjectInfo {
        scripts: ScriptInfo {
            object,
            name: "Synthetic extended object".into(),
            running: 2,
            total: 3,
            memory: 196608,
            cpu_seconds: 0.00125,
            character_seconds: 0.0002,
        },
        description: "Description, with separators | []".into(),
        root: id(2),
        prims: 3,
        land_impact: 2,
        inventory: 5,
        velocity: [1.0, -2.0, 0.0],
        position: [128.0, 130.0, 26.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        rotation_degrees: [0.0; 3],
        angular_velocity: [0.0; 3],
        creator: id(3),
        owner: Uuid::nil(),
        previous_owner: Uuid::nil(),
        rezzer: id(4),
        group: id(5),
        created: "2026-10-17T10:00:00Z".into(),
        rezzed: "2026-10-17T11:00:00Z".into(),
        pathfinding: -1,
        attachment_point: 31,
        temporary_attachment: true,
        observer_position: [128.0, 126.0, 26.0],
    })
}

#[test]
fn object_inspection_replaces_a_basic_report_for_the_same_target_and_formats_all_extended_fields() {
    let (mut s, endpoint, now) = setup();
    assert!(s.inspect(id(1), now));
    let old = command_request(s.update(Some(&endpoint), now).0);
    assert!(s.inspect_object(id(1), now, &HashMap::from([(31, "Center 2".into())])));
    assert!(!s.inspect_object(id(1), now, &HashMap::from([(31, "Center 2".into())])));
    assert!(!s.owns(old));
    let command = s.update(Some(&endpoint), now).0;
    assert!(matches!(&command, Some(NetCommand::BridgeQuery { query: Query::ObjectInfo(object), .. }) if *object == id(1)));
    let request = command_request(command);
    assert!(s.reply(old, Err("obsolete basic report".into()), now).is_none());
    let message = s
        .reply(request, Ok(Reply::ObjectInfo(object_info(id(1)))), now)
        .expect("extended report");
    for expected in [
        "Synthetic extended object".to_owned(),
        "2/3".to_owned(),
        aurora_i18n::number(192.0, 2),
        aurora_i18n::number(1.25, 3),
        aurora_i18n::number(4.0, 3),
        "Description, with separators | []".to_owned(),
        id(1).to_string(),
        id(2).to_string(),
        format!("secondlife:///app/agent/{}/about", id(3)),
        id(4).to_string(),
        format!("secondlife:///app/group/{}/about", id(5)),
        "2026-10-17T10:00:00Z".to_owned(),
        "2026-10-17T11:00:00Z".to_owned(),
        aurora_i18n::tr!("bridge-object-info-group-owned").to_owned(),
        aurora_i18n::tr!("common-yes").to_owned(),
        crate::ui::appearance::items::point_name("Center 2").to_owned(),
    ] {
        assert!(message.contains(&expected), "missing {expected}");
    }
    assert!(message.lines().count() >= 23 && !message.contains("{ $"));
    assert!(!message.contains(&endpoint.url) && !message.contains(&endpoint.token.to_string()));
}

#[test]
fn object_inspection_checks_reply_kind_target_region_rotation_and_timeouts() {
    let (mut s, endpoint, now) = setup();
    for reply in [Reply::ObjectInfo(object_info(id(2))), Reply::Altitudes(vec![])] {
        assert!(s.inspect_object(id(1), now, &HashMap::from([(31, "Center 2".into())])));
        let request = command_request(s.update(Some(&endpoint), now).0);
        assert_eq!(
            s.reply(request, Ok(reply), now).as_deref(),
            Some(aurora_i18n::tr!("bridge-request-response"))
        );
    }
    s.inspect_object(id(1), now, &HashMap::from([(31, "Center 2".into())]));
    let request = command_request(s.update(Some(&endpoint), now).0);
    assert!(s.sync(id(10), Some(2), Some(&endpoint)).is_some());
    assert!(s.reply(request, Ok(Reply::ObjectInfo(object_info(id(1)))), now).is_none());
    s.inspect_object(id(1), now, &HashMap::from([(31, "Center 2".into())]));
    let request = command_request(s.update(Some(&endpoint), now).0);
    assert_eq!(
        s.reply(request, Ok(Reply::ObjectInfo(object_info(id(1)))), now + QUERY_TIMEOUT)
            .as_deref(),
        Some(aurora_i18n::tr!("bridge-query-timeout"))
    );
}

fn radar_world() -> (crate::world::World, RegionHandle, aurora_net::objects::ObjectUpdate) {
    let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        let _ = world.apply(event);
    }
    world.agent.position = Vec3::new(160.0, 160.0, 20.0);
    let region = world.main_region.expect("demo region");
    let mut avatar = crate::demo::action_avatar(false);
    avatar.full_id = id(1);
    avatar.local_id = 49001;
    avatar.position = Vec3::new(0.0, 0.0, 20.0);
    world.objects.upsert(region, avatar.clone());
    world.objects.refresh_labelled();
    (world, region, avatar)
}
fn high_coarse(world: &mut crate::world::World, region: RegionHandle) {
    let _ = world.apply(NetEvent::CoarseLocations {
        handle: region,
        positions: vec![(id(1), Vec3::new(160.0, 160.0, 1020.0))],
    });
}
fn avatar_sighting(world: &crate::world::World) -> Sighting {
    crate::world::radar::sightings(world)
        .into_iter()
        .find(|s| s.id == id(1))
        .expect("radar avatar")
}
#[test]
fn unresolved_seat_ancestors_and_cycles_fall_back_to_coarse_until_the_complete_chain_arrives() {
    let (mut world, region, mut avatar) = radar_world();
    avatar.parent_id = 49002;
    avatar.position = Vec3::new(1.0, 0.0, 1.0);
    world.objects.upsert(region, avatar);
    high_coarse(&mut world, region);
    assert!(!avatar_sighting(&world).altitude_known);
    let mut seat = crate::demo::action_avatar(false);
    seat.pcode = aurora_prim::params::LL_PCODE_VOLUME;
    seat.full_id = id(2);
    seat.local_id = 49002;
    seat.parent_id = 49003;
    seat.position = Vec3::new(2.0, 0.0, 1.0);
    seat.rotation = glam::Quat::IDENTITY;
    world.objects.upsert(region, seat.clone());
    assert!(
        !avatar_sighting(&world).altitude_known,
        "an available direct seat does not make its missing ancestor known"
    );
    let mut root = seat.clone();
    root.full_id = id(3);
    root.local_id = 49003;
    root.parent_id = 0;
    root.position = Vec3::new(150.0, 150.0, 2498.0);
    root.rotation = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    world.objects.upsert(region, root.clone());
    let complete = avatar_sighting(&world);
    assert!(complete.loaded && complete.altitude_known);
    assert!(complete.position.abs_diff_eq(Vec3::new(150.0, 153.0, 2500.0), 0.001));
    root.parent_id = seat.local_id;
    world.objects.upsert(region, root);
    assert!(
        !avatar_sighting(&world).altitude_known,
        "a cycle must neither hang the radar nor invent a region position"
    );
}
#[test]
fn nil_targets_do_not_prevent_lookup_of_other_missing_sightings() {
    let (mut s, endpoint, now) = setup();
    let mut seen = [
        Sighting {
            id: Uuid::nil(),
            ..sighting()
        },
        Sighting {
            altitude_known: false,
            ..sighting()
        },
    ];
    let command = s.radar(&mut seen, true, Some(&endpoint), now).expect("valid target lookup");
    let NetCommand::BridgeQuery { query, .. } = command else {
        panic!("query")
    };
    assert_eq!(query, Query::Altitudes(vec![id(1)]));
    assert!(query.valid());
}

#[test]
fn known_native_and_coarse_altitudes_do_not_trigger_bridge_queries() {
    for (loaded, z) in [(true, 250.0), (true, 2500.0), (false, 248.0)] {
        let (mut service, endpoint, now) = setup();
        let raw = Sighting {
            loaded,
            altitude_known: true,
            position: Vec3::new(48.0, 0.0, z),
            ..sighting()
        };
        for seconds in [0, 7, 14, 60] {
            let mut seen = [raw.clone()];
            assert!(
                service
                    .radar(&mut seen, true, Some(&endpoint), now + Duration::from_secs(seconds))
                    .is_none()
            );
            assert_eq!(seen[0].position, raw.position);
        }
        assert!(service.attempts.is_empty() && service.heights.is_empty());
    }
}

#[test]
fn cached_and_late_bridge_heights_cannot_override_a_newly_known_height() {
    for reply_before_native in [true, false] {
        let (mut service, endpoint, now) = setup();
        let request = command_request(service.radar(&mut [sighting()], true, Some(&endpoint), now));
        let mut native = [Sighting {
            loaded: true,
            altitude_known: true,
            position: Vec3::new(48.0, 0.0, 250.0),
            ..sighting()
        }];
        let reply = Ok(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2500.0 }]));
        if reply_before_native {
            let _ = service.reply(request, reply, now);
        } else {
            let _ = service.radar(&mut native, true, Some(&endpoint), now);
            let _ = service.reply(request, reply, now);
        }
        assert!(service.radar(&mut native, true, Some(&endpoint), now).is_none());
        assert_eq!(native[0].position.z, 250.0);
        assert!(service.heights.is_empty());
        let mut radar = crate::world::radar::Radar::default();
        radar.sweep(native.into(), Vec3::Z * 20.0, Some(1), &HashMap::new(), 256.0, 0.0);
        assert!(
            radar
                .get(&id(1))
                .and_then(|e| e.range)
                .is_some_and(|range| (range - 235.0).abs() < 0.1)
        );
    }
}

#[test]
fn missing_coarse_height_is_completed_through_the_bridge_and_distance_is_three_dimensional() {
    let (mut world, region, avatar) = radar_world();
    world.objects.remove(&crate::world::objects::ObjKey {
        region,
        local_id: avatar.local_id,
    });
    high_coarse(&mut world, region);
    let raw = avatar_sighting(&world);
    assert!(!raw.loaded && !raw.altitude_known);
    let (mut service, endpoint, now) = setup();
    let _ = service.sync(world.agent_id, Some(region), Some(&endpoint));
    let request = command_request(service.radar(&mut [raw], true, Some(&endpoint), now));
    let _ = service.reply(request, Ok(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2500.0 }])), now);
    let mut corrected = vec![avatar_sighting(&world)];
    let _ = service.radar(&mut corrected, true, Some(&endpoint), now);
    let mut radar = crate::world::radar::Radar::default();
    radar.sweep(corrected, world.agent.position, Some(region), &HashMap::new(), 256.0, 0.0);
    assert_eq!(radar.get(&id(1)).and_then(|e| e.range), Some(2480.0));
    let mut disabled = [avatar_sighting(&world)];
    let _ = service.radar(&mut disabled, false, Some(&endpoint), now);
    assert!(!disabled[0].altitude_known);
}
