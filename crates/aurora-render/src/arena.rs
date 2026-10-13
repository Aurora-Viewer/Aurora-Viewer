//! GPU memory arenas: one big vertex buffer, one big u16 index buffer and
//! a storage buffer of draw records, all sub-allocated on the CPU side.
//! The geometry buffers grow ahead of need, a slice per frame (`Pool`).

use crate::gpu_cull::{CullFace, DirtyChunks, GpuTable};
use crate::types::{DrawRecord, SkinVertex, Vertex};
use crate::upload::{StagedMesh, UploadQueue};
use crate::writes::GpuWrites;
use std::collections::{BTreeMap, BTreeSet};

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

/// Share of a pool in use that starts its growth: early enough for the move
/// to end, at [`COPY_PER_FRAME`], long before the pool is full.
const GROW_AT: (u64, u64) = (7, 8);

/// Bytes of a pool's widest stream moved to its larger buffers each frame.
/// A buffer is copied by the GPU, which draws nothing meanwhile: in one go,
/// the 768 MB of vertices of a crowded region held a frame for 14 ms (8 ms
/// for 384 MB). 32 MB is about half a millisecond.
const COPY_PER_FRAME: u64 = 32 * 1024 * 1024;

/// AURORA_ARENA_GROWTH=whole: a pool only grows once it is full, to twice
/// its size, copied at once, as before — to compare.
fn grows_at_once() -> bool {
    static WHOLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WHOLE.get_or_init(|| std::env::var("AURORA_ARENA_GROWTH").is_ok_and(|v| v.trim() == "whole"))
}

/// Capacity a pool of `cap` elements grows to, with room for `need` more:
/// half as much again (it grows before it is full, so doubling would leave
/// it less than half used; `double`: twice, the growth at once), an even
/// count, `max` at most. `None` when `max` cannot hold `need` more.
fn grown(cap: u32, need: u32, max: u32, double: bool) -> Option<u32> {
    let larger = if double { cap as u64 * 2 } else { cap as u64 * 3 / 2 };
    let want = larger.max(cap as u64 + need as u64 + 1) & !1;
    let new = want.min(max as u64 & !1) as u32;
    (new > cap && new - cap >= need).then_some(new)
}

