//! Dev Toolbox J9: the skeleton of each animated object (animesh) the target
//! wears. Gathered from the scene (`Scene::worn_animesh_report`), written by
//! `text`, a pure function the tests read. Its rules restate the ones the
//! scene follows: the joints a mesh is skinned to, the joint position
//! overrides of its alternate inverse binds and which one wins
//! (LLVector3OverrideMap::findActiveOverride, indra/llcharacter/lljoint.cpp:
//! the highest mesh id), the control skeleton's rest and current local
//! positions, and the motions playing on it (#1492).

use glam::{EulerRot, Quat, Vec3};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use uuid::Uuid;

/// Strictly more than 0.1 mm from the skeleton file's position
/// (LLJoint::aboveJointPosThreshold), as `JointOverrides::add`.
const OVERRIDE_THRESHOLD: f32 = 0.0001;
/// Joint lines written per object; the rest are counted.
const MAX_JOINTS: usize = 48;

/// What J9 says of the target.
pub struct Report {
    /// The wearer's name (anonymised with the copy).
    pub wearer: String,
    pub objects: Vec<Animesh>,
}

/// One worn animated object.
pub struct Animesh {
    pub root: Uuid,
    pub point: u8,
    /// The root's place on its attachment point (its local transform).
    pub offset: Vec3,
    pub rotation: Quat,
    pub prims: usize,
    pub meshes: Vec<MeshSkin>,
    /// Joint index -> name, rest position of the skeleton file.
    pub joints: Vec<JointRef>,
    /// The control skeleton's rest local positions (with overrides), when
    /// the scene built it.
    pub rest: Option<Vec<Vec3>>,
    /// Last frame's animated offsets from `rest` and local rotations.
    pub pose: Option<(Vec<Vec3>, Vec<Quat>)>,
    pub animations: Vec<AnimLine>,
    /// Animations the simulator signals whose asset is not playing.
    pub waiting: Vec<Uuid>,
}

#[derive(Clone)]
pub struct JointRef {
    pub name: String,
    pub xml: Vec3,
}

/// A rigged mesh of the object.
pub struct MeshSkin {
    pub prim: Uuid,
    pub mesh: Uuid,
    /// None: its skin is not loaded yet.
    pub skin: Option<Skin>,
}

pub struct Skin {
    /// Joint names in the skin, and whether the skeleton knows each one.
    pub joints: Vec<(String, Option<usize>)>,
    /// Translations of the alternate inverse binds, in skin order (empty:
    /// none, so no position override).
    pub alt_positions: Vec<Vec3>,
    pub lock_scale: bool,
    pub pelvis_offset: f32,
}

/// One motion on the control skeleton.
pub struct AnimLine {
    pub id: Uuid,
    pub priority: i32,
    pub looping: bool,
    pub duration: f32,
    pub time: f64,
    pub weight: f32,
    /// Joints keyed in position: name, joint index, keys (time, position).
    pub positions: Vec<(String, Option<usize>, Vec<(f32, Vec3)>)>,
    /// Joints keyed in rotation only.
    pub rotations: Vec<String>,
}

/// A joint position override a mesh asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverrideAsk {
    pub mesh: Uuid,
    pub position: Vec3,
}

/// Per joint, every override asked (winner first): the highest mesh id
/// wins (LLVector3OverrideMap::findActiveOverride), at or below the
/// threshold nothing is asked.
pub fn overrides(meshes: &[MeshSkin], joints: &[JointRef]) -> BTreeMap<usize, Vec<OverrideAsk>> {
    let mut out: BTreeMap<usize, Vec<OverrideAsk>> = BTreeMap::new();
    for m in meshes {
        let Some(skin) = &m.skin else { continue };
        if m.mesh.is_nil() || skin.alt_positions.len() != skin.joints.len() {
            continue;
        }
        for ((_, joint), position) in skin.joints.iter().zip(&skin.alt_positions) {
            let Some(j) = *joint else { continue };
            let Some(xml) = joints.get(j).map(|r| r.xml) else { continue };
            if !position.is_finite() || (*position - xml).length_squared() <= OVERRIDE_THRESHOLD * OVERRIDE_THRESHOLD {
                continue;
            }
            out.entry(j).or_default().push(OverrideAsk {
                mesh: m.mesh,
                position: *position,
            });
        }
    }
    for asks in out.values_mut() {
        asks.sort_by_key(|a| std::cmp::Reverse(a.mesh));
    }
    out
}

