//! Prepares preview geometry and image maps on background workers. Orbit,
//! lighting, PBR maps and the final display run entirely on the GPU.

use super::*;
use aurora_render::preview::{Image, Material, Model, Part, Vertex};
use glam::Vec3;

pub fn material(value: &PbrMaterial) -> Material {
    Material {
        color: value.base_color_factor,
        emissive: value.emissive_factor,
        metallic: value.metallic_factor,
        roughness: value.roughness_factor,
        alpha_mode: match value.alpha_mode {
            aurora_assets::AlphaMode::Opaque => 0,
            aurora_assets::AlphaMode::Blend => 1,
            aurora_assets::AlphaMode::Mask => 2,
        },
        alpha_cutoff: value.alpha_cutoff,
        normal_scale: 1.0,
        double_sided: value.double_sided,
    }
}
fn image(pixels: &DecodedImage, options: import_texture::Options) -> Result<Arc<Image>, aurora_assets::AssetError> {
    let pixels = import_texture::prepare(pixels, options)?;
    let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, pixels.width as usize * pixels.height as usize);
    Ok(Arc::new(Image {
        size: [pixels.width, pixels.height],
        rgba: rgba.into(),
    }))
}
fn sphere() -> (Vec<Vertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for y in 0..=32 {
        let latitude = y as f32 / 32.0 * std::f32::consts::PI;
        for x in 0..=64 {
            let longitude = x as f32 / 64.0 * std::f32::consts::TAU;
            let normal = [latitude.sin() * longitude.cos(), latitude.sin() * longitude.sin(), latitude.cos()];
            vertices.push(Vertex {
                position: normal,
                normal,
                uv: [x as f32 / 64.0, y as f32 / 32.0],
                ..Default::default()
            });
            if x < 64 && y < 32 {
                let first = y * 65 + x;
                indices.extend_from_slice(&[first, first + 65, first + 1, first + 1, first + 65, first + 66]);
            }
        }
    }
    (vertices, indices)
}
fn bounds(parts: &[Part]) -> (Vec3, f32) {
    let point = |part: &Part, vertex: &Vertex| {
        let point = Vec3::from_array(vertex.position);
        let mut p = Vec3::ZERO;
        let mut sum = 0.0;
        if let Some(skin) = &part.skin {
            for (j, w) in vertex.joints.iter().zip(vertex.weights) {
                if w > 0.0
                    && let Some(m) = skin.rest.get(*j as usize)
                {
                    p += m.transform_point3(point) * w;
                    sum += w;
                }
            }
        }
        if sum > 0.0001 { p / sum } else { point }
    };
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for part in parts {
        for vertex in part.vertices.iter() {
            let p = point(part, vertex);
            min = min.min(p);
            max = max.max(p);
        }
    }
    let center = (min + max) * 0.5;
    if parts.is_empty() || !center.is_finite() {
        (Vec3::ZERO, 1.0)
    } else {
        let radius = parts
            .iter()
            .flat_map(|part| part.vertices.iter().map(|v| point(part, v).distance_squared(center)))
            .fold(0.0, f32::max)
            .sqrt();
        (center, radius.max(0.01))
    }
}

