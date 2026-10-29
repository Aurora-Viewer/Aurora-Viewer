//! Named local pose format. No subject identity, names, region or world coordinates.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LoadMode {
    All,
    Rotations,
    Selective,
    SelectiveRotations,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Joint {
    pub name: String,
    rotation: [f32; 4],
    translation: [f32; 3],
    scale: [f32; 3],
    locked: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    version: u8,
    pub name: String,
    pub priority: i32,
    pub joints: Vec<Joint>,
}
impl Document {
    pub fn capture(rig: &Rig, session: &data::Session, name: String, priority: i32) -> Self {
        Self {
            version: 1,
            name,
            priority,
            joints: rig
                .canonical_names
                .iter()
                .enumerate()
                .map(|(bone, name)| Joint {
                    name: name.clone(),
                    rotation: session.pose.rotations[bone].to_array(),
                    translation: (session.pose.positions[bone] - session.base.positions[bone]).to_array(),
                    scale: (session.pose.scales[bone] / session.base.scales[bone]).to_array(),
                    locked: session.pose.locks.contains_key(&bone),
                })
                .collect(),
        }
    }
    pub fn valid(&self, rig: &Rig) -> bool {
        let mut seen = HashSet::new();
        self.version == 1
            && !self.name.trim().is_empty()
            && self.name.len() <= 400
            && !self.name.chars().any(char::is_control)
            && (0..=6).contains(&self.priority)
            && !self.joints.is_empty()
            && self.joints.len() <= rig.len()
            && self.joints.iter().all(|j| {
                let q = Quat::from_array(j.rotation);
                let p = Vec3::from_array(j.translation);
                let s = Vec3::from_array(j.scale);
                rig.names.contains_key(&j.name)
                    && seen.insert(&j.name)
                    && q.is_finite()
                    && (0.99..=1.01).contains(&q.length_squared())
                    && p.is_finite()
                    && p.abs().max_element() <= 100.0
                    && data::scale_valid(s)
            })
    }
    pub fn apply(&self, rig: &Rig, session: &mut data::Session, mode: LoadMode, model: Mat4) -> bool {
        if !self.valid(rig) {
            return false;
        }
        let selective = matches!(mode, LoadMode::Selective | LoadMode::SelectiveRotations);
        let rotations_only = matches!(mode, LoadMode::Rotations | LoadMode::SelectiveRotations);
        let mut pose = (*session.pose).clone();
        let mut lock = Vec::new();
        for joint in &self.joints {
            let Some(&bone) = rig.names.get(&joint.name) else { continue };
            if selective && session.edited(bone) {
                continue;
            }
            pose.rotations[bone] = Quat::from_array(joint.rotation).normalize();
            if !rotations_only {
                pose.positions[bone] = session.base.positions[bone] + Vec3::from_array(joint.translation);
                pose.scales[bone] = session.base.scales[bone] * Vec3::from_array(joint.scale);
            }
            pose.locks.remove(&bone);
            if joint.locked {
                lock.push(bone);
            }
        }
        if !pose.valid(rig) {
            return false;
        }
        pose.keep_offsets(rig, &session.base);
        let world = pose.matrices(rig, model);
        for bone in lock {
            pose.locks.insert(bone, model * world[bone]);
        }
        session.checkpoint();
        session.pose = Arc::new(pose);
        true
    }
}
