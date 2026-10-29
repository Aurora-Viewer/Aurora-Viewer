//! Raw mouse motion of the devices that give a position, not a motion.
//!
//! Remote desktops (Shadow, Remote Desktop, Parsec…), virtual machines and
//! pen tablets hand the pointer to Windows as an absolute position
//! (`MOUSE_MOVE_ABSOLUTE`, 0 to 65535 across the screen). winit 0.30 takes
//! every raw packet for a motion: its test, `has_flag(usFlags,
//! MOUSE_MOVE_RELATIVE)`, is of a flag worth 0 and always true. The position
//! came as a motion of thousands of pixels, and every camera drag (holding
//! our avatar or its name tag, Alt, mouselook, the right drag) threw the view
//! to the sky or the ground at the first touch of the touchpad.
//!
//! Like LLWindowWin32's `WM_INPUT` (indra/llwindow/llwindowwin32.cpp,
//! originally LGPL 2.1), the position is scaled to the screen and the motion
//! is its difference with the previous one. The packet is read in winit's
//! message hook, just before winit handles the same message, and the
//! `MouseMotion` winit then sends takes the motion computed here ([`motion`]).
//! Unlike the Legacy Viewer, which measures the first position from the
//! corner of the screen, the first position of a device moves nothing.

/// The packet gives a position (`MOUSE_MOVE_ABSOLUTE`).
const ABSOLUTE: u16 = 0x01;
/// The position spans the whole virtual desktop (`MOUSE_VIRTUAL_DESKTOP`).
const VIRTUAL_DESKTOP: u16 = 0x02;
/// Not in the SDK headers: the Legacy Viewer reads the position of such a
/// packet (touch screens) in pixels, not in 0 to 65535.
const PIXELS: u16 = 0x10;
/// The end of the normalised range of an absolute position.
const RANGE: f64 = 65535.0;

/// One raw mouse packet.
#[derive(Debug, Clone, Copy)]
pub struct Packet {
    /// The device that sent it (`RAWINPUTHEADER::hDevice`).
    pub device: isize,
    pub flags: u16,
    pub x: i32,
    pub y: i32,
}

/// The last position of the absolute device that moved last.
#[derive(Debug, Default)]
pub struct Absolute {
    last: Option<(isize, f64, f64)>,
}

impl Absolute {
    /// The motion of `packet` in pixels, when it gives a position; None for
    /// a relative packet, whose motion is winit's. `screen(virtual_desktop)`:
    /// the size in pixels of the primary screen or of the virtual desktop.
    pub fn packet(&mut self, packet: Packet, screen: impl FnOnce(bool) -> (f64, f64)) -> Option<(f64, f64)> {
        if packet.flags & ABSOLUTE == 0 {
            // another device moved: the next position starts afresh
            self.last = None;
            return None;
        }
        let (x, y) = if packet.flags & PIXELS != 0 {
            (f64::from(packet.x), f64::from(packet.y))
        } else {
            let (width, height) = screen(packet.flags & VIRTUAL_DESKTOP != 0);
            (f64::from(packet.x) / RANGE * width, f64::from(packet.y) / RANGE * height)
        };
        let motion = match self.last {
            Some((device, lx, ly)) if device == packet.device => (x - lx, y - ly),
            _ => (0.0, 0.0),
        };
        self.last = Some((packet.device, x, y));
        Some(motion)
    }
}

#[cfg(windows)]
thread_local! {
    /// The motion of the absolute packet winit is handling, set by the
    /// message hook for each raw packet (None: a relative one).
    static PENDING: std::cell::Cell<Option<(f64, f64)>> = const { std::cell::Cell::new(None) };
}

/// The motion to use for winit's `MouseMotion` of `delta`: its own for a
/// relative device, the one computed from the position for an absolute
/// one; None when it moves nothing.
pub fn motion(delta: (f64, f64)) -> Option<(f64, f64)> {
    #[cfg(windows)]
    if let Some(motion) = PENDING.with(std::cell::Cell::take) {
        return (motion != (0.0, 0.0)).then_some(motion);
    }
    Some(delta)
}

/// Read each raw packet before winit does: winit's message hook runs on the
/// main thread for each message taken from the queue, before its dispatch.
#[cfg(windows)]
pub fn install(builder: &mut winit::event_loop::EventLoopBuilder<()>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_INPUT};
    use winit::platform::windows::EventLoopBuilderExtWindows;

    let mut absolute = Absolute::default();
    builder.with_msg_hook(move |msg| {
        // SAFETY: winit gives the `MSG` it has just taken from the queue,
        // alive for the duration of the call.
        let msg = unsafe { &*msg.cast::<MSG>() };
        if msg.message == WM_INPUT {
            let motion = read(msg.lParam).and_then(|packet| absolute.packet(packet, screen));
            PENDING.with(|pending| pending.set(motion));
        }
        // winit handles the message as usual
        false
    });
}