fn classic_morphs(vertices: &mut [Vertex], part: &str, hand: u32, emote: &str) {
    // Port of LLHandMotion (indra/llcharacter/llhandmotion.cpp) and
    // LLPolyMorphTarget (indra/newview/llpolymorph.cpp), originally LGPL 2.1.
    // Deltas are static between edits; apply them once on the worker.
    const HANDS: [&str; 14] = [
        "",
        "Hands_Relaxed",
        "Hands_Point",
        "Hands_Fist",
        "Hands_Relaxed_L",
        "Hands_Point_L",
        "Hands_Fist_L",
        "Hands_Relaxed_R",
        "Hands_Point_R",
        "Hands_Fist_R",
        "Hands_Salute_R",
        "Hands_Typing",
        "Hands_Peace_R",
        "Hands_Spread_R",
    ];
    let hand = HANDS.get(hand as usize).copied().unwrap_or("");
    if hand.is_empty() && emote.is_empty() {
        return;
    }
    let file = match part {
        "head" => "avatar_head.llm",
        "upper" => "avatar_upper_body.llm",
        "lower" => "avatar_lower_body.llm",
        _ => return,
    };
    let Ok(data) = std::fs::read(crate::scene::avatar::AvatarLibrary::data_dir().join(file)) else {
        return;
    };
    let Ok(mesh) = aurora_assets::parse_llm(&data) else { return };
    let mut remaps: HashMap<u32, Vec<usize>> = HashMap::new();
    for (from, to) in &mesh.vertex_remaps {
        if *from >= 0 && *to >= 0 {
            remaps.entry(*from as u32).or_default().push(*to as usize);
        }
    }
    for morph in mesh
        .morphs
        .iter()
        .filter(|m| (!hand.is_empty() && m.name == hand) || (!emote.is_empty() && m.name.eq_ignore_ascii_case(emote)))
    {
        for (slot, index) in morph.vertex_indices.iter().enumerate() {
            let apply = |vertex: &mut Vertex| {
                if let Some(delta) = morph.d_positions.get(slot) {
                    vertex.position = (Vec3::from_array(vertex.position) + Vec3::from_array(*delta)).to_array();
                }
                if let Some(delta) = morph.d_normals.get(slot) {
                    vertex.normal = (Vec3::from_array(vertex.normal) + Vec3::from_array(*delta))
                        .normalize_or_zero()
                        .to_array();
                }
            };
            if let Some(vertex) = vertices.get_mut(*index as usize) {
                apply(vertex);
            }
            for to in remaps.get(index).into_iter().flatten() {
                if let Some(vertex) = vertices.get_mut(*to) {
                    apply(vertex);
                }
            }
        }
    }
}

fn mesh_joint_world(
    rig: &crate::scene::anim::Rig,
    skin: &aurora_assets::import_mesh::Skin,
    imported: bool,
    pelvis: f32,
) -> Vec<glam::Mat4> {
    let mut positions = rig.local_pos.clone();
    if imported {
        for (name, bind) in skin.names.iter().zip(&skin.alternate_bind) {
            if let Some(&joint) = rig.names.get(name) {
                positions[joint] = bind.w_axis.truncate();
            }
        }
    }
    rig.world_with(&rig.local_rot, Vec3::Z * pelvis, &positions, &rig.scale)
}

