//! Native raw controllers (gilrs-core) and 3Dconnexion HID reports.
//! HID layouts: FreeSpacenav's src/dev.c device table and PySpaceMouse's
//! device specifications: reports 1/2 carry LE i16 translation/rotation,
//! or report 1 carries all six axes; report 3 carries the button bitset.
//!
//! Nothing is enumerated or opened while the input is off and its
//! preferences are closed: like LLViewerJoystick::init, called at start-up
//! only with JoystickEnabled and by LLFloaterJoystick::postBuild
//! (indra/newview/llappviewer.cpp, llfloaterjoystick.cpp). A HID scan asks
//! every device for its strings, real USB requests that make some gaming
//! mice reset (disconnect / reconnect) when repeated, so the scans only
//! ever read the strings of 3D mice.

use super::{AXES, Device, Snapshot};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::{Duration, Instant};

/// What the frame needs from the devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Use {
    /// No controller library, no scan, no open device.
    Off,
    /// The preferences list the devices; 3D mice stay closed.
    List,
    /// The input is enabled.
    On,
}

pub struct Backend {
    state: Arc<Mutex<Snapshot>>,
    mode: Arc<AtomicU8>,
}

impl Default for Backend {
    fn default() -> Self {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let mode = Arc::new(AtomicU8::new(Use::Off as u8));
        let weak = Arc::downgrade(&state);
        let wanted = mode.clone();
        if let Err(e) = std::thread::Builder::new().name("joystick-input".into()).spawn(move || {
            let mut worker = Worker::default();
            while let Some(shared) = weak.upgrade() {
                let mode = match wanted.load(Ordering::Relaxed) {
                    2 => Use::On,
                    1 => Use::List,
                    _ => Use::Off,
                };
                let snapshot = worker.poll(mode);
                *shared.lock() = snapshot;
                drop(shared);
                // Idle, only the mode is watched.
                std::thread::sleep(Duration::from_millis(if mode == Use::Off { 100 } else { 8 }));
            }
        }) {
            log::warn!("joystick input worker: {e}");
            state.lock().error = e.to_string();
        }
        Self { state, mode }
    }
}

impl Backend {
    pub fn poll(&self, mode: Use, latest: &mut Snapshot) {
        self.mode.store(mode as u8, Ordering::Relaxed);
        if let Some(s) = self.state.try_lock() {
            latest.clone_from(&s);
        }
    }
}

struct Hid {
    path: std::ffi::CString,
    info: Device,
    handle: Option<hidapi::HidDevice>,
    axis_at: [Instant; 2],
}

#[derive(Default)]
struct Worker {
    /// Started with the input or its preferences; dropping it stops the
    /// polling thread gilrs-core runs on Windows.
    gilrs: Option<gilrs_core::Gilrs>,
    /// Created at the first 3D mouse found, then kept: hidapi has no
    /// context without a first unfiltered enumeration.
    hidapi: Option<hidapi::HidApi>,
    started: bool,
    pads: std::collections::BTreeMap<usize, Device>,
    mice: Vec<Hid>,
    scan_at: Option<Instant>,
    error: String,
}

impl Worker {
    fn start(&mut self) {
        self.started = true;
        match gilrs_core::Gilrs::new() {
            Ok(g) => self.gilrs = Some(g),
            Err(e) => {
                log::warn!("joystick backend: {e}");
                self.error = e.to_string();
            }
        }
    }

    fn stop(&mut self) {
        self.started = false;
        self.gilrs = None;
        self.pads.clear();
        self.mice.clear();
        self.scan_at = None;
        self.error.clear();
    }

