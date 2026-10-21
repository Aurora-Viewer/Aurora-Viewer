use super::*;
use crate::demo::anti_spam as demo;
use crate::scene::avatar::AvatarLibrary;
use aurora_net::NetEvent;
use std::sync::Arc;

const ME: Uuid = Uuid::from_u128(0x678);

fn registry() -> Registry {
    let mut r = Registry::default();
    r.configure(Settings {
        enabled: true,
        ..Default::default()
    });
    r
}

fn world() -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.anti_spam = registry();
    w
}

#[test]
fn anti_spam_defaults_round_trip_and_old_settings_are_opt_in() {
    let old: crate::settings::Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(old.anti_spam, Settings::default());
    let r = registry();
    let saved = serde_json::to_string(&r.settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&saved).unwrap(), r.settings);
    assert!(!Settings::default().enabled);
    let previous: Settings = serde_json::from_str(r#"{"enabled":true,"amount":2}"#).unwrap();
    assert_eq!(previous.newlines, 70);
}

#[test]
fn anti_spam_interval_boundary_and_session_block() {
    let mut r = registry();
    let t = Instant::now();
    for _ in 0..10 {
        assert_eq!(r.check(Queue::Im, ME, t), Decision::Allow);
    }
    assert_eq!(r.check(Queue::Im, ME, t + Duration::from_secs(4)), Decision::NewBlock);
    assert_eq!(r.check(Queue::Im, ME, t + Duration::from_secs(600)), Decision::Block);
    r.unblock(Queue::Im, ME);
    for _ in 0..10 {
        assert_eq!(r.check(Queue::Im, ME, t), Decision::Allow);
    }
    assert_eq!(r.check(Queue::Im, ME, t + Duration::from_millis(4001)), Decision::Allow);
    assert_eq!(r.check(Queue::ScriptDialog, ME, t), Decision::Allow);
    assert_eq!(r.check(Queue::Im, demo::INVENTORY_SOURCE, t), Decision::Allow);
}

#[test]
fn anti_spam_global_queue_and_sound_multipliers() {
    let mut r = registry();
    let t = Instant::now();
    for _ in 0..60 {
        assert_eq!(r.check(Queue::Sound, ME, t), Decision::Allow);
        assert_eq!(r.check(Queue::SoundPreload, ME, t), Decision::Allow);
    }
    assert_eq!(r.check(Queue::Sound, ME, t), Decision::NewBlock);
    assert!(!r.is_blocked(Queue::SoundPreload, ME));
    let global = Settings {
        global: true,
        ..r.settings
    };
    r.configure(global);
    assert!(r.blocks().is_empty());
    for _ in 0..10 {
        assert_eq!(r.check(Queue::ScriptDialog, ME, t), Decision::Allow);
    }
    assert_eq!(r.check(Queue::Im, ME, t), Decision::NewBlock);
    assert!(r.is_blocked(Queue::Sound, ME));
    r.reset();
    assert!(!r.is_blocked(Queue::Sound, ME));
}

#[test]
fn anti_spam_bounds_rotating_sources_and_reclaims_inactive_counters() {
    let mut r = registry();
    let t = Instant::now();
    for n in 1..=MAX_SOURCES + 100 {
        r.check(Queue::Im, Uuid::from_u128(n as u128), t);
    }
    assert_eq!(r.entries.len(), MAX_SOURCES);
    assert_eq!(r.overflow.len(), 1);
    assert_eq!(r.check(Queue::Im, Uuid::from_u128(100_000), t), Decision::Block);
    assert_eq!(r.check(Queue::Im, ME, t), Decision::Allow);
    assert_eq!(
        r.check(Queue::Im, Uuid::from_u128(100_000), t + Duration::from_secs(5)),
        Decision::Allow
    );
    assert!(r.entries.len() < MAX_SOURCES);
    r.configure(Settings::default());
    for _ in 0..100 {
        assert_eq!(r.check(Queue::Im, ME, t), Decision::Allow);
    }
    assert!(r.entries.is_empty());
}

#[test]
fn anti_spam_stops_dialogs_before_attention_and_keeps_other_sources_usable() {
    let mut w = world();
    for _ in 0..15 {
        w.apply(demo::dialog(demo::DIALOG_SOURCE));
    }
    assert_eq!(w.notifications.list.len(), 10);
    assert_eq!(w.ui_sounds.len(), 10);
    let messages = w.chat.len();
    w.apply(demo::dialog(demo::DIALOG_SOURCE));
    assert_eq!(w.chat.len(), messages, "a blocked event must not create another warning");
    assert!(!w.mutes.is_muted_id(&demo::DIALOG_SOURCE));
    w.apply(demo::dialog(demo::SOUND_SOURCE));
    assert_eq!(w.notifications.list.len(), 11);
    w.anti_spam.unblock(Queue::ScriptDialog, demo::DIALOG_SOURCE);
    w.apply(demo::dialog(demo::DIALOG_SOURCE));
    assert_eq!(w.notifications.list.len(), 12);
}

