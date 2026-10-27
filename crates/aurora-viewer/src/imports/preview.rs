//! Prepares preview geometry and image maps on background workers. Orbit,
//! lighting, PBR maps and the final display run entirely on the GPU.

use super::*;
use aurora_render::preview::{Image, Material, Model, Part, Vertex};
use glam::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaterialShape {
    #[default]
    Sphere,
    Cube,
    Aurora,
}
impl MaterialShape {
    pub const ALL: [Self; 3] = [Self::Sphere, Self::Cube, Self::Aurora];
    pub fn label(self) -> &'static str {
        match self {
            Self::Sphere => aurora_i18n::tr!("import-preview-sphere"),
            Self::Cube => aurora_i18n::tr!("import-preview-cube"),
            Self::Aurora => aurora_i18n::tr!("import-preview-aurora"),
        }
    }
}

pub(crate) fn material_geometry(shape: MaterialShape) -> Result<Vec<(Arc<[Vertex]>, Arc<[u32]>)>, aurora_assets::AssetError> {
    match shape {
        MaterialShape::Sphere => {
            let (vertices, indices) = sphere();
            Ok(vec![(vertices.into(), indices.into())])
        }
        MaterialShape::Cube => {
            let volume = aurora_prim::generate_volume(&aurora_prim::VolumeParams::default(), 2.0);
            Ok(volume
                .faces
                .into_iter()
                .map(|face| {
                    let vertices = face
                        .positions
                        .iter()
                        .enumerate()
                        .map(|(i, p)| {
                            // Prim UVs start at the bottom; preview images start at the top.
                            let uv = face.uvs[i];
                            Vertex::new(*p, face.normals[i], [uv[0], 1.0 - uv[1]])
                        })
                        .collect::<Vec<_>>();
                    (
                        vertices.into(),
                        face.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
                    )
                })
                .collect())
        }
        MaterialShape::Aurora => {
            // Cache the bundled geometry once on a worker. Its embedded branding
            // material is replaced by the material being inspected on every face.
            static GEOMETRY: std::sync::OnceLock<Result<Vec<(Arc<[Vertex]>, Arc<[u32]>)>, String>> = std::sync::OnceLock::new();
            GEOMETRY
                .get_or_init(|| {
                    let result = (|| {
                        let doc =
                            aurora_assets::import_gltf::Document::parse(include_bytes!("../../../../assets/models/aurora-3d.glb"), |_| {
                                Err(aurora_assets::AssetError::invalid("bundled model must be self-contained"))
                            })?;
                        let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))?;
                        let scene = aurora_assets::import_mesh::gltf::parse(&doc, &skeleton)?;
                        let mut parts = Vec::new();
                        // The GLB's front faces -Y. Present the emblem near face-on
                        // with the shared reset camera, keeping a visible bevel.
                        let orientation =
                            glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2 + aurora_render::preview::View::default().yaw - 0.25);
                        for instance in &scene.instances {
                            let model = &scene.models[instance.model];
                            let transform = orientation * instance.transform;
                            let normal = transform.inverse().transpose();
                            for face in &model.faces {
                                let vertices = face
                                    .vertices
                                    .iter()
                                    .map(|v| {
                                        Vertex::new(
                                            transform.transform_point3(Vec3::from_array(v.position)).to_array(),
                                            normal.transform_vector3(Vec3::from_array(v.normal)).normalize_or_zero().to_array(),
                                            [v.uv[0], 1.0 - v.uv[1]],
                                        )
                                    })
                                    .collect::<Vec<_>>();
                                parts.push((vertices.into(), face.indices.clone().into()));
                            }
                        }
                        Ok(parts)
                    })();
                    result.map_err(|error: aurora_assets::AssetError| error.to_string())
                })
                .clone()
                .map_err(aurora_assets::AssetError::invalid)
        }
    }
}

fn material_thumbnails(source: &Source) -> [Option<(Uuid, egui::ColorImage)>; 4] {
    let Source::Material { images, .. } = source else {
        return std::array::from_fn(|_| None);
    };
    images.each_ref().map(|image| {
        image.as_ref().and_then(|image| {
            let pixels = &image.pixels;
            let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, pixels.width as usize * pixels.height as usize);
            let rgba = image::RgbaImage::from_raw(pixels.width, pixels.height, rgba)?;
            let pixels = if pixels.width <= 96 && pixels.height <= 96 {
                rgba
            } else {
                image::DynamicImage::ImageRgba8(rgba).thumbnail(96, 96).into_rgba8()
            };
            Some((
                image.key,
                egui::ColorImage::from_rgba_unmultiplied([pixels.width() as usize, pixels.height() as usize], pixels.as_raw()),
            ))
        })
    })
}