/// Linear position between the keys around `time` (LLKeyframeMotion's
/// position curve), the first or last key outside them.
pub fn position_at(keys: &[(f32, Vec3)], time: f32) -> Option<Vec3> {
    let first = keys.first()?;
    if time <= first.0 {
        return Some(first.1);
    }
    for w in keys.windows(2) {
        let ((t0, p0), (t1, p1)) = (w[0], w[1]);
        if time <= t1 {
            let span = t1 - t0;
            let f = if span > 0.0 { (time - t0) / span } else { 1.0 };
            return Some(p0.lerp(p1, f));
        }
    }
    keys.last().map(|k| k.1)
}

impl super::Scene {
    /// What J9 says of the animated objects avatar `avatar` wears.
    pub fn worn_animesh_report(&self, world: &crate::world::World, avatar: Uuid, now: std::time::Instant) -> Report {
        let rig = &self.avatar_lib.rig;
        let joints: Vec<JointRef> = (0..rig.len())
            .map(|j| JointRef {
                name: rig.canonical_names.get(j).cloned().unwrap_or_default(),
                xml: rig.local_pos[j],
            })
            .collect();
        let mut objects = Vec::new();
        let worn = world
            .objects
            .index_of_uuid(&avatar)
            .and_then(|i| world.objects.get(i))
            .map(|av| world.objects.children_of(&av.key).to_vec())
            .unwrap_or_default();
        for idx in worn {
            // an animated linkset root: it owns its control skeleton
            if super::animesh::owner(&world.objects, idx).is_none_or(|(_, owner)| owner != idx) {
                continue;
            }
            let Some(root) = world.objects.get(idx) else { continue };
            let members = super::animesh::members(&world.objects, idx);
            let mut meshes = Vec::new();
            let mut signalled = Vec::new();
            for &i in &members {
                let Some(o) = world.objects.get(i) else { continue };
                signalled.extend(world.animations_of(&o.full_id).iter().map(|a| a.id));
                let volume = o.visual_volume();
                if !volume.is_mesh() {
                    continue;
                }
                let Some(sculpt) = volume.sculpt else { continue };
                let skin = match self.meshes.meta(&sculpt.texture) {
                    None => None,
                    Some(meta) => {
                        // a mesh without skin is not rigged: no joints to show
                        let Some(skin) = meta.skin else { continue };
                        Some(Skin {
                            joints: skin.joint_names.iter().map(|n| (n.clone(), rig.names.get(n).copied())).collect(),
                            alt_positions: skin.alt_inverse_bind.iter().map(|m| m.w_axis.truncate()).collect(),
                            lock_scale: skin.lock_scale_if_joint_position,
                            pelvis_offset: skin.pelvis_offset,
                        })
                    }
                };
                meshes.push(MeshSkin {
                    prim: o.full_id,
                    mesh: sculpt.texture,
                    skin,
                });
            }
            let controller = self.motions.get(&root.full_id);
            let animations: Vec<AnimLine> = controller
                .map(|c| c.playing(now))
                .unwrap_or_default()
                .into_iter()
                .map(|(id, bound, time, weight)| {
                    let a = &bound.anim;
                    AnimLine {
                        id,
                        priority: a.base_priority,
                        looping: a.looping,
                        duration: a.duration,
                        time,
                        weight,
                        positions: a
                            .joints
                            .iter()
                            .filter(|j| !j.pos_keys.is_empty())
                            .map(|j| {
                                (
                                    j.joint_name.clone(),
                                    rig.names.get(&j.joint_name).copied(),
                                    j.pos_keys.iter().map(|k| (k.time, k.position)).collect(),
                                )
                            })
                            .collect(),
                        rotations: a
                            .joints
                            .iter()
                            .filter(|j| j.pos_keys.is_empty() && !j.rot_keys.is_empty())
                            .map(|j| j.joint_name.clone())
                            .collect(),
                    }
                })
                .collect();
            signalled.sort_unstable();
            signalled.dedup();
            signalled.retain(|id| !animations.iter().any(|a| a.id == *id));
            objects.push(Animesh {
                root: root.full_id,
                point: root.attachment_point(),
                offset: root.position,
                rotation: root.rotation,
                prims: members.len(),
                meshes,
                joints: joints.clone(),
                rest: self.skeletons.get(&root.full_id).map(|s| s.base.local_pos.clone()),
                pose: controller.map(|c| {
                    let (rot, delta) = c.held_pose();
                    (delta.to_vec(), rot.to_vec())
                }),
                animations,
                waiting: signalled,
            });
        }
        Report {
            wearer: world.social.name_of(&avatar),
            objects,
        }
    }
}

