//! Application bridge for the importer, including local previews and the
//! entirely offline demo server.

use super::*;
use crate::imports::{Playback, Source, ui::Action};
use aurora_net::inventory::upload::{Prices, Receipt};

const SOUND_PREVIEW: u64 = u64::MAX - 10;

impl App {
    pub(super) fn show_imports(&mut self, ctx: &egui::Context, palette: &Palette) {
        self.imports.load_preferences = self.settings.imports.clone();
        if self.demo && !self.imports.demo_started {
            self.world.balance = Some(1000);
            self.imports.demo_started = true;
            crate::imports::demo::start(&mut self.imports, &self.scene.jobs, ctx);
        }
        self.imports.sound_limit = if self.settings.grid == crate::settings::GridChoice::Custom {
            60
        } else {
            30
        };
        for entry in &mut self.imports.entries {
            if let Some(Source::Mesh(draft)) = &mut entry.source {
                if !self.imports.confirm
                    && !matches!(entry.status, crate::imports::Status::Uploading)
                    && draft.quoted_at.is_some_and(|at| at.elapsed().as_secs() >= 300)
                {
                    draft.invalidate_quote();
                }
                self.imports.discarded_quotes.append(&mut draft.discarded_quotes);
            }
        }
        let discarded: Vec<_> = std::mem::take(&mut self.imports.discarded_quotes);
        for token in discarded {
            self.send(NetCommand::DiscardMeshQuote(token));
        }
        let commands = self.imports.poll(ctx, &self.scene.jobs, &self.world, &self.settings.inventory);
        for command in commands {
            self.send(command);
        }
        if self.demo
            && self.imports.demo_mode == "bulk-upload"
            && !self.imports.demo_upload_started
            && self.imports.pending_files.is_none()
            && !self.imports.entries.is_empty()
        {
            self.imports.demo_upload_started = true;
            self.imports.start(
                &self.world,
                &self.settings.inventory,
                self.settings.imports.permissions(),
                &Prices {
                    texture: Some(10),
                    large_texture: Some(40),
                    sound: Some(10),
                    animation: Some(10),
                },
            );
        }
        self.apply_local_imports();
        self.poll_local_mesh_rez(ctx);
        if self.demo
            && self.imports.demo_mode == "local-mesh-applied"
            && !self.imports.demo_local_applied
            && let Some(id) = self.imports.active
            && let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(970))
            && let Some(object) = self.world.objects.get(idx)
        {
            let target = object.full_id;
            let key = object.key;
            self.build.selection = vec![key];
            if let Some(entry) = self.imports.entries.iter_mut().find(|e| e.id == id) {
                entry.local_use_scale = true;
            }
            self.imports.local_targets.insert(
                target,
                crate::imports::local::Target {
                    entry: id,
                    model: 0,
                    use_scale: true,
                },
            );
            self.imports.demo_local_applied = true;
            if let Some(gfx) = &self.gfx {
                self.imports.publish_local(id, &self.scene.jobs, ctx, gfx.renderer.staging_pool());
            }
        }
        self.imports.watch_local(&self.scene.jobs, ctx);
        for Playback::Audio(clip) in self.imports.playback.drain(..) {
            if let Some(engine) = &self.audio_engine {
                engine.play_world(SOUND_PREVIEW, aurora_audio::Channel::Ui, &clip, false, (1.0, 1.0));
            }
        }
        // Seed the scripted preview before tracking user preference changes.
        if self.demo && self.imports.demo_mode == "animation-own" && !self.imports.demo_animation_positioned {
            self.settings.imports.preview_on_avatar = true;
        }
        let before = self.settings.imports.clone();
        if self.imports.open {
            if let Some(id) = self.imports.active.filter(|id| {
                self.imports
                    .entries
                    .iter()
                    .any(|e| e.id == *id && matches!(e.source, Some(Source::Animation { .. })) && !e.player.started)
            }) {
                self.imports.play(id, &self.scene.jobs, ctx, self.scene.avatar_lib.rig.clone());
            }
            for entry in &mut self.imports.entries {
                if self.imports.active == Some(entry.id) && matches!(entry.source, Some(Source::Animation { .. })) {
                    if self.demo
                        && self.imports.demo_mode == "animation-own"
                        && !self.imports.demo_animation_positioned
                        && entry.player.bound.is_some()
                    {
                        entry.player.seek(0.75);
                        entry.player.playing = false;
                        self.imports.demo_animation_positioned = true;
                    }
                    entry.player.tick(entry.id, &self.scene.avatar_lib.rig, &mut self.scene.anims);
                    if entry.player.playing {
                        ctx.request_repaint();
                    }
                }
            }
        }
        let was_open = self.imports.open;
        let actions = crate::imports::ui::show(
            ctx,
            palette,
            &mut self.imports,
            &mut self.settings.imports,
            &self.world,
            &self.settings.inventory,
        );
        if self.settings.imports != before {
            self.settings.save();
        }
        if was_open && !self.imports.open {
            if let Some(request) = self.imports.play_request.take() {
                self.world.preview_animation(request, false);
            }
            if let Some(engine) = &self.audio_engine {
                engine.stop_world(SOUND_PREVIEW);
            }
            for entry in &mut self.imports.entries {
                entry.player.playing = false;
            }
        }
        for action in actions {
            match action {
                Action::Files => self.imports.choose_files(&self.scene.jobs, ctx, self.settings.imports.texture()),
                Action::Start => {
                    let prices = if self.demo {
                        Prices {
                            texture: Some(10),
                            large_texture: Some(40),
                            sound: Some(10),
                            animation: Some(10),
                        }
                    } else {
                        self.world.login.as_ref().map(|l| Prices::from_login(&l.raw)).unwrap_or_default()
                    };
                    let prices = prices.with_fallback(self.world.upload_cost);
                    self.imports
                        .start(&self.world, &self.settings.inventory, self.settings.imports.permissions(), &prices);
                }
                Action::Cancel => self.imports.cancel_remaining(),
                Action::Preview(id) => {
                    if self
                        .imports
                        .entries
                        .iter()
                        .find(|e| e.id == id)
                        .is_some_and(|e| e.texture_target != 0)
                    {
                        self.imports.request_model(id, &self.scene.jobs, ctx, self.scene.avatar_lib.clone());
                    } else {
                        self.imports.request_preview(id, &self.scene.jobs, ctx);
                    }
                }
                Action::Play(id, start) => {
                    if let Some(previous) = self.imports.play_request.take() {
                        self.world.preview_animation(previous, false);
                    }
                    if let Some(engine) = &self.audio_engine {
                        engine.stop_world(SOUND_PREVIEW);
                    }
                    if start {
                        self.imports.play(id, &self.scene.jobs, ctx, self.scene.avatar_lib.rig.clone());
                    }
                }
                Action::Reload(id) => self.imports.reload(id, &self.scene.jobs, ctx),
                Action::RestoreDraft(id) => self.imports.restore_draft(id),
                Action::ResetMesh(id) => {
                    if let Some(entry) = self.imports.entries.iter_mut().find(|e| e.id == id) {
                        entry.view = Default::default();
                    }
                    self.imports.reload(id, &self.scene.jobs, ctx);
                }
                Action::Quote(id) => self.imports.quote_mesh(
                    id,
                    ctx,
                    &self.scene.jobs,
                    &self.world,
                    &self.settings.inventory,
                    self.settings.imports.permissions(),
                ),
                Action::MeshFile(id, level) => self.imports.mesh_file(id, level, &self.scene.jobs, ctx),
                Action::PhysicsPreset(id, save) => {
                    if save {
                        if let Some(Source::Mesh(draft)) = self.imports.entries.iter().find(|e| e.id == id).and_then(|e| e.source.as_ref())
                            && let Some(path) = &draft.physics_path
                        {
                            self.settings.imports.mesh_physics_preset = path.to_string_lossy().into_owned();
                            self.settings.save();
                        }
                    } else {
                        self.imports
                            .mesh_file_path(id, self.settings.imports.mesh_physics_preset.clone().into(), &self.scene.jobs, ctx);
                    }
                }
                Action::Open(id) => self.inventory_ui.show_original(&self.world.inventory, id),
                Action::Local(id) => {
                    if let Some(entry) = self.imports.entries.iter().find(|e| e.id == id)
                        && let Some(Source::Mesh(draft)) = &entry.source
                    {
                        let model = draft.scene.instances.get(entry.local_model).map(|i| i.model).unwrap_or(0);
                        let targets: Vec<_> = self
                            .build
                            .selection
                            .iter()
                            .filter_map(|key| self.world.objects.index_of(key))
                            .filter_map(|idx| self.world.objects.get(idx))
                            .filter(|o| o.pcode == aurora_prim::params::LL_PCODE_VOLUME)
                            .map(|o| o.full_id)
                            .collect();
                        if targets.is_empty() {
                            self.imports.message = aurora_i18n::tr!("import-local-mesh-selection").into();
                            continue;
                        }
                        for target in targets {
                            self.imports.local_targets.insert(
                                target,
                                crate::imports::local::Target {
                                    entry: id,
                                    model,
                                    use_scale: entry.local_use_scale,
                                },
                            );
                        }
                    }
                    self.apply_local_mesh_targets();
                    if self.imports.entries.iter().find(|e| e.id == id).is_some_and(|e| {
                        matches!(e.source, Some(Source::Mesh(_)))
                            && !e.local_mesh_ids.is_empty()
                            && e.local_generation == e.preview_generation
                    }) {
                        continue;
                    }
                    if let Some(gfx) = &self.gfx {
                        self.imports.publish_local(id, &self.scene.jobs, ctx, gfx.renderer.staging_pool());
                    }
                }
                Action::ClearLocalMesh => {
                    let ids: Vec<_> = self
                        .build
                        .selection
                        .iter()
                        .filter_map(|key| self.world.objects.index_of(key))
                        .filter_map(|idx| self.world.objects.get(idx))
                        .map(|o| o.full_id)
                        .collect();
                    self.clear_local_mesh_targets(&ids);
                }
                Action::RezLocalMesh(id) => {
                    if let Some(Source::Mesh(draft)) = self.imports.entries.iter().find(|e| e.id == id).and_then(|e| e.source.as_ref()) {
                        let model = draft
                            .scene
                            .instances
                            .get(self.imports.entries.iter().find(|e| e.id == id).map_or(0, |e| e.local_model))
                            .map_or(0, |i| i.model);
                        self.imports.local_rez = Some(crate::imports::local::Rez {
                            entry: id,
                            model,
                            known: self
                                .world
                                .objects
                                .iter()
                                .filter(|(_, o)| {
                                    o.owner_id == self.world.agent_id && o.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0
                                })
                                .map(|(_, o)| o.full_id)
                                .collect(),
                        });
                        self.build.open_build(crate::build::Tool::Create);
                        self.imports.message = aurora_i18n::tr!("import-local-mesh-rez-hint").into();
                    }
                }
                Action::MapFile(id, slot) => self.imports.map_file(id, slot, &self.scene.jobs, ctx),
                Action::Resume => {
                    for entry in &mut self.imports.entries {
                        if matches!(entry.status, crate::imports::Status::Cancelled) {
                            entry.status = crate::imports::Status::Ready;
                        }
                    }
                }
                Action::Remove(id) => {
                    let targets: Vec<_> = self
                        .imports
                        .local_targets
                        .iter()
                        .filter(|(_, target)| target.entry == id)
                        .map(|(id, _)| *id)
                        .collect();
                    self.clear_local_mesh_targets(&targets);
                    if let Some(index) = self.imports.entries.iter().position(|e| e.id == id) {
                        let mut entry = self.imports.entries.remove(index);
                        if let Some(Source::Mesh(draft)) = &mut entry.source {
                            draft.invalidate();
                            self.imports.discarded_quotes.append(&mut draft.discarded_quotes);
                        }
                        self.world.local_assets.remove(&id);
                        self.ui_images.remove(&id);
                        if entry.local_published {
                            self.scene.materials.remove_local(&id);
                        }
                        for asset in &entry.local_mesh_ids {
                            self.scene.meshes.remove_local(asset);
                        }
                        if let Some(gfx) = &mut self.gfx {
                            for (asset, slot) in &entry.local_slots {
                                if !self.imports.entries.iter().any(|e| e.local_slots.contains_key(asset)) {
                                    self.scene.textures.unregister_local(asset);
                                    gfx.renderer.free_texture(*slot);
                                }
                            }
                        }
                        self.scene.jobs.discard(entry);
                        self.imports.active = self.imports.entries.iter().find(|e| self.imports.choice.accepts(e)).map(|e| e.id);
                    }
                }
                Action::MapInventory(id, slot) => {
                    let asset = self
                        .imports
                        .entries
                        .iter()
                        .find(|e| e.id == id)
                        .and_then(|e| match &e.source {
                            Some(Source::Material { value, .. }) => crate::imports::maps::ids(value)[slot],
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.imports.map_target = Some((id, slot));
                    self.imports
                        .map_picker
                        .open_with_defaults(aurora_i18n::tr!("import-texture"), asset, None, true);
                }
            }
        }
        self.update_import_avatar_preview();
        if let Some(asset) = self.imports.map_picker.show(ctx, palette, &self.world, &self.ui_images)
            && let Some((id, slot)) = self.imports.map_target.take()
        {
            self.imports.select_map(id, slot, asset);
        }
        let mut maps: Vec<_> = self.imports.map_picker.wanted_images.drain().collect();
        if let Some(entry) = self.imports.active.and_then(|id| self.imports.entries.iter().find(|e| e.id == id))
            && let Some(Source::Material { value, images }) = &entry.source
        {
            maps.extend(
                crate::imports::maps::ids(value)
                    .into_iter()
                    .enumerate()
                    .filter_map(|(slot, id)| if images[slot].is_none() { id } else { None }),
            );
        }
        if let Some(gfx) = &mut self.gfx {
            for asset in maps {
                self.scene.textures.want_ui_image(&mut gfx.renderer, asset);
                self.scene.textures.want_sculpt(&asset);
                if let Some(pixels) = self.scene.textures.sculpt_map(&asset) {
                    self.imports.request_remote_map(asset, pixels, &self.scene.jobs, ctx);
                }
            }
        }
        let missing: Vec<_> = self
            .imports
            .entries
            .iter()
            .filter(|e| {
                self.imports.active == Some(e.id)
                    && e.preview.is_none()
                    && (!e.preview_pending || e.preview_job_generation != e.preview_generation)
                    && e.preview_error.is_none()
                    && e.texture_target == 0
                    && matches!(e.source, Some(Source::Texture(_)))
            })
            .map(|e| e.id)
            .collect();
        for id in missing {
            self.imports.request_preview(id, &self.scene.jobs, ctx);
        }
        let models: Vec<_> = self
            .imports
            .entries
            .iter()
            .filter(|e| {
                self.imports.active == Some(e.id)
                    && e.model.is_none()
                    && (!e.preview_pending || e.preview_job_generation != e.preview_generation)
                    && e.preview_error.is_none()
                    && (matches!(e.source, Some(Source::Material { .. } | Source::Mesh(_) | Source::Animation { .. }))
                        || (e.texture_target != 0 && matches!(e.source, Some(Source::Texture(_)))))
            })
            .map(|e| e.id)
            .collect();
        for id in models {
            self.imports.request_model(id, &self.scene.jobs, ctx, self.scene.avatar_lib.clone());
        }
        let locals: Vec<_> = self
            .imports
            .entries
            .iter()
            .filter(|e| {
                e.local_published
                    && !e.local_pending
                    && e.local_generation != e.preview_generation
                    && e.local_failed_generation != Some(e.preview_generation)
            })
            .map(|e| e.id)
            .collect();
        if let Some(gfx) = &self.gfx {
            for id in locals {
                self.imports.publish_local(id, &self.scene.jobs, ctx, gfx.renderer.staging_pool());
            }
        }
        for entry in self.imports.entries.iter().filter(|e| e.local_published) {
            if let Some(image) = &entry.preview {
                self.ui_images.insert(entry.id, image.clone());
            }
        }
    }

    fn update_import_avatar_preview(&mut self) {
        let desired = self
            .imports
            .entries
            .iter()
            .find(|e| {
                self.imports.open
                    && self.imports.active == Some(e.id)
                    && self.settings.imports.preview_on_avatar
                    && e.player.request.is_some()
                    && e.player.request == self.imports.play_request
            })
            .and_then(|e| e.player.bound.as_ref().map(|bound| (e.id, bound.clone(), e.player.clock())));
        if self.imports.own_avatar_preview != desired.as_ref().map(|(id, _, _)| *id) {
            if let Some(id) = self.imports.own_avatar_preview.take() {
                self.world.preview_animation(id, false);
                if let Some(bound) = self.scene.anims.remove_local(&id) {
                    self.scene.jobs.discard(bound);
                }
            }
            if let Some((id, bound, _)) = &desired {
                self.scene.anims.insert(*id, bound.clone());
                self.world.preview_animation(*id, true);
                self.imports.own_avatar_preview = Some(*id);
            }
        }
        if let Some((id, bound, _)) = &desired
            && self.imports.own_avatar_preview == Some(*id)
            && !self.scene.anims.get(id).is_some_and(|current| Arc::ptr_eq(&current, bound))
        {
            self.scene.anims.insert(*id, bound.clone());
        }
        if let Some((id, _, (start, sample))) = desired {
            self.world.set_animation_preview_clock(id, start, sample);
        }
    }

    fn poll_local_mesh_rez(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.imports.local_rez.as_ref() else { return };
        if self.imports.entries.iter().all(|e| e.id != pending.entry) {
            self.imports.local_rez = None;
            return;
        }
        let target = self
            .build
            .selection
            .iter()
            .filter_map(|key| self.world.objects.index_of(key))
            .filter_map(|index| self.world.objects.get(index))
            .find(|object| {
                object.pcode == aurora_prim::params::LL_PCODE_VOLUME
                    && object.owner_id == self.world.agent_id
                    && object.parent_id == 0
                    && object.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0
                    && !pending.known.contains(&object.full_id)
            })
            .map(|object| object.full_id);
        let Some(target) = target else {
            if !self.build.open || self.build.tool != crate::build::Tool::Create {
                self.imports.local_rez = None;
                self.imports.message.clear();
            }
            return;
        };
        let pending = self.imports.local_rez.take().expect("pending rez");
        self.imports.local_targets.insert(
            target,
            crate::imports::local::Target {
                entry: pending.entry,
                model: pending.model,
                use_scale: true,
            },
        );
        if self
            .imports
            .entries
            .iter()
            .find(|e| e.id == pending.entry)
            .is_some_and(|e| e.local_mesh_ids.is_empty())
        {
            if let Some(gfx) = &self.gfx {
                self.imports
                    .publish_local(pending.entry, &self.scene.jobs, ctx, gfx.renderer.staging_pool());
            }
        } else {
            self.apply_local_mesh_targets();
        }
        self.imports.message.clear();
    }
    fn apply_local_mesh_targets(&mut self) {
        let mut invalid = Vec::new();
        for (id, target) in &self.imports.local_targets {
            let Some(entry) = self.imports.entries.iter().find(|e| e.id == target.entry) else {
                continue;
            };
            let Some(asset) = entry.local_mesh_ids.get(target.model).copied() else {
                if !entry.local_mesh_ids.is_empty() {
                    invalid.push(*id);
                }
                continue;
            };
            let scale = if target.use_scale {
                if let Some(Source::Mesh(draft)) = &entry.source {
                    draft
                        .scene
                        .instances
                        .iter()
                        .find(|i| i.model == target.model)
                        .map(|i| i.transform.to_scale_rotation_translation().0 * draft.options.scale)
                } else {
                    None
                }
            } else {
                None
            };
            let Some(idx) = self.world.objects.index_of_uuid(id) else {
                continue;
            };
            if self.world.objects.get(idx).is_some_and(|object| {
                object
                    .render
                    .local_volume
                    .as_deref()
                    .and_then(|v| v.sculpt)
                    .is_some_and(|s| s.texture == asset)
                    && object.render.local_scale == scale
            }) {
                continue;
            }
            let Some(object) = self.world.objects.get_mut(idx) else { continue };
            let mut volume = object.volume;
            volume.sculpt = Some(aurora_prim::params::SculptParams {
                texture: asset,
                sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
            });
            object.render.local_volume = Some(Box::new(volume));
            object.render.local_scale = scale;
            object.shape_dirty = true;
            object.render.needs_records = true;
        }
        self.clear_local_mesh_targets(&invalid);
    }
    fn clear_local_mesh_targets(&mut self, ids: &[uuid::Uuid]) {
        for id in ids {
            self.imports.local_targets.remove(id);
            if let Some(idx) = self.world.objects.index_of_uuid(id)
                && let Some(object) = self.world.objects.get_mut(idx)
            {
                object.render.local_volume = None;
                object.render.local_scale = None;
                object.shape_dirty = true;
                object.render.needs_records = true;
            }
        }
    }
    fn apply_local_imports(&mut self) {
        if self.imports.local_uploads.is_empty() {
            return;
        }
        let Some(gfx) = &mut self.gfx else {
            return;
        };
        let mut pending = Vec::new();
        let mut mesh_changed = false;
        let previous_slots: std::collections::HashMap<_, _> = self
            .imports
            .entries
            .iter()
            .flat_map(|e| e.local_slots.iter().map(|(id, slot)| (*id, *slot)))
            .collect();
        let mut shared_slots = previous_slots.clone();
        for prepared in self.imports.local_uploads.drain(..) {
            if prepared.textures.iter().any(|t| !t.staged.data.ready()) {
                pending.push(prepared);
                continue;
            }
            let Some(entry) = self
                .imports
                .entries
                .iter_mut()
                .find(|e| e.id == prepared.entry && e.preview_generation == prepared.generation)
            else {
                self.scene.jobs.discard(prepared);
                continue;
            };
            let mut complete = true;
            let retained: std::collections::HashSet<_> = prepared.textures.iter().map(|t| t.id).collect();
            for texture in prepared.textures {
                let slot = shared_slots
                    .get(&texture.id)
                    .copied()
                    .or_else(|| gfx.renderer.textures.create_placeholder());
                if let Some(slot) = slot
                    && gfx.renderer.replace_texture_staged(slot, &texture.staged)
                {
                    entry.local_slots.insert(texture.id, slot);
                    shared_slots.insert(texture.id, slot);
                    self.scene
                        .textures
                        .register_local_image(texture.id, slot, texture.alpha, texture.has_alpha);
                } else {
                    if !shared_slots.contains_key(&texture.id)
                        && let Some(slot) = slot
                    {
                        gfx.renderer.free_texture(slot);
                    }
                    complete = false;
                }
            }
            if complete {
                entry.local_slots.retain(|id, _| retained.contains(id));
                let material = prepared.material.is_some();
                if let Some(value) = prepared.material {
                    self.scene.materials.insert(entry.id, value);
                }
                if prepared.meshes.is_empty() {
                    self.world.local_assets.insert(entry.id, (entry.name.clone(), material));
                } else {
                    for id in entry.local_mesh_ids.drain(..) {
                        self.scene.meshes.remove_local(&id);
                    }
                    for mesh in prepared.meshes {
                        entry.local_mesh_ids.push(mesh.id);
                        self.scene.meshes.insert_local(mesh.id, mesh.data, mesh.header, mesh.skin);
                    }
                    mesh_changed = true;
                }
                entry.local_published = true;
                entry.local_generation = prepared.generation;
            } else {
                entry.local_failed_generation = Some(prepared.generation);
                self.imports.message = aurora_i18n::tr!("import-local-staging-busy").into();
            }
            entry.local_pending = false;
        }
        self.imports.local_uploads = pending;
        let referenced: std::collections::HashSet<_> = self.imports.entries.iter().flat_map(|e| e.local_slots.keys().copied()).collect();
        for (asset, slot) in previous_slots {
            if !referenced.contains(&asset) {
                self.scene.textures.unregister_local(&asset);
                gfx.renderer.free_texture(slot);
            }
        }
        if mesh_changed {
            self.apply_local_mesh_targets();
        }
    }

    pub(super) fn demo_asset_upload(&mut self, request: uuid::Uuid, upload: aurora_net::inventory::upload::Request) {
        let result = upload.body().map(|_| {
            let item = aurora_net::inventory::InvItem {
                id: uuid::Uuid::new_v4(),
                asset_id: uuid::Uuid::new_v4(),
                parent: upload.folder,
                name: upload.name,
                desc: upload.description,
                asset_type: upload.kind.asset_type(),
                inv_type: match upload.kind {
                    aurora_net::inventory::upload::Kind::Texture => 0,
                    aurora_net::inventory::upload::Kind::Sound => 1,
                    aurora_net::inventory::upload::Kind::Animation => 19,
                    aurora_net::inventory::upload::Kind::Material => 26,
                    aurora_net::inventory::upload::Kind::Mesh => 6,
                },
                owner: self.world.agent_id,
                creator: self.world.agent_id,
                base_mask: 0x7fffffff,
                owner_mask: 0x7fffffff,
                next_owner_mask: upload.permissions.next_owner,
                group_mask: upload.permissions.group,
                everyone_mask: upload.permissions.everyone,
                ..Default::default()
            };
            Receipt {
                item,
                cost: upload.expected_cost,
            }
        });
        if let Ok(receipt) = &result
            && let Some(balance) = &mut self.world.balance
        {
            *balance -= receipt.cost;
        }
        self.on_app_event(NetEvent::AssetUploaded { request, result });
    }
}
