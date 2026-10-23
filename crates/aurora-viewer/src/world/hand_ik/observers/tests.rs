use super::*;
use crate::{
    demo,
    scene::{avatar::AvatarLibrary, ik},
    world::rig_graph::Definition,
};
use aurora_net::NetEvent;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Arc;

const RIDER: Uuid = Uuid::from_u128(0x9840_bbbb);
const BUILDER: Uuid = Uuid::from_u128(0x9840_cccc);

fn fixture(worn: bool) -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.2, !worn);
    objects[3].full_id = RIDER;
    if worn {
        objects[0].parent_id = objects[3].local_id;
    }
    for o in objects.iter_mut().take(3) {
        o.owner_id = if worn { RIDER } else { BUILDER };
    }
    let mut local = demo::action_avatar(false);
    local.local_id = 9999;
    objects.push(local);
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    world
}

fn definition() -> Definition {
    serde_json::from_value(serde_json::json!({"nodes":[],"bindings":[{
        "bone":"mHead","mode":"direct","target":{"kind":"bone","bone":"mPelvis"},
        "offset":{"position":[0.1,0.2,0.3],"rotation":[0.0,0.0,0.0,1.0]}
    }]}))
    .unwrap()
}

fn with_profile(mut chat: ChatMessage, name: &str, cycles: &[u32], mask: u32) -> ChatMessage {
    let mut packet: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(graph_wire::PREFIX).unwrap()).unwrap();
    packet["profile"] = name.into();
    packet["cycles"] = serde_json::json!(cycles);
    packet["mask"] = mask.into();
    chat.message = format!("{}{packet}", graph_wire::PREFIX);
    chat
}

#[test]
fn named_profiles_switch_cached_bindings_for_observers_and_wearers_preserving_shared_contact_lifetimes() {
    for worn in [false, true] {
        let mut world = fixture(worn);
        let now = Instant::now();
        let mut rig = definition();
        let mut right = rig.bindings[0].clone();
        right.bone = "mWristRight".into();
        rig.bindings.push(right);
        rig.profiles = vec![
            crate::world::rig_graph::Profile {
                bindings: None,
                smoothing: None,
                name: "drive".into(),
                bones: vec!["mHead".into(), "mWristRight".into()],
            },
            crate::world::rig_graph::Profile {
                bindings: None,
                smoothing: None,
                name: "gas_only".into(),
                bones: vec!["mWristRight".into()],
            },
        ];
        let command = world.update_hand_contacts(now).unwrap();
        let bundle = Uuid::new_v4();
        let send = |world: &mut World, revision, enabled, profile: &str, cycles: &[u32], mask, definition| {
            for packet in reply(&command, revision, enabled, bundle, definition) {
                let mut packet = with_profile(packet, profile, cycles, mask);
                if worn {
                    packet.owner_id = RIDER;
                }
                world.receive_hand_contacts(&packet, now);
            }
        };
        fn contacts(w: &World) -> &Contacts {
            w.hand_contacts.for_avatar(w.agent_id, RIDER).unwrap()
        }
        send(&mut world, 1, false, "", &[0, 0], 0xffff, Some(&rig));
        assert!(contacts(&world).graphs().is_empty());
        send(&mut world, 2, true, "drive", &[1, 1], 0, None);
        let first = contacts(&world).graphs()[0].activation.clone();
        let cached = contacts(&world).graphs()[0].definition.clone();
        assert_eq!(contacts(&world).graphs()[0].bones.len(), 2);
        send(&mut world, 3, true, "gas_only", &[1, 1], 1, None);
        assert_eq!(contacts(&world).graphs()[0].bones, vec!["mWristRight"]);
        assert_eq!(
            contacts(&world).graphs()[0].activation.cycles[1],
            first.cycles[1],
            "the shared hand does not reactivate"
        );
        send(&mut world, 4, true, "drive", &[2, 1], 0, None);
        let second = contacts(&world).graphs()[0].activation.clone();
        assert_ne!(second.cycles[0], first.cycles[0]);
        assert_eq!(second.cycles[1], first.cycles[1]);
        assert!(Arc::ptr_eq(&contacts(&world).graphs()[0].definition, &cached));
        send(&mut world, 5, true, "drive", &[2, 1], 0, None);
        assert_eq!(contacts(&world).graphs()[0].activation, second, "duplicate commands are idempotent");
        // Intermediate release was never received. Source counters still
        // distinguish the returning hand while the other keeps its lifetime.
        send(&mut world, 6, true, "drive", &[3, 1], 0, None);
        let missed = contacts(&world).graphs()[0].activation.clone();
        assert_ne!(missed.cycles[0], second.cycles[0]);
        assert_eq!(missed.cycles[1], second.cycles[1]);
        send(&mut world, 7, true, "unknown", &[3, 1], 0, None);
        assert_eq!(
            contacts(&world).graphs()[0].activation,
            missed,
            "unknown keywords are rejected atomically"
        );
        send(&mut world, 8, false, "drive", &[3, 1], 0, None);
        assert!(contacts(&world).graphs().is_empty());
        send(&mut world, 9, true, "drive", &[4, 2], 0, None);
        assert!(world.hand_contacts.graphs().is_empty(), "the observer is unaffected");
        let root = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
        world.objects.remove_idx(root);
        world.update_hand_contacts(now + Duration::from_millis(100));
        assert!(
            contacts(&world).graphs().is_empty(),
            "source lifecycle overrides the selected preset"
        );
    }
}

