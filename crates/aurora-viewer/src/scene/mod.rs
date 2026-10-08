//! GPU scene synchronisation: geometry cache, per-face draw records,
//! texture/mesh/material streaming, culling and draw-list building.

pub mod anim;
pub mod avatar;
pub mod banlines;
pub mod complexity;
pub mod impostors;
pub mod jobs;
pub mod legacy_mat;
pub mod loading;
pub mod meshes;
pub mod particles;
pub mod probes;
pub mod shape;
pub mod sounds;
pub mod textures;
pub mod water;

use crate::world::World;
use crate::world::objects::{Object, ObjectStore};
use crate::world::terrain::{self, CHUNK_CELLS, Composition};
use aurora_net::NetClient;
use aurora_prim::te::TextureFace;
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, Renderer, ShadowCaster, flags};
use avatar::AvatarLibrary;
use glam::{Mat4, Quat, Vec3, Vec4};
use jobs::{AlphaKind, JobResult, Jobs};
use meshes::{MaterialStreamer, MeshStreamer};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use textures::{TexSource, TextureStreamer};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeomKey {
    Prim { hash: u64, lod: u8 },
    Sculpt { hash: u64, lod: u8 },
    Mesh { id: Uuid, lod: u8 },
    AvatarPart(u8),
}

pub struct GpuGeom {
    pub faces: Vec<Option<MeshAlloc>>,
    pub min: Vec3,
    pub max: Vec3,
}

enum GeomState {
    Pending,
    Ready(Arc<GpuGeom>),
    Failed,
}

struct GeomEntry {
    state: GeomState,
    refs: u32,
    unused_frames: u32,
}

/// Pass a face is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    Opaque,
    OpaqueTwoSided,
    Mask,
    MaskTwoSided,
    Blend,
    Hidden,
}

/// Cached per-face draw info.
#[derive(Debug, Clone, Copy)]
pub struct FaceDraw {
    pub record: u32,
    pub cmd: DrawCmd,
    pub base_slot: u32,
    pub tex_id: Uuid,
    /// Normal, specular / metallic-roughness and emissive maps (sized like
    /// the base texture).
    pub aux_tex: [Uuid; 3],
    /// Repeats of those maps (a finer normal map needs more resolution).
    pub aux_repeats: [f32; 3],
    pub te_alpha: f32,
    pub pbr_alpha: Option<u8>, // 0 opaque, 1 blend, 2 mask
    /// Legacy material alpha mode (`legacy_mat::alpha_mode`).
    pub legacy_alpha: Option<u8>,
    pub two_sided: bool,
    pub repeats: f32,
    /// TE glow > 0: also drawn in the glow pass.
    pub glow: bool,
}

/// Per-object GPU data kept by the scene (indexed like the object slab).
#[derive(Default)]
pub struct ObjGpu {
    pub faces: Vec<FaceDraw>,
    pub tex_ids: Vec<Uuid>,
    pub material_ids: Vec<Uuid>,
    pub geom: Option<GeomKey>,
    pub wanted_geom: Option<GeomKey>,
    pub lod: u8,
    pub center: Vec3,
    pub radius: f32,
    pub tex_generation: u64,
    /// Avatars: bake slots used by worn attachments (bakes on mesh), whose
    /// system body parts are hidden.
    pub bom_mask: u64,
    /// Drawn as a too-complex silhouette (system body in flat grey).
    pub jelly: bool,
    /// Rigged mesh (skinned to an avatar skeleton).
    pub rigged: bool,
    pub mat_generation: u64,
    pub is_avatar: bool,
    pub hud: bool,
    pub sculpt_wait: Option<Uuid>,
    /// Avatar wearing this object (attachments), for the avatar limit.
    pub owner_avatar: Option<usize>,
    /// Reflection probe flagged as a mirror (box).
    pub mirror: bool,
}

struct TerrainChunk {
    alloc: MeshAlloc,
    record: u32,
    center: Vec3,
    radius: f32,
}

struct RegionGpu {
    chunks: HashMap<u32, TerrainChunk>,
    detail_slots: [u32; 4],
    detail_ids: [Uuid; 4],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SceneStats {
    pub objects: usize,
    pub visible_objects: usize,
    pub geometries: usize,
    pub geom_pending: usize,
    pub jobs: usize,
    pub cull_ms: f32,
    pub avatars_hidden: usize,
    pub sync_ms: f32,
}

pub struct Scene {
    pub jobs: Jobs,
    job_rx: crossbeam_channel::Receiver<JobResult>,
    geoms: HashMap<GeomKey, GeomEntry>,
    pub textures: TextureStreamer,
    pub meshes: MeshStreamer,
    pub materials: MaterialStreamer,
    /// Legacy materials (normal / specular maps, alpha mode) by material id.
    pub legacy_mats: legacy_mat::LegacyMaterials,
    pub avatar_lib: Arc<AvatarLibrary>,
    pub gpu: Vec<ObjGpu>,
    regions: HashMap<u64, RegionGpu>,
    water: water::WaterGpu,
    /// Ban lines of the parcel we may not enter.
    pub banlines: banlines::BanLines,
    pub particles: particles::ParticleManager,
    /// Avatar rendering complexity by avatar id (complexity.rs).
    pub avatar_complexity: HashMap<Uuid, u32>,
    /// Avatars over RenderAvatarMaxComplexity: grey silhouettes, no attachments.
    pub too_complex: std::collections::HashSet<usize>,
    /// Build tools: selected mesh / sculpted objects drawn as wireframes
    /// (object index -> root color).
    pub selection_wire: HashMap<usize, bool>,
    /// Far avatars drawn as pictures (RenderAvatarMaxNonImpostors).
    impostors: impostors::Impostors,
    /// Avatars still loading (clouds).
    loading: loading::Loading,
    /// Hide loading avatars (shown as clouds) until they are complete.
    pub hide_loading: bool,
    /// Progress bars under the names of loading avatars.
    pub loading_bars: bool,
    /// Surface area of the generated sculpts (attachment area limit).
    pub sculpt_area: HashMap<GeomKey, f32>,
    /// Attachment surface area of each avatar (m², RenderAutoMuteSurfaceAreaLimit).
    pub avatar_area: HashMap<Uuid, f32>,
    complexity_rules: u64,
    /// Debug « Afficher la transparence »: Some(rigged faces too).
    pub debug_alpha: Option<bool>,
    /// Culling view of the last lists (debug overlay).
    pub last_cull: Option<CullView>,
    complexity_at: Option<Instant>,
    complexity_max: u32,
    /// Avatars drawn (the nearest ones; own avatar always).
    pub max_avatars: usize,
    /// Build reflection draw lists (planar reflections enabled).
    pub reflections: bool,
    /// Camera vertical field of view (LOD scaling).
    pub fov_y: f32,
    /// Sky / water textures: (id, slot).
    env_tex: [(Uuid, u32); 4],
    /// Index of the agent avatar (always drawn).
    pub agent_idx: Option<usize>,
    pub lists: DrawLists,
    pub stats: SceneStats,
    frame: u32,
    pub lod_factor: f32,
    pub draw_distance: f32,
    last_origin: Option<(u32, u32)>,
    blend_tmp: Vec<(f32, DrawCmd, bool)>,
    pub anims: anim::AnimStreamer,
    /// World and interface sounds.
    pub sounds: sounds::SoundManager,
    palette_slots: HashMap<Uuid, u32>,
    palette_free: Vec<u32>,
    palette_count: u32,
    palettes: Vec<[[f32; 4]; 4]>,
    last_palette_gc: Instant,
    /// Periodic streaming summary in the log.
    last_diag: Instant,
    /// Rest skeleton (shape + joint offsets of worn meshes) per skeleton owner.
    skeletons: HashMap<Uuid, AvatarSkeleton>,
    /// Playing motions and held pose per skeleton owner.
    motions: HashMap<Uuid, anim::Controller>,
    /// Skin bindings uploaded to the GPU: the default skeleton first (system
    /// avatar), then each rigged mesh (by asset id).
    skin_binds: Vec<aurora_render::SkinBind>,
    skin_bind_ranges: HashMap<Uuid, u32>,
    skin_binds_dirty: bool,
}

/// Camera data used for culling and LOD.
#[derive(Clone, Copy)]
pub struct CullView {
    pub planes: [Vec4; 6],
    pub eye: Vec3,
    pub screen_height: f32,
    pub tan_half_fov: f32,
}

impl CullView {
    pub fn new(view_proj: Mat4, eye: Vec3, screen_height: f32, fov_y: f32) -> CullView {
        let m = view_proj.transpose();
        let r0 = m.x_axis;
        let r1 = m.y_axis;
        let r2 = m.z_axis;
        let r3 = m.w_axis;
        let norm = |p: Vec4| p / p.truncate().length().max(1e-6);
        // reverse-Z infinite: near plane is r3 - r2, far plane absent (use r2 as "far" accept-all)
        CullView {
            planes: [
                norm(r3 + r0),
                norm(r3 - r0),
                norm(r3 + r1),
                norm(r3 - r1),
                norm(r3 - r2),
                Vec4::new(0.0, 0.0, 0.0, 1.0),
            ],
            eye,
            screen_height,
            tan_half_fov: (fov_y * 0.5).tan(),
        }
    }

    #[inline]
    pub fn sphere_visible(&self, c: Vec3, r: f32) -> bool {
        for p in &self.planes {
            if p.truncate().dot(c) + p.w < -r {
                return false;
            }
        }
        true
    }

    /// Approximate projected diameter in pixels.
    #[inline]
    pub fn pixel_size(&self, c: Vec3, r: f32) -> f32 {
        let d = (c - self.eye).length().max(0.1);
        (r * 2.0 / (d * self.tan_half_fov * 2.0)) * self.screen_height
    }
}

/// Detail level like LLVOVolume::calcLOD + computeLODDetail: distance
/// factor, near ramp, FOV scaling and LLVolumeLODGroup::getDetailFromTan.
pub fn lod_for(radius: f32, distance: f32, lod_factor: f32, fov_y: f32) -> u8 {
    let mut d = distance * (1.0 - lod_factor * 0.1).max(0.01);
    let ramp = lod_factor * 2.0;
    if d < ramp && ramp > 0.0 {
        d = (d / ramp) * (d / ramp) * ramp;
    }
    d *= std::f32::consts::PI / 3.0;
    let lod_factor = lod_factor * (60f32.to_radians() / fov_y.max(0.1));
    let round = |v: f32| (v * 100.0).round() / 100.0;
    let (d, radius) = (round(d), round(radius));
    if d <= 0.0 || radius <= 0.0 {
        return 3;
    }
    let tan = round((lod_factor * radius) / d);
    if tan > 0.24 {
        3
    } else if tan > 0.06 {
        2
    } else if tan > 0.03 {
        1
    } else {
        0
    }
}

/// Prim material code of light sources (material_codes.h LL_MCODE_LIGHT).
const LL_MCODE_LIGHT: u8 = 7;

fn te_record_base(model: Mat4, f: &TextureFace) -> DrawRecord {
    let shiny = f.shiny();
    let roughness = match shiny {
        1 => 0.45,
        2 => 0.3,
        3 => 0.15,
        _ => 0.85,
    };
    let mut fl = 0;
    if f.fullbright() {
        fl |= flags::FULLBRIGHT;
    }
    if f.planar() {
        fl |= flags::PLANAR;
    }
    // a shiny face without material is lit like a legacy material whose
    // specular color, glossiness and environment are its shininess
    // (LLFace: vertex alpha = 0, .25, .5, .75)
    let s = [0.0f32, 0.25, 0.5, 0.75][shiny as usize & 3];
    if shiny > 0 {
        fl |= flags::LEGACY_MAT;
    }
    let sl = s.powf(2.2);
    DrawRecord {
        model: model.to_cols_array_2d(),
        base_color: f.color,
        emissive: [0.0, 0.0, 0.0, f.glow],
        uv_st: [f.scale_s, f.scale_t, f.offset_s, f.offset_t],
        params: [f.rotation, 0.0, roughness, 0.5],
        tex: [
            aurora_render::textures::WHITE,
            aurora_render::textures::FLAT_NORMAL,
            aurora_render::textures::WHITE,
            aurora_render::textures::WHITE,
        ],
        flags: [fl, 0, 0, 0],
        legacy: [0.0, 0.0, s, s],
        spec_color: [sl, sl, sl, 1.0],
        ..Default::default()
    }
}

impl Scene {
    pub fn new(cache_dir: std::path::PathBuf, avatar_lib: Arc<AvatarLibrary>) -> Scene {
        let (jobs, job_rx) = Jobs::new();
        let cache_dir_anim = cache_dir.clone();
        Scene {
            jobs,
            job_rx,
            geoms: HashMap::new(),
            textures: TextureStreamer::new(cache_dir.clone()),
            meshes: MeshStreamer::new(cache_dir.clone()),
            materials: MaterialStreamer::new(cache_dir),
            legacy_mats: Default::default(),
            avatar_lib,
            gpu: Vec::new(),
            regions: HashMap::new(),
            water: water::WaterGpu::default(),
            banlines: banlines::BanLines::default(),
            particles: particles::ParticleManager::default(),
            avatar_complexity: HashMap::new(),
            too_complex: Default::default(),
            selection_wire: HashMap::new(),
            debug_alpha: None,
            last_cull: None,
            impostors: Default::default(),
            loading: Default::default(),
            hide_loading: true,
            loading_bars: true,
            sculpt_area: HashMap::new(),
            avatar_area: HashMap::new(),
            complexity_rules: 0,
            complexity_at: None,
            complexity_max: 0,
            max_avatars: 16,
            reflections: true,
            fov_y: 1.0,
            env_tex: [(Uuid::nil(), 0); 4],
            agent_idx: None,
            lists: DrawLists::default(),
            stats: SceneStats::default(),
            frame: 0,
            lod_factor: 2.0,
            draw_distance: 96.0,
            last_origin: None,
            blend_tmp: Vec::new(),
            sounds: sounds::SoundManager::new(cache_dir_anim.clone()),
            anims: anim::AnimStreamer::new(cache_dir_anim),
            palette_slots: HashMap::new(),
            palette_free: Vec::new(),
            palette_count: 0,
            palettes: Vec::new(),
            last_palette_gc: Instant::now(),
            last_diag: Instant::now(),
            skeletons: HashMap::new(),
            motions: HashMap::new(),
            skin_binds: Vec::new(),
            skin_bind_ranges: HashMap::new(),
            skin_binds_dirty: false,
        }
    }

