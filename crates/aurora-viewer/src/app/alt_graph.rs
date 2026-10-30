//! AltGr in shortcuts and under the mouse. On a keyboard layout with AltGr
//! (French, German…) Windows types the key as Ctrl + Alt. The Legacy Viewer
//! reads both for shortcuts (LLKeyboardWin32::currentMask,
//! indra/llwindow/llkeyboardwin32.cpp) and its camera gestures
//! (LLToolCamera::handleHover, indra/newview/lltoolfocus.cpp, originally
//! LGPL 2.1). winit hides both modifiers while AltGr is down. Count them
//! back for shortcut capture and use, leaving text/web/media input alone:
//! typing `@` must not run a shortcut (LLViewerWindow::handleKey,
//! indra/newview/llviewerwindow.cpp).

use super::App;
use crate::keybinds::Mods;
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

/// Whether AltGr is held after this key event: the layout types the right
/// Alt key as AltGraph (on a layout without AltGr it is a plain Alt, which
/// winit already reports); its release, whatever it is typed as, lets go.
fn alt_graph_after(held: bool, logical: &Key, physical: PhysicalKey, pressed: bool) -> bool {
    let alt_graph = *logical == Key::Named(NamedKey::AltGraph);
    if alt_graph || physical == PhysicalKey::Code(KeyCode::AltRight) {
        alt_graph && pressed
    } else {
        held
    }
}

/// Alt and Ctrl as the mouse reads them: AltGr counts as both.
fn mouse_alt_ctrl(alt: bool, ctrl: bool, alt_graph: bool) -> (bool, bool) {
    (alt || alt_graph, ctrl || alt_graph)
}

fn produces_text(logical: &Key, text: Option<&str>) -> bool {
    matches!(logical, Key::Dead(_)) || text.is_some_and(|s| s.chars().any(|c| !c.is_control()))
}

/// Shortcut capture always reads AltGr as Ctrl + Alt, using the existing
/// binding format and conflict checks. A text owner keeps winit's modifiers.
pub(super) fn keyboard_mods(mut mods: Mods, alt_graph: bool, text_owner: bool, capturing: bool) -> Mods {
    if alt_graph && (capturing || !text_owner) {
        mods.ctrl = true;
        mods.alt = true;
    }
    mods
}

impl App {
    fn shortcut_text_owner(&self) -> bool {
        self.egui_ctx.egui_wants_keyboard_input()
            || self.profile_ui.web_has_focus()
            || self.search_ui.web_has_focus()
            || self.media.focus.is_some()
    }

    pub(super) fn altgr_text_input(&self, ke: &KeyEvent) -> bool {
        self.alt_graph
            && self.options_ui.capture.is_none()
            && produces_text(&ke.logical_key, ke.text.as_deref())
            && self.shortcut_text_owner()
    }

    pub(super) fn shortcut_mods(&self, mods: Mods) -> Mods {
        keyboard_mods(mods, self.alt_graph, false, false)
    }

    /// Only character-producing AltGr presses yield to a text owner;
    /// AltGr + F9 still runs Ctrl + Alt + F9 while a field is focused.
    pub(super) fn key_mods(&self, ke: &KeyEvent) -> Mods {
        keyboard_mods(
            Mods {
                ctrl: self.ctrl,
                shift: self.shift,
                alt: self.alt,
            },
            self.alt_graph,
            self.altgr_text_input(ke),
            self.options_ui.capture.is_some(),
        )
    }

    /// Native shortcuts and web/media input must not also fire egui's
    /// unmodified key actions. Releases always reach egui to clear holds.
    pub(super) fn altgr_native_key(&self, ke: &KeyEvent) -> bool {
        self.alt_graph
            && ke.state == ElementState::Pressed
            && (self.options_ui.capture.is_some()
                || !self.egui_ctx.egui_wants_keyboard_input()
                || !produces_text(&ke.logical_key, ke.text.as_deref()))
    }

