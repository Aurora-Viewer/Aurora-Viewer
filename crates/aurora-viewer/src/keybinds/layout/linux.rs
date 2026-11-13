//! Linux keyboard layout: the live XKB keymap, from the Wayland compositor
//! or the X server, read with libxkbcommon (loaded at run time, as winit
//! does). Each key's character is its level 1 keysym in the active layout;
//! `KeyboardLayout::from_typed` turns them into labels and names.
//!
//! Every layout of the keymap is read (`us,fr` gives two). X11 reports the
//! active one at start; Wayland only tells a focused surface, and XWayland
//! takes the compositor's once one of its windows has the focus, so there
//! the first layout stands in until the watch hears the active one.
//!
//! `OsWatch` follows the layout while the viewer runs, on the window's own
//! display server: on Wayland a keyboard of our own on winit's connection
//! (the group of `wl_keyboard.modifiers`, and a new keymap), on X11 a
//! connection of our own, read by a thread of its own, that selects XKB's
//! group changes and new keymaps (`XkbStateNotify`, `XkbNewKeyboardNotify`,
//! `XkbMapNotify`). Neither waits in the frame loop.

use std::ptr;

use winit::keyboard::KeyCode;
use winit::platform::scancode::PhysicalKeyExtScancode;
use xkbcommon_dl::{self as xkb, XkbCommon, xkb_context_flags, xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS};

use super::{KeyboardLayout, Layouts, dead_key_char, detected, detected_keys};

pub(super) fn detect() -> Option<Layouts> {
    let Some(lib) = xkb::xkbcommon_option() else {
        log::info!("libxkbcommon not found: no keyboard layout");
        return None;
    };
    let context = Context::new(lib)?;
    // WAYLAND_SOCKET is a socket handed to one client: connecting through it
    // here would take winit's connection away.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("WAYLAND_SOCKET").is_none();
    let (keymap, active, guessed, source) = wayland
        .then(|| wayland::keymap(&context).map(|keymap| (keymap, 0, true, "Wayland compositor")))
        .flatten()
        .or_else(|| x11::keymap(&context).map(|(keymap, layout, xwayland)| (keymap, layout, xwayland, "X server")))?;
    let all = layouts_of(&keymap, source);
    let count = all.len();
    if all.iter().all(Option::is_none) {
        return None;
    }
    let active = usize::try_from(active).unwrap_or(0);
    log::info!(
        "keyboard layout: layout {} in use{}",
        active + 1,
        if guessed && count > 1 {
            " until the active one is reported"
        } else {
            ""
        }
    );
    // a layout without the movement letters falls back to US QWERTY alone
    Some(Layouts::new(
        all.into_iter().map(Option::unwrap_or_default).collect(),
        active,
        guessed,
    ))
}

/// Every layout of `keymap`: `None` for one without the movement letters,
/// which counts as US QWERTY.
fn layouts_of(keymap: &Keymap, source: &str) -> Vec<Option<KeyboardLayout>> {
    // SAFETY: the keymap is valid while `keymap` lives.
    let count = unsafe { (keymap.lib.xkb_keymap_num_layouts)(keymap.ptr) }.max(1);
    (0..count)
        .map(|layout| {
            log::info!("keyboard layout: layout {} of {count} of the {source}'s keymap", layout + 1);
            detected(KeyboardLayout::from_typed(
                detected_keys().filter_map(|key| Some((key, keymap.typed(key, layout)?))),
            ))
        })
        .collect()
}

/// Every layout of `keymap`, one without the movement letters as US QWERTY:
/// what a watch picks the active layout from.
fn watched_layouts(keymap: &Keymap, source: &str) -> Vec<KeyboardLayout> {
    layouts_of(keymap, source).into_iter().map(Option::unwrap_or_default).collect()
}