#[test]
fn named_profiles_ignore_server_animations_and_local_previews() {
    let mut world = fixture(false);
    world.agent_id = RIDER;
    let mut rig = definition();
    rig.profiles = vec![crate::world::rig_graph::Profile {
        bindings: None,
        smoothing: None,
        name: "drive".into(),
        bones: vec!["mHead".into()],
    }];
    let now = Instant::now();
    let command = world.update_hand_contacts(now).unwrap();
    let bundle = Uuid::new_v4();
    for packet in reply(&command, 1, true, bundle, Some(&rig)) {
        world.receive_hand_contacts(&with_profile(packet, "drive", &[1], 0), now);
    }
    let first = world.hand_contacts.graphs()[0].activation.clone();
    let animation = Uuid::from_u128(0xb001);
    world.preview_animation(animation, true);
    for anims in [vec![(animation, 1)], Vec::new(), vec![(animation, 2)]] {
        world.apply(NetEvent::AvatarAnimations {
            avatar: RIDER,
            anims,
            sources: Vec::new(),
        });
        assert_eq!(world.hand_contacts.graphs()[0].activation, first);
    }
    world.preview_animation(animation, false);
    assert_eq!(world.hand_contacts.graphs()[0].activation, first);
    for packet in reply(&command, 2, false, bundle, None) {
        world.receive_hand_contacts(&with_profile(packet, "drive", &[1], 0), now);
    }
    assert!(world.hand_contacts.graphs().is_empty());
}

fn reply(command: &NetCommand, revision: u32, enabled: bool, bundle: Uuid, rig: Option<&Definition>) -> Vec<ChatMessage> {
    let NetCommand::ScriptDialogReply {
        object_id,
        channel: CHANNEL,
        label,
        ..
    } = command
    else {
        panic!("observer probe")
    };
    let fields: Vec<_> = label.split('|').collect();
    assert_eq!(fields[0], "auroraIKObserve");
    assert_eq!(
        *object_id,
        Uuid::parse_str(fields[1]).unwrap(),
        "address the root, like a dialog reply at the object's position"
    );
    let bytes = rig.map(|d| serde_json::to_vec(d).unwrap());
    let chunks: Vec<_> = bytes.as_ref().map(|b| b.chunks(80).collect()).unwrap_or_default();
    (0..chunks.len().max(1))
        .map(|part| {
            let mut packet = serde_json::json!({"avatar":fields[2],"nonce":fields[3],"revision":revision,
            "bundle":bundle,"epoch":Uuid::from_u128(1),"enabled":enabled,"mask":0,"time":2.0});
            if !chunks.is_empty() {
                packet["part"] = part.into();
                packet["count"] = chunks.len().into();
                packet["data"] = STANDARD.encode(chunks[part]).into();
            }
            ChatMessage {
                from_name: "Demo IK publisher".into(),
                source_id: *object_id,
                owner_id: BUILDER,
                source_type: ChatSourceType::Object,
                chat_type: ChatType::Other(9),
                position: Vec3::ZERO,
                message: format!("{}{}", graph_wire::PREFIX, packet),
            }
        })
        .collect()
}

