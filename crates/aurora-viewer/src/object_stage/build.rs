//! What the 3D view is made from, taken on the main thread (`Input`), and the
//! model made of it on a worker (`StageModel`).

use super::layout::{self, Bounds};
use super::{Group, Place};
use crate::scene::Scene;
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
struct Hold {
    joint: Option<usize>,
    point: Vec3,
    rotation: Quat,
    /// The linkset's transforms, with the prim's scale.
    local: Mat4,
}

struct RawPrim {
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
    pub bounds: Bounds,
    bones: Range<usize>,
    parts: Range<usize>,
}

pub struct GroupShape {
    pub root: Uuid,
    pub units: Range<usize>,
    pub bounds: Bounds,
    /// What it moves by once the whole is pulled apart.
    away: Vec3,
    /// Its box once its own parts are pulled apart.
    apart: Bounds,
}

pub struct StageModel {
    pub model: Arc<Model>,
    pub units: Vec<Unit>,
    /// In the order asked for; one with nothing to draw has no units.
    pub groups: Vec<GroupShape>,
    /// Every bone, at rest in the model's space.
    base: Vec<Mat4>,
    /// What each unit moves by once its own group is pulled apart.
    unit_away: Vec<Vec3>,
    together: Bounds,
    spread: Bounds,
    /// The triangles as they stand together, for picking.
    skinned: Vec<Arc<[Vec3]>>,
    images: HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
    meshes: HashMap<(Uuid, usize), Arc<Vec<aurora_assets::MeshFace>>>,
}

/// How the model stands in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Standing {
    /// 0 together, 1 pulled apart.
    pub apart: f32,
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
        let skeleton = self.worn.then(|| stance(lib, &self.prims));
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
            out.prim(prim, drawn, skin, skeleton.as_deref(), rig, &images);
        }
        out.finish(self.roots, self.worn, images, meshes)
    }
}

/// The joints of a neutral stance, with what the worn meshes ask of them.
fn stance(lib: &AvatarLibrary, prims: &[RawPrim]) -> Vec<Mat4> {
    let rig = &lib.rig;
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
    parts: Vec<Part>,
    units: Vec<Unit>,
    base: Vec<Mat4>,
    skinned: Vec<Arc<[Vec3]>>,
    vertices: usize,
}

