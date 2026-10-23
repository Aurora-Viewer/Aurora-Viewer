use super::*;
use crate::{
    demo,
    scene::{avatar::AvatarLibrary, ik},
};
use aurora_net::NetEvent;
use std::sync::Arc;

fn fixture(worn: bool) -> World {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = demo::DEMO_AGENT;
    world.main_region = Some(1);
    let mut objects = demo::hand_ik::objects(0.2, !worn);
    if worn {
        objects[0].parent_id = objects[3].local_id;
    }
    world.apply(NetEvent::ObjectUpdates { handle: 1, objects });
    world
}

fn negotiate(world: &mut World, now: Instant) -> NetCommand {
    let command = world.update_hand_contacts(now).unwrap();
    for event in demo::hand_ik::reply(&command) {
        world.apply(event);
    }
    command
}

#[test]
fn simulator_chat_authenticates_seat_and_worn_publishers_when_object_update_omits_the_owner() {
    let now = Instant::now();
    for worn in [false, true] {
        let mut world = fixture(worn);
        let source = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
        world.objects.get_mut(source).unwrap().owner_id = Uuid::nil();
        let command = world.update_hand_contacts(now).unwrap();
        let NetEvent::Chat(chat) = demo::hand_ik::reply(&command).remove(0) else {
            panic!("chat")
        };
        let mut invalid = chat.clone();
        invalid.owner_id = Uuid::nil();
        world.receive_hand_contacts(&invalid, now);
        assert!(world.hand_contacts.graphs().is_empty(), "nil chat metadata is not authenticated");
        if worn {
            invalid.owner_id = Uuid::new_v4();
            world.receive_hand_contacts(&invalid, now);
            assert!(world.hand_contacts.graphs().is_empty(), "the wearer still has to own the publisher");
        }
        world.receive_hand_contacts(&chat, now);
        assert_eq!(
            world.hand_contacts.graphs().len(),
            1,
            "worn={worn}: omitted owner must not discard an authenticated reply"
        );
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert_eq!(ik::targets(&world, avatar, 0.0, now).len(), 2);
        let fresh = world.update_hand_contacts(now + REFRESH + Duration::from_secs(1)).unwrap();
        for event in demo::hand_ik::reply(&fresh) {
            world.apply(event);
        }
        assert_eq!(world.hand_contacts.graphs().len(), 1);
    }
}

#[test]
fn installing_a_script_after_authoring_wakes_backoff_even_if_the_seat_owner_field_is_missing() {
    let now = Instant::now();
    let mut world = fixture(false);
    let source = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
    world.objects.get_mut(source).unwrap().owner_id = Uuid::nil();
    let mut command = None;
    for i in 0..5 {
        command = world.update_hand_contacts(now + RETRY * i);
    }
    assert!(world.update_hand_contacts(now + Duration::from_secs(20)).is_none());
    let NetEvent::Chat(mut announcement) = demo::hand_ik::reply(&command.unwrap()).remove(0) else {
        panic!("chat")
    };
    announcement.chat_type = ChatType::OwnerSay;
    announcement.message = "auroraIKAvailable".into();
    assert!(world.receive_hand_contacts(&announcement, now + Duration::from_secs(20)));
    let command = world
        .update_hand_contacts(now + Duration::from_secs(20))
        .expect("new script must wake silent-source backoff");
    for event in demo::hand_ik::reply(&command) {
        world.apply(event);
    }
    assert_eq!(world.hand_contacts.graphs().len(), 1);
}

#[test]
fn explicit_object_import_requests_fresh_data_without_replacing_the_live_runtime() {
    let now = Instant::now();
    let mut world = fixture(false);
    let old = negotiate(&mut world, now);
    let original = world.hand_contacts.graphs()[0].definition.clone();
    let (seat, ticket) = world
        .request_object_ik(demo::action_id(demo::hand_ik::ROOT), now + Duration::from_millis(100))
        .unwrap();
    assert!(
        world.update_hand_contacts(now + Duration::from_millis(100)).is_none(),
        "shared rate limit"
    );
    let command = world.update_hand_contacts(now + Duration::from_millis(250)).unwrap();
    let NetCommand::ScriptDialogReply { label, .. } = &command else {
        panic!("probe")
    };
    assert_eq!(label.split('|').nth(4), Some(Uuid::nil().to_string().as_str()));
    assert!(
        Arc::ptr_eq(&world.hand_contacts.graphs()[0].definition, &original),
        "keep valid corrections while loading"
    );
    for event in demo::hand_ik::reply(&old) {
        let NetEvent::Chat(chat) = event else { panic!("chat") };
        world.receive_hand_contacts(&chat, now);
    }
    assert!(matches!(world.object_ik_reload(seat, ticket, now), ReloadStatus::Waiting { .. }));
    let NetEvent::Chat(mut chat) = demo::hand_ik::reply(&command).remove(0) else {
        panic!("chat")
    };
    let mut body: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(PREFIX).unwrap()).unwrap();
    let mut heartbeat = body.clone();
    for field in ["part", "count", "data"] {
        heartbeat.as_object_mut().unwrap().remove(field);
    }
    chat.message = format!("{PREFIX}{heartbeat}");
    world.receive_hand_contacts(&chat, now);
    assert!(
        matches!(world.object_ik_reload(seat, ticket, now), ReloadStatus::Waiting { .. }),
        "cached heartbeat is not fresh data"
    );
    body["revision"] = 2.into();
    body["enabled"] = false.into();
    chat.message = format!("{PREFIX}{body}");
    world.receive_hand_contacts(&chat, now);
    let ReloadStatus::Ready(data) = world.object_ik_reload(seat, ticket, now) else {
        panic!("fresh snapshot")
    };
    assert!(!data.enabled);
    assert_eq!(data.runtime.bones.len(), 2, "inactive runtime data remains editable");
    assert_eq!(data.runtime.source, seat);
    assert!(
        Arc::ptr_eq(&world.hand_contacts.graphs()[0].definition, &original),
        "authoring does not change runtime state"
    );
}

