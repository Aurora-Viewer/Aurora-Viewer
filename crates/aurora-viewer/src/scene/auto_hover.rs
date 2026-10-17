//! Attachment-driven grounding in a neutral pose. This extends Firestorm's
//! manual LLFloaterHoverHeight / LLVOAvatarSelf::sendHoverHeight: animations
//! never drive the estimate, and only rendered vertices with foot weights count.
use super::meshes::MeshView;
use super::{GpuGeom, Scene, anim, avatar::AvatarLibrary};
use crate::world::World;
use aurora_assets::{MeshFace, SkinInfo};
use aurora_render::SkinVertex;
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3, Vec4};
use rayon::prelude::*;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Measurement {
    height: f32,
    #[cfg(test)]
    lowest: Option<f32>,
}

/// LLWorld::resolveStepHeightGlobal reads this region-space contact plane.
/// Auto hover deliberately omits its legacy 5 cm Havok compensation: that
/// lowers the inferred floor and our shoes by 5 cm in the online comparison.
/// Intersect the reported plane vertically, including on sloped surfaces;
/// normal distance alone is not a vertical height when the normal is tilted.
fn contact_clearance(position: Vec3, plane: Option<Vec4>) -> Option<f32> {
    let plane = plane?;
    if !plane.is_finite() || plane.z <= 1e-4 || !position.is_finite() {
        return None;
    }
    let distance = (position.dot(plane.truncate()) - plane.w) / plane.z;
    (distance.is_finite() && distance > 0.0).then_some(distance)
}

pub(super) struct AutoHover {
    observed: Option<u64>,
    changed: Instant,
    finished: Option<u64>,
    pending: bool,
    tx: Sender<(u64, Option<Measurement>, bool)>,
    rx: Receiver<(u64, Option<Measurement>, bool)>,
    measurement: Option<Measurement>,
    used_contact: bool,
    offset: Option<f32>,
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
            measurement: None,
            used_contact: false,
            offset: None,
            base: None,
            base_hash: 0,
            matrices: Vec::new(),
        }
    }
}

