//! The selection and give beams in the world (`world::beams`), after
//! LLHUDEffectSpiral and LLViewerPartSourceBeam (indra/newview/
//! llhudeffecttrail.cpp, llviewerpartsource.cpp; Linden Research,
//! originally LGPL 2.1): from the left wrist of the avatar (mWristLeft) to
//! the object it edits, grabs or gives to, or to the point it aims at.
//!
//! Aurora's look, chosen by the human: a thread in the sender's colour that
//! rises in a light arc from the hand and brightens toward the target, a
//! glow running along it, the halo of the look-at targets on the point
//! aimed at with a ripple where each glow arrives, and the avatar's name in
//! a small pill (`ui::lookat_targets`). The Legacy Viewer's
//! (`beams::Style::Legacy`): its stream of particles, 0.1 m wide, each
//! crossing the beam in half a second and fading to 40 % on its way.
//!
//! Painted on the world layer like the look-at lines, each piece after the
//! mark of where it stands: the scene hides what is behind it.

use super::hud::{Projector, world_painter};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use crate::world::beams::{Beam, Settings, Style};
use egui::{Color32, FontId, Pos2, Stroke, vec2};
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

/// How far a piece stands toward the camera when the scene is asked what
/// hides it: the end of a beam lies on the surface it aims at.
const PULL: f32 = 0.15;
/// The same for the name above the target (m).
const NAME_PULL: f32 = 1.0;
/// Fade-in of a new beam (s).
const FADE_IN: f32 = 0.2;
/// The arc's rise at its middle, as a share of the beam's length, and its
/// bounds (m).
const LIFT: f32 = 0.12;
const LIFT_RANGE: (f32, f32) = (0.03, 1.5);
/// How many pieces a beam is cut into: one per this many points on
/// screen, within the bounds.
const SCREEN_PIECE: f32 = 24.0;
const PIECES: (usize, usize) = (6, 32);
/// Glows running from the hand to the target, per second, and how long the
/// lit part of each is (share of the beam): a short bright head, a longer
/// tail behind it.
const PULSE_RATE: f32 = 0.9;
const PULSE_HEAD: f32 = 0.035;
const PULSE_TAIL: f32 = 0.16;
/// The ripple a glow leaves on the target: how far it spreads (points) and
/// how long it lasts (share of a glow's period).
const RIPPLE: f32 = 22.0;
const RIPPLE_TIME: f32 = 0.45;
/// LLViewerPartSourceBeam: a particle every 0.025 s, living 0.5 s, 0.1 m
/// wide, its alpha going from the colour's to 0.4.
const PART_RATE: f32 = 0.025;
const PART_AGE: f32 = 0.5;
const PART_SIZE: f32 = 0.1;
const PART_END_ALPHA: f32 = 0.4;

/// `p` moved `PULL` toward `eye`, never more than half way.
fn toward(p: Vec3, eye: Vec3) -> Vec3 {
    let to = eye - p;
    let d = to.length();
    if d < 1e-3 { p } else { p + to / d * PULL.min(d * 0.5) }
}

/// The arc from `a` to `b` at `t` (a quadratic curve lifted at its middle).
fn arc(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let len = a.distance(b);
    let lift = (len * LIFT).clamp(LIFT_RANGE.0, LIFT_RANGE.1);
    let c = (a + b) * 0.5 + Vec3::Z * lift;
    let u = 1.0 - t;
    a * (u * u) + c * (2.0 * u * t) + b * (t * t)
}

/// How many pieces a beam gets for its length on screen (`screen`, none
/// when an end is off screen).
fn piece_count(screen: Option<f32>) -> usize {
    screen
        .map_or(PIECES.1 / 2, |points| (points / SCREEN_PIECE).ceil() as usize)
        .clamp(PIECES.0, PIECES.1)
}

/// The phase (0..1) of each glow at `time`, a beam's own `seed` keeping two
/// beams out of step.
fn pulse_phases(time: f64, seed: f32) -> [f32; 2] {
    let p = ((time * f64::from(PULSE_RATE)) as f32 + seed).fract();
    [p, (p + 0.5).fract()]
}

/// How lit the beam is at `t` by a glow at `phase`: a bright head, a tail
/// behind it, nothing ahead.
fn pulse_at(t: f32, phase: f32) -> f32 {
    let d = phase - t;
    if d >= 0.0 {
        (-(d / PULSE_TAIL).powi(2)).exp()
    } else {
        (-(d / PULSE_HEAD).powi(2)).exp()
    }
}