fn v(p: Vec3) -> String {
    format!("({:.4}, {:.4}, {:.4})", p.x, p.y, p.z)
}

fn deg(q: Quat) -> String {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    format!("({:.1}, {:.1}, {:.1})°", x.to_degrees(), y.to_degrees(), z.to_degrees())
}

fn name(joints: &[JointRef], j: usize) -> &str {
    joints.get(j).map_or("?", |r| r.name.as_str())
}

/// The J9 text.
pub fn text(r: &Report) -> String {
    let mut s = String::new();
    if r.objects.is_empty() {
        let _ = writeln!(s, "worn animated objects of '{}': none", r.wearer);
        return s;
    }
    let _ = writeln!(s, "worn animated objects of '{}': {}", r.wearer, r.objects.len());
    for a in &r.objects {
        object(&mut s, a);
    }
    s
}

fn object(s: &mut String, a: &Animesh) {
    let rigged = a.meshes.len();
    let _ = writeln!(
        s,
        "animesh {} on point {}: offset on its point {}, rotation {}, {} prims, {} rigged meshes",
        a.root,
        a.point,
        v(a.offset),
        deg(a.rotation),
        a.prims,
        rigged
    );
    for m in &a.meshes {
        let Some(skin) = &m.skin else {
            let _ = writeln!(s, "  mesh {} on prim {}: skin not loaded yet", m.mesh, m.prim);
            continue;
        };
        // an empty slot binds nothing (counted, not listed)
        let names: Vec<&str> = skin.joints.iter().map(|(n, _)| n.as_str()).filter(|n| !n.is_empty()).collect();
        let unnamed = skin.joints.len() - names.len();
        let unknown: Vec<&str> = skin
            .joints
            .iter()
            .filter(|(n, j)| j.is_none() && !n.is_empty())
            .map(|(n, _)| n.as_str())
            .collect();
        let _ = writeln!(
            s,
            "  mesh {} on prim {}: {} joints{} [{}], alternate binds {}, lock scale {}, pelvis offset {:.4}{}",
            m.mesh,
            m.prim,
            names.len(),
            if unnamed > 0 {
                format!(" (+{unnamed} unnamed)")
            } else {
                String::new()
            },
            names.join(", "),
            if skin.alt_positions.is_empty() {
                "none".to_string()
            } else {
                skin.alt_positions.len().to_string()
            },
            skin.lock_scale,
            skin.pelvis_offset,
            if unknown.is_empty() {
                String::new()
            } else {
                format!(", unknown to the skeleton: [{}]", unknown.join(", "))
            }
        );
    }
    let asks = overrides(&a.meshes, &a.joints);
    if asks.is_empty() {
        let _ = writeln!(s, "  joint position overrides: none");
    } else {
        let _ = writeln!(
            s,
            "  joint position overrides ({} joints; the highest mesh id wins, LLVector3OverrideMap::findActiveOverride):",
            asks.len()
        );
        for (j, list) in &asks {
            let won = list[0];
            let others: Vec<String> = list[1..].iter().map(|o| format!("{} from {}", v(o.position), o.mesh)).collect();
            let _ = writeln!(
                s,
                "    {}: {} from {} (xml {}){}",
                name(&a.joints, *j),
                v(won.position),
                won.mesh,
                v(a.joints[*j].xml),
                if others.is_empty() {
                    String::new()
                } else {
                    format!("; losing: {}", others.join("; "))
                }
            );
        }
    }
    // the joints worth a line: overridden, or keyed by a motion
    let mut keyed: BTreeMap<usize, ()> = asks.keys().map(|j| (*j, ())).collect();
    for an in &a.animations {
        for (_, j, _) in &an.positions {
            if let Some(j) = j {
                keyed.insert(*j, ());
            }
        }
    }
    for an in &a.animations {
        for n in &an.rotations {
            if let Some(j) = a.joints.iter().position(|r| &r.name == n) {
                keyed.insert(j, ());
            }
        }
    }
    match &a.rest {
        None => {
            let _ = writeln!(s, "  control skeleton: not built yet");
        }
        Some(rest) => {
            let _ = writeln!(
                s,
                "  control skeleton, local positions (m): xml / rest with overrides / now, rotation now ({} joints)",
                keyed.len()
            );
            for (n, j) in keyed.keys().enumerate() {
                if n == MAX_JOINTS {
                    let _ = writeln!(s, "    … {} more joints", keyed.len() - MAX_JOINTS);
                    break;
                }
                let xml = a.joints.get(*j).map_or(Vec3::ZERO, |r| r.xml);
                let rest_p = rest.get(*j).copied().unwrap_or(xml);
                let (now, rot) = match &a.pose {
                    Some((delta, rot)) if !delta.is_empty() => {
                        (Some(rest_p + delta.get(*j).copied().unwrap_or(Vec3::ZERO)), rot.get(*j).copied())
                    }
                    _ => (None, None),
                };
                let _ = writeln!(
                    s,
                    "    {}: {} / {} / {}, {}",
                    name(&a.joints, *j),
                    v(xml),
                    v(rest_p),
                    now.map_or("not posed yet".to_string(), v),
                    rot.map_or("-".to_string(), deg)
                );
            }
        }
    }
    let _ = writeln!(s, "  animations playing: {}", a.animations.len());
    for an in &a.animations {
        let _ = writeln!(
            s,
            "    {} priority {}, {}, {:.2} s, at {:.2} s, weight {:.2}, {} joints in position, {} in rotation only",
            an.id,
            an.priority,
            if an.looping { "loop" } else { "once" },
            an.duration,
            an.time,
            an.weight,
            an.positions.len(),
            an.rotations.len()
        );
        for (n, _, keys) in &an.positions {
            let first = keys.first().map_or(Vec3::ZERO, |k| k.1);
            let now = position_at(keys, an.time as f32).unwrap_or(first);
            let _ = writeln!(s, "      {} position: {} keys, first {}, now {}", n, keys.len(), v(first), v(now));
        }
        if !an.rotations.is_empty() {
            let _ = writeln!(s, "      rotations: {}", an.rotations.join(", "));
        }
    }
    if !a.waiting.is_empty() {
        let ids: Vec<String> = a.waiting.iter().map(Uuid::to_string).collect();
        let _ = writeln!(s, "  signalled, not playing (asset loading or missing): {}", ids.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joints() -> Vec<JointRef> {
        vec![
            JointRef {
                name: "mPelvis".into(),
                xml: Vec3::new(0.0, 0.0, 1.067),
            },
            JointRef {
                name: "mGroin".into(),
                xml: Vec3::new(0.064, 0.0, -0.097),
            },
            JointRef {
                name: "mTail1".into(),
                xml: Vec3::new(-0.116, 0.0, 0.047),
            },
        ]
    }

    fn mesh(id: u128, positions: &[Vec3]) -> MeshSkin {
        MeshSkin {
            prim: Uuid::from_u128(id + 1000),
            mesh: Uuid::from_u128(id),
            skin: Some(Skin {
                joints: vec![("mPelvis".into(), Some(0)), ("mGroin".into(), Some(1)), ("mTail1".into(), Some(2))],
                alt_positions: positions.to_vec(),
                lock_scale: false,
                pelvis_offset: 0.0,
            }),
        }
    }

    #[test]
    fn the_highest_mesh_id_wins_and_default_positions_ask_nothing() {
        let j = joints();
        let low = mesh(1, &[j[0].xml, Vec3::new(0.1, 0.0, -0.2), j[2].xml]);
        let high = mesh(9, &[j[0].xml, Vec3::new(0.2, 0.0, -0.3), j[2].xml + Vec3::splat(0.00005)]);
        let asks = overrides(&[low, high], &j);
        assert_eq!(
            asks.keys().copied().collect::<Vec<_>>(),
            [1],
            "pelvis and tail stay at their default"
        );
        assert_eq!(asks[&1][0].mesh, Uuid::from_u128(9));
        assert_eq!(asks[&1][1].mesh, Uuid::from_u128(1));
    }

    #[test]
    fn a_skin_without_matching_alternate_binds_asks_nothing() {
        let j = joints();
        let partial = mesh(3, &[Vec3::ONE]);
        assert!(overrides(&[partial], &j).is_empty());
    }

    #[test]
    fn position_keys_interpolate_and_hold_outside() {
        let keys = [(0.0, Vec3::ZERO), (1.0, Vec3::new(2.0, 0.0, 0.0))];
        assert_eq!(position_at(&keys, -1.0), Some(Vec3::ZERO));
        assert_eq!(position_at(&keys, 0.5), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert_eq!(position_at(&keys, 3.0), Some(Vec3::new(2.0, 0.0, 0.0)));
        assert_eq!(position_at(&[], 0.0), None);
    }

    #[test]
    fn the_text_names_overrides_skeleton_and_motions() {
        let j = joints();
        let mut rest: Vec<Vec3> = j.iter().map(|r| r.xml).collect();
        rest[1] = Vec3::new(0.2, 0.0, -0.3);
        let report = Report {
            wearer: "Avatar 1".into(),
            objects: vec![Animesh {
                root: Uuid::from_u128(7),
                point: 10,
                offset: Vec3::new(0.0, 0.0, 0.1),
                rotation: Quat::IDENTITY,
                prims: 3,
                meshes: vec![
                    mesh(1, &[j[0].xml, Vec3::new(0.1, 0.0, -0.2), j[2].xml]),
                    mesh(9, &[j[0].xml, Vec3::new(0.2, 0.0, -0.3), j[2].xml]),
                    MeshSkin {
                        prim: Uuid::from_u128(5),
                        mesh: Uuid::from_u128(6),
                        skin: None,
                    },
                ],
                joints: j.clone(),
                rest: Some(rest),
                pose: Some((vec![Vec3::ZERO, Vec3::ZERO, Vec3::new(0.0, 0.0, 0.05)], vec![Quat::IDENTITY; 3])),
                animations: vec![AnimLine {
                    id: Uuid::from_u128(42),
                    priority: 4,
                    looping: true,
                    duration: 2.0,
                    time: 1.0,
                    weight: 1.0,
                    positions: vec![("mTail1".into(), Some(2), vec![(0.0, Vec3::ZERO), (2.0, Vec3::new(0.0, 0.0, 0.1))])],
                    rotations: vec!["mGroin".into()],
                }],
                waiting: vec![Uuid::from_u128(43)],
            }],
        };
        let t = text(&report);
        assert!(t.starts_with("worn animated objects of 'Avatar 1': 1\n"), "{t}");
        assert!(t.contains("on point 10"), "{t}");
        assert!(t.contains("skin not loaded yet"), "{t}");
        assert!(
            t.contains(&format!("mGroin: (0.2000, 0.0000, -0.3000) from {}", Uuid::from_u128(9))),
            "{t}"
        );
        assert!(
            t.contains(&format!("losing: (0.1000, 0.0000, -0.2000) from {}", Uuid::from_u128(1))),
            "{t}"
        );
        assert!(
            t.contains("mTail1: (-0.1160, 0.0000, 0.0470) / (-0.1160, 0.0000, 0.0470) / (-0.1160, 0.0000, 0.0970)"),
            "{t}"
        );
        assert!(
            t.contains("mTail1 position: 2 keys, first (0.0000, 0.0000, 0.0000), now (0.0000, 0.0000, 0.0500)"),
            "{t}"
        );
        assert!(t.contains("rotations: mGroin"), "{t}");
        assert!(t.contains(&Uuid::from_u128(43).to_string()), "{t}");
        // no world position, no region: only the place on the point
        assert!(!t.contains("region"));
    }

    #[test]
    fn unnamed_slots_are_counted_and_unknown_names_listed() {
        let mut m = mesh(1, &[]);
        if let Some(skin) = &mut m.skin {
            skin.joints.push((String::new(), None));
            skin.joints.push(("mNotABone".into(), None));
        }
        let t = text(&Report {
            wearer: "Me".into(),
            objects: vec![Animesh {
                root: Uuid::from_u128(7),
                point: 10,
                offset: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                prims: 1,
                meshes: vec![m],
                joints: joints(),
                rest: None,
                pose: None,
                animations: Vec::new(),
                waiting: Vec::new(),
            }],
        });
        assert!(t.contains("4 joints (+1 unnamed) [mPelvis, mGroin, mTail1, mNotABone]"), "{t}");
        assert!(t.contains("alternate binds none"), "{t}");
        assert!(t.contains("unknown to the skeleton: [mNotABone]"), "{t}");
        assert!(t.contains("control skeleton: not built yet"), "{t}");
    }

    #[test]
    fn a_target_without_animesh_says_so() {
        let t = text(&Report {
            wearer: "Me".into(),
            objects: Vec::new(),
        });
        assert_eq!(t, "worn animated objects of 'Me': none\n");
    }
}
