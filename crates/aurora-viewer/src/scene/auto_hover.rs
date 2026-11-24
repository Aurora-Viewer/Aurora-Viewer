//! Avatar-only grounding in a neutral pose. This extends the Legacy Viewer's
//! manual LLFloaterHoverHeight / LLVOAvatarSelf::sendHoverHeight: animations
//! never drive the estimate, and only rendered vertices with foot weights count.
//! The reference is the shaped rest skeleton, independent of movement state,
//! streamed physical size, world contact and animated render palettes.
use super::meshes::MeshView;
use super::{GpuGeom, Scene, anim, avatar::AvatarLibrary};
use crate::world::{World, objects::Object};
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
/// The longest our own estimate waits for the login outfit restore to
/// start.
const OUTFIT_RESTORE_WAIT: Duration = Duration::from_secs(90);
/// The longest it waits for a started restore. Every stage of the
/// restore gives up by itself and ends it (about three minutes at worst,
/// with the Current Outfit fetched three times): this only guards one that
/// got stuck.
const OUTFIT_RESTORE_RUN: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Measurement {
    height: f32,
    /// No sole to measure yet: the last published height stands (and
    /// marks its copy, which turning automatic height off drops).
    kept: bool,
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
    /// The last measurement our hover height was set from, since the last
    /// login or automatic height last turned on: what a kept estimate
    /// leaves standing.
    published: Option<Measurement>,
    // Pose placement retains the last successful height while the current
    // outfit is recalibrated. Only value() may authorize hover publication.
    grounding: Option<(uuid::Uuid, f32)>,
    offset: Option<f32>,
    base: Option<Arc<anim::SkeletonBase>>,
    base_hash: u64,
    matrices: Vec<Mat4>,
    second_life: bool,
    checked_at: Option<Instant>,
    /// When our hover height began waiting for the login outfit restore,
    /// when that restore started, whether the wait is over for this login,
    /// and whether it holds now.
    outfit_wait: Option<Instant>,
    outfit_started: Option<Instant>,
    outfit_waited: bool,
    held: bool,
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
            published: None,
            grounding: None,
            offset: None,
            base: None,
            base_hash: 0,
            matrices: Vec::new(),
            second_life: true,
            checked_at: None,
            outfit_wait: None,
            outfit_started: None,
            outfit_waited: false,
            held: false,
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
    avatar: uuid::Uuid,
    signature: u64,
    ready: bool,
    estimate: Estimate,
    body_height: f32,
    shape_hover: f32,
    box_height: f32,
    root_height: f32,
}

impl AutoHover {
    /// A new login: its outfit restore is waited for again.
    fn new_login(&mut self) {
        // the last session's estimate and height are not this login's
        self.observed = None;
        self.finished = None;
        self.measurement = None;
        self.published = None;
        self.outfit_wait = None;
        self.outfit_started = None;
        self.outfit_waited = false;
        self.held = false;
    }

    /// Whether the login outfit restore holds the hover height now. Its
    /// attachments all worn release it, but the region can still take one
    /// off and the restore put it back: only the restore over (or one that
    /// cannot run) or the wait running out end the wait for this login:
    /// `OUTFIT_RESTORE_WAIT` for the restore to start, then
    /// `OUTFIT_RESTORE_RUN` from its start.
    fn outfit_hold(&mut self, world: &World, now: Instant) -> bool {
        if self.outfit_waited {
            return false;
        }
        let since = *self.outfit_wait.get_or_insert(now);
        if world.outfit.started() {
            self.outfit_started.get_or_insert(now);
        }
        let waited = now.saturating_duration_since(since);
        let timed_out = match self.outfit_started {
            Some(started) => now.saturating_duration_since(started) >= OUTFIT_RESTORE_RUN,
            None => waited >= OUTFIT_RESTORE_WAIT,
        };
        let restoring = world.outfit_restoring();
        if self.outfit_started.is_none() && !restoring && !timed_out {
            // Nothing to wait for yet (no inventory capability in this
            // region): a later seed answer can still bring it, and the
            // restore then holds the height again. A hold already in force
            // (the arrival not complete) stays: the outfit is still partial.
            return self.held;
        }
        if !restoring && !world.outfit_attachments_worn() || timed_out {
            self.outfit_waited = true;
            log::info!("auto hover: login outfit restore over after {:.1} s", waited.as_secs_f32());
            return false;
        }
        if restoring != self.held {
            log::info!(
                "auto hover: login outfit {} after {:.1} s",
                if restoring {
                    "restore holds the height"
                } else {
                    "attachments worn, measuring"
                },
                waited.as_secs_f32()
            );
        }
        restoring
    }

    fn value(&self) -> Option<f32> {
        (self.observed.is_some() && self.finished == self.observed)
            .then_some(self.measurement)
            .flatten()
            .map(|measurement| measurement.height)
    }