/// The ripple of the glows that reached the target: its spread and its
/// strength (0..1), or none between two.
fn ripple(phases: [f32; 2]) -> Option<(f32, f32)> {
    // a glow reaches the target at phase 1, then starts again from 0
    let k = phases
        .iter()
        .copied()
        .filter(|p| *p < RIPPLE_TIME)
        .fold(None, |m: Option<f32>, p| Some(m.map_or(p, |m| m.min(p))))?;
    let k = k / RIPPLE_TIME;
    Some((4.0 + RIPPLE * k, (1.0 - k).powi(2)))
}

/// A steady 0..1 number for a beam: its glows start at their own place.
fn seed(effect: &Uuid) -> f32 {
    (effect.as_u128() % 1000) as f32 / 1000.0
}

/// The sender's colour, opaque, and its alpha (0..1).
fn sender_color(c: [u8; 4]) -> (Color32, f32) {
    // theme-guard: content — the colour the sender chose for its beam
    (Color32::from_rgb(c[0], c[1], c[2]), f32::from(c[3]) / 255.0)
}

/// Where a beam leaves from: the left wrist of an avatar (LLViewerPartSourceBeam::
/// update), else the object itself.
fn source_pos(world: &World, scene: &Scene, id: &Uuid, now: Instant) -> Option<Vec3> {
    let idx = world.objects.index_of_uuid(id)?;
    if world.objects.get(idx)?.is_avatar() {
        scene.joint_world(world, id, "mWristLeft", now)
    } else {
        Some(Scene::object_transform(world, idx, now, 0)?.0)
    }
}

/// Where a beam coming from `from` enters the box of `half` extents at
/// `center`, turned by `rot`: its centre when `from` is inside.
fn surface(from: Vec3, center: Vec3, rot: Quat, half: Vec3) -> Vec3 {
    let o = rot.inverse() * (from - center);
    if (0..3).all(|i| o[i].abs() <= half[i]) {
        return center;
    }
    // along o -> 0, the box is entered once every axis is within its slab
    let enter = (0..3)
        .map(|i| {
            if o[i].abs() > half[i] {
                (o[i].abs() - half[i]) / o[i].abs()
            } else {
                0.0
            }
        })
        .fold(0.0f32, f32::max);
    center + rot * (o * (1.0 - enter))
}

