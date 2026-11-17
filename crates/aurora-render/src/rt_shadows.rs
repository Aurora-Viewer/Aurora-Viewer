//! Ray-traced sun shadows of the main view, an experiment behind
//! `AURORA_RT_SHADOWS` (off by default; Vulkan devices with ray queries).
//!
//! Through wgpu's ray queries (`EXPERIMENTAL_RAY_QUERY`): a bottom level per
//! face geometry of the arena, a top level of the faces placed in the world,
//! built again only when one is added or moves, and one compute pass after
//! the contact shadows that traces a ray per pixel of the depth prepass
//! towards the sun (shaders/rt_shadows.wgsl). Its mask takes the contact
//! shadows' place in the frame group, the darker of the two, so the shading
//! reads one texture as before.
//!
//! What is traced: the opaque and alpha-masked faces of objects placed in
//! the world, the terrain, and the skeletons (avatars with what they wear,
//! animated objects): their faces are posed on the GPU (shaders/rt_pose.wgsl,
//! the vertex shaders' skinning) into a bottom level each, built again only
//! when the skeleton moved, in a top level of their own. Every instance
//! carries its record, so a hit finds its surface (shaders/rt_common.wgsl):
//! an alpha-masked face lets the ray through under its cutoff. Not yet:
//! blended faces. The shadow maps still shade those passes the mask does
//! not cover (blended faces, reflections).

use std::collections::HashMap;
use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};

use crate::arena::RecordStore;
use crate::gpu_cull::{CullTables, NO_OBJECT, object_flags, pass};
use crate::pipeline_cache::CachedPipelines;
use crate::types::flags;

/// The device feature the experiment needs.
pub const FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;

/// Which faces a ray passes (`AURORA_RT_SHADOWS`): `1` none, `front` those
/// it meets from their front, `back` those it meets from behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cull {
    None,
    Front,
    Back,
}

impl Cull {
    /// The ray flags of WGSL's `RayDesc` (Vulkan's `gl_RayFlags*`).
    fn ray_flags(self) -> u32 {
        match self {
            Cull::None => 0,
            Cull::Back => 0x10,
            Cull::Front => 0x20,
        }
    }
}

/// The experiment's mode, read once: None when `AURORA_RT_SHADOWS` is unset,
/// empty or `0`.
pub fn mode() -> Option<Cull> {
    static MODE: OnceLock<Option<Cull>> = OnceLock::new();
    *MODE.get_or_init(|| parse(std::env::var("AURORA_RT_SHADOWS").ok().as_deref()))
}

fn parse(value: Option<&str>) -> Option<Cull> {
    match value?.trim() {
        "" | "0" | "off" => None,
        "front" => Some(Cull::Front),
        "back" => Some(Cull::Back),
        _ => Some(Cull::None),
    }
}

/// Whether the traced reflections are on too (`AURORA_RT_REFLECTIONS`, with
/// `AURORA_RT_SHADOWS`): they take SSR's place where SSR is on.
pub fn reflections() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_REFLECTIONS").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether the traced indirect light is on too (`AURORA_RT_GI`, with
/// `AURORA_RT_SHADOWS`): it takes the screen-space indirect light's place.
pub fn gi() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_GI").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether the local point lights' shadows are traced too
/// (`AURORA_RT_LIGHTS`, with `AURORA_RT_SHADOWS`).
pub fn local_lights() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| mode().is_some() && std::env::var("AURORA_RT_LIGHTS").is_ok_and(|v| !matches!(v.trim(), "" | "0" | "off")))
}

/// Whether to ask a device for [`FEATURES`]: the experiment is on and the
/// adapter has them.
pub fn wanted(adapter: wgpu::Features) -> bool {
    mode().is_some() && adapter.contains(FEATURES)
}

/// A face to trace: its record, its geometry in the arena, where it stands,
/// and whether it is opaque (an alpha-masked face is tested at each hit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub record: u32,
    pub base_vertex: i32,
    pub first_index: u32,
    pub index_count: u32,
    pub opaque: bool,
    pub model: [[f32; 4]; 4],
}

/// A rigged or worn face: its world positions exist only once posed, so the
/// pose pass (shaders/rt_pose.wgsl) writes them on the frames its skeleton's
/// bottom level is built again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub record: u32,
    pub base_vertex: u32,
    pub first_index: u32,
    pub index_count: u32,
    pub opaque: bool,
}

/// A skeleton's faces (an avatar with what it wears, or an animated
/// object), keyed by its palette's first entry: one bottom level each, built
/// again on the frames `rebuild` says, from `Faces::posed[first..first + count]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PosedGroup {
    pub key: u32,
    pub first: u32,
    pub count: u32,
    /// Posed vertices of all its faces (one per index).
    pub vertices: u32,
    /// A hash of its faces: another set makes its bottom level again.
    pub faces: u64,
    pub rebuild: bool,
}

/// What the frame traces: faces placed by their model matrix, and the
/// skeletons' faces, posed on the GPU.
#[derive(Clone, Debug, Default)]
pub struct Faces {
    pub placed: Vec<Instance>,
    /// The faces of the skeletons built again this frame, by skeleton.
    pub posed: Vec<Posed>,
    pub groups: Vec<PosedGroup>,
}

/// Posed triangles built again in one frame, nearest skeleton first (the
/// nearest is always built); the others keep an earlier frame's pose.
const REBUILD_TRIANGLES: u64 = 1 << 19;
/// How far from its joint a vertex is taken to be when a joint turns (m).
const JOINT_REACH: f32 = 1.0;

/// What the main thread remembers of each skeleton's last built pose, so a
/// skeleton whose vertices moved less than a shadow ray's start can tell
/// keeps its bottom level (wgpu has no refit: a level is only built again
/// from scratch): the nearest skeletons first, within a frame's budget.
#[derive(Default)]
pub(crate) struct PoseTracker {
    seen: HashMap<u32, Seen>,
    frame: u64,
    scratch: Vec<Candidate>,
}

struct Seen {
    faces: u64,
    /// A level was built for these faces (one left waiting by the budget
    /// has none yet).
    built: bool,
    joints: Vec<[[f32; 4]; 4]>,
    models: Vec<[[f32; 4]; 4]>,
    last: u64,
}

struct Candidate {
    key: u32,
    face: Posed,
    model: [[f32; 4]; 4],
    origin: [f32; 3],
}

/// How far a vertex can have moved between two placements of its joint.
fn drift(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> f32 {
    let mut d = 0.0f32;
    for c in 0..3 {
        for r in 0..3 {
            d = d.max((a[c][r] - b[c][r]).abs() * JOINT_REACH);
        }
    }
    for r in 0..3 {
        d = d.max((a[3][r] - b[3][r]).abs());
    }
    d
}

/// The drift a skeleton at `distance` may have before its shadow shows it:
/// the trace's ray start offset there (rt_shadows.wgsl).
fn drift_allowed(distance: f32) -> f32 {
    0.01 + 0.002 * distance
}

fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    (glam::Mat4::from_cols_array_2d(a) * glam::Mat4::from_cols_array_2d(b)).to_cols_array_2d()
}

