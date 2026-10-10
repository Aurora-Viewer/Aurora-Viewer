//! GPU-driven draw lists (shaders/cull.wgsl).
//!
//! The scene keeps two tables on the GPU, updated only when something
//! changes: one entry per drawable face (indexed by its draw record, kept in
//! `RecordStore`) and one per object (bounding sphere, flags, avatar state).
//! Each frame a compute pass tests every face against every view of the
//! frame (main view, shadow cascades, water reflection, mirror, reflection
//! probe face) and writes the indirect draw arguments of each pass, in record
//! order, into fixed-size bins drawn with `multi_draw_indexed_indirect_count`.
//! Blended faces (sorted back to front), glow and the debug lists stay on
//! the CPU (`Scene::build_lists`), which only visits the objects that have
//! some (`ObjGpu::special`).
//!
//! The CPU path (all lists built by `Scene::build_lists`) remains when the
//! device lacks `MULTI_DRAW_INDIRECT_COUNT`, and with AURORA_CPU_CULL=1.
//!
//! Two owners: the tables are the scene's, kept by the main thread
//! ([`GpuTable`], [`CullTables`]) and written through the frame's journal;
//! the compute culling and its bins ([`GpuCull`]) are the render thread's,
//! which gets the tables' buffers and sizes with each frame packet.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

use crate::types::DrawIndexedIndirect;
use crate::writes::GpuWrites;

/// `CullFace::object` / `CullObject::avatar` of nothing.
pub const NO_OBJECT: u32 = u32::MAX;

/// Pass of a face (`CullFace::bits` low bits), decided on the CPU by the
/// scene's alpha classification.
pub mod pass {
    pub const HIDDEN: u32 = 0;
    pub const OPAQUE: u32 = 1;
    pub const OPAQUE_2S: u32 = 2;
    pub const MASK: u32 = 3;
    pub const MASK_2S: u32 = 4;
    pub const BLEND: u32 = 5;
    pub const BITS: u32 = 7;
}

/// `CullFace::bits`: the object's first face (counts the visible objects).
pub const FACE_FIRST: u32 = 8;

/// `CullObject::flags`.
pub mod object_flags {
    pub const ACTIVE: u32 = 1;
    /// An avatar's own body.
    pub const AVATAR: u32 = 2;
    pub const RIGGED: u32 = 4;
    pub const HUD: u32 = 8;
    /// Avatars, their attachments and animesh: not in reflection probes.
    pub const NO_PROBE: u32 = 16;
}

/// `CullObject::state` of an avatar entry, set every frame by the scene.
pub mod avatar_state {
    /// Among the nearest avatars drawn in full (else an impostor).
    pub const FULL: u32 = 1;
    /// Over the complexity limit: silhouette, attachments hidden.
    pub const TOO_COMPLEX: u32 = 2;
    /// Still loading and hidden until complete.
    pub const LOADING: u32 = 4;
}

pub const BINS: usize = 10;
pub const MAIN_BINS: usize = 4;
pub const BIN_SHADOW: usize = 4;
pub const BIN_REFL_WATER: usize = 7;
pub const BIN_REFL_MIRROR: usize = 8;
pub const BIN_PROBE: usize = 9;
/// Argument regions: the bins, then the occlusion's phase 1 and phase 2
/// prepass lists of the main bins (depth only, in no particular order).
pub const REGION_PRE1: usize = 10;
pub const REGION_PRE2: usize = 14;
const REGIONS: usize = 18;

const COUNT_VISIBLE: usize = 10;
const COUNT_TRIANGLES: usize = 11;
const COUNT_HIDDEN: usize = 12;
/// Counts of the phase 1 and phase 2 prepass lists of each main bin.
pub const COUNT_PRE1: usize = 16;
pub const COUNT_PRE2: usize = 20;
const COUNTS: usize = 32;

/// `CullFrame::sizes.w`.
pub mod frame_flags {
    pub const SHADOWS: u32 = 1;
    pub const REFLECTIONS: u32 = 2;
    pub const WATER: u32 = 4;
    pub const MIRROR: u32 = 8;
    pub const PROBE: u32 = 16;
    pub const OCCLUSION: u32 = 32;
}

const CLASSIFY_GROUP: u32 = 128;

/// One drawable face, indexed by its draw record (must match `CullFace` in
/// cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct CullFace {
    pub index_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    /// Object slot in `GpuCull::objects`, `NO_OBJECT` for records drawn
    /// from the CPU lists only (terrain, water, ban lines...).
    pub object: u32,
    /// `pass` | `FACE_FIRST`.
    pub bits: u32,
    pub _pad: [u32; 3],
}

impl CullFace {
    pub const NONE: CullFace = CullFace {
        index_count: 0,
        first_index: 0,
        base_vertex: 0,
        object: NO_OBJECT,
        bits: 0,
        _pad: [0; 3],
    };
}

/// One object of the scene, indexed like the scene's object slab (must
/// match `CullObject` in cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct CullObject {
    /// Bounding sphere (center, radius).
    pub sphere: [f32; 4],
    pub flags: u32,
    /// Avatar whose state applies: itself, the wearer of an attachment, or
    /// `NO_OBJECT`.
    pub avatar: u32,
    /// `avatar_state` of an avatar entry.
    pub state: u32,
    pub _pad: u32,
}

