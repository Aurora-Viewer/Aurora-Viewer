//! Isolated avatar hint geometry. Shared compressed assets are decoded once
//! per snapshot off the frame thread, then visible min/max hints are morphed
//! on workers. The existing GPU preview pass draws without a readback.

use super::material::Face;
use crate::{
    scene::{
        Scene,
        avatar::{AvatarLibrary, BAKES, IMG_INVISIBLE},
    },
    world::World,
};
use aurora_render::preview::{Image, Material, Model, Part, Skin, Vertex};
use glam::{Mat4, Quat, Vec3};
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
};
use uuid::Uuid;

struct RawMesh {
    asset: Uuid,
    data: Option<crate::scene::meshes::MeshView>,
    skin: Option<Arc<aurora_assets::SkinInfo>>,
    rigid: Option<Rigid>,
    appearance: Vec<Face>,
}
struct Mesh {
    asset: Uuid,
    faces: Arc<Vec<aurora_assets::MeshFace>>,
    geometry: Arc<Vec<Geometry>>,
    skin: Option<Arc<aurora_assets::SkinInfo>>,
    rigid: Option<Rigid>,
    appearance: Vec<Face>,
}
type Geometry = (Arc<[Vertex]>, Arc<[u32]>);
fn geometry(face: &aurora_assets::MeshFace) -> Geometry {
    (
        face.positions
            .iter()
            .enumerate()
            .map(|(i, p)| Vertex {
                position: *p,
                normal: face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                uv: face.uvs.get(i).copied().unwrap_or_default(),
                joints: face.joints.get(i).copied().unwrap_or_default().map(u32::from),
                weights: face.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
            })
            .collect::<Vec<_>>()
            .into(),
        face.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
    )
}
#[derive(Clone)]
struct Rigid {
    joint: Option<usize>,
    point: Vec3,
    rotation: Quat,
    local: Mat4,
}
pub struct Input {
    textures: HashMap<Uuid, crate::scene::textures::PreviewSource>,
    body: HashMap<usize, Uuid>,
    hidden_bakes: u64,
    meshes: Vec<RawMesh>,
    pose: (Vec<Quat>, Vec<Vec3>),
    pub key: u64,
    /// What the key covers, apart: the bakes, the worn meshes and the
    /// textures (which of them made the hints be built again).
    pub parts: [u64; 3],
    /// What it leaves out on purpose: where the worn rigid meshes sit and
    /// the tint, glow and placement of the faces (counted in the log).
    pub ignored: [u64; 2],
}
pub struct Snapshot {
    textures: HashMap<Uuid, Arc<Image>>,
    revisions: HashMap<Uuid, crate::scene::textures::PreviewRevision>,
    body: HashMap<usize, Uuid>,
    hidden_bakes: u64,
    meshes: Vec<Mesh>,
    pose: (Vec<Quat>, Vec<Vec3>),
}

/// The hints look at the avatar from the front of its root, while the
/// frozen pose may have turned the pelvis: the customise animation turns it
/// half a turn, an AO stand a little. Only its turn about the vertical is
/// dropped; its tilt stays.
fn facing_front(mut pose: (Vec<Quat>, Vec<Vec3>), pelvis: Option<usize>) -> (Vec<Quat>, Vec<Vec3>) {
    if let Some(rotation) = pelvis.and_then(|pelvis| pose.0.get_mut(pelvis)) {
        let twist = Quat::from_xyzw(0.0, 0.0, rotation.z, rotation.w).normalize();
        if twist.is_finite() {
            *rotation = (*rotation * twist.inverse()).normalize();
        }
    }
    pose
}

/// LLModel's MAX_MODEL_FACES (indra/llprimitive/llmodel.h): a mesh has no
/// more faces, where a texture entry holds MAX_TES.
const MAX_MESH_FACES: usize = 8;

