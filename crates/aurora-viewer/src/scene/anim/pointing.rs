//! LLEditingMotion and LLTargetingMotion (indra/llcharacter, originally
//! LGPL 2.1). Joint states enter the regular priority blender, including
//! the additive targeting torso; control avatars never run these motions.

use super::{Fade, HIGH_PRIORITY, ProceduralState, Rig, SkeletonBase, constrain, facing, interpolant, nlerp};
use glam::{Mat4, Quat, Vec3};
use std::time::{Duration, Instant};

pub(super) struct Frame<'a> {
    pub rotations: &'a [Quat],
    pub position_delta: &'a [Vec3],
    pub pelvis: Vec3,
    pub base: Option<&'a SkeletonBase>,
    pub point: Option<Vec3>,
    pub look: Option<Vec3>,
    pub aiming: bool,
}

#[derive(Default)]
pub(super) struct Pointing {
    edit: Fade,
    target: Fade,
    arm: Option<[(usize, Quat); 4]>,
    original: Option<[(usize, Quat); 4]>,
    started: Option<Instant>,
    torso: Quat,
}

impl Pointing {
    pub(super) fn animation_is_older(&self, activation: Instant) -> bool {
        self.started.is_none_or(|started| activation <= started)
    }

    pub(super) fn arm_joint(&self, joint: usize) -> bool {
        self.original.is_some_and(|joints| joints.iter().any(|(j, _)| *j == joint))
    }
    pub(super) fn editing(&self) -> bool {
        self.edit.weight > 0.0
    }

    pub(super) fn resume(&mut self, duration: Duration) {
        self.started = self.started.map(|at| at + duration);
        for fade in [&mut self.edit, &mut self.target] {
            fade.change = fade.change.map(|at| at + duration);
        }
    }