/// The faces to trace this frame, from the records and the culling tables
/// the main thread keeps: opaque and alpha-masked faces (blended ones are
/// not traced yet), and no HUD. `palettes` is what the GPU's joint
/// palettes hold, `camera` the eye.
pub(crate) fn gather(
    records: &RecordStore,
    cull: &CullTables,
    palettes: &[[[f32; 4]; 4]],
    camera: glam::Vec3,
    tracker: &mut PoseTracker,
    out: &mut Faces,
) {
    out.placed.clear();
    out.posed.clear();
    out.groups.clear();
    tracker.frame += 1;
    tracker.scratch.clear();
    for id in 0..records.faces.len() {
        let Some(face) = records.faces.get(id) else {
            continue;
        };
        if face.index_count < 3 || face.base_vertex < 0 {
            continue;
        }
        let Some(record) = records.get(id as u32) else {
            continue;
        };
        let rf = record.flags[0];
        let index_count = face.index_count - face.index_count % 3;
        let opaque = rf & flags::ALPHA_MASK == 0;
        // a top level's custom index holds 24 bits
        if id >= 1 << 24 {
            continue;
        }
        if face.object == NO_OBJECT {
            if rf & flags::TERRAIN != 0 {
                out.placed.push(Instance {
                    record: id as u32,
                    base_vertex: face.base_vertex,
                    first_index: face.first_index,
                    index_count,
                    opaque,
                    model: record.model,
                });
            }
            continue;
        }
        let Some(object) = cull.objects.get(face.object as usize) else {
            continue;
        };
        let face_pass = face.bits & pass::BITS;
        if object.flags & object_flags::ACTIVE == 0
            || object.flags & object_flags::HUD != 0
            || !matches!(face_pass, pass::OPAQUE | pass::OPAQUE_2S | pass::MASK | pass::MASK_2S)
            || rf & flags::ALPHA_BLEND != 0
        {
            continue;
        }
        let worn_or_posed = object_flags::AVATAR | object_flags::RIGGED | object_flags::POSED;
        if object.flags & worn_or_posed != 0 || object.anchor != 0 || rf & (flags::SKINNED | flags::ANCHORED | flags::LIP_SYNC) != 0 {
            let key = record.flags[1];
            let palette = |i: u32| palettes.get(i as usize);
            // where the face stands, as rt_pose.wgsl places it
            let origin = match (rf & flags::SKINNED != 0, rf & flags::ANCHORED != 0) {
                (true, true) => palette(key + crate::types::SKELETON_ANCHOR).map(|m| m[3]),
                (false, true) => palette(key).map(|m| mul(m, &record.model)[3]),
                _ => Some(record.model[3]),
            }
            .unwrap_or(record.model[3]);
            tracker.scratch.push(Candidate {
                key,
                face: Posed {
                    record: id as u32,
                    base_vertex: face.base_vertex as u32,
                    first_index: face.first_index,
                    index_count,
                    opaque,
                },
                model: record.model,
                origin: [origin[0], origin[1], origin[2]],
            });
        } else {
            out.placed.push(Instance {
                record: id as u32,
                base_vertex: face.base_vertex,
                first_index: face.first_index,
                index_count,
                opaque,
                model: record.model,
            });
        }
    }
    choose_rebuilds(palettes, camera, tracker, out);
}

/// Groups the posed candidates by skeleton and picks the skeletons built
/// again this frame: new ones, those whose faces changed, and those whose
/// vertices drifted past what a shadow shows, nearest first within
/// [`REBUILD_TRIANGLES`].
fn choose_rebuilds(palettes: &[[[f32; 4]; 4]], camera: glam::Vec3, tracker: &mut PoseTracker, out: &mut Faces) {
    use std::hash::{Hash, Hasher};
    let frame = tracker.frame;
    let stride = crate::types::PALETTE_STRIDE as usize;
    tracker.scratch.sort_by_key(|c| c.key);
    // (start, end, distance, must, faces' hash) of each skeleton's run
    let mut runs: Vec<(usize, usize, f32, bool, u64)> = Vec::new();
    let mut start = 0;
    while start < tracker.scratch.len() {
        let key = tracker.scratch[start].key;
        let mut end = start;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        while end < tracker.scratch.len() && tracker.scratch[end].key == key {
            let f = tracker.scratch[end].face;
            (f.record, f.base_vertex, f.first_index, f.index_count, f.opaque).hash(&mut hasher);
            end += 1;
        }
        let faces = hasher.finish();
        let run = &tracker.scratch[start..end];
        let distance = glam::Vec3::from(run[0].origin).distance(camera);
        let joints = palettes
            .get(key as usize..(key as usize + stride).min(palettes.len()))
            .unwrap_or(&[]);
        let must = match tracker.seen.get_mut(&key) {
            Some(seen) if seen.built && seen.faces == faces && seen.joints.len() == joints.len() => {
                seen.last = frame;
                let moved = seen
                    .joints
                    .iter()
                    .zip(joints)
                    .map(|(a, b)| drift(a, b))
                    .chain(seen.models.iter().zip(run).map(|(a, c)| drift(a, &c.model)))
                    .fold(0.0f32, f32::max);
                moved > drift_allowed(distance)
            }
            Some(seen) if seen.faces == faces => {
                seen.last = frame;
                true
            }
            _ => {
                tracker.seen.insert(
                    key,
                    Seen {
                        faces,
                        built: false,
                        joints: Vec::new(),
                        models: Vec::new(),
                        last: frame,
                    },
                );
                true
            }
        };
        runs.push((start, end, distance, must, faces));
        start = end;
    }
    tracker.seen.retain(|_, s| s.last == frame);
    // the skeletons that must be built again, nearest first, within budget
    let mut order: Vec<usize> = (0..runs.len()).filter(|&r| runs[r].3).collect();
    order.sort_by(|&a, &b| runs[a].2.total_cmp(&runs[b].2));
    let mut chosen = vec![false; runs.len()];
    let mut triangles = 0u64;
    for r in order {
        let (s, e, _, _, _) = runs[r];
        let tris: u64 = tracker.scratch[s..e].iter().map(|c| c.face.index_count as u64 / 3).sum();
        if triangles > 0 && triangles + tris > REBUILD_TRIANGLES {
            continue;
        }
        triangles += tris;
        chosen[r] = true;
    }
    for (r, &(s, e, _, _, faces)) in runs.iter().enumerate() {
        let run = &tracker.scratch[s..e];
        let key = run[0].key;
        let vertices: u32 = run.iter().map(|c| c.face.index_count).sum();
        let first = out.posed.len() as u32;
        if chosen[r] {
            out.posed.extend(run.iter().map(|c| c.face));
            if let Some(seen) = tracker.seen.get_mut(&key) {
                seen.built = true;
                seen.joints.clear();
                seen.joints.extend_from_slice(
                    palettes
                        .get(key as usize..(key as usize + stride).min(palettes.len()))
                        .unwrap_or(&[]),
                );
                seen.models.clear();
                seen.models.extend(run.iter().map(|c| c.model));
            }
        }
        out.groups.push(PosedGroup {
            key,
            first,
            count: if chosen[r] { run.len() as u32 } else { 0 },
            vertices,
            faces,
            rebuild: chosen[r],
        });
    }
}

/// The pass's constants (must match `Params` in rt_shadows.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct ParamsU {
    pub world_from_clip: [[f32; 4]; 4],
    pub camera: [f32; 4],
    pub light: [f32; 4],
    pub screen: [f32; 4],
    pub mode: [f32; 4],
}

/// Bytes of an arena vertex (`types::Vertex`), its position first.
const VERTEX_BYTES: u64 = std::mem::size_of::<crate::types::Vertex>() as u64;
/// The most vertices a face's u16 indices reach past its first.
const FACE_VERTICES: u64 = 1 << 16;
/// Triangles of bottom levels built in one frame: the others wait (and their
/// faces are not traced) until a later frame.
const BUILD_TRIANGLES: u64 = 1 << 20;
/// Frames a bottom level no face used is kept.
const KEEP_FRAMES: u64 = 600;
/// How far a ray goes (m).
const REACH: f32 = 512.0;

const RESULT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The most posed vertices (one per index) a frame traces: the faces past
/// it are left out.
const POSED_VERTICES: u64 = 1 << 22;
/// Bytes of a posed vertex (a vec4 of f32).
const POSED_BYTES: u64 = 16;

/// A posed face of the pose pass (must match `Face` in rt_pose.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PoseFace {
    record: u32,
    base_vertex: u32,
    first_index: u32,
    index_count: u32,
    out_offset: u32,
    _pad: [u32; 3],
}

/// The pose pass's constants (must match `Params` in rt_pose.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PoseParams {
    counts: [u32; 4],
}

/// The pose pass and what it writes: the posed vertices of the skeletons
/// built again this frame, their bottom levels, and a top level of them all.
struct Poser {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    faces: Option<wgpu::Buffer>,
    vertices: Option<wgpu::Buffer>,
    /// The records of the top level's skeletons' faces, in its order
    /// (rt_common.wgsl `posed_records`).
    records: wgpu::Buffer,
    record_list: Vec<u32>,
    /// Each skeleton's bottom level, by its palette's first entry.
    levels: HashMap<u32, Level>,
    /// A top level of its own: built again on the frames a skeleton was,
    /// while the scene's stands as long as nothing in it moves.
    tlas: Option<wgpu::Tlas>,
    /// The skeletons the top level holds, in its order.
    placed: Vec<u32>,
    /// Where each skeleton built this frame starts in the posed vertices.
    offsets: Vec<(usize, u32)>,
    table: Vec<PoseFace>,
    frame: u64,
}

