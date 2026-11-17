//! The Mouse Camera: the middle button held over the world takes the
//! camera, focused on the clicked point, until it is released.
//!
//!   drag            orbit left / right, zoom in / out
//!   Ctrl + drag     orbit on both axes
//!   + left button   orbit on both axes
//!   + right button  pan (no context menu while held)
//!
//! Port of Catznip's Settings-MouseCam patch (Kitty Barnett;
//! LLToolPie::handleMiddleMouseDown / handleMiddleClickPick,
//! indra/newview/lltoolpie.cpp; LLToolCamera::pickCallback, handleHover and
//! handleMouseUp, indra/newview/lltoolfocus.cpp), originally LGPL 2.1. It is
//! a rebindable action (`Action::MouseCam`), unbound by default: Preferences ›
//! Mouse and keyboard gives it the middle button and Ctrl + it, taking the
//! button from push-to-talk, where Catznip turns itself off whenever
//! push-to-talk is on the middle button. It reuses the alt-camera's focus and
//! drags: the plain drag is its zoom drag, the others its orbit and pan.
//!
//! Unlike Catznip, a press on the ground takes the camera too, not only one
//! on an object, and the left and right buttons pressed during the grab only
//! change the drag: their releases do not end it, and a release that comes
//! after the grab ended is not a click either.

use super::*;
use crate::camera::ToolMode;
use crate::keybinds::Input;

/// The bound input holding the camera, and the other buttons pressed since.
#[derive(Debug, Default)]
pub(super) struct MouseCam {
    input: Option<Input>,
    left: bool,
    right: bool,
}

impl MouseCam {
    pub(super) fn active(&self) -> bool {
        self.input.is_some()
    }

    /// The input that took the camera is let go.
    pub(super) fn released_by(&self, input: &Input) -> bool {
        self.input.as_ref() == Some(input)
    }

    /// A left or right button event that belongs to the grab: pressed or
    /// released while it lasts, or the release of one pressed during it.
    pub(super) fn swallows(&mut self, button: MouseButton, pressed: bool) -> bool {
        let held = match button {
            MouseButton::Left => &mut self.left,
            MouseButton::Right => &mut self.right,
            _ => return false,
        };
        if self.input.is_some() {
            *held = pressed;
            return true;
        }
        if !pressed && *held {
            *held = false;
            return true;
        }
        false
    }

    /// The drag the mouse makes now (LLToolCamera::handleHover with the
    /// buttons as MASK_ORBIT / MASK_PAN).
    pub(super) fn tool_mode(&self, ctrl: bool, shift: bool) -> ToolMode {
        tool_mode(self.left, self.right, ctrl, shift)
    }
}

fn tool_mode(left: bool, right: bool, ctrl: bool, shift: bool) -> ToolMode {
    if right {
        ToolMode::Pan
    } else {
        // plain: the zoom drag (horizontal turns, vertical zooms)
        ToolMode::from_mods(ctrl || left, shift)
    }
}

impl App {
    /// The bound input is pressed (mouse_cam, KEYSTATE_DOWN): only over the
    /// world, out of mouselook, with no other button or camera drag going.
    pub(super) fn start_mouse_cam(&mut self, input: Input) {
        if self.mouse_cam.active()
            || self.camera.mouselook()
            || self.mouse_mode != MouseMode::None
            || self.left_down
            || self.right_drag
            || self.hud_drag.is_some()
            || self.ui_takes_pointer()
        {
            return;
        }
        self.mouse_cam.input = Some(input);
        self.click(click_pick::Click::MouseCam);
    }

    /// The press with the depth under it (LLToolCamera::pickCallback): the
    /// camera focuses on the clicked point and the drag begins. A click on
    /// the sky focuses nothing and the drag does nothing until the release
    /// (LLToolCamera::handleHover without a valid click point).
    pub(super) fn mouse_cam_press(&mut self) {
        if !self.mouse_cam.active() {
            return;
        }
        let Some(picked) = self.click_pick() else {
            return;
        };
        let Some(point) = picked.visible_point() else {
            return;
        };
        let (x, y) = self.cursor_pos;
        let ray = picked.ray.unwrap_or_else(|| self.build.cam.ray(x, y));
        let now = Instant::now();
        let idx = match picked.drawn.blend.or(picked.drawn.opaque) {
            Some(d) => self.scene.record_object(d.record).map(|(idx, _)| idx),
            None => self.scene.interaction_at(&self.world, point, now, self.build.open),
        };
        let object = idx.and_then(|i| crate::camera::pick_focus_object(&self.world, i, now));
        self.camera
            .alt_focus(point, object, ray, &mut self.world.agent, &self.settings.camera);
        self.camera.begin_drag(false);
        self.mouse_mode = MouseMode::MouseCam;
        // Settings-MouseCam: the cursor comes back where it was pressed
        self.set_mouselook_grab(true);
    }

    /// The bound input is released (LLToolCamera::handleMiddleMouseUp).
    pub(super) fn end_mouse_cam(&mut self) {
        self.mouse_cam.input = None;
        if self.mouse_mode == MouseMode::MouseCam {
            self.mouse_mode = MouseMode::None;
            self.camera.end_drag();
            if !self.camera.mouselook() {
                self.set_mouselook_grab(false);
            }
        }
    }

    /// The window lost the focus: no release will come.
    pub(super) fn drop_mouse_cam(&mut self) {
        self.end_mouse_cam();
        self.mouse_cam = MouseCam::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grabbed() -> MouseCam {
        MouseCam {
            input: Input::mouse(MouseButton::Middle),
            ..Default::default()
        }
    }

    #[test]
    fn buttons_and_ctrl_choose_the_drag() {
        assert_eq!(tool_mode(false, false, false, false), ToolMode::Zoom);
        assert_eq!(tool_mode(false, false, true, false), ToolMode::Orbit);
        assert_eq!(tool_mode(true, false, false, false), ToolMode::Orbit);
        assert_eq!(tool_mode(false, true, false, false), ToolMode::Pan);
        assert_eq!(tool_mode(true, true, false, false), ToolMode::Pan);
        assert_eq!(tool_mode(false, false, true, true), ToolMode::Pan);
    }

    #[test]
    fn the_other_buttons_only_change_the_drag() {
        let mut cam = grabbed();
        assert!(cam.swallows(MouseButton::Right, true));
        assert_eq!(cam.tool_mode(false, false), ToolMode::Pan);
        assert!(cam.swallows(MouseButton::Right, false));
        assert_eq!(cam.tool_mode(false, false), ToolMode::Zoom);
        assert!(cam.swallows(MouseButton::Left, true));
        assert_eq!(cam.tool_mode(false, false), ToolMode::Orbit);
        assert!(!cam.swallows(MouseButton::Middle, false));
    }

    #[test]
    fn a_release_after_the_grab_is_not_a_click() {
        let mut cam = grabbed();
        assert!(cam.swallows(MouseButton::Left, true));
        cam.input = None;
        // its release is still the grab's, the next press is a click again
        assert!(cam.swallows(MouseButton::Left, false));
        assert!(!cam.swallows(MouseButton::Left, true));
        assert!(!cam.swallows(MouseButton::Left, false));
        assert!(!cam.swallows(MouseButton::Right, false));
    }

    #[test]
    fn only_the_input_that_took_the_camera_lets_it_go() {
        let cam = grabbed();
        assert!(cam.active());
        assert!(cam.released_by(&Input::mouse(MouseButton::Middle).expect("bindable")));
        assert!(!cam.released_by(&Input::mouse(MouseButton::Back).expect("bindable")));
        assert!(!MouseCam::default().active());
    }
}
