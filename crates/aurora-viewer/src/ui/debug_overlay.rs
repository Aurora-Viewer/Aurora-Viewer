//! Debug overlays drawn over the 3D view (Preferences › Debug): bounding
//! boxes, culling decisions, lights, glowing objects, reflection probes,
//! avatar skeletons and detail levels. Lines are projected on the CPU and
//! painted by egui, on top of the scene (no depth test).

use super::hud::Projector;
use crate::scene::{LodInfo, Scene};
use crate::settings::DebugView;
use crate::world::World;
use egui::{Color32, Stroke};
use glam::{Mat4, Quat, Vec3};
use std::time::Instant;

/// Objects considered around the camera (nearest first).
const MAX_OBJECTS: usize = 600;
pub const RANGE: f32 = 96.0;
/// Detail levels: labels drawn at most, and how far from the pointer
/// (points) an object is labelled.
const LOD_LABELS: usize = 16;
const LOD_REACH: f32 = 150.0;
/// Height of a label line (points).
const LOD_LINE: f32 = 13.0;
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

/// The label of a detail level, like Firestorm's "LOD Info" (LOD, Dist,
/// BiasedR): the level shown, the one the distance asks for when it is
/// another, that distance and the radius, and how many objects share them
/// (the rigged meshes an avatar wears are judged together).
fn lod_text(info: &LodInfo, count: usize) -> String {
    let mut s = format!("LOD {}", info.shown);
    if info.asked != info.shown {
        s.push_str(&format!(" → {}", info.asked));
    }
    s.push_str(&format!(
        " · d {} · r {}",
        aurora_i18n::number(info.distance as f64, 2),
        aurora_i18n::number(info.radius as f64, 2)
    ));
    if info.rigged {
        s.push_str(" · ");
        s.push_str(aurora_i18n::tr!("ui-debug-overlay-rigged"));
    }
    if count > 1 {
        s.push_str(&format!(" ×{count}"));
    }
    s
}

/// Green at the finest level, orange below it, red while the level shown
/// is not the one asked for (its geometry is on its way, or never came).
fn lod_color(info: &LodInfo) -> Color32 {
    if info.shown != info.asked {
        RED
    } else if info.shown >= 3 {
        GREEN
    } else {
        ORANGE
    }
}

