//! What the 3D view is made from, taken on the main thread (`Input`), and the
//! model made of it on a worker (`StageModel`).

use super::layout::{self, Bounds};
use super::own::{self, Own, OwnInput};
use super::{Group, Place};
use crate::scene::Scene;
use crate::scene::anim::SkeletonBase;
use crate::scene::avatar::{AvatarLibrary, IMG_INVISIBLE};
use crate::scene::jobs::SculptMap;
use crate::scene::meshes::MeshView;
use crate::scene::textures::{PreviewRevision, PreviewSource, is_builtin_transparent};
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
/// The avatar's own body without its bake (Scene::sync_avatar).
const PLAIN_BODY: [f32; 4] = [0.62, 0.58, 0.66, 1.0];
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
    /// The avatar's own body: its parts that show, and their morphs' weights.
    Body {
        parts: Vec<usize>,
        values: Vec<u8>,
        generation: u64,
    },
}

/// Where a prim is held: on a joint of the avatar (and the point's own
/// offset there), or in the space of its linkset.
#[derive(Clone, Copy)]
pub(super) struct Hold {
    joint: Option<usize>,
    point: Vec3,
    rotation: Quat,
    /// The linkset's transforms, with the prim's scale.
    pub local: Mat4,
}

impl Hold {
    /// Nowhere else than where it is made.
    const HERE: Hold = Hold {
        joint: None,
        point: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        local: Mat4::IDENTITY,
    };

    /// Its place on the joints of `skeleton`.
    pub fn on(&self, skeleton: &[Mat4], pelvis: usize) -> Mat4 {
        // mRoot (Avatar Center) is outside the skeleton: the pelvis' height,
        // as the Shape editor's hints
        let joint = self
            .joint
            .and_then(|j| skeleton.get(j))
            .copied()
            .unwrap_or_else(|| Mat4::from_translation(skeleton.get(pelvis).map_or(Vec3::ZERO, |m| m.w_axis.truncate())));
        let (_, rotation, _) = joint.to_scale_rotation_translation();
        Mat4::from_rotation_translation(rotation.normalize() * self.rotation, joint.transform_point3(self.point)) * self.local
    }
}

pub(super) struct RawPrim {
    id: Uuid,
    group: usize,
    shape: Shape,
    hold: Hold,
    faces: Vec<Face>,
    /// The skeleton of its own its linkset has (an animesh).
    own: Option<usize>,
}

