//! Ray-traced sun shadows of the main view, an experiment behind
//! `AURORA_RT_SHADOWS` (off by default; Vulkan devices with ray queries).
//!
//! Through wgpu's ray queries (`EXPERIMENTAL_RAY_QUERY`): a bottom level per
//! face geometry of the arena, a top level of the faces placed in the world,
//! rebuilt each frame, and one compute pass after the contact shadows that
//! traces a ray per pixel of the depth prepass towards the sun
//! (shaders/rt_shadows.wgsl). Its mask takes the contact shadows' place in
//! the frame group, the darker of the two, so the shading reads one texture
//! as before.
//!
//! What is traced: the opaque faces of objects placed in the world, the
//! terrain, and the avatars with what they wear: their faces are posed on
//! the GPU each frame (shaders/rt_pose.wgsl, the vertex shaders' skinning)
//! into one bottom level and a top level of their own, built again every
//! frame, while the scene's top level stands as long as nothing moves. Not yet: alpha-masked and
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

/// Whether to ask a device for [`FEATURES`]: the experiment is on and the
/// adapter has them.
pub fn wanted(adapter: wgpu::Features) -> bool {
    mode().is_some() && adapter.contains(FEATURES)
}

/// A face to trace: its geometry in the arena and where it stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub base_vertex: i32,
    pub first_index: u32,
    pub index_count: u32,
    pub model: [[f32; 4]; 4],
}

/// A rigged or worn face: its world positions exist only once posed, so the
/// pose pass (shaders/rt_pose.wgsl) writes them each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub record: u32,
    pub base_vertex: u32,
    pub first_index: u32,
    pub index_count: u32,
}

/// What the frame traces: faces placed by their model matrix, and faces
/// posed on the GPU.
#[derive(Clone, Debug, Default)]
pub struct Faces {
    pub placed: Vec<Instance>,
    pub posed: Vec<Posed>,
}

/// The faces to trace this frame, from the records and the culling tables
/// the main thread keeps: opaque faces only (alpha-masked and blended ones
/// are not traced yet), and no HUD.
pub(crate) fn gather(records: &RecordStore, cull: &CullTables, out: &mut Faces) {
    out.placed.clear();
    out.posed.clear();
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
        if face.object == NO_OBJECT {
            if rf & flags::TERRAIN != 0 {
                out.placed.push(Instance {
                    base_vertex: face.base_vertex,
                    first_index: face.first_index,
                    index_count,
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
            || !(face_pass == pass::OPAQUE || face_pass == pass::OPAQUE_2S)
            || rf & (flags::ALPHA_MASK | flags::ALPHA_BLEND) != 0
        {
            continue;
        }
        let worn_or_posed = object_flags::AVATAR | object_flags::RIGGED | object_flags::POSED;
        if object.flags & worn_or_posed != 0 || object.anchor != 0 || rf & (flags::SKINNED | flags::ANCHORED | flags::LIP_SYNC) != 0 {
            out.posed.push(Posed {
                record: id as u32,
                base_vertex: face.base_vertex as u32,
                first_index: face.first_index,
                index_count,
            });
        } else {
            out.placed.push(Instance {
                base_vertex: face.base_vertex,
                first_index: face.first_index,
                index_count,
                model: record.model,
            });
        }
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

/// The pose pass and what it writes: the posed vertices and the bottom
/// level built from them each frame.
struct Poser {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    params: wgpu::Buffer,
    faces: Option<wgpu::Buffer>,
    vertices: Option<wgpu::Buffer>,
    /// The bottom level and the vertices it was made for.
    blas: Option<(wgpu::Blas, u32)>,
    /// A top level of its own, of the one posed bottom level: rebuilt each
    /// frame, while the scene's stands as long as nothing moves.
    tlas: Option<wgpu::Tlas>,
    /// The vertices posed this frame (0: none, the top level is stale).
    posed: u32,
    table: Vec<PoseFace>,
    /// A buffer too big to bind was logged.
    warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    base_vertex: i32,
    first_index: u32,
    index_count: u32,
}

struct Bottom {
    blas: wgpu::Blas,
    used: u64,
}

/// The mask the shading reads in place of the contact shadows'.
pub(crate) struct Targets {
    size: (u32, u32),
    pub result: wgpu::TextureView,
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
}

pub(crate) struct RtShadows {
    layout: wgpu::BindGroupLayout,
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
}

/// Readbacks in flight for the pass's timestamps.
const RING: usize = 4;

/// The GPU time of the pose, the builds and the trace: four timestamps
/// (before the pose, after it, before the trace, after it), resolved and read back a few frames
/// later; their averages are logged every [`Timer::REPORT`] frames.
struct Timer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    ring: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::atomic::AtomicU8>)>,
    next: usize,
    period_ns: f32,
    sums: [f64; 3],
    count: u32,
    frames: u32,
}

/// Timestamps a frame writes.
const QUERIES: u32 = 4;

/// `Timer::ring` states.
const FREE: u8 = 0;
const WRITTEN: u8 = 1;
const MAPPING: u8 = 2;
const MAPPED: u8 = 3;

impl Timer {
    const REPORT: u32 = 240;

    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("rt shadows"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERIES,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows timestamps"),
            size: QUERIES as u64 * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ring = (0..RING)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rt shadows timestamps readback"),
                    size: QUERIES as u64 * 8,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, std::sync::Arc::new(std::sync::atomic::AtomicU8::new(FREE)))
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            ring,
            next: 0,
            period_ns: queue.get_timestamp_period(),
            sums: [0.0; 3],
            count: 0,
            frames: 0,
        })
    }

    /// Before this frame's work: the readbacks the last frames wrote are
    /// mapped, those mapped are read.
    fn harvest(&mut self, stats: &RtStats) {
        use std::sync::atomic::Ordering;
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
                        let mut t = [0u64; QUERIES as usize];
                        bytemuck::cast_slice_mut::<u64, u8>(&mut t).copy_from_slice(&data[..QUERIES as usize * 8]);
                        if t.windows(2).all(|w| w[1] >= w[0]) {
                            let ms = |a: u64, b: u64| (b - a) as f64 * self.period_ns as f64 / 1e6;
                            for (k, sum) in self.sums.iter_mut().enumerate() {
                                *sum += ms(t[k], t[k + 1]);
                            }
                            self.count += 1;
                        }
                    }
                    buffer.unmap();
                    state.store(FREE, Ordering::Release);
                }
                _ => {}
            }
        }
        self.frames += 1;
        if self.frames >= Self::REPORT && self.count > 0 {
            let n = self.count as f64;
            log::info!(
                "AURORA_RT_SHADOWS: pose {:.3} ms, build {:.3} ms, trace {:.3} ms (GPU, mean of {} frames); {} placed faces, {} bottom levels ({} built last frame, {} triangles), {} waiting; {} posed faces ({} vertices)",
                self.sums[0] / n,
                self.sums[1] / n,
                self.sums[2] / n,
                self.count,
                stats.instances,
                stats.bottoms,
                stats.built,
                stats.built_triangles,
                stats.waiting,
                stats.posed_faces,
                stats.posed_vertices,
            );
            self.sums = [0.0; 3];
            self.count = 0;
            self.frames = 0;
        }
    }

    /// After this frame's work: its timestamps to a free readback.
    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        use std::sync::atomic::Ordering;
        encoder.resolve_query_set(&self.queries, 0..QUERIES, &self.resolve, 0);
        let (buffer, state) = &self.ring[self.next];
        if state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.resolve, 0, buffer, 0, QUERIES as u64 * 8);
            state.store(WRITTEN, Ordering::Release);
            self.next = (self.next + 1) % RING;
        }
    }
}