pub fn material(value: &PbrMaterial) -> Material {
    Material {
        uv: Material::default().uv,
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
pub(crate) fn sphere() -> (Vec<Vertex>, Vec<u32>) {
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
pub(crate) fn bounds(parts: &[Part]) -> (Vec3, f32) {
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
    let Some(data) = crate::scene::avatar::AvatarLibrary::file(file) else {
        return;
    };
    let Ok(mesh) = aurora_assets::parse_llm(data) else { return };
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
    bend: f32,
) -> Vec<glam::Mat4> {
    let mut positions = rig.local_pos.clone();
    if imported {
        for (name, bind) in skin.names.iter().zip(&skin.alternate_bind) {
            if let Some(&joint) = rig.names.get(name) {
                positions[joint] = bind.w_axis.truncate();
            }
        }
    }
    let mut rotations = rig.local_rot.clone();
    let bend = bend.clamp(0.0, 90.0).to_radians();
    for (name, turn) in [
        ("mElbowLeft", glam::Quat::from_rotation_z(-bend)),
        ("mElbowRight", glam::Quat::from_rotation_z(bend)),
        ("mKneeLeft", glam::Quat::from_rotation_y(bend)),
        ("mKneeRight", glam::Quat::from_rotation_y(bend)),
    ] {
        if let Some(&joint) = rig.names.get(name) {
            rotations[joint] *= turn;
        }
    }
    rig.world_with(&rotations, Vec3::Z * pelvis, &positions, &rig.scale)
}

fn build(
    source: &Source,
    target: usize,
    shape: MaterialShape,
    options: import_texture::Options,
    avatars: &crate::scene::avatar::AvatarLibrary,
) -> Result<Model, aurora_assets::AssetError> {
    let mut parts = Vec::new();
    match source {
        Source::Mesh(draft) => {
            let mut cache = HashMap::new();
            let mut marked_joints = std::collections::HashSet::new();
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
                        let world = mesh_joint_world(&avatars.rig, skin, draft.show_overrides, draft.options.pelvis, draft.preview_bend);
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
                    let mut preview_material = value.map(|v| material(&v.value)).unwrap_or_default();
                    // LLModelPreview::render (indra/newview/llmodelpreview.cpp):
                    // mesh previews draw with depth writes and blending disabled,
                    // even when the source glTF material is BLEND. Keep masks as
                    // they are; this change is limited to the blend mode.
                    if preview_material.alpha_mode == 1 {
                        preview_material.alpha_mode = 0;
                    }
                    parts.push(Part {
                        diagnostic: false,
                        vertices: vertices.into(),
                        indices: face.indices.clone().into(),
                        textures: maps,
                        material: preview_material,
                        skin: skin.clone(),
                    });
                    parts.extend(diagnostic_part);
                }
                // LLModelPreview keeps applying overrides separate from
                // show_joint_positions. Debug dots are never mesh surfaces.
                if draft.show_joints
                    && draft.preview_weights
                    && !draft.preview_physics
                    && let Some(skin) = &high.skin
                {
                    let world = mesh_joint_world(&avatars.rig, skin, draft.show_overrides, draft.options.pelvis, draft.preview_bend);
                    for name in &skin.names {
                        let Some(&joint) = avatars.rig.names.get(name) else { continue };
                        if !marked_joints.insert(joint) {
                            continue;
                        }
                        // LLVOAvatar::renderBones uses 1 mm, 16 mm selected.
                        let radius = if *name == draft.inspector.selected_joint { 0.016 } else { 0.001 };
                        let (mut vertices, indices) = sphere();
                        for vertex in &mut vertices {
                            vertex.position = (Vec3::from_array(vertex.position) * radius).to_array();
                            vertex.weights = [1.0, 0.0, 0.0, 0.0];
                        }
                        parts.push(Part {
                            diagnostic: false,
                            vertices: vertices.into(),
                            indices: indices.into(),
                            textures: std::array::from_fn(|_| None),
                            material: Material::default(),
                            skin: Some(Arc::new(aurora_render::preview::Skin {
                                joints: vec![joint],
                                inverse_bind: vec![glam::Mat4::IDENTITY],
                                rest: vec![world[joint]],
                            })),
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
            for (vertices, indices) in material_geometry(shape)? {
                parts.push(Part {
                    diagnostic: false,
                    vertices,
                    indices,
                    textures: maps.clone(),
                    material: material(value),
                    skin: None,
                });
            }
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
                    if let Some(bytes) = crate::scene::avatar::AvatarLibrary::file(file)
                        && let Ok(mesh) = aurora_assets::parse_llm(bytes)
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
            parts = dummy_avatar_parts(avatars, animation.hand_pose, &animation.emote_name);
        }
        _ => {}
    }
    let (center, radius) = bounds(&parts);
    Ok(Model { parts, center, radius })
}

/// The preview's stand-in avatar: the default body meshes (no skirt),
/// skinned to the default skeleton, with the hand pose and the emote of the
/// animation shown (LLPreviewAnimation's dummy avatar,
/// indra/newview/llfloaterbvhpreview.cpp, originally LGPL 2.1). The pose
/// palette has one more matrix than the rig, the identity.
pub(crate) fn dummy_avatar_parts(avatars: &crate::scene::avatar::AvatarLibrary, hand_pose: u32, emote: &str) -> Vec<Part> {
    let mut parts = Vec::new();
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
        classic_morphs(&mut vertices, part.name, hand_pose, emote);
        parts.push(Part {
            diagnostic: false,
            vertices: vertices.into(),
            indices: part.indices.iter().map(|i| u32::from(*i)).collect::<Vec<_>>().into(),
            textures: std::array::from_fn(|_| None),
            material: Material::default(),
            skin: Some(skin.clone()),
        });
    }
    parts
}

/// [`dummy_avatar_parts`] as a whole preview model.
pub(crate) fn dummy_avatar(avatars: &crate::scene::avatar::AvatarLibrary, hand_pose: u32, emote: &str) -> Model {
    let parts = dummy_avatar_parts(avatars, hand_pose, emote);
    let (center, radius) = bounds(&parts);
    Model { parts, center, radius }
}

impl Imports {
    pub fn request_large_map(&mut self, id: Uuid, slot: usize, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return;
        };
        let Some(Source::Material { images, .. }) = &entry.source else {
            return;
        };
        let Some(image) = images.get(slot).and_then(Option::as_ref) else {
            return;
        };
        if entry.material_large_pending[slot] {
            return;
        }
        entry.material_large_pending[slot] = true;
        let image = image.clone();
        let generation = entry.preview_generation;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let pixels = &image.pixels;
            let rgba = crate::scene::jobs::to_rgba(pixels.components, &pixels.data, pixels.width as usize * pixels.height as usize);
            let pixels = image::RgbaImage::from_raw(pixels.width, pixels.height, rgba).map(|rgba| {
                let scaled = image::DynamicImage::ImageRgba8(rgba)
                    .thumbnail(pixels.width.min(2048), pixels.height.min(2048))
                    .to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([scaled.width() as usize, scaled.height() as usize], scaled.as_raw())
            });
            let _ = tx.send(Worker::MaterialLargePreview(id, generation, slot, image.key, pixels));
            wake.request_repaint();
        });
    }
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
        if let Some(deadline) = entry.preview_not_before
            && let Some(delay) = deadline.checked_duration_since(std::time::Instant::now())
        {
            ctx.request_repaint_after(delay);
            return;
        }
        let Some(mut source) = entry.source.clone() else {
            return;
        };
        let options = self.options(entry);
        let target = entry.texture_target;
        let shape = entry.material_shape;
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
                build(&source, target, shape, options, &avatars).map(|model| (model, prepared))
            })();
            match result {
                Ok((model, prepared)) => {
                    if matches!(source, Source::Material { .. }) {
                        let _ = tx.send(Worker::MaterialPreviews(id, generation, material_thumbnails(&source)));
                    }
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

#[cfg(test)]
mod tests {
    #[test]
    fn mesh_preview_ignores_blend_without_changing_the_source_or_material_preview() {
        let avatars = crate::scene::avatar::AvatarLibrary::load();
        let mut draft = crate::imports::tests::mesh_fixture();
        for source in &mut Arc::make_mut(&mut draft.scene).materials {
            source.value.alpha_mode = aurora_assets::AlphaMode::Blend;
            source.value.base_color_factor = [0.2, 0.4, 0.6, 0.5];
        }
        for show_textures in [false, true] {
            draft.show_textures = show_textures;
            let model = build(
                &Source::Mesh(Box::new(draft.clone())),
                0,
                MaterialShape::default(),
                Default::default(),
                &avatars,
            )
            .expect("mesh preview");
            assert!(!model.parts.is_empty());
            for part in &model.parts {
                assert_eq!(part.material.alpha_mode, 0, "mesh surfaces write depth");
                assert_eq!(part.material.color, [0.2, 0.4, 0.6, 0.5]);
            }
        }
        for source in &draft.scene.materials {
            assert_eq!(source.value.alpha_mode, aurora_assets::AlphaMode::Blend);
            assert_eq!(material(&source.value).alpha_mode, 1, "material previews retain blending");
        }
        for source in &mut Arc::make_mut(&mut draft.scene).materials {
            source.value.alpha_mode = aurora_assets::AlphaMode::Mask;
        }
        let model = build(
            &Source::Mesh(Box::new(draft)),
            0,
            MaterialShape::default(),
            Default::default(),
            &avatars,
        )
        .expect("masked mesh preview");
        assert!(model.parts.iter().all(|part| part.material.alpha_mode == 2));
    }

    #[test]
    fn mesh_import_parity_imported_positions_do_not_create_marker_spheres() {
        use aurora_assets::import_mesh::Skin;
        let avatars = crate::scene::avatar::AvatarLibrary::load();
        let mut draft = crate::imports::tests::mesh_fixture();
        let model = &mut Arc::make_mut(&mut draft.scene).models[0];
        model.skin = Some(Skin {
            names: vec!["mPelvis".into()],
            inverse_bind: vec![glam::Mat4::IDENTITY],
            alternate_bind: vec![glam::Mat4::from_translation(Vec3::new(0.0, 0.0, 1.4))],
            bind_shape: glam::Mat4::IDENTITY,
            pelvis_offset: 0.0,
        });
        for vertex in model.faces.iter_mut().flat_map(|face| &mut face.vertices) {
            vertex.weights = [1.0, 0.0, 0.0, 0.0];
        }
        let faces = model.faces.len();
        draft.preview_weights = true;
        draft.show_overrides = true;
        draft.inspector.selected_joint = "mPelvis".into();
        let render = |draft: &super::super::mesh::Draft| {
            build(
                &Source::Mesh(Box::new(draft.clone())),
                0,
                MaterialShape::default(),
                Default::default(),
                &avatars,
            )
            .expect("rig preview")
        };
        let plain = render(&draft);
        assert_eq!(plain.parts.len(), faces, "overrides alone never add geometry");
        assert!((plain.parts[0].skin.as_ref().expect("skin").rest[0].w_axis.z - 1.4).abs() < 0.0001);
        draft.show_joints = true;
        let marked = render(&draft);
        assert_eq!(marked.parts.len(), faces + 1);
        let marker = marked.parts.last().expect("marker");
        assert_eq!(marker.skin.as_ref().expect("animated marker").joints, [avatars.rig.pelvis]);
        assert!(marker.vertices.iter().all(|v| v.weights == [1.0, 0.0, 0.0, 0.0]));
        assert!((marker.vertices[0].position[2] - 0.016).abs() < 0.00001);
        draft.preview_physics = true;
        assert_eq!(render(&draft).parts.len(), faces, "collision view never adds bone markers");
    }

    #[test]
    fn mesh_lod_bend_diagnostic_moves_wrists_and_ankles_without_editing_the_upload_geometry() {
        let avatars = crate::scene::avatar::AvatarLibrary::load();
        let mut draft = crate::imports::tests::mesh_fixture();
        let skin = aurora_assets::import_mesh::Skin {
            names: vec!["mPelvis".into()],
            inverse_bind: vec![glam::Mat4::IDENTITY],
            alternate_bind: Vec::new(),
            bind_shape: glam::Mat4::IDENTITY,
            pelvis_offset: 0.0,
        };
        let neutral = super::mesh_joint_world(&avatars.rig, &skin, false, 0.0, 0.0);
        let bent = super::mesh_joint_world(&avatars.rig, &skin, false, 0.0, 60.0);
        for name in ["mWristLeft", "mWristRight", "mAnkleLeft", "mAnkleRight"] {
            let joint = avatars.rig.names[name];
            assert!((neutral[joint].w_axis - bent[joint].w_axis).length() > 0.05, "{name}");
        }
        let original = crate::imports::mesh::prepare(&draft).expect("original upload data");
        draft.preview_bend = 60.0;
        let diagnostic = crate::imports::mesh::prepare(&draft).expect("same upload data");
        for (a, b) in original.lods.iter().zip(&diagnostic.lods) {
            for (a, b) in a.iter().zip(b) {
                let a = a.as_ref().expect("LOD");
                let b = b.as_ref().expect("LOD");
                let positions = |model: &aurora_assets::import_mesh::Model| {
                    model
                        .faces
                        .iter()
                        .flat_map(|face| face.vertices.iter().map(|vertex| vertex.position))
                        .collect::<Vec<_>>()
                };
                assert_eq!(positions(a), positions(b));
            }
        }
    }
    use super::*;

    #[test]
    fn each_material_shape_uses_the_inspected_material_and_its_maps_on_all_faces() {
        let avatars = crate::scene::avatar::AvatarLibrary::load();
        let pixels = DecodedImage {
            width: 1,
            height: 1,
            components: 4,
            data: vec![30, 60, 90, 120],
            discard: 0,
        };
        let map = files::local_image(pixels, "test map".into());
        let value = PbrMaterial {
            metallic_factor: 0.7,
            roughness_factor: 0.23,
            ..Default::default()
        };
        let source = Source::Material {
            value: value.clone(),
            images: Box::new(std::array::from_fn(|_| Some(map.clone()))),
        };
        let mut triangles = Vec::new();
        for shape in MaterialShape::ALL {
            let model = build(&source, 0, shape, Default::default(), &avatars).expect("material preview");
            assert!(model.center.is_finite() && model.radius.is_finite() && model.radius > 0.0);
            for part in &model.parts {
                assert!(part.material == material(&value));
                assert!(part.textures.iter().all(Option::is_some));
                assert!(Arc::ptr_eq(
                    part.textures[0].as_ref().expect("map"),
                    part.textures[1].as_ref().expect("map")
                ));
                assert!(part.indices.iter().all(|i| (*i as usize) < part.vertices.len()));
                if shape == MaterialShape::Cube {
                    for vertex in part.vertices.iter().filter(|v| v.normal[2].abs() < 0.5) {
                        assert!(
                            (vertex.uv[1] - (0.5 - vertex.position[2])).abs() < 0.00001,
                            "cube side faces show the top image row at the top"
                        );
                    }
                    for triangle in part.indices.as_chunks::<3>().0 {
                        let vertices = triangle.map(|i| part.vertices[i as usize]);
                        let [a, b, c] = vertices.map(|v| Vec3::from_array(v.position));
                        let normal = vertices.iter().map(|v| Vec3::from_array(v.normal)).sum::<Vec3>();
                        assert!((b - a).cross(c - a).dot(normal) >= -0.00001, "cube triangles face outwards");
                    }
                }
            }
            triangles.push(model.parts.iter().map(|part| part.indices.len() / 3).sum::<usize>());
        }
        assert!(triangles[1] >= 12 && triangles[1] < triangles[0]);
        assert_eq!(triangles[2], 2342);
        let thumbs = material_thumbnails(&source);
        assert!(
            thumbs
                .iter()
                .all(|thumb| thumb.as_ref().is_some_and(|(key, image)| *key == map.key && image.size == [1, 1]))
        );
    }

    #[test]
    fn bundled_aurora_model_resource_names_are_consistent_and_self_contained() {
        let doc = aurora_assets::import_gltf::Document::parse(include_bytes!("../../../../assets/models/aurora-3d.glb"), |_| {
            panic!("bundled model must not load external resources")
        })
        .expect("bundled GLB");
        for key in [
            "scenes",
            "nodes",
            "meshes",
            "materials",
            "images",
            "textures",
            "buffers",
            "bufferViews",
            "accessors",
            "samplers",
        ] {
            for resource in doc.json[key].as_array().expect("resource array") {
                assert!(resource["name"].as_str().expect("name").starts_with("Aurora 3D"));
                assert!(resource.get("uri").is_none());
            }
        }
    }

    #[test]
    fn preview_uses_the_selected_prepared_lod_geometry() {
        let mut draft = crate::imports::tests::mesh_fixture();
        draft.prepared = Some(Arc::new(super::super::mesh::prepare(&draft).expect("LODs")));
        let avatars = crate::scene::avatar::AvatarLibrary::load();
        let mut counts = Vec::new();
        for level in (0..4).rev() {
            draft.preview_lod = level;
            let expected = draft.prepared.as_ref().expect("prepared").lods[0][level]
                .as_ref()
                .expect("LOD")
                .triangles();
            let model = build(
                &Source::Mesh(Box::new(draft.clone())),
                0,
                MaterialShape::default(),
                Default::default(),
                &avatars,
            )
            .expect("preview");
            let count = model.parts.iter().map(|part| part.indices.len() / 3).sum::<usize>();
            assert_eq!(count, expected);
            counts.push(count);
        }
        assert!(counts[1..].iter().all(|count| *count < counts[0]));
    }
}
