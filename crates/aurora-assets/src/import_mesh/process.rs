//! Worker-side LOD reduction, normal generation and convex physics.
//! meshoptimizer replaces the legacy GLOD machinery; the visible choices
//! still control target triangles/error and preserve material slots.

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

fn shadow_indices(model: &Model, ranges: &[(u32, u32)], attributes: u8) -> Vec<u32> {
    let mut shadow = Vec::with_capacity(model.triangles() * 3);
    for (face, (start, _)) in model.faces.iter().zip(ranges) {
        let mut welded = std::collections::HashMap::new();
        for index in &face.indices {
            let vertex = &face.vertices[*index as usize];
            let bits = |value: f32| if value == 0.0 { 0 } else { value.to_bits() };
            let mut key = [0; 8];
            key[..3].copy_from_slice(&vertex.position.map(bits));
            if attributes == 2 {
                key[3..6].copy_from_slice(&vertex.normal.map(bits));
            }
            if attributes > 0 {
                key[6..].copy_from_slice(&vertex.uv.map(bits));
            }
            let key = (key, vertex.joints, vertex.weights.map(bits));
            let representative = *welded.entry(key).or_insert(*start + *index);
            shadow.push(representative);
        }
    }
    shadow
}

fn simplify_shadow(indices: &[u32], vertices: &[Vertex], target: usize, error: f32) -> Vec<u32> {
    // Unused copies at a welded position still look like locked seams to
    // meshoptimizer. Compact its input, then map back to the original slots.
    let mut remap = vec![u32::MAX; vertices.len()];
    let mut original = Vec::new();
    let mut compact_vertices = Vec::new();
    let compact_indices = indices
        .iter()
        .map(|index| {
            let mapped = &mut remap[*index as usize];
            if *mapped == u32::MAX {
                *mapped = original.len() as u32;
                original.push(*index);
                compact_vertices.push(vertices[*index as usize].clone());
            }
            *mapped
        })
        .collect::<Vec<_>>();
    meshopt::simplify_decoder(
        &compact_indices,
        &compact_vertices,
        target,
        error,
        meshopt::SimplifyOptions::None,
        None,
    )
    .into_iter()
    .map(|index| original[index as usize])
    .collect()
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
        let mut ranges = Vec::new();
        for face in &model.faces {
            face.validate()?;
            let start = vertices.len() as u32;
            ranges.push((start, start + face.vertices.len() as u32));
            vertices.extend_from_slice(&face.vertices);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(AssetError::invalid("cancelled"));
        }
        // Firestorm's FULL pass also welds identical exported corners, in
        // both triangle and error modes, without relaxing any attributes.
        let indices = shadow_indices(model, &ranges, 2);
        let target = if options.error_only {
            3.min(indices.len())
        } else {
            options.triangles.saturating_mul(3).min(indices.len())
        };
        let mut output = if target == 0 && !options.error_only {
            Vec::new()
        } else {
            simplify_shadow(&indices, &vertices, target, options.error)
        };
        // Port of LLModelPreview::genMeshOptimizerLODs (llmodelpreview.cpp,
        // LGPL 2.1): AUTO progressively welds normal/UV seams
        // when precise simplification misses the triangle budget by > 1.8x.
        // Keep each material's vertex range separate so no face changes slots.
        if !options.error_only && target > 0 {
            for attributes in [1, 0] {
                if output.len() as f64 <= target as f64 * 1.8 {
                    break;
                }
                if cancel.load(Ordering::Relaxed) {
                    return Err(AssetError::invalid("cancelled"));
                }
                let shadow = shadow_indices(model, &ranges, attributes);
                let candidate = simplify_shadow(&shadow, &vertices, target, options.error);
                if !candidate.is_empty() && candidate.len() < output.len() {
                    output = candidate;
                }
            }
            if output.len() as f64 > target as f64 * 1.8 {
                let mut candidate = Vec::new();
                for (face, (start, _)) in model.faces.iter().zip(&ranges) {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(AssetError::invalid("cancelled"));
                    }
                    if face.indices.is_empty() {
                        continue;
                    }
                    let budget = (face.indices.len() / 3 * options.triangles / source_triangles).max(1) * 3;
                    let reduced = meshopt::simplify_sloppy_decoder(
                        &face.indices,
                        &face.vertices,
                        budget.min(face.indices.len()),
                        options.error,
                        None,
                    );
                    candidate.extend(reduced.iter().map(|index| *start + *index));
                }
                if !candidate.is_empty() && candidate.len() < output.len() {
                    output = candidate;
                }
            }
        }
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
        let target = if options.error_only {
            3.min(face.indices.len())
        } else {
            (face.indices.len() / 3 * options.triangles / source_triangles) * 3
        };
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

