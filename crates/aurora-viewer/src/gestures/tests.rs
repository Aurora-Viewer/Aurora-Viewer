use super::*;

fn chat(text: &str) -> Step {
    Step::Chat {
        text: text.into(),
        flags: 0,
    }
}

#[test]
fn completion_uses_active_triggers_exact_matches_and_common_suffixes() {
    let mut manager = Manager::default();
    for (id, trigger, active) in [
        (1, "/eject", true),
        (2, "/ejection", true),
        (3, "/enjoy", false),
        (4, "/éject", true),
    ] {
        let id = Uuid::from_u128(id);
        manager.entries.insert(
            id,
            Entry {
                asset: Uuid::nil(),
                definition: Gesture {
                    trigger: trigger.into(),
                    ..Default::default()
                },
            },
        );
        if active {
            manager.active.insert(id);
        }
    }
    manager.active.insert(Uuid::from_u128(99)); // Still loading.
    assert_eq!(manager.complete("/ej").as_deref(), Some("/eject"));
    assert_eq!(manager.complete("/EJ").as_deref(), Some("/EJect"));
    assert_eq!(manager.complete("/eject").as_deref(), Some("/eject"));
    assert_eq!(manager.complete("/Éj").as_deref(), Some("/Éject"));
    assert_eq!(manager.complete("/en"), None);
    assert_eq!(manager.complete("/missing"), None);
    assert_eq!(manager.complete(""), None);
    manager.active.insert(Uuid::from_u128(3));
    assert_eq!(manager.complete("/e").as_deref(), Some("/e"));
    assert_eq!(manager.complete("/ej").as_deref(), Some("/eject"));
    manager.enabled = false;
    assert_eq!(manager.complete("/ej"), None);
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

/// A hotkey pressed again and again plays its gesture at each press, as in
/// the Legacy Viewer (LLGestureMgr::triggerGesture): a press while it still
/// plays starts it over, the animation asked again with no stop before it;
/// only a step waiting for the key's release holds the key back.
#[test]
fn a_repeated_hotkey_plays_the_gesture_at_each_press() {
    let mut m = Manager::default();
    let now = Instant::now();
    let id = Uuid::from_u128(1);
    let anim = Uuid::from_u128(3);
    let dance = Gesture {
        key: 0xa2,
        steps: vec![
            Step::Animation {
                name: String::new(),
                asset: anim,
                flags: 0,
            },
            Step::Wait {
                seconds: 3.0,
                flags: WAIT_TIME,
            },
            chat("ta-da"),
        ],
        ..Default::default()
    };
    m.entries.insert(
        id,
        Entry {
            asset: Uuid::nil(),
            definition: dance.clone(),
        },
    );
    m.active.insert(id);
    // first press: the animation starts, the gesture waits its 3 s
    assert_eq!(m.key_trigger(0xa2, 0), Some(id));
    m.play(id, dance.clone(), Some(0xa2));
    assert_eq!(m.tick(now, &BTreeSet::new()), [Effect::Animation(anim, true)]);
    // pressed again while it plays (its release not seen yet): played again
    assert_eq!(m.key_trigger(0xa2, 0), Some(id), "the press was held back while the gesture played");
    m.play(id, dance.clone(), Some(0xa2));
    assert_eq!(
        m.tick(now + Duration::from_millis(100), &BTreeSet::from([anim])),
        [Effect::Animation(anim, true)],
        "a restart asks the animation again, without stopping it first"
    );
    // a step that waits for the key's release holds the key until then
    let hold = Gesture {
        key: 0xa2,
        steps: vec![
            Step::Wait {
                seconds: 5.0,
                flags: WAIT_RELEASE | WAIT_TIME,
            },
            chat("released"),
        ],
        ..Default::default()
    };
    m.entries.insert(
        id,
        Entry {
            asset: Uuid::nil(),
            definition: hold.clone(),
        },
    );
    m.play(id, hold, Some(0xa2));
    assert!(m.tick(now, &BTreeSet::new()).is_empty());
    assert_eq!(m.key_trigger(0xa2, 0), None);
    m.release(Some(0xa2));
    assert_eq!(m.tick(now, &BTreeSet::new()), [Effect::Chat("released".into())]);
    assert_eq!(m.key_trigger(0xa2, 0), Some(id));
    // a held key repeats into gestures, except F1 to F12 (LLViewerInput::handleKey)
    assert!(Manager::repeat_triggers(b'G'));
    assert!(!Manager::repeat_triggers(0xa2));
    assert!(!Manager::repeat_triggers(0xac));
}

/// A clap (a sound, short waits, the sound again) pressed again before its
/// end starts over at once and keeps its rhythm from the new press (#915).
/// The app steps playing gestures at every frame, as LLAppViewer::idle
/// does: stepped with a half-second poll, its 0.3 s waits lasted 0.5 s.
#[test]
fn a_clap_pressed_again_starts_over_on_its_own_rhythm() {
    let mut m = Manager::default();
    let now = Instant::now();
    let id = Uuid::from_u128(1);
    let sound = Uuid::from_u128(2);
    let clap = || Step::Sound {
        name: String::new(),
        asset: sound,
        flags: 0,
    };
    let wait = || Step::Wait {
        seconds: 0.3,
        flags: WAIT_TIME,
    };
    let g = Gesture {
        key: 0xa7,
        steps: vec![clap(), wait(), clap(), wait(), clap()],
        ..Default::default()
    };
    let at = |ms| now + Duration::from_millis(ms);
    m.play(id, g.clone(), Some(0xa7));
    assert_eq!(m.tick(now, &BTreeSet::new()), [Effect::Sound(sound)]);
    assert!(m.tick(at(250), &BTreeSet::new()).is_empty());
    m.play(id, g, Some(0xa7));
    assert_eq!(m.tick(at(250), &BTreeSet::new()), [Effect::Sound(sound)], "the press plays at once");
    assert!(m.tick(at(400), &BTreeSet::new()).is_empty(), "the first wait starts over too");
    assert_eq!(m.tick(at(560), &BTreeSet::new()), [Effect::Sound(sound)]);
    assert_eq!(m.tick(at(870), &BTreeSet::new()), [Effect::Sound(sound)]);
    assert!(!m.is_playing(id));
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
fn numeric_gesture_shortcuts_accept_both_keyboard_rows() {
    use winit::keyboard::{Key, KeyCode, NamedKey, NativeKey};

    // winit removes NumLock in key_without_modifiers, so keypad digits can
    // arrive as navigation keys or as an unidentified key (keypad 5).
    for (digit, row, pad, unmodified) in [
        (b'0', KeyCode::Digit0, KeyCode::Numpad0, Key::Named(NamedKey::Insert)),
        (b'1', KeyCode::Digit1, KeyCode::Numpad1, Key::Named(NamedKey::End)),
        (b'2', KeyCode::Digit2, KeyCode::Numpad2, Key::Named(NamedKey::ArrowDown)),
        (b'3', KeyCode::Digit3, KeyCode::Numpad3, Key::Named(NamedKey::PageDown)),
        (b'4', KeyCode::Digit4, KeyCode::Numpad4, Key::Named(NamedKey::ArrowLeft)),
        (b'5', KeyCode::Digit5, KeyCode::Numpad5, Key::Unidentified(NativeKey::Windows(0x0c))),
        (b'6', KeyCode::Digit6, KeyCode::Numpad6, Key::Named(NamedKey::ArrowRight)),
        (b'7', KeyCode::Digit7, KeyCode::Numpad7, Key::Named(NamedKey::Home)),
        (b'8', KeyCode::Digit8, KeyCode::Numpad8, Key::Named(NamedKey::ArrowUp)),
        (b'9', KeyCode::Digit9, KeyCode::Numpad9, Key::Named(NamedKey::PageUp)),
    ] {
        let id = Uuid::from_u128(u128::from(digit));
        let mut manager = Manager::default();
        manager.active.insert(id);
        for mask in [0, 1, 4, 5] {
            let definition = Gesture {
                key: digit,
                mask,
                steps: vec![
                    Step::Wait {
                        seconds: 0.0,
                        flags: WAIT_RELEASE,
                    },
                    chat("released"),
                ],
                ..Default::default()
            };
            manager.entries.insert(
                id,
                Entry {
                    asset: Uuid::nil(),
                    definition,
                },
            );
            for (code, key) in [
                (row, &unmodified),
                (pad, &unmodified),
                (pad, &Key::Character(char::from(digit).to_string().into())),
            ] {
                let translated = keys::from_key(code, key);
                assert_eq!(translated, Some(digit), "{code:?} with {key:?}");
                let translated = translated.expect("numeric shortcut");
                assert_eq!(manager.key_trigger(translated, mask ^ 1), None);
                assert_eq!(manager.key_trigger(translated, mask), Some(id));
                let definition = manager.entries[&id].definition.clone();
                manager.play(id, definition, Some(translated));
                let now = Instant::now();
                assert!(manager.tick(now, &BTreeSet::new()).is_empty());
                assert_eq!(manager.key_trigger(translated, mask), None, "wait for the triggering key");
                manager.release(keys::from_key(code, key));
                assert_eq!(manager.tick(now, &BTreeSet::new()), [Effect::Chat("released".into())]);
                assert_eq!(manager.key_trigger(translated, mask), Some(id), "allow the next press");
            }
        }
    }
    assert_eq!(keys::from_key(KeyCode::ArrowRight, &Key::Named(NamedKey::ArrowRight)), Some(0x83));
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
