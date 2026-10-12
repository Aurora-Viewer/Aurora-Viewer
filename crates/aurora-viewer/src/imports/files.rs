//! Native file selection and bounded loading, off the frame thread.

use super::*;
use aurora_assets::{
    AssetError, import_animation,
    import_gltf::{Document, Image},
    import_sound,
};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

fn read(path: &Path) -> Result<Vec<u8>, AssetError> {
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

fn relative_path(root: &Path, uri: &str) -> Result<PathBuf, AssetError> {
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
    let decoded = String::from_utf8(decoded).map_err(|_| AssetError::invalid("resource URI encoding"))?;
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
        key: Uuid::from_bytes(id),
        empty_alpha: import_texture::empty_alpha(&pixels),
        pixels: Arc::new(pixels),
        asset: None,
        name,
    }
}
fn mesh_scene(path: &Path, data: &[u8]) -> Result<(aurora_assets::import_mesh::Scene, HashMap<usize, LocalImage>), AssetError> {
    let root = path
        .parent()
        .ok_or_else(|| AssetError::invalid("mesh directory"))?
        .canonicalize()
        .map_err(|_| AssetError::invalid("mesh directory"))?;
    let skeleton = aurora_assets::Skeleton::parse(include_bytes!("../../assets/character/avatar_skeleton.xml"))?;
    let mut images = HashMap::new();
    let scene = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dae")) {
        let mut scene = aurora_assets::import_mesh::collada::parse(data, &skeleton)?;
        for (index, uri) in &scene.image_uris {
            match relative(&root, uri).and_then(|bytes| import_texture::decode_map(&bytes)) {
                Ok(pixels) => {
                    images.insert(*index, local_image(pixels, uri.clone()));
                }
                Err(error) => scene.warnings.push(error.to_string()),
            }
        }
        scene
    } else {
        let doc = Document::parse(data, |uri| relative(&root, uri))?;
        let mut scene = aurora_assets::import_mesh::gltf::parse(&doc, &skeleton)?;
        for material in &scene.materials {
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

fn load(
    path: &Path,
    choice: Choice,
    options: import_texture::Options,
    sound_limit: u32,
    prefs: &Preferences,
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
        Choice::Bulk => !matches!(extension.as_str(), "dae" | "bvh"),
        Choice::Local => matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "bmp" | "tga" | "j2c" | "j2k" | "jp2" | "gltf" | "glb"
        ),
    };
    if !matches_choice {
        return Err(AssetError::Unsupported("file type for this import window".into()));
    }
    let source = match extension.as_str() {
        "dae" | "gltf" | "glb" if choice == Choice::Mesh => {
            let (scene, images) = mesh_scene(path, &data)?;
            let mut draft = mesh::Draft::new(scene, images);
            draft.options.skin &= prefs.mesh_auto_weights;
            draft.preview_weights &= prefs.mesh_auto_preview_weights && draft.options.skin;
            if prefs.mesh_reliable_default {
                for lod in &mut draft.lods[..3] {
                    lod.source = mesh::LodSource::Reliable;
                }
            }
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            for level in [2, 1, 0, 4] {
                let suffix = &prefs.mesh_suffixes[level];
                if suffix.is_empty() || suffix.contains(['/', '\\', ':']) {
                    continue;
                }
                let candidate = path.with_file_name(format!("{stem}_{suffix}.{extension}"));
                if candidate.is_file() {
                    match read(&candidate).and_then(|data| mesh_scene(&candidate, &data)) {
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
        "wav" => Source::Sound(Arc::new(import_sound::decode_wav(&data, sound_limit)?)),
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
            let doc = Document::parse(&data, |uri| relative(&root, uri))?;
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
    entry.local = choice == Choice::Local;
    if entry.local {
        entry.watch = Some(local::Watch::new(vec![path.to_owned()]));
    }
    Ok(vec![entry])
}

pub(super) fn encode(source: &Source, options: import_texture::Options) -> Result<Vec<u8>, String> {
    let result = match source {
        Source::Texture(pixels) => import_texture::prepare(pixels, options).and_then(|image| import_texture::encode(&image, options)),
        Source::Sound(sound) => sound.encode(),
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
                    .map(|pixels| local_image(pixels, path.file_stem().unwrap_or_default().to_string_lossy().into_owned()))
                    .map_err(|e| e.to_string());
                let _ = tx.send(Worker::Map(id, generation, slot, result));
                wake.request_repaint();
            }
        });
    }
    pub fn mesh_file(&mut self, id: Uuid, level: usize, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-choose-files"))
                .add_filter(aurora_i18n::tr!("import-mesh"), &["dae", "gltf", "glb"])
                .pick_file()
            {
                let result = read(&path)
                    .and_then(|data| mesh_scene(&path, &data))
                    .map(|(scene, images)| (Arc::new(scene), images, path))
                    .map_err(|e| e.to_string());
                let _ = tx.send(Worker::MeshFile(id, level, result));
                wake.request_repaint();
            }
        });
    }
    pub fn mesh_file_path(&mut self, id: Uuid, path: PathBuf, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let result = read(&path)
                .and_then(|data| mesh_scene(&path, &data))
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
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        let path = entry.path.clone();
        let options = entry.options;
        let sound_limit = self.sound_limit;
        let prefs = self.load_preferences.clone();
        let choice = if entry.local {
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
            let _ = tx.send(Worker::Reloaded(
                id,
                load(&path, choice, options, sound_limit, &prefs).map_err(|e| e.to_string()),
            ));
            wake.request_repaint();
        });
    }
    pub fn choose_files(&mut self, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, options: import_texture::Options) {
        if self.pending_files.is_some() || self.busy() {
            return;
        }
        let request = Uuid::new_v4();
        self.pending_files = Some(request);
        let choice = self.choice;
        let sound_limit = self.sound_limit;
        let prefs = self.load_preferences.clone();
        let mut retained = self
            .entries
            .iter()
            .filter_map(|e| e.source.as_ref())
            .map(Source::retained_bytes)
            .sum::<usize>();
        let existing = self.entries.len();
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let filter: &[&str] = match choice {
                Choice::Texture => &["tga", "bmp", "jpg", "jpeg", "png", "j2c", "j2k", "jp2"],
                Choice::Sound => &["wav"],
                Choice::Animation => &["bvh", "anim"],
                Choice::Material => &["gltf", "glb"],
                Choice::Mesh => &["dae", "gltf", "glb"],
                Choice::Bulk => &["tga", "bmp", "jpg", "jpeg", "png", "wav", "anim", "gltf", "glb"],
                Choice::Local => &["tga", "bmp", "jpg", "jpeg", "png", "gltf", "glb"],
            };
            let paths = rfd::FileDialog::new()
                .set_title(aurora_i18n::tr!("import-title"))
                .add_filter(choice.label(), filter)
                .pick_files()
                .unwrap_or_default();
            let mut entries = Vec::new();
            for (index, path) in paths.into_iter().enumerate() {
                if index >= 256 || existing + entries.len() >= 512 {
                    entries.push(Entry::new(path, Err(aurora_i18n::tr!("import-batch-limit").into()), options));
                    break;
                }
                match load(&path, choice, options, sound_limit, &prefs) {
                    Ok(mut loaded) => {
                        let size = loaded
                            .iter()
                            .filter_map(|e| e.source.as_ref())
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
            let _ = tx.send(Worker::Loaded(request, entries));
            wake.request_repaint();
        });
    }
}