/// One buffer of a pool: `stride` bytes per element.
struct Stream {
    buffer: wgpu::Buffer,
    stride: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

/// Larger buffers a pool is moving to: the first `copied` elements are in
/// them, like everything written since the move started.
struct Move {
    buffers: Vec<wgpu::Buffer>,
    capacity: u32,
    copied: u32,
}

/// Parallel buffers sub-allocated together (the vertices and their skin
/// stream; the indices). A draw reads one buffer, so a pool that is full
/// has to move to a larger one. It starts early (`GROW_AT`) and copies a
/// slice per frame (`advance`) while it goes on serving its old capacity;
/// what is written meanwhile goes to both sets of buffers. The frame the
/// last slice goes, the larger buffers take over. Only an allocation that
/// finds no room before that (a burst, or no hole wide enough) copies the
/// rest at once, as every growth used to.
struct Pool {
    streams: Vec<Stream>,
    ranges: RangeAllocator,
    moving: Option<Move>,
    /// Elements the largest buffer allowed holds.
    max: u32,
    /// Growths refused for lack of room, to say so once.
    full: bool,
}

impl Pool {
    fn new(device: &wgpu::Device, capacity: u32, max_buffer_size: u64, streams: &[(&'static str, u64, wgpu::BufferUsages)]) -> Pool {
        let widest = streams.iter().map(|s| s.1).max().unwrap_or(1);
        Pool {
            streams: streams
                .iter()
                .map(|&(label, stride, usage)| Stream {
                    buffer: make_buffer(device, label, capacity as u64 * stride, usage),
                    stride,
                    label,
                    usage,
                })
                .collect(),
            ranges: RangeAllocator::new(capacity),
            moving: None,
            max: (max_buffer_size / widest).min(u32::MAX as u64) as u32,
            full: false,
        }
    }

    fn bytes(&self) -> u64 {
        let held: u64 = self.streams.iter().map(|s| s.buffer.size()).sum();
        held + self.moving.iter().flat_map(|m| &m.buffers).map(|b| b.size()).sum::<u64>()
    }

    /// Create the larger buffers; the copy follows, slice by slice.
    fn start(&mut self, device: &wgpu::Device, need: u32) -> bool {
        let Some(capacity) = grown(self.ranges.capacity(), need, self.max, grows_at_once()) else {
            if !self.full {
                self.full = true;
                log::warn!(
                    "{} cannot grow beyond {} MB, the largest buffer of this GPU: the meshes it has no room for are not drawn",
                    self.streams[0].label,
                    self.streams[0].buffer.size() / (1024 * 1024)
                );
            }
            return false;
        };
        self.moving = Some(Move {
            buffers: self
                .streams
                .iter()
                .map(|s| make_buffer(device, s.label, capacity as u64 * s.stride, s.usage))
                .collect(),
            capacity,
            copied: 0,
        });
        true
    }

    /// Copy up to `elements` more to the larger buffers, which take over
    /// once everything is there. Recorded after the writes and the copies
    /// already made into the old buffers this frame.
    fn step(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, elements: u32) {
        let Some(m) = &mut self.moving else {
            return;
        };
        let old = self.ranges.capacity();
        // (even: the indices are two bytes and a copy counts in fours)
        let count = (elements.max(2) & !1).min(old - m.copied);
        if count > 0 {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("arena growth") });
            for (s, to) in self.streams.iter().zip(&m.buffers) {
                let at = m.copied as u64 * s.stride;
                enc.copy_buffer_to_buffer(&s.buffer, at, to, at, count as u64 * s.stride);
            }
            uploads.flush(writes, Some(enc));
            m.copied += count;
        }
        if m.copied < old {
            return;
        }
        let Some(m) = self.moving.take() else {
            return;
        };
        for (s, to) in self.streams.iter_mut().zip(m.buffers) {
            s.buffer = to;
        }
        self.ranges.grow(m.capacity);
        let s = &self.streams[0];
        log::info!("{} grown to {} MB", s.label, s.buffer.size() / (1024 * 1024));
    }

    /// The frame's slice of a move under way.
    fn advance(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue) {
        let widest = self.streams.iter().map(|s| s.stride).max().unwrap_or(1);
        self.step(device, writes, uploads, (COPY_PER_FRAME / widest) as u32);
    }

    /// A range of `len` elements. Starts the pool's growth when it is
    /// nearly full; without room now, ends the move under way at once, then
    /// grows on the spot as a last resort.
    fn alloc(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, len: u32) -> Option<u32> {
        let mut at = self.ranges.alloc(len);
        if at.is_none() && self.moving.is_some() {
            self.step(device, writes, uploads, u32::MAX);
            at = self.ranges.alloc(len);
        }
        if at.is_none() && self.start(device, len) {
            self.step(device, writes, uploads, u32::MAX);
            at = self.ranges.alloc(len);
        }
        let (used, capacity) = (self.ranges.used() as u64, self.ranges.capacity() as u64);
        if at.is_some() && self.moving.is_none() && used * GROW_AT.1 >= capacity * GROW_AT.0 && !grows_at_once() {
            self.start(device, 0);
        }
        at
    }

    /// Buffers a write to stream `i` goes to: the one drawn from, and the
    /// larger one while the pool moves.
    fn targets(&self, i: usize) -> impl Iterator<Item = &wgpu::Buffer> {
        std::iter::once(&self.streams[i].buffer).chain(self.moving.iter().map(move |m| &m.buffers[i]))
    }

    /// Write elements of stream `i` from the main thread's memory.
    fn write(&self, writes: &mut GpuWrites, i: usize, at: u32, data: &[u8]) {
        for buffer in self.targets(i) {
            writes.write_buffer(buffer, at as u64 * self.streams[i].stride, data);
        }
    }

