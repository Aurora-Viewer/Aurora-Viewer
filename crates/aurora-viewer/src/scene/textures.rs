//! Texture streaming: progressive JPEG2000 fetch by byte ranges, disk cache,
//! parallel decode at the discard level matching on-screen size, bindless
//! slot management and GPU upload budgeting.
//!
//! Uploads keep the frame smooth (Firestorm creates its GL textures within
//! 2–5 ms a frame, LLViewerTextureList::updateImagesCreateTextures; here the
//! budget is ~1 ms): the decode job also writes the mip chain into mapped
//! staging memory (aurora_render::upload), so the main thread only records
//! the GPU copies, the most visible textures first (first texels before
//! upgrades, then by on-screen size), within a time and a byte budget.
//!
//! Nothing here walks every texture at intervals: unused textures wait in
//! a queue in the order they were released and are evicted a few per
//! frame, downloaded data to persist is listed when it arrives, and what
//! the main thread no longer needs (decoded levels, the J2C data of
//! evicted textures) is freed by the trash thread (`Jobs::discard`).

use super::jobs::{AlphaKind, JobResult, Jobs, SculptMap, Trash, classify_alpha, to_rgba};
use super::texture_budget::{self, Pressure};
use aurora_assets::J2kInfo;
use aurora_net::fetch::ByteRange;
use aurora_net::{FetchRequest, FetchResult, Fetcher};
use aurora_render::{MipLevel, Renderer, StagedTexture, StagingPool, build_mips};
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_TEXTURE: u64 = 1 << 60;

/// ViewerAsset URL as built by LL: `cap + "/?<kind>=<uuid>"`.
pub fn asset_url(base: &str, kind: &str, id: &Uuid) -> String {
    if base.contains('?') {
        format!("{base}&{kind}={id}")
    } else if base.ends_with('/') {
        format!("{base}?{kind}={id}")
    } else {
        format!("{base}/?{kind}={id}")
    }
}
const INITIAL_BYTES: usize = 16 * 1024;
const UNUSED_EVICT: Duration = Duration::from_secs(45);
/// Unused textures evicted per frame at most, and queue entries looked at
/// (those used again since are only skipped): the textures of a region
/// left by teleport go in about a second instead of in one frame.
const EVICT_PER_FRAME: usize = 64;
const EVICT_LOOKS_PER_FRAME: usize = 512;
/// Uploads set aside in a frame because their texture page is still being
/// made, before the frame stops looking for one that can go.
const PAGE_WAITS: usize = 16;
/// Downloaded data is written to the disk cache this often.
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(2);
/// The memory budget is checked this often (a query to the driver).
const BUDGET_EVERY: Duration = Duration::from_millis(250);
/// Textures lowered a second under memory pressure, and per frame at least
/// and at most: GPU copies of their coarser levels, small (Firestorm scales
/// down within its 2 to 5 ms of texture work a frame, at least five
/// textures). By the second, not by the frame: a video memory that
/// overflows makes slow frames, and giving it back must not slow down with
/// them.
const REDUCTIONS_PER_SECOND: f32 = 3000.0;
const REDUCTIONS_PER_FRAME: std::ops::RangeInclusive<usize> = 8..=512;
/// Firestorm asks for MAX_IMAGE_DATA_SIZE (indra/llimage/llimage.h) when it
/// wants the whole file.
const MAX_IMAGE_DATA_SIZE: u64 = 4096 * 4096 * 8;
/// Past this end, LLTextureFetchWorker sends an open range (`bytes=a-`).
const HTTP_REQUESTS_RANGE_END_MAX: u64 = 20_000_000;

/// Byte range of the next request when we hold `have` bytes and want
/// `needed` (port of LLTextureFetchWorker::doWork, SEND_HTTP_REQ, in
/// indra/newview/lltexturefetch.cpp, originally LGPL 2.1): a resumed
/// request starts one byte early so that it is always partially
/// satisfiable (some caches answer an unsatisfiable range with a 200 and
/// the whole asset), and a range ending past HTTP_REQUESTS_RANGE_END_MAX
/// is left open.
fn next_range(have: usize, needed: usize) -> ByteRange {
    let have = have as u64;
    let desired = (needed as u64).min(MAX_IMAGE_DATA_SIZE).max(have + 1);
    let (mut start, mut size) = (have, desired - have);
    if start > 0 {
        start -= 1;
        size += 1;
    }
    let end = (start + size <= HTTP_REQUESTS_RANGE_END_MAX).then(|| start + size - 1);
    ByteRange { start, end }
}

