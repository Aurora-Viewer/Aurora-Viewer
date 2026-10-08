//! Debug overlays drawn over the 3D view (Préférences › Debug): bounding
//! boxes, culling decisions, lights, glowing objects, reflection probes and
//! avatar skeletons. Lines are projected on the CPU and painted by egui, on
//! top of the scene (no depth test).

use super::hud::Projector;
use crate::scene::Scene;
use crate::settings::DebugView;
use crate::world::World;
use egui::{Color32, Stroke};
use glam::{Mat4, Quat, Vec3};
use std::time::Instant;

/// Objects considered around the camera (nearest first).
const MAX_OBJECTS: usize = 600;
const RANGE: f32 = 96.0;
const SKELETON_RANGE: f32 = 32.0;

const GREEN: Color32 = Color32::from_rgb(80, 220, 120);
const RED: Color32 = Color32::from_rgb(240, 70, 70);
const ORANGE: Color32 = Color32::from_rgb(245, 160, 50);
const GREY: Color32 = Color32::from_gray(140);

struct Lines<'a> {
    painter: egui::Painter,
    proj: &'a Projector,
}

impl Lines<'_> {
    fn seg(&self, a: Vec3, b: Vec3, color: Color32) {
        if let (Some((pa, _)), Some((pb, _))) = (self.proj.project(a), self.proj.project(b)) {
            self.painter.line_segment([pa, pb], Stroke::new(1.0, color));
        }
    }

    /// Box from 8 corners in the (±x, ±y, ±z) order of `corner`.
    fn cube(&self, corner: impl Fn(f32, f32, f32) -> Vec3, color: Color32) {
        let s = [-1.0f32, 1.0];
        for &a in &s {
            for &b in &s {
                self.seg(corner(-1.0, a, b), corner(1.0, a, b), color);
                self.seg(corner(a, -1.0, b), corner(a, 1.0, b), color);
                self.seg(corner(a, b, -1.0), corner(a, b, 1.0), color);
            }
        }
    }

    fn obb(&self, pos: Vec3, rot: Quat, half: Vec3, color: Color32) {
        self.cube(|x, y, z| pos + rot * (half * Vec3::new(x, y, z)), color);
    }

    /// Sphere as its three great circles.
    fn sphere(&self, c: Vec3, r: f32, color: Color32) {
        const N: usize = 32;
        let pt = |axis: usize, i: usize| {
            let a = i as f32 / N as f32 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            c + r * match axis {
                0 => Vec3::new(co, s, 0.0),
                1 => Vec3::new(co, 0.0, s),
                _ => Vec3::new(0.0, co, s),
            }
        };
        for axis in 0..3 {
            for i in 0..N {
                self.seg(pt(axis, i), pt(axis, i + 1), color);
            }
        }
    }

    /// Bounding sphere as the circle it covers on screen.
    fn screen_circle(&self, c: Vec3, r: f32, eye: Vec3, color: Color32) {
        let Some((pc, w)) = self.proj.project(c) else {
            return;
        };
        // radius in points: project a point r away, perpendicular to the view
        let side = (c - eye).cross(Vec3::Z).try_normalize().unwrap_or(Vec3::X);
        let rp = self
            .proj
            .project(c + side * r)
            .map(|(p, _)| p.distance(pc))
            .unwrap_or(r / w.max(0.01) * 500.0);
        self.painter.circle_stroke(pc, rp.max(1.5), Stroke::new(1.0, color));
    }

    fn label(&self, at: Vec3, text: &str, color: Color32) {
        if let Some((p, _)) = self.proj.project(at) {
            self.painter.text(
                p + egui::vec2(4.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                text,
                egui::FontId::proportional(10.0),
                color,
            );
        }
    }

    fn cross(&self, at: Vec3, size: f32, color: Color32) {
        if let Some((p, _)) = self.proj.project(at) {
            let s = Stroke::new(1.5, color);
            self.painter.line_segment([p - egui::vec2(size, 0.0), p + egui::vec2(size, 0.0)], s);
            self.painter.line_segment([p - egui::vec2(0.0, size), p + egui::vec2(0.0, size)], s);
        }
    }
}

/// Probes passed in: (origin, radius, kind, box world-to-unit, captured).
pub type ProbeList = Vec<(Vec3, f32, f32, Mat4, bool)>;