fn build(
    source: &Source,
    target: usize,
    options: import_texture::Options,
    avatars: &crate::scene::avatar::AvatarLibrary,
) -> Result<Model, aurora_assets::AssetError> {
    let mut parts = Vec::new();
    match source {
        Source::Mesh(draft) => {
            let mut cache = HashMap::new();
            for instance in &draft.scene.instances {
                let Some(high) = draft.scene.models.get(instance.model) else {
                    continue;
                };
                let transform = instance.transform;
                let normal_matrix = transform.inverse().transpose();
                let model = draft
                    .prepared
                    .as_ref()
                    .and_then(|prepared| {
                        if draft.preview_physics {
                            prepared.physics.get(instance.model).and_then(Option::as_ref)
                        } else {
                            prepared
                                .lods
                                .get(instance.model)
                                .and_then(|lods| lods[draft.preview_lod.min(3)].as_ref())
                        }
                    })
                    .unwrap_or(high);
                if draft.preview_physics
                    && let Some(physics) = draft.prepared.as_ref().and_then(|p| p.hulls.get(instance.model))
                {
                    let hulls: Vec<_> = if !physics.hulls.is_empty() {
                        physics.hulls.iter().collect()
                    } else if draft.prepared.as_ref().is_some_and(|p| p.physics[instance.model].is_none()) {
                        physics.base.iter().collect()
                    } else {
                        Vec::new()
                    };
                    for hull in hulls {
                        let center = hull.vertices.iter().map(|p| Vec3::from_array(*p)).fold(Vec3::ZERO, |a, b| a + b)
                            / hull.vertices.len().max(1) as f32;
                        let vertices = hull
                            .vertices
                            .iter()
                            .map(|p| {
                                Vertex::new(
                                    transform.transform_point3(Vec3::from_array(*p) + center * draft.explode).to_array(),
                                    normal_matrix
                                        .transform_vector3((Vec3::from_array(*p) - center).normalize_or_zero())
                                        .normalize_or_zero()
                                        .to_array(),
                                    [0.0; 2],
                                )
                            })
                            .collect::<Vec<_>>();
                        parts.push(Part {
                            diagnostic: false,
                            vertices: vertices.into(),
                            indices: hull.indices.iter().flatten().copied().collect::<Vec<_>>().into(),
                            textures: std::array::from_fn(|_| None),
                            material: Material::default(),
                            skin: None,
                        });
                    }
                    if !physics.hulls.is_empty() || draft.prepared.as_ref().is_some_and(|p| p.physics[instance.model].is_none()) {
                        continue;
                    }
                }
                let skin = if draft.preview_weights && !draft.preview_physics {
                    model.skin.as_ref().and_then(|skin| {
                        let joints = skin
                            .names
                            .iter()
                            .map(|name| avatars.rig.names.get(name).copied())
                            .collect::<Option<Vec<_>>>()?;
                        let inverse = skin.inverse_bind.iter().map(|m| *m * skin.bind_shape).collect::<Vec<_>>();
                        let world = mesh_joint_world(&avatars.rig, skin, draft.options.joints, draft.options.pelvis);
                        let rest = joints.iter().zip(&inverse).map(|(j, m)| world[*j] * *m).collect();
                        Some(Arc::new(aurora_render::preview::Skin {
                            joints,
                            inverse_bind: inverse,
                            rest,
                        }))
                    })
                } else {
                    None
                };
                for (slot, face) in model.faces.iter().enumerate() {
                    let value = draft.scene.materials.get(*instance.materials.get(slot).unwrap_or(&face.material));
                    let mut maps = std::array::from_fn(|_| None);
                    if draft.show_textures
                        && !draft.preview_physics
                        && let Some(value) = value
                    {
                        for (slot, index) in value.images.iter().enumerate() {
                            if let Some(local) = index.and_then(|i| draft.images.get(&i)) {
                                let map = if let Some(map) = cache.get(&local.key) {
                                    Arc::clone(map)
                                } else {
                                    let map = image(&local.pixels, options)?;
                                    cache.insert(local.key, map.clone());
                                    map
                                };
                                maps[slot] = Some(map);
                            }
                        }
                    }
                    let vertices = face
                        .vertices
                        .iter()
                        .map(|v| Vertex {
                            position: if skin.is_some() {
                                v.position
                            } else {
                                transform.transform_point3(Vec3::from_array(v.position)).to_array()
                            },
                            normal: if skin.is_some() {
                                v.normal
                            } else {
                                normal_matrix
                                    .transform_vector3(Vec3::from_array(v.normal))
                                    .normalize_or_zero()
                                    .to_array()
                            },
                            uv: [v.uv[0], 1.0 - v.uv[1]],
                            joints: v.joints.map(u32::from),
                            weights: if skin.is_some() { v.weights } else { [0.0; 4] },
                        })
                        .collect::<Vec<_>>();
                    let mut diagnostic_part = None;
                    if draft.preview_physics {
                        let mut diagnostic = Vec::new();
                        for triangle in face.indices.as_chunks::<3>().0 {
                            if *triangle != [0; 3]
                                && aurora_assets::import_mesh::process::degenerate(
                                    triangle.map(|i| Vec3::from_array(face.vertices[i as usize].position) * 0.5),
                                )
                            {
                                diagnostic.extend(triangle.iter().map(|i| vertices[*i as usize]));
                            }
                        }
                        if !diagnostic.is_empty() {
                            let indices = (0..diagnostic.len() as u32).collect::<Vec<_>>();
                            diagnostic_part = Some(Part {
                                diagnostic: true,
                                vertices: diagnostic.into(),
                                indices: indices.into(),
                                textures: std::array::from_fn(|_| None),
                                material: Material::default(),
                                skin: None,
                            });
                        }
                    }
                    parts.push(Part {
                        diagnostic: false,
                        vertices: vertices.into(),
                        indices: face.indices.clone().into(),
                        textures: maps,
                        material: value.map(|v| material(&v.value)).unwrap_or_default(),
                        skin: skin.clone(),
                    });
                    parts.extend(diagnostic_part);
                }
                if (draft.show_joints || draft.show_overrides)
                    && let Some(skin) = &high.skin
                {
                    let world = mesh_joint_world(&avatars.rig, skin, draft.show_overrides, draft.options.pelvis);
                    for name in &skin.names {
                        let Some(&joint) = avatars.rig.names.get(name) else { continue };
                        let center = world[joint].w_axis.truncate();
                        let (mut vertices, indices) = sphere();
                        for vertex in &mut vertices {
                            vertex.position = (Vec3::from_array(vertex.position) * 0.015 + center).to_array();
                        }
                        parts.push(Part {
                            diagnostic: false,
                            vertices: vertices.into(),
                            indices: indices.into(),
                            textures: std::array::from_fn(|_| None),
                            material: Material::default(),
                            skin: None,
                        });
                    }
                }
            }
        }
        Source::Material { value, images } => {
            let mut maps: [Option<Arc<Image>>; 4] = std::array::from_fn(|_| None);
            let mut cache = HashMap::new();
            for (slot, source) in images.iter().enumerate() {
                if let Some(source) = source {
                    let map = if let Some(map) = cache.get(&source.key) {
                        Arc::clone(map)
                    } else {
                        let map = image(&source.pixels, options)?;
                        cache.insert(source.key, map.clone());
                        map
                    };
                    maps[slot] = Some(map);
                }
            }
            let (vertices, indices) = sphere();
            parts.push(Part {
                diagnostic: false,
                vertices: vertices.into(),
                indices: indices.into(),
                textures: maps,
                material: material(value),
                skin: None,
            });
        }
        Source::Texture(pixels) if target == 9 => {
            let params = aurora_prim::VolumeParams {
                sculpt: Some(aurora_prim::params::SculptParams {
                    texture: Uuid::nil(),
                    sculpt_type: 1,
                }),
                ..Default::default()
            };
            let volume = aurora_prim::sculpt::generate_sculpt(&params, 4.0, pixels.width, pixels.height, pixels.components, &pixels.data);
            for face in volume.faces {
                let vertices = face
                    .positions
                    .iter()
                    .enumerate()
                    .map(|(i, position)| Vertex {
                        position: *position,
                        normal: face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                        uv: face.uvs.get(i).copied().unwrap_or([0.0; 2]),
                        ..Default::default()
                    })
                    .collect::<Vec<_>>();
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: face.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    textures: std::array::from_fn(|_| None),
                    material: Default::default(),
                    skin: None,
                });
            }
        }
        Source::Texture(pixels) => {
            let map = image(pixels, options)?;
            let target_name = match target {
                1 => "hair",
                2 | 5 => "head",
                3 | 6 => "upper",
                4 | 7 => "lower",
                _ => "skirt",
            };
            let male = (5..=7).contains(&target);
            let shape = avatars.shape_params.preview_shape(male, &avatars.rig);
            let world = avatars
                .rig
                .world_with(&avatars.rig.local_rot, Vec3::ZERO, &shape.local_pos, &shape.scale);
            let mut inverse = avatars.rig.default_world_inv.clone();
            inverse.push(glam::Mat4::IDENTITY);
            let mut rest = world.iter().zip(&inverse).map(|(m, inv)| *m * *inv).collect::<Vec<_>>();
            rest.push(glam::Mat4::IDENTITY);
            let skin = Arc::new(aurora_render::preview::Skin {
                joints: (0..inverse.len()).collect(),
                inverse_bind: inverse,
                rest,
            });
            for part in avatars.parts.iter().filter(|part| part.name == target_name) {
                let mut vertices = part
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(index, vertex)| Vertex {
                        position: vertex.pos,
                        normal: [
                            vertex.normal[0] as f32 / 127.0,
                            vertex.normal[1] as f32 / 127.0,
                            vertex.normal[2] as f32 / 127.0,
                        ],
                        uv: [vertex.uv[0], 1.0 - vertex.uv[1]],
                        joints: part.skin.get(index).copied().unwrap_or_default().joints.map(u32::from),
                        weights: part
                            .skin
                            .get(index)
                            .copied()
                            .unwrap_or_default()
                            .weights
                            .map(|w| f32::from(w) / 255.0),
                    })
                    .collect::<Vec<_>>();
                if male {
                    let file = match target_name {
                        "head" => "avatar_head.llm",
                        "upper" => "avatar_upper_body.llm",
                        "lower" => "avatar_lower_body.llm",
                        _ => "",
                    };
                    if let Ok(bytes) = std::fs::read(crate::scene::avatar::AvatarLibrary::data_dir().join(file))
                        && let Ok(mesh) = aurora_assets::parse_llm(&bytes)
                    {
                        let name = match target_name {
                            "head" => "Male_Head",
                            "upper" => "Male_Torso",
                            _ => "Male_Legs",
                        };
                        if let Some(morph) = mesh.morphs.iter().find(|m| m.name == name) {
                            for (index, delta) in morph.vertex_indices.iter().zip(&morph.d_positions) {
                                if let Some(vertex) = vertices.get_mut(*index as usize) {
                                    vertex.position = (Vec3::from_array(vertex.position) + Vec3::from_array(*delta)).to_array();
                                }
                            }
                        }
                    }
                }
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: part.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    textures: [Some(map.clone()), None, None, None],
                    material: Default::default(),
                    skin: Some(skin.clone()),
                });
            }
        }
        Source::Animation { animation, .. } => {
            let rig = &avatars.rig;
            let mut inverse = rig.default_world_inv.clone();
            inverse.push(glam::Mat4::IDENTITY);
            let skin = Arc::new(aurora_render::preview::Skin {
                joints: (0..inverse.len()).collect(),
                rest: vec![glam::Mat4::IDENTITY; inverse.len()],
                inverse_bind: inverse,
            });
            for part in avatars.parts.iter().filter(|p| p.name != "skirt") {
                let mut vertices = part
                    .vertices
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let weights = part.skin.get(i).copied().unwrap_or_default();
                        Vertex {
                            position: v.pos,
                            normal: v.normal[..3]
                                .try_into()
                                .map(|a: [i8; 3]| a.map(|n| n as f32 / 127.0))
                                .unwrap_or([0.0, 0.0, 1.0]),
                            uv: [v.uv[0], 1.0 - v.uv[1]],
                            joints: weights.joints.map(u32::from),
                            weights: weights.weights.map(|w| f32::from(w) / 255.0),
                        }
                    })
                    .collect::<Vec<_>>();
                classic_morphs(&mut vertices, part.name, animation.hand_pose, &animation.emote_name);
                parts.push(Part {
                    diagnostic: false,
                    vertices: vertices.into(),
                    indices: part.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    textures: std::array::from_fn(|_| None),
                    material: Material::default(),
                    skin: Some(skin.clone()),
                });
            }
        }
        _ => {}
    }
    let (center, radius) = bounds(&parts);
    Ok(Model { parts, center, radius })
}

impl Imports {
    pub fn request_model(
        &mut self,
        id: Uuid,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
        avatars: Arc<crate::scene::avatar::AvatarLibrary>,
    ) {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        let Some(mut source) = entry.source.clone() else {
            return;
        };
        let options = self.options(entry);
        let target = entry.texture_target;
        let generation = entry.preview_generation;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        let entry = self.entries.iter_mut().find(|e| e.id == id).expect("entry exists");
        entry.preview_pending = true;
        entry.preview_job_generation = generation;
        entry.preview_error = None;
        jobs.execute(move || {
            let result = (|| {
                let prepared = if let Source::Mesh(draft) = &mut source {
                    if draft.prepared.is_none() {
                        draft.prepared = Some(Arc::new(super::mesh::prepare(draft)?));
                    }
                    draft.prepared.clone()
                } else {
                    None
                };
                build(&source, target, options, &avatars).map(|model| (model, prepared))
            })();
            match result {
                Ok((model, prepared)) => {
                    let _ = tx.send(Worker::Model(id, generation, Arc::new(model), prepared));
                }
                Err(error) => {
                    let _ = tx.send(Worker::PreviewFailed(id, generation, error.to_string()));
                }
            }
            wake.request_repaint();
        });
    }
}
