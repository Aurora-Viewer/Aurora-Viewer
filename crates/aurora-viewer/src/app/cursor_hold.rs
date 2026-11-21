//! The system cursor while the camera holds it: hidden and locked during
//! mouselook (LLToolGun) and during the camera drags (LLToolCamera), given
//! back whatever ends the hold. Alt held in mouselook gives it back for as
//! long as it is held (LLToolCompGun::handleHover, indra/newview/lltoolcomp.cpp,
//! originally LGPL 2.1).
//!
//! While held the cursor is the tool's (the mouse capture of LLToolGun and
//! LLToolCamera): the windows neither see it hover, nor get its clicks or its
//! wheel. Windows and X11 cannot lock it, only confine it to the window, so
//! mouselook brings it back to the centre whenever it moves
//! (LLToolGun::handleHover → LLViewerWindow::moveCursorToCenter,
//! indra/newview/lltoolgun.cpp): it never drifts over a window (#1343).
//!
//! Wayland only moves the cursor while it is locked, as a hint the
//! compositor applies when the lock ends, once a surface commit carried it.
//! So the cursor is moved before the lock is released, and the release
//! waits for a frame to be presented: without it the cursor stayed where it
//! was pressed instead of coming back at the centre after an Alt+click. The
//! compositor moves it a moment after the release, so it is shown only once
//! it is reported at its new place (or after [`SHOW_WAIT`]): shown with the
//! release, it flashed at the press position for a frame or two.

use super::App;
use std::time::{Duration, Instant};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::KeyCode;
use winit::window::CursorGrabMode;

/// Wayland: how long a released cursor stays hidden at most while waiting
/// for the compositor to report it at its new place (a compositor that
/// ignored the hint shows it where it is after this).
const SHOW_WAIT: Duration = Duration::from_millis(150);

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
    /// Wayland: the frame of a give-back whose cursor move is still a hint;
    /// the lock is released once a frame after it presented that hint.
    unlock_after: Option<(u64, (f32, f32))>,
    /// Wayland: released and still hidden, until the cursor is reported near
    /// this point or the deadline passes.
    show_at: Option<((f32, f32), Instant)>,
}

impl CursorHold {
    pub(super) fn held(&self) -> bool {
        self.kind.is_some()
    }

    /// Whether the cursor must stay hidden: held, or given back on Wayland
    /// and not yet at its new place. egui shows the cursor again whenever
    /// its icon changes, so its icon stays None until then.
    pub(super) fn hidden(&self) -> bool {
        self.held() || self.unlock_after.is_some() || self.show_at.is_some()
    }

    /// Take the cursor; None when it is already held (a drag that turns into
    /// mouselook keeps its first hold). `focus`: the camera drag began on a
    /// valid focus point.
    pub(super) fn take(&mut self, mouselook: bool, focus: bool, cursor: (f32, f32)) -> Option<HoldKind> {
        // a hold asked after a postponed give-back keeps the cursor, and
        // so does one asked while a Wayland release waits for its frame
        self.pending = None;
        self.unlock_after = None;
        self.show_at = None;
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
        let kind = self.kind.take()?;
        Some(match kind {
            _ if focus_lost => Return::Stay,
            HoldKind::Mouselook | HoldKind::Focus => Return::Center,
            HoldKind::Drag { origin } => Return::At(origin),
        })
    }

    /// Wayland: the cursor was given back at `frame` and moved by a hint to
    /// `target`; the lock waits for [`Self::unlock_due`].
    fn unlock_later(&mut self, frame: u64, target: (f32, f32)) {
        self.unlock_after = Some((frame, target));
    }

    /// Whether the waiting release is due at `frame`: two frames later, so
    /// that a whole frame, and the surface commit of its present, came
    /// between the hint and the release. When due, the cursor waits hidden
    /// for [`Self::show_due`].
    fn unlock_due(&mut self, frame: u64, now: Instant) -> bool {
        match self.unlock_after {
            Some((at, target)) if frame >= at + 2 => {
                self.unlock_after = None;
                self.show_at = Some((target, now + SHOW_WAIT));
                true
            }
            _ => false,
        }
    }

