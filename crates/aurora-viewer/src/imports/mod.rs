//! Asset import controller. Firestorm reference: llviewermenufile.cpp,
//! llfloaterimagepreview.cpp, llfloaterbvhpreview.cpp, llmaterialeditor.cpp.
//! Pure codecs live in aurora-assets; filesystem work and conversion are
//! scheduled on the scene's low-priority workers, never the frame thread.

pub mod animation;
pub mod demo;
pub mod files;
pub mod local;
pub mod maps;
pub mod mesh;
pub mod preview;
#[cfg(test)]
mod tests;
pub mod ui;

use aurora_assets::{Animation, DecodedImage, PbrMaterial, import_sound::Sound, import_texture};
use aurora_net::inventory::upload::{Error, Kind, Permissions, Prices, Receipt, Request};
use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Texture,
    Sound,
    Animation,
    Material,
    Mesh,
    Bulk,
    Local,
}
impl Choice {
    pub fn accepts(self, entry: &Entry) -> bool {
        if self == Self::Local {
            return entry.local;
        }
        if entry.local {
            return false;
        }
        match entry.source.as_ref().map(Source::kind) {
            Some(kind) => {
                (self == Choice::Bulk && kind != Kind::Mesh && !matches!(entry.source, Some(Source::Animation { bvh: Some(_), .. })))
                    || matches!(
                        (self, kind),
                        (Choice::Texture, Kind::Texture)
                            | (Choice::Sound, Kind::Sound)
                            | (Choice::Animation, Kind::Animation)
                            | (Choice::Material, Kind::Material)
                            | (Choice::Mesh, Kind::Mesh)
                    )
            }
            None => true,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Texture => aurora_i18n::tr!("import-texture"),
            Self::Sound => aurora_i18n::tr!("import-sound"),
            Self::Animation => aurora_i18n::tr!("import-animation"),
            Self::Material => aurora_i18n::tr!("import-material"),
            Self::Mesh => aurora_i18n::tr!("import-mesh"),
            Self::Bulk => aurora_i18n::tr!("import-bulk"),
            Self::Local => aurora_i18n::tr!("import-local"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub max_width: u32,
    pub max_height: u32,
    pub lossless_small: bool,
    pub next_modify: bool,
    pub next_copy: bool,
    pub next_transfer: bool,
    pub share_group: bool,
    pub everyone_copy: bool,
    pub show_payment: bool,
    pub preview_on_avatar: bool,
    pub mesh_auto_weights: bool,
    pub mesh_auto_preview_weights: bool,
    pub mesh_reliable_default: bool,
    pub mesh_suffixes: [String; 5],
    pub mesh_physics_preset: String,
    pub local_assume_meters: bool,
    pub local_mesh_auto_reload: bool,
    pub local_reload_seconds: f32,
    pub preview_background: Option<[f32; 4]>,
    pub preview_edges: Option<[f32; 4]>,
    pub preview_physics_fill: Option<[f32; 4]>,
    pub preview_physics_edges: Option<[f32; 4]>,
    pub preview_degenerate_fill: Option<[f32; 4]>,
    pub preview_degenerate_edges: Option<[f32; 4]>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            max_width: 1024,
            max_height: 1024,
            lossless_small: true,
            next_modify: false,
            next_copy: false,
            next_transfer: true,
            share_group: false,
            everyone_copy: false,
            show_payment: true,
            preview_on_avatar: false,
            mesh_auto_weights: true,
            mesh_auto_preview_weights: true,
            mesh_reliable_default: false,
            mesh_suffixes: ["LOD0".into(), "LOD1".into(), "LOD2".into(), String::new(), "PHYS".into()],
            mesh_physics_preset: String::new(),
            local_assume_meters: false,
            local_mesh_auto_reload: false,
            local_reload_seconds: 3.0,
            preview_background: None,
            preview_edges: None,
            preview_physics_fill: None,
            preview_physics_edges: None,
            preview_degenerate_fill: None,
            preview_degenerate_edges: None,
        }
    }
}
impl Preferences {
    pub fn texture(&self) -> import_texture::Options {
        import_texture::Options {
            max_width: self.max_width,
            max_height: self.max_height,
            lossless_small: self.lossless_small,
            strip_alpha: true,
        }
    }
    pub fn permissions(&self) -> Permissions {
        use aurora_net::build::perm;
        Permissions {
            next_owner: perm::MOVE
                | if self.next_modify { perm::MODIFY } else { 0 }
                | if self.next_copy { perm::COPY } else { 0 }
                | if self.next_transfer || !self.next_copy { perm::TRANSFER } else { 0 },
            group: if self.share_group {
                perm::COPY | perm::MOVE | perm::MODIFY
            } else {
                0
            },
            everyone: if self.everyone_copy { perm::COPY } else { 0 },
        }
    }
}

#[derive(Clone)]
pub struct LocalImage {
    pub key: Uuid,
    pub name: String,
    pub pixels: Arc<DecodedImage>,
    pub asset: Option<Uuid>,
    pub empty_alpha: bool,
}
impl LocalImage {
    fn upload_key(&self, options: import_texture::Options) -> Uuid {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(self.key.as_bytes());
        let (w, h) = import_texture::dimensions(self.pixels.width, self.pixels.height, options);
        hash.update(w.to_le_bytes());
        hash.update(h.to_le_bytes());
        hash.update([
            u8::from(options.lossless_small && w * h <= 128 * 128),
            u8::from(options.strip_alpha && self.empty_alpha),
        ]);
        let digest = hash.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Uuid::from_bytes(bytes)
    }
}
#[derive(Clone)]
pub enum Source {
    Texture(Arc<DecodedImage>),
    Sound(Arc<Sound>),
    Animation {
        bytes: Arc<Vec<u8>>,
        animation: Arc<Animation>,
        bvh: Option<BvhInfo>,
    },
    Material {
        value: PbrMaterial,
        images: Box<[Option<LocalImage>; 4]>,
    },
    Mesh(Box<mesh::Draft>),
}
#[derive(Clone, Copy)]
pub struct BvhInfo {
    pub frames: usize,
    pub frame_time: f32,
}
impl Source {
    /// Conservative size of retained input; shared maps may be counted twice.
    pub fn retained_bytes(&self) -> usize {
        let model = |model: &aurora_assets::import_mesh::Model| {
            model
                .faces
                .iter()
                .map(|f| f.vertices.len() * std::mem::size_of::<aurora_assets::import_mesh::Vertex>() + f.indices.len() * 4)
                .sum::<usize>()
        };
        match self {
            Self::Texture(image) => image.data.len(),
            Self::Sound(sound) => sound.mono.len() * 4,
            Self::Animation { bytes, animation, .. } => {
                bytes.len()
                    + animation
                        .joints
                        .iter()
                        .map(|j| {
                            j.rot_keys.len() * std::mem::size_of::<aurora_assets::anim::RotKey>()
                                + j.pos_keys.len() * std::mem::size_of::<aurora_assets::anim::PosKey>()
                        })
                        .sum::<usize>()
            }
            Self::Material { images, .. } => images.iter().flatten().map(|i| i.pixels.data.len()).sum(),
            Self::Mesh(draft) => {
                draft.scene.models.iter().map(model).sum::<usize>()
                    + draft.images.values().map(|i| i.pixels.data.len()).sum::<usize>()
                    + draft
                        .lods
                        .iter()
                        .filter_map(|l| l.file.as_ref())
                        .map(|s| s.models.iter().map(model).sum::<usize>())
                        .sum::<usize>()
                    + draft.physics_file.as_ref().map_or(0, |s| s.models.iter().map(model).sum::<usize>())
            }
        }
    }
    pub fn kind(&self) -> Kind {
        match self {
            Self::Texture(_) => Kind::Texture,
            Self::Sound(_) => Kind::Sound,
            Self::Animation { .. } => Kind::Animation,
            Self::Material { .. } => Kind::Material,
            Self::Mesh(_) => Kind::Mesh,
        }
    }
    pub fn dimensions(&self, options: import_texture::Options) -> (u32, u32) {
        match self {
            Self::Texture(image) => import_texture::dimensions(image.width, image.height, options),
            _ => (0, 0),
        }
    }
}

pub struct Entry {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    pub folder: Uuid,
    pub selected: bool,
    pub source: Option<Source>,
    pub options: import_texture::Options,
    pub preview: Option<egui::TextureHandle>,
    pub preview_generation: u64,
    pub model_generation: Option<u64>,
    pub preview_not_before: Option<std::time::Instant>,
    pub preview_pending: bool,
    pub reloading: bool,
    pub preview_job_generation: u64,
    pub preview_error: Option<String>,
    pub model: Option<Arc<aurora_render::preview::Model>>,
    pub view: aurora_render::preview::View,
    pub texture_target: usize,
    pub player: animation::Player,
    pub local: bool,
    pub local_pending: bool,
    pub local_published: bool,
    pub local_generation: u64,
    pub local_failed_generation: Option<u64>,
    pub local_slots: HashMap<Uuid, u32>,
    pub local_mesh_ids: Vec<Uuid>,
    pub local_model: usize,
    pub local_use_scale: bool,
    pub watch: Option<local::Watch>,
    pub material_index: usize,
    pub empty_alpha: bool,
    pub local_live: bool,
    pub status: Status,
}
#[derive(Clone)]
pub enum Status {
    Ready,
    Error(String),
    Preparing,
    Uploading,
    Done(Uuid),
    Cancelled,
}
impl Entry {
    pub fn invalidate_preview(&mut self) -> Option<String> {
        self.preview_generation += 1;
        // Keep the last model on screen while a worker prepares its replacement.
        self.preview_not_before = Some(std::time::Instant::now() + std::time::Duration::from_millis(120));
        self.preview_error.take()
    }