    /// Re-lists the 3D mice, asking nothing of the other HID devices.
    fn scan(&mut self) {
        let wanted = present_space_mice().unwrap_or_else(|| SPACE_MOUSE_VENDORS.map(|v| (v, 0)).to_vec());
        if wanted.is_empty() {
            self.mice.clear();
            return;
        }
        if self.hidapi.is_none() {
            // The one unfiltered enumeration of the session (hidapi's
            // constructor always makes it), and only with a 3D mouse present.
            match hidapi::HidApi::new() {
                Ok(api) => self.hidapi = Some(api),
                Err(e) => {
                    log::warn!("joystick backend: {e}");
                    self.error = e.to_string();
                    return;
                }
            }
        }
        let Some(api) = self.hidapi.as_mut() else {
            return;
        };
        let listed = api
            .reset_devices()
            .and_then(|()| wanted.iter().try_for_each(|&(vendor, product)| api.add_devices(vendor, product)));
        if let Err(e) = listed {
            log::debug!("SpaceMouse enumeration: {e}");
            return;
        }
        self.mice.retain(|m| api.device_list().any(|d| d.path() == m.path.as_c_str()));
        for d in api
            .device_list()
            .filter(|d| space_mouse(d.vendor_id(), d.product_id()) && d.usage_page() == 1 && d.usage() == 8)
        {
            if self.mice.iter().any(|m| m.path.as_c_str() == d.path()) {
                continue;
            }
            // Persist a path hash, never the OS path or device serial.
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            d.path().hash(&mut hash);
            let info = Device::new(
                format!("ndof:{:04x}:{:04x}:{:x}", d.vendor_id(), d.product_id(), hash.finish()),
                d.product_string().unwrap_or("SpaceMouse").to_owned(),
                6,
            );
            self.mice.push(Hid {
                path: d.path().to_owned(),
                info,
                handle: None,
                axis_at: [Instant::now(); 2],
            });
        }
    }

    fn poll(&mut self, mode: Use) -> Snapshot {
        if mode == Use::Off {
            if self.started {
                self.stop();
            }
            return Snapshot {
                updated: Some(Instant::now()),
                ..Snapshot::default()
            };
        }
        if !self.started {
            self.start();
        }
        let enabled = mode == Use::On;
        if let Some(g) = self.gilrs.as_mut() {
            // Keep native ordering stable; SpaceMouse is handled by HID only.
            for id in 0..g.last_gamepad_hint() {
                if let Some(pad) = g.gamepad(id).filter(|p| p.is_connected())
                    && !pad.vendor_id().zip(pad.product_id()).is_some_and(|(v, p)| space_mouse(v, p))
                {
                    self.pads.entry(id).or_insert_with(|| {
                        Device::new(
                            format!("joystick:{}:{id}", uuid::Uuid::from_bytes(pad.uuid())),
                            pad.name().to_owned(),
                            pad.axes().len(),
                        )
                    });
                }
            }
            for _ in 0..1024 {
                let Some(event) = g.next_event() else {
                    break;
                };
                use gilrs_core::EventType as E;
                if matches!(event.event, E::Disconnected) {
                    self.pads.remove(&event.id);
                    continue;
                }
                let Some((pad, state)) = g.gamepad(event.id).zip(self.pads.get_mut(&event.id)) else {
                    continue;
                };
                match event.event {
                    E::AxisValueChanged(value, code) => {
                        if let Some(n) = pad.axes().iter().position(|c| *c == code).filter(|n| *n < AXES)
                            && let Some(info) = pad.axis_info(code)
                        {
                            state.axes[n] = normalize_axis(value, info.min, info.max, code);
                        }
                    }
                    E::ButtonPressed(code) | E::ButtonReleased(code) => {
                        if let Some(n) = pad.buttons().iter().position(|c| *c == code) {
                            state.button(n, matches!(event.event, E::ButtonPressed(_)));
                        }
                    }
                    _ => {}
                }
            }
        }
        let scanning = self.scan_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(1));
        if scanning {
            self.scan_at = Some(Instant::now());
            self.scan();
        }
        for m in &mut self.mice {
            if !enabled {
                m.handle = None;
                m.info.axes = [0.0; AXES];
                m.info.buttons = 0;
                continue;
            }
            if m.handle.is_none()
                && scanning
                && let Some(api) = &self.hidapi
            {
                match api.open_path(&m.path) {
                    Ok(handle) => {
                        m.handle = Some(handle);
                        if self.gilrs.is_some() {
                            self.error.clear();
                        }
                    }
                    Err(e) => {
                        log::debug!("SpaceMouse open: {e}");
                        self.error = e.to_string();
                    }
                }
            }
            if let Some(handle) = &m.handle {
                let mut data = [0u8; 64];
                for _ in 0..64 {
                    match handle.read_timeout(&mut data, 0) {
                        Ok(0) => break,
                        Ok(n) => {
                            let changed = decode_hid(&mut m.info, &data[..n]);
                            for (i, moved) in changed.into_iter().enumerate() {
                                if moved {
                                    m.axis_at[i] = Instant::now();
                                }
                            }
                        }
                        Err(e) => {
                            log::debug!("SpaceMouse read: {e}");
                            self.error = e.to_string();
                            m.handle = None;
                            m.info.axes = [0.0; AXES];
                            m.info.buttons = 0;
                            break;
                        }
                    }
                }
            }
            // Wireless receivers may disappear without an unplug event or a
            // final zero packet. Never integrate the last displacement forever.
            for i in 0..2 {
                if m.axis_at[i].elapsed() > Duration::from_millis(250) {
                    m.info.axes[i * 3..i * 3 + 3].fill(0.0);
                }
            }
        }
        Snapshot {
            devices: self
                .mice
                .iter()
                .filter(|m| !enabled || m.handle.is_some())
                .map(|m| m.info.clone())
                .chain(self.pads.values().cloned())
                .take(16)
                .collect(),
            updated: Some(Instant::now()),
            error: self.error.clone(),
        }
    }
}

