//! The one-time rez is owned independently of the picker/animation floater,
//! so closing either still runs identification and cleanup.
use super::*;
use crate::imports::{
    Source,
    animation_inventory::{self as inventory, Pending},
};
use aurora_net::build::BuildCmd;

impl App {
    pub(super) fn choose_inventory_animation_model(&mut self, entry: uuid::Uuid) {
        if self.imports.animation_inventory.pending.is_some() {
            return;
        }
        self.imports.animation_inventory.entry = Some(entry);
        self.imports.animation_inventory.error = None;
    }

    pub(super) fn show_inventory_animation_model(&mut self, ctx: &egui::Context, palette: &Palette) {
        let selected = crate::imports::ui::animation_inventory::show(
            &mut self.imports.animation_inventory,
            ctx,
            palette,
            &mut self.world.inventory,
            self.world.agent_id,
            self.world.movement_complete && !self.world.teleporting && self.imports.local_rez.is_none(),
        );
        let Some(item) = selected else { return };
        self.prepare_inventory_animation_model(item, ctx);
    }

    fn prepare_inventory_animation_model(&mut self, item: uuid::Uuid, ctx: &egui::Context) {
        let Some(entry_id) = self.imports.animation_inventory.entry else {
            return;
        };
        let profile = self.imports.animation_inventory.profile.clone();
        if let Ok(item) = inventory::available(&self.world.inventory, item, self.world.agent_id)
            && let Some(saved) = self.imports.load_preferences.animation_models.iter().find(|saved| {
                saved
                    .inventory
                    .as_ref()
                    .is_some_and(|origin| origin.profile == profile && origin.item == item.id)
            })
        {
            // Re-adding an already prepared item is a purely local selection.
            if self.demo {
                log::info!("demo animation inventory: saved_model_reused=true rez_once=0");
            }
            let saved = saved.id;
            self.imports
                .select_animation_model(entry_id, saved, &self.scene.jobs, ctx, self.scene.avatar_lib.clone());
            self.imports.animation_inventory.entry = None;
            return;
        }
        let Some(entry) = self
            .imports
            .entries
            .iter_mut()
            .find(|e| e.id == entry_id && matches!(e.source, Some(Source::Animation { .. })))
        else {
            return;
        };
        if entry.animation_model.pending {
            return;
        }
        let generation = entry.animation_model.generation + 1;
        match inventory::plan(
            &self.world,
            entry_id,
            generation,
            item,
            profile,
            crate::settings::config_dir().join("animation_models"),
        ) {
            Ok((pending, command)) => {
                entry.animation_model.generation = generation;
                entry.animation_model.pending = true;
                entry.animation_model.error = None;
                self.imports.animation_inventory.pending = Some(pending);
                if self.demo {
                    log::info!("demo animation inventory: rez_once=1");
                }
                self.send(NetCommand::Build(command));
            }
            Err(error) => self.imports.animation_inventory.error = Some(error),
        }
    }

