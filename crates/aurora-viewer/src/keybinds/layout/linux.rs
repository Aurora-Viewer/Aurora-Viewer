//! Linux keyboard layout: the live XKB keymap, from the Wayland compositor
//! or the X server, read with libxkbcommon (loaded at run time, as winit
//! does). Each key's character is its level 1 keysym in the active layout;
//! `KeyboardLayout::from_typed` turns them into labels and names.
//!
//! Wayland only tells a focused surface which layout is active, so on
//! Wayland the first layout of the keymap is used; X11 reports the active
//! one.

use std::ptr;

use winit::keyboard::KeyCode;
use winit::platform::scancode::PhysicalKeyExtScancode;
use xkbcommon_dl::{self as xkb, XkbCommon, xkb_context_flags, xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS};

use super::{KeyboardLayout, dead_key_char, detected, detected_keys};

pub(super) fn detect() -> Option<KeyboardLayout> {
    let Some(lib) = xkb::xkbcommon_option() else {
        log::info!("libxkbcommon not found: no keyboard layout");
        return None;
    };
    let context = Context::new(lib)?;
    // WAYLAND_SOCKET is a socket handed to one client: connecting through it
    // here would take winit's connection away.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("WAYLAND_SOCKET").is_none();
    let (keymap, layout, source) = wayland
        .then(|| wayland::keymap(&context).map(|keymap| (keymap, 0, "Wayland compositor")))
        .flatten()
        .or_else(|| x11::keymap(&context).map(|(keymap, layout)| (keymap, layout, "X server")))?;
    // SAFETY: the keymap is valid while `keymap` lives.
    let layouts = unsafe { (lib.xkb_keymap_num_layouts)(keymap.ptr) };
    log::info!("keyboard layout: layout {} of {layouts} of the {source}'s keymap", layout + 1);
    detected(KeyboardLayout::from_typed(
        detected_keys().filter_map(|key| Some((key, keymap.typed(key, layout)?))),
    ))
}

struct Context {
    lib: &'static XkbCommon,
    ptr: *mut xkb::xkb_context,
}

impl Context {
    fn new(lib: &'static XkbCommon) -> Option<Self> {
        // SAFETY: plain constructor; null on failure.
        let ptr = unsafe { (lib.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS) };
        (!ptr.is_null()).then_some(Self { lib, ptr })
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // SAFETY: the reference taken by xkb_context_new, released once.
        unsafe { (self.lib.xkb_context_unref)(self.ptr) }
    }
}

struct Keymap {
    lib: &'static XkbCommon,
    ptr: *mut xkb::xkb_keymap,
}

impl Keymap {
    fn new(context: &Context, ptr: *mut xkb::xkb_keymap) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { lib: context.lib, ptr })
    }

    /// The character `key` types without modifiers in `layout`.
    fn typed(&self, key: KeyCode, layout: u32) -> Option<char> {
        // XKB keycodes are the evdev codes winit uses, plus 8.
        let code = key.to_scancode()? + 8;
        let mut syms = ptr::null();
        // SAFETY: the keymap is valid; xkbcommon points `syms` at its own
        // array of the returned length, which lives as long as the keymap.
        let sym = unsafe {
            let layouts = (self.lib.xkb_keymap_num_layouts_for_key)(self.ptr, code);
            if layouts == 0 {
                return None;
            }
            // a key with fewer layouts wraps, like XKB's default group action
            if (self.lib.xkb_keymap_key_get_syms_by_level)(self.ptr, code, layout % layouts, 0, &mut syms) != 1 {
                return None;
            }
            *syms
        };
        // SAFETY: a pure keysym conversion.
        let unicode = unsafe { (self.lib.xkb_keysym_to_utf32)(sym) };
        dead_key_char(sym)
            .or_else(|| char::from_u32(unicode))
            .filter(|c| !c.is_control() && !c.is_whitespace())
    }
}

impl Drop for Keymap {
    fn drop(&mut self) {
        // SAFETY: the reference taken at creation, released once.
        unsafe { (self.lib.xkb_keymap_unref)(self.ptr) }
    }
}

mod wayland {
    use std::fs::File;
    use std::os::unix::fs::FileExt;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

    use super::{Context, Keymap, xkb};

    /// The keymap text the compositor sends to a new keyboard.
    #[derive(Default)]
    struct Text(Option<Vec<u8>>);

