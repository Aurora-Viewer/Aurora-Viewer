//! Bounded, versioned rest-mesh snapshots. No file paths or simulator assets
//! are accepted. Skinning uses the normal local mesh pipeline (VJLocalMeshObject,
//! indra/newview/vjlocalmesh.cpp); the live transport is an Aurora extension.

use aurora_assets::import_mesh::{self, Instance, Model, Scene, Skin, Vertex, encode};
use glam::{Mat4, Vec3};
use serde::Deserialize;
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;
#[path = "appearance.rs"]
mod appearance;
pub use appearance::{Image, Material, PreparedImage, STAGING_BUSY};

pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_OBJECTS: usize = 32;
pub(super) const MAX_CORNERS: usize = 600_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub protocol: u32,
    pub revision: u64,
    pub meshes: Vec<Mesh>,
    #[serde(default)]
    pub images: Vec<Image>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mesh {
    pub name: String,
    #[serde(default)]
    pub source_id: Option<Uuid>,
    #[serde(default)]
    pub link: Option<Link>,
    pub joints: Vec<String>,
    /// Column-major inverse binds in metres, in SL's Z-up coordinate frame.
    pub inverse_bind: Vec<[f32; 16]>,
    pub faces: Vec<Surface>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub color: [f32; 4],
    #[serde(default)]
    pub material: Option<Material>,
    pub vertices: Vec<Corner>,
    pub indices: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Corner {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub joints: [u8; 4],
    pub weights: [f32; 4],
}

#[derive(Clone)]
pub struct Prepared {
    pub revision: u64,
    pub meshes: Vec<PreparedMesh>,
    pub triangles: usize,
    pub images: Vec<PreparedImage>,
    pub sources: Vec<PreparedSource>,
}

#[derive(Clone)]
pub struct PreparedSource {
    pub id: Uuid,
    pub name: String,
    pub link: Option<Link>,
    /// Immutable import input, shared with a draft when Upload is selected.
    pub scene: Arc<Scene>,
}

/// Static object origins in the same baked coordinate frame as the vertices.
/// Parentage is flattened to one root, matching LLSelectMgr's linkset family.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub parent: Uuid,
    pub offset: [f32; 3],
}
impl PreparedSource {
    pub fn rigged(&self) -> bool {
        self.scene.models.iter().any(|model| model.skin.is_some())
    }
}

#[derive(Clone)]
pub struct PreparedMesh {
    pub id: Uuid,
    pub source: Uuid,
    pub name: String,
    pub data: Arc<Vec<u8>>,
    pub header: aurora_assets::MeshHeader,
    pub skin: Option<Arc<aurora_assets::SkinInfo>>,
    /// All render batches of a source share the same normalization bounds.
    pub size: Vec3,
    pub center: Vec3,
    pub colors: Vec<[f32; 4]>,
    pub materials: Vec<Option<(Uuid, aurora_assets::material::PbrMaterial)>>,
}

pub fn prepare(bytes: &[u8], supported: &HashSet<String>) -> Result<Prepared, String> {
    prepare_staged(bytes, supported, None, &Default::default())
}

pub fn prepare_staged(
    bytes: &[u8],
    supported: &HashSet<String>,
    pool: Option<&aurora_render::StagingPool>,
    cache: &std::collections::HashMap<Uuid, PreparedImage>,
) -> Result<Prepared, String> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(aurora_i18n::tr!("blender-live-invalid-size").into());
    }
    let snapshot: Snapshot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    prepare_snapshot(snapshot, supported, pool, cache)
}

