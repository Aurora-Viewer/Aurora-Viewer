//! Attachment-driven grounding in a neutral pose. This extends Firestorm's
//! manual LLFloaterHoverHeight / LLVOAvatarSelf::sendHoverHeight: animations
//! never drive the estimate, and only rendered vertices with foot weights count.
use super::{GpuGeom, Scene, anim, avatar::AvatarLibrary};
use crate::world::World;
use aurora_assets::{MeshFace, SkinInfo};
use aurora_render::SkinVertex;
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3};
use rayon::prelude::*;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(200);

pub(super) struct AutoHover {
    observed: Option<u64>,
    changed: Instant,
    finished: Option<u64>,
    pending: bool,
    tx: Sender<(u64, Option<f32>)>,
    rx: Receiver<(u64, Option<f32>)>,
    base: Option<Arc<anim::SkeletonBase>>,
    base_hash: u64,
    matrices: Vec<Mat4>,
}

impl Default for AutoHover {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            observed: None,
            changed: Instant::now(),
            finished: None,
            pending: false,
            tx,
            rx,
            base: None,
            base_hash: 0,
            matrices: Vec::new(),
        }
    }
}

enum Surface {
    Mesh {
        data: Arc<Vec<u8>>,
        skin: Option<Arc<SkinInfo>>,
        rigid: Option<Mat4>,
    },
    Geometry {
        geom: Arc<GpuGeom>,
        model: Mat4,
    },
}

struct Estimate {
    lib: Arc<AvatarLibrary>,
    base: Arc<anim::SkeletonBase>,
    ground: f32,
    surfaces: Vec<Surface>,
}

/// Ankle aliases (lFoot/rFoot), foot, toe and fitted-mesh collision volumes
/// all describe feet. Do not include knees or vertices whose foot weight is zero.
fn foot_joint(rig: &anim::Rig, joint: usize) -> bool {
    [
        "mAnkleLeft",
        "mAnkleRight",
        "mFootLeft",
        "mFootRight",
        "mToeLeft",
        "mToeRight",
        "L_FOOT",
        "R_FOOT",
        "mHindLimb3Left",
        "mHindLimb3Right",
        "mHindLimb4Left",
        "mHindLimb4Right",
    ]
    .iter()
    .any(|name| rig.names.get(*name) == Some(&joint))
}

fn bits(values: &[f32], hash: &mut impl Hasher) {
    for value in values {
        value.to_bits().hash(hash);
    }
}

