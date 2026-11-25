//! What the 3D view is made from, taken on the main thread (`Input`), and the
//! model made of it on a worker (`StageModel`).

use super::layout::{self, Bounds};
use super::{Group, Place};
use crate::scene::Scene;
use crate::scene::anim::SkeletonBase;
use crate::scene::avatar::{AvatarLibrary, IMG_INVISIBLE};
use crate::scene::jobs::SculptMap;
use crate::scene::meshes::MeshView;
use crate::scene::textures::{PreviewRevision, PreviewSource};
use crate::shape_editor::material::Face;
use crate::world::World;
use aurora_assets::SkinInfo;
use aurora_prim::VolumeParams;
use aurora_render::preview::{Image, Material, Model, Part, Pose, Skin, Vertex};
use glam::{Mat4, Quat, Vec3};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use uuid::Uuid;

/// Longest side of a texture of the view: the cost of its own copies.
const TEXTURE_SIDE: u32 = 512;
/// Past these, the rest of the objects is left out of the view.
const MAX_PRIMS: usize = 1024;
const MAX_VERTICES: usize = 1_500_000;
/// A linkset's parents are followed no deeper.
const MAX_DEPTH: usize = 16;
/// A prim has at most nine faces, a mesh eight, a sculpt one.
const MAX_FACES: usize = 9;

enum Shape {
    Mesh {
        asset: Uuid,
        data: Option<Box<MeshView>>,
        skin: Option<Arc<SkinInfo>>,
    },
    Prim(Box<VolumeParams>),
    Sculpt(Box<VolumeParams>, Arc<SculptMap>),
}

/// Where a prim is held: on a joint of the avatar (and the point's own
/// offset there), or in the space of its linkset.
#[derive(Clone, Copy)]
struct Hold {
    joint: Option<usize>,
    point: Vec3,
    rotation: Quat,
    /// The linkset's transforms, with the prim's scale.
    local: Mat4,
}

pub(super) struct RawPrim {
    id: Uuid,
    group: usize,
    shape: Shape,
    hold: Hold,
    faces: Vec<Face>,
}

/// Everything a model is made from, held by reference: cheap to take.
pub struct Input {
    prims: Vec<RawPrim>,
    roots: Vec<Uuid>,
    worn: bool,
    /// The wearer's shape.
    body: Option<Arc<SkeletonBase>>,
    textures: HashMap<Uuid, PreviewSource>,
    /// Changes when the model would.
    pub key: u64,
}

impl Input {
    pub fn capture(world: &World, scene: &Scene, groups: &[Group], place: Place) -> Input {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let (worn, wearer) = match place {
            Place::Worn(avatar) => (true, Some(avatar)),
            Place::World => (false, None),
        };
        worn.hash(&mut hasher);
        let mut prims = Vec::new();
        'groups: for (g, group) in groups.iter().enumerate() {
            group.root.hash(&mut hasher);
            for &index in &group.prims {
                if prims.len() >= MAX_PRIMS {
                    break 'groups;
                }
                if let Some(prim) = raw_prim(world, scene, g, index, worn, wearer) {
                    hash_prim(&prim, &mut hasher);
                    prims.push(prim);
                }
            }
        }
        let body = wearer
            .and_then(|i| world.objects.get(i))
            .and_then(|o| scene.skeleton_base(&o.full_id));
        body.as_ref().map(|b| Arc::as_ptr(b) as usize).hash(&mut hasher);
        let ids: HashSet<Uuid> = prims.iter().flat_map(|p| p.faces.iter().flat_map(|f| f.textures)).collect();
        let mut ids: Vec<Uuid> = ids.into_iter().filter(|id| !id.is_nil() && *id != IMG_INVISIBLE).collect();
        ids.sort_unstable();
        let mut textures = HashMap::with_capacity(ids.len());
        for id in ids {
            let source = scene.textures.preview_source(&id);
            id.hash(&mut hasher);
            source.image_revision().hash(&mut hasher);
            textures.insert(id, source);
        }
        Input {
            prims,
            roots: groups.iter().map(|g| g.root).collect(),
            worn,
            body,
            textures,
            // (0 is "no model yet")
            key: hasher.finish().max(1),
        }
    }
}

