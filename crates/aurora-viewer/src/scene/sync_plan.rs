//! Placement of the objects the scene sync visits: what each one needs
//! this frame (a transform-only update, or the full sync), computed in
//! parallel over a read-only view of the world and the scene.

use super::anchors::{self, AnchorRef};
use super::meshes::MeshStreamer;
use super::sync_sets::{PendingGeom, SyncState, Xf, XfCache};
use super::textures::TextureStreamer;
use super::{GeomEntry, GeomKey, GeomState, GpuGeom, ObjGpu, Scene, anim, animesh, lod_for};
use crate::fast_hash::FastMap;
use crate::world::World;
use crate::world::objects::Object;
use aurora_render::arena::RecordStore;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// Radius of an avatar for the detail level of its rigged meshes and the
/// rank of whatever it wears: the diagonal of its animated extents
/// (LLVOAvatar::getLastAnimExtents), about 2.2 m standing.
pub(super) const AVATAR_LOD_RADIUS: f32 = 2.2;

/// Radius a mesh is ranked by in the download queue, from its LOD radius.
/// Anything worn by an avatar ranks as the whole avatar (calculate_score,
/// llmeshrepository.cpp: rigged or attachment, the avatar's extents),
/// already before its header tells it is rigged: ranked by its own scale (a
/// fraction of a metre for a head or its parts), a nearby avatar's head
/// waited behind the scenery. Cold-cache arrivals at KB on 2026-10-27 (a
/// fast link, 24 requests at once): 43 avatars shown only by the 60 s limit
/// ranked by their own scale, 7 ranked as their avatar; the scenery's
/// textures start later meanwhile (1,100 instead of 3,100 in the first
/// 30 s), which more requests at once make up for. Turned off on 2026-10-18,
/// when a slow link was the limit. `AURORA_WORN_RANK=own` ranks them by
/// their own scale, as before.
pub(super) fn mesh_rank_radius(lod_radius: f32, worn: bool) -> f32 {
    static AVATAR_SCALE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let avatar_scale = *AVATAR_SCALE.get_or_init(|| std::env::var("AURORA_WORN_RANK").as_deref() != Ok("own"));
    rank_radius(lod_radius, worn && avatar_scale)
}

fn rank_radius(lod_radius: f32, worn: bool) -> f32 {
    if worn { lod_radius.max(AVATAR_LOD_RADIUS) } else { lod_radius }
}

/// What the sync does with an object this frame.
pub(super) enum Plan {
    /// Only its placement changed: applied from the parallel planning.
    Move(MoveSync),
    /// Full `sync_object` (geometry, faces, avatars…), with its transform
    /// when known. When the frame has no time left for it, the object
    /// waits in the backlog; `interim` then keeps the faces it already has
    /// in place (a moving object does not freeze while it waits).
    Full { xf: Option<Xf>, interim: Option<MoveSync> },
}

impl Plan {
    pub(super) fn xf(&self) -> Option<Xf> {
        match self {
            Plan::Move(m) => Some(m.xf),
            Plan::Full { xf, .. } => *xf,
        }
    }
}

/// A transform-only update, computed in parallel.
pub(super) struct MoveSync {
    pub(super) xf: Xf,
    pub(super) model: [[f32; 4]; 4],
    pub(super) center: Vec3,
    pub(super) radius: f32,
    pub(super) owner_avatar: Option<usize>,
    pub(super) mirror: bool,
    /// Placed by the GPU: `model` is relative to this anchor (a rigged
    /// mesh: its skeleton's, and the identity).
    pub(super) anchor: Option<AnchorRef>,
    pub(super) local_center: Vec3,
    pub(super) posed_rest: Option<[f32; 4]>,
    /// Some face record holds another model matrix or another anchor.
    pub(super) changed: bool,
    /// Updated (`needs_records`): the flag is cleared once applied.
    pub(super) clear_flags: bool,
    /// (radius, distance) its detail level was checked with (a HUD
    /// attachment: no size, it never changes level); None when it was not.
    pub(super) lod: Option<(f32, f32)>,
}

