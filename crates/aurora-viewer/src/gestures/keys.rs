//! Second Life KEY/MASK values from indra_constants.h and LLKeyboard.
use winit::keyboard::{Key, KeyCode, NamedKey};

pub fn from_key(code: KeyCode, unmodified: &Key) -> Option<u8> {
    // LLKeyboardWin32::mTranslateKeyMap (indra/llwindow/llkeyboardwin32.cpp)
    // gives both number rows the same codes. Use the physical keypad key:
    // winit's key_without_modifiers also removes NumLock.
    Some(match code {
        KeyCode::Digit0 | KeyCode::Numpad0 => b'0',
        KeyCode::Digit1 | KeyCode::Numpad1 => b'1',
        KeyCode::Digit2 | KeyCode::Numpad2 => b'2',
        KeyCode::Digit3 | KeyCode::Numpad3 => b'3',
        KeyCode::Digit4 | KeyCode::Numpad4 => b'4',
        KeyCode::Digit5 | KeyCode::Numpad5 => b'5',
        KeyCode::Digit6 | KeyCode::Numpad6 => b'6',
        KeyCode::Digit7 | KeyCode::Numpad7 => b'7',
        KeyCode::Digit8 | KeyCode::Numpad8 => b'8',
        KeyCode::Digit9 | KeyCode::Numpad9 => b'9',
        KeyCode::NumpadAdd => 0x95,
        KeyCode::NumpadSubtract => 0x96,
        KeyCode::NumpadMultiply => 0x97,
        KeyCode::NumpadDivide => 0x98,
        _ => {
            #[cfg(windows)]
            if let Some(translated) = super::windows::character(unmodified) {
                return translated;
            }
            return from_event(unmodified);
        }
    })
}

pub fn from_event(key: &Key) -> Option<u8> {
    match key {
        Key::Character(s) => {
            let mut c = s.chars();
            let first = c.next()?.to_ascii_uppercase();
            (c.next().is_none() && first.is_ascii() && !first.is_ascii_control()).then_some(first as u8)
        }
        Key::Named(n) => Some(match n {
            NamedKey::Space => 32,
            NamedKey::Enter => 0x81,
            NamedKey::ArrowLeft => 0x82,
            NamedKey::ArrowRight => 0x83,
            NamedKey::ArrowUp => 0x84,
            NamedKey::ArrowDown => 0x85,
            NamedKey::Escape => 0x86,
            NamedKey::Backspace => 0x87,
            NamedKey::Delete => 0x88,
            NamedKey::Shift => KEY_SHIFT,
            NamedKey::Control => KEY_CONTROL,
            NamedKey::Alt => KEY_ALT,
            NamedKey::Home => 0x8c,
            NamedKey::End => 0x8d,
            NamedKey::PageUp => 0x8e,
            NamedKey::PageDown => 0x8f,
            NamedKey::Insert => 0x92,
            NamedKey::CapsLock => 0x93,
            NamedKey::Tab => 0x94,
            NamedKey::ContextMenu => 0x99,
            NamedKey::F1 => 0xa1,
            NamedKey::F2 => 0xa2,
            NamedKey::F3 => 0xa3,
            NamedKey::F4 => 0xa4,
            NamedKey::F5 => 0xa5,
            NamedKey::F6 => 0xa6,
            NamedKey::F7 => 0xa7,
            NamedKey::F8 => 0xa8,
            NamedKey::F9 => 0xa9,
            NamedKey::F10 => 0xaa,
            NamedKey::F11 => 0xab,
            NamedKey::F12 => 0xac,
            _ => return None,
        }),
        _ => None,
    }
}

pub fn label(key: u8) -> String {
    match key {
        255 => "---".into(),
        32 => aurora_i18n::tr!("keybinds-space").into(),
        33..=126 => char::from(key).to_string(),
        0xa1..=0xac => format!("F{}", key - 0xa0),
        _ => {
            let code = match key {
                0x81 => "Enter",
                0x82 => "ArrowLeft",
                0x83 => "ArrowRight",
                0x84 => "ArrowUp",
                0x85 => "ArrowDown",
                0x86 => "Escape",
                0x87 => "Backspace",
                0x88 => "Delete",
                KEY_SHIFT => "ShiftLeft",
                KEY_CONTROL => "ControlLeft",
                KEY_ALT => "AltLeft",
                0x8c => "Home",
                0x8d => "End",
                0x8e => "PageUp",
                0x8f => "PageDown",
                0x92 => "Insert",
                0x93 => "CapsLock",
                0x94 => "Tab",
                0x99 => "ContextMenu",
                0x95 | 0xcc => "NumpadAdd",
                0x96 | 0xcd => "NumpadSubtract",
                0x97 | 0xce => "NumpadMultiply",
                0x98 | 0xcf => "NumpadDivide",
                0xc0 => "Numpad8",
                0xc1 => "Numpad2",
                0xc2 => "Numpad4",
                0xc3 => "Numpad6",
                0xc4 => "Numpad7",
                0xc5 => "Numpad1",
                0xc6 => "Numpad9",
                0xc7 => "Numpad3",
                0xc8 => "Numpad5",
                0xc9 => "Numpad0",
                0xca => "NumpadDecimal",
                0xcb => "NumpadEnter",
                _ => return format!("0x{key:02X}"),
            };
            crate::keybinds::Input::Key(code.into()).label()
        }
    }
}

