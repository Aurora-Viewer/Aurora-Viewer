//! Versioned private visual snapshots, never object/script/inventory assets.
//! Named joints survive changes to the viewer's internal skeleton ordering.
use super::*;
use aurora_render::preview::{Image, Material, Part, Skin, Vertex};
use base64::{Engine, engine::general_purpose::STANDARD};
use glam::{Mat4, Vec3};
use std::io::{Read, Write};

const MAGIC: &[u8] = b"AURORA-ANIMATION-MODEL-1\n";
const LIMIT: u64 = 256 << 20;
#[cfg(test)]
mod tests;

#[derive(Serialize, Deserialize)]
struct Snapshot {
    joints: Vec<(String, [f32; 3], [f32; 3])>,
    parts: Vec<Surface>,
    images: Vec<Picture>,
}
#[derive(Serialize, Deserialize)]
struct Picture {
    size: [u32; 2],
    rgba: String,
}
#[derive(Serialize, Deserialize)]
struct Surface {
    vertices: Vec<([f32; 3], [f32; 3], [f32; 2], [u32; 4], [f32; 4])>,
    indices: Vec<u32>,
    textures: [Option<usize>; 4],
    color: [f32; 4],
    emissive: [f32; 3],
    metallic: f32,
    roughness: f32,
    alpha_cutoff: f32,
    alpha_mode: u32,
    normal_scale: f32,
    double_sided: bool,
    uv: [[[f32; 4]; 2]; 4],
    binding: Vec<(String, [f32; 16])>,
}

fn invalid() -> String {
    tr("import-animation-inventory-cache-error")
}
fn finite(values: impl IntoIterator<Item = f32>) -> bool {
    values.into_iter().all(|v| v.is_finite() && v.abs() < 100_000.0)
}

