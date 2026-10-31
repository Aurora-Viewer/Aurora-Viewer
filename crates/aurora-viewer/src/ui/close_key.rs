//! Ctrl+W closes the frontmost window, as File › Close Window does
//! (LLFileCloseWindow, indra/newview/llviewermenufile.cpp;
//! LLFloater::closeFrontmostFloater and LLFloaterView::
//! getFrontmostClosableFloater, indra/llui/llfloater.cpp, originally LGPL
//! 2.1): the window on top, minimized or not, wherever the keyboard is.
//! Ctrl+Shift+W closes every one of them, as File › Close All Windows
//! (LLFileCloseAllWindows, LLFloaterView::closeAllChildren; menu_viewer.xml
//! `shortcut="control|shift|W"`). On macOS Command is the same keyboard
//! mask as Control.
//!
//! The menu's accelerator goes before the focused field there; egui's text
//! fields take Ctrl+W to erase a word, so the key is taken at the start of
//! the interface's frame ([`begin`]), before any field reads it, and only
//! when a window is there to close. Every `widgets::Floater` declares itself
//! while it is drawn ([`shown`]) and closes as its cross would when the key
//! is for it ([`take`]). The Conversations window asks first, to close its
//! conversation shown instead (LLMultiFloater::closeDockedFloater); Close
//! All still closes that window. The mini-map has no title bar, so it is
//! not in the frontmost list ([`shown_extra`]) and only Close All takes it.

use crate::keybinds::{Input, Mods};
use egui::{Id, Order};
use std::collections::HashSet;
use winit::keyboard::{Key as NativeKey, KeyCode};

/// Reserve Close Window and Close All Windows before native movement and
/// gestures, including repeats. `mods.ctrl` is the keyboard mask: Control,
/// and Command on macOS ([`crate::keybinds::control_down`]; both set
/// MASK_CONTROL in LLKeyboardMacOSX::currentMask). Shift is Close All;
/// Alt is Close Window Group and stays free. egui still receives the event
/// and decides what closes (or edits a word when nothing is open).
/// LLViewerInput::scanKey (indra/newview/llviewerinput.cpp) likewise
/// leaves menu accelerators out of held controls. Drop an earlier hold
/// too, so releasing the modifier before W cannot resume movement.
pub fn take_native_key(key: &NativeKey, code: KeyCode, mods: Mods, down: &mut HashSet<Input>) -> bool {
    if mods.ctrl && !mods.alt && crate::keybinds::native_shortcut_key(key, code) == Some(egui::Key::W) {
        down.remove(&Input::key(code));
        true
    } else {
        false
    }
}

/// The windows drawn on the last frame, the one Ctrl+W closes this frame,
/// and whether Ctrl+Shift+W closes every window drawn then.
#[derive(Clone, Default)]
struct State {
    shown: Vec<(Id, u64)>,
    /// Windows outside the frontmost list (the mini-map). Close All only.
    extra: Vec<(Id, u64)>,
    target: Option<(Id, u64)>,
    all: bool,
}

fn state_id() -> Id {
    Id::unique("close_key_state")
}

/// The window `id` is drawn this frame: Ctrl+W can close it on the next,
/// and Ctrl+Shift+W closes it with the others.
pub fn shown(ctx: &egui::Context, id: Id) {
    note(ctx, id, false);
}

/// A window drawn this frame that Ctrl+W does not pick as frontmost.
/// Ctrl+Shift+W still closes it (the mini-map).
pub fn shown_extra(ctx: &egui::Context, id: Id) {
    note(ctx, id, true);
}

fn note(ctx: &egui::Context, id: Id, extra: bool) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<State>(state_id());
        let list = if extra { &mut state.extra } else { &mut state.shown };
        list.retain(|(other, at)| *other != id && at + 1 >= frame);
        list.push((id, frame));
    });
}

fn listed(list: &[(Id, u64)], id: Id, frame: u64) -> bool {
    list.iter().any(|(other, at)| *other == id && at + 1 >= frame)
}

/// At the start of the interface's frame: Ctrl+Shift+W closes every window
/// drawn on the last frame, and Ctrl+W alone goes to the frontmost. The key
/// is taken. Nothing with a dialog open (the menu entry is disabled while
/// an alert shows), nor without a window: a field keeps the key then.
pub fn begin(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    state.shown.retain(|(_, at)| at + 1 >= frame);
    state.extra.retain(|(_, at)| at + 1 >= frame);
    state.target = None;
    state.all = false;
    // held down, it closes once, not on each repeat
    if ctx.memory(|m| m.top_modal_layer()).is_none() {
        let shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if pressed_alone(ctx, egui::Key::W, shift) && (!state.shown.is_empty() || !state.extra.is_empty()) {
            ctx.input_mut(|i| i.consume_key(shift, egui::Key::W));
            state.all = true;
        } else if pressed_alone(ctx, egui::Key::W, egui::Modifiers::COMMAND) {
            // back to front
            let front = ctx.memory(|m| {
                m.layer_ids()
                    // a window's own layer: Middle, or Foreground over the
                    // photo viewfinder (the poser)
                    .filter_map(|layer| {
                        state
                            .shown
                            .iter()
                            .find(|(id, _)| layer.id == *id && layer.order != Order::Background)
                    })
                    .map(|(id, _)| *id)
                    .last()
            });
            if let Some(id) = front {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W));
                state.target = Some((id, frame));
            }
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
}

