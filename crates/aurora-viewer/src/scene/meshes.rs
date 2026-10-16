//! Mesh and GLTF material asset streaming (whole-asset fetch + disk cache).

use super::GeomKey;
use super::jobs::{JobResult, Jobs, mesh_to_faces};
use aurora_assets::material::PbrOverride;
use aurora_assets::{MeshHeader, PbrMaterial, SkinInfo};
use super::fetch_priority::{self, urgent};
use aurora_net::{FetchRequest, FetchResult, Fetcher, Lane};
use aurora_render::StagingPool;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FETCH_KIND_MESH: u64 = 2 << 60;
pub const FETCH_KIND_MATERIAL: u64 = 3 << 60;

#[derive(Default)]
struct MeshEntry {
    data: Option<Arc<Vec<u8>>>,
    header: Option<MeshHeader>,
    skin: Option<Arc<SkinInfo>>,
    state: u8, // 0 new, 1 cache check, 2 fetching, 3 ready, 4 failed, 5 to fetch, 6 not wanted
    retry_at: Option<Instant>,
    failures: u32,
    /// The asset server does not have it, or what it sent is not a mesh:
    /// asked again much later, and not expected meanwhile.
    gone: bool,
    wanted_lods: HashSet<u8>,
    building: HashSet<u8>,
    /// Listed in `MeshStreamer::todo`.
    queued: bool,
    /// Size on screen of its largest user, in pixels, and whether one is
    /// ours (`MeshStreamer::need`): the rank of its download.
    px: f32,
    own: bool,
    /// Its download under way, and the rank it was last given.
    fetch_key: u64,
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
}

impl MeshEntry {
    /// `MeshStreamer::update` has something to do for it, now or at its
    /// retry time: a new mesh to read from the disk cache, a download to
    /// start or to start again, LODs to build.
    fn has_work(&self) -> bool {
        match self.state {
            0 | 5 => true,
            3 => !self.wanted_lods.is_empty(),
            4 => self.failures <= 3,
            _ => false,
        }
    }
}

/// Info about a mesh needed for placement.
#[derive(Clone)]
pub struct MeshMeta {
    pub skin: Option<Arc<SkinInfo>>,
    /// Byte size of each LOD (lowest .. high), for the render cost.
    pub lod_bytes: [u32; 4],
}

pub struct MeshStreamer {
    entries: HashMap<Uuid, MeshEntry>,
    /// The meshes `update` has something to do for (`MeshEntry::has_work`),
    /// each listed once. It only visits these: a region leaves thousands of
    /// loaded meshes behind, and walking them all at each frame cost 0.13 ms
    /// for good after one visit to a busy region (7,700 meshes).
    todo: Vec<Uuid>,
    by_key: HashMap<u64, Uuid>,
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
}

