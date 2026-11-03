use super::{
    tests::{fixture, point, portal, wire},
    *,
};

fn receive(state: &mut State, world: &World, peer: Uuid, message: Message, now: Instant) {
    state.receive(world, &wire(peer, message), now, 1000);
}

fn request(state: &mut State, world: &World, now: Instant) -> Message {
    state.update(world, Vec3::ZERO, now);
    state
        .out
        .iter()
        .find_map(|(_, message)| matches!(message, Message::Request { .. }).then(|| message.clone()))
        .unwrap()
}

#[test]
fn private_destinations_are_never_announced_and_only_authenticated_friends_receive_nonce_bound_details() {
    let (mut creator, mut server) = fixture();
    let peer = Uuid::from_u128(2);
    let stranger = Uuid::from_u128(3);
    creator.social.form_friendship(peer);
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    let now = Instant::now();
    assert!(server.create(p.clone(), creator.agent.position, now, 1000));
    receive(&mut server, &creator, stranger, Message::Discover { region: 256 }, now);
    assert!(server.out.iter().all(|(_, message)| match message {
        Message::Open(ad) => ad.destination.is_none(),
        _ => true,
    }));

    let (mut observer, mut client) = fixture();
    observer.agent_id = peer;
    observer.agent.position = point(0.0);
    observer.social.form_friendship(creator.agent_id);
    client.sync(&observer, None, 1000);
    receive(&mut client, &observer, creator.agent_id, Message::Open(p.announcement()), now);
    assert!(!client.entries[0].allowed(&observer));
    assert!(client.entries[0].waiting_for_clearance());
    let ask = request(&mut client, &observer, now);
    assert!(client.out.iter().all(|(target, _)| *target == creator.agent_id));
    receive(&mut server, &creator, stranger, ask.clone(), now);
    receive(&mut server, &creator, peer, ask.clone(), now);
    server.update(&creator, Vec3::ZERO, now);
    let grants: Vec<_> = server
        .out
        .iter()
        .filter(|(_, message)| matches!(message, Message::Details { .. }))
        .cloned()
        .collect();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].0, peer);
    let grant = grants[0].1.clone();
    receive(&mut client, &observer, stranger, grant.clone(), now);
    assert!(client.entries[0].details.is_none());
    let Message::Details { region, id, details, .. } = grant.clone() else {
        panic!("details")
    };
    receive(
        &mut client,
        &observer,
        creator.agent_id,
        Message::Details {
            region,
            id,
            nonce: Uuid::from_u128(999),
            details,
        },
        now,
    );
    assert!(client.entries[0].details.is_none());
    receive(&mut client, &observer, creator.agent_id, grant, now);
    assert!(client.entries[0].allowed(&observer));
    assert_eq!(client.entries[0].resolved(), Some(p));
    assert!(
        client.entries[0].waiting_for_clearance(),
        "details must never reset the anti-drop gate"
    );
    assert!(client.update(&observer, Vec3::ZERO, now + Duration::from_secs(1)).is_none());
    assert!(client.out.is_empty() && client.fetches.is_empty());

    // A grant queued while friendship existed must not escape after revocation.
    creator.social.end_friendship(&peer);
    server.update(&creator, Vec3::ZERO, now + Duration::from_millis(250));
    assert!(!server.out.iter().any(|(_, message)| matches!(message, Message::Details { .. })));
}

#[test]
fn private_portal_can_finish_granting_access_while_its_creator_starts_teleporting() {
    let (mut creator, mut server) = fixture();
    let (mut observer, mut client) = fixture();
    observer.agent_id = Uuid::from_u128(2);
    creator.social.form_friendship(observer.agent_id);
    observer.social.form_friendship(creator.agent_id);
    client.sync(&observer, None, 1000);
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    let now = Instant::now();
    assert!(server.create(p.clone(), creator.agent.position, now, 1000));
    receive(&mut client, &observer, creator.agent_id, Message::Open(p.announcement()), now);
    let ask = request(&mut client, &observer, now);

    // The creator has requested a teleport, but their bridge is still in
    // the source region and can finish the visitor's permission exchange.
    creator.teleporting = true;
    server.sync(&creator, None, 1000);
    receive(&mut server, &creator, observer.agent_id, ask, now);
    server.update(&creator, Vec3::ZERO, now);
    let grant = server
        .out
        .iter()
        .find_map(|(_, m)| matches!(m, Message::Details { .. }).then(|| m.clone()))
        .expect("starting a teleport must not discard the pending invitation");
    receive(&mut client, &observer, creator.agent_id, grant, now);
    assert!(client.entries[0].allowed(&observer));

    creator.main_region = Some(512);
    server.sync(&creator, None, 1000);
    assert!(
        server.entries.is_empty() && server.out.is_empty(),
        "actual departure retires source state"
    );
    observer.agent.position = point(1.0);
    assert!(client.update(&observer, Vec3::NEG_X, now + Duration::from_millis(700)).is_none());
    observer.agent.position = point(-0.5);
    assert_eq!(
        client.update(&observer, Vec3::NEG_X, now + Duration::from_millis(800)),
        Some((creator.agent_id, p))
    );
}

