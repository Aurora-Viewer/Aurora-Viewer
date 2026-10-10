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

use crate::upload::{StagedTexture, UploadQueue};
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
const RESERVED: u32 = 5;
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
/// Emptied pages are handed to the reaper thread one every this long (and
/// at least one, at most [`PAGES_DESTROYED_PER_FRAME`], a frame). Once the
/// bind group no longer holds a page, dropping it frees its video memory:
/// ~0.4 ms each on the thread that drops it. The pages a rebuild released
/// went together (1 to 4 ms in the frames where textures move to a finer
/// level), and the ~300 pages of a region left by teleport made one
/// submit of 18 to 50 ms. Paced, because the render thread meets the
/// reaper on the device's memory allocator at each queue write.
/// With the frames drawn by the render thread (render_thread.rs), the frame
/// in flight still holds the bind group of before the rebuild in its
/// packet: handles are reference counted, so a page handed to the reaper
/// meanwhile lives until that frame ends (it is then freed there, by the
/// render thread, never under a draw that reads it).
const PAGE_DESTROY_INTERVAL: Duration = Duration::from_millis(4);
const PAGES_DESTROYED_PER_FRAME: usize = 8;

/// Pages to hand to the reaper `since_last` after the previous ones.
fn pages_to_destroy(waiting: usize, since_last: Duration) -> usize {
    let due = (since_last.as_micros() / PAGE_DESTROY_INTERVAL.as_micros()) as usize;
    waiting.min(due.clamp(1, PAGES_DESTROYED_PER_FRAME))
}

type DeadPage = (wgpu::Texture, wgpu::TextureView);

/// Destroys emptied pages on a thread of its own, a few per frame.
struct PageReaper {
    /// None when the thread could not start: pages are then dropped by the
    /// caller.
    tx: Option<std::sync::mpsc::Sender<DeadPage>>,
}

impl PageReaper {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<DeadPage>();
        let spawned = std::thread::Builder::new().name("aurora-pages".into()).spawn(move || {
            for page in rx {
                drop(page);
            }
        });
        match spawned {
            Ok(_) => PageReaper { tx: Some(tx) },
            Err(e) => {
                log::warn!("texture pages: reaper thread not started ({e}); pages are destroyed on the render thread");
                PageReaper { tx: None }
            }
        }
    }

    fn destroy(&self, page: DeadPage) {
        if let Some(tx) = &self.tx {
            // (a page that cannot be sent comes back in the error and is
            // dropped here)
            let _ = tx.send(page);
        }
    }
}
const NO_SLOT: u32 = u32::MAX;

/// Mip level data, tightly packed RGBA8.
pub struct MipLevel<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

/// What makes textures share a page: level-0 size and mip count (always
/// RGBA8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PageKey {
    width: u32,
    height: u32,
    mips: u32,
}

impl PageKey {
    /// Bytes of one layer with its mip chain.
    fn layer_bytes(&self) -> u64 {
        (0..self.mips)
            .map(|i| level_dim(self.width, i) as u64 * level_dim(self.height, i) as u64 * 4)
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
        mips.iter()
            .map(|m| (m.width, m.height, m.data.len() >= m.width as usize * m.height as usize * 4)),
        max_dim,
    )
}

