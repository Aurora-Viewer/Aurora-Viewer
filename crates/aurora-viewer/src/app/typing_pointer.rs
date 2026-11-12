//! The mouse pointer hidden while a text field takes typing, shown again when
//! the mouse moves. Port of `LLWindow::hideCursorUntilMouseMove`, called by
//! `LLLineEditor` and `LLTextEditor` when a key adds characters
//! (indra/llui/lllineeditor.cpp, indra/llui/lltexteditor.cpp, indra/llwindow/
//! llwindowwin32.cpp, llwindowmacosx.cpp, llwindowsdl.cpp, originally LGPL 2.1).
//!
//! On Windows the Legacy Viewer follows the system setting "Hide pointer while
//! typing" (`SPI_GETMOUSEVANISH`, default on); macOS and Linux hide it
//! whatever the system says. The pointer is hidden through the platform output
//! of the frame (`CursorIcon::None`), like the camera's hold, so that egui
//! shows it again by itself when the hide ends.

use super::App;
use winit::event::{ElementState, Ime, WindowEvent};

/// How far (px) the pointer must move from where it was hidden to count as
/// moved: the system repeats the last position now and then.
const MOVE_PX: f32 = 0.5;

/// Where the pointer was hidden by typing, if it is.
#[derive(Debug, Default)]
pub(super) struct TypingPointer {
    at: Option<(f32, f32)>,
}

impl TypingPointer {
    pub(super) fn hidden(&self) -> bool {
        self.at.is_some()
    }

    /// A character was typed with the pointer at `cursor`. `enabled` is asked
    /// only when the pointer is shown, so a system setting that changes is
    /// followed at the next hide. True when the pointer has just vanished.
    pub(super) fn typed(&mut self, cursor: (f32, f32), enabled: impl FnOnce() -> bool) -> bool {
        if self.at.is_some() || !enabled() {
            return false;
        }
        self.at = Some(cursor);
        true
    }

    /// The pointer is now at `to`. True when that shows it again.
    pub(super) fn moved(&mut self, to: (f32, f32)) -> bool {
        match self.at {
            Some(from) if (to.0 - from.0).abs() > MOVE_PX || (to.1 - from.1).abs() > MOVE_PX => self.show(),
            _ => false,
        }
    }

    /// Show the pointer whatever it did. True when it was hidden.
    pub(super) fn show(&mut self) -> bool {
        self.at.take().is_some()
    }
}

/// Whether typing hides the pointer on this system.
#[cfg(windows)]
fn vanish_enabled() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETMOUSEVANISH, SystemParametersInfoW};
    // (a Win32 BOOL, four bytes: not a Rust bool)
    let mut on: i32 = 0;
    // SAFETY: `on` is a live four-byte BOOL, the size SPI_GETMOUSEVANISH
    // writes through the pointer; uiParam and fWinIni are unused for a query.
    let read = unsafe { SystemParametersInfoW(SPI_GETMOUSEVANISH, 0, (&mut on as *mut i32).cast(), 0) };
    // (a failed query counts as on, like LLWindowWin32's constructor)
    read == 0 || on != 0
}

/// macOS and Linux hide it whatever the system says (LLWindowMacOSX and
/// LLWindowSDL::hideCursorUntilMouseMove).
#[cfg(not(windows))]
fn vanish_enabled() -> bool {
    true
}

impl App {
    /// Once per window event, before the others: typing hides the pointer, a
    /// move, the focus or the window's edge show it.
    pub(super) fn typing_pointer_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event: ke, .. }
                if ke.state == ElementState::Pressed && super::alt_graph::produces_text(&ke.logical_key, ke.text.as_deref()) =>
            {
                self.hide_pointer_for_typing();
            }
            // (a composition in progress, as LLTextEditor::updatePreedit)
            WindowEvent::Ime(Ime::Commit(_)) => self.hide_pointer_for_typing(),
            WindowEvent::Ime(Ime::Preedit(text, _)) if !text.is_empty() => self.hide_pointer_for_typing(),
            WindowEvent::CursorMoved { position, .. } => {
                self.typing_pointer.moved((position.x as f32, position.y as f32));
            }
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(_) => {
                self.typing_pointer.show();
            }
            _ => {}
        }
    }

    fn hide_pointer_for_typing(&mut self) {
        // the camera's hold owns the pointer; a scripted demo must not hide
        // the real one of someone using the PC (AURORA_DEMO_CLICKS)
        if self.cursor_hold.held() || self.clicks.scripted() || !self.egui_ctx.text_edit_focused() {
            return;
        }
        let Some(g) = &mut self.gfx else {
            return;
        };
        if self.typing_pointer.typed(self.cursor_pos, vanish_enabled) {
            // the windows stop seeing it: a hover under it goes
            g.egui_state.egui_input_mut().events.push(egui::Event::PointerGone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_hides_the_pointer_when_the_system_allows_it() {
        let mut p = TypingPointer::default();
        assert!(p.typed((10.0, 10.0), || true));
        assert!(p.hidden());
    }

    #[test]
    fn typing_keeps_the_pointer_when_the_system_forbids_it() {
        let mut p = TypingPointer::default();
        assert!(!p.typed((10.0, 10.0), || false));
        assert!(!p.hidden());
    }

    #[test]
    fn a_hidden_pointer_does_not_ask_the_system_again() {
        let mut p = TypingPointer::default();
        assert!(p.typed((10.0, 10.0), || true));
        assert!(!p.typed((10.0, 10.0), || panic!("asked while hidden")));
        assert!(p.hidden());
    }

    #[test]
    fn the_system_repeating_the_position_does_not_show_it() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), || true);
        assert!(!p.moved((10.0, 10.0)));
        assert!(!p.moved((10.3, 10.2)));
        assert!(p.hidden());
    }

    #[test]
    fn a_real_move_shows_it_once() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), || true);
        assert!(p.moved((14.0, 10.0)));
        assert!(!p.hidden());
        assert!(!p.moved((20.0, 10.0)));
    }

    #[test]
    fn the_focus_or_the_edge_shows_it() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), || true);
        assert!(p.show());
        assert!(!p.show());
        assert!(!p.hidden());
    }

    #[test]
    fn it_can_hide_again_after_a_move() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), || true);
        p.moved((50.0, 50.0));
        assert!(p.typed((50.0, 50.0), || true));
    }
}
