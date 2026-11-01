//! Bindless texture table, pooled into texture pages.
//!
//! Draw records reference textures by slot, a stable `u32`. Textures of the
//! same size and mip count share one `texture_2d_array` page, one layer
//! each; the shader reads the slot's packed (page, layer) location from a
//! storage buffer (`tex_loc`) and samples that layer of
//! `binding_array<texture_2d_array<f32>>[page]`.
//!
//! Why pages: wgpu-core merges every element of a bind group into the usage
//! scope of each pass that binds it (then again at pass end and at submit),
//! so with one texture per element `encoder.finish()` grew with the number
//! of textures (~9,400 in a busy region, several ms a frame). Pages bring the
//! array down to a few hundred elements, and streaming only rewrites entries
//! of the location buffer: the bind group changes only when a page is
//! created or dropped.
//!
//! The image is unchanged: a layer holds the same texels and mip chain as
//! the standalone texture did, and is sampled with the same sampler and the
//! same implicit derivatives.
//!
//! Pages are created and destroyed off the frame's threads, by the page
//! thread ([`PageWorker`]): the streamer asks before each upload whether
//! its layer is there ([`TextureTable::prepare_page`]), and the page of a
//! size is made ahead of the uploads queued for it. Textures created by a
//! direct call (media, ban lines, the reserved slots) still create their
//! page on the spot when they need one.

use crate::upload::{StagedTexture, TexelFormat, UploadQueue};
use crate::writes::{GpuWrites, TexelDst};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::num::{NonZeroU32, NonZeroU64};
use std::time::{Duration, Instant};

/// Reserved slots.
pub const WHITE: u32 = 0;
pub const FLAT_NORMAL: u32 = 1;
pub const BLACK: u32 = 2;
pub const TRANSPARENT: u32 = 3;
/// The grey texel shown by textures still loading. Their slots point at
/// this one layer ([`TextureTable::create_placeholder`]) instead of each
/// getting a 1×1 layer written through the queue: a texture met for the
/// first time cost ~1.3 µs of `write_texture` in the scene sync, 16 ms for
/// the 12,000 of a region arrival.
pub const PLACEHOLDER: u32 = 4;
/// The grey of a texture that will not come (missing on the asset server,
/// failing again and again): LLViewerFetchedTexture::sDefaultImagep, the
/// 0x7f grey the Legacy Viewer binds instead (llviewertexture.cpp). A layer
/// of its own, so that the shader tells it from the loading grey
/// (`rez_still_grey` in object.wgsl): such a face is no rez grid for ever.
pub const MISSING: u32 = 5;
/// IMG_ALPHA_GRAD_2D, used by the region terrain's height/noise blend.
pub const TERRAIN_RAMP: u32 = 6;
/// Decoded top-down L8 pixels from skins/default/textures/alpha_gradient_2d.j2c
/// (Copyright Linden Research, Inc., originally LGPL 2.1). Kept linear.
pub const TERRAIN_RAMP_PIXELS: &[u8; 256 * 256] = include_bytes!("terrain_alpha_ramp.r8");

/// LLTerrainPaintMap's height/noise bake, using the same bilinear 2D ramp
/// and 0.1 layer threshold as pbrterrainF.glsl. Noise is in GL coordinates.
pub fn terrain_blend_weights(c: f32, noise: f32) -> [f32; 4] {
    let ramp = |u: f32| {
        let x = (u * 256.0 - 0.5).clamp(0.0, 255.0);
        let y = ((1.0 - noise) * 256.0 - 0.5).clamp(0.0, 255.0);
        let (ix, iy) = (x as usize, y as usize);
        let texel = |x: usize, y: usize| f32::from(TERRAIN_RAMP_PIXELS[y * 256 + x]) / 255.0;
        let row = |y| texel(ix, y) + (texel((ix + 1).min(255), y) - texel(ix, y)) * (x - ix as f32);
        row(iy) + (row((iy + 1).min(255)) - row(iy)) * (y - iy as f32)
    };
    let (a, b, f) = (ramp(c), ramp(c - 2.0), ramp(c - 1.0));
    let w = [f * a, f * (1.0 - a), (1.0 - f) * b, (1.0 - f) * (1.0 - b)].map(|v| (v - 0.1).max(0.0));
    let sum: f32 = w.iter().sum();
    w.map(|v| v / sum)
}
const RESERVED: u32 = 7;
/// The white texture is the first one created: page 0, layer 0. Freed slots
/// point there, so a stale draw record samples white, never a dropped page.
const WHITE_LOC: u32 = 0;

/// The first page of a size holds about this much (at least one layer), and
/// each further page half as much as the pages of that size already hold,
/// up to `PAGE_MAX_BYTES`: a size seen once wastes little, a common size
/// ends in large pages (few array elements), and the free layers of the
/// newest page stay under a third of the memory of that size. Past 32 MB,
/// fewer elements gain little while each size could leave that much unused.
const PAGE_FIRST_BYTES: u64 = 2 << 20;
const PAGE_MAX_BYTES: u64 = 32 << 20;
/// A location packs the page above `LAYER_BITS` and the layer below
/// (`sample_tex` in common.wgsl).
const LAYER_BITS: u32 = 16;
const MAX_LAYERS: u32 = 2048;
const MAX_PAGES: u32 = 1 << (32 - LAYER_BITS);
/// Slots of the table. A slot is an entry of the location buffer (4 bytes),
/// not an element of the bound array: only the pages count against what the
/// device lets a shader bind. While each texture was an element of its own
/// the two were one number, at most 16,384, and the table kept that size
/// once textures went to pages: arriving in a busy region (9,000 textures)
/// 45 s or less after leaving another, the last 1,500 textures found no
/// slot and were drawn white for as long as they were used. Four times
/// that: more than the video memory holds textures for.
pub const SLOTS: u32 = 1 << 16;
const MAX_SLOTS: u32 = 1 << 20;

/// Slots of the table: [`SLOTS`], or what AURORA_TEXTURE_SLOTS asks for (to
/// test a full table).
pub fn slot_capacity(forced: Option<&str>) -> u32 {
    forced
        .and_then(|v| v.trim().parse::<u32>().ok())
        .map_or(SLOTS, |n| n.clamp(RESERVED + 1, MAX_SLOTS))
}

/// Pages the table can hold, from the elements the device lets a shader
/// bind in one array.
fn page_capacity(bindable: u32, partial: bool) -> u32 {
    if partial { bindable } else { bindable.min(UNPARTIAL_PAGES) }.clamp(1, MAX_PAGES)
}
/// Without partially bound arrays every element of the page array is bound
/// in every pass (holes repeat page 0), so the array is kept shorter.
const UNPARTIAL_PAGES: u32 = 2048;
/// Rebuilds for dropped pages only are throttled; a new page is bound before
/// the next draw.
const REBUILD_THROTTLE: Duration = Duration::from_millis(100);
/// Compaction: at most one page a second is emptied into the other pages of
/// its size, when at most a quarter of its layers are used and the copy is
/// small. Without it, pages left nearly empty after textures are freed (a
/// teleport, a crowd leaving) would keep their memory.
const COMPACT_INTERVAL: Duration = Duration::from_secs(1);
const COMPACT_MAX_BYTES: u64 = 16 << 20;
/// Under memory pressure (`TextureTable::set_eager_compaction`) a page is
/// emptied this often, whatever its fill: at most 32 MB of GPU copies each
/// time, 640 MB a second, so the pages the lowered textures left are given
/// back within seconds without the copies weighing on the frames.
const COMPACT_EAGER_INTERVAL: Duration = Duration::from_millis(50);
/// Emptied pages are handed to the page thread one every this long (and
/// at least one, at most [`PAGES_DESTROYED_PER_FRAME`], a frame). Once the
/// bind group no longer holds a page, dropping it frees its video memory:
/// ~0.4 ms each on the thread that drops it. The pages a rebuild released
/// went together (1 to 4 ms in the frames where textures move to a finer
/// level), and the ~300 pages of a region left by teleport made one
/// submit of 18 to 50 ms. Paced, because the render thread meets the
/// page thread on the device's memory allocator at each queue write.
/// With the frames drawn by the render thread (render_thread.rs), the frame
/// in flight still holds the bind group of before the rebuild in its
/// packet: handles are reference counted, so a page handed to the page thread
/// meanwhile lives until that frame ends (it is then freed there, by the
/// render thread, never under a draw that reads it).
const PAGE_DESTROY_INTERVAL: Duration = Duration::from_millis(4);
const PAGES_DESTROYED_PER_FRAME: usize = 8;

/// Pages to hand to the page thread `since_last` after the previous ones.
fn pages_to_destroy(waiting: usize, since_last: Duration) -> usize {
    let due = (since_last.as_micros() / PAGE_DESTROY_INTERVAL.as_micros()) as usize;
    waiting.min(due.clamp(1, PAGES_DESTROYED_PER_FRAME))
}

/// A page made for the streamed uploads and not used after this long is
/// destroyed (the textures that asked for it were freed meanwhile).
const SPARE_TTL: Duration = Duration::from_secs(2);
/// A page asked from the page thread and not there after this long is
/// created by the caller instead (one takes about a millisecond).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

type DeadPage = (wgpu::Texture, wgpu::TextureView);

/// A page created by the page thread, not in the table yet.
struct MadePage {
    key: PageKey,
    layers: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Time its creation took on the page thread.
    ms: f32,
}

enum PageJob {
    Make { key: PageKey, layers: u32 },
    Destroy(DeadPage),
}

/// Creates the pages of the streamed uploads and destroys the emptied ones
/// on a thread of its own (`aurora-pages`). Most pages are carved out of a
/// block of video memory the device already holds (a few µs), but the one
/// that needs a new block (128 then 256 MB each) waits for the driver to
/// allocate it, under the lock of the device's memory allocator: 1.0 to
/// 1.6 ms on the main thread in the streaming demo, at times 3 to 10 ms on
/// the grid with the video memory loaded. Smaller blocks
/// (`MemoryHints::Manual`, 64 MB) were tried and not kept: each costs less
/// (longest page creation 1.3 → 0.55 ms alone), but they come more often,
/// and in the viewer the render thread's `r_resources` step got longer at
/// worst (1.3 → 4.8 ms, median of 3 passes of the streaming demo; the
/// blocks of the staging memory of its queue writes shrink with them).
///
/// The render thread still meets the page thread on that lock (each of its
/// queue writes allocates a staging buffer), for the time of the block at
/// most; the main thread no longer does.
struct PageWorker {
    /// None when the thread could not start: pages are then created and
    /// dropped by the caller.
    tx: Option<std::sync::mpsc::Sender<PageJob>>,
    made: std::sync::mpsc::Receiver<MadePage>,
}

impl PageWorker {
    fn new(device: &wgpu::Device) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<PageJob>();
        let (made_tx, made) = std::sync::mpsc::channel::<MadePage>();
        let device = device.clone();
        let spawned = std::thread::Builder::new().name("aurora-pages".into()).spawn(move || {
            for job in rx {
                // (the render thread can wait for it: frame_priority.rs)
                crate::frame_priority::apply();
                match job {
                    PageJob::Make { key, layers } => {
                        let t0 = Instant::now();
                        let (texture, view) = match make_page(&device, key, layers) {
                            Ok(page) => page,
                            Err(e) => {
                                // (nothing comes back: once the request times
                                // out, the frame's thread tries again itself)
                                log::debug!("texture page not made by the page thread: {e}");
                                continue;
                            }
                        };
                        let ms = t0.elapsed().as_secs_f32() * 1000.0;
                        // (the table is gone: the page is dropped here)
                        let _ = made_tx.send(MadePage {
                            key,
                            layers,
                            texture,
                            view,
                            ms,
                        });
                    }
                    PageJob::Destroy(page) => drop(page),
                }
            }
        });
        match spawned {
            Ok(_) => PageWorker { tx: Some(tx), made },
            Err(e) => {
                log::warn!("texture pages: page thread not started ({e}); pages are created and destroyed by the frame's threads");
                PageWorker { tx: None, made }
            }
        }
    }

    /// Ask for a page; false when the thread is not there to make it.
    fn make(&self, key: PageKey, layers: u32) -> bool {
        self.tx.as_ref().is_some_and(|tx| tx.send(PageJob::Make { key, layers }).is_ok())
    }

    fn destroy(&self, page: DeadPage) {
        if let Some(tx) = &self.tx {
            // (a page that cannot be sent comes back in the error and is
            // dropped here)
            let _ = tx.send(PageJob::Destroy(page));
        }
    }
}

/// What to do about a page the streamed uploads wait for.
#[derive(Debug, PartialEq, Eq)]
enum Ask {
    /// Ask the page thread for it.
    Send,
    /// Already asked: wait.
    Wait,
    /// Asked too long ago: the caller creates it.
    GiveUp,
}

/// Pages asked from the page thread and not received yet, one per size.
#[derive(Default)]
struct PageRequests {
    asked: HashMap<PageKey, Instant>,
}

impl PageRequests {
    fn ask(&mut self, key: PageKey, now: Instant) -> Ask {
        match self.asked.get(&key) {
            None => {
                self.asked.insert(key, now);
                Ask::Send
            }
            Some(&since) if now.saturating_duration_since(since) < REQUEST_TIMEOUT => Ask::Wait,
            Some(_) => {
                // (if it still comes, it is a spare page like another)
                self.asked.remove(&key);
                Ask::GiveUp
            }
        }
    }

    fn done(&mut self, key: PageKey) {
        self.asked.remove(&key);
    }
}

/// A page that took long to create (video memory under pressure?) is worth
/// a line: a hitch when the main thread created it, and on the page thread
/// the render thread may have waited for the allocator's lock meanwhile.
fn log_slow_page(key: PageKey, layers: u32, ms: f32, by: &str) {
    if ms > 4.0 {
        log::info!(
            "texture page {}×{} ({} layers, {} MB) created in {ms:.1} ms by the {by}",
            key.width,
            key.height,
            layers,
            (layers as u64 * key.layer_bytes()) >> 20
        );
    }
}

/// A page, or why video memory could not hold it.
fn make_page(device: &wgpu::Device, key: PageKey, layers: u32) -> Result<DeadPage, String> {
    let desc = wgpu::TextureDescriptor {
        label: Some("texture page"),
        size: wgpu::Extent3d {
            width: key.width,
            height: key.height,
            depth_or_array_layers: layers,
        },
        mip_level_count: key.mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: key.format.wgpu(),
        // COPY_SRC: compaction copies layers between pages
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    // explicit: the default view of a one-layer texture would be D2
    let view = wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    };
    crate::gpu_alloc::try_texture(device, &desc, &view, layers as u64 * key.layer_bytes())
}

const NO_SLOT: u32 = u32::MAX;

/// Mip level data, tightly packed RGBA8.
pub struct MipLevel<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

/// What makes textures share a page: level-0 size, mip count and texel
/// format (RGBA8, or BC7 for the streamed textures that compress well).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PageKey {
    width: u32,
    height: u32,
    mips: u32,
    format: TexelFormat,
}

impl PageKey {
    /// Key of a regular chain of `levels` levels, as `chain_key` keeps it.
    fn of_chain(format: TexelFormat, width: u32, height: u32, levels: u32) -> Self {
        let mips = levels.min(32 - width.max(height).max(1).leading_zeros());
        PageKey {
            width,
            height,
            mips,
            format,
        }
    }

    /// Bytes of one layer with its mip chain.
    fn layer_bytes(&self) -> u64 {
        (0..self.mips)
            .map(|i| self.format.level_bytes(level_dim(self.width, i), level_dim(self.height, i)))
            .sum()
    }
}

