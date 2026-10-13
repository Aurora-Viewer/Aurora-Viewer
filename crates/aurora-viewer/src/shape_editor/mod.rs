//! Editing the worn shape without replacing authoritative server state.
//! LLPanelEditWearable / LLAgentWearables (Firestorm, originally LGPL 2.1).

pub mod catalog;
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
    sync::{Arc, mpsc},
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
        self.original.params != self.edited.params
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
    snapshot: Option<Arc<preview::Snapshot>>,
    snapshot_key: u64,
    snapshot_pending: Option<u64>,
    snapshot_tx: mpsc::Sender<(u64, preview::Snapshot)>,
    snapshot_rx: mpsc::Receiver<(u64, preview::Snapshot)>,
    checked: Instant,
    generation: u64,
    models_generation: u64,
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
            snapshot: None,
            snapshot_key: 0,
            snapshot_pending: None,
            snapshot_tx,
            snapshot_rx,
            checked: Instant::now() - Duration::from_secs(1),
            generation: 1,
            models_generation: 0,
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
        self.draft = None;
        self.models.clear();
        self.snapshot = None;
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
        self.models_generation = 0;
        if let Some(draft) = &self.draft {
            let values = world
                .avatar_lib
                .shape_params
                .edit_values(world.server_visual_params(), &draft.edited.params);
            world.preview_shape(Some(values));
        }
    }

    pub fn close(&mut self, world: &mut World) {
        self.open = false;
        self.loading = None;
        self.draft = None;
        self.models.clear();
        self.snapshot = None;
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
                self.snapshot = Some(Arc::new(snapshot));
                self.models_generation = 0;
            }
        }
        while let Ok((generation, key, models)) = self.model_rx.try_recv() {
            self.model_pending = None;
            if generation == self.generation && key == self.snapshot_key {
                self.models.extend(models.into_iter().map(|(id, max, model)| ((id, max), model)));
                self.models_generation = generation;
            }
        }
        if self.hints && self.draft.is_some() && self.checked.elapsed() > Duration::from_millis(300) {
            self.checked = Instant::now();
            let input = preview::Input::capture(world, scene);
            if input.key != self.snapshot_key {
                self.snapshot_key = input.key;
                self.snapshot = None;
                self.models.clear();
            }
            if self.snapshot.is_none() && self.snapshot_pending.is_none() {
                let key = input.key;
                self.snapshot_pending = Some(key);
                let tx = self.snapshot_tx.clone();
                let ctx = ctx.clone();
                scene.jobs.run(move || {
                    let _ = tx.send((key, input.prepare()));
                    ctx.request_repaint();
                });
            }
        }
    }

    pub fn request_hints(&mut self, world: &World, jobs: &Jobs, ctx: &egui::Context, visible: &[i32]) {
        let (Some(snapshot), Some(draft)) = (&self.snapshot, &self.draft) else {
            return;
        };
        if !self.hints
            || self.model_pending.is_some()
            || self.model_requested.elapsed() < Duration::from_millis(80)
            || (self.models_generation == self.generation
                && visible
                    .iter()
                    .all(|id| self.models.contains_key(&(*id, false)) && self.models.contains_key(&(*id, true))))
        {
            return;
        }
        let visible = visible.to_vec();
        let snapshot = snapshot.clone();
        let lib = world.avatar_lib.clone();
        let values = world.server_visual_params().to_vec();
        let params = draft.edited.params.clone();
        let generation = self.generation;
        let key = self.snapshot_key;
        self.model_pending = Some((generation, key));
        self.model_requested = Instant::now();
        let tx = self.model_tx.clone();
        let ctx = ctx.clone();
        jobs.run(move || {
            let models = visible
                .into_iter()
                .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == id))
                .flat_map(|param| [(param, false), (param, true)])
                .collect::<Vec<_>>()
                .into_par_iter()
                .map(|(param, max)| {
                    let mut changed = params.clone();
                    changed.insert(param.id, if max { param.max } else { param.min });
                    (param.id, max, snapshot.model(&lib, &values, &changed, param.group, param.elevation))
                })
                .collect();
            let _ = tx.send((generation, key, models));
            ctx.request_repaint();
        });
    }
}
