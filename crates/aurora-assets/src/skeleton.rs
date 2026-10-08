//! Avatar skeleton definition (`character/avatar_skeleton.xml`).
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/llappearance/llavatarappearance.cpp` (`LLAvatarBoneInfo::parseXml`,
//!   `LLAvatarSkeletonInfo::parseXml`, `LLAvatarAppearance::setupBone`)
//! - `indra/llcharacter/lljoint.cpp`, `indra/llmath/xform.cpp`
//!   (`LLXformMatrix::update` / `updateMatrix`)
//! - `indra/llmath/llquaternion.cpp` (`mayaQ`)
//!
//! Copyright (C) 2001-2024, Linden Research, Inc. and the Firestorm project.
//!
//! World transforms follow `LLXformMatrix` with `scale_child_offset = true`
//! (set for every `LLJoint`):
//! ```text
//! world_pos = parent.world_pos + parent.world_rot * (pos * parent.scale)
//! world_rot = parent.world_rot * local_rot
//! world     = T(world_pos) * R(world_rot) * S(scale)   // own scale only, never inherited
//! ```
//! `local_rot` is `mayaQ(x, y, z, XYZ)`: rotate about X, then Y, then Z
//! (LL's quaternion product is reversed relative to Hamilton order; the
//! quaternions here are standard glam ones).

use aurora_llsd::xml::{self, Element};
use glam::{Mat4, Quat, Vec3};

use crate::AssetError;

/// One bone or collision volume.
#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub pos: Vec3,
    pub rot_euler_deg: Vec3,
    pub scale: Vec3,
    /// Skin offset (`pivot`); zero for collision volumes.
    pub pivot: Vec3,
    pub aliases: Vec<String>,
    pub is_collision_volume: bool,
    /// Default-pose transform relative to the skeleton root's parent frame.
    pub world: Mat4,
    /// Bone end point (`end` attribute, local space).
    pub end: Vec3,
    /// `connected="true"`.
    pub connected: bool,
    /// `support` attribute ("base" or "extended").
    pub support: String,
    /// `group` attribute.
    pub group: String,
}

impl Joint {
    pub fn world_position(&self) -> Vec3 {
        self.world.w_axis.truncate()
    }
}

/// Parsed skeleton; joints are stored in document (pre-order) order, so a
/// parent always precedes its children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

/// `mayaQ(x, y, z, LLQuaternion::XYZ)` with angles in degrees.
pub fn maya_q_xyz(deg: Vec3) -> Quat {
    let qx = Quat::from_rotation_x(deg.x.to_radians());
    let qy = Quat::from_rotation_y(deg.y.to_radians());
    let qz = Quat::from_rotation_z(deg.z.to_radians());
    qz * qy * qx
}

fn parse_vec3(el: &Element, attr: &str) -> Result<Option<Vec3>, AssetError> {
    let Some(s) = el.attr(attr) else {
        return Ok(None);
    };
    let mut it = s.split_whitespace().map(str::parse::<f32>);
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(x)), Some(Ok(y)), Some(Ok(z))) => Ok(Some(Vec3::new(x, y, z))),
        _ => Err(AssetError::invalid(format!(
            "skeleton: bad {attr} \"{s}\" on {}",
            el.attr("name").unwrap_or("?")
        ))),
    }
}

fn required_vec3(el: &Element, attr: &str) -> Result<Vec3, AssetError> {
    parse_vec3(el, attr)?.ok_or_else(|| AssetError::invalid(format!("skeleton: {} missing {attr}", el.attr("name").unwrap_or("?"))))
}

struct WorldState {
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
}

impl Skeleton {
    /// Parse `avatar_skeleton.xml`.
    pub fn parse(xml_data: &[u8]) -> Result<Skeleton, AssetError> {
        let root = xml::parse(xml_data)?;
        if root.name != "linden_skeleton" {
            return Err(AssetError::invalid("not a linden_skeleton document"));
        }
        let mut sk = Skeleton::default();
        let mut state = Vec::new();
        for el in root.elements() {
            sk.walk(el, None, &mut state)?;
        }
        if sk.joints.is_empty() {
            return Err(AssetError::invalid("skeleton has no bones"));
        }
        Ok(sk)
    }

