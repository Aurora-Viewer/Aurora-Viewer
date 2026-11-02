//! Mesh drafts, worker preparation and frozen fee requests. Reference:
//! LLModelPreview and LLMeshUploadThread (Firestorm, originally LGPL 2.1).

pub(super) mod inspector;
#[cfg(test)]
mod parity_tests;
pub(crate) mod preview;

use super::*;
use aurora_assets::{
    AssetError,
    import_mesh::{self as codec, Model, Scene, encode, process},
};
use aurora_llsd::{Llsd, llsd_map};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LodSource {
    File,
    Auto,
    Combine,
    Fast,
    Reliable,
    Above,
}
#[derive(Clone)]
pub struct Lod {
    pub source: LodSource,
    pub file: Option<Arc<Scene>>,
    pub path: Option<PathBuf>,
    pub triangles: usize,
    pub error: f32,
    pub error_mode: bool,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub scale: f32,
    pub textures: bool,
    pub skin: bool,
    pub joints: bool,
    pub lock_scale: bool,
    pub pelvis: f32,
    pub normals: bool,
    pub crease: f32,
    /// -1 hull, 0..3 LOD, 4 bounds, 5 file, 6 cube, 7 hexagon, 8 user preset.
    pub physics_source: i32,
    pub decompose: bool,
    pub resolution: u32,
    pub max_hulls: u32,
    pub max_vertices: u32,
    pub tolerance: f32,
    pub fill_mode: u32,
    pub retain: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            scale: 1.0,
            textures: false,
            skin: false,
            joints: false,
            lock_scale: false,
            pelvis: 0.0,
            normals: false,
            crease: 75.0,
            physics_source: -1,
            decompose: false,
            resolution: 400000,
            max_hulls: 8,
            max_vertices: 32,
            tolerance: 1.0,
            fill_mode: 0,
            retain: 1.0,
        }
    }
}
#[derive(Clone)]
pub struct Draft {
    pub inspector: inspector::Inspector,
    pub scene: Arc<Scene>,
    pub images: HashMap<usize, LocalImage>,
    pub lods: [Lod; 4],
    pub physics_file: Option<Arc<Scene>>,
    pub physics_path: Option<PathBuf>,
    pub options: Options,
    pub lod_suffixes: [String; 5],
    pub upload_timeout: u32,
    pub prepared: Option<Arc<Prepared>>,
    pub quote: Option<aurora_net::inventory::mesh_upload::Quote>,
    pub quoted_at: Option<std::time::Instant>,
    pub discarded_quotes: Vec<Uuid>,
    pub pending: Option<(Uuid, u64)>,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<std::sync::atomic::AtomicU32>,
    pub preview_lod: usize,
    pub preview_physics: bool,
    pub show_textures: bool,
    pub explode: f32,
    pub preview_weights: bool,
    pub preview_bend: f32,
    pub show_joints: bool,
    pub show_overrides: bool,
}
#[derive(Clone)]
pub struct Prepared {
    pub lods: Vec<[Option<Model>; 4]>,
    pub physics: Vec<Option<Model>>,
    pub hulls: Vec<process::Physics>,
    pub degenerate_triangles: usize,
}
impl Draft {
    /// LLModelPreview::rebuildUploadData: per-prim scale and whole-scene span
    /// both constrain the upload factor (64 m prims, 240 m linkset span on SL).
    pub fn dimensions_and_limit(&self) -> (glam::Vec3, f32) {
        let mut min = glam::Vec3::splat(f32::MAX);
        let mut max = glam::Vec3::splat(f32::MIN);
        let mut prim_scale = 0.0f32;
        for instance in &self.scene.instances {
            let matrix = instance.transform;
            prim_scale = prim_scale
                .max(matrix.x_axis.truncate().length())
                .max(matrix.y_axis.truncate().length())
                .max(matrix.z_axis.truncate().length());
            let bounds = self
                .scene
                .normalized_bounds
                .get(&instance.model)
                .copied()
                .unwrap_or([glam::Vec3::splat(-0.5), glam::Vec3::splat(0.5)]);
            for x in [bounds[0].x, bounds[1].x] {
                for y in [bounds[0].y, bounds[1].y] {
                    for z in [bounds[0].z, bounds[1].z] {
                        let point = matrix.transform_point3(glam::Vec3::new(x, y, z));
                        min = min.min(point);
                        max = max.max(point);
                    }
                }
            }
        }
        let span = max - min;
        if !span.is_finite() || prim_scale <= 0.0 {
            return (glam::Vec3::ZERO, 64.0);
        }
        (
            span * self.options.scale,
            (64.0 / prim_scale).min(240.0 / span.max_element().max(f32::EPSILON)),
        )
    }
    pub fn new(scene: Scene, images: HashMap<usize, LocalImage>) -> Self {
        Self::from_shared(Arc::new(scene), images)
    }
    pub fn from_shared(scene: Arc<Scene>, images: HashMap<usize, LocalImage>) -> Self {
        let triangles = scene.models.iter().map(Model::triangles).sum::<usize>();
        let skin = scene.models.iter().any(|m| m.skin.is_some());
        let mut draft = Self {
            inspector: inspector::Inspector::new(&scene),
            scene,
            images,
            lods: std::array::from_fn(|i| Lod {
                source: if i == 3 { LodSource::File } else { LodSource::Auto },
                file: None,
                path: None,
                triangles: (triangles / 4usize.pow((3 - i) as u32)).max(1),
                error: 0.01,
                error_mode: false,
            }),
            physics_file: None,
            physics_path: None,
            options: Options {
                skin,
                ..Default::default()
            },
            lod_suffixes: Preferences::default().mesh_suffixes,
            upload_timeout: 600,
            prepared: None,
            quote: None,
            quoted_at: None,
            discarded_quotes: Vec::new(),
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            preview_lod: 3,
            preview_bend: 0.0,
            preview_physics: false,
            show_textures: true,
            explode: 0.0,
            preview_weights: skin,
            show_joints: false,
            show_overrides: false,
        };
        draft.clamp_scale();
        draft
    }
    /// LLModelPreview::rebuildUploadData clamps a newly loaded oversized
    /// scene to the per-prim and linkset limits before calculating its fee.
    pub fn clamp_scale(&mut self) {
        if self.options.scale.is_finite() {
            self.options.scale = self.options.scale.min(self.dimensions_and_limit().1).max(0.01);
        }
    }
    pub fn invalidate(&mut self) {
        self.invalidate_quote();
        self.prepared = None;
    }
    pub fn finish_edit(&mut self, previous: &Options, lod_changed: bool) -> bool {
        if !lod_changed && *previous == self.options {
            return false;
        }
        let mut geometry = previous.clone();
        // These affect the instance resources or skin header, not the LODs
        // or convex geometry already computed on the workers.
        geometry.scale = self.options.scale;
        geometry.textures = self.options.textures;
        geometry.joints = self.options.joints;
        geometry.lock_scale = self.options.lock_scale;
        geometry.pelvis = self.options.pelvis;
        if lod_changed || geometry != self.options {
            self.invalidate();
        } else {
            self.invalidate_quote();
        }
        true
    }
    pub fn invalidate_quote(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(std::sync::atomic::AtomicU32::new(0));
        self.pending = None;
        if let Some(quote) = self.quote.take() {
            self.discarded_quotes.push(quote.token);
        }
        self.quoted_at = None;
    }
}

