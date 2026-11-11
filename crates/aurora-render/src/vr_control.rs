//! Requests cross to the rendering worker without moving graphics/session ownership.
use std::{
    sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    time::Instant,
};

use parking_lot::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VrState {
    Unavailable,
    Off,
    Starting,
    Active,
    Stopping,
    Failed,
}
impl VrState {
    pub fn engaged(self) -> bool {
        matches!(self, Self::Starting | Self::Active | Self::Stopping)
    }
}
pub(crate) struct Control {
    wanted: AtomicBool,
    state: AtomicU8,
    yaw_delta: AtomicU32,
    translation: RwLock<glam::Vec3>,
    input: RwLock<crate::VrInputState>,
    pointer: RwLock<crate::VrPointer>,
    display: RwLock<crate::VrDisplay>,
    /// Pending vibration per hand: (amplitude, seconds).
    haptics: parking_lot::Mutex<[Option<(f32, f32)>; 2]>,
    recenter: AtomicBool,
    /// World-space shift of the view (ahead of the eyes, height), applied on
    /// top of the anchor each frame, so re-anchoring keeps it.
    offset: RwLock<glam::Vec3>,
    panel: RwLock<Option<crate::VrPanel>>,
    /// The hover tip's label by the beam (part of the interface image).
    tip: RwLock<Option<crate::VrPanel>>,
    /// The wrist's chat panel above the left hand (part of the interface image).
    wrist: RwLock<Option<crate::VrPanel>>,
}
impl Control {
    pub fn new(available: bool) -> Self {
        Self {
            wanted: AtomicBool::new(false),
            state: AtomicU8::new(if available { VrState::Off } else { VrState::Unavailable } as u8),
            yaw_delta: AtomicU32::new(0.0f32.to_bits()),
            translation: RwLock::new(glam::Vec3::ZERO),
            input: RwLock::new(crate::VrInputState::default()),
            pointer: RwLock::new(crate::VrPointer::default()),
            display: RwLock::new(crate::VrDisplay::default()),
            haptics: parking_lot::Mutex::new([None; 2]),
            recenter: AtomicBool::new(false),
            offset: RwLock::new(glam::Vec3::ZERO),
            panel: RwLock::new(None),
            tip: RwLock::new(None),
            wrist: RwLock::new(None),
        }
    }
    pub fn set_display(&self, display: crate::VrDisplay) {
        *self.display.write() = display;
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn display(&self) -> crate::VrDisplay {
        *self.display.read()
    }
    /// A pulse on one hand; a stronger pending one is kept.
    pub fn haptic(&self, side: crate::VrSide, amplitude: f32, seconds: f32) {
        if !(amplitude.is_finite() && seconds.is_finite()) {
            return;
        }
        let slot = &mut self.haptics.lock()[side as usize];
        if slot.is_none_or(|(a, _)| a < amplitude) {
            *slot = Some((amplitude, seconds));
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn take_haptics(&self) -> [Option<(f32, f32)>; 2] {
        std::mem::take(&mut *self.haptics.lock())
    }
    pub fn recenter(&self) {
        self.recenter.store(true, Ordering::Release);
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn take_recenter(&self) -> bool {
        self.recenter.swap(false, Ordering::AcqRel)
    }
    pub fn set_offset(&self, offset: glam::Vec3) {
        if offset.is_finite() {
            *self.offset.write() = offset;
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn offset(&self) -> glam::Vec3 {
        *self.offset.read()
    }
    pub fn state(&self) -> VrState {
        match self.state.load(Ordering::Acquire) {
            1 => VrState::Off,
            2 => VrState::Starting,
            3 => VrState::Active,
            4 => VrState::Stopping,
            5 => VrState::Failed,
            _ => VrState::Unavailable,
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
    pub fn wanted(&self) -> bool {
        self.wanted.load(Ordering::Acquire)
    }
    pub fn request(&self, on: bool) -> bool {
        if self.state() == VrState::Unavailable {
            return false;
        }
        self.wanted.store(on, Ordering::Release);
        self.state.store(
            if on {
                VrState::Starting
            } else if self.state().engaged() {
                VrState::Stopping
            } else {
                VrState::Off
            } as u8,
            Ordering::Release,
        );
        true
    }
    pub fn input(&self) -> crate::VrInputState {
        *self.input.read()
    }
    pub fn set_pointer(&self, mut pointer: crate::VrPointer) {
        if !pointer.length.is_finite() {
            pointer.length = 0.0;
        }
        *self.pointer.write() = pointer;
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn pointer(&self) -> crate::VrPointer {
        *self.pointer.read()
    }
    pub fn add_yaw(&self, radians: f32) {
        if radians.is_finite() {
            let mut old = self.yaw_delta.load(Ordering::Relaxed);
            loop {
                let next = (f32::from_bits(old) + radians).clamp(-std::f32::consts::PI, std::f32::consts::PI);
                match self
                    .yaw_delta
                    .compare_exchange_weak(old, next.to_bits(), Ordering::Release, Ordering::Relaxed)
                {
                    Ok(_) => break,
                    Err(found) => old = found,
                }
            }
        }
    }
    pub fn add_translation(&self, delta: glam::Vec3) {
        if delta.is_finite() {
            *self.translation.write() += delta;
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn take_yaw(&self) -> f32 {
        f32::from_bits(self.yaw_delta.swap(0.0f32.to_bits(), Ordering::AcqRel))
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn take_translation(&self) -> glam::Vec3 {
        std::mem::take(&mut *self.translation.write())
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn publish_input(&self, input: crate::VrInputState) {
        *self.input.write() = input;
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn publish_head(&self, forward: glam::Vec3, position: glam::Vec3) {
        let mut input = self.input.write();
        input.head_forward = forward;
        input.head_position = position;
        input.updated_at = Some(Instant::now());
    }
    /// The interface panel to draw in VR (None: hidden).
    pub fn set_panel(&self, panel: Option<crate::VrPanel>) {
        *self.panel.write() = panel.filter(|p| p.center.is_finite() && p.right.is_finite() && p.up.is_finite());
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn panel(&self) -> Option<crate::VrPanel> {
        *self.panel.read()
    }
    pub fn set_tip(&self, tip: Option<crate::VrPanel>) {
        *self.tip.write() = tip.filter(|p| p.center.is_finite() && p.right.is_finite() && p.up.is_finite());
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn tip(&self) -> Option<crate::VrPanel> {
        *self.tip.read()
    }
    pub fn set_wrist(&self, wrist: Option<crate::VrPanel>) {
        *self.wrist.write() = wrist.filter(|p| p.center.is_finite() && p.right.is_finite() && p.up.is_finite());
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn wrist(&self) -> Option<crate::VrPanel> {
        *self.wrist.read()
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
    pub fn started(&self) {
        self.state.store(
            if self.wanted() { VrState::Active } else { VrState::Stopping } as u8,
            Ordering::Release,
        );
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
    pub fn finish(&self, failed: bool) {
        self.wanted.store(false, Ordering::Release);
        let mut input = self.input.write();
        *input = crate::VrInputState::default();
        input.updated_at = Some(Instant::now());
        *self.pointer.write() = crate::VrPointer::default();
        *self.panel.write() = None;
        *self.tip.write() = None;
        *self.wrist.write() = None;
        self.state
            .store(if failed { VrState::Failed } else { VrState::Off } as u8, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vr_control_repeated_start_stop_and_retry() {
        let c = Control::new(true);
        for _ in 0..3 {
            assert!(c.request(true));
            assert!(c.wanted());
            assert_eq!(c.state(), VrState::Starting);
            c.started();
            assert_eq!(c.state(), VrState::Active);
            assert!(c.request(false));
            assert_eq!(c.state(), VrState::Stopping);
            c.finish(false);
            assert_eq!(c.state(), VrState::Off);
        }
        c.request(true);
        c.finish(true);
        assert_eq!(c.state(), VrState::Failed);
        assert!(c.request(true));
        assert_eq!(c.state(), VrState::Starting);
    }
    #[test]
    fn vr_control_unavailable_and_cancelled_start() {
        let unavailable = Control::new(false);
        assert!(!unavailable.request(true));
        assert!(!unavailable.wanted());
        let c = Control::new(true);
        c.request(true);
        c.request(false);
        c.started();
        assert_eq!(c.state(), VrState::Stopping);
        assert!(!c.wanted());
        c.finish(false);
        assert!(!c.state().engaged());
    }
}
