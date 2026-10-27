//! Selected collision geometry, following `renderOnePhysicsShape` and
//! `LLPhysicsShapeBuilderUtil::determinePhysicsShape` (indra/newview,
//! originally LGPL 2.1). Decode/triangulate on rayon, then draw on the GPU;
//! visual LODs, face materials, picking and shadows remain independent.

use super::jobs::{FaceGeom, mesh_to_faces, volume_to_faces};
use super::meshes::{MeshStreamer, MeshView};
use crate::world::{World, objects::ObjKey};
use aurora_assets::{AssetError, MeshFace, mesh_physics};
use aurora_prim::{VolumeParams, params::*};
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, Renderer};
use crossbeam_channel::{Receiver, TryRecvError};
use glam::{Mat4, Vec3};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    volume: u64,
    scale: [u32; 3],
    kind: u8,
}

enum Geometry {
    Pending(Receiver<Result<Vec<FaceGeom>, AssetError>>),
    Ready(Vec<MeshAlloc>),
}

#[derive(Default)]
pub struct PhysicsShapes {
    geometry: HashMap<Key, Geometry>,
    records: HashMap<ObjKey, u32>,
    active: Vec<(Key, u32)>,
    wanted: HashSet<Uuid>,
}

/// No collision overlay for unknown properties, None, flexible prims or HUDs.
/// The simulator's shape type is required: a visual mesh cannot substitute.
pub fn selection(tool: &crate::build::BuildTool, world: &World, settings: &crate::build::BuildSettings) -> Vec<(usize, u8)> {
    if !settings.show_physics_shape || !tool.open || tool.tool != crate::build::Tool::Edit || tool.hud_selected(world) {
        return Vec::new();
    }
    tool.sel_prims(world)
        .into_iter()
        .filter_map(|idx| {
            let object = world.objects.get(idx)?;
            let kind = tool.physics.get(&object.key.local_id)?.shape_type;
            (matches!(kind, 0 | 2)
                && object.pcode == LL_PCODE_VOLUME
                && object.extra.flexible.is_none()
                && object.volume.path.curve_type != LL_PCODE_PATH_FLEXIBLE)
                .then_some((idx, kind))
        })
        .collect()
}