#[test]
fn explicit_object_import_times_out_and_cancels_on_source_lifecycle_changes() {
    let now = Instant::now();
    let mut world = fixture(false);
    negotiate(&mut world, now);
    let (seat, ticket) = world.request_object_ik(demo::action_id(demo::hand_ik::ROOT), now).unwrap();
    assert!(matches!(
        world.object_ik_reload(seat, ticket, now + reload::TIMEOUT),
        ReloadStatus::TimedOut
    ));
    assert_eq!(world.hand_contacts.graphs().len(), 1);
    for action in 0..5 {
        let mut world = fixture(false);
        let (seat, ticket) = world.request_object_ik(demo::action_id(demo::hand_ik::ROOT), now).unwrap();
        match action {
            0 => {
                world.apply(NetEvent::ObjectUpdates {
                    handle: 1,
                    objects: vec![demo::action_avatar(false)],
                });
                assert!(matches!(world.object_ik_reload(seat, ticket, now), ReloadStatus::Waiting { .. }));
                continue;
            }
            1 => {
                let i = world.objects.index_of_uuid(&seat.root).unwrap();
                world.objects.remove_idx(i);
            }
            2 => world.teleporting = true,
            3 => world.main_region = Some(2),
            _ => world.cancel_object_ik(),
        }
        assert!(
            matches!(world.object_ik_reload(seat, ticket, now), ReloadStatus::Cancelled),
            "action {action}"
        );
    }
    assert!(
        fixture(true).request_object_ik(demo::action_id(demo::hand_ik::ROOT), now).is_some(),
        "worn objects are explicit import sources too"
    );
}

#[test]
fn unused_objects_and_worn_attachments_answer_only_the_explicit_selected_source_request() {
    for worn in [false, true] {
        let now = Instant::now();
        let mut world = fixture(worn);
        if !worn {
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![demo::action_avatar(false)],
            });
        }
        let (source, ticket) = world.request_object_ik(demo::action_id(demo::hand_ik::LEFT), now).unwrap();
        assert_eq!(source.root, demo::action_id(demo::hand_ik::ROOT));
        let command = world.update_hand_contacts(now).unwrap();
        for event in demo::hand_ik::reply(&command) {
            world.apply(event);
        }
        assert!(matches!(world.object_ik_reload(source, ticket, now), ReloadStatus::Ready(_)));
        assert!(world.hand_contacts.graphs().is_empty());
        world.cancel_object_ik();
        assert!(matches!(world.object_ik_reload(source, ticket, now), ReloadStatus::Cancelled));
        assert_eq!(world.hand_contacts.editor.retained_bytes(), 0);
    }
}

#[test]
fn shared_protocol_negotiates_our_seat_and_owned_worn_sources_without_legacy_fallback() {
    for worn in [false, true] {
        let mut world = fixture(worn);
        let now = Instant::now();
        let command = negotiate(&mut world, now);
        let NetCommand::ScriptDialogReply {
            object_id, channel, label, ..
        } = command
        else {
            panic!("probe")
        };
        assert_eq!(object_id, demo::action_id(demo::hand_ik::ROOT));
        assert_eq!(channel, CHANNEL);
        assert!(label.starts_with("auroraIKObserve|"));
        assert_eq!(world.hand_contacts.graphs().len(), 1);
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert_eq!(ik::targets(&world, avatar, 0.0, now).len(), 2);
        assert!(world.chat.is_empty(), "protocol traffic must not reach nearby chat");
        for prefix in ["auroraIK/1 ", "auroraIK/2 "] {
            let mut packet = demo::hand_ik::reply(&NetCommand::ScriptDialogReply {
                sender: None,
                object_id,
                channel,
                index: 0,
                label: label.clone(),
            })
            .remove(0);
            let NetEvent::Chat(chat) = &mut packet else { panic!("packet") };
            chat.message = format!("{prefix}{{}}");
            assert!(!world.receive_hand_contacts(chat, now), "no obsolete wire parser");
        }
    }
}

