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
                    f.as_ref().map(|f| PickFace {
                        positions: f.vertices.iter().map(|v| v.pos).collect(),
                        uvs: f.vertices.iter().map(|v| v.uv).collect(),
                        indices: f.indices.clone(),
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
        sculpt: Option<Arc<SculptMap>>,
    },
    TextureFailed {
        id: Uuid,
    },
    TextureCache {
        id: Uuid,
        data: Option<Vec<u8>>,
        complete: bool,
    },
    MeshCache {
        id: Uuid,
        data: Option<Vec<u8>>,
    },
    Material {
        id: Uuid,
        material: Option<aurora_assets::PbrMaterial>,
    },
    /// Work whose result was delivered through another channel.
    Done,
}

/// Background jobs give way to the frame: while decodes keep every core
/// busy (arrival in a region), the main thread, its `par_iter` workers and
/// the encoder threads would otherwise wait for a core behind them (frames
/// of 2.4 ms stretched to 5–8 ms in AURORA_DEMO_STREAM). Idle cores still go
/// to the jobs, so loading is not slower.
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
}

impl JobResult {
    /// A geometry built by a job, prepared for the GPU on the job's thread.
    pub fn geometry(key: GeomKey, (faces, min, max): (Vec<FaceData>, Vec3, Vec3), area: f32, pool: Option<&StagingPool>) -> Self {
        let pick = !matches!(key, GeomKey::AvatarPart(_));
        JobResult::Geometry {
            key,
            geom: PreparedGeom::new(faces, min, max, pick, pool),
            area,
        }
    }
}

#[derive(Clone)]
pub struct Jobs {
    tx: Sender<JobResult>,
    pub in_flight: Arc<AtomicUsize>,
    /// Background pool, apart from the global one used by the frame
    /// (`par_iter` of culling and poses): a frame never waits behind a
    /// JPEG 2000 decode or a disk read.
    pool: Arc<rayon::ThreadPool>,
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

    pub fn pending(&self) -> usize {
        self.in_flight.load(Ordering::Relaxed)
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
            let verts: Vec<Vertex> = f
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
            let verts: Vec<Vertex> = f
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
            let skin = match joint_map {
                Some(map) if f.joints.len() == f.positions.len() && f.weights.len() == f.positions.len() => Some(
                    f.joints
                        .iter()
                        .zip(f.weights.iter())
                        .map(|(j, w)| {
                            let mut sv = SkinVertex::default();
                            for k in 0..4 {
                                sv.joints[k] = map.get(j[k] as usize).copied().unwrap_or(0);
                                sv.weights[k] = (w[k].clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                            sv
                        })
                        .collect(),
                ),
                _ => None,
            };
            Some(FaceGeom {
                vertices: verts,
                indices: f.indices.clone(),
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
}
