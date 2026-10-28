//! Mesh assets, downloaded by parts, and GLTF material assets.
//!
//! A mesh drawn below its high level of detail is no longer downloaded
//! whole (every level and its physics, in one request): its header first,
//! then only the blocks that are drawn, a level of detail and the skin, by
//! byte ranges. Port of the requests of LLMeshRepoThread
//! (indra/newview/llmeshrepository.cpp, originally LGPL 2.1):
//! `fetchMeshHeader` asks for the first 4,096 bytes (`MESH_HEADER_SIZE`,
//! the blocks that fit in them come with them), `fetchMeshLOD` and
//! `fetchMeshSkinInfo` the range of one block. Two missing blocks close to
//! each other are asked for in one request.
//!
//! **Different from Firestorm, deliberately:** the first request is sized
//! by the level wanted (`first_request`), and a mesh first wanted at its
//! high level is still downloaded whole, in one request. The blocks of an
//! asset go from the coarsest level to the finest, then the physics and the
//! skin, and the high level is three quarters of the bytes of all meshes:
//! the start of the asset holds the header and the coarser levels.
//! Measured on a local set of 3,000 mesh assets, requests and bytes against
//! the whole assets in one request each: to show the lowest or low level, the first
//! 4 KiB then what is missing take 1.28 and 1.46 requests for 3 and 6 % of
//! the bytes, the first 8 KiB 1.24 and 1.33 for 6 and 8 %; the medium
//! level, 1.85 requests for 22 % with 4 KiB, 1.35 for 32 % with 32 KiB; the
//! high level, 1.86 requests for 80 %: one more round trip before what is
//! near is drawn, for a fifth of its bytes. What is left over one request
//! is mostly the skin of rigged meshes, at the end of the asset. The
//! bytes are parsed by jobs (header, skin): the main thread only keeps
//! them, it no longer copies a whole asset nor parses it in a frame. What
//! is held is kept in the disk cache (`mesh_bytes.rs`).

use super::GeomKey;
use super::fetch_priority::{self, urgent};
use super::jobs::{JobResult, Jobs, mesh_to_faces};
use super::mesh_bytes::{MeshBytes, ranges_to_fetch};
use crate::fast_hash::FastMap;
use aurora_assets::material::PbrOverride;
use aurora_assets::{AssetError, MeshHeader, PbrMaterial, SkinInfo};
use aurora_net::fetch::ByteRange;
use aurora_net::{FetchRequest, FetchResult, Fetcher, Lane};
use aurora_render::StagingPool;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_MESH: u64 = 2 << 60;
pub const FETCH_KIND_MATERIAL: u64 = 3 << 60;

/// The first request of a mesh: the start of the asset, its header and the
/// coarser levels, sized by the finest level wanted (None: the whole
/// asset). See the module comment for the measure behind the sizes.
fn first_request(finest_wanted: u8) -> Option<usize> {
    match finest_wanted {
        0 | 1 => Some(8 << 10),
        2 => Some(32 << 10),
        _ => None,
    }
}
/// Two missing blocks this close are asked for in one request.
const JOIN_GAP: usize = 4096;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Stage {
    /// Not looked for yet.
    #[default]
    New,
    /// Its disk cache file is being read.
    Reading,
    /// Its header is to be downloaded.
    ToFetch,
    /// Its header is downloading.
    Fetching,
    /// Its header is known: its blocks are downloaded as they are wanted.
    Known,
    /// Its header did not come, or a block failed again and again.
    Failed,
    /// No object wanted it before its header came: wanted again, it
    /// starts over.
    Unwanted,
}

#[derive(Default)]
struct MeshEntry {
    /// What is held of the asset.
    bytes: MeshBytes,
    header: Option<MeshHeader>,
    skin: Option<Arc<SkinInfo>>,
    /// Its skin block did not decode: built without it, as when the whole
    /// asset came.
    skin_bad: bool,
    stage: Stage,
    /// Held in memory only (imports, demo fixtures): never downloaded.
    local: bool,
    /// The header is larger than the first request: the whole asset is asked.
    whole: bool,
    retry_at: Option<Instant>,
    failures: u32,
    /// The asset server does not have it, or what it sent is not a mesh:
    /// asked again much later, and not expected meanwhile.
    gone: bool,
    wanted_lods: HashSet<u8>,
    wanted_physics: bool,
    building: HashSet<u8>,
    /// Listed in `MeshStreamer::todo`.
    queued: bool,
    /// Size on screen of its largest user, in pixels, and whether one is
    /// ours (`MeshStreamer::need`): the rank of its downloads.
    px: f32,
    own: bool,
    /// Its downloads under way (key, range asked), and the rank they were
    /// last given.
    inflight: Vec<(u64, Option<(usize, usize)>)>,
    sent: f32,
}

impl MeshEntry {
    fn lane(&self) -> (Lane, f32) {
        if self.own {
            (Lane::Urgent, urgent::OWN)
        } else {
            (Lane::Bulk, fetch_priority::mesh(self.px))
        }
    }

    /// Header and skin known: placed and built as a whole asset was.
    fn ready(&self) -> bool {
        self.header
            .as_ref()
            .is_some_and(|h| h.skin.is_none() || self.skin.is_some() || self.skin_bad)
    }

    /// The block drawn for a level of detail (`actual_lod`).
    fn lod_block(&self, lod: u8) -> Option<(usize, usize)> {
        let h = self.header.as_ref()?;
        h.actual_lod(lod as usize).and_then(|a| h.lods[a])
    }

    /// Blocks wanted and not held: the skin, those of the wanted levels.
    fn missing(&self) -> Vec<(usize, usize)> {
        let Some(h) = &self.header else {
            return Vec::new();
        };
        let skin = h.skin.filter(|_| self.skin.is_none() && !self.skin_bad);
        skin.into_iter()
            .chain(self.wanted_lods.iter().filter_map(|&lod| self.lod_block(lod)))
            .chain(self.wanted_physics.then_some(h.physics_convex).flatten())
            .chain(self.wanted_physics.then_some(h.physics_mesh).flatten())
            .filter(|b| !self.bytes.holds(*b))
            .collect()
    }

    /// A missing block that no download under way brings.
    fn uncovered(&self) -> Vec<(usize, usize)> {
        let mut missing = self.missing();
        missing.retain(|&(start, len)| {
            !self.inflight.iter().any(|(_, r)| match r {
                None => true,
                Some((s, l)) => *s <= start && s + l >= start + len,
            })
        });
        missing
    }

    /// A wanted level whose block is held, once the skin is known.
    fn buildable(&self) -> bool {
        self.ready()
            && self
                .wanted_lods
                .iter()
                .any(|&lod| self.lod_block(lod).is_some_and(|b| self.bytes.holds(b)))
    }

    /// `MeshStreamer::update` has something to do for it, now or at its
    /// retry time: a new mesh to read from the disk cache, a header to
    /// download, a block to download or a level to build.
    fn has_work(&self) -> bool {
        match self.stage {
            Stage::New | Stage::ToFetch => true,
            Stage::Known => self.buildable() || (!self.local && !self.uncovered().is_empty()),
            Stage::Failed => self.failures <= 3,
            Stage::Reading | Stage::Fetching | Stage::Unwanted => false,
        }
    }
}

/// What a job made of mesh bytes read from the disk or downloaded: the
/// bytes, and the header and skin found in them.
pub struct MeshArrival {
    bytes: MeshBytes,
    header: Option<MeshHeader>,
    skin: Option<Result<SkinInfo, String>>,
    /// Not a mesh: why.
    invalid: Option<String>,
    /// The header goes past the bytes held (larger than the first request).
    truncated: bool,
}

