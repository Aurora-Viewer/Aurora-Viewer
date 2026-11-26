//! `AURORA_DEMO_INVENTORY=animation-play-two`: two animations of the
//! inventory, one per arm, that play at the same time (#4790).

use glam::Quat;
use uuid::Uuid;

/// The clips, each keying one shoulder alone: both arms are raised only
/// while both play.
pub fn clips() -> Vec<(Uuid, aurora_assets::Animation)> {
    let base = super::idle_animation();
    [("mShoulderLeft", 1.2), ("mShoulderRight", -1.2)]
        .into_iter()
        .zip(0u128..)
        .map(|((joint, angle), n)| {
            let mut clip = base.clone();
            clip.base_priority = 4;
            clip.joints = vec![aurora_assets::JointMotion {
                joint_name: joint.into(),
                priority: 4,
                rot_keys: vec![aurora_assets::anim::RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_x(angle),
                }],
                pos_keys: Vec::new(),
            }];
            (Uuid::from_u128(0xD0D0_A111_0000_0000_0000_0000_0000_4790 + (n << 32)), clip)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_clips_key_different_joints() {
        let clips = clips();
        assert_eq!(clips.len(), 2);
        assert_ne!(clips[0].0, clips[1].0);
        assert_ne!(clips[0].1.joints[0].joint_name, clips[1].1.joints[0].joint_name);
    }
}
