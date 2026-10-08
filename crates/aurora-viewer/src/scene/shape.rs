//! Avatar shape: the visual parameters of `avatar_lad.xml` applied to the
//! skeleton, ported from LLVisualParam / LLDriverParam /
//! LLPolySkeletalDistortion and LLAvatarAppearance::computeBodySize
//! (Second Life viewer, originally LGPL 2.1).
//!
//! The simulator sends the weights of the transmitted parameters (groups 0
//! and 3, by increasing id) as bytes. Driver parameters (height, "male",
//! face sliders...) set the weights of the parameters they drive; skeletal
//! parameters then scale and move bones: `scale = default + Σ w · Δscale`,
//! `position = default + Σ w · Δoffset`. Parameters made for the other sex
//! keep their default weight. Body morphs move and scale the collision
//! volumes (volume morphs); their vertex deformations of the system body
//! mesh are not applied yet.

use super::anim::Rig;
use aurora_assets::Skeleton;
use aurora_llsd::xml;
use glam::Vec3;
use std::collections::{BTreeMap, HashMap};

const SEX_FEMALE: u8 = 1;
const SEX_MALE: u8 = 2;
const SEX_BOTH: u8 = 3;
/// "male" driver parameter.
const PARAM_MALE: i32 = 80;
/// "Hover" (raises the whole avatar).
const PARAM_HOVER: i32 = 11001;

#[derive(Debug, Clone)]
struct Driven {
    id: i32,
    min1: f32,
    max1: f32,
    max2: f32,
    min2: f32,
}

#[derive(Debug, Clone)]
struct BoneDelta {
    joint: usize,
    scale: Vec3,
    offset: Option<Vec3>,
}

#[derive(Debug, Clone)]
enum Kind {
    Skeleton(Vec<BoneDelta>),
    Driver(Vec<Driven>),
    Other,
}

#[derive(Debug, Clone)]
struct Param {
    group: u8,
    min: f32,
    max: f32,
    default: f32,
    sex: u8,
    kind: Kind,
}

/// The parameter definitions of `avatar_lad.xml`.
#[derive(Debug, Default)]
pub struct ShapeParams {
    params: BTreeMap<i32, Param>,
    /// Ids of the parameters sent in AvatarAppearance, in message order.
    transmitted: Vec<i32>,
}

/// Skeleton of one avatar: local joint positions and scales.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub local_pos: Vec<Vec3>,
    pub scale: Vec<Vec3>,
    pub male: bool,
    /// Hover parameter (m).
    pub hover: f32,
}