#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &egui::Context, world: &World, scene: &Scene, proj: &Projector, eye: Vec3, dv: &DebugView, probes: Option<ProbeList>) {
    if !dv.any_overlay() {
        return;
    }
    let lines = Lines {
        painter: ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("debug_overlay"))),
        proj,
    };
    let now = Instant::now();

    // objects near the camera, nearest first
    let mut near: Vec<(f32, usize)> = scene
        .gpu
        .iter()
        .enumerate()
        .filter(|(_, g)| !g.faces.is_empty() && !g.hud)
        .map(|(i, g)| (g.center.distance(eye), i))
        .filter(|(d, _)| *d < RANGE)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(MAX_OBJECTS);

    for &(_, idx) in &near {
        let Some(g) = scene.gpu.get(idx) else {
            continue;
        };
        let Some(o) = world.objects.get(idx) else {
            continue;
        };
        // worn objects and rigged meshes follow their avatar's box
        let worn = g.owner_avatar.is_some() || g.rigged;
        if dv.bounds
            && !worn
            && let Some((pos, rot, _)) = Scene::object_transform(world, idx, now, 0)
        {
            let half = if g.is_avatar {
                Vec3::new(0.3, 0.3, o.scale.z.max(1.0) * 0.5)
            } else {
                o.scale * 0.5
            };
            lines.obb(pos, rot, half, Color32::from_rgb(94, 234, 212));
        }
        if dv.glow_objects
            && g.faces.iter().any(|f| f.glow)
            && let Some((pos, rot, _)) = Scene::object_transform(world, idx, now, 0)
        {
            if worn {
                lines.screen_circle(g.center, g.radius, eye, ORANGE);
            } else {
                lines.obb(pos, rot, o.scale * 0.5, ORANGE);
            }
        }
        if dv.culling {
            // the decision build_lists made, on the sphere it tested
            let (color, why) = match &scene.last_cull {
                Some(v) => {
                    let d = (g.center - v.eye).length();
                    if d - g.radius > scene.draw_distance && !g.is_avatar {
                        (ORANGE, "distance")
                    } else if !v.sphere_visible(g.center, g.radius) {
                        (RED, "hors champ")
                    } else if v.pixel_size(g.center, g.radius) < 1.5 && !g.is_avatar {
                        (GREY, "trop petit")
                    } else {
                        (GREEN, "")
                    }
                }
                None => (GREY, ""),
            };
            lines.screen_circle(g.center, g.radius, eye, color);
            if !why.is_empty() && g.center.distance(eye) < 24.0 {
                lines.label(g.center, why, color);
            }
        }
    }

    if dv.lights {
        for (idx, o) in world.objects.iter() {
            let Some(l) = o.extra.light else {
                continue;
            };
            let Some(g) = scene.gpu.get(idx) else {
                continue;
            };
            if g.hud || g.center.distance(eye) > RANGE {
                continue;
            }
            let c = Color32::from_rgb(
                (l.color[0].clamp(0.0, 1.0) * 255.0) as u8,
                (l.color[1].clamp(0.0, 1.0) * 255.0) as u8,
                (l.color[2].clamp(0.0, 1.0) * 255.0) as u8,
            );
            lines.cross(g.center, 6.0, c);
            lines.sphere(g.center, l.radius, c.gamma_multiply(0.7));
            if g.center.distance(eye) < 24.0 {
                lines.label(g.center, &format!("{:.1} m, chute {:.2}", l.radius, l.falloff), c);
            }
        }
    }

    if let (true, Some(list)) = (dv.probes, probes) {
        for (origin, radius, kind, box_inv, done) in list {
            if origin.distance(eye) > RANGE + radius {
                continue;
            }
            let color = match (kind, done) {
                (_, false) => GREY,
                (0.0, _) => Color32::from_rgb(94, 234, 212),
                _ => Color32::from_rgb(139, 92, 246),
            };
            lines.cross(origin, 5.0, color);
            if kind < 0.0 {
                let m = box_inv.inverse();
                lines.cube(|x, y, z| m.transform_point3(Vec3::new(x, y, z)), color);
            } else {
                lines.sphere(origin, radius, color);
            }
        }
    }

    if dv.skeletons {
        let rig = &world.avatar_lib.rig;
        let mut bone = vec![false; rig.parent.len()];
        for (name, &j) in &rig.names {
            if let Some(b) = bone.get_mut(j) {
                *b = name.starts_with('m');
            }
        }
        let pelvis = world.avatar_lib.pelvis;
        for (idx, o) in world.objects.iter() {
            if !o.is_avatar() {
                continue;
            }
            let Some(poses) = world.avatar_poses.get(&o.full_id) else {
                continue;
            };
            let Some((pp, pr, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if pp.distance(eye) > SKELETON_RANGE {
                continue;
            }
            let joint = |j: usize| -> Option<Vec3> {
                let m = poses.get(j)?;
                let rest = rig.default_world_inv.get(j)?.inverse().w_axis.truncate();
                Some(pp + pr * (m.transform_point3(rest) - pelvis))
            };
            for j in 0..rig.parent.len().min(poses.len()) {
                let Some(parent) = rig.parent[j] else {
                    continue;
                };
                if !(bone[j] && bone.get(parent).copied().unwrap_or(false)) {
                    continue;
                }
                if let (Some(a), Some(b)) = (joint(j), joint(parent)) {
                    lines.seg(a, b, Color32::from_rgb(250, 204, 21));
                }
            }
        }
    }
}
