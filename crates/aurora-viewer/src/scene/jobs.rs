//! Background work (rayon) and its results.

use super::GeomKey;
use aurora_render::{SkinVertex, Vertex};
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

pub enum JobResult {
    Geometry {
        key: GeomKey,
        faces: Vec<FaceData>,
        min: Vec3,
        max: Vec3,
        /// LLVolume::getSurfaceArea (sculpts; 1 otherwise).
        area: f32,
    },
    GeometryFailed {
        key: GeomKey,
    },
    Texture {
        id: Uuid,
        discard: u8,
        mips: Vec<(u32, u32, Vec<u8>)>,
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

#[derive(Clone)]
pub struct Jobs {
    tx: Sender<JobResult>,
    pub in_flight: Arc<AtomicUsize>,
}

impl Jobs {
    pub fn new() -> (Jobs, Receiver<JobResult>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        (
            Jobs {
                tx,
                in_flight: Arc::new(AtomicUsize::new(0)),
            },
            rx,
        )
    }

    pub fn spawn(&self, f: impl FnOnce() -> JobResult + Send + 'static) {
        let tx = self.tx.clone();
        let n = self.in_flight.clone();
        n.fetch_add(1, Ordering::Relaxed);
        rayon::spawn(move || {
            let r = f();
            n.fetch_sub(1, Ordering::Relaxed);
            let _ = tx.send(r);
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