impl Input {
    pub fn capture(world: &World, scene: &Scene) -> Self {
        let mut out = Self {
            textures: HashMap::new(),
            body: HashMap::new(),
            hidden_bakes: 0,
            meshes: Vec::new(),
            pose: facing_front(
                scene.preview_pose(&world.agent_id),
                world.avatar_lib.rig.names.get("mPelvis").copied(),
            ),
            key: 0,
            parts: [0; 3],
            ignored: [0; 2],
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Hints freeze a pose. AO cycles and blink signals are not appearance
        // changes and must never invalidate the resident's comparison images.
        let Some(index) = world.objects.index_of_uuid(&world.agent_id) else {
            world.agent_id.hash(&mut hasher);
            out.key = hasher.finish();
            return out;
        };
        out.hidden_bakes = scene.bom_mask(world, index);
        out.hidden_bakes.hash(&mut hasher);
        for (slot, _, _) in BAKES {
            if let Some(id) = world.bake_texture(index, slot) {
                slot.hash(&mut hasher);
                id.hash(&mut hasher);
                out.body.insert(slot, id);
            }
        }
        out.parts[0] = std::mem::take(&mut hasher).finish();
        let root = index;
        let mut stack = vec![(index, 0)];
        let mut visited = HashSet::new();
        let mut signatures = Vec::new();
        let mut moves = Vec::new();
        let mut looks = Vec::new();
        while let Some((index, depth)) = stack.pop() {
            if visited.len() >= 20_000 || depth >= 16 || !visited.insert(index) {
                continue;
            }
            let Some(object) = world.objects.get(index) else {
                continue;
            };
            let parent_avatar = world
                .objects
                .parent_of(object)
                .and_then(|parent| world.objects.get(parent))
                .is_some_and(|parent| parent.is_avatar());
            if object.is_avatar() {
                if index == root {
                    stack.extend(world.objects.children_of(&object.key).iter().map(|index| (*index, depth + 1)));
                }
                continue;
            }
            if parent_avatar
                && (world
                    .avatar_lib
                    .attach_points
                    .get(&object.attachment_point())
                    .is_some_and(|p| p.hud)
                    || object.extra.extended_mesh_flags.unwrap_or(0) & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0)
            {
                continue;
            }
            stack.extend(world.objects.children_of(&object.key).iter().map(|index| (*index, depth + 1)));
            let Some(id) = object
                .visual_volume()
                .sculpt
                .filter(|_| object.visual_volume().is_mesh())
                .map(|p| p.texture)
            else {
                continue;
            };
            {
                let data = scene.meshes.view(&id);
                let skin = data
                    .as_ref()
                    .and_then(|m| m.skin.clone())
                    .filter(|skin| !skin.joint_names.is_empty());
                let rigid = skin.is_none().then(|| Rigid::capture(world, index)).flatten();
                // The geometry once held (a later level is never decoded
                // again: `prepare` keeps the first), the skin's presence,
                // the joint of a rigid mesh: not where scripts move it.
                let mut mesh_hash = std::collections::hash_map::DefaultHasher::new();
                id.hash(&mut mesh_hash);
                data.as_ref().is_some_and(|m| m.lod_section(3).is_some()).hash(&mut mesh_hash);
                skin.as_ref().map(|skin| skin.joint_names.len()).hash(&mut mesh_hash);
                let mut move_hash = std::collections::hash_map::DefaultHasher::new();
                if let Some(rigid) = &rigid {
                    rigid.joint.hash(&mut mesh_hash);
                    for value in rigid
                        .local
                        .to_cols_array()
                        .into_iter()
                        .chain(rigid.point.to_array())
                        .chain(rigid.rotation.to_array())
                    {
                        value.to_bits().hash(&mut move_hash);
                    }
                }
                let appearance: Vec<_> = (0..MAX_MESH_FACES)
                    .map(|face| Face::capture(world, scene, object, face, root))
                    .collect();
                let mut look_hash = std::collections::hash_map::DefaultHasher::new();
                for face in &appearance {
                    face.hash(&mut mesh_hash);
                    face.hash_look(&mut look_hash);
                }
                moves.push((object.full_id, move_hash.finish()));
                looks.push((object.full_id, look_hash.finish()));
                out.meshes.push(RawMesh {
                    asset: id,
                    data,
                    skin,
                    rigid,
                    appearance,
                });
                signatures.push((object.full_id, mesh_hash.finish()));
            }
        }
        signatures.sort_unstable();
        signatures.hash(&mut hasher);
        out.parts[1] = std::mem::take(&mut hasher).finish();
        for (index, mut list) in [moves, looks].into_iter().enumerate() {
            list.sort_unstable();
            list.hash(&mut hasher);
            out.ignored[index] = std::mem::take(&mut hasher).finish();
        }
        let ids: HashSet<_> = out
            .body
            .values()
            .copied()
            .chain(
                out.meshes
                    .iter()
                    .flat_map(|mesh| mesh.appearance.iter().flat_map(|face| face.textures)),
            )
            .collect();
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        for id in ids {
            if id.is_nil() || id == IMG_INVISIBLE {
                continue;
            }
            let source = scene.textures.preview_source(&id);
            id.hash(&mut hasher);
            source.image_revision().hash(&mut hasher);
            out.textures.insert(id, source);
        }
        out.parts[2] = hasher.finish();
        let mut key = std::collections::hash_map::DefaultHasher::new();
        (world.agent_id, out.parts).hash(&mut key);
        out.key = key.finish();
        out
    }

    pub fn prepare(self, previous: Option<Arc<Snapshot>>) -> Snapshot {
        let revisions = self.textures.iter().map(|(id, source)| (*id, source.image_revision())).collect();
        let textures = self
            .textures
            .into_par_iter()
            .filter_map(|(id, source)| {
                let image = previous
                    .as_ref()
                    .filter(|old| old.revisions.get(&id) == Some(&source.image_revision()))
                    .and_then(|old| old.textures.get(&id).cloned())
                    .or_else(|| source.image(id))
                    .or_else(|| previous.as_ref().and_then(|snapshot| snapshot.textures.get(&id).cloned()));
                image.map(|image| (id, image))
            })
            .collect();
        let meshes = self
            .meshes
            .into_par_iter()
            .filter_map(|mesh| {
                // An asset UUID is immutable. Reuse its decoded geometry when
                // textures arrive; a transient unavailable source cannot erase it.
                let faces = previous
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .meshes
                            .iter()
                            .find(|ready| ready.asset == mesh.asset)
                            .map(|ready| ready.faces.clone())
                    })
                    .or_else(|| {
                        // the finest level held
                        let bytes = mesh.data.as_ref()?.lod_section(3)?;
                        Some(Arc::new(aurora_assets::decode_mesh_lod(bytes).ok()?))
                    })?;
                Some(Mesh {
                    asset: mesh.asset,
                    geometry: previous
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot
                                .meshes
                                .iter()
                                .find(|ready| ready.asset == mesh.asset)
                                .map(|ready| ready.geometry.clone())
                        })
                        .unwrap_or_else(|| Arc::new(faces.iter().map(geometry).collect())),
                    faces,
                    skin: mesh.skin.or_else(|| {
                        previous.as_ref().and_then(|snapshot| {
                            snapshot
                                .meshes
                                .iter()
                                .find(|ready| ready.asset == mesh.asset)
                                .and_then(|ready| ready.skin.clone())
                        })
                    }),
                    rigid: mesh.rigid,
                    appearance: mesh.appearance,
                })
            })
            .collect();
        Snapshot {
            textures,
            revisions,
            body: self.body,
            hidden_bakes: self.hidden_bakes,
            meshes,
            pose: previous
                .as_ref()
                .filter(|snapshot| !snapshot.pose.0.is_empty())
                .map(|snapshot| snapshot.pose.clone())
                .unwrap_or(self.pose),
        }
    }
}

