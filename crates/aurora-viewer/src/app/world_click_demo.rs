//! Offline regressions of real mouse presses (#416, #419, #420), including
//! the delayed GPU answer and the corresponding touch release.
use super::*;

/// `worn-own`: where the prim sits on our chest's attachment point (on the
/// back, facing the default camera).
const OWN_PRIM: Vec3 = Vec3::ZERO;

/// `worn-own-mesh`: a point of the synthetic chest mesh on our back, inside
/// our box but off the classic body, where only the mesh is drawn.
const OWN_MESH: Vec3 = Vec3::new(-0.12, 0.0, 0.0);

/// `worn-other-link`: the child prim, without a script, of a worn linkset
/// whose root holds the touch script.
const WORN_CHILD: u32 = 988;

fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_WORLD_CLICKS").ok()).as_deref()
}

impl App {
    pub(super) fn world_click_demo_steps(&mut self) {
        let Some(spec) = mode().filter(|_| self.demo) else { return };
        let mode = spec.strip_suffix("-keep-camera").unwrap_or(spec);
        let drag = mode.ends_with("-drag");
        if mode.starts_with("avatar") || spec.ends_with("-keep-camera") {
            // Camera comparisons need a stationary subject even when another
            // demo/device left movement inputs active before this fixture.
            self.down.clear();
            self.controls_held.clear();
            self.settings.joystick.enabled = false;
        }
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
            if mode.starts_with("avatar") || spec.ends_with("-keep-camera") {
                self.stop_autopilot(true);
            }
            if spec.ends_with("-keep-camera") || mode == "avatar-return" || drag {
                self.settings.camera.click_avatar_keeps_camera = spec.ends_with("-keep-camera");
            }
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
            let cursor = if mode.starts_with("avatar-tag") {
                self.name_tags.iter().find(|t| t.id == self.world.agent_id).map(|t| {
                    let p = t.rect.center() * self.egui_ctx.pixels_per_point();
                    (p.x, p.y)
                })
            } else if mode.starts_with("avatar") {
                self.world
                    .objects
                    .avatars()
                    .find(|(_, o)| (o.full_id == self.world.agent_id) != mode.contains("other"))
                    .and_then(|(idx, _)| Scene::object_transform(&self.world, idx, Instant::now(), 0))
                    .and_then(|(pos, _, _)| self.build.cam.project_px(pos + Vec3::Z * 0.4))
            } else if mode.starts_with("photo-hud") {
                self.scene.lists.hud_view.map(|view| view.pixel(Vec3::new(0.0, 0.18, 0.028)).into())
            } else if mode == "ground" {
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
                    _ if mode.starts_with("photo-touch") => 975,
                    "worn-other-link" => WORN_CHILD,
                    _ if mode.starts_with("worn-") => 975,
                    _ => 971,
                };
                self.world.objects.index_of_uuid(&crate::demo::action_id(local)).and_then(|idx| {
                    let pos = if mode == "worn-mesh"
                        || mode.starts_with("worn-own")
                        || (mode.starts_with("worn-other") && mode != "worn-other-link")
                    {
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
                        let offset = if mode == "worn-mesh" {
                            Vec3::new(0.30, 0.0, 0.0)
                        } else if mode.starts_with("worn-own-mesh") {
                            OWN_MESH
                        } else {
                            OWN_PRIM
                        };
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
            self.alt = mode.starts_with("camera-") || mode.contains("-alt");
            self.ctrl = mode.contains("-ctrl");
            self.shift = mode.contains("-shift");
            log::info!("demo world click: mode={mode}, cursor={:?}", self.cursor_pos);
            self.world_click_camera_note("before", spec);
            let over_ui = self.ui_takes_pointer();
            log::info!("demo world click UI: over_ui={over_ui}");
            self.mouse_input(MouseButton::Left, true, over_ui);
            if mode.starts_with("photo-") {
                log::info!(
                    "demo photo press: ctrl={}, waiting={}, point={:?}",
                    self.ctrl,
                    self.clicks.waiting(),
                    self.snapshots.vf.point,
                );
            }
        }
        if drag && (710..=760).contains(&self.frame_count) {
            self.camera_mouse_delta(6.0, 0.0);
        }
        if self.frame_count == if drag { 761 } else { 701 } {
            self.mouse_input(MouseButton::Left, false, false);
            // Release modifiers before the deferred pick finishes: its
            // captured press must still choose the original gesture.
            self.ctrl = false;
            self.shift = false;
        }
        if self.frame_count == if drag { 820 } else { 740 } {
            self.alt = false;
            self.ctrl = false;
            self.world_click_camera_note("after", spec);
            let focus = self.camera.focus_point();
            let target = focus
                .and_then(|p| self.scene.pick_at(&self.world, p, Instant::now()))
                .and_then(|idx| self.world.objects.get(idx))
                .map(|o| o.key.local_id);
            log::info!(
                "demo world click result: mode={mode}, focus={focus:?}, target={target:?}, held={}, steer={}",
                self.interactions.held.is_some(),
                self.mouse_mode == MouseMode::Steer
            );
            if mode.starts_with("photo-") {
                let hud_touched = self
                    .world
                    .objects
                    .index_of_uuid(&crate::demo::hud::object(31, false).full_id)
                    .and_then(|idx| self.world.objects.get(idx))
                    .is_some_and(|o| o.text.contains(aurora_i18n::tr!("demo-hud-touched")));
                log::info!(
                    "demo photo interaction: point={:?}, pick={}, dialog={}, ctrl={}, left_down={}, hud_touched={hud_touched}",
                    self.snapshots.vf.point,
                    self.snapshots.vf.pick.is_some(),
                    self.interactions.dialog.is_some(),
                    self.ctrl,
                    self.left_down,
                );
            }
        }
    }

    fn world_click_camera_note(&self, phase: &str, spec: &str) {
        log::info!(
            "demo world click camera: {phase} {spec}, position={:?}, target={:?}, fov={}, object={:?}, agent_yaw={}, body_yaw={:?}",
            self.camera.position,
            self.camera.target,
            self.camera.fov_y,
            self.camera.focus_object_id(),
            self.world.agent.yaw,
            self.world.bodies.get(&self.world.agent_id).map(|b| {
                let forward = b.rotation * Vec3::X;
                forward.y.atan2(forward.x)
            })
        );
    }

    fn world_click_fixture(&mut self, mode: &str) {
        let mode = mode.strip_suffix("-ctrl").unwrap_or(mode);
        if mode.starts_with("photo-") {
            self.panels.snapshot = true;
            self.settings.photo_window = crate::settings::PhotoWindowStyle::Aurora;
            self.settings.snapshot.huds = true;
            self.settings.show_huds = true;
            self.settings.snapshot.viewfinder.hints = true;
            self.settings.snapshot.viewfinder.display = crate::snapshot::viewfinder::Display::Full;
            self.settings.snapshot.dof.focus = crate::dof::PhotoFocus::Click;
            self.snapshots.ui.freeze_world = false;
            if mode.starts_with("photo-hud") {
                for event in crate::demo::hud::events() {
                    self.world.apply(event);
                }
            } else {
                for mut event in crate::demo::action_events() {
                    if let NetEvent::ObjectUpdates { objects, .. } = &mut event {
                        objects.retain(|o| o.local_id == 975);
                        if let Some(o) = objects.first() {
                            self.camera.set_view(
                                o.position + Vec3::new(-3.0, 0.0, 1.0),
                                o.position,
                                &mut self.world.agent,
                                &self.settings.camera,
                            );
                        }
                    }
                    self.world.apply(event);
                }
            }
            return;
        }
        if mode.starts_with("avatar") {
            let target = self
                .world
                .objects
                .avatars()
                .find(|(_, o)| (o.full_id == self.world.agent_id) != mode.contains("other"));
            if let Some((idx, o)) = target {
                let id = o.full_id;
                if let Some((pos, _, _)) = Scene::object_transform(&self.world, idx, Instant::now(), 0) {
                    self.settings.set_render_exception(id, 2);
                    let focus = pos + Vec3::new(-0.3, 0.2, 0.5);
                    self.camera.set_view(
                        focus + Vec3::new(1.7, -2.3, 0.9),
                        focus,
                        &mut self.world.agent,
                        &self.settings.camera,
                    );
                }
            }
            return;
        }
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
            let own = mode.starts_with("worn-own");
            let covered = matches!(
                mode,
                "worn-other-covered"
                    | "worn-other-under"
                    | "worn-other-hidden-under"
                    | "worn-other-hidden-mesh-under"
                    | "worn-other-hidden-inside"
            );
            // `worn-other-hidden-inside`: a small clicker sunk into the body,
            // its faces well under the rigged mesh's surface (#1241)
            let inside = mode == "worn-other-hidden-inside";
            // `worn-other-hidden*`: every face of the scripted attachment at
            // alpha 0, the invisible clickers of a headpat or a slapper
            let hidden = mode.starts_with("worn-other-hidden");
            let unrigged_mesh = matches!(mode, "worn-other-mesh" | "worn-other-hidden-mesh" | "worn-other-hidden-mesh-under");
            if unrigged_mesh && let Some(g) = self.gfx.as_mut() {
                // a mesh without a skin, placed by its attachment point
                self.scene.install_demo_unrigged_cube(&mut g.renderer);
            }
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
            // `worn-own-mesh`: the scripted rigged mesh on our own chest
            let own_mesh = mode.starts_with("worn-own-mesh");
            let mut events = if mode == "worn-mesh" || own_mesh {
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
                        if mode == "worn-own-passive" {
                            o.update_flags = 0;
                        }
                        if mode == "worn-prim" {
                            o.scale = Vec3::splat(0.5);
                        }
                        if (own && !own_mesh) || mode.starts_with("worn-other") {
                            // on the body, inside the wearer's box
                            o.position = OWN_PRIM;
                            o.scale = Vec3::splat(if inside {
                                0.1
                            } else if covered {
                                0.25
                            } else {
                                0.6
                            });
                        }
                        if own_mesh && let Some(te) = o.texture_entry.as_mut() {
                            // opaque: the press depth is then the mesh, inside
                            // our box whatever the pose, not the floor behind
                            for f in &mut std::sync::Arc::make_mut(te).faces {
                                f.texture = aurora_prim::te::BLANK_TEXTURE;
                            }
                        }
                        if hidden && let Some(te) = o.texture_entry.as_mut() {
                            for f in &mut std::sync::Arc::make_mut(te).faces {
                                f.color[3] = 0.0;
                            }
                        }
                        if unrigged_mesh {
                            let mesh = aurora_prim::params::SculptParams {
                                texture: crate::demo::WORN_UNRIGGED_MESH,
                                sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
                            };
                            o.volume.sculpt = Some(mesh);
                            o.extra.sculpt = Some(mesh);
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
            if covered {
                // rigged clothing without a script over the scripted prim:
                // blended (`worn-other-covered`) or opaque (`worn-other-under`)
                if let Some(g) = self.gfx.as_mut() {
                    self.scene.install_demo_animesh(&mut g.renderer);
                }
                for mut event in crate::demo::own_rigged_events(wearer) {
                    if let NetEvent::ObjectUpdates { objects, .. } = &mut event {
                        for o in objects.iter_mut() {
                            o.owner_id = owner;
                            o.update_flags = 0;
                            if mode != "worn-other-covered"
                                && let Some(te) = o.texture_entry.as_mut()
                            {
                                for f in &mut std::sync::Arc::make_mut(te).faces {
                                    f.texture = aurora_prim::te::BLANK_TEXTURE;
                                }
                            }
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
            self.settings.plate_shift_click = if mode.contains("-none") {
                crate::settings::PlateClick::Nothing
            } else {
                crate::settings::PlateClick::InstantMessage
            };
            self.settings.plate_ctrl_click = if mode.contains("-none") {
                crate::settings::PlateClick::Nothing
            } else {
                crate::settings::PlateClick::Menu
            };
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
        let mut overlay = if mode.starts_with("plate-wall") {
            crate::demo::action_overlay(976, pos, rotation)
        } else {
            let alpha = match mode {
                m if m.starts_with("camera-faint") => "blend-faint",
                m if m.starts_with("camera-glass") => "blend-half",
                m if m.starts_with("camera-mask") => "mask-zero",
                _ => "blend-zero",
            };
            crate::demo::alpha_click_overlay(alpha, pos, rotation)
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
        if mode == "camera-occluded" {
            let mut wall = crate::demo::action_overlay(977, pos + direction, rotation);
            if let NetEvent::ObjectUpdates { objects, .. } = &mut wall {
                for o in objects {
                    o.scale = Vec3::new(0.12, 5.0, 5.0);
                }
            }
            self.world.apply(wall);
        }
    }
}
