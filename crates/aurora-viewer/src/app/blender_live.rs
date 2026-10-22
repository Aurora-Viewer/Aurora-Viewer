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
        if self.blender_live.show(ctx, palette) {
            self.blender_live.start_requested = true;
        }
        if std::mem::take(&mut self.blender_live.save_addon) {
            self.blender_live.addon.save(&self.scene.jobs, ctx);
        }
    }

    pub(super) fn clear_blender_preview(&mut self) {
        for id in self.blender_live.objects.drain(..) {
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
        if std::mem::take(&mut self.blender_live.start_requested) {
            self.blender_live.start(
                self.blender_joints(),
                self.blender_rest_axes(),
                &self.egui_ctx,
                renderer.staging_pool(),
            );
        }
        if self.blender_live.owner.is_some_and(|owner| owner != self.world.agent_id) {
            self.blender_live.stop();
        }
        if self.blender_live.clear {
            self.blender_live.clear = false;
            self.blender_live.snapshot = None;
            self.blender_live.owner = None;
            self.clear_blender_preview();
        }
        // An offline fixture uses the exact protocol and production skin path.
        if self.demo
            && !self.blender_live.demo_started
            && std::env::var_os("AURORA_DEMO_BLENDER_LIVE").is_some()
            && self.world.objects.index_of_uuid(&self.world.agent_id).is_some()
        {
            self.blender_live.demo_started = true;
            self.blender_live.open = true;
            self.blender_live.start(
                self.blender_joints(),
                self.blender_rest_axes(),
                &self.egui_ctx,
                renderer.staging_pool(),
            );
            match crate::blender_live::demo_fixture(&self.world.avatar_lib.rig) {
                Ok(snapshot) => self.blender_live.snapshot = Some(snapshot),
                Err(error) => self.blender_live.error = Some(error),
            }
        }
        if let Some(snapshot) = self.blender_live.take_latest() {
            if let Some(old) = self.blender_live.snapshot.replace(snapshot) {
                self.scene.jobs.discard(old);
            }
            self.blender_live.revision = None;
            self.blender_live.owner = Some(self.world.agent_id);
        }
        let missing = self
            .blender_live
            .objects
            .iter()
            .any(|id| self.world.objects.index_of_uuid(id).is_none());
        if let Some(snapshot) = self.blender_live.snapshot.as_ref()
            && (Some(snapshot.revision) != self.blender_live.revision || missing)
            && self.world.objects.index_of_uuid(&self.world.agent_id).is_some()
        {
            self.install_blender_snapshot(snapshot.clone(), renderer);
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
            log::info!(
                "demo Blender live: revision={:?} objects={} ready_skinned={ready} appearances={appearances} images={images}",
                self.blender_live.revision,
                self.blender_live.objects.len()
            );
        }
    }

    fn install_blender_snapshot(&mut self, snapshot: Prepared, renderer: &mut aurora_render::Renderer) {
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
        for asset in self.blender_live.assets.drain(..) {
            if !snapshot.meshes.iter().any(|mesh| mesh.id == asset) {
                self.scene.retire_preview_mesh(asset);
            }
        }
        while self.blender_live.objects.len() > snapshot.meshes.len() {
            if let Some(id) = self.blender_live.objects.pop()
                && let Some(index) = self.world.objects.index_of_uuid(&id)
            {
                self.world.objects.remove_idx(index);
            }
        }
        for (index, mesh) in snapshot.meshes.into_iter().enumerate() {
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
            self.scene.meshes.insert_local(mesh.id, mesh.data, mesh.header, Some(mesh.skin));
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
            if let Some(existing) = self
                .blender_live
                .objects
                .get(index)
                .and_then(|id| self.world.objects.index_of_uuid(id))
                .and_then(|i| self.world.objects.get_mut(i))
            {
                // Keep the draw object and its last good geometry while the
                // replacement is staged; no disappearance between snapshots.
                existing.volume = volume;
                existing.extra = extra;
                existing.te = Some(Arc::new(TextureEntry { faces }));
                existing.shape_dirty = true;
                existing.material_dirty = true;
                existing.render.needs_records = true;
                continue;
            }
            let id = Uuid::new_v4();
            let update = ObjectUpdate {
                local_id: index as u32 + 1,
                full_id: id,
                parent_id: parent.local_id,
                pcode: aurora_prim::params::LL_PCODE_VOLUME,
                state: 1u8.rotate_left(4),
                crc: 0,
                material: 0,
                click_action: crate::interaction::code::IGNORE,
                scale: Vec3::ONE,
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                acceleration: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                update_flags: 0,
                owner_id: Uuid::nil(),
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
            self.world.objects.insert_preview_attachment(parent, update);
            if index < self.blender_live.objects.len() {
                self.blender_live.objects[index] = id;
            } else {
                self.blender_live.objects.push(id);
            }
            log::debug!("Blender preview installed: {}", mesh.name);
        }
        self.blender_live.triangles = snapshot.triangles;
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