/// Page key of a chain given as (width, height, data complete) per level.
fn chain_key(mut levels: impl Iterator<Item = (u32, u32, bool)>, max_dim: u32) -> Option<PageKey> {
    let (w, h, complete) = levels.next()?;
    if w == 0 || h == 0 || w > max_dim || h > max_dim || !complete {
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
fn compaction_pick(pages: &[(u32, u32, u32)], layer_bytes: u64) -> Option<(u32, u32)> {
    let free: u32 = pages.iter().map(|&(_, used, len)| len - used).sum();
    pages
        .iter()
        .filter(|&&(page, used, len)| {
            page != 0 && used * 4 <= len && used as u64 * layer_bytes <= COMPACT_MAX_BYTES && used <= free - (len - used)
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
    warned_full: bool,
    /// CPU time spent creating pages, accumulated (AURORA_PROFILE).
    create_ms: f32,
    /// Emptied pages the current bind group may still hold: taken by
    /// `TextureTable::maintain` once it is rebuilt.
    retired: Vec<DeadPage>,
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
            loc: vec![WHITE_LOC; capacity as usize],
            loc_dirty: None,
            bytes: 0,
            live: 0,
            page_bytes: 0,
            created: false,
            dropped: false,
            freed: false,
            warned_full: false,
            create_ms: 0.0,
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

    fn new_page(&mut self, device: &wgpu::Device, key: PageKey) -> Option<u32> {
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
        let allocated: u64 = self.by_key.get(&key).map_or(0, |ids| {
            ids.iter()
                .filter_map(|&p| self.page(p))
                .map(|p| p.layers.len as u64 * key.layer_bytes())
                .sum()
        });
        let layers = page_layers(key.layer_bytes(), allocated, self.max_layers);
        let t0 = Instant::now();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture page"),
            size: wgpu::Extent3d {
                width: key.width,
                height: key.height,
                depth_or_array_layers: layers,
            },
            mip_level_count: key.mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // COPY_SRC: compaction copies layers between pages
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        // explicit: the default view of a one-layer texture would be D2
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.create_ms += ms;
        if ms > 4.0 {
            // a hitch worth explaining (video memory under pressure?)
            log::info!(
                "texture page {}×{} ({} layers, {} MB) created in {ms:.1} ms",
                key.width,
                key.height,
                layers,
                (layers as u64 * key.layer_bytes()) >> 20
            );
        }
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
        let Some(key) = chain_key(staged.levels.iter().map(|l| (l.width, l.height, true)), self.max_dim) else {
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
    fn needs_page(&self, slot: u32, width: u32, height: u32, levels: u32) -> bool {
        // the chain kept by `chain_key` for a regular chain
        let mips = levels.min(32 - width.max(height).max(1).leading_zeros());
        let key = PageKey { width, height, mips };
        if self
            .slots
            .get(slot as usize)
            .and_then(Option::as_ref)
            .is_some_and(|s| s.key == key && !s.shared)
        {
            return false;
        }
        !self
            .by_key
            .get(&key)
            .is_some_and(|ids| ids.iter().any(|&p| self.page(p).is_some_and(|pg| pg.layers.used() < pg.layers.len)))
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

    /// Empty one sparse page into the other pages of its size (GPU copies of
    /// every level, submitted before the frame that sees the new locations).
    fn compact(&mut self, device: &wgpu::Device, uploads: &mut UploadQueue) {
        if !std::mem::take(&mut self.freed) {
            return;
        }
        let candidate = self.by_key.iter().filter(|(_, ids)| ids.len() > 1).find_map(|(key, ids)| {
            let pages: Vec<(u32, u32, u32)> = ids
                .iter()
                .filter_map(|&p| self.page(p).map(|pg| (p, pg.layers.used(), pg.layers.len)))
                .collect();
            compaction_pick(&pages, key.layer_bytes()).map(|(page, _)| (page, *key))
        });
        let Some((src, key)) = candidate else {
            return;
        };
        let Some(moves) = self.page(src).map(|p| {
            p.owners
                .iter()
                .enumerate()
                .filter(|&(_, &s)| s != NO_SLOT)
                .map(|(l, &s)| (l as u32, s))
                .collect::<Vec<_>>()
        }) else {
            return;
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
                            width: level_dim(key.width, level),
                            height: level_dim(key.height, level),
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
    /// frame by the reaper (`pages_to_destroy`).
    doomed: std::collections::VecDeque<DeadPage>,
    reaper: PageReaper,
    last_destroy: Instant,
}

impl TextureTable {
    pub fn new(device: &wgpu::Device, writes: &mut GpuWrites, capacity: u32, anisotropy: u16) -> Self {
        let partial = device.features().contains(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY);
        let limits = device.limits();
        let capacity = capacity.max(RESERVED + 1);
        let page_capacity = if partial { capacity } else { capacity.min(UNPARTIAL_PAGES) }.min(MAX_PAGES);
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
        for (slot, rgba) in [
            (WHITE, [255u8, 255, 255, 255]),
            (FLAT_NORMAL, [128, 128, 255, 255]),
            (BLACK, [0, 0, 0, 255]),
            (TRANSPARENT, [0, 0, 0, 0]),
            (PLACEHOLDER, [148, 148, 156, 255]),
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
            reaper: PageReaper::new(),
            last_destroy: Instant::now(),
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

    /// Whether new texels of this size for `slot` would need a new page
    /// (none of that size has a free layer, and the slot is not already of
    /// that size). `levels`: length of the mip chain, as uploaded.
    pub fn needs_page(&self, slot: u32, width: u32, height: u32, levels: u32) -> bool {
        self.store.needs_page(slot, width, height, levels)
    }

    /// CPU time spent creating pages since the start (ms; AURORA_PROFILE).
    pub fn page_create_ms(&self) -> f32 {
        self.store.create_ms
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
                "page {} layer {} ({}×{}, {} levels{})",
                s.page,
                s.layer,
                s.key.width,
                s.key.height,
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

    /// Once a frame, when it is handed to the render thread: compact
    /// (throttled), record the upload of the changed locations, and rebuild
    /// the bind group when pages changed (at once for a new page, throttled
    /// for dropped ones).
    pub fn maintain(&mut self, device: &wgpu::Device, writes: &mut GpuWrites, uploads: &mut UploadQueue, force: bool) {
        if self.last_compact.elapsed() >= COMPACT_INTERVAL {
            self.last_compact = Instant::now();
            self.store.compact(device, uploads);
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
                self.reaper.destroy(page);
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
pub fn build_mips(width: u32, height: u32, data: Vec<u8>, max_levels: u32) -> Vec<(u32, u32, Vec<u8>)> {
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
                // alpha-weighted color average to avoid dark fringes
                let wa = prev[a + 3] as u32;
                let wb = prev[b + 3] as u32;
                let wc = prev[c + 3] as u32;
                let wd = prev[d + 3] as u32;
                let wsum = wa + wb + wc + wd;
                let o = ((y * nw + x) * 4) as usize;
                for ch in 0..3 {
                    let weighted = prev[a + ch] as u32 * wa
                        + prev[b + ch] as u32 * wb
                        + prev[c + ch] as u32 * wc
                        + prev[d + ch] as u32 * wd
                        + wsum / 2;
                    let v = weighted
                        .checked_div(wsum)
                        .unwrap_or((prev[a + ch] as u32 + prev[b + ch] as u32 + prev[c + ch] as u32 + prev[d + ch] as u32 + 2) / 4);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writes::QueueSink;
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
                mips: 7
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
        let key = |width, height, mips| PageKey { width, height, mips };
        assert_eq!(key(4, 2, 3).layer_bytes(), (8 + 2 + 1) * 4);
        assert_eq!(key(1, 1, 1).layer_bytes(), 4);
        assert_eq!(key(64, 64, 1).layer_bytes(), 64 * 64 * 4);
    }

    #[test]
    fn locations_pack_page_and_layer() {
        for (page, layer) in [(0, 0), (1, 0), (0, 2047), (16383, 2047), (65535, 65535)] {
            assert_eq!(unpack(pack(page, layer)), (page, layer));
        }
        assert_eq!(pack(3, 5), (3 << 16) | 5);
        assert_eq!(pack(0, 0), WHITE_LOC);
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
    fn pages_grow_from_small_to_large() {
        // 1024² with mips (5.3 MB a layer): single layers, then up to 32 MB
        let big = PageKey {
            width: 1024,
            height: 1024,
            mips: 11,
        }
        .layer_bytes();
        assert_eq!(growth(big, 12), vec![1, 1, 1, 1, 2, 3, 4, 6, 6, 6, 6, 6]);
        // 64² with mips (21 KB a layer): 2 MB pages, then half the total
        let small = PageKey {
            width: 64,
            height: 64,
            mips: 7,
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
        assert_eq!(compaction_pick(&[(1, 10, 16), (2, 4, 16)], kb), Some((2, 4)));
        // the emptiest candidate first
        assert_eq!(compaction_pick(&[(1, 2, 16), (2, 3, 16), (3, 10, 64)], kb), Some((1, 2)));
        // more than a quarter used
        assert_eq!(compaction_pick(&[(1, 10, 16), (2, 5, 16)], kb), None);
        // no room in the other pages
        assert_eq!(compaction_pick(&[(1, 15, 16), (2, 2, 16)], kb), None);
        // page 0 (reserved slots) never moves
        assert_eq!(compaction_pick(&[(0, 1, 2048), (1, 10, 16)], kb), None);
        // too much to copy at once (4 × 8 MB)
        assert_eq!(compaction_pick(&[(1, 40, 64), (2, 4, 16)], 8 << 20), None);
        assert_eq!(compaction_pick(&[(1, 1, 64), (2, 4, 16)], 8 << 20), Some((1, 1)));
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
        let rows: u32 = (0..k.mips).map(|l| level_dim(k.height, l)).sum();
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: rows as u64 * 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut offset = 0u64;
        for l in 0..k.mips {
            let (w, h) = (level_dim(k.width, l), level_dim(k.height, l));
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
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            offset += h as u64 * 256;
        }
        queue.submit(std::iter::once(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().expect("mapped read-back").to_vec();
        let mut out = Vec::new();
        let mut row = 0usize;
        for l in 0..k.mips {
            let (w, h) = (level_dim(k.width, l) as usize, level_dim(k.height, l) as usize);
            let mut level = Vec::new();
            for y in 0..h {
                let start = (row + y) * 256;
                level.extend_from_slice(&data[start..start + w * 4]);
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

    /// GPU check of the shared placeholder: slots of textures still loading
    /// show the one grey layer without owning it; their first texels (even
    /// 1×1 ones, the placeholder's own size) go to a layer of their own and
    /// leave it untouched, and freeing them does not release it.
    /// `cargo test -p aurora-render placeholders -- --ignored`.
    #[test]
    #[ignore]
    fn placeholders_share_one_layer_on_the_gpu() {
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
        assert!(store.needs_page(waiting[0], 16, 16, 5));
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
        assert!(store.needs_page(waiting[1], 1, 1, 1) || store.page(0).is_some_and(|p| p.layers.used() < p.layers.len));
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

    /// The whole table on the GPU as the renderer uses it: random creations,
    /// upgrades, frees and compactions between "frames" (`maintain`), then
    /// every live slot is sampled through the bind group and the location
    /// buffer (one pixel per slot) and must give its texture's first texel.
    /// `cargo test -p aurora-render pages_sampled -- --ignored`.
    #[test]
    #[ignore]
    fn pages_sampled_through_the_bind_group() {
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
        let mut table = TextureTable::new(&device, &mut writes.borrow_mut(), SLOTS, 8);
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
