use super::*;
use crate::{demo, scene::avatar::AvatarLibrary};
use aurora_net::NetEvent;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Arc;

const RIDER: Uuid = Uuid::from_u128(0x9840_bbbb);
const BRIDGE: Uuid = Uuid::from_u128(0x9870);
const SESSION: Uuid = Uuid::from_u128(0x9871);
const BUNDLE: Uuid = Uuid::from_u128(0x9872);

fn fixture(observed: bool, worn: bool) -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = if observed { demo::DEMO_AGENT } else { RIDER };
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.2, !worn);
    objects[3].full_id = RIDER;
    if worn {
        objects[0].parent_id = objects[3].local_id;
    }
    for object in objects.iter_mut().take(3) {
        object.owner_id = RIDER;
    }
    let mut bridge = objects[0].clone();
    bridge.local_id = 9870;
    bridge.full_id = BRIDGE;
    bridge.owner_id = world.agent_id;
    if observed {
        let mut local = demo::action_avatar(false);
        local.local_id = 9999;
        bridge.parent_id = local.local_id;
        objects.push(local);
    } else {
        bridge.parent_id = objects[3].local_id;
    }
    objects.push(bridge);
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    world.hand_contacts.relay_required = true;
    world.hand_contacts.set_relay(Relay {
        root: BRIDGE,
        region: 1,
        session: SESSION,
    });
    world
}

fn envelope(world: &World, message: &str) -> ChatMessage {
    ChatMessage {
        from_name: "Offline bridge IK".into(),
        source_id: BRIDGE,
        owner_id: world.agent_id,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        position: Vec3::ZERO,
        message: format!(
            "<auroraIKRelay>{SESSION}|{}|{RIDER}|{message}</auroraIKRelay>",
            demo::action_id(demo::hand_ik::ROOT)
        ),
    }
}

fn receive(world: &mut World, message: &str, now: Instant) {
    world.receive_ik_relay(&envelope(world, message), now);
}

fn announce(world: &mut World, now: Instant) {
    receive(world, &format!("{AVAILABLE}{BUNDLE}|[\"{RIDER}\"]"), now);
}

fn contacts(world: &World) -> &Contacts {
    world.hand_contacts.for_avatar(world.agent_id, RIDER).unwrap()
}

fn declaration(world: &mut World, command: &NetCommand, now: Instant) {
    for packet in declaration_packets(command, BUNDLE, 1) {
        receive(world, &packet, now);
    }
}

fn declaration_packets(command: &NetCommand, bundle: Uuid, revision: u32) -> Vec<String> {
    let NetCommand::ScriptDialogReply {
        object_id, channel, label, ..
    } = command
    else {
        panic!("relay command")
    };
    assert_eq!((*object_id, *channel), (BRIDGE, super::super::super::bridge::ik::CONTROL));
    let fields: Vec<_> = label.split('|').collect();
    assert_eq!(&fields[..3], &["auroraIKRelay", &SESSION.to_string(), "observe"]);
    assert_eq!(fields[4], RIDER.to_string());
    let rig = serde_json::json!({"nodes":[],"bindings":[{
        "bone":"mWristLeft","target":{"kind":"prim","id":demo::action_id(demo::hand_ik::LEFT)}
    }],"profiles":[{"name":"drive","bones":["mWristLeft"]}]});
    let bytes = serde_json::to_vec(&rig).unwrap();
    let parts: Vec<_> = bytes.chunks(160).collect();
    parts
        .iter()
        .enumerate()
        .rev()
        .map(|(part, bytes)| {
            let packet = serde_json::json!({"avatar":RIDER,"nonce":fields[5],"revision":revision,"bundle":bundle,
            "epoch":bundle,"enabled":false,"mask":65535,"profile":"","cycles":[0],"time":0.0,
            "part":part,"count":parts.len(),"data":STANDARD.encode(bytes)});
            format!("{PREFIX}{packet}")
        })
        .collect()
}

