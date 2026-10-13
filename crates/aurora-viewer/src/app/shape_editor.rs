use super::*;
use crate::shape_editor::worn_shape;

impl App {
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
                if let Some(object) = crate::shape_editor::demo::object(&self.world) {
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
        self.shape_ui.hints = self.settings.shape_show_hints;
        self.shape_ui.feet = self.settings.shape_height_feet;
        if let Some(command) = self.shape_ui.open_item(&mut self.world, item) {
            if self.shape_camera.is_none() {
                self.shape_camera = Some(self.camera.clone());
                let center = self.world.agent.predicted_position();
                let eye = center + glam::Quat::from_rotation_z(self.world.agent.yaw) * Vec3::new(3.4, 0.0, 0.15);
                self.camera.set_view(eye, center, &mut self.world.agent, &self.settings.camera);
            }
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

    pub(super) fn shape_action(&mut self, action: ui::shape::Action) {
        match action {
            ui::shape::Action::Retry => {
                if let Some(item) = worn_shape(&self.world) {
                    self.open_shape_editor(item);
                }
            }
            ui::shape::Action::Save(name) => {
                if let Some(command) = self.shape_ui.save(&self.world, name) {
                    if self.demo {
                        if let NetCommand::SaveWearable {
                            request,
                            item,
                            data,
                            copy_name,
                        } = command
                        {
                            let mut item = *item;
                            if let Some(name) = copy_name {
                                item.id = uuid::Uuid::new_v4();
                                item.name = name;
                            }
                            item.asset_id = uuid::Uuid::new_v4();
                            // Exercise the exact encoder/parser, without any grid traffic.
                            let result = aurora_assets::wearable::Wearable::parse(&data)
                                .map(|wearable| {
                                    self.demo_documents.insert(item.id, data);
                                    let mut appearance = self.world.server_appearance().cloned().unwrap_or(aurora_net::AvatarAppearance {
                                        avatar_id: self.world.agent_id,
                                        texture_entry: None,
                                        visual_params: Vec::new(),
                                        cof_version: 0,
                                        hover_height: 0.0,
                                        attachments: 0,
                                    });
                                    appearance.visual_params = self
                                        .world
                                        .avatar_lib
                                        .shape_params
                                        .edit_values(self.world.server_visual_params(), &wearable.params);
                                    self.world.apply(NetEvent::Appearance(appearance));
                                    item
                                })
                                .map_err(|_| aurora_i18n::tr!("shape-error").to_owned());
                            self.on_app_event(NetEvent::WearableSaved { request, result });
                        }
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
        self.open_shape_editor(item);
        self.shape_ui.group = match mode.as_str() {
            "head" | "mesh-head" | "static-head" => 1,
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
        }
        self.shape_ui.changed(&mut self.world);
        if matches!(mode.as_str(), "mesh-head" | "static-head") {
            let (tx, rx) = std::sync::mpsc::channel();
            self.shape_ui.demo_mesh = Some(rx);
            let library = self.world.avatar_lib.clone();
            self.scene.jobs.run(move || {
                let _ = tx.send(crate::shape_editor::demo::asset(&library, mode == "mesh-head"));
            });
        }
    }
}
