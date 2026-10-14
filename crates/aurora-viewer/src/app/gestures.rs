//! Gesture lifecycle and routing; no I/O or network waits on the frame thread.
use super::App;
use crate::gestures::{Effect, Entry, Manager};
use crate::ui::gestures::{Action, GesturesUi};
use crate::world::inventory::{FetchState, actions as rules};
use aurora_assets::gesture::{ANIM_STOP, Gesture, Step};
use aurora_net::inventory::operations::{Mutation, Operation};
use aurora_net::{ChatType, NetCommand};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    pub manager: Manager,
    pub ui: GesturesUi,
    pub new_editor: bool,
    pending: BTreeMap<Uuid, Load>,
    saves: BTreeMap<Uuid, Save>,
    failed: BTreeSet<(Uuid, Uuid)>,
    cof: BTreeSet<Uuid>,
    generation: Option<u64>,
    scan_generation: Option<u64>,
    demo_seeded: bool,
    demo_played: bool,
    missing_items: BTreeSet<Uuid>,
    known_items: BTreeSet<Uuid>,
}

#[derive(Default)]
struct Load {
    asset: Uuid,
    open: bool,
    play: bool,
    active: Option<bool>,
    requested: bool,
    inform: bool,
}
struct Save {
    item: Uuid,
    definition: Gesture,
    description: String,
    data: Vec<u8>,
    metadata: bool,
}

impl State {
    fn queue_load(&mut self, item: Uuid, asset: Uuid, open: bool, play: bool, active: Option<bool>, inform: bool) {
        let load = self.pending.entry(item).or_insert_with(|| Load {
            asset,
            ..Default::default()
        });
        if load.asset != asset {
            *load = Load {
                asset,
                ..Default::default()
            };
        }
        load.open |= open;
        load.play |= play;
        if active.is_some() {
            load.active = active;
            load.inform = inform;
        }
        // Like mActive[item] = NULL in LLGestureMgr: activation is visible and
        // cancellable immediately, before the asset arrives.
        if active == Some(true) && self.manager.active.insert(item) {
            self.manager.changed();
        }
    }

    fn cancel_load(&mut self, item: Uuid) -> bool {
        if let Some(load) = self.pending.get_mut(&item) {
            load.active = Some(false);
            load.play = false;
        }
        self.manager.active.remove(&item)
    }

    fn take_load(&mut self, item: Uuid, asset: Uuid) -> Option<Load> {
        if self.pending.get(&item).is_none_or(|p| p.asset != asset) {
            return None;
        }
        self.pending.remove(&item)
    }
    fn unavailable(&mut self, inv: &crate::world::inventory::Inventory) -> Vec<Uuid> {
        self.known_items
            .extend(self.manager.active.iter().filter(|id| inv.items.contains_key(id)).copied());
        self.manager
            .active
            .iter()
            .filter(|id| rules::in_type(inv, **id, 14) || (self.known_items.contains(id) && !inv.items.contains_key(id)))
            .copied()
            .collect()
    }
}

