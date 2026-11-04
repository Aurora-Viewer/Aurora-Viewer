//! Studio lifecycle, render-only camera and native pointer ownership.

use super::App;
use crate::{
    avatar_studio::{Studio, Transition, View},
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
        let bottom = feet.into_iter().flatten().map(|p| p.z).reduce(f32::min).unwrap_or(root.z - 0.9) - 0.06;
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
            } else {
                self.avatar_studio = Studio::default();
                self.scene.studio_stage.clear(renderer);
            }
            renderer.cut_motion_history();
            log::info!("avatar studio: {}", if active { "entered" } else { "left" });
        }
        let active = self.avatar_studio.active;
        self.world.avatar_studio = active;
        if !self.focused || self.studio_transition.busy() {
            self.avatar_studio.rotating = false;
            self.avatar_studio.panning = false;
        }
        self.avatar_studio.update(dt);
    }

    pub(super) fn studio_view(&self, aspect: f32) -> Option<View> {
        self.avatar_studio.active.then(|| {
            let root = self.studio_root();
            self.avatar_studio.view(root + Vec3::Z * self.avatar_studio.floor_offset, aspect)
        })
    }

    pub(super) fn studio_mouse(&mut self, button: MouseButton, pressed: bool, over_ui: bool) -> bool {
        if self.studio_transition.busy() && !over_ui && matches!(button, MouseButton::Left | MouseButton::Right) {
            return true;
        }
        if !self.avatar_studio.active || !matches!(button, MouseButton::Left | MouseButton::Right) {
            return false;
        }
        let held = match button {
            MouseButton::Right => &mut self.avatar_studio.panning,
            _ => &mut self.avatar_studio.rotating,
        };
        if !pressed && *held {
            *held = false;
            return true;
        }
        if pressed && !over_ui && self.scene.hud_pick(&self.world, self.cursor_pos, false).is_none() {
            *held = true;
            return true;
        }
        false
    }

    pub(super) fn studio_scroll(&mut self, steps: f32) -> bool {
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
        fp.spot_shadows = [None; 2];
        fp.probes = Default::default();
        fp.mirror = None;
        fp.shadows = false;
        fp.water_height = view.floor.z + crate::scene::avatar_studio::WATER_OFFSET;
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
        let effect_age = self.studio_effect_age();
        fp.sky.studio_aurora[3][3] = effect_age.map_or(0.0, Transition::white);
        fp.sky.star_brightness = 250.0;
        fp.fog_density = 0.0;
        fp.exposure = 1.0;
        fp.exposure_lock = true;
        fp.bloom = 0.0;
        fp.dof = None;
        fp.cutout = false;
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
        self.scene.studio_stage.append(renderer, lists, view.floor, self.avatar_studio.yaw);
    }

    pub(super) fn studio_overlay(&mut self, ctx: &egui::Context) {
        if !self.avatar_studio.active {
            return;
        }
        let mut reset = false;
        egui::Area::new(egui::Id::unique("avatar_studio_controls"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                crate::ui::widgets::overlay(ui, &self.palette, egui::Margin::same(10), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("avatar-studio-title"));
                        reset = ui.button(aurora_i18n::tr!("common-reset")).clicked();
                        if ui.button(aurora_i18n::tr!("avatar-studio-exit")).clicked() {
                            self.panels.avatar_studio = false;
                        }
                    });
                    ui.small(aurora_i18n::tr!("avatar-studio-help"));
                });
            });
        if reset {
            let offset = self.avatar_studio.floor_offset;
            self.avatar_studio.enter(self.avatar_studio.facing, self.studio_subject().1);
            self.avatar_studio.floor_offset = offset;
        }
    }
}
