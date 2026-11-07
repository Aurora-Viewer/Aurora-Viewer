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
#[cfg(any(windows, test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// `WM_CHAR` / `WM_SYSCHAR` with its UTF-16 unit.
    Char(u16),
    /// `WM_KEYDOWN`, `WM_KEYUP` and their `SYS` versions.
    Key,
}

/// The high half of a pair waiting for its low half.
#[cfg(any(windows, test))]
#[derive(Debug, Default)]
pub struct Pairs {
    high: Option<u16>,
    /// A key message came after the high half: the halves belong to two
    /// presses, which winit does not join.
    split: bool,
}

#[cfg(any(windows, test))]
impl Pairs {
    /// One message; the character the two halves make when winit drops it.
    pub fn message(&mut self, msg: Msg) -> Option<char> {
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

thread_local! {
    /// The character joined for the key event winit is about to send.
    static PENDING: std::cell::Cell<Option<char>> = const { std::cell::Cell::new(None) };
}

/// Called by the message hook for each message, before winit handles it.
#[cfg(windows)]
pub fn hook(pairs: &mut Pairs, msg: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP};
    let msg = match msg.message {
        WM_CHAR | WM_SYSCHAR => Msg::Char(msg.wParam as u16),
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => Msg::Key,
        _ => return,
    };
    if let Some(c) = pairs.message(msg) {
        PENDING.with(|p| p.set(Some(c)));
    }
}

/// Before the window event reaches egui: the press of the low half, which
/// winit sends without text, gets the character joined by the hook.
pub fn attach(event: &mut winit::event::WindowEvent) {
    let winit::event::WindowEvent::KeyboardInput { event: key, .. } = event else {
        return;
    };
    if let Some(text) = take_for(key.state == winit::event::ElementState::Pressed, key.text.is_some()) {
        key.logical_key = winit::keyboard::Key::Character(text.clone());
        key.text = Some(text);
    }
}

/// The joined character, taken by any key event so that it is never given
/// late: kept only by a press that came without text.
fn take_for(pressed: bool, has_text: bool) -> Option<winit::keyboard::SmolStr> {
    let c = PENDING.with(std::cell::Cell::take)?;
    (pressed && !has_text).then(|| winit::keyboard::SmolStr::new(c.to_string()))
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
    fn the_character_goes_to_the_next_press_without_text_only() {
        // nothing joined: nothing given
        assert_eq!(take_for(true, false), None);
        PENDING.with(|p| p.set(Some('😀')));
        // a release, or a press that has its own text, drops it
        assert_eq!(take_for(false, false), None);
        assert_eq!(take_for(true, false), None, "taken once, never given later");
        PENDING.with(|p| p.set(Some('😀')));
        assert_eq!(take_for(true, true), None);
        PENDING.with(|p| p.set(Some('😀')));
        assert_eq!(take_for(true, false).as_deref(), Some("😀"));
        assert_eq!(take_for(true, false), None);
    }
}
