//! Isolated avatar hint geometry. Shared compressed assets are decoded once
//! per snapshot off the frame thread, then visible min/max hints are morphed
//! on workers. The existing GPU preview pass draws without a readback.

use crate::{
    scene::{
        Scene,
        avatar::{AvatarLibrary, IMG_INVISIBLE},
    },
    world::World,
};
use aurora_render::preview::{Image, Material, Model, Part, Skin, Vertex};
use glam::{Mat4, Quat, Vec3};
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
};
use uuid::Uuid;

struct RawMesh {
    data: Arc<Vec<u8>>,
    skin: Arc<aurora_assets::SkinInfo>,
    textures: Vec<Uuid>,
    colors: Vec<[f32; 4]>,
}
struct Mesh {
    faces: Vec<aurora_assets::MeshFace>,
    skin: Arc<aurora_assets::SkinInfo>,
    textures: Vec<Uuid>,
    colors: Vec<[f32; 4]>,
}
pub struct Input {
    textures: HashMap<Uuid, (Arc<Vec<u8>>, PathBuf)>,
    body: HashMap<usize, Uuid>,
    meshes: Vec<RawMesh>,
    pose: (Vec<Quat>, Vec<Vec3>),
    pub key: u64,
}
pub struct Snapshot {
    textures: HashMap<Uuid, Arc<Image>>,
    body: HashMap<usize, Uuid>,
    meshes: Vec<Mesh>,
    pose: (Vec<Quat>, Vec<Vec3>),
}

