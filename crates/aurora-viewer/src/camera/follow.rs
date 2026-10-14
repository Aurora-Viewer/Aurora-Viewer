//! Scripted camera parameters, source priority and motion. Port of
//! LLFollowCamParams / LLFollowCam / LLFollowCamMgr (indra/newview/llfollowcam.cpp),
//! process_set_follow_cam_properties (llviewermessage.cpp) and the wire IDs in
//! indra/llmessage/llfollowcamparams.h, originally LGPL 2.1.

use glam::{Quat, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

const MIN_DISTANCE: f32 = 0.5;
const MAX_DISTANCE: f32 = 50.0; // LLAgentCamera's MAX_CAMERA_DISTANCE_FROM_AGENT

#[derive(Debug, Clone, Copy)]
pub(crate) struct Params {
    pitch: f32,
    focus_offset: Vec3,
    position_lag: f32,
    focus_lag: f32,
    distance: f32,
    behind_angle: f32,
    behind_lag: f32,
    position_threshold: f32,
    focus_threshold: f32,
    position: Option<Vec3>,
    focus: Option<Vec3>,
    pub position_locked: bool,
    focus_locked: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            pitch: 0.0,
            focus_offset: Vec3::X,
            position_lag: 0.1,
            focus_lag: 0.1,
            distance: 3.0,
            behind_angle: 10.0,
            behind_lag: 0.0,
            position_threshold: 1.0,
            focus_threshold: 1.0,
            position: None,
            focus: None,
            position_locked: false,
            focus_locked: false,
        }
    }
}

/// Last activated wins. Updating a source without CAMERA_ACTIVE keeps its
/// priority; deactivation keeps its parameters, clearing removes them.
#[derive(Default)]
pub(crate) struct Sources {
    params: HashMap<Uuid, Params>,
    regions: HashMap<Uuid, aurora_net::RegionHandle>,
    stack: Vec<Uuid>,
}

impl Sources {
    pub fn has_active(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn active(&self) -> Option<&Params> {
        self.stack.last().and_then(|id| self.params.get(id))
    }

    pub fn set(&mut self, object: Uuid, region: aurora_net::RegionHandle, properties: &[(i32, f32)], origin: Vec3) {
        if object.is_nil() {
            return;
        }
        // Vector components are assembled from zero per message, as in
        // process_set_follow_cam_properties, independently of older vectors.
        let mut position = None;
        let mut focus = None;
        let mut offset = None;
        for &(kind, value) in properties {
            if !value.is_finite() {
                continue;
            }
            // The aggregate vector IDs (1, 13, 17) are LSL-only. Unknown
            // properties must not create a source or change its priority.
            if !matches!(kind, 0 | 2..=12 | 14..=16 | 18..=22) {
                continue;
            }
            let p = self.params.entry(object).or_default();
            self.regions.insert(object, region);
            match kind {
                0 => p.pitch = value.clamp(-45.0, 80.0),
                2..=4 => offset.get_or_insert(Vec3::ZERO)[(kind - 2) as usize] = value,
                5 => p.position_lag = value.clamp(0.0, 3.0),
                6 => p.focus_lag = value.clamp(0.0, 3.0),
                7 => p.distance = value.clamp(MIN_DISTANCE, 1000.0),
                8 => p.behind_angle = value.clamp(0.0, 180.0),
                9 => p.behind_lag = value.clamp(0.0, 3.0),
                10 => p.position_threshold = value.clamp(0.0, 4.0),
                11 => p.focus_threshold = value.clamp(0.0, 4.0),
                12 => {
                    self.stack.retain(|id| *id != object);
                    if value != 0.0 {
                        self.stack.push(object);
                    }
                }
                14..=16 => position.get_or_insert(Vec3::ZERO)[(kind - 14) as usize] = value,
                18..=20 => focus.get_or_insert(Vec3::ZERO)[(kind - 18) as usize] = value,
                21 => p.position_locked = value != 0.0,
                22 => p.focus_locked = value != 0.0,
                _ => {}
            }
        }
        if let Some(p) = self.params.get_mut(&object) {
            if let Some(v) = position {
                let v = v + origin;
                if v.length_squared().is_finite() {
                    p.position = Some(v);
                }
            }
            if let Some(v) = focus {
                let v = v + origin;
                if v.length_squared().is_finite() {
                    p.focus = Some(v);
                }
            }
            if let Some(v) = offset {
                p.focus_offset = v.clamp(Vec3::splat(-10.0), Vec3::splat(10.0));
            }
        }
    }

    pub fn clear(&mut self, object: Uuid) {
        self.deactivate(object);
        self.params.remove(&object);
        self.regions.remove(&object);
    }

    /// getOffObject / detachObject deactivate the source without discarding
    /// its parameters. Only CAMERA_ACTIVE can reactivate it afterwards.
    pub fn deactivate(&mut self, object: Uuid) {
        self.stack.retain(|id| *id != object);
    }

    pub fn clear_region(&mut self, region: aurora_net::RegionHandle, keep: impl Fn(Uuid) -> bool) {
        let removed: Vec<_> = self
            .regions
            .iter()
            .filter(|(id, h)| **h == region && !keep(**id))
            .map(|(id, _)| *id)
            .collect();
        for id in removed {
            self.clear(id);
        }
    }

    pub fn shift(&mut self, delta: Vec3) {
        for p in self.params.values_mut() {
            for v in [&mut p.position, &mut p.focus].into_iter().flatten() {
                *v += delta;
            }
        }
    }
}

/// The simulated points are kept in world space; the output is smoothed in
/// the avatar's frame, so a moving or turning vehicle remains steady.
#[derive(Clone)]
pub(super) struct Simulation {
    position: Vec3,
    focus: Vec3,
    relative_position: Vec3,
    relative_focus: Vec3,
    last_position: Vec3,
    last_focus: Vec3,
    last_distance: f32,
    pub distance: f32,
    pub position_locked: bool,
    pub eye: Vec3,
    pub at: Vec3,
}

impl Default for Simulation {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            focus: Vec3::ZERO,
            relative_position: Vec3::ZERO,
            relative_focus: Vec3::ZERO,
            last_position: Vec3::ZERO,
            last_focus: Vec3::ZERO,
            last_distance: 3.0,
            distance: 3.0,
            position_locked: false,
            eye: Vec3::ZERO,
            at: Vec3::ZERO,
        }
    }
}