fn state(world: &mut World, revision: u32, enabled: bool, generation: u32, now: Instant) {
    let packet = serde_json::json!({"revision":revision,"bundle":BUNDLE,
        "epoch":Uuid::from_u128(100 + u128::from(generation)),"enabled":enabled,"mask":0,
        "profile":"drive","cycles":[generation],"time":0.0});
    receive(world, &format!("{STATE}{packet}"), now);
}

#[test]
fn ik_diagnostics_explain_reception_and_editor_suppression_without_retaining_private_packets() {
    let mut world = fixture(false, true);
    let now = Instant::now();
    announce(&mut world, now);
    let request = world.update_hand_contacts(now).unwrap();
    declaration(&mut world, &request, now);
    state(&mut world, 2, true, 1, now);
    let NetCommand::ScriptDialogReply { label, .. } = &request else {
        panic!("request")
    };
    let fields: Vec<_> = label.split('|').collect();
    let malformed = serde_json::json!({"avatar":RIDER,"nonce":fields[5],"revision":3,"bundle":BUNDLE,
        "epoch":BUNDLE,"enabled":true,"mask":0,"time":0.0,"profile":"drive","cycles":[1],
        "part":0,"count":1,"data":"*invalid*"});
    receive(&mut world, &format!("{PREFIX}{malformed}"), now);
    state(&mut world, 4, false, 1, now);
    let mut truncated = envelope(&world, &format!("{PREFIX}{malformed}"));
    truncated.message.truncate(truncated.message.len() - 5);
    world.receive_ik_relay(&truncated, now);
    let bytes = world.hand_contacts.total_retained_bytes();
    let report = world.describe_ik(true, true, 3, now);
    assert!(report.contains("editor_open=true suppress_scripts=true editor_goals=3"));
    assert!(report.contains("definition=true enabled=false profile=\"drive\""));
    assert!(report.contains("active_packets=1"));
    assert!(report.contains("last_failure=invalid_base64"));
    assert!(report.contains("last_failure=malformed_or_truncated_envelope"));
    for id in [SESSION.to_string(), BUNDLE.to_string(), RIDER.to_string(), fields[5].into()] {
        assert!(!report.contains(&id), "identities and authentication are omitted");
    }
    assert!(report.len() < 64 * 1024);
    assert_eq!(world.hand_contacts.total_retained_bytes(), bytes);
    assert!(world.update_hand_contacts(now + Duration::from_secs(300)).is_none());
}

