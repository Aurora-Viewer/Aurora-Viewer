//! The selection's prims for an export: which ones (the menu's selection,
//! ColladaExportFloater::addSelectedObjects), what the rights read of each
//! (`rights::PrimFacts`), and their geometry, copied on the frame thread
//! then generated on a worker (`generate`).

use super::model::{SourceFace, SourcePrim};
use super::rights::{PrimFacts, Shape};
use crate::build::BuildTool;
use crate::scene::Scene;
use crate::scene::jobs::SculptMap;
use crate::scene::meshes::MeshView;
use crate::world::World;
use crate::world::objects::ObjKey;
use aurora_net::build::ObjectProps;
use aurora_prim::params::VolumeParams;
use aurora_prim::te::TextureFace;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// The prims a menu on `idx` exports: those of the selection when `idx`
/// is in it (every prim of each linkset; only the selected prims with
/// "Edit linked"), else `idx`'s own linkset. The first is the root the
/// file is centred on.
pub fn prims_of(build: &BuildTool, world: &World, idx: usize) -> Vec<ObjKey> {
    let key = world.objects.get(idx).map(|o| o.key);
    let in_selection = build.selection_linked && key.is_some_and(|k| build.selection.contains(&k));
    let indices: Vec<usize> = if in_selection {
        build.selection.iter().filter_map(|k| world.objects.index_of(k)).collect()
    } else {
        build
            .menu_roots(world, idx)
            .into_iter()
            .flat_map(|root| crate::build::family(world, root))
            .collect()
    };
    let mut seen = std::collections::HashSet::new();
    indices
        .into_iter()
        .filter_map(|i| world.objects.get(i))
        .filter(|o| !o.is_avatar())
        .map(|o| o.key)
        .filter(|k| seen.insert(*k))
        .collect()
}

/// What a prim still waits for before the rule can be read or its
/// geometry made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Its ObjectProperties (creator, owner, masks).
    Properties,
    /// Its mesh header or the mesh's highest detail held.
    Mesh,
    /// Its sculpt map's pixels, or its comment.
    Sculpt,
}

/// The rule's facts of a prim, or what they wait for; `None` for no
/// volume (a tree, grass: getVolume() is null, never exported).
pub fn facts(
    world: &World,
    scene: &Scene,
    key: ObjKey,
    props: &dyn Fn(Uuid) -> Option<ObjectProps>,
    comments: &HashMap<Uuid, Option<Uuid>>,
) -> Option<Result<PrimFacts, Waiting>> {
    let o = world.objects.get(world.objects.index_of(&key)?)?;
    if o.pcode != aurora_prim::params::LL_PCODE_VOLUME || o.is_tree() {
        return None;
    }
    let Some(p) = props(o.full_id).filter(|p| p.full) else {
        return Some(Err(Waiting::Properties));
    };
    let shape = match o.volume.sculpt {
        Some(s) if s.is_mesh() => Shape::Mesh {
            creator: scene.meshes.view(&s.texture).map(|v| v.header.creator),
        },
        Some(s) => {
            let Some(uploader) = comments.get(&s.texture) else {
                return Some(Err(Waiting::Sculpt));
            };
            Shape::Sculpt {
                map: s.texture,
                uploader: *uploader,
            }
        }
        None => Shape::Prim,
    };
    Some(Ok(PrimFacts {
        creator: p.creator_id,
        owner: p.owner_id,
        owner_mask: p.owner_mask,
        everyone_mask: p.everyone_mask,
        group_owned: !p.owner_id.is_nil() && p.owner_id == p.group_id,
        scale: o.scale,
        shape,
    }))
}

/// A prim's geometry as held, to make on a worker.
pub enum Geometry {
    Volume(VolumeParams),
    Sculpt(VolumeParams, Arc<SculptMap>),
    Mesh(MeshView),
}

