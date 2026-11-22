//! Command-click on the world as a right click on macOS (#2299), as in the
//! Legacy Viewer (`mouseDown:` and `mouseUp:`, mSimulatedRightClick,
//! indra/llwindow/llopenglview-objc.mm, originally LGPL 2.1). The press is
//! turned into the right button's before anything reads it. Command is no
//! Control for the mouse (LLKeyboardMacOSX::currentMask): the click carries
//! no modifier and does not extend a menu's selection.
//!
//! Deliberate differences: our windows keep Command-click, which picks
//! several rows in their lists (the Legacy Viewer's use Control there), and
//! so do the poser and the animation editor, and a HUD while Command is the
//! key that moves HUDs; Caps Lock and Fn do not turn the right click off.

use super::App;
use winit::event::{ElementState, MouseButton, WindowEvent};

/// The right click a Command-click became, from its press to its release:
/// Command may be let go before the button.
#[derive(Debug, Default)]
pub(super) struct CommandClick {
    down: bool,
}

impl CommandClick {
    /// Whether this button event is the right button's. `starts`: a left
    /// press made now would be a Command-click. A release follows its press.
    /// A release that never came (focus lost) is forgotten at the next press.
    fn takes(&mut self, button: MouseButton, pressed: bool, starts: impl FnOnce() -> bool) -> bool {
        if button != MouseButton::Left {
            return false;
        }
        if pressed {
            self.down = starts();
            self.down
        } else {
            std::mem::take(&mut self.down)
        }
    }
}

/// Command and no other modifier, as the Legacy Viewer asks.
fn command_alone(command: bool, ctrl: bool, shift: bool, alt: bool) -> bool {
    command && !ctrl && !shift && !alt
}

impl App {
    /// The event as the viewer reads it: a Command-click on the world is the
    /// right button's. (`App::command` is never set off macOS.)
    pub(super) fn read_command_click(&mut self, event: WindowEvent) -> WindowEvent {
        let WindowEvent::MouseInput { device_id, state, button } = event else {
            return event;
        };
        let mut click = std::mem::take(&mut self.command_click);
        let taken = click.takes(button, state == ElementState::Pressed, || {
            command_alone(self.command, self.ctrl, self.shift, self.alt) && !self.command_is_taken()
        });
        self.command_click = click;
        WindowEvent::MouseInput {
            device_id,
            state,
            button: if taken { MouseButton::Right } else { button },
        }
    }

    /// Whether Command with the left button already has a use here: the
    /// right button already orbits, the poser and the animation editor (their
    /// parts and timeline), a window under the pointer, a HUD while Command
    /// is the key that moves HUDs.
    fn command_is_taken(&self) -> bool {
        self.right_drag
            || self.ik_editor.open
            || self.ui_takes_pointer()
            || (self.hud_drag_requested() && self.huds_shown() && self.scene.hud_move_pick(&self.world, self.cursor_pos).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: MouseButton = MouseButton::Left;

    #[test]
    fn a_command_click_is_the_right_button_from_press_to_release() {
        let mut click = CommandClick::default();
        assert!(click.takes(LEFT, true, || true));
        // (Command let go first: the release does not ask again)
        assert!(click.takes(LEFT, false, || unreachable!("asked on a release")));
        assert!(!click.takes(LEFT, false, || true), "one release for one press");
    }

    #[test]
    fn a_plain_click_stays_left_when_command_comes_after_the_press() {
        let mut click = CommandClick::default();
        assert!(!click.takes(LEFT, true, || false));
        assert!(!click.takes(LEFT, false, || true));
    }

    #[test]
    fn a_lost_release_does_not_reach_the_next_click() {
        let mut click = CommandClick::default();
        assert!(click.takes(LEFT, true, || true));
        assert!(!click.takes(LEFT, true, || false));
        assert!(!click.takes(LEFT, false, || true));
    }

    #[test]
    fn the_other_buttons_are_left_alone() {
        let mut click = CommandClick::default();
        assert!(click.takes(LEFT, true, || true));
        for button in [MouseButton::Right, MouseButton::Middle, MouseButton::Back] {
            assert!(!click.takes(button, true, || true));
            assert!(!click.takes(button, false, || true));
        }
        assert!(click.takes(LEFT, false, || false), "still the Command-click's release");
    }

    #[test]
    fn only_command_alone_makes_a_right_click() {
        assert!(command_alone(true, false, false, false));
        assert!(!command_alone(false, false, false, false));
        assert!(!command_alone(false, true, false, false), "Control-click is not Command-click");
        assert!(!command_alone(true, true, false, false));
        assert!(!command_alone(true, false, true, false));
        assert!(!command_alone(true, false, false, true));
    }
}
