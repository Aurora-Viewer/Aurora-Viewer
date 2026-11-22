//! The system cursor while the camera holds it: hidden and locked during
//! mouselook (LLToolGun) and during the camera drags (LLToolCamera), given
//! back whatever ends the hold. Alt held in mouselook gives it back for as
//! long as it is held (LLToolCompGun::handleHover, indra/newview/lltoolcomp.cpp,
//! originally LGPL 2.1).
//!
//! While held the cursor is the tool's (the mouse capture of LLToolGun and
//! LLToolCamera): the windows neither see it hover, nor get its clicks or its
//! wheel. Mouselook brings it back to the centre whenever it moves
//! (LLToolGun::handleHover → LLViewerWindow::moveCursorToCenter,
//! indra/newview/lltoolgun.cpp): X11 cannot lock it, only confine it to the
//! window, and there it never drifts over a window (#1343); on Wayland the
//! move under the lock is the hint of where it shows when given back.
//! Except on macOS: a warp there couples the cursor to the mouse again,
//! undoing the lock, and the warp back came in as an equal and opposite raw
//! motion that turned the view back where mouselook began (#3310). The
//! Legacy Viewer does not warp a decoupled cursor there either
//! (LLViewerWindow::moveCursorToCenter, isWrapMouse is !mCursorDecoupled,
//! indra/llwindow/llwindowmacosx.h).

use super::App;
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::KeyCode;
use winit::window::CursorGrabMode;

/// A pointer event the windows must not see while the camera holds the
/// cursor. A release still reaches them: a press they saw before the hold
/// ends there, and a release alone does nothing.
pub(super) fn held_from_ui(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::CursorMoved { .. } | WindowEvent::MouseWheel { .. } => true,
        WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
        _ => false,
    }
}

/// Why the camera took the cursor: it decides where the cursor reappears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoldKind {
    /// LLToolGun::handleSelect: the cursor goes to the centre of the window
    /// and comes back there (handleDeselect).
    Mouselook,
    /// LLToolCamera::handleMouseUp on a valid click point (Alt+click, focus
    /// tool): the cursor comes back at the centre, where the camera turned
    /// the clicked point, so it is still over what was clicked.
    Focus,
    /// LLToolCamera::handleMouseUp otherwise (avatar steering, a click with
    /// no point): the cursor comes back where the drag started.
    Drag { origin: (f32, f32) },
}

/// Where the cursor is put when the camera gives it back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Return {
    /// Left where it is (focus lost: LLViewerWindow::handleFocusLost only
    /// shows it again).
    Stay,
    Center,
    At((f32, f32)),
}

/// What the camera asked of the cursor. `kind` is set as soon as the cursor
/// is hidden, even when the system refused to lock it: what was hidden must
/// always be shown again (#271).
#[derive(Debug, Default)]
pub(super) struct CursorHold {
    kind: Option<HoldKind>,
    /// A give-back asked while the window was out of reach (the frame holds
    /// it while the interface is built): done as soon as the frame gives the
    /// window back, so the cursor is never left hidden and locked.
    pending: Option<bool>,
    /// macOS locked the cursor: a warp would undo the lock, so mouselook
    /// never brings it back to the centre.
    warp_unlocks: bool,
    /// The next raw motion is dropped: on macOS the first one after the lock
    /// still carries the warp to the centre that preceded it
    /// (LLWindowMacOSX::adjustCursorDecouple sets mCursorIgnoreNextDelta,
    /// indra/llwindow/llwindowmacosx.cpp).
    skip_motion: bool,
}

impl CursorHold {
    pub(super) fn held(&self) -> bool {
        self.kind.is_some()
    }

    /// The system answered the lock asked after `take`. `warped`: the cursor
    /// was just moved to the centre, a move macOS reports in the next raw
    /// motion.
    pub(super) fn locked(&mut self, locked: bool, warped: bool) {
        self.lock_answer(locked, warped, cfg!(target_os = "macos"));
    }