    /// Exercise capture and the real command dispatcher without a grid or
    /// changing the demo's saved bindings: AURORA_DEMO_OPTIONS=shortcuts:altgr.
    pub(super) fn demo_altgr_shortcuts(&mut self) {
        if !self.demo || self.frame_count != 600 || std::env::var("AURORA_DEMO_OPTIONS").as_deref() != Ok("shortcuts:altgr") {
            return;
        }
        use crate::keybinds::{Action, Binding, Input};
        use crate::ui::options::{Capture, Page};
        self.demo_release_keyboard();
        let bindings = self.settings.keybinds.clone();
        let modifiers = (self.ctrl, self.alt, self.shift, self.alt_graph);
        let input = Input::key(KeyCode::F9);
        for (slot, shift) in [false, true].into_iter().enumerate() {
            (self.ctrl, self.alt, self.shift, self.alt_graph) = (false, false, shift, true);
            self.options_ui.capture = Some(Capture::Binding(Action::DevToolbox, slot));
            if let Some(captured) = self.options_ui.capture_key(KeyCode::F9, true, false) {
                self.finish_capture(captured);
            }
            let expected = Binding::captured(
                input.clone(),
                Mods {
                    ctrl: true,
                    alt: true,
                    shift,
                },
            );
            let captured = self.settings.keybinds.get(Action::DevToolbox)[slot].as_ref() == Some(&expected);
            self.panels.dev_toolbox = false;
            self.on_input(input.clone(), true);
            self.on_input(input.clone(), false);
            log::info!(
                "demo altgr shortcuts: slot={slot} shift={shift} captured={captured} opened={}",
                self.panels.dev_toolbox
            );
        }
        self.settings.keybinds = bindings;
        self.settings.save();
        (self.ctrl, self.alt, self.shift, self.alt_graph) = modifiers;
        self.options_ui.open(Page::Shortcuts, 4);
        self.options_ui.query = crate::keybinds::label(Action::DevToolbox).into();
    }

    /// Every key event of the window, before anything can take it.
    pub(super) fn track_alt_graph(&mut self, ke: &KeyEvent) {
        self.alt_graph = alt_graph_after(self.alt_graph, &ke.logical_key, ke.physical_key, ke.state == ElementState::Pressed);
    }

    /// Alt and Ctrl for the mouse's camera gestures and the cursor over the
    /// world. In mouselook AltGr stays a character key: it does not free the
    /// cursor (cursor_hold.rs).
    pub(super) fn mouse_alt_ctrl(&self) -> (bool, bool) {
        mouse_alt_ctrl(self.alt, self.ctrl, self.alt_graph && !self.camera.mouselook())
    }

