//! Native stand/crouch foot placement and stand-up recovery, following
//! LLKeyframeStandMotion / LLKeyframeFallMotion (indra/llcharacter,
//! originally LGPL 2.1). Corrections belong to each motion before blending.
//! The optional terrain solver replaces this pass, including its pelvis tilt.

use super::*;
use crate::ao::State;
use grounding::{Frame, Placed, Surface};

pub(super) fn supported(id: Uuid) -> bool {
    matches!(State::from_motion(id), Some(State::Standing | State::Crouching | State::StandingUp))
}

#[derive(Clone, Copy)]
pub(super) struct Recovery {
    tilt: Quat,
    ease_in: f32,
}

impl Recovery {
    fn new(frame: Frame, surface: Surface, height: f32) -> Self {
        let rotation = frame.to_world.to_scale_rotation_translation().1;
        let up = rotation.inverse() * surface.normal.normalize_or(Vec3::Z);
        Self {
            tilt: facing_ground(Vec3::X, up).unwrap_or(Quat::IDENTITY),
            // LLKeyframeFallMotion::getEaseInDuration, with invalid or upward
            // velocity treated as an already grounded activation.
            ease_in: if frame.velocity_z < -1e-5 {
                height / -frame.velocity_z
            } else {
                0.4
            },
        }
    }
}

pub(super) fn ease_in(recovery: Option<Recovery>, fallback: f32) -> f32 {
    recovery.map_or(fallback, |recovery| recovery.ease_in)
}

fn facing_ground(forward: Vec3, up: Vec3) -> Option<Quat> {
    let up = up.try_normalize()?;
    let left = up.cross(forward).try_normalize()?;
    let forward = left.cross(up);
    let q = Quat::from_mat3(&Mat3::from_cols(forward, left, up)).normalize();
    q.is_finite().then_some(q)
}

/// At most the pelvis and six leg rotations. No changes to unrelated tracks,
/// source animation data, or the controller's held uncorrected channels.
pub(super) fn sample(
    motion: &mut Motion,
    rig: &Rig,
    base: Option<&SkeletonBase>,
    frame: Frame,
    ground: &dyn Fn(Vec3) -> Option<Surface>,
    now: Instant,
) -> Vec<(usize, Quat)> {
    let standup = motion.id == State::StandingUp.motion();
    if !supported(motion.id) {
        return Vec::new();
    }
    let rest = base.map_or(rig.local_pos.as_slice(), |base| &base.local_pos);
    let scale = base.map_or(rig.scale.as_slice(), |base| &base.scale);
    let time = motion.local_time(now) as f32;
    let animation = &motion.anim.anim;
    let track = |joint| {
        animation
            .joints
            .iter()
            .zip(&motion.anim.joints)
            .find_map(|(track, bound)| (*bound == Some(joint)).then_some(track))
    };
    let Some(pelvis_track) = track(rig.pelvis) else {
        return Vec::new();
    };
    let pelvis_rotation = pelvis_track
        .rotation_at_loop(time, animation.loop_range())
        .unwrap_or(rig.local_rot[rig.pelvis]);
    let pelvis_offset = pelvis_track
        .position_at_loop(time, animation.loop_range())
        .filter(|p| p.is_finite())
        .map_or(Vec3::ZERO, |position| position - (rest[rig.pelvis] - rig.local_pos[rig.pelvis]));
    // Only the short root-to-pelvis path and two three-joint chains are needed;
    // the full skeleton and geometry are never traversed by this solver.
    let mut path = Vec::new();
    let mut parent = rig.parent[rig.pelvis];
    while let Some(joint) = parent {
        path.push(joint);
        parent = rig.parent[joint];
    }
    let (mut position, mut rotation) = (Vec3::ZERO, Quat::IDENTITY);
    let parent_scale = |joint: usize| rig.parent[joint].map_or(rig.parent_scale[joint], |parent| scale[parent]);
    for joint in path.into_iter().rev() {
        position += rotation * (rest[joint] * parent_scale(joint));
        rotation = (rotation * rig.local_rot[joint]).normalize();
    }
    position += rotation * (rest[rig.pelvis] * parent_scale(rig.pelvis) + pelvis_offset);
    rotation = (rotation * pelvis_rotation).normalize();
    let world_rotation = frame.to_world.to_scale_rotation_translation().1;
    position = frame.to_world.transform_point3(position);
    rotation = (world_rotation * rotation).normalize();
    if !position.is_finite() || !rotation.is_finite() {
        return Vec::new();
    }
    if standup {
        if motion.recovery.is_none() {
            let Some(surface) = ground(position) else {
                return Vec::new();
            };
            motion.recovery = Some(Recovery::new(frame, surface, rig.local_pos[rig.pelvis].z.max(0.01)));
        }
        let recovery = motion.recovery.unwrap_or(Recovery {
            tilt: Quat::IDENTITY,
            ease_in: 0.4,
        });
        let fraction = if animation.duration > 0.0 { time / animation.duration } else { 1.0 };
        let weight = ((fraction - 0.5) / 0.25).clamp(0.0, 1.0);
        let tilt = recovery.tilt.slerp(Quat::IDENTITY, weight);
        return vec![(rig.pelvis, (pelvis_rotation * tilt).normalize())];
    }
    let mut result = Vec::with_capacity(6);
    for side in ["Left", "Right"] {
        let joints = ["Hip", "Knee", "Ankle"].map(|bone| rig.names.get(&format!("m{bone}{side}")).copied());
        let [Some(hip), Some(knee), Some(ankle)] = joints else { continue };
        if rig.parent[hip] != Some(rig.pelvis) || rig.parent[knee] != Some(hip) || rig.parent[ankle] != Some(knee) {
            continue;
        }
        let mut points = [Vec3::ZERO; 3];
        let mut rotations = [Quat::IDENTITY; 3];
        let (mut p, mut q) = (position, rotation);
        let mut complete = true;
        for (i, joint) in [hip, knee, ankle].into_iter().enumerate() {
            let Some(track) = track(joint) else {
                complete = false;
                break;
            };
            p += q * (rest[joint] * parent_scale(joint));
            q = (q * track.rotation_at_loop(time, animation.loop_range()).unwrap_or(rig.local_rot[joint])).normalize();
            points[i] = p;
            rotations[i] = q;
        }
        if !complete {
            continue;
        }
        let Some(surface) = ground(points[2]) else { continue };
        let target_rotation = facing_ground(rotations[2] * Vec3::X, surface.normal);
        let target = super::super::hand_ik::Target {
            position: surface.position,
            rotation: target_rotation,
            contact: Vec3::ZERO,
        };
        // The native motion's forward pole remains in the pelvis frame, as
        // LLKeyframeStandMotion's two RP3 solvers. Use the shared analytic math.
        if let Some(solved) = super::super::hand_ik::solve(points, rotations, rotation, target, rotation * Vec3::X) {
            result.extend([hip, knee, ankle].into_iter().zip(solved));
        }
    }
    result
}

