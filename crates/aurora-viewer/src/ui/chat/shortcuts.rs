//! The chat fields' rebindable shortcuts (Preferences › Controls ›
//! Shortcuts › Chat, `keybinds::is_chat`): a line break, shout, whisper,
//! ((OOC)), the sent messages' history and the conversations' cycling.
//! The Legacy Viewer hard-codes them (FSFloaterIM::handleKeyHere,
//! FSNearbyChatControl::handleKeyHere, indra/newview); Aurora reads them
//! from the user's table (human request of 2026-11-15). Send (Enter),
//! Close (Ctrl+W) and Find (Ctrl+F) stay fixed.
//!
//! The app publishes the table once a frame (`publish`); the fields read
//! it from the context (`get`), which falls back to the defaults (tests,
//! a frame before the first publish).

use crate::keybinds::{Action, Binding, KeyBindings};

/// The chat actions' bindings, as the user set them.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatKeys {
    map: Vec<(Action, [Option<Binding>; 2])>,
}

impl ChatKeys {
    pub(crate) fn of(kb: &KeyBindings) -> ChatKeys {
        ChatKeys {
            map: kb.map.iter().filter(|(a, _)| crate::keybinds::is_chat(*a)).cloned().collect(),
        }
    }

    fn bindings(&self, a: Action) -> impl Iterator<Item = &Binding> {
        self.map.iter().filter(move |(x, _)| *x == a).flat_map(|(_, b)| b.iter().flatten())
    }

    /// Whether a key press is one of the action's bindings.
    pub(super) fn hit(&self, a: Action, key: egui::Key, physical: Option<egui::Key>, m: egui::Modifiers) -> bool {
        self.bindings(a).any(|b| b.matches_egui(key, physical, m))
    }

    /// The same for an event: a key pressed (repeats included).
    pub(super) fn pressed(&self, a: Action, event: &egui::Event) -> bool {
        matches!(
            event,
            egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } if self.hit(a, *key, *physical_key, *modifiers)
        )
    }

    /// The first binding's keys, for a tooltip; None when unbound.
    pub(super) fn label(&self, a: Action) -> Option<String> {
        self.bindings(a).next().map(Binding::label)
    }
}

fn id() -> egui::Id {
    egui::Id::unique("chat_shortcuts")
}

/// The user's table, for this frame's chat fields.
pub(crate) fn publish(ctx: &egui::Context, kb: &KeyBindings) {
    let keys = ChatKeys::of(kb);
    ctx.data_mut(|d| {
        if d.get_temp::<ChatKeys>(id()).as_ref() != Some(&keys) {
            d.insert_temp(id(), keys);
        }
    });
}

/// The table published, else the defaults.
pub(super) fn get(ctx: &egui::Context) -> ChatKeys {
    ctx.data(|d| d.get_temp::<ChatKeys>(id()))
        .unwrap_or_else(|| ChatKeys::of(&KeyBindings::default()))
}

/// The field's tooltip: Enter to send, then each shortcut that is bound
/// (the nearby chat's volumes and ((OOC)) only there), joined as the
/// language joins a list of keys.
pub(super) fn tip(keys: &ChatKeys, local: bool, channel: bool) -> String {
    let mut parts = vec![aurora_i18n::tr!("ui-chat-key-send").to_owned()];
    let mut add = |a: Action, line: fn(String) -> String| {
        if let Some(k) = keys.label(a) {
            parts.push(line(k));
        }
    };
    add(Action::ChatNewLine, |k| aurora_i18n::tr!("ui-chat-key-new-line", keys = k));
    if local {
        add(Action::ChatShout, |k| aurora_i18n::tr!("ui-chat-key-shout", keys = k));
        add(Action::ChatWhisper, |k| aurora_i18n::tr!("ui-chat-key-whisper", keys = k));
    }
    add(Action::ChatOoc, |k| aurora_i18n::tr!("ui-chat-key-ooc", keys = k));
    if channel {
        parts.push(aurora_i18n::tr!("ui-chat-key-channel").to_owned());
    }
    parts.join(aurora_i18n::tr!("ui-chat-keys-join"))
}