/// Model matrix and culling bounds of an object on its bound geometry.
pub(super) struct Placement {
    /// In the world, or relative to `anchor`.
    pub(super) model: Mat4,
    /// World bounds.
    pub(super) center: Vec3,
    pub(super) radius: f32,
    pub(super) rigged: bool,
    pub(super) skeleton_owner: Option<(Uuid, usize)>,
    /// Placed by the GPU (anchors.rs): a worn object from its attachment
    /// point, a rigged mesh from its skeleton's anchor.
    pub(super) anchor: Option<AnchorRef>,
    /// `center` in the anchor's space.
    pub(super) local_center: Vec3,
    /// Rigged mesh posed by the GPU: the sphere of its skeleton in the
    /// anchor's space, which stands for its bounds without a sane pose.
    pub(super) posed_rest: Option<[f32; 4]>,
}

/// Scene state the frame list consults (sync_sets.rs).
pub(super) struct FrameState<'a> {
    pub(super) gpu: &'a [ObjGpu],
    pub(super) geoms: &'a FastMap<GeomKey, GeomEntry>,
    pub(super) textures: &'a TextureStreamer,
    pub(super) mat_gen: u64,
    pub(super) mat_changes: &'a super::mat_changes::MatChanges,
    /// HUD attachments are placed when they change, not with their wearer
    /// at each frame (false with AURORA_SYNC_SLICE).
    pub(super) still_huds: bool,
}

impl SyncState for FrameState<'_> {
    fn rigged(&self, idx: usize) -> bool {
        self.gpu.get(idx).is_some_and(|g| g.moves_with_skeleton())
    }

    fn hud(&self, idx: usize) -> bool {
        self.still_huds && self.gpu.get(idx).is_some_and(|g| g.hud)
    }

    fn anchored(&self, idx: usize) -> bool {
        self.gpu.get(idx).is_some_and(|g| g.anchor.is_some())
    }

    fn needs_sync(&self, idx: usize, o: &Object) -> bool {
        o.render.needs_records
            || o.shape_dirty
            || o.material_dirty
            || self
                .gpu
                .get(idx)
                .is_some_and(|g| self.mat_changes.stale(&g.material_ids, g.mat_generation, self.mat_gen))
    }

    fn pending_due(&self, idx: usize) -> PendingGeom {
        let Some(g) = self.gpu.get(idx) else {
            return PendingGeom::Done;
        };
        let Some(key) = g.wanted_geom else {
            return PendingGeom::Done;
        };
        if g.geom == Some(key) {
            return PendingGeom::Done;
        }
        match self.geoms.get(&key).map(|e| &e.state) {
            Some(GeomState::Ready(_)) => PendingGeom::Due,
            // still being built or downloaded; a failed one is asked again
            // once the geometry cache drops it (Scene::geom_changed)
            Some(GeomState::Pending) | Some(GeomState::Failed) => PendingGeom::Waits(wait_token(&key)),
            // a sculpt waits for its texture, or its placeholder when the
            // texture is missing; anything else is asked for
            None => match key {
                GeomKey::Sculpt { hash, lod } => {
                    let due = g.sculpt_wait.is_some_and(|t| {
                        self.textures.sculpt_map(&t).is_some()
                            || self.textures.is_unavailable(&t) && {
                                let placeholder = GeomKey::SculptPlaceholder { hash, lod };
                                g.geom != Some(placeholder)
                                    && !matches!(
                                        self.geoms.get(&placeholder).map(|e| &e.state),
                                        Some(GeomState::Pending) | Some(GeomState::Failed)
                                    )
                            }
                    });
                    if due { PendingGeom::Due } else { PendingGeom::Later }
                }
                _ => PendingGeom::Due,
            },
        }
    }
}

/// What the objects parked for geometry `key` are woken by (never 0):
/// equal keys give equal tokens, a rare equal token for another key only
/// wakes them for nothing.
pub(super) fn wait_token(key: &GeomKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    key.hash(&mut h);
    h.finish().max(1)
}

/// The detail level of an object as the debug overlay shows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodInfo {
    /// Where the object is drawn (centre and radius of its bounds).
    pub at: Vec3,
    pub bounds: f32,
    /// Level of the geometry it is drawn with (0 lowest, 3 highest).
    pub shown: u8,
    /// Level its distance asks for at this moment.
    pub asked: u8,
    /// Radius and distance the level comes from (`lod_metrics`).
    pub radius: f32,
    pub distance: f32,
    pub rigged: bool,
}