fn level_dim(size: u32, level: u32) -> u32 {
    size.checked_shr(level).unwrap_or(0).max(1)
}

/// Page key of a mip chain: level 0 must be valid and fit the device; only
/// the levels that form a chain from it are kept (each half the previous,
/// data long enough), as the standalone textures did. A short level 0 is
/// refused: in a shared page its layer would show the previous texels.
fn page_key(mips: &[MipLevel], max_dim: u32) -> Option<PageKey> {
    chain_key(
        TexelFormat::Rgba8,
        mips.iter()
            .map(|m| (m.width, m.height, m.data.len() >= m.width as usize * m.height as usize * 4)),
        max_dim,
    )
}

/// Page key of a chain given as (width, height, data complete) per level.
fn chain_key(format: TexelFormat, mut levels: impl Iterator<Item = (u32, u32, bool)>, max_dim: u32) -> Option<PageKey> {
    let (w, h, complete) = levels.next()?;
    if w == 0 || h == 0 || w > max_dim || h > max_dim || !complete || !format.fits(w, h) {
        return None;
    }
    // a longer chain is invalid for the device (1×1 is the last level)
    let max_levels = 32 - w.max(h).leading_zeros();
    let count = 1 + levels
        .zip(1..max_levels)
        .take_while(|&((lw, lh, complete), i)| lw == level_dim(w, i) && lh == level_dim(h, i) && complete)
        .count() as u32;
    Some(PageKey {
        width: w,
        height: h,
        mips: count,
        format,
    })
}

/// Page key of a texture once its `levels` finest levels are dropped: level
/// `levels` becomes level 0 and the rest of the chain follows. None when
/// nothing would be dropped or nothing would be left, or when the level
/// that would come first is not made of whole blocks (a BC7 page cannot
/// start there: the texture keeps its size).
fn shrunk_key(key: PageKey, levels: u32) -> Option<PageKey> {
    let (width, height) = (level_dim(key.width, levels), level_dim(key.height, levels));
    (levels > 0 && levels < key.mips && key.format.fits(width, height)).then_some(PageKey {
        width,
        height,
        mips: key.mips - levels,
        format: key.format,
    })
}

fn pack(page: u32, layer: u32) -> u32 {
    debug_assert!(page < MAX_PAGES && layer < 1 << LAYER_BITS);
    (page << LAYER_BITS) | layer
}

#[cfg(test)]
fn unpack(loc: u32) -> (u32, u32) {
    (loc >> LAYER_BITS, loc & ((1 << LAYER_BITS) - 1))
}

/// Whether to have the next page of a size made while an upload takes one
/// of its `free` layers (`spare`: those of the spare page, used once the
/// others are full): when the `queued` uploads behind it need more layers
/// than will be left, and no spare page will be.
fn wants_spare(free: u32, spare: u32, queued: u32) -> bool {
    let spare_left = if free > 0 { spare } else { 0 };
    spare_left == 0 && queued > (free + spare).saturating_sub(1)
}

/// Layers of a new page of a size whose pages already hold `allocated`
/// bytes.
fn page_layers(layer_bytes: u64, allocated: u64, max_layers: u32) -> u32 {
    let target = (allocated / 2).clamp(PAGE_FIRST_BYTES, PAGE_MAX_BYTES);
    (target / layer_bytes.max(1)).clamp(1, max_layers.max(1) as u64) as u32
}

/// The page to take a layer from, among (page, used, layers) of one size:
/// the fullest one with room, so the emptier ones drain and get dropped.
fn pick_page(pages: impl Iterator<Item = (u32, u32, u32)>) -> Option<u32> {
    pages
        .filter(|&(_, used, len)| used < len)
        .max_by_key(|&(page, used, _)| (used, Reverse(page)))
        .map(|(page, _, _)| page)
}

/// The page to empty into the others of its size, among (page, used,
/// layers): sparse (at most a quarter used), small enough to copy, and
/// fitting in the free layers of the others. Page 0 holds the reserved slots
/// and never moves.
///
/// `eager` (memory pressure): any page that fits in the others, up to a
/// whole page to copy. Textures lowered to give memory back leave their
/// layers one here, one there: without it the pages they were in would stay
/// allocated, half empty, while smaller ones are created.
fn compaction_pick(pages: &[(u32, u32, u32)], layer_bytes: u64, eager: bool) -> Option<(u32, u32)> {
    let free: u32 = pages.iter().map(|&(_, used, len)| len - used).sum();
    let max_bytes = if eager { PAGE_MAX_BYTES } else { COMPACT_MAX_BYTES };
    pages
        .iter()
        .filter(|&&(page, used, len)| {
            page != 0 && (eager || used * 4 <= len) && used as u64 * layer_bytes <= max_bytes && used <= free - (len - used)
        })
        .min_by_key(|&&(page, used, _)| (used, page))
        .map(|&(page, used, _)| (page, used))
}

/// Free layers of a page, lowest first.
struct LayerAlloc {
    len: u32,
    /// Layers below this were handed out at least once.
    next: u32,
    freed: BinaryHeap<Reverse<u32>>,
}

impl LayerAlloc {
    fn new(len: u32) -> Self {
        Self {
            len,
            next: 0,
            freed: BinaryHeap::new(),
        }
    }

    fn alloc(&mut self) -> Option<u32> {
        if let Some(Reverse(layer)) = self.freed.pop() {
            return Some(layer);
        }
        (self.next < self.len).then(|| {
            self.next += 1;
            self.next - 1
        })
    }

    fn free(&mut self, layer: u32) {
        debug_assert!(layer < self.next);
        self.freed.push(Reverse(layer));
    }

    fn used(&self) -> u32 {
        self.next - self.freed.len() as u32
    }
}

struct Page {
    key: PageKey,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    layers: LayerAlloc,
    /// Slot of each layer (`NO_SLOT` when free), to move them on compaction.
    owners: Vec<u32>,
}

struct Slot {
    key: PageKey,
    page: u32,
    layer: u32,
    /// Shows the shared placeholder layer: the slot owns no layer yet.
    shared: bool,
}

/// Slots, pages and the CPU copy of the location buffer (no bind group).
struct PageStore {
    slots: Vec<Option<Slot>>,
    /// Free slots, lowest first.
    free_slots: BinaryHeap<Reverse<u32>>,
    capacity: u32,
    pages: Vec<Option<Page>>,
    /// Free page indices, lowest first (keeps the bound array short).
    free_pages: BinaryHeap<Reverse<u32>>,
    /// Live pages of each size.
    by_key: HashMap<PageKey, Vec<u32>>,
    page_capacity: u32,
    max_layers: u32,
    max_dim: u32,
    /// The device has BC texture compression: BC7 chains are accepted.
    bc7: bool,
    /// Location of each slot, uploaded by range in `TextureTable::maintain`.
    loc: Vec<u32>,
    loc_dirty: Option<(usize, usize)>,
    /// Texel bytes of the live textures (the streamer's memory budget, as
    /// before pages) and bytes of the allocated pages.
    bytes: u64,
    live: u32,
    page_bytes: u64,
    /// A page was created (the bind group must include it before the next
    /// draw) / dropped / a layer freed (compaction may apply).
    created: bool,
    dropped: bool,
    freed: bool,
    /// Memory pressure: compaction takes any page that fits in the others
    /// (`compaction_pick`).
    eager: bool,
    warned_full: bool,
    /// A page could not be made (video memory full): when to try again.
    page_backoff: crate::gpu_alloc::Backoff,
    /// Pages created so far, by whom and in how long (AURORA_PROFILE).
    made: PagesMade,
    /// Pages the page thread made, waiting for the first texture of their
    /// size (with when they arrived).
    spares: Vec<(MadePage, Instant)>,
    /// Emptied pages the current bind group may still hold: taken by
    /// `TextureTable::maintain` once it is rebuilt.
    retired: Vec<DeadPage>,
}

/// Pages created since the start: by the caller (the main thread) and by
/// the page thread, with the time they took there.
#[derive(Debug, Default, Clone, Copy)]
pub struct PagesMade {
    pub inline: u32,
    pub inline_ms: f32,
    pub threaded: u32,
    pub threaded_ms: f32,
    /// The longest one on the page thread.
    pub threaded_max_ms: f32,
    /// Made by the page thread and destroyed unused (`SPARE_TTL`).
    pub unused: u32,
}

impl PageStore {
    fn new(capacity: u32, page_capacity: u32, max_layers: u32, max_dim: u32) -> Self {
        Self {
            slots: Vec::with_capacity(capacity as usize),
            free_slots: BinaryHeap::new(),
            capacity,
            pages: Vec::new(),
            free_pages: BinaryHeap::new(),
            by_key: HashMap::new(),
            page_capacity,
            max_layers,
            max_dim,
            bc7: false,
            loc: vec![WHITE_LOC; capacity as usize],
            loc_dirty: None,
            bytes: 0,
            live: 0,
            page_bytes: 0,
            created: false,
            dropped: false,
            freed: false,
            eager: false,
            warned_full: false,
            page_backoff: Default::default(),
            made: PagesMade::default(),
            spares: Vec::new(),
            retired: Vec::new(),
        }
    }

    fn page(&self, page: u32) -> Option<&Page> {
        self.pages.get(page as usize)?.as_ref()
    }

    fn set_loc(&mut self, slot: u32, loc: u32) {
        let i = slot as usize;
        if let Some(l) = self.loc.get_mut(i) {
            *l = loc;
            self.loc_dirty = Some(match self.loc_dirty {
                Some((lo, hi)) => (lo.min(i), hi.max(i + 1)),
                None => (i, i + 1),
            });
        }
    }

    /// A free layer for `slot` in an existing page of that size (not
    /// `exclude`).
    fn find_layer(&mut self, key: PageKey, slot: u32, exclude: Option<u32>) -> Option<(u32, u32)> {
        let ids = self.by_key.get(&key)?;
        let page = pick_page(
            ids.iter()
                .filter(|&&p| Some(p) != exclude)
                .filter_map(|&p| self.page(p).map(|pg| (p, pg.layers.used(), pg.layers.len))),
        )?;
        let pg = self.pages.get_mut(page as usize)?.as_mut()?;
        let layer = pg.layers.alloc()?;
        pg.owners[layer as usize] = slot;
        Some((page, layer))
    }

    /// A layer for `slot`, in a new page if those of that size are full.
    fn place(&mut self, device: &wgpu::Device, key: PageKey, slot: u32) -> Option<(u32, u32)> {
        if let Some(found) = self.find_layer(key, slot, None) {
            return Some(found);
        }
        self.new_page(device, key)?;
        self.find_layer(key, slot, None)
    }

    /// Whether a page can still be added to the table.
    fn has_page_room(&self) -> bool {
        !self.free_pages.is_empty() || (self.pages.len() as u32) < self.page_capacity
    }

    /// Layers of the next page of a size, from what its pages already hold
    /// (the spare one included).
    fn next_page_layers(&self, key: PageKey) -> u32 {
        let layers: u64 = self
            .by_key
            .get(&key)
            .map_or(0, |ids| ids.iter().filter_map(|&p| self.page(p)).map(|p| p.layers.len as u64).sum());
        let allocated = (layers + self.spare_layers(key) as u64) * key.layer_bytes();
        page_layers(key.layer_bytes(), allocated, self.max_layers)
    }

    /// Free layers in the pages of a size.
    fn free_layers(&self, key: PageKey) -> u32 {
        self.by_key.get(&key).map_or(0, |ids| {
            ids.iter()
                .filter_map(|&p| self.page(p))
                .map(|p| p.layers.len - p.layers.used())
                .sum()
        })
    }

    /// Layers of the spare pages of a size.
    fn spare_layers(&self, key: PageKey) -> u32 {
        self.spares.iter().filter(|(p, _)| p.key == key).map(|(p, _)| p.layers).sum()
    }

