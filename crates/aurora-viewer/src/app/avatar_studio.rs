//! Studio lifecycle, render-only camera and native pointer ownership.

use super::App;
use crate::{
    avatar_studio::{Lighting, Pose, Studio, Transition, View},
    scene::Scene,
};
use aurora_render::{FrameParams, PointLight, Renderer};
use glam::{Quat, Vec3};
use std::time::Instant;
use winit::event::MouseButton;

impl App {
    fn studio_root(&self) -> Vec3 {
        self.world
            .objects
            .index_of_uuid(&self.world.agent_id)
            .and_then(|i| Scene::object_transform(&self.world, i, Instant::now(), 0))
            .map_or(self.world.agent.position, |(p, _, _)| p)
    }

    fn studio_subject(&self) -> (Vec3, f32) {
        let now = Instant::now();
        let id = &self.world.agent_id;
        let root = self.studio_root();
        let feet = ["mFootLeft", "mFootRight"].map(|name| self.scene.joint_world(&self.world, id, name, now));
        let bottom = feet.into_iter().flatten().map(|p| p.z).reduce(f32::min).unwrap_or(root.z - 0.9) - crate::avatar_studio::SOLE_HEIGHT;
        let top = self.scene.joint_world(&self.world, id, "mHead", now).map_or(root.z + 0.85, |p| p.z) + 0.2;
        (root.with_z(bottom), (top - bottom).clamp(0.5, 6.0))
    }

    pub(super) fn studio_frame(&mut self, renderer: &mut Renderer, dt: f32) {
        if self.demo && self.frame_count == 640 && std::env::var_os("AURORA_DEMO_AVATAR_STUDIO").is_some() {
            self.panels.avatar_studio = true;
            log::info!("avatar studio demo: toolbar command avatar_studio");
        }
        if self.demo && self.frame_count == 960 && std::env::var("AURORA_DEMO_AVATAR_STUDIO").is_ok_and(|mode| mode == "cycle") {
            self.panels.avatar_studio = false;
        }
        if !self.panels.avatar_studio && !self.avatar_studio.active && !self.studio_transition.busy() {
            self.world.avatar_studio = false;
            return;
        }
        let restricted = self.world.rlva.camera_owner().is_some()
            || [
                "setcam_unlock",
                "setcam_avdistmin",
                "setcam_avdistmax",
                "setcam_origindistmin",
                "setcam_origindistmax",
                "setcam_eyeoffset",
                "setcam_focusoffset",
                "setcam_mouselook",
                "setcam_fovmin",
                "setcam_fovmax",
            ]
            .iter()
            .any(|name| self.world.rlva.has(name));
        let switch = if !self.in_world() || self.world.teleporting || renderer.vr_engaged() || restricted {
            self.panels.avatar_studio = false;
            self.studio_transition.cancel();
            Some(false)
        } else {
            self.studio_transition.step(self.panels.avatar_studio, dt)
        };
        if let Some(active) = switch.filter(|active| *active != self.avatar_studio.active) {
            if active {
                self.autopilot = None;
                self.input = crate::agent::MoveInput::default();
                self.down.clear();
                let (floor, height) = self.studio_subject();
                self.avatar_studio.enter(self.world.agent.yaw, height);
                self.avatar_studio.floor_offset = floor.z - self.studio_root().z;
                if self.demo && std::env::var("AURORA_DEMO_AVATAR_STUDIO").is_ok_and(|mode| mode == "grounding") {
                    self.avatar_studio.floor_offset -= 1.25;
                }
                if self.demo && std::env::var("AURORA_DEMO_AVATAR_STUDIO").is_ok_and(|mode| mode == "pedestal") {
                    self.avatar_studio.inspect_feet();
                }
                if self.demo
                    && std::env::var("AURORA_DEMO_AVATAR_STUDIO")
                        .is_ok_and(|mode| matches!(mode.as_str(), "hover-height" | "animation-editor-hover"))
                {
                    self.avatar_studio.hover_height = 0.35;
                }
            } else {
                let close_animation_editor = self.avatar_studio.animation_editor;
                self.avatar_studio = Studio::default();
                if close_animation_editor {
                    self.ik_editor.close_animation_editor_from_studio();
                }
                self.scene.studio_stage.clear(renderer);
            }
            renderer.cut_motion_history();
            log::info!("avatar studio: {}", if active { "entered" } else { "left" });
        }
        let active = self.avatar_studio.active;
        if self.demo
            && active
            && let Ok(mode) = std::env::var("AURORA_DEMO_AVATAR_STUDIO")
        {
            if matches!(mode.as_str(), "animation-editor" | "animation-editor-hover" | "animation-jump") && self.frame_count == 800 {
                self.avatar_studio.animation_editor = true;
                if mode == "animation-jump" {
                    self.ik_editor.demo_jump_animation_editor_from_studio(&self.world);
                } else {
                    self.ik_editor.demo_animation_editor_from_studio(&self.world);
                }
            }
            match mode.as_str() {
                "t-pose" => self.avatar_studio.pose = Pose::T,
                "a-pose" => self.avatar_studio.pose = Pose::A,
                "grounding" => self.avatar_studio.pose = Pose::A,
                "environment" => self.avatar_studio.lighting = Lighting::Environment,
                "pose-cycle" => {
                    self.avatar_studio.pose = match self.frame_count {
                        0..800 => Pose::T,
                        800..960 => Pose::A,
                        _ => Pose::Animated,
                    }
                }
                _ => {}
            }
        }
        self.world.avatar_studio = active;
        if active
            && self.avatar_studio.animation_editor
            && !self.ik_editor.animation_editor_active()
            && !self.ik_editor.animation_editor_return_pending()
        {
            self.avatar_studio.animation_editor = false;
        }
        self.scene.studio_pose = if active && !self.avatar_studio.animation_editor {
            self.avatar_studio.pose
        } else {
            Pose::Animated
        };
        self.scene.studio_floor_offset = self.avatar_studio.floor_offset;
        if !self.focused || self.studio_transition.busy() {
            self.avatar_studio.rotating = false;
            self.avatar_studio.panning = false;
            self.avatar_studio.panning_plane = false;
        }
        self.avatar_studio.update(dt);
        // Publish the eased value after updating it, so the rendered pose and
        // the Studio's current state do not diverge by one frame.
        self.scene.studio_hover_height = if active { self.avatar_studio.display_hover_height } else { 0.0 };
        self.studio_photo_frame(renderer);
    }

