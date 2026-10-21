//! Ctrl+W closes the frontmost window, as File › Close Window does
//! (LLFileCloseWindow, indra/newview/llviewermenufile.cpp;
//! LLFloater::closeFrontmostFloater and LLFloaterView::
//! getFrontmostClosableFloater, indra/llui/llfloater.cpp, originally LGPL
//! 2.1): the window on top, minimized or not, wherever the keyboard is.
//!
//! The menu's accelerator goes before the focused field there; egui's text
//! fields take Ctrl+W to erase a word, so the key is taken at the start of
//! the interface's frame ([`begin`]), before any field reads it, and only
//! when a window is there to close. Every `widgets::Floater` declares itself
//! while it is drawn ([`shown`]) and closes as its cross would when the key
//! is for it ([`take`]). The Conversations window asks first, to close its
//! conversation shown instead (LLMultiFloater::closeDockedFloater).

use egui::{Id, LayerId, Order};

/// The windows drawn on the last frame, and the one Ctrl+W closes this
/// frame.
#[derive(Clone, Default)]
struct State {
    shown: Vec<(Id, u64)>,
    target: Option<(Id, u64)>,
}

fn state_id() -> Id {
    Id::new("close_key_state")
}

/// The window `id` is drawn this frame: Ctrl+W can close it on the next.
pub fn shown(ctx: &egui::Context, id: Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<State>(state_id());
        state.shown.retain(|(other, at)| *other != id && at + 1 >= frame);
        state.shown.push((id, frame));
    });
}

/// At the start of the interface's frame: Ctrl+W alone goes to the frontmost
/// window drawn on the last frame, and the key is taken. Nothing with a
/// dialog open (the menu entry is disabled while an alert shows), nor
/// without a window: a field keeps the key then.
pub fn begin(ctx: &egui::Context) {
    let frame = ctx.cumulative_frame_nr();
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    state.shown.retain(|(_, at)| at + 1 >= frame);
    state.target = None;
    // held down, it closes one window, not one per repeat
    if pressed_alone(ctx, egui::Key::W, egui::Modifiers::COMMAND) && ctx.memory(|m| m.top_modal_layer()).is_none() {
        // back to front
        let front = ctx.memory(|m| {
            m.layer_ids()
                .filter_map(|layer| state.shown.iter().find(|(id, _)| LayerId::new(Order::Middle, *id) == layer))
                .map(|(id, _)| *id)
                .last()
        });
        if let Some(id) = front {
            ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W));
            state.target = Some((id, frame));
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
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

/// Ctrl+W asks the window `id` to close on this frame; true once.
pub fn take(ctx: &egui::Context, id: Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<State>(state_id());
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
        let field = Id::new("left_field");
        let mut frame = |frames: &mut Frames, events: Vec<egui::Event>, closed: &mut Vec<&'static str>, left: &mut bool| {
            frames.run(events, |ui| {
                let ctx = ui.ctx().clone();
                begin(&ctx);
                *left |= w_left(&ctx);
                if !windows {
                    ui.add(egui::TextEdit::singleline(&mut text).id(field));
                }
                for (name, x) in [("left", 0.0), ("right", 400.0)].into_iter().filter(|_| windows) {
                    let id = Id::new(name);
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
        frames
            .ctx
            .move_to_top(LayerId::new(Order::Middle, Id::new(if raise_left { "left" } else { "right" })));
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
            (key(egui::Key::W, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT), true),
            (key(egui::Key::W, egui::Modifiers::ALT), true),
            (key(egui::Key::Q, egui::Modifiers::COMMAND), false),
        ] {
            assert_eq!(press(false, true, vec![event]), (Vec::new(), w));
        }
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