impl Input {
    pub fn capture(world: &World, scene: &Scene) -> Self {
        let mut out = Self {
            textures: HashMap::new(),
            body: HashMap::new(),
            meshes: Vec::new(),
            pose: scene.preview_pose(&world.agent_id),
            key: 0,
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        world.agent_id.hash(&mut hasher);
        for motion in world.animations.get(&world.agent_id).into_iter().flatten() {
            motion.id.hash(&mut hasher);
            scene.anims.is_loaded(&motion.id).hash(&mut hasher);
        }
        let Some(index) = world.objects.index_of_uuid(&world.agent_id) else {
            return out;
        };
        for part in &world.avatar_lib.parts {
            if let Some(id) = world.bake_texture(index, part.bake_te) {
                part.bake_te.hash(&mut hasher);
                id.hash(&mut hasher);
                out.body.insert(part.bake_te, id);
            }
        }
        let mut stack = vec![index];
        let mut visited = HashSet::new();
        while let Some(index) = stack.pop() {
            if !visited.insert(index) {
                continue;
            }
            let Some(object) = world.objects.get(index) else {
                continue;
            };
            if object.is_avatar()
                || world
                    .avatar_lib
                    .attach_points
                    .get(&object.attachment_point())
                    .is_some_and(|p| p.hud)
            {
                if object.is_avatar() {
                    stack.extend(world.objects.children_of(&object.key));
                }
                continue;
            }
            stack.extend(world.objects.children_of(&object.key));
            let Some(id) = object
                .visual_volume()
                .sculpt
                .filter(|_| object.visual_volume().is_mesh())
                .map(|p| p.texture)
            else {
                continue;
            };
            if let Some((data, skin)) = scene.meshes.preview_source(&id) {
                id.hash(&mut hasher);
                data.len().hash(&mut hasher);
                let textures: Vec<Uuid> = object
                    .te
                    .as_ref()
                    .map(|te| {
                        te.faces
                            .iter()
                            .map(|face| {
                                crate::scene::avatar::bake_slot_for(&face.texture)
                                    .and_then(|(slot, _)| out.body.get(&slot).copied())
                                    .unwrap_or(face.texture)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                textures.hash(&mut hasher);
                let colors: Vec<_> = object
                    .te
                    .as_ref()
                    .map(|te| te.faces.iter().map(|face| face.color).collect())
                    .unwrap_or_default();
                for color in &colors {
                    for channel in color {
                        channel.to_bits().hash(&mut hasher);
                    }
                }
                out.meshes.push(RawMesh {
                    data,
                    skin,
                    textures,
                    colors,
                });
            }
        }
        let ids: HashSet<_> = out
            .body
            .values()
            .copied()
            .chain(out.meshes.iter().flat_map(|mesh| mesh.textures.iter().copied()))
            .collect();
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        for id in ids {
            if id.is_nil() || id == IMG_INVISIBLE {
                continue;
            }
            let source = scene.textures.preview_source(&id);
            id.hash(&mut hasher);
            source.0.len().hash(&mut hasher);
            out.textures.insert(id, source);
        }
        out.key = hasher.finish();
        out
    }

    pub fn prepare(self) -> Snapshot {
        let textures = self
            .textures
            .into_par_iter()
            .filter_map(|(id, (bytes, path))| {
                let bytes = if bytes.is_empty() {
                    std::fs::read(path).ok().map(Arc::new).unwrap_or(bytes)
                } else {
                    bytes
                };
                aurora_assets::decode_j2k(&bytes, 2).ok().map(|pixels| {
                    (
                        id,
                        Arc::new(Image {
                            size: [pixels.width, pixels.height],
                            rgba: crate::scene::jobs::to_rgba(pixels.components, &pixels.data, (pixels.width * pixels.height) as usize)
                                .into(),
                        }),
                    )
                })
            })
            .collect();
        let meshes = self
            .meshes
            .into_par_iter()
            .filter_map(|mesh| {
                let header = aurora_assets::parse_mesh_header(&mesh.data).ok()?;
                let lod = header.actual_lod(3)?;
                let bytes = aurora_assets::mesh::section_bytes(&mesh.data, header.lods[lod]?)?;
                Some(Mesh {
                    faces: aurora_assets::decode_mesh_lod(bytes).ok()?,
                    skin: mesh.skin,
                    textures: mesh.textures,
                    colors: mesh.colors,
                })
            })
            .collect();
        Snapshot {
            textures,
            body: self.body,
            meshes,
            pose: self.pose,
        }
    }
}

impl Snapshot {
    pub fn model(&self, avatars: &AvatarLibrary, values: &[u8], params: &BTreeMap<i32, f32>, group: usize, elevation: f32) -> Arc<Model> {
        let weights = avatars.shape_params.weights_with(values, params);
        let mut shape = avatars.shape_params.shape_weights(weights.clone(), &avatars.rig);
        let rig = &avatars.rig;
        for mesh in &self.meshes {
            let skin = &mesh.skin;
            if skin.alt_inverse_bind.len() != skin.joint_names.len() {
                continue;
            }
            for (name, alt) in skin.joint_names.iter().zip(&skin.alt_inverse_bind) {
                if name == "mPelvis" {
                    continue;
                }
                if let Some(&index) = rig.names.get(name) {
                    let pos = alt.w_axis.truncate();
                    if pos.is_finite() && (pos - rig.local_pos[index]).length_squared() > 1e-8 {
                        shape.local_pos[index] = pos;
                        if skin.lock_scale_if_joint_position {
                            shape.scale[index] = rig.scale[index];
                        }
                    }
                }
            }
        }
        for (pos, delta) in shape.local_pos.iter_mut().zip(&self.pose.1) {
            *pos += *delta;
        }
        let rotation = if self.pose.0.len() == rig.len() {
            &self.pose.0
        } else {
            &rig.local_rot
        };
        let world = rig.world_with(rotation, Vec3::ZERO, &shape.local_pos, &shape.scale);
        let mut inverse = rig.default_world_inv.clone();
        inverse.push(Mat4::IDENTITY);
        let mut rest: Vec<_> = world.iter().zip(&inverse).map(|(m, i)| *m * *i).collect();
        rest.push(Mat4::IDENTITY);
        let system_skin = Arc::new(Skin {
            joints: (0..inverse.len()).collect(),
            inverse_bind: inverse,
            rest,
        });
        let mut parts = Vec::new();
        for part in avatars.parts.iter().filter(|p| {
            self.body.get(&p.bake_te) != Some(&IMG_INVISIBLE)
                && (p.name != "skirt"
                    || self
                        .body
                        .get(&p.bake_te)
                        .is_some_and(|id| !id.is_nil() && *id != crate::scene::avatar::IMG_DEFAULT_AVATAR))
        }) {
            let mw: Vec<_> = part.morphs.iter().map(|m| weights.get(&m.param).copied().unwrap_or(0.0)).collect();
            let (geometry, _, _) = crate::scene::morph::morphed(part, &mw);
            let vertices = geometry
                .vertices
                .iter()
                .enumerate()
                .map(|(index, v)| {
                    let s = part.skin.get(index).copied().unwrap_or_default();
                    Vertex {
                        position: v.pos,
                        normal: v.normal[..3]
                            .try_into()
                            .map(|n: [i8; 3]| n.map(|x| x as f32 / 127.0))
                            .unwrap_or([0.0, 0.0, 1.0]),
                        uv: v.uv,
                        joints: s.joints.map(u32::from),
                        weights: s.weights.map(|x| x as f32 / 255.0),
                    }
                })
                .collect::<Vec<_>>();
            let texture = self.body.get(&part.bake_te).and_then(|id| self.textures.get(id)).cloned();
            parts.push(Part {
                diagnostic: false,
                vertices: vertices.into(),
                indices: part.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                textures: [texture, None, None, None],
                material: Material {
                    alpha_mode: if matches!(part.name, "hair" | "eyelashes" | "skirt") {
                        2
                    } else {
                        1
                    },
                    alpha_cutoff: 0.5,
                    ..Default::default()
                },
                skin: Some(system_skin.clone()),
            });
        }
        for mesh in &self.meshes {
            let (faces, skin, textures) = (&mesh.faces, &mesh.skin, &mesh.textures);
            let joints: Vec<_> = skin
                .joint_names
                .iter()
                .map(|n| rig.names.get(n).copied().unwrap_or(rig.len()))
                .collect();
            let inverse: Vec<_> = skin.inverse_bind.iter().map(|m| *m * skin.bind_shape).collect();
            let rest = joints
                .iter()
                .zip(&inverse)
                .map(|(j, m)| world.get(*j).copied().unwrap_or(Mat4::IDENTITY) * *m)
                .collect();
            let binding = Arc::new(Skin {
                joints,
                inverse_bind: inverse,
                rest,
            });
            for (index, face) in faces.iter().enumerate() {
                let vertices = face
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(i, p)| Vertex {
                        position: *p,
                        normal: face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                        uv: face.uvs.get(i).copied().unwrap_or_default(),
                        joints: face.joints.get(i).copied().unwrap_or_default().map(u32::from),
                        weights: face.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
                    })
                    .collect::<Vec<_>>();
                let texture = textures.get(index).and_then(|id| self.textures.get(id)).cloned();
                let color = mesh.colors.get(index).copied().unwrap_or([1.0; 4]);
                let blend = color[3] < 1.0
                    || texture
                        .as_ref()
                        .is_some_and(|image| image.rgba.iter().skip(3).step_by(4).any(|alpha| *alpha != 0 && *alpha != 255));
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: face.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    textures: [texture, None, None, None],
                    material: Material {
                        color,
                        alpha_mode: if blend { 2 } else { 1 },
                        alpha_cutoff: 0.5,
                        ..Default::default()
                    },
                    skin: Some(binding.clone()),
                });
            }
        }
        let joint = match group {
            1..=6 => "mHead",
            7 => "mTorso",
            _ => "mPelvis",
        };
        let mut center = rig
            .names
            .get(joint)
            .and_then(|j| world.get(*j))
            .map(|m| m.w_axis.truncate())
            .unwrap_or(Vec3::Z)
            + Vec3::Z * elevation;
        if group == 0 {
            let head = rig.names.get("mHead").and_then(|j| world.get(*j)).map(|m| m.w_axis.z);
            let foot = rig.names.get("mFootLeft").and_then(|j| world.get(*j)).map(|m| m.w_axis.z);
            if let Some((head, foot)) = head.zip(foot) {
                center.z = (head + foot + 0.1) * 0.5 + elevation;
            }
        }
        Arc::new(Model {
            parts,
            center,
            radius: 1.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rigged_hints_follow_shape_bones_preserve_tint_and_apply_camera_elevation_once() {
        let library = AvatarLibrary::load();
        let chest = library.rig.names["mChest"];
        let tint = [0.25, 0.5, 0.75, 0.8];
        let snapshot = Snapshot {
            textures: HashMap::new(),
            body: HashMap::new(),
            meshes: vec![Mesh {
                faces: vec![aurora_assets::MeshFace {
                    positions: vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]],
                    indices: vec![0, 1, 2],
                    joints: vec![[0; 4]; 3],
                    weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
                    ..Default::default()
                }],
                skin: Arc::new(aurora_assets::SkinInfo {
                    joint_names: vec!["mChest".into()],
                    inverse_bind: vec![library.rig.default_world_inv[chest]],
                    ..Default::default()
                }),
                textures: Vec::new(),
                colors: vec![tint],
            }],
            pose: Default::default(),
        };
        let low = snapshot.model(&library, &[], &BTreeMap::from([(33, -2.3)]), 8, -0.5);
        let high = snapshot.model(&library, &[], &BTreeMap::from([(33, 2.0)]), 8, -0.5);
        let mesh = low.parts.last().expect("rigged face");
        assert_eq!(mesh.material.color, tint);
        assert_eq!(mesh.material.alpha_mode, 2);
        assert_ne!(
            mesh.skin.as_ref().expect("skin").rest,
            high.parts.last().expect("face").skin.as_ref().expect("skin").rest
        );
        let centered = snapshot.model(&library, &[], &BTreeMap::from([(33, -2.3)]), 8, 0.0);
        assert!((low.center.z - centered.center.z + 0.5).abs() < 1e-6);
    }
    #[test]
    fn shape_hints_deform_the_body_and_keep_invisible_system_parts_hidden() {
        let library = AvatarLibrary::load();
        let snapshot = Snapshot {
            textures: HashMap::new(),
            body: HashMap::from([(8, IMG_INVISIBLE)]),
            meshes: Vec::new(),
            pose: Default::default(),
        };
        let low = snapshot.model(&library, &[], &BTreeMap::from([(33, -2.3), (80, 1.0)]), 0, 0.0);
        let high = snapshot.model(&library, &[], &BTreeMap::from([(33, 2.0), (80, 1.0)]), 0, 0.0);
        assert!(!low.parts.is_empty());
        assert_eq!(low.parts.len(), high.parts.len());
        // Height changes the bone palette, even though this slider has no morph.
        assert_ne!(
            low.parts[0].skin.as_ref().expect("skin").rest,
            high.parts[0].skin.as_ref().expect("skin").rest
        );
        let without_mask = Snapshot {
            textures: HashMap::new(),
            body: HashMap::new(),
            meshes: Vec::new(),
            pose: Default::default(),
        }
        .model(&library, &[], &BTreeMap::new(), 1, 0.0);
        assert!(without_mask.parts.len() > low.parts.len());
        let normal = snapshot.model(&library, &[], &BTreeMap::from([(1, -0.3)]), 1, 0.1);
        let brow = snapshot.model(&library, &[], &BTreeMap::from([(1, 2.0)]), 1, 0.1);
        // With a head visible, facial controls change its actual vertices.
        let visible = Snapshot {
            textures: HashMap::new(),
            body: HashMap::new(),
            meshes: Vec::new(),
            pose: Default::default(),
        };
        let a = visible.model(&library, &[], &BTreeMap::from([(1, -0.3)]), 1, 0.1);
        let b = visible.model(&library, &[], &BTreeMap::from([(1, 2.0)]), 1, 0.1);
        assert_eq!(normal.parts.len(), brow.parts.len());
        assert!(
            a.parts
                .iter()
                .zip(&b.parts)
                .any(|(a, b)| a.vertices.iter().zip(b.vertices.iter()).any(|(a, b)| a.position != b.position))
        );
    }
}