    // ------------------------------------------------------------ geometry

    fn geom_ready(&self, key: &GeomKey) -> Option<Arc<GpuGeom>> {
        match self.geoms.get(key).map(|e| &e.state) {
            Some(GeomState::Ready(g)) => Some(g.clone()),
            _ => None,
        }
    }

    fn geom_failed(&self, key: &GeomKey) -> bool {
        matches!(self.geoms.get(key).map(|e| &e.state), Some(GeomState::Failed))
    }

    fn geom_ref(&mut self, key: GeomKey) {
        if let Some(e) = self.geoms.get_mut(&key) {
            e.refs += 1;
            e.unused_frames = 0;
        }
    }

    fn geom_unref(&mut self, key: GeomKey) {
        if let Some(e) = self.geoms.get_mut(&key) {
            e.refs = e.refs.saturating_sub(1);
        }
    }

    /// Ensure a geometry is (being) built. Returns true when ready.
    fn request_geom(&mut self, key: GeomKey, obj: &Object) -> bool {
        if let Some(e) = self.geoms.get(&key) {
            return matches!(e.state, GeomState::Ready(_));
        }
        match key {
            GeomKey::Prim { lod, .. } => {
                let params = obj.volume;
                let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize % 4];
                self.jobs.spawn(move || {
                    let mesh = aurora_prim::generate_volume(&params, detail);
                    let (faces, min, max) = jobs::volume_to_faces(&mesh);
                    JobResult::Geometry {
                        key,
                        faces,
                        min,
                        max,
                        area: 1.0,
                    }
                });
            }
            GeomKey::Sculpt { lod, .. } => {
                let params = obj.volume;
                let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize % 4];
                let Some(tex) = params.sculpt.map(|s| s.texture) else {
                    return false;
                };
                let Some(map) = self.textures.sculpt_map(&tex) else {
                    return false; // wait for the sculpt texture
                };
                self.jobs.spawn(move || {
                    let mesh = aurora_prim::generate_sculpt(&params, detail, map.width, map.height, map.components, &map.pixels);
                    let (faces, min, max) = jobs::volume_to_faces(&mesh);
                    let area = mesh.surface_area;
                    JobResult::Geometry {
                        key,
                        faces,
                        min,
                        max,
                        area,
                    }
                });
            }
            GeomKey::Mesh { id, lod } => {
                self.meshes.want(id, lod);
            }
            GeomKey::AvatarPart(_) => {}
        }
        self.geoms.insert(
            key,
            GeomEntry {
                state: GeomState::Pending,
                refs: 0,
                unused_frames: 0,
            },
        );
        false
    }

    fn upload_geom(&mut self, renderer: &mut Renderer, key: GeomKey, faces: Vec<jobs::FaceData>, min: Vec3, max: Vec3) {
        let gfaces = faces
            .into_iter()
            .map(|f| {
                f.and_then(|g| match &g.skin {
                    Some(sk) => renderer.upload_skinned_mesh(&g.vertices, sk, &g.indices),
                    None => renderer.upload_mesh(&g.vertices, &g.indices),
                })
            })
            .collect();
        let g = Arc::new(GpuGeom { faces: gfaces, min, max });
        match self.geoms.get_mut(&key) {
            Some(e) => {
                if let GeomState::Ready(old) = std::mem::replace(&mut e.state, GeomState::Ready(g)) {
                    for f in old.faces.iter().flatten() {
                        renderer.free_mesh(*f);
                    }
                }
            }
            None => {
                self.geoms.insert(
                    key,
                    GeomEntry {
                        state: GeomState::Ready(g),
                        refs: 0,
                        unused_frames: 0,
                    },
                );
            }
        }
    }

    fn ensure_avatar_parts(&mut self, renderer: &mut Renderer) {
        let lib = self.avatar_lib.clone();
        for (i, part) in lib.parts.iter().enumerate() {
            let key = GeomKey::AvatarPart(i as u8);
            if self.geoms.contains_key(&key) {
                continue;
            }
            let mut min = Vec3::splat(f32::MAX);
            let mut max = Vec3::splat(f32::MIN);
            for v in &part.vertices {
                let p = Vec3::from_array(v.pos);
                min = min.min(p);
                max = max.max(p);
            }
            let skin = (part.skin.len() == part.vertices.len()).then(|| part.skin.clone());
            self.upload_geom(
                renderer,
                key,
                vec![Some(jobs::FaceGeom {
                    vertices: part.vertices.clone(),
                    indices: part.indices.clone(),
                    skin,
                })],
                min,
                max,
            );
            if let Some(e) = self.geoms.get_mut(&key) {
                e.refs = u32::MAX / 2; // permanent
            }
        }
    }

    // ------------------------------------------------------------ results

    pub fn process_results(&mut self, renderer: &mut Renderer, net: &NetClient, budget: std::time::Duration) {
        let t0 = Instant::now();
        while let Ok(r) = net.fetch_results.try_recv() {
            let kind = r.key & (0xF << 60);
            if kind == textures::FETCH_KIND_TEXTURE {
                self.textures.on_fetch(r);
            } else if kind == meshes::FETCH_KIND_MESH {
                self.meshes.on_fetch(r, &self.jobs);
            } else if kind == meshes::FETCH_KIND_MATERIAL {
                self.materials.on_fetch(r, &self.jobs);
            } else if kind == anim::FETCH_KIND_ANIM {
                let rig = self.avatar_lib.rig.clone();
                self.anims.on_fetch(r, &self.jobs, &rig);
            } else if kind == sounds::FETCH_KIND_SOUND {
                self.sounds.on_fetch(r, &self.jobs);
            }
        }
        while t0.elapsed() < budget {
            let Ok(r) = self.job_rx.try_recv() else {
                break;
            };
            match r {
                JobResult::Geometry {
                    key,
                    faces,
                    min,
                    max,
                    area,
                } => {
                    if let GeomKey::Mesh { id, lod } = key {
                        self.meshes.on_geometry_done(&id, lod);
                    }
                    if matches!(key, GeomKey::Sculpt { .. }) {
                        self.sculpt_area.insert(key, area);
                    }
                    self.upload_geom(renderer, key, faces, min, max);
                }
                JobResult::GeometryFailed { key } => {
                    if let GeomKey::Mesh { id, lod } = key {
                        self.meshes.on_geometry_done(&id, lod);
                    }
                    if let Some(e) = self.geoms.get_mut(&key) {
                        e.state = GeomState::Failed;
                    }
                }
                JobResult::MeshCache { id, data } => self.meshes.on_cache(id, data),
                JobResult::Material { id, material } => self.materials.on_material(id, material),
                JobResult::Done => {}
                other => self.textures.on_job(other),
            }
        }
    }

    // ------------------------------------------------------------ objects

    fn release_obj(&mut self, renderer: &mut Renderer, g: &mut ObjGpu) {
        for f in g.faces.drain(..) {
            renderer.records.free(f.record);
        }
        for t in g.tex_ids.drain(..) {
            self.textures.release(&t);
        }
        if let Some(k) = g.geom.take() {
            self.geom_unref(k);
        }
        g.wanted_geom = None;
        g.material_ids.clear();
    }

    /// Free GPU state for removed objects.
    pub fn collect_garbage(&mut self, renderer: &mut Renderer, store: &mut ObjectStore) {
        // Objects removed from the slab: their slot index is free now.
        let removed: Vec<Object> = std::mem::take(&mut store.graveyard);
        if removed.is_empty() {
            return;
        }
        for (i, slot) in store.slots.iter().enumerate() {
            if slot.is_none()
                && let Some(mut g) = self.gpu.get_mut(i).map(std::mem::take)
            {
                self.release_obj(renderer, &mut g);
            }
        }
    }

    /// Release everything (logout / teleport cleanup).
    pub fn clear(&mut self, renderer: &mut Renderer) {
        let mut gpus = std::mem::take(&mut self.gpu);
        for g in gpus.iter_mut() {
            self.release_obj(renderer, g);
        }
        for (_, r) in self.regions.drain() {
            for (_, c) in r.chunks {
                renderer.free_mesh(c.alloc);
                renderer.records.free(c.record);
            }
            for id in r.detail_ids {
                self.textures.release(&id);
            }
        }
        self.water.clear(renderer);
        self.banlines.clear(renderer);
        self.particles.clear(&mut self.textures);
        self.last_origin = None;
        self.palette_slots.clear();
        self.motions.clear();
        self.palette_free.clear();
        self.palette_count = 0;
    }

    /// (radius, distance) used for an object's LOD (LLVOVolume::calcLOD):
    /// rigged meshes use their skeleton owner's distance and size, other
    /// volumes the LOD-biased scale (LLVolume::mLODScaleBias).
    fn lod_metrics(&self, world: &World, idx: usize, pos: Vec3, eye: Vec3, now: Instant) -> (f32, f32) {
        let Some(o) = world.objects.get(idx) else {
            return (1.0, pos.distance(eye));
        };
        if o.volume.is_mesh() {
            let rigged = o
                .volume
                .sculpt
                .is_some_and(|s| self.meshes.meta(&s.texture).is_some_and(|m| m.skin.is_some()));
            if rigged
                && let Some((_, owner)) = Self::skeleton_owner(world, idx)
                && let (Some(ow), Some((op, _, _))) = (world.objects.get(owner), Self::object_transform(world, owner, now, 0))
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

    /// What is at a picked world point: an avatar (capsule test) or the most
    /// specific object whose oriented box contains the point.
    pub fn pick_at(&self, world: &World, point: Vec3, now: Instant) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (idx, g) in self.gpu.iter().enumerate() {
            if g.faces.is_empty() || g.hud {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            if g.is_avatar {
                let rel = point - g.center;
                if Vec3::new(rel.x, rel.y, 0.0).length() < 0.6 && (-1.25..=1.15).contains(&rel.z) {
                    return Some(idx);
                }
                continue;
            }
            if g.owner_avatar.is_some() && o.volume.is_mesh() {
                continue; // rigged / worn mesh: the avatar capsule handles it
            }
            let Some((pos, rot, _)) = Self::object_transform(world, idx, now, 0) else {
                continue;
            };
            let scale = o.scale.max(Vec3::splat(0.01));
            let local = rot.inverse() * (point - pos);
            let tol = Vec3::splat(0.06);
            if (local.abs() - (scale * 0.5 + tol)).max_element() <= 0.0 {
                let volume = scale.x * scale.y * scale.z;
                if best.is_none_or(|(v, _)| volume < v) {
                    best = Some((volume, idx));
                }
            }
        }
        best.map(|(_, i)| i)
    }

    /// Keep the sky / water textures resident; returns their slots (0 until loaded).
    pub fn env_textures(&mut self, renderer: &mut Renderer, ids: [Uuid; 4]) -> [u32; 4] {
        let mut out = [0u32; 4];
        for (i, id) in ids.iter().enumerate() {
            if self.env_tex[i].0 != *id {
                let old = self.env_tex[i].0;
                let slot = if id.is_nil() {
                    0
                } else {
                    self.textures.acquire(renderer, *id, TexSource::Asset)
                };
                if !old.is_nil() {
                    self.textures.release(&old);
                }
                self.env_tex[i] = (*id, slot);
            }
            if !id.is_nil() {
                self.textures.note_usage(id, 2048.0);
                if self.textures.is_loaded(id) {
                    out[i] = self.env_tex[i].1;
                }
            }
        }
        out
    }

    /// Nearest visible mirror (SL hero probe: a box reflection probe flagged
    /// as mirror, plane through its center along its local +Z, facing the camera).
    pub fn find_mirror(&self, world: &World, view: &CullView, now: Instant) -> Option<aurora_render::MirrorParams> {
        let mut best: Option<(f32, aurora_render::MirrorParams)> = None;
        for (idx, g) in self.gpu.iter().enumerate() {
            if !g.mirror {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some((pos, rot, hud)) = Self::object_transform(world, idx, now, 0) else {
                continue;
            };
            let d = pos.distance(view.eye);
            if hud || d > self.draw_distance || !view.sphere_visible(pos, (o.scale * 0.5).length()) {
                continue;
            }
            let normal = rot * Vec3::Z;
            if normal.dot(view.eye - pos) < 0.0 {
                continue;
            }
            if best.as_ref().is_some_and(|(bd, _)| *bd <= d) {
                continue;
            }
            let to_box = Mat4::from_scale_rotation_translation(o.scale.max(Vec3::splat(0.01)), rot, pos).inverse();
            best = Some((
                d,
                aurora_render::MirrorParams {
                    point: pos,
                    normal,
                    world_to_box: to_box,
                },
            ));
        }
        best.map(|(_, m)| m)
    }

    fn object_geom_key(&self, o: &Object, lod: u8) -> Option<GeomKey> {
        if o.is_avatar() || o.is_tree() || o.pcode != aurora_prim::params::LL_PCODE_VOLUME {
            return None;
        }
        if o.volume.is_mesh() {
            return o.volume.sculpt.map(|s| GeomKey::Mesh { id: s.texture, lod });
        }
        if o.volume.is_sculpt() {
            return Some(GeomKey::Sculpt {
                hash: o.volume.cache_key(),
                lod,
            });
        }
        Some(GeomKey::Prim {
            hash: o.volume.cache_key(),
            lod,
        })
    }

    /// World (render-space) transform of an object, following parents. For
    /// an avatar: its root (pelvis) position, shape offset included.
    pub fn object_transform(world: &World, idx: usize, now: Instant, depth: u32) -> Option<(Vec3, Quat, bool)> {
        let (p, r, hud) = Self::object_transform_raw(world, idx, now, depth)?;
        let o = world.objects.get(idx)?;
        let dz = if o.is_avatar() {
            world.avatar_root_dz.get(&o.full_id).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        Some((p + Vec3::Z * dz, r, hud))
    }

    fn object_transform_raw(world: &World, idx: usize, now: Instant, depth: u32) -> Option<(Vec3, Quat, bool)> {
        let o = world.objects.get(idx)?;
        let (p, mut r) = if o.full_id == world.agent_id && world.agent.has_local_control() {
            // The agent is already in main-region coordinates. Its object
            // can still belong to the previous/next region during an arrival.
            (
                world.agent.position - world.region_offset(o.key.region)?,
                world.agent.body_rotation(),
            )
        } else {
            o.predicted(now)
        };
        // a standing avatar is drawn turned toward where it goes
        if o.parent_id == 0
            && o.is_avatar()
            && let Some(b) = world.bodies.get(&o.full_id)
        {
            r = b.rotation;
        }
        if o.parent_id == 0 || depth > 16 {
            let off = world.region_offset(o.key.region)?;
            return Some((off + p, r, false));
        }
        let pidx = world.objects.parent_of(o)?;
        let parent = world.objects.get(pidx)?;
        let (pp, pr, phud) = Self::object_transform(world, pidx, now, depth + 1)?;
        if parent.is_avatar() && o.state != 0 {
            // attachment root: relative to the attachment point
            let point = o.attachment_point();
            let lib = &world.avatar_lib;
            let ap = lib.attach_points.get(&point).copied();
            let hud = ap.is_some_and(|a| a.hud) || (31..=38).contains(&point);
            let (mut apos, mut arot) = ap.map(|a| (a.position, a.rotation)).unwrap_or((Vec3::ZERO, Quat::IDENTITY));
            // follow the animated bone (LLViewerJointAttachment under its joint)
            if let Some((a, j)) = ap.and_then(|a| Some((a, a.joint?)))
                && let Some(m) = world.avatar_poses.get(&parent.full_id).and_then(|p| p.get(j))
            {
                let (_, jrot, _) = m.to_scale_rotation_translation();
                apos = m.transform_point3(a.skel_position) - lib.pelvis;
                arot = jrot.normalize() * arot;
            }
            let pos = pp + pr * (apos + arot * p);
            return Some((pos, pr * arot * r, hud));
        }
        Some((pp + pr * p, (pr * r).normalize(), phud))
    }

    /// Bring GPU records up to date for every object (incremental).
    pub fn sync(&mut self, renderer: &mut Renderer, world: &mut World, view: &CullView) {
        let t0 = Instant::now();
        self.frame = self.frame.wrapping_add(1);
        self.ensure_avatar_parts(renderer);
        self.collect_garbage(renderer, &mut world.objects);

        // Render origin changed: every transform is stale.
        let origin = world.main_origin();
        if origin != self.last_origin {
            self.last_origin = origin;
            for o in world.objects.slots.iter_mut().flatten() {
                o.render.needs_records = true;
            }
            for r in self.regions.values_mut() {
                for (_, c) in r.chunks.drain() {
                    renderer.free_mesh(c.alloc);
                    renderer.records.free(c.record);
                }
            }
            for reg in world.regions.values_mut() {
                reg.dirty_chunks.extend(0..reg.heightmap.chunk_count());
            }
            self.water.clear(renderer);
        }

        self.sync_terrain(renderer, world);
        self.water.sync(renderer, world);

        if self.gpu.len() < world.objects.slots.len() {
            self.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        }
        let now = Instant::now();
        let tex_gen = self.textures.generation;
        let mat_gen = self.materials.generation;
        let n = world.objects.slots.len();
        for idx in 0..n {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            // objects whose parent moves need new transforms too
            let moving = o.is_moving() || o.parent_id != 0 || o.is_avatar() || o.full_id == world.agent_id;
            let lod_due = (self.frame.wrapping_add(idx as u32)).is_multiple_of(24);
            let g = &self.gpu[idx];
            let sculpt_ready = g.sculpt_wait.is_some_and(|t| self.textures.sculpt_map(&t).is_some());
            let mats_changed = !g.material_ids.is_empty() && g.mat_generation != mat_gen;
            let pending = g.wanted_geom.is_some_and(|k| Some(k) != g.geom);
            if !(o.render.needs_records
                || o.shape_dirty
                || o.material_dirty
                || moving
                || lod_due
                || pending
                || sculpt_ready
                || mats_changed)
            {
                continue;
            }
            self.sync_object(renderer, world, idx, now, view);
            let _ = tex_gen;
        }

        // Garbage-collect unused geometry after a while.
        let mut drop_keys = Vec::new();
        for (k, e) in self.geoms.iter_mut() {
            if e.refs == 0 {
                e.unused_frames += 1;
                if e.unused_frames > 600 {
                    drop_keys.push(*k);
                }
            }
        }
        for k in drop_keys {
            if let Some(e) = self.geoms.remove(&k)
                && let GeomState::Ready(g) = e.state
            {
                for f in g.faces.iter().flatten() {
                    renderer.free_mesh(*f);
                }
            }
        }

        self.stats.objects = world.objects.len();
        self.stats.geometries = self.geoms.len();
        self.stats.geom_pending = self.geoms.values().filter(|e| matches!(e.state, GeomState::Pending)).count();
        self.stats.jobs = self.jobs.pending();
        self.stats.sync_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn sync_object(&mut self, renderer: &mut Renderer, world: &mut World, idx: usize, now: Instant, view: &CullView) {
        let Some((pos, rot, hud)) = Self::object_transform(world, idx, now, 0) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let is_avatar = o.is_avatar();
        let scale = o.scale.max(Vec3::splat(0.001));
        let radius = (scale * 0.5).length().max(0.05);

        // ---- avatar
        if is_avatar {
            self.sync_avatar(renderer, world, idx, pos, rot);
            if let Some(o) = world.objects.get_mut(idx) {
                o.render.needs_records = false;
                o.shape_dirty = false;
                o.material_dirty = false;
            }
            return;
        }

        // ---- geometry / LOD
        let (lod_radius, lod_dist) = self.lod_metrics(world, idx, pos, view.eye, now);
        let lod = if hud {
            3
        } else {
            lod_for(lod_radius, lod_dist, self.lod_factor, self.fov_y)
        };
        let wanted = self.object_geom_key(o, lod);
        let owner_avatar = Self::wearer_avatar(world, idx).map(|(_, i)| i);
        let g = &mut self.gpu[idx];
        g.hud = hud;
        g.owner_avatar = owner_avatar;
        // reflection probe with the box and mirror flags (LLReflectionProbeParams)
        g.mirror = o.extra.reflection_probe.is_some_and(|p| p.flags & 0x4 != 0 && p.flags & 0x1 != 0);
        let shape_dirty = o.shape_dirty;
        if wanted != g.wanted_geom || shape_dirty {
            g.wanted_geom = wanted;
        }
        let mut sculpt_wait = None;
        if let Some(GeomKey::Sculpt { .. }) = wanted
            && let Some(s) = o.volume.sculpt
        {
            sculpt_wait = Some(s.texture);
        }
        let o_clone_volume = o.volume;
        let _ = o_clone_volume;
        let mut bind_new = None;
        if let Some(k) = wanted {
            // need an Object reference for building: re-borrow
            let obj = world.objects.get(idx);
            let ready = match obj {
                Some(obj) => self.request_geom(k, obj),
                None => false,
            };
            if ready {
                if self.gpu[idx].geom != Some(k) {
                    bind_new = Some(k);
                }
            } else if self.geom_failed(&k) {
                // fall back to any other LOD already loaded
            }
        }
        if let Some(st) = sculpt_wait {
            // sculpt textures are fetched at full resolution and kept on the CPU
            if !self.gpu[idx].tex_ids.contains(&st) {
                let _ = self.textures.acquire(renderer, st, TexSource::Asset);
                self.textures.want_sculpt(&st);
                self.gpu[idx].tex_ids.push(st);
            }
            self.gpu[idx].sculpt_wait = Some(st);
        }
        if let Some(k) = bind_new {
            if let Some(old) = self.gpu[idx].geom.replace(k) {
                self.geom_unref(old);
            }
            self.geom_ref(k);
            self.gpu[idx].lod = lod;
            if self.gpu[idx].sculpt_wait.is_some() && matches!(k, GeomKey::Sculpt { .. }) {
                self.gpu[idx].sculpt_wait = None;
            }
        }

        // ---- records
        let Some(geom_key) = self.gpu[idx].geom else {
            // Not ready: keep bounds for culling anyway.
            let g = &mut self.gpu[idx];
            g.center = pos;
            g.radius = radius;
            return;
        };
        let Some(geom) = self.geom_ready(&geom_key) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let rigged = matches!(geom_key, GeomKey::Mesh { id, .. } if self.meshes.meta(&id).is_some_and(|m| m.skin.is_some()));
        let model = if rigged {
            // Rigged meshes are already in avatar skeleton space (bind pose):
            // place them relative to the avatar that wears them.
            let pelvis = world.avatar_lib.pelvis;
            match Self::skeleton_owner(world, idx) {
                Some((_, owner_idx)) => match Self::object_transform(world, owner_idx, now, 0) {
                    Some((ap, ar, _)) => Mat4::from_rotation_translation(ar, ap) * Mat4::from_translation(-pelvis),
                    None => Mat4::from_scale_rotation_translation(scale, rot, pos),
                },
                None => Mat4::from_scale_rotation_translation(scale, rot, pos),
            }
        } else {
            Mat4::from_scale_rotation_translation(scale, rot, pos)
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
        if rigged {
            // the bind-pose bounds can be anywhere (the bones bring the mesh
            // onto the skeleton): cull with the wearer's extent, as LL does
            if let Some((_, owner_idx)) = Self::skeleton_owner(world, idx)
                && let (Some(ow), Some((ap, _, _))) = (world.objects.get(owner_idx), Self::object_transform(world, owner_idx, now, 0))
            {
                center = ap;
                radius = if ow.is_avatar() { 2.5 } else { (ow.scale.length() * 0.75).max(1.0) };
            }
        }

        let full_rebuild = o.material_dirty
            || o.shape_dirty
            || self.gpu[idx].faces.len() != geom.faces.iter().filter(|f| f.is_some()).count()
            || bind_new.is_some()
            || (!self.gpu[idx].material_ids.is_empty() && self.gpu[idx].mat_generation != self.materials.generation);

        if full_rebuild {
            self.rebuild_faces(renderer, world, idx, &geom, model);
        } else {
            // transform-only update
            let g = &self.gpu[idx];
            for f in &g.faces {
                if let Some(rec) = renderer.records.get(f.record).copied() {
                    let mut r = rec;
                    r.model = model.to_cols_array_2d();
                    renderer.records.set(f.record, r);
                }
            }
        }
        let g = &mut self.gpu[idx];
        g.center = center;
        g.radius = radius;
        g.rigged = rigged;
        g.is_avatar = false;
        g.hud = hud;
        if let Some(o) = world.objects.get_mut(idx) {
            o.render.needs_records = false;
            o.shape_dirty = false;
            o.material_dirty = false;
        }
    }

    /// The object owning the skeleton that drives a rigged mesh: the wearing
    /// avatar, or the root of an animated-mesh (animesh) linkset.
    pub fn skeleton_owner(world: &World, mut idx: usize) -> Option<(Uuid, usize)> {
        for _ in 0..16 {
            let o = world.objects.get(idx)?;
            if o.is_avatar() {
                return Some((o.full_id, idx));
            }
            if o.parent_id == 0 {
                let animated = o
                    .extra
                    .extended_mesh_flags
                    .is_some_and(|f| f & aurora_prim::extra::EXTENDED_MESH_ANIMATED != 0);
                return animated.then_some((o.full_id, idx));
            }
            {
                let p = world.objects.parent_of(o)?;
                idx = p
            }
        }
        None
    }

    /// Palette base (first matrix index) for a skeleton owner.
    fn palette_base(&mut self, owner: Uuid) -> u32 {
        if let Some(s) = self.palette_slots.get(&owner) {
            return *s * anim::PALETTE_JOINTS as u32;
        }
        let slot = self.palette_free.pop().unwrap_or_else(|| {
            self.palette_count += 1;
            self.palette_count - 1
        });
        self.palette_slots.insert(owner, slot);
        slot * anim::PALETTE_JOINTS as u32
    }

    fn rebuild_faces(&mut self, renderer: &mut Renderer, world: &World, idx: usize, geom: &GpuGeom, model: Mat4) {
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        // release previous records/textures
        let mut g = std::mem::take(&mut self.gpu[idx]);
        for f in g.faces.drain(..) {
            renderer.records.free(f.record);
        }
        let old_tex: Vec<Uuid> = std::mem::take(&mut g.tex_ids);
        g.material_ids.clear();
        let te = o.te.clone();
        let wearer = Self::wearer_avatar(world, idx);
        let rigged_owner = match g.geom {
            Some(GeomKey::Mesh { id, .. }) => match self.meshes.meta(&id).and_then(|m| m.skin).filter(|s| !s.joint_names.is_empty()) {
                Some(skin) => Self::skeleton_owner(world, idx).map(|(owner, i)| (owner, i, self.mesh_binds(id, &skin))),
                None => None,
            },
            _ => None,
        };
        for (fi, face) in geom.faces.iter().enumerate() {
            let Some(alloc) = face else {
                continue;
            };
            let tf = te.as_ref().map(|t| *t.face(fi)).unwrap_or_default();
            if tf.color[3] < 0.004 {
                // fully transparent face: LL registers it as PASS_ALPHA_INVISIBLE,
                // never drawn, and its glow is not drawn either
                continue;
            }
            let mut rec = te_record_base(model, &tf);
            // LLVOVolume: prims of the "light" material are fullbright
            if o.prim_material == LL_MCODE_LIGHT {
                rec.flags[0] |= flags::FULLBRIGHT;
            }
            let mut pbr_alpha = None;
            let mut aux_tex = [Uuid::nil(); 3];
            let base_repeats = tf.scale_s.abs().max(tf.scale_t.abs()).clamp(0.1, 16.0);
            let mut aux_repeats = [base_repeats; 3];
            let mut two_sided = false;
            // GLTF material?
            let mat_id = o
                .extra
                .render_materials
                .iter()
                .find(|(te_idx, _)| *te_idx as usize == fi)
                .map(|(_, id)| *id)
                .filter(|id| !id.is_nil());
            let mut base_tex = tf.texture;
            if let Some(mid) = mat_id {
                g.material_ids.push(mid);
                if let Some(m) = self.materials.get(&mid) {
                    // glTF materials replace the legacy shininess
                    rec.flags[0] &= !flags::LEGACY_MAT;
                    rec.flags[0] |= flags::PBR;
                    rec.flags[0] &= !flags::FULLBRIGHT;
                    let c = m.base_color_factor;
                    rec.base_color = [c[0] * tf.color[0], c[1] * tf.color[1], c[2] * tf.color[2], c[3] * tf.color[3]];
                    rec.params[1] = m.metallic_factor;
                    rec.params[2] = m.roughness_factor;
                    rec.params[3] = m.alpha_cutoff;
                    rec.emissive = [m.emissive_factor[0], m.emissive_factor[1], m.emissive_factor[2], tf.glow];
                    let t = &m.transforms[0];
                    rec.uv_st = [t.scale[0], t.scale[1], t.offset[0], t.offset[1]];
                    rec.params[0] = t.rotation;
                    base_tex = m.base_color_texture.unwrap_or(Uuid::nil());
                    let mut slot_of = |id: Option<Uuid>, default: u32, g: &mut ObjGpu, s: &mut Self| -> u32 {
                        match id {
                            Some(id) if !id.is_nil() => {
                                g.tex_ids.push(id);
                                s.textures.acquire(renderer, id, TexSource::Asset)
                            }
                            _ => default,
                        }
                    };
                    aux_tex = [
                        m.normal_texture.unwrap_or_default(),
                        m.metallic_roughness_texture.unwrap_or_default(),
                        m.emissive_texture.unwrap_or_default(),
                    ];
                    rec.tex[1] = slot_of(m.normal_texture, aurora_render::textures::FLAT_NORMAL, &mut g, self);
                    rec.tex[2] = slot_of(m.metallic_roughness_texture, aurora_render::textures::WHITE, &mut g, self);
                    rec.tex[3] = slot_of(m.emissive_texture, aurora_render::textures::WHITE, &mut g, self);
                    pbr_alpha = Some(match m.alpha_mode {
                        aurora_assets::AlphaMode::Opaque => 0,
                        aurora_assets::AlphaMode::Blend => 1,
                        aurora_assets::AlphaMode::Mask => 2,
                    });
                    two_sided = m.double_sided;
                }
            }
            // legacy material (LLMaterial) of the texture entry
            let mut legacy_alpha = None;
            if mat_id.is_none() && !tf.material_id.is_nil() {
                g.material_ids.push(tf.material_id);
                let cap = world.regions.get(&o.key.region).and_then(|r| {
                    r.caps.get("RenderMaterials").map(|url| legacy_mat::RegionCap {
                        url,
                        limits: r.materials_limits,
                    })
                });
                if let Some(lm) = self.legacy_mats.get(&tf.material_id, cap) {
                    rec.flags[0] |= flags::LEGACY_MAT;
                    aux_tex[0] = lm.normal_map;
                    aux_tex[1] = lm.specular_map;
                    let rep = |st: [f32; 4]| st[0].abs().max(st[1].abs()).clamp(0.1, 64.0);
                    aux_repeats[0] = rep(lm.normal_st);
                    aux_repeats[1] = rep(lm.specular_st);
                    if !lm.normal_map.is_nil() {
                        g.tex_ids.push(lm.normal_map);
                        rec.tex[1] = self.textures.acquire(renderer, lm.normal_map, TexSource::Asset);
                    }
                    if !lm.specular_map.is_nil() {
                        g.tex_ids.push(lm.specular_map);
                        rec.tex[2] = self.textures.acquire(renderer, lm.specular_map, TexSource::Asset);
                    }
                    rec.mat_uv = lm.normal_st;
                    rec.spec_uv = lm.specular_st;
                    // without a specular map the material's specular color, exponent
                    // and environment are ignored: the face's shininess (none / low /
                    // medium / high) stands for all three (LLVolumeGeometryManager)
                    let lin = |c: f32| c.clamp(0.0, 1.0).powf(2.2);
                    let (spec_rgb, gloss, env) = if lm.specular_map.is_nil() {
                        let s = [0.0, 0.25, 0.5, 0.75][tf.shiny() as usize & 3];
                        ([s; 3], s, s)
                    } else {
                        let c = lm.specular_color;
                        (
                            [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0],
                            lm.specular_exp as f32 / 255.0,
                            lm.env_intensity as f32 / 255.0,
                        )
                    };
                    rec.legacy = [lm.normal_rot, lm.specular_rot, gloss, env];
                    rec.spec_color = [lin(spec_rgb[0]), lin(spec_rgb[1]), lin(spec_rgb[2]), 1.0];
                    rec.params[3] = lm.alpha_cutoff as f32 / 255.0;
                    if lm.diffuse_alpha_mode == legacy_mat::alpha_mode::EMISSIVE {
                        rec.flags[0] |= flags::EMISSIVE_MASK;
                    }
                    legacy_alpha = Some(lm.diffuse_alpha_mode);
                }
            }
            // texture (with bake-on-mesh redirection)
            let mut source = TexSource::Asset;
            if let Some((bake_te, bake)) = avatar::bake_slot_for(&base_tex) {
                match wearer.and_then(|(aid, a)| world.bake_texture(a, bake_te).map(|t| (aid, t))) {
                    Some((aid, tid)) => {
                        base_tex = tid;
                        source = world.bake_source(aid, bake, tid);
                    }
                    None => base_tex = Uuid::nil(),
                }
            }
            // media replaces the diffuse texture (LLViewerMediaTexture::switchTexture)
            if let Some(media_tex) = world.media.face_texture(&o.full_id, fi as u8, &base_tex) {
                base_tex = media_tex;
                source = TexSource::Asset;
            }
            if !base_tex.is_nil() {
                g.tex_ids.push(base_tex);
            }
            rec.tex[0] = self.textures.acquire(renderer, base_tex, source);
            if let Some((owner, _, binds)) = rigged_owner {
                rec.flags[0] |= flags::SKINNED;
                rec.flags[1] = self.palette_base(owner);
                rec.flags[2] = binds;
            }
            let record = renderer.records.alloc(rec);
            g.faces.push(FaceDraw {
                record,
                cmd: DrawCmd {
                    index_count: alloc.index_count,
                    first_index: alloc.index_offset,
                    base_vertex: alloc.vertex_offset as i32,
                    record,
                    bounds: [0.0; 4],
                },
                base_slot: rec.tex[0],
                tex_id: base_tex,
                aux_tex,
                aux_repeats,
                te_alpha: rec.base_color[3],
                pbr_alpha,
                legacy_alpha,
                two_sided,
                repeats: tf.scale_s.abs().max(tf.scale_t.abs()).clamp(0.1, 16.0),
                glow: rec.emissive[3] > 0.0,
            });
        }
        for t in old_tex {
            self.textures.release(&t);
        }
        g.tex_generation = self.textures.generation;
        g.mat_generation = self.materials.generation;
        self.gpu[idx] = g;
    }

    /// For an attachment, the wearing avatar (full id, object index).
    fn wearer_avatar(world: &World, mut idx: usize) -> Option<(Uuid, usize)> {
        for _ in 0..16 {
            let o = world.objects.get(idx)?;
            if o.is_avatar() {
                return Some((o.full_id, idx));
            }
            idx = world.objects.parent_of(o)?;
        }
        None
    }

    /// Bake slots (texture-entry index bits) shown by the avatar's attachments
    /// and their child prims.
    fn bom_mask(world: &World, avatar_idx: usize) -> u64 {
        let Some(av) = world.objects.get(avatar_idx) else {
            return 0;
        };
        let mut mask = 0u64;
        let mut scan = |i: usize| {
            if let Some(te) = world.objects.get(i).and_then(|o| o.te.as_ref()) {
                for f in &te.faces {
                    if let Some((slot, _)) = avatar::bake_slot_for(&f.texture) {
                        mask |= 1u64 << slot.min(63);
                    }
                }
            }
        };
        for &att in world.objects.children_of(&av.key) {
            scan(att);
            if let Some(a) = world.objects.get(att) {
                for &c in world.objects.children_of(&a.key) {
                    scan(c);
                }
            }
        }
        mask
    }

    fn sync_avatar(&mut self, renderer: &mut Renderer, world: &World, idx: usize, pos: Vec3, rot: Quat) {
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let lib = self.avatar_lib.clone();
        // parts are in skeleton space: put the pelvis at the avatar position
        let avatar_model = Mat4::from_rotation_translation(rot, pos) * Mat4::from_translation(-lib.pelvis);
        let palette = self.palette_base(o.full_id);
        let system_binds = self.system_binds();
        let appearance_gen = world.appearance_generation(&o.full_id);
        // bakes used by attachments (checked every 30 frames)
        let bom_mask = if !self.gpu[idx].is_avatar || (self.frame as usize + idx).is_multiple_of(30) {
            Self::bom_mask(world, idx)
        } else {
            self.gpu[idx].bom_mask
        };
        // over the complexity limit: grey silhouette of the system body
        // (Pipeline::generateImpostor: avatar geometry only, alpha masking
        // off, filled with the muted color LLColor4::grey4)
        let jelly = self.too_complex.contains(&idx);
        let g = &self.gpu[idx];
        let needs_full = !g.is_avatar
            || g.faces.len() != lib.parts.len()
            || g.tex_generation != appearance_gen
            || g.bom_mask != bom_mask
            || g.jelly != jelly;
        if needs_full {
            let mut g = std::mem::take(&mut self.gpu[idx]);
            for f in g.faces.drain(..) {
                renderer.records.free(f.record);
            }
            let old_tex: Vec<Uuid> = std::mem::take(&mut g.tex_ids);
            for (pi, part) in lib.parts.iter().enumerate() {
                let Some(geom) = self.geom_ready(&GeomKey::AvatarPart(pi as u8)) else {
                    continue;
                };
                let Some(Some(alloc)) = geom.faces.first().copied() else {
                    continue;
                };
                let tex = world.bake_texture(idx, part.bake_te);
                // Skirt only when a skirt bake exists; invisible bakes hide parts.
                let hidden = match tex {
                    Some(t) => t == avatar::IMG_INVISIBLE || (part.name == "skirt" && t == avatar::IMG_DEFAULT_AVATAR),
                    None => part.name == "skirt",
                };
                // a worn mesh shows this bake: hide the system part
                // (LLVOAvatar::updateMeshVisibility)
                let hidden = if jelly {
                    part.name == "skirt" && tex.is_none_or(|t| t == avatar::IMG_DEFAULT_AVATAR)
                } else {
                    hidden || bom_mask & (1u64 << part.bake_te.min(63)) != 0
                };
                let mut rec = DrawRecord {
                    model: avatar_model.to_cols_array_2d(),
                    params: [0.0, 0.0, 0.8, 0.5],
                    flags: [flags::SKINNED, palette, system_binds, 0],
                    ..Default::default()
                };
                let mut slot = aurora_render::textures::WHITE;
                if jelly {
                    rec.base_color = [0.3, 0.3, 0.3, 1.0];
                    rec.flags[0] |= flags::FULLBRIGHT;
                } else if let Some(t) = tex.filter(|t| *t != avatar::IMG_DEFAULT_AVATAR && !t.is_nil()) {
                    let src = avatar::bake_name(part.bake_te)
                        .map(|b| world.bake_source(o.full_id, b, t))
                        .unwrap_or(TexSource::Asset);
                    slot = self.textures.acquire(renderer, t, src);
                    g.tex_ids.push(t);
                } else {
                    rec.base_color = [0.62, 0.58, 0.66, 1.0];
                }
                rec.tex[0] = slot;
                let record = renderer.records.alloc(rec);
                g.faces.push(FaceDraw {
                    record,
                    cmd: DrawCmd {
                        index_count: alloc.index_count,
                        first_index: alloc.index_offset,
                        base_vertex: alloc.vertex_offset as i32,
                        record,
                        bounds: [0.0; 4],
                    },
                    base_slot: slot,
                    tex_id: tex.unwrap_or(Uuid::nil()),
                    aux_tex: [Uuid::nil(); 3],
                    aux_repeats: [1.0; 3],
                    te_alpha: if hidden { 0.0 } else { 1.0 },
                    pbr_alpha: None,
                    legacy_alpha: None,
                    two_sided: false,
                    repeats: 1.0,
                    glow: false,
                });
            }
            for t in old_tex {
                self.textures.release(&t);
            }
            g.is_avatar = true;
            g.tex_generation = appearance_gen;
            g.bom_mask = bom_mask;
            g.jelly = jelly;
            self.gpu[idx] = g;
        } else {
            for f in &self.gpu[idx].faces {
                if let Some(mut r) = renderer.records.get(f.record).copied() {
                    r.model = avatar_model.to_cols_array_2d();
                    renderer.records.set(f.record, r);
                }
            }
        }
        let g = &mut self.gpu[idx];
        g.center = pos;
        g.radius = 1.2;
    }

    // ------------------------------------------------------------ terrain & water

    fn sync_terrain(&mut self, renderer: &mut Renderer, world: &mut World) {
        let handles: Vec<u64> = world.regions.keys().copied().collect();
        // drop GPU data of vanished regions
        let stale: Vec<u64> = self.regions.keys().filter(|h| !world.regions.contains_key(h)).copied().collect();
        for h in stale {
            if let Some(r) = self.regions.remove(&h) {
                for (_, c) in r.chunks {
                    renderer.free_mesh(c.alloc);
                    renderer.records.free(c.record);
                }
                for id in r.detail_ids {
                    self.textures.release(&id);
                }
            }
        }
        let mut built = 0;
        for h in handles {
            let Some(offset) = world.region_offset(h) else {
                continue;
            };
            let Some(reg) = world.regions.get_mut(&h) else {
                continue;
            };
            let Some(info) = reg.info.clone() else {
                continue;
            };
            let rg = self.regions.entry(h).or_insert_with(|| RegionGpu {
                chunks: HashMap::new(),
                detail_slots: [aurora_render::textures::WHITE; 4],
                detail_ids: [Uuid::nil(); 4],
            });
            if rg.detail_ids != info.terrain_detail {
                for (i, id) in info.terrain_detail.iter().enumerate() {
                    let old = rg.detail_ids[i];
                    rg.detail_slots[i] = self.textures.acquire(renderer, *id, TexSource::Asset);
                    self.textures.release(&old);
                    rg.detail_ids[i] = *id;
                }
                reg.dirty_chunks.extend(0..reg.heightmap.chunk_count());
            }
            if reg.dirty_chunks.is_empty() || built > 6 {
                continue;
            }
            let comp = Composition {
                start_height: info.terrain_start_height,
                height_range: info.terrain_height_range,
                origin_x: (h >> 32) as f64,
                origin_y: (h & 0xFFFF_FFFF) as f64,
            };
            let chunks: Vec<u32> = reg.dirty_chunks.iter().copied().take(8).collect();
            for c in chunks {
                reg.dirty_chunks.remove(&c);
                let (v, i) = terrain::build_chunk(&reg.heightmap, &comp, c);
                let Some(alloc) = renderer.upload_mesh(&v, &i) else {
                    continue;
                };
                let (mut zmin, mut zmax) = (f32::MAX, f32::MIN);
                for vv in &v {
                    zmin = zmin.min(vv.pos[2]);
                    zmax = zmax.max(vv.pos[2]);
                }
                let cx_n = reg.heightmap.size_x / CHUNK_CELLS;
                let x0 = (c % cx_n * CHUNK_CELLS) as f32;
                let y0 = (c / cx_n * CHUNK_CELLS) as f32;
                let half = CHUNK_CELLS as f32 * 0.5;
                let center = offset + Vec3::new(x0 + half, y0 + half, (zmin + zmax) * 0.5);
                let radius = Vec3::new(half, half, (zmax - zmin) * 0.5).length();
                let rec = DrawRecord {
                    model: Mat4::from_translation(offset).to_cols_array_2d(),
                    uv_st: [12.0, 12.0, 0.0, 0.0],
                    tex: rg.detail_slots,
                    ..Default::default()
                };
                let record = renderer.records.alloc(rec);
                if let Some(old) = rg.chunks.insert(
                    c,
                    TerrainChunk {
                        alloc,
                        record,
                        center,
                        radius,
                    },
                ) {
                    renderer.free_mesh(old.alloc);
                    renderer.records.free(old.record);
                }
                built += 1;
            }
            // keep detail texture slots current (placeholders become real textures in place)
            let _ = rg.detail_slots;
        }
    }

    // ------------------------------------------------------------ culling

    /// Build this frame's draw lists.
    pub fn build_lists(&mut self, view: &CullView, shadows: bool) {
        let t0 = Instant::now();
        self.lists.clear();
        self.last_cull = Some(*view);
        let debug_alpha = self.debug_alpha;
        let dd = self.draw_distance;
        let shadow_dist = dd.min(256.0);
        // terrain
        for r in self.regions.values() {
            for c in r.chunks.values() {
                if (c.center - view.eye).length() - c.radius > dd * 1.5 {
                    continue;
                }
                let cmd = DrawCmd {
                    index_count: c.alloc.index_count,
                    first_index: c.alloc.index_offset,
                    base_vertex: c.alloc.vertex_offset as i32,
                    record: c.record,
                    bounds: [0.0; 4],
                };
                if shadows {
                    self.lists.shadow_casters.push(ShadowCaster {
                        cmd,
                        center: c.center,
                        radius: c.radius,
                    });
                }
                if self.reflections {
                    self.lists.reflection_terrain.push(cmd);
                }
                if view.sphere_visible(c.center, c.radius) {
                    self.lists.terrain.push(cmd);
                }
            }
        }
        self.lists.water.extend(self.water.draws());

        // nearest avatars in full (own avatar always), the others as
        // impostors (RenderAvatarMaxNonImpostors)
        let mut avatars: Vec<(f32, usize)> = self
            .gpu
            .iter()
            .enumerate()
            .filter(|(_, g)| g.is_avatar)
            .map(|(i, g)| {
                (
                    if Some(i) == self.agent_idx {
                        -1.0
                    } else {
                        g.center.distance(view.eye)
                    },
                    i,
                )
            })
            .collect();
        avatars.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut avatar_ok = vec![false; self.gpu.len()];
        for (_, i) in avatars.iter().take(self.max_avatars.max(1)) {
            avatar_ok[*i] = true;
        }
        let imp_avatars: Vec<(usize, Vec3, f32, bool)> = avatars
            .iter()
            .skip(self.max_avatars.max(1))
            .filter_map(|&(d, i)| {
                let g = self.gpu.get(i)?;
                (d < dd && !g.faces.is_empty()).then(|| (i, g.center, d, view.sphere_visible(g.center, 1.6)))
            })
            .collect();
        let mut plan = self.impostors.plan(&imp_avatars, view.eye, Instant::now());
        let hidden_loading = if self.hide_loading { Some(&self.loading.indices) } else { None };
        self.stats.avatars_hidden = imp_avatars.len();
        let reflections = self.reflections;
        let refl_dist = dd.min(128.0);

        // objects in parallel
        let alpha = &self.textures.alpha_by_slot;
        let alpha_channel = &self.textures.alpha_channel_by_slot;
        let selection_wire = &self.selection_wire;
        struct Local {
            opaque: Vec<DrawCmd>,
            opaque2: Vec<DrawCmd>,
            mask: Vec<DrawCmd>,
            mask2: Vec<DrawCmd>,
            blend: Vec<(f32, DrawCmd, bool)>,
            glow: Vec<DrawCmd>,
            glow_alpha: Vec<DrawCmd>,
            /// Draws of the avatars pictured for their impostor: (capture slot, draw, blended).
            impostor: Vec<(usize, DrawCmd, bool)>,
            debug_red: Vec<DrawCmd>,
            select_root: Vec<DrawCmd>,
            select_child: Vec<DrawCmd>,
            debug_blue: Vec<DrawCmd>,
            casters: Vec<ShadowCaster>,
            refl: Vec<ShadowCaster>,
            probe: Vec<ShadowCaster>,
            visible: usize,
            usage: Vec<(Uuid, f32)>,
        }
        let new_local = || Local {
            opaque: Vec::new(),
            opaque2: Vec::new(),
            mask: Vec::new(),
            mask2: Vec::new(),
            blend: Vec::new(),
            glow: Vec::new(),
            glow_alpha: Vec::new(),
            impostor: Vec::new(),
            debug_red: Vec::new(),
            select_root: Vec::new(),
            select_child: Vec::new(),
            debug_blue: Vec::new(),
            casters: Vec::new(),
            refl: Vec::new(),
            probe: Vec::new(),
            visible: 0,
            usage: Vec::new(),
        };
        let note_usage = self.frame.is_multiple_of(30);
        let result = self
            .gpu
            .par_iter()
            .enumerate()
            .fold(new_local, |mut l, (idx, g)| {
                if g.faces.is_empty() || g.hud {
                    return l;
                }
                // avatar limit: far avatars and everything they wear are drawn
                // as impostors; pictured ones give their draws to the picture
                let avatar = if g.is_avatar { Some(idx) } else { g.owner_avatar };
                if let Some(a) = avatar.filter(|a| !avatar_ok.get(*a).copied().unwrap_or(true)) {
                    if let Some(&slot) = plan.capture.get(&a) {
                        for f in &g.faces {
                            let tex_alpha = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                            match classify(f, tex_alpha) {
                                Pass::Hidden => {}
                                Pass::Blend => l.impostor.push((slot, f.cmd, true)),
                                _ => l.impostor.push((slot, f.cmd, false)),
                            }
                        }
                    }
                    return l;
                }
                // too complex: only its silhouette, no attachments
                if g.owner_avatar.is_some_and(|a| self.too_complex.contains(&a)) {
                    return l;
                }
                // still loading: a cloud until complete (not half an avatar)
                if avatar.is_some_and(|a| hidden_loading.is_some_and(|h| h.contains(&a))) {
                    return l;
                }
                let d = (g.center - view.eye).length();
                if d - g.radius > dd && !g.is_avatar {
                    return l;
                }
                let in_view = view.sphere_visible(g.center, g.radius);
                let casts = shadows && d - g.radius < shadow_dist;
                let reflects = reflections && d - g.radius < refl_dist;
                if !in_view && !casts && !reflects {
                    return l;
                }
                // tiny objects far away are culled (LL-like small object culling)
                let px = view.pixel_size(g.center, g.radius);
                if px < 1.5 && !g.is_avatar {
                    return l;
                }
                if in_view {
                    l.visible += 1;
                }
                for f in &g.faces {
                    // whole-object sphere for the GPU occlusion test (avatars: their
                    // whole animated extent)
                    let cmd = f.cmd.with_bounds(g.center, if g.is_avatar { g.radius.max(2.5) } else { g.radius });
                    let tex_alpha = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                    let pass = classify(f, tex_alpha);
                    // blended faces glow in their sorted place; hidden faces do not glow
                    if f.glow && in_view && pass != Pass::Blend && pass != Pass::Hidden {
                        // LLPipeline::getPoolTypeFromTE + canRenderAsMask: a glowing face
                        // whose texture has an alpha channel goes to the alpha pool unless
                        // its material says none / mask / emissive
                        let channel = alpha_channel.get(f.base_slot as usize).copied().unwrap_or(false);
                        let pool = f.pbr_alpha.is_none() && channel && matches!(f.legacy_alpha, None | Some(legacy_mat::alpha_mode::BLEND));
                        if pool {
                            l.glow_alpha.push(cmd);
                        } else {
                            l.glow.push(cmd);
                        }
                    }
                    // debug « Afficher la transparence » (renderDebugAlpha): blended,
                    // masked and invisible faces in red, material masks in blue;
                    // rigged faces only when asked (Firestorm sShowDebugAlphaRigged)
                    if let Some(rigged_too) = debug_alpha.filter(|_| in_view)
                        && (rigged_too || !(g.rigged || g.is_avatar))
                    {
                        let material = f.pbr_alpha.is_some() || f.legacy_alpha.is_some();
                        match pass {
                            Pass::Blend | Pass::Hidden => l.debug_red.push(cmd),
                            Pass::Mask | Pass::MaskTwoSided if material => l.debug_blue.push(cmd),
                            Pass::Mask | Pass::MaskTwoSided => l.debug_red.push(cmd),
                            _ => {}
                        }
                    }
                    if in_view {
                        match selection_wire.get(&idx) {
                            Some(true) => l.select_root.push(cmd),
                            Some(false) => l.select_child.push(cmd),
                            None => {}
                        }
                    }
                    if pass == Pass::Hidden {
                        continue;
                    }
                    if casts && matches!(pass, Pass::Opaque | Pass::OpaqueTwoSided | Pass::Mask | Pass::MaskTwoSided) {
                        l.casters.push(ShadowCaster {
                            cmd,
                            center: g.center,
                            radius: g.radius,
                        });
                    }
                    if reflects && matches!(pass, Pass::Opaque | Pass::OpaqueTwoSided | Pass::Mask | Pass::MaskTwoSided) {
                        let c = ShadowCaster {
                            cmd,
                            center: g.center,
                            radius: g.radius,
                        };
                        l.refl.push(c);
                        if avatar.is_none() {
                            l.probe.push(c);
                        }
                    }
                    if !in_view {
                        continue;
                    }
                    if note_usage {
                        l.usage.push((f.tex_id, px * f.repeats));
                        for (t, r) in f.aux_tex.iter().zip(f.aux_repeats).filter(|(t, _)| !t.is_nil()) {
                            l.usage.push((*t, px * r));
                        }
                    }
                    match pass {
                        Pass::Opaque => l.opaque.push(cmd),
                        Pass::OpaqueTwoSided => l.opaque2.push(cmd),
                        Pass::Mask => l.mask.push(cmd),
                        Pass::MaskTwoSided => l.mask2.push(cmd),
                        Pass::Blend => l.blend.push((d, cmd, f.glow)),
                        Pass::Hidden => {}
                    }
                }
                l
            })
            .reduce(new_local, |mut a, mut b| {
                a.opaque.append(&mut b.opaque);
                a.opaque2.append(&mut b.opaque2);
                a.mask.append(&mut b.mask);
                a.mask2.append(&mut b.mask2);
                a.blend.append(&mut b.blend);
                a.glow.append(&mut b.glow);
                a.glow_alpha.append(&mut b.glow_alpha);
                a.impostor.append(&mut b.impostor);
                a.debug_red.append(&mut b.debug_red);
                a.debug_blue.append(&mut b.debug_blue);
                a.select_root.append(&mut b.select_root);
                a.select_child.append(&mut b.select_child);
                a.casters.append(&mut b.casters);
                a.refl.append(&mut b.refl);
                a.probe.append(&mut b.probe);
                a.visible += b.visible;
                a.usage.append(&mut b.usage);
                a
            });
        self.lists.opaque = result.opaque;
        self.lists.opaque_two_sided = result.opaque2;
        self.lists.mask = result.mask;
        self.lists.mask_two_sided = result.mask2;
        self.blend_tmp = result.blend;
        self.blend_tmp.extend(self.banlines.cmds.iter().map(|(d, c)| (*d, *c, false)));
        self.blend_tmp.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        self.lists.blend.extend(self.blend_tmp.iter().map(|(_, c, _)| *c));
        self.lists.blend_glow.extend(self.blend_tmp.iter().map(|(_, _, g)| *g));
        self.lists.shadow_casters.extend(result.casters);
        self.lists.reflection = result.refl;
        self.lists.probe = result.probe;
        self.lists.glow = result.glow;
        self.lists.glow_alpha = result.glow_alpha;
        for (slot, cmd, blend) in result.impostor {
            if let Some(c) = plan.captures.get_mut(slot) {
                impostors::add_draw(c, cmd, blend);
            }
        }
        self.lists.impostor_captures = std::mem::take(&mut plan.captures);
        self.lists.impostor_sprites = std::mem::take(&mut plan.sprites);
        self.lists.debug_red = result.debug_red;
        self.lists.debug_blue = result.debug_blue;
        self.lists.select_root = result.select_root;
        self.lists.select_child = result.select_child;
        self.stats.visible_objects = result.visible;

        if note_usage {
            for (id, px) in result.usage {
                self.textures.note_usage(&id, px);
            }
            // terrain detail textures: always wanted at decent resolution
            for r in self.regions.values() {
                for id in &r.detail_ids {
                    self.textures.note_usage(id, 512.0);
                }
            }
            self.textures.recompute_wants();
        }
        self.stats.cull_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    /// Per-frame streaming work.
    pub fn stream(&mut self, renderer: &mut Renderer, net: &NetClient, world: &World, texture_budget: u64) {
        let va = world.viewer_asset_url();
        let va = va.as_deref();
        self.textures.update(&self.jobs, &net.fetcher, va);
        let rig = self.avatar_lib.rig.clone();
        self.meshes.update(&self.jobs, &net.fetcher, va, &rig);
        self.anims.update(&self.jobs, &net.fetcher, va, &rig);
        self.sounds.fetch(&self.jobs, &net.fetcher, va);
        self.materials.update(&self.jobs, &net.fetcher, va);
        if self.legacy_mats.update(net.runtime(), &net.caps_http()) {
            // objects using materials are rebuilt on a new generation
            self.materials.generation += 1;
        }
        if self.skin_binds_dirty {
            self.skin_binds_dirty = false;
            renderer.set_skin_binds(&self.skin_binds);
        }
        self.textures.upload(renderer, 24 * 1024 * 1024);
        self.textures.maintain(renderer, &self.jobs, texture_budget);
        if self.last_diag.elapsed() > std::time::Duration::from_secs(30) {
            self.last_diag = std::time::Instant::now();
            let (ready, fetching, failed) = self.meshes.counts();
            let t = &self.textures.stats;
            let (lm_ready, lm_wait, lm_unknown) = self.legacy_mats.counts();
            log::info!(
                "streaming: {} objects, meshes {ready} ready / {fetching} downloading / {failed} failed, textures {}/{} ({} downloading), legacy materials {lm_ready} ready / {lm_wait} pending / {lm_unknown} unknown, viewer asset cap {}",
                world.objects.len(),
                t.loaded,
                t.total,
                t.fetching,
                if va.is_some() { "ok" } else { "MISSING" }
            );
        }
    }
}

impl Scene {
    /// Diagnostic after a teleport: what our avatar is drawn with (object,
    /// palette, animations resolved).
    pub fn own_avatar_diag(&mut self, world: &World, eye: Vec3) -> String {
        let me = world.agent_id;
        let Some(idx) = world.objects.index_of_uuid(&me) else {
            return "no avatar object".into();
        };
        let obj = world
            .objects
            .get(idx)
            .map(|o| format!("local id {}, position {:.1?}, parent {}", o.key.local_id, o.position, o.parent_id))
            .unwrap_or_default();
        let g = self.gpu.get(idx);
        let center = g.map(|g| g.center).unwrap_or(Vec3::ZERO);
        let near = center.distance(eye) < self.draw_distance + 32.0;
        let anims = world.animations_of(&me).to_vec();
        let mut missing = Vec::new();
        for (id, _, _) in &anims {
            if self.anims.get(id).is_none() {
                missing.push(id.to_string());
            }
        }
        format!(
            "{obj}; drawn center {center:.1?} (avatar {}, {} faces), eye {eye:.1?}, posed {near}, palette {:?}, animations {} ({} not loaded {:?})",
            g.is_some_and(|g| g.is_avatar),
            g.map_or(0, |g| g.faces.len()),
            self.palette_slots.get(&me),
            anims.len(),
            missing.len(),
            missing
        )
    }

    /// Evaluate skeleton poses of every palette owner near the camera and
    /// upload the joint palettes.
    pub fn update_poses(&mut self, renderer: &mut Renderer, world: &mut World, eye: Vec3, now: Instant) {
        // free palettes of owners that disappeared
        if self.last_palette_gc.elapsed().as_secs_f32() > 2.0 {
            self.last_palette_gc = now;
            let gone: Vec<Uuid> = self
                .palette_slots
                .keys()
                .filter(|id| world.objects.index_of_uuid(id).is_none())
                .copied()
                .collect();
            for id in gone {
                if let Some(slot) = self.palette_slots.remove(&id) {
                    self.palette_free.push(slot);
                }
            }
        }
        let n = self.palette_count as usize * anim::PALETTE_JOINTS;
        if n == 0 {
            world.avatar_poses.clear();
            return;
        }
        if self.palettes.len() != n {
            self.palettes.resize(n, Mat4::IDENTITY.to_cols_array_2d());
        }
        let rig = self.avatar_lib.rig.clone();
        // gather work: (slot, anims)
        #[allow(clippy::type_complexity)]
        let mut work: Vec<(usize, anim::Controller, Arc<anim::SkeletonBase>, Uuid)> = Vec::new();
        let mut root_dz: Vec<(Uuid, f32)> = Vec::new();
        let mut posed: Vec<(Uuid, usize)> = Vec::new();
        let owners: Vec<(Uuid, u32)> = self.palette_slots.iter().map(|(k, v)| (*k, *v)).collect();
        for (owner, slot) in owners {
            let Some(idx) = world.objects.index_of_uuid(&owner) else {
                continue;
            };
            let near = self
                .gpu
                .get(idx)
                .map(|g| g.center.distance(eye) < self.draw_distance + 32.0)
                .unwrap_or(true);
            if !near {
                continue;
            }
            // where it looks (LLHUDEffectLookAt::update / calcTargetPosition)
            let target = if owner == world.agent_id {
                Some(world.look_at.own.target)
            } else {
                world.look_at.remote.get(&owner).copied()
            };
            let look = match target {
                None => Some((false, None)),
                Some(t) if t.kind == crate::world::lookat::NONE => Some((true, None)),
                Some(t) => match (self.look_target_pos(world, owner, &t, now), self.head_world(world, &owner, now)) {
                    (Some(tp), Some(head)) => {
                        let rot = Self::object_transform(world, idx, now, 0).map(|x| x.1).unwrap_or(Quat::IDENTITY);
                        let d = rot.inverse() * (tp - head);
                        d.is_finite().then_some((false, Some(d)))
                    }
                    _ => None,
                },
            };
            let mut motions = self.motions.remove(&owner).unwrap_or_default();
            match look {
                Some((true, _)) => motions.look_cleared = true,
                Some((false, None)) => {
                    motions.look_cleared = false;
                    motions.look = None;
                }
                Some((false, Some(d))) => {
                    motions.look_cleared = false;
                    motions.look = Some(d);
                }
                None => {}
            }
            let anims = &mut self.anims;
            motions.sync(world.animations_of(&owner), now, |id| anims.get(id));
            let (base, dz) = self.skeleton_of(world, owner, idx, now);
            root_dz.push((owner, dz));
            work.push((slot as usize, motions, base, owner));
            posed.push((owner, slot as usize));
        }
        let pal = &mut self.palettes;
        let mut chunks: Vec<Option<&mut [[[f32; 4]; 4]]>> = pal.chunks_mut(anim::PALETTE_JOINTS).map(Some).collect();
        let jobs: Vec<(&mut [[[f32; 4]; 4]], &mut (usize, anim::Controller, Arc<anim::SkeletonBase>, Uuid))> = work
            .iter_mut()
            .filter_map(|w| chunks.get_mut(w.0).and_then(|c| c.take()).map(|c| (c, w)))
            .collect();
        jobs.into_par_iter().for_each(|(chunk, (_, motions, base, _))| {
            for m in chunk.iter_mut().skip(rig.len()) {
                *m = Mat4::IDENTITY.to_cols_array_2d();
            }
            motions.evaluate(&rig, now, Some(base), chunk);
        });
        for (_, motions, _, owner) in work {
            self.motions.insert(owner, motions);
        }
        let slots = &self.palette_slots;
        self.motions.retain(|id, _| slots.contains_key(id));
        // joint matrices for the attachments (rest -> posed, skeleton space)
        let joints = rig.len().min(anim::PALETTE_JOINTS);
        world.avatar_poses.retain(|id, _| posed.iter().any(|p| p.0 == *id));
        world.avatar_root_dz.clear();
        world.avatar_root_dz.extend(root_dz);
        self.skeletons
            .retain(|id, _| world.avatar_poses.contains_key(id) || posed.iter().any(|p| p.0 == *id));
        for (owner, slot) in posed {
            let base = slot * anim::PALETTE_JOINTS;
            let Some(src) = self.palettes.get(base..base + joints) else {
                continue;
            };
            let dst = world.avatar_poses.entry(owner).or_default();
            dst.clear();
            // rest -> posed (attachment points are given in the default pose)
            dst.extend(
                src.iter()
                    .zip(&rig.default_world_inv)
                    .map(|(m, inv)| Mat4::from_cols_array_2d(m) * *inv),
            );
        }
        renderer.set_palettes(&self.palettes);
    }
}

impl Scene {
    /// World position of an avatar's head as posed last frame.
    pub fn head_world(&self, world: &World, id: &Uuid, now: Instant) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(id)?;
        let (p, r, _) = Self::object_transform(world, idx, now, 0)?;
        Some(match self.motions.get(id).and_then(|c| c.head_pos()) {
            Some(h) => p + r * (h - world.avatar_lib.pelvis),
            None => p + r * Vec3::new(0.0, 0.0, 0.65),
        })
    }

    /// Where a look-at target is (LLHUDEffectLookAt::calcTargetPosition).
    fn look_target_pos(&self, world: &World, source: Uuid, t: &crate::world::lookat::Target, now: Instant) -> Option<Vec3> {
        use crate::world::lookat::{FREELOOK, MOUSELOOK};
        let off = t.offset.as_vec3();
        match t.object {
            Some(obj) => {
                let idx = world.objects.index_of_uuid(&obj)?;
                let o = world.objects.get(idx)?;
                let (p, r, hud) = Self::object_transform(world, idx, now, 0)?;
                if o.is_avatar() {
                    let off = if obj == source && off.length_squared() < 1e-4 {
                        Vec3::X
                    } else {
                        off
                    };
                    // MOUSELOOK / FREELOOK: a world-space direction
                    let rot = if matches!(t.kind, MOUSELOOK | FREELOOK) {
                        Quat::IDENTITY
                    } else {
                        r
                    };
                    Some(self.head_world(world, &obj, now)? + rot * off)
                } else {
                    (!hud).then(|| p + r * off)
                }
            }
            None => {
                let (mx, my) = world.main_origin()?;
                Some(Vec3::new(
                    (t.offset.x - mx as f64) as f32,
                    (t.offset.y - my as f64) as f32,
                    t.offset.z as f32,
                ))
            }
        }
    }

    /// Skin bindings of the system avatar mesh: index = rig joint (plus the
    /// identity slot), inverse bind = inverse default joint matrix.
    fn system_binds(&mut self) -> u32 {
        if self.skin_binds.is_empty() {
            let rig = &self.avatar_lib.rig;
            let n = rig.len();
            for j in 0..n {
                self.skin_binds.push(aurora_render::SkinBind {
                    inverse_bind: rig.default_world_inv[j].to_cols_array_2d(),
                    joint: [j as u32, 0, 0, 0],
                });
            }
            self.skin_binds.push(aurora_render::SkinBind {
                inverse_bind: Mat4::IDENTITY.to_cols_array_2d(),
                joint: [n as u32, 0, 0, 0],
            });
            self.skin_binds_dirty = true;
        }
        0
    }

    /// Skin bindings of a rigged mesh: its inverse bind matrices and the rig
    /// joints its joint names refer to. Unknown joints keep the inverse bind
    /// alone (identity joint), as LLSkinningUtil does.
    fn mesh_binds(&mut self, mesh: Uuid, skin: &aurora_assets::SkinInfo) -> u32 {
        self.system_binds();
        if let Some(b) = self.skin_bind_ranges.get(&mesh) {
            return *b;
        }
        let base = self.skin_binds.len() as u32;
        let rig = &self.avatar_lib.rig;
        let identity = rig.len() as u32;
        for (name, ib) in skin.joint_names.iter().zip(&skin.inverse_bind) {
            self.skin_binds.push(aurora_render::SkinBind {
                inverse_bind: ib.to_cols_array_2d(),
                joint: [rig.names.get(name).map(|&j| j as u32).unwrap_or(identity), 0, 0, 0],
            });
        }
        self.skin_bind_ranges.insert(mesh, base);
        self.skin_binds_dirty = true;
        base
    }

    /// Rest skeleton of an avatar (or animesh) and the vertical offset of
    /// its pelvis from the object position:
    /// - the shape (visual parameters of its last appearance),
    /// - joint positions imposed by the rigged meshes it wears
    ///   (LLVOAvatar::addAttachmentOverridesForObject: the translation of each
    ///   alternate inverse bind matrix is the joint's local position; with
    ///   lock_scale_if_joint_position the joint keeps its default scale),
    /// - LLVOAvatar::updateCharacter: the pelvis sits (0.5 · body height −
    ///   pelvis to foot) below the object center, raised by the hover.
    ///
    /// Attachments are re-read every 2 s, the shape on each new appearance.
    fn skeleton_of(&mut self, world: &World, owner: Uuid, owner_idx: usize, now: Instant) -> (Arc<anim::SkeletonBase>, f32) {
        let generation = world.appearance_generation(&owner);
        if let Some(s) = self.skeletons.get(&owner)
            && s.appearance_gen == generation
            && now.duration_since(s.at).as_secs_f32() < 2.0
        {
            return (s.base.clone(), s.root_dz);
        }
        let rig = self.avatar_lib.rig.clone();
        // joint offsets of the worn meshes
        let mut overrides: Vec<(usize, Vec3, bool)> = Vec::new();
        let mut stack = vec![owner_idx];
        let mut guard = 0;
        while let Some(i) = stack.pop() {
            guard += 1;
            if guard > 20_000 {
                break;
            }
            let Some(o) = world.objects.get(i) else {
                continue;
            };
            stack.extend_from_slice(world.objects.children_of(&o.key));
            if i == owner_idx || !o.volume.is_mesh() {
                continue;
            }
            let Some(skin) = o.volume.sculpt.and_then(|s| self.meshes.meta(&s.texture)).and_then(|m| m.skin) else {
                continue;
            };
            if skin.alt_inverse_bind.is_empty() || skin.alt_inverse_bind.len() != skin.joint_names.len() {
                continue;
            }
            for (name, alt) in skin.joint_names.iter().zip(&skin.alt_inverse_bind) {
                // the pelvis stays at the avatar position
                if name == "mPelvis" {
                    continue;
                }
                let Some(&j) = rig.names.get(name) else {
                    continue;
                };
                let p = alt.w_axis.truncate();
                if p.is_finite() && (p - rig.local_pos[j]).length_squared() > 1e-8 {
                    overrides.push((j, p, skin.lock_scale_if_joint_position));
                }
            }
        }
        let avatar = world.objects.get(owner_idx).is_some_and(|o| o.is_avatar());
        let (mut local_pos, mut scale, hover) = if avatar {
            let values = world.visual_params(&owner).unwrap_or(&[]);
            let s = self.avatar_lib.shape_params.shape(values, &rig);
            (s.local_pos, s.scale, s.hover)
        } else {
            (rig.local_pos.clone(), rig.scale.clone(), 0.0)
        };
        for (j, p, lock) in overrides {
            local_pos[j] = p;
            if lock {
                scale[j] = rig.scale[j];
            }
        }
        let root_dz = if avatar {
            let (pelvis_to_foot, height) = shape::body_size(&rig, &local_pos, &scale);
            let seated = world.objects.get(owner_idx).is_some_and(|o| o.parent_id != 0);
            let hover_offset = if seated { 0.0 } else { world.appearance_hover(&owner) };
            let pelvis_shift = local_pos[rig.pelvis].z - rig.local_pos[rig.pelvis].z;
            hover - (0.5 * height - pelvis_to_foot) + hover_offset - pelvis_shift
        } else {
            0.0
        };
        let base = Arc::new(anim::SkeletonBase { local_pos, scale });
        self.skeletons.insert(
            owner,
            AvatarSkeleton {
                at: now,
                appearance_gen: generation,
                base: base.clone(),
                root_dz,
            },
        );
        (base, root_dz)
    }
}

impl Scene {
    /// Diagnostic: one log line per attachment of an avatar (inventory name,
    /// mesh / rigged, faces drawn, position relative to the avatar).
    pub fn log_attachments(&self, world: &World, avatar: Uuid) {
        let Some(aidx) = world.objects.index_of_uuid(&avatar) else {
            return;
        };
        let Some(av) = world.objects.get(aidx) else {
            return;
        };
        let apos = Self::object_transform(world, aidx, Instant::now(), 0)
            .map(|t| t.0)
            .unwrap_or(Vec3::ZERO);
        log::info!(
            "attachments of {}: root offset {:.3} m, appearance params {}",
            av.display_name().unwrap_or_default(),
            world.avatar_root_dz.get(&avatar).copied().unwrap_or(0.0),
            world.visual_params(&avatar).map(|v| v.len()).unwrap_or(0)
        );
        for (te, name, _) in avatar::BAKES {
            if let Some(t) = world.bake_texture(aidx, te) {
                log::info!("  bake {name}: {t} — {}", self.textures.describe(&t));
            }
        }
        let alpha = &self.textures.alpha_by_slot;
        for &att in world.objects.children_of(&av.key) {
            let Some(root) = world.objects.get(att) else {
                continue;
            };
            let name = root
                .attachment_item_id()
                .and_then(|id| world.inventory.items.get(&id))
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "?".into());
            let mut prims = vec![att];
            prims.extend_from_slice(world.objects.children_of(&root.key));
            // the whole linkset is detailed when one of its prims shows the head bake
            let head_set = prims.iter().any(|&i| {
                world
                    .objects
                    .get(i)
                    .and_then(|o| o.te.as_ref())
                    .is_some_and(|t| t.faces.iter().any(|f| f.texture == avatar::BAKES[0].2))
            });
            let (mut meshes, mut rigged, mut faces, mut shown, mut pending) = (0, 0, 0, 0, 0);
            let mut far = 0.0f32;
            for &i in &prims {
                let Some(o) = world.objects.get(i) else {
                    continue;
                };
                if o.volume.is_mesh() {
                    meshes += 1;
                    if o.volume
                        .sculpt
                        .and_then(|s| self.meshes.meta(&s.texture))
                        .is_some_and(|m| m.skin.is_some())
                    {
                        rigged += 1;
                    }
                }
                let Some(g) = self.gpu.get(i) else {
                    continue;
                };
                if g.geom.is_none() {
                    pending += 1;
                }
                for f in &g.faces {
                    faces += 1;
                    let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                    if classify(f, a) != Pass::Hidden {
                        shown += 1;
                    }
                }
                if !g.faces.is_empty() {
                    far = far.max(g.center.distance(apos));
                }
                // per-face detail for the head (attachment points 2 skull,
                // 12 chin... any point carrying a head bake)
                let te = o.te.as_ref();
                if head_set {
                    let mut line = String::new();
                    for (fi, f) in g.faces.iter().enumerate() {
                        let tf = te.map(|t| *t.face(fi)).unwrap_or_default();
                        let tex = match avatar::bake_slot_for(&tf.texture) {
                            Some((_, name)) => format!("bake:{name}"),
                            None => format!("{} ({})", tf.texture, self.textures.describe(&tf.texture)),
                        };
                        let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                        line.push_str(&format!(
                            " [{fi} {tex} rgba {:.2},{:.2},{:.2},{:.2} legacy {:?} pbr {:?} tex {:?} -> {:?}]",
                            tf.color[0],
                            tf.color[1],
                            tf.color[2],
                            f.te_alpha,
                            f.legacy_alpha,
                            f.pbr_alpha,
                            a,
                            classify(f, a)
                        ));
                    }
                    if g.faces.is_empty() {
                        line = format!(" no faces (geometry {:?}, mesh {})", g.geom, o.volume.is_mesh());
                    }
                    log::info!("  head prim {}:{line}", o.full_id);
                }
            }
            // legacy materials of our own attachments (normal / specular maps)
            if avatar == world.agent_id {
                let mut seen: Vec<Uuid> = Vec::new();
                for &i in &prims {
                    let Some(te) = world.objects.get(i).and_then(|o| o.te.as_ref()) else {
                        continue;
                    };
                    for f in &te.faces {
                        if f.material_id.is_nil() || seen.contains(&f.material_id) {
                            continue;
                        }
                        seen.push(f.material_id);
                        if let Some(lm) = self.legacy_mats.peek(&f.material_id) {
                            log::info!(
                                "  material {}: normal {} ({}) x{:.2?}, specular {} ({}) x{:.2?}, color {:?} exp {} env {} alpha mode {}",
                                f.material_id,
                                lm.normal_map,
                                self.textures.describe(&lm.normal_map),
                                &lm.normal_st[..2],
                                lm.specular_map,
                                self.textures.describe(&lm.specular_map),
                                &lm.specular_st[..2],
                                lm.specular_color,
                                lm.specular_exp,
                                lm.env_intensity,
                                lm.diffuse_alpha_mode
                            );
                        }
                    }
                }
            }
            log::info!(
                "attachment '{name}' point {}: {} prims ({meshes} mesh, {rigged} rigged, {pending} without geometry), faces {shown}/{faces} drawn, farthest center {far:.2} m from the avatar, root offset {:.3?} on its point",
                root.attachment_point(),
                prims.len(),
                root.position
            );
        }
    }

    /// Diagnostic: the objects (not avatars nor their attachments) within
    /// `radius` m of `center`: lights and every face's texture entry, as
    /// the build floater would show them.
    pub fn log_nearby(&self, world: &World, center: Vec3, radius: f32) {
        let alpha = &self.textures.alpha_by_slot;
        let mut n = 0;
        for (idx, o) in world.objects.iter() {
            let Some(g) = self.gpu.get(idx) else {
                continue;
            };
            if g.is_avatar || g.owner_avatar.is_some() || g.hud || g.faces.is_empty() {
                continue;
            }
            if g.center.distance(center) - g.radius > radius {
                continue;
            }
            n += 1;
            log::info!(
                "object {} local {}: {} material {} scale {:.2?} center {:.1?}, {} faces, light {:?}",
                o.full_id,
                o.key.local_id,
                if o.volume.is_mesh() { "mesh" } else { "prim" },
                o.prim_material,
                o.scale,
                g.center,
                g.faces.len(),
                o.extra.light
            );
            let te = o.te.as_ref();
            let mut mats: Vec<Uuid> = Vec::new();
            for (fi, f) in g.faces.iter().enumerate() {
                let tf = te.map(|t| *t.face(fi)).unwrap_or_default();
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                let pbr = o.extra.render_materials.iter().find(|(t, _)| *t as usize == fi).map(|(_, id)| *id);
                log::info!(
                    "  face {fi}: tex {} ({}), color {:.2?}, glow {:.2}, fullbright {}, shiny {}, bump {}, planar {}, repeats {:.2}x{:.2}, material {}, pbr {:?}, pass {:?}",
                    tf.texture,
                    self.textures.describe(&tf.texture),
                    tf.color,
                    tf.glow,
                    tf.fullbright(),
                    tf.shiny(),
                    tf.bump(),
                    tf.planar(),
                    tf.scale_s,
                    tf.scale_t,
                    tf.material_id,
                    pbr,
                    classify(f, a)
                );
                if !tf.material_id.is_nil() && !mats.contains(&tf.material_id) {
                    mats.push(tf.material_id);
                }
            }
            for id in mats {
                match self.legacy_mats.peek(&id) {
                    Some(lm) => log::info!(
                        "  material {id}: normal {} x{:.2?}, specular {} x{:.2?}, color {:?} exp {} env {} alpha mode {}",
                        lm.normal_map,
                        &lm.normal_st[..2],
                        lm.specular_map,
                        &lm.specular_st[..2],
                        lm.specular_color,
                        lm.specular_exp,
                        lm.env_intensity,
                        lm.diffuse_alpha_mode
                    ),
                    None => log::info!("  material {id}: not loaded"),
                }
            }
        }
        log::info!("nearby objects: {n} within {radius} m of {center:.1?}");
        // faces with glow worn by the avatars around (what feeds the glow pass)
        for (idx, o) in world.objects.iter() {
            let Some(g) = self.gpu.get(idx) else {
                continue;
            };
            if g.owner_avatar.is_none() || g.hud || g.center.distance(center) > radius {
                continue;
            }
            let te = o.te.as_ref();
            for (fi, f) in g.faces.iter().enumerate() {
                let tf = te.map(|t| *t.face(fi)).unwrap_or_default();
                if tf.glow <= 0.0 {
                    continue;
                }
                let a = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
                // the attachment root: the prim whose parent is the avatar
                let mut root = o;
                for _ in 0..256 {
                    match world.objects.parent_of(root).and_then(|p| world.objects.get(p)) {
                        Some(p) if !p.is_avatar() => root = p,
                        _ => break,
                    }
                }
                let item = root
                    .attachment_item_id()
                    .and_then(|id| world.inventory.items.get(&id))
                    .map(|i| i.name.clone())
                    .unwrap_or_else(|| "?".into());
                log::info!(
                    "  worn glow: '{item}' prim {} face {fi}: glow {:.2}, tex {} ({}, {:?}, alpha channel {}), color {:.2?}, fullbright {}, material {} legacy alpha {:?}, pass {:?}, two sided {}",
                    o.full_id,
                    tf.glow,
                    tf.texture,
                    self.textures.describe(&tf.texture),
                    a,
                    self.textures
                        .alpha_channel_by_slot
                        .get(f.base_slot as usize)
                        .copied()
                        .unwrap_or(false),
                    tf.color,
                    tf.fullbright(),
                    tf.material_id,
                    f.legacy_alpha,
                    classify(f, a),
                    f.two_sided
                );
            }
        }
    }
}

/// Cached rest skeleton of one skeleton owner.
struct AvatarSkeleton {
    at: Instant,
    appearance_gen: u64,
    base: Arc<anim::SkeletonBase>,
    root_dz: f32,
}

fn classify(f: &FaceDraw, tex_alpha: AlphaKind) -> Pass {
    // fully transparent face, or the transparent texture (unused layers of
    // mesh heads and bodies): nothing to draw
    if f.te_alpha < 0.004 || (f.base_slot == aurora_render::textures::TRANSPARENT && f.pbr_alpha.is_none()) {
        return Pass::Hidden;
    }
    let two = f.two_sided;
    match f.pbr_alpha {
        Some(0) => return if two { Pass::OpaqueTwoSided } else { Pass::Opaque },
        Some(1) => return Pass::Blend,
        Some(2) => return if two { Pass::MaskTwoSided } else { Pass::Mask },
        _ => {}
    }
    if f.te_alpha < 0.999 {
        return Pass::Blend;
    }
    // alpha mode chosen by the creator (legacy material) wins over detection
    let opaque = if two { Pass::OpaqueTwoSided } else { Pass::Opaque };
    match f.legacy_alpha {
        Some(legacy_mat::alpha_mode::NONE) | Some(legacy_mat::alpha_mode::EMISSIVE) => return opaque,
        Some(legacy_mat::alpha_mode::BLEND) => {
            return if tex_alpha == AlphaKind::Opaque { opaque } else { Pass::Blend };
        }
        Some(legacy_mat::alpha_mode::MASK) => return if two { Pass::MaskTwoSided } else { Pass::Mask },
        _ => {}
    }
    match tex_alpha {
        AlphaKind::Opaque => {
            if two {
                Pass::OpaqueTwoSided
            } else {
                Pass::Opaque
            }
        }
        AlphaKind::Mask => {
            if two {
                Pass::MaskTwoSided
            } else {
                Pass::Mask
            }
        }
        AlphaKind::Blend => Pass::Blend,
    }
}