/// Ctrl+Shift+W is closing every window this frame. Conversations uses it
/// to close the window instead of the one conversation Ctrl+W closes.
pub fn closes_every_window(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(state_id()).is_some_and(|state| state.all))
}

/// `key` was pressed this frame with exactly `modifiers` (Ctrl+Shift+W is
/// not Ctrl+W), not repeated.
fn pressed_alone(ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, modifiers: m, .. } if *k == key && m.matches_exact(modifiers))
        })
    })
}

/// Ctrl+W asks the window `id` to close on this frame, once. Ctrl+Shift+W
/// asks every window drawn on the last frame; each of them hears it.
pub fn take(ctx: &egui::Context, id: Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<State>(state_id());
        if state.all {
            return listed(&state.shown, id, frame) || listed(&state.extra, id, frame);
        }
        let mine = state.target == Some((id, frame));
        if mine {
            state.target = None;
        }
        mine
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    #[test]
    fn ctrl_w_never_holds_movement_even_after_ctrl_is_released() {
        use crate::keybinds::{Action, Binding, KeyBindings};

        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        let shifted = Mods { shift: true, ..ctrl };
        for (mods, code, letter) in [
            (ctrl, KeyCode::KeyW, "w"),
            (ctrl, KeyCode::KeyZ, "w"), // AZERTY
            (ctrl, KeyCode::KeyW, "W"),
            (ctrl, KeyCode::KeyW, "ц"), // egui's non-Latin fallback
            (shifted, KeyCode::KeyW, "w"),
            (shifted, KeyCode::KeyZ, "w"), // AZERTY Close All
        ] {
            let input = Input::key(code);
            let other = Input::key(KeyCode::ArrowDown);
            let key = NativeKey::Character(letter.into());
            let mut bindings = KeyBindings::default();
            let actions = [Action::Forward, Action::CamZoomIn, Action::PushToTalk, Action::HudDrag];
            for action in actions {
                bindings.set(action, 0, Some(Binding::captured(input.clone(), Mods::default())));
            }
            // The same press/repeat path also cancels a pre-existing hold.
            let mut down = HashSet::from([input.clone(), other.clone()]);
            for _ in 0..3 {
                assert!(take_native_key(&key, code, mods, &mut down));
                assert_eq!(down, HashSet::from([other.clone()]));
                for mods in [ctrl, shifted, Mods::default()] {
                    for action in actions {
                        assert!(!bindings.held(action, &down, mods), "{letter}: {action:?}");
                    }
                }
            }
            assert!(!bindings.held(Action::Forward, &down, Mods::default()));
            let _ = bindings.update_held_input(&mut down, &input, false, Mods::default());
            assert!(!bindings.held(Action::Forward, &down, Mods::default()));
            let _ = bindings.update_held_input(&mut down, &input, true, Mods::default());
            assert!(
                bindings.held(Action::Forward, &down, Mods::default()),
                "a fresh plain press still moves"
            );
        }
    }

    #[test]
    fn native_close_reservation_keeps_other_keys_and_modifier_combinations() {
        let input = Input::key(KeyCode::KeyW);
        for (letter, mods) in [
            ("w", Mods::default()),
            (
                "w",
                Mods {
                    ctrl: true,
                    alt: true,
                    shift: true,
                },
            ),
            (
                "w",
                Mods {
                    ctrl: true,
                    alt: true,
                    shift: false,
                },
            ),
            (
                "w",
                Mods {
                    alt: true,
                    ..Default::default()
                },
            ),
            (
                "z",
                Mods {
                    ctrl: true,
                    ..Default::default()
                },
            ), // AZERTY Ctrl+Z
            (
                "z",
                Mods {
                    ctrl: true,
                    shift: true,
                    alt: false,
                },
            ), // Ctrl+Shift+Z, including AZERTY's physical W key
        ] {
            let mut down = HashSet::from([input.clone()]);
            assert!(!take_native_key(
                &NativeKey::Character(letter.into()),
                KeyCode::KeyW,
                mods,
                &mut down
            ));
            assert!(down.contains(&input));
        }
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn w_left(ctx: &egui::Context) -> bool {
        ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::W, .. })))
    }

    /// Two windows, "right" raised over "left" unless `raise_left`, the
    /// keyboard in a field of "left" (with no window but `windows` false:
    /// the field alone). Returns the windows asked to close over the frames
    /// that follow `events`, and whether the key was left to the fields.
    fn press(raise_left: bool, windows: bool, events: Vec<egui::Event>) -> (Vec<&'static str>, bool) {
        let mut frames = Frames::new();
        let mut text = String::from("two words");
        let mut closed = Vec::new();
        let mut left_to_fields = false;
        let field = Id::unique("left_field");
        let mut frame = |frames: &mut Frames, events: Vec<egui::Event>, closed: &mut Vec<&'static str>, left: &mut bool| {
            frames.run(events, |ui| {
                let ctx = ui.ctx().clone();
                begin(&ctx);
                *left |= w_left(&ctx);
                if !windows {
                    ui.add(egui::TextEdit::singleline(&mut text).id(field));
                }
                for (name, x) in [("left", 0.0), ("right", 400.0)].into_iter().filter(|_| windows) {
                    let id = Id::unique(name);
                    egui::Window::new(name).id(id).fixed_pos(egui::pos2(x, 0.0)).show(&ctx, |ui| {
                        if name == "left" {
                            ui.add(egui::TextEdit::singleline(&mut text).id(field));
                        }
                    });
                    shown(&ctx, id);
                    if take(&ctx, id) {
                        closed.push(name);
                    }
                }
                ctx.memory_mut(|m| m.request_focus(field));
            });
        };
        let mut ignored = false;
        frame(&mut frames, Vec::new(), &mut closed, &mut ignored);
        frames.ctx.move_to_top(egui::LayerId::new(
            Order::Middle,
            Id::unique(if raise_left { "left" } else { "right" }),
        ));
        frame(&mut frames, Vec::new(), &mut closed, &mut ignored);
        frame(&mut frames, events, &mut closed, &mut left_to_fields);
        frame(&mut frames, Vec::new(), &mut closed, &mut left_to_fields);
        (closed, left_to_fields)
    }

    #[test]
    fn ctrl_w_closes_the_frontmost_window_before_any_field_reads_it() {
        let w = || vec![key(egui::Key::W, egui::Modifiers::COMMAND)];
        // the window on top, whichever has the keyboard; once
        assert_eq!(press(false, true, w()), (vec!["right"], false));
        assert_eq!(press(true, true, w()), (vec!["left"], false));
    }

    #[test]
    fn other_keys_close_nothing() {
        for (event, w) in [
            (key(egui::Key::W, egui::Modifiers::ALT), true),
            (key(egui::Key::Q, egui::Modifiers::COMMAND), false),
            (
                key(
                    egui::Key::W,
                    egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT,
                ),
                true,
            ),
        ] {
            assert_eq!(press(false, true, vec![event]), (Vec::new(), w));
        }
    }

    #[test]
    fn ctrl_shift_w_closes_every_window_before_a_field_reads_it() {
        let chord = || vec![key(egui::Key::W, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)];
        assert_eq!(press(false, true, chord()), (vec!["left", "right"], false));
        assert_eq!(press(true, true, chord()), (vec!["left", "right"], false));
        // no window: the field keeps the chord
        assert_eq!(press(false, false, chord()), (Vec::new(), true));
    }

    #[test]
    fn ctrl_shift_w_closes_a_window_ctrl_w_does_not_pick() {
        let mut frames = Frames::new();
        let map = Id::unique("map");
        let mut closed = false;
        let frame = |frames: &mut Frames, events: Vec<egui::Event>, closed: &mut bool| {
            frames.run(events, |ui| {
                let ctx = ui.ctx().clone();
                begin(&ctx);
                if take(&ctx, map) {
                    *closed = true;
                }
                shown_extra(&ctx, map);
            });
        };
        frame(&mut frames, Vec::new(), &mut closed);
        frame(&mut frames, vec![key(egui::Key::W, egui::Modifiers::COMMAND)], &mut closed);
        assert!(!closed, "Ctrl+W leaves the mini-map");
        // release W, or the next press is a repeat and does not close again
        frame(
            &mut frames,
            vec![egui::Event::Key {
                key: egui::Key::W,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut closed,
        );
        frame(
            &mut frames,
            vec![key(egui::Key::W, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
            &mut closed,
        );
        assert!(closed);
    }

    #[test]
    fn without_a_window_the_field_keeps_ctrl_w() {
        // egui's own Ctrl+W then: the word before the cursor
        assert_eq!(
            press(false, false, vec![key(egui::Key::W, egui::Modifiers::COMMAND)]),
            (Vec::new(), true)
        );
    }
}