#[test]
fn network_renewals_preserve_solved_contacts_for_driver_observer_and_worn_sources() {
    for observed in [false, true] {
        for worn in [false, true] {
            let mut world = fixture(worn);
            if !observed {
                world.agent_id = RIDER;
            }
            let now = Instant::now();
            let mut definition = definition();
            definition.bindings[0].bone = "mWristLeft".into();
            let mut right = definition.bindings[0].clone();
            right.bone = "mWristRight".into();
            right.offset.position[1] *= -1.0;
            definition.bindings.push(right);
            let bundle = Uuid::new_v4();
            let mut previous = world.update_hand_contacts(now).unwrap();
            let send = |world: &mut World, command: &NetCommand, revision, data, at| {
                for mut packet in reply(command, revision, true, bundle, data) {
                    if worn {
                        packet.owner_id = RIDER;
                    }
                    world.receive_hand_contacts(&packet, at);
                }
            };
            send(&mut world, &previous, 1, Some(&definition), now);
            let contacts = |w: &World| {
                w.hand_contacts.for_avatar(w.agent_id, RIDER).unwrap().graphs()[0]
                    .activation
                    .clone()
            };
            let first = contacts(&world);
            assert!(!first.session.is_nil());
            let rig = world.avatar_lib.rig.clone();
            let avatar = world.objects.index_of_uuid(&RIDER).unwrap();
            let mut pose = ik::Pose::default();
            pose.targets.extend(ik::targets(&world, avatar, 0.0, now));
            pose.apply(&rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, now, false);
            let expected = pose
                .apply(
                    &rig,
                    &rig.local_rot,
                    &rig.local_pos,
                    Vec3::ZERO,
                    &rig.scale,
                    now + Duration::from_secs(1),
                    false,
                )
                .unwrap();
            assert!(expected.positions.iter().zip(&rig.local_pos).any(|(a, b)| a.distance(*b) > 0.05));
            for renewal in 1..=4 {
                let at = now + (REFRESH + Duration::from_secs(1)) * renewal;
                let fresh = world.update_hand_contacts(at).unwrap();
                let nonce = |command: &NetCommand| match command {
                    NetCommand::ScriptDialogReply { label, .. } => label.split('|').nth(3).unwrap().to_owned(),
                    _ => panic!("probe"),
                };
                assert_ne!(nonce(&fresh), nonce(&previous), "authentication must still rotate");
                // A late response from the preceding request cannot change state.
                send(&mut world, &previous, 1000, None, at);
                assert_eq!(contacts(&world), first);
                // Cached heartbeats and same-definition full reloads are equivalent.
                send(&mut world, &fresh, renewal + 1, (renewal % 2 == 0).then_some(&definition), at);
                assert_eq!(
                    contacts(&world),
                    first,
                    "renewal restarted contacts: observed={observed}, worn={worn}"
                );
                pose.targets = ik::targets(&world, avatar, 0.0, at);
                assert_eq!(pose.targets.len(), 2);
                let actual = pose
                    .apply(&rig, &rig.local_rot, &rig.local_pos, Vec3::ZERO, &rig.scale, at, false)
                    .unwrap();
                for (a, b) in actual.rotations.iter().zip(&expected.rotations) {
                    assert!(a.dot(*b).abs() > 0.999999, "renewal must not restart entry smoothing");
                }
                for (a, b) in actual.positions.iter().zip(&expected.positions) {
                    assert!(a.abs_diff_eq(*b, 1e-5), "renewal must not release the hands");
                }
                previous = fresh;
            }
        }
    }
}

