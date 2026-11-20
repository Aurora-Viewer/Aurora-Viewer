//! Named local pose format. No subject identity, names, region or world coordinates.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum LoadMode {
    #[default]
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
    /// `rotation` is a change to apply on the destination's own origin, like
    /// the Legacy Viewer's diff; a complete rotation otherwise. Absent in
    /// complete joints and in the files saved before it existed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    change: bool,
}
impl Joint {
    pub fn new(name: String, rotation: Quat, translation: Vec3, scale: Vec3, locked: bool, change: bool) -> Self {
        Self {
            name,
            rotation: rotation.to_array(),
            translation: translation.to_array(),
            scale: scale.to_array(),
            locked,
            change,
        }
    }
}
#[cfg(test)]
impl Joint {
    pub fn rotation(&self) -> Quat {
        Quat::from_array(self.rotation)
    }
    pub fn translation(&self) -> Vec3 {
        Vec3::from_array(self.translation)
    }
    pub fn locked(&self) -> bool {
        self.locked
    }
    pub fn change(&self) -> bool {
        self.change
    }
}
/// What a document's translations and scales are measured from. Aurora's
/// own: a translation added to and a scale multiplied by the destination's
/// starting pose. The Legacy Viewer's pose files (`legacy_pose`) add their
/// scale instead (FSJointState::getTargetScale, `mScale + mBaseScale`), and
/// before their version 4 give the joint's whole position and scale
/// (FSPoserAnimator::loadJointPosition and loadJointScale without
/// `loadPositionAsDelta`, indra/newview/fsposeranimator.cpp), the pelvis's
/// excepted: its whole position is measured from the Legacy Viewer's rest at
/// zero (LLAvatarAppearance::buildSkeleton), so it is added like a change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Channels {
    #[default]
    Aurora,
    LegacyDelta,
    LegacyAbsolute,
}
impl Channels {
    fn is_aurora(&self) -> bool {
        *self == Self::Aurora
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    version: u8,
    pub name: String,
    pub priority: i32,
    pub joints: Vec<Joint>,
    /// Absent in Aurora's own files.
    #[serde(default, skip_serializing_if = "Channels::is_aurora")]
    pub channels: Channels,
}
impl Document {
    pub fn new(name: String, priority: i32, joints: Vec<Joint>, channels: Channels) -> Self {
        Self {
            version: 1,
            name,
            priority,
            joints,
            channels,
        }
    }
    pub fn capture(rig: &Rig, session: &data::Session, name: String, priority: i32) -> Self {
        Self {
            version: 1,
            name,
            priority,
            joints: rig
                .canonical_names
                .iter()
                .enumerate()
                .map(|(bone, name)| {
                    // FSFloaterPoser::savePoseToXml: only what was authored
                    let origin = session.pose.origin[bone];
                    let rotation = session.pose.rotations[bone];
                    let complete = origin.dot(rig.local_rot[bone]).abs() > 0.9999999;
                    Joint {
                        name: name.clone(),
                        rotation: if complete {
                            rotation
                        } else {
                            (origin.inverse() * rotation).normalize()
                        }
                        .to_array(),
                        translation: (session.pose.positions[bone] - session.base.positions[bone]).to_array(),
                        scale: (session.pose.scales[bone] / session.base.scales[bone]).to_array(),
                        locked: session.pose.locks.contains_key(&bone),
                        change: !complete,
                    }
                })
                .collect(),
            channels: Channels::Aurora,
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
                    && match self.channels {
                        // an added scale is zero where nothing changed
                        Channels::LegacyDelta => s.is_finite() && s.abs().max_element() <= 100.0,
                        Channels::Aurora | Channels::LegacyAbsolute => data::scale_valid(s),
                    }
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
            let rotation = Quat::from_array(joint.rotation).normalize();
            if joint.change {
                pose.rotations[bone] = (pose.origin[bone] * rotation).normalize();
            } else {
                pose.rotations[bone] = rotation;
                pose.origin[bone] = rig.local_rot[bone];
            }
            if !rotations_only {
                let (translation, scale) = (Vec3::from_array(joint.translation), Vec3::from_array(joint.scale));
                let base = (session.base.positions[bone], session.base.scales[bone]);
                let (position, scale) = match self.channels {
                    Channels::Aurora => (base.0 + translation, base.1 * scale),
                    Channels::LegacyDelta => (base.0 + translation, base.1 + scale),
                    Channels::LegacyAbsolute if bone == rig.pelvis => (base.0 + translation, scale),
                    Channels::LegacyAbsolute => (translation, scale),
                };
                pose.positions[bone] = position;
                // a scale the poser cannot keep leaves the joint's own, not
                // the whole pose refused
                pose.scales[bone] = if data::scale_valid(scale) { scale } else { base.1 };
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
