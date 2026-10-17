//! AO orchestration: inventory work stays on the existing network runtime.

use super::*;
use crate::ao::{inventory, runtime};
use crate::ui::ao::{Action, Pending};
use crate::world::inventory::FetchState;
use uuid::Uuid;

impl App {
    pub(super) fn poll_ao(&mut self) {
        if !self.in_world() || self.world.agent_id.is_nil() {
            return;
        }
        if self.ao_agent != self.world.agent_id {
            self.ao_agent = self.world.agent_id;
            self.ao_catalog = Default::default();
            self.ao_bootstrap = Default::default();
            self.ao_runtime = Default::default();
            self.ao_ui = Default::default();
            self.ao_missing.clear();
            self.ao_default_applied = false;
            self.world.ao_pose = None;
        }
        if self.demo && self.ao_demo.is_some() {
            let ready = !matches!(self.ao_demo.as_deref(), Some("firestorm-wait" | "folder-names-wait")) || self.frame_count >= 720;
            crate::ao::demo::inventory_services(&mut self.world, ready);
        }
        if self.demo
            && self.ao_ui.prototype.is_none()
            && let Some(mode) = self.ao_demo.as_deref().filter(|mode| mode.starts_with("design-"))
        {
            self.ao_ui.prototype = Some(crate::ui::ao::prototype::Prototype::new(mode));
            if mode == "design-swim" {
                self.ao_ui.state = crate::ao::State::SwimmingForward;
            }
        }
        if self.demo && self.frame_count == 360 {
            match self.ao_demo.as_deref() {
                Some("folder-names" | "folder-names-wait") => {
                    self.panels.inventory = true;
                    let inv = &self.world.inventory;
                    let brand = inventory::child(inv, inv.root, "#Aurora");
                    let bridge = brand.and_then(|brand| {
                        inventory::child(inv, brand, crate::world::bridge::SUBFOLDER).or_else(|| inventory::child(inv, brand, "Bridge"))
                    });
                    self.inventory_ui
                        .open_folders([Some(inv.root), brand, bridge].into_iter().flatten());
                    crate::ui::widgets::place_floater(&self.egui_ctx, "ao", egui::pos2(90.0, 80.0), egui::vec2(600.0, 710.0));
                }
                Some("migrate" | "migrate-done") => {
                    self.panels.ao = true;
                    self.ao_ui.migration = Some(Default::default());
                }
                Some("movement" | "swim-missing") => self.open_movement(),
                Some(mode) if mode.starts_with("interface") => {
                    self.panels.ao = true;
                    self.ao_ui.interface_demo(mode, &self.ao_catalog);
                    if mode == "interface-group"
                        && let Some(set) = self.ao_catalog.selected(
                            self.settings
                                .ao
                                .accounts
                                .get(&self.world.agent_id)
                                .map_or(Uuid::nil(), |a| a.selected),
                        )
                        && let Some(c) = set.states.get(&crate::ao::State::Standing)
                        && let Some(index) = c.steps.iter().position(|s| s.group)
                    {
                        self.ao_runtime.choose(crate::ao::State::Standing, c.folder, index);
                    }
                    crate::ui::widgets::place_floater(&self.egui_ctx, "ao", egui::pos2(220.0, 90.0), egui::vec2(560.0, 760.0));
                }
                Some("swim") => {
                    self.panels.ao = true;
                    self.ao_ui.state = crate::ao::State::SwimmingForward;
                    // Expand only the owned offline capture; ordinary users
                    // retain the manager's saved size and can scroll/resize it.
                    crate::ui::widgets::place_floater(&self.egui_ctx, "ao", egui::pos2(220.0, 120.0), egui::vec2(560.0, 740.0));
                }
                _ => {}
            }
        }
        self.ao_catalog.refresh(&mut self.world.inventory);
        let selected = self
            .settings
            .ao
            .accounts
            .get(&self.world.agent_id)
            .map_or(Uuid::nil(), |a| a.selected);
        let writable = crate::ao::bootstrap::Bootstrap::writable(&self.world);
        if !self.ao_ui.busy()
            && let Some(result) = self
                .ao_bootstrap
                .poll(&mut self.world.inventory, &self.ao_catalog, selected, writable)
        {
            match result {
                Ok((change, select)) => self.apply_ao_action(Action::Edit(change, select)),
                Err(reason) => self.ao_ui.message = reason,
            }
        }
        self.ao_ui.startup_loading = self.ao_bootstrap.loading();
        if let Some(migration) = self.ao_ui.migration.as_mut() {
            migration.refresh(&mut self.world.inventory, writable);
        }
        if self.demo && self.ao_demo.as_deref() == Some("migrate-done") && self.frame_count == 720 {
            if let Some(migration) = self.ao_ui.migration.as_mut()
                && let Some(id) = migration.source.sets.iter().find(|s| s.name == "Firestorm Walk Set").map(|s| s.id)
            {
                migration.select(id);
            }
            self.apply_ao_action(Action::Migrate);
        }
        if !self.ao_default_applied && self.ao_catalog.root.is_some() && !self.ao_catalog.loading {
            if !self.ao_catalog.sets.iter().any(|s| s.id == selected)
                && let Some(set) = self.ao_catalog.sets.iter().find(|s| s.default)
            {
                self.settings.ao.accounts.entry(self.world.agent_id).or_default().selected = set.id;
            }
            self.ao_default_applied = true;
        }
        let missing: Vec<_> = self
            .ao_catalog
            .missing
            .iter()
            .chain(self.ao_bootstrap.missing())
            .chain(self.ao_ui.migration.iter().flat_map(|m| &m.source.missing))
            .copied()
            .filter(|id| self.ao_missing.insert(*id))
            .collect();
        for chunk in missing.chunks(100) {
            self.send(NetCommand::FetchItems {
                items: chunk.to_vec(),
                owner: self.world.agent_id,
            });
        }
        if let Some(item) = self.ao_ui.import_folder {
            let Some(note) = self.world.inventory.items.get(&item).cloned() else {
                self.ao_ui.import_folder = None;
                return;
            };
            match self.world.inventory.folders.get(&note.parent).map(|f| f.state) {
                Some(FetchState::Fetched) => {
                    self.ao_ui.import_folder = None;
                    self.ao_ui.import = Some(item);
                    if self.demo && item == crate::ao::demo::NOTECARD {
                        self.ao_import_result(item, Ok(crate::ao::demo::NOTECARD_TEXT.as_bytes().to_vec()));
                    } else {
                        self.send(NetCommand::PreviewInventoryItem(note));
                    }
                }
                Some(FetchState::Failed) | None => {
                    self.ao_ui.import_folder = None;
                    self.ao_ui.message = aurora_i18n::tr!("ao-fetch-failed").into();
                }
                _ => self.world.inventory.request(note.parent),
            }
        }
    }