    fn grounding_value(&self, avatar: uuid::Uuid) -> Option<f32> {
        self.observed?;
        self.grounding.filter(|(owner, _)| *owner == avatar).map(|(_, height)| height)
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
            avatar,
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
                if result.is_some_and(|m| m.kept) {
                    // The height and its baseline for the offset controls
                    // stay the ones last published, not a partial outfit
                    // measured meanwhile (none yet: the saved height
                    // stands). Foot placement keeps the last measured.
                    self.measurement = self.published.map(|published| Measurement { kept: true, ..published });
                    completed = true;
                    continue;
                }
                self.measurement = result;
                if let Some(measurement) = result {
                    self.grounding = Some((avatar, measurement.height));
                }
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

/// `hidden`: one bit per texture-entry face the renderer never draws
/// (`hidden_faces`); those faces are not measured.
enum Surface {
    Mesh {
        mesh: Box<MeshView>,
        rigid: Option<Mat4>,
        hidden: u64,
        ankle: Option<f32>,
    },
    Geometry {
        geom: Arc<GpuGeom>,
        model: Mat4,
        hidden: u64,
        ankle: Option<f32>,
    },
}

struct Estimate {
    lib: Arc<AvatarLibrary>,
    base: Arc<anim::SkeletonBase>,
    ground: f32,
    surfaces: Vec<Surface>,
    values: Vec<u8>,
    /// The renderer hides the system lower body (under a mesh body that shows
    /// its bake, or an alpha layer): its feet are not what stands.
    system_feet_hidden: bool,
}

/// Ankle aliases (lFoot/rFoot), foot, toe and fitted-mesh collision volumes
/// all describe feet. Do not include knees or vertices whose foot weight is zero.
fn foot_joint(rig: &anim::Rig, joint: usize) -> bool {
    foot_side(rig, joint).is_some()
}

/// The foot joints of each limb, in `KNEES` order: the legs, then the
/// hind limbs of a creature rig, each with a knee of its own.
const FEET: [&[&str]; 4] = [
    &["mAnkleLeft", "mFootLeft", "mToeLeft", "L_FOOT"],
    &["mAnkleRight", "mFootRight", "mToeRight", "R_FOOT"],
    &["mHindLimb3Left", "mHindLimb4Left"],
    &["mHindLimb3Right", "mHindLimb4Right"],
];

/// The knee each limb's foot is held to; a limb and its pair (`i ^ 1`)
/// stand in for each other when one is missing.
const KNEES: [&str; 4] = ["mKneeLeft", "mKneeRight", "mHindLimb2Left", "mHindLimb2Right"];

/// The limb a foot joint belongs to, as `FEET` and `KNEES`.
fn foot_side(rig: &anim::Rig, joint: usize) -> Option<usize> {
    FEET.iter()
        .position(|limb| limb.iter().any(|name| rig.names.get(*name) == Some(&joint)))
}

/// The limb (as `FEET` and `KNEES`) a joint at or below a knee belongs to.
fn leg_limb(rig: &anim::Rig, mut joint: usize) -> Option<usize> {
    for _ in 0..rig.len() {
        if let Some(limb) = foot_side(rig, joint).or_else(|| KNEES.iter().position(|name| rig.names.get(*name) == Some(&joint))) {
            return Some(limb);
        }
        joint = rig.parent.get(joint).copied().flatten()?;
    }
    None
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

/// The faces the renderer never draws: a zero texture-entry alpha
/// (`te_invisible`, LL's PASS_ALPHA_INVISIBLE) or the transparent texture
/// without a glTF material (`classify`'s `Pass::Hidden`). Mesh bodies and
/// boots carry every foot variant (flat, high, paws…) and hide the unused
/// ones from their HUDs this way: a hidden variant is not a sole, and
/// switching variants must recalibrate (#3962).
fn hidden_faces(object: &Object) -> u64 {
    (0..aurora_prim::te::MAX_TES.min(64))
        .filter(|&face| {
            super::te_invisible(object, face)
                || (object.visual_texture_face(face).texture == aurora_prim::te::TRANSPARENT_TEXTURE
                    && object.visual_material(face).is_none())
        })
        .fold(0, |mask, face| mask | 1 << face)
}

fn face_shown(hidden: u64, face: usize) -> bool {
    face >= 64 || hidden & (1 << face) == 0
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
            self.auto_hover.grounding_value(owner).unwrap_or(0.0)
        } else {
            self.remote_soles
                .entries
                .get(&owner)
                .and_then(|state| state.grounding_value(owner))?
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
        if self.auto_hover.held {
            return None;
        }
        self.auto_hover.value()
    }

    /// A logout: the next login's outfit restore is waited for again.
    pub fn forget_login(&mut self) {
        self.auto_hover.new_login();
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
        if !enabled {
            // a kept estimate's copy of it goes too: no sole was measured
            self.auto_hover.held = false;
            self.auto_hover.published = None;
            if self.auto_hover.measurement.is_some_and(|m| m.kept) {
                self.auto_hover.measurement = None;
            }
        }
        if !enabled && !self.ground_avatar_feet {
            self.auto_hover.observed = None;
            self.auto_hover.finished = None;
            self.auto_hover.grounding = None;
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
        // At login the outfit arrives piece by piece for many seconds: each
        // piece gave a height of its own, the avatar sank and rose with
        // them and each was saved. The soles are still measured (foot
        // placement stands on them), but the hover height keeps its saved
        // value until the login restore has brought the outfit back, then
        // takes the estimate of the whole outfit.
        let was_held = self.auto_hover.held;
        self.auto_hover.held = enabled && self.auto_hover.outfit_hold(world, now);
        let released = was_held && !self.auto_hover.held;
        let ready = snapshot.ready;
        let completed = self.auto_hover.update(snapshot, &self.jobs, now);
        if self.auto_hover.held {
            return;
        }
        let offset = crate::world::hover_height::bounded(offset).unwrap_or(0.0);
        let offset_changed = self.auto_hover.offset != Some(offset);
        self.auto_hover.offset = enabled.then_some(offset);
        if enabled
            && ready
            && (completed || offset_changed || released)
            && let Some(height) = self.auto_hover.value()
        {
            world.hover_height.preview(height + offset);
            self.auto_hover.published = self.auto_hover.measurement;
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
            // the ankle of the limb a rigid piece is worn on, which a sole
            // reaches
            let ankle = ap
                .and_then(|a| a.joint)
                .and_then(|j| leg_limb(rig, j))
                .and_then(|limb| rig.names.get(FEET[limb][0]))
                .and_then(|&joint| matrices.get(joint))
                .map(|ankle| ankle.w_axis.z);
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
                // Calibration uses the reported pose, so use its full size
                // too, without following temporary render damping.
                let scale = object.local_preview_scale(object.scale);
                bits(&scale.to_array(), &mut hash);
                let hidden = hidden_faces(object);
                hidden.hash(&mut hash);
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
                                        rigid: (!own_rig).then(|| model * Mat4::from_scale(scale)),
                                        hidden,
                                        ankle: ankle.filter(|_| !own_rig),
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
                            model: model * Mat4::from_scale(scale),
                            hidden,
                            ankle,
                        }),
                        None => ready = false,
                    }
                }
                for &child in world.objects.children_of(&object.key) {
                    stack.push((child, model, depth + 1));
                }
            }
        }
        let system_feet_hidden = world.objects.index_of_uuid(&avatar).is_some_and(|idx| {
            self.avatar_lib.parts.iter().find(|part| part.name == "lower").is_some_and(|part| {
                self.gpu
                    .get(idx)
                    .is_some_and(|g| g.is_avatar && g.bom_mask & (1u64 << part.bake_te.min(63)) != 0)
                    || world.worn_bake_texture(idx, part.bake_te) == Some(super::avatar::IMG_INVISIBLE)
            })
        });
        system_feet_hidden.hash(&mut hash);
        Some(SoleSnapshot {
            avatar,
            signature: hash.finish(),
            ready,
            estimate: Estimate {
                lib: self.avatar_lib.clone(),
                base: skeleton.base.clone(),
                ground,
                surfaces,
                values: world.visual_params(&avatar).unwrap_or(&[]).to_vec(),
                system_feet_hidden,
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

#[cfg(test)]
fn mesh_min(
    faces: &[MeshFace],
    hidden: u64,
    skin: Option<&SkinInfo>,
    rigid: Option<Mat4>,
    rig: &anim::Rig,
    matrices: &[Mat4],
) -> Option<f32> {
    mesh_limb_min(faces, hidden, skin, rigid, rig, matrices)
        .into_iter()
        .flatten()
        .reduce(f32::min)
}

/// The lowest point per limb (`KNEES` order) of the vertices each foot
/// influences; a vertex weighted to several feet counts for each. A rigid
/// surface has no limbs: its lowest point is given for all.
fn mesh_limb_min(
    faces: &[MeshFace],
    hidden: u64,
    skin: Option<&SkinInfo>,
    rigid: Option<Mat4>,
    rig: &anim::Rig,
    matrices: &[Mat4],
) -> [Option<f32>; 4] {
    let mut min = None;
    let shown = || {
        faces
            .iter()
            .enumerate()
            .filter(|(i, _)| face_shown(hidden, *i))
            .map(|(_, face)| face)
    };
    if let Some(model) = rigid {
        for face in shown() {
            for &index in &face.indices {
                if let Some(p) = face.positions.get(index as usize) {
                    lower(&mut min, model.transform_point3(Vec3::from_array(*p)));
                }
            }
        }
        return [min; 4];
    }
    let mut limbs = [None; 4];
    let Some(skin) = skin else { return limbs };
    let sides: Vec<Option<usize>> = skin
        .joint_names
        .iter()
        .map(|name| rig.names.get(name).and_then(|&j| foot_side(rig, j)))
        .collect();
    let feet: Vec<bool> = sides.iter().map(Option::is_some).collect();
    if !feet.iter().any(|&foot| foot) {
        return limbs;
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
    for face in shown() {
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
                vertex.weights[k] = aurora_render::pack_skin_weight(face.weights[i][k]);
            }
            if let Some(point) = skinned_point(
                skin.bind_shape.transform_point3(Vec3::from_array(*position)),
                &vertex,
                &transforms,
                &feet,
            ) {
                for (&joint, weight) in vertex.joints.iter().zip(vertex.weights) {
                    if weight > 0
                        && let Some(Some(side)) = sides.get(joint as usize)
                    {
                        lower(&mut limbs[*side], point);
                    }
                }
            }
        }
    }
    limbs
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
        // No sole stands above both knees. While an outfit arrives, a rigged
        // piece that carries a little foot weight high up (an upper body, a
        // top) can be the first to load, before the legs and feet: its lowest
        // foot-weighted vertex sat at hip height, read as a hover of -0.75 m
        // and sank the avatar to the waist until the feet arrived. Such a
        // piece is no sole; the others, or the system feet, measure instead.
        // Each foot is held to its own knee (a creature's hind feet to the
        // hind knees): mesh joint offsets can set the knees at different
        // heights.
        let knee = KNEES.map(|name| rig.names.get(name).and_then(|&joint| matrices.get(joint)).map(|knee| knee.w_axis.z));
        let knee: [Option<f32>; 4] = std::array::from_fn(|i| knee[i].or(knee[i ^ 1]));
        let system_feet_hidden = self.system_feet_hidden;
        let (min, rigged_feet, skipped) = self
            .surfaces
            .into_par_iter()
            .map(|surface| -> Result<(Option<f32>, bool, bool), ()> {
                let mut min = None;
                let mut limbs = [None; 4];
                let rigged = matches!(&surface, Surface::Mesh { rigid: None, .. });
                let ankle = match &surface {
                    Surface::Mesh { ankle, .. } | Surface::Geometry { ankle, .. } => *ankle,
                };
                match surface {
                    Surface::Geometry { geom, model, hidden, .. } => {
                        let shown = geom.pick_faces.iter().enumerate().filter(|(i, _)| face_shown(hidden, *i));
                        for face in shown.filter_map(|(_, face)| face.as_ref()) {
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
                    Surface::Mesh { mesh, rigid, hidden, .. } => {
                        // the finest level held (the high one, as LLVOAvatar reads)
                        let section = mesh.lod_section(3).ok_or(())?;
                        let faces = aurora_assets::decode_mesh_lod(section).map_err(|_| ())?;
                        limbs = mesh_limb_min(&faces, hidden, mesh.skin.as_deref(), rigid, rig, &matrices);
                        for z in limbs.into_iter().flatten() {
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
                if rigged {
                    let kept = limbs
                        .iter()
                        .zip(knee)
                        .filter_map(|(&lowest, knee)| lowest.filter(|&z| knee.is_none_or(|knee| z <= knee)))
                        .reduce(f32::min);
                    if min.is_some() && kept.is_none() {
                        log::info!("auto hover rigged surface skipped: lowest {limbs:.3?} m is above the knees ({knee:.3?} m)");
                        return Ok((None, false, true));
                    }
                    min = kept;
                } else if system_feet_hidden
                    && let (Some(lowest), Some(ankle)) = (min, ankle)
                    && lowest > ankle
                {
                    // Over a hidden system body a rigid piece worn on a
                    // leg stands alone (a shoe): only if it reaches its
                    // ankle. A knee pad or shin guard is no sole and is left
                    // out, not skipped: the system feet measure, as before.
                    log::info!("auto hover rigid surface left out: lowest {lowest:.3} m is above its ankle ({ankle:.3} m)");
                    return Ok((None, false, false));
                }
                Ok((min, rigged && min.is_some(), false))
            })
            .try_reduce(
                || (None, false, false),
                |left, right| {
                    let min = match (left.0, right.0) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    Ok((min, left.1 || right.1, left.2 || right.2))
                },
            )
            .ok()?;
        // Over a hidden system body the system feet are not what stands: a
        // rigid sole does, whatever else was skipped. With none and pieces
        // above the knees skipped, the feet have not arrived yet: keep the
        // last accepted height, the feet arriving change the signature.
        let hidden_feet = !rigged_feet && self.system_feet_hidden;
        if hidden_feet && skipped && min.is_none() {
            log::info!("auto hover estimate kept the previous height: no sole yet under a hidden system body, {surfaces} surfaces");
            return Some(Measurement {
                height: f32::NAN,
                kept: true,
                #[cfg(test)]
                lowest: None,
            });
        }
        let min = if rigged_feet || hidden_feet && min.is_some() {
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
            kept: false,
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
        assert!((mesh_min(&[face()], 0, Some(&skin()), None, &lib.rig, &matrices).unwrap() + 0.3).abs() < 1e-5);
        let mut unrelated = face();
        unrelated.weights.fill([0.0, 1.0, 0.0, 0.0]);
        assert_eq!(mesh_min(&[unrelated], 0, Some(&skin()), None, &lib.rig, &matrices), None);
    }

    #[test]
    fn tiny_weights_rounded_to_zero_do_not_select_low_vertices() {
        let lib = library();
        let mut geometry = face();
        geometry.weights[1] = [0.000001, 0.999999, 0.0, 0.0];
        let matrices = vec![Mat4::IDENTITY; lib.rig.len()];
        assert!((mesh_min(&[geometry], 0, Some(&skin()), None, &lib.rig, &matrices).unwrap() + 0.3).abs() < 1e-5);
    }

    #[test]
    fn selected_vertices_blend_every_joint_and_normalize_packed_weights() {
        let vertex = SkinVertex {
            joints: [0, 1, 0, 0],
            weights: [16384, 49151, 0, 0],
        };
        let matrices = [Mat4::from_translation(-Vec3::Z), Mat4::from_translation(Vec3::Z)];
        let point = skinned_point(Vec3::ZERO, &vertex, &matrices, &[true, false]).unwrap();
        assert!((point.z - 32767.0 / 65535.0).abs() < 1e-6);
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
        assert!(mesh_min(&[face()], 0, Some(&skin), None, &lib.rig, &matrices).unwrap().abs() < 1e-5);
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
        let actual = mesh_min(&[geometry], 0, Some(&skin), None, &lib.rig, &matrices).unwrap();
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
        let actual = mesh_min(&[geometry], 0, Some(&skin), None, &lib.rig, &matrices);
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
        assert!((mesh_min(&[geometry], 0, None, Some(model), &lib.rig, &[]).unwrap() + 0.2).abs() < 1e-5);
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

    fn logged_in(world: &mut World) {
        world.login = Some(Arc::new(aurora_net::LoginResponse {
            agent_id: world.agent_id,
            session_id: uuid::Uuid::nil(),
            secure_session_id: uuid::Uuid::nil(),
            first_name: "Nova".into(),
            last_name: "Resident".into(),
            sim_ip: std::net::Ipv4Addr::LOCALHOST,
            sim_port: 0,
            circuit_code: 0,
            region_x: 0,
            region_y: 0,
            region_size_x: 256,
            region_size_y: 256,
            seed_capability: String::new(),
            look_at: Vec3::X,
            message: String::new(),
            agent_appearance_service: String::new(),
            inventory_root: uuid::Uuid::nil(),
            mfa_hash: None,
            raw: aurora_llsd::Llsd::Undef,
        }));
    }

    /// The scene finds the hidden system lower body itself (here an
    /// invisible lower bake, as an alpha layer gives) and a change of it
    /// changes the signature, so the estimate is run again.
    #[test]
    fn an_invisible_lower_bake_hides_the_system_feet_and_rechecks() {
        let (mut world, scene, _) = fixture();
        let agent = world.agent_id;
        let snapshot = |world: &World| {
            let snapshot = scene.sole_snapshot(world, agent, &scene.auto_hover).unwrap();
            (snapshot.estimate.system_feet_hidden, snapshot.signature)
        };
        let (hidden, shown) = snapshot(&world);
        assert!(!hidden, "the system feet of a plain avatar are shown");
        let lower = world.avatar_lib.parts.iter().find(|part| part.name == "lower").unwrap().bake_te;
        world.show_local_bakes([(lower, super::super::avatar::IMG_INVISIBLE)].into());
        let (hidden, signature) = snapshot(&world);
        assert!(hidden, "an invisible lower bake hides the system feet");
        assert_ne!(signature, shown, "hiding the system feet must run the estimate again");
        // The wearable editor's preview is not worn: an alpha layer gives
        // it a new texture, invisible or not, until it is saved.
        world.preview_bakes([(lower, uuid::Uuid::from_u128(77))].into());
        assert_eq!(snapshot(&world), (true, signature), "a wearable preview changed the measured feet");
        world.preview_bakes(Default::default());
        world.show_local_bakes(Default::default());
        assert_eq!(snapshot(&world), (false, shown));
        // RLVa's @showself only stops drawing our avatar (the Legacy
        // Viewer's LLAgent::needsRenderAvatar): what stands is unchanged.
        world.rlva.configure(true);
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "RLVa test object".to_owned(),
            source_id: uuid::Uuid::from_u128(4),
            owner_id: agent,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            region: None,
            position: Vec3::ZERO,
            message: "@showself=n".to_owned(),
        }));
        assert!(world.rlva.has("showself"));
        assert_eq!(snapshot(&world), (false, shown), "@showself changed the measured feet");
    }

    #[test]
    fn the_login_outfit_restore_holds_the_saved_height_then_measures_once() {
        use crate::world::outfit::OutfitRestore;
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        scene.ground_avatar_feet = true;
        assert!(world.outfit_restoring());
        let saved = world.hover_height.value();
        let piece = uuid::Uuid::from_u128(200);
        let mut worn_piece = crate::demo::auto_hover::rigged_object();
        worn_piece.name_values = format!("AttachItemID STRING RW SV {piece}");
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![worn_piece],
        });
        // While the restore runs the soles are measured for foot placement,
        // but the hover height keeps its saved value and the offset
        // controls get no baseline.
        settle(&mut scene, &mut world, now);
        assert!(scene.auto_hover.held);
        assert!(scene.auto_hover.grounding.is_some(), "foot placement waited for the login restore");
        assert_eq!(world.hover_height.value(), saved, "the saved height moved during the login restore");
        assert_eq!(scene.automatic_hover_base(&world), None, "the offset controls saw a partial outfit");
        scene.update_auto_hover(&mut world, true, 0.1, true, now + SETTLE);
        assert_eq!(
            world.hover_height.value(),
            saved,
            "an offset change published during the login restore"
        );
        // Every attachment worn: the estimate already made applies at once.
        world.outfit = OutfitRestore::attaching(&[piece], &[]);
        assert!(!world.outfit_restoring());
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE);
        assert!(!scene.auto_hover.held);
        let whole = world.hover_height.value();
        assert!(whole > 0.1 && whole < 0.3, "{whole}");
        // The region takes a piece off again: the hold is back until the
        // restore puts it on, before the restore's next poll notices.
        world.outfit = OutfitRestore::attaching(&[piece], &[piece]);
        world.apply(NetEvent::ObjectsKilled {
            handle: 1,
            local_ids: vec![crate::demo::auto_hover::LEFT],
        });
        settle(&mut scene, &mut world, now + SETTLE * 2);
        assert!(scene.auto_hover.held, "a piece taken off after the release was measured");
        assert_eq!(world.hover_height.value(), whole);
        // The restore over: the wait ends for this login.
        world.outfit = OutfitRestore::finished();
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 3);
        assert!(!scene.auto_hover.held && scene.auto_hover.outfit_waited);
        assert_eq!(world.hover_height.value(), scene.automatic_hover_base(&world).unwrap());
        assert_ne!(world.hover_height.value(), whole);
        // A logout: the next login waits for its restore again.
        scene.forget_login();
        world.outfit = OutfitRestore::default();
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 4);
        assert!(scene.auto_hover.held, "the second login was not held");
        assert_eq!(scene.automatic_hover_base(&world), None);

        // A restore that never ends holds for OUTFIT_RESTORE_WAIT at most.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        let saved = world.hover_height.value();
        settle(&mut scene, &mut world, now + OUTFIT_RESTORE_WAIT - SETTLE * 2);
        assert!(scene.auto_hover.held, "released before OUTFIT_RESTORE_WAIT");
        assert_eq!(world.hover_height.value(), saved);
        settle(&mut scene, &mut world, now + OUTFIT_RESTORE_WAIT);
        assert!(world.outfit_restoring());
        assert!(world.hover_height.value() > 0.1 && world.hover_height.value() < 0.3);

        // A started restore can run longer (one retry of a missing piece):
        // it is held for OUTFIT_RESTORE_RUN from its start.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        let saved = world.hover_height.value();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        world.outfit = crate::world::outfit::OutfitRestore::attaching(&[uuid::Uuid::from_u128(200)], &[]);
        rest(&mut scene, &world, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        settle(&mut scene, &mut world, now + OUTFIT_RESTORE_WAIT);
        assert!(scene.auto_hover.held, "a started restore was cut at OUTFIT_RESTORE_WAIT");
        assert_eq!(world.hover_height.value(), saved);
        assert!(OUTFIT_RESTORE_RUN > crate::world::outfit::SLOWEST_RESTORE_TESTED);
        settle(&mut scene, &mut world, now + OUTFIT_RESTORE_RUN - SETTLE * 2);
        assert!(scene.auto_hover.held, "released before OUTFIT_RESTORE_RUN");
        settle(&mut scene, &mut world, now + OUTFIT_RESTORE_RUN);
        assert!(!scene.auto_hover.held && scene.auto_hover.outfit_waited);
        assert!(world.hover_height.value() > 0.1 && world.hover_height.value() < 0.3);

        // With the automatic height off, foot placement is not held: it
        // needs the soles now, the hover height is not touched.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        scene.ground_avatar_feet = true;
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        rest(&mut scene, &world, now);
        let saved = world.hover_height.value();
        scene.update_auto_hover(&mut world, false, 0.0, true, now);
        for _ in 0..1000 {
            scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
            if scene.auto_hover.grounding.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(world.outfit_restoring());
        assert!(scene.auto_hover.grounding.is_some(), "foot placement waited for the login restore");
        assert_eq!(world.hover_height.value(), saved);
    }

    #[test]
    fn a_kept_estimate_leaves_the_last_height_and_its_baseline() {
        let (mut world, mut scene, now) = fixture();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let measured = scene.auto_hover.value().unwrap();
        assert_eq!(scene.automatic_hover_base(&world), Some(measured));
        // The outfit changes and its estimate finds no sole yet (feet not
        // arrived under a hidden body): the height and the offset controls'
        // baseline stay the last ones.
        let shoe = world
            .objects
            .index_of_uuid(&crate::demo::u(crate::demo::auto_hover::LEFT.into()))
            .unwrap();
        world.objects.get_mut(shoe).unwrap().position.x += 0.01;
        let at = now + Duration::from_secs(1);
        scene.update_auto_hover(&mut world, true, 0.0, true, at);
        let kept = Measurement {
            height: f32::NAN,
            kept: true,
            lowest: None,
        };
        scene.auto_hover.tx.send((scene.auto_hover.observed.unwrap(), Some(kept))).unwrap();
        scene.update_auto_hover(&mut world, true, 0.0, true, at);
        assert_eq!(scene.auto_hover.value(), Some(measured), "a kept estimate erased the height");
        assert_eq!(
            scene.automatic_hover_base(&world),
            Some(measured),
            "the offset controls lost their baseline"
        );
        assert_eq!(world.hover_height.value(), measured);
    }

    /// The outfit changes and its estimate finds no sole (feet not
    /// arrived under a hidden body): a kept result for the new signature.
    fn keep_estimate(scene: &mut Scene, world: &mut World, at: Instant) {
        let shoe = world
            .objects
            .index_of_uuid(&crate::demo::u(crate::demo::auto_hover::LEFT.into()))
            .unwrap();
        world.objects.get_mut(shoe).unwrap().position.x += 0.01;
        let before = scene.auto_hover.observed;
        scene.update_auto_hover(world, true, 0.0, true, at);
        assert_ne!(scene.auto_hover.observed, before, "the outfit change was not seen");
        assert_eq!(scene.auto_hover.finished, None);
        let kept = Measurement {
            height: f32::NAN,
            kept: true,
            lowest: None,
        };
        scene.auto_hover.tx.send((scene.auto_hover.observed.unwrap(), Some(kept))).unwrap();
        scene.update_auto_hover(world, true, 0.0, true, at);
        assert_eq!(scene.auto_hover.finished, scene.auto_hover.observed);
    }

    /// A kept estimate leaves standing only a height already published:
    /// not a partial outfit measured during the login hold, nor the last
    /// session's or the last time automatic height was on.
    #[test]
    fn a_kept_estimate_publishes_only_a_height_already_published() {
        use crate::world::outfit::OutfitRestore;
        let rigged = |world: &mut World, scene: &mut Scene| {
            crate::demo::auto_hover::install_mesh(&world.avatar_lib, scene).unwrap();
            world.apply(NetEvent::ObjectUpdates {
                handle: 1,
                objects: vec![crate::demo::auto_hover::rigged_object()],
            });
        };
        // Measured during the login hold, then the restored outfit has no
        // sole: the release leaves the saved height.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        let saved = world.hover_height.value();
        rigged(&mut world, &mut scene);
        settle(&mut scene, &mut world, now);
        assert!(scene.auto_hover.held && scene.auto_hover.measurement.is_some());
        keep_estimate(&mut scene, &mut world, now + SETTLE);
        world.outfit = OutfitRestore::finished();
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 2);
        assert!(!scene.auto_hover.held);
        assert_eq!(world.hover_height.value(), saved, "the hold's partial outfit was published");
        assert_eq!(scene.automatic_hover_base(&world), None);

        // A logout, then a login whose first estimate is kept: the last
        // session's height is not published.
        let (mut world, mut scene, now) = fixture();
        rigged(&mut world, &mut scene);
        settle(&mut scene, &mut world, now);
        let first = world.hover_height.value();
        assert!(scene.auto_hover.value().is_some());
        scene.forget_login();
        assert_eq!(scene.auto_hover.value(), None, "the last session's estimate stood");
        assert_eq!(scene.auto_hover.observed, None);
        world.hover_height.preview(first + 0.2);
        keep_estimate(&mut scene, &mut world, now + SETTLE);
        assert_eq!(scene.auto_hover.value(), None, "the last session's height stood");
        assert_eq!(world.hover_height.value(), first + 0.2);

        // Automatic height off (foot placement off too), the height set by
        // hand, then on again with a kept estimate: the hand's height stays.
        let (mut world, mut scene, now) = fixture();
        rigged(&mut world, &mut scene);
        settle(&mut scene, &mut world, now);
        let first = world.hover_height.value();
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
        world.hover_height.preview(first + 0.2);
        keep_estimate(&mut scene, &mut world, now + SETTLE * 2);
        assert_eq!(world.hover_height.value(), first + 0.2, "an old automatic height came back");

        // The same with foot placement on: the soles were measured all
        // along, but nothing was published while it was off.
        let (mut world, mut scene, now) = fixture();
        scene.ground_avatar_feet = true;
        rigged(&mut world, &mut scene);
        settle(&mut scene, &mut world, now);
        let first = world.hover_height.value();
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
        world.hover_height.preview(first + 0.2);
        keep_estimate(&mut scene, &mut world, now + SETTLE * 2);
        assert_eq!(world.hover_height.value(), first + 0.2, "an old automatic height came back");
        assert!(scene.auto_hover.grounding.is_some(), "foot placement lost its sole");

        // A kept estimate first, then automatic height off and the height
        // set by hand, then on again with the outfit unchanged: the kept
        // copy of the old height is not published.
        let (mut world, mut scene, now) = fixture();
        scene.ground_avatar_feet = true;
        rigged(&mut world, &mut scene);
        settle(&mut scene, &mut world, now);
        let first = world.hover_height.value();
        keep_estimate(&mut scene, &mut world, now + SETTLE);
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE * 2);
        world.hover_height.preview(first + 0.2);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 3);
        assert_eq!(world.hover_height.value(), first + 0.2, "a kept copy of the old height came back");
    }

    /// A region without the inventory capability yet (a neighbour's first
    /// seed answer): the restore cannot start and nothing is held, but the
    /// capability arriving later starts it and holds the height again.
    #[test]
    fn the_hold_comes_back_when_the_inventory_capability_arrives() {
        // Held while the arrival is not complete, then complete in a region
        // without the capability: the outfit is still partial, the hold
        // stays (not latched) until the capability starts the restore.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        let saved = world.hover_height.value();
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        assert!(scene.auto_hover.held);
        world.movement_complete = true;
        assert!(!world.outfit_restoring());
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 2);
        assert!(scene.auto_hover.held, "the arrival released the hold of a partial outfit");
        assert!(!scene.auto_hover.outfit_waited);
        assert_eq!(world.hover_height.value(), saved);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + OUTFIT_RESTORE_WAIT);
        assert!(!scene.auto_hover.held && scene.auto_hover.outfit_waited);

        // Held, then automatic height (and foot placement) off: on again in
        // a region without the capability, nothing is held.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        assert!(scene.auto_hover.held);
        scene.update_auto_hover(&mut world, false, 0.0, true, now + SETTLE);
        world.movement_complete = true;
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 2);
        assert!(!scene.auto_hover.held, "a hold from before automatic height was off came back");

        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        world.movement_complete = true;
        assert!(!world.outfit_restoring());
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        assert!(!scene.auto_hover.held);
        assert!(
            !scene.auto_hover.outfit_waited,
            "a missing capability ended the wait for this login"
        );
        let measured = world.hover_height.value();
        let region = world.regions.get_mut(&1).unwrap();
        Arc::make_mut(&mut region.caps).insert("FetchInventoryDescendents2".into(), String::new());
        assert!(world.outfit_restoring());
        scene.update_auto_hover(&mut world, true, 0.0, true, now + SETTLE * 2);
        assert!(scene.auto_hover.held, "the late restore was not held");
        assert_eq!(world.hover_height.value(), measured);
        // Never started: the wait still ends at OUTFIT_RESTORE_WAIT.
        let (mut world, mut scene, now) = fixture();
        logged_in(&mut world);
        world.movement_complete = true;
        scene.update_auto_hover(&mut world, true, 0.0, true, now);
        scene.update_auto_hover(&mut world, true, 0.0, true, now + OUTFIT_RESTORE_WAIT - SETTLE);
        assert!(!scene.auto_hover.outfit_waited, "a missing capability ended the wait");
        scene.update_auto_hover(&mut world, true, 0.0, true, now + OUTFIT_RESTORE_WAIT);
        assert!(scene.auto_hover.outfit_waited);
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
            system_feet_hidden: false,
            surfaces: vec![Surface::Mesh {
                mesh: Box::new(mesh.unwrap()),
                rigid: None,
                hidden: 0,
                ankle: None,
            }],
        };
        assert!((estimate.calculate().unwrap().height - 0.18).abs() < 0.001);
    }

    /// Every foot joint hangs below the knee its limb is held to, on the
    /// avatar skeleton: a hind foot under a hind knee, not a leg's.
    #[test]
    fn each_foot_is_held_to_the_knee_above_it() {
        let rig = &library().rig;
        for (limb, feet) in FEET.iter().enumerate() {
            let knee = rig.names[KNEES[limb]];
            for name in *feet {
                let Some(&foot) = rig.names.get(*name) else {
                    assert!(name.ends_with("_FOOT"), "{name} is not in the skeleton");
                    continue;
                };
                assert_eq!(foot_side(rig, foot), Some(limb), "{name}");
                let mut joint = foot;
                while joint != knee {
                    joint = rig.parent[joint].unwrap_or_else(|| panic!("{name} is not below {}", KNEES[limb]));
                }
            }
        }
    }

    /// A rigid piece is held to the ankle of the limb it is worn on.
    #[test]
    fn a_leg_joint_belongs_to_its_own_limb() {
        let rig = &library().rig;
        let limb = |name: &str| leg_limb(rig, rig.names[name]);
        assert_eq!(limb("mKneeLeft"), Some(0));
        assert_eq!(limb("mKneeRight"), Some(1));
        assert_eq!(limb("mFootRight"), Some(1));
        assert_eq!(limb("mToeLeft"), Some(0));
        assert_eq!(limb("mHindLimb4Right"), Some(3));
        assert_eq!(limb("mHindLimb2Left"), Some(2));
        assert_eq!(limb("mPelvis"), None);
        assert_eq!(limb("mHipRight"), None);
    }

    #[test]
    fn rigged_foot_weights_above_the_knees_are_no_sole_and_leave_the_system_feet() {
        let lib = library();
        let mut scene = Scene::new(std::path::PathBuf::new(), lib.clone());
        let rig = &lib.rig;
        let left_ankle = rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale)[rig.names["mAnkleLeft"]]
            .w_axis
            .z;
        let mut estimate = |raise: Option<f32>, system_feet_hidden: bool, left_knee_lift: f32, rigid_shoe: Option<f32>| {
            let mut surfaces = match raise {
                None => Vec::new(),
                Some(raise) => {
                    // The right shoe raised to hip height and the left one
                    // hidden: the only foot-weighted geometry is a piece that
                    // arrived before the legs and feet, on the neutral rig.
                    crate::demo::auto_hover::install_variant_mesh(&lib, &mut scene, -raise).unwrap();
                    vec![Surface::Mesh {
                        mesh: Box::new(scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap()),
                        rigid: None,
                        hidden: 1,
                        ankle: None,
                    }]
                }
            };
            if let Some(lift) = rigid_shoe {
                // the same piece, worn rigid (a shoe that is not rigged)
                // and lifted by `lift`
                surfaces.push(Surface::Mesh {
                    mesh: Box::new(scene.meshes.view(&crate::demo::auto_hover::MESH).unwrap()),
                    rigid: Some(Mat4::from_translation(Vec3::Z * lift)),
                    hidden: 0,
                    ankle: Some(left_ankle),
                });
            }
            Estimate {
                lib: lib.clone(),
                base: Arc::new(anim::SkeletonBase {
                    local_pos: {
                        let mut local_pos = lib.rig.local_pos.clone();
                        local_pos[lib.rig.names["mKneeLeft"]].z += left_knee_lift;
                        local_pos
                    },
                    scale: lib.rig.scale.clone(),
                }),
                ground: 0.0,
                values: Vec::new(),
                system_feet_hidden,
                surfaces,
            }
            .calculate()
        };
        let barefoot = estimate(None, false, 0.0, None).unwrap();
        let shin = estimate(Some(0.2), false, 0.0, None).unwrap();
        let hip = estimate(Some(0.9), false, 0.0, None).unwrap();
        assert!(shin.lowest.unwrap() > barefoot.lowest.unwrap(), "a shoe below the knees is a sole");
        assert_ne!(shin, barefoot);
        assert_eq!(hip, barefoot, "foot weights above the knees were measured as a sole");
        // Under a mesh body the system feet are hidden: nothing visible
        // stands yet, so the previous height is kept.
        assert!(
            estimate(Some(0.9), true, 0.0, None).is_some_and(|m| m.kept),
            "hidden system feet were measured"
        );
        // A rigid shoe beside the skipped piece is a visible sole: it is
        // measured alone, not kept out with the hidden feet.
        // Lifted to stand just above the system feet: measured with them,
        // the system feet would be the lowest.
        let unlifted = estimate(Some(0.9), true, 0.0, Some(0.0)).expect("a rigid sole under a hidden body was discarded");
        let lift = barefoot.lowest.unwrap() - unlifted.lowest.unwrap() + 0.05;
        let rigid = estimate(Some(0.9), true, 0.0, Some(lift)).unwrap().lowest.unwrap();
        assert!(
            (rigid - barefoot.lowest.unwrap() - 0.05).abs() < 1e-4,
            "the hidden system feet were measured: {rigid}"
        );
        assert_eq!(
            estimate(Some(0.9), false, 0.0, Some(lift)).unwrap().lowest,
            barefoot.lowest,
            "shown feet stand lower"
        );
        // Nothing skipped: the same rigid sole is measured alone too, so a
        // skipped piece coming or going does not move the height.
        assert_eq!(
            estimate(None, true, 0.0, Some(lift)).unwrap().lowest,
            Some(rigid),
            "the hidden system feet were measured with no piece skipped"
        );
        // Nothing visible and nothing skipped: the system feet still stand.
        assert_eq!(estimate(None, true, 0.0, None).unwrap(), barefoot);
        assert_eq!(
            estimate(Some(0.2), true, 0.0, None).unwrap(),
            shin,
            "a visible sole under a hidden body is still measured"
        );
        // A rigid piece on a leg above its ankle (a shin guard, a knee
        // pad) is no sole over a hidden body: it is left out and the system
        // feet stand, as over shown feet. Nothing is kept for it: an outfit
        // that keeps it would never be measured.
        assert!(rigid < left_ankle, "the lifted shoe {rigid} must reach the ankle {left_ankle}");
        let guard = left_ankle - unlifted.lowest.unwrap() + 0.1;
        assert_eq!(
            estimate(None, true, 0.0, Some(guard)).unwrap(),
            barefoot,
            "a rigid piece above its ankle stood the avatar on it"
        );
        assert_eq!(estimate(None, false, 0.0, Some(guard)).unwrap(), barefoot);
        assert!(
            estimate(Some(0.9), true, 0.0, Some(guard)).is_some_and(|m| m.kept),
            "with a skipped piece and no sole, the feet have not arrived"
        );
        // Joint offsets raise the left knee well above the right one. The
        // right shoe, between the two knees, is above its own knee: no sole,
        // even though it is below the higher knee.
        let rig = &lib.rig;
        let mut local_pos = rig.local_pos.clone();
        local_pos[rig.names["mKneeLeft"]].z += 0.6;
        let lifted = rig.world_with(&rig.local_rot, Vec3::ZERO, &local_pos, &rig.scale);
        let [left_knee, right_knee, ..] = KNEES.map(|name| lifted[rig.names[name]].w_axis.z);
        assert!(left_knee > right_knee + 0.5, "knees {left_knee} {right_knee}");
        // The shoe is raised linearly: from the shin case, 0.8 m puts it
        // between the knees.
        let shoe = shin.lowest.unwrap() + 0.6;
        assert!(shoe > right_knee && shoe < left_knee, "shoe {shoe}, knees {left_knee} {right_knee}");
        assert_eq!(
            estimate(Some(0.8), false, 0.6, None).unwrap(),
            estimate(None, false, 0.6, None).unwrap(),
            "a foot above its own knee was measured against the other knee"
        );
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
        let lowest = mesh_min(&faces, 0, mesh.skin.as_deref(), None, &lib.rig, &neutral).unwrap();
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
        let lowest = mesh_min(&faces, 0, mesh.skin.as_deref(), None, &lib.rig, &matrices).unwrap();
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
                let lowest = mesh_min(&faces, 0, mesh.skin.as_deref(), None, &lib.rig, &matrices).unwrap();
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
        let lowest = mesh_min(&faces, 0, mesh.skin.as_deref(), None, &lib.rig, &scene.auto_hover.matrices).unwrap();
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
                    kept: false,
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
    fn own_foot_placement_keeps_its_last_sole_while_recalibrating() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        scene.ground_avatar_feet = true;
        crate::demo::auto_hover::install_mesh(&world.avatar_lib, &mut scene).unwrap();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let owner = world.agent_id;
        let previous_stamp = scene.auto_hover.finished.unwrap();
        let previous_sole = scene.grounding_sole_height(&world, owner).unwrap();
        let previous_height = scene.automatic_hover_base(&world).unwrap();
        assert_eq!(
            scene.auto_hover.grounding_value(uuid::Uuid::nil()),
            None,
            "another avatar cannot reuse the retained sole"
        );
        world.hover_height.preview(0.35);
        world.hover_height.commit();
        let shoe = world
            .objects
            .index_of_uuid(&crate::demo::u(crate::demo::auto_hover::LEFT.into()))
            .unwrap();
        // A rigged attachment's object transform is not its skinned sole.
        // Updating that transform invalidates the outfit without moving it.
        world.objects.get_mut(shoe).unwrap().position.x += 0.01;
        let at = now + Duration::from_secs(1);
        scene.update_auto_hover(&mut world, false, 0.0, true, at);
        assert_eq!(scene.automatic_hover_base(&world), None, "new outfit is not ready to publish");
        assert_eq!(
            scene.grounding_sole_height(&world, owner),
            Some(previous_sole),
            "recalibration discarded the visible shoe compensation"
        );
        scene
            .auto_hover
            .tx
            .send((
                previous_stamp,
                Some(Measurement {
                    height: 1.7,
                    kept: false,
                    lowest: Some(0.0),
                }),
            ))
            .unwrap();
        scene.update_auto_hover(&mut world, true, 0.1, true, at);
        assert_eq!(
            world.hover_height.value(),
            0.35,
            "an offset edit must not publish a retained or stale height"
        );
        assert_eq!(scene.grounding_sole_height(&world, owner), Some(previous_sole));
        // A failed replacement is not a new valid sole either.
        scene.auto_hover.tx.send((scene.auto_hover.observed.unwrap(), None)).unwrap();
        scene.update_auto_hover(&mut world, false, 0.0, true, at);
        assert_eq!(scene.automatic_hover_base(&world), None);
        assert_eq!(scene.grounding_sole_height(&world, owner), Some(previous_sole));
        world.objects.get_mut(shoe).unwrap().position.x += 0.01;
        let at = at + Duration::from_millis(50);
        scene.update_auto_hover(&mut world, false, 0.0, true, at);
        for _ in 0..2 {
            scene.update_auto_hover(&mut world, false, 0.0, true, at + SETTLE);
        }
        assert_eq!(scene.automatic_hover_base(&world), Some(previous_height));
        assert_eq!(scene.grounding_sole_height(&world, owner), Some(previous_sole));
        assert_eq!(world.hover_height.value(), 0.35, "foot placement must preserve manual hover");
        scene.ground_avatar_feet = false;
        scene.update_auto_hover(&mut world, false, 0.0, true, at + Duration::from_secs(1));
        assert_eq!(scene.automatic_hover_base(&world), None);
        assert_ne!(
            scene.grounding_sole_height(&world, owner),
            Some(previous_sole),
            "disabled calibration retained a usable height"
        );
    }

    #[test]
    fn remote_foot_placement_stays_active_while_its_sole_is_recalibrated() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        world.agent.flying = true;
        scene.ground_avatar_feet = true;
        let resident = remote_shoes(&mut scene, &mut world, now, 1, 0.06);
        let avatar = world.objects.index_of_uuid(&resident.avatar).unwrap();
        world.objects.get_mut(avatar).unwrap().velocity = Vec3::ZERO;
        for frame in 0..90 {
            remote_step(&mut scene, &mut world, now + Duration::from_millis(frame * 16));
        }
        let previous_sole = scene.grounding_sole_height(&world, resident.avatar).unwrap();
        let previous_height = scene.remote_soles.entries[&resident.avatar].value().unwrap();
        let shoe = world
            .objects
            .index_of(&crate::world::objects::ObjKey {
                region: 1,
                local_id: resident.shoe,
            })
            .unwrap();
        world.objects.get_mut(shoe).unwrap().position.x += 0.01;
        let at = now + Duration::from_secs(2);
        remote_step(&mut scene, &mut world, at);
        assert_eq!(scene.remote_soles.entries[&resident.avatar].value(), None);
        assert_eq!(scene.grounding_sole_height(&world, resident.avatar), Some(previous_sole));
        let avatar = world.objects.index_of_uuid(&resident.avatar).unwrap();
        assert!(
            scene.grounding_frame(&world, avatar, at).is_some(),
            "recalibration switched off remote foot placement"
        );
        for frame in 1..=45 {
            let at = at + Duration::from_millis(frame * 16);
            remote_step(&mut scene, &mut world, at);
            assert!(
                rendered_remote_sole(&scene, &world, &resident, at).abs() < 0.003,
                "recalibration moved the rendered sole at frame {frame}"
            );
        }
        assert_eq!(scene.remote_soles.entries[&resident.avatar].value(), Some(previous_height));
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

    fn install_variant_shoes(scene: &mut Scene, lib: &AvatarLibrary, drop: f32) {
        crate::demo::auto_hover::install_variant_mesh(lib, scene, drop).unwrap();
    }

    fn set_face(world: &mut World, local_id: u32, face: usize, edit: impl FnOnce(&mut aurora_prim::te::TextureFace)) {
        let index = world
            .objects
            .index_of(&crate::world::objects::ObjKey { region: 1, local_id })
            .unwrap();
        let object = world.objects.get_mut(index).unwrap();
        let mut te = object.te.as_deref().cloned().unwrap_or_default();
        edit(&mut te.faces[face]);
        object.te = Some(Arc::new(te));
    }

    #[test]
    fn hidden_foot_variants_are_not_soles_and_switching_them_recalibrates() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (mut world, mut scene, now) = fixture();
        scene.ground_avatar_feet = true;
        install_variant_shoes(&mut scene, &world.avatar_lib, 0.3);
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        settle(&mut scene, &mut world, now);
        let shown = scene.automatic_hover_base(&world).unwrap();
        let owner = world.agent_id;
        let shown_sole = scene.grounding_sole_height(&world, owner).unwrap();
        // The HUD hides the lower variant: a zero alpha, as the renderer omits.
        set_face(&mut world, crate::demo::auto_hover::LEFT, 1, |face| face.color[3] = 0.0);
        settle(&mut scene, &mut world, now + Duration::from_secs(1));
        let hidden = scene.automatic_hover_base(&world).unwrap();
        assert!(
            (shown - hidden - 0.3).abs() < 0.01,
            "a hidden variant still set the sole: {shown} -> {hidden}"
        );
        assert!((scene.grounding_sole_height(&world, owner).unwrap() - shown_sole - 0.3).abs() < 0.01);
        // The transparent texture hides it too, as the renderer's Pass::Hidden.
        set_face(&mut world, crate::demo::auto_hover::LEFT, 1, |face| {
            face.color[3] = 1.0;
            face.texture = aurora_prim::te::TRANSPARENT_TEXTURE;
        });
        settle(&mut scene, &mut world, now + Duration::from_secs(2));
        assert!((scene.automatic_hover_base(&world).unwrap() - hidden).abs() < 1e-5);
        // Showing it again measures it again.
        set_face(&mut world, crate::demo::auto_hover::LEFT, 1, |face| {
            face.texture = aurora_prim::te::DEFAULT_TEXTURE;
        });
        settle(&mut scene, &mut world, now + Duration::from_secs(3));
        assert!((scene.automatic_hover_base(&world).unwrap() - shown).abs() < 1e-5);
        // A tint or a texture change of a shown face is no outfit change.
        let stamp = scene.auto_hover.finished;
        set_face(&mut world, crate::demo::auto_hover::LEFT, 0, |face| {
            face.color = [0.2, 0.4, 0.6, 1.0]
        });
        scene.update_auto_hover(&mut world, true, 0.0, true, now + Duration::from_secs(4));
        assert_eq!(scene.auto_hover.finished, stamp, "a tint recalibrated the outfit");
    }

    #[test]
    fn hidden_face_mask_follows_the_renderer() {
        let (mut world, _scene, _now) = fixture();
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::auto_hover::rigged_object()],
        });
        let left = crate::demo::auto_hover::LEFT;
        set_face(&mut world, left, 2, |face| face.color[3] = 0.0);
        set_face(&mut world, left, 3, |face| face.texture = aurora_prim::te::TRANSPARENT_TEXTURE);
        set_face(&mut world, left, 4, |face| face.color[3] = 0.5);
        let index = world
            .objects
            .index_of(&crate::world::objects::ObjKey { region: 1, local_id: left })
            .unwrap();
        let mask = hidden_faces(world.objects.get(index).unwrap());
        assert_eq!(mask & 0b11111, 0b01100, "alpha 0 and the transparent texture only");
        assert!(face_shown(mask, 4) && !face_shown(mask, 2) && face_shown(mask, 64));
    }

    #[test]
    fn invalid_loaded_mesh_keeps_the_previous_height_instead_of_applying_a_partial_estimate() {
        let lib = library();
        let estimate = Estimate {
            values: Vec::new(),
            system_feet_hidden: false,
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
                        creator: uuid::Uuid::nil(),
                    },
                    bytes: super::super::mesh_bytes::MeshBytes::whole(Arc::new(vec![0xff; 4])),
                    skin: None,
                }),
                rigid: Some(Mat4::IDENTITY),
                hidden: 0,
                ankle: None,
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