    pub(super) fn studio_photo_frame(&mut self, renderer: &mut Renderer) {
        let wanted = self.avatar_studio.active && self.viewfinder_on() && self.in_world() && !self.world.teleporting;
        if wanted && self.studio_photo_session.is_none() {
            if let Some(view) = self.studio_view(1.0) {
                self.studio_photo_session = Some(crate::avatar_studio::PhotoSession::enter(
                    &mut self.camera,
                    &self.world.agent,
                    &self.settings.camera,
                    view,
                ));
            }
        } else if !wanted && let Some(session) = self.studio_photo_session.take() {
            session.restore(&mut self.camera, &mut self.world.agent);
            if self.world.teleporting || !self.in_world() {
                self.camera.snap();
            }
        } else {
            return;
        }
        self.avatar_studio.rotating = false;
        self.avatar_studio.panning = false;
        self.avatar_studio.panning_plane = false;
        self.right_drag = false;
        self.mouse_mode = super::MouseMode::None;
        self.set_mouselook_grab(self.camera.mouselook());
        renderer.cut_motion_history();
    }

    pub(super) fn studio_agent_controls(&self) -> aurora_net::AgentControls {
        self.studio_photo_session.as_ref().map_or_else(
            || self.world.agent.controls(&self.input, &self.camera, self.settings.draw_distance),
            |session| session.controls(&self.world.agent, &self.input, self.settings.draw_distance),
        )
    }

