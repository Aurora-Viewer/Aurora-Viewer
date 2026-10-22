//! Who gets Ctrl+F. Three features ask for it:
//!
//! - the script editor, for its own find (LLFloaterScriptSearch);
//! - a window with a find bar over a conversation (Conversations, the
//!   history of a conversation), like LLFloaterSearchReplace on a chat
//!   history;
//! - "Search", the accelerator of Firestorm's menu (menu_viewer.xml),
//!   a shortcut of the viewer (`keybinds::Action::Search` by default).
//!
//! One rule decides, [`owner`]: an active script editor owns its shortcut
//! across its fields, controls and menus. Otherwise the find window whose
//! field has the keyboard wins; with the keyboard nowhere, the find window
//! under the pointer; else the viewer's shortcut. It is asked from
//! both ends with the same egui state: by a find window while it is drawn
//! ([`take`]), and by the app before it runs the viewer's shortcut bound to
//! these keys ([`window_takes_it`]). `scripts/focus.rs` tracks script
//! activation and dispatches the key after all editor commands have run.

use egui::{Id, LayerId};

/// Where the keyboard is when Ctrl+F is pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyboard {
    /// In no field.
    Nowhere,
    /// In the active script editor, including its controls and popup menus.
    ScriptEditor,
    /// In a field of a window that has a find bar: the layer of that window.
    FindWindow(LayerId),
    /// In any other field (the chat bar, a filter, the search window…).
    Elsewhere,
}

/// What Ctrl+F does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The find of the script editor.
    ScriptEditor,
    /// The find bar of this window.
    FindWindow(LayerId),
    /// The viewer's shortcut on these keys: "Search" by default.
    Search,
}

/// The rule. `pointer` is the window with a find bar under the pointer, if
/// any: it only counts while the keyboard is nowhere, so that a field keeps
/// the shortcuts of its own window wherever the pointer rests.
pub fn owner(keyboard: Keyboard, pointer: Option<LayerId>) -> Owner {
    match (keyboard, pointer) {
        (Keyboard::ScriptEditor, _) => Owner::ScriptEditor,
        (Keyboard::FindWindow(window), _) | (Keyboard::Nowhere, Some(window)) => Owner::FindWindow(window),
        (Keyboard::Nowhere, None) | (Keyboard::Elsewhere, _) => Owner::Search,
    }
}

/// A window with a find bar, as it said on the last frame it was drawn.
#[derive(Clone)]
struct Claim {
    layer: LayerId,
    /// Its fields: the keyboard in one of them is the keyboard in the window.
    fields: Vec<Id>,
    frame: u64,
}

fn claims_id() -> Id {
    Id::unique("find_shortcut_claims")
}

/// The windows with a find bar drawn on the last frame.
fn claims(ctx: &egui::Context) -> Vec<Claim> {
    // read outside the closure: the context is locked inside it
    let now = ctx.cumulative_frame_nr();
    let mut claims = ctx.data(|d| d.get_temp::<Vec<Claim>>(claims_id())).unwrap_or_default();
    claims.retain(|claim| claim.frame + 1 >= now);
    claims
}

fn claim(ctx: &egui::Context, layer: LayerId, fields: &[Id]) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let claims = d.get_temp_mut_or_default::<Vec<Claim>>(claims_id());
        claims.retain(|claim| claim.layer != layer && claim.frame + 1 >= frame);
        claims.push(Claim {
            layer,
            fields: fields.to_vec(),
            frame,
        });
    });
}

/// Who gets Ctrl+F now: the rule on where the keyboard and the pointer are.
pub fn current_owner(ctx: &egui::Context) -> Owner {
    let claims = claims(ctx);
    let keyboard = if super::scripts::editor_focused(ctx) {
        Keyboard::ScriptEditor
    } else {
        match ctx.memory(|m| m.focused()) {
            None => Keyboard::Nowhere,
            Some(field) => match claims.iter().find(|claim| claim.fields.contains(&field)) {
                Some(claim) => Keyboard::FindWindow(claim.layer),
                None => Keyboard::Elsewhere,
            },
        }
    };
    let pointer = ctx
        .pointer_latest_pos()
        .and_then(|pos| ctx.layer_id_at(pos))
        .filter(|layer| claims.iter().any(|claim| claim.layer == *layer));
    owner(keyboard, pointer)
}

/// Ctrl+F was pressed for the find bar of the window this `ui` is in, whose
/// fields are `own`; the key is consumed when taken. To call on each frame
/// the window is drawn: it is also how the window is known to have a find
/// bar.
pub fn take(ui: &egui::Ui, own: &[Id]) -> bool {
    let ctx = ui.ctx();
    claim(ctx, ui.layer_id(), own);
    current_owner(ctx) == Owner::FindWindow(ui.layer_id()) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
}