const SPACE_MOUSE_VENDORS: [u16; 2] = [0x046d, 0x256f];

/// The 3D mice plugged in, read from the HID interface paths Windows
/// lists, without opening any device. `None` where the paths cannot be
/// read: the scan then filters by vendor (on Linux and macOS, hidapi reads
/// the strings from the system's cache, not from the devices).
fn present_space_mice() -> Option<Vec<(u16, u16)>> {
    #[cfg(windows)]
    match hid_interface_paths() {
        Ok(paths) => {
            let mut ids: Vec<_> = paths
                .iter()
                .filter_map(|p| path_ids(p))
                .filter(|&(v, p)| space_mouse(v, p))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            return Some(ids);
        }
        Err(e) => log::debug!("HID interface list: {e}"),
    }
    None
}

/// The present HID interfaces, as hidapi's own hid_enumerate lists them
/// before opening each one (etc/hidapi/windows/hid.c).
#[cfg(windows)]
fn hid_interface_paths() -> Result<Vec<String>, String> {
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW, CM_Get_Device_Interface_ListW, CR_BUFFER_SMALL,
        CR_SUCCESS,
    };
    // GUID_DEVINTERFACE_HID (hidclass.h), without windows-sys' whole HID module.
    const HID: windows_sys::core::GUID = windows_sys::core::GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);
    loop {
        let mut len = 0u32;
        // SAFETY: `len` and the GUID outlive the call; a null device id
        // lists the interfaces of every device.
        let cr = unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, &HID, std::ptr::null(), CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        if cr != CR_SUCCESS {
            return Err(format!("list size: CONFIGRET {cr}"));
        }
        let mut list = vec![0u16; len as usize];
        // SAFETY: `list` holds the `len` UTF-16 units the call may write.
        let cr =
            unsafe { CM_Get_Device_Interface_ListW(&HID, std::ptr::null(), list.as_mut_ptr(), len, CM_GET_DEVICE_INTERFACE_LIST_PRESENT) };
        match cr {
            // A double-NUL-terminated list of NUL-terminated paths.
            CR_SUCCESS => {
                return Ok(list
                    .split(|c| *c == 0)
                    .filter(|p| !p.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect());
            }
            // A device arrived between the two calls.
            CR_BUFFER_SMALL => {}
            _ => return Err(format!("list: CONFIGRET {cr}")),
        }
    }
}

/// Vendor and product of a HID interface path: `vid_256f&pid_c635` over
/// USB, `vid&0002256f_pid&c63a` over Bluetooth (the vendor id prefixed by
/// its source).
fn path_ids(path: &str) -> Option<(u16, u16)> {
    let path = path.to_ascii_lowercase();
    let digits = |key: &str| {
        let at = path.find(&format!("{key}_")).or_else(|| path.find(&format!("{key}&")))? + key.len() + 1;
        let run = path[at..].bytes().take_while(u8::is_ascii_hexdigit).count();
        (run >= 4).then(|| &path[at..at + run])
    };
    let vendor = digits("vid")?;
    let product = digits("pid")?;
    Some((
        u16::from_str_radix(&vendor[vendor.len() - 4..], 16).ok()?,
        u16::from_str_radix(&product[..4], 16).ok()?,
    ))
}

