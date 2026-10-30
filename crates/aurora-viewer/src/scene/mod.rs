//! GPU scene synchronisation: geometry cache, per-face draw records,
//! texture/mesh/material streaming, culling and draw-list building.

pub mod anchors;
pub mod anim;
mod anim_box;
pub mod animesh;
pub mod animesh_report;
mod auto_hover;
pub mod avatar;
pub mod banlines;
pub mod blend_order;
mod blender_live;
pub mod body_physics;
pub mod complexity;
pub mod creation_preview;
pub(crate) mod default_motions;
pub mod describe;
pub mod drawn;
pub mod fetch_priority;
mod flexible;
mod foot_scope;
mod foot_support;
pub mod geometry_need;
mod grounding;
mod hand_ik;
pub mod hover;
pub mod hud;
pub(crate) mod ik;
pub mod impostors;
pub mod inspect;
pub mod jobs;
pub(crate) mod joint_overrides;
pub mod legacy_mat;
pub mod lights;
mod list_index;
pub(crate) mod live_morphs;
pub mod loading;
mod lod_watch;
pub mod mat_changes;
pub mod mesh_bytes;
pub mod mesh_inspection;
pub mod meshes;
pub mod morph;
pub mod particles;
pub mod physics_shapes;
mod pick_accel;
pub mod picking;
mod plants;
pub mod portals;
pub mod probes;
pub mod rez;
mod sculpt_watch;
mod selection_demo;
pub mod settings;
pub mod shape;
mod shown_bakes;
mod skin_binds;
pub mod sounds;
pub mod still_check;
mod sync_check;
mod sync_plan;
mod sync_slow;
mod world_overlays;
pub use sync_plan::LodInfo;
pub mod sync_sets;
pub mod tangents;
mod terrain_surface;
pub mod texture_budget;
pub(crate) mod texture_cache;
pub mod texture_importance;
mod texture_load;
mod texture_ui;
use terrain_surface::SurfaceGpu;
pub mod textures;
pub mod water;
mod water_contacts;
mod water_effects;
pub mod wind_sound;

use crate::fast_hash::{FastMap, FastSet};
use crate::frame_par::{self, Grain};
use crate::world::World;
use crate::world::objects::{Object, ObjectStore};
use crate::world::terrain::{self, CHUNK_CELLS, Composition};
use aurora_net::NetClient;
use aurora_prim::te::TextureFace;
use aurora_render::gpu_cull::{CullFace, CullObject, FACE_FIRST, GpuCullView, NO_OBJECT, avatar_state, object_flags};
use aurora_render::{DrawCmd, DrawLists, DrawRecord, MeshAlloc, Renderer, ShadowCaster, flags};
use avatar::AvatarLibrary;
use glam::{Mat4, Quat, Vec3, Vec4};
use jobs::{AlphaKind, JobResult, Jobs};
use meshes::{MaterialStreamer, MeshStreamer};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;
use sync_plan::{FrameState, MoveSync, Placement, Plan};
use sync_sets::{FullSyncBudget, SyncSets, Xf, XfCache};
use textures::{TexSource, TextureStreamer};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GeomKey {
    Prim {
        hash: u64,
        lod: u8,
    },
    Flexible {
        id: Uuid,
        hash: u64,
        lod: u8,
    },
    Sculpt {
        hash: u64,
        lod: u8,
    },
    /// The sphere a sculpt shows while its map is missing or failed
    /// (`aurora_prim::sculpt_placeholder`); `hash` is its sculpt's.
    SculptPlaceholder {
        hash: u64,
        lod: u8,
    },
    Mesh {
        id: Uuid,
        lod: u8,
    },
    /// A Linden tree of a species (`aurora_prim::tree`), shared by every
    /// tree of it at this detail level (0 the coarsest, like the others).
    Tree {
        species: u8,
        lod: u8,
    },
    /// A Linden grass tuft (`aurora_prim::grass`): its own, standing on the
    /// ground under it; `hash` covers its species, size and position.
    Grass {
        id: Uuid,
        hash: u64,
    },
    AvatarPart(u8),
    /// A system body part morphed by a set of visual parameter weights
    /// (morph.rs), shared by the avatars that have them.
    AvatarMorph {
        part: u8,
        weights: u64,
    },
}

/// How a system body part is drawn, whatever the alpha class of its baked
/// texture (LLVOAvatar::renderSkinned and renderTransparent, drawn by
/// LLDrawPoolAvatar with its minimum alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemPart {
    /// Head, upper and lower body, eyes: in the opaque passes, alpha-tested.
    Body,
    /// Hair, eyelashes, skirt: blended, with the same alpha test.
    /// (Firestorm also writes their depth; the blended faces here do not,
    /// they are sorted with the others.)
    Transparent,
}

/// LLDrawPoolAvatar::sMinimumAlpha, and the alpha rejection of the skirt in
/// LLVOAvatar::renderTransparent.
const AVATAR_MINIMUM_ALPHA: f32 = 0.2;
const AVATAR_SKIRT_MINIMUM_ALPHA: f32 = 0.25;

impl SystemPart {
    fn of(part: &str) -> SystemPart {
        match part {
            "hair" | "eyelashes" | "skirt" => SystemPart::Transparent,
            _ => SystemPart::Body,
        }
    }
}

pub struct GpuGeom {
    pub faces: Vec<Option<MeshAlloc>>,
    pub min: Vec3,
    pub max: Vec3,
    pub joint_bounds: Vec<animesh::JointBounds>,
    pub pick_faces: Vec<Option<picking::PickFace>>,
}

enum GeomState {
    Pending,
    Ready(Arc<GpuGeom>),
    Failed,
}

struct GeomEntry {
    state: GeomState,
    refs: u32,
    /// Scene frame its last reference went away (or it was created):
    /// unused for `GEOM_KEEP_FRAMES` frames, it is freed.
    zero_since: u32,
}

/// Frames an unused geometry is kept (an object may want it back soon).
const GEOM_KEEP_FRAMES: u32 = 600;
/// Unused geometries looked at per frame once their time has come.
const GEOM_FREED_PER_FRAME: usize = 256;
/// Frames between two looks at what an avatar's attachments show of its
/// bakes when nothing it wears changed: a safety net for a change the
/// store did not report (every 30 frames before, 2 % of the main thread
/// in a busy club).
const BOM_SWEEP_FRAMES: usize = 300;

/// How the frame's loops over objects are shared between the main thread
/// and the pool's workers (frame_par.rs); the costs are those of the dev
/// profile. Placing an object in the sync's planning: 0.3 µs.
const PLAN_GRAIN: Grain = Grain { chunk: 32, per_helper: 8 };

/// The per-object table with room for `n` objects, its memory already
/// written once: the pages of a fresh allocation are only given by the
/// system when first written, which the frame filling the table for a
/// region's objects paid for (0.4 ms for 12 000 objects).
fn prefaulted_table(n: usize) -> Vec<ObjGpu> {
    let mut gpu = Vec::new();
    gpu.resize_with(n, ObjGpu::default);
    gpu.clear();
    gpu
}

/// First syncs of new objects between two readings of the clock.
const ARRIVALS_PER_CLOCK: usize = 8;
/// An object's faces sorted into the draw lists: about as much. With the
/// lists culled on the GPU a frame visits a few hundred objects, on the
/// calling thread alone.
const LISTS_GRAIN: Grain = Grain { chunk: 128, per_helper: 4 };
/// The pose of an avatar: 8 µs.
const POSES_GRAIN: Grain = Grain { chunk: 1, per_helper: 8 };
/// A flexible prim simulated and meshed again: 50 µs or more.
const FLEXIBLE_GRAIN: Grain = Grain { chunk: 1, per_helper: 2 };

/// Seconds a cached rest skeleton is used before it is rebuilt anyway (the
/// net under the events that rebuild it): 2 to 3 s, by owner. With the same
/// 2 s for all, the avatars of a crowd that arrived together were rebuilt in
/// the same frame from then on (40 avatars: one frame in two seconds 1.5 ms
/// longer).
fn skeleton_max_age(owner: &Uuid) -> f32 {
    // (the id's low bits spread over 0..1, also for ids that follow each other)
    2.0 + ((owner.as_u128() as u32).wrapping_mul(0x9E37_79B9) >> 22) as f32 / 1024.0
}

/// Where the ray `origin + dir * t` (t ≥ 0) enters the upright cylinder of
/// `radius` around `center`, from `center.z + low` to `center.z + high`.
pub(crate) fn ray_capsule(origin: Vec3, dir: Vec3, center: Vec3, radius: f32, low: f32, high: f32) -> Option<f32> {
    let (o, d) = (origin - center, dir);
    // the interval of t inside the infinite cylinder
    let a = d.x * d.x + d.y * d.y;
    let (mut t0, mut t1) = if a < 1e-9 {
        if o.x * o.x + o.y * o.y > radius * radius {
            return None;
        }
        (f32::NEG_INFINITY, f32::INFINITY)
    } else {
        let b = o.x * d.x + o.y * d.y;
        let c = o.x * o.x + o.y * o.y - radius * radius;
        let disc = b * b - a * c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        ((-b - s) / a, (-b + s) / a)
    };
    // within its height
    if d.z.abs() < 1e-9 {
        if !(low..=high).contains(&o.z) {
            return None;
        }
    } else {
        let (za, zb) = ((low - o.z) / d.z, (high - o.z) / d.z);
        t0 = t0.max(za.min(zb));
        t1 = t1.min(za.max(zb));
    }
    (t1 >= t0.max(0.0)).then_some(t0.max(0.0))
}

/// Pass a face is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pass {
    Opaque,
    OpaqueTwoSided,
    Mask,
    MaskTwoSided,
    Blend,
    #[default]
    Hidden,
}

impl Pass {
    /// Code of the pass in the GPU face table (`gpu_cull::pass`).
    pub fn gpu_code(self) -> u32 {
        use aurora_render::gpu_cull::pass;
        match self {
            Pass::Opaque => pass::OPAQUE,
            Pass::OpaqueTwoSided => pass::OPAQUE_2S,
            Pass::Mask => pass::MASK,
            Pass::MaskTwoSided => pass::MASK_2S,
            Pass::Blend => pass::BLEND,
            Pass::Hidden => pass::HIDDEN,
        }
    }

    /// Alpha mode bits of the face's draw record (shaders, shadows).
    fn record_flags(self) -> u32 {
        match self {
            Pass::Blend => flags::ALPHA_BLEND,
            Pass::Mask | Pass::MaskTwoSided => flags::ALPHA_MASK,
            _ => 0,
        }
    }
}

/// Cached per-face draw info.
#[derive(Debug, Clone, Copy)]
pub struct FaceDraw {
    /// Texture-entry index; omitted transparent geometry leaves holes.
    pub te_index: u8,
    pub record: u32,
    pub cmd: DrawCmd,
    pub base_slot: u32,
    pub tex_id: Uuid,
    /// Normal, specular / metallic-roughness and emissive maps (sized like
    /// the base texture).
    pub aux_tex: [Uuid; 3],
    pub te_alpha: f32,
    pub pbr_alpha: Option<u8>, // 0 opaque, 1 blend, 2 mask
    /// Legacy material alpha mode (`legacy_mat::alpha_mode`).
    pub legacy_alpha: Option<u8>,
    pub two_sided: bool,
    /// The smaller of the texture entry's two scales, or of the frame grid of
    /// its texture animation: every texture of the face is loaded for it
    /// (`texture_importance::per_repeats`).
    pub repeats: f32,
    /// TE glow > 0: also drawn in the glow pass.
    pub glow: bool,
    /// Pass from the alpha classification (`face_mode`), redone when a
    /// texture changes alpha class (`TextureStreamer::generation`).
    pub pass: Pass,
    /// Glowing face drawn in LL's alpha pool (`glow_alpha`).
    pub glow_pool: bool,
    /// `glow` was set by the rez effect alone (rez.rs), for its line.
    pub rez_glow: bool,
    /// Part of a system avatar body: its pass follows the avatar pool.
    pub system: Option<SystemPart>,
    pub first_person_hidden: bool,
}

impl FaceDraw {
    /// LLVolumeGeometryManager::genDrawInfo sends opaque alpha-ramp faces
    /// to PASS_INVISIBLE. They affect water only, never scene colour/depth.
    fn water_exclusion(&self) -> bool {
        self.tex_id == aurora_prim::te::WATER_EXCLUSION_TEXTURE && self.pbr_alpha.is_none() && self.te_alpha >= 0.999
    }

    /// Its zero alpha hides the whole face, as LL never draws it: a
    /// Blinn-Phong face, or a blended glTF material. An opaque glTF
    /// material ignores its base color alpha and a masked one leaves it to
    /// the shader's cutoff (LLVolumeGeometryManager::rebuildGeom and
    /// genDrawInfo, indra/newview/llvovolume.cpp, originally LGPL 2.1).
    pub fn fully_transparent(&self) -> bool {
        self.te_alpha < 0.004 && matches!(self.pbr_alpha, None | Some(1))
    }

    /// Alpha mode bits of the face's draw record (shaders, shadows). A
    /// blended system body part is also alpha-tested.
    fn record_flags(&self) -> u32 {
        let test = self.pass == Pass::Blend && self.system.is_some();
        self.pass.record_flags() | if test { flags::ALPHA_MASK } else { 0 }
    }
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
    pub rlva_silhouette: bool,
    /// Avatars: the morphed geometries of its body parts it holds
    /// (`GeomKey::AvatarMorph`), and whether one of them is still being
    /// built (the part is drawn unmorphed meanwhile).
    pub morph_geoms: Vec<GeomKey>,
    pub morph_pending: bool,
    /// Rigged mesh (skinned to an avatar skeleton).
    pub rigged: bool,
    /// Skeleton bound to the face records (changes on attachment / flag updates).
    pub skeleton_owner: Option<Uuid>,
    /// Rigged: where its blended faces come among those of its skeleton
    /// (`blend_order::RigOrder`).
    pub rig_order: Option<blend_order::RigOrder>,
    pub mat_generation: u64,
    /// Geometry faces the records were built for (fully transparent faces
    /// get no record, so `faces` can be shorter).
    pub built_faces: usize,
    pub is_avatar: bool,
    pub parcel_hidden: bool,
    /// Left out by the photo's isolation (`snapshot::isolate`): kept, only
    /// flagged for the GPU lists (`object_flags::PHOTO_HIDDEN`) and left
    /// out of the CPU ones, so that it comes back at once.
    pub photo_hidden: bool,
    pub hud: bool,
    pub sculpt_wait: Option<Uuid>,
    /// Avatar wearing this object (attachments), for the avatar limit.
    pub owner_avatar: Option<usize>,
    pub first_person_hidden: bool,
    /// Reflection probe flagged as a mirror (box); listed in
    /// `Scene::mirror_objects`.
    pub mirror: bool,
    /// Listed in `Scene::light_objects` by its last sync.
    pub is_light: bool,
    /// Has blended or glowing faces: still listed by the CPU when the GPU
    /// culls the other draws (gpu_cull.rs).
    pub special: bool,
    /// Avatars: sphere of the posed body (system mesh skinned by the current
    /// palette), for culling and occlusion; None until posed.
    pub posed_sphere: Option<(Vec3, f32)>,
    /// Avatars: `gpu_cull::avatar_state` of the last lists.
    pub cull_state: u32,
    /// Placed by the GPU (anchors.rs): the records are placed from this
    /// anchor (a worn object's attachment point, a rigged mesh's skeleton),
    /// and the sync no longer visits the object while only its wearer moves.
    pub anchor: Option<anchors::AnchorRef>,
    /// Anchored: `center` in the anchor's space (`center` itself is brought
    /// up to date from the anchor at each frame).
    pub local_center: Vec3,
    /// Anchored rigged mesh: what the GPU poses its bounds from.
    pub posed: Option<anchors::Posed>,
    /// Anchored rigged mesh: sphere of its skeleton this frame, which the
    /// lists built by the CPU test it with (its own bounds are only as
    /// fresh as its last visit).
    pub wide: Option<(Vec3, f32)>,
}

impl ObjGpu {
    /// Its wanted geometry is bound, or the sphere placeholder stands in
    /// for a sculpt whose map is missing (nothing more is coming).
    pub fn geometry_built(&self) -> bool {
        match (self.wanted_geom, self.geom) {
            (Some(GeomKey::Sculpt { hash, lod }), Some(GeomKey::SculptPlaceholder { hash: h, lod: l })) => hash == h && lod == l,
            (wanted, geom) => wanted.is_none_or(|k| geom == Some(k)) && self.sculpt_wait.is_none(),
        }
    }

    /// Culling sphere: the bounds, or for an avatar its posed body (2.5 m
    /// around its position until posed).
    pub fn sphere(&self) -> (Vec3, f32) {
        if self.is_avatar {
            self.posed_sphere.unwrap_or((self.center, 2.5))
        } else {
            self.wide.unwrap_or((self.center, self.radius))
        }
    }

    /// Rigged mesh the sync places again at each frame with its skeleton
    /// (one the GPU places needs no visit).
    pub fn moves_with_skeleton(&self) -> bool {
        self.rigged && self.anchor.is_none()
    }

    /// Avatar whose limits apply: itself, or the wearer of an attachment.
    pub fn avatar(&self, idx: usize) -> Option<usize> {
        if self.is_avatar { Some(idx) } else { self.owner_avatar }
    }

    /// Entry of the GPU object table.
    fn cull_object(&self, idx: usize) -> CullObject {
        if self.faces.is_empty() {
            // nothing to draw, but an avatar's state still rules its
            // attachments
            return CullObject {
                state: self.cull_state,
                ..CullObject::NONE
            };
        }
        let (c, r) = self.sphere();
        // on the GPU, a worn object's sphere follows its anchor, and a
        // rigged mesh's is posed from its joint boxes (else its skeleton's)
        let (sphere, anchor) = match (self.anchor, self.posed) {
            (Some(a), Some(p)) => (p.rest, a.matrix + 1),
            (Some(a), None) => (self.local_center.extend(self.radius).to_array(), a.matrix + 1),
            (None, _) => ([c.x, c.y, c.z, r], 0),
        };
        let posed = self.posed.filter(|p| anchor != 0 && p.count > 0);
        let avatar = self.avatar(idx);
        let mut fl = object_flags::ACTIVE;
        for (on, bit) in [
            (self.is_avatar, object_flags::AVATAR),
            (self.rigged, object_flags::RIGGED),
            (self.hud, object_flags::HUD),
            (avatar.is_some() || self.skeleton_owner.is_some(), object_flags::NO_PROBE),
            (posed.is_some(), object_flags::POSED),
            (self.first_person_hidden, object_flags::FIRST_PERSON_HIDDEN),
            (self.photo_hidden, object_flags::PHOTO_HIDDEN),
        ] {
            if on {
                fl |= bit;
            }
        }
        CullObject {
            sphere,
            flags: fl,
            avatar: avatar.map_or(NO_OBJECT, |a| a as u32),
            state: self.cull_state,
            anchor,
            boxes: posed.map_or(0, |p| p.boxes),
            box_count: posed.map_or(0, |p| p.count),
            binds: posed.map_or(0, |p| p.binds),
            _pad: 0,
        }
    }
}

/// Pass of a face and whether its glow goes to LL's alpha pool, from the
/// alpha class and alpha channel of its base texture.
///
/// `object_auto_mask`: when false, a texture that looks binary is still
/// blended. Matches `LLFace::canRenderAsMask` (indra/newview/llface.cpp):
/// never auto alpha-mask a rigged face or a HUD attachment. Soft genital
/// and hair alphas stay visible; an explicit MASK material / PBR mask is
/// unchanged.
fn face_mode(f: &FaceDraw, alpha: &[AlphaKind], channel: &[bool], object_auto_mask: bool) -> (Pass, bool) {
    let tex_alpha = alpha.get(f.base_slot as usize).copied().unwrap_or(AlphaKind::Opaque);
    let mut pass = classify(f, tex_alpha);
    // LLFace::canRenderAsMask: never auto-mask rigged faces, HUD, or glow.
    // Creator MASK / PBR mask and system body parts keep their pass.
    let auto = object_auto_mask && !f.glow;
    if !auto
        && matches!(pass, Pass::Mask | Pass::MaskTwoSided)
        && f.pbr_alpha.is_none()
        && !matches!(f.legacy_alpha, Some(legacy_mat::alpha_mode::MASK))
        && f.system.is_none()
    {
        pass = Pass::Blend;
    }
    // LLPipeline::getPoolTypeFromTE + canRenderAsMask: a glowing face whose
    // texture has an alpha channel goes to the alpha pool unless its
    // material says none / mask / emissive
    let has_channel = channel.get(f.base_slot as usize).copied().unwrap_or(false);
    let pool = f.pbr_alpha.is_none() && has_channel && matches!(f.legacy_alpha, None | Some(legacy_mat::alpha_mode::BLEND));
    (pass, pool)
}

/// What the object tests of a frame need (CPU lists).
struct CullCtx<'a> {
    view: &'a CullView,
    draw_distance: f32,
    shadow_distance: f32,
    reflection_distance: f32,
    shadows: bool,
    reflections: bool,
}

/// An object that passed the tests: its culling sphere and the views it
/// may be drawn in.
struct ObjVis {
    sphere: (Vec3, f32),
    in_view: bool,
    casts: bool,
    reflects: bool,
}

/// Object tests of the CPU lists, the same as `face_mask` in cull.wgsl:
/// avatar limit (impostor), too complex (attachments hidden), still
/// loading, draw distance, view / shadow / reflection reach and small
/// object culling, on the culling sphere (avatars: their posed body).
fn object_visibility(
    g: &ObjGpu,
    idx: usize,
    ctx: &CullCtx,
    avatar_full: impl Fn(usize) -> bool,
    too_complex: impl Fn(usize) -> bool,
    loading: impl Fn(usize) -> bool,
) -> Option<ObjVis> {
    if g.faces.is_empty() || g.hud {
        return None;
    }
    let avatar = g.avatar(idx);
    if avatar.is_some_and(|a| !avatar_full(a)) {
        return None;
    }
    // too complex: only its silhouette, no attachments
    if g.owner_avatar.is_some_and(&too_complex) {
        return None;
    }
    // still loading: a cloud until complete (not half an avatar)
    if avatar.is_some_and(loading) {
        return None;
    }
    let (sc, sr) = g.sphere();
    let d = (sc - ctx.view.eye).length();
    if d - sr > ctx.draw_distance && !g.is_avatar {
        return None;
    }
    let in_view = ctx.view.sphere_visible(sc, sr);
    let casts = ctx.shadows && d - sr < ctx.shadow_distance;
    let reflects = ctx.reflections && d - sr < ctx.reflection_distance;
    if !in_view && !casts && !reflects {
        return None;
    }
    // tiny objects far away are culled (LL-like small object culling)
    if ctx.view.pixel_size(sc, sr) < 1.5 && !g.is_avatar {
        return None;
    }
    Some(ObjVis {
        sphere: (sc, sr),
        in_view,
        casts,
        reflects,
    })
}

/// Texture usage of a face of an object whose textures are sized `px`
/// (`texture_importance::texture_pixels`): its base texture and those of
/// its material, all of the same size, lowered by the face's repeats as the
/// Legacy Viewer does (id, pixels, in view).
fn note_face_usage(usage: &mut Vec<(Uuid, f32, bool)>, f: &FaceDraw, px: f32, in_view: bool) {
    let px = texture_importance::per_repeats(px, f.repeats);
    usage.push((f.tex_id, px, in_view));
    for t in f.aux_tex.iter().filter(|t| !t.is_nil()) {
        usage.push((*t, px, in_view));
    }
}

/// GPU face table entry of a face of object `idx`.
fn cull_face(f: &FaceDraw, idx: usize, first: bool) -> CullFace {
    CullFace {
        index_count: f.cmd.index_count,
        first_index: f.cmd.first_index,
        base_vertex: f.cmd.base_vertex,
        object: idx as u32,
        bits: if f.water_exclusion() {
            aurora_render::gpu_cull::pass::WATER_EXCLUSION
        } else {
            f.pass.gpu_code()
        } | if first { FACE_FIRST } else { 0 }
            | if f.first_person_hidden {
                aurora_render::gpu_cull::FACE_FIRST_PERSON_HIDDEN
            } else {
                0
            },
        _pad: [0; 3],
    }
}

/// Faces listed by the CPU even with the GPU lists: blended (sorted back
/// to front) and glowing ones.
fn is_special(faces: &[FaceDraw]) -> bool {
    faces.iter().any(|f| f.pass == Pass::Blend || (f.glow && f.pass != Pass::Hidden))
}

struct TerrainChunk {
    alloc: MeshAlloc,
    record: u32,
    center: Vec3,
    radius: f32,
}

struct RegionGpu {
    chunks: HashMap<u32, TerrainChunk>,
    surface: SurfaceGpu,
    offset: Vec3,
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
    /// Objects updated by the last sync, and those whose faces were rebuilt.
    pub synced: usize,
    pub rebuilt: usize,
    /// Objects whose full sync waits for a frame with time left.
    pub sync_backlog: usize,
    /// Skeleton owners posed this frame.
    pub posed: usize,
    /// Worn objects placed by the GPU (anchors.rs): not visited by the sync.
    pub anchored: usize,
    /// Skeletons built again this frame (a shape, an attachment or their
    /// age changed), out of `posed` (AURORA_PROFILE).
    pub skeletons_rebuilt: usize,
    /// How the avatars of the last lists are drawn (AURORA_PROFILE).
    pub avatars: AvatarCounts,
}

/// How the avatars of a frame's lists are drawn (`avatar summary`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AvatarCounts {
    /// Every avatar of the scene.
    pub total: usize,
    /// The nearest ones drawn in full (RenderAvatarMaxNonImpostors), and
    /// those of them in the view.
    pub full: usize,
    pub full_in_view: usize,
    /// The farther ones drawn as impostors within the draw distance, those
    /// in the view, and those whose picture was taken this frame.
    pub impostors: usize,
    pub impostors_in_view: usize,
    pub pictured: usize,
    /// Over the complexity limit (grey silhouettes), and hidden while they
    /// load.
    pub silhouettes: usize,
    pub loading: usize,
}

/// What the worn objects of the avatars cost (`avatar summary`), read
/// every 10 s with AURORA_PROFILE by walking the objects.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AvatarSample {
    /// Objects worn by avatars (HUDs left out), and those of the avatars
    /// drawn in full.
    pub attachments: usize,
    pub attachments_full: usize,
    /// Rigged meshes worn, and their triangles at the level of detail
    /// built, for every avatar and for those drawn in full.
    pub rigged: usize,
    pub rigged_triangles: u64,
    pub rigged_triangles_full: u64,
    /// Triangles of every worn object of the avatars drawn in full.
    pub worn_triangles_full: u64,
    /// Rendering complexity: the sum over the avatars, the largest, and
    /// the avatars it is known for.
    pub complexity_total: u64,
    pub complexity_max: u32,
    pub complexity_known: usize,
    /// Animations playing on the posed skeletons, and the most on one.
    pub animations: usize,
    pub animations_max: usize,
}

