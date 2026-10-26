//! Editing the worn shape without replacing authoritative server state.
//! LLPanelEditWearable / LLAgentWearables (Firestorm, originally LGPL 2.1).

pub mod camera;
pub mod catalog;
pub mod demo;
mod material;
mod preview;
#[cfg(test)]
pub(crate) mod tests;
use crate::{
    scene::{Scene, jobs::Jobs},
    world::World,
};
use aurora_assets::wearable::Wearable;
use aurora_net::{NetCommand, inventory::InvItem};
use aurora_render::preview::Model;
use rayon::prelude::*;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct Draft {
    pub item: InvItem,
    pub original: Wearable,
    pub edited: Wearable,
    pub pending: Option<Uuid>,
}
impl Draft {
    pub fn dirty(&self) -> bool {
        self.original.params != self.edited.params || self.original.textures != self.edited.textures
    }
    pub fn can_modify(&self, world: &World) -> bool {
        world
            .inventory
            .items
            .get(&self.item.id)
            .is_some_and(|i| i.owner == world.agent_id && i.owner_mask & 0x4000 != 0)
    }
    pub fn can_copy(&self, world: &World) -> bool {
        world
            .inventory
            .items
            .get(&self.item.id)
            .is_some_and(|i| i.owner == world.agent_id && i.owner_mask & 0x8000 != 0)
    }
    pub fn value(&self, param: &catalog::Param) -> f32 {
        self.edited
            .params
            .get(&param.id)
            .copied()
            .unwrap_or(param.default)
            .clamp(param.min, param.max)
    }
    pub fn set(&mut self, param: &catalog::Param, value: f32) {
        if value.is_finite() {
            self.edited.params.insert(param.id, value.clamp(param.min, param.max));
        }
    }
}
type Hint = (i32, bool, Arc<Model>);

pub struct State {
    pub open: bool,
    pub group: usize,
    pub camera_pending: bool,
    pub(crate) category_wheel: f32,
    pub(crate) category_wheel_frame: Option<u64>,
    pub filter: String,
    pub hints: bool,
    pub feet: bool,
    pub draft: Option<Draft>,
    pub loading: Option<(Uuid, Uuid)>,
    pub message: String,
    pub copy_name: Option<String>,
    pub close_confirmation: bool,
    pub close_after_save: bool,
    pub models: HashMap<(i32, bool), Arc<Model>>,
    /// Live avatar edits continue during dragging; comparison images wait
    /// until the control is released and edits have settled.
    pub editing: bool,
    edited_at: Option<Instant>,
    hint_keys: HashMap<i32, u64>,
    worker_generation: Arc<AtomicU64>,
    pub(crate) demo_mesh: Option<mpsc::Receiver<Result<demo::MeshAsset, String>>>,
    pub(crate) demo_drag: Option<Instant>,
    snapshot: Option<Arc<preview::Snapshot>>,
    snapshot_key: u64,
    snapshot_ready_key: Option<u64>,
    snapshot_pending: Option<u64>,
    snapshot_tx: mpsc::Sender<(u64, preview::Snapshot)>,
    snapshot_rx: mpsc::Receiver<(u64, preview::Snapshot)>,
    checked: Instant,
    /// The parts of the last snapshot asked for, and how many times each
    /// made it be built again since `rebuilds_since` (logged).
    snapshot_parts: Option<[u64; 3]>,
    rebuilds: [u32; 3],
    rebuilds_since: Instant,
    /// The left-out parts of the last capture, and how often they moved.
    ignored_parts: Option<[u64; 2]>,
    ignored: [u32; 2],
    generation: u64,
    model_pending: Option<(u64, u64)>,
    model_tx: mpsc::Sender<(u64, u64, Vec<Hint>)>,
    model_rx: mpsc::Receiver<(u64, u64, Vec<Hint>)>,
    model_requested: Instant,
}
impl Default for State {
    fn default() -> Self {
        let (snapshot_tx, snapshot_rx) = mpsc::channel();
        let (model_tx, model_rx) = mpsc::channel();
        Self {
            open: false,
            group: 0,
            camera_pending: false,
            category_wheel: 0.0,
            category_wheel_frame: None,
            filter: String::new(),
            hints: true,
            feet: false,
            draft: None,
            loading: None,
            message: String::new(),
            copy_name: None,
            close_confirmation: false,
            close_after_save: false,
            models: HashMap::new(),
            editing: false,
            edited_at: None,
            hint_keys: HashMap::new(),
            worker_generation: Arc::new(AtomicU64::new(1)),
            demo_mesh: None,
            demo_drag: None,
            snapshot: None,
            snapshot_key: 0,
            snapshot_ready_key: None,
            snapshot_pending: None,
            snapshot_tx,
            snapshot_rx,
            checked: Instant::now() - Duration::from_secs(1),
            snapshot_parts: None,
            rebuilds: [0; 3],
            rebuilds_since: Instant::now(),
            ignored_parts: None,
            ignored: [0; 2],
            generation: 1,
            model_pending: None,
            model_tx,
            model_rx,
            model_requested: Instant::now() - Duration::from_secs(1),
        }
    }
}

