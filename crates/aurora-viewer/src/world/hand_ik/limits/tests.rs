use super::*;
use crate::{demo, scene::avatar::AvatarLibrary, world::rig_graph::MAX_BYTES};
use aurora_net::NetEvent;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::Arc;

fn fixture(worn: bool) -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.0, !worn);
    if worn {
        objects[0].parent_id = objects[3].local_id;
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    world
}

fn chat(command: &NetCommand) -> ChatMessage {
    let NetEvent::Chat(chat) = demo::hand_ik::reply(command).remove(0) else {
        panic!("chat")
    };
    chat
}

fn padded_packets(command: &NetCommand, owner: Uuid, size: usize) -> Vec<ChatMessage> {
    let NetCommand::ScriptDialogReply { object_id, label, .. } = command else {
        panic!("request")
    };
    let fields: Vec<_> = label.split('|').collect();
    let mut packet = serde_json::json!({
        "avatar":fields[2],"nonce":fields[3],"revision":1,
        "bundle":Uuid::from_u128(0x9840_eeee),"epoch":Uuid::from_u128(0x9840_ffff),
        "enabled":true,"time":0.0
    });
    let chat = ChatMessage {
        from_name: "Bounded publisher".into(),
        source_id: *object_id,
        owner_id: owner,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::Other(9),
        position: Vec3::ZERO,
        message: String::new(),
    };
    let mut bytes = br#"{"bindings":[{"bone":"mHead","target":{"kind":"source"}}]}"#.to_vec();
    bytes.resize(size, b' ');
    let parts: Vec<_> = bytes.chunks(320).collect();
    parts
        .iter()
        .enumerate()
        .map(|(i, bytes)| {
            packet["part"] = i.into();
            packet["count"] = parts.len().into();
            packet["data"] = STANDARD.encode(bytes).into();
            let mut chat = chat.clone();
            chat.message = format!("{PREFIX}{packet}");
            chat
        })
        .collect()
}

#[test]
fn unused_announcements_and_unsolicited_payloads_never_create_sessions_or_chat_history() {
    let now = Instant::now();
    for worn in [false, true] {
        let mut world = fixture(worn);
        let command = world.update_hand_contacts(now).unwrap();
        let mut announcement = chat(&command);
        if worn {
            let mut root = demo::hand_ik::objects(0.0, false).remove(0);
            root.parent_id = 0;
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![root],
            });
        } else {
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![demo::action_avatar(false)],
            });
        }
        world.update_hand_contacts(now + Duration::from_secs(1));
        announcement.message = "auroraIKAvailable".into();
        for _ in 0..200 {
            for kind in [ChatType::Normal, ChatType::Whisper, ChatType::Shout] {
                announcement.chat_type = kind;
                world.apply(NetEvent::Chat(announcement.clone()));
            }
            for packet in padded_packets(&command, world.agent_id, 640) {
                world.apply(NetEvent::Chat(packet));
            }
        }
        assert!(world.chat.is_empty(), "discarded protocol traffic must not accumulate in chat");
        assert!(world.hand_contacts.sources().is_empty());
        assert_eq!(world.hand_contacts.total_retained_bytes(), 0);
        assert!(world.hand_contacts.observers.is_empty());
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: demo::hand_ik::objects(0.0, true),
        });
        let fresh = world.update_hand_contacts(now + Duration::from_secs(2)).unwrap();
        let mut announcement = chat(&fresh);
        announcement.message = "auroraIKAvailable".into();
        world.receive_hand_contacts(&announcement, now + Duration::from_secs(2));
        world.receive_hand_contacts(&chat(&fresh), now + Duration::from_secs(2));
        assert_eq!(world.hand_contacts.graphs().len(), 1, "actual use negotiates fresh data");
    }
}

#[test]
fn expiry_discards_inactive_definitions_partial_transfers_and_reload_snapshots() {
    let now = Instant::now();
    for worn in [false, true] {
        let mut world = fixture(worn);
        let command = world.update_hand_contacts(now).unwrap();
        let mut packet = chat(&command);
        let mut body: serde_json::Value = serde_json::from_str(packet.message.strip_prefix(PREFIX).unwrap()).unwrap();
        body["enabled"] = false.into();
        packet.message = format!("{PREFIX}{body}");
        world.receive_hand_contacts(&packet, now);
        assert!(world.hand_contacts.retained_bytes() > 0);
        world.update_hand_contacts(now + LEASE);
        assert_eq!(world.hand_contacts.total_retained_bytes(), 0, "even inactive data expires");
        world.receive_hand_contacts(&packet, now + LEASE);
        assert_eq!(world.hand_contacts.total_retained_bytes(), 0, "old nonce cannot revive a cache");
    }
    let mut world = fixture(false);
    let (seat, ticket) = world.request_object_ik(demo::action_id(demo::hand_ik::ROOT), now).unwrap();
    let command = world.update_hand_contacts(now).unwrap();
    world.receive_hand_contacts(&chat(&command), now);
    assert!(matches!(world.object_ik_reload(seat, ticket, now), ReloadStatus::Ready(_)));
    world.update_hand_contacts(now + reload::TIMEOUT);
    assert!(matches!(
        world.object_ik_reload(seat, ticket, now + reload::TIMEOUT),
        ReloadStatus::TimedOut
    ));
    // A timed-out partial replacement releases its buffer without another packet.
    let mut world = fixture(false);
    let command = world.update_hand_contacts(now).unwrap();
    world.receive_hand_contacts(&padded_packets(&command, world.agent_id, 640).remove(0), now);
    assert_eq!(world.hand_contacts.retained_bytes(), 320);
    world.update_hand_contacts(now + Duration::from_secs(15));
    assert_eq!(world.hand_contacts.retained_bytes(), 0);
}