#[test]
fn observers_accept_authenticated_publishers_without_object_update_ownership_and_keep_wearer_checks() {
    let now = Instant::now();
    for worn in [false, true] {
        let mut world = fixture(worn);
        let source = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
        world.objects.get_mut(source).unwrap().owner_id = Uuid::nil();
        let command = world
            .update_hand_contacts(now)
            .expect("discover from real parentage even without the owner field");
        let packets = reply(&command, 1, true, Uuid::new_v4(), Some(&definition()));
        if worn {
            for packet in &packets {
                world.receive_hand_contacts(packet, now);
            }
            assert!(
                world.hand_contacts.for_avatar(world.agent_id, RIDER).unwrap().graphs().is_empty(),
                "wrong wearer owner"
            );
        }
        for mut packet in packets {
            if worn {
                packet.owner_id = RIDER;
            }
            world.receive_hand_contacts(&packet, now);
        }
        assert_eq!(world.hand_contacts.for_avatar(world.agent_id, RIDER).unwrap().graphs().len(), 1);
        assert!(world.hand_contacts.graphs().is_empty(), "only the observed avatar is affected");
    }
}

#[test]
fn driver_and_observer_constraints_coexist_and_fast_stand_sit_requires_a_new_nonce() {
    let now = Instant::now();
    let mut world = fixture(false);
    let mut local = demo::action_avatar(true);
    local.local_id = 9999;
    local.parent_id = demo::hand_ik::ROOT;
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![local],
    });
    let own = world.update_hand_contacts(now).unwrap();
    for p in reply(&own, 1, true, Uuid::new_v4(), Some(&definition())) {
        world.receive_hand_contacts(&p, now);
    }
    let observed = world.update_hand_contacts(now + Duration::from_millis(250)).unwrap();
    let old = reply(&observed, 1, true, Uuid::new_v4(), Some(&definition()));
    for p in &old {
        world.receive_hand_contacts(p, now);
    }
    assert_eq!(world.hand_contacts.graphs().len(), 1);
    assert_eq!(world.hand_contacts.observers[&RIDER].graphs().len(), 1);
    let mut standing = demo::action_avatar(false);
    standing.full_id = RIDER;
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![standing],
    });
    world.update_hand_contacts(now + Duration::from_millis(500));
    let mut seated = demo::hand_ik::objects(0.2, true).remove(3);
    seated.full_id = RIDER;
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![seated],
    });
    let fresh = world.update_hand_contacts(now + Duration::from_secs(2)).unwrap();
    assert_ne!(format!("{fresh:?}"), format!("{observed:?}"));
    for p in &old {
        world.receive_hand_contacts(p, now + Duration::from_secs(2));
    }
    assert!(world.hand_contacts.observers[&RIDER].graphs().is_empty());
    assert_eq!(world.hand_contacts.graphs().len(), 1, "the driver's independent session survives");
    for p in reply(&fresh, 1, true, Uuid::new_v4(), Some(&definition())) {
        world.receive_hand_contacts(&p, now + Duration::from_secs(2));
    }
    assert_eq!(world.hand_contacts.observers[&RIDER].graphs().len(), 1);
}

