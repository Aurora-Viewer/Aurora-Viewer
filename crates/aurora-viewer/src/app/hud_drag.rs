//! Configurable held-key HUD gestures, editing view zoom and offline scenarios.
use super::*;
use aurora_render::HudView;
use uuid::Uuid;

impl App {
    pub(super) fn refresh_local_huds(&mut self) {
        self.world.refresh_hud_scales();
        let selected = self.build.roots(&self.world);
        for command in hud_metadata_commands(&mut self.world, &selected, Instant::now()) {
            self.send(command);
        }
    }
    /// Own the wheel before media or camera, even on a no-modify/locked HUD.
    pub(super) fn on_hud_scroll(&mut self, steps: f32) -> bool {
        self.build.sync_hud_zoom(&mut self.world);
        if !self.settings.show_huds
            || self.camera.mouselook()
            || !matches!(self.screen, Screen::World)
            || self.mouse_mode != MouseMode::None
        {
            return false;
        }
        if !self.hud_drag_requested() {
            if !self.build.open || !self.build.hud_selected(&self.world) {
                return false;
            }
            // LLAgentCamera::handleScrollWheel / cameraZoomIn (Legacy Viewer,
            // originally LGPL 2.1): a HUD selection zooms the entire HUD view.
            // No hit under the cursor is needed; local scale profiles stay intact.
            if self.hud_drag.is_none() && !self.build.manip.dragging() && self.interactions.held.is_none() {
                self.world.hud_zoom = hud_view_zoom(self.world.hud_zoom, steps);
            }
            return true;
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
        self.world.hud_scales.set(id, scale);
        self.world.dirty_hud(root);
        self.refresh_local_huds();
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
        // View zoom must only change the projection, not local HUD positions.
        self.world.hud_scales.prepare_clamp([view.width, view.height]);
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
            // (a partly loaded linkset is measured again once it is complete)
            if !self.world.hud_scales.needs_clamp(id) || !self.scene.hud_loaded(&self.world, root) {
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

    /// render_ui_2d (indra/newview/llviewerdisplay.cpp): the normal screen
    /// shrinks with the common HUD projection, revealing offscreen attachments.
    pub(super) fn draw_hud_view_bounds(&self, ctx: &egui::Context) {
        if !self.huds_shown() || !self.build.open || !self.build.hud_selected(&self.world) {
            return;
        }
        let Some(view) = self.scene.lists.hud_view.filter(|view| view.zoom < 1.0) else {
            return;
        };
        let [left, top, right, bottom] = view.screen_bounds();
        let ppp = ctx.pixels_per_point();
        let rect = egui::Rect::from_min_max(egui::pos2(left / ppp, top / ppp), egui::pos2(right / ppp, bottom / ppp));
        ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hud_view_bounds")))
            .rect_stroke(rect, 0.0, egui::Stroke::new(1.0, self.palette.ink), egui::StrokeKind::Inside);
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
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("hud_scale")));
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
        // Build tools own their modifiers, including Ctrl for rotation.
        // The gesture's Control key is the physical key. Command is a
        // shortcut modifier on macOS and must not start the drag.
        !self.build.open && self.settings.keybinds.held(Action::HudDrag, &self.down, self.physical_mods())
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
        if self.build.open {
            // Finish any gesture already underway before editing took over,
            // without following the pointer into a build operation.
            if let Some(cmd) = self.hud_drag.take().and_then(|drag| drag.release(&self.world)) {
                self.send(NetCommand::Build(cmd));
            }
            return;
        }
        if let Some(drag) = &self.hud_drag
            && !view.is_some_and(|view| drag.update(&mut self.world, view, self.cursor_pos))
        {
            self.hud_drag = None;
        }
    }

    pub(super) fn demo_hud_drag_steps(&mut self) {
        self.demo_hud_view_zoom_steps();
        self.demo_hud_scale_steps();
        if !self.demo || !std::env::var("AURORA_DEMO_HUDS").is_ok_and(|mode| mode.starts_with("drag")) {
            return;
        }
        let mode = std::env::var("AURORA_DEMO_HUDS").unwrap_or_default();
        if (self.frame_count == 680 && mode.starts_with("drag-lock")) || (self.frame_count == 700 && mode == "drag-lock-unlock") {
            self.on_ctx_action(ui::context::CtxAction::SetHudMoveLock(
                crate::world::objects::ObjKey {
                    region: self.world.main_region.unwrap_or_default(),
                    local_id: crate::demo::hud::CHILD,
                },
                self.frame_count == 680,
            ));
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
            if mode == "drag-lock-camera" {
                self.alt = true;
            }
            self.on_left_press();
            log::info!(
                "demo HUD drag start: active={}, script_touch={}, camera={:?}, key={}",
                self.hud_drag.is_some(),
                self.interactions.held.is_some(),
                self.mouse_mode,
                self.demo_hud_drag_binding().map(|b| b.label()).unwrap_or_default()
            );
        } else if (740..840).contains(&self.frame_count) {
            if mode == "drag-lock-camera" {
                self.camera_mouse_delta(1.2, -0.7);
                if self.frame_count == 780 {
                    log::info!(
                        "demo HUD locked camera: mode={:?}, hud_drag={}",
                        self.mouse_mode,
                        self.hud_drag.is_some()
                    );
                }
            }
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
        } else if self.frame_count == 880 && mode == "drag-lock" {
            let key = crate::world::objects::ObjKey {
                region: self.world.main_region.unwrap_or_default(),
                local_id: crate::demo::hud::CHILD,
            };
            if let Some(idx) = self.world.objects.index_of(&key)
                && let Some(view) = self.scene.lists.hud_view
                && let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0)
            {
                self.cursor_pos = (
                    view.width * 0.5 - pos.y * view.height * view.zoom,
                    view.height * 0.5 - pos.z * view.height * view.zoom,
                );
                self.open_context_menu();
            }
        }
    }

