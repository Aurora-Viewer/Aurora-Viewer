//! Demo of the pose stand (toolbar `pose_stand`), without a grid:
//! `AURORA_DEMO_POSE_STAND=<mode>` puts the Pose stand button at the end of
//! the bottom toolbar (for this run only) and opens the stand at frame 600.
//! The demo simulator answers the stand's AgentAnimation as the grid does:
//! our avatar plays the pose asked for, over the stand.
//!
//! The poses are grid animations the demo has no server for: two have a
//! stand-in here (the default "Arms down, Legs together" and the T-pose);
//! the others stay missing, so the avatar keeps its stand, as a pose whose
//! asset cannot be fetched.
//!
//! - `1` or `open`: the stand opens on its default pose, the T-pose is
//!   chosen at frame 680 and the stand stays open.
//! - `cycle`: every pose of the list in turn, one each 40 frames from frame
//!   640, then the stand closes (frame 1080). Each start, stop and the
//!   closing are logged ("pose stand: …").

use crate::pose_stand::POSES;
use aurora_net::{NetCommand, NetEvent};
use std::sync::OnceLock;
use uuid::Uuid;

/// The frame the stand opens.
pub const OPEN_FRAME: u64 = 600;
/// `open`: the frame the T-pose is chosen.
pub const T_POSE_FRAME: u64 = 680;
/// `cycle`: the first pose's frame, and the frames between two.
pub const CYCLE_FROM: u64 = 640;
pub const CYCLE_STEP: u64 = 40;

/// The T-pose of posestand.xml.
pub const T_POSE: Uuid = POSES[4].id;

pub fn mode() -> Option<&'static str> {
    static MODE: OnceLock<Option<String>> = OnceLock::new();
    MODE.get_or_init(|| {
        let mode = std::env::var("AURORA_DEMO_POSE_STAND").ok()?;
        match mode.trim() {
            "" | "0" => None,
            "1" => Some("open".into()),
            m => Some(m.to_owned()),
        }
    })
    .as_deref()
}

/// What the scenario does at `frame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Open,
    Pose(Uuid),
    Close,
}

/// The scenario's step at `frame`, if any.
pub fn step(mode: &str, frame: u64) -> Option<Step> {
    if frame == OPEN_FRAME {
        return Some(Step::Open);
    }
    match mode {
        "open" => (frame == T_POSE_FRAME).then_some(Step::Pose(T_POSE)),
        "cycle" if frame >= CYCLE_FROM && (frame - CYCLE_FROM).is_multiple_of(CYCLE_STEP) => {
            let poses = crate::pose_stand::sorted();
            let index = ((frame - CYCLE_FROM) / CYCLE_STEP) as usize;
            match index.cmp(&poses.len()) {
                std::cmp::Ordering::Less => Some(Step::Pose(poses[index].id)),
                std::cmp::Ordering::Equal => Some(Step::Close),
                std::cmp::Ordering::Greater => None,
            }
        }
        _ => None,
    }
}

/// The simulator's answer to a pose started or stopped: our animations,
/// the stand and the pose while it plays.
pub fn reply(cmd: &NetCommand) -> Option<Vec<NetEvent>> {
    let NetCommand::AgentAnimation { anim, start } = cmd else {
        return None;
    };
    if !POSES.iter().any(|p| p.id == *anim) {
        return None;
    }
    let mut anims = vec![(super::IDLE_ANIM, 1)];
    if *start {
        anims.push((*anim, 2));
    }
    Some(vec![NetEvent::AvatarAnimations {
        avatar: super::DEMO_AGENT,
        anims,
        sources: Vec::new(),
    }])
}

/// The stand-ins of two poses: the arms down (the stand's own arms, held),
/// and the T-pose (the skeleton's rest: arms out).
pub fn animations() -> Vec<(Uuid, aurora_assets::Animation)> {
    use aurora_assets::anim::{JointMotion, RotKey};
    use glam::Quat;
    let joint = |name: &str, rotation: Quat| JointMotion {
        joint_name: name.into(),
        priority: -1,
        rot_keys: vec![RotKey { time: 0.0, rotation }],
        pos_keys: Vec::new(),
    };
    let held = |joints: Vec<JointMotion>| aurora_assets::Animation {
        base_priority: 4,
        duration: 1.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 1.0,
        looping: true,
        ends_after: None,
        ease_in: 0.3,
        ease_out: 0.3,
        hand_pose: 0,
        joints,
    };
    let down = 1.35f32;
    vec![
        (
            crate::pose_stand::DEFAULT_POSE,
            held(vec![
                joint("mShoulderLeft", Quat::from_rotation_x(-down)),
                joint("mShoulderRight", Quat::from_rotation_x(down)),
                joint("mElbowLeft", Quat::IDENTITY),
                joint("mElbowRight", Quat::IDENTITY),
                joint("mHipLeft", Quat::IDENTITY),
                joint("mHipRight", Quat::IDENTITY),
            ]),
        ),
        (
            T_POSE,
            held(vec![
                joint("mShoulderLeft", Quat::IDENTITY),
                joint("mShoulderRight", Quat::IDENTITY),
                joint("mElbowLeft", Quat::IDENTITY),
                joint("mElbowRight", Quat::IDENTITY),
                joint("mWristLeft", Quat::IDENTITY),
                joint("mWristRight", Quat::IDENTITY),
            ]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_goes_through_every_pose_then_closes() {
        assert_eq!(step("cycle", OPEN_FRAME), Some(Step::Open));
        let poses: Vec<Uuid> = (0..POSES.len() as u64)
            .filter_map(|i| match step("cycle", CYCLE_FROM + i * CYCLE_STEP) {
                Some(Step::Pose(id)) => Some(id),
                _ => None,
            })
            .collect();
        assert_eq!(poses.len(), POSES.len());
        assert!(POSES.iter().all(|p| poses.contains(&p.id)));
        let end = CYCLE_FROM + POSES.len() as u64 * CYCLE_STEP;
        assert_eq!(step("cycle", end), Some(Step::Close));
        assert_eq!(step("cycle", end + CYCLE_STEP), None);
        assert_eq!(step("open", T_POSE_FRAME), Some(Step::Pose(T_POSE)));
    }

    #[test]
    fn the_simulator_plays_the_pose_over_the_stand() {
        let start = reply(&NetCommand::AgentAnimation { anim: T_POSE, start: true });
        let Some([NetEvent::AvatarAnimations { anims, .. }]) = start.as_deref() else {
            panic!("no answer to a pose");
        };
        assert_eq!(anims, &vec![(super::super::IDLE_ANIM, 1), (T_POSE, 2)]);
        let other = reply(&NetCommand::AgentAnimation {
            anim: Uuid::from_u128(7),
            start: true,
        });
        assert!(other.is_none());
    }
}
