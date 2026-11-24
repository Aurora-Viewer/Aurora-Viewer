//! The Fit lab's range of motion: poses at the ends of what a body does
//! (arms up, a reach, a hug, a squat, kneeling, a knee raised, a twist, a
//! bend), where a garment stretches, tears or lets the body through. The
//! mannequins hold one, or take them in turn. Aurora's own clips, built
//! here and never fetched or sent: the mannequins are client-only
//! (mannequins.rs).
//!
//! Axes of the avatar skeleton (avatar_skeleton.xml): X forward, Y to the
//! avatar's left, Z up. From the rest T pose: an arm turns down about X
//! (left negative, right positive, as the demo's idle) and forward about Z;
//! a hip swings the leg forward with a negative turn about Y, a knee bends
//! back with a positive one, the spine leans forward with a positive one.

use aurora_assets::Animation;
use aurora_assets::anim::{JointMotion, PosKey, RotKey};
use glam::{Quat, Vec3};
use uuid::Uuid;

/// A pose of the range of motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pose {
    ArmsUp,
    Reach,
    Hug,
    Squat,
    Kneel,
    KneeUp,
    Twist,
    BendForward,
}

/// How long the cycle takes to reach a pose, and holds it (seconds).
const MOVE: f32 = 0.8;
const HOLD: f32 = 1.4;
/// The arms' rest: down along the body (the demo idle's angle).
const ARMS_DOWN: f32 = 1.3;

impl Pose {
    pub const ALL: [Pose; 8] = [
        Pose::ArmsUp,
        Pose::Reach,
        Pose::Hug,
        Pose::Squat,
        Pose::Kneel,
        Pose::KneeUp,
        Pose::Twist,
        Pose::BendForward,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pose::ArmsUp => aurora_i18n::tr!("fit-lab-pose-arms-up"),
            Pose::Reach => aurora_i18n::tr!("fit-lab-pose-reach"),
            Pose::Hug => aurora_i18n::tr!("fit-lab-pose-hug"),
            Pose::Squat => aurora_i18n::tr!("fit-lab-pose-squat"),
            Pose::Kneel => aurora_i18n::tr!("fit-lab-pose-kneel"),
            Pose::KneeUp => aurora_i18n::tr!("fit-lab-pose-knee-up"),
            Pose::Twist => aurora_i18n::tr!("fit-lab-pose-twist"),
            Pose::BendForward => aurora_i18n::tr!("fit-lab-pose-bend"),
        }
    }

    /// Its name in `AURORA_DEMO_BLENDER_LIVE=fit-lab-pose-<name>`.
    pub fn key(self) -> &'static str {
        match self {
            Pose::ArmsUp => "arms-up",
            Pose::Reach => "reach",
            Pose::Hug => "hug",
            Pose::Squat => "squat",
            Pose::Kneel => "kneel",
            Pose::KneeUp => "knee-up",
            Pose::Twist => "twist",
            Pose::BendForward => "bend",
        }
    }

    /// Its joints' rotations, and the pelvis' drop (metres).
    fn joints(self) -> (Vec<(&'static str, Quat)>, f32) {
        let arms = |left: Quat, right: Quat| [("mShoulderLeft", left), ("mShoulderRight", right)];
        let down = |a: f32| arms(Quat::from_rotation_x(-a), Quat::from_rotation_x(a));
        let elbows = |a: f32| [("mElbowLeft", Quat::from_rotation_z(-a)), ("mElbowRight", Quat::from_rotation_z(a))];
        let legs = |hip: f32, knee: f32, ankle: f32| {
            [
                ("mHipLeft", Quat::from_rotation_y(-hip)),
                ("mHipRight", Quat::from_rotation_y(-hip)),
                ("mKneeLeft", Quat::from_rotation_y(knee)),
                ("mKneeRight", Quat::from_rotation_y(knee)),
                ("mAnkleLeft", Quat::from_rotation_y(ankle)),
                ("mAnkleRight", Quat::from_rotation_y(ankle)),
            ]
        };
        match self {
            // overhead, a little forward
            Pose::ArmsUp => (
                [
                    arms(
                        Quat::from_rotation_z(-0.25) * Quat::from_rotation_x(1.35),
                        Quat::from_rotation_z(0.25) * Quat::from_rotation_x(-1.35),
                    )
                    .as_slice(),
                    &elbows(0.15),
                ]
                .concat(),
                0.0,
            ),
            // both arms straight ahead, at shoulder height
            Pose::Reach => (arms(Quat::from_rotation_z(-1.45), Quat::from_rotation_z(1.45)).to_vec(), 0.0),
            // arms crossed before the chest
            Pose::Hug => (
                [
                    arms(
                        Quat::from_rotation_z(-1.75) * Quat::from_rotation_x(-0.25),
                        Quat::from_rotation_z(1.75) * Quat::from_rotation_x(0.25),
                    )
                    .as_slice(),
                    &elbows(1.2),
                ]
                .concat(),
                0.0,
            ),
            // thighs near level, heels down, the torso over the knees, the
            // arms forward for balance
            Pose::Squat => (
                [
                    legs(1.75, 2.2, -0.45).as_slice(),
                    &arms(
                        Quat::from_rotation_z(-1.3) * Quat::from_rotation_x(-0.2),
                        Quat::from_rotation_z(1.3) * Quat::from_rotation_x(0.2),
                    ),
                    &[("mTorso", Quat::from_rotation_y(0.45))],
                ]
                .concat(),
                0.52,
            ),
            // upright on both knees, the shins flat behind
            Pose::Kneel => ([legs(0.0, 1.57, 0.6).as_slice(), &down(ARMS_DOWN)].concat(), 0.48),
            // the left knee raised to the hip
            Pose::KneeUp => (
                vec![
                    ("mHipLeft", Quat::from_rotation_y(-1.6)),
                    ("mKneeLeft", Quat::from_rotation_y(1.7)),
                    down(ARMS_DOWN - 0.35)[0],
                    down(ARMS_DOWN - 0.35)[1],
                ],
                0.0,
            ),
            // the chest turned and leaning to the side
            Pose::Twist => (
                [
                    [
                        ("mTorso", Quat::from_rotation_z(0.55)),
                        ("mChest", Quat::from_rotation_z(0.3) * Quat::from_rotation_x(0.3)),
                    ]
                    .as_slice(),
                    &down(ARMS_DOWN - 0.25),
                ]
                .concat(),
                0.0,
            ),
            // a bow from the hips, arms hanging
            Pose::BendForward => (
                [
                    [
                        ("mTorso", Quat::from_rotation_y(0.9)),
                        ("mChest", Quat::from_rotation_y(0.35)),
                        ("mHipLeft", Quat::from_rotation_y(-0.35)),
                        ("mHipRight", Quat::from_rotation_y(-0.35)),
                    ]
                    .as_slice(),
                    &arms(
                        Quat::from_rotation_z(-0.6) * Quat::from_rotation_x(-ARMS_DOWN),
                        Quat::from_rotation_z(0.6) * Quat::from_rotation_x(ARMS_DOWN),
                    ),
                ]
                .concat(),
                0.0,
            ),
        }
    }
}