    /// Whether `slot` has a layer of that size of its own (new texels of
    /// that size overwrite it in place).
    fn holds(&self, slot: u32, key: PageKey) -> bool {
        self.slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|s| s.key == key && !s.shared)
    }

    /// A page from the page thread: kept until a texture of its size needs
    /// a layer (`new_page`).
    fn add_spare(&mut self, page: MadePage) {
        self.made.threaded += 1;
        self.made.threaded_ms += page.ms;
        self.made.threaded_max_ms = self.made.threaded_max_ms.max(page.ms);
        log_slow_page(page.key, page.layers, page.ms, "page thread");
        self.spares.push((page, Instant::now()));
    }

    /// The spare pages no texture took in `SPARE_TTL` (no bind group ever
    /// held them: they can be destroyed at once).
    fn expired_spares(&mut self, now: Instant) -> Vec<DeadPage> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.spares.len() {
            if now.saturating_duration_since(self.spares[i].1) >= SPARE_TTL {
                let (page, _) = self.spares.swap_remove(i);
                out.push((page.texture, page.view));
                self.made.unused += 1;
            } else {
                i += 1;
            }
        }
        out
    }

    /// A new page of that size in the table: the spare one the page thread
    /// made if there is one, else created here.
    fn new_page(&mut self, device: &wgpu::Device, key: PageKey) -> Option<u32> {
        let spare = self.spares.iter().position(|(p, _)| p.key == key);
        let made = match spare {
            Some(_) => None,
            None => {
                // a page video memory could not hold is not installed (an
                // invalid texture failed every upload into it): the
                // texture waits, and the page is asked again a little later
                let now = Instant::now();
                if !self.page_backoff.ready(now) {
                    return None;
                }
                let layers = self.next_page_layers(key);
                let t0 = Instant::now();
                match make_page(device, key, layers) {
                    Ok((texture, view)) => {
                        self.page_backoff.succeeded();
                        let ms = t0.elapsed().as_secs_f32() * 1000.0;
                        self.made.inline += 1;
                        self.made.inline_ms += ms;
                        log_slow_page(key, layers, ms, "main thread");
                        Some((layers, texture, view))
                    }
                    Err(e) => {
                        if !self.page_backoff.waiting() {
                            log::warn!(
                                "texture page {}×{} ({} MB) not made ({e}): video memory is full, the textures that need it wait",
                                key.width,
                                key.height,
                                (layers as u64 * key.layer_bytes()) >> 20
                            );
                        }
                        self.page_backoff.failed(now);
                        return None;
                    }
                }
            }
        };
        let index = match self.free_pages.pop() {
            Some(Reverse(i)) => i,
            None if (self.pages.len() as u32) < self.page_capacity => {
                self.pages.push(None);
                (self.pages.len() - 1) as u32
            }
            None => {
                if !std::mem::replace(&mut self.warned_full, true) {
                    log::warn!("texture pages: all {} page slots in use", self.page_capacity);
                }
                return None;
            }
        };
        let (layers, texture, view) = match (spare, made) {
            (_, Some(made)) => made,
            (Some(at), None) => {
                let (page, _) = self.spares.swap_remove(at);
                (page.layers, page.texture, page.view)
            }
            (None, None) => return None,
        };
        self.page_bytes += layers as u64 * key.layer_bytes();
        self.pages[index as usize] = Some(Page {
            key,
            texture,
            view,
            layers: LayerAlloc::new(layers),
            owners: vec![NO_SLOT; layers as usize],
        });
        self.by_key.entry(key).or_default().push(index);
        self.created = true;
        Some(index)
    }

    /// Free a layer; an empty page is dropped (not destroyed: the current
    /// bind group may still reference it until rebuilt, and it is then
    /// destroyed with a few others per frame).
    fn release(&mut self, page: u32, layer: u32) {
        let Some(Some(p)) = self.pages.get_mut(page as usize) else {
            return;
        };
        if let Some(o) = p.owners.get_mut(layer as usize) {
            *o = NO_SLOT;
        }
        p.layers.free(layer);
        if p.layers.used() > 0 || page == 0 {
            self.freed = true;
            return;
        }
        let key = p.key;
        self.page_bytes = self.page_bytes.saturating_sub(p.layers.len as u64 * key.layer_bytes());
        if let Some(p) = self.pages[page as usize].take() {
            self.retired.push((p.texture, p.view));
        }
        if let Some(ids) = self.by_key.get_mut(&key) {
            ids.retain(|&i| i != page);
            if ids.is_empty() {
                self.by_key.remove(&key);
            }
        }
        self.free_pages.push(Reverse(page));
        self.dropped = true;
    }

    fn upload(&self, writes: &mut GpuWrites, page: u32, layer: u32, mips: &[MipLevel], count: u32) {
        let Some(p) = self.page(page) else {
            return;
        };
        // `page_key` checked every level kept: sizes and data lengths
        for (i, m) in mips.iter().take(count as usize).enumerate() {
            let dst = TexelDst {
                mip: i as u32,
                x: 0,
                y: 0,
                layer,
                width: m.width,
                height: m.height,
            };
            writes.write_texture(&p.texture, dst, m.data);
        }
    }

    fn create(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, mips: &[MipLevel]) -> Option<u32> {
        let key = page_key(mips, self.max_dim)?;
        let slot = match self.free_slots.pop() {
            Some(Reverse(s)) => s,
            None => {
                if self.slots.len() as u32 >= self.capacity {
                    return None;
                }
                self.slots.push(None);
                (self.slots.len() - 1) as u32
            }
        };
        let Some((page, layer)) = self.place(device, key, slot) else {
            self.free_slots.push(Reverse(slot));
            return None;
        };
        self.upload(writes, page, layer, mips, key.mips);
        self.slots[slot as usize] = Some(Slot {
            key,
            page,
            layer,
            shared: false,
        });
        self.bytes += key.layer_bytes();
        self.live += 1;
        self.set_loc(slot, pack(page, layer));
        Some(slot)
    }

    /// A slot showing the shared placeholder layer until its first texels
    /// arrive (`replace` / `replace_staged`): nothing is written and no
    /// layer is taken.
    fn create_placeholder(&mut self) -> Option<u32> {
        let (key, page, layer) = self
            .slots
            .get(PLACEHOLDER as usize)
            .and_then(Option::as_ref)
            .map(|s| (s.key, s.page, s.layer))?;
        let slot = match self.free_slots.pop() {
            Some(Reverse(s)) => s,
            None => {
                if self.slots.len() as u32 >= self.capacity {
                    return None;
                }
                self.slots.push(None);
                (self.slots.len() - 1) as u32
            }
        };
        self.slots[slot as usize] = Some(Slot {
            key,
            page,
            layer,
            shared: true,
        });
        self.live += 1;
        self.set_loc(slot, pack(page, layer));
        Some(slot)
    }

    /// A slot still showing the loading placeholder shows the missing grey
    /// ([`MISSING`]) instead; its first texels, if they ever come, replace
    /// it as they replace the placeholder. False for any other slot.
    fn show_missing(&mut self, slot: u32) -> bool {
        let Some((key, page, layer)) = self
            .slots
            .get(MISSING as usize)
            .and_then(Option::as_ref)
            .map(|s| (s.key, s.page, s.layer))
        else {
            return false;
        };
        if slot < RESERVED {
            return false;
        }
        let Some(Some(s)) = self.slots.get_mut(slot as usize) else {
            return false;
        };
        if !s.shared {
            return false;
        }
        *s = Slot {
            key,
            page,
            layer,
            shared: true,
        };
        self.set_loc(slot, pack(page, layer));
        true
    }

    fn replace(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, slot: u32, mips: &[MipLevel]) -> bool {
        let Some(key) = page_key(mips, self.max_dim) else {
            return false;
        };
        let Some((page, layer)) = self.target(device, slot, key) else {
            return false;
        };
        self.upload(writes, page, layer, mips, key.mips);
        true
    }

    /// Like `replace`, from a mip chain staged by a background job: the
    /// copies are recorded in the frame's upload encoder.
    fn replace_staged(&mut self, device: &wgpu::Device, uploads: &mut UploadQueue, slot: u32, staged: &StagedTexture) -> bool {
        if staged.format == TexelFormat::Bc7 && !self.bc7 {
            return false;
        }
        let Some(key) = chain_key(staged.format, staged.levels.iter().map(|l| (l.width, l.height, true)), self.max_dim) else {
            return false;
        };
        let Some((page, layer)) = self.target(device, slot, key) else {
            return false;
        };
        let Some(p) = self.page(page) else {
            return false;
        };
        uploads.copy_to_layer(device, staged, &p.texture, layer, key.mips);
        true
    }

    /// The layer that receives `slot`'s new texels of size `key`. Same size:
    /// the slot's layer, overwritten in place (the write is replayed with
    /// this frame: the frames before it keep the old texels). Another size (higher
    /// resolution): a layer of a page of the new size, then the old layer
    /// is freed.
    fn target(&mut self, device: &wgpu::Device, slot: u32, key: PageKey) -> Option<(u32, u32)> {
        if slot < RESERVED || slot as usize >= self.slots.len() {
            return None;
        }
        if let Some(s) = &self.slots[slot as usize]
            && s.key == key
            && !s.shared
        {
            return Some((s.page, s.layer));
        }
        let (page, layer) = self.place(device, key, slot)?;
        match self.slots[slot as usize].replace(Slot {
            key,
            page,
            layer,
            shared: false,
        }) {
            // (the placeholder layer stays: other slots show it)
            Some(old) if old.shared => {}
            Some(old) => {
                self.release(old.page, old.layer);
                self.bytes = self.bytes.saturating_sub(old.key.layer_bytes());
            }
            None => {
                // a freed slot filled again: no longer free
                self.free_slots.retain(|r| r.0 != slot);
                self.live += 1;
            }
        }
        self.bytes += key.layer_bytes();
        self.set_loc(slot, pack(page, layer));
        Some((page, layer))
    }

    /// Whether new texels of this size for `slot` would need a new page
    /// (none of that size has a free layer, and the slot is not already of
    /// that size). `levels`: length of the mip chain, as uploaded.
    #[cfg(test)]
    fn needs_page(&self, slot: u32, format: TexelFormat, width: u32, height: u32, levels: u32) -> bool {
        let key = PageKey::of_chain(format, width, height, levels);
        !self.holds(slot, key) && self.free_layers(key) == 0
    }

    fn free(&mut self, slot: u32) {
        if slot < RESERVED {
            return;
        }
        let Some(old) = self.slots.get_mut(slot as usize).and_then(Option::take) else {
            return;
        };
        if !old.shared {
            self.release(old.page, old.layer);
            self.bytes = self.bytes.saturating_sub(old.key.layer_bytes());
        }
        self.live = self.live.saturating_sub(1);
        self.free_slots.push(Reverse(slot));
        self.set_loc(slot, WHITE_LOC);
    }

    /// Drop the `levels` finest levels of `slot` to give memory back: the
    /// levels it keeps are copied on the GPU to a layer of the smaller size
    /// (nothing is decoded again, the texels already there are the ones a
    /// lower resolution shows), then the old layer is freed. As for
    /// compaction, the copies go to the upload encoder, submitted before
    /// the frame that sees the new location. False when the slot has no
    /// layer of its own, too few levels, or no page could be had.
    fn shrink(&mut self, device: &wgpu::Device, uploads: &mut UploadQueue, slot: u32, levels: u32) -> bool {
        if slot < RESERVED {
            return false;
        }
        let Some((key, page, layer)) = self
            .slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .filter(|s| !s.shared)
            .map(|s| (s.key, s.page, s.layer))
        else {
            return false;
        };
        let Some(small) = shrunk_key(key, levels) else {
            return false;
        };
        let Some((dst, dst_layer)) = self.place(device, small, slot) else {
            return false;
        };
        let encoder = uploads.encoder(device);
        if let (Some(s), Some(d)) = (self.page(page), self.page(dst)) {
            for level in 0..small.mips {
                let (width, height) = small
                    .format
                    .copy_extent(level_dim(small.width, level), level_dim(small.height, level));
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &s.texture,
                        mip_level: level + levels,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &d.texture,
                        mip_level: level,
                        origin: wgpu::Origin3d { x: 0, y: 0, z: dst_layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        self.slots[slot as usize] = Some(Slot {
            key: small,
            page: dst,
            layer: dst_layer,
            shared: false,
        });
        self.bytes = self.bytes.saturating_sub(key.layer_bytes()) + small.layer_bytes();
        self.set_loc(slot, pack(dst, dst_layer));
        // (the last layer of a page drops it; the encoder keeps it alive)
        self.release(page, layer);
        true
    }

    /// Empty one sparse page into the other pages of its size (GPU copies of
    /// every level, submitted before the frame that sees the new locations).
    /// Whether a page was emptied.
    fn compact(&mut self, device: &wgpu::Device, uploads: &mut UploadQueue) -> bool {
        if !std::mem::take(&mut self.freed) {
            return false;
        }
        let eager = self.eager;
        let candidate = self.by_key.iter().filter(|(_, ids)| ids.len() > 1).find_map(|(key, ids)| {
            let pages: Vec<(u32, u32, u32)> = ids
                .iter()
                .filter_map(|&p| self.page(p).map(|pg| (p, pg.layers.used(), pg.layers.len)))
                .collect();
            compaction_pick(&pages, key.layer_bytes(), eager).map(|(page, _)| (page, *key))
        });
        let Some((src, key)) = candidate else {
            return false;
        };
        let Some(moves) = self.page(src).map(|p| {
            p.owners
                .iter()
                .enumerate()
                .filter(|&(_, &s)| s != NO_SLOT)
                .map(|(l, &s)| (l as u32, s))
                .collect::<Vec<_>>()
        }) else {
            return false;
        };
        // in the upload encoder: after the copies of this frame's uploads
        // (a moved layer may just have been written), before the passes
        let encoder = uploads.encoder(device);
        for (src_layer, slot) in moves {
            let Some((dst, dst_layer)) = self.find_layer(key, slot, Some(src)) else {
                break;
            };
            if let (Some(s), Some(d)) = (self.page(src), self.page(dst)) {
                for level in 0..key.mips {
                    let (width, height) = key.format.copy_extent(level_dim(key.width, level), level_dim(key.height, level));
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &s.texture,
                            mip_level: level,
                            origin: wgpu::Origin3d { x: 0, y: 0, z: src_layer },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &d.texture,
                            mip_level: level,
                            origin: wgpu::Origin3d { x: 0, y: 0, z: dst_layer },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
            if let Some(Some(s)) = self.slots.get_mut(slot as usize) {
                s.page = dst;
                s.layer = dst_layer;
            }
            self.set_loc(slot, pack(dst, dst_layer));
            // the last one drops the page; the encoder keeps it alive
            self.release(src, src_layer);
        }
        // other pages may qualify: look again at the next interval
        self.freed = true;
        true
    }
}

pub struct TextureTable {
    pub layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    /// Packed location of every slot (`tex_loc` in common.wgsl).
    loc_buffer: wgpu::Buffer,
    store: PageStore,
    /// The device allows a bind group shorter than the layout's array
    /// (PARTIALLY_BOUND_BINDING_ARRAY); else every page element is bound.
    partial: bool,
    /// Page elements in the current bind group.
    bound: u32,
    /// The bind group must be rebuilt before the next draw, whatever the
    /// throttle (new page, sampler change).
    rebuild_now: bool,
    dirty: bool,
    last_rebuild: Instant,
    last_compact: Instant,
    /// Emptied pages no bind group holds any more, destroyed a few per
    /// frame by the page thread (`pages_to_destroy`).
    doomed: std::collections::VecDeque<DeadPage>,
    worker: PageWorker,
    /// Pages the streamed uploads wait for.
    requests: PageRequests,
    last_destroy: Instant,
    /// Reductions asked this frame (slot, levels to drop, its size when
    /// asked), done in `maintain`.
    shrinks: Vec<(u32, u32, PageKey)>,
    /// The last eager compaction emptied a page: more may follow.
    compacting: bool,
}

impl TextureTable {
    /// `capacity`: slots (textures) of the table; `bindable`: elements the
    /// device lets a shader bind in one array, which bounds the pages.
    pub fn new(device: &wgpu::Device, writes: &mut GpuWrites, capacity: u32, bindable: u32, anisotropy: u16) -> Self {
        let partial = device.features().contains(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY);
        let limits = device.limits();
        let capacity = capacity.max(RESERVED + 1);
        let page_capacity = page_capacity(bindable, partial);
        let max_layers = limits.max_texture_array_layers.clamp(1, MAX_LAYERS);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: NonZeroU32::new(page_capacity),
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(4),
                    },
                    count: None,
                },
            ],
        });
        let loc_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture locations"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut store = PageStore::new(capacity, page_capacity, max_layers, limits.max_texture_dimension_2d);
        store.bc7 = device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        for (slot, rgba) in [
            (WHITE, [255u8, 255, 255, 255]),
            (FLAT_NORMAL, [128, 128, 255, 255]),
            (BLACK, [0, 0, 0, 255]),
            (TRANSPARENT, [0, 0, 0, 0]),
            (PLACEHOLDER, [148, 148, 156, 255]),
            (MISSING, [127, 127, 127, 255]),
        ] {
            let created = store.create(
                device,
                writes,
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &rgba,
                }],
            );
            debug_assert_eq!(created, Some(slot));
        }
        let ramp: Vec<u8> = TERRAIN_RAMP_PIXELS.iter().flat_map(|&v| [v, v, v, 255]).collect();
        let ramp_mips = build_mips(256, 256, ramp, u32::MAX);
        let levels: Vec<_> = ramp_mips
            .iter()
            .map(|(w, h, data)| MipLevel {
                width: *w,
                height: *h,
                data,
            })
            .collect();
        let created = store.create(device, writes, &levels);
        debug_assert_eq!(created, Some(TERRAIN_RAMP));
        debug_assert_eq!(store.loc[WHITE as usize], WHITE_LOC);
        // the reserved slots are not counted in the statistics
        store.live = 0;
        store.bytes = 0;
        store.loc_dirty = None;
        store.created = false;
        writes.write_buffer(&loc_buffer, 0, bytemuck::cast_slice(&store.loc));
        let sampler = Self::make_sampler(device, anisotropy);
        let bound = bound_len(&store, partial);
        let bind_group = Self::build(device, &layout, &sampler, &loc_buffer, &store, bound);
        Self {
            layout,
            bind_group,
            sampler,
            loc_buffer,
            store,
            partial,
            bound,
            rebuild_now: false,
            dirty: false,
            last_rebuild: Instant::now(),
            last_compact: Instant::now(),
            doomed: Default::default(),
            worker: PageWorker::new(device),
            requests: PageRequests::default(),
            last_destroy: Instant::now(),
            shrinks: Vec::new(),
            compacting: false,
        }
    }

    fn make_sampler(device: &wgpu::Device, anisotropy: u16) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            anisotropy_clamp: anisotropy.clamp(1, 16),
            border_color: None,
        })
    }

    /// Change the anisotropic filtering level (1 = off, up to 16); applied
    /// by the next `maintain`, before the next draw.
    pub fn set_anisotropy(&mut self, device: &wgpu::Device, anisotropy: u16) {
        self.sampler = Self::make_sampler(device, anisotropy);
        self.dirty = true;
        self.rebuild_now = true;
    }

    fn build(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        loc_buffer: &wgpu::Buffer,
        store: &PageStore,
        len: u32,
    ) -> wgpu::BindGroup {
        let Some(first) = store.page(0) else {
            unreachable!("page 0 holds the reserved slots and is never dropped");
        };
        let views: Vec<&wgpu::TextureView> = (0..len).map(|i| store.page(i).map_or(&first.view, |p| &p.view)).collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("textures"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: loc_buffer.as_entire_binding(),
                },
            ],
        })
    }

    pub fn live(&self) -> u32 {
        self.store.live
    }

    pub fn bytes(&self) -> u64 {
        self.store.bytes
    }

    /// Live pages and their allocated memory (free layers included).
    pub fn pages(&self) -> u32 {
        self.store.pages.iter().filter(|p| p.is_some()).count() as u32
    }

    pub fn page_bytes(&self) -> u64 {
        self.store.page_bytes
    }

    pub fn capacity(&self) -> u32 {
        self.store.capacity
    }

    /// The device has BC texture compression: `replace_staged` accepts BC7
    /// chains (a quarter of the video memory of RGBA8).
    pub fn bc7(&self) -> bool {
        self.store.bc7
    }

    /// CPU time the caller spent creating pages since the start (ms;
    /// AURORA_PROFILE): those of the streamed uploads are made by the page
    /// thread and cost it nothing (`prepare_page`).
    pub fn page_create_ms(&self) -> f32 {
        self.store.made.inline_ms
    }

    /// Pages created since the start, by the caller and by the page thread.
    pub fn pages_made(&self) -> PagesMade {
        self.store.made
    }

    /// Before a streamed upload (`replace` / `replace_staged` of a chain of
    /// `levels` levels): true when its texels can be placed now without
    /// creating a page. Otherwise the page is being made by the page thread
    /// and the upload must wait for a later frame: creating it here cost
    /// the main thread 0.4 to 1.5 ms whenever the device's allocator took a
    /// new block of video memory, at times much more.
    ///
    /// `queued`: the other uploads of that size waiting behind this one.
    /// When they outnumber the layers left, the next page is asked for at
    /// once (one spare page per size at most), so that a size being loaded
    /// finds its pages ready and waits only for its first one: a frame per
    /// page would slow the uploads of the large sizes down at 60 frames a
    /// second (a 2048² texture fills a page).
    ///
    /// True as well when the page thread cannot make the page (not started,
    /// no answer in `REQUEST_TIMEOUT`, table full): the upload then creates
    /// it or is refused, as before.
    pub fn prepare_page(&mut self, slot: u32, (format, width, height, levels): (TexelFormat, u32, u32, u32), queued: u32) -> bool {
        self.receive_pages();
        let key = PageKey::of_chain(format, width, height, levels);
        if self.store.holds(slot, key) || !self.store.has_page_room() {
            return true;
        }
        let (free, spare) = (self.store.free_layers(key), self.store.spare_layers(key));
        if free + spare == 0 {
            return !self.ask_for_page(key);
        }
        // this upload takes a layer (of the spare page when the others are
        // full, which then joins the table)
        if wants_spare(free, spare, queued) {
            self.ask_for_page(key);
        }
        true
    }

    /// Have the page thread make the next page of a size, unless it is at
    /// it already. False when it cannot: the caller creates the page.
    fn ask_for_page(&mut self, key: PageKey) -> bool {
        match self.requests.ask(key, Instant::now()) {
            Ask::Send => {
                if self.worker.make(key, self.store.next_page_layers(key)) {
                    true
                } else {
                    self.requests.done(key);
                    false
                }
            }
            Ask::Wait => true,
            Ask::GiveUp => {
                log::warn!("texture pages: no page from the page thread in {REQUEST_TIMEOUT:?}; created by the main thread");
                false
            }
        }
    }

    /// Take the pages the page thread finished.
    fn receive_pages(&mut self) {
        while let Ok(page) = self.worker.made.try_recv() {
            self.requests.done(page.key);
            self.store.add_spare(page);
        }
    }

    pub fn is_full(&self) -> bool {
        self.store.free_slots.is_empty() && self.store.slots.len() as u32 >= self.store.capacity
    }

    /// Create a texture from a full mip chain (level 0 first). Returns the slot.
    pub fn create(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, mips: &[MipLevel]) -> Option<u32> {
        self.store.create(device, writes, mips)
    }

    /// A slot for a texture still loading: it shows the shared grey
    /// placeholder ([`PLACEHOLDER`]) until `replace` / `replace_staged`
    /// gives it texels. Costs no GPU work.
    pub fn create_placeholder(&mut self) -> Option<u32> {
        self.store.create_placeholder()
    }

    /// A texture that will not come: its slot, while it still shows the
    /// loading placeholder, shows the missing grey ([`MISSING`]). Costs no
    /// GPU work. False for a slot with texels of its own.
    pub fn show_missing(&mut self, slot: u32) -> bool {
        self.store.show_missing(slot)
    }

    /// Replace the contents of an existing slot (e.g. higher resolution).
    pub fn replace(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, slot: u32, mips: &[MipLevel]) -> bool {
        self.store.replace(device, writes, slot, mips)
    }

    /// Replace the contents of a slot from a staged mip chain (its region
    /// must be ready); the copies go to the frame's upload encoder.
    pub fn replace_staged(&mut self, device: &wgpu::Device, uploads: &mut UploadQueue, slot: u32, staged: &StagedTexture) -> bool {
        self.store.replace_staged(device, uploads, slot, staged)
    }

    /// Size of the texture in a slot (level 0).
    pub fn size_of(&self, slot: u32) -> Option<(u32, u32)> {
        let s = self.store.slots.get(slot as usize)?.as_ref()?;
        Some((s.key.width, s.key.height))
    }

    /// Diagnostic: where a slot lives (page, layer, size and levels).
    pub fn describe(&self, slot: u32) -> String {
        match self.store.slots.get(slot as usize).and_then(Option::as_ref) {
            None => "free".into(),
            Some(s) => format!(
                "page {} layer {} ({}×{} {:?}, {} levels{})",
                s.page,
                s.layer,
                s.key.width,
                s.key.height,
                s.key.format,
                s.key.mips,
                if self.store.loc.get(slot as usize) == Some(&pack(s.page, s.layer)) {
                    ""
                } else {
                    ", location out of date"
                }
            ),
        }
    }

    /// Overwrite a rectangle of level 0 in place (media textures updated
    /// every frame: no new texture, no bind group rebuild). `data` holds
    /// `h` rows of `w` RGBA8 pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn write_region(&self, writes: &mut GpuWrites, slot: u32, x: u32, y: u32, w: u32, h: u32, data: &[u8]) -> bool {
        let Some(Some(s)) = self.store.slots.get(slot as usize) else {
            return false;
        };
        let Some(page) = self.store.page(s.page) else {
            return false;
        };
        let need = w as usize * h as usize * 4;
        if slot < RESERVED
            || s.shared
            || s.key.format != TexelFormat::Rgba8
            || w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|r| r > s.key.width)
            || y.checked_add(h).is_none_or(|b| b > s.key.height)
            || data.len() < need
        {
            return false;
        }
        let dst = TexelDst {
            mip: 0,
            x,
            y,
            layer: s.layer,
            width: w,
            height: h,
        };
        writes.write_texture(&page.texture, dst, &data[..need]);
        true
    }

    pub fn free(&mut self, slot: u32) {
        self.store.free(slot);
    }

    /// Ask for the `levels` finest levels of a slot to be dropped (memory
    /// budget): what it keeps is copied on the GPU to a smaller layer, see
    /// `PageStore::shrink`. Done when the frame is handed to the render
    /// thread, not here: the layer it leaves is free for others from then
    /// on, and a texture written to it through the journal later in this
    /// frame would land before the copy reads it. Returns the levels that
    /// will be dropped: down to its last level at most, none for a slot
    /// without texels of its own or already asked this frame.
    pub fn shrink(&mut self, slot: u32, levels: u32) -> u32 {
        let Some(key) = self
            .store
            .slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .filter(|s| slot >= RESERVED && !s.shared)
            .map(|s| s.key)
        else {
            return 0;
        };
        let levels = levels.min(key.mips - 1);
        if shrunk_key(key, levels).is_none() || self.shrinks.iter().any(|s| s.0 == slot) {
            return 0;
        }
        self.shrinks.push((slot, levels, key));
        levels
    }

    /// Memory pressure on or off: under pressure the pages are compacted
    /// often and whatever their fill, see [`COMPACT_EAGER_INTERVAL`].
    pub fn set_eager_compaction(&mut self, eager: bool) {
        if eager && !self.store.eager {
            // (pages the usual rule left alone may qualify now)
            self.store.freed = true;
        }
        self.store.eager = eager;
        self.compacting &= eager;
    }

    /// Memory is still being given back: reductions asked and not done
    /// yet, or pages being emptied under pressure.
    pub fn settling(&self) -> bool {
        !self.shrinks.is_empty() || self.compacting
    }

    /// Once a frame, when it is handed to the render thread: reduce the
    /// textures asked to, compact (throttled), record the upload of the
    /// changed locations, and rebuild the bind group when pages changed (at
    /// once for a new page, throttled for dropped ones).
    pub fn maintain(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, force: bool) {
        self.receive_pages();
        for page in self.store.expired_spares(Instant::now()) {
            self.worker.destroy(page);
        }
        // reductions: those that find a layer in a page of their new size
        // are only copies; when none has room the page comes from the page
        // thread, as for the streamed uploads (`prepare_page`), and the
        // reduction waits for it (on the grid, thousands of textures
        // lowered at once, their pages created here, made frames of 10 to
        // 14 ms instead of 6)
        for (slot, levels, key) in std::mem::take(&mut self.shrinks) {
            // (a slot freed or given texels of another size since is left
            // as it is)
            let Some(small) = shrunk_key(key, levels).filter(|_| self.store.holds(slot, key)) else {
                continue;
            };
            let room = self.store.free_layers(small) + self.store.spare_layers(small) > 0;
            if !room && self.store.has_page_room() && self.ask_for_page(small) {
                self.shrinks.push((slot, levels, key));
                continue;
            }
            self.store.shrink(device, uploads, slot, levels);
        }
        let eager = self.store.eager;
        if self.last_compact.elapsed() >= if eager { COMPACT_EAGER_INTERVAL } else { COMPACT_INTERVAL } {
            self.last_compact = Instant::now();
            self.compacting = self.store.compact(device, uploads) && eager;
        }
        if let Some((lo, hi)) = self.store.loc_dirty.take() {
            writes.write_buffer(&self.loc_buffer, lo as u64 * 4, bytemuck::cast_slice(&self.store.loc[lo..hi]));
        }
        if std::mem::take(&mut self.store.created) {
            self.dirty = true;
            self.rebuild_now = true;
        }
        if std::mem::take(&mut self.store.dropped) {
            self.dirty = true;
        }
        if self.dirty && (force || self.rebuild_now || self.last_rebuild.elapsed() >= REBUILD_THROTTLE) {
            self.bound = bound_len(&self.store, self.partial);
            self.bind_group = Self::build(device, &self.layout, &self.sampler, &self.loc_buffer, &self.store, self.bound);
            self.dirty = false;
            self.rebuild_now = false;
            self.last_rebuild = Instant::now();
            // the pages emptied before this rebuild are in no bind group now
            self.doomed.extend(self.store.retired.drain(..));
        }
        if !self.doomed.is_empty() {
            let n = pages_to_destroy(self.doomed.len(), self.last_destroy.elapsed());
            self.last_destroy = Instant::now();
            for page in self.doomed.drain(..n) {
                self.worker.destroy(page);
            }
        }
    }
}