fn hash_prim(prim: &RawPrim, hasher: &mut impl Hasher) {
    prim.id.hash(hasher);
    match &prim.shape {
        Shape::Mesh { asset, data, skin } => {
            asset.hash(hasher);
            // a finer level that arrives makes another model
            data.as_ref().and_then(|d| d.lod_section(3)).map(<[u8]>::len).hash(hasher);
            skin.as_ref().map(|s| s.joint_names.len()).hash(hasher);
        }
        Shape::Prim(params) => format!("{params:?}").hash(hasher),
        Shape::Sculpt(params, map) => {
            format!("{params:?}").hash(hasher);
            (Arc::as_ptr(map) as usize).hash(hasher);
        }
    }
    // (not where scripts move it: the prims of a running door would make a
    // model at each look)
    prim.hold.joint.hash(hasher);
    for face in &prim.faces {
        face.hash(hasher);
        face.hash_look(hasher);
        face.gloss.map(f32::to_bits).hash(hasher);
        face.fullbright.hash(hasher);
    }
}

fn raw_prim(world: &World, scene: &Scene, group: usize, index: usize, worn: bool, wearer: Option<usize>) -> Option<RawPrim> {
    let object = world.objects.get(index).filter(|o| !o.is_avatar())?;
    let hold = hold(world, index, worn)?;
    let params = object.visual_volume();
    let shape = match params.sculpt {
        Some(sculpt) if params.is_mesh() => {
            let data = scene.meshes.view(&sculpt.texture).map(Box::new);
            let skin = data
                .as_ref()
                .and_then(|m| m.skin.clone())
                .filter(|skin| worn && !skin.joint_names.is_empty());
            Shape::Mesh {
                asset: sculpt.texture,
                data,
                skin,
            }
        }
        Some(sculpt) => Shape::Sculpt(Box::new(params), scene.textures.sculpt_map(&sculpt.texture)?),
        None => Shape::Prim(Box::new(params)),
    };
    // a worn face shows its wearer's bakes; nobody's in the world
    let faces = (0..MAX_FACES)
        .map(|face| Face::capture(world, scene, object, face, wearer.unwrap_or(usize::MAX)))
        .collect();
    Some(RawPrim {
        id: object.full_id,
        group,
        shape,
        hold,
        faces,
    })
}

/// None for what is on a HUD point: no place in the view.
fn hold(world: &World, index: usize, worn: bool) -> Option<Hold> {
    let mut object = world.objects.get(index)?;
    let mut local = Mat4::from_scale(object.visual_scale());
    for _ in 0..MAX_DEPTH {
        let parent = world.objects.parent_of(object).and_then(|i| world.objects.get(i));
        match parent {
            Some(parent) if parent.is_avatar() => {
                let point = world.avatar_lib.attach_points.get(&object.attachment_point());
                if point.is_some_and(|p| p.hud) {
                    return None;
                }
                if !worn {
                    return Some(Hold {
                        joint: None,
                        point: Vec3::ZERO,
                        rotation: Quat::IDENTITY,
                        local,
                    });
                }
                return Some(Hold {
                    joint: point.and_then(|p| p.joint),
                    point: point.map_or(Vec3::ZERO, |p| p.local_position),
                    rotation: point.map_or(Quat::IDENTITY, |p| p.rotation),
                    local: Mat4::from_rotation_translation(object.rotation, object.position) * local,
                });
            }
            Some(parent) => {
                local = Mat4::from_rotation_translation(object.rotation, object.position) * local;
                object = parent;
            }
            None => {
                return (!worn).then(|| Hold {
                    joint: None,
                    point: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    // as it stands in the world, at the origin
                    local: Mat4::from_quat(object.rotation) * local,
                });
            }
        }
    }
    None
}

/// One prim of the model.
pub struct Unit {
    pub id: Uuid,
    pub group: usize,
    clusters: Range<usize>,
    bones: Range<usize>,
    parts: Range<usize>,
}

pub struct GroupShape {
    pub root: Uuid,
    pub units: Range<usize>,
}

