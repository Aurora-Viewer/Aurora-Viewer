//! Native file selection and bounded loading, off the frame thread.

use super::*;
use aurora_assets::{
    AssetError, import_animation,
    import_gltf::{Document, Image},
    import_sound,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub(super) fn read(path: &Path) -> Result<Vec<u8>, AssetError> {
    if path
        .extension()
        .is_some_and(|e| ["dae", "gltf", "glb"].iter().any(|ext| e.eq_ignore_ascii_case(ext)))
    {
        return read_mesh(path);
    }
    let file = File::open(path).map_err(|_| AssetError::invalid("file cannot be opened"))?;
    let mut bytes = Vec::new();
    file.take(256 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AssetError::invalid("file read failed"))?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(AssetError::TooLarge("import file"));
    }
    Ok(bytes)
}

fn read_mesh(path: &Path) -> Result<Vec<u8>, AssetError> {
    let mut file = File::open(path).map_err(|_| AssetError::invalid("file cannot be opened"))?;
    let size = file
        .metadata()
        .ok()
        .and_then(|m| usize::try_from(m.len()).ok())
        .ok_or(AssetError::TooLarge("mesh file"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| AssetError::TooLarge("mesh file memory"))?;
    file.read_to_end(&mut bytes).map_err(|_| AssetError::invalid("file read failed"))?;
    Ok(bytes)
}

fn decode_uri(uri: &str) -> Result<String, AssetError> {
    // glTF URI percent escapes are bytes, then UTF-8; '+' remains a literal.
    let mut decoded = Vec::new();
    let mut bytes = uri.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let a = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| AssetError::invalid("resource URI escape"))?;
            let b = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| AssetError::invalid("resource URI escape"))?;
            (a * 16 + b) as u8
        } else {
            byte
        });
    }
    String::from_utf8(decoded).map_err(|_| AssetError::invalid("resource URI encoding"))
}
fn relative_path(root: &Path, uri: &str) -> Result<PathBuf, AssetError> {
    let decoded = decode_uri(uri)?;
    if decoded.contains(':') || decoded.starts_with(['/', '\\']) {
        return Err(AssetError::invalid("resource must be a relative file"));
    }
    let target = root
        .join(decoded)
        .canonicalize()
        .map_err(|_| AssetError::invalid("glTF resource not found"))?;
    Ok(target)
}
fn relative(root: &Path, uri: &str) -> Result<Vec<u8>, AssetError> {
    read(&relative_path(root, uri)?)
}
fn collada_image_path(root: &Path, uri: &str) -> Result<PathBuf, AssetError> {
    // LLDAELoader accepts both relative texture paths and exporter file URIs.
    // Only local files are resolved; remote/UNC resources never trigger I/O.
    let raw = uri.strip_prefix("file:///").or_else(|| uri.strip_prefix("file:/")).unwrap_or(uri);
    let decoded = decode_uri(raw)?;
    if decoded.starts_with(['/', '\\']) || decoded.contains("://") {
        return Err(AssetError::invalid("texture resource must be a local file"));
    }
    let path = PathBuf::from(decoded);
    if path.is_absolute() {
        path.canonicalize().map_err(|_| AssetError::invalid("texture resource not found"))
    } else {
        relative_path(root, raw)
    }
}
fn dependencies(doc: &Document, root: &Path, path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_owned()];
    for key in ["buffers", "images"] {
        for value in doc.json[key].as_array().into_iter().flatten() {
            if let Some(uri) = value["uri"].as_str()
                && !uri.starts_with("data:")
                && Uuid::parse_str(uri).is_err()
                && let Ok(path) = relative_path(root, uri)
            {
                paths.push(path);
            }
        }
    }
    paths
}