#[test]
fn regional_and_per_avatar_budgets_bound_many_live_attachment_publishers() {
    let now = Instant::now();
    let mut world = fixture(false);
    let mut objects = Vec::new();
    let mut sources = Vec::new();
    for person in 0..5u32 {
        let id = if person == 0 {
            world.agent_id
        } else {
            Uuid::from_u128(0x800000 + u128::from(person))
        };
        let mut avatar = demo::action_avatar(false);
        avatar.full_id = id;
        avatar.local_id = 20000 + person;
        objects.push(avatar);
        for item in 0..5u32 {
            let mut root = demo::hand_ik::objects(0.0, false).remove(0);
            root.full_id = Uuid::from_u128(0x900000 + u128::from(person * 5 + item));
            root.local_id = 30000 + person * 5 + item;
            root.parent_id = 20000 + person;
            root.owner_id = id;
            sources.push((
                id,
                Seat {
                    root: root.full_id,
                    region: 1,
                },
            ));
            objects.push(root);
        }
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    world.update_hand_contacts(now);
    for (i, (id, source)) in sources.iter().enumerate() {
        let at = now + Duration::from_secs(4 + i as u64);
        let local = world.agent_id;
        let contacts = if *id == local {
            &mut world.hand_contacts
        } else {
            world.hand_contacts.observers.get_mut(id).unwrap()
        };
        let command = contacts.session_mut(*source).unwrap().update(Some(*source), local, at).unwrap();
        for packet in padded_packets(&command, *id, MAX_BYTES) {
            world.receive_hand_contacts(&packet, at);
            assert!(world.hand_contacts.total_retained_bytes() <= REGION_BYTES);
            for contacts in std::iter::once(&world.hand_contacts).chain(world.hand_contacts.observers.values()) {
                assert!(contacts.retained_bytes() <= AVATAR_BYTES);
            }
        }
    }
    assert_eq!(world.hand_contacts.total_retained_bytes(), REGION_BYTES);
    assert_eq!(world.hand_contacts.retained_bytes(), AVATAR_BYTES);
    assert_eq!(
        world.hand_contacts.observers.values().filter(|c| c.retained_bytes() == 0).count(),
        1
    );
}

#[test]
fn a_definition_cannot_exceed_its_byte_limit_or_the_remaining_session_allowance() {
    let now = Instant::now();
    let mut world = fixture(false);
    let command = world.update_hand_contacts(now).unwrap();
    let packet = chat(&command);
    let header: serde_json::Value = serde_json::from_str(packet.message.strip_prefix(PREFIX).unwrap()).unwrap();
    let nonce: Uuid = serde_json::from_value(header["nonce"].clone()).unwrap();
    for (size, allowance, accepted) in [
        (MAX_BYTES, MAX_BYTES, true),
        (MAX_BYTES + 1, 2 * MAX_BYTES, false),
        (MAX_BYTES, MAX_BYTES - 1, false),
    ] {
        let mut state = graph_wire::State::default();
        for packet in padded_packets(&command, world.agent_id, size) {
            state.receive_bounded(&packet.message, world.agent_id, nonce, now, allowance);
            assert!(state.retained_bytes() <= allowance);
        }
        assert_eq!(state.definition.is_some(), accepted);
        assert_eq!(state.retained_bytes(), if accepted { MAX_BYTES } else { 0 });
    }
}

#[test]
fn ingress_bursts_refill_at_a_fixed_rate_and_repeated_announcements_do_not_bypass_request_spacing() {
    let now = Instant::now();
    let mut ingress = Ingress::default();
    for _ in 0..64 {
        assert!(ingress.allow(now, 8.0, 64.0));
    }
    assert!(!ingress.allow(now, 8.0, 64.0));
    assert!(!ingress.allow(now + Duration::from_millis(124), 8.0, 64.0));
    assert!(ingress.allow(now + Duration::from_millis(125), 8.0, 64.0));
    assert!(!ingress.allow(now + Duration::from_millis(125), 8.0, 64.0));
    let mut world = fixture(false);
    world.update_hand_contacts(now).unwrap();
    let source = own_seat(&world).unwrap();
    // Exhausting a known source is rejected before JSON parsing and cannot
    // spend the remaining global burst on that source's malformed packets.
    for _ in 0..64 {
        assert!(world.hand_contacts.admits(source, now));
    }
    assert!(!world.hand_contacts.admits(source, now));
    for _ in 0..64 {
        assert!(world.hand_contacts.ingress.allow(now, 64.0, 128.0));
    }
    assert!(!world.hand_contacts.ingress.allow(now, 64.0, 128.0));
    let mut world = fixture(false);
    let command = world.update_hand_contacts(now).unwrap();
    let mut announcement = chat(&command);
    announcement.message = "auroraIKAvailable".into();
    world.receive_hand_contacts(&announcement, now);
    assert!(world.update_hand_contacts(now + Duration::from_millis(250)).is_some());
    for tick in 1..=10 {
        let at = now + Duration::from_millis(250 + tick * 250);
        world.receive_hand_contacts(&announcement, at);
        assert!(
            world.update_hand_contacts(at).is_none(),
            "announcements cannot force repeated queries"
        );
    }
}
