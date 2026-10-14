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
                if self.frame_count == 600 {
                    self.hover_height_ui.open = true;
                    self.panels.chat = false;
                    self.panels.people = false;
                    self.panels.perf = false;
                    self.settings.camera.hover_affects_camera = mode == "camera";
                    if let Ok(value) = mode.parse::<f32>() {
                        self.world.hover_height.preview(value);
                        self.world.hover_height.commit();
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