/// A skeleton's bottom level: a geometry per face, so a hit's geometry index
/// is its face (`records`).
struct Level {
    blas: wgpu::Blas,
    /// The hash of the faces it was made for.
    faces: u64,
    records: Vec<u32>,
    sizes: Vec<wgpu::BlasTriangleGeometrySizeDescriptor>,
    used: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    base_vertex: i32,
    first_index: u32,
    index_count: u32,
    opaque: bool,
}

struct Bottom {
    blas: wgpu::Blas,
    used: u64,
}

/// The mask the shading reads in place of the contact shadows', and the
/// traced reflections SSR's composite reads.
pub(crate) struct Targets {
    size: (u32, u32),
    pub result: wgpu::TextureView,
    pub reflect: Option<wgpu::TextureView>,
    pub gi: Option<GiTargets>,
    pub lights: Option<LightTargets>,
}

/// The traced point lights' choices and share (two of each: one read while
/// the other is written), and the sun's mask before the share joins it.
pub(crate) struct LightTargets {
    choice: [wgpu::TextureView; 2],
    share: [wgpu::TextureView; 2],
    sun: wgpu::TextureView,
}

const CHOICE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const SHARE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The lights pass's constants (must match `Params` in rt_lights.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightsU {
    world_from_clip: [[f32; 4]; 4],
    prev_clip_from_world: [[f32; 4]; 4],
    camera: [f32; 4],
    prev_camera: [f32; 4],
    screen: [f32; 4],
    mode: [f32; 4],
}

/// A light of the frame's list (must match `Light` in rt_lights.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
struct TracedLightU {
    pos_radius: [f32; 4],
    color_falloff: [f32; 4],
    source: [f32; 4],
}

/// The most lights of the frame's list (the frame uniform's).
const MAX_TRACED_LIGHTS: usize = 64;

/// The traced point lights' passes (shaders/rt_lights.wgsl).
struct Lights {
    trace_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    lights: wgpu::Buffer,
    /// Last frame's lights: the same list keeps last frame's choices.
    listed: Vec<TracedLightU>,
    timer: Option<Timer>,
    last: usize,
    history_for: Option<(u32, u32)>,
    prev_clip_from_world: glam::Mat4,
    prev_camera: glam::Vec3,
    frame: u32,
}

/// The traced indirect light's history, at half resolution (two of each:
/// one read while the other is written), and its full resolution result.
pub(crate) struct GiTargets {
    half: (u32, u32),
    color: [wgpu::TextureView; 2],
    dist: [wgpu::TextureView; 2],
    pub result: wgpu::TextureView,
}

const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const GI_DIST_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// How far an indirect ray goes (m): the light bounced from further off
/// is faint, and shorter rays are quicker.
const GI_REACH: f32 = 64.0;

/// The GI pass's constants (must match `Params` in rt_gi.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GiU {
    clip_from_world: [[f32; 4]; 4],
    world_from_clip: [[f32; 4]; 4],
    prev_clip_from_world: [[f32; 4]; 4],
    camera: [f32; 4],
    prev_camera: [f32; 4],
    lit: [[f32; 4]; 5],
    screen: [f32; 4],
    half: [f32; 4],
    mode: [f32; 4],
}

/// The traced indirect light's passes (shaders/rt_gi.wgsl).
struct Gi {
    trace_layout: wgpu::BindGroupLayout,
    filter_layout: wgpu::BindGroupLayout,
    trace: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    timer: Option<Timer>,
    /// The history written last frame (`GiTargets::color` and `dist`).
    last: usize,
    /// The size the history was written at, None before the first frame.
    history_for: Option<(u32, u32)>,
    prev_clip_from_world: glam::Mat4,
    prev_camera: glam::Vec3,
    frame: u32,
}

const REFLECT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The reflection pass's constants (must match `Params` in rt_reflect.wgsl).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectU {
    clip_from_world: [[f32; 4]; 4],
    world_from_clip: [[f32; 4]; 4],
    camera: [f32; 4],
    lit: [[f32; 4]; 5],
    screen: [f32; 4],
    mode: [f32; 4],
}

/// The frame's view and light, as the reflections and the GI shade what
/// the view does not see.
pub(crate) struct TraceLight {
    pub clip_from_world: glam::Mat4,
    pub camera: glam::Vec3,
    pub sun_dir: glam::Vec3,
    pub sun_up: bool,
    pub sun_color: [f32; 4],
    pub sky_zenith: [f32; 4],
    pub sky_horizon: [f32; 4],
    pub ground_color: [f32; 4],
}

impl TraceLight {
    /// rt_light.wgsl's `HitLight`.
    fn lit(&self) -> [[f32; 4]; 5] {
        [
            self.sun_dir
                .normalize_or(glam::Vec3::Z)
                .extend(if self.sun_up { 1.0 } else { 0.0 })
                .to_array(),
            self.sun_color,
            self.sky_zenith,
            self.sky_horizon,
            self.ground_color,
        ]
    }
}

/// The traced reflections' pass (shaders/rt_reflect.wgsl).
struct Reflect {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    timer: Option<Timer>,
}

/// What the last frame traced (AURORA_PROFILE).
#[derive(Clone, Copy, Debug, Default)]
pub struct RtStats {
    pub instances: usize,
    pub bottoms: usize,
    pub built: usize,
    pub built_triangles: u64,
    pub waiting: usize,
    pub posed_faces: usize,
    pub posed_vertices: u64,
    pub skeletons: usize,
    pub skeletons_built: usize,
}

pub(crate) struct RtShadows {
    layout: wgpu::BindGroupLayout,
    /// The scene's tables as the ray-traced passes read them (group 1 of
    /// rt_common.wgsl).
    scene_layout: wgpu::BindGroupLayout,
    /// A buffer past the device's binding size was logged.
    warned: bool,
    pipeline: wgpu::ComputePipeline,
    buffer: wgpu::Buffer,
    cull: Cull,
    bottoms: HashMap<Key, Bottom>,
    /// The arena buffers the bottom levels were built from: a new one
    /// (the arena grew) builds them all again.
    arena: Option<(wgpu::Buffer, wgpu::Buffer)>,
    tlas: Option<wgpu::Tlas>,
    /// The faces the top level holds: the same faces in the same places
    /// keep it as it is, with no build.
    built_for: Vec<Instance>,
    frame: u64,
    pub stats: RtStats,
    timer: Option<Timer>,
    poser: Poser,
    reflect: Option<Reflect>,
    gi: Option<Gi>,
    lights: Option<Lights>,
    /// The posed top level holds a skeleton this frame.
    posed_live: bool,
}

/// Readbacks in flight for the pass's timestamps.
const RING: usize = 4;

/// The GPU time of a pass's steps: a timestamp at each step's start and one
/// at the last one's end, resolved and read back a few frames later; their
/// averages come out every [`Timer::REPORT`] frames. The shadows have four
/// (before the pose, after it, before the trace, after it), the reflections
/// two.
struct Timer {
    queries: wgpu::QuerySet,
    /// Timestamps a frame writes.
    count: u32,
    resolve: wgpu::Buffer,
    ring: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::atomic::AtomicU8>)>,
    next: usize,
    period_ns: f32,
    sums: Vec<f64>,
    samples: u32,
    frames: u32,
}

/// `Timer::ring` states.
const FREE: u8 = 0;
const WRITTEN: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

impl Timer {
    const REPORT: u32 = 240;