#[test]
fn private_portal_grant_after_a_refused_crossing_allows_a_nearby_retry() {
    let (mut creator, mut server) = fixture();
    let (mut observer, mut client) = fixture();
    observer.agent_id = Uuid::from_u128(2);
    creator.social.form_friendship(observer.agent_id);
    observer.social.form_friendship(creator.agent_id);
    client.sync(&observer, None, 1000);
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    let now = Instant::now();
    assert!(server.create(p.clone(), creator.agent.position, now, 1000));
    receive(&mut client, &observer, creator.agent_id, Message::Open(p.announcement()), now);
    let ask = request(&mut client, &observer, now);
    observer.agent.position = point(1.0);
    assert!(client.update(&observer, Vec3::NEG_X, now + Duration::from_millis(700)).is_none());
    observer.agent.position = point(-0.5);
    assert!(client.update(&observer, Vec3::NEG_X, now + Duration::from_millis(800)).is_none());
    assert!(client.entries[0].waiting_for_clearance());

    // The creator's permission response arrives after the visitor passed.
    receive(&mut server, &creator, observer.agent_id, ask, now);
    server.update(&creator, Vec3::ZERO, now);
    let grant = server
        .out
        .iter()
        .find_map(|(_, m)| matches!(m, Message::Details { .. }).then(|| m.clone()))
        .unwrap();
    receive(&mut client, &observer, creator.agent_id, grant, now + Duration::from_millis(900));
    assert!(client.entries[0].allowed(&observer));
    assert!(client.update(&observer, Vec3::ZERO, now + Duration::from_millis(900)).is_none());
    observer.agent.position = point(-1.2);
    assert!(client.update(&observer, Vec3::NEG_X, now + Duration::from_millis(1000)).is_none());
    observer.agent.position = point(0.5);
    assert_eq!(
        client.update(&observer, Vec3::X, now + Duration::from_millis(1100)),
        Some((creator.agent_id, p))
    );
}

#[test]
fn unanswered_private_requests_have_one_retry_and_closed_expired_or_replaced_portals_reject_late_details() {
    let (world, mut client) = fixture();
    let owner = Uuid::from_u128(2);
    let now = Instant::now();
    let p = Portal {
        access: Access::Friends,
        ..portal()
    };
    receive(&mut client, &world, owner, Message::Open(p.announcement()), now);
    let ask = request(&mut client, &world, now);
    request(&mut client, &world, now + Duration::from_secs(1));
    assert_eq!(client.out.len(), 1);
    request(&mut client, &world, now + Duration::from_secs(2));
    assert_eq!(client.out.len(), 2);
    request(&mut client, &world, now + Duration::from_secs(20));
    assert_eq!(client.out.len(), 2);
    let Message::Request { region, id, nonce } = ask else {
        panic!("request")
    };
    let grant = Message::Details {
        region,
        id,
        nonce,
        details: p.details(),
    };
    let now = now + Duration::from_secs(21);
    receive(&mut client, &world, owner, Message::Close { region, id }, now);
    assert!(client.out.is_empty() && client.fetches.is_empty());
    receive(&mut client, &world, owner, grant.clone(), now);
    assert!(client.entries.is_empty());
    let replacement = Portal {
        id: Uuid::from_u128(8),
        ..p
    };
    receive(&mut client, &world, owner, Message::Open(replacement.announcement()), now);
    request(&mut client, &world, now);
    receive(&mut client, &world, owner, grant, now);
    assert!(client.entries[0].details.is_none());
    client.sync(&world, None, 1120);
    assert!(client.entries.is_empty() && client.fetches.is_empty() && client.out.is_empty());
}

