//! Requests cross to the rendering worker without moving graphics/session ownership.
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

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
}
impl Control {
    pub fn new(available: bool) -> Self {
        Self {
            wanted: AtomicBool::new(false),
            state: AtomicU8::new(if available { VrState::Off } else { VrState::Unavailable } as u8),
        }
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
