//! Alt + left-button HUD gesture and its offline input scenario.
use super::*;
use aurora_render::HudView;

impl App {
    pub(super) fn update_hud_drag(&mut self, view: Option<HudView>) {
        if let Some(drag) = &self.hud_drag
            && !view.is_some_and(|view| drag.update(&mut self.world, view, self.cursor_pos))
        {
            self.hud_drag = None;
        }
    }

    pub(super) fn demo_hud_drag_steps(&mut self) {
        if !self.demo || !std::env::var("AURORA_DEMO_HUDS").is_ok_and(|mode| mode.starts_with("drag")) {
            return;
        }
        let child = (self.frame_count == 720)
            .then(|| {
                self.world
                    .objects
                    .iter()
                    .find(|(_, o)| o.key.local_id == crate::demo::hud::CHILD)
                    .map(|(idx, _)| idx)
            })
            .flatten();
        if self.frame_count == 720
            && let Some(view) = self.scene.lists.hud_view
            && let Some(idx) = child
            && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0)
        {
            self.cursor_pos = (
                view.width * 0.5 - pos.y * view.height * view.zoom,
                view.height * 0.5 - pos.z * view.height * view.zoom,
            );
            self.alt = true;
            self.on_left_press();
            log::info!(
                "demo HUD drag start: active={}, script_touch={}, camera={:?}",
                self.hud_drag.is_some(),
                self.interactions.held.is_some(),
                self.mouse_mode
            );
        } else if (740..840).contains(&self.frame_count) {
            self.cursor_pos.0 += 1.2;
            self.cursor_pos.1 -= 0.7;
            if self.frame_count == 800 {
                self.alt = false;
                log::info!("demo HUD drag: Alt released while left button stays down");
            }
        } else if self.frame_count == 840 {
            self.on_left_release();
            self.alt = false;
            log::info!(
                "demo HUD drag end: active={}, script_touch={}, camera={:?}",
                self.hud_drag.is_some(),
                self.interactions.held.is_some(),
                self.mouse_mode
            );
        }
    }
}