#[test]
fn anti_spam_inventory_and_teleport_offers_share_the_legacy_im_queue() {
    use crate::world::notifications::im;
    for dialog in [
        im::INVENTORY_OFFERED,
        im::TASK_INVENTORY_OFFERED,
        im::LURE_USER,
        im::TELEPORT_REQUEST,
    ] {
        let mut w = world();
        for n in 0..15 {
            w.apply(demo::offer(ME, demo::INVENTORY_SOURCE, dialog, n));
        }
        assert_eq!(w.notifications.list.len(), 10, "dialog {dialog}");
        assert_eq!(w.ui_sounds.len(), 10, "no sound for suppressed offer {dialog}");
        assert!(w.anti_spam.is_blocked(Queue::Im, demo::INVENTORY_SOURCE));
        w.apply(demo::offer(ME, demo::INVENTORY_SOURCE, im::LURE_USER, 100));
        assert_eq!(w.notifications.list.len(), 10);
    }
}

#[test]
fn anti_spam_typing_saved_text_nil_and_self_are_exempt() {
    let mut w = world();
    for dialog in [41, 42, 0, 19] {
        for n in 0..30 {
            let NetEvent::InstantMessage(mut im) = demo::offer(ME, demo::INVENTORY_SOURCE, dialog, n) else {
                unreachable!()
            };
            im.offline = true;
            assert!(!w.anti_spam_im(&im));
        }
    }
    for _ in 0..30 {
        assert!(!w.anti_spam_check(Queue::ScriptDialog, ME, true));
        assert!(!w.anti_spam_check(Queue::ScriptDialog, Uuid::nil(), true));
    }
    assert!(w.anti_spam.entries.is_empty());
}

#[test]
fn anti_spam_audio_sources_preloads_collisions_and_stop_commands() {
    let mut w = world();
    for _ in 0..60 {
        assert!(w.apply(demo::sound()).is_some());
    }
    assert!(w.apply(demo::sound()).is_none());
    assert!(w.sound_blocked(&demo::SOUND_SOURCE, &demo::SOUND_OWNER));
    // A non-null sound still plays in the audio engine even with FLAG_STOP;
    // an attacker must not use that flag to bypass the receive filter.
    assert!(
        w.apply(NetEvent::AttachedSound {
            object: demo::SOUND_SOURCE,
            owner: demo::SOUND_OWNER,
            sound: Uuid::from_u128(1),
            gain: 1.0,
            flags: 32,
        })
        .is_none()
    );
    // A nil sound clears an attached loop, including one already blocked.
    assert!(
        w.apply(NetEvent::AttachedSound {
            object: demo::SOUND_SOURCE,
            owner: demo::SOUND_OWNER,
            sound: Uuid::nil(),
            gain: 0.0,
            flags: 32,
        })
        .is_some()
    );
    let mut w = world();
    for n in 1..=60 {
        assert!(
            w.apply(NetEvent::PreloadSounds(vec![demo::preload(Uuid::from_u128(n), demo::SOUND_OWNER)]))
                .is_some()
        );
    }
    // Rotating object IDs cannot bypass the owner's sound preload limit;
    // another owner's entry in the same packet must still reach the app.
    let clean = demo::preload(demo::TELEPORT_SOURCE, demo::INVENTORY_SOURCE);
    let out = w.apply(NetEvent::PreloadSounds(vec![
        demo::preload(demo::SOUND_SOURCE, demo::SOUND_OWNER),
        clean,
    ]));
    let Some(NetEvent::PreloadSounds(kept)) = out else {
        panic!("clean preload must survive")
    };
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].owner, clean.owner);
    for _ in 0..100 {
        let mut collision = demo::sound();
        if let NetEvent::SoundTrigger { sound, .. } = &mut collision {
            *sound = Uuid::from_u128(crate::scene::sounds::COLLISION[0]);
        }
        assert!(w.apply(collision).is_some());
    }
}

