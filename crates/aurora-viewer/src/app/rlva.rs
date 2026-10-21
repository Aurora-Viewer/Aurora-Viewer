//! RLVa preference and action integration, shared by every UI layout.
use super::*;
use uuid::Uuid;

impl App {
    pub(super) fn draw_rlva_overlays(&self, ui: &egui::Ui) {
        if !self.world.rlva.has("setoverlay") {
            return;
        }
        let painter = ui.painter();
        for frame in self.world.rlva_overlay_frames(Instant::now()) {
            if let Some(texture) = self.ui_images.get(&frame.texture) {
                let [r, g, b, a] = frame.color;
                // theme-guard: content — script-controlled image tint, not UI decoration.
                let color = egui::Color32::from_rgba_unmultiplied(
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    (a * 255.0) as u8,
                );
                painter.image(
                    texture.id(),
                    ui.ctx().content_rect(),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    color,
                );
            }
        }
    }

    pub(super) fn rlva_overlay_blocks(&self) -> bool {
        let size = self.gfx.as_ref().map(|g| g.window.inner_size());
        let Some(size) = size.filter(|s| s.width > 0 && s.height > 0) else {
            return false;
        };
        let (u, v) = (self.cursor_pos.0 / size.width as f32, self.cursor_pos.1 / size.height as f32);
        self.world
            .rlva_overlay_frames(Instant::now())
            .iter()
            .any(|frame| frame.blocks_touch && self.ui_image_masks.get(&frame.texture).is_none_or(|mask| mask.hit(u, v)))
    }

    pub(super) fn update_rlva(&mut self) {
        self.demo_rlva();
        self.world.rlva.configure(self.settings.rlva_enabled);
        self.media.enforce_focus(&self.world);
        if !self.settings.rlva_enabled && !self.appearance_ui.pending_actor.is_nil() {
            if let Some((_, sync)) = self.appearance_ui.pending.as_mut() {
                *sync = false;
            }
            self.appearance_ui.attachments.clear();
            self.appearance_ui.pending_actor = Uuid::nil();
        }
        self.world.eep.rlva_locked = self.world.rlva.has("setenv");
        self.world.inventory.rlva_preview_mask = self.world.rlva.preview_mask();
        self.script_editors.rlva_hidden = self.world.rlva.has("viewscript");
        self.world
            .rlva
            .collect(Instant::now(), |id| self.world.objects.index_of_uuid(&id).is_some());
        self.world.rlva.appearance_busy = self.appearance_ui.pending.is_some();
        if !self.world.rlva.appearance_busy && self.world.rlva.appearance.is_empty() {
            self.world.rlva.self_detach = None;
        }
        self.world.rlva_update_stand();
        if let Some((handle, position)) = self.world.rlva.stand_return {
            self.send(NetCommand::TeleportTo {
                handle,
                position,
                look_at: self.world.agent.forward(),
            });
            self.world.rlva.stand_return = None;
        }
        let commands = self.world.rlva_pending();
        self.rlva_environment_asset();
        self.rlva_camera_commands();
        if self.appearance_ui.pending.is_none() && !self.world.rlva.appearance.is_empty() {
            let (actor, action) = self.world.rlva.appearance.remove(0);
            self.apply_appearance_action_from(action, actor);
        }
        for (actor, command) in commands {
            self.send_rlva(command, actor);
        }
        // Protocol replies are not user chat and must bypass chat redirection
        // and sendchannel, as RlvUtil::sendChatReply does.
        for command in std::mem::take(&mut self.world.rlva.out) {
            if matches!(command, NetCommand::ChatReply { .. }) {
                self.net.send(command);
            } else {
                self.send(command);
            }
        }
        if self.world.rlva.has("showinv") {
            self.panels.inventory = false;
        }
        if self.world.rlva.has("showminimap") {
            self.panels.minimap = false;
        }
        if self.world.rlva.has("showworldmap") {
            self.panels.world_map = false;
        }
        if self.world.rlva.has("alwaysrun") && self.always_run {
            self.set_always_run(false);
        }
        if self.world.rlva.has("temprun") {
            self.temp_run = None;
        }
        let invalid: Vec<_> = self
            .build
            .selection
            .iter()
            .filter(|key| self.world.objects.index_of(key).is_none_or(|i| !self.world.rlva_can_edit(i)))
            .copied()
            .collect();
        if !invalid.is_empty() {
            self.build.deselect(&self.world, &invalid);
        }
        if self.world.rlva.has("viewwireframe") {
            self.settings.debug.wireframe = false;
        }
        if self.world.rlva.has("viewtransparent") {
            self.settings.debug.show_alpha = false;
        }
    }