impl PhysicsShapes {
    pub fn update(&mut self, renderer: &mut Renderer, world: &World, meshes: &mut MeshStreamer, selected: &[(usize, u8)]) {
        self.active.clear();
        let mut records = HashSet::new();
        let mut wanted = HashSet::new();
        let mut used = HashSet::new();
        let now = Instant::now();
        for &(idx, kind) in selected {
            let Some(object) = world.objects.get(idx) else { continue };
            let Some((position, rotation, false)) = super::Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let params = object.volume;
            // The reference editor does not draw traditional sculpt physics.
            if params.sculpt.is_some_and(|s| !s.is_mesh()) {
                continue;
            }
            let key = Key {
                volume: params.cache_key(),
                // Mesh collision changes only at convexification thresholds.
                // A scale drag updates its GPU transform without decoding and
                // triangulating every hull again at every pointer movement.
                scale: if params.is_mesh() {
                    [object.scale.to_array().into_iter().filter(|v| *v < 0.2).count() as u32, 0, 0]
                } else {
                    object.scale.to_array().map(f32::to_bits)
                },
                kind,
            };
            used.insert(key);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.geometry.entry(key) {
                let view = if let Some(sculpt) = params.sculpt {
                    wanted.insert(sculpt.texture);
                    meshes.want_physics(sculpt.texture, true);
                    let Some(view) = meshes.view(&sculpt.texture) else { continue };
                    // Wait for both blocks so a base hull never hides an analysed
                    // decomposition or a still-downloading triangle shape.
                    if [view.header.physics_convex, view.header.physics_mesh]
                        .into_iter()
                        .flatten()
                        .any(|r| view.bytes.section(r).is_none())
                    {
                        continue;
                    }
                    Some(view)
                } else {
                    None
                };
                let scale = object.scale;
                let (send, receive) = crossbeam_channel::bounded(1);
                rayon::spawn(move || {
                    let _ = send.send(prepare(params, scale, kind, view));
                });
                entry.insert(Geometry::Pending(receive));
            } else if let Some(sculpt) = params.sculpt {
                wanted.insert(sculpt.texture);
            }
            let Some(geometry) = self.geometry.get_mut(&key) else { continue };
            if let Geometry::Pending(receive) = geometry {
                match receive.try_recv() {
                    Ok(result) => {
                        let mut uploaded = Vec::new();
                        match result {
                            Ok(faces) => {
                                for face in faces {
                                    if let Some(mesh) = renderer.upload_mesh(&face.vertices, &face.indices) {
                                        uploaded.push(mesh);
                                    } else {
                                        for mesh in uploaded.drain(..) {
                                            renderer.free_mesh(mesh);
                                        }
                                        log::warn!("physics shape: geometry arena full");
                                        break;
                                    }
                                }
                            }
                            Err(error) => log::warn!("physics shape: {error}"),
                        }
                        *geometry = Geometry::Ready(uploaded);
                    }
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => {
                        *geometry = Geometry::Ready(Vec::new());
                        continue;
                    }
                }
            }
            records.insert(object.key);
            let data = DrawRecord {
                model: Mat4::from_scale_rotation_translation(object.scale, rotation, position).to_cols_array_2d(),
                ..Default::default()
            };
            let record = *self.records.entry(object.key).or_insert_with(|| renderer.records.alloc(data));
            renderer.records.set(record, data);
            self.active.push((key, record));
        }
        self.records.retain(|key, record| {
            if records.contains(key) {
                true
            } else {
                renderer.records.free(*record);
                false
            }
        });
        for id in self.wanted.difference(&wanted) {
            meshes.want_physics(*id, false);
        }
        self.wanted = wanted;
        // Retain a small cache across eye toggles; never grow with a long edit
        // session or keep obsolete drag scales indefinitely.
        if self.geometry.len() > 128 {
            self.geometry.retain(|key, geometry| {
                if used.contains(key) {
                    return true;
                }
                free(renderer, geometry);
                false
            });
        }
    }

    pub fn append(&self, lists: &mut DrawLists) {
        for (key, record) in &self.active {
            let Some(Geometry::Ready(meshes)) = self.geometry.get(key) else {
                continue;
            };
            lists.physics_shapes.extend(meshes.iter().map(|mesh| DrawCmd {
                index_count: mesh.index_count,
                first_index: mesh.index_offset,
                base_vertex: mesh.vertex_offset as i32,
                record: *record,
                bounds: [0.0; 4],
            }));
        }
    }

    pub fn clear(&mut self, renderer: &mut Renderer) {
        for geometry in self.geometry.values() {
            free(renderer, geometry);
        }
        for record in self.records.values() {
            renderer.records.free(*record);
        }
        *self = Self::default();
    }
}

fn free(renderer: &mut Renderer, geometry: &Geometry) {
    if let Geometry::Ready(meshes) = geometry {
        for mesh in meshes {
            renderer.free_mesh(*mesh);
        }
    }
}

fn prepare(params: VolumeParams, scale: Vec3, kind: u8, view: Option<MeshView>) -> Result<Vec<FaceGeom>, AssetError> {
    if let Some(view) = view {
        let faces = mesh_shape(&view, scale, kind)?;
        return Ok(mesh_to_faces(&faces, None, None).0.into_iter().flatten().collect());
    }
    prim_shape(params, scale, kind)
}

