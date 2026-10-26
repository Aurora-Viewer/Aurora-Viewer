//! GPU memory arenas: one big vertex buffer, one big u16 index buffer and
//! a storage buffer of draw records, all sub-allocated on the CPU side.

use crate::gpu_alloc::{Backoff, fallback_capacities, try_buffer};
use crate::gpu_cull::{CullFace, DirtyChunks, GpuTable};
use crate::types::{DrawRecord, SkinVertex, Vertex};
use crate::upload::{StagedMesh, UploadQueue};
use crate::writes::GpuWrites;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// Best-fit range allocator with coalescing (units are elements): the
/// smallest free range that fits, the lowest one among equals, found
/// through an index by length. First fit walked the free ranges in address
/// order: with the ~10,000 holes a region leaves after a few detail-level
/// changes, each allocation cost 9 to 14 µs, twice per face put in the
/// arena (0.2 µs now).
#[derive(Debug)]
pub struct RangeAllocator {
    /// offset -> length
    free: BTreeMap<u32, u32>,
    /// (length, offset) of the same ranges.
    by_len: BTreeSet<(u32, u32)>,
    capacity: u32,
    used: u32,
}

impl RangeAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut a = Self {
            free: BTreeMap::new(),
            by_len: BTreeSet::new(),
            capacity,
            used: 0,
        };
        if capacity > 0 {
            a.insert_free(0, capacity);
        }
        a
    }

    fn insert_free(&mut self, off: u32, len: u32) {
        self.free.insert(off, len);
        self.by_len.insert((len, off));
    }

    fn remove_free(&mut self, off: u32) -> Option<u32> {
        let len = self.free.remove(&off)?;
        self.by_len.remove(&(len, off));
        Some(len)
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let &(flen, off) = self.by_len.range((len, 0)..).next()?;
        self.remove_free(off);
        if flen > len {
            self.insert_free(off + len, flen - len);
        }
        self.used += len;
        Some(off)
    }

    pub fn free(&mut self, off: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.used = self.used.saturating_sub(len);
        let mut start = off;
        let mut length = len;
        // merge with previous
        if let Some((&poff, &plen)) = self.free.range(..off).next_back()
            && poff + plen == off
        {
            self.remove_free(poff);
            start = poff;
            length += plen;
        }
        // merge with next
        if let Some(nlen) = self.remove_free(off + len) {
            length += nlen;
        }
        self.insert_free(start, length);
    }

    /// Extend capacity (after the backing buffer grew).
    pub fn grow(&mut self, new_capacity: u32) {
        if new_capacity <= self.capacity {
            return;
        }
        let extra_off = self.capacity;
        let extra_len = new_capacity - self.capacity;
        self.capacity = new_capacity;
        self.used += extra_len; // free() subtracts it again
        self.free(extra_off, extra_len);
    }
}

/// A mesh living in the geometry arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeshAlloc {
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    /// Real index count (allocation may be padded to even).
    pub index_count: u32,
}

impl MeshAlloc {
    fn index_alloc_len(&self) -> u32 {
        (self.index_count + 1) & !1
    }
}

/// Capacity of a full buffer that must take `need` more elements: twice
/// its size, or more until `need` fits, within `max`. None when even `max`
/// leaves no room for it.
fn grown_capacity(old: u32, need: u32, max: u32) -> Option<u32> {
    let mut cap = old.saturating_mul(2).max(1);
    while cap - old < need && cap < u32::MAX {
        cap = cap.saturating_mul(2);
    }
    if cap > max {
        cap = max;
    }
    (cap > old && cap - old >= need).then_some(cap)
}

/// Capacity for a growth ahead of need to `wanted` elements: None when the
/// buffer is already that large, else `wanted` rounded up to whole
/// megabytes of elements (a few sizes, not one per wish), within `max`.
fn ahead_capacity(old: u32, wanted: u32, max: u32) -> Option<u32> {
    const STEP: u32 = 1 << 20;
    let cap = wanted.div_ceil(STEP).saturating_mul(STEP).min(max);
    (cap > old).then_some(cap)
}

/// The (vertices, indices) a growth ahead of need may ask for within
/// `allowance` bytes of new buffers: the wish when it fits, else the growth
/// of both buffers scaled down alike (a new buffer is whole, the old one is
/// freed after the copy). A buffer kept at its size is not replaced.
fn ahead_within(vertices: (u32, u32), indices: (u32, u32), allowance: u64) -> (u32, u32) {
    let (vold, vwant) = (vertices.0, vertices.1.max(vertices.0));
    let (iold, iwant) = (indices.0, indices.1.max(indices.0));
    let bytes = |v: u32, i: u32| {
        let vb = if v > vold { v as u64 * (VERTEX_SIZE + SKIN_SIZE) } else { 0 };
        let ib = if i > iold { i as u64 * 2 } else { 0 };
        vb + ib
    };
    if bytes(vwant, iwant) <= allowance {
        return (vwant, iwant);
    }
    // the largest fraction of the growth that fits, by halving
    let mut f = 0.5;
    while f > 1.0 / 64.0 {
        let v = vold + ((vwant - vold) as f64 * f) as u32;
        let i = (iold + ((iwant - iold) as f64 * f) as u32) & !1;
        if bytes(v, i) <= allowance {
            return (v, i);
        }
        f /= 2.0;
    }
    (vold, iold)
}