/// Append a ranged reply starting at `offset` to `data`; it may overlap the
/// end of what we have (the byte asked again). False, leaving `data`
/// untouched, when the reply would leave a gap or ends before our data
/// (Firestorm aborts that load).
fn merge_reply(data: &mut Vec<u8>, offset: u64, body: &[u8]) -> bool {
    let have = data.len() as u64;
    if offset > have || have > offset + body.len() as u64 {
        return false;
    }
    let skip = (have - offset) as usize;
    data.extend_from_slice(&body[skip..]);
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TexSource {
    Asset,
    /// Server-side bake fetched from the appearance service.
    Bake {
        url: String,
    },
}

/// Picking follows LLImageGL::updatePickMask / getMask: one bit per 2×2
/// block, alpha > 32 at its first texel, repeated UVs. Top-left decoded rows
/// are indexed from the bottom, like world UVs. No OpenGL state or
/// readback is involved; masks are built by decode workers and retired off-thread.
pub struct PickMask {
    width: u32,
    height: u32,
    bits: Vec<u8>,
}

impl PickMask {
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let pixels = (width as usize).checked_mul(height as usize)?;
        if width == 0 || height == 0 || rgba.len() != pixels.checked_mul(4)? {
            return None;
        }
        let (w, h) = (width.div_ceil(2), height.div_ceil(2));
        let mut bits = vec![0; (w as usize * h as usize).div_ceil(8)];
        for y in 0..h {
            for x in 0..w {
                let row = height - 1 - y * 2;
                if rgba[((row * width + x * 2) as usize) * 4 + 3] > 32 {
                    let bit = (y * w + x) as usize;
                    bits[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        Some(Self { width: w, height: h, bits })
    }

    pub fn hits(&self, uv: glam::Vec2) -> bool {
        let uv = if uv.is_finite() { uv } else { glam::Vec2::ZERO };
        let x = (uv.x.rem_euclid(1.0) * self.width as f32) as usize;
        let y = (uv.y.rem_euclid(1.0) * self.height as f32) as usize;
        let bit = y.min(self.height as usize - 1) * self.width as usize + x.min(self.width as usize - 1);
        self.bits[bit / 8] & (1 << (bit % 8)) != 0
    }
}

struct Entry {
    slot: u32,
    refs: u32,
    source: TexSource,
    /// The J2C bytes so far; shared with decode jobs and cache writes
    /// (cloned only if a job still holds it when more bytes arrive).
    data: Arc<Vec<u8>>,
    complete: bool,
    info: Option<J2kInfo>,
    decoded: Option<u8>,
    want: u8,
    /// Pixels it covers on screen (largest face in view), and those it
    /// would cover from where the camera is, in view or not; `fixed_px`:
    /// noted by what memory pressure never lowers (HUDs, sky, terrain).
    /// All three decay, see `recompute_wants`.
    need_px: f32,
    away_px: f32,
    fixed_px: f32,
    /// Size of its level 0 on the GPU (0 until the first upload).
    gpu_size: (u32, u32),
    fetching: bool,
    decoding: bool,
    cache_state: u8, // 0 unchecked, 1 loading, 2 done
    failures: u32,
    retry_at: Option<Instant>,
    keep_pixels: bool,
    sculpt: Option<Arc<SculptMap>>,
    alpha: AlphaKind,
    unused_since: Option<Instant>,
    fetch_key: u64,
    dirty_cache: bool,
    /// Why the last fetch failed (status, error kind, host; never the URL).
    last_error: Option<String>,
    /// The failure was already logged at info level.
    error_logged: bool,
    /// The last fetch was answered 404: asked again much later.
    gone: bool,
}

struct PendingUpload {
    id: Uuid,
    discard: u8,
    /// Staged by the decode job: only the copies are left to record.
    staged: Option<StagedTexture>,
    /// The levels in memory: the whole chain when not staged (written by
    /// the main thread), else only those up to 256 px (UI copies).
    mips: Vec<(u32, u32, Vec<u8>)>,
    alpha: AlphaKind,
    alpha_channel: bool,
    pick_mask: Option<PickMask>,
    priority: UploadPriority,
    /// Tests: as if staged in a chunk a job still writes in.
    #[cfg(test)]
    held: bool,
}

/// Upload order: textures still showing their placeholder first, then the
/// largest on screen (Firestorm's decode priority is the on-screen pixel
/// area), then the oldest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UploadPriority {
    pub first: bool,
    pub pixels: f32,
    pub seq: u64,
}

impl Eq for UploadPriority {}

impl Ord for UploadPriority {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.first
            .cmp(&o.first)
            .then(self.pixels.total_cmp(&o.pixels))
            .then(o.seq.cmp(&self.seq))
    }
}

impl PartialOrd for UploadPriority {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for PendingUpload {
    fn eq(&self, o: &Self) -> bool {
        self.priority == o.priority
    }
}

impl Eq for PendingUpload {}

impl Ord for PendingUpload {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.priority.cmp(&o.priority)
    }
}

impl PartialOrd for PendingUpload {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// What one frame may spend on uploads: main-thread time, and bytes copied
/// by the GPU (a copy is ~1 ms of GPU per 24 MB).
#[derive(Clone, Copy, Debug)]
pub struct UploadBudget {
    pub time: Duration,
    pub bytes: u64,
}

/// Main-thread time a frame of `frame` may give to a streaming step: 5 % of
/// the frame (the share Firestorm gives its texture updates, 50 ms a second
/// in llviewerdisplay.cpp), at least `floor` and at most 3 ms. A fast frame
/// keeps the floor; at 15 or 30 frames a second (window in the background,
/// modest machine) the same work is not spread over ten times more time.
pub fn frame_share(frame: Duration, floor: Duration) -> Duration {
    (frame / 20).clamp(floor, Duration::from_millis(3).max(floor))
}

impl UploadBudget {
    /// The budget of a frame that lasts `frame`.
    pub fn for_frame(frame: Duration) -> Self {
        UploadBudget {
            time: frame_share(frame, Duration::from_micros(800)),
            bytes: 24 << 20,
        }
    }
}

impl UploadBudget {
    /// Whether another upload may start after `done` uploads of `bytes`
    /// in `elapsed`: always the first of the frame (a texture larger than
    /// the budget still goes), then while both budgets hold.
    pub fn allows(&self, done: usize, bytes: u64, elapsed: Duration) -> bool {
        done == 0 || (bytes < self.bytes && elapsed < self.time)
    }
}

impl PendingUpload {
    /// Its staged data can be copied (or it has none: a main-thread write).
    fn ready(&self) -> bool {
        #[cfg(test)]
        if self.held {
            return false;
        }
        self.staged.as_ref().is_none_or(|s| s.data.ready())
    }

    /// Level-0 size and number of levels of the chain to upload (what
    /// decides its texture page).
    fn chain(&self) -> Option<(u32, u32, u32)> {
        match &self.staged {
            Some(staged) => staged.levels.first().map(|l| (l.width, l.height, staged.levels.len() as u32)),
            None => self.mips.first().map(|m| (m.0, m.1, self.mips.len() as u32)),
        }
    }

    /// Done with this upload. Its staging region is released here (the
    /// chunk is recycled once its last region is gone, which must not wait
    /// for another thread); its levels in memory are left to the trash
    /// thread: freeing them here took 1–3 ms a frame while the decode jobs
    /// allocate on every core, unseen by the time budget.
    fn retire(self, trash: &mut Trash<Vec<u8>>) {
        if let Some(mask) = self.pick_mask {
            trash.push(mask.bits);
        }
        for (_, _, pixels) in self.mips {
            if !pixels.is_empty() {
                trash.push(pixels);
            }
        }
    }
}

/// Decoded textures waiting for the GPU, most urgent first.
#[derive(Default)]
struct PendingUploads {
    heap: BinaryHeap<PendingUpload>,
    /// Staged in a chunk a job still writes in: not copyable yet. Kept out
    /// of the heap, so a frame does not pop and push back each of them on
    /// its way to the uploads that can go.
    unready: Vec<PendingUpload>,
    /// Finest discard level waiting, per texture: a coarser one still in
    /// the queue is dropped when its turn comes (the finer one replaces it).
    finest: HashMap<Uuid, u8>,
    /// Uploads waiting, per chain size: tells the renderer when to prepare
    /// the next texture page of a size (`TextureTable::prepare_page`).
    chains: HashMap<(u32, u32, u32), u32>,
    seq: u64,
}

impl PendingUploads {
    fn len(&self) -> usize {
        self.heap.len() + self.unready.len()
    }

    /// Queue an upload; `first`: the texture still shows its placeholder,
    /// `pixels`: its size on screen.
    fn push(&mut self, mut u: PendingUpload, first: bool, pixels: f32) {
        self.seq += 1;
        u.priority = UploadPriority {
            first,
            pixels,
            seq: self.seq,
        };
        self.requeue(u);
    }

    /// Put back an upload that cannot go yet, at its place.
    fn requeue(&mut self, u: PendingUpload) {
        let finest = self.finest.entry(u.id).or_insert(u.discard);
        *finest = (*finest).min(u.discard);
        if let Some(chain) = u.chain() {
            *self.chains.entry(chain).or_default() += 1;
        }
        if u.ready() {
            self.heap.push(u);
        } else {
            self.unready.push(u);
        }
    }

    /// Once a frame: the uploads whose staging chunk became copyable join
    /// the others (one flag read per waiting upload).
    fn promote_ready(&mut self) {
        let mut i = 0;
        while i < self.unready.len() {
            if self.unready[i].ready() {
                let u = self.unready.swap_remove(i);
                self.heap.push(u);
            } else {
                i += 1;
            }
        }
    }

    /// Uploads of that chain size waiting (copyable or not).
    fn queued(&self, chain: (u32, u32, u32)) -> u32 {
        self.chains.get(&chain).copied().unwrap_or(0)
    }

    /// The most urgent copyable upload that is still the finest of its
    /// texture; the coarser ones met on the way are retired.
    fn pop(&mut self, trash: &mut Trash<Vec<u8>>) -> Option<PendingUpload> {
        while let Some(u) = self.heap.pop() {
            if let Some(chain) = u.chain()
                && let std::collections::hash_map::Entry::Occupied(mut n) = self.chains.entry(chain)
            {
                *n.get_mut() -= 1;
                if *n.get() == 0 {
                    n.remove();
                }
            }
            match self.finest.get(&u.id) {
                Some(&f) if f < u.discard => {
                    u.retire(trash);
                    continue;
                }
                Some(_) => {
                    self.finest.remove(&u.id);
                }
                None => {}
            }
            return Some(u);
        }
        None
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TextureStats {
    pub total: usize,
    pub loaded: usize,
    pub fetching: usize,
    pub decoding: usize,
    pub pending_upload: usize,
    pub uploaded_bytes_frame: u64,
    pub uploads_frame: usize,
    /// Uploads since the start: staged by the decode job (the main thread
    /// recorded copies) and direct (written by the main thread).
    pub uploads_staged: u64,
    pub uploads_direct: u64,
    /// In use, never loaded, and the last fetch failed.
    pub failing: usize,
    /// Memory budget (texture_budget.rs): bytes the pages may hold now, the
    /// pressure step, and textures lowered on the GPU since the start.
    pub target_bytes: u64,
    pub pressure: u8,
    pub reduced: u64,
}

/// Demo textures without a file to decode (AURORA_DEMO_STREAM under a
/// forced memory budget): `texels` draws the mip chain of one of them
/// (`has`) at a discard level, on the background pool, when the streamer
/// wants it finer than it is, as a decode of downloaded data would. Without
/// it a demo texture lowered by the memory budget could never load again.
#[derive(Clone, Copy)]
pub struct DemoSource {
    pub has: fn(&Uuid) -> bool,
    pub texels: fn(&Uuid, u8) -> Vec<(u32, u32, Vec<u8>)>,
}

/// Discard level of a texture of `full` pixels (its largest side) to cover
/// `px` pixels on screen, among the `levels` its file holds.
fn discard_for(full: f32, px: f32, levels: Option<u8>) -> u8 {
    let d = (full / px.max(8.0)).log2().floor().clamp(0.0, 5.0) as u8;
    levels.map_or(d, |l| d.min(l.min(5)))
}

/// The discard level a texture is loaded to, and the one it is lowered to
/// under memory pressure when it holds more, the sizes on screen divided
/// by what the pressure step takes (`texture_budget::dropped`: in view,
/// out of view). Loading follows what is in view; lowering stops at what
/// the texture would need from where the camera is, half a level finer, so
/// the two never chase each other (`keep <= want`).
fn wanted(full: f32, levels: Option<u8>, e: &Entry, (in_view, away): (f32, f32)) -> (u8, u8) {
    let seen = e.fixed_px.max(e.need_px / in_view);
    let near = seen.max(e.away_px / away);
    (
        discard_for(full, seen, levels),
        discard_for(full, near * std::f32::consts::SQRT_2, levels),
    )
}

/// Size of a texture at full resolution: that of its file, else from what
/// it holds on the GPU (demo textures have no file), else unknown.
fn full_size(e: &Entry) -> Option<(u32, u32)> {
    match (e.info, e.decoded) {
        (Some(i), _) => Some((i.width, i.height)),
        (None, Some(d)) if e.gpu_size.0 > 0 => Some((e.gpu_size.0 << d, e.gpu_size.1 << d)),
        _ => None,
    }
}

/// Bytes of a texture's mip chain at a discard level, about (a third more
/// than its level 0); 0 while its size is not known.
fn bytes_at(e: &Entry, discard: u8) -> u64 {
    full_size(e).map_or(0, |(w, h)| (w as u64 * h as u64 * 16 / 3) >> (2 * discard))
}

/// Alpha class of a bindless slot; a change bumps `generation` and is
/// listed in `changes` (`TextureStreamer::take_alpha_changes`).
fn set_slot_alpha(alpha: &mut Vec<AlphaKind>, generation: &mut u64, changes: &mut Vec<u32>, slot: u32, a: AlphaKind) {
    let i = slot as usize;
    if i >= alpha.len() {
        alpha.resize(i + 1024, AlphaKind::Opaque);
    }
    if alpha[i] != a {
        alpha[i] = a;
        *generation += 1;
        changes.push(slot);
    }
}

pub struct TextureStreamer {
    entries: HashMap<Uuid, Entry>,
    by_fetch_key: HashMap<u64, Uuid>,
    next_key: u64,
    uploads: PendingUploads,
    /// Staging memory of the renderer, given to the decode jobs.
    pub staging: Option<Arc<StagingPool>>,
    /// Alpha class by bindless slot (fast lookup during list building).
    pub alpha_by_slot: Vec<AlphaKind>,
    pub(super) pick_masks: HashMap<Uuid, PickMask>,
    /// Per slot: the decoded image has an alpha channel (LL sends such
    /// faces to the alpha pool when rigged or glowing, even if opaque).
    pub alpha_channel_by_slot: Vec<bool>,
    cache_dir: PathBuf,
    /// Procedural demo textures: id -> slot.
    local_slots: HashMap<Uuid, u32>,
    /// Bumped whenever a slot's alpha class or alpha channel, or a sculpt
    /// map, changes.
    pub generation: u64,
    /// Slots whose alpha class or alpha channel changed since the scene
    /// last took them: it classifies the faces using them again (their
    /// pass is cached, and the GPU lists read it).
    alpha_changes: Vec<u32>,
    max_decodes: usize,
    decodes: usize,
    /// Memory pressure step (texture_budget.rs), decided in `maintain`.
    pressure: Pressure,
    last_budget: Instant,
    /// Passes of `recompute_wants` since the step last changed.
    passes_at_step: u32,
    /// Loaded textures to lower to a discard level, listed by the last
    /// pass under pressure and done a few per frame, the largest (the
    /// bytes they hold, last of the tuple) first: taken from the end.
    reductions: Vec<(Uuid, u8, u64)>,
    /// Bytes the textures would hold one pressure step below, from the
    /// last pass.
    bytes_below: u64,
    /// Textures the demo draws itself, see [`DemoSource`].
    pub demo_source: Option<DemoSource>,
    /// Textures also wanted as egui images (profile pictures...): small CPU copies.
    ui_wanted: std::collections::HashSet<Uuid>,
    ui_ready: HashMap<Uuid, (u32, u32, Vec<u8>)>,
    pub stats: TextureStats,
    last_maintenance: Instant,
    /// Unused textures are evicted after this long (shortened by the
    /// AURORA_DEMO_TEXTURES_CHURN test).
    pub evict_after: Duration,
    /// Textures whose last reference went away, oldest first, with the
    /// time it did (an entry used again since has another `unused_since`).
    unused: VecDeque<(Instant, Uuid)>,
    /// Textures with downloaded data not written to the disk cache yet
    /// (`Entry::dirty_cache`), each once.
    dirty: Vec<Uuid>,
    /// Decoded levels the uploads are done with and download bodies once
    /// merged, freed off this thread.
    trash: Trash<Vec<u8>>,
}

impl TextureStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("tex"));
        TextureStreamer {
            // (room for a busy region, ~14,000 textures at BackBone: a
            // table of that size growing moves every entry in one frame)
            entries: HashMap::with_capacity(28_000),
            by_fetch_key: HashMap::new(),
            next_key: 1,
            uploads: PendingUploads::default(),
            staging: None,
            pick_masks: HashMap::from([(
                aurora_prim::te::TRANSPARENT_TEXTURE,
                PickMask {
                    width: 1,
                    height: 1,
                    bits: vec![0],
                },
            )]),
            // built-in slots: the transparent texture has no visible pixel
            alpha_by_slot: {
                let mut a = vec![AlphaKind::Opaque; 16];
                a[aurora_render::textures::TRANSPARENT as usize] = AlphaKind::Mask;
                a
            },
            alpha_channel_by_slot: Vec::new(),
            cache_dir,
            local_slots: HashMap::new(),
            generation: 0,
            max_decodes: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(2),
            decodes: 0,
            pressure: Pressure::new(Instant::now()),
            last_budget: Instant::now(),
            passes_at_step: 0,
            reductions: Vec::new(),
            bytes_below: 0,
            demo_source: None,
            ui_wanted: Default::default(),
            ui_ready: HashMap::new(),
            stats: TextureStats::default(),
            last_maintenance: Instant::now(),
            evict_after: UNUSED_EVICT,
            unused: VecDeque::new(),
            dirty: Vec::new(),
            trash: Trash::default(),
            alpha_changes: Vec::new(),
        }
    }

    /// The slots whose alpha class or alpha channel changed since the last
    /// call (a slot may be listed more than once).
    pub fn take_alpha_changes(&mut self, out: &mut Vec<u32>) {
        out.append(&mut self.alpha_changes);
    }

    fn cache_path(cache_dir: &std::path::Path, id: &Uuid, complete: bool) -> PathBuf {
        cache_dir
            .join("tex")
            .join(format!("{id}.{}", if complete { "j2c" } else { "part" }))
    }

    /// Reference a texture; returns its bindless slot (placeholder until loaded).
    /// Diagnostic: loading state of a texture.
    /// Full resolution of a texture once its header is known.
    pub fn full_size(&self, id: &Uuid) -> Option<(u32, u32)> {
        self.entries.get(id)?.info.map(|i| (i.width, i.height))
    }

    /// The texture has see-through pixels (None until it is decoded): the
    /// build floater's « Mode alpha » is only offered then.
    pub fn has_alpha(&self, id: &Uuid) -> Option<bool> {
        if !self.is_loaded(id) {
            return None;
        }
        Some(self.entries.get(id)?.alpha != AlphaKind::Opaque)
    }

    pub fn describe(&self, id: &Uuid) -> String {
        match self.entries.get(id) {
            None => "not requested".into(),
            Some(e) => format!(
                "{} bytes{}, decoded {:?}, want {}, alpha {:?}, failures {}{}{}{}{}{}",
                e.data.len(),
                if e.complete { " (complete)" } else { "" },
                e.decoded,
                e.want,
                e.alpha,
                e.failures,
                if e.fetching { ", fetching" } else { "" },
                if e.decoding { ", decoding" } else { "" },
                e.retry_at
                    .and_then(|t| t.checked_duration_since(Instant::now()))
                    .map(|d| format!(", retry in {} s", d.as_secs()))
                    .unwrap_or_default(),
                match &e.source {
                    TexSource::Bake { .. } => ", bake service",
                    TexSource::Asset => "",
                },
                e.last_error
                    .as_deref()
                    .map(|err| format!(", last error: {err}"))
                    .unwrap_or_default()
            ),
        }
    }

    /// Diagnostic: up to `max` textures in use that are still not decoded.
    pub fn stuck(&self, max: usize) -> Vec<(Uuid, String)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.refs > 0 && e.decoded.is_none())
            .take(max)
            .map(|(id, _)| (*id, self.describe(id)))
            .collect()
    }

    /// Diagnostic: why a face drawn with `slot` for texture `id` would not
    /// show that texture (None when it should).
    pub fn face_problem(&self, id: &Uuid, slot: u32, table: &aurora_render::textures::TextureTable) -> Option<&'static str> {
        if id.is_nil()
            || *id == aurora_prim::te::BLANK_TEXTURE
            || *id == aurora_prim::te::TRANSPARENT_TEXTURE
            || self.local_slots.contains_key(id)
        {
            return None;
        }
        let Some(e) = self.entries.get(id) else {
            return Some("texture not requested");
        };
        if e.slot != slot {
            return Some("drawn with another slot than the streamer's (stale record)");
        }
        let Some(discard) = e.decoded else {
            return Some("not loaded yet");
        };
        let expected = e.info.map(|i| ((i.width >> discard).max(1), (i.height >> discard).max(1)));
        match table.size_of(slot) {
            None => Some("slot free in the renderer"),
            Some((1, 1)) if expected.is_some_and(|s| s != (1, 1)) => Some("renderer still holds the 1×1 placeholder"),
            _ => None,
        }
    }

    pub fn acquire(&mut self, renderer: &mut Renderer, id: Uuid, source: TexSource) -> u32 {
        if id.is_nil() {
            return aurora_render::textures::WHITE;
        }
        if id == aurora_prim::te::TRANSPARENT_TEXTURE {
            return aurora_render::textures::TRANSPARENT;
        }
        if id == aurora_prim::te::BLANK_TEXTURE {
            return aurora_render::textures::WHITE;
        }
        if let Some(e) = self.entries.get_mut(&id) {
            e.refs += 1;
            e.unused_since = None;
            if e.source == TexSource::Asset && source != TexSource::Asset {
                e.source = source;
            }
            return e.slot;
        }
        // procedural textures of the offline demo (never streamed)
        if let Some(slot) = self.local_slots.get(&id) {
            return *slot;
        }
        if let Some((px, w, h)) = crate::demo::local_texture(&id)
            && let Some(slot) = renderer.create_texture(&[MipLevel {
                width: w,
                height: h,
                data: &px,
            }])
        {
            set_slot_alpha(
                &mut self.alpha_by_slot,
                &mut self.generation,
                &mut self.alpha_changes,
                slot,
                super::jobs::classify_alpha(&px),
            );
            self.local_slots.insert(id, slot);
            return slot;
        }
        // the shared grey placeholder until the first texels arrive: no
        // GPU work for a texture met for the first time
        let slot = renderer.textures.create_placeholder().unwrap_or(aurora_render::textures::WHITE);
        set_slot_alpha(
            &mut self.alpha_by_slot,
            &mut self.generation,
            &mut self.alpha_changes,
            slot,
            AlphaKind::Opaque,
        );
        self.entries.insert(
            id,
            Entry {
                slot,
                refs: 1,
                source,
                data: Arc::default(),
                complete: false,
                info: None,
                decoded: None,
                want: 2,
                need_px: 0.0,
                away_px: 0.0,
                fixed_px: 0.0,
                gpu_size: (0, 0),
                fetching: false,
                decoding: false,
                cache_state: 0,
                failures: 0,
                retry_at: None,
                keep_pixels: false,
                sculpt: None,
                alpha: AlphaKind::Opaque,
                unused_since: None,
                fetch_key: 0,
                dirty_cache: false,
                last_error: None,
                error_logged: false,
                gone: false,
            },
        );
        slot
    }

    /// A texture drawn by the viewer itself (media): `acquire` returns its slot.
    pub fn register_local(&mut self, id: Uuid, slot: u32) {
        set_slot_alpha(
            &mut self.alpha_by_slot,
            &mut self.generation,
            &mut self.alpha_changes,
            slot,
            AlphaKind::Opaque,
        );
        if let Some(c) = self.alpha_channel_by_slot.get_mut(slot as usize)
            && *c
        {
            *c = false;
            self.generation += 1;
            self.alpha_changes.push(slot);
        }
        self.local_slots.insert(id, slot);
    }

    pub fn unregister_local(&mut self, id: &Uuid) {
        self.local_slots.remove(id);
    }

    pub fn release(&mut self, id: &Uuid) {
        if let Some(e) = self.entries.get_mut(id) {
            let held = e.refs > 0;
            e.refs = e.refs.saturating_sub(1);
            if e.refs == 0 && (held || e.unused_since.is_none()) {
                // queued for eviction, in release order
                let now = Instant::now();
                e.unused_since = Some(now);
                self.unused.push_back((now, *id));
            }
        }
    }

    /// Mark a texture as a sculpt map (pixels are kept for geometry).
    pub fn want_sculpt(&mut self, id: &Uuid) {
        if let Some(e) = self.entries.get_mut(id) {
            e.keep_pixels = true;
            e.want = 0;
            e.need_px = e.need_px.max(4096.0);
        }
    }

    pub fn sculpt_map(&self, id: &Uuid) -> Option<Arc<SculptMap>> {
        self.entries.get(id).and_then(|e| e.sculpt.clone())
    }

    /// Keep a texture resident and deliver a small RGBA copy for the UI.
    pub fn want_ui_image(&mut self, renderer: &mut Renderer, id: Uuid) {
        if id.is_nil() {
            return;
        }
        if self.ui_wanted.insert(id) {
            // procedural textures of the offline demo are never streamed
            if let Some((px, w, h)) = crate::demo::local_texture(&id) {
                self.ui_ready.insert(id, (w, h, px));
                return;
            }
            let _ = self.acquire(renderer, id, TexSource::Asset);
        }
        self.note_usage(&id, 256.0);
    }

    /// Decoded UI copy, once available (taken by the caller).
    pub fn take_ui_image(&mut self, id: &Uuid) -> Option<(u32, u32, Vec<u8>)> {
        self.ui_ready.remove(id)
    }

    /// Something uses this texture (it is streamed). An entry nothing
    /// references any more only waits for its eviction: it is not fetched.
    pub fn is_wanted(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.refs > 0)
    }

    /// Never loaded and not coming for now: missing on the asset server
    /// (asked again in ten minutes), or failing again and again.
    pub fn is_unavailable(&self, id: &Uuid) -> bool {
        self.entries
            .get(id)
            .is_some_and(|e| e.decoded.is_none() && (e.gone || e.failures > 3))
    }

    /// Discard level on the GPU (None until the first upload).
    pub fn decoded_level(&self, id: &Uuid) -> Option<u8> {
        self.entries.get(id)?.decoded
    }

    pub fn is_loaded(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.decoded.is_some())
    }

    /// Record that a texture covers about `px` pixels on screen.
    #[inline]
    pub fn note_usage(&mut self, id: &Uuid, px: f32) {
        if let Some(e) = self.entries.get_mut(id)
            && px > e.need_px
        {
            e.need_px = px;
        }
    }

    /// Record that a texture would cover about `px` pixels from where the
    /// camera is, on an object out of view: not loaded for it, but under
    /// memory pressure it is not lowered further than that (Firestorm's
    /// pixel area does not depend on the view either: turning round does
    /// not show blurred textures).
    #[inline]
    pub fn note_usage_away(&mut self, id: &Uuid, px: f32) {
        if let Some(e) = self.entries.get_mut(id)
            && px > e.away_px
        {
            e.away_px = px;
        }
    }

    /// Like `note_usage`, for what memory pressure never lowers (as
    /// Firestorm's boosted textures: HUDs, sky, terrain).
    #[inline]
    pub fn note_usage_fixed(&mut self, id: &Uuid, px: f32) {
        if let Some(e) = self.entries.get_mut(id)
            && px > e.fixed_px
        {
            e.fixed_px = px;
        }
    }

    /// Recompute desired discard levels from accumulated usage, once every
    /// object was seen. Under memory pressure (texture_budget.rs), also
    /// list the loaded textures to lower, done a few per frame by
    /// `maintain`: those clearly finer than their size on screen (half a
    /// level of margin, so that a texture at the edge of a level is not
    /// lowered then loaded again as the camera sways).
    pub fn recompute_wants(&mut self) {
        let step = self.pressure.step();
        let dropped = texture_budget::dropped(step);
        let below = texture_budget::dropped(step.saturating_sub(1));
        let TextureStreamer {
            entries,
            reductions,
            ui_wanted,
            ..
        } = self;
        reductions.clear();
        let mut bytes_below = 0;
        for (id, e) in entries.iter_mut() {
            if e.keep_pixels {
                e.want = 0;
                bytes_below += bytes_at(e, 0);
                continue;
            }
            let full = full_size(e).map_or(512, |(w, h)| w.max(h)) as f32;
            let levels = e.info.map(|i| i.levels);
            let (want, keep) = wanted(full, levels, e, dropped);
            e.want = want;
            if step > 0 {
                let (want_below, keep_below) = wanted(full, levels, e, below);
                // (there it would be lowered to `keep`, or loaded up to `want`)
                bytes_below += bytes_at(e, e.decoded.map_or(want_below, |d| d.clamp(keep_below, want_below)));
                // (bakes and interface images are left alone, as Firestorm's
                // boosted textures; unused ones are evicted instead)
                if let Some(d) = e.decoded
                    && keep > d
                    && e.refs > 0
                    && !e.decoding
                    && e.source == TexSource::Asset
                    && !ui_wanted.contains(id)
                {
                    reductions.push((*id, keep, bytes_at(e, d)));
                }
            }
            // decay so textures that shrank on screen can be lowered later
            e.need_px *= 0.5;
            e.away_px *= 0.5;
            e.fixed_px *= 0.5;
        }
        // (memory comes back soonest from the largest; the id settles ties,
        // the table has no order of its own)
        reductions.sort_unstable_by_key(|&(id, _, bytes)| (bytes, id));
        self.bytes_below = bytes_below;
        self.passes_at_step = self.passes_at_step.saturating_add(1);
    }

    fn bytes_needed(e: &Entry) -> usize {
        match &e.info {
            // full resolution: the whole file (LLTextureFetch::createRequest),
            // the size estimate would miss the last quality layers
            Some(_) if e.want == 0 => usize::MAX,
            Some(info) => aurora_assets::bytes_for_discard(info, e.want),
            None => INITIAL_BYTES,
        }
    }

    fn url_for(&self, id: &Uuid, e: &Entry, viewer_asset: Option<&str>) -> Option<String> {
        match &e.source {
            TexSource::Bake { url } => Some(url.clone()),
            TexSource::Asset => viewer_asset.map(|base| asset_url(base, "texture_id", id)),
        }
    }

    /// Drive fetches and decodes. Runs every frame over every texture, so the
    /// common case (nothing to do) must stay a few comparisons: no key copy,
    /// no second lookup, cache paths only for the first disk read.
    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        let now = Instant::now();
        let mut fetching = 0;
        let mut loaded = 0;
        let mut failing = 0;
        let TextureStreamer {
            entries,
            by_fetch_key,
            next_key,
            cache_dir,
            decodes,
            max_decodes,
            staging,
            demo_source,
            ..
        } = self;
        // first disk reads of the new textures, queued in one go
        let mut cache_reads = Vec::new();
        for (&id, e) in entries.iter_mut() {
            if e.decoded.is_some() {
                loaded += 1;
            }
            if e.fetching {
                fetching += 1;
            }
            if e.refs == 0 {
                continue;
            }
            if e.decoded.is_none() && e.last_error.is_some() {
                failing += 1;
            }
            if e.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            // 1. disk cache
            if e.cache_state == 0 {
                e.cache_state = 1;
                cache_reads.push(id);
                continue;
            }
            if e.cache_state == 1 || e.decoding || e.fetching {
                continue;
            }
            // (before looking at the data, which is behind a pointer: most
            // textures stop here, every frame)
            let better = match e.decoded {
                None => true,
                Some(d) => e.want < d,
            };
            if !better {
                continue;
            }
            if let Some(source) = demo_source.filter(|s| e.data.is_empty() && (s.has)(&id)) {
                // (its first texels come from the scenario)
                if e.decoded.is_some() && *decodes < *max_decodes {
                    e.decoding = true;
                    *decodes += 1;
                    let (want, pool) = (e.want, staging.clone());
                    jobs.spawn(move || {
                        texture_result(
                            id,
                            want,
                            (source.texels)(&id, want),
                            (AlphaKind::Opaque, false),
                            None,
                            pool.as_deref(),
                        )
                    });
                }
                continue;
            }
            let needed = Self::bytes_needed(e);
            let have_enough = !e.data.is_empty() && (e.complete || e.data.len() >= needed);
            if have_enough || (e.info.is_some() && e.decoded.is_none() && !e.data.is_empty() && e.failures > 0) {
                if *decodes >= *max_decodes {
                    continue;
                }
                // Decode at the best level the data allows.
                let mut discard = e.want;
                if let Some(info) = &e.info
                    && !e.complete
                {
                    // level 0 only from the complete file (a partial one is blurry)
                    while discard < 5 && (discard == 0 || aurora_assets::bytes_for_discard(info, discard) > e.data.len()) {
                        discard += 1;
                    }
                }
                if e.decoded.is_some_and(|d| discard >= d) {
                    continue;
                }
                e.decoding = true;
                *decodes += 1;
                let data = e.data.clone();
                let keep = e.keep_pixels;
                let pool = staging.clone();
                jobs.spawn(move || decode_job(id, &data, discard, keep, pool.as_deref()));
                continue;
            }
            // 2. fetch more
            let Some(url) = (match &e.source {
                TexSource::Bake { url } => Some(url.clone()),
                TexSource::Asset => viewer_asset.map(|base| asset_url(base, "texture_id", &id)),
            }) else {
                continue;
            };
            let range = next_range(e.data.len(), needed);
            let key = FETCH_KIND_TEXTURE | *next_key;
            *next_key += 1;
            e.fetch_key = key;
            e.fetching = true;
            let priority = e.need_px.max(e.fixed_px).max(1.0) + if e.decoded.is_none() { 1e5 } else { 0.0 };
            by_fetch_key.insert(key, id);
            fetcher.request(FetchRequest {
                key,
                url,
                range: Some(range),
                priority,
                accept: "image/x-j2c",
            });
        }
        let dir = cache_dir.clone();
        jobs.spawn_many(cache_reads, move |id| {
            for complete in [true, false] {
                if let Ok(d) = crate::cache::read_touch(Self::cache_path(&dir, &id, complete)) {
                    return JobResult::TextureCache {
                        id,
                        data: Some(d),
                        complete,
                    };
                }
            }
            JobResult::TextureCache {
                id,
                data: None,
                complete: false,
            }
        });
        self.stats.total = self.entries.len();
        self.stats.loaded = loaded;
        self.stats.fetching = fetching;
        self.stats.failing = failing;
        self.stats.decoding = self.decodes;
        self.stats.pending_upload = self.uploads.len();
        let _ = Self::url_for;
    }

    pub fn on_fetch(&mut self, r: FetchResult) {
        let Some(id) = self.by_fetch_key.remove(&r.key) else {
            return;
        };
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        e.fetching = false;
        // LLTextureFetchWorker::callbackHttpGet / WAIT_HTTP_REQ: a 200 is the
        // whole asset, a 206 continues our data, a 416 completes it.
        let outcome = match r.data {
            Ok(bytes) if bytes.is_empty() => {
                if r.complete && !e.data.is_empty() {
                    e.complete = true;
                    Ok(())
                } else {
                    Err(format!("status {} with no data", r.status))
                }
            }
            Ok(bytes) if r.status != 206 => {
                // (what it replaces is freed off this thread, as the body
                // of a merged reply below)
                let old = std::mem::replace(&mut e.data, Arc::new(bytes));
                if let Ok(old) = Arc::try_unwrap(old)
                    && !old.is_empty()
                {
                    self.trash.push(old);
                }
                e.complete = true;
                Ok(())
            }
            Ok(bytes) => {
                let merged = merge_reply(Arc::make_mut(&mut e.data), r.offset, &bytes);
                let len = bytes.len();
                self.trash.push(bytes);
                if merged {
                    e.complete |= r.complete;
                    Ok(())
                } else {
                    Err(format!(
                        "status 206 at byte {} ({len} bytes) does not continue our {} bytes",
                        r.offset,
                        e.data.len()
                    ))
                }
            }
            Err(err) => Err(err),
        };
        match outcome {
            Ok(()) => {
                if e.info.is_none() {
                    e.info = aurora_assets::j2k_info(&e.data);
                }
                e.failures = 0;
                if !e.dirty_cache {
                    e.dirty_cache = true;
                    self.dirty.push(id);
                }
                e.last_error = None;
                e.error_logged = false;
                e.gone = false;
            }
            Err(err) => {
                e.failures += 1;
                let backoff = 2u64.pow(e.failures.min(6));
                e.retry_at = Some(Instant::now() + Duration::from_secs(backoff));
                e.gone = r.status == 404;
                if r.status == 404 {
                    e.retry_at = Some(Instant::now() + Duration::from_secs(600));
                }
                let source = match e.source {
                    TexSource::Bake { .. } => " (bake service)",
                    TexSource::Asset => "",
                };
                if e.error_logged {
                    log::debug!("texture {id} fetch failed{source}: {err} (failure {})", e.failures);
                } else {
                    e.error_logged = true;
                    log::info!("texture {id} fetch failed{source}: {err}");
                }
                e.last_error = Some(err);
            }
        }
    }

    pub fn on_job(&mut self, r: JobResult) {
        match r {
            JobResult::TextureCache { id, data, complete } => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.cache_state = 2;
                    if let Some(d) = data
                        && d.len() > e.data.len()
                    {
                        e.info = aurora_assets::j2k_info(&d);
                        e.data = Arc::new(d);
                        e.complete = complete;
                    }
                }
            }
            JobResult::Texture {
                id,
                discard,
                mips,
                staged,
                alpha,
                alpha_channel,
                pick_mask,
                sculpt,
            } => {
                self.decodes = self.decodes.saturating_sub(1);
                let Some(e) = self.entries.get_mut(&id) else {
                    return;
                };
                e.decoding = false;
                if sculpt.is_some() {
                    e.sculpt = sculpt;
                    self.generation += 1;
                }
                self.uploads.push(
                    PendingUpload {
                        id,
                        discard,
                        staged,
                        mips,
                        alpha,
                        alpha_channel,
                        pick_mask,
                        priority: UploadPriority::default(),
                        #[cfg(test)]
                        held: false,
                    },
                    e.decoded.is_none(),
                    e.need_px.max(e.fixed_px),
                );
            }
            JobResult::TextureFailed { id } => {
                self.decodes = self.decodes.saturating_sub(1);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.decoding = false;
                    e.failures += 1;
                    // Corrupt cached data: drop it and refetch.
                    if e.failures >= 2 {
                        e.data = Arc::default();
                        e.complete = false;
                        e.info = None;
                        let _ = std::fs::remove_file(self.cache_dir.join("tex").join(format!("{id}.j2c")));
                        let _ = std::fs::remove_file(self.cache_dir.join("tex").join(format!("{id}.part")));
                    }
                    e.retry_at = Some(Instant::now() + Duration::from_secs(2u64.pow(e.failures.min(6))));
                }
            }
            _ => {}
        }
    }

    /// AURORA_DEMO_TEXTURES: the stress textures arrive as decoded jobs, a
    /// low-resolution level first then the full one, so they take the
    /// streaming path of grid textures (placeholder, then upgrades).
    pub fn install_demo_stress(&mut self, renderer: &mut Renderer, textures: impl Iterator<Item = u32>) {
        for i in textures {
            let id = crate::demo::stress_texture(i);
            let _ = self.acquire(renderer, id, TexSource::Asset);
            for discard in [2, 0] {
                self.on_job(texture_result(
                    id,
                    discard as u8,
                    crate::demo::stress_texture_mips(i, discard),
                    (AlphaKind::Opaque, false),
                    None,
                    self.staging.as_deref(),
                ));
            }
        }
    }

    /// AURORA_DEMO_STREAM: the downloaded data a texture loaded at full
    /// resolution holds (the demo textures arrive decoded, without any).
    pub fn demo_attach_data(&mut self, id: &Uuid, data: Vec<u8>) {
        if let Some(e) = self.entries.get_mut(id)
            && e.decoded == Some(0)
        {
            e.data = Arc::new(data);
            e.complete = true;
        }
    }

    /// Send decoded textures to the GPU, most visible first, within the
    /// frame's budget (at least one a frame, so a slow one still goes).
    pub fn upload(&mut self, renderer: &mut Renderer, jobs: &Jobs, budget: UploadBudget) {
        let t0 = Instant::now();
        let mut spent = 0u64;
        let mut done = 0usize;
        // uploads whose texture page is not there yet, put back at the end
        let mut waiting: Vec<PendingUpload> = Vec::new();
        self.uploads.promote_ready();
        while budget.allows(done, spent, t0.elapsed()) {
            let Some(mut u) = self.uploads.pop(&mut self.trash) else {
                break;
            };
            let Some(e) = self.entries.get_mut(&u.id) else {
                u.retire(&mut self.trash);
                continue;
            };
            if e.decoded.is_some_and(|d| d <= u.discard) {
                u.retire(&mut self.trash);
                continue;
            }
            if !u.ready() {
                // (never expected: only copyable uploads are in the heap)
                self.uploads.requeue(u);
                continue;
            }
            // a new texture page is made by the renderer's page thread
            // (0.4 to 1.5 ms here when the device took a new block of video
            // memory), ahead of the uploads queued for it: only the first
            // of a size waits a frame or two, and the others go meanwhile
            let queued = &self.uploads;
            if u.chain()
                .is_some_and(|(w, h, levels)| !renderer.textures.prepare_page(e.slot, w, h, levels, queued.queued((w, h, levels))))
            {
                waiting.push(u);
                if waiting.len() >= PAGE_WAITS {
                    // (thousands may wait for that same page)
                    break;
                }
                continue;
            }
            let (accepted, level0, levels) = match &u.staged {
                Some(staged) => (
                    renderer.replace_texture_staged(e.slot, staged),
                    staged.levels.first().map(|l| (l.width, l.height)),
                    staged.levels.len(),
                ),
                None => {
                    let levels: Vec<MipLevel> = u
                        .mips
                        .iter()
                        .map(|(w, h, d)| MipLevel {
                            width: *w,
                            height: *h,
                            data: d,
                        })
                        .collect();
                    (
                        renderer.replace_texture(e.slot, &levels),
                        levels.first().map(|l| (l.width, l.height)),
                        levels.len(),
                    )
                }
            };
            done += 1;
            if !accepted {
                // keep the current texels, and do not decode it again in a loop
                log::warn!(
                    "texture {}: upload refused by the renderer (slot {}, {levels} levels, level 0 {level0:?}, staged {})",
                    u.id,
                    e.slot,
                    u.staged.is_some(),
                );
                e.decoded = Some(u.discard);
                e.failures += 1;
                u.retire(&mut self.trash);
                continue;
            }
            spent += match &u.staged {
                Some(staged) => {
                    self.stats.uploads_staged += 1;
                    staged.bytes()
                }
                None => {
                    self.stats.uploads_direct += 1;
                    u.mips.iter().map(|m| m.2.len() as u64).sum::<u64>()
                }
            };
            e.gpu_size = level0.unwrap_or_default();
            if self.ui_wanted.contains(&u.id) {
                // largest level that fits 256 px for the UI copy (taken
                // from the upload, which is done with it)
                let at = u.mips.iter().position(|m| m.0.max(m.1) <= 256).or(u.mips.len().checked_sub(1));
                if let Some((w, h, d)) = at.and_then(|i| u.mips.get_mut(i))
                    && self.ui_ready.get(&u.id).is_none_or(|r| r.0 < *w)
                {
                    self.ui_ready.insert(u.id, (*w, *h, std::mem::take(d)));
                }
            }
            e.decoded = Some(u.discard);
            e.alpha = u.alpha;
            let old = if let Some(mask) = u.pick_mask.take() {
                self.pick_masks.insert(u.id, mask)
            } else {
                self.pick_masks.remove(&u.id)
            };
            if let Some(mask) = old {
                self.trash.push(mask.bits);
            }
            set_slot_alpha(
                &mut self.alpha_by_slot,
                &mut self.generation,
                &mut self.alpha_changes,
                e.slot,
                u.alpha,
            );
            let slot = e.slot as usize;
            if slot >= self.alpha_channel_by_slot.len() {
                self.alpha_channel_by_slot.resize(slot + 1024, false);
            }
            if self.alpha_channel_by_slot[slot] != u.alpha_channel {
                self.alpha_channel_by_slot[slot] = u.alpha_channel;
                self.generation += 1;
                self.alpha_changes.push(e.slot);
            }
            u.retire(&mut self.trash);
        }
        for u in waiting {
            self.uploads.requeue(u);
        }
        self.trash.empty(jobs);
        self.stats.uploaded_bytes_frame = spent;
        self.stats.uploads_frame = done;
    }

    /// Keep the memory budget, evict unused textures and persist downloaded
    /// data. Every frame: a few of the textures to lower under memory
    /// pressure (`reduce`), and a few of those unused for `evict_after`,
    /// taken from the front of the release queue; a few times a second: the
    /// budget and the pressure step (`press`); every two seconds: the data
    /// downloaded since, written to the disk cache by the jobs. No
    /// pass over every texture (with ~9,000 of them, and a region's worth
    /// evicted at once 45 s after a teleport, that pass took 5 to 65 ms).
    pub fn maintain(&mut self, renderer: &mut Renderer, jobs: &Jobs, budget_bytes: u64, frame: Duration) {
        let now = Instant::now();
        self.reduce(renderer, frame);
        self.press(renderer, now, budget_bytes);
        let entries = &self.entries;
        // (under memory pressure, what nothing uses goes sooner)
        let evict_after = if self.pressure.step() > 0 {
            self.evict_after.min(texture_budget::PRESSURE_EVICT)
        } else {
            self.evict_after
        };
        let evict = eviction_round(&mut self.unused, now, evict_after, |id, since| match entries.get(id) {
            Some(e) if e.refs == 0 && e.unused_since == Some(since) => {
                if e.fetching || e.decoding {
                    Evictable::Busy
                } else {
                    Evictable::Yes
                }
            }
            // used again since (a later release queues it again), or gone
            _ => Evictable::No,
        });
        if !evict.is_empty() {
            // the J2C data and sculpt maps go with the entries: freed off
            // this thread
            let mut gone = Vec::with_capacity(evict.len());
            for id in evict {
                if let Some(e) = self.entries.remove(&id) {
                    if let Some(mask) = self.pick_masks.remove(&id) {
                        self.trash.push(mask.bits);
                    }
                    renderer.free_texture(e.slot);
                    set_slot_alpha(
                        &mut self.alpha_by_slot,
                        &mut self.generation,
                        &mut self.alpha_changes,
                        e.slot,
                        AlphaKind::Opaque,
                    );
                    gone.push((e.data, e.sculpt, e.last_error));
                }
            }
            jobs.discard(gone);
        }
        if now.duration_since(self.last_maintenance) < HOUSEKEEPING_EVERY {
            return;
        }
        self.last_maintenance = now;
        // downloaded data to persist: shared, not copied, and queued in one go
        let mut writes = Vec::new();
        let mut later = Vec::new();
        for id in std::mem::take(&mut self.dirty) {
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            if !e.dirty_cache {
                continue;
            }
            if e.fetching {
                // more bytes are on their way: written with them
                later.push(id);
            } else {
                e.dirty_cache = false;
                if !e.data.is_empty() {
                    writes.push((id, e.complete, e.data.clone()));
                }
            }
        }
        self.dirty = later;
        let dir = self.cache_dir.clone();
        jobs.spawn_many(writes, move |(id, complete, data)| {
            let path = Self::cache_path(&dir, &id, complete);
            let tmp = path.with_extension("tmp");
            if crate::cache::write(&tmp, &data).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
            if complete {
                let _ = std::fs::remove_file(Self::cache_path(&dir, &id, false));
            }
            JobResult::TextureCache { id, data: None, complete }
        });
    }

    /// Lower a few of the textures listed by the last pass under memory
    /// pressure: the levels they keep are copied on the GPU to a smaller
    /// layer (`TextureTable::shrink`), nothing is decoded. Each is checked
    /// again, the list may be a few frames old.
    fn reduce(&mut self, renderer: &mut Renderer, frame: Duration) {
        let most =
            ((frame.as_secs_f32() * REDUCTIONS_PER_SECOND) as usize).clamp(*REDUCTIONS_PER_FRAME.start(), *REDUCTIONS_PER_FRAME.end());
        let mut done = 0;
        while done < most {
            let Some((id, keep, _)) = self.reductions.pop() else {
                break;
            };
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            let Some(d) = e.decoded.filter(|&d| keep > d && e.refs > 0 && !e.decoding) else {
                continue;
            };
            let levels = renderer.textures.shrink(e.slot, (keep - d) as u32) as u8;
            if levels > 0 {
                e.decoded = Some(d + levels);
                e.gpu_size = ((e.gpu_size.0 >> levels).max(1), (e.gpu_size.1 >> levels).max(1));
                self.stats.reduced += 1;
                done += 1;
            }
        }
    }

    /// Memory budget, a few times a second: what the textures may hold now
    /// and the pressure step for what they hold (texture_budget.rs).
    fn press(&mut self, renderer: &mut Renderer, now: Instant, setting: u64) {
        if now.duration_since(self.last_budget) < BUDGET_EVERY {
            return;
        }
        self.last_budget = now;
        let (used, live) = (renderer.textures.page_bytes(), renderer.textures.bytes());
        let video = renderer.video_memory.read(&renderer.device);
        let target = texture_budget::forced_target().unwrap_or_else(|| texture_budget::texture_target(setting, used, video));
        let before = self.pressure.step();
        // one step below, the pages would hold the textures' bytes with
        // the free room they have now (none taken back from step 1: only
        // what loads from then on changes)
        let below = if before > 1 {
            (self.bytes_below as f64 * (used as f64 / live.max(1) as f64)) as u64
        } else {
            used
        };
        let step = self.pressure.update(
            now,
            texture_budget::Reading {
                used,
                target,
                settled: self.passes_at_step >= 2 && self.reductions.is_empty() && !renderer.textures.settling(),
                below,
            },
        );
        if step != before {
            self.passes_at_step = 0;
            renderer.textures.set_eager_compaction(step > 0);
            log::info!(
                "texture memory: pressure {before} -> {step} ({} MB in pages for a target of {} MB, {} MB of textures{}; {} lowered so far)",
                used >> 20,
                target >> 20,
                live >> 20,
                video.map_or(String::new(), |v| format!(
                    ", video memory: {} MB of resources, {} MB taken from the driver, budget {} MB",
                    v.allocated >> 20,
                    v.usage >> 20,
                    v.budget >> 20
                )),
                self.stats.reduced,
            );
        }
        self.stats.target_bytes = target;
        self.stats.pressure = step;
    }
}