fn vec3(s: &str) -> Option<Vec3> {
    let v: Vec<f32> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

fn f32_attr(e: &xml::Element, name: &str) -> Option<f32> {
    e.attr(name).and_then(|v| v.trim().parse().ok())
}

/// U8_to_F32 (llquantize.h).
fn u8_to_f32(v: u8, lower: f32, upper: f32) -> f32 {
    let delta = upper - lower;
    let val = v as f32 / 255.0 * delta + lower;
    if val.abs() < delta / 255.0 { 0.0 } else { val }
}

impl ShapeParams {
    /// Parse every `<param>` of the file (skeleton, meshes, layers and
    /// drivers). Bones are resolved against the skeleton; scale deltas are
    /// inherited by the bones' collision volumes, as LLPolySkeletalDistortion::setInfo.
    pub fn parse(lad: &[u8], sk: &Skeleton, rig: &Rig) -> ShapeParams {
        let mut out = ShapeParams::default();
        let Ok(root) = xml::parse(lad) else {
            log::warn!("avatar_lad.xml: cannot parse the visual parameters");
            return out;
        };
        let mut stack = vec![&root];
        while let Some(el) = stack.pop() {
            for c in el.elements() {
                if c.name == "param" {
                    if let Some(id) = c.attr("id").and_then(|v| v.trim().parse::<i32>().ok())
                        && let Some(p) = Self::parse_param(c, sk, rig)
                    {
                        out.params.entry(id).or_insert(p);
                    }
                } else {
                    stack.push(c);
                }
            }
        }
        out.transmitted = out
            .params
            .iter()
            .filter(|(_, p)| p.group == 0 || p.group == 3)
            .map(|(id, _)| *id)
            .collect();
        log::info!(
            "avatar shape: {} visual parameters, {} transmitted",
            out.params.len(),
            out.transmitted.len()
        );
        out
    }

    fn parse_param(e: &xml::Element, sk: &Skeleton, rig: &Rig) -> Option<Param> {
        let min = f32_attr(e, "value_min").unwrap_or(0.0);
        let max = f32_attr(e, "value_max").unwrap_or(1.0);
        let default = f32_attr(e, "value_default").map(|d| d.clamp(min, max)).unwrap_or(0.0);
        let sex = match e.attr("sex").unwrap_or("both") {
            "male" => SEX_MALE,
            "female" => SEX_FEMALE,
            _ => SEX_BOTH,
        };
        let group = e.attr("group").and_then(|g| g.trim().parse().ok()).unwrap_or(0);
        let kind = if let Some(s) = e.child("param_skeleton") {
            let mut deltas: Vec<BoneDelta> = Vec::new();
            let mut set = |joint: usize, scale: Vec3, offset: Option<Vec3>| {
                // later entries for the same joint replace earlier ones (map)
                match deltas.iter_mut().find(|d| d.joint == joint) {
                    Some(d) => {
                        d.scale = scale;
                        if offset.is_some() {
                            d.offset = offset;
                        }
                    }
                    None => deltas.push(BoneDelta { joint, scale, offset }),
                }
            };
            for b in s.elements().filter(|b| b.name == "bone") {
                let (Some(name), Some(scale)) = (b.attr("name"), b.attr("scale").and_then(vec3)) else {
                    continue;
                };
                let Some(&j) = rig.names.get(name) else {
                    continue;
                };
                let offset = b.attr("offset").and_then(vec3);
                set(j, scale, offset);
                // collision volumes inherit the scale deformation
                for (ci, child) in sk.joints.iter().enumerate().take(rig.len()) {
                    if child.parent == Some(j) && child.is_collision_volume {
                        set(ci, child.scale * scale, None);
                    }
                }
            }
            Kind::Skeleton(deltas)
        } else if let Some(m) = e.child("param_morph").filter(|m| m.elements().any(|v| v.name == "volume_morph")) {
            // body morphs also scale / move collision volumes: what fitted
            // meshes follow (LLPolyMorphTarget volume morphs)
            let deltas = m
                .elements()
                .filter(|v| v.name == "volume_morph")
                .filter_map(|v| {
                    let joint = *rig.names.get(v.attr("name")?)?;
                    Some(BoneDelta {
                        joint,
                        scale: v.attr("scale").and_then(vec3).unwrap_or(Vec3::ZERO),
                        offset: v.attr("pos").and_then(vec3),
                    })
                })
                .collect();
            Kind::Skeleton(deltas)
        } else if let Some(d) = e.child("param_driver") {
            let driven = d
                .elements()
                .filter(|x| x.name == "driven")
                .filter_map(|x| {
                    let id = x.attr("id")?.trim().parse().ok()?;
                    let min1 = f32_attr(x, "min1").unwrap_or(min);
                    let max1 = f32_attr(x, "max1").unwrap_or(max);
                    let max2 = f32_attr(x, "max2").unwrap_or(max1);
                    let min2 = f32_attr(x, "min2").unwrap_or(max1);
                    Some(Driven {
                        id,
                        min1,
                        max1,
                        max2,
                        min2,
                    })
                })
                .collect();
            Kind::Driver(driven)
        } else {
            Kind::Other
        };
        Some(Param {
            group,
            min,
            max,
            default,
            sex,
            kind,
        })
    }

    #[cfg(test)]
    pub fn transmitted_count(&self) -> usize {
        self.transmitted.len()
    }

    /// LLDriverParam::getDrivenWeight.
    fn driven_weight(driver: &Param, d: &Driven, driven: &Param, input: f32) -> f32 {
        let (dmin, dmax) = (driven.min, driven.max);
        if input <= d.min1 {
            if d.min1 == d.max1 && d.min1 <= driver.min { dmax } else { dmin }
        } else if input <= d.max1 {
            let t = (input - d.min1) / (d.max1 - d.min1);
            dmin + t * (dmax - dmin)
        } else if input <= d.max2 {
            dmax
        } else if input <= d.min2 {
            let t = (input - d.max2) / (d.min2 - d.max2);
            dmax + t * (dmin - dmax)
        } else if d.max2 >= driver.max {
            dmax
        } else {
            dmin
        }
    }

    /// LLVisualParam::setWeight (clamped) and LLDriverParam::setWeight.
    fn set_weight(&self, weights: &mut HashMap<i32, f32>, id: i32, w: f32, depth: u32) {
        let Some(p) = self.params.get(&id) else {
            return;
        };
        let w = if w.is_finite() { w.clamp(p.min, p.max) } else { p.default };
        weights.insert(id, w);
        if depth > 4 {
            return;
        }
        if let Kind::Driver(list) = &p.kind {
            for d in list {
                if let Some(dp) = self.params.get(&d.id) {
                    let dw = Self::driven_weight(p, d, dp, w);
                    self.set_weight(weights, d.id, dw, depth + 1);
                }
            }
        }
    }

    /// Weights of every parameter from the AvatarAppearance values (empty:
    /// defaults).
    pub fn weights(&self, values: &[u8]) -> HashMap<i32, f32> {
        let mut weights: HashMap<i32, f32> = self.params.iter().map(|(id, p)| (*id, p.default)).collect();
        // drivers push their default weight to what they drive
        for (id, p) in &self.params {
            if matches!(p.kind, Kind::Driver(_)) {
                self.set_weight(&mut weights, *id, p.default, 0);
            }
        }
        for (id, v) in self.transmitted.iter().zip(values) {
            if let Some(p) = self.params.get(id) {
                self.set_weight(&mut weights, *id, u8_to_f32(*v, p.min, p.max), 0);
            }
        }
        weights
    }

    /// The avatar's skeleton for these AvatarAppearance values.
    pub fn shape(&self, values: &[u8], rig: &Rig) -> Shape {
        let weights = self.weights(values);
        let male = weights.get(&PARAM_MALE).copied().unwrap_or(0.0) > 0.5;
        let sex = if male { SEX_MALE } else { SEX_FEMALE };
        let mut local_pos = rig.local_pos.clone();
        let mut scale = rig.scale.clone();
        for (id, p) in &self.params {
            let Kind::Skeleton(deltas) = &p.kind else {
                continue;
            };
            let w = if p.sex & sex != 0 {
                weights.get(id).copied().unwrap_or(p.default)
            } else {
                p.default
            };
            if w == 0.0 {
                continue;
            }
            for d in deltas {
                if let Some(s) = scale.get_mut(d.joint) {
                    *s += d.scale * w;
                }
                if let (Some(o), Some(p)) = (d.offset, local_pos.get_mut(d.joint)) {
                    *p += o * w;
                }
            }
        }
        Shape {
            local_pos,
            scale,
            male,
            hover: weights.get(&PARAM_HOVER).copied().unwrap_or(0.0),
        }
    }
}

/// LLAvatarAppearance::computeBodySize: (pelvis to foot, body height).
pub fn body_size(rig: &Rig, local_pos: &[Vec3], scale: &[Vec3]) -> (f32, f32) {
    let j = |n: &str| rig.names.get(n).copied();
    let pos = |n: &str| j(n).and_then(|i| local_pos.get(i)).copied().unwrap_or(Vec3::ZERO);
    let sc = |n: &str| j(n).and_then(|i| scale.get(i)).copied().unwrap_or(Vec3::ONE);
    let pelvis_to_foot = pos("mHipLeft").z * sc("mPelvis").z
        - pos("mKneeLeft").z * sc("mHipLeft").z
        - pos("mAnkleLeft").z * sc("mKneeLeft").z
        - pos("mFootLeft").z * sc("mAnkleLeft").z;
    let height = pelvis_to_foot
        + std::f32::consts::SQRT_2 * (pos("mSkull").z * sc("mHead").z)
        + pos("mHead").z * sc("mNeck").z
        + pos("mNeck").z * sc("mChest").z
        + pos("mChest").z * sc("mTorso").z
        + pos("mTorso").z * sc("mPelvis").z;
    (pelvis_to_foot, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> (Skeleton, Rig, ShapeParams) {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/character/");
        let sk = Skeleton::parse(&std::fs::read(format!("{dir}avatar_skeleton.xml")).unwrap()).unwrap();
        let rig = Rig::new(&sk);
        let p = ShapeParams::parse(&std::fs::read(format!("{dir}avatar_lad.xml")).unwrap(), &sk, &rig);
        (sk, rig, p)
    }

    #[test]
    fn shape_from_appearance_values() {
        let (_, rig, p) = load();
        assert_eq!(p.transmitted_count(), 253);
        // defaults: a female avatar of about 1.9 m (body size incl. head)
        let d = p.shape(&[], &rig);
        assert!(!d.male);
        let (p2f, h) = body_size(&rig, &d.local_pos, &d.scale);
        eprintln!("default pelvis to foot {p2f}, height {h}, root offset {}", 0.5 * h - p2f);
        assert!(p2f > 0.8 && p2f < 1.3, "pelvis to foot {p2f}");
        assert!(h > 1.5 && h < 2.3, "height {h}");
        // "male" at its maximum (index of param 80 in the message)
        let i80 = p.transmitted.iter().position(|id| *id == PARAM_MALE).unwrap();
        let mut v = vec![0u8; p.transmitted_count()];
        // keep every other parameter at its default value
        for (k, id) in p.transmitted.iter().enumerate() {
            let q = &p.params[id];
            v[k] = (((q.default - q.min) / (q.max - q.min)) * 255.0).round() as u8;
        }
        v[i80] = 255;
        let m = p.shape(&v, &rig);
        assert!(m.male);
        assert_ne!(m.scale, d.scale, "male skeleton changes bone scales");
        // the Height slider changes the body height
        let ih = p.transmitted.iter().position(|id| *id == 33).unwrap();
        let mut tall = v.clone();
        tall[ih] = 255;
        let mut short = v.clone();
        short[ih] = 0;
        let (s1, s2) = (p.shape(&tall, &rig), p.shape(&short, &rig));
        let ht = body_size(&rig, &s1.local_pos, &s1.scale).1;
        let hs = body_size(&rig, &s2.local_pos, &s2.scale).1;
        assert!(ht > hs + 0.2, "tall {ht} short {hs}");
        // body morphs carry collision volume deformations (fitted mesh)
        let belly = rig.names["BELLY"];
        assert!(
            p.params
                .values()
                .any(|q| matches!(&q.kind, Kind::Skeleton(d) if d.iter().any(|b| b.joint == belly)))
        );
    }
}