/// What moves a bone of the model.
enum Driver {
    /// A joint of the avatar.
    Joint(usize),
    /// Held on one (or on the avatar's centre).
    Held(Hold),
    /// Nothing: an object of the world.
    Still(Mat4),
}

/// The triangles of a part that follow one bone above all, in the space of
/// their vertices: what is pointed at is looked for there, so that a body that
/// moves needs no skinning.
struct Cluster {
    bone: usize,
    part: usize,
    bounds: Bounds,
    triangles: Vec<[u32; 3]>,
}

pub struct StageModel {
    pub model: Arc<Model>,
    pub units: Vec<Unit>,
    /// In the order asked for; one with nothing to draw has no units.
    pub groups: Vec<GroupShape>,
    drivers: Vec<Driver>,
    /// From the vertices to each bone.
    bind: Vec<Mat4>,
    clusters: Vec<Cluster>,
    pelvis: usize,
    /// Where the pelvis is in the still stances.
    pelvis_rest: Vec3,
    /// The bones standing: arms wide, arms down.
    still: [Arc<[Mat4]>; 2],
    images: HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
    meshes: HashMap<(Uuid, usize), Arc<Vec<aurora_assets::MeshFace>>>,
}

/// How the model stands in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Standing {
    /// The group alone in the view, and how far that has come (0 to 1).
    pub focus: Option<(usize, f32)>,
}

/// A mesh face or a prim's, ready to be a part.
struct Drawn {
    vertices: Arc<[Vertex]>,
    indices: Arc<[u32]>,
    face: usize,
}

impl Input {
    /// The model; None when nothing of it can be drawn.
    pub fn prepare(self, lib: &AvatarLibrary, previous: Option<&StageModel>) -> Option<StageModel> {
        let images: HashMap<Uuid, (PreviewRevision, Arc<Image>)> = self
            .textures
            .into_par_iter()
            .filter_map(|(id, source)| {
                let revision = source.image_revision();
                let kept = previous.and_then(|p| p.images.get(&id));
                let image = kept
                    .filter(|(r, _)| *r == revision)
                    .map(|(_, image)| image.clone())
                    .or_else(|| source.image_at_size(id, TEXTURE_SIDE))
                    .or_else(|| kept.map(|(_, image)| image.clone()))?;
                Some((id, (revision, image)))
            })
            .collect();
        let decoded: Vec<Option<(Uuid, usize, Arc<Vec<aurora_assets::MeshFace>>)>> = self
            .prims
            .par_iter()
            .map(|prim| {
                let Shape::Mesh { asset, data, .. } = &prim.shape else {
                    return None;
                };
                let section = data.as_ref()?.lod_section(3)?;
                let key = (*asset, section.len());
                let faces = match previous.and_then(|p| p.meshes.get(&key)) {
                    Some(faces) => faces.clone(),
                    None => Arc::new(aurora_assets::decode_mesh_lod(section).ok()?),
                };
                Some((key.0, key.1, faces))
            })
            .collect();
        let meshes: HashMap<(Uuid, usize), Arc<Vec<aurora_assets::MeshFace>>> = decoded
            .iter()
            .flatten()
            .map(|(asset, len, faces)| ((*asset, *len), faces.clone()))
            .collect();
        let rig = &lib.rig;
        let skeletons = [false, true].map(|relaxed| self.worn.then(|| stance(lib, &self.prims, self.body.as_deref(), relaxed)));
        let mut out = Building::default();
        for (prim, decoded) in self.prims.iter().zip(&decoded) {
            let drawn = match (&prim.shape, decoded) {
                (Shape::Mesh { .. }, Some((_, _, faces))) => mesh_faces(faces),
                (Shape::Mesh { .. }, None) => continue,
                (Shape::Prim(params), _) => prim_faces(&aurora_prim::generate_volume(params, aurora_prim::volume::DETAIL_SCALES[3])),
                (Shape::Sculpt(params, map), _) => prim_faces(&aurora_prim::generate_sculpt(
                    params,
                    aurora_prim::volume::DETAIL_SCALES[3],
                    map.width,
                    map.height,
                    map.components,
                    &map.pixels,
                )),
            };
            let skin = match &prim.shape {
                Shape::Mesh { skin, .. } => skin.as_deref(),
                _ => None,
            };
            out.prim(prim, drawn, skin, self.worn, rig, &images);
        }
        out.finish(self.roots, skeletons, rig.pelvis, images, meshes)
    }
}

