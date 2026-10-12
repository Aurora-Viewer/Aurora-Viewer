//! Mesh imports and upload preparation. Reference: LLDAELoader, LLGLTFLoader,
//! LLModelPreview, LLModel::writeModel and LLMeshUploadThread (Firestorm,
//! originally LGPL 2.1). Geometry parsing and processing are pure worker jobs.

pub mod collada;
pub mod encode;
pub mod gltf;
pub mod process;
pub mod skin;
#[cfg(test)]
mod tests;
mod vhacd;

use crate::{AssetError, PbrMaterial};
use glam::{Mat4, Vec3};

#[derive(Debug, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub joints: [u8; 4],
    pub weights: [f32; 4],
}
impl meshopt::DecodePosition for Vertex {
    fn decode_position(&self) -> [f32; 3] {
        self.position
    }
}
#[derive(Debug, Clone)]
pub struct Face {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub material: usize,
}
#[derive(Debug, Clone)]
pub struct Skin {
    pub names: Vec<String>,
    pub inverse_bind: Vec<Mat4>,
    pub alternate_bind: Vec<Mat4>,
    pub bind_shape: Mat4,
    pub pelvis_offset: f32,
}
#[derive(Debug, Clone)]
pub struct Model {
    pub name: String,
    /// Extra faces of the same source geometry share a root physics shape.
    pub submodel: u32,
    pub faces: Vec<Face>,
    pub skin: Option<Skin>,
    pub center: Vec3,
    pub size: Vec3,
}
#[derive(Debug, Clone)]
pub struct Instance {
    pub model: usize,
    pub transform: Mat4,
    pub name: String,
    pub materials: Vec<usize>,
}
#[derive(Debug, Clone)]
pub struct Material {
    pub name: String,
    pub value: PbrMaterial,
    pub images: [Option<usize>; 4],
}
#[derive(Debug, Clone, Default)]
pub struct Scene {
    pub models: Vec<Model>,
    pub instances: Vec<Instance>,
    pub materials: Vec<Material>,
    pub warnings: Vec<String>,
    pub image_uris: std::collections::HashMap<usize, String>,
}

impl Face {
    pub fn validate(&self) -> Result<(), AssetError> {
        if self.vertices.len() > 65534 || self.indices.len() > 3 * 1024 * 1024 {
            return Err(AssetError::TooLarge("mesh face"));
        }
        self.validate_geometry()
    }
    pub fn validate_geometry(&self) -> Result<(), AssetError> {
        if self.vertices.len() > 4 * 1024 * 1024 || self.indices.len() > 12 * 1024 * 1024 {
            return Err(AssetError::TooLarge("mesh primitive"));
        }
        if !self.indices.len().is_multiple_of(3) || self.indices.iter().any(|i| *i as usize >= self.vertices.len()) {
            return Err(AssetError::invalid("mesh triangle indices"));
        }
        if self.vertices.iter().any(|v| {
            !v.position
                .iter()
                .chain(v.normal.iter())
                .chain(v.uv.iter())
                .chain(v.weights.iter())
                .all(|f| f.is_finite())
                || v.weights.iter().any(|w| *w < 0.0)
        }) {
            return Err(AssetError::invalid("nonfinite mesh attribute or invalid skin weight"));
        }
        Ok(())
    }
    pub fn compact(&mut self) {
        let mut map = vec![u32::MAX; self.vertices.len()];
        let mut vertices = Vec::new();
        for index in &mut self.indices {
            let slot = &mut map[*index as usize];
            if *slot == u32::MAX {
                *slot = vertices.len() as u32;
                vertices.push(self.vertices[*index as usize].clone());
            }
            *index = *slot;
        }
        self.vertices = vertices;
    }
}
impl Model {
    pub fn normalize(&mut self) -> Result<(), AssetError> {
        let (center, size) = normalization(&self.faces)?;
        self.normalize_to(center, size)
    }
    pub fn normalize_to(&mut self, center: Vec3, size: Vec3) -> Result<(), AssetError> {
        if self.faces.is_empty() || self.faces.len() > 8 {
            return Err(AssetError::invalid("mesh model needs between one and eight faces"));
        }
        for face in &self.faces {
            face.validate()?;
        }
        self.center = center;
        self.size = size;
        if !self.center.is_finite() || !self.size.is_finite() {
            return Err(AssetError::invalid("mesh bounds overflow"));
        }
        if let Some(skin) = &mut self.skin {
            skin.bind_shape *= Mat4::from_scale_rotation_translation(self.size, glam::Quat::IDENTITY, self.center);
        }
        for face in &mut self.faces {
            for vertex in &mut face.vertices {
                vertex.position = ((Vec3::from_array(vertex.position) - self.center) / self.size).to_array();
                vertex.normal = (Vec3::from_array(vertex.normal) * self.size).normalize_or_zero().to_array();
            }
        }
        Ok(())
    }
    pub fn triangles(&self) -> usize {
        self.faces.iter().map(|f| f.indices.len() / 3).sum()
    }
}

/// LLDAELoader normalizes the entire source before splitting extra faces.
pub fn normalization(faces: &[Face]) -> Result<(Vec3, Vec3), AssetError> {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for face in faces {
        face.validate()?;
        for vertex in &face.vertices {
            let point = Vec3::from_array(vertex.position);
            min = min.min(point);
            max = max.max(point);
        }
    }
    if !min.is_finite() || min.x == f32::MAX {
        return Err(AssetError::invalid("empty mesh"));
    }
    Ok(((min + max) * 0.5, (max - min).max(Vec3::splat(0.00001))))
}

/// Split a primitive by both the 16-bit vertex limit and the eight-face
/// limit, preserving every triangle and every material instead of trimming.
pub fn split_face(vertices: Vec<Vertex>, indices: Vec<u32>, material: usize) -> Result<Vec<Face>, AssetError> {
    use std::collections::HashMap;
    if vertices.len() > 4 * 1024 * 1024
        || indices.len() > 12 * 1024 * 1024
        || !indices.len().is_multiple_of(3)
        || indices.iter().any(|i| *i as usize >= vertices.len())
    {
        return Err(AssetError::invalid("mesh primitive size or indices"));
    }
    let mut parts = Vec::new();
    let mut map = HashMap::new();
    let mut face = Face {
        vertices: Vec::new(),
        indices: Vec::new(),
        material,
    };
    for triangle in indices.as_chunks::<3>().0 {
        let new = triangle.iter().filter(|i| !map.contains_key(*i)).count();
        if face.vertices.len() + new > 65534 {
            face.validate()?;
            parts.push(face);
            face = Face {
                vertices: Vec::new(),
                indices: Vec::new(),
                material,
            };
            map.clear();
        }
        for old in triangle {
            let index = *map.entry(*old).or_insert_with(|| {
                let i = face.vertices.len() as u32;
                face.vertices.push(vertices[*old as usize].clone());
                i
            });
            face.indices.push(index);
        }
    }
    if !face.indices.is_empty() {
        face.validate()?;
        parts.push(face);
    }
    Ok(parts)
}