    fn demo_rlva(&mut self) {
        if !self.demo {
            return;
        }
        let Ok(mode) = std::env::var("AURORA_DEMO_RLVA") else {
            return;
        };
        if self.frame_count == 360 {
            self.settings.rlva_enabled = false;
            self.panels.settings = true;
            self.options_ui.page = ui::options::Page::Protections;
            self.options_ui.tab = 0;
        }
        if self.frame_count == 700 {
            self.settings.rlva_enabled = mode != "disabled";
            self.world.rlva.configure(self.settings.rlva_enabled);
            let scenario = mode.strip_suffix("-released").unwrap_or(&mode);
            let message = if scenario == "baseline" {
                self.panels.settings = false;
                "@clear".to_owned()
            } else if let Some(effect) = scenario.strip_prefix("sphere-") {
                let (number, params, strength) = match effect {
                    "blur" => (1, "0/0/0/0", 8.0),
                    "variable" => (2, "15/0/0/0", 4.0),
                    "chromatic" => (3, "12/0/-12/0", 1.0),
                    "pixel" => (4, "16/16/0/0", 1.0),
                    _ => (0, "0/0/0/0", 0.9),
                };
                self.panels.settings = false;
                format!(
                    "@setsphere=n,setsphere_mode:{number}=force,setsphere_origin:1=force,setsphere_distmin:2=force,setsphere_distmax:15=force,setsphere_valuemin:0=force,setsphere_valuemax:{strength}=force,setsphere_param:{params}=force"
                )
            } else if scenario == "overlay" {
                self.panels.settings = false;
                format!(
                    "@setoverlay=n,setoverlay_texture:{}=force,setoverlay_alpha:0.7=force,setoverlay_touch=n",
                    crate::demo::TEX_HOLES
                )
            } else if matches!(scenario, "textures" | "silhouette" | "self" | "head") {
                self.panels.settings = false;
                match scenario {
                    "textures" => format!("@setcam_textures:{}=n", crate::demo::TEX_GRADIENT),
                    "silhouette" => "@camavdist:0=n".to_owned(),
                    "self" => "@showself=n".to_owned(),
                    _ => "@showselfhead=n".to_owned(),
                }
            } else {
                "@fly=n,jump=n,unsit=n,sendchat=n,pay=n".to_owned()
            };
            let event = aurora_net::ChatMessage {
                from_name: "RLVa demo object".to_owned(),
                source_id: Uuid::from_u128(0x726c7661),
                owner_id: self.world.agent_id,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: Vec3::ZERO,
                message,
            };
            self.world.apply(NetEvent::Chat(event));
            log::info!(
                "RLVa demo: mode={mode}, enabled={}, fly={}, jump={}, unsit={}, sendchat={}",
                self.world.rlva.enabled(),
                self.world.rlva.has("fly"),
                self.world.rlva.has("jump"),
                self.world.rlva.has("unsit"),
                self.world.rlva.has("sendchat")
            );
        }
        if (mode == "released" || mode.ends_with("-released")) && self.frame_count == 760 {
            self.settings.rlva_enabled = false;
            self.world.rlva.configure(false);
            log::info!(
                "RLVa demo: disabled, all restrictions released={}",
                !self.world.rlva.has("fly") && !self.world.rlva.has("sendchat")
            );
        }
    }

    fn send_rlva(&mut self, command: NetCommand, actor: Uuid) {
        self.send_from(command, actor);
    }