/// A relaxed stance: the arms down, the elbows and the fingers a little bent,
/// the feet a little apart. (At rest the skeleton stands arms wide, its axes
/// the avatar's: forward, left, up.)
fn relaxed() -> Vec<(String, Quat)> {
    let turn = |x: f32, y: f32, z: f32| {
        let r = f32::to_radians;
        Quat::from_rotation_x(r(x)) * Quat::from_rotation_y(r(y)) * Quat::from_rotation_z(r(z))
    };
    let mut out = vec![
        ("mNeck".to_owned(), turn(0.0, 3.0, 0.0)),
        ("mChest".to_owned(), turn(0.0, -2.0, 0.0)),
    ];
    // the right side mirrors the left one
    for (side, m) in [("Left", 1.0), ("Right", -1.0)] {
        let mut add = |name: &str, x: f32, y: f32, z: f32| out.push((format!("{name}{side}"), turn(m * x, y, m * z)));
        add("mCollar", -5.0, 0.0, 0.0);
        add("mShoulder", -66.0, 0.0, -4.0);
        add("mElbow", 0.0, 0.0, -16.0);
        add("mWrist", -4.0, 0.0, -2.0);
        add("mHip", 3.5, 0.0, 4.0);
        add("mAnkle", -3.5, 0.0, 0.0);
        for (finger, curl) in [("Index", 10.0), ("Middle", 14.0), ("Ring", 18.0), ("Pinky", 22.0)] {
            for (joint, part) in [(1, 0.8), (2, 1.2), (3, 0.8)] {
                add(&format!("mHand{finger}{joint}"), -curl * part, 0.0, 0.0);
            }
        }
        add("mHandThumb1", -8.0, 0.0, 6.0);
    }
    out
}

/// The joints of a stance: arms wide, or relaxed.
pub(super) fn stance(lib: &AvatarLibrary, prims: &[RawPrim], body: Option<&SkeletonBase>, arms_down: bool) -> Vec<Mat4> {
    let rig = &lib.rig;
    let mut rotations = rig.local_rot.clone();
    if arms_down {
        for (name, rotation) in relaxed() {
            if let Some(joint) = rig.names.get(name.as_str()).and_then(|j| rotations.get_mut(*j)) {
                *joint *= rotation;
            }
        }
    }
    // in the wearer's own shape, with the joints their meshes move
    if let Some(body) = body {
        return rig.world_with(&rotations, Vec3::ZERO, &body.local_pos, &body.scale);
    }
    let mut overrides = crate::scene::joint_overrides::JointOverrides::new(rig);
    for prim in prims {
        if let Shape::Mesh {
            asset, skin: Some(skin), ..
        } = &prim.shape
        {
            overrides.add(rig, *asset, skin);
        }
    }
    let (mut positions, mut scales) = (rig.local_pos.clone(), rig.scale.clone());
    overrides.apply(rig, &mut positions, &mut scales);
    rig.world_with(&rotations, Vec3::ZERO, &positions, &scales)
}

fn mesh_faces(faces: &[aurora_assets::MeshFace]) -> Vec<Drawn> {
    faces
        .iter()
        .enumerate()
        .map(|(face, f)| Drawn {
            vertices: f
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| Vertex {
                    position: *p,
                    normal: f.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                    uv: f.uvs.get(i).copied().unwrap_or_default(),
                    joints: f.joints.get(i).copied().unwrap_or_default().map(u32::from),
                    weights: f.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
                })
                .collect(),
            indices: f.indices.iter().map(|i| u32::from(*i)).collect(),
            face,
        })
        .collect()
}

fn prim_faces(volume: &aurora_prim::VolumeMesh) -> Vec<Drawn> {
    volume
        .faces
        .iter()
        .enumerate()
        .map(|(face, f)| Drawn {
            vertices: f
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let normal = f.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                    Vertex::new(*p, normal, f.uvs.get(i).copied().unwrap_or_default())
                })
                .collect(),
            indices: f.indices.iter().map(|i| u32::from(*i)).collect(),
            face,
        })
        .collect()
}