#[test]
fn packed_utf8_declarations_commit_atomically_for_local_and_observed_attachments() {
    for observed in [false, true] {
        let mut world = fixture(observed, true);
        let now = Instant::now();
        announce(&mut world, now);
        let NetCommand::ScriptDialogReply { label, .. } = world.update_hand_contacts(now).unwrap() else {
            panic!("request")
        };
        let fields: Vec<_> = label.split('|').collect();
        let bindings: Vec<_> = ["Left", "Right"]
            .into_iter()
            .flat_map(|side| {
                ["Wrist", "HandIndex3", "HandThumb3"].into_iter().map(move |joint| {
                    serde_json::json!({
                        "bone":format!("m{joint}{side}"),
                        "target":{"kind":"link","object":"source","link":format!("Poignée {side}")},
                        "offset":{"position":[0.01,0.02,0.03],"rotation":[0,0,0,1]}
                    })
                })
            })
            .collect();
        let definition = serde_json::json!({"nodes":[],"bindings":bindings,"profiles":[
            {"name":"gauche","bindings":[0,1,2]},{"name":"droite","bindings":[3,4,5]}
        ]});
        let bytes = serde_json::to_vec(&definition).unwrap();
        let encoded = STANDARD.encode(&bytes);
        let blocks: Vec<_> = encoded.as_bytes().chunks(256).collect();
        let packed: Vec<_> = blocks.chunks(2).map(|pair| pair.concat()).collect();
        assert!(packed.len() < blocks.len());
        for (received, (part, data)) in packed.iter().enumerate().rev().enumerate() {
            let packet = serde_json::json!({"avatar":RIDER,"nonce":fields[5],"revision":1,"bundle":BUNDLE,
                "epoch":BUNDLE,"enabled":false,"mask":65535,"profile":"","cycles":[0,0,0,0,0,0],"time":0.0,
                "part":part,"count":packed.len(),"data":std::str::from_utf8(data).unwrap()});
            let message = format!("{PREFIX}{packet}");
            assert!(message.len() <= 880);
            assert!(envelope(&world, &message).message.len() <= 1024);
            receive(&mut world, &message, now + Duration::from_millis(received as u64 * 100));
            let session = contacts(&world).worn.get(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
            assert_eq!(session.graph.definition.is_some(), received + 1 == packed.len());
        }
        let session = contacts(&world).worn.get(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
        let expected: crate::world::rig_graph::Definition = serde_json::from_value(definition).unwrap();
        assert_eq!(session.graph.definition.as_ref().unwrap().as_ref(), &expected);
        assert!(contacts(&world).graphs().is_empty(), "preloading does not activate IK");
    }
}

#[test]
fn an_early_release_cannot_acknowledge_a_missing_definition_and_stop_initial_retries() {
    for observed in [false, true] {
        for worn in [false, true] {
            for partial in [false, true] {
                let mut world = fixture(observed, worn);
                let now = Instant::now();
                announce(&mut world, now);
                let initial = world.update_hand_contacts(now).unwrap();
                let mut packets = declaration_packets(&initial, BUNDLE, 1);
                assert!(packets.len() > 1);
                let last = packets.pop().unwrap();
                if partial {
                    for packet in packets {
                        receive(&mut world, &packet, now + Duration::from_millis(10));
                    }
                    assert!(contacts(&world).sessions().any(|s| s.graph.transferring(now)));
                }
                // The off push overtakes the requested definition on the network.
                state(&mut world, 2, false, 1, now + Duration::from_millis(100));
                receive(&mut world, &last, now + Duration::from_millis(200));
                let session = contacts(&world).sessions().find(|s| s.seat.is_some()).unwrap();
                assert!(session.graph.definition.is_none());
                assert!(session.received_at.is_none(), "a release is not a complete definition");
                let retry_at = now + Duration::from_secs(3);
                let retry = world.update_hand_contacts(retry_at).expect("finite initial retry");
                declaration(&mut world, &retry, retry_at);
                assert!(contacts(&world).sessions().any(|s| s.graph.definition.is_some()));
                state(&mut world, 3, true, 2, retry_at);
                assert_eq!(contacts(&world).graphs().len(), 1);
                for seconds in [4, 25, 70, 300] {
                    assert!(world.update_hand_contacts(now + Duration::from_secs(seconds)).is_none());
                }
            }
        }
    }
}

#[test]
fn a_new_bundle_requires_its_own_complete_definition_even_after_late_old_state_and_release() {
    for observed in [false, true] {
        for worn in [false, true] {
            let mut world = fixture(observed, worn);
            let now = Instant::now();
            announce(&mut world, now);
            let initial = world.update_hand_contacts(now).unwrap();
            declaration(&mut world, &initial, now);
            state(&mut world, 2, true, 1, now);
            let next = Uuid::from_u128(0x9873);
            let at = now + Duration::from_secs(1);
            receive(&mut world, &format!("{AVAILABLE}{next}|[\"{RIDER}\"]"), at);
            state(&mut world, 3, false, 1, at);
            let request = world.update_hand_contacts(at).expect("new definition request");
            let packet = |revision, enabled| {
                serde_json::json!({
                    "bundle":next,"epoch":next,"revision":revision,"enabled":enabled,
                    "mask":0,"profile":"drive","cycles":[2],"time":0.0
                })
            };
            receive(&mut world, &format!("{STATE}{}", packet(2, false)), at);
            let session = contacts(&world).sessions().find(|s| s.seat.is_some()).unwrap();
            assert_eq!(session.graph.bundle(), Some(BUNDLE));
            assert!(session.received_at.is_none(), "the old rig cannot acknowledge the new one");
            for packet in declaration_packets(&request, next, 3) {
                receive(&mut world, &packet, at);
            }
            let session = contacts(&world).sessions().find(|s| s.seat.is_some()).unwrap();
            assert_eq!(session.graph.bundle(), Some(next));
            assert!(session.received_at.is_some());
            receive(&mut world, &format!("{STATE}{}", packet(4, true)), at);
            assert_eq!(contacts(&world).graphs().len(), 1);
            for seconds in [2, 25, 70, 300] {
                assert!(world.update_hand_contacts(now + Duration::from_secs(seconds)).is_none());
            }
        }
    }
}

#[test]
fn relay_preloads_on_use_and_pushes_profiles_without_any_idle_discovery_or_renewal() {
    for observed in [false, true] {
        for worn in [false, true] {
            let mut world = fixture(observed, worn);
            let now = Instant::now();
            for seconds in [0, 1, 25, 70, 300] {
                assert!(world.update_hand_contacts(now + Duration::from_secs(seconds)).is_none());
            }
            assert!(world.hand_contacts.scan_at.is_none());
            assert!(world.hand_contacts.sources().is_empty(), "only announced sources are queried");
            let at = now + Duration::from_secs(301);
            announce(&mut world, at);
            let command = world.update_hand_contacts(at).unwrap();
            declaration(&mut world, &command, at);
            assert!(contacts(&world).graphs().is_empty(), "preloading does not activate IK");
            let cached = contacts(&world)
                .sessions()
                .find(|s| s.graph.definition.is_some())
                .unwrap()
                .graph
                .definition
                .clone()
                .unwrap();
            state(&mut world, 2, true, 1, at);
            let first = contacts(&world).graphs()[0].activation.clone();
            assert_eq!(contacts(&world).graphs()[0].bones, vec!["mWristLeft"]);
            assert!(Arc::ptr_eq(&cached, &contacts(&world).graphs()[0].definition));
            announce(&mut world, at + Duration::from_secs(1));
            for seconds in [1, 25, 70, 3600] {
                assert!(
                    world.update_hand_contacts(at + Duration::from_secs(seconds)).is_none(),
                    "no periodic network traffic"
                );
                assert_eq!(contacts(&world).graphs()[0].activation, first);
            }
            state(&mut world, 3, false, 1, at + Duration::from_secs(3601));
            assert!(contacts(&world).graphs().is_empty());
            state(&mut world, 4, true, 2, at + Duration::from_secs(3602));
            assert_ne!(contacts(&world).graphs()[0].activation, first);
            assert!(
                Arc::ptr_eq(&cached, &contacts(&world).graphs()[0].definition),
                "restart uses preloaded data"
            );
            // A stale state event cannot undo the latest restart.
            state(&mut world, 3, false, 1, at + Duration::from_secs(3603));
            assert_eq!(contacts(&world).graphs().len(), 1);
            let index = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
            world.objects.remove_idx(index);
            assert!(world.update_hand_contacts(at + Duration::from_secs(3604)).is_none());
            assert_eq!(world.hand_contacts.total_retained_bytes(), 0, "source removal releases cached data");
        }
    }
}

#[test]
fn relay_requires_the_exact_worn_root_owner_session_and_live_eligible_subject() {
    let now = Instant::now();
    let mut world = fixture(true, false);
    let message = format!("{AVAILABLE}{BUNDLE}|[\"{RIDER}\"]");
    let valid = envelope(&world, &message);
    for invalid in [
        ChatMessage {
            source_id: Uuid::new_v4(),
            ..valid.clone()
        },
        ChatMessage {
            owner_id: Uuid::new_v4(),
            ..valid.clone()
        },
        ChatMessage {
            message: valid.message.replace(&SESSION.to_string(), &Uuid::new_v4().to_string()),
            ..valid.clone()
        },
        ChatMessage {
            message: valid.message.replace(&RIDER.to_string(), &Uuid::new_v4().to_string()),
            ..valid.clone()
        },
    ] {
        world.receive_ik_relay(&invalid, now);
        assert!(world.hand_contacts.observers.is_empty());
    }
    let index = world.objects.index_of_uuid(&BRIDGE).unwrap();
    world.objects.remove_idx(index);
    world.receive_ik_relay(&valid, now);
    assert!(world.hand_contacts.observers.is_empty());
    assert!(world.update_hand_contacts(now).is_none());
    assert!(world.hand_contacts.scan_at.is_none(), "missing relay cannot start an active search");
    assert!(world.chat.is_empty());
}

#[test]
fn publisher_and_relay_bound_maximum_wire_envelopes_and_use_a_separate_script() {
    let packet = serde_json::json!({"avatar":RIDER,"nonce":SESSION,"revision":i32::MAX,"bundle":BUNDLE,
        "epoch":BUNDLE,"enabled":true,"mask":65535,"profile":"a".repeat(64),
        "cycles":vec![i32::MAX;16],"time":1_000_000_000.0,
        "part":47,"count":48,"data":"a".repeat(256)});
    let world = fixture(true, false);
    assert!(envelope(&world, &format!("{PREFIX}{packet}")).message.len() <= 1024);
    assert!(
        8192usize.div_ceil(189) <= 48,
        "UTF-8 boundaries still fit the reassembly part limit"
    );
    let publisher = include_str!("../../../../../../scripts/lsl/rig-contacts.lsl");
    assert!(publisher.contains("llRegionSay(DISCOVERY, \"auroraIKState/1 \""));
    assert!(publisher.contains("llRegionSayTo(peer, DISCOVERY,"));
    let relay = include_str!("../../bridge/ik.lsl");
    assert!(!relay.contains("llSetTimerEvent"));
    assert!(!relay.contains("llSetForce"));
    assert!(!relay.contains("llRequestSecureURL"));
}

#[test]
fn regional_notice_can_precede_objects_and_reappearance_without_periodic_queries() {
    let now = Instant::now();
    let mut world = fixture(true, false);
    let root_id = demo::action_id(demo::hand_ik::ROOT);
    let index = world.objects.index_of_uuid(&root_id).unwrap();
    world.objects.remove_idx(index);
    announce(&mut world, now);
    assert!(
        world.hand_contacts.observers.is_empty(),
        "no settings before live source validation"
    );
    assert!(world.update_hand_contacts(now).is_none());
    assert!(world.hand_contacts.declarations.bytes() > 0);
    let mut restored = demo::hand_ik::objects(0.2, true);
    restored[3].full_id = RIDER;
    for object in restored.iter_mut().take(3) {
        object.owner_id = RIDER;
    }
    let root = restored[0].clone();
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: vec![root.clone()],
    });
    assert!(
        world.update_hand_contacts(now + Duration::from_millis(500)).is_none(),
        "the rider has not arrived yet"
    );
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: restored[1..].to_vec(),
    });
    let command = world.update_hand_contacts(now + Duration::from_secs(1)).unwrap();
    declaration(&mut world, &command, now + Duration::from_secs(1));
    state(&mut world, 2, true, 1, now + Duration::from_secs(1));
    // Repeated full updates must not retire the request nonce or retransmit.
    for second in 2..10 {
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![root.clone()],
        });
        assert!(world.update_hand_contacts(now + Duration::from_secs(second)).is_none());
    }
    let index = world.objects.index_of_uuid(&root_id).unwrap();
    world.objects.remove_idx(index);
    world.update_hand_contacts(now + Duration::from_secs(10));
    assert_eq!(world.hand_contacts.total_retained_bytes(), 0);
    world.apply(NetEvent::ObjectUpdates {
        handle: 1,
        objects: restored,
    });
    assert!(
        world.update_hand_contacts(now + Duration::from_secs(11)).is_some(),
        "appearance is an event, not a scan"
    );
}