/// Parse what `bytes` holds: the header when they start the asset and it is
/// not known yet (`header`), the skin when wanted and held.
fn arrival(bytes: MeshBytes, header: Option<MeshHeader>, want_skin: bool) -> MeshArrival {
    let mut a = MeshArrival {
        bytes,
        header: None,
        skin: None,
        invalid: None,
        truncated: false,
    };
    let header = match header {
        Some(h) => Some(h),
        None => match a.bytes.first().map(aurora_assets::parse_mesh_header) {
            Some(Ok(h)) => {
                a.header = Some(h.clone());
                Some(h)
            }
            Some(Err(AssetError::Truncated(_))) if !a.bytes.is_whole() => {
                a.truncated = true;
                None
            }
            Some(Err(e)) => {
                a.invalid = Some(e.to_string());
                None
            }
            None => None,
        },
    };
    if want_skin
        && let Some(range) = header.and_then(|h| h.skin)
        && let Some(section) = a.bytes.section(range)
    {
        a.skin = Some(aurora_assets::decode_skin(section).map_err(|e| e.to_string()));
    }
    a
}

/// Info about a mesh needed for placement.
#[derive(Clone)]
pub struct MeshMeta {
    pub skin: Option<Arc<SkinInfo>>,
    /// Byte size of each LOD (lowest .. high), for the render cost.
    pub lod_bytes: [u32; 4],
}

/// What is held of a mesh, for the readers of its geometry outside the
/// scene (feet on rigged shoes, media faces).
#[derive(Clone)]
pub struct MeshView {
    pub header: MeshHeader,
    pub bytes: MeshBytes,
    pub skin: Option<Arc<SkinInfo>>,
}

impl MeshView {
    /// The finest level of detail held, at most `lod`, else the coarsest
    /// held above it.
    pub fn lod_section(&self, lod: usize) -> Option<&[u8]> {
        let held = |l: usize| self.header.lods[l].and_then(|r| self.bytes.section(r));
        (0..=lod.min(3)).rev().find_map(held).or_else(|| (lod.min(3) + 1..4).find_map(held))
    }
}

/// Disk cache file of a mesh asset; None without a disk cache (an empty
/// `cache_dir`).
fn cache_path(cache_dir: &std::path::Path, id: &Uuid) -> Option<PathBuf> {
    (!cache_dir.as_os_str().is_empty()).then(|| cache_dir.join("mesh").join(format!("{id}.mesh")))
}

pub struct MeshStreamer {
    entries: FastMap<Uuid, MeshEntry>,
    /// The meshes `update` has something to do for (`MeshEntry::has_work`),
    /// each listed once. It only visits these: a region leaves thousands of
    /// loaded meshes behind, and walking them all at each frame cost 0.13 ms
    /// for good after one visit to a busy region (7,700 meshes).
    todo: Vec<Uuid>,
    by_key: FastMap<u64, Uuid>,
    next_key: u64,
    /// Downloads to rank again, or to drop (no object wants them any more),
    /// told to the fetcher at the next `update`.
    moved: Vec<(u64, f32)>,
    dropped: Vec<u64>,
    cache_dir: PathBuf,
    pub fetching: usize,
    /// Failures written to the log (the first ones, then one in 50).
    logged_failures: u32,
    /// Bumped whenever a mesh's metadata (header, skin) arrives: skeletons
    /// using worn meshes check their joint offsets again.
    pub meta_generation: u64,
    /// The renderer's staging memory, given to the LOD jobs.
    pub staging: Option<Arc<StagingPool>>,
    /// Bytes of mesh assets downloaded, and requests sent (`counts`).
    pub downloaded: u64,
    pub requests: u64,
    /// When `mesh load:` was last written, and the requests it told.
    logged_at: Option<Instant>,
    logged_requests: u64,
}

