//! Windows Alt codes in every text field: left Alt held, digits typed on the
//! numpad, Alt released gives the character (Alt + 3 = ♥, Alt + 0233 = é).
//!
//! Windows sends that character as a `WM_CHAR` after the Alt key-up; winit
//! 0.30 only attaches a `WM_CHAR` to the key press before it and drops this
//! one (platform_impl/windows/keyboard.rs, "probably IME"), and egui-winit
//! turns each digit typed with Alt held into text of its own. So the
//! sequence is read here, from the key events: the digits are kept from the
//! text field, and the character is handed to egui when Alt is released.
//!
//! Windows' rules: without a leading 0 the number is read in the OEM code
//! page (437 in the United States, 850 in Western Europe), its 1 to 31 and
//! 127 shown as glyphs (☺ ♥ ►…); with a leading 0 in the ANSI code page
//! (Windows-1252 in Western Europe: 0128 = €). Numbers above 255 wrap
//! around 256, as in Windows' edit controls. On Windows the system's own
//! code pages are used; elsewhere, and in the tests, 437 and 1252.

use winit::keyboard::KeyCode;

/// An Alt code being typed.
#[derive(Debug, Default)]
pub struct AltCodes {
    /// Left Alt is held.
    held: bool,
    digits: String,
    /// Another key came in between: the sequence gives nothing.
    spoiled: bool,
}

/// The numpad digit of a key, if it is one.
pub fn numpad_digit(code: KeyCode) -> Option<char> {
    Some(match code {
        KeyCode::Numpad0 => '0',
        KeyCode::Numpad1 => '1',
        KeyCode::Numpad2 => '2',
        KeyCode::Numpad3 => '3',
        KeyCode::Numpad4 => '4',
        KeyCode::Numpad5 => '5',
        KeyCode::Numpad6 => '6',
        KeyCode::Numpad7 => '7',
        KeyCode::Numpad8 => '8',
        KeyCode::Numpad9 => '9',
        _ => return None,
    })
}

impl AltCodes {
    /// Left Alt pressed: a new sequence.
    pub fn alt_pressed(&mut self) {
        *self = AltCodes {
            held: true,
            ..AltCodes::default()
        };
    }

    /// A key pressed while a text field has the keyboard: true when it is a
    /// digit of the sequence, which the text field must not see.
    pub fn key_pressed(&mut self, code: KeyCode, repeat: bool) -> bool {
        if !self.held {
            return false;
        }
        match numpad_digit(code) {
            Some(d) => {
                if !repeat {
                    self.digits.push(d);
                }
                true
            }
            None => {
                if !matches!(code, KeyCode::AltLeft) {
                    self.spoiled = true;
                }
                false
            }
        }
    }

    /// Left Alt released: the character typed, if any. `delivered` is the
    /// text the platform attached to the release (a future winit, another
    /// system): it is used as it is, never added to ours.
    pub fn alt_released(&mut self, delivered: Option<&str>) -> Option<String> {
        self.released_with(delivered, platform_char)
    }

    fn released_with(&mut self, delivered: Option<&str>, char_of: impl Fn(CodePage, u8) -> Option<char>) -> Option<String> {
        let sequence = std::mem::take(self);
        if !sequence.held || sequence.spoiled || sequence.digits.is_empty() {
            return None;
        }
        if let Some(text) = delivered.filter(|t| !t.is_empty() && !t.chars().any(char::is_control)) {
            return Some(text.to_owned());
        }
        decode(&sequence.digits, char_of).map(String::from)
    }

    /// The window lost the keyboard: nothing is pending.
    pub fn reset(&mut self) {
        *self = AltCodes::default();
    }

    /// A sequence is being typed (left Alt held).
    pub fn active(&self) -> bool {
        self.held
    }
}

/// Which code page a number is read in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodePage {
    /// No leading 0 (437, 850…).
    Oem,
    /// A leading 0 (1252…).
    Ansi,
}

/// The character of `digits` typed with Alt, read by `char_of` in its code
/// page; None for 0, a control character or an empty slot.
pub fn decode(digits: &str, char_of: impl Fn(CodePage, u8) -> Option<char>) -> Option<char> {
    let value: u64 = digits.parse().ok()?;
    let page = if digits.starts_with('0') { CodePage::Ansi } else { CodePage::Oem };
    let byte = (value % 256) as u8;
    if byte == 0 {
        return None;
    }
    char_of(page, byte).filter(|c| !c.is_control())
}

