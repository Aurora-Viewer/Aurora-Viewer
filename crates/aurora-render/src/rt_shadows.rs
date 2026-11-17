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
//! What is traced: the opaque faces of objects placed in the world and the
//! terrain. Not yet: rigged and worn faces (posed by the vertex shaders, so
//! their positions exist on the GPU only), alpha-masked and blended faces.
//! The shadow maps still shade those passes the mask does not cover (blended
//! faces, reflections).

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

/// The faces to trace this frame, from the records and the culling tables
/// the main thread keeps (see the module's comment for what is left out).
pub(crate) fn gather(records: &RecordStore, cull: &CullTables, out: &mut Vec<Instance>) {
    out.clear();
    for id in 0..records.faces.len() {
        let Some(face) = records.faces.get(id) else {
            continue;
        };
        if face.index_count < 3 {
            continue;
        }
        let Some(record) = records.get(id as u32) else {
            continue;
        };
        let rf = record.flags[0];
        let placed = if face.object == NO_OBJECT {
            rf & flags::TERRAIN != 0
        } else {
            let Some(object) = cull.objects.get(face.object as usize) else {
                continue;
            };
            let worn_or_posed = object_flags::HUD | object_flags::AVATAR | object_flags::RIGGED | object_flags::POSED;
            let face_pass = face.bits & pass::BITS;
            object.flags & object_flags::ACTIVE != 0
                && object.flags & worn_or_posed == 0
                && object.anchor == 0
                && (face_pass == pass::OPAQUE || face_pass == pass::OPAQUE_2S)
                && rf & (flags::SKINNED | flags::ANCHORED | flags::ALPHA_MASK | flags::ALPHA_BLEND | flags::LIP_SYNC) == 0
        };
        if placed {
            out.push(Instance {
                base_vertex: face.base_vertex,
                first_index: face.first_index,
                index_count: face.index_count - face.index_count % 3,
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
}

/// Readbacks in flight for the pass's timestamps.
const RING: usize = 4;

/// The GPU time of the builds and of the trace: three timestamps (before the
/// builds, before the trace, after it), resolved and read back a few frames
/// later; their averages are logged every [`Timer::REPORT`] frames.
struct Timer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    ring: Vec<(wgpu::Buffer, std::sync::Arc<std::sync::atomic::AtomicU8>)>,
    next: usize,
    period_ns: f32,
    sums: [f64; 2],
    count: u32,
    frames: u32,
}

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
            count: 3,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rt shadows timestamps"),
            size: 3 * 8,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ring = (0..RING)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("rt shadows timestamps readback"),
                    size: 3 * 8,
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
            sums: [0.0; 2],
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
                        let mut t = [0u64; 3];
                        bytemuck::cast_slice_mut::<u64, u8>(&mut t).copy_from_slice(&data[..24]);
                        if t[1] >= t[0] && t[2] >= t[1] {
                            let ms = |a: u64, b: u64| (b - a) as f64 * self.period_ns as f64 / 1e6;
                            self.sums[0] += ms(t[0], t[1]);
                            self.sums[1] += ms(t[1], t[2]);
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
                "AURORA_RT_SHADOWS: build {:.3} ms, trace {:.3} ms (GPU, mean of {} frames); {} faces traced, {} bottom levels ({} built last frame, {} triangles), {} waiting",
                self.sums[0] / n,
                self.sums[1] / n,
                self.count,
                stats.instances,
                stats.bottoms,
                stats.built,
                stats.built_triangles,
                stats.waiting,
            );
            self.sums = [0.0; 2];
            self.count = 0;
            self.frames = 0;
        }
    }

    /// After this frame's work: its timestamps to a free readback.
    fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        use std::sync::atomic::Ordering;
        encoder.resolve_query_set(&self.queries, 0..3, &self.resolve, 0);
        let (buffer, state) = &self.ring[self.next];
        if state.load(Ordering::Acquire) == FREE {
            encoder.copy_buffer_to_buffer(&self.resolve, 0, buffer, 0, 3 * 8);
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
                    ty: wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
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
        })
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

    /// Builds what is missing of the bottom levels (within a budget), the top
    /// level of `instances`, and traces the mask from `depth` (and `contact`,
    /// the contact shadows' mask, or a black texture).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        t: &Targets,
        depth: &wgpu::TextureView,
        contact: &wgpu::TextureView,
        vertices: &wgpu::Buffer,
        indices: &wgpu::Buffer,
        instances: &[Instance],
    ) {
        self.frame += 1;
        let frame = self.frame;
        if let Some(timer) = &mut self.timer {
            timer.harvest(&self.stats);
            // the builds' start: an empty pass's timestamp, as wgpu times no
            // build of its own
            drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows builds"),
                timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: None,
                }),
            }));
        }
        let same_arena = self
            .arena
            .as_ref()
            .is_some_and(|(v, i)| v == vertices && i == indices);
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
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rt shadows"),
                timestamp_writes: self.timer.as_ref().map(|timer| wgpu::ComputePassTimestampWrites {
                    query_set: &timer.queries,
                    beginning_of_pass_write_index: Some(1),
                    end_of_pass_write_index: Some(2),
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
