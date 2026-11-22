//! Control-click as a right click on macOS (#2299). macOS reports it as the
//! left button with Control held (winit passes `buttonNumber` on), and each
//! application makes a right click of it. The press is turned into the right
//! button's before the windows or the world read it, so that both agree.
//!
//! The Legacy Viewer turns a Command-click into a right click instead and
//! leaves Control-click to the left button (`mouseDown:`,
//! indra/llwindow/llopenglview-objc.mm, originally LGPL 2.1). A deliberate
//! difference: Control-click is what every Mac application does, and Command
//! picks several rows in our lists. The gestures that need Control with the
//! left button keep it (`control_is_taken`).

use super::App;
use winit::event::{ElementState, MouseButton, WindowEvent};

/// The right click a Control-click became, from its press to its release:
/// Control may be let go before the button.
#[derive(Debug, Default)]
pub(super) struct ControlClick {
    down: bool,
}

impl ControlClick {
    /// Whether this button event is the right button's. `starts`: a left
    /// press made now would be a Control-click. A release follows its press.
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

/// Control and no other modifier: with Shift, Option or Command it belongs to
/// the grab tool's spin, the camera or a shortcut.
fn control_alone(ctrl: bool, shift: bool, alt: bool, command: bool) -> bool {
    ctrl && !shift && !alt && !command
}

impl App {
    /// The event as the viewer reads it, and whether it is a Control-click.
    pub(super) fn read_control_click(&mut self, event: WindowEvent) -> (WindowEvent, bool) {
        if !cfg!(target_os = "macos") {
            return (event, false);
        }
        let WindowEvent::MouseInput { device_id, state, button } = event else {
            return (event, false);
        };
        let mut click = std::mem::take(&mut self.control_click);
        let taken = click.takes(button, state == ElementState::Pressed, || {
            control_alone(self.ctrl, self.shift, self.alt, self.command) && !self.control_is_taken()
        });
        self.control_click = click;
        let button = if taken { MouseButton::Right } else { button };
        (WindowEvent::MouseInput { device_id, state, button }, taken)
    }

    /// Whether Control with the left button already has a use here.
    fn control_is_taken(&self) -> bool {
        // the camera or mouselook holds the pointer, or the right button
        // already orbits
        if self.camera.mouselook() || self.cursor_hold.held() || self.right_drag {
            return true;
        }
        // the poser and the animation editor, their timeline included
        if self.ik_editor.open {
            return true;
        }
        // (in the windows Command picks several rows: nothing is lost there)
        if self.ui_takes_pointer() {
            return false;
        }
        // the build tools (selection, lift), the photo viewfinder's touch,
        // the HUD move gesture, a name plate with an action for Control
        self.build.open
            || self.viewfinder_on()
            || (self.hud_drag_requested() && self.huds_shown() && self.scene.hud_move_pick(&self.world, self.cursor_pos).is_some())
            || self.plate_cursor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: MouseButton = MouseButton::Left;

    #[test]
    fn a_control_click_is_the_right_button_from_press_to_release() {
        let mut click = ControlClick::default();
        assert!(click.takes(LEFT, true, || true));
        // (Control let go first: the release does not ask again)
        assert!(click.takes(LEFT, false, || unreachable!("asked on a release")));
        assert!(!click.takes(LEFT, false, || true), "one release for one press");
    }

    #[test]
    fn a_plain_click_stays_left_when_control_comes_after_the_press() {
        let mut click = ControlClick::default();
        assert!(!click.takes(LEFT, true, || false));
        assert!(!click.takes(LEFT, false, || true));
    }

    #[test]
    fn a_lost_release_does_not_reach_the_next_click() {
        let mut click = ControlClick::default();
        assert!(click.takes(LEFT, true, || true));
        assert!(!click.takes(LEFT, true, || false));
        assert!(!click.takes(LEFT, false, || true));
    }

    #[test]
    fn the_other_buttons_are_left_alone() {
        let mut click = ControlClick::default();
        assert!(click.takes(LEFT, true, || true));
        for button in [MouseButton::Right, MouseButton::Middle, MouseButton::Back] {
            assert!(!click.takes(button, true, || true));
            assert!(!click.takes(button, false, || true));
        }
        assert!(click.takes(LEFT, false, || false), "still the Control-click's release");
    }

    #[test]
    fn only_control_alone_makes_a_right_click() {
        assert!(control_alone(true, false, false, false));
        assert!(!control_alone(false, false, false, false));
        assert!(!control_alone(true, true, false, false), "Control+Shift spins");
        assert!(!control_alone(true, false, true, false), "Control+Option orbits");
        assert!(!control_alone(true, false, false, true));
        assert!(!control_alone(false, false, false, true), "Command-click is not Control-click");
    }
}
