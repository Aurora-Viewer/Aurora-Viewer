//! Placement of the objects the scene sync visits: what each one needs
//! this frame (a transform-only update, or the full sync), computed in
//! parallel over a read-only view of the world and the scene.

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
    /// Some face record holds another model matrix.
    pub(super) changed: bool,
    /// Updated (`needs_records`): the flag is cleared once applied.
    pub(super) clear_flags: bool,
}

/// Model matrix and culling bounds of an object on its bound geometry.
pub(super) struct Placement {
    pub(super) model: Mat4,
    pub(super) center: Vec3,
    pub(super) radius: f32,
    pub(super) rigged: bool,
    pub(super) skeleton_owner: Option<(Uuid, usize)>,
}

/// Scene state the frame list consults (sync_sets.rs).
pub(super) struct FrameState<'a> {
    pub(super) gpu: &'a [ObjGpu],
    pub(super) geoms: &'a HashMap<GeomKey, GeomEntry>,
    pub(super) textures: &'a TextureStreamer,
    pub(super) mat_gen: u64,
}

impl SyncState for FrameState<'_> {
    fn rigged(&self, idx: usize) -> bool {
        self.gpu.get(idx).is_some_and(|g| g.rigged)
    }

    fn needs_sync(&self, idx: usize, o: &Object) -> bool {
        o.render.needs_records
            || o.shape_dirty
            || o.material_dirty
            || self
                .gpu
                .get(idx)
                .is_some_and(|g| !g.material_ids.is_empty() && g.mat_generation != self.mat_gen)
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

/// Read-only view of the world and the scene for placing objects; shared
/// by the rayon workers of the sync's planning.
pub(super) struct SyncCtx<'a> {
    world: &'a World,
    meshes: &'a MeshStreamer,
    geoms: &'a HashMap<GeomKey, GeomEntry>,
    gpu: &'a [ObjGpu],
    palette_slots: &'a HashMap<Uuid, u32>,
    skin_bind_ranges: &'a HashMap<Uuid, u32>,
    skin_binds: &'a [aurora_render::SkinBind],
    palettes: &'a [[[f32; 4]; 4]],
    records: &'a RecordStore,
    xf: &'a XfCache,
    now: Instant,
    eye: Vec3,
    lod_factor: f32,
    fov_y: f32,
    mat_gen: u64,
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
        if o.volume.is_mesh() {
            let rigged = o
                .volume
                .sculpt
                .is_some_and(|s| self.meshes.meta(&s.texture).is_some_and(|m| m.skin.is_some()));
            if rigged
                && let Some((_, owner)) = Scene::skeleton_owner(world, idx)
                && let (Some(ow), Some((op, _, _))) = (world.objects.get(owner), self.transform(owner))
            {
                // avatars: diagonal of the animated extents (~2.2 m);
                // animesh: half of it (LL uses the dynamic box)
                let radius = if ow.is_avatar() { 2.2 } else { (ow.scale.length() * 0.5).max(0.5) };
                return (radius, op.distance(eye));
            }
            return ((o.scale * 0.5).length(), pos.distance(eye));
        }
        let path = o.volume.path.curve_type & 0xF0;
        let profile = o.volume.profile.curve_type & 0x0F;
        let bias = if o.volume.is_sculpt() {
            Vec3::splat(0.5)
        } else if path == aurora_prim::params::LL_PCODE_PATH_LINE && profile == aurora_prim::params::LL_PCODE_PROFILE_CIRCLE {
            // cylinders don't care about the Z axis
            Vec3::new(0.6, 0.6, 0.0)
        } else if path == aurora_prim::params::LL_PCODE_PATH_CIRCLE {
            Vec3::splat(0.6)
        } else {
            Vec3::splat(0.5)
        };
        ((bias * o.scale).length(), pos.distance(eye))
    }

    /// Model matrix and bounds of an object drawn with `geom`.
    pub(super) fn placement(&self, idx: usize, o: &Object, xf: Xf, geom_key: GeomKey, geom: &GpuGeom) -> Placement {
        let world = self.world;
        let (pos, rot, _) = xf;
        let scale = o.scale.max(Vec3::splat(0.001));
        let rigged = matches!(geom_key, GeomKey::Mesh { id, .. } if self.meshes.meta(&id).is_some_and(|m| m.skin.is_some()));
        let skeleton_owner = rigged.then(|| Scene::skeleton_owner(world, idx)).flatten();
        let owner_xf = skeleton_owner.and_then(|(_, owner_idx)| self.transform(owner_idx));
        let model = match (skeleton_owner, owner_xf) {
            // Rigged meshes are already in avatar skeleton space (bind pose):
            // place them relative to the avatar that wears them.
            (Some((_, owner_idx)), Some((ap, mut ar, _))) => {
                // LLControlAvatar::matchVolumeTransform: ground animesh
                // also follow their root mesh's unscaled bind rotation.
                if let Some(root) = world.objects.get(owner_idx)
                    && !root.is_avatar()
                    && Scene::wearer_avatar(world, owner_idx).is_none()
                    && let Some(skin) = root.volume.sculpt.and_then(|s| self.meshes.meta(&s.texture)).and_then(|m| m.skin)
                {
                    let (_, bind_rot, _) = skin.bind_shape.to_scale_rotation_translation();
                    if bind_rot.is_finite() {
                        ar = (ar * bind_rot).normalize();
                    }
                }
                Mat4::from_rotation_translation(ar, ap) * Mat4::from_translation(-world.avatar_lib.pelvis)
            }
            _ => Mat4::from_scale_rotation_translation(scale, rot, pos),
        };
        let center_local = (geom.min + geom.max) * 0.5;
        let half = (geom.max - geom.min) * 0.5;
        let mut center = model.transform_point3(center_local);
        let col_scale = model
            .x_axis
            .truncate()
            .length()
            .max(model.y_axis.truncate().length())
            .max(model.z_axis.truncate().length());
        let mut radius = (half.length() * col_scale).max(0.05);
        // the bind-pose bounds can be anywhere (the bones bring the mesh
        // onto the skeleton): cull with the wearer's extent, as LL does
        if let (Some((_, owner_idx)), Some((ap, _, _))) = (skeleton_owner, owner_xf)
            && let Some(ow) = world.objects.get(owner_idx)
        {
            center = ap;
            radius = if ow.is_avatar() { 2.5 } else { (ow.scale.length() * 0.75).max(1.0) };
        }
        if let Some((owner, _)) = skeleton_owner
            && let Some(&slot) = self.palette_slots.get(&owner)
            && let Some(&binds) = match geom_key {
                GeomKey::Mesh { id, .. } => self.skin_bind_ranges.get(&id),
                _ => None,
            }
            && let Some((c, r)) = animesh::posed_bounds(&geom.joint_bounds, |j| {
                let b = self.skin_binds.get(binds as usize + j as usize)?;
                let p = self.palettes.get(slot as usize * anim::PALETTE_JOINTS + b.joint[0] as usize)?;
                Some(model * Mat4::from_cols_array_2d(p) * Mat4::from_cols_array_2d(&b.inverse_bind))
            })
        {
            center = c;
            radius = r;
        }
        Placement {
            model,
            center,
            radius,
            rigged,
            skeleton_owner,
        }
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
        if g.sculpt_wait.is_some() || (!g.material_ids.is_empty() && g.mat_generation != self.mat_gen) {
            return full();
        }
        let (pos, _, hud) = xf;
        let lod = if hud {
            3
        } else {
            let (radius, distance) = self.lod_metrics(idx, o, pos);
            lod_for(radius, distance, self.lod_factor, self.fov_y)
        };
        let Some(key) = Scene::object_geom_key(o, lod) else {
            return full();
        };
        if g.wanted_geom != Some(key) || g.geom != Some(key) {
            return full();
        }
        // a sculpt keeps its texture held (the full sync acquires it again
        // after its faces were rebuilt)
        if matches!(key, GeomKey::Sculpt { .. }) && o.volume.sculpt.is_some_and(|s| !g.tex_ids.contains(&s.texture)) {
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
        let changed = g.faces.iter().any(|f| self.records.get(f.record).is_some_and(|r| r.model != model));
        Plan::Move(MoveSync {
            xf,
            model,
            center: p.center,
            radius: p.radius,
            owner_avatar: Scene::wearer_avatar(world, idx).map(|(_, i)| i),
            // reflection probe with the box and mirror flags (LLReflectionProbeParams)
            mirror: o.extra.reflection_probe.is_some_and(|p| p.flags & 0x4 != 0 && p.flags & 0x1 != 0),
            changed,
            clear_flags: o.render.needs_records,
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
        let changed = g.faces.iter().any(|f| self.records.get(f.record).is_some_and(|r| r.model != model));
        changed.then(|| MoveSync {
            xf,
            model,
            center: p.center,
            radius: p.radius,
            owner_avatar: Scene::wearer_avatar(self.world, idx).map(|(_, i)| i),
            mirror: g.mirror,
            changed,
            clear_flags: false,
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
            skin_bind_ranges: &self.skin_bind_ranges,
            skin_binds: &self.skin_binds,
            palettes: &self.palettes,
            records,
            xf: &self.xf_cache,
            now,
            eye,
            lod_factor: self.lod_factor,
            fov_y: self.fov_y,
            mat_gen: self.materials.generation,
        }
    }
}