    pub(super) fn poll_inventory_animation_model(&mut self, ctx: &egui::Context) {
        if self.demo
            && let Some(mesh) = self.imports.animation_inventory.demo_mesh.take()
        {
            self.scene.meshes.insert_local(mesh.id, mesh.data, mesh.header, mesh.skin);
            crate::world::inventory::demo::seed(&mut self.world.inventory, self.world.agent_id);
            let id = inventory::DEMO_ITEM;
            let parent = uuid::Uuid::from_u128(8000);
            self.world.inventory.items.insert(
                id,
                aurora_net::inventory::InvItem {
                    id,
                    parent,
                    name: "Inventory Bento tail".into(),
                    owner: self.world.agent_id,
                    creator: self.world.agent_id,
                    asset_type: 6,
                    inv_type: 6,
                    owner_mask: 0x7fff_ffff,
                    base_mask: 0x7fff_ffff,
                    ..Default::default()
                },
            );
            if let Some(folder) = self.world.inventory.folders.get_mut(&parent) {
                folder.items.push(id);
            }
            self.world.inventory.generation += 1;
            if let Some(entry) = self.imports.active {
                self.choose_inventory_animation_model(entry);
                self.imports.animation_inventory.selected = Some(id);
                self.imports.animation_inventory.query = "Bento tail".into();
                self.imports.animation_inventory.demo_seeded = true;
                if self.imports.demo_mode == "animation-inventory-ready" {
                    self.prepare_inventory_animation_model(id, ctx);
                }
            }
        }
        let Some(mut pending) = self.imports.animation_inventory.pending.take() else {
            return;
        };
        if pending.checked.elapsed() < Duration::from_millis(100) {
            self.imports.animation_inventory.pending = Some(pending);
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        pending.checked = Instant::now();
        pending.cancelled |= !self
            .imports
            .entries
            .iter()
            .any(|e| e.id == pending.entry && e.animation_model.generation == pending.generation);
        if self.world.agent_id != pending.agent
            || self.world.main().is_none_or(|r| r.handle != pending.handle)
            || !self.world.movement_complete
        {
            self.finish_inventory_animation_model(pending, Some(aurora_i18n::tr!("import-animation-inventory-cleanup-warning").into()));
            return;
        }
        if let Some(at) = pending.cleaning {
            if pending.root.is_some_and(|(id, _)| self.world.objects.index_of_uuid(&id).is_none()) {
                self.finish_inventory_animation_model(pending, None);
                return;
            }
            if at.elapsed() > Duration::from_secs(15) {
                self.finish_inventory_animation_model(pending, Some(aurora_i18n::tr!("import-animation-inventory-cleanup-warning").into()));
                return;
            }
        } else {
            if pending.root.is_none() {
                let candidates: Vec<_> = self
                    .world
                    .objects
                    .iter()
                    .filter(|(_, object)| pending.matches(object, self.build.props.get(&object.full_id)))
                    .map(|(_, object)| (object.full_id, object.key))
                    .collect();
                if candidates.len() > 1 {
                    self.finish_inventory_animation_model(
                        pending,
                        Some(aurora_i18n::tr!("import-animation-inventory-identify-error").into()),
                    );
                    return;
                }
                if let Some(root) = candidates.first() {
                    pending.root = Some(*root);
                    pending.stable = Instant::now();
                } else {
                    let requests: Vec<_> = self
                        .world
                        .objects
                        .iter()
                        .filter(|(_, o)| {
                            o.key.region == pending.handle
                                && o.parent_id == 0
                                && !o.is_avatar()
                                && o.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0
                                && !pending.known.contains(&o.full_id)
                                && !pending.requested.contains(&o.full_id)
                        })
                        .take(32usize.saturating_sub(pending.requested.len()))
                        .map(|(_, o)| (o.full_id, o.key.local_id))
                        .collect();
                    if !requests.is_empty() {
                        pending.requested.extend(requests.iter().map(|(id, _)| *id));
                        self.send(NetCommand::Build(BuildCmd::Select {
                            handle: pending.handle,
                            local_ids: requests.into_iter().map(|(_, local)| local).collect(),
                        }));
                    }
                    if pending.started.elapsed() > Duration::from_secs(30) {
                        self.finish_inventory_animation_model(
                            pending,
                            Some(aurora_i18n::tr!("import-animation-inventory-identify-error").into()),
                        );
                        return;
                    }
                }
            }
            if let Some((root, _)) = pending.root {
                if self.world.objects.index_of_uuid(&root).is_none() {
                    pending.result = Some(Err(aurora_i18n::tr!("import-animation-inventory-gone").into()));
                    self.finish_inventory_animation_model(pending, None);
                    return;
                }
                if let Some(worker) = &pending.worker {
                    match worker.try_recv() {
                        Ok(result) => {
                            pending.result = Some(result);
                            pending.worker = None;
                        }
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            pending.result = Some(Err(aurora_i18n::tr!("import-animation-inventory-cache-error").into()));
                            pending.worker = None;
                        }
                        Err(crossbeam_channel::TryRecvError::Empty) => {}
                    }
                }
                if pending.result.is_none() && pending.worker.is_none() && !pending.cancelled {
                    // Wait for linkset membership to settle before snapshotting.
                    let mut members = vec![root];
                    let mut cursor = 0;
                    while cursor < members.len() && members.len() <= 256 {
                        if let Some(object) = self
                            .world
                            .objects
                            .index_of_uuid(&members[cursor])
                            .and_then(|i| self.world.objects.get(i))
                        {
                            members.extend(
                                self.world
                                    .objects
                                    .children_of(&object.key)
                                    .iter()
                                    .filter_map(|i| self.world.objects.get(*i))
                                    .map(|o| o.full_id),
                            );
                        }
                        cursor += 1;
                    }
                    if members != pending.members {
                        pending.members = members;
                        pending.stable = Instant::now();
                    }
                    if pending.members.len() > 256 {
                        pending.result = Some(Err(aurora_i18n::tr!("import-animation-inventory-large").into()));
                    } else if pending.stable.elapsed() > Duration::from_millis(1200) {
                        match inventory::geometry::capture(&self.world, &mut self.scene, root, &mut pending.textures) {
                            Ok(Some(input)) => {
                                let (tx, rx) = crossbeam_channel::bounded(1);
                                pending.worker = Some(rx);
                                let avatars = self.scene.avatar_lib.clone();
                                let path = pending.saved.path.clone();
                                let wake = ctx.clone();
                                self.scene.jobs.execute(move || {
                                    let result = input.prepare(path, &avatars).and_then(|model| {
                                        inventory::snapshot::save(&model, &avatars)?;
                                        Ok(model)
                                    });
                                    let _ = tx.send(result);
                                    wake.request_repaint();
                                });
                            }
                            Ok(None) => {}
                            Err(error) => pending.result = Some(Err(error)),
                        }
                    }
                }
                if pending.started.elapsed() > Duration::from_secs(90) && pending.result.is_none() {
                    pending.result = Some(Err(aurora_i18n::tr!("import-animation-inventory-timeout").into()));
                }
                if (pending.result.is_some() || pending.cancelled) && pending.worker.is_none() {
                    if let Some(command) = pending.cleanup(&self.world) {
                        pending.cleaning = Some(Instant::now());
                        self.send(NetCommand::Build(command));
                    } else {
                        self.finish_inventory_animation_model(
                            pending,
                            Some(aurora_i18n::tr!("import-animation-inventory-cleanup-warning").into()),
                        );
                        return;
                    }
                }
            }
        }
        self.imports.animation_inventory.pending = Some(pending);
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    fn finish_inventory_animation_model(&mut self, mut pending: Pending, warning: Option<String>) {
        for id in &pending.textures {
            self.scene.textures.want_model_preview(*id, false);
        }
        pending.cancelled |= pending.agent != self.world.agent_id
            || pending
                .saved
                .inventory
                .as_ref()
                .is_some_and(|origin| origin.profile != self.imports.animation_inventory.profile);
        let result = pending.result.take();
        if self.demo {
            log::info!(
                "demo animation inventory: snapshot_saved={} cleanup_confirmed={} cancelled={}",
                result.as_ref().is_some_and(|r| r.is_ok()),
                warning.is_none(),
                pending.cancelled
            );
        }
        let error = warning
            .clone()
            .or_else(|| result.as_ref().and_then(|result| result.as_ref().err()).cloned());
        if !pending.cancelled
            && let Some(result) = result
        {
            self.imports
                .complete_inventory_model(pending.entry, pending.generation, pending.saved, result);
        }
        if let Some(entry) = self
            .imports
            .entries
            .iter_mut()
            .find(|e| e.id == pending.entry && e.animation_model.generation == pending.generation)
        {
            entry.animation_model.pending = false;
            if let Some(error) = &error {
                entry.animation_model.error = Some(error.clone());
            }
        }
        self.imports.animation_inventory.error = error.clone();
        if warning.is_some()
            && let Some(error) = error
        {
            self.world.system_notice(&error, crate::world::notice_topics::Topic::Warning);
        }
        if self.imports.animation_inventory.error.is_none() {
            self.imports.animation_inventory.entry = None;
        }
    }
}