pub(super) fn local_image(pixels: DecodedImage, name: String) -> LocalImage {
    let mut hash = Sha256::new();
    hash.update(pixels.width.to_le_bytes());
    hash.update(pixels.height.to_le_bytes());
    hash.update([pixels.components]);
    hash.update(&pixels.data);
    let digest = hash.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    LocalImage {
        path: None,
        key: Uuid::from_bytes(id),
        empty_alpha: import_texture::empty_alpha(&pixels),
        pixels: Arc::new(pixels),
        asset: None,
        name,
    }
}
pub(super) fn mesh_scene(
    path: &Path,
    data: &[u8],
    assume_meters: bool,
    document: Option<Document>,
    model_limit: u32,
) -> Result<(aurora_assets::import_mesh::Scene, HashMap<usize, LocalImage>), AssetError> {
    let root = path
        .parent()
        .ok_or_else(|| AssetError::invalid("mesh directory"))?
        .canonicalize()
        .map_err(|_| AssetError::invalid("mesh directory"))?;
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))?;
    let mut images = HashMap::new();
    let scene = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dae")) {
        let mut scene = aurora_assets::import_mesh::collada::parse_with_options(data, &skeleton, assume_meters, model_limit)?;
        let used_images: HashSet<_> = scene
            .used_materials()
            .into_iter()
            .filter_map(|i| scene.materials.get(i))
            .flat_map(|m| m.images.into_iter().flatten())
            .collect();
        for (index, uri) in &scene.image_uris {
            if !used_images.contains(index) {
                continue;
            }
            match collada_image_path(&root, uri)
                .and_then(|path| read(&path))
                .and_then(|bytes| import_texture::decode_map(&bytes))
            {
                Ok(pixels) => {
                    images.insert(*index, local_image(pixels, uri.clone()));
                }
                Err(error) => scene.warnings.push(error.to_string()),
            }
        }
        scene
    } else {
        let doc = match document {
            Some(doc) => doc,
            None => Document::parse_mesh(data, |uri| read_mesh(&relative_path(&root, uri)?))?,
        };
        let mut scene = aurora_assets::import_mesh::gltf::parse_with_options(&doc, &skeleton, model_limit)?;
        for key in ["buffers", "images"] {
            for resource in doc.json[key].as_array().into_iter().flatten() {
                if let Some(uri) = resource["uri"].as_str()
                    && !uri.starts_with("data:")
                    && Uuid::parse_str(uri).is_err()
                {
                    scene.source_uris.push(uri.into());
                }
            }
        }
        for index in scene.used_materials() {
            let material = &scene.materials[index];
            for index in material.images.iter().flatten() {
                if !images.contains_key(index) {
                    let result = doc.image(*index, |uri| relative(&root, uri)).and_then(|image| match image {
                        Image::Bytes(bytes) => import_texture::decode_map(&bytes),
                        Image::Existing(_) => Err(AssetError::invalid("existing mesh texture")),
                    });
                    match result {
                        Ok(pixels) => {
                            images.insert(*index, local_image(pixels, format!("{}_{}", material.name, index + 1)));
                        }
                        Err(error) => scene.warnings.push(error.to_string()),
                    }
                }
            }
        }
        scene
    };
    Ok((scene, images))
}