#[test]
fn arrival_and_region_return_rediscover_identical_object_ids_without_reusing_the_old_session() {
    let now = Instant::now();
    let mut world = fixture(false);
    let command = world.update_hand_contacts(now).unwrap();
    let packets = reply(&command, 1, true, Uuid::new_v4(), Some(&definition()));
    for p in &packets {
        world.receive_hand_contacts(p, now);
    }
    world.teleporting = true;
    assert!(world.update_hand_contacts(now + Duration::from_secs(1)).is_none());
    world.teleporting = false;
    world.main_region = Some(2);
    let mut objects = demo::hand_ik::objects(0.2, true);
    objects[3].full_id = RIDER;
    for o in objects.iter_mut().take(3) {
        o.owner_id = BUILDER;
    }
    let mut local = demo::action_avatar(false);
    local.local_id = 9999;
    objects.push(local);
    world.apply(NetEvent::ObjectUpdates { handle: 2, objects });
    let fresh = world.update_hand_contacts(now + Duration::from_secs(2)).unwrap();
    for p in &packets {
        world.receive_hand_contacts(p, now + Duration::from_secs(2));
    }
    assert!(world.hand_contacts.observers[&RIDER].graphs().is_empty());
    for p in reply(&fresh, 1, true, Uuid::new_v4(), Some(&definition())) {
        world.receive_hand_contacts(&p, now + Duration::from_secs(2));
    }
    assert_eq!(world.hand_contacts.observers[&RIDER].graphs()[0].source.region, 2);
}

#[test]
fn discovery_budget_is_shared_across_avatars_and_known_leases_precede_unknown_attachments() {
    let now = Instant::now();
    let mut world = fixture(false);
    let command = world.update_hand_contacts(now).unwrap();
    for p in reply(&command, 1, true, Uuid::new_v4(), Some(&definition())) {
        world.receive_hand_contacts(&p, now);
    }
    let mut additions = Vec::new();
    for n in 0..4u32 {
        let id = Uuid::from_u128(0x100000 + u128::from(n));
        let mut avatar = demo::action_avatar(false);
        avatar.full_id = id;
        avatar.local_id = 20000 + n;
        additions.push(avatar);
        for index in 0..128u32 {
            let mut root = demo::hand_ik::objects(0.0, false).remove(0);
            root.local_id = 30000 + n * 128 + index;
            root.full_id = Uuid::from_u128(0x200000 + u128::from(n * 128 + index));
            root.owner_id = id;
            root.parent_id = 20000 + n;
            additions.push(root);
        }
    }
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: additions,
    });
    let refresh = world.update_hand_contacts(now + REFRESH + Duration::from_secs(1)).unwrap();
    let NetCommand::ScriptDialogReply { label, .. } = refresh else {
        panic!("refresh")
    };
    assert_eq!(label.split('|').nth(2).unwrap(), RIDER.to_string());
    let total = world.hand_contacts.sources().len() + world.hand_contacts.observers.values().map(|c| c.sources().len()).sum::<usize>();
    assert!(total <= MAX_SOURCES);
    assert!(total > 128, "exercise the complete cross-avatar budget");
    let mut previous = now + REFRESH + Duration::from_secs(1);
    for tick in 1..=40u64 {
        let at = now + REFRESH + Duration::from_secs(1) + Duration::from_millis(tick * 50);
        if world.update_hand_contacts(at).is_some() {
            assert!(at.saturating_duration_since(previous) >= NETWORK_SPACING);
            previous = at;
        }
    }
}