/// The mouse packet of a `WM_INPUT`, None for another device or an error.
/// The data stays readable until the message is handled: winit reads it again.
#[cfg(windows)]
fn read(lparam: isize) -> Option<Packet> {
    use windows_sys::Win32::UI::Input::{GetRawInputData, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE};

    let mut data = RAWINPUT::default();
    let mut size = std::mem::size_of::<RAWINPUT>() as u32;
    // SAFETY: `lparam` is the HRAWINPUT of a `WM_INPUT` not handled yet;
    // `data` is a RAWINPUT of the `size` given, which a mouse packet fits.
    let read = unsafe {
        GetRawInputData(
            lparam as _,
            RID_INPUT,
            (&raw mut data).cast(),
            &mut size,
            std::mem::size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == 0 || read == u32::MAX || data.header.dwType != RIM_TYPEMOUSE {
        return None;
    }
    // SAFETY: a mouse packet: the union holds a RAWMOUSE.
    let mouse = unsafe { data.data.mouse };
    Some(Packet {
        device: data.header.hDevice as isize,
        flags: mouse.usFlags,
        x: mouse.lLastX,
        y: mouse.lLastY,
    })
}

/// The size of the primary screen, or of the virtual desktop, in pixels.
#[cfg(windows)]
fn screen(virtual_desktop: bool) -> (f64, f64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN};

    let (x, y) = if virtual_desktop {
        (SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN)
    } else {
        (SM_CXSCREEN, SM_CYSCREEN)
    };
    // SAFETY: `GetSystemMetrics` only reads a system value.
    unsafe { (f64::from(GetSystemMetrics(x)), f64::from(GetSystemMetrics(y))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (f64, f64) = (1920.0, 1080.0);

    fn packet(device: isize, flags: u16, x: i32, y: i32) -> Packet {
        Packet { device, flags, x, y }
    }

    #[test]
    fn a_relative_packet_keeps_the_motion_winit_reads() {
        let mut abs = Absolute::default();
        assert_eq!(abs.packet(packet(1, 0, 5, -3), |_| SCREEN), None);
    }

    #[test]
    fn a_position_becomes_its_difference_with_the_last_in_pixels() {
        let mut abs = Absolute::default();
        // the first touch, in the middle of the screen: no motion, not a
        // jump from the corner
        assert_eq!(abs.packet(packet(1, ABSOLUTE, 32768, 32768), |_| SCREEN), Some((0.0, 0.0)));
        // a tenth of the screen to the right and down
        let (dx, dy) = abs
            .packet(packet(1, ABSOLUTE, 32768 + 6554, 32768 + 6554), |_| SCREEN)
            .expect("absolute");
        assert!((dx - 192.0).abs() < 0.1, "{dx}");
        assert!((dy - 108.0).abs() < 0.1, "{dy}");
        // back up: a negative motion
        let (_, dy) = abs.packet(packet(1, ABSOLUTE, 32768 + 6554, 32768), |_| SCREEN).expect("absolute");
        assert!((dy + 108.0).abs() < 0.1, "{dy}");
    }

    #[test]
    fn the_virtual_desktop_flag_scales_to_the_virtual_desktop() {
        let mut abs = Absolute::default();
        let flags = ABSOLUTE | VIRTUAL_DESKTOP;
        let screen = |virtual_desktop: bool| if virtual_desktop { (3840.0, 1080.0) } else { SCREEN };
        abs.packet(packet(1, flags, 0, 0), screen);
        let (dx, _) = abs.packet(packet(1, flags, 65535, 0), screen).expect("absolute");
        assert!((dx - 3840.0).abs() < 0.01, "{dx}");
    }

    #[test]
    fn a_touch_position_is_already_in_pixels() {
        let mut abs = Absolute::default();
        abs.packet(packet(1, ABSOLUTE | PIXELS, 100, 100), |_| SCREEN);
        assert_eq!(abs.packet(packet(1, ABSOLUTE | PIXELS, 110, 95), |_| SCREEN), Some((10.0, -5.0)));
    }

    #[test]
    fn another_device_or_a_relative_packet_starts_afresh() {
        let mut abs = Absolute::default();
        abs.packet(packet(1, ABSOLUTE, 1000, 1000), |_| SCREEN);
        // a second absolute device far away: no jump between the two
        assert_eq!(abs.packet(packet(2, ABSOLUTE, 60000, 60000), |_| SCREEN), Some((0.0, 0.0)));
        // a relative mouse in between: the next position does not jump either
        abs.packet(packet(3, 0, 4, 4), |_| SCREEN);
        assert_eq!(abs.packet(packet(2, ABSOLUTE, 1000, 1000), |_| SCREEN), Some((0.0, 0.0)));
    }

    #[test]
    fn without_a_hook_the_motion_is_winits() {
        assert_eq!(motion((3.0, -2.0)), Some((3.0, -2.0)));
    }
}
