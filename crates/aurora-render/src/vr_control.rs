//! Requests cross to the rendering worker without moving graphics/session ownership.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

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
/// One coherent predicted VIEW-space sample, with its coordinate/reference identity.
#[derive(Debug, Clone, Copy)]
pub struct HeadPose {
    pub rotation: glam::Quat,
    pub position: glam::Vec3,
    pub rig_position: glam::Vec3,
    pub generation: u64,
    pub origin: Option<u64>,
    pub revision: u64,
    pub sampled: std::time::Instant,
    pub mode: crate::stereo_view::RigMode,
}
impl HeadPose {
    pub fn fresh(&self) -> bool {
        self.sampled.elapsed() <= std::time::Duration::from_millis(250)
    }
    pub fn matches(&self, origin: Option<u64>, revision: u64) -> bool {
        self.fresh() && self.origin == origin && self.revision == revision
    }
}

pub(crate) struct Control {
    wanted: AtomicBool,
    state: AtomicU8,
    focused: AtomicBool,
    head_pose: Mutex<Option<HeadPose>>,
    reference: AtomicU64,
}
impl Control {
    pub fn new(available: bool) -> Self {
        Self {
            wanted: AtomicBool::new(false),
            focused: AtomicBool::new(false),
            head_pose: Mutex::new(None),
            reference: AtomicU64::new(0),
            state: AtomicU8::new(if available { VrState::Off } else { VrState::Unavailable } as u8),
        }
    }
    pub fn head_pose(&self) -> Option<HeadPose> {
        if !self.focused() {
            return None;
        }
        self.head_pose.try_lock().ok().and_then(|pose| *pose).filter(HeadPose::fresh)
    }
    pub fn reference(&self, changed: bool) -> u64 {
        if changed {
            self.reference.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            self.reference.load(Ordering::Acquire)
        }
    }
    pub fn set_head_pose(&self, mut pose: HeadPose) {
        let length = pose.rotation.length_squared();
        if !pose.rotation.is_finite()
            || !length.is_finite()
            || length < 1e-8
            || !pose.position.is_finite()
            || !pose.rig_position.is_finite()
        {
            return;
        }
        pose.rotation = pose.rotation.normalize();
        if let Ok(mut head) = self.head_pose.try_lock() {
            *head = Some(pose);
        }
    }
    pub fn focused(&self) -> bool {
        self.focused.load(Ordering::Acquire)
    }
    pub fn set_focused(&self, focused: bool) {
        self.focused.store(focused, Ordering::Release);
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
        if let Ok(mut head) = self.head_pose.try_lock() {
            *head = None;
        }
        self.focused.store(false, Ordering::Release);
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
        self.focused.store(false, Ordering::Release);
        if let Ok(mut head) = self.head_pose.try_lock() {
            *head = None;
        }
        self.state
            .store(if failed { VrState::Failed } else { VrState::Off } as u8, Ordering::Release);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn sample(rotation: glam::Quat) -> HeadPose {
        HeadPose {
            rotation,
            position: glam::Vec3::ZERO,
            rig_position: glam::Vec3::ZERO,
            generation: 1,
            mode: crate::stereo_view::RigMode::FirstPerson,
            origin: Some(42),
            revision: 0,
            sampled: std::time::Instant::now(),
        }
    }
    #[test]
    fn vr_feedback_rejects_invalid_positions_quaternions_and_stale_samples() {
        let control = Control::new(true);
        control.set_focused(true);
        control.set_head_pose(sample(glam::Quat::from_xyzw(f32::MAX, 0.0, 0.0, 1.0)));
        assert!(control.head_pose().is_none());
        let mut pose = sample(glam::Quat::IDENTITY);
        pose.position.x = f32::NAN;
        control.set_head_pose(pose);
        assert!(control.head_pose().is_none());
        let mut pose = sample(glam::Quat::IDENTITY);
        pose.sampled -= std::time::Duration::from_secs(1);
        control.set_head_pose(pose);
        assert!(control.head_pose().is_none());
    }
    #[test]
    fn vr_head_pose_is_coherent_and_clears_across_sessions_and_focus_loss() {
        let control = Control::new(true);
        control.request(true);
        let mut pose = sample(glam::Quat::from_rotation_y(1.2));
        pose.position = glam::Vec3::new(2.0, 3.0, 4.0);
        control.set_head_pose(pose);
        control.set_focused(true);
        let received = control.head_pose().expect("pose");
        assert_eq!(
            (received.rotation, received.position, received.generation, received.origin),
            (pose.rotation, pose.position, 1, Some(42))
        );
        control.set_focused(false);
        assert!(control.head_pose().is_none());
        control.finish(false);
        control.request(true);
        control.set_focused(true);
        assert!(control.head_pose().is_none());
        assert_eq!(control.reference(true), 1);
        assert_eq!(control.reference(true), 2);
    }
    #[test]
    fn vr_same_region_short_teleport_rejects_fresh_in_flight_old_revision() {
        let mut before = sample(glam::Quat::IDENTITY);
        before.position = glam::Vec3::new(0.5, 0.0, 1.6);
        assert!(before.matches(Some(42), 0));
        assert!(
            !before.matches(Some(42), 1),
            "a fresh sample at the same region is still stale after teleport"
        );
        before.revision = 1;
        assert!(before.matches(Some(42), 1));
    }
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