/// Whether the texture at the front of the unused queue can be evicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Evictable {
    Yes,
    /// Still unused but a fetch or a decode is in flight: asked again later.
    Busy,
    /// Used again since it was queued, or already gone.
    No,
}

/// One frame of eviction: the textures at the front of `unused` (oldest
/// release first) whose time has come, at most [`EVICT_PER_FRAME`] of them
/// and [`EVICT_LOOKS_PER_FRAME`] queue entries looked at. `state` tells
/// what became of each entry; busy ones go back to the end of the queue.
fn eviction_round(
    unused: &mut VecDeque<(Instant, Uuid)>,
    now: Instant,
    evict_after: Duration,
    mut state: impl FnMut(&Uuid, Instant) -> Evictable,
) -> Vec<Uuid> {
    let mut evict = Vec::new();
    let mut busy = Vec::new();
    let mut looks = 0;
    while evict.len() < EVICT_PER_FRAME && looks < EVICT_LOOKS_PER_FRAME {
        let Some(&(since, id)) = unused.front() else {
            break;
        };
        if now.saturating_duration_since(since) <= evict_after {
            break;
        }
        unused.pop_front();
        looks += 1;
        match state(&id, since) {
            Evictable::Yes => evict.push(id),
            Evictable::Busy => busy.push((since, id)),
            Evictable::No => {}
        }
    }
    unused.extend(busy);
    evict
}

