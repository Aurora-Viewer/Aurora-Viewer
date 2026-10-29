//! Capture only the rezzed linkset's rigged mesh surfaces. Compressed mesh
//! and texture decoding, joint overrides and bounds run on a scene worker.
use super::*;
use crate::{scene::Scene, shape_editor::material::Face, world::World};
use aurora_render::preview::{Part, Skin, Vertex};
use glam::{Mat4, Vec3};

pub struct Input {
    meshes: Vec<(Uuid, crate::scene::meshes::MeshView, Vec<Face>)>,
    textures: HashMap<Uuid, crate::scene::textures::PreviewSource>,
}

/// None until the full high LOD and skin of every mesh in the linkset arrive.
/// Ordinary prim roots may be invisible supports; the picker explicitly
/// promises rigged mesh surfaces, as does the local animation-model importer.
pub fn capture(world: &World, scene: &mut Scene, root: Uuid, wanted: &mut HashSet<Uuid>) -> Result<Option<Input>, String> {
    let root_index = world
        .objects
        .index_of_uuid(&root)
        .ok_or_else(|| tr("import-animation-inventory-gone"))?;
    let mut stack = vec![root_index];
    let mut visited = HashSet::new();
    let mut meshes = Vec::new();
    let mut texture_ids = HashSet::new();
    let mut ready = true;
    while let Some(index) = stack.pop() {
        if !visited.insert(index) || visited.len() > 256 {
            return Err(tr("import-animation-inventory-large"));
        }
        let Some(object) = world.objects.get(index) else { continue };
        stack.extend(world.objects.children_of(&object.key));
        if object.is_avatar() || object.parent_id != 0 && world.objects.parent_of(object).is_none() {
            return Err(tr("import-animation-inventory-gone"));
        }
        let volume = object.visual_volume();
        let Some(id) = volume.sculpt.filter(|_| volume.is_mesh()).map(|s| s.texture) else {
            continue;
        };
        scene.meshes.want(id, 3);
        scene.meshes.need(id, 1024.0, false);
        let Some(data) = scene.meshes.view(&id) else {
            ready = false;
            continue;
        };
        let high = data.header.actual_lod(3).and_then(|lod| data.header.lods[lod]);
        if high.is_none_or(|range| data.bytes.section(range).is_none()) || data.header.skin.is_some() && data.skin.is_none() {
            ready = false;
            continue;
        }
        if !data.skin.as_ref().is_some_and(|skin| !skin.joint_names.is_empty()) {
            continue;
        }
        let faces: Vec<_> = (0..8).map(|face| Face::capture(world, scene, object, face, root_index)).collect();
        for face in &faces {
            texture_ids.extend(face.textures.iter().copied().filter(|id| !id.is_nil()));
            // Scene streaming normally requests these nearby surfaces. An
            // unresolved PBR material must not silently become white forever.
        }
        for face in 0..8 {
            if let Some(id) = object.visual_material(face) {
                scene.materials.resolve(&id);
                ready &= scene.materials.peek(&id).is_some();
            } else {
                let id = object.visual_texture_face(face).material_id;
                if !id.is_nil() {
                    let cap = world.regions.get(&object.key.region).and_then(|region| {
                        region.caps.get("RenderMaterials").map(|url| crate::scene::legacy_mat::RegionCap {
                            url,
                            limits: region.materials_limits,
                        })
                    });
                    ready &= scene.legacy_mats.get(&id, cap).is_some();
                }
            }
        }
        meshes.push((id, data, faces));
    }
    if !ready {
        return Ok(None);
    }
    if meshes.is_empty() {
        return Err(tr("import-animation-model-rigged"));
    }
    if texture_ids.union(wanted).count() > 1024 {
        return Err(tr("import-animation-inventory-large"));
    }
    for id in &texture_ids {
        scene.textures.want_model_preview(*id, true);
        wanted.insert(*id);
    }
    if texture_ids.iter().any(|id| !scene.textures.model_preview_ready(id)) {
        return Ok(None);
    }
    let textures = texture_ids.into_iter().map(|id| (id, scene.textures.preview_source(&id))).collect();
    Ok(Some(Input { meshes, textures }))
}