#[derive(Default)]
struct Building {
    parts: Vec<Part>,
    units: Vec<Unit>,
    drivers: Vec<Driver>,
    bind: Vec<Mat4>,
    clusters: Vec<Cluster>,
    vertices: usize,
}

impl Building {
    fn prim(
        &mut self,
        prim: &RawPrim,
        drawn: Vec<Drawn>,
        rigged: Option<&SkinInfo>,
        worn: bool,
        rig: &crate::scene::anim::Rig,
        images: &HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
    ) {
        let (first_bone, first_part, first_cluster) = (self.bind.len(), self.parts.len(), self.clusters.len());
        let count: usize = drawn.iter().map(|d| d.vertices.len()).sum();
        if self.vertices + count > MAX_VERTICES {
            return;
        }
        let rigged = rigged.filter(|_| worn);
        // a rigged mesh follows the stance, a bone per joint of its skin;
        // anything else has one bone of its own, at the place it is held
        match rigged {
            Some(info) => {
                for (name, matrix) in info.joint_names.iter().zip(&info.inverse_bind) {
                    let (joint, matrix) = rig.skin_bind(name, *matrix);
                    self.drivers.push(Driver::Joint(joint));
                    self.bind.push(matrix * info.bind_shape);
                }
            }
            None => {
                self.drivers.push(if worn {
                    Driver::Held(prim.hold)
                } else {
                    Driver::Still(prim.hold.local)
                });
                self.bind.push(Mat4::IDENTITY);
            }
        }
        let bones = first_bone..self.bind.len();
        if bones.is_empty() {
            return;
        }
        let skin = self.bind[bones.clone()].to_vec();
        let rest = vec![Mat4::IDENTITY; bones.len()];
        let shared = Arc::new(Skin {
            joints: bones.clone().collect(),
            inverse_bind: skin,
            rest: rest.clone(),
        });
        for d in drawn {
            let Some(look) = prim.faces.get(d.face) else { continue };
            if look.textures[0] == IMG_INVISIBLE || look.material.color[3] < 0.004 || d.indices.is_empty() {
                continue;
            }
            // what is rigid follows its one bone, whatever weights it carries
            let vertices: Arc<[Vertex]> = if rigged.is_some() {
                d.vertices
            } else {
                d.vertices
                    .iter()
                    .map(|v| Vertex {
                        joints: [0; 4],
                        weights: [1.0, 0.0, 0.0, 0.0],
                        ..*v
                    })
                    .collect()
            };
            if !vertices.iter().all(|v| Vec3::from_array(v.position).is_finite()) {
                continue;
            }
            self.clusters
                .extend(clusters(&vertices, &d.indices, first_bone, bones.len(), self.parts.len()));
            let mut textures = look.textures.map(|id| images.get(&id).map(|(_, image)| image.clone()));
            if look.fullbright {
                // it gives its own light
                textures[3] = textures[0].clone();
            }
            let blend = look.material.color[3] < 1.0
                || textures[0]
                    .as_ref()
                    .is_some_and(|image| image.rgba.iter().skip(3).step_by(4).any(|a| *a != 0 && *a != 255));
            self.vertices += vertices.len();
            self.parts.push(Part {
                diagnostic: false,
                vertices,
                indices: d.indices,
                textures,
                material: Material {
                    // a texture entry's tint is a displayed colour, a PBR
                    // material's is light
                    color: match look.gloss {
                        Some(_) => {
                            let [r, g, b, a] = look.material.color;
                            [r.max(0.0).powf(2.2), g.max(0.0).powf(2.2), b.max(0.0).powf(2.2), a]
                        }
                        None => look.material.color,
                    },
                    roughness: look.gloss.map_or(look.material.roughness, |gloss| 1.0 - 0.85 * gloss),
                    emissive: if look.fullbright {
                        let c = look.material.color;
                        [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6]
                    } else {
                        look.material.emissive
                    },
                    alpha_mode: match (look.alpha_explicit, blend) {
                        (true, _) => look.material.alpha_mode,
                        (false, true) => 1,
                        (false, false) => 2,
                    },
                    ..look.material
                },
                skin: Some(shared.clone()),
            });
        }
        if self.parts.len() == first_part {
            // nothing of it shows
            self.drivers.truncate(first_bone);
            self.bind.truncate(first_bone);
            return;
        }
        self.units.push(Unit {
            id: prim.id,
            group: prim.group,
            clusters: first_cluster..self.clusters.len(),
            bones,
            parts: first_part..self.parts.len(),
        });
    }