/// 437 and 1252, the code pages of an English Windows.
pub fn table_char(page: CodePage, byte: u8) -> Option<char> {
    match page {
        CodePage::Oem => cp437(byte),
        CodePage::Ansi => cp1252(byte),
    }
}

/// The system's code pages on Windows (what Notepad gives for the same
/// keys), the tables elsewhere.
fn platform_char(page: CodePage, byte: u8) -> Option<char> {
    #[cfg(windows)]
    if let Some(c) = windows_char(page, byte) {
        return Some(c);
    }
    table_char(page, byte)
}

#[cfg(windows)]
fn windows_char(page: CodePage, byte: u8) -> Option<char> {
    use windows_sys::Win32::Globalization::{GetACP, GetOEMCP, MB_USEGLYPHCHARS, MultiByteToWideChar};
    let mut wide = [0u16; 4];
    // SAFETY: plain conversions of one byte into a buffer of known length.
    let n = unsafe {
        let cp = match page {
            CodePage::Oem => GetOEMCP(),
            CodePage::Ansi => GetACP(),
        };
        MultiByteToWideChar(cp, MB_USEGLYPHCHARS, &byte, 1, wide.as_mut_ptr(), wide.len() as i32)
    };
    if n <= 0 {
        return None;
    }
    char::decode_utf16(wide[..n as usize].iter().copied()).next()?.ok()
}

/// Code page 437, with the glyphs Windows shows for 1 to 31 and 127.
fn cp437(byte: u8) -> Option<char> {
    const LOW: [char; 32] = [
        '\0', '☺', '☻', '♥', '♦', '♣', '♠', '•', '◘', '○', '◙', '♂', '♀', '♪', '♫', '☼', '►', '◄', '↕', '‼', '¶', '§', '▬', '↨', '↑', '↓',
        '→', '←', '∟', '↔', '▲', '▼',
    ];
    const HIGH: [char; 128] = [
        'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', // 0x80
        'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', // 0x90
        'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', // 0xA0
        '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', // 0xB0
        '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', // 0xC0
        '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', // 0xD0
        'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', // 0xE0
        '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}', // 0xF0
    ];
    match byte {
        0 => None,
        1..=31 => Some(LOW[byte as usize]),
        127 => Some('⌂'),
        32..=126 => Some(byte as char),
        _ => Some(HIGH[(byte - 0x80) as usize]),
    }
}