impl Input {
    pub fn prepare(self, path: PathBuf, avatars: &crate::scene::avatar::AvatarLibrary) -> Result<Arc<Model>, String> {
        if self.textures.len() > 1024 {
            return Err(tr("import-animation-inventory-large"));
        }
        let mut textures = HashMap::new();
        let mut texture_bytes = 0usize;
        for (id, source) in self.textures {
            let image = source.image(id).ok_or_else(|| tr("import-animation-inventory-cache-error"))?;
            texture_bytes += image.rgba.len();
            if texture_bytes > 64 << 20 {
                return Err(tr("import-animation-inventory-large"));
            }
            textures.insert(id, image);
        }
        let rig = &avatars.rig;
        let mut base = crate::scene::anim::SkeletonBase {
            local_pos: rig.local_pos.clone(),
            scale: rig.scale.clone(),
        };
        let mut overrides = crate::scene::joint_overrides::JointOverrides::new(rig);
        for (id, data, _) in &self.meshes {
            if let Some(skin) = &data.skin {
                overrides.add(rig, *id, skin);
            }
        }
        overrides.apply(rig, &mut base.local_pos, &mut base.scale);
        let world = rig.world_with(&rig.local_rot, Vec3::ZERO, &base.local_pos, &base.scale);
        let mut parts = Vec::new();
        let mut vertices = 0usize;
        for (_, data, appearance) in self.meshes {
            let info = data.skin.as_ref().ok_or_else(|| tr("import-animation-model-rigged"))?;
            let (joints, inverse_bind): (Vec<_>, Vec<_>) = info
                .joint_names
                .iter()
                .zip(&info.inverse_bind)
                .map(|(name, matrix)| {
                    let (joint, inverse) = rig.skin_bind(name, *matrix);
                    (joint, inverse * info.bind_shape)
                })
                .unzip();
            let rest = joints
                .iter()
                .zip(&inverse_bind)
                .map(|(joint, inverse)| world.get(*joint).copied().unwrap_or(Mat4::IDENTITY) * *inverse)
                .collect();
            let skin = Arc::new(Skin {
                joints,
                inverse_bind,
                rest,
            });
            let block = data.lod_section(3).ok_or_else(|| tr("import-animation-inventory-gone"))?;
            let faces = aurora_assets::decode_mesh_lod(block).map_err(|e| e.to_string())?;
            for (face, look) in faces.iter().zip(appearance) {
                if look.textures[0] == crate::scene::avatar::IMG_INVISIBLE || look.material.color[3] < 0.004 {
                    continue;
                }
                vertices += face.positions.len();
                if vertices > 1_000_000 {
                    return Err(tr("import-animation-inventory-large"));
                }
                let vertices: Vec<_> = face
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(i, position)| Vertex {
                        position: *position,
                        normal: face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                        uv: face.uvs.get(i).copied().unwrap_or_default(),
                        joints: face.joints.get(i).copied().unwrap_or_default().map(u32::from),
                        weights: face.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
                    })
                    .collect();
                let images = look.textures.map(|id| textures.get(&id).cloned());
                let blend = look.material.color[3] < 1.0
                    || images[0]
                        .as_ref()
                        .is_some_and(|image| image.rgba.iter().skip(3).step_by(4).any(|a| *a != 0 && *a != 255));
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: face.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    textures: images,
                    material: aurora_render::preview::Material {
                        alpha_mode: if look.alpha_explicit {
                            look.material.alpha_mode
                        } else if blend {
                            1
                        } else {
                            2
                        },
                        ..look.material
                    },
                    skin: Some(skin.clone()),
                });
            }
        }
        if parts.is_empty() {
            return Err(tr("import-animation-model-rigged"));
        }
        let mut geometry = aurora_render::preview::Model {
            parts,
            center: Vec3::ZERO,
            radius: 1.0,
        };
        super::super::animation_model::frame_rotations(&mut geometry, &base, rig);
        Ok(Arc::new(Model {
            path,
            base,
            geometry: Arc::new(geometry),
        }))
    }
}