    fn finish(
        self,
        roots: Vec<Uuid>,
        skeletons: [Option<Vec<Mat4>>; 2],
        pelvis: usize,
        images: HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
        meshes: HashMap<(Uuid, usize), Arc<Vec<aurora_assets::MeshFace>>>,
    ) -> Option<StageModel> {
        if self.units.is_empty() {
            return None;
        }
        let range = |units: &[Unit], g: usize| {
            let start = units.partition_point(|u| u.group < g);
            start..units.partition_point(|u| u.group <= g)
        };
        let groups: Vec<GroupShape> = roots
            .into_iter()
            .enumerate()
            .map(|(g, root)| GroupShape {
                root,
                units: range(&self.units, g),
            })
            .collect();
        let mut model = StageModel {
            model: Arc::new(Model {
                parts: self.parts,
                // the palette brings everything into the unit sphere
                center: Vec3::ZERO,
                radius: 1.0,
            }),
            units: self.units,
            groups,
            drivers: self.drivers,
            bind: self.bind,
            clusters: self.clusters,
            pelvis,
            pelvis_rest: skeletons[1]
                .as_ref()
                .and_then(|s| s.get(pelvis))
                .map_or(Vec3::ZERO, |m| m.w_axis.truncate()),
            still: [Arc::from([]), Arc::from([])],
            images,
            meshes,
        };
        let empty = Vec::new();
        model.still = skeletons
            .each_ref()
            .map(|skeleton| model.bones(skeleton.as_ref().unwrap_or(&empty)));
        if skeletons[0].is_none() {
            // objects of the world, side by side
            let shown: Vec<usize> = (0..model.groups.len()).filter(|g| !model.groups[*g].units.is_empty()).collect();
            let boxes: Vec<Bounds> = shown.iter().map(|g| model.group_bounds(*g, &model.still[0])).collect();
            for (g, by) in shown.into_iter().zip(layout::side_by_side(&boxes)) {
                for unit in model.groups[g].units.clone() {
                    for bone in model.units[unit].bones.clone() {
                        if let Driver::Still(at) = &mut model.drivers[bone] {
                            *at = Mat4::from_translation(by) * *at;
                        }
                    }
                }
            }
            let bones = model.bones(&empty);
            model.still = [bones.clone(), bones];
        }
        Some(model)
    }
}

/// The triangles of a face by the bone that moves them most.
fn clusters(vertices: &[Vertex], indices: &[u32], first_bone: usize, bones: usize, part: usize) -> Vec<Cluster> {
    let mut by_bone: HashMap<usize, Cluster> = HashMap::new();
    for tri in indices.as_chunks::<3>().0 {
        let corners = tri.map(|i| vertices.get(i as usize));
        let [Some(a), Some(b), Some(c)] = corners else { continue };
        let mut most = (0, -1.0f32);
        for v in [a, b, c] {
            for (joint, weight) in v.joints.iter().zip(v.weights) {
                let sum: f32 = [a, b, c]
                    .iter()
                    .flat_map(|o| o.joints.iter().zip(o.weights))
                    .filter(|(j, _)| *j == joint)
                    .map(|(_, w)| w)
                    .sum();
                if weight > 0.0 && sum > most.1 {
                    most = (*joint as usize, sum);
                }
            }
        }
        let bone = first_bone + most.0.min(bones.saturating_sub(1));
        let cluster = by_bone.entry(bone).or_insert_with(|| Cluster {
            bone,
            part,
            bounds: Bounds::default(),
            triangles: Vec::new(),
        });
        for v in [a, b, c] {
            cluster.bounds.add(Vec3::from_array(v.position));
        }
        cluster.triangles.push(*tri);
    }
    let mut out: Vec<Cluster> = by_bone.into_values().collect();
    out.sort_by_key(|c| c.bone);
    out
}