impl MeshStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("mesh"));
        MeshStreamer {
            entries: HashMap::new(),
            todo: Vec::new(),
            by_key: HashMap::new(),
            next_key: 1,
            moved: Vec::new(),
            dropped: Vec::new(),
            cache_dir,
            fetching: 0,
            logged_failures: 0,
            meta_generation: 0,
            staging: None,
        }
    }

    /// Ask for a LOD of a mesh; geometry arrives later as `JobResult::Geometry`.
    pub fn want(&mut self, id: Uuid, lod: u8) {
        let e = self.entries.entry(id).or_default();
        e.wanted_lods.insert(lod);
        if e.state == 6 {
            e.state = 5;
        }
        Self::attend(&mut self.todo, id, e);
    }

    /// An object using a mesh covers `px` pixels on screen (`own`: worn by
    /// our avatar). A download still waiting moves up to the nearest user;
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
        if e.state == 2 && e.fetch_key != 0 {
            if lane != was.0 {
                // (a new request under the same key replaces the waiting one)
                self.moved.push((e.fetch_key, f32::NAN));
            } else if fetch_priority::changed(e.sent, priority) {
                e.sent = priority;
                self.moved.push((e.fetch_key, priority));
            }
        }
    }

    /// No object uses the mesh any more (the scene let all its geometries
    /// go). A download still waiting for a slot is dropped, as Firestorm
    /// drops pending mesh requests whose volumes are gone; one already
    /// sent ends, and is ignored. Wanted again, it starts over.
    pub fn unwant(&mut self, id: &Uuid) {
        let Some(e) = self.entries.get_mut(id) else {
            return;
        };
        e.wanted_lods.clear();
        e.px = 0.0;
        e.own = false;
        match e.state {
            2 => {
                self.by_key.remove(&e.fetch_key);
                self.dropped.push(e.fetch_key);
                e.fetch_key = 0;
                e.state = 6;
            }
            5 => e.state = 6,
            _ => {}
        }
    }

    /// List a mesh for `update`, once.
    fn attend(todo: &mut Vec<Uuid>, id: Uuid, e: &mut MeshEntry) {
        if !std::mem::replace(&mut e.queued, true) {
            todo.push(id);
        }
    }

    /// Disk cache file of a mesh asset (media picking reads it).
    pub fn cache_path(&self, id: &Uuid) -> PathBuf {
        self.cache_dir.join("mesh").join(format!("{id}.mesh"))
    }

    pub fn preview_source(&self, id: &Uuid) -> (Option<Arc<Vec<u8>>>, Option<Arc<SkinInfo>>) {
        self.entries
            .get(id)
            .map(|entry| (entry.data.clone(), entry.skin.clone()))
            .unwrap_or_default()
    }

    pub fn meta(&self, id: &Uuid) -> Option<MeshMeta> {
        let e = self.entries.get(id)?;
        let h = e.header.as_ref()?;
        let lod_bytes = h.lods.map(|l| l.map(|(_, size)| size as u32).unwrap_or(0));
        Some(MeshMeta {
            skin: e.skin.clone(),
            lod_bytes,
        })
    }

    /// In-memory meshes for local previews and offline fixtures; no asset fetch.
    pub fn insert_local(&mut self, id: Uuid, data: Arc<Vec<u8>>, header: MeshHeader, skin: Option<Arc<SkinInfo>>) {
        self.entries.insert(
            id,
            MeshEntry {
                data: Some(data),
                header: Some(header),
                skin,
                state: 3,
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
                    version: 1,
                }),
                state: 3,
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
        self.entries.get(id).is_some_and(|e| e.state == 4 && (e.gone || e.failures > 3))
    }

    /// Diagnostic: the state of a mesh asset, without requesting it.
    pub fn describe(&self, id: &Uuid) -> String {
        let Some(e) = self.entries.get(id) else {
            return "not requested".into();
        };
        let state = match e.state {
            0 => "new",
            1 => "reading the disk cache",
            2 => "downloading",
            3 => "loaded",
            4 if e.gone => "unavailable",
            4 => "download failed",
            _ => "waiting to download",
        };
        let retry = e
            .retry_at
            .map(|t| t.saturating_duration_since(Instant::now()).as_secs())
            .filter(|s| *s > 0)
            .map(|s| format!(", retry in {s} s"))
            .unwrap_or_default();
        let mut building: Vec<u8> = e.building.iter().copied().collect();
        building.sort_unstable();
        format!("{state}, failures {}{retry}, LODs being built {building:?}", e.failures)
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>, rig: &Arc<super::anim::Rig>) {
        let now = Instant::now();
        for key in self.dropped.drain(..) {
            fetcher.cancel(key);
        }
        for (key, priority) in self.moved.drain(..) {
            match self.by_key.get(&key).and_then(|id| self.entries.get_mut(id)) {
                // (to the other lane: asked again under its key)
                Some(e) if priority.is_nan() => {
                    if let Some(base) = viewer_asset {
                        let (lane, priority) = e.lane();
                        e.sent = priority;
                        let id = self.by_key[&key];
                        fetcher.request(Self::request(key, base, &id, lane, priority));
                    }
                }
                Some(_) => fetcher.reprioritise(key, priority),
                None => {}
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
                match e.state {
                    0 => {
                        e.state = 1;
                        let path = self.cache_dir.join("mesh").join(format!("{id}.mesh"));
                        jobs.spawn(move || JobResult::MeshCache {
                            id,
                            data: crate::cache::read_touch(path).ok(),
                        });
                    }
                    4 if e.failures <= 3 => {
                        e.state = 5; // trigger fetch below
                    }
                    _ => {}
                }
                if e.state == 5 {
                    let Some(base) = viewer_asset else {
                        e.state = 4;
                        break 'visit;
                    };
                    let key = FETCH_KIND_MESH | self.next_key;
                    self.next_key += 1;
                    self.by_key.insert(key, id);
                    e.state = 2;
                    e.fetch_key = key;
                    let (lane, priority) = e.lane();
                    e.sent = priority;
                    fetcher.request(Self::request(key, base, &id, lane, priority));
                }
                if e.state == 3 && !e.wanted_lods.is_empty() {
                    let (Some(data), Some(header)) = (e.data.clone(), e.header.clone()) else {
                        break 'visit;
                    };
                    let lods: Vec<u8> = e.wanted_lods.drain().collect();
                    for lod in lods {
                        if !e.building.insert(lod) {
                            continue;
                        }
                        let skin = e.skin.clone();
                        let header = header.clone();
                        let data = data.clone();
                        let rig = rig.clone();
                        let pool = self.staging.clone();
                        jobs.spawn(move || build_mesh_lod(id, lod, &data, &header, skin, &rig, pool.as_deref()));
                    }
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
    }

    fn request(key: u64, base: &str, id: &Uuid, lane: Lane, priority: f32) -> FetchRequest {
        FetchRequest {
            key,
            url: super::textures::asset_url(base, "mesh_id", id),
            range: None,
            priority,
            lane,
            accept: "application/vnd.ll.mesh",
        }
    }

    fn accept_data(&mut self, id: Uuid, data: Vec<u8>) -> bool {
        let Some(e) = self.entries.get_mut(&id) else {
            return false;
        };
        match aurora_assets::parse_mesh_header(&data) {
            Ok(h) => {
                if let Some(range) = h.skin
                    && let Some(sec) = aurora_assets::mesh::section_bytes(&data, range)
                {
                    match aurora_assets::decode_skin(sec) {
                        Ok(s) => e.skin = Some(Arc::new(s)),
                        Err(err) => log::debug!("mesh {id} skin: {err}"),
                    }
                }
                e.header = Some(h);
                e.data = Some(Arc::new(data));
                e.state = 3;
                e.gone = false;
                e.building.clear();
                // (the LODs asked for while it was coming)
                Self::attend(&mut self.todo, id, e);
                self.meta_generation += 1;
                true
            }
            Err(err) => {
                self.log_failure(format_args!("mesh {id}: invalid asset ({err})"));
                false
            }
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
            return;
        };
        if let Some(e) = self.entries.get_mut(&id) {
            e.fetch_key = 0;
        }
        match r.data {
            Ok(data) if !data.is_empty() => {
                let copy = data.clone();
                if self.accept_data(id, data) {
                    let path = self.cache_dir.join("mesh").join(format!("{id}.mesh"));
                    jobs.spawn(move || {
                        let _ = crate::cache::write(path, &copy);
                        JobResult::Done
                    });
                    return;
                }
                // not a mesh: it will not get better
                self.fail(id, Duration::from_secs(120), true);
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
            match e.state {
                3 => c.0 += 1,
                2 => c.1 += 1,
                4 => c.2 += 1,
                _ => {}
            }
        }
        c
    }

    /// A download that failed, asked again after `backoff`; `gone` when the
    /// asset is missing or invalid (see `failed`).
    fn fail(&mut self, id: Uuid, backoff: Duration, gone: bool) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.state = 4;
            e.failures += 1;
            e.gone = gone;
            e.retry_at = Some(Instant::now() + backoff);
            Self::attend(&mut self.todo, id, e);
        }
    }

    pub fn on_cache(&mut self, id: Uuid, data: Option<Vec<u8>>) {
        let Some(e) = self.entries.get_mut(&id) else {
            return;
        };
        if e.state != 1 {
            return;
        }
        match data {
            Some(d) => {
                if !self.accept_data(id, d)
                    && let Some(e) = self.entries.get_mut(&id)
                {
                    e.state = 5;
                    Self::attend(&mut self.todo, id, e);
                }
            }
            None => {
                e.state = 5;
                Self::attend(&mut self.todo, id, e);
            }
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
    data: &[u8],
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
    let Some(section) = aurora_assets::mesh::section_bytes(data, range) else {
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
}

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
        }
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
        let e = self.entries.entry(*id).or_insert_with(MatEntry::new);
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
    }

    /// The material if it is loaded, without requesting it (diagnostics).
    pub fn remove_local(&mut self, id: &Uuid) {
        if self.entries.remove(id).is_some() {
            self.generation += 1;
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
            entries: HashMap::new(),
            todo: Vec::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir: PathBuf::new(),
            fetching: 0,
            logged_failures: 0,
            meta_generation: 0,
            staging: None,
        }
    }

    /// A download of mesh `id` in flight, as `update` leaves it.
    fn mesh_fetching(s: &mut MeshStreamer, id: Uuid) -> u64 {
        s.want(id, 3);
        let key = FETCH_KIND_MESH | s.next_key;
        s.next_key += 1;
        s.by_key.insert(key, id);
        if let Some(e) = s.entries.get_mut(&id) {
            e.state = 2;
        }
        key
    }

    /// A fetcher whose requests go nowhere: its runtime is never driven.
    fn idle_fetcher() -> (tokio::runtime::Runtime, Fetcher) {
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        let (fetcher, _results) = Fetcher::new(rt.handle(), reqwest::Client::new(), 1, Default::default());
        (rt, fetcher)
    }

    const CAP: Option<&str> = Some("https://assets.example/cap");

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
        assert_eq!(s.entries[&id].state, 1);
        assert!(s.todo.is_empty());
        // not in the cache, and no capability yet: asked again at each frame
        s.on_cache(id, None);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, None, &rig);
        assert_eq!(s.todo, [id]);
        // downloaded once the capability is there
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].state, 2);
        assert_eq!(s.fetching, 1);
        assert!(s.todo.is_empty());
        // a failed download stays listed until its retry
        let key = *s.by_key.keys().next().expect("download");
        s.on_fetch(failed(key, 503), &jobs);
        assert_eq!(s.todo, [id]);
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].state, 4, "not before its retry time");
        assert_eq!(s.todo, [id]);
        assert_eq!(s.fetching, 0);
        s.entries.get_mut(&id).expect("entry").retry_at = Some(Instant::now());
        s.update(&jobs, &fetcher, CAP, &rig);
        assert_eq!(s.entries[&id].state, 2);
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
        s.entries.get_mut(&loaded).expect("entry").data = Some(Arc::new(Vec::new()));
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
        let (jobs, _rx) = Jobs::new();
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