impl Rigid {
    fn capture(world: &World, mut index: usize) -> Option<Self> {
        let object = world.objects.get(index)?;
        let scale = Mat4::from_scale(object.visual_scale());
        let mut local = Mat4::IDENTITY;
        for _ in 0..16 {
            let object = world.objects.get(index)?;
            local = Mat4::from_rotation_translation(object.rotation, object.position) * local;
            index = world.objects.parent_of(object)?;
            if world.objects.get(index)?.is_avatar() {
                let point = world.avatar_lib.attach_points.get(&object.attachment_point());
                if point.is_some_and(|point| point.hud) {
                    return None;
                }
                let joint = point.and_then(|point| point.joint);
                return Some(Self {
                    joint,
                    point: point.map(|point| point.local_position).unwrap_or(Vec3::ZERO),
                    rotation: point.map(|point| point.rotation).unwrap_or(Quat::IDENTITY),
                    local: local * scale,
                });
            }
        }
        None
    }

    fn transform(&self, joints: &[Mat4], root: Vec3) -> Mat4 {
        // mRoot (Avatar Center) is outside the animated skeleton. In the
        // hint's feet-origin space it stays at the default pelvis height.
        let joint = self
            .joint
            .and_then(|joint| joints.get(joint))
            .copied()
            .unwrap_or(Mat4::from_translation(root));
        let (_, rotation, _) = joint.to_scale_rotation_translation();
        Mat4::from_rotation_translation(rotation.normalize() * self.rotation, joint.transform_point3(self.point)) * self.local
    }
}