/// LLVolume::mLODScaleBias (LLVolume::generate): what scales an object's
/// size into the radius its detail level is chosen with. Sculpts follow
/// their path like any other volume (a circle, as the build tools make
/// them: 0.6); only meshes always take 0.5.
fn lod_scale_bias(volume: &aurora_prim::params::VolumeParams) -> Vec3 {
    use aurora_prim::params::{LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE};
    let path = volume.path.curve_type & 0xF0;
    let profile = volume.profile.curve_type & 0x0F;
    if volume.is_mesh() {
        Vec3::splat(0.5)
    } else if path == LL_PCODE_PATH_LINE && profile == LL_PCODE_PROFILE_CIRCLE {
        // cylinders don't care about the Z axis
        Vec3::new(0.6, 0.6, 0.0)
    } else if path == LL_PCODE_PATH_CIRCLE {
        Vec3::splat(0.6)
    } else {
        Vec3::splat(0.5)
    }
}

/// Read-only view of the world and the scene for placing objects; shared
/// by the rayon workers of the sync's planning.
pub(super) struct SyncCtx<'a> {
    world: &'a World,
    meshes: &'a MeshStreamer,
    geoms: &'a FastMap<GeomKey, GeomEntry>,
    gpu: &'a [ObjGpu],
    palette_slots: &'a FastMap<Uuid, u32>,
    skin_binds: &'a super::skin_binds::SkinBinds,
    palettes: &'a [[[f32; 4]; 4]],
    anim_boxes: &'a FastMap<Uuid, super::anim_box::AnimBox>,
    records: &'a RecordStore,
    xf: &'a XfCache,
    now: Instant,
    eye: Vec3,
    lod_factor: f32,
    fov_y: f32,
    mesh_lod_overrides: &'a HashMap<crate::world::objects::ObjKey, u8>,
    mesh_lod_resets: &'a HashMap<crate::world::objects::ObjKey, super::mesh_inspection::LodReset>,
    mat_gen: u64,
    mat_changes: &'a super::mat_changes::MatChanges,
}

