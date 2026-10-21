//! RLVa preference and action integration, shared by every UI layout.
use super::*;
use uuid::Uuid;

impl App {
    pub(super) fn update_rlva(&mut self) {
        self.demo_rlva();
        self.world.rlva.configure(self.settings.rlva_enabled);
        self.world
            .rlva
            .collect(Instant::now(), |id| self.world.objects.index_of_uuid(&id).is_some());
        let commands = self.world.rlva_pending();
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
            if matches!(command, NetCommand::Chat { .. }) {
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
            let event = aurora_net::ChatMessage {
                from_name: "RLVa demo object".to_owned(),
                source_id: Uuid::from_u128(0x726c7661),
                owner_id: self.world.agent_id,
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: Vec3::ZERO,
                message: "@fly=n,jump=n,unsit=n,sendchat=n,pay=n".to_owned(),
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
        if mode == "released" && self.frame_count == 760 {
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

    fn rlva_camera_commands(&mut self) {
        use crate::world::rlva::{Param, camera::vector};
        for (actor, c) in std::mem::take(&mut self.world.rlva.viewer) {
            if let Param::Reply(channel) = c.param {
                let value = match c.behaviour.as_str() {
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