    pub(super) fn states(&mut self, rig: &Rig, frame: Frame<'_>, now: Instant, dt: f32) -> Vec<ProceduralState> {
        let editing = frame.point.is_some_and(|p| p.is_finite());
        if editing && !self.edit.on {
            self.started = Some(now);
        }
        let weight = self.edit.update(editing, now, if editing { 0.0 } else { 0.5 });
        let target_weight = self.target.update(frame.aiming, now, if frame.aiming { 0.3 } else { 0.5 });
        let mut out = Vec::new();
        if weight == 0.0 && target_weight == 0.0 && self.original.is_none() {
            return out;
        }
        let pos = frame.base.map_or(&rig.local_pos, |base| &base.local_pos);
        let scale = frame.base.map_or(&rig.scale, |base| &base.scale);
        let mut positions = pos.clone();
        for (p, delta) in positions.iter_mut().zip(frame.position_delta) {
            *p += *delta;
        }
        let pelvis_rest = pos[rig.pelvis] - rig.local_pos[rig.pelvis];
        let world = rig.world_with(frame.rotations, frame.pelvis - pelvis_rest, &positions, scale);
        let joint = |name: &str| rig.names.get(name).copied();
        if let (Some(shoulder), Some(elbow), Some(wrist), Some(torso)) =
            (joint("mShoulderLeft"), joint("mElbowLeft"), joint("mWristLeft"), joint("mTorso"))
        {
            if editing && self.original.is_none() {
                self.original = Some([shoulder, elbow, wrist, torso].map(|j| (j, frame.rotations[j])));
            }
            if let (Some(point), Some(parent)) = (frame.point.filter(|p| p.is_finite()), rig.parent[shoulder]) {
                let parent_pos = world[parent].w_axis.truncate();
                let parent_rot = rotation(world[parent]);
                let mut target = point - parent_pos;
                let distance = target.length();
                target = target.normalize_or(Vec3::X);
                let normal = rotation(world[torso]) * Vec3::new(1.0, 1.0, 0.0).normalize();
                let dot = normal.dot(target);
                if dot < 0.0 {
                    target += normal * (2.0 * dot);
                    target.z += (-dot * 5.0).clamp(0.0, 5.0);
                    target = target.normalize_or(Vec3::X);
                }
                let goal = parent_pos + target * distance;
                // LLJointSolverRP3 resets the chain to its base rotations
                // before each solve. Feeding last frame's bent arm back
                // into it would accumulate twist for nearby targets.
                let shoulder_rot = (parent_rot * rig.local_rot[shoulder]).normalize();
                let elbow_rot = (shoulder_rot * rig.local_rot[elbow]).normalize();
                let shoulder_pos = parent_pos + parent_rot * (positions[shoulder] * scale[parent]);
                let elbow_pos = shoulder_pos + shoulder_rot * (positions[elbow] * scale[shoulder]);
                // Virtual wrist offset and elbow axis from LLEditingMotion.
                let wrist_pos = elbow_pos + elbow_rot * (positions[wrist] * scale[elbow] + Vec3::new(0.0, 0.2, 0.0));
                if let Some((s, e)) = solve(
                    shoulder_pos,
                    elbow_pos,
                    wrist_pos,
                    goal,
                    parent_rot * Vec3::new(-1.0, 1.0, 0.0),
                    elbow_rot * Vec3::new(-0.682683, 0.0, -0.730714),
                ) {
                    let shoulder_goal = (parent_rot.inverse() * s * shoulder_rot).normalize();
                    let elbow_goal = (shoulder_rot.inverse() * e * elbow_rot).normalize();
                    let old = self.arm.unwrap_or([shoulder, elbow, wrist, torso].map(|j| (j, frame.rotations[j])));
                    let lag = interpolant(dt, 0.1);
                    self.arm = Some([
                        (shoulder, nlerp(lag, old[0].1, shoulder_goal)),
                        (elbow, nlerp(lag, old[1].1, elbow_goal)),
                        (wrist, Quat::IDENTITY),
                        (torso, Quat::IDENTITY),
                    ]);
                }
            }
            if let Some(original) = self.original {
                for (j, r) in original {
                    out.push((j, -1, 1.0, r, false));
                }
                if weight > 0.0 {
                    if let Some(arm) = self.arm {
                        for (j, r) in arm {
                            out.push((j, HIGH_PRIORITY, weight, r, false));
                        }
                    }
                } else {
                    self.original = None;
                    self.arm = None;
                }
            }
        }
        if target_weight > 0.0
            && let (Some(torso), Some(hand), Some(look)) = (joint("mTorso"), joint("mWristRight"), frame.look)
            && let (Some(aim), Some(hand_aim)) = (look.try_normalize().and_then(facing), facing(rotation(world[hand]) * -Vec3::Y))
        {
            let current = rotation(world[torso]);
            let goal = (current.inverse() * aim * hand_aim.inverse() * current).normalize();
            self.torso = nlerp(interpolant(dt, 0.25), self.torso, goal);
            let total = constrain(self.torso * frame.rotations[torso], std::f32::consts::FRAC_PI_2 * 0.8);
            self.torso = (total * frame.rotations[torso].inverse()).normalize();
            out.push((torso, HIGH_PRIORITY, target_weight, self.torso, true));
        }
        out
    }
}

fn rotation(matrix: Mat4) -> Quat {
    matrix.to_scale_rotation_translation().1.normalize()
}

/// LLJointSolverRP3::solve in vector form: bend the elbow, aim the chain,
/// then align its elbow axis with the pole/goal plane. Returned deltas are
/// in skeleton space; no bone lengths or translations are changed.
fn solve(a: Vec3, b: Vec3, c: Vec3, goal: Vec3, pole: Vec3, elbow_axis: Vec3) -> Option<(Quat, Quat)> {
    let ab = b - a;
    let bc = c - b;
    let ag = goal - a;
    let (al, bl) = (ab.length(), bc.length());
    if al < 1e-5 || bl < 1e-5 || !goal.is_finite() {
        return None;
    }
    let axis = ag.try_normalize()?;
    let bend_axis = ab.cross(bc).try_normalize().or_else(|| pole.cross(ab).try_normalize())?;
    let angle = (ab.dot(bc) / (al * bl)).clamp(-1.0, 1.0).acos();
    let theta = ((ag.length_squared() - al * al - bl * bl) / (2.0 * al * bl))
        .clamp(-1.0, 1.0)
        .acos();
    let bend = Quat::from_axis_angle(bend_axis, theta - angle);
    let aim = Quat::from_rotation_arc((ab + bend * bc).try_normalize()?, axis);
    let plane = pole.cross(ag).try_normalize()?;
    let normal = (aim * elbow_axis).try_normalize()?;
    let align = Quat::from_rotation_arc(normal, plane);
    let shoulder = (align * aim).normalize();
    (shoulder.is_finite() && bend.is_finite()).then_some((shoulder, bend))
}