    fn new(device: &wgpu::Device, queue: &wgpu::Queue, count: u32) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("rt timer"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt timestamps"),
            size: count as u64 * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ring = (0..RING)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rt timestamps readback"),
                    size: count as u64 * 8,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, std::sync::Arc::new(std::sync::atomic::AtomicU8::new(FREE)))
            })
            .collect();
        Some(Self {
            queries,
            count,
            resolve,
            ring,
            next: 0,
            period_ns: queue.get_timestamp_period(),
            sums: vec![0.0; count as usize - 1],
            samples: 0,
            frames: 0,
        })
    }

    /// Before this frame's work: the readbacks the last frames wrote are
    /// mapped, those mapped are read. Every [`Timer::REPORT`] frames, the
    /// mean time of each step (ms) and the frames they cover.
    fn harvest(&mut self) -> Option<(Vec<f64>, u32)> {
        use std::sync::atomic::Ordering;
        let n = self.count as usize;
        for (buffer, state) in &self.ring {
            match state.load(Ordering::Acquire) {
                WRITTEN => {
                    state.store(MAPPING, Ordering::Release);
                    let state = state.clone();
                    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { MAPPED } else { FREE }, Ordering::Release);
                    });
                }
                MAPPED => {
                    if let Ok(data) = buffer.slice(..).get_mapped_range() {
                        let mut t = vec![0u64; n];
                        bytemuck::cast_slice_mut::<u64, u8>(&mut t).copy_from_slice(&data[..n * 8]);
                        if t.windows(2).all(|w| w[1] >= w[0]) {
                            let ms = |a: u64, b: u64| (b - a) as f64 * self.period_ns as f64 / 1e6;
                            for (k, sum) in self.sums.iter_mut().enumerate() {
                                *sum += ms(t[k], t[k + 1]);
                            }
                            self.samples += 1;
                        }
                    }
                    buffer.unmap();
                    state.store(FREE, Ordering::Release);
                }
                _ => {}
            }
        }
        self.frames += 1;
        if self.frames < Self::REPORT || self.samples == 0 {
            return None;
        }
        let k = self.samples as f64;
        let means = self.sums.iter().map(|s| s / k).collect();
        let samples = self.samples;
        self.sums.iter_mut().for_each(|s| *s = 0.0);
        self.samples = 0;
        self.frames = 0;
        Some((means, samples))
    }

    /// After this frame's work: its timestamps to a free readback.
    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        use std::sync::atomic::Ordering;
        encoder.resolve_query_set(&self.queries, 0..self.count, &self.resolve, 0);
        let (buffer, state) = &self.ring[self.next];
        if state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.resolve, 0, buffer, 0, self.count as u64 * 8);
            state.store(WRITTEN, Ordering::Release);
            self.next = (self.next + 1) % RING;
        }
    }
}

impl RtShadows {
    /// None when the experiment is off or the device has no ray queries.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, textures: &wgpu::BindGroupLayout) -> Option<Self> {
        let cull = mode()?;
        if !device.features().contains(FEATURES) {
            log::warn!("AURORA_RT_SHADOWS: the device has no ray queries, the shadow maps stay");
            return None;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, include_str!("shaders/rt_shadows.wgsl")]).into()),
        });
        let scene_entries: Vec<wgpu::BindGroupLayoutEntry> = (0..8).map(|b| storage_entry(b, true)).collect();
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt scene"),
            entries: &scene_entries,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt shadows"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ParamsU>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: RESULT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt shadows"),
            bind_group_layouts: &[Some(&layout), Some(&scene_layout), Some(textures)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt shadows"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_trace"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows"),
            size: std::mem::size_of::<ParamsU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("AURORA_RT_SHADOWS: ray-traced sun shadows on ({cull:?} faces culled)");
        Some(Self {
            layout,
            pipeline,
            buffer,
            cull,
            bottoms: HashMap::new(),
            arena: None,
            tlas: None,
            built_for: Vec::new(),
            frame: 0,
            stats: RtStats::default(),
            timer: Timer::new(device, queue, 4),
            poser: Poser::new(device, &scene_layout),
            reflect: reflections().then(|| Reflect::new(device, queue, &scene_layout, textures)),
            gi: gi().then(|| Gi::new(device, queue, &scene_layout, textures)),
            lights: local_lights().then(|| Lights::new(device, queue, &scene_layout, textures)),
            posed_live: false,
            scene_layout,
            warned: false,
        })
    }
}

impl Poser {
    fn new(device: &wgpu::Device, scene_layout: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows pose"),
            source: wgpu::ShaderSource::Wgsl(source(&[include_str!("shaders/rt_pose.wgsl")]).into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt shadows pose"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PoseParams>() as u64),
                    },
                    count: None,
                },
                storage_entry(1, true),
                storage_entry(2, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt shadows pose"),
            bind_group_layouts: &[Some(&layout), Some(scene_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt shadows pose"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_pose"),
            compilation_options: Default::default(),
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows pose"),
            size: std::mem::size_of::<PoseParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            pipeline,
            params,
            faces: None,
            vertices: None,
            records: posed_records_buffer(device, 4096),
            record_list: Vec::new(),
            levels: HashMap::new(),
            tlas: None,
            placed: Vec::new(),
            offsets: Vec::new(),
            table: Vec::new(),
            frame: 0,
        }
    }

    /// Poses the faces of the skeletons built again this frame into the
    /// posed buffer (up to [`POSED_VERTICES`]): the vertices posed.
    fn pose(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene_group: &wgpu::BindGroup,
        faces: &Faces,
    ) -> u32 {
        self.frame += 1;
        self.table.clear();
        self.offsets.clear();
        let mut total = 0u64;
        for (g, group) in faces.groups.iter().enumerate() {
            if !group.rebuild || total + group.vertices as u64 > POSED_VERTICES {
                continue;
            }
            let run = faces
                .posed
                .get(group.first as usize..(group.first + group.count) as usize)
                .unwrap_or(&[]);
            self.offsets.push((g, total as u32));
            for f in run {
                self.table.push(PoseFace {
                    record: f.record,
                    base_vertex: f.base_vertex,
                    first_index: f.first_index,
                    index_count: f.index_count,
                    out_offset: total as u32,
                    _pad: [0; 3],
                });
                total += f.index_count as u64;
            }
        }
        if total == 0 {
            return 0;
        }
        let table_bytes = (self.table.len() * std::mem::size_of::<PoseFace>()) as u64;
        if self.faces.as_ref().is_none_or(|b| b.size() < table_bytes) {
            self.faces = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt shadows posed faces"),
                size: table_bytes.next_power_of_two().max(4096),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let bytes = total.next_power_of_two().max(3 << 10) * POSED_BYTES;
        if self.vertices.as_ref().is_none_or(|b| b.size() < bytes) {
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt shadows posed vertices"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                mapped_at_creation: false,
            }));
        }
        let (Some(table), Some(posed)) = (self.faces.as_ref(), self.vertices.as_ref()) else {
            self.offsets.clear();
            return 0;
        };
        queue.write_buffer(table, 0, bytemuck::cast_slice(&self.table));
        let workgroups = (total as u32).div_ceil(64);
        let x = workgroups.min(65_535);
        let y = workgroups.div_ceil(x);
        let params = PoseParams {
            counts: [self.table.len() as u32, total as u32, 0, x * 64],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt shadows pose"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: table.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: posed.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rt shadows pose"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_bind_group(1, scene_group, &[]);
        pass.dispatch_workgroups(x, y, 1);
        total as u32
    }

    /// Builds the bottom levels of the skeletons posed this frame, and the
    /// top level (with its faces' records) when one was built or the
    /// skeletons changed: the levels built, and the skeletons the top level
    /// holds.
    fn build(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, faces: &Faces) -> (usize, usize) {
        let frame = self.frame;
        // a level made for each skeleton built with faces its level was
        // not made for
        for &(g, _) in &self.offsets {
            let group = faces.groups[g];
            if self.levels.get(&group.key).is_some_and(|l| l.faces == group.faces) {
                continue;
            }
            let run = faces
                .posed
                .get(group.first as usize..(group.first + group.count) as usize)
                .unwrap_or(&[]);
            let sizes: Vec<_> = run.iter().map(|f| posed_size(f.index_count, f.opaque)).collect();
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("rt shadows skeleton"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: sizes.clone(),
                },
            );
            self.levels.insert(
                group.key,
                Level {
                    blas,
                    faces: group.faces,
                    records: run.iter().map(|f| f.record).collect(),
                    sizes,
                    used: frame,
                },
            );
        }
        for group in &faces.groups {
            if let Some(level) = self.levels.get_mut(&group.key) {
                level.used = frame;
            }
        }
        self.levels.retain(|_, l| l.used == frame);
        let placed: Vec<u32> = faces.groups.iter().map(|g| g.key).filter(|k| self.levels.contains_key(k)).collect();
        if self.offsets.is_empty() && placed == self.placed {
            return (0, placed.len());
        }
        let wanted = placed.len().max(1).next_power_of_two().max(64);
        if self.tlas.as_ref().is_none_or(|t| t.get().len() < wanted) {
            self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("rt shadows skeletons"),
                max_instances: wanted as u32,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }));
        }
        // the faces' records, each skeleton's where its instance says
        self.record_list.clear();
        let mut starts = Vec::with_capacity(placed.len());
        for key in &placed {
            starts.push(self.record_list.len() as u32);
            if let Some(level) = self.levels.get(key) {
                self.record_list.extend_from_slice(&level.records);
            }
        }
        let bytes = (self.record_list.len().max(1) * 4) as u64;
        if self.records.size() < bytes {
            self.records = posed_records_buffer(device, bytes.next_power_of_two());
        }
        queue.write_buffer(&self.records, 0, bytemuck::cast_slice(&self.record_list));
        let (Some(tlas), Some(buffer)) = (self.tlas.as_mut(), self.vertices.as_ref()) else {
            return (0, 0);
        };
        const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        {
            let slots = tlas.get_mut_slice(0..wanted).expect("top level of `wanted` slots");
            for (n, slot) in slots.iter_mut().enumerate() {
                *slot = placed
                    .get(n)
                    .and_then(|k| self.levels.get(k))
                    .map(|l| wgpu::TlasInstance::new(&l.blas, IDENTITY, starts[n] & 0xff_ffff, 0xFF));
            }
        }
        let entries: Vec<wgpu::BlasBuildEntry<'_>> = self
            .offsets
            .iter()
            .filter_map(|&(g, offset)| {
                let level = self.levels.get(&faces.groups[g].key)?;
                let mut first = offset;
                let geometries = level
                    .sizes
                    .iter()
                    .map(|size| {
                        let geometry = wgpu::BlasTriangleGeometry {
                            size,
                            vertex_buffer: buffer,
                            first_vertex: first,
                            vertex_stride: POSED_BYTES,
                            index_buffer: None,
                            first_index: None,
                            transform_buffer: None,
                            transform_buffer_offset: None,
                        };
                        first += size.vertex_count;
                        geometry
                    })
                    .collect();
                Some(wgpu::BlasBuildEntry {
                    blas: &level.blas,
                    geometry: wgpu::BlasGeometries::TriangleGeometries(geometries),
                })
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));
        let built = entries.len();
        drop(entries);
        self.placed = placed;
        (built, self.placed.len())
    }

    /// The posed top level to trace this frame, if it holds a skeleton.
    fn traced(&self) -> Option<&wgpu::Tlas> {
        self.tlas.as_ref().filter(|_| !self.placed.is_empty())
    }
}