impl Snapshot {
    pub fn model(&self, avatars: &AvatarLibrary, values: &[u8], params: &BTreeMap<i32, f32>, group: usize, elevation: f32) -> Arc<Model> {
        let weights = avatars.shape_params.weights_with(values, params);
        let mut shape = avatars.shape_params.shape_weights(weights.clone(), &avatars.rig);
        let rig = &avatars.rig;
        let mut overrides = crate::scene::joint_overrides::JointOverrides::new(rig);
        for mesh in &self.meshes {
            if let Some(skin) = &mesh.skin {
                overrides.add(rig, mesh.asset, skin);
            }
        }
        overrides.apply(rig, &mut shape.local_pos, &mut shape.scale);
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
            self.hidden_bakes & (1u64 << p.bake_te.min(63)) == 0
                && self.body.get(&p.bake_te) != Some(&IMG_INVISIBLE)
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
                        1
                    } else {
                        2
                    },
                    alpha_cutoff: 0.5,
                    ..Default::default()
                },
                skin: Some(system_skin.clone()),
            });
        }
        for mesh in &self.meshes {
            let faces = &mesh.faces;
            let binding = mesh.skin.as_ref().filter(|skin| !skin.joint_names.is_empty()).map(|skin| {
                let (joints, inverse): (Vec<_>, Vec<_>) = skin
                    .joint_names
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(n, m)| {
                        let (joint, m) = rig.skin_bind(n, *m);
                        (joint, m * skin.bind_shape)
                    })
                    .unzip();
                let rest = joints
                    .iter()
                    .zip(&inverse)
                    .map(|(j, m)| world.get(*j).copied().unwrap_or(Mat4::IDENTITY) * *m)
                    .collect();
                Arc::new(Skin {
                    joints,
                    inverse_bind: inverse,
                    rest,
                })
            });
            let transform = binding
                .is_none()
                .then_some(mesh.rigid.as_ref())
                .flatten()
                .map(|rigid| rigid.transform(&world, avatars.pelvis))
                .unwrap_or(Mat4::IDENTITY);
            let normal_transform = glam::Mat3::from_mat4(transform).inverse().transpose();
            for (index, face) in faces.iter().enumerate() {
                let Some(appearance) = mesh.appearance.get(index).or_else(|| mesh.appearance.first()) else {
                    continue;
                };
                if appearance.textures[0] == IMG_INVISIBLE || appearance.material.color[3] < 0.004 {
                    continue;
                }
                let (vertices, indices) = mesh.geometry.get(index).cloned().unwrap_or_else(|| geometry(face));
                let vertices = if binding.is_some() {
                    vertices
                } else {
                    vertices
                        .iter()
                        .map(|v| Vertex {
                            position: transform.transform_point3(Vec3::from_array(v.position)).to_array(),
                            normal: (normal_transform * Vec3::from_array(v.normal)).normalize_or(Vec3::Z).to_array(),
                            ..*v
                        })
                        .collect::<Vec<_>>()
                        .into()
                };
                let textures = appearance.textures.map(|id| self.textures.get(&id).cloned());
                let blend = appearance.material.color[3] < 1.0
                    || textures[0]
                        .as_ref()
                        .is_some_and(|image| image.rgba.iter().skip(3).step_by(4).any(|alpha| *alpha != 0 && *alpha != 255));
                parts.push(Part {
                    diagnostic: false,
                    vertices,
                    indices,
                    textures,
                    material: Material {
                        alpha_mode: if appearance.alpha_explicit {
                            appearance.material.alpha_mode
                        } else if blend {
                            1
                        } else {
                            2
                        },
                        ..appearance.material
                    },
                    skin: binding.clone(),
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
    fn avatar_center_hints_keep_root_and_child_sizes_independent_of_the_animated_pelvis() {
        let library = Arc::new(AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::worn::events()) {
            world.apply(event);
        }
        let root = world.objects.index_of_uuid(&crate::demo::worn::item_id()).expect("root");
        let child = world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == crate::demo::worn::CHILD)
            .map(|(i, _)| i)
            .expect("child");
        world.objects.get_mut(root).expect("root").state = 40u8.rotate_left(4);
        world.objects.get_mut(root).expect("root").scale = Vec3::new(2.0, 0.4, 1.3);
        let root_object = world.objects.get(root).expect("root");
        let root_local = Mat4::from_rotation_translation(root_object.rotation, root_object.position);
        let rig = &library.rig;
        let rest = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale);
        let moving: Vec<_> = rest
            .iter()
            .map(|matrix| Mat4::from_rotation_translation(Quat::from_rotation_x(0.8), Vec3::new(0.2, -0.3, 0.5)) * *matrix)
            .collect();
        for index in [root, child] {
            let captured = Rigid::capture(&world, index).expect("captured linkset");
            assert_eq!(captured.joint, None, "Avatar Center belongs to mRoot");
            let object = world.objects.get(index).expect("object");
            let local = Mat4::from_scale_rotation_translation(object.scale, object.rotation, object.position);
            let expected = Mat4::from_translation(library.pelvis) * if index == root { local } else { root_local * local };
            assert!(captured.transform(&rest, library.pelvis).abs_diff_eq(expected, 1e-6));
            assert!(captured.transform(&moving, library.pelvis).abs_diff_eq(expected, 1e-6));
        }
        world.objects.get_mut(root).expect("root").state = 25u8.rotate_left(4);
        let hip = Rigid::capture(&world, root).expect("hip attachment");
        assert_eq!(hip.joint, Some(rig.names["mHipLeft"]));
        assert!(
            !hip.transform(&rest, library.pelvis)
                .abs_diff_eq(hip.transform(&moving, library.pelvis), 1e-4)
        );
    }
    fn empty() -> Snapshot {
        Snapshot {
            textures: HashMap::new(),
            revisions: HashMap::new(),
            body: HashMap::new(),
            hidden_bakes: 0,
            meshes: Vec::new(),
            pose: Default::default(),
        }
    }

    #[test]
    fn rigged_skin_parity_appearance_hint_uses_the_same_pelvis_fallback() {
        let library = AvatarLibrary::load();
        let mut snapshot = empty();
        snapshot.meshes.push(Mesh {
            asset: Uuid::from_u128(1),
            faces: Arc::new(vec![aurora_assets::MeshFace {
                positions: vec![[0.0, 0.0, 1.8], [0.1, 0.0, 1.8], [0.0, 0.1, 1.8]],
                indices: vec![0, 1, 2],
                joints: vec![[0; 4]; 3],
                weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
                ..Default::default()
            }]),
            geometry: Arc::new(Vec::new()),
            skin: Some(Arc::new(aurora_assets::SkinInfo {
                joint_names: vec!["obsolete_joint".into()],
                inverse_bind: vec![Mat4::from_translation(Vec3::new(0.1, 0.2, -0.3))],
                bind_shape: Mat4::from_translation(Vec3::Z * 0.2),
                ..Default::default()
            })),
            rigid: None,
            appearance: vec![Face {
                textures: [Uuid::nil(); 4],
                material: Material::default(),
                alpha_explicit: false,
            }],
        });
        let unknown = snapshot.model(&library, &[], &BTreeMap::new(), 1, 0.0);
        Arc::make_mut(snapshot.meshes[0].skin.as_mut().unwrap()).joint_names[0] = "mPelvis".into();
        let canonical = snapshot.model(&library, &[], &BTreeMap::new(), 1, 0.0);
        let binding = unknown.parts.last().unwrap().skin.as_ref().unwrap();
        let expected = canonical.parts.last().unwrap().skin.as_ref().unwrap();
        assert_eq!(binding.joints, expected.joints);
        assert_eq!(binding.inverse_bind, expected.inverse_bind);
        assert_eq!(binding.rest, expected.rest);
    }
    fn scene(library: Arc<AvatarLibrary>) -> Scene {
        Scene::new(std::env::temp_dir().join(format!("aurora-shape-hints-{}", Uuid::new_v4())), library)
    }

    #[test]
    fn reserved_transparent_shell_keeps_the_mesh_head_skin_visible() {
        let library = Arc::new(AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in crate::demo::events() {
            world.apply(event);
        }
        let object = super::super::demo::layered_object(&world).expect("layered head");
        world.objects.upsert(world.main_region.expect("region"), object);
        let mut scene = scene(library.clone());
        let asset = super::super::demo::layered_asset(&library).expect("layered mesh");
        scene
            .meshes
            .insert_local(super::super::demo::ASSET, asset.data, asset.header, asset.skin);
        let mut input = Input::capture(&world, &scene);
        for source in input.textures.values_mut() {
            source.procedural = true;
        }
        let skin_id = Uuid::from_u128(143);
        input.body.insert(8, skin_id);
        input.meshes[0].appearance[0].textures[0] = skin_id;
        let mut source = scene.textures.preview_source(&skin_id);
        source.bytes =
            Arc::new(aurora_assets::j2k::encode_texture_j2k(64, 64, 4, &[190, 120, 80, 255].repeat(64 * 64), true).expect("skin jpeg"));
        input.textures.insert(skin_id, source);
        let snapshot = input.prepare(None);
        let model = snapshot.model(&library, &[], &BTreeMap::new(), 1, 0.0);
        let skin = &model.parts[model.parts.len() - 2];
        let shell = model.parts.last().expect("shell");
        let texture = skin.textures[0].as_ref().expect("mesh skin pixels");
        assert!(
            texture
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[0] != pixel[1] && pixel[3] > 0)
        );
        assert_eq!(
            shell.textures[0].as_ref().expect("reserved transparent pixels").rgba.as_ref(),
            &[0; 4]
        );
        assert_eq!(shell.material.alpha_mode, 2, "transparent texels must be discarded");
    }

    #[test]
    fn builtins_and_ready_progressive_resolution_need_no_additional_download() {
        let scene = scene(Arc::new(AvatarLibrary::load()));
        let transparent = scene
            .textures
            .preview_source(&aurora_prim::te::TRANSPARENT_TEXTURE)
            .image(aurora_prim::te::TRANSPARENT_TEXTURE)
            .expect("reserved pixels");
        assert_eq!(transparent.size, [1, 1]);
        assert_eq!(transparent.rgba.as_ref(), &[0; 4]);
        let blank = scene
            .textures
            .preview_source(&aurora_prim::te::BLANK_TEXTURE)
            .image(aurora_prim::te::BLANK_TEXTURE)
            .expect("reserved pixels");
        assert_eq!(blank.rgba.as_ref(), &[255; 4]);
        let id = Uuid::from_u128(142);
        let mut source = scene.textures.preview_source(&id);
        source.bytes =
            Arc::new(aurora_assets::j2k::encode_texture_j2k(64, 64, 4, &[200, 100, 50, 255].repeat(64 * 64), true).expect("jpeg"));
        source.decoded = Some(3);
        let pixels = source.image(id).expect("same resolution as world");
        assert_eq!(pixels.size, [8, 8]);
    }

    #[test]
    fn texture_hints_read_progressive_jpeg_and_cache_only_bc7_pixels() {
        let scene = scene(Arc::new(AvatarLibrary::load()));
        let id = Uuid::from_u128(111);
        let source = scene.textures.preview_source(&id);
        let rgba = [210u8, 120, 60, 255].repeat(64 * 64);
        let jpeg = aurora_assets::j2k::encode_texture_j2k(64, 64, 4, &rgba, true).expect("texture");
        std::fs::write(source.jpeg.with_extension("part"), jpeg).expect("progressive cache");
        let image = source.image(id).expect("progressive pixels");
        assert_eq!(&image.rgba[..4], &[210, 120, 60, 255]);
        let mips = aurora_render::build_mips(64, 64, rgba, 16);
        let chain = aurora_render::bc7::encode_chain(&mips, false).expect("blocks");
        let meta = crate::scene::texture_cache::Meta {
            info: aurora_assets::J2kInfo {
                width: 64,
                height: 64,
                components: 4,
                levels: 6,
            },
            discard: 0,
            alpha: crate::scene::jobs::AlphaKind::Opaque,
            alpha_channel: false,
        };
        let source = scene.textures.preview_source(&id);
        std::fs::remove_file(source.jpeg.with_extension("part")).expect("only BC7 remains");
        std::fs::write(
            &source.blocks,
            crate::scene::texture_cache::encode(&meta, None, &chain).expect("cache"),
        )
        .expect("write");
        let image = source.image(id).expect("cache-only pixels");
        assert_eq!(image.size, [64, 64]);
        for (actual, expected) in image.rgba[..4].iter().zip([210u8, 120, 60, 255]) {
            assert!(actual.abs_diff(expected) < 5);
        }
    }

    #[test]
    fn dragging_keeps_live_avatar_edits_and_waits_to_refresh_comparison_images() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let scene = scene(world.avatar_lib.clone());
        let ctx = egui::Context::default();
        state.snapshot = Some(Arc::new(empty()));
        state.snapshot_ready_key = Some(1);
        let param = super::super::catalog::PARAMS.iter().find(|p| p.id == 33).expect("height");
        state.editing = true;
        let before = world.visual_params(&world.agent_id).expect("preview").to_vec();
        for tick in 0..100 {
            state
                .draft
                .as_mut()
                .expect("draft")
                .set(param, param.min + (param.max - param.min) * tick as f32 / 100.0);
            state.changed(&mut world);
            // Even a long pause while holding the slider cannot dispatch hints.
            state.edited_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
            state.request_hints(&world, &scene.jobs, &ctx, &[33]);
            assert!(state.model_pending.is_none());
        }
        assert_ne!(world.visual_params(&world.agent_id).expect("edited"), before);
        state.editing = false;
        state.edited_at = Some(std::time::Instant::now());
        state.request_hints(&world, &scene.jobs, &ctx, &[33]);
        assert!(state.model_pending.is_none(), "numeric edits coalesce too");
        state.edited_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        state.request_hints(&world, &scene.jobs, &ctx, &[33]);
        assert!(state.model_pending.is_some(), "one batch after release");
    }

    #[test]
    fn a_slider_own_endpoints_and_rigged_geometry_are_reused() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        let mut scene = scene(world.avatar_lib.clone());
        let asset = super::super::demo::asset(&world.avatar_lib, true).expect("head");
        scene
            .meshes
            .insert_local(super::super::demo::ASSET, asset.data, asset.header, asset.skin);
        world.objects.upsert(
            world.main_region.expect("region"),
            super::super::demo::object(&world).expect("object"),
        );
        let snapshot = Input::capture(&world, &scene).prepare(None);
        let a = snapshot.model(&world.avatar_lib, &[], &BTreeMap::from([(33, -2.3)]), 1, 0.0);
        let b = snapshot.model(&world.avatar_lib, &[], &BTreeMap::from([(33, 2.0)]), 1, 0.0);
        assert!(Arc::ptr_eq(
            &a.parts.last().expect("a").vertices,
            &b.parts.last().expect("b").vertices
        ));
        assert!(Arc::ptr_eq(
            &a.parts.last().expect("a").indices,
            &b.parts.last().expect("b").indices
        ));
        let mut params = BTreeMap::from([(33, 0.0), (80, 0.0)]);
        let key = super::super::hint_key(1, &[], &params, 33);
        params.insert(33, 2.0);
        assert_eq!(key, super::super::hint_key(1, &[], &params, 33));
        params.insert(80, 1.0);
        assert_ne!(key, super::super::hint_key(1, &[], &params, 33));
    }

    #[test]
    fn scripted_tints_and_moves_leave_the_hints_alone_but_a_hidden_face_does_not() {
        let library = Arc::new(AvatarLibrary::load());
        let mut world = World::new(library.clone());
        for event in crate::demo::events() {
            world.apply(event);
        }
        let mut object = super::super::demo::object(&world).expect("mesh head");
        let region = world.main_region.expect("region");
        world.objects.upsert(region, object.clone());
        let mut scene = scene(library.clone());
        let asset = super::super::demo::asset(&library, false).expect("head asset");
        scene
            .meshes
            .insert_local(super::super::demo::ASSET, asset.data, asset.header, asset.skin);
        let first = Input::capture(&world, &scene);
        assert!(first.meshes[0].rigid.is_some());
        assert_eq!(first.meshes[0].appearance.len(), MAX_MESH_FACES);
        let mut texture = (**object.texture_entry.as_ref().expect("faces")).clone();
        texture.faces[0].color[0] *= 0.5;
        texture.faces[0].glow = 0.4;
        object.texture_entry = Some(Arc::new(texture.clone()));
        object.position += Vec3::splat(0.01);
        world.objects.upsert(region, object.clone());
        let pulsed = Input::capture(&world, &scene);
        assert_eq!(pulsed.key, first.key, "a pulsing tint or a scripted move rebuilds nothing");
        assert_ne!(pulsed.ignored, first.ignored, "but the log counts them");
        texture.faces[0].color[3] = 0.0;
        object.texture_entry = Some(Arc::new(texture));
        world.objects.upsert(region, object);
        assert_ne!(Input::capture(&world, &scene).key, first.key, "a face hidden by a body HUD shows");
    }

    #[test]
    fn worn_mesh_heads_replace_the_system_head_and_animation_cycles_keep_the_content_key() {
        let library = Arc::new(AvatarLibrary::load());
        for rigged in [false, true] {
            let mut world = World::new(library.clone());
            for event in crate::demo::events() {
                world.apply(event);
            }
            let object = super::super::demo::object(&world).expect("mesh head");
            world.objects.upsert(world.main_region.expect("region"), object);
            let mut scene = scene(library.clone());
            let asset = super::super::demo::asset(&library, rigged).expect("head asset");
            scene
                .meshes
                .insert_local(super::super::demo::ASSET, asset.data, asset.header, asset.skin);
            let input = Input::capture(&world, &scene);
            assert_ne!(input.hidden_bakes & (1 << 8), 0, "BOM head hides the system head");
            assert_eq!(input.meshes.len(), 1);
            let key = input.key;
            world.animations.insert(
                world.agent_id,
                vec![crate::world::PlayingAnimation::from_signal(
                    Uuid::from_u128(99),
                    1,
                    std::time::Instant::now(),
                    None,
                )],
            );
            assert_eq!(Input::capture(&world, &scene).key, key);
            let snapshot = input.prepare(None);
            let model = snapshot.model(&library, &[], &BTreeMap::new(), 1, 0.0);
            let head = model.parts.last().expect("actual head");
            assert_eq!(head.material.color, super::super::demo::TINT);
            assert_eq!(head.skin.is_some(), rigged);
            assert!(head.vertices.len() > 3);
            let classic = Snapshot {
                body: snapshot.body.clone(),
                ..empty()
            }
            .model(&library, &[], &BTreeMap::new(), 1, 0.0);
            let hidden = library.parts.iter().filter(|part| part.bake_te == 8).count();
            assert_eq!(model.parts.len(), classic.parts.len() - hidden + 1);
            let changed = snapshot.model(&library, &[], &BTreeMap::from([(33, 2.0)]), 1, 0.0);
            let changed = changed.parts.last().expect("changed head");
            if rigged {
                assert_ne!(head.skin.as_ref().expect("skin").rest, changed.skin.as_ref().expect("skin").rest);
            } else {
                assert_ne!(head.vertices[0].position, changed.vertices[0].position);
                let size = super::super::catalog::PARAMS
                    .iter()
                    .filter(|param| param.group == 1)
                    .min_by(|a, b| a.order.total_cmp(&b.order))
                    .expect("head size");
                let small = snapshot.model(&library, &[], &BTreeMap::from([(size.id, size.min)]), 1, 0.0);
                let large = snapshot.model(&library, &[], &BTreeMap::from([(size.id, size.max)]), 1, 0.0);
                let edge = |model: &Model| {
                    let part = model.parts.last().expect("static head");
                    Vec3::from_array(part.vertices[0].position).distance(Vec3::from_array(part.vertices[1].position))
                };
                assert!(
                    (edge(&small) - edge(&large)).abs() < 1e-5,
                    "rigid heads retain object scale, as in the world"
                );
            }
        }
    }

    #[test]
    fn hints_drop_the_pelvis_turn_of_the_customise_animation_but_keep_its_tilt() {
        let tilt = Quat::from_rotation_x(0.2);
        let pose = (
            vec![Quat::IDENTITY, tilt * Quat::from_rotation_z(std::f32::consts::PI)],
            vec![Vec3::ZERO; 2],
        );
        let front = facing_front(pose, Some(1));
        assert_eq!(front.0[0], Quat::IDENTITY);
        assert!(front.0[1].abs_diff_eq(tilt, 1e-5), "{:?}", front.0[1]);
        let forward = front.0[1] * Vec3::X;
        assert!(forward.x > 0.9, "the pelvis faces the hints' camera again: {forward:?}");
        assert_eq!(facing_front((Vec::new(), Vec::new()), Some(1)).0, Vec::<Quat>::new());
    }

    #[test]
    fn transient_sources_keep_ready_geometry_textures_and_the_frozen_pose() {
        let id = Uuid::from_u128(1);
        let image = Arc::new(Image {
            size: [1, 1],
            rgba: Arc::from([255u8; 4]),
        });
        let faces = Arc::new(vec![aurora_assets::MeshFace {
            positions: vec![[0.0; 3]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        }]);
        let previous = Snapshot {
            textures: HashMap::from([(id, image.clone())]),
            meshes: vec![Mesh {
                asset: id,
                faces: faces.clone(),
                geometry: Arc::new(Vec::new()),
                skin: None,
                rigid: None,
                appearance: vec![Face {
                    textures: [id, Uuid::nil(), Uuid::nil(), Uuid::nil()],
                    material: Material::default(),
                    alpha_explicit: false,
                }],
            }],
            pose: (vec![Quat::IDENTITY], vec![Vec3::ZERO]),
            ..empty()
        };
        let input = Input {
            textures: HashMap::from([(
                id,
                crate::scene::textures::PreviewSource {
                    local: None,
                    bytes: Arc::new(vec![0]),
                    jpeg: Default::default(),
                    blocks: Default::default(),
                    decoded: None,
                    size: None,
                    procedural: false,
                },
            )]),
            body: HashMap::new(),
            hidden_bakes: 0,
            meshes: vec![RawMesh {
                asset: id,
                data: None,
                skin: None,
                rigid: None,
                appearance: vec![Face {
                    textures: [id, Uuid::nil(), Uuid::nil(), Uuid::nil()],
                    material: Material {
                        color: [0.5; 4],
                        ..Default::default()
                    },
                    alpha_explicit: false,
                }],
            }],
            pose: (vec![Quat::from_rotation_z(1.0)], vec![Vec3::ONE]),
            key: 2,
            parts: [0; 3],
            ignored: [0; 2],
        };
        let next = input.prepare(Some(Arc::new(previous)));
        assert!(Arc::ptr_eq(&next.textures[&id], &image));
        assert!(Arc::ptr_eq(&next.meshes[0].faces, &faces));
        assert_eq!(next.meshes[0].appearance[0].material.color, [0.5; 4]);
        assert_eq!(next.pose.0, vec![Quat::IDENTITY]);
    }

    #[test]
    fn refreshes_keep_visible_hints_and_reject_models_from_the_previous_snapshot() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let scene = scene(world.avatar_lib.clone());
        let ctx = egui::Context::default();
        let old = empty().model(&world.avatar_lib, &[], &BTreeMap::new(), 1, 0.0);
        state.snapshot = Some(Arc::new(empty()));
        state.snapshot_ready_key = Some(1);
        state.snapshot_key = 2;
        state.snapshot_pending = Some(2);
        state.checked = std::time::Instant::now();
        state.models.insert((33, false), old.clone());
        state.update(&mut world, &scene, &ctx);
        assert!(Arc::ptr_eq(&state.models[&(33, false)], &old));
        state.snapshot_tx.send((2, empty())).expect("snapshot");
        state.update(&mut world, &scene, &ctx);
        assert!(Arc::ptr_eq(&state.models[&(33, false)], &old));
        let new = empty().model(&world.avatar_lib, &[], &BTreeMap::from([(33, 2.0)]), 1, 0.0);
        state
            .model_tx
            .send((state.generation, 1, vec![(33, false, new.clone())]))
            .expect("stale hint");
        state.update(&mut world, &scene, &ctx);
        assert!(Arc::ptr_eq(&state.models[&(33, false)], &old));
        state
            .model_tx
            .send((state.generation, 2, vec![(33, false, new.clone())]))
            .expect("fresh hint");
        state.update(&mut world, &scene, &ctx);
        assert!(Arc::ptr_eq(&state.models[&(33, false)], &new));
    }

    #[test]
    fn rigged_hints_follow_shape_bones_preserve_tint_and_apply_camera_elevation_once() {
        let library = AvatarLibrary::load();
        let chest = library.rig.names["mChest"];
        let tint = [0.25, 0.5, 0.75, 0.8];
        let snapshot = Snapshot {
            textures: HashMap::new(),
            revisions: HashMap::new(),
            body: HashMap::new(),
            hidden_bakes: 0,
            meshes: vec![Mesh {
                asset: Uuid::from_u128(1),
                rigid: None,
                faces: Arc::new(vec![aurora_assets::MeshFace {
                    positions: vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]],
                    indices: vec![0, 1, 2],
                    joints: vec![[0; 4]; 3],
                    weights: vec![[1.0, 0.0, 0.0, 0.0]; 3],
                    ..Default::default()
                }]),
                geometry: Arc::new(Vec::new()),
                skin: Some(Arc::new(aurora_assets::SkinInfo {
                    joint_names: vec!["mChest".into()],
                    inverse_bind: vec![library.rig.default_world_inv[chest]],
                    ..Default::default()
                })),
                appearance: vec![Face {
                    textures: [Uuid::nil(); 4],
                    material: Material {
                        color: tint,
                        ..Default::default()
                    },
                    alpha_explicit: false,
                }],
            }],
            pose: Default::default(),
        };
        let low = snapshot.model(&library, &[], &BTreeMap::from([(33, -2.3)]), 8, -0.5);
        let high = snapshot.model(&library, &[], &BTreeMap::from([(33, 2.0)]), 8, -0.5);
        let mesh = low.parts.last().expect("rigged face");
        assert_eq!(mesh.material.color, tint);
        assert_eq!(mesh.material.alpha_mode, 1);
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
            revisions: HashMap::new(),
            body: HashMap::from([(8, IMG_INVISIBLE)]),
            hidden_bakes: 0,
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
            revisions: HashMap::new(),
            body: HashMap::new(),
            hidden_bakes: 0,
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
            revisions: HashMap::new(),
            body: HashMap::new(),
            hidden_bakes: 0,
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