    pub(super) fn studio_view(&self, aspect: f32) -> Option<View> {
        self.avatar_studio.active.then(|| {
            let root = self.studio_root();
            let mut view = self.avatar_studio.view(root + Vec3::Z * self.avatar_studio.floor_offset, aspect);
            if self.studio_photo_session.is_some() {
                view.eye = self.camera.position;
                view.target = self.camera.target;
                view.view = self.camera.view();
                view.proj = self.camera.proj(aspect);
            }
            view
        })
    }

    pub(super) fn studio_mouse(&mut self, button: MouseButton, pressed: bool, over_ui: bool) -> bool {
        if self.viewfinder_on() {
            return false;
        }
        if self.studio_transition.busy() && !over_ui && matches!(button, MouseButton::Left | MouseButton::Right | MouseButton::Middle) {
            return true;
        }
        if !self.avatar_studio.active || !matches!(button, MouseButton::Left | MouseButton::Right | MouseButton::Middle) {
            return false;
        }
        let held = match button {
            MouseButton::Right => &mut self.avatar_studio.panning,
            MouseButton::Middle => &mut self.avatar_studio.panning_plane,
            _ => &mut self.avatar_studio.rotating,
        };
        if !pressed && *held {
            *held = false;
            if button == MouseButton::Right && self.right_gesture.opens_menu() && !over_ui {
                self.open_context_menu();
            }
            return true;
        }
        // Edit from an attachment's context menu must keep selection and
        // manipulator presses/releases; the build camera already uses the Studio view.
        if button == MouseButton::Left && self.build.open {
            return false;
        }
        if pressed && !over_ui && self.scene.hud_pick(&self.world, self.cursor_pos, false).is_none() {
            *held = true;
            if button == MouseButton::Right {
                self.right_gesture = super::RightGesture::default();
            }
            return true;
        }
        false
    }

    pub(super) fn studio_scroll(&mut self, steps: f32) -> bool {
        if self.viewfinder_on() {
            return false;
        }
        if self.studio_transition.busy() {
            return true;
        }
        if !self.avatar_studio.active || self.scene.hud_pick(&self.world, self.cursor_pos, false).is_some() {
            return false;
        }
        self.avatar_studio.zoom(steps);
        true
    }

    fn studio_effect_age(&self) -> Option<f32> {
        if self.settings.reduce_motion {
            return None;
        }
        if self.demo {
            match std::env::var("AURORA_DEMO_AVATAR_STUDIO").as_deref() {
                Ok("white") => return Some(0.22),
                Ok("reveal") => return Some(0.65),
                _ => {}
            }
        }
        self.studio_transition.age()
    }

    /// Last UI shape: cover the whole scene, HUDs and windows together.
    /// Chat/voice and the ordinary animation update keep running underneath.
    pub(super) fn studio_fade(&self, ctx: &egui::Context, shapes: &mut Vec<egui::epaint::ClippedShape>) {
        let alpha = self.studio_transition.black();
        if alpha > 0.0 {
            let rect = ctx.content_rect();
            // theme-guard: content — a fade through black, not a panel surface.
            let color = egui::Color32::from_black_alpha((alpha * 255.0).round() as u8);
            shapes.push(egui::epaint::ClippedShape::new(rect, egui::Shape::rect_filled(rect, 0.0, color)));
        }
    }