fn mesh_shape(view: &MeshView, scale: Vec3, kind: u8) -> Result<Vec<MeshFace>, AssetError> {
    let decomposition = view
        .header
        .physics_convex
        .and_then(|r| view.bytes.section(r))
        .map(mesh_physics::decode)
        .transpose()?
        .unwrap_or_default();
    let small = scale.to_array().into_iter().filter(|v| *v < 0.2).count();
    if small == 3 {
        let volume = aurora_prim::generate_volume(&VolumeParams::default(), 1.0);
        return Ok(volume
            .faces
            .into_iter()
            .map(|f| MeshFace {
                positions: f.positions,
                indices: f.indices,
                ..Default::default()
            })
            .collect());
    }
    if kind != 2 && small <= 1 {
        if !decomposition.hulls.is_empty() {
            return Ok(decomposition.hulls);
        }
        if let Some(section) = view.header.physics_mesh.and_then(|r| view.bytes.section(r)) {
            return aurora_assets::decode_mesh_lod(section);
        }
    }
    Ok(decomposition.base.into_iter().collect())
}

fn prim_shape(mut params: VolumeParams, scale: Vec3, kind: u8) -> Result<Vec<FaceGeom>, AssetError> {
    let avg = scale.element_sum() / 3.0;
    let small = scale.to_array().into_iter().filter(|v| *v < 0.2).count();
    let complete = |begin: f32, end: f32| begin <= 0.05 / avg && end >= 1.0 - 0.05 / avg;
    let profile_complete = complete(params.profile.begin, params.profile.end);
    let path_complete = complete(params.path.begin, params.path.end);
    let simple = params.profile.hollow <= 0.1 / avg && params.path.shear.iter().all(|v| v.abs() <= 0.05 / avg);
    let no_taper = params.path.scale.iter().all(|v| (v - 1.0).abs() <= 0.05 / avg);
    let no_twist = params.path.twist_begin.abs() <= 0.09 / avg && params.path.twist_end.abs() <= 0.09 / avg;
    let mut dimensions = Vec3::ONE;
    let mut center = Vec3::ZERO;
    let mut convex = kind == 2 || small > 1;
    let mut implicit = false;
    if simple && profile_complete {
        if params.profile.curve_type == LL_PCODE_PROFILE_SQUARE && params.path.curve_type == LL_PCODE_PATH_LINE && no_taper && no_twist {
            if !path_complete {
                dimensions = scale.max(Vec3::splat(0.05)) / scale;
                dimensions.z = (scale.z * (params.path.end - params.path.begin)).max(0.05) / scale.z;
                center.z = 0.5 * (params.path.end + params.path.begin - 1.0);
            }
            params = VolumeParams::default();
            implicit = true;
        } else if params.profile.curve_type == LL_PCODE_PROFILE_CIRCLE_HALF
            && params.path.curve_type == LL_PCODE_PATH_CIRCLE
            && path_complete
            && params.path.taper.iter().all(|v| v.abs() <= 0.05 / avg)
            && no_twist
        {
            if scale.x == scale.z && scale.y == scale.z {
                params = crate::build::shapes::shapes()[8].params();
                implicit = true;
            } else if small > 1 {
                params = VolumeParams::default();
                dimensions = scale.map(|v| if v < 0.2 { 0.75 } else { 1.0 });
                implicit = true;
            }
        } else if params.profile.curve_type == LL_PCODE_PROFILE_CIRCLE
            && params.path.curve_type == LL_PCODE_PATH_LINE
            && scale.x == scale.y
            && no_taper
        {
            if small > 1 {
                params = VolumeParams::default();
                dimensions = scale.map(|v| if v < 0.2 { 0.75 } else { 1.0 });
            } else {
                dimensions.z = params.path.end - params.path.begin;
                center.z = 0.5 * (params.path.end + params.path.begin - 1.0);
                params = VolumeParams {
                    profile: aurora_prim::params::ProfileParams {
                        curve_type: LL_PCODE_PROFILE_CIRCLE,
                        ..Default::default()
                    },
                    ..Default::default()
                };
            }
            implicit = true;
        }
    }
    if !implicit {
        if small == 3 || (small > 1 && profile_complete && path_complete && params.path.curve_type == LL_PCODE_PATH_LINE) {
            params = VolumeParams::default();
            convex = false;
        } else if small == 1 && avg > 3.0 {
            dimensions = scale.max(Vec3::splat(0.1)) / scale;
        }
    } else {
        convex = false;
    }
    let detail = if implicit {
        3
    } else {
        1 + usize::from(avg > 5.0) + usize::from(avg > 25.0)
    };
    let mut mesh = aurora_prim::generate_volume(&params, aurora_prim::volume::DETAIL_SCALES[detail]);
    for face in &mut mesh.faces {
        for p in &mut face.positions {
            *p = (Vec3::from_array(*p) * dimensions + center).to_array();
        }
    }
    if convex {
        let hull = mesh_physics::convex_hull(mesh.faces.iter().flat_map(|f| f.positions.iter().copied()))?;
        Ok(mesh_to_faces(&[hull], None, None).0.into_iter().flatten().collect())
    } else {
        Ok(volume_to_faces(&mesh).0.into_iter().flatten().collect())
    }
}