    /// Copy elements of stream `i` from a staged region.
    fn copy_staged(&self, device: &wgpu::Device, uploads: &mut UploadQueue, staged: &crate::upload::Staged, range: (u64, u64), i: usize, at: u32) {
        for buffer in self.targets(i) {
            uploads.copy_to_buffer(device, staged, range, buffer, at as u64 * self.streams[i].stride);
        }
    }
}

fn make_buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

/// Streams of the vertex pool.
const VERTICES: usize = 0;
const SKIN: usize = 1;

pub struct GeometryArena {
    /// Vertices, and the parallel stream of `SkinVertex` (same element
    /// offsets).
    vertices: Pool,
    indices: Pool,
}

const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
const SKIN_SIZE: u64 = std::mem::size_of::<SkinVertex>() as u64;

impl GeometryArena {
    pub fn new(device: &wgpu::Device, max_buffer_size: u64) -> Self {
        let vcap: u32 = 2 * 1024 * 1024; // 48 MB
        let icap: u32 = 8 * 1024 * 1024; // 16 MB
        Self {
            vertices: Pool::new(
                device,
                vcap,
                max_buffer_size,
                &[
                    ("vertex arena", VERTEX_SIZE, wgpu::BufferUsages::VERTEX),
                    ("skin arena", SKIN_SIZE, wgpu::BufferUsages::VERTEX),
                ],
            ),
            indices: Pool::new(device, icap, max_buffer_size, &[("index arena", 2, wgpu::BufferUsages::INDEX)]),
        }
    }

    /// The buffers the frame draws from.
    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertices.streams[VERTICES].buffer
    }