/// Two sets of bones mixed: `t` 0 the first, 1 the second.
pub fn mixed(a: &Arc<[Mat4]>, b: &Arc<[Mat4]>, t: f32) -> Arc<[Mat4]> {
    if t <= 0.0 || a.len() != b.len() {
        return a.clone();
    }
    if t >= 1.0 {
        return b.clone();
    }
    a.iter().zip(b.iter()).map(|(a, b)| *a * (1.0 - t) + *b * t).collect()
}

/// What the view frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub center: Vec3,
    pub radius: f32,
    /// Height of what it stands on.
    pub low: f32,
}

impl Frame {
    fn of(b: Bounds) -> Self {
        if b.empty() {
            return Frame {
                center: Vec3::ZERO,
                radius: 1.0,
                low: 0.0,
            };
        }
        Frame {
            center: b.center(),
            radius: b.radius(),
            low: b.lo.z,
        }
    }

    pub fn mix(self, to: Frame, t: f32) -> Frame {
        Frame {
            center: self.center + (to.center - self.center) * t,
            radius: self.radius + (to.radius - self.radius) * t,
            low: self.low + (to.low - self.low) * t,
        }
    }

    /// The floor, in the view.
    pub fn floor(&self) -> f32 {
        (self.low - self.center.z) / self.radius
    }
}

impl StageModel {
    /// Every bone, for the joints of `skeleton`.
    pub fn bones(&self, skeleton: &[Mat4]) -> Arc<[Mat4]> {
        self.drivers
            .iter()
            .map(|driver| match driver {
                Driver::Joint(joint) => skeleton.get(*joint).copied().unwrap_or(Mat4::IDENTITY),
                Driver::Still(at) => *at,
                Driver::Held(hold) => {
                    // mRoot (Avatar Center) is outside the skeleton: the
                    // pelvis' height, as the Shape editor's hints
                    let joint =
                        hold.joint.and_then(|j| skeleton.get(j)).copied().unwrap_or_else(|| {
                            Mat4::from_translation(skeleton.get(self.pelvis).map_or(Vec3::ZERO, |m| m.w_axis.truncate()))
                        });
                    let (_, rotation, _) = joint.to_scale_rotation_translation();
                    Mat4::from_rotation_translation(rotation.normalize() * hold.rotation, joint.transform_point3(hold.point)) * hold.local
                }
            })
            .collect()
    }

    /// The bones of a still stance.
    pub fn still(&self, arms_down: bool) -> &Arc<[Mat4]> {
        &self.still[usize::from(arms_down)]
    }

    /// The box of a unit, as its bones are.
    fn unit_bounds(&self, unit: usize, bones: &[Mat4]) -> Bounds {
        let mut out = Bounds::default();
        for cluster in &self.clusters[self.units[unit].clusters.clone()] {
            let m = bones[cluster.bone] * self.bind[cluster.bone];
            for corner in cluster.bounds.corners() {
                out.add(m.transform_point3(corner));
            }
        }
        out
    }

    fn group_bounds(&self, group: usize, bones: &[Mat4]) -> Bounds {
        let boxes: Vec<Bounds> = self.groups[group].units.clone().map(|u| self.unit_bounds(u, bones)).collect();
        layout::whole(&boxes)
    }

    /// What the view frames: a sphere and its floor.
    pub fn frame(&self, focus: Option<(usize, f32)>, bones: &[Mat4]) -> Frame {
        let boxes: Vec<Bounds> = (0..self.units.len()).map(|u| self.unit_bounds(u, bones)).collect();
        let all = Frame::of(layout::whole(&boxes));
        match focus.and_then(|(g, f)| Some((self.groups.get(g)?, f))) {
            Some((group, f)) if !group.units.is_empty() => all.mix(Frame::of(layout::whole(&boxes[group.units.clone()])), f),
            _ => all,
        }
    }