fn aligned(model: &Model, high: &Model) -> Model {
    let mut model = model.clone();
    for face in &mut model.faces {
        for vertex in &mut face.vertices {
            vertex.position = ((glam::Vec3::from_array(vertex.position) * model.size + model.center - high.center) / high.size).to_array();
            vertex.normal = (glam::Vec3::from_array(vertex.normal) / model.size * high.size)
                .normalize_or_zero()
                .to_array();
        }
    }
    model.center = high.center;
    model.size = high.size;
    model
}
fn file_model(draft: &Draft, scene: &Scene, index: usize, faces: bool, above: Option<&Model>) -> Result<Model, AssetError> {
    let high = &draft.scene.models[index];
    let name = |value: &str| {
        let (base, copy) = value
            .rsplit_once("_copy_")
            .filter(|(_, suffix)| suffix.parse::<usize>().is_ok())
            .map_or((value, None), |(base, suffix)| (base, Some(suffix)));
        let base = draft
            .lod_suffixes
            .iter()
            .filter(|suffix| !suffix.is_empty())
            .find_map(|suffix| base.strip_suffix(&format!("_{suffix}")))
            .unwrap_or(base);
        copy.map_or_else(|| base.to_owned(), |suffix| format!("{base}_copy_{suffix}"))
    };
    let candidates: Vec<_> = scene
        .models
        .iter()
        .enumerate()
        .filter(|(_, model)| model.submodel == high.submodel && name(&model.name) == name(&high.name))
        .map(|(index, _)| index)
        .collect();
    let matched = if candidates.len() == 1 {
        Some(candidates[0])
    } else if scene.models.len() == draft.scene.models.len() {
        Some(index)
    } else if !faces {
        scene.models.iter().position(|model| name(&model.name) == "default_physics_shape")
    } else {
        None
    };
    let Some(matched) = matched else {
        if faces && high.submodel > 0 {
            return Ok(above.unwrap_or(high).clone());
        }
        return Err(AssetError::invalid("LOD model names/counts disagree"));
    };
    let mut model = aligned(&scene.models[matched], high);
    if !faces {
        return Ok(model);
    }
    let bindings = |scene: &Scene, index: usize| {
        scene.material_bindings.get(&index).cloned().unwrap_or_else(|| {
            scene.models[index]
                .faces
                .iter()
                .map(|face| {
                    scene
                        .materials
                        .get(face.material)
                        .map_or_else(|| format!("slot:{}", face.material), |material| material.name.clone())
                })
                .collect()
        })
    };
    let expected = bindings(&draft.scene, index);
    let found = bindings(scene, matched);
    let mut available: Vec<_> = found.into_iter().zip(model.faces).map(|(name, face)| (name, Some(face))).collect();
    model.faces = expected
        .iter()
        .enumerate()
        .map(|(slot, name)| {
            available
                .iter_mut()
                .find(|(candidate, face)| candidate == name && face.is_some())
                .and_then(|(_, face)| face.take())
                .unwrap_or_else(|| codec::Face {
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    material: high.faces[slot].material,
                })
        })
        .collect();
    if available.iter().any(|(_, face)| face.is_some()) {
        return Err(AssetError::invalid("LOD materials are not a subset of High"));
    }
    Ok(model)
}
fn box_model(high: &Model) -> Model {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    Model {
        name: high.name.clone(),
        submodel: high.submodel,
        skin: None,
        center: high.center,
        size: high.size,
        faces: vec![codec::Face {
            material: 0,
            vertices: positions
                .into_iter()
                .map(|position| codec::Vertex {
                    position,
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0; 2],
                    joints: [0; 4],
                    weights: [0.0; 4],
                })
                .collect(),
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
            ],
        }],
    }
}