/// Interpolate mutually exclusive corrections relative to the same current
/// animation pose. Repeated toggles start from the visible correction; neither
/// solver receives the other solver's result as an input.
#[derive(Default)]
pub(super) struct Handoff {
    mode: Option<bool>,
    previous: Option<Correction>,
    from: Option<(Instant, Correction)>,
}

#[derive(Clone)]
struct Correction {
    rotations: Vec<(usize, Quat)>,
    pelvis: Vec3,
}

impl Handoff {
    pub(super) fn resume(&mut self, duration: Duration) {
        if let Some((at, _)) = &mut self.from {
            *at += duration;
        }
    }

    #[allow(clippy::too_many_arguments)] // Two candidate poses and the shared mode/clock/priority policy.
    pub(super) fn apply(
        &mut self,
        rig: &Rig,
        source: &[Quat],
        selected: &[Quat],
        pelvis: Vec3,
        aurora: bool,
        allowed: [bool; 2],
        now: Instant,
    ) -> Option<Placed> {
        if self.mode.is_some_and(|mode| mode != aurora) {
            self.from = self.previous.clone().map(|previous| (now, previous));
        }
        self.mode = Some(aurora);
        let joints = [
            "mPelvis",
            "mHipLeft",
            "mKneeLeft",
            "mAnkleLeft",
            "mHipRight",
            "mKneeRight",
            "mAnkleRight",
        ];
        let mut correction = Correction {
            rotations: joints
                .iter()
                .filter_map(|name| {
                    let joint = *rig.names.get(*name)?;
                    Some((joint, (selected.get(joint)? * source.get(joint)?.inverse()).normalize()))
                })
                .collect(),
            pelvis,
        };
        if let Some((at, from)) = &self.from {
            let weight = cubic_step(now.saturating_duration_since(*at).as_secs_f32() / 0.5);
            for ((joint, q), (_, previous)) in correction.rotations.iter_mut().zip(&from.rotations) {
                let can_blend = if *joint == rig.pelvis {
                    allowed.into_iter().all(|allow| allow)
                } else {
                    allowed[usize::from(joints[4..].iter().any(|name| rig.names.get(*name) == Some(joint)))]
                };
                if can_blend {
                    *q = previous.slerp(*q, weight).normalize();
                }
            }
            if allowed.into_iter().all(|allow| allow) {
                correction.pelvis = from.pelvis.lerp(pelvis, weight);
            }
            if weight >= 1.0 {
                self.from = None;
            }
        }
        self.previous = Some(correction.clone());
        self.from.as_ref()?;
        let mut result = Placed {
            rotations: selected.to_vec(),
            pelvis: correction.pelvis,
        };
        for (joint, q) in correction.rotations {
            result.rotations[joint] = (q * source[joint]).normalize();
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests;
