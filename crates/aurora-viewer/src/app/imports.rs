//! Application bridge for the importer, including local previews and the
//! entirely offline demo server.

use super::*;
use crate::imports::{Playback, Source, ui::Action};
use aurora_net::inventory::upload::{Prices, Receipt};

const SOUND_PREVIEW: u64 = u64::MAX - 10;

impl App {
    pub(super) fn show_imports(&mut self, ctx: &egui::Context, palette: &Palette) {
        if self.demo && !self.imports.demo_started {
            self.world.balance = Some(1000);
            self.imports.demo_started = true;
            self.imports.load_preferences = self.settings.imports.clone();
            crate::imports::demo::start(&mut self.imports, &self.scene.jobs, ctx);
            if self.imports.demo_mode.starts_with("folders") {
                self.settings.imports.use_default_folders = false;
                self.settings.imports.folder_layout = Some(self.imports.folders.layout);
                self.settings.imports.batch_folder = self.world.inventory.root;
            }
            if self.imports.demo_mode.starts_with("mesh") {
                // Offline previews use the existing synthetic idle; normal
                // operation loads the standard asset through AnimStreamer.
                let rig = &self.scene.avatar_lib.rig;
                self.scene.anims.insert(
                    crate::own_motion::ANIM_STAND,
                    std::sync::Arc::new(crate::scene::anim::BoundAnim::bind(crate::demo::idle_animation(), rig)),
                );
            }
            if self.imports.demo_mode == "bulk-mesh-denied" {
                self.imports.mesh_permission_account = self.world.agent_id;
            }
        }
        self.imports
            .sync_destinations(&self.settings.imports, &self.world, &self.settings.inventory);
        if !self.imports.persistence.started() {
            let persistence_path = if self.demo && !self.imports.demo_mode.is_empty() {
                std::env::var_os("AURORA_DEMO_IMPORT_REGISTRY").map(std::path::PathBuf::from)
            } else {
                Some(crate::imports::Imports::registry_path())
            };
            if let Some(path) = persistence_path {
                self.imports.start_local_registry(
                    path,
                    crate::imports::persistence::profile(self.world.agent_id, &self.settings.login_uri()),
                    &self.scene.jobs,
                    ctx,
                );
            } else {
                self.imports.persistence.disable();
            }
        }
        self.imports.poll_local_registry();
        self.sync_local_picker_bindings();
        let expired = self.imports.expire_local_appearances(&mut self.world.objects);
        self.build
            .forget_expired_local_picker_appearances(&expired, &self.imports.local_appearance_targets);
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
        self.save_import_animation_models();
        for command in commands {
            self.send(command);
        }
        if self.demo && self.imports.demo_mode == "folders-lods" && !self.imports.entries.is_empty() {
            let key = egui::Id::unique("import_demo_lod_family_shown");
            if !ctx.data(|data| data.get_temp::<bool>(key)).unwrap_or(false) {
                self.imports.active = self
                    .imports
                    .entries
                    .iter()
                    .find(|entry| matches!(entry.source, Some(Source::Mesh(_))))
                    .map(|entry| entry.id);
                self.imports.tab = crate::imports::ui::Tab::Batch;
                ctx.data_mut(|data| data.insert_temp(key, true));
            }
        }
        for (id, rights) in self.imports.advance_confirmed(&self.world, &self.settings.inventory) {
            self.imports
                .quote_mesh(id, ctx, &self.scene.jobs, &self.world, &self.settings.inventory, rights);
        }
        if self.demo
            && matches!(self.imports.demo_mode.as_str(), "bulk-upload" | "folders-upload")
            && !self.imports.demo_upload_started
            && self.imports.pending_files.is_none()
            && !self.imports.entries.is_empty()
            && self.imports.folder_plan_valid(&self.world, &self.settings.inventory)
        {
            let missing_quotes: Vec<_> = self
                .imports
                .entries
                .iter()
                .filter_map(|entry| {
                    if let Some(Source::Mesh(draft)) = &entry.source
                        && draft.quote.is_none()
                        && draft.pending.is_none()
                        && matches!(entry.status, crate::imports::Status::Ready)
                    {
                        Some(entry.id)
                    } else {
                        None
                    }
                })
                .collect();
            for id in missing_quotes {
                self.imports.quote_mesh(
                    id,
                    ctx,
                    &self.scene.jobs,
                    &self.world,
                    &self.settings.inventory,
                    self.settings.imports.permissions_for(aurora_net::inventory::upload::Kind::Mesh),
                );
            }
            let prices = Prices {
                texture: Some(10),
                large_texture: Some(40),
                sound: Some(10),
                animation: Some(10),
            };
            if self.imports.total(&prices).is_some() {
                self.imports.demo_upload_started = true;
                self.imports
                    .start_confirmed(&self.world, &self.settings.inventory, &self.settings.imports, &prices);
            }
        }
        let local_demo = self.demo
            && self.imports.demo_mode.starts_with("local-")
            && ["-applied", "-removed", "-rez", "-restored"]
                .iter()
                .any(|suffix| self.imports.demo_mode.ends_with(suffix));
        if local_demo && self.imports.demo_mode.ends_with("-removed") && self.frame_count == 400 {
            self.clear_local_mesh_targets(&[crate::demo::action_id(970)]);
            self.imports
                .clear_appearance_targets(&mut self.world.objects, &[crate::demo::action_id(970)], None);
        }
        self.poll_local_rez(ctx);
        if local_demo
            && self.imports.demo_mode != "local-restored"
            && !self.imports.demo_local_applied
            && let Some(id) = self.imports.active
            && let Some(index) = self.world.objects.index_of_uuid(&crate::demo::action_id(970))
            && let Some(object) = self.world.objects.get(index)
        {
            let target = object.full_id;
            let key = object.key;
            self.build.selection = vec![key];
            if let Some(entry) = self.imports.entries.iter_mut().find(|entry| entry.id == id) {
                entry.local_use_scale = true;
            }
            if self.imports.demo_mode.ends_with("-rez") {
                self.build.open_build(crate::build::Tool::Create);
                self.imports.local_rez = Some(crate::imports::local::Rez {
                    entry: id,
                    known: Default::default(),
                });
                if let Some(object) = self.world.objects.get_mut(index) {
                    object.owner_id = self.world.agent_id;
                    object.update_flags |= crate::build::FLAGS_CREATE_SELECTED;
                }
            } else {
                self.imports.target_local(id, target, &[], true);
                self.publish_local_import(id, ctx);
            }
            self.imports.demo_local_applied = true;
        }
        if local_demo
            && self.frame_count.is_multiple_of(120)
            && let Some(entry) = self
                .imports
                .active
                .and_then(|id| self.imports.entries.iter().find(|entry| entry.id == id))
            && let Some(index) = self.world.objects.index_of_uuid(&crate::demo::action_id(970))
            && let Some(object) = self.world.objects.get(index)
        {
            let mesh = entry.local_mesh_ids.first().copied();
            let matches_mesh = |key| matches!(key, Some(crate::scene::GeomKey::Mesh { id, .. }) if Some(id) == mesh);
            let gpu = self.scene.gpu.get(index);
            let texture = gpu.is_some_and(|gpu| gpu.faces.iter().any(|face| face.tex_id == entry.id));
            let material = gpu.is_some_and(|gpu| gpu.material_ids.contains(&entry.id));
            log::info!(
                "demo local preview: published={} pending={} mesh={} texture={} material={} overlay={} server_prim={} server_texture={} server_material={} rez_pending={}",
                entry.local_published,
                entry.local_pending,
                gpu.is_some_and(|gpu| matches_mesh(gpu.geom)),
                texture,
                material,
                object.render.local_volume.is_some() || !object.render.local_appearance.is_empty(),
                !object.volume.is_mesh(),
                object
                    .te
                    .as_ref()
                    .is_some_and(|te| te.faces.iter().any(|face| face.texture == entry.id)),
                object.extra.render_materials.iter().any(|(_, asset)| *asset == entry.id),
                self.imports.local_rez.is_some()
            );
        }
        self.imports.watch_local(&self.scene.jobs, ctx);
        if self.demo
            && self.imports.demo_mode == "folders-upload"
            && self.imports.demo_upload_started
            && !self.imports.busy()
            && self.imports.entries.is_empty()
            && self.frame_count.is_multiple_of(120)
        {
            let inventory = &self.world.inventory;
            let root = inventory
                .folders
                .values()
                .find(|f| f.info.parent == inventory.root && f.info.name == "Creator Pack")
                .map(|f| f.info.id);
            let items: Vec<_> = inventory
                .items
                .values()
                .filter(|item| root.is_some_and(|root| crate::world::inventory::actions::under(inventory, item.parent, root)))
                .collect();
            let matched = items
                .iter()
                .filter(|item| {
                    inventory.folders.get(&item.parent).is_some_and(|folder| {
                        matches!(
                            (item.name.as_str(), folder.info.name.as_str()),
                            ("Fabric", "Coat" | "Hat") | ("Coat", "Coat") | ("Idle", "Animations") | ("Bell", "Sounds")
                        )
                    })
                })
                .count();
            log::info!(
                "demo folder upload: items={} matched={} spent={}",
                items.len(),
                matched,
                self.imports.spent
            );
        }
        for Playback::Audio { entry, clip, duration } in self.imports.playback.drain(..) {
            let editor = self
                .imports
                .entries
                .iter_mut()
                .find(|sound| sound.id == entry)
                .map(|entry| &mut entry.sound);
            if let Some(editor) = editor {
                editor.preparing = false;
                if self.audio_engine.is_some() {
                    editor.started = Some(std::time::Instant::now());
                    editor.playing_duration = duration;
                    self.imports.sound_playing = Some(entry);
                }
            }
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
                let mesh_visible = self.imports.active == Some(entry.id)
                    || self.imports.preview_zoom.is_some_and(|(id, slot)| id == entry.id && slot.is_none());
                if mesh_visible
                    && let Some(Source::Mesh(draft)) = &entry.source
                    && entry
                        .mesh_player
                        .tick(draft, &self.scene.avatar_lib.rig, &mut self.scene.anims, std::time::Instant::now())
                {
                    ctx.request_repaint();
                }
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
                    entry.player.tick(
                        entry.id,
                        &self.scene.avatar_lib.rig,
                        &mut self.scene.anims,
                        entry.animation_model.current.as_ref().map(|model| &model.base),
                    );
                    if entry.player.playing {
                        ctx.request_repaint();
                    }
                }
            }
        }
        let was_open = self.imports.open;
        let mut actions = crate::imports::ui::show(
            ctx,
            palette,
            &mut self.imports,
            &mut self.settings.imports,
            &self.world,
            &self.settings.inventory,
        );
        // The texture picker's Local tab manages the importer's local previews.
        for request in std::mem::take(&mut self.world.local_asset_requests) {
            match request {
                crate::world::LocalAssetRequest::Add { materials } => {
                    self.imports
                        .choose_local_files(&self.scene.jobs, ctx, self.settings.imports.texture(), materials)
                }
                crate::world::LocalAssetRequest::Remove(id) => actions.push(Action::Remove(id)),
                crate::world::LocalAssetRequest::Upload(id) => {
                    self.imports
                        .upload_local(id, &self.scene.jobs, ctx, self.settings.imports.texture())
                }
            }
        }
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
                entry.sound.stop();
            }
            self.imports.sound_playing = None;
        }
        for action in actions {
            match action {
                Action::Files => self.imports.choose_files(&self.scene.jobs, ctx, self.settings.imports.texture()),
                Action::Folder => self.imports.choose_folder(&self.scene.jobs, ctx, self.settings.imports.texture()),
                Action::StageLocalUploads => {
                    self.imports
                        .stage_local_uploads(&self.world, &self.settings.inventory, &self.settings.imports);
                }
                Action::Zoom(id, slot) => {
                    self.imports.preview_zoom = Some((id, slot));
                    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("import_large_preview")));
                }
                Action::LargeMap(id, slot) => self.imports.request_large_map(id, slot, &self.scene.jobs, ctx),
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
                        .start_confirmed(&self.world, &self.settings.inventory, &self.settings.imports, &prices);
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
                    for entry in &mut self.imports.entries {
                        entry.sound.stop();
                    }
                    self.imports.sound_playing = None;
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
                Action::AnimationModel(id) => {
                    self.imports
                        .animation_model_file(id, &self.scene.jobs, ctx, self.scene.avatar_lib.clone());
                }
                Action::DefaultAnimationModel(id) => {
                    self.imports.reset_animation_models(id);
                }
                Action::SelectAnimationModel(id, saved) => {
                    self.imports
                        .select_animation_model(id, saved, &self.scene.jobs, ctx, self.scene.avatar_lib.clone());
                }
                Action::RemoveAnimationModel(saved) => {
                    self.imports.remove_animation_model(saved, &self.scene.jobs);
                }
                Action::SoundGain(id) => {
                    if self.imports.sound_playing == Some(id)
                        || self.imports.entries.iter().any(|entry| entry.id == id && entry.sound.preparing)
                    {
                        if let Some(engine) = &self.audio_engine {
                            engine.stop_world(SOUND_PREVIEW);
                        }
                        if let Some(entry) = self.imports.entries.iter_mut().find(|entry| entry.id == id) {
                            entry.sound.stop();
                        }
                        self.imports.sound_playing = None;
                        self.imports.play(id, &self.scene.jobs, ctx, self.scene.avatar_lib.rig.clone());
                    }
                }
                Action::Reload(id) => {
                    if self
                        .imports
                        .entries
                        .iter()
                        .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Sound(_))))
                    {
                        self.imports.play_request = None;
                        self.imports.sound_playing = None;
                        if let Some(engine) = &self.audio_engine {
                            engine.stop_world(SOUND_PREVIEW);
                        }
                    }
                    self.imports.reload(id, &self.scene.jobs, ctx);
                }
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
                    self.settings.imports.permissions_for(aurora_net::inventory::upload::Kind::Mesh),
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
                    // Firestorm's local mesh targets a selected object, whereas
                    // textures/materials follow the build selection's faces/linksets.
                    let selected = if self
                        .imports
                        .entries
                        .iter()
                        .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Mesh(_))))
                    {
                        self.build
                            .selection
                            .iter()
                            .filter_map(|key| self.world.objects.index_of(key))
                            .collect()
                    } else {
                        self.build.sel_prims(&self.world)
                    };
                    let targets: Vec<_> = selected
                        .into_iter()
                        .filter_map(|index| self.world.objects.get(index))
                        .filter(|object| object.pcode == aurora_prim::params::LL_PCODE_VOLUME)
                        .map(|object| (object.full_id, self.build.faces.get(&object.key).cloned().unwrap_or_default()))
                        .collect();
                    if targets.is_empty() {
                        self.imports.message = aurora_i18n::tr!("import-local-mesh-selection").into();
                        continue;
                    }
                    let use_scale = self
                        .imports
                        .entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .is_some_and(|entry| entry.local_use_scale);
                    for (target, faces) in targets {
                        self.imports.target_local(id, target, &faces, use_scale);
                    }
                    self.apply_local_mesh_targets();
                    self.imports.apply_appearance_targets(&mut self.world.objects);
                    if self
                        .imports
                        .entries
                        .iter()
                        .find(|e| e.id == id)
                        .is_some_and(|e| e.local_published && e.local_generation == e.preview_generation)
                    {
                        continue;
                    }
                    self.publish_local_import(id, ctx);
                }
                Action::ClearLocal(entry) => {
                    let ids: Vec<_> = self
                        .build
                        .sel_prims(&self.world)
                        .into_iter()
                        .filter_map(|index| self.world.objects.get(index))
                        .map(|object| object.full_id)
                        .collect();
                    let meshes: Vec<_> = ids
                        .iter()
                        .copied()
                        .filter(|id| self.imports.local_targets.get(id).is_some_and(|target| target.entry == entry))
                        .collect();
                    self.clear_local_mesh_targets(&meshes);
                    self.imports.clear_appearance_targets(&mut self.world.objects, &ids, Some(entry));
                }
                Action::RezLocal(id) => {
                    if self.imports.entries.iter().any(|entry| entry.id == id && entry.source.is_some()) {
                        self.imports.local_rez = Some(crate::imports::local::Rez {
                            entry: id,
                            known: self
                                .world
                                .objects
                                .iter()
                                .filter(|(_, object)| {
                                    object.owner_id == self.world.agent_id && object.update_flags & crate::build::FLAGS_CREATE_SELECTED != 0
                                })
                                .map(|(_, object)| object.full_id)
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
                    if self
                        .imports
                        .entries
                        .iter()
                        .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Sound(_))))
                    {
                        self.imports.play_request = None;
                        self.imports.sound_playing = None;
                        if let Some(engine) = &self.audio_engine {
                            engine.stop_world(SOUND_PREVIEW);
                        }
                    }
                    let targets: Vec<_> = self
                        .imports
                        .local_targets
                        .iter()
                        .filter(|(_, target)| target.entry == id)
                        .map(|(id, _)| *id)
                        .collect();
                    self.clear_local_mesh_targets(&targets);
                    let targets: Vec<_> = self
                        .imports
                        .local_appearance_targets
                        .iter()
                        .filter(|(_, appearance)| appearance.entry() == id)
                        .map(|((id, _), _)| *id)
                        .collect();
                    self.imports.clear_appearance_targets(&mut self.world.objects, &targets, Some(id));
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
                        for (asset, slot) in &entry.local_slots {
                            if !self.imports.entries.iter().any(|entry| entry.local_slots.contains_key(asset)) {
                                self.imports.retired_local_slots.push((*asset, *slot));
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
        self.save_import_animation_models();
        self.update_import_avatar_preview();
        if let Some(asset) = self.imports.map_picker.show(ctx, palette, &mut self.world, &self.ui_images)
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
                self.scene.textures.want_ui_image(&mut gfx.renderer, asset, &self.scene.jobs);
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
                (self.imports.active == Some(e.id) || e.local)
                    && e.preview.is_none()
                    && (!e.preview_pending || e.preview_job_generation != e.preview_generation)
                    && e.preview_error.is_none()
                    && (e.texture_target == 0 || e.local)
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
                    && (e.model.is_none() || e.model_generation != Some(e.preview_generation))
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
                // Local images/materials must be selectable before their
                // first application. Mesh resources still load on demand.
                (e.local_published
                    || e.local_restore_active
                    || (e.local && matches!(e.source, Some(Source::Texture(_) | Source::Material { .. }))))
                    && e.source.is_some()
                    && !e.local_pending
                    && (!e.local_published || e.local_generation != e.preview_generation)
                    && e.local_failed_generation != Some(e.preview_generation)
            })
            .map(|e| e.id)
            .collect();
        for id in locals {
            self.publish_local_import(id, ctx);
        }
        for entry in self.imports.entries.iter().filter(|e| e.local_published) {
            if let Some(image) = &entry.preview {
                self.ui_images.insert(entry.id, image.clone());
            }
        }
        self.imports.save_local_registry(false);
        self.show_local_appearance_status(ctx, palette);
    }

    fn show_local_appearance_status(&mut self, ctx: &egui::Context, palette: &Palette) {
        use crate::imports::ui::local_status::{self, Action};
        let status = self.imports.appearance_status(&self.world.objects);
        match local_status::show(ctx, palette, &mut self.imports.local_status, &status) {
            Some(Action::Library) => {
                self.imports.open(crate::imports::Choice::Local);
                ui::widgets::open_floater(ctx, crate::imports::Choice::Local.window_id(), &mut self.imports.open);
            }
            Some(Action::Remove) => {
                self.sync_local_picker_bindings();
                self.build.forget_local_picker_appearances();
                self.imports.clear_all_appearance_targets(&mut self.world.objects);
                self.imports.local_status.reveal = None;
            }
            None => {}
        }
    }

    pub(super) fn local_appearance_demo_input(&mut self, raw: &mut egui::RawInput) {
        let Ok(mode) = std::env::var("AURORA_DEMO_LOCAL_APPEARANCES") else {
            return;
        };
        if self.frame_count == 600 {
            self.imports.open = false;
            self.panels.people = false;
            self.panels.inventory = false;
            self.panels.perf = false;
            if let Some(index) = self.world.objects.index_of_uuid(&crate::demo::action_id(970))
                && let Some(focus) = crate::camera::pick_focus_object(&self.world, index, Instant::now())
            {
                self.camera
                    .zoom_to(focus.1.position, Some(focus), &mut self.world.agent, &self.settings.camera);
            }
        }
        let (key, start) = match self.frame_count {
            650..=651 if !matches!(mode.as_str(), "1" | "reveal") => ("local_appearance_warning_rect", 650),
            680..=681 if mode == "reveal" => ("local_appearance_reveal_rect", 680),
            850..=851 if mode == "reveal" => ("local_appearance_reveal_rect", 850),
            680..=681 if matches!(mode.as_str(), "confirm" | "remove" | "cancel") => ("local_appearance_remove_rect", 680),
            710..=711 if mode == "remove" => ("local_appearance_confirm_rect", 710),
            710..=711 if mode == "cancel" => ("local_appearance_cancel_rect", 710),
            _ => return,
        };
        if let Some(rect) = self.egui_ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(key))) {
            let pos = rect.center();
            raw.events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: self.frame_count == start,
                    modifiers: Default::default(),
                },
            ]);
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

    fn save_import_animation_models(&mut self) {
        if std::mem::take(&mut self.imports.animation_models_dirty) {
            self.settings
                .imports
                .animation_models
                .clone_from(&self.imports.load_preferences.animation_models);
            self.settings.imports.animation_model_last = self.imports.load_preferences.animation_model_last;
            self.settings.save();
        }
    }

    fn poll_local_rez(&mut self, ctx: &egui::Context) {
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
        self.imports.target_local(pending.entry, target, &[], true);
        if self
            .imports
            .entries
            .iter()
            .find(|e| e.id == pending.entry)
            .is_some_and(|e| !e.local_published || e.local_generation != e.preview_generation)
        {
            self.publish_local_import(pending.entry, ctx);
        } else {
            self.apply_local_mesh_targets();
            self.imports.apply_appearance_targets(&mut self.world.objects);
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
            // counted again from the local mesh (face stepping, texturing)
            self.build.face_counts.remove(id);
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
            self.build.face_counts.remove(id);
        }
    }
    pub(super) fn apply_local_imports(&mut self, renderer: &mut aurora_render::Renderer) {
        self.sync_local_picker_bindings();
        for (asset, slot) in self.imports.retired_local_slots.drain(..) {
            self.scene.textures.unregister_local(&asset);
            renderer.free_texture(slot);
        }
        self.imports.install_local_meshes(&mut self.scene.meshes, &self.scene.jobs);
        if self.imports.local_uploads.is_empty() {
            self.apply_local_mesh_targets();
            self.imports.apply_appearance_targets(&mut self.world.objects);
            return;
        }
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
            let Some(entry) = self.imports.entries.iter_mut().find(|e| e.id == prepared.entry) else {
                self.scene.jobs.discard(prepared);
                continue;
            };
            if entry.preview_generation != prepared.generation {
                entry.local_pending = false;
                self.scene.jobs.discard(prepared);
                continue;
            }
            let mut complete = true;
            let retained: std::collections::HashSet<_> = prepared.textures.iter().map(|t| t.id).collect();
            for texture in prepared.textures {
                let slot = shared_slots
                    .get(&texture.id)
                    .copied()
                    .or_else(|| renderer.textures.create_placeholder());
                if let Some(slot) = slot
                    && renderer.replace_texture_staged(slot, &texture.staged)
                {
                    entry.local_slots.insert(texture.id, slot);
                    shared_slots.insert(texture.id, slot);
                    if let Some(image) = prepared.images.get(&texture.id) {
                        self.scene.textures.set_local_pixels(texture.id, image.clone());
                    }
                    self.scene
                        .textures
                        .register_local_image(texture.id, slot, texture.alpha, texture.has_alpha);
                } else {
                    if !shared_slots.contains_key(&texture.id)
                        && let Some(slot) = slot
                    {
                        renderer.free_texture(slot);
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
                renderer.free_texture(slot);
            }
        }
        self.apply_local_mesh_targets();
        self.imports.apply_appearance_targets(&mut self.world.objects);
    }

    fn publish_local_import(&mut self, id: uuid::Uuid, ctx: &egui::Context) {
        if self
            .imports
            .entries
            .iter()
            .any(|entry| entry.id == id && matches!(entry.source, Some(Source::Mesh(_))))
        {
            self.imports.publish_local(id, &self.scene.jobs, ctx, None);
        } else {
            self.imports
                .publish_local(id, &self.scene.jobs, ctx, self.scene.textures.staging.clone());
        }
    }

    pub(super) fn sync_local_picker_bindings(&mut self) {
        self.imports
            .sync_local_appearance_changes(self.build.local_appearance_changes.drain(..));
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
                    aurora_net::inventory::upload::Kind::Snapshot => 15,
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