impl CullObject {
    pub const NONE: CullObject = CullObject {
        sphere: [0.0; 4],
        flags: 0,
        avatar: NO_OBJECT,
        state: 0,
        _pad: 0,
    };
}

/// Per-frame culling parameters (must match `CullFrame` in cull.wgsl).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct CullFrame {
    pub planes: [[f32; 4]; 6],
    pub water: [[f32; 4]; 6],
    pub mirror: [[f32; 4]; 6],
    pub probe: [[f32; 4]; 6],
    pub cascades: [[[f32; 4]; 4]; 3],
    /// Camera, draw distance.
    pub eye: [f32; 4],
    /// Shadow distance, reflection distance, pixel scale.
    pub dist: [f32; 4],
    /// Faces, objects, slots per bin, `frame_flags`.
    pub sizes: [u32; 4],
    /// Classify workgroups.
    pub groups: [u32; 4],
}

impl CullFrame {
    /// Frame parameters from the scene's view, the cascade matrices and the
    /// water reflection, mirror and probe face view-projections (`flags`
    /// says which are drawn); sizes are filled by `GpuCull::prepare`.
    pub fn new(view: &GpuCullView, cascades: &[Mat4; 3], [water, mirror, probe]: [Mat4; 3], flags: u32) -> CullFrame {
        let planes = |vp: Mat4| view_planes(vp).map(|p| p.to_array());
        CullFrame {
            planes: view.planes.map(|p| p.to_array()),
            water: planes(water),
            mirror: planes(mirror),
            probe: planes(probe),
            cascades: cascades.map(|m| m.to_cols_array_2d()),
            eye: view.eye.extend(view.draw_distance).to_array(),
            dist: [view.shadow_distance, view.reflection_distance, view.pixel_scale, 0.0],
            sizes: [0, 0, 0, flags],
            groups: [0; 4],
        }
    }
}

/// What the scene gives the renderer for the GPU culling of a frame (the
/// renderer adds the views it computes itself: cascades, reflections).
#[derive(Debug, Clone, Copy)]
pub struct GpuCullView {
    /// Main view frustum (`CullView` planes; may be the frozen debug view).
    pub planes: [Vec4; 6],
    pub eye: Vec3,
    pub draw_distance: f32,
    pub shadow_distance: f32,
    pub reflection_distance: f32,
    /// Screen height / tan(fov / 2): projected size of a sphere in pixels
    /// is radius * pixel_scale / distance.
    pub pixel_scale: f32,
    pub shadows: bool,
    pub reflections: bool,
}

/// Side and near planes of a reverse-Z infinite view (the sixth accepts all).
pub fn view_planes(view_proj: Mat4) -> [Vec4; 6] {
    let m = view_proj.transpose();
    let norm = |p: Vec4| p / p.truncate().length().max(1e-6);
    [
        norm(m.w_axis + m.x_axis),
        norm(m.w_axis - m.x_axis),
        norm(m.w_axis + m.y_axis),
        norm(m.w_axis - m.y_axis),
        // reverse-Z: z_clip <= w_clip in front of the near plane
        norm(m.w_axis - m.z_axis),
        Vec4::new(0.0, 0.0, 0.0, 1.0),
    ]
}

pub fn sphere_in(planes: &[[f32; 4]; 6], c: Vec3, r: f32) -> bool {
    planes.iter().all(|p| Vec4::from_array(*p).truncate().dot(c) + p[3] >= -r)
}

/// Renderer cascade test (orthographic, radius in clip units).
pub fn in_cascade(m: &Mat4, c: Vec3, r: f32) -> bool {
    let p = *m * c.extend(1.0);
    let rr = r * m.x_axis.truncate().length();
    !(p.x + rr < -1.0 || p.x - rr > 1.0 || p.y + rr < -1.0 || p.y - rr > 1.0 || p.z - rr > 1.0)
}

