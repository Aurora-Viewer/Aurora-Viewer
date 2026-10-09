//! Initial keyboard labels and movement defaults from the active Windows
//! layout, rather than the language of Windows. Firestorm's key_bindings.xml
//! uses fixed WASD letters; adapting those to AZERTY is intentional.

use std::sync::OnceLock;
use winit::keyboard::KeyCode;

#[derive(Default)]
pub(super) struct KeyboardLayout {
    labels: Vec<(KeyCode, char)>,
}

const WASD: [KeyCode; 4] = [KeyCode::KeyW, KeyCode::KeyS, KeyCode::KeyA, KeyCode::KeyD];

impl KeyboardLayout {
    fn letters(&self) -> [char; 4] {
        let azerty = WASD
            .iter()
            .zip(['Z', 'S', 'Q', 'D'])
            .all(|(key, letter)| self.labels.contains(&(*key, letter)));
        if azerty { ['Z', 'S', 'Q', 'D'] } else { ['W', 'S', 'A', 'D'] }
    }

    fn mapped_movement(&self) -> Option<[KeyCode; 4]> {
        let mut keys = WASD;
        for (key, letter) in keys.iter_mut().zip(self.letters()) {
            *key = self.labels.iter().find(|(_, c)| *c == letter)?.0;
        }
        Some(keys)
    }

    pub(super) fn movement_keys(&self) -> [KeyCode; 4] {
        self.mapped_movement().unwrap_or(WASD)
    }

    pub(super) fn label(&self, code: &str) -> Option<String> {
        self.labels
            .iter()
            .find(|(key, _)| format!("{key:?}") == code)
            .map(|(_, c)| c.to_string())
    }

    pub(super) fn wasd() -> Self {
        Self {
            labels: WASD.into_iter().zip(['W', 'S', 'A', 'D']).collect(),
        }
    }

    pub(super) fn zqsd() -> Self {
        use KeyCode as K;
        Self {
            labels: vec![
                (K::KeyW, 'Z'),
                (K::KeyS, 'S'),
                (K::KeyA, 'Q'),
                (K::KeyD, 'D'),
                (K::KeyQ, 'A'),
                (K::KeyZ, 'W'),
            ],
        }
    }
}

pub(super) fn current() -> &'static KeyboardLayout {
    static LAYOUT: OnceLock<KeyboardLayout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        if std::env::var_os("AURORA_DEMO").is_some() {
            match std::env::var("AURORA_DEMO_KEYBOARD").as_deref() {
                Ok("wasd") => return KeyboardLayout::wasd(),
                Ok("zqsd") => return KeyboardLayout::zqsd(),
                Ok("fallback") => return KeyboardLayout::default(),
                _ => {}
            }
        }
        detect().unwrap_or_else(|| {
            log::info!("keyboard layout detection unavailable; using WASD defaults");
            KeyboardLayout::default()
        })
    })
}

#[cfg(not(windows))]
fn detect() -> Option<KeyboardLayout> {
    None
}

#[cfg(windows)]
fn detect() -> Option<KeyboardLayout> {
    use KeyCode as K;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, MAPVK_VK_TO_CHAR, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW};
    use winit::platform::scancode::PhysicalKeyExtScancode;

    // SAFETY: thread 0 asks Windows for this thread's active layout. The HKL
    // is an opaque OS handle, used only by MapVirtualKeyExW, never dereferenced.
    let hkl = unsafe { GetKeyboardLayout(0) };
    if hkl.is_null() {
        return None;
    }
    let mut labels = Vec::new();
    for key in [
        K::KeyA,
        K::KeyB,
        K::KeyC,
        K::KeyD,
        K::KeyE,
        K::KeyF,
        K::KeyG,
        K::KeyH,
        K::KeyI,
        K::KeyJ,
        K::KeyK,
        K::KeyL,
        K::KeyM,
        K::KeyN,
        K::KeyO,
        K::KeyP,
        K::KeyQ,
        K::KeyR,
        K::KeyS,
        K::KeyT,
        K::KeyU,
        K::KeyV,
        K::KeyW,
        K::KeyX,
        K::KeyY,
        K::KeyZ,
        K::Backquote,
        K::Minus,
        K::Equal,
        K::BracketLeft,
        K::BracketRight,
        K::Semicolon,
        K::Quote,
        K::Backslash,
        K::Comma,
        K::Period,
        K::Slash,
        K::IntlBackslash,
    ] {
        let Some(scan) = key.to_scancode() else { continue };
        // SAFETY: both calls take integer key codes and the valid OS layout
        // handle above. No buffers or keyboard state are read or modified.
        let character = unsafe {
            let vk = MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, hkl);
            if vk == 0 {
                continue;
            }
            MapVirtualKeyExW(vk, MAPVK_VK_TO_CHAR, hkl)
        };
        // The low word contains the unshifted character; the high bit marks
        // dead keys. Mapping scan -> VK first is required for AZERTY letters:
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-mapvirtualkeyexw
        if let Some(c) = char::from_u32(character & 0xffff).filter(|c| !c.is_control() && !c.is_whitespace()) {
            labels.push((key, c.to_ascii_uppercase()));
        }
    }
    let layout = KeyboardLayout { labels };
    layout.mapped_movement()?;
    log::info!("keyboard movement defaults: {}", layout.letters().iter().collect::<String>());
    Some(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_labels_are_available_before_any_key_press() {
        let qwerty = KeyboardLayout::wasd();
        let azerty = KeyboardLayout::zqsd();
        assert_eq!(qwerty.letters(), ['W', 'S', 'A', 'D']);
        assert_eq!(azerty.letters(), ['Z', 'S', 'Q', 'D']);
        for (code, qwerty_label, azerty_label) in [("KeyW", "W", "Z"), ("KeyA", "A", "Q"), ("KeyS", "S", "S"), ("KeyD", "D", "D")] {
            assert_eq!(qwerty.label(code).as_deref(), Some(qwerty_label));
            assert_eq!(azerty.label(code).as_deref(), Some(azerty_label));
        }
        assert_eq!(azerty.label("KeyQ").as_deref(), Some("A"));
        assert_eq!(azerty.label("KeyZ").as_deref(), Some("W"));
    }

    #[test]
    fn unavailable_or_incomplete_layout_falls_back_to_wasd() {
        assert_eq!(KeyboardLayout::default().movement_keys(), WASD);
        let incomplete = KeyboardLayout {
            labels: vec![(KeyCode::KeyW, 'Z'), (KeyCode::KeyA, 'Q')],
        };
        assert_eq!(incomplete.mapped_movement(), None);
        assert_eq!(incomplete.movement_keys(), WASD);
    }

    #[test]
    fn other_layouts_use_the_actual_positions_of_wasd_letters() {
        let layout = KeyboardLayout {
            labels: vec![
                (KeyCode::KeyZ, 'W'),
                (KeyCode::KeyO, 'S'),
                (KeyCode::KeyA, 'A'),
                (KeyCode::KeyH, 'D'),
            ],
        };
        assert_eq!(layout.movement_keys(), [KeyCode::KeyZ, KeyCode::KeyO, KeyCode::KeyA, KeyCode::KeyH]);
    }
}