pub fn worn_shape(world: &World) -> Option<Uuid> {
    crate::world::appearance::cof(&world.inventory)
        .into_iter()
        .flat_map(|cof| crate::world::appearance::folder_links(&world.inventory, cof))
        .filter(|l| !l.folder)
        .map(|l| l.target)
        .find(|id| {
            world
                .inventory
                .items
                .get(id)
                .is_some_and(|it| it.asset_type == 13 && it.flags & 0xff == 0)
        })
}

impl State {
    pub fn open_item(&mut self, world: &mut World, item: Uuid) -> Option<NetCommand> {
        if self.draft.as_ref().is_some_and(|d| d.pending.is_some() || d.dirty()) {
            self.open = true;
            return None;
        }
        self.open = true;
        self.message.clear();
        self.camera_pending = true;
        self.draft = None;
        self.models.clear();
        self.hint_keys.clear();
        self.snapshot = None;
        self.snapshot_ready_key = None;
        self.snapshot_parts = None;
        self.group = 0;
        self.filter.clear();
        self.copy_name = None;
        self.close_confirmation = false;
        world.preview_shape(None);
        let Some(it) = world
            .inventory
            .items
            .get(&item)
            .filter(|it| Some(it.id) == worn_shape(world))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("shape-no-shape").into();
            return None;
        };
        self.loading = Some((it.id, it.asset_id));
        Some(NetCommand::PreviewInventoryItem(it))
    }

    pub fn loaded(&mut self, world: &mut World, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        if self.loading != Some((item, asset)) {
            return false;
        }
        self.loading = None;
        let Some(info) = world
            .inventory
            .items
            .get(&item)
            .filter(|i| i.asset_id == asset && Some(item) == worn_shape(world))
            .cloned()
        else {
            self.message = aurora_i18n::tr!("shape-changed").into();
            return true;
        };
        match result
            .as_ref()
            .map_err(|e| e.clone())
            .and_then(|bytes| Wearable::parse(bytes).map_err(|_| aurora_i18n::tr!("shape-error").into()))
        {
            Ok(mut original) if original.kind == 0 => {
                for param in catalog::PARAMS {
                    original.params.entry(param.id).or_insert(param.default);
                }
                original.params.entry(80).or_insert(0.0);
                self.draft = Some(Draft {
                    item: info,
                    edited: original.clone(),
                    original,
                    pending: None,
                });
                self.changed(world);
            }
            Ok(_) => self.message = aurora_i18n::tr!("shape-error").into(),
            Err(reason) => self.message = reason,
        }
        true
    }

    pub fn changed(&mut self, world: &mut World) {
        self.generation = self.generation.wrapping_add(1);
        self.worker_generation.store(self.generation, Ordering::Relaxed);
        self.edited_at = Some(Instant::now());
        if let Some(draft) = &self.draft {
            let values = world
                .avatar_lib
                .shape_params
                .edit_values(world.server_visual_params(), &draft.edited.params);
            world.preview_shape(Some(values));
        }
    }

    pub fn close(&mut self, world: &mut World) {
        self.generation = self.generation.wrapping_add(1);
        self.worker_generation.store(self.generation, Ordering::Relaxed);
        self.open = false;
        self.loading = None;
        self.draft = None;
        self.models.clear();
        self.hint_keys.clear();
        self.snapshot = None;
        self.snapshot_ready_key = None;
        self.snapshot_parts = None;
        self.close_confirmation = false;
        self.copy_name = None;
        self.close_after_save = false;
        world.preview_shape(None);
    }

    pub fn save(&mut self, world: &World, copy_name: Option<String>) -> Option<NetCommand> {
        let draft = self.draft.as_mut()?;
        if draft.pending.is_some() {
            return None;
        }
        let valid = if copy_name.is_some() {
            draft.can_copy(world) && (draft.can_modify(world) || !draft.dirty())
        } else {
            draft.can_modify(world)
        };
        let current = world.inventory.items.get(&draft.item.id)?;
        if !valid || current.asset_id != draft.item.asset_id || Some(current.id) != worn_shape(world) {
            self.message = aurora_i18n::tr!("shape-save-denied").into();
            return None;
        }
        let name = copy_name.as_deref().unwrap_or(&current.name);
        let data = match draft.edited.encode(name) {
            Ok(data) => data,
            Err(_) => {
                self.message = aurora_i18n::tr!("shape-error").into();
                return None;
            }
        };
        let request = Uuid::new_v4();
        draft.pending = Some(request);
        self.message.clear();
        Some(NetCommand::SaveWearable {
            request,
            item: Box::new(current.clone()),
            data,
            copy_name,
        })
    }

    pub fn saved(&mut self, world: &mut World, request: Uuid, result: Result<InvItem, String>) -> Option<Uuid> {
        let draft = self.draft.as_mut().filter(|d| d.pending == Some(request))?;
        draft.pending = None;
        match result {
            Ok(item) => {
                let copied = item.id != draft.item.id;
                let id = item.id;
                draft.item = item.clone();
                draft.original = draft.edited.clone();
                world.inventory.add_items(vec![item]);
                self.message = aurora_i18n::tr!("shape-saved").into();
                self.copy_name = None;
                // A copied shape is worn through the regular confirmed COF path.
                if copied {
                    self.close(world);
                    return Some(id);
                }
                if self.close_after_save {
                    self.close(world);
                }
            }
            Err(reason) => {
                self.message = reason;
                self.close_after_save = false;
            }
        }
        None
    }

    pub fn update(&mut self, world: &mut World, scene: &Scene, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if let Some(draft) = &self.draft
            && draft.pending.is_none()
            && (Some(draft.item.id) != worn_shape(world)
                || world
                    .inventory
                    .items
                    .get(&draft.item.id)
                    .is_none_or(|i| i.asset_id != draft.item.asset_id))
        {
            self.close(world);
            self.message = aurora_i18n::tr!("shape-changed").into();
            self.open = true;
            return;
        }
        while let Ok((key, snapshot)) = self.snapshot_rx.try_recv() {
            self.snapshot_pending = None;
            if key == self.snapshot_key {
                let old = self.snapshot.replace(Arc::new(snapshot));
                scene.jobs.discard(old);
                self.snapshot_ready_key = Some(key);
            } else {
                scene.jobs.discard(snapshot);
            }
        }
        while let Ok((generation, key, models)) = self.model_rx.try_recv() {
            self.model_pending = None;
            if generation == self.generation && Some(key) == self.snapshot_ready_key {
                if let Some(draft) = &self.draft {
                    for (id, _, _) in &models {
                        self.hint_keys
                            .insert(*id, hint_key(key, world.server_visual_params(), &draft.edited.params, *id));
                    }
                }
                let old: Vec<_> = models
                    .into_iter()
                    .filter_map(|(id, max, model)| self.models.insert((id, max), model))
                    .collect();
                scene.jobs.discard(old);
            } else {
                scene.jobs.discard(models);
            }
        }
        if self.hints && self.draft.is_some() && self.checked.elapsed() > Duration::from_millis(300) {
            self.checked = Instant::now();
            let input = preview::Input::capture(world, scene);
            self.snapshot_key = input.key;
            if let Some(parts) = self.ignored_parts {
                for (count, (a, b)) in self.ignored.iter_mut().zip(parts.iter().zip(&input.ignored)) {
                    *count += (a != b) as u32;
                }
            }
            self.ignored_parts = Some(input.ignored);
            if self.snapshot_ready_key != Some(input.key) && self.snapshot_pending.is_none() {
                if let Some(parts) = self.snapshot_parts {
                    for (count, changed) in self.rebuilds.iter_mut().zip(parts.iter().zip(&input.parts).map(|(a, b)| a != b)) {
                        *count += changed as u32;
                    }
                }
                self.snapshot_parts = Some(input.parts);
                let key = input.key;
                self.snapshot_pending = Some(key);
                let tx = self.snapshot_tx.clone();
                let ctx = ctx.clone();
                let previous = self.snapshot.clone();
                scene.jobs.run(move || {
                    let _ = tx.send((key, input.prepare(previous)));
                    ctx.request_repaint();
                });
            }
        }
        if self.rebuilds_since.elapsed() >= Duration::from_secs(10) {
            let [bakes, meshes, textures] = std::mem::take(&mut self.rebuilds);
            let [moves, looks] = std::mem::take(&mut self.ignored);
            if bakes + meshes + textures + moves + looks > 0 {
                log::info!(
                    "shape hints: built again for bakes {bakes}, worn meshes {meshes}, textures {textures} times in 10 s; left alone: rigid meshes moved {moves}, face tints changed {looks} times"
                );
            }
            self.rebuilds_since = Instant::now();
        }
    }

    pub fn request_hints(&mut self, world: &World, jobs: &Jobs, ctx: &egui::Context, visible: &[i32]) {
        let (Some(snapshot), Some(draft), Some(key)) = (&self.snapshot, &self.draft, self.snapshot_ready_key) else {
            return;
        };
        if !self.hints
            || self.editing
            || self.edited_at.is_some_and(|time| time.elapsed() < Duration::from_millis(180))
            || self.model_pending.is_some()
            || self.model_requested.elapsed() < Duration::from_millis(80)
        {
            return;
        }
        // A slider's own value is replaced by min/max in its hints: moving
        // that slider cannot change its comparison images.
        let visible: Vec<_> = visible
            .iter()
            .copied()
            .filter(|id| {
                self.hint_keys.get(id) != Some(&hint_key(key, world.server_visual_params(), &draft.edited.params, *id))
                    || !self.models.contains_key(&(*id, false))
                    || !self.models.contains_key(&(*id, true))
            })
            .collect();
        if visible.is_empty() {
            return;
        }
        let snapshot = snapshot.clone();
        let lib = world.avatar_lib.clone();
        let values = world.server_visual_params().to_vec();
        let params = draft.edited.params.clone();
        let generation = self.generation;
        let worker_generation = self.worker_generation.clone();
        self.model_pending = Some((generation, key));
        self.model_requested = Instant::now();
        if let Some(start) = self.demo_drag {
            log::info!(
                "shape hints: dispatch {} comparison images; demo_elapsed={:.2}s",
                visible.len() * 2,
                start.elapsed().as_secs_f32()
            );
        }
        let tx = self.model_tx.clone();
        let ctx = ctx.clone();
        jobs.run(move || {
            let models = visible
                .into_iter()
                .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == id))
                .flat_map(|param| [(param, false), (param, true)])
                .collect::<Vec<_>>()
                .into_par_iter()
                .filter_map(|(param, max)| {
                    if worker_generation.load(Ordering::Relaxed) != generation {
                        return None;
                    }
                    let mut changed = params.clone();
                    changed.insert(param.id, if max { param.max } else { param.min });
                    Some((param.id, max, snapshot.model(&lib, &values, &changed, param.group, param.elevation)))
                })
                .collect();
            let _ = tx.send((generation, key, models));
            ctx.request_repaint();
        });
    }
}

fn hint_key(snapshot: u64, values: &[u8], params: &std::collections::BTreeMap<i32, f32>, excluded: i32) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    snapshot.hash(&mut hasher);
    values.hash(&mut hasher);
    for (id, value) in params.iter().filter(|(id, _)| **id != excluded) {
        id.hash(&mut hasher);
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}