    pub(super) fn update_ao(&mut self, now: Instant) {
        if self.world.teleporting || !self.world.movement_complete {
            return;
        }
        let account = self.settings.ao.accounts.get(&self.world.agent_id).cloned().unwrap_or_default();
        // (with Aurora's movement, the walk or the stop of the keys first)
        let raw = self.world.own_signals();
        let preview = self
            .demo
            .then(|| self.ao_ui.prototype.as_mut().map(|prototype| prototype.signal(now)))
            .flatten()
            .map(|signal| [signal]);
        let raw = preview.as_ref().map_or(raw, |signals| signals.as_slice());
        let set = self.ao_catalog.selected(account.selected);
        let on_object = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .is_some_and(|o| o.parent_id != 0);
        let water = self.world.main_water_height();
        let mut swim_input = self.input;
        if self.camera.mouselook() {
            swim_input.strafe_left |= swim_input.turn_left;
            swim_input.strafe_right |= swim_input.turn_right;
        }
        // Offline preview states share stock flight/jump/fall UUIDs. Carry
        // their explicit context just like the real movement controller does.
        let preview_swim = preview.as_ref().and_then(|signals| {
            self.ao_ui
                .prototype
                .as_ref()
                .filter(|p| p.pose.swimming())
                .map(|p| (p.pose, signals[0]))
        });
        let frame = runtime::Frame {
            raw,
            run_hold: self
                .movement_runtime
                .run_start
                .map(|start| crate::world::PlayingAnimation::from_signal(crate::ao::State::Running.motion(), 0, start, None)),
            native_running: self.movement_runtime.native_running,
            enabled: account.enabled,
            swim_hold: preview_swim.or_else(|| self.movement_runtime.swim_signal(now, &swim_input)),
            mouselook: self.camera.mouselook(),
            on_object,
            seat_animations: self.world.seat_has_animations(),
            underwater: if preview.is_some() {
                self.ao_ui.prototype.as_ref().is_some_and(|prototype| prototype.underwater())
            } else {
                self.world.agent.position.z < water
            },
            region: self.world.main_region,
        };
        let anims = &mut self.scene.anims;
        let (pose, commands) = self.ao_runtime.update(
            set,
            frame,
            now,
            |id| anims.get(id).is_some(),
            |len| ((Uuid::new_v4().as_u128() >> 64) as usize) % len.max(1),
        );
        self.world.ao_pose = Some(pose);
        for (anim, start) in commands {
            log::debug!("AO animation {} {anim}", if start { "start" } else { "stop" });
            self.send(NetCommand::AgentAnimation { anim, start });
        }
    }