fn hex_model(high: &Model) -> Model {
    // Firestorm fs_resources/hex_phys.dae: six sides, capped, normalized.
    let mut model = box_model(high);
    let face = &mut model.faces[0];
    face.vertices = (0..12)
        .map(|i| {
            let angle = (i % 6) as f32 * std::f32::consts::TAU / 6.0;
            codec::Vertex {
                position: [angle.sin() * 0.5, angle.cos() * 0.5, if i < 6 { -0.5 } else { 0.5 }],
                normal: [angle.sin(), angle.cos(), 0.0],
                uv: [0.0; 2],
                joints: [0; 4],
                weights: [0.0; 4],
            }
        })
        .collect();
    face.indices.clear();
    for i in 0..6 {
        let j = (i + 1) % 6;
        face.indices.extend_from_slice(&[i, j, i + 6, j, j + 6, i + 6]);
    }
    for i in 1..5 {
        face.indices.extend_from_slice(&[0, i + 1, i, 6, i + 6, i + 7]);
    }
    model
}

pub fn prepare(draft: &Draft) -> Result<Prepared, AssetError> {
    let options = &draft.options;
    if !options.scale.is_finite() || options.scale < 0.01 || options.scale > draft.dimensions_and_limit().1 || draft.scene.models.is_empty()
    {
        return Err(AssetError::invalid("mesh scale or linkset size"));
    }
    let total = draft.scene.models.iter().map(Model::triangles).sum::<usize>().max(1);
    let mut prepared = Prepared {
        lods: Vec::new(),
        physics: Vec::new(),
        hulls: Vec::new(),
        degenerate_triangles: 0,
    };
    let results: Result<Vec<_>, AssetError> = draft
        .scene
        .models
        .par_iter()
        .enumerate()
        .map(|(index, high)| {
            if draft.cancel.load(Ordering::Relaxed) {
                return Err(AssetError::invalid("cancelled"));
            }
            let mut lods: [Option<Model>; 4] = std::array::from_fn(|_| None);
            for level in (0..4).rev() {
                let spec = &draft.lods[level];
                let mut model = match spec.source {
                    LodSource::File => {
                        if let Some(scene) = &spec.file {
                            file_model(draft, scene, index, true, lods.get(level + 1).and_then(Option::as_ref))?
                        } else if level == 3 {
                            high.clone()
                        } else {
                            return Err(AssetError::invalid("LOD file missing"));
                        }
                    }
                    LodSource::Above => lods
                        .get(level + 1)
                        .and_then(Option::as_ref)
                        .cloned()
                        .ok_or_else(|| AssetError::invalid("high LOD cannot use the LOD above"))?,
                    method => process::lod(
                        high,
                        process::LodOptions {
                            triangles: if spec.error_mode {
                                0
                            } else {
                                high.triangles() * spec.triangles / total
                            },
                            // Firestorm uses 100% error in triangle-budget mode;
                            // the saved error threshold applies only to error mode.
                            error: if spec.error_mode { spec.error } else { 1.0 },
                            method: match method {
                                LodSource::Auto => process::Method::Auto,
                                LodSource::Reliable => process::Method::Reliable,
                                LodSource::Fast => process::Method::Fast,
                                _ => process::Method::Precise,
                            },
                            crease_angle: None,
                            error_only: spec.error_mode,
                        },
                        &draft.cancel,
                    )?,
                };
                if options.normals {
                    for face in &mut model.faces {
                        process::normals(face, options.crease)?;
                    }
                }
                if options.skin && level < 3 {
                    codec::skin::transfer_weights(&mut model, high)?;
                }
                lods[level] = Some(model);
            }
            if high.submodel > 0 {
                // LLMeshUploadThread::generateHulls / packModelIntance
                // (indra/newview/llmeshrepository.cpp) also supplies a base
                // hull for split models whose instance collision is None.
                let hulls = process::Physics {
                    base: Some(process::base_hull(high)?),
                    hulls: Vec::new(),
                };
                return Ok((lods, None, hulls));
            }
            let mut physics = match options.physics_source {
                -1 => None,
                0..=3 => lods[options.physics_source as usize].clone(),
                4 | 6 => Some(box_model(high)),
                7 => Some(hex_model(high)),
                5 | 8 => Some(file_model(
                    draft,
                    draft
                        .physics_file
                        .as_deref()
                        .ok_or_else(|| AssetError::invalid("physics file missing"))?,
                    index,
                    false,
                    None,
                )?),
                _ => return Err(AssetError::invalid("physics source")),
            };
            if let Some(model) = &mut physics
                && options.retain < 1.0
            {
                *model = process::lod(
                    model,
                    process::LodOptions {
                        triangles: (model.triangles() as f32 * options.retain) as usize,
                        error: 1.0,
                        method: process::Method::Fast,
                        crease_angle: None,
                        error_only: false,
                    },
                    &draft.cancel,
                )?;
            }
            let hulls = if options.decompose {
                process::analyze_progress(
                    physics
                        .as_ref()
                        .ok_or_else(|| AssetError::invalid("choose a physics source before analyzing"))?,
                    process::PhysicsOptions {
                        resolution: options.resolution,
                        max_hulls: options.max_hulls,
                        max_vertices: options.max_vertices,
                        tolerance: options.tolerance,
                        fill_mode: options.fill_mode,
                    },
                    &draft.cancel,
                    &draft.progress,
                )?
            } else {
                process::Physics {
                    base: Some(process::base_hull(high)?),
                    hulls: Vec::new(),
                }
            };
            Ok((lods, physics, hulls))
        })
        .collect();
    for (lods, physics, hulls) in results? {
        if hulls.hulls.is_empty()
            && let Some(model) = &physics
        {
            prepared.degenerate_triangles += model
                .faces
                .iter()
                .map(|face| {
                    face.indices
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .filter(|t| {
                            **t != [0; 3]
                                && process::degenerate(t.map(|i| glam::Vec3::from_array(face.vertices[i as usize].position) * 0.5))
                        })
                        .count()
                })
                .sum::<usize>();
        }
        prepared.lods.push(lods);
        prepared.physics.push(physics);
        prepared.hulls.push(hulls);
    }
    Ok(prepared)
}