impl MeshStreamer {
    /// `cache_dir` empty: no disk cache (tests, which run in the crate's
    /// folder: a relative `mesh/` wrote cache files among the sources).
    pub fn new(cache_dir: PathBuf) -> Self {
        if !cache_dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(cache_dir.join("mesh"));
        }
        MeshStreamer {
            entries: FastMap::default(),
            todo: Vec::new(),
            by_key: FastMap::default(),
            next_key: 1,
            moved: Vec::new(),
            dropped: Vec::new(),
            cache_dir,
            fetching: 0,
            logged_failures: 0,
            meta_generation: 0,
            staging: None,
            downloaded: 0,
            requests: 0,
            logged_at: None,
            logged_requests: 0,
        }
    }

    /// Ask for a LOD of a mesh; geometry arrives later as `JobResult::Geometry`.
    pub fn want(&mut self, id: Uuid, lod: u8) {
        let e = self.entries.entry(id).or_default();
        e.wanted_lods.insert(lod);
        if e.stage == Stage::Unwanted {
            e.stage = Stage::ToFetch;
        }
        Self::attend(&mut self.todo, id, e);
    }

    /// Fetch collision blocks only while the build inspector needs them.
    pub fn want_physics(&mut self, id: Uuid, wanted: bool) {
        let e = self.entries.entry(id).or_default();
        e.wanted_physics = wanted;
        if wanted && e.stage == Stage::Unwanted {
            e.stage = Stage::ToFetch;
        }
        Self::attend(&mut self.todo, id, e);
    }

    /// An object using a mesh covers `px` pixels on screen (`own`: worn by
    /// our avatar). Its downloads still waiting move up to the nearest user;
    /// ours go in the urgent lane.
    pub fn need(&mut self, id: Uuid, px: f32, own: bool) {
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        if px <= e.px && (e.own || !own) {
            return;
        }
        let was = e.lane();
        e.px = e.px.max(px);
        e.own |= own;
        let (lane, priority) = e.lane();
        if e.inflight.is_empty() {
            return;
        }
        if lane != was.0 {
            // (a new request under the same key replaces the waiting one)
            self.moved.extend(e.inflight.iter().map(|(key, _)| (*key, f32::NAN)));
        } else if fetch_priority::changed(e.sent, priority) {
            e.sent = priority;
            self.moved.extend(e.inflight.iter().map(|(key, _)| (*key, priority)));
        }
    }

    /// No object uses the mesh any more (the scene let all its geometries
    /// go). Its downloads still waiting for a slot are dropped, as Firestorm
    /// drops pending mesh requests whose volumes are gone; those already
    /// sent end, and are ignored. Wanted again, what is missing is asked
    /// again.
    pub fn unwant(&mut self, id: &Uuid) {
        let Some(e) = self.entries.get_mut(id) else {
            return;
        };
        e.wanted_lods.clear();
        e.wanted_physics = false;
        e.px = 0.0;
        e.own = false;
        for (key, _) in e.inflight.drain(..) {
            self.by_key.remove(&key);
            self.dropped.push(key);
        }
        if matches!(e.stage, Stage::Fetching | Stage::ToFetch) {
            e.stage = Stage::Unwanted;
        }
    }

    /// List a mesh for `update`, once.
    fn attend(todo: &mut Vec<Uuid>, id: Uuid, e: &mut MeshEntry) {
        if !std::mem::replace(&mut e.queued, true) {
            todo.push(id);
        }
    }

    /// What is held of a mesh, once its header is known.
    pub fn view(&self, id: &Uuid) -> Option<MeshView> {
        let e = self.entries.get(id)?;
        Some(MeshView {
            header: e.header.clone()?,
            bytes: e.bytes.clone(),
            skin: e.skin.clone(),
        })
    }

    pub fn meta(&self, id: &Uuid) -> Option<MeshMeta> {
        let e = self.entries.get(id).filter(|e| e.ready())?;
        let h = e.header.as_ref()?;
        let lod_bytes = h.lods.map(|l| l.map(|(_, size)| size as u32).unwrap_or(0));
        Some(MeshMeta {
            skin: e.skin.clone(),
            lod_bytes,
        })
    }

    /// What undoes the bind shape a skinned mesh's geometry holds
    /// (`build_mesh_lod`), for an object that draws it without a skeleton;
    /// None for a mesh without a skin (or not known yet) and for a bind
    /// shape that cannot be undone.
    pub fn unbind(&self, id: &Uuid) -> Option<glam::Mat4> {
        let e = self.entries.get(id).filter(|e| e.ready())?;
        let bind = e.skin.as_ref()?.bind_shape;
        (bind.determinant().abs() > 1e-12).then(|| bind.inverse())
    }

    /// In-memory meshes for local previews and offline fixtures; no asset fetch.
    pub fn insert_local(&mut self, id: Uuid, data: Arc<Vec<u8>>, header: MeshHeader, skin: Option<Arc<SkinInfo>>) {
        self.entries.insert(
            id,
            MeshEntry {
                bytes: MeshBytes::whole(data),
                header: Some(header),
                skin,
                stage: Stage::Known,
                local: true,
                ..Default::default()
            },
        );
        self.meta_generation += 1;
    }
    pub fn remove_local(&mut self, id: &Uuid) {
        if self.entries.remove(id).is_some() {
            self.meta_generation += 1;
        }
    }

    /// Live snapshots can hold large CPU buffers too: release their last
    /// allocation off the frame thread once geometry retirement is safe.
    pub fn retire_local(&mut self, id: &Uuid, jobs: &Jobs) {
        if let Some(entry) = self.entries.remove(id) {
            self.meta_generation += 1;
            jobs.discard(entry);
        }
    }

    pub fn insert_demo_skin(&mut self, id: Uuid, skin: SkinInfo) {
        self.meta_generation += 1;
        self.entries.insert(
            id,
            MeshEntry {
                skin: Some(Arc::new(skin)),
                header: Some(MeshHeader {
                    header_size: 0,
                    lods: [Some((0, 1)); 4],
                    skin: None,
                    physics_convex: None,
                    physics_mesh: None,
                    version: 1,
                }),
                stage: Stage::Known,
                local: true,
                ..Default::default()
            },
        );
    }

    /// AURORA_DEMO_CLOUD=self: a mesh the asset server answered 404 for, as
    /// `on_fetch` leaves it (no asset fetch offline).
    pub fn insert_demo_unavailable(&mut self, id: Uuid) {
        self.entries.entry(id).or_default();
        self.fail(id, Duration::from_secs(600), true);
    }

    pub fn insert_demo_static(&mut self, id: Uuid) {
        self.insert_demo_skin(id, SkinInfo::default());
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.skin = None;
        }
    }

    /// The mesh asset is not expected any more: missing on the asset server
    /// or invalid (LLMeshHeaderHandler::processFailure marks every LOD
    /// unavailable at once), or its download failed again and again.
    pub fn failed(&self, id: &Uuid) -> bool {
        self.entries
            .get(id)
            .is_some_and(|e| e.stage == Stage::Failed && (e.gone || e.failures > 3))
    }

    /// Diagnostic: the state of a mesh asset, without requesting it.
    pub fn describe(&self, id: &Uuid) -> String {
        let Some(e) = self.entries.get(id) else {
            return "not requested".into();
        };
        let state = match e.stage {
            Stage::New => "new",
            Stage::Reading => "reading the disk cache",
            Stage::ToFetch => "waiting to download",
            Stage::Fetching => "downloading its header",
            Stage::Known if !e.inflight.is_empty() => "downloading blocks",
            Stage::Known if e.ready() => "loaded",
            Stage::Known => "waiting for its skin",
            Stage::Failed if e.gone => "unavailable",
            Stage::Failed => "download failed",
            Stage::Unwanted => "not wanted",
        };
        let retry = e
            .retry_at
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
            .filter(|s| *s > 0)
            .map(|s| format!(", retry in {s} s"))
            .unwrap_or_default();
        let mut building: Vec<u8> = e.building.iter().copied().collect();
        building.sort_unstable();
        format!(
            "{state}, {} bytes held, failures {}{retry}, LODs being built {building:?}",
            e.bytes.held(),
            e.failures
        )
    }

    fn request(key: u64, base: &str, id: &Uuid, range: Option<(usize, usize)>, lane: Lane, priority: f32) -> FetchRequest {
        FetchRequest {
            key,
            url: super::textures::asset_url(base, "mesh_id", id),
            range: range.map(|(start, len)| ByteRange {
                start: start as u64,
                end: Some((start + len).saturating_sub(1) as u64),
            }),
            priority,
            lane,
            accept: "application/vnd.ll.mesh",
        }
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>, rig: &Arc<super::anim::Rig>) {
        let now = Instant::now();
        for key in self.dropped.drain(..) {
            fetcher.cancel(key);
        }
        for (key, priority) in self.moved.drain(..) {
            let Some(id) = self.by_key.get(&key).copied() else {
                continue;
            };
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            if priority.is_nan() {
                // to the other lane: asked again under its key
                if let (Some(base), Some(&(_, range))) = (viewer_asset, e.inflight.iter().find(|(k, _)| *k == key)) {
                    let (lane, priority) = e.lane();
                    e.sent = priority;
                    fetcher.request(Self::request(key, base, &id, range, lane, priority));
                }
            } else {
                fetcher.reprioritise(key, priority);
            }
        }
        let mut todo = std::mem::take(&mut self.todo);
        for id in todo.drain(..) {
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            e.queued = false;
            // (a block: whatever stops the visit, the mesh is listed again
            // below while it has work left)
            'visit: {
                if e.retry_at.is_some_and(|t| now < t) {
                    break 'visit;
                }
                match e.stage {
                    Stage::New => {
                        e.stage = Stage::Reading;
                        let path = cache_path(&self.cache_dir, &id);
                        jobs.spawn(move || {
                            let bytes = path
                                .and_then(|path| crate::cache::read_touch(path).ok())
                                .and_then(MeshBytes::decode);
                            crate::deep_profile::mesh_cache_read(bytes.is_some());
                            let timer = crate::deep_profile::timer();
                            let arrival = bytes.map(|bytes| Box::new(arrival(bytes, None, true)));
                            crate::deep_profile::mesh_parsed(timer.filter(|_| arrival.is_some()));
                            JobResult::MeshCache { id, arrival }
                        });
                        break 'visit;
                    }
                    Stage::Failed if e.failures <= 3 => {
                        e.retry_at = None;
                        e.stage = if e.header.is_some() { Stage::Known } else { Stage::ToFetch };
                    }
                    _ => {}
                }
                let Some(base) = viewer_asset.filter(|_| !e.local) else {
                    // (without the capability yet: asked again at the next frame)
                    if e.stage == Stage::Known {
                        Self::build(jobs, id, e, rig, self.staging.as_ref());
                    }
                    break 'visit;
                };
                if e.stage == Stage::ToFetch {
                    // the start of the asset (fetchMeshHeader), or the whole
                    // asset when its high level is wanted (see above) or its
                    // header is larger than that start
                    let finest = e.wanted_lods.iter().copied().max().unwrap_or(0);
                    let range = first_request(finest).filter(|_| !e.whole).map(|len| (0, len));
                    let key = FETCH_KIND_MESH | self.next_key;
                    self.next_key += 1;
                    self.by_key.insert(key, id);
                    self.requests += 1;
                    e.stage = Stage::Fetching;
                    e.inflight.push((key, range));
                    let (lane, priority) = e.lane();
                    e.sent = priority;
                    fetcher.request(Self::request(key, base, &id, range, lane, priority));
                    break 'visit;
                }
                if e.stage == Stage::Known {
                    // the blocks drawn (fetchMeshLOD, fetchMeshSkinInfo)
                    let uncovered = e.uncovered();
                    if !uncovered.is_empty() {
                        let (lane, priority) = e.lane();
                        e.sent = priority;
                        for range in ranges_to_fetch(&e.bytes, &uncovered, JOIN_GAP) {
                            let key = FETCH_KIND_MESH | self.next_key;
                            self.next_key += 1;
                            self.by_key.insert(key, id);
                            self.requests += 1;
                            e.inflight.push((key, Some(range)));
                            fetcher.request(Self::request(key, base, &id, Some(range), lane, priority));
                        }
                    }
                    Self::build(jobs, id, e, rig, self.staging.as_ref());
                }
            }
            if e.has_work() {
                Self::attend(&mut self.todo, id, e);
            }
        }
        // (the emptied list keeps its room when nothing was listed again)
        if self.todo.is_empty() {
            self.todo = todo;
        }
        // one entry per download under way
        self.fetching = self.by_key.len();
        // `mesh load:` every 10 s while meshes download (the bytes of an
        // arrival beside those of the `texture load:` line)
        if self.requests != self.logged_requests && self.logged_at.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(10)) {
            self.logged_at = Some(now);
            self.logged_requests = self.requests;
            let (ready, fetching, failed) = self.counts();
            log::info!(
                "mesh load: {} requests, {:.1} MB downloaded since the start; {} meshes loaded, {} downloading, {} failed",
                self.requests,
                self.downloaded as f64 / 1e6,
                ready,
                fetching,
                failed
            );
        }
    }

    /// Build the wanted levels whose block is held, once the skin is known.
    fn build(jobs: &Jobs, id: Uuid, e: &mut MeshEntry, rig: &Arc<super::anim::Rig>, staging: Option<&Arc<StagingPool>>) {
        if !e.ready() {
            return;
        }
        let Some(header) = e.header.clone() else {
            return;
        };
        let lods: Vec<u8> = e
            .wanted_lods
            .iter()
            .copied()
            .filter(|&lod| e.lod_block(lod).is_some_and(|b| e.bytes.holds(b)))
            .collect();
        for lod in lods {
            e.wanted_lods.remove(&lod);
            if !e.building.insert(lod) {
                continue;
            }
            let skin = e.skin.clone();
            let header = header.clone();
            let bytes = e.bytes.clone();
            let rig = rig.clone();
            let pool = staging.cloned();
            jobs.spawn(move || {
                let timer = crate::deep_profile::timer();
                let built = build_mesh_lod(id, lod, &bytes, &header, skin, &rig, pool.as_deref());
                crate::deep_profile::mesh_built(timer);
                built
            });
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        e.inflight.retain(|(k, _)| *k != r.key);
        match r.data {
            Ok(data) if !data.is_empty() => {
                self.downloaded += data.len() as u64;
                // (parsed by a job: header and skin; the main thread only
                // keeps the bytes, `on_part`)
                let data = Arc::new(data);
                let bytes = if r.offset == 0 && r.status == 200 {
                    MeshBytes::whole(data)
                } else {
                    let mut b = MeshBytes::default();
                    b.add(r.offset as usize, data);
                    b
                };
                let header = e.header.clone();
                let want_skin = e.skin.is_none() && !e.skin_bad;
                jobs.spawn(move || {
                    let timer = crate::deep_profile::timer();
                    let arrival = Box::new(arrival(bytes, header, want_skin));
                    crate::deep_profile::mesh_parsed(timer);
                    JobResult::MeshPart { id, arrival }
                });
            }
            _ => {
                let why = match &r.data {
                    Err(e) => e.clone(),
                    Ok(_) => "empty response".to_owned(),
                };
                self.log_failure(format_args!("mesh {id}: download failed (HTTP {}, {why})", r.status));
                let missing = r.status == 404;
                let backoff = if missing { 600 } else { 5 };
                self.fail(id, Duration::from_secs(backoff), missing);
            }
        }
    }

    /// Bytes downloaded, parsed by their job.
    pub fn on_part(&mut self, id: Uuid, a: MeshArrival, jobs: &Jobs) {
        if self.keep(id, a, true) {
            let (Some(e), Some(path)) = (self.entries.get(&id), cache_path(&self.cache_dir, &id)) else {
                return;
            };
            let bytes = e.bytes.clone();
            jobs.spawn(move || {
                let _ = bytes.save(&path);
                JobResult::Done
            });
        }
    }

    /// The disk cache file of a mesh, parsed by its job (None: not there,
    /// or damaged).
    pub fn on_cache(&mut self, id: Uuid, a: Option<MeshArrival>) {
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        if e.stage != Stage::Reading {
            return;
        }
        let kept = a.is_some_and(|a| self.keep(id, a, false));
        if let Some(e) = self.entries.get_mut(&id) {
            if !kept || e.header.is_none() {
                e.stage = Stage::ToFetch;
            }
            Self::attend(&mut self.todo, id, e);
        }
    }

    /// Keep what a job made of bytes; false when they were not a mesh.
    fn keep(&mut self, id: Uuid, a: MeshArrival, downloaded: bool) -> bool {
        let Some(e) = self.entries.get_mut(&id) else {
            return false;
        };
        if let Some(why) = a.invalid {
            if downloaded {
                self.log_failure(format_args!("mesh {id}: invalid asset ({why})"));
                // not a mesh: it will not get better
                self.fail(id, Duration::from_secs(120), true);
            }
            return false;
        }
        if a.truncated {
            // a header larger than the first request (Firestorm assumes
            // 4 KiB is enough): the whole asset
            e.whole = true;
            e.stage = Stage::ToFetch;
            Self::attend(&mut self.todo, id, e);
            return false;
        }
        if a.bytes.is_whole() {
            e.bytes = a.bytes;
        } else {
            for (start, part) in a.bytes.into_parts() {
                e.bytes.add(start, part);
            }
        }
        if let Some(h) = a.header {
            e.header = Some(h);
            self.meta_generation += 1;
        }
        if e.header.is_some() && matches!(e.stage, Stage::Reading | Stage::Fetching | Stage::ToFetch | Stage::Failed) {
            e.stage = Stage::Known;
        }
        match a.skin {
            Some(Ok(s)) => {
                e.skin = Some(Arc::new(s));
                self.meta_generation += 1;
            }
            Some(Err(why)) => {
                log::debug!("mesh {id} skin: {why}");
                e.skin_bad = true;
            }
            None => {}
        }
        e.failures = 0;
        e.gone = false;
        e.retry_at = None;
        Self::attend(&mut self.todo, id, e);
        true
    }

    fn log_failure(&mut self, msg: std::fmt::Arguments) {
        self.logged_failures += 1;
        let n = self.logged_failures;
        if n <= 20 || n.is_multiple_of(50) {
            log::warn!("{msg} [failure #{n}]");
        }
    }

    /// (ready, fetching, failed) mesh assets.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for e in self.entries.values() {
            if e.stage == Stage::Fetching || !e.inflight.is_empty() {
                c.1 += 1;
            } else if e.stage == Stage::Known && e.ready() {
                c.0 += 1;
            } else if e.stage == Stage::Failed {
                c.2 += 1;
            }
        }
        c
    }

    /// A download that failed, asked again after `backoff`; `gone` when the
    /// asset is missing or invalid (see `failed`). A block that failed more
    /// than three times leaves the mesh failed, as its header does.
    fn fail(&mut self, id: Uuid, backoff: Duration, gone: bool) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.failures += 1;
            e.gone = gone;
            e.retry_at = Some(Instant::now() + backoff);
            if e.header.is_none() || gone || e.failures > 3 {
                e.stage = Stage::Failed;
            }
            Self::attend(&mut self.todo, id, e);
        }
    }

    pub fn on_geometry_done(&mut self, id: &Uuid, lod: u8) {
        if let Some(e) = self.entries.get_mut(id) {
            e.building.remove(&lod);
        }
    }
}

