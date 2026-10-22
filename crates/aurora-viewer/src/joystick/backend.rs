//! Native raw controllers (gilrs-core) and 3Dconnexion HID reports.
//! HID layouts: FreeSpacenav's src/dev.c device table and PySpaceMouse's
//! device specifications: reports 1/2 carry LE i16 translation/rotation,
//! or report 1 carries all six axes; report 3 carries the button bitset.

use super::{AXES, Device, Snapshot};
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

pub struct Backend {
    state: Arc<Mutex<Snapshot>>,
    enabled: Arc<AtomicBool>,
}

impl Default for Backend {
    fn default() -> Self {
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let enabled = Arc::new(AtomicBool::new(false));
        let weak = Arc::downgrade(&state);
        let on = enabled.clone();
        if let Err(e) = std::thread::Builder::new().name("joystick-input".into()).spawn(move || {
            let mut worker = Worker::new();
            while let Some(shared) = weak.upgrade() {
                let snapshot = worker.poll(on.load(Ordering::Relaxed));
                *shared.lock() = snapshot;
                drop(shared);
                std::thread::sleep(Duration::from_millis(8));
            }
        }) {
            log::warn!("joystick input worker: {e}");
            state.lock().error = e.to_string();
        }
        Self { state, enabled }
    }
}

impl Backend {
    pub fn poll(&self, enabled: bool, latest: &mut Snapshot) {
        self.enabled.store(enabled, Ordering::Relaxed);
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

struct Worker {
    gilrs: Option<gilrs_core::Gilrs>,
    hidapi: Option<hidapi::HidApi>,
    pads: std::collections::BTreeMap<usize, Device>,
    mice: Vec<Hid>,
    scan_at: Instant,
    error: String,
}

impl Worker {
    fn new() -> Self {
        let mut error = Vec::new();
        let gilrs = gilrs_core::Gilrs::new().map_err(|e| error.push(e.to_string())).ok();
        let hidapi = hidapi::HidApi::new().map_err(|e| error.push(e.to_string())).ok();
        for e in &error {
            log::warn!("joystick backend: {e}");
        }
        Self {
            gilrs,
            hidapi,
            pads: Default::default(),
            mice: Vec::new(),
            scan_at: Instant::now() - Duration::from_secs(2),
            error: error.join("; "),
        }
    }

    fn poll(&mut self, enabled: bool) -> Snapshot {
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
                            state.axes[n] = normalize(value, info.min, info.max);
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
        let scanning = self.scan_at.elapsed() >= Duration::from_secs(1);
        if scanning {
            self.scan_at = Instant::now();
            if let Some(api) = self.hidapi.as_mut() {
                match api.refresh_devices() {
                    Ok(()) => {
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
                    Err(e) => log::debug!("SpaceMouse enumeration: {e}"),
                }
            }
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
}