/// The labels to draw: the objects that show within `reach` of the pointer,
/// those it is over first (the disc of their bounds), then the nearest to
/// it, `max` at most. `on_screen` gives where a point shows. Objects with
/// the same numbers (what an avatar wears) make one label, at the first of
/// them.
fn lod_labels(
    infos: &[LodInfo],
    pointer: egui::Pos2,
    reach: f32,
    max: usize,
    on_screen: impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Vec<(egui::Pos2, LodInfo, usize)> {
    let mut near: Vec<((f32, f32), egui::Pos2, LodInfo)> = infos
        .iter()
        .filter_map(|i| {
            let p = on_screen(i.at)?;
            let d = p.distance(pointer);
            // (the size of its bounds on screen, from a point above it)
            let disc = on_screen(i.at + Vec3::Z * i.bounds).map_or(0.0, |q| q.distance(p));
            let outside = (d - disc).max(0.0);
            (outside <= reach).then_some(((outside, d), p, *i))
        })
        .collect();
    near.sort_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)));
    let mut out: Vec<(egui::Pos2, LodInfo, usize)> = Vec::new();
    for (_, p, info) in near {
        let same = |o: &LodInfo| {
            o.rigged
                && info.rigged
                && (o.shown, o.asked) == (info.shown, info.asked)
                && o.radius == info.radius
                && o.distance == info.distance
        };
        if let Some(group) = out.iter_mut().find(|(_, o, _)| same(o)) {
            group.2 += 1;
        } else if out.len() < max {
            out.push((p, info, 1));
        }
    }
    // one under the other where they would cover each other
    out.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    for i in 1..out.len() {
        let (above, here) = (out[i - 1].0, out[i].0);
        if (here.x - above.x).abs() < 170.0 && here.y < above.y + LOD_LINE {
            out[i].0.y = above.y + LOD_LINE;
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    world: &World,
    scene: &Scene,
    proj: &Projector,
    eye: Vec3,
    dv: &DebugView,
    probes: Option<ProbeList>,
    lod: &[LodInfo],
) {
    if !dv.any_overlay() {
        return;
    }
    let lines = Lines {
        painter: ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("debug_overlay"))),
        proj,
    };
    let now = Instant::now();

    if dv.lod_info {
        let pointer = ctx.input(|i| i.pointer.latest_pos()).unwrap_or_else(|| ctx.content_rect().center());
        let font = egui::FontId::proportional(11.0);
        for (at, info, count) in lod_labels(lod, pointer, LOD_REACH, LOD_LABELS, |p| proj.project(p).map(|(p, _)| p)) {
            let text = lod_text(&info, count);
            // (a dark copy under it: the label is read over any scene)
            let shadow = Color32::from_black_alpha(200);
            lines
                .painter
                .text(at + egui::vec2(5.0, 1.0), egui::Align2::LEFT_CENTER, &text, font.clone(), shadow);
            lines.painter.text(
                at + egui::vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                &text,
                font.clone(),
                lod_color(&info),
            );
            lines.painter.circle_filled(at, 2.0, lod_color(&info));
        }
    }

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
                        (RED, aurora_i18n::tr!("ui-debug-overlay-outside-view"))
                    } else if v.pixel_size(g.center, g.radius) < 1.5 && !g.is_avatar {
                        (GREY, aurora_i18n::tr!("ui-debug-overlay-too-small"))
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
                lines.label(
                    g.center,
                    &aurora_i18n::tr!(
                        "status-m-falling",
                        value0 = aurora_i18n::number_text(&format!("{:.1}", l.radius)),
                        value1 = aurora_i18n::number_text(&format!("{:.2}", l.falloff))
                    ),
                    c,
                );
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
        for (idx, o) in world.objects.avatars() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn info(x: f32, shown: u8, asked: u8, distance: f32, rigged: bool) -> LodInfo {
        LodInfo {
            at: Vec3::new(x, 0.0, 0.0),
            bounds: 0.0,
            shown,
            asked,
            radius: 1.25,
            distance,
            rigged,
        }
    }

    #[test]
    fn a_label_tells_the_level_and_what_it_comes_from() {
        // (numbers and words in the language the tests run in)
        let n = |v: f64| aurora_i18n::number(v, 2);
        let r = n(1.25);
        assert_eq!(lod_text(&info(0.0, 3, 3, 12.5, false), 1), format!("LOD 3 · d {} · r {r}", n(12.5)));
        assert_eq!(
            lod_text(&info(0.0, 1, 2, 40.0, false), 1),
            format!("LOD 1 → 2 · d {} · r {r}", n(40.0))
        );
        assert_eq!(
            lod_text(&info(0.0, 2, 2, 23.0, true), 6),
            format!("LOD 2 · d {} · r {r} · {} ×6", n(23.0), aurora_i18n::tr!("ui-debug-overlay-rigged"))
        );
        assert_eq!(lod_color(&info(0.0, 3, 3, 1.0, false)), GREEN);
        assert_eq!(lod_color(&info(0.0, 2, 2, 1.0, false)), ORANGE);
        assert_eq!(lod_color(&info(0.0, 2, 3, 1.0, false)), RED);
    }

    #[test]
    fn labels_are_those_around_the_pointer() {
        // objects along x, shown where they are (1 m = 1 point)
        let on_screen = |p: Vec3| (p.x >= 0.0).then_some(egui::pos2(p.x, 100.0));
        let infos = [
            info(500.0, 3, 3, 10.0, false),
            info(520.0, 2, 2, 30.0, false),
            // behind the camera: not shown
            info(-5.0, 2, 2, 30.0, false),
            // too far from the pointer
            info(900.0, 1, 1, 80.0, false),
            // three rigged meshes of one avatar, and one of another
            info(560.0, 2, 2, 25.0, true),
            info(561.0, 2, 2, 25.0, true),
            info(562.0, 2, 2, 25.0, true),
            info(563.0, 2, 2, 26.0, true),
        ];
        let labels = lod_labels(&infos, egui::pos2(510.0, 100.0), 150.0, 16, on_screen);
        assert_eq!(labels.len(), 4);
        let counts: Vec<usize> = labels.iter().map(|l| l.2).collect();
        assert_eq!(counts.iter().sum::<usize>(), 6);
        assert!(counts.contains(&3), "{counts:?}");
        // none over another: a line apart where they are close
        for (i, a) in labels.iter().enumerate() {
            for b in &labels[i + 1..] {
                assert!((a.0.y - b.0.y).abs() >= LOD_LINE - 0.01, "{:?} {:?}", a.0, b.0);
            }
        }
        // no more than asked, the nearest to the pointer first
        let two = lod_labels(&infos, egui::pos2(510.0, 100.0), 150.0, 2, on_screen);
        assert_eq!(two.len(), 2);
        assert!(two.iter().all(|l| !l.1.rigged || l.2 == 3), "{two:?}");
        // a large object the pointer is over comes before a small one nearer
        // to it: its centre is far, its bounds are under the pointer
        let on_plane = |p: Vec3| Some(egui::pos2(p.x, 100.0 - p.z));
        let mut large = info(100.0, 1, 1, 60.0, false);
        large.bounds = 300.0;
        let small = info(400.0, 3, 3, 5.0, false);
        let first = lod_labels(&[small, large], egui::pos2(380.0, 100.0), 150.0, 1, on_plane);
        assert_eq!(first[0].1.shown, 1);
    }
}
