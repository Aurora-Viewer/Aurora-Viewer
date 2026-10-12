//! Mesh drafts, worker preparation and frozen fee requests. Reference:
//! LLModelPreview and LLMeshUploadThread (Firestorm, originally LGPL 2.1).

use super::*;
use aurora_assets::{
    AssetError,
    import_mesh::{self as codec, Model, Scene, encode, process},
};
use aurora_llsd::{Llsd, llsd_map};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
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
#[derive(Clone, PartialEq)]
pub struct Options {
    pub scale: f32,
    pub textures: bool,
    pub skin: bool,
    pub joints: bool,
    pub lock_scale: bool,
    pub pelvis: f32,
    pub normals: bool,
    pub crease: f32,
    /// -1: convex hull only, 0..3: LOD, 4: bounding box, 5: file.
    pub physics_source: i32,
    pub decompose: bool,
    pub resolution: u32,
    pub max_hulls: u32,
    pub max_vertices: u32,
    pub tolerance: f32,
    pub surface: bool,
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
            surface: false,
            retain: 1.0,
        }
    }
}
#[derive(Clone)]
pub struct Draft {
    pub scene: Arc<Scene>,
    pub images: HashMap<usize, LocalImage>,
    pub lods: [Lod; 4],
    pub physics_file: Option<Arc<Scene>>,
    pub physics_path: Option<PathBuf>,
    pub options: Options,
    pub prepared: Option<Arc<Prepared>>,
    pub quote: Option<aurora_net::inventory::mesh_upload::Quote>,
    pub pending: Option<(Uuid, u64)>,
    pub cancel: Arc<AtomicBool>,
    pub preview_lod: usize,
    pub preview_physics: bool,
    pub show_textures: bool,
    pub explode: f32,
    pub preview_weights: bool,
    pub show_joints: bool,
    pub show_overrides: bool,
}
#[derive(Clone)]
pub struct Prepared {
    pub lods: Vec<[Option<Model>; 4]>,
    pub physics: Vec<Option<Model>>,
    pub hulls: Vec<process::Physics>,
}
impl Draft {
    pub fn new(scene: Scene, images: HashMap<usize, LocalImage>) -> Self {
        let triangles = scene.models.iter().map(Model::triangles).sum::<usize>();
        let skin = scene.models.iter().any(|m| m.skin.is_some());
        Self {
            scene: Arc::new(scene),
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
            prepared: None,
            quote: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            preview_lod: 3,
            preview_physics: false,
            show_textures: true,
            explode: 0.0,
            preview_weights: skin,
            show_joints: false,
            show_overrides: false,
        }
    }
    pub fn invalidate(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.pending = None;
        self.quote = None;
        self.prepared = None;
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
fn match_model<'a>(scene: &'a Scene, high: &Model, index: usize, count: usize, faces: bool) -> Result<&'a Model, AssetError> {
    let unique_name = scene.models.iter().filter(|m| m.name == high.name).count() == 1;
    let model = scene
        .models
        .iter()
        .find(|m| unique_name && m.name == high.name)
        .or_else(|| (scene.models.len() == count).then(|| scene.models.get(index)).flatten())
        .ok_or_else(|| AssetError::invalid("LOD model names/counts disagree"))?;
    if faces && model.faces.len() != high.faces.len() {
        return Err(AssetError::invalid("LOD material face counts disagree"));
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

pub fn prepare(draft: &Draft) -> Result<Prepared, AssetError> {
    let options = &draft.options;
    if !options.scale.is_finite()
        || !(0.01..=64.0).contains(&options.scale)
        || draft.scene.models.is_empty()
        || draft.scene.models.len() > 256
        || draft.scene.instances.len() > 256
    {
        return Err(AssetError::invalid("mesh scale or linkset size"));
    }
    let total = draft.scene.models.iter().map(Model::triangles).sum::<usize>().max(1);
    let mut prepared = Prepared {
        lods: Vec::new(),
        physics: Vec::new(),
        hulls: Vec::new(),
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
                            aligned(match_model(scene, high, index, draft.scene.models.len(), true)?, high)
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
                            error: spec.error,
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
                lods[level] = Some(model);
            }
            let mut physics = match options.physics_source {
                -1 => None,
                0..=3 => lods[options.physics_source as usize].clone(),
                4 => Some(box_model(high)),
                5 => Some(aligned(
                    match_model(
                        draft
                            .physics_file
                            .as_deref()
                            .ok_or_else(|| AssetError::invalid("physics file missing"))?,
                        high,
                        index,
                        draft.scene.models.len(),
                        false,
                    )?,
                    high,
                )),
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
                process::analyze(
                    physics
                        .as_ref()
                        .ok_or_else(|| AssetError::invalid("choose a physics source before analyzing"))?,
                    process::PhysicsOptions {
                        resolution: options.resolution,
                        max_hulls: options.max_hulls,
                        max_vertices: options.max_vertices,
                        tolerance: options.tolerance,
                        fill_surface: options.surface,
                    },
                    &draft.cancel,
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
                    secondary: None,
                },
            )
            .map(Llsd::Binary)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut texture_list = Vec::new();
    let mut texture_info = Vec::new();
    let mut texture_index = HashMap::new();
    if draft.options.textures {
        if draft
            .scene
            .materials
            .iter()
            .any(|m| m.images[0].is_some_and(|i| !draft.images.contains_key(&i)))
        {
            return Err(AssetError::invalid("mesh diffuse texture missing"));
        }
        for material in &draft.scene.materials {
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
            "physics_shape_type" => if prepared.physics[instance.model].is_some() { 0 } else { 2 },
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
        query: Arc::new(aurora_llsd::to_xml(&query)),
        resources: Arc::new(aurora_llsd::to_xml(&resources)),
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
        let textures = Self::default_folder(world, prefs, Kind::Texture);
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) else {
            return;
        };
        let Some(Source::Mesh(draft)) = &mut entry.source else {
            return;
        };
        draft.invalidate();
        let request_id = Uuid::new_v4();
        let generation = entry.preview_generation;
        draft.pending = Some((request_id, generation));
        let draft = draft.clone();
        let folder = entry.folder;
        let name = entry.name.clone();
        let description = entry.description.clone();
        let options = entry.options;
        let wake = ctx.clone();
        let tx = self.tx.clone();
        jobs.execute(move || {
            let result = prepare(&draft)
                .and_then(|prepared| {
                    request(&draft, &prepared, folder, textures, &name, &description, rights, options)
                        .map(|request| (Arc::new(prepared), request))
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
                    Ok(quote) => draft.quote = Some(quote),
                    Err(error) => self.message = ui::error(error),
                }
                return;
            }
        }
    }
}
