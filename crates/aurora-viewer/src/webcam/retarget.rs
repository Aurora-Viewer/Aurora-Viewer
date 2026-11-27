use glam::{Mat3, Quat, Vec3};

use super::Frame;
use crate::scene::anim::Rig;

const SIDES: [&str; 2] = ["Left", "Right"];
const FINGERS: [(&str, usize); 5] = [("Thumb", 1), ("Index", 5), ("Middle", 9), ("Ring", 13), ("Pinky", 17)];

#[derive(Clone, Copy)]
pub(super) struct Channels {
    pub head: bool,
    pub eyes: bool,
    pub expression: bool,
    pub hands: bool,
    pub upper_body: bool,
    pub lower_body: bool,
    pub expression_strength: f32,
}

impl Default for Channels {
    fn default() -> Self {
        Self {
            head: true,
            eyes: true,
            expression: true,
            hands: true,
            upper_body: true,
            lower_body: true,
            expression_strength: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
struct Face {
    head: Quat,
    balance: f32,
    jaw: f32,
    openness: [f32; 2],
    gaze: [[f32; 2]; 2],
}

impl Face {
    const REST: Self = Self {
        head: Quat::IDENTITY,
        balance: 1.0,
        jaw: 0.035,
        openness: [0.26; 2],
        gaze: [[0.0; 2]; 2],
    };

    fn measure(points: [[f32; 3]; 16]) -> Option<Self> {
        let p = points.map(Vec3::from_array);
        let left = p[0] - p[1];
        let up = p[2] - p[3];
        let head = basis(left.cross(up), left)?;
        let lip_width = p[6].distance(p[7]).max(0.001);
        let eye = |outer: Vec3, inner: Vec3, upper: Vec3, lower: Vec3, iris: Vec3| {
            let width = outer.distance(inner).max(0.001);
            let offset = head.inverse() * (iris - (outer + inner) * 0.5) / width;
            (upper.distance(lower) / width, [offset.y, offset.z])
        };
        let (left_open, left_gaze) = eye(p[0], p[8], p[9], p[10], p[11]);
        let (right_open, right_gaze) = eye(p[1], p[12], p[13], p[14], p[15]);
        let left_width = p[0].distance(p[8]);
        let right_width = p[1].distance(p[12]);
        Some(Self {
            head,
            balance: left_width.min(right_width) / left_width.max(right_width).max(0.001),
            jaw: p[4].distance(p[5]) / lip_width,
            openness: [left_open, right_open],
            gaze: [left_gaze, right_gaze],
        })
    }
}

pub(crate) struct Pose {
    body: [Option<Vec3>; 14],
    hands: [Option<[[f32; 3]; 21]>; 2],
    face: Option<(Face, Face)>,
    head: Option<Quat>,
    chest: Option<Quat>,
    pelvis: Option<Quat>,
    feet: [Option<super::FootContact>; 2],
    strength: f32,
    channels: Channels,
    recorded: Option<Vec<Option<Quat>>>,
}

impl Pose {
    pub(super) fn new(frame: &Frame, neutral: Option<&Frame>, strength: f32, channels: Channels) -> Self {
        let neutral_face = neutral.and_then(|f| Face::measure(f.face?));
        let neutral_torso = neutral.and_then(|f| torso(&body_points(f)));
        let neutral_head = neutral.and_then(|f| coarse_head(f.head?));
        Self::with_reference(
            frame,
            neutral_face,
            neutral_head,
            neutral_torso.map(|q| (q, neutral.map_or(0.0, |f| f.profile))),
            strength,
            channels,
        )
    }

    pub(super) fn still(frame: &Frame, channels: Channels) -> Self {
        // A photograph has no earlier frame to calibrate against.
        Self::with_reference(
            frame,
            Some(Face::REST),
            Some(Quat::IDENTITY),
            Some((Quat::IDENTITY, 0.0)),
            1.0,
            channels,
        )
    }

    fn with_reference(
        frame: &Frame,
        neutral_face: Option<Face>,
        neutral_head: Option<Quat>,
        neutral_torso: Option<(Quat, f32)>,
        strength: f32,
        channels: Channels,
    ) -> Self {
        let face = frame.face.and_then(Face::measure).zip(neutral_face);
        let body = body_points(frame);
        let torso_frame = torso(&body);
        let torso_orientation = torso_frame.zip(neutral_torso);
        let torso_delta = torso_orientation.map(|(current, (rest, profile))| profile_torso(current, rest, frame.profile.max(profile)));
        let local_delta = |current: Quat, rest: Quat| {
            if let Some((current_torso, (neutral_torso, _))) = torso_orientation {
                (current_torso.inverse() * current) * (neutral_torso.inverse() * rest).inverse()
            } else {
                current * rest.inverse()
            }
        };
        let coarse = frame
            .head
            .and_then(coarse_head)
            .zip(neutral_head)
            .map(|(head, rest)| local_delta(head, rest));
        let has_torso = torso_frame.is_some();
        let head = if let Some((mesh, rest)) = face {
            let reliable = ((mesh.balance - 0.35) / 0.55).clamp(0.0, 1.0);
            let motion = limited(local_delta(mesh.head, rest.head), 1.05);
            let weight = if has_torso { reliable * 0.75 } else { reliable };
            Some(coarse.map_or(Quat::IDENTITY, |q| limited(q, 1.05)).slerp(motion, weight))
        } else {
            coarse.map(|q| limited(q, 1.05)).or_else(|| has_torso.then_some(Quat::IDENTITY))
        };
        // A tracked torso bend first turns the pelvis. The chest takes only
        // the remaining motion, so the same rotation is not applied twice.
        let pelvis = torso_delta
            .filter(|_| channels.lower_body && body[6].is_some() && body[7].is_some())
            .map(|delta| limited(delta, 0.78));
        let chest = torso_delta.map(|delta| limited(pelvis.map_or(delta, |base| base.inverse() * delta), 0.48));
        Self {
            body,
            hands: [frame.left, frame.right],
            face,
            head,
            chest,
            pelvis,
            feet: frame.feet,
            strength,
            channels,
            recorded: None,
        }
    }

    pub(crate) fn feet(&self) -> [Option<super::FootContact>; 2] {
        if self.channels.lower_body {
            self.feet.map(|contact| {
                contact.map(|mut foot| {
                    foot.weight *= self.strength;
                    foot
                })
            })
        } else {
            [None; 2]
        }
    }

    pub(super) fn recorded(rotations: Vec<Option<Quat>>) -> Self {
        let mut pose = Self::still(&Frame::default(), Channels::default());
        pose.recorded = Some(rotations);
        pose
    }

    pub(crate) fn rotations(&self, rig: &Rig, animated: &[Quat]) -> Vec<Quat> {
        if let Some(recorded) = &self.recorded {
            return animated
                .iter()
                .enumerate()
                .map(|(index, &base)| recorded.get(index).copied().flatten().unwrap_or(base))
                .collect();
        }
        self.solve(rig, animated, &self.targets(rig))
    }

    pub(super) fn capture(&self, rig: &Rig, feet: &mut crate::scene::anim::WebcamFeet) -> Vec<Option<Quat>> {
        let targets = self.targets(rig);
        let mut rotations = self.solve(rig, &rig.local_rot, &targets);
        let contacts = self.feet();
        if contacts.iter().any(Option::is_some) {
            let world = rig.world_with(&rotations, Vec3::ZERO, &rig.local_pos, &rig.scale);
            feet.apply(rig, &mut rotations, &world, contacts);
        } else {
            *feet = Default::default();
        }
        rotations
            .into_iter()
            .zip(targets)
            .map(|(rotation, target)| target.map(|_| rotation))
            .collect()
    }

    pub(super) fn retrack_hand(&self, rig: &Rig, animated: &[Quat], side: usize) -> Vec<Option<Quat>> {
        let mut targets = vec![None; animated.len()];
        self.hand_targets(rig, &mut targets, side, SIDES[side]);
        self.solve(rig, animated, &targets)
            .into_iter()
            .zip(targets)
            .map(|(rotation, target)| target.map(|_| rotation))
            .collect()
    }

    fn targets(&self, rig: &Rig) -> Vec<Option<Target>> {
        let mut targets = vec![None; rig.local_rot.len()];
        if let Some(pelvis) = self.pelvis {
            set(rig, &mut targets, "mPelvis", Target::Local(pelvis));
        }
        if self.channels.upper_body
            && let Some(chest) = self.chest
        {
            set(rig, &mut targets, "mChest", Target::Local(chest));
        }
        if let Some((face, neutral)) = self.face {
            self.face_targets(rig, &mut targets, face, neutral);
        }
        if self.channels.head
            && let Some(head) = self.head
        {
            set(rig, &mut targets, "mHead", Target::Local(head));
        }
        for (i, side) in SIDES.iter().enumerate() {
            if self.channels.upper_body {
                self.arm_targets(rig, &mut targets, i, side);
            }
            if self.channels.lower_body {
                self.leg_targets(rig, &mut targets, i, side);
            }
            if self.channels.hands {
                self.hand_targets(rig, &mut targets, i, side);
            }
        }

        targets
    }

    fn solve(&self, rig: &Rig, animated: &[Quat], targets: &[Option<Target>]) -> Vec<Quat> {
        let mut output = animated.to_vec();
        let mut world = vec![Quat::IDENTITY; rig.local_rot.len()];
        for &joint in &rig.order {
            let parent = rig.parent[joint].map_or(Quat::IDENTITY, |p| world[p]);
            if let Some(target) = targets[joint] {
                let rest = rig.local_rot[joint];
                let local = match target {
                    Target::Local(delta) => rest * delta,
                    Target::Absolute(goal) => parent.inverse() * goal,
                    Target::Direction { from, to } => {
                        let from = rest * from;
                        let to = parent.inverse() * to;
                        Quat::from_rotation_arc(from, to) * rest
                    }
                };
                output[joint] = output[joint].slerp(local.normalize(), self.strength);
            }
            world[joint] = parent * output[joint];
        }
        output
    }

    fn face_targets(&self, rig: &Rig, targets: &mut [Option<Target>], face: Face, neutral: Face) {
        let detail = ((face.balance - 0.25) / 0.55).clamp(0.0, 1.0);
        let gain = self.channels.expression_strength * detail;
        let expression = self.channels.expression && gain > 0.0;
        if expression {
            let jaw = ((face.jaw - neutral.jaw - 0.015) * 1.55 * gain).clamp(0.0, 0.56);
            set(rig, targets, "mFaceJaw", Target::Local(Quat::from_rotation_y(jaw)));
        }
        for (i, side) in SIDES.iter().enumerate() {
            if self.channels.eyes {
                let yaw = (face.gaze[i][0] - neutral.gaze[i][0]).clamp(-0.23, 0.23) * detail;
                let pitch = (neutral.gaze[i][1] - face.gaze[i][1]).clamp(-0.15, 0.15) * detail;
                let eye = Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch);
                for joint in ["mEye", "mFaceEyeAlt"] {
                    set(rig, targets, &format!("{joint}{side}"), Target::Local(eye));
                }
            }
            if expression {
                let blink = ((1.0 - face.openness[i] / neutral.openness[i].max(0.1)) * gain).clamp(0.0, 1.0);
                for (joint, angle) in [("mFaceEyeLidUpper", blink * 0.48), ("mFaceEyeLidLower", -blink * 0.14)] {
                    set(rig, targets, &format!("{joint}{side}"), Target::Local(Quat::from_rotation_y(angle)));
                }
            }
        }
    }

    fn arm_targets(&self, rig: &Rig, targets: &mut [Option<Target>], i: usize, side: &str) {
        let [shoulder, elbow, wrist] = [self.body[i * 3], self.body[i * 3 + 1], self.body[i * 3 + 2]];
        limb_targets(rig, targets, side, [("Shoulder", shoulder), ("Elbow", elbow), ("Wrist", wrist)]);
    }

    fn leg_targets(&self, rig: &Rig, targets: &mut [Option<Target>], i: usize, side: &str) {
        let [hip, knee, ankle, toe] = [self.body[6 + i], self.body[8 + i], self.body[10 + i], self.body[12 + i]];
        limb_targets(rig, targets, side, [("Hip", hip), ("Knee", knee), ("Ankle", ankle)]);
        if let Some((ankle, toe)) = ankle.zip(toe)
            && let Some(to) = (toe - ankle).try_normalize()
            && let Some((&foot, &toe_joint)) = rig.names.get(&format!("mFoot{side}")).zip(rig.names.get(&format!("mToe{side}")))
            && let Some(from) = (rig.local_pos[foot] + rig.local_rot[foot] * rig.local_pos[toe_joint]).try_normalize()
        {
            set(rig, targets, &format!("mAnkle{side}"), Target::Direction { from, to });
        }
    }

    fn hand_targets(&self, rig: &Rig, targets: &mut [Option<Target>], i: usize, side: &str) {
        let Some(hand) = self.hands[i] else { return };
        let hand = hand.map(Vec3::from_array);
        let index = format!("mHandIndex1{side}");
        let pinky = format!("mHandPinky1{side}");
        if let Some((&index, &pinky)) = rig.names.get(&index).zip(rig.names.get(&pinky)) {
            let index = rig.local_pos[index];
            let pinky = rig.local_pos[pinky];
            let across = hand[5] - hand[17];
            let forward = (hand[5] + hand[17]) * 0.5 - hand[0];
            // A collapsed palm plane gives arbitrary twist about the
            // forearm. Leave the wrist to its parent until it opens again.
            if across.cross(forward).length_squared() > 0.04 * across.length_squared() * forward.length_squared()
                && let Some((tracked, rest)) = basis(across, forward).zip(basis(index - pinky, (index + pinky) * 0.5))
            {
                set(rig, targets, &format!("mWrist{side}"), Target::Absolute(tracked * rest.inverse()));
            }
        }
        for (finger, start) in FINGERS {
            let mut previous = hand[start] - hand[0];
            for joint in 0..3 {
                let link = hand[start + joint + 1] - hand[start + joint];
                if let Some((a, b)) = previous.try_normalize().zip(link.try_normalize()) {
                    let curl = (a.angle_between(b) - 0.16).clamp(0.0, 1.55);
                    let sign = if i == 0 { -1.0 } else { 1.0 };
                    let axis = if finger == "Thumb" { Vec3::Z * -sign } else { Vec3::X * sign };
                    let delta = Quat::from_axis_angle(axis, curl);
                    set(rig, targets, &format!("mHand{finger}{}{side}", joint + 1), Target::Local(delta));
                }
                previous = link;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    Local(Quat),
    Absolute(Quat),
    Direction { from: Vec3, to: Vec3 },
}

fn set(rig: &Rig, targets: &mut [Option<Target>], joint: &str, target: Target) {
    if let Some(&joint) = rig.names.get(joint) {
        targets[joint] = Some(target);
    }
}

fn limb_targets(rig: &Rig, targets: &mut [Option<Target>], side: &str, joints: [(&str, Option<Vec3>); 3]) {
    for pair in joints.windows(2) {
        let [(joint, start), (child, end)] = pair else { continue };
        if let Some((start, end)) = start.zip(*end)
            && let Some(to) = (end - start).try_normalize()
            && let Some(&child) = rig.names.get(&format!("m{child}{side}"))
            && let Some(from) = rig.local_pos[child].try_normalize()
        {
            set(rig, targets, &format!("m{joint}{side}"), Target::Direction { from, to });
        }
    }
}

fn body_points(frame: &Frame) -> [Option<Vec3>; 14] {
    frame.body.map(|point| point.map(Vec3::from_array))
}

fn coarse_head(points: [[f32; 3]; 3]) -> Option<Quat> {
    let [nose, left, right] = points.map(Vec3::from_array);
    let across = left - right;
    if across.length_squared() < 0.0025 {
        return None;
    }
    let across = across.try_normalize()?;
    let forward = nose - (left + right) * 0.5;
    let forward = forward - across * forward.dot(across);
    if forward.length_squared() < 0.0004 {
        return None;
    }
    basis(forward, across)
}

fn torso(body: &[Option<Vec3>; 14]) -> Option<Quat> {
    let left = body[0]?;
    let right = body[3]?;
    let up = body[6].zip(body[7]).map_or(Vec3::Z, |(a, b)| (left + right - a - b) * 0.5);
    let across = left - right;
    basis(across.cross(up), across)
}

fn profile_torso(current: Quat, rest: Quat, profile: f32) -> Quat {
    let delta = current * rest.inverse();
    if profile <= 0.0 {
        return delta;
    }
    // The shoulder axis becomes foreshortened in profile. Isolate the lean
    // visible in the image from yaw, so a change in depth cannot tip the rig.
    let up = |q: Quat| {
        let v = q * Vec3::Z;
        (-v.y).atan2(v.z)
    };
    let heading = delta * Vec3::X;
    let yaw = heading.y.atan2(heading.x).clamp(-1.05, 1.05);
    let lean = (up(current) - up(rest)).clamp(-0.60, 0.60);
    delta.slerp(Quat::from_rotation_x(lean) * Quat::from_rotation_z(yaw), profile.clamp(0.0, 1.0))
}

fn basis(x: Vec3, y: Vec3) -> Option<Quat> {
    let x = x.try_normalize()?;
    let z = x.cross(y).try_normalize()?;
    let y = z.cross(x);
    Some(Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize())
}

fn limited(rotation: Quat, limit: f32) -> Quat {
    let rotation = if rotation.w < 0.0 { -rotation } else { rotation };
    let (axis, angle) = rotation.to_axis_angle();
    Quat::from_axis_angle(axis, angle.min(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_follows_chest_and_profile_points_can_add_local_turn() {
        let xml = include_str!("../../assets/character/avatar_skeleton.xml");
        let rig = Rig::new(&aurora_assets::Skeleton::parse(xml.as_bytes()).unwrap());
        let mut rest = Frame::default();
        rest.body[0] = Some([0.0, 0.2, 0.5]);
        rest.body[3] = Some([0.0, -0.2, 0.5]);
        rest.body[6] = Some([0.0, 0.15, 0.0]);
        rest.body[7] = Some([0.0, -0.15, 0.0]);
        rest.head = Some([[0.08, 0.0, 0.7], [0.0, 0.08, 0.7], [0.0, -0.08, 0.7]]);
        let turn = Quat::from_rotation_z(0.50);
        let face_turn = Quat::from_rotation_z(-0.20);
        let mut frame = Frame::default();
        frame.body = rest.body.map(|p| p.map(|p| (turn * Vec3::from_array(p)).to_array()));
        let head = rig.names["mHead"];
        let without_face = Pose::new(&frame, Some(&rest), 1.0, Channels::default()).rotations(&rig, &rig.local_rot);
        assert!(without_face[head].abs_diff_eq(rig.local_rot[head], 1e-5));
        frame.head = rest.head.map(|p| p.map(|p| (turn * face_turn * Vec3::from_array(p)).to_array()));
        let profile = Pose::new(&frame, Some(&rest), 1.0, Channels::default()).rotations(&rig, &rig.local_rot);
        assert!(profile[head].abs_diff_eq(rig.local_rot[head] * face_turn, 1e-4));
    }

    #[test]
    fn tracked_torso_turns_pelvis_without_doubling_the_chest_rotation() {
        let xml = include_str!("../../assets/character/avatar_skeleton.xml");
        let rig = Rig::new(&aurora_assets::Skeleton::parse(xml.as_bytes()).unwrap());
        let mut rest = Frame::default();
        rest.body[0] = Some([0.0, 0.2, 0.5]);
        rest.body[3] = Some([0.0, -0.2, 0.5]);
        rest.body[6] = Some([0.0, 0.15, 0.0]);
        rest.body[7] = Some([0.0, -0.15, 0.0]);
        let turn = Quat::from_rotation_x(0.35);
        let mut frame = Frame::default();
        frame.body = rest.body.map(|p| p.map(|p| (turn * Vec3::from_array(p)).to_array()));
        let pose = Pose::new(&frame, Some(&rest), 1.0, Channels::default());
        let rotations = pose.rotations(&rig, &rig.local_rot);
        let root = rig.names["mPelvis"];
        let chest = rig.names["mChest"];
        assert!(rotations[root].abs_diff_eq(rig.local_rot[root] * turn, 1e-4));
        assert!(rotations[chest].abs_diff_eq(rig.local_rot[chest], 1e-4));
    }

    #[test]
    fn basis_is_right_handed_and_rejects_collinear_input() {
        assert!(basis(Vec3::X, Vec3::Y).unwrap().abs_diff_eq(Quat::IDENTITY, 1e-5));
        assert!(basis(Vec3::X, Vec3::X).is_none());
        let turned = basis(Vec3::Y, -Vec3::X).unwrap();
        assert!((turned * Vec3::X).abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn rotation_limit_takes_the_shortest_arc() {
        let turn = -Quat::from_rotation_z(0.8);
        assert!(limited(turn, 0.3).abs_diff_eq(Quat::from_rotation_z(0.3), 1e-5));
    }

    #[test]
    fn tracked_arms_use_parent_space_and_leave_legs_animated() {
        let xml = include_str!("../../assets/character/avatar_skeleton.xml");
        let rig = Rig::new(&aurora_assets::Skeleton::parse(xml.as_bytes()).unwrap());
        let mut frame = Frame::default();
        frame.body[0] = Some([0.0, 0.2, 0.0]);
        frame.body[1] = Some([0.0, 0.3, -0.2]);
        frame.body[2] = Some([0.1, 0.3, 0.1]);
        let mut animated = rig.local_rot.clone();
        let collar = rig.names["mCollarLeft"];
        let knee = rig.names["mKneeLeft"];
        animated[collar] = Quat::from_rotation_z(0.2);
        animated[knee] = Quat::from_rotation_y(0.4);
        let shoulder = rig.names["mShoulderLeft"];
        let elbow = rig.names["mElbowLeft"];
        let wrist = rig.names["mWristLeft"];
        let pose = Pose::new(&frame, None, 1.0, Channels::default()).rotations(&rig, &animated);
        let upper = pose[collar] * pose[shoulder];
        let lower = upper * pose[elbow];
        let up_dir = Vec3::new(0.0, 0.1, -0.2).normalize();
        let low_dir = Vec3::new(0.1, 0.0, 0.3).normalize();
        assert!((upper * rig.local_pos[elbow].normalize()).abs_diff_eq(up_dir, 1e-4));
        assert!((lower * rig.local_pos[wrist].normalize()).abs_diff_eq(low_dir, 1e-4));
        assert_eq!(pose[knee], animated[knee]);
        let released = Pose::new(&frame, None, 0.0, Channels::default()).rotations(&rig, &animated);
        assert!(released[shoulder].abs_diff_eq(animated[shoulder], 1e-5));
        let disabled = Channels {
            upper_body: false,
            ..Channels::default()
        };
        let disabled = Pose::new(&frame, None, 1.0, disabled).rotations(&rig, &animated);
        assert_eq!(disabled[shoulder], animated[shoulder]);
    }

    #[test]
    fn visible_leg_drives_avatar_segments_and_disabled_channel_leaves_animation() {
        let xml = include_str!("../../assets/character/avatar_skeleton.xml");
        let rig = Rig::new(&aurora_assets::Skeleton::parse(xml.as_bytes()).unwrap());
        let mut frame = Frame::default();
        frame.body[6] = Some([0.0, 0.1, 0.0]);
        frame.body[8] = Some([0.3, 0.1, -0.2]);
        frame.body[10] = Some([0.0, 0.2, -0.4]);
        frame.body[12] = Some([0.1, 0.2, -0.5]);

        let animated = rig.local_rot.clone();
        let output = Pose::new(&frame, None, 1.0, Channels::default()).rotations(&rig, &animated);
        let mut world = vec![Quat::IDENTITY; output.len()];
        for &joint in &rig.order {
            world[joint] = rig.parent[joint].map_or(Quat::IDENTITY, |p| world[p]) * output[joint];
        }
        let hip = rig.names["mHipLeft"];
        let knee = rig.names["mKneeLeft"];
        let ankle = rig.names["mAnkleLeft"];
        let foot = rig.names["mFootLeft"];
        let toe = rig.names["mToeLeft"];
        let foot_axis = rig.local_pos[foot] + rig.local_rot[foot] * rig.local_pos[toe];
        assert!((world[hip] * rig.local_pos[knee].normalize()).abs_diff_eq(Vec3::new(0.3, 0.0, -0.2).normalize(), 1e-4));
        assert!((world[knee] * rig.local_pos[ankle].normalize()).abs_diff_eq(Vec3::new(-0.3, 0.1, -0.2).normalize(), 1e-4));
        assert!((world[ankle] * foot_axis.normalize()).abs_diff_eq(Vec3::new(0.1, 0.0, -0.1).normalize(), 1e-4));
        assert_eq!(output[rig.names["mHipRight"]], animated[rig.names["mHipRight"]]);

        let disabled = Channels {
            lower_body: false,
            ..Channels::default()
        };
        assert_eq!(Pose::new(&frame, None, 1.0, disabled).rotations(&rig, &animated), animated);
    }
}
