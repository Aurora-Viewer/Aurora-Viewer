//! Configurable held-key + left-button HUD gesture and its offline scenario.
use super::*;
use aurora_render::HudView;
use uuid::Uuid;

impl App {
    pub(super) fn refresh_local_huds(&mut self) {
        self.world.refresh_hud_scales();
        let Some(me) = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
        else {
            return;
        };
        let missing: Vec<_> = self
            .world
            .objects
            .children_of(&me.key)
            .iter()
            .filter_map(|idx| {
                let o = self.world.objects.get(*idx)?;
                if !(31..=38).contains(&o.attachment_point()) {
                    return None;
                }
                o.attachment_item_id()
                    .filter(|item| !self.world.inventory.items.contains_key(item))
                    .map(|item| (o.full_id, item))
            })
            .collect();
        let items: Vec<_> = missing
            .into_iter()
            .filter(|(id, _)| self.world.hud_scales.request_item(*id))
            .map(|(_, item)| item)
            .collect();
        if !items.is_empty() {
            self.send(NetCommand::FetchItems {
                items,
                owner: self.world.agent_id,
            });
        }
    }
    /// Own the wheel before media or camera, even on a no-modify/locked HUD.
    pub(super) fn on_hud_scroll(&mut self, steps: f32) -> bool {
        if !self.hud_drag_requested()
            || !self.settings.show_huds
            || self.camera.mouselook()
            || !matches!(self.screen, Screen::World)
            || self.mouse_mode != MouseMode::None
        {
            return false;
        }
        let Some(idx) = self.scene.hud_move_pick(&self.world, self.cursor_pos) else {
            return false;
        };
        let Some(root) = self.world.hud_root(idx) else { return false };
        if self.hud_drag.is_some() || self.build.manip.dragging() || self.interactions.held.is_some() {
            return true;
        }
        self.world.refresh_hud_scales();
        let Some(o) = self.world.objects.get(root) else { return true };
        let id = o.full_id;
        let scale = crate::world::hud_scale::wheel_scale(self.world.hud_scales.scale(id), steps, self.shift);
        let missing_item = o.attachment_item_id().filter(|item| !self.world.inventory.items.contains_key(item));
        self.world.hud_scales.set(id, scale);
        self.world.dirty_hud(root);
        if let Some(item) = missing_item
            && self.world.hud_scales.request_item(id)
        {
            self.send(NetCommand::FetchItems {
                items: vec![item],
                owner: self.world.agent_id,
            });
        }
        log::debug!("HUD local scale: {:.0}%", scale * 100.0);
        true
    }

    pub(super) fn reset_hud_scale(&mut self, key: crate::world::objects::ObjKey, forget: bool) {
        let Some(root) = self.world.objects.index_of(&key).and_then(|idx| self.world.hud_root(idx)) else {
            return;
        };
        let Some(o) = self.world.objects.get(root) else { return };
        let id = o.full_id;
        if forget {
            self.world.hud_scales.forget(id);
        } else {
            self.world.hud_scales.set(id, 1.0);
            self.world.hud_scales.set_offset(id, Vec3::ZERO);
            self.world.dirty_hud(root);
        }
    }

