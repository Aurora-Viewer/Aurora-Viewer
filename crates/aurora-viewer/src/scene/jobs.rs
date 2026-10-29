//! Background work (rayon) and its results.

use super::GeomKey;
use super::animesh::JointBounds;
use super::picking::PickFace;
use aurora_render::{SkinVertex, StagedMesh, StagingPool, Vertex};
use crossbeam_channel::{Receiver, Sender};
use glam::Vec3;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlphaKind {
    #[default]
    Opaque,
    Mask,
    Blend,
}

pub struct SculptMap {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub pixels: Vec<u8>,
}

pub struct FaceGeom {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub skin: Option<Vec<SkinVertex>>,
}

pub type FaceData = Option<FaceGeom>;

/// Where a face's data waits for the GPU.
pub enum FaceUpload {
    /// Written into the renderer's staging memory by the job: the main
    /// thread only records the copies.
    Staged(StagedMesh),
    /// Kept in memory for a main-thread write (staging memory full, or a
    /// geometry built on the main thread).
    Memory(FaceGeom),
}

/// A built geometry with everything the main thread would otherwise
/// compute when it arrives: the picking copy of its triangles, the bounds
/// of its skinned vertices per joint, and its vertex data staged for the
/// GPU when the renderer's staging memory has room.
pub struct PreparedGeom {
    pub faces: Vec<Option<FaceUpload>>,
    pub min: Vec3,
    pub max: Vec3,
    pub joint_bounds: Vec<JointBounds>,
    pub pick_faces: Vec<Option<PickFace>>,
}