fn normalize_axis(value: i32, min: i32, max: i32, code: gilrs_core::EvCode) -> f32 {
    use gilrs_core::native_ev_codes::{AXIS_LT2, AXIS_RT2};
    // Windows gamepad triggers report pressure, with zero at release
    // (gilrs-core's windows_wgi/gamepad.rs::axis_info). Treating that
    // range as a centred stick made release -1, so relative zoom could not
    // arm and would keep zooming after the trigger was released.
    if cfg!(windows) && min >= 0 && max > min && matches!(code, AXIS_LT2 | AXIS_RT2) {
        return ((f64::from(value) - f64::from(min)) / (f64::from(max) - f64::from(min))).clamp(0.0, 1.0) as f32;
    }
    normalize(value, min, max)
}

fn normalize(value: i32, min: i32, max: i32) -> f32 {
    if max <= min {
        return 0.0;
    }
    // Signed native ranges have one more negative value. Preserve the
    // exact zero, including with a user-selected zero dead zone.
    if min < 0 && max > 0 {
        return (if value >= 0 {
            f64::from(value) / f64::from(max)
        } else {
            -f64::from(value) / f64::from(min)
        } as f32)
            .clamp(-1.0, 1.0);
    }
    ((2.0 * (f64::from(value) - f64::from(min)) / (f64::from(max) - f64::from(min)) - 1.0) as f32).clamp(-1.0, 1.0)
}

fn space_mouse(vendor: u16, product: u16) -> bool {
    match vendor {
        0x046d => matches!(product, 0xc621 | 0xc623 | 0xc625..=0xc629 | 0xc62b),
        0x256f => matches!(
            product,
            0xc62e | 0xc62f | 0xc631..=0xc633 | 0xc635 | 0xc636 | 0xc638 | 0xc63a | 0xc652
        ),
        _ => false,
    }
}

