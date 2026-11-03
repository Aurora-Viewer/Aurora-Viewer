//! Remembered local previews. The Legacy Viewer's local bitmap/material/mesh lists
//! inspire the references and live reload; retaining the library and client-only
//! object bindings across sessions is an intentional Aurora improvement.
//! Only metadata is saved in the private profile, never source pixels/geometry.

use super::*;
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FILE: &str = "local_previews.json";
const MAX_REGISTRY: u64 = 16 * 1024 * 1024;
const APPEARANCE_LIFETIME: u64 = 7 * 24 * 60 * 60;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

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
    #[serde(default)]
    appearance_expirations: Vec<(Uuid, Option<u8>, u64)>,
}
impl Profile {
    fn expire_appearances(&mut self, now: u64) {
        let mut deadlines: HashMap<_, _> = self
            .appearance_expirations
            .drain(..)
            .map(|(object, face, deadline)| ((object, face), deadline))
            .collect();
        // Old registries have no timestamps: grant one full lifetime, then
        // persist the deadline. Reconnects and source reloads never renew it.
        self.appearances
            .retain(|(object, face, _)| *deadlines.entry((*object, *face)).or_insert(now.saturating_add(APPEARANCE_LIFETIME)) > now);
        self.appearance_expirations = self
            .appearances
            .iter()
            .map(|(object, face, _)| (*object, *face, deadlines[&(*object, *face)]))
            .collect();
    }
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
    appearance_expirations: HashMap<(Uuid, Option<u8>), u64>,
    next_expiration: u64,
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
            let restored = read_registry(&read_path).map(|mut registry| {
                let now = unix_now();
                for profile in registry.profiles.values_mut() {
                    profile.expire_appearances(now);
                }
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
                    self.persistence.appearance_expirations.extend(
                        profile
                            .appearance_expirations
                            .iter()
                            .map(|(object, face, deadline)| ((*object, *face), *deadline)),
                    );
                    self.persistence.next_expiration = 0;
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
        self.save_local_registry_at(force, unix_now());
    }
    fn save_local_registry_at(&mut self, force: bool, now: u64) {
        if !self.persistence.ready || (!force && self.persistence.next.is_some_and(|next| next > Instant::now())) {
            return;
        }
        self.persistence.next = Some(Instant::now() + Duration::from_millis(500));
        self.persistence
            .appearance_expirations
            .retain(|key, _| self.local_appearance_targets.contains_key(key));
        for key in self.local_appearance_targets.keys() {
            if let std::collections::hash_map::Entry::Vacant(entry) = self.persistence.appearance_expirations.entry(*key) {
                entry.insert(now.saturating_add(APPEARANCE_LIFETIME));
                self.persistence.next_expiration = 0;
            }
        }
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
                self.persistence.appearance_expirations[&(*object, *face)],
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
        let appearance_expirations = appearances
            .iter()
            .map(|(object, face, _)| (*object, *face, self.persistence.appearance_expirations[&(*object, *face)]))
            .collect();
        self.persistence.registry.assets = assets;
        self.persistence.registry.profiles.insert(
            self.persistence.profile.clone(),
            Profile {
                meshes,
                appearances,
                appearance_expirations,
            },
        );
        for profile in self.persistence.registry.profiles.values_mut() {
            profile.meshes.retain(|(_, target)| ids.contains(&target.entry));
            profile.appearances.retain(|(_, _, appearance)| ids.contains(&appearance.entry()));
            profile.expire_appearances(now);
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

    /// A new application renews only this object's addressed face. Library
    /// edits, GPU publication and reconnect restoration leave deadlines alone.
    pub fn set_local_appearance_target(
        &mut self,
        object: Uuid,
        face: Option<u8>,
        appearance: Option<crate::world::objects::LocalAppearance>,
    ) {
        let key = (object, face);
        if let Some(appearance) = appearance {
            self.local_appearance_targets.insert(key, appearance);
            self.persistence
                .appearance_expirations
                .insert(key, unix_now().saturating_add(APPEARANCE_LIFETIME));
            self.persistence.next_expiration = 0;
        } else {
            self.local_appearance_targets.remove(&key);
            self.persistence.appearance_expirations.remove(&key);
        }
    }

    pub fn expire_local_appearances(&mut self, objects: &mut crate::world::objects::ObjectStore) -> HashSet<(Uuid, Option<u8>)> {
        self.expire_local_appearances_at(objects, unix_now())
    }
    pub fn sync_local_appearance_changes(
        &mut self,
        changes: impl IntoIterator<Item = (Uuid, Option<u8>, Option<crate::world::objects::LocalAppearance>)>,
    ) {
        let mut split = HashMap::new();
        for (object, face, appearance) in changes {
            // The picker splits a whole-object application into equivalent
            // face links before editing one face. Preserve the other faces'
            // original deadlines instead of treating that split as new use.
            if face.is_none()
                && appearance.is_none()
                && let Some(original) = self.local_appearance_targets.get(&(object, None))
                && let Some(deadline) = self.persistence.appearance_expirations.get(&(object, None))
            {
                split.insert(object, (*original, *deadline));
            }
            self.set_local_appearance_target(object, face, appearance);
            if let Some((original, deadline)) = split.get(&object)
                && appearance == Some(*original)
            {
                self.persistence.appearance_expirations.insert((object, face), *deadline);
            }
        }
    }
    fn expire_local_appearances_at(&mut self, objects: &mut crate::world::objects::ObjectStore, now: u64) -> HashSet<(Uuid, Option<u8>)> {
        if self.persistence.appearance_expirations.is_empty() || now < self.persistence.next_expiration {
            return HashSet::new();
        }
        let mut expired = HashSet::new();
        self.persistence.appearance_expirations.retain(|key, deadline| {
            if *deadline <= now {
                expired.insert(*key);
                false
            } else {
                true
            }
        });
        self.persistence.next_expiration = self.persistence.appearance_expirations.values().copied().min().unwrap_or(u64::MAX);
        for (id, face) in &expired {
            self.local_appearance_targets.remove(&(*id, *face));
            if let Some(index) = objects.index_of_uuid(id)
                && let Some(object) = objects.get_mut(index)
                && object.render.local_appearance.remove(face).is_some()
            {
                object.material_dirty = true;
                object.render.needs_records = true;
            }
        }
        expired
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
    fn legacy_appearance_bindings_get_seven_days_once_and_expire_at_the_exact_deadline() {
        let mut profile: Profile = serde_json::from_value(serde_json::json!({
            "meshes": [],
            "appearances": [[Uuid::from_u128(1), null, {"Texture": Uuid::from_u128(2)}]]
        }))
        .expect("old registry without deadlines");
        profile.expire_appearances(100);
        let deadline = 100 + APPEARANCE_LIFETIME;
        assert_eq!(profile.appearance_expirations, vec![(Uuid::from_u128(1), None, deadline)]);
        let mut profile: Profile = serde_json::from_slice(&serde_json::to_vec(&profile).expect("save")).expect("restart");
        profile.expire_appearances(deadline - 1);
        assert_eq!(profile.appearances.len(), 1);
        assert_eq!(
            profile.appearance_expirations[0].2, deadline,
            "restoration never renews the lifetime"
        );
        profile.expire_appearances(deadline);
        assert!(profile.appearances.is_empty());
        assert!(profile.appearance_expirations.is_empty());
    }

    #[test]
    fn expiration_clears_loaded_and_absent_bindings_but_keeps_newer_faces_library_and_meshes() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let mut imports = restored(path.clone(), "account", &jobs);
        let mut texture = files.entry("texture-demo.png");
        let mut material = files.entry("material-demo.gltf");
        let mesh = files.entry("mesh-demo.dae");
        texture.local_published = true;
        material.local_published = true;
        let ids = [texture.id, material.id, mesh.id];
        imports.entries = vec![texture, material, mesh];
        let mut objects = crate::world::objects::ObjectStore::default();
        let mut update = crate::demo::action_avatar(false);
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let object = update.full_id;
        let absent = Uuid::from_u128(999);
        let index = objects.upsert(0, update);
        let mut parked_update = crate::demo::action_avatar(false);
        parked_update.full_id = absent;
        parked_update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        objects.upsert(1, parked_update);
        imports.target_local(ids[0], object, &[], false);
        imports.target_local(ids[1], object, &[1], false);
        imports.target_local(ids[1], absent, &[3], false);
        imports.target_local(ids[2], object, &[], true);
        imports.apply_appearance_targets(&mut objects);
        let mut parked = objects.park_region(1, &[]);
        assert_eq!(parked.len(), 1);
        let before = objects.get(index).expect("object").te.clone();
        let now = unix_now();
        imports.persistence.appearance_expirations.insert((object, None), now);
        imports.persistence.appearance_expirations.insert((absent, Some(3)), now);
        assert!(imports.expire_local_appearances_at(&mut objects, now - 1).is_empty());
        assert_eq!(
            imports.expire_local_appearances_at(&mut objects, now),
            HashSet::from([(object, None), (absent, Some(3))])
        );
        assert_eq!(imports.local_appearance_targets.len(), 1);
        let remaining = objects.get(index).expect("object");
        assert_eq!(remaining.local_appearance(0), None);
        assert_eq!(
            remaining.local_appearance(1),
            Some(crate::world::objects::LocalAppearance::Material(ids[1]))
        );
        assert!(remaining.material_dirty && remaining.render.needs_records);
        assert!(Arc::ptr_eq(
            remaining.te.as_ref().expect("server faces"),
            before.as_ref().expect("server faces")
        ));
        assert_eq!(imports.local_targets[&object].entry, ids[2]);
        let restored_index = objects.restore(parked.remove(0)).expect("returning object");
        imports.apply_appearance_targets(&mut objects);
        assert!(
            objects
                .get(restored_index)
                .expect("returning object")
                .render
                .local_appearance
                .is_empty()
        );
        imports.flush_local_registry();
        let restarted = restored(path, "account", &jobs);
        assert_eq!(restarted.local_appearance_targets.len(), 1);
        assert_eq!(restarted.entries.len(), 3);
        assert_eq!(restarted.local_targets[&object].entry, ids[2]);
        assert!(files.0.join("texture-demo.png").is_file());
        assert!(files.0.join("material-demo.gltf").is_file());
    }

    #[test]
    fn splitting_a_whole_object_binding_keeps_unedited_faces_original_deadlines() {
        use crate::world::objects::LocalAppearance;
        let mut imports = Imports::default();
        let object = Uuid::from_u128(101);
        let original = LocalAppearance::Texture(Uuid::from_u128(102));
        let replacement = LocalAppearance::Material(Uuid::from_u128(103));
        imports.set_local_appearance_target(object, None, Some(original));
        imports.persistence.appearance_expirations.insert((object, None), 1000);
        imports.sync_local_appearance_changes([
            (object, None, None),
            (object, Some(0), Some(original)),
            (object, Some(1), Some(original)),
            (object, Some(1), Some(replacement)),
        ]);
        assert!(!imports.persistence.appearance_expirations.contains_key(&(object, None)));
        assert_eq!(imports.persistence.appearance_expirations[&(object, Some(0))], 1000);
        assert!(imports.persistence.appearance_expirations[&(object, Some(1))] >= unix_now() + APPEARANCE_LIFETIME - 1);
        assert_eq!(imports.local_appearance_targets[&(object, Some(1))], replacement);
    }

    #[test]
    fn saved_deadlines_survive_reconnect_and_library_edits_and_expired_profiles_are_pruned() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let mut imports = restored(path.clone(), "account", &jobs);
        let texture = files.entry("texture-demo.png");
        let id = texture.id;
        imports.entries.push(texture);
        let object = Uuid::from_u128(100);
        imports.target_local(id, object, &[1, 2], false);
        let deadline = unix_now() + 100;
        imports.persistence.appearance_expirations.insert((object, Some(1)), deadline);
        imports.flush_local_registry();
        let mut restarted = restored(path.clone(), "account", &jobs);
        assert_eq!(restarted.persistence.appearance_expirations[&(object, Some(1))], deadline);
        restarted.entries[0].name = "Edited library row".into();
        restarted.save_local_registry_at(true, deadline - 1);
        assert_eq!(
            restarted.persistence.registry.profiles["account"].appearance_expirations[0].2,
            deadline
        );
        restarted.target_local(id, object, &[2], false);
        assert_eq!(restarted.persistence.appearance_expirations[&(object, Some(1))], deadline);
        assert!(restarted.persistence.appearance_expirations[&(object, Some(2))] > deadline);
        restarted
            .persistence
            .registry
            .profiles
            .insert("other".into(), restarted.persistence.registry.profiles["account"].clone());
        restarted.save_local_registry_at(true, deadline);
        assert_eq!(restarted.persistence.registry.profiles["account"].appearances.len(), 1);
        assert_eq!(restarted.persistence.registry.profiles["other"].appearances.len(), 1);
        restarted.persistence.finish();
        // Simulate another deadline passing while the viewer is closed.
        let mut saved = read_registry(&path).expect("saved registry");
        let profile = saved.profiles.get_mut("account").expect("account");
        profile
            .appearances
            .push((object, Some(3), crate::world::objects::LocalAppearance::Texture(id)));
        profile.appearance_expirations.push((object, Some(3), unix_now() - 1));
        write_registry(&path, &serde_json::to_vec(&saved).expect("registry bytes")).expect("synthetic expired registry");
        let restarted = restored(path, "account", &jobs);
        assert!(!restarted.local_appearance_targets.contains_key(&(object, Some(1))));
        assert!(restarted.local_appearance_targets.contains_key(&(object, Some(2))));
        assert!(
            !restarted.local_appearance_targets.contains_key(&(object, Some(3))),
            "expired before restoration"
        );
        assert_eq!(restarted.entries[0].name, "Edited library row");
    }

    #[test]
    fn global_appearance_removal_survives_restart_and_keeps_library_meshes_and_other_profiles() {
        let files = Files::new();
        let path = files.0.join(FILE);
        let (jobs, _) = crate::scene::jobs::Jobs::new();
        let mut imports = restored(path.clone(), "first", &jobs);
        let mut texture = files.entry("texture-demo.png");
        let mut material = files.entry("material-demo.gltf");
        let mesh = files.entry("mesh-demo.dae");
        texture.local_published = true;
        material.local_published = true;
        let ids = [texture.id, material.id, mesh.id];
        imports.entries = vec![texture, material, mesh];
        let mut objects = crate::world::objects::ObjectStore::default();
        let mut update = crate::demo::action_avatar(false);
        update.pcode = aurora_prim::params::LL_PCODE_VOLUME;
        let object = update.full_id;
        let index = objects.upsert(0, update);
        imports.target_local(ids[0], object, &[], false);
        imports.target_local(ids[1], Uuid::from_u128(555), &[3], false);
        imports.target_local(ids[2], object, &[], true);
        imports.apply_appearance_targets(&mut objects);
        assert_eq!(imports.appearance_status(&objects).remembered_objects, 2);
        imports.flush_local_registry();

        let mut other = restored(path.clone(), "second", &jobs);
        other.target_local(ids[0], object, &[1], false);
        other.flush_local_registry();
        let mut imports = restored(path.clone(), "first", &jobs);
        imports.clear_all_appearance_targets(&mut objects);
        assert!(objects.get(index).expect("object").render.local_appearance.is_empty());
        assert_eq!(imports.entries.len(), 3);
        assert_eq!(imports.local_targets[&object].entry, ids[2]);
        imports.flush_local_registry();
        let restarted = restored(path.clone(), "first", &jobs);
        assert!(restarted.local_appearance_targets.is_empty());
        assert_eq!(restarted.entries.len(), 3);
        assert_eq!(restarted.local_targets[&object].entry, ids[2]);
        let other = restored(path, "second", &jobs);
        assert_eq!(other.local_appearance_targets.len(), 1);
        assert!(files.0.join("texture-demo.png").is_file());
        assert!(files.0.join("material-demo.gltf").is_file());
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