pub(super) fn load(
    path: &Path,
    choice: Choice,
    options: import_texture::Options,
    sound_limit: u32,
    prefs: &Preferences,
    local: bool,
) -> Result<Vec<Entry>, AssetError> {
    let data = read(path)?;
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let matches_choice = match choice {
        Choice::Texture => matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2"),
        Choice::Sound => extension == "wav",
        Choice::Animation => matches!(extension.as_str(), "bvh" | "anim"),
        Choice::Material => matches!(extension.as_str(), "gltf" | "glb"),
        Choice::Mesh => matches!(extension.as_str(), "dae" | "gltf" | "glb"),
        Choice::Bulk => matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2" | "wav" | "anim" | "dae" | "gltf" | "glb"
        ),
        Choice::Local => matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2" | "dae" | "gltf" | "glb"
        ),
    };
    if !matches_choice {
        return Err(AssetError::Unsupported("file type for this import window".into()));
    }
    // Local and batch glTF files are classified by content. The dedicated
    // material tab can still extract materials from a document with meshes.
    let mut document = if matches!(choice, Choice::Local | Choice::Bulk) && matches!(extension.as_str(), "gltf" | "glb") {
        let root = path
            .parent()
            .ok_or_else(|| AssetError::invalid("asset directory"))?
            .canonicalize()
            .map_err(|_| AssetError::invalid("asset directory"))?;
        Some(Document::parse_mesh(&data, |uri| read_mesh(&relative_path(&root, uri)?))?)
    } else {
        None
    };
    let detected_mesh = matches!(choice, Choice::Local | Choice::Bulk)
        && (extension == "dae"
            || document.as_ref().is_some_and(|doc| {
                doc.json
                    .get("meshes")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|meshes| !meshes.is_empty())
            }));
    let source = match extension.as_str() {
        "dae" | "gltf" | "glb" if choice == Choice::Mesh || detected_mesh => {
            let (scene, images) = mesh_scene(
                path,
                &data,
                local && prefs.local_assume_meters,
                document.take(),
                prefs.mesh_model_limit,
            )?;
            let mut draft = mesh::Draft::new(scene, images);
            draft.lod_suffixes = prefs.mesh_suffixes.clone();
            draft.upload_timeout = prefs.mesh_upload_timeout;
            draft.lods[3].path = Some(path.to_owned());
            draft.options.skin &= prefs.mesh_auto_weights;
            draft.preview_weights &= prefs.mesh_auto_preview_weights && draft.options.skin;
            if prefs.mesh_reliable_default {
                for lod in &mut draft.lods[..3] {
                    lod.source = mesh::LodSource::Reliable;
                }
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let high_suffix = format!("_{}", prefs.mesh_suffixes[3]);
            let stem = if prefs.mesh_suffixes[3].is_empty() {
                stem.as_ref()
            } else {
                stem.strip_suffix(&high_suffix).unwrap_or(stem.as_ref())
            };
            for level in [2, 1, 0, 4] {
                let suffix = &prefs.mesh_suffixes[level];
                if suffix.is_empty() || suffix.contains(['/', '\\', ':']) {
                    continue;
                }
                let candidate = path.with_file_name(format!("{stem}_{suffix}.{extension}"));
                if candidate.is_file() {
                    match read(&candidate)
                        .and_then(|data| mesh_scene(&candidate, &data, local && prefs.local_assume_meters, None, prefs.mesh_model_limit))
                    {
                        Ok((scene, _)) => {
                            if level < 4 {
                                draft.lods[level].file = Some(Arc::new(scene));
                                draft.lods[level].path = Some(candidate);
                                draft.lods[level].source = mesh::LodSource::File;
                            } else {
                                draft.physics_file = Some(Arc::new(scene));
                                draft.physics_path = Some(candidate);
                                draft.options.physics_source = 5;
                            }
                        }
                        Err(error) => Arc::make_mut(&mut draft.scene).warnings.push(error.to_string()),
                    }
                }
            }
            Source::Mesh(Box::new(draft))
        }
        "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2" => Source::Texture(Arc::new(import_texture::decode(&data)?)),
        "wav" => Source::Sound(Arc::new(import_sound::decode_wav(&data, sound::SOURCE_LIMIT)?)),
        "anim" => Source::Animation {
            animation: Arc::new(aurora_assets::parse_animation_upload(&data)?),
            bytes: Arc::new(data),
            bvh: None,
        },
        "bvh" if choice != Choice::Bulk => {
            let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))?;
            let bvh = import_animation::parse_bvh(&data, &skeleton)?;
            Source::Animation {
                bytes: Arc::new(Vec::new()),
                bvh: Some(BvhInfo {
                    frames: bvh.frames,
                    frame_time: bvh.frame_time,
                }),
                animation: Arc::new(bvh.animation),
            }
        }
        "bvh" => return Err(AssetError::Unsupported("BVH requires its animation import window".into())),
        "gltf" | "glb" if choice != Choice::Mesh => {
            let root = path
                .parent()
                .ok_or_else(|| AssetError::invalid("material directory"))?
                .canonicalize()
                .map_err(|_| AssetError::invalid("material directory"))?;
            let doc = match document {
                Some(doc) => doc,
                None => Document::parse(&data, |uri| relative(&root, uri))?,
            };
            let mut cache = HashMap::<usize, LocalImage>::new();
            let mut entries = Vec::new();
            for (index, material) in doc.materials()?.into_iter().enumerate() {
                let mut value = material.value;
                // LLGLTFMaterial::sanitizeAssetMaterial: texture transforms
                // belong to face overrides, not imported inventory materials.
                value.transforms = [aurora_assets::TextureTransform::default(); 4];
                let mut images: [Option<LocalImage>; 4] = std::array::from_fn(|_| None);
                for (slot, index) in material.images.into_iter().enumerate() {
                    if let Some(index) = index {
                        let image = if let Some(image) = cache.get(&index) {
                            image.clone()
                        } else {
                            match doc.image(index, |uri| relative(&root, uri))? {
                                Image::Existing(_) => return Err(AssetError::invalid("unexpected existing image")),
                                Image::Bytes(bytes) => {
                                    let pixels = Arc::new(import_texture::decode_map(&bytes)?);
                                    let mut hash = Sha256::new();
                                    hash.update(pixels.width.to_le_bytes());
                                    hash.update(pixels.height.to_le_bytes());
                                    hash.update([pixels.components]);
                                    hash.update(&pixels.data);
                                    let digest = hash.finalize();
                                    let mut id = [0u8; 16];
                                    id.copy_from_slice(&digest[..16]);
                                    let image = LocalImage {
                                        path: None,
                                        key: Uuid::from_bytes(id),
                                        empty_alpha: import_texture::empty_alpha(&pixels),
                                        name: format!("{}_{}", path.file_stem().unwrap_or_default().to_string_lossy(), index + 1),
                                        pixels,
                                        asset: None,
                                    };
                                    cache.insert(index, image.clone());
                                    image
                                }
                            }
                        };
                        let mut image = image;
                        if choice == Choice::Local {
                            let mut hash = Sha256::new();
                            hash.update(path.as_os_str().to_string_lossy().as_bytes());
                            hash.update(index.to_le_bytes());
                            let digest = hash.finalize();
                            let mut id = [0u8; 16];
                            id.copy_from_slice(&digest[..16]);
                            image.key = Uuid::from_bytes(id);
                        }
                        images[slot] = Some(image);
                    }
                }
                for slot in [1, 3] {
                    if let Some(image) = &mut images[slot]
                        && image.pixels.components == 4
                    {
                        *image = local_image(aurora_assets::import_material::rgb(&image.pixels)?, image.name.clone());
                    }
                }
                let occlusion = if let Some(occlusion) = material.occlusion.filter(|i| Some(*i) != material.images[2]) {
                    match doc.image(occlusion, |uri| relative(&root, uri))? {
                        Image::Bytes(bytes) => Some(import_texture::decode_map(&bytes)?),
                        Image::Existing(_) => {
                            return Err(AssetError::Unsupported(
                                "separate occlusion asset must be a local image for packing".into(),
                            ));
                        }
                    }
                } else {
                    None
                };
                if let Some(packed) = aurora_assets::import_material::orm(
                    images[2].as_ref().map(|i| i.pixels.as_ref()),
                    occlusion.as_ref(),
                    material.occlusion.is_some(),
                )? {
                    let name = images[2]
                        .as_ref()
                        .map(|i| i.name.clone())
                        .unwrap_or_else(|| format!("{}_ORM", path.file_stem().unwrap_or_default().to_string_lossy()));
                    images[2] = Some(local_image(packed, name));
                }
                if choice == Choice::Local {
                    for (slot, image) in images.iter_mut().enumerate() {
                        if let Some(image) = image {
                            let mut hash = Sha256::new();
                            hash.update(path.as_os_str().to_string_lossy().as_bytes());
                            hash.update(material.images[slot].unwrap_or(usize::MAX).to_le_bytes());
                            hash.update([if slot == 0 {
                                0
                            } else if slot == 2 {
                                2
                            } else {
                                1
                            }]);
                            if slot == 2 {
                                hash.update(material.occlusion.unwrap_or(usize::MAX).to_le_bytes());
                            }
                            let digest = hash.finalize();
                            let mut id = [0u8; 16];
                            id.copy_from_slice(&digest[..16]);
                            image.key = Uuid::from_bytes(id);
                        }
                    }
                }
                let mut entry = Entry::new(
                    path.to_owned(),
                    Ok(Source::Material {
                        value,
                        images: Box::new(images),
                    }),
                    options,
                );
                entry.name = format!("{} ({})", path.file_stem().unwrap_or_default().to_string_lossy(), material.name);
                entry.material_index = index;
                entry.selected = choice != Choice::Material || index == 0;
                entry.local = choice == Choice::Local;
                if entry.local {
                    entry.watch = Some(local::Watch::new(dependencies(&doc, &root, path)));
                }
                entries.push(entry);
            }
            if entries.is_empty() {
                return Err(AssetError::invalid("document has no material"));
            }
            return Ok(entries);
        }
        _ => return Err(AssetError::Unsupported(format!("import format {extension}"))),
    };
    let mut entry = Entry::new(path.to_owned(), Ok(source), options);
    if matches!(&entry.source, Some(Source::Sound(sound)) if sound.duration() > sound_limit as f32) {
        entry.sound.options.segment = Some(sound_limit as f32);
    }
    entry.local = choice == Choice::Local;
    if entry.local {
        entry.watch = Some(local::Watch::new(vec![path.to_owned()]));
    }
    Ok(vec![entry])
}