fn build_mesh_lod(
    id: Uuid,
    lod: u8,
    bytes: &MeshBytes,
    header: &MeshHeader,
    skin: Option<Arc<SkinInfo>>,
    rig: &super::anim::Rig,
    pool: Option<&StagingPool>,
) -> JobResult {
    let key = GeomKey::Mesh { id, lod };
    let Some(actual) = header.actual_lod(lod as usize) else {
        return JobResult::GeometryFailed { key };
    };
    let Some(range) = header.lods.get(actual).copied().flatten() else {
        return JobResult::GeometryFailed { key };
    };
    let Some(section) = bytes.section(range) else {
        return JobResult::GeometryFailed { key };
    };
    match aurora_assets::decode_mesh_lod(section) {
        Ok(faces) => {
            // Rigged meshes are shown in bind pose: apply the bind shape matrix
            // (avatar skeleton space). Static meshes keep normalized object space.
            let xform = skin.as_ref().map(|s| s.bind_shape);
            // joints stay mesh-local: the record's skin bindings map them to the
            // owner's skeleton with this mesh's inverse bind matrices
            let _ = rig;
            let map: Option<Vec<u8>> = skin
                .as_ref()
                .filter(|s| !s.joint_names.is_empty())
                .map(|s| (0..s.joint_names.len()).map(|i| i.min(255) as u8).collect());
            JobResult::geometry(key, mesh_to_faces(&faces, xform, map.as_deref()), 1.0, pool)
        }
        Err(e) => {
            log::debug!("mesh {id} lod {lod}: {e}");
            JobResult::GeometryFailed { key }
        }
    }
}

