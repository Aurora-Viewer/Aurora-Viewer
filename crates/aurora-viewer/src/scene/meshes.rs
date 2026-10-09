//! Mesh and GLTF material asset streaming (whole-asset fetch + disk cache).

use super::GeomKey;
use super::jobs::{JobResult, Jobs, mesh_to_faces};
use aurora_assets::{MeshHeader, PbrMaterial, SkinInfo};
use aurora_net::{FetchRequest, FetchResult, Fetcher};
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
    state: u8, // 0 new, 1 cache check, 2 fetching, 3 ready, 4 failed
    retry_at: Option<Instant>,
    failures: u32,
    wanted_lods: HashSet<u8>,
    building: HashSet<u8>,
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
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    pub fetching: usize,
    /// Failures written to the log (the first ones, then one in 50).
    logged_failures: u32,
}

impl MeshStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("mesh"));
        MeshStreamer {
            entries: HashMap::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            fetching: 0,
            logged_failures: 0,
        }
    }

    /// Ask for a LOD of a mesh; geometry arrives later as `JobResult::Geometry`.
    pub fn want(&mut self, id: Uuid, lod: u8) {
        let e = self.entries.entry(id).or_default();
        e.wanted_lods.insert(lod);
    }

    /// Disk cache file of a mesh asset (media picking reads it).
    pub fn cache_path(&self, id: &Uuid) -> PathBuf {
        self.cache_dir.join("mesh").join(format!("{id}.mesh"))
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

    pub fn failed(&self, id: &Uuid) -> bool {
        self.entries.get(id).is_some_and(|e| e.state == 4 && e.failures > 3)
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>, rig: &Arc<super::anim::Rig>) {
        let now = Instant::now();
        let mut fetching = 0;
        for (id, e) in self.entries.iter_mut() {
            if e.state == 2 {
                fetching += 1;
            }
            if e.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            match e.state {
                0 => {
                    e.state = 1;
                    let path = self.cache_dir.join("mesh").join(format!("{id}.mesh"));
                    let id = *id;
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
                    continue;
                };
                let key = FETCH_KIND_MESH | self.next_key;
                self.next_key += 1;
                self.by_key.insert(key, *id);
                e.state = 2;
                fetcher.request(FetchRequest {
                    key,
                    url: super::textures::asset_url(base, "mesh_id", id),
                    range: None,
                    // geometry first: texture priorities (pixel area) would starve it
                    priority: 1e12,
                    accept: "application/vnd.ll.mesh",
                });
            }
            if e.state == 3 {
                let (Some(data), Some(header)) = (e.data.clone(), e.header.clone()) else {
                    continue;
                };
                let lods: Vec<u8> = e.wanted_lods.drain().collect();
                for lod in lods {
                    if !e.building.insert(lod) {
                        continue;
                    }
                    let id = *id;
                    let skin = e.skin.clone();
                    let header = header.clone();
                    let data = data.clone();
                    let rig = rig.clone();
                    jobs.spawn(move || build_mesh_lod(id, lod, &data, &header, skin, &rig));
                }
            }
        }
        self.fetching = fetching;
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
                e.building.clear();
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
                self.fail(id, Duration::from_secs(120));
            }
            _ => {
                let why = match &r.data {
                    Err(e) => e.clone(),
                    Ok(_) => "empty response".to_owned(),
                };
                self.log_failure(format_args!("mesh {id}: download failed (HTTP {}, {why})", r.status));
                let backoff = if r.status == 404 { 600 } else { 5 };
                self.fail(id, Duration::from_secs(backoff));
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

    fn fail(&mut self, id: Uuid, backoff: Duration) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.state = 4;
            e.failures += 1;
            e.retry_at = Some(Instant::now() + backoff);
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
                }
            }
            None => e.state = 5,
        }
    }

    pub fn on_geometry_done(&mut self, id: &Uuid, lod: u8) {
        if let Some(e) = self.entries.get_mut(id) {
            e.building.remove(&lod);
        }
    }
}