    /// Whether the released cursor may be shown: reported at `pos` near its
    /// target (the compositor moved it), or the wait is over. Clears it.
    fn show_due(&mut self, pos: (f32, f32), now: Instant) -> bool {
        match self.show_at {
            Some(((x, y), deadline)) if ((pos.0 - x).abs() <= 2.0 && (pos.1 - y).abs() <= 2.0) || now >= deadline => {
                self.show_at = None;
                true
            }
            _ => false,
        }
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
        if kind == HoldKind::Mouselook {
            // LLToolGun::handleSelect: the hidden cursor waits at the centre,
            // not over the button that started mouselook
            self.move_cursor_to_center();
        }
        let Some(g) = &self.gfx else {
            return;
        };
        if let Err(e) = g
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined))
        {
            log::warn!("camera: the cursor could not be locked ({e}), it is only hidden");
        }
        g.window.set_cursor_visible(false);
        // the windows stop seeing it now: a window it was over loses its hover
        if let Some(g) = &mut self.gfx {
            g.egui_state.egui_input_mut().events.push(egui::Event::PointerGone);
        }
    }

    /// The hidden mouselook cursor moved (it can only be confined): back to
    /// the centre, like LLToolGun::handleHover. The view turns from the raw
    /// motion, which the move back does not produce.
    pub(super) fn recenter_mouselook_cursor(&mut self) {
        if self.cursor_hold.kind != Some(HoldKind::Mouselook) {
            return;
        }
        let Some(g) = &self.gfx else {
            return;
        };
        let size = g.window.inner_size();
        let center = (size.width as f32 / 2.0, size.height as f32 / 2.0);
        if (self.cursor_pos.0 - center.0).abs() > 0.5 || (self.cursor_pos.1 - center.1).abs() > 0.5 {
            self.move_cursor(center);
        }
    }

    /// Give the cursor back. `focus_lost`: like LLViewerWindow::handleFocusLost,
    /// shown and released where it is; mouselook takes it again when the
    /// window has the focus back.
    pub(super) fn release_cursor(&mut self, focus_lost: bool) {
        if focus_lost && (self.cursor_hold.unlock_after.take().is_some() || self.cursor_hold.show_at.take().is_some()) {
            // the window goes: no frame to wait for, release it now
            self.unlock_cursor();
        }
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
        let Some(g) = &self.gfx else {
            return;
        };
        if back != Return::Stay && wayland(&g.window) {
            // moved while still locked: the only move Wayland allows
            match back {
                Return::Center => self.move_cursor_to_center(),
                Return::At((x, y)) => self.move_cursor((x, y)),
                Return::Stay => {}
            }
            self.cursor_hold.unlock_later(self.frame_count, self.cursor_pos);
            return;
        }
        let _ = g.window.set_cursor_grab(CursorGrabMode::None);
        match back {
            Return::Stay => {}
            Return::Center => self.move_cursor_to_center(),
            Return::At((x, y)) => self.move_cursor((x, y)),
        }
        self.unlock_cursor();
    }

    /// Release the lock and show the cursor.
    fn unlock_cursor(&mut self) {
        if let Some(g) = &self.gfx {
            let _ = g.window.set_cursor_grab(CursorGrabMode::None);
            g.window.set_cursor_visible(true);
        }
    }

    /// The give-back the frame postponed, once it has the window back, and
    /// the Wayland release once its cursor hint was presented.
    pub(super) fn release_postponed_cursor(&mut self) {
        if let Some(focus_lost) = self.cursor_hold.pending {
            self.release_cursor(focus_lost);
        }
        let now = Instant::now();
        if let Some(g) = &self.gfx
            && self.cursor_hold.unlock_due(self.frame_count, now)
        {
            // still hidden: the compositor moves it a moment later
            let _ = g.window.set_cursor_grab(CursorGrabMode::None);
        }
        self.show_released_cursor(now);
    }

    /// Wayland: show the released cursor once it is at its new place (each
    /// cursor move, and each frame for the deadline).
    pub(super) fn show_released_cursor(&mut self, now: Instant) {
        if let Some(g) = &self.gfx
            && self.cursor_hold.show_due(self.cursor_pos, now)
        {
            g.window.set_cursor_visible(true);
        }
    }

    fn move_cursor_to_center(&mut self) {
        let Some(g) = &self.gfx else {
            return;
        };
        let size = g.window.inner_size();
        self.move_cursor((size.width as f32 / 2.0, size.height as f32 / 2.0));
    }

    fn move_cursor(&mut self, (x, y): (f32, f32)) {
        let Some(g) = &self.gfx else {
            return;
        };
        let _ = g
            .window
            .set_cursor_position(winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)));
        self.cursor_pos = (x, y);
    }
}

/// Whether the window is a Wayland surface, where the cursor can only be
/// moved while locked.
fn wayland(window: &winit::window::Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wayland_release_waits_for_a_presented_frame() {
        let mut hold = CursorHold::default();
        hold.take(false, true, (40.0, 50.0));
        assert_eq!(hold.give_back(false), Some(Return::Center));
        let t = Instant::now();
        hold.unlock_later(10, (500.0, 300.0));
        // the frame that carries the hint has not been presented yet
        assert!(!hold.unlock_due(10, t));
        assert!(!hold.unlock_due(11, t));
        assert!(hold.hidden());
        assert!(hold.unlock_due(12, t));
        assert!(!hold.held());
        assert!(hold.hidden());
        // once only
        assert!(!hold.unlock_due(13, t));
        // released but hidden while still reported at the press position
        assert!(!hold.show_due((40.0, 50.0), t));
        assert!(hold.show_due((500.5, 299.0), t));
        assert!(!hold.hidden());
        assert!(!hold.show_due((500.0, 300.0), t));
        // a new hold before it is due keeps the cursor locked and hidden
        hold.unlock_later(20, (500.0, 300.0));
        hold.take(false, true, (40.0, 50.0));
        assert!(!hold.unlock_due(30, t));
        assert!(!hold.show_due((500.0, 300.0), t + SHOW_WAIT));
    }

    #[test]
    fn a_wayland_cursor_the_compositor_did_not_move_is_shown_after_the_wait() {
        let mut hold = CursorHold::default();
        let t = Instant::now();
        hold.unlock_later(0, (500.0, 300.0));
        assert!(hold.unlock_due(2, t));
        assert!(!hold.show_due((40.0, 50.0), t + SHOW_WAIT / 2));
        assert!(hold.show_due((40.0, 50.0), t + SHOW_WAIT));
    }

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
