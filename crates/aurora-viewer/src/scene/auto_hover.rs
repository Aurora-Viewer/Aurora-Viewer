//! Avatar-only grounding in a neutral pose. This extends the Legacy Viewer's
//! manual LLFloaterHoverHeight / LLVOAvatarSelf::sendHoverHeight: animations
//! never drive the estimate, and only rendered vertices with foot weights count.
//! The reference is the shaped rest skeleton, independent of movement state,
//! streamed physical size, world contact and animated render palettes.
use super::meshes::MeshView;
use super::{GpuGeom, Scene, anim, avatar::AvatarLibrary};
use crate::world::World;
use aurora_assets::{MeshFace, SkinInfo};
use aurora_render::SkinVertex;
use crossbeam_channel::{Receiver, Sender};
use glam::{Mat4, Vec3};
use rayon::prelude::*;
use std::collections::HashMap;
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

pub(super) struct AutoHover {
    observed: Option<u64>,
    changed: Instant,
    finished: Option<u64>,
    pending: bool,
    tx: Sender<(u64, Option<Measurement>)>,
    rx: Receiver<(u64, Option<Measurement>)>,
    measurement: Option<Measurement>,
    offset: Option<f32>,
    base: Option<Arc<anim::SkeletonBase>>,
    base_hash: u64,
    matrices: Vec<Mat4>,
    second_life: bool,
    checked_at: Option<Instant>,
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
            offset: None,
            base: None,
            base_hash: 0,
            matrices: Vec::new(),
            second_life: true,
            checked_at: None,
        }
    }
}

/// Remote residents have independent channels and signatures. Four inspections
/// per frame, in round-robin order, bound asset traversal in crowded scenes.
#[derive(Default)]
pub(super) struct RemoteSoles {
    entries: HashMap<uuid::Uuid, AutoHover>,
    owners: Vec<uuid::Uuid>,
    cursor: usize,
}

struct SoleSnapshot {
    signature: u64,
    ready: bool,
    estimate: Estimate,
    body_height: f32,
    shape_hover: f32,
    box_height: f32,
    root_height: f32,
}

impl AutoHover {
    fn value(&self) -> Option<f32> {
        (self.observed.is_some() && self.finished == self.observed)
            .then_some(self.measurement)
            .flatten()
            .map(|measurement| measurement.height)
    }

    fn prepare_base(&mut self, rig: &anim::Rig, base: &Arc<anim::SkeletonBase>) {
        if self.base.as_ref().is_none_or(|cached| !Arc::ptr_eq(cached, base)) {
            if self.base.as_deref() != Some(base.as_ref()) {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                for point in base.local_pos.iter().chain(&base.scale) {
                    bits(&point.to_array(), &mut hash);
                }
                self.base_hash = hash.finish();
                self.matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
            }
            self.base = Some(base.clone());
        }
    }

    /// Poll and invalidate before accepting a result. The channel belongs to
    /// one avatar; a retired cache's late job cannot reach its replacement.
    fn update(&mut self, snapshot: SoleSnapshot, jobs: &super::jobs::Jobs, now: Instant) -> bool {
        let SoleSnapshot {
            signature,
            ready,
            estimate,
            body_height,
            shape_hover,
            box_height,
            root_height,
        } = snapshot;
        if self.observed != Some(signature) {
            self.observed = Some(signature);
            self.changed = now;
            self.finished = None;
        }
        let mut completed = false;
        while let Ok((stamp, result)) = self.rx.try_recv() {
            self.pending = false;
            if ready && self.observed == Some(stamp) {
                self.finished = Some(stamp);
                self.measurement = result;
                completed = true;
            }
        }
        if ready && !self.pending && self.finished != Some(signature) && now.saturating_duration_since(self.changed) >= SETTLE {
            log::info!(
                "auto hover standing model: shape body {body_height:.3} m, shape hover {shape_hover:.3} m, physical box {box_height:.3} m, neutral root reference {root_height:.3} m"
            );
            self.pending = true;
            let tx = self.tx.clone();
            jobs.execute(move || {
                let _ = tx.send((signature, estimate.calculate()));
            });
        }
        completed
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
    values: Vec<u8>,
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

fn lower_leg_joint(rig: &anim::Rig, mut joint: usize) -> bool {
    for _ in 0..rig.len() {
        if foot_joint(rig, joint) || ["mKneeLeft", "mKneeRight"].iter().any(|name| rig.names.get(*name) == Some(&joint)) {
            return true;
        }
        let Some(parent) = rig.parent.get(joint).copied().flatten() else {
            break;
        };
        joint = parent;
    }
    false
}

fn bits(values: &[f32], hash: &mut impl Hasher) {
    for value in values {
        value.to_bits().hash(hash);
    }
}

/// SL's shape-derived agent size is bounded to 1.1..2.45 m. Its standing
/// physical bounding box is another 0.2 m taller (llGetAgentSize wiki).
/// The half-height therefore includes 0.1 m on each side of the centre;
/// it is not a surface/contact correction. Legacy's AgentSetAppearance
/// omits shape hover on OpenSim, where this Havok padding does not apply.
fn standing_box_height(body_height: f32, shape_hover: f32, second_life: bool) -> Option<f32> {
    if !body_height.is_finite() || body_height <= 0.0 || !shape_hover.is_finite() {
        return None;
    }
    Some(if second_life {
        (body_height + shape_hover).clamp(1.1, 2.45) + 0.2
    } else {
        body_height
    })
}

/// HUDs without avatar foot skin cannot alter calibration. A foot-rigged
/// mesh is still selected by its skin at ANY attachment point, even there.
fn has_rigged_foot_mesh(scene: &Scene, world: &World, root: usize, avatar: uuid::Uuid) -> bool {
    let mut stack = vec![(root, 0)];
    while let Some((index, depth)) = stack.pop() {
        let Some(object) = world.objects.get(index) else { continue };
        if let Some(sculpt) = object.visual_volume().sculpt
            && let Some(skin) = scene.meshes.meta(&sculpt.texture).and_then(|meta| meta.skin)
            && Scene::skeleton_owner(world, index).is_some_and(|owner| owner.0 == avatar)
            && skin.joint_names.iter().any(|name| {
                scene
                    .avatar_lib
                    .rig
                    .names
                    .get(name)
                    .is_some_and(|&joint| foot_joint(&scene.avatar_lib.rig, joint))
            })
        {
            return true;
        }
        if depth < 16 {
            stack.extend(world.objects.children_of(&object.key).iter().map(|&child| (child, depth + 1)));
        }
    }
    false
}

impl Scene {
    pub fn set_auto_hover_second_life(&mut self, second_life: bool) {
        self.auto_hover.second_life = second_life;
    }

    pub fn automatic_hover_box_height(&self, world: &World) -> Option<f32> {
        let skeleton = self.skeletons.get(&world.agent_id)?;
        standing_box_height(
            skeleton.neutral_body_height,
            skeleton.neutral_shape_hover,
            self.auto_hover.second_life,
        )
    }

    /// Reuse the neutral geometry calibration, including worn soles, without
    /// scanning geometry on a pose worker or cancelling shoe hover with IK.
    pub(super) fn grounding_sole_height(&self, world: &World, owner: uuid::Uuid) -> Option<f32> {
        let skeleton = self.skeletons.get(&owner)?;
        let height = standing_box_height(
            skeleton.neutral_body_height,
            skeleton.neutral_shape_hover,
            self.auto_hover.second_life,
        )?;
        let adjustment = if owner == world.agent_id {
            self.automatic_hover_base(world).unwrap_or(0.0)
        } else {
            self.remote_soles.entries.get(&owner).and_then(AutoHover::value)?
        };
        Some(skeleton.neutral_root_height - height * 0.5 - adjustment)
    }

    /// Keep the raw estimate for immediate offset previews, without decoding
    /// or skinning the attachments again. Unfinished outfits have no baseline.
    pub fn automatic_hover_base(&self, world: &World) -> Option<f32> {
        world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i))?;
        if !world.hover_height_available() || !world.hover_height.ready() {
            return None;
        }
        self.auto_hover.value()
    }