    pub(super) fn apply_ao_action(&mut self, action: Action) {
        match action {
            Action::Preferences | Action::MovementPreferences => {
                if !self.demo {
                    self.settings.save();
                }
            }
            Action::Reload => {
                if !self.ao_ui.busy() {
                    self.ao_bootstrap = Default::default();
                    self.ao_missing.clear();
                    self.ao_ui.message.clear();
                    self.ao_catalog.reload(&mut self.world.inventory);
                }
            }
            Action::Import(item) => {
                if self.ao_ui.busy() {
                    return;
                }
                if let Some(note) = self.world.inventory.items.get(&item).filter(|it| it.asset_type == 7) {
                    if note.owner_mask & crate::world::inventory::actions::COPY == 0 {
                        self.ao_ui.message = aurora_i18n::tr!("app-inventory-you-are-not-allowed-to-read-this-content").into();
                        return;
                    }
                    self.world.inventory.request(note.parent);
                    self.ao_ui.import_folder = Some(item);
                    self.ao_ui.message.clear();
                }
            }
            Action::Create(set) => match inventory::create(&self.world.inventory, &self.ao_catalog, &set) {
                Ok((change, id)) => self.apply_ao_action(Action::Edit(change, Some(id))),
                Err(reason) => self.ao_ui.message = reason,
            },
            Action::Migrate => {
                if self.ao_ui.busy() {
                    return;
                }
                let writable = crate::ao::bootstrap::Bootstrap::writable(&self.world);
                let Some(migration) = self.ao_ui.migration.as_mut() else { return };
                self.ao_catalog.refresh(&mut self.world.inventory);
                migration.refresh(&mut self.world.inventory, writable);
                match migration.plan(&self.world.inventory, &self.ao_catalog) {
                    Ok((change, id)) => {
                        self.ao_ui.migration = None;
                        self.apply_ao_action(Action::Edit(change, Some(id)));
                    }
                    Err(reason) => migration.message = reason,
                }
            }
            Action::ReloadMigration => {
                if !self.ao_ui.busy()
                    && let Some(migration) = self.ao_ui.migration.as_mut()
                {
                    for id in &migration.source.missing {
                        self.ao_missing.remove(id);
                    }
                    migration.source.reload(&mut self.world.inventory);
                    migration.message.clear();
                }
            }
            Action::Edit(change, select) => self.queue_ao_edit(change, select, None),
            Action::EditContainer(change, target) => self.queue_ao_edit(change, None, Some(target)),
        }
    }

    fn queue_ao_edit(
        &mut self,
        change: aurora_net::inventory::operations::Mutation,
        select: Option<Uuid>,
        editor: Option<crate::ui::ao::EditorTarget>,
    ) {
        if self.ao_ui.busy() {
            return;
        }
        let protected = &self.settings.inventory.protected;
        let existing: Vec<_> = change
            .refresh
            .iter()
            .copied()
            .filter(|id| self.world.inventory.folders.contains_key(id))
            .collect();
        if !inventory::editable(&self.world.inventory, &change, protected) {
            self.ao_ui.message = aurora_i18n::tr!("ao-protected").into();
            return;
        }
        let request = Uuid::new_v4();
        self.ao_ui.pending = Some(Pending {
            request,
            refresh: existing,
            select,
            editor,
        });
        self.ao_ui.message.clear();
        self.send(NetCommand::EditInventory { request, change });
    }

    pub(super) fn ao_inventory_result(&mut self, result: Result<aurora_net::inventory::operations::Reply, String>) {
        let Some(pending) = self.ao_ui.pending.take() else {
            return;
        };
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents);
                if let Some(id) = pending.select.and_then(|id| reply.created.get(&id).copied()) {
                    self.settings.ao.accounts.entry(self.world.agent_id).or_default().selected = id;
                    if !self.demo {
                        self.settings.save();
                    }
                }
                self.ao_catalog.invalidate();
                self.ao_catalog.refresh(&mut self.world.inventory);
                if let Some(target) = pending.editor
                    && self.ao_ui.finish_editor(target, &reply.created)
                    && let Some(set) = self.ao_catalog.sets.iter().find(|s| s.id == target.set)
                    && let Some(config) = set.states.get(&target.state)
                    && let Some(index) = config.steps.iter().position(|s| s.group && Some(s.id) == self.ao_ui.container)
                {
                    self.ao_runtime.choose(target.state, config.folder, index);
                }
                self.ao_ui.message = aurora_i18n::tr!("ao-saved").into();
                self.ao_ui.reset_cycle();
            }
            Err(reason) => {
                self.ao_ui.message = reason;
                // AIS can fail after some operations succeeded. Reload the
                // server's state; never replay a possibly completed creation.
                for id in pending.refresh {
                    if let Some(f) = self.world.inventory.folders.get_mut(&id) {
                        f.state = FetchState::Unknown;
                    }
                    self.world.inventory.request(id);
                }
            }
        }
        self.ao_catalog.invalidate();
    }

    pub(super) fn ao_import_result(&mut self, item: Uuid, result: Result<Vec<u8>, String>) {
        self.ao_ui.import = None;
        let parsed = result
            .and_then(|data| crate::world::inventory::preview::text(7, &data))
            .and_then(|(text, _)| inventory::import(&self.world.inventory, item, &text));
        match parsed {
            Ok((mut set, warnings)) => {
                set.default = self.ao_catalog.sets.is_empty();
                self.ao_ui.warnings = warnings;
                self.apply_ao_action(Action::Create(set));
            }
            Err(reason) => self.ao_ui.message = reason,
        }
    }
}