impl App {
    pub(super) fn restore_gestures(&mut self, raw: &aurora_llsd::Llsd) {
        if let aurora_llsd::Llsd::Array(values) = &raw["gestures"] {
            for value in values {
                let (item, asset) = (value["item_id"].as_uuid(), value["asset_id"].as_uuid());
                if !item.is_nil() && !asset.is_nil() {
                    self.gesture_state.manager.active.insert(item);
                    self.gesture_state.pending.insert(
                        item,
                        Load {
                            asset,
                            active: Some(true),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        self.world.active_gestures.clone_from(&self.gesture_state.manager.active);
    }

    fn gesture_request(&mut self, id: Uuid, open: bool, play: bool, active: Option<bool>, inform: bool) {
        let Some(id) = rules::original(&self.world.inventory, id) else {
            return;
        };
        let Some(it) = self.world.inventory.items.get(&id) else {
            return;
        };
        if it.asset_type != 21 {
            return;
        }
        let asset = it.asset_id;
        self.gesture_state.known_items.insert(id);
        if open {
            self.gesture_state.ui.open(&self.world.inventory, id);
        }
        if self.gesture_state.manager.entries.get(&id).is_some_and(|e| e.asset == asset) {
            if open
                && let Some(definition) = self.gesture_state.manager.entries.get(&id).map(|e| e.definition.clone())
                && let Some(editor) = self.gesture_state.ui.editors.get_mut(&id)
                && editor.definition.is_none()
            {
                editor.loaded(definition);
            }
            if let Some(on) = active {
                self.gesture_active(id, on, inform);
            }
            if play {
                self.gesture_play(id, None);
            }
            return;
        }
        self.gesture_state.failed.remove(&(id, asset));
        self.gesture_state.queue_load(id, asset, open, play, active, inform);
        self.world.active_gestures.clone_from(&self.gesture_state.manager.active);
        if asset.is_nil() {
            self.gesture_loaded(id, asset, Ok(Gesture::default()));
        }
    }

    pub(super) fn gesture_loaded(&mut self, item: Uuid, asset: Uuid, result: Result<Gesture, String>) {
        let Some(load) = self.gesture_state.take_load(item, asset) else {
            return;
        };
        if self.gesture_state.known_items.contains(&item) && !self.world.inventory.items.contains_key(&item) {
            return;
        }
        if self.world.inventory.items.get(&item).is_some_and(|it| it.asset_id != asset) {
            return;
        }
        self.gesture_state.manager.changed();
        match result {
            Ok(definition) => {
                if load.open
                    && let Some(editor) = self.gesture_state.ui.editors.get_mut(&item)
                    && editor.definition.is_none()
                {
                    editor.loaded(definition.clone());
                }
                self.gesture_state.manager.entries.insert(item, Entry { asset, definition });
                if let Some(active) = load.active {
                    self.gesture_active(item, active, false);
                    if active && load.inform && !asset.is_nil() {
                        self.send(NetCommand::SetGestureActive { item, asset, active: true });
                    }
                }
                if load.play {
                    self.gesture_play(item, None);
                }
            }
            Err(error) => {
                self.gesture_state.failed.insert((item, asset));
                self.gesture_state.ui.message.clone_from(&error);
                if let Some(editor) = self.gesture_state.ui.editors.get_mut(&item) {
                    editor.message = error;
                }
                self.gesture_state.manager.active.remove(&item);
                self.world.active_gestures.clone_from(&self.gesture_state.manager.active);
            }
        }
    }

    fn gesture_active(&mut self, id: Uuid, on: bool, inform: bool) {
        self.gesture_state.manager.changed();
        if on {
            let changed = self.gesture_state.manager.active.insert(id);
            if changed
                && inform
                && let Some(entry) = self.gesture_state.manager.entries.get(&id)
                && !entry.asset.is_nil()
            {
                self.send(NetCommand::SetGestureActive {
                    item: id,
                    asset: entry.asset,
                    active: true,
                });
            }
        } else {
            let changed = self.gesture_state.cancel_load(id);
            let effects = self.gesture_state.manager.stop(id);
            self.gesture_effects(effects);
            if changed && inform {
                self.send(NetCommand::SetGestureActive {
                    item: id,
                    asset: Uuid::nil(),
                    active: false,
                });
                if let Some(cof) = crate::world::appearance::cof(&self.world.inventory) {
                    let removed: Vec<_> = self
                        .world
                        .inventory
                        .items
                        .values()
                        .filter(|it| it.parent == cof && it.asset_type == 24 && it.asset_id == id)
                        .map(|it| it.id)
                        .collect();
                    if !removed.is_empty() {
                        let change = Mutation {
                            operations: removed.iter().map(|id| Operation::Delete { id: *id, folder: false }).collect(),
                            refresh: vec![cof],
                            removed,
                            ..Default::default()
                        };
                        self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(change));
                    }
                }
            }
        }
        self.world.active_gestures.clone_from(&self.gesture_state.manager.active);
    }

    pub(super) fn gesture_play(&mut self, id: Uuid, key: Option<u8>) {
        if let Some(definition) = self.gesture_state.manager.entries.get(&id).map(|e| e.definition.clone()) {
            self.gesture_preview(id, definition, key);
        } else {
            self.gesture_request(id, false, true, None, false);
        }
    }

    fn gesture_preview(&mut self, id: Uuid, definition: Gesture, key: Option<u8>) {
        for step in &definition.steps {
            match step {
                Step::Animation { asset, flags, .. } if flags & ANIM_STOP == 0 && !asset.is_nil() => {
                    self.scene.anims.get(asset);
                }
                Step::Sound { asset, .. } if !asset.is_nil() => self.scene.sounds.want(*asset),
                _ => {}
            }
        }
        let effects = self.gesture_state.manager.play(id, definition, key);
        self.gesture_effects(effects);
        self.tick_gestures();
    }

    fn tick_gestures(&mut self) {
        if !self.gesture_state.manager.has_playing() {
            return;
        }
        let signaled = self.world.animations_of(&self.world.agent_id).iter().map(|a| a.id).collect();
        let effects = self.gesture_state.manager.tick_ready(Instant::now(), &signaled, |step| match step {
            Step::Animation { asset, flags, .. } => asset.is_nil() || flags & ANIM_STOP != 0 || self.scene.anims.asset_settled(asset),
            Step::Sound { asset, .. } => asset.is_nil() || self.scene.sounds.asset_settled(asset),
            _ => true,
        });
        self.gesture_effects(effects);
    }

    fn gesture_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Animation(anim, start) => {
                    if self.demo {
                        let motions = self.world.animations.entry(self.world.agent_id).or_default();
                        motions.retain(|a| a.id != anim);
                        if start {
                            motions.push(crate::world::PlayingAnimation::from_signal(anim, 0, Instant::now(), None));
                        }
                    } else {
                        self.send(NetCommand::AgentAnimation { anim, start });
                    }
                }
                Effect::Sound(asset) => {
                    if self.demo {
                        self.scene
                            .sounds
                            .trigger(asset, self.world.agent.position, Some(self.world.agent_id), 1.0);
                    }
                    if !self.demo {
                        self.send(NetCommand::GestureSound {
                            asset,
                            position: self.world.agent.position
                                - self
                                    .world
                                    .main_region
                                    .and_then(|handle| self.world.region_offset(handle))
                                    .unwrap_or(glam::Vec3::ZERO),
                        });
                    }
                }
                Effect::Chat(text) => {
                    let (channel, message) = crate::ui::chat::parse_channel(&text);
                    if !message.is_empty() && (channel != 0 || !self.chat_command(&message)) {
                        self.send(NetCommand::Chat {
                            message: crate::ui::emoji::expand_shortcodes(&message),
                            channel,
                            chat_type: ChatType::Normal,
                        });
                    }
                }
            }
        }
    }

    pub(super) fn gesture_action(&mut self, action: Action) {
        match action {
            Action::Open(id) => self.gesture_request(id, true, false, None, false),
            Action::Play(id) => {
                if self.gesture_state.manager.is_playing(id) {
                    let effects = self.gesture_state.manager.stop(id);
                    self.gesture_effects(effects);
                } else {
                    self.gesture_request(id, false, true, Some(true), true);
                }
            }
            Action::Preview(id, definition) => self.gesture_preview(id, definition, None),
            Action::Stop(id) => {
                let effects = self.gesture_state.manager.stop(id);
                self.gesture_effects(effects);
            }
            Action::Active(ids, on) => {
                for id in ids {
                    let Some(id) = rules::original(&self.world.inventory, id) else {
                        continue;
                    };
                    if rules::library(&self.world.inventory, id) || (on && rules::in_type(&self.world.inventory, id, 14)) {
                        continue;
                    }
                    if on {
                        self.gesture_request(id, false, false, Some(true), true);
                    } else {
                        self.gesture_active(id, false, true);
                    }
                }
            }
            Action::Save {
                item,
                definition,
                description,
                close,
            } => self.gesture_save(item, definition, description, close),
            Action::Create => {
                self.gesture_state.new_editor = true;
                let parent = rules::system(&self.world.inventory, 21).unwrap_or(self.world.inventory.root);
                self.apply_inventory_action(crate::ui::inventory::InvAction::Create {
                    parent,
                    kind: aurora_net::inventory::operations::NewItem::Gesture,
                    name: aurora_i18n::tr!("gesture-new-name").into(),
                });
                self.panels.inventory = true;
            }
            Action::Delete(ids) => {
                self.inventory_ui.dialog = Some(crate::ui::inventory::EditDialog::Delete(ids, false));
                self.panels.inventory = true;
            }
            Action::ShowInventory(id) => {
                self.inventory_ui.show_original(&self.world.inventory, id);
                self.panels.inventory = true;
            }
            Action::AddToOutfit(ids) => {
                if let Some(cof) = crate::world::appearance::cof(&self.world.inventory) {
                    let existing = crate::world::appearance::folder_links(&self.world.inventory, cof);
                    let links: Vec<_> = ids
                        .into_iter()
                        .filter(|id| !existing.iter().any(|l| l.target == *id))
                        .filter_map(|id| self.world.inventory.items.get(&id))
                        .filter(|it| it.asset_type == 21)
                        .map(|it| aurora_net::outfits::OutfitLink {
                            target: it.id,
                            name: it.name.clone(),
                            desc: it.desc.clone(),
                            folder: false,
                            inv_type: 20,
                        })
                        .collect();
                    if !links.is_empty() {
                        self.apply_inventory_action(crate::ui::inventory::InvAction::Edit(Mutation {
                            operations: vec![Operation::Links { parent: cof, links }],
                            refresh: vec![cof],
                            ..Default::default()
                        }));
                    }
                }
            }
            Action::Refresh => {
                self.gesture_state.scan_generation = None;
                self.gesture_state.manager.changed();
                self.gesture_state.failed.clear();
                self.gesture_state
                    .manager
                    .entries
                    .retain(|id, _| self.gesture_state.manager.active.contains(id));
            }
        }
    }

    pub(super) fn update_gestures(&mut self) {
        self.seed_gesture_demo();
        if self.demo
            && !self.gesture_state.demo_played
            && self.frame_count >= 650
            && std::env::var("AURORA_DEMO_GESTURES").is_ok_and(|v| v == "play")
        {
            self.gesture_state.demo_played = true;
            self.gesture_action(Action::Play(Uuid::from_u128(8106)));
            self.gesture_action(Action::Play(Uuid::from_u128(951001)));
        }
        self.gesture_state.manager.enabled = self
            .settings
            .gestures
            .accounts
            .get(&self.world.agent_id)
            .is_none_or(|prefs| prefs.enabled);
        if self.world.viewer_asset_url().is_some() || self.demo {
            let missing: Vec<_> = self
                .gesture_state
                .pending
                .keys()
                .filter(|id| !self.world.inventory.items.contains_key(id))
                .copied()
                .collect();
            if !missing.is_empty() && self.gesture_state.generation != Some(self.world.inventory.generation) {
                self.send(NetCommand::FetchItems {
                    items: missing,
                    owner: self.world.agent_id,
                });
            }
            let capacity = 32usize.saturating_sub(self.gesture_state.pending.values().filter(|load| load.requested).count());
            let pending: Vec<_> = self
                .gesture_state
                .pending
                .iter_mut()
                .filter(|(_, p)| !p.requested)
                .take(capacity)
                .map(|(id, p)| {
                    p.requested = true;
                    (*id, p.asset)
                })
                .collect();
            for (item, asset) in pending {
                self.send(NetCommand::LoadGesture { item, asset });
            }
        }
        if self.gesture_state.generation != Some(self.world.inventory.generation) {
            self.gesture_state.generation = Some(self.world.inventory.generation);
            if let Some(cof) = crate::world::appearance::cof(&self.world.inventory)
                && self
                    .world
                    .inventory
                    .folders
                    .get(&cof)
                    .is_some_and(|f| f.state == FetchState::Fetched)
            {
                let missing: Vec<_> = crate::world::appearance::folder_links(&self.world.inventory, cof)
                    .iter()
                    .filter(|link| link.inv_type == 20 && !self.world.inventory.items.contains_key(&link.target))
                    .map(|link| link.target)
                    .filter(|id| self.gesture_state.missing_items.insert(*id))
                    .collect();
                for items in missing.chunks(100) {
                    self.send(NetCommand::FetchItems {
                        items: items.to_vec(),
                        owner: self.world.agent_id,
                    });
                }
                let current: BTreeSet<_> = crate::world::appearance::folder_links(&self.world.inventory, cof)
                    .iter()
                    .filter_map(|l| self.world.inventory.items.get(&l.target))
                    .filter(|it| it.asset_type == 21)
                    .map(|it| it.id)
                    .collect();
                let added: Vec<_> = current.difference(&self.gesture_state.cof).copied().collect();
                let removed: Vec<_> = self.gesture_state.cof.difference(&current).copied().collect();
                self.gesture_state.cof = current;
                for id in added {
                    self.gesture_request(id, false, false, Some(true), true);
                }
                for id in removed {
                    self.gesture_active(id, false, true);
                }
            }
            let trashed = self.gesture_state.unavailable(&self.world.inventory);
            for id in trashed {
                self.gesture_active(id, false, true);
            }
        }
        self.tick_gestures();
    }

    pub(super) fn show_gestures(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.panels.gestures && self.gesture_state.ui.editors.is_empty() {
            return;
        }
        if self.panels.gestures {
            if self.gesture_state.scan_generation != Some(self.world.inventory.generation) {
                self.gesture_state.scan_generation = Some(self.world.inventory.generation);
                let folders: Vec<_> = self
                    .world
                    .inventory
                    .folders
                    .values()
                    .filter(|f| f.info.type_default == 21 || rules::in_type(&self.world.inventory, f.info.id, 21))
                    .filter(|f| f.state == FetchState::Unknown)
                    .map(|f| f.info.id)
                    .collect();
                for folder in folders {
                    self.world.inventory.request(folder);
                }
            }
            self.gesture_state.ui.refresh_catalog(&self.world.inventory);
            let capacity = 32usize.saturating_sub(self.gesture_state.pending.len());
            let to_load: Vec<_> = self
                .gesture_state
                .ui
                .catalog
                .iter()
                .copied()
                .filter(|id| {
                    self.world.inventory.items.get(id).is_some_and(|it| {
                        !self.gesture_state.manager.entries.contains_key(id)
                            && !self.gesture_state.pending.contains_key(id)
                            && !self.gesture_state.failed.contains(&(*id, it.asset_id))
                    })
                })
                .take(capacity)
                .collect();
            for id in to_load {
                self.gesture_request(id, false, false, None, false);
            }
        }
        let before = self.settings.gestures.clone();
        for action in crate::ui::gestures::show(
            ctx,
            p,
            &self.world.inventory,
            &self.gesture_state.manager,
            &mut self.gesture_state.ui,
            self.settings.gestures.accounts.entry(self.world.agent_id).or_default(),
            &mut self.panels.gestures,
        ) {
            self.gesture_action(action);
        }
        if before != self.settings.gestures {
            self.settings.save();
            self.gesture_state.manager.enabled = self
                .settings
                .gestures
                .accounts
                .get(&self.world.agent_id)
                .is_none_or(|prefs| prefs.enabled);
            if !self
                .settings
                .gestures
                .accounts
                .get(&self.world.agent_id)
                .is_none_or(|prefs| prefs.enabled)
            {
                let effects = self.gesture_state.manager.stop_all();
                self.gesture_effects(effects);
            }
        }
    }

    fn gesture_save(&mut self, item: Uuid, definition: Gesture, description: String, close: bool) {
        let Some(it) = self.world.inventory.items.get(&item) else {
            return;
        };
        let Some(editor) = self.gesture_state.ui.editors.get_mut(&item) else {
            return;
        };
        if editor.saving.is_some() || rules::library(&self.world.inventory, item) {
            return;
        }
        if it.owner_mask & rules::MODIFY == 0
            && (editor.saved.as_ref().is_none_or(|g| g.steps != definition.steps) || it.desc != description)
        {
            editor.message = aurora_i18n::tr!("gesture-modify-denied").into();
            return;
        }
        let data = match definition.encode() {
            Ok(data) if data.len() <= 1000 => data,
            Ok(_) => {
                editor.message = aurora_i18n::tr!("gesture-too-large").into();
                return;
            }
            Err(_) => {
                editor.message = aurora_i18n::tr!("gesture-invalid").into();
                return;
            }
        };
        let request = Uuid::new_v4();
        editor.saving = Some(request);
        editor.close_after_save = close;
        editor.message.clear();
        let metadata = description != it.desc;
        let parent = it.parent;
        self.gesture_state.saves.insert(
            request,
            Save {
                item,
                definition,
                description: description.clone(),
                data: data.clone(),
                metadata,
            },
        );
        if metadata {
            let change = Mutation {
                operations: vec![Operation::Patch {
                    id: item,
                    folder: false,
                    body: aurora_llsd::llsd_map! { "desc" => description },
                }],
                refresh: vec![parent],
                ..Default::default()
            };
            self.send(NetCommand::EditInventory { request, change });
        } else {
            self.send(NetCommand::SaveInventoryContent {
                request,
                item,
                asset_type: 21,
                data,
            });
        }
    }

    pub(super) fn gesture_save_pending(&self, request: Uuid, item: Uuid) -> bool {
        self.gesture_state
            .saves
            .get(&request)
            .is_some_and(|s| s.item == item && !s.metadata)
    }

    pub(super) fn gesture_metadata_result(
        &mut self,
        request: Uuid,
        result: &Result<aurora_net::inventory::operations::Reply, String>,
    ) -> bool {
        let Some(save) = self.gesture_state.saves.get_mut(&request).filter(|s| s.metadata) else {
            return false;
        };
        save.metadata = false;
        let (item, data) = (save.item, save.data.clone());
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents.clone());
                self.send(NetCommand::SaveInventoryContent {
                    request,
                    item,
                    asset_type: 21,
                    data,
                });
            }
            Err(error) => self.gesture_saved(request, item, Err(error.clone())),
        }
        true
    }

    pub(super) fn gesture_saved(&mut self, request: Uuid, item: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        let Some(save) = self.gesture_state.saves.remove(&request) else {
            return;
        };
        if save.item != item {
            return;
        }
        if let Some(editor) = self.gesture_state.ui.editors.get_mut(&item) {
            editor.saving = None;
        }
        match result {
            Ok(saved) => {
                self.gesture_state.manager.changed();
                let asset = saved.asset_id;
                self.world.inventory.add_items(vec![saved]);
                if let Some(editor) = self.gesture_state.ui.editors.get_mut(&item) {
                    editor.saved = Some(save.definition.clone());
                    editor.saved_description = save.description;
                    if editor.close_after_save && !editor.dirty() {
                        editor.open = false;
                    }
                }
                let similar = if self.gesture_state.manager.active.contains(&item) {
                    self.gesture_state.manager.similar(item, &save.definition)
                } else {
                    Vec::new()
                };
                for id in similar {
                    self.gesture_active(id, false, true);
                }
                let effects = self.gesture_state.manager.stop(item);
                self.gesture_effects(effects);
                self.gesture_state.manager.entries.insert(
                    item,
                    Entry {
                        asset,
                        definition: save.definition,
                    },
                );
                if self.gesture_state.manager.active.contains(&item) {
                    self.send(NetCommand::SetGestureActive { item, asset, active: true });
                }
            }
            Err(error) => {
                if let Some(editor) = self.gesture_state.ui.editors.get_mut(&item) {
                    editor.message = error;
                    editor.close_after_save = false;
                }
            }
        }
    }

    pub(super) fn gesture_demo_command(&mut self, command: &NetCommand) -> bool {
        match command {
            NetCommand::LoadGesture { item, asset } => {
                let definition = self
                    .demo_documents
                    .get(item)
                    .map_or_else(|| Ok(Gesture::default()), |data| Gesture::decode(data).map_err(|e| e.to_string()));
                self.gesture_loaded(*item, *asset, definition);
                true
            }
            NetCommand::SetGestureActive { .. } | NetCommand::GestureSound { .. } => true,
            _ => false,
        }
    }

    pub(super) fn gesture_demo_input(&mut self, raw: &mut egui::RawInput) {
        let mode = std::env::var("AURORA_DEMO_GESTURES").unwrap_or_default();
        if matches!(mode.as_str(), "communicate" | "communicate-open") {
            raw.focused = true;
            let mouse = match self.frame_count {
                540 => Some((egui::pos2(14.0, 11.0), Some(true))),
                541 => Some((egui::pos2(14.0, 11.0), Some(false))),
                560 => Some((egui::pos2(100.0, 62.0), None)),
                650 if mode == "communicate-open" => Some((egui::pos2(245.0, 84.0), Some(true))),
                651 if mode == "communicate-open" => Some((egui::pos2(245.0, 84.0), Some(false))),
                _ => None,
            };
            if let Some((pos, pressed)) = mouse {
                raw.events.push(egui::Event::PointerMoved(pos));
                if let Some(pressed) = pressed {
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
            return;
        }
        if !matches!(mode.as_str(), "completion" | "completion-send") {
            return;
        }
        // The synthetic keystrokes own egui focus even when the automated
        // demo window is behind the user's current desktop window.
        raw.focused = true;
        match self.frame_count {
            540 => self.chat_ui.focus_request = true,
            550 => raw.events.push(egui::Event::Text("/ej".into())),
            650 if mode == "completion-send" => raw.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }),
            _ => {}
        }
    }

    fn seed_gesture_demo(&mut self) {
        if !self.demo || self.gesture_state.demo_seeded || std::env::var_os("AURORA_DEMO_GESTURES").is_none() {
            return;
        }
        self.gesture_state.demo_seeded = true;
        crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
        let definition = Gesture {
            trigger: if std::env::var("AURORA_DEMO_GESTURES").is_ok_and(|v| v.starts_with("completion")) {
                "/eject"
            } else {
                "/hello"
            }
            .into(),
            replacement: aurora_i18n::tr!("gesture-demo-replacement").into(),
            key: 0xa8,
            steps: vec![
                Step::Chat {
                    text: aurora_i18n::tr!("gesture-demo-chat").into(),
                    flags: 0,
                },
                Step::Wait {
                    seconds: 1.0,
                    flags: aurora_assets::gesture::WAIT_TIME,
                },
                Step::Chat {
                    text: "/28574 T4".into(),
                    flags: 0,
                },
            ],
            ..Default::default()
        };
        let item = Uuid::from_u128(8106);
        if let Ok(data) = definition.encode() {
            self.demo_documents.insert(item, data);
        }
        self.gesture_request(item, false, false, Some(true), false);
        let (sound, clip) = crate::demo::chime();
        self.scene.sounds.insert(sound, clip);
        if let Some(mut sound_item) = self.world.inventory.items.get(&Uuid::from_u128(8105)).cloned() {
            sound_item.asset_id = sound;
            self.world.inventory.add_items(vec![sound_item]);
        }
        if let Some(template) = self.world.inventory.items.get(&item).cloned() {
            for (id, name, trigger, key, steps, active) in [
                (
                    951001,
                    aurora_i18n::tr!("gesture-demo-animation-name"),
                    "/wave",
                    0xa9,
                    vec![
                        Step::Animation {
                            name: aurora_i18n::tr!("gesture-demo-animation-name").into(),
                            asset: crate::demo::IDLE_ANIM,
                            flags: 0,
                        },
                        Step::Wait {
                            seconds: 1.0,
                            flags: aurora_assets::gesture::WAIT_TIME,
                        },
                        Step::Animation {
                            name: aurora_i18n::tr!("gesture-demo-animation-name").into(),
                            asset: crate::demo::IDLE_ANIM,
                            flags: ANIM_STOP,
                        },
                        Step::Wait {
                            seconds: 0.0,
                            flags: aurora_assets::gesture::WAIT_ANIMATIONS,
                        },
                        Step::Chat {
                            text: aurora_i18n::tr!("gesture-demo-finished").into(),
                            flags: 0,
                        },
                    ],
                    true,
                ),
                (
                    951002,
                    aurora_i18n::tr!("gesture-demo-sound-name"),
                    "/chime",
                    0xaa,
                    vec![Step::Sound {
                        name: aurora_i18n::tr!("gesture-demo-sound-name").into(),
                        asset: sound,
                        flags: 0,
                    }],
                    false,
                ),
            ] {
                let mut it = template.clone();
                it.id = Uuid::from_u128(id);
                it.asset_id = Uuid::from_u128(id + 100);
                it.name = name.into();
                let definition = Gesture {
                    trigger: trigger.into(),
                    key,
                    steps,
                    ..Default::default()
                };
                if let Ok(data) = definition.encode() {
                    self.demo_documents.insert(it.id, data);
                }
                let item = it.id;
                self.world.inventory.add_items(vec![it]);
                self.gesture_request(item, false, false, active.then_some(true), false);
            }
        }
        self.panels.gestures = !std::env::var("AURORA_DEMO_GESTURES").is_ok_and(|v| v.starts_with("communicate"));
        if std::env::var("AURORA_DEMO_GESTURES").is_ok_and(|v| v == "editor") {
            self.gesture_action(Action::Open(item));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_waits_for_inventory_but_removed_known_items_are_deactivated() {
        let mut state = State::default();
        let item = Uuid::from_u128(8106);
        let mut inv = crate::world::inventory::Inventory::default();
        state.manager.active.insert(item);
        assert!(state.unavailable(&inv).is_empty(), "login metadata has not arrived");
        crate::world::inventory::demo::seed(&mut inv, Uuid::from_u128(2));
        assert!(state.unavailable(&inv).is_empty());
        inv.items.remove(&item);
        assert_eq!(state.unavailable(&inv), [item]);
    }
    #[test]
    fn activation_can_be_cancelled_before_loading_and_stale_results_do_not_replace_new_requests() {
        let mut state = State::default();
        let item = Uuid::from_u128(1);
        let old = Uuid::from_u128(2);
        let new = Uuid::from_u128(3);
        state.queue_load(item, old, false, true, Some(true), true);
        assert!(state.manager.active.contains(&item));
        state.cancel_load(item);
        assert!(!state.manager.active.contains(&item));
        let load = state.take_load(item, old).expect("pending load");
        assert_eq!(load.active, Some(false));
        assert!(!load.play);
        state.queue_load(item, old, true, false, None, false);
        state.queue_load(item, new, true, false, None, false);
        assert!(state.take_load(item, old).is_none());
        assert_eq!(state.take_load(item, new).expect("new load").asset, new);
    }
}