fn posed_records_buffer(device: &wgpu::Device, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rt shadows posed records"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A ray-traced shader's source: the enables, rt_common.wgsl, then `parts`
/// in order (the shared rt_view.wgsl and rt_light.wgsl, then its own).
fn source(parts: &[&str]) -> String {
    let mut src = format!(
        "enable wgpu_ray_query;\nenable wgpu_binding_array;\n{}\n",
        include_str!("shaders/rt_common.wgsl")
    );
    for part in parts {
        src.push_str(part);
        src.push('\n');
    }
    src
}

const RT_VIEW: &str = include_str!("shaders/rt_view.wgsl");
const RT_LIGHT: &str = include_str!("shaders/rt_light.wgsl");

/// A posed face's geometry: `vertices` unindexed, three a triangle; an
/// alpha-masked face is tested at its hits.
fn posed_size(vertices: u32, opaque: bool) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: vertices,
        index_format: None,
        index_count: None,
        flags: geometry_flags(opaque),
    }
}

fn geometry_flags(opaque: bool) -> wgpu::AccelerationStructureGeometryFlags {
    if opaque {
        wgpu::AccelerationStructureGeometryFlags::OPAQUE
    } else {
        wgpu::AccelerationStructureGeometryFlags::empty()
    }
}

impl Targets {
    /// The mask, when the device traces (AURORA_RT_SHADOWS and ray queries).
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Option<Self> {
        if mode().is_none() || !device.features().contains(FEATURES) {
            return None;
        }
        let size = (width.max(1), height.max(1));
        let result = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("rt shadows"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: RESULT_FORMAT,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let reflect = reflections().then(|| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("rt reflections"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: REFLECT_FORMAT,
                    usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        });
        let gi = gi().then(|| {
            let half = (size.0.div_ceil(2), size.1.div_ceil(2));
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &'static str, (width, height): (u32, u32), format: wgpu::TextureFormat| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            GiTargets {
                half,
                color: [view("rt gi history", half, GI_FORMAT), view("rt gi history", half, GI_FORMAT)],
                dist: [
                    view("rt gi distance", half, GI_DIST_FORMAT),
                    view("rt gi distance", half, GI_DIST_FORMAT),
                ],
                result: view("rt gi", size, GI_FORMAT),
            }
        });
        let lights = local_lights().then(|| {
            let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |label: &'static str, format: wgpu::TextureFormat| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            LightTargets {
                choice: [view("rt light choices", CHOICE_FORMAT), view("rt light choices", CHOICE_FORMAT)],
                share: [view("rt light share", SHARE_FORMAT), view("rt light share", SHARE_FORMAT)],
                sun: view("rt sun mask", RESULT_FORMAT),
            }
        });
        Some(Targets {
            size,
            result,
            reflect,
            gi,
            lights,
        })
    }
}

