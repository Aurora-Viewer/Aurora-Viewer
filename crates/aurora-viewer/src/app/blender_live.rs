use super::*;
use crate::blender_live::protocol::Prepared;
use aurora_net::objects::{ObjectUpdate, ParticleUpdate};
use aurora_prim::{ExtraParams, TextureEntry, VolumeParams, params::SculptParams, te::TextureFace};
use glam::Quat;
use uuid::Uuid;

impl App {
    fn blender_joints(&self) -> HashSet<String> {
        self.world
            .avatar_lib
            .rig
            .names
            .keys()
            .chain(self.world.avatar_lib.rig.skin_points.keys())
            .cloned()
            .collect()
    }

    fn blender_rest_axes(&self) -> std::collections::HashMap<String, [f32; 16]> {
        let rig = &self.world.avatar_lib.rig;
        self.blender_joints()
            .into_iter()
            .map(|name| {
                let (joint, local) = rig.skin_bind(&name, glam::Mat4::IDENTITY);
                let mut inverse = (rig.default_world_inv[joint].inverse() * local).inverse();
                inverse.w_axis = glam::Vec4::W;
                (name, inverse.to_cols_array())
            })
            .collect()
    }

    pub(super) fn show_blender_live(&mut self, ctx: &egui::Context, palette: &Palette) {
        self.blender_live.edit_linked = self.settings.build.edit_linked;
        self.blender_live.attach_choices = self
            .world
            .avatar_lib
            .attach_points
            .iter()
            .map(|(&id, point)| {
                let name = self
                    .world
                    .avatar_lib
                    .attach_names
                    .get(&id)
                    .map(String::as_str)
                    .unwrap_or(aurora_i18n::tr!("common-attachment"));
                (id, point.hud, crate::ui::appearance::items::point_name(name).to_owned())
            })
            .collect();
        self.blender_live.attach_choices.sort_by(|a, b| a.2.cmp(&b.2));
        self.blender_live.shape.blocked = crate::blender_live::shape::editable(&self.world)
            .and_then(|_| self.blender_live.shape_connection())
            .err();
        self.blender_live.shape.apply_blocked = crate::blender_live::shape::editable(&self.world)
            .and_then(|_| self.blender_live.shape_apply_connection())
            .err();
        let can_upload = !self.imports.busy()
            && self.imports.pending_files.is_none()
            && self.imports.entries.len() < 512
            && !self.blender_live.import.busy();
        if self.blender_live.show(ctx, palette, can_upload) {
            self.blender_live.start_requested = true;
        }
        if std::mem::take(&mut self.blender_live.save_addon) {
            self.blender_live.addon.save(&self.scene.jobs, ctx);
        }
        if let Some(action) = self.blender_live.shape.requested.take() {
            self.send_blender_shape(action);
        }
        let choose = std::mem::take(&mut self.blender_live.installer_choose);
        let scan = std::mem::take(&mut self.blender_live.installer_scan);
        if choose {
            self.blender_live.installer.choose(&self.scene.jobs, ctx);
        } else if scan {
            self.blender_live.installer.scan(&self.scene.jobs, ctx);
        }
        if let Some(executable) = self.blender_live.installer_target.take() {
            self.blender_live.installer.install(executable, &self.scene.jobs, ctx);
        }
        if let Some(id) = self.blender_live.upload_requested.take()
            && can_upload
            && let Some(snapshot) = &self.blender_live.snapshot
            && let Some(source) = snapshot.sources.iter().find(|source| source.id == id)
        {
            if self.demo && std::env::var("AURORA_DEMO_BLENDER_LIVE").as_deref() == Ok("upload") {
                self.imports
                    .open_live_mesh(source.clone(), snapshot.images.clone(), &self.scene.jobs, ctx);
            } else {
                self.blender_live.request_import(id);
            }
        }
    }

    pub(super) fn clear_blender_preview(&mut self) {
        self.blender_live.instances.clear();
        for id in self.blender_live.objects.drain(..) {
            self.build.props.remove(&id);
            if let Some(index) = self.world.objects.index_of_uuid(&id) {
                self.world.objects.remove_idx(index);
            }
        }
        for asset in self.blender_live.assets.drain(..) {
            self.scene.retire_preview_mesh(asset);
        }
        self.blender_live.triangles = 0;
        self.blender_live.revision = None;
        self.scene.reset_skeleton(self.world.agent_id, false);
    }