fn floats(values: impl IntoIterator<Item = f32>) -> Llsd {
    Llsd::Array(values.into_iter().map(|f| Llsd::Real(f64::from(f))).collect())
}
pub fn request(
    draft: &Draft,
    prepared: &Prepared,
    folder: Uuid,
    textures_folder: Uuid,
    name: &str,
    description: &str,
    rights: Permissions,
    texture_options: import_texture::Options,
) -> Result<aurora_net::inventory::mesh_upload::Request, AssetError> {
    if name.trim().is_empty() || name.len() > 63 || description.len() > 127 || name.contains('\0') || description.contains('\0') {
        return Err(AssetError::invalid("mesh name or description"));
    }
    if prepared.degenerate_triangles > 0 {
        return Err(AssetError::invalid(aurora_i18n::tr!(
            "import-degenerate-physics",
            count = prepared.degenerate_triangles
        )));
    }
    let meshes = prepared
        .lods
        .iter()
        .enumerate()
        .map(|(i, lods)| {
            encode::asset(
                lods,
                prepared.physics[i].as_ref(),
                &prepared.hulls[i],
                encode::Options {
                    skin: draft.options.skin,
                    joints: draft.options.joints,
                    lock_scale: draft.options.lock_scale,
                    pelvis_offset: draft.options.pelvis,
                    secondary: (draft.scene.models[i].submodel > 0).then_some(draft.scene.models[i].submodel),
                },
            )
            .map(Llsd::Binary)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut texture_list = Vec::new();
    let mut texture_info = Vec::new();
    let mut texture_index = HashMap::new();
    let used_materials = draft.scene.used_materials();
    if draft.options.textures {
        if draft
            .scene
            .materials
            .iter()
            .enumerate()
            .filter(|(index, _)| used_materials.contains(index))
            .any(|(_, m)| m.images[0].is_some_and(|i| !draft.images.contains_key(&i)))
        {
            return Err(AssetError::invalid("mesh diffuse texture missing"));
        }
        for (index, material) in draft.scene.materials.iter().enumerate() {
            if !used_materials.contains(&index) {
                continue;
            }
            if let Some(index) = material.images[0]
                && let Some(image) = draft.images.get(&index)
                && !texture_index.contains_key(&image.key)
            {
                let pixels = import_texture::prepare(&image.pixels, texture_options)?;
                texture_index.insert(image.key, texture_list.len() as i32);
                texture_info.push(llsd_map! { "width" => pixels.width as i32, "height" => pixels.height as i32 });
                texture_list.push(Llsd::Binary(import_texture::encode(&pixels, texture_options)?));
            }
        }
    }
    let mut instances = Vec::new();
    for instance in &draft.scene.instances {
        let model = draft
            .scene
            .models
            .get(instance.model)
            .ok_or_else(|| AssetError::invalid("mesh instance index"))?;
        let (scale, rotation, position) = instance.transform.to_scale_rotation_translation();
        if !scale.is_finite() || !rotation.is_finite() || !position.is_finite() || scale.min_element() <= 0.0 {
            return Err(AssetError::invalid("mesh instance transform"));
        }
        let faces = model.faces.iter().enumerate().map(|(slot, face)| {
            let material = draft.scene.materials.get(*instance.materials.get(slot).unwrap_or(&face.material));
            let mut face = llsd_map! { "diffuse_color" => floats(material.map(|m| m.value.base_color_factor).unwrap_or([1.0;4])), "fullbright" => false };
            if let Some(material) = material && let Some(image) = material.images[0].and_then(|i| draft.images.get(&i))
                && let Some(index) = texture_index.get(&image.key) {
                face.insert("image", Llsd::Integer(*index));
                for (key, value) in [("scales",1.0),("scalet",1.0),("offsets",0.0),("offsett",0.0),("imagerot",0.0)] { face.insert(key, Llsd::Real(value)); }
            }
            face
        }).collect();
        instances.push(
            llsd_map! { "position" => floats((position * draft.options.scale).to_array()), "rotation" => floats(rotation.to_array()),
            "scale" => floats((scale * draft.options.scale).to_array()), "material" => 3,
            "physics_shape_type" => if model.submodel > 0 { 1 } else if prepared.physics[instance.model].is_some() { 0 } else { 2 },
            "mesh" => instance.model as i32, "mesh_name" => instance.name.as_str(), "face_list" => Llsd::Array(faces) },
        );
    }
    let mut formats = llsd_map! { "high" => 0, "physics" => if draft.options.physics_source == -1 { "none" } else { "generated" } };
    for (i, key) in ["lowest", "low", "medium", "high"].into_iter().enumerate() {
        formats.insert(
            key,
            match draft.lods[i].source {
                LodSource::File => Llsd::String(
                    draft.lods[i]
                        .path
                        .as_ref()
                        .and_then(|p| p.extension())
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into()),
                ),
                LodSource::Above => "lod above".into(),
                _ => "generated".into(),
            },
        );
    }
    let resources = llsd_map! { "mesh_list" => Llsd::Array(meshes), "texture_list" => Llsd::Array(texture_list),
    "instance_list" => Llsd::Array(instances), "source_format" => formats, "metric" => "MUT_Unspecified" };
    // A fee query carries compressed mesh geometry and dimensions, but no
    // texture bytes. The final transfer carries the same frozen geometry.
    let mut query_resources = resources.clone();
    query_resources.insert(
        "texture_list",
        Llsd::Array(texture_info.iter().map(|_| Llsd::Binary(Vec::new())).collect()),
    );
    query_resources.insert("texture_info", Llsd::Array(texture_info));
    let query = llsd_map! { "folder_id" => folder, "texture_folder_id" => textures_folder, "asset_type" => "mesh", "inventory_type" => "object",
    "name" => name, "description" => description, "next_owner_mask" => rights.next_owner as i32, "group_mask" => rights.group as i32,
    "everyone_mask" => rights.everyone as i32, "asset_resources" => query_resources };
    Ok(aurora_net::inventory::mesh_upload::Request {
        folder,
        texture_folder: textures_folder,
        name: name.into(),
        description: description.into(),
        query: Arc::new(aurora_llsd::to_xml(&query)),
        resources: Arc::new(aurora_llsd::to_xml(&resources)),
        timeout_secs: draft.upload_timeout,
    })
}

impl Imports {
    pub fn quote_mesh(
        &mut self,
        id: Uuid,
        ctx: &egui::Context,
        jobs: &crate::scene::jobs::Jobs,
        world: &crate::world::World,
        prefs: &crate::ui::inventory::InventoryPreferences,
        rights: Permissions,
    ) {
        if !self.with_choice(Choice::Mesh, |imports| imports.folder_plan_valid(world, prefs)) {
            self.message = aurora_i18n::tr!("import-invalid-request").into();
            return;
        }
        let planned_folder = if self.folders.active() {
            self.entries
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.upload_path.as_ref())
                .and_then(|path| self.folders.resolved(path, &world.inventory, &prefs.protected))
                .or(Some(self.folders.destination))
        } else {
            None
        };
        let shared_destination = self.folders.active() || !self.load_preferences.use_default_folders;
        let textures = Self::default_folder(world, prefs, Kind::Texture);
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && !e.reloading) else {
            return;
        };
        let Some(Source::Mesh(draft)) = &mut entry.source else {
            return;
        };
        if self.mesh_permission_account != world.agent_id {
            self.mesh_permission_account = world.agent_id;
            self.mesh_permission_denied = false;
        }
        draft.invalidate_quote();
        draft.inspector.clear();
        draft.upload_timeout = self.load_preferences.mesh_upload_timeout.max(1);
        draft.inspector.push(aurora_i18n::tr!("import-log-calculating"), false);
        let request_id = Uuid::new_v4();
        let generation = entry.preview_generation;
        draft.pending = Some((request_id, generation));
        let draft = draft.clone();
        let folder = planned_folder.unwrap_or(entry.folder);
        let textures = if shared_destination { folder } else { textures };
        let name = entry.name.clone();
        let description = entry.description.clone();
        let options = entry.options;
        let wake = ctx.clone();
        let tx = self.tx.clone();
        jobs.execute(move || {
            let result = draft
                .prepared
                .as_ref()
                .map_or_else(|| prepare(&draft).map(Arc::new), |prepared| Ok(prepared.clone()))
                .and_then(|prepared| {
                    request(&draft, &prepared, folder, textures, &name, &description, rights, options).map(|request| (prepared, request))
                })
                .map_err(|e| e.to_string());
            let _ = tx.send(Worker::MeshPrepared(id, request_id, generation, result));
            wake.request_repaint();
        });
    }
    pub fn mesh_quoted(&mut self, request: Uuid, result: Result<aurora_net::inventory::mesh_upload::Quote, Error>) {
        for entry in &mut self.entries {
            if let Some(Source::Mesh(draft)) = &mut entry.source
                && draft
                    .pending
                    .is_some_and(|(id, revision)| id == request && revision == entry.preview_generation)
            {
                draft.pending = None;
                match result {
                    Ok(quote) => {
                        self.mesh_permission_denied = false;
                        draft.quoted_at = Some(std::time::Instant::now());
                        draft.quote = Some(quote);
                    }
                    Err(error) => {
                        if matches!(error, Error::MeshPermission) {
                            self.mesh_permission_denied = true;
                        }
                        self.message = ui::error(error);
                        draft.inspector.push(&self.message, true);
                    }
                }
                return;
            }
        }
        if let Ok(quote) = result {
            self.discarded_quotes.push(quote.token);
        }
    }
}