    pub(super) fn keep_scaled_huds_accessible(&mut self, view: HudView) {
        if self.hud_drag.is_some() || self.build.manip.dragging() {
            return;
        }
        self.world.hud_scales.prepare_clamp([view.width, view.height, view.zoom]);
        let Some(me) = self
            .world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| self.world.objects.get(i))
        else {
            return;
        };
        let roots = self.world.objects.children_of(&me.key).to_vec();
        for root in roots {
            let Some(o) = self.world.objects.get(root) else { continue };
            let id = o.full_id;
            if !self.world.hud_scales.needs_clamp(id) {
                continue;
            }
            let Some(rect) = self.scene.hud_rect(&self.world, root, view) else {
                continue;
            };
            let delta = accessible_delta(rect, [view.width, view.height]);
            self.world.hud_scales.clamped(id);
            if delta != [0.0, 0.0] {
                let px = view.height * view.zoom;
                self.world.hud_scales.set_offset(
                    id,
                    self.world.hud_scales.offset(id) + Vec3::new(0.0, -delta[0] / px, -delta[1] / px),
                );
                self.world.dirty_hud(root);
            }
        }
    }

    pub(super) fn draw_hud_scale(&self, ctx: &egui::Context) {
        if !self.hud_drag_requested()
            || !self.settings.show_huds
            || self.camera.mouselook()
            || self.mouse_mode != MouseMode::None
            || ctx.is_pointer_over_egui()
            || ctx.egui_wants_pointer_input()
        {
            return;
        }
        let Some(view) = self.scene.lists.hud_view else { return };
        let Some(idx) = self.scene.hud_move_pick(&self.world, self.cursor_pos) else {
            return;
        };
        let Some(bounds) = self.scene.hud_rect(&self.world, idx, view) else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        let rect = egui::Rect::from_min_max(
            egui::pos2(bounds[0] / ppp, bounds[1] / ppp),
            egui::pos2(bounds[2] / ppp, bounds[3] / ppp),
        );
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("hud_scale")));
        painter.rect_stroke(
            rect.expand(2.0),
            2.0,
            egui::Stroke::new(1.0, self.palette.violet_light),
            egui::StrokeKind::Outside,
        );
        let text = format!("{:.0} %", self.world.hud_scale(idx) * 100.0);
        let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), self.palette.ink);
        let screen = ctx.content_rect();
        let label = egui::pos2(
            rect.left()
                .clamp(screen.left(), (screen.right() - galley.size().x - 8.0).max(screen.left())),
            (rect.top() - galley.size().y - 8.0).clamp(screen.top(), (screen.bottom() - galley.size().y - 8.0).max(screen.top())),
        );
        painter.rect_filled(
            egui::Rect::from_min_size(label, galley.size() + egui::vec2(8.0, 6.0)),
            2.0,
            self.palette.panel,
        );
        painter.galley(label + egui::vec2(4.0, 3.0), galley, self.palette.ink);
    }

    pub(super) fn hud_drag_requested(&self) -> bool {
        self.settings.keybinds.held(Action::HudDrag, &self.down, self.mods())
    }

    fn demo_hud_drag_binding(&self) -> Option<crate::keybinds::Binding> {
        let slot =
            usize::from(std::env::var("AURORA_DEMO_HUDS").is_ok_and(|mode| matches!(mode.as_str(), "drag-secondary" | "scale-secondary")));
        self.settings.keybinds.get(Action::HudDrag)[slot].clone()
    }

    fn demo_hud_drag_key(&mut self, pressed: bool) {
        let Some(binding) = self.demo_hud_drag_binding() else { return };
        self.ctrl = pressed && binding.ctrl;
        self.alt = pressed && binding.alt;
        self.shift = pressed && binding.shift;
        match &binding.input {
            Input::Key(code) if matches!(code.as_str(), "AltLeft" | "AltRight") => self.alt = pressed,
            Input::Key(code) if matches!(code.as_str(), "ControlLeft" | "ControlRight") => self.ctrl = pressed,
            Input::Key(code) if matches!(code.as_str(), "ShiftLeft" | "ShiftRight") => self.shift = pressed,
            input => {
                if pressed {
                    self.down.insert(input.clone());
                } else {
                    self.down.remove(input);
                }
            }
        }
    }
    pub(super) fn update_hud_drag(&mut self, view: Option<HudView>) {
        if let Some(drag) = &self.hud_drag
            && !view.is_some_and(|view| drag.update(&mut self.world, view, self.cursor_pos))
        {
            self.hud_drag = None;
        }
    }

    pub(super) fn demo_hud_drag_steps(&mut self) {
        self.demo_hud_scale_steps();
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
            self.demo_hud_drag_key(true);
            self.on_left_press();
            log::info!(
                "demo HUD drag start: active={}, script_touch={}, camera={:?}, key={}",
                self.hud_drag.is_some(),
                self.interactions.held.is_some(),
                self.mouse_mode,
                self.demo_hud_drag_binding().map(|b| b.label()).unwrap_or_default()
            );
        } else if (740..840).contains(&self.frame_count) {
            self.cursor_pos.0 += 1.2;
            self.cursor_pos.1 -= 0.7;
            if self.frame_count == 800 {
                self.demo_hud_drag_key(false);
                log::info!("demo HUD drag: gesture key released while left button stays down");
            }
        } else if self.frame_count == 840 {
            self.on_left_release();
            self.demo_hud_drag_key(false);
            log::info!(
                "demo HUD drag end: active={}, script_touch={}, camera={:?}",
                self.hud_drag.is_some(),
                self.interactions.held.is_some(),
                self.mouse_mode
            );
        }
    }

    fn demo_hud_scale_steps(&mut self) {
        if !self.demo || !std::env::var("AURORA_DEMO_HUDS").is_ok_and(|mode| mode.starts_with("scale")) {
            return;
        }
        let mode = std::env::var("AURORA_DEMO_HUDS").unwrap_or_default();
        let root = self.world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id);
        if self.frame_count == 720
            && let (Some(root), Some(view)) = (root, self.scene.lists.hud_view)
            && let Some((pos, _, _)) = Scene::object_transform(&self.world, root, Instant::now(), 0)
        {
            if mode == "scale"
                && let Some(o) = self.world.objects.get_mut(root)
            {
                o.update_flags = 0;
            }
            self.cursor_pos = (
                view.width * 0.5 - pos.y * view.height * view.zoom,
                view.height * 0.5 - pos.z * view.height * view.zoom,
            );
            self.demo_hud_drag_key(true);
            let before = self.camera.position;
            let handled = self.on_hud_scroll(5.0);
            log::info!(
                "demo HUD local zoom: handled={handled}, scale={:.3}, camera_unchanged={}, script_touch={}",
                self.world.hud_scale(root),
                self.camera.position == before,
                self.interactions.held.is_some()
            );
        } else if self.frame_count == 840 {
            self.demo_hud_drag_key(false);
            if let Some(root) = root {
                let key = self.world.objects.get(root).map(|o| o.key);
                if mode == "scale-reset"
                    && let Some(key) = key
                {
                    self.reset_hud_scale(key, false);
                }
                if mode == "scale-forget"
                    && let Some(key) = key
                {
                    self.reset_hud_scale(key, true);
                }
            }
        } else if self.frame_count == 900 && mode == "scale-copy" {
            if let Some(root) = root
                && let Some(item) = self.world.objects.get(root).and_then(|o| o.attachment_item_id())
                && let Some(mut item) = self.world.inventory.items.get(&item).cloned()
                && let Some(handle) = self.world.objects.get(root).map(|o| o.key.region)
            {
                let mut update = crate::demo::hud::object(31, false);
                update.full_id = Uuid::from_u128(8399);
                item.id = update.full_id;
                update.name_values = format!("AttachItemID STRING RW SV {}", item.id);
                self.world.inventory.add_items(vec![item]);
                self.world.apply(NetEvent::ObjectUpdates {
                    handle,
                    objects: vec![update],
                });
            }
        } else if self.frame_count == 902 && mode == "scale-copy" {
            if let Some(root) = self.world.objects.index_of_uuid(&Uuid::from_u128(8399)) {
                log::info!("demo HUD replacement: restored_scale={:.3}", self.world.hud_scale(root));
            }
        } else if self.frame_count == 880 && mode == "scale-touch" {
            self.on_left_press();
        } else if self.frame_count == 900 && mode == "scale-touch" {
            self.on_left_release();
        }
    }
}

/// Keep at least 24 pixels visible; small panels keep their full width/height.
fn accessible_delta(rect: [f32; 4], screen: [f32; 2]) -> [f32; 2] {
    std::array::from_fn(|axis| {
        let margin = (rect[axis + 2] - rect[axis]).clamp(0.0, 24.0).min(screen[axis] * 0.5);
        if rect[axis + 2] < margin {
            margin - rect[axis + 2]
        } else if rect[axis] > screen[axis] - margin {
            screen[axis] - margin - rect[axis]
        } else {
            0.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_and_offscreen_huds_keep_a_reachable_strip() {
        assert_eq!(accessible_delta([-1000.0, -1000.0, 2000.0, 2000.0], [1280.0, 720.0]), [0.0, 0.0]);
        assert_eq!(accessible_delta([1300.0, 10.0, 1400.0, 110.0], [1280.0, 720.0]), [-44.0, 0.0]);
        assert_eq!(accessible_delta([-110.0, -110.0, -10.0, -10.0], [1280.0, 720.0]), [34.0, 34.0]);
    }
}
