//! Streaming uploads without the copy on the main thread.
//!
//! `Queue::write_texture` / `write_buffer` copy the data into a staging
//! buffer they create for the call, under the queue's lock, on the calling
//! thread: for a busy region's textures that was ~9 GB/s of memcpy on the
//! main thread (24 MB, the old per-frame cap, took 2.5–6 ms, plus a
//! staging buffer per mip level).
//!
//! Here the background jobs that decode textures and build geometry write
//! the data themselves into persistently mapped staging chunks
//! ([`StagingPool::stage`]); the main thread only records the GPU copies
//! into one upload encoder ([`UploadQueue`]), submitted first in the
//! frame's submit, before the passes that sample the new texels. Chunks are
//! recycled: once closed, unmapped (no job still writing), their copies
//! submitted and nothing staged in them still pending, they are mapped
//! again (`map_async`) and come back empty.
//!
//! Chunk life: open (jobs allocate and write) → closed at the next
//! [`UploadQueue::prepare`] → unmapped when no job writes in it any more
//! (its staged data can be copied from then on) → mapping again after the
//! submit that follows the drop of its last [`Staged`] region → free.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Size of a regular chunk; a larger request gets a chunk of its own size.
const CHUNK_BYTES: u64 = 16 << 20;
/// Staging memory cap (host memory the GPU reads). Past it, `stage` returns
/// None and the caller keeps its data for the main thread's `write_*`.
const POOL_MAX_BYTES: u64 = 256 << 20;
/// Offsets inside a chunk (and rows of texture levels) are aligned to this:
/// wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`, also enough for buffer copies.
pub const STAGING_ALIGN: u64 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;

fn align(v: u64) -> u64 {
    v.div_ceil(STAGING_ALIGN) * STAGING_ALIGN
}

struct Chunk {
    buffer: wgpu::Buffer,
    /// Jobs writing in this chunk right now (counted at allocation, under
    /// the pool lock, released when their view is dropped).
    writers: AtomicU32,
    /// `Staged` regions of this chunk not dropped yet.
    staged: AtomicU32,
    /// Closed and unmapped: its regions can be copied.
    unmapped: AtomicBool,
}

impl Chunk {
    fn size(&self) -> u64 {
        self.buffer.size()
    }
}

/// An open chunk and its next free offset.
struct Open {
    chunk: Arc<Chunk>,
    offset: u64,
}

#[derive(Default)]
struct PoolState {
    open: Option<Open>,
    /// Mapped and empty.
    free: Vec<Arc<Chunk>>,
    /// Closed: waiting to be unmapped, then recycled.
    closed: Vec<Arc<Chunk>>,
    /// Memory of every chunk alive (free, open, closed or mapping).
    total: u64,
}

/// Mapped staging memory shared by the background jobs and the renderer.
pub struct StagingPool {
    device: wgpu::Device,
    state: Mutex<PoolState>,
    /// Chunks mapped again (pushed by the `map_async` callbacks, which
    /// take no other lock).
    returned: Arc<Mutex<Vec<(Arc<Chunk>, bool)>>>,
    /// Held by the job writing into staging memory (see `stage`).
    write_turn: Mutex<()>,
}

/// A region written in a staging chunk; the chunk is not recycled while
/// one of its regions is alive.
pub struct Staged {
    chunk: Arc<Chunk>,
    offset: u64,
    size: u64,
}

impl Staged {
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The data can be copied: its chunk is closed and unmapped.
    pub fn ready(&self) -> bool {
        self.chunk.unmapped.load(Ordering::Acquire)
    }

    fn buffer(&self) -> &wgpu::Buffer {
        &self.chunk.buffer
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.chunk.staged.fetch_sub(1, Ordering::AcqRel);
    }
}

impl std::fmt::Debug for Staged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Staged({} bytes at {})", self.size, self.offset)
    }
}