/// Reference of `face_mask` in cull.wgsl (kept line for line): the bins of
/// face `i`, and whether it counts as a visible object.
pub fn face_bins(frame: &CullFrame, faces: &[CullFace], objects: &[CullObject], i: usize) -> (u32, bool) {
    let Some(f) = faces.get(i) else {
        return (0, false);
    };
    let n = frame.sizes[1];
    if f.object >= n {
        return (0, false);
    }
    let o = objects[f.object as usize];
    if o.flags & object_flags::ACTIVE == 0 || o.flags & object_flags::HUD != 0 {
        return (0, false);
    }
    let is_avatar = o.flags & object_flags::AVATAR != 0;
    if o.avatar < n {
        let st = objects[o.avatar as usize].state;
        if st & avatar_state::FULL == 0 || (st & avatar_state::TOO_COMPLEX != 0 && !is_avatar) || st & avatar_state::LOADING != 0 {
            return (0, false);
        }
    }
    let c = Vec3::new(o.sphere[0], o.sphere[1], o.sphere[2]);
    let r = o.sphere[3];
    let d = (c - Vec3::new(frame.eye[0], frame.eye[1], frame.eye[2])).length();
    if d - r > frame.eye[3] && !is_avatar {
        return (0, false);
    }
    let fl = frame.sizes[3];
    let in_view = sphere_in(&frame.planes, c, r);
    let casts = fl & frame_flags::SHADOWS != 0 && d - r < frame.dist[0];
    let reflects = fl & frame_flags::REFLECTIONS != 0 && d - r < frame.dist[1];
    if !in_view && !casts && !reflects {
        return (0, false);
    }
    let px = r * frame.dist[2] / d.max(0.1);
    if px < 1.5 && !is_avatar {
        return (0, false);
    }
    let visible = in_view && f.bits & FACE_FIRST != 0;
    let p = f.bits & pass::BITS;
    if p == pass::HIDDEN {
        return (0, visible);
    }
    let mut m = 0u32;
    if casts && (p != pass::BLEND || o.flags & (object_flags::RIGGED | object_flags::AVATAR) != 0) {
        for (k, cm) in frame.cascades.iter().enumerate() {
            if in_cascade(&Mat4::from_cols_array_2d(cm), c, r) {
                m |= 1 << (BIN_SHADOW + k);
            }
        }
    }
    let solid = (pass::OPAQUE..=pass::MASK_2S).contains(&p);
    if reflects && solid {
        if fl & frame_flags::WATER != 0 && sphere_in(&frame.water, c, r) {
            m |= 1 << BIN_REFL_WATER;
        }
        if fl & frame_flags::MIRROR != 0 && sphere_in(&frame.mirror, c, r) {
            m |= 1 << BIN_REFL_MIRROR;
        }
        if fl & frame_flags::PROBE != 0 && o.flags & object_flags::NO_PROBE == 0 && sphere_in(&frame.probe, c, r) {
            m |= 1 << BIN_PROBE;
        }
    }
    if in_view && solid {
        m |= 1 << (p - pass::OPAQUE);
    }
    (m, visible)
}

/// `pack_bins` of cull.wgsl: one 8-bit counter per bin, four per lane.
pub fn pack_bins(m: u32) -> [u32; 4] {
    let mut v = [0u32; 4];
    for b in 0..BINS {
        if (m >> b) & 1 != 0 {
            v[b >> 2] += 1 << ((b & 3) * 8);
        }
    }
    v
}

/// Rank of each face in each of its bins inside one workgroup of the
/// scatter pass, as cull.wgsl computes it (inclusive prefix sum of the
/// packed counters minus the face's own): reference for the tests.
pub fn workgroup_ranks(masks: &[u32]) -> Vec<[u32; BINS]> {
    let mut acc = [0u32; 4];
    masks
        .iter()
        .map(|&m| {
            let own = pack_bins(m);
            for k in 0..4 {
                acc[k] += own[k];
            }
            let mut r = [0u32; BINS];
            for (b, rb) in r.iter_mut().enumerate() {
                let excl = acc[b >> 2] - own[b >> 2];
                *rb = (excl >> ((b & 3) * 8)) & 0xff;
            }
            r
        })
        .collect()
}

/// Elements per dirty chunk of a `GpuTable`.
const CHUNK: usize = 64;

/// Dirty chunks of a mirrored table, coalesced into upload ranges.
#[derive(Default, Debug)]
struct DirtyChunks {
    dirty: Vec<bool>,
    any: bool,
}

impl DirtyChunks {
    fn mark(&mut self, i: usize) {
        let c = i / CHUNK;
        if self.dirty.len() <= c {
            self.dirty.resize(c + 1, false);
        }
        self.dirty[c] = true;
        self.any = true;
    }

    fn clear(&mut self) {
        self.dirty.iter_mut().for_each(|d| *d = false);
        self.any = false;
    }

    /// Element ranges to upload (clipped to `len`); clears the marks.
    fn take_ranges(&mut self, len: usize) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        if !self.any {
            return out;
        }
        let mut c = 0;
        while c < self.dirty.len() {
            if !self.dirty[c] {
                c += 1;
                continue;
            }
            let start = c;
            while c < self.dirty.len() && self.dirty[c] {
                self.dirty[c] = false;
                c += 1;
            }
            let (a, b) = (start * CHUNK, (c * CHUNK).min(len));
            if a < b {
                out.push(a..b);
            }
        }
        self.any = false;
        out
    }
}

/// Storage buffer mirrored on the CPU; only changed chunks are uploaded.
pub struct GpuTable<T: Pod + PartialEq> {
    label: &'static str,
    buffer: wgpu::Buffer,
    mirror: Vec<T>,
    fill: T,
    dirty: DirtyChunks,
    /// Bumped when the buffer is reallocated (bind groups must follow).
    pub generation: u64,
    /// Bytes sent by the last `flush`.
    pub uploaded: u64,
}

impl<T: Pod + PartialEq> GpuTable<T> {
    pub fn new(device: &wgpu::Device, label: &'static str, cap: usize, fill: T) -> Self {
        GpuTable {
            label,
            buffer: Self::make(device, label, cap.max(1)),
            mirror: Vec::with_capacity(cap),
            fill,
            dirty: DirtyChunks::default(),
            generation: 0,
            uploaded: 0,
        }
    }