pub struct Scene {
    auto_hover: auto_hover::AutoHover,
    remote_soles: auto_hover::RemoteSoles,
    foot_scope: foot_scope::Scope,
    foot_support: foot_support::FootSupport,
    sculpt_watch: Option<sculpt_watch::SculptWatch>,
    /// Optional local-only diagnostic toggle, exposed in Hover Height by
    /// AURORA_TEST_NEUTRAL_POSE, initially off. Keep evaluating the real motion controller
    /// while displaying the exact shaped rest skeleton used by auto hover.
    pub neutral_pose_test: Option<bool>,
    pub jobs: Jobs,
    job_rx: crossbeam_channel::Receiver<JobResult>,
    geoms: FastMap<GeomKey, GeomEntry>,
    flexibles: FastMap<usize, flexible::State>,
    /// Grass tufts drawn, and the terrain they were placed on (plants.rs).
    grass: HashMap<usize, plants::GrassBuilt>,
    pub textures: TextureStreamer,
    pub meshes: MeshStreamer,
    pub materials: MaterialStreamer,
    /// Legacy materials (normal / specular maps, alpha mode) by material id.
    pub legacy_mats: legacy_mat::LegacyMaterials,
    pub avatar_lib: Arc<AvatarLibrary>,
    pub gpu: Vec<ObjGpu>,
    regions: HashMap<u64, RegionGpu>,
    water: water::WaterGpu,
    water_effects: water_effects::Effects,
    /// Ban lines of the parcel we may not enter.
    pub banlines: banlines::BanLines,
    pub portals: portals::Portals,
    pub particles: particles::ParticleManager,
    /// Bundled swimming fallback for our own rendering only. Its private ID
    /// is never sent in AgentAnimation; remote viewers receive a real asset.
    pub movement_preview: Option<crate::world::PlayingAnimation>,
    /// Skeletons whose animations are paused while something they carry is
    /// selected (build/anim_pause.rs), set by the app each frame.
    pub paused_skeletons: HashSet<Uuid>,
    /// Every skeleton holds its pose: the photo mode's frozen world.
    pub pause_all_skeletons: bool,
    pub ik_edit: ik::EditRequest,
    pub poser: HashMap<Uuid, Arc<crate::ik_edit::poser::SubjectPose>>,
    /// Follow receipt clocks instead of the Legacy Viewer's activation / pause clock.
    pub synchronize_animations: bool,
    pub ground_avatar_feet: bool,
    pub ground_feet_while_moving: bool,
    pub ground_feet_max_height: f32,
    /// Verified native Movelock standing pose, not a manually requested seat.
    pub(crate) ground_feet_native_stand: bool,
    /// Current movement input/AO semantics, before simulator velocity arrives.
    pub(crate) ground_feet_own_moving: bool,
    /// Avatar rendering complexity by avatar id (complexity.rs).
    pub avatar_complexity: HashMap<Uuid, u32>,
    /// Avatars over RenderAvatarMaxComplexity: grey silhouettes, no attachments.
    pub too_complex: std::collections::HashSet<usize>,
    parcel_visibility_version: u64,
    photo_isolation_version: u64,
    /// The instant a photo drawn in several frames shows
    /// (`snapshot::Hold`): the sync places and moves what it visits at it,
    /// not at the clock's.
    pub held_at: Option<Instant>,
    /// The avatars and linksets chosen for the photo, every prim and
    /// attachment, outlined while choosing (`snapshot::isolate`).
    pub photo_marks: std::collections::HashSet<usize>,
    /// The avatar an inventory drag would give to, and what it wears,
    /// outlined like the photo's (`mark_give_target`, #991).
    give_marks: (Option<usize>, std::collections::HashSet<usize>),
    rlva_silhouettes: std::collections::HashSet<usize>,
    /// Objects the sync visits: moving, changed, pending (sync_sets.rs).
    sets: SyncSets,
    /// Transforms placed this frame (parents of the next levels).
    xf_cache: XfCache,
    /// Anchors of what the GPU places (worn objects, rigged meshes), and
    /// the joint boxes of the rigged geometries it poses.
    anchors: anchors::Anchors,
    joint_boxes: anchors::JointBoxes,
    /// Palettes posed by this frame's `update_poses`, uploaded with their
    /// anchors once the sync has placed their owners.
    posed_slots: Vec<usize>,
    touched_tmp: Vec<usize>,
    /// When each material last changed: only its own users are rebuilt.
    pub(crate) mat_changes: mat_changes::MatChanges,
    mat_changed_tmp: Vec<Uuid>,
    /// Geometries whose last reference went away, oldest first (key, frame).
    geom_zero: VecDeque<(GeomKey, u32)>,
    /// Geometries still being built or downloaded.
    geom_pending: usize,
    retired_preview_meshes: HashSet<Uuid>,
    preview_appearances: HashMap<Uuid, blender_live::Appearance>,
    preview_images: HashMap<Uuid, blender_live::ImageSlot>,
    /// Objects by base texture slot of their faces, as (slot, object):
    /// faces are classified when built, and when a slot changes alpha
    /// class only the faces using it are classified again.
    slot_users: std::collections::BTreeSet<(u32, u32)>,
    alpha_changes_tmp: Vec<u32>,
    /// Main-thread time a frame gives to the full syncs that can wait
    /// (new objects, geometry bound, faces rebuilt); the others wait in
    /// the backlog (sync_sets.rs). Set by the app from the frame time.
    pub sync_budget: std::time::Duration,
    /// Geometries freed this frame: their picking triangles are released
    /// off the main thread.
    geom_trash: jobs::Trash<Arc<GpuGeom>>,
    /// Bounds of the objects for the hover search, and its kept answer
    /// (hover.rs).
    pick: hover::PickIndex,
    pub hover: hover::HoverCache,
    /// Our own HUD objects with a geometry, in index order, as
    /// `build_huds` found them for the HUD view of the frame: what a HUD
    /// pick looks at.
    hud_own: Vec<usize>,
    /// Exact picking tests done since the hover search last took them
    /// (profile).
    exact_tests: hover::ExactTests,
    /// Slots of removed objects whose GPU state is still to be released,
    /// oldest first, and whether a slot is among them.
    removals: VecDeque<usize>,
    removal_pending: Vec<bool>,
    /// Joint boxes of the system avatar meshes, all parts merged (posed
    /// avatar spheres).
    system_bounds: Vec<animesh::JointBounds>,
    /// Objects visited by the CPU lists this frame (reused).
    visit_tmp: Vec<(usize, bool)>,
    /// Objects whose faces were built since the lists were last made: the
    /// size of their textures on screen is noted at once.
    fresh_usage: FastSet<usize>,
    /// Avatars whose bake slots shown by their attachments (`bom_mask`)
    /// may have changed: something they wear changed, arrived or left.
    bom_stale: FastSet<usize>,
    /// What the lists look for in each object slot (list_index.rs).
    list_index: list_index::ListIndex,
    /// Objects with light parameters (kept by `sync_object`, which sees every
    /// object when it arrives and on each update): the light list reads
    /// these instead of every object each frame. May hold removed indices;
    /// readers check the object again.
    pub light_objects: std::collections::BTreeSet<usize>,
    /// Texture each projector of the frame's light list holds (object
    /// index: texture, slot), see `lights.rs`.
    light_textures: HashMap<usize, (Uuid, u32)>,
    /// The light list's working buffers, empty between frames: the list is
    /// built in their room instead of new ones each frame (`lights.rs`).
    light_scratch: lights::Scratch,
    /// The projectors that cast shadows (`lights.rs`).
    spot_shadows: lights::SpotShadowSlots,
    /// Objects flagged as mirrors by their last sync, for `find_mirror`: as
    /// for the lights, it may hold removed indices and readers check `mirror`.
    mirror_objects: std::collections::BTreeSet<usize>,
    /// Build tools: selected mesh / sculpted objects drawn as wireframes
    /// (object index -> root color).
    pub selection_wire: HashMap<usize, bool>,
    /// Four cached levels held only for the mesh currently being inspected.
    inspected_mesh: Option<Uuid>,
    /// Temporary build-tool overrides, never saved or sent to the simulator.
    mesh_lod_overrides: HashMap<crate::world::objects::ObjKey, u8>,
    mesh_lod_resets: HashMap<crate::world::objects::ObjKey, mesh_inspection::LodReset>,
    /// Selected prims whose deformed silhouette is prepared with their mesh.
    pub selection_silhouettes: HashSet<usize>,
    /// Selected texture-entry bits for each prim; all 45 TE slots must fit.
    pub selection_faces: HashMap<usize, u64>,
    selection_changed: Option<Instant>,
    /// Far avatars drawn as pictures (RenderAvatarMaxNonImpostors).
    impostors: impostors::Impostors,
    /// The avatars the last lists drew as impostors, by object index.
    far_avatars: FastMap<usize, impostors::Far>,
    /// When each skeleton was last posed (an impostor keeps its pose
    /// between pictures).
    posed_at: FastMap<Uuid, Instant>,
    /// The skeletons posed in this frame, by object index.
    posed_now: FastSet<usize>,
    /// AURORA_SYNC_CHECK: impostor pictures taken, and those taken with a
    /// pose of an earlier frame (must stay 0), logged every 10 s.
    picture_check: (u64, u64, Option<Instant>),
    /// Avatars still loading (clouds).
    loading: loading::Loading,
    /// The rez effect of objects and avatars appearing (rez.rs).
    pub rez: rez::Rez,
    /// The bakes drawn: the last good one while a new one loads.
    shown_bakes: shown_bakes::ShownBakes,
    /// Hide loading avatars (shown as clouds) until they are complete.
    pub hide_loading: bool,
    /// Progress bars under the names of loading avatars.
    pub loading_bars: bool,
    /// Lights worn by avatars light the scene (LLPipeline::
    /// sRenderAttachedLights; off by the photo's light).
    pub attached_lights: bool,
    /// Surface area of the generated sculpts (attachment area limit).
    pub sculpt_area: HashMap<GeomKey, f32>,
    /// LOD triangle counts by prim shape (complexity.rs).
    prim_triangles: std::sync::Mutex<HashMap<u64, [u32; 4]>>,
    /// Attachment surface area of each avatar (m², RenderAutoMuteSurfaceAreaLimit).
    pub avatar_area: HashMap<Uuid, f32>,
    complexity_rules: u64,
    /// Debug "Show transparency": Some(rigged faces too).
    debug_alpha: Option<bool>,
    beacon_highlights: HashSet<usize>,
    pub beacon_markers: Vec<(Vec3, crate::world::beacons::Kind)>,
    land_overlays: world_overlays::LandOverlays,
    /// Dev Toolbox R3: every face tinted by the pass it is drawn in.
    pass_view: bool,
    /// Culling view of the last lists (debug overlay).
    pub last_cull: Option<CullView>,
    complexity_at: Option<Instant>,
    complexity_max: u32,
    /// Avatars (slot, id) still to refresh in this complexity round, and
    /// how many per frame (complexity.rs).
    complexity_round: Vec<(usize, Uuid)>,
    complexity_slice: usize,
    /// Avatars drawn (the nearest ones; own avatar always).
    pub max_avatars: usize,
    /// Build reflection draw lists (planar reflections enabled).
    pub reflections: bool,
    /// Camera vertical field of view (LOD scaling).
    pub fov_y: f32,
    /// Sky / water textures: (id, slot).
    env_tex: [(Uuid, u32); 6],
    /// Index of the agent avatar (always drawn).
    pub agent_idx: Option<usize>,
    /// FirstPersonAvatarVisible while in mouselook; None otherwise.
    pub first_person: Option<bool>,
    rlva_render_state: (Option<Uuid>, bool, bool),
    pub lists: DrawLists,
    pub creation_preview: creation_preview::CreationPreview,
    pub physics_shapes: physics_shapes::PhysicsShapes,
    pub stats: SceneStats,
    /// AURORA_STILL_CHECK: what changes while the picture should stand still.
    pub still: Option<still_check::StillCheck>,
    /// AURORA_SYNC_CHECK: what the sync visits, and what comes of it.
    sync_check: Option<sync_check::SyncCheck>,
    /// AURORA_SYNC_SLOW: the limit above which a frame's sync is detailed.
    sync_slow: Option<f32>,
    /// HUD attachments of our avatar and what places them besides their
    /// own updates (screen aspect, local zoom and offset): they are placed
    /// again when it changes, not at each frame.
    hud_roots: FastMap<usize, (Uuid, [f32; 5])>,
    hud_roots_tmp: Vec<(usize, Uuid, [f32; 5])>,
    /// When the last sync ran (the rounds of the detail level checks).
    last_sync: Option<Instant>,
    frame: u32,
    pub lod_factor: f32,
    pub draw_distance: f32,
    last_origin: Option<(u32, u32)>,
    blend_tmp: Vec<blend_order::BlendEntry>,
    pub anims: anim::AnimStreamer,
    object_signals: FastMap<Uuid, animesh::Signals>,
    /// World and interface sounds.
    pub sounds: sounds::SoundManager,
    /// Environment settings assets (the EEP default day).
    pub settings: settings::SettingsStreamer,
    /// Landmark asset fetches done, for `World::landmarks` (the app passes
    /// them on).
    pub landmark_results: Vec<aurora_net::FetchResult>,
    palette_slots: FastMap<Uuid, u32>,
    palette_free: Vec<u32>,
    palette_count: u32,
    palettes: Vec<[[f32; 4]; 4]>,
    pub live_morphs: live_morphs::LiveMorphs,
    /// Avatar body physics: which avatars move, and their motions.
    pub body_physics: body_physics::Config,
    physics_rig: Option<Arc<body_physics::PhysicsRig>>,
    physics: FastMap<Uuid, body_physics::BodyPhysics>,
    last_palette_gc: Instant,
    /// Periodic streaming summary in the log.
    last_diag: Instant,
    /// The renderer's staging memory, given to the geometry jobs.
    staging: Option<Arc<aurora_render::StagingPool>>,
    /// Geometry whose staged data is not copyable yet (next frame).
    geom_waiting: Vec<(GeomKey, jobs::PreparedGeom, f32)>,
    /// The other buffer of `geom_waiting`, empty: each frame the list is
    /// rebuilt into it, so neither grows again from nothing (with the
    /// system allocator, that growth queued the main thread behind the
    /// jobs during bursts of results).
    geom_waiting_spare: Vec<(GeomKey, jobs::PreparedGeom, f32)>,
    /// Streaming time of the frame by part (AURORA_PROFILE `s_*`), taken by
    /// the frame profile at the end of the frame.
    pub parts: crate::frame_profile::Parts,
    /// Rest skeleton (shape + joint offsets of worn meshes) per skeleton owner.
    skeletons: FastMap<Uuid, AvatarSkeleton>,
    /// Rigged extents per joint of each animated object (anim_box.rs).
    rig_extents: FastMap<Uuid, anim_box::RigExtents>,
    /// This frame's animated box of each animated object, which the
    /// detail level of its rigged meshes is judged by (anim_box.rs).
    anim_boxes: FastMap<Uuid, anim_box::AnimBox>,
    /// Skeleton owner of each object listed in a cached skeleton's members.
    skeleton_member_of: FastMap<usize, Uuid>,
    /// Mesh metadata generation the cached skeletons were checked against.
    skeleton_meta_gen: u64,
    /// Playing motions and held pose per skeleton owner.
    motions: FastMap<Uuid, anim::Controller>,
    /// The frame's poses (`update_poses`), empty between frames: each
    /// entry carries a controller and its physics, about a kilobyte, and
    /// the list grew again from nothing every frame.
    pose_work: Vec<PoseWork>,
    /// The skeleton owners visited by `update_poses`, same.
    pose_owners: Vec<(Uuid, u32)>,
    /// Skin bindings uploaded to the GPU: the default skeleton first (system
    /// avatar), then each rigged mesh (by asset id).
    skin_binds: skin_binds::SkinBinds,
}

/// What an object asks of its mesh's download (`MeshStreamer::need`).
#[derive(Clone, Copy)]
struct MeshNeed {
    /// Its size on screen, in pixels (`CullView::pixel_size`).
    px: f32,
    /// Worn by our avatar, or a HUD.
    own: bool,
}