fn build_mesh_lod(id: Uuid, lod: u8, data: &[u8], header: &MeshHeader, skin: Option<Arc<SkinInfo>>, rig: &super::anim::Rig) -> JobResult {
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
            let (faces, min, max) = mesh_to_faces(&faces, xform, map.as_deref());
            JobResult::Geometry {
                key,
                faces,
                min,
                max,
                area: 1.0,
            }
        }
        Err(e) => {
            log::debug!("mesh {id} lod {lod}: {e}");
            JobResult::GeometryFailed { key }
        }
    }
}

// ------------------------------------------------------------------ materials

struct MatEntry {
    material: Option<Arc<PbrMaterial>>,
    state: u8, // 0 new, 1 cache, 2 fetching, 3 ready, 4 failed
    retry_at: Option<Instant>,
    failures: u32,
}

pub struct MaterialStreamer {
    entries: HashMap<Uuid, MatEntry>,
    by_key: HashMap<u64, Uuid>,
    next_key: u64,
    cache_dir: PathBuf,
    /// Bumped whenever a material arrives.
    pub generation: u64,
}

impl MaterialStreamer {
    pub fn new(cache_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(cache_dir.join("mat"));
        MaterialStreamer {
            entries: HashMap::new(),
            by_key: HashMap::new(),
            next_key: 1,
            cache_dir,
            generation: 0,
        }
    }

    pub fn get(&mut self, id: &Uuid) -> Option<Arc<PbrMaterial>> {
        let e = self.entries.entry(*id).or_insert(MatEntry {
            material: None,
            state: 0,
            retry_at: None,
            failures: 0,
        });
        e.material.clone()
    }

    /// A material known locally (offline demo).
    pub fn insert(&mut self, id: Uuid, m: PbrMaterial) {
        self.entries.insert(
            id,
            MatEntry {
                material: Some(Arc::new(m)),
                state: 3,
                retry_at: None,
                failures: 0,
            },
        );
        self.generation += 1;
    }

    /// The material if it is loaded, without requesting it (diagnostics).
    pub fn peek(&self, id: &Uuid) -> Option<Arc<PbrMaterial>> {
        self.entries.get(id).and_then(|e| e.material.clone())
    }

    pub fn update(&mut self, jobs: &Jobs, fetcher: &Fetcher, viewer_asset: Option<&str>) {
        let now = Instant::now();
        for (id, e) in self.entries.iter_mut() {
            if e.retry_at.is_some_and(|t| now < t) {
                continue;
            }
            if e.state == 0 {
                e.state = 1;
                let path = self.cache_dir.join("mat").join(format!("{id}.mat"));
                let id = *id;
                jobs.spawn(move || {
                    let material = crate::cache::read_touch(path)
                        .ok()
                        .and_then(|d| aurora_assets::parse_material_asset(&d).ok());
                    JobResult::Material { id, material }
                });
            } else if e.state == 4 && e.failures <= 3 {
                let Some(base) = viewer_asset else {
                    continue;
                };
                let key = FETCH_KIND_MATERIAL | self.next_key;
                self.next_key += 1;
                self.by_key.insert(key, *id);
                e.state = 2;
                fetcher.request(FetchRequest {
                    key,
                    url: super::textures::asset_url(base, "material_id", id),
                    range: None,
                    priority: 9e11,
                    accept: "*/*",
                });
            }
        }
    }

    pub fn on_fetch(&mut self, r: FetchResult, jobs: &Jobs) {
        let Some(id) = self.by_key.remove(&r.key) else {
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
                if let Some(e) = self.entries.get_mut(&id) {
                    e.state = 2;
                }
            }
            Err(_) => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.state = 4;
                    e.failures += 1;
                    e.retry_at = Some(Instant::now() + Duration::from_secs(if r.status == 404 { 600 } else { 5 }));
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
                e.state = 3;
                self.generation += 1;
            }
            None => {
                // cache miss or invalid asset: fetch (or count a failure)
                if e.state == 1 {
                    e.state = 4;
                } else {
                    e.state = 4;
                    e.failures += 1;
                    e.retry_at = Some(Instant::now() + Duration::from_secs(30));
                }
            }
        }
    }
}