/// A full buffer is replaced by one twice its size, its content copied on
/// the GPU. What holds the frame then is not the copy (32 MB: 0.2 ms) but
/// the first use of the new buffer: fresh video memory costs about 21 µs
/// per MB the first time anything touches it, the whole buffer at once
/// (22 ms for 1 GB on an RTX 4090; `new_buffer_first_use`, below). Hence
/// the doubling: growing by less, or ahead of need a slice of the copy per
/// frame (tried: same stalls, more of them), allocates more buffers and
/// pays more. Only smaller buffers would make these stalls small, and a
/// draw reads one vertex buffer. What can be done is to pay it while
/// nothing is shown: behind a loading or teleport screen the arena grows
/// at once to what the region needed at the last visit, or ahead of need
/// (`grow_ahead`, driven by the viewer's scene/geometry_need.rs).
pub struct GeometryArena {
    pub vertex_buffer: wgpu::Buffer,
    /// Parallel stream of `SkinVertex` (same element offsets as vertices).
    pub skin_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    vertices: RangeAllocator,
    indices: RangeAllocator,
    max_buffer_size: u64,
    /// A growth was refused at the largest buffer: said once.
    full: bool,
    /// A growth failed for want of video memory: when to try again.
    backoff: Backoff,
}

const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
const SKIN_SIZE: u64 = std::mem::size_of::<SkinVertex>() as u64;

impl GeometryArena {
    pub fn new(device: &wgpu::Device, max_buffer_size: u64) -> Self {
        let vcap: u32 = 2 * 1024 * 1024; // 48 MB
        let icap: u32 = 8 * 1024 * 1024; // 16 MB
        Self {
            vertex_buffer: Self::make(device, "vertex arena", vcap as u64 * VERTEX_SIZE, wgpu::BufferUsages::VERTEX),
            skin_buffer: Self::make(device, "skin arena", vcap as u64 * SKIN_SIZE, wgpu::BufferUsages::VERTEX),
            index_buffer: Self::make(device, "index arena", icap as u64 * 2, wgpu::BufferUsages::INDEX),
            vertices: RangeAllocator::new(vcap),
            indices: RangeAllocator::new(icap),
            max_buffer_size,
            full: false,
            backoff: Backoff::default(),
        }
    }