/// A real encoded mesh asset for offline captures: a ring with a separate
/// triangular collision box, or two analysed hulls; both carry a base hull.
pub fn demo_asset(analysed: bool) -> Result<Vec<u8>, AssetError> {
    use aurora_assets::import_mesh::{Face, Model, Vertex, encode, process};
    let model = |params: VolumeParams, size: Vec3, offset: Vec3| {
        let mesh = aurora_prim::generate_volume(&params, 1.0);
        let mut face = Face {
            vertices: Vec::new(),
            indices: Vec::new(),
            material: 0,
        };
        for source in mesh.faces {
            let base = face.vertices.len() as u32;
            face.vertices.extend(source.positions.iter().map(|p| Vertex {
                position: (Vec3::from_array(*p) * size + offset).to_array(),
                normal: [0.0, 0.0, 1.0],
                uv: [0.0; 2],
                joints: [0; 4],
                weights: [0.0; 4],
            }));
            face.indices.extend(source.indices.iter().map(|i| base + *i as u32));
        }
        Model {
            name: "Collision display fixture".into(),
            submodel: 0,
            faces: vec![face],
            skin: None,
            center: Vec3::ZERO,
            size: Vec3::ONE,
        }
    };
    let mut ring = crate::build::shapes::shapes()[10].params();
    ring.path.scale = [0.45, 0.2];
    let visual = model(ring, Vec3::ONE, Vec3::ZERO);
    let collision = model(VolumeParams::default(), Vec3::new(0.9, 0.9, 0.12), Vec3::new(0.0, 0.0, 0.2));
    let mut physics = process::Physics {
        base: Some(process::base_hull(&visual)?),
        ..Default::default()
    };
    if analysed {
        for x in [-0.25, 0.25] {
            let bar = model(VolumeParams::default(), Vec3::new(0.12, 0.9, 0.4), Vec3::new(x, 0.0, 0.0));
            physics.hulls.push(process::base_hull(&bar)?);
        }
    }
    encode::asset(
        &[None, None, None, Some(visual)],
        Some(&collision),
        &physics,
        encode::Options::default(),
    )
}

