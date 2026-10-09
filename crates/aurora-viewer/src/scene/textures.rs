//! Texture streaming: progressive JPEG2000 fetch by byte ranges, disk cache,
//! parallel decode at the discard level matching on-screen size, bindless
//! slot management and GPU upload budgeting.

use super::jobs::{AlphaKind, JobResult, Jobs, SculptMap, classify_alpha, to_rgba};
use aurora_assets::J2kInfo;
use aurora_net::fetch::ByteRange;
use aurora_net::{FetchRequest, FetchResult, Fetcher};
use aurora_render::{MipLevel, Renderer, build_mips};
use std::collections::{HashMap, VecDeque};
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
    data: Vec<u8>,
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
    mips: Vec<(u32, u32, Vec<u8>)>,
    alpha: AlphaKind,
    alpha_channel: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TextureStats {
    pub total: usize,
    pub loaded: usize,
    pub fetching: usize,
    pub decoding: usize,
    pub pending_upload: usize,
    pub uploaded_bytes_frame: u64,
    /// In use, never loaded, and the last fetch failed.
    pub failing: usize,
}

pub struct TextureStreamer {
    entries: HashMap<Uuid, Entry>,
    by_fetch_key: HashMap<u64, Uuid>,
    next_key: u64,
    uploads: VecDeque<PendingUpload>,
    /// Alpha class by bindless slot (fast lookup during list building).
    pub alpha_by_slot: Vec<AlphaKind>,
    /// Per slot: the decoded image has an alpha channel (LL sends such
    /// faces to the alpha pool when rigged or glowing, even if opaque).
    pub alpha_channel_by_slot: Vec<bool>,
    cache_dir: PathBuf,
    /// Bumped whenever a texture's alpha class or a sculpt map changes.
    /// Procedural demo textures: id -> slot.
    local_slots: HashMap<Uuid, u32>,
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
            uploads: VecDeque::new(),
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
            if slot as usize >= self.alpha_by_slot.len() {
                self.alpha_by_slot.resize(slot as usize + 1024, AlphaKind::Opaque);
            }
            self.alpha_by_slot[slot as usize] = super::jobs::classify_alpha(&px);
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
        if slot as usize >= self.alpha_by_slot.len() {
            self.alpha_by_slot.resize(slot as usize + 1024, AlphaKind::Opaque);
        }
        self.alpha_by_slot[slot as usize] = AlphaKind::Opaque;
        self.entries.insert(
            id,
            Entry {
                slot,
                refs: 1,
                source,
                data: Vec::new(),
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
        if slot as usize >= self.alpha_by_slot.len() {
            self.alpha_by_slot.resize(slot as usize + 1024, AlphaKind::Opaque);
        }
        self.alpha_by_slot[slot as usize] = AlphaKind::Opaque;
        if let Some(c) = self.alpha_channel_by_slot.get_mut(slot as usize) {
            *c = false;
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
            ..
        } = self;
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
                let cache_complete_path = Self::cache_path(cache_dir, &id, true);
                let cache_part_path = Self::cache_path(cache_dir, &id, false);
                jobs.spawn(move || {
                    if let Ok(d) = crate::cache::read_touch(&cache_complete_path) {
                        return JobResult::TextureCache {
                            id,
                            data: Some(d),
                            complete: true,
                        };
                    }
                    if let Ok(d) = crate::cache::read_touch(&cache_part_path) {
                        return JobResult::TextureCache {
                            id,
                            data: Some(d),
                            complete: false,
                        };
                    }
                    JobResult::TextureCache {
                        id,
                        data: None,
                        complete: false,
                    }
                });
                continue;
            }
            if e.cache_state == 1 || e.decoding || e.fetching {
                continue;
            }
            let needed = Self::bytes_needed(e);
            let have_enough = !e.data.is_empty() && (e.complete || e.data.len() >= needed);
            let better = match e.decoded {
                None => true,
                Some(d) => e.want < d,
            };
            if !better {
                continue;
            }
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
                jobs.spawn(move || decode_job(id, data, discard, keep));
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
                e.data = bytes;
                e.complete = true;
                Ok(())
            }
            Ok(bytes) => {
                if merge_reply(&mut e.data, r.offset, &bytes) {
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
                        e.data = d;
                        e.complete = complete;
                    }
                }
            }
            JobResult::Texture {
                id,
                discard,
                mips,
                alpha,
                alpha_channel,
                sculpt,
            } => {
                self.decodes = self.decodes.saturating_sub(1);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.decoding = false;
                    if sculpt.is_some() {
                        e.sculpt = sculpt;
                        self.generation += 1;
                    }
                }
                self.uploads.push_back(PendingUpload {
                    id,
                    discard,
                    mips,
                    alpha,
                    alpha_channel,
                });
            }
            JobResult::TextureFailed { id } => {
                self.decodes = self.decodes.saturating_sub(1);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.decoding = false;
                    e.failures += 1;
                    // Corrupt cached data: drop it and refetch.
                    if e.failures >= 2 {
                        e.data.clear();
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
                self.on_job(JobResult::Texture {
                    id,
                    discard: discard as u8,
                    mips: crate::demo::stress_texture_mips(i, discard),
                    alpha: AlphaKind::Opaque,
                    alpha_channel: false,
                    sculpt: None,
                });
            }
        }
    }

    /// Upload decoded textures within a byte budget.
    pub fn upload(&mut self, renderer: &mut Renderer, budget_bytes: u64) {
        let mut spent = 0u64;
        while spent < budget_bytes {
            let Some(u) = self.uploads.pop_front() else {
                break;
            };
            let Some(e) = self.entries.get_mut(&u.id) else {
                continue;
            };
            if e.decoded.is_some_and(|d| d <= u.discard) {
                continue;
            }
            let levels: Vec<MipLevel> = u
                .mips
                .iter()
                .map(|(w, h, d)| MipLevel {
                    width: *w,
                    height: *h,
                    data: d,
                })
                .collect();
            let accepted = renderer.replace_texture(e.slot, &levels);
            if !accepted {
                // keep the current texels, and do not decode it again in a loop
                log::warn!(
                    "texture {}: upload refused by the renderer (slot {}, {} levels, level 0 {:?}, {} bytes)",
                    u.id,
                    e.slot,
                    levels.len(),
                    levels.first().map(|l| (l.width, l.height)),
                    levels.first().map_or(0, |l| l.data.len())
                );
                e.decoded = Some(u.discard);
                e.failures += 1;
            }
            if accepted {
                spent += u.mips.iter().map(|m| m.2.len() as u64).sum::<u64>();
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
                if e.alpha != u.alpha {
                    e.alpha = u.alpha;
                    self.generation += 1;
                }
                if let Some(a) = self.alpha_by_slot.get_mut(e.slot as usize) {
                    *a = u.alpha;
                }
                let slot = e.slot as usize;
                if slot >= self.alpha_channel_by_slot.len() {
                    self.alpha_channel_by_slot.resize(slot + 1024, false);
                }
                if self.alpha_channel_by_slot[slot] != u.alpha_channel {
                    self.alpha_channel_by_slot[slot] = u.alpha_channel;
                    self.generation += 1;
                }
            }
        }
        self.stats.uploaded_bytes_frame = spent;
    }

    /// Evict unused textures and persist downloaded data.
    pub fn maintain(&mut self, renderer: &mut Renderer, jobs: &Jobs, budget_bytes: u64) {
        if self.last_maintenance.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_maintenance = Instant::now();
        let now = Instant::now();
        let mut evict = Vec::new();
        for (id, e) in self.entries.iter_mut() {
            if e.dirty_cache && !e.fetching && !e.data.is_empty() {
                e.dirty_cache = false;
                let complete = e.complete;
                let path = self
                    .cache_dir
                    .join("tex")
                    .join(format!("{id}.{}", if complete { "j2c" } else { "part" }));
                let part = self.cache_dir.join("tex").join(format!("{id}.part"));
                let data = e.data.clone();
                let id = *id;
                jobs.spawn(move || {
                    let tmp = path.with_extension("tmp");
                    if crate::cache::write(&tmp, &data).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                    if complete {
                        let _ = std::fs::remove_file(part);
                    }
                    JobResult::TextureCache { id, data: None, complete }
                });
            }
            if e.refs == 0 && e.unused_since.is_some_and(|t| now.duration_since(t) > self.evict_after) && !e.fetching && !e.decoding {
                evict.push(*id);
            }
        }
        for id in evict {
            if let Some(e) = self.entries.remove(&id) {
                renderer.free_texture(e.slot);
                if let Some(a) = self.alpha_by_slot.get_mut(e.slot as usize) {
                    *a = AlphaKind::Opaque;
                }
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

fn decode_job(id: Uuid, data: Vec<u8>, discard: u8, keep_pixels: bool) -> JobResult {
    match aurora_assets::decode_j2k(&data, discard) {
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
            JobResult::Texture {
                id,
                discard: img.discard,
                mips,
                alpha,
                alpha_channel,
                sculpt,
            }
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