    fn make(device: &wgpu::Device, label: &str, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (cap * std::mem::size_of::<T>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn len(&self) -> usize {
        self.mirror.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mirror.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        self.mirror.get(i)
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Set entry `i` (the table grows with the fill value); unchanged
    /// entries are not uploaded again.
    pub fn set(&mut self, i: usize, v: T) {
        if i >= self.mirror.len() {
            if v == self.fill {
                return;
            }
            let from = self.mirror.len();
            self.mirror.resize(i + 1, self.fill);
            for k in from..=i {
                self.dirty.mark(k);
            }
        }
        if self.mirror[i] != v {
            self.mirror[i] = v;
            self.dirty.mark(i);
        }
    }

    /// At least `len` entries (new ones hold the fill value).
    pub fn ensure_len(&mut self, len: usize) {
        let from = self.mirror.len();
        if len > from {
            self.mirror.resize(len, self.fill);
            for k in from..len {
                self.dirty.mark(k);
            }
        }
    }

    /// Every entry back to the fill value.
    pub fn reset(&mut self) {
        let fill = self.fill;
        for i in 0..self.mirror.len() {
            if self.mirror[i] != fill {
                self.mirror[i] = fill;
                self.dirty.mark(i);
            }
        }
    }

    /// Record the upload of the changed chunks in the frame's journal.
    pub fn flush(&mut self, device: &wgpu::Device, writes: &mut GpuWrites) {
        let size = std::mem::size_of::<T>();
        self.uploaded = 0;
        let needed = (self.mirror.len() * size) as u64;
        if needed > self.buffer.size() {
            let cap = self.mirror.len().next_power_of_two();
            self.buffer = Self::make(device, self.label, cap);
            self.generation += 1;
            writes.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.mirror));
            self.uploaded = needed;
            self.dirty.clear();
            return;
        }
        for r in self.dirty.take_ranges(self.mirror.len()) {
            writes.write_buffer(&self.buffer, (r.start * size) as u64, bytemuck::cast_slice(&self.mirror[r.clone()]));
            self.uploaded += (r.len() * size) as u64;
        }
    }
}

/// Bin sizes and statistics of a frame, read back a frame or two later.
#[derive(Debug, Clone, Copy, Default)]
pub struct CullCounts {
    pub bins: [u32; BINS],
    pub visible_objects: u32,
    pub triangles: u32,
    /// Main-bin draws hidden by the occlusion.
    pub hidden: u32,
}

impl CullCounts {
    pub fn main_draws(&self) -> u32 {
        self.bins[..MAIN_BINS].iter().sum()
    }

    pub fn shadow_draws(&self) -> u32 {
        self.bins[BIN_SHADOW..BIN_SHADOW + 3].iter().sum()
    }
}

fn storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
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

fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}

/// The scene's side of the GPU culling, on the main thread.
pub struct CullTables {
    /// Objects table (indexed like the scene's object slab).
    pub objects: GpuTable<CullObject>,
    /// Last counts read back by the render thread (a few frames old).
    pub last: CullCounts,
}

impl CullTables {
    pub fn new(device: &wgpu::Device) -> Self {
        CullTables {
            objects: GpuTable::new(device, "cull objects", 4096, CullObject::NONE),
            last: CullCounts::default(),
        }
    }
}

pub struct GpuCull {
    lists_layout: wgpu::BindGroupLayout,
    occlude_layout: wgpu::BindGroupLayout,
    classify: wgpu::ComputePipeline,
    scan: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
    occlude: wgpu::ComputePipeline,
    frame: wgpu::Buffer,
    args: wgpu::Buffer,
    /// Slots per bin (argument region).
    cap: u32,
    counts: wgpu::Buffer,
    masks: wgpu::Buffer,
    group_data: wgpu::Buffer,
    /// This frame: faces tested and classify workgroups.
    faces: u32,
    groups: u32,
    readback: Vec<(wgpu::Buffer, Arc<AtomicBool>, bool)>,
    /// Readback slot of this frame's copy (set while the frame's draw
    /// closures borrow the bins).
    copied: std::cell::Cell<Option<usize>>,
    /// Last counts read back.
    pub last: CullCounts,
    max_groups: u32,
}

