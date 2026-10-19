//! Standing leg IK and landing recovery, before animation priority blending.
//! Port of LLKeyframeStandMotion / LLKeyframeFallMotion (indra/llcharacter,
//! originally LGPL 2.1). The analytical two-bone solve preserves bone lengths.

use super::*;
use crate::ao::{STAND_VARIANTS, State};
use glam::Vec4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Surface {
    pub position: Vec3,
    pub normal: Vec3,
}

#[derive(Clone, Copy)]
pub(crate) struct Frame {
    pub to_world: Mat4,
    pub contact: Option<Vec4>,
    pub velocity_z: f32,
    pub pelvis_height: f32,
    pub normal: Vec3,
}

pub(super) struct Stand {
    base: Option<SkeletonBase>,
    corrections: [[Quat; 3]; 2],
    sampled: Option<Instant>,
    root: Option<Vec3>,
}

impl Default for Stand {
    fn default() -> Self {
        Self {
            base: None,
            corrections: [[Quat::IDENTITY; 3]; 2],
            sampled: None,
            root: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Landing {
    pub rotation: Quat,
    pub ease_in: f32,
    pub support_sampled: bool,
}

impl Landing {
    pub(super) fn new(frame: Frame) -> Self {
        let (_, rotation, _) = frame.to_world.to_scale_rotation_translation();
        let normal = rotation.inverse() * frame.normal;
        let forward = (Vec3::X - normal * normal.x).normalize_or(Vec3::X);
        Self {
            rotation: Quat::from_mat3(&Mat3::from_cols(forward, normal.cross(forward), normal)).normalize(),
            support_sampled: false,
            ease_in: if frame.velocity_z < -0.001 {
                (frame.pelvis_height / -frame.velocity_z).clamp(0.01, 10.0)
            } else {
                0.4
            },
        }
    }
}

pub(super) fn standing(id: Uuid) -> bool {
    id == State::Standing.motion() || id == State::Crouching.motion() || STAND_VARIANTS.contains(&id)
}

pub(super) fn landing(id: Uuid) -> bool {
    id == State::StandingUp.motion()
}

fn rotation(world: &[Mat4], joint: usize) -> Quat {
    world[joint].to_scale_rotation_translation().1.normalize()
}

/// A rigid chain's two segment rotations, with forward-facing knees even
/// when the input pose is straight. Reject degenerate mesh skeletons.
fn solve(hip: Vec3, knee: Vec3, ankle: Vec3, target: Vec3) -> Option<(Quat, Quat)> {
    let upper = knee - hip;
    let lower = ankle - knee;
    let (a, b) = (upper.length(), lower.length());
    let delta = target - hip;
    let distance = delta.length();
    if !a.is_finite() || !b.is_finite() || !distance.is_finite() || a < 1e-5 || b < 1e-5 || distance < 1e-5 {
        return None;
    }
    let axis = delta / distance;
    let distance = distance.clamp((a - b).abs() + 1e-6, a + b - 1e-6);
    let along = (a * a - b * b + distance * distance) / (2.0 * distance);
    let pole = (Vec3::X - axis * axis.x)
        .try_normalize()
        .unwrap_or_else(|| axis.any_orthonormal_vector());
    let bend = hip + axis * along + pole * (a * a - along * along).max(0.0).sqrt();
    let hip_rot = Quat::from_rotation_arc(upper / a, (bend - hip).normalize());
    let knee_rot = Quat::from_rotation_arc((hip_rot * lower).normalize(), (hip + axis * distance - bend).normalize());
    Some((hip_rot, knee_rot))
}

impl Stand {
    fn active(&self) -> bool {
        self.corrections.iter().flatten().any(|q| !q.abs_diff_eq(Quat::IDENTITY, 1e-5))
    }

    pub(super) fn resume(&mut self, duration: Duration) {
        self.sampled = self.sampled.and_then(|at| at.checked_add(duration));
    }

    pub(super) fn sample(
        &mut self,
        rig: &Rig,
        anim: &BoundAnim,
        time: f32,
        base: &SkeletonBase,
        frame: Option<Frame>,
        now: Instant,
        ground: &dyn Fn(Vec3) -> Option<Surface>,
    ) -> Option<Vec<Quat>> {
        if frame.is_none() && !self.active() {
            self.sampled = None;
            return None;
        }
        let chains = [["mHipLeft", "mKneeLeft", "mAnkleLeft"], ["mHipRight", "mKneeRight", "mAnkleRight"]];
        let mut joints = [[0; 3]; 2];
        for (dst, names) in joints.iter_mut().zip(chains) {
            for (dst, name) in dst.iter_mut().zip(names) {
                *dst = *rig.names.get(name)?;
            }
            if rig.parent[dst[1]] != Some(dst[0]) || rig.parent[dst[2]] != Some(dst[1]) {
                return None;
            }
        }
        let root = frame.map(|f| f.to_world.w_axis.truncate());
        let teleported = root.zip(self.root).is_some_and(|(a, b)| a.distance_squared(b) > 4.0);
        if self.base.as_ref() != Some(base) || teleported {
            self.base = Some(base.clone());
            self.corrections = [[Quat::IDENTITY; 3]; 2];
            self.sampled = None;
        }
        self.root = root.or(self.root);
        let dt = self
            .sampled
            .replace(now)
            .map_or(0.0, |at| now.saturating_duration_since(at).as_secs_f32())
            .min(0.1);
        let mut rot = rig.local_rot.clone();
        let mut pos = base.local_pos.clone();
        let mut pelvis_offset = Vec3::ZERO;
        for (motion, joint) in anim.anim.joints.iter().zip(&anim.joints) {
            let Some(joint) = *joint else { continue };
            if let Some(r) = motion.rotation_at_loop(time, anim.anim.loop_range()) {
                rot[joint] = r;
            }
            if let Some(p) = motion.position_at_loop(time, anim.anim.loop_range()).filter(|p| p.is_finite()) {
                if joint == rig.pelvis {
                    pelvis_offset = p - (base.local_pos[joint] - rig.local_pos[joint]);
                } else {
                    pos[joint] = p;
                }
            }
        }
        let projected = frame.and_then(|frame| {
            let world = rig.world_with(&rot, pelvis_offset, &pos, &base.scale);
            project(rig, joints, &world, &pos, &base.scale, frame, ground)
        });
        // Human-requested improvement over LLKeyframeStandMotion's retarget
        // thresholds: continuously sample support and smooth only the IK
        // corrections, never the authored pose. The 80 ms half-life is time
        // based, so starts, steps, support loss and the setting's fade-out
        // behave alike at different frame rates. Shape resets/teleports start
        // from the new authored pose; editing suspends this clock.
        let alpha = 1.0 - 2.0_f32.powf(-dt / 0.08);
        for (side, chain) in joints.into_iter().enumerate() {
            for (bone, joint) in chain.into_iter().enumerate() {
                let desired = projected.map_or(Quat::IDENTITY, |pose| (rot[joint].inverse() * pose[side][bone]).normalize());
                let desired = if desired.w < 0.0 { -desired } else { desired };
                let correction = &mut self.corrections[side][bone];
                *correction = correction.slerp(desired, alpha).normalize();
                if projected.is_none() && correction.abs_diff_eq(Quat::IDENTITY, 1e-5) {
                    *correction = Quat::IDENTITY;
                }
                rot[joint] = (rot[joint] * *correction).normalize();
            }
        }
        Some(rot)
    }
}

fn project(
    rig: &Rig,
    joints: [[usize; 3]; 2],
    world: &[Mat4],
    pos: &[Vec3],
    scale: &[Vec3],
    frame: Frame,
    ground: &dyn Fn(Vec3) -> Option<Surface>,
) -> Option<[[Quat; 3]; 2]> {
    let inverse = frame.to_world.inverse();
    let mut result = [[Quat::IDENTITY; 3]; 2];
    for (side, [hip, knee, ankle]) in joints.into_iter().enumerate() {
        let p = |j: usize| world[j].w_axis.truncate();
        let target = ground(frame.to_world.transform_point3(p(ankle)))?;
        // Preserve the sole's clearance below the ankle. Projecting the
        // ankle itself onto the surface sinks mFoot by its local height.
        let foot_name = if side == 0 { "mFootLeft" } else { "mFootRight" };
        let clearance = rig
            .names
            .get(foot_name)
            .filter(|&&j| rig.parent[j] == Some(ankle))
            .map_or(0.0, |&j| (-pos[j].z * scale[ankle].z).max(0.0));
        let target_pos = inverse.transform_point3(target.position + target.normal * clearance);
        let (hip_delta, knee_delta) = solve(p(hip), p(knee), p(ankle), target_pos)?;
        let parent_rot = rig.parent[hip].map_or(Quat::IDENTITY, |j| rotation(world, j));
        let hip_rot = (hip_delta * rotation(world, hip)).normalize();
        let knee_rot = (knee_delta * hip_delta * rotation(world, knee)).normalize();
        let normal = inverse.transform_vector3(target.normal).normalize_or(Vec3::Z);
        let old_forward = (knee_delta * hip_delta * rotation(world, ankle)) * Vec3::X;
        let forward = (old_forward - normal * old_forward.dot(normal)).normalize_or(Vec3::X);
        let ankle_rot = Quat::from_mat3(&Mat3::from_cols(forward, normal.cross(forward), normal)).normalize();
        result[side] = [
            (parent_rot.inverse() * hip_rot).normalize(),
            (hip_rot.inverse() * knee_rot).normalize(),
            (knee_rot.inverse() * ankle_rot).normalize(),
        ];
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_assets::anim::{JointMotion, RotKey};

    fn fixture() -> (Arc<Rig>, SkeletonBase, Arc<BoundAnim>) {
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = lib.rig.clone();
        let base = SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        let mut animation = crate::demo::idle_animation();
        for name in ["mAnkleLeft", "mAnkleRight", "mPelvis"] {
            animation.joints.push(JointMotion {
                joint_name: name.into(),
                priority: -1,
                rot_keys: vec![RotKey {
                    time: 0.0,
                    rotation: Quat::IDENTITY,
                }],
                pos_keys: Vec::new(),
            });
        }
        let anim = Arc::new(BoundAnim::bind(animation, &rig));
        (rig, base, anim)
    }

    fn frame() -> Frame {
        Frame {
            to_world: Mat4::IDENTITY,
            contact: None,
            velocity_z: 0.0,
            pelvis_height: 1.0,
            normal: Vec3::Z,
        }
    }

    fn signal(id: Uuid, start: Instant) -> PlayingAnimation {
        PlayingAnimation {
            id,
            sequence: 1,
            sequence_start: start,
            continuous_start: start,
        }
    }

    #[test]
    fn grounding_two_bone_solve_preserves_lengths_and_handles_unreachable_targets() {
        let (hip, knee, ankle) = (Vec3::Z, Vec3::Z * 0.5, Vec3::ZERO);
        for target in [Vec3::new(0.1, 0.0, 0.3), Vec3::new(0.0, 0.2, -5.0), Vec3::Z * 0.999] {
            let (a, b) = solve(hip, knee, ankle, target).expect("valid bones");
            let new_knee = hip + a * (knee - hip);
            let new_ankle = new_knee + b * a * (ankle - knee);
            assert!((new_knee.distance(hip) - 0.5).abs() < 1e-5);
            assert!((new_ankle.distance(new_knee) - 0.5).abs() < 1e-5);
            if hip.distance(target) < 1.0 {
                assert!(new_ankle.distance(target) < 1e-4);
            }
            assert!(a.is_finite() && b.is_finite());
        }
        assert!(solve(hip, hip, ankle, Vec3::X).is_none());
        assert!(solve(hip, knee, ankle, Vec3::NAN).is_none());
    }

    #[test]
    fn grounding_standing_feet_reach_separate_steps_and_follow_slope_normals() {
        let (rig, mut base, anim) = fixture();
        // Mesh joint offsets and nonuniform scales participate in the solve.
        base.local_pos[rig.names["mKneeLeft"]].z *= 0.95;
        base.scale[rig.names["mHipRight"]] = Vec3::new(0.9, 1.1, 1.05);
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
        let ankle = rig.names["mAnkleLeft"];
        let floor = rest[ankle].w_axis.z + 0.2;
        let normal = Vec3::new(0.12, -0.08, 1.0).normalize();
        let mut avatar_frame = frame();
        avatar_frame.to_world = Mat4::from_rotation_translation(Quat::from_rotation_z(0.8), Vec3::new(13.0, -8.0, 20.0));
        let inverse = avatar_frame.to_world.inverse();
        let ground = |p: Vec3| {
            let p = inverse.transform_point3(p);
            Some(Surface {
                position: avatar_frame
                    .to_world
                    .transform_point3(p.with_z(floor + if p.y < 0.0 { 0.07 } else { 0.0 })),
                normal: avatar_frame.to_world.transform_vector3(normal),
            })
        };
        let mut stand = Stand::default();
        let start = Instant::now();
        for i in 0..120 {
            stand.sample(
                &rig,
                &anim,
                0.0,
                &base,
                Some(avatar_frame),
                start + Duration::from_secs_f32(i as f32 / 60.0),
                &ground,
            );
        }
        let rot = stand
            .sample(&rig, &anim, 0.0, &base, Some(avatar_frame), start + Duration::from_secs(2), &ground)
            .expect("settled IK pose");
        let posed = rig.world_with(&rot, Vec3::ZERO, &base.local_pos, &base.scale);
        for side in ["Left", "Right"] {
            let joint = rig.names[&format!("mAnkle{side}")];
            let expected = ground(avatar_frame.to_world.transform_point3(rest[joint].w_axis.truncate())).unwrap();
            let foot = rig.names[&format!("mFoot{side}")];
            let clearance = -base.local_pos[foot].z * base.scale[joint].z;
            let target = inverse.transform_point3(expected.position + expected.normal * clearance);
            assert!(posed[joint].w_axis.truncate().distance(target) < 0.001);
            let sole = avatar_frame.to_world.transform_point3(posed[foot].w_axis.truncate());
            assert!(
                (sole - expected.position).dot(expected.normal).abs() < 0.001,
                "sole stays on the support plane"
            );
            assert!((rotation(&posed, joint) * Vec3::Z).distance(normal) < 0.001);
            let hip = rig.names[&format!("mHip{side}")];
            let knee = rig.names[&format!("mKnee{side}")];
            assert!((posed[hip].w_axis.distance(posed[knee].w_axis) - rest[hip].w_axis.distance(rest[knee].w_axis)).abs() < 0.001);
        }
    }

    #[test]
    fn grounding_controller_preserves_priority_and_live_disable_restores_the_authored_pose() {
        let (rig, base, anim) = fixture();
        let now = Instant::now();
        let stand = signal(State::Standing.motion(), now);
        let mut controller = Controller {
            ground: Some(frame()),
            ..Controller::default()
        };
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); rig.len()];
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(p.z + 0.25),
                normal: Vec3::Z,
            })
        };
        for i in 0..4 {
            let at = now + Duration::from_secs(i);
            controller.sync(&[stand], at, |_| Some(anim.clone()));
            controller.evaluate_on_ground(&rig, at, Some(&base), &mut out, ground);
        }
        let knee = rig.names["mKneeLeft"];
        assert!(!controller.rot[knee].abs_diff_eq(Quat::IDENTITY, 0.01));
        let mut custom = anim.anim.clone();
        custom.base_priority = 6;
        let custom = Arc::new(BoundAnim::bind(custom, &rig));
        let custom_signal = signal(Uuid::from_u128(42), now);
        let mut at = now + Duration::from_secs(5);
        controller.sync(&[stand, custom_signal], at, |id| {
            Some(if *id == stand.id { anim.clone() } else { custom.clone() })
        });
        at += Duration::from_secs(1);
        controller.sync(&[stand, custom_signal], at, |id| {
            Some(if *id == stand.id { anim.clone() } else { custom.clone() })
        });
        controller.evaluate_on_ground(&rig, at, Some(&base), &mut out, ground);
        assert!(controller.rot[knee].abs_diff_eq(Quat::IDENTITY, 1e-5), "priority-six pose wins");
        controller.ground = None;
        for i in 1..=120 {
            let now = at + Duration::from_secs_f32(i as f32 / 60.0);
            controller.sync(&[stand], now, |_| Some(anim.clone()));
            controller.evaluate(&rig, now, Some(&base), &mut out);
        }
        assert!(
            controller.rot[knee].abs_diff_eq(Quat::IDENTITY, 1e-5),
            "disabling restores keyframes"
        );
    }

    #[test]
    fn grounding_landing_tilt_and_ease_follow_velocity_and_recover_by_three_quarters() {
        let (rig, base, anim) = fixture();
        let mut animation = anim.anim.clone();
        animation.looping = false;
        animation.duration = 2.0;
        let anim = Arc::new(BoundAnim::bind(animation, &rig));
        let now = Instant::now();
        let mut f = frame();
        f.normal = Vec3::new(-0.3, 0.0, 1.0).normalize();
        f.velocity_z = -5.0;
        let mut controller = Controller {
            ground: Some(f),
            ..Controller::default()
        };
        let playing = [signal(State::StandingUp.motion(), now)];
        controller.sync(&playing, now, |_| Some(anim.clone()));
        let landing = controller.motions[0].landing.expect("landing activated");
        assert!((landing.ease_in - 0.2).abs() < 1e-5);
        assert!((landing.rotation * Vec3::Z).distance(f.normal) < 1e-5);
        f.velocity_z = 0.0;
        assert_eq!(Landing::new(f).ease_in, 0.4);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); rig.len()];
        for (seconds, tilted) in [(0.8, true), (1.5, false)] {
            let at = now + Duration::from_secs_f32(seconds);
            controller.sync(&playing, at, |_| Some(anim.clone()));
            controller.evaluate(&rig, at, Some(&base), &mut out);
            assert_eq!(!controller.rot[rig.pelvis].abs_diff_eq(Quat::IDENTITY, 1e-4), tilted);
        }
    }

    #[test]
    fn grounding_missing_support_keeps_finite_authored_pose_and_control_avatars_are_excluded() {
        let (rig, base, anim) = fixture();
        let now = Instant::now();
        for control in [false, true] {
            let mut controller = Controller {
                control,
                ground: Some(frame()),
                ..Controller::default()
            };
            let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); rig.len()];
            for i in 0..4 {
                let at = now + Duration::from_secs(i);
                controller.sync(&[signal(State::Standing.motion(), now)], at, |_| Some(anim.clone()));
                controller.evaluate_on_ground(&rig, at, Some(&base), &mut out, |_| None);
            }
            assert!(out.iter().all(|m| Mat4::from_cols_array_2d(m).is_finite()));
            assert!(controller.rot[rig.names["mKneeLeft"]].abs_diff_eq(Quat::IDENTITY, 1e-6));
        }
    }

    #[test]
    fn grounding_corrections_ease_in_retarget_and_fade_out_independently_of_frame_rate() {
        let (rig, base, anim) = fixture();
        let knee = rig.names["mKneeLeft"];
        let now = Instant::now();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(p.z + 0.2),
                normal: Vec3::Z,
            })
        };
        let mut samples = Vec::new();
        for fps in [30, 60, 144] {
            let mut stand = Stand::default();
            let first = stand
                .sample(&rig, &anim, 0.0, &base, Some(frame()), now, &ground)
                .expect("starting pose");
            assert!(first[knee].abs_diff_eq(Quat::IDENTITY, 1e-6), "no activation pop");
            let mut rot = first;
            for i in 1..=fps {
                rot = stand
                    .sample(
                        &rig,
                        &anim,
                        0.0,
                        &base,
                        Some(frame()),
                        now + Duration::from_secs_f64(i as f64 / fps as f64),
                        &ground,
                    )
                    .expect("easing pose");
            }
            samples.push(rot[knee]);
            let before = rot[knee];
            let changed = |p: Vec3| {
                Some(Surface {
                    position: p.with_z(p.z + 0.35),
                    normal: Vec3::new(0.0, -0.1, 1.0).normalize(),
                })
            };
            let switched = stand
                .sample(&rig, &anim, 0.0, &base, Some(frame()), now + Duration::from_secs(1), &changed)
                .expect("same-time retarget");
            assert!(
                switched[knee].abs_diff_eq(before, 1e-6),
                "retargeting cannot jump without elapsed time"
            );
            for i in 1..=fps * 2 {
                rot = stand
                    .sample(
                        &rig,
                        &anim,
                        0.0,
                        &base,
                        None,
                        now + Duration::from_secs_f64(1.0 + i as f64 / fps as f64),
                        &ground,
                    )
                    .unwrap_or_else(|| rig.local_rot.clone());
            }
            assert!(rot[knee].abs_diff_eq(Quat::IDENTITY, 1e-5), "disable settles to the authored pose");
        }
        assert!(samples[0].abs_diff_eq(samples[1], 2e-5));
        assert!(samples[1].abs_diff_eq(samples[2], 2e-5));
    }

    #[test]
    fn grounding_pause_and_teleport_do_not_replay_old_corrections() {
        let (rig, base, anim) = fixture();
        let now = Instant::now();
        let mut stand = Stand::default();
        let ground = |p: Vec3| {
            Some(Surface {
                position: p.with_z(p.z + 0.2),
                normal: Vec3::Z,
            })
        };
        stand.sample(&rig, &anim, 0.0, &base, Some(frame()), now, &ground);
        let before = stand
            .sample(&rig, &anim, 0.0, &base, Some(frame()), now + Duration::from_millis(50), &ground)
            .unwrap();
        stand.resume(Duration::from_secs(10));
        let after = stand
            .sample(&rig, &anim, 0.0, &base, Some(frame()), now + Duration::from_millis(10050), &ground)
            .unwrap();
        assert!(before[rig.names["mKneeLeft"]].abs_diff_eq(after[rig.names["mKneeLeft"]], 1e-6));
        let mut moved = frame();
        moved.to_world = Mat4::from_translation(Vec3::X * 100.0);
        let teleported = stand
            .sample(&rig, &anim, 0.0, &base, Some(moved), now + Duration::from_secs(11), &ground)
            .unwrap();
        assert!(teleported[rig.names["mKneeLeft"]].abs_diff_eq(Quat::IDENTITY, 1e-6));
    }
}
