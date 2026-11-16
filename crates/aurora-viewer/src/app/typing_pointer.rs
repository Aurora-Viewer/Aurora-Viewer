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

use winit::event::WindowEvent;

/// How far (px) the pointer must move from where it was hidden to count as
/// moved: the system repeats the last position now and then.
const MOVE_PX: f32 = 0.5;

/// Where the pointer was hidden by typing, if it is.
#[derive(Debug, Default)]
pub(super) struct TypingPointer {
    at: Option<((f32, f32), egui::Id)>,
}

impl TypingPointer {
    pub(super) fn hidden(&self) -> bool {
        self.at.is_some()
    }

    /// A character was typed with the pointer at `cursor`. `enabled` is asked
    /// only when the pointer is shown, so a system setting that changes is
    /// followed at the next hide. True when the pointer has just vanished.
    fn typed(&mut self, cursor: (f32, f32), field: egui::Id, enabled: impl FnOnce() -> bool) -> bool {
        if self.at.is_some_and(|(_, owner)| owner == field) {
            return false;
        }
        self.at = enabled().then_some((cursor, field));
        self.hidden()
    }

    /// The pointer is now at `to`. True when that shows it again.
    fn moved(&mut self, to: (f32, f32)) -> bool {
        match self.at {
            Some((from, _)) if (to.0 - from.0).abs() > MOVE_PX || (to.1 - from.1).abs() > MOVE_PX => self.show(),
            _ => false,
        }
    }

    /// A field can lose focus without the window losing it (Tab, Escape, or a closed floater).
    pub(super) fn hidden_for(&mut self, ctx: &egui::Context) -> bool {
        if let Some((_, field)) = self.at
            && !ctx.memory(|memory| memory.has_focus(field))
        {
            self.show();
        }
        self.hidden()
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

/// Observe the text egui-winit delivered after shortcut and IME handling.
pub(super) fn inserts_text(event: &egui::Event) -> bool {
    match event {
        egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text)) => !text.is_empty(),
        _ => false,
    }
}

impl TypingPointer {
    pub(super) fn window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.moved((position.x as f32, position.y as f32));
            }
            WindowEvent::CursorLeft { .. } | WindowEvent::Focused(_) => {
                self.show();
            }
            _ => {}
        }
    }

    pub(super) fn on_text(&mut self, ctx: &egui::Context, cursor: (f32, f32), events: &mut Vec<egui::Event>) {
        if ctx.text_edit_focused()
            && let Some(field) = ctx.memory(|memory| memory.focused())
            && self.typed(cursor, field, vanish_enabled)
        {
            events.push(egui::Event::PointerGone);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_inserted_text_and_nonempty_composition_hide_the_pointer() {
        use egui::{Event, ImeEvent};
        for text in ["@", "\u{e9}", "\u{3042}"] {
            assert!(inserts_text(&Event::Text(text.into())));
            assert!(inserts_text(&Event::Ime(ImeEvent::Preedit {
                text: text.into(),
                active_range_chars: None,
            })));
            assert!(inserts_text(&Event::Ime(ImeEvent::Commit(text.into()))));
        }
        for event in [
            Event::Copy,
            Event::Cut,
            Event::Paste("clipboard".into()),
            Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            Event::Ime(ImeEvent::Commit(String::new())),
            Event::Ime(ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }),
        ] {
            assert!(!inserts_text(&event));
        }
    }

    #[test]
    fn leaving_the_text_field_shows_the_pointer_without_mouse_motion() {
        let ctx = egui::Context::default();
        let first = egui::Id::unique("first field");
        let second = egui::Id::unique("second field");
        let mut p = TypingPointer::default();
        ctx.memory_mut(|m| m.request_focus(first));
        assert!(p.typed((10.0, 10.0), first, || true));
        assert!(p.hidden_for(&ctx));
        ctx.memory_mut(|m| m.request_focus(second));
        assert!(!p.hidden_for(&ctx));
        assert!(p.typed((10.0, 10.0), second, || true));
        ctx.memory_mut(|m| m.surrender_focus(second));
        assert!(!p.hidden_for(&ctx));
    }

    #[test]
    fn a_focused_editor_drops_hover_once_if_the_platform_enables_hiding() {
        let ctx = egui::Context::default();
        let mut draft = String::new();
        let mut pointer = TypingPointer::default();
        let mut events = Vec::new();
        pointer.on_text(&ctx, (5.0, 8.0), &mut events);
        assert!(events.is_empty());
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.text_edit_singleline(&mut draft).request_focus();
        });
        pointer.on_text(&ctx, (5.0, 8.0), &mut events);
        pointer.on_text(&ctx, (5.0, 8.0), &mut events);
        assert_eq!(pointer.hidden_for(&ctx), vanish_enabled());
        if pointer.hidden() {
            assert!(matches!(events.as_slice(), [egui::Event::PointerGone]));
        } else {
            assert!(events.is_empty());
        }
    }

    #[test]
    fn typing_hides_the_pointer_when_the_system_allows_it() {
        let mut p = TypingPointer::default();
        assert!(p.typed((10.0, 10.0), egui::Id::NULL, || true));
        assert!(p.hidden());
    }

    #[test]
    fn typing_keeps_the_pointer_when_the_system_forbids_it() {
        let mut p = TypingPointer::default();
        assert!(!p.typed((10.0, 10.0), egui::Id::NULL, || false));
        assert!(!p.hidden());
    }

    #[test]
    fn a_hidden_pointer_does_not_ask_the_system_again() {
        let mut p = TypingPointer::default();
        assert!(p.typed((10.0, 10.0), egui::Id::NULL, || true));
        assert!(!p.typed((10.0, 10.0), egui::Id::NULL, || panic!("asked while hidden")));
        assert!(p.hidden());
    }

    #[test]
    fn the_system_repeating_the_position_does_not_show_it() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), egui::Id::NULL, || true);
        assert!(!p.moved((10.0, 10.0)));
        assert!(!p.moved((10.3, 10.2)));
        assert!(p.hidden());
    }

    #[test]
    fn a_real_move_shows_it_once() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), egui::Id::NULL, || true);
        assert!(p.moved((14.0, 10.0)));
        assert!(!p.hidden());
        assert!(!p.moved((20.0, 10.0)));
    }

    #[test]
    fn the_focus_or_the_edge_shows_it() {
        let mut p = TypingPointer::default();
        for event in [
            WindowEvent::Focused(false),
            WindowEvent::Focused(true),
            WindowEvent::CursorLeft {
                device_id: winit::event::DeviceId::dummy(),
            },
        ] {
            p.typed((10.0, 10.0), egui::Id::NULL, || true);
            p.window_event(&event);
            assert!(!p.hidden());
        }
    }

    #[test]
    fn it_can_hide_again_after_a_move() {
        let mut p = TypingPointer::default();
        p.typed((10.0, 10.0), egui::Id::NULL, || true);
        p.moved((50.0, 50.0));
        assert!(p.typed((50.0, 50.0), egui::Id::NULL, || true));
    }
}