impl PreparedGeom {
    /// `pick`: keep a copy of the triangles for ray picking (not for the
    /// avatar's own body parts).
    pub fn new(faces: Vec<FaceData>, min: Vec3, max: Vec3, pick: bool, pool: Option<&StagingPool>) -> Self {
        let pick_faces = if pick {
            faces
                .iter()
                .map(|f| {
                    f.as_ref().map(|f| {
                        PickFace::new(
                            f.vertices.iter().map(|v| v.pos).collect(),
                            f.vertices.iter().map(|v| v.uv).collect(),
                            f.indices.clone(),
                        )
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut boxes = std::collections::HashMap::<u8, (Vec3, Vec3)>::new();
        for f in faces.iter().flatten() {
            if let Some(skin) = &f.skin {
                for (v, s) in f.vertices.iter().zip(skin) {
                    let p = Vec3::from_array(v.pos);
                    for (&j, &w) in s.joints.iter().zip(&s.weights) {
                        if w != 0 {
                            let b = boxes.entry(j).or_insert((p, p));
                            b.0 = b.0.min(p);
                            b.1 = b.1.max(p);
                        }
                    }
                }
            }
        }
        let joint_bounds = boxes
            .into_iter()
            .map(|(joint, (min, max))| JointBounds { joint, min, max })
            .collect();
        let faces = faces
            .into_iter()
            .map(|f| {
                let f = f?;
                Some(
                    match pool.and_then(|pool| StagedMesh::new(pool, &f.vertices, f.skin.as_deref(), &f.indices)) {
                        Some(staged) => FaceUpload::Staged(staged),
                        None => FaceUpload::Memory(f),
                    },
                )
            })
            .collect();
        PreparedGeom {
            faces,
            min,
            max,
            joint_bounds,
            pick_faces,
        }
    }

    /// Every staged face can be copied (no job still writes in its chunk).
    pub fn ready(&self) -> bool {
        self.faces
            .iter()
            .flatten()
            .all(|f| !matches!(f, FaceUpload::Staged(s) if !s.data.ready()))
    }
}

pub enum JobResult {
    Geometry {
        key: GeomKey,
        geom: PreparedGeom,
        /// LLVolume::getSurfaceArea (sculpts; 1 otherwise).
        area: f32,
    },
    GeometryFailed {
        key: GeomKey,
    },
    Texture {
        id: Uuid,
        discard: u8,
        /// Levels kept in memory (all of them unless `staged`).
        mips: Vec<(u32, u32, Vec<u8>)>,
        /// The chain written into the renderer's staging memory by the job.
        staged: Option<aurora_render::StagedTexture>,
        alpha: AlphaKind,
        /// The image has an alpha channel (2 or 4 components), whatever its values.
        alpha_channel: bool,
        pick_mask: Option<super::textures::PickMask>,
        sculpt: Option<Arc<SculptMap>>,
    },
    TextureFailed {
        id: Uuid,
    },
    /// The JPEG 2000 data of a texture read from the disk cache (none: not
    /// there).
    TextureCache {
        id: Uuid,
        data: Option<Vec<u8>>,
        complete: bool,
        /// The transcoded cache says BC7 would harm this texture.
        keep_rgba: bool,
    },
    /// A texture read back from the transcoded disk cache
    /// (`texture_cache.rs`), staged as a decode job would have: `texture`
    /// is its `Texture` result, `info` the JPEG 2000 image it comes from and
    /// `finest` the finest discard level the file holds.
    TextureTranscoded {
        id: Uuid,
        info: aurora_assets::J2kInfo,
        finest: u8,
        texture: Box<JobResult>,
    },
    /// A mesh's disk cache file, parsed (None: not there, or damaged).
    MeshCache {
        id: Uuid,
        arrival: Option<Box<super::meshes::MeshArrival>>,
    },
    /// Bytes of a mesh downloaded, parsed.
    MeshPart {
        id: Uuid,
        arrival: Box<super::meshes::MeshArrival>,
    },
    Material {
        id: Uuid,
        material: Option<aurora_assets::PbrMaterial>,
    },
    /// Work whose result was delivered through another channel.
    Done,
}

/// Background jobs give way to the frame: while decodes keep every core
/// busy (arrival in a region), the main thread, the workers that help it
/// (frame_par.rs) and the encoder threads would otherwise wait for a core
/// behind them (frames of 2.4 ms stretched to 5–8 ms in AURORA_DEMO_STREAM).
/// Idle cores still go to the jobs, so loading is not slower.
fn below_normal_priority() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        // SAFETY: `GetCurrentThread` returns the calling thread's
        // pseudo-handle, always valid for it; `SetThreadPriority` only
        // reads that handle and an integer.
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
    }
    // Linux: the nice value is per thread there, and the scheduler shares
    // the cores by weight rather than by strict priority as Windows does
    // (a step of nice is ~1.25× less weight). +10 over the process's base
    // leaves a job thread about a tenth of a frame thread's weight (110 /
    // 1024), near Windows' "frame first" without starving the decodes:
    // the AURORA_DEMO_STREAM arrival loads in the same time either way.
    #[cfg(target_os = "linux")]
    {
        use aurora_render::frame_priority::linux;
        // lowering needs no privilege
        linux::set_thread_nice(linux::base_nice() + 10);
    }
}

impl JobResult {
    /// A geometry built by a job, prepared for the GPU on the job's thread.
    pub fn geometry(key: GeomKey, (faces, min, max): (Vec<FaceData>, Vec3, Vec3), area: f32, pool: Option<&StagingPool>) -> Self {
        let pick = !matches!(key, GeomKey::AvatarPart(_) | GeomKey::AvatarMorph { .. });
        JobResult::Geometry {
            key,
            geom: PreparedGeom::new(faces, min, max, pick, pool),
            area,
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: `execute` runs its work on the calling thread. The pool's
    /// threads run below normal priority, and on a loaded CI runner a UI
    /// test could wait more than 5 s for its preview job.
    static EXECUTE_INLINE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: run what `Jobs::execute` is given on this thread, synchronously,
/// until the guard is dropped (a harness run with one test thread reuses it).
#[cfg(test)]
#[must_use]
pub fn execute_inline_on_this_thread() -> ExecuteInline {
    EXECUTE_INLINE.set(true);
    ExecuteInline
}

#[cfg(test)]
pub struct ExecuteInline;

#[cfg(test)]
impl Drop for ExecuteInline {
    fn drop(&mut self) {
        EXECUTE_INLINE.set(false);
    }
}

#[derive(Clone)]
pub struct Jobs {
    tx: Sender<JobResult>,
    pub in_flight: Arc<AtomicUsize>,
    /// Background pool, apart from the global one used by the frame
    /// (the parallel loops of frame_par.rs): a frame never waits behind a
    /// JPEG 2000 decode or a disk read.
    pool: Arc<rayon::ThreadPool>,
    /// What the main thread hands over to be freed (`discard`).
    trash: Sender<Box<dyn Send>>,
}

/// The thread that frees what `Jobs::discard` receives, until every `Jobs`
/// is gone. If it cannot start, `discard` frees in place.
fn start_trash_thread() -> Sender<Box<dyn Send>> {
    let (tx, rx) = crossbeam_channel::unbounded::<Box<dyn Send>>();
    // (normal priority: behind the decode jobs it could fall behind what the
    // main thread hands over)
    let spawned = std::thread::Builder::new().name("aurora-trash".into()).spawn(move || {
        for garbage in rx {
            drop(garbage);
        }
    });
    if let Err(e) = spawned {
        log::warn!("trash thread: {e}; freeing on the calling thread");
    }
    tx
}

impl Jobs {
    pub fn new() -> (Jobs, Receiver<JobResult>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.saturating_sub(2).max(1))
            .thread_name(|i| format!("aurora-jobs-{i}"))
            .start_handler(|_| below_normal_priority())
            .build()
            .map(Arc::new)
            .unwrap_or_else(|e| {
                log::warn!("background pool: {e}; falling back to one thread");
                // a pool of one thread cannot fail in practice; keep going
                Arc::new(rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("one-thread pool"))
            });
        (
            Jobs {
                tx,
                in_flight: Arc::new(AtomicUsize::new(0)),
                pool,
                trash: start_trash_thread(),
            },
            rx,
        )
    }

    pub fn spawn(&self, f: impl FnOnce() -> JobResult + Send + 'static) {
        let tx = self.tx.clone();
        let n = self.in_flight.clone();
        n.fetch_add(1, Ordering::Relaxed);
        self.pool.spawn(move || {
            let r = f();
            n.fetch_sub(1, Ordering::Relaxed);
            let _ = tx.send(r);
        });
    }

    /// UI file imports share the low-priority worker pool without feeding
    /// their results into the scene's geometry/texture result channel.
    pub fn execute(&self, work: impl FnOnce() + Send + 'static) {
        #[cfg(test)]
        if EXECUTE_INLINE.get() {
            work();
            return;
        }
        self.pool.spawn(work);
    }

    /// A job that reports by itself (it fills something it shares with the
    /// scene): nothing comes back through the results.
    pub fn run(&self, f: impl FnOnce() + Send + 'static) {
        self.pool.spawn(f);
    }

    /// One job per item, queued with a single wake-up of the pool: a burst
    /// of small jobs (the cache reads of a few hundred new textures) costs
    /// the main thread one `spawn` instead of one each.
    pub fn spawn_many<T: Send + 'static>(&self, items: Vec<T>, f: impl Fn(T) -> JobResult + Send + Sync + 'static) {
        if items.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let n = self.in_flight.clone();
        n.fetch_add(items.len(), Ordering::Relaxed);
        self.pool.spawn(move || {
            use rayon::prelude::*;
            items.into_par_iter().for_each(|item| {
                let r = f(item);
                n.fetch_sub(1, Ordering::Relaxed);
                let _ = tx.send(r);
            });
        });
    }

    /// Free `value` on the trash thread. Releasing decoded pixels, J2C data
    /// or picking triangles on the main thread costs it far more than the
    /// work itself while the jobs allocate on every core: the heap
    /// serializes large blocks, so a handful of frees of 16–256 KB waited
    /// 1–3 ms behind them (measured in `TextureStreamer::upload`, where the
    /// time budget could not see it). A thread of its own rather than the
    /// pool: behind a long queue of jobs the garbage would pile up.
    pub fn discard<T: Send + 'static>(&self, value: T) {
        // (without the thread, the value comes back in the error and is
        // freed here, as before)
        let _ = self.trash.send(Box::new(value));
    }

    pub fn pending(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
    }
}

/// Values to free off the main thread, gathered over a step and handed to
/// the trash thread in one go ([`Jobs::discard`]).
pub struct Trash<T: Send + 'static>(Vec<T>);

impl<T: Send + 'static> Default for Trash<T> {
    fn default() -> Self {
        Trash(Vec::new())
    }
}

impl<T: Send + 'static> Trash<T> {
    pub fn push(&mut self, value: T) {
        self.0.push(value);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Hand what was gathered to the trash thread (nothing when empty).
    pub fn empty(&mut self, jobs: &Jobs) {
        if !self.0.is_empty() {
            jobs.discard(std::mem::take(&mut self.0));
        }
    }
}

/// Classify texture alpha: fully opaque, binary (mask) or smooth (blend).
pub fn classify_alpha(rgba: &[u8]) -> AlphaKind {
    let mut transparent = 0usize;
    let mut partial = 0usize;
    let mut i = 3;
    let mut n = 0usize;
    while i < rgba.len() {
        let a = rgba[i];
        if a < 250 {
            transparent += 1;
            if a > 12 && a < 243 {
                partial += 1;
            }
        }
        n += 1;
        i += 4;
    }
    if transparent == 0 || n == 0 {
        AlphaKind::Opaque
    } else if partial * 20 < transparent.max(1) && partial * 50 < n {
        AlphaKind::Mask
    } else {
        AlphaKind::Blend
    }
}

/// Expand 1-4 component pixels to RGBA8.
pub fn to_rgba(components: u8, data: &[u8], pixels: usize) -> Vec<u8> {
    let c = components.max(1) as usize;
    let mut out = Vec::with_capacity(pixels * 4);
    for p in 0..pixels {
        let s = &data[(p * c).min(data.len())..((p + 1) * c).min(data.len())];
        let px = match (c, s) {
            (1, [l]) => [*l, *l, *l, 255],
            (2, [l, a]) => [*l, *l, *l, *a],
            (3, [r, g, b]) => [*r, *g, *b, 255],
            (_, [r, g, b, a, ..]) => [*r, *g, *b, *a],
            _ => [128, 128, 128, 255],
        };
        out.extend_from_slice(&px);
    }
    out
}

/// Convert prim/sculpt volume faces into GPU vertex data.
pub fn volume_to_faces(mesh: &aurora_prim::VolumeMesh) -> (Vec<FaceData>, Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let faces = mesh
        .faces
        .iter()
        .map(|f| {
            if f.positions.is_empty() || f.indices.len() < 3 {
                return None;
            }
            let mut verts: Vec<Vertex> = f
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let v = Vec3::from_array(*p);
                    min = min.min(v);
                    max = max.max(v);
                    let n = f.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                    let uv = f.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
                    Vertex::new(*p, n, uv)
                })
                .collect();
            if !f.uvs.is_empty() {
                super::tangents::pack_volume_tangents(&mut verts, &f.indices);
            }
            Some(FaceGeom {
                vertices: verts,
                indices: f.indices.clone(),
                skin: None,
            })
        })
        .collect();
    if min.x > max.x {
        min = Vec3::splat(-0.5);
        max = Vec3::splat(0.5);
    }
    (faces, min, max)
}