pub fn save(model: &Model, avatars: &crate::scene::avatar::AvatarLibrary) -> Result<(), String> {
    let rig = &avatars.rig;
    let mut images = Vec::new();
    let mut image_ids = HashMap::new();
    let mut parts = Vec::new();
    for part in &model.geometry.parts {
        let textures = part.textures.each_ref().map(|image| {
            image.as_ref().map(|image| {
                let key = Arc::as_ptr(image) as usize;
                *image_ids.entry(key).or_insert_with(|| {
                    let index = images.len();
                    images.push(Picture {
                        size: image.size,
                        rgba: STANDARD.encode(&image.rgba),
                    });
                    index
                })
            })
        });
        let m = part.material;
        let binding = part
            .skin
            .as_ref()
            .map(|skin| {
                skin.joints
                    .iter()
                    .zip(&skin.inverse_bind)
                    .map(|(joint, inverse)| {
                        Ok((
                            rig.canonical_names.get(*joint).ok_or_else(invalid)?.clone(),
                            inverse.to_cols_array(),
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()
            })
            .transpose()?
            .unwrap_or_default();
        parts.push(Surface {
            vertices: part
                .vertices
                .iter()
                .map(|v| (v.position, v.normal, v.uv, v.joints, v.weights))
                .collect(),
            indices: part.indices.to_vec(),
            textures,
            color: m.color,
            emissive: m.emissive,
            metallic: m.metallic,
            roughness: m.roughness,
            alpha_cutoff: m.alpha_cutoff,
            alpha_mode: m.alpha_mode,
            normal_scale: m.normal_scale,
            double_sided: m.double_sided,
            uv: m.uv,
            binding,
        });
    }
    let snapshot = Snapshot {
        joints: rig
            .canonical_names
            .iter()
            .zip(&model.base.local_pos)
            .zip(&model.base.scale)
            .map(|((name, pos), scale)| (name.clone(), pos.to_array(), scale.to_array()))
            .collect(),
        parts,
        images,
    };
    let mut compressed = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    serde_json::to_writer(&mut compressed, &snapshot).map_err(|e| e.to_string())?;
    let compressed = compressed.finish().map_err(|e| e.to_string())?;
    if compressed.len() as u64 > LIMIT {
        return Err(invalid());
    }
    // Validate our output with the same limits as future sessions before
    // publishing a reference. A partial/oversized model is never remembered.
    let mut bytes = MAGIC.to_vec();
    bytes.extend(compressed);
    decode(model.path.clone(), &bytes, avatars)?;
    let parent = model.path.parent().ok_or_else(invalid)?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = model.path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &model.path)
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

pub fn decode(path: PathBuf, bytes: &[u8], avatars: &crate::scene::avatar::AvatarLibrary) -> Result<Arc<Model>, String> {
    let compressed = bytes.strip_prefix(MAGIC).ok_or_else(invalid)?;
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(compressed)
        .take(LIMIT + 1)
        .read_to_end(&mut json)
        .map_err(|_| invalid())?;
    if json.len() as u64 > LIMIT {
        return Err(invalid());
    }
    let snapshot: Snapshot = serde_json::from_slice(&json).map_err(|_| invalid())?;
    let rig = &avatars.rig;
    if snapshot.parts.is_empty() || snapshot.parts.len() > 2048 || snapshot.joints.len() > 512 || snapshot.images.len() > 1024 {
        return Err(invalid());
    }
    let mut base = crate::scene::anim::SkeletonBase {
        local_pos: rig.local_pos.clone(),
        scale: rig.scale.clone(),
    };
    let mut seen = HashSet::new();
    for (name, pos, scale) in snapshot.joints {
        let joint = *rig.names.get(&name).ok_or_else(invalid)?;
        if !seen.insert(joint) || !finite(pos) || !finite(scale) || scale.iter().any(|v| *v <= 0.0) {
            return Err(invalid());
        }
        base.local_pos[joint] = Vec3::from_array(pos);
        base.scale[joint] = Vec3::from_array(scale);
    }
    let mut image_bytes = 0usize;
    let mut images = Vec::new();
    for image in snapshot.images {
        if image.size.iter().any(|v| *v == 0 || *v > 2048) {
            return Err(invalid());
        }
        let expected = image.size[0] as usize * image.size[1] as usize * 4;
        image_bytes += expected;
        if image_bytes > 64 << 20 || image.rgba.len() > expected.div_ceil(3) * 4 {
            return Err(invalid());
        }
        let rgba = STANDARD.decode(image.rgba).map_err(|_| invalid())?;
        if rgba.len() != expected {
            return Err(invalid());
        }
        images.push(Arc::new(Image {
            size: image.size,
            rgba: rgba.into(),
        }));
    }
    let world = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
    if world.iter().any(|matrix| !matrix.is_finite()) {
        return Err(invalid());
    }
    let mut parts = Vec::new();
    let mut total_vertices = 0usize;
    let mut total_indices = 0usize;
    for surface in snapshot.parts {
        total_vertices += surface.vertices.len();
        total_indices += surface.indices.len();
        if total_vertices > 1_000_000
            || total_indices > 6_000_000
            || surface.indices.len() < 3
            || surface.indices.len() % 3 != 0
            || surface.binding.len() > 256
        {
            return Err(invalid());
        }
        let mut joints = Vec::new();
        let mut inverse_bind = Vec::new();
        for (name, matrix) in surface.binding {
            if !finite(matrix) {
                return Err(invalid());
            }
            joints.push(*rig.names.get(&name).ok_or_else(invalid)?);
            inverse_bind.push(Mat4::from_cols_array(&matrix));
        }
        if joints.is_empty() {
            return Err(invalid());
        }
        let mut vertices = Vec::new();
        for (position, normal, uv, palette, weights) in surface.vertices {
            if !finite(position)
                || !finite(normal)
                || !finite(uv)
                || weights.iter().any(|w| !w.is_finite() || *w < 0.0 || *w > 1.001)
                || palette
                    .iter()
                    .zip(weights)
                    .any(|(index, weight)| weight > 0.0 && *index as usize >= joints.len())
            {
                return Err(invalid());
            }
            vertices.push(Vertex {
                position,
                normal,
                uv,
                joints: palette,
                weights,
            });
        }
        if surface.indices.iter().any(|i| *i as usize >= vertices.len()) || surface.textures.iter().flatten().any(|i| *i >= images.len()) {
            return Err(invalid());
        }
        let material = Material {
            color: surface.color,
            emissive: surface.emissive,
            metallic: surface.metallic,
            roughness: surface.roughness,
            alpha_cutoff: surface.alpha_cutoff,
            alpha_mode: surface.alpha_mode,
            normal_scale: surface.normal_scale,
            double_sided: surface.double_sided,
            uv: surface.uv,
        };
        if material.alpha_mode > 2
            || !finite(
                material
                    .color
                    .into_iter()
                    .chain(material.emissive)
                    .chain([material.metallic, material.roughness, material.alpha_cutoff, material.normal_scale])
                    .chain(material.uv.into_iter().flatten().flatten()),
            )
        {
            return Err(invalid());
        }
        let rest = joints
            .iter()
            .zip(&inverse_bind)
            .map(|(joint, inverse)| world[*joint] * *inverse)
            .collect();
        parts.push(Part {
            diagnostic: false,
            vertices: vertices.into(),
            indices: surface.indices.into(),
            textures: surface.textures.map(|id| id.and_then(|id| images.get(id).cloned())),
            material,
            skin: Some(Arc::new(Skin {
                joints,
                inverse_bind,
                rest,
            })),
        });
    }
    let mut geometry = aurora_render::preview::Model {
        parts,
        center: Vec3::ZERO,
        radius: 1.0,
    };
    super::super::animation_model::frame_rotations(&mut geometry, &base, rig);
    if !geometry.center.is_finite() || !geometry.radius.is_finite() {
        return Err(invalid());
    }
    Ok(Arc::new(Model {
        path,
        base,
        geometry: Arc::new(geometry),
    }))
}