    fn demo_hud_view_zoom_steps(&mut self) {
        if !self.demo || !std::env::var("AURORA_DEMO_HUDS").is_ok_and(|mode| mode == "edit-scroll") {
            return;
        }
        let Some(root) = self.world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id) else {
            return;
        };
        if self.frame_count == 720 {
            self.build.open_build(crate::build::Tool::Edit);
            self.build.click_select(&mut self.world, &self.settings.build, Some(root), false);
            // Start with distinct local sizes, including a HUD outside the view.
            let id = self.world.objects.get(root).map(|o| o.full_id);
            if let Some(id) = id {
                self.world.hud_scales.set(id, 2.0);
                self.world.dirty_hud(root);
            }
            self.demo_hud_drag_key(false);
        } else if self.frame_count == 740 {
            if let Some(o) = self.world.objects.get_mut(root) {
                o.position.y = 1.2;
                o.render.needs_records = true;
            }
        } else if self.frame_count == 760 || self.frame_count == 900 {
            self.cursor_pos = (10.0, 300.0); // Empty space still zooms all HUDs.
            let before = self.camera.position;
            let handled = self.on_hud_scroll(if self.frame_count == 760 { -5.0 } else { 5.0 });
            log::info!(
                "demo HUD editing view: handled={handled}, view_zoom={:.3}, local_scale={:.3}, camera_unchanged={}",
                self.world.hud_zoom,
                self.world.hud_scale(root),
                self.camera.position == before
            );
        } else if self.frame_count == 820 {
            if let Some(view) = self.scene.lists.hud_view
                && let Some((pos, _, _)) = Scene::object_transform(&self.world, root, Instant::now(), 0)
            {
                self.cursor_pos = (
                    view.width * 0.5 - pos.y * view.height * view.zoom,
                    view.height * 0.5 - pos.z * view.height * view.zoom,
                );
                self.demo_hud_drag_key(true);
                let position = self.world.objects.get(root).map(|o| o.position);
                let scale = self.world.hud_scale(root);
                self.on_left_press();
                let dragged = self.hud_drag.is_some();
                self.cursor_pos.0 += 40.0;
                self.update_hud_drag(Some(view));
                self.on_left_release();
                let handled = self.on_hud_scroll(1.0);
                self.demo_hud_drag_key(false);
                log::info!(
                    "demo HUD editing gesture: drag={dragged}, position_unchanged={}, local_scale_unchanged={}, wheel_handled={handled}",
                    self.world.objects.get(root).map(|o| o.position) == position,
                    self.world.hud_scale(root) == scale
                );
            }
        } else if self.frame_count == 910 || self.frame_count == 930 || self.frame_count == 950 {
            self.build.open_build(crate::build::Tool::Edit);
            self.build.click_select(&mut self.world, &self.settings.build, Some(root), false);
            self.on_hud_scroll(-5.0);
            log::info!(
                "demo HUD view before transition: frame={}, view_zoom={:.3}",
                self.frame_count,
                self.world.hud_zoom
            );
        } else if self.frame_count == 920 {
            self.build.close(&mut self.world, &mut self.settings.build);
            let before = self.world.hud_zoom;
            let handled = self.on_hud_scroll(-1.0);
            log::info!(
                "demo HUD view outside edit: handled={handled}, view_unchanged={}, reset={}",
                self.world.hud_zoom == before,
                self.world.hud_zoom == 1.0
            );
        } else if self.frame_count == 940 {
            let ground = self.world.objects.iter().find_map(|(idx, _)| {
                (crate::build::selectable(&self.world, idx)
                    && Scene::object_transform(&self.world, idx, Instant::now(), 0).is_some_and(|(_, _, hud)| !hud))
                .then_some(idx)
            });
            self.build.open_build(crate::build::Tool::Edit);
            self.build.click_select(&mut self.world, &self.settings.build, ground, false);
            let handled = self.on_hud_scroll(-1.0);
            log::info!(
                "demo HUD view with world selection: handled={handled}, reset={}",
                self.world.hud_zoom == 1.0
            );
        } else if self.frame_count == 960 {
            let next = self.world.objects.index_of_uuid(&crate::demo::hud::object(32, false).full_id);
            let before = self.world.hud_zoom;
            self.build.click_select(&mut self.world, &self.settings.build, next, false);
            self.build.sync_hud_zoom(&mut self.world);
            log::info!("demo HUD view with another HUD: view_unchanged={}", self.world.hud_zoom == before);
        } else if self.frame_count == 970 {
            self.build.deselect_all(&self.world);
            self.build.sync_hud_zoom(&mut self.world);
            log::info!("demo HUD view with empty selection: reset={}", self.world.hud_zoom == 1.0);
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
        } else if self.frame_count == 900
            && matches!(
                mode.as_str(),
                "scale-copy" | "scale-reattach" | "scale-rename" | "scale-temp" | "scale-temp-item" | "scale-temp-instance"
            )
        {
            if let Some(root) = root
                && let Some(handle) = self.world.objects.get(root).map(|o| o.key.region)
            {
                let mut update = crate::demo::hud::object(31, false);
                update.full_id = Uuid::from_u128(8399);
                if matches!(mode.as_str(), "scale-temp" | "scale-temp-item" | "scale-temp-instance") {
                    update.name_values = if mode == "scale-temp-item" {
                        format!("AttachItemID STRING RW SV {}", Uuid::from_u128(999_002))
                    } else if mode == "scale-temp-instance" {
                        format!("AttachItemID STRING RW SV {}", update.full_id)
                    } else {
                        String::new()
                    };
                } else if let Some(mut item) = self
                    .world
                    .objects
                    .get(root)
                    .and_then(|o| o.attachment_item_id())
                    .and_then(|id| self.world.inventory.items.get(&id))
                    .cloned()
                {
                    if mode == "scale-copy" {
                        item.id = update.full_id;
                        item.name = aurora_i18n::tr!("demo-hud-copy-inventory-name").into();
                    } else if mode == "scale-rename" {
                        item.name = aurora_i18n::tr!("demo-hud-renamed-inventory-name").into();
                    }
                    update.name_values = format!("AttachItemID STRING RW SV {}", item.id);
                    self.world.inventory.add_items(vec![item]);
                }
                // Remove the linkset first: the new rezzed object must restore
                // from its inventory item, with no surviving local instance.
                self.world.objects.remove_idx(root);
                self.world.refresh_hud_scales();
                let mut objects = vec![update];
                for event in crate::demo::hud::events() {
                    if let NetEvent::ObjectUpdates { objects: updates, .. } = event {
                        objects.extend(updates.into_iter().filter(|o| o.local_id == crate::demo::hud::CHILD).map(|mut o| {
                            o.full_id = Uuid::from_u128(8398);
                            o
                        }));
                    }
                }
                self.world.apply(NetEvent::ObjectUpdates { handle, objects });
            }
        } else if self.frame_count == 904
            && matches!(
                mode.as_str(),
                "scale-copy" | "scale-reattach" | "scale-rename" | "scale-temp" | "scale-temp-item" | "scale-temp-instance"
            )
        {
            if let Some(root) = self.world.objects.index_of_uuid(&Uuid::from_u128(8399)) {
                log::info!(
                    "demo HUD replacement: mode={mode}, restored_scale={:.3}",
                    self.world.hud_scale(root)
                );
            }
        } else if self.frame_count == 880 && mode == "scale-touch" {
            self.on_left_press();
        } else if self.frame_count == 900 && mode == "scale-touch" {
            self.on_left_release();
        }
    }
}