impl SyncCtx<'_> {
    /// An object's transform, from this frame's cache when already placed.
    fn transform(&self, idx: usize) -> Option<Xf> {
        self.xf
            .get(idx)
            .or_else(|| Scene::object_transform_in(self.world, idx, self.now, 0, Some(self.xf)))
    }

    /// (radius, distance) used for an object's LOD (LLVOVolume::calcLOD):
    /// rigged meshes use their skeleton owner's distance and size (an
    /// animated object's animated box, anim_box.rs), other volumes the
    /// LOD-biased scale (LLVolume::mLODScaleBias).
    pub(super) fn lod_metrics(&self, idx: usize, o: &Object, pos: Vec3) -> (f32, f32) {
        let world = self.world;
        let eye = self.eye;
        if let Some(radius) = super::plants::tree_lod_radius(o) {
            return (radius, pos.distance(eye));
        }
        if o.visual_volume().is_mesh() {
            let rigged = o
                .visual_volume()
                .sculpt
                .is_some_and(|s| self.meshes.meta(&s.texture).is_some_and(|m| m.skin.is_some()));
            if rigged
                && let Some((owner_id, owner)) = Scene::skeleton_owner(world, idx)
                && let (Some(ow), Some((op, _, _))) = (world.objects.get(owner), self.transform(owner))
            {
                // animesh: half the diagonal of its animated box, from its
                // centre; before its first box, the size of its root
                if !ow.is_avatar()
                    && let Some(b) = self.anim_boxes.get(&owner_id)
                {
                    return b.lod_metrics(eye);
                }
                // avatars: diagonal of the animated extents
                let radius = if ow.is_avatar() {
                    AVATAR_LOD_RADIUS
                } else {
                    (ow.visual_scale_at(self.now).length() * 0.5).max(0.5)
                };
                return (radius, op.distance(eye));
            }
            return ((o.visual_scale_at(self.now) * 0.5).length(), pos.distance(eye));
        }
        (
            (lod_scale_bias(&o.visual_volume()) * o.visual_scale_at(self.now)).length(),
            pos.distance(eye),
        )
    }

    /// What the debug overlay shows of the detail level of object `idx`
    /// (ui/debug_overlay.rs): None for what has no level of its own
    /// (avatars, HUD attachments, objects not drawn yet).
    pub(super) fn lod_info(&self, idx: usize) -> Option<LodInfo> {
        let g = self.gpu.get(idx)?;
        let o = self.world.objects.get(idx)?;
        if g.is_avatar || g.hud || g.faces.is_empty() {
            return None;
        }
        let (pos, _, _) = self.transform(idx)?;
        let (radius, distance) = self.lod_metrics(idx, o, pos);
        let level = |key: Option<GeomKey>| match key? {
            GeomKey::Prim { lod, .. }
            | GeomKey::Flexible { lod, .. }
            | GeomKey::Sculpt { lod, .. }
            | GeomKey::SculptPlaceholder { lod, .. }
            | GeomKey::Mesh { lod, .. }
            | GeomKey::Tree { lod, .. } => Some(lod),
            GeomKey::Grass { .. } | GeomKey::AvatarPart(_) | GeomKey::AvatarMorph { .. } => None,
        };
        Some(LodInfo {
            at: g.center,
            bounds: g.radius,
            shown: level(g.geom)?,
            asked: super::mesh_inspection::effective_lod(
                self.mesh_lod_overrides,
                self.mesh_lod_resets,
                o,
                false,
                lod_for(radius, distance, self.lod_factor, self.fov_y),
            ),
            radius,
            distance,
            rigged: g.rigged,
        })
    }

    /// Model matrix and bounds of an object drawn with `geom`.
    pub(super) fn placement(&self, idx: usize, o: &Object, xf: Xf, geom_key: GeomKey, geom: &GpuGeom) -> Placement {
        let world = self.world;
        let (pos, rot, _) = xf;
        let scale = (o.visual_scale_at(self.now) * if xf.2 { world.hud_scale(idx) } else { 1.0 }).max(Vec3::splat(0.001));
        let skinned = matches!(geom_key, GeomKey::Mesh { id, .. } if self.meshes.meta(&id).is_some_and(|m| m.skin.is_some()));
        let skeleton_owner = skinned.then(|| Scene::skeleton_owner(world, idx)).flatten();
        // A skinned mesh is drawn rigged only on a skeleton (worn, or an
        // animated object); rezzed in the world, it is a static mesh
        // (LLVolumeGeometryManager::rebuildGeom, indra/newview/llvovolume.cpp):
        // its geometry's bind shape is undone.
        let rigged = skeleton_owner.is_some();
        let unbind = match geom_key {
            GeomKey::Mesh { id, .. } if skinned && !rigged => self.meshes.unbind(&id),
            _ => None,
        };
        let owner_xf = skeleton_owner.and_then(|(_, owner_idx)| self.transform(owner_idx));
        let on_gpu = !xf.2 && anchors::enabled();
        // Rigged meshes are already in avatar skeleton space (bind pose):
        // placed with the skeleton of the avatar that wears them.
        let skeleton = match (skeleton_owner, owner_xf) {
            (Some((_, owner_idx)), Some((ap, ar, _))) => {
                let hud_scale = if xf.2 { world.hud_scale(idx) } else { 1.0 };
                Some((
                    owner_idx,
                    ap,
                    anchors::skeleton_model(world, self.meshes, owner_idx, ap, ar, hud_scale),
                ))
            }
            _ => None,
        };
        // its records read the skeleton's place in its palette: nothing to
        // write again when the wearer moves
        let skeleton_anchor = skeleton.filter(|_| on_gpu).and_then(|(owner_idx, ..)| {
            let owner = world.objects.get(owner_idx)?;
            let matrix = Scene::anchor_matrix(self.palette_slots, self.palettes.len(), owner, 0)?;
            Some(AnchorRef {
                wearer: owner_idx as u32,
                slot: 0,
                matrix,
            })
        });
        // a worn object that only follows its attachment point: placed
        // relative to it once, the GPU does the rest
        // (a Linden plant keeps its own placement, worn or not)
        let plant = super::plants::plant_model(geom_key, o, pos, rot);
        let point_anchor = (!rigged && on_gpu && plant.is_none())
            .then(|| {
                anchors::anchored(world, o, |wearer, slot| {
                    Scene::anchor_matrix(self.palette_slots, self.palettes.len(), wearer, slot)
                })
            })
            .flatten();
        let unbind = unbind.unwrap_or(Mat4::IDENTITY);
        let world_model = match (skeleton, plant) {
            (Some((_, _, model)), _) => model,
            (None, Some(model)) => model,
            (None, None) => Mat4::from_scale_rotation_translation(scale, rot, pos) * unbind,
        };
        let model = match (skeleton_anchor, point_anchor) {
            (Some(_), _) => Mat4::IDENTITY,
            (None, Some((_, local_pos, local_rot))) => Mat4::from_scale_rotation_translation(scale, local_rot, local_pos) * unbind,
            (None, None) => world_model,
        };
        let center_local = (geom.min + geom.max) * 0.5;
        let half = (geom.max - geom.min) * 0.5;
        let mut center = world_model.transform_point3(center_local);
        let col_scale = world_model
            .x_axis
            .truncate()
            .length()
            .max(world_model.y_axis.truncate().length())
            .max(world_model.z_axis.truncate().length());
        let mut radius = (half.length() * col_scale).max(0.05);
        let mut posed_rest = None;
        // the bind-pose bounds can be anywhere (the bones bring the mesh
        // onto the skeleton): cull with the wearer's extent, as LL does
        if let Some((owner_idx, ap, _)) = skeleton
            && let Some(ow) = world.objects.get(owner_idx)
        {
            center = ap;
            radius = anchors::skeleton_extent(ow);
            let pelvis = world.avatar_lib.pelvis;
            posed_rest = skeleton_anchor.map(|_| [pelvis.x, pelvis.y, pelvis.z, radius]);
        }
        let model_for_bounds = world_model;
        if let Some((owner, _)) = skeleton_owner
            && let Some(&slot) = self.palette_slots.get(&owner)
            && let Some(binds) = match geom_key {
                GeomKey::Mesh { id, .. } => self.skin_binds.base_of(&id),
                _ => None,
            }
            && let Some((c, r)) = animesh::posed_bounds(&geom.joint_bounds, |j| {
                let b = self.skin_binds.list().get(binds as usize + j as usize)?;
                let p = self.palettes.get(slot as usize * anim::PALETTE_JOINTS + b.joint[0] as usize)?;
                Some(model_for_bounds * Mat4::from_cols_array_2d(p) * Mat4::from_cols_array_2d(&b.inverse_bind))
            })
        {
            center = c;
            radius = r;
        }
        let local_center = match (skeleton_anchor, point_anchor) {
            (Some(_), _) => world_model.inverse().transform_point3(center),
            (None, Some(_)) => model.transform_point3(center_local),
            (None, None) => center,
        };
        Placement {
            model,
            center,
            radius,
            rigged,
            skeleton_owner,
            anchor: skeleton_anchor.or(point_anchor.map(|a| a.0)),
            local_center,
            posed_rest,
        }
    }

    /// The geometry the object's place and the camera ask for at this
    /// moment (what a check of its detail level would want); None when it
    /// cannot be placed.
    pub(super) fn wanted_now(&self, idx: usize, o: &Object) -> Option<Option<GeomKey>> {
        if !self.world.parcel_object_visible(idx) {
            return Some(None);
        }
        let (pos, _, hud) = self.transform(idx)?;
        let (radius, distance) = self.lod_metrics(idx, o, pos);
        let lod = super::mesh_inspection::effective_lod(
            self.mesh_lod_overrides,
            self.mesh_lod_resets,
            o,
            hud,
            lod_for(radius, distance, self.lod_factor, self.fov_y),
        );
        Some(Scene::object_geom_key(o, lod))
    }

    /// What this frame's sync does with an object. A transform-only update
    /// is planned here when nothing else changed: same geometry and detail
    /// level, no new faces, materials or skeleton; anything else is left to
    /// the full sync.
    pub(super) fn plan(&self, idx: usize) -> Plan {
        let world = self.world;
        let none = Plan::Full { xf: None, interim: None };
        if !world.parcel_object_visible(idx) {
            return none;
        }
        let Some(o) = world.objects.get(idx) else {
            return none;
        };
        let Some(xf) = self.transform(idx) else {
            return none;
        };
        let Some(g) = self.gpu.get(idx) else {
            return Plan::Full {
                xf: Some(xf),
                interim: None,
            };
        };
        if o.is_avatar() || g.is_avatar {
            // (never put off: no interim placement)
            return Plan::Full {
                xf: Some(xf),
                interim: None,
            };
        }
        let full = || Plan::Full {
            xf: Some(xf),
            interim: self.interim_move(idx, o, g, xf),
        };
        if o.shape_dirty || o.material_dirty {
            return full();
        }
        if g.sculpt_wait.is_some() || self.mat_changes.stale(&g.material_ids, g.mat_generation, self.mat_gen) {
            return full();
        }
        let (pos, _, hud) = xf;
        let metrics = if hud { (0.0, 0.0) } else { self.lod_metrics(idx, o, pos) };
        let lod = super::mesh_inspection::effective_lod(
            self.mesh_lod_overrides,
            self.mesh_lod_resets,
            o,
            hud,
            lod_for(metrics.0, metrics.1, self.lod_factor, self.fov_y),
        );
        let Some(key) = Scene::object_geom_key(o, lod) else {
            return full();
        };
        if g.wanted_geom != Some(key) || g.geom != Some(key) {
            return full();
        }
        // a sculpt keeps its texture held (the full sync acquires it again
        // after its faces were rebuilt)
        if matches!(key, GeomKey::Sculpt { .. }) && o.visual_volume().sculpt.is_some_and(|s| !g.tex_ids.contains(&s.texture)) {
            return full();
        }
        let Some(GeomState::Ready(geom)) = self.geoms.get(&key).map(|e| &e.state) else {
            return full();
        };
        let p = self.placement(idx, o, xf, key, geom);
        if g.rigged != p.rigged
            || g.skeleton_owner != p.skeleton_owner.map(|s| s.0)
            || g.built_faces != geom.faces.iter().filter(|f| f.is_some()).count()
        {
            return full();
        }
        let model = p.model.to_cols_array_2d();
        let changed = g.hud != hud || self.records_differ(g, &model, p.anchor);
        Plan::Move(MoveSync {
            xf,
            model,
            center: p.center,
            radius: p.radius,
            owner_avatar: Scene::wearer_avatar(world, idx).map(|(_, i)| i),
            // reflection probe with the box and mirror flags (LLReflectionProbeParams)
            mirror: o.extra.reflection_probe.is_some_and(|p| p.flags & 0x4 != 0 && p.flags & 0x1 != 0),
            anchor: p.anchor,
            local_center: p.local_center,
            posed_rest: p.posed_rest,
            changed,
            clear_flags: o.render.needs_records,
            lod: Some(metrics),
        })
    }

    /// Some face record of the object holds another model matrix, or is
    /// placed from another anchor.
    fn records_differ(&self, g: &ObjGpu, model: &[[f32; 4]; 4], anchor: Option<AnchorRef>) -> bool {
        let anchor = anchor.map(|a| a.matrix);
        g.faces.iter().any(|f| {
            self.records
                .get(f.record)
                // (a skinned record has its palette there, never an anchor)
                .is_some_and(|r| r.model != *model || r.anchor() != anchor)
        })
    }

    /// Placement of the faces an object already has, for the frames its
    /// full sync waits in the backlog: only when they moved, and when the
    /// geometry they were built on is still placed the same way (a mesh
    /// found rigged since changes space with its new faces only). The
    /// object stays to be synced: no flag is cleared.
    fn interim_move(&self, idx: usize, o: &Object, g: &ObjGpu, xf: Xf) -> Option<MoveSync> {
        if g.faces.is_empty() {
            return None;
        }
        let key = g.geom?;
        let GeomState::Ready(geom) = &self.geoms.get(&key)?.state else {
            return None;
        };
        let p = self.placement(idx, o, xf, key, geom);
        if g.rigged != p.rigged || g.skeleton_owner != p.skeleton_owner.map(|s| s.0) {
            return None;
        }
        let model = p.model.to_cols_array_2d();
        let changed = self.records_differ(g, &model, p.anchor);
        changed.then(|| MoveSync {
            xf,
            model,
            center: p.center,
            radius: p.radius,
            owner_avatar: Scene::wearer_avatar(self.world, idx).map(|(_, i)| i),
            mirror: g.mirror,
            anchor: p.anchor,
            local_center: p.local_center,
            posed_rest: p.posed_rest,
            changed,
            clear_flags: false,
            lod: None,
        })
    }
}