#[test]
fn relay_restart_resynchronizes_cached_state_and_rejects_its_old_private_session() {
    let now = Instant::now();
    let mut world = fixture(true, false);
    announce(&mut world, now);
    let first = world.update_hand_contacts(now).unwrap();
    declaration(&mut world, &first, now);
    state(&mut world, 2, true, 1, now);
    let cached = contacts(&world).graphs()[0].definition.clone();
    let fresh = Uuid::new_v4();
    world.hand_contacts.set_relay(Relay {
        root: BRIDGE,
        region: 1,
        session: fresh,
    });
    let command = world.update_hand_contacts(now + Duration::from_secs(1)).unwrap();
    let NetCommand::ScriptDialogReply { label, .. } = command else {
        panic!("relay command")
    };
    let fields: Vec<_> = label.split('|').collect();
    assert_eq!(fields[1], fresh.to_string());
    assert_eq!(
        fields[6],
        BUNDLE.to_string(),
        "request the current state using the retained definition"
    );
    let packet = serde_json::json!({"avatar":RIDER,"nonce":fields[5],"revision":3,"bundle":BUNDLE,
        "epoch":Uuid::from_u128(101),"enabled":false,"mask":0,"profile":"drive","cycles":[1],"time":0.0});
    let old = envelope(&world, &format!("{PREFIX}{packet}"));
    world.receive_ik_relay(&old, now + Duration::from_secs(1));
    assert_eq!(
        contacts(&world).graphs().len(),
        1,
        "old private envelopes cannot change the new session"
    );
    let current = ChatMessage {
        message: old.message.replace(&SESSION.to_string(), &fresh.to_string()),
        ..old
    };
    world.receive_ik_relay(&current, now + Duration::from_secs(1));
    assert!(
        contacts(&world).graphs().is_empty(),
        "the fresh cached header restores a missed off state"
    );
    assert!(Arc::ptr_eq(
        &cached,
        contacts(&world).sessions().find_map(|s| s.graph.definition.as_ref()).unwrap()
    ));
}

