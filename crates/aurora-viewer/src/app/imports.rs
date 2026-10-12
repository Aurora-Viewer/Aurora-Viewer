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
        self.apply_local_imports();
        self.imports.watch_local(&self.scene.jobs, ctx);
        for playback in self.imports.playback.drain(..) {
            match playback {
                Playback::Audio(clip) => {
                    if let Some(engine) = &self.audio_engine {
                        engine.play_world(SOUND_PREVIEW, aurora_audio::Channel::Ui, &clip, false, (1.0, 1.0));
                    }
                }
                Playback::Animation(id, animation) => {
                    self.scene.anims.insert(id, animation);
                    self.world.preview_animation(id, true);
                }
            }
        }
        let before = self.settings.imports.clone();
        if self.imports.open {
            for entry in &mut self.imports.entries {
                if self.imports.active == Some(entry.id) && matches!(entry.source, Some(Source::Animation { .. })) {
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
                    if let Some(gfx) = &self.gfx {
                        self.imports.publish_local(id, &self.scene.jobs, ctx, gfx.renderer.staging_pool());
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
            .filter(|e| e.local_published && !e.local_pending && e.local_generation != e.preview_generation)
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

    fn apply_local_imports(&mut self) {
        let Some(gfx) = &mut self.gfx else {
            return;
        };
        let mut pending = Vec::new();
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
                    complete = false;
                }
            }
            if complete {
                entry.local_slots.retain(|id, _| retained.contains(id));
                let material = prepared.material.is_some();
                if let Some(value) = prepared.material {
                    self.scene.materials.insert(entry.id, value);
                }
                self.world.local_assets.insert(entry.id, (entry.name.clone(), material));
                entry.local_published = true;
                entry.local_generation = prepared.generation;
            } else {
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
