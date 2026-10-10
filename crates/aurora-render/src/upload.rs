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
//! The submit is the render thread's (render_thread.rs): the main thread
//! hands the frame's encoder over with the chunks that hold nothing more to
//! copy ([`UploadQueue::take_frame`]); they are mapped again only after
//! that frame is submitted ([`FrameUploads::after_submit`]), never while a
//! recorded copy still reads them.
//!
//! The chunks are all created with the renderer, on the main thread: a job
//! never allocates GPU memory (wgpu's allocator lock held by a job creating
//! a chunk made a page creation on the main thread wait 35 ms). When they
//! are all in use, a job waits for one to come back (a fraction of a
//! second at most); after that, or for data larger than a chunk,
//! [`StagingPool::stage`] returns None and the caller keeps its data for a
//! `write_*` by the main thread, within its time budget.
//!
//! Chunk life: open (jobs allocate and write) → closed at the next
//! [`UploadQueue::prepare`] → unmapped when no job writes in it any more
//! (its staged data can be copied from then on) → taken out with the frame
//! that follows the drop of its last [`Staged`] region, mapping again
//! after that frame's submit → free.

use crate::types::{SkinVertex, Vertex};
use crate::writes::GpuWrites;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Chunks of the pool and their size: 8 × 32 MB (a 2048² texture with its
/// mips is 22 MB), or 4 × 16 MB on a GPU with little memory.
const CHUNKS: (usize, u64) = (8, 32 << 20);
const CHUNKS_SMALL: (usize, u64) = (4, 16 << 20);
/// When every chunk is in use, a job waits this long for one to come back
/// (they do within a few frames) before keeping its data for a main-thread
/// write: under a burst (thousands of textures decoded at once) the jobs
/// follow the pace of the GPU copies instead of leaving their memcpy to
/// the main thread. The wait ends by itself when nothing is being drawn.
const STAGE_WAIT: Duration = Duration::from_millis(100);
/// A chunk with data stays open at most this many frames.
const OPEN_FRAMES: u32 = 8;
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
    /// Frames (`prepare` calls) since it was opened.
    frames: u32,
}

#[derive(Default)]
struct PoolState {
    open: Option<Open>,
    /// Mapped and empty.
    free: Vec<Arc<Chunk>>,
    /// Closed: waiting to be unmapped, then recycled.
    closed: Vec<Arc<Chunk>>,
}

/// Chunks whose mapping completed (and whether it succeeded), and the
/// signal for the jobs waiting for one.
type Returned = Arc<(Mutex<Vec<(Arc<Chunk>, bool)>>, Condvar)>;