/// What `generate` needs of a prim.
pub struct PrimInput {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub faces: Vec<TextureFace>,
    pub geometry: Geometry,
}

/// The highest detail of a prim's geometry, copied from the scene, or what
/// it waits for (and asked for: the mesh's highest level of detail, the
/// sculpt map's pixels). The position and rotation are the drawn ones
/// (getRenderPosition, getRenderRotation), the scale the network's.
pub fn input(world: &World, scene: &mut Scene, key: ObjKey, now: Instant) -> Result<PrimInput, Waiting> {
    let idx = world.objects.index_of(&key).ok_or(Waiting::Properties)?;
    let o = world.objects.get(idx).ok_or(Waiting::Properties)?;
    let (position, rotation, _) = Scene::object_transform(world, idx, now, 0).ok_or(Waiting::Properties)?;
    let volume = o.volume;
    let geometry = match volume.sculpt {
        Some(s) if s.is_mesh() => {
            scene.meshes.want(s.texture, 3);
            let view = scene.meshes.view(&s.texture).ok_or(Waiting::Mesh)?;
            let best = view.header.actual_lod(3).and_then(|l| view.header.lods[l]);
            if !best.is_some_and(|range| view.bytes.holds(range)) {
                return Err(Waiting::Mesh);
            }
            Geometry::Mesh(view)
        }
        Some(s) => {
            scene.textures.want_sculpt(&s.texture);
            Geometry::Sculpt(volume, scene.textures.sculpt_map(&s.texture).ok_or(Waiting::Sculpt)?)
        }
        None => Geometry::Volume(volume),
    };
    let te = o.te.clone();
    let count = match &geometry {
        Geometry::Mesh(_) => aurora_prim::te::MAX_TES,
        _ => aurora_prim::volume::num_faces(&volume).max(1),
    };
    Ok(PrimInput {
        position,
        rotation,
        scale: o.scale,
        faces: (0..count).map(|i| te.as_ref().map(|te| *te.face(i)).unwrap_or_default()).collect(),
        geometry,
    })
}

/// The prim's faces at the highest detail (DETAIL_SCALES[3], the mesh's
/// high LOD or the finest held): heavy, on a worker.
pub fn generate(input: PrimInput) -> SourcePrim {
    let detail = aurora_prim::volume::DETAIL_SCALES[3];
    let te = |i: usize| input.faces.get(i).copied().unwrap_or_default();
    let faces = match &input.geometry {
        Geometry::Volume(params) => volume_faces(aurora_prim::generate_volume(params, detail), &te),
        Geometry::Sculpt(params, map) => volume_faces(
            aurora_prim::generate_sculpt(params, detail, map.width, map.height, map.components, &map.pixels),
            &te,
        ),
        Geometry::Mesh(view) => match view.lod_section(3).map(aurora_assets::mesh::decode_mesh_lod) {
            Some(Ok(faces)) => faces
                .into_iter()
                .enumerate()
                .map(|(i, f)| SourceFace {
                    positions: f.positions,
                    normals: f.normals,
                    uvs: f.uvs,
                    indices: f.indices.into_iter().map(u32::from).collect(),
                    te: te(i),
                })
                .collect(),
            Some(Err(e)) => {
                log::warn!("object export: mesh LOD unreadable: {e}");
                Vec::new()
            }
            None => Vec::new(),
        },
    };
    SourcePrim {
        position: input.position,
        rotation: input.rotation,
        scale: input.scale,
        faces,
    }
}

fn volume_faces(mesh: aurora_prim::VolumeMesh, te: &dyn Fn(usize) -> TextureFace) -> Vec<SourceFace> {
    mesh.faces
        .into_iter()
        .enumerate()
        .map(|(i, f)| SourceFace {
            positions: f.positions,
            normals: f.normals,
            uvs: f.uvs,
            indices: f.indices.into_iter().map(u32::from).collect(),
            te: te(i),
        })
        .collect()
}
