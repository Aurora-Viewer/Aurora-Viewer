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

use super::jobs::{AlphaKind, JobResult, Jobs, SculptMap, classify_alpha, to_rgba};
use aurora_assets::J2kInfo;
use aurora_net::fetch::ByteRange;
use aurora_net::{FetchRequest, FetchResult, Fetcher};
use aurora_render::{MipLevel, Renderer, StagedTexture, StagingPool, build_mips};
use std::collections::{BinaryHeap, HashMap};
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
    need_px: f32,
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
    priority: UploadPriority,
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

/// Decoded textures waiting for the GPU, most urgent first.
#[derive(Default)]
struct PendingUploads {
    heap: BinaryHeap<PendingUpload>,
    /// Finest discard level waiting, per texture: a coarser one still in
    /// the queue is dropped when its turn comes (the finer one replaces it).
    finest: HashMap<Uuid, u8>,
    seq: u64,
}

impl PendingUploads {
    fn len(&self) -> usize {
        self.heap.len()
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
        self.heap.push(u);
    }

    /// The most urgent upload that is still the finest of its texture.
    fn pop(&mut self) -> Option<PendingUpload> {
        while let Some(u) = self.heap.pop() {
            match self.finest.get(&u.id) {
                Some(&f) if f < u.discard => continue,
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
}

/// Alpha class of a bindless slot; a change bumps `generation`.
fn set_slot_alpha(alpha: &mut Vec<AlphaKind>, generation: &mut u64, slot: u32, a: AlphaKind) {
    let i = slot as usize;
    if i >= alpha.len() {
        alpha.resize(i + 1024, AlphaKind::Opaque);
    }
    if alpha[i] != a {
        alpha[i] = a;
        *generation += 1;
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
    /// Per slot: the decoded image has an alpha channel (LL sends such
    /// faces to the alpha pool when rigged or glowing, even if opaque).
    pub alpha_channel_by_slot: Vec<bool>,
    cache_dir: PathBuf,
    /// Procedural demo textures: id -> slot.
    local_slots: HashMap<Uuid, u32>,
    /// Bumped whenever a slot's alpha class or alpha channel, or a sculpt
    /// map, changes: the scene classifies its faces again (the faces'
    /// pass is cached, and the GPU lists read it).
    pub generation: u64,
    max_decodes: usize,
    decodes: usize,
    pub discard_bias: u8,
    /// Textures also wanted as egui images (profile pictures...): small CPU copies.
    ui_wanted: std::collections::HashSet<Uuid>,
    ui_ready: HashMap<Uuid, (u32, u32, Vec<u8>)>,
    pub stats: TextureStats,
    last_maintenance: Instant,
    /// Unused textures are evicted after this long (shortened by the
    /// AURORA_DEMO_TEXTURES_CHURN test).
    pub evict_after: Duration,
}

impl TextureStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("tex"));
        TextureStreamer {
            entries: HashMap::new(),
            by_fetch_key: HashMap::new(),
            next_key: 1,
            uploads: PendingUploads::default(),
            staging: None,
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
            discard_bias: 0,
            ui_wanted: Default::default(),
            ui_ready: HashMap::new(),
            stats: TextureStats::default(),
            last_maintenance: Instant::now(),
            evict_after: UNUSED_EVICT,
        }
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
                slot,
                super::jobs::classify_alpha(&px),
            );
            self.local_slots.insert(id, slot);
            return slot;
        }
        let placeholder = [148u8, 148, 156, 255];
        let slot = renderer
            .create_texture(&[MipLevel {
                width: 1,
                height: 1,
                data: &placeholder,
            }])
            .unwrap_or(aurora_render::textures::WHITE);
        set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, slot, AlphaKind::Opaque);
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
            },
        );
        slot
    }

    /// A texture drawn by the viewer itself (media): `acquire` returns its slot.
    pub fn register_local(&mut self, id: Uuid, slot: u32) {
        set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, slot, AlphaKind::Opaque);
        if let Some(c) = self.alpha_channel_by_slot.get_mut(slot as usize)
            && *c
        {
            *c = false;
            self.generation += 1;
        }
        self.local_slots.insert(id, slot);
    }

    pub fn unregister_local(&mut self, id: &Uuid) {
        self.local_slots.remove(id);
    }

    pub fn release(&mut self, id: &Uuid) {
        if let Some(e) = self.entries.get_mut(id) {
            e.refs = e.refs.saturating_sub(1);
            if e.refs == 0 {
                e.unused_since = Some(Instant::now());
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

    /// Something uses this texture (it is streamed).
    pub fn is_wanted(&self, id: &Uuid) -> bool {
        self.entries.contains_key(id)
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

    /// Recompute desired discard levels from accumulated usage.
    pub fn recompute_wants(&mut self) {
        for e in self.entries.values_mut() {
            if e.keep_pixels {
                e.want = 0;
                continue;
            }
            let full = e.info.map(|i| i.width.max(i.height)).unwrap_or(512) as f32;
            let need = e.need_px.max(8.0);
            let mut d = (full / need).log2().floor().clamp(0.0, 5.0) as u8;
            if let Some(info) = e.info {
                d = d.min(info.levels.min(5));
            }
            d = (d + self.discard_bias).min(5);
            // Never drop below what is already loaded when still referenced.
            e.want = d;
            e.need_px *= 0.5; // decay so textures that shrank on screen can be lowered later
        }
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
            let priority = e.need_px.max(1.0) + if e.decoded.is_none() { 1e5 } else { 0.0 };
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
                e.data = Arc::new(bytes);
                e.complete = true;
                Ok(())
            }
            Ok(bytes) => {
                if merge_reply(Arc::make_mut(&mut e.data), r.offset, &bytes) {
                    e.complete |= r.complete;
                    Ok(())
                } else {
                    Err(format!(
                        "status 206 at byte {} ({} bytes) does not continue our {} bytes",
                        r.offset,
                        bytes.len(),
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
                e.dirty_cache = true;
                e.last_error = None;
                e.error_logged = false;
            }
            Err(err) => {
                e.failures += 1;
                let backoff = 2u64.pow(e.failures.min(6));
                e.retry_at = Some(Instant::now() + Duration::from_secs(backoff));
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
                        priority: UploadPriority::default(),
                    },
                    e.decoded.is_none(),
                    e.need_px,
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
    pub fn upload(&mut self, renderer: &mut Renderer, budget: UploadBudget) {
        let t0 = Instant::now();
        let mut spent = 0u64;
        let mut done = 0usize;
        // staged by a job still writing in the same chunk: next frame
        let mut waiting = Vec::new();
        while budget.allows(done, spent, t0.elapsed()) {
            let Some(u) = self.uploads.pop() else {
                break;
            };
            let Some(e) = self.entries.get_mut(&u.id) else {
                continue;
            };
            if e.decoded.is_some_and(|d| d <= u.discard) {
                continue;
            }
            if u.staged.as_ref().is_some_and(|s| !s.data.ready()) {
                waiting.push(u);
                continue;
            }
            // a new texture page (up to ~1 ms to create) only while most of
            // the frame's budget is left: next frame, it goes first
            let size = match &u.staged {
                Some(staged) => staged.levels.first().map(|l| (l.width, l.height, staged.levels.len())),
                None => u.mips.first().map(|m| (m.0, m.1, u.mips.len())),
            };
            if done > 0
                && t0.elapsed() > budget.time / 4
                && size.is_some_and(|(w, h, levels)| renderer.textures.needs_page(e.slot, w, h, levels as u32))
            {
                // (the uploads behind it wait too: thousands may need
                // that same page)
                waiting.push(u);
                break;
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
            if self.ui_wanted.contains(&u.id) {
                // largest level that fits 256 px for the UI copy
                if let Some((w, h, d)) = u.mips.iter().find(|m| m.0.max(m.1) <= 256).or(u.mips.last()) {
                    let better = self.ui_ready.get(&u.id).is_none_or(|r| r.0 < *w);
                    if better {
                        self.ui_ready.insert(u.id, (*w, *h, d.clone()));
                    }
                }
            }
            e.decoded = Some(u.discard);
            e.alpha = u.alpha;
            set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, e.slot, u.alpha);
            let slot = e.slot as usize;
            if slot >= self.alpha_channel_by_slot.len() {
                self.alpha_channel_by_slot.resize(slot + 1024, false);
            }
            if self.alpha_channel_by_slot[slot] != u.alpha_channel {
                self.alpha_channel_by_slot[slot] = u.alpha_channel;
                self.generation += 1;
            }
        }
        for u in waiting {
            self.uploads.requeue(u);
        }
        self.stats.uploaded_bytes_frame = spent;
        self.stats.uploads_frame = done;
    }

    /// Evict unused textures and persist downloaded data.
    pub fn maintain(&mut self, renderer: &mut Renderer, jobs: &Jobs, budget_bytes: u64) {
        if self.last_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_maintenance = Instant::now();
        let now = Instant::now();
        let mut evict = Vec::new();
        // downloaded data to persist: shared, not copied, and queued in one go
        let mut writes = Vec::new();
        for (id, e) in self.entries.iter_mut() {
            if e.dirty_cache && !e.fetching && !e.data.is_empty() {
                e.dirty_cache = false;
                writes.push((*id, e.complete, e.data.clone()));
            }
            if e.refs == 0 && e.unused_since.is_some_and(|t| now.duration_since(t) > self.evict_after) && !e.fetching && !e.decoding {
                evict.push(*id);
            }
        }
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
        for id in evict {
            if let Some(e) = self.entries.remove(&id) {
                renderer.free_texture(e.slot);
                set_slot_alpha(&mut self.alpha_by_slot, &mut self.generation, e.slot, AlphaKind::Opaque);
            }
        }
        // Memory budget: raise the global discard bias when above budget.
        let used = renderer.textures.bytes();
        if used > budget_bytes && self.discard_bias < 2 {
            self.discard_bias += 1;
            log::info!("texture memory {} MB over budget: discard bias {}", used >> 20, self.discard_bias);
        } else if used < budget_bytes / 2 && self.discard_bias > 0 {
            self.discard_bias -= 1;
        }
    }
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

    fn pending(id: u128, discard: u8) -> PendingUpload {
        PendingUpload {
            id: Uuid::from_u128(id),
            discard,
            staged: None,
            mips: Vec::new(),
            alpha: AlphaKind::Opaque,
            alpha_channel: false,
            priority: UploadPriority::default(),
        }
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
        let order: Vec<u128> = std::iter::from_fn(|| q.pop()).map(|u| u.id.as_u128()).collect();
        assert_eq!(order, [3, 5, 2, 1, 4]);
    }

    #[test]
    fn coarser_level_is_dropped_when_a_finer_one_waits() {
        let mut q = PendingUploads::default();
        q.push(pending(1, 2), true, 100.0);
        q.push(pending(2, 2), true, 50.0);
        // the full resolution of texture 1 arrives before its level 2 went
        q.push(pending(1, 0), true, 100.0);
        let got: Vec<(u128, u8)> = std::iter::from_fn(|| q.pop()).map(|u| (u.id.as_u128(), u.discard)).collect();
        assert_eq!(got, [(1, 0), (2, 2)]);
        assert!(q.finest.is_empty());
        // a coarser level queued after a finer one comes after it (the
        // caller drops it: the texture is finer by then)
        q.push(pending(3, 0), true, 1.0);
        q.push(pending(3, 2), true, 1.0);
        assert_eq!(q.pop().map(|u| u.discard), Some(0));
        assert_eq!(q.pop().map(|u| u.discard), Some(2));
        assert!(q.pop().is_none() && q.finest.is_empty());
    }

    #[test]
    fn requeued_upload_keeps_its_place() {
        let mut q = PendingUploads::default();
        q.push(pending(1, 0), true, 100.0);
        q.push(pending(2, 0), true, 50.0);
        // texture 1 is not ready (its staging chunk is still written)
        let first = q.pop().expect("upload");
        assert_eq!(first.id.as_u128(), 1);
        q.requeue(first);
        q.push(pending(3, 0), true, 75.0);
        let order: Vec<u128> = std::iter::from_fn(|| q.pop()).map(|u| u.id.as_u128()).collect();
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