/// A window keeps Ctrl+F for its own find: the viewer's shortcut on these
/// keys does not run.
pub fn window_takes_it(ctx: &egui::Context) -> bool {
    current_owner(ctx) != Owner::Search
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    fn window(name: &str) -> LayerId {
        LayerId::new(egui::Order::Middle, Id::unique(name))
    }

    #[test]
    fn the_field_with_the_keyboard_then_the_pointer_then_the_viewer() {
        let (conversations, history) = (window("conversations"), window("chat_history"));
        // a focused text field's owner wins, wherever the pointer is
        for pointer in [None, Some(conversations)] {
            assert_eq!(owner(Keyboard::ScriptEditor, pointer), Owner::ScriptEditor);
            assert_eq!(owner(Keyboard::FindWindow(history), pointer), Owner::FindWindow(history));
        }
        assert_eq!(owner(Keyboard::FindWindow(conversations), None), Owner::FindWindow(conversations));
        // the keyboard nowhere: the conversation under the pointer
        assert_eq!(owner(Keyboard::Nowhere, Some(conversations)), Owner::FindWindow(conversations));
        // nothing focused, the pointer elsewhere: "Search"
        assert_eq!(owner(Keyboard::Nowhere, None), Owner::Search);
        // another field (chat bar, inventory filter, the search window's own)
        // leaves the viewer its shortcut, even with the pointer on a conversation
        assert_eq!(owner(Keyboard::Elsewhere, None), Owner::Search);
        assert_eq!(owner(Keyboard::Elsewhere, Some(conversations)), Owner::Search);
    }

    const FIND_FIELD: &str = "find_field";
    const OTHER_FIELD: &str = "other_field";
    const ON_FIND_WINDOW: egui::Pos2 = egui::pos2(150.0, 150.0);
    const ON_OTHER_WINDOW: egui::Pos2 = egui::pos2(750.0, 150.0);
    const ON_WORLD: egui::Pos2 = egui::pos2(450.0, 700.0);

    /// Frames of a window with a find bar beside another window, the
    /// pointer on `pointer` and the keyboard in `keyboard`; then what the
    /// app is told before it runs its shortcut, and whether the find window
    /// takes Ctrl+F on the frame that follows: never both, never neither.
    fn ctrl_f(pointer: egui::Pos2, keyboard: Option<&str>, find_window_shown: bool) -> (bool, bool) {
        let mut frames = Frames::new();
        let mut taken = false;
        let mut frame = |frames: &mut Frames, events: Vec<egui::Event>| {
            let (mut find, mut other) = (String::new(), String::new());
            frames.run(events, |ui| {
                let ctx = ui.ctx().clone();
                if let Some(field) = keyboard {
                    ctx.memory_mut(|m| m.request_focus(Id::unique(field)));
                }
                let area = |name: &str, x: f32| egui::Area::new(Id::unique(name)).fixed_pos(egui::pos2(x, 0.0));
                if find_window_shown {
                    area("find", 0.0).show(&ctx, |ui| {
                        ui.set_min_size(egui::vec2(300.0, 300.0));
                        ui.add(egui::TextEdit::singleline(&mut find).id(Id::unique(FIND_FIELD)));
                        taken = take(ui, &[Id::unique(FIND_FIELD)]);
                    });
                }
                area("other", 600.0).show(&ctx, |ui| {
                    ui.set_min_size(egui::vec2(300.0, 300.0));
                    ui.add(egui::TextEdit::singleline(&mut other).id(Id::unique(OTHER_FIELD)));
                });
            });
        };
        frame(&mut frames, vec![egui::Event::PointerMoved(pointer)]);
        frame(&mut frames, Vec::new());
        // the key arrives between two frames: the app asks first
        let yields = window_takes_it(&frames.ctx);
        let key = egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        frame(&mut frames, vec![key]);
        (yields, taken)
    }

    #[test]
    fn the_app_and_the_find_window_agree_on_each_press() {
        // the keyboard in the find window: its find bar
        assert_eq!(ctrl_f(ON_OTHER_WINDOW, Some(FIND_FIELD), true), (true, true));
        // the keyboard nowhere, the pointer over the find window: its find bar
        assert_eq!(ctrl_f(ON_FIND_WINDOW, None, true), (true, true));
        // the keyboard nowhere, the pointer elsewhere: the viewer's shortcut
        assert_eq!(ctrl_f(ON_OTHER_WINDOW, None, true), (false, false));
        assert_eq!(ctrl_f(ON_WORLD, None, true), (false, false));
        // the keyboard in another field: the viewer's shortcut
        assert_eq!(ctrl_f(ON_FIND_WINDOW, Some(OTHER_FIELD), true), (false, false));
        // no find window on screen: nothing to yield to
        assert_eq!(ctrl_f(ON_FIND_WINDOW, None, false), (false, false));
    }

    /// A window that is no longer drawn stops taking the shortcut.
    #[test]
    fn a_closed_find_window_gives_the_shortcut_back() {
        let mut frames = Frames::new();
        let frame = |frames: &mut Frames, shown: bool| {
            frames.run(vec![egui::Event::PointerMoved(ON_FIND_WINDOW)], |ui| {
                let ctx = ui.ctx().clone();
                if shown {
                    egui::Area::new(Id::unique("find")).fixed_pos(egui::Pos2::ZERO).show(&ctx, |ui| {
                        ui.set_min_size(egui::vec2(300.0, 300.0));
                        take(ui, &[]);
                    });
                }
            });
        };
        frame(&mut frames, true);
        frame(&mut frames, true);
        assert!(window_takes_it(&frames.ctx));
        frame(&mut frames, false);
        assert!(!window_takes_it(&frames.ctx));
    }
}
