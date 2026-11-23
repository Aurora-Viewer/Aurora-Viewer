//! A window outside brought in front on Wayland.
//!
//! No program takes the focus by itself there: it asks the compositor for
//! an activation token (xdg-activation), which the compositor honours when
//! the token proves that the user is at the program: it names the surface
//! with the keyboard and the serial of the event that gave it. winit asks
//! without a serial (`Window::request_user_attention`), and the compositor
//! then only signals the window: on GNOME a notice in the message list,
//! unseen with the banners off.
//!
//! So the viewer follows the keyboard's focus itself, with a seat and a
//! keyboard of its own on winit's connection, and asks with the surface
//! that has it and the serial of its `wl_keyboard.enter`: what mutter's
//! `token_can_activate` (src/wayland/meta-wayland-activation.c) accepts for
//! as long as that surface keeps the keyboard, whichever key or click asked.
//! A window brought in front while another program has the keyboard (a
//! message that arrives) proves nothing, and is left to winit's request:
//! the compositor signals it and steals no focus.
//!
//! The surfaces are winit's: they are named by their address, and read as
//! objects only from a window that is open when they are used.

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// The surface with the keyboard, by its address, and the serial of the
/// `enter` that gave it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Focus(Option<(usize, u32)>);

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl Focus {
    fn enter(&mut self, surface: usize, serial: u32) {
        self.0 = Some((surface, serial));
    }

    /// (the `leave` of a surface that lost the keyboard earlier changes
    /// nothing: the events of two surfaces can cross)
    fn leave(&mut self, surface: usize) {
        if self.0.is_some_and(|(focused, _)| focused == surface) {
            self.0 = None;
        }
    }

    /// The serial that proves the keyboard is in one of the surfaces
    /// `open`, with which one.
    fn proof(&self, open: impl IntoIterator<Item = usize>) -> Option<(usize, u32)> {
        let (focused, serial) = self.0?;
        open.into_iter().any(|surface| surface == focused).then_some((focused, serial))
    }
}

/// The viewer's own asking for a window to come in front. Does nothing
/// until [`Activation::start`] finds a Wayland session.
#[derive(Default)]
pub(crate) struct Activation {
    #[cfg(target_os = "linux")]
    wayland: Option<imp::Wayland>,
}

