use super::*;

fn chat(text: &str) -> Step {
    Step::Chat {
        text: text.into(),
        flags: 0,
    }
}

#[test]
fn simultaneous_steps_and_explicit_timer() {
    let mut manager = Manager::default();
    let now = Instant::now();
    let id = Uuid::from_u128(1);
    manager.play(
        id,
        Gesture {
            steps: vec![
                chat("first"),
                chat("second"),
                Step::Wait {
                    seconds: 2.0,
                    flags: WAIT_TIME,
                },
                chat("last"),
            ],
            ..Default::default()
        },
        None,
    );
    assert_eq!(
        manager.tick(now, &BTreeSet::new()),
        [Effect::Chat("first".into()), Effect::Chat("second".into())]
    );
    assert!(manager.tick(now + Duration::from_secs(1), &BTreeSet::new()).is_empty());
    assert_eq!(
        manager.tick(now + Duration::from_secs(2), &BTreeSet::new()),
        [Effect::Chat("last".into())]
    );
    assert!(!manager.is_playing(id));
}

#[test]
fn preload_blocks_every_step_until_assets_settle() {
    let mut m = Manager::default();
    let id = Uuid::from_u128(1);
    let now = Instant::now();
    m.play(
        id,
        Gesture {
            steps: vec![
                chat("first"),
                Step::Sound {
                    name: String::new(),
                    asset: Uuid::from_u128(2),
                    flags: 0,
                },
            ],
            ..Default::default()
        },
        None,
    );
    assert!(m.tick_ready(now, &BTreeSet::new(), |s| !matches!(s, Step::Sound { .. })).is_empty());
    assert_eq!(
        m.tick_ready(now, &BTreeSet::new(), |_| true),
        [Effect::Chat("first".into()), Effect::Sound(Uuid::from_u128(2))]
    );
}

#[test]
fn animation_wait_needs_server_start_then_stop_and_stop_owns_only_its_animations() {
    let mut m = Manager::default();
    let now = Instant::now();
    let id = Uuid::from_u128(1);
    let anim = Uuid::from_u128(2);
    let unrelated = Uuid::from_u128(3);
    let g = Gesture {
        steps: vec![
            Step::Animation {
                name: "test".into(),
                asset: anim,
                flags: 0,
            },
            Step::Wait {
                seconds: 0.0,
                flags: WAIT_ANIMATIONS,
            },
            chat("done"),
        ],
        ..Default::default()
    };
    m.play(id, g.clone(), None);
    assert_eq!(m.tick(now, &BTreeSet::from([unrelated])), [Effect::Animation(anim, true)]);
    assert!(m.tick(now + Duration::from_secs(1), &BTreeSet::from([unrelated])).is_empty());
    assert!(m.tick(now + Duration::from_secs(2), &BTreeSet::from([anim, unrelated])).is_empty());
    assert_eq!(
        m.tick(now + Duration::from_secs(3), &BTreeSet::from([unrelated])),
        [Effect::Chat("done".into())]
    );
    m.play(id, g, None);
    m.tick(now, &BTreeSet::from([unrelated]));
    assert_eq!(m.stop(id), [Effect::Animation(anim, false)]);
}

#[test]
fn animation_wait_timeout_and_key_release() {
    let mut m = Manager::default();
    let now = Instant::now();
    let id = Uuid::from_u128(1);
    m.play(
        id,
        Gesture {
            steps: vec![
                Step::Animation {
                    name: String::new(),
                    asset: Uuid::from_u128(3),
                    flags: 0,
                },
                Step::Wait {
                    seconds: 0.0,
                    flags: WAIT_ANIMATIONS,
                },
                chat("timeout"),
            ],
            ..Default::default()
        },
        None,
    );
    m.tick(now, &BTreeSet::new());
    assert_eq!(
        m.tick(now + Duration::from_secs(30), &BTreeSet::new()),
        [Effect::Chat("timeout".into())]
    );
    m.stop(id);
    m.play(
        id,
        Gesture {
            steps: vec![
                Step::Wait {
                    seconds: 5.0,
                    flags: WAIT_RELEASE | WAIT_TIME,
                },
                chat("release"),
            ],
            ..Default::default()
        },
        Some(65),
    );
    assert!(m.tick(now, &BTreeSet::new()).is_empty());
    m.release(Some(66));
    assert!(m.tick(now, &BTreeSet::new()).is_empty());
    m.release(Some(65));
    assert_eq!(m.tick(now, &BTreeSet::new()), [Effect::Chat("release".into())]);
}

#[test]
fn first_word_trigger_replacement_case_and_global_disable() {
    let mut m = Manager::default();
    let id = Uuid::from_u128(1);
    m.active.insert(id);
    m.entries.insert(
        id,
        Entry {
            asset: Uuid::from_u128(2),
            definition: Gesture {
                trigger: "/hello".into(),
                replacement: "Hello".into(),
                key: 0xa8,
                mask: 4,
                ..Default::default()
            },
        },
    );
    assert_eq!(m.revise("one  /HELLO /hello"), ("one Hello /hello".into(), Some(id)));
    assert_eq!(m.revise("/hello!"), ("/hello!".into(), None));
    assert_eq!(m.key_trigger(0xa8, 4), Some(id));
    assert_eq!(m.key_trigger(0xa8, 0), None);
    m.enabled = false;
    assert_eq!(m.revise("one  /hello"), ("one  /hello".into(), None));
    assert_eq!(m.key_trigger(0xa8, 4), None);
}

#[test]
fn firestorm_key_codes_and_ctrl_g_default() {
    use winit::keyboard::{Key, NamedKey};
    assert_eq!(keys::from_event(&Key::Character("g".into())), Some(b'G'));
    assert_eq!(keys::from_event(&Key::Named(NamedKey::F8)), Some(0xa8));
    assert_eq!(keys::from_event(&Key::Character("é".into())), None);
    assert_eq!(
        keys::from_key(winit::keyboard::KeyCode::Digit4, &Key::Character("'".into())),
        Some(b'4')
    );
    assert_eq!(
        keys::from_key(winit::keyboard::KeyCode::KeyW, &Key::Character("z".into())),
        Some(b'Z')
    );
    assert_eq!(
        keys::from_key(winit::keyboard::KeyCode::NumpadAdd, &Key::Character("+".into())),
        Some(0x95)
    );
    let binding = crate::keybinds::KeyBindings::default();
    assert_eq!(
        binding.triggered(
            &crate::keybinds::Input::Key("KeyG".into()),
            crate::keybinds::Mods {
                ctrl: true,
                ..Default::default()
            }
        ),
        Some(crate::keybinds::Action::Gestures)
    );
}
