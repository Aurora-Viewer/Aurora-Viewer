//! Remembered local previews. Firestorm's local bitmap/material/mesh lists
//! inspire the references and live reload; retaining the library and client-only
//! object bindings across sessions is an intentional Aurora improvement.
//! Only metadata is saved in the private profile, never source pixels/geometry.

use super::*;
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FILE: &str = "local_previews.json";
const MAX_REGISTRY: u64 = 16 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
enum Map {
    Source,
    File(PathBuf),
    Asset(Uuid),
    Empty,
}
#[derive(Clone, Serialize, Deserialize)]
struct Lod {
    source: mesh::LodSource,
    path: Option<PathBuf>,
    triangles: usize,
    error: f32,
    error_mode: bool,
}
#[derive(Clone, Serialize, Deserialize)]
enum Parameters {
    Texture,
    Material {
        asset: Vec<u8>,
        maps: [Map; 4],
    },
    Mesh {
        options: mesh::Options,
        lods: [Lod; 4],
        physics: Option<PathBuf>,
        view: [bool; 6],
        lod: usize,
        explode: f32,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Asset {
    id: Uuid,
    path: PathBuf,
    name: String,
    material_index: usize,
    parameters: Option<Parameters>,
    size: [u32; 2],
    lossless: bool,
    strip_alpha: bool,
    live: bool,
    meters: bool,
    active: bool,
    model: usize,
    use_scale: bool,
    texture_target: usize,
    camera: [f32; 3],
    pan: [f32; 2],
    edges: bool,
    uv: bool,
    watched: Vec<PathBuf>,
}
impl Asset {
    pub fn capture(entry: &Entry) -> Self {
        let parameters = match &entry.source {
            Some(Source::Texture(_)) => Some(Parameters::Texture),
            Some(Source::Material { value, images }) => Some(Parameters::Material {
                asset: value.encode_asset().unwrap_or_default(),
                maps: std::array::from_fn(|slot| match &images[slot] {
                    Some(image) if image.asset.is_some() => Map::Asset(image.asset.expect("existing asset")),
                    Some(image) => image.path.as_ref().map_or(Map::Source, |path| Map::File(path.clone())),
                    None => maps::ids(value)[slot].map_or(Map::Empty, Map::Asset),
                }),
            }),
            Some(Source::Mesh(draft)) => Some(Parameters::Mesh {
                options: draft.options.clone(),
                lods: std::array::from_fn(|level| {
                    let lod = &draft.lods[level];
                    Lod {
                        source: lod.source,
                        path: lod.path.clone(),
                        triangles: lod.triangles,
                        error: lod.error,
                        error_mode: lod.error_mode,
                    }
                }),
                physics: draft.physics_path.clone(),
                view: [
                    draft.preview_physics,
                    draft.show_textures,
                    draft.preview_weights,
                    draft.show_joints,
                    draft.show_overrides,
                    entry.local_use_scale,
                ],
                lod: draft.preview_lod,
                explode: draft.explode,
            }),
            _ => entry.local_saved.as_ref().and_then(|saved| saved.parameters.clone()),
        };
        let mut watched = entry
            .watch
            .as_ref()
            .map(|watch| watch.paths.clone())
            .unwrap_or_else(|| vec![entry.path.clone()]);
        if let Some(Parameters::Material { maps, .. }) = &parameters {
            watched.extend(maps.iter().filter_map(|map| match map {
                Map::File(path) => Some(path.clone()),
                _ => None,
            }));
        }
        Self {
            id: entry.id,
            path: entry.path.clone(),
            name: entry.name.clone(),
            material_index: entry.material_index,
            parameters,
            size: [entry.options.max_width, entry.options.max_height],
            lossless: entry.options.lossless_small,
            strip_alpha: entry.options.strip_alpha,
            live: entry.local_live,
            meters: entry.local_assume_meters,
            active: entry.local_published || entry.local_restore_active,
            model: entry.local_model,
            use_scale: entry.local_use_scale,
            texture_target: entry.texture_target,
            camera: [entry.view.yaw, entry.view.pitch, entry.view.distance],
            pan: entry.view.pan,
            edges: entry.view.edges,
            uv: entry.view.uv,
            watched,
        }
    }
    fn metadata(&self, entry: &mut Entry) {
        entry.id = self.id;
        entry.name = self.name.clone();
        entry.material_index = self.material_index;
        entry.local = true;
        entry.local_live = self.live;
        entry.local_assume_meters = self.meters;
        entry.local_restore_active = self.active;
        entry.local_model = self.model;
        entry.local_use_scale = self.use_scale;
        entry.texture_target = self.texture_target.min(9);
        [entry.view.yaw, entry.view.pitch, entry.view.distance] = self.camera;
        entry.view.pan = self.pan;
        entry.view.edges = self.edges;
        entry.view.uv = self.uv;
        let paths = entry.watch.take().map_or_else(Vec::new, |watch| watch.paths);
        entry.watch = Some(local::Watch::new(paths.into_iter().chain(self.watched.iter().cloned()).collect()));
        entry.local_saved = Some(self.clone());
    }
    pub fn restore(&self, prefs: &Preferences) -> Result<Entry, String> {
        let options = import_texture::Options {
            max_width: self.size[0],
            max_height: self.size[1],
            lossless_small: self.lossless,
            strip_alpha: self.strip_alpha,
        };
        let prefs = Preferences {
            local_assume_meters: self.meters,
            ..prefs.clone()
        };
        let choice = match self.parameters {
            Some(Parameters::Texture) => Choice::Texture,
            Some(Parameters::Material { .. }) => Choice::Material,
            Some(Parameters::Mesh { .. }) => Choice::Mesh,
            None => Choice::Local,
        };
        if !self.path.is_file() {
            return Err(aurora_i18n::tr!("import-local-file-missing").into());
        }
        let mut loaded = files::load(&self.path, choice, options, 30, &prefs, true).map_err(|error| error.to_string())?;
        if self.material_index >= loaded.len() {
            return Err(aurora_i18n::tr!("import-reload-material-count").into());
        }
        let mut entry = loaded.remove(self.material_index);
        match (&self.parameters, &mut entry.source) {
            (Some(Parameters::Material { asset, maps }), Some(Source::Material { value, images })) => {
                *value = aurora_assets::parse_material_asset(asset).map_err(|error| error.to_string())?;
                for (slot, map) in maps.iter().enumerate() {
                    match map {
                        Map::Source => {}
                        Map::File(path) => {
                            let pixels = files::read(path)
                                .and_then(|bytes| import_texture::decode_map(&bytes))
                                .map_err(|error| error.to_string())?;
                            let mut image = files::local_image(pixels, path.file_stem().unwrap_or_default().to_string_lossy().into_owned());
                            image.path = Some(path.clone());
                            images[slot] = Some(image);
                            maps::set(value, slot, None);
                        }
                        Map::Asset(id) => {
                            images[slot] = None;
                            maps::set(value, slot, Some(*id));
                        }
                        Map::Empty => {
                            images[slot] = None;
                            maps::set(value, slot, None);
                        }
                    }
                }
            }
            (
                Some(Parameters::Mesh {
                    options,
                    lods,
                    physics,
                    view,
                    lod,
                    explode,
                }),
                Some(Source::Mesh(draft)),
            ) => {
                draft.options = options.clone();
                for (level, saved) in lods.iter().enumerate() {
                    let file = if level != 3 && saved.source == mesh::LodSource::File {
                        let path = saved
                            .path
                            .as_ref()
                            .ok_or_else(|| aurora_i18n::tr!("import-local-file-missing").to_owned())?;
                        let (scene, _) = files::read(path)
                            .and_then(|bytes| files::mesh_scene(path, &bytes, self.meters, None, prefs.mesh_model_limit))
                            .map_err(|error| error.to_string())?;
                        Some(Arc::new(scene))
                    } else {
                        None
                    };
                    draft.lods[level] = mesh::Lod {
                        source: saved.source,
                        path: saved.path.clone(),
                        file,
                        triangles: saved.triangles,
                        error: saved.error,
                        error_mode: saved.error_mode,
                    };
                }
                if let Some(path) = physics {
                    let (scene, _) = files::read(path)
                        .and_then(|bytes| files::mesh_scene(path, &bytes, self.meters, None, prefs.mesh_model_limit))
                        .map_err(|error| error.to_string())?;
                    draft.physics_file = Some(Arc::new(scene));
                    draft.physics_path = Some(path.clone());
                }
                [
                    draft.preview_physics,
                    draft.show_textures,
                    draft.preview_weights,
                    draft.show_joints,
                    draft.show_overrides,
                    entry.local_use_scale,
                ] = *view;
                draft.preview_lod = (*lod).min(3);
                draft.explode = *explode;
            }
            _ => {}
        }
        files::localize(std::slice::from_mut(&mut entry), &prefs);
        self.metadata(&mut entry);
        Ok(entry)
    }
    fn unavailable(&self, message: String) -> Entry {
        let mut entry = Entry::new(
            self.path.clone(),
            Err(message),
            import_texture::Options {
                max_width: self.size[0],
                max_height: self.size[1],
                lossless_small: self.lossless,
                strip_alpha: self.strip_alpha,
            },
        );
        self.metadata(&mut entry);
        entry
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Profile {
    meshes: Vec<(Uuid, local::Target)>,
    appearances: Vec<(Uuid, Option<u8>, crate::world::objects::LocalAppearance)>,
}
#[derive(Default, Serialize, Deserialize)]
struct Registry {
    version: u32,
    assets: Vec<Asset>,
    profiles: BTreeMap<String, Profile>,
}
struct Restored {
    registry: Registry,
    entries: Vec<Entry>,
}

#[derive(Default)]
pub struct State {
    started: bool,
    ready: bool,
    registry: Registry,
    profile: String,
    reader: Option<Receiver<Result<Restored, ()>>>,
    receipts: Option<Receiver<bool>>,
    writer: Option<Sender<Vec<u8>>>,
    worker: Option<JoinHandle<()>>,
    last: Vec<u8>,
    observed: Option<u64>,
    next: Option<Instant>,
}
impl Drop for State {
    fn drop(&mut self) {
        self.finish();
    }
}
impl State {
    pub fn started(&self) -> bool {
        self.started
    }
    pub fn disable(&mut self) {
        self.started = true;
    }
    fn finish(&mut self) {
        self.writer.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn read_registry(path: &Path) -> Result<Registry, ()> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Registry {
                version: 1,
                ..Default::default()
            });
        }
        Err(_) => return Err(()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_REGISTRY + 1).read_to_end(&mut bytes).map_err(|_| ())?;
    if bytes.len() as u64 > MAX_REGISTRY {
        return Err(());
    }
    let registry: Registry = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if registry.version != 1 || registry.assets.len() > 512 {
        return Err(());
    }
    let mut ids = HashSet::new();
    if registry.assets.iter().any(|asset| asset.id.is_nil() || !ids.insert(asset.id)) {
        return Err(());
    }
    Ok(registry)
}
fn write_registry(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    std::fs::write(&temporary, bytes)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}
pub fn profile(agent: Uuid, grid: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(grid.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{agent}/{digest}")
}
impl Imports {
    pub fn start_local_registry(&mut self, path: PathBuf, profile: String, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        if self.persistence.started {
            return;
        }
        self.persistence.started = true;
        self.persistence.profile = profile;
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.persistence.reader = Some(rx);
        let prefs = self.load_preferences.clone();
        let wake = ctx.clone();
        let read_path = path.clone();
        jobs.execute(move || {
            let restored = read_registry(&read_path).map(|registry| {
                let mut retained = 0usize;
                let entries = registry
                    .assets
                    .iter()
                    .map(|asset| {
                        let entry = asset.restore(&prefs).unwrap_or_else(|message| asset.unavailable(message));
                        let bytes = entry.source.as_ref().map_or(0, Source::retained_bytes);
                        if retained.saturating_add(bytes) > 1024 * 1024 * 1024 {
                            asset.unavailable(aurora_i18n::tr!("import-batch-limit").into())
                        } else {
                            retained += bytes;
                            entry
                        }
                    })
                    .collect();
                Restored { registry, entries }
            });
            let _ = tx.send(restored);
            wake.request_repaint();
        });
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        let (receipt, receipts) = crossbeam_channel::unbounded();
        self.persistence.receipts = Some(receipts);
        let worker = std::thread::Builder::new().name("aurora-local-previews".into()).spawn(move || {
            while let Ok(mut bytes) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    bytes = newer;
                }
                let saved = write_registry(&path, &bytes).is_ok();
                let _ = receipt.send(saved);
            }
        });
        if let Ok(worker) = worker {
            self.persistence.writer = Some(tx);
            self.persistence.worker = Some(worker);
        } else {
            self.message = aurora_i18n::tr!("import-local-save-failed").into();
        }
    }
    pub fn poll_local_registry(&mut self) {
        if let Some(receipts) = &self.persistence.receipts {
            for saved in receipts.try_iter() {
                if !saved {
                    self.persistence.last.clear();
                    self.persistence.observed = None;
                    self.persistence.next = Some(Instant::now() + Duration::from_secs(3));
                    self.message = aurora_i18n::tr!("import-local-save-failed").into();
                } else if self.message == aurora_i18n::tr!("import-local-save-failed") {
                    self.message.clear();
                }
            }
        }
        let Some(result) = self.persistence.reader.as_ref().and_then(|reader| reader.try_recv().ok()) else {
            return;
        };
        self.persistence.reader = None;
        match result {
            Ok(restored) => {
                self.persistence.registry = restored.registry;
                if let Some(profile) = self.persistence.registry.profiles.get(&self.persistence.profile) {
                    self.local_targets.extend(profile.meshes.iter().cloned());
                    self.local_appearance_targets.extend(
                        profile
                            .appearances
                            .iter()
                            .map(|(object, face, appearance)| ((*object, *face), *appearance)),
                    );
                }
                self.entries.extend(restored.entries);
                if self.active.is_none() {
                    self.active = self.entries.iter().find(|entry| self.choice.accepts(entry)).map(|entry| entry.id);
                }
                self.persistence.ready = true;
            }
            Err(()) => {
                // Preserve an unreadable/unsupported registry; never replace it
                // with an empty list merely because restoration failed.
                self.message = aurora_i18n::tr!("import-local-library-unavailable").into();
            }
        }
    }
    pub fn save_local_registry(&mut self, force: bool) {
        if !self.persistence.ready || (!force && self.persistence.next.is_some_and(|next| next > Instant::now())) {
            return;
        }
        self.persistence.next = Some(Instant::now() + Duration::from_millis(500));
        // Hash lightweight edit revisions/metadata first: an unchanged library
        // never re-encodes material assets or allocates a JSON snapshot.
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for entry in self.entries.iter().filter(|entry| entry.local) {
            (
                entry.id,
                &entry.path,
                &entry.name,
                entry.preview_generation,
                (
                    entry.local_live,
                    entry.local_assume_meters,
                    entry.local_published || entry.local_restore_active,
                    entry.local_model,
                    entry.local_use_scale,
                    entry.texture_target,
                ),
                (
                    entry.options.max_width,
                    entry.options.max_height,
                    entry.options.strip_alpha,
                    entry.options.lossless_small,
                ),
            )
                .hash(&mut hash);
            [
                entry.view.yaw,
                entry.view.pitch,
                entry.view.distance,
                entry.view.pan[0],
                entry.view.pan[1],
            ]
            .map(f32::to_bits)
            .hash(&mut hash);
            (entry.view.edges, entry.view.uv).hash(&mut hash);
        }
        for (object, target) in &self.local_targets {
            (object, target.entry, target.model, target.use_scale).hash(&mut hash);
        }
        for ((object, face), appearance) in &self.local_appearance_targets {
            (
                object,
                face,
                appearance.entry(),
                matches!(appearance, crate::world::objects::LocalAppearance::Material(_)),
            )
                .hash(&mut hash);
        }
        let observed = hash.finish();
        if !force && self.persistence.observed == Some(observed) {
            return;
        }
        self.persistence.observed = Some(observed);
        let assets: Vec<_> = self.entries.iter().filter(|entry| entry.local).map(Asset::capture).collect();
        let ids: HashSet<_> = assets.iter().map(|asset| asset.id).collect();
        let mut meshes: Vec<_> = self
            .local_targets
            .iter()
            .map(|(object, target)| (*object, target.clone()))
            .collect();
        meshes.sort_by_key(|(object, _)| *object);
        let mut appearances: Vec<_> = self
            .local_appearance_targets
            .iter()
            .map(|((object, face), appearance)| (*object, *face, *appearance))
            .collect();
        appearances.sort_by_key(|(object, face, _)| (*object, *face));
        self.persistence.registry.assets = assets;
        self.persistence
            .registry
            .profiles
            .insert(self.persistence.profile.clone(), Profile { meshes, appearances });
        for profile in self.persistence.registry.profiles.values_mut() {
            profile.meshes.retain(|(_, target)| ids.contains(&target.entry));
            profile.appearances.retain(|(_, _, appearance)| ids.contains(&appearance.entry()));
        }
        if let Ok(bytes) = serde_json::to_vec(&self.persistence.registry)
            && bytes.len() as u64 <= MAX_REGISTRY
            && bytes != self.persistence.last
            && let Some(writer) = &self.persistence.writer
            && writer.send(bytes.clone()).is_ok()
        {
            self.persistence.last = bytes;
        }
    }
    pub fn flush_local_registry(&mut self) {
        self.save_local_registry(true);
        self.persistence.finish();
    }
    pub fn registry_path() -> PathBuf {
        crate::settings::config_dir().join(FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("aurora-preview-registry-{}", Uuid::new_v4()));
            demo::fixtures(&root).expect("synthetic fixtures");
            Self(root)
        }
        fn entry(&self, name: &str) -> Entry {
            let mut entries = files::load(
                &self.0.join(name),
                Choice::Local,
                Default::default(),
                30,
                &Preferences::default(),
                true,
            )
            .expect("file");
            files::localize(&mut entries, &Preferences::default());
            entries.remove(0)
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn restored(path: PathBuf, profile: &str, jobs: &crate::scene::jobs::Jobs) -> Imports {
        let mut imports = Imports::default();
        imports.start_local_registry(path, profile.into(), jobs, &egui::Context::default());
        let deadline = Instant::now() + Duration::from_secs(10);
        while imports.persistence.reader.is_some() {
            assert!(Instant::now() < deadline, "restore worker finishes");
            imports.poll_local_registry();
            std::thread::sleep(Duration::from_millis(5));
        }
        imports
    }
    #[test]
    fn restart_preserves_all_types_settings_and_bindings_and_removal_never_deletes_sources() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let account = profile(Uuid::from_u128(10), "grid-one");
        let other_grid = profile(Uuid::from_u128(10), "grid-two");
        let mut imports = restored(path.clone(), &account, &jobs);
        let mut texture = files.entry("texture-demo.png");
        texture.name = "Remembered image".into();
        texture.options.max_width = 128;
        texture.options.strip_alpha = false;
        texture.local_live = false;
        texture.local_published = true;
        texture.view.yaw = 0.7;
        texture.texture_target = 7;
        let mut material = files.entry("material-demo.gltf");
        let Some(Source::Material { value, images }) = &mut material.source else {
            panic!("material")
        };
        value.base_color_factor = [0.1, 0.2, 0.3, 1.0];
        value.roughness_factor = 0.8;
        let custom = files.0.join("custom.png");
        image::save_buffer(&custom, &[90; 4 * 4 * 4], 4, 4, image::ColorType::Rgba8).expect("custom map");
        let mut map = files::local_image(
            import_texture::decode_map(&files::read(&custom).expect("bytes")).expect("image"),
            "custom".into(),
        );
        map.path = Some(custom.clone());
        images[0] = Some(map);
        material.local_published = true;
        let mut mesh = files.entry("mesh-demo.dae");
        mesh.local_published = true;
        mesh.local_use_scale = true;
        let Some(Source::Mesh(draft)) = &mut mesh.source else {
            panic!("mesh")
        };
        draft.options.scale = 0.75;
        draft.lods[1].error = 0.037;
        draft.lods[1].error_mode = true;
        draft.preview_lod = 1;
        let ids = [texture.id, material.id, mesh.id];
        imports.entries = vec![texture, material, mesh];
        let object = Uuid::from_u128(100);
        imports.target_local(ids[0], object, &[], false);
        imports.target_local(ids[1], object, &[1], false);
        imports.target_local(ids[2], object, &[], true);
        assert!(imports.persistence.ready, "registry ready");
        assert!(imports.persistence.writer.is_some(), "writer started: {}", imports.message);
        imports.flush_local_registry();
        serde_json::to_vec(&imports.persistence.registry).expect("registry serializes");
        assert!(!imports.persistence.last.is_empty(), "save queued");
        assert!(
            std::fs::metadata(&path).expect("registry").len() < 16_384,
            "metadata, not image/mesh bytes"
        );
        let mut restarted = restored(path.clone(), &account, &jobs);
        assert_eq!(restarted.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(), ids);
        assert!(
            restarted
                .entries
                .iter()
                .all(|entry| entry.local_restore_active && !entry.local_published)
        );
        assert_eq!(restarted.entries[0].name, "Remembered image");
        assert_eq!(restarted.entries[0].options.max_width, 128);
        assert!(!restarted.entries[0].options.strip_alpha && !restarted.entries[0].local_live);
        assert_eq!(restarted.entries[0].view.yaw, 0.7);
        assert_eq!(restarted.entries[0].texture_target, 7);
        let Some(Source::Material { value, images }) = &restarted.entries[1].source else {
            panic!("restored material")
        };
        assert_eq!(value.base_color_factor, [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(value.roughness_factor, 0.8);
        assert_eq!(images[0].as_ref().expect("custom map").path.as_ref(), Some(&custom));
        assert!(restarted.entries[1].watch.as_ref().expect("watch").paths.contains(&custom));
        let Some(Source::Mesh(draft)) = &restarted.entries[2].source else {
            panic!("restored mesh")
        };
        assert_eq!(draft.options.scale, 0.75);
        assert_eq!(draft.lods[1].error, 0.037);
        assert!(draft.lods[1].error_mode);
        assert_eq!(draft.preview_lod, 1);
        assert_eq!(restarted.local_targets[&object].entry, ids[2]);
        assert_eq!(restarted.local_appearance_targets[&(object, Some(1))].entry(), ids[1]);
        let other = restored(path.clone(), &other_grid, &jobs);
        assert_eq!(other.entries.len(), 3, "library shared across accounts/grids");
        assert!(
            other.local_targets.is_empty() && other.local_appearance_targets.is_empty(),
            "bindings isolated by grid"
        );
        // Deleting a library row removes saved bindings in every profile.
        restarted.entries.retain(|entry| entry.id != ids[2]);
        restarted.flush_local_registry();
        let restarted = restored(path.clone(), &account, &jobs);
        assert_eq!(restarted.entries.len(), 2);
        assert!(restarted.local_targets.is_empty());
        assert!(files.0.join("mesh-demo.dae").is_file(), "original source is untouched");
    }
    #[test]
    fn missing_sources_remain_saved_and_can_recover_with_their_original_id_and_settings() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let mut imports = restored(path.clone(), "account", &jobs);
        let mut entry = files.entry("mesh-demo.dae");
        entry.local_published = true;
        let Some(Source::Mesh(draft)) = &mut entry.source else {
            panic!("mesh")
        };
        draft.options.scale = 0.4;
        let id = entry.id;
        imports.entries.push(entry);
        imports.flush_local_registry();
        std::fs::remove_file(files.0.join("mesh-demo.dae")).expect("remove test fixture");
        let mut missing = restored(path.clone(), "account", &jobs);
        assert_eq!(missing.entries.len(), 1);
        assert!(matches!(missing.entries[0].status, Status::Error(_)));
        assert!(missing.entries[0].source.is_none());
        missing.flush_local_registry();
        assert_eq!(read_registry(&path).expect("saved missing row").assets.len(), 1);
        std::fs::write(files.0.join("mesh-demo.dae"), demo::DAE).expect("source returns");
        let recovered = restored(path, "account", &jobs);
        assert_eq!(recovered.entries[0].id, id);
        assert!(recovered.entries[0].local_restore_active);
        let Some(Source::Mesh(draft)) = &recovered.entries[0].source else {
            panic!("mesh recovered")
        };
        assert_eq!(draft.options.scale, 0.4);
    }
    #[test]
    fn unreadable_registry_is_never_overwritten_and_profile_keys_contain_no_grid_url() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let original = b"{unsupported format";
        std::fs::write(&path, original).expect("registry");
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let mut imports = restored(path.clone(), "account", &jobs);
        imports.entries.push(files.entry("texture-demo.png"));
        imports.flush_local_registry();
        assert_eq!(std::fs::read(path).expect("original registry"), original);
        let key = profile(Uuid::from_u128(10), "https://grid.example/login");
        assert!(!key.contains("https") && !key.contains("grid.example"));
        assert_ne!(key, profile(Uuid::from_u128(11), "https://grid.example/login"));
    }
}