/// Page elements to bind: every page up to the last live one; the whole
/// array without partially bound arrays.
fn bound_len(store: &PageStore, partial: bool) -> u32 {
    bound_for(store.pages.iter().rposition(Option::is_some), store.page_capacity, partial)
}

fn bound_for(last_live: Option<usize>, page_capacity: u32, partial: bool) -> u32 {
    if !partial {
        return page_capacity;
    }
    last_live.map_or(1, |i| i as u32 + 1)
}

/// Generate a box-filtered mip chain from RGBA8 data (level 0 included).
/// Every channel is the plain average of its four texels, as the Legacy
/// Viewer's mips (glGenerateMipmap, or LLImageBase::generateMip,
/// indra/llimage/llimage.cpp, originally LGPL 2.1): the colour of the empty
/// texels around a cut-out blends into it down the levels, and content is
/// made against that look (that colour is often a dark one around grass
/// and leaves, which darkens them with the distance); averaged by the alpha, as before, a
/// grass bed stayed the colour of its blades and came out much brighter
/// than in the Legacy Viewer.
pub fn build_mips(width: u32, height: u32, data: Vec<u8>, max_levels: u32) -> Vec<(u32, u32, Vec<u8>)> {
    box_mips(width, height, data, max_levels, false)
}

/// The same chain with each colour averaged by the alpha of its texels (the
/// empty ones leave no trace): what Aurora did before, kept to compare
/// (`AURORA_MIP_FILTER=weighted`).
pub fn build_mips_alpha_weighted(width: u32, height: u32, data: Vec<u8>, max_levels: u32) -> Vec<(u32, u32, Vec<u8>)> {
    box_mips(width, height, data, max_levels, true)
}

fn box_mips(width: u32, height: u32, data: Vec<u8>, max_levels: u32, alpha_weighted: bool) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let (mut w, mut h) = (width, height);
    out.push((w, h, data));
    while (w > 1 || h > 1) && (out.len() as u32) < max_levels {
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let Some((_, _, prev)) = out.last() else {
            break;
        };
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let x0 = (x * 2).min(w - 1);
                let x1 = (x * 2 + 1).min(w - 1);
                let y0 = (y * 2).min(h - 1);
                let y1 = (y * 2 + 1).min(h - 1);
                let idx = |xx: u32, yy: u32| ((yy * w + xx) * 4) as usize;
                let (a, b, c, d) = (idx(x0, y0), idx(x1, y0), idx(x0, y1), idx(x1, y1));
                let wa = prev[a + 3] as u32;
                let wb = prev[b + 3] as u32;
                let wc = prev[c + 3] as u32;
                let wd = prev[d + 3] as u32;
                let wsum = wa + wb + wc + wd;
                let o = ((y * nw + x) * 4) as usize;
                for ch in 0..3 {
                    let plain = (prev[a + ch] as u32 + prev[b + ch] as u32 + prev[c + ch] as u32 + prev[d + ch] as u32 + 2) / 4;
                    let v = if alpha_weighted {
                        let weighted = prev[a + ch] as u32 * wa
                            + prev[b + ch] as u32 * wb
                            + prev[c + ch] as u32 * wc
                            + prev[d + ch] as u32 * wd
                            + wsum / 2;
                        weighted.checked_div(wsum).unwrap_or(plain)
                    } else {
                        plain
                    };
                    next[o + ch] = v as u8;
                }
                next[o + 3] = ((wsum + 2) / 4) as u8;
            }
        }
        out.push((nw, nh, next));
        w = nw;
        h = nh;
    }
    out
}

