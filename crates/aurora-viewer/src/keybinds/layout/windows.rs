//! Windows keyboard layout: the thread's input locale (`GetKeyboardLayout`),
//! each key read through its virtual key (`KeyboardLayout::from_virtual_keys`).
//!
//! A switch while the viewer runs changes the window thread's input locale;
//! `OsWatch` compares it once a frame, the check winit itself makes on each
//! key (`LayoutCache::get_current_layout`), and reads a locale it has not
//! seen. The comparison is one call, without a system round trip.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, HKL, MAPVK_VK_TO_CHAR, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW};
use winit::platform::scancode::PhysicalKeyExtScancode;

use super::{KeyboardLayout, detected, detected_keys};

fn active() -> HKL {
    // SAFETY: thread 0 asks Windows for this thread's active layout. The HKL
    // is an opaque OS handle, used only by MapVirtualKeyExW, never dereferenced.
    unsafe { GetKeyboardLayout(0) }
}

/// The layout of `hkl`, if it carries the movement letters.
fn read(hkl: HKL) -> Option<KeyboardLayout> {
    let keys = detected_keys().filter_map(|key| {
        let scan = key.to_scancode()?;
        // SAFETY: both calls take integer key codes and a valid OS layout
        // handle. No buffers or keyboard state are read or modified.
        // Mapping scan -> VK first is required for AZERTY letters:
        // https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-mapvirtualkeyexw
        unsafe {
            let vk = MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, hkl);
            (vk != 0).then(|| (key, vk, MapVirtualKeyExW(vk, MAPVK_VK_TO_CHAR, hkl)))
        }
    });
    detected(KeyboardLayout::from_virtual_keys(keys))
}

pub(super) fn detect() -> Option<KeyboardLayout> {
    let hkl = active();
    if hkl.is_null() {
        return None;
    }
    read(hkl)
}

/// The input locale last seen on the window's thread (the address of its
/// handle: only compared).
pub(super) struct OsWatch {
    last: usize,
}

impl OsWatch {
    pub(super) fn new(_: &winit::window::Window) -> Option<Self> {
        log::info!("keyboard layout: following the input locale of the window's thread");
        Some(Self { last: active() as usize })
    }

    /// The layout of a locale switched to since the last call. A locale
    /// without the movement letters (Cyrillic, Greek…) counts as US
    /// QWERTY, as at start.
    pub(super) fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
        let hkl = active();
        if hkl.is_null() || hkl as usize == self.last {
            return Ok(None);
        }
        self.last = hkl as usize;
        log::info!("keyboard layout: input locale {:#x} active", hkl as usize);
        Ok(Some(read(hkl).unwrap_or_default()))
    }

    /// The layout read at start was the active one: nothing to settle.
    pub(super) fn focused(&mut self) {}
}