/// The nearby chat's volume button: its name, and the keys that shout or
/// whisper when they are bound.
pub(super) fn volume_tip(keys: &ChatKeys) -> String {
    let mut tip = aurora_i18n::tr!("ui-chat-volume-tip").to_owned();
    let lines: Vec<String> = [
        keys.label(Action::ChatShout)
            .map(|k| aurora_i18n::tr!("ui-chat-key-shout", keys = k)),
        keys.label(Action::ChatWhisper)
            .map(|k| aurora_i18n::tr!("ui-chat-key-whisper", keys = k)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !lines.is_empty() {
        tip.push('\n');
        tip.push_str(&lines.join(aurora_i18n::tr!("ui-chat-keys-join")));
    }
    tip
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::Input;
    use egui::{Key, Modifiers};

    fn press(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CTRL: Modifiers = Modifiers::CTRL;

    #[test]
    fn the_defaults_are_the_ones_chosen() {
        let keys = ChatKeys::of(&KeyBindings::default());
        let ctrl_shift = CTRL | SHIFT;
        assert!(keys.pressed(Action::ChatNewLine, &press(Key::Enter, SHIFT)));
        assert!(keys.pressed(Action::ChatNewLine, &press(Key::Enter, ctrl_shift)));
        assert!(keys.pressed(Action::ChatShout, &press(Key::Enter, CTRL)));
        assert!(keys.pressed(Action::ChatOoc, &press(Key::Enter, Modifiers::ALT)));
        assert!(keys.pressed(Action::ChatHistoryPrevious, &press(Key::ArrowUp, CTRL)));
        assert!(keys.pressed(Action::ChatHistoryNext, &press(Key::ArrowDown, CTRL)));
        assert!(keys.pressed(Action::ChatNextConversation, &press(Key::Tab, CTRL)));
        assert!(keys.pressed(Action::ChatNextConversation, &press(Key::ArrowDown, Modifiers::ALT)));
        assert!(keys.pressed(Action::ChatPreviousConversation, &press(Key::Tab, ctrl_shift)));
        assert!(keys.pressed(Action::ChatPreviousConversation, &press(Key::ArrowUp, Modifiers::ALT)));
        // whisper is unbound: Shift+Enter is the line break now
        assert_eq!(keys.label(Action::ChatWhisper), None);
        assert!(!keys.pressed(Action::ChatWhisper, &press(Key::Enter, SHIFT)));
        // Alt+Left / Right no longer cycle (two keys per action)
        assert!(!keys.pressed(Action::ChatNextConversation, &press(Key::ArrowRight, Modifiers::ALT)));
        // exact modifiers: plain Enter is none of them, Ctrl+Alt+Enter neither
        for a in [Action::ChatNewLine, Action::ChatShout, Action::ChatOoc] {
            assert!(!keys.pressed(a, &press(Key::Enter, Modifiers::NONE)));
            assert!(!keys.pressed(a, &press(Key::Enter, CTRL | Modifiers::ALT)));
        }
        // a release is no press
        let release = egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: SHIFT,
        };
        assert!(!keys.pressed(Action::ChatNewLine, &release));
    }

    #[test]
    fn a_rebound_key_follows_the_table_by_its_place() {
        let mut kb = KeyBindings::default();
        let f5 = Binding::captured(Input::key(winit::keyboard::KeyCode::F5), crate::keybinds::Mods::default());
        kb.set(Action::ChatWhisper, 0, Some(f5));
        kb.set(Action::ChatNewLine, 0, None);
        kb.set(Action::ChatNewLine, 1, None);
        let keys = ChatKeys::of(&kb);
        assert!(keys.pressed(Action::ChatWhisper, &press(Key::F5, Modifiers::NONE)));
        assert!(!keys.pressed(Action::ChatNewLine, &press(Key::Enter, SHIFT)));
        // the physical key decides, as for the world's bindings (AZERTY's A
        // is the Q place)
        let mut kb = KeyBindings::default();
        kb.set(
            Action::ChatOoc,
            0,
            Some(Binding::captured(
                Input::key(winit::keyboard::KeyCode::KeyQ),
                crate::keybinds::Mods {
                    ctrl: true,
                    ..Default::default()
                },
            )),
        );
        let keys = ChatKeys::of(&kb);
        let azerty_a = egui::Event::Key {
            key: Key::A,
            physical_key: Some(Key::Q),
            pressed: true,
            repeat: false,
            modifiers: CTRL,
        };
        assert!(keys.pressed(Action::ChatOoc, &azerty_a));
    }

    #[test]
    fn command_is_not_control_for_the_chat_keys() {
        let keys = ChatKeys::of(&KeyBindings::default());
        let cmd = Modifiers {
            mac_cmd: true,
            command: true,
            ..Default::default()
        };
        assert!(!keys.pressed(Action::ChatHistoryPrevious, &press(Key::ArrowUp, cmd)));
    }

    #[test]
    fn a_mouse_binding_never_matches_a_key() {
        let b = Binding::captured(Input::Mouse("Middle".into()), crate::keybinds::Mods::default());
        assert!(!b.matches_egui(Key::Enter, None, Modifiers::NONE));
    }

    #[test]
    fn the_tip_lists_only_what_is_bound() {
        let keys = ChatKeys::of(&KeyBindings::default());
        let local = tip(&keys, true, true);
        let im = tip(&keys, false, false);
        let shout = keys.label(Action::ChatShout).expect("shout bound");
        let new_line = keys.label(Action::ChatNewLine).expect("new line bound");
        assert!(local.contains(&shout) && local.contains(&new_line));
        assert!(im.contains(&new_line) && !im.contains(&shout));
        // whisper is unbound: send, new line, shout, ((OOC)) and the channel
        assert_eq!(local.split(aurora_i18n::tr!("ui-chat-keys-join")).count(), 5);
        assert_eq!(im.split(aurora_i18n::tr!("ui-chat-keys-join")).count(), 3);
        let mut kb = KeyBindings::default();
        kb.set(Action::ChatShout, 0, None);
        assert_eq!(volume_tip(&ChatKeys::of(&kb)), aurora_i18n::tr!("ui-chat-volume-tip"));
    }
}
