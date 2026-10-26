use super::App;
use crate::world::hover_height::Posture;

impl App {
    pub(super) fn update_hover_height(&mut self) {
        if self.demo {
            if let Ok(mode) = std::env::var("AURORA_DEMO_GROUNDING") {
                crate::demo::grounding::tick(&mut self.world, &mut self.scene, self.frame_count, &mode);
                if mode.contains("-transition") {
                    match self.frame_count {
                        660 | 720 => self.settings.ground_avatar_feet = mode.contains("-legacy"),
                        666 | 750 => self.settings.ground_avatar_feet = !mode.contains("-legacy"),
                        _ => {}
                    }
                }
                if self.frame_count == 600 {
                    self.settings.ground_avatar_feet = !mode.ends_with("-off") && !mode.contains("-legacy");
                    self.settings.ground_feet_while_moving = mode.contains("-moving");
                    self.settings.ground_feet_max_height = crate::settings::DEFAULT_GROUND_FEET_HEIGHT;
                    self.panels.chat = false;
                    self.panels.people = false;
                    self.panels.perf = false;
                    let yaw = if mode.contains("-front") { std::f32::consts::PI } else { 1.0 };
                    self.camera.demo_orbit(yaw, 0.45, Some(5.0), &self.settings.camera);
                }
                if mode.contains("-range") && self.frame_count == 900 {
                    self.settings.ground_feet_max_height = 1.0;
                }
            }
            let mode = std::env::var("AURORA_DEMO_HOVER_HEIGHT").ok();
            if let Some(region) = self.world.main_region.and_then(|handle| self.world.regions.get_mut(&handle)) {
                region.avatar_hover_height = mode.as_deref() != Some("unsupported");
                // Demo commands are intercepted by App::send, never HTTP.
                if !region.caps.contains_key("AgentPreferences") {
                    std::sync::Arc::make_mut(&mut region.caps).insert("AgentPreferences".into(), String::new());
                }
            }
            if let Some(mode) = mode.as_deref() {
                crate::demo::auto_hover::tick(&mut self.world, &mut self.scene, self.frame_count, mode);
                if self.frame_count == 600 {
                    self.hover_height_ui.open = true;
                    self.panels.chat = false;
                    self.panels.people = false;
                    self.panels.perf = false;
                    self.settings.camera.hover_affects_camera = mode == "camera";
                    if mode.starts_with("auto-") {
                        self.settings.auto_hover_height = mode != "auto-static-off";
                        self.settings.auto_hover_offset = 0.0;
                        // Frame the feet after login has reset the camera.
                        self.camera.demo_orbit(0.0, 0.3, Some(5.0), &self.settings.camera);
                    }
                    if let Ok(value) = mode.parse::<f32>() {
                        self.world.hover_height.preview(value);
                        self.world.hover_height.commit();
                    }
                }
                if mode == "auto-offset" {
                    match self.frame_count {
                        700 => self.settings.auto_hover_offset = 0.05,
                        860 => self.settings.auto_hover_offset = -0.03,
                        _ => {}
                    }
                }
                if mode == "auto-state" {
                    match self.frame_count {
                        700 | 840 => {
                            self.settings.auto_hover_height = false;
                            self.world.hover_height.preview(0.0);
                            self.world.hover_height.commit();
                        }
                        740 | 880 => self.settings.auto_hover_height = true,
                        _ => {}
                    }
                }
                if matches!(mode, "auto-ik" | "auto-ik-moving") {
                    if self.frame_count == 600 {
                        self.settings.ground_avatar_feet = true;
                        self.settings.ground_feet_while_moving = mode == "auto-ik-moving";
                    }
                    match self.frame_count {
                        720 => self.settings.auto_hover_height = false,
                        840 => self.settings.auto_hover_height = true,
                        _ => {}
                    }
                }
                // A separate ground height, then the sit shortcut: the avatar
                // floats 0.5 m above its ground pose, and stands back at 0.
                if mode == "sit-ground" {
                    match self.frame_count {
                        600 => {
                            self.world.hover_height.set_separate(Posture::OnGround, true);
                            self.world.hover_height.preview_seated(Posture::OnGround, 0.5);
                            self.world.hover_height.commit_seated(Posture::OnGround);
                            self.camera.demo_orbit(1.0, 0.3, Some(5.0), &self.settings.camera);
                        }
                        660 | 900 => self.sit_stand_shortcut(),
                        _ => {}
                    }
                }
                if matches!(mode, "cycle" | "camera") {
                    match self.frame_count {
                        720 => self.world.hover_height.preview(0.4),
                        780 => self.world.hover_height.commit(),
                        960 => {
                            self.world.hover_height.preview(0.0);
                            self.world.hover_height.commit();
                        }
                        _ => {}
                    }
                }
            }
        }
        let agent = &self.world.agent;
        self.world.hover_height.set_posture(if agent.ground_sit {
            Posture::OnGround
        } else if agent.seated {
            Posture::OnObject
        } else {
            Posture::Standing
        });
        let region = self.world.hover_height_target();
        if let Some(command) = self.world.hover_height.next_update(region) {
            self.send(command);
        }
    }
}