impl Simulation {
    pub fn reset(&mut self, eye: Vec3, at: Vec3, subject: Vec3, rotation: Quat) {
        self.position = eye;
        self.focus = at;
        self.last_position = eye;
        self.last_focus = at;
        self.relative_position = rotation.inverse() * (eye - subject);
        self.relative_focus = rotation.inverse() * (at - subject);
        self.eye = eye;
        self.at = at;
    }

    pub fn shift(&mut self, delta: Vec3) {
        for v in [
            &mut self.position,
            &mut self.focus,
            &mut self.last_position,
            &mut self.last_focus,
            &mut self.eye,
            &mut self.at,
        ] {
            *v += delta;
        }
    }

    /// Wheel sign: positive zooms out; a locked position cannot be zoomed.
    /// Returns true on reaching the mouselook entry distance.
    pub fn zoom(&mut self, clicks: f32) -> bool {
        if self.position_locked || clicks == 0.0 || !clicks.is_finite() {
            return false;
        }
        let amount = clicks * self.distance * 0.1;
        let amount = amount.signum() * amount.abs().max(0.1);
        let distance = self.distance + amount;
        self.distance = distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        clicks < 0.0 && distance < MIN_DISTANCE
    }

    pub fn update(&mut self, p: &Params, subject: Vec3, rotation: Quat, dt: f32) {
        self.position_locked = p.position_locked;
        if p.distance != self.last_distance {
            self.last_distance = p.distance;
            self.distance = p.distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        }
        if let Some(v) = p.position
            && v != self.last_position
        {
            self.last_position = v;
            self.position = v;
        }
        if let Some(v) = p.focus
            && v != self.last_focus
        {
            self.last_focus = v;
            self.focus = v;
        }

        let desired_focus = subject + rotation * p.focus_offset;
        let to_subject = desired_focus - self.position;
        let inverse = rotation.inverse();
        let relative_lerp = super::interpolant(0.05, dt);
        if p.focus_locked {
            self.relative_focus = inverse * (self.focus - subject);
        } else {
            self.focus = pull(self.focus, desired_focus, p.focus_threshold, p.focus_lag, dt);
            self.relative_focus = self.relative_focus.lerp_monotonic(inverse * (self.focus - subject), relative_lerp);
        }

        if p.position_locked {
            self.relative_position = inverse * (self.position - subject);
        } else {
            let flat = Vec3::new(to_subject.x, to_subject.y, 0.0);
            let direction = flat.normalize_or(Vec3::Z);
            let (sin, cos) = p.pitch.to_radians().sin_cos();
            let offset = Vec3::new(direction.x * cos, direction.y * cos, -sin) * self.distance;
            self.position = pull(self.position, desired_focus - offset, p.position_threshold, p.position_lag, dt);
            let distance = to_subject.length();
            if distance > MAX_DISTANCE {
                self.position = desired_focus - to_subject * (MAX_DISTANCE / distance);
            }
            if p.behind_angle < 179.0 {
                let offset = self.position - self.focus;
                let flat = Vec3::new(offset.x, offset.y, 0.0);
                let back = rotation * -Vec3::X;
                let back = Vec3::new(back.x, back.y, 0.0).normalize_or(-Vec3::X);
                let direction = flat.normalize_or(back);
                let angle = back.cross(direction).z.atan2(back.dot(direction));
                let excess = angle.abs() - p.behind_angle.to_radians();
                if excess > 0.0 {
                    let turn = -angle.signum() * excess * super::interpolant(p.behind_lag, dt);
                    let turned = Quat::from_rotation_z(turn) * flat;
                    self.position.x = self.focus.x + turned.x;
                    self.position.y = self.focus.y + turned.y;
                }
            }
            self.relative_position = self
                .relative_position
                .lerp_monotonic(inverse * (self.position - subject), relative_lerp);
        }
        // Locked world points stay bit-identical while the subject turns;
        // a quaternion round trip would introduce visible sub-pixel jitter.
        self.eye = if p.position_locked {
            self.position
        } else {
            subject + rotation * self.relative_position
        };
        self.at = if p.focus_locked {
            self.focus
        } else {
            subject + rotation * self.relative_focus
        };
    }
}