/// LLDAELoader::addVolumeFacesFromDomMesh: missing normals sum connected
/// unit triangle normals at each authored vertex without changing indices.
pub fn missing_normals(face: &mut Face) -> Result<(), AssetError> {
    face.validate_geometry()?;
    let mut sums = super::allocate(face.vertices.len(), Vec3::ZERO, "mesh normals memory")?;
    for triangle in face.indices.as_chunks::<3>().0 {
        let point = |i: u32| Vec3::from_array(face.vertices[i as usize].position);
        let normal = (point(triangle[1]) - point(triangle[0]))
            .cross(point(triangle[2]) - point(triangle[0]))
            .normalize_or_zero();
        for &index in triangle {
            sums[index as usize] += normal;
        }
    }
    for (vertex, sum) in face.vertices.iter_mut().zip(sums) {
        vertex.normal = sum.normalize_or_zero().to_array();
    }
    Ok(())
}

pub fn normals(face: &mut Face, crease_degrees: f32) -> Result<(), AssetError> {
    if !crease_degrees.is_finite() || !(0.0..=180.0).contains(&crease_degrees) {
        return Err(AssetError::invalid("normal crease angle"));
    }
    use std::collections::HashMap;
    face.validate_geometry()?;
    // LLModel::generateNormals skips faces whose faceted copy cannot use
    // 16-bit indices, including the fully smooth setting.
    if face.indices.len() > u16::MAX as usize {
        return Ok(());
    }
    let key = |p: [f32; 3]| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() });
    if crease_degrees == 180.0 {
        // Keep authored vertex indices stable until Collada has associated
        // controller weights. Dense indexed faces need one sum per position,
        // rather than storing every adjacent triangle a second time.
        let mut sums = HashMap::<[u32; 3], Vec3>::new();
        for triangle in face.indices.as_chunks::<3>().0 {
            let point = |i: u32| Vec3::from_array(face.vertices[i as usize].position);
            let area = (point(triangle[1]) - point(triangle[0])).cross(point(triangle[2]) - point(triangle[0]));
            for &index in triangle {
                *sums.entry(key(face.vertices[index as usize].position)).or_default() += area;
            }
        }
        for vertex in &mut face.vertices {
            vertex.normal = sums
                .get(&key(vertex.position))
                .copied()
                .unwrap_or(Vec3::ZERO)
                .normalize_or_zero()
                .to_array();
        }
        return Ok(());
    }
    let mut by_position = HashMap::<[u32; 3], Vec<(Vec3, Vec3)>>::new();
    for triangle in face.indices.as_chunks::<3>().0 {
        let point = |i: u32| Vec3::from_array(face.vertices[i as usize].position);
        let area = (point(triangle[1]) - point(triangle[0])).cross(point(triangle[2]) - point(triangle[0]));
        let n = area.normalize_or_zero();
        for index in triangle {
            by_position
                .entry(key(face.vertices[*index as usize].position))
                .or_default()
                .push((n, area));
        }
    }
    let threshold = crease_degrees.to_radians().cos();
    // LLModel::generateNormals facets first: one authored vertex may need
    // different normals on opposite sides of a crease. UVs/weights survive.
    let mut vertices = Vec::new();
    let mut indices = Vec::with_capacity(face.indices.len());
    let mut remap = HashMap::new();
    for triangle in face.indices.as_chunks::<3>().0 {
        let point = |i: u32| Vec3::from_array(face.vertices[i as usize].position);
        let reference = (point(triangle[1]) - point(triangle[0]))
            .cross(point(triangle[2]) - point(triangle[0]))
            .normalize_or_zero();
        for &index in triangle {
            let mut vertex = face.vertices[index as usize].clone();
            vertex.normal = by_position[&key(vertex.position)]
                .iter()
                .filter(|(normal, _)| normal.dot(reference) >= threshold - 0.000001)
                .fold(Vec3::ZERO, |sum, (_, area)| sum + *area)
                .normalize_or_zero()
                .to_array();
            let identity = (
                key(vertex.position),
                key(vertex.normal),
                vertex.uv.map(f32::to_bits),
                vertex.joints,
                vertex.weights.map(f32::to_bits),
            );
            let next = vertices.len() as u32;
            let mapped = *remap.entry(identity).or_insert_with(|| {
                vertices.push(vertex);
                next
            });
            indices.push(mapped);
        }
    }
    face.vertices = vertices;
    face.indices = indices;
    face.validate()?;
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
    // generateHulls needs positions only. Copying the triangle list here
    // wastes substantial working memory for dense indexed source faces.
    let mut vertices = Vec::new();
    for face in &model.faces {
        face.validate()?;
        vertices.extend(
            face.vertices
                .iter()
                .map(|v| Vector::new(v.position[0], v.position[1], v.position[2])),
        );
    }
    if vertices.is_empty() || model.faces.iter().all(|face| face.indices.is_empty()) {
        return Err(AssetError::invalid("empty physics geometry"));
    }
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