/// Everything a model is made from, held by reference: cheap to take.
pub struct Input {
    prims: Vec<RawPrim>,
    roots: Vec<Uuid>,
    worn: bool,
    /// The wearer's shape.
    body: Option<Arc<SkeletonBase>>,
    own: Vec<OwnInput>,
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
        let mut own: Vec<OwnInput> = Vec::new();
        'groups: for (g, group) in groups.iter().enumerate() {
            group.root.hash(&mut hasher);
            let skeleton = OwnInput::of(world, scene, group, g, worn).map(|o| {
                o.hash(&mut hasher);
                own.push(o);
                own.len() - 1
            });
            for &index in &group.prims {
                if prims.len() >= MAX_PRIMS {
                    break 'groups;
                }
                if let Some(prim) = raw_prim(world, scene, g, index, worn, wearer, skeleton) {
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
            own,
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
        Shape::Body { parts, generation, .. } => {
            parts.hash(hasher);
            generation.hash(hasher);
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

fn raw_prim(
    world: &World,
    scene: &Scene,
    group: usize,
    index: usize,
    worn: bool,
    wearer: Option<usize>,
    own: Option<usize>,
) -> Option<RawPrim> {
    let object = world.objects.get(index)?;
    if object.is_avatar() {
        // their own body, only on themselves
        return (wearer == Some(index)).then(|| own_body(world, scene, group, index)).flatten();
    }
    let hold = hold(world, index, worn)?;
    let params = object.visual_volume();
    let shape = match params.sculpt {
        Some(sculpt) if params.is_mesh() => {
            let data = scene.meshes.view(&sculpt.texture).map(Box::new);
            let skin = data
                .as_ref()
                .and_then(|m| m.skin.clone())
                .filter(|skin| (worn || own.is_some()) && !skin.joint_names.is_empty());
            Shape::Mesh {
                asset: sculpt.texture,
                data,
                skin,
            }
        }
        Some(sculpt) => Shape::Sculpt(Box::new(params), scene.textures.sculpt_map(&sculpt.texture)?),
        None => Shape::Prim(Box::new(params)),
    };
    // a worn face shows its wearer's bakes; nobody's in the world, nor on an
    // animesh (Scene::bake_wearer)
    let bakes = wearer.filter(|_| own.is_none()).unwrap_or(usize::MAX);
    let faces = (0..MAX_FACES)
        .map(|face| Face::capture(world, scene, object, face, bakes))
        .collect();
    Some(RawPrim {
        id: object.full_id,
        group,
        shape,
        hold,
        faces,
        own,
    })
}

/// The parts of the avatar's own body that show, as `Scene::sync_avatar` draws
/// them.
fn own_body(world: &World, scene: &Scene, group: usize, index: usize) -> Option<RawPrim> {
    let avatar = world.objects.get(index)?;
    let shown = own::body_parts(world, scene, index);
    let faces = shown
        .iter()
        .map(|&(part, bake)| {
            let blended = matches!(world.avatar_lib.parts[part].name, "hair" | "eyelashes" | "skirt");
            Face {
                textures: [bake.unwrap_or_default(), Uuid::nil(), Uuid::nil(), Uuid::nil()],
                material: Material {
                    color: [1.0; 4],
                    alpha_mode: if blended { 1 } else { 2 },
                    alpha_cutoff: 0.2,
                    roughness: 0.6,
                    ..Default::default()
                },
                alpha_explicit: true,
                gloss: None,
                fullbright: false,
            }
        })
        .collect();
    (!shown.is_empty()).then(|| RawPrim {
        id: avatar.full_id,
        group,
        shape: Shape::Body {
            parts: shown.iter().map(|(part, _)| *part).collect(),
            values: world.visual_params(&avatar.full_id).unwrap_or(&[]).to_vec(),
            generation: world.appearance_generation(&avatar.full_id),
        },
        hold: Hold::HERE,
        faces,
        own: None,
    })
}

/// None for what is on a HUD point: no place in the view.
fn hold(world: &World, index: usize, worn: bool) -> Option<Hold> {
    let scale = world.objects.get(index)?.visual_scale();
    hold_from(world, index, worn, Mat4::from_scale(scale))
}

/// `hold` of what is at `local` in the space of a prim.
pub(super) fn hold_from(world: &World, index: usize, worn: bool, mut local: Mat4) -> Option<Hold> {
    let mut object = world.objects.get(index)?;
    for _ in 0..MAX_DEPTH {
        let parent = world.objects.parent_of(object).and_then(|i| world.objects.get(i));
        match parent {
            Some(parent) if parent.is_avatar() => {
                let point = world.avatar_lib.attach_points.get(&object.attachment_point());
                if point.is_some_and(|p| p.hud) {
                    return None;
                }
                if !worn {
                    return Some(Hold { local, ..Hold::HERE });
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
                // as it stands in the world, at the origin
                return (!worn).then(|| Hold {
                    local: Mat4::from_quat(object.rotation) * local,
                    ..Hold::HERE
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
pub(super) enum Driver {
    /// A joint of the avatar.
    Joint(usize),
    /// A joint of an animesh's own skeleton (its index).
    Own(usize, usize),
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
    /// Joints of a skeleton: those of the wearer come first in what `bones`
    /// is given, then those of each animesh (`own`).
    joints: usize,
    own: Vec<Own>,
    /// The bones at rest.
    still: Arc<[Mat4]>,
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
        let skeleton = self.worn.then(|| rest(lib, &self.prims, self.body.as_deref()));
        let own: Vec<Own> = self
            .own
            .into_iter()
            .enumerate()
            .map(|(set, o)| {
                let skins = self.prims.iter().filter(|p| p.own == Some(set));
                let still = o.joints.unwrap_or_else(|| at_rest(lib, skins, o.base.as_deref()));
                let place = if self.worn {
                    Driver::Held(o.hold)
                } else {
                    Driver::Still(o.hold.local)
                };
                Own {
                    root: o.root,
                    group: o.group,
                    place,
                    after: Mat4::from_scale(Vec3::splat(o.scale)) * Mat4::from_translation(-lib.pelvis),
                    still,
                }
            })
            .collect();
        let mut out = Building {
            worn: self.worn,
            ..Building::default()
        };
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
                (Shape::Body { parts, values, .. }, _) => own::body_faces(lib, parts, values)
                    .into_iter()
                    .enumerate()
                    .map(|(face, (vertices, indices))| Drawn { vertices, indices, face })
                    .collect(),
            };
            let skin = match &prim.shape {
                Shape::Mesh { skin, .. } => skin.as_deref(),
                _ => None,
            };
            out.prim(prim, drawn, skin, rig, &images);
        }
        out.finish(self.roots, skeleton, rig, own, images, meshes)
    }
}

/// The joints at rest, in the wearer's own shape: only for an avatar whose
/// pose is not known (the view never poses anyone).
pub(super) fn rest(lib: &AvatarLibrary, prims: &[RawPrim], body: Option<&SkeletonBase>) -> Vec<Mat4> {
    at_rest(lib, prims.iter().filter(|p| p.own.is_none()), body)
}

fn at_rest<'a>(lib: &AvatarLibrary, prims: impl Iterator<Item = &'a RawPrim>, body: Option<&SkeletonBase>) -> Vec<Mat4> {
    let rig = &lib.rig;
    // with the joints their meshes move
    if let Some(body) = body {
        return rig.world_with(&rig.local_rot, Vec3::ZERO, &body.local_pos, &body.scale);
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
    rig.world_with(&rig.local_rot, Vec3::ZERO, &positions, &scales)
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
    worn: bool,
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
        rig: &crate::scene::anim::Rig,
        images: &HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
    ) {
        let (first_bone, first_part, first_cluster) = (self.bind.len(), self.parts.len(), self.clusters.len());
        let count: usize = drawn.iter().map(|d| d.vertices.len()).sum();
        if self.vertices + count > MAX_VERTICES {
            return;
        }
        // a rigged mesh follows a skeleton, a bone per joint of its skin;
        // anything else has one bone of its own, at the place it is held
        let skinned = matches!(prim.shape, Shape::Body { .. }) || rigged.is_some();
        match rigged {
            Some(info) => {
                for (name, matrix) in info.joint_names.iter().zip(&info.inverse_bind) {
                    let (joint, matrix) = rig.skin_bind(name, *matrix);
                    self.drivers.push(match prim.own {
                        Some(set) => Driver::Own(set, joint),
                        None => Driver::Joint(joint),
                    });
                    self.bind.push(matrix * info.bind_shape);
                }
            }
            None if skinned => {
                for (joint, matrix) in rig.default_world_inv.iter().enumerate() {
                    self.drivers.push(Driver::Joint(joint));
                    self.bind.push(*matrix);
                }
                // (mRoot, outside the skeleton)
                self.drivers.push(Driver::Still(Mat4::IDENTITY));
                self.bind.push(Mat4::IDENTITY);
            }
            None => {
                self.drivers.push(if self.worn {
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
            let hidden = is_builtin_transparent(&look.textures[0]) && look.gloss.is_some();
            if hidden || look.textures[0] == IMG_INVISIBLE || look.material.color[3] < 0.004 || d.indices.is_empty() {
                continue;
            }
            // what is rigid follows its one bone, whatever weights it carries
            let vertices: Arc<[Vertex]> = if skinned {
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
            let mut color = look.material.color;
            if matches!(prim.shape, Shape::Body { .. }) && textures[0].is_none() {
                // (its bake is not there yet)
                color = PLAIN_BODY;
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
                        None => color,
                    },
                    roughness: look.gloss.map_or(look.material.roughness, |gloss| 1.0 - 0.85 * gloss),
                    emissive: if look.fullbright {
                        let c = look.material.color;
                        [c[0] * 0.6, c[1] * 0.6, c[2] * 0.6]
                    } else {
                        look.material.emissive
                    },
                    alpha_mode: match (look.alpha_explicit, look.material.alpha_mode, blend) {
                        // (nothing to see through: what blends would leave no
                        // depth, and show its far side)
                        (true, 1, false) => 0,
                        (true, mode, _) => mode,
                        (false, _, true) => 1,
                        (false, _, false) => 2,
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
        skeleton: Option<Vec<Mat4>>,
        rig: &crate::scene::anim::Rig,
        own: Vec<Own>,
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
            pelvis: rig.pelvis,
            joints: rig.len(),
            own,
            still: Arc::from([]),
            images,
            meshes,
        };
        let empty = Vec::new();
        model.still = model.bones(skeleton.as_ref().unwrap_or(&empty));
        if skeleton.is_none() {
            // objects of the world, side by side
            let shown: Vec<usize> = (0..model.groups.len()).filter(|g| !model.groups[*g].units.is_empty()).collect();
            let boxes: Vec<Bounds> = shown.iter().map(|g| model.group_bounds(*g, &model.still)).collect();
            for (g, by) in shown.into_iter().zip(layout::side_by_side(&boxes)) {
                for own in model.own.iter_mut().filter(|o| o.group == g) {
                    if let Driver::Still(at) = &mut own.place {
                        *at = Mat4::from_translation(by) * *at;
                    }
                }
                for unit in model.groups[g].units.clone() {
                    for bone in model.units[unit].bones.clone() {
                        if let Driver::Still(at) = &mut model.drivers[bone] {
                            *at = Mat4::from_translation(by) * *at;
                        }
                    }
                }
            }
            model.still = model.bones(&empty);
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

/// Two sets of joints mixed: `t` 0 the first, 1 the second.
pub fn mixed(a: &[Mat4], b: &[Mat4], t: f32) -> Vec<Mat4> {
    if a.len() != b.len() {
        return b.to_vec();
    }
    a.iter().zip(b).map(|(a, b)| *a * (1.0 - t) + *b * t).collect()
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

/// The frame of a moving avatar: held by its body, so that nothing that moves
/// on it (a tail, wings, arms, a swaying hip) moves the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steady {
    pub frame: Frame,
    /// Where the body is for the frame to stay as it is.
    body: Vec3,
}

impl Steady {
    /// The body in the middle, with what shows on either side of it.
    pub fn of(bounds: &Bounds, body: Vec3) -> Steady {
        let mut frame = Frame::of(*bounds);
        if !bounds.empty() {
            let inside = |v: f32, lo: f32, hi: f32| v.clamp(lo, hi);
            frame.center.x = inside(body.x, bounds.lo.x, bounds.hi.x);
            frame.center.y = inside(body.y, bounds.lo.y, bounds.hi.y);
            let half = (bounds.hi - frame.center).max(frame.center - bounds.lo);
            // (a long tail may reach past the view rather than make the
            // avatar small)
            frame.radius = half.length().clamp(frame.radius, frame.radius * 1.2);
        }
        Steady { frame, body }
    }

    /// What is shown now is of another size (more of it has arrived).
    pub fn outgrown(&self, bounds: &Bounds) -> bool {
        let now = Frame::of(*bounds).radius;
        now > self.frame.radius * 1.12 || now < self.frame.radius * 0.6
    }

    /// Move `ease` (0 to 1) of the way after a body that left its place or
    /// its ground.
    pub fn follow(&mut self, bounds: &Bounds, body: Vec3, ease: f32) {
        let away = body - self.body;
        let past = away.length() - self.frame.radius * 0.3;
        if past > 0.0 {
            let by = away.normalize() * past * ease;
            self.frame.center += by;
            self.body += by;
            self.frame.low += by.z;
        }
        let now = Frame::of(*bounds);
        let slack = self.frame.radius * 0.25;
        let low = self.frame.low.clamp(now.low - slack, now.low + slack);
        self.frame.low += (low - self.frame.low) * ease;
        let radius = self.frame.radius.max(now.radius / 1.5);
        self.frame.radius += (radius - self.frame.radius) * ease;
    }
}

impl StageModel {
    /// Every bone, for the joints of `skeleton`: the wearer's, then those of
    /// each animesh (those at rest for the ones it lacks).
    pub fn bones(&self, skeleton: &[Mat4]) -> Arc<[Mat4]> {
        let place = |driver: &Driver| match driver {
            Driver::Still(at) => *at,
            Driver::Held(hold) => hold.on(skeleton, self.pelvis),
            Driver::Joint(joint) => skeleton.get(*joint).copied().unwrap_or(Mat4::IDENTITY),
            Driver::Own(..) => Mat4::IDENTITY,
        };
        let places: Vec<Mat4> = self.own.iter().map(|o| place(&o.place) * o.after).collect();
        self.drivers
            .iter()
            .map(|driver| match driver {
                Driver::Own(set, joint) => {
                    let live = skeleton.get(self.joints * (set + 1) + joint);
                    let joint = live.or_else(|| self.own[*set].still.get(*joint));
                    places[*set] * joint.copied().unwrap_or(Mat4::IDENTITY)
                }
                driver => place(driver),
            })
            .collect()
    }

    /// The animesh shown, by their roots, and their joints at rest.
    pub fn own(&self) -> impl Iterator<Item = (&Uuid, &[Mat4])> {
        self.own.iter().map(|o| (&o.root, o.still.as_slice()))
    }

    /// Joints of the wearer, the first in what `bones` is given.
    pub fn wearer_joints(&self) -> usize {
        self.joints
    }

    /// Where the body is: its pelvis.
    pub fn body(&self, skeleton: &[Mat4]) -> Vec3 {
        skeleton.get(self.pelvis).map_or(Vec3::ZERO, |m| m.w_axis.truncate())
    }

    /// The bones at rest.
    pub fn still(&self) -> &Arc<[Mat4]> {
        &self.still
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

    /// The box of everything.
    pub fn bounds(&self, bones: &[Mat4]) -> Bounds {
        let boxes: Vec<Bounds> = (0..self.units.len()).map(|u| self.unit_bounds(u, bones)).collect();
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