/// ObjectProperties contains the creator; ObjectPropertiesFamily does not.
/// Use LLSelectMgr's full ObjectSelect / ObjectDeselect messages without changing
/// the local selection, and keep anything already selected selected on the sim.
/// Source: Firestorm indra/newview/llselectmgr.cpp, originally LGPL 2.1.
fn hud_metadata_commands(world: &mut World, selected: &[usize], now: Instant) -> Vec<NetCommand> {
    let Some(me) = world.objects.index_of_uuid(&world.agent_id).and_then(|i| world.objects.get(i)) else {
        return Vec::new();
    };
    let roots: Vec<_> = world
        .objects
        .children_of(&me.key)
        .iter()
        .filter_map(|idx| {
            let o = world.objects.get(*idx)?;
            (31..=38)
                .contains(&o.attachment_point())
                .then_some((*idx, o.full_id, o.key, o.attachment_item_id()))
        })
        .collect();
    let mut regions = std::collections::BTreeMap::<_, (Vec<u32>, Vec<u32>)>::new();
    let mut items = Vec::new();
    for (root, id, key, item) in roots {
        if world.hud_scales.request_properties(id, now) {
            let (select, deselect) = regions.entry(key.region).or_default();
            select.push(key.local_id);
            if !selected.contains(&root) {
                deselect.push(key.local_id);
            }
        }
        if let Some(item) = item.filter(|item| !item.is_nil() && (*item != id || world.inventory.items.contains_key(item)))
            && !world
                .inventory
                .items
                .get(&item)
                .is_some_and(|it| !it.creator.is_nil() && !it.name.trim().is_empty())
            && world.hud_scales.request_item(id, now)
            && !items.contains(&item)
        {
            items.push(item);
        }
    }
    let mut commands = Vec::new();
    for (handle, (local_ids, deselect)) in regions {
        commands.push(NetCommand::Build(aurora_net::build::BuildCmd::Select { handle, local_ids }));
        if !deselect.is_empty() {
            commands.push(NetCommand::Build(aurora_net::build::BuildCmd::Deselect {
                handle,
                local_ids: deselect,
            }));
        }
    }
    if !items.is_empty() {
        commands.push(NetCommand::FetchItems {
            items,
            owner: world.agent_id,
        });
    }
    commands
}