impl Activation {
    /// Follows the keyboard's focus from now on, on a Wayland session. Once
    /// for the session; every other call does nothing.
    pub fn start(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "linux")]
        if self.wayland.is_none() {
            self.wayland = imp::Wayland::new(event_loop);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = event_loop;
    }

    /// Once a frame: reads what the compositor answered. The tokens it
    /// gave, each with the key [`Activation::ask`] was given, for
    /// [`Activation::activate`].
    pub fn pump(&mut self) -> Vec<(u64, String)> {
        #[cfg(target_os = "linux")]
        if let Some(wayland) = &mut self.wayland {
            return wayland.pump();
        }
        Vec::new()
    }

    /// Asks the compositor for a token for the window `key`, when the
    /// keyboard is in one of the viewer's windows, `open`. False when it
    /// is not, or outside a Wayland session: nothing was asked.
    pub fn ask(&mut self, key: u64, open: &[&Window]) -> bool {
        #[cfg(target_os = "linux")]
        if let Some(wayland) = &mut self.wayland {
            return wayland.ask(key, open);
        }
        let _ = (key, open);
        false
    }

    /// Hands the compositor the token it gave for `window`, which it then
    /// brings in front.
    pub fn activate(&mut self, token: &str, window: &Window) {
        #[cfg(target_os = "linux")]
        if let Some(wayland) = &mut self.wayland {
            wayland.activate(token, window);
        }
        let _ = (token, window);
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::c_void;
    use std::ptr::NonNull;

    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::protocol::{wl_keyboard, wl_registry, wl_seat, wl_surface};
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
    use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
    use winit::event_loop::ActiveEventLoop;
    use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    use winit::window::Window;

    use super::Focus;

    /// What the compositor told our objects.
    #[derive(Default)]
    struct Seen {
        seat: Option<wl_seat::WlSeat>,
        keyboard: Option<wl_keyboard::WlKeyboard>,
        activation: Option<xdg_activation_v1::XdgActivationV1>,
        focus: Focus,
        /// The tokens given, with the key of the window each is for.
        done: Vec<(u64, String)>,
    }

    pub(super) struct Wayland {
        connection: Connection,
        queue: EventQueue<Seen>,
        seen: Seen,
    }

    /// The `wl_surface` of a window of winit's, on a Wayland session.
    fn surface(window: &Window) -> Option<NonNull<c_void>> {
        match window.window_handle().ok()?.as_raw() {
            RawWindowHandle::Wayland(handle) => Some(handle.surface),
            _ => None,
        }
    }

    fn address(surface: &wl_surface::WlSurface) -> usize {
        surface.id().as_ptr() as usize
    }

    impl Wayland {
        pub(super) fn new(event_loop: &ActiveEventLoop) -> Option<Self> {
            let RawDisplayHandle::Wayland(handle) = event_loop.display_handle().ok()?.as_raw() else {
                return None;
            };
            // SAFETY: the `wl_display` of winit's event loop, which lives
            // until that loop ends; this is dropped before, with the
            // windows outside (`Outside::exit`, from `App::exiting`).
            let backend = unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) };
            let connection = Connection::from_backend(backend);
            let queue = connection.new_event_queue();
            connection.display().get_registry(&queue.handle(), ());
            let _ = connection.flush();
            Some(Self {
                connection,
                queue,
                seen: Seen::default(),
            })
        }

        /// winit reads the connection; this only hands our objects what
        /// was read for them.
        pub(super) fn pump(&mut self) -> Vec<(u64, String)> {
            if let Err(e) = self.queue.dispatch_pending(&mut self.seen) {
                log::debug!("window outside: an event of the compositor was not read: {e}");
            }
            let _ = self.connection.flush();
            std::mem::take(&mut self.seen.done)
        }

        /// `surface` as an object of this connection.
        fn object(&self, surface: NonNull<c_void>) -> Option<wl_surface::WlSurface> {
            // SAFETY: the surface of a window of winit's that is open (the
            // callers take it from the window): a live `wl_surface` of the
            // display this connection is on, used before they return.
            let id = unsafe { ObjectId::from_ptr(wl_surface::WlSurface::interface(), surface.as_ptr().cast()) }.ok()?;
            wl_surface::WlSurface::from_id(&self.connection, id).ok()
        }

        pub(super) fn ask(&mut self, key: u64, open: &[&Window]) -> bool {
            let (Some(activation), Some(seat)) = (&self.seen.activation, &self.seen.seat) else {
                return false;
            };
            let surfaces: Vec<NonNull<c_void>> = open.iter().filter_map(|window| surface(window)).collect();
            let Some((focused, serial)) = self.seen.focus.proof(surfaces.iter().map(|s| s.as_ptr() as usize)) else {
                return false;
            };
            let Some(from) = surfaces
                .iter()
                .find(|s| s.as_ptr() as usize == focused)
                .and_then(|s| self.object(*s))
            else {
                return false;
            };
            let token = activation.get_activation_token(&self.queue.handle(), key);
            token.set_serial(serial, seat);
            token.set_surface(&from);
            token.commit();
            let _ = self.connection.flush();
            true
        }

        pub(super) fn activate(&mut self, token: &str, window: &Window) {
            let (Some(activation), Some(target)) = (&self.seen.activation, surface(window).and_then(|s| self.object(s))) else {
                return;
            };
            activation.activate(token.to_owned(), &target);
            let _ = self.connection.flush();
        }
    }

    impl Drop for Wayland {
        fn drop(&mut self) {
            if let Some(keyboard) = self.seen.keyboard.take()
                && keyboard.version() >= 3
            {
                keyboard.release();
            }
            if let Some(seat) = self.seen.seat.take()
                && seat.version() >= 5
            {
                seat.release();
            }
            if let Some(activation) = self.seen.activation.take() {
                activation.destroy();
            }
            let _ = self.connection.flush();
        }
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Seen {
        fn event(
            seen: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            queue: &QueueHandle<Self>,
        ) {
            let wl_registry::Event::Global { name, interface, version } = event else {
                return;
            };
            if interface == wl_seat::WlSeat::interface().name && seen.seat.is_none() {
                // (5: the seat and its keyboard are released at the end)
                seen.seat = Some(registry.bind(name, version.min(5), queue, ()));
            } else if interface == xdg_activation_v1::XdgActivationV1::interface().name && seen.activation.is_none() {
                seen.activation = Some(registry.bind(name, 1, queue, ()));
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for Seen {
        fn event(seen: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, queue: &QueueHandle<Self>) {
            let wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(capabilities),
            } = event
            else {
                return;
            };
            let has = capabilities.contains(wl_seat::Capability::Keyboard);
            if has && seen.keyboard.is_none() {
                seen.keyboard = Some(seat.get_keyboard(queue, ()));
            } else if !has && let Some(keyboard) = seen.keyboard.take() {
                if keyboard.version() >= 3 {
                    keyboard.release();
                }
                seen.focus = Focus::default();
            }
        }
    }

    impl Dispatch<wl_keyboard::WlKeyboard, ()> for Seen {
        fn event(seen: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            match event {
                wl_keyboard::Event::Enter { serial, surface, .. } => seen.focus.enter(address(&surface), serial),
                wl_keyboard::Event::Leave { surface, .. } => seen.focus.leave(address(&surface)),
                // (the keymap's file closes as it drops)
                _ => {}
            }
        }
    }

    impl Dispatch<xdg_activation_v1::XdgActivationV1, ()> for Seen {
        fn event(
            _: &mut Self,
            _: &xdg_activation_v1::XdgActivationV1,
            _: xdg_activation_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, u64> for Seen {
        fn event(
            seen: &mut Self,
            token: &xdg_activation_token_v1::XdgActivationTokenV1,
            event: xdg_activation_token_v1::Event,
            key: &u64,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_activation_token_v1::Event::Done { token: given } = event {
                seen.done.push((*key, given));
                token.destroy();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keyboard_in_an_open_window_is_the_proof() {
        let mut focus = Focus::default();
        assert_eq!(focus.proof([1, 2]), None, "no keyboard yet");
        focus.enter(1, 40);
        assert_eq!(focus.proof([1, 2]), Some((1, 40)));
        // the keyboard in a window that closed, or of another kind
        assert_eq!(focus.proof([2]), None);
        assert_eq!(focus.proof([]), None);
    }

    #[test]
    fn the_keyboard_follows_the_last_window_entered() {
        let mut focus = Focus::default();
        focus.enter(1, 40);
        // the events of two surfaces cross: the second is entered before
        // the first is told it was left
        focus.enter(2, 41);
        focus.leave(1);
        assert_eq!(focus.proof([1, 2]), Some((2, 41)));
        focus.leave(2);
        assert_eq!(focus.proof([1, 2]), None, "another program has the keyboard");
    }
}