#[test]
fn unknown_local_attachments_do_not_starve_other_avatars_and_the_seat_wins_bone_conflicts() {
    let now = Instant::now();
    let mut world = fixture(false);
    let mut local = demo::action_avatar(true);
    local.local_id = 9999;
    local.parent_id = demo::hand_ik::ROOT;
    let mut worn = demo::hand_ik::objects(0.0, false).remove(0);
    worn.local_id = 20001;
    worn.full_id = Uuid::from_u128(0x400000);
    worn.owner_id = world.agent_id;
    worn.parent_id = 9999;
    let worn_id = worn.full_id;
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![local, worn],
    });
    let own = world.update_hand_contacts(now).unwrap();
    for p in reply(&own, 1, true, Uuid::new_v4(), Some(&definition())) {
        world.receive_hand_contacts(&p, now);
    }
    let observed = world.update_hand_contacts(now + NETWORK_SPACING).unwrap();
    let NetCommand::ScriptDialogReply { label, .. } = observed else {
        panic!("observer")
    };
    assert_eq!(
        label.split('|').nth(2).unwrap(),
        RIDER.to_string(),
        "discover another avatar before probing more local attachments"
    );
    let mut received = false;
    for tick in 2..10u32 {
        if let Some(command) = world.update_hand_contacts(now + NETWORK_SPACING * tick)
            && let NetCommand::ScriptDialogReply { object_id, .. } = &command
            && *object_id == worn_id
        {
            for mut p in reply(&command, 1, true, Uuid::new_v4(), Some(&definition())) {
                p.owner_id = world.agent_id;
                world.receive_hand_contacts(&p, now + NETWORK_SPACING * tick);
            }
            received = true;
            break;
        }
    }
    assert!(received);
    assert_eq!(world.hand_contacts.graphs().len(), 1);
    assert_eq!(world.hand_contacts.graphs()[0].source.root, demo::action_id(demo::hand_ik::ROOT));
    let mut local = demo::action_avatar(false);
    local.local_id = 9999;
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![local],
    });
    world.update_hand_contacts(now + Duration::from_secs(4));
    assert_eq!(world.hand_contacts.graphs().len(), 1);
    assert_eq!(
        world.hand_contacts.graphs()[0].source.root,
        worn_id,
        "standing restores the independently cached worn source"
    );
}

#[test]
fn observers_negotiate_independently_and_solve_for_the_rider_rather_than_the_observer() {
    let now = Instant::now();
    let mut a = fixture(false);
    let mut b = fixture(false);
    let ca = a.update_hand_contacts(now).unwrap();
    let cb = b.update_hand_contacts(now).unwrap();
    let bundle = Uuid::new_v4();
    let packets = reply(&ca, 1, true, bundle, Some(&definition()));
    for packet in packets.iter().rev() {
        assert!(a.receive_hand_contacts(packet, now));
    }
    assert!(a.hand_contacts.graphs().is_empty(), "the observing avatar stays untouched");
    let remote = a.objects.index_of_uuid(&RIDER).unwrap();
    let local = a.objects.index_of_uuid(&a.agent_id).unwrap();
    assert!(ik::targets(&a, local, 0.0, now).is_empty());
    let goals = ik::targets(&a, remote, 0.0, now);
    assert_eq!(goals.len(), 1);
    assert!(
        !goals[0].follow.as_ref().unwrap().context.external(),
        "nil avatar means this rider's working pose"
    );
    let rig = &a.avatar_lib.rig;
    let matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
    let expected = matrices[rig.pelvis].w_axis.truncate() + Vec3::new(0.1, 0.2, 0.3);
    let followed = goals[0]
        .follow
        .as_ref()
        .unwrap()
        .context
        .target(&definition().bindings[0].target, &matrices)
        .unwrap();
    assert!(followed.p().distance(matrices[rig.pelvis].w_axis.truncate()) < 1e-5);
    let preview = goals[0].follow.as_ref().unwrap().context.preview(&goals[0], &matrices).unwrap();
    assert!(
        preview.position.distance(expected) < 1e-5,
        "offsets are evaluated in the rider's own pelvis frame"
    );
    for packet in &packets {
        assert!(b.receive_hand_contacts(packet, now));
    }
    assert!(
        ik::targets(&b, remote, 0.0, now).is_empty(),
        "another observer's nonce cannot commit"
    );
    for packet in reply(&cb, 1, true, bundle, Some(&definition())) {
        assert!(b.receive_hand_contacts(&packet, now));
    }
    assert_eq!(ik::targets(&b, remote, 0.0, now).len(), 1);
    for packet in reply(&ca, 2, false, bundle, None) {
        a.receive_hand_contacts(&packet, now);
    }
    assert!(ik::targets(&a, remote, 0.0, now).is_empty());
    assert_eq!(ik::targets(&b, remote, 0.0, now).len(), 1, "independent subscription state");
}