/// Camera data used for culling and LOD.
#[derive(Clone, Copy)]
pub struct CullView {
    pub planes: [Vec4; 6],
    pub eye: Vec3,
    pub screen_height: f32,
    pub tan_half_fov: f32,
    /// Pixels per radian (LLDrawable::sCurPixelAngle), for the textures'
    /// sizes (`texture_importance`).
    pub pixel_angle: f32,
    /// What `texture_importance` reads of the camera: its view direction,
    /// the cosine of half its widest field (LLViewerCamera's
    /// mCosHalfCameraFOV) and whether it moves too fast for the boost.
    /// Without `with_texture_camera` nothing is important to the camera.
    pub forward: Vec3,
    pub cos_half_fov: f32,
    pub camera_fast: bool,
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
            pixel_angle: screen_height / fov_y.max(1e-3),
            forward: Vec3::ZERO,
            cos_half_fov: 1.0,
            camera_fast: true,
        }
    }

    /// The camera as `texture_importance` reads it: view direction, window
    /// aspect (the widest half field, cos(0.5 x fov x max(1, aspect)) as
    /// LLViewerCamera::updateCameraLocation) and whether it moves fast.
    pub fn with_texture_camera(mut self, forward: Vec3, aspect: f32, fast: bool) -> CullView {
        let half = (self.tan_half_fov.atan() * aspect.max(1.0)).min(std::f32::consts::PI);
        self.forward = forward.normalize_or_zero();
        self.cos_half_fov = half.cos();
        self.camera_fast = fast;
        self
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
        // Without a material, LL uses 0.33 for PASS_ALPHA_MASK (not grass's
        // 0.5): LLVolumeGeometryManager::updateGeometry, llvovolume.cpp.
        params: [f.rotation, 0.0, roughness, 0.33],
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

/// Texture animation fields of face `fi` of `o`, if it is animated.
///
/// Face selection as LLVOVolume::animateTextures: the animation's face, or
/// every face for ALL_SIDES (-1) and out-of-range faces. The animation
/// combines with the texture entry's transform (`tf`).
fn tex_anim_record(
    o: &Object,
    fi: usize,
    face_count: usize,
    tf: &TextureFace,
    pbr: bool,
    clock: aurora_render::tex_anim::AnimClock,
) -> Option<aurora_render::tex_anim::RecordAnim> {
    use aurora_render::tex_anim::{TexAnimParams, TexXform, record_anim};
    let ta = o.tex_anim?;
    if let Ok(face) = usize::try_from(ta.face)
        && face < face_count
        && face != fi
    {
        return None;
    }
    // At rate 0 Firestorm bakes the first frame into the texture entry
    // (LLVOVolume::animateTextures setTEOffset / setTEScale / setTERotation),
    // which PBR faces ignore: they show no animation.
    if pbr && ta.rate == 0.0 {
        return None;
    }
    let params = TexAnimParams {
        mode: ta.mode,
        size_x: ta.size_x,
        size_y: ta.size_y,
        start: ta.start,
        length: ta.length,
        rate: ta.rate,
    };
    let te = TexXform {
        rot: tf.rotation,
        scale: [tf.scale_s, tf.scale_t],
        offset: [tf.offset_s, tf.offset_t],
    };
    record_anim(&params, clock.ms(o.tex_anim_clock.start), o.tex_anim_clock.phase, &te)
}

impl Scene {
    /// LLViewHighlightTransparent / LLPipeline::rebuildDrawInfo: invisible
    /// faces need temporary records while the transparency overlay is on.
    /// See indra/newview/llviewermenu.cpp and pipeline.cpp (LGPL 2.1).
    pub fn set_debug_alpha(&mut self, world: &mut World, debug_alpha: Option<bool>) {
        if self.debug_alpha.is_some() != debug_alpha.is_some() {
            for (idx, slot) in world.objects.slots.iter_mut().enumerate() {
                let Some(object) = slot else { continue };
                if object.is_avatar() || !(0..aurora_prim::te::MAX_TES).any(|face| te_invisible(object, face)) {
                    continue;
                }
                object.material_dirty = true;
                object.render.needs_records = true;
                self.sets.dirty.insert(idx);
            }
        }
        self.debug_alpha = debug_alpha;
    }

    /// Dev Toolbox P4 "What changes when still" (AURORA_STILL_CHECK).
    pub fn set_still_check(&mut self, on: bool) {
        if on != self.still.is_some() {
            self.still = on.then(still_check::StillCheck::start);
        }
    }

    /// Dev Toolbox P5 "What the sync visits" (AURORA_SYNC_CHECK).
    pub fn set_sync_check(&mut self, on: bool) {
        if on != self.sync_check.is_some() {
            self.sync_check = on.then(sync_check::SyncCheck::start);
        }
    }

    pub fn sync_check_on(&self) -> bool {
        self.sync_check.is_some()
    }

    /// Dev Toolbox P3 "Slow frames" (AURORA_SYNC_SLOW): the limit in ms.
    pub fn set_slow_sync(&mut self, limit_ms: Option<f32>) {
        self.sync_slow = limit_ms;
    }

    pub fn slow_sync(&self) -> Option<f32> {
        self.sync_slow
    }

    /// Dev Toolbox O9 "Follow sculpts" (AURORA_SCULPT_WATCH) of an avatar.
    pub fn set_sculpt_watch(&mut self, target: Option<Uuid>) {
        if target != self.sculpt_watch.as_ref().map(|w| w.target()) {
            self.sculpt_watch = target.map(sculpt_watch::SculptWatch::new);
        }
    }

    pub fn sculpt_watch_target(&self) -> Option<Uuid> {
        self.sculpt_watch.as_ref().map(|w| w.target())
    }

    /// Dev Toolbox R3 "Pass and alpha per face": opaque faces in green,
    /// masked in blue, blended in red, fully transparent ones (built for it,
    /// like the transparency overlay builds them) in yellow.
    pub fn set_pass_view(&mut self, world: &mut World, on: bool) {
        if self.pass_view == on {
            return;
        }
        let invisible_built = self.debug_alpha.is_some() || self.pass_view;
        self.pass_view = on;
        if invisible_built != (self.debug_alpha.is_some() || on) {
            for (idx, slot) in world.objects.slots.iter_mut().enumerate() {
                let Some(object) = slot else { continue };
                if object.is_avatar() || !(0..aurora_prim::te::MAX_TES).any(|face| te_invisible(object, face)) {
                    continue;
                }
                object.material_dirty = true;
                object.render.needs_records = true;
                self.sets.dirty.insert(idx);
            }
        }
    }

    /// Invisible selected meshes need records for their wireframe, including
    /// while dragging. They retain Pass::Hidden in all ordinary world passes.
    pub fn set_wire_selection(&mut self, world: &mut World, selected: HashMap<usize, bool>) {
        if self.selection_wire == selected {
            return;
        }
        for &idx in self.selection_wire.keys().chain(selected.keys()) {
            let changed = self.selection_wire.contains_key(&idx) != selected.contains_key(&idx);
            if changed
                && let Some(object) = world.objects.get_mut(idx)
                && (0..aurora_prim::te::MAX_TES).any(|face| te_invisible(object, face))
            {
                object.material_dirty = true;
                object.render.needs_records = true;
            }
        }
        self.selection_wire = selected;
    }

    /// Invisible selected faces get temporary records for their overlay only.
    /// Ordinary transparent faces keep their zero-record path. Changing an
    /// opaque face selection never rebuilds its geometry or material records.
    pub fn set_face_selection(&mut self, world: &mut World, selected: HashMap<usize, u64>) {
        if self.selection_faces == selected {
            return;
        }
        self.selection_changed = Some(Instant::now());
        for &idx in self.selection_faces.keys().chain(selected.keys()) {
            let changed = self.selection_faces.get(&idx).copied().unwrap_or(0) ^ selected.get(&idx).copied().unwrap_or(0);
            let invisible = changed != 0
                && world.objects.get(idx).is_some_and(|object| {
                    (0..aurora_prim::te::MAX_TES)
                        .any(|face| changed & 1u64.checked_shl(face as u32).unwrap_or(0) != 0 && te_invisible(object, face))
                });
            if invisible && let Some(object) = world.objects.get_mut(idx) {
                // Face allocations changed: a placement update alone cannot
                // restore a previously omitted transparent face's record.
                object.material_dirty = true;
                object.render.needs_records = true;
            }
        }
        self.selection_faces = selected;
    }

    pub fn reveal_selected_faces(&mut self) {
        if !self.selection_faces.is_empty() {
            self.selection_changed = Some(Instant::now());
        }
    }

    pub fn selection_pulse(&self, now: Instant) -> f32 {
        if self.selection_faces.is_empty() {
            return 0.0;
        }
        self.selection_changed.map_or(0.0, |at| {
            let remaining = (1.0 - now.saturating_duration_since(at).as_secs_f32() / 0.7).clamp(0.0, 1.0);
            0.18 * remaining * remaining
        })
    }

    pub fn new(cache_dir: std::path::PathBuf, avatar_lib: Arc<AvatarLibrary>) -> Scene {
        let (jobs, job_rx) = Jobs::new();
        let cache_dir_anim = cache_dir.clone();
        let live: Vec<i32> = body_physics::driven_ids()
            .chain(default_motions::emote_params())
            .chain(default_motions::HAND_PARAMS[1..].iter().copied())
            .collect();
        let lips = live_morphs::LiveMorphs::new(&avatar_lib, &live);
        let mut anims = anim::AnimStreamer::new(cache_dir_anim.clone());
        default_motions::register(&mut anims, &avatar_lib.rig);
        let physics_rig = body_physics::PhysicsRig::new(&avatar_lib.shape_params, &avatar_lib.rig).map(Arc::new);
        Scene {
            jobs,
            job_rx,
            sculpt_watch: sculpt_watch::SculptWatch::from_env(),
            geoms: Default::default(),
            flexibles: Default::default(),
            grass: HashMap::new(),
            textures: TextureStreamer::new(cache_dir.clone()),
            meshes: MeshStreamer::new(cache_dir.clone()),
            materials: MaterialStreamer::new(cache_dir),
            legacy_mats: Default::default(),
            avatar_lib,
            // (room for a busy region: the first doublings of a growing
            // `Vec` would copy megabytes in the middle of an arrival)
            gpu: prefaulted_table(32_768),
            regions: HashMap::new(),
            water: water::WaterGpu::default(),
            water_effects: Default::default(),
            banlines: banlines::BanLines::default(),
            portals: portals::Portals::default(),
            particles: particles::ParticleManager::default(),
            movement_preview: None,
            paused_skeletons: HashSet::new(),
            pause_all_skeletons: false,
            ik_edit: ik::EditRequest::default(),
            poser: HashMap::new(),
            synchronize_animations: false,
            ground_avatar_feet: false,
            ground_feet_while_moving: false,
            ground_feet_max_height: crate::settings::DEFAULT_GROUND_FEET_HEIGHT,
            ground_feet_native_stand: false,
            ground_feet_own_moving: false,
            avatar_complexity: HashMap::new(),
            too_complex: Default::default(),
            parcel_visibility_version: 0,
            photo_isolation_version: 0,
            held_at: None,
            photo_marks: Default::default(),
            give_marks: Default::default(),
            rlva_silhouettes: Default::default(),
            sets: SyncSets::default(),
            xf_cache: XfCache::default(),
            anchors: Default::default(),
            joint_boxes: Default::default(),
            posed_slots: Vec::new(),
            touched_tmp: Vec::new(),
            mat_changes: mat_changes::MatChanges::default(),
            mat_changed_tmp: Vec::new(),
            geom_zero: VecDeque::new(),
            geom_pending: 0,
            retired_preview_meshes: HashSet::new(),
            preview_appearances: HashMap::new(),
            preview_images: HashMap::new(),
            slot_users: Default::default(),
            alpha_changes_tmp: Vec::new(),
            sync_budget: std::time::Duration::from_millis(1),
            geom_trash: Default::default(),
            pick: Default::default(),
            hover: Default::default(),
            hud_own: Vec::new(),
            exact_tests: Default::default(),
            removals: VecDeque::new(),
            removal_pending: Vec::new(),
            system_bounds: Vec::new(),
            visit_tmp: Vec::new(),
            fresh_usage: Default::default(),
            bom_stale: Default::default(),
            list_index: Default::default(),
            light_objects: Default::default(),
            light_textures: HashMap::new(),
            light_scratch: Default::default(),
            spot_shadows: Default::default(),
            mirror_objects: Default::default(),
            selection_wire: Default::default(),
            inspected_mesh: None,
            mesh_lod_overrides: HashMap::new(),
            mesh_lod_resets: HashMap::new(),
            selection_silhouettes: Default::default(),
            selection_faces: Default::default(),
            selection_changed: None,
            debug_alpha: None,
            beacon_highlights: Default::default(),
            beacon_markers: Vec::new(),
            land_overlays: Default::default(),
            pass_view: false,
            last_cull: None,
            impostors: Default::default(),
            far_avatars: Default::default(),
            posed_at: Default::default(),
            posed_now: Default::default(),
            picture_check: (0, 0, None),
            loading: Default::default(),
            rez: Default::default(),
            shown_bakes: Default::default(),
            hide_loading: true,
            loading_bars: true,
            attached_lights: true,
            sculpt_area: HashMap::new(),
            prim_triangles: Default::default(),
            avatar_area: HashMap::new(),
            complexity_rules: 0,
            complexity_at: None,
            complexity_max: 0,
            complexity_round: Vec::new(),
            complexity_slice: 0,
            max_avatars: 16,
            reflections: true,
            fov_y: 1.0,
            env_tex: [(Uuid::nil(), 0); 6],
            agent_idx: None,
            first_person: None,
            rlva_render_state: (None, false, false),
            lists: DrawLists::default(),
            creation_preview: creation_preview::CreationPreview::default(),
            physics_shapes: physics_shapes::PhysicsShapes::default(),
            stats: SceneStats::default(),
            still: still_check::StillCheck::from_env(),
            sync_check: sync_check::SyncCheck::from_env(),
            sync_slow: sync_slow::SlowSync::limit_from_env(),
            hud_roots: Default::default(),
            hud_roots_tmp: Vec::new(),
            last_sync: None,
            frame: 0,
            lod_factor: 2.0,
            draw_distance: 96.0,
            last_origin: None,
            blend_tmp: Vec::new(),
            sounds: sounds::SoundManager::new(cache_dir_anim.clone()),
            settings: settings::SettingsStreamer::new(cache_dir_anim.clone()),
            landmark_results: Vec::new(),
            anims,
            object_signals: Default::default(),
            palette_slots: Default::default(),
            palette_free: Vec::new(),
            palette_count: lips.slots,
            palettes: lips.matrices.clone(),
            live_morphs: lips,
            body_physics: body_physics::Config::default(),
            physics_rig,
            physics: Default::default(),
            last_palette_gc: Instant::now(),
            last_diag: Instant::now(),
            staging: None,
            geom_waiting: Vec::new(),
            geom_waiting_spare: Vec::new(),
            parts: Default::default(),
            skeletons: Default::default(),
            rig_extents: Default::default(),
            anim_boxes: Default::default(),
            skeleton_member_of: Default::default(),
            skeleton_meta_gen: 0,
            motions: Default::default(),
            pose_work: Vec::new(),
            pose_owners: Vec::new(),
            skin_binds: Default::default(),
            auto_hover: Default::default(),
            remote_soles: Default::default(),
            foot_scope: foot_scope::Scope::from_environment(),
            foot_support: Default::default(),
            neutral_pose_test: (std::env::var("AURORA_TEST_NEUTRAL_POSE").as_deref() == Ok("1")).then_some(false),
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
        }
    }

    fn geom_unref(&mut self, key: GeomKey) {
        if let Some(e) = self.geoms.get_mut(&key)
            && e.refs > 0
        {
            e.refs -= 1;
            if e.refs == 0 {
                e.zero_since = self.frame;
                self.geom_zero.push_back((key, self.frame));
            }
        }
    }

    /// A new geometry entry, unreferenced until an object binds it.
    fn geom_insert(&mut self, key: GeomKey, state: GeomState) {
        if matches!(state, GeomState::Pending) {
            self.geom_pending += 1;
        }
        let old = self.geoms.insert(
            key,
            GeomEntry {
                state,
                refs: 0,
                zero_since: self.frame,
            },
        );
        if old.is_some_and(|e| matches!(e.state, GeomState::Pending)) {
            self.geom_pending = self.geom_pending.saturating_sub(1);
        }
        self.geom_zero.push_back((key, self.frame));
    }

    /// Free the geometries nobody used for `GEOM_KEEP_FRAMES` frames: only
    /// the entries whose last reference went away that long ago are looked
    /// at, not every geometry, and at most `GEOM_FREED_PER_FRAME` a frame
    /// (those of a region left by teleport all come due in the same frame).
    fn geom_gc(&mut self, renderer: &mut Renderer) {
        self.collect_preview_meshes(renderer);
        let mut looked = 0;
        while let Some(&(key, since)) = self.geom_zero.front() {
            if self.frame.wrapping_sub(since) <= GEOM_KEEP_FRAMES || looked >= GEOM_FREED_PER_FRAME {
                break;
            }
            looked += 1;
            self.geom_zero.pop_front();
            // referenced again since (or released again later: a later entry)
            if !self.geoms.get(&key).is_some_and(|e| e.refs == 0 && e.zero_since == since) {
                continue;
            }
            match self.geoms.remove(&key).map(|e| e.state) {
                Some(GeomState::Ready(g)) => {
                    for f in g.faces.iter().flatten() {
                        renderer.free_mesh(*f);
                    }
                    self.joint_boxes.release(&key);
                    self.geom_trash.push(g);
                }
                Some(GeomState::Pending) => self.geom_pending = self.geom_pending.saturating_sub(1),
                _ => {}
            }
            // (what waited for it asks for it again)
            self.sets.geometry_changed(sync_plan::wait_token(&key));
            // the skin bindings go with the last detail level of the mesh:
            // an object drawn with them holds one of its geometries
            if let GeomKey::Mesh { id, .. } = key
                && !(0..4).any(|lod| self.geoms.contains_key(&GeomKey::Mesh { id, lod }))
            {
                self.skin_binds.release(&id);
                // no object wants it any more: a download still waiting goes
                self.meshes.unwant(&id);
            }
        }
        self.geom_trash.empty(&self.jobs);
        debug_assert!(
            !self.frame.is_multiple_of(600)
                || self.geom_pending == self.geoms.values().filter(|e| matches!(e.state, GeomState::Pending)).count(),
            "pending geometry count out of step"
        );
    }

    /// Ensure a geometry is (being) built. Returns true when ready.
    fn request_geom(&mut self, key: GeomKey, obj: &Object, need: MeshNeed) -> bool {
        if let Some(e) = self.geoms.get(&key) {
            if let (GeomKey::Mesh { id, .. }, GeomState::Pending) = (key, &e.state) {
                // (another user of a mesh still downloading: it may be nearer)
                self.meshes.need(id, need.px, need.own);
            }
            return matches!(e.state, GeomState::Ready(_));
        }
        match key {
            GeomKey::Prim { lod, .. } => {
                let params = obj.volume;
                let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize % 4];
                let pool = self.staging.clone();
                self.jobs.spawn(move || {
                    let mesh = aurora_prim::generate_volume(&params, detail);
                    JobResult::geometry(key, jobs::volume_to_faces(&mesh), 1.0, pool.as_deref())
                });
            }
            GeomKey::Flexible { lod, .. } => {
                let params = obj.volume;
                let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize % 4];
                let pool = self.staging.clone();
                self.jobs.spawn(move || {
                    let sim = aurora_prim::flexible::Flexible::new(0, 1.0, Vec3::ZERO, Quat::IDENTITY);
                    let mesh =
                        aurora_prim::volume::generate_flexible_volume(&params, detail, &sim.path(Vec3::ZERO, Quat::IDENTITY), Vec3::ONE);
                    JobResult::geometry(key, jobs::volume_to_faces(&mesh), 1.0, pool.as_deref())
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
                let pool = self.staging.clone();
                self.jobs.spawn(move || {
                    let mesh = aurora_prim::generate_sculpt(&params, detail, map.width, map.height, map.components, &map.pixels);
                    JobResult::geometry(key, jobs::volume_to_faces(&mesh), mesh.surface_area, pool.as_deref())
                });
            }
            GeomKey::SculptPlaceholder { lod, .. } => {
                let params = obj.volume;
                let detail = aurora_prim::volume::DETAIL_SCALES[lod as usize % 4];
                let pool = self.staging.clone();
                self.jobs.spawn(move || {
                    let mesh = aurora_prim::sculpt_placeholder(&params, detail);
                    JobResult::geometry(key, jobs::volume_to_faces(&mesh), mesh.surface_area, pool.as_deref())
                });
            }
            GeomKey::Mesh { id, lod } => {
                self.meshes.want(id, lod);
                self.meshes.need(id, need.px, need.own);
            }
            GeomKey::Tree { species, lod } => {
                let pool = self.staging.clone();
                self.jobs.spawn(move || {
                    let s = aurora_prim::tree::tree_species(Some(species));
                    let mesh = aurora_prim::tree::generate_tree(s, plants::tree_lod(lod));
                    JobResult::geometry(key, jobs::volume_to_faces(&mesh), 1.0, pool.as_deref())
                });
            }
            // (built at once by `request_grass`, which needs the ground)
            GeomKey::Grass { .. } => return false,
            // (uploaded by `ensure_avatar_parts`, built by `avatar_morph`)
            GeomKey::AvatarPart(_) | GeomKey::AvatarMorph { .. } => {}
        }
        self.geom_insert(key, GeomState::Pending);
        false
    }

    /// The geometry of a system body part morphed by an avatar's shape
    /// (`weights`: the effective weight of every visual parameter), built by
    /// a job when no avatar had these weights yet. None when no morph
    /// applies to the part.
    fn avatar_morph(&mut self, part: u8, weights: &HashMap<i32, f32>) -> Option<GeomKey> {
        let lib = self.avatar_lib.clone();
        let (w, hash) = morph::part_weights(lib.parts.get(part as usize)?, weights)?;
        let key = GeomKey::AvatarMorph { part, weights: hash };
        if !self.geoms.contains_key(&key) {
            let pool = self.staging.clone();
            self.jobs.spawn(move || match lib.parts.get(part as usize) {
                Some(p) => {
                    let (face, min, max) = morph::morphed(p, &w);
                    JobResult::geometry(key, (vec![Some(face)], min, max), 1.0, pool.as_deref())
                }
                None => JobResult::GeometryFailed { key },
            });
            self.geom_insert(key, GeomState::Pending);
        }
        Some(key)
    }

    /// Put a prepared geometry in the arena: staged faces are copied by the
    /// GPU (their regions must be ready), the others written from here.
    fn upload_geom(&mut self, renderer: &mut Renderer, key: GeomKey, geom: jobs::PreparedGeom) {
        let gfaces = geom
            .faces
            .into_iter()
            .map(|f| match f? {
                jobs::FaceUpload::Staged(staged) => renderer.upload_mesh_staged(&staged),
                jobs::FaceUpload::Memory(g) => match &g.skin {
                    Some(sk) => renderer.upload_skinned_mesh(&g.vertices, sk, &g.indices),
                    None => renderer.upload_mesh(&g.vertices, &g.indices),
                },
            })
            .collect();
        let g = Arc::new(GpuGeom {
            faces: gfaces,
            min: geom.min,
            max: geom.max,
            joint_bounds: geom.joint_bounds,
            pick_faces: geom.pick_faces,
        });
        match self.geoms.get_mut(&key) {
            Some(e) => match std::mem::replace(&mut e.state, GeomState::Ready(g)) {
                GeomState::Ready(old) => {
                    for f in old.faces.iter().flatten() {
                        renderer.free_mesh(*f);
                    }
                    self.joint_boxes.release(&key);
                    self.geom_trash.push(old);
                }
                GeomState::Pending => self.geom_pending = self.geom_pending.saturating_sub(1),
                GeomState::Failed => {}
            },
            None => self.geom_insert(key, GeomState::Ready(g)),
        }
        self.sets.geometry_changed(sync_plan::wait_token(&key));
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
            let faces = vec![Some(jobs::FaceGeom {
                vertices: part.vertices.clone(),
                indices: part.indices.clone(),
                skin,
            })];
            self.upload_geom(renderer, key, jobs::PreparedGeom::new(faces, min, max, false, None));
            if let Some(e) = self.geoms.get_mut(&key) {
                e.refs = u32::MAX / 2; // permanent
            }
        }
    }

    // ------------------------------------------------------------ results

    pub fn process_results(&mut self, renderer: &mut Renderer, net: &NetClient, budget: std::time::Duration) {
        use crate::frame_profile::Part;
        let t0 = Instant::now();
        // what the jobs staged so far becomes copyable (once a frame, before
        // the geometry here and the textures in `stream`)
        renderer.prepare_uploads();
        if self.staging.is_none() {
            let pool = renderer.staging_pool();
            self.textures.staging = Some(pool.clone());
            // (AURORA_TEXTURE_BC7=0: every texture stays RGBA8, to compare)
            let wanted = std::env::var("AURORA_TEXTURE_BC7").map_or(true, |v| v != "0");
            self.textures.bc7 = wanted && renderer.textures.bc7();
            // (AURORA_TEXTURE_BC7_CACHE=0: BC7 without its disk cache, every
            // texture decoded from its JPEG 2000 data as before)
            let cached = std::env::var("AURORA_TEXTURE_BC7_CACHE").map_or(true, |v| v != "0");
            self.textures.bc7_cache = self.textures.bc7 && cached;
            log::info!(
                "textures: BC7 compression {} (GPU support: {})",
                if self.textures.bc7 { "on" } else { "off" },
                renderer.textures.bc7()
            );
            self.meshes.staging = Some(pool.clone());
            self.staging = Some(pool);
        }
        // Three kinds of results share the budget, each with a part of it so
        // that none starves the others, and each at least one a frame:
        // geometry whose staging chunk became copyable (the first half),
        // downloads (up to three quarters), finished jobs (the rest).
        // Before, only the jobs were bounded: a chunk that became copyable
        // put all its geometry in the arena in the same frame.
        let mut placed = 0;
        let mut waiting = std::mem::replace(&mut self.geom_waiting, std::mem::take(&mut self.geom_waiting_spare));
        for (key, geom, area) in waiting.drain(..) {
            if geom.ready() && (placed == 0 || t0.elapsed() < budget / 2) {
                placed += 1;
                self.place_geometry(renderer, key, geom, area);
            } else {
                self.geom_waiting.push((key, geom, area));
            }
        }
        self.geom_waiting_spare = waiting;
        let t = self.parts.lap(Part::Geometry, t0);
        let mut fetched = 0;
        while fetched == 0 || t0.elapsed() < budget * 3 / 4 {
            let Ok(r) = net.fetch_results.try_recv() else {
                break;
            };
            fetched += 1;
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
            } else if kind == settings::FETCH_KIND_SETTINGS {
                self.settings.on_fetch(r, &self.jobs);
            } else if kind == crate::world::landmarks::FETCH_KIND_LANDMARK {
                self.landmark_results.push(r);
            }
        }
        let mut t = self.parts.lap(Part::Fetched, t);
        let mut jobs_done = 0;
        while jobs_done == 0 || t0.elapsed() < budget {
            let Ok(r) = self.job_rx.try_recv() else {
                break;
            };
            jobs_done += 1;
            let part = if matches!(r, JobResult::Geometry { .. }) {
                Part::Geometry
            } else {
                Part::Decoded
            };
            match r {
                JobResult::Geometry { key, geom, area } => self.finish_geometry(renderer, key, geom, area),
                JobResult::GeometryFailed { key } => {
                    if let GeomKey::Mesh { id, lod } = key {
                        self.meshes.on_geometry_done(&id, lod);
                    }
                    if let Some(e) = self.geoms.get_mut(&key)
                        && matches!(std::mem::replace(&mut e.state, GeomState::Failed), GeomState::Pending)
                    {
                        self.geom_pending = self.geom_pending.saturating_sub(1);
                    }
                }
                JobResult::MeshCache { id, arrival } => self.meshes.on_cache(id, arrival.map(|a| *a)),
                JobResult::MeshPart { id, arrival } => self.meshes.on_part(id, *arrival, &self.jobs),
                JobResult::Material { id, material } => self.materials.on_material(id, material),
                JobResult::Done => {}
                other => self.textures.on_job(other),
            }
            t = self.parts.lap(part, t);
        }
        self.geom_trash.empty(&self.jobs);
    }

    /// A geometry built by a job goes to the GPU, or waits for a coming
    /// frame while another job still writes in its staging chunk.
    fn finish_geometry(&mut self, renderer: &mut Renderer, key: GeomKey, geom: jobs::PreparedGeom, area: f32) {
        if geom.ready() {
            self.place_geometry(renderer, key, geom, area);
        } else {
            self.geom_waiting.push((key, geom, area));
        }
    }

    /// Put a geometry whose staged data is copyable in the arena.
    fn place_geometry(&mut self, renderer: &mut Renderer, key: GeomKey, geom: jobs::PreparedGeom, area: f32) {
        if let GeomKey::Mesh { id, lod } = key {
            self.meshes.on_geometry_done(&id, lod);
        }
        if matches!(key, GeomKey::Sculpt { .. }) {
            self.sculpt_area.insert(key, area);
        }
        self.upload_geom(renderer, key, geom);
    }

    // ------------------------------------------------------------ objects

    /// Free the draw records of an object's faces.
    fn drop_faces(&mut self, renderer: &mut Renderer, idx: usize, g: &mut ObjGpu) {
        for f in g.faces.drain(..) {
            renderer.records.free(f.record);
            self.slot_users.remove(&(f.base_slot, idx as u32));
        }
    }

    fn release_obj(&mut self, renderer: &mut Renderer, idx: usize, g: &mut ObjGpu) {
        self.flexibles.remove(&idx);
        self.rez.forget(idx);
        self.drop_faces(renderer, idx, g);
        for t in g.tex_ids.drain(..) {
            self.textures.release(&t);
        }
        if let Some(k) = g.geom.take() {
            self.geom_unref(k);
        }
        for k in g.morph_geoms.drain(..) {
            self.geom_unref(k);
        }
        g.morph_pending = false;
        g.wanted_geom = None;
        g.material_ids.clear();
    }

    /// Free GPU state for removed objects.
    ///
    /// Within `budget`: a region going away removes thousands of objects in
    /// one frame (2.7 ms for 6,000). The rest waits its turn, still drawn
    /// for a few frames; a slot taken again by a new object is released
    /// before that object is synced (`release_slot`).
    pub fn collect_garbage(&mut self, renderer: &mut Renderer, store: &mut ObjectStore, budget: std::time::Duration) {
        store.graveyard.clear();
        let now = Instant::now();
        // (`sync` gave the derez its kills already; no derez without the world)
        store.killed.clear();
        for i in std::mem::take(&mut store.removed_slots) {
            if self.removal_pending.len() <= i {
                self.removal_pending.resize(i + 1, false);
            }
            // (removed, taken again and removed again before its turn: once)
            if !std::mem::replace(&mut self.removal_pending[i], true) {
                self.removals.push_back(i);
            }
        }
        if self.removals.is_empty() {
            return;
        }
        let deadline = Instant::now() + budget;
        let mut done = 0u32;
        // (what a derez still draws waits for its end: rez.rs)
        let mut held = Vec::new();
        while let Some(i) = self.removals.pop_front() {
            if self.rez.holds(i, now) {
                held.push(i);
                continue;
            }
            self.release_slot(renderer, i);
            done += 1;
            // (the clock read every few objects: each costs under a microsecond)
            if done.is_multiple_of(16) && Instant::now() >= deadline {
                break;
            }
        }
        self.removals.extend(held);
    }

    /// Release the GPU state left in slot `i` by a removed object, if it
    /// still waits for it: a slot reused since holds the old object's
    /// state, and the new object is built from scratch.
    fn release_slot(&mut self, renderer: &mut Renderer, i: usize) {
        if !self.removal_pending.get(i).copied().unwrap_or(false) {
            return;
        }
        self.removal_pending[i] = false;
        self.sets.forget(i);
        self.anchors.forget(i);
        self.pick.remove(i);
        self.list_index.remove(i);
        self.bom_stale.remove(&i);
        if let Some(mut g) = self.gpu.get_mut(i).map(std::mem::take) {
            // (something worn left: its wearer's bake slots)
            if let Some(wearer) = g.owner_avatar {
                self.bom_stale.insert(wearer);
            }
            self.release_obj(renderer, i, &mut g);
        }
        renderer.cull.objects.set(i, CullObject::NONE);
    }

    /// Release everything (logout / teleport cleanup).
    pub fn clear(&mut self, renderer: &mut Renderer) {
        self.remote_soles = Default::default();
        self.foot_scope.clear();
        self.release_foot_support();
        self.flexibles.clear();
        let mut gpus = std::mem::take(&mut self.gpu);
        for (idx, g) in gpus.iter_mut().enumerate() {
            self.release_obj(renderer, idx, g);
        }
        // (emptied, with the room it had)
        gpus.clear();
        self.gpu = gpus;
        self.removals.clear();
        self.removal_pending.clear();
        self.slot_users.clear();
        self.pick.reset();
        self.list_index.reset();
        renderer.cull.objects.reset();
        for (_, r) in self.regions.drain() {
            for (_, c) in r.chunks {
                renderer.free_mesh(c.alloc);
                renderer.records.free(c.record);
            }
            r.surface.free(renderer, &mut self.textures);
        }
        self.water.clear(renderer);
        self.banlines.clear(renderer);
        self.portals.clear(renderer);
        self.creation_preview.clear(renderer);
        self.physics_shapes.clear(renderer);
        self.land_overlays.clear(renderer);
        self.particles.clear(&mut self.textures);
        // (our avatar is a first appearance again at the next login)
        self.loading = Default::default();
        self.last_origin = None;
        self.sets.clear();
        self.anchors.clear();
        self.hud_roots.clear();
        self.give_marks = Default::default();
        self.posed_slots.clear();
        self.palette_slots.clear();
        self.motions.clear();
        self.palette_free.clear();
        self.palette_count = self.live_morphs.slots;
        self.live_morphs.uploaded = false;
    }

    /// The nearest avatar the ray `(origin, dir)` meets before `far`, and
    /// where: the hover search leaves avatars out (the cursor's actions are
    /// the objects'), the world's hover tips name them. Each avatar is the
    /// upright capsule of `pick_at` narrowed to the body: `pick_at` tests a
    /// point already on the avatar, a ray would meet that one in the air
    /// around the head and shoulders.
    pub fn avatar_on_ray(&self, world: &World, (origin, dir): (Vec3, Vec3), far: f32) -> Option<(usize, Vec3)> {
        const RADIUS: f32 = 0.3;
        let mut best: Option<(f32, usize)> = None;
        for (idx, o) in world.objects.avatars() {
            // not ours: the camera behind it keeps it under the pointer
            if o.full_id == world.agent_id {
                continue;
            }
            let Some(g) = self.gpu.get(idx).filter(|g| g.is_avatar && !g.faces.is_empty()) else {
                continue;
            };
            let Some(t) = ray_capsule(origin, dir, g.center, RADIUS, -1.1, 0.95) else {
                continue;
            };
            if t <= far && best.is_none_or(|(b, _)| t < b) {
                best = Some((t, idx));
            }
        }
        best.map(|(t, idx)| (idx, origin + dir * t))
    }

    /// What is at a picked world point: an avatar (capsule test) or the most
    /// specific object whose oriented box contains the point.
    pub fn pick_at(&self, world: &World, point: Vec3, now: Instant) -> Option<usize> {
        self.pick_at_filtered(world, point, now, false)
    }

    fn pick_at_filtered(&self, world: &World, point: Vec3, now: Instant, skip_ignored: bool) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (idx, g) in self.gpu.iter().enumerate() {
            if g.faces.is_empty() || g.hud {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            if skip_ignored && o.click_action == crate::interaction::code::IGNORE {
                continue;
            }
            if g.is_avatar {
                let rel = point - g.center;
                if Vec3::new(rel.x, rel.y, 0.0).length() < 0.6 && (-1.25..=1.15).contains(&rel.z) {
                    return Some(idx);
                }
                continue;
            }
            if g.owner_avatar.is_some() && o.visual_volume().is_mesh() {
                continue; // rigged / worn mesh: the avatar capsule handles it
            }
            let scale = o.visual_scale_at(now).max(Vec3::splat(0.01));
            // the geometry centre lies in the prim box: farther than the box
            // diagonal plus the tolerance, the point cannot be inside
            if !g.rigged && g.radius > 0.0 && (point - g.center).length() > scale.length() + 0.2 {
                continue;
            }
            let Some((pos, rot, _)) = Self::object_transform(world, idx, now, 0) else {
                continue;
            };
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
    pub fn env_textures(&mut self, renderer: &mut Renderer, ids: [Uuid; 6]) -> [u32; 6] {
        let mut out = [0u32; 6];
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
                self.textures.note_usage_fixed(id, 2048.0);
                if self.textures.is_loaded(id) || self.textures.is_local(id) {
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
        // in index order: of two mirrors at the same distance, the first wins
        for &idx in &self.mirror_objects {
            if !self.gpu.get(idx).is_some_and(|g| g.mirror) {
                continue;
            }
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let Some((pos, rot, hud)) = Self::object_transform(world, idx, now, 0) else {
                continue;
            };
            let d = pos.distance(view.eye);
            if hud || d > self.draw_distance || !view.sphere_visible(pos, (o.visual_scale_at(now) * 0.5).length()) {
                continue;
            }
            let normal = rot * Vec3::Z;
            if normal.dot(view.eye - pos) < 0.0 {
                continue;
            }
            if best.as_ref().is_some_and(|(bd, _)| *bd <= d) {
                continue;
            }
            let to_box = Mat4::from_scale_rotation_translation(o.visual_scale_at(now).max(Vec3::splat(0.01)), rot, pos).inverse();
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

    fn object_geom_key(o: &Object, lod: u8) -> Option<GeomKey> {
        use aurora_prim::params::{LL_PCODE_LEGACY_GRASS, LL_PCODE_LEGACY_TREE};
        // (LL_PCODE_TREE_NEW objects are never created by the Legacy
        // Viewer: LLViewerObject::createObject returns none for them)
        match o.pcode {
            LL_PCODE_LEGACY_TREE => {
                let species = o
                    .tree_species
                    .filter(|&s| (s as usize) < aurora_prim::tree::TREE_SPECIES.len())
                    .unwrap_or(0);
                return Some(GeomKey::Tree { species, lod: lod.min(3) });
            }
            LL_PCODE_LEGACY_GRASS => {
                return Some(GeomKey::Grass {
                    id: o.full_id,
                    hash: plants::grass_hash(o),
                });
            }
            _ => {}
        }
        if o.is_avatar() || o.is_tree() || o.pcode != aurora_prim::params::LL_PCODE_VOLUME {
            return None;
        }
        if o.visual_volume().is_mesh() {
            return o.visual_volume().sculpt.map(|s| GeomKey::Mesh { id: s.texture, lod });
        }
        if o.is_flexible() {
            return Some(GeomKey::Flexible {
                id: o.full_id,
                hash: o.visual_volume().cache_key(),
                lod,
            });
        }
        if o.visual_volume().is_sculpt() {
            return Some(GeomKey::Sculpt {
                hash: o.visual_volume().cache_key(),
                lod,
            });
        }
        Some(GeomKey::Prim {
            hash: o.visual_volume().cache_key(),
            lod,
        })
    }

    /// World (render-space) transform of an object, following parents. For
    /// an avatar: its root (pelvis) position, shape offset included.
    pub fn object_transform(world: &World, idx: usize, now: Instant, depth: u32) -> Option<Xf> {
        Self::object_transform_in(world, idx, now, depth, None)
    }

    /// Object position before its own avatar shape and hover offsets.
    /// Camera focus must use this directly: adding then subtracting offsets
    /// loses precision and couples the view to the renderer's pose timing.
    pub fn object_transform_unshifted(world: &World, idx: usize, now: Instant) -> Option<Xf> {
        Self::object_transform_raw(world, idx, now, 0, None)
    }

    /// Where our avatar's object is (render space) while it sits on another:
    /// its seat-relative position through the seat's transform, without the
    /// shape offset of the drawn root (sitting branch of
    /// LLAgent::setPositionAgent). None when standing or before the seat is
    /// known.
    pub fn seated_agent_position(world: &World, now: Instant) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(&world.agent_id)?;
        let o = world.objects.get(idx).filter(|o| o.parent_id != 0)?;
        world.objects.parent_of(o)?;
        Self::object_transform_raw(world, idx, now, 0, None).map(|(p, _, _)| p)
    }

    /// `object_transform`, taking the parents' transforms from `cache` when
    /// they were placed this frame (same `now`, so the same result).
    fn object_transform_in(world: &World, idx: usize, now: Instant, depth: u32, cache: Option<&XfCache>) -> Option<Xf> {
        let (p, r, hud) = Self::object_transform_raw(world, idx, now, depth, cache)?;
        let o = world.objects.get(idx)?;
        let dz = if o.is_avatar() {
            world.avatar_shape_dz.get(&o.full_id).copied().unwrap_or(0.0)
                + world.appearance_hover(&o.full_id)
                + world.water_visual_dz(&o.full_id)
        } else {
            0.0
        };
        // updateRootPositionAndRotation: a seated root follows the full seat
        // rotation; hover is in avatar-local space (vehicles can tilt).
        let offset = if o.is_avatar() && o.parent_id != 0 {
            r * (Vec3::Z * dz)
        } else {
            Vec3::Z * dz
        };
        Some((p + offset, r, hud))
    }

    fn object_transform_raw(world: &World, idx: usize, now: Instant, depth: u32, cache: Option<&XfCache>) -> Option<Xf> {
        let o = world.objects.get(idx)?;
        // (the photo mode's frozen world holds our avatar too)
        let held = world.objects.held_pose(o);
        let (p, mut r) = if let Some(pose) = held {
            pose
        } else if o.full_id == world.agent_id && world.agent.has_local_control() {
            // The agent is already in main-region coordinates. Its object
            // can still belong to the previous/next region during an arrival.
            (
                world.agent.position - world.region_offset(o.key.region)?,
                world.agent.body_rotation(),
            )
        } else {
            let (mut p, r) = world.objects.predicted(o, now);
            // LLViewerObject::interpolateLinearMotion: an avatar that is
            // extrapolated does not sink into the land
            if o.is_avatar() && o.parent_id == 0 && o.velocity != Vec3::ZERO {
                p.z = world.avatar_floor(o, p).map_or(p.z, |floor| p.z.max(floor));
            }
            (p, r)
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
        let (pp, pr, phud) = match cache.and_then(|c| c.get(pidx)) {
            Some(xf) => xf,
            None => Self::object_transform_in(world, pidx, now, depth + 1, cache)?,
        };
        if parent.is_avatar() && o.state != 0 {
            // attachment root: relative to the attachment point (the frame
            // the GPU places worn objects from, anchors.rs)
            let anchors::AttachFrame { pos: apos, rot: arot, hud } = anchors::attach_frame(world, parent, o.attachment_point());
            if hud {
                let offset = if parent.full_id == world.agent_id {
                    world.hud_scales.offset(o.full_id)
                } else {
                    Vec3::ZERO
                };
                return Some((apos + arot * p + offset, arot * r, true));
            }
            let pos = pp + pr * (apos + arot * p);
            return Some((pos, pr * arot * r, hud));
        }
        Some((
            pp + pr * (p * if phud { world.hud_scale(idx) } else { 1.0 }),
            (pr * r).normalize(),
            phud,
        ))
    }

    /// The photo's isolation (`snapshot::isolate`): what it leaves out is
    /// flagged, every object again when it changes, else those synced this
    /// frame (an object arriving); and while choosing in the scene, what
    /// is chosen is outlined, every prim and attachment of it.
    fn sync_photo_isolation(&mut self, renderer: &mut Renderer, world: &World, synced: &[usize]) {
        use crate::snapshot::isolate;
        let iso = &world.photo_isolation;
        if self.photo_isolation_version != iso.version {
            self.photo_isolation_version = iso.version;
            for idx in 0..self.gpu.len() {
                let hidden = !isolate::object_visible(world, idx);
                let g = &mut self.gpu[idx];
                if g.photo_hidden != hidden {
                    g.photo_hidden = hidden;
                    renderer.cull.objects.set(idx, g.cull_object(idx));
                }
            }
        } else if iso.applied() {
            for &idx in synced {
                let hidden = !isolate::object_visible(world, idx);
                if let Some(g) = self.gpu.get_mut(idx) {
                    g.photo_hidden = hidden;
                }
            }
        }
        self.photo_marks.clear();
        if iso.picking {
            let mut stack: Vec<usize> = iso.chosen().iter().filter_map(|id| world.objects.index_of_uuid(id)).collect();
            while let Some(idx) = stack.pop() {
                if self.photo_marks.insert(idx)
                    && let Some(o) = world.objects.get(idx)
                {
                    stack.extend(world.objects.children_of(&o.key).iter().copied());
                }
            }
        }
    }

    /// Outline the avatar `avatar` (an object index) and everything it
    /// wears while an inventory drag would give to it, as the photo's
    /// chosen subjects are (#991); `None` once the drag leaves it. Read again
    /// at each call: what it wears may arrive meanwhile.
    pub fn mark_give_target(&mut self, world: &World, avatar: Option<usize>) {
        let (target, marks) = &mut self.give_marks;
        if avatar.is_none() && target.is_none() {
            return;
        }
        *target = avatar;
        marks.clear();
        let mut stack: Vec<usize> = avatar.into_iter().collect();
        while let Some(idx) = stack.pop() {
            if marks.insert(idx)
                && let Some(o) = world.objects.get(idx)
            {
                stack.extend(world.objects.children_of(&o.key).iter().copied());
            }
        }
    }

    fn sync_parcel_crossings(&mut self, world: &mut World) {
        if world.parcel_visibility.active() {
            // Anchored attachments normally follow their wearer entirely on the
            // GPU. A parcel crossing must also visit those subtrees this frame.
            let crossings: Vec<_> = self
                .sets
                .own_moving
                .as_slice()
                .iter()
                .copied()
                .filter(|&idx| {
                    world.objects.get(idx).is_some_and(|o| o.is_avatar() || o.parent_id == 0)
                        && self
                            .gpu
                            .get(idx)
                            .is_some_and(|g| g.parcel_hidden == world.parcel_object_visible(idx))
                })
                .collect();
            let mut visited = std::collections::HashSet::new();
            let mut stack: Vec<_> = crossings.into_iter().map(|idx| (idx, 0)).collect();
            while let Some((idx, depth)) = stack.pop() {
                if depth >= sync_sets::MAX_DEPTH || !visited.insert(idx) {
                    continue;
                }
                if let Some(o) = world.objects.get_mut_untracked(idx) {
                    o.render.needs_records = true;
                    let key = o.key;
                    self.sets.dirty.insert(idx);
                    stack.extend(world.objects.children_of(&key).iter().map(|&child| (child, depth + 1)));
                }
            }
        }
    }

    /// Bring GPU records up to date, visiting only the objects that need it
    /// (sync_sets.rs): what moves (avatars, their attachments, moving
    /// linksets), what changed, what waits for its geometry, and this
    /// frame's LOD slice. Their placement (transform, detail level, bounds)
    /// is computed in parallel, parents' level first so children reuse the
    /// parents' transforms; transform-only updates are then applied in turn
    /// (unchanged records are not uploaded again), the rest goes through the
    /// full `sync_object`. What an avatar wears is placed by the GPU from
    /// the anchors written here (anchors.rs) and is not visited.
    pub fn sync(&mut self, renderer: &mut Renderer, world: &mut World, view: &CullView) {
        self.finish_mesh_lod_resets(world, Instant::now());
        if self.parcel_visibility_version != world.parcel_visibility.version {
            self.parcel_visibility_version = world.parcel_visibility.version;
            for (idx, object) in world.objects.slots.iter_mut().enumerate() {
                if let Some(object) = object {
                    object.render.needs_records = true;
                    self.sets.dirty.insert(idx);
                }
            }
        }
        let rlva_state = (
            world.rlva.camera_texture(),
            world.rlva.has("showself"),
            world.rlva.has("showselfhead"),
        );
        if self.rlva_render_state != rlva_state {
            self.rlva_render_state = rlva_state;
            for (idx, object) in world.objects.slots.iter_mut().enumerate() {
                if let Some(object) = object {
                    object.render.needs_records = true;
                    object.material_dirty = true;
                    self.sets.dirty.insert(idx);
                    if object.is_avatar()
                        && let Some(gpu) = self.gpu.get_mut(idx)
                    {
                        gpu.tex_generation = u64::MAX;
                    }
                }
            }
        }
        use crate::frame_profile::Part;
        let t0 = Instant::now();
        let mut slow = self.sync_slow.map(|limit| sync_slow::SlowSync::new(limit, t0));
        let step = |slow: &mut Option<sync_slow::SlowSync>, name| {
            if let Some(s) = slow {
                s.step(name);
            }
        };
        self.frame = self.frame.wrapping_add(1);
        self.ensure_avatar_parts(renderer);
        step(&mut slow, "parts");
        // (the derez of what was killed first: it looks for walls in the world)
        let kills = std::mem::take(&mut world.objects.killed);
        if !kills.is_empty() {
            self.note_kills(renderer, world, kills, Instant::now());
        }
        self.collect_garbage(renderer, &mut world.objects, self.sync_budget / 2);
        step(&mut slow, "garbage");
        self.update_shown_bakes(world);
        // settings assets the environment waits for (the default day, the
        // environment selector's items)
        for id in world.eep.wanted_settings() {
            match self.settings.get(id) {
                settings::SettingsState::Ready(s) => world.eep.on_settings(id, Some(s)),
                settings::SettingsState::Failed => world.eep.on_settings(id, None),
                settings::SettingsState::Pending => {}
            }
        }

        // Render origin changed: every transform is stale.
        let origin = world.main_origin();
        if origin != self.last_origin {
            if let (Some(old), Some(new)) = (self.last_origin, origin) {
                let shift = Vec3::new((old.0 as f64 - new.0 as f64) as f32, (old.1 as f64 - new.1 as f64) as f32, 0.0);
                for state in self.flexibles.values_mut() {
                    state.sim.shift(shift);
                }
            }
            self.last_origin = origin;
            for (idx, o) in world.objects.slots.iter_mut().enumerate() {
                if let Some(o) = o {
                    o.render.needs_records = true;
                    self.sets.dirty.insert(idx);
                }
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

        step(&mut slow, "origin");
        self.sync_terrain(renderer, world);
        self.water.sync(renderer, world);
        step(&mut slow, "terrain");

        if self.gpu.len() < world.objects.slots.len() {
            self.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        }
        // the GPU object table covers every slot (an attachment's avatar
        // index is always in it, as in the CPU lists)
        renderer.cull.objects.ensure_len(self.gpu.len());
        let now = self.held_at.unwrap_or_else(Instant::now);
        let n = world.objects.slots.len();
        let agent = world.agent_id;
        // objects changed since the last frame
        let mut touched = std::mem::take(&mut self.touched_tmp);
        world.objects.take_touched(&mut touched);
        for &idx in &touched {
            // a new object in the slot of a removed one not released yet
            self.release_slot(renderer, idx);
            self.note_worn_change(world, idx);
            // a new object: its first sync waits its turn, without a
            // place in this frame's list
            if world.objects.is_arrival(idx) {
                self.sets.arrivals.push(idx);
                continue;
            }
            let rigged = self.gpu.get(idx).is_some_and(|g| g.moves_with_skeleton());
            self.sets.touched(&world.objects, agent, idx, rigged);
            // (its click action or flags may have changed)
            self.pick.changed(idx);
        }
        touched.clear();
        self.touched_tmp = touched;
        self.sync_parcel_crossings(world);
        step(&mut slow, "touched");
        // a new material generation: the faces using materials are rebuilt
        let mat_gen = self.materials.generation;
        if mat_gen != self.sets.seen_mat_gen {
            self.sets.seen_mat_gen = mat_gen;
            for &idx in self.sets.material_users.as_slice() {
                if self
                    .gpu
                    .get(idx)
                    .is_some_and(|g| self.mat_changes.stale(&g.material_ids, g.mat_generation, mat_gen))
                {
                    self.sets.dirty.insert(idx);
                }
            }
        }
        // detail level checks: by events, or the slice of every object
        let events = !sync_sets::slice_mode();
        let dt = self.last_sync.replace(now).map_or(0.0, |last| (now - last).as_secs_f32());
        let camera_moved = self.sets.lod.advance(view.eye, self.lod_factor, self.fov_y);
        if events {
            self.note_hud_changes(world);
        }
        let list = {
            let state = FrameState {
                gpu: &self.gpu,
                geoms: &self.geoms,
                textures: &self.textures,
                mat_gen,
                mat_changes: &self.mat_changes,
                still_huds: events,
            };
            let checks = if events {
                sync_sets::LodChecks::Events {
                    anchored: self.anchors.members(),
                    dt,
                    camera_moved,
                }
            } else {
                sync_sets::LodChecks::Slice
            };
            self.sets.frame_list(&world.objects, agent, self.frame, n, &state, checks)
        };
        self.stats.synced = list.items.len();
        self.stats.rebuilt = 0;
        let mut check = self.sync_check.take();
        if let Some(c) = &mut check {
            c.listed(list.sources, list.lod_checked);
        }
        let t_part = self.parts.lap(Part::SyncList, t0);
        step(&mut slow, "list");

        // (the animated objects' boxes first: their rigged meshes' level)
        self.update_anim_boxes(world, now);
        // placement in parallel, one level of the parent chains at a time
        self.xf_cache.next_frame(n);
        let mut plans: Vec<(usize, Plan)> = Vec::with_capacity(list.items.len());
        for level in list.levels() {
            // (a few objects, as the avatars' level: this thread alone)
            let planned: Vec<Vec<(usize, Plan)>> = {
                let ctx = self.sync_ctx(world, &renderer.records, view.eye, now);
                frame_par::fold(
                    &level,
                    PLAN_GRAIN,
                    || Vec::with_capacity(PLAN_GRAIN.chunk),
                    |mut part, &idx| {
                        part.push((idx, ctx.plan(idx)));
                        part
                    },
                )
            };
            for part in planned {
                for (idx, plan) in &part {
                    if let Some(xf) = plan.xf() {
                        self.xf_cache.set(*idx, xf);
                    }
                }
                plans.extend(part);
            }
        }
        let t_part = self.parts.lap(Part::SyncPlan, t_part);
        step(&mut slow, "plan");
        let mut synced: Vec<usize> = plans.iter().map(|(idx, _)| *idx).collect();
        // Moves and avatars are applied whatever they cost. The other full
        // syncs (new objects, geometry bound, faces rebuilt) only while the
        // frame's budget lasts: the rest waits in the backlog, served
        // nearest first, and the faces a waiting object already has keep
        // following it.
        let mut budget = FullSyncBudget::new(self.sync_budget);
        for (idx, plan) in plans {
            let kind = check.as_ref().and_then(|_| {
                world
                    .objects
                    .get(idx)
                    .map(|o| sync_check::Kind::of(world, idx, o, self.gpu.get(idx)))
            });
            let mut note = |outcome| {
                if let (Some(c), Some(kind)) = (&mut check, kind) {
                    c.visited(kind, outcome);
                }
            };
            match plan {
                Plan::Move(m) => {
                    note(if m.changed {
                        sync_check::Outcome::Moved
                    } else {
                        sync_check::Outcome::Checked
                    });
                    if let Some(s) = &mut slow {
                        s.moved();
                    }
                    self.apply_move(renderer, world, idx, m)
                }
                Plan::Full { xf, interim } => {
                    let avatar = self.gpu.get(idx).is_some_and(|g| g.is_avatar) || world.objects.get(idx).is_some_and(|o| o.is_avatar());
                    // (without a transform the sync only notes that the
                    // object still waits for its parent)
                    let now_or_never = avatar || xf.is_none();
                    if !now_or_never && self.sets.arrivals.contains(idx) {
                        // listed with its parent: its turn comes with the
                        // other new objects
                        note(sync_check::Outcome::Waiting);
                        continue;
                    }
                    if now_or_never || (self.sets.backlog.is_empty() && budget.allows()) {
                        note(sync_check::Outcome::Full);
                        let started = Instant::now();
                        self.sets.backlog.remove(idx);
                        self.sync_object(renderer, world, idx, now, view, xf);
                        self.after_full_sync(world, idx, mat_gen);
                        if let Some(s) = &mut slow {
                            let kind = if avatar { sync_slow::Full::Avatar } else { sync_slow::Full::Listed };
                            s.full(kind, idx, started);
                        }
                        if !now_or_never {
                            budget.spend(started.elapsed());
                        }
                    } else {
                        note(sync_check::Outcome::Waiting);
                        if let Some(m) = interim {
                            self.apply_move(renderer, world, idx, m);
                        }
                        let distance = xf.map_or(0.0, |(pos, _, _)| pos.distance(view.eye));
                        self.sets.backlog.push(idx, distance);
                    }
                }
            }
        }
        // New objects first, for half of the time left at most: what they
        // ask for (a geometry to build, a mesh to download) takes the
        // longest to come, and the objects already waiting for their faces
        // keep the other half.
        step(&mut slow, "apply");
        let reserve = budget.left() / 2;
        let (before, started) = (synced.len(), Instant::now());
        self.sync_arrivals(renderer, world, now, view, mat_gen, &mut budget, reserve, &mut synced);
        if let Some(s) = &mut slow {
            s.fulls(sync_slow::Full::Arrival, synced.len() - before, started);
        }
        step(&mut slow, "arrivals");
        while budget.allows() {
            let Some(idx) = self.sets.backlog.pop() else {
                break;
            };
            let started = Instant::now();
            self.sync_object(renderer, world, idx, now, view, None);
            self.after_full_sync(world, idx, mat_gen);
            synced.push(idx);
            if let Some(s) = &mut slow {
                s.full(sync_slow::Full::Backlog, idx, started);
            }
            budget.spend(started.elapsed());
        }
        step(&mut slow, "backlog");
        let (before, started) = (synced.len(), Instant::now());
        self.sync_arrivals(
            renderer,
            world,
            now,
            view,
            mat_gen,
            &mut budget,
            std::time::Duration::ZERO,
            &mut synced,
        );
        if let Some(s) = &mut slow {
            s.fulls(sync_slow::Full::Arrival, synced.len() - before, started);
        }
        step(&mut slow, "arrivals");
        self.stats.sync_backlog = self.sets.backlog.len() + self.sets.arrivals.len();
        if let Some(c) = &mut check {
            let range = c.sweep_range(self.gpu.len());
            let end = range.end >= self.gpu.len();
            c.swept(self.lod_behind(world, &renderer.records, view.eye, now, range), end);
            if c.frame() {
                c.write();
            }
        }
        self.sync_check = check;
        step(&mut slow, "check");
        let t_part = self.parts.lap(Part::SyncApply, t_part);
        self.update_anchors(renderer, world, now, self.sync_check.is_some());
        let t_part = self.parts.lap(Part::SyncAnchors, t_part);
        step(&mut slow, "anchors");
        self.sync_flexibles(renderer, world, &synced, now, view);
        self.sync_grass_ground(renderer, world, now, &mut synced);
        step(&mut slow, "flexibles");
        self.sync_photo_isolation(renderer, world, &synced);
        // GPU object table: bounds and flags of what was synced (unchanged
        // entries are not uploaded); the same bounds for the hover search,
        // the same flags for the lists
        for idx in synced {
            if let Some(g) = self.gpu.get(idx) {
                renderer.cull.objects.set(idx, g.cull_object(idx));
                self.pick.set(idx, hover::PickEntry::of(g));
                self.list_index.set(idx, g);
            }
        }
        step(&mut slow, "tables");

        self.geom_gc(renderer);
        let t_part = self.parts.lap(Part::SyncApply, t_part);
        step(&mut slow, "geom_gc");
        self.stats.objects = world.objects.len();
        self.stats.geometries = self.geoms.len();
        self.stats.geom_pending = self.geom_pending;
        self.stats.jobs = self.jobs.pending();
        self.stats.sync_ms = t0.elapsed().as_secs_f32() * 1000.0;
        // Textures changed alpha class (first texels, a finer level): the
        // faces using their slots are classified again (new faces are
        // classified when built).
        let mut changed = std::mem::take(&mut self.alpha_changes_tmp);
        // Textures showing their first pixels: the rez effect looks at
        // the objects using them (rez.rs) instead of walking them all.
        self.textures.take_first_texels(&mut changed);
        self.rez_first_texels(renderer, &changed);
        changed.clear();
        self.textures.take_alpha_changes(&mut changed);
        if !changed.is_empty() {
            self.reclassify_slots(renderer, &changed);
            changed.clear();
        }
        self.alpha_changes_tmp = changed;
        self.parts.lap(Part::SyncAlpha, t_part);
        if let Some(watch) = &mut self.sculpt_watch {
            watch.poll(world, &self.gpu, &self.textures, now);
        }
        if let Some(mut s) = slow {
            s.step("alpha");
            let gpu = &self.gpu;
            let name = |idx: usize| match world.objects.get(idx) {
                Some(o) => {
                    let kind = sync_check::Kind::of(world, idx, o, gpu.get(idx));
                    let faces = gpu.get(idx).map_or(0, |g| g.faces.len());
                    format!("{kind:?} {} ({faces} faces)", o.key.local_id)
                }
                None => format!("removed slot {idx}"),
            };
            if let Some(line) = s.line(name) {
                log::info!("{line}");
            }
        }
    }

    /// Classify again the faces whose base texture is in one of `slots`
    /// (found through `slot_users`, not by a pass over every face of the
    /// scene, which ran in most frames while textures arrive); the changed
    /// ones update their record's alpha mode, their GPU face entry and
    /// their object's `special` flag. Shadow shaders use the same face
    /// mode as the lists.
    fn reclassify_slots(&mut self, renderer: &mut Renderer, slots: &[u32]) {
        let alpha = &self.textures.alpha_by_slot;
        let channel = &self.textures.alpha_channel_by_slot;
        for &slot in slots {
            for &(_, idx) in self.slot_users.range((slot, 0)..=(slot, u32::MAX)) {
                let idx = idx as usize;
                let Some(g) = self.gpu.get_mut(idx) else {
                    continue;
                };
                // LLFace::canRenderAsMask: never auto-mask a rigged mesh or HUD
                let object_auto_mask = !g.rigged && !g.hud;
                let mut any = false;
                for (fi, f) in g.faces.iter_mut().enumerate() {
                    if f.base_slot != slot {
                        continue;
                    }
                    let (pass, pool) = face_mode(f, alpha, channel, object_auto_mask);
                    if pass == f.pass && pool == f.glow_pool {
                        continue;
                    }
                    any = true;
                    let before = f.record_flags();
                    f.pass = pass;
                    f.glow_pool = pool;
                    let mode = f.record_flags();
                    let mode_changed = before != mode;
                    let (record, face) = (f.record, cull_face(f, idx, fi == 0));
                    if mode_changed && let Some(mut r) = renderer.records.get(record).copied() {
                        r.flags[0] = (r.flags[0] & !(flags::ALPHA_BLEND | flags::ALPHA_MASK)) | mode;
                        renderer.records.set(record, r);
                    }
                    renderer.records.set_face(record, face);
                }
                if any {
                    g.special = is_special(&g.faces);
                    self.list_index.set(idx, g);
                    // (a face that became see-through, or stopped being)
                    self.pick.changed(idx);
                }
            }
        }
    }

    /// Classify freshly built faces of object `idx`: pass, record alpha
    /// mode, GPU face entries, `special`; and note the object as a user of
    /// their texture slots.
    ///
    /// `object_auto_mask`: see [`face_mode`]. Pass `rigged_owner.is_none() &&
    /// !hud` from `rebuild_faces` — `g.rigged` is only updated after that
    /// rebuild returns.
    fn finish_faces(&mut self, renderer: &mut Renderer, idx: usize, g: &mut ObjGpu, object_auto_mask: bool) {
        // (new faces: what the cursor is over may have changed)
        self.pick.changed(idx);
        self.fresh_usage.insert(idx);
        let alpha = &self.textures.alpha_by_slot;
        let channel = &self.textures.alpha_channel_by_slot;
        let mut last_slot = None;
        // (an object appearing keeps its rez effect through a rebuild)
        // (and its geometry may have been what it waited for)
        if let Some((v, glow)) = self.rez.value_of(idx) {
            rez::rez_faces(renderer, &mut g.faces, v, glow);
            self.rez.note_ready(idx);
        }
        for (fi, f) in g.faces.iter_mut().enumerate() {
            // (faces of an object mostly share their texture: one entry)
            if last_slot != Some(f.base_slot) {
                last_slot = Some(f.base_slot);
                self.slot_users.insert((f.base_slot, idx as u32));
            }
            let (pass, pool) = face_mode(f, alpha, channel, object_auto_mask);
            f.pass = pass;
            f.glow_pool = pool;
            let mode = f.record_flags();
            if mode != 0
                && let Some(mut r) = renderer.records.get(f.record).copied()
            {
                r.flags[0] = (r.flags[0] & !(flags::ALPHA_BLEND | flags::ALPHA_MASK)) | mode;
                renderer.records.set(f.record, r);
            }
            renderer.records.set_face(f.record, cull_face(f, idx, fi == 0));
        }
        g.special = is_special(&g.faces);
    }

    /// Keep `light_objects` in step with an object's light parameters.
    fn note_light(&mut self, world: &World, idx: usize) {
        let light = world.objects.get(idx).is_some_and(|o| o.extra.light.is_some());
        if let Some(g) = self.gpu.get_mut(idx)
            && g.is_light != light
        {
            g.is_light = light;
            if light {
                self.light_objects.insert(idx);
            } else {
                self.light_objects.remove(&idx);
            }
        }
    }

    /// Transform-only update planned in parallel: bounds and the faces'
    /// model matrix (records uploaded only when it changed).
    fn apply_move(&mut self, renderer: &mut Renderer, world: &mut World, idx: usize, m: MoveSync) {
        self.note_light(world, idx);
        let posed = self.posed_of(renderer, self.gpu.get(idx).and_then(|g| g.geom), m.posed_rest);
        let Some(g) = self.gpu.get_mut(idx) else {
            return;
        };
        g.posed = posed;
        g.wide = g.wide.filter(|_| posed.is_some());
        g.hud = m.xf.2;
        g.owner_avatar = m.owner_avatar;
        if m.mirror != g.mirror {
            g.mirror = m.mirror;
            if m.mirror {
                self.mirror_objects.insert(idx);
            } else {
                self.mirror_objects.remove(&idx);
            }
        }
        g.center = m.center;
        g.radius = m.radius;
        g.anchor = m.anchor;
        g.local_center = m.local_center;
        if m.changed {
            if let Some(still) = &mut self.still
                && !g.hud
                && let Some(old) = g.faces.first().and_then(|f| renderer.records.get(f.record))
                // (same space: not a record passing to or from an anchor)
                && old.anchor() == m.anchor.map(|a| a.matrix)
                && let Some(o) = world.objects.get(idx)
            {
                let moved = Vec3::from_slice(&old.model[3][..3]).distance(Vec3::from_slice(&m.model[3][..3]));
                still.moved(o.full_id, m.owner_avatar.is_some(), moved, || still_check::describe(o, g.lod));
            }
            for f in &g.faces {
                if g.hud {
                    renderer.records.set_hud_model(f.record, m.model, world.hud_scale(idx));
                } else {
                    renderer.records.set_placement(f.record, m.model, m.anchor.map(|a| a.matrix));
                }
            }
        }
        self.anchors.placed(world, idx, m.anchor);
        if let Some((radius, distance)) = m.lod {
            self.watch_lod(idx, m.anchor.is_some(), radius, distance);
        }
        if m.clear_flags
            && let Some(o) = world.objects.get_mut_untracked(idx)
        {
            o.render.needs_records = false;
        }
    }

    /// The detail level of object `idx` was checked with this size and
    /// distance: when it is looked at again. What follows a moving object
    /// is checked at each frame, what the GPU places from an avatar in
    /// turn (sync_sets.rs); the others when the camera has travelled far
    /// enough to change their level (lod_watch.rs). An object whose turn
    /// came before that keeps the wait it has.
    fn watch_lod(&mut self, idx: usize, anchored: bool, radius: f32, distance: f32) {
        if anchored || self.sets.follows_motion(idx) {
            self.sets.lod.forget(idx);
        } else if !(self.sets.only_checked(idx) && self.sets.lod.is_watched(idx)) {
            let slack = lod_watch::lod_slack(radius, distance, self.lod_factor, self.fov_y);
            self.sets.lod.watch(idx, slack);
        }
    }

    /// AURORA_SYNC_CHECK: a look at the objects of `range` (the frame's
    /// slice of the check's pass over every object). Those at rest whose
    /// detail level could be judged and those whose wanted geometry is not
    /// what their distance asks for at this moment, then the same for the
    /// objects the GPU places from an avatar. New objects and those whose
    /// full sync waits for its turn are left out.
    fn lod_behind(
        &self,
        world: &World,
        records: &aurora_render::arena::RecordStore,
        eye: Vec3,
        now: Instant,
        range: std::ops::Range<usize>,
    ) -> [usize; 4] {
        let ctx = self.sync_ctx(world, records, eye, now);
        let mut out = [0; 4];
        for (idx, g) in self.gpu.iter().enumerate().take(range.end).skip(range.start) {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            if g.is_avatar || o.is_avatar() || g.hud || g.wanted_geom.is_none() {
                continue;
            }
            if self.sets.arrivals.contains(idx) || self.sets.backlog.contains(idx) || self.sets.follows_motion(idx) {
                continue;
            }
            let Some(wanted) = ctx.wanted_now(idx, o) else {
                continue;
            };
            let at = if g.anchor.is_some() { 2 } else { 0 };
            out[at] += 1;
            if wanted != g.wanted_geom {
                out[at + 1] += 1;
            }
        }
        out
    }

    /// HUD attachments of our avatar are not placed again at each frame
    /// with what follows it: note those whose place on the screen changed
    /// without an update of theirs (window resized, local zoom or offset),
    /// for this frame's list.
    fn note_hud_changes(&mut self, world: &mut World) {
        let mut roots = std::mem::take(&mut self.hud_roots_tmp);
        roots.clear();
        if let Some(me) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) {
            for &idx in world.objects.children_of(&me.key) {
                let Some(o) = world.objects.get(idx).filter(|o| (31..=38).contains(&o.attachment_point())) else {
                    continue;
                };
                let offset = world.hud_scales.offset(o.full_id);
                let placed = [world.hud_aspect, world.hud_scales.scale(o.full_id), offset.x, offset.y, offset.z];
                roots.push((idx, o.full_id, placed));
            }
        }
        if roots.len() != self.hud_roots.len() {
            self.hud_roots.retain(|idx, _| roots.iter().any(|r| r.0 == *idx));
        }
        for &(idx, id, placed) in &roots {
            if self.hud_roots.insert(idx, (id, placed)) == Some((id, placed)) {
                continue;
            }
            // (as an update of the attachment: its prims are placed with it)
            if let Some(o) = world.objects.get_mut_untracked(idx) {
                o.render.needs_records = true;
            }
            self.sets.dirty.insert(idx);
        }
        self.hud_roots_tmp = roots;
    }

    /// First sync of the new objects, in arrival order, while `budget`
    /// allows it beyond `reserve`.
    #[allow(clippy::too_many_arguments)]
    fn sync_arrivals(
        &mut self,
        renderer: &mut Renderer,
        world: &mut World,
        now: Instant,
        view: &CullView,
        mat_gen: u64,
        budget: &mut FullSyncBudget,
        reserve: std::time::Duration,
        synced: &mut Vec<usize>,
    ) {
        while !self.sets.arrivals.is_empty() && budget.allows_beyond(reserve) {
            // (the clock is read once for a few objects: a first sync that
            // only asks for a geometry takes a fraction of a microsecond)
            let started = Instant::now();
            for _ in 0..ARRIVALS_PER_CLOCK {
                let Some(idx) = self.sets.arrivals.pop() else {
                    break;
                };
                self.sync_object(renderer, world, idx, now, view, None);
                self.after_full_sync(world, idx, mat_gen);
                synced.push(idx);
            }
            budget.spend(started.elapsed());
        }
    }

    /// The materials changed since the last frame (arrived, given up, local):
    /// a new generation for the legacy ones (the glTF store counts its own),
    /// and each noted at the generation that counts it, so that only their
    /// users are rebuilt (mat_changes.rs).
    pub(crate) fn note_material_changes(&mut self) {
        let mut changed = std::mem::take(&mut self.mat_changed_tmp);
        self.materials.take_changed(&mut changed);
        let gltf = changed.len();
        self.legacy_mats.take_changed(&mut changed);
        if changed.len() > gltf {
            self.materials.generation += 1;
        }
        let generation = self.materials.generation;
        for id in changed.drain(..) {
            self.mat_changes.note(id, generation);
        }
        self.mat_changed_tmp = changed;
    }

    /// Bookkeeping after a full sync: what could not finish is retried next
    /// frame, geometry still pending is watched, material users noted, and
    /// a mesh found rigged starts moving. A new object is no longer one.
    fn after_full_sync(&mut self, world: &mut World, idx: usize, mat_gen: u64) {
        // (whichever path synced a new object first: the arrivals, the
        // objects listed, the backlog)
        if world.objects.is_arrival(idx) {
            self.rez.arrived(idx);
        }
        world.objects.settle(idx);
        let Some(g) = self.gpu.get(idx) else {
            return;
        };
        let mats_changed = self.mat_changes.stale(&g.material_ids, g.mat_generation, mat_gen);
        if world
            .objects
            .get(idx)
            .is_some_and(|o| o.render.needs_records || o.shape_dirty || o.material_dirty || mats_changed)
        {
            self.sets.dirty.insert(idx);
        }
        self.sets.geometry_wanted(idx, g.wanted_geom.is_some() && g.wanted_geom != g.geom);
        if g.material_ids.is_empty() {
            self.sets.material_users.remove(idx);
        } else {
            self.sets.material_users.insert(idx);
        }
        let rigged = g.moves_with_skeleton();
        self.sets.synced(&world.objects, world.agent_id, idx, rigged);
    }

    fn sync_object(&mut self, renderer: &mut Renderer, world: &mut World, idx: usize, now: Instant, view: &CullView, xf: Option<Xf>) {
        let parcel_visibility = world.parcel_object_visibility(idx);
        if parcel_visibility != Some(true) {
            if let Some(mut g) = self.gpu.get_mut(idx).map(std::mem::take) {
                self.release_obj(renderer, idx, &mut g);
            }
            if let Some(g) = self.gpu.get_mut(idx) {
                g.parcel_hidden = true;
            }
            self.anchors.forget(idx);
            self.pick.remove(idx);
            self.list_index.remove(idx);
            self.sets.lod.forget(idx);
            if let Some(object) = world.objects.get_mut_untracked(idx) {
                object.render.needs_records = parcel_visibility.is_none();
                object.shape_dirty = false;
                object.material_dirty = false;
            }
            return;
        }
        if let Some(g) = self.gpu.get_mut(idx) {
            g.parcel_hidden = false;
        }
        self.note_light(world, idx);
        let Some((pos, rot, hud)) = xf.or_else(|| Self::object_transform_in(world, idx, now, 0, Some(&self.xf_cache))) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let is_avatar = o.is_avatar();
        let scale = o.visual_scale_at(now).max(Vec3::splat(0.001));
        let radius = (scale * 0.5).length().max(0.05);

        // ---- avatar
        if is_avatar {
            self.sync_avatar(renderer, world, idx, pos, rot);
            if let Some(o) = world.objects.get_mut_untracked(idx) {
                o.render.needs_records = false;
                o.shape_dirty = false;
                o.material_dirty = false;
            }
            return;
        }

        // ---- geometry / LOD
        let (lod_radius, lod_dist) = if hud {
            (0.0, 0.0)
        } else {
            self.sync_ctx(world, &renderer.records, view.eye, now).lod_metrics(idx, o, pos)
        };
        let lod = mesh_inspection::display_lod(
            &self.mesh_lod_overrides,
            o,
            hud,
            lod_for(lod_radius, lod_dist, self.lod_factor, self.fov_y),
        );
        // (whatever comes of this sync; what the GPU places leaves below)
        self.watch_lod(idx, false, lod_radius, lod_dist);
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let wanted = Self::object_geom_key(o, lod);
        let wearer = Self::wearer_avatar(world, idx);
        let owner_avatar = wearer.map(|(_, i)| i);
        let first_person_hidden = Self::attachment_first_person_hidden(world, idx);
        // what its mesh is asked for at: its size on screen, and ours first
        let rank_radius = sync_plan::mesh_rank_radius(lod_radius, wearer.is_some());
        let need = MeshNeed {
            px: if hud {
                f32::MAX
            } else {
                rank_radius / lod_dist.max(0.1) * view.screen_height / view.tan_half_fov.max(1e-6)
            },
            own: hud || wearer.is_some_and(|(id, _)| id == world.agent_id),
        };
        let g = &mut self.gpu[idx];
        g.hud = hud;
        g.owner_avatar = owner_avatar;
        g.first_person_hidden = first_person_hidden;
        // reflection probe with the box and mirror flags (LLReflectionProbeParams)
        let mirror = o.extra.reflection_probe.is_some_and(|p| p.flags & 0x4 != 0 && p.flags & 0x1 != 0);
        if mirror != g.mirror {
            g.mirror = mirror;
            if mirror {
                self.mirror_objects.insert(idx);
            } else {
                self.mirror_objects.remove(&idx);
            }
        }
        let shape_dirty = o.shape_dirty;
        if wanted != g.wanted_geom || shape_dirty {
            g.wanted_geom = wanted;
        }
        let mut sculpt_wait = None;
        if let Some(GeomKey::Sculpt { .. }) = wanted
            && let Some(s) = o.visual_volume().sculpt
        {
            sculpt_wait = Some(s.texture);
        }
        let previous_geom = self.gpu[idx].geom;
        let mut bind_new = None;
        if let Some(k @ GeomKey::Grass { .. }) = wanted {
            self.request_grass(renderer, world, idx, k, pos);
        }
        if let Some(k) = wanted {
            // need an Object reference for building: re-borrow
            let obj = world.objects.get(idx);
            let ready = match obj {
                Some(obj) => self.request_geom(k, obj, need),
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
                self.gpu[idx].tex_ids.push(st);
            }
            // A face can already hold this UUID as an ordinary colour map.
            self.textures.want_sculpt(&st);
            // a map missing or failed shows the sphere placeholder until it
            // decodes (LLVOVolume::sculpt passes isMissingAsset as
            // LLVolume::sculpt's visible_placeholder); one still loading
            // draws nothing, as there
            if let Some(GeomKey::Sculpt { hash, lod }) = wanted
                && bind_new.is_none()
                && self.gpu[idx].geom != wanted
                && self.textures.is_unavailable(&st)
            {
                let k = GeomKey::SculptPlaceholder { hash, lod };
                if self.gpu[idx].geom != Some(k) && world.objects.get(idx).is_some_and(|obj| self.request_geom(k, obj, need)) {
                    bind_new = Some(k);
                }
            }
        }
        // waits for its sculpt texture only until this sculpt is bound (a
        // wait left set kept the object synced every frame)
        self.gpu[idx].sculpt_wait = sculpt_wait.filter(|_| self.gpu[idx].geom != wanted);
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
            // Not ready: keep bounds for culling anyway. The pending set
            // brings it back when its geometry can progress.
            let g = &mut self.gpu[idx];
            g.center = pos;
            g.radius = radius;
            g.anchor = None;
            g.posed = None;
            g.wide = None;
            if let Some(o) = world.objects.get_mut_untracked(idx) {
                o.render.needs_records = false;
                o.shape_dirty = false;
                o.material_dirty = false;
            }
            return;
        };
        let Some(geom) = self.geom_ready(&geom_key) else {
            return;
        };
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        let Placement {
            model,
            center,
            radius,
            rigged,
            skeleton_owner,
            anchor,
            local_center,
            posed_rest,
        } = self
            .sync_ctx(world, &renderer.records, view.eye, now)
            .placement(idx, o, (pos, rot, hud), geom_key, &geom);

        let why = still_check::RebuildWhy {
            material: o.material_dirty,
            skeleton: self.gpu[idx].skeleton_owner != skeleton_owner.map(|p| p.0),
            rigged: self.gpu[idx].rigged != rigged,
            shape: o.shape_dirty,
            // compared with the faces built, not drawn: fully transparent
            // faces get no record and made this rebuild every frame
            faces: self.gpu[idx].built_faces != geom.faces.iter().filter(|f| f.is_some()).count(),
            geometry: bind_new.is_some(),
            material_generation: self.mat_changes.stale(
                &self.gpu[idx].material_ids,
                self.gpu[idx].mat_generation,
                self.materials.generation,
            ),
        };
        let full_rebuild = why.any();

        if full_rebuild {
            self.stats.rebuilt += 1;
            if let Some(still) = &mut self.still {
                still.rebuilt(o.full_id, why, || {
                    format!(
                        "{}, was {:?}, {:.2} m away",
                        still_check::describe(o, lod),
                        previous_geom,
                        pos.distance(view.eye)
                    )
                });
            }
            self.rebuild_faces(renderer, world, idx, &geom, model, anchor);
        } else {
            // transform-only update; an unchanged record is not uploaded again
            let model = model.to_cols_array_2d();
            for f in &self.gpu[idx].faces {
                if hud {
                    renderer.records.set_hud_model(f.record, model, world.hud_scale(idx));
                } else {
                    renderer.records.set_placement(f.record, model, anchor.map(|a| a.matrix));
                }
            }
        }
        self.anchors.placed(world, idx, anchor);
        if anchor.is_some() {
            self.sets.lod.forget(idx);
        }
        // (after the faces: a rigged mesh gets its skin bindings with them)
        let posed = self.posed_of(renderer, Some(geom_key), posed_rest);
        let rig_order = skeleton_owner
            .filter(|_| rigged)
            .map(|(_, skeleton)| blend_order::RigOrder::of(world, idx, skeleton));
        let g = &mut self.gpu[idx];
        g.center = center;
        g.radius = radius;
        g.anchor = anchor;
        g.local_center = local_center;
        g.posed = posed;
        g.wide = g.wide.filter(|_| posed.is_some());
        g.rigged = rigged;
        g.skeleton_owner = skeleton_owner.map(|p| p.0);
        g.rig_order = rig_order;
        g.is_avatar = false;
        g.hud = hud;
        if let Some(o) = world.objects.get_mut_untracked(idx) {
            o.render.needs_records = false;
            o.shape_dirty = false;
            o.material_dirty = false;
        }
    }

    /// What the GPU poses the bounds of a rigged mesh from, when it places
    /// it (`rest`: the sphere of its skeleton, `Placement::posed_rest`):
    /// the joint boxes of its geometry, given their place in the GPU table
    /// here, and the skin bindings of its mesh. Without them the skeleton's
    /// sphere stands for its bounds until its next visit.
    fn posed_of(&mut self, renderer: &mut Renderer, key: Option<GeomKey>, rest: Option<[f32; 4]>) -> Option<anchors::Posed> {
        let mut posed = anchors::Posed {
            boxes: 0,
            count: 0,
            binds: 0,
            rest: rest?,
        };
        if let Some(key @ GeomKey::Mesh { id, .. }) = key
            && let Some(binds) = self.skin_binds.base_of(&id)
            && let Some(GeomState::Ready(geom)) = self.geoms.get(&key).map(|e| &e.state)
            && let Some((boxes, count)) = self
                .joint_boxes
                .range(key, &geom.joint_bounds, |i, b| renderer.cull.boxes.set(i, b))
        {
            posed.boxes = boxes;
            posed.count = count;
            posed.binds = binds;
        }
        Some(posed)
    }

    /// The object owning the skeleton that drives a rigged mesh: the wearing
    /// avatar, or the root of an animated-mesh (animesh) linkset.
    pub fn skeleton_owner(world: &World, idx: usize) -> Option<(Uuid, usize)> {
        animesh::owner(&world.objects, idx)
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

    /// `model`: the object's placement, relative to `anchor` when the GPU
    /// places it (a worn object, anchors.rs).
    fn rebuild_faces(
        &mut self,
        renderer: &mut Renderer,
        world: &World,
        idx: usize,
        geom: &GpuGeom,
        model: Mat4,
        anchor: Option<anchors::AnchorRef>,
    ) {
        let Some(o) = world.objects.get(idx) else {
            return;
        };
        // release previous records/textures
        let mut g = std::mem::take(&mut self.gpu[idx]);
        self.drop_faces(renderer, idx, &mut g);
        let old_tex: Vec<Uuid> = std::mem::take(&mut g.tex_ids);
        g.material_ids.clear();
        let wearer = Self::wearer_avatar(world, idx);
        let bake_wearer = Self::bake_wearer(world, idx, g.hud);
        let rigged_owner = match g.geom {
            Some(GeomKey::Mesh { id, .. }) => match self.meshes.meta(&id).and_then(|m| m.skin).filter(|s| !s.joint_names.is_empty()) {
                Some(skin) => {
                    Self::skeleton_owner(world, idx).map(|(owner, i)| (owner, i, self.skin_binds.mesh(id, &skin, &self.avatar_lib.rig)))
                }
                None => None,
            },
            _ => None,
        };
        let hud_zoom = if g.hud { world.hud_scale(idx) } else { 1.0 };
        let selected_faces = self.selection_faces.get(&idx).copied().unwrap_or(0);
        // a Linden plant: its species texture, alpha-tested at 0.5, whatever
        // its texture entry says
        let plant = plants::plant_face(o);
        let visible = world.rlva_object_visible(idx);
        for (fi, face) in geom.faces.iter().enumerate() {
            if !visible {
                continue;
            }
            let Some(alloc) = face else {
                continue;
            };
            let tf = plant.unwrap_or_else(|| o.visual_texture_face(fi));
            let mat_id = o.visual_material(fi).filter(|_| plant.is_none());
            // a glTF material ignores the texture entry's color: its own base
            // color alpha decides, in `classify`
            let invisible = tf.color[3] < 0.004 && mat_id.is_none();
            if invisible
                && self.debug_alpha.is_none()
                && !self.pass_view
                && !self.beacon_highlights.contains(&idx)
                && !self.selection_wire.contains_key(&idx)
                && selected_faces & 1u64.checked_shl(fi as u32).unwrap_or(0) == 0
            {
                // fully transparent face: LL registers it as PASS_ALPHA_INVISIBLE,
                // never drawn, and its glow is not drawn either
                continue;
            }
            let mut rec = te_record_base(model, &tf);
            // Linden plants cut at 0.5, not the 0.33 of other faces without a
            // material (LLDrawPoolTree, setMinimumAlpha(0.5))
            if plant.is_some() {
                rec.params[3] = 0.5;
            }
            // LLVOVolume: prims of the "light" material are fullbright
            if o.prim_material == LL_MCODE_LIGHT {
                rec.flags[0] |= flags::FULLBRIGHT;
            }
            let mut pbr_alpha = None;
            let mut aux_tex = [Uuid::nil(); 3];
            // texture streaming: the face's repeats, the texture entry's
            // smaller scale for every map (LLViewerTextureList::
            // updateImageDecodePriority: glTF transforms not counted), or
            // the frame grid of a texture animation (below)
            let mut repeats = tf.scale_s.abs().min(tf.scale_t.abs());
            let mut two_sided = false;
            // GLTF material?
            let mut base_tex = tf.texture;
            if let Some(mid) = mat_id {
                g.material_ids.push(mid);
                // render material: the asset with the face's override on
                // top, or the default material while the asset loads or
                // when it cannot be loaded; never the texture entry's
                // diffuse texture (see meshes::render_material)
                let ov = world
                    .gltf_overrides
                    .get(&o.key)
                    .and_then(|s| s.iter().rev().find(|(f, _)| *f as usize == fi))
                    .map(|(_, ov)| ov)
                    .filter(|_| o.local_appearance(fi).is_none());
                let m = meshes::render_material(&self.materials.resolve(&mid), ov);
                // glTF materials replace the legacy shininess
                rec.flags[0] &= !flags::LEGACY_MAT;
                rec.flags[0] |= flags::PBR;
                rec.flags[0] &= !flags::FULLBRIGHT;
                // the material's base color replaces the face color (LLFace::getGeometryVolume)
                rec.base_color = m.base_color_factor;
                rec.params[1] = m.metallic_factor;
                rec.params[2] = m.roughness_factor;
                rec.params[3] = m.alpha_cutoff;
                rec.emissive = [m.emissive_factor[0], m.emissive_factor[1], m.emissive_factor[2], tf.glow];
                // one transform per map; the texture entry's repeats do not
                // apply to glTF faces (LLFace::getGeometryVolume)
                let so = |t: &aurora_assets::material::TextureTransform| [t.scale[0], t.scale[1], t.offset[0], t.offset[1]];
                let [tb, tn, tmr, te_] = &m.transforms;
                rec.uv_st = so(tb);
                rec.params[0] = tb.rotation;
                rec.mat_uv = so(tn);
                rec.spec_uv = so(tmr);
                rec.spec_color = so(te_);
                rec.legacy = [tn.rotation, tmr.rotation, te_.rotation, 0.0];
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
                if two_sided {
                    rec.flags[0] |= flags::DOUBLE_SIDED;
                }
            }
            // legacy material (LLMaterial) of the texture entry
            let mut legacy_alpha = plant.map(|_| legacy_mat::alpha_mode::MASK);
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
            // texture (with bake-on-mesh redirection); without a bake wearer
            // the placeholder stays, an ordinary texture asset
            let mut source = TexSource::Asset;
            if let Some((bake_te, bake)) = avatar::bake_slot_for(&base_tex)
                && let Some((aid, a)) = bake_wearer
            {
                match self.shown_bake(renderer, world, a, bake_te) {
                    Some(tid) => {
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
            if wearer.is_none()
                && plant.is_none()
                && let Some(texture) = world.rlva.camera_texture()
            {
                base_tex = texture;
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
            // texture animation: written once, evaluated by the vertex shaders
            let pbr = rec.flags[0] & flags::PBR != 0;
            if plant.is_none()
                && let Some(a) = tex_anim_record(o, fi, geom.faces.len(), &tf, pbr, renderer.anim_clock())
            {
                rec.flags[0] |= flags::TEX_ANIM;
                rec.flags[3] = a.packed;
                rec.anim = a.anim;
                rec.anim_xf = a.constants;
                // A flipbook shows one frame of its texture at a time: it is
                // loaded for the frame's size on screen (#1229). The Legacy
                // Viewer does so only at rate 0, where the frame goes into the
                // texture entry's scale (LLVOVolume::animateTextures); a running
                // animation keeps the entry's scale there, and a 4 x 4 grid
                // loads four times too coarse each way.
                if let Some(r) = a.frame_repeats {
                    repeats = r;
                }
            }
            rec.set_hud_zoom(hud_zoom);
            rec.set_anchor(anchor.map(|a| a.matrix));
            let record = renderer.records.alloc(rec);
            g.faces.push(FaceDraw {
                te_index: fi as u8,
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
                te_alpha: if invisible { 0.0 } else { rec.base_color[3] },
                pbr_alpha,
                legacy_alpha,
                two_sided,
                repeats,
                glow: rec.emissive[3] > 0.0,
                pass: Pass::Hidden,
                glow_pool: false,
                rez_glow: false,
                system: None,
                first_person_hidden: false,
            });
        }
        for t in old_tex {
            self.textures.release(&t);
        }
        g.tex_generation = self.textures.generation;
        g.mat_generation = self.materials.generation;
        g.built_faces = geom.faces.iter().filter(|f| f.is_some()).count();
        // g.rigged is set only after rebuild_faces returns; use the skin we
        // just bound so the first classify already skips auto-Mask.
        let object_auto_mask = rigged_owner.is_none() && !g.hud;
        self.finish_faces(renderer, idx, &mut g, object_auto_mask);
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

    /// The avatar whose bakes replace the "use baked" placeholders of object
    /// `idx`: its wearer, unless it is a HUD or an animated object, whose
    /// skeleton is its own. On the ground, on a HUD or on an animesh the
    /// placeholder is drawn as the texture asset it is
    /// (LLViewerObject::getBakedTextureForMagicId, llviewerobject.cpp).
    fn bake_wearer(world: &World, idx: usize, hud: bool) -> Option<(Uuid, usize)> {
        let wearer = Self::wearer_avatar(world, idx)?;
        (!hud && Self::skeleton_owner(world, idx) == Some(wearer)).then_some(wearer)
    }

    /// LLVOAvatarSelf::updateAttachmentVisibility uses the root attachment's
    /// avatar_lad.xml flag for all its linked prims, rigged or not.
    fn attachment_first_person_hidden(world: &World, mut idx: usize) -> bool {
        for _ in 0..64 {
            let Some(object) = world.objects.get(idx) else { return false };
            let Some(parent) = world.objects.parent_of(object) else {
                return false;
            };
            if world.objects.get(parent).is_some_and(|p| p.is_avatar()) {
                return world
                    .avatar_lib
                    .attach_points
                    .get(&object.attachment_point())
                    .is_some_and(|p| !p.hud && !p.visible_in_first_person);
            }
            idx = parent;
        }
        false
    }

    fn first_person_object_visible(&self, idx: usize, g: &ObjGpu) -> bool {
        g.hud
            || self.first_person.is_none()
            || self.agent_idx.is_none()
            || g.avatar(idx) != self.agent_idx
            || (self.first_person == Some(true) && !g.first_person_hidden)
    }

    pub fn set_first_person(&mut self, show: Option<bool>) {
        if self.first_person != show {
            self.first_person = show;
            self.hover.invalidate();
        }
    }

    /// Object `idx` changed or arrived through the store: if it is worn, what
    /// its wearer's attachments show of its bakes is looked at again.
    fn note_worn_change(&mut self, world: &World, idx: usize) {
        if world.objects.get(idx).is_some_and(|o| !o.is_avatar())
            && let Some((_, wearer)) = Self::wearer_avatar(world, idx)
        {
            self.bom_stale.insert(wearer);
        }
    }

    /// Bake slots (texture-entry index bits) shown by the avatar's attachments
    /// and their child prims.
    pub(crate) fn bom_mask(world: &World, avatar_idx: usize) -> u64 {
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
            if Self::skeleton_owner(world, att).is_some_and(|(_, i)| i != avatar_idx) {
                continue;
            }
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
        // its records read the skeleton's place in the palette (anchors.rs):
        // nothing to write again when the avatar moves or turns
        let anchor = anchors::enabled()
            .then(|| Self::anchor_matrix(&self.palette_slots, self.palettes.len(), o, 0))
            .flatten();
        if anchor.is_some() {
            self.anchors.keep(idx as u32, o.full_id, 0);
        }
        let record_model = if anchor.is_some() { Mat4::IDENTITY } else { avatar_model }.to_cols_array_2d();
        let system_binds = self.skin_binds.system(&self.avatar_lib.rig);
        let appearance_gen = world.appearance_generation(&o.full_id);
        // bakes used by attachments: when something worn changed, and every
        // BOM_SWEEP_FRAMES in case (each time it walks every prim worn)
        let bom_mask =
            if !self.gpu[idx].is_avatar || self.bom_stale.remove(&idx) || (self.frame as usize + idx).is_multiple_of(BOM_SWEEP_FRAMES) {
                Self::bom_mask(world, idx)
            } else {
                self.gpu[idx].bom_mask
            };
        // over the complexity limit: grey silhouette of the system body
        // (Pipeline::generateImpostor: avatar geometry only, alpha masking
        // off, filled with the muted color LLColor4::grey4)
        let jelly = self.too_complex.contains(&idx);
        let rlva_silhouette = self.rlva_silhouettes.contains(&idx);
        let g = &self.gpu[idx];
        // (a morphed part that was being built is there, or never will be)
        let morphs_arrived = g.morph_pending && g.morph_geoms.iter().all(|k| self.geom_ready(k).is_some() || self.geom_failed(k));
        let needs_full = !g.is_avatar
            || g.faces.len()
                != lib
                    .parts
                    .iter()
                    .filter(|p| world.rlva_body_part_visible(o.full_id == world.agent_id, p.name))
                    .count()
            || g.tex_generation != appearance_gen
            || g.bom_mask != bom_mask
            || g.jelly != jelly
            || g.rlva_silhouette != rlva_silhouette
            || morphs_arrived;
        if needs_full {
            let mut g = std::mem::take(&mut self.gpu[idx]);
            self.drop_faces(renderer, idx, &mut g);
            let old_tex: Vec<Uuid> = std::mem::take(&mut g.tex_ids);
            // the body parts as the avatar's shape morphs them (morph.rs)
            let mut weights: Option<HashMap<i32, f32>> = None;
            let held = std::mem::take(&mut g.morph_geoms);
            g.morph_pending = false;
            for (pi, part) in lib.parts.iter().enumerate() {
                if !world.rlva_body_part_visible(o.full_id == world.agent_id, part.name) {
                    continue;
                }
                let tex = self.shown_bake(renderer, world, idx, part.bake_te);
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
                // (a hidden part is not morphed: most avatars hide the whole
                // system body under a mesh one)
                let mut key = GeomKey::AvatarPart(pi as u8);
                let morphed = (!hidden).then(|| {
                    let w = weights.get_or_insert_with(|| {
                        let mut w = lib.shape_params.effective_weights(world.visual_params(&o.full_id).unwrap_or(&[]));
                        // the relaxed hands of LLHandMotion, the pose most of
                        // the time, are baked: the live weights move from them
                        w.insert(default_motions::RELAXED_HANDS, 1.0);
                        w
                    });
                    self.avatar_morph(pi as u8, w)
                });
                if let Some(morphed) = morphed.flatten() {
                    self.geom_ref(morphed);
                    g.morph_geoms.push(morphed);
                    if self.geom_ready(&morphed).is_some() {
                        key = morphed;
                    } else {
                        // drawn as its file has it until the morph is built
                        g.morph_pending |= !self.geom_failed(&morphed);
                    }
                }
                let Some(geom) = self.geom_ready(&key) else {
                    continue;
                };
                let Some(Some(alloc)) = geom.faces.first().copied() else {
                    continue;
                };
                let minimum_alpha = if part.name == "skirt" {
                    AVATAR_SKIRT_MINIMUM_ALPHA
                } else {
                    AVATAR_MINIMUM_ALPHA
                };
                let mut rec = DrawRecord {
                    model: record_model,
                    params: [0.0, 0.0, 0.8, minimum_alpha],
                    flags: [flags::SKINNED, palette, system_binds, 0],
                    ..Default::default()
                };
                if let Some((base, count, moving)) = self.live_morphs.parts.get(pi).copied().flatten() {
                    rec.flags[0] |= flags::LIP_SYNC;
                    rec.anim = [alloc.vertex_offset, base, count, moving];
                }
                let mut slot = aurora_render::textures::WHITE;
                if jelly {
                    rec.base_color = if rlva_silhouette {
                        [0.05, 0.05, 0.05, 1.0]
                    } else {
                        [0.3, 0.3, 0.3, 1.0]
                    };
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
                rec.set_anchor(anchor);
                let record = renderer.records.alloc(rec);
                g.faces.push(FaceDraw {
                    te_index: 0,
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
                    te_alpha: if hidden { 0.0 } else { 1.0 },
                    pbr_alpha: None,
                    legacy_alpha: None,
                    two_sided: false,
                    repeats: 1.0,
                    glow: false,
                    pass: Pass::Hidden,
                    glow_pool: false,
                    rez_glow: false,
                    system: Some(SystemPart::of(part.name)),
                    first_person_hidden: matches!(part.name, "head" | "hair" | "eyelashes" | "eye_left" | "eye_right"),
                });
            }
            for t in old_tex {
                self.textures.release(&t);
            }
            // (after the new ones are held: a geometry kept is not let go)
            for k in held {
                self.geom_unref(k);
            }
            g.is_avatar = true;
            g.tex_generation = appearance_gen;
            g.bom_mask = bom_mask;
            g.jelly = jelly;
            g.rlva_silhouette = rlva_silhouette;
            // System parts set FaceDraw::system; auto-Mask rules do not apply.
            self.finish_faces(renderer, idx, &mut g, true);
            self.gpu[idx] = g;
        } else {
            // (unchanged records are not uploaded again)
            for f in &self.gpu[idx].faces {
                renderer.records.set_placement(f.record, record_model, anchor);
            }
        }
        let posed = self.posed_avatar_sphere(o.full_id, palette, system_binds, avatar_model, pos);
        let g = &mut self.gpu[idx];
        g.center = pos;
        g.radius = 1.2;
        g.posed_sphere = posed;
    }

    /// Sphere of an avatar's system body as posed by its current palette
    /// (joint boxes of the skinned vertices, as `animesh::posed_bounds`),
    /// for culling and occlusion; None before a sane pose exists.
    fn posed_avatar_sphere(&mut self, _id: Uuid, palette_base: u32, binds: u32, model: Mat4, pos: Vec3) -> Option<(Vec3, f32)> {
        if self.system_bounds.is_empty() {
            let mut boxes = HashMap::<u8, (Vec3, Vec3)>::new();
            for i in 0..self.avatar_lib.parts.len() {
                for b in self.geom_ready(&GeomKey::AvatarPart(i as u8))?.joint_bounds.iter() {
                    let e = boxes.entry(b.joint).or_insert((b.min, b.max));
                    e.0 = e.0.min(b.min);
                    e.1 = e.1.max(b.max);
                }
            }
            self.system_bounds = boxes
                .into_iter()
                .map(|(joint, (min, max))| animesh::JointBounds { joint, min, max })
                .collect();
        }
        let (c, r) = animesh::posed_bounds(&self.system_bounds, |j| {
            let b = self.skin_binds.list().get(binds as usize + j as usize)?;
            let p = self.palettes.get(palette_base as usize + b.joint[0] as usize)?;
            Some(model * Mat4::from_cols_array_2d(p) * Mat4::from_cols_array_2d(&b.inverse_bind))
        })?;
        // a palette not posed yet collapses the body (or throws it away)
        (c.is_finite() && (0.3..8.0).contains(&r) && c.distance(pos) < 4.0).then_some((c, r))
    }

    // ------------------------------------------------------------ terrain & water

    // ------------------------------------------------------------ culling

    /// Build this frame's draw lists.
    ///
    /// With the GPU lists (`Renderer::gpu_culling`, gpu_cull.rs) the GPU
    /// culls the opaque and masked faces, the object shadow casters and
    /// the reflection / probe draws from the scene's tables; this only
    /// visits the objects with blended or glowing faces (`ObjGpu::special`),
    /// a thirtieth of the objects for the texture usage, the selection,
    /// the pictured impostors and, in debug, everything. Without them (or
    /// AURORA_CPU_CULL=1) every object is visited and every list built here.
    pub fn build_lists(&mut self, renderer: &mut Renderer, view: &CullView, shadows: bool) {
        let t0 = Instant::now();
        self.lists.clear();
        self.lists.portal_clipping = !self.portals.planes.is_empty();
        self.last_cull = Some(*view);
        let gpu_mode = renderer.gpu_culling();
        let debug_alpha = self.debug_alpha;
        let pass_view = self.pass_view;
        let dd = self.draw_distance;
        let shadow_dist = dd.min(256.0);
        // terrain
        for (&handle, r) in &self.regions {
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
                    self.lists.terrain_shadow_casters.push(ShadowCaster {
                        cmd,
                        center: c.center,
                        radius: c.radius,
                        alpha: r.surface.masked,
                    });
                }
                if self.reflections {
                    // with its bounds: each reflection pass keeps the chunks
                    // in its own frustum
                    let reflected = cmd.with_bounds(c.center, c.radius);
                    if r.surface.pbr {
                        self.lists.reflection_terrain_pbr.push(reflected);
                    } else {
                        self.lists.reflection_terrain.push(reflected);
                    }
                }
                if view.sphere_visible(c.center, c.radius) {
                    // one shader per kind of terrain (env.wgsl)
                    if r.surface.masked {
                        self.lists.terrain_mask.push(cmd);
                    } else if r.surface.pbr {
                        self.lists.terrain_pbr.push(cmd);
                    } else {
                        self.lists.terrain.push(cmd);
                    }
                    for record in self.land_overlays.records(handle) {
                        self.lists.land_overlay.push(DrawCmd { record, ..cmd });
                    }
                }
            }
        }
        self.lists.water.extend(self.water.draws(view, self.water_effects.geometry));

        // nearest avatars in full (own avatar always), the others as
        // impostors (RenderAvatarMaxNonImpostors)
        let index_mode = list_index::mode();
        // (a thirtieth of the frames: the check walks every object)
        if index_mode == list_index::Mode::Check && self.frame.is_multiple_of(30) {
            self.list_index.verify(&self.gpu);
        }
        let by_index = index_mode != list_index::Mode::Walk;
        let avatar_distance = |i: usize, g: &ObjGpu| {
            if Some(i) == self.agent_idx {
                -1.0
            } else {
                g.center.distance(view.eye)
            }
        };
        // (those the photo's isolation leaves out take no place, not even
        // as an impostor)
        let mut avatars: Vec<(f32, usize)> = if by_index {
            self.list_index
                .with(list_index::AVATAR)
                .filter_map(|(i, _)| Some(i).zip(self.gpu.get(i).filter(|g| !g.photo_hidden)))
                .map(|(i, g)| (avatar_distance(i, g), i))
                .collect()
        } else {
            self.gpu
                .iter()
                .enumerate()
                .filter(|(_, g)| g.is_avatar && !g.photo_hidden)
                .map(|(i, g)| (avatar_distance(i, g), i))
                .collect()
        };
        avatars.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut avatar_ok = vec![false; self.gpu.len()];
        for (_, i) in avatars.iter().take(self.max_avatars.max(1)) {
            avatar_ok[*i] = true;
        }
        for &index in self.portals.planes.keys() {
            avatar_ok[index] = true;
        }
        // (as when drawn in full, one still loading is a cloud until it is
        // complete, and one over the complexity limit is its silhouette)
        let imp_avatars: Vec<impostors::Far> = avatars
            .iter()
            .skip(self.max_avatars.max(1))
            .filter(|(_, index)| !self.portals.planes.contains_key(index))
            .filter_map(|&(d, i)| {
                let g = self.gpu.get(i)?;
                let loading = self.hide_loading && self.loading.indices.contains(&i);
                (d < dd && !g.faces.is_empty() && !loading).then(|| impostors::Far {
                    idx: i,
                    center: g.center,
                    distance: d,
                    in_view: view.sphere_visible(g.center, 1.6),
                    silhouette: self.too_complex.contains(&i),
                })
            })
            .collect();
        let mut avatar_far = vec![false; self.gpu.len()];
        for a in &imp_avatars {
            avatar_far[a.idx] = true;
        }
        let mut plan = self.impostors.plan(&imp_avatars, view.eye, Instant::now());
        self.far_avatars.clear();
        self.far_avatars.extend(imp_avatars.iter().map(|a| (a.idx, *a)));
        if self.sync_check.is_some() {
            let (taken, stale, since) = &mut self.picture_check;
            *taken += plan.capture.len() as u64;
            *stale += plan.capture.keys().filter(|i| !self.posed_now.contains(i)).count() as u64;
            let since = since.get_or_insert_with(Instant::now);
            if since.elapsed().as_secs() >= 10 {
                let (kept, wrong) = std::mem::take(&mut self.anchors.kept_centres);
                let (anchors, wrong_anchors) = std::mem::take(&mut self.anchors.kept_anchors);
                log::info!(
                    "impostor pictures: {taken} taken, {stale} with the pose of an earlier frame | anchored centres kept: {kept}, {wrong} not where their anchor puts them | anchors kept: {anchors}, {wrong_anchors} not what their wearer gives"
                );
                self.picture_check = (0, 0, Some(Instant::now()));
            }
        }
        self.stats.avatars_hidden = imp_avatars.len();
        if aurora_net::probe::enabled() {
            let full: Vec<usize> = avatars.iter().filter(|(_, i)| avatar_ok[*i]).map(|(_, i)| *i).collect();
            self.stats.avatars = AvatarCounts {
                total: avatars.len(),
                full: full.len(),
                full_in_view: full
                    .iter()
                    .filter(|&&i| self.gpu.get(i).is_some_and(|g| view.sphere_visible(g.center, 1.6)))
                    .count(),
                impostors: imp_avatars.len(),
                impostors_in_view: imp_avatars.iter().filter(|a| a.in_view).count(),
                pictured: plan.capture.len(),
                silhouettes: avatars.iter().filter(|(_, i)| self.too_complex.contains(i)).count(),
                loading: if self.hide_loading {
                    avatars.iter().filter(|(_, i)| self.loading.indices.contains(i)).count()
                } else {
                    0
                },
            };
        }
        // avatar states of the GPU object table (few avatars: every frame)
        for &(_, i) in &avatars {
            let mut st = 0;
            if avatar_ok[i] {
                st |= avatar_state::FULL;
            }
            if self.too_complex.contains(&i) {
                st |= avatar_state::TOO_COMPLEX;
            }
            if self.hide_loading && self.loading.indices.contains(&i) {
                st |= avatar_state::LOADING;
            }
            if self.gpu[i].cull_state != st {
                self.gpu[i].cull_state = st;
                renderer.cull.objects.set(i, self.gpu[i].cull_object(i));
            }
        }
        let hidden_loading = if self.hide_loading { Some(&self.loading.indices) } else { None };
        let reflections = self.reflections;
        let refl_dist = dd.min(128.0);

        // objects visited here: all of them on the CPU path; with the GPU
        // lists the special ones, the usage slice, the selection, the
        // pictured impostors and, in debug, all
        // under memory pressure the textures go back to their size on screen:
        // no importance boost (the Legacy Viewer divides it out with its
        // discard bias, updateImageDecodePriority)
        let boost_textures = self.textures.stats.pressure == 0;
        let usage_phase = (self.frame % 30) as usize;
        // Objects whose faces were built since the last frame have the size
        // of their textures on screen noted now rather than at their turn,
        // up to 30 frames away: a new texture is then asked for at the rank
        // and for the range that size calls for, not blindly.
        let mut fresh = std::mem::take(&mut self.fresh_usage);
        let usage_due = |i: usize| i % 30 == usage_phase || fresh.contains(&i);
        let mut visit = std::mem::take(&mut self.visit_tmp);
        visit.clear();
        if gpu_mode {
            let debug = debug_alpha.is_some() || pass_view;
            let selection = &self.selection_wire;
            let captured = |g: &ObjGpu, i: usize| !plan.capture.is_empty() && g.avatar(i).is_some_and(|a| plan.capture.contains_key(&a));
            if by_index {
                // (the pictured avatars and what they wear: read in the
                // objects, on the frames a picture is taken)
                let capturing = !plan.capture.is_empty();
                for (i, bits) in self.list_index.with(list_index::DRAWN) {
                    let usage = usage_due(i);
                    if usage
                        || bits & list_index::SPECIAL != 0
                        || debug
                        || selection.contains_key(&i)
                        || self.selection_faces.contains_key(&i)
                        || self.photo_marks.contains(&i)
                        || self.give_marks.1.contains(&i)
                        || self.beacon_highlights.contains(&i)
                        || (capturing && self.gpu.get(i).is_some_and(|g| captured(g, i)))
                    {
                        visit.push((i, usage));
                    }
                }
            } else {
                for (i, g) in self.gpu.iter().enumerate() {
                    if g.faces.is_empty() || g.hud {
                        continue;
                    }
                    let usage = usage_due(i);
                    if usage
                        || g.special
                        || debug
                        || selection.contains_key(&i)
                        || self.selection_faces.contains_key(&i)
                        || self.photo_marks.contains(&i)
                        || self.give_marks.1.contains(&i)
                        || self.beacon_highlights.contains(&i)
                        || captured(g, i)
                    {
                        visit.push((i, usage));
                    }
                }
            }
        } else {
            visit.extend((0..self.gpu.len()).map(|i| (i, usage_due(i))));
        }
        fresh.clear();
        self.fresh_usage = fresh;
        let full = !gpu_mode;
        let ctx = CullCtx {
            view,
            draw_distance: dd,
            shadow_distance: shadow_dist,
            reflection_distance: refl_dist,
            shadows,
            reflections,
        };

        // objects in parallel
        let blend_casts = aurora_render::shadow_cull::blend_casts();
        let selection_wire = &self.selection_wire;
        let selection_faces = &self.selection_faces;
        let photo_marks = &self.photo_marks;
        let give_marks = &self.give_marks.1;
        struct Local {
            water_exclusion: Vec<DrawCmd>,
            opaque: Vec<DrawCmd>,
            opaque2: Vec<DrawCmd>,
            mask: Vec<DrawCmd>,
            mask2: Vec<DrawCmd>,
            blend: Vec<blend_order::BlendEntry>,
            glow: Vec<DrawCmd>,
            glow_alpha: Vec<DrawCmd>,
            /// Draws of the avatars pictured for their impostor: (capture slot, draw, blended).
            impostor: Vec<(usize, DrawCmd, bool)>,
            debug_red: Vec<DrawCmd>,
            select_root: Vec<DrawCmd>,
            select_child: Vec<DrawCmd>,
            select_faces: Vec<DrawCmd>,
            debug_blue: Vec<DrawCmd>,
            debug_green: Vec<DrawCmd>,
            debug_yellow: Vec<DrawCmd>,
            casters: Vec<ShadowCaster>,
            refl: Vec<ShadowCaster>,
            probe: Vec<ShadowCaster>,
            visible: usize,
            usage: Vec<(Uuid, f32, bool)>,
        }
        let new_local = || Local {
            water_exclusion: Vec::new(),
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
            select_faces: Vec::new(),
            debug_blue: Vec::new(),
            debug_green: Vec::new(),
            debug_yellow: Vec::new(),
            casters: Vec::new(),
            refl: Vec::new(),
            probe: Vec::new(),
            visible: 0,
            usage: Vec::new(),
        };
        let result = frame_par::fold(&visit, LISTS_GRAIN, new_local, |mut l: Local, &(idx, usage)| {
            let Some(g) = self.gpu.get(idx) else {
                return l;
            };
            if g.faces.is_empty() || g.hud {
                return l;
            }
            if g.photo_hidden {
                // left out of the photo, not of the scene: its textures held
                // as out of view, so that memory pressure lowers them like
                // any other out of view instead of down to nothing, and the
                // entire scene comes back as it was rather than reloading
                // all at once with the video memory full (#1374)
                if usage {
                    let px = texture_importance::texture_pixels(view, g.center, g.radius, false);
                    for f in g.faces.iter().filter(|f| f.pass != Pass::Hidden) {
                        note_face_usage(&mut l.usage, f, px, false);
                    }
                }
                return l;
            }
            let own_first_person = self
                .first_person
                .filter(|_| self.agent_idx.is_some() && g.avatar(idx) == self.agent_idx);
            if own_first_person == Some(false) {
                return l;
            }
            // avatar limit: far avatars and everything they wear are drawn
            // as impostors; pictured ones give their draws to the picture
            let avatar = g.avatar(idx);
            if let Some(a) = avatar.filter(|a| !avatar_ok.get(*a).copied().unwrap_or(true)) {
                // (too complex: its silhouette alone, without what it wears)
                if !avatar_far.get(a).copied().unwrap_or(false) || g.owner_avatar.is_some_and(|a| self.too_complex.contains(&a)) {
                    return l;
                }
                if let Some(&slot) = plan.capture.get(&a) {
                    for f in &g.faces {
                        match f.pass {
                            Pass::Hidden => {}
                            Pass::Blend => l.impostor.push((slot, f.cmd, true)),
                            _ => l.impostor.push((slot, f.cmd, false)),
                        }
                    }
                }
                if usage {
                    // its textures as fine as the picture shows them: without
                    // a size they stay at their coarsest level, and an avatar
                    // that was never among the nearest is a blur of colors
                    let (sc, sr) = g.sphere();
                    let in_view = view.sphere_visible(sc, sr);
                    let px = view.pixel_size(g.center, g.radius).min(impostors::picture_pixels(g.radius));
                    for f in g.faces.iter().filter(|f| f.pass != Pass::Hidden) {
                        note_face_usage(&mut l.usage, f, px, in_view);
                    }
                }
                return l;
            }
            let Some(ObjVis {
                sphere: (sc, sr),
                in_view,
                casts,
                reflects,
            }) = object_visibility(
                g,
                idx,
                &ctx,
                |a| avatar_ok.get(a).copied().unwrap_or(true),
                |a| self.too_complex.contains(&a),
                |a| hidden_loading.is_some_and(|h| h.contains(&a)),
            )
            else {
                return l;
            };
            if in_view {
                l.visible += 1;
            }
            // texture usage and blend order on the object's own bounds: its
            // textures as the Legacy Viewer sizes them, finer near the camera
            // and in the middle of the view
            let px = texture_importance::texture_pixels(view, g.center, g.radius, in_view && boost_textures);
            // a rigged mesh's blended faces take their place among those of
            // its skeleton, by the skeleton's distance (`blend_order`)
            let rig = g.rig_order.filter(|_| g.rigged);
            let blend_center = rig.and_then(|r| self.gpu.get(r.skeleton as usize)).map_or(g.center, |s| s.center);
            let blend_d = (blend_center - view.eye).length();
            for f in &g.faces {
                // object sphere for the GPU occlusion test
                let in_view = in_view && !(own_first_person == Some(true) && (g.first_person_hidden || f.first_person_hidden));
                let cmd = f.cmd.with_bounds(sc, sr);
                let pass = f.pass;
                // blended faces glow in their sorted place; hidden faces do not glow
                if f.glow && in_view && pass != Pass::Blend && pass != Pass::Hidden {
                    if f.glow_pool {
                        l.glow_alpha.push(cmd);
                    } else {
                        l.glow.push(cmd);
                    }
                }
                // debug "Show transparency" (renderDebugAlpha): blended,
                // masked and invisible faces in red, material masks in blue;
                // rigged faces only when asked (Firestorm sShowDebugAlphaRigged)
                if in_view && self.beacon_highlights.contains(&idx) {
                    l.debug_red.push(cmd);
                } else if let Some(rigged_too) = debug_alpha.filter(|_| in_view)
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
                if pass_view && in_view {
                    match pass {
                        Pass::Opaque | Pass::OpaqueTwoSided => l.debug_green.push(cmd),
                        Pass::Mask | Pass::MaskTwoSided => l.debug_blue.push(cmd),
                        Pass::Blend => l.debug_red.push(cmd),
                        Pass::Hidden => l.debug_yellow.push(cmd),
                    }
                }
                if in_view {
                    match selection_wire.get(&idx) {
                        Some(true) => l.select_root.push(cmd),
                        Some(false) => l.select_child.push(cmd),
                        None => {}
                    }
                    if selection_faces
                        .get(&idx)
                        .is_some_and(|mask| mask & 1u64.checked_shl(f.te_index.into()).unwrap_or(0) != 0)
                        || ((photo_marks.contains(&idx) || give_marks.contains(&idx)) && pass != Pass::Hidden)
                    {
                        l.select_faces.push(cmd);
                    }
                }
                if pass == Pass::Hidden {
                    // LLVolumeGeometryManager excludes attachments and rigged
                    // meshes from PASS_INVISIBLE. No ordinary depth or shadow.
                    if full && in_view && f.water_exclusion() && avatar.is_none() && !g.rigged {
                        l.water_exclusion.push(cmd);
                    }
                    continue;
                }
                if full {
                    // LLPipeline::renderShadow includes alpha-blended faces,
                    // rigged (fur, hair) or not (glass, foliage), with an
                    // alpha-tested shadow (fs_shadow)
                    if casts && (pass != Pass::Blend || g.rigged || g.is_avatar || blend_casts) {
                        l.casters.push(ShadowCaster {
                            cmd,
                            center: sc,
                            radius: sr,
                            alpha: pass.record_flags() != 0,
                        });
                    }
                    if reflects && matches!(pass, Pass::Opaque | Pass::OpaqueTwoSided | Pass::Mask | Pass::MaskTwoSided) {
                        let c = ShadowCaster {
                            cmd,
                            center: sc,
                            radius: sr,
                            alpha: false,
                        };
                        l.refl.push(c);
                        if avatar.is_none() && g.skeleton_owner.is_none() {
                            l.probe.push(c);
                        }
                    }
                }
                if usage {
                    // (out of view too: what memory pressure may not
                    // lower a texture under, see `note_usage_away`)
                    note_face_usage(&mut l.usage, f, px, in_view);
                }
                if !in_view {
                    continue;
                }
                match pass {
                    Pass::Blend => l.blend.push(blend_order::BlendEntry {
                        distance: blend_d,
                        cmd,
                        glow: f.glow,
                        rigged: g.rigged,
                        object: idx as u32,
                        face: f.te_index,
                        rig,
                    }),
                    _ if !full => {}
                    Pass::Opaque => l.opaque.push(cmd),
                    Pass::OpaqueTwoSided => l.opaque2.push(cmd),
                    Pass::Mask => l.mask.push(cmd),
                    Pass::MaskTwoSided => l.mask2.push(cmd),
                    Pass::Hidden => {}
                }
            }
            l
        })
        .into_iter()
        .reduce(|mut a, mut b| {
            a.water_exclusion.append(&mut b.water_exclusion);
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
            a.debug_green.append(&mut b.debug_green);
            a.debug_yellow.append(&mut b.debug_yellow);
            a.select_root.append(&mut b.select_root);
            a.select_child.append(&mut b.select_child);
            a.select_faces.append(&mut b.select_faces);
            a.casters.append(&mut b.casters);
            a.refl.append(&mut b.refl);
            a.probe.append(&mut b.probe);
            a.visible += b.visible;
            a.usage.append(&mut b.usage);
            a
        })
        .unwrap_or_else(new_local);
        self.visit_tmp = visit;
        self.lists.opaque = result.opaque;
        self.lists.water_exclusion = result.water_exclusion;
        self.lists.opaque_two_sided = result.opaque2;
        self.lists.mask = result.mask;
        self.lists.mask_two_sided = result.mask2;
        self.blend_tmp = result.blend;
        self.blend_tmp.extend(
            [(self.banlines.cmds.as_slice(), false), (self.portals.cmds.as_slice(), true)]
                .into_iter()
                .flat_map(|(cmds, glow)| {
                    cmds.iter().map(move |(d, c)| blend_order::BlendEntry {
                        distance: *d,
                        cmd: *c,
                        glow,
                        rigged: false,
                        object: u32::MAX,
                        face: 0,
                        rig: None,
                    })
                }),
        );
        self.lists.blend_depth = blend_order::sort(&mut self.blend_tmp, blend_order::rigged_apart());
        self.lists.blend_depth_mode = blend_order::depth_mode();
        let runs = match self.lists.blend_depth_mode {
            aurora_render::RiggedBlendDepth::Layers => blend_order::each_object,
            aurora_render::RiggedBlendDepth::Split => blend_order::rig_objects,
            _ => blend_order::rig_runs,
        };
        runs(&self.blend_tmp, self.lists.blend_depth, &mut self.lists.blend_depth_runs);
        self.lists.blend.extend(self.blend_tmp.iter().map(|e| e.cmd));
        self.lists.blend_glow.extend(self.blend_tmp.iter().map(|e| e.glow));
        self.lists.shadow_casters.extend(result.casters);
        self.lists.reflection = result.refl;
        self.lists.probe = result.probe;
        self.lists.glow = result.glow;
        self.lists.select_faces = result.select_faces;
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
        self.lists.debug_green = result.debug_green;
        self.lists.debug_yellow = result.debug_yellow;
        self.lists.select_root = result.select_root;
        self.lists.select_child = result.select_child;
        if gpu_mode {
            self.lists.gpu = Some(GpuCullView {
                first_person: self.agent_idx.and_then(|own| self.first_person.map(|show| (own as u32, show))),
                planes: view.planes,
                eye: view.eye,
                draw_distance: dd,
                shadow_distance: shadow_dist,
                reflection_distance: refl_dist,
                pixel_scale: view.screen_height / view.tan_half_fov.max(1e-6),
                shadows,
                reflections,
            });
            // counted by the GPU, a frame or two late
            self.stats.visible_objects = renderer.cull.last.visible_objects as usize;
        } else {
            self.stats.visible_objects = result.visible;
        }

        // texture usage: a thirtieth of the objects each frame, wants
        // recomputed once all were seen
        for (id, px, in_view) in result.usage {
            if in_view {
                self.textures.note_usage(&id, px);
            } else {
                self.textures.note_usage_away(&id, px);
            }
        }
        if usage_phase == 0 {
            // LLTerrainMaterials::boost: TERRAIN_DECODE_PRIORITY = 2048²,
            // for every material map too. A 512 px cap loses terrain relief.
            for r in self.regions.values() {
                for id in &r.surface.texture_ids {
                    self.textures.note_usage_fixed(id, terrain_surface::DECODE_PIXELS);
                }
            }
            self.textures.recompute_wants();
        }
        self.stats.cull_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    /// Per-frame streaming work.
    pub fn stream(&mut self, renderer: &mut Renderer, net: &NetClient, world: &World, texture_budget: u64, frame: std::time::Duration) {
        use crate::frame_profile::Part;
        let t = Instant::now();
        let va = world.viewer_asset_url();
        let va = va.as_deref();
        self.textures.set_agent(&world.agent_id);
        self.textures.update(&self.jobs, &net.fetcher, va);
        let t = self.parts.lap(Part::TexUpdate, t);
        let rig = self.avatar_lib.rig.clone();
        self.meshes.update(&self.jobs, &net.fetcher, va, &rig);
        self.anims.update(&self.jobs, &net.fetcher, va, &rig);
        self.sounds.fetch(&self.jobs, &net.fetcher, va);
        self.materials.update(&self.jobs, &net.fetcher, va);
        self.settings.update(&self.jobs, &net.fetcher, va);
        // (what arrived is counted by note_material_changes)
        self.legacy_mats.update(net.runtime(), &net.caps_http());
        self.note_material_changes();
        let t = self.parts.lap(Part::Assets, t);
        for dirty in self.skin_binds.take_dirty() {
            renderer.set_skin_binds(self.skin_binds.list(), dirty);
        }
        let t = self.parts.lap(Part::Skin, t);
        let pages_before = renderer.textures.page_create_ms();
        self.textures.upload(renderer, &self.jobs, textures::UploadBudget::for_frame(frame));
        self.parts.add(Part::Pages, renderer.textures.page_create_ms() - pages_before);
        let t = self.parts.lap(Part::Upload, t);
        self.textures.maintain(renderer, &self.jobs, texture_budget, frame);
        let t = self.parts.lap(Part::Maintain, t);
        if self.last_diag.elapsed() > std::time::Duration::from_secs(30) {
            self.last_diag = std::time::Instant::now();
            let (ready, fetching, failed) = self.meshes.counts();
            let t = &self.textures.stats;
            let (lm_ready, lm_wait, lm_unknown) = self.legacy_mats.counts();
            let (gm_ready, gm_wait, gm_missing) = self.materials.counts();
            let (binds_used, binds_len) = self.skin_binds.counts();
            let pages = renderer.textures.pages_made();
            log::info!(
                "streaming: {} objects, meshes {ready} ready / {fetching} downloading / {failed} failed, skin bindings {binds_used} used of {binds_len}, textures {}/{} ({} with work, {} downloading, {} failing, uploads {} staged / {} direct, pages {} by the page thread / {} by the main thread), legacy materials {lm_ready} ready / {lm_wait} pending / {lm_unknown} unknown, glTF materials {gm_ready} ready / {gm_wait} pending / {gm_missing} missing, viewer asset cap {}",
                world.objects.len(),
                t.loaded,
                t.total,
                t.listed,
                t.fetching,
                t.failing,
                t.uploads_staged,
                t.uploads_direct,
                pages.threaded,
                pages.inline,
                if va.is_some() { "ok" } else { "MISSING" }
            );
            // textures in use that never loaded (they show the placeholder)
            for (id, state) in self.textures.stuck(8) {
                log::info!("  texture {id} not loaded: {state}");
            }
        }
        self.parts.lap(Part::Diag, t);
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
        for signal in &anims {
            if self.anims.get(&signal.id).is_none() {
                missing.push(signal.id.to_string());
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

    /// Evaluate skeleton poses of every palette owner near the camera; the
    /// sync of the same frame uploads their joint palettes
    /// (`update_anchors`). Only our avatar reports timed animation stops
    /// to the simulator (LLVOAvatarSelf::requestStopMotion).
    /// Objects our avatar wears (HUDs left out): a step of the teleport and
    /// crossing timelines (AURORA_PROFILE).
    pub fn own_attachment_count(&self) -> usize {
        let Some(own) = self.agent_idx else {
            return 0;
        };
        self.gpu
            .iter()
            .filter(|g| g.owner_avatar == Some(own) && !g.hud && !g.is_avatar)
            .count()
    }

    /// What the avatars wear and play (`avatar summary`, AURORA_PROFILE):
    /// one walk over the objects, every 10 s.
    pub fn avatar_sample(&self) -> AvatarSample {
        let mut s = AvatarSample::default();
        let drawn_in_full = |owner: usize| {
            self.gpu
                .get(owner)
                .is_some_and(|a| a.cull_state & avatar_state::FULL != 0 && a.cull_state & avatar_state::TOO_COMPLEX == 0)
        };
        for g in &self.gpu {
            let Some(owner) = g.owner_avatar.filter(|_| !g.hud && !g.is_avatar) else {
                continue;
            };
            let triangles: u64 = g.faces.iter().map(|f| u64::from(f.cmd.index_count / 3)).sum();
            let full = drawn_in_full(owner);
            s.attachments += 1;
            if full {
                s.attachments_full += 1;
                s.worn_triangles_full += triangles;
            }
            if g.rigged {
                s.rigged += 1;
                s.rigged_triangles += triangles;
                if full {
                    s.rigged_triangles_full += triangles;
                }
            }
        }
        for &c in self.avatar_complexity.values() {
            s.complexity_total += u64::from(c);
            s.complexity_max = s.complexity_max.max(c);
        }
        s.complexity_known = self.avatar_complexity.len();
        for m in self.motions.values() {
            s.animations += m.motion_count();
            s.animations_max = s.animations_max.max(m.motion_count());
        }
        s
    }

    pub fn update_poses(&mut self, world: &mut World, eye: Vec3, now: Instant) -> Vec<Uuid> {
        let mut completed = Vec::new();
        // free palettes of owners that disappeared
        if self.last_palette_gc.elapsed().as_secs_f32() > 2.0 {
            self.last_palette_gc = now;
            let gone: Vec<Uuid> = self
                .palette_slots
                .keys()
                .filter(|id| {
                    world
                        .objects
                        .index_of_uuid(id)
                        .is_none_or(|i| Self::skeleton_owner(world, i).is_none_or(|p| p.0 != **id))
                })
                .copied()
                .collect();
            for id in gone {
                world.avatar_models.remove(&id);
                if let Some(slot) = self.palette_slots.remove(&id) {
                    self.palette_free.push(slot);
                }
            }
        }
        self.stats.posed = 0;
        self.foot_scope.support_frames = 0;
        self.stats.skeletons_rebuilt = 0;
        let n = self.palette_count as usize * anim::PALETTE_JOINTS;
        if n == 0 {
            world.avatar_poses.clear();
            world.avatar_models.clear();
            return completed;
        }
        if self.palettes.len() != n {
            self.palettes.resize(n, Mat4::IDENTITY.to_cols_array_2d());
        }
        let rig = self.avatar_lib.rig.clone();
        // gather work: (slot, anims)
        let mut work = std::mem::take(&mut self.pose_work);
        let mut shape_dz: Vec<(Uuid, f32)> = Vec::new();
        let mut posed: Vec<(Uuid, usize)> = Vec::new();
        self.posed_now.clear();
        // impostors keeping their last pose
        let mut held: Vec<Uuid> = Vec::new();
        let every_frame = impostors::pose_every_frame();
        let mut owners = std::mem::take(&mut self.pose_owners);
        owners.extend(self.palette_slots.iter().map(|(k, v)| (*k, *v)));
        let changed = self.changed_skeletons(world);
        for (owner, slot) in owners.drain(..) {
            let Some(idx) = world.objects.index_of_uuid(&owner) else {
                continue;
            };
            if Self::skeleton_owner(world, idx).is_none_or(|p| p.0 != owner) {
                continue;
            }
            let near = self
                .gpu
                .get(idx)
                .map(|g| g.center.distance(eye) < self.draw_distance + 32.0)
                .unwrap_or(true);
            // Our animation stops drive simulator state even when the camera
            // is looking elsewhere (in particular the pre-jump -> jump).
            if !near && owner != world.agent_id {
                continue;
            }
            // where it looks (LLHUDEffectLookAt::update / calcTargetPosition)
            let target = if owner == world.agent_id {
                Some(world.look_at.own.target)
            } else {
                world.look_at.remote.get(&owner).copied()
            };
            use crate::world::lookat::Gaze;
            let ahead = owner == world.agent_id && world.look_at.own_ahead;
            let look = match crate::world::lookat::gaze(target, ahead) {
                Gaze::Unknown | Gaze::Ahead => Some((false, None)),
                Gaze::Cleared => Some((true, None)),
                Gaze::At(t) => match (self.look_target_pos(world, owner, &t, now), self.head_world(world, &owner, now)) {
                    (Some(tp), Some(head)) => {
                        let rot = Self::object_transform(world, idx, now, 0).map(|x| x.1).unwrap_or(Quat::IDENTITY);
                        let d = rot.inverse() * (tp - head);
                        d.is_finite().then_some((false, Some(d)))
                    }
                    _ => None,
                },
            };
            let control = world.objects.get(idx).is_some_and(|o| !o.is_avatar());
            let point = if owner == world.agent_id {
                Some(world.point_at.own.target)
            } else {
                world.point_at.remote.get(&owner).copied()
            };
            let point = point.filter(|t| t.kind != 0).and_then(|t| {
                let target = self.look_target_pos(world, owner, &t, now)?;
                let (root, rot, _) = Self::object_transform(world, idx, now, 0)?;
                let point = rot.inverse() * (target - root);
                point.is_finite().then_some(point)
            });
            let mut motions = self.motions.remove(&owner).unwrap_or_else(|| {
                if control {
                    anim::Controller::for_control_avatar()
                } else {
                    anim::Controller::default()
                }
            });
            motions.control = control;
            motions.point = point;
            let model =
                Self::object_transform(world, idx, now, 0).map(|(p, q, _)| anchors::skeleton_model(world, &self.meshes, idx, p, q, 1.0));
            if let Some(model) = model {
                world.avatar_models.insert(owner, model);
            }
            motions.poser = self.poser.get(&owner).zip(model).map(|(pose, model)| (pose.clone(), model));
            motions.sitting = !control && world.objects.get(idx).is_some_and(|o| o.parent_id != 0);
            let preview = if owner == world.agent_id && !self.ik_edit.active {
                world.animation_preview_clock
            } else {
                None
            };
            let pose_now = preview.map_or(now, |(_, _, sample)| sample);
            motions.set_preview_clock(preview.map(|(id, start, _)| (Some(id), start)));
            motions.set_synchronized(self.synchronize_animations);
            // The static layer freezes the displayed pose, while normal motion
            // clocks keep running so Stop restores the current animation phase.
            motions.set_paused(
                self.pause_all_skeletons || self.paused_skeletons.contains(&owner) || (owner == world.agent_id && self.ik_edit.active),
                now,
            );
            motions.ik.edit = if owner == world.agent_id {
                self.ik_edit.clone()
            } else {
                ik::EditRequest::default()
            };
            motions.ground = self.grounding_frame(world, idx, now);
            motions.legacy_ground = self.legacy_grounding_frame(world, idx, now);
            motions.aurora_feet = self.ground_avatar_feet;
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
            if motions.control {
                let playing = self.object_signals.entry(owner).or_default().update(world, idx);
                motions.sync(&playing, now, |id| anims.get(id));
            } else {
                let mut playing;
                let signals = if owner == world.agent_id
                    && world.animation_preview_clock.is_none()
                    && let Some(preview) = self.movement_preview
                {
                    playing = world.animations_of(&owner).to_vec();
                    playing.push(preview);
                    playing.as_slice()
                } else {
                    world.animations_of(&owner)
                };
                let stopped = motions.sync(signals, pose_now, |id| anims.get(id));
                if owner == world.agent_id {
                    completed.extend(stopped);
                }
            }
            // an impostor keeps its last pose until its picture is nearly
            // due (impostors.rs); its animations above still start and stop
            // on time
            if !every_frame
                && motions.poser.is_none()
                && !self.portals.planes.contains_key(&idx)
                && owner != world.agent_id
                && world.avatar_poses.contains_key(&owner)
                && let Some(a) = self.far_avatars.get(&idx)
                && let Some(&at) = self.posed_at.get(&owner)
                && self.impostors.pose_can_wait(a, eye, at, now)
            {
                if let Some(s) = self.skeletons.get(&owner) {
                    shape_dz.push((owner, s.shape_dz));
                }
                self.motions.insert(owner, motions);
                held.push(owner);
                continue;
            }
            let cached = changed.as_ref().is_some_and(|c| !c.contains(&owner))
                && self.skeletons.get(&owner).is_some_and(|s| {
                    s.appearance_gen == world.appearance_generation(&owner)
                        && now.duration_since(s.at).as_secs_f32() < skeleton_max_age(&owner)
                });
            let (base, dz) = match self.skeletons.get(&owner).filter(|_| cached) {
                Some(s) => (s.base.clone(), s.shape_dz),
                None => {
                    self.stats.skeletons_rebuilt += 1;
                    self.skeleton_of(world, owner, idx, now)
                }
            };
            shape_dz.push((owner, dz));
            motions.ik.targets = if !control && preview.is_none() {
                ik::targets(world, idx, dz, now)
            } else {
                Vec::new()
            };
            for frame in [&mut motions.ground, &mut motions.legacy_ground].into_iter().flatten() {
                let previous = world.avatar_shape_dz.get(&owner).copied().unwrap_or(0.0);
                frame.to_world.w_axis.z += dz - previous;
                frame.sole_height = self.grounding_sole_height(world, owner);
            }
            let physics = self.physics_job(world, owner, idx, control, eye, now);
            self.foot_scope.support_frames += usize::from(motions.ground.is_some());
            work.push((slot as usize, motions, base, owner, pose_now, physics));
            posed.push((owner, slot as usize));
            self.posed_now.insert(idx);
        }
        self.pose_owners = owners;
        self.stats.posed = work.len();
        let grounded: Vec<usize> = work
            .iter()
            .filter(|w| w.1.ground.is_some())
            .filter_map(|w| world.objects.index_of_uuid(&w.3))
            .collect();
        self.update_foot_support(world, &grounded, now);
        // Keep the scene available for read-only support traces on pose workers.
        let mut pal = std::mem::take(&mut self.palettes);
        let mut chunks: Vec<Option<&mut [[[f32; 4]; 4]]>> = pal.chunks_mut(anim::PALETTE_JOINTS).map(Some).collect();
        let mut jobs: Vec<(&mut [[[f32; 4]; 4]], &mut PoseWork)> = work
            .iter_mut()
            .filter_map(|w| chunks.get_mut(w.0).and_then(|c| c.take()).map(|c| (c, w)))
            .collect();
        let neutral_owner = (self.neutral_pose_test == Some(true)).then_some(world.agent_id);
        let physics_rig = self.physics_rig.clone();
        frame_par::for_each_mut(&mut jobs, POSES_GRAIN, |(chunk, (_, motions, base, owner, pose_now, physics))| {
            // (the anchors after the joints are the sync's)
            for m in chunk.iter_mut().take(anim::ANCHOR_BASE).skip(rig.len()) {
                *m = Mat4::IDENTITY.to_cols_array_2d();
            }
            let frame = motions.ground;
            let native_frame = motions.legacy_ground;
            motions.evaluate_with_support(
                &rig,
                *pose_now,
                Some(base),
                chunk,
                |point| frame.and_then(|frame| self.ground_surface(world, frame, point, now)),
                |point| native_frame.and_then(|frame| self.legacy_ground_surface(world, frame, point)),
            );
            if motions.poser.is_none()
                && !motions.ik.has_external_targets()
                && let (Some(job), Some(physics_rig)) = (physics, physics_rig.as_deref())
            {
                job.run(physics_rig, &rig, now, chunk);
            }
            if neutral_owner == Some(*owner) {
                let neutral = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
                for (dst, matrix) in chunk.iter_mut().zip(neutral) {
                    *dst = matrix.to_cols_array_2d();
                }
            }
        });
        drop(jobs);
        drop(chunks);
        // Resolve cross-avatar references from one shared first-pass snapshot.
        // Each dependent avatar gets at most one additional parallel solve at
        // the same animation clock, without an iterative inter-avatar loop.
        if work.iter().any(|(_, m, _, _, _, _)| m.ik.has_external_targets()) {
            for (owner, slot) in &posed {
                let start = *slot * anim::PALETTE_JOINTS;
                if let Some(src) = pal.get(start..start + rig.len()) {
                    let dst = world.avatar_poses.entry(*owner).or_default();
                    dst.clear();
                    dst.extend(
                        src.iter()
                            .zip(&rig.default_world_inv)
                            .map(|(m, inv)| Mat4::from_cols_array_2d(m) * *inv),
                    );
                }
            }
            for (_, motions, _, _, _, _) in &mut work {
                if motions.ik.has_external_targets() {
                    motions.ik.refresh_external_targets(world, now);
                }
            }
            let mut chunks: Vec<_> = pal.chunks_mut(anim::PALETTE_JOINTS).map(Some).collect();
            let mut jobs: Vec<_> = work
                .iter_mut()
                .filter(|(_, m, _, _, _, _)| m.ik.has_external_targets())
                .filter_map(|w| chunks.get_mut(w.0).and_then(|c| c.take()).map(|c| (c, w)))
                .collect();
            frame_par::for_each_mut(&mut jobs, POSES_GRAIN, |(chunk, (_, motions, base, owner, pose_now, physics))| {
                let frame = motions.ground;
                let native_frame = motions.legacy_ground;
                motions.evaluate_with_support(
                    &rig,
                    *pose_now,
                    Some(base),
                    chunk,
                    |point| frame.and_then(|frame| self.ground_surface(world, frame, point, now)),
                    |point| native_frame.and_then(|frame| self.legacy_ground_surface(world, frame, point)),
                );
                // Advance body physics only once, from the final constrained pose.
                if let (Some(job), Some(physics_rig)) = (physics, physics_rig.as_deref()) {
                    job.run(physics_rig, &rig, now, chunk);
                }
                if neutral_owner == Some(*owner) {
                    let neutral = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
                    for (dst, matrix) in chunk.iter_mut().zip(neutral) {
                        *dst = matrix.to_cols_array_2d();
                    }
                }
            });
        }
        self.palettes = pal;
        for (_, motions, _, owner, _, physics) in work.drain(..) {
            self.motions.insert(owner, motions);
            if let Some(job) = physics {
                self.physics.insert(owner, job.state);
            }
        }
        self.pose_work = work;
        for (owner, _) in &posed {
            self.posed_at.insert(*owner, now);
        }
        let slots = &self.palette_slots;
        self.posed_at.retain(|id, _| slots.contains_key(id));
        self.motions.retain(|id, _| slots.contains_key(id));
        self.physics.retain(|id, _| slots.contains_key(id));
        self.object_signals.retain(|id, _| slots.contains_key(id));
        // joint matrices for the attachments (rest -> posed, skeleton space)
        let joints = rig.len().min(anim::PALETTE_JOINTS);
        world
            .avatar_poses
            .retain(|id, _| posed.iter().any(|p| p.0 == *id) || held.contains(id));
        world.avatar_shape_dz.clear();
        world.avatar_shape_dz.extend(shape_dz);
        self.skeletons
            .retain(|id, _| world.avatar_poses.contains_key(id) || posed.iter().any(|p| p.0 == *id));
        if self.skeleton_member_of.len() > 4 * self.skeletons.values().map(|s| s.members.len()).sum::<usize>() + 1024 {
            let skeletons = &self.skeletons;
            self.skeleton_member_of.retain(|_, owner| skeletons.contains_key(owner));
        }
        // only the skeletons posed this frame are uploaded (the others
        // keep their last pose), by the sync: with the anchors of what they
        // wear, which need their owner's place in this frame
        self.posed_slots.clear();
        self.posed_slots.extend(posed.iter().map(|p| p.1));
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
        completed
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

    /// The point between an avatar's eyes, animated (`anim::Controller::eyes_pos`),
    /// else its head (`head_world`).
    pub fn eyes_world(&self, world: &World, id: &Uuid, now: Instant) -> Option<Vec3> {
        let idx = world.objects.index_of_uuid(id)?;
        let (p, r, _) = Self::object_transform(world, idx, now, 0)?;
        match self.motions.get(id).and_then(|c| c.eyes_pos()) {
            Some(e) => Some(p + r * (e - world.avatar_lib.pelvis)),
            None => self.head_world(world, id, now),
        }
    }

    /// Animated eyes relative to the avatar root, before its world transform.
    /// Camera placement can then apply the hover-camera preference directly.
    pub fn eyes_offset(&self, world: &World, id: &Uuid) -> Option<Vec3> {
        Some(self.motions.get(id)?.eyes_pos()? - world.avatar_lib.pelvis)
    }

    /// Stable name tag above the avatar root, also used for click depth.
    /// Its height includes shape, mesh offsets and translation deformers;
    /// neither body/head rotations nor animation bobbing move the anchor.
    pub fn name_tag_anchor(&self, id: &Uuid, root: Vec3) -> Vec3 {
        let height = self.motions.get(id).and_then(|c| c.name_tag_height).map_or(1.05, |h| h + 0.2);
        root + Vec3::Z * height
    }

    /// Last rendered shape/pose, including mesh joint overrides, for framing.
    pub fn joint_world(&self, world: &World, id: &Uuid, name: &str, now: Instant) -> Option<Vec3> {
        let index = world.objects.index_of_uuid(id)?;
        let (position, rotation, _) = Self::object_transform(world, index, now, 0)?;
        let rig = &world.avatar_lib.rig;
        let joint = *rig.names.get(name)?;
        let rest = rig.default_world_inv[joint].inverse().w_axis.truncate();
        let posed = world
            .avatar_poses
            .get(id)
            .and_then(|pose| pose.get(joint))
            .map(|delta| delta.transform_point3(rest))
            .unwrap_or(rest);
        Some(position + rotation * (posed - world.avatar_lib.pelvis))
    }

    pub fn preview_pose(&self, avatar: &Uuid) -> (Vec<Quat>, Vec<Vec3>) {
        self.motions.get(avatar).map(|motion| motion.preview_pose()).unwrap_or_default()
    }

    /// Where a look-at target is (LLHUDEffectLookAt::calcTargetPosition).
    pub fn look_target_pos(&self, world: &World, source: Uuid, t: &crate::world::lookat::Target, now: Instant) -> Option<Vec3> {
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

    /// Rest skeleton of an avatar (or animesh) and the vertical offset of
    /// its pelvis from the object position:
    /// - the shape (visual parameters of its last appearance),
    /// - joint positions imposed by the rigged meshes it wears
    ///   (LLVOAvatar::addAttachmentOverridesForObject: the translation of each
    ///   alternate inverse bind matrix is the joint's local position; with
    ///   lock_scale_if_joint_position the joint keeps its default scale),
    /// - LLVOAvatar::updateRootPositionAndRotation: standing/ground-sit roots
    ///   use body size and shape hover. Adjustable hover is applied live by
    ///   object_transform_in, outside this cache (also while seated).
    ///
    /// Joint overrides never cross a control-avatar boundary. Rebuild when
    /// topology, skin metadata or appearance changes, including late assets.
    fn skeleton_of(&mut self, world: &World, owner: Uuid, owner_idx: usize, now: Instant) -> (Arc<anim::SkeletonBase>, f32) {
        use std::hash::{Hash, Hasher};
        let members = animesh::members(&world.objects, owner_idx);
        let mut signature = std::collections::hash_map::DefaultHasher::new();
        let seated = world.objects.get(owner_idx).is_some_and(|o| o.is_avatar() && o.parent_id != 0);
        for &i in &members {
            if let Some(o) = world.objects.get(i) {
                (o.full_id, o.pcode, o.parent_id, o.extra.extended_mesh_flags).hash(&mut signature);
                if let Some(s) = o.visual_volume().sculpt {
                    s.texture.hash(&mut signature);
                    if let Some(skin) = self.meshes.meta(&s.texture).and_then(|m| m.skin) {
                        (Arc::as_ptr(&skin) as usize).hash(&mut signature);
                    }
                }
            }
        }
        let signature = signature.finish();
        let generation = world.appearance_generation(&owner);
        self.note_skeleton_members(owner, &members);
        if let Some(s) = self.skeletons.get_mut(&owner)
            && s.appearance_gen == generation
            && s.signature == signature
            && now.duration_since(s.at).as_secs_f32() < skeleton_max_age(&owner)
        {
            if s.members != members {
                s.members = members;
            }
            return (s.base.clone(), s.shape_dz);
        }
        let rig = self.avatar_lib.rig.clone();
        let mut overrides = joint_overrides::JointOverrides::new(&rig);
        for &i in &members {
            let Some(o) = world.objects.get(i) else {
                continue;
            };
            if !o.visual_volume().is_mesh() {
                continue;
            }
            if let Some(mesh) = o.visual_volume().sculpt
                && let Some(skin) = self.meshes.meta(&mesh.texture).and_then(|m| m.skin)
            {
                overrides.add(&rig, mesh.texture, &skin);
            }
        }
        let avatar = world.objects.get(owner_idx).is_some_and(|o| o.is_avatar());
        let mut physics = None;
        let (mut local_pos, mut scale, hover) = if avatar {
            let values = world.visual_params(&owner).unwrap_or(&[]);
            let s = self.avatar_lib.shape_params.shape(values, &rig);
            if let Some(physics_rig) = &self.physics_rig {
                let weights = self.avatar_lib.shape_params.effective_weights(values);
                physics = body_physics::Params::from_weights(physics_rig, &weights, s.male).map(Arc::new);
            }
            (s.local_pos, s.scale, s.hover)
        } else {
            (rig.local_pos.clone(), rig.scale.clone(), 0.0)
        };
        // Server baking derives the standing agent size from the wearables,
        // before local mesh joint overrides; it never sees those mesh assets.
        let neutral_body_height = if avatar {
            shape::body_size(&rig, &local_pos, &scale).1
        } else {
            0.0
        };
        overrides.apply(&rig, &mut local_pos, &mut scale);
        let (shape_dz, neutral_root_height) = if avatar {
            let (pelvis_to_foot, height) = shape::body_size(&rig, &local_pos, &scale);
            let standing = hover + overrides.pelvis_fixup() - (0.5 * height - pelvis_to_foot);
            // Neutral geometry and the physical centre share one coordinate
            // frame, including arbitrary mesh pelvis translations. Seating
            // changes only the live rendered root, never this standing model.
            (if seated { 0.0 } else { standing }, local_pos[rig.pelvis].z - standing)
        } else {
            (overrides.pelvis_fixup(), 0.0)
        };
        let base = Arc::new(anim::SkeletonBase { local_pos, scale });
        self.skeletons.insert(
            owner,
            AvatarSkeleton {
                at: now,
                appearance_gen: generation,
                signature,
                base: base.clone(),
                shape_dz,
                neutral_root_height,
                neutral_body_height,
                neutral_shape_hover: hover,
                members,
                physics,
            },
        );
        (base, shape_dz)
    }

    /// The weights of the body physics' driven parameters of an avatar at
    /// its last pose (the system body's live morphs, live_morphs.rs).
    fn physics_driven(&self, id: &Uuid, out: &mut Vec<(i32, f32)>) {
        out.clear();
        if let (Some(state), Some(params), Some(rig)) = (
            self.physics.get(id),
            self.skeletons.get(id).and_then(|s| s.physics.as_deref()),
            self.physics_rig.as_deref(),
        ) {
            state.driven_weights(rig, params, out);
        }
    }

    /// Collision volume offsets of an avatar's body physics at its last pose
    /// (demo log).
    pub fn body_physics_offsets(&self, id: &Uuid) -> Vec<(usize, Vec3)> {
        let mut out = Vec::new();
        if let (Some(state), Some(params), Some(rig)) = (
            self.physics.get(id),
            self.skeletons.get(id).and_then(|s| s.physics.as_deref()),
            self.physics_rig.as_deref(),
        ) {
            state.offsets(rig, params, &mut out);
        }
        out
    }

    /// The body physics of an avatar this frame (body_physics.rs): None, and
    /// its motions forgotten, when it wears none or the physics are off. An
    /// avatar too small on screen for the detail setting keeps its last
    /// shape.
    fn physics_job(&mut self, world: &World, owner: Uuid, idx: usize, control: bool, eye: Vec3, now: Instant) -> Option<PhysicsJob> {
        let params = self
            .skeletons
            .get(&owner)
            .and_then(|s| s.physics.clone())
            .filter(|_| !control && self.body_physics.on() && self.physics_rig.is_some());
        let root = params.as_ref().and_then(|_| Self::object_transform(world, idx, now, 0));
        let (Some(params), Some((position, rotation, _))) = (params, root) else {
            self.physics.remove(&owner);
            return None;
        };
        let update = self.body_physics.updates(owner == world.agent_id, position.distance(eye));
        let mut state = self.physics.remove(&owner).unwrap_or_else(|| body_physics::BodyPhysics::new(now));
        if !update {
            state.hold();
        }
        Some(PhysicsJob {
            state,
            params,
            root: (position, rotation),
            update,
        })
    }

    /// Tools.ResyncAnimations: every resident and control avatar restarts on
    /// our screen only, with one shared instant for the whole scene.
    pub fn resync_animations(&mut self, now: Instant) {
        use rayon::prelude::*;
        self.motions.par_iter_mut().for_each(|(_, motions)| motions.resync_animations(now));
    }

    /// "Reset skeleton" of an avatar or an animated object
    /// (LLVOAvatar::resetSkeleton): the rest skeleton is built again from
    /// the shape and the worn meshes on the next frame, and the pose held
    /// from finished motions goes; `animations` also restarts the motions
    /// (`anim::Controller::reset_pose`). Local only: what the others see of
    /// our avatar goes through the ViewerEffect (app/reset_skeleton.rs).
    pub fn reset_skeleton(&mut self, owner: Uuid, animations: bool) {
        self.skeletons.remove(&owner);
        if let Some(motions) = self.motions.get_mut(&owner) {
            motions.reset_pose(animations);
        }
    }

    /// Note which objects a skeleton is made of (its previous members that
    /// left are forgotten).
    fn note_skeleton_members(&mut self, owner: Uuid, members: &[usize]) {
        if let Some(s) = self.skeletons.get(&owner) {
            if s.members == members {
                return;
            }
            for i in &s.members {
                if self.skeleton_member_of.get(i) == Some(&owner) {
                    self.skeleton_member_of.remove(i);
                }
            }
        }
        for &i in members {
            self.skeleton_member_of.insert(i, owner);
        }
    }

    /// Skeleton owners whose rest skeleton may have changed since the last
    /// frame (None: any of them): the owners, old and new, of the objects
    /// changed or removed since (their shape, links, meshes), or all of them
    /// when mesh metadata arrived. The others keep their cached skeleton
    /// without hashing their members again (still rebuilt every 2 s and on
    /// a new appearance). Poses run before the sync takes these lists.
    fn changed_skeletons(&mut self, world: &World) -> Option<std::collections::HashSet<Uuid>> {
        if self.meshes.meta_generation != self.skeleton_meta_gen {
            self.skeleton_meta_gen = self.meshes.meta_generation;
            return None;
        }
        let mut changed = std::collections::HashSet::new();
        for &idx in world.objects.touched().iter().chain(&world.objects.removed_slots) {
            if let Some(&owner) = self.skeleton_member_of.get(&idx) {
                changed.insert(owner);
            }
            if let Some((owner, _)) = Self::skeleton_owner(world, idx) {
                changed.insert(owner);
            }
        }
        Some(changed)
    }
}

/// One skeleton's pose of the frame: palette slot, motions, rest skeleton,
/// owner, pose clock and body physics.
type PoseWork = (usize, anim::Controller, Arc<anim::SkeletonBase>, Uuid, Instant, Option<PhysicsJob>);

/// One avatar's body physics, run with its pose.
struct PhysicsJob {
    state: body_physics::BodyPhysics,
    params: Arc<body_physics::Params>,
    /// The avatar's place in the world (its skeleton's origin).
    root: (Vec3, Quat),
    update: bool,
}

impl PhysicsJob {
    /// Advance the motions with the joints just posed, then move the
    /// collision volumes of the pose.
    fn run(&mut self, physics_rig: &body_physics::PhysicsRig, rig: &anim::Rig, now: Instant, palette: &mut [[[f32; 4]; 4]]) {
        if self.update {
            let [chest, pelvis] = physics_rig.joints();
            if let (Some(chest), Some(pelvis)) = (
                body_physics::joint_world(palette, chest, self.root),
                body_physics::joint_world(palette, pelvis, self.root),
            ) {
                self.state.update(physics_rig, &self.params, now, [chest, pelvis]);
            }
        }
        let mut offsets = Vec::with_capacity(4);
        self.state.offsets(physics_rig, &self.params, &mut offsets);
        body_physics::apply(rig, &offsets, palette);
    }
}

/// Cached rest skeleton of one skeleton owner.
struct AvatarSkeleton {
    signature: u64,
    at: Instant,
    appearance_gen: u64,
    base: Arc<anim::SkeletonBase>,
    shape_dz: f32,
    /// Neutral pelvis minus the standing rendered root offset.
    neutral_root_height: f32,
    /// Shape-only body height, before mesh overrides, as used by server baking.
    neutral_body_height: f32,
    neutral_shape_hover: f32,
    /// Objects whose shape or meshes went into it (see `skeleton_member_of`).
    members: Vec<usize>,
    /// Body physics its appearance asks for (body_physics.rs).
    physics: Option<Arc<body_physics::Params>>,
}

/// A face `rebuild_faces` omits (LL's PASS_ALPHA_INVISIBLE): a zero
/// texture-entry alpha. A glTF material ignores the texture entry's color
/// (LLVolumeGeometryManager::genDrawInfo): its faces are always built and
/// `FaceDraw::fully_transparent` decides.
fn te_invisible(o: &Object, face: usize) -> bool {
    o.visual_texture_face(face).color[3] < 0.004 && o.visual_material(face).is_none()
}

fn classify(f: &FaceDraw, tex_alpha: AlphaKind) -> Pass {
    // fully transparent face, or the transparent texture (unused layers of
    // mesh heads and bodies): nothing to draw
    if f.fully_transparent() || (f.base_slot == aurora_render::textures::TRANSPARENT && f.pbr_alpha.is_none()) {
        return Pass::Hidden;
    }
    // A system body part: the avatar pool decides, not the texture alone.
    // The body is never blended, whatever the alpha of its bake; hair,
    // eyelashes and skirt are blended where their bake is, and alpha-tested
    // like the body.
    match f.system {
        Some(SystemPart::Body) => return if tex_alpha == AlphaKind::Opaque { Pass::Opaque } else { Pass::Mask },
        Some(SystemPart::Transparent) => {
            return match tex_alpha {
                AlphaKind::Opaque => Pass::Opaque,
                AlphaKind::Mask => Pass::Mask,
                AlphaKind::Blend => Pass::Blend,
            };
        }
        None => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;

    #[test]
    fn bakes_replace_placeholders_only_on_worn_non_hud_non_animesh_objects() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for e in crate::demo::events().into_iter().chain(crate::demo::animesh_events()) {
            w.apply(e);
        }
        let index = |local: u32| {
            w.objects
                .iter()
                .find(|(_, o)| o.key.local_id == local)
                .map(|(i, _)| i)
                .expect("fixture object")
        };
        let avatar = index(9001);
        let wearer = Some((w.objects.get(avatar).expect("avatar").full_id, avatar));
        // a rigged mesh worn by the avatar takes its bakes, not on a HUD
        assert_eq!(Scene::bake_wearer(&w, index(9100), false), wearer);
        assert_eq!(Scene::bake_wearer(&w, index(9100), true), None);
        // an animesh, worn (9203) or on the ground (9200, 9202), has none
        for animesh in [9203, 9200, 9202] {
            assert_eq!(Scene::bake_wearer(&w, index(animesh), false), None, "{animesh}");
        }
        // nor has an ordinary object on the ground
        let ground = w
            .objects
            .iter()
            .find(|(_, o)| o.parent_id == 0 && !o.is_avatar() && o.extra.extended_mesh_flags.is_none())
            .map(|(i, _)| i)
            .expect("ground object");
        assert_eq!(Scene::bake_wearer(&w, ground, false), None);
    }

    #[test]
    fn first_person_visibility_uses_attachment_roots_and_preserves_huds_and_other_avatars() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        let events = crate::demo::first_person_visibility_events();
        let handle = match &events[0] {
            NetEvent::ObjectUpdates { handle, .. } => *handle,
            _ => unreachable!(),
        };
        world.apply(NetEvent::MainRegionChanged { handle });
        world.apply(NetEvent::ObjectUpdates {
            handle,
            objects: vec![crate::demo::action_avatar(false)],
        });
        for event in events {
            world.apply(event);
        }
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let mut scene = Scene::new(Default::default(), lib);
        scene.agent_idx = Some(own);
        let body = ObjGpu {
            is_avatar: true,
            ..Default::default()
        };
        let other = own + world.objects.len() + 1;
        for mode in [None, Some(false), Some(true), None] {
            scene.set_first_person(mode);
            assert_eq!(scene.first_person_object_visible(own, &body), mode != Some(false));
            assert!(scene.first_person_object_visible(other, &body));
            for (id, hidden) in [(29_001, true), (29_002, true), (29_003, false), (29_004, true)] {
                let idx = world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap();
                assert_eq!(Scene::attachment_first_person_hidden(&world, idx), hidden);
                let attachment = ObjGpu {
                    owner_avatar: Some(own),
                    first_person_hidden: hidden,
                    rigged: true,
                    ..Default::default()
                };
                assert_eq!(
                    scene.first_person_object_visible(idx, &attachment),
                    mode.is_none() || mode == Some(true) && !hidden
                );
                let rigid = ObjGpu {
                    rigged: false,
                    ..attachment
                };
                assert_eq!(
                    scene.first_person_object_visible(idx, &rigid),
                    mode.is_none() || mode == Some(true) && !hidden
                );
                let other_attachment = ObjGpu {
                    owner_avatar: Some(other),
                    ..rigid
                };
                assert!(scene.first_person_object_visible(idx, &other_attachment));
                let hud = ObjGpu {
                    hud: true,
                    owner_avatar: Some(own),
                    first_person_hidden: hidden,
                    ..Default::default()
                };
                assert!(scene.first_person_object_visible(idx, &hud));
            }
        }
    }

    #[test]
    fn a_worn_prim_changing_has_its_wearers_bake_slots_looked_at_again() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        for ev in crate::demo::events()
            .into_iter()
            .chain(crate::demo::cloud_self_events(&lib.shape_params))
        {
            world.apply(ev);
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        let own = world.objects.index_of_uuid(&world.agent_id).expect("demo agent");
        let key = world.objects.get(own).expect("avatar").key;
        let worn = world.objects.children_of(&key)[0];
        // the avatar itself moving: nothing it wears changed
        scene.note_worn_change(&world, own);
        assert!(scene.bom_stale.is_empty());
        scene.note_worn_change(&world, worn);
        assert!(scene.bom_stale.contains(&own));
        // a prim on the ground has no wearer
        scene.bom_stale.clear();
        let ground = (0..world.objects.slots.len())
            .find(|&i| world.objects.get(i).is_some_and(|o| !o.is_avatar() && o.parent_id == 0))
            .expect("a prim on the ground");
        scene.note_worn_change(&world, ground);
        assert!(scene.bom_stale.is_empty());
    }

    #[test]
    fn zero_subtype_sculpt_selects_map_geometry_for_every_lod() {
        use aurora_prim::params::*;
        let mut update = crate::demo::action_avatar(false);
        update.pcode = LL_PCODE_VOLUME;
        update.volume.path.curve_type = LL_PCODE_PATH_CIRCLE;
        update.volume.profile.curve_type = LL_PCODE_PROFILE_CIRCLE;
        let map = Uuid::from_u128(587);
        for flags in [
            0,
            LL_SCULPT_FLAG_INVERT,
            LL_SCULPT_FLAG_MIRROR,
            LL_SCULPT_FLAG_INVERT | LL_SCULPT_FLAG_MIRROR,
        ] {
            update.volume.sculpt = Some(SculptParams {
                texture: map,
                sculpt_type: flags,
            });
            update.extra.sculpt = update.volume.sculpt;
            let mut object = Object::from_update(1, update.clone());
            for lod in 0..4 {
                assert!(matches!(Scene::object_geom_key(&object, lod), Some(GeomKey::Sculpt { lod: level, .. }) if level == lod));
            }
            object.volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
            object.extra.flexible = Some(aurora_prim::extra::FlexibleParams {
                softness: 1,
                tension: 1.0,
                air_friction: 1.0,
                gravity: 0.0,
                wind_sensitivity: 0.0,
                user_force: Vec3::ZERO,
            });
            assert!(!object.is_flexible(), "stale flexible fields cannot override an active sculpt");
            assert!(matches!(Scene::object_geom_key(&object, 3), Some(GeomKey::Sculpt { .. })));
        }
        update.volume.sculpt = None;
        assert!(matches!(
            Scene::object_geom_key(&Object::from_update(1, update.clone()), 3),
            Some(GeomKey::Prim { .. })
        ));
        update.volume.sculpt = Some(SculptParams {
            texture: map,
            sculpt_type: LL_SCULPT_TYPE_MESH,
        });
        assert_eq!(
            Scene::object_geom_key(&Object::from_update(1, update), 3),
            Some(GeomKey::Mesh { id: map, lod: 3 })
        );
    }

    #[test]
    fn local_neutral_test_matches_auto_hover_rest_and_preserves_live_animations() {
        let lib = Arc::new(AvatarLibrary::load());
        let rig = lib.rig.clone();
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        world.apply(NetEvent::MainRegionChanged { handle: 1 });
        let mut other = crate::demo::action_avatar(false);
        other.local_id = 9001;
        other.full_id = crate::demo::DEMO_LOUP;
        world.apply(NetEvent::ObjectUpdates {
            handle: 1,
            objects: vec![crate::demo::action_avatar(false), other],
        });
        let animation_id = Uuid::from_u128(0xaaaa1234);
        let mut animation = crate::demo::deform::animation(&rig);
        animation.joints = vec![aurora_assets::anim::JointMotion {
            joint_name: "mPelvis".into(),
            priority: 4,
            rot_keys: Vec::new(),
            pos_keys: vec![aurora_assets::anim::PosKey {
                time: 0.0,
                position: Vec3::Z * 0.1,
            }],
        }];
        animation.looping = true;
        animation.ease_in = 0.0;
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        assert_ne!(
            scene.neutral_pose_test,
            Some(true),
            "test launches must start with normal animations"
        );
        scene.anims.insert(animation_id, Arc::new(anim::BoundAnim::bind(animation, &rig)));
        for avatar in [world.agent_id, crate::demo::DEMO_LOUP] {
            scene.palette_base(avatar);
            world.apply(NetEvent::AvatarAnimations {
                sources: Vec::new(),
                avatar,
                anims: vec![(animation_id, 1)],
            });
        }
        let now = Instant::now();
        scene.update_poses(&mut world, Vec3::ZERO, now);
        let own_before = world.avatar_poses[&world.agent_id].clone();
        let other_before = world.avatar_poses[&crate::demo::DEMO_LOUP].clone();
        let signals_before = world.animations_of(&world.agent_id).to_vec();
        scene.neutral_pose_test = Some(true);
        assert!(
            scene
                .update_poses(&mut world, Vec3::ZERO, now + std::time::Duration::from_millis(300))
                .is_empty()
        );
        let base = &scene.skeletons[&world.agent_id].base;
        let neutral = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
        let foot = rig.names["mFootLeft"];
        let rest_foot = rig.default_world_inv[foot].inverse().w_axis.truncate();
        let neutral_foot = world.avatar_poses[&world.agent_id][foot].transform_point3(rest_foot);
        let animated_foot = own_before[foot].transform_point3(rest_foot);
        assert!(
            (animated_foot.z - neutral_foot.z - 0.1).abs() < 1e-5,
            "10 cm posed offset must disappear in the local neutral view"
        );
        for (actual, expected) in world.avatar_poses[&world.agent_id]
            .iter()
            .zip(neutral.iter().zip(&rig.default_world_inv))
        {
            assert!(
                actual.abs_diff_eq(*expected.0 * *expected.1, 1e-6),
                "exact shaped rest skeleton used by auto hover"
            );
        }
        assert!(
            world.avatar_poses[&crate::demo::DEMO_LOUP][foot].abs_diff_eq(other_before[foot], 1e-6),
            "other avatars retain their animated feet"
        );
        assert_eq!(world.animations_of(&world.agent_id), signals_before);
        scene.neutral_pose_test = Some(false);
        scene.update_poses(&mut world, Vec3::ZERO, now + std::time::Duration::from_millis(600));
        assert!(
            world.avatar_poses[&world.agent_id][foot].abs_diff_eq(own_before[foot], 1e-6),
            "restore without resetting or stopping motions"
        );
    }

    #[test]
    fn seated_mouselook_uses_the_current_pose_on_a_reclined_seat() {
        use crate::camera::{Camera, CameraSettings, FrameInput};

        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        for frame in [650, 700] {
            for event in crate::camera::demo::sit_events("sit-ml-reclined", frame) {
                if let NetEvent::ObjectUpdates { handle, .. } = &event {
                    world.main_region = Some(*handle);
                }
                world.apply(event);
            }
        }
        let owner = world.agent_id;
        let mut scene = Scene::new(Default::default(), lib.clone());
        scene.palette_base(owner);
        let mut camera = Camera::default();
        let mut settings = CameraSettings {
            hover_affects_camera: true,
            ..Default::default()
        };
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        let now = Instant::now();
        let mut previous = None;
        // Two actual poses bend the torso and move the eyes. Evaluate the
        // skeleton before the camera, as App's deferred seated update does.
        for (i, bend) in [0.0, 0.8].into_iter().enumerate() {
            let mut animation = crate::demo::seat_animation();
            animation.ease_in = 0.0;
            animation.joints.push(aurora_assets::anim::JointMotion {
                joint_name: "mTorso".into(),
                priority: 5,
                rot_keys: vec![aurora_assets::anim::RotKey {
                    time: 0.0,
                    rotation: Quat::from_rotation_y(bend),
                }],
                pos_keys: Vec::new(),
            });
            let animation_id = Uuid::from_u128(0x101500 + i as u128);
            scene
                .anims
                .insert(animation_id, Arc::new(anim::BoundAnim::bind(animation, &lib.rig)));
            world.apply(NetEvent::AvatarAnimations {
                avatar: owner,
                anims: vec![(animation_id, 1)],
                sources: Vec::new(),
            });
            let now = now + std::time::Duration::from_secs(i as u64);
            scene.update_poses(&mut world, Vec3::ZERO, now);
            let eyes = scene.eyes_world(&world, &owner, now).expect("posed eyes");
            let input = FrameInput {
                dt: 1.0,
                now,
                steering: false,
                build_mode: false,
                edit_tracking: None,
                eye_offset: scene.eyes_offset(&world, &owner),
                draw_distance: 128.0,
                aspect: 1.0,
            };
            camera.update(&mut world, &settings, &input);
            let index = world.objects.index_of_uuid(&owner).unwrap();
            let rotation = Scene::object_transform(&world, index, now, 0).unwrap().1;
            assert!(camera.position.abs_diff_eq(eyes + rotation * Vec3::Z * 0.1, 2e-5));
            let aim = camera.forward();
            settings.mouselook_follow_eyes = true;
            camera.update(&mut world, &settings, &input);
            assert!(camera.position.abs_diff_eq(eyes, 2e-5), "follow the actual posed midpoint");
            assert!(camera.forward().abs_diff_eq(aim, 2e-5), "head motion must not rotate the camera");
            settings.mouselook_follow_eyes = false;
            if let Some(previous) = previous {
                assert!(eyes.distance(previous) > 0.1, "the animation must move the eyes");
            }
            previous = Some(eyes);
        }
    }

    #[test]
    fn ground_mouselook_tracks_the_lowered_pelvis_pose() {
        use crate::camera::{Camera, CameraSettings, FrameInput};

        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        for event in crate::demo::sit_events(700) {
            if let NetEvent::ObjectUpdates { handle, .. } = &event {
                world.main_region = Some(*handle);
            }
            world.apply(event);
        }
        let owner = world.agent_id;
        let id = crate::world::body::ANIM_SIT_GROUND_CONSTRAINED;
        let mut scene = Scene::new(Default::default(), lib.clone());
        scene.palette_base(owner);
        scene.anims.insert(
            id,
            Arc::new(anim::BoundAnim::bind(crate::demo::first_person_ground_animation(), &lib.rig)),
        );
        world.apply(NetEvent::AvatarAnimations {
            avatar: owner,
            anims: vec![(id, 1)],
            sources: Vec::new(),
        });
        assert!(world.agent.ground_sit && !world.agent.seated);
        let now = Instant::now();
        scene.update_poses(&mut world, Vec3::ZERO, now);
        let eyes = scene.eyes_world(&world, &owner, now).unwrap();
        let mut settings = CameraSettings {
            hover_affects_camera: true,
            ..Default::default()
        };
        let mut camera = Camera::default();
        camera.change_to_mouselook(&mut world.agent, &settings, false);
        let input = FrameInput {
            dt: 1.0,
            now,
            steering: false,
            build_mode: false,
            edit_tracking: None,
            eye_offset: scene.eyes_offset(&world, &owner),
            draw_distance: 128.0,
            aspect: 1.0,
        };
        camera.update(&mut world, &settings, &input);
        assert!((camera.position.z - eyes.z - 0.1).abs() < 1e-5);
        let standing_eye = world.agent.position.z + 0.7;
        assert!(
            camera.position.z < standing_eye - 0.3,
            "the ground pose must actually lower the camera"
        );
        let aim = camera.forward();
        settings.mouselook_follow_eyes = true;
        camera.update(&mut world, &settings, &input);
        assert!(camera.position.abs_diff_eq(eyes, 2e-5));
        assert!(camera.forward().abs_diff_eq(aim, 2e-5));
    }

    #[test]
    fn seat_root_skips_body_height_and_cache_changes_immediately_on_standing() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for frame in [250, 300] {
            for event in crate::demo::sit_events(frame) {
                world.apply(event);
            }
        }
        let owner = world.agent_id;
        let idx = world.objects.index_of_uuid(&owner).expect("demo avatar");
        let mut scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let now = Instant::now();
        let (_, seated_dz) = scene.skeleton_of(&world, owner, idx, now);
        assert!(seated_dz.abs() < 1e-5, "seat root must match simulator offset: {seated_dz}");
        let generation = world.appearance_generation(&owner);
        for event in crate::demo::sit_events(700) {
            world.apply(event);
        }
        assert_eq!(world.appearance_generation(&owner), generation);
        let (_, standing_dz) = scene.skeleton_of(&world, owner, idx, now);
        let rig = &world.avatar_lib.rig;
        let shape = world.avatar_lib.shape_params.shape(&[], rig);
        let (foot, height) = shape::body_size(rig, &shape.local_pos, &shape.scale);
        let expected = shape.hover - (0.5 * height - foot);
        assert!((standing_dz - expected).abs() < 1e-5);
        assert!((standing_dz - seated_dz).abs() > 0.01, "cache must not keep the seated offset");
        // A ground sit retains the standing/body-height calculation.
        world.agent.ground_sit = true;
        let (_, ground_dz) = scene.skeleton_of(&world, owner, idx, now);
        assert_eq!(ground_dz, standing_dz);
        for event in crate::demo::sit_events(300) {
            world.apply(event);
        }
        let (_, seated_again_dz) = scene.skeleton_of(&world, owner, idx, now);
        assert_eq!(seated_again_dz, seated_dz);
    }

    #[test]
    fn seated_hover_follows_the_avatar_rotation_on_a_tilted_linked_seat() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for frame in [250, 300] {
            for event in crate::demo::sit_events(frame) {
                if let NetEvent::ObjectUpdates { handle, .. } = &event {
                    world.main_region = Some(*handle);
                }
                world.apply(event);
            }
        }
        let owner = world.agent_id;
        world.apply(NetEvent::Capabilities {
            handle: world.main_region.unwrap(),
            caps: Arc::new(HashMap::new()),
        });
        let region = world.regions.get_mut(&world.main_region.unwrap()).unwrap();
        region.avatar_hover_height = true;
        Arc::make_mut(&mut region.caps).insert("AgentPreferences".into(), String::new());
        world.hover_height.preview(0.25);
        world.hover_height.commit();
        world.apply(NetEvent::Appearance(aurora_net::AvatarAppearance {
            avatar_id: owner,
            texture_entry: None,
            visual_params: world.avatar_lib.shape_params.values_with(&[]),
            cof_version: 1,
            appearance_version: 1,
            // Our preference must survive a stale simulator appearance.
            hover_height: -0.5,
            attachments: 0,
        }));
        let idx = world.objects.index_of_uuid(&owner).expect("demo avatar");
        let seat_idx = world.objects.parent_of(world.objects.get(idx).expect("avatar")).expect("seat");
        let seat = world.objects.get_mut(seat_idx).expect("seat");
        seat.rotation = Quat::from_rotation_y(0.7) * Quat::from_rotation_z(1.2);
        let mut scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let now = Instant::now();
        let (_, dz) = scene.skeleton_of(&world, owner, idx, now);
        assert!(dz.abs() < 1e-5);
        world.avatar_shape_dz.insert(owner, dz);
        let (raw, rotation, _) = Scene::object_transform_raw(&world, idx, now, 0, None).expect("raw transform");
        let (posed, _, _) = Scene::object_transform(&world, idx, now, 0).expect("posed transform");
        assert!(posed.abs_diff_eq(raw + rotation * (Vec3::Z * 0.25), 1e-5));
        assert!(!posed.abs_diff_eq(raw + Vec3::Z * 0.25, 1e-3));
        let (base, _) = scene.skeleton_of(&world, owner, idx, now);
        world.hover_height.preview(0.75);
        // The new hover is visible even before another pose update.
        let (adjusted_root, _, _) = Scene::object_transform(&world, idx, now, 0).expect("adjusted transform");
        assert!(adjusted_root.abs_diff_eq(raw + rotation * (Vec3::Z * 0.75), 1e-5));
        let (same_base, adjusted) = scene.skeleton_of(&world, owner, idx, now);
        assert!(Arc::ptr_eq(&base, &same_base), "hover must not rebuild the skeleton");
        assert_eq!(adjusted, dz);
        world.regions.get_mut(&world.main_region.unwrap()).unwrap().avatar_hover_height = false;
        let (disabled, _, _) = Scene::object_transform(&world, idx, now, 0).expect("unsupported transform");
        assert!(disabled.abs_diff_eq(raw, 1e-5));
    }

    #[test]
    fn seated_agent_position_goes_through_the_seat() {
        fn apply(world: &mut World, frame: u64) {
            for event in crate::demo::sit_events(frame) {
                if let NetEvent::ObjectUpdates { handle, .. } = &event {
                    world.main_region = Some(*handle);
                }
                world.apply(event);
            }
        }
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        let now = Instant::now();
        apply(&mut world, 250);
        assert_eq!(Scene::seated_agent_position(&world, now), None);
        apply(&mut world, 300);
        let idx = world.objects.index_of_uuid(&world.agent_id).expect("demo avatar");
        let seat_idx = world.objects.parent_of(world.objects.get(idx).expect("avatar")).expect("seat");
        // the seat moved and tipped over like a vehicle: the avatar, 0.4 m
        // above its centre, goes with it
        let seat = world.objects.get_mut(seat_idx).expect("seat");
        seat.position = Vec3::new(40.0, 200.0, 60.0);
        seat.rotation = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let carried = Scene::seated_agent_position(&world, now).expect("seated");
        assert!(carried.abs_diff_eq(Vec3::new(40.0, 199.6, 60.0), 1e-4), "{carried:?}");
        apply(&mut world, 700);
        assert_eq!(Scene::seated_agent_position(&world, now), None);
    }

    /// The demo region with a prim linked under Violet Wolf's hat, the
    /// avatar posed off its rest pose.
    fn posed_attachment() -> (World, [usize; 3]) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for ev in crate::demo::events() {
            world.apply(ev);
        }
        let avatar_id = Uuid::from_u128(0xA0E0_A6E1_0000_0000_0000_0000_0000_0002);
        let avatar = world.objects.index_of_uuid(&avatar_id).expect("Loup Violet");
        let hat = world
            .objects
            .index_of_uuid(&Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | 9100))
            .expect("hat");
        let mut u = crate::demo::action_avatar(false);
        let region = world.objects.get(hat).expect("hat").key.region;
        u.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        u.local_id = 9150;
        u.full_id = Uuid::from_u128(9150);
        u.parent_id = 9100;
        u.position = Vec3::new(0.1, -0.05, 0.2);
        u.rotation = Quat::from_rotation_x(0.4);
        let child = world.objects.upsert(region, u);
        let pose = Mat4::from_rotation_translation(Quat::from_rotation_z(0.3), Vec3::new(0.02, 0.0, 0.05));
        world.avatar_poses.insert(avatar_id, vec![pose; lib.rig.len()]);
        (world, [avatar, hat, child])
    }

    #[test]
    fn surface_float_moves_the_drawn_root_and_linked_attachments_without_moving_simulator_positions() {
        let (mut world, ids) = posed_attachment();
        let now = Instant::now();
        let root = Scene::object_transform_unshifted(&world, ids[0], now).expect("avatar").0;
        world.agent_id = world.objects.get(ids[0]).expect("avatar").full_id;
        world.agent.position = root;
        world.movement_complete = true;
        let physical = (
            world.agent.position,
            world.agent.server_pos,
            world.objects.get(ids[0]).expect("avatar").position,
        );
        let before: Vec<_> = ids
            .iter()
            .map(|&id| Scene::object_transform(&world, id, now, 0).expect("transform"))
            .collect();
        for frame in 0..60 {
            world.water_float.update(1, root, frame as f32 / 60.0, 0.25, false);
        }
        let dz = world.water_float.offset();
        assert!(dz > 0.249);
        for (id, (position, rotation, hud)) in ids.into_iter().zip(before) {
            let after = Scene::object_transform(&world, id, now, 0).expect("shifted transform");
            assert!(after.0.abs_diff_eq(position + Vec3::Z * dz, 1e-4));
            assert_eq!((after.1, after.2), (rotation, hud));
        }
        assert!(
            Scene::object_transform_unshifted(&world, ids[0], now)
                .expect("physical root")
                .0
                .abs_diff_eq(root, 1e-4)
        );
        assert_eq!(
            (
                world.agent.position,
                world.agent.server_pos,
                world.objects.get(ids[0]).expect("avatar").position
            ),
            physical
        );
        assert_eq!(world.water_visual_dz(&Uuid::nil()), 0.0);
        world.agent.seated = true;
        assert_eq!(world.water_visual_dz(&world.agent_id), 0.0);
    }

    /// HUD attachments are not placed again at each frame: what moves
    /// them on the screen without an update of theirs is noted for the
    /// sync, once.
    #[test]
    fn hud_attachments_are_listed_when_their_place_on_the_screen_changes() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        let me = world.objects.index_of_uuid(&world.agent_id).expect("our avatar");
        let huds: Vec<usize> = world
            .objects
            .children_of(&world.objects.get(me).expect("avatar").key)
            .iter()
            .copied()
            .filter(|&i| world.objects.get(i).is_some_and(|o| (31..=38).contains(&o.attachment_point())))
            .collect();
        assert_eq!(huds.len(), 8);
        let mut scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        // what a sync does with the noted objects
        let noted = |scene: &mut Scene, world: &mut World| -> Vec<usize> {
            scene.note_hud_changes(world);
            let mut out = scene.sets.dirty.take();
            out.retain(|&i| world.objects.get(i).is_some_and(|o| o.render.needs_records));
            for &i in &out {
                world.objects.get_mut_untracked(i).expect("object").render.needs_records = false;
            }
            out.sort_unstable();
            out
        };
        for &i in &huds {
            world.objects.get_mut_untracked(i).expect("hud").render.needs_records = false;
        }
        let mut sorted = huds.clone();
        sorted.sort_unstable();
        // first seen: all of them; then nothing while nothing changes
        assert_eq!(noted(&mut scene, &mut world), sorted);
        assert_eq!(noted(&mut scene, &mut world), Vec::<usize>::new());
        // the window is resized: every HUD point moves
        world.hud_aspect = 2.0;
        assert_eq!(noted(&mut scene, &mut world), sorted);
        assert_eq!(noted(&mut scene, &mut world), Vec::<usize>::new());
        // one HUD is zoomed, another one dragged: those two
        let id = |world: &World, i: usize| world.objects.get(i).expect("hud").full_id;
        world.hud_scales.set(id(&world, huds[2]), 1.5);
        assert_eq!(noted(&mut scene, &mut world), vec![huds[2]]);
        // (an offset is kept for the HUDs that have a local profile)
        world.hud_scales.set(id(&world, huds[5]), 1.0);
        assert_eq!(noted(&mut scene, &mut world), Vec::<usize>::new());
        world.hud_scales.set_offset(id(&world, huds[5]), Vec3::new(0.0, 0.1, -0.05));
        assert_eq!(noted(&mut scene, &mut world), vec![huds[5]]);
        // the world zoom of the HUDs is the projection's: nothing to place
        world.hud_zoom = 0.5;
        assert_eq!(noted(&mut scene, &mut world), Vec::<usize>::new());
        // detached: forgotten, and noted again if it comes back
        world.objects.remove_idx(huds[0]);
        assert_eq!(noted(&mut scene, &mut world), Vec::<usize>::new());
        assert_eq!(scene.hud_roots.len(), 7);
    }

    /// The busy crowd holds what its documentation says, each kind where
    /// the sync treats it as such: nothing of it is placed by the GPU.
    #[test]
    fn the_busy_crowd_adds_what_the_sync_places_itself() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        let n = 3;
        let events = crate::demo::events()
            .into_iter()
            .chain(crate::demo::crowd_events(n, false))
            .collect::<Vec<_>>();
        for ev in events {
            world.apply(ev);
        }
        let before = world.objects.len();
        for ev in crate::demo::crowd_busy_events(n) {
            world.apply(ev);
        }
        let added = n * 4 + crate::demo::BUSY_SPINNERS * 2 + crate::demo::BUSY_HUD_PRIMS;
        assert_eq!(world.objects.len(), before + added as usize);
        let any = |_: &Object, slot: u8| Some(slot as u32);
        let (mut flexible, mut spinning, mut below, mut hud) = (0, 0, 0, 0);
        for idx in 0..world.objects.slots.len() {
            let Some(o) = world.objects.get(idx) else {
                continue;
            };
            let parent = world.objects.parent_of(o).and_then(|p| world.objects.get(p));
            if world.hud_root(idx).is_some() {
                hud += 1;
                assert!(anchors::anchored(&world, o, any).is_none());
            } else if o.is_flexible() {
                flexible += 1;
                assert!(parent.is_some_and(|p| p.is_avatar()) && anchors::anchored(&world, o, any).is_none());
            } else if o.has_motion() {
                spinning += 1;
                assert!(anchors::anchored(&world, o, any).is_none());
            } else if parent.is_some_and(|p| p.has_motion()) {
                below += 1;
                assert!(anchors::anchored(&world, o, any).is_none());
            }
        }
        assert_eq!(hud, crate::demo::BUSY_HUD_PRIMS);
        assert_eq!(flexible, n);
        assert_eq!(spinning, n + crate::demo::BUSY_SPINNERS);
        assert_eq!(below, 2 * n + crate::demo::BUSY_SPINNERS);
    }

    #[test]
    fn cached_parent_transforms_give_the_same_placement() {
        let (world, [avatar, hat, child]) = posed_attachment();
        let now = Instant::now();
        let direct: Vec<Xf> = [avatar, hat, child]
            .iter()
            .map(|&i| Scene::object_transform(&world, i, now, 0).expect("placed"))
            .collect();
        // the levels of a frame: the avatar, then its attachment, then the prim
        let mut cache = XfCache::default();
        cache.next_frame(world.objects.slots.len());
        for (k, &i) in [avatar, hat, child].iter().enumerate() {
            let xf = Scene::object_transform_in(&world, i, now, 0, Some(&cache)).expect("placed");
            assert_eq!(xf, direct[k]);
            cache.set(i, xf);
        }
        // the prim follows the posed bone, not the rest pose
        let mut rest = World::new(world.avatar_lib.clone());
        for ev in crate::demo::events() {
            rest.apply(ev);
        }
        let hat_rest = Scene::object_transform(&rest, hat, now, 0).expect("placed");
        assert!(hat_rest.0.distance(direct[1].0) > 1e-3);
    }

    #[test]
    fn skeletons_are_checked_again_only_when_their_objects_change() {
        let (mut world, [avatar, hat, child]) = posed_attachment();
        let lib = world.avatar_lib.clone();
        let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/skeleton-tests")
            .join(Uuid::new_v4().to_string());
        let mut scene = Scene::new(cache, lib);
        let now = Instant::now();
        let avatar_id = world.objects.get(avatar).expect("avatar").full_id;
        scene.skeleton_of(&world, avatar_id, avatar, now);
        let mut drained = Vec::new();
        world.objects.take_touched(&mut drained);
        let changed = |scene: &mut Scene, world: &World| scene.changed_skeletons(world).expect("no new mesh");
        assert!(changed(&mut scene, &world).is_empty());
        // a prim of an attachment updated
        world.objects.touch(child);
        assert!(changed(&mut scene, &world).contains(&avatar_id));
        world.objects.take_touched(&mut drained);
        // the hat dropped on the ground: its old owner is checked again
        let region = world.objects.get(hat).expect("hat").key.region;
        let mut u = crate::demo::action_avatar(false);
        u.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        u.local_id = 9100;
        u.full_id = world.objects.get(hat).expect("hat").full_id;
        u.parent_id = 0;
        world.objects.upsert(region, u);
        assert!(changed(&mut scene, &world).contains(&avatar_id));
        world.objects.take_touched(&mut drained);
        // mesh metadata arrived: every skeleton
        scene
            .meshes
            .insert_demo_skin(crate::demo::ANIMESH_MESH, aurora_assets::SkinInfo::default());
        assert!(scene.changed_skeletons(&world).is_none());
        assert!(changed(&mut scene, &world).is_empty());
    }

    pub(in crate::scene) fn test_face(p: Pass, glow: bool) -> FaceDraw {
        FaceDraw {
            te_index: 0,
            record: 0,
            cmd: DrawCmd {
                index_count: 6,
                first_index: 0,
                base_vertex: 0,
                record: 0,
                bounds: [0.0; 4],
            },
            base_slot: 0,
            tex_id: Uuid::nil(),
            aux_tex: [Uuid::nil(); 3],
            te_alpha: 1.0,
            pbr_alpha: None,
            legacy_alpha: None,
            two_sided: false,
            repeats: 1.0,
            glow,
            pass: p,
            glow_pool: false,
            rez_glow: false,
            system: None,
            first_person_hidden: false,
        }
    }

    #[test]
    fn hide_water_is_invisible_but_keeps_its_gpu_exclusion_draw() {
        let mut face = FaceDraw {
            tex_id: aurora_prim::te::WATER_EXCLUSION_TEXTURE,
            base_slot: aurora_render::textures::TRANSPARENT,
            ..test_face(Pass::Hidden, true)
        };
        assert_eq!(classify(&face, AlphaKind::Mask), Pass::Hidden);
        assert!(face.water_exclusion());
        assert_eq!(
            cull_face(&face, 0, true).bits,
            aurora_render::gpu_cull::pass::WATER_EXCLUSION | FACE_FIRST
        );
        assert!(!is_special(&[face]), "a hidden water mask must not add glow draws");
        face.te_alpha = 0.5;
        assert!(!face.water_exclusion());
        face.te_alpha = 1.0;
        face.pbr_alpha = Some(0);
        assert!(!face.water_exclusion());
        face.pbr_alpha = None;
        face.tex_id = aurora_prim::te::TRANSPARENT_TEXTURE;
        assert!(!face.water_exclusion(), "ordinary transparency must not hide water");
    }

    /// LLVOAvatar::renderSkinned / renderTransparent: the system body is
    /// alpha-tested and never blended, whatever the alpha of its bake; hair,
    /// eyelashes and skirt are blended, and alpha-tested as well.
    #[test]
    fn system_body_parts_follow_the_avatar_pool_not_their_bake_alone() {
        let part = |name: &str, tex: AlphaKind| {
            let mut f = FaceDraw {
                system: Some(SystemPart::of(name)),
                ..test_face(Pass::Hidden, false)
            };
            f.pass = classify(&f, tex);
            (f.pass, f.record_flags())
        };
        for body in ["head", "upper", "lower", "eye_left", "eye_right"] {
            assert_eq!(part(body, AlphaKind::Opaque), (Pass::Opaque, 0), "{body}");
            assert_eq!(part(body, AlphaKind::Mask), (Pass::Mask, flags::ALPHA_MASK), "{body}");
            assert_eq!(part(body, AlphaKind::Blend), (Pass::Mask, flags::ALPHA_MASK), "{body}");
        }
        for soft in ["hair", "eyelashes", "skirt"] {
            assert_eq!(part(soft, AlphaKind::Opaque), (Pass::Opaque, 0), "{soft}");
            assert_eq!(part(soft, AlphaKind::Mask), (Pass::Mask, flags::ALPHA_MASK), "{soft}");
            assert_eq!(
                part(soft, AlphaKind::Blend),
                (Pass::Blend, flags::ALPHA_BLEND | flags::ALPHA_MASK),
                "{soft}"
            );
        }
        // a part hidden by an invisible bake or a worn mesh stays hidden
        let hidden = FaceDraw {
            system: Some(SystemPart::Body),
            te_alpha: 0.0,
            ..test_face(Pass::Hidden, false)
        };
        assert_eq!(classify(&hidden, AlphaKind::Blend), Pass::Hidden);
        // the other blended faces are not alpha-tested
        let prim = FaceDraw {
            pass: Pass::Blend,
            ..test_face(Pass::Blend, false)
        };
        assert_eq!(prim.record_flags(), flags::ALPHA_BLEND);
    }

    /// LLFace::canRenderAsMask: never auto alpha-mask a rigged face, a HUD
    /// face, or a glowing face. Soft genital / hair alphas stay Blend. An
    /// explicit MASK material or PBR mask still Masks.
    #[test]
    fn rigged_faces_are_never_auto_alpha_masked() {
        let alpha = [AlphaKind::Mask];
        let channel = [true];
        let face = |glow: bool, legacy: Option<u8>, pbr: Option<u8>| FaceDraw {
            glow,
            legacy_alpha: legacy,
            pbr_alpha: pbr,
            base_slot: 0,
            ..test_face(Pass::Hidden, false)
        };
        // auto-mask allowed: binary texture → Mask
        assert_eq!(face_mode(&face(false, None, None), &alpha, &channel, true).0, Pass::Mask);
        // rigged / HUD (object_auto_mask false): same texture → Blend
        assert_eq!(face_mode(&face(false, None, None), &alpha, &channel, false).0, Pass::Blend);
        // glowing face: never auto-mask
        assert_eq!(face_mode(&face(true, None, None), &alpha, &channel, true).0, Pass::Blend);
        // creator MASK material still Masks on a rigged mesh
        assert_eq!(
            face_mode(&face(false, Some(legacy_mat::alpha_mode::MASK), None), &alpha, &channel, false).0,
            Pass::Mask
        );
        // PBR mask still Masks
        assert_eq!(face_mode(&face(false, None, Some(2)), &alpha, &channel, false).0, Pass::Mask);
        // two-sided auto-Mask → Blend when auto-mask is off
        let two = FaceDraw {
            two_sided: true,
            base_slot: 0,
            ..test_face(Pass::Hidden, false)
        };
        assert_eq!(face_mode(&two, &alpha, &channel, false).0, Pass::Blend);
        assert_eq!(face_mode(&two, &alpha, &channel, true).0, Pass::MaskTwoSided);
    }

    /// LLVolumeGeometryManager::rebuildGeom / genDrawInfo: a zero base color
    /// alpha hides a blended glTF face only. An opaque one ignores it and a
    /// masked one is left to the shader's cutoff; a Blinn-Phong face with a
    /// zero alpha stays hidden.
    #[test]
    fn zero_alpha_hides_a_gltf_face_only_when_blended() {
        let face = |pbr_alpha: Option<u8>, two_sided: bool| FaceDraw {
            te_alpha: 0.0,
            pbr_alpha,
            two_sided,
            ..test_face(Pass::Hidden, false)
        };
        for tex in [AlphaKind::Opaque, AlphaKind::Mask, AlphaKind::Blend] {
            assert_eq!(classify(&face(Some(0), false), tex), Pass::Opaque, "{tex:?}");
            assert_eq!(classify(&face(Some(0), true), tex), Pass::OpaqueTwoSided, "{tex:?}");
            assert_eq!(classify(&face(Some(2), false), tex), Pass::Mask, "{tex:?}");
            assert_eq!(classify(&face(Some(2), true), tex), Pass::MaskTwoSided, "{tex:?}");
            assert_eq!(classify(&face(Some(1), false), tex), Pass::Hidden, "{tex:?}");
            assert_eq!(classify(&face(None, false), tex), Pass::Hidden, "{tex:?}");
        }
        assert!(!face(Some(0), false).fully_transparent());
        assert!(!face(Some(2), false).fully_transparent());
        assert!(face(Some(1), false).fully_transparent());
        assert!(face(None, false).fully_transparent());
        // a faint blended face is still drawn
        let faint = FaceDraw {
            te_alpha: 0.01,
            ..face(Some(1), false)
        };
        assert_eq!(classify(&faint, AlphaKind::Opaque), Pass::Blend);
    }

    #[test]
    fn transparency_toggle_rebuilds_invisible_animesh_once_and_preserves_face_selection() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events() {
            world.apply(event);
        }
        let mut objects = crate::demo::animesh_objects(crate::demo::ANIMESH_MESH);
        crate::demo::make_animesh_invisible(&mut objects);
        world.apply(NetEvent::ObjectUpdates {
            handle: world.main_region.unwrap(),
            objects,
        });
        let index = |local| world.objects.iter().find(|(_, o)| o.key.local_id == local).unwrap().0;
        let (opaque, invisible_root, invisible_mesh) = (index(9200), index(9201), index(9202));
        let unrigged_children = [index(9205), index(9206)];
        let mut scene = Scene::new(Default::default(), lib);
        let selection = HashMap::from([(invisible_mesh, 1)]);
        scene.set_face_selection(&mut world, selection.clone());
        let clean = |scene: &mut Scene, world: &mut World| {
            scene.sets.dirty.take();
            for object in world.objects.slots.iter_mut().flatten() {
                object.material_dirty = false;
                object.render.needs_records = false;
            }
        };
        clean(&mut scene, &mut world);
        scene.set_debug_alpha(&mut world, Some(false));
        for idx in [invisible_root, invisible_mesh].into_iter().chain(unrigged_children) {
            let object = world.objects.get(idx).unwrap();
            assert!(object.material_dirty && object.render.needs_records);
            assert!(scene.sets.dirty.contains(idx));
        }
        assert!(!world.objects.get(opaque).unwrap().material_dirty);
        for idx in unrigged_children {
            let object = world.objects.get(idx).unwrap();
            assert!(object.visual_volume().is_mesh());
            assert_eq!(object.visual_volume().sculpt.unwrap().texture, crate::demo::ANIMESH_UNRIGGED_MESH);
            assert!(Scene::skeleton_owner(&world, idx).is_some());
        }
        assert_eq!(Scene::skeleton_owner(&world, invisible_mesh).unwrap().1, invisible_root);
        clean(&mut scene, &mut world);
        // An unchanged overlay and its independent rigged filter need no
        // material rebuild: only the draw lists change.
        scene.set_debug_alpha(&mut world, Some(false));
        scene.set_debug_alpha(&mut world, Some(true));
        assert_eq!(scene.sets.dirty.len(), 0);
        assert!(!world.objects.get(invisible_mesh).unwrap().material_dirty);
        scene.set_debug_alpha(&mut world, None);
        assert!(world.objects.get(invisible_mesh).unwrap().material_dirty);
        assert_eq!(scene.selection_faces, selection);
        clean(&mut scene, &mut world);
        scene.set_debug_alpha(&mut world, None);
        assert_eq!(scene.sets.dirty.len(), 0);
        scene.set_debug_alpha(&mut world, Some(true));
        assert!(scene.sets.dirty.contains(invisible_mesh));
    }

    #[test]
    fn selecting_opaque_faces_keeps_records_and_transparent_faces_rebuild_only_on_changes() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::face_selection_events(false)) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(99620)).unwrap();
        let object = world.objects.get_mut(idx).unwrap();
        Arc::make_mut(object.te.as_mut().unwrap()).faces[0].color[3] = 0.0;
        object.render.needs_records = false;
        object.material_dirty = false;
        let mut scene = Scene::new(Default::default(), lib);
        scene.set_face_selection(&mut world, HashMap::from([(idx, 1 << 1)]));
        assert!(!world.objects.get(idx).unwrap().render.needs_records);
        scene.set_face_selection(&mut world, HashMap::from([(idx, (1 << 1) | 1)]));
        assert!(world.objects.get(idx).unwrap().render.needs_records);
        assert!(world.objects.get(idx).unwrap().material_dirty);
        world.objects.get_mut(idx).unwrap().render.needs_records = false;
        world.objects.get_mut(idx).unwrap().material_dirty = false;
        scene.set_face_selection(&mut world, HashMap::from([(idx, (1 << 1) | 1)]));
        assert!(!world.objects.get(idx).unwrap().render.needs_records);
        scene.set_face_selection(&mut world, HashMap::from([(idx, 1)]));
        assert!(!world.objects.get(idx).unwrap().render.needs_records);
        scene.set_face_selection(&mut world, HashMap::new());
        assert!(world.objects.get(idx).unwrap().render.needs_records);
    }

    #[test]
    fn high_texture_entry_selection_does_not_wrap_onto_another_face() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        for event in crate::demo::events().into_iter().chain(crate::demo::face_selection_events(false)) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&crate::demo::action_id(99620)).unwrap();
        let object = world.objects.get_mut(idx).unwrap();
        Arc::make_mut(object.te.as_mut().unwrap()).faces[44].color[3] = 0.0;
        object.render.needs_records = false;
        object.material_dirty = false;
        let mut scene = Scene::new(Default::default(), lib);
        scene.set_face_selection(&mut world, HashMap::from([(idx, 1u64 << 12)]));
        assert!(!world.objects.get(idx).unwrap().render.needs_records);
        scene.set_face_selection(&mut world, HashMap::from([(idx, (1u64 << 12) | (1u64 << 44))]));
        assert!(world.objects.get(idx).unwrap().render.needs_records);
        world.objects.get_mut(idx).unwrap().render.needs_records = false;
        scene.set_face_selection(&mut world, HashMap::from([(idx, (1u64 << 12) | (1u64 << 44))]));
        assert!(!world.objects.get(idx).unwrap().render.needs_records);
        scene.set_face_selection(&mut world, HashMap::from([(idx, 1u64 << 12)]));
        assert!(world.objects.get(idx).unwrap().render.needs_records);
    }

    #[test]
    fn selection_pulse_ends_and_unchanged_frames_do_not_restart_it() {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        let mut scene = Scene::new(Default::default(), lib);
        let selected = HashMap::from([(7, 1 << 3)]);
        scene.set_face_selection(&mut world, selected.clone());
        let at = scene.selection_changed.unwrap();
        assert!((scene.selection_pulse(at) - 0.18).abs() < 1e-6);
        assert!(scene.selection_pulse(at + std::time::Duration::from_millis(350)) < 0.05);
        scene.set_face_selection(&mut world, selected);
        assert_eq!(scene.selection_changed, Some(at));
        assert_eq!(scene.selection_pulse(at + std::time::Duration::from_millis(700)), 0.0);
        scene.reveal_selected_faces();
        assert!(scene.selection_pulse(Instant::now()) > 0.17);
        scene.set_face_selection(&mut world, HashMap::new());
        assert_eq!(scene.selection_pulse(Instant::now()), 0.0);
    }

    /// The GPU culling (reference of cull.wgsl, `gpu_cull::face_bins`) takes
    /// the same decisions as the CPU lists for random objects, avatars and
    /// avatar states: main view bins, shadow casters, reflections, probes
    /// and visible objects.
    #[test]
    fn gpu_culling_matches_the_cpu_lists() {
        use aurora_render::gpu_cull::{BIN_PROBE, BIN_REFL_WATER, BIN_SHADOW, BIN_SHADOW_ALPHA, CullFrame, face_bins, frame_flags};
        let mut seed = 0x853c_49e6_748f_ea9bu64;
        let mut rand = move |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as u32
        };
        let eye = Vec3::new(128.0, 128.0, 25.0);
        let view_m = glam::camera::rh::view::look_at_mat4(eye, eye + Vec3::new(1.0, 0.3, -0.1), Vec3::Z);
        let fov = 1.0f32;
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(fov, 16.0 / 9.0, 0.1);
        let view = CullView::new(proj * view_m, eye, 900.0, fov);
        let passes = [
            Pass::Opaque,
            Pass::OpaqueTwoSided,
            Pass::Mask,
            Pass::MaskTwoSided,
            Pass::Blend,
            Pass::Hidden,
        ];
        let n = 4000usize;
        // anchors of worn objects (whole metres: the same sum to the bit
        // on both sides)
        let anchor_at: Vec<Vec3> = (0..32)
            .map(|_| Vec3::new(rand(100) as f32, rand(100) as f32, rand(20) as f32))
            .collect();
        let palettes: Vec<[[f32; 4]; 4]> = anchor_at.iter().map(|t| Mat4::from_translation(*t).to_cols_array_2d()).collect();
        let mut gpu: Vec<ObjGpu> = (0..n).map(|_| ObjGpu::default()).collect();
        for (i, g) in gpu.iter_mut().enumerate() {
            g.center = eye + Vec3::new(rand(400) as f32 - 200.0, rand(400) as f32 - 200.0, rand(60) as f32 - 30.0);
            g.radius = 0.02 + rand(400) as f32 * 0.01;
            g.is_avatar = rand(12) == 0;
            if g.is_avatar {
                g.posed_sphere = (rand(2) == 0).then(|| (g.center + Vec3::Z, 0.9));
            } else if rand(4) == 0 {
                // an attachment, sometimes of an index that is no avatar
                let wearer = rand(n as u32);
                g.owner_avatar = Some(wearer as usize);
                if rand(2) == 0 {
                    // placed by the GPU: the table holds its sphere
                    // relative to the anchor
                    let matrix = rand(anchor_at.len() as u32);
                    g.anchor = Some(anchors::AnchorRef { wearer, slot: 1, matrix });
                    g.local_center = g.center - anchor_at[matrix as usize];
                }
            }
            g.rigged = rand(6) == 0;
            g.hud = rand(40) == 0;
            g.skeleton_owner = (rand(10) == 0).then(Uuid::new_v4);
            let faces = rand(3) as usize;
            g.faces = (0..faces).map(|_| test_face(passes[rand(6) as usize], false)).collect();
            let _ = i;
        }
        let full: Vec<bool> = (0..n).map(|i| gpu[i].is_avatar && rand(3) != 0).collect();
        let complex: Vec<bool> = (0..n).map(|i| gpu[i].is_avatar && rand(5) == 0).collect();
        let loading: Vec<bool> = (0..n).map(|i| gpu[i].is_avatar && rand(6) == 0).collect();
        for i in 0..n {
            let mut st = 0;
            for (on, bit) in [
                (full[i], avatar_state::FULL),
                (complex[i], avatar_state::TOO_COMPLEX),
                (loading[i], avatar_state::LOADING),
            ] {
                if on {
                    st |= bit;
                }
            }
            gpu[i].cull_state = st;
        }
        let ctx = CullCtx {
            view: &view,
            draw_distance: 128.0,
            shadow_distance: 128.0,
            reflection_distance: 64.0,
            shadows: true,
            reflections: true,
        };
        let gview = GpuCullView {
            first_person: None,
            planes: view.planes,
            eye,
            draw_distance: ctx.draw_distance,
            shadow_distance: ctx.shadow_distance,
            reflection_distance: ctx.reflection_distance,
            pixel_scale: view.screen_height / view.tan_half_fov,
            shadows: true,
            reflections: true,
        };
        // cascades and reflection views that hold everything: the bins then
        // show the CPU's casts / reflects decisions
        let all = Mat4::from_scale(Vec3::splat(1e-5));
        let flags = frame_flags::SHADOWS | frame_flags::REFLECTIONS | frame_flags::WATER | frame_flags::PROBE | frame_flags::BLEND_CASTS;
        let mut frame = CullFrame::new(
            &gview,
            &[all; 3],
            &[aurora_render::shadow_cull::CASTER_PLANES_ALL; 3],
            0.0,
            [Mat4::IDENTITY; 3],
            [None; 2],
            flags,
        );
        frame.water = [[0.0, 0.0, 0.0, 1.0]; 6];
        frame.probe = frame.water;
        frame.sizes[1] = n as u32;
        let objects: Vec<CullObject> = gpu.iter().enumerate().map(|(i, g)| g.cull_object(i)).collect();
        let spheres = aurora_render::gpu_cull::world_spheres(&objects, &palettes, &[], &[]);
        let mut faces = Vec::new();
        let mut owner = Vec::new();
        for (i, g) in gpu.iter().enumerate() {
            for (fi, f) in g.faces.iter().enumerate() {
                faces.push(cull_face(f, i, fi == 0));
                owner.push((i, fi));
            }
        }
        let (mut drawn, mut casting) = (0, 0);
        for (k, &(i, fi)) in owner.iter().enumerate() {
            let g = &gpu[i];
            let f = &g.faces[fi];
            let (m, visible) = face_bins(&frame, &faces, &objects, &spheres, k);
            let vis = object_visibility(g, i, &ctx, |a| full.get(a).copied().unwrap_or(true), |a| complex[a], |a| loading[a]);
            let solid = matches!(f.pass, Pass::Opaque | Pass::OpaqueTwoSided | Pass::Mask | Pass::MaskTwoSided);
            let (main, casts, refl, probe, in_view) = match &vis {
                Some(v) => (
                    v.in_view && solid,
                    v.casts && f.pass != Pass::Hidden,
                    v.reflects && solid,
                    v.reflects && solid && g.avatar(i).is_none() && g.skeleton_owner.is_none(),
                    v.in_view,
                ),
                None => (false, false, false, false, false),
            };
            assert_eq!(m & 0xf != 0, main, "main view, object {i}");
            if main {
                assert_eq!(m & 0xf, 1 << (f.pass.gpu_code() - 1), "pass bin, object {i}");
            }
            // depth only unless the face's shadow is alpha tested
            let (bin, other) = if f.pass.record_flags() == 0 {
                (BIN_SHADOW, BIN_SHADOW_ALPHA)
            } else {
                (BIN_SHADOW_ALPHA, BIN_SHADOW)
            };
            assert_eq!(m >> bin & 7 == 7, casts, "shadows, object {i}");
            assert_eq!(m >> other & 7, 0, "shadows of the other kind, object {i}");
            assert_eq!(m >> BIN_REFL_WATER & 1 == 1, refl, "reflection, object {i}");
            assert_eq!(m >> BIN_PROBE & 1 == 1, probe, "probe, object {i}");
            assert_eq!(visible, in_view && fi == 0, "visible object {i}");
            drawn += main as u32;
            casting += casts as u32;
        }
        assert!(
            drawn > 50 && casting > 100,
            "the scene exercises the tests ({drawn} drawn, {casting} casting)"
        );
    }
}