    pub(super) fn studio_render(&mut self, renderer: &mut Renderer, fp: &mut FrameParams, view: View) {
        let studio_lighting = self.avatar_studio.lighting == Lighting::Studio;
        if studio_lighting {
            let rotation = Quat::from_rotation_z(self.avatar_studio.yaw);
            let lights = [
                (Vec3::new(2.8, -2.5, 3.5), Vec3::splat(2.8)),
                (Vec3::new(1.5, 3.0, 2.8), Vec3::new(1.2, 1.3, 1.5)),
                (Vec3::new(-2.0, 0.5, 3.5), Vec3::splat(1.8)),
            ];
            fp.lights = lights
                .into_iter()
                .map(|(position, color)| PointLight {
                    position: view.floor + rotation * position,
                    color,
                    radius: 12.0,
                    falloff: 0.5,
                    projector: None,
                })
                .collect();
        }
        fp.spot_shadows.fill(None);
        fp.probes = Default::default();
        fp.mirror = None;
        fp.shadows = false;
        fp.water_height = view.floor.z + crate::scene::avatar_studio::WATER_OFFSET;
        let effect_age = self.studio_effect_age();
        if studio_lighting {
            fp.water.fog_color = Vec3::new(0.025, 0.04, 0.075);
            fp.water.fog_density = 2.0;
            // Same palette and curtain motion as the login illustration, rendered
            // in 3D so the existing water path can reflect it.
            fp.sky.studio_aurora = [self.palette.navy, self.palette.teal, self.palette.violet, self.palette.indigo]
                .map(|c| [f32::from(c.r()) / 255.0, f32::from(c.g()) / 255.0, f32::from(c.b()) / 255.0, 0.0]);
            fp.sky.studio_aurora[0][3] = 1.0;
            fp.sky.studio_aurora[1][3] = self.avatar_studio.yaw;
            fp.sky.studio_aurora[2][3] = if self.settings.reduce_motion {
                0.0
            } else {
                self.start.elapsed().as_secs_f32()
            };
            fp.sky.star_brightness = 250.0;
            fp.fog_density = 0.0;
            if !self.viewfinder_on() {
                fp.exposure = 1.0;
                fp.exposure_lock = true;
                fp.bloom = 0.0;
            }
        }
        fp.sky.studio_aurora[3][3] = effect_age.map_or(0.0, Transition::white);
        if !self.viewfinder_on() {
            fp.dof = None;
            fp.cutout = false;
        }
        let lists = &mut self.scene.lists;
        crate::scene::avatar_studio::hide_region_terrain(lists);
        lists.water.clear();
        lists.water_events.clear();
        lists.water_contacts.clear();
        lists.water_ground = Default::default();
        lists.particles.clear();
        lists.land_overlay.clear();
        lists.creation_preview.clear();
        lists.physics_shapes.clear();
        lists.impostor_sprites.clear();
        if let Some(age) = effect_age {
            let colors = [self.palette.teal, self.palette.violet_light, self.palette.indigo_light].map(|c| [c.r(), c.g(), c.b()]);
            crate::scene::avatar_studio::reveal_particles(
                &mut lists.particles,
                view.floor,
                self.avatar_studio.height(),
                age,
                colors,
                self.settings.max_particles.min(96) as usize,
            );
        }
        if !fp.cutout {
            self.scene.studio_stage.append(renderer, lists, view.floor, self.avatar_studio.yaw);
        }
    }

