//! Characters beyond U+FFFF (most emoji: 😀 👍 🔥) typed as two key presses.
//!
//! Remote desktops (Chrome Remote Desktop's "send text", RDP clients), on-
//! screen keyboards and text-injection tools type with `SendInput` and
//! `KEYEVENTF_UNICODE`: one `VK_PACKET` press per UTF-16 unit. A character
//! beyond U+FFFF is two units, a surrogate pair, so it arrives as two
//! presses, each with a `WM_CHAR` of one half. winit 0.30 builds each key
//! event's text from its own `WM_CHAR`s only (platform_impl/windows/
//! keyboard.rs, `finalize`); a lone half is not valid UTF-16, so both
//! presses come without text and the character is lost. Characters of the
//! Basic Multilingual Plane (❤️ is U+2764 and U+FE0F) are one unit each and
//! were never affected.
//!
//! winit's message hook sees each message before winit does: the two halves
//! are joined here when key messages came between them (one press each),
//! and the character is given to the key event winit sends for the second
//! press ([`attach`]), so it reaches the text field in its place among the
//! other characters. A pair winit joins itself (one press, two `WM_CHAR`s
//! in a row, as some keyboard layouts send) is left to winit.
//!
//! Like the Legacy Viewer: LLWindowWin32's `WM_CHAR`
//! (indra/llwindow/llwindowwin32.cpp) hands each unit to
//! `LLWindow::handleUnicodeUTF16` (indra/llwindow/llwindow.cpp, originally
//! LGPL 2.1), which keeps a high half until the next unit, whatever press
//! it came with. Unlike it, a lone half is dropped rather than typed.

/// A message of the window's queue, as far as the join needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    /// `WM_CHAR` / `WM_SYSCHAR` with its UTF-16 unit.
    Char(u16),
    /// `WM_KEYDOWN`, `WM_KEYUP` and their `SYS` versions.
    Key,
}

/// The high half of a pair waiting for its low half.
#[derive(Debug, Default)]
struct Pairs {
    high: Option<u16>,
    /// A key message came after the high half: the halves belong to two
    /// presses, which winit does not join.
    split: bool,
}

impl Pairs {
    /// One message; the character the two halves make when winit drops it.
    fn message(&mut self, msg: Msg) -> Option<char> {
        match msg {
            Msg::Key => {
                self.split |= self.high.is_some();
                None
            }
            Msg::Char(unit @ 0xD800..=0xDBFF) => {
                self.high = Some(unit);
                self.split = false;
                None
            }
            Msg::Char(unit @ 0xDC00..=0xDFFF) => {
                let high = self.high.take().filter(|_| std::mem::take(&mut self.split))?;
                char::decode_utf16([high, unit]).next()?.ok()
            }
            Msg::Char(_) => {
                self.high = None;
                self.split = false;
                None
            }
        }
    }
}

/// Only a text-less VK_PACKET press from the window that received both
/// halves may get the reconstructed character. Focus changes, pointer
/// presses and regular keys interrupt the sequence.
#[derive(Default)]
struct PacketInput {
    window: Option<u64>,
    pairs: Pairs,
    pending: Option<char>,
}

impl PacketInput {
    fn message(&mut self, window: u64, msg: Msg) {
        if self.window != Some(window) {
            if !matches!(msg, Msg::Key) {
                return;
            }
            *self = Self::default();
            self.window = Some(window);
        }
        self.pending = self.pairs.message(msg);
    }

    fn cancel(&mut self, window: u64) {
        if self.window == Some(window) {
            *self = Self::default();
        }
    }

    fn take_for(&mut self, window: u64, packet_press: bool) -> Option<char> {
        if self.window != Some(window) {
            return None;
        }
        self.pending.take().filter(|_| packet_press)
    }
}

#[cfg(windows)]
thread_local! {
    static INPUT: std::cell::RefCell<PacketInput> = std::cell::RefCell::default();
}

/// Called by the message hook before winit dispatches the message.
#[cfg(windows)]
pub fn hook(msg: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_PACKET;
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP};
    // winit 0.30's Windows WindowId is the HWND converted to u64.
    let window = msg.hwnd as isize as u64;
    let message = match msg.message {
        WM_CHAR | WM_SYSCHAR => Msg::Char(msg.wParam as u16),
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP if msg.wParam == usize::from(VK_PACKET) => Msg::Key,
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
            INPUT.with_borrow_mut(|state| state.cancel(window));
            return;
        }
        _ => return,
    };
    INPUT.with_borrow_mut(|state| state.message(window, message));
}