#[test]
fn local_sources_release_on_parentage_deletion_region_teleport_and_lease_expiry() {
    let replay = |world: &mut World, command: &NetCommand, at: Instant| {
        for event in demo::hand_ik::reply(command) {
            let NetEvent::Chat(chat) = event else { panic!("contact reply") };
            world.receive_hand_contacts(&chat, at);
        }
    };
    for action in 0..6 {
        let worn = action == 1;
        let mut world = fixture(worn);
        let now = Instant::now();
        let command = world.update_hand_contacts(now).unwrap();
        replay(&mut world, &command, now);
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert_eq!(ik::targets(&world, avatar, 0.0, now).len(), 2, "active before action {action}");
        let ended = if action == 5 { now + LEASE + Duration::from_secs(1) } else { now };
        match action {
            0 => world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![demo::action_avatar(false)],
            }),
            1 => world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![demo::hand_ik::objects(0.0, false).remove(0)],
            }),
            2 => {
                let i = world.objects.index_of_uuid(&demo::action_id(demo::hand_ik::ROOT)).unwrap();
                world.objects.remove_idx(i);
                None
            }
            3 => {
                world.teleporting = true;
                None
            }
            4 => {
                world.main_region = Some(2);
                None
            }
            _ => {
                world.update_hand_contacts(ended);
                None
            }
        };
        assert!(ik::targets(&world, avatar, 0.0, ended).is_empty(), "action {action}");
        let later = ended + Duration::from_secs(2);
        world.update_hand_contacts(later);
        replay(&mut world, &command, later);
        assert!(
            ik::targets(&world, avatar, 0.0, later).is_empty(),
            "old reply after action {action}"
        );
    }
}

#[test]
fn shared_protocol_retries_silent_publishers_and_renews_with_a_new_nonce() {
    let now = Instant::now();
    let mut world = fixture(false);
    for i in 0..5 {
        assert!(world.update_hand_contacts(now + RETRY * i).is_some());
    }
    assert!(world.update_hand_contacts(now + RETRY * 5).is_none());
    assert!(world.update_hand_contacts(now + Duration::from_secs(73)).is_some());
    let mut world = fixture(false);
    let old = negotiate(&mut world, now);
    let fresh = world.update_hand_contacts(now + REFRESH + Duration::from_secs(1)).unwrap();
    let clock = world.hand_contacts.graphs()[0].anchor;
    for event in demo::hand_ik::reply(&old) {
        world.apply(event);
    }
    assert_eq!(world.hand_contacts.graphs()[0].anchor, clock);
    for event in demo::hand_ik::reply(&fresh) {
        world.apply(event);
    }
    assert_eq!(world.hand_contacts.graphs().len(), 1);
}

#[test]
fn seat_authorization_allows_a_different_builder_but_worn_sources_require_the_wearer() {
    for worn in [false, true] {
        let mut world = fixture(worn);
        let mut object = demo::hand_ik::objects(0.0, !worn).remove(0);
        if worn {
            object.parent_id = demo::action_avatar(false).local_id;
        }
        object.owner_id = Uuid::new_v4();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![object],
        });
        assert_eq!(world.update_hand_contacts(Instant::now()).is_some(), !worn);
    }
}

#[test]
fn malformed_partial_and_forged_packets_never_replace_a_valid_rig_or_extend_its_lease() {
    let now = Instant::now();
    let mut world = fixture(false);
    let command = negotiate(&mut world, now);
    let old = world.hand_contacts.graphs()[0].definition.clone();
    let anchor = world.hand_contacts.graphs()[0].anchor;
    for action in 0..5 {
        let NetEvent::Chat(mut chat) = demo::hand_ik::reply(&command).remove(0) else {
            panic!("packet")
        };
        if action == 0 {
            chat.owner_id = Uuid::new_v4();
        } else if action == 1 {
            chat.source_id = demo::action_id(demo::hand_ik::LEFT);
        } else if action == 2 {
            chat.source_type = ChatSourceType::Agent;
        } else if action == 3 {
            chat.message = format!("{PREFIX}{}", "x".repeat(1024));
        } else {
            let mut body: serde_json::Value = serde_json::from_str(chat.message.strip_prefix(PREFIX).unwrap()).unwrap();
            body["revision"] = 2.into();
            body["count"] = 2.into();
            chat.message = format!("{PREFIX}{body}");
        }
        world.receive_hand_contacts(&chat, now + Duration::from_secs(10));
        assert!(Arc::ptr_eq(&world.hand_contacts.graphs()[0].definition, &old));
        assert_eq!(world.hand_contacts.graphs()[0].anchor, anchor);
    }
}