    /// Runs a mouse button's handler with AltGr counted as Ctrl + Alt (the
    /// deferred click keeps them with its other modifiers), then gives the
    /// keyboard its own modifiers back.
    pub(super) fn with_mouse_mods(&mut self, handler: impl FnOnce(&mut Self)) {
        let keys = (self.alt, self.ctrl);
        let mouse = self.mouse_alt_ctrl();
        (self.alt, self.ctrl) = mouse;
        handler(self);
        // (unless the handler set them itself, as a demo does)
        if (self.alt, self.ctrl) == mouse {
            (self.alt, self.ctrl) = keys;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{Action, Binding, Input, KeyBindings};
    use crate::ui::options::{Capture, OptionsUi};
    use std::collections::HashSet;

    const ALT_RIGHT: PhysicalKey = PhysicalKey::Code(KeyCode::AltRight);

    #[test]
    fn altgr_is_held_from_its_press_to_its_release() {
        let altgr = Key::Named(NamedKey::AltGraph);
        assert!(alt_graph_after(false, &altgr, ALT_RIGHT, true));
        // (the key repeats while held)
        assert!(alt_graph_after(true, &altgr, ALT_RIGHT, true));
        assert!(!alt_graph_after(true, &altgr, ALT_RIGHT, false));
        // a release typed otherwise still lets go
        assert!(!alt_graph_after(true, &Key::Named(NamedKey::Alt), ALT_RIGHT, false));
    }

    #[test]
    fn right_alt_without_altgr_and_other_keys_are_not_altgr() {
        // a layout without AltGr: winit already reports right Alt as Alt
        assert!(!alt_graph_after(false, &Key::Named(NamedKey::Alt), ALT_RIGHT, true));
        // the characters typed with AltGr leave it held
        let at = Key::Character("@".into());
        assert!(alt_graph_after(true, &at, PhysicalKey::Code(KeyCode::KeyQ), true));
        assert!(alt_graph_after(true, &at, PhysicalKey::Code(KeyCode::KeyQ), false));
        assert!(!alt_graph_after(false, &at, PhysicalKey::Code(KeyCode::KeyQ), true));
    }

    #[test]
    fn altgr_reads_as_ctrl_alt_under_the_mouse() {
        // AltGr + drag orbits, AltGr + Shift pans (ToolMode::from_mods)
        assert_eq!(mouse_alt_ctrl(false, false, true), (true, true));
        assert_eq!(mouse_alt_ctrl(false, false, false), (false, false));
        assert_eq!(mouse_alt_ctrl(true, false, false), (true, false));
        assert_eq!(mouse_alt_ctrl(true, true, false), (true, true));
    }

    #[test]
    fn altgr_keeps_existing_shortcuts_and_text_modifiers() {
        for ctrl in [false, true] {
            for alt in [false, true] {
                for shift in [false, true] {
                    let raw = Mods { ctrl, alt, shift };
                    for text_owner in [false, true] {
                        assert_eq!(keyboard_mods(raw, false, text_owner, false), raw);
                        assert_eq!(keyboard_mods(raw, true, true, false), raw, "text/web/media owns the keyboard");
                        let expected = Mods {
                            ctrl: true,
                            alt: true,
                            shift,
                        };
                        assert_eq!(keyboard_mods(raw, true, false, false), expected);
                        assert_eq!(keyboard_mods(raw, true, text_owner, true), expected, "capture reads Ctrl+Alt");
                    }
                }
            }
        }
    }

    #[test]
    fn altgr_characters_and_dead_keys_yield_to_text_but_function_keys_keep_their_shortcuts() {
        for text in ["@", "€", "{", "}", "\\"] {
            let logical = Key::Character(text.into());
            assert!(produces_text(&logical, Some(text)));
            let mods = keyboard_mods(Mods::default(), true, produces_text(&logical, Some(text)), false);
            assert_eq!(mods, Mods::default(), "{text} belongs to the focused field");
        }
        assert!(produces_text(&Key::Dead(Some('^')), None));
        assert!(!produces_text(&Key::Named(NamedKey::Enter), Some("\r")));
        let f9 = Key::Named(NamedKey::F9);
        assert!(!produces_text(&f9, None));
        let mods = keyboard_mods(Mods::default(), true, produces_text(&f9, None), false);
        assert!(mods.ctrl && mods.alt, "a field does not prevent AltGr+F9");
    }

    #[test]
    fn altgr_captures_the_same_binding_and_conflicts_as_ctrl_alt() {
        for slot in 0..2 {
            for shift in [false, true] {
                let mut options = OptionsUi::default();
                options.capture = Some(Capture::Binding(Action::DevToolbox, slot));
                assert_eq!(options.capture_key(KeyCode::AltRight, true, false), None);
                let input = options.capture_key(KeyCode::F9, true, false).flatten().expect("captured F9");
                let mods = keyboard_mods(
                    Mods {
                        shift,
                        ..Default::default()
                    },
                    true,
                    true,
                    options.capture.is_some(),
                );
                assert!(options.take_capture() == Some(Capture::Binding(Action::DevToolbox, slot)));
                let captured = Binding::captured(input.clone(), mods);
                let ctrl_alt = Binding::captured(
                    input,
                    Mods {
                        ctrl: true,
                        alt: true,
                        shift,
                    },
                );
                assert_eq!(captured, ctrl_alt);
                let mut keys = KeyBindings::default();
                keys.set(Action::DevToolbox, slot, Some(captured));
                assert!(keys.users(&ctrl_alt).contains(&Action::DevToolbox));
                let saved = serde_json::to_vec(&keys).expect("save synthetic shortcuts");
                let loaded: KeyBindings = serde_json::from_slice(&saved).expect("reload synthetic shortcuts");
                assert_eq!(loaded.get(Action::DevToolbox)[slot], Some(ctrl_alt));
                assert_eq!(loaded.triggered(&Input::key(KeyCode::F9), mods), Some(Action::DevToolbox));
            }
        }
    }

    #[test]
    fn altgr_runs_every_existing_ctrl_alt_binding_with_the_same_priority() {
        let keys = KeyBindings::default();
        for (_, slots) in &keys.map {
            for binding in slots.iter().flatten().filter(|b| b.ctrl && b.alt) {
                let raw = Mods {
                    shift: binding.shift,
                    ..Default::default()
                };
                let altgr = keyboard_mods(raw, true, false, false);
                let ctrl_alt = Mods {
                    ctrl: true,
                    alt: true,
                    shift: binding.shift,
                };
                assert_eq!(keys.triggered(&binding.input, altgr), keys.triggered(&binding.input, ctrl_alt));
                let down = HashSet::from([binding.input.clone()]);
                for action in crate::keybinds::all_actions() {
                    assert_eq!(keys.held(action, &down, altgr), keys.held(action, &down, ctrl_alt), "{action:?}");
                }
            }
        }
        let down = HashSet::from([Input::key(KeyCode::ArrowUp)]);
        let mods = keyboard_mods(Mods::default(), true, false, false);
        assert!(keys.held(Action::CamOrbitOver, &down, mods));
        assert!(!keys.held(Action::Forward, &down, mods), "the camera keeps the arrow from walking");
    }

    #[test]
    fn an_altgr_command_never_leaves_its_key_held_after_releasing_altgr() {
        let mut keys = KeyBindings::default();
        let input = Input::key(KeyCode::ArrowUp);
        let mods = keyboard_mods(Mods::default(), true, false, false);
        keys.set(Action::DevToolbox, 0, Some(Binding::captured(input.clone(), mods)));
        let mut down = HashSet::new();
        assert_eq!(keys.update_held_input(&mut down, &input, true, mods), Some(Action::DevToolbox));
        assert!(
            !keys.held(Action::Forward, &down, Mods::default()),
            "AltGr released before the arrow"
        );
        assert_eq!(keys.update_held_input(&mut down, &input, false, Mods::default()), None);
        assert!(down.is_empty());
    }
}
