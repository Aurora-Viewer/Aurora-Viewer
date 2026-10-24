//! Four limb effectors, independent bend targets and rest-frame symmetry.
use super::{Frame, Goal, Rig};
use glam::{Quat, Vec3};

#[derive(Clone, Copy)]
pub(super) struct Limb {
    pub end: usize,
    pub middle: usize,
    pub root: usize,
    pub label: &'static str,
    pub pole_label: &'static str,
    pub leg: bool,
    pub right: bool,
}

pub(super) fn limbs(rig: &Rig) -> Vec<Limb> {
    [
        (
            "mWristLeft",
            "mElbowLeft",
            "mShoulderLeft",
            "ik-edit-hand-left",
            "ik-edit-elbow-left",
            false,
            false,
        ),
        (
            "mWristRight",
            "mElbowRight",
            "mShoulderRight",
            "ik-edit-hand-right",
            "ik-edit-elbow-right",
            false,
            true,
        ),
        (
            "mAnkleLeft",
            "mKneeLeft",
            "mHipLeft",
            "ik-edit-foot-left",
            "ik-edit-knee-left",
            true,
            false,
        ),
        (
            "mAnkleRight",
            "mKneeRight",
            "mHipRight",
            "ik-edit-foot-right",
            "ik-edit-knee-right",
            true,
            true,
        ),
    ]
    .into_iter()
    .filter_map(|(end, middle, root, label, pole_label, leg, right)| {
        let limb = Limb {
            end: *rig.names.get(end)?,
            middle: *rig.names.get(middle)?,
            root: *rig.names.get(root)?,
            label,
            pole_label,
            leg,
            right,
        };
        (rig.parent[limb.end] == Some(limb.middle) && rig.parent[limb.middle] == Some(limb.root)).then_some(limb)
    })
    .collect()
}

impl Limb {
    /// Keep the current bend plane when first grabbing a pole; a straight
    /// limb gets a stable outward/downward arm or forward knee direction.
    pub fn pole(self, frame: &Frame) -> Vec3 {
        let inverse = frame.model.inverse();
        let [root, middle, end] = [self.root, self.middle, self.end].map(|j| inverse.transform_point3(frame.joints[j].w_axis.truncate()));
        let axis = (end - root).normalize_or_zero();
        let default = if self.leg {
            Vec3::X
        } else {
            Vec3::new(0.0, if self.right { -1.0 } else { 1.0 }, -0.5)
        };
        let projected = |v: Vec3| {
            let v = v - axis * axis.dot(v);
            // Sub-millimetre noise in a straight limb must not pick a random plane.
            (v.length_squared() > 0.001 * 0.001).then(|| v.normalize())
        };
        let bend = projected(middle - root).or_else(|| projected(default)).unwrap_or(Vec3::Z);
        middle + bend * root.distance(end).max(0.2) * 0.65
    }
}

pub(super) fn reflect(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.y, v.z)
}

fn reflect_rotation(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, q.y, -q.z, q.w)
}

#[derive(Default)]
pub(super) struct Mirrors {
    pairs: Vec<Option<usize>>,
    rest: Vec<Quat>,
}

impl Mirrors {
    pub fn initialize(&mut self, rig: &Rig) {
        if self.pairs.len() == rig.len() {
            return;
        }
        self.pairs = rig
            .canonical_names
            .iter()
            .map(|name| {
                let opposite = if let Some(stem) = name.strip_suffix("Left") {
                    format!("{stem}Right")
                } else if let Some(stem) = name.strip_suffix("Right") {
                    format!("{stem}Left")
                } else if let Some(stem) = name.strip_prefix("L_") {
                    format!("R_{stem}")
                } else if let Some(stem) = name.strip_prefix("LEFT_") {
                    format!("RIGHT_{stem}")
                } else if let Some(stem) = name.strip_prefix("RIGHT_") {
                    format!("LEFT_{stem}")
                } else {
                    let stem = name.strip_prefix("R_")?;
                    format!("L_{stem}")
                };
                rig.names.get(&opposite).copied()
            })
            .collect();
        self.rest = rig
            .default_world_inv
            .iter()
            .map(|m| m.inverse().to_scale_rotation_translation().1.normalize())
            .collect();
    }

    pub fn counterpart(&self, bone: usize) -> Option<usize> {
        self.pairs.get(bone).copied().flatten()
    }

    pub fn goal(&self, goal: &Goal) -> Option<Goal> {
        let bone = self.counterpart(goal.bone)?;
        let from = self.rest[goal.bone];
        let to = self.rest[bone];
        Some(Goal {
            follow: None,
            weight: goal.weight,
            bone,
            root: self.counterpart(goal.root).unwrap_or(goal.root),
            position: reflect(goal.position),
            rotation: goal.rotation.map(|q| (reflect_rotation(q * from.inverse()) * to).normalize()),
            contact: to.inverse() * reflect(from * goal.contact),
            pole: goal.pole.map(reflect),
            mode: goal.mode,
        })
    }
}

#[cfg(test)]
mod tests;
