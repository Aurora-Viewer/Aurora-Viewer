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

use super::fetch_priority;
use super::jobs::{AlphaKind, JobResult, Jobs, SculptMap, Trash, classify_alpha, to_rgba};
use super::texture_budget::{self, Pressure};
use super::texture_cache::{self, Transcoded};
use super::texture_load::{DecodeMeter, LoadTotals, LoadWatch};
use aurora_assets::J2kInfo;
use aurora_net::fetch::ByteRange;
use aurora_net::{FetchRequest, FetchResult, Fetcher, Lane};
use aurora_render::{MipLevel, Renderer, StagedTexture, StagingPool, TexelFormat, build_mips};
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
/// First request of a texture whose size on screen is not known yet, and
/// the least asked for one that is.
const INITIAL_BYTES: usize = 16 * 1024;
/// The size the first request assumes for a file it has not seen: that of
/// most textures large enough on screen to need more than `INITIAL_BYTES`.
const TYPICAL: J2kInfo = J2kInfo {
    width: 1024,
    height: 1024,
    components: 3,
    levels: 5,
};

/// Bytes of the first request of a texture that covers `px` pixels on
/// screen (0: not known). The size of the file is not known before its
/// first bytes, and what a level needs depends on it: this asks what a
/// `TYPICAL` texture needs to show `px` pixels, so that the first reply is
/// enough for most textures and one round trip shows them at the level they
/// are wanted. Never its full resolution: that is the whole file, megabytes
/// for a large one, and is asked once the size is known. A smaller file
/// comes whole; a texture that needs more gets its first pixels from this
/// reply while the rest is asked (`holds_first_pixels`).
/// Firestorm always starts with `FIRST_PACKET_SIZE` (600 bytes) and asks
/// for the level in a second request (LLTextureFetchWorker::doWork).
/// Textures drawn with the built-in transparent slot, never downloaded: the
/// transparent texture, and the bake of a part an alpha layer hides whole
/// (IMG_INVISIBLE, never fetched from the bake service:
/// LLVOAvatar::getBakedTextureImage, indra/newview/llvoavatar.cpp). The
/// viewer-local alpha ramp is invisible in colour; its geometry goes to
/// the separate water exclusion pass (LLDrawPoolWaterExclusion::render).
pub fn is_builtin_transparent(id: &Uuid) -> bool {
    *id == aurora_prim::te::TRANSPARENT_TEXTURE || *id == super::avatar::IMG_INVISIBLE || *id == aurora_prim::te::WATER_EXCLUSION_TEXTURE
}

fn first_bytes(px: f32) -> usize {
    if px <= 0.0 {
        return INITIAL_BYTES;
    }
    let discard = discard_for(TYPICAL.width as f32, px, None).max(1);
    aurora_assets::bytes_for_discard(&TYPICAL, discard).max(INITIAL_BYTES)
}

/// Finest level, from `want` towards the coarsest, that `len` bytes of a
/// file hold. Level 0 only from the complete file (a partial one is
/// blurry).
fn level_held(info: &J2kInfo, len: usize, want: u8) -> u8 {
    let mut discard = want;
    while discard < 5 && (discard == 0 || aurora_assets::bytes_for_discard(info, discard) > len) {
        discard += 1;
    }
    discard
}
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
/// A second, full-resolution bit plane preserves alpha > 0 for left clicks
/// on world Blend faces, without changing the legacy mask used by HUDs.
pub struct PickMask {
    width: u32,
    height: u32,
    bits: Vec<u8>,
}

impl PickMask {
    /// As stored in the transcoded cache: (width, height, bits).
    fn parts(&self) -> (u32, u32, &[u8]) {
        (self.width, self.height, &self.bits)
    }