    pub(super) fn update_blender_live(&mut self, renderer: &mut aurora_render::Renderer) {
        self.blender_live.capture_placements(&mut self.world);
        if let Some((source, action)) = self.blender_live.object_action.take() {
            let source = if matches!(
                action,
                crate::blender_live::objects::Action::Transform { .. } | crate::blender_live::objects::Action::Edit
            ) {
                source
            } else {
                self.blender_live.linked_root(source)
            };
            if matches!(action, crate::blender_live::objects::Action::Edit) {
                if let Some(idx) = self
                    .blender_live
                    .edit_target(source, self.settings.build.edit_linked)
                    .and_then(|id| self.world.objects.index_of_uuid(&id))
                {
                    self.build.open_build(crate::build::Tool::Edit);
                    self.build.click_select(&mut self.world, &self.settings.build, Some(idx), false);
                    self.flush_build();
                }
            } else {
                let revision = self.blender_live.revision;
                let ceiling = self
                    .blender_live
                    .root(source)
                    .and_then(|id| self.world.objects.index_of_uuid(&id))
                    .and_then(|idx| Scene::object_transform(&self.world, idx, Instant::now(), 0))
                    .filter(|(_, _, hud)| !hud)
                    .map_or(self.world.agent.position.z, |(position, _, _)| {
                        position.z.max(self.world.agent.position.z)
                    });
                self.blender_live.place_object(source, action, &self.world);
                let floor_offset = self.blender_live.floor_offset(source);
                if matches!(
                    action,
                    crate::blender_live::objects::Action::Rez | crate::blender_live::objects::Action::Drop
                ) && let Some(placement) = self.blender_live.placements.get_mut(&source)
                    && let Some(floor) = self.scene.preview_floor(&self.world, placement.position, ceiling)
                {
                    placement.position.z = floor + floor_offset;
                }
                if matches!(action, crate::blender_live::objects::Action::Transform { .. }) {
                    self.blender_live.apply_transform(source, &mut self.world);
                    self.blender_live.revision = revision;
                }
            }
        }
        self.update_blender_shape();
        if self.blender_live.owner.is_some_and(|owner| owner != self.world.agent_id) {
            self.blender_live.stop();
        }
        self.blender_live.poll_import();
        if !self.imports.busy()
            && self.imports.pending_files.is_none()
            && self.imports.entries.len() < 512
            && let Some(bundle) = self.blender_live.import.ready.take()
        {
            self.imports.open_blender_lods(bundle, &self.scene.jobs, &self.egui_ctx);
        }
        if std::mem::take(&mut self.blender_live.start_requested) {
            self.blender_live.start(
                self.blender_joints(),
                self.blender_rest_axes(),
                &self.egui_ctx,
                renderer.staging_pool(),
            );
        }
        if self.blender_live.clear {
            self.blender_live.clear = false;
            if let Some(snapshot) = self.blender_live.snapshot.take() {
                self.scene.jobs.discard(snapshot);
            }
            self.blender_live.owner = None;
            self.blender_live.states.clear();
            self.blender_live.selected = None;
            self.blender_live.upload_requested = None;
            self.blender_live.placements.clear();
            self.blender_live.placement_dirty.clear();
            self.blender_live.object_action = None;
            self.blender_live.world_origin = None;
            self.clear_blender_preview();
        }
        // An offline fixture uses the exact protocol and production skin path.
        if self.demo
            && !self.blender_live.demo_started
            && std::env::var_os("AURORA_DEMO_BLENDER_LIVE").is_some()
            && self.world.objects.index_of_uuid(&self.world.agent_id).is_some()
            && (self.frame_count >= 300 || std::env::var("AURORA_DEMO_BLENDER_LIVE").is_ok_and(|mode| !mode.starts_with("static")))
        {
            self.blender_live.demo_started = true;
            self.blender_live.open = true;
            // Isolate automated bridge tests from already-open Blender clients.
            if let Ok(port) = std::env::var("AURORA_DEMO_BLENDER_PORT").unwrap_or_default().parse::<u16>()
                && port >= 1024
            {
                self.blender_live.port = port;
            }
            self.blender_live.start(
                self.blender_joints(),
                self.blender_rest_axes(),
                &self.egui_ctx,
                renderer.staging_pool(),
            );
            let mode = std::env::var("AURORA_DEMO_BLENDER_LIVE").unwrap_or_default();
            if mode == "plugins" || mode.starts_with("installer-") {
                self.blender_live.tab = crate::blender_live::Tab::Plugins;
            }
            self.blender_live.installer.demo(&mode);
            if mode.starts_with("shape") {
                self.demo_blender_shape(mode == "shape-readonly");
            }
            let fixture = if mode.starts_with("static") {
                crate::blender_live::demo_static_fixture(&self.world.avatar_lib.rig, mode.starts_with("static-linked"))
            } else {
                crate::blender_live::demo_fixture(&self.world.avatar_lib.rig, mode != "1")
            };
            match fixture {
                Ok(snapshot) if !mode.starts_with("shape") && mode != "lods" => {
                    self.blender_live.receive(snapshot);
                    self.blender_live.owner = Some(self.world.agent_id);
                    if mode == "objects" {
                        let ids: Vec<_> = self
                            .blender_live
                            .snapshot
                            .as_ref()
                            .into_iter()
                            .flat_map(|s| &s.sources)
                            .map(|s| s.id)
                            .collect();
                        if ids.len() == 3 {
                            self.blender_live.set_state(ids[1], crate::blender_live::SourceState::Hidden);
                            self.blender_live.set_state(ids[2], crate::blender_live::SourceState::Removed);
                        }
                    } else if mode == "upload" {
                        self.blender_live.upload_requested = self.blender_live.selected;
                        self.blender_live.open = false;
                    }
                }
                Ok(_) => {}
                Err(error) => self.blender_live.error = Some(error),
            }
        }
        if let Some(snapshot) = self.blender_live.take_latest() {
            if let Some(old) = self.blender_live.receive(snapshot) {
                self.scene.jobs.discard(old);
            }
            self.blender_live.owner = Some(self.world.agent_id);
        }
        if self.demo
            && self.frame_count == 450
            && let Ok(mode) = std::env::var("AURORA_DEMO_BLENDER_LIVE")
            && mode.starts_with("static")
            && let Some(source) = self.blender_live.selected
        {
            use crate::blender_live::objects::Action;
            let source = if mode == "static-linked-child" {
                self.blender_live
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.sources.iter().find(|s| s.link.is_some()))
                    .map_or(source, |s| s.id)
            } else {
                source
            };
            self.blender_live.selected = Some(source);
            if mode == "static-linked-child" {
                self.build.set_edit_linked(&mut self.world, &mut self.settings.build, true);
            }
            let action = match mode.as_str() {
                "static-hud" | "static-linked-hud" | "static-linked-hud-menu" => Action::Attach(31),
                "static-attached" | "static-linked-attached" => Action::Attach(6),
                "static-detached" | "static-linked-detached" => Action::Detach,
                _ => Action::Edit,
            };
            if matches!(
                mode.as_str(),
                "static" | "static-linked" | "static-linked-child" | "static-linked-menu"
            ) && let Some(placement) = self.blender_live.placements.get(&source)
            {
                let position = if let Some(parent) = placement.parent.and_then(|id| self.blender_live.placements.get(&id)) {
                    parent.position + parent.rotation * placement.position
                } else {
                    placement.position
                };
                self.camera.set_view(
                    position + Vec3::new(3.0, -3.0, 2.0),
                    position,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
            }
            self.blender_live.object_action = Some((source, action));
        }
        if self.demo
            && self.frame_count == 520
            && std::env::var("AURORA_DEMO_BLENDER_LIVE")
                .is_ok_and(|mode| matches!(mode.as_str(), "static-linked-menu" | "static-linked-hud-menu"))
            && let Some(source) = self
                .blender_live
                .snapshot
                .as_ref()
                .and_then(|s| s.sources.iter().find(|s| s.link.is_some()))
            && let Some(idx) = self
                .blender_live
                .root(source.id)
                .and_then(|id| self.world.objects.index_of_uuid(&id))
            && let Some(object) = self.world.objects.get(idx)
        {
            let key = object.key;
            let id = object.full_id;
            let point = Scene::object_transform(&self.world, idx, Instant::now(), 0).map_or(Vec3::ZERO, |xf| xf.0);
            let worn = crate::ui::context::wearer(&self.world, idx) == Some(self.world.agent_id);
            self.build.select_for_menu(&mut self.world, &self.settings.build, idx);
            self.flush_build();
            self.context_menu = Some(crate::ui::context::ContextMenu {
                pos: egui::pos2(700.0, 180.0),
                point,
                target: if worn {
                    crate::ui::context::Target::Attachment { key }
                } else {
                    crate::ui::context::Target::Object {
                        key,
                        full_id: id,
                        offset: Vec3::ZERO,
                    }
                },
                pie_path: Vec::new(),
                pie_input_frame: None,
            });
        }
        if self.demo
            && std::env::var("AURORA_DEMO_BLENDER_LIVE").as_deref() == Ok("lods")
            && !self.blender_live.import.demo_requested
            && let Some(source) = self.blender_live.snapshot.as_ref().and_then(|snapshot| snapshot.sources.first())
        {
            let id = source.id;
            self.blender_live.import.demo_requested = true;
            self.blender_live.request_import(id);
        }
        let missing = self
            .blender_live
            .objects
            .iter()
            .any(|id| self.world.objects.index_of_uuid(id).is_none());
        if let Some(snapshot) = self.blender_live.snapshot.as_ref()
            && (Some(snapshot.revision) != self.blender_live.revision || missing)
            && self.world.objects.index_of_uuid(&self.world.agent_id).is_some()
            && let Some(snapshot) = self.blender_live.snapshot.clone()
        {
            self.install_blender_snapshot(snapshot, renderer);
        }
        if self.demo && self.blender_live.demo_started && self.frame_count.is_multiple_of(120) {
            let (appearances, images) = self.scene.preview_appearance_counts();
            let ready = self
                .blender_live
                .objects
                .iter()
                .filter_map(|id| self.world.objects.index_of_uuid(id))
                .filter(|&index| {
                    let ready = self.scene.gpu.get(index).is_some_and(|gpu| {
                        let current =
                            matches!(gpu.geom, Some(crate::scene::GeomKey::Mesh { id, .. }) if self.blender_live.assets.contains(&id));
                        gpu.rigged && !gpu.faces.is_empty() && current
                    });
                    let own = Scene::skeleton_owner(&self.world, index).is_some_and(|(owner, _)| owner == self.world.agent_id);
                    ready && own
                })
                .count();
            let (mut ready_static, mut ready_hud) = (0, 0);
            for id in &self.blender_live.objects {
                if let Some(gpu) = self.world.objects.index_of_uuid(id).and_then(|idx| self.scene.gpu.get(idx))
                    && !gpu.rigged
                    && !gpu.faces.is_empty()
                    && matches!(gpu.geom, Some(crate::scene::GeomKey::Mesh { id, .. }) if self.blender_live.assets.contains(&id))
                {
                    ready_static += 1;
                    ready_hud += usize::from(gpu.hud);
                }
            }
            log::info!(
                "demo Blender live: revision={:?} objects={} ready_skinned={ready} ready_static={ready_static} ready_hud={ready_hud} appearances={appearances} images={images}",
                self.blender_live.revision,
                self.blender_live.objects.len()
            );
        }
    }

