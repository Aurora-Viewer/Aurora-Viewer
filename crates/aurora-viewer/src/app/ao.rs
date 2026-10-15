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
            self.ao_runtime = Default::default();
            self.ao_ui = Default::default();
            self.ao_missing.clear();
            self.ao_default_applied = false;
            self.world.ao_pose = None;
        }
        self.ao_catalog.refresh(&mut self.world.inventory);
        if !self.ao_default_applied && !self.ao_catalog.loading {
            if let Some(set) = self.ao_catalog.sets.iter().find(|s| s.default) {
                self.settings.ao.accounts.entry(self.world.agent_id).or_default().selected = set.id;
            }
            self.ao_default_applied = true;
        }
        let missing: Vec<_> = self
            .ao_catalog
            .missing
            .iter()
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
        let set = self.ao_catalog.selected(account.selected);
        let on_object = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .is_some_and(|o| o.parent_id != 0);
        let water = self.world.main_water_height();
        let frame = runtime::Frame {
            raw,
            enabled: account.enabled,
            mouselook: self.camera.mouselook(),
            on_object,
            seat_animations: self.world.seat_has_animations(),
            underwater: self.world.agent.position.z < water,
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
            Action::Preferences => self.settings.save(),
            Action::Reload => {
                if !self.ao_ui.busy() {
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
            Action::Edit(change, select) => {
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
                });
                self.ao_ui.message.clear();
                self.send(NetCommand::EditInventory { request, change });
            }
        }
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
                    self.settings.save();
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