    /// A mask read back from the transcoded cache, if its bits fit its size.
    fn from_parts((width, height, bits): texture_cache::MaskParts) -> Option<Self> {
        let fits = width > 0 && height > 0 && Some(bits.len()) == texture_cache::pick_mask_bytes(width, height);
        fits.then_some(Self { width, height, bits })
    }

    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let pixels = (width as usize).checked_mul(height as usize)?;
        if width == 0 || height == 0 || rgba.len() != pixels.checked_mul(4)? {
            return None;
        }
        let (w, h) = (width.div_ceil(2), height.div_ceil(2));
        let coarse = (w as usize * h as usize).div_ceil(8);
        let mut bits = vec![0; coarse + pixels.div_ceil(8)];
        for y in 0..h {
            for x in 0..w {
                let row = height - 1 - y * 2;
                if rgba[((row * width + x * 2) as usize) * 4 + 3] > 32 {
                    let bit = (y * w + x) as usize;
                    bits[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        for y in 0..height {
            for x in 0..width {
                if rgba[(((height - 1 - y) * width + x) as usize) * 4 + 3] != 0 {
                    let bit = (y * width + x) as usize;
                    bits[coarse + bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        Some(Self { width, height, bits })
    }

    pub fn hits(&self, uv: glam::Vec2) -> bool {
        self.hits_plane(uv, self.width.div_ceil(2), self.height.div_ceil(2), 0)
    }

    pub fn hits_nonzero(&self, uv: glam::Vec2) -> bool {
        let coarse = (self.width.div_ceil(2) as usize * self.height.div_ceil(2) as usize).div_ceil(8);
        self.hits_plane(uv, self.width, self.height, coarse)
    }

    fn hits_plane(&self, uv: glam::Vec2, width: u32, height: u32, offset: usize) -> bool {
        let uv = if uv.is_finite() { uv } else { glam::Vec2::ZERO };
        let x = (uv.x.rem_euclid(1.0) * width as f32) as usize;
        let y = (uv.y.rem_euclid(1.0) * height as f32) as usize;
        let bit = y.min(height as usize - 1) * width as usize + x.min(width as usize - 1);
        self.bits[offset + bit / 8] & (1 << (bit % 8)) != 0
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
    /// Pixels in view `want` was last computed from (0 before the first
    /// time): unlike `need_px`, it does not decay between two computations.
    seen_px: f32,
    /// Its first request, until its first pixels are shown (the wait the
    /// `texture load:` line tells).
    asked_at: Option<Instant>,
    /// Size of its level 0 on the GPU (0 until the first upload).
    gpu_size: (u32, u32),
    fetching: bool,
    decoding: bool,
    /// 0 unchecked, 1 being read, 2 the JPEG 2000 data is read (or is not
    /// on the disk), 3 shown from the transcoded cache, the JPEG 2000 data
    /// left on the disk until a finer level is wanted.
    cache_state: u8,
    /// Finest discard level the transcoded cache holds for this texture.
    transcoded: Option<u8>,
    /// BC7 would harm this texture (known from the transcoded cache).
    keep_rgba: bool,
    failures: u32,
    retry_at: Option<Instant>,
    keep_pixels: bool,
    sculpt: Option<Arc<SculptMap>>,
    alpha: AlphaKind,
    unused_since: Option<Instant>,
    fetch_key: u64,
    /// The rank its download was last given (`fetch_priority`).
    sent: f32,
    /// Our own avatar's bake, or an image of the interface: in the
    /// fetcher's urgent lane.
    urgent: bool,
    dirty_cache: bool,
    /// Why the last fetch failed (status, error kind, host; never the URL).
    last_error: Option<String>,
    /// The failure was already logged at info level.
    error_logged: bool,
    /// The last fetch was answered 404: asked again much later.
    gone: bool,
    /// Listed in `TextureStreamer::todo`.
    queued: bool,
    /// What it adds to `TextureStreamer::counts` (`count` bits).
    counted: u8,
    /// "Refresh textures" asked: what is held of it is forgotten as soon as
    /// no read, decode or download is under way (`TextureStreamer::refresh`).
    refresh: bool,
    /// Bumped by each refresh: an upload decoded before it is dropped.
    epoch: u32,
}

/// `Entry::counted`.
mod count {
    pub const LOADED: u8 = 1;
    pub const FETCHING: u8 = 2;
    pub const FAILING: u8 = 4;
    pub const READING: u8 = 8;
}

/// How many textures are in each state of `count`, kept as they change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    loaded: usize,
    fetching: usize,
    failing: usize,
    reading: usize,
}

impl Counts {
    /// A texture went from the states `old` to the states `new`.
    fn change(&mut self, old: u8, new: u8) {
        for (bit, n) in [
            (count::LOADED, &mut self.loaded),
            (count::FETCHING, &mut self.fetching),
            (count::FAILING, &mut self.failing),
            (count::READING, &mut self.reading),
        ] {
            match (old & bit != 0, new & bit != 0) {
                (false, true) => *n += 1,
                (true, false) => *n = n.saturating_sub(1),
                _ => {}
            }
        }
    }
}

impl Entry {
    fn needs_sculpt_pixels(&self) -> bool {
        self.keep_pixels && self.sculpt.is_none()
    }

    /// GPU residency does not fulfill a late request for sculpt-map pixels.
    fn needs_decode(&self, discard: u8) -> bool {
        self.decoded.is_none_or(|decoded| discard < decoded) || self.needs_sculpt_pixels()
    }

    /// Pixels it covers on screen as far as known now: what `want` was
    /// computed from, or what was noted since if that is more (a texture
    /// just arrived has only that). 0: not seen yet.
    fn px_now(&self) -> f32 {
        self.seen_px.max(self.need_px).max(self.fixed_px)
    }

    /// A texture used for the first time has its size on screen noted in
    /// the frame its object is built (`Scene::fresh_usage`), before
    /// `recompute_wants` next comes by: its level follows from that size at
    /// once, for a file of unknown size as that pass would compute it,
    /// instead of the level of a new entry.
    fn want_before_the_first_pass(&mut self) {
        let px = self.need_px.max(self.fixed_px);
        if self.seen_px <= 0.0 && px > 0.0 && !self.keep_pixels {
            self.want = discard_for(
                full_size(self).map_or(512, |(w, h)| w.max(h)) as f32,
                px,
                self.info.map(|i| i.levels),
            );
        }
    }

    /// The file just told its size (first bytes downloaded, or read from
    /// the disk): `want` was computed for a texture of unknown size, taken
    /// as 512 pixels, and is computed again for this one. Without it a
    /// 1024 texture is decoded and downloaded one level finer than its size
    /// on screen asks, until `recompute_wants` next comes by: four times
    /// the pixels, four times the bytes.
    fn want_for_its_size(&mut self) {
        let px = self.px_now();
        if self.keep_pixels || px <= 0.0 {
            return;
        }
        if let Some(info) = self.info {
            self.want = discard_for(info.width.max(info.height) as f32, px, Some(info.levels));
        }
    }

    /// Nothing of it is shown, the bytes held do not reach the level
    /// wanted, but they hold a coarser one: its first pixels, to decode now
    /// while the rest is downloaded.
    fn holds_first_pixels(&self, needed: usize) -> bool {
        self.decoded.is_none()
            && !self.complete
            && self.data.len() < needed
            && self
                .info
                .is_some_and(|info| aurora_assets::bytes_for_discard(&info, aurora_assets::j2k::MAX_DISCARD_LEVEL) <= self.data.len())
    }

    /// `TextureStreamer::update` has something to do for it, now or once a
    /// delay, a decode slot or an upload has passed: its first disk read,
    /// or a finer level than the one on the GPU to read, decode or
    /// download. Not while nothing uses it, nor while a read, a decode or a
    /// download is under way: their result lists it again.
    fn has_work(&self) -> bool {
        if self.refresh {
            return !self.busy();
        }
        if self.refs == 0 {
            return false;
        }
        if self.cache_state == 0 {
            return true;
        }
        if self.busy() {
            return false;
        }
        self.needs_decode(self.want)
    }

    /// A disk read, a decode or a download of it is under way.
    fn busy(&self) -> bool {
        self.cache_state == 1 || self.decoding || self.fetching
    }

    /// Never loaded and not coming for now: missing on the asset server
    /// (asked again in ten minutes), or failing again and again.
    fn unavailable(&self) -> bool {
        self.decoded.is_none() && (self.gone || self.failures > 3)
    }

    /// Texture Refresh (destroy_texture, llviewermenu.cpp): what is held of
    /// it is forgotten and it is downloaded again, not read from the disk
    /// (its files are removed by the caller). The slot keeps showing its
    /// texels until the new ones replace them, where Firestorm shows the
    /// grey placeholder meanwhile. Returns the data it held, to free.
    fn forget(&mut self) -> Arc<Vec<u8>> {
        self.refresh = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.complete = false;
        self.info = None;
        self.decoded = None;
        // (2: the JPEG 2000 data is read, or not on the disk)
        self.cache_state = 2;
        self.transcoded = None;
        self.keep_rgba = false;
        self.failures = 0;
        self.retry_at = None;
        self.last_error = None;
        self.error_logged = false;
        self.gone = false;
        self.dirty_cache = false;
        self.asked_at = None;
        self.fetch_key = 0;
        std::mem::take(&mut self.data)
    }

    /// Its states for `Counts`.
    fn count_bits(&self) -> u8 {
        let mut bits = 0;
        if self.decoded.is_some() {
            bits |= count::LOADED;
        }
        if self.fetching {
            bits |= count::FETCHING;
        }
        if self.refs > 0 && self.decoded.is_none() && self.last_error.is_some() {
            bits |= count::FAILING;
        }
        if self.cache_state == 1 {
            bits |= count::READING;
        }
        bits
    }
}

/// AURORA_TEXTURE_TODO.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TodoMode {
    /// `update` visits the textures listed for it.
    Listed,
    /// It walks every texture, as before the list.
    Walk,
    /// It visits the listed ones; the list and the counters are compared
    /// with a walk of every texture now and then.
    Check,
}

fn todo_mode() -> TodoMode {
    static MODE: std::sync::OnceLock<TodoMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_TEXTURE_TODO").as_deref() {
        Ok("0") => TodoMode::Walk,
        Ok("check") => TodoMode::Check,
        _ => TodoMode::Listed,
    })
}

/// What decides the texture page of an upload: texel format, level-0
/// size and number of levels.
type ChainSize = (TexelFormat, u32, u32, u32);

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
    /// `Entry::epoch` when it was decoded: one decoded before a refresh is
    /// dropped.
    epoch: u32,
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

    /// Texel format, level-0 size and number of levels of the chain to
    /// upload (what decides its texture page).
    fn chain(&self) -> Option<ChainSize> {
        match &self.staged {
            Some(staged) => staged
                .levels
                .first()
                .map(|l| (staged.format, l.width, l.height, staged.levels.len() as u32)),
            None => self.mips.first().map(|m| (TexelFormat::Rgba8, m.0, m.1, self.mips.len() as u32)),
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
    chains: HashMap<ChainSize, u32>,
    seq: u64,
}

impl PendingUploads {
    fn len(&self) -> usize {
        self.heap.len() + self.unready.len()
    }

    /// An upload of `id` at `discard` or finer is waiting.
    fn covers(&self, id: &Uuid, discard: u8) -> bool {
        self.finest.get(id).is_some_and(|&f| f <= discard)
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
    fn queued(&self, chain: ChainSize) -> u32 {
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
    /// Listed for `update`: they have work, or wait for a delay, a decode
    /// slot or an upload.
    pub listed: usize,
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

/// A bake of our own avatar: its URL holds `/texture/<our agent>/` (the
/// appearance service's `texture/<avatar>/<bake>/<texture>`, `bake_source`).
fn own_bake(own_bakes: &str, source: &TexSource) -> bool {
    matches!(source, TexSource::Bake { url } if !own_bakes.is_empty() && url.contains(own_bakes))
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

/// Width of a texture's UI copy decoded at `discard` or coarser: no finer
/// than the level that fits 256 px (`texture_cache::preview_discard`).
fn ui_width(width: u32, height: u32, discard: u8) -> u32 {
    let level = texture_cache::preview_discard(width, height).max(discard);
    width.div_ceil(1 << level.min(31))
}

/// Bytes of a texture's mip chain at a discard level, about (a third more
/// than its level 0); 0 while its size is not known.
fn bytes_at(e: &Entry, discard: u8) -> u64 {
    full_size(e).map_or(0, |(w, h)| (w as u64 * h as u64 * 16 / 3) >> (2 * discard))
}

/// `Entry::slot` of a texture that could not have one (`acquire`, with the
/// table full of textures in use): the white texture, which its faces draw.
/// The renderer takes no texels for it.
const NO_SLOT: u32 = aurora_render::textures::WHITE;

/// What an evicted texture leaves to free off the main thread: its J2C
/// data, its sculpt map and its last error.
type Evicted = (Arc<Vec<u8>>, Option<Arc<SculptMap>>, Option<String>);

/// The slots of the renderer's texture table, as the streamer takes them
/// and gives them back (the tests stand in for the table).
trait SlotTable {
    /// A slot showing the grey placeholder, None when the table is full.
    fn placeholder(&mut self) -> Option<u32>;
    fn free(&mut self, slot: u32);
    /// The slot, while it shows the placeholder, shows the missing grey.
    fn show_missing(&mut self, slot: u32) -> bool;
}

impl SlotTable for Renderer {
    fn placeholder(&mut self) -> Option<u32> {
        self.textures.create_placeholder()
    }

    fn free(&mut self, slot: u32) {
        self.free_texture(slot);
    }

    fn show_missing(&mut self, slot: u32) -> bool {
        self.textures.show_missing(slot)
    }
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
    /// The textures `update` has something to do for (`Entry::has_work`),
    /// each listed once. It only visits these: walking every texture at
    /// each frame cost 0.14 to 0.17 ms for the 4,300 textures of a skybox
    /// at rest, and 0.26 to 0.45 ms with the 13,700 of a busy region, kept
    /// 45 s after leaving it. Every change to what `has_work` reads goes
    /// through `settle`.
    todo: Vec<Uuid>,
    /// Loaded, downloading, failing and being read: counted as the
    /// textures change, not by the walk.
    counts: Counts,
    /// AURORA_TEXTURE_TODO=check: updates since the session began.
    checks: u32,
    by_fetch_key: HashMap<u64, Uuid>,
    /// Downloads to rank again, or to drop (nothing uses the texture any
    /// more), told to the fetcher at the next `update`.
    moved: Vec<(u64, f32)>,
    dropped: Vec<u64>,
    /// `/texture/<our agent>/`: the path of our own bakes (`set_agent`).
    own_bakes: String,
    own_agent: Uuid,
    next_key: u64,
    uploads: PendingUploads,
    /// Staging memory of the renderer, given to the decode jobs.
    pub staging: Option<Arc<StagingPool>>,
    /// The decode jobs compress to BC7 the textures that come out of it
    /// unharmed (`aurora_render::bc7`): the GPU has BC compression and
    /// AURORA_TEXTURE_BC7 does not turn it off.
    pub bc7: bool,
    /// The BC7 chains are kept on the disk and read back instead of
    /// decoding the JPEG 2000 data again (`texture_cache.rs`).
    pub bc7_cache: bool,
    /// Alpha class by bindless slot (fast lookup during list building).
    pub alpha_by_slot: Vec<AlphaKind>,
    pub(super) pick_masks: HashMap<Uuid, PickMask>,
    /// Per slot: the decoded image has an alpha channel (LL sends such
    /// faces to the alpha pool when rigged or glowing, even if opaque).
    pub alpha_channel_by_slot: Vec<bool>,
    cache_dir: PathBuf,
    /// Procedural demo textures: id -> slot.
    local_slots: HashMap<Uuid, u32>,
    /// Worker-prepared local pixels, shared with avatar baking and UI previews.
    local_images: HashMap<Uuid, Arc<aurora_render::preview::Image>>,
    /// Bumped whenever a slot's alpha class or alpha channel, or a sculpt
    /// map, changes.
    pub generation: u64,
    /// Slots whose alpha class or alpha channel changed since the scene
    /// last took them: it classifies the faces using them again (their
    /// pass is cached, and the GPU lists read it).
    alpha_changes: Vec<u32>,
    /// Slots whose texture showed its first pixels since the last
    /// `take_first_texels` (the rez effect's objects waiting for them).
    first_texels: Vec<u32>,
    max_decodes: usize,
    decodes: usize,
    /// Memory pressure step (texture_budget.rs), decided in `maintain`.
    pressure: Pressure,
    /// The window in the background long enough to give its textures back
    /// (texture_budget::Background), and its state (`set_window_state`).
    background: texture_budget::Background,
    in_front: bool,
    minimized: bool,
    last_budget: Instant,
    /// Passes of `recompute_wants` since the step last changed.
    passes_at_step: u32,
    /// Loaded textures to lower to a discard level, listed by the last
    /// pass under pressure and done a few per frame, the largest (the
    /// bytes they hold, last of the tuple) first: taken from the end.
    reductions: Vec<(Uuid, u8, u64)>,
    /// Bytes of textures the reductions should give back, from the last
    /// budget reading (texture_budget::lowering_goal); no limit until the
    /// first one.
    to_lower: u64,
    /// Bytes the textures would hold one pressure step below, from the
    /// last pass.
    bytes_below: u64,
    /// The photo quality: the textures in view at full resolution.
    pub full_resolution: bool,
    /// Textures in view still loading, from the last pass.
    pending_in_view: usize,
    /// The last `update` had somewhere to download textures from.
    can_fetch: bool,
    /// Textures the demo draws itself, see [`DemoSource`].
    pub demo_source: Option<DemoSource>,
    /// Textures also wanted as egui images (profile pictures...): small CPU copies.
    ui_wanted: std::collections::HashSet<Uuid>,
    /// Session-long UI users, independent from the bounded hover cache.
    ui_permanent: std::collections::HashSet<Uuid>,
    ui_ready: HashMap<Uuid, (u32, u32, Vec<u8>)>,
    ui_previews: super::texture_ui::Images,
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
    /// Refreshed textures whose files the next `update` removes from the
    /// disk cache.
    forgotten: Vec<Uuid>,
    /// What evicted textures leave to free (`evict`), handed to the trash
    /// thread by `maintain`.
    evicted: Vec<Evicted>,
    /// Textures in use that have no slot (`acquire`, with the table full),
    /// and those the log was not told of yet (one line for all of them,
    /// `report_slotless`).
    slotless: usize,
    slotless_unreported: usize,
    /// Textures that just became unavailable (`is_unavailable`): `upload`
    /// points their slot at the missing grey, so that their faces leave the
    /// rez grid (`textures::MISSING`).
    unavailable: Vec<Uuid>,
    /// Decoded levels the uploads are done with and download bodies once
    /// merged, freed off this thread.
    trash: Trash<Vec<u8>>,
    /// Thread that writes the transcoded cache (started with the first
    /// file to write).
    cache_writer: Option<texture_cache::Writer>,
    /// Measure of the loading episodes (`texture load:` in the log): what
    /// the decode jobs spent, what the main thread counts, and the episode
    /// under way.
    meter: Arc<DecodeMeter>,
    load: LoadTotals,
    watch: LoadWatch,
}

/// Shared image sources for an isolated appearance preview, decoded on a worker.
pub struct PreviewSource {
    pub local: Option<Arc<aurora_render::preview::Image>>,
    pub bytes: Arc<Vec<u8>>,
    pub jpeg: PathBuf,
    pub blocks: PathBuf,
    pub decoded: Option<u8>,
    pub size: Option<(u32, u32)>,
    pub procedural: bool,
}
pub type PreviewRevision = (usize, Option<u8>, Option<(u32, u32)>, bool);
impl PreviewSource {
    /// Appearance editing can use resident pixels immediately and improve
    /// them as finer levels arrive; export still requires the original data.
    pub fn bake_ready(&self, id: Uuid) -> bool {
        self.local.is_some()
            || self.decoded.is_some()
            || self.procedural
            || is_builtin_transparent(&id)
            || id == aurora_prim::te::BLANK_TEXTURE
    }

    pub fn bake_revision(&self) -> PreviewRevision {
        let mut revision = self.revision();
        if self.local.is_none() {
            // Download packets do not change pixels until a new resident level
            // is ready. Invalidating for every packet would starve the worker.
            revision.0 = 0;
        }
        revision
    }

    pub fn bake_image(self, id: Uuid) -> Option<Arc<aurora_render::preview::Image>> {
        if is_builtin_transparent(&id) || id == aurora_prim::te::BLANK_TEXTURE {
            return Some(Arc::new(aurora_render::preview::Image {
                size: [1, 1],
                rgba: if is_builtin_transparent(&id) { vec![0; 4] } else { vec![255; 4] }.into(),
            }));
        }
        let blocks = self.blocks.clone();
        let discard = self.decoded.unwrap_or(0);
        self.inspector_image(id).or_else(|| {
            // A texture loaded from BC7 can be fully visible without any JPEG
            // bytes in memory or on disk. Decode its resident cache level on
            // the worker, preserving its resolution instead of the UI thumbnail.
            let texture_cache::Transcoded::Bc7(cached) = texture_cache::read(&blocks, discard)? else {
                return None;
            };
            let (width, height, blocks) = cached.levels.first()?;
            let mut rgba = aurora_render::bc7::decode_level(blocks, *width, *height)?;
            if !cached.alpha_channel || cached.alpha == AlphaKind::Opaque {
                // Opaque BC7 encoders may quantize alpha to 254: the world
                // ignores that channel, and a layer composite must do so too.
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel[3] = 255;
                }
            }
            Some(Arc::new(aurora_render::preview::Image {
                size: [*width, *height],
                rgba: rgba.into(),
            }))
        })
    }

    /// Inspector pixels at the resident resolution, decoded on a worker.
    /// Exports must wait for discard 0 instead of using the small BC7 preview.
    pub fn inspector_image(self, id: Uuid) -> Option<Arc<aurora_render::preview::Image>> {
        if let Some(image) = self.local {
            return Some(image);
        }
        if let Some((rgba, width, height)) = crate::demo::local_texture(&id) {
            return Some(Arc::new(aurora_render::preview::Image {
                size: [width, height],
                rgba: rgba.into(),
            }));
        }
        let discard = self.decoded?;
        let bytes = if self.bytes.is_empty() {
            Arc::new(std::fs::read(&self.jpeg).ok()?)
        } else {
            self.bytes
        };
        let pixels = aurora_assets::decode_j2k(&bytes, discard).ok()?;
        Some(Arc::new(aurora_render::preview::Image {
            size: [pixels.width, pixels.height],
            rgba: super::jobs::to_rgba(pixels.components, &pixels.data, (pixels.width * pixels.height) as usize).into(),
        }))
    }
    pub fn revision(&self) -> PreviewRevision {
        (
            self.local.as_ref().map_or(self.bytes.len(), |image| Arc::as_ptr(image) as usize),
            self.decoded,
            self.size,
            self.procedural,
        )
    }
    /// What `image` depends on: the level it decodes, never finer than
    /// preview_discard, instead of the exact bytes and level held. The
    /// texture budget lowers and raises the levels held all the time on a
    /// busy region; small previews built again at each move (the shape
    /// editor's hints) cost a hitch every few hundred milliseconds.
    pub fn image_revision(&self) -> PreviewRevision {
        if self.local.is_some() {
            return self.revision();
        }
        let Some((width, height)) = self.size.filter(|_| self.decoded.is_some()) else {
            return self.revision();
        };
        let level = texture_cache::preview_discard(width, height).max(self.decoded.unwrap_or(0));
        (0, Some(level), self.size, self.procedural)
    }
    pub fn image(self, id: Uuid) -> Option<Arc<aurora_render::preview::Image>> {
        if let Some(image) = self.local {
            return Some(image);
        }
        let image = |width, height, rgba: Vec<u8>| {
            Arc::new(aurora_render::preview::Image {
                size: [width, height],
                rgba: rgba.into(),
            })
        };
        // Reserved world textures have GPU texels but no fetched asset or
        // cache file. A missing transparent map would expose a white shell
        // over the skin of a layered mesh head.
        if is_builtin_transparent(&id) {
            return Some(image(1, 1, vec![0; 4]));
        }
        if id == aurora_prim::te::BLANK_TEXTURE {
            return Some(image(1, 1, vec![255; 4]));
        }
        if self.procedural
            && let Some((pixels, w, h)) = crate::demo::local_texture(&id)
        {
            return Some(image(w, h, pixels));
        }
        if let Some(texture_cache::Transcoded::Bc7(cached)) = texture_cache::read_preview(&self.blocks)
            && let Some((w, h, blocks)) = cached.levels.first()
            && let Some(rgba) = aurora_render::bc7::decode_level(blocks, *w, *h)
        {
            return Some(image(*w, *h, rgba));
        }
        let bytes = if self.bytes.is_empty() {
            // Progressive downloads may only have a .part cache. Their ready
            // coarse pixels are already visible in the main scene.
            std::fs::read(&self.jpeg)
                .or_else(|_| std::fs::read(self.jpeg.with_extension("part")))
                .ok()
                .map(Arc::new)?
        } else {
            self.bytes
        };
        let info = aurora_assets::j2k_info(&bytes)?;
        // A progressive stream may only hold the coarser level already
        // visible in the world. Never ask these bytes for a finer decode.
        let discard = texture_cache::preview_discard(info.width, info.height).max(self.decoded.unwrap_or(0));
        let pixels = aurora_assets::decode_j2k(&bytes, discard).ok()?;
        Some(image(
            pixels.width,
            pixels.height,
            super::jobs::to_rgba(pixels.components, &pixels.data, (pixels.width * pixels.height) as usize),
        ))
    }
}

impl TextureStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("tex"));
        TextureStreamer {
            // (room for a busy region, ~14,000 textures at BackBone: a
            // table of that size growing moves every entry in one frame)
            entries: HashMap::with_capacity(28_000),
            todo: Vec::new(),
            counts: Counts::default(),
            checks: 0,
            by_fetch_key: HashMap::new(),
            moved: Vec::new(),
            dropped: Vec::new(),
            own_bakes: String::new(),
            own_agent: Uuid::nil(),
            next_key: 1,
            uploads: PendingUploads::default(),
            staging: None,
            bc7: false,
            bc7_cache: false,
            pick_masks: [
                aurora_prim::te::TRANSPARENT_TEXTURE,
                super::avatar::IMG_INVISIBLE,
                aurora_prim::te::WATER_EXCLUSION_TEXTURE,
            ]
            .into_iter()
            .map(|id| {
                let mask = PickMask {
                    width: 1,
                    height: 1,
                    bits: vec![0; 2],
                };
                (id, mask)
            })
            .collect(),
            // built-in slots: the transparent texture has no visible pixel
            alpha_by_slot: {
                let mut a = vec![AlphaKind::Opaque; 16];
                a[aurora_render::textures::TRANSPARENT as usize] = AlphaKind::Mask;
                a
            },
            alpha_channel_by_slot: Vec::new(),
            cache_dir,
            local_slots: HashMap::new(),
            local_images: HashMap::new(),
            generation: 0,
            max_decodes: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).max(2),
            decodes: 0,
            pressure: Pressure::new(Instant::now()),
            background: texture_budget::Background::default(),
            in_front: true,
            minimized: false,
            last_budget: Instant::now(),
            passes_at_step: 0,
            reductions: Vec::new(),
            to_lower: u64::MAX,
            bytes_below: 0,
            full_resolution: false,
            pending_in_view: 0,
            can_fetch: false,
            demo_source: None,
            ui_wanted: Default::default(),
            ui_permanent: Default::default(),
            ui_ready: HashMap::new(),
            ui_previews: Default::default(),
            stats: TextureStats::default(),
            last_maintenance: Instant::now(),
            evict_after: UNUSED_EVICT,
            unused: VecDeque::new(),
            dirty: Vec::new(),
            forgotten: Vec::new(),
            evicted: Vec::new(),
            slotless: 0,
            slotless_unreported: 0,
            unavailable: Vec::new(),
            trash: Trash::default(),
            cache_writer: None,
            alpha_changes: Vec::new(),
            first_texels: Vec::new(),
            meter: Arc::default(),
            load: LoadTotals::default(),
            watch: LoadWatch::default(),
        }
    }

    /// The slots whose alpha class or alpha channel changed since the last
    /// call (a slot may be listed more than once).
    /// The slots whose texture showed its first pixels since the last
    /// call (appended to `out`).
    pub fn take_first_texels(&mut self, out: &mut Vec<u32>) {
        out.append(&mut self.first_texels);
    }

    pub fn take_alpha_changes(&mut self, out: &mut Vec<u32>) {
        out.append(&mut self.alpha_changes);
    }

    /// The same slots, left for the sync to take: what the hover search
    /// must not trust between two syncs (hover.rs).
    pub fn pending_alpha_changes(&self) -> &[u32] {
        &self.alpha_changes
    }

    /// The cache folder the textures' files are in (`tex/` below it).
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// The transcoded chain of a texture (`texture_cache.rs`).
    fn transcoded_path(cache_dir: &std::path::Path, id: &Uuid) -> PathBuf {
        cache_dir.join("tex").join(format!("{id}.bc7"))
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
        self.entries.get(id).and_then(full_size)
    }

    /// Share source bytes and cache locations, never GPU readback or disk IO
    /// on the frame thread. A cache-only load must also invalidate hints.
    pub fn preview_source(&self, id: &Uuid) -> PreviewSource {
        let entry = self.entries.get(id);
        PreviewSource {
            local: self.local_images.get(id).cloned(),
            bytes: entry.map(|e| e.data.clone()).unwrap_or_default(),
            jpeg: Self::cache_path(&self.cache_dir, id, true),
            blocks: Self::transcoded_path(&self.cache_dir, id),
            decoded: entry.and_then(|e| e.decoded),
            size: entry.and_then(|e| e.info).map(|i| (i.width, i.height)),
            procedural: self.local_slots.contains_key(id),
        }
    }

    /// The texture has see-through pixels (None until it is decoded): the
    /// build floater's "Alpha mode" is only offered then.
    pub fn has_alpha(&self, id: &Uuid) -> Option<bool> {
        if !self.is_loaded(id) {
            return None;
        }
        Some(self.entries.get(id)?.alpha != AlphaKind::Opaque)
    }

    pub fn describe(&self, id: &Uuid) -> String {
        match self.entries.get(id) {
            None if is_builtin_transparent(id) => "built-in transparent, never downloaded".into(),
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
        if id.is_nil() || *id == aurora_prim::te::BLANK_TEXTURE || is_builtin_transparent(id) || self.local_slots.contains_key(id) {
            return None;
        }
        let Some(e) = self.entries.get(id) else {
            return Some("texture not requested");
        };
        if e.slot != slot {
            return Some("drawn with another slot than the streamer's (stale record)");
        }
        if e.slot == NO_SLOT {
            return Some("no slot: the texture table was full when it was first used (drawn white)");
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

    /// After any change to an entry: its states in the counters, and its
    /// place on the list of `update` when it has work.
    fn settle(todo: &mut Vec<Uuid>, counts: &mut Counts, id: Uuid, e: &mut Entry) {
        let bits = e.count_bits();
        if bits != e.counted {
            counts.change(e.counted, bits);
            e.counted = bits;
        }
        if !e.queued && e.has_work() {
            e.queued = true;
            todo.push(id);
        }
    }

    /// AURORA_TEXTURE_TODO=check: every texture with work must be listed,
    /// and the counters must be those of a walk of every texture.
    fn verify(&self) {
        let mut walked = Counts::default();
        let mut reported = 0;
        for (id, e) in &self.entries {
            walked.change(0, e.count_bits());
            if e.has_work() && !e.queued && reported < 8 {
                reported += 1;
                log::warn!(
                    "texture todo check: {id} has work and is not listed (cache state {}, decoded {:?}, want {}, refs {})",
                    e.cache_state,
                    e.decoded,
                    e.want,
                    e.refs
                );
            }
        }
        if walked != self.counts {
            log::warn!("texture todo check: counters {:?}, a walk gives {walked:?}", self.counts);
        }
    }

    pub fn acquire(&mut self, renderer: &mut Renderer, id: Uuid, source: TexSource) -> u32 {
        if id.is_nil() {
            return aurora_render::textures::WHITE;
        }
        if is_builtin_transparent(&id) {
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
            Self::settle(&mut self.todo, &mut self.counts, id, e);
            return e.slot;
        }
        // procedural textures of the offline demo (never streamed)
        if let Some(slot) = self.local_slots.get(&id) {
            return *slot;
        }
        if let Some((px, w, h)) = crate::demo::local_texture(&id) {
            let alpha = super::jobs::classify_alpha(&px);
            let pick_mask = (alpha != AlphaKind::Opaque).then(|| PickMask::from_rgba(w, h, &px)).flatten();
            let levels = mips_for(w, h, px, if crate::demo::local_texture_mips(&id) { 16 } else { 1 }, alpha);
            let mips: Vec<MipLevel> = levels
                .iter()
                .map(|(width, height, data)| MipLevel {
                    width: *width,
                    height: *height,
                    data,
                })
                .collect();
            if let Some(slot) = renderer.create_texture(&mips) {
                set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, &mut self.alpha_changes, slot, alpha);
                if let Some(mask) = pick_mask {
                    self.pick_masks.insert(id, mask);
                }
                self.local_slots.insert(id, slot);
                return slot;
            }
        }
        // the shared grey placeholder until the first texels arrive: no
        // GPU work for a texture met for the first time
        let slot = self.new_slot(renderer);
        set_slot_alpha(
            &mut self.alpha_by_slot,
            &mut self.generation,
            &mut self.alpha_changes,
            slot,
            AlphaKind::Opaque,
        );
        let e = self.entries.entry(id).or_insert(Entry {
            slot,
            refs: 1,
            source,
            data: Arc::default(),
            complete: false,
            info: None,
            decoded: None,
            // Not seen yet: the coarsest level, which `recompute_wants`
            // gives whatever is out of view. The level of a new entry was a
            // quarter of its size, read again from the disk and decoded once
            // the coarsest was shown, or downloaded further, for every
            // texture of an arrival out of view: at a login on Agni, 2,208
            // of the 3,590 decodes of the first 10 s. What is in view has
            // its size noted before its first read
            // (`want_before_the_first_pass`).
            want: aurora_assets::j2k::MAX_DISCARD_LEVEL,
            need_px: 0.0,
            away_px: 0.0,
            fixed_px: 0.0,
            seen_px: 0.0,
            asked_at: None,
            gpu_size: (0, 0),
            fetching: false,
            decoding: false,
            cache_state: 0,
            transcoded: None,
            keep_rgba: false,
            failures: 0,
            retry_at: None,
            keep_pixels: false,
            sculpt: None,
            alpha: AlphaKind::Opaque,
            unused_since: None,
            fetch_key: 0,
            sent: 0.0,
            urgent: false,
            dirty_cache: false,
            last_error: None,
            error_logged: false,
            gone: false,
            queued: false,
            counted: 0,
            refresh: false,
            epoch: 0,
        });
        e.urgent = own_bake(&self.own_bakes, &e.source);
        Self::settle(&mut self.todo, &mut self.counts, id, e);
        slot
    }

    /// Our avatar: its bakes go in the fetcher's urgent lane, as Firestorm
    /// boosts them (`BOOST_AVATAR_BAKED_SELF`).
    pub fn set_agent(&mut self, agent: &Uuid) {
        if *agent != self.own_agent {
            self.own_agent = *agent;
            self.own_bakes = if agent.is_nil() {
                String::new()
            } else {
                format!("/texture/{agent}/")
            };
        }
    }

    /// Rank of a texture's download: its lane, then its size on screen.
    fn rank(e: &Entry) -> (Lane, f32) {
        if e.urgent {
            (Lane::Urgent, fetch_priority::urgent::OWN)
        } else {
            (Lane::Bulk, fetch_priority::texture(e.px_now(), e.decoded.is_some()))
        }
    }

    /// "Refresh textures" (destroy_texture, llviewermenu.cpp): the texture
    /// is downloaded again, and its files in the disk cache go. What it
    /// holds is forgotten by `update`, at once or, while a disk read, a
    /// decode or a download of it is under way, once that is over (its
    /// result would bring the old data back). Built-in textures (blank,
    /// transparent, those of the demo) are left alone, as Firestorm leaves
    /// IMG_DEFAULT and its default textures. Returns whether it was asked.
    pub fn refresh(&mut self, id: &Uuid) -> bool {
        if id.is_nil() || is_builtin_transparent(id) || *id == aurora_prim::te::BLANK_TEXTURE || self.local_slots.contains_key(id) {
            return false;
        }
        match self.entries.get_mut(id) {
            Some(e) => {
                e.refresh = true;
                Self::settle(&mut self.todo, &mut self.counts, *id, e);
            }
            // not held: only its files, as removeFromCache
            None => self.forgotten.push(*id),
        }
        true
    }

    /// The slot of a texture met for the first time. With the table full,
    /// the textures nothing uses any more give theirs back at once, oldest
    /// release first, instead of after `evict_after`: on arrival in a region
    /// they are those of the region just left, and a texture in use is
    /// never the one that goes without. [`NO_SLOT`], counted for the log,
    /// when every slot holds a texture in use.
    fn new_slot(&mut self, table: &mut impl SlotTable) -> u32 {
        if let Some(slot) = table.placeholder() {
            return slot;
        }
        let entries = &self.entries;
        let unused = eviction_round(&mut self.unused, Instant::now(), Duration::ZERO, |id, since| {
            evictable(entries, id, since)
        });
        self.evict(table, unused);
        table.placeholder().unwrap_or_else(|| {
            // (drawn white, and nothing decoded for it is sent: `upload`)
            self.slotless += 1;
            self.slotless_unreported += 1;
            NO_SLOT
        })
    }

    /// Forget evicted textures: their slots go back to the table, and what
    /// they held (J2C data, sculpt map) waits in `evicted` to be freed off
    /// this thread.
    fn evict(&mut self, table: &mut impl SlotTable, ids: Vec<Uuid>) {
        self.evicted.reserve(ids.len());
        for id in ids {
            let Some(e) = self.entries.remove(&id) else {
                continue;
            };
            self.counts.change(e.counted, 0);
            if let Some(mask) = self.pick_masks.remove(&id) {
                self.trash.push(mask.bits);
            }
            if e.slot == NO_SLOT {
                self.slotless = self.slotless.saturating_sub(1);
            } else {
                table.free(e.slot);
                set_slot_alpha(
                    &mut self.alpha_by_slot,
                    &mut self.generation,
                    &mut self.alpha_changes,
                    e.slot,
                    AlphaKind::Opaque,
                );
            }
            self.evicted.push((e.data, e.sculpt, e.last_error));
        }
    }

    /// The textures that became unavailable show the missing grey, as the
    /// Legacy Viewer binds its default image for a missing asset
    /// (LLViewerTexture::bindDefaultImage): with the rez effect on, a face
    /// still showing the loading grey is the grid, which would stay for as
    /// long as the texture is in use. One still unavailable only (it may
    /// have loaded since), in its own slot (an evicted one gave it back).
    fn show_unavailable(&mut self, table: &mut impl SlotTable) {
        for id in std::mem::take(&mut self.unavailable) {
            if let Some(e) = self.entries.get(&id).filter(|e| e.unavailable() && e.slot != NO_SLOT) {
                table.show_missing(e.slot);
            }
        }
    }

    /// One line of the log for the textures left without a slot since the
    /// last one (not one each: they come by the thousand).
    fn report_slotless(&mut self, capacity: u32) {
        let new = std::mem::take(&mut self.slotless_unreported);
        if new > 0 {
            log::warn!(
                "texture table full ({capacity} slots, all holding a texture in use): {new} more textures have none, {} in all; they are drawn white until nothing uses them and they are met again",
                self.slotless
            );
        }
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
        self.local_images.remove(id);
    }

    pub fn set_local_pixels(&mut self, id: Uuid, image: Arc<aurora_render::preview::Image>) {
        self.local_images.insert(id, image);
    }
    pub fn register_local_image(&mut self, id: Uuid, slot: u32, alpha: AlphaKind, has_alpha: bool) {
        set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, &mut self.alpha_changes, slot, alpha);
        if let Some(channel) = self.alpha_channel_by_slot.get_mut(slot as usize)
            && *channel != has_alpha
        {
            *channel = has_alpha;
            self.generation += 1;
            self.alpha_changes.push(slot);
        }
        self.local_slots.insert(id, slot);
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
            Self::settle(&mut self.todo, &mut self.counts, *id, e);
        }
    }

    /// Mark a texture as a sculpt map (pixels are kept for geometry).
    pub fn want_sculpt(&mut self, id: &Uuid) {
        if let Some(e) = self.entries.get_mut(id) {
            e.keep_pixels = true;
            e.want = 0;
            e.need_px = e.need_px.max(4096.0);
            Self::settle(&mut self.todo, &mut self.counts, *id, e);
        }
    }

    pub fn sculpt_map(&self, id: &Uuid) -> Option<Arc<SculptMap>> {
        self.entries.get(id).and_then(|e| e.sculpt.clone())
    }

    /// State only, for the explicitly selected avatar's attachment diagnostic.
    pub(super) fn sculpt_watch_status(&self, id: &Uuid) -> String {
        let Some(e) = self.entries.get(id) else {
            return format!("no-entry null={}", id.is_nil());
        };
        format!(
            "refs={} keep_pixels={} complete={} bytes={} info={:?} decoded={:?} want={} cache={} fetching={} decoding={} failures={} gone={} queued={}",
            e.refs,
            e.keep_pixels,
            e.complete,
            e.data.len(),
            e.info,
            e.decoded,
            e.want,
            e.cache_state,
            e.fetching,
            e.decoding,
            e.failures,
            e.gone,
            e.queued,
        )
    }

    /// Keep a texture resident and deliver a small RGBA copy for the UI.
    pub fn want_ui_image(&mut self, renderer: &mut Renderer, id: Uuid, jobs: &Jobs) {
        if !id.is_nil() {
            self.ui_permanent.insert(id);
        }
        self.want_transient_ui_image(renderer, id, jobs);
    }

    /// The inventory hover cache owns one bounded, releasable UI interest.
    /// All UI users share one streamer reference per asset; a permanent
    /// consumer can adopt an asset first requested by the hover cache.
    pub fn want_transient_ui_image(&mut self, renderer: &mut Renderer, id: Uuid, jobs: &Jobs) {
        if id.is_nil() {
            return;
        }
        if self.ui_wanted.insert(id) {
            // Reserved textures bypass asset streaming and therefore never
            // reach the upload callback that normally prepares UI images.
            if is_builtin_transparent(&id) || id == aurora_prim::te::BLANK_TEXTURE {
                self.ui_ready
                    .insert(id, (1, 1, if is_builtin_transparent(&id) { vec![0; 4] } else { vec![255; 4] }));
                return;
            }
            // procedural textures of the offline demo are never streamed
            if let Some((px, w, h)) = crate::demo::local_texture(&id) {
                self.ui_ready.insert(id, (w, h, px));
                return;
            }
            let _ = self.acquire(renderer, id, TexSource::Asset);
            if let Some(e) = self.entries.get_mut(&id) {
                e.urgent = true;
            }
        }
        if !self.ui_ready.contains_key(&id) {
            let source = self.preview_source(&id);
            self.ui_previews.request(id, source, jobs);
        }
        self.note_usage(&id, 256.0);
    }

    pub fn release_transient_ui_image(&mut self, id: &Uuid) {
        if !self.ui_permanent.contains(id) && self.ui_wanted.remove(id) {
            self.ui_ready.remove(id);
            self.release(id);
            if let Some(entry) = self.entries.get_mut(id)
                && entry.refs == 0
            {
                entry.urgent = false;
            }
        }
    }

    /// Decoded UI copy, once available (taken by the caller).
    pub fn take_ui_image(&mut self, id: &Uuid) -> Option<(u32, u32, Vec<u8>)> {
        self.ui_previews.poll(&mut self.ui_ready, &self.ui_wanted);
        self.ui_ready.remove(id)
    }

    /// Width of the finest UI copy of a texture (its full size halved until
    /// it fits 256 px), once its header is known.
    pub fn ui_image_width(&self, id: &Uuid) -> Option<u32> {
        let (width, height) = self.full_size(id)?;
        Some(ui_width(width, height, 0))
    }

    /// Keep loading a texture whose UI copy came out `have` px wide, from
    /// the first levels of a progressive download: a copy taken once and
    /// kept stays blurred, its texture no longer being asked for. The finer
    /// copy comes through `take_ui_image`, from the next upload, or from a
    /// worker when a finer level is already resident.
    pub fn refine_ui_image(&mut self, id: Uuid, have: u32, jobs: &Jobs) {
        self.note_usage(&id, 256.0);
        if self.ui_ready.contains_key(&id) {
            return;
        }
        let finer = self
            .entries
            .get(&id)
            .and_then(|e| Some(ui_width(e.info?.width, e.info?.height, e.decoded?)))
            .is_some_and(|width| width > have);
        if finer {
            let source = self.preview_source(&id);
            self.ui_previews.request(id, source, jobs);
        }
    }

    /// Something uses this texture (it is streamed). An entry nothing
    /// references any more only waits for its eviction: it is not fetched.
    pub fn is_wanted(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.refs > 0)
    }

    /// Never loaded and not coming for now: missing on the asset server
    /// (asked again in ten minutes), or failing again and again.
    pub fn is_unavailable(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(Entry::unavailable)
    }

    /// A face with this texture shows what it will show for now: loaded,
    /// not coming (`is_unavailable`), or drawn without a download (the
    /// blank and the built-in transparent textures, local ones), which
    /// never have an entry and so never count as loaded.
    pub fn is_settled(&self, id: &Uuid) -> bool {
        id.is_nil()
            || *id == aurora_prim::te::BLANK_TEXTURE
            || is_builtin_transparent(id)
            || self.local_slots.contains_key(id)
            || self.is_loaded(id)
            || self.is_unavailable(id)
    }

    /// Discard level on the GPU (None until the first upload).
    pub fn decoded_level(&self, id: &Uuid) -> Option<u8> {
        self.entries.get(id)?.decoded
    }

    pub fn is_loaded(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.decoded.is_some())
    }

    /// A procedural texture of the offline demo, resident as soon as acquired.
    pub fn is_local(&self, id: &Uuid) -> bool {
        self.local_slots.contains_key(id)
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
        // the photo quality: every texture in view at full resolution, while
        // the memory allows it (under pressure the budget decides, as always)
        let full = self.full_resolution && step == 0;
        let can_fetch = self.can_fetch;
        let TextureStreamer {
            entries,
            reductions,
            ui_wanted,
            todo,
            counts,
            by_fetch_key,
            moved,
            dropped: dropped_fetches,
            ..
        } = self;
        reductions.clear();
        let mut bytes_below = 0;
        let mut pending = 0;
        for (id, e) in entries.iter_mut() {
            if e.keep_pixels {
                e.want = 0;
                bytes_below += bytes_at(e, 0);
                Self::settle(todo, counts, *id, e);
                continue;
            }
            let full_px = full_size(e).map_or(512, |(w, h)| w.max(h)) as f32;
            let levels = e.info.map(|i| i.levels);
            let (want, keep) = wanted(full_px, levels, e, dropped);
            let in_view = e.need_px > 0.0 || e.fixed_px > 0.0;
            e.want = if full && in_view && e.source == TexSource::Asset { 0 } else { want };
            // (still on its way: there is somewhere to download it from, or
            // what is held of it has more to give)
            let reachable = can_fetch || e.fetching || e.decoding || !e.data.is_empty() || e.cache_state < 2;
            if in_view
                && e.refs > 0
                && e.source == TexSource::Asset
                && e.decoded.is_none_or(|d| d > e.want)
                && !e.gone
                && e.failures <= 3
                && reachable
            {
                pending += 1;
            }
            e.seen_px = e.fixed_px.max(e.need_px / dropped.0);
            if step > 0 {
                let (want_below, keep_below) = wanted(full_px, levels, e, below);
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
            // A download waiting for a slot follows what is seen now: it
            // moves with its size on screen, and goes when nothing uses the
            // texture any more (a region left, an object gone). Firestorm
            // drops every texture request at a teleport
            // (LLViewerTextureList::updateImages); one already sent ends.
            if e.fetching && !e.urgent && e.fetch_key != 0 {
                if e.refs == 0 {
                    by_fetch_key.remove(&e.fetch_key);
                    dropped_fetches.push(e.fetch_key);
                    e.fetch_key = 0;
                    e.fetching = false;
                } else {
                    let (_, now) = Self::rank(e);
                    if fetch_priority::changed(e.sent, now) {
                        e.sent = now;
                        moved.push((e.fetch_key, now));
                    }
                }
            }
            // decay so textures that shrank on screen can be lowered later
            e.need_px *= 0.5;
            e.away_px *= 0.5;
            e.fixed_px *= 0.5;
            // (a finer level wanted: `update` hears of it)
            Self::settle(todo, counts, *id, e);
        }
        // (memory comes back soonest from the largest; the id settles ties,
        // the table has no order of its own)
        reductions.sort_unstable_by_key(|&(id, _, bytes)| (bytes, id));
        // only what brings the memory back under its goal, the rest kept
        // as it is: lowering every texture finer than its size on screen
        // gave back gigabytes for a few megabytes over, loaded again as
        // soon as the camera moved (texture_budget.rs)
        let mut freed = 0u64;
        let mut first = reductions.len();
        while first > 0 && freed < self.to_lower {
            first -= 1;
            let (id, keep, held) = reductions[first];
            let levels = entries.get(&id).and_then(|e| e.decoded).map_or(0, |d| keep.saturating_sub(d));
            freed += held - (held >> (2 * levels as u32).min(63));
        }
        reductions.drain(..first);
        self.bytes_below = bytes_below;
        self.pending_in_view = pending;
        self.passes_at_step = self.passes_at_step.saturating_add(1);
    }

    /// Textures in view still loading towards the level they want (the
    /// photo quality waits for them, `app::snapshot`).
    pub fn pending_in_view(&self) -> usize {
        self.pending_in_view
    }

    fn bytes_needed(e: &Entry) -> usize {
        match &e.info {
            // full resolution: the whole file (LLTextureFetch::createRequest),
            // the size estimate would miss the last quality layers
            Some(_) if e.want == 0 => usize::MAX,
            Some(info) => aurora_assets::bytes_for_discard(info, e.want),
            None => first_bytes(e.px_now()),
        }
    }

    fn url_for(&self, id: &Uuid, e: &Entry, viewer_asset: Option<&str>) -> Option<String> {
        match &e.source {
            TexSource::Bake { url } => Some(url.clone()),
            TexSource::Asset => viewer_asset.map(|base| asset_url(base, "texture_id", id)),
        }
    }

    /// Drive fetches and decodes, for the textures that have work (`todo`):
    /// the others are not visited. While most of them have some (a region
    /// arriving), the table is walked in its own order instead, without a
    /// lookup each. A texture is visited at each frame while it waits for a
    /// retry delay, a decode slot or its upload, as it was before the list.
    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        self.can_fetch = viewer_asset.is_some();
        for key in self.dropped.drain(..) {
            fetcher.cancel(key);
        }
        for (key, priority) in self.moved.drain(..) {
            fetcher.reprioritise(key, priority);
        }
        let now = Instant::now();
        let mode = todo_mode();
        if mode == TodoMode::Check {
            self.checks = self.checks.wrapping_add(1);
            // (a thirtieth of the frames: the check walks every texture)
            if self.checks.is_multiple_of(30) {
                self.verify();
            }
        }
        let TextureStreamer {
            entries,
            by_fetch_key,
            next_key,
            cache_dir,
            decodes,
            max_decodes,
            staging,
            meter,
            load,
            uploads,
            bc7,
            bc7_cache,
            ui_wanted,
            cache_writer,
            demo_source,
            todo,
            counts,
            forgotten,
            trash,
            ..
        } = self;
        if *bc7_cache && cache_writer.is_none() {
            *cache_writer = texture_cache::Writer::start();
            // (without its thread, the cache is only read)
        }
        let (bc7, bc7_cache) = (*bc7, *bc7_cache);
        // first disk reads of the new textures, queued in one go
        let mut cache_reads = Vec::new();
        // what waits for a decode slot (rank, texture, level)
        let mut to_decode: Vec<(UploadPriority, Uuid, u8)> = Vec::new();
        {
            let mut visit = |id: Uuid, e: &mut Entry| {
                if e.refresh {
                    // (listed only once nothing of it is under way)
                    if e.busy() {
                        return;
                    }
                    if let Ok(old) = Arc::try_unwrap(e.forget())
                        && !old.is_empty()
                    {
                        trash.push(old);
                    }
                    forgotten.push(id);
                }
                if e.refs == 0 {
                    return;
                }
                if e.retry_at.is_some_and(|t| now < t) {
                    return;
                }
                // 1. disk cache
                // (a texture whose pixels are kept, or copied for the
                // interface, needs a decode: the transcoded cache has blocks)
                let blocks_do = bc7_cache && !e.keep_pixels && !ui_wanted.contains(&id);
                if e.cache_state == 0 {
                    e.cache_state = 1;
                    e.want_before_the_first_pass();
                    // its size on screen is not known yet: the coarsest level of
                    // a transcoded chain shows it at once, and the level it
                    // needs is read when known (the default level would load
                    // every texture of a region at a quarter of its size)
                    let level = if e.need_px > 0.0 {
                        e.want
                    } else {
                        aurora_assets::j2k::MAX_DISCARD_LEVEL
                    };
                    cache_reads.push((id, level, blocks_do));
                    return;
                }
                if e.cache_state == 1 || e.decoding || e.fetching {
                    return;
                }
                // (before looking at the data, which is behind a pointer: most
                // textures stop here, every frame)
                let better = e.needs_decode(e.want);
                if !better {
                    return;
                }
                if e.cache_state == 3 {
                    // shown from the transcoded cache and another level is
                    // wanted: from that file again if it holds it, else the
                    // JPEG 2000 data is read at last, for a decode
                    let from_file = blocks_do && e.transcoded.is_some_and(|finest| e.want >= finest);
                    if from_file && uploads.covers(&id, e.want) {
                        return;
                    }
                    e.cache_state = 1;
                    cache_reads.push((id, e.want, from_file));
                    return;
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
                    return;
                }
                let needed = Self::bytes_needed(e);
                let have_enough = !e.data.is_empty() && (e.complete || e.data.len() >= needed);
                let after_failure = e.info.is_some() && e.decoded.is_none() && !e.data.is_empty() && e.failures > 0;
                // First pixels in one round trip: what the first reply holds
                // is decoded while the range of the wanted level is asked
                // for, instead of showing nothing until that second reply
                // (as Firestorm does). One more decode, of a small level.
                let first_pixels = !have_enough && !after_failure && e.holds_first_pixels(needed);
                if have_enough || after_failure || first_pixels {
                    'decode: {
                        // Decode at the best level the data allows.
                        let discard = match &e.info {
                            Some(info) if !e.complete => level_held(info, e.data.len(), e.want),
                            _ => e.want,
                        };
                        if !e.needs_decode(discard) {
                            break 'decode;
                        }
                        // That level, or a finer one, is already decoded and waits
                        // for its turn to the GPU (`decoded` only follows once it is
                        // sent): decoding it again each time a decode slot frees up
                        // was half of the decodes of an arrival (AURORA_DEMO_STREAM
                        // with j2k=, 9000 textures: 9126 of 18129).
                        if !e.needs_sculpt_pixels() && uploads.covers(&id, discard) {
                            break 'decode;
                        }
                        // (given a slot below, once every texture was seen)
                        let rank = UploadPriority {
                            first: e.decoded.is_none(),
                            pixels: e.px_now(),
                            seq: 0,
                        };
                        to_decode.push((rank, id, discard));
                    }
                    if !first_pixels {
                        return;
                    }
                }
                // 2. fetch more
                let Some(url) = (match &e.source {
                    TexSource::Bake { url } => Some(url.clone()),
                    TexSource::Asset => viewer_asset.map(|base| asset_url(base, "texture_id", &id)),
                }) else {
                    return;
                };
                let range = next_range(e.data.len(), needed);
                let key = FETCH_KIND_TEXTURE | *next_key;
                *next_key += 1;
                e.fetch_key = key;
                e.fetching = true;
                if e.decoded.is_none() && e.data.is_empty() && e.asked_at.is_none() {
                    e.asked_at = Some(now);
                    load.first_blind += u64::from(e.px_now() <= 0.0);
                }
                let (lane, priority) = Self::rank(e);
                e.sent = priority;
                by_fetch_key.insert(key, id);
                fetcher.request(FetchRequest {
                    key,
                    url,
                    range: Some(range),
                    priority,
                    lane,
                    accept: "image/x-j2c",
                });
            };
            let walk_all = mode == TodoMode::Walk;
            let mut listed = std::mem::take(todo);
            if walk_all || listed.len() * 2 > entries.len() {
                listed.clear();
                for (&id, e) in entries.iter_mut() {
                    if !(walk_all || e.queued) {
                        continue;
                    }
                    e.queued = false;
                    visit(id, e);
                    Self::settle(todo, counts, id, e);
                }
            } else {
                for id in listed.drain(..) {
                    let Some(e) = entries.get_mut(&id) else {
                        continue;
                    };
                    e.queued = false;
                    visit(id, e);
                    Self::settle(todo, counts, id, e);
                }
            }
            // (the emptied list keeps its room when nothing was listed again)
            if todo.is_empty() {
                *todo = listed;
            }
        }
        // Decode slots go to what shows nothing yet, then to the largest on
        // screen, as the uploads to the GPU do (`UploadPriority`). They went
        // in the order of the work list, so on an arrival a texture out of
        // view could take the slot of the one in front of the camera.
        let free = max_decodes.saturating_sub(*decodes);
        if to_decode.len() > free {
            to_decode.sort_unstable_by_key(|d| std::cmp::Reverse(d.0));
            to_decode.truncate(free);
        }
        for (_, id, discard) in to_decode {
            let Some(e) = entries.get_mut(&id) else {
                continue;
            };
            e.decoding = true;
            *decodes += 1;
            load.again += u64::from(e.decoded.is_some());
            let data = e.data.clone();
            let job = DecodeJob {
                id,
                discard,
                keep_pixels: e.keep_pixels,
                bc7: bc7 && !e.keep_rgba,
                // (not over a file that already holds this level)
                cache: e
                    .info
                    .filter(|_| bc7_cache && !e.keep_rgba && !e.transcoded.is_some_and(|finest| finest <= discard))
                    .zip(cache_writer.clone())
                    .map(|(info, writer)| (Self::transcoded_path(cache_dir, &id), info, writer)),
            };
            let pool = staging.clone();
            let meter = meter.clone();
            jobs.spawn(move || decode_job(job, &data, pool.as_deref(), &meter));
            Self::settle(todo, counts, id, e);
        }
        let dir = cache_dir.clone();
        let pool = staging.clone();
        jobs.spawn_many(cache_reads, move |(id, want, blocks)| {
            read_job(&dir, id, want, blocks, pool.as_deref())
        });
        if !forgotten.is_empty() {
            log::info!("refreshing {} textures: downloaded again", forgotten.len());
            let dir = cache_dir.clone();
            jobs.spawn_many(std::mem::take(forgotten), move |id| {
                remove_cached(&dir, &id);
                JobResult::Done
            });
        }
        self.stats.total = self.entries.len();
        let Counts {
            loaded,
            fetching,
            failing,
            reading,
        } = self.counts;
        self.stats.loaded = loaded;
        self.stats.fetching = fetching;
        self.stats.failing = failing;
        self.stats.listed = self.todo.len();
        self.stats.decoding = self.decodes;
        self.stats.pending_upload = self.uploads.len();
        let _ = Self::url_for;
        let busy = reading > 0 || fetching > 0 || self.decodes > 0 || self.uploads.len() > 0;
        // (totals: the main thread's counters and what the jobs have spent so far)
        let (counted, meter) = (self.load, &self.meter);
        let totals = || {
            let mut totals = counted;
            meter.read_into(&mut totals);
            totals
        };
        if let Some(r) = self.watch.step(now, busy, totals) {
            log::info!("{}", r.added.describe(r.took, !r.done, (loaded, self.entries.len())));
        }
    }

    /// Arrival in a region (login, teleport, crossing by teleport): the
    /// loading episode under way ends there, so the next `texture load:`
    /// line counts the new region alone.
    pub fn mark_arrival(&mut self) {
        let (counted, meter) = (self.load, &self.meter);
        let totals = || {
            let mut totals = counted;
            meter.read_into(&mut totals);
            totals
        };
        if let Some(r) = self.watch.cut(Instant::now(), totals) {
            log::info!(
                "{} (cut by an arrival)",
                r.added.describe(r.took, false, (self.stats.loaded, self.stats.total))
            );
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult) {
        let Some(id) = self.by_fetch_key.remove(&r.key) else {
            return;
        };
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        e.fetching = false;
        if let Ok(bytes) = &r.data {
            self.load.fetched_bytes += bytes.len() as u64;
            self.load.replies += 1;
        }
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
                    e.want_for_its_size();
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
                let was_unavailable = e.unavailable();
                e.failures += 1;
                let backoff = 2u64.pow(e.failures.min(6));
                e.retry_at = Some(Instant::now() + Duration::from_secs(backoff));
                // the asset CDN answers 403 for a missing asset (S3), as for
                // meshes; LLTextureFetchWorker gives up on it as on any
                // failed GET (WAIT_HTTP_REQ, "Fail harder")
                e.gone = matches!(r.status, 403 | 404);
                if e.gone {
                    e.retry_at = Some(Instant::now() + Duration::from_secs(600));
                }
                if !was_unavailable && e.unavailable() {
                    self.unavailable.push(id);
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
        Self::settle(&mut self.todo, &mut self.counts, id, e);
    }

    /// The texture disk cache since the start: JPEG 2000 data found and
    /// missed, and textures read back as BC7 without a decode.
    pub fn cache_counts(&self) -> [u64; 3] {
        [self.load.cache_hits, self.load.cache_misses, self.load.transcoded]
    }

    pub fn on_job(&mut self, r: JobResult) {
        match r {
            JobResult::TextureTranscoded { id, info, finest, texture } => {
                self.load.transcoded += 1;
                if let Some(e) = self.entries.get_mut(&id) {
                    e.cache_state = 3;
                    e.transcoded = Some(finest);
                    e.info.get_or_insert(info);
                    Self::settle(&mut self.todo, &mut self.counts, id, e);
                }
                // (queued as the result of a decode; it took no decode slot,
                // and the handler below gives one back)
                self.decodes += 1;
                self.on_job(*texture);
            }
            JobResult::TextureCache {
                id,
                data,
                complete,
                keep_rgba,
            } => {
                if data.is_some() {
                    self.load.cache_hits += 1;
                } else {
                    self.load.cache_misses += 1;
                }
                if let Some(e) = self.entries.get_mut(&id) {
                    e.cache_state = 2;
                    e.keep_rgba |= keep_rgba;
                    if let Some(d) = data
                        && d.len() > e.data.len()
                    {
                        e.info = aurora_assets::j2k_info(&d);
                        e.data = Arc::new(d);
                        e.complete = complete;
                        e.want_for_its_size();
                    }
                    Self::settle(&mut self.todo, &mut self.counts, id, e);
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
                        epoch: e.epoch,
                        #[cfg(test)]
                        held: false,
                    },
                    e.decoded.is_none(),
                    e.need_px.max(e.fixed_px),
                );
                Self::settle(&mut self.todo, &mut self.counts, id, e);
            }
            JobResult::TextureFailed { id } => {
                self.decodes = self.decodes.saturating_sub(1);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.decoding = false;
                    let was_unavailable = e.unavailable();
                    e.failures += 1;
                    if !was_unavailable && e.unavailable() {
                        self.unavailable.push(id);
                    }
                    // Corrupt cached data: drop it and refetch.
                    if e.failures >= 2 {
                        e.data = Arc::default();
                        e.complete = false;
                        e.info = None;
                        let _ = std::fs::remove_file(self.cache_dir.join("tex").join(format!("{id}.j2c")));
                        let _ = std::fs::remove_file(self.cache_dir.join("tex").join(format!("{id}.part")));
                    }
                    e.retry_at = Some(Instant::now() + Duration::from_secs(2u64.pow(e.failures.min(6))));
                    Self::settle(&mut self.todo, &mut self.counts, id, e);
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

    /// AURORA_DEMO_STREAM with `j2k=`: the texture is in use, the disk
    /// cache was looked up and it has no JPEG 2000 data (nothing there, or
    /// its transcoded chain does not hold the level wanted): on the grid, a
    /// download would start; the scenario brings the file instead.
    pub fn demo_needs_data(&self, id: &Uuid) -> bool {
        self.entries
            .get(id)
            .is_some_and(|e| e.refs > 0 && e.cache_state == 2 && e.data.is_empty() && !e.decoding)
    }

    /// AURORA_DEMO_SCULPT_MISSING: the asset server has no such texture, as
    /// a 404 answers. Returns whether it was not known missing already.
    pub fn demo_missing(&mut self, id: &Uuid) -> bool {
        let Some(e) = self.entries.get_mut(id).filter(|e| !e.gone) else {
            return false;
        };
        if !e.unavailable() {
            self.unavailable.push(*id);
        }
        e.gone = true;
        e.retry_at = Some(Instant::now() + Duration::from_secs(600));
        Self::settle(&mut self.todo, &mut self.counts, *id, e);
        true
    }

    /// For the tests of its users: a texture the asset server does not have.
    #[cfg(test)]
    pub(super) fn test_missing(&mut self, id: Uuid) {
        let mut e = tests::entry(None, 0.0);
        e.gone = true;
        self.entries.insert(id, e);
    }

    /// For the tests of its users: the texture's sculpt map decoded.
    #[cfg(test)]
    pub(super) fn test_sculpt_map(&mut self, id: Uuid) {
        let e = self.entries.entry(id).or_insert_with(|| tests::entry(None, 0.0));
        e.decoded = Some(0);
        e.gone = false;
        e.sculpt = Some(Arc::new(SculptMap {
            width: 1,
            height: 1,
            components: 3,
            pixels: vec![128; 3],
        }));
    }

    /// AURORA_DEMO_SCULPT_MISSING=late: the texture's file is in the disk
    /// cache now; it is read again, as a retry would find it on the server.
    pub fn demo_reread(&mut self, id: &Uuid) {
        if let Some(e) = self.entries.get_mut(id) {
            drop(e.forget());
            e.cache_state = 0;
            Self::settle(&mut self.todo, &mut self.counts, *id, e);
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
        self.show_unavailable(renderer);
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
            if u.epoch != e.epoch {
                // decoded from what a refresh forgot (its level no longer
                // waits: `update` may decode the new data at it)
                let id = u.id;
                u.retire(&mut self.trash);
                Self::settle(&mut self.todo, &mut self.counts, id, e);
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
            // (a texture without a slot has nowhere to go, and asks for no page)
            let placed = e.slot != NO_SLOT;
            let queued = &self.uploads;
            if placed
                && u.chain()
                    .is_some_and(|chain| !renderer.textures.prepare_page(e.slot, chain, queued.queued(chain)))
            {
                waiting.push(u);
                if waiting.len() >= PAGE_WAITS {
                    // (thousands may wait for that same page)
                    break;
                }
                continue;
            }
            let (accepted, level0, levels) = match &u.staged {
                _ if !placed => (false, None, 0),
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
            done += usize::from(placed);
            if !accepted {
                // keep the current texels, and do not decode it again in a loop
                if placed {
                    log::warn!(
                        "texture {}: upload refused by the renderer (slot {}, {levels} levels, level 0 {level0:?}, staged {})",
                        u.id,
                        e.slot,
                        u.staged.is_some(),
                    );
                    e.failures += 1;
                }
                // (without a slot it is loaded as far as the streamer goes:
                // what waits for it, a sculpt map or an avatar's bakes, does
                // not wait for ever; `report_slotless` told the log)
                e.decoded = Some(u.discard);
                Self::settle(&mut self.todo, &mut self.counts, u.id, e);
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
            if let Some(asked) = e.asked_at.take()
                && e.decoded.is_none()
            {
                self.load.first_pixels(asked.elapsed(), e.px_now());
            }
            if e.decoded.is_none() {
                self.first_texels.push(e.slot);
            }
            e.decoded = Some(u.discard);
            e.alpha = u.alpha;
            // (a finer level may still be wanted: `update` hears of it)
            Self::settle(&mut self.todo, &mut self.counts, u.id, e);
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
        let evict = eviction_round(&mut self.unused, now, evict_after, |id, since| evictable(entries, id, since));
        self.evict(renderer, evict);
        if !self.evicted.is_empty() {
            // the J2C data and sculpt maps go with the entries (those a
            // full table took since the last frame too): freed off this
            // thread
            jobs.discard(std::mem::take(&mut self.evicted));
        }
        if now.duration_since(self.last_maintenance) < HOUSEKEEPING_EVERY {
            return;
        }
        self.last_maintenance = now;
        self.report_slotless(renderer.textures.capacity());
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
            JobResult::Done
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
                Self::settle(&mut self.todo, &mut self.counts, id, e);
                self.stats.reduced += 1;
                done += 1;
            }
        }
    }

    /// The window's state, each frame: whether it has the focus, whether it
    /// is minimised (`press` gives the textures back behind,
    /// texture_budget::Background).
    pub fn set_window_state(&mut self, in_front: bool, minimized: bool) {
        self.in_front = in_front;
        self.minimized = minimized;
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
        // a window behind for a minute or minimised for a second gives its
        // textures back, at once while the memory is short, and takes up
        // again once in front (LLViewerTexture::updateClass,
        // texture_budget::Background)
        let crowded = video.is_some_and(|v| v.usage > v.budget);
        let away = self.background.update(now, self.in_front, self.minimized, crowded);
        if away && !self.pressure.held() {
            self.pressure.hold_max(now);
            let place = if self.minimized { "minimised" } else { "in the background" };
            if crowded {
                log::info!("texture memory: the window is {place} with the video memory short, freeing up video memory");
            } else {
                let after = if self.minimized {
                    texture_budget::MINIMIZED_AFTER
                } else {
                    texture_budget::BACKGROUNDED_AFTER
                };
                log::info!(
                    "texture memory: the window was {place} for {} s, freeing up video memory",
                    after.as_secs()
                );
            }
        } else if !away && self.pressure.held() {
            self.pressure.release(now);
            log::info!("texture memory: the window is in front again, textures load as before");
        }
        // what the pages would hold one step below, from step 1 too
        // (texture_budget::below_estimate)
        let below = texture_budget::below_estimate(self.bytes_below, used, live);
        let step = self.pressure.update(
            now,
            texture_budget::Reading {
                used,
                target,
                settled: self.passes_at_step >= 2 && self.reductions.is_empty() && !renderer.textures.settling(),
                below,
            },
        );
        // (behind: everything the highest step allows, not only what brings
        // the pages under the target)
        self.to_lower = if self.pressure.held() {
            u64::MAX
        } else {
            texture_budget::lowering_goal(used, live, target)
        };
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

/// What became of the texture `id`, queued as unused when it was released
/// at `since`.
fn evictable(entries: &HashMap<Uuid, Entry>, id: &Uuid, since: Instant) -> Evictable {
    match entries.get(id) {
        Some(e) if e.refs == 0 && e.unused_since == Some(since) => {
            if e.fetching || e.decoding {
                Evictable::Busy
            } else {
                Evictable::Yes
            }
        }
        // used again since (a later release queues it again), or gone
        _ => Evictable::No,
    }
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
    mips: Vec<(u32, u32, Vec<u8>)>,
    alpha: (AlphaKind, bool),
    sculpt: Option<Arc<SculptMap>>,
    pool: Option<&StagingPool>,
) -> JobResult {
    texture_result_from(id, discard, mips, None, alpha, sculpt, pool, None)
}

/// [`texture_result`], staging the BC7 blocks of the chain (`compressed`)
/// instead of its RGBA8 levels when the job has them. Without room in the
/// staging memory, the RGBA8 chain stays for the main-thread write, as an
/// uncompressed texture.
/// A decoded level under this size says nothing of how BC7 treats the
/// larger ones: its refusal is not written to the transcoded cache.
const KEEP_RGBA_FROM: u32 = 128;

/// The mips of a texture whose alpha is a mask keep what the alpha test
/// draws at every level (`aurora_render::preserve_alpha_coverage`), so that
/// foliage, grass, fences and hair cut out of a texture do not thin out with
/// the distance. The Legacy Viewer leaves the box-filtered mips of the
/// driver (LLImageGL), which do; this is a deliberate improvement.
/// `AURORA_ALPHA_COVERAGE=0` keeps the plain mips, to compare.
pub(crate) fn alpha_coverage_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_ALPHA_COVERAGE").map_or(true, |v| v.trim() != "0"))
}

/// `AURORA_MIP_FILTER=weighted`: the colour of the mips averaged by the
/// alpha, as before (`aurora_render::build_mips_alpha_weighted`), to compare
/// with the Legacy Viewer's plain average. Clear the transcoded cache
/// (`<cache>/tex/*.bc7`) between the two, or it serves the mips of the other.
fn mips_alpha_weighted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("AURORA_MIP_FILTER").is_ok_and(|v| v.trim() == "weighted"))
}

/// The mip chain of a decoded texture, with its alpha coverage kept when
/// its alpha is a mask. The texture does not know the cutoff of the faces
/// that will use it: the default one (0.5) is taken, the cutoff of every
/// face whose alpha is detected as a mask and of most masked materials.
fn mips_for(width: u32, height: u32, rgba: Vec<u8>, levels: u32, alpha: AlphaKind) -> Vec<(u32, u32, Vec<u8>)> {
    let mut mips = if mips_alpha_weighted() {
        aurora_render::build_mips_alpha_weighted(width, height, rgba, levels)
    } else {
        build_mips(width, height, rgba, levels)
    };
    if alpha == AlphaKind::Mask && alpha_coverage_on() {
        aurora_render::preserve_alpha_coverage(&mut mips, aurora_render::MASK_CUTOFF);
    }
    mips
}

/// Where a decode job leaves its result in the transcoded cache.
struct CacheWrite<'a> {
    path: &'a std::path::Path,
    writer: &'a texture_cache::Writer,
    info: J2kInfo,
    /// BC7 was tried: no chain means it would harm the texture.
    tried: bool,
}

fn texture_result_from(
    id: Uuid,
    discard: u8,
    mut mips: Vec<(u32, u32, Vec<u8>)>,
    compressed: Option<aurora_render::bc7::Chain>,
    (alpha, alpha_channel): (AlphaKind, bool),
    sculpt: Option<Arc<SculptMap>>,
    pool: Option<&StagingPool>,
    cache: Option<CacheWrite>,
) -> JobResult {
    // Generated on the decode worker, before staging may release CPU levels.
    let pick_mask = (alpha != AlphaKind::Opaque)
        .then(|| mips.first().and_then(|(w, h, rgba)| PickMask::from_rgba(*w, *h, rgba)))
        .flatten();
    if let Some(cache) = cache {
        let meta = texture_cache::Meta {
            info: cache.info,
            discard,
            alpha,
            alpha_channel,
        };
        let file = match &compressed {
            Some(chain) => texture_cache::encode(&meta, pick_mask.as_ref().map(PickMask::parts), chain),
            // BC7 was tried and harms it: remembered, unless the level is
            // too small to tell for the larger ones
            None => mips
                .first()
                .filter(|(w, h, _)| cache.tried && TexelFormat::Bc7.fits(*w, *h) && (*w).max(*h) >= KEEP_RGBA_FROM)
                .map(|_| texture_cache::encode_keep_rgba(&meta)),
        };
        if let Some(file) = file {
            cache.writer.write(cache.path.to_path_buf(), file);
        }
    }
    let staged = pool.and_then(|pool| {
        let (format, chain) = match &compressed {
            Some(blocks) => (TexelFormat::Bc7, blocks),
            None => (TexelFormat::Rgba8, &mips),
        };
        let levels: Vec<(u32, u32, &[u8])> = chain.iter().map(|(w, h, d)| (*w, *h, d.as_slice())).collect();
        StagedTexture::with_format(pool, format, &levels)
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

/// The files a texture leaves in the disk cache (LLTextureCache::
/// removeFromCache for a refresh): its JPEG 2000 data, whole or partial,
/// and its transcoded chain.
fn remove_cached(dir: &std::path::Path, id: &Uuid) {
    for path in [
        TextureStreamer::cache_path(dir, id, true),
        TextureStreamer::cache_path(dir, id, false),
        TextureStreamer::transcoded_path(dir, id),
    ] {
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("texture {id}: its cached file could not be removed: {e}");
        }
    }
}

/// One decode: the texture, the level, and what to do with the result.
struct DecodeJob {
    id: Uuid,
    discard: u8,
    keep_pixels: bool,
    /// Compress to BC7 when the texture comes out of it unharmed.
    bc7: bool,
    /// File of the transcoded cache to write, with the header of the image
    /// and the thread that writes it.
    cache: Option<(PathBuf, J2kInfo, texture_cache::Writer)>,
}

/// A texture from the disk cache: its transcoded chain at `want` when
/// `blocks` allows it and the file is there (staged, ready for the GPU),
/// else its JPEG 2000 data.
fn read_job(dir: &std::path::Path, id: Uuid, want: u8, blocks: bool, pool: Option<&StagingPool>) -> JobResult {
    let mut keep_rgba = false;
    if blocks {
        match texture_cache::read(&TextureStreamer::transcoded_path(dir, &id), want) {
            Some(Transcoded::Bc7(cached)) => {
                let levels: Vec<(u32, u32, &[u8])> = cached.levels.iter().map(|(w, h, d)| (*w, *h, d.as_slice())).collect();
                // (without room in the staging memory: decoded, as before)
                if let Some(staged) = pool.and_then(|pool| StagedTexture::with_format(pool, TexelFormat::Bc7, &levels)) {
                    return JobResult::TextureTranscoded {
                        id,
                        info: cached.info,
                        finest: cached.finest,
                        texture: Box::new(JobResult::Texture {
                            id,
                            discard: cached.discard,
                            mips: Vec::new(),
                            staged: Some(staged),
                            alpha: cached.alpha,
                            alpha_channel: cached.alpha_channel,
                            pick_mask: cached.pick_mask.and_then(PickMask::from_parts),
                            sculpt: None,
                        }),
                    };
                }
            }
            Some(Transcoded::KeepRgba) => keep_rgba = true,
            None => {}
        }
    }
    for complete in [true, false] {
        if let Ok(d) = crate::cache::read_touch(TextureStreamer::cache_path(dir, &id, complete)) {
            return JobResult::TextureCache {
                id,
                data: Some(d),
                complete,
                keep_rgba,
            };
        }
    }
    JobResult::TextureCache {
        id,
        data: None,
        complete: false,
        keep_rgba,
    }
}

fn decode_job(job: DecodeJob, data: &[u8], pool: Option<&StagingPool>, meter: &DecodeMeter) -> JobResult {
    let DecodeJob {
        id,
        discard,
        keep_pixels,
        bc7,
        cache,
    } = job;
    let timer = crate::deep_profile::timer();
    match aurora_assets::decode_j2k_timed(data, discard) {
        Ok((img, decode)) => {
            let start = Instant::now();
            let pixels = (img.width * img.height) as usize;
            let rgba = if img.components == 4 {
                img.data.clone()
            } else {
                to_rgba(img.components, &img.data, pixels)
            };
            let alpha_channel = img.components == 4 || img.components == 2;
            let alpha = if alpha_channel { classify_alpha(&rgba) } else { AlphaKind::Opaque };
            let rgba_done = start.elapsed();
            let sculpt = keep_pixels.then(|| {
                Arc::new(SculptMap {
                    width: img.width,
                    height: img.height,
                    components: img.components,
                    pixels: img.data,
                })
            });
            let before_mips = start.elapsed();
            let mips = mips_for(img.width, img.height, rgba, 16, alpha);
            let mips_done = start.elapsed();
            let tried = bc7 && pool.is_some();
            let compressed = tried
                .then(|| aurora_render::bc7::encode_chain(&mips, alpha != AlphaKind::Opaque))
                .flatten();
            let bc7_done = start.elapsed();
            let cache = cache.as_ref().map(|(path, info, writer)| CacheWrite {
                path,
                writer,
                info: *info,
                tried,
            });
            let result = texture_result_from(id, img.discard, mips, compressed, (alpha, alpha_channel), sculpt, pool, cache);
            let as_blocks = matches!(&result, JobResult::Texture { staged: Some(s), .. } if s.format == TexelFormat::Bc7);
            let stages = [rgba_done, mips_done - before_mips, bc7_done - mips_done, start.elapsed() - bc7_done];
            meter.add(img.discard, pixels as u64, decode, stages, as_blocks);
            crate::deep_profile::texture_decoded(timer, tried.then_some(stages[2]));
            result
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
    fn wearable_bakes_use_resident_bc7_pixels_without_jpeg_and_upgrade_progressively() {
        let id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("aurora-bake-{id}.bc7"));
        let mips = aurora_render::build_mips(512, 512, [25, 100, 200, 255].repeat(512 * 512), 16);
        let blocks = aurora_render::bc7::encode_chain(&mips, false).expect("compressible image");
        let meta = texture_cache::Meta {
            info: aurora_assets::J2kInfo {
                width: 512,
                height: 512,
                components: 4,
                levels: 10,
            },
            discard: 0,
            alpha: AlphaKind::Opaque,
            alpha_channel: false,
        };
        std::fs::write(&path, texture_cache::encode(&meta, None, &blocks).expect("cache")).expect("write cache");
        let source = |decoded, bytes| PreviewSource {
            local: None,
            bytes: Arc::new(vec![0; bytes]),
            jpeg: path.with_extension("j2k"),
            blocks: path.clone(),
            decoded,
            size: Some((512, 512)),
            procedural: false,
        };
        assert!(
            source(Some(1), 0).bake_ready(id),
            "coarse resident maps must not block all wearables"
        );
        let image = source(Some(1), 0).bake_image(id).expect("resident BC7 bake source");
        assert_eq!(image.size, [256, 256]);
        assert_eq!(image.rgba[3], 255);
        let image = source(Some(0), 0).bake_image(id).expect("full resident BC7 source");
        assert_eq!(
            image.size,
            [512, 512],
            "bakes retain full source resolution above the UI thumbnail limit"
        );
        assert_ne!(source(Some(0), 0).bake_revision(), source(Some(1), 0).bake_revision());
        assert_eq!(
            source(Some(1), 0).bake_revision(),
            source(Some(1), 500).bake_revision(),
            "packets do not restart an unchanged bake"
        );
        let transparent = aurora_prim::te::TRANSPARENT_TEXTURE;
        assert!(source(None, 0).bake_ready(transparent));
        assert_eq!(
            source(None, 0).bake_image(transparent).expect("built-in transparent").rgba.as_ref(),
            &[0; 4]
        );
        assert!(!source(None, 0).bake_ready(id));
        std::fs::remove_file(path).expect("remove own cache fixture");
    }

    #[test]
    fn local_pixels_are_shared_refreshed_and_released_without_a_download() {
        let mut textures = TextureStreamer::new(PathBuf::new());
        let id = Uuid::new_v4();
        let image = |color: [u8; 4]| {
            Arc::new(aurora_render::preview::Image {
                size: [1, 1],
                rgba: color.to_vec().into(),
            })
        };
        let first = image([255, 0, 0, 100]);
        textures.set_local_pixels(id, first.clone());
        let source = textures.preview_source(&id);
        let revision = source.image_revision();
        assert!(Arc::ptr_eq(&source.inspector_image(id).expect("bake pixels"), &first));
        assert!(Arc::ptr_eq(&textures.preview_source(&id).image(id).expect("UI pixels"), &first));
        let second = image([0, 0, 255, 200]);
        textures.set_local_pixels(id, second.clone());
        assert_ne!(textures.preview_source(&id).image_revision(), revision);
        assert!(Arc::ptr_eq(
            &textures.preview_source(&id).inspector_image(id).expect("reloaded pixels"),
            &second
        ));
        textures.unregister_local(&id);
        assert!(textures.preview_source(&id).local.is_none());
        assert!(
            !textures.entries.contains_key(&id),
            "no asset fetch or disk cache entry for local pixels"
        );
    }

    #[test]
    fn preview_images_ignore_the_budget_moving_finer_levels() {
        let source = |bytes: usize, decoded: Option<u8>| PreviewSource {
            local: None,
            bytes: Arc::new(vec![0; bytes]),
            jpeg: PathBuf::new(),
            blocks: PathBuf::new(),
            decoded,
            size: Some((1024, 1024)),
            procedural: false,
        };
        // 1024 px: the previews decode level 2 (256 px)
        let held = source(90_000, Some(0)).image_revision();
        assert_eq!(held, source(40_000, Some(1)).image_revision());
        assert_eq!(held, source(20_000, Some(2)).image_revision());
        assert_ne!(held, source(5_000, Some(3)).image_revision(), "a coarser level shows");
        assert_ne!(source(5_000, None).image_revision(), source(9_000, None).image_revision());
        assert_ne!(source(90_000, Some(0)).revision(), source(40_000, Some(1)).revision());
    }

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
        assert!(!textures.pick_masks[&super::super::avatar::IMG_INVISIBLE].hits(glam::Vec2::splat(0.5)));
        assert!(!textures.pick_masks[&aurora_prim::te::WATER_EXCLUSION_TEXTURE].hits(glam::Vec2::splat(0.5)));
        assert!(is_builtin_transparent(&aurora_prim::te::WATER_EXCLUSION_TEXTURE));
    }

    #[test]
    fn nonzero_pick_mask_preserves_each_texel_and_survives_cache_round_trip() {
        let mut rgba = vec![255; 3 * 3 * 4];
        for (pixel, alpha) in [(0, 0), (1, 1), (2, 32), (3, 33), (4, 0), (5, 127), (6, 255), (7, 0), (8, 1)] {
            rgba[pixel * 4 + 3] = alpha;
        }
        let mask = PickMask::from_rgba(3, 3, &rgba).unwrap();
        let (w, h, bits) = mask.parts();
        let restored = PickMask::from_parts((w, h, bits.to_vec())).unwrap();
        for y in 0..3 {
            for x in 0..3 {
                let uv = glam::Vec2::new((x as f32 + 0.5) / 3.0, (2.5 - y as f32) / 3.0);
                let nonzero = rgba[(y * 3 + x) * 4 + 3] != 0;
                assert_eq!(mask.hits_nonzero(uv), nonzero);
                assert_eq!(restored.hits_nonzero(uv + glam::Vec2::new(-2.0, 3.0)), nonzero);
                assert_eq!(restored.hits(uv), mask.hits(uv));
            }
        }
        assert!(PickMask::from_parts((w, h, vec![0])).is_none());
    }

    fn rgba(width: u32, height: u32, levels: u32) -> ChainSize {
        (TexelFormat::Rgba8, width, height, levels)
    }

    /// A 1024 × 1024 texture in use, loaded at `decoded`, covering `need_px`
    /// pixels in view.
    pub(super) fn entry(decoded: Option<u8>, need_px: f32) -> Entry {
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
            seen_px: 0.0,
            asked_at: None,
            gpu_size: decoded.map_or((0, 0), |d| (1024 >> d, 1024 >> d)),
            fetching: false,
            decoding: false,
            cache_state: 2,
            transcoded: None,
            keep_rgba: false,
            failures: 0,
            retry_at: None,
            keep_pixels: false,
            sculpt: None,
            alpha: AlphaKind::Opaque,
            unused_since: None,
            fetch_key: 0,
            sent: 0.0,
            urgent: false,
            dirty_cache: false,
            last_error: None,
            error_logged: false,
            gone: false,
            queued: false,
            counted: 0,
            refresh: false,
            epoch: 0,
        }
    }

    #[test]
    fn inventory_hover_cache_releases_only_its_ui_interest_and_preserves_other_users() {
        for (permanent, world_refs) in [(false, 0), (false, 1), (true, 0), (true, 1)] {
            let image = Uuid::from_u128(0x7e57);
            let mut e = entry(Some(2), 256.0);
            e.refs = 1 + world_refs;
            e.urgent = true;
            let mut streamer = pressed(0, vec![(image.as_u128(), e)]);
            streamer.ui_wanted.insert(image);
            if permanent {
                // A profile/editor can adopt a texture first loaded by hover.
                streamer.ui_permanent.insert(image);
            }
            streamer.ui_ready.insert(image, (1, 1, vec![255; 4]));
            streamer.release_transient_ui_image(&image);
            let refs = if permanent { 1 + world_refs } else { world_refs };
            assert_eq!(streamer.entries[&image].refs, refs);
            assert_eq!(streamer.ui_wanted.contains(&image), permanent);
            assert_eq!(streamer.ui_ready.contains_key(&image), permanent);
            assert_eq!(streamer.entries[&image].unused_since.is_some(), refs == 0);
            if refs == 0 {
                assert!(!streamer.entries[&image].urgent);
            }
            streamer.release_transient_ui_image(&image);
            assert_eq!(streamer.entries[&image].refs, refs, "release is idempotent");
        }
    }

    #[test]
    fn late_sculpt_pixel_request_decodes_even_when_the_gpu_is_full_resolution() {
        let id = Uuid::from_u128(587);
        for cache_state in [2, 3] {
            let mut s = pressed(
                0,
                vec![(
                    587,
                    Entry {
                        cache_state,
                        ..entry(Some(0), 64.0)
                    },
                )],
            );
            s.want_sculpt(&id);
            let e = &s.entries[&id];
            assert!(e.keep_pixels);
            assert_eq!(e.want, 0);
            assert!(s.sculpt_map(&id).is_none());
            assert!(e.has_work(), "a GPU texture does not supply a CPU sculpt map");
            assert!(e.needs_decode(0));
        }
    }

    #[test]
    fn color_decode_started_before_sculpt_request_schedules_the_missing_cpu_pixels() {
        let id = Uuid::from_u128(587);
        let mut s = pressed(
            0,
            vec![(
                587,
                Entry {
                    decoding: true,
                    ..entry(None, 64.0)
                },
            )],
        );
        s.want_sculpt(&id);
        // The worker captured keep_pixels=false before want_sculpt was called.
        s.on_job(texture_result(
            id,
            0,
            vec![(1, 1, vec![255; 4])],
            (AlphaKind::Opaque, false),
            None,
            None,
        ));
        // The next upload commits this level to Entry::decoded.
        s.entries.get_mut(&id).unwrap().decoded = Some(0);
        assert!(s.sculpt_map(&id).is_none());
        assert!(s.entries[&id].has_work());
        assert!(s.entries[&id].needs_decode(0));
        let pixels = vec![128; 64 * 64 * 3];
        let data = aurora_assets::j2k::encode_texture_j2k(64, 64, 3, &pixels, true).unwrap();
        let result = decode_job(
            DecodeJob {
                id,
                discard: 0,
                keep_pixels: true,
                bc7: false,
                cache: None,
            },
            &data,
            None,
            &DecodeMeter::default(),
        );
        s.on_job(result);
        assert_eq!(s.sculpt_map(&id).unwrap().width, 64);
        assert!(!s.entries[&id].has_work(), "fulfilled CPU requests must stop decoding");
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

    /// The file of a square texture of `side` pixels.
    fn file(side: u32) -> J2kInfo {
        J2kInfo {
            width: side,
            height: side,
            components: 3,
            levels: 5,
        }
    }

    #[test]
    fn the_first_request_follows_the_size_on_screen() {
        let level = |discard| aurora_assets::bytes_for_discard(&TYPICAL, discard);
        // not seen yet, or small on screen: the least
        assert_eq!(first_bytes(0.0), INITIAL_BYTES);
        assert_eq!(first_bytes(40.0), INITIAL_BYTES);
        assert_eq!(first_bytes(128.0), INITIAL_BYTES);
        // larger: what a 1024 texture needs to show that many pixels
        assert_eq!(first_bytes(200.0), level(2));
        assert_eq!(first_bytes(256.0), level(2));
        assert_eq!(first_bytes(300.0), level(1));
        assert_eq!((level(2), level(1)), (45_656, 176_728));
        // never the whole file before its size is known
        assert_eq!(first_bytes(600.0), level(1));
        assert_eq!(first_bytes(4096.0), level(1));
    }

    #[test]
    fn the_level_decoded_is_the_finest_the_bytes_hold() {
        let info = file(1024);
        let needs = |discard| aurora_assets::bytes_for_discard(&info, discard);
        // the first reply of a texture wanted at a quarter of its size
        assert_eq!(level_held(&info, INITIAL_BYTES, 2), 3);
        assert_eq!(level_held(&info, needs(2), 2), 2);
        assert_eq!(level_held(&info, needs(2) - 1, 2), 3);
        // never finer than wanted
        assert_eq!(level_held(&info, needs(1), 4), 4);
        // full resolution only from the complete file
        assert_eq!(level_held(&info, needs(1), 0), 1);
        // too little for anything: the coarsest is tried
        assert_eq!(level_held(&info, 100, 2), 5);
    }

    #[test]
    fn first_pixels_are_decoded_while_the_wanted_level_downloads() {
        let info = file(1024);
        let needed = aurora_assets::bytes_for_discard(&info, 2);
        let mut e = entry(None, 200.0);
        e.info = Some(info);
        e.data = Arc::new(vec![0; INITIAL_BYTES]);
        assert!(e.holds_first_pixels(needed));
        // enough for the level wanted: that one is decoded, nothing first
        assert!(!e.holds_first_pixels(INITIAL_BYTES));
        // already showing something: the wanted level alone follows
        e.decoded = Some(5);
        assert!(!e.holds_first_pixels(needed));
        e.decoded = None;
        // the whole file is there
        e.complete = true;
        assert!(!e.holds_first_pixels(needed));
        e.complete = false;
        // not even the coarsest level, or a file whose size is unknown
        e.data = Arc::new(vec![0; 100]);
        assert!(!e.holds_first_pixels(needed));
        e.data = Arc::new(vec![0; INITIAL_BYTES]);
        e.info = None;
        assert!(!e.holds_first_pixels(needed));
    }

    #[test]
    fn the_level_wanted_follows_the_size_of_the_file_once_known() {
        // 200 pixels on screen, wanted as for a file of 512 until it is read
        let level = |side: u32| {
            let mut e = entry(None, 200.0);
            e.want_before_the_first_pass();
            assert_eq!(e.want, 1);
            e.info = Some(file(side));
            e.want_for_its_size();
            e.want
        };
        assert_eq!([256, 512, 1024, 2048].map(level), [0, 1, 2, 3]);
        // what `recompute_wants` saw counts, although `need_px` has decayed
        let mut e = entry(None, 100.0);
        e.seen_px = 200.0;
        assert_eq!(e.px_now(), 200.0);
        e.info = Some(file(1024));
        e.want_for_its_size();
        assert_eq!(e.want, 2);
        // not seen: left as it is, as is a texture whose pixels are kept
        let mut unseen = entry(None, 0.0);
        unseen.want = 2;
        unseen.info = Some(file(1024));
        unseen.want_before_the_first_pass();
        unseen.want_for_its_size();
        assert_eq!(unseen.want, 2);
        let mut kept = entry(None, 50.0);
        kept.keep_pixels = true;
        kept.info = Some(file(1024));
        kept.want_before_the_first_pass();
        kept.want_for_its_size();
        assert_eq!(kept.want, 0);
        // a level already computed by the pass is not computed again from
        // what was noted since
        let mut passed = entry(None, 20.0);
        passed.seen_px = 300.0;
        passed.want = 0;
        passed.want_before_the_first_pass();
        assert_eq!(passed.want, 0);
    }

    #[test]
    fn the_pass_keeps_the_size_it_saw() {
        let mut s = pressed(0, vec![(1, entry(None, 300.0))]);
        s.recompute_wants();
        let e = &s.entries[&Uuid::from_u128(1)];
        assert_eq!((e.seen_px, e.need_px), (300.0, 150.0));
        assert_eq!(e.px_now(), 300.0);
    }

    #[test]
    fn terrain_material_maps_request_the_reference_two_kilopixel_resolution() {
        let mut e = entry(Some(2), 0.0);
        e.info = Some(file(2048));
        e.gpu_size = (512, 512);
        let id = Uuid::from_u128(1);
        let mut s = pressed(0, vec![(1, e)]);
        s.note_usage_fixed(&id, super::super::terrain_surface::DECODE_PIXELS);
        s.recompute_wants();
        assert_eq!(s.entries[&id].want, 0, "terrain normal maps must upgrade beyond the 512 px preview");
    }

    #[test]
    fn hud_textures_load_at_full_resolution_even_under_memory_pressure() {
        let id = Uuid::from_u128(1);
        for step in [0, 2] {
            // a 1024 atlas on a 30 px button, three levels coarse on the GPU
            let mut e = entry(Some(3), 30.0);
            e.info = Some(file(1024));
            e.gpu_size = (128, 128);
            let mut s = pressed(step, vec![(1, e)]);
            s.note_usage_fixed(&id, super::super::hud::TEXTURE_PIXELS);
            s.recompute_wants();
            assert_eq!(s.entries[&id].want, 0, "pressure step {step}");
        }
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
    fn a_waiting_download_follows_the_view_and_goes_with_its_last_user() {
        let waiting = |key: u64, refs: u32, sent_px: f32| {
            let mut e = entry(None, 300.0);
            e.refs = refs;
            e.fetching = true;
            e.fetch_key = key;
            e.sent = fetch_priority::texture(sent_px, false);
            e
        };
        // asked for while far, now large on screen; nothing uses the second
        // any more; the third has not moved
        let mut s = pressed(
            0,
            vec![(1, waiting(11, 1, 10.0)), (2, waiting(12, 0, 300.0)), (3, waiting(13, 1, 300.0))],
        );
        for (key, n) in [(11, 1), (12, 2), (13, 3)] {
            s.by_fetch_key.insert(key, Uuid::from_u128(n));
        }
        s.recompute_wants();
        assert_eq!(s.moved, [(11, fetch_priority::texture(300.0, false))]);
        assert_eq!(s.dropped, [12]);
        assert!(!s.entries[&Uuid::from_u128(2)].fetching);
        assert!(!s.by_fetch_key.contains_key(&12));
        assert!(s.by_fetch_key.contains_key(&13));
    }

    #[test]
    fn our_own_bakes_are_urgent() {
        let mut s = TextureStreamer::new(PathBuf::new());
        let me = Uuid::from_u128(0xA1);
        s.set_agent(&me);
        let bake = |avatar: Uuid| TexSource::Bake {
            url: format!("https://appearance.example/texture/{avatar}/head/{}", Uuid::from_u128(7)),
        };
        assert!(own_bake(&s.own_bakes, &bake(me)));
        assert!(!own_bake(&s.own_bakes, &bake(Uuid::from_u128(0xB2))));
        assert!(!own_bake(&s.own_bakes, &TexSource::Asset));
        // before login, nothing is ours
        assert!(!own_bake("", &bake(me)));
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
    fn a_texture_has_work_until_it_shows_the_level_it_wants() {
        // (the test entry: in use, disk cache looked up, level 0 wanted)
        let mut e = entry(None, 100.0);
        assert!(e.has_work(), "never loaded");
        e.cache_state = 0;
        assert!(e.has_work(), "its first disk read");
        // a read, a download or a decode under way: their result is awaited
        e.cache_state = 1;
        assert!(!e.has_work());
        e.cache_state = 2;
        e.fetching = true;
        assert!(!e.has_work());
        e.fetching = false;
        e.decoding = true;
        assert!(!e.has_work());
        e.decoding = false;
        // shown at a coarser level than wanted, then at that level
        e.decoded = Some(2);
        assert!(e.has_work());
        e.decoded = Some(0);
        assert!(!e.has_work());
        // a coarser level wanted is nothing to load
        e.want = 3;
        assert!(!e.has_work());
        // nothing uses it: it only waits for its eviction
        e.decoded = None;
        assert!(e.has_work());
        e.refs = 0;
        assert!(!e.has_work());
    }

    /// The counters a walk of every texture gives.
    fn walked(s: &TextureStreamer) -> Counts {
        let mut c = Counts::default();
        for e in s.entries.values() {
            c.change(0, e.count_bits());
        }
        c
    }

    #[test]
    fn textures_are_listed_once_and_counted_as_they_change() {
        let mut s = pressed(0, vec![(1, entry(None, 100.0)), (2, entry(Some(0), 100.0))]);
        let (new, shown) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let settle = |s: &mut TextureStreamer, id: Uuid, change: &dyn Fn(&mut Entry)| {
            let e = s.entries.get_mut(&id).expect("entry");
            change(e);
            TextureStreamer::settle(&mut s.todo, &mut s.counts, id, e);
        };
        // the one to load is listed, once; the one shown is only counted
        for _ in 0..2 {
            settle(&mut s, new, &|_| {});
            settle(&mut s, shown, &|_| {});
        }
        assert_eq!(s.todo, [new]);
        assert_eq!(
            s.counts,
            Counts {
                loaded: 1,
                ..Default::default()
            }
        );
        // its download starts, then fails
        settle(&mut s, new, &|e| e.fetching = true);
        assert_eq!((s.counts.fetching, s.counts.failing), (1, 0));
        settle(&mut s, new, &|e| {
            e.fetching = false;
            e.last_error = Some("status 503".into());
        });
        assert_eq!((s.counts.fetching, s.counts.failing), (0, 1));
        assert_eq!(s.counts, walked(&s));
        // nothing uses it any more: no longer failing, as for the walk
        s.release(&new);
        assert_eq!(s.counts.failing, 0);
        assert_eq!(s.counts, walked(&s));
        // a finer level wanted of the one shown lists it; shown, it is done
        settle(&mut s, shown, &|e| e.decoded = Some(3));
        assert_eq!(s.todo, [new, shown]);
        settle(&mut s, shown, &|e| e.decoded = Some(0));
        assert_eq!(s.counts.loaded, 1);
        // the pass over every texture keeps both right
        s.recompute_wants();
        assert_eq!(s.counts, walked(&s));
        for (id, e) in &s.entries {
            assert!(!e.has_work() || e.queued, "{id}");
        }
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

    #[test]
    fn pressure_lowers_only_what_brings_the_memory_under_its_goal() {
        // three 1024 textures loaded whole, 200 px on screen: each would
        // give back three quarters of its bytes at level 1
        let mb = 1024 * 1024 * 16 / 3;
        let lower = |to_lower: u64| {
            let mut s = pressed(1, (1..=3).map(|id| (id, entry(Some(0), 200.0))).collect());
            s.to_lower = to_lower;
            s.recompute_wants();
            s.reductions.iter().map(|r| r.0.as_u128()).collect::<Vec<_>>()
        };
        // a little more than one gives back: two of them, taken from the end
        assert_eq!(lower(mb), vec![2, 3]);
        // under the goal: nothing lowered
        assert!(lower(0).is_empty());
        // far over it: all of them
        assert_eq!(lower(10 * mb), vec![1, 2, 3]);
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
            epoch: 0,
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
        assert_eq!(
            (q.queued(rgba(4, 4, 3)), q.queued(rgba(2, 2, 2)), q.queued(rgba(8, 8, 4))),
            (2, 1, 0)
        );
        // the one popped no longer counts; put back, it does again
        let u = q.pop(&mut trash).expect("upload");
        assert_eq!((u.id.as_u128(), q.queued(rgba(2, 2, 2))), (3, 0));
        q.requeue(u);
        assert_eq!(q.queued(rgba(2, 2, 2)), 1);
        // a coarser level retired on the way is forgotten too
        q.push(sized(4, 2, &small), true, 90.0);
        q.push(sized(4, 0, &big), true, 90.0);
        assert_eq!((q.queued(rgba(4, 4, 3)), q.queued(rgba(2, 2, 2))), (3, 2));
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

    /// A texture table with these slots left, which notes those given back.
    #[derive(Default)]
    struct Slots {
        left: Vec<u32>,
        freed: Vec<u32>,
        missing: Vec<u32>,
    }

    impl SlotTable for Slots {
        fn placeholder(&mut self) -> Option<u32> {
            self.left.pop()
        }

        fn free(&mut self, slot: u32) {
            self.freed.push(slot);
            self.left.push(slot);
        }

        fn show_missing(&mut self, slot: u32) -> bool {
            self.missing.push(slot);
            true
        }
    }

    /// A failed download of texture `n` (in slot `10 + n`, never loaded).
    fn fail(s: &mut TextureStreamer, n: u128, status: u16) {
        let key = 1000 + n as u64;
        let e = s.entries.entry(id(n)).or_insert_with(|| {
            let mut e = entry(None, 100.0);
            e.slot = 10 + n as u32;
            e
        });
        e.fetching = true;
        e.fetch_key = key;
        s.by_fetch_key.insert(key, id(n));
        s.on_fetch(FetchResult {
            key,
            status,
            complete: false,
            offset: 0,
            data: Err(format!("status {status}")),
        });
    }

    #[test]
    fn a_texture_missing_on_the_asset_server_leaves_the_rez_grid() {
        let mut s = pressed(0, Vec::new());
        let mut table = Slots::default();
        // the asset CDN answers 403 for a missing asset, as 404: given up
        // at once, asked again in ten minutes
        fail(&mut s, 1, 403);
        fail(&mut s, 2, 404);
        for n in [1, 2] {
            let e = &s.entries[&id(n)];
            assert!(e.gone && s.is_unavailable(&id(n)));
            assert!(e.retry_at.is_some_and(|t| t > Instant::now() + Duration::from_secs(500)));
        }
        // a server error is retried; only the fourth failure in a row gives up
        for _ in 0..3 {
            fail(&mut s, 3, 503);
        }
        assert!(!s.entries[&id(3)].gone && !s.is_unavailable(&id(3)));
        s.show_unavailable(&mut table);
        assert_eq!(table.missing, [11, 12]);
        fail(&mut s, 3, 503);
        assert!(s.is_unavailable(&id(3)));
        // told once; one that loaded meanwhile keeps its texels
        fail(&mut s, 3, 503);
        fail(&mut s, 4, 404);
        s.entries.get_mut(&id(4)).expect("entry").decoded = Some(2);
        table.missing.clear();
        s.show_unavailable(&mut table);
        assert_eq!(table.missing, [13]);
    }

    /// The streamer with texture `n` in slot `10 + n` for each of `used`
    /// and of `unused`, the latter released in that order a while ago.
    fn full_table(used: &[u128], unused: &[u128]) -> TextureStreamer {
        let held = |n: &u128| {
            let mut e = entry(Some(0), 100.0);
            e.slot = 10 + *n as u32;
            (*n, e)
        };
        let mut s = pressed(0, used.iter().chain(unused).map(held).collect());
        let t0 = Instant::now() - Duration::from_secs(5);
        for (i, n) in unused.iter().enumerate() {
            let since = t0 + Duration::from_millis(i as u64);
            let e = s.entries.get_mut(&id(*n)).expect("entry");
            (e.refs, e.unused_since) = (0, Some(since));
            s.unused.push_back((since, id(*n)));
        }
        s
    }

    #[test]
    fn a_full_table_gives_the_slots_of_unused_textures_to_new_ones() {
        // the region just left (2, 3 and 4, released 5 s ago: 40 s before
        // their eviction) holds every slot the textures in use (1, 5) do not
        let mut s = full_table(&[1, 5], &[2, 3, 4]);
        let mut table = Slots::default();
        // 3 is in use again; 2 is still being decoded, it cannot go yet
        s.entries.get_mut(&id(3)).expect("entry").refs = 1;
        s.entries.get_mut(&id(3)).expect("entry").unused_since = None;
        s.entries.get_mut(&id(2)).expect("entry").decoding = true;
        // a new texture takes the slot of the oldest one that can go
        assert_eq!(s.new_slot(&mut table), 14);
        assert_eq!(table.freed, [14]);
        assert!(!s.entries.contains_key(&id(4)));
        assert_eq!((s.slotless, s.evicted.len()), (0, 1));
        // those in use and the one being decoded were left alone
        for n in [1, 2, 3, 5] {
            assert_eq!(s.entries[&id(n)].slot, 10 + n as u32);
        }
        assert_eq!(s.unused.iter().map(|u| u.1).collect::<Vec<_>>(), [id(2)]);
        // a table with room evicts nothing before its time
        let mut s = full_table(&[1], &[2, 3]);
        let mut table = Slots {
            left: vec![40],
            ..Default::default()
        };
        assert_eq!(s.new_slot(&mut table), 40);
        assert!(table.freed.is_empty());
        assert_eq!((s.entries.len(), s.unused.len()), (3, 2));
    }

    #[test]
    fn textures_without_a_slot_are_counted_and_reported_together() {
        // every slot holds a texture in use
        let mut s = full_table(&[1, 2], &[]);
        let mut table = Slots::default();
        assert_eq!(s.new_slot(&mut table), NO_SLOT);
        assert_eq!(s.new_slot(&mut table), NO_SLOT);
        assert_eq!((s.slotless, s.slotless_unreported), (2, 2));
        assert!(table.freed.is_empty());
        assert_eq!(s.entries.len(), 2);
        // one line of the log for both, then nothing until there are more
        s.report_slotless(16384);
        assert_eq!((s.slotless, s.slotless_unreported), (2, 0));
        // evicted once nothing uses it: it had no slot to give back, and
        // the white texture its faces drew is not freed
        let since = Instant::now() - Duration::from_secs(60);
        s.entries.insert(
            id(9),
            Entry {
                slot: NO_SLOT,
                refs: 0,
                unused_since: Some(since),
                ..entry(None, 0.0)
            },
        );
        s.unused.push_back((since, id(9)));
        let (entries, now) = (&s.entries, Instant::now());
        let due = eviction_round(&mut s.unused, now, UNUSED_EVICT, |id, since| evictable(entries, id, since));
        assert_eq!(due, [id(9)]);
        s.evict(&mut table, due);
        assert_eq!(s.slotless, 1);
        assert!(table.freed.is_empty());
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
    fn waiting_upload_covers_its_level_and_the_coarser_ones() {
        let mut q = PendingUploads::default();
        let mut trash = Trash::default();
        assert!(!q.covers(&id(1), 5));
        q.push(pending(1, 2), true, 100.0);
        // no decode again at 2 or coarser while it waits; a finer one may go
        assert!(q.covers(&id(1), 2) && q.covers(&id(1), 5));
        assert!(!q.covers(&id(1), 1) && !q.covers(&id(2), 5));
        // once it left for the GPU, the texture's `decoded` level takes over
        assert!(q.pop(&mut trash).is_some());
        assert!(!q.covers(&id(1), 5));
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

    /// A profile picture's UI copy is 256 px wide at most, and a coarser
    /// resident level gives a coarser copy (still to be refined).
    #[test]
    fn ui_copy_width_follows_the_level_held() {
        assert_eq!(ui_width(1024, 768, 0), 256);
        assert_eq!(ui_width(1024, 768, 2), 256);
        assert_eq!(ui_width(1024, 768, 4), 64);
        assert_eq!(ui_width(128, 128, 0), 128);
        assert_eq!(ui_width(128, 128, 1), 64);
        assert_eq!(ui_width(300, 200, 1), 150);
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

    /// A streamer over a cache folder of its own, removed when dropped.
    struct RefreshBench {
        dir: PathBuf,
        s: TextureStreamer,
        jobs: Jobs,
        results: crossbeam_channel::Receiver<JobResult>,
        _rt: tokio::runtime::Runtime,
        fetcher: Fetcher,
    }

    impl RefreshBench {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("aurora-refresh-{}", Uuid::new_v4()));
            let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
            let (fetcher, _results) = Fetcher::new(rt.handle(), reqwest::Client::new(), 1, Default::default());
            let (jobs, results) = Jobs::new();
            RefreshBench {
                s: TextureStreamer::new(dir.clone()),
                dir,
                jobs,
                results,
                _rt: rt,
                fetcher,
            }
        }

        /// A texture shown from its downloaded data, with its three files.
        fn loaded(&mut self, id: Uuid) {
            let mut e = entry(Some(2), 100.0);
            e.data = Arc::new(vec![1, 2, 3]);
            e.complete = true;
            e.cache_state = 3;
            e.transcoded = Some(2);
            e.failures = 2;
            self.s.entries.insert(id, e);
            for name in self.files(id) {
                std::fs::write(name, b"old").expect("cache file");
            }
        }

        fn files(&self, id: Uuid) -> [PathBuf; 3] {
            [
                TextureStreamer::cache_path(&self.dir, &id, true),
                TextureStreamer::cache_path(&self.dir, &id, false),
                TextureStreamer::transcoded_path(&self.dir, &id),
            ]
        }

        /// One `update`, then its jobs to the end (their results are
        /// dropped: these tests look at the entries and the files).
        fn update(&mut self) {
            self.s.update(&self.jobs, &self.fetcher, None);
            let until = Instant::now() + Duration::from_secs(10);
            while self.jobs.pending() > 0 && Instant::now() < until {
                let _ = self.results.recv_timeout(Duration::from_millis(20));
            }
            while self.results.try_recv().is_ok() {}
        }
    }

    impl Drop for RefreshBench {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_refresh_forgets_the_texture_and_its_files() {
        let mut b = RefreshBench::new();
        let id = Uuid::from_u128(0x7e57);
        b.loaded(id);
        assert!(b.s.refresh(&id));
        b.update();
        let e = &b.s.entries[&id];
        assert!(e.data.is_empty() && !e.complete && e.info.is_none());
        // shown again from what is downloaded, not read from the disk
        assert_eq!((e.decoded, e.cache_state, e.transcoded), (None, 2, None));
        assert_eq!((e.failures, e.epoch, e.refresh), (0, 1, false));
        // the slot still holds the texels until the new ones come
        assert_eq!(e.slot, 9);
        for file in b.files(id) {
            assert!(!file.exists(), "{} still there", file.display());
        }
    }

    #[test]
    fn a_refresh_waits_for_the_download_under_way() {
        let mut b = RefreshBench::new();
        let id = Uuid::from_u128(0x7e58);
        b.loaded(id);
        let e = b.s.entries.get_mut(&id).expect("entry");
        e.fetching = true;
        e.fetch_key = 77;
        b.s.by_fetch_key.insert(77, id);
        assert!(b.s.refresh(&id));
        b.update();
        // nothing forgotten while the reply may still bring old bytes
        assert_eq!(b.s.entries[&id].data.len(), 3);
        assert!(b.files(id).iter().all(|f| f.exists()));
        b.s.on_fetch(FetchResult {
            key: 77,
            status: 200,
            complete: true,
            offset: 0,
            data: Ok(vec![4, 5, 6, 7]),
        });
        b.update();
        let e = &b.s.entries[&id];
        assert!(e.data.is_empty());
        assert_eq!(e.epoch, 1);
        assert!(b.files(id).iter().all(|f| !f.exists()));
    }

    #[test]
    fn only_streamed_textures_are_refreshed() {
        let mut b = RefreshBench::new();
        assert!(!b.s.refresh(&Uuid::nil()));
        assert!(!b.s.refresh(&aurora_prim::te::BLANK_TEXTURE));
        assert!(!b.s.refresh(&aurora_prim::te::TRANSPARENT_TEXTURE));
        assert!(!b.s.refresh(&aurora_prim::te::WATER_EXCLUSION_TEXTURE));
        // the invisible bake is never downloaded, so never refreshed either
        assert!(!b.s.refresh(&super::super::avatar::IMG_INVISIBLE));
        assert!(b.s.entries.is_empty());
        // a texture not held: only its files go
        let id = Uuid::from_u128(0x7e59);
        for file in b.files(id) {
            std::fs::write(file, b"old").expect("cache file");
        }
        assert!(b.s.refresh(&id));
        b.update();
        assert!(!b.s.entries.contains_key(&id));
        assert!(b.files(id).iter().all(|f| !f.exists()));
    }

    #[test]
    fn an_unused_texture_is_forgotten_but_not_downloaded() {
        let mut b = RefreshBench::new();
        let id = Uuid::from_u128(0x7e5a);
        b.loaded(id);
        b.s.entries.get_mut(&id).expect("entry").refs = 0;
        assert!(b.s.refresh(&id));
        b.update();
        let e = &b.s.entries[&id];
        assert!(e.data.is_empty() && !e.fetching);
        assert!(!e.has_work());
    }
}