pub(super) fn localize(entries: &mut [Entry], prefs: &Preferences) {
    for entry in entries {
        entry.local = true;
        entry.local_assume_meters = prefs.local_assume_meters;
        if let Some(Source::Mesh(draft)) = &entry.source {
            entry.local_live = prefs.local_mesh_auto_reload;
            let mut paths = mesh_dependencies(&entry.path, &draft.scene);
            for lod in &draft.lods {
                if let Some(path) = &lod.path {
                    paths.extend(
                        lod.file
                            .as_ref()
                            .map_or_else(|| vec![path.clone()], |scene| mesh_dependencies(path, scene)),
                    );
                }
            }
            if let Some(path) = &draft.physics_path {
                paths.extend(
                    draft
                        .physics_file
                        .as_ref()
                        .map_or_else(|| vec![path.clone()], |scene| mesh_dependencies(path, scene)),
                );
            }
            paths.sort();
            paths.dedup();
            entry.watch = Some(super::local::Watch::new(paths));
        }
    }
}

fn mesh_dependencies(path: &Path, scene: &aurora_assets::import_mesh::Scene) -> Vec<PathBuf> {
    let mut paths = vec![path.to_owned()];
    if let Some(root) = path.parent() {
        paths.extend(
            scene
                .source_uris
                .iter()
                .chain(scene.image_uris.values())
                .filter_map(|uri| collada_image_path(root, uri).ok()),
        );
    }
    paths
}

pub(super) fn encode(source: &Source, options: import_texture::Options, sound_options: sound::Options) -> Result<Vec<u8>, String> {
    let result = match source {
        Source::Texture(pixels) => import_texture::prepare(pixels, options).and_then(|image| import_texture::encode(&image, options)),
        Source::Sound(sound) => return sound_options.render(sound)?.encode().map_err(|error| error.to_string()),
        Source::Animation { bytes, animation, bvh } => {
            if bvh.is_some() {
                import_animation::encode(animation)
            } else {
                Ok(bytes.as_ref().clone())
            }
        }
        Source::Material { value, .. } => value.encode_asset(),
        Source::Mesh(_) => Err(AssetError::invalid("mesh upload requires a fee quote")),
    };
    result.map_err(|e| e.to_string())
}