    /// The compositor's keymap, over a connection of our own that only asks
    /// for it.
    pub(super) fn keymap(context: &Context) -> Option<Keymap> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) = registry_queue_init::<Text>(&connection).ok()?;
        let seat: wl_seat::WlSeat = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        seat.get_keyboard(&queue.handle(), ());
        let mut text = Text::default();
        queue.roundtrip(&mut text).ok()?;
        let mut text = text.0?;
        while text.last() == Some(&0) {
            text.pop();
        }
        // SAFETY: a buffer of the given length, copied by xkbcommon.
        let ptr = unsafe {
            (context.lib.xkb_keymap_new_from_buffer)(
                context.ptr,
                text.as_ptr().cast(),
                text.len(),
                xkb::xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                super::XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        };
        Keymap::new(context, ptr)
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Text {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for Text {
        fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }

    impl Dispatch<wl_keyboard::WlKeyboard, ()> for Text {
        fn event(text: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } = event
            {
                // read at offset 0: the compositor may share one file between clients
                let mut buf = vec![0; size as usize];
                if File::from(fd).read_exact_at(&mut buf, 0).is_ok() {
                    text.0 = Some(buf);
                }
            }
        }
    }
}

mod x11 {
    use std::ptr;

    use x11rb::xcb_ffi::{XCBConnection, load_libxcb};
    use xkbcommon_dl::x11::{self as xkb_x11, xkb_x11_setup_xkb_extension_flags::XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS};

    use super::{Context, Keymap, xkb};

    /// The X server's keymap and its active layout.
    pub(super) fn keymap(context: &Context) -> Option<(Keymap, u32)> {
        let lib = xkb_x11::xkbcommon_x11_option()?;
        // libxcb is loaded on first use, and a failed lazy load panics:
        // load it here, where its absence only means no layout.
        load_libxcb().ok()?;
        let (connection, _) = XCBConnection::connect(None).ok()?;
        let raw = connection.get_raw_xcb_connection();
        // SAFETY: `raw` is the live XCB connection owned by `connection`,
        // which outlives these calls; the null out-parameters are optional.
        unsafe {
            let ready = (lib.xkb_x11_setup_xkb_extension)(
                raw,
                xkb_x11::XKB_X11_MIN_MAJOR_XKB_VERSION,
                xkb_x11::XKB_X11_MIN_MINOR_XKB_VERSION,
                XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            let device = (lib.xkb_x11_get_core_keyboard_device_id)(raw);
            if ready != 1 || device < 0 {
                return None;
            }
            let keymap = Keymap::new(
                context,
                (lib.xkb_x11_keymap_new_from_device)(context.ptr, raw, device, super::XKB_KEYMAP_COMPILE_NO_FLAGS),
            )?;
            let state = (lib.xkb_x11_state_new_from_device)(keymap.ptr, raw, device);
            let layout = if state.is_null() {
                0
            } else {
                let layout = (context.lib.xkb_state_serialize_layout)(state, xkb::xkb_state_component::XKB_STATE_LAYOUT_EFFECTIVE);
                (context.lib.xkb_state_unref)(state);
                layout
            };
            Some((keymap, layout))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;

    use super::*;

    /// A keymap compiled by the system's libxkbcommon from its layout name.
    fn named(context: &Context, layout: &str) -> Option<Keymap> {
        let layout = CString::new(layout).expect("layout name");
        let names = xkb::xkb_rule_names {
            rules: ptr::null(),
            model: ptr::null(),
            layout: layout.as_ptr(),
            variant: ptr::null(),
            options: ptr::null(),
        };
        // SAFETY: valid context and names, read during the call only.
        Keymap::new(context, unsafe {
            (context.lib.xkb_keymap_new_from_names)(context.ptr, &names, XKB_KEYMAP_COMPILE_NO_FLAGS)
        })
    }

    #[test]
    fn real_xkb_keymaps_give_azerty_and_qwertz() {
        let Some(context) = xkb::xkbcommon_option().and_then(Context::new) else {
            eprintln!("skipped: libxkbcommon is not installed");
            return;
        };
        let (Some(fr), Some(de)) = (named(&context, "fr"), named(&context, "de")) else {
            eprintln!("skipped: xkeyboard-config has no fr or de layout");
            return;
        };
        let read = |keymap: &Keymap| KeyboardLayout::from_typed(detected_keys().filter_map(|key| Some((key, keymap.typed(key, 0)?))));
        let azerty = read(&fr);
        assert_eq!(azerty.letters(), ['Z', 'S', 'Q', 'D']);
        assert_eq!(azerty.key_for('M'), Some(KeyCode::Semicolon));
        assert_eq!(azerty.label("Quote").as_deref(), Some("Ù"));
        assert_eq!(azerty.label("BracketLeft").as_deref(), Some("^"), "dead_circumflex");
        assert_eq!(azerty.key_for('`'), None);
        let qwertz = read(&de);
        assert_eq!(qwertz.letters(), ['W', 'S', 'A', 'D']);
        assert_eq!(qwertz.key_for('Z'), Some(KeyCode::KeyY));
        assert_eq!(qwertz.label("Minus").as_deref(), Some("ß"));
    }
}