impl StagingPool {
    pub fn new(device: &wgpu::Device) -> Arc<Self> {
        Arc::new(StagingPool {
            device: device.clone(),
            state: Mutex::new(PoolState::default()),
            returned: Default::default(),
            write_turn: Mutex::new(()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolState> {
        // a panic in a job while holding the lock leaves plain data behind
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take back the chunks whose mapping completed.
    fn receive(&self, s: &mut PoolState) {
        let returned = std::mem::take(&mut *self.returned.lock().unwrap_or_else(|e| e.into_inner()));
        for (chunk, ok) in returned {
            if ok {
                chunk.unmapped.store(false, Ordering::Release);
                s.free.push(chunk);
            } else {
                s.total = s.total.saturating_sub(chunk.size());
            }
        }
    }

    /// Reserve `size` bytes, write them with `fill` (from any thread) and
    /// return the region; None when the pool is at its cap (the caller then
    /// keeps its data for a main-thread write).
    pub fn stage(&self, size: u64, fill: impl FnOnce(&mut wgpu::WriteOnly<'_, [u8]>)) -> Option<Staged> {
        let size = align(size.max(4));
        let (chunk, offset) = self.allocate(size)?;
        {
            let view = chunk.buffer.get_mapped_range_mut(offset..offset + size);
            // One job writes at a time: staging memory is write-combined,
            // and 30 jobs streaming into it at once stalled every thread of
            // the process, the main one included (a 16 ms frame per wave of
            // textures in AURORA_DEMO_STREAM). One writer still moves
            // ~9 GB/s, far more than a region streams.
            let _turn = self.write_turn.lock().unwrap_or_else(|e| e.into_inner());
            match view {
                Ok(mut view) => fill(&mut view.slice(..)),
                Err(e) => {
                    // never expected: the chunk is mapped while open
                    log::warn!("staging: mapped range unavailable: {e}");
                    chunk.writers.fetch_sub(1, Ordering::AcqRel);
                    chunk.staged.fetch_sub(1, Ordering::AcqRel);
                    return None;
                }
            }
        }
        chunk.writers.fetch_sub(1, Ordering::AcqRel);
        Some(Staged { chunk, offset, size })
    }

    fn allocate(&self, size: u64) -> Option<(Arc<Chunk>, u64)> {
        let mut s = self.lock();
        self.receive(&mut s);
        if !s.open.as_ref().is_some_and(|o| o.offset + size <= o.chunk.size()) {
            if let Some(o) = s.open.take() {
                s.closed.push(o.chunk);
            }
            let chunk = match s.free.iter().position(|c| c.size() >= size) {
                Some(i) => s.free.swap_remove(i),
                None => {
                    let bytes = size.max(CHUNK_BYTES);
                    // drop free chunks too small for this request to make
                    // room, else give up
                    while s.total + bytes > POOL_MAX_BYTES {
                        let c = s.free.pop()?;
                        s.total -= c.size();
                    }
                    s.total += bytes;
                    // created without the lock: the main thread's
                    // `prepare` must not wait behind an allocation
                    drop(s);
                    let chunk = Arc::new(Chunk {
                        buffer: self.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("streaming staging"),
                            size: bytes,
                            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: true,
                        }),
                        writers: AtomicU32::new(0),
                        staged: AtomicU32::new(0),
                        unmapped: AtomicBool::new(false),
                    });
                    s = self.lock();
                    // another job may have opened a chunk meanwhile: this
                    // one waits in the free list
                    if s.open.as_ref().is_some_and(|o| o.offset + size <= o.chunk.size()) {
                        s.free.push(chunk);
                    } else {
                        if let Some(o) = s.open.take() {
                            s.closed.push(o.chunk);
                        }
                        s.open = Some(Open { chunk, offset: 0 });
                    }
                    return Self::bump(&mut s, size);
                }
            };
            s.open = Some(Open { chunk, offset: 0 });
        }
        Self::bump(&mut s, size)
    }

    /// A region of the open chunk (which has room for it).
    fn bump(s: &mut PoolState, size: u64) -> Option<(Arc<Chunk>, u64)> {
        let o = s.open.as_mut()?;
        let offset = o.offset;
        o.offset += size;
        o.chunk.writers.fetch_add(1, Ordering::AcqRel);
        o.chunk.staged.fetch_add(1, Ordering::AcqRel);
        Some((o.chunk.clone(), offset))
    }

    /// Main thread, before copying: close the open chunk (what the jobs
    /// staged so far becomes copyable once they are done writing) and
    /// unmap the closed chunks nobody writes in any more.
    fn prepare(&self) {
        let mut s = self.lock();
        if let Some(o) = s.open.take_if(|o| o.offset > 0) {
            s.closed.push(o.chunk);
        }
        for c in &s.closed {
            if !c.unmapped.load(Ordering::Acquire) && c.writers.load(Ordering::Acquire) == 0 {
                c.buffer.unmap();
                c.unmapped.store(true, Ordering::Release);
            }
        }
    }

    /// Main thread, right after a submit that consumed every recorded copy:
    /// map again the unmapped chunks with no region left.
    fn recycle(&self) {
        let mut s = self.lock();
        let mut i = 0;
        while i < s.closed.len() {
            let c = &s.closed[i];
            if c.unmapped.load(Ordering::Acquire) && c.staged.load(Ordering::Acquire) == 0 {
                let chunk = s.closed.swap_remove(i);
                let returned = self.returned.clone();
                let back = chunk.clone();
                chunk.buffer.map_async(wgpu::MapMode::Write, .., move |r| {
                    returned.lock().unwrap_or_else(|e| e.into_inner()).push((back, r.is_ok()));
                });
            } else {
                i += 1;
            }
        }
    }

    /// Staging memory allocated (bytes).
    pub fn bytes(&self) -> u64 {
        self.lock().total
    }
}

/// Layout of a mip chain in staging memory: rows padded to
/// [`STAGING_ALIGN`], each level at an aligned offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StagedLevel {
    pub width: u32,
    pub height: u32,
    pub offset: u64,
    pub bytes_per_row: u32,
}

/// Levels of `dims` (RGBA8) and the total size.
pub fn level_layout(dims: impl IntoIterator<Item = (u32, u32)>) -> (Vec<StagedLevel>, u64) {
    let mut offset = 0;
    let levels = dims
        .into_iter()
        .map(|(width, height)| {
            let bytes_per_row = align(width as u64 * 4) as u32;
            let l = StagedLevel {
                width,
                height,
                offset,
                bytes_per_row,
            };
            offset += align(bytes_per_row as u64 * height as u64);
            l
        })
        .collect();
    (levels, offset)
}

/// Write tightly packed RGBA8 levels into `dst` at their staged layout.
pub fn write_levels(dst: &mut wgpu::WriteOnly<'_, [u8]>, levels: &[StagedLevel], data: &[&[u8]]) {
    for (l, src) in levels.iter().zip(data) {
        let row = l.width as usize * 4;
        if row == l.bytes_per_row as usize {
            let len = row * l.height as usize;
            dst.slice(l.offset as usize..l.offset as usize + len).copy_from_slice(&src[..len]);
            continue;
        }
        for y in 0..l.height as usize {
            let at = l.offset as usize + y * l.bytes_per_row as usize;
            dst.slice(at..at + row).copy_from_slice(&src[y * row..(y + 1) * row]);
        }
    }
}

/// A mip chain staged by a background job, ready to be copied into a
/// texture layer ([`crate::Renderer::replace_texture_staged`]).
#[derive(Debug)]
pub struct StagedTexture {
    pub levels: Vec<StagedLevel>,
    pub data: Staged,
}

impl StagedTexture {
    /// Stage tightly packed RGBA8 levels (level 0 first); None when the pool
    /// is full.
    pub fn new(pool: &StagingPool, mips: &[(u32, u32, &[u8])]) -> Option<Self> {
        let usable = mips.iter().all(|(w, h, d)| d.len() >= *w as usize * *h as usize * 4);
        if mips.is_empty() || !usable {
            return None;
        }
        let (levels, size) = level_layout(mips.iter().map(|m| (m.0, m.1)));
        let data: Vec<&[u8]> = mips.iter().map(|m| m.2).collect();
        let staged = pool.stage(size, |dst| write_levels(dst, &levels, &data))?;
        Some(StagedTexture { levels, data: staged })
    }

    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|l| l.width as u64 * l.height as u64 * 4).sum()
    }
}

/// Per-frame copies of the staged data, recorded by the main thread and
/// submitted first in the frame's submit.
pub struct UploadQueue {
    pool: Arc<StagingPool>,
    encoder: Option<wgpu::CommandEncoder>,
}

impl UploadQueue {
    pub fn new(device: &wgpu::Device) -> Self {
        UploadQueue {
            pool: StagingPool::new(device),
            encoder: None,
        }
    }