    pub(super) fn studio_overlay(&mut self, ctx: &egui::Context) {
        if !self.avatar_studio.active || self.viewfinder_on() {
            return;
        }
        let mut reset = false;
        let mut reset_animation = false;
        egui::Area::new(egui::Id::unique("avatar_studio_controls"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 52.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                crate::ui::widgets::island(ui, &self.palette, |ui| {
                    let controls = [
                        (
                            "rotate-360",
                            aurora_i18n::tr!("common-rotate"),
                            aurora_i18n::tr!("avatar-studio-rotate-tip"),
                        ),
                        (
                            "arrows-vertical",
                            aurora_i18n::tr!("avatar-studio-action-focus"),
                            aurora_i18n::tr!("avatar-studio-focus-tip"),
                        ),
                        (
                            "arrows-move",
                            aurora_i18n::tr!("avatar-studio-action-pan"),
                            aurora_i18n::tr!("avatar-studio-pan-tip"),
                        ),
                        (
                            "zoom-in-filled",
                            aurora_i18n::tr!("avatar-studio-action-zoom"),
                            aurora_i18n::tr!("avatar-studio-zoom-tip"),
                        ),
                    ];
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let width = |text: &str| {
                        ui.painter()
                            .layout_no_wrap(text.to_owned(), font.clone(), self.palette.ink)
                            .size()
                            .x
                    };
                    let gap = ui.spacing().item_spacing.x;
                    let legend_width =
                        controls.iter().map(|(_, label, _)| 45.0 + gap + width(label)).sum::<f32>() + 3.0 * (6.0 + 2.0 * gap);
                    let title = aurora_i18n::tr!("avatar-studio-title");
                    let reset_label = aurora_i18n::tr!("common-reset");
                    let exit_label = aurora_i18n::tr!("avatar-studio-exit");
                    ui.horizontal(|ui| {
                        ui.set_height(crate::ui::bars::islands::SLOT);
                        let title_font = egui::TextStyle::Body.resolve(ui.style());
                        let title_galley = ui.painter().layout_no_wrap(title.to_owned(), title_font, self.palette.ink);
                        let (title_rect, _) = ui.allocate_exact_size(
                            egui::vec2(title_galley.size().x, crate::ui::bars::islands::SLOT),
                            egui::Sense::hover(),
                        );
                        crate::ui::widgets::paint_ink_centered(ui.painter(), title_galley, title_rect.center(), self.palette.ink);
                        ui.add_space(8.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(legend_width, crate::ui::bars::islands::SLOT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                for (i, (icon, label, tip)) in controls.into_iter().enumerate() {
                                    if i > 0 {
                                        // The human requested boundaries between these
                                        // passive hints; use the shared divider treatment.
                                        crate::ui::widgets::divider(ui, &self.palette);
                                    }
                                    let (rect, response) = ui.allocate_exact_size(egui::vec2(45.0, 24.0), egui::Sense::hover());
                                    studio_mouse_hint(ui.painter(), rect, i, icon, &self.palette);
                                    response.union(ui.label(label)).on_hover_text(tip);
                                }
                            },
                        );
                        ui.add_space(8.0);
                        let demo_menu = self.demo.then(|| std::env::var("AURORA_DEMO_AVATAR_STUDIO").unwrap_or_default());
                        studio_pose_menu(
                            ui,
                            &self.palette,
                            &mut self.avatar_studio.pose,
                            demo_menu.as_deref() == Some("pose-menu") && self.frame_count == 800,
                        );
                        studio_hover_menu(
                            ui,
                            &self.palette,
                            &mut self.avatar_studio.hover_height,
                            demo_menu.as_deref() == Some("hover-menu") && self.frame_count == 800,
                        );
                        studio_lighting_menu(
                            ui,
                            &self.palette,
                            &mut self.avatar_studio.lighting,
                            demo_menu.as_deref() == Some("lighting-menu") && self.frame_count == 800,
                        );
                        let animation_label = if self.avatar_studio.animation_editor {
                            aurora_i18n::tr!("avatar-studio-animation-editor-close-short")
                        } else {
                            aurora_i18n::tr!("avatar-studio-animation-editor-short")
                        };
                        if crate::ui::widgets::action_button(
                            ui,
                            &self.palette,
                            "timeline",
                            animation_label,
                            crate::ui::widgets::ActionKind::Flat,
                            egui::vec2(0.0, crate::ui::bars::islands::SLOT),
                        )
                        .on_hover_text(aurora_i18n::tr!("avatar-studio-animation-editor"))
                        .clicked()
                        {
                            self.avatar_studio.animation_editor = !self.avatar_studio.animation_editor;
                            if self.avatar_studio.animation_editor {
                                self.ik_editor.open_animation_editor_from_studio(&self.world);
                            } else {
                                self.ik_editor.close_animation_editor_from_studio();
                            }
                        }
                        if self.avatar_studio.animation_editor
                            && self.ik_editor.animation_editor_active()
                            && !self.ik_editor.studio_animation_timeline_open()
                            && crate::ui::widgets::action_button(
                                ui,
                                &self.palette,
                                "timeline",
                                aurora_i18n::tr!("avatar-studio-animation-timeline-short"),
                                crate::ui::widgets::ActionKind::Flat,
                                egui::vec2(0.0, crate::ui::bars::islands::SLOT),
                            )
                            .on_hover_text(aurora_i18n::tr!("avatar-studio-animation-timeline-show"))
                            .clicked()
                        {
                            self.ik_editor.show_studio_animation_timeline();
                        }
                        if ui.button(reset_label).clicked() {
                            if self.avatar_studio.animation_editor {
                                self.avatar_studio.reset_animation_confirm = true;
                            } else {
                                reset = true;
                            }
                        }
                        if ui.button(exit_label).clicked() {
                            self.panels.avatar_studio = false;
                        }
                    });
                });
            });
        if self.avatar_studio.animation_editor {
            self.ik_editor.studio_animation_timeline(ctx, &self.palette, &self.world);
        }
        if self.avatar_studio.reset_animation_confirm {
            let mut answer = None;
            let modal = crate::ui::widgets::Modal::new(egui::Id::unique("avatar_studio_reset_animation")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("avatar-studio-reset-animation-confirm"))
                        .size(17.0)
                        .color(self.palette.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("avatar-studio-reset-animation-description"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let size = egui::Vec2::new(0.0, 26.0);
                    if crate::ui::widgets::action_button(
                        ui,
                        &self.palette,
                        "refresh",
                        aurora_i18n::tr!("avatar-studio-reset-animation-action"),
                        crate::ui::widgets::ActionKind::Danger,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if crate::ui::widgets::action_button(
                        ui,
                        &self.palette,
                        "x",
                        aurora_i18n::tr!("common-cancel"),
                        crate::ui::widgets::ActionKind::Flat,
                        size,
                    )
                    .clicked()
                    {
                        answer = Some(false);
                    }
                });
            });
            if modal.should_close() && answer.is_none() {
                answer = Some(false);
            }
            match answer {
                Some(true) => {
                    self.avatar_studio.reset_animation_confirm = false;
                    reset = true;
                    reset_animation = true;
                }
                Some(false) => self.avatar_studio.reset_animation_confirm = false,
                None => {}
            }
        }
        if reset {
            if reset_animation {
                self.ik_editor.reset_animation_from_avatar_studio(&self.world);
            }
            let offset = self.avatar_studio.floor_offset;
            let pose = self.avatar_studio.pose;
            let lighting = self.avatar_studio.lighting;
            let animation_editor = self.avatar_studio.animation_editor;
            let hover_height = self.avatar_studio.hover_height;
            self.avatar_studio.enter(self.avatar_studio.facing, self.studio_subject().1);
            self.avatar_studio.floor_offset = offset;
            self.avatar_studio.pose = pose;
            self.avatar_studio.lighting = lighting;
            self.avatar_studio.animation_editor = animation_editor;
            self.avatar_studio.hover_height = hover_height;
        }
    }
}