impl Scene {
    /// Called after the rest skeleton has been refreshed, before scene sync.
    /// Snapshots contain only Arc handles and transforms; decoding, skinning
    /// and vertex scans use the scene's low-priority pool. Old jobs are ignored
    /// after another wear/detach, a HUD update cannot trigger a recalibration.
    pub fn update_auto_hover(&mut self, world: &mut World, enabled: bool, now: Instant) {
        if !enabled {
            self.auto_hover.observed = None;
            self.auto_hover.finished = None;
            while self.auto_hover.rx.try_recv().is_ok() {
                self.auto_hover.pending = false;
            }
            return;
        }
        if !world.hover_height_available() || !world.hover_height.ready() {
            return;
        }
        let Some(owner) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) else {
            return;
        };
        // A seated or flying pose has no useful ground contact. Re-evaluate
        // the pending outfit when back on the ground instead of moving a seat.
        if owner.parent_id != 0 || world.agent.flying {
            return;
        }
        if !owner.scale.z.is_finite() || owner.scale.z <= 0.0 {
            return;
        }
        let Some(skeleton) = self.skeletons.get(&world.agent_id) else {
            return;
        };
        let rig = &self.avatar_lib.rig;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        world.agent_id.hash(&mut hash);
        let state = &mut self.auto_hover;
        if state.base.as_ref().is_none_or(|base| !Arc::ptr_eq(base, &skeleton.base)) {
            if state.base.as_deref() != Some(skeleton.base.as_ref()) {
                let mut base_hash = std::collections::hash_map::DefaultHasher::new();
                for p in skeleton.base.local_pos.iter().chain(&skeleton.base.scale) {
                    bits(&p.to_array(), &mut base_hash);
                }
                state.base_hash = base_hash.finish();
                state.matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &skeleton.base.local_pos, &skeleton.base.scale);
            }
            state.base = Some(skeleton.base.clone());
        }
        state.base_hash.hash(&mut hash);
        bits(&[skeleton.shape_dz, owner.scale.z], &mut hash);
        let matrices = &self.auto_hover.matrices;
        let mut surfaces = Vec::new();
        let mut ready = true;
        for &root_idx in world.objects.children_of(&owner.key) {
            let Some(root) = world.objects.get(root_idx) else { continue };
            let point = root.attachment_point();
            let ap = self.avatar_lib.attach_points.get(&point);
            if root.state == 0 || (31..=38).contains(&point) || ap.is_some_and(|a| a.hud) {
                continue;
            }
            (root.full_id, point).hash(&mut hash);
            let on_foot = ap.and_then(|a| a.joint).is_some_and(|j| foot_joint(rig, j));
            let frame = ap
                .map(|a| {
                    let delta = a
                        .joint
                        .and_then(|j| matrices.get(j).zip(rig.default_world_inv.get(j)))
                        .map(|(m, inv)| *m * *inv);
                    let (position, rotation) = match delta {
                        Some(m) => (
                            m.transform_point3(a.skel_position),
                            m.to_scale_rotation_translation().1 * a.rotation,
                        ),
                        None => (a.skel_position, a.rotation),
                    };
                    Mat4::from_rotation_translation(rotation.normalize(), position)
                })
                .unwrap_or(Mat4::from_translation(self.avatar_lib.pelvis));
            let mut stack = vec![(root_idx, frame, 0)];
            while let Some((idx, parent, depth)) = stack.pop() {
                let Some(object) = world.objects.get(idx) else { continue };
                if depth > 16 {
                    continue;
                }
                object.full_id.hash(&mut hash);
                let model = parent * Mat4::from_rotation_translation(object.rotation, object.position);
                bits(&object.position.to_array(), &mut hash);
                bits(&object.rotation.to_array(), &mut hash);
                bits(&object.visual_scale().to_array(), &mut hash);
                let volume = object.visual_volume();
                if volume.is_mesh() {
                    if let Some(sculpt) = volume.sculpt {
                        sculpt.texture.hash(&mut hash);
                        let (data, skin) = self.meshes.preview_source(&sculpt.texture);
                        data.as_ref().map(|d| Arc::as_ptr(d) as usize).hash(&mut hash);
                        skin.as_ref().map(|s| Arc::as_ptr(s) as usize).hash(&mut hash);
                        if let Some(data) = data {
                            // Worn animesh has its own controller and does not
                            // deform with the avatar's feet.
                            let own_rig = skin.is_some() && Self::skeleton_owner(world, idx).is_some_and(|s| s.0 == world.agent_id);
                            if own_rig || (skin.is_none() && on_foot) {
                                surfaces.push(Surface::Mesh {
                                    data,
                                    skin,
                                    rigid: (!own_rig).then(|| model * Mat4::from_scale(object.visual_scale())),
                                });
                            }
                        } else if !self.meshes.failed(&sculpt.texture) {
                            ready = false;
                        }
                    }
                } else if on_foot {
                    let key = self.gpu.get(idx).and_then(|g| g.geom);
                    key.hash(&mut hash);
                    let geom = key.and_then(|k| self.geom_ready(&k));
                    geom.as_ref().map(|g| Arc::as_ptr(g) as usize).hash(&mut hash);
                    match geom {
                        Some(geom) => surfaces.push(Surface::Geometry {
                            geom,
                            model: model * Mat4::from_scale(object.visual_scale()),
                        }),
                        None => ready = false,
                    }
                }
                for &child in world.objects.children_of(&object.key) {
                    stack.push((child, model, depth + 1));
                }
            }
        }
        let signature = hash.finish();
        let state = &mut self.auto_hover;
        if state.observed != Some(signature) {
            state.observed = Some(signature);
            state.changed = now;
            state.finished = None;
        }
        while let Ok((stamp, result)) = state.rx.try_recv() {
            state.pending = false;
            if ready && state.observed == Some(stamp) {
                state.finished = Some(stamp);
                if let Some(value) = result {
                    log::info!("auto hover height: {value:.3} m");
                    world.hover_height.preview(value);
                    world.hover_height.commit();
                }
            }
        }
        if !ready || state.pending || state.finished == Some(signature) || now.duration_since(state.changed) < SETTLE {
            return;
        }
        let estimate = Estimate {
            lib: self.avatar_lib.clone(),
            base: skeleton.base.clone(),
            // The simulator places the object's physical centre half its
            // reported scale above the floor (World::avatar_floor). The
            // shaped skeleton's computed height can differ: using it here
            // raised every type of shoe by half that difference (~10 cm).
            ground: self.avatar_lib.pelvis.z - skeleton.shape_dz - 0.5 * owner.scale.z,
            surfaces,
        };
        state.pending = true;
        let tx = state.tx.clone();
        self.jobs.execute(move || {
            let _ = tx.send((signature, estimate.calculate()));
        });
    }
}