/// Windows-1252; its five empty slots give nothing.
fn cp1252(byte: u8) -> Option<char> {
    const C1: [Option<char>; 32] = [
        Some('€'),
        None,
        Some('‚'),
        Some('ƒ'),
        Some('„'),
        Some('…'),
        Some('†'),
        Some('‡'),
        Some('ˆ'),
        Some('‰'),
        Some('Š'),
        Some('‹'),
        Some('Œ'),
        None,
        Some('Ž'),
        None,
        None,
        Some('‘'),
        Some('’'),
        Some('“'),
        Some('”'),
        Some('•'),
        Some('–'),
        Some('—'),
        Some('˜'),
        Some('™'),
        Some('š'),
        Some('›'),
        Some('œ'),
        None,
        Some('ž'),
        Some('Ÿ'),
    ];
    match byte {
        0x80..=0x9f => C1[(byte - 0x80) as usize],
        // the rest is Latin-1
        _ => char::from_u32(u32::from(byte)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &[KeyCode]) -> Option<String> {
        let mut a = AltCodes::default();
        a.alt_pressed();
        for &k in keys {
            a.key_pressed(k, false);
        }
        // the tables, not the code pages of the machine running the tests
        a.released_with(None, table_char)
    }

    #[test]
    fn the_oem_code_page_without_a_leading_zero() {
        use KeyCode::*;
        assert_eq!(typed(&[Numpad3]).as_deref(), Some("♥"));
        assert_eq!(typed(&[Numpad1]).as_deref(), Some("☺"));
        assert_eq!(typed(&[Numpad1, Numpad6]).as_deref(), Some("►"));
        assert_eq!(typed(&[Numpad1, Numpad3, Numpad0]).as_deref(), Some("é"));
        assert_eq!(typed(&[Numpad6, Numpad5]).as_deref(), Some("A"));
        assert_eq!(typed(&[Numpad1, Numpad2, Numpad7]).as_deref(), Some("⌂"));
    }

    #[test]
    fn the_ansi_code_page_with_a_leading_zero() {
        use KeyCode::*;
        assert_eq!(typed(&[Numpad0, Numpad2, Numpad3, Numpad3]).as_deref(), Some("é"));
        assert_eq!(typed(&[Numpad0, Numpad1, Numpad2, Numpad8]).as_deref(), Some("€"));
        assert_eq!(typed(&[Numpad0, Numpad1, Numpad5, Numpad3]).as_deref(), Some("™"));
        // an empty slot of 1252, a control character: nothing
        assert_eq!(typed(&[Numpad0, Numpad1, Numpad2, Numpad9]), None);
        assert_eq!(typed(&[Numpad0, Numpad9]), None);
    }

    #[test]
    fn numbers_above_255_wrap_and_zero_gives_nothing() {
        use KeyCode::*;
        assert_eq!(typed(&[Numpad2, Numpad5, Numpad9]).as_deref(), Some("♥"));
        assert_eq!(typed(&[Numpad0]), None);
        assert_eq!(typed(&[Numpad2, Numpad5, Numpad6]), None);
    }

    #[test]
    fn another_key_or_no_digit_gives_nothing() {
        let mut a = AltCodes::default();
        a.alt_pressed();
        assert!(a.key_pressed(KeyCode::Numpad3, false));
        assert!(!a.key_pressed(KeyCode::KeyA, false));
        assert_eq!(a.alt_released(None), None);
        a.alt_pressed();
        assert_eq!(a.alt_released(None), None);
        // without Alt, a numpad digit is the text field's
        assert!(!a.key_pressed(KeyCode::Numpad3, false));
    }

    #[test]
    fn a_key_repeat_adds_no_digit() {
        let mut a = AltCodes::default();
        a.alt_pressed();
        assert!(a.key_pressed(KeyCode::Numpad1, false));
        assert!(a.key_pressed(KeyCode::Numpad1, true));
        assert!(a.key_pressed(KeyCode::Numpad6, false));
        assert_eq!(a.digits, "16");
    }

    #[test]
    fn text_the_platform_delivered_is_used_as_it_is() {
        let mut a = AltCodes::default();
        a.alt_pressed();
        a.key_pressed(KeyCode::Numpad3, false);
        assert_eq!(a.alt_released(Some("♥")).as_deref(), Some("♥"));
        // nothing pending afterwards: never inserted twice
        assert_eq!(a.alt_released(Some("♥")), None);
    }

    /// The path of the window events in `App::alt_code_key`: Alt + 3 on the
    /// numpad in a focused field gives ♥ at Alt's release, and the digit,
    /// kept from egui, is not typed.
    #[test]
    fn a_focused_field_receives_the_character_and_not_the_digits() {
        let ctx = egui::Context::default();
        let mut text = String::from("I ");
        let id = egui::Id::unique("alt-code-field");
        let frame = |events: Vec<egui::Event>, text: &mut String| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let r = ui.add(egui::TextEdit::singleline(text).id(id));
                r.request_focus();
            });
            output.textures_delta.clear();
        };
        frame(Vec::new(), &mut text);
        frame(Vec::new(), &mut text);
        let mut codes = AltCodes::default();
        let mut events = Vec::new();
        codes.alt_pressed();
        // the digit is the sequence's: nothing reaches egui
        if !codes.key_pressed(KeyCode::Numpad3, false) {
            events.push(egui::Event::Text("3".into()));
        }
        if let Some(c) = codes.released_with(None, table_char) {
            events.push(egui::Event::Text(c));
        }
        frame(events, &mut text);
        assert_eq!(text, "I ♥");
    }

    #[test]
    fn code_page_437_gives_a_printable_character_for_every_byte() {
        for b in 1..=255u8 {
            let c = table_char(CodePage::Oem, b);
            assert!(c.is_some_and(|c| !c.is_control()), "437 {b}: {c:?}");
        }
        assert_eq!(table_char(CodePage::Oem, 0xff), Some('\u{a0}'));
        assert_eq!(table_char(CodePage::Ansi, 0xe9), Some('é'));
    }
}