    /// `locked`, the system given (`macos`) so that the tests cover both.
    fn lock_answer(&mut self, locked: bool, warped: bool, macos: bool) {
        self.warp_unlocks = locked && macos;
        self.skip_motion = self.warp_unlocks && warped;
    }

    /// The held cursor moved: whether mouselook must bring it back to the
    /// centre (everywhere but a cursor macOS locked).
    pub(super) fn recenters(&self) -> bool {
        self.kind == Some(HoldKind::Mouselook) && !self.warp_unlocks
    }

    /// A raw motion arrived: false when it is the warp's and must not turn
    /// the view.
    pub(super) fn take_motion(&mut self) -> bool {
        !std::mem::take(&mut self.skip_motion)
    }

    /// Take the cursor; None when it is already held (a drag that turns into
    /// mouselook keeps its first hold). `focus`: the camera drag began on a
    /// valid focus point.
    pub(super) fn take(&mut self, mouselook: bool, focus: bool, cursor: (f32, f32)) -> Option<HoldKind> {
        // a hold asked after a postponed give-back keeps the cursor
        self.pending = None;
        if self.kind.is_some() {
            return None;
        }
        let kind = if mouselook {
            HoldKind::Mouselook
        } else if focus {
            HoldKind::Focus
        } else {
            HoldKind::Drag { origin: cursor }
        };
        self.kind = Some(kind);
        Some(kind)
    }

    /// Give the cursor back; None when it was not held. `focus_lost`: the
    /// window lost the focus, the cursor stays where the system left it.
    pub(super) fn give_back(&mut self, focus_lost: bool) -> Option<Return> {
        self.pending = None;
        self.warp_unlocks = false;
        self.skip_motion = false;
        let kind = self.kind.take()?;
        Some(match kind {
            _ if focus_lost => Return::Stay,
            HoldKind::Mouselook | HoldKind::Focus => Return::Center,
            HoldKind::Drag { origin } => Return::At(origin),
        })
    }
}

/// Alt in mouselook (LLToolCompGun::handleHover): held, the gun gives way to
/// the grab tool, the cursor comes back at the centre and moves freely over
/// the windows, the HUDs and the world while the view stays still; let go,
/// the gun takes it again. Like the Legacy Viewer, neither switch happens
/// while the left button is down: a click or a drag ends with the tool that
/// began it.
#[derive(Debug, Default)]
pub(super) struct AltCursor {
    free: bool,
    left: bool,
}

/// What the cursor must do after `AltCursor::update`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum AltSwitch {
    /// LLToolGun::handleDeselect: shown and released at the centre.
    Free,
    /// LLToolGun::handleSelect: hidden and locked at the centre again.
    Take,
}

impl AltCursor {
    /// Alt frees the cursor: the view no longer follows the mouse, a click
    /// lands under the cursor and not at the crosshair.
    pub(super) fn free(&self) -> bool {
        self.free
    }

    /// The left button of the window, pressed or released.
    pub(super) fn left_button(&mut self, pressed: bool) {
        self.left = pressed;
    }

    /// Once a frame. `mouselook`: in mouselook with the window focused (the
    /// cursor is given back otherwise); `alt`: Alt held.
    pub(super) fn update(&mut self, mouselook: bool, alt: bool) -> Option<AltSwitch> {
        if !mouselook {
            // out of mouselook or of the focus, the cursor is already shown;
            // mouselook takes it again with the focus, Alt then frees it
            self.free = false;
            return None;
        }
        if self.left || self.free == alt {
            return None;
        }
        self.free = alt;
        Some(if alt { AltSwitch::Free } else { AltSwitch::Take })
    }
}

impl App {
    pub(super) fn mouselook_capture_enabled(&self) -> bool {
        if self.vr_camera.is_some() {
            self.settings.camera.vr.mouse_look
                && matches!(
                    self.vr_mode,
                    Some(crate::camera::VrView::FirstPerson | crate::camera::VrView::Avatar)
                )
        } else {
            self.camera.mouselook()
        }
    }

