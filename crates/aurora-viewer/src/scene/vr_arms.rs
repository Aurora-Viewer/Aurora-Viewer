//! Our avatar's arms reaching for the VR controllers, in this viewer only:
//! the protocol carries no hand positions, so other viewers keep showing the
//! animation. Each arm is solved over the animated pose with the two-segment
//! solver of the hand contacts (`hand_ik::solve`); the wrist keeps its
//! animated bend, and nothing is written back into the animation's state.

use super::anim::Rig;
use glam::{Quat, Vec3};

/// The wrist targets, left then right, in the avatar's model space.
pub type Hands = [Option<Vec3>; 2];

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
        let Some(target) = *target else { continue };
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
