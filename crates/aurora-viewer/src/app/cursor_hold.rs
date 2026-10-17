//! The system cursor while the camera holds it: hidden and locked during
//! mouselook (LLToolGun) and during the camera drags (LLToolCamera), given
//! back whatever ends the hold.

use super::App;
use winit::window::CursorGrabMode;

/// Why the camera took the cursor: it decides where the cursor reappears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoldKind {
    /// LLToolGun::handleSelect: the cursor goes to the centre of the window
    /// and comes back there (handleDeselect).
    Mouselook,
    /// LLToolCamera::handleMouseUp: the cursor comes back where the drag
    /// started.
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
}

impl CursorHold {
    pub(super) fn held(&self) -> bool {
        self.kind.is_some()
    }

    /// Take the cursor; None when it is already held (a drag that turns into
    /// mouselook keeps its first hold).
    pub(super) fn take(&mut self, mouselook: bool, cursor: (f32, f32)) -> Option<HoldKind> {
        // a hold asked after a postponed give-back keeps the cursor
        self.pending = None;
        if self.kind.is_some() {
            return None;
        }
        let kind = if mouselook {
            HoldKind::Mouselook
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
            HoldKind::Mouselook => Return::Center,
            HoldKind::Drag { origin } => Return::At(origin),
        })
    }
}

impl App {
    /// Hide and lock the cursor for mouselook or a camera drag (`on`), or
    /// give it back.
    pub(super) fn set_mouselook_grab(&mut self, on: bool) {
        if on {
            self.hold_cursor();
        } else {
            self.release_cursor(false);
        }
    }

    fn hold_cursor(&mut self) {
        if self.gfx.is_none() || self.clicks.scripted() {
            // AURORA_DEMO_CLICKS: several camera drags a second must not
            // take the real cursor of someone using the PC
            return;
        }
        let mouselook = self.camera.mouselook();
        let Some(kind) = self.cursor_hold.take(mouselook, self.cursor_pos) else {
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
        let Some(g) = &self.gfx else {
            return;
        };
        let _ = g.window.set_cursor_grab(CursorGrabMode::None);
        match back {
            Return::Stay => {}
            Return::Center => self.move_cursor_to_center(),
            Return::At((x, y)) => self.move_cursor((x, y)),
        }
        if let Some(g) = &self.gfx {
            g.window.set_cursor_visible(true);
        }
    }

    /// The give-back the frame postponed, once it has the window back.
    pub(super) fn release_postponed_cursor(&mut self) {
        if let Some(focus_lost) = self.cursor_hold.pending {
            self.release_cursor(focus_lost);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouselook_gives_the_cursor_back_at_the_centre() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(true, (10.0, 900.0)), Some(HoldKind::Mouselook));
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::Center));
        assert!(!hold.held());
        assert_eq!(hold.give_back(false), None);
    }

    #[test]
    fn a_drag_gives_the_cursor_back_where_it_started() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(false, (40.0, 50.0)), Some(HoldKind::Drag { origin: (40.0, 50.0) }));
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn a_second_take_keeps_the_first_hold() {
        let mut hold = CursorHold::default();
        hold.take(false, (40.0, 50.0));
        assert_eq!(hold.take(true, (0.0, 0.0)), None);
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn losing_the_focus_shows_the_cursor_where_it_is() {
        let mut hold = CursorHold::default();
        hold.take(true, (0.0, 0.0));
        assert_eq!(hold.give_back(true), Some(Return::Stay));
        // mouselook takes it again once the window has the focus back
        assert_eq!(hold.take(true, (0.0, 0.0)), Some(HoldKind::Mouselook));
    }

    #[test]
    fn a_postponed_give_back_is_kept_until_done_and_cancelled_by_a_new_hold() {
        let mut hold = CursorHold::default();
        assert!(hold.take(false, (10.0, 20.0)).is_some());
        // asked while the window was busy: still held, give-back pending
        hold.pending = Some(false);
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::At((10.0, 20.0))));
        assert_eq!(hold.pending, None);
        assert!(!hold.held());
        // a new hold before the frame gave the window back drops the pending give-back
        hold.pending = Some(true);
        assert!(hold.take(true, (0.0, 0.0)).is_some());
        assert_eq!(hold.pending, None);
    }
}