    pub fn pool(&self) -> &Arc<StagingPool> {
        &self.pool
    }

    /// Before recording copies this frame.
    pub fn prepare(&self) {
        self.pool.prepare();
    }

    /// The upload encoder of this frame.
    pub fn encoder(&mut self, device: &wgpu::Device) -> &mut wgpu::CommandEncoder {
        self.encoder.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("streaming uploads"),
            })
        })
    }

    /// Copy a staged region into a buffer (4-byte aligned offset and size).
    pub fn copy_to_buffer(&mut self, device: &wgpu::Device, staged: &Staged, dst: &wgpu::Buffer, dst_offset: u64, size: u64) {
        let src = staged.buffer().clone();
        let offset = staged.offset;
        self.encoder(device).copy_buffer_to_buffer(&src, offset, dst, dst_offset, size);
    }

    /// Copy staged levels into `layer` of a texture array, level `i` of the
    /// chain into mip `i`.
    pub fn copy_to_layer(&mut self, device: &wgpu::Device, staged: &StagedTexture, texture: &wgpu::Texture, layer: u32, count: u32) {
        let src = staged.data.buffer().clone();
        let base = staged.data.offset;
        let enc = self.encoder(device);
        for (i, l) in staged.levels.iter().take(count as usize).enumerate() {
            enc.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &src,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: base + l.offset,
                        bytes_per_row: Some(l.bytes_per_row),
                        rows_per_image: Some(l.height),
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: i as u32,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: l.width,
                    height: l.height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// The recorded copies, to submit before anything that reads them.
    pub fn finish(&mut self) -> Option<wgpu::CommandBuffer> {
        self.encoder.take().map(|e| e.finish())
    }

    /// Submit the recorded copies now, followed by `then` (an arena growth
    /// or another copy that must see them).
    pub fn flush(&mut self, queue: &wgpu::Queue, then: Option<wgpu::CommandBuffer>) {
        let cmds: Vec<wgpu::CommandBuffer> = self.finish().into_iter().chain(then).collect();
        if !cmds.is_empty() {
            queue.submit(cmds);
        }
    }

    /// After the frame's submit (which consumed every recorded copy).
    pub fn recycle(&self) {
        self.pool.recycle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_aligned() {
        let (levels, size) = level_layout([(64, 32), (32, 16), (16, 8), (1, 1)]);
        // 64 px rows are 256 bytes: tight; smaller rows are padded
        assert_eq!(levels[0].bytes_per_row, 256);
        assert_eq!(levels[1].offset, 256 * 32);
        assert_eq!(levels[1].bytes_per_row, 256);
        assert_eq!(levels[2].offset, 256 * 32 + 256 * 16);
        assert_eq!(levels[3].bytes_per_row, 256);
        assert_eq!(size, levels[3].offset + 256);
        assert!(levels.iter().all(|l| l.offset % STAGING_ALIGN == 0));
        let (big, size) = level_layout([(1024, 512)]);
        assert_eq!((big[0].bytes_per_row, size), (4096, 4096 * 512));
    }

    #[test]
    fn levels_are_written_row_by_row() {
        let l0: Vec<u8> = (0..64 * 2 * 4).map(|i| i as u8).collect();
        let l1: Vec<u8> = (0..4u8).map(|i| 200 + i).collect();
        let (levels, size) = level_layout([(64, 2), (1, 1)]);
        let mut out = vec![0u8; size as usize];
        write_levels(&mut wgpu::WriteOnly::from(out.as_mut_slice()), &levels, &[&l0, &l1]);
        assert_eq!(&out[..512], &l0[..]);
        assert_eq!(&out[512..516], &l1[..]);
        assert!(out[516..].iter().all(|&b| b == 0));
        // a padded level: 2 px rows of 8 bytes in 256-byte rows
        let px: Vec<u8> = (1..=16).collect();
        let (levels, size) = level_layout([(2, 2)]);
        let mut out = vec![0u8; size as usize];
        write_levels(&mut wgpu::WriteOnly::from(out.as_mut_slice()), &levels, &[&px]);
        assert_eq!(&out[..8], &px[..8]);
        assert_eq!(&out[256..264], &px[8..]);
        assert!(out[8..256].iter().all(|&b| b == 0));
    }
}