    fn install_blender_snapshot(&mut self, snapshot: Prepared, renderer: &mut aurora_render::Renderer) {
        // Hidden and derezzed sources also retain their current hierarchy.
        let initial: Vec<_> = snapshot
            .sources
            .iter()
            .filter(|source| !source.rigged() && source.link.is_none() && !self.blender_live.placements.contains_key(&source.id))
            .map(|source| source.id)
            .collect();
        self.blender_live.prepare_placements(&snapshot, &self.world);
        for source in initial {
            let offset = self.blender_live.floor_offset(source);
            if let Some(placement) = self.blender_live.placements.get_mut(&source)
                && let Some(floor) = self
                    .scene
                    .preview_floor(&self.world, placement.position, self.world.agent.position.z)
            {
                placement.position.z = floor + offset;
            }
        }
        let Some(snapshot) = self.blender_live.visible_snapshot() else {
            return;
        };
        if !self.scene.install_preview_images(&snapshot.images, renderer) {
            return;
        }
        let Some(parent) = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
            .map(|o| o.key)
        else {
            self.blender_live.error = Some(aurora_i18n::tr!("blender-live-no-avatar").into());
            return;
        };
        let previous_assets: HashSet<_> = self.blender_live.assets.drain(..).collect();
        for &asset in &previous_assets {
            if !snapshot.meshes.iter().any(|mesh| mesh.id == asset) {
                self.scene.retire_preview_mesh(asset);
            }
        }
        let mut old = std::mem::take(&mut self.blender_live.instances);
        let mut parts = std::collections::HashMap::<Uuid, usize>::new();
        let mut roots = std::collections::HashMap::new();
        self.blender_live.objects.clear();
        for mesh in snapshot.meshes {
            let part = parts.entry(mesh.source).or_default();
            let key = (mesh.source, *part);
            *part += 1;
            let rigged = mesh.skin.is_some();
            let placement = (!rigged).then(|| self.blender_live.placements[&mesh.source]);
            self.scene.install_preview_materials(mesh.id, &mesh.materials);
            let extra = ExtraParams {
                render_materials: mesh
                    .materials
                    .iter()
                    .enumerate()
                    .filter_map(|(face, value)| value.as_ref().map(|(id, _)| (face as u8, *id)))
                    .collect(),
                ..Default::default()
            };
            if !previous_assets.contains(&mesh.id) {
                self.scene.meshes.insert_local(mesh.id, mesh.data, mesh.header, mesh.skin);
            }
            self.blender_live.assets.push(mesh.id);
            let faces = mesh
                .colors
                .iter()
                .enumerate()
                .map(|(face, color)| TextureFace {
                    texture: Uuid::nil(),
                    color: if mesh.materials[face].is_some() { [1.0; 4] } else { *color },
                    ..Default::default()
                })
                .collect();
            let volume = VolumeParams {
                sculpt: Some(SculptParams {
                    texture: mesh.id,
                    sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
                }),
                ..Default::default()
            };
            let id = old.remove(&key).unwrap_or_else(Uuid::new_v4);
            let local_id = self
                .world
                .objects
                .index_of_uuid(&id)
                .and_then(|i| self.world.objects.get(i))
                .map(|o| o.key.local_id)
                .unwrap_or_else(|| {
                    let id = self.blender_live.next_local_id;
                    self.blender_live.next_local_id = id.saturating_add(1);
                    id
                });
            let attachment = placement.and_then(|p| match p.location {
                crate::blender_live::objects::Location::Attachment(point) => Some(point),
                _ => None,
            });
            let object_parent = self.blender_live.preview_parent(mesh.source, key.1, rigged, parent, &roots);
            let (position, rotation) = self
                .blender_live
                .preview_transform(mesh.source, key.1)
                .unwrap_or((Vec3::ZERO, Quat::IDENTITY));
            let update = ObjectUpdate {
                local_id,
                full_id: id,
                parent_id: object_parent.map_or(0, |key| key.local_id),
                pcode: aurora_prim::params::LL_PCODE_VOLUME,
                state: if rigged {
                    1
                } else {
                    attachment
                        .filter(|_| key.1 == 0 && placement.is_some_and(|p| p.parent.is_none()))
                        .unwrap_or(0)
                }
                .rotate_left(4),
                crc: 0,
                material: 0,
                click_action: if rigged {
                    crate::interaction::code::IGNORE
                } else {
                    crate::interaction::code::DISABLED
                },
                scale: placement.map_or(Vec3::ONE, |p| p.size * p.scale),
                position,
                rotation,
                velocity: Vec3::ZERO,
                acceleration: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                update_flags: if rigged {
                    0
                } else {
                    crate::ui::context::flags::OBJECT_MOVE
                        | crate::ui::context::flags::OBJECT_MODIFY
                        | crate::ui::context::flags::OBJECT_YOU_OWNER
                },
                owner_id: self.world.agent_id,
                volume,
                texture_entry: Some(Arc::new(TextureEntry { faces })),
                extra,
                name_values: String::new(),
                text: String::new(),
                text_color: [0; 4],
                media_url: String::new(),
                tree_species: None,
                texture_anim: None,
                foot_plane: None,
                particles: ParticleUpdate::Clear,
                sound: Default::default(),
            };
            let index = self.world.objects.upsert_preview(object_parent, update);
            if !rigged {
                let mask = aurora_net::build::perm::MOVE | aurora_net::build::perm::MODIFY;
                self.build.props.insert(
                    id,
                    aurora_net::build::ObjectProps {
                        object_id: id,
                        full: true,
                        owner_id: self.world.agent_id,
                        creator_id: self.world.agent_id,
                        base_mask: mask,
                        owner_mask: mask,
                        name: mesh.name.clone(),
                        ..Default::default()
                    },
                );
            }
            if key.1 == 0
                && let Some(o) = self.world.objects.get(index)
            {
                roots.insert(mesh.source, o.key);
            }
            self.blender_live.instances.insert(key, id);
            self.blender_live.objects.push(id);
            log::debug!("Blender preview installed: {}", mesh.name);
        }
        for id in old.into_values() {
            self.build.props.remove(&id);
            if let Some(idx) = self.world.objects.index_of_uuid(&id) {
                self.world.objects.remove_idx(idx);
            }
        }
        self.blender_live.triangles = snapshot.triangles;
        self.blender_live.placement_dirty.clear();
        self.blender_live.revision = Some(snapshot.revision);
        self.blender_live.error = None;
        log::info!(
            "Blender live revision {}: {} draw objects, {} triangles",
            snapshot.revision,
            self.blender_live.objects.len(),
            snapshot.triangles
        );
    }
}
