//! macOS keyboard layout: the current keyboard input source
//! (`TISCopyCurrentKeyboardLayoutInputSource`), each key's unshifted
//! character read from its 'uchr' table with `UCKeyTranslate`, as SDL and
//! winit read it; `KeyboardLayout::from_us_places` names the keys.
//!
//! A change of input source is posted on the distributed notification
//! center (`kTISNotifySelectedKeyboardInputSourceChanged`, which SDL's
//! Cocoa keyboard also observes). Its callback runs on the main run loop,
//! which winit runs, and only raises a flag that `OsWatch` reads once a
//! frame. The Text Input Sources calls belong to the main thread: every one
//! here is made there.

use std::ffi::c_void;
use std::ptr;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use winit::keyboard::KeyCode;
use winit::platform::scancode::PhysicalKeyExtScancode;

use super::{KeyboardLayout, detected, detected_keys};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type NotificationCallback = extern "C" fn(*mut c_void, *mut c_void, CFStringRef, *const c_void, *const c_void);

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFDataGetBytePtr(data: CFTypeRef) -> *const u8;
    fn CFNotificationCenterGetDistributedCenter() -> *mut c_void;
    fn CFNotificationCenterAddObserver(
        center: *mut c_void,
        observer: *const c_void,
        callback: NotificationCallback,
        name: CFStringRef,
        object: *const c_void,
        suspension_behavior: isize,
    );
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    static kTISNotifySelectedKeyboardInputSourceChanged: CFStringRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> CFTypeRef;
    fn TISGetInputSourceProperty(source: CFTypeRef, key: CFStringRef) -> CFTypeRef;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        virtual_key_code: u16,
        key_action: u16,
        modifier_key_state: u32,
        keyboard_type: u32,
        key_translate_options: u32,
        dead_key_state: *mut u32,
        max_string_length: usize,
        actual_string_length: *mut usize,
        unicode_string: *mut u16,
    ) -> i32;
}

/// kUCKeyActionDisplay: the character a key shows, not typed into text.
const KEY_ACTION_DISPLAY: u16 = 3;
/// kUCKeyTranslateNoDeadKeysMask: a dead key gives its own sign (AZERTY's ^).
const NO_DEAD_KEYS: u32 = 1;
/// kCFNotificationSuspensionBehaviorDeliverImmediately.
const DELIVER_IMMEDIATELY: isize = 4;

/// The layout of the current keyboard input source, if it has a 'uchr'
/// table and carries the movement letters. `None` off the main thread.
fn read() -> Option<KeyboardLayout> {
    if objc2::MainThreadMarker::new().is_none() {
        log::warn!("keyboard layout: asked off the main thread, not read");
        return None;
    }
    // SAFETY: on the main thread. The source is a +1 reference (the Copy
    // rule), released below; the layout data is a CFData the source owns
    // (the Get rule), read only before that release.
    let layout = unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();
        if source.is_null() {
            return None;
        }
        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let layout = (!data.is_null()).then(|| {
            let table = CFDataGetBytePtr(data).cast::<c_void>();
            let keyboard_type = u32::from(LMGetKbdType());
            KeyboardLayout::from_us_places(detected_keys().filter_map(|key| Some((key, typed(table, keyboard_type, key)?))))
        });
        CFRelease(source);
        layout
    };
    if layout.is_none() {
        log::info!("keyboard layout: the input source has no Unicode layout table");
    }
    detected(layout?)
}

/// The character `key` shows without modifiers in the 'uchr' `table`.
///
/// # Safety
/// `table` points to a live 'uchr' table.
unsafe fn typed(table: *const c_void, keyboard_type: u32, key: KeyCode) -> Option<char> {
    let code = u16::try_from(key.to_scancode()?).ok()?;
    let (mut dead_key_state, mut len, mut chars) = (0u32, 0usize, [0u16; 4]);
    // SAFETY: `table` is live (the caller's contract); the out-parameters
    // are local, and `chars` holds the length given.
    let status = unsafe {
        UCKeyTranslate(
            table,
            code,
            KEY_ACTION_DISPLAY,
            0,
            keyboard_type,
            NO_DEAD_KEYS,
            &mut dead_key_state,
            chars.len(),
            &mut len,
            chars.as_mut_ptr(),
        )
    };
    if status != 0 {
        return None;
    }
    char::decode_utf16(chars[..len.min(chars.len())].iter().copied())
        .next()?
        .ok()
        .filter(|c| !c.is_control() && !c.is_whitespace())
}

pub(super) fn detect() -> Option<KeyboardLayout> {
    read()
}

/// The input source changed since `OsWatch` last looked.
static CHANGED: AtomicBool = AtomicBool::new(false);

extern "C" fn input_source_changed(_: *mut c_void, _: *mut c_void, _: CFStringRef, _: *const c_void, _: *const c_void) {
    CHANGED.store(true, Ordering::Relaxed);
}

pub(super) struct OsWatch;

impl OsWatch {
    pub(super) fn new(_: &winit::window::Window) -> Option<Self> {
        static OBSERVE: Once = Once::new();
        OBSERVE.call_once(|| {
            // SAFETY: the distributed center lives for the process; the
            // observer is only a token (a static's address) and the callback
            // a plain function, never removed, so neither outlives the other.
            unsafe {
                CFNotificationCenterAddObserver(
                    CFNotificationCenterGetDistributedCenter(),
                    ptr::from_ref(&CHANGED).cast(),
                    input_source_changed,
                    kTISNotifySelectedKeyboardInputSourceChanged,
                    ptr::null(),
                    DELIVER_IMMEDIATELY,
                );
            }
            log::info!("keyboard layout: following the keyboard input source");
        });
        Some(Self)
    }

    /// The layout of the input source selected since the last call. A
    /// source without the movement letters (Cyrillic, Greek…) counts as US
    /// QWERTY, as at start.
    pub(super) fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
        if !CHANGED.swap(false, Ordering::Relaxed) {
            return Ok(None);
        }
        log::info!("keyboard layout: the keyboard input source changed");
        Ok(Some(read().unwrap_or_default()))
    }

    /// The layout read at start was the active one: nothing to settle.
    pub(super) fn focused(&mut self) {}
}