    pub fn skin_buffer(&self) -> &wgpu::Buffer {
        &self.vertices.streams[SKIN].buffer
    }

    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.indices.streams[0].buffer
    }

    /// Video memory held, the larger buffers of a move under way included.
    pub fn bytes(&self) -> u64 {
        self.vertices.bytes() + self.indices.bytes()
    }

    pub fn used(&self) -> (u64, u64) {
        (self.vertices.ranges.used() as u64, self.indices.ranges.used() as u64)
    }

    /// Once a frame, after its uploads: the slice of each pool that is
    /// moving to larger buffers.
    pub fn advance(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue) {
        self.vertices.advance(device, writes, uploads);
        self.indices.advance(device, writes, uploads);
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
        let voff = self.vertices.alloc(device, writes, uploads, vcount)?;
        let Some(ioff) = self.indices.alloc(device, writes, uploads, ialloc) else {
            self.vertices.ranges.free(voff, vcount);
            return None;
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
        self.vertices.write(writes, VERTICES, voff, bytemuck::cast_slice(vertices));
        if let Some(sk) = skin
            && sk.len() == vertices.len()
        {
            self.vertices.write(writes, SKIN, voff, bytemuck::cast_slice(sk));
        }
        if icount % 2 == 1 {
            let mut padded = Vec::with_capacity(m.index_alloc_len() as usize);
            padded.extend_from_slice(indices);
            padded.push(*indices.last().unwrap_or(&0));
            self.indices.write(writes, 0, ioff, bytemuck::cast_slice(&padded));
        } else {
            self.indices.write(writes, 0, ioff, bytemuck::cast_slice(indices));
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
        self.vertices.copy_staged(device, uploads, &mesh.data, vertices, VERTICES, m.vertex_offset);
        self.vertices.copy_staged(device, uploads, &mesh.data, skin, SKIN, m.vertex_offset);
        self.indices.copy_staged(device, uploads, &mesh.data, indices, 0, m.index_offset);
        Some(m)
    }

    /// New vertices for a mesh of the arena (as many as it holds).
    pub fn write_vertices(&self, writes: &mut GpuWrites, m: MeshAlloc, vertices: &[Vertex]) {
        self.vertices.write(writes, VERTICES, m.vertex_offset, bytemuck::cast_slice(vertices));
    }

    pub fn free(&mut self, m: MeshAlloc) {
        self.vertices.ranges.free(m.vertex_offset, m.vertex_count);
        self.indices.ranges.free(m.index_offset, m.index_alloc_len());
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
        }
    }

    fn make(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw records"),
            size: (cap * std::mem::size_of::<DrawRecord>()) as u64,
            // COPY_SRC: a grown buffer takes over this one's records by a GPU copy
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
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
        if needed as u64 > self.buffer.size() {
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
            let grown = Self::make(device, cap);
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("grow draw records"),
            });
            enc.copy_buffer_to_buffer(&self.buffer, 0, &grown, 0, old_size);
            writes.submit(vec![enc]);
            self.buffer = grown;
            self.generation += 1;
        }
        if !self.dirty.any() {
            return;
        }
        for range in self.dirty.take_ranges(self.mirror.len()) {
            for (first, records) in self.mirror.runs(range.start, range.end) {
                writes.write_buffer(&self.buffer, (first * rec_size) as u64, bytemuck::cast_slice(records));
                self.uploaded += std::mem::size_of_val(records) as u64;
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
        let frame = |arena: &mut GeometryArena, writes: &mut GpuWrites, uploads: &mut UploadQueue| {
            arena.advance(&device, writes, uploads);
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
        let (mut moves, mut deformed) = (0usize, None);
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
            live.push((i, i, alloc, skinned));
            moves += usize::from(arena.vertices.moving.is_some());
            if arena.vertices.moving.as_ref().is_some_and(|m| m.copied > 0) && deformed.is_none() {
                // a mesh already copied to the larger buffers gets new
                // vertices while the move goes on: both sets must hold them
                let (_, _, first, _) = live[0];
                let (v, _, _) = mesh(1000, first.vertex_count, 6);
                arena.write_vertices(&mut writes, first, &v);
                live[0].0 = 1000;
                deformed = Some(i);
            }
            if i % 2 == 1 {
                // a frame: the copies are submitted, the chunks come back
                frame(&mut arena, &mut writes, &mut uploads);
                device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            }
        }
        frame(&mut arena, &mut writes, &mut uploads);
        assert!(arena.vertices.ranges.capacity() > 2 * 1024 * 1024 && arena.indices.ranges.capacity() > 8 * 1024 * 1024);
        assert!(moves > 2 && deformed.is_some(), "the vertex pool moved over several frames ({moves}), a mesh deformed meanwhile");
        // a move still under way does not hide anything: the buffers drawn
        // from hold every mesh
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
        for (shape, i, m, skinned) in live {
            let (v, _, _) = mesh(shape, m.vertex_count, 6);
            let (_, sk, idx) = mesh(i, m.vertex_count, m.index_count);
            let got = read(
                arena.vertex_buffer(),
                m.vertex_offset as u64 * VERTEX_SIZE,
                m.vertex_count as u64 * VERTEX_SIZE,
            );
            assert!(got == bytemuck::cast_slice::<_, u8>(&v), "mesh {i} vertices");
            if skinned {
                let got = read(
                    arena.skin_buffer(),
                    m.vertex_offset as u64 * SKIN_SIZE,
                    m.vertex_count as u64 * SKIN_SIZE,
                );
                assert!(got == bytemuck::cast_slice::<_, u8>(&sk), "mesh {i} skin");
            }
            // the padded allocation: an odd count repeats its last index
            let got = read(arena.index_buffer(), m.index_offset as u64 * 2, m.index_alloc_len() as u64 * 2);
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

    #[test]
    fn pools_grow_by_half_within_the_largest_buffer() {
        assert_eq!(grown(2_000_000, 0, u32::MAX, false), Some(3_000_000));
        assert_eq!(grown(2_000_000, 0, u32::MAX, true), Some(4_000_000));
        // room for what is asked, an even count
        assert_eq!(grown(1_000, 5_001, u32::MAX, false), Some(6_002));
        assert_eq!(grown(1_001, 0, u32::MAX, false), Some(1_500));
        // up to the largest buffer, then no further
        assert_eq!(grown(80_000_000, 0, 89_478_485, false), Some(89_478_484));
        assert_eq!(grown(89_478_484, 0, 89_478_485, false), None);
        assert_eq!(grown(80_000_000, 20_000_000, 89_478_485, true), None);
        // a pool starts growing with an eighth of it still free, which a
        // frame's slice covers many times over
        const { assert!(GROW_AT.0 < GROW_AT.1 && COPY_PER_FRAME >= 8 * 1024 * 1024) };
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