fn pull(current: Vec3, ideal: Vec3, threshold: f32, lag: f32, dt: f32) -> Vec3 {
    let offset = ideal - current;
    let distance = offset.length();
    if distance <= threshold || distance < 0.0001 {
        return current;
    }
    current.lerp_monotonic(current + offset * (1.0 - threshold / distance), super::interpolant(lag, dt))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(s: &mut Sources, id: u128, properties: &[(i32, f32)]) {
        s.set(Uuid::from_u128(id), 1, properties, Vec3::ZERO);
    }

    #[test]
    fn activation_priority_deactivation_and_clear_are_per_object() {
        let mut s = Sources::default();
        set(&mut s, 1, &[(12, 1.0), (7, 6.0)]);
        set(&mut s, 2, &[(12, 1.0), (7, 9.0)]);
        set(&mut s, 1, &[(0, 20.0)]);
        assert_eq!(s.active().expect("active").distance, 9.0);
        set(&mut s, 1, &[(12, 1.0)]);
        assert_eq!(s.active().expect("active").pitch, 20.0);
        assert_eq!(s.stack.len(), 2);
        set(&mut s, 1, &[(12, 0.0)]);
        assert_eq!(s.active().expect("active").distance, 9.0);
        s.clear(Uuid::from_u128(2));
        assert!(s.active().is_none());
        set(&mut s, 1, &[(12, 1.0)]);
        assert_eq!(s.active().expect("retained").distance, 6.0);
        s.clear(Uuid::from_u128(1));
        set(&mut s, 1, &[(12, 1.0)]);
        assert_eq!(s.active().expect("defaults").distance, 3.0);
    }

    #[test]
    fn local_deactivation_retains_parameters_without_reactivating_on_partial_updates() {
        let mut s = Sources::default();
        set(&mut s, 1, &[(12, 1.0), (7, 6.0), (21, 1.0)]);
        set(&mut s, 2, &[(12, 1.0), (7, 9.0)]);
        s.deactivate(Uuid::from_u128(2));
        assert_eq!(s.active().expect("previous object").distance, 6.0);
        set(&mut s, 2, &[(0, 30.0)]);
        assert_eq!(s.active().expect("partial update keeps priority").distance, 6.0);
        set(&mut s, 2, &[(12, 1.0)]);
        let p = s.active().expect("explicit reactivation");
        assert_eq!((p.distance, p.pitch), (9.0, 30.0));
        for _ in 0..1000 {
            set(&mut s, 2, &[(12, 1.0)]);
        }
        assert_eq!(s.stack, [Uuid::from_u128(1), Uuid::from_u128(2)]);
        // Clearing a background source must neither release nor move the top.
        s.clear(Uuid::from_u128(1));
        assert_eq!(s.active().expect("foreground remains").distance, 9.0);
        s.clear(Uuid::from_u128(2));
        assert!(!s.has_active());
    }

    #[test]
    fn packet_vectors_are_assembled_before_locks_and_shift_as_points() {
        let mut s = Sources::default();
        s.set(
            Uuid::from_u128(1),
            1,
            &[(12, 1.0), (14, 10.0), (16, 5.0), (22, 1.0), (19, 20.0), (2, 2.0)],
            Vec3::X * 256.0,
        );
        let p = s.active().expect("active");
        assert_eq!(p.position, Some(Vec3::new(266.0, 0.0, 5.0)));
        assert_eq!(p.focus, Some(Vec3::new(256.0, 20.0, 0.0)));
        assert_eq!(p.focus_offset, Vec3::X * 2.0);
        assert!(p.focus_locked);
        s.shift(Vec3::new(-256.0, 128.0, 0.0));
        let p = s.active().expect("shifted");
        assert_eq!(p.position, Some(Vec3::new(10.0, 128.0, 5.0)));
        assert_eq!(p.focus_offset, Vec3::X * 2.0);
        set(&mut s, 1, &[(15, 7.0)]);
        assert_eq!(s.active().expect("partial vector").position, Some(Vec3::Y * 7.0));
    }

    #[test]
    fn malformed_values_are_ignored_and_scalar_ranges_match_firestorm() {
        let mut s = Sources::default();
        set(&mut s, 0, &[(12, 1.0)]);
        set(&mut s, 1, &[(12, f32::NAN), (7, f32::INFINITY), (999, 1.0), (13, 1.0)]);
        assert!(s.params.is_empty());
        set(
            &mut s,
            1,
            &[
                (12, 1.0),
                (0, 1000.0),
                (5, -1.0),
                (6, 9.0),
                (7, -1.0),
                (8, 500.0),
                (9, -1.0),
                (10, 9.0),
                (11, -1.0),
                (2, 30.0),
                (14, f32::MAX),
            ],
        );
        let p = s.active().expect("active");
        assert_eq!((p.pitch, p.position_lag, p.focus_lag, p.distance), (80.0, 0.0, 3.0, 0.5));
        assert_eq!(
            (p.behind_angle, p.behind_lag, p.position_threshold, p.focus_threshold),
            (180.0, 0.0, 4.0, 0.0)
        );
        assert_eq!(p.focus_offset, Vec3::X * 10.0);
        assert!(p.position.is_none());
    }

    fn settled(p: &Params, subject: Vec3, rotation: Quat) -> Simulation {
        let mut sim = Simulation::default();
        sim.reset(subject - rotation * Vec3::X * 3.0, subject, subject, rotation);
        for _ in 0..600 {
            sim.update(p, subject, rotation, 1.0 / 60.0);
        }
        sim
    }

    #[test]
    fn distance_pitch_offset_and_behindness_follow_the_subject() {
        let p = Params {
            distance: 6.0,
            pitch: 30.0,
            focus_offset: Vec3::Z,
            position_threshold: 0.0,
            focus_threshold: 0.0,
            ..Default::default()
        };
        let subject = Vec3::new(128.0, 128.0, 25.0);
        let rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let sim = settled(&p, subject, rotation);
        assert!(sim.at.distance(subject + Vec3::Z) < 0.001);
        assert!(sim.eye.distance(subject + Vec3::new(0.0, -6.0 * 30f32.to_radians().cos(), 4.0)) < 0.01);
        // Initially ahead of the subject: the behindness cone pulls back,
        // preserving pitch and horizontal radius.
        let mut sim = Simulation::default();
        sim.reset(subject + Vec3::X * 5.0, subject, subject, Quat::IDENTITY);
        for _ in 0..600 {
            sim.update(&p, subject, Quat::IDENTITY, 1.0 / 60.0);
        }
        let offset = sim.eye - sim.at;
        assert!(offset.x < 0.0);
        assert!(((-offset.x).atan2(offset.y.abs()).to_degrees() - 80.0).abs() < 0.1);
    }

    #[test]
    fn locks_keep_world_points_and_unlocks_resume_following() {
        let mut p = Params {
            position: Some(Vec3::new(125.0, 124.0, 30.0)),
            focus: Some(Vec3::new(129.0, 128.0, 26.0)),
            position_locked: true,
            focus_locked: true,
            ..Default::default()
        };
        let mut sim = settled(&p, Vec3::new(128.0, 128.0, 25.0), Quat::IDENTITY);
        for i in 0..120 {
            sim.update(
                &p,
                Vec3::new(128.0 + i as f32 * 0.1, 130.0, 26.0),
                Quat::from_rotation_z(i as f32 * 0.02),
                1.0 / 60.0,
            );
            assert_eq!(sim.eye, p.position.expect("position"));
            assert_eq!(sim.at, p.focus.expect("focus"));
        }
        assert!(!sim.zoom(-100.0));
        p.position_locked = false;
        p.focus_locked = false;
        let subject = Vec3::new(140.0, 130.0, 26.0);
        for _ in 0..600 {
            sim.update(&p, subject, Quat::IDENTITY, 1.0 / 60.0);
        }
        assert!(sim.at.distance(subject + Vec3::X) < 1.001);
        assert!(sim.eye.distance(p.position.expect("old position")) > 8.0);
    }

    #[test]
    fn thresholds_prevent_pull_and_zoom_survives_unchanged_parameters() {
        let p = Params {
            behind_angle: 180.0,
            focus_offset: Vec3::ZERO,
            ..Default::default()
        };
        let mut sim = settled(&p, Vec3::ZERO, Quat::IDENTITY);
        sim.update(&p, Vec3::X * 0.5, Quat::IDENTITY, 1.0);
        assert_eq!(sim.focus, Vec3::ZERO);
        assert_eq!(sim.position, -Vec3::X * 3.0);
        assert!(!sim.zoom(1.0));
        assert_eq!(sim.distance, 3.3);
        sim.update(&p, Vec3::ZERO, Quat::IDENTITY, 1.0 / 60.0);
        assert_eq!(sim.distance, 3.3);
        assert!(sim.zoom(-100.0));
        assert_eq!(sim.distance, 0.5);
        sim.update(&Params { distance: 500.0, ..p }, Vec3::ZERO, Quat::IDENTITY, 0.1);
        assert_eq!(sim.distance, MAX_DISTANCE);
    }

    #[test]
    fn zero_offsets_and_origin_shifts_keep_the_simulation_finite() {
        let mut sources = Sources::default();
        set(
            &mut sources,
            1,
            &[
                (12, 1.0),
                (14, 128.0),
                (15, 128.0),
                (16, 25.0),
                (18, 128.0),
                (19, 128.0),
                (20, 25.0),
            ],
        );
        let subject = Vec3::new(128.0, 128.0, 25.0);
        let mut sim = settled(sources.active().expect("active"), subject, Quat::IDENTITY);
        let before = (sim.eye, sim.at);
        let shift = Vec3::new(-256.0, 256.0, 0.0);
        sources.shift(shift);
        sim.shift(shift);
        sim.update(sources.active().expect("active"), subject + shift, Quat::IDENTITY, 0.0);
        assert!(sim.eye.is_finite() && sim.at.is_finite());
        assert!(sim.eye.distance(before.0 + shift) < 0.001);
        assert!(sim.at.distance(before.1 + shift) < 0.001);
    }

    #[test]
    fn region_removal_preserves_a_source_carried_with_the_avatar() {
        let mut sources = Sources::default();
        set(&mut sources, 1, &[(12, 1.0), (7, 6.0)]);
        set(&mut sources, 2, &[(12, 1.0), (7, 9.0)]);
        sources.clear_region(1, |id| id == Uuid::from_u128(1));
        assert_eq!(sources.active().expect("carried source").distance, 6.0);
        assert_eq!(sources.params.len(), 1);
        sources.clear_region(1, |_| false);
        assert!(sources.active().is_none());
    }

    #[test]
    fn lag_and_relative_smoothing_use_elapsed_time() {
        let p = Params {
            focus_offset: Vec3::ZERO,
            focus_threshold: 0.0,
            focus_lag: 0.2,
            ..Default::default()
        };
        let subject = Vec3::new(10.0, 0.0, 0.0);
        let mut sim = Simulation::default();
        sim.reset(-Vec3::X * 3.0, Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY);
        sim.update(&p, subject, Quat::IDENTITY, 0.2);
        // A 0.2-second half-life moves the world focus halfway; the
        // 0.05-second relative smoothing also includes subject movement.
        assert_eq!(sim.focus, Vec3::X * 5.0);
        assert_eq!(sim.at, Vec3::X * 5.3125);
        let slow = Params { focus_lag: 3.0, ..p };
        let mut slow_sim = Simulation::default();
        slow_sim.reset(-Vec3::X * 3.0, Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY);
        slow_sim.update(&slow, subject, Quat::IDENTITY, 0.2);
        assert!(slow_sim.focus.x < sim.focus.x);
    }
}