impl RtShadows {
    /// None when the experiment is off or the device has no ray queries.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let cull = mode()?;
        if !device.features().contains(FEATURES) {
            log::warn!("AURORA_RT_SHADOWS: the device has no ray queries, the shadow maps stay");
            return None;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rt_shadows.wgsl").into()),
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
            bind_group_layouts: &[Some(&layout)],
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
            timer: Timer::new(device, queue),
            poser: Poser::new(device),
        })
    }
}

impl Poser {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rt shadows pose"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/rt_pose.wgsl").into()),
        });
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
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
                storage(1, true),
                storage(2, true),
                storage(3, true),
                storage(4, true),
                storage(5, true),
                storage(6, true),
                storage(7, true),
                storage(8, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rt shadows pose"),
            bind_group_layouts: &[Some(&layout)],
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
            blas: None,
            tlas: None,
            posed: 0,
            table: Vec::new(),
            warned: false,
        }
    }

    /// Poses `faces` into the posed buffer (up to [`POSED_VERTICES`]) and
    /// readies the bottom level for them: the vertices posed, 0 when none.
    fn pose(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &crate::packet::SceneGpu,
        faces: &[Posed],
    ) -> u32 {
        self.posed = 0;
        self.table.clear();
        let mut total = 0u64;
        for f in faces {
            if total + f.index_count as u64 > POSED_VERTICES {
                break;
            }
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
        if total == 0 {
            return 0;
        }
        let max = device.limits().max_storage_buffer_binding_size;
        let bound = [
            &scene.records,
            &scene.palettes,
            &scene.skin_binds,
            &scene.vertices,
            &scene.skin,
            &scene.indices,
        ];
        if bound.iter().any(|b| b.size() > max) {
            if !self.warned {
                log::warn!("AURORA_RT_SHADOWS: a scene buffer is larger than the device binds; avatars cast no traced shadow");
                self.warned = true;
            }
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
        // the bottom level is made for a power of two of triangles, and
        // built each frame for those posed
        let capacity = (total / 3).next_power_of_two().max(1024) * 3;
        if self.vertices.as_ref().is_none_or(|b| b.size() < capacity * POSED_BYTES) {
            self.vertices = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rt shadows posed vertices"),
                size: capacity * POSED_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::BLAS_INPUT,
                mapped_at_creation: false,
            }));
        }
        if self.blas.as_ref().is_none_or(|(_, n)| (*n as u64) < total) {
            let blas = device.create_blas(
                &wgpu::CreateBlasDescriptor {
                    label: Some("rt shadows posed"),
                    flags: wgpu::AccelerationStructureFlags::PREFER_FAST_BUILD,
                    update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                },
                wgpu::BlasGeometrySizeDescriptors::Triangles {
                    descriptors: vec![posed_size(capacity as u32)],
                },
            );
            self.blas = Some((blas, capacity as u32));
        }
        if self.tlas.is_none() {
            self.tlas = Some(device.create_tlas(&wgpu::CreateTlasDescriptor {
                label: Some("rt shadows posed"),
                max_instances: 1,
                flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                update_mode: wgpu::AccelerationStructureUpdateMode::Build,
            }));
        }
        let (Some(table), Some(posed)) = (self.faces.as_ref(), self.vertices.as_ref()) else {
            return 0;
        };
        queue.write_buffer(table, 0, bytemuck::cast_slice(&self.table));
        let groups = (total as u32).div_ceil(64);
        let x = groups.min(65_535);
        let y = groups.div_ceil(x);
        let record_vec4s = (std::mem::size_of::<crate::types::DrawRecord>() / 16) as u32;
        let params = PoseParams {
            counts: [self.table.len() as u32, total as u32, record_vec4s, x * 64],
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
                    resource: scene.records.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scene.palettes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: scene.skin_binds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: scene.vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: scene.skin.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: scene.indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
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
        pass.dispatch_workgroups(x, y, 1);
        self.posed = total as u32;
        self.posed
    }

    /// Builds the posed bottom level from what the pose wrote, and its top
    /// level.
    fn build(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (true, Some((blas, _)), Some(buffer), Some(tlas)) = (self.posed > 0, &self.blas, &self.vertices, self.tlas.as_mut()) else {
            return;
        };
        const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        tlas[0] = Some(wgpu::TlasInstance::new(blas, IDENTITY, 0, 0xFF));
        let size = posed_size(self.posed);
        let entry = wgpu::BlasBuildEntry {
            blas,
            geometry: wgpu::BlasGeometries::TriangleGeometries(vec![wgpu::BlasTriangleGeometry {
                size: &size,
                vertex_buffer: buffer,
                first_vertex: 0,
                vertex_stride: POSED_BYTES,
                index_buffer: None,
                first_index: None,
                transform_buffer: None,
                transform_buffer_offset: None,
            }]),
        };
        encoder.build_acceleration_structures(std::iter::once(&entry), std::iter::once(&*tlas));
    }

    /// The posed top level to trace this frame, if any.
    fn traced(&self) -> Option<&wgpu::Tlas> {
        self.tlas.as_ref().filter(|_| self.posed > 0)
    }
}