#[cfg(windows)]
pub fn attach(window: winit::window::WindowId, event: &mut winit::event::WindowEvent) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_PACKET;
    use winit::event::{ElementState, TouchPhase, WindowEvent};
    use winit::keyboard::{Key, NativeKey};

    let window = u64::from(window);
    match event {
        WindowEvent::Focused(_)
        | WindowEvent::Destroyed
        | WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        }
        | WindowEvent::Touch(winit::event::Touch {
            phase: TouchPhase::Started,
            ..
        }) => {
            INPUT.with_borrow_mut(|state| state.cancel(window));
        }
        WindowEvent::KeyboardInput {
            event: key, is_synthetic, ..
        } => {
            let packet_press = !*is_synthetic
                && key.state == ElementState::Pressed
                && key.text.is_none()
                && key.logical_key == Key::Unidentified(NativeKey::Windows(VK_PACKET));
            if let Some(c) = INPUT.with_borrow_mut(|state| state.take_for(window, packet_press)) {
                let text = winit::keyboard::SmolStr::new(c.to_string());
                key.logical_key = Key::Character(text.clone());
                key.text = Some(text);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// The messages of `SendInput` with `KEYEVENTF_UNICODE`: a press per unit.
    fn injected(pairs: &mut Pairs, s: &str) -> Vec<char> {
        let mut out = Vec::new();
        for u in units(s) {
            pairs.message(Msg::Key);
            out.extend(pairs.message(Msg::Char(u)));
            pairs.message(Msg::Key);
        }
        out
    }

    #[test]
    fn a_pair_typed_as_two_presses_is_joined() {
        let mut pairs = Pairs::default();
        assert_eq!(injected(&mut pairs, "😀"), vec!['😀']);
        assert_eq!(injected(&mut pairs, "hi 👍🔥!"), vec!['👍', '🔥']);
    }

    #[test]
    fn plane_zero_characters_are_winits() {
        let mut pairs = Pairs::default();
        // ❤️ is U+2764 U+FE0F: winit gives both
        assert!(injected(&mut pairs, "❤️ é ♥ :heart:").is_empty());
    }

    #[test]
    fn a_pair_of_one_press_is_left_to_winit() {
        // a layout that sends both halves after one key down: winit joins them
        let mut pairs = Pairs::default();
        let [high, low] = units("😀")[..] else { panic!("a pair") };
        pairs.message(Msg::Key);
        assert_eq!(pairs.message(Msg::Char(high)), None);
        assert_eq!(pairs.message(Msg::Char(low)), None);
        pairs.message(Msg::Key);
        // and nothing is left over for the next one
        assert_eq!(injected(&mut pairs, "😀"), vec!['😀']);
    }

    #[test]
    fn a_lone_half_gives_nothing() {
        let mut pairs = Pairs::default();
        let [high, low] = units("😀")[..] else { panic!("a pair") };
        // a low half alone
        pairs.message(Msg::Key);
        assert_eq!(pairs.message(Msg::Char(low)), None);
        // a high half followed by another character
        pairs.message(Msg::Char(high));
        pairs.message(Msg::Key);
        assert_eq!(pairs.message(Msg::Char(u16::from(b'a'))), None);
        pairs.message(Msg::Key);
        assert_eq!(pairs.message(Msg::Char(low)), None);
    }

    #[test]
    fn characters_stay_with_their_window_and_packet_press() {
        let [high, low] = units("😀")[..] else { panic!("a pair") };
        let mut input = PacketInput::default();
        let start = |input: &mut PacketInput| {
            input.message(1, Msg::Key);
            input.message(1, Msg::Char(high));
            input.message(1, Msg::Key);
        };
        start(&mut input);
        input.message(2, Msg::Char(low));
        assert_eq!(input.take_for(2, true), None);
        start(&mut input);
        input.cancel(1);
        input.message(1, Msg::Char(low));
        assert_eq!(input.take_for(1, true), None);
        start(&mut input);
        input.message(1, Msg::Char(low));
        assert_eq!(input.take_for(2, true), None);
        assert_eq!(input.take_for(1, false), None);
        assert_eq!(input.take_for(1, true), None);
        start(&mut input);
        input.message(1, Msg::Char(low));
        assert_eq!(input.take_for(1, true), Some('😀'));
        assert_eq!(input.take_for(1, true), None);
    }
}