/// Button highlights reuse clipped Tabler masks, keeping the mouse silhouette
/// and movement glyphs consistent with the viewer's camera controls.
fn studio_mouse_hint(painter: &egui::Painter, rect: egui::Rect, button: usize, action: &str, p: &crate::theme::Palette) {
    let scale = rect.height() / 32.0;
    let mouse = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(32.0 * scale));
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let paint = |painter: &egui::Painter, name: &str, rect, tint| {
        if let Some(texture) = crate::ui::icons::global(name) {
            painter.image(texture.id(), rect, uv, tint);
        }
    };
    let point = |x: f32, y: f32| mouse.min + mouse.size() * egui::vec2(x / 24.0, y / 24.0);
    if button < 2 {
        let (left, right) = if button == 0 { (5.0, 12.0) } else { (12.0, 19.0) };
        let clip = egui::Rect::from_min_max(point(left, 2.0), point(right, 10.0));
        paint(
            &painter.with_clip_rect(clip.intersect(painter.clip_rect())),
            "mouse-filled",
            mouse,
            p.violet_light,
        );
        paint(painter, "mouse-2", mouse, p.muted);
    } else {
        paint(painter, "mouse", mouse, p.muted);
        let clip = egui::Rect::from_min_max(point(10.0, 6.0), point(14.0, 12.0));
        paint(
            &painter.with_clip_rect(clip.intersect(painter.clip_rect())),
            "mouse",
            mouse,
            p.violet_light,
        );
        if button == 3 {
            let scroll = egui::Rect::from_center_size(mouse.right_center() - egui::vec2(2.0, 0.0) * scale, egui::vec2(10.0, 22.0) * scale);
            paint(painter, "arrows-vertical", scroll, p.violet_light);
        }
    }
    let movement = egui::Rect::from_center_size(rect.right_center() - egui::vec2(10.0, 0.0) * scale, egui::Vec2::splat(20.0 * scale));
    paint(painter, action, movement, p.ink);
}