impl Imports {
    /// One texture linked to an inventory material map. Upload remains in the
    /// normal inspector, including its price confirmation and retries.
    pub(crate) fn choose_material_map_upload(
        &mut self,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
        options: import_texture::Options,
    ) -> Option<Uuid> {
        if self.pending_files.is_some() || self.busy() || self.entries.len() >= 512 {
            return None;
        }
        let request = Uuid::new_v4();
        let entry_id = Uuid::new_v4();
        self.pending_files = Some(request);
        self.open(Choice::Texture);
        self.active = Some(entry_id);
        for entry in &mut self.entries {
            entry.selected = false;
        }
        let prefs = self.load_preferences.clone();
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let mut entries = Vec::new();
            if let Some(path) = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-title"))
                .add_filter(Choice::Texture.label(), &["tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2"])
                .pick_file()
            {
                entries = load(&path, Choice::Texture, options, 30, &prefs, false)
                    .unwrap_or_else(|reason| vec![Entry::new(path, Err(reason.to_string()), options)]);
                for entry in &mut entries {
                    entry.id = entry_id;
                }
            }
            let _ = tx.send(Worker::Loaded(request, entries));
            wake.request_repaint();
        });
        Some(entry_id)
    }
    pub fn map_file(&mut self, id: Uuid, slot: usize, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        let generation = entry.preview_generation;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-choose-files"))
                .add_filter(
                    aurora_i18n::tr!("import-texture"),
                    &["png", "jpg", "jpeg", "tga", "bmp", "j2c", "j2k", "jp2"],
                )
                .pick_file()
            {
                let result = read(&path)
                    .and_then(|data| import_texture::decode_map(&data))
                    .map(|pixels| {
                        let mut image = local_image(pixels, path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
                        image.path = Some(path);
                        image
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(Worker::Map(id, generation, slot, result));
                wake.request_repaint();
            }
        });
    }
    pub fn mesh_file(&mut self, id: Uuid, level: usize, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let meters = self.load_preferences.local_assume_meters && self.entries.iter().any(|e| e.id == id && e.local);
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-choose-files"))
                .add_filter(aurora_i18n::tr!("import-mesh"), &["dae", "gltf", "glb"])
                .pick_file()
            {
                let result = read(&path)
                    .and_then(|data| mesh_scene(&path, &data, meters, None, model_limit))
                    .map(|(scene, images)| (Arc::new(scene), images, path))
                    .map_err(|e| e.to_string());
                let _ = tx.send(Worker::MeshFile(id, level, result));
                wake.request_repaint();
            }
        });
    }
    pub fn mesh_file_path(&mut self, id: Uuid, path: PathBuf, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let meters = self.load_preferences.local_assume_meters && self.entries.iter().any(|e| e.id == id && e.local);
        let model_limit = self.load_preferences.mesh_model_limit;
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = read(&path)
                .and_then(|data| mesh_scene(&path, &data, meters, None, model_limit))
                .map(|(scene, images)| (Arc::new(scene), images, path))
                .map_err(|e| e.to_string());
            let _ = tx.send(Worker::MeshFile(id, 8, result));
            wake.request_repaint();
        });
    }
    pub fn reload(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && !e.reloading && !e.from_live) else {
            return;
        };
        entry.reloading = true;
        entry.status = Status::Preparing;
        let path = entry.path.clone();
        let remembered = entry.local.then(|| persistence::Asset::capture(entry));
        let options = entry.options;
        let sound_limit = self.sound_limit;
        let prefs = self.load_preferences.clone();
        let local = entry.local;
        let choice = if entry.local && matches!(entry.source, Some(Source::Mesh(_))) {
            Choice::Mesh
        } else if entry.local {
            Choice::Local
        } else {
            match entry.source.as_ref().map(Source::kind) {
                Some(Kind::Mesh) => Choice::Mesh,
                _ => self.choice,
            }
        };
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            if let Some(remembered) = remembered {
                let _ = tx.send(Worker::Reloaded(id, remembered.restore(&prefs).map(|entry| vec![entry])));
                wake.request_repaint();
                return;
            }
            let _ = tx.send(Worker::Reloaded(
                id,
                load(&path, choice, options, sound_limit, &prefs, local)
                    .map(|mut entries| {
                        if local {
                            localize(&mut entries, &prefs);
                        }
                        entries
                    })
                    .map_err(|e| e.to_string()),
            ));
            wake.request_repaint();
        });
    }
    pub fn choose_files(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, options: import_texture::Options) {
        if self.busy() {
            return;
        }
        self.load_files(jobs, ctx, options, self.choice, self.choice == Choice::Local, None);
    }
    /// The texture picker's Local tab "Add" (LLFloaterTexturePicker::onBtnAdd,
    /// indra/newview/lltexturectrl.cpp): images, or the materials of glTF
    /// files, kept as local previews. Local entries never join an upload, so
    /// a running one does not hold them back.
    pub fn choose_local_files(
        &mut self,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
        options: import_texture::Options,
        materials: bool,
    ) {
        if self.pending_files.is_some() {
            return;
        }
        let choice = if materials { Choice::Material } else { Choice::Texture };
        self.load_files(jobs, ctx, options, choice, true, None);
        self.picker_files = true;
    }
    /// The Local tab's "Upload" (onBtnUpload): the importer opens on the
    /// file's type and loads it again as an ordinary upload; the local
    /// preview stays.
    pub fn upload_local(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, options: import_texture::Options) {
        let Some(entry) = self.entries.iter().find(|e| e.id == id && e.local) else {
            return;
        };
        let choice = match entry.source {
            Some(Source::Texture(_)) => Choice::Texture,
            Some(Source::Material { .. }) => Choice::Material,
            _ => return,
        };
        let path = entry.path.clone();
        self.open(choice);
        if !self.busy() {
            self.load_files(jobs, ctx, options, choice, false, Some(vec![path]));
        }
    }
    /// Loads `paths`, or the files chosen in a dialog filtered on `choice`.
    fn load_files(
        &mut self,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
        options: import_texture::Options,
        choice: Choice,
        local: bool,
        paths: Option<Vec<PathBuf>>,
    ) {
        if self.pending_files.is_some() {
            return;
        }
        let request = Uuid::new_v4();
        self.pending_files = Some(request);
        self.picker_files = false;
        let sound_limit = self.sound_limit;
        let prefs = self.load_preferences.clone();
        let mut retained = self
            .entries
            .iter()
            .filter_map(|e| e.source.as_ref())
            .filter(|source| !matches!(source, Source::Mesh(_)))
            .map(Source::retained_bytes)
            .sum::<usize>();
        let existing = self.entries.len();
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let paths = paths.unwrap_or_else(|| {
                let filter: &[&str] = match choice {
                    Choice::Texture => &["tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2"],
                    Choice::Sound => &["wav"],
                    Choice::Animation => &["bvh", "anim"],
                    Choice::Material => &["gltf", "glb"],
                    Choice::Mesh => &["dae", "gltf", "glb"],
                    Choice::Bulk => &[
                        "tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2", "wav", "anim", "dae", "gltf", "glb",
                    ],
                    Choice::Local => &["tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2", "dae", "gltf", "glb"],
                };
                rfd::FileDialog::new()
                    .set_title(aurora_i18n::tr!("import-title"))
                    .add_filter(choice.label(), filter)
                    .pick_files()
                    .unwrap_or_default()
            });
            let mut entries = Vec::new();
            for (index, path) in paths.into_iter().enumerate() {
                if index >= 256 || existing + entries.len() >= 512 {
                    entries.push(Entry::new(path, Err(aurora_i18n::tr!("import-batch-limit").into()), options));
                    break;
                }
                match load(&path, choice, options, sound_limit, &prefs, local) {
                    Ok(mut loaded) => {
                        let size = loaded
                            .iter()
                            .filter_map(|e| e.source.as_ref())
                            .filter(|source| !matches!(source, Source::Mesh(_)))
                            .map(Source::retained_bytes)
                            .sum::<usize>();
                        // Preserve a failed row instead of silently truncating
                        // the selection or allowing the draft to exhaust RAM.
                        if retained.saturating_add(size) > 1024 * 1024 * 1024 || existing + entries.len() + loaded.len() > 512 {
                            entries.push(Entry::new(path, Err(aurora_i18n::tr!("import-batch-limit").into()), options));
                            break;
                        }
                        retained += size;
                        entries.append(&mut loaded);
                    }
                    Err(error) => entries.push(Entry::new(path, Err(error.to_string()), options)),
                }
            }
            if local {
                localize(&mut entries, &prefs);
            }
            let _ = tx.send(Worker::Loaded(request, entries));
            wake.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_loads_collada_and_detects_glb_meshes_without_changing_material_extraction() {
        let root = std::env::temp_dir().join(format!("aurora-batch-{}", Uuid::new_v4()));
        super::super::demo::fixtures(&root).expect("fixtures");
        let model = root.join("aurora-3d.glb");
        std::fs::write(&model, include_bytes!("../../../../assets/models/aurora-3d.glb")).expect("GLB fixture");
        let prefs = Preferences::default();
        for (path, kind) in [
            (root.join("mesh-demo.dae"), Kind::Mesh),
            (model.clone(), Kind::Mesh),
            (root.join("material-demo.gltf"), Kind::Material),
            (root.join("texture-demo.png"), Kind::Texture),
        ] {
            let entries = load(&path, Choice::Bulk, prefs.texture(), 30, &prefs, false).expect("batch file");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].source.as_ref().expect("source").kind(), kind);
            assert!(Choice::Bulk.accepts(&entries[0]));
        }
        let entries = load(&model, Choice::Material, prefs.texture(), 30, &prefs, false).expect("extract material");
        assert!(matches!(entries[0].source, Some(Source::Material { .. })));
        assert!(load(&root.join("animation-demo.bvh"), Choice::Bulk, prefs.texture(), 30, &prefs, false).is_err());
        std::fs::remove_dir_all(root).expect("remove own fixtures");
    }

    #[test]
    #[ignore = "requires AURORA_IMPORT_TEST_DIR local asset fixtures"]
    fn local_asset_directory_loads_and_prepares_supported_files() {
        let root = PathBuf::from(std::env::var_os("AURORA_IMPORT_TEST_DIR").expect("fixture directory"));
        let prefs = Preferences {
            mesh_suffixes: ["lowest".into(), "low".into(), "medium".into(), "high".into(), "physics".into()],
            ..Default::default()
        };
        let mut counts = [0usize; 4];
        let mut errors = Vec::new();
        let mut paths = Vec::new();
        for dir in [root.clone(), root.join("meshes"), root.join("textures")] {
            if dir.is_dir() {
                paths.extend(std::fs::read_dir(dir).expect("directory").map(|e| e.expect("entry").path()));
            }
        }
        paths.sort();
        for path in paths {
            let ext = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
            let choice = match ext.as_str() {
                "dae" if path.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("_high")) => Choice::Mesh,
                "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2" => Choice::Texture,
                "wav" => Choice::Sound,
                "bvh" | "anim" => Choice::Animation,
                "gltf" | "glb" => Choice::Material,
                _ => continue,
            };
            let result = load(&path, choice, prefs.texture(), 30, &prefs, false).map_err(|e| e.to_string());
            let result = result.and_then(|entries| {
                for entry in entries {
                    let source = entry.source.as_ref().ok_or("missing source")?;
                    if let Source::Mesh(draft) = source {
                        if !draft.scene.warnings.is_empty() {
                            return Err(format!("mesh warnings: {:?}", draft.scene.warnings));
                        }
                        let prepared = mesh::prepare(draft).map_err(|e| e.to_string())?;
                        let triangles: Vec<_> = (0..4)
                            .map(|level| {
                                prepared
                                    .lods
                                    .iter()
                                    .filter_map(|m| m[level].as_ref())
                                    .map(|m| m.triangles())
                                    .sum::<usize>()
                            })
                            .collect();
                        for (level, lod) in draft.lods.iter().enumerate().take(3) {
                            if let Some(file) = &lod.file {
                                assert_eq!(triangles[level], file.models.iter().map(|m| m.triangles()).sum::<usize>());
                            }
                        }
                        mesh::request(
                            draft,
                            &prepared,
                            Uuid::new_v4(),
                            Uuid::new_v4(),
                            "Fixture",
                            "",
                            Permissions::default(),
                            prefs.texture(),
                        )
                        .map_err(|e| e.to_string())?;
                        counts[0] += 1;
                    } else {
                        let data = encode(source, entry.options, entry.sound.options)?;
                        assert!(!data.is_empty());
                        let index = match source {
                            Source::Texture(_) => 1,
                            Source::Sound(_) => 2,
                            Source::Animation { animation, bvh, .. } => {
                                if bvh.is_none() {
                                    assert!(
                                        data == read(&entry.path).expect("original bytes"),
                                        "native animation bytes were altered"
                                    );
                                }
                                let rig = crate::scene::anim::Rig::new(
                                    &aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))
                                        .expect("skeleton"),
                                );
                                let bound = Arc::new(crate::scene::anim::BoundAnim::bind(animation.as_ref().clone(), &rig));
                                assert!(bound.joints.iter().any(Option::is_some), "no recognized animation joints");
                                let mut player = super::super::animation::Player::default();
                                player.bound = Some(bound);
                                player.seek(animation.duration * 0.5);
                                player.tick(entry.id, &rig, &mut crate::scene::anim::AnimStreamer::new(PathBuf::new()));
                                assert!(player.pose.expect("sampled pose").iter().flatten().flatten().all(|v| v.is_finite()));
                                3
                            }
                            _ => 3,
                        };
                        counts[index] += 1;
                    }
                }
                Ok(())
            });
            if let Err(error) = result {
                // Local fixture traces intentionally omit personal names/paths.
                errors.push(format!(
                    "fixture {} ({ext}): {error}",
                    counts.iter().sum::<usize>() + errors.len() + 1
                ));
            }
        }
        println!(
            "Loaded: {} meshes, {} textures, {} sounds, {} animations/materials",
            counts[0], counts[1], counts[2], counts[3]
        );
        assert!(errors.is_empty(), "{}", errors.join("\n"));
        assert!(counts.iter().sum::<usize>() > 0, "no supported fixture files");
    }

    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("aurora-import-files-{}", Uuid::new_v4()));
            super::super::demo::fixtures(&root).expect("fixtures");
            Self(root)
        }
        fn load(&self, name: &str, choice: Choice) -> Vec<Entry> {
            load(
                &self.0.join(name),
                choice,
                Default::default(),
                30,
                &Preferences::default(),
                choice == Choice::Local,
            )
            .expect("load")
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            // Only this test's freshly created UUID directory is removed.
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn local_picker_detects_textures_materials_collada_and_gltf_meshes_together() {
        let files = Files::new();
        assert!(matches!(
            files.load("texture-demo.png", Choice::Local)[0].source,
            Some(Source::Texture(_))
        ));
        assert!(matches!(
            files.load("material-demo.gltf", Choice::Local)[0].source,
            Some(Source::Material { .. })
        ));
        assert!(matches!(
            files.load("mesh-demo.dae", Choice::Local)[0].source,
            Some(Source::Mesh(_))
        ));
        let points: [f32; 9] = [0., 0., 0., 1., 0., 0., 0., 1., 1.];
        let bytes: Vec<_> = points.into_iter().flat_map(f32::to_le_bytes).collect();
        std::fs::write(files.0.join("mesh.bin"), &bytes).expect("buffer");
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{"uri": "mesh.bin", "byteLength": bytes.len()}],
            "bufferViews": [{"buffer": 0, "byteLength": bytes.len()}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "nodes": [{"mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0
        });
        std::fs::write(files.0.join("mesh.gltf"), json.to_string()).expect("document");
        let mut entries = files.load("mesh.gltf", Choice::Local);
        localize(&mut entries, &Preferences::default());
        let Some(Source::Mesh(draft)) = &entries[0].source else {
            panic!("glTF mesh, not a material")
        };
        assert_eq!(draft.scene.models[0].triangles(), 1);
        assert!(
            entries[0]
                .watch
                .as_ref()
                .expect("watch")
                .paths
                .contains(&files.0.join("mesh.bin").canonicalize().expect("path"))
        );
    }
    #[test]
    fn long_wav_sources_enable_bounded_splitting_instead_of_uploading_an_oversized_asset() {
        let files = Files::new();
        let mut wav = std::fs::read(files.0.join("sound-demo.wav")).expect("WAV fixture");
        let length = 44100_u32 * 70 * 4;
        wav.resize(length as usize + 44, 0);
        wav[4..8].copy_from_slice(&(length + 36).to_le_bytes());
        wav[40..44].copy_from_slice(&length.to_le_bytes());
        std::fs::write(files.0.join("long.wav"), wav).expect("long fixture");
        let entries = files.load("long.wav", Choice::Sound);
        let entry = &entries[0];
        let Some(Source::Sound(source)) = &entry.source else {
            panic!("sound")
        };
        assert_eq!(source.duration(), 70.0);
        assert_eq!(entry.sound.options.segment, Some(30.0));
        assert_eq!(entry.sound.options.pieces(source).expect("pieces"), 3);
        assert!(entry.sound.options.importable(source, 30));
        assert!(!sound::Options::default().importable(source, 30));
        assert_eq!(
            sound::Options {
                piece: Some(2),
                ..entry.sound.options
            }
            .duration(source),
            10.0
        );
    }
    #[test]
    fn disk_files_follow_filters_codecs_and_mesh_reload() {
        let files = Files::new();
        assert!(matches!(
            files.load("texture-demo.png", Choice::Texture)[0].source,
            Some(Source::Texture(_))
        ));
        assert!(matches!(
            files.load("sound-demo.wav", Choice::Sound)[0].source,
            Some(Source::Sound(_))
        ));
        assert!(matches!(
            files.load("animation-demo.bvh", Choice::Animation)[0].source,
            Some(Source::Animation { bvh: Some(_), .. })
        ));
        let native = files.load("animation-demo.anim", Choice::Bulk).remove(0);
        assert_eq!(
            encode(native.source.as_ref().expect("animation"), native.options, native.sound.options).expect("encode"),
            read(&native.path).expect("bytes")
        );
        assert!(
            load(
                &files.0.join("animation-demo.bvh"),
                Choice::Bulk,
                Default::default(),
                30,
                &Preferences::default(),
                false
            )
            .is_err()
        );
        assert!(
            load(
                &files.0.join("mesh-demo.dae"),
                Choice::Bulk,
                Default::default(),
                30,
                &Preferences::default(),
                false
            )
            .is_ok()
        );
        let mut mesh = files.load("mesh-demo.dae", Choice::Mesh);
        localize(&mut mesh, &Preferences::default());
        let Some(Source::Mesh(draft)) = &mesh[0].source else {
            panic!("mesh")
        };
        assert_eq!(draft.scene.models[0].triangles(), 12);
        assert_eq!(draft.lods[3].path.as_ref(), Some(&mesh[0].path));
        assert!(!mesh[0].local_live);
        assert_eq!(mesh[0].watch.as_ref().expect("watch").paths, [mesh[0].path.clone()]);
    }
    #[test]
    fn material_dependency_changes_reload_pixels_with_stable_local_ids() {
        let files = Files::new();
        let before = files.load("material-demo.gltf", Choice::Local).remove(0);
        let watch = before.watch.as_ref().expect("watch");
        assert!(
            watch
                .paths
                .contains(&files.0.join("texture-demo.png").canonicalize().expect("path"))
        );
        let Some(Source::Material { images: original, .. }) = before.source else {
            panic!("material")
        };
        image::save_buffer(
            files.0.join("texture-demo.png"),
            &[180u8; 16 * 16 * 4],
            16,
            16,
            image::ColorType::Rgba8,
        )
        .expect("image");
        assert_ne!(watch.stamp, super::super::local::stamp(&watch.paths));
        let after = files.load("material-demo.gltf", Choice::Local).remove(0);
        let Some(Source::Material { images: updated, .. }) = after.source else {
            panic!("material")
        };
        let (a, b) = (original[0].as_ref().expect("map"), updated[0].as_ref().expect("map"));
        assert_eq!(a.key, b.key);
        assert_eq!(b.pixels.width, 16);
        assert_ne!(a.pixels.data, b.pixels.data);
        std::fs::remove_file(files.0.join("texture-demo.png")).expect("remove fixture");
        assert!(
            load(
                &files.0.join("material-demo.gltf"),
                Choice::Local,
                Default::default(),
                30,
                &Preferences::default(),
                true
            )
            .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn collada_resolves_local_exporter_file_uris_and_rejects_remote_resources() {
        let files = Files::new();
        let path = files.0.join("texture-demo.png").canonicalize().expect("path");
        let without_prefix = path.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
        assert_eq!(
            collada_image_path(&files.0, &format!("file:///{without_prefix}")).expect("file URI"),
            path
        );
        for uri in [
            "https://example.invalid/image.png",
            "file://server/share/image.png",
            r"\\server\share\image.png",
        ] {
            assert!(collada_image_path(&files.0, uri).is_err());
        }
        assert!(relative_path(&files.0, &without_prefix).is_err());
        assert_eq!(decode_uri("carte%20%C3%A9t%C3%A9+1.png").expect("UTF-8"), "carte été+1.png");
    }
}
