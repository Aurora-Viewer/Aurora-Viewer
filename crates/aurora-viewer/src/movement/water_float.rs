//! Local render-only heave. Simulator positions and bridge targets stay flat.
use glam::Vec3;

#[derive(Default)]
pub struct Follower {
    previous: Option<(u64, Vec3, f32)>,
    z: f32,
    surface: bool,
}

impl Follower {
    pub fn offset(&self) -> f32 {
        self.z
    }

    pub fn at_surface(&mut self, frame: &super::Frame, state: super::State, pose: Option<crate::ao::State>, transitioning: bool) -> bool {
        use crate::ao::State as Pose;
        let surface_pose = matches!(pose, Some(Pose::Floating | Pose::SwimmingForward))
            || (state == super::State::WaterWalking
                && matches!(
                    pose,
                    Some(Pose::Standing | Pose::Walking | Pose::Running | Pose::Crouching | Pose::CrouchWalk)
                ));
        // Free-camera/focus suspension releases the motor, and its AO signal
        // becomes stock hover/flight again. Preserve an established surface
        // hold while inspecting it, but never infer a new hold from a speed.
        let inspecting = frame.suspended
            && self.surface
            && !frame.flying
            && matches!(
                pose,
                None | Some(Pose::UnderwaterFloating | Pose::Hovering | Pose::Flying | Pose::FlyingSlow)
            );
        // A swim pose is the intent. The simulator root need not settle within
        // 30 cm of the bridge target (shape, float height, latency). Only reject
        // roots whose body cannot reach the surface; terrain/bridge readiness
        // and measured velocity do not gate a visual offset.
        self.surface = (surface_pose || inspecting)
            && (state != super::State::Submerged || frame.suspended)
            && !transitioning
            && !frame.seated
            && !frame.jumping
            && frame.position.is_finite()
            && frame.height.is_finite()
            && frame
                .water
                .is_some_and(|level| level.is_finite() && (frame.position.z - level).abs() <= frame.height * 0.5 + 0.5);
        self.surface
    }

    pub fn update(&mut self, region: u64, position: Vec3, time: f32, target: f32, reset: bool) {
        if reset || !position.is_finite() || !time.is_finite() || !target.is_finite() {
            *self = Self::default();
            return;
        }
        let previous = self.previous.replace((region, position, time));
        let Some((old_region, old_position, old_time)) = previous else {
            return;
        };
        let dt = time - old_time;
        if old_region != region || old_position.distance(position) > 3.0 || !(0.0..=0.25).contains(&dt) {
            self.z = 0.0;
            self.surface = false;
            return;
        }
        // The macro spectrum is slow: a short exponential response removes
        // state/quality transitions without noticeable phase lag or overshoot.
        let change = (target.clamp(-0.6, 0.6) - self.z) * (1.0 - (-dt / 0.10).exp());
        self.z += change.clamp(-dt, dt);
        if self.z.abs() < 0.0001 {
            self.z = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surface_float_follows_crests_and_troughs_smoothly_at_different_frame_rates() {
        for fps in [30, 60, 144] {
            let mut follower = Follower::default();
            for frame in 0..fps * 8 {
                let time = frame as f32 / fps as f32;
                let target = 0.3 * (time * 1.4).sin();
                let old = follower.offset();
                follower.update(1, Vec3::X * time, time, target, false);
                assert!((follower.offset() - old).abs() <= 1.001 / fps as f32);
                if time > 1.0 {
                    assert!((follower.offset() - target).abs() < 0.05);
                }
            }
            for frame in 0..fps {
                follower.update(1, Vec3::X * 8.0, 8.0 + frame as f32 / fps as f32, 0.0, false);
            }
            assert_eq!(follower.offset(), 0.0, "disabling waves or leaving the surface settles to zero");
        }
    }
    #[test]
    fn surface_float_discards_old_region_teleports_seats_gaps_and_bad_values() {
        let warm = || {
            let mut f = Follower::default();
            for i in 0..60 {
                f.update(1, Vec3::ZERO, i as f32 / 60.0, 0.3, false);
            }
            assert!(f.offset() > 0.29);
            f
        };
        for (region, position, time, target, reset) in [
            (2, Vec3::ZERO, 1.0, 0.3, false),
            (1, Vec3::X * 10.0, 1.0, 0.3, false),
            (1, Vec3::ZERO, 2.0, 0.3, false),
            (1, Vec3::ZERO, 1.0, 0.3, true),
            (1, Vec3::ZERO, 1.0, f32::NAN, false),
        ] {
            let mut f = warm();
            f.update(region, position, time, target, reset);
            assert_eq!(f.offset(), 0.0);
        }
    }
}
