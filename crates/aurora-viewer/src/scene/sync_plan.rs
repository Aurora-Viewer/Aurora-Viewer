//! Placement of the objects the scene sync visits: what each one needs
//! this frame (a transform-only update, or the full sync), computed in
//! parallel over a read-only view of the world and the scene.

use super::anchors::{self, AnchorRef};
use super::meshes::MeshStreamer;
use super::sync_sets::{SyncState, Xf, XfCache};
use super::textures::TextureStreamer;
use super::{GeomEntry, GeomKey, GeomState, GpuGeom, ObjGpu, Scene, anim, animesh, lod_for};
use crate::world::World;
use crate::world::objects::Object;
use aurora_render::arena::RecordStore;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

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
    pub(super) geoms: &'a HashMap<GeomKey, GeomEntry>,
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

    fn pending_due(&self, idx: usize) -> Option<bool> {
        let g = self.gpu.get(idx)?;
        let key = g.wanted_geom?;
        if g.geom == Some(key) {
            return None;
        }
        Some(match self.geoms.get(&key).map(|e| &e.state) {
            Some(GeomState::Ready(_)) => true,
            // still being built or downloaded; a failed one is asked again
            // once the geometry cache drops it
            Some(GeomState::Pending) | Some(GeomState::Failed) => false,
            // a sculpt waits for its texture; anything else is asked for
            None => match key {
                GeomKey::Sculpt { .. } => g.sculpt_wait.is_some_and(|t| self.textures.sculpt_map(&t).is_some()),
                _ => true,
            },
        })
    }
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
    geoms: &'a HashMap<GeomKey, GeomEntry>,
    gpu: &'a [ObjGpu],
    palette_slots: &'a HashMap<Uuid, u32>,
    skin_binds: &'a super::skin_binds::SkinBinds,
    palettes: &'a [[[f32; 4]; 4]],
    records: &'a RecordStore,
    xf: &'a XfCache,
    now: Instant,
    eye: Vec3,
    lod_factor: f32,
    fov_y: f32,
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
    /// rigged meshes use their skeleton owner's distance and size, other
    /// volumes the LOD-biased scale (LLVolume::mLODScaleBias).
    pub(super) fn lod_metrics(&self, idx: usize, o: &Object, pos: Vec3) -> (f32, f32) {
        let world = self.world;
        let eye = self.eye;
        if o.visual_volume().is_mesh() {
            let rigged = o
                .visual_volume()
                .sculpt
                .is_some_and(|s| self.meshes.meta(&s.texture).is_some_and(|m| m.skin.is_some()));
            if rigged
                && let Some((_, owner)) = Scene::skeleton_owner(world, idx)
                && let (Some(ow), Some((op, _, _))) = (world.objects.get(owner), self.transform(owner))
            {
                // avatars: diagonal of the animated extents (~2.2 m);
                // animesh: half of it (LL uses the dynamic box)
                let radius = if ow.is_avatar() {
                    2.2
                } else {
                    (ow.visual_scale().length() * 0.5).max(0.5)
                };
                return (radius, op.distance(eye));
            }
            return ((o.visual_scale() * 0.5).length(), pos.distance(eye));
        }
        ((lod_scale_bias(&o.visual_volume()) * o.visual_scale()).length(), pos.distance(eye))
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
            GeomKey::Prim { lod, .. } | GeomKey::Flexible { lod, .. } | GeomKey::Sculpt { lod, .. } | GeomKey::Mesh { lod, .. } => {
                Some(lod)
            }
            GeomKey::AvatarPart(_) | GeomKey::AvatarMorph { .. } => None,
        };
        Some(LodInfo {
            at: g.center,
            bounds: g.radius,
            shown: level(g.geom)?,
            asked: lod_for(radius, distance, self.lod_factor, self.fov_y),
            radius,
            distance,
            rigged: g.rigged,
        })
    }

    /// Model matrix and bounds of an object drawn with `geom`.
    pub(super) fn placement(&self, idx: usize, o: &Object, xf: Xf, geom_key: GeomKey, geom: &GpuGeom) -> Placement {
        let world = self.world;
        let (pos, rot, _) = xf;
        let scale = (o.visual_scale() * if xf.2 { world.hud_scale(idx) } else { 1.0 }).max(Vec3::splat(0.001));
        let rigged = matches!(geom_key, GeomKey::Mesh { id, .. } if self.meshes.meta(&id).is_some_and(|m| m.skin.is_some()));
        let skeleton_owner = rigged.then(|| Scene::skeleton_owner(world, idx)).flatten();
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
        let point_anchor = (!rigged && on_gpu)
            .then(|| {
                anchors::anchored(world, o, |wearer, slot| {
                    Scene::anchor_matrix(self.palette_slots, self.palettes.len(), wearer, slot)
                })
            })
            .flatten();
        let world_model = match skeleton {
            Some((_, _, model)) => model,
            None => Mat4::from_scale_rotation_translation(scale, rot, pos),
        };
        let model = match (skeleton_anchor, point_anchor) {
            (Some(_), _) => Mat4::IDENTITY,
            (None, Some((_, local_pos, local_rot))) => Mat4::from_scale_rotation_translation(scale, local_rot, local_pos),
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
        let (pos, _, hud) = self.transform(idx)?;
        let lod = if hud {
            3
        } else {
            let (radius, distance) = self.lod_metrics(idx, o, pos);
            lod_for(radius, distance, self.lod_factor, self.fov_y)
        };
        Some(Scene::object_geom_key(o, lod))
    }

    /// What this frame's sync does with an object. A transform-only update
    /// is planned here when nothing else changed: same geometry and detail
    /// level, no new faces, materials or skeleton; anything else is left to
    /// the full sync.
    pub(super) fn plan(&self, idx: usize) -> Plan {
        let world = self.world;
        let none = Plan::Full { xf: None, interim: None };
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
        let lod = if hud {
            3
        } else {
            lod_for(metrics.0, metrics.1, self.lod_factor, self.fov_y)
        };
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
            records,
            xf: &self.xf_cache,
            now,
            eye,
            lod_factor: self.lod_factor,
            fov_y: self.fov_y,
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
}