impl RtShadows {
    /// The constants of this frame's pass: the projection the depth prepass
    /// was drawn with, the eye, the light (w: whether it is up), and whether
    /// the contact mask is read.
    pub(crate) fn write(
        &self,
        queue: &wgpu::Queue,
        t: &Targets,
        clip_from_world: glam::Mat4,
        camera: glam::Vec3,
        light: glam::Vec3,
        light_up: bool,
        contact: bool,
    ) {
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = ParamsU {
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            camera: camera.extend(1.0).to_array(),
            light: light
                .normalize_or(glam::Vec3::Z)
                .extend(if light_up { 1.0 } else { 0.0 })
                .to_array(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [self.cull.ray_flags() as f32, if contact { 1.0 } else { 0.0 }, REACH, 0.0],
        };
        queue.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&u));
    }

    /// Poses the posed faces and builds their levels, builds what is missing
    /// of the bottom levels (within a budget) and the scene's top level when
    /// a face was added or moved, and traces the mask from `depth` (and `contact`, the contact shadows'
    /// mask, or a black texture).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
        faces: &Faces,
    ) {
        let (vertices, indices, instances) = (&scene.vertices, &scene.indices, faces.placed.as_slice());
        self.frame += 1;
        let frame = self.frame;
        if let Some(timer) = &mut self.timer {
            if let Some((ms, frames)) = timer.harvest() {
                let st = &self.stats;
                log::info!(
                    "AURORA_RT_SHADOWS: pose {:.3} ms, build {:.3} ms, trace {:.3} ms (GPU, mean of {frames} frames); {} placed faces, {} bottom levels ({} built last frame, {} triangles), {} waiting; {} skeletons ({} built last frame: {} faces, {} vertices)",
                    ms[0],
                    ms[1],
                    ms[2],
                    st.instances,
                    st.bottoms,
                    st.built,
                    st.built_triangles,
                    st.waiting,
                    st.skeletons,
                    st.skeletons_built,
                    st.posed_faces,
                    st.posed_vertices,
                );
            }
            // the pose's start: an empty pass's timestamp, as the pose may
            // not run and wgpu times no build of its own
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows start"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            }));
        }
        let pose_group = self.scene_group(device, scene);
        let posed = self.poser.pose(device, queue, encoder, &pose_group, faces);
        if let Some(timer) = &self.timer {
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows posed"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: None,
                }),
            }));
        }
        let (skeletons_built, skeletons) = self.poser.build(device, queue, encoder, faces);
        self.posed_live = skeletons > 0;
        // the trace reads the posed top level only when it holds a skeleton
        queue.write_buffer(
            &self.buffer,
            (std::mem::offset_of!(ParamsU, mode) + 12) as u64,
            bytemuck::bytes_of(&if skeletons > 0 { 1.0f32 } else { 0.0 }),
        );
        let posed_faces = if posed > 0 { self.poser.table.len() } else { 0 };
        self.stats.posed_faces = posed_faces;
        self.stats.posed_vertices = posed as u64;
        self.stats.skeletons = skeletons;
        self.stats.skeletons_built = skeletons_built;
        let same_arena = self.arena.as_ref().is_some_and(|(v, i)| v == vertices && i == indices);
        if !same_arena {
            self.bottoms.clear();
            self.arena = Some((vertices.clone(), indices.clone()));
        }
        let arena_vertices = vertices.size() / VERTEX_BYTES;
        let arena_indices = indices.size() / 2;
        // the bottom levels to build: new geometries, nearest first in the
        // list's order, until the frame's budget
        let mut new: Vec<(Key, wgpu::BlasTriangleGeometrySizeDescriptor)> = Vec::new();
        let mut triangles = 0u64;
        let mut waiting = 0usize;
        for i in instances {
            let key = Key {
                base_vertex: i.base_vertex,
                first_index: i.first_index,
                index_count: i.index_count,
                opaque: i.opaque,
            };
            if let Some(b) = self.bottoms.get_mut(&key) {
                b.used = frame;
                continue;
            }
            if new.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let first = i.base_vertex.max(0) as u64;
            if first >= arena_vertices || i.first_index as u64 + i.index_count as u64 > arena_indices {
                continue;
            }
            let tris = i.index_count as u64 / 3;
            if triangles > 0 && triangles + tris > BUILD_TRIANGLES {
                waiting += 1;
                continue;
            }
            triangles += tris;
            new.push((
                key,
                wgpu::BlasTriangleGeometrySizeDescriptor {
                    vertex_format: wgpu::VertexFormat::Float32x3,
                    vertex_count: (arena_vertices - first).min(FACE_VERTICES) as u32,
                    index_format: Some(wgpu::IndexFormat::Uint16),
                    index_count: Some(i.index_count),
                    flags: geometry_flags(i.opaque),
                },
            ));
        }
        let created: Vec<wgpu::Blas> = new
            .iter()
            .map(|(_, size)| {
                device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("rt shadows face"),
                        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: vec![size.clone()],
                    },
                )
            })
            .collect();
        // the levels unused for a while go
        self.bottoms.retain(|_, b| frame - b.used <= KEEP_FRAMES);
        // nothing new, nothing moved: the top level stands
        let wanted = instances.len().max(1).next_power_of_two().max(1024);
        let same = same_arena
            && new.is_empty()
            && waiting == 0
            && self.tlas.as_ref().is_some_and(|t| t.get().len() >= wanted)
            && self.built_for.as_slice() == instances;
        if same {
            self.stats.built = 0;
            self.stats.built_triangles = 0;
            self.trace(device, encoder, t, depth, contact, scene);
            return;
        }
        // the top level: grown in powers of two
        if self.tlas.as_ref().is_none_or(|t| t.get().len() < wanted) {
            self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("rt shadows scene"),
                max_instances: wanted as u32,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }));
        }
        let Some(tlas) = self.tlas.as_mut() else {
            return;
        };
        let mut placed = 0usize;
        {
            let slots = tlas.get_mut_slice(0..wanted).expect("top level of `wanted` slots");
            for i in instances {
                let key = Key {
                    base_vertex: i.base_vertex,
                    first_index: i.first_index,
                    index_count: i.index_count,
                    opaque: i.opaque,
                };
                let blas = match self.bottoms.get(&key) {
                    Some(b) => &b.blas,
                    None => match new.iter().position(|(k, _)| *k == key) {
                        Some(n) => &created[n],
                        None => continue,
                    },
                };
                slots[placed] = Some(wgpu::TlasInstance::new(blas, row_major(&i.model), i.record, 0xFF));
                placed += 1;
            }
            for slot in &mut slots[placed..] {
                *slot = None;
            }
        }
        let entries: Vec<wgpu::BlasBuildEntry<'_>> = new
            .iter()
            .zip(&created)
            .map(|((key, size), blas)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                    size,
                    vertex_buffer: vertices,
                    first_vertex: key.base_vertex.max(0) as u32,
                    vertex_stride: VERTEX_BYTES,
                    index_buffer: Some(indices),
                    first_index: Some(key.first_index),
                    transform_buffer: None,
                    transform_buffer_offset: None,
                }]),
            })
            .collect();
        encoder.build_acceleration_structures(entries.iter(), std::iter::once(&*tlas));
        drop(entries);
        self.stats = RtStats {
            instances: placed,
            bottoms: self.bottoms.len() + created.len(),
            built: created.len(),
            built_triangles: triangles,
            waiting,
            posed_faces,
            posed_vertices: posed as u64,
            skeletons,
            skeletons_built,
        };
        for ((key, _), blas) in new.into_iter().zip(created) {
            self.bottoms.insert(key, Bottom { blas, used: frame });
        }
        // a face left out (its level waiting) builds again next frame
        self.built_for.clear();
        if waiting == 0 {
            self.built_for.extend_from_slice(instances);
        }
        self.trace(device, encoder, t, depth, contact, scene);
    }

    /// The trace of the mask through the top level as it stands.
    fn trace(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        // after the build: the posed records' buffer may have grown
        let scene_group = self.scene_group(device, scene);
        let Some(tlas) = self.tlas.as_ref() else {
            return;
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt shadows"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(contact),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    // with the traced lights, their filter writes the mask
                    resource: wgpu::BindingResource::TextureView(t.lights.as_ref().map_or(&t.result, |l| &l.sun)),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows"),
                timestamp_writes: self.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(2),
                    end_of_pass_write_index: Some(3),
                }),
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut self.timer {
            timer.resolve(encoder);
        }
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn tlas_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure { vertex_return: false },
        count: None,
    }
}

impl Reflect {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt reflections"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_reflect.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt reflections"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<ReflectU>() as u64),
                    },
                    count: None,
                },
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: REFLECT_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt reflections"),
            bind_group_layouts: &[Some(&layout), Some(scene_layout), Some(textures)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
            label: Some("rt reflections"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cs_reflect"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt reflections"),
            size: std::mem::size_of::<ReflectU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("AURORA_RT_REFLECTIONS: ray-traced reflections on, in SSR's place");
        Reflect {
            layout,
            pipeline,
            buffer,
            timer: Timer::new(device, queue, 2),
        }
    }
}

fn storage_texture_entry(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
        count: None,
    }
}

fn uniform_entry<T>() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<T>() as u64),
        },
        count: None,
    }
}

fn view(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

impl Gi {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt gi"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_gi.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt gi trace"),
            entries: &[
                uniform_entry::<GiU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(2, unfiltered),
                texture_entry(3, unfiltered),
                tlas_entry(4),
                tlas_entry(5),
                storage_texture_entry(6, GI_FORMAT),
                storage_texture_entry(7, GI_DIST_FORMAT),
                texture_entry(8, unfiltered),
            ],
        });
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt gi filter"),
            entries: &[
                uniform_entry::<GiU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                texture_entry(9, unfiltered),
                texture_entry(10, unfiltered),
                storage_texture_entry(11, GI_FORMAT),
            ],
        });
        let pipeline = |label: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], entry: &'static str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let trace = pipeline("rt gi trace", &[Some(&trace_layout), Some(scene_layout), Some(textures)], "cs_gi");
        let filter = pipeline("rt gi filter", &[Some(&filter_layout)], "cs_gi_filter");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt gi"),
            size: std::mem::size_of::<GiU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("AURORA_RT_GI: ray-traced indirect light on, in the screen-space indirect light's place");
        Gi {
            trace_layout,
            filter_layout,
            trace,
            filter,
            buffer,
            timer: Timer::new(device, queue, 3),
            last: 0,
            history_for: None,
            prev_clip_from_world: glam::Mat4::IDENTITY,
            prev_camera: glam::Vec3::ZERO,
            frame: 0,
        }
    }
}