#[cfg(test)]
mod tests {
    use super::super::{BoundAnim, Controller, PALETTE_JOINTS};
    use super::*;
    use crate::world::PlayingAnimation;
    use aurora_assets::{Animation, JointMotion, Skeleton};
    use std::sync::Arc;
    use uuid::Uuid;

    fn rig() -> Rig {
        let skeleton = Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")).expect("skeleton");
        Rig::new(&skeleton)
    }

    fn evaluate(c: &mut Controller, rig: &Rig, time: Instant) -> Vec<Mat4> {
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); PALETTE_JOINTS];
        c.evaluate(rig, time, None, &mut out);
        out.iter().take(rig.len()).map(Mat4::from_cols_array_2d).collect()
    }

    #[test]
    fn editing_tracks_targets_with_finite_rotations_and_releases_the_arm() {
        let rig = rig();
        let shoulder = rig.names["mShoulderLeft"];
        let wrist = rig.names["mWristLeft"];
        let now = Instant::now();
        let mut c = Controller::default();
        evaluate(&mut c, &rig, now);
        let original = c.rot[shoulder];
        c.point = Some(Vec3::new(4.0, 1.0, 1.5));
        for frame in 1..100 {
            let matrices = evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
            assert!(matrices.iter().all(|matrix| matrix.is_finite()));
            assert!(matrices[wrist].w_axis.truncate().distance(matrices[shoulder].w_axis.truncate()) < 1.0);
        }
        assert!(c.rot[shoulder].angle_between(original) > 0.2, "the arm must actually move");
        let first = c.rot[shoulder];
        c.point = Some(Vec3::new(3.0, -3.0, 2.5));
        for frame in 100..200 {
            evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        assert!(c.rot[shoulder].angle_between(first) > 0.1, "the arm follows a new target");
        c.point = None;
        for frame in 200..250 {
            evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        assert!(c.rot[shoulder].angle_between(original) < 0.001, "no frozen arm after deselection");
        assert!(!c.pointing.editing());
    }

    #[test]
    fn editing_obeys_stronger_animation_and_control_avatar_and_pause_rules() {
        let rig = rig();
        let shoulder = rig.names["mShoulderLeft"];
        let now = Instant::now();
        let id = Uuid::new_v4();
        let desired = Quat::from_rotation_x(0.6);
        let clip = Arc::new(BoundAnim::bind(
            Animation {
                base_priority: 4,
                duration: 1.0,
                emote_name: String::new(),
                loop_in: 0.0,
                loop_out: 1.0,
                looping: true,
                ease_in: 0.0,
                ease_out: 0.0,
                hand_pose: 3,
                joints: vec![JointMotion {
                    joint_name: "mShoulderLeft".into(),
                    priority: 4,
                    rot_keys: vec![aurora_assets::anim::RotKey {
                        time: 0.0,
                        rotation: desired,
                    }],
                    pos_keys: Vec::new(),
                }],
            },
            &rig,
        ));
        let mut c = Controller {
            point: Some(Vec3::new(4.0, 0.0, 1.5)),
            ..Controller::default()
        };
        c.sync(
            &[PlayingAnimation {
                id,
                sequence: 1,
                sequence_start: now,
                continuous_start: now,
            }],
            now,
            |_| Some(clip.clone()),
        );
        evaluate(&mut c, &rig, now + Duration::from_secs(1));
        assert!(c.rot[shoulder].angle_between(desired) < 0.001);
        c.set_paused(true, now + Duration::from_secs(1));
        let held = evaluate(&mut c, &rig, now + Duration::from_secs(2));
        c.point = Some(Vec3::splat(-5.0));
        assert_eq!(evaluate(&mut c, &rig, now + Duration::from_secs(4)), held);
        let mut control = Controller::for_control_avatar();
        control.point = c.point;
        evaluate(&mut control, &rig, now);
        evaluate(&mut control, &rig, now + Duration::from_secs(1));
        assert_eq!(control.rot, rig.local_rot);
    }

    #[test]
    fn gun_aim_adds_a_bounded_torso_rotation_and_eases_out() {
        let rig = rig();
        let now = Instant::now();
        let torso = rig.names["mTorso"];
        let mut c = Controller {
            aiming: true,
            look: Some(Vec3::new(3.0, -2.0, 1.0)),
            ..Controller::default()
        };
        evaluate(&mut c, &rig, now);
        for frame in 1..120 {
            evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        assert!(c.pointing.torso.angle_between(Quat::IDENTITY) > 0.1);
        assert!(c.pointing.torso.angle_between(Quat::IDENTITY) <= std::f32::consts::FRAC_PI_2 * 0.8 + 0.01);
        assert!(
            c.rot[torso].angle_between(Quat::IDENTITY) < 0.4,
            "held pose contains only the head's small torso contribution"
        );
        c.aiming = false;
        for frame in 120..180 {
            evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        assert_eq!(c.pointing.target.weight, 0.0);
    }

    #[test]
    fn editing_priority_ties_follow_motion_activation_order() {
        let rig = rig();
        let shoulder = rig.names["mShoulderLeft"];
        let now = Instant::now();
        let id = Uuid::new_v4();
        let later = Uuid::new_v4();
        let clip = |angle| {
            Arc::new(BoundAnim::bind(
                Animation {
                    base_priority: 2,
                    duration: 1.0,
                    emote_name: String::new(),
                    loop_in: 0.0,
                    loop_out: 1.0,
                    looping: true,
                    ease_in: 0.0,
                    ease_out: 0.0,
                    hand_pose: 1,
                    joints: vec![JointMotion {
                        joint_name: "mShoulderLeft".into(),
                        priority: 2,
                        rot_keys: vec![aurora_assets::anim::RotKey {
                            time: 0.0,
                            rotation: Quat::from_rotation_x(angle),
                        }],
                        pos_keys: Vec::new(),
                    }],
                },
                &rig,
            ))
        };
        let older = clip(0.6);
        let newer = clip(-0.5);
        let signal = |id, at| PlayingAnimation {
            id,
            sequence: 1,
            sequence_start: at,
            continuous_start: at,
        };
        let mut c = Controller::default();
        c.sync(&[signal(id, now)], now, |_| Some(older.clone()));
        evaluate(&mut c, &rig, now);
        c.point = Some(Vec3::new(4.0, 1.0, 1.5));
        for frame in 1..100 {
            evaluate(&mut c, &rig, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        assert!(
            c.rot[shoulder].angle_between(Quat::from_rotation_x(0.6)) > 0.2,
            "a newer editing motion wins priority 2"
        );
        assert!(
            c.live_morphs().iter().any(|&(param, weight)| param == 669 && weight > 0.9),
            "the editing hand request has priority 3"
        );
        let at = now + Duration::from_secs(2);
        c.sync(&[signal(id, now), signal(later, at)], at, |key| {
            Some(if *key == id { older.clone() } else { newer.clone() })
        });
        evaluate(&mut c, &rig, at);
        assert!(
            c.rot[shoulder].angle_between(Quat::from_rotation_x(-0.5)) < 0.001,
            "a newer priority-2 animation wins over editing"
        );
    }

    #[test]
    fn gun_targeting_depends_on_direction_rather_than_target_distance() {
        let rig = rig();
        let now = Instant::now();
        let direction = Vec3::new(3.0, -2.0, 1.0);
        let mut near = Controller {
            aiming: true,
            look: Some(direction),
            ..Controller::default()
        };
        let mut far = Controller {
            aiming: true,
            look: Some(direction * 10.0),
            ..Controller::default()
        };
        for frame in 0..120 {
            let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
            evaluate(&mut near, &rig, at);
            evaluate(&mut far, &rig, at);
        }
        assert!(near.pointing.torso.angle_between(far.pointing.torso) < 0.002);
    }
}