/// Mapped staging memory shared by the background jobs and the renderer.
pub struct StagingPool {
    state: Mutex<PoolState>,
    /// Memory of the pool (bytes) and size of its chunks.
    total: u64,
    chunk_bytes: u64,
    /// Chunks mapped again (pushed by the `map_async` callbacks, which
    /// take no other lock).
    returned: Returned,
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
    /// `small`: a GPU with little memory (integrated, or under 4 GB).
    pub fn new(device: &wgpu::Device, small: bool) -> Arc<Self> {
        let (count, chunk_bytes) = if small { CHUNKS_SMALL } else { CHUNKS };
        let free = (0..count)
            .map(|_| {
                Arc::new(Chunk {
                    buffer: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("streaming staging"),
                        size: chunk_bytes,
                        usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: true,
                    }),
                    writers: AtomicU32::new(0),
                    staged: AtomicU32::new(0),
                    unmapped: AtomicBool::new(false),
                })
            })
            .collect();
        Arc::new(StagingPool {
            state: Mutex::new(PoolState {
                open: None,
                free,
                closed: Vec::new(),
            }),
            total: count as u64 * chunk_bytes,
            chunk_bytes,
            returned: Default::default(),
            write_turn: Mutex::new(()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolState> {
        // a panic in a job while holding the lock leaves plain data behind
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The main thread never waits for the pool: a job (which runs at a
    /// lower priority) may hold the lock; what `prepare` or
    /// `take_recyclable` would have done is done at the next frame.
    fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, PoolState>> {
        match self.state.try_lock() {
            Ok(s) => Some(s),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// Take back the chunks whose mapping completed.
    fn receive(&self, s: &mut PoolState) {
        let returned = std::mem::take(&mut *self.returned.0.lock().unwrap_or_else(|e| e.into_inner()));
        for (chunk, ok) in returned {
            if ok {
                chunk.unmapped.store(false, Ordering::Release);
                s.free.push(chunk);
            } else {
                // never expected; the pool goes on with one chunk less
                log::warn!("staging: a chunk could not be mapped again");
            }
        }
    }

    /// Reserve `size` bytes, write them with `fill` (from any thread) and
    /// return the region. Waits up to [`STAGE_WAIT`] for a chunk when all
    /// are in use; None after that or when `size` exceeds a chunk (the
    /// caller then keeps its data for a main-thread write).
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
        if size > self.chunk_bytes {
            return None;
        }
        let deadline = Instant::now() + STAGE_WAIT;
        loop {
            {
                let mut s = self.lock();
                self.receive(&mut s);
                if s.open.as_ref().is_some_and(|o| o.offset + size <= o.chunk.size()) {
                    return Self::bump(&mut s, size);
                }
                // the next free chunk; with none, the open one stays open
                // for smaller requests
                if let Some(chunk) = s.free.pop() {
                    if let Some(o) = s.open.replace(Open {
                        chunk,
                        offset: 0,
                        frames: 0,
                    }) {
                        s.closed.push(o.chunk);
                    }
                    return Self::bump(&mut s, size);
                }
            }
            // every chunk is in use: wait for one to come back
            let (returned, came_back) = &*self.returned;
            let list = returned.lock().unwrap_or_else(|e| e.into_inner());
            if list.is_empty() {
                let left = deadline.checked_duration_since(Instant::now())?;
                let (list, _) = came_back.wait_timeout(list, left).unwrap_or_else(|e| e.into_inner());
                if list.is_empty() && Instant::now() >= deadline {
                    return None;
                }
            }
        }
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
    /// unmap the closed chunks nobody writes in any more. While closed
    /// chunks still hold data to copy, the open one is left to fill up to
    /// half (a chunk closed every frame with little in it would exhaust
    /// the pool), a few frames at most.
    fn prepare(&self) {
        let Some(mut s) = self.try_lock() else {
            return;
        };
        let busy = s.closed.iter().any(|c| c.staged.load(Ordering::Acquire) > 0);
        let chunk_bytes = self.chunk_bytes;
        if let Some(o) = s.open.as_mut() {
            o.frames += 1;
        }
        if let Some(o) = s
            .open
            .take_if(|o| o.offset > 0 && (!busy || o.offset >= chunk_bytes / 2 || o.frames > OPEN_FRAMES))
        {
            s.closed.push(o.chunk);
        }
        for c in &s.closed {
            if !c.unmapped.load(Ordering::Acquire) && c.writers.load(Ordering::Acquire) == 0 {
                c.buffer.unmap();
                c.unmapped.store(true, Ordering::Release);
            }
        }
    }

    /// Main thread, when the frame's copies are handed over: the unmapped
    /// chunks with no region left. Every copy out of them is recorded by
    /// now (a copy needs a live region, and a closed chunk gets no new
    /// one), in this frame's encoders or earlier ones; they are mapped
    /// again by [`FrameUploads::after_submit`], once those are submitted.
    fn take_recyclable(&self) -> Vec<Arc<Chunk>> {
        let Some(mut s) = self.try_lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < s.closed.len() {
            let c = &s.closed[i];
            if c.unmapped.load(Ordering::Acquire) && c.staged.load(Ordering::Acquire) == 0 {
                out.push(s.closed.swap_remove(i));
            } else {
                i += 1;
            }
        }
        out
    }

    /// Map a recyclable chunk again; it comes back free through `returned`.
    fn map_again(returned: &Returned, chunk: Arc<Chunk>) {
        let returned = returned.clone();
        let back = chunk.clone();
        chunk.buffer.map_async(wgpu::MapMode::Write, .., move |r| {
            returned.0.lock().unwrap_or_else(|e| e.into_inner()).push((back, r.is_ok()));
            returned.1.notify_all();
        });
    }

    /// Staging memory of the pool (bytes).
    pub fn bytes(&self) -> u64 {
        self.total
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

/// A mesh staged by a background job: its vertices, its skin stream when
/// rigged, and its u16 indices padded to an even count (as the arena
/// allocates them), ready for [`crate::Renderer::upload_mesh_staged`].
#[derive(Debug)]
pub struct StagedMesh {
    pub vertex_count: u32,
    pub index_count: u32,
    pub skinned: bool,
    pub data: Staged,
}

impl StagedMesh {
    /// Stage a mesh; None when the pool is full or the mesh is empty (the
    /// caller keeps its data for `upload_mesh`).
    pub fn new(pool: &StagingPool, vertices: &[Vertex], skin: Option<&[SkinVertex]>, indices: &[u16]) -> Option<Self> {
        if vertices.is_empty() || indices.is_empty() || vertices.len() > u32::MAX as usize / 2 {
            return None;
        }
        let skin = skin.filter(|s| s.len() == vertices.len());
        let v: &[u8] = bytemuck::cast_slice(vertices);
        let sk: &[u8] = skin.map(bytemuck::cast_slice).unwrap_or_default();
        let i: &[u8] = bytemuck::cast_slice(indices);
        let odd = indices.len() % 2 == 1;
        let size = (v.len() + sk.len() + i.len() + if odd { 2 } else { 0 }) as u64;
        let data = pool.stage(size, |dst| {
            dst.slice(..v.len()).copy_from_slice(v);
            dst.slice(v.len()..v.len() + sk.len()).copy_from_slice(sk);
            let at = v.len() + sk.len();
            dst.slice(at..at + i.len()).copy_from_slice(i);
            if odd {
                // the padding repeats the last index
                dst.slice(at + i.len()..at + i.len() + 2).copy_from_slice(&i[i.len() - 2..]);
            }
        })?;
        Some(StagedMesh {
            vertex_count: vertices.len() as u32,
            index_count: indices.len() as u32,
            skinned: skin.is_some(),
            data,
        })
    }

    /// Byte ranges (offset, size) of the vertices, the skin stream and the
    /// padded indices inside the staged region.
    pub(crate) fn ranges(&self) -> [(u64, u64); 3] {
        mesh_ranges(self.vertex_count, self.index_count, self.skinned)
    }
}

fn mesh_ranges(vertex_count: u32, index_count: u32, skinned: bool) -> [(u64, u64); 3] {
    let v = vertex_count as u64 * std::mem::size_of::<Vertex>() as u64;
    let sk = if skinned {
        vertex_count as u64 * std::mem::size_of::<SkinVertex>() as u64
    } else {
        0
    };
    let i = ((index_count as u64 + 1) & !1) * 2;
    [(0, v), (v, sk), (v + sk, i)]
}

/// Per-frame copies of the staged data, recorded by the main thread and
/// submitted first in the frame's submit.
pub struct UploadQueue {
    pool: Arc<StagingPool>,
    encoder: Option<wgpu::CommandEncoder>,
}

impl UploadQueue {
    pub fn new(device: &wgpu::Device, small: bool) -> Self {
        UploadQueue {
            pool: StagingPool::new(device, small),
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

    /// Copy `size` bytes at `at` of a staged region into a buffer (offsets
    /// and size multiples of 4).
    pub fn copy_to_buffer(&mut self, device: &wgpu::Device, staged: &Staged, (at, size): (u64, u64), dst: &wgpu::Buffer, dst_offset: u64) {
        if size == 0 {
            return;
        }
        let src = staged.buffer().clone();
        let offset = staged.offset + at;
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

    /// Submit the recorded copies at this point of the frame, followed by
    /// `then` (an arena growth or another copy that must see them): recorded
    /// in the frame's journal, after the writes made so far.
    pub fn flush(&mut self, writes: &mut GpuWrites, then: Option<wgpu::CommandEncoder>) {
        writes.submit(self.encoder.take().into_iter().chain(then).collect());
    }

    /// End of the frame on the main thread: the copies recorded since the
    /// last flush, to submit before the passes that read them, and the
    /// staging chunks to map again once they are submitted.
    pub fn take_frame(&mut self) -> FrameUploads {
        FrameUploads {
            encoder: self.encoder.take(),
            recycle: self.pool.take_recyclable(),
            returned: self.pool.returned.clone(),
        }
    }
}

/// The streamed copies of one frame, handed to whoever submits the frame.
pub struct FrameUploads {
    encoder: Option<wgpu::CommandEncoder>,
    recycle: Vec<Arc<Chunk>>,
    returned: Returned,
}

impl FrameUploads {
    /// The recorded copies: first command buffer of the frame's submit.
    pub fn take_commands(&mut self) -> Option<wgpu::CommandBuffer> {
        self.encoder.take().map(|e| e.finish())
    }

    /// After the submit that consumed [`Self::take_commands`] (and the
    /// journal's own submits before it): the emptied chunks are mapped
    /// again and come back to the pool.
    pub fn after_submit(self) {
        debug_assert!(self.encoder.is_none(), "staged copies dropped without a submit");
        for chunk in self.recycle {
            StagingPool::map_again(&self.returned, chunk);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GPU round trip of the staging pool: regions staged from several
    /// threads at once, copied into a buffer and read back, over enough
    /// rounds that every chunk is closed, unmapped, recycled and written
    /// again. Needs a GPU, so not run by check.ps1:
    /// `cargo test -p aurora-render staging_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn staging_round_trip_on_the_gpu() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let mut uploads = UploadQueue::new(&device, true);
        let pool = uploads.pool().clone();
        assert_eq!(pool.bytes(), 4 * (16 << 20));
        // larger than a chunk: the caller keeps its data
        assert!(pool.stage((16 << 20) + 4, |_| {}).is_none());
        const REGION: usize = 3 << 20;
        const PER_ROUND: usize = 8;
        let dst = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (REGION * PER_ROUND) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let pattern = |round: usize, i: usize| -> Vec<u8> { (0..REGION).map(|b| (b * 7 + i * 31 + round * 101) as u8).collect() };
        // 12 rounds of 24 MB through a 64 MB pool: chunks must come back
        for round in 0..12 {
            let staged: Vec<Staged> = std::thread::scope(|s| {
                let jobs: Vec<_> = (0..PER_ROUND)
                    .map(|i| {
                        let pool = &pool;
                        s.spawn(move || {
                            let data = pattern(round, i);
                            pool.stage(REGION as u64, |dst| dst.copy_from_slice(&data))
                                .expect("room in the pool")
                        })
                    })
                    .collect();
                jobs.into_iter().map(|j| j.join().expect("job")).collect()
            });
            uploads.prepare();
            if staged.iter().any(|r| !r.ready()) {
                // a chunk left open while others hold data: closed next frame
                for _ in 0..=OPEN_FRAMES {
                    uploads.prepare();
                }
            }
            for (i, region) in staged.iter().enumerate() {
                assert!(region.ready(), "round {round} region {i}");
                uploads.copy_to_buffer(&device, region, (0, REGION as u64), &dst, (i * REGION) as u64);
            }
            drop(staged);
            // a frame: the copies are submitted, the emptied chunks mapped again
            let mut frame = uploads.take_frame();
            queue.submit(frame.take_commands());
            frame.after_submit();
            dst.map_async(wgpu::MapMode::Read, .., |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            {
                let got = dst.get_mapped_range(..).expect("mapped");
                for i in 0..PER_ROUND {
                    assert!(
                        got[i * REGION..(i + 1) * REGION] == pattern(round, i)[..],
                        "round {round} region {i}"
                    );
                }
            }
            dst.unmap();
        }
    }

    #[test]
    fn mesh_ranges_follow_each_other() {
        // 3 vertices of 24 bytes, a skin stream of 8, 5 indices padded to 6
        assert_eq!(mesh_ranges(3, 5, true), [(0, 72), (72, 24), (96, 12)]);
        assert_eq!(mesh_ranges(3, 6, false), [(0, 72), (72, 0), (72, 12)]);
        // every offset and size suits a buffer copy
        for r in mesh_ranges(7, 9, true) {
            assert!(r.0 % wgpu::COPY_BUFFER_ALIGNMENT == 0 && r.1 % wgpu::COPY_BUFFER_ALIGNMENT == 0);
        }
    }

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
