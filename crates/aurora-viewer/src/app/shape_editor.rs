use super::*;
use crate::shape_editor::worn_shape;

impl App {
    pub(super) fn demo_shape_drag(&mut self) {
        if self.shape_ui.demo_drag.is_some() && self.shape_ui.models.is_empty() {
            self.shape_ui.demo_drag = Some(std::time::Instant::now());
        }
        let Some(start) = self.shape_ui.demo_drag else {
            return;
        };
        let elapsed = start.elapsed().as_secs_f32();
        let dragging = (2.0..8.0).contains(&elapsed);
        if dragging
            && let Some(draft) = &mut self.shape_ui.draft
            && let Some(param) = crate::shape_editor::catalog::PARAMS
                .iter()
                .filter(|p| p.group == 1)
                .min_by(|a, b| a.order.total_cmp(&b.order))
        {
            let value = param.min + (param.max - param.min) * (0.5 + 0.45 * (elapsed * 3.0).sin());
            draft.set(param, value);
            self.shape_ui.changed(&mut self.world);
        }
        self.shape_ui.editing = dragging;
    }
    pub(super) fn update_demo_shape_mesh(&mut self) {
        let Some(result) = self.shape_ui.demo_mesh.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.shape_ui.demo_mesh = None;
        match result {
            Ok(asset) => {
                self.scene
                    .meshes
                    .insert_local(crate::shape_editor::demo::ASSET, asset.data, asset.header, asset.skin);
                let object = if std::env::var("AURORA_DEMO_SHAPE").as_deref() == Ok("layered-head") {
                    crate::shape_editor::demo::layered_object(&self.world)
                } else {
                    crate::shape_editor::demo::object(&self.world)
                };
                if let Some(mut object) = object {
                    if std::env::var("AURORA_DEMO_SHAPE").as_deref() == Ok("pbr-head") {
                        let id = uuid::Uuid::from_u128(0xA0E0_5A9E_0000_0000_0000_0000_0000_0003);
                        self.scene.materials.resolve(&id);
                        self.scene.materials.on_material(
                            id,
                            Some(aurora_assets::PbrMaterial {
                                base_color_texture: Some(crate::scene::avatar::BAKES[0].2),
                                metallic_factor: 0.0,
                                roughness_factor: 0.7,
                                ..Default::default()
                            }),
                        );
                        object.extra.render_materials = vec![(0, id)];
                        // A conflicting TE tint makes the material replacement
                        // visible: a PBR head must retain the skin's own color.
                    }
                    self.world.apply(NetEvent::ObjectUpdates {
                        handle: self.world.main_region.unwrap_or_default(),
                        objects: vec![object],
                    });
                }
            }
            Err(error) => log::warn!("shape head fixture: {error}"),
        }
    }
    pub(super) fn open_shape_editor(&mut self, item: uuid::Uuid) {
        if !self.close_wearable_editor() {
            return;
        }
        if !self.close_physics_editor_for_shape() {
            self.appearance_ui.message = aurora_i18n::tr!("physics-other-editor").into();
            return;
        }
        self.shape_ui.hints = self.settings.shape_show_hints;
        self.shape_ui.feet = self.settings.shape_height_feet;
        if let Some(command) = self.shape_ui.open_item(&mut self.world, item) {
            if self.shape_camera.is_none() {
                self.shape_camera = Some(self.camera.clone());
            }
            self.focus_shape_category();
            self.panels.appearance = false;
            if self.demo {
                let data = self.demo_documents.get(&item).cloned().map(Ok).unwrap_or_else(|| {
                    crate::world::inventory::wearable::create(0, &self.world.inventory.items[&item].name, self.world.agent_id)
                });
                let asset = self.world.inventory.items[&item].asset_id;
                self.shape_ui.loaded(&mut self.world, item, asset, &data);
            } else {
                self.send(command);
            }
        }
    }