    /// What the view frames while the avatar moves. Anchored where it stands:
    /// a tail, an arm or a swaying hip never moves the picture; only a body
    /// that leaves its place (a dance, a sit pose set aside) is followed, by
    /// what it is past `SLACK` of the frame.
    pub fn live_frame(&self, joints: &[Mat4]) -> Frame {
        const SLACK: f32 = 0.3;
        let mut frame = self.frame(None, &self.still[1]);
        let at = joints.get(self.pelvis).map_or(self.pelvis_rest, |m| m.w_axis.truncate());
        let away = at - self.pelvis_rest;
        let past = away.length() - frame.radius * SLACK;
        if past > 0.0 {
            frame.center += away.normalize() * past;
        }
        frame
    }

    /// How large a group shows: the others shrink away from one alone.
    fn shown(&self, group: usize, standing: Standing) -> f32 {
        match standing.focus {
            Some((g, f)) if g != group => 1.0 - f,
            _ => 1.0,
        }
    }

    pub fn pose(&self, standing: Standing, frame: &Frame, bones: &[Mat4]) -> Pose {
        let Frame { center, radius, .. } = *frame;
        let view = Mat4::from_scale(Vec3::splat(1.0 / radius)) * Mat4::from_translation(-center);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); bones.len()];
        for unit in &self.units {
            let middle = center;
            // (never quite 0: a matrix without an inverse has no normals)
            let size = self.shown(unit.group, standing).max(1e-4);
            let m = view * Mat4::from_translation(middle) * Mat4::from_scale(Vec3::splat(size)) * Mat4::from_translation(-middle);
            for bone in unit.bones.clone() {
                out[bone] = (m * bones[bone]).to_cols_array_2d();
            }
        }
        out.into()
    }

    /// The class of each part's outline, by its unit (`Highlight`).
    pub fn classes(&self, class: impl Fn(&Unit) -> u8) -> Arc<[u8]> {
        let mut out = vec![0; self.model.parts.len()];
        for unit in &self.units {
            out[unit.parts.clone()].fill(class(unit));
        }
        out.into()
    }

    /// The prim a ray of the view meets first, as the model stands.
    pub fn pick(&self, standing: Standing, frame: &Frame, bones: &[Mat4], from: Vec3, dir: Vec3) -> Option<&Unit> {
        // back into the model's own space
        let from = from * frame.radius + frame.center;
        let mut best: Option<(f32, usize)> = None;
        for (u, unit) in self.units.iter().enumerate() {
            if self.shown(unit.group, standing) < 0.5 {
                continue;
            }
            for cluster in &self.clusters[unit.clusters.clone()] {
                // the ray where the bone's vertices are; its length along it
                // stays the same
                let m = bones[cluster.bone] * self.bind[cluster.bone];
                if m.determinant().abs() < 1e-18 {
                    continue;
                }
                let back = m.inverse();
                let (at, along) = (back.transform_point3(from), back.transform_vector3(dir));
                let near = |t: f32| best.is_none_or(|(b, _)| t < b);
                if !cluster.bounds.hit(at, along).is_some_and(near) {
                    continue;
                }
                let vertices = &self.model.parts[cluster.part].vertices;
                for tri in &cluster.triangles {
                    let Some([a, b, c]) = corner(vertices, tri) else { continue };
                    if let Some(t) = triangle(at, along, a, b, c)
                        && best.is_none_or(|(b, _)| t < b)
                    {
                        best = Some((t, u));
                    }
                }
            }
        }
        best.map(|(_, u)| &self.units[u])
    }

    /// How many prims of an object the view shows.
    pub fn parts_of(&self, root: &Uuid) -> usize {
        self.groups.iter().find(|g| g.root == *root).map_or(0, |g| g.units.len())
    }
}

fn corner(vertices: &[Vertex], tri: &[u32; 3]) -> Option<[Vec3; 3]> {
    let at = |i: u32| vertices.get(i as usize).map(|v| Vec3::from_array(v.position));
    Some([at(tri[0])?, at(tri[1])?, at(tri[2])?])
}

/// Möller–Trumbore, either side of the triangle.
fn triangle(from: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-20 {
        return None;
    }
    let s = from - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t)
}