// ------------------------------------------------------------------ materials

/// Downloads of a material before it is given up (transient errors; a
/// missing asset is given up at once).
const MATERIAL_ATTEMPTS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MatState {
    New,
    /// Reading the disk cache.
    Cache,
    Fetching,
    /// Waiting to (re)download (`retry_at`).
    Retry,
    Ready,
    /// Given up: missing on the asset server, unreadable, or failing.
    Missing,
}

struct MatEntry {
    material: Option<Arc<PbrMaterial>>,
    state: MatState,
    retry_at: Option<Instant>,
    failures: u32,
    /// HTTP status of the last failed download (0: none, or a network
    /// error), for the diagnostics.
    last_status: u16,
    /// Listed in `MaterialStreamer::todo`.
    queued: bool,
}

impl MatEntry {
    fn new() -> Self {
        MatEntry {
            material: None,
            state: MatState::New,
            retry_at: None,
            failures: 0,
            last_status: 0,
            queued: false,
        }
    }

    /// `MaterialStreamer::update` has something to do for it, now or at its
    /// retry time.
    fn has_work(&self) -> bool {
        matches!(self.state, MatState::New | MatState::Retry)
    }
}

/// Where a GLTF material asset stands for the faces using it.
#[derive(Clone, Debug)]
pub enum MaterialStatus {
    Loaded(Arc<PbrMaterial>),
    /// Being read from the disk cache or downloaded.
    Pending,
    /// Given up (missing asset, invalid asset, repeated failures).
    Missing,
}

#[cfg(test)]
impl MaterialStatus {
    fn is_loaded(&self) -> bool {
        matches!(self, MaterialStatus::Loaded(_))
    }
}

/// The material a face with a GLTF material id is drawn with. Port of
/// LLViewerObject::updateTEMaterialTextures / initRenderMaterial and
/// LLGLTFMaterialList::getMaterial (indra/newview, originally LGPL 2.1): such a
/// face is a glTF face from the start and never shows its texture entry's
/// diffuse texture (LLVOVolume even drops it from texture streaming). While
/// the asset loads, the face has LLFetchedGLTFMaterial's default values
/// (white) and its override waits (setTEGLTFMaterialOverride queues it until
/// the fetch completes); an asset that cannot be loaded keeps the default
/// values, with the override on top. Clothing textured through overrides
/// (PRIM_GLTF_BASE_COLOR on a blank material) thus shows the override's
/// texture even when the material asset itself is unavailable.
pub fn render_material(status: &MaterialStatus, ov: Option<&PbrOverride>) -> PbrMaterial {
    let mut m = match status {
        MaterialStatus::Loaded(base) => (**base).clone(),
        MaterialStatus::Pending => return PbrMaterial::default(),
        MaterialStatus::Missing => PbrMaterial::default(),
    };
    if let Some(ov) = ov {
        m.apply_override(ov);
    }
    m
}

pub struct MaterialStreamer {
    entries: HashMap<Uuid, MatEntry>,
    /// The materials `update` has something to do for (`MatEntry::has_work`),
    /// each listed once: it visits these, not every material known.
    todo: Vec<Uuid>,
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    /// Bumped whenever a material arrives or is given up: the faces using
    /// it are rebuilt.
    pub generation: u64,
    /// Materials whose state changed with the generation, since the scene
    /// last took them (mat_changes.rs).
    changed: Vec<Uuid>,
}

/// BLANK_MATERIAL_ASSET_ID has the default glTF appearance without a fetch.
pub const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

