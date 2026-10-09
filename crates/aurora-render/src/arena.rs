//! GPU memory arenas: one big vertex buffer, one big u16 index buffer and
//! a storage buffer of draw records, all sub-allocated on the CPU side.

use crate::types::{DrawRecord, SkinVertex, Vertex};
use std::collections::BTreeMap;

/// First-fit range allocator with coalescing (units are elements).
#[derive(Debug)]
pub struct RangeAllocator {
    /// offset -> length
    free: BTreeMap<u32, u32>,
    capacity: u32,
    used: u32,
}

impl RangeAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        Self { free, capacity, used: 0 }
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
        let (&off, &flen) = self.free.iter().find(|(_, l)| **l >= len)?;
        self.free.remove(&off);
        if flen > len {
            self.free.insert(off + len, flen - len);
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
            self.free.remove(&poff);
            start = poff;
            length += plen;
        }
        // merge with next
        if let Some(&nlen) = self.free.get(&(off + len)) {
            self.free.remove(&(off + len));
            length += nlen;
        }
        self.free.insert(start, length);
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

pub struct GeometryArena {
    pub vertex_buffer: wgpu::Buffer,
    /// Parallel stream of `SkinVertex` (same element offsets as vertices).
    pub skin_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    vertices: RangeAllocator,
    indices: RangeAllocator,
    max_buffer_size: u64,
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
        }
    }

    fn make(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    pub fn bytes(&self) -> u64 {
        self.vertex_buffer.size() + self.skin_buffer.size() + self.index_buffer.size()
    }

    pub fn used(&self) -> (u64, u64) {
        (self.vertices.used() as u64, self.indices.used() as u64)
    }

    fn grow_vertices(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, need: u32) -> bool {
        let old = self.vertices.capacity();
        let mut new_cap = old.saturating_mul(2);
        while new_cap - old < need {
            new_cap = new_cap.saturating_mul(2);
        }
        if new_cap as u64 * VERTEX_SIZE > self.max_buffer_size {
            new_cap = (self.max_buffer_size / VERTEX_SIZE) as u32;
            if new_cap <= old || new_cap - old < need {
                return false;
            }
        }
        let nb = Self::make(device, "vertex arena", new_cap as u64 * VERTEX_SIZE, wgpu::BufferUsages::VERTEX);
        let ns = Self::make(device, "skin arena", new_cap as u64 * SKIN_SIZE, wgpu::BufferUsages::VERTEX);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow vb") });
        enc.copy_buffer_to_buffer(&self.vertex_buffer, 0, &nb, 0, old as u64 * VERTEX_SIZE);
        enc.copy_buffer_to_buffer(&self.skin_buffer, 0, &ns, 0, old as u64 * SKIN_SIZE);
        queue.submit([enc.finish()]);
        self.vertex_buffer = nb;
        self.skin_buffer = ns;
        self.vertices.grow(new_cap);
        log::info!("vertex arena grown to {} MB", new_cap as u64 * VERTEX_SIZE / (1024 * 1024));
        true
    }

    fn grow_indices(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, need: u32) -> bool {
        let old = self.indices.capacity();
        let mut new_cap = old.saturating_mul(2);
        while new_cap - old < need {
            new_cap = new_cap.saturating_mul(2);
        }
        if new_cap as u64 * 2 > self.max_buffer_size {
            new_cap = (self.max_buffer_size / 2) as u32 & !1;
            if new_cap <= old || new_cap - old < need {
                return false;
            }
        }
        let nb = Self::make(device, "index arena", new_cap as u64 * 2, wgpu::BufferUsages::INDEX);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grow ib") });
        enc.copy_buffer_to_buffer(&self.index_buffer, 0, &nb, 0, old as u64 * 2);
        queue.submit([enc.finish()]);
        self.index_buffer = nb;
        self.indices.grow(new_cap);
        log::info!("index arena grown to {} MB", new_cap as u64 * 2 / (1024 * 1024));
        true
    }

    /// Upload a mesh. Indices are relative to the mesh's first vertex.
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        skin: Option<&[SkinVertex]>,
        indices: &[u16],
    ) -> Option<MeshAlloc> {
        if vertices.is_empty() || indices.is_empty() || vertices.len() > u32::MAX as usize / 2 {
            return None;
        }
        let vcount = vertices.len() as u32;
        let icount = indices.len() as u32;
        let ialloc = (icount + 1) & !1;
        let voff = match self.vertices.alloc(vcount) {
            Some(o) => o,
            None => {
                if !self.grow_vertices(device, queue, vcount) {
                    return None;
                }
                self.vertices.alloc(vcount)?
            }
        };
        let ioff = match self.indices.alloc(ialloc) {
            Some(o) => o,
            None => {
                if !self.grow_indices(device, queue, ialloc) {
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
        queue.write_buffer(&self.vertex_buffer, voff as u64 * VERTEX_SIZE, bytemuck::cast_slice(vertices));
        if let Some(sk) = skin
            && sk.len() == vertices.len()
        {
            queue.write_buffer(&self.skin_buffer, voff as u64 * SKIN_SIZE, bytemuck::cast_slice(sk));
        }
        if icount % 2 == 1 {
            let mut padded = Vec::with_capacity(ialloc as usize);
            padded.extend_from_slice(indices);
            padded.push(*indices.last().unwrap_or(&0));
            queue.write_buffer(&self.index_buffer, ioff as u64 * 2, bytemuck::cast_slice(&padded));
        } else {
            queue.write_buffer(&self.index_buffer, ioff as u64 * 2, bytemuck::cast_slice(indices));
        }
        Some(MeshAlloc {
            vertex_offset: voff,
            vertex_count: vcount,
            index_offset: ioff,
            index_count: icount,
        })
    }

    pub fn free(&mut self, m: MeshAlloc) {
        self.vertices.free(m.vertex_offset, m.vertex_count);
        self.indices.free(m.index_offset, m.index_alloc_len());
    }
}

/// Records per dirty chunk: small enough that scattered updates (worn
/// attachments, avatars) do not drag big untouched ranges along; neighbours
/// are still uploaded in one write.
const CHUNK: usize = 64;

/// Storage buffer of `DrawRecord`s with a CPU mirror and chunked dirty uploads.
pub struct RecordStore {
    pub buffer: wgpu::Buffer,
    mirror: Vec<DrawRecord>,
    free: Vec<u32>,
    dirty: Vec<bool>,
    any_dirty: bool,
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
            mirror: Vec::with_capacity(cap),
            free: Vec::new(),
            dirty: Vec::new(),
            any_dirty: false,
            live: 0,
            generation: 0,
            uploaded: 0,
        }
    }

    fn make(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw records"),
            size: (cap * std::mem::size_of::<DrawRecord>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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
            return id;
        }
        let id = self.mirror.len() as u32;
        self.mirror.push(rec);
        self.mark(id as usize);
        id
    }

    fn mark(&mut self, idx: usize) {
        let c = idx / CHUNK;
        if self.dirty.len() <= c {
            self.dirty.resize(c + 1, false);
        }
        self.dirty[c] = true;
        self.any_dirty = true;
    }

    pub fn set(&mut self, id: u32, rec: DrawRecord) {
        if let Some(slot) = self.mirror.get_mut(id as usize) {
            *slot = rec;
            self.mark(id as usize);
        }
    }

    /// Replace a record's model matrix only when it differs (moving objects:
    /// no copy of the whole record, and an unchanged one is not uploaded).
    pub fn set_model(&mut self, id: u32, model: [[f32; 4]; 4]) {
        if let Some(slot) = self.mirror.get_mut(id as usize)
            && slot.model != model
        {
            slot.model = model;
            self.mark(id as usize);
        }
    }

    pub fn get(&self, id: u32) -> Option<&DrawRecord> {
        self.mirror.get(id as usize)
    }

    pub fn free(&mut self, id: u32) {
        if (id as usize) < self.mirror.len() {
            self.live = self.live.saturating_sub(1);
            self.free.push(id);
        }
    }

    /// Upload dirty chunks; reallocates the buffer when it is too small.
    pub fn flush(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let rec_size = std::mem::size_of::<DrawRecord>();
        self.uploaded = 0;
        let needed = self.mirror.len() * rec_size;
        if needed as u64 > self.buffer.size() {
            let mut cap = self.buffer.size() as usize / rec_size;
            while cap * rec_size < needed {
                cap *= 2;
            }
            self.buffer = Self::make(device, cap);
            self.generation += 1;
            // everything must be re-uploaded
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.mirror));
            self.uploaded = needed as u64;
            self.dirty.iter_mut().for_each(|d| *d = false);
            self.any_dirty = false;
            return;
        }
        if !self.any_dirty {
            return;
        }
        let mut c = 0;
        while c < self.dirty.len() {
            if !self.dirty[c] {
                c += 1;
                continue;
            }
            // coalesce consecutive dirty chunks
            let start = c;
            while c < self.dirty.len() && self.dirty[c] {
                self.dirty[c] = false;
                c += 1;
            }
            let a = start * CHUNK;
            let b = (c * CHUNK).min(self.mirror.len());
            if a < b {
                queue.write_buffer(&self.buffer, (a * rec_size) as u64, bytemuck::cast_slice(&self.mirror[a..b]));
                self.uploaded += ((b - a) * rec_size) as u64;
            }
        }
        self.any_dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