fn decode_hid(state: &mut Device, data: &[u8]) -> [bool; 2] {
    let mut changed = [false; 2];
    match data.first().copied() {
        Some(1 | 2) if data.len() >= 7 => {
            let start = if data[0] == 1 { 0 } else { 3 };
            let count = if data[0] == 1 && data.len() >= 13 { 6 } else { 3 };
            for n in 0..count {
                state.axes[start + n] = (f32::from(i16::from_le_bytes([data[1 + n * 2], data[2 + n * 2]])) / 350.0).clamp(-1.0, 1.0);
                // HID has positive Z down. Normalize Z, Rx and Ry to the
                // common controller convention, keeping native axis numbers.
                // See PySpaceMouse's documented HID axis conventions
                // (troubleshooting.md).
                if matches!(start + n, 2..=4) {
                    state.axes[start + n] = -state.axes[start + n];
                }
            }
            changed[start / 3] = true;
            if count == 6 {
                changed[1] = true;
            }
        }
        Some(3) if data.len() >= 2 => {
            let bits = data[1..]
                .iter()
                .take(4)
                .enumerate()
                .fold(0u32, |bits, (n, b)| bits | u32::from(*b) << (n * 8));
            for n in 0..32 {
                state.button(n, bits & (1 << n) != 0);
            }
        }
        _ => {}
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn joystick_trigger_pressure_releases_at_zero_and_arms_relative_flycam_zoom() {
        use crate::joystick::{Controller, Settings, Snapshot};
        use gilrs_core::native_ev_codes::{AXIS_LSTICKX, AXIS_LT2, AXIS_RT2};
        assert_eq!(normalize_axis(50, 0, 100, AXIS_LSTICKX), 0.0, "unsigned sticks stay centred");
        for code in [AXIS_LT2, AXIS_RT2] {
            assert_eq!(normalize_axis(0, 0, i32::MAX, code), 0.0);
            assert_eq!(normalize_axis(i32::MAX, 0, i32::MAX, code), 1.0);
            let mut settings = Settings {
                enabled: true,
                mapping: [-1; super::super::CHANNELS],
                ..Default::default()
            };
            settings.mapping[6] = 0;
            settings.flycam.gain[6] = 1.0;
            settings.flycam.feather = 0.0;
            let mut snapshot = Snapshot {
                devices: vec![Device::new("test".into(), "Trigger".into(), 1)],
                updated: Some(Instant::now()),
                ..Default::default()
            };
            let mut controller = Controller::default();
            let mut camera = crate::joystick::flycam::Flycam::from_view(glam::Vec3::ZERO, glam::Vec3::X, glam::Vec3::Z, 1.0);
            controller.sample(&snapshot, &settings, true, true, 0.1);
            snapshot.devices[0].axes[0] = normalize_axis(i32::MAX, 0, i32::MAX, code);
            camera.step(controller.sample(&snapshot, &settings, true, true, 0.1).axes, &settings, 0.1);
            assert!(
                camera.fov < 0.95,
                "pressing a trigger must change the lens without crossing a false centre"
            );
            let held = camera.fov;
            snapshot.devices[0].axes[0] = normalize_axis(0, 0, i32::MAX, code);
            camera.step(controller.sample(&snapshot, &settings, true, true, 0.1).axes, &settings, 0.1);
            assert_eq!(camera.fov, held, "release must stop relative zoom");
        }
    }

    #[test]
    fn joystick_native_ranges_do_not_overflow() {
        assert!((normalize(0, -32768, 32767)).abs() < 0.0001);
        assert_eq!(normalize(i32::MAX, i32::MIN, i32::MAX), 1.0);
        assert_eq!(normalize(10, 10, 10), 0.0);
        assert_eq!(normalize(-999, 0, 100), -1.0);
    }

    #[test]
    fn spacemouse_split_combined_truncated_and_button_reports() {
        let mut d = Device::new("test".into(), "test".into(), 6);
        let mut report = vec![1];
        for v in [350i16, -350, 175, -175, 0, 350] {
            report.extend(v.to_le_bytes());
        }
        assert_eq!(decode_hid(&mut d, &report), [true, true]);
        assert_eq!(&d.axes[..6], &[1.0, -1.0, -0.5, 0.5, 0.0, 1.0]);
        assert_eq!(decode_hid(&mut d, &[2, 0, 0, 0, 0, 0, 0]), [false, true]);
        assert_eq!(&d.axes[3..6], &[0.0; 3]);
        let before = d.axes;
        assert_eq!(decode_hid(&mut d, &[1, 0, 0]), [false; 2]);
        assert_eq!(d.axes, before);
        decode_hid(&mut d, &[3, 1, 0, 0, 128]);
        assert_eq!(d.buttons, 0x80000001);
        decode_hid(&mut d, &[3, 1, 0, 0, 128]);
        assert_eq!(d.presses[0], 1);
        decode_hid(&mut d, &[3, 0]);
        assert_eq!(d.buttons, 0);
        assert!(!space_mouse(0x256f, 0xc650)); // CadMouse is an ordinary mouse.
    }

    #[test]
    fn spacemouse_ids_from_usb_and_bluetooth_interface_paths() {
        let usb = r"\\?\HID#VID_256F&PID_C635&MI_00#8&2a2c1f0b&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(path_ids(usb), Some((0x256f, 0xc635)));
        let classic =
            r"\\?\HID#{00001124-0000-1000-8000-00805f9b34fb}_VID&0002256f_PID&c63a#9&1&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(path_ids(classic), Some((0x256f, 0xc63a)));
        let le = r"\\?\HID#{00001812-0000-1000-8000-00805f9b34fb}_Dev_VID&02256f_PID&c63a_REV&0100_d0a1#9&2&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(path_ids(le), Some((0x256f, 0xc63a)));
        // A gaming mouse is never a 3D mouse, and odd paths are skipped.
        let mouse = path_ids(r"\\?\HID#VID_0DB0&PID_0B4F&MI_00#7&1&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}");
        assert!(mouse.is_some_and(|(v, p)| !space_mouse(v, p)));
        assert_eq!(path_ids(r"\\?\HID#ConvertedDevice&Col01#5&1&0&0000#{4d1e55b2}"), None);
        assert_eq!(path_ids(r"\\?\HID#VID_25&PID_C635"), None);
    }
}