    fn make(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&Self::desc(label, size, usage))
    }

    fn try_make(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> Result<wgpu::Buffer, String> {
        try_buffer(device, &Self::desc(label, size, usage))
    }

    fn desc(label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::BufferDescriptor<'_> {
        wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.vertex_buffer.size() + self.skin_buffer.size() + self.index_buffer.size()
    }

    pub fn used(&self) -> (u64, u64) {
        (self.vertices.used() as u64, self.indices.used() as u64)
    }

    /// The arena cannot grow any more: the meshes it has no room for are
    /// not drawn, which the log must say (once).
    fn refused(&mut self, what: &str, bytes: u64) {
        if !std::mem::replace(&mut self.full, true) {
            log::warn!(
                "{what} arena full at {} MB, the largest buffer of this GPU: the meshes it has no room for are not drawn",
                bytes / (1024 * 1024)
            );
        }
    }

    /// The capacities (vertices, indices) the arena holds.
    pub fn capacity(&self) -> (u32, u32) {
        (self.vertices.capacity(), self.indices.capacity())
    }

    fn max_vertices(&self) -> u32 {
        (self.max_buffer_size / VERTEX_SIZE).min(u32::MAX as u64) as u32
    }

    fn max_indices(&self) -> u32 {
        (self.max_buffer_size / 2).min(u32::MAX as u64) as u32 & !1
    }

    fn grow_vertices(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, need: u32) -> bool {
        let old = self.vertices.capacity();
        let Some(cap) = grown_capacity(old, need, self.max_vertices()) else {
            self.refused("vertex", old as u64 * VERTEX_SIZE);
            return false;
        };
        let now = Instant::now();
        if !self.backoff.ready(now) {
            return false;
        }
        let mut why = String::new();
        for c in fallback_capacities(old, need, cap, 1) {
            match self.grow_vertices_to(device, writes, uploads, c) {
                Ok(()) => {
                    self.backoff.succeeded();
                    return true;
                }
                Err(e) => why = e,
            }
        }
        self.out_of_memory("vertex", old as u64 * VERTEX_SIZE, &why, now);
        false
    }

    /// A growth failed at every size down to what the mesh needed: the
    /// arena keeps its buffers (an invalid one in their place failed every
    /// write after it), the meshes that do not fit are not drawn, and the
    /// growth is tried again a little later.
    fn out_of_memory(&mut self, what: &str, bytes: u64, why: &str, now: Instant) {
        if !self.backoff.waiting() {
            log::warn!(
                "{what} arena could not grow past {} MB ({why}): video memory is full, the meshes that do not fit are not drawn until it grows",
                bytes / (1024 * 1024)
            );
        }
        self.backoff.failed(now);
    }

    fn grow_vertices_to(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        new_cap: u32,
    ) -> Result<(), String> {
        let old = self.vertices.capacity();
        let nb = Self::try_make(device, "vertex arena", new_cap as u64 * VERTEX_SIZE, wgpu::BufferUsages::VERTEX)?;
        let ns = Self::try_make(device, "skin arena", new_cap as u64 * SKIN_SIZE, wgpu::BufferUsages::VERTEX)?;
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow vb") });
        enc.copy_buffer_to_buffer(&self.vertex_buffer, 0, &nb, 0, old as u64 * VERTEX_SIZE);
        enc.copy_buffer_to_buffer(&self.skin_buffer, 0, &ns, 0, old as u64 * SKIN_SIZE);
        // the writes and the copies already recorded into the old buffers
        // come first: the journal keeps the growth at this point of the frame
        uploads.flush(writes, Some(enc));
        self.vertex_buffer = nb;
        self.skin_buffer = ns;
        self.vertices.grow(new_cap);
        log::info!("vertex arena grown to {} MB", new_cap as u64 * VERTEX_SIZE / (1024 * 1024));
        Ok(())
    }

    fn grow_indices(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, need: u32) -> bool {
        let old = self.indices.capacity();
        let Some(cap) = grown_capacity(old, need, self.max_indices()) else {
            self.refused("index", old as u64 * 2);
            return false;
        };
        let now = Instant::now();
        if !self.backoff.ready(now) {
            return false;
        }
        let mut why = String::new();
        for c in fallback_capacities(old, need, cap, 2) {
            match self.grow_indices_to(device, writes, uploads, c) {
                Ok(()) => {
                    self.backoff.succeeded();
                    return true;
                }
                Err(e) => why = e,
            }
        }
        self.out_of_memory("index", old as u64 * 2, &why, now);
        false
    }

    fn grow_indices_to(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        new_cap: u32,
    ) -> Result<(), String> {
        let old = self.indices.capacity();
        let nb = Self::try_make(device, "index arena", new_cap as u64 * 2, wgpu::BufferUsages::INDEX)?;
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow ib") });
        enc.copy_buffer_to_buffer(&self.index_buffer, 0, &nb, 0, old as u64 * 2);
        uploads.flush(writes, Some(enc));
        self.index_buffer = nb;
        self.indices.grow(new_cap);
        log::info!("index arena grown to {} MB", new_cap as u64 * 2 / (1024 * 1024));
        Ok(())
    }

    /// Grow the arena now, while nothing it costs can be seen (a loading or
    /// teleport screen), so that it holds `vertices` and `indices` without
    /// growing again in the middle of the scene: each buffer is replaced at
    /// most once, by one of the size asked for, within the largest buffer
    /// of the GPU and the `allowance` bytes the video memory budget leaves
    /// (when the driver reports one). Nothing happens for a buffer that is
    /// already as large, nor when its allocation fails: the arena then
    /// grows on need, as without this. Returns whether a buffer was
    /// replaced.
    pub fn grow_ahead(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        vertices: u32,
        indices: u32,
        allowance: Option<u64>,
    ) -> bool {
        let (vertices, indices) = match allowance {
            Some(bytes) => ahead_within((self.vertices.capacity(), vertices), (self.indices.capacity(), indices & !1), bytes),
            None => (vertices, indices & !1),
        };
        let mut grown = false;
        if let Some(cap) = ahead_capacity(self.vertices.capacity(), vertices, self.max_vertices()) {
            match self.grow_vertices_to(device, writes, uploads, cap) {
                Ok(()) => grown = true,
                Err(e) => log::warn!(
                    "vertex arena not grown ahead to {} MB ({e}): it grows on need",
                    cap as u64 * VERTEX_SIZE / (1024 * 1024)
                ),
            }
        }
        if let Some(cap) = ahead_capacity(self.indices.capacity(), indices, self.max_indices()) {
            match self.grow_indices_to(device, writes, uploads, cap) {
                Ok(()) => grown = true,
                Err(e) => log::warn!(
                    "index arena not grown ahead to {} MB ({e}): it grows on need",
                    cap as u64 * 2 / (1024 * 1024)
                ),
            }
        }
        grown
    }

    /// Vertex and index ranges for a mesh, growing the arena when full.
    fn reserve(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        vcount: u32,
        icount: u32,
    ) -> Option<MeshAlloc> {
        let ialloc = (icount + 1) & !1;
        let voff = match self.vertices.alloc(vcount) {
            Some(o) => o,
            None => {
                if !self.grow_vertices(device, writes, uploads, vcount) {
                    return None;
                }
                self.vertices.alloc(vcount)?
            }
        };
        let ioff = match self.indices.alloc(ialloc) {
            Some(o) => o,
            None => {
                if !self.grow_indices(device, writes, uploads, ialloc) {
                    self.vertices.free(voff, vcount);
                    return None;
                }
                match self.indices.alloc(ialloc) {
                    Some(o) => o,
                    None => {
                        self.vertices.free(voff, vcount);
                        return None;
                    }
                }
            }
        };
        Some(MeshAlloc {
            vertex_offset: voff,
            vertex_count: vcount,
            index_offset: ioff,
            index_count: icount,
        })
    }

    /// Upload a mesh. Indices are relative to the mesh's first vertex.
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        vertices: &[Vertex],
        skin: Option<&[SkinVertex]>,
        indices: &[u16],
    ) -> Option<MeshAlloc> {
        if vertices.is_empty() || indices.is_empty() || vertices.len() > u32::MAX as usize / 2 {
            return None;
        }
        let icount = indices.len() as u32;
        let m = self.reserve(device, writes, uploads, vertices.len() as u32, icount)?;
        let (voff, ioff) = (m.vertex_offset, m.index_offset);
        writes.write_buffer(&self.vertex_buffer, voff as u64 * VERTEX_SIZE, bytemuck::cast_slice(vertices));
        if let Some(sk) = skin
            && sk.len() == vertices.len()
        {
            writes.write_buffer(&self.skin_buffer, voff as u64 * SKIN_SIZE, bytemuck::cast_slice(sk));
        }
        if icount % 2 == 1 {
            let mut padded = Vec::with_capacity(m.index_alloc_len() as usize);
            padded.extend_from_slice(indices);
            padded.push(*indices.last().unwrap_or(&0));
            writes.write_buffer(&self.index_buffer, ioff as u64 * 2, bytemuck::cast_slice(&padded));
        } else {
            writes.write_buffer(&self.index_buffer, ioff as u64 * 2, bytemuck::cast_slice(indices));
        }
        Some(m)
    }

    /// Upload a mesh staged by a background job (its region must be
    /// ready): the copies are recorded in the frame's upload encoder.
    pub fn alloc_staged(
        &mut self,
        device: &wgpu::Device,
        writes: &mut GpuWrites,
        uploads: &mut UploadQueue,
        mesh: &StagedMesh,
    ) -> Option<MeshAlloc> {
        let m = self.reserve(device, writes, uploads, mesh.vertex_count, mesh.index_count)?;
        let [vertices, skin, indices] = mesh.ranges();
        let voff = m.vertex_offset as u64;
        uploads.copy_to_buffer(device, &mesh.data, vertices, &self.vertex_buffer, voff * VERTEX_SIZE);
        uploads.copy_to_buffer(device, &mesh.data, skin, &self.skin_buffer, voff * SKIN_SIZE);
        uploads.copy_to_buffer(device, &mesh.data, indices, &self.index_buffer, m.index_offset as u64 * 2);
        Some(m)
    }

    pub fn free(&mut self, m: MeshAlloc) {
        self.vertices.free(m.vertex_offset, m.vertex_count);
        self.indices.free(m.index_offset, m.index_alloc_len());
    }
}