pub(super) fn prepare_snapshot(
    snapshot: Snapshot,
    supported: &HashSet<String>,
    pool: Option<&aurora_render::StagingPool>,
    cache: &std::collections::HashMap<Uuid, PreparedImage>,
) -> Result<Prepared, String> {
    if snapshot.protocol != VERSION || snapshot.meshes.len() > MAX_OBJECTS {
        return Err(aurora_i18n::tr!("blender-live-invalid-protocol").into());
    }
    let images = appearance::images(snapshot.images, pool, cache)?;
    let mut total = 0usize;
    let mut total_indices = 0usize;
    let mut meshes = Vec::new();
    let mut triangles = 0;
    let mut sources = Vec::new();
    let mut source_ids = HashSet::new();
    let mut occurrences = std::collections::HashMap::<String, u32>::new();
    for mesh in snapshot.meshes {
        if mesh.name.len() > 256 || mesh.name.chars().any(char::is_control) {
            return Err(aurora_i18n::tr!("blender-live-invalid-name").into());
        }
        // Older add-ons identify an object by name. Occurrences keep even
        // ambiguous legacy names separate; new add-ons also survive renaming.
        let occurrence = occurrences.entry(mesh.name.clone()).or_default();
        let source = mesh.source_id.unwrap_or_else(|| {
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new();
            hash.update(b"Aurora Blender Live source v1");
            hash.update(occurrence.to_le_bytes());
            hash.update(mesh.name.as_bytes());
            let mut bytes = [0; 16];
            bytes.copy_from_slice(&hash.finalize()[..16]);
            Uuid::from_bytes(bytes)
        });
        *occurrence += 1;
        if source.is_nil() || !source_ids.insert(source) {
            return Err(aurora_i18n::tr!("blender-live-invalid-source").into());
        }
        let mut scene = Scene::default();
        let count = mesh.joints.len();
        if count > 110 || count != mesh.inverse_bind.len() {
            return Err(aurora_i18n::tr!("blender-live-invalid-joints").into());
        }
        let mut names = HashSet::new();
        for name in &mesh.joints {
            if !supported.contains(name) || !names.insert(name) {
                return Err(aurora_i18n::tr!("blender-live-unknown-joint", joint = name.as_str()));
            }
        }
        let inverse_bind: Vec<_> = mesh.inverse_bind.iter().map(Mat4::from_cols_array).collect();
        if inverse_bind.iter().any(|m| {
            !m.is_finite()
                || !m.determinant().is_finite()
                || m.determinant().abs() < 1e-12
                || m.x_axis.w != 0.0
                || m.y_axis.w != 0.0
                || m.z_axis.w != 0.0
                || m.w_axis.w != 1.0
        }) {
            return Err(aurora_i18n::tr!("blender-live-invalid-bind").into());
        }
        if mesh.faces.is_empty() || mesh.faces.len() > 32 {
            return Err(aurora_i18n::tr!("blender-live-invalid-surfaces").into());
        }
        let mut faces = Vec::new();
        let mut colors = Vec::new();
        let mut materials = Vec::new();
        for surface in mesh.faces {
            total = total.saturating_add(surface.vertices.len());
            total_indices = total_indices.saturating_add(surface.indices.len());
            if total > MAX_CORNERS || total_indices > MAX_CORNERS * 3 {
                return Err(aurora_i18n::tr!("blender-live-invalid-geometry").into());
            }
            if surface.vertices.is_empty()
                || surface.indices.is_empty()
                || surface.color.iter().any(|f| !f.is_finite() || !(0.0..=1.0).contains(f))
            {
                return Err(aurora_i18n::tr!("blender-live-invalid-color").into());
            }
            let mut vertices = Vec::with_capacity(surface.vertices.len());
            for v in surface.vertices {
                let sum: f32 = v.weights.iter().sum();
                let invalid_skin = if count == 0 {
                    v.weights != [0.0; 4] || v.joints != [0; 4]
                } else {
                    sum <= 0.000001 || v.joints.iter().any(|j| *j as usize >= count)
                };
                if !sum.is_finite()
                    || invalid_skin
                    || v.weights.iter().any(|w| *w < 0.0)
                    || !Vec3::from_array(v.normal).is_finite()
                    || Vec3::from_array(v.normal).length_squared() < 1e-12
                {
                    return Err(aurora_i18n::tr!("blender-live-invalid-weights").into());
                }
                vertices.push(Vertex {
                    position: v.position,
                    normal: Vec3::from_array(v.normal).normalize_or_zero().to_array(),
                    uv: v.uv,
                    joints: v.joints.map(u16::from),
                    weights: if count == 0 {
                        [0.0; 4]
                    } else {
                        v.weights.map(|weight| weight / sum)
                    },
                });
            }
            let material = colors.len();
            colors.push(surface.color);
            let value = surface.material.map(|value| value.prepare(surface.color, &images)).transpose()?;
            let pbr = value.clone().unwrap_or_else(|| aurora_assets::material::PbrMaterial {
                base_color_factor: surface.color,
                ..Default::default()
            });
            let textures = [
                pbr.base_color_texture,
                pbr.normal_texture,
                pbr.metallic_roughness_texture,
                pbr.emissive_texture,
            ];
            scene.materials.push(import_mesh::Material {
                name: format!("{} {}", mesh.name, material + 1),
                images: textures.map(|id| id.and_then(|id| images.iter().position(|image| image.id == id))),
                value: pbr,
            });
            materials.push(value);
            faces.extend(import_mesh::split_face(vertices, surface.indices, material).map_err(|e| e.to_string())?);
        }
        // Split materials and 16-bit vertex batches instead of truncating them.
        let (center, size) = import_mesh::normalization(&faces).map_err(|e| e.to_string())?;
        for (part, chunk) in faces.chunks(8).enumerate() {
            let mut model = Model {
                name: mesh.name.clone(),
                submodel: part as u32,
                faces: chunk.to_vec(),
                skin: (count != 0).then(|| Skin {
                    names: mesh.joints.clone(),
                    inverse_bind: inverse_bind.clone(),
                    alternate_bind: Vec::new(),
                    bind_shape: Mat4::IDENTITY,
                    pelvis_offset: 0.0,
                }),
                center: Vec3::ZERO,
                size: Vec3::ONE,
            };
            model.normalize_to(center, size).map_err(|e| e.to_string())?;
            triangles += model.triangles();
            scene.instances.push(Instance {
                model: scene.models.len(),
                transform: Mat4::from_scale_rotation_translation(size, glam::Quat::IDENTITY, center),
                name: mesh.name.clone(),
                materials: model.faces.iter().map(|face| face.material).collect(),
            });
            scene.normalized_bounds.insert(scene.models.len(), model.normalized_bounds());
            scene.models.push(model.clone());
            let colors = chunk.iter().map(|face| colors[face.material]).collect();
            let materials = chunk
                .iter()
                .map(|face| materials[face.material].clone().map(|value| (Uuid::new_v4(), value)))
                .collect();
            let data = encode::asset(
                &[None, None, None, Some(model)],
                None,
                &import_mesh::process::Physics::default(),
                encode::Options {
                    skin: count != 0,
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            let header = aurora_assets::parse_mesh_header(&data).map_err(|e| e.to_string())?;
            let skin = if count == 0 {
                None
            } else {
                let bytes = header
                    .skin
                    .and_then(|range| aurora_assets::mesh::section_bytes(&data, range))
                    .ok_or("Missing prepared skin")?;
                Some(Arc::new(aurora_assets::decode_skin(bytes).map_err(|_| "Invalid prepared skin")?))
            };
            meshes.push(PreparedMesh {
                id: Uuid::new_v4(),
                source,
                name: mesh.name.clone(),
                data: Arc::new(data),
                header,
                skin,
                size,
                center,
                colors,
                materials,
            });
        }
        sources.push(PreparedSource {
            id: source,
            name: mesh.name,
            link: mesh.link,
            scene: Arc::new(scene),
        });
    }
    for source in &sources {
        if let Some(link) = source.link
            && (source.rigged()
                || link.parent == source.id
                || !Vec3::from_array(link.offset).is_finite()
                || Vec3::from_array(link.offset).abs().max_element() > 65536.0
                || !sources
                    .iter()
                    .any(|parent| parent.id == link.parent && parent.link.is_none() && !parent.rigged()))
        {
            return Err(aurora_i18n::tr!("blender-live-invalid-link").into());
        }
    }
    // Parents are installed before children independently of Blender list order.
    sources.sort_by_key(|source| source.link.is_some());
    meshes.sort_by_key(|mesh| sources.iter().position(|source| source.id == mesh.source));
    Ok(Prepared {
        revision: snapshot.revision,
        meshes,
        triangles,
        images,
        sources,
    })
}