impl Lights {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, scene_layout: &wgpu::BindGroupLayout, textures: &wgpu::BindGroupLayout) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt lights"),
            source: wgpu::ShaderSource::Wgsl(source(&[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_lights.wgsl")]).into()),
        });
        let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
        let lights_entry = storage_entry(2, true);
        let trace_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights trace"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(1, wgpu::TextureSampleType::Depth),
                lights_entry,
                texture_entry(3, unfiltered),
                storage_texture_entry(4, CHOICE_FORMAT),
                tlas_entry(5),
                tlas_entry(6),
                texture_entry(7, unfiltered),
                storage_texture_entry(8, SHARE_FORMAT),
            ],
        });
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rt lights filter"),
            entries: &[
                uniform_entry::<LightsU>(),
                texture_entry(9, unfiltered),
                texture_entry(10, unfiltered),
                storage_texture_entry(11, RESULT_FORMAT),
            ],
        });
        let pipeline = |label: &'static str, layouts: &[Option<&wgpu::BindGroupLayout>], entry: &'static str| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_compute_pipeline_cached(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        // the trace reads the scene's tables and textures at each candidate
        // hit (rt_common.wgsl `stops`)
        let trace = pipeline(
            "rt lights trace",
            &[Some(&trace_layout), Some(scene_layout), Some(textures)],
            "cs_lights",
        );
        let filter = pipeline("rt lights filter", &[Some(&filter_layout)], "cs_lights_filter");
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt lights"),
            size: std::mem::size_of::<LightsU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt lights list"),
            size: (std::mem::size_of::<TracedLightU>() * MAX_TRACED_LIGHTS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        log::info!("AURORA_RT_LIGHTS: ray-traced point light shadows on");
        Lights {
            trace_layout,
            filter_layout,
            trace,
            filter,
            buffer,
            lights,
            listed: Vec::new(),
            timer: Timer::new(device, queue, 3),
            last: 0,
            history_for: None,
            prev_clip_from_world: glam::Mat4::IDENTITY,
            prev_camera: glam::Vec3::ZERO,
            frame: 0,
        }
    }
}

/// The frame's lights as rt_lights.wgsl reads them, in the frame uniform's
/// order.
fn traced_lights(lights: &[crate::types::PointLight]) -> Vec<TracedLightU> {
    lights
        .iter()
        .take(MAX_TRACED_LIGHTS)
        .map(|l| TracedLightU {
            pos_radius: l.position.extend(l.radius).to_array(),
            color_falloff: l.color.extend(l.falloff).to_array(),
            source: [l.source_radius, if l.projector.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0],
        })
        .collect()
}

impl RtShadows {
    /// Whether the point lights' shadows are traced: the frame then binds
    /// the mask with their share (common.wgsl `local_share`).
    pub(crate) fn traces_lights(&self) -> bool {
        self.lights.is_some()
    }

    /// Traces the point lights' share into the mask's `.b`, after the sun's
    /// pass (which wrote the targets' sun mask) and before the scene is lit.
    /// Runs on every traced frame while the traced lights are on: the mask
    /// the frame binds is its.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_lights(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        clip_from_world: glam::Mat4,
        camera: glam::Vec3,
        point_lights: &[crate::types::PointLight],
        depth: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(l), Some(tlas), Some(lt)) = (self.lights.as_mut(), self.tlas.as_ref(), t.lights.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut l.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "AURORA_RT_LIGHTS: trace {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {} lights listed)",
                ms[0],
                ms[1],
                l.listed.len()
            );
        }
        let listed = traced_lights(point_lights);
        let same_lights = listed == l.listed;
        if !listed.is_empty() {
            queue.write_buffer(&l.lights, 0, bytemuck::cast_slice(&listed));
        }
        l.listed = listed;
        let (read, write) = (l.last, 1 - l.last);
        let valid = l.history_for == Some(t.size);
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = LightsU {
            world_from_clip: clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: l.prev_clip_from_world.to_cols_array_2d(),
            camera: camera.extend(1.0).to_array(),
            prev_camera: l.prev_camera.extend(1.0).to_array(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [
                l.listed.len() as f32,
                if self.posed_live { 1.0 } else { 0.0 },
                (l.frame % (1 << 20)) as f32,
                match (valid, same_lights) {
                    (false, _) => 0.0,
                    (true, false) => 1.0,
                    (true, true) => 3.0,
                },
            ],
        };
        queue.write_buffer(&l.buffer, 0, bytemuck::bytes_of(&u));
        let uniform = wgpu::BindGroupEntry {
            binding: 0,
            resource: l.buffer.as_entire_binding(),
        };
        let trace_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights trace"),
            layout: &l.trace_layout,
            entries: &[
                uniform.clone(),
                view(1, depth),
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: l.lights.as_entire_binding(),
                },
                view(3, &lt.choice[read]),
                view(4, &lt.choice[write]),
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                view(7, &lt.share[read]),
                view(8, &lt.share[write]),
            ],
        });
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt lights filter"),
            layout: &l.filter_layout,
            entries: &[uniform, view(9, &lt.share[write]), view(10, &lt.sun), view(11, &t.result)],
        });
        let groups = (t.size.0.div_ceil(8), t.size.1.div_ceil(8));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights trace"),
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&l.trace);
            pass.set_bind_group(0, &trace_group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt lights filter"),
                timestamp_writes: l.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: Some(2),
                }),
            });
            pass.set_pipeline(&l.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        if let Some(timer) = &mut l.timer {
            timer.resolve(encoder);
        }
        l.last = write;
        l.history_for = Some(t.size);
        l.prev_clip_from_world = clip_from_world;
        l.prev_camera = camera;
        l.frame = l.frame.wrapping_add(1);
    }
}

impl RtShadows {
    /// Whether the traced indirect light is on: the shading reads it this
    /// frame when the frame traces.
    pub(crate) fn lights_indirectly(&self) -> bool {
        self.gi.is_some()
    }

    /// Traces this frame's indirect light into the targets' GI result,
    /// after the shadows' pass built this frame's levels and before the
    /// scene is lit; the view's hits read `scene_copy`, last frame's.
    /// `intensity` scales it as it scales GTAO's (Indirect light intensity).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_gi(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        intensity: f32,
        depth: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(g), Some(tlas), Some(gt)) = (self.gi.as_mut(), self.tlas.as_ref(), t.gi.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut g.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!(
                "AURORA_RT_GI: trace {:.3} ms, filter {:.3} ms (GPU, mean of {frames} frames, {}x{} rays)",
                ms[0],
                ms[1],
                gt.half.0,
                gt.half.1
            );
        }
        let (read, write) = (g.last, 1 - g.last);
        let valid = g.history_for == Some(t.size);
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = GiU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            prev_clip_from_world: g.prev_clip_from_world.to_cols_array_2d(),
            camera: light.camera.extend(1.0).to_array(),
            prev_camera: g.prev_camera.extend(1.0).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            half: [gt.half.0 as f32, gt.half.1 as f32, intensity, 0.0],
            mode: [
                GI_REACH,
                if self.posed_live { 1.0 } else { 0.0 },
                (g.frame % (1 << 20)) as f32,
                if valid { 1.0 } else { 0.0 },
            ],
        };
        queue.write_buffer(&g.buffer, 0, bytemuck::bytes_of(&u));
        let trace_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi trace"),
            layout: &g.trace_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(2, &gt.color[read]),
                view(3, &gt.dist[read]),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                view(6, &gt.color[write]),
                view(7, &gt.dist[write]),
                view(8, scene_copy),
            ],
        });
        let filter_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt gi filter"),
            layout: &g.filter_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: g.buffer.as_entire_binding(),
                },
                view(1, depth),
                view(9, &gt.color[write]),
                view(10, &gt.dist[write]),
                view(11, &gt.result),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi trace"),
                timestamp_writes: g.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&g.trace);
            pass.set_bind_group(0, &trace_group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(gt.half.0.div_ceil(8), gt.half.1.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt gi filter"),
                timestamp_writes: g.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: Some(2),
                }),
            });
            pass.set_pipeline(&g.filter);
            pass.set_bind_group(0, &filter_group, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut g.timer {
            timer.resolve(encoder);
        }
        g.last = write;
        g.history_for = Some(t.size);
        g.prev_clip_from_world = light.clip_from_world;
        g.prev_camera = light.camera;
        g.frame = g.frame.wrapping_add(1);
    }
}