impl GpuCull {
    /// The GPU lists need indirect draw counts and enough storage buffers;
    /// AURORA_CPU_CULL=1 keeps the CPU lists (comparison).
    pub fn supported(device: &wgpu::Device) -> bool {
        static FORCE_CPU: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let force_cpu = *FORCE_CPU.get_or_init(|| std::env::var_os("AURORA_CPU_CULL").is_some_and(|v| v != "0"));
        !force_cpu
            && device.features().contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
            && device.limits().max_storage_buffers_per_shader_stage >= 8
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cull"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", include_str!("shaders/hiz_test.wgsl"), include_str!("shaders/cull.wgsl")).into(),
            ),
        });
        let lists_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull lists"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
            ],
        });
        let occlude_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cull occlusion"),
            entries: &[
                uniform(0),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                storage(6, false),
                storage(7, false),
                uniform(8),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(entry),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let classify = pipeline(&lists_layout, "cs_classify");
        let scan = pipeline(&lists_layout, "cs_scan");
        let scatter = pipeline(&lists_layout, "cs_scatter");
        let occlude = pipeline(&occlude_layout, "cs_occlude");
        let frame = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull frame"),
            size: std::mem::size_of::<CullFrame>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull counts"),
            size: (COUNTS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cap = 4096;
        let mut s = GpuCull {
            lists_layout,
            occlude_layout,
            classify,
            scan,
            scatter,
            occlude,
            frame,
            args: Self::make_args(device, cap),
            cap,
            counts,
            masks: Self::make_storage(device, "cull masks", cap as u64 * 4),
            group_data: Self::make_storage(device, "cull groups", (cap / CLASSIFY_GROUP) as u64 * BINS as u64 * 4),
            faces: 0,
            groups: 0,
            readback: Vec::new(),
            copied: std::cell::Cell::new(None),
            last: CullCounts::default(),
            max_groups: device.limits().max_compute_workgroups_per_dimension,
        };
        s.readback = (0..3)
            .map(|_| {
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cull readback"),
                    size: (COUNTS * 4) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (b, Arc::new(AtomicBool::new(false)), false)
            })
            .collect();
        s
    }

    fn make_storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size.max(16),
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        })
    }

    fn make_args(device: &wgpu::Device, cap: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cull args"),
            size: REGIONS as u64 * cap as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64,
            // copy source: GPU test read-back
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Size the buffers for `faces` face slots and upload the frame
    /// parameters (`frame.sizes` and `groups` are filled in here); `objects`:
    /// entries of the scene's object table this frame.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, faces: usize, objects: usize, mut frame: CullFrame) {
        let faces = (faces as u32).min(self.max_groups.saturating_mul(CLASSIFY_GROUP));
        if faces > self.cap {
            self.cap = faces.next_power_of_two();
            self.args = Self::make_args(device, self.cap);
            self.masks = Self::make_storage(device, "cull masks", self.cap as u64 * 4);
            self.group_data = Self::make_storage(device, "cull groups", self.cap.div_ceil(CLASSIFY_GROUP) as u64 * BINS as u64 * 4);
            log::info!("GPU culling: {} slots per bin", self.cap);
        }
        self.faces = faces;
        self.groups = faces.div_ceil(CLASSIFY_GROUP).max(1);
        frame.sizes[0] = faces;
        frame.sizes[1] = objects as u32;
        frame.sizes[2] = self.cap;
        frame.groups = [self.groups, 0, 0, 0];
        queue.write_buffer(&self.frame, 0, bytemuck::bytes_of(&frame));
    }

    /// Classify, scan and scatter: this frame's bins.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        faces: &wgpu::Buffer,
        objects: &wgpu::Buffer,
        visibility: &wgpu::Buffer,
        timestamps: Option<wgpu::ComputePassTimestampWrites>,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull lists"),
            layout: &self.lists_layout,
            entries: &[
                entry(0, &self.frame),
                entry(1, faces),
                entry(2, objects),
                entry(3, visibility),
                entry(4, &self.masks),
                entry(5, &self.group_data),
                entry(6, &self.counts),
                entry(7, &self.args),
            ],
        });
        encoder.clear_buffer(&self.counts, 0, None);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu culling"),
            timestamp_writes: timestamps,
        });
        pass.set_bind_group(0, &bg, &[]);
        pass.set_pipeline(&self.classify);
        pass.dispatch_workgroups(self.groups, 1, 1);
        pass.set_pipeline(&self.scan);
        pass.dispatch_workgroups(BINS as u32, 1, 1);
        pass.set_pipeline(&self.scatter);
        pass.dispatch_workgroups(self.groups, 1, 1);
    }

    /// Occlusion test of the main bins (after the Hi-Z pyramid of the phase
    /// 1 prepass): hidden draws get instance_count 0, phase 2 arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_occlusion(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        faces: &wgpu::Buffer,
        objects: &wgpu::Buffer,
        visibility: &wgpu::Buffer,
        occl_params: &wgpu::Buffer,
        hiz: &wgpu::TextureView,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull occlusion"),
            layout: &self.occlude_layout,
            entries: &[
                entry(0, &self.frame),
                entry(1, faces),
                entry(2, objects),
                entry(3, visibility),
                entry(6, &self.counts),
                entry(7, &self.args),
                entry(8, occl_params),
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(hiz),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu culling occlusion"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.occlude);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(self.cap.div_ceil(64).min(self.max_groups), MAIN_BINS as u32, 1);
    }

    /// Copy this frame's counts for the statistics (mapped after the submit).
    pub fn copy_counts(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(slot) = self.readback.iter().position(|r| !r.2) {
            encoder.copy_buffer_to_buffer(&self.counts, 0, &self.readback[slot].0, 0, (COUNTS * 4) as u64);
            self.copied.set(Some(slot));
        }
    }

    pub fn after_submit(&mut self) {
        let Some(slot) = self.copied.take() else {
            return;
        };
        let (buf, ready, pending) = &mut self.readback[slot];
        let ready = ready.clone();
        *pending = true;
        buf.map_async(wgpu::MapMode::Read, .., move |r| {
            if r.is_ok() {
                ready.store(true, Ordering::Release);
            }
        });
    }

    /// Collect the finished count readbacks (never waits).
    pub fn harvest(&mut self) {
        for (buf, ready, pending) in self.readback.iter_mut() {
            if *pending && ready.load(Ordering::Acquire) {
                if let Ok(view) = buf.get_mapped_range(..) {
                    let n: &[u32] = bytemuck::cast_slice(&view[..COUNTS * 4]);
                    let mut c = CullCounts::default();
                    c.bins.copy_from_slice(&n[..BINS]);
                    c.visible_objects = n[COUNT_VISIBLE];
                    c.triangles = n[COUNT_TRIANGLES];
                    c.hidden = n[COUNT_HIDDEN];
                    self.last = c;
                }
                buf.unmap();
                *pending = false;
                ready.store(false, Ordering::Release);
            }
        }
    }

    pub fn args(&self) -> &wgpu::Buffer {
        &self.args
    }

    pub fn counts(&self) -> &wgpu::Buffer {
        &self.counts
    }

    /// Slots per bin: the `max_count` of the indirect count draws.
    pub fn cap(&self) -> u32 {
        self.cap
    }

    /// Byte offset of argument region `r` (a bin or a prepass copy).
    pub fn region_offset(&self, r: usize) -> u64 {
        r as u64 * self.cap as u64 * std::mem::size_of::<DrawIndexedIndirect>() as u64
    }

    /// Byte offset of count `c`: a bin, or `COUNT_PRE1` / `COUNT_PRE2` + bin.
    pub fn count_offset(c: usize) -> u64 {
        c as u64 * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wgsl_fields(src: &str, name: &str) -> Vec<String> {
        let start = src.find(&format!("struct {name} {{")).expect("struct in cull.wgsl");
        let body = &src[start..start + src[start..].find('}').expect("end of struct")];
        body.lines()
            .skip(1)
            .filter_map(|l| l.trim().split(':').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty() && !n.starts_with("//"))
            .collect()
    }

    #[test]
    fn layouts_match_wgsl() {
        let src = include_str!("shaders/cull.wgsl");
        assert_eq!(std::mem::size_of::<CullFace>(), 32);
        assert_eq!(
            wgsl_fields(src, "CullFace"),
            ["index_count", "first_index", "base_vertex", "object", "bits", "_p0", "_p1", "_p2"]
        );
        assert_eq!(std::mem::size_of::<CullObject>(), 32);
        assert_eq!(wgsl_fields(src, "CullObject"), ["sphere", "flags", "avatar", "state", "_p"]);
        // uniform layout: 4 x 6 planes, 3 matrices, 4 vectors
        assert_eq!(std::mem::size_of::<CullFrame>(), 4 * 6 * 16 + 3 * 64 + 4 * 16);
        assert_eq!(
            wgsl_fields(src, "CullFrame"),
            ["planes", "water", "mirror", "probe", "cascades", "eye", "dist", "sizes", "groups"]
        );
    }

    /// The constants shared with the shader have the same values.
    #[test]
    fn constants_match_wgsl() {
        let src = include_str!("shaders/cull.wgsl");
        let value = |name: &str| -> u32 {
            let l = src
                .lines()
                .find(|l| l.starts_with(&format!("const {name}:")))
                .unwrap_or_else(|| panic!("const {name}"));
            let v = l
                .split('=')
                .nth(1)
                .expect("value")
                .trim()
                .trim_end_matches(';')
                .trim_end_matches('u');
            v.parse().unwrap_or_else(|_| panic!("{name}: {v}"))
        };
        let pairs: [(&str, u32); 28] = [
            ("BINS", BINS as u32),
            ("MAIN_BINS", MAIN_BINS as u32),
            ("BIN_SHADOW", BIN_SHADOW as u32),
            ("BIN_REFL_WATER", BIN_REFL_WATER as u32),
            ("BIN_REFL_MIRROR", BIN_REFL_MIRROR as u32),
            ("BIN_PROBE", BIN_PROBE as u32),
            ("REGION_PRE1", REGION_PRE1 as u32),
            ("REGION_PRE2", REGION_PRE2 as u32),
            ("COUNT_VISIBLE", COUNT_VISIBLE as u32),
            ("COUNT_TRIANGLES", COUNT_TRIANGLES as u32),
            ("COUNT_HIDDEN", COUNT_HIDDEN as u32),
            ("COUNT_PRE1", COUNT_PRE1 as u32),
            ("COUNT_PRE2", COUNT_PRE2 as u32),
            ("PASS_BITS", pass::BITS),
            ("PASS_HIDDEN", pass::HIDDEN),
            ("PASS_OPAQUE", pass::OPAQUE),
            ("PASS_MASK_2S", pass::MASK_2S),
            ("PASS_BLEND", pass::BLEND),
            ("FACE_FIRST", FACE_FIRST),
            ("OBJ_ACTIVE", object_flags::ACTIVE),
            ("OBJ_AVATAR", object_flags::AVATAR),
            ("OBJ_RIGGED", object_flags::RIGGED),
            ("OBJ_HUD", object_flags::HUD),
            ("OBJ_NO_PROBE", object_flags::NO_PROBE),
            ("AV_FULL", avatar_state::FULL),
            ("AV_TOO_COMPLEX", avatar_state::TOO_COMPLEX),
            ("AV_LOADING", avatar_state::LOADING),
            ("F_OCCLUSION", frame_flags::OCCLUSION),
        ];
        for (name, v) in pairs {
            assert_eq!(value(name), v, "{name}");
        }
        for (name, v) in [
            ("F_SHADOWS", frame_flags::SHADOWS),
            ("F_REFLECTIONS", frame_flags::REFLECTIONS),
            ("F_WATER", frame_flags::WATER),
            ("F_MIRROR", frame_flags::MIRROR),
            ("F_PROBE", frame_flags::PROBE),
        ] {
            assert_eq!(value(name), v, "{name}");
        }
        const { assert!(REGION_PRE2 + MAIN_BINS == REGIONS && REGION_PRE1 == BINS) };
    }

    #[test]
    fn dirty_chunks_coalesce_and_clip() {
        let mut d = DirtyChunks::default();
        assert!(d.take_ranges(1000).is_empty());
        d.mark(3);
        d.mark(70);
        d.mark(300);
        assert_eq!(d.take_ranges(310), vec![0..128, 256..310]);
        assert!(d.take_ranges(310).is_empty());
    }

    /// Packed 8-bit counters never carry into the next bin for a full
    /// workgroup, and give each face its rank among the earlier faces.
    #[test]
    fn packed_ranks_match_naive_ranks() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..20 {
            let masks: Vec<u32> = (0..CLASSIFY_GROUP)
                .map(|_| {
                    if round == 0 {
                        (1 << BINS) - 1
                    } else {
                        (rand() as u32) & ((1 << BINS) - 1)
                    }
                })
                .collect();
            let ranks = workgroup_ranks(&masks);
            let mut seen = [0u32; BINS];
            for (m, r) in masks.iter().zip(&ranks) {
                for b in 0..BINS {
                    if (m >> b) & 1 != 0 {
                        assert_eq!(r[b], seen[b]);
                        seen[b] += 1;
                    }
                }
            }
        }
    }

    fn frame_for_tests() -> CullFrame {
        let view = glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::X, Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.5, 0.1);
        let planes = view_planes(proj * view).map(|p| p.to_array());
        // orthographic box of 100 m around the origin
        let cascade = Mat4::from_scale(Vec3::new(0.02, 0.02, 0.01));
        CullFrame {
            planes,
            water: planes,
            mirror: planes,
            probe: planes,
            cascades: [cascade.to_cols_array_2d(); 3],
            eye: [0.0, 0.0, 0.0, 64.0],
            dist: [64.0, 32.0, 1000.0 / 0.5, 0.0],
            sizes: [0, 4, 0, frame_flags::SHADOWS | frame_flags::REFLECTIONS | frame_flags::WATER],
            groups: [0; 4],
        }
    }

    #[test]
    fn reference_bins_follow_the_cpu_rules() {
        let frame = frame_for_tests();
        let obj = |c: [f32; 3], r: f32| CullObject {
            sphere: [c[0], c[1], c[2], r],
            flags: object_flags::ACTIVE,
            ..CullObject::NONE
        };
        let mut objects = vec![
            obj([10.0, 0.0, 0.0], 1.0),  // in view, near
            obj([-10.0, 0.0, 0.0], 1.0), // behind: shadows only
            obj([100.0, 0.0, 0.0], 1.0), // beyond the draw distance
            CullObject {
                flags: object_flags::ACTIVE | object_flags::AVATAR,
                avatar: 3,
                state: 0,
                ..obj([5.0, 0.0, 0.0], 1.0)
            },
        ];
        let face = |object: u32, p: u32| CullFace {
            index_count: 30,
            object,
            bits: p | FACE_FIRST,
            ..CullFace::NONE
        };
        let faces = [
            face(0, pass::OPAQUE),
            face(0, pass::BLEND),
            face(1, pass::MASK),
            face(2, pass::OPAQUE),
            face(3, pass::OPAQUE),
            face(0, pass::HIDDEN),
        ];
        let shadows = 0b111 << BIN_SHADOW;
        assert_eq!(face_bins(&frame, &faces, &objects, 0), (1 | shadows | 1 << BIN_REFL_WATER, true));
        // blended: the CPU draws it, not a caster unless rigged
        assert_eq!(face_bins(&frame, &faces, &objects, 1), (0, true));
        // behind the camera: casts only
        assert_eq!(face_bins(&frame, &faces, &objects, 2).0, shadows);
        assert_eq!(face_bins(&frame, &faces, &objects, 3), (0, false));
        // an avatar not among the nearest: impostor, nothing on the GPU
        assert_eq!(face_bins(&frame, &faces, &objects, 4), (0, false));
        objects[3].state = avatar_state::FULL;
        assert_eq!(face_bins(&frame, &faces, &objects, 4).0 & 1, 1);
        objects[3].state = avatar_state::FULL | avatar_state::LOADING;
        assert_eq!(face_bins(&frame, &faces, &objects, 4).0, 0);
        assert_eq!(face_bins(&frame, &faces, &objects, 5), (0, true));
    }

    /// The compute passes give the reference bins, in record order, for a
    /// random scene larger than one workgroup and one scan chunk. Needs a
    /// GPU: `cargo test -p aurora-render gpu_bins -- --ignored`.
    #[test]
    #[ignore]
    fn gpu_bins_match_the_reference() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as u32
        };
        let mut cull = GpuCull::new(&device);
        let mut tables = CullTables::new(&device);
        let n_obj = 3000u32;
        let mut objects = Vec::new();
        for i in 0..n_obj {
            let avatar = rand(10) == 0;
            let o = CullObject {
                sphere: [
                    rand(200) as f32 - 100.0,
                    rand(200) as f32 - 100.0,
                    rand(40) as f32 - 20.0,
                    0.1 + rand(50) as f32 * 0.1,
                ],
                flags: object_flags::ACTIVE
                    | if avatar { object_flags::AVATAR } else { 0 }
                    | if rand(8) == 0 { object_flags::RIGGED } else { 0 }
                    | if rand(30) == 0 { object_flags::HUD } else { 0 }
                    | if rand(5) == 0 { object_flags::NO_PROBE } else { 0 },
                avatar: if avatar {
                    i
                } else if rand(6) == 0 {
                    rand(n_obj)
                } else {
                    NO_OBJECT
                },
                state: rand(8),
                _pad: 0,
            };
            tables.objects.set(i as usize, o);
            objects.push(o);
        }
        let n_faces = 70_000usize;
        let faces: Vec<CullFace> = (0..n_faces)
            .map(|_| {
                if rand(20) == 0 {
                    return CullFace::NONE;
                }
                CullFace {
                    index_count: 3 + 3 * rand(100),
                    first_index: rand(100_000),
                    base_vertex: rand(100_000) as i32,
                    object: rand(n_obj),
                    bits: rand(6) | if rand(4) == 0 { FACE_FIRST } else { 0 },
                    _pad: [0; 3],
                }
            })
            .collect();
        let mut table = GpuTable::new(&device, "faces", 16, CullFace::NONE);
        for (i, f) in faces.iter().enumerate() {
            table.set(i, *f);
        }
        let mut writes = GpuWrites::default();
        table.flush(&device, &mut writes);
        tables.objects.flush(&device, &mut writes);
        writes.replay(&mut crate::writes::QueueSink(&queue));
        // visible last frame (occlusion phase 1 list)
        let was: Vec<u32> = (0..n_faces).map(|_| rand(2)).collect();
        let visibility = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: n_faces as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&visibility, 0, bytemuck::cast_slice(&was));
        let mut frame = frame_for_tests();
        frame.sizes[3] = frame_flags::SHADOWS | frame_flags::REFLECTIONS | frame_flags::WATER | frame_flags::PROBE | frame_flags::OCCLUSION;
        frame.probe = view_planes(
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.5, 1.0, 0.1)
                * glam::camera::rh::view::look_at_mat4(Vec3::ZERO, Vec3::Y, Vec3::Z),
        )
        .map(|p| p.to_array());
        cull.prepare(&device, &queue, n_faces, tables.objects.len(), frame);
        let mut f = frame;
        f.sizes[0] = n_faces as u32;
        f.sizes[1] = n_obj;
        let mut encoder = device.create_command_encoder(&Default::default());
        cull.encode(&device, &mut encoder, table.buffer(), tables.objects.buffer(), &visibility, None);
        let size = cull.args().size();
        let read = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let args_read = read("args");
        let counts_read = read("counts");
        encoder.copy_buffer_to_buffer(cull.args(), 0, &args_read, 0, size);
        encoder.copy_buffer_to_buffer(cull.counts(), 0, &counts_read, 0, (COUNTS * 4) as u64);
        queue.submit([encoder.finish()]);
        args_read.map_async(wgpu::MapMode::Read, .., |_| {});
        counts_read.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let args = args_read.get_mapped_range(..).expect("args mapped");
        let args: &[u32] = bytemuck::cast_slice(&args);
        let counts = counts_read.get_mapped_range(..).expect("counts mapped");
        let counts: &[u32] = bytemuck::cast_slice(&counts[..COUNTS * 4]);
        let mut expect: Vec<Vec<u32>> = vec![Vec::new(); BINS];
        let (mut visible, mut tris) = (0, 0);
        for i in 0..n_faces {
            let (m, v) = face_bins(&f, &faces, &objects, i);
            visible += v as u32;
            for (b, e) in expect.iter_mut().enumerate() {
                if (m >> b) & 1 != 0 {
                    e.push(i as u32);
                    if b < MAIN_BINS {
                        tris += faces[i].index_count / 3;
                    }
                }
            }
        }
        assert!(expect.iter().take(MAIN_BINS + 3).all(|e| !e.is_empty()), "every bin used");
        assert_eq!(counts[COUNT_VISIBLE], visible);
        assert_eq!(counts[COUNT_TRIANGLES], tris);
        let cap = cull.cap() as usize;
        for (b, e) in expect.iter().enumerate() {
            assert_eq!(counts[b] as usize, e.len(), "bin {b} size");
            for (j, &rec) in e.iter().enumerate() {
                let a = &args[(b * cap + j) * 5..(b * cap + j) * 5 + 5];
                let fc = &faces[rec as usize];
                assert_eq!(
                    a,
                    [fc.index_count, 1, fc.first_index, fc.base_vertex as u32, rec],
                    "bin {b} slot {j}"
                );
            }
            if b < MAIN_BINS {
                // phase 1 prepass list: the bin's draws visible last frame, any order
                let mut want: Vec<u32> = e.iter().copied().filter(|&r| was[r as usize] != 0).collect();
                let n = counts[COUNT_PRE1 + b] as usize;
                let mut got: Vec<u32> = (0..n).map(|j| args[((REGION_PRE1 + b) * cap + j) * 5 + 4]).collect();
                want.sort_unstable();
                got.sort_unstable();
                assert_eq!(got, want, "phase 1 list of bin {b}");
            }
        }
    }
}