/// The layout `group` names among a keymap's `layouts`. A group beyond
/// them wraps, like XKB's default group action.
fn layout_of_group(layouts: &[KeyboardLayout], group: u32) -> Option<KeyboardLayout> {
    let index = usize::try_from(group).ok()? % layouts.len().max(1);
    layouts.get(index).cloned()
}

/// The active layout on the window's display server.
pub(super) enum OsWatch {
    Wayland(wayland::Watch),
    X11(x11::Watch),
}

impl OsWatch {
    pub(super) fn new(window: &winit::window::Window) -> Option<Self> {
        use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        match window.display_handle().ok()?.as_raw() {
            RawDisplayHandle::Wayland(handle) => wayland::Watch::new(handle).map(Self::Wayland),
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => x11::Watch::new().map(Self::X11),
            _ => None,
        }
    }

    /// The layout made active since the last call, if any.
    pub(super) fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
        match self {
            Self::Wayland(watch) => watch.poll(),
            Self::X11(watch) => watch.poll(),
        }
    }

    /// Wayland reports the group to a keyboard entering the window; XWayland
    /// sends no event when its group already matches.
    pub(super) fn focused(&mut self) {
        if let Self::X11(watch) = self {
            watch.focused();
        }
    }
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
    use std::mem::ManuallyDrop;
    use std::os::unix::fs::FileExt;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
    use winit::raw_window_handle::WaylandDisplayHandle;

    use super::{Context, KeyboardLayout, Keymap, xkb};

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
        compile(context, text.0?)
    }

    /// Compiles a keymap the compositor sent as text.
    fn compile(context: &Context, mut text: Vec<u8>) -> Option<Keymap> {
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

    /// The keymap text of a `wl_keyboard.keymap` event, if it is XKB's.
    fn keymap_text(event: wl_keyboard::Event) -> Option<Vec<u8>> {
        let wl_keyboard::Event::Keymap {
            format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
            fd,
            size,
        } = event
        else {
            return None;
        };
        // read at offset 0: the compositor may share one file between clients
        let mut buf = vec![0; usize::try_from(size).ok()?];
        File::from(fd).read_exact_at(&mut buf, 0).ok()?;
        Some(buf)
    }

    /// The active layout, heard by a keyboard of our own on winit's
    /// connection: the compositor sends it the keymap, and `modifiers`,
    /// whose `group` is the active layout, to every keyboard of the focused
    /// client, ours with winit's. Its queue is ours alone; winit's event loop
    /// reads the socket and libwayland files our events there.
    ///
    /// Dropped, it releases its keyboard and seat where their versions can,
    /// and leaves its queue alive: the registry has no destructor, and a
    /// queue destroyed under a live proxy is a use-after-free in libwayland
    /// before 1.22 (a warning and lost events after).
    pub(in super::super) struct Watch {
        queue: ManuallyDrop<EventQueue<Heard>>,
        heard: Heard,
        seat: wl_seat::WlSeat,
        keyboard: wl_keyboard::WlKeyboard,
    }

    #[derive(Default)]
    struct Heard {
        /// The seat has a keyboard (`wl_seat.capabilities`).
        keyboard: bool,
        /// The layouts of the keymap last sent.
        layouts: Vec<KeyboardLayout>,
        /// The group last reported.
        group: Option<u32>,
        /// The group or the keymap changed since the last poll.
        changed: bool,
    }

    impl Watch {
        pub(in super::super) fn new(handle: WaylandDisplayHandle) -> Option<Self> {
            // SAFETY: winit's wl_display, which lives as long as its event
            // loop; the watch belongs to the window's `Gfx`, dropped in
            // `exiting` while the loop still lives.
            let backend = unsafe { wayland_client::backend::Backend::from_foreign_display(handle.display.as_ptr().cast()) };
            let connection = Connection::from_backend(backend);
            let (globals, mut queue) = registry_queue_init::<Heard>(&connection).ok()?;
            // version 5 has `release`
            let seat: wl_seat::WlSeat = globals.bind(&queue.handle(), 1..=5, ()).ok()?;
            let mut heard = Heard::default();
            // `get_keyboard` on a seat that never had a keyboard is a protocol
            // error, which would end winit's connection with ours: wait for
            // the seat's capabilities first (one round trip, at window
            // creation, as `registry_queue_init` makes)
            queue.roundtrip(&mut heard).ok()?;
            if !heard.keyboard {
                log::info!("keyboard layout: the Wayland seat has no keyboard, no layout to follow");
                return None;
            }
            let keyboard = seat.get_keyboard(&queue.handle(), ());
            connection.flush().ok()?;
            log::info!("keyboard layout: following the active layout on the Wayland connection");
            Some(Self {
                queue: ManuallyDrop::new(queue),
                heard,
                seat,
                keyboard,
            })
        }

        /// The layout reported since the previous call: a new group, or the
        /// group's layout in a new keymap. Never waits: only the events
        /// winit's loop already read are dispatched.
        pub(in super::super) fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
            self.queue.dispatch_pending(&mut self.heard).map_err(|e| e.to_string())?;
            let Some(group) = self.heard.group.filter(|_| self.heard.changed) else {
                return Ok(None);
            };
            self.heard.changed = false;
            Ok(super::layout_of_group(&self.heard.layouts, group))
        }
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Heard {
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

    impl Drop for Watch {
        fn drop(&mut self) {
            if self.keyboard.version() >= 3 {
                self.keyboard.release();
            }
            if self.seat.version() >= 5 {
                self.seat.release();
            }
            // the queue is left alive on purpose (see `Watch`)
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for Heard {
        fn event(heard: &mut Self, _: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(capabilities),
            } = event
            {
                heard.keyboard = capabilities.contains(wl_seat::Capability::Keyboard);
            }
        }
    }

    impl Dispatch<wl_keyboard::WlKeyboard, ()> for Heard {
        fn event(heard: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            match event {
                // sent on every modifier change: only a new group counts
                wl_keyboard::Event::Modifiers { group, .. } if heard.group != Some(group) => {
                    heard.group = Some(group);
                    heard.changed = true;
                }
                event @ wl_keyboard::Event::Keymap { .. } => {
                    // sent when the keyboard is made, then on a new keymap
                    let context = xkb::xkbcommon_option().and_then(Context::new);
                    let keymap = context
                        .as_ref()
                        .zip(keymap_text(event))
                        .and_then(|(context, text)| compile(context, text));
                    if let Some(keymap) = keymap {
                        heard.layouts = super::watched_layouts(&keymap, "Wayland compositor");
                        heard.changed = true;
                    }
                }
                _ => {}
            }
        }
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
            // the keymap's own fd closes with the event
            if let Some(buf) = keymap_text(event) {
                text.0 = Some(buf);
            }
        }
    }
}

mod x11 {
    use std::ptr;
    use std::sync::mpsc;
    use std::time::Duration;

    use x11rb::connection::{Connection as _, RequestConnection as _};
    use x11rb::protocol::Event;
    use x11rb::protocol::xkb::{self as xkb_proto, ConnectionExt as _};
    use x11rb::xcb_ffi::{XCBConnection, load_libxcb};
    use xkbcommon_dl::x11::{self as xkb_x11, xkb_x11_setup_xkb_extension_flags::XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS};

    use super::{Context, KeyboardLayout, Keymap, xkb};

    /// A connection to the X server with libxkbcommon's XKB set up on it,
    /// and its core keyboard.
    fn connect() -> Option<(XCBConnection, &'static xkb_x11::XkbCommonX11, i32)> {
        let lib = xkb_x11::xkbcommon_x11_option()?;
        // libxcb is loaded on first use, and a failed lazy load panics:
        // load it here, where its absence only means no layout.
        load_libxcb().ok()?;
        let (connection, _) = XCBConnection::connect(None).ok()?;
        let raw = connection.get_raw_xcb_connection();
        // SAFETY: `raw` is the live XCB connection owned by `connection`,
        // which outlives these calls; the null out-parameters are optional.
        let device = unsafe {
            let ready = (lib.xkb_x11_setup_xkb_extension)(
                raw.cast(),
                xkb_x11::XKB_X11_MIN_MAJOR_XKB_VERSION,
                xkb_x11::XKB_X11_MIN_MINOR_XKB_VERSION,
                XKB_X11_SETUP_XKB_EXTENSION_NO_FLAGS,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
            let device = (lib.xkb_x11_get_core_keyboard_device_id)(raw.cast());
            if ready != 1 || device < 0 {
                return None;
            }
            device
        };
        Some((connection, lib, device))
    }

    /// The keymap of `device` and its active layout.
    fn read(context: &Context, connection: &XCBConnection, lib: &xkb_x11::XkbCommonX11, device: i32) -> Option<(Keymap, u32)> {
        let raw = connection.get_raw_xcb_connection().cast();
        // SAFETY: `raw` is the live XCB connection `connection` owns, with
        // XKB set up (`connect`); the keymap and state are released here or
        // by `Keymap`.
        unsafe {
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

    /// The X server's keymap, its active layout, and whether that layout
    /// only stands in for the active one: XWayland sets its group from the
    /// compositor once one of its windows has the focus, and reports the
    /// first layout until then.
    pub(super) fn keymap(context: &Context) -> Option<(Keymap, u32, bool)> {
        let (connection, lib, device) = connect()?;
        let (keymap, layout) = read(context, &connection, lib, device)?;
        let xwayland = matches!(connection.extension_information("XWAYLAND"), Ok(Some(_)));
        Some((keymap, layout, xwayland))
    }

    /// XKB's group and keymap changes, heard on a connection of our own
    /// (the group is the server's, the same for every client, and winit's
    /// connection is not ours to read), by a thread of its own: reading a
    /// new keymap or the group waits for the server, which the frame loop
    /// must not. The thread looks at the socket every `TICK` and at once
    /// when told of a focus; the frame loop only takes what it sent.
    pub(in super::super) struct Watch {
        layouts: mpsc::Receiver<Result<KeyboardLayout, String>>,
        focus: mpsc::Sender<()>,
    }

    /// How often the thread reads the events the server queued.
    const TICK: Duration = Duration::from_millis(100);

    impl Watch {
        pub(in super::super) fn new() -> Option<Self> {
            let (send, layouts) = mpsc::channel();
            let (focus, focused) = mpsc::channel();
            std::thread::Builder::new()
                .name("keyboard-layout-x11".into())
                .spawn(move || {
                    let error = match Listener::new() {
                        Ok(mut listener) => listener.run(&focused, &send),
                        Err(e) => e,
                    };
                    let _ = send.send(Err(error));
                })
                .map_err(|e| log::warn!("keyboard layout: no X11 watch thread: {e}"))
                .ok()?;
            Some(Self { layouts, focus })
        }

        /// The window got the focus: the thread asks for the group.
        pub(in super::super) fn focused(&mut self) {
            let _ = self.focus.send(());
        }

        /// The layout the thread reported since the last call. Never waits.
        pub(in super::super) fn poll(&mut self) -> Result<Option<KeyboardLayout>, String> {
            let mut last = None;
            loop {
                match self.layouts.try_recv() {
                    Ok(Ok(layout)) => last = Some(layout),
                    Ok(Err(e)) => return Err(e),
                    Err(mpsc::TryRecvError::Empty) => return Ok(last),
                    Err(mpsc::TryRecvError::Disconnected) => return Err("the X11 watch thread ended".into()),
                }
            }
        }
    }

    /// The thread's side: the connection and the keymap's layouts.
    struct Listener {
        connection: XCBConnection,
        lib: &'static xkb_x11::XkbCommonX11,
        device: i32,
        /// The layouts of the keymap, read again when it changes.
        layouts: Vec<KeyboardLayout>,
    }

    impl Listener {
        fn new() -> Result<Self, String> {
            let (connection, lib, device) = connect().ok_or("no XKB on the X server")?;
            // x11rb parses XKB's events once it knows the extension
            connection
                .extension_information(xkb_proto::X11_EXTENSION_NAME)
                .map_err(|e| e.to_string())?
                .ok_or("no XKB extension")?;
            let details = xkb_proto::SelectEventsAux::new().state_notify(xkb_proto::SelectEventsAuxStateNotify {
                affect_state: xkb_proto::StatePart::GROUP_STATE,
                state_details: xkb_proto::StatePart::GROUP_STATE,
            });
            let keymap_events = xkb_proto::EventType::NEW_KEYBOARD_NOTIFY | xkb_proto::EventType::MAP_NOTIFY;
            let keymap_parts = xkb_proto::MapPart::KEY_TYPES | xkb_proto::MapPart::KEY_SYMS;
            connection
                .xkb_select_events(
                    u16::try_from(device).map_err(|e| e.to_string())?,
                    xkb_proto::EventType::from(0u16),
                    keymap_events,
                    keymap_parts,
                    keymap_parts,
                    &details,
                )
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|e| e.to_string())?;
            let context = Context::new(xkb::xkbcommon_option().ok_or("no libxkbcommon")?).ok_or("no XKB context")?;
            let (keymap, _) = read(&context, &connection, lib, device).ok_or("the keymap could not be read")?;
            let layouts = super::watched_layouts(&keymap, "X server");
            log::info!("keyboard layout: following XKB's group and keymap on the X server");
            Ok(Self {
                connection,
                lib,
                device,
                layouts,
            })
        }

        /// Reports each layout made active until the watch is dropped, or
        /// returns why it stopped.
        fn run(&mut self, focused: &mpsc::Receiver<()>, send: &mpsc::Sender<Result<KeyboardLayout, String>>) -> String {
            loop {
                let read_group = match focused.recv_timeout(TICK) {
                    Ok(()) => true,
                    Err(mpsc::RecvTimeoutError::Timeout) => false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return "dropped".into(),
                };
                match self.step(read_group) {
                    Ok(Some(layout)) => {
                        if send.send(Ok(layout)).is_err() {
                            return "dropped".into();
                        }
                    }
                    Ok(None) => {}
                    Err(e) => return e,
                }
            }
        }

        /// The layout made active since the last step: a new group, or the
        /// active layout of a new keymap; with `read_group` (a focus), the
        /// group in use if no event told it.
        fn step(&mut self, read_group: bool) -> Result<Option<KeyboardLayout>, String> {
            let (mut group, mut new_keymap) = (None, false);
            while let Some(event) = self.connection.poll_for_event().map_err(|e| e.to_string())? {
                match event {
                    Event::XkbStateNotify(state) if i32::from(state.device_id) == self.device => {
                        group = Some(u32::from(u8::from(state.group)));
                    }
                    Event::XkbNewKeyboardNotify(_) | Event::XkbMapNotify(_) => new_keymap = true,
                    _ => {}
                }
            }
            if read_group && group.is_none() {
                let state = self
                    .connection
                    .xkb_get_state(u16::try_from(self.device).map_err(|e| e.to_string())?);
                let state = state.map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
                group = Some(u32::from(u8::from(state.group)));
            }
            if new_keymap {
                log::info!("keyboard layout: the X server's keymap changed");
                let context = xkb::xkbcommon_option().and_then(Context::new).ok_or("libxkbcommon is gone")?;
                let (keymap, active) = read(&context, &self.connection, self.lib, self.device).ok_or("the new keymap could not be read")?;
                self.layouts = super::watched_layouts(&keymap, "X server");
                group = Some(active);
            }
            Ok(group.and_then(|group| super::layout_of_group(&self.layouts, group)))
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