/// Alpha of the cutoff an alpha-tested face draws with when nothing else
/// sets it (0.5, as a texture whose alpha is a mask): a texel draws from 128.
pub const MASK_CUTOFF: u8 = 128;

/// Share of the texels of an RGBA8 level whose alpha reaches `cutoff`:
/// what an alpha test at that cutoff keeps.
pub fn alpha_coverage(data: &[u8], cutoff: u8) -> f32 {
    let n = data.len() / 4;
    if n == 0 {
        return 0.0;
    }
    let kept = data.as_chunks::<4>().0.iter().filter(|p| p[3] >= cutoff).count();
    kept as f32 / n as f32
}

/// Keep what an alpha test at `cutoff` draws the same at every level of a
/// mip chain (Castaño, "Computing Alpha Mipmaps", 2010). The box filter
/// averages thin opaque texels with the empty ones around them, so a leaf,
/// a blade of grass, a hair strand or a fence wire falls under the cutoff a
/// few levels down and the foliage thins out with the distance. Each level
/// from 1 on gets its alpha scaled so that the share of texels at or above
/// the cutoff matches level 0's: the threshold of the level that keeps that
/// share is found on its alpha histogram, and the level is scaled by
/// cutoff / threshold. Level 0 and the colour are left as they are; a
/// level that level 0 leaves fully drawn or fully hidden is not touched.
pub fn preserve_alpha_coverage(mips: &mut [(u32, u32, Vec<u8>)], cutoff: u8) {
    let cutoff = cutoff.max(1);
    let Some(target) = mips.first().map(|(_, _, d)| alpha_coverage(d, cutoff)) else {
        return;
    };
    if target <= 0.0 || target >= 1.0 {
        return;
    }
    for (_, _, data) in mips.iter_mut().skip(1) {
        let n = data.len() / 4;
        if n == 0 {
            continue;
        }
        let mut hist = [0u32; 256];
        for p in data.as_chunks::<4>().0 {
            hist[p[3] as usize] += 1;
        }
        // kept[t]: texels with alpha >= t, from 255 down
        let mut kept = [0u32; 257];
        for t in (0..256).rev() {
            kept[t] = kept[t + 1] + hist[t];
        }
        let want = target * n as f32;
        let threshold = (1..256usize)
            .min_by(|&a, &b| {
                let da = (kept[a] as f32 - want).abs();
                let db = (kept[b] as f32 - want).abs();
                // closest share first; on a tie the lower threshold, the
                // fuller one (thinning out is what this is against)
                da.total_cmp(&db).then(a.cmp(&b))
            })
            .unwrap_or(cutoff as usize) as u32;
        if threshold == cutoff as u32 {
            continue;
        }
        // floor(alpha × cutoff / threshold): alpha >= threshold maps to
        // >= cutoff, alpha < threshold to < cutoff
        let cut = cutoff as u32;
        for p in data.as_chunks_mut::<4>().0 {
            p[3] = (p[3] as u32 * cut / threshold).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline_cache::CachedPipelines;
    use crate::writes::QueueSink;

    /// A 64 × 64 grid of one-texel opaque lines every 4 texels (a fence or
    /// grass blades), the rest transparent.
    fn thin_lines() -> Vec<u8> {
        let mut px = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 0..64 {
                let i = (y * 64 + x) * 4;
                px[i..i + 3].copy_from_slice(&[40, 160, 60]);
                px[i + 3] = if x % 4 == 0 { 255 } else { 0 };
            }
        }
        px
    }

    /// A blade colour beside an empty texel of another colour: the plain
    /// average takes both, as the Legacy Viewer's mips; the weighted one
    /// keeps the blade's.
    #[test]
    fn empty_texels_colour_blends_into_the_mips() {
        let mut px = vec![0u8; 2 * 2 * 4];
        for (i, p) in px.chunks_mut(4).enumerate() {
            p.copy_from_slice(if i % 2 == 0 { &[200, 220, 100, 255] } else { &[40, 44, 20, 0] });
        }
        let plain = build_mips(2, 2, px.clone(), 16);
        assert_eq!(plain[1].2, vec![120, 132, 60, 128]);
        let weighted = build_mips_alpha_weighted(2, 2, px, 16);
        assert_eq!(weighted[1].2, vec![200, 220, 100, 128]);
    }

    #[test]
    fn thin_alpha_fades_under_the_cutoff_without_coverage() {
        let mips = build_mips(64, 64, thin_lines(), 16);
        assert!((alpha_coverage(&mips[0].2, MASK_CUTOFF) - 0.25).abs() < 1e-3);
        // two levels down a line is a quarter of a texel: nothing draws
        assert_eq!(alpha_coverage(&mips[2].2, MASK_CUTOFF), 0.0);
    }

    /// 128 × 128 texels, a quarter of them opaque, scattered (splitmix64).
    fn scattered() -> Vec<u8> {
        let mut px = vec![0u8; 128 * 128 * 4];
        for i in 0..128 * 128u64 {
            let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            px[i as usize * 4 + 3] = if (z ^ (z >> 31)) % 4 == 0 { 255 } else { 0 };
        }
        px
    }

    #[test]
    fn coverage_is_kept_at_every_level() {
        let plain = build_mips(128, 128, scattered(), 16);
        let mut mips = plain.clone();
        preserve_alpha_coverage(&mut mips, MASK_CUTOFF);
        let target = alpha_coverage(&mips[0].2, MASK_CUTOFF);
        assert!((target - 0.25).abs() < 0.02, "{target}");
        // plain mips lose most of it at 32 × 32
        assert!(alpha_coverage(&plain[2].2, MASK_CUTOFF) < 0.1);
        // kept ones hold it down to 8 × 8, within what the few alpha values
        // of a level allow (at 32 × 32, 16 samples a texel: 0.19 or 0.37)
        for (w, _, d) in mips.iter().take_while(|(w, _, _)| *w >= 8) {
            let c = alpha_coverage(d, MASK_CUTOFF);
            assert!((c - target).abs() < 0.13, "{w} × {w}: {c} for {target}");
        }
    }

    #[test]
    fn coverage_leaves_level_0_colour_opaque_and_empty_textures_alone() {
        let px = thin_lines();
        let mut mips = build_mips(64, 64, px.clone(), 16);
        let colours: Vec<Vec<u8>> = mips
            .iter()
            .map(|(_, _, d)| d.as_chunks::<4>().0.iter().flat_map(|p| p[..3].to_vec()).collect())
            .collect();
        preserve_alpha_coverage(&mut mips, MASK_CUTOFF);
        assert_eq!(mips[0].2, px);
        for (i, (_, _, d)) in mips.iter().enumerate() {
            let c: Vec<u8> = d.as_chunks::<4>().0.iter().flat_map(|p| p[..3].to_vec()).collect();
            assert_eq!(c, colours[i], "level {i}: only the alpha changes");
        }
        // fully opaque and fully transparent chains stay as built
        for a in [0u8, 255] {
            let solid = [10, 20, 30, a].repeat(32 * 32);
            let built = build_mips(32, 32, solid.clone(), 16);
            let mut kept = built.clone();
            preserve_alpha_coverage(&mut kept, MASK_CUTOFF);
            assert_eq!(kept, built);
        }
    }

    #[test]
    fn scaling_puts_the_threshold_on_the_cutoff() {
        // level 0 half covered; level 1 four texels at 40, 60, 80, 100: the
        // threshold keeping half of them is 80, so 80 and 100 draw, 40 and 60
        // do not
        let mut level0 = [0, 0, 0, 255].repeat(8);
        level0.extend([0, 0, 0, 0].repeat(8));
        let level1: Vec<u8> = [40u8, 60, 80, 100].iter().flat_map(|&a| [0, 0, 0, a]).collect();
        let mut mips = vec![(4, 4, level0), (2, 2, level1)];
        preserve_alpha_coverage(&mut mips, MASK_CUTOFF);
        let a: Vec<u8> = mips[1].2.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        assert!(a[0] < MASK_CUTOFF && a[1] < MASK_CUTOFF, "{a:?}");
        assert!(a[2] >= MASK_CUTOFF && a[3] >= MASK_CUTOFF, "{a:?}");
        assert!((alpha_coverage(&mips[1].2, MASK_CUTOFF) - 0.5).abs() < 1e-6);
    }
    use std::cell::RefCell;

    fn chain<'a>(data: &'a [u8], dims: &[(u32, u32)]) -> Vec<MipLevel<'a>> {
        dims.iter()
            .map(|&(width, height)| MipLevel {
                width,
                height,
                data: &data[..(width * height * 4) as usize],
            })
            .collect()
    }

    #[test]
    fn page_key_keeps_the_valid_part_of_the_chain() {
        let d = vec![0u8; 64 * 64 * 4];
        let full = chain(&d, &[(64, 32), (32, 16), (16, 8), (8, 4), (4, 2), (2, 1), (1, 1)]);
        assert_eq!(
            page_key(&full, 16384),
            Some(PageKey {
                width: 64,
                height: 32,
                mips: 7,
                format: TexelFormat::Rgba8,
            })
        );
        // a level of the wrong size ends the chain
        assert_eq!(
            page_key(&chain(&d, &[(64, 32), (32, 16), (10, 10)]), 16384).map(|k| k.mips),
            Some(2)
        );
        // not a power of two
        let npot = chain(&d, &[(48, 48), (24, 24), (12, 12), (6, 6), (3, 3), (1, 1)]);
        assert_eq!(page_key(&npot, 16384).map(|k| k.mips), Some(6));
        // 1×1 levels past the end of the chain are ignored
        assert_eq!(
            page_key(&chain(&d, &[(2, 2), (1, 1), (1, 1), (1, 1)]), 16384).map(|k| k.mips),
            Some(2)
        );
        // a short level ends the chain; a short level 0 is refused
        let short = [
            MipLevel {
                width: 4,
                height: 4,
                data: &d[..64],
            },
            MipLevel {
                width: 2,
                height: 2,
                data: &d[..15],
            },
        ];
        assert_eq!(page_key(&short, 16384).map(|k| k.mips), Some(1));
        assert_eq!(page_key(&short[1..], 16384), None);
        // empty, zero sized or larger than the device allows
        assert_eq!(page_key(&[], 16384), None);
        assert_eq!(page_key(&chain(&d, &[(0, 4)]), 16384), None);
        assert_eq!(page_key(&full, 32), None);
    }

    #[test]
    fn layer_bytes_count_the_mip_chain() {
        let key = |width, height, mips| PageKey {
            width,
            height,
            mips,
            format: TexelFormat::Rgba8,
        };
        assert_eq!(key(4, 2, 3).layer_bytes(), (8 + 2 + 1) * 4);
        assert_eq!(key(1, 1, 1).layer_bytes(), 4);
        assert_eq!(key(64, 64, 1).layer_bytes(), 64 * 64 * 4);
    }

    #[test]
    fn bc7_pages_hold_whole_blocks() {
        let key = |width, height, mips| PageKey {
            width,
            height,
            mips,
            format: TexelFormat::Bc7,
        };
        // a quarter of RGBA8 at the large levels; the levels under 4 texels
        // still take one block each
        assert_eq!(key(64, 64, 1).layer_bytes(), 64 * 64);
        assert_eq!(key(8, 8, 4).layer_bytes(), (4 + 1 + 1 + 1) * 16);
        // 100 × 60: 50 × 30 is 13 × 8 blocks
        assert_eq!(key(100, 60, 2).layer_bytes(), (25 * 15 + 13 * 8) * 16);
        // level 0 must be whole blocks, in its own pages
        let levels = |w, h| [(w, h, true), (w / 2, h / 2, true)].into_iter();
        assert_eq!(chain_key(TexelFormat::Bc7, levels(64, 32), 16384), Some(key(64, 32, 2)));
        assert_eq!(chain_key(TexelFormat::Bc7, levels(66, 32), 16384), None);
        assert_ne!(
            chain_key(TexelFormat::Bc7, levels(64, 32), 16384),
            chain_key(TexelFormat::Rgba8, levels(64, 32), 16384)
        );
    }

    #[test]
    fn locations_pack_page_and_layer() {
        for (page, layer) in [(0, 0), (1, 0), (0, 2047), (16383, 2047), (65535, 65535)] {
            assert_eq!(unpack(pack(page, layer)), (page, layer));
        }
        assert_eq!(pack(3, 5), (3 << 16) | 5);
        assert_eq!(pack(0, 0), WHITE_LOC);
    }

    #[test]
    fn the_table_has_more_slots_than_the_device_binds_pages() {
        // the slots do not follow what the device binds
        assert_eq!(slot_capacity(None), SLOTS);
        const { assert!(SLOTS > 16384) };
        // (the pages do: every element at most, fewer when all are bound)
        assert_eq!(page_capacity(16384, true), 16384);
        assert_eq!(page_capacity(16384, false), UNPARTIAL_PAGES);
        assert_eq!(page_capacity(64, false), 64);
        assert_eq!(page_capacity(1 << 20, true), MAX_PAGES);
        assert_eq!(page_capacity(0, true), 1);
        // AURORA_TEXTURE_SLOTS: a smaller or larger table, within bounds
        assert_eq!(slot_capacity(Some("16384")), 16384);
        assert_eq!(slot_capacity(Some(" 200 ")), 200);
        assert_eq!(slot_capacity(Some("0")), RESERVED + 1);
        assert_eq!(slot_capacity(Some("4000000000")), MAX_SLOTS);
        assert_eq!(slot_capacity(Some("many")), SLOTS);
    }

    /// A store with its reserved slots and no page behind them (no device):
    /// enough for what only hands slots out.
    fn bare_store(capacity: u32, page_capacity: u32) -> PageStore {
        let mut store = PageStore::new(capacity, page_capacity, MAX_LAYERS, 4096);
        for _ in 0..RESERVED {
            store.slots.push(Some(Slot {
                key: PageKey::of_chain(TexelFormat::Rgba8, 1, 1, 1),
                page: 0,
                layer: 0,
                shared: false,
            }));
        }
        store
    }

    #[test]
    fn textures_waiting_for_their_texels_take_a_slot_and_no_page() {
        // a device that binds 64 pages, and more textures in use than the
        // 16,384 the table once stopped at
        let mut store = bare_store(slot_capacity(None), page_capacity(64, true));
        let waiting: Vec<u32> = (0..20_000).map(|_| store.create_placeholder().expect("slot")).collect();
        assert_eq!(waiting.last(), Some(&(RESERVED + 19_999)));
        assert_eq!((store.live, store.pages.len(), store.bytes), (20_000, 0, 0));
        // up to its last slot; then none, until one is freed
        while store.create_placeholder().is_some() {}
        assert_eq!(store.live, SLOTS - RESERVED);
        store.free(waiting[7]);
        assert_eq!(store.create_placeholder(), Some(waiting[7]));
        assert_eq!(store.create_placeholder(), None);
        // the reserved slots are never handed out nor freed
        store.free(WHITE);
        assert_eq!(store.create_placeholder(), None);
    }

    #[test]
    fn a_texture_that_will_not_come_leaves_the_loading_grey() {
        let mut store = bare_store(64, page_capacity(64, true));
        for (slot, layer) in [(PLACEHOLDER, 4), (MISSING, 5)] {
            if let Some(Some(s)) = store.slots.get_mut(slot as usize) {
                s.layer = layer;
            }
            store.set_loc(slot, pack(0, layer));
        }
        let waiting = store.create_placeholder().expect("slot");
        let other = store.create_placeholder().expect("slot");
        assert_eq!(store.loc[waiting as usize], store.loc[PLACEHOLDER as usize]);
        // the missing grey, without a layer of its own; the others still load
        let (bytes, live) = (store.bytes, store.live);
        assert!(store.show_missing(waiting));
        assert_eq!(store.loc[waiting as usize], store.loc[MISSING as usize]);
        assert_ne!(store.loc[waiting as usize], store.loc[PLACEHOLDER as usize]);
        assert_eq!(store.loc[other as usize], store.loc[PLACEHOLDER as usize]);
        assert_eq!((store.bytes, store.live), (bytes, live));
        assert!(store.slots[waiting as usize].as_ref().is_some_and(|s| s.shared));
        // never a reserved slot, a free one or one with texels of its own
        assert!(!store.show_missing(PLACEHOLDER));
        assert!(!store.show_missing(WHITE));
        assert!(!store.show_missing(63));
        if let Some(Some(s)) = store.slots.get_mut(other as usize) {
            s.shared = false;
            s.layer = 9;
        }
        assert!(!store.show_missing(other));
        // freed, it is white again and handed out as a placeholder
        store.free(waiting);
        assert_eq!(store.loc[waiting as usize], WHITE_LOC);
        assert_eq!(store.create_placeholder(), Some(waiting));
        assert_eq!(store.loc[waiting as usize], store.loc[PLACEHOLDER as usize]);
    }

    /// Layers of successive pages of one size, each sized from the
    /// pages before it.
    fn growth(layer_bytes: u64, pages: usize) -> Vec<u32> {
        let mut allocated = 0;
        (0..pages)
            .map(|_| {
                let layers = page_layers(layer_bytes, allocated, 2048);
                allocated += layers as u64 * layer_bytes;
                layers
            })
            .collect()
    }

    #[test]
    fn the_key_of_a_regular_chain_is_its_page_key() {
        for (w, h) in [(1024u32, 1024u32), (512, 256), (384, 384), (2048, 2), (1, 1)] {
            let full = 32 - w.max(h).leading_zeros();
            // the whole chain, a cut one, and levels past 1×1 (ignored)
            for levels in [full, full.min(3), full + 2] {
                let dims = (0..levels).map(|l| (level_dim(w, l), level_dim(h, l), true));
                assert_eq!(
                    chain_key(TexelFormat::Rgba8, dims, 16384),
                    Some(PageKey::of_chain(TexelFormat::Rgba8, w, h, levels)),
                    "{w}×{h}, {levels} levels"
                );
            }
        }
    }

    #[test]
    fn the_next_page_is_asked_before_the_layers_run_out() {
        // nothing waits behind this upload: no page ahead
        assert!(!wants_spare(3, 0, 0));
        // more uploads waiting than layers left once this one has its own
        assert!(wants_spare(3, 0, 3));
        assert!(!wants_spare(3, 0, 2));
        // a spare page is there and stays one: never a second
        assert!(!wants_spare(1, 6, 100));
        // this upload starts the spare page: the next one if the queue fills it
        assert!(wants_spare(0, 6, 6));
        assert!(!wants_spare(0, 6, 5));
        // a page of one layer (2048²): one ahead as long as uploads wait
        assert!(wants_spare(0, 1, 1));
        assert!(!wants_spare(0, 1, 0));
    }

    #[test]
    fn a_page_is_asked_once_until_it_comes_or_is_given_up() {
        let key = |width| PageKey {
            width,
            height: 64,
            mips: 7,
            format: TexelFormat::Rgba8,
        };
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut requests = PageRequests::default();
        assert_eq!(requests.ask(key(64), t0), Ask::Send);
        assert_eq!(requests.ask(key(64), t0 + ms(10)), Ask::Wait);
        // each size has its own
        assert_eq!(requests.ask(key(128), t0 + ms(10)), Ask::Send);
        // received: the next page of that size is a new request
        requests.done(key(64));
        assert_eq!(requests.ask(key(64), t0 + ms(20)), Ask::Send);
        // no answer: the caller creates the page, and may ask again later
        assert_eq!(requests.ask(key(64), t0 + ms(19) + REQUEST_TIMEOUT), Ask::Wait);
        assert_eq!(requests.ask(key(64), t0 + ms(20) + REQUEST_TIMEOUT), Ask::GiveUp);
        assert_eq!(requests.ask(key(64), t0 + ms(21) + REQUEST_TIMEOUT), Ask::Send);
        assert_eq!(requests.ask(key(128), t0 + ms(30)), Ask::Wait);
    }

    #[test]
    fn pages_grow_from_small_to_large() {
        // 1024² with mips (5.3 MB a layer): single layers, then up to 32 MB
        let big = PageKey {
            width: 1024,
            height: 1024,
            mips: 11,
            format: TexelFormat::Rgba8,
        }
        .layer_bytes();
        assert_eq!(growth(big, 12), vec![1, 1, 1, 1, 2, 3, 4, 6, 6, 6, 6, 6]);
        // 64² with mips (21 KB a layer): 2 MB pages, then half the total
        let small = PageKey {
            width: 64,
            height: 64,
            mips: 7,
            format: TexelFormat::Rgba8,
        }
        .layer_bytes();
        assert_eq!(growth(small, 6), vec![96, 96, 96, 144, 216, 324]);
        // the free layers of a new page stay under a third of the size's memory
        let mut allocated = 0u64;
        for _ in 0..40 {
            let page = page_layers(small, allocated, 2048) as u64 * small;
            allocated += page;
            assert!(page <= PAGE_FIRST_BYTES.max(allocated / 3 + small), "{page} of {allocated}");
        }
        // tiny textures are bounded by the layer limit, huge ones get one
        assert_eq!(page_layers(4, 0, 2048), 2048);
        assert_eq!(page_layers(4, 0, 256), 256);
        assert_eq!(page_layers(small, 1 << 40, 2048), (PAGE_MAX_BYTES / small) as u32);
        assert_eq!(page_layers(small, 1 << 40, 1024), 1024);
        assert_eq!(page_layers(100 << 20, 1 << 30, 2048), 1);
    }

    #[test]
    fn layers_are_reused_lowest_first() {
        let mut a = LayerAlloc::new(3);
        assert_eq!([a.alloc(), a.alloc(), a.alloc(), a.alloc()], [Some(0), Some(1), Some(2), None]);
        assert_eq!(a.used(), 3);
        a.free(2);
        a.free(0);
        assert_eq!(a.used(), 1);
        assert_eq!([a.alloc(), a.alloc(), a.alloc()], [Some(0), Some(2), None]);
    }

    #[test]
    fn the_fullest_page_with_room_is_filled_first() {
        assert_eq!(pick_page([(1, 3, 4), (2, 10, 16), (3, 16, 16)].into_iter()), Some(2));
        // equal use: the lowest page
        assert_eq!(pick_page([(5, 2, 4), (4, 2, 8)].into_iter()), Some(4));
        assert_eq!(pick_page([(1, 4, 4)].into_iter()), None);
        assert_eq!(pick_page(std::iter::empty()), None);
    }

    #[test]
    fn compaction_empties_a_sparse_page_that_fits_elsewhere() {
        let kb = 1024;
        // a quarter used, fits in the free layers of page 1
        assert_eq!(compaction_pick(&[(1, 10, 16), (2, 4, 16)], kb, false), Some((2, 4)));
        // the emptiest candidate first
        assert_eq!(compaction_pick(&[(1, 2, 16), (2, 3, 16), (3, 10, 64)], kb, false), Some((1, 2)));
        // more than a quarter used
        assert_eq!(compaction_pick(&[(1, 10, 16), (2, 5, 16)], kb, false), None);
        // no room in the other pages
        assert_eq!(compaction_pick(&[(1, 15, 16), (2, 2, 16)], kb, false), None);
        // page 0 (reserved slots) never moves
        assert_eq!(compaction_pick(&[(0, 1, 2048), (1, 10, 16)], kb, false), None);
        // too much to copy at once (4 × 8 MB)
        assert_eq!(compaction_pick(&[(1, 40, 64), (2, 4, 16)], 8 << 20, false), None);
        assert_eq!(compaction_pick(&[(1, 1, 64), (2, 4, 16)], 8 << 20, false), Some((1, 1)));
    }

    #[test]
    fn eager_compaction_takes_any_page_that_fits_elsewhere() {
        let kb = 1024;
        // more than a quarter used: left alone as a rule, emptied under
        // memory pressure
        assert_eq!(compaction_pick(&[(1, 10, 16), (2, 5, 16)], kb, true), Some((2, 5)));
        // half-empty pages left by lowered textures: one goes into the other
        assert_eq!(compaction_pick(&[(1, 3, 5), (2, 2, 5)], 5 << 20, true), Some((2, 2)));
        // still only when the others have the room
        assert_eq!(compaction_pick(&[(1, 15, 16), (2, 2, 16)], kb, true), None);
        assert_eq!(compaction_pick(&[(1, 4, 5), (2, 4, 5)], 5 << 20, true), None);
        // page 0 never moves (the others may move into it), and never more
        // than a whole page to copy (5 × 8 MB)
        assert_eq!(compaction_pick(&[(0, 9, 16), (1, 10, 16)], kb, true), None);
        assert_eq!(compaction_pick(&[(0, 1, 2048), (1, 10, 16)], kb, true), Some((1, 10)));
        assert_eq!(compaction_pick(&[(1, 5, 64), (2, 60, 64)], 8 << 20, true), None);
        assert_eq!(compaction_pick(&[(1, 4, 64), (2, 60, 64)], 8 << 20, true), Some((1, 4)));
    }

    #[test]
    fn emptied_pages_are_destroyed_at_a_steady_pace() {
        let ms = Duration::from_millis;
        // a fast frame: one page, so that the backlog always shrinks
        assert_eq!(pages_to_destroy(300, ms(2)), 1);
        assert_eq!(pages_to_destroy(300, ms(5)), 1);
        // 30 frames a second: as many pages a second as at 200
        assert_eq!(pages_to_destroy(300, ms(33)), 8);
        assert_eq!(pages_to_destroy(300, ms(17)), 4);
        // a long frame does not release a burst
        assert_eq!(pages_to_destroy(300, ms(500)), PAGES_DESTROYED_PER_FRAME);
        assert_eq!(pages_to_destroy(3, ms(500)), 3);
        assert_eq!(pages_to_destroy(0, ms(500)), 0);
    }

    #[test]
    fn bound_length_follows_the_last_live_page() {
        assert_eq!(bound_for(None, 16384, true), 1);
        assert_eq!(bound_for(Some(0), 16384, true), 1);
        assert_eq!(bound_for(Some(299), 16384, true), 300);
        // without partially bound arrays every element is bound
        assert_eq!(bound_for(Some(3), 2048, false), 2048);
    }

    #[test]
    fn mips_chain() {
        let m = build_mips(8, 4, vec![255u8; 8 * 4 * 4], 16);
        let dims: Vec<_> = m.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(dims, vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
        assert!(m.iter().all(|(w, h, d)| d.len() == (w * h * 4) as usize));
        assert_eq!(m[3].2, vec![255, 255, 255, 255]);
    }

    #[test]
    fn shrunk_key_drops_the_finest_levels() {
        let key = PageKey {
            width: 1024,
            height: 512,
            mips: 11,
            format: TexelFormat::Rgba8,
        };
        assert_eq!(
            shrunk_key(key, 2),
            Some(PageKey {
                width: 256,
                height: 128,
                mips: 9,
                format: TexelFormat::Rgba8,
            })
        );
        // down to the last level, never past it
        assert_eq!(
            shrunk_key(key, 10),
            Some(PageKey {
                width: 1,
                height: 1,
                mips: 1,
                format: TexelFormat::Rgba8,
            })
        );
        assert_eq!(shrunk_key(key, 11), None);
        assert_eq!(shrunk_key(key, 0), None);
        // sizes that are not powers of two halve as their mip chain does
        let odd = PageKey {
            width: 48,
            height: 6,
            mips: 6,
            format: TexelFormat::Rgba8,
        };
        assert_eq!(
            shrunk_key(odd, 3),
            Some(PageKey {
                width: 6,
                height: 1,
                mips: 3,
                format: TexelFormat::Rgba8,
            })
        );
        // the memory of a layer falls by about four per level dropped
        let small = shrunk_key(key, 1).expect("one level dropped");
        assert!(small.layer_bytes() * 4 >= key.layer_bytes() && small.layer_bytes() * 3 < key.layer_bytes());
    }

    /// Texels of test texture `i` at a level: a value proper to the texture,
    /// the level and the texel.
    fn texels(i: u32, w: u32, h: u32, level: u32) -> Vec<u8> {
        let (lw, lh) = (level_dim(w, level), level_dim(h, level));
        (0..lw * lh)
            .flat_map(|t| [i as u8, (i >> 8) as u8, level as u8 * 16 + (t % 16) as u8, (t / 16) as u8])
            .collect()
    }

    /// What the render thread does with a frame: the journal in order, then
    /// the frame's copies, then the emptied staging chunks mapped again.
    fn run_frame(queue: &wgpu::Queue, writes: &RefCell<GpuWrites>, uploads: &mut UploadQueue) {
        writes.borrow_mut().replay(&mut QueueSink(queue));
        let mut frame = uploads.take_frame();
        queue.submit(frame.take_commands());
        frame.after_submit();
    }

    /// Read every level of a slot back from its page.
    fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, store: &PageStore, slot: u32) -> Vec<Vec<u8>> {
        let s = store.slots[slot as usize].as_ref().expect("live slot");
        let page = store.page(s.page).expect("live page");
        assert_eq!(store.loc[slot as usize], pack(s.page, s.layer));
        let k = s.key;
        // (rows of blocks for a BC7 page: the read-back gives the blocks)
        let rows: u32 = (0..k.mips).map(|l| k.format.rows(level_dim(k.height, l))).sum();
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: rows as u64 * 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut offset = 0u64;
        for l in 0..k.mips {
            let (w, h) = k.format.copy_extent(level_dim(k.width, l), level_dim(k.height, l));
            let h_rows = k.format.rows(h);
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &page.texture,
                    mip_level: l,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: s.layer },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(h_rows),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            offset += h_rows as u64 * 256;
        }
        queue.submit(std::iter::once(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().expect("mapped read-back").to_vec();
        let mut out = Vec::new();
        let mut row = 0usize;
        for l in 0..k.mips {
            let (w, h) = (level_dim(k.width, l), level_dim(k.height, l));
            let (w, h) = (k.format.row_bytes(w) as usize, k.format.rows(h) as usize);
            let mut level = Vec::new();
            for y in 0..h {
                let start = (row + y) * 256;
                level.extend_from_slice(&data[start..start + w]);
            }
            row += h;
            out.push(level);
        }
        out
    }

    /// GPU round trip: textures created, upgraded (in place and to another
    /// size), freed, and a sparse page compacted; every live slot must read
    /// back its own texels at every level. Needs a GPU, so not run by
    /// check.ps1: `cargo test -p aurora-render pages_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn pages_round_trip_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        // default limits: 256 layers, so 16×16 textures fill pages of 256
        let max_layers = device.limits().max_texture_array_layers.min(MAX_LAYERS);
        assert_eq!(max_layers, 256);
        let mut store = PageStore::new(4096, 64, max_layers, 4096);
        let writes = RefCell::new(GpuWrites::default());
        for _ in 0..RESERVED {
            store.create(
                &device,
                &mut writes.borrow_mut(),
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &[255; 4],
                }],
            );
        }
        let upload = |store: &mut PageStore, i: u32, size: u32, slot: Option<u32>| {
            let levels: Vec<Vec<u8>> = (0..5).map(|l| texels(i, size, size, l)).collect();
            let mips: Vec<MipLevel> = levels
                .iter()
                .enumerate()
                .map(|(l, d)| MipLevel {
                    width: level_dim(size, l as u32),
                    height: level_dim(size, l as u32),
                    data: d,
                })
                .collect();
            match slot {
                Some(s) => {
                    assert!(store.replace(&device, &mut writes.borrow_mut(), s, &mips));
                    s
                }
                None => store.create(&device, &mut writes.borrow_mut(), &mips).expect("slot"),
            }
        };
        // 600 textures of 16×16 (5 levels): pages 1, 2 (256 each) and 3
        let mut live: Vec<(u32, u32, u32)> = (0..600).map(|i| (upload(&mut store, i, 16, None), i, 16)).collect();
        assert_eq!(store.by_key.len(), 2);
        // free every other one: page 3 is left with 44 of 256 layers
        let (gone, kept): (Vec<_>, Vec<_>) = live.into_iter().partition(|&(_, i, _)| i % 2 == 0);
        for (slot, _, _) in gone {
            store.free(slot);
            assert_eq!(store.loc[slot as usize], WHITE_LOC);
        }
        live = kept;
        // upgrades: same size in place, then a larger size (moves page)
        for entry in live.iter_mut().take(20) {
            entry.1 += 1000;
            upload(&mut store, entry.1, 16, Some(entry.0));
        }
        for entry in live.iter_mut().skip(20).take(20) {
            entry.1 += 2000;
            entry.2 = 32;
            upload(&mut store, entry.1, 32, Some(entry.0));
        }
        let before = store.page(3).map(|p| p.layers.used());
        assert!(before.is_some_and(|u| u * 4 <= 256), "{before:?}");
        let mut uploads = UploadQueue::new(&device, true);
        store.compact(&device, &mut uploads);
        run_frame(&queue, &writes, &mut uploads);
        assert!(store.page(3).is_none(), "page 3 emptied and dropped");
        assert!(store.dropped);
        for &(slot, i, size) in &live {
            let got = read_back(&device, &queue, &store, slot);
            for (l, level) in got.iter().enumerate() {
                assert_eq!(level, &texels(i, size, size, l as u32), "slot {slot} texture {i} level {l}");
            }
        }
        assert_eq!(store.live as usize, live.len() + RESERVED as usize);
    }

    /// GPU check of the reductions: textures lose their finest levels on the
    /// GPU and must read back, in their smaller layer, the texels their
    /// coarser levels held; the layers they leave are reused by new textures
    /// in the same frame without spoiling the copies, their memory is given
    /// back, and what cannot be reduced is refused.
    /// `cargo test -p aurora-render shrink -- --ignored`.
    #[test]
    #[ignore]
    fn textures_shrink_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let max_layers = device.limits().max_texture_array_layers.min(MAX_LAYERS);
        let mut store = PageStore::new(4096, 64, max_layers, 4096);
        let writes = RefCell::new(GpuWrites::default());
        for _ in 0..RESERVED {
            store.create(
                &device,
                &mut writes.borrow_mut(),
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &[255; 4],
                }],
            );
        }
        // 64×32 textures with their whole chain (7 levels)
        let create = |store: &mut PageStore, i: u32| {
            let levels: Vec<Vec<u8>> = (0..7).map(|l| texels(i, 64, 32, l)).collect();
            let mips: Vec<MipLevel> = levels
                .iter()
                .enumerate()
                .map(|(l, d)| MipLevel {
                    width: level_dim(64, l as u32),
                    height: level_dim(32, l as u32),
                    data: d,
                })
                .collect();
            store.create(&device, &mut writes.borrow_mut(), &mips).expect("slot")
        };
        // (the reserved slots count here; the table leaves them out)
        let reserved = store.bytes;
        let slots: Vec<u32> = (0..40).map(|i| create(&mut store, i)).collect();
        let mut uploads = UploadQueue::new(&device, true);
        run_frame(&queue, &writes, &mut uploads);
        let full = store.bytes - reserved;
        let layer = PageKey {
            width: 64,
            height: 32,
            mips: 7,
            format: TexelFormat::Rgba8,
        }
        .layer_bytes();
        assert_eq!(full, 40 * layer);
        // every other texture loses 1 to 3 levels
        let dropped = |i: u32| if i.is_multiple_of(2) { 1 + i / 2 % 3 } else { 0 };
        for (i, &slot) in slots.iter().enumerate() {
            let k = dropped(i as u32);
            assert_eq!(store.shrink(&device, &mut uploads, slot, k), k > 0, "texture {i}");
        }
        // what cannot be reduced: the whole chain, a reserved slot, a
        // placeholder, a free slot
        assert!(!store.shrink(&device, &mut uploads, slots[1], 7));
        assert!(!store.shrink(&device, &mut uploads, WHITE, 1));
        let waiting = store.create_placeholder().expect("placeholder slot");
        assert!(!store.shrink(&device, &mut uploads, waiting, 1));
        store.free(waiting);
        assert!(!store.shrink(&device, &mut uploads, waiting, 1));
        // new textures take the layers just left (staged uploads are copies
        // recorded after the reductions, in the same encoder)
        let chains: Vec<(u32, StagedTexture)> = (100..110)
            .map(|i| {
                let levels: Vec<Vec<u8>> = (0..7).map(|l| texels(i, 64, 32, l)).collect();
                let chain: Vec<(u32, u32, &[u8])> = levels
                    .iter()
                    .enumerate()
                    .map(|(l, d)| (level_dim(64, l as u32), level_dim(32, l as u32), d.as_slice()))
                    .collect();
                (i, StagedTexture::new(uploads.pool(), &chain).expect("staging room"))
            })
            .collect();
        uploads.prepare();
        let staged: Vec<(u32, u32)> = chains
            .iter()
            .map(|(i, texture)| {
                assert!(texture.data.ready());
                let slot = store.create_placeholder().expect("slot");
                assert!(store.replace_staged(&device, &mut uploads, slot, texture));
                (slot, *i)
            })
            .collect();
        drop(chains);
        run_frame(&queue, &writes, &mut uploads);
        for (i, &slot) in slots.iter().enumerate() {
            let k = dropped(i as u32);
            let got = read_back(&device, &queue, &store, slot);
            assert_eq!(got.len() as u32, 7 - k, "texture {i}");
            for (l, level) in got.iter().enumerate() {
                assert_eq!(
                    level,
                    &texels(i as u32, 64, 32, l as u32 + k),
                    "texture {i} level {l} after dropping {k}"
                );
            }
        }
        for &(slot, i) in &staged {
            let got = read_back(&device, &queue, &store, slot);
            for (l, level) in got.iter().enumerate() {
                assert_eq!(level, &texels(i, 64, 32, l as u32), "new texture {i} level {l}");
            }
        }
        let small: u64 = (0..40u32)
            .map(|i| {
                shrunk_key(
                    PageKey {
                        width: 64,
                        height: 32,
                        mips: 7,
                        format: TexelFormat::Rgba8,
                    },
                    dropped(i),
                )
                .map_or(layer, |k| k.layer_bytes())
            })
            .sum();
        assert_eq!(store.bytes - reserved, small + 10 * layer);
        assert!(small < full * 3 / 4, "{small} of {full}");
        assert_eq!(store.live as usize, 50 + RESERVED as usize);
    }

    /// GPU check of the shared placeholder: slots of textures still loading
    /// show the one grey layer without owning it; their first texels (even
    /// 1×1 ones, the placeholder's own size) go to a layer of their own and
    /// BC7 chains staged by a job land block for block in pages of their
    /// own, levels under one block and sizes that are not a power of two
    /// included; an RGBA8 chain then moves the slot back to an RGBA8 page.
    /// `cargo test -p aurora-render bc7_pages -- --ignored`.
    #[test]
    #[ignore]
    fn bc7_pages_round_trip_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        if !adapter.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            eprintln!("no BC texture compression: skipped");
            return;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TEXTURE_COMPRESSION_BC,
            ..Default::default()
        }))
        .expect("GPU device");
        let max_layers = device.limits().max_texture_array_layers.min(MAX_LAYERS);
        let mut store = PageStore::new(4096, 64, max_layers, 4096);
        let writes = RefCell::new(GpuWrites::default());
        for slot in 0..RESERVED {
            let created = store.create(
                &device,
                &mut writes.borrow_mut(),
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &[slot as u8; 4],
                }],
            );
            assert_eq!(created, Some(slot));
        }
        let mut uploads = UploadQueue::new(&device, true);
        // blocks proper to the texture, the level and the block
        let blocks = |i: u8, w: u32, h: u32| -> Vec<Vec<u8>> {
            (0..32 - w.max(h).leading_zeros())
                .map(|l| {
                    let n = TexelFormat::Bc7.level_bytes(level_dim(w, l), level_dim(h, l)) as usize;
                    (0..n)
                        .map(|b| i.wrapping_mul(31) ^ (l as u8 * 16) ^ (b / 16) as u8 ^ (b as u8 % 16))
                        .collect()
                })
                .collect()
        };
        let sizes = [(64u32, 64u32), (64, 64), (16, 8), (40, 12), (4, 4)];
        let mut slots = Vec::new();
        for (i, &(w, h)) in sizes.iter().enumerate() {
            let slot = store.create_placeholder().expect("slot");
            let data = blocks(i as u8, w, h);
            let levels: Vec<(u32, u32, &[u8])> = data
                .iter()
                .enumerate()
                .map(|(l, d)| (level_dim(w, l as u32), level_dim(h, l as u32), d.as_slice()))
                .collect();
            let staged = StagedTexture::with_format(uploads.pool(), TexelFormat::Bc7, &levels).expect("staging room");
            uploads.prepare();
            // refused without the device feature, accepted with it
            assert!(!store.replace_staged(&device, &mut uploads, slot, &staged));
            store.bc7 = true;
            assert!(store.replace_staged(&device, &mut uploads, slot, &staged));
            store.bc7 = false;
            drop(staged);
            run_frame(&queue, &writes, &mut uploads);
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            slots.push((slot, data));
        }
        for (slot, data) in &slots {
            assert_eq!(&read_back(&device, &queue, &store, *slot), data, "slot {slot}");
        }
        // the two 64 × 64 share a page, apart from the RGBA8 pages
        let key = |store: &PageStore, slot: u32| store.slots[slot as usize].as_ref().map(|s| (s.page, s.key.format));
        assert_eq!(key(&store, slots[0].0), key(&store, slots[1].0));
        assert_eq!(key(&store, slots[0].0).map(|k| k.1), Some(TexelFormat::Bc7));
        assert_ne!(key(&store, slots[0].0).map(|k| k.0), Some(0));
        // a quarter of the RGBA8 size (64 × 64 with 7 levels: 4 levels of
        // whole blocks, 3 of one block)
        assert_eq!(
            store.slots[slots[0].0 as usize].as_ref().map(|s| s.key.layer_bytes()),
            Some((256 + 64 + 16 + 4 + 1 + 1 + 1) * 16)
        );
        // an RGBA8 chain of the same size: another page, the BC7 layer freed
        let slot = slots[0].0;
        let rgba: Vec<Vec<u8>> = (0..7).map(|l| texels(9, 64, 64, l)).collect();
        let mips: Vec<MipLevel> = rgba
            .iter()
            .enumerate()
            .map(|(l, d)| MipLevel {
                width: level_dim(64, l as u32),
                height: level_dim(64, l as u32),
                data: d,
            })
            .collect();
        assert!(store.replace(&device, &mut writes.borrow_mut(), slot, &mips));
        run_frame(&queue, &writes, &mut uploads);
        assert_eq!(read_back(&device, &queue, &store, slot), rgba);
        assert_eq!(key(&store, slot).map(|k| k.1), Some(TexelFormat::Rgba8));
        assert_eq!(read_back(&device, &queue, &store, slots[1].0), slots[1].1);
        // memory to give back: the finest level dropped on the GPU, the
        // others block for block in a BC7 page of the smaller size
        assert!(store.shrink(&device, &mut uploads, slots[1].0, 1));
        run_frame(&queue, &writes, &mut uploads);
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        assert_eq!(read_back(&device, &queue, &store, slots[1].0), slots[1].1[1..]);
        assert_eq!(
            store.slots[slots[1].0 as usize].as_ref().map(|s| (s.key.width, s.key.format)),
            Some((32, TexelFormat::Bc7))
        );
        // 40 × 12 would start at 20 × 6, which is not whole blocks: kept
        assert!(!store.shrink(&device, &mut uploads, slots[3].0, 1));
        assert_eq!(read_back(&device, &queue, &store, slots[3].0), slots[3].1);
    }

    /// leave it untouched, and freeing them does not release it.
    /// `cargo test -p aurora-render placeholders -- --ignored`.
    #[test]
    #[ignore]
    fn placeholders_share_one_layer_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let max_layers = device.limits().max_texture_array_layers.min(MAX_LAYERS);
        let mut store = PageStore::new(4096, 64, max_layers, 4096);
        // the journal of the frame, replayed before each read-back
        let mut writes = GpuWrites::default();
        const GREY: [u8; 4] = [148, 148, 156, 255];
        for slot in 0..RESERVED {
            let texel = if slot == PLACEHOLDER { GREY } else { [slot as u8; 4] };
            let created = store.create(
                &device,
                &mut writes,
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &texel,
                }],
            );
            assert_eq!(created, Some(slot));
        }
        let (bytes, pages) = (store.bytes, store.pages.len());
        let grey_loc = store.loc[PLACEHOLDER as usize];
        let waiting: Vec<u32> = (0..300).map(|_| store.create_placeholder().expect("slot")).collect();
        // no layer taken, nothing allocated, every slot shows the grey texel
        assert_eq!((store.bytes, store.pages.len()), (bytes, pages));
        assert_eq!(store.live, RESERVED + 300);
        assert!(waiting.iter().all(|&s| store.loc[s as usize] == grey_loc));
        assert_eq!(store.page(0).map(|p| p.layers.used()), Some(RESERVED));
        // (placeholders recorded nothing: the journal only holds the reserved texels)
        writes.replay(&mut QueueSink(&queue));
        assert!(writes.is_empty());
        assert_eq!(read_back(&device, &queue, &store, waiting[7]), vec![GREY.to_vec()]);
        // first texels of a texture: a layer of its own
        let levels: Vec<Vec<u8>> = (0..5).map(|l| texels(42, 16, 16, l)).collect();
        let mips: Vec<MipLevel> = levels
            .iter()
            .enumerate()
            .map(|(l, d)| MipLevel {
                width: level_dim(16, l as u32),
                height: level_dim(16, l as u32),
                data: d,
            })
            .collect();
        assert!(store.needs_page(waiting[0], TexelFormat::Rgba8, 16, 16, 5));
        assert!(store.replace(&device, &mut writes, waiting[0], &mips));
        writes.replay(&mut QueueSink(&queue));
        assert_ne!(store.loc[waiting[0] as usize], grey_loc);
        assert_eq!(read_back(&device, &queue, &store, waiting[0]), levels);
        // a 1×1 texture has the placeholder's size: not written over it
        let one = [MipLevel {
            width: 1,
            height: 1,
            data: &[9, 8, 7, 6],
        }];
        assert!(store.needs_page(waiting[1], TexelFormat::Rgba8, 1, 1, 1) || store.page(0).is_some_and(|p| p.layers.used() < p.layers.len));
        assert!(store.replace(&device, &mut writes, waiting[1], &one));
        writes.replay(&mut QueueSink(&queue));
        assert_ne!(store.loc[waiting[1] as usize], grey_loc);
        assert_eq!(read_back(&device, &queue, &store, waiting[1]), vec![vec![9, 8, 7, 6]]);
        assert_eq!(read_back(&device, &queue, &store, PLACEHOLDER), vec![GREY.to_vec()]);
        assert_eq!(read_back(&device, &queue, &store, waiting[2]), vec![GREY.to_vec()]);
        // the media path refuses to draw into the shared layer
        // (TextureTable::write_region checks `shared`); frees leave it alone
        for &s in &waiting {
            store.free(s);
            assert_eq!(store.loc[s as usize], WHITE_LOC);
        }
        assert_eq!(store.live, RESERVED);
        assert_eq!(store.bytes, bytes);
        assert_eq!(store.page(0).map(|p| p.layers.used()), Some(RESERVED));
        assert_eq!(read_back(&device, &queue, &store, PLACEHOLDER), vec![GREY.to_vec()]);
        // freed slots are handed out again, as placeholders too
        let again = store.create_placeholder().expect("slot");
        assert!(waiting.contains(&again));
        assert_eq!(store.loc[again as usize], grey_loc);
    }

    /// Randomized GPU check of the page store: creations, upgrades to the
    /// same or another size, frees, slot reuse and compaction interleaved
    /// (as streaming and eviction do), with every live slot read back from
    /// its location at regular intervals. `cargo test -p aurora-render
    /// pages_fuzz -- --ignored`.
    #[test]
    #[ignore]
    fn pages_fuzz_on_the_gpu() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        // small pages so that sizes spread over several pages and compact
        let mut store = PageStore::new(4096, 512, 16, 4096);
        let mut uploads = UploadQueue::new(&device, true);
        let writes = RefCell::new(GpuWrites::default());
        for _ in 0..RESERVED {
            store.create(
                &device,
                &mut writes.borrow_mut(),
                &[MipLevel {
                    width: 1,
                    height: 1,
                    data: &[255; 4],
                }],
            );
        }
        let sizes = [(1, 1), (4, 4), (8, 8), (16, 16), (16, 8), (8, 16), (12, 12), (32, 32)];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rand = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let upload = |store: &mut PageStore, uploads: &mut UploadQueue, id: u32, (w, h): (u32, u32), slot: Option<u32>| {
            let count = 32 - w.max(h).leading_zeros();
            let levels: Vec<Vec<u8>> = (0..count).map(|l| texels(id, w, h, l)).collect();
            let mips: Vec<MipLevel> = levels
                .iter()
                .enumerate()
                .map(|(l, d)| MipLevel {
                    width: level_dim(w, l as u32),
                    height: level_dim(h, l as u32),
                    data: d,
                })
                .collect();
            match slot {
                // every other upgrade staged as a decode job does: copies
                // in the upload encoder, submitted with the frame
                Some(s) if id.is_multiple_of(2) => {
                    let staged_levels: Vec<(u32, u32, &[u8])> = mips.iter().map(|m| (m.width, m.height, m.data)).collect();
                    let staged = StagedTexture::new(uploads.pool(), &staged_levels).expect("staging room");
                    uploads.prepare();
                    assert!(staged.data.ready());
                    let done = store.replace_staged(&device, uploads, s, &staged);
                    drop(staged);
                    run_frame(&queue, &writes, uploads);
                    // wait for the GPU: the chunks are mapped again
                    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
                    done.then_some(s)
                }
                Some(s) => store.replace(&device, &mut writes.borrow_mut(), s, &mips).then_some(s),
                None => store.create(&device, &mut writes.borrow_mut(), &mips),
            }
        };
        // slot -> (texture id, size)
        let mut live: std::collections::BTreeMap<u32, (u32, (u32, u32))> = Default::default();
        let mut next_id = 1u32;
        for step in 0..3000 {
            match rand(10) {
                0..=3 => {
                    // a new texture starts as a 1×1 placeholder
                    let slot = upload(&mut store, &mut uploads, next_id, (1, 1), None).expect("slot");
                    assert!(!live.contains_key(&slot), "slot {slot} handed out twice");
                    live.insert(slot, (next_id, (1, 1)));
                    next_id += 1;
                }
                4..=6 if !live.is_empty() => {
                    // upgrade (sometimes to the same size: in place)
                    let slot = *live.keys().nth(rand(live.len())).expect("slot");
                    let size = sizes[rand(sizes.len())];
                    assert_eq!(upload(&mut store, &mut uploads, next_id, size, Some(slot)), Some(slot));
                    live.insert(slot, (next_id, size));
                    next_id += 1;
                }
                7..=8 if !live.is_empty() => {
                    let slot = *live.keys().nth(rand(live.len())).expect("slot");
                    store.free(slot);
                    live.remove(&slot);
                }
                _ => {
                    store.compact(&device, &mut uploads);
                    // a submit in the middle of a frame, kept in its place
                    // by the journal
                    uploads.flush(&mut writes.borrow_mut(), None);
                }
            }
            if step % 250 == 249 {
                run_frame(&queue, &writes, &mut uploads);
                for (&slot, &(id, (w, h))) in &live {
                    let got = read_back(&device, &queue, &store, slot);
                    for (l, level) in got.iter().enumerate() {
                        assert_eq!(level, &texels(id, w, h, l as u32), "step {step} slot {slot} texture {id} level {l}");
                    }
                }
                // owners and slots agree
                for (p, page) in store.pages.iter().enumerate() {
                    let Some(page) = page else { continue };
                    for (layer, &owner) in page.owners.iter().enumerate() {
                        if owner != NO_SLOT {
                            let s = store.slots[owner as usize].as_ref().expect("owner is live");
                            assert_eq!((s.page, s.layer), (p as u32, layer as u32));
                        }
                    }
                }
            }
        }
        assert_eq!(store.live as usize, live.len() + RESERVED as usize);
    }

    /// The page thread on the GPU: a streamed upload waits for the first
    /// page of its size, the next page is ready before the first is full
    /// while uploads wait behind, and a spare page nothing took is
    /// destroyed. `cargo test -p aurora-render page_thread -- --ignored`.
    #[test]
    #[ignore]
    fn pages_come_from_the_page_thread() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | (adapter.features() & wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        let mut writes = GpuWrites::default();
        let mut table = TextureTable::new(&device, &mut writes, SLOTS, 512, 8);
        let mut uploads = UploadQueue::new(&device, true);
        // (the page of the reserved slots was created here)
        let inline = table.pages_made().inline;
        assert_eq!((table.pages(), table.pages_made().threaded), (1, 0));
        // 64² with its mips is 21 KB: pages of 96 layers at first
        const COUNT: u32 = 100;
        let key = PageKey::of_chain(TexelFormat::Rgba8, 64, 64, 7);
        let chain = |i: u32| -> Vec<Vec<u8>> { (0..7).map(|l| texels(i, 64, 64, l)).collect() };
        let slots: Vec<u32> = (0..COUNT).map(|_| table.create_placeholder().expect("slot")).collect();
        // true once the upload of `slot` can go, as the frames of the streamer ask
        let ready = |table: &mut TextureTable, slot: u32, queued: u32| {
            let t0 = Instant::now();
            while !table.prepare_page(slot, (TexelFormat::Rgba8, 64, 64, 7), queued) {
                assert!(t0.elapsed() < Duration::from_secs(5), "no page from the page thread");
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        // no page of that size yet: the page thread makes it, the upload waits
        assert!(!table.prepare_page(slots[0], (TexelFormat::Rgba8, 64, 64, 7), COUNT - 1));
        assert_eq!((table.pages(), table.store.free_layers(key)), (1, 0));
        for (i, &slot) in slots.iter().enumerate() {
            // (the second page was asked with the first upload: 99 waited
            // behind it for the 95 layers left)
            ready(&mut table, slot, COUNT - 1 - i as u32);
            let levels = chain(i as u32);
            let mips: Vec<MipLevel> = levels
                .iter()
                .enumerate()
                .map(|(l, d)| MipLevel {
                    width: level_dim(64, l as u32),
                    height: level_dim(64, l as u32),
                    data: d,
                })
                .collect();
            assert!(table.replace(&device, &mut writes, slot, &mips));
            // in place from now on: no layer, no page
            assert!(table.prepare_page(slot, (TexelFormat::Rgba8, 64, 64, 7), 1000));
        }
        let made = table.pages_made();
        assert_eq!((made.threaded, made.inline, table.pages()), (2, inline, 3));
        assert_eq!((table.store.free_layers(key), table.store.spare_layers(key)), (2 * 96 - COUNT, 0));
        table.maintain(&device, &mut writes, &mut uploads, false);
        writes.replay(&mut QueueSink(&queue));
        for (i, &slot) in slots.iter().enumerate() {
            assert_eq!(read_back(&device, &queue, &table.store, slot), chain(i as u32), "slot {slot}");
        }
        // a page asked for an upload that never came: kept a while, then destroyed
        let unused = PageKey::of_chain(TexelFormat::Rgba8, 32, 32, 6);
        let extra = table.create_placeholder().expect("slot");
        assert!(!table.prepare_page(extra, (TexelFormat::Rgba8, 32, 32, 6), 0));
        let t0 = Instant::now();
        while table.store.spare_layers(unused) == 0 {
            assert!(t0.elapsed() < Duration::from_secs(5), "no page from the page thread");
            std::thread::sleep(Duration::from_millis(1));
            table.maintain(&device, &mut writes, &mut uploads, false);
        }
        assert_eq!((table.pages_made().threaded, table.pages()), (3, 3));
        std::thread::sleep(SPARE_TTL);
        table.maintain(&device, &mut writes, &mut uploads, false);
        assert_eq!((table.store.spare_layers(unused), table.pages()), (0, 3));
        assert_eq!(table.pages_made().unused, 1);
        // asked again when an upload does come
        assert!(!table.prepare_page(extra, (TexelFormat::Rgba8, 32, 32, 6), 0));
    }

    /// The whole table on the GPU as the renderer uses it: random creations,
    /// upgrades, frees and compactions between "frames" (`maintain`), then
    /// every live slot is sampled through the bind group and the location
    /// buffer (one pixel per slot) and must give its texture's first texel.
    /// `cargo test -p aurora-render pages_sampled -- --ignored`.
    #[test]
    #[ignore]
    fn pages_sampled_through_the_bind_group() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let features = wgpu::Features::TEXTURE_BINDING_ARRAY
            | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
            | (adapter.features() & wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("GPU device");
        const SLOTS: u32 = 512;
        let writes = RefCell::new(GpuWrites::default());
        let mut table = TextureTable::new(&device, &mut writes.borrow_mut(), SLOTS, SLOTS, 8);
        let mut uploads = UploadQueue::new(&device, true);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "enable wgpu_binding_array;
                @group(0) @binding(0) var tex_pages: binding_array<texture_2d_array<f32>>;
                @group(0) @binding(1) var tex_sampler: sampler;
                @group(0) @binding(2) var<storage, read> tex_loc: array<u32>;
                @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
                    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
                }
                @fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
                    // row 0: level 0, row 1: the last level (first texel of each)
                    let loc = tex_loc[u32(p.x)];
                    let levels = textureNumLevels(tex_pages[loc >> 16u]);
                    let level = select(0u, levels - 1u, p.y > 1.0);
                    let dims = textureDimensions(tex_pages[loc >> 16u], level);
                    let uv = vec2<f32>(0.5) / vec2<f32>(dims);
                    return textureSampleLevel(tex_pages[loc >> 16u], tex_sampler, uv, loc & 0xffffu, f32(level));
                }"
                .into(),
            ),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&table.layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: SLOTS,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());
        let sample_all = |table: &TextureTable| -> Vec<u8> {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: SLOTS as u64 * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &table.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
            enc.copy_texture_to_buffer(
                target.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SLOTS * 4),
                        rows_per_image: Some(2),
                    },
                },
                wgpu::Extent3d {
                    width: SLOTS,
                    height: 2,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(std::iter::once(enc.finish()));
            buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            buf.slice(..).get_mapped_range().expect("mapped read-back").to_vec()
        };
        let sizes = [
            (1, 1),
            (4, 4),
            (64, 64),
            (128, 128),
            (128, 64),
            (12, 12),
            (256, 256),
            (1024, 1024),
            (2048, 2048),
            (2048, 1024),
            (512, 1024),
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rand = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let upload = |table: &mut TextureTable, id: u32, (w, h): (u32, u32), slot: Option<u32>| {
            let levels: Vec<Vec<u8>> = (0..32 - w.max(h).leading_zeros()).map(|l| texels(id, w, h, l)).collect();
            let mips: Vec<MipLevel> = levels
                .iter()
                .enumerate()
                .map(|(l, d)| MipLevel {
                    width: level_dim(w, l as u32),
                    height: level_dim(h, l as u32),
                    data: d,
                })
                .collect();
            match slot {
                Some(s) => table.replace(&device, &mut writes.borrow_mut(), s, &mips).then_some(s),
                None => table.create(&device, &mut writes.borrow_mut(), &mips),
            }
        };
        let mut live: std::collections::BTreeMap<u32, (u32, (u32, u32))> = Default::default();
        let mut next_id = 1u32;
        // More distinct page keys than Metal's ordinary 96 texture slots.
        // Every page is sampled through the production binding-array layout.
        for width in 1..=128 {
            let slot = upload(&mut table, next_id, (width, 1), None).expect("bindless page");
            live.insert(slot, (next_id, (width, 1)));
            next_id += 1;
        }
        assert!(table.store.pages.len() > 96);
        for frame in 0..400 {
            for _ in 0..rand(12) {
                match rand(10) {
                    0..=3 if live.len() < SLOTS as usize - 8 => {
                        let slot = upload(&mut table, next_id, (1, 1), None).expect("slot");
                        live.insert(slot, (next_id, (1, 1)));
                        next_id += 1;
                    }
                    4..=6 if !live.is_empty() => {
                        let slot = *live.keys().nth(rand(live.len())).expect("slot");
                        let size = sizes[rand(sizes.len())];
                        assert_eq!(upload(&mut table, next_id, size, Some(slot)), Some(slot));
                        live.insert(slot, (next_id, size));
                        next_id += 1;
                    }
                    7..=9 if !live.is_empty() => {
                        let slot = *live.keys().nth(rand(live.len())).expect("slot");
                        table.free(slot);
                        live.remove(&slot);
                    }
                    _ => {}
                }
            }
            if frame % 7 == 0 {
                // let the throttled compaction run this frame
                table.last_compact = Instant::now() - COMPACT_INTERVAL;
            }
            table.maintain(&device, &mut writes.borrow_mut(), &mut uploads, false);
            run_frame(&queue, &writes, &mut uploads);
            let px = sample_all(&table);
            for (&slot, &(id, (w, h))) in &live {
                let at = |row: u32| &px[(row * SLOTS + slot) as usize * 4..][..4];
                let last = 31 - w.max(h).leading_zeros();
                assert_eq!(at(0), &texels(id, w, h, 0)[..4], "frame {frame} slot {slot} texture {id}");
                assert_eq!(
                    at(1),
                    &texels(id, w, h, last)[..4],
                    "frame {frame} slot {slot} texture {id} level {last}"
                );
            }
            assert_eq!(&px[..4], &[255, 255, 255, 255]);
        }
        eprintln!("{} live, {} pages", live.len(), table.pages());
    }
}