enum Surface {
    Mesh { mesh: Box<MeshView>, rigid: Option<Mat4> },
    Geometry { geom: Arc<GpuGeom>, model: Mat4 },
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
    /// Keep the raw estimate for immediate offset previews, without decoding
    /// or skinning the attachments again. Unfinished outfits have no baseline.
    pub fn automatic_hover_base(&self, world: &World) -> Option<f32> {
        let owner = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i))?;
        if owner.parent_id != 0 || world.agent.flying || !world.hover_height_available() || !world.hover_height.ready() {
            return None;
        }
        let state = &self.auto_hover;
        (state.observed.is_some() && state.finished == state.observed)
            .then_some(state.measurement)
            .flatten()
            .map(|m| m.height)
    }

    /// Called after the rest skeleton has been refreshed, before scene sync.
    /// Snapshots contain only Arc handles and transforms; decoding, skinning
    /// and vertex scans use the scene's low-priority pool. Old jobs are ignored
    /// after another wear/detach, a HUD update cannot trigger a recalibration.
    pub fn update_auto_hover(&mut self, world: &mut World, enabled: bool, offset: f32, commit: bool, now: Instant) {
        if !enabled {
            self.auto_hover.observed = None;
            self.auto_hover.finished = None;
            self.auto_hover.offset = None;
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
        let contact = contact_clearance(owner.position, owner.foot_plane);
        // A cleared plane is an explicit airborne report. An absent block
        // (offline demo / first full update) can still use the physical size.
        if owner.foot_plane.is_some() && contact.is_none() {
            return;
        }
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
                    let (position, rotation) = a.frame(a.joint.and_then(|j| matrices.get(j)).copied());
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
                        // (once a level of detail is held: the blocks arrive one
                        // by one, a new one changes the hash)
                        let mesh = self.meshes.view(&sculpt.texture).filter(|m| m.lod_section(3).is_some());
                        mesh.as_ref().map(|m| m.bytes.held()).hash(&mut hash);
                        let skin = mesh.as_ref().and_then(|m| m.skin.clone());
                        skin.as_ref().map(|s| Arc::as_ptr(s) as usize).hash(&mut hash);
                        if let Some(mesh) = mesh {
                            // Worn animesh has its own controller and does not
                            // deform with the avatar's feet.
                            let own_rig = skin.is_some() && Self::skeleton_owner(world, idx).is_some_and(|s| s.0 == world.agent_id);
                            if own_rig || (skin.is_none() && on_foot) {
                                surfaces.push(Surface::Mesh {
                                    mesh: Box::new(mesh),
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
        // Upgrade an initial size-only estimate once the first contact plane
        // arrives. Later motion/contact changes do not overwrite manual edits.
        if state.finished == Some(signature) && !state.used_contact && contact.is_some() {
            state.finished = None;
        }
        let offset = crate::world::hover_height::bounded(offset).unwrap_or(0.0);
        let offset_changed = state.offset != Some(offset);
        state.offset = Some(offset);
        if ready
            && offset_changed
            && state.finished == Some(signature)
            && let Some(measurement) = state.measurement
        {
            world.hover_height.preview(measurement.height + offset);
            if commit {
                world.hover_height.commit();
            }
        }
        while let Ok((stamp, result, used_contact)) = state.rx.try_recv() {
            state.pending = false;
            if ready && state.observed == Some(stamp) {
                state.finished = Some(stamp);
                state.measurement = result;
                state.used_contact = used_contact;
                if let Some(Measurement { height: value, .. }) = result {
                    world.hover_height.preview(value + offset);
                    log::info!("auto hover height: {:.3} m", world.hover_height.value());
                    if commit {
                        world.hover_height.commit();
                    }
                }
            }
        }
        if !ready || state.pending || state.finished == Some(signature) || now.duration_since(state.changed) < SETTLE {
            return;
        }
        let estimate = Estimate {
            lib: self.avatar_lib.clone(),
            base: skeleton.base.clone(),
            // Physical size alone omits the simulator's collision clearance.
            // On platforms, use the actual contact plane when available.
            ground: self.avatar_lib.pelvis.z - skeleton.shape_dz - contact.unwrap_or(0.5 * owner.scale.z),
            surfaces,
        };
        state.pending = true;
        let tx = state.tx.clone();
        self.jobs.execute(move || {
            let _ = tx.send((signature, estimate.calculate(), contact.is_some()));
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
    fn calculate(self) -> Option<Measurement> {
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
                    Surface::Mesh { mesh, rigid } => {
                        // the finest level held (the high one, as LLVOAvatar reads)
                        let section = mesh.lod_section(3).ok_or(())?;
                        let faces = aurora_assets::decode_mesh_lod(section).map_err(|_| ())?;
                        if let Some(z) = mesh_min(&faces, mesh.skin.as_deref(), rigid, rig, &matrices) {
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
        // Apply the user offset before the final server bounds, otherwise a
        // large raw estimate followed by a negative offset clamps too early.
        let height = min.map_or(0.0, |z| self.ground - z);
        height.is_finite().then_some(Measurement {
            height,
            #[cfg(test)]
            lowest: min,
        })
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
        settle_offset(scene, world, now, 0.0);
    }

    fn settle_offset(scene: &mut Scene, world: &mut World, now: Instant, offset: f32) {
        rest(scene, world, now);
        scene.update_auto_hover(world, true, offset, true, now);
        for _ in 0..1000 {
            scene.update_auto_hover(world, true, offset, true, now + SETTLE);
            if scene.auto_hover.finished == scene.auto_hover.observed {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("grounding job did not finish");
    }

    #[test]
    fn fixed_offset_previews_cached_height_and_survives_outfit_changes() {
        let (mut world, mut scene, now) = fixture();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle_offset(&mut scene, &mut world, now, 0.05);
        let base = scene.automatic_hover_base(&world).unwrap();
        assert!((world.hover_height.value() - base - 0.05).abs() < 1e-5);
        let stamp = scene.auto_hover.finished;
        let Some(aurora_net::NetCommand::SetHoverHeight { request, .. }) = world.hover_height.next_update(Some(1)) else {
            panic!("initial committed auto height");
        };
        world.hover_height.reply(request, 1, true);
        scene.update_auto_hover(&mut world, true, 0.1, false, now + Duration::from_secs(1));
        assert!((world.hover_height.value() - base - 0.1).abs() < 1e-5);
        assert_eq!(scene.auto_hover.finished, stamp);
        assert!(!scene.auto_hover.pending, "offset changes must reuse the estimate");
        assert!(world.hover_height.next_update(Some(1)).is_none(), "a held drag must only preview");
        world.hover_height.commit(); // slider release
        let Some(aurora_net::NetCommand::SetHoverHeight { request, value, .. }) = world.hover_height.next_update(Some(1)) else {
            panic!("released offset must commit the preview");
        };
        assert!((value - base - 0.1).abs() < 1e-5);
        world.hover_height.reply(request, 1, true);
        scene.update_auto_hover(&mut world, true, -0.03, true, now + Duration::from_secs(2));
        assert!((world.hover_height.value() - base + 0.03).abs() < 1e-5);
        for _ in 0..10 {
            scene.update_auto_hover(&mut world, true, -0.03, true, now + Duration::from_secs(3));
        }
        assert!(
            (world.hover_height.value() - base + 0.03).abs() < 1e-5,
            "offset cannot accumulate across frames"
        );
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        scene.update_auto_hover(&mut world, false, 0.5, true, now + Duration::from_secs(4));
        assert_eq!(world.hover_height.value(), 0.35);
        assert!(scene.automatic_hover_base(&world).is_none());
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        settle_offset(&mut scene, &mut world, now + Duration::from_secs(5), -0.03);
        assert_eq!(world.hover_height.value(), -0.03, "barefoot auto height keeps the fixed offset");
    }

    #[test]
    fn fixed_offset_is_added_before_the_final_hover_limit() {
        use aurora_assets::import_mesh::{encode, process::Physics};
        let (mut world, mut scene, now) = fixture();
        let mut model = crate::demo::auto_hover::rigged_model(&world.avatar_lib);
        for vertex in model.faces.iter_mut().flat_map(|f| &mut f.vertices) {
            vertex.position[2] -= 3.0;
        }
        model.normalize().unwrap();
        let data = encode::asset(
            &[None, None, None, Some(model)],
            None,
            &Physics::default(),
            encode::Options {
                skin: true,
                ..Default::default()
            },
        )
        .unwrap();
        let header = aurora_assets::parse_mesh_header(&data).unwrap();
        let skin = header
            .skin
            .map(|range| Arc::new(aurora_assets::decode_skin(aurora_assets::mesh::section_bytes(&data, range).unwrap()).unwrap()));
        scene
            .meshes
            .insert_local(crate::demo::auto_hover::MESH, Arc::new(data), header, skin);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle_offset(&mut scene, &mut world, now, -0.5);
        let base = scene.automatic_hover_base(&world).unwrap();
        assert!(base > crate::world::hover_height::MAX);
        assert!((world.hover_height.value() - (base - 0.5)).abs() < 1e-5);
        scene.update_auto_hover(&mut world, true, 0.5, true, now + Duration::from_secs(1));
        assert_eq!(world.hover_height.value(), crate::world::hover_height::MAX);
        scene.update_auto_hover(&mut world, true, f32::NAN, true, now + Duration::from_secs(2));
        assert_eq!(world.hover_height.value(), crate::world::hover_height::MAX);
    }

    #[test]
    fn rigged_shoes_use_high_lod_and_ignore_lower_geometry_with_zero_foot_weights() {
        let lib = library();
        let mut scene = Scene::new(std::path::PathBuf::new(), lib.clone());
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        let mesh = scene.meshes.view(&crate::demo::auto_hover::MESH);
        let foot = lib.rig.default_world_inv[lib.rig.names["mFootLeft"]].inverse().w_axis.z;
        let estimate = Estimate {
            lib: lib.clone(),
            base: Arc::new(anim::SkeletonBase {
                local_pos: lib.rig.local_pos.clone(),
                scale: lib.rig.scale.clone(),
            }),
            ground: foot,
            surfaces: vec![Surface::Mesh {
                mesh: Box::new(mesh.unwrap()),
                rigid: None,
            }],
        };
        assert!((estimate.calculate().unwrap().height - 0.18).abs() < 0.001);
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
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
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
        let mesh = scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap();
        let section = mesh.bytes.section(mesh.header.lods[3].unwrap()).unwrap();
        let faces = aurora_assets::decode_mesh_lod(section).unwrap();
        let lowest = mesh_min(&faces, mesh.skin.as_deref(), None, &lib.rig, &scene.auto_hover.matrices).unwrap();
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
    fn contact_plane_removes_collision_clearance_and_preserves_manual_gap() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let floor = 40.0;
        let avatar = world.objects.get_mut(owner).unwrap();
        avatar.scale.z = 1.9;
        avatar.position.z = floor + 0.95 + 0.1; // extra physics clearance omitted by a size-only estimate
        world.agent.position.z = avatar.position.z;
        scene.palette_base(world.agent_id);
        scene.neutral_pose_test = Some(true);
        scene.update_poses(&mut world, Vec3::ZERO, now);
        settle(&mut scene, &mut world, now);
        let previous = world.hover_height.value();
        // Full updates may precede the first terse collision plane.
        world.objects.get_mut(owner).unwrap().foot_plane = Some(Vec4::new(0.0, 0.0, 1.0, floor));
        let gap = |scene: &Scene, world: &World| {
            Scene::object_transform(world, owner, now, 0).unwrap().0.z - lib.pelvis.z
                + scene.auto_hover.measurement.unwrap().lowest.unwrap()
                - floor
        };
        assert!((gap(&scene, &world) - 0.1).abs() < 1e-5);
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        let grounded = world.hover_height.value();
        assert!((grounded - previous + 0.1).abs() < 1e-5);
        assert!(gap(&scene, &world).abs() < 1e-5);
        let root = Scene::object_transform(&world, owner, now, 0).unwrap().0;
        let lowest = scene.auto_hover.measurement.unwrap().lowest.unwrap();
        assert!((root.z - lib.pelvis.z + lowest - floor).abs() < 1e-5);

        world.hover_height.preview(grounded + 0.1);
        world.hover_height.commit();
        assert!((gap(&scene, &world) - 0.1).abs() < 1e-5);
        world.objects.get_mut(owner).unwrap().foot_plane = Some(Vec4::ZERO);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + Duration::from_secs(2));
        assert!((world.hover_height.value() - grounded - 0.1).abs() < 1e-5);
        world.objects.get_mut(owner).unwrap().foot_plane = Some(Vec4::new(0.0, 0.0, 1.0, floor));
        scene.update_auto_hover(&mut world, true, 0.0, true, now + Duration::from_secs(3));
        assert!(
            (world.hover_height.value() - grounded - 0.1).abs() < 1e-5,
            "landing preserves a manual edit"
        );
    }

    #[test]
    fn contact_plane_uses_the_reported_surface_without_a_legacy_margin() {
        let position = Vec3::new(3.0, 4.0, 20.95);
        let clearance = contact_clearance(position, Some(Vec4::new(0.0, 0.0, 1.0, 20.0))).unwrap();
        let floor = position.z - clearance;
        assert!(
            (floor - 20.0).abs() < 1e-5,
            "reported floor at 20 m must stay there, got {floor:.6} m"
        );
        let normal = Vec3::new(0.0, 0.6, 0.8);
        let plane = normal.extend(position.dot(normal) - 0.95 * normal.z);
        assert!((contact_clearance(position, Some(plane)).unwrap() - 0.95).abs() < 1e-5);
        assert!((contact_clearance(position, Some(plane * 2.0)).unwrap() - 0.95).abs() < 1e-5);
        for plane in [
            None,
            Some(Vec4::ZERO),
            Some(Vec4::splat(f32::NAN)),
            Some(Vec4::new(0.0, 0.0, -1.0, 0.0)),
        ] {
            assert_eq!(contact_clearance(position, plane), None);
        }
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
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        let old = scene.auto_hover.observed.unwrap();
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
        assert!(!scene.auto_hover.pending, "wait for missing mesh");
        scene
            .auto_hover
            .tx
            .send((
                old,
                Some(Measurement {
                    height: 1.7,
                    lowest: Some(0.0),
                }),
                false,
            ))
            .unwrap();
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        assert_eq!(world.hover_height.value(), 0.42, "discard detached outfit result");
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        world.objects.get_mut(owner).unwrap().parent_id = 999;
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.42);
        world.objects.get_mut(owner).unwrap().parent_id = 0;
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
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
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
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
        let previous = world.hover_height.value();
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let avatar = world.objects.get_mut(owner).unwrap();
        let floor = avatar.position.z - 0.5 * physical_height - 0.1;
        avatar.foot_plane = Some(Vec4::new(0.0, 0.0, 1.0, floor));
        settle(&mut scene, &mut world, now + Duration::from_secs(2));
        assert!(
            (world.hover_height.value() - previous + 0.1).abs() < 1e-5,
            "contact clearance also grounds a rigid linked heel"
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
                mesh: Box::new(MeshView {
                    header: aurora_assets::MeshHeader {
                        header_size: 0,
                        lods: [Some((0, 4)); 4],
                        skin: None,
                        physics_convex: None,
                        version: 1,
                    },
                    bytes: super::super::mesh_bytes::MeshBytes::whole(Arc::new(vec![0xff; 4])),
                    skin: None,
                }),
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
        assert_eq!(settings.auto_hover_offset, 0.0);
        let disabled: crate::settings::Settings =
            serde_json::from_str("{\"auto_hover_height\":false,\"auto_hover_offset\":-0.05}").unwrap();
        assert!(!disabled.auto_hover_height);
        assert_eq!(disabled.auto_hover_offset, -0.05);
        let reloaded: crate::settings::Settings = serde_json::from_str(&serde_json::to_string(&disabled).unwrap()).unwrap();
        assert_eq!(reloaded.auto_hover_offset, -0.05);
    }
}
