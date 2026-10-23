//! Local authoring samples. Native assets have continuous key times rather
//! than a stored frame rate; the editor uses a fixed 30 Hz inspection grid.
use super::*;
use crate::scene::anim::BoundAnim;
use std::sync::Arc;

pub const FPS: f32 = 30.0;

#[derive(Clone)]
pub struct Sample {
    pub animation: Arc<BoundAnim>,
    pub frame: u32,
}

pub fn last_frame(duration: f32) -> u32 {
    if duration.is_finite() {
        (duration.clamp(0.0, 60.0) * FPS).ceil() as u32
    } else {
        0
    }
}

/// LLJointStateBlender's per-joint priorities, with explicit list order in
/// place of motion start time. Earlier rows win ties; rotation and translation
/// remain independent channels. No asset, controller or simulator is changed.
pub fn apply(rig: &Rig, pose: &mut LocalPose, pelvis: Vec3, samples: &[Sample]) {
    let mut rotations: Vec<Option<(i32, Quat)>> = vec![None; rig.len()];
    let mut positions: Vec<Option<(i32, Vec3)>> = vec![None; rig.len()];
    for sample in samples.iter().rev() {
        let animation = &sample.animation.anim;
        let time = (sample.frame.min(last_frame(animation.duration)) as f32 / FPS).min(animation.duration);
        if !time.is_finite() {
            continue;
        }
        for (track, bone) in animation.joints.iter().zip(&sample.animation.joints) {
            let Some(bone) = bone.filter(|b| *b < rig.len()) else { continue };
            let priority = if track.priority >= 0 {
                track.priority
            } else {
                animation.base_priority
            };
            if rotations[bone].is_none_or(|(p, _)| priority >= p)
                && let Some(q) = track.rotation_at(time).filter(|q| q.is_finite() && q.length_squared() > 0.00001)
            {
                rotations[bone] = Some((priority, q.normalize()));
            }
            if positions[bone].is_none_or(|(p, _)| priority >= p)
                && let Some(p) = track.position_at(time).filter(|p| p.is_finite())
            {
                positions[bone] = Some((priority, p));
            }
        }
    }
    for bone in 0..rig.len().min(pose.rotations.len()).min(pose.positions.len()) {
        if let Some((_, q)) = rotations[bone] {
            pose.rotations[bone] = q;
        }
        if let Some((_, p)) = positions[bone] {
            pose.positions[bone] = if bone == rig.pelvis { rig.local_pos[bone] + p - pelvis } else { p };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use aurora_assets::{
        Animation, JointMotion,
        anim::{PosKey, RotKey},
    };

    fn sample(rig: &Rig, base_priority: i32, joint_priority: i32, q: Quat, frame: u32) -> Sample {
        Sample {
            frame,
            animation: Arc::new(BoundAnim::bind(
                Animation {
                    base_priority,
                    duration: 2.0,
                    emote_name: String::new(),
                    looping: true,
                    loop_in: 0.0,
                    loop_out: 2.0,
                    ease_in: 1.0,
                    ease_out: 1.0,
                    hand_pose: 0,
                    joints: vec![JointMotion {
                        joint_name: "mHead".into(),
                        priority: joint_priority,
                        rot_keys: vec![
                            RotKey {
                                time: 0.0,
                                rotation: Quat::IDENTITY,
                            },
                            RotKey { time: 2.0, rotation: q },
                        ],
                        pos_keys: Vec::new(),
                    }],
                },
                rig,
            )),
        }
    }
    fn local(rig: &Rig) -> LocalPose {
        LocalPose {
            rotations: rig.local_rot.clone(),
            positions: rig.local_pos.clone(),
        }
    }

    #[test]
    fn fixed_frames_reorder_equal_priorities_but_keep_native_joint_priorities() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let head = rig.names["mHead"];
        let a = sample(rig, 2, -1, Quat::from_rotation_z(1.0), 30);
        let b = sample(rig, 2, -1, Quat::from_rotation_x(0.6), 60);
        let mut pose = local(rig);
        apply(rig, &mut pose, Vec3::ZERO, &[a.clone(), b.clone()]);
        assert!(pose.rotations[head].dot(Quat::from_rotation_z(0.5)).abs() > 0.99999);
        apply(rig, &mut pose, Vec3::ZERO, &[b.clone(), a.clone()]);
        assert!(pose.rotations[head].dot(Quat::from_rotation_x(0.6)).abs() > 0.99999);
        let high = sample(rig, 0, 5, Quat::from_rotation_x(0.9), 60);
        let low = sample(rig, 6, 0, Quat::from_rotation_z(0.8), 60);
        for rows in [vec![high.clone(), low.clone()], vec![low, high]] {
            apply(rig, &mut pose, Vec3::ZERO, &rows);
            assert!(pose.rotations[head].dot(Quat::from_rotation_x(0.9)).abs() > 0.99999);
        }
        let mut translation = sample(rig, 1, -1, Quat::IDENTITY, 60);
        let bound = Arc::get_mut(&mut translation.animation).unwrap();
        bound.anim.joints[0].rot_keys.clear();
        bound.anim.joints[0].pos_keys = vec![PosKey {
            time: 0.0,
            position: Vec3::new(0.1, 0.2, 0.3),
        }];
        apply(rig, &mut pose, Vec3::ZERO, &[translation, b]);
        assert_eq!(
            pose.positions[head],
            Vec3::new(0.1, 0.2, 0.3),
            "a rotation-only layer cannot hide translation"
        );
        assert_eq!(last_frame(2.015), 61);
        assert_eq!(last_frame(f32::NAN), 0);
    }

    #[test]
    fn frame_changes_and_live_suppression_do_not_bake_into_frozen_input_and_ik_stays_last() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let head = rig.names["mHead"];
        let now = Instant::now();
        let pelvis = Vec3::new(0.03, 0.0, 0.2);
        let mut real = rig.local_rot.clone();
        real[head] = Quat::from_rotation_y(0.4);
        let mut shaped = rig.local_pos.clone();
        shaped[rig.pelvis] += Vec3::Z * 0.12;
        shaped[head] += Vec3::Y * 0.01;
        let mut animated = shaped.clone();
        animated[head] += Vec3::X * 0.02;
        let mut pose = Pose::default();
        pose.set_rest_positions(&shaped);
        pose.edit.active = true;
        let output = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        assert_eq!(output.rotations, real);
        pose.edit.animations = vec![sample(rig, 2, -1, Quat::from_rotation_z(1.0), 0)];
        let first = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        assert!(first.rotations[head].abs_diff_eq(Quat::IDENTITY, 1e-6));
        pose.edit.animations[0].frame = 60;
        let last = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        assert!(
            last.rotations[head].dot(Quat::from_rotation_z(1.0)).abs() > 0.99999,
            "sample endpoints bypass looping and ease-in"
        );
        pose.edit.animations.clear();
        pose.edit.suppress_live = true;
        let neutral = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        assert_eq!(neutral.rotations, rig.local_rot);
        assert_eq!(neutral.positions[head], shaped[head]);
        let world = rig.world_with(&neutral.rotations, pelvis, &neutral.positions, &rig.scale);
        assert!(
            world[rig.pelvis].w_axis.truncate().distance(shaped[rig.pelvis]) < 1e-5,
            "neutral removes animated pelvis offset, preserving shape"
        );
        pose.edit.suppress_live = false;
        let restored = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        assert_eq!(restored.rotations, real);
        assert_eq!(restored.positions, animated);
        pose.edit.animations = vec![sample(rig, 2, -1, Quat::from_rotation_z(1.0), 60)];
        let target = Vec3::new(0.2, 0.1, 1.6);
        let rotation = Quat::from_rotation_x(0.7);
        pose.edit.goals.push(Goal {
            follow: None,
            bone: head,
            root: head,
            position: target,
            rotation: Some(rotation),
            contact: Vec3::ZERO,
            pole: None,
            mode: Mode::Direct,
            weight: 1.0,
        });
        let pinned = pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, true).unwrap();
        let world = rig.world_with(&pinned.rotations, pelvis, &pinned.positions, &rig.scale);
        assert!(world[head].w_axis.truncate().distance(target) < 1e-5);
        assert!(world[head].to_scale_rotation_translation().1.dot(rotation).abs() > 0.99999);
        pose.edit = EditRequest::default();
        assert!(pose.apply(rig, &real, &animated, pelvis, &rig.scale, now, false).is_none());
        assert_eq!(real[head], Quat::from_rotation_y(0.4));
    }
}