impl Building {
    fn prim(
        &mut self,
        prim: &RawPrim,
        drawn: Vec<Drawn>,
        rigged: Option<&SkinInfo>,
        skeleton: Option<&[Mat4]>,
        rig: &crate::scene::anim::Rig,
        images: &HashMap<Uuid, (PreviewRevision, Arc<Image>)>,
    ) {
        let (first_bone, first_part) = (self.base.len(), self.parts.len());
        let count: usize = drawn.iter().map(|d| d.vertices.len()).sum();
        if self.vertices + count > MAX_VERTICES {
            return;
        }
        let mut bounds = Bounds::default();
        let rigged = rigged.filter(|_| skeleton.is_some());
        // a rigged mesh follows the stance, a bone per joint of its skin;
        // anything else has one bone of its own, at the place it is held
        let skin = match (rigged, skeleton) {
            (Some(info), Some(skeleton)) => {
                let mut inverse_bind = Vec::with_capacity(info.joint_names.len());
                for (name, matrix) in info.joint_names.iter().zip(&info.inverse_bind) {
                    let (joint, matrix) = rig.skin_bind(name, *matrix);
                    self.base.push(skeleton.get(joint).copied().unwrap_or(Mat4::IDENTITY));
                    inverse_bind.push(matrix * info.bind_shape);
                }
                inverse_bind
            }
            _ => {
                let held = match skeleton {
                    Some(skeleton) => {
                        // mRoot (Avatar Center) is outside the skeleton: the
                        // pelvis' height, as the Shape editor's hints
                        let joint = prim.hold.joint.and_then(|j| skeleton.get(j)).copied().unwrap_or_else(|| {
                            Mat4::from_translation(skeleton.get(rig.pelvis).map_or(Vec3::ZERO, |m| m.w_axis.truncate()))
                        });
                        let (_, rotation, _) = joint.to_scale_rotation_translation();
                        Mat4::from_rotation_translation(rotation.normalize() * prim.hold.rotation, joint.transform_point3(prim.hold.point))
                            * prim.hold.local
                    }
                    None => prim.hold.local,
                };
                self.base.push(held);
                vec![Mat4::IDENTITY]
            }
        };
        if skin.is_empty() {
            return;
        }
        let bones = first_bone..self.base.len();
        let rest: Vec<Mat4> = bones.clone().zip(&skin).map(|(b, inverse)| self.base[b] * *inverse).collect();
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
            let posed: Arc<[Vec3]> = vertices.iter().map(|v| skinned(v, &rest)).collect();
            if !posed.iter().all(|p| p.is_finite()) {
                continue;
            }
            posed.iter().for_each(|p| bounds.add(*p));
            let textures = look.textures.map(|id| images.get(&id).map(|(_, image)| image.clone()));
            let blend = look.material.color[3] < 1.0
                || textures[0]
                    .as_ref()
                    .is_some_and(|image| image.rgba.iter().skip(3).step_by(4).any(|a| *a != 0 && *a != 255));
            self.vertices += vertices.len();
            self.skinned.push(posed);
            self.parts.push(Part {
                diagnostic: false,
                vertices,
                indices: d.indices,
                textures,
                material: Material {
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
        if self.parts.len() == first_part || bounds.empty() {
            // nothing of it shows
            self.parts.truncate(first_part);
            self.skinned.truncate(first_part);
            self.base.truncate(first_bone);
            return;
        }
        self.units.push(Unit {
            id: prim.id,
            group: prim.group,
            bounds,
            bones,
            parts: first_part..self.parts.len(),
        });
    }

    fn finish(
        mut self,
        roots: Vec<Uuid>,
        worn: bool,
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
        let boxes_of = |units: &[Unit], r: Range<usize>| units[r].iter().map(|u| u.bounds).collect::<Vec<_>>();
        if !worn {
            // objects of the world, side by side
            let ranges: Vec<Range<usize>> = (0..roots.len()).map(|g| range(&self.units, g)).filter(|r| !r.is_empty()).collect();
            let boxes: Vec<Bounds> = ranges.iter().map(|r| layout::whole(&boxes_of(&self.units, r.clone()))).collect();
            for (r, by) in ranges.into_iter().zip(layout::side_by_side(&boxes)) {
                for unit in &mut self.units[r] {
                    unit.bounds = unit.bounds.moved(by);
                    for bone in unit.bones.clone() {
                        self.base[bone] = Mat4::from_translation(by) * self.base[bone];
                    }
                    for part in unit.parts.clone() {
                        self.skinned[part] = self.skinned[part].iter().map(|p| *p + by).collect();
                    }
                }
            }
        }
        let mut unit_away = vec![Vec3::ZERO; self.units.len()];
        let mut groups: Vec<GroupShape> = roots
            .into_iter()
            .enumerate()
            .map(|(g, root)| {
                let units = range(&self.units, g);
                let boxes = boxes_of(&self.units, units.clone());
                let mut apart = Bounds::default();
                for ((u, b), away) in units.clone().zip(&boxes).zip(layout::parts_apart(&boxes)) {
                    unit_away[u] = away;
                    apart.join(b.moved(away));
                }
                GroupShape {
                    root,
                    units,
                    bounds: layout::whole(&boxes),
                    away: Vec3::ZERO,
                    apart,
                }
            })
            .collect();
        let present: Vec<usize> = (0..groups.len()).filter(|g| !groups[*g].units.is_empty()).collect();
        let boxes: Vec<Bounds> = present.iter().map(|g| groups[*g].bounds).collect();
        let together = layout::whole(&boxes);
        let aways: Vec<Vec3> = if worn {
            layout::worn_apart(&boxes)
        } else {
            boxes
                .iter()
                .map(|b| (b.center() - together.center()) * Vec3::new(1.0, 0.5, 0.3))
                .collect()
        };
        let mut spread = Bounds::default();
        for ((g, b), away) in present.iter().zip(&boxes).zip(aways) {
            groups[*g].away = away;
            spread.join(b.moved(away));
        }
        Some(StageModel {
            model: Arc::new(Model {
                parts: self.parts,
                // the palette brings everything into the unit sphere
                center: Vec3::ZERO,
                radius: 1.0,
            }),
            units: self.units,
            groups,
            base: self.base,
            unit_away,
            together,
            spread,
            skinned: self.skinned,
            images,
            meshes,
        })
    }
}

fn skinned(v: &Vertex, bones: &[Mat4]) -> Vec3 {
    let p = Vec3::from_array(v.position);
    let (mut out, mut sum) = (Vec3::ZERO, 0.0);
    for (joint, weight) in v.joints.iter().zip(v.weights) {
        if weight > 0.0
            && let Some(bone) = bones.get((*joint as usize).min(bones.len() - 1))
        {
            out += bone.transform_point3(p) * weight;
            sum += weight;
        }
    }
    if sum > 1e-5 { out / sum } else { p }
}

fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t
}

impl StageModel {
    /// The sphere the view shows: its centre and radius.
    fn framed(&self, standing: Standing) -> (Vec3, f32) {
        let mix = |a: &Bounds, b: &Bounds, t: f32| (lerp(a.center(), b.center(), t), a.radius() + (b.radius() - a.radius()) * t);
        let (center, radius) = mix(&self.together, &self.spread, standing.apart);
        match standing.focus.and_then(|(g, f)| Some((self.groups.get(g)?, f))) {
            Some((group, f)) if !group.units.is_empty() => {
                let (c, r) = mix(&group.bounds, &group.apart, standing.apart);
                (lerp(center, c, f), radius + (r - radius) * f)
            }
            _ => (center, radius),
        }
    }

    /// What a unit moves by.
    fn moved(&self, unit: usize, standing: Standing) -> Vec3 {
        let group = self.units[unit].group;
        let (mine, f) = standing.focus.map_or((false, 0.0), |(g, f)| (g == group, f));
        let whole = self.groups[group].away * standing.apart * (1.0 - f);
        let own = if mine {
            self.unit_away[unit] * standing.apart * f
        } else {
            Vec3::ZERO
        };
        whole + own
    }

    /// How large a group shows: the others shrink away from one alone.
    fn shown(&self, group: usize, standing: Standing) -> f32 {
        match standing.focus {
            Some((g, f)) if g != group => 1.0 - f,
            _ => 1.0,
        }
    }

    pub fn pose(&self, standing: Standing) -> Pose {
        let (center, radius) = self.framed(standing);
        let view = Mat4::from_scale(Vec3::splat(1.0 / radius)) * Mat4::from_translation(-center);
        let mut out = vec![Mat4::IDENTITY.to_cols_array_2d(); self.base.len()];
        for (u, unit) in self.units.iter().enumerate() {
            let middle = self.groups[unit.group].bounds.center();
            // (never quite 0: a matrix without an inverse has no normals)
            let size = self.shown(unit.group, standing).max(1e-4);
            let m = view
                * Mat4::from_translation(self.moved(u, standing) + middle)
                * Mat4::from_scale(Vec3::splat(size))
                * Mat4::from_translation(-middle);
            for bone in unit.bones.clone() {
                out[bone] = (m * self.base[bone]).to_cols_array_2d();
            }
        }
        out.into()
    }

    /// A tone for each part, by its unit.
    pub fn tones(&self, tone: impl Fn(&Unit) -> [f32; 4]) -> aurora_render::preview::Tones {
        let mut out = vec![[0.0; 4]; self.model.parts.len()];
        for unit in &self.units {
            out[unit.parts.clone()].fill(tone(unit));
        }
        out.into()
    }

    /// The prim a ray of the view meets first, as the model stands.
    pub fn pick(&self, standing: Standing, from: Vec3, dir: Vec3) -> Option<&Unit> {
        let (center, radius) = self.framed(standing);
        // back into the model's own space
        let from = from * radius + center;
        let mut best: Option<(f32, usize)> = None;
        for (u, unit) in self.units.iter().enumerate() {
            if self.shown(unit.group, standing) < 0.5 {
                continue;
            }
            let start = from - self.moved(u, standing);
            let near = |t: f32| best.is_none_or(|(b, _)| t < b);
            if !unit.bounds.hit(start, dir).is_some_and(near) {
                continue;
            }
            for part in unit.parts.clone() {
                let points = &self.skinned[part];
                for tri in self.model.parts[part].indices.as_chunks::<3>().0 {
                    let Some([a, b, c]) = corner(points, tri) else { continue };
                    if let Some(t) = triangle(start, dir, a, b, c)
                        && best.is_none_or(|(b, _)| t < b)
                    {
                        best = Some((t, u));
                    }
                }
            }
        }
        best.map(|(_, u)| &self.units[u])
    }

    /// How many parts a group can be pulled into.
    pub fn parts_of(&self, root: &Uuid) -> usize {
        self.groups.iter().find(|g| g.root == *root).map_or(0, |g| g.units.len())
    }

    /// How many groups have something to draw.
    pub fn shown_groups(&self) -> usize {
        self.groups.iter().filter(|g| !g.units.is_empty()).count()
    }
}

fn corner(points: &[Vec3], tri: &[u32; 3]) -> Option<[Vec3; 3]> {
    Some([
        *points.get(tri[0] as usize)?,
        *points.get(tri[1] as usize)?,
        *points.get(tri[2] as usize)?,
    ])
}

/// Möller–Trumbore, either side of the triangle.
fn triangle(from: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = from - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t)
}