    /// Once a frame, before mouselook takes a cursor it does not hold: Alt
    /// frees it or gives it back to the gun.
    pub(super) fn mouselook_alt_cursor(&mut self) {
        // (AURORA_DEMO_CAMERA scripts the keys: the window needs no focus)
        let focused = self.focused || self.demo && std::env::var_os("AURORA_DEMO_CAMERA").is_some();
        let capture = self.mouselook_capture_enabled();
        if !capture && self.cursor_hold.kind == Some(HoldKind::Mouselook) {
            self.release_cursor(false);
        }
        let mouselook = capture && focused && !self.exit_prompt.open;
        match self.alt_cursor.update(mouselook, self.alt && !self.alt_gr_alone()) {
            Some(AltSwitch::Free) => self.release_cursor(false),
            Some(AltSwitch::Take) => self.set_mouselook_grab(true),
            None => {}
        }
    }

    /// AltGr alone (right Alt, typed as Ctrl + Alt) is a character key, not
    /// the camera's Alt: typing `@` in mouselook must not free the cursor.
    pub(super) fn alt_gr_alone(&self) -> bool {
        self.down.contains(&super::Input::key(KeyCode::AltRight)) && !self.down.contains(&super::Input::key(KeyCode::AltLeft))
    }
    /// Hide and lock the cursor for mouselook or a camera drag (`on`), or
    /// give it back.
    pub(super) fn set_mouselook_grab(&mut self, on: bool) {
        if on {
            self.hold_cursor(false);
        } else {
            self.release_cursor(false);
        }
    }

    /// Hide and lock the cursor for a camera drag that focused on the clicked
    /// point: it comes back at the centre, over that point.
    pub(super) fn hold_focus_cursor(&mut self) {
        self.hold_cursor(true);
    }

    /// Hide and lock the cursor for a drag that began at `origin`: it comes
    /// back there (the right button's orbit, which only holds it once it
    /// moves past a click's jitter).
    pub(super) fn hold_drag_cursor(&mut self, origin: (f32, f32)) {
        self.hold_cursor_from(false, origin);
    }

    fn hold_cursor(&mut self, focus: bool) {
        self.hold_cursor_from(focus, self.cursor_pos);
    }