    fn walk(&mut self, el: &Element, parent: Option<usize>, state: &mut Vec<WorldState>) -> Result<(), AssetError> {
        let is_collision_volume = match el.name.as_str() {
            "bone" => false,
            "collision_volume" => true,
            _ => return Ok(()),
        };
        let name = el
            .attr("name")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| AssetError::invalid("skeleton: joint without name"))?
            .to_owned();
        let pos = required_vec3(el, "pos")?;
        let rot_euler_deg = required_vec3(el, "rot")?;
        let scale = required_vec3(el, "scale")?;
        let pivot = if is_collision_volume {
            Vec3::ZERO
        } else {
            required_vec3(el, "pivot")?
        };
        let end = parse_vec3(el, "end")?.unwrap_or(Vec3::ZERO);
        let aliases = el
            .attr("aliases")
            .map(|a| a.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();

        let local_rot = maya_q_xyz(rot_euler_deg);
        let (world_pos, world_rot) = match parent.and_then(|p| state.get(p)) {
            Some(ps) => (ps.pos + ps.rot * (pos * ps.scale), (ps.rot * local_rot).normalize()),
            None => (pos, local_rot),
        };
        let index = self.joints.len();
        state.push(WorldState {
            pos: world_pos,
            rot: world_rot,
            scale,
        });
        self.joints.push(Joint {
            name,
            parent,
            pos,
            rot_euler_deg,
            scale,
            pivot,
            aliases,
            is_collision_volume,
            world: Mat4::from_scale_rotation_translation(scale, world_rot, world_pos),
            end,
            connected: el.attr("connected").is_some_and(|c| c.eq_ignore_ascii_case("true")),
            support: el.attr("support").unwrap_or("base").to_owned(),
            group: el.attr("group").unwrap_or_default().to_owned(),
        });
        for child in el.elements() {
            self.walk(child, Some(index), state)?;
        }
        Ok(())
    }

    /// Find a joint by name or alias.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.joints
            .iter()
            .position(|j| j.name == name)
            .or_else(|| self.joints.iter().position(|j| j.aliases.iter().any(|a| a == name)))
    }

    /// Number of bones (excluding collision volumes).
    pub fn bone_count(&self) -> usize {
        self.joints.iter().filter(|j| !j.is_collision_volume).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKELETON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aurora-viewer/assets/character/avatar_skeleton.xml");

    #[test]
    fn small_skeleton() {
        let xml = br#"<linden_skeleton num_bones="2" num_collision_volumes="1" version="2.0">
          <bone name="mPelvis" aliases="hip avatar_mPelvis" pos="0 0 1" rot="0 0 90" scale="2 2 2" pivot="0 0 1" end="0 0 0.1" connected="false" support="base">
            <collision_volume name="PELVIS" pos="1 0 0" rot="0 0 0" scale="0.1 0.2 0.3"/>
            <bone name="mTorso" pos="1 0 0" rot="0 0 0" scale="1 1 1" pivot="0 0 0" connected="true" support="extended"/>
          </bone>
        </linden_skeleton>"#;
        let sk = Skeleton::parse(xml).unwrap();
        assert_eq!(sk.joints.len(), 3);
        assert_eq!(sk.find("hip"), Some(0));
        assert_eq!(sk.find("mTorso"), Some(2));
        assert_eq!(sk.find("nope"), None);
        let torso = &sk.joints[2];
        assert_eq!(torso.parent, Some(0));
        assert!(torso.connected);
        assert_eq!(torso.support, "extended");
        // pos (1,0,0) scaled by parent scale 2 then rotated 90deg about Z => (0,2,0) + (0,0,1)
        let p = torso.world_position();
        assert!((p - Vec3::new(0.0, 2.0, 1.0)).length() < 1e-5, "{p}");
        // Scale is not inherited: torso world has unit scale.
        let (s, _, _) = torso.world.to_scale_rotation_translation();
        assert!((s - Vec3::ONE).length() < 1e-5);
        let cv = &sk.joints[1];
        assert!(cv.is_collision_volume);
        let (s, _, _) = cv.world.to_scale_rotation_translation();
        assert!((s - Vec3::new(0.1, 0.2, 0.3)).length() < 1e-5);
    }

    #[test]
    fn maya_order() {
        // X then Y: rotate +Y by 90 about X -> +Z, then 90 about Y -> +X.
        let q = maya_q_xyz(Vec3::new(90.0, 90.0, 0.0));
        let v = q * Vec3::Y;
        assert!((v - Vec3::X).length() < 1e-5, "{v}");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Skeleton::parse(b"").is_err());
        assert!(Skeleton::parse(b"<foo/>").is_err());
        assert!(
            Skeleton::parse(
                b"<linden_skeleton><bone name=\"a\" pos=\"1 2\" rot=\"0 0 0\" scale=\"1 1 1\" pivot=\"0 0 0\"/></linden_skeleton>"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_real_skeleton() {
        let Ok(data) = std::fs::read(SKELETON) else {
            eprintln!("skipping: {SKELETON} not found");
            return;
        };
        let sk = Skeleton::parse(&data).unwrap();
        assert_eq!(sk.bone_count(), 133);
        assert_eq!(sk.joints.len() - sk.bone_count(), 26);
        assert_eq!(sk.joints[0].name, "mPelvis");
        assert_eq!(sk.find("avatar_mPelvis"), Some(0));
        let head = &sk.joints[sk.find("mHead").unwrap()];
        let hp = head.world_position();
        assert!(hp.z > 1.6 && hp.z < 2.0, "{hp}");
        for (i, j) in sk.joints.iter().enumerate() {
            if let Some(p) = j.parent {
                assert!(p < i);
                assert!(!sk.joints[p].is_collision_volume);
            }
        }
    }
}