    /// Called after the rest skeleton has been refreshed, before scene sync.
    /// Foot placement also needs the neutral sole measurement when automatic
    /// height is off; only `enabled` permits changing the user's hover height.
    /// Snapshots contain only Arc handles and transforms; decoding, skinning
    /// and vertex scans use the scene's low-priority pool. Old jobs are ignored
    /// after another wear/detach; HUDs without avatar foot skin cannot trigger
    /// a recalibration.
    pub fn update_auto_hover(&mut self, world: &mut World, enabled: bool, offset: f32, commit: bool, now: Instant) {
        self.update_remote_soles(world, now);
        self.foot_scope.log(self.ground_avatar_feet, self.remote_soles.entries.len(), now);
        if !enabled && !self.ground_avatar_feet {
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
        let Some(skeleton) = self.skeletons.get(&world.agent_id) else {
            return;
        };
        self.auto_hover.prepare_base(&self.avatar_lib.rig, &skeleton.base);
        let Some(snapshot) = self.sole_snapshot(world, world.agent_id, &self.auto_hover) else {
            return;
        };
        let ready = snapshot.ready;
        let completed = self.auto_hover.update(snapshot, &self.jobs, now);
        let offset = crate::world::hover_height::bounded(offset).unwrap_or(0.0);
        let offset_changed = self.auto_hover.offset != Some(offset);
        self.auto_hover.offset = enabled.then_some(offset);
        if enabled
            && ready
            && (completed || offset_changed)
            && let Some(height) = self.auto_hover.value()
        {
            world.hover_height.preview(height + offset);
            log::info!(
                "auto hover height: {:.3} m (raw {height:.3} m, offset {offset:.3} m)",
                world.hover_height.value()
            );
            if commit {
                world.hover_height.commit();
            }
        }
    }

    /// Read-only remote calibration cannot preview or publish hover settings.
    /// Its work is independent of our account's hover capability/readiness.
    fn update_remote_soles(&mut self, world: &World, now: Instant) {
        if !self.ground_avatar_feet || crate::settings::foot_height_limit(self.ground_feet_max_height) == 0.0 {
            self.remote_soles = RemoteSoles::default();
            return;
        }
        let mut owners = std::mem::take(&mut self.remote_soles.owners);
        owners.clear();
        owners.extend(world.avatar_poses.keys().copied().filter(|owner| {
            *owner != world.agent_id
                && self.foot_scope.allows(*owner)
                && self.skeletons.contains_key(owner)
                && world
                    .objects
                    .index_of_uuid(owner)
                    .and_then(|index| world.objects.get(index))
                    .is_some_and(|object| object.is_avatar())
        }));
        self.remote_soles.entries.retain(|owner, _| {
            world.avatar_poses.contains_key(owner)
                && self.foot_scope.allows(*owner)
                && *owner != world.agent_id
                && self.skeletons.contains_key(owner)
                && world
                    .objects
                    .index_of_uuid(owner)
                    .and_then(|index| world.objects.get(index))
                    .is_some_and(|object| object.is_avatar())
        });
        if owners.is_empty() {
            self.remote_soles.cursor = 0;
            self.remote_soles.owners = owners;
            return;
        }
        let count = owners.len().min(4);
        for step in 0..count {
            let owner = owners[(self.remote_soles.cursor + step) % owners.len()];
            let Some(skeleton) = self.skeletons.get(&owner) else { continue };
            let state = self.remote_soles.entries.entry(owner).or_default();
            if state.checked_at.is_some_and(|at| now.saturating_duration_since(at) < SETTLE) {
                continue;
            }
            state.checked_at = Some(now);
            state.second_life = self.auto_hover.second_life;
            state.prepare_base(&self.avatar_lib.rig, &skeleton.base);
            let Some(snapshot) = self
                .remote_soles
                .entries
                .get(&owner)
                .and_then(|state| self.sole_snapshot(world, owner, state))
            else {
                continue;
            };
            if let Some(state) = self.remote_soles.entries.get_mut(&owner) {
                state.update(snapshot, &self.jobs, now);
            }
        }
        self.remote_soles.cursor = (self.remote_soles.cursor + count) % owners.len();
        self.remote_soles.owners = owners;
    }

    /// Snapshot only handles, neutral transforms and the outfit signature.
    /// Decoding, morphing and skinning remain on the low-priority job pool.
    fn sole_snapshot(&self, world: &World, avatar: uuid::Uuid, state: &AutoHover) -> Option<SoleSnapshot> {
        let owner = world.objects.index_of_uuid(&avatar).and_then(|index| world.objects.get(index))?;
        let skeleton = self.skeletons.get(&avatar)?;
        if !skeleton.neutral_root_height.is_finite() {
            return None;
        }
        let rig = &self.avatar_lib.rig;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        avatar.hash(&mut hash);
        world.visual_params(&avatar).unwrap_or(&[]).hash(&mut hash);
        state.base_hash.hash(&mut hash);
        state.second_life.hash(&mut hash);
        let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, state.second_life)?;
        let matrices = &state.matrices;
        let ground = skeleton.neutral_root_height - 0.5 * box_height;
        bits(&[ground], &mut hash);
        let mut surfaces = Vec::new();
        let mut ready = true;
        for &root_idx in world.objects.children_of(&owner.key) {
            let Some(root) = world.objects.get(root_idx) else { continue };
            let point = root.attachment_point();
            let ap = self.avatar_lib.attach_points.get(&point);
            // A Blender fitting preview must never change the hover value
            // sent to the simulator or continuously invalidate calibration.
            if root.state == 0 || root.is_client_preview() {
                continue;
            }
            if ((31..=38).contains(&point) || ap.is_some_and(|a| a.hud)) && !has_rigged_foot_mesh(self, world, root_idx, avatar) {
                continue;
            }
            (root.full_id, point).hash(&mut hash);
            let on_foot = ap.and_then(|a| a.joint).is_some_and(|j| lower_leg_joint(rig, j));
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
                            if mesh.header.skin.is_some() && skin.is_none() {
                                // A rigged mesh awaiting its skin is not a
                                // rigid object or an absent pair of shoes.
                                ready = false;
                            } else {
                                // Worn animesh has its own controller and does not
                                // deform with the avatar's feet.
                                let own_rig = skin.is_some() && Self::skeleton_owner(world, idx).is_some_and(|s| s.0 == avatar);
                                if own_rig || (skin.is_none() && on_foot) {
                                    surfaces.push(Surface::Mesh {
                                        mesh: Box::new(mesh),
                                        rigid: (!own_rig).then(|| model * Mat4::from_scale(object.visual_scale())),
                                    });
                                }
                            }
                        } else {
                            // Rendering can omit a failed asset; calibration
                            // must retain the last height for an incomplete outfit.
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
        Some(SoleSnapshot {
            signature: hash.finish(),
            ready,
            estimate: Estimate {
                lib: self.avatar_lib.clone(),
                base: skeleton.base.clone(),
                ground,
                surfaces,
                values: world.visual_params(&avatar).unwrap_or(&[]).to_vec(),
            },
            body_height: skeleton.neutral_body_height,
            shape_hover: skeleton.neutral_shape_hover,
            box_height,
            root_height: skeleton.neutral_root_height,
        })
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
            let (joint, inv) = rig.skin_bind(name, *inv);
            matrices.get(joint).copied().unwrap_or(Mat4::IDENTITY) * inv
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
                vertex.joints[k] = (face.joints[i][k] as usize).min(skin.joint_names.len() - 1) as u8;
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

/// A mesh body supplies its own feet; otherwise measure the system feet,
/// with their shape morphs and skinning, even with no worn attachment.
fn system_foot_min(lib: &AvatarLibrary, values: &[u8], matrices: &[Mat4]) -> Option<f32> {
    let part = lib.parts.iter().find(|part| part.name == "lower")?;
    let weights = lib.shape_params.effective_weights(values);
    let weights = super::morph::part_weights(part, &weights).map_or_else(|| vec![0.0; part.morphs.len()], |(weights, _)| weights);
    let (geometry, ..) = super::morph::morphed(part, &weights);
    let skin = geometry.skin.as_ref()?;
    let transforms: Vec<_> = matrices
        .iter()
        .zip(&lib.rig.default_world_inv)
        .map(|(world, inv)| *world * *inv)
        .collect();
    let feet: Vec<_> = (0..lib.rig.len()).map(|joint| foot_joint(&lib.rig, joint)).collect();
    let mut min = None;
    for &index in &geometry.indices {
        let i = index as usize;
        if let (Some(vertex), Some(skin)) = (geometry.vertices.get(i), skin.get(i))
            && let Some(point) = skinned_point(Vec3::from_array(vertex.pos), skin, &transforms, &feet)
        {
            lower(&mut min, point);
        }
    }
    min
}

impl Estimate {
    fn calculate(self) -> Option<Measurement> {
        let rig = &self.lib.rig;
        let matrices = rig.world_with(&rig.local_rot, Vec3::ZERO, &self.base.local_pos, &self.base.scale);
        let surfaces = self.surfaces.len();
        let (min, rigged_feet) = self
            .surfaces
            .into_par_iter()
            .map(|surface| -> Result<(Option<f32>, bool), ()> {
                let mut min = None;
                let rigged = matches!(&surface, Surface::Mesh { rigid: None, .. });
                match surface {
                    Surface::Geometry { geom, model } => {
                        for face in geom.pick_faces.iter().flatten() {
                            for &index in &face.indices {
                                if let Some(p) = face.positions.get(index as usize) {
                                    lower(&mut min, model.transform_point3(Vec3::from_array(*p)));
                                }
                            }
                        }
                        let (size, rotation, center) = model.to_scale_rotation_translation();
                        log::info!(
                            "auto hover rigid surface: size {size:.3?}, neutral center {center:.3?}, rotation {rotation:.4?}, lowest {min:?}"
                        );
                    }
                    Surface::Mesh { mesh, rigid } => {
                        // the finest level held (the high one, as LLVOAvatar reads)
                        let section = mesh.lod_section(3).ok_or(())?;
                        let faces = aurora_assets::decode_mesh_lod(section).map_err(|_| ())?;
                        if let Some(z) = mesh_min(&faces, mesh.skin.as_deref(), rigid, rig, &matrices) {
                            lower(&mut min, Vec3::Z * z);
                        }
                        if let Some(model) = rigid {
                            let (size, rotation, center) = model.to_scale_rotation_translation();
                            log::info!(
                                "auto hover rigid surface: size {size:.3?}, neutral center {center:.3?}, rotation {rotation:.4?}, lowest {min:?}"
                            );
                        }
                    }
                }
                Ok((min, rigged && min.is_some()))
            })
            .try_reduce(
                || (None, false),
                |left, right| {
                    let min = match (left.0, right.0) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    Ok((min, left.1 || right.1))
                },
            )
            .ok()?;
        let min = if rigged_feet {
            min
        } else {
            match (min, system_foot_min(&self.lib, &self.values, &matrices)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
        };
        // Apply the user offset before the final server bounds, otherwise a
        // large raw estimate followed by a negative offset clamps too early.
        let height = min.map_or(0.0, |z| self.ground - z);
        log::info!(
            "auto hover estimate: reference {:.3} m, lowest {min:?}, raw {height:.3} m, neutral pelvis {:.3} m, {surfaces} surfaces",
            self.ground,
            matrices[rig.pelvis].w_axis.z
        );
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
    use glam::{Quat, Vec4};

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
    fn standing_physical_box_uses_shape_limits_and_havok_padding_without_live_state() {
        assert_eq!(standing_box_height(1.9, 0.0, true), Some(2.1));
        assert!((standing_box_height(1.9, 0.2, true).unwrap() - 2.3).abs() < 1e-6);
        assert!((standing_box_height(0.5, 0.0, true).unwrap() - 1.3).abs() < 1e-6);
        assert!((standing_box_height(4.0, 0.0, true).unwrap() - 2.65).abs() < 1e-6);
        assert_eq!(standing_box_height(1.9, 0.2, false), Some(1.9));
        assert_eq!(standing_box_height(f32::NAN, 0.0, true), None);
        assert_eq!(standing_box_height(1.9, f32::INFINITY, true), None);
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
    fn rigged_skin_parity_foot_measurement_blends_unknown_joints_as_the_pelvis() {
        let lib = library();
        let mut matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        matrices[lib.rig.pelvis] = Mat4::from_translation(Vec3::Z);
        let mut skin = skin();
        skin.joint_names[1] = "obsolete_joint".into();
        let mut geometry = face();
        geometry.positions[1][2] = 0.0;
        geometry.weights.fill([0.5, 0.5, 0.0, 0.0]);
        let actual = mesh_min(&[geometry], Some(&skin), None, &lib.rig, &matrices).unwrap();
        // The drawn minimum is -.1 -.2 + .5 * pelvis height (1 m).
        assert!(
            (actual - 0.2).abs() < 1e-5,
            "foot measurement disagrees with pelvis skinning: {actual}"
        );
    }

    #[test]
    fn rigged_skin_parity_foot_measurement_bounds_indices_to_the_last_joint() {
        let lib = library();
        let matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        let mut skin = skin();
        skin.joint_names.swap(0, 1);
        let mut geometry = face();
        geometry.positions[1][2] = 0.0;
        geometry.joints.fill([254, 0, 0, 0]);
        geometry.weights.fill([1.0, 0.0, 0.0, 0.0]);
        let actual = mesh_min(&[geometry], Some(&skin), None, &lib.rig, &matrices);
        assert!(
            actual.is_some_and(|height| (height + 0.3).abs() < 1e-5),
            "invalid index must reach the final foot joint: {actual:?}"
        );
    }

    #[test]
    fn foot_aliases_toes_fitted_feet_and_hind_feet_are_supported() {
        let lib = library();
        for name in ["lFoot", "rFoot", "mToeLeft", "R_FOOT", "mHindLimb3Left"] {
            assert!(foot_joint(&lib.rig, lib.rig.names[name]), "{name}");
        }
        assert!(!foot_joint(&lib.rig, lib.rig.names["mKneeLeft"]));
        for name in ["mKneeLeft", "mKneeRight", "mAnkleLeft", "mFootRight", "mToeLeft"] {
            assert!(lower_leg_joint(&lib.rig, lib.rig.names[name]), "rigid lower-leg point {name}");
        }
        for name in ["mHipLeft", "mChest", "mWristRight", "mFaceNoseCenter"] {
            assert!(!lower_leg_joint(&lib.rig, lib.rig.names[name]), "unrelated rigid point {name}");
        }
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
        // Geometry/calibration fixtures have no main camera. Scope selection
        // and the saved budget are exercised separately below.
        scene.foot_scope = super::super::foot_scope::Scope::from_mode("all");
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
        assert!(
            (world.hover_height.value() - scene.automatic_hover_base(&world).unwrap() + 0.03).abs() < 1e-5,
            "system feet keep the fixed offset after shoe removal"
        );
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
            values: Vec::new(),
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
        assert_eq!(world.hover_height.value(), scene.automatic_hover_base(&world).unwrap());
    }

    #[test]
    fn blender_live_shoes_do_not_change_hover_or_invalidate_manual_height() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        settle(&mut scene, &mut world, now);
        let calibration = scene.auto_hover.finished;
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        let parent = world
            .objects
            .index_of_uuid(&world.agent_id)
            .and_then(|i| world.objects.get(i))
            .expect("avatar")
            .key;
        world
            .objects
            .insert_preview_attachment(parent, crate::demo::auto_hover::rigged_object());
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert_eq!(world.hover_height.value(), 0.35);
        assert_eq!(scene.auto_hover.finished, calibration);
    }

    #[test]
    fn aurora_calibrates_shoes_without_writing_the_manual_hover_height() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        scene.ground_avatar_feet = true;
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        let Some(aurora_net::NetCommand::SetHoverHeight { request, .. }) = world.hover_height.next_update(Some(1)) else {
            panic!("initial manual height");
        };
        world.hover_height.reply(request, 1, true);
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, false, 0.5, true, now);
        for _ in 0..1000 {
            scene.update_auto_hover(&mut world, false, 0.5, true, now + SETTLE);
            if scene.automatic_hover_base(&world).is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let baseline = scene
            .automatic_hover_base(&world)
            .expect("IK needs neutral shoe calibration even with automatic height off");
        assert_eq!(world.hover_height.value(), 0.35);
        assert!(
            world.hover_height.next_update(Some(1)).is_none(),
            "calibration must not publish a hover change"
        );
        scene.update_auto_hover(&mut world, true, 0.0, true, now + Duration::from_secs(1));
        assert_eq!(
            world.hover_height.value(),
            baseline,
            "enabling automatic height must apply the cached estimate"
        );
    }

    #[test]
    fn remote_soles_use_each_outfit_and_keep_contact_without_own_hover_support() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = true;
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        let Some(aurora_net::NetCommand::SetHoverHeight { request, .. }) = world.hover_height.next_update(Some(1)) else {
            panic!("initial manual height");
        };
        world.hover_height.reply(request, 1, true);
        let residents = [
            remote_shoes(&mut scene, &mut world, now, 1, 0.0),
            remote_shoes(&mut scene, &mut world, now, 2, 0.07),
        ];
        scene.ground_avatar_feet = true;
        world.regions.get_mut(&1).unwrap().avatar_hover_height = false;
        for resident in &residents {
            let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
            world.objects.get_mut(index).unwrap().velocity = Vec3::ZERO;
            assert!(
                scene.grounding_frame(&world, index, now).is_none(),
                "an uncalibrated resident must keep the source height"
            );
            world.objects.get_mut(index).unwrap().velocity = Vec3::X * 0.2;
        }
        for frame in 0..90 {
            remote_step(&mut scene, &mut world, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        let baselines = residents
            .each_ref()
            .map(|resident| scene.remote_soles.entries[&resident.avatar].value().unwrap());
        assert!(
            (baselines[1] - baselines[0] - 0.07).abs() < 0.0001,
            "shoe depths were mixed between residents"
        );
        let mut frame = 90;
        for while_moving in [false, true] {
            scene.ground_feet_while_moving = while_moving;
            for moving in [true, false] {
                for resident in &residents {
                    let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
                    world.objects.get_mut(index).unwrap().velocity = if moving { Vec3::X * 0.2 } else { Vec3::ZERO };
                }
                for _ in 0..90 {
                    let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
                    remote_step(&mut scene, &mut world, at);
                    for resident in &residents {
                        let sole = rendered_remote_sole(&scene, &world, resident, at);
                        assert!(
                            sole.abs() < 0.003,
                            "remote sole = {sole:.6}, moving={moving}, placement while moving={while_moving}"
                        );
                    }
                    frame += 1;
                }
            }
        }
        assert_eq!(world.hover_height.value(), 0.35);
        assert!(
            world.hover_height.next_update(Some(1)).is_none(),
            "observing residents published our hover height"
        );
        for (resident, baseline) in residents.iter().zip(baselines) {
            assert_eq!(
                scene.remote_soles.entries[&resident.avatar].value(),
                Some(baseline),
                "motion invalidated a neutral calibration"
            );
        }
        scene.ground_feet_while_moving = false;
        let _replacement = remote_shoes(&mut scene, &mut world, now + Duration::from_secs(20), 1, 0.04);
        for frame in 0..90 {
            remote_step(
                &mut scene,
                &mut world,
                now + Duration::from_secs(20) + Duration::from_secs_f32(frame as f32 / 60.0),
            );
        }
        let changed = scene.remote_soles.entries[&residents[0].avatar].value().unwrap();
        assert!(
            (changed - baselines[0] - 0.04).abs() < 0.0001,
            "changed shoes kept an old sole reference"
        );
        assert_eq!(
            scene.remote_soles.entries[&residents[1].avatar].value(),
            Some(baselines[1]),
            "an outfit change invalidated another resident"
        );
        let index = world.objects.index_of_uuid(&residents[0].avatar).unwrap();
        world.objects.get_mut(index).unwrap().velocity = Vec3::ZERO;
        remote_step(&mut scene, &mut world, now + Duration::from_secs(25));
        assert!(rendered_remote_sole(&scene, &world, &residents[0], now + Duration::from_secs(25)).abs() < 0.003);
        let stale = scene.remote_soles.entries[&residents[0].avatar].tx.clone();
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![residents[0].object, residents[0].shoe],
        });
        remote_step(&mut scene, &mut world, now + Duration::from_secs(30));
        assert!(!scene.remote_soles.entries.contains_key(&residents[0].avatar));
        assert!(stale.send((0, None)).is_err(), "retired resident still accepts jobs");
        assert_eq!(scene.remote_soles.entries[&residents[1].avatar].value(), Some(baselines[1]));
    }

    #[test]
    fn remote_soles_preserve_absolute_pelvis_pose_without_crouching() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = true;
        let resident = remote_shoes(&mut scene, &mut world, now, 1, 0.0);
        let lib = world.avatar_lib.clone();
        let mesh = scene.meshes.view(&resident.mesh).unwrap();
        let mut skin = mesh.skin.as_deref().unwrap().clone();
        skin.lock_scale_if_joint_position = false;
        skin.alt_inverse_bind = skin
            .joint_names
            .iter()
            .map(|name| {
                Mat4::from_translation(if name == "mPelvis" {
                    Vec3::Z * 0.28
                } else {
                    lib.rig.local_pos[lib.rig.names[name]]
                })
            })
            .collect();
        scene.meshes.insert_local(
            resident.mesh,
            Arc::new(mesh.bytes.first().unwrap().to_vec()),
            mesh.header,
            Some(Arc::new(skin)),
        );
        crate::demo::auto_hover::hold_pelvis(&mut world, &mut scene);
        world.apply(NetEvent::AvatarAnimations {
            avatar: resident.avatar,
            sources: Vec::new(),
            anims: world.own_signals().iter().map(|signal| (signal.id, signal.sequence)).collect(),
        });
        scene.ground_avatar_feet = true;
        for frame in 0..90 {
            remote_step(&mut scene, &mut world, now + Duration::from_secs_f32(frame as f32 / 60.0));
        }
        let source = world.avatar_poses[&resident.avatar].clone();
        let at = now + Duration::from_secs_f32(89.0 / 60.0);
        assert!(
            rendered_remote_sole(&scene, &world, &resident, at).abs() < 0.003,
            "source feet already touch the floor"
        );
        let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
        world.objects.get_mut(index).unwrap().velocity = Vec3::ZERO;
        for frame in 90..180 {
            let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
            remote_step(&mut scene, &mut world, at);
            assert!(rendered_remote_sole(&scene, &world, &resident, at).abs() < 0.003);
            let placed = &world.avatar_poses[&resident.avatar];
            let hip = lib.rig.names["mHipLeft"];
            let drop = placed[hip].w_axis.z - source[hip].w_axis.z;
            assert!(drop.abs() < 0.005, "placement lowered an already grounded body by {drop:.6} m");
            for name in ["mHipLeft", "mHipRight", "mKneeLeft", "mKneeRight"] {
                let joint = lib.rig.names[name];
                let before = source[joint].to_scale_rotation_translation().1;
                let after = placed[joint].to_scale_rotation_translation().1;
                assert!(before.angle_between(after).abs() < 0.1, "placement changed {name} on flat ground");
            }
        }
    }

    struct RemoteShoes {
        avatar: uuid::Uuid,
        object: u32,
        shoe: u32,
        mesh: uuid::Uuid,
    }

    fn remote_shoes(scene: &mut Scene, world: &mut World, now: Instant, serial: u32, drop: f32) -> RemoteShoes {
        use aurora_assets::import_mesh::{encode, process::Physics};
        let lib = world.avatar_lib.clone();
        let mut model = crate::demo::auto_hover::rigged_model(&lib);
        for vertex in model.faces.iter_mut().take(2).flat_map(|face| &mut face.vertices) {
            vertex.position[2] -= drop;
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
        let fixture = RemoteShoes {
            avatar: uuid::Uuid::from_u128(0xd0d0_bbbb_0000_0000_0000_0000_0000_0000 + u128::from(serial)),
            object: 9500 + serial,
            shoe: 9600 + serial,
            mesh: uuid::Uuid::from_u128(0xd0d0_cccc_0000_0000_0000_0000_0000_0000 + u128::from(serial)),
        };
        scene.meshes.insert_local(fixture.mesh, Arc::new(data), header, skin);
        let mut avatar = crate::demo::action_avatar(false);
        avatar.local_id = fixture.object;
        avatar.full_id = fixture.avatar;
        avatar.velocity = Vec3::X * 0.2;
        avatar.foot_plane = Some(Vec4::new(0.0, 0.0, 1.0, 0.05));
        let mut shoe = crate::demo::auto_hover::rigged_object();
        shoe.local_id = fixture.shoe;
        shoe.full_id = uuid::Uuid::from_u128(0xd0d0_dddd_0000_0000_0000_0000_0000_0000 + u128::from(serial));
        shoe.parent_id = fixture.object;
        shoe.extra.sculpt.as_mut().unwrap().texture = fixture.mesh;
        shoe.volume.sculpt = shoe.extra.sculpt;
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![avatar, shoe],
        });
        let index = world.objects.index_of_uuid(&fixture.avatar).unwrap();
        scene.skeleton_of(world, fixture.avatar, index, now);
        let skeleton = &scene.skeletons[&fixture.avatar];
        let half = 0.5 * standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
        let neutral = lib
            .rig
            .world_with(&lib.rig.local_rot, Vec3::ZERO, &skeleton.base.local_pos, &skeleton.base.scale);
        let mesh = scene.meshes.view(&fixture.mesh).unwrap();
        let faces = aurora_assets::decode_mesh_lod(mesh.lod_section(3).unwrap()).unwrap();
        let lowest = mesh_min(&faces, mesh.skin.as_deref(), None, &lib.rig, &neutral).unwrap();
        let hover = skeleton.neutral_root_height - half - lowest;
        world.objects.get_mut(index).unwrap().position = Vec3::new(serial as f32 * 3.0, 0.0, half + hover);
        scene.palette_base(fixture.avatar);
        fixture
    }

    fn remote_step(scene: &mut Scene, world: &mut World, at: Instant) {
        scene.update_poses(world, Vec3::ZERO, at);
        scene.update_auto_hover(world, false, 0.0, true, at);
    }

    fn rendered_remote_sole(scene: &Scene, world: &World, resident: &RemoteShoes, at: Instant) -> f32 {
        let lib = &world.avatar_lib;
        let mesh = scene.meshes.view(&resident.mesh).unwrap();
        let faces = aurora_assets::decode_mesh_lod(mesh.lod_section(3).unwrap()).unwrap();
        let matrices: Vec<_> = world.avatar_poses[&resident.avatar]
            .iter()
            .zip(&lib.rig.default_world_inv)
            .map(|(pose, inv)| *pose * inv.inverse())
            .collect();
        let lowest = mesh_min(&faces, mesh.skin.as_deref(), None, &lib.rig, &matrices).unwrap();
        let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
        Scene::object_transform(world, index, at, 0).unwrap().0.z - lib.pelvis.z + lowest
    }

    #[test]
    fn remote_soles_bound_inspection_work_and_reach_every_visible_resident() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = true;
        for serial in 1..=9 {
            remote_shoes(&mut scene, &mut world, now, serial, serial as f32 * 0.01);
        }
        scene.ground_avatar_feet = true;
        for frame in 0..90 {
            let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
            remote_step(&mut scene, &mut world, at);
            assert!(
                scene
                    .remote_soles
                    .entries
                    .values()
                    .filter(|state| state.checked_at == Some(at))
                    .count()
                    <= 4
            );
        }
        assert_eq!(scene.remote_soles.entries.len(), 9);
        assert!(
            scene.remote_soles.entries.values().all(|state| state.value().is_some()),
            "round-robin scheduling starved a resident"
        );
        world.avatar_poses.clear();
        scene.update_remote_soles(&world, now + Duration::from_secs(2));
        assert!(scene.remote_soles.entries.is_empty(), "unposed residents retained their shoe cache");
    }

