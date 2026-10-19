//! Offline regressions of real mouse presses (#416, #419, #420), including
//! the delayed GPU answer and the corresponding touch release.
use super::*;

/// `worn-own`: where the prim sits on our chest's attachment point (on the
/// back, facing the default camera).
const OWN_PRIM: Vec3 = Vec3::ZERO;

/// `worn-other-link`: the child prim, without a script, of a worn linkset
/// whose root holds the touch script.
const WORN_CHILD: u32 = 988;

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
        if mode == "mouselook-touch" && self.frame_count == 690 {
            // a touchable panel right under the crosshair
            let pos = self.camera.position + self.camera.forward() * 2.0;
            let rotation = glam::Quat::from_rotation_arc(Vec3::X, -self.camera.forward());
            let mut panel = crate::demo::action_overlay(975, pos, rotation);
            if let NetEvent::ObjectUpdates { objects, .. } = &mut panel {
                for o in objects {
                    o.click_action = crate::interaction::code::TOUCH;
                    o.update_flags = ui::context::flags::HANDLE_TOUCH;
                }
            }
            self.world.apply(panel);
        }
        if (680..=700).contains(&self.frame_count) && !mode.starts_with("mouselook-") {
            let cursor = if mode == "ground" {
                // the plaza's floor, below the avatar
                self.gfx.as_ref().map(|g| {
                    let (w, h) = g.renderer.size();
                    (w as f32 * 0.3, h as f32 * 0.85)
                })
            } else if mode.starts_with("plate-") {
                self.name_tags
                    .iter()
                    .filter(|t| t.id != self.world.agent_id)
                    .min_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x))
                    .map(|t| {
                        let p = t.rect.center() * self.egui_ctx.pixels_per_point();
                        (p.x, p.y)
                    })
            } else {
                let local = match mode {
                    "worn-other-link" => WORN_CHILD,
                    _ if mode.starts_with("worn-") => 975,
                    _ => 971,
                };
                self.world.objects.index_of_uuid(&crate::demo::action_id(local)).and_then(|idx| {
                    let pos = if ["worn-mesh", "worn-own", "worn-other", "worn-other-covered"].contains(&mode) {
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
                        let offset = if mode == "worn-mesh" { Vec3::new(0.30, 0.0, 0.0) } else { OWN_PRIM };
                        pos + rot * (pose.transform_point3(rest + offset) - self.world.avatar_lib.pelvis)
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
            let over_ui = self.ui_takes_pointer();
            log::info!("demo world click UI: over_ui={over_ui}");
            self.mouse_input(MouseButton::Left, true, over_ui);
        }
        if self.frame_count == 701 {
            self.mouse_input(MouseButton::Left, false, false);
        }
        if self.frame_count == 740 {
            self.alt = false;
            log::info!(
                "demo world click result: mode={mode}, focus={:?}, held={}, steer={}",
                self.camera.focus_point(),
                self.interactions.held.is_some(),
                self.mouse_mode == MouseMode::Steer
            );
        }
    }

    fn world_click_fixture(&mut self, mode: &str) {
        if mode.starts_with("mouselook-") {
            // a weapon took the mouselook button (llTakeControls, passed on)
            if mode == "mouselook-gun" {
                use aurora_net::control::{ML_LBUTTON_DOWN, ML_LBUTTON_UP};
                self.world
                    .apply(NetEvent::ScriptControlChange(vec![(true, ML_LBUTTON_DOWN | ML_LBUTTON_UP, true)]));
            }
            self.toggle_mouselook();
            return;
        }
        if mode == "ground" {
            return;
        }
        if mode.starts_with("worn-") {
            // `worn-own`: the prim is ours (issue #488)
            let own = mode == "worn-own";
            let Some((wearer_idx, wearer, owner)) = self
                .world
                .objects
                .avatars()
                .find(|(_, o)| (o.full_id == self.world.agent_id) == own)
                .map(|(idx, o)| (idx, o.key.local_id, o.full_id))
            else {
                return;
            };
            // The user's saved "never render"/complexity rules must not
            // turn the fixture into a silhouette with hidden attachments.
            if !own {
                self.settings.set_render_exception(owner, 2);
            }
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
                    for o in objects.iter_mut() {
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
                        if own || mode.starts_with("worn-other") {
                            // on the body, inside the wearer's box
                            o.position = OWN_PRIM;
                            o.scale = Vec3::splat(if mode == "worn-other-covered" { 0.25 } else { 0.6 });
                        }
                    }
                    if mode == "worn-other-link"
                        && let Some(mut child) = objects.first().cloned()
                    {
                        // a scriptless child above the scripted root
                        child.local_id = WORN_CHILD;
                        child.full_id = crate::demo::action_id(WORN_CHILD);
                        child.parent_id = 975;
                        child.state = 0;
                        child.position = Vec3::new(0.0, 0.0, 0.5);
                        child.scale = Vec3::splat(0.3);
                        child.update_flags = 0;
                        objects.push(child);
                    }
                }
            }
            if mode == "worn-other-covered" {
                // rigged clothing without a script over the scripted prim
                if let Some(g) = self.gfx.as_mut() {
                    self.scene.install_demo_animesh(&mut g.renderer);
                }
                for mut event in crate::demo::own_rigged_events(wearer) {
                    if let NetEvent::ObjectUpdates { objects, .. } = &mut event {
                        for o in objects.iter_mut() {
                            o.owner_id = owner;
                            o.update_flags = 0;
                        }
                    }
                    events.push(event);
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
