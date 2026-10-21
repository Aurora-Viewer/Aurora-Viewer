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