    #[test]
    fn remote_saved_budget_updates_calibration_live_and_keeps_our_avatar() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = false;
        world.agent.position = Vec3::new(-5.0, 0.0, 1.0);
        let residents: Vec<_> = (1..=16)
            .map(|serial| remote_shoes(&mut scene, &mut world, now, serial, 0.0))
            .collect();
        scene.foot_scope = super::super::foot_scope::Scope::default();
        scene.ground_avatar_feet = true;
        scene.ground_feet_while_moving = true;
        let eye = Vec3::Z;
        let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        let view = super::super::CullView::new(
            projection * glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::X, Vec3::Z),
            eye,
            900.0,
            std::f32::consts::FRAC_PI_2,
        );
        let mut frame = 0;
        for maximum in [5, 2, 0, 12] {
            scene.set_foot_scope_limit(maximum);
            for _ in 0..90 {
                let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
                scene.update_foot_scope(&world, view, at);
                remote_step(&mut scene, &mut world, at);
                frame += 1;
            }
            assert_eq!(scene.remote_soles.entries.len(), maximum as usize);
            for (number, resident) in residents.iter().enumerate() {
                let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
                assert_eq!(scene.grounding_frame(&world, index, now).is_some(), number < maximum as usize);
            }
            let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
            assert!(
                scene.grounding_frame(&world, own, now).is_some(),
                "our avatar stays corrected even with a zero budget"
            );
        }
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert!(
            scene.grounding_frame(&world, own, now).is_some(),
            "our avatar stays corrected outside the camera"
        );
        let away = super::super::CullView::new(
            projection * glam::camera::rh::view::look_at_mat4(eye, eye - Vec3::X, Vec3::Z),
            eye,
            900.0,
            std::f32::consts::FRAC_PI_2,
        );
        let at = now + Duration::from_secs(8);
        scene.update_foot_scope(&world, away, at);
        remote_step(&mut scene, &mut world, at);
        assert!(scene.remote_soles.entries.is_empty(), "out-of-view residents still calibrate");
        for resident in residents {
            let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
            assert!(scene.grounding_frame(&world, index, at).is_none());
        }
        assert!(scene.grounding_frame(&world, own, at).is_some());
    }

    #[test]
    fn remote_saved_budget_acquisition_stays_continuous_through_camera_reentry() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = true;
        let resident = remote_shoes(&mut scene, &mut world, now, 1, 0.0);
        let index = world.objects.index_of_uuid(&resident.avatar).unwrap();
        world.objects.get_mut(index).unwrap().position.z -= 0.18;
        // A moving resident's render root is already clamped above the land.
        // Keep this fixture stationary to exercise actual source penetration.
        world.objects.get_mut(index).unwrap().velocity = Vec3::ZERO;
        scene.foot_scope = super::super::foot_scope::Scope::default();
        scene.ground_avatar_feet = true;
        scene.ground_feet_while_moving = true;
        let eye = Vec3::Z;
        let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(std::f32::consts::FRAC_PI_2, 1.0, 0.1);
        let view = |direction| {
            super::super::CullView::new(
                projection * glam::camera::rh::view::look_at_mat4(eye, eye + direction, Vec3::Z),
                eye,
                900.0,
                std::f32::consts::FRAC_PI_2,
            )
        };
        let mut previous = None::<Vec<Mat4>>;
        for frame in 0..540 {
            let at = now + Duration::from_secs_f32(frame as f32 / 60.0);
            scene.set_foot_scope_limit(if frame < 60 || (300..390).contains(&frame) { 0 } else { 5 });
            scene.update_foot_scope(&world, view(if frame == 180 { -Vec3::X } else { Vec3::X }), at);
            remote_step(&mut scene, &mut world, at);
            let rig = &world.avatar_lib.rig;
            let matrices: Vec<_> = world.avatar_poses[&resident.avatar]
                .iter()
                .zip(&rig.default_world_inv)
                .map(|(pose, inverse)| *pose * inverse.inverse())
                .collect();
            if let Some(previous) = &previous {
                for name in ["mPelvis", "mKneeLeft", "mKneeRight", "mAnkleLeft", "mAnkleRight"] {
                    let joint = rig.names[name];
                    let distance = matrices[joint].w_axis.distance(previous[joint].w_axis);
                    assert!(distance < 2.5 / 60.0, "{name} jumped {distance:.6} m at frame {frame}");
                }
            }
            previous = Some(matrices);
            if frame == 59 || frame == 389 {
                let sole = rendered_remote_sole(&scene, &world, &resident, at);
                assert!(
                    sole < -0.17,
                    "the source penetration was not reproduced: sole {sole:.6} m at frame {frame}"
                );
            }
            if [179, 299, 539].contains(&frame) {
                let sole = rendered_remote_sole(&scene, &world, &resident, at);
                assert!(sole.abs() < 0.003, "sole {sole:.6} m after acquisition at frame {frame}");
            }
        }
    }

    #[test]
    fn aurora_idle_keeps_measured_shoe_soles_without_changing_manual_hover() {
        for while_moving in [false, true] {
            let (mut world, mut scene, now) = fixture();
            let lib = world.avatar_lib.clone();
            crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![crate::demo::auto_hover::rigged_object()],
            });
            let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
            let skeleton = &scene.skeletons[&world.agent_id];
            let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
            world.objects.get_mut(owner).unwrap().position.z = 0.5 * box_height;
            world.objects.get_mut(owner).unwrap().foot_plane = Some(Vec4::new(0.0, 0.0, 1.0, 0.05));
            world.agent.position.z = 0.5 * box_height;
            world.agent.flying = false;
            settle(&mut scene, &mut world, now);
            let manual = world.hover_height.value();
            scene.ground_avatar_feet = true;
            scene.ground_feet_while_moving = while_moving;
            scene.palette_base(world.agent_id);
            let mesh = scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap();
            let faces = aurora_assets::decode_mesh_lod(mesh.lod_section(3).unwrap()).unwrap();
            for frame in 0..=180 {
                let at = now + Duration::from_secs(1) + Duration::from_secs_f32(frame as f32 / 60.0);
                scene.ground_feet_own_moving = frame < 60;
                scene.update_auto_hover(&mut world, false, 0.0, true, at);
                scene.update_poses(&mut world, Vec3::ZERO, at);
                let matrices: Vec<_> = world.avatar_poses[&world.agent_id]
                    .iter()
                    .zip(&lib.rig.default_world_inv)
                    .map(|(posed, inv)| *posed * inv.inverse())
                    .collect();
                let lowest = mesh_min(&faces, mesh.skin.as_deref(), None, &lib.rig, &matrices).unwrap();
                let root = Scene::object_transform(&world, owner, at, 0).unwrap().0;
                let sole = root.z - lib.pelvis.z + lowest;
                assert!(sole.abs() < 0.003, "frame {frame}: stopping changed the sole height to {sole:.6} m");
                assert_eq!(world.hover_height.value(), manual, "IK must not write the manual hover height");
            }
        }
    }

    #[test]
    fn neutral_geometry_aligns_with_the_standing_physical_box_and_ignores_streamed_size() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        scene.palette_base(world.agent_id);
        scene.neutral_pose_test = Some(true);
        scene.update_poses(&mut world, Vec3::ZERO, now);
        // Put the standing physical box bottom at zero in an empty world.
        let skeleton = &scene.skeletons[&world.agent_id];
        let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
        world.objects.get_mut(owner).unwrap().position.z = 0.5 * box_height;
        world.agent.position.z = 0.5 * box_height;
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
        let stamp = scene.auto_hover.finished;
        for (i, height) in [12.0, 0.0, f32::NAN, 0.5, 1.9].into_iter().enumerate() {
            world.objects.get_mut(owner).unwrap().scale.z = height;
            settle(&mut scene, &mut world, now + Duration::from_secs(i as u64 + 1));
            assert_eq!(
                world.hover_height.value(),
                previous,
                "physical size is not the anatomical reference"
            );
            assert_eq!(scene.auto_hover.finished, stamp);
        }
        let root = Scene::object_transform(&world, owner, now, 0).unwrap().0;
        assert!((root.z - lib.pelvis.z + lowest).abs() < 1e-5);
    }

    #[test]
    fn rigged_foot_vertices_measure_the_same_at_every_attachment_point() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        let mut points: Vec<_> = lib.attach_points.keys().copied().collect();
        points.sort_unstable();
        let mut expected = None;
        for (i, point) in points.into_iter().enumerate() {
            let mut object = crate::demo::auto_hover::rigged_object();
            object.state = point.rotate_left(4);
            object.position = Vec3::new(0.3, -0.2, 0.5);
            object.rotation = Quat::from_rotation_y(0.7);
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![object],
            });
            settle(&mut scene, &mut world, now + Duration::from_secs(i as u64));
            let height = scene.automatic_hover_base(&world).unwrap();
            assert_eq!(height, *expected.get_or_insert(height), "rigged shoes at attachment {point}");
        }
    }

    #[test]
    fn mesh_body_deformation_keeps_the_shape_physical_box_and_compensates_its_rendered_root() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let before = scene.automatic_hover_base(&world).unwrap();
        let skeleton = &scene.skeletons[&world.agent_id];
        let body_height = skeleton.neutral_body_height;
        let chest = lib.rig.names["mChest"];
        let torso = lib.rig.names["mTorso"];
        let expected_shift = 0.2 * skeleton.base.scale[torso].z;
        let mesh = scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap();
        let mut skin = mesh.skin.as_deref().unwrap().clone();
        skin.lock_scale_if_joint_position = false;
        skin.joint_names.push("mChest".into());
        skin.inverse_bind.push(lib.rig.default_world_inv[chest]);
        skin.alt_inverse_bind = skin
            .joint_names
            .iter()
            .map(|name| {
                let joint = lib.rig.names[name];
                let position = if joint == lib.rig.pelvis {
                    Vec3::ZERO
                } else if joint == chest {
                    skeleton.base.local_pos[chest] + Vec3::Z * 0.4
                } else {
                    skeleton.base.local_pos[joint]
                };
                Mat4::from_translation(position)
            })
            .collect();
        scene.meshes.insert_local(
            crate::demo::auto_hover::MESH,
            Arc::new(mesh.bytes.first().unwrap().to_vec()),
            mesh.header,
            Some(Arc::new(skin)),
        );
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert_eq!(
            scene.skeletons[&world.agent_id].neutral_body_height, body_height,
            "mesh metadata must not resize the simulator's shape-derived physical box"
        );
        let after = scene.automatic_hover_base(&world).unwrap();
        assert!(
            (after - before - expected_shift).abs() < 1e-5,
            "the root's body-size correction must be compensated: {before} -> {after}"
        );
    }

    #[test]
    fn system_feet_are_measured_even_without_worn_geometry() {
        let (mut world, mut scene, now) = fixture();
        settle(&mut scene, &mut world, now);
        assert!(
            scene.auto_hover.measurement.unwrap().lowest.is_some(),
            "bare feet must have a geometric minimum"
        );
        let skeleton = &scene.skeletons[&world.agent_id];
        let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        world.objects.get_mut(owner).unwrap().position.z = 0.5 * box_height;
        world.agent.position.z = 0.5 * box_height;
        scene.palette_base(world.agent_id);
        scene.neutral_pose_test = Some(true);
        scene.update_poses(&mut world, Vec3::ZERO, now);
        let lowest = system_foot_min(&scene.avatar_lib, &[], &scene.auto_hover.matrices).unwrap();
        let root = Scene::object_transform(&world, owner, now, 0).unwrap().0;
        assert!(
            (root.z - scene.avatar_lib.pelvis.z + lowest).abs() < 1e-5,
            "system sole must align with the physical box bottom"
        );
    }

    #[test]
    fn a_single_foot_prim_stilt_uses_its_full_depth_and_remeasures_after_resizing() {
        use super::super::{GeomState, ObjGpu, jobs};
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let shoe_height = world.hover_height.value();
        let skeleton = &scene.skeletons[&world.agent_id];
        let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
        let ground = skeleton.neutral_root_height - 0.5 * box_height;
        let foot = scene.auto_hover.matrices[lib.rig.names["mFootLeft"]].w_axis.z;
        let foot_base = ground - foot;
        for (i, length) in [0.6, 1.2, 2.5].into_iter().enumerate() {
            let mut object = crate::demo::auto_hover::stilt(length);
            let point = [7_u8, 27, 24][i]; // foot, left shin, right shin
            object.state = point.rotate_left(4);
            let ap = lib.attach_points[&point];
            let (anchor, _) = ap.frame(ap.joint.map(|joint| scene.auto_hover.matrices[joint]));
            object.position.z = foot - anchor.z - 0.5 * length;
            let id = object.full_id;
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![object],
            });
            let index = world.objects.index_of_uuid(&id).unwrap();
            let object = world.objects.get(index).unwrap();
            let key = Scene::object_geom_key(object, 3).unwrap();
            let mesh = aurora_prim::generate_volume(&object.visual_volume(), aurora_prim::volume::DETAIL_SCALES[3]);
            let (faces, min, max) = jobs::volume_to_faces(&mesh);
            let prepared = jobs::PreparedGeom::new(faces, min, max, true, None);
            scene.gpu.resize_with(scene.gpu.len().max(index + 1), ObjGpu::default);
            scene.gpu[index].geom = Some(key);
            scene.geom_insert(
                key,
                GeomState::Ready(Arc::new(GpuGeom {
                    faces: Vec::new(),
                    min,
                    max,
                    joint_bounds: prepared.joint_bounds,
                    pick_faces: prepared.pick_faces,
                })),
            );
            settle(&mut scene, &mut world, now + Duration::from_secs(i as u64 + 1));
            assert!(
                (scene.automatic_hover_base(&world).unwrap() - length - foot_base).abs() < 1e-5,
                "a {length} m stilt must lift the foot by its complete depth"
            );
            assert!((world.hover_height.value() - length - foot_base).abs() < 1e-5);
        }
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::STILT],
        });
        settle(&mut scene, &mut world, now + Duration::from_secs(4));
        assert!(
            (world.hover_height.value() - shoe_height).abs() < 1e-5,
            "removing the stilt restores the sole depth"
        );
    }

    #[test]
    fn auto_hover_root_translation_does_not_change_calibration_or_an_absolute_pelvis_animation() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        scene.palette_base(world.agent_id);
        crate::demo::auto_hover::hold_pelvis(&mut world, &mut scene);
        scene.update_poses(&mut world, Vec3::ZERO, now);
        settle(&mut scene, &mut world, now);
        let expected = scene.automatic_hover_base(&world).unwrap();
        let foot = lib.rig.names["mFootLeft"];
        let animated_foot = world.avatar_poses[&world.agent_id][foot];
        for (i, shift) in [Vec3::new(0.2, -0.1, 10.53), Vec3::new(-0.1, 0.3, -8.0)].into_iter().enumerate() {
            crate::demo::auto_hover::shift_pelvis(&lib, &mut scene, shift).unwrap();
            let at = now + Duration::from_secs(i as u64 + 1);
            scene.update_poses(&mut world, Vec3::ZERO, at);
            assert!(
                world.avatar_poses[&world.agent_id][foot].abs_diff_eq(animated_foot, 1e-5),
                "absolute animation already replaces the rest pelvis"
            );
            settle(&mut scene, &mut world, at);
            let base = scene.automatic_hover_base(&world).unwrap();
            assert!(
                (base - expected).abs() < 1e-5,
                "a rest pelvis shift {shift:?} changed the raw calibration from {expected} to {base}"
            );
            assert!(
                (world.hover_height.value() - expected).abs() < 1e-5,
                "root shifts must not saturate the transmitted height"
            );
        }
    }

    #[test]
    fn auto_hover_has_the_same_baseline_when_enabled_standing_flying_or_seated() {
        let mut expected = None;
        for (flying, parent, physical_size) in [
            (false, 0, 1.9),
            (true, 0, 1.9),
            (false, 999, 1.9),
            (false, 0, 12.0),
            (true, 0, 0.5),
            (false, 999, 0.0),
        ] {
            let (mut world, mut scene, now) = fixture();
            crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![crate::demo::auto_hover::rigged_object()],
            });
            let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
            let avatar = world.objects.get_mut(owner).unwrap();
            avatar.parent_id = parent;
            avatar.scale.z = physical_size;
            world.agent.flying = flying;
            world.avatar_poses.insert(
                world.agent_id,
                vec![Mat4::from_translation(Vec3::Z * 20.0); world.avatar_lib.rig.len()],
            );
            settle(&mut scene, &mut world, now);
            let base = scene
                .automatic_hover_base(&world)
                .expect("a neutral baseline exists in every movement state");
            assert_eq!(
                base,
                *expected.get_or_insert(base),
                "flying {flying}, parent {parent}, size {physical_size}"
            );
            assert_eq!(world.hover_height.value(), base);
        }
    }

    #[test]
    fn auto_hover_keeps_the_cached_baseline_and_manual_edits_across_movement_states() {
        let (mut world, mut scene, now) = fixture();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let base = scene.automatic_hover_base(&world).unwrap();
        let signature = scene.auto_hover.finished;
        world.hover_height.preview(base + 0.1);
        world.hover_height.commit();
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        for (i, (flying, parent, physical_size)) in [(true, 0, 12.0), (false, 999, 0.5), (false, 0, 1.9)].into_iter().enumerate() {
            world.agent.flying = flying;
            let avatar = world.objects.get_mut(owner).unwrap();
            avatar.parent_id = parent;
            avatar.scale.z = physical_size;
            settle(&mut scene, &mut world, now + Duration::from_secs(i as u64 + 1));
            assert_eq!(scene.automatic_hover_base(&world), Some(base));
            assert_eq!(world.hover_height.value(), base + 0.1);
            assert_eq!(scene.auto_hover.finished, signature);
            assert!(!scene.auto_hover.pending, "movement must not schedule another mesh scan");
        }
    }

    #[test]
    fn auto_hover_is_identical_in_empty_space_and_with_any_contact_plane() {
        let mut expected = None;
        for (position, plane) in [
            (Vec3::new(3.0, 4.0, 21.05), None),
            (Vec3::new(3.0, 4.0, 21.05), Some(Vec4::new(0.0, 0.0, 1.0, 20.0))),
            (Vec3::new(-100.0, 500.0, 4000.0), Some(Vec4::new(0.0, 0.6, 0.8, 3000.0))),
            (Vec3::new(0.0, 0.0, -500.0), Some(Vec4::ZERO)),
            (Vec3::new(5.0, 6.0, 2000.0), Some(Vec4::splat(f32::NAN))),
            (Vec3::new(1.0, 2.0, 30.0), Some(Vec4::new(0.0, 0.0, -1.0, 0.0))),
        ] {
            // No terrain, platform or other world geometry exists in this fixture.
            let (mut world, mut scene, now) = fixture();
            crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![crate::demo::auto_hover::rigged_object()],
            });
            let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
            let avatar = world.objects.get_mut(owner).unwrap();
            avatar.scale.z = 1.9;
            avatar.position = position;
            avatar.foot_plane = plane;
            world.agent.position = position;
            world.hover_height.preview(-1.75);
            world.hover_height.commit();
            settle(&mut scene, &mut world, now);
            assert!(
                scene.auto_hover.measurement.and_then(|m| m.lowest).is_some(),
                "calibration must measure the feet even without contact"
            );
            let height = world.hover_height.value();
            assert_eq!(height, *expected.get_or_insert(height), "position {position:?}, plane {plane:?}");
            let Some(aurora_net::NetCommand::SetHoverHeight { value, .. }) = world.hover_height.next_update(Some(1)) else {
                panic!("calibration must send the same height to the simulator");
            };
            assert_eq!(value, height);
        }
    }

    #[test]
    fn auto_hover_contact_changes_preserve_the_cached_estimate_and_manual_edits() {
        let (mut world, mut scene, now) = fixture();
        let lib = world.avatar_lib.clone();
        crate::demo::auto_hover::install_mesh(&lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        let owner = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let avatar = world.objects.get_mut(owner).unwrap();
        avatar.scale.z = 1.9;
        avatar.position.z = 41.05;
        world.agent.position.z = avatar.position.z;
        settle(&mut scene, &mut world, now);
        let base = world.hover_height.value();
        let stamp = scene.auto_hover.finished;
        world.hover_height.preview(base + 0.1);
        world.hover_height.commit();
        for (i, plane) in [
            Some(Vec4::new(0.0, 0.0, 1.0, 40.0)),
            Some(Vec4::ZERO),
            None,
            Some(Vec4::splat(f32::NAN)),
        ]
        .into_iter()
        .enumerate()
        {
            world.objects.get_mut(owner).unwrap().foot_plane = plane;
            settle(&mut scene, &mut world, now + Duration::from_secs(i as u64 + 1));
            assert_eq!(world.hover_height.value(), base + 0.1, "contact must preserve manual edits");
            assert_eq!(scene.auto_hover.finished, stamp);
            assert_eq!(scene.automatic_hover_base(&world), Some(base));
            assert!(!scene.auto_hover.pending, "contact changes must not schedule new geometry work");
        }
        scene.update_auto_hover(&mut world, true, -0.03, true, now + Duration::from_secs(8));
        assert_eq!(
            world.hover_height.value(),
            base - 0.03,
            "offset edits reuse the same independent estimate"
        );
    }

    #[test]
    fn loading_detach_stale_results_and_disable_do_not_apply_old_heights() {
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
            ))
            .unwrap();
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        assert_eq!(world.hover_height.value(), 0.42, "discard detached outfit result");
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.42);
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert_eq!(world.hover_height.value(), scene.automatic_hover_base(&world).unwrap());
    }

    #[test]
    fn auto_hover_waits_for_rigged_skin_metadata_before_committing_a_height() {
        let (mut world, mut scene, now) = fixture();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        let mesh = scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap();
        scene.meshes.insert_local(
            crate::demo::auto_hover::MESH,
            Arc::new(mesh.bytes.first().unwrap().to_vec()),
            mesh.header,
            None,
        );
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        world.hover_height.preview(0.42);
        world.hover_height.commit();
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
        assert_eq!(world.hover_height.value(), 0.42);
        assert!(!scene.auto_hover.pending, "geometry without its skin cannot be measured");
        assert_eq!(scene.automatic_hover_base(&world), None);
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        assert!(scene.auto_hover.measurement.and_then(|m| m.lowest).is_some());
        assert_ne!(world.hover_height.value(), 0.42);
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
        let box_height = standing_box_height(skeleton.neutral_body_height, skeleton.neutral_shape_hover, true).unwrap();
        let ground = skeleton.neutral_root_height - 0.5 * box_height;
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
        assert_eq!(
            world.hover_height.value(),
            previous,
            "contact cannot change rigid-linkset calibration"
        );
    }

    #[test]
    fn invalid_loaded_mesh_keeps_the_previous_height_instead_of_applying_a_partial_estimate() {
        let lib = library();
        let estimate = Estimate {
            values: Vec::new(),
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
                        physics_mesh: None,
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
    fn persisted_auto_option_defaults_off_and_preserves_saved_choices() {
        assert!(!crate::settings::Settings::default().auto_hover_height);
        let settings: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.auto_hover_height);
        assert_eq!(settings.auto_hover_offset, 0.0);
        for enabled in [false, true] {
            let saved: crate::settings::Settings = serde_json::from_value(serde_json::json!({
                "auto_hover_height": enabled,
                "auto_hover_offset": -0.05,
            }))
            .unwrap();
            assert_eq!(saved.auto_hover_height, enabled);
            assert_eq!(saved.auto_hover_offset, -0.05);
            let reloaded: crate::settings::Settings = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
            assert_eq!(reloaded.auto_hover_height, enabled);
            assert_eq!(reloaded.auto_hover_offset, -0.05);
        }
    }
}