pub fn shortcut(key: u8, mask: u32) -> String {
    if key == 255 {
        return "---".into();
    }
    format!("{}{}", shortcut_prefix(mask), label(key))
}

pub const KEY_SHIFT: u8 = 0x89;
pub const KEY_CONTROL: u8 = 0x8a;
pub const KEY_ALT: u8 = 0x8b;

const LEVEL_SHIFT: u8 = 1;
const LEVEL_CONTROL: u8 = 2;
const LEVEL_ALT: u8 = 4;

/// Shift, Ctrl and Alt levels after a key event, one per modifier whatever
/// its side, as LLKeyboard::mKeyLevel: either side raises it, either
/// side's release lowers it. One level also comes down when Windows sends
/// no release for the first of two held Shift keys: the last one lowers it.
pub fn modifier_levels_after(levels: u8, physical: KeyCode, pressed: bool) -> u8 {
    let bit = match physical {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => LEVEL_SHIFT,
        KeyCode::ControlLeft | KeyCode::ControlRight => LEVEL_CONTROL,
        KeyCode::AltLeft | KeyCode::AltRight => LEVEL_ALT,
        _ => return levels,
    };
    if pressed { levels | bit } else { levels & !bit }
}

/// Gesture mask of a key press, or `None` when the press plays no gesture,
/// as the Legacy Viewer gives it on Windows: LLKeyboardWin32::handleKeyDown takes the mask from the key levels
/// (LLKeyboardWin32::currentMask) before LLKeyboard::handleTranslatedKeyDown
/// raises the pressed key's level, so Shift, Ctrl or Alt going down carry
/// no bit of their own unless that level was already raised (an
/// auto-repeat, or the other side held): a held Shift plays once. winit
/// already counts the pressed modifier in `ctrl` / `shift` / `alt` on
/// Windows, so its own bit is dropped from `levels_before`, the levels
/// before this event. The Legacy Viewer's gesture editor offers no Alt mask
/// (LLPreviewGesture: None, Shift, Ctrl) and a key pressed with Alt down
/// carries MASK_ALT, so such a press plays nothing; Alt going down alone is
/// a gesture key.
pub fn gesture_mask(key: u8, levels_before: u8, repeat: bool, ctrl: bool, shift: bool, alt: bool) -> Option<u32> {
    let own = match key {
        KEY_SHIFT => LEVEL_SHIFT,
        KEY_CONTROL => LEVEL_CONTROL,
        KEY_ALT => LEVEL_ALT,
        _ => 0,
    };
    let was_down = repeat || levels_before & own != 0;
    let alt = alt && (was_down || key != KEY_ALT);
    if alt {
        return None;
    }
    let ctrl = ctrl && (was_down || key != KEY_CONTROL);
    let shift = shift && (was_down || key != KEY_SHIFT);
    Some(u32::from(ctrl) | if shift { 4 } else { 0 })
}

/// Modifier half of a gesture shortcut. Mask 1 is Control, which the Mac
/// keyboard reports as Command (LLKeyboard::stringFromAccelerator); the
/// gesture mask has no `use_mac_ctrl` bit.
pub fn shortcut_prefix(mask: u32) -> String {
    match mask {
        1 => {
            if cfg!(target_os = "macos") {
                "⌘".into()
            } else {
                "Ctrl+".into()
            }
        }
        4 => aurora_i18n::tr!("keybinds-shift-2").into(),
        _ => String::new(),
    }
}

/// The modifier named in the gesture editor, without the key.
pub fn modifier_name(mask: u32) -> String {
    match mask {
        1 => {
            if cfg!(target_os = "macos") {
                "⌘".into()
            } else {
                "Ctrl".into()
            }
        }
        4 => aurora_i18n::tr!("keybinds-shift").into(),
        _ => "---".into(),
    }
}

pub fn choices() -> impl Iterator<Item = u8> {
    std::iter::once(255)
        .chain(32..=126)
        .chain(0x81..=0x8f)
        .chain([0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99])
        .chain(0xa1..=0xac)
        .chain(0xc0..=0xcf)
}