    fn rlva_environment_asset(&mut self) {
        use crate::scene::settings::SettingsState;
        use crate::world::{eep::Settings as Environment, env_select::SettingsKind, inventory::FetchState};
        let Some((actor, command)) = self.world.rlva.environment_request.take() else {
            return;
        };
        if !self.world.rlva.enabled() || self.world.rlva.has_except("setenv", actor) {
            return;
        }
        let asset = if let Ok(id) = Uuid::parse_str(&command.option) {
            Some(id)
        } else {
            let root = self.world.inventory.lib_root;
            let environments = self
                .world
                .inventory
                .folders
                .values()
                .find(|f| f.library && f.info.parent == root && f.info.name.eq_ignore_ascii_case("Environments"))
                .map(|f| f.info.id);
            let mut stack = environments.map_or_else(|| vec![root], |id| vec![id]);
            let mut seen = std::collections::HashSet::new();
            let mut loading = Vec::new();
            let mut asset = None;
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(folder) = self.world.inventory.folders.get(&id) else {
                    continue;
                };
                if folder.state != FetchState::Fetched {
                    loading.push(id);
                }
                if environments.is_some() {
                    stack.extend(folder.children.iter().copied());
                }
                let kind = if command.behaviour == "setenv_daycycle" {
                    SettingsKind::Day
                } else {
                    SettingsKind::Sky
                };
                asset = asset.or_else(|| {
                    folder
                        .items
                        .iter()
                        .filter_map(|id| self.world.inventory.items.get(id))
                        .find(|it| {
                            it.asset_type == 56
                                && SettingsKind::from_flags(it.flags) == Some(kind)
                                && it.name.eq_ignore_ascii_case(&command.option)
                        })
                        .map(|it| it.asset_id)
                });
            }
            if asset.is_none() && !loading.is_empty() {
                for id in loading.into_iter().take(16) {
                    self.world.inventory.request(id);
                }
                self.world.rlva.environment_request = Some((actor, command));
                return;
            }
            asset
        };
        let Some(asset) = asset.filter(|id| !id.is_nil()) else {
            return;
        };
        match self.scene.settings.get(asset) {
            SettingsState::Pending => self.world.rlva.environment_request = Some((actor, command)),
            SettingsState::Failed => {}
            SettingsState::Ready(settings) => {
                let (sky, water) = self.world.rlva_environment_frames(self.world.eep.current());
                self.world.rlva.environment = Some((actor, sky));
                match settings.as_ref() {
                    Environment::Sky(sky) => {
                        self.world.rlva.environment = Some((actor, *sky));
                        self.world.rlva.environment_day = None;
                        self.world.rlva.environment_water = Some(water);
                    }
                    Environment::Water(water) => self.world.rlva.environment_water = Some(*water),
                    Environment::Day(day) => {
                        self.world.rlva.environment_day = Some(std::sync::Arc::new(day.clone()));
                        self.world.rlva.environment_water = None;
                    }
                }
            }
        }
    }

    fn rlva_camera_commands(&mut self) {
        use crate::world::rlva::{Param, camera::vector};
        for (actor, c) in std::mem::take(&mut self.world.rlva.viewer) {
            if c.behaviour.starts_with("getdebug_") {
                if let Param::Reply(channel) = c.param {
                    let text = match c.behaviour.as_str() {
                        "getdebug_avatarsex" => self.world.rlva.pseudo_sex.map_or_else(
                            || {
                                self.world
                                    .visual_params(&self.world.agent_id)
                                    .map(|values| {
                                        u8::from(self.world.avatar_lib.shape_params.weights(values).get(&80).copied().unwrap_or(0.0) > 0.5)
                                            .to_string()
                                    })
                                    .unwrap_or_default()
                            },
                            |value| u8::from(value).to_string(),
                        ),
                        "getdebug_aspectratio" => {
                            let rect = self.egui_ctx.content_rect();
                            format!("{:.3}", rect.width() / rect.height().max(1.0))
                        }
                        "getdebug_renderresolutiondivisor"
                        | "getdebug_windlightuseatmosshaders"
                        | "getdebug_restrainedloveforbidgivetorlv" => "1".to_owned(),
                        "getdebug_restrainedlovenosetenv" => "0".to_owned(),
                        _ => String::new(),
                    };
                    self.world.rlva.reply(channel, text);
                }
                continue;
            }
            if c.behaviour == "setdebug_avatarsex" {
                if !self.world.rlva.has_except("setdebug", actor) {
                    self.world.rlva.pseudo_sex = match c.option.as_str() {
                        "true" | "1" => Some(true),
                        "false" | "0" => Some(false),
                        _ => self.world.rlva.pseudo_sex,
                    };
                }
                continue;
            }
            if c.behaviour == "getheightoffset" {
                if c.option.is_empty()
                    && let Param::Reply(channel) = c.param
                {
                    self.world
                        .rlva
                        .reply(channel, format!("{:.2}", self.world.hover_height.value() * 100.0));
                }
                continue;
            }
            if c.behaviour == "adjustheight" {
                if self.world.hover_height_available()
                    && let Some(height) = crate::world::rlva::camera::height_offset(&c.option, self.pelvis_to_foot())
                {
                    self.settings.auto_hover_height = false;
                    self.world.hover_height.preview(height);
                    self.world.hover_height.commit();
                }
                continue;
            }
            if let Param::Reply(channel) = c.param {
                if c.behaviour == "getcam_textures" {
                    if c.option.is_empty() {
                        self.world.rlva.reply(
                            channel,
                            self.world.rlva.camera_texture().map(|id| id.to_string()).unwrap_or_default(),
                        );
                    }
                    continue;
                }
                let value = match c.behaviour.as_str() {
                    "getcam_avdist" => self.camera.position.distance(self.world.agent.position),
                    "getcam_fov" => self.camera.fov_y,
                    "getcam_fovmin" => self.world.rlva.camera_limit("setcam_fovmin", 0.1, true),
                    "getcam_fovmax" => self.world.rlva.camera_limit("setcam_fovmax", std::f32::consts::PI - 0.1, false),
                    "getcam_avdistmin" => self.world.rlva.camera_limit("setcam_avdistmin", 0.0, true),
                    "getcam_avdistmax" => self.world.rlva.camera_limit("setcam_avdistmax", f32::MAX, false),
                    _ => continue,
                };
                if c.option.is_empty() {
                    self.world.rlva.reply(channel, format!("{value:.6}"));
                }
                continue;
            }
            if self.world.rlva.camera_owner().is_some_and(|owner| owner != actor) {
                continue;
            }
            match c.behaviour.as_str() {
                "setcam_eyeoffset" | "setcam_focusoffset" => {
                    if c.option.is_empty() {
                        self.world.rlva.camera_values.remove(&c.behaviour);
                    } else if vector(&c.option).is_some() {
                        self.world.rlva.camera_values.insert(c.behaviour, c.option);
                    }
                }
                "setcam_eyeoffsetscale" | "setcam_fov" => {
                    if c.option.is_empty() {
                        self.world.rlva.camera_values.remove(&c.behaviour);
                    } else if c
                        .option
                        .parse::<f32>()
                        .is_ok_and(|v| v.is_finite() && v > 0.0 && (c.behaviour != "setcam_fov" || v < std::f32::consts::PI))
                    {
                        self.world.rlva.camera_values.insert(c.behaviour, c.option);
                    }
                }
                "setcam_mode" => match c.option.as_str() {
                    "mouselook" if self.world.rlva.camera_value("setcam_mouselook").is_none() => {
                        self.camera.change_to_mouselook(&mut self.world.agent, &self.settings.camera, true)
                    }
                    "thirdperson" | "thirdview" => self.camera.change_to_default(&mut self.world.agent, &self.settings.camera),
                    _ => {}
                },
                "setcam_focus" => {
                    let mut parts = c.option.split(';');
                    let Some(target) = parts.next() else {
                        continue;
                    };
                    let focus = vector(target).or_else(|| {
                        Uuid::parse_str(target)
                            .ok()
                            .and_then(|id| self.world.objects.index_of_uuid(&id))
                            .and_then(|i| crate::scene::Scene::object_transform_unshifted(&self.world, i, Instant::now()))
                            .map(|(p, _, _)| p)
                    });
                    let Some(focus) = focus else {
                        continue;
                    };
                    let distance = parts
                        .next()
                        .filter(|s| !s.is_empty())
                        .map_or(Some(self.camera.position.distance(focus)), |s| s.parse::<f32>().ok());
                    let Some(distance) = distance.filter(|d| d.is_finite() && *d >= 0.0) else {
                        continue;
                    };
                    let direction = parts
                        .next()
                        .filter(|s| !s.is_empty())
                        .map_or(Some(self.camera.position - focus), vector)
                        .and_then(Vec3::try_normalize);
                    let Some(direction) = direction else {
                        continue;
                    };
                    self.camera.set_view(
                        focus + direction * distance.max(0.001),
                        focus,
                        &mut self.world.agent,
                        &self.settings.camera,
                    );
                }
                _ => {}
            }
        }
    }
}