#[test]
fn identity_only_declarations_are_bounded_and_an_empty_use_event_clears_the_source() {
    let now = Instant::now();
    let mut world = fixture(true, false);
    announce(&mut world, now);
    let command = world.update_hand_contacts(now).unwrap();
    declaration(&mut world, &command, now);
    state(&mut world, 2, true, 1, now);
    receive(&mut world, &format!("{AVAILABLE}{BUNDLE}|[]"), now);
    assert!(contacts(&world).graphs().is_empty());
    assert_eq!(world.hand_contacts.total_retained_bytes(), 0);
    assert_eq!(world.hand_contacts.declarations.bytes(), 0);
    for index in 1..=2000u128 {
        let source = Uuid::from_u128(0x100000 + index);
        let mut chat = envelope(&world, &format!("{AVAILABLE}{BUNDLE}|[\"{RIDER}\"]"));
        chat.message = chat
            .message
            .replace(&demo::action_id(demo::hand_ik::ROOT).to_string(), &source.to_string());
        world.receive_ik_relay(&chat, now + Duration::from_millis(index as u64 * 20));
    }
    assert!(world.hand_contacts.declarations.bytes() <= 16 * 1024);
    assert_eq!(
        world.hand_contacts.total_retained_bytes(),
        0,
        "announcements do not cache rig settings"
    );
    assert!(world.update_hand_contacts(now + Duration::from_secs(41)).is_none());
}