/// Elements per block of [`Blocks`]: 1 MB of draw records, a multiple of
/// the dirty chunks (`gpu_cull::CHUNK`).
const BLOCK: usize = 4096;

/// A growable array stored in blocks of [`BLOCK`] elements: it grows by
/// whole blocks and never moves what it holds. As one `Vec`, the mirror of
/// the draw records was copied whole each time it doubled (4, 8, 16 MB on
/// the main thread, in the middle of the scene sync, while a region
/// arrives).
struct Blocks<T> {
    blocks: Vec<Vec<T>>,
    len: usize,
}

impl<T> Blocks<T> {
    fn new() -> Self {
        Blocks {
            blocks: Vec::new(),
            len: 0,
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, v: T) {
        if self.len.is_multiple_of(BLOCK) {
            self.blocks.push(Vec::with_capacity(BLOCK));
        }
        if let Some(b) = self.blocks.last_mut() {
            b.push(v);
            self.len += 1;
        }
    }

    fn get(&self, i: usize) -> Option<&T> {
        self.blocks.get(i / BLOCK)?.get(i % BLOCK)
    }

    fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        self.blocks.get_mut(i / BLOCK)?.get_mut(i % BLOCK)
    }

    /// The elements `a..b` as contiguous runs (first index, slice): one per
    /// block the range crosses.
    fn runs(&self, a: usize, b: usize) -> impl Iterator<Item = (usize, &[T])> {
        let b = b.min(self.len);
        let mut at = a;
        std::iter::from_fn(move || {
            if at >= b {
                return None;
            }
            let block = at / BLOCK;
            let end = b.min((block + 1) * BLOCK);
            let run = (at, &self.blocks[block][at % BLOCK..end - block * BLOCK]);
            at = end;
            Some(run)
        })
    }
}

/// Storage buffer of `DrawRecord`s with a CPU mirror; only the changed
/// records are uploaded (`DirtyChunks`).
/// Each record also has a `CullFace` (GPU draw lists, gpu_cull.rs), reset
/// when the record is allocated or freed: only object faces set one.
pub struct RecordStore {
    pub buffer: wgpu::Buffer,
    /// GPU culling entry of each record (its face, when it is an object's).
    pub faces: GpuTable<CullFace>,
    mirror: Blocks<DrawRecord>,
    free: Vec<u32>,
    dirty: DirtyChunks,
    live: u32,
    /// Set when the buffer was reallocated (bind group must be rebuilt).
    pub generation: u64,
    /// Bytes sent by the last `flush` (AURORA_PROFILE summary).
    pub uploaded: u64,
    /// The buffer could not grow (video memory full): when to try again.
    backoff: Backoff,
}

impl RecordStore {
    pub fn new(device: &wgpu::Device) -> Self {
        let cap = 16384usize;
        Self {
            buffer: Self::make(device, cap),
            faces: GpuTable::new(device, "cull faces", cap, CullFace::NONE),
            mirror: Blocks::new(),
            free: Vec::new(),
            dirty: DirtyChunks::default(),
            live: 0,
            generation: 0,
            uploaded: 0,
            backoff: Backoff::default(),
        }
    }