/// A decoded mip chain as a finished job. When the renderer's staging
/// memory has room, the job writes the chain there itself (the main thread
/// then only records the GPU copies) and keeps only the levels up to 256 px
/// (UI copies); else the whole chain stays in memory for a main-thread
/// write.
pub fn texture_result(
    id: Uuid,
    discard: u8,
    mut mips: Vec<(u32, u32, Vec<u8>)>,
    (alpha, alpha_channel): (AlphaKind, bool),
    sculpt: Option<Arc<SculptMap>>,
    pool: Option<&StagingPool>,
) -> JobResult {
    // Generated on the decode worker, before staging may release CPU levels.
    let pick_mask = (alpha != AlphaKind::Opaque)
        .then(|| mips.first().and_then(|(w, h, rgba)| PickMask::from_rgba(*w, *h, rgba)))
        .flatten();
    let staged = pool.and_then(|pool| {
        let levels: Vec<(u32, u32, &[u8])> = mips.iter().map(|(w, h, d)| (*w, *h, d.as_slice())).collect();
        StagedTexture::new(pool, &levels)
    });
    if staged.is_some() {
        mips.retain(|m| m.0.max(m.1) <= 256);
    }
    JobResult::Texture {
        id,
        discard,
        mips,
        staged,
        alpha,
        alpha_channel,
        pick_mask,
        sculpt,
    }
}