#[test]
fn anti_spam_owned_objects_are_exempt_unless_requested() {
    let mut w = world();
    let (handle, mut object) = crate::demo::events()
        .into_iter()
        .find_map(|event| match event {
            NetEvent::ObjectUpdates { handle, mut objects } => objects.pop().map(|o| (handle, o)),
            _ => None,
        })
        .unwrap();
    object.owner_id = ME;
    object.full_id = demo::DIALOG_SOURCE;
    w.objects.upsert(handle, object);
    for _ in 0..15 {
        w.apply(demo::dialog(demo::DIALOG_SOURCE));
    }
    assert!(w.anti_spam.entries.is_empty());
    w.anti_spam.configure(Settings {
        own_objects: true,
        ..w.anti_spam.settings
    });
    for _ in 0..11 {
        w.apply(demo::dialog(demo::DIALOG_SOURCE));
    }
    assert!(w.anti_spam.is_blocked(Queue::ScriptDialog, demo::DIALOG_SOURCE));
}

#[test]
fn anti_spam_script_gifts_use_the_owner_id_including_our_own_when_enabled() {
    use crate::world::notifications::im;
    for (enabled, own_objects, giver, expected) in [
        (true, true, ME, 2),
        (true, false, ME, 4),
        (false, true, ME, 4),
        (true, false, demo::INVENTORY_SOURCE, 2),
    ] {
        let mut w = world();
        w.anti_spam.configure(Settings {
            enabled,
            own_objects,
            amount: 2,
            period: 30,
            ..Default::default()
        });
        for n in 0..4 {
            // llGiveInventory identifies the owner in AgentData.AgentID;
            // MessageBlock.ID is a new transaction, never the source object.
            w.apply(demo::offer(ME, giver, im::TASK_INVENTORY_OFFERED, n));
        }
        assert_eq!(
            w.notifications.list.len(),
            expected,
            "enabled={enabled}, own={own_objects}, giver={giver}"
        );
        assert_eq!(w.ui_sounds.len(), expected);
        assert_eq!(w.anti_spam.is_blocked(Queue::Im, giver), expected == 2);
        if expected == 2 {
            assert_eq!(w.chat.len(), 1, "one block notice, no offer cards or spam notices");
            w.anti_spam.unblock(Queue::Im, giver);
            w.apply(demo::offer(ME, giver, im::TASK_INVENTORY_OFFERED, 100));
            assert_eq!(w.notifications.list.len(), 3);
        }
        // Self text is still exempt even when own scripted gifts are counted.
        let NetEvent::InstantMessage(im) = demo::offer(ME, ME, 0, 200) else {
            unreachable!()
        };
        assert!(!w.anti_spam_im(&im));
    }
}

#[test]
fn anti_spam_newline_boundary_counts_lf_and_blocks_until_reset() {
    let mut r = registry();
    assert_eq!(r.check_newlines(ME, &"\n".repeat(70)), Decision::Allow);
    assert_eq!(r.check_newlines(ME, &"\r\n".repeat(70)), Decision::Allow);
    assert_eq!(r.check_newlines(ME, &"long wrapped text ".repeat(1000)), Decision::Allow);
    assert_eq!(r.check_newlines(ME, &"\n".repeat(71)), Decision::NewBlock);
    assert_eq!(r.check_newlines(ME, "short"), Decision::Block);
    assert_eq!(r.check(Queue::Im, ME, Instant::now() + Duration::from_secs(600)), Decision::Block);
    r.unblock(Queue::Im, ME);
    assert_eq!(r.check_newlines(ME, "short"), Decision::Allow);
    r.configure(Settings { newlines: 5, ..r.settings });
    assert_eq!(r.check_newlines(ME, &"\n".repeat(6)), Decision::NewBlock);
    r.reset();
    assert_eq!(r.check_newlines(ME, "short"), Decision::Allow);
    r.configure(Settings::default());
    assert_eq!(r.check_newlines(ME, &"\n".repeat(1001)), Decision::Allow);
}

fn chat(source: Uuid, owner: Uuid, source_type: ChatSourceType, text: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: "Anti-spam test".into(),
        source_id: source,
        owner_id: owner,
        source_type,
        chat_type: ChatType::Normal,
        position: glam::Vec3::ZERO,
        message: text.into(),
    })
}

#[test]
fn anti_spam_chat_newlines_filter_before_history_attention_and_owner_im() {
    let mut w = world();
    w.anti_spam.configure(Settings {
        newlines: 5,
        ..w.anti_spam.settings
    });
    let owner = demo::SOUND_OWNER;
    let long = "\n".repeat(6);
    w.apply(chat(demo::DIALOG_SOURCE, owner, ChatSourceType::Object, &long));
    assert_eq!(w.chat.len(), 1, "only the block notice enters history");
    assert!(!w.chat.iter().any(|c| c.text == long));
    assert!(w.ui_sounds.is_empty());
    assert!(w.anti_spam.is_blocked(Queue::Im, owner));
    w.apply(chat(
        demo::INVENTORY_SOURCE,
        owner,
        ChatSourceType::Object,
        "another object, same owner",
    ));
    w.apply(demo::offer(ME, owner, 0, 0));
    assert_eq!(w.chat.len(), 1, "owner's later chat and IM also blocked");
    w.anti_spam.unblock(Queue::Im, owner);
    w.apply(chat(demo::DIALOG_SOURCE, owner, ChatSourceType::Object, "short"));
    assert_eq!(w.chat.back().unwrap().text, "short");
    assert!(!w.mutes.is_muted_id(&owner), "no permanent mute");
}