/// The posed bottom level's geometry: `vertices` unindexed, three a triangle.
fn posed_size(vertices: u32) -> wgpu::BlasTriangleGeometrySizeDescriptor {
    wgpu::BlasTriangleGeometrySizeDescriptor {
        vertex_format: wgpu::VertexFormat::Float32x3,
        vertex_count: vertices,
        index_format: None,
        index_count: None,
        flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
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
        Some(Targets { size, result })
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
            timer.harvest(&self.stats);
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
        let posed = self.poser.pose(device, queue, encoder, scene, &faces.posed);
        // the trace reads the posed top level only when there is one
        queue.write_buffer(
            &self.buffer,
            (std::mem::offset_of!(ParamsU, mode) + 12) as u64,
            bytemuck::bytes_of(&if posed > 0 { 1.0f32 } else { 0.0 }),
        );
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
        self.poser.build(encoder);
        let posed_faces = if posed > 0 { self.poser.table.len() } else { 0 };
        self.stats.posed_faces = posed_faces;
        self.stats.posed_vertices = posed as u64;
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
                    flags: wgpu::AccelerationStructureGeometryFlags::OPAQUE,
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
            self.trace(device, encoder, t, depth, contact);
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
                };
                let blas = match self.bottoms.get(&key) {
                    Some(b) => &b.blas,
                    None => match new.iter().position(|(k, _)| *k == key) {
                        Some(n) => &created[n],
                        None => continue,
                    },
                };
                slots[placed] = Some(wgpu::TlasInstance::new(blas, row_major(&i.model), 0, 0xFF));
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
        };
        for ((key, _), blas) in new.into_iter().zip(created) {
            self.bottoms.insert(key, Bottom { blas, used: frame });
        }
        // a face left out (its level waiting) builds again next frame
        self.built_for.clear();
        if waiting == 0 {
            self.built_for.extend_from_slice(instances);
        }
        self.trace(device, encoder, t, depth, contact);
    }

    /// The trace of the mask through the top level as it stands.
    fn trace(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
    ) {
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
                    resource: wgpu::BindingResource::TextureView(&t.result),
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
            pass.dispatch_workgroups(t.size.0.div_ceil(8), t.size.1.div_ceil(8), 1);
        }
        if let Some(timer) = &mut self.timer {
            timer.resolve(encoder);
        }
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