/// Where a beam from `from` ends: on the surface of its target object, the
/// side it comes from (the Legacy Viewer aims at getRenderPosition, the
/// centre, where the object hides the end; none on a HUD), else at its
/// global position.
fn target_pos(world: &World, b: &Beam, from: Vec3, now: Instant) -> Option<Vec3> {
    match b.target {
        Some(id) => {
            let idx = world.objects.index_of_uuid(&id)?;
            let (p, rot, hud) = Scene::object_transform(world, idx, now, 0)?;
            let half = world.objects.get(idx)?.scale * 0.5;
            (!hud).then(|| surface(from, p, rot, half))
        }
        None => {
            let (mx, my) = world.main_origin()?;
            Some(Vec3::new(
                (b.position.x - f64::from(mx)) as f32,
                (b.position.y - f64::from(my)) as f32,
                b.position.z as f32,
            ))
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &egui::Context, p: &Palette, world: &World, scene: &Scene, proj: &Projector, eye: Vec3, s: &Settings, name_format: u8) {
    if !s.show || world.beams.shown.is_empty() {
        return;
    }
    let painter = world_painter(ctx);
    let now = Instant::now();
    let time = ctx.input(|i| i.time);
    for (effect, b) in &world.beams.shown {
        let alpha = b.alpha(now, FADE_IN);
        let Some(from) = source_pos(world, scene, &b.source, now) else {
            continue;
        };
        let Some(to) = target_pos(world, b, from, now) else { continue };
        if alpha <= 0.0 || from.distance_squared(to) < 1e-4 {
            continue;
        }
        let (color, own_alpha) = sender_color(b.color);
        let alpha = alpha * own_alpha;
        match s.style {
            Style::Legacy => particles(ctx, &painter, proj, eye, from, to, color, alpha, time),
            Style::Aurora => {
                let name = (b.source != world.agent_id)
                    .then(|| super::lookat_targets::name(world, &b.source, name_format))
                    .flatten();
                thread(ctx, &painter, p, proj, eye, from, to, color, alpha, time, seed(effect), name);
            }
        }
    }
    aurora_render::world_label_at(ctx, None);
}

/// Aurora's beam: the arc, its glows, the halo and the ripple on the target,
/// the name above it.
#[allow(clippy::too_many_arguments)]
fn thread(
    ctx: &egui::Context,
    painter: &egui::Painter,
    p: &Palette,
    proj: &Projector,
    eye: Vec3,
    from: Vec3,
    to: Vec3,
    color: Color32,
    alpha: f32,
    time: f64,
    seed: f32,
    name: Option<String>,
) {
    let ends = proj.project(from).zip(proj.project(to)).map(|((a, _), (b, _))| a.distance(b));
    let n = piece_count(ends);
    let phases = pulse_phases(time, seed);
    // a touch of white where a glow is, so a dark colour still shines
    let bright = color.lerp_to_gamma(Color32::WHITE, 0.55); // theme-guard: content — the glow's light
    let mut last: Option<Pos2> = None;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let w = arc(from, to, t);
        let Some((at, _)) = proj.project(w) else {
            last = None;
            continue;
        };
        if let Some(prev) = last {
            let mid_t = (i as f32 - 0.5) / n as f32;
            aurora_render::world_label_at(ctx, Some(toward(arc(from, to, mid_t), eye)));
            // born at the hand, brightest at the target
            let base = alpha * (0.3 + 0.7 * mid_t.powf(1.3));
            let lit = phases.iter().map(|ph| pulse_at(mid_t, *ph)).fold(0.0f32, f32::max) * alpha;
            let seg = [prev, at];
            painter.line_segment(seg, Stroke::new(11.0, color.gamma_multiply(0.06 * base + 0.14 * lit)));
            painter.line_segment(seg, Stroke::new(5.0, color.gamma_multiply(0.2 * base + 0.35 * lit)));
            painter.line_segment(seg, Stroke::new(2.0, color.lerp_to_gamma(bright, 0.25).gamma_multiply(base)));
            if lit > 0.02 {
                painter.line_segment(seg, Stroke::new(2.0 + 2.0 * lit, bright.gamma_multiply(lit)));
            }
        }
        last = Some(at);
    }
    // a spark in the hand
    if let Some((h, _)) = proj.project(from) {
        aurora_render::world_label_at(ctx, Some(toward(from, eye)));
        painter.circle_filled(h, 6.0, color.gamma_multiply(0.15 * alpha));
        painter.circle_filled(h, 2.5, bright.gamma_multiply(0.8 * alpha));
    }
    let Some((c, _)) = proj.project(to) else { return };
    aurora_render::world_label_at(ctx, Some(toward(to, eye)));
    if let Some((r, strength)) = ripple(phases) {
        painter.circle_stroke(c, r, Stroke::new(2.0, bright.gamma_multiply(0.8 * strength * alpha)));
    }
    // the look-at halo: soft light, the dot on a dark edge that keeps it
    // readable on a light background
    painter.circle_filled(c, 16.0, color.gamma_multiply(0.08 * alpha));
    painter.circle_filled(c, 9.0, color.gamma_multiply(0.24 * alpha));
    painter.circle_filled(c, 4.5, p.navy.gamma_multiply(0.7 * alpha));
    painter.circle_filled(c, 3.5, color.lerp_to_gamma(bright, 0.3).gamma_multiply(alpha));
    if let Some(text) = name {
        // above the target, the pill overlaps the object's near sides: it
        // is weighed further toward the camera than the halo
        let d = eye.distance(to);
        aurora_render::world_label_at(ctx, Some(to.lerp(eye, (NAME_PULL / d.max(1e-3)).min(0.5))));
        let galley = painter.layout_no_wrap(text, FontId::proportional(12.0), p.ink.gamma_multiply(alpha));
        let size = galley.size() + vec2(14.0, 4.0);
        let rect = egui::Rect::from_center_size(c - vec2(0.0, 20.0 + size.y * 0.5), size);
        let round = size.y * 0.5;
        painter.rect_filled(rect, round, p.panel.gamma_multiply(0.85 * alpha));
        painter.rect_stroke(
            rect,
            round,
            Stroke::new(1.0, color.gamma_multiply(0.6 * alpha)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.center() - galley.size() * 0.5, galley, p.ink);
    }
}

/// The Legacy Viewer's beam: particles from the hand to the target, each
/// crossing it in `PART_AGE`, one every `PART_RATE`.
#[allow(clippy::too_many_arguments)]
fn particles(
    ctx: &egui::Context,
    painter: &egui::Painter,
    proj: &Projector,
    eye: Vec3,
    from: Vec3,
    to: Vec3,
    color: Color32,
    alpha: f32,
    time: f64,
) {
    let count = (PART_AGE / PART_RATE).round() as usize;
    let base = (time as f32 / PART_AGE).fract();
    for k in 0..count {
        let frac = (base + k as f32 / count as f32).fract();
        // LLViewerPartSourceBeam::updatePart: from the source toward the
        // target as the particle ages
        let w = from.lerp(to, frac);
        let (Some((c, _)), Some((up, _))) = (proj.project(w), proj.project(w + Vec3::Z * PART_SIZE * 0.5)) else {
            continue;
        };
        let r = c.distance(up).max(1.0);
        let a = alpha * (1.0 + (PART_END_ALPHA - 1.0) * frac);
        aurora_render::world_label_at(ctx, Some(toward(w, eye)));
        // the soft particle texture: a glow and its centre
        painter.circle_filled(c, r, color.gamma_multiply(0.35 * a));
        painter.circle_filled(c, r * 0.5, color.gamma_multiply(a));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arc_joins_the_ends_and_rises_in_the_middle() {
        let (a, b) = (Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0));
        assert!(arc(a, b, 0.0).distance(a) < 1e-5);
        assert!(arc(a, b, 1.0).distance(b) < 1e-5);
        let mid = arc(a, b, 0.5);
        // half the control point's lift of 12 % of 4 m
        assert!((mid.z - 0.24).abs() < 1e-4, "{mid}");
        // a long beam rises no more than its bound
        let far = arc(a, Vec3::new(100.0, 0.0, 0.0), 0.5);
        assert!((far.z - LIFT_RANGE.1 * 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_glow_lights_its_head_and_tail_only() {
        assert!((pulse_at(0.5, 0.5) - 1.0).abs() < 1e-6);
        // behind it, a long tail; ahead, almost nothing
        assert!(pulse_at(0.4, 0.5) > 0.5);
        assert!(pulse_at(0.6, 0.5) < 0.01);
        let [a, b] = pulse_phases(1.25, 0.3);
        assert!((0.0..1.0).contains(&a) && (0.0..1.0).contains(&b));
        assert!(((b - a).rem_euclid(1.0) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_ripple_spreads_and_fades_after_a_glow_arrives() {
        let (r0, s0) = ripple([0.0, 0.5]).expect("a glow just arrived");
        let (r1, s1) = ripple([0.3, 0.8]).expect("still spreading");
        assert!(r1 > r0 && s1 < s0);
        assert!(ripple([0.6, 0.95]).is_none(), "none between two glows");
    }

    #[test]
    fn the_beam_ends_on_the_side_of_the_box_it_comes_from() {
        let half = Vec3::splat(0.25);
        let at = surface(Vec3::new(-4.0, 0.0, 0.0), Vec3::ZERO, Quat::IDENTITY, half);
        assert!(at.distance(Vec3::new(-0.25, 0.0, 0.0)) < 1e-5, "{at}");
        // turned a quarter: the long side faces the beam
        let rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let at = surface(Vec3::new(0.0, 4.0, 0.0), Vec3::ZERO, rot, Vec3::new(1.0, 0.1, 0.1));
        assert!(at.distance(Vec3::new(0.0, 1.0, 0.0)) < 1e-4, "{at}");
        // from inside, the centre
        assert_eq!(surface(Vec3::splat(0.1), Vec3::ZERO, Quat::IDENTITY, half), Vec3::ZERO);
    }

    #[test]
    fn a_beam_gets_more_pieces_the_longer_it_is_on_screen() {
        assert_eq!(piece_count(Some(10.0)), PIECES.0);
        assert_eq!(piece_count(Some(480.0)), 20);
        assert_eq!(piece_count(Some(5000.0)), PIECES.1);
        assert_eq!(piece_count(None), PIECES.1 / 2);
    }
}