impl MaterialStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("mat"));
        MaterialStreamer {
            entries: HashMap::new(),
            todo: Vec::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            generation: 0,
            changed: Vec::new(),
        }
    }

    /// The materials changed since the last call (mat_changes.rs).
    pub fn take_changed(&mut self, into: &mut Vec<Uuid>) {
        into.append(&mut self.changed);
    }

    /// The material if it is loaded; requests it otherwise.
    pub fn get(&mut self, id: &Uuid) -> Option<Arc<PbrMaterial>> {
        self.entry(id).material.clone()
    }

    /// Where the material stands; requests it when it is new.
    pub fn resolve(&mut self, id: &Uuid) -> MaterialStatus {
        Self::status_of(self.entry(id))
    }

    /// The entry of a material, new and listed for `update` when unknown.
    fn entry(&mut self, id: &Uuid) -> &mut MatEntry {
        let e = self.entries.entry(*id).or_insert_with(|| {
            let mut entry = MatEntry::new();
            if *id == BLANK_MATERIAL {
                entry.material = Some(Arc::new(PbrMaterial::default()));
                entry.state = MatState::Ready;
            }
            entry
        });
        if e.has_work() {
            Self::attend(&mut self.todo, *id, e);
        }
        e
    }

    /// List a material for `update`, once.
    fn attend(todo: &mut Vec<Uuid>, id: Uuid, e: &mut MatEntry) {
        if !std::mem::replace(&mut e.queued, true) {
            todo.push(id);
        }
    }

    fn status_of(e: &MatEntry) -> MaterialStatus {
        match (&e.material, e.state) {
            (Some(m), _) => MaterialStatus::Loaded(m.clone()),
            (None, MatState::Missing) => MaterialStatus::Missing,
            _ => MaterialStatus::Pending,
        }
    }

    /// A material known locally (offline demo).
    pub fn insert(&mut self, id: Uuid, m: PbrMaterial) {
        let mut e = MatEntry::new();
        e.material = Some(Arc::new(m));
        e.state = MatState::Ready;
        self.entries.insert(id, e);
        self.generation += 1;
        self.changed.push(id);
    }

    /// The material if it is loaded, without requesting it (diagnostics).
    pub fn remove_local(&mut self, id: &Uuid) {
        if self.entries.remove(id).is_some() {
            self.generation += 1;
            self.changed.push(*id);
        }
    }

    pub fn peek(&self, id: &Uuid) -> Option<Arc<PbrMaterial>> {
        self.entries.get(id).and_then(|e| e.material.clone())
    }

    /// Diagnostic: the state of a material, without requesting it.
    pub fn describe(&self, id: &Uuid) -> String {
        let Some(e) = self.entries.get(id) else {
            return "not requested".into();
        };
        let retry = e
            .retry_at
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
            .filter(|s| *s > 0)
            .map(|s| format!(", retry in {s} s"))
            .unwrap_or_default();
        match e.state {
            MatState::Ready => "loaded".into(),
            MatState::New | MatState::Cache => "reading the disk cache".into(),
            MatState::Fetching => format!("downloading (failures {})", e.failures),
            MatState::Retry => format!(
                "waiting to download (failures {}, last status {}{retry})",
                e.failures, e.last_status
            ),
            MatState::Missing if e.last_status == 0 && e.failures == 0 => "missing: invalid asset".into(),
            MatState::Missing => format!("missing: HTTP {} after {} attempt(s)", e.last_status, e.failures),
        }
    }

    /// (loaded, pending, missing) materials, for the streaming summary.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut c = (0, 0, 0);
        for e in self.entries.values() {
            match e.state {
                MatState::Ready => c.0 += 1,
                MatState::Missing => c.2 += 1,
                _ => c.1 += 1,
            }
        }
        c
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        let now = Instant::now();
        let mut todo = std::mem::take(&mut self.todo);
        for id in todo.drain(..) {
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            e.queued = false;
            if !e.retry_at.is_some_and(|t| now < t) {
                match e.state {
                    MatState::New => {
                        e.state = MatState::Cache;
                        let path = self.cache_dir.join("mat").join(format!("{id}.mat"));
                        jobs.spawn(move || {
                            let material = crate::cache::read_touch(path)
                                .ok()
                                .and_then(|d| aurora_assets::parse_material_asset(&d).ok());
                            JobResult::Material { id, material }
                        });
                    }
                    // (without the capability yet: asked again at the next frame)
                    MatState::Retry => {
                        if let Some(base) = viewer_asset {
                            let key = FETCH_KIND_MATERIAL | self.next_key;
                            self.next_key += 1;
                            self.by_key.insert(key, id);
                            e.state = MatState::Fetching;
                            e.retry_at = None;
                            // LLGLTFMaterialList::getMaterial -> LLViewerAssetStorage
                            // (ViewerAsset capability, `material_id=`)
                            fetcher.request(FetchRequest {
                                key,
                                url: super::textures::asset_url(base, "material_id", &id),
                                range: None,
                                priority: urgent::MATERIAL,
                                lane: Lane::Urgent,
                                accept: "*/*",
                            });
                        }
                    }
                    _ => {}
                }
            }
            if e.has_work() {
                Self::attend(&mut self.todo, id, e);
            }
        }
        // (the emptied list keeps its room when nothing was listed again)
        if self.todo.is_empty() {
            self.todo = todo;
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        match r.data {
            Ok(d) => {
                let path = self.cache_dir.join("mat").join(format!("{id}.mat"));
                jobs.spawn(move || {
                    let material = aurora_assets::parse_material_asset(&d).ok();
                    if material.is_some() {
                        let _ = crate::cache::write(path, &d);
                    }
                    JobResult::Material { id, material }
                });
                e.state = MatState::Fetching;
            }
            Err(_) => {
                // only the status is kept: the error text may hold the
                // capability URL
                e.failures += 1;
                e.last_status = r.status;
                // the asset CDN answers 403 for a missing asset (S3); LL's
                // asset storage does not retry a missing asset either
                if matches!(r.status, 403 | 404) || e.failures >= MATERIAL_ATTEMPTS {
                    Self::give_up(&id, e);
                    self.generation += 1;
                    self.changed.push(id);
                } else {
                    e.state = MatState::Retry;
                    e.retry_at = Some(Instant::now() + Duration::from_secs(5));
                    Self::attend(&mut self.todo, id, e);
                }
            }
        }
    }

    pub fn on_material(&mut self, id: Uuid, material: Option<PbrMaterial>) {
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        match material {
            Some(m) => {
                e.material = Some(Arc::new(m));
                e.state = MatState::Ready;
                e.retry_at = None;
                self.generation += 1;
                self.changed.push(id);
            }
            // cache miss (or unreadable cache file): download it
            None if e.state == MatState::Cache => {
                e.state = MatState::Retry;
                Self::attend(&mut self.todo, id, e);
            }
            // a downloaded asset that does not parse will not get better: LL
            // keeps the default values (onAssetLoadComplete still completes)
            None => {
                Self::give_up(&id, e);
                self.generation += 1;
                self.changed.push(id);
            }
        }
    }

    fn give_up(id: &Uuid, e: &mut MatEntry) {
        e.state = MatState::Missing;
        e.retry_at = None;
        if e.last_status == 0 && e.failures == 0 {
            log::info!("GLTF material {id}: invalid asset; its faces keep the default material and their overrides, like Firestorm");
        } else {
            log::info!(
                "GLTF material {id} unavailable (HTTP {} after {} attempt(s)); its faces keep the default material and their overrides, like Firestorm",
                e.last_status,
                e.failures
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_assets::material::{OVERRIDE_NULL_UUID, TEXTURE_BASE_COLOR};

    fn streamer() -> MaterialStreamer {
        MaterialStreamer {
            entries: HashMap::new(),
            todo: Vec::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir: PathBuf::new(),
            generation: 0,
            changed: Vec::new(),
        }
    }

    /// A download of `id` in flight, as `update` leaves it.
    fn fetching(s: &mut MaterialStreamer, id: Uuid) -> u64 {
        let key = FETCH_KIND_MATERIAL | s.next_key;
        s.next_key += 1;
        s.by_key.insert(key, id);
        if let Some(e) = s.entries.get_mut(&id) {
            e.state = MatState::Fetching;
        }
        key
    }

    fn failed(key: u64, status: u16) -> FetchResult {
        FetchResult {
            key,
            status,
            complete: true,
            offset: 0,
            data: Err(format!("HTTP {status}")),
        }
    }

    fn mesh_streamer() -> MeshStreamer {
        MeshStreamer {
            entries: FastMap::default(),
            todo: Vec::new(),
            by_key: FastMap::default(),
            next_key: 1,
            moved: Vec::new(),
            dropped: Vec::new(),
            cache_dir: PathBuf::new(),
            fetching: 0,
            logged_failures: 0,
            meta_generation: 0,
            staging: None,
            downloaded: 0,
            requests: 0,
            logged_at: None,
            logged_requests: 0,
        }
    }

    /// A download of mesh `id`'s header in flight, as `update` leaves it.
    fn mesh_fetching(s: &mut MeshStreamer, id: Uuid) -> u64 {
        s.want(id, 3);
        let key = FETCH_KIND_MESH | s.next_key;
        s.next_key += 1;
        s.by_key.insert(key, id);
        if let Some(e) = s.entries.get_mut(&id) {
            e.stage = Stage::Fetching;
            e.inflight.push((key, None));
        }
        key
    }

    /// The next mesh part a job parsed, handed to the streamer.
    fn take_part(s: &mut MeshStreamer, jobs: &Jobs, rx: &crossbeam_channel::Receiver<JobResult>) {
        loop {
            match rx.recv_timeout(Duration::from_secs(10)).expect("job result") {
                JobResult::MeshPart { id, arrival } => return s.on_part(id, *arrival, jobs),
                _ => continue,
            }
        }
    }

    fn reply(key: u64, data: &[u8], offset: usize) -> FetchResult {
        FetchResult {
            key,
            status: 206,
            complete: false,
            offset: offset as u64,
            data: Ok(data.to_vec()),
        }
    }

    #[test]
    fn a_far_mesh_comes_by_parts_and_a_near_one_whole() {
        let (jobs, rx) = Jobs::new();
        let (_rt, fetcher) = idle_fetcher();
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let rig = lib.rig.clone();
        let mut model = crate::demo::auto_hover::rigged_model(&lib);
        model.normalize().expect("model");
        let asset = aurora_assets::import_mesh::encode::asset(
            &[None, None, None, Some(model)],
            None,
            &aurora_assets::import_mesh::process::Physics::default(),
            aurora_assets::import_mesh::encode::Options {
                skin: true,
                ..Default::default()
            },
        )
        .expect("asset");
        let header = aurora_assets::parse_mesh_header(&asset).expect("header");
        let (high, skin) = (header.lods[3].expect("high"), header.skin.expect("skin"));

        // far: the start of the asset first, sized for a coarse level
        let mut s = mesh_streamer();
        let id = Uuid::from_u128(1);
        s.want(id, 1);
        s.update(&jobs, &fetcher, CAP, &rig);
        s.on_cache(id, None);
        s.update(&jobs, &fetcher, CAP, &rig);
        let (key, range) = s.entries[&id].inflight[0];
        assert_eq!(range, Some((0, 8 << 10)));
        // its reply holds the header only: placed nowhere yet (skin missing)
        s.on_fetch(reply(key, &asset[..header.header_size], 0), &jobs);
        take_part(&mut s, &jobs, &rx);
        assert_eq!(s.entries[&id].stage, Stage::Known);
        assert!(s.meta(&id).is_none());
        // the level drawn (the only one, high) and the skin after it: one request
        s.update(&jobs, &fetcher, CAP, &rig);
        let asked: Vec<_> = s.entries[&id].inflight.iter().map(|(_, r)| *r).collect();
        let (start, end) = (high.0.min(skin.0), (high.0 + high.1).max(skin.0 + skin.1));
        assert_eq!(asked, [Some((start, end - start))]);
        let key = s.entries[&id].inflight[0].0;
        s.on_fetch(reply(key, &asset[start..end], start), &jobs);
        take_part(&mut s, &jobs, &rx);
        assert!(s.meta(&id).is_some_and(|m| m.skin.is_some()));
        s.update(&jobs, &fetcher, CAP, &rig);
        assert!(s.entries[&id].building.contains(&1));
        assert_eq!(s.requests, 2);

        // near: the whole asset in one request
        let near = Uuid::from_u128(2);
        s.want(near, 3);
        s.update(&jobs, &fetcher, CAP, &rig);
        s.on_cache(near, None);
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&near].inflight[0].1, None);
    }

    /// A skinned mesh rezzed in the world: its geometry is built with the
    /// bind shape (`build_mesh_lod`), and `unbind` brings it back into the
    /// unit box its object's scale fills, as the Legacy Viewer draws it.
    #[test]
    fn unbind_brings_a_skinned_mesh_back_into_its_object_box() {
        use glam::Vec3;
        let lib = crate::scene::avatar::AvatarLibrary::load();
        let mut model = crate::demo::auto_hover::rigged_model(&lib);
        model.normalize().expect("model");
        let encode = |skin| {
            aurora_assets::import_mesh::encode::asset(
                &[None, None, None, Some(model.clone())],
                None,
                &aurora_assets::import_mesh::process::Physics::default(),
                aurora_assets::import_mesh::encode::Options {
                    skin,
                    ..Default::default()
                },
            )
            .expect("asset")
        };
        let (asset, plain) = (encode(true), encode(false));
        let header = aurora_assets::parse_mesh_header(&asset).expect("header");
        let skin = aurora_assets::decode_skin(aurora_assets::mesh::section_bytes(&asset, header.skin.expect("skin")).expect("skin"))
            .expect("skin");
        let bind = skin.bind_shape;
        let mut s = mesh_streamer();
        let (rigged, still) = (Uuid::from_u128(1), Uuid::from_u128(2));
        s.insert_local(rigged, Arc::new(asset.clone()), header.clone(), Some(Arc::new(skin)));
        let plain_header = aurora_assets::parse_mesh_header(&plain).expect("header");
        s.insert_local(still, Arc::new(plain), plain_header, None);
        assert!(s.unbind(&still).is_none());
        let unbind = s.unbind(&rigged).expect("unbind");

        let high = aurora_assets::mesh::section_bytes(&asset, header.lods[3].expect("high")).expect("high");
        let faces = aurora_assets::decode_mesh_lod(high).expect("faces");
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in faces.iter().flat_map(|f| &f.positions) {
            let built = bind.transform_point3(Vec3::from_array(*p));
            // the geometry as built lies on the skeleton, metres away
            let drawn = unbind.transform_point3(built);
            (lo, hi) = (lo.min(drawn), hi.max(drawn));
        }
        assert!(
            lo.cmpge(Vec3::splat(-0.501)).all() && hi.cmple(Vec3::splat(0.501)).all(),
            "{lo} {hi}"
        );
        assert!((hi - lo).cmpgt(Vec3::splat(0.99)).all(), "{lo} {hi}");
        assert!(bind.transform_point3(Vec3::ZERO).length() > 0.5);
    }

    /// Without a cache folder (the tests' streamers) a mesh is neither read
    /// from nor written to a disk cache: the tests run in the crate's folder,
    /// where a relative `mesh/` put cache files among the sources.
    #[test]
    fn without_a_cache_folder_there_is_no_disk_cache() {
        let id = Uuid::from_u128(1);
        assert_eq!(cache_path(std::path::Path::new(""), &id), None);
        assert_eq!(
            cache_path(std::path::Path::new("cache"), &id),
            Some(PathBuf::from("cache").join("mesh").join(format!("{id}.mesh")))
        );
    }

    /// A fetcher whose requests go nowhere: its runtime is never driven.
    fn idle_fetcher() -> (tokio::runtime::Runtime, Fetcher) {
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let (fetcher, _results) = Fetcher::new(rt.handle(), reqwest::Client::new(), 1, Default::default());
        (rt, fetcher)
    }

    const CAP: Option<&str> = Some("https://assets.example/cap");

    #[test]
    fn physics_shape_blocks_are_requested_only_while_inspecting_without_forcing_visual_lods() {
        let asset = super::super::physics_shapes::demo_asset(false).unwrap();
        let header = aurora_assets::parse_mesh_header(&asset).unwrap();
        let id = Uuid::from_u128(778);
        let mut streamer = mesh_streamer();
        streamer.entries.insert(
            id,
            MeshEntry {
                header: Some(header.clone()),
                stage: Stage::Known,
                ..Default::default()
            },
        );
        assert!(streamer.entries[&id].missing().is_empty());
        streamer.want_physics(id, true);
        let entry = &streamer.entries[&id];
        assert!(entry.wanted_lods.is_empty());
        assert_eq!(entry.missing(), [header.physics_convex.unwrap(), header.physics_mesh.unwrap()]);
        streamer.want_physics(id, false);
        assert!(streamer.entries[&id].missing().is_empty());
    }

    #[test]
    fn a_mesh_download_follows_its_nearest_user_and_goes_with_the_last() {
        let mut s = mesh_streamer();
        let id = Uuid::from_u128(5);
        let key = mesh_fetching(&mut s, id);
        if let Some(e) = s.entries.get_mut(&id) {
            e.sent = fetch_priority::mesh(40.0);
        }
        s.need(id, 40.0, false);
        // a user a little nearer: not worth telling the fetcher
        s.need(id, 50.0, false);
        assert!(s.moved.is_empty());
        // one much nearer: ranked again; a farther one changes nothing
        s.need(id, 400.0, false);
        s.need(id, 10.0, false);
        assert_eq!(s.moved, [(key, fetch_priority::mesh(400.0))]);
        // worn by us: asked again in the urgent lane
        s.need(id, 10.0, true);
        assert_eq!(s.entries[&id].lane(), (Lane::Urgent, urgent::OWN));
        assert!(s.moved[1].1.is_nan());
        // no object wants it any more: the waiting download goes
        s.unwant(&id);
        assert_eq!(s.dropped, [key]);
        assert_eq!(s.entries[&id].stage, Stage::Unwanted);
        assert!(!s.by_key.contains_key(&key));
        assert!(!s.entries[&id].has_work());
        // wanted again: downloaded again, at its new rank
        s.want(id, 1);
        assert_eq!(s.entries[&id].stage, Stage::ToFetch);
        assert_eq!(s.entries[&id].lane(), (Lane::Bulk, fetch_priority::mesh(0.0)));
    }

    #[test]
    fn update_only_visits_the_meshes_that_have_work() {
        let (jobs, _rx) = Jobs::new();
        let (_rt, fetcher) = idle_fetcher();
        let rig = crate::scene::avatar::AvatarLibrary::load().rig.clone();
        let mut s = mesh_streamer();
        // a region's worth of loaded meshes: nothing to visit
        for i in 0..2000 {
            s.insert_demo_skin(Uuid::from_u128(1000 + i), SkinInfo::default());
        }
        assert!(s.todo.is_empty());
        s.update(&jobs, &fetcher, CAP, &rig);
        assert!(s.todo.is_empty());

        // a new mesh is listed once, however often it is asked for
        let id = Uuid::from_u128(1);
        s.want(id, 2);
        s.want(id, 3);
        assert_eq!(s.todo, [id]);
        // its disk cache is read: it waits for the answer, off the list
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].stage, Stage::Reading);
        assert!(s.todo.is_empty());
        // not in the cache, and no capability yet: asked again at each frame
        s.on_cache(id, None);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, None, &rig);
        assert_eq!(s.todo, [id]);
        // downloaded once the capability is there
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].stage, Stage::Fetching);
        assert_eq!(s.fetching, 1);
        assert!(s.todo.is_empty());
        // a failed download stays listed until its retry
        let key = *s.by_key.keys().next().expect("download");
        s.on_fetch(failed(key, 503), &jobs);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].stage, Stage::Failed, "not before its retry time");
        assert_eq!(s.todo, [id]);
        assert_eq!(s.fetching, 0);
        s.entries.get_mut(&id).expect("entry").retry_at = Some(Instant::now());
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].stage, Stage::Fetching);
        assert!(s.todo.is_empty());
        // given up after the fourth failure: off the list for good
        for _ in 0..3 {
            let key = *s.by_key.keys().next().expect("download");
            s.on_fetch(failed(key, 503), &jobs);
            s.entries.get_mut(&id).expect("entry").retry_at = Some(Instant::now());
            s.update(&jobs, &fetcher, CAP, &rig);
        }
        assert!(s.failed(&id));
        assert!(s.todo.is_empty());

        // a LOD asked of a loaded mesh lists it for the frame that builds it
        let loaded = Uuid::from_u128(1000);
        s.entries.get_mut(&loaded).expect("entry").bytes = MeshBytes::whole(Arc::new(vec![0]));
        s.want(loaded, 1);
        assert_eq!(s.todo, [loaded]);
        s.update(&jobs, &fetcher, CAP, &rig);
        assert!(s.entries[&loaded].building.contains(&1));
        assert!(s.todo.is_empty());
    }

    #[test]
    fn update_only_visits_the_materials_that_have_work() {
        let (jobs, _rx) = Jobs::new();
        let (_rt, fetcher) = idle_fetcher();
        let mut s = streamer();
        for i in 0..500 {
            s.insert(Uuid::from_u128(1000 + i), PbrMaterial::default());
        }
        assert!(s.resolve(&Uuid::from_u128(1000)).is_loaded());
        assert!(s.todo.is_empty());

        // a new material is listed once, however often it is asked for
        let id = Uuid::from_u128(1);
        s.resolve(&id);
        s.get(&id);
        assert_eq!(s.todo, [id]);
        // its disk cache is read: it waits for the answer, off the list
        s.update(&jobs, &fetcher, CAP);
        assert_eq!(s.entries[&id].state, MatState::Cache);
        s.resolve(&id);
        assert!(s.todo.is_empty());
        // a cache miss: downloaded when the capability is there
        s.on_material(id, None);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, None);
        assert_eq!(s.entries[&id].state, MatState::Retry);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, CAP);
        assert_eq!(s.entries[&id].state, MatState::Fetching);
        assert!(s.todo.is_empty());
        // a failed download stays listed until its retry
        let key = *s.by_key.keys().next().expect("download");
        s.on_fetch(failed(key, 503), &jobs);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, CAP);
        assert_eq!(s.entries[&id].state, MatState::Retry, "not before its retry time");
        assert_eq!(s.todo, [id]);
        // loaded at last: nothing left to visit
        s.on_material(id, Some(PbrMaterial::default()));
        s.update(&jobs, &fetcher, CAP);
        assert!(s.todo.is_empty());
        assert!(s.resolve(&id).is_loaded());
        assert!(s.todo.is_empty());
    }

    #[test]
    fn missing_or_invalid_mesh_is_not_expected_at_once() {
        let (jobs, rx) = Jobs::new();
        let mut s = mesh_streamer();
        // the asset server does not have it: its wearer does not wait for it
        let missing = Uuid::from_u128(1);
        let key = mesh_fetching(&mut s, missing);
        assert!(!s.failed(&missing), "being downloaded");
        s.on_fetch(failed(key, 404), &jobs);
        assert!(s.failed(&missing));
        assert!(s.describe(&missing).starts_with("unavailable"), "{}", s.describe(&missing));
        // a transient error is retried: expected until the fourth failure
        let flaky = Uuid::from_u128(2);
        for attempt in 1..=4 {
            let key = mesh_fetching(&mut s, flaky);
            s.on_fetch(failed(key, 503), &jobs);
            assert_eq!(s.failed(&flaky), attempt > 3, "attempt {attempt}");
        }
        // what arrived is not a mesh
        let junk = Uuid::from_u128(3);
        let key = mesh_fetching(&mut s, junk);
        s.on_fetch(
            FetchResult {
                key,
                status: 200,
                complete: true,
                offset: 0,
                data: Ok(vec![1, 2, 3]),
            },
            &jobs,
        );
        // (told once its job looked at it)
        take_part(&mut s, &jobs, &rx);
        assert!(s.failed(&junk));
        assert_eq!(s.describe(&Uuid::from_u128(4)), "not requested");
    }

    fn denim_override() -> PbrOverride {
        let mut ov = PbrOverride::default();
        ov.textures[TEXTURE_BASE_COLOR] = Uuid::from_u128(0xd3e1);
        ov.base_color_factor = Some([0.5, 0.6, 1.0, 1.0]);
        ov
    }

    #[test]
    fn pending_material_is_the_default_without_override() {
        let m = render_material(&MaterialStatus::Pending, Some(&denim_override()));
        assert_eq!(m, PbrMaterial::default());
        assert_eq!(m.base_color_texture, None, "drawn white, never with the texture entry");
    }

    #[test]
    fn missing_material_keeps_the_override() {
        let m = render_material(&MaterialStatus::Missing, Some(&denim_override()));
        assert_eq!(m.base_color_texture, Some(Uuid::from_u128(0xd3e1)));
        assert_eq!(m.base_color_factor, [0.5, 0.6, 1.0, 1.0]);
        assert_eq!(m.metallic_factor, 1.0, "LLGLTFMaterial defaults elsewhere");
        assert_eq!(render_material(&MaterialStatus::Missing, None), PbrMaterial::default());
    }

    #[test]
    fn loaded_material_takes_the_override_on_top() {
        let base = PbrMaterial {
            base_color_texture: Some(Uuid::from_u128(7)),
            normal_texture: Some(Uuid::from_u128(8)),
            roughness_factor: 0.25,
            ..Default::default()
        };
        let m = render_material(&MaterialStatus::Loaded(Arc::new(base.clone())), Some(&denim_override()));
        assert_eq!(m.base_color_texture, Some(Uuid::from_u128(0xd3e1)));
        assert_eq!(m.normal_texture, base.normal_texture);
        assert_eq!(m.roughness_factor, 0.25);
        let mut clear = PbrOverride::default();
        clear.textures[TEXTURE_BASE_COLOR] = OVERRIDE_NULL_UUID;
        let m = render_material(&MaterialStatus::Loaded(Arc::new(base)), Some(&clear));
        assert_eq!(m.base_color_texture, None);
    }

    #[test]
    fn missing_asset_is_given_up_at_once() {
        let (jobs, _rx) = Jobs::new();
        let mut s = streamer();
        let id = Uuid::from_u128(1);
        assert!(matches!(s.resolve(&id), MaterialStatus::Pending));
        let key = fetching(&mut s, id);
        let gen0 = s.generation;
        s.on_fetch(failed(key, 403), &jobs);
        assert!(matches!(s.resolve(&id), MaterialStatus::Missing));
        assert!(s.generation > gen0, "faces using it are rebuilt with their overrides");
        assert_eq!(s.counts(), (0, 0, 1));
        assert!(s.describe(&id).contains("HTTP 403"));
    }

    #[test]
    fn transient_errors_retry_then_give_up() {
        let (jobs, _rx) = Jobs::new();
        let mut s = streamer();
        let id = Uuid::from_u128(2);
        s.resolve(&id);
        for attempt in 1..MATERIAL_ATTEMPTS {
            let key = fetching(&mut s, id);
            s.on_fetch(failed(key, 503), &jobs);
            assert!(matches!(s.resolve(&id), MaterialStatus::Pending), "attempt {attempt}");
            assert_eq!(s.entries[&id].state, MatState::Retry);
        }
        let key = fetching(&mut s, id);
        s.on_fetch(failed(key, 0), &jobs);
        assert!(matches!(s.resolve(&id), MaterialStatus::Missing));
    }

    #[test]
    fn cache_miss_downloads_and_invalid_asset_is_missing() {
        let mut s = streamer();
        let id = Uuid::from_u128(3);
        s.resolve(&id);
        s.entries.get_mut(&id).expect("entry").state = MatState::Cache;
        s.on_material(id, None);
        assert_eq!(s.entries[&id].state, MatState::Retry, "a cache miss is not a failure");
        s.entries.get_mut(&id).expect("entry").state = MatState::Fetching;
        s.on_material(id, None);
        assert!(matches!(s.resolve(&id), MaterialStatus::Missing));
        assert_eq!(s.describe(&id), "missing: invalid asset");
        let other = Uuid::from_u128(4);
        s.resolve(&other);
        s.on_material(other, Some(PbrMaterial::default()));
        assert!(matches!(s.resolve(&other), MaterialStatus::Loaded(_)));
        assert_eq!(s.counts(), (1, 0, 1));
    }
}
