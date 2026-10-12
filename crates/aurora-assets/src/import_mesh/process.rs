//! Worker-side LOD reduction, normal generation and convex physics.
//! meshoptimizer replaces the legacy GLOD machinery; the visible choices
//! still control target triangles/error and preserve UV/face boundaries.

use super::*;
use parry3d::{math::Vector, transformation::try_convex_hull};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Auto,
    Reliable,
    Precise,
    Fast,
}
#[derive(Debug, Clone, Copy)]
pub struct LodOptions {
    pub triangles: usize,
    pub error: f32,
    pub method: Method,
    pub crease_angle: Option<f32>,
    pub error_only: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct PhysicsOptions {
    pub resolution: u32,
    pub max_hulls: u32,
    pub max_vertices: u32,
    pub tolerance: f32,
    /// V-HACD 4.1: 0 Flood, 1 Surface Only, 2 Raycast.
    pub fill_mode: u32,
}
impl Default for PhysicsOptions {
    fn default() -> Self {
        Self {
            resolution: 400000,
            max_hulls: 8,
            max_vertices: 32,
            tolerance: 1.0,
            fill_mode: 0,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Hull {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
}
#[derive(Debug, Clone, Default)]
pub struct Physics {
    pub hulls: Vec<Hull>,
    pub base: Option<Hull>,
}

/// Port of ll_is_degenerate (indra/llprimitive/llmodel.cpp, LGPL 2.1).
/// The asymmetric edge comparisons match the reference's tolerance rule.
pub fn degenerate([a, b, c]: [Vec3; 3]) -> bool {
    let (ab, ac, cb) = (a - b, a - c, c - b);
    let (x, y, z) = (ab.length_squared(), ac.length_squared(), cb.length_squared());
    let balanced = x <= 100.0 * y && x <= 100.0 * z && y <= 100.0 * x && x <= 100.0 * z && z <= 100.0 * x && x <= 100.0 * y;
    let tolerance = if balanced { 1e-11 } else { 1e-7 };
    ab.cross(ac).length_squared() < tolerance || (b - a).cross(b - c).length_squared() < tolerance || x * z - ab.dot(cb).powi(2) == 0.0
}

pub fn lod(model: &Model, options: LodOptions, cancel: &AtomicBool) -> Result<Model, AssetError> {
    if !options.error.is_finite() || !(0.0..=1.0).contains(&options.error) {
        return Err(AssetError::invalid("LOD error threshold"));
    }
    let source_triangles = model.triangles().max(1);
    let mut reduced = model.clone();
    if options.method == Method::Auto {
        // One triangle budget across the model, with material boundaries
        // represented by separate vertex ranges. Unlike a per-face budget,
        // this spends triangles where the geometry actually needs them.
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut ranges = Vec::new();
        for face in &model.faces {
            face.validate()?;
            let start = vertices.len() as u32;
            ranges.push((start, start + face.vertices.len() as u32));
            vertices.extend_from_slice(&face.vertices);
            indices.extend(face.indices.iter().map(|i| start + *i));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(AssetError::invalid("cancelled"));
        }
        let target = options.triangles.saturating_mul(3).min(indices.len());
        let output = if target == 0 && !options.error_only {
            Vec::new()
        } else {
            meshopt::simplify_decoder(&indices, &vertices, target, options.error, meshopt::SimplifyOptions::None, None)
        };
        for face in &mut reduced.faces {
            face.indices.clear();
        }
        for triangle in output.as_chunks::<3>().0 {
            if let Some((face, (start, _))) = ranges
                .iter()
                .enumerate()
                .find(|(_, (start, end))| triangle.iter().all(|i| i >= start && i < end))
            {
                reduced.faces[face].indices.extend(triangle.iter().map(|i| i - *start));
            } else {
                return Err(AssetError::invalid("LOD crossed a material boundary"));
            }
        }
        for face in &mut reduced.faces {
            face.compact();
            if let Some(angle) = options.crease_angle {
                normals(face, angle)?;
            }
        }
        return Ok(reduced);
    }
    for face in &mut reduced.faces {
        if cancel.load(Ordering::Relaxed) {
            return Err(AssetError::invalid("cancelled"));
        }
        let target = (face.indices.len() / 3 * options.triangles / source_triangles) * 3;
        let flags = match options.method {
            Method::Reliable => meshopt::SimplifyOptions::LockBorder,
            Method::Auto | Method::Precise => meshopt::SimplifyOptions::None,
            Method::Fast => meshopt::SimplifyOptions::Permissive | meshopt::SimplifyOptions::Prune,
        };
        face.indices = if target == 0 && !options.error_only {
            Vec::new()
        } else if options.method == Method::Fast {
            meshopt::simplify_sloppy_decoder(&face.indices, &face.vertices, target.min(face.indices.len()), options.error, None)
        } else {
            meshopt::simplify_decoder(
                &face.indices,
                &face.vertices,
                target.min(face.indices.len()),
                options.error,
                flags,
                None,
            )
        };
        face.compact();
        if let Some(angle) = options.crease_angle {
            normals(face, angle)?;
        }
        face.validate()?;
    }
    Ok(reduced)
}

pub fn normals(face: &mut Face, crease_degrees: f32) -> Result<(), AssetError> {
    if !crease_degrees.is_finite() || !(0.0..=180.0).contains(&crease_degrees) {
        return Err(AssetError::invalid("normal crease angle"));
    }
    use std::collections::HashMap;
    face.validate_geometry()?;
    let mut by_position = HashMap::<[u32; 3], Vec<Vec3>>::new();
    let key = |p: [f32; 3]| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    for triangle in face.indices.as_chunks::<3>().0 {
        let point = |i: u32| Vec3::from_array(face.vertices[i as usize].position);
        let n = (point(triangle[1]) - point(triangle[0]))
            .cross(point(triangle[2]) - point(triangle[0]))
            .normalize_or_zero();
        for index in triangle {
            by_position.entry(key(face.vertices[*index as usize].position)).or_default().push(n);
        }
    }
    let threshold = crease_degrees.to_radians().cos();
    for vertex in &mut face.vertices {
        let original = Vec3::from_array(vertex.normal).normalize_or_zero();
        if let Some(adjacent) = by_position.get(&key(vertex.position)) {
            let reference = if original == Vec3::ZERO { adjacent[0] } else { original };
            let n = adjacent
                .iter()
                .filter(|n| n.dot(reference) >= threshold - 0.000001)
                .copied()
                .fold(Vec3::ZERO, |sum, n| sum + n)
                .normalize_or_zero();
            vertex.normal = n.to_array();
        }
    }
    Ok(())
}

fn geometry(model: &Model) -> Result<(Vec<Vector>, Vec<[u32; 3]>), AssetError> {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    for face in &model.faces {
        face.validate()?;
        let start = vertices.len() as u32;
        vertices.extend(
            face.vertices
                .iter()
                .map(|v| Vector::new(v.position[0], v.position[1], v.position[2])),
        );
        for t in face.indices.as_chunks::<3>().0 {
            triangles.push([start + t[0], start + t[1], start + t[2]]);
        }
    }
    if vertices.len() > 1024 * 1024 || triangles.is_empty() {
        return Err(AssetError::TooLarge("physics geometry"));
    }
    Ok((vertices, triangles))
}
pub fn base_hull(model: &Model) -> Result<Hull, AssetError> {
    let (vertices, _) = geometry(model)?;
    let (points, indices) = match try_convex_hull(&vertices) {
        Ok(hull) => hull,
        Err(_) => {
            let mut min = Vector::splat(f32::MAX);
            let mut max = Vector::splat(f32::MIN);
            for p in &vertices {
                min = min.min(*p);
                max = max.max(*p);
            }
            let center = (min + max) * 0.5;
            let size = (max - min).max(Vector::splat(0.0001));
            if (max - min).length_squared() < 0.000000001 {
                return Err(AssetError::invalid("degenerate physics points"));
            }
            let corners = (0..8)
                .map(|i| {
                    center
                        + Vector::new(
                            if i & 1 == 0 { -0.5 } else { 0.5 },
                            if i & 2 == 0 { -0.5 } else { 0.5 },
                            if i & 4 == 0 { -0.5 } else { 0.5 },
                        ) * size
                })
                .collect::<Vec<_>>();
            try_convex_hull(&corners).map_err(|_| AssetError::invalid("physics bounding hull"))?
        }
    };
    bounded_hull(points, indices, 256)
}
fn bounded_hull(points: Vec<Vector>, indices: Vec<[u32; 3]>, maximum: usize) -> Result<Hull, AssetError> {
    if points.len() <= maximum {
        return Ok(Hull {
            vertices: points.iter().map(|p| [p.x, p.y, p.z]).collect(),
            indices,
        });
    }
    // Farthest-point sampling preserves extremes and distributes the limited
    // hull vertices over its surface. Distances are updated incrementally;
    // it never compares all pairs or allocates a quadratic matrix.
    let mut selected = Vec::new();
    let mut distances = vec![f32::MAX; points.len()];
    let mut next = points
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.x.total_cmp(&b.1.x))
        .map(|(i, _)| i)
        .ok_or_else(|| AssetError::invalid("empty hull"))?;
    while selected.len() < maximum {
        selected.push(points[next]);
        for (i, p) in points.iter().enumerate() {
            distances[i] = distances[i].min((*p - points[next]).length_squared());
        }
        let Some((index, distance)) = distances.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else {
            break;
        };
        if *distance <= 0.0000000001 {
            break;
        }
        next = index;
    }
    if selected.len() == 3 {
        return Ok(Hull {
            vertices: selected.iter().map(|p| [p.x, p.y, p.z]).collect(),
            indices: vec![[0, 1, 2]],
        });
    }
    let (points, indices) = try_convex_hull(&selected).map_err(|_| AssetError::invalid("simplified hull"))?;
    Ok(Hull {
        vertices: points.iter().map(|p| [p.x, p.y, p.z]).collect(),
        indices,
    })
}
pub fn analyze(model: &Model, options: PhysicsOptions, cancel: &AtomicBool) -> Result<Physics, AssetError> {
    analyze_progress(model, options, cancel, &AtomicU32::new(0))
}
pub fn analyze_progress(model: &Model, options: PhysicsOptions, cancel: &AtomicBool, progress: &AtomicU32) -> Result<Physics, AssetError> {
    if !(1000..=16000000).contains(&options.resolution)
        || !(1..=256).contains(&options.max_hulls)
        || !(3..=256).contains(&options.max_vertices)
        || !options.tolerance.is_finite()
        || !(0.0001..=99.0).contains(&options.tolerance)
        || options.fill_mode > 2
    {
        return Err(AssetError::invalid("physics parameters"));
    }
    let (vertices, triangles) = geometry(model)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AssetError::invalid("cancelled"));
    }
    let points: Vec<_> = vertices.iter().map(|p| [p.x, p.y, p.z]).collect();
    let hulls = super::vhacd::analyze(&points, &triangles, options, cancel, progress)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(AssetError::invalid("cancelled"));
    }
    if hulls.is_empty() || hulls.len() > 256 {
        return Err(AssetError::invalid("empty or excessive physics hulls"));
    }
    Ok(Physics {
        hulls,
        base: Some(base_hull(model)?),
    })
}