/// Legacy Viewer's wheel factor with winit's opposite scroll sign; same bounds
/// as Aurora's World > HUDs view zoom controls (never a per-object resize).
fn hud_view_zoom(zoom: f32, steps: f32) -> f32 {
    if !steps.is_finite() {
        return zoom;
    }
    (zoom * 0.8_f32.powf(-steps.clamp(-100.0, 100.0))).clamp(0.1, 1.0)
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
    fn hud_metadata_queries_are_batched_retry_and_preserve_the_existing_selection() {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(event);
        }
        world.hud_scales = crate::world::hud_scale::HudScales::default();
        crate::demo::hud::seed_inventory(&mut world);
        world.refresh_hud_scales();
        let id = crate::demo::hud::object(31, false).full_id;
        let selected = world.objects.index_of_uuid(&id).unwrap();
        let now = Instant::now();
        let commands = hud_metadata_commands(&mut world, &[selected], now);
        assert_eq!(commands.len(), 2);
        assert!(
            matches!(&commands[0], NetCommand::Build(aurora_net::build::BuildCmd::Select { local_ids, .. })
            if local_ids.len() == 8 && local_ids.contains(&crate::demo::hud::FIRST) && !local_ids.contains(&8241))
        );
        assert!(
            matches!(&commands[1], NetCommand::Build(aurora_net::build::BuildCmd::Deselect { local_ids, .. })
            if local_ids.len() == 7 && !local_ids.contains(&crate::demo::hud::FIRST))
        );
        assert!(hud_metadata_commands(&mut world, &[selected], now + Duration::from_secs(9)).is_empty());
        assert_eq!(
            hud_metadata_commands(&mut world, &[selected], now + Duration::from_secs(10)).len(),
            2
        );
        let props: Vec<_> = (31..=38)
            .map(|point| aurora_net::build::ObjectProps {
                object_id: crate::demo::hud::object(point, false).full_id,
                name: format!("HUD {point}"),
                creator_id: Uuid::from_u128(10),
                full: true,
                ..Default::default()
            })
            .collect();
        world.on_hud_properties(&props);
        assert!(hud_metadata_commands(&mut world, &[selected], now + Duration::from_secs(20)).is_empty());
    }

    #[test]
    fn editing_view_wheel_reveals_offscreen_huds_and_preserves_local_sizes() {
        let view_zoom = hud_view_zoom(1.0, -5.0);
        let view = HudView::new([1600, 900], view_zoom, -1.0, 1.0);
        let screen_x = view.width * 0.5 - 1.2 * view.height * view.zoom;
        assert!((0.0..view.width).contains(&screen_x));
        assert!((hud_view_zoom(view_zoom, 5.0) - 1.0).abs() < 1e-5);
        assert_eq!(hud_view_zoom(view_zoom, f32::NAN), view_zoom);
        assert_eq!(hud_view_zoom(0.1, -1000.0), 0.1);
        assert_eq!(hud_view_zoom(1.0, 1000.0), 1.0);
        let mut scales = crate::world::hud_scale::HudScales::default();
        let id = Uuid::from_u128(1);
        scales.set(id, 2.0);
        scales.prepare_clamp([view.width, view.height]);
        scales.set_offset(id, Vec3::Y * 0.2);
        scales.clamped(id);
        let restored = HudView::new([1600, 900], hud_view_zoom(view_zoom, 5.0), -1.0, 1.0);
        scales.prepare_clamp([restored.width, restored.height]);
        assert!(!scales.needs_clamp(id));
        assert_eq!(scales.scale(id), 2.0);
        assert_eq!(scales.offset(id), Vec3::Y * 0.2);
    }

    #[test]
    fn oversized_and_offscreen_huds_keep_a_reachable_strip() {
        assert_eq!(accessible_delta([-1000.0, -1000.0, 2000.0, 2000.0], [1280.0, 720.0]), [0.0, 0.0]);
        assert_eq!(accessible_delta([1300.0, 10.0, 1400.0, 110.0], [1280.0, 720.0]), [-44.0, 0.0]);
        assert_eq!(accessible_delta([-110.0, -110.0, -10.0, -10.0], [1280.0, 720.0]), [34.0, 34.0]);
    }
}