    pub fn new(path: PathBuf, source: Result<Source, String>, options: import_texture::Options) -> Self {
        let (source, status) = match source {
            Ok(source) => (Some(source), Status::Ready),
            Err(error) => (None, Status::Error(error)),
        };
        Self {
            id: Uuid::new_v4(),
            name: path.file_stem().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            path,
            description: String::new(),
            folder: Uuid::nil(),
            selected: source.is_some(),
            empty_alpha: matches!(&source,Some(Source::Texture(image)) if import_texture::empty_alpha(image)),
            source,
            options,
            preview: None,
            preview_generation: 0,
            model_generation: None,
            preview_not_before: None,
            preview_pending: false,
            reloading: false,
            preview_job_generation: 0,
            preview_error: None,
            model: None,
            view: Default::default(),
            texture_target: 0,
            player: Default::default(),
            local: false,
            local_pending: false,
            local_published: false,
            local_generation: 0,
            local_failed_generation: None,
            local_slots: HashMap::new(),
            local_mesh_ids: Vec::new(),
            local_model: 0,
            local_use_scale: false,
            watch: None,
            material_index: 0,
            local_live: true,
            status,
        }
    }
}

pub(super) enum Worker {
    Loaded(Uuid, Vec<Entry>),
    Reloaded(Uuid, Result<Vec<Entry>, String>),
    Encoded(Uuid, Result<Vec<u8>, String>),
    Preview(Uuid, u64, egui::ColorImage),
    Model(Uuid, u64, Arc<aurora_render::preview::Model>, Option<Arc<mesh::Prepared>>),
    PreviewFailed(Uuid, u64, String),
    MeshPrepared(
        Uuid,
        Uuid,
        u64,
        Result<(Arc<mesh::Prepared>, aurora_net::inventory::mesh_upload::Request), String>,
    ),
    MeshFile(
        Uuid,
        usize,
        Result<(Arc<aurora_assets::import_mesh::Scene>, HashMap<usize, LocalImage>, PathBuf), String>,
    ),
    Local(Uuid, u64, Result<local::Prepared, String>),
    Watch(Uuid, local::Stamp),
    Map(Uuid, u64, usize, Result<LocalImage, String>),
    Remote(Uuid, LocalImage),
    Audio(Uuid, aurora_audio::SoundClip),
    Animation(Uuid, Uuid, Arc<crate::scene::anim::BoundAnim>),
}
pub enum Playback {
    Audio(aurora_audio::SoundClip),
}

struct Step {
    key: Uuid,
    entry: Option<Uuid>,
    kind: Kind,
    folder: Uuid,
    name: String,
    description: String,
    source: Source,
    options: import_texture::Options,
    cost: i32,
    permissions: Permissions,
}

pub struct Imports {
    pub open: bool,
    pub choice: Choice,
    pub entries: Vec<Entry>,
    pub active: Option<Uuid>,
    pub message: String,
    pub confirm: bool,
    pub scale_bulk_1k: bool,
    pub pending_files: Option<Uuid>,
    /// One-shot menu intent, consumed after drawing the import window.
    pub choose_files_on_open: bool,
    pub(super) tx: Sender<Worker>,
    rx: Receiver<Worker>,
    queue: VecDeque<Step>,
    current: Option<(Uuid, Step)>,
    uploaded: HashMap<Uuid, Receipt>,
    pub play_request: Option<Uuid>,
    pub playback: Vec<Playback>,
    pub local_uploads: Vec<local::Prepared>,
    pub sound_limit: u32,
    pub map_picker: crate::ui::texture_picker::TexturePicker,
    destination: ui::destination::Picker,
    pub map_target: Option<(Uuid, usize)>,
    pub pending_remote: HashSet<Uuid>,
    pub demo_started: bool,
    pub demo_mode: String,
    pub demo_local_applied: bool,
    pub demo_upload_started: bool,
    pub demo_animation_positioned: bool,
    pub own_avatar_preview: Option<Uuid>,
    pub load_preferences: Preferences,
    cost_overrides: HashMap<(Kind, bool), i32>,
    pub spent: i32,
    pub discarded_quotes: Vec<Uuid>,
    pub local_targets: HashMap<Uuid, local::Target>,
    pub local_appearance_targets: HashMap<(Uuid, Option<u8>), crate::world::objects::LocalAppearance>,
    pub retired_local_slots: Vec<(Uuid, u32)>,
    pub local_rez: Option<local::Rez>,
}
impl Default for Imports {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            open: false,
            choice: Choice::Texture,
            entries: Vec::new(),
            active: None,
            message: String::new(),
            confirm: false,
            scale_bulk_1k: false,
            pending_files: None,
            choose_files_on_open: false,
            tx,
            rx,
            queue: VecDeque::new(),
            current: None,
            uploaded: HashMap::new(),
            play_request: None,
            playback: Vec::new(),
            local_uploads: Vec::new(),
            sound_limit: 30,
            map_picker: Default::default(),
            destination: Default::default(),
            map_target: None,
            pending_remote: HashSet::new(),
            demo_started: false,
            demo_mode: String::new(),
            demo_local_applied: false,
            demo_upload_started: false,
            demo_animation_positioned: false,
            own_avatar_preview: None,
            load_preferences: Default::default(),
            cost_overrides: HashMap::new(),
            spent: 0,
            discarded_quotes: Vec::new(),
            local_targets: HashMap::new(),
            local_appearance_targets: HashMap::new(),
            retired_local_slots: Vec::new(),
            local_rez: None,
        }
    }
}
impl Imports {
    pub fn demo_mesh_request(&self) -> Option<Request> {
        let step = &self.current.as_ref()?.1;
        (step.kind == Kind::Mesh).then(|| Request {
            kind: Kind::Mesh,
            folder: step.folder,
            name: step.name.clone(),
            description: step.description.clone(),
            permissions: step.permissions,
            expected_cost: step.cost,
            data: vec![1],
        })
    }
    pub fn busy(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }
    pub fn open(&mut self, choice: Choice) {
        self.open = true;
        self.choice = choice;
        if self
            .active
            .is_none_or(|id| self.entries.iter().all(|e| e.id != id || !choice.accepts(e)))
        {
            self.active = self.entries.iter().find(|e| choice.accepts(e)).map(|e| e.id);
        }
    }
    pub fn options(&self, entry: &Entry) -> import_texture::Options {
        let mut options = entry.options;
        if self.choice == Choice::Bulk && self.scale_bulk_1k {
            options.max_width = options.max_width.min(1024);
            options.max_height = options.max_height.min(1024);
        }
        options
    }
    fn default_folder(world: &crate::world::World, prefs: &crate::ui::inventory::InventoryPreferences, kind: Kind) -> Uuid {
        use crate::world::inventory::actions;
        let folder = prefs.upload_folders[kind.folder_slot()];
        if actions::destination(&world.inventory, folder, &prefs.protected) {
            return folder;
        }
        actions::system(&world.inventory, kind.asset_type()).unwrap_or(world.inventory.root)
    }
    pub fn total(&self, prices: &Prices) -> Option<i32> {
        if self
            .entries
            .iter()
            .any(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Error(_)))
        {
            return None;
        }
        let mut total: i32 = 0;
        let mut images = HashSet::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready))
        {
            let source = entry.source.as_ref()?;
            let options = self.options(entry);
            total = total.checked_add(match source {
                Source::Mesh(draft) => draft.quote.as_ref()?.cost,
                _ => self.price(prices, source.kind(), source.dimensions(options))?,
            })?;
            if let Source::Material { images: textures, .. } = source {
                for image in textures.iter().flatten().filter(|i| i.asset.is_none()) {
                    let key = image.upload_key(options);
                    if images.insert(key) && !self.uploaded.contains_key(&key) {
                        total = total.checked_add(self.price(
                            prices,
                            Kind::Texture,
                            import_texture::dimensions(image.pixels.width, image.pixels.height, options),
                        )?)?;
                    }
                }
            }
        }
        Some(total)
    }
    pub fn start(
        &mut self,
        world: &crate::world::World,
        prefs: &crate::ui::inventory::InventoryPreferences,
        permissions: Permissions,
        prices: &Prices,
    ) {
        if self.busy() {
            return;
        }
        let Some(total) = self.total(prices) else {
            self.message = aurora_i18n::tr!("import-price-unavailable").into();
            return;
        };
        if total > 0 && world.balance.is_none_or(|balance| balance < total) {
            self.message = aurora_i18n::tr!("import-insufficient-balance").into();
            return;
        }
        if self
            .entries
            .iter()
            .filter(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready))
            .any(|entry| {
                entry.name.trim().is_empty()
                    || entry.name.len() > 63
                    || entry.description.len() > 127
                    || entry.name.contains('\0')
                    || entry.description.contains('\0')
                    || !crate::world::inventory::actions::destination(&world.inventory, entry.folder, &prefs.protected)
            })
        {
            self.message = aurora_i18n::tr!("import-invalid-request").into();
            return;
        }
        let mut images = HashSet::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| self.choice.accepts(e) && e.selected && matches!(e.status, Status::Ready))
        {
            let Some(source) = &entry.source else {
                continue;
            };
            let options = self.options(entry);
            if let Source::Material { images: textures, .. } = source {
                for image in textures.iter().flatten().filter(|i| i.asset.is_none()) {
                    let key = image.upload_key(options);
                    if images.insert(key) && !self.uploaded.contains_key(&key) {
                        self.queue.push_back(Step {
                            key,
                            entry: None,
                            kind: Kind::Texture,
                            folder: Self::default_folder(world, prefs, Kind::Texture),
                            name: image.name.clone(),
                            description: String::new(),
                            source: Source::Texture(image.pixels.clone()),
                            options,
                            cost: self
                                .price(
                                    prices,
                                    Kind::Texture,
                                    import_texture::dimensions(image.pixels.width, image.pixels.height, options),
                                )
                                .unwrap_or(0),
                            permissions,
                        });
                    }
                }
            }
            self.queue.push_back(Step {
                key: entry.id,
                entry: Some(entry.id),
                kind: source.kind(),
                folder: entry.folder,
                name: entry.name.clone(),
                description: entry.description.clone(),
                source: source.clone(),
                options,
                cost: match source {
                    Source::Mesh(draft) => draft.quote.as_ref().map(|q| q.cost).unwrap_or(0),
                    _ => self.price(prices, source.kind(), source.dimensions(options)).unwrap_or(0),
                },
                permissions,
            });
        }
        for entry in &mut self.entries {
            if self.choice.accepts(entry) && entry.selected && matches!(entry.status, Status::Ready) {
                entry.status = Status::Preparing;
            }
        }
        self.confirm = false;
        self.spent = 0;
        self.message.clear();
    }
    pub fn cancel_remaining(&mut self) {
        self.queue.clear();
        for entry in &mut self.entries {
            if matches!(entry.status, Status::Preparing) {
                entry.status = Status::Cancelled;
            }
        }
        // An in-flight paid upload runs to its confirmed result. Cancelling a
        // future step never discards the current server acknowledgement.
    }
    pub fn restore_draft(&mut self, id: Uuid) {
        if self.busy() {
            return;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.source.is_some() && matches!(e.status, Status::Error(_)))
        {
            if let Some(Source::Mesh(draft)) = &mut entry.source {
                draft.invalidate_quote();
            }
            entry.status = Status::Ready;
            entry.preview_error = None;
            entry.preview_pending = false;
            entry.preview_generation += 1;
        }
        // Keep the outcome/verification warning visible. Restoring editing
        // never sends a request; a new paid upload requires confirmation.
    }
    pub fn uploaded(&mut self, request: Uuid, result: Result<Receipt, Error>) -> Option<aurora_net::inventory::InvItem> {
        if self.current.as_ref().is_none_or(|(id, _)| *id != request) {
            return None;
        }
        let (_, step) = self.current.take()?;
        match result {
            Ok(receipt) => {
                self.spent = self.spent.saturating_add(receipt.cost);
                let item = receipt.item.clone();
                if let Some(entry) = step.entry.and_then(|id| self.entries.iter_mut().find(|e| e.id == id)) {
                    entry.status = Status::Done(item.id);
                }
                self.uploaded.insert(step.key, receipt);
                Some(item)
            }
            Err(error) => {
                if let Error::PriceChanged(Some(cost)) = error {
                    let dimensions = step.source.dimensions(step.options);
                    self.cost_overrides
                        .insert((step.kind, u64::from(dimensions.0) * u64::from(dimensions.1) > 1024 * 1024), cost);
                }
                self.fail(step.entry, ui::error(error));
                None
            }
        }
    }
    fn price(&self, prices: &Prices, kind: Kind, dimensions: (u32, u32)) -> Option<i32> {
        self.cost_overrides
            .get(&(kind, u64::from(dimensions.0) * u64::from(dimensions.1) > 1024 * 1024))
            .copied()
            .or_else(|| prices.cost(kind, dimensions))
    }
    fn fail(&mut self, entry: Option<Uuid>, reason: String) {
        if let Some(entry) = entry.and_then(|id| self.entries.iter_mut().find(|e| e.id == id)) {
            entry.status = Status::Error(reason.clone());
        }
        self.message = reason;
        self.cancel_remaining();
    }
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        jobs: &crate::scene::jobs::Jobs,
        world: &crate::world::World,
        prefs: &crate::ui::inventory::InventoryPreferences,
    ) -> Vec<aurora_net::NetCommand> {
        let mut commands = Vec::new();
        while let Ok(worker) = self.rx.try_recv() {
            match worker {
                Worker::Loaded(request, mut entries) if self.pending_files == Some(request) => {
                    self.pending_files = None;
                    for entry in &mut entries {
                        if let Some(source) = &entry.source {
                            entry.folder = Self::default_folder(world, prefs, source.kind());
                        }
                    }
                    if self.active.is_none() {
                        self.active = entries.iter().find(|e| self.choice.accepts(e)).map(|e| e.id);
                    }
                    self.entries.extend(entries);
                }
                Worker::Reloaded(id, result) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                        entry.reloading = false;
                        match result {
                            Ok(mut loaded) if entry.material_index < loaded.len() => {
                                let mut updated = loaded.remove(entry.material_index);
                                if let Some(Source::Mesh(draft)) = &mut entry.source {
                                    draft.invalidate_quote();
                                    self.discarded_quotes.append(&mut draft.discarded_quotes);
                                }
                                jobs.discard(entry.source.take());
                                entry.source = updated.source.take();
                                entry.status = Status::Ready;
                                entry.preview = None;
                                entry.preview_generation += 1;
                                entry.preview_pending = false;
                                entry.preview_error = None;
                                entry.model = None;
                                entry.player = Default::default();
                                entry.empty_alpha = updated.empty_alpha;
                                entry.watch = updated.watch.take();
                            }
                            Ok(_) => entry.status = Status::Error(aurora_i18n::tr!("import-reload-material-count").into()),
                            Err(error) => entry.status = Status::Error(error),
                        }
                    }
                }
                Worker::Encoded(request, result) if self.current.as_ref().is_some_and(|(id, _)| *id == request) => match result {
                    Ok(data) => {
                        let step = &self.current.as_ref().expect("matched request").1;
                        if let Some(entry) = step.entry.and_then(|id| self.entries.iter_mut().find(|e| e.id == id)) {
                            entry.status = Status::Uploading;
                        }
                        commands.push(aurora_net::NetCommand::UploadAsset {
                            request,
                            upload: Request {
                                kind: step.kind,
                                folder: step.folder,
                                name: step.name.clone(),
                                description: step.description.clone(),
                                expected_cost: step.cost,
                                permissions: step.permissions,
                                data,
                            },
                        });
                    }
                    Err(error) => {
                        let step = self.current.take().expect("matched request").1;
                        self.fail(step.entry, error);
                    }
                },
                Worker::Preview(id, generation, pixels) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.preview_generation == generation) {
                        entry.preview_pending = false;
                        entry.preview = Some(ctx.load_texture(format!("import_{id}"), pixels, egui::TextureOptions::LINEAR));
                    }
                }
                Worker::Model(id, generation, model, prepared) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.preview_generation == generation) {
                        entry.preview_pending = false;
                        entry.model = Some(model);
                        entry.model_generation = Some(generation);
                        entry.preview_not_before = None;
                        if let Some(Source::Mesh(draft)) = &mut entry.source
                            && draft.prepared.is_none()
                        {
                            draft.prepared = prepared;
                        }
                    }
                }
                Worker::PreviewFailed(id, generation, error) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.preview_generation == generation) {
                        entry.preview_pending = false;
                        entry.preview_not_before = None;
                        entry.preview_error = Some(error.clone());
                        self.message = error;
                    }
                }
                Worker::Local(id, generation, result) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                        if generation == entry.preview_generation {
                            match result {
                                Ok(prepared) => self.local_uploads.push(prepared),
                                Err(error) => {
                                    entry.local_pending = false;
                                    entry.local_failed_generation = Some(generation);
                                    self.message = error;
                                }
                            }
                        } else {
                            entry.local_pending = false;
                        }
                    }
                }
                Worker::Watch(id, stamp) => {
                    let mut changed = false;
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
                        && let Some(watch) = &mut entry.watch
                    {
                        watch.pending = false;
                        changed = stamp != watch.stamp;
                        watch.stamp = stamp;
                    }
                    if changed {
                        self.reload(id, jobs, ctx);
                    }
                }
                Worker::Remote(asset, image) => self.cached_map(asset, image),
                Worker::Map(id, generation, slot, result) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.preview_generation == generation)
                        && let Some(Source::Material { value, images }) = &mut entry.source
                    {
                        match result {
                            Ok(image) => {
                                images[slot] = Some(image);
                                maps::set(value, slot, None);
                                entry.preview_generation += 1;
                                entry.model = None;
                                entry.preview_error = None;
                            }
                            Err(error) => self.message = error,
                        }
                    }
                }
                Worker::MeshPrepared(id, request, generation, result) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id && e.preview_generation == generation)
                        && let Some(Source::Mesh(draft)) = &mut entry.source
                        && draft.pending == Some((request, generation))
                    {
                        match result {
                            Ok((prepared, upload)) => {
                                draft.prepared = Some(prepared);
                                entry.model = None;
                                commands.push(aurora_net::NetCommand::QuoteMesh { request, upload });
                            }
                            Err(error) => {
                                draft.pending = None;
                                self.message = error;
                            }
                        }
                    }
                }
                Worker::MeshFile(id, level, result) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
                        && let Some(Source::Mesh(draft)) = &mut entry.source
                    {
                        match result {
                            Ok((scene, images, path)) => {
                                draft.invalidate();
                                if level == 3 {
                                    let triangles = scene.models.iter().map(aurora_assets::import_mesh::Model::triangles).sum::<usize>();
                                    for (i, lod) in draft.lods.iter_mut().enumerate() {
                                        if lod.file.as_ref().is_some_and(|file| file.models.len() != scene.models.len()) {
                                            lod.file = None;
                                            lod.path = None;
                                            lod.source = mesh::LodSource::Auto;
                                        }
                                        lod.triangles = (triangles / 4usize.pow((3 - i) as u32)).max(1);
                                    }
                                    draft.lods[3].file = None;
                                    draft.lods[3].path = Some(path.clone());
                                    draft.lods[3].source = mesh::LodSource::File;
                                    draft.scene = scene;
                                    draft.images = images;
                                    entry.path = path;
                                } else if level < 4 {
                                    draft.lods[level].file = Some(scene);
                                    draft.lods[level].path = Some(path);
                                    draft.lods[level].source = mesh::LodSource::File;
                                } else {
                                    draft.physics_file = Some(scene);
                                    draft.physics_path = Some(path);
                                    draft.options.physics_source = if level == 8 { 8 } else { 5 };
                                }
                                if let Some(error) = entry.invalidate_preview()
                                    && self.message == error
                                {
                                    self.message.clear();
                                }
                                if entry.local {
                                    let live = entry.local_live;
                                    files::localize(std::slice::from_mut(entry), &self.load_preferences);
                                    entry.local_live = live;
                                }
                            }
                            Err(error) => self.message = error,
                        }
                    }
                }
                Worker::Audio(request, clip) if self.play_request == Some(request) => self.playback.push(Playback::Audio(clip)),
                Worker::Animation(request, id, animation) if self.play_request == Some(request) => {
                    if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
                        entry.player.request = Some(request);
                        entry.player.bound = Some(animation);
                        entry.player.seek(0.0);
                        entry.player.playing = true;
                    }
                }
                _ => {}
            }
        }
        if self.current.is_none()
            && let Some(mut step) = self.queue.pop_front()
        {
            if let Source::Mesh(draft) = &step.source {
                if let Some(quote) = &draft.quote {
                    let request = Uuid::new_v4();
                    commands.push(aurora_net::NetCommand::UploadMesh {
                        request,
                        token: quote.token,
                    });
                    if let Some(entry) = step.entry.and_then(|id| self.entries.iter_mut().find(|e| e.id == id)) {
                        entry.status = Status::Uploading;
                    }
                    self.current = Some((request, step));
                } else {
                    self.fail(step.entry, aurora_i18n::tr!("import-price-unavailable").into());
                }
                return commands;
            }
            if let Source::Material { value, images } = &mut step.source {
                let mut ids = [
                    value.base_color_texture,
                    value.normal_texture,
                    value.metallic_roughness_texture,
                    value.emissive_texture,
                ];
                for (slot, image) in images.iter().enumerate() {
                    if let Some(image) = image
                        && image.asset.is_none()
                    {
                        let Some(receipt) = self.uploaded.get(&image.upload_key(step.options)) else {
                            self.fail(step.entry, aurora_i18n::tr!("import-unverified").into());
                            return commands;
                        };
                        ids[slot] = Some(receipt.item.asset_id);
                    }
                }
                value.base_color_texture = ids[0];
                value.normal_texture = ids[1];
                value.metallic_roughness_texture = ids[2];
                value.emissive_texture = ids[3];
            }
            let source = step.source.clone();
            let options = step.options;
            let request = Uuid::new_v4();
            let tx = self.tx.clone();
            let wake = ctx.clone();
            jobs.execute(move || {
                let result = files::encode(&source, options);
                let _ = tx.send(Worker::Encoded(request, result));
                wake.request_repaint();
            });
            self.current = Some((request, step));
        }
        commands
    }
    pub fn request_preview(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context) {
        let Some(entry) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        let Some(Source::Texture(source)) = &entry.source else {
            return;
        };
        let source = source.clone();
        let options = self.options(entry);
        let tx = self.tx.clone();
        let wake = ctx.clone();
        let generation = entry.preview_generation;
        let entry = self.entries.iter_mut().find(|e| e.id == id).expect("entry exists");
        entry.preview_pending = true;
        entry.preview_job_generation = generation;
        entry.preview_error = None;
        jobs.execute(move || match import_texture::prepare(&source, options) {
            Ok(image) => {
                let rgba = crate::scene::jobs::to_rgba(image.components, &image.data, image.width as usize * image.height as usize);
                let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &rgba);
                let _ = tx.send(Worker::Preview(id, generation, pixels));
                wake.request_repaint();
            }
            Err(error) => {
                let _ = tx.send(Worker::PreviewFailed(id, generation, error.to_string()));
                wake.request_repaint();
            }
        });
    }
    pub fn play(&mut self, id: Uuid, jobs: &crate::scene::jobs::Jobs, ctx: &egui::Context, rig: Arc<crate::scene::anim::Rig>) {
        let Some(source) = self.entries.iter().find(|e| e.id == id).and_then(|e| e.source.clone()) else {
            return;
        };
        let request = Uuid::new_v4();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            entry.player.started = true;
        }
        self.play_request = Some(request);
        if let Some(entry) = self.entries.iter().find(|entry| entry.id == id)
            && let (Some(Source::Animation { animation, .. }), Some(bound)) = (&entry.source, &entry.player.bound)
            && animation::metadata(animation) == animation::metadata(&bound.anim)
        {
            let _ = self.tx.send(Worker::Animation(request, id, bound.clone()));
            ctx.request_repaint();
            return;
        }
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            match source {
                Source::Sound(sound) => {
                    let _ = tx.send(Worker::Audio(request, aurora_audio::SoundClip::from_pcm(&sound.mono, 1, 44100)));
                }
                Source::Animation { animation, .. } => {
                    let _ = tx.send(Worker::Animation(
                        request,
                        id,
                        Arc::new(crate::scene::anim::BoundAnim::bind((*animation).clone(), &rig)),
                    ));
                }
                _ => {}
            }
            wake.request_repaint();
        });
    }
}