fn lower(min: &mut Option<f32>, point: Vec3) {
    if point.is_finite() {
        *min = Some(min.map_or(point.z, |old| old.min(point.z)));
    }
}

/// Match packed GPU weights: a tiny source influence rounded to zero does
/// not select a vertex; selected vertices still blend ALL their influences.
fn skinned_point(position: Vec3, vertex: &SkinVertex, transforms: &[Mat4], feet: &[bool]) -> Option<Vec3> {
    if !vertex
        .joints
        .iter()
        .zip(vertex.weights)
        .any(|(&j, w)| w > 0 && feet.get(j as usize) == Some(&true))
    {
        return None;
    }
    let sum: u32 = vertex.weights.iter().map(|&w| u32::from(w)).sum();
    if sum == 0 {
        return None;
    }
    let mut point = Vec3::ZERO;
    for (&joint, weight) in vertex.joints.iter().zip(vertex.weights) {
        if weight > 0 {
            point += transforms
                .get(joint as usize)
                .copied()
                .unwrap_or(Mat4::IDENTITY)
                .transform_point3(position)
                * (f32::from(weight) / sum as f32);
        }
    }
    Some(point)
}

fn mesh_min(faces: &[MeshFace], skin: Option<&SkinInfo>, rigid: Option<Mat4>, rig: &anim::Rig, matrices: &[Mat4]) -> Option<f32> {
    let mut min = None;
    if let Some(model) = rigid {
        for face in faces {
            for &index in &face.indices {
                if let Some(p) = face.positions.get(index as usize) {
                    lower(&mut min, model.transform_point3(Vec3::from_array(*p)));
                }
            }
        }
        return min;
    }
    let skin = skin?;
    let feet: Vec<bool> = skin
        .joint_names
        .iter()
        .map(|name| rig.names.get(name).is_some_and(|&j| foot_joint(rig, j)))
        .collect();
    if !feet.iter().any(|&foot| foot) {
        return None;
    }
    let transforms: Vec<Mat4> = skin
        .joint_names
        .iter()
        .zip(&skin.inverse_bind)
        .map(|(name, inv)| {
            rig.names
                .get(name)
                .and_then(|&j| matrices.get(j))
                .copied()
                .unwrap_or(Mat4::IDENTITY)
                * *inv
        })
        .collect();
    // Same packing as jobs::mesh_to_faces, without rebuilding normals, UVs
    // and GPU buffers for an already loaded mesh just to measure its feet.
    for face in faces {
        if face.positions.len() > 65536
            || face.indices.len() < 3
            || face.joints.len() != face.positions.len()
            || face.weights.len() != face.positions.len()
        {
            continue;
        }
        for &index in &face.indices {
            let i = index as usize;
            let Some(position) = face.positions.get(i) else { continue };
            let mut vertex = SkinVertex::default();
            for k in 0..4 {
                vertex.joints[k] = if (face.joints[i][k] as usize) < skin.joint_names.len() {
                    face.joints[i][k]
                } else {
                    0
                };
                vertex.weights[k] = (face.weights[i][k].clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            if let Some(point) = skinned_point(
                skin.bind_shape.transform_point3(Vec3::from_array(*position)),
                &vertex,
                &transforms,
                &feet,
            ) {
                lower(&mut min, point);
            }
        }
    }
    min
}

impl Estimate {
    fn calculate(self) -> Option<f32> {
        let rig = &self.lib.rig;
        let matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &self.base.local_pos, &self.base.scale);
        let min = self
            .surfaces
            .into_par_iter()
            .map(|surface| -> Result<Option<f32>, ()> {
                let mut min = None;
                match surface {
                    Surface::Geometry { geom, model } => {
                        for face in geom.pick_faces.iter().flatten() {
                            for &index in &face.indices {
                                if let Some(p) = face.positions.get(index as usize) {
                                    lower(&mut min, model.transform_point3(Vec3::from_array(*p)));
                                }
                            }
                        }
                    }
                    Surface::Mesh { data, skin, rigid } => {
                        let header = aurora_assets::parse_mesh_header(&data).map_err(|_| ())?;
                        let lod = header.actual_lod(3).ok_or(())?;
                        let range = header.lods[lod].ok_or(())?;
                        let section = aurora_assets::mesh::section_bytes(&data, range).ok_or(())?;
                        let faces = aurora_assets::decode_mesh_lod(section).map_err(|_| ())?;
                        if let Some(z) = mesh_min(&faces, skin.as_deref(), rigid, rig, &matrices) {
                            lower(&mut min, Vec3::Z * z);
                        }
                    }
                }
                Ok(min)
            })
            .try_reduce(
                || None,
                |left, right| {
                    Ok(match (left, right) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    })
                },
            )
            .ok()?;
        crate::world::hover_height::bounded(min.map_or(0.0, |z| self.ground - z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;
    use glam::Quat;

    fn library() -> Arc<AvatarLibrary> {
        static LIB: std::sync::OnceLock<Arc<AvatarLibrary>> = std::sync::OnceLock::new();
        LIB.get_or_init(|| Arc::new(AvatarLibrary::load())).clone()
    }

    fn face() -> MeshFace {
        MeshFace {
            positions: vec![[0.0, 0.0, -0.1], [0.1, 0.0, -10.0], [0.0, 0.1, 0.2], [0.0, 0.0, -50.0]],
            indices: vec![0, 1, 2],
            joints: vec![[0, 1, 0, 0]; 4],
            weights: vec![
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
            ],
            ..Default::default()
        }
    }

    fn skin() -> SkinInfo {
        SkinInfo {
            joint_names: vec!["mFootLeft".into(), "mPelvis".into()],
            inverse_bind: vec![Mat4::IDENTITY; 2],
            bind_shape: Mat4::from_translation(Vec3::Z * -0.2),
            ..Default::default()
        }
    }

    #[test]
    fn only_drawn_vertices_with_nonzero_foot_weights_select_the_minimum() {
        let lib = library();
        let matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        assert!((mesh_min(&[face()], Some(&skin()), None, &lib.rig, &matrices).unwrap() + 0.3).abs() < 1e-5);
        let mut unrelated = face();
        unrelated.weights.fill([0.0, 1.0, 0.0, 0.0]);
        assert_eq!(mesh_min(&[unrelated], Some(&skin()), None, &lib.rig, &matrices), None);
    }

    #[test]
    fn tiny_weights_rounded_to_zero_do_not_select_low_vertices() {
        let lib = library();
        let mut geometry = face();
        geometry.weights[1] = [0.0001, 0.9999, 0.0, 0.0];
        let matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        assert!((mesh_min(&[geometry], Some(&skin()), None, &lib.rig, &matrices).unwrap() + 0.3).abs() < 1e-5);
    }

    #[test]
    fn selected_vertices_blend_every_joint_and_normalize_packed_weights() {
        let vertex = SkinVertex {
            joints: [0, 1, 0, 0],
            weights: [64, 191, 0, 0],
        };
        let matrices = [Mat4::from_translation(-Vec3::Z), Mat4::from_translation(Vec3::Z)];
        let point = skinned_point(Vec3::ZERO, &vertex, &matrices, &[true, false]).unwrap();
        assert!((point.z - 127.0 / 255.0).abs() < 1e-6);
        assert_eq!(skinned_point(Vec3::ZERO, &vertex, &matrices, &[false, false]), None);
        assert_eq!(skinned_point(Vec3::ZERO, &SkinVertex::default(), &matrices, &[true, true]), None);
    }

    #[test]
    fn inverse_binds_shape_scale_and_offsets_match_the_gpu_skinning_order() {
        let lib = library();
        let mut matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        matrices[lib.rig.names["mFootLeft"]] =
            Mat4::from_scale_rotation_translation(Vec3::new(1.0, 1.0, 2.0), Quat::IDENTITY, Vec3::Z * 0.4);
        let mut skin = skin();
        skin.inverse_bind[0] = Mat4::from_translation(Vec3::Z * 0.1);
        // (-.1 - .2 + .1) * 2 + .4 = 0
        assert!(mesh_min(&[face()], Some(&skin), None, &lib.rig, &matrices).unwrap().abs() < 1e-5);
    }

    #[test]
    fn foot_aliases_toes_fitted_feet_and_hind_feet_are_supported() {
        let lib = library();
        for name in ["lFoot", "rFoot", "mToeLeft", "R_FOOT", "mHindLimb3Left"] {
            assert!(foot_joint(&lib.rig, lib.rig.names[name]), "{name}");
        }
        assert!(!foot_joint(&lib.rig, lib.rig.names["mKneeLeft"]));
    }

    #[test]
    fn rigid_geometry_uses_transformed_vertices_instead_of_a_bounding_box() {
        let lib = library();
        let geometry = MeshFace {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let model = Mat4::from_scale_rotation_translation(
            Vec3::new(2.0, 1.0, 2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
            Vec3::Z * -0.2,
        );
        assert!((mesh_min(&[geometry], None, Some(model), &lib.rig, &[]).unwrap() + 0.2).abs() < 1e-5);
    }

    fn fixture() -> (World, Scene, Instant) {
        let lib = library();
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        world.apply(NetEvent::MainRegionChanged { handle: 1 });
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::action_avatar(false)],
        });
        let region = world.regions.get_mut(&1).unwrap();
        region.avatar_hover_height = true;
        Arc::make_mut(&mut region.caps).insert("AgentPreferences".into(), String::new());
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        let now = Instant::now();
        rest(&mut scene, &world, now);
        (world, scene, now)
    }

    fn rest(scene: &mut Scene, world: &World, now: Instant) {
        scene.skeleton_of(world, world.agent_id, world.objects.index_of_uuid(&world.agent_id).unwrap(), now);
    }

    fn settle(scene: &mut Scene, world: &mut World, now: Instant) {
        rest(scene, world, now);
        scene.update_auto_hover(world, true, now);
        for _ in 0..1000 {
            scene.update_auto_hover(world, true, now + SETTLE);
            if scene.auto_hover.finished == scene.auto_hover.observed {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("grounding job did not finish");
    }

    #[test]
    fn rigged_shoes_use_high_lod_and_ignore_lower_geometry_with_zero_foot_weights() {
        let lib = library();
        let mut scene = Scene::new(std::path::PathBuf::new(), lib.clone());
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        let (data, skin) = scene.meshes.preview_source(&crate::demo::auto_hover::MESH);
        let foot = lib.rig.default_world_inv[lib.rig.names["mFootLeft"]].inverse().w_axis.z;
        let estimate = Estimate {
            lib: lib.clone(),
            base: Arc::new(anim::SkeletonBase {
                local_pos: lib.rig.local_pos.clone(),
                scale: lib.rig.scale.clone(),
            }),
            ground: foot,
            surfaces: vec![Surface::Mesh {
                data: data.unwrap(),
                skin,
                rigid: None,
            }],
        };
        assert!((estimate.calculate().unwrap() - 0.18).abs() < 0.001);
    }

    #[test]
    fn body_attachment_changes_recalculate_and_huds_and_animation_poses_preserve_manual_edits() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        assert!(world.hover_height.value() > 0.1 && world.hover_height.value() < 0.3);
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        let mut hud = crate::demo::worn::root();
        hud.state = 31u8.rotate_left(4);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![hud],
        });
        world
            .avatar_poses
            .insert(world.agent_id, vec![Mat4::from_translation(Vec3::Z * 12.0); lib.rig.len()]);
        rest(&mut scene, &world, now + SETTLE);
        scene.update_auto_hover(&mut world, true, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.35);
        assert_eq!(scene.auto_hover.finished, scene.auto_hover.observed);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert_eq!(world.hover_height.value(), 0.0);
    }

    #[test]
    fn simulator_physical_height_places_the_lowest_shoe_vertex_on_the_floor() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let physical_height = 1.9;
        let avatar = world.objects.get_mut(owner).unwrap();
        avatar.scale.z = physical_height;
        avatar.position.z = 0.5 * physical_height; // physics centre above a floor at z = 0
        world.agent.position.z = 0.5 * physical_height;
        scene.palette_base(world.agent_id);
        scene.neutral_pose_test = Some(true);
        scene.update_poses(&mut world, Vec3::ZERO, now);
        settle(&mut scene, &mut world, now);
        let (data, skin) = scene.meshes.preview_source(&crate::demo::auto_hover::MESH);
        let data = data.unwrap();
        let header = aurora_assets::parse_mesh_header(&data).unwrap();
        let faces = aurora_assets::decode_mesh_lod(aurora_assets::mesh::section_bytes(&data, header.lods[3].unwrap()).unwrap()).unwrap();
        let lowest = mesh_min(&faces, skin.as_deref(), None, &lib.rig, &scene.auto_hover.matrices).unwrap();
        let root = Scene::object_transform(&world, owner, now, 0).unwrap().0;
        let bottom_world = root.z - lib.pelvis.z + lowest;
        assert!(
            bottom_world.abs() < 1e-5,
            "shoe must touch the physical floor, bottom = {bottom_world:.6} m"
        );
        let previous = world.hover_height.value();
        world.objects.get_mut(owner).unwrap().scale.z += 0.2;
        world.agent.position.z += 0.1;
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert!(
            (world.hover_height.value() - previous + 0.1).abs() < 1e-5,
            "late physics height must invalidate the cached estimate"
        );
        let root = Scene::object_transform(&world, owner, now, 0).unwrap().0;
        assert!((root.z - lib.pelvis.z + lowest).abs() < 1e-5);
    }

    #[test]
    fn loading_detach_stale_results_seating_and_disable_do_not_apply_old_heights() {
        let (mut world, mut scene, now) = fixture();
        world.hover_height.preview(0.42);
        world.hover_height.commit();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, now);
        let old = scene.auto_hover.observed.unwrap();
        scene.update_auto_hover(&mut world, true, now + SETTLE);
        assert!(!scene.auto_hover.pending, "wait for missing mesh");
        scene.auto_hover.tx.send((old, Some(1.7))).unwrap();
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, now);
        assert_eq!(world.hover_height.value(), 0.42, "discard detached outfit result");
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        world.objects.get_mut(owner).unwrap().parent_id = 999;
        scene.update_auto_hover(&mut world, true, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.42);
        world.objects.get_mut(owner).unwrap().parent_id = 0;
        scene.update_auto_hover(&mut world, false, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.42);
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert_eq!(world.hover_height.value(), 0.0);
    }

    #[test]
    fn rigid_foot_linksets_include_the_linked_heel_and_defer_until_geometry_arrives() {
        use super::super::{GeomKey, GeomState, ObjGpu, picking::PickFace};
        let (mut world, mut scene, now) = fixture();
        world.hover_height.preview(0.4);
        world.hover_height.commit();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: crate::demo::auto_hover::rigid_objects(),
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, now);
        scene.update_auto_hover(&mut world, true, now + SETTLE);
        assert!(!scene.auto_hover.pending);
        assert_eq!(world.hover_height.value(), 0.4);
        for id in [
            crate::demo::auto_hover::LEFT,
            crate::demo::auto_hover::RIGHT,
            crate::demo::auto_hover::HEEL,
        ] {
            let key = GeomKey::Prim {
                hash: u64::from(id),
                lod: 3,
            };
            let index = world
                .objects
                .index_of(&crate::world::objects::ObjKey { region: 1, local_id: id })
                .unwrap();
            if scene.gpu.len() <= index {
                scene.gpu.resize_with(index + 1, ObjGpu::default);
            }
            scene.gpu[index].geom = Some(key);
            scene.geom_insert(
                key,
                GeomState::Ready(Arc::new(GpuGeom {
                    faces: Vec::new(),
                    min: Vec3::splat(-0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: vec![Some(PickFace::new(
                        vec![[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.0, 0.5, -0.5]],
                        Vec::new(),
                        vec![0, 1, 2],
                    ))],
                })),
            );
        }
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        let skeleton = &scene.skeletons[&world.agent_id];
        let matrices = &scene.auto_hover.matrices;
        let foot = matrices[scene.avatar_lib.rig.names["mFootLeft"]].w_axis.z;
        let physical_height = world
            .objects
            .get(world.objects.index_of_uuid(&world.agent_id).unwrap())
            .unwrap()
            .scale
            .z;
        let ground = scene.avatar_lib.pelvis.z - skeleton.shape_dz - 0.5 * physical_height;
        assert!(
            (world.hover_height.value() - (ground - foot + 0.22)).abs() < 1e-5,
            "linked heel must be the lowest point"
        );
    }

    #[test]
    fn invalid_loaded_mesh_keeps_the_previous_height_instead_of_applying_a_partial_estimate() {
        let lib = library();
        let estimate = Estimate {
            base: Arc::new(anim::SkeletonBase {
                local_pos: lib.rig.local_pos.clone(),
                scale: lib.rig.scale.clone(),
            }),
            lib,
            ground: 0.0,
            surfaces: vec![Surface::Mesh {
                data: Arc::new(vec![0xff; 4]),
                skin: None,
                rigid: Some(Mat4::IDENTITY),
            }],
        };
        assert_eq!(estimate.calculate(), None);
        assert_eq!(crate::world::hover_height::bounded(-0.25), Some(-0.25));
    }

    #[test]
    fn persisted_auto_option_defaults_on_and_can_be_disabled() {
        let settings: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.auto_hover_height);
        let disabled: crate::settings::Settings = serde_json::from_str("{\"auto_hover_height\":false}").unwrap();
        assert!(!disabled.auto_hover_height);
    }
}
