//! AltGr under the mouse. On a keyboard layout with AltGr (French, German…)
//! Windows types the key as Ctrl + Alt, so the Legacy Viewer's mouse mask
//! holds both and AltGr + left drag orbits the camera like Ctrl + Alt
//! (LLToolCamera::handleHover, indra/newview/lltoolfocus.cpp, originally
//! LGPL 2.1). winit hides both modifiers while AltGr is down, for the
//! characters it types: the mouse alone counts them back, never the keyboard
//! shortcuts, so typing `@` runs no Ctrl + Alt shortcut.

use super::App;
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

impl App {
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
}