#[test]
fn group_grants_wait_for_one_fresh_complete_server_member_list_shared_by_all_requesters() {
    use aurora_net::groups::{GroupMember, LastSeen, MembersPage};
    let (mut world, mut server) = fixture();
    let group = Uuid::from_u128(20);
    let member = Uuid::from_u128(2);
    let stranger = Uuid::from_u128(3);
    world.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Demo group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    let p = Portal {
        access: Access::Group(group),
        ..portal()
    };
    let now = Instant::now();
    server.create(p.clone(), world.agent.position, now, 1000);
    for peer in [member, stranger] {
        receive(
            &mut server,
            &world,
            peer,
            Message::Request {
                region: 256,
                id: p.id,
                nonce: peer,
            },
            now,
        );
    }
    server.update(&world, Vec3::ZERO, now);
    assert!(!server.out.iter().any(|(_, message)| matches!(message, Message::Details { .. })));
    assert_eq!(server.take_group_refresh(), Some(group));
    assert_eq!(server.take_group_refresh(), None);
    world.group_data.request_members(group);
    let commands = world.group_data.take_commands();
    let [NetCommand::GroupMembersRequest { request, .. }] = commands.as_slice() else {
        panic!("one shared request")
    };
    let make_member = |id| GroupMember {
        id,
        contribution: 0,
        powers: 0,
        title: String::new(),
        last_seen: LastSeen::Online,
        is_owner: false,
    };
    world.group_data.on_event(&aurora_net::NetEvent::GroupMembers(Box::new(MembersPage {
        group,
        request: *request,
        total: 2,
        members: vec![make_member(member)],
        whole: false,
    })));
    server.update(&world, Vec3::ZERO, now);
    assert!(!server.out.iter().any(|(_, message)| matches!(message, Message::Details { .. })));
    world.group_data.on_event(&aurora_net::NetEvent::GroupMembers(Box::new(MembersPage {
        group,
        request: *request,
        total: 2,
        members: vec![make_member(world.agent_id)],
        whole: false,
    })));
    server.update(&world, Vec3::ZERO, now);
    server.update(&world, Vec3::ZERO, now + SEND_INTERVAL);
    let targets: Vec<_> = server
        .out
        .iter()
        .filter_map(|(peer, message)| matches!(message, Message::Details { .. }).then_some(*peer))
        .collect();
    assert_eq!(targets, [member]);
    world.groups.groups.clear();
    server.update(&world, Vec3::ZERO, now + SEND_INTERVAL + Duration::from_millis(1));
    assert!(!server.out.iter().any(|(_, message)| matches!(message, Message::Details { .. })));
}

#[test]
fn public_portals_are_complete_without_requests_and_private_queues_cannot_broadcast_or_grow_unbounded() {
    let (world, mut state) = fixture();
    let now = Instant::now();
    let p = portal();
    receive(&mut state, &world, Uuid::from_u128(2), Message::Open(p.announcement()), now);
    state.update(&world, Vec3::ZERO, now);
    state.update(&world, Vec3::ZERO, now + Duration::from_millis(1));
    assert_eq!(state.last_exchange, Some(now), "idle denial checks do not poll at render frequency");
    assert_eq!(state.entries[0].resolved(), Some(p.clone()));
    assert!(state.fetches.is_empty() && state.out.is_empty());
    let p = Portal {
        access: Access::Friends,
        ..p
    };
    let oversized = Portal {
        destination: "\"".repeat(128),
        position: [8191.9995, 8191.9995, 4095.9998],
        access: Access::Group(Uuid::from_u128(20)),
        ..p.clone()
    };
    assert!(
        !state.create(oversized, world.agent.position, now, 1000),
        "the redacted announcement cannot hide an oversized detail response"
    );
    let ask = Message::Request {
        region: 256,
        id: p.id,
        nonce: Uuid::from_u128(5),
    };
    assert!(!state.queue(Uuid::nil(), ask.clone()));
    assert!(!state.queue(
        Uuid::nil(),
        Message::Details {
            region: 256,
            id: p.id,
            nonce: Uuid::from_u128(5),
            details: p.details()
        }
    ));
    state.create(p, world.agent.position, now, 1000);
    for peer in 3..(LIMIT * 6) {
        receive(&mut state, &world, Uuid::from_u128(peer as u128), ask.clone(), now);
    }
    state.update(&world, Vec3::ZERO, now + SEND_INTERVAL);
    assert!(state.inquiries.len() <= LIMIT * 4);
    assert!(state.out.iter().all(|(_, message)| matches!(message, Message::Open(_))));
    assert!(include_str!("../bridge/query.lsl").contains("llJsonValueType(decoded, [\"Details\"])"));
}