pub fn install_demo(meshes: &mut MeshStreamer, analysed: bool) {
    let result = demo_asset(analysed).and_then(|bytes| {
        let header = aurora_assets::parse_mesh_header(&bytes)?;
        meshes.insert_local(crate::demo::FACE_SELECTION_MESH, std::sync::Arc::new(bytes), header, None);
        Ok(())
    });
    if let Err(error) = result {
        log::warn!("demo physics shape: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildSettings, BuildTool, Tool};
    use std::sync::Arc;

    fn view(analysed: bool) -> MeshView {
        let bytes = demo_asset(analysed).unwrap();
        let header = aurora_assets::parse_mesh_header(&bytes).unwrap();
        MeshView {
            header,
            bytes: super::super::mesh_bytes::MeshBytes::whole(Arc::new(bytes)),
            skin: None,
        }
    }

    #[test]
    fn physics_shape_uses_collision_triangles_or_analysed_hulls_and_convex_uses_base() {
        let triangular = view(false);
        assert!(triangular.header.physics_mesh.is_some());
        let prim = mesh_shape(&triangular, Vec3::splat(2.0), 0).unwrap();
        let convex = mesh_shape(&triangular, Vec3::splat(2.0), 2).unwrap();
        assert_eq!(prim.len(), 1);
        assert!(prim[0].positions.iter().all(|p| p[2] > 0.13 && p[2] < 0.27));
        assert!(convex[0].positions.iter().any(|p| p[2] < 0.0));
        let analysed = view(true);
        assert!(analysed.header.physics_mesh.is_none());
        assert_eq!(mesh_shape(&analysed, Vec3::splat(2.0), 0).unwrap().len(), 2);
        assert_eq!(mesh_shape(&analysed, Vec3::splat(2.0), 2).unwrap().len(), 1);
    }

    #[test]
    fn physics_shape_toggle_close_none_flexible_and_linked_selection() {
        let mut world = World::new(Arc::new(super::super::avatar::AvatarLibrary::load()));
        for event in crate::demo::events()
            .into_iter()
            .chain(crate::demo::complex_face_selection_events())
        {
            world.apply(event);
        }
        let idx = world.objects.iter().find(|(_, o)| o.key.local_id == 99620).unwrap().0;
        let mut tool = BuildTool::default();
        let mut settings = BuildSettings::default();
        tool.open_build(Tool::Edit);
        tool.click_select(&mut world, &settings, Some(idx), false);
        for id in [99620, 99621] {
            tool.physics.insert(id, aurora_net::build::PhysicsParams::default());
        }
        assert!(selection(&tool, &world, &settings).is_empty());
        settings.show_physics_shape = true;
        assert_eq!(selection(&tool, &world, &settings).len(), 2);
        tool.physics.get_mut(&99621).unwrap().shape_type = 1;
        assert_eq!(selection(&tool, &world, &settings), [(idx, 0)]);
        tool.set_edit_linked(&mut world, &mut settings, true);
        assert_eq!(selection(&tool, &world, &settings), [(idx, 0)]);
        world.objects.get_mut(idx).unwrap().volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
        assert!(selection(&tool, &world, &settings).is_empty());
        world.objects.get_mut(idx).unwrap().volume.path.curve_type = LL_PCODE_PATH_LINE;
        tool.close(&mut world, &mut settings);
        assert!(selection(&tool, &world, &settings).is_empty());
    }

    #[test]
    fn physics_shape_convexification_fills_a_hollow_prim_and_cut_box_keeps_its_center() {
        let mut params = VolumeParams::default();
        params.profile.hollow = 0.6;
        let prim = prim_shape(params, Vec3::splat(2.0), 0).unwrap();
        let hull = prim_shape(params, Vec3::splat(2.0), 2).unwrap();
        assert!(
            prim.iter()
                .flat_map(|f| &f.vertices)
                .any(|v| v.pos[0].abs() < 0.4 && v.pos[1].abs() < 0.4)
        );
        assert!(
            hull.iter()
                .flat_map(|f| &f.vertices)
                .all(|v| v.pos[0].abs() > 0.49 && v.pos[1].abs() > 0.49)
        );
        params.profile.hollow = 0.0;
        params.path.begin = 0.25;
        params.path.end = 0.75;
        let cut = prim_shape(params, Vec3::splat(2.0), 0).unwrap();
        assert!(cut.iter().flat_map(|f| &f.vertices).all(|v| v.pos[2].abs() <= 0.25001));
    }
}