impl RtShadows {
    /// Whether the traced reflections can take SSR's place this frame:
    /// they are on and the frame traced.
    pub(crate) fn reflects(&self) -> bool {
        self.reflect.is_some() && self.tlas.is_some()
    }

    /// Traces the reflections of the glossy pixels of `gbuf` into the
    /// targets' reflection texture, after the shadows' pass built this
    /// frame's levels; the view's hits read `scene_copy`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_reflections(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        light: &TraceLight,
        depth: &wgpu::TextureView,
        gbuf: &wgpu::TextureView,
        scene_copy: &wgpu::TextureView,
        scene: &crate::packet::SceneGpu,
    ) {
        let scene_group = self.scene_group(device, scene);
        let (Some(r), Some(tlas), Some(out)) = (self.reflect.as_mut(), self.tlas.as_ref(), t.reflect.as_ref()) else {
            return;
        };
        if let Some(timer) = &mut r.timer
            && let Some((ms, frames)) = timer.harvest()
        {
            log::info!("AURORA_RT_REFLECTIONS: trace {:.3} ms (GPU, mean of {frames} frames)", ms[0]);
        }
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let u = ReflectU {
            clip_from_world: light.clip_from_world.to_cols_array_2d(),
            world_from_clip: light.clip_from_world.inverse().to_cols_array_2d(),
            camera: light.camera.extend(1.0).to_array(),
            lit: light.lit(),
            screen: [w, h, 1.0 / w, 1.0 / h],
            mode: [REACH, if self.posed_live { 1.0 } else { 0.0 }, 0.0, 0.0],
        };
        queue.write_buffer(&r.buffer, 0, bytemuck::bytes_of(&u));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt reflections"),
            layout: &r.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: r.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(gbuf),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(scene_copy),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::AccelerationStructure(tlas),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    // without posed faces the scene's stands in, unread
                    resource: wgpu::BindingResource::AccelerationStructure(self.poser.traced().unwrap_or(tlas)),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(out),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt reflections"),
                timestamp_writes: r.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
            });
            pass.set_pipeline(&r.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &scene_group, &[]);
            pass.set_bind_group(2, &scene.textures, &[]);
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut r.timer {
            timer.resolve(encoder);
        }
    }
}

impl RtShadows {
    /// The scene's tables for the ray-traced passes (rt_common.wgsl group
    /// 1), each bound up to the device's limit.
    fn scene_group(&mut self, device: &wgpu::Device, scene: &crate::packet::SceneGpu) -> wgpu::BindGroup {
        let max = device.limits().max_storage_buffer_binding_size;
        let buffers = [
            &scene.records,
            &scene.palettes,
            &scene.skin_binds,
            &scene.vertices,
            &scene.skin,
            &scene.indices,
            &scene.faces,
            &self.poser.records,
        ];
        if !self.warned && buffers.iter().any(|b| b.size() > max) {
            log::warn!("AURORA_RT_SHADOWS: a scene buffer is larger than the device binds; what lies past it is not traced right");
            self.warned = true;
        }
        let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
            .iter()
            .enumerate()
            .map(|(binding, b)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: b,
                    offset: 0,
                    size: wgpu::BufferSize::new(b.size().min(max)),
                }),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rt scene"),
            layout: &self.scene_layout,
            entries: &entries,
        })
    }
}

/// A column-major model matrix as the top level's 3×4 row-major transform.
fn row_major(m: &[[f32; 4]; 4]) -> [f32; 12] {
    std::array::from_fn(|i| m[i % 4][i / 4])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rt_shadows_shaders_are_valid() {
        for (name, parts) in [
            ("rt_shadows", &[RT_VIEW, include_str!("shaders/rt_shadows.wgsl")][..]),
            ("rt_pose", &[include_str!("shaders/rt_pose.wgsl")][..]),
            ("rt_reflect", &[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_reflect.wgsl")][..]),
            ("rt_gi", &[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_gi.wgsl")][..]),
            ("rt_lights", &[RT_VIEW, RT_LIGHT, include_str!("shaders/rt_lights.wgsl")][..]),
        ] {
            let src = source(parts);
            let module = match naga::front::wgsl::parse_str(&src) {
                Ok(m) => m,
                Err(e) => panic!("{name}: {}", e.emit_to_string(&src)),
            };
            let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
            if let Err(e) = v.validate(&module) {
                panic!("{name}: {}", e.emit_to_string(&src));
            }
        }
    }

    fn candidate(key: u32, record: u32, x: f32) -> Candidate {
        let mut model = glam::Mat4::IDENTITY.to_cols_array_2d();
        model[3][0] = x;
        Candidate {
            key,
            face: Posed {
                record,
                base_vertex: 0,
                first_index: 0,
                index_count: 300,
                opaque: true,
            },
            model,
            origin: [x, 0.0, 0.0],
        }
    }

    fn choose(tracker: &mut PoseTracker, candidates: Vec<Candidate>) -> Vec<(u32, bool)> {
        tracker.frame += 1;
        tracker.scratch = candidates;
        let mut out = Faces::default();
        choose_rebuilds(&[], glam::Vec3::ZERO, tracker, &mut out);
        out.groups.iter().map(|g| (g.key, g.rebuild)).collect()
    }

    #[test]
    fn rt_shadows_a_still_skeleton_keeps_its_level() {
        let mut t = PoseTracker::default();
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.0)]), vec![(0, true)]);
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.0)]), vec![(0, false)]);
        // less than the ray's start offset at 5 m (0.02 m): kept
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.015)]), vec![(0, false)]);
        // further: built again
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.05)]), vec![(0, true)]);
        // another face set: built again even when still
        assert_eq!(choose(&mut t, vec![candidate(0, 1, 5.05), candidate(0, 2, 5.05)]), vec![(0, true)]);
    }

    #[test]
    fn rt_shadows_rebuilds_go_nearest_first_within_the_budget() {
        let mut t = PoseTracker::default();
        let big = |key: u32, x: f32| {
            let mut c = candidate(key, key, x);
            c.face.index_count = (REBUILD_TRIANGLES * 3 / 2) as u32;
            c
        };
        let groups = choose(&mut t, vec![big(1, 30.0), big(2, 3.0), big(3, 10.0)]);
        // the nearest two fit, the farthest waits for a later frame
        assert_eq!(groups, vec![(1, false), (2, true), (3, true)]);
        let groups = choose(&mut t, vec![big(1, 30.0), big(2, 3.0), big(3, 10.0)]);
        assert_eq!(groups, vec![(1, true), (2, false), (3, false)]);
    }

    #[test]
    fn rt_shadows_drift_counts_a_turn_at_the_joint_reach() {
        let a = glam::Mat4::IDENTITY.to_cols_array_2d();
        let b = glam::Mat4::from_rotation_z(0.01).to_cols_array_2d();
        let d = drift(&a, &b);
        assert!((d - 0.01 * JOINT_REACH).abs() < 1e-3, "{d}");
        assert_eq!(drift(&a, &a), 0.0);
    }

    #[test]
    fn rt_shadows_mode_parses_the_variable() {
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("0")), None);
        assert_eq!(parse(Some("")), None);
        assert_eq!(parse(Some("1")), Some(Cull::None));
        assert_eq!(parse(Some("front")), Some(Cull::Front));
        assert_eq!(parse(Some("back")), Some(Cull::Back));
    }

    #[test]
    fn rt_shadows_row_major_keeps_the_translation_last() {
        let m = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(2.0, 3.0, 4.0),
            glam::Quat::from_rotation_z(0.5),
            glam::Vec3::new(10.0, 20.0, 30.0),
        );
        let r = row_major(&m.to_cols_array_2d());
        let p = glam::Vec3::new(0.3, -1.2, 2.5);
        let want = m.transform_point3(p);
        let got = glam::Vec3::new(
            r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3],
            r[4] * p.x + r[5] * p.y + r[6] * p.z + r[7],
            r[8] * p.x + r[9] * p.y + r[10] * p.z + r[11],
        );
        assert!(got.distance(want) < 1e-4, "{got} != {want}");
    }
}