/// Mesh faces draw their triangles in the Legacy Viewer's order
/// (`aurora_assets::legacy_triangle_order`); `AURORA_MESH_ORDER=asset` keeps
/// the order of the asset, as before (#935). Read once.
pub fn legacy_triangle_order() -> bool {
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LEGACY.get_or_init(|| std::env::var("AURORA_MESH_ORDER").map_or(true, |v| v.trim() != "asset"))
}

/// Convert decoded mesh faces, optionally applying a transform (bind shape)
/// and remapping skin joints to rig indices (`joint_map[mesh joint] = palette index`).
pub fn mesh_to_faces(
    faces: &[aurora_assets::MeshFace],
    xform: Option<glam::Mat4>,
    joint_map: Option<&[u8]>,
) -> (Vec<FaceData>, Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let normal_m = xform.map(|m| glam::Mat3::from_mat4(m).inverse().transpose());
    let out = faces
        .iter()
        .map(|f| {
            if f.positions.is_empty() || f.indices.len() < 3 || f.positions.len() > 65536 {
                return None;
            }
            let mut verts: Vec<Vertex> = f
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut v = Vec3::from_array(*p);
                    let mut n = f.normals.get(i).map(|n| Vec3::from_array(*n)).unwrap_or(Vec3::Z);
                    if let (Some(m), Some(nm)) = (xform, normal_m) {
                        v = m.transform_point3(v);
                        n = nm * n;
                    }
                    let n = n.normalize_or(Vec3::Z);
                    min = min.min(v);
                    max = max.max(v);
                    let uv = f.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
                    Vertex::new(v.to_array(), n.to_array(), uv)
                })
                .collect();
            let mut indices = f.indices.clone();
            // (the vertex each one comes from, when the tangents split some)
            let mut sources: Option<Vec<u16>> = None;
            let scale = Vec3::from_array(f.normalized_scale.unwrap_or([1.0; 3]));
            if let Some(mut corners) = super::tangents::mesh_corner_tangents(&f.positions, &f.normals, &f.uvs, &f.indices, scale) {
                if let Some(m) = xform {
                    // a rigged face's tangents go through the bind shape like
                    // its positions, as the Legacy Viewer's skinning moves them
                    // (avatarSkinV.glsl matrices, materialV.glsl)
                    for (t, _) in &mut corners {
                        *t = m.transform_vector3(Vec3::from_array(*t)).normalize_or(Vec3::X).to_array();
                    }
                }
                match super::tangents::weld(&verts, &f.indices, &corners) {
                    Some(welded) => {
                        verts = welded.vertices;
                        indices = welded.indices;
                        sources = Some(welded.sources);
                    }
                    None => super::tangents::pack_volume_tangents(&mut verts, &f.indices),
                }
            }
            if legacy_triangle_order() {
                aurora_assets::legacy_triangle_order(&mut indices, verts.len());
            }
            let skin: Option<Vec<SkinVertex>> = match joint_map {
                Some(map) if !map.is_empty() && f.joints.len() == f.positions.len() && f.weights.len() == f.positions.len() => Some(
                    f.joints
                        .iter()
                        .zip(f.weights.iter())
                        .map(|(j, w)| {
                            let mut sv = SkinVertex::default();
                            for k in 0..4 {
                                // LLSkinningUtil::getPerVertexSkinMatrix:
                                // clamp in the mesh's palette before remapping.
                                sv.joints[k] = map[(j[k] as usize).min(map.len() - 1)];
                                sv.weights[k] = (w[k].clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                            sv
                        })
                        .collect(),
                ),
                _ => None,
            };
            let skin = match (skin, &sources) {
                (Some(skin), Some(sources)) => Some(sources.iter().map(|&s| skin[s as usize]).collect()),
                (skin, _) => skin,
            };
            Some(FaceGeom {
                vertices: verts,
                indices,
                skin,
            })
        })
        .collect();
    if min.x > max.x {
        min = Vec3::splat(-0.5);
        max = Vec3::splat(0.5);
    }
    (out, min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigged_skin_parity_bounds_indices_before_mapping_and_keeps_weights() {
        let face = aurora_assets::MeshFace {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            joints: vec![[0, 1, 2, 0], [3, 254, 255, 1], [2, 0, 1, 255]],
            weights: vec![[0.25; 4], [0.5, 0.25, 0.25, 0.0], [1.0, 0.0, 0.0, 0.0]],
            ..Default::default()
        };
        let (faces, _, _) = mesh_to_faces(std::slice::from_ref(&face), None, Some(&[7, 3, 9]));
        let vertices = faces[0].as_ref().unwrap().skin.as_ref().unwrap();
        assert_eq!(vertices[0].joints, [7, 3, 9, 7], "valid indices retain their mapping");
        assert_eq!(
            vertices[1].joints,
            [9, 9, 9, 3],
            "invalid indices clamp to the last mesh joint before mapping"
        );
        assert_eq!(vertices[1].weights, [128, 64, 64, 0]);
        assert_eq!(vertices[2].joints, [9, 7, 3, 9]);
        let (faces, _, _) = mesh_to_faces(&[face], None, Some(&[]));
        assert!(faces[0].as_ref().unwrap().skin.is_none(), "an empty palette cannot skin vertices");
    }

    #[test]
    fn alpha_classes() {
        let opaque = vec![255u8; 64];
        assert_eq!(classify_alpha(&opaque), AlphaKind::Opaque);
        let mut mask = vec![255u8; 400];
        for i in (3..200).step_by(4) {
            mask[i] = 0;
        }
        assert_eq!(classify_alpha(&mask), AlphaKind::Mask);
        let mut blend = vec![255u8; 400];
        for i in (3..400).step_by(4) {
            blend[i] = 128;
        }
        assert_eq!(classify_alpha(&blend), AlphaKind::Blend);
    }

    #[test]
    fn rgba_expand() {
        assert_eq!(to_rgba(1, &[10, 20], 2), vec![10, 10, 10, 255, 20, 20, 20, 255]);
        assert_eq!(to_rgba(3, &[1, 2, 3], 1), vec![1, 2, 3, 255]);
    }

    /// Tells on which thread it is dropped.
    struct Probe(std::sync::mpsc::Sender<std::thread::ThreadId>);

    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }

    #[test]
    fn discarded_values_are_freed_on_another_thread() {
        let (jobs, _results) = Jobs::new();
        let (tx, rx) = std::sync::mpsc::channel();
        // one value, then a gathered batch
        jobs.discard(Probe(tx.clone()));
        let mut trash = Trash::default();
        trash.empty(&jobs);
        trash.push(Probe(tx.clone()));
        trash.push(Probe(tx));
        assert_eq!(trash.len(), 2);
        trash.empty(&jobs);
        assert_eq!(trash.len(), 0);
        let here = std::thread::current().id();
        for _ in 0..3 {
            let dropped_on = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("value freed");
            assert_ne!(dropped_on, here);
        }
    }
}