    pub(super) fn focus_shape_category(&mut self) {
        // The initial demo/login pose may not have been evaluated yet. Frame
        // the rendered skeleton once it exists, including its root height.
        if !self.world.avatar_poses.contains_key(&self.world.agent_id) {
            self.shape_ui.camera_pending = true;
            return;
        }
        let turned = self.customize_turns();
        if let Some((eye, target)) =
            crate::shape_editor::camera::view(&self.world, &self.scene, self.shape_ui.group, self.camera.fov_y, turned)
        {
            let (eye, target) = egui::AreaState::load(&self.egui_ctx, egui::Id::unique("shape_editor"))
                .map(|editor| {
                    crate::shape_editor::camera::beside_editor(
                        (eye, target),
                        self.camera.fov_y,
                        self.egui_ctx.content_rect(),
                        editor.rect(),
                    )
                })
                .unwrap_or((eye, target));
            self.camera
                .set_appearance_view(eye, target, &mut self.world.agent, &self.settings.camera);
            self.shape_ui.camera_pending = false;
        }
    }

    pub(super) fn shape_action(&mut self, action: ui::shape::Action) {
        match action {
            ui::shape::Action::CustomizeAnimation(enabled) => {
                self.settings.customize_animation = enabled;
                self.settings.save();
            }
            ui::shape::Action::Retry => {
                if let Some(item) = worn_shape(&self.world) {
                    self.open_shape_editor(item);
                }
            }
            ui::shape::Action::Save(name) => {
                if let Some(command) = self.shape_ui.save(&self.world, name) {
                    if self.demo {
                        self.demo_save_wearable(command);
                    } else {
                        self.send(command);
                    }
                }
            }
        }
    }

    pub(super) fn demo_shape_editor(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_SHAPE") else {
            return;
        };
        if mode == "t-pose" {
            self.settings.customize_animation = true;
            for (id, done) in [
                (super::avatar_health::ANIM_CUSTOMIZE, false),
                (super::avatar_health::ANIM_CUSTOMIZE_DONE, true),
            ] {
                self.scene.anims.insert(
                    id,
                    std::sync::Arc::new(crate::scene::anim::BoundAnim::bind(
                        crate::shape_editor::demo::customize_animation(done),
                        &self.world.avatar_lib.rig,
                    )),
                );
            }
        }
        crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
        let Some(item) = worn_shape(&self.world) else {
            return;
        };
        if let Some(info) = self.world.inventory.items.get_mut(&item) {
            info.name = aurora_i18n::tr!("shape-demo-name").into();
            if mode == "readonly" {
                info.owner_mask &= !0x4000;
            }
            if mode == "nocopy" {
                info.owner_mask &= !0x8000;
            }
        }
        self.panels.perf = false;
        self.panels.inventory = false;
        self.panels.appearance = false;
        self.panels.people = false;
        self.panels.chat = false;
        self.panels.minimap = false;
        self.open_shape_editor(item);
        self.shape_ui.group = match mode.as_str() {
            "head" | "mesh-head" | "static-head" | "mesh-drag" | "pbr-head" | "layered-head" => 1,
            "eyes" => 2,
            "ears" => 3,
            "nose" => 4,
            "mouth" => 5,
            "chin" => 6,
            "torso" => 7,
            "legs" => 8,
            _ => 0,
        };
        if mode == "save-as" {
            self.shape_ui.copy_name = Some(aurora_i18n::tr!("shape-demo-name").into());
        }
        if let Some(draft) = &mut self.shape_ui.draft {
            if mode == "male" {
                draft.edited.params.insert(80, 1.0);
            }
            if mode == "dirty" {
                draft.edited.params.insert(33, 2.0);
                self.shape_ui.close_confirmation = true;
            }
            if mode == "saving" {
                draft.pending = Some(uuid::Uuid::from_u128(91));
            }
        }
        if mode == "save-feedback" {
            self.shape_ui.message = format!(
                "{} {}",
                aurora_i18n::tr!("shape-save-unconfirmed"),
                aurora_i18n::tr!("shape-save-interrupted")
            );
        }
        self.shape_ui.changed(&mut self.world);
        self.focus_shape_category();
        if mode == "mesh-drag" {
            self.shape_ui.demo_drag = Some(std::time::Instant::now());
        }
        if matches!(
            mode.as_str(),
            "mesh-head" | "static-head" | "mesh-drag" | "pbr-head" | "layered-head"
        ) {
            let (tx, rx) = std::sync::mpsc::channel();
            self.shape_ui.demo_mesh = Some(rx);
            let library = self.world.avatar_lib.clone();
            self.scene.jobs.run(move || {
                let result = if mode == "layered-head" {
                    crate::shape_editor::demo::layered_asset(&library)
                } else {
                    crate::shape_editor::demo::asset(&library, mode != "static-head")
                };
                let _ = tx.send(result);
            });
        }
    }
}