impl Scene {
    /// Read-only view of what placing an object needs: shared by the
    /// parallel planning of the frame's objects and the full sync.
    pub(super) fn sync_ctx<'a>(&'a self, world: &'a World, records: &'a RecordStore, eye: Vec3, now: Instant) -> SyncCtx<'a> {
        SyncCtx {
            world,
            meshes: &self.meshes,
            geoms: &self.geoms,
            gpu: &self.gpu,
            palette_slots: &self.palette_slots,
            skin_binds: &self.skin_binds,
            palettes: &self.palettes,
            anim_boxes: &self.anim_boxes,
            records,
            xf: &self.xf_cache,
            now,
            eye,
            lod_factor: self.lod_factor,
            fov_y: self.fov_y,
            mesh_lod_overrides: &self.mesh_lod_overrides,
            mesh_lod_resets: &self.mesh_lod_resets,
            mat_gen: self.materials.generation,
            mat_changes: &self.mat_changes,
        }
    }

    /// The detail levels of the objects within `range` of `eye`, for the
    /// debug overlay.
    pub fn lod_infos(&self, world: &World, records: &RecordStore, eye: Vec3, now: Instant, range: f32) -> Vec<LodInfo> {
        let ctx = self.sync_ctx(world, records, eye, now);
        (0..self.gpu.len())
            .filter(|&idx| self.gpu[idx].center.distance(eye) - self.gpu[idx].radius < range)
            .filter_map(|idx| ctx.lod_info(idx))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_prim::params::{
        LL_PCODE_PATH_CIRCLE, LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, LL_SCULPT_TYPE_MESH,
        LL_SCULPT_TYPE_SPHERE, SculptParams, VolumeParams,
    };

    #[test]
    fn parcel_visibility_crossings_revisit_gpu_placed_attachments_in_both_directions() {
        let (mut world, mut scene, _, _) = crate::scene::picking::tests::scene_with_two_objects();
        let mut settings = crate::settings::Settings::default();
        crate::demo::parcel_visibility::install(&mut world, &mut settings, "always");
        let avatar = world.objects.index_of_uuid(&crate::demo::DEMO_LOUP).unwrap();
        let local_id = world.objects.get(avatar).unwrap().key.local_id;
        for event in crate::demo::own_rigged_events(local_id) {
            world.apply(event);
        }
        let child = world
            .objects
            .children_of(&world.objects.get(avatar).unwrap().key)
            .iter()
            .copied()
            .find(|&idx| world.objects.get(idx).unwrap().key.local_id == crate::demo::OWN_RIGGED)
            .unwrap();
        scene.gpu.resize_with(world.objects.slots.len(), Default::default);
        scene.sets.own_moving.insert(avatar);
        world.update_parcel_visibility(&settings.parcel_visibility, Instant::now());
        for (x, previously_hidden) in [(120.0, false), (140.0, true)] {
            scene.sets.dirty.take();
            world.objects.get_mut_untracked(avatar).unwrap().position = Vec3::new(x, 140.0, 25.0);
            world.objects.get_mut_untracked(child).unwrap().render.needs_records = false;
            scene.gpu[avatar].parcel_hidden = previously_hidden;
            scene.sync_parcel_crossings(&mut world);
            assert!(
                scene.sets.dirty.contains(child),
                "the attachment changes visibility with its wearer"
            );
            assert!(world.objects.get(child).unwrap().render.needs_records);
        }
    }

    fn volume(path: u8, profile: u8, sculpt: Option<u8>) -> VolumeParams {
        let mut v = VolumeParams::default();
        v.path.curve_type = path;
        v.profile.curve_type = profile;
        v.sculpt = sculpt.map(|sculpt_type| SculptParams {
            texture: Uuid::from_u128(7),
            sculpt_type,
        });
        v
    }

    /// LLVolume::generate: 0.5 by default, cylinders without their height,
    /// 0.6 along a circle, and a mesh always 0.5.
    #[test]
    fn the_lod_radius_follows_the_volume_like_firestorm() {
        let bias = |path, profile, sculpt| lod_scale_bias(&volume(path, profile, sculpt));
        assert_eq!(bias(LL_PCODE_PATH_LINE, 0x01, None), Vec3::splat(0.5));
        assert_eq!(bias(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, None), Vec3::new(0.6, 0.6, 0.0));
        assert_eq!(bias(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, None), Vec3::splat(0.6));
        // a sculpt is a sphere under its map: the circle's 0.6, not 0.5
        assert_eq!(
            bias(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, Some(LL_SCULPT_TYPE_SPHERE)),
            Vec3::splat(0.6)
        );
        // a mesh whatever the path it was given
        assert_eq!(
            bias(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, Some(LL_SCULPT_TYPE_MESH)),
            Vec3::splat(0.5)
        );
        assert_eq!(
            bias(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, Some(LL_SCULPT_TYPE_MESH)),
            Vec3::splat(0.5)
        );
        // a 1 m sculpt keeps its finest level to 10 m, as in Firestorm (8.5 m with 0.5)
        let radius = (Vec3::splat(0.6) * Vec3::ONE).length();
        assert_eq!(lod_for(radius, 10.0, 2.0, 60f32.to_radians()), 3);
        assert_eq!(lod_for((Vec3::splat(0.5) * Vec3::ONE).length(), 10.0, 2.0, 60f32.to_radians()), 2);
    }

    /// calculate_score: what an avatar wears ranks as the avatar, header
    /// known or not; a larger attachment keeps its own size.
    #[test]
    fn worn_meshes_rank_as_their_avatar() {
        assert_eq!(rank_radius(0.12, true), AVATAR_LOD_RADIUS);
        assert_eq!(rank_radius(4.0, true), 4.0);
        assert_eq!(rank_radius(0.12, false), 0.12);
    }

    /// LLVOVolume::sculpt: the sphere placeholder only for a missing map,
    /// asked for once, and the sculpt as soon as its map decodes.
    #[test]
    fn a_sculpt_with_a_missing_map_waits_for_its_placeholder() {
        let map = Uuid::from_u128(7);
        let (sculpt, placeholder) = (GeomKey::Sculpt { hash: 1, lod: 2 }, GeomKey::SculptPlaceholder { hash: 1, lod: 2 });
        let mut gpu = vec![ObjGpu {
            wanted_geom: Some(sculpt),
            sculpt_wait: Some(map),
            ..Default::default()
        }];
        let mut geoms = FastMap::default();
        let mut textures = TextureStreamer::new(std::path::PathBuf::new());
        let mat_changes = super::super::mat_changes::MatChanges::default();
        let due = |gpu: &[ObjGpu], geoms: &FastMap<GeomKey, GeomEntry>, textures: &TextureStreamer| {
            let state = FrameState {
                gpu,
                geoms,
                textures,
                mat_gen: 0,
                mat_changes: &mat_changes,
                still_huds: false,
            };
            state.pending_due(0)
        };
        // still loading: nothing to build, as there (no event for the
        // texture: not parked)
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Later);
        textures.test_missing(map);
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Due);
        let entry = |state| GeomEntry {
            state,
            refs: 0,
            zero_since: 0,
        };
        geoms.insert(placeholder, entry(GeomState::Pending));
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Later);
        geoms.insert(placeholder, entry(GeomState::Failed));
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Later);
        geoms.remove(&placeholder);
        gpu[0].geom = Some(placeholder);
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Later);
        // the map decoded: the sculpt replaces the sphere
        textures.test_sculpt_map(map);
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Due);
        // being built: parked for it, until it is ready
        geoms.insert(sculpt, entry(GeomState::Pending));
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Waits(wait_token(&sculpt)));
        assert_ne!(wait_token(&sculpt), wait_token(&placeholder));
        let built = GpuGeom {
            faces: Vec::new(),
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            joint_bounds: Vec::new(),
            pick_faces: Vec::new(),
        };
        geoms.insert(sculpt, entry(GeomState::Ready(std::sync::Arc::new(built))));
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Due);
        gpu[0].geom = Some(sculpt);
        assert_eq!(due(&gpu, &geoms, &textures), PendingGeom::Done);
    }
}