fn decode_job(id: Uuid, data: &[u8], discard: u8, keep_pixels: bool, pool: Option<&StagingPool>) -> JobResult {
    match aurora_assets::decode_j2k(data, discard) {
        Ok(img) => {
            let pixels = (img.width * img.height) as usize;
            let rgba = if img.components == 4 {
                img.data.clone()
            } else {
                to_rgba(img.components, &img.data, pixels)
            };
            let alpha_channel = img.components == 4 || img.components == 2;
            let alpha = if alpha_channel { classify_alpha(&rgba) } else { AlphaKind::Opaque };
            let sculpt = keep_pixels.then(|| {
                Arc::new(SculptMap {
                    width: img.width,
                    height: img.height,
                    components: img.components,
                    pixels: img.data,
                })
            });
            let mips = build_mips(img.width, img.height, rgba, 16);
            texture_result(id, img.discard, mips, (alpha, alpha_channel), sculpt, pool)
        }
        Err(e) => {
            log::debug!("texture {id} decode failed at discard {discard}: {e}");
            JobResult::TextureFailed { id }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_mask_threshold_repeated_uvs_and_invalid_pixels() {
        let mut rgba = vec![0; 4 * 4 * 4];
        rgba[(3 * 4) * 4 + 3] = 32;
        rgba[(3 * 4 + 2) * 4 + 3] = 33;
        rgba[4 * 4 + 3] = 255;
        let mask = PickMask::from_rgba(4, 4, &rgba).unwrap();
        assert!(!mask.hits(glam::Vec2::new(0.2, 0.3)));
        assert!(mask.hits(glam::Vec2::new(0.7, 0.3)));
        assert!(mask.hits(glam::Vec2::new(-0.3, 2.3)));
        assert!(mask.hits(glam::Vec2::new(0.2, 0.7)));
        assert!(!mask.hits(glam::Vec2::new(0.7, 0.7)));
        assert!(!mask.hits(glam::Vec2::splat(f32::NAN)));
        assert!(PickMask::from_rgba(0, 2, &rgba).is_none());
        assert!(PickMask::from_rgba(4, 4, &rgba[..4]).is_none());
        let textures = TextureStreamer::new(PathBuf::new());
        assert!(!textures.pick_masks[&aurora_prim::te::TRANSPARENT_TEXTURE].hits(glam::Vec2::splat(0.5)));
    }

    /// A 1024 × 1024 texture in use, loaded at `decoded`, covering `need_px`
    /// pixels in view.
    fn entry(decoded: Option<u8>, need_px: f32) -> Entry {
        Entry {
            slot: 9,
            refs: 1,
            source: TexSource::Asset,
            data: Arc::default(),
            complete: false,
            info: None,
            decoded,
            want: 0,
            need_px,
            away_px: 0.0,
            fixed_px: 0.0,
            gpu_size: decoded.map_or((0, 0), |d| (1024 >> d, 1024 >> d)),
            fetching: false,
            decoding: false,
            cache_state: 2,
            failures: 0,
            retry_at: None,
            keep_pixels: false,
            sculpt: None,
            alpha: AlphaKind::Opaque,
            unused_since: None,
            fetch_key: 0,
            dirty_cache: false,
            last_error: None,
            error_logged: false,
            gone: false,
        }
    }

    #[test]
    fn discard_follows_the_size_on_screen() {
        assert_eq!(discard_for(1024.0, 1024.0, None), 0);
        assert_eq!(discard_for(1024.0, 2000.0, None), 0);
        assert_eq!(discard_for(1024.0, 300.0, None), 1);
        assert_eq!(discard_for(1024.0, 256.0, None), 2);
        // never past level 5, nor past the levels of the file
        assert_eq!(discard_for(1024.0, 0.0, None), 5);
        assert_eq!(discard_for(1024.0, 0.0, Some(3)), 3);
        assert_eq!(discard_for(64.0, 8.0, Some(5)), 3);
    }

    #[test]
    fn lowering_stops_half_a_level_above_loading() {
        // in view at 512 px: loaded to level 1, not lowered while at 0
        let e = entry(Some(0), 512.0);
        assert_eq!(wanted(1024.0, None, &e, (1.0, 1.0)), (1, 0));
        // clearly smaller on screen: lowered
        let e = entry(Some(0), 300.0);
        assert_eq!(wanted(1024.0, None, &e, (1.0, 1.0)), (1, 1));
        // whatever the sizes and the levels dropped, never lowered past
        // what would load again
        for need in [0.0, 5.0, 40.0, 181.0, 182.0, 362.0, 363.0, 724.0, 725.0, 3000.0] {
            for away in [0.0, 90.0, 400.0, 1500.0] {
                for step in 0..=texture_budget::MAX_PRESSURE {
                    let mut e = entry(Some(0), need);
                    e.away_px = away;
                    let (want, keep) = wanted(1024.0, Some(5), &e, texture_budget::dropped(step));
                    assert!(keep <= want, "need {need} away {away} step {step}: keep {keep} want {want}");
                }
            }
        }
    }

    #[test]
    fn out_of_view_textures_keep_what_their_distance_asks() {
        // behind the camera, 500 px from where it is: not loaded for that
        // (level 5 wanted), but not lowered either while memory allows
        let mut e = entry(Some(0), 0.0);
        e.away_px = 500.0;
        let at = |e: &Entry, step: u8| wanted(1024.0, None, e, texture_budget::dropped(step));
        assert_eq!(at(&e, 1), (5, 0));
        // then half a level at a time under it, down to two levels
        assert_eq!([2, 3, 4, 5, 6, 7].map(|s| at(&e, s).1), [1, 1, 2, 2, 2, 2]);
        // in view: nothing until those out of view have lost a level
        let seen = entry(Some(0), 500.0);
        assert_eq!([1, 2, 3].map(|s| at(&seen, s)), [(1, 0); 3]);
        assert_eq!([4, 5, 6, 7].map(|s| at(&seen, s)), [(1, 1), (2, 1), (2, 2), (3, 2)]);
        // HUDs, sky and terrain: whatever the pressure
        let mut fixed = entry(Some(0), 0.0);
        fixed.fixed_px = 500.0;
        assert_eq!(at(&fixed, texture_budget::MAX_PRESSURE), (1, 0));
    }

    #[test]
    fn bytes_follow_the_discard_level() {
        // a demo texture (no file): its full size from what the GPU holds
        let e = entry(Some(2), 0.0);
        assert_eq!(full_size(&e), Some((1024, 1024)));
        assert_eq!(bytes_at(&e, 0), 1024 * 1024 * 16 / 3);
        assert_eq!(bytes_at(&e, 2), (1024 * 1024 * 16 / 3) >> 4);
        // nothing known yet
        assert_eq!(bytes_at(&entry(None, 0.0), 0), 0);
    }

    /// The streamer under memory pressure step `step`, holding `entries`.
    fn pressed(step: u8, entries: Vec<(u128, Entry)>) -> TextureStreamer {
        let mut s = TextureStreamer::new(PathBuf::new());
        let t0 = Instant::now();
        for i in 0..step as u32 {
            s.pressure.update(
                t0 + Duration::from_secs(2 * i as u64),
                texture_budget::Reading {
                    used: 2,
                    target: 1,
                    settled: true,
                    below: 2,
                },
            );
        }
        assert_eq!(s.pressure.step(), step);
        s.entries.extend(entries.into_iter().map(|(id, e)| (Uuid::from_u128(id), e)));
        s
    }

    #[test]
    fn nothing_is_lowered_without_memory_pressure() {
        let mut s = pressed(0, vec![(1, entry(Some(0), 20.0)), (2, entry(Some(0), 0.0))]);
        s.recompute_wants();
        assert!(s.reductions.is_empty());
        // (loading still follows the size on screen)
        assert_eq!(s.entries[&Uuid::from_u128(1)].want, 5);
    }

    #[test]
    fn pressure_lists_what_is_finer_than_its_size_on_screen() {
        let bake = Entry {
            source: TexSource::Bake { url: String::new() },
            ..entry(Some(0), 20.0)
        };
        let sculpt = Entry {
            keep_pixels: true,
            ..entry(Some(0), 20.0)
        };
        let unused = Entry {
            refs: 0,
            ..entry(Some(0), 20.0)
        };
        let decoding = Entry {
            decoding: true,
            ..entry(Some(0), 20.0)
        };
        let mut s = pressed(
            1,
            vec![
                (1, entry(Some(0), 200.0)),
                (2, entry(Some(0), 900.0)),
                (3, entry(Some(3), 200.0)),
                (4, entry(None, 0.0)),
                (5, bake),
                (6, sculpt),
                (7, unused),
                (8, decoding),
                (9, entry(Some(0), 20.0)),
                (10, entry(Some(1), 40.0)),
            ],
        );
        s.ui_wanted.insert(Uuid::from_u128(9));
        s.recompute_wants();
        // only the textures loaded finer than their size on screen asks
        // for: the one at 200 px to level 1 (level 2 would load it again
        // at 256 px), the one at 40 px to level 4; the largest is last
        // (taken first)
        let mb = 1024 * 1024 * 16 / 3;
        assert_eq!(s.reductions, vec![(Uuid::from_u128(10), 4, mb >> 2), (Uuid::from_u128(1), 1, mb)]);
        // bytes one step below: the same textures at the same levels
        assert!(s.bytes_below > 0);
        // the sizes decay for the next pass
        assert_eq!(s.entries[&Uuid::from_u128(1)].need_px, 100.0);
    }

    fn pending(id: u128, discard: u8) -> PendingUpload {
        PendingUpload {
            id: Uuid::from_u128(id),
            discard,
            staged: None,
            mips: vec![(1, 1, vec![0; 4])],
            alpha: AlphaKind::Opaque,
            alpha_channel: false,
            pick_mask: None,
            priority: UploadPriority::default(),
            held: false,
        }
    }

    #[test]
    fn uploads_not_copyable_yet_wait_apart() {
        let mut q = PendingUploads::default();
        let mut trash = Trash::default();
        // texture 1 is the most urgent, but a job still writes in its chunk
        let mut held = pending(1, 0);
        held.held = true;
        q.push(held, true, 900.0);
        q.push(pending(2, 0), true, 50.0);
        q.push(pending(3, 0), false, 400.0);
        assert_eq!(q.len(), 3);
        q.promote_ready();
        // the copyable ones go, in their order, without meeting the other
        assert_eq!(q.pop(&mut trash).map(|u| u.id.as_u128()), Some(2));
        assert_eq!(q.pop(&mut trash).map(|u| u.id.as_u128()), Some(3));
        assert!(q.pop(&mut trash).is_none());
        assert_eq!(q.len(), 1);
        // its chunk is closed: it takes its place among the newer ones
        q.push(pending(4, 0), true, 100.0);
        q.unready[0].held = false;
        q.promote_ready();
        assert!(q.unready.is_empty());
        let order: Vec<u128> = std::iter::from_fn(|| q.pop(&mut trash)).map(|u| u.id.as_u128()).collect();
        assert_eq!(order, [1, 4]);
        // a coarser level waiting apart is dropped for a finer one in the heap
        let mut coarse = pending(5, 2);
        coarse.held = true;
        q.push(coarse, true, 10.0);
        q.push(pending(5, 0), true, 10.0);
        q.unready[0].held = false;
        q.promote_ready();
        let got: Vec<u8> = std::iter::from_fn(|| q.pop(&mut trash)).map(|u| u.discard).collect();
        assert_eq!(got, [0]);
        assert!(q.finest.is_empty() && q.len() == 0);
    }

    #[test]
    fn waiting_uploads_are_counted_per_chain_size() {
        let mut q = PendingUploads::default();
        let mut trash = Trash::default();
        let sized = |id, discard, mips: &[(u32, u32)]| {
            let mut u = pending(id, discard);
            u.mips = mips.iter().map(|&(w, h)| (w, h, vec![0; (w * h * 4) as usize])).collect();
            u
        };
        let (big, small) = ([(4, 4), (2, 2), (1, 1)], [(2, 2), (1, 1)]);
        // copyable or not, both wait for a layer of their size
        let mut held = sized(1, 0, &big);
        held.held = true;
        q.push(held, true, 10.0);
        q.push(sized(2, 0, &big), true, 20.0);
        q.push(sized(3, 0, &small), true, 30.0);
        assert_eq!((q.queued((4, 4, 3)), q.queued((2, 2, 2)), q.queued((8, 8, 4))), (2, 1, 0));
        // the one popped no longer counts; put back, it does again
        let u = q.pop(&mut trash).expect("upload");
        assert_eq!((u.id.as_u128(), q.queued((2, 2, 2))), (3, 0));
        q.requeue(u);
        assert_eq!(q.queued((2, 2, 2)), 1);
        // a coarser level retired on the way is forgotten too
        q.push(sized(4, 2, &small), true, 90.0);
        q.push(sized(4, 0, &big), true, 90.0);
        assert_eq!((q.queued((4, 4, 3)), q.queued((2, 2, 2))), (3, 2));
        q.unready[0].held = false;
        q.promote_ready();
        while q.pop(&mut trash).is_some() {}
        assert!(q.chains.is_empty());
    }

    #[test]
    fn retired_uploads_leave_their_pixels_to_the_trash() {
        let mut trash = Trash::default();
        let mut u = pending(1, 0);
        u.mips = vec![(2, 2, vec![0; 16]), (1, 1, vec![0; 4]), (1, 1, Vec::new())];
        u.retire(&mut trash);
        // (a level taken for the UI copy left an empty one behind)
        assert_eq!(trash.len(), 2);
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn eviction_takes_the_oldest_unused_textures_first() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut unused: VecDeque<(Instant, Uuid)> = (0..5).map(|i| (t0 + s(i), id(i as u128))).collect();
        // at 47.5 s with 45 s of grace: released at 0, 1 and 2 s
        let evict = eviction_round(&mut unused, t0 + Duration::from_millis(47_500), s(45), |_, _| Evictable::Yes);
        assert_eq!(evict, [id(0), id(1), id(2)]);
        assert_eq!(unused.len(), 2);
        // nothing is due before its time
        assert!(eviction_round(&mut unused, t0 + s(48), s(45), |_, _| Evictable::Yes).is_empty());
        assert_eq!(unused.front().map(|u| u.1), Some(id(3)));
    }

    #[test]
    fn eviction_skips_textures_used_again_and_retries_busy_ones() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut unused: VecDeque<(Instant, Uuid)> = (0..6).map(|i| (t0 + s(i), id(i as u128))).collect();
        let mut asked = Vec::new();
        let evict = eviction_round(&mut unused, t0 + s(100), s(45), |id, since| {
            asked.push((*id, since));
            match id.as_u128() {
                // acquired again since, or already gone
                1 | 4 => Evictable::No,
                // a fetch still in flight
                2 => Evictable::Busy,
                _ => Evictable::Yes,
            }
        });
        assert_eq!(evict, [id(0), id(3), id(5)]);
        // each entry was asked once, with the time it was released
        assert_eq!(asked.len(), 6);
        assert_eq!(asked[3], (id(3), t0 + s(3)));
        // the busy one waits at the end with its release time
        assert_eq!(unused, [(t0 + s(2), id(2))]);
        let evict = eviction_round(&mut unused, t0 + s(101), s(45), |_, _| Evictable::Yes);
        assert_eq!(evict, [id(2)]);
        assert!(unused.is_empty());
    }

    #[test]
    fn eviction_is_spread_over_frames() {
        let t0 = Instant::now();
        let n = 5 * EVICT_PER_FRAME + 7;
        let mut unused: VecDeque<(Instant, Uuid)> = (0..n).map(|i| (t0, id(i as u128))).collect();
        let now = t0 + Duration::from_secs(60);
        let mut frames = 0;
        let mut total = 0;
        while !unused.is_empty() {
            let evict = eviction_round(&mut unused, now, UNUSED_EVICT, |_, _| Evictable::Yes);
            assert!(!evict.is_empty() && evict.len() <= EVICT_PER_FRAME);
            total += evict.len();
            frames += 1;
        }
        assert_eq!((total, frames), (n, 6));
        // entries used again since are only looked at, a bounded number a frame
        let mut unused: VecDeque<(Instant, Uuid)> = (0..3 * EVICT_LOOKS_PER_FRAME).map(|i| (t0, id(i as u128))).collect();
        assert!(eviction_round(&mut unused, now, UNUSED_EVICT, |_, _| Evictable::No).is_empty());
        assert_eq!(unused.len(), 2 * EVICT_LOOKS_PER_FRAME);
        // all busy: they are not asked twice in the same frame
        let mut unused: VecDeque<(Instant, Uuid)> = (0..10).map(|i| (t0, id(i as u128))).collect();
        let mut asked = 0;
        let evict = eviction_round(&mut unused, now, UNUSED_EVICT, |_, _| {
            asked += 1;
            Evictable::Busy
        });
        assert!(evict.is_empty());
        assert_eq!((asked, unused.len()), (10, 10));
    }

    #[test]
    fn placeholders_then_largest_on_screen_then_oldest() {
        let mut q = PendingUploads::default();
        q.push(pending(1, 0), false, 900.0); // upgrade, large
        q.push(pending(2, 2), true, 10.0); // first texels, small
        q.push(pending(3, 2), true, 400.0); // first texels, large
        q.push(pending(4, 0), false, 50.0); // upgrade, small
        q.push(pending(5, 2), true, 400.0); // same as 3, queued later
        assert_eq!(q.len(), 5);
        let mut trash = Trash::default();
        let order: Vec<u128> = std::iter::from_fn(|| q.pop(&mut trash)).map(|u| u.id.as_u128()).collect();
        assert_eq!(order, [3, 5, 2, 1, 4]);
    }

    #[test]
    fn coarser_level_is_dropped_when_a_finer_one_waits() {
        let mut q = PendingUploads::default();
        q.push(pending(1, 2), true, 100.0);
        q.push(pending(2, 2), true, 50.0);
        // the full resolution of texture 1 arrives before its level 2 went
        q.push(pending(1, 0), true, 100.0);
        let mut trash = Trash::default();
        let got: Vec<(u128, u8)> = std::iter::from_fn(|| q.pop(&mut trash))
            .map(|u| (u.id.as_u128(), u.discard))
            .collect();
        assert_eq!(got, [(1, 0), (2, 2)]);
        assert!(q.finest.is_empty());
        // the pixels of the level dropped on the way are left to the trash
        assert_eq!(trash.len(), 1);
        // a coarser level queued after a finer one comes after it (the
        // caller drops it: the texture is finer by then)
        q.push(pending(3, 0), true, 1.0);
        q.push(pending(3, 2), true, 1.0);
        assert_eq!(q.pop(&mut trash).map(|u| u.discard), Some(0));
        assert_eq!(q.pop(&mut trash).map(|u| u.discard), Some(2));
        assert!(q.pop(&mut trash).is_none() && q.finest.is_empty());
    }

    #[test]
    fn requeued_upload_keeps_its_place() {
        let mut q = PendingUploads::default();
        q.push(pending(1, 0), true, 100.0);
        q.push(pending(2, 0), true, 50.0);
        // texture 1 is not ready (its staging chunk is still written)
        let mut trash = Trash::default();
        let first = q.pop(&mut trash).expect("upload");
        assert_eq!(first.id.as_u128(), 1);
        q.requeue(first);
        q.push(pending(3, 0), true, 75.0);
        let order: Vec<u128> = std::iter::from_fn(|| q.pop(&mut trash)).map(|u| u.id.as_u128()).collect();
        assert_eq!(order, [1, 3, 2]);
    }

    #[test]
    fn budget_follows_the_frame_time() {
        let ms = |v: f32| Duration::from_secs_f32(v / 1000.0);
        // fast frames: the floor
        assert_eq!(UploadBudget::for_frame(ms(2.0)).time, Duration::from_micros(800));
        assert_eq!(UploadBudget::for_frame(ms(6.5)).time, Duration::from_micros(800));
        // 30 frames a second: 5 % of the frame
        let t = UploadBudget::for_frame(ms(33.3)).time;
        assert!((t.as_secs_f32() * 1000.0 - 1.665).abs() < 0.01, "{t:?}");
        // 15 frames a second and slower: capped
        assert_eq!(UploadBudget::for_frame(ms(66.7)).time, Duration::from_millis(3));
        assert_eq!(UploadBudget::for_frame(ms(100.0)).time, Duration::from_millis(3));
        assert_eq!(frame_share(ms(10.0), Duration::from_millis(1)), Duration::from_millis(1));
    }

    #[test]
    fn budget_always_lets_one_upload_through() {
        let b = UploadBudget {
            time: Duration::from_millis(1),
            bytes: 1000,
        };
        // nothing done yet: even over time (a slow frame start)
        assert!(b.allows(0, 0, Duration::from_millis(5)));
        assert!(b.allows(3, 999, Duration::from_micros(999)));
        // either budget spent stops the frame's uploads
        assert!(!b.allows(1, 1000, Duration::ZERO));
        assert!(!b.allows(1, 10, Duration::from_millis(1)));
    }

    #[test]
    fn first_request_starts_at_zero() {
        assert_eq!(
            next_range(0, INITIAL_BYTES),
            ByteRange {
                start: 0,
                end: Some(16383)
            }
        );
    }

    #[test]
    fn resumed_request_overlaps_one_byte() {
        // 16 KB held, 40 KB wanted: bytes=16383-40959
        assert_eq!(
            next_range(16384, 40960),
            ByteRange {
                start: 16383,
                end: Some(40959)
            }
        );
    }

    #[test]
    fn whole_file_requests_are_open_ended() {
        // usize::MAX = full resolution: MAX_IMAGE_DATA_SIZE, past the 20 MB end
        assert_eq!(next_range(16384, usize::MAX), ByteRange { start: 16383, end: None });
        assert_eq!(next_range(0, usize::MAX), ByteRange { start: 0, end: None });
        // a large partial wish stays bounded below the limit
        assert_eq!(
            next_range(0, 20_000_000),
            ByteRange {
                start: 0,
                end: Some(19_999_999)
            }
        );
        assert_eq!(
            next_range(1, 20_000_000),
            ByteRange {
                start: 0,
                end: Some(19_999_999)
            }
        );
        assert_eq!(next_range(0, 20_000_001), ByteRange { start: 0, end: None });
    }

    #[test]
    fn request_always_asks_for_new_bytes() {
        assert_eq!(next_range(100, 50), ByteRange { start: 99, end: Some(100) });
    }

    #[test]
    fn replies_merge_over_the_overlapping_byte() {
        let mut data = vec![1, 2, 3, 4];
        assert!(merge_reply(&mut data, 3, &[4, 5, 6]));
        assert_eq!(data, [1, 2, 3, 4, 5, 6]);
        // contiguous reply (server ignored the overlap)
        assert!(merge_reply(&mut data, 6, &[7]));
        assert_eq!(data, [1, 2, 3, 4, 5, 6, 7]);
        // from the start: only the new tail is kept
        assert!(merge_reply(&mut data, 0, &[1, 2, 3, 4, 5, 6, 7, 8]));
        assert_eq!(data, [1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn replies_that_leave_a_gap_are_refused() {
        let mut data = vec![1, 2, 3, 4];
        assert!(!merge_reply(&mut data, 5, &[6]));
        assert!(!merge_reply(&mut data, 0, &[1, 2]));
        assert_eq!(data, [1, 2, 3, 4]);
        let mut empty = Vec::new();
        assert!(merge_reply(&mut empty, 0, &[9]));
        assert_eq!(empty, [9]);
    }
}