/// The ids the mannequins play: one per pose held, one for the cycle.
/// Aurora's own (no asset has them), kept from the server's namespace by
/// their fixed high bits.
pub fn animation_id(pose: Option<Pose>) -> Uuid {
    let index = pose.map_or(0, |p| Pose::ALL.iter().position(|q| *q == p).map_or(0, |i| i + 1));
    Uuid::from_u128(0xF17_1AB0_0000_4000_8000_0000_0000_0000 | index as u128)
}

/// Every clip, with its id: the scene registers them before a mannequin
/// plays one.
pub fn animations() -> Vec<(Uuid, Animation)> {
    std::iter::once(None)
        .chain(Pose::ALL.into_iter().map(Some))
        .map(|pose| (animation_id(pose), animation(pose)))
        .collect()
}

/// The joints every pose sets: a joint a pose leaves alone is at rest
/// there, so that the cycle brings it back.
fn rest(joint: &str) -> Quat {
    match joint {
        "mShoulderLeft" => Quat::from_rotation_x(-ARMS_DOWN),
        "mShoulderRight" => Quat::from_rotation_x(ARMS_DOWN),
        _ => Quat::IDENTITY,
    }
}

/// A pose held (`Some`), or all of them in turn, each reached in `MOVE`
/// and held `HOLD`, the last one leading back to the first.
pub fn animation(pose: Option<Pose>) -> Animation {
    let poses: Vec<Pose> = pose.map_or_else(|| Pose::ALL.to_vec(), |p| vec![p]);
    let step = MOVE + HOLD;
    let duration = if poses.len() == 1 { 1.0 } else { step * poses.len() as f32 };
    let shapes: Vec<_> = poses.iter().map(|p| p.joints()).collect();
    let mut names: Vec<&'static str> = shapes.iter().flat_map(|(joints, _)| joints.iter().map(|(n, _)| *n)).collect();
    names.extend(["mShoulderLeft", "mShoulderRight"]);
    names.sort_unstable();
    names.dedup();
    let at = |shape: &(Vec<(&'static str, Quat)>, f32), name: &str| {
        shape.0.iter().find(|(n, _)| *n == name).map_or_else(|| rest(name), |(_, q)| *q)
    };
    // the times each pose is reached and left, the last one also at the
    // start, so that the loop closes on it
    let times: Vec<(f32, usize)> = if poses.len() == 1 {
        vec![(0.0, 0)]
    } else {
        std::iter::once((0.0, poses.len() - 1))
            .chain((0..poses.len()).flat_map(|i| {
                let start = step * i as f32;
                [(start + MOVE, i), (start + step, i)]
            }))
            .collect()
    };
    let mut joints: Vec<JointMotion> = names
        .iter()
        .map(|name| JointMotion {
            joint_name: (*name).into(),
            priority: 4,
            rot_keys: times
                .iter()
                .map(|&(time, i)| RotKey {
                    time,
                    rotation: at(&shapes[i], name),
                })
                .collect(),
            pos_keys: Vec::new(),
        })
        .collect();
    joints.push(JointMotion {
        joint_name: "mPelvis".into(),
        priority: 4,
        rot_keys: Vec::new(),
        pos_keys: times
            .iter()
            .map(|&(time, i)| PosKey {
                time,
                position: Vec3::NEG_Z * shapes[i].1,
            })
            .collect(),
    });
    Animation {
        base_priority: 4,
        duration,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: duration,
        looping: true,
        ends_after: None,
        ease_in: 0.4,
        ease_out: 0.4,
        // relaxed hands (the upload window's default)
        hand_pose: 1,
        joints,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pose_and_the_cycle_is_a_looping_clip_of_its_own() {
        let all = animations();
        assert_eq!(all.len(), Pose::ALL.len() + 1);
        let mut ids: Vec<Uuid> = all.iter().map(|(id, _)| *id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "one id each");
        for (_, clip) in &all {
            assert!(clip.looping && clip.duration > 0.0);
            for joint in &clip.joints {
                let times: Vec<f32> = joint
                    .rot_keys
                    .iter()
                    .map(|k| k.time)
                    .chain(joint.pos_keys.iter().map(|k| k.time))
                    .collect();
                assert!(times.windows(2).all(|w| w[0] <= w[1]), "{} keys in order", joint.joint_name);
                assert!(times.iter().all(|t| (0.0..=clip.duration).contains(t)));
            }
        }
    }

    #[test]
    fn the_cycle_closes_on_its_last_pose_and_reaches_every_pose() {
        let cycle = animation(None);
        let shoulder = cycle.joints.iter().find(|j| j.joint_name == "mShoulderLeft").expect("arms");
        let first = shoulder.rot_keys.first().expect("keys").rotation;
        let last = shoulder.rot_keys.last().expect("keys").rotation;
        assert!(first.angle_between(last) < 1e-5, "the loop closes");
        assert_eq!(shoulder.rot_keys.len(), 1 + 2 * Pose::ALL.len());
        // the squat and the kneel lower the pelvis, standing poses do not
        let pelvis = |pose| {
            let clip = animation(Some(pose));
            clip.joints
                .iter()
                .find(|j| j.joint_name == "mPelvis")
                .map(|j| j.pos_keys[0].position.z)
        };
        assert!(pelvis(Pose::Squat).is_some_and(|z| z < -0.3));
        assert!(pelvis(Pose::Kneel).is_some_and(|z| z < -0.3));
        assert_eq!(pelvis(Pose::ArmsUp), Some(0.0));
        // arms up raises the left arm above the shoulder: +Y turned to +Z
        let arms_up = animation(Some(Pose::ArmsUp));
        let left = arms_up.joints.iter().find(|j| j.joint_name == "mShoulderLeft").expect("arm");
        assert!((left.rot_keys[0].rotation * Vec3::Y).z > 0.8);
    }
}