    fn hold_cursor_from(&mut self, focus: bool, origin: (f32, f32)) {
        if self.gfx.is_none() || self.clicks.scripted() {
            // AURORA_DEMO_CLICKS: several camera drags a second must not
            // take the real cursor of someone using the PC
            return;
        }
        // the camera's hold takes over from a pointer hidden by typing
        self.typing_pointer.show();
        let mouselook = self.mouselook_capture_enabled();
        let Some(kind) = self.cursor_hold.take(mouselook, focus, origin) else {
            return;
        };
        let warped = kind == HoldKind::Mouselook;
        if warped {
            // LLToolGun::handleSelect: the hidden cursor waits at the centre,
            // not over the button that started mouselook. Before the lock:
            // on macOS a warp undoes it.
            self.move_cursor_to_center();
        }
        let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) else {
            return;
        };
        let locked = win.set_cursor_grab(CursorGrabMode::Locked).is_ok();
        if !locked && let Err(e) = win.set_cursor_grab(CursorGrabMode::Confined) {
            log::warn!("camera: the cursor could not be locked ({e}), it is only hidden");
        }
        win.set_cursor_visible(false);
        self.cursor_hold.locked(locked, warped);
        // the windows stop seeing it now: a window it was over loses its hover
        if let Some(egui_state) = self.gfx.as_mut().and_then(|g| g.egui_state.as_mut()) {
            egui_state.egui_input_mut().events.push(egui::Event::PointerGone);
        }
    }

    /// The hidden mouselook cursor moved: back to the centre, like
    /// LLToolGun::handleHover. The view turns from the raw motion, which the
    /// move back does not produce outside macOS, where a locked cursor is
    /// left alone (see the module comment).
    pub(super) fn recenter_mouselook_cursor(&mut self) {
        if !self.cursor_hold.recenters() {
            return;
        }
        let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) else {
            return;
        };
        let size = win.inner_size();
        let center = (size.width as f32 / 2.0, size.height as f32 / 2.0);
        if (self.cursor_pos.0 - center.0).abs() > 0.5 || (self.cursor_pos.1 - center.1).abs() > 0.5 {
            self.move_cursor(center);
        }
    }

    /// Give the cursor back. `focus_lost`: like LLViewerWindow::handleFocusLost,
    /// shown and released where it is; mouselook takes it again when the
    /// window has the focus back.
    pub(super) fn release_cursor(&mut self, focus_lost: bool) {
        if self.gfx.is_none() {
            // the window is the frame's for now: forgetting the hold here
            // left the cursor hidden and locked for good
            if self.cursor_hold.held() {
                self.cursor_hold.pending = Some(focus_lost);
            }
            return;
        }
        let Some(back) = self.cursor_hold.give_back(focus_lost) else {
            return;
        };
        let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) else {
            return;
        };
        let _ = win.set_cursor_grab(CursorGrabMode::None);
        match back {
            Return::Stay => {}
            Return::Center => self.move_cursor_to_center(),
            Return::At((x, y)) => self.move_cursor((x, y)),
        }
        if let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) {
            win.set_cursor_visible(true);
        }
    }

    /// The give-back the frame postponed, once it has the window back.
    pub(super) fn release_postponed_cursor(&mut self) {
        if let Some(focus_lost) = self.cursor_hold.pending {
            self.release_cursor(focus_lost);
        }
    }

    fn move_cursor_to_center(&mut self) {
        let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) else {
            return;
        };
        let size = win.inner_size();
        self.move_cursor((size.width as f32 / 2.0, size.height as f32 / 2.0));
    }

    fn move_cursor(&mut self, (x, y): (f32, f32)) {
        let Some(win) = self.gfx.as_ref().and_then(|g| g.window.as_ref()) else {
            return;
        };
        let _ = win.set_cursor_position(winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)));
        self.cursor_pos = (x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouselook_gives_the_cursor_back_at_the_centre() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(true, false, (10.0, 900.0)), Some(HoldKind::Mouselook));
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::Center));
        assert!(!hold.held());
        assert_eq!(hold.give_back(false), None);
    }

    #[test]
    fn a_drag_gives_the_cursor_back_where_it_started() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(false, false, (40.0, 50.0)), Some(HoldKind::Drag { origin: (40.0, 50.0) }));
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn a_focus_drag_gives_the_cursor_back_at_the_centre() {
        // the camera turned the clicked point to the centre: the cursor
        // reappears over it, not over what is now at the press position
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(false, true, (40.0, 50.0)), Some(HoldKind::Focus));
        assert_eq!(hold.give_back(false), Some(Return::Center));
        // losing the focus still leaves it where it is
        hold.take(false, true, (40.0, 50.0));
        assert_eq!(hold.give_back(true), Some(Return::Stay));
    }

    #[test]
    fn a_second_take_keeps_the_first_hold() {
        let mut hold = CursorHold::default();
        hold.take(false, false, (40.0, 50.0));
        assert_eq!(hold.take(true, false, (0.0, 0.0)), None);
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn losing_the_focus_shows_the_cursor_where_it_is() {
        let mut hold = CursorHold::default();
        hold.take(true, false, (0.0, 0.0));
        assert_eq!(hold.give_back(true), Some(Return::Stay));
        // mouselook takes it again once the window has the focus back
        assert_eq!(hold.take(true, false, (0.0, 0.0)), Some(HoldKind::Mouselook));
    }

    #[test]
    fn alt_frees_the_mouselook_cursor_while_held() {
        let mut alt = AltCursor::default();
        assert_eq!(alt.update(true, false), None);
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        assert!(alt.free());
        // once a frame: nothing more while Alt stays held
        assert_eq!(alt.update(true, true), None);
        assert_eq!(alt.update(true, false), Some(AltSwitch::Take));
        assert!(!alt.free());
        // Alt outside mouselook is the camera's, not this
        assert_eq!(alt.update(false, true), None);
        assert!(!alt.free());
    }

    #[test]
    fn the_left_button_down_keeps_the_tool_it_began_with() {
        let mut alt = AltCursor::default();
        // a mouselook click held, then Alt: the gun keeps the cursor
        alt.left_button(true);
        assert_eq!(alt.update(true, true), None);
        alt.left_button(false);
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        // a drag begun with the free cursor, then Alt let go: free until released
        alt.left_button(true);
        assert_eq!(alt.update(true, false), None);
        assert!(alt.free());
        alt.left_button(false);
        assert_eq!(alt.update(true, false), Some(AltSwitch::Take));
    }

    #[test]
    fn leaving_mouselook_or_the_focus_ends_the_free_cursor_without_taking_it() {
        let mut alt = AltCursor::default();
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        // (Alt+Tab: the focus goes with Alt still down)
        assert_eq!(alt.update(false, true), None);
        assert!(!alt.free());
        // back with Alt still held: freed again once mouselook took it
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
    }

    #[test]
    fn only_a_cursor_macos_locked_is_left_off_the_centre() {
        // Windows and Wayland lock it, X11 only confines it: recentred as
        // before (on Wayland the move is where it shows when given back)
        for (locked, macos) in [(true, false), (false, false), (false, true)] {
            let mut hold = CursorHold::default();
            hold.take(true, false, (0.0, 0.0));
            hold.lock_answer(locked, true, macos);
            assert!(hold.recenters(), "locked {locked}, macOS {macos}");
            assert!(hold.take_motion(), "locked {locked}, macOS {macos}");
        }
        // macOS locked it: a warp would undo the lock (#3310)
        let mut hold = CursorHold::default();
        hold.take(true, false, (0.0, 0.0));
        hold.lock_answer(true, true, true);
        assert!(!hold.recenters());
        // given back and taken again (Alt), the lock refused this time
        hold.give_back(false);
        hold.take(true, false, (0.0, 0.0));
        hold.lock_answer(false, true, true);
        assert!(hold.recenters());
        // a camera drag is never recentred
        let mut drag = CursorHold::default();
        drag.take(false, false, (40.0, 50.0));
        drag.lock_answer(false, false, false);
        assert!(!drag.recenters());
    }

    #[test]
    fn the_warp_before_a_macos_lock_does_not_turn_the_view() {
        let mut hold = CursorHold::default();
        hold.take(true, false, (0.0, 0.0));
        hold.lock_answer(true, true, true);
        // the first raw motion after it carries the warp: dropped, once
        assert!(!hold.take_motion());
        assert!(hold.take_motion());
        // no warp before the lock (a camera drag): nothing is dropped
        hold.give_back(false);
        hold.take(false, false, (0.0, 0.0));
        hold.lock_answer(true, false, true);
        assert!(hold.take_motion());
        // a give-back forgets a motion still to drop
        hold.give_back(false);
        hold.take(true, false, (0.0, 0.0));
        hold.lock_answer(true, true, true);
        hold.give_back(false);
        assert!(hold.take_motion());
    }

    #[test]
    fn a_postponed_give_back_is_kept_until_done_and_cancelled_by_a_new_hold() {
        let mut hold = CursorHold::default();
        assert!(hold.take(false, false, (10.0, 20.0)).is_some());
        // asked while the window was busy: still held, give-back pending
        hold.pending = Some(false);
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::At((10.0, 20.0))));
        assert_eq!(hold.pending, None);
        assert!(!hold.held());
        // a new hold before the frame gave the window back drops the pending give-back
        hold.pending = Some(true);
        assert!(hold.take(true, false, (0.0, 0.0)).is_some());
        assert_eq!(hold.pending, None);
    }
}
