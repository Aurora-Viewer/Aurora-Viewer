//! LLKeyboardWin32::mTranslateKeyMap (Firestorm, LGPL 2.1). Gesture assets
//! store Windows virtual keys translated to SL codes, independent of Shift
//! and of the characters printed by an international keyboard layout.
use winit::keyboard::Key;

#[link(name = "user32")]
// SAFETY: these signatures match User32's scalar keyboard-layout API.
unsafe extern "system" {
    fn GetKeyboardLayout(thread: u32) -> isize;
    fn VkKeyScanExW(character: u16, layout: isize) -> i16;
}

pub fn character(key: &Key) -> Option<Option<u8>> {
    let character = match key {
        Key::Character(text) => {
            let mut chars = text.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            c
        }
        Key::Dead(Some(c)) => *c,
        _ => return None,
    };
    if u32::from(character) > u32::from(u16::MAX) {
        return None;
    }
    // SAFETY: these User32 calls query this thread's layout and translate one
    // scalar character; they neither read buffers nor retain any pointer.
    let virtual_key = unsafe { VkKeyScanExW(character as u16, GetKeyboardLayout(0)) };
    if virtual_key == -1 {
        return None;
    }
    Some(translate((virtual_key & 255) as u8))
}

fn translate(key: u8) -> Option<u8> {
    Some(match key {
        b'0'..=b'9' | b'A'..=b'Z' | 32 => key,
        0xba => b';',
        0xbb => b'=',
        0xbc => b',',
        0xbd => b'-',
        0xbe => b'.',
        0xbf => 0x98,
        0xc0 => b'`',
        0xdb => b'[',
        0xdc => b'\\',
        0xdd => b']',
        0xde => b'\'',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oem_keys_keep_firestorm_virtual_codes() {
        assert_eq!(translate(0xdb), Some(b'['));
        assert_eq!(translate(0xbf), Some(0x98));
        assert_eq!(translate(b'G'), Some(b'G'));
        assert_eq!(translate(0xff), None);
    }
}
