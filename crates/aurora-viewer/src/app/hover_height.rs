use super::App;

impl App {
    pub(super) fn update_hover_height(&mut self) {
        if self.demo {
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
        let region = self.world.hover_height_available().then_some(self.world.main_region).flatten();
        if let Some(command) = self.world.hover_height.next_update(region) {
            self.send(command);
        }
    }
}