    fn make(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&Self::desc(cap))
    }

    fn desc(cap: usize) -> wgpu::BufferDescriptor<'static> {
        wgpu::BufferDescriptor {
            label: Some("draw records"),
            size: (cap * std::mem::size_of::<DrawRecord>()) as u64,
            // COPY_SRC: a grown buffer takes over this one's records by a GPU copy
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        }
    }

    pub fn live(&self) -> u32 {
        self.live
    }

    /// Record ids in use or freed (ids are below this).
    pub fn slot_count(&self) -> usize {
        self.mirror.len()
    }

    pub fn alloc(&mut self, rec: DrawRecord) -> u32 {
        self.live += 1;
        if let Some(id) = self.free.pop() {
            self.set(id, rec);
            self.faces.set(id as usize, CullFace::NONE);
            return id;
        }
        let id = self.mirror.len() as u32;
        self.mirror.push(rec);
        self.mark(id as usize);
        id
    }

    fn mark(&mut self, idx: usize) {
        self.dirty.mark(idx);
    }

    pub fn set(&mut self, id: u32, rec: DrawRecord) {
        if let Some(slot) = self.mirror.get_mut(id as usize) {
            *slot = rec;
            self.mark(id as usize);
        }
    }

    /// GPU-clock effects keep a static material until placement/theme changes.
    pub fn set_if_changed(&mut self, id: u32, rec: DrawRecord) {
        if let Some(slot) = self.mirror.get_mut(id as usize)
            && bytemuck::bytes_of(slot) != bytemuck::bytes_of(&rec)
        {
            *slot = rec;
            self.mark(id as usize);
        }
    }

    /// Place a record: its model matrix, in the world or placed from anchor
    /// matrix `anchor` (`flags::ANCHORED`: what an avatar wears keeps the
    /// same record while its wearer moves). Only what differs is written
    /// (no copy of the whole record), and an unchanged record is not
    /// uploaded again.
    pub fn set_placement(&mut self, id: u32, model: [[f32; 4]; 4], anchor: Option<u32>) {
        if let Some(slot) = self.mirror.get_mut(id as usize) {
            let old_flags = slot.flags;
            slot.set_hud_zoom(1.0);
            slot.set_anchor(anchor);
            if slot.model != model || slot.flags != old_flags {
                slot.model = model;
                self.mark(id as usize);
            }
        }
    }

    pub fn set_hud_model(&mut self, id: u32, model: [[f32; 4]; 4], zoom: f32) {
        if let Some(slot) = self.mirror.get_mut(id as usize) {
            let old_flags = slot.flags[0];
            slot.set_hud_zoom(zoom);
            if slot.model != model || slot.flags[0] != old_flags {
                slot.model = model;
                self.mark(id as usize);
            }
        }
    }

    /// The rez effect of a record (`DrawRecord::rez`), uploaded only when
    /// it changes.
    pub fn set_rez(&mut self, id: u32, rez: [u32; 4]) {
        if let Some(slot) = self.mirror.get_mut(id as usize)
            && slot.rez != rez
        {
            slot.rez = rez;
            self.mark(id as usize);
        }
    }

    /// Bodies and world attachments share one finite portal opening.
    pub fn set_portal_clip(&mut self, id: u32, clip: Option<crate::PortalClip>) {
        if let Some(slot) = self.mirror.get_mut(id as usize) {
            let flags =
                (slot.flags[0] & !crate::types::flags::PORTAL_CLIP) | if clip.is_some() { crate::types::flags::PORTAL_CLIP } else { 0 };
            let data = clip.map_or([0.0; 4], |clip| clip.data);
            if slot.portal_clip != data || slot.flags[0] != flags {
                slot.portal_clip = data;
                slot.flags[0] = flags;
                self.mark(id as usize);
            }
        }
    }

    /// The face drawn with record `id` (GPU draw lists).
    pub fn set_face(&mut self, id: u32, face: CullFace) {
        if (id as usize) < self.mirror.len() {
            self.faces.set(id as usize, face);
        }
    }

    pub fn get(&self, id: u32) -> Option<&DrawRecord> {
        self.mirror.get(id as usize)
    }

    pub fn free(&mut self, id: u32) {
        if (id as usize) < self.mirror.len() {
            self.live = self.live.saturating_sub(1);
            self.free.push(id);
            self.faces.set(id as usize, CullFace::NONE);
        }
    }

    /// Record the upload of the changed records in the frame's journal;
    /// reallocates the buffer when it is too small.
    pub fn flush(&mut self, device: &wgpu::Device, writes: &mut GpuWrites) {
        self.faces.flush(device, writes);
        let rec_size = std::mem::size_of::<DrawRecord>();
        self.uploaded = 0;
        let needed = self.mirror.len() * rec_size;
        let now = Instant::now();
        if needed as u64 > self.buffer.size() && self.backoff.ready(now) {
            let old_size = self.buffer.size();
            let mut cap = old_size as usize / rec_size;
            while cap * rec_size < needed {
                cap *= 2;
            }
            // The records the old buffer holds move on the GPU: a submit
            // recorded at this point of the frame's journal, so that the
            // writes of the frames before it are in the old buffer when it
            // is copied, and this frame's writes (recorded below) land
            // after the copy; the new records are all dirty. Writing the
            // whole mirror again through the queue copied 8 to 32 MB on
            // the main thread each time a region outgrew the buffer.
            match try_buffer(device, &Self::desc(cap)) {
                Ok(grown) => {
                    self.backoff.succeeded();
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("grow draw records"),
                    });
                    enc.copy_buffer_to_buffer(&self.buffer, 0, &grown, 0, old_size);
                    writes.submit(vec![enc]);
                    self.buffer = grown;
                    self.generation += 1;
                }
                Err(e) => {
                    if !self.backoff.waiting() {
                        log::warn!(
                            "draw records not grown to {} MB ({e}): video memory is full, the records past {} are not drawn until they fit",
                            (cap * rec_size) >> 20,
                            old_size as usize / rec_size
                        );
                    }
                    self.backoff.failed(now);
                }
            }
        }
        if !self.dirty.any() {
            return;
        }
        // (records past the buffer wait for its growth, which copies the
        // old ones and keeps the rest dirty)
        let fits = self.buffer.size() as usize / rec_size;
        let mut waiting = Vec::new();
        for range in self.dirty.take_ranges(self.mirror.len()) {
            if range.end > fits {
                waiting.push(range.start.max(fits)..range.end);
            }
            let range = range.start.min(fits)..range.end.min(fits);
            for (first, records) in self.mirror.runs(range.start, range.end) {
                writes.write_buffer(&self.buffer, (first * rec_size) as u64, bytemuck::cast_slice(records));
                self.uploaded += std::mem::size_of_val(records) as u64;
            }
        }
        for r in waiting {
            for k in r {
                self.dirty.mark(k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_grow_without_moving_and_give_contiguous_runs() {
        let mut b = Blocks::new();
        for i in 0..(2 * BLOCK + 10) as u32 {
            b.push(i);
        }
        assert_eq!(b.len(), 2 * BLOCK + 10);
        assert_eq!(b.blocks.len(), 3);
        let first = b.get(0).map(|v| v as *const u32);
        for i in 0..BLOCK as u32 {
            b.push(i);
        }
        // nothing moved
        assert_eq!(b.get(0).map(|v| v as *const u32), first);
        assert_eq!(b.get(BLOCK + 3), Some(&(BLOCK as u32 + 3)));
        assert_eq!(b.get(b.len()), None);
        if let Some(v) = b.get_mut(2 * BLOCK) {
            *v = 7;
        }
        assert_eq!(b.get(2 * BLOCK), Some(&7));
        // a range inside a block: one run
        let runs: Vec<(usize, usize)> = b.runs(10, 20).map(|(at, s)| (at, s.len())).collect();
        assert_eq!(runs, [(10, 10)]);
        // across blocks: cut at the block borders, clamped to the length
        let runs: Vec<(usize, usize)> = b.runs(BLOCK - 64, b.len() + 500).map(|(at, s)| (at, s.len())).collect();
        assert_eq!(runs, [(BLOCK - 64, 64), (BLOCK, BLOCK), (2 * BLOCK, BLOCK), (3 * BLOCK, 10)]);
        let (at, s) = b.runs(BLOCK - 1, BLOCK + 1).nth(1).expect("second run");
        assert_eq!((at, s), (BLOCK, &[BLOCK as u32][..]));
        assert!(b.runs(5, 5).next().is_none());
        assert!(b.runs(b.len(), b.len() + 1).next().is_none());
    }

    #[test]
    fn dirty_chunks_fit_in_blocks() {
        assert!(BLOCK.is_multiple_of(crate::gpu_cull::CHUNK));
    }

    /// GPU round trip of the geometry arena: meshes written by the main
    /// thread and meshes staged as by a job, across a growth of both
    /// arenas (the copies recorded before it must land first), read back
    /// from the arena buffers. Needs a GPU, so not run by check.ps1:
    /// `cargo test -p aurora-render arena_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn arena_round_trip_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut uploads = UploadQueue::new(&device, true);
        let mut writes = GpuWrites::default();
        let mut arena = GeometryArena::new(&device, 1 << 30);
        // a frame as the render thread runs it: the journal in order, then
        // the frame's copies, then the emptied staging chunks mapped again
        let frame = |writes: &mut GpuWrites, uploads: &mut UploadQueue| {
            writes.replay(&mut crate::writes::QueueSink(&queue));
            let mut frame = uploads.take_frame();
            queue.submit(frame.take_commands());
            frame.after_submit();
        };
        let mesh = |i: u32, vcount: u32, icount: u32| {
            let v: Vec<Vertex> = (0..vcount)
                .map(|k| Vertex::new([i as f32, k as f32, 0.5], [0.0, 0.0, 1.0], [k as f32, i as f32]))
                .collect();
            let sk: Vec<SkinVertex> = (0..vcount)
                .map(|k| SkinVertex {
                    joints: [i as u8, k as u8, 0, 0],
                    weights: [255, 0, 0, 0],
                })
                .collect();
            let idx: Vec<u16> = (0..icount).map(|k| (k % vcount) as u16).collect();
            (v, sk, idx)
        };
        // enough vertices and indices to outgrow both arenas (2 M / 8 M)
        let mut live = Vec::new();
        for i in 0..80u32 {
            let (vcount, icount) = (60_000, 300_001 + (i % 2));
            let (v, sk, idx) = mesh(i, vcount, icount);
            let skinned = i % 3 == 0;
            let alloc = if i % 2 == 0 {
                let staged = StagedMesh::new(uploads.pool(), &v, skinned.then_some(&sk[..]), &idx).expect("staging room");
                uploads.prepare();
                assert!(staged.data.ready());
                arena.alloc_staged(&device, &mut writes, &mut uploads, &staged)
            } else {
                arena.alloc(&device, &mut writes, &mut uploads, &v, skinned.then_some(&sk[..]), &idx)
            }
            .expect("room in the arena");
            live.push((i, alloc, skinned));
            if i % 2 == 1 {
                // a frame: the copies are submitted, the chunks come back
                frame(&mut writes, &mut uploads);
                device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            }
        }
        frame(&mut writes, &mut uploads);
        assert!(arena.vertices.capacity() > 2 * 1024 * 1024 && arena.indices.capacity() > 8 * 1024 * 1024);
        let read = |buffer: &wgpu::Buffer, offset: u64, size: u64| -> Vec<u8> {
            let out = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(buffer, offset, &out, 0, size);
            queue.submit([enc.finish()]);
            out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            out.get_mapped_range(..).expect("mapped").to_vec()
        };
        for (i, m, skinned) in live {
            let (v, sk, idx) = mesh(i, m.vertex_count, m.index_count);
            let got = read(
                &arena.vertex_buffer,
                m.vertex_offset as u64 * VERTEX_SIZE,
                m.vertex_count as u64 * VERTEX_SIZE,
            );
            assert!(got == bytemuck::cast_slice::<_, u8>(&v), "mesh {i} vertices");
            if skinned {
                let got = read(
                    &arena.skin_buffer,
                    m.vertex_offset as u64 * SKIN_SIZE,
                    m.vertex_count as u64 * SKIN_SIZE,
                );
                assert!(got == bytemuck::cast_slice::<_, u8>(&sk), "mesh {i} skin");
            }
            // the padded allocation: an odd count repeats its last index
            let got = read(&arena.index_buffer, m.index_offset as u64 * 2, m.index_alloc_len() as u64 * 2);
            let mut padded = idx.clone();
            padded.resize(m.index_alloc_len() as usize, idx[idx.len() - 1]);
            assert!(got == bytemuck::cast_slice::<_, u8>(&padded), "mesh {i} indices");
        }
    }

    /// GPU round trip of the draw records across growths of their buffer:
    /// what the old buffer held is copied on the GPU, the records changed
    /// or added in the same frame are written after it, and every record
    /// reads back as its mirror. Needs a GPU, so not run by check.ps1:
    /// `cargo test -p aurora-render records_grow -- --ignored`.
    #[test]
    #[ignore]
    fn records_grow_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut store = RecordStore::new(&device);
        let mut writes = GpuWrites::default();
        let record = |i: u32| DrawRecord {
            base_color: [i as f32, 1.0, 2.0, 3.0],
            tex: [i, i + 1, i + 2, i + 3],
            ..Default::default()
        };
        let first_size = store.buffer.size();
        let mut ids = Vec::new();
        // three frames: within the first buffer, across one growth, across two
        for count in [10_000u32, 20_000, 70_000] {
            // records of the earlier frames change in the frame of the growth
            for &id in ids.iter().step_by(97) {
                store.set(id, record(id + 1_000_000));
            }
            while (ids.len() as u32) < count {
                let i = ids.len() as u32;
                ids.push(store.alloc(record(i)));
            }
            // a frame: the journal replayed in order (the growth copy
            // between the writes before it and those after), then a submit
            store.flush(&device, &mut writes);
            writes.replay(&mut crate::writes::QueueSink(&queue));
            queue.submit([]);
        }
        assert!(store.buffer.size() >= first_size * 4 && store.generation == 2);
        let size = (ids.len() * std::mem::size_of::<DrawRecord>()) as u64;
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&store.buffer, 0, &out, 0, size);
        queue.submit([enc.finish()]);
        out.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let bytes = out.get_mapped_range(..).expect("mapped").to_vec();
        let got: &[DrawRecord] = bytemuck::cast_slice(&bytes);
        for &id in &ids {
            let expected = store.get(id).copied().expect("record");
            assert!(
                bytemuck::bytes_of(&got[id as usize]) == bytemuck::bytes_of(&expected),
                "record {id}"
            );
        }
        // (and the mirror holds what was set)
        assert_eq!(store.get(97).map(|r| r.tex[0]), Some(1_000_097));
        assert_eq!(store.get(69_999).map(|r| r.tex[0]), Some(69_999));
    }

    /// What the first use of a new buffer costs, by size: the measurement
    /// behind the comment of `GeometryArena`. Needs a GPU, prints its times:
    /// `cargo test -p aurora-render new_buffer_first_use -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn new_buffer_first_use() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        eprintln!("{}", adapter.get_info().name);
        let make = |mb: u64| GeometryArena::make(&device, "test", mb << 20, wgpu::BufferUsages::VERTEX);
        let timed = |work: &dyn Fn()| {
            let t = std::time::Instant::now();
            work();
            queue.submit([]);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            t.elapsed().as_secs_f64() * 1e3
        };
        let source = make(64);
        timed(&|| queue.write_buffer(&source, 0, &[0u8; 4]));
        for mb in [256u64, 512, 1024] {
            let buffer = make(mb);
            let first = timed(&|| queue.write_buffer(&buffer, 0, &[0u8; 4]));
            let second = timed(&|| queue.write_buffer(&buffer, 8, &[0u8; 4]));
            let copy = timed(&|| {
                let mut enc = device.create_command_encoder(&Default::default());
                enc.copy_buffer_to_buffer(&source, 0, &buffer, (mb - 32) << 20, 32 << 20);
                queue.submit([enc.finish()]);
            });
            eprintln!("{mb} MB: first use {first:.2} ms, second {second:.2} ms, 32 MB copied into it {copy:.2} ms");
            assert!(second < first, "the first use of a buffer is the one that costs");
        }
    }

    #[test]
    fn a_full_buffer_doubles_until_the_mesh_fits() {
        let max = 1 << 26;
        assert_eq!(grown_capacity(1000, 10, max), Some(2000));
        // a mesh larger than the buffer: doubled again
        assert_eq!(grown_capacity(1000, 2500, max), Some(4000));
        // capped at the largest buffer, as long as the mesh fits there
        assert_eq!(grown_capacity(max / 2 + 10, 100, max), Some(max));
        assert_eq!(grown_capacity(max - 50, 100, max), None);
        assert_eq!(grown_capacity(max, 1, max), None);
    }

    #[test]
    fn growth_ahead_rounds_to_whole_steps_and_never_shrinks() {
        let step = 1 << 20;
        let max = 64 * step;
        assert_eq!(ahead_capacity(2 * step, step, max), None);
        assert_eq!(ahead_capacity(2 * step, 2 * step, max), None);
        assert_eq!(ahead_capacity(2 * step, 2 * step + 1, max), Some(3 * step));
        assert_eq!(ahead_capacity(2 * step, 20 * step - 7, max), Some(20 * step));
        assert_eq!(ahead_capacity(2 * step, 100 * step, max), Some(max));
        assert_eq!(ahead_capacity(max, 100 * step, max), None);
    }

    #[test]
    fn growth_ahead_stays_within_the_budget_left() {
        const MB: u64 = 1 << 20;
        let per_vertex = VERTEX_SIZE + SKIN_SIZE;
        // the log's case: 2 M → 30.4 M vertices, 8 M → 138 M indices
        let (v, i) = ((2_097_152, 30_408_704), (8_388_608, 138_412_032));
        let whole = 30_408_704 * per_vertex + 138_412_032 * 2;
        assert_eq!(ahead_within(v, i, whole), (v.1, i.1));
        // half of it allowed: a fraction of each growth, within it
        let (nv, ni) = ahead_within(v, i, whole / 2);
        assert!(nv > v.0 && nv < v.1 && ni > i.0 && ni < i.1 && ni % 2 == 0);
        assert!(nv as u64 * per_vertex + ni as u64 * 2 <= whole / 2);
        // nothing left: no growth ahead at all
        assert_eq!(ahead_within(v, i, 0), (v.0, i.0));
        assert_eq!(ahead_within(v, i, 10 * MB), (v.0, i.0));
        // a buffer already large enough costs nothing
        assert_eq!(ahead_within((5, 5), (8, 1000), 2000), (5, 1000));
    }

    #[test]
    fn allocator_coalesces() {
        let mut a = RangeAllocator::new(100);
        let x = a.alloc(10).unwrap();
        let y = a.alloc(20).unwrap();
        let z = a.alloc(30).unwrap();
        assert_eq!((x, y, z), (0, 10, 30));
        a.free(y, 20);
        a.free(x, 10);
        assert_eq!(a.alloc(30), Some(0));
        a.free(0, 30);
        a.free(z, 30);
        assert_eq!(a.alloc(100), Some(0));
        assert_eq!(a.alloc(1), None);
        a.grow(200);
        assert_eq!(a.alloc(100), Some(100));
    }

    /// Free ranges of an allocator, by offset, checked against its index
    /// by length.
    fn free_ranges(a: &RangeAllocator) -> Vec<(u32, u32)> {
        let by_len: BTreeSet<(u32, u32)> = a.free.iter().map(|(&off, &len)| (len, off)).collect();
        assert_eq!(by_len, a.by_len, "index by length out of step");
        a.free.iter().map(|(&off, &len)| (off, len)).collect()
    }

    #[test]
    fn allocator_takes_the_smallest_range_that_fits() {
        let mut a = RangeAllocator::new(1000);
        let blocks: Vec<u32> = [100, 50, 100, 20, 100, 50, 100].iter().map(|&l| a.alloc(l).unwrap()).collect();
        assert_eq!(blocks, [0, 100, 150, 250, 270, 370, 420]);
        // holes of 50, 20 and 50 between kept blocks, and the tail of 480
        a.free(100, 50);
        a.free(250, 20);
        a.free(370, 50);
        assert_eq!(free_ranges(&a), [(100, 50), (250, 20), (370, 50), (520, 480)]);
        // the smallest that fits, not the first
        assert_eq!(a.alloc(20), Some(250));
        // equal sizes: the lowest offset; the rest of the hole stays free
        assert_eq!(a.alloc(30), Some(100));
        assert_eq!(free_ranges(&a), [(130, 20), (370, 50), (520, 480)]);
        assert_eq!(a.alloc(60), Some(520));
        assert_eq!(a.alloc(1000), None);
        assert_eq!(a.used(), 100 * 4 + 20 + 30 + 60);
    }

    #[test]
    fn allocator_stays_consistent_under_churn() {
        let mut state = 7u64;
        let mut rnd = |m: u32| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 33) as u32) % m
        };
        let mut a = RangeAllocator::new(50_000);
        let mut live: Vec<(u32, u32)> = Vec::new();
        for step in 0..5000 {
            if rnd(3) > 0 || live.is_empty() {
                let len = 1 + rnd(200);
                if let Some(off) = a.alloc(len) {
                    live.push((off, len));
                }
            } else {
                let (off, len) = live.swap_remove(rnd(live.len() as u32) as usize);
                a.free(off, len);
            }
            if step % 250 == 0 {
                // no overlap between live blocks and free ranges, nothing lost
                let mut all: Vec<(u32, u32)> = live.iter().copied().chain(free_ranges(&a)).collect();
                all.sort_unstable();
                let mut at = 0;
                for (off, len) in all {
                    assert_eq!(off, at, "gap or overlap at step {step}");
                    at = off + len;
                }
                assert_eq!(at, a.capacity());
                assert_eq!(a.used(), live.iter().map(|l| l.1).sum::<u32>());
                // free neighbours are always merged
                let free = free_ranges(&a);
                assert!(free.windows(2).all(|w| w[0].0 + w[0].1 < w[1].0));
            }
        }
    }
}