#[test]
fn observer_lifecycle_releases_on_stand_detach_deletion_region_change_teleport_and_reset() {
    let now = Instant::now();
    for action in 0..7 {
        let worn = action == 1;
        let mut world = fixture(worn);
        let command = world.update_hand_contacts(now).unwrap();
        let mut packets = reply(&command, 1, true, Uuid::new_v4(), Some(&definition()));
        if worn {
            for p in &mut packets {
                p.owner_id = RIDER;
            }
        }
        for p in &packets {
            assert!(world.receive_hand_contacts(p, now));
        }
        let avatar = world.objects.index_of_uuid(&RIDER).unwrap();
        assert_eq!(ik::targets(&world, avatar, 0.0, now).len(), 1);
        match action {
            0 => {
                let mut o = demo::action_avatar(false);
                o.full_id = RIDER;
                world.apply(NetEvent::ObjectUpdates {
                    handle: 1,
                    objects: vec![o],
                });
            }
            1 => {
                let mut o = demo::hand_ik::objects(0.0, false).remove(0);
                o.owner_id = RIDER;
                world.apply(NetEvent::ObjectUpdates {
                    handle: 1,
                    objects: vec![o],
                });
            }
            2 => {
                let i = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
                world.objects.remove_idx(i);
            }
            3 => world.teleporting = true,
            4 => world.main_region = Some(2),
            5 => {
                world.reset();
            }
            _ => {
                world.update_hand_contacts(now + LEASE + Duration::from_secs(1));
            }
        }
        assert!(
            ik::targets(&world, avatar, 0.0, now).is_empty(),
            "action {action} must release without a final script packet"
        );
        world.update_hand_contacts(now + Duration::from_secs(2));
        for p in &packets {
            world.receive_hand_contacts(p, now + Duration::from_secs(2));
        }
        assert!(
            ik::targets(&world, avatar, 0.0, now).is_empty(),
            "old replies must not resurrect action {action}"
        );
    }
}

#[test]
fn observer_rejects_wrong_owner_subject_nonce_and_out_of_order_state_and_bounds_probes() {
    let mut world = fixture(false);
    let now = Instant::now();
    let command = world.update_hand_contacts(now).unwrap();
    assert!(world.update_hand_contacts(now + Duration::from_millis(100)).is_none());
    let bundle = Uuid::new_v4();
    for action in 0..3 {
        let packets = reply(&command, 1, true, bundle, Some(&definition()));
        for mut p in packets {
            if action == 0 {
                p.owner_id = Uuid::new_v4();
            } else {
                let mut data: serde_json::Value = serde_json::from_str(p.message.strip_prefix(graph_wire::PREFIX).unwrap()).unwrap();
                data[if action == 1 { "avatar" } else { "nonce" }] = Uuid::new_v4().to_string().into();
                p.message = format!("{}{data}", graph_wire::PREFIX);
            }
            world.receive_hand_contacts(&p, now);
        }
        assert!(world.hand_contacts.observers[&RIDER].graphs().is_empty());
    }
    for p in reply(&command, 2, true, bundle, Some(&definition())) {
        world.receive_hand_contacts(&p, now);
    }
    for p in reply(&command, 3, false, bundle, None) {
        world.receive_hand_contacts(&p, now);
    }
    for p in reply(&command, 2, true, bundle, Some(&definition())) {
        world.receive_hand_contacts(&p, now);
    }
    assert!(world.hand_contacts.observers[&RIDER].graphs().is_empty());
    assert!(
        world.update_hand_contacts(now + REFRESH + Duration::from_secs(1)).is_some(),
        "arrival state renews through bounded probes"
    );
}
