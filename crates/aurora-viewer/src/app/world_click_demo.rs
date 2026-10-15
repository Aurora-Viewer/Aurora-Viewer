//! Offline regressions of real mouse presses (#416, #419, #420), including
//! the delayed GPU answer and the corresponding touch release.
use super::*;

fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WORLD_CLICKS").ok()).as_deref()
}

impl App {
    pub(super) fn world_click_demo_steps(&mut self) {
        let Some(mode) = mode().filter(|_| self.demo) else { return };
        if self.frame_count == 580 {
            // Saved panels must not cover the actual world press under test.
            self.panels.ao = false;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.perf = false;
            self.panels.minimap = false;
            self.panels.inventory = false;
            self.panels.settings = false;
        }
        if self.frame_count == 600 {
            self.world_click_fixture(mode);
        }
        if (680..=700).contains(&self.frame_count) {
            let cursor = if mode.starts_with("plate-") {
                self.name_tags
                    .iter()
                    .filter(|t| t.id != self.world.agent_id)
                    .min_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x))
                    .map(|t| {
                        let p = t.rect.center() * self.egui_ctx.pixels_per_point();
                        (p.x, p.y)
                    })
            } else {
                let local = if mode.starts_with("worn-") { 975 } else { 971 };
                self.world.objects.index_of_uuid(&crate::demo::action_id(local)).and_then(|idx| {
                    let pos = if mode == "worn-mesh" {
                        // A known point of the synthetic chest mesh, outside
                        // the classic body's silhouette. Its bounds' centre
                        // is inside the wearer and correctly hits the body.
                        let object = self.world.objects.get(idx)?;
                        let wearer = self.world.objects.parent_of(object)?;
                        let avatar = self.world.objects.get(wearer)?;
                        let joint = *self.world.avatar_lib.rig.names.get("mChest")?;
                        let pose = *self.world.avatar_poses.get(&avatar.full_id)?.get(joint)?;
                        let rest = self.world.avatar_lib.attach_points.get(&1)?.skel_position;
                        let (pos, rot, _) = Scene::object_transform(&self.world, wearer, Instant::now(), 0)?;
                        pos + rot * (pose.transform_point3(rest + Vec3::new(0.30, 0.0, 0.0)) - self.world.avatar_lib.pelvis)
                    } else {
                        Scene::object_transform(&self.world, idx, Instant::now(), 0)?.0
                    };
                    self.build.cam.project_px(pos)
                })
            };
            if let Some(cursor) = cursor {
                self.cursor_pos = cursor;
            }
        }
        if self.frame_count == 700 {
            self.alt = mode.starts_with("camera-");
            log::info!("demo world click: mode={mode}, cursor={:?}", self.cursor_pos);
            let over_ui = self.egui_ctx.is_pointer_over_egui() || self.egui_ctx.egui_wants_pointer_input();
            log::info!("demo world click UI: over_ui={over_ui}");
            self.mouse_input(MouseButton::Left, true, over_ui);
        }
        if self.frame_count == 701 {
            self.mouse_input(MouseButton::Left, false, false);
        }
        if self.frame_count == 740 {
            self.alt = false;
            log::info!(
                "demo world click result: mode={mode}, focus={:?}, held={}",
                self.camera.focus_point(),
                self.interactions.held.is_some()
            );
        }
    }

    fn world_click_fixture(&mut self, mode: &str) {
        if mode.starts_with("worn-") {
            let Some((wearer_idx, wearer, owner)) = self
                .world
                .objects
                .avatars()
                .find(|(_, o)| o.full_id != self.world.agent_id)
                .map(|(idx, o)| (idx, o.key.local_id, o.full_id))
            else {
                return;
            };
            // The user's saved "never render"/complexity rules must not
            // turn the fixture into a silhouette with hidden attachments.
            self.settings.set_render_exception(owner, 2);
            if let Some((pos, _, _)) = Scene::object_transform(&self.world, wearer_idx, Instant::now(), 0) {
                let focus = pos + Vec3::Z * 0.5;
                self.camera.set_view(
                    focus + Vec3::new(0.0, -4.0, 1.0),
                    focus,
                    &mut self.world.agent,
                    &self.settings.camera,
                );
            }
            let mut events = if mode == "worn-mesh" {
                if let Some(g) = self.gfx.as_mut() {
                    self.scene.install_demo_animesh(&mut g.renderer);
                }
                crate::demo::own_rigged_events(wearer)
            } else {
                vec![crate::demo::action_overlay(975, Vec3::new(0.0, 1.2, 0.0), glam::Quat::IDENTITY)]
            };
            for event in &mut events {
                if let NetEvent::ObjectUpdates { objects, .. } = event {
                    for o in objects {
                        o.local_id = 975;
                        o.full_id = crate::demo::action_id(975);
                        o.parent_id = wearer;
                        o.owner_id = owner;
                        o.state = 0x10; // chest
                        o.click_action = crate::interaction::code::TOUCH;
                        o.update_flags = ui::context::flags::HANDLE_TOUCH; // no move/modify rights
                        if mode == "worn-prim" {
                            o.scale = Vec3::splat(0.5);
                        }
                    }
                }
            }
            for event in events {
                self.world.apply(event);
            }
            return;
        }
        let point = if mode.starts_with("plate-") {
            self.settings.plate_click = crate::settings::PlateClick::Profile;
            self.name_tags
                .iter()
                .filter(|t| t.id != self.world.agent_id)
                .min_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x))
                .map(|t| t.at)
        } else {
            let mut point = None;
            for mut event in crate::demo::action_events() {
                if let NetEvent::ObjectUpdates { objects, .. } = &mut event {
                    objects.retain(|o| o.local_id == 971);
                    point = objects.first().map(|o| o.position).or(point);
                    self.world.apply(event);
                }
            }
            point
        };
        let Some(point) = point else { return };
        let direction = (self.camera.position - point).normalize_or(-Vec3::X);
        let pos = point + direction * 2.0;
        let rotation = glam::Quat::from_rotation_arc(Vec3::X, direction);
        let mut overlay = if mode == "plate-wall" {
            crate::demo::action_overlay(976, pos, rotation)
        } else {
            crate::demo::alpha_click_overlay(if mode == "camera-glass" { "blend-half" } else { "blend-zero" }, pos, rotation)
        };
        if let NetEvent::ObjectUpdates { objects, .. } = &mut overlay {
            for o in objects {
                o.scale = Vec3::new(0.12, 5.0, 5.0);
                if mode == "camera-te" {
                    o.update_flags = ui::context::flags::HANDLE_TOUCH;
                    if let Some(te) = o.texture_entry.as_mut() {
                        for f in &mut std::sync::Arc::make_mut(te).faces {
                            f.color[3] = 0.0;
                        }
                    }
                }
            }
        }
        self.world.apply(overlay);
    }
}