#[test]
fn anti_spam_im_newlines_include_saved_messages_and_keep_legacy_exemptions() {
    for offline in [false, true] {
        let mut w = world();
        let NetEvent::InstantMessage(mut im) = demo::offer(ME, demo::INVENTORY_SOURCE, 0, 0) else {
            unreachable!()
        };
        im.message = "\n".repeat(71);
        im.offline = offline;
        w.apply(NetEvent::InstantMessage(im.clone()));
        assert_eq!(w.chat.len(), 1);
        assert!(w.ui_sounds.is_empty());
        assert!(w.social.ims.is_empty());
        assert!(w.anti_spam.is_blocked(Queue::Im, im.from_agent_id));
        im.message = "short".into();
        w.apply(NetEvent::InstantMessage(im));
        assert_eq!(w.chat.len(), 1);
    }
    for (dialog, source) in [(3, demo::INVENTORY_SOURCE), (0, ME), (0, Uuid::nil()), (41, demo::INVENTORY_SOURCE)] {
        let mut w = world();
        let NetEvent::InstantMessage(mut im) = demo::offer(ME, source, dialog, 0) else {
            unreachable!()
        };
        im.message = "\n".repeat(71);
        assert!(!w.anti_spam_im(&im));
        assert!(w.anti_spam.blocks().is_empty());
    }
}

#[test]
fn anti_spam_chat_own_objects_rate_category_and_global_mode() {
    let mut w = world();
    let (handle, mut object) = crate::demo::events()
        .into_iter()
        .find_map(|e| match e {
            NetEvent::ObjectUpdates { handle, mut objects } => objects.pop().map(|o| (handle, o)),
            _ => None,
        })
        .unwrap();
    object.owner_id = ME;
    object.full_id = demo::DIALOG_SOURCE;
    w.objects.upsert(handle, object);
    w.apply(chat(demo::DIALOG_SOURCE, ME, ChatSourceType::Object, &"\n".repeat(71)));
    assert!(w.anti_spam.blocks().is_empty());
    w.anti_spam.configure(Settings {
        own_objects: true,
        ..w.anti_spam.settings
    });
    w.apply(chat(demo::DIALOG_SOURCE, ME, ChatSourceType::Object, &"\n".repeat(71)));
    assert!(w.anti_spam.is_blocked(Queue::Im, demo::DIALOG_SOURCE));
    assert!(!w.anti_spam.is_blocked(Queue::Im, ME));
    w.anti_spam.configure(Settings {
        amount: 2,
        ..w.anti_spam.settings
    });
    for _ in 0..3 {
        w.apply(chat(demo::DIALOG_SOURCE, ME, ChatSourceType::Object, "short"));
    }
    assert!(w.anti_spam.is_blocked(Queue::Chat, demo::DIALOG_SOURCE));
    assert!(!w.anti_spam.is_blocked(Queue::ScriptDialog, demo::DIALOG_SOURCE));
    w.apply(demo::dialog(demo::DIALOG_SOURCE));
    assert_eq!(w.notifications.list.len(), 1);
    w.anti_spam.configure(Settings {
        global: true,
        ..w.anti_spam.settings
    });
    w.apply(chat(demo::DIALOG_SOURCE, ME, ChatSourceType::Object, "short"));
    w.apply(demo::dialog(demo::DIALOG_SOURCE));
    w.apply(demo::dialog(demo::DIALOG_SOURCE));
    assert!(w.anti_spam.is_blocked(Queue::ScriptDialog, demo::DIALOG_SOURCE));
    assert_eq!(w.notifications.list.len(), 2);
}

#[test]
fn anti_spam_newline_floods_share_the_bounded_source_table() {
    let mut r = registry();
    let long = "\n".repeat(71);
    for n in 1..=MAX_SOURCES + 100 {
        r.check_newlines(Uuid::from_u128(n as u128), &long);
    }
    assert_eq!(r.entries.len(), MAX_SOURCES);
    assert_eq!(r.overflow.len(), 1);
    assert!(r.is_blocked(Queue::Im, Uuid::from_u128(1)));
}
