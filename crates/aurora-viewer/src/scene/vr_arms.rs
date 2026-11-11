//! Our avatar's arms reaching for the VR controllers, in this viewer only:
//! the protocol carries no hand positions, so other viewers keep showing the
//! animation. Each arm is solved over the animated pose with the two-segment
//! solver of the hand contacts (`hand_ik::solve`); the wrist keeps its
//! animated bend, and nothing is written back into the animation's state.

use super::anim::Rig;
use glam::{Quat, Vec3};

/// One hand of ours: where its wrist is (the avatar's model space once
/// handed to the pose), how far the grip and the trigger are pulled, and
/// how far the avatar's hand goes for our reach (`arm_scale`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hand {
    pub wrist: Vec3,
    pub grip: f32,
    pub trigger: f32,
    pub scale: f32,
}

/// The hands, left then right.
pub type Hands = [Option<Hand>; 2];

/// Radians each finger joint bends at a full pull: the index on the
/// trigger, the others on the grip.
const CURL: f32 = 0.45;

/// The fingers of one side curled by the grip and the trigger. A finger's
/// joints bend about their local X axis (the avatar's forward at rest, the
/// fingers along Y, the palm down), towards the palm: negative on the left,
/// positive on the right.
fn curl_fingers(rig: &Rig, rotations: &mut [Quat], side: &str, hand: &Hand) {
    let sign = if side == "Left" { -1.0 } else { 1.0 };
    for (finger, pull) in [
        ("Index", hand.trigger),
        ("Middle", hand.grip),
        ("Ring", hand.grip),
        ("Pinky", hand.grip),
    ] {
        let angle = sign * CURL * pull.clamp(0.0, 1.0);
        if angle == 0.0 {
            continue;
        }
        for joint in 1..=3 {
            if let Some(&j) = rig.names.get(&format!("mHand{finger}{joint}{side}"))
                && let Some(q) = rotations.get_mut(j)
            {
                *q = (*q * Quat::from_rotation_x(angle)).normalize();
            }
        }
    }
}

/// Our arm's length from the span of both arms held out to the sides
/// (between the controllers' aim points): the shoulders' width and the
/// hands' length taken away.
pub fn arm_from_span(span: f32) -> f32 {
    ((span - 0.16 - 0.35) * 0.5).max(0.25)
}

impl super::Scene {
    /// Our avatar's arm, shoulder to wrist, in its shape (metres).
    pub fn own_arm_length(&self, world: &crate::world::World) -> Option<f32> {
        let rig = &world.avatar_lib.rig;
        let base = &self.skeletons.get(&world.agent_id)?.base;
        let elbow = *rig.names.get("mElbowLeft")?;
        let wrist = *rig.names.get("mWristLeft")?;
        let length = base.local_pos.get(elbow)?.length() + base.local_pos.get(wrist)?.length();
        (length.is_finite() && length > 0.1).then_some(length)
    }
}

/// The elbow's bend for each side: down, a little back and out from the body,
/// as a relaxed arm's (avatar axes: X forward, Y left, Z up).
fn pole(left: bool) -> Vec3 {
    let out = if left { Vec3::Y } else { Vec3::NEG_Y };
    (out * 0.5 - Vec3::Z - Vec3::X * 0.3).normalize()
}

/// Turn the shoulders, elbows and wrists of `rotations` (local, the frame's
/// blended pose) so that each wrist reaches its target, or as far towards
/// it as the arm goes.
pub(super) fn apply(rig: &Rig, rotations: &mut [Quat], positions: &[Vec3], pelvis: Vec3, scale: &[Vec3], hands: &Hands) {
    if hands.iter().all(Option::is_none) || rotations.len() != rig.len() {
        return;
    }
    let world = rig.world_with(rotations, pelvis, positions, scale);
    for (side, target) in ["Left", "Right"].into_iter().zip(hands) {
        let Some(hand) = *target else { continue };
        let joints = ["Shoulder", "Elbow", "Wrist"].map(|bone| rig.names.get(&format!("m{bone}{side}")).copied());
        let [Some(shoulder), Some(elbow), Some(wrist)] = joints else { continue };
        if rig.parent[elbow] != Some(shoulder) || rig.parent[wrist] != Some(elbow) {
            continue;
        }
        let Some(collar) = rig.parent[shoulder] else { continue };
        let pose = |joint: usize| {
            let (_, rotation, translation) = world[joint].to_scale_rotation_translation();
            (rotation.normalize(), translation)
        };
        let (parent, _) = pose(collar);
        let [(r0, p0), (r1, p1), (r2, p2)] = [shoulder, elbow, wrist].map(pose);
        // our reach scaled to the avatar's arm, from its shoulder
        let target = p0 + (hand.wrist - p0) * hand.scale.clamp(0.3, 3.0);
        let goal = super::hand_ik::Target {
            position: target,
            rotation: None,
            contact: Vec3::ZERO,
        };
        if let Some(solved) = super::hand_ik::solve([p0, p1, p2], [r0, r1, r2], parent, goal, pole(side == "Left")) {
            rotations[shoulder] = solved[0];
            rotations[elbow] = solved[1];
            rotations[wrist] = solved[2];
        }
        curl_fingers(rig, rotations, side, &hand);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_length_comes_from_the_span_less_shoulders_and_hands() {
        // a 1.71 m span: 0.6 m arms
        assert!((arm_from_span(1.71) - 0.6).abs() < 1e-5);
        // a span shorter than the shoulders: a floor, no negative arm
        assert_eq!(arm_from_span(0.2), 0.25);
    }

    #[test]
    fn elbows_bend_down_and_away_from_the_body() {
        for (left, out) in [(true, 1.0), (false, -1.0)] {
            let p = pole(left);
            assert!(p.z < -0.5, "down: {p}");
            assert!(p.y * out > 0.2, "outward: {p}");
            assert!((p.length() - 1.0).abs() < 1e-5);
        }
    }
}