fn studio_pose_menu(ui: &mut egui::Ui, p: &crate::theme::Palette, pose: &mut Pose, open: bool) {
    use crate::ui::{menu, widgets};
    let response = widgets::action_button(
        ui,
        p,
        "chevron-down",
        aurora_i18n::tr!("avatar-studio-pose"),
        widgets::ActionKind::Flat,
        egui::vec2(0.0, crate::ui::bars::islands::SLOT),
    );
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        for (value, icon, label) in [
            (Pose::Animated, "walk", aurora_i18n::tr!("avatar-studio-pose-animated")),
            (Pose::T, "man-filled", aurora_i18n::tr!("avatar-studio-pose-t")),
            (Pose::A, "man-filled", aurora_i18n::tr!("avatar-studio-pose-a")),
        ] {
            if menu::check(ui, p, icon, label, *pose == value, true) {
                *pose = value;
            }
        }
    });
}

fn studio_hover_menu(ui: &mut egui::Ui, p: &crate::theme::Palette, hover_height: &mut f32, open: bool) {
    use crate::ui::{menu, widgets};
    let height = format!("{:.2}", *hover_height);
    let response = widgets::action_button(
        ui,
        p,
        "arrows-vertical",
        &aurora_i18n::tr!("avatar-studio-hover-height", height = height.as_str()),
        widgets::ActionKind::Flat,
        // Keep the toolbar and popup anchor fixed as the numeric value changes.
        egui::vec2(154.0, crate::ui::bars::islands::SLOT),
    );
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    let response = response.on_hover_text(aurora_i18n::tr!("avatar-studio-hover-height-hint"));
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_width(280.0);
        ui.set_max_width(280.0);
        ui.add_space(4.0);
        ui.add(egui::Label::new(aurora_i18n::tr!("avatar-studio-hover-height-hint")).wrap());
        ui.add_space(8.0);
        ui.add_sized(
            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
            egui::Slider::new(hover_height, 0.0..=1.0)
                .step_by(0.01)
                .fixed_decimals(2)
                .suffix(aurora_i18n::tr!("avatar-studio-hover-height-suffix")),
        );
        ui.add_space(6.0);
        studio_hover_reset_row(ui, hover_height);
        ui.add_space(4.0);
    });
}

fn studio_hover_reset_row(ui: &mut egui::Ui, hover_height: &mut f32) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if ui.button(aurora_i18n::tr!("common-reset")).clicked() {
                *hover_height = 0.0;
            }
        },
    )
    .response
}

fn studio_lighting_menu(ui: &mut egui::Ui, p: &crate::theme::Palette, lighting: &mut Lighting, open: bool) {
    use crate::ui::{menu, widgets};
    let response = widgets::action_button(
        ui,
        p,
        "chevron-down",
        aurora_i18n::tr!("avatar-studio-lighting"),
        widgets::ActionKind::Flat,
        egui::vec2(0.0, crate::ui::bars::islands::SLOT),
    );
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        for (value, icon, label) in [
            (Lighting::Studio, "bulb-filled", aurora_i18n::tr!("avatar-studio-lighting-studio")),
            (
                Lighting::Environment,
                "sun-filled",
                aurora_i18n::tr!("avatar-studio-lighting-environment"),
            ),
        ] {
            if menu::check(ui, p, icon, label, *lighting == value, true) {
                *lighting = value;
            }
        }
    });
}

#[cfg(test)]
mod hover_menu_tests {
    use super::*;

    #[test]
    fn hover_reset_row_stays_one_control_high_in_a_tall_popup() {
        let ctx = egui::Context::default();
        let mut row_height = None;
        let mut hover_height = 0.35;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                row_height = Some(studio_hover_reset_row(ui, &mut hover_height).rect.height());
            },
        );
        output.textures_delta.clear();
        assert!(row_height.is_some_and(|height| height <= 40.0));
        assert_eq!(hover_height, 0.35);
    }
}
