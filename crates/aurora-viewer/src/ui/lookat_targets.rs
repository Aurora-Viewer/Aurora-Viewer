//! Where the avatars look, shown in the world (Preferences › Privacy ›
//! "Show look-at targets"), port of LLHUDEffectLookAt::render
//! (indra/newview/llhudeffectlookat.cpp, Linden Research and Firestorm,
//! originally LGPL 2.1): a crosshair at each target in the colour of its
//! type, the avatar's name over it (DebugLookAtShowNames) and a line back to
//! the avatar (ExodusLookAtLines, FIRE-16912). The line leaves from
//! between the eyes rather than the avatar's root.
//!
//! Firestorm draws them in the world with the depth test and without
//! writing depth. Here they are painted on the world layer like the name
//! tags, each piece after the mark of where it stands
//! (`aurora_render::world_label_at`): the scene hides what is behind it.

use super::hud::{Projector, world_painter};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use crate::world::lookat::{self, Settings, Target};
use egui::{Align2, Color32, FontId, Stroke, vec2};
use glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// Half the size of a crosshair (m): Firestorm's lines of ±1 scaled by 0.1.
const ARM: f32 = 0.1;
/// The name stands this far above the crosshair (m).
const NAME_ABOVE: f32 = 0.3;
/// How far a piece stands toward the camera when the scene is asked what
/// hides it: a target lies on the surface looked at, which must not cut it.
const PULL: f32 = 0.15;
/// Length (m) of the pieces a line is cut into, each hidden on its own.
const PIECE: f32 = 0.5;
const MAX_PIECES: usize = 24;
/// Dots and gaps of the ring style's line (m), and how many at most.
const DOT: f32 = 0.08;
const MAX_DOTS: usize = 60;

/// The name over a crosshair, in the format of DebugLookAtShowNames (1
/// complete, 2 display, 3 legacy "Firstname Lastname", 4 username); none
/// for 0 or a name not known yet.
fn name(world: &World, id: &Uuid, format: u8) -> Option<String> {
    let names = &world.social.avatar_names;
    let n = names.get(id)?;
    let o = &names.options;
    Some(match format {
        0 => return None,
        1 => n.complete(o),
        2 => n.display(o),
        3 => n.user_name_for_display(o),
        _ => n.username.clone(),
    })
}

/// The colour of a type (the attention sets of Firestorm).
fn color(kind: u8) -> Color32 {
    let [r, g, b] = lookat::color(kind);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// `p` moved `PULL` toward `eye`, never more than half way.
fn toward(p: Vec3, eye: Vec3) -> Vec3 {
    let to = eye - p;
    let d = to.length();
    if d < 1e-3 { p } else { p + to / d * PULL.min(d * 0.5) }
}

/// Whose targets are drawn: ours unless hidden (DebugLookAtHideOwn) or
/// private (PrivateLookAtTarget), and every other avatar's.
pub fn sources(world: &World, s: &Settings) -> Vec<(Uuid, Target)> {
    let me = world.agent_id;
    let mut out: Vec<(Uuid, Target)> = world
        .look_at
        .remote
        .iter()
        .filter(|(id, _)| **id != me)
        .map(|(id, t)| (*id, *t))
        .collect();
    if !(s.hide_own || s.private) {
        out.push((me, world.look_at.own.target));
    }
    out.retain(|(_, t)| t.kind != lookat::NONE);
    out
}

/// Cut `a`–`b` into pieces of about `len` metres, at most `max`.
fn pieces(a: Vec3, b: Vec3, len: f32, max: usize) -> impl Iterator<Item = (Vec3, Vec3)> {
    let n = ((a.distance(b) / len).ceil() as usize).clamp(1, max);
    (0..n).map(move |i| (a.lerp(b, i as f32 / n as f32), a.lerp(b, (i + 1) as f32 / n as f32)))
}

/// How the targets look, while the human chooses
/// (`AURORA_LOOKAT_STYLE`): Firestorm's wire crosshair, or one of two
/// Aurora proposals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// LLHUDEffectLookAt::render: wire crosshair, attention colours, text
    /// name, solid line.
    Cross,
    /// A glowing dot with a soft halo, a line that fades in from the head,
    /// the name in a small pill.
    Halo,
    /// A thin ring around a dot, a dotted line, the name in light text.
    Ring,
}

impl Style {
    /// AURORA_LOOKAT_STYLE=cross|halo|ring (cross by default).
    pub fn from_env() -> Style {
        static STYLE: std::sync::OnceLock<Style> = std::sync::OnceLock::new();
        *STYLE.get_or_init(|| match std::env::var("AURORA_LOOKAT_STYLE").as_deref() {
            Ok("halo") => Style::Halo,
            Ok("ring") => Style::Ring,
            _ => Style::Cross,
        })
    }
}

/// The colour of a type in the Aurora styles: the palette of the theme,
/// one hue per kind of attention.
fn theme_color(p: &Palette, kind: u8) -> Color32 {
    match kind {
        lookat::IDLE | lookat::AUTO_LISTEN | lookat::NONE => p.muted,
        lookat::FREELOOK => p.indigo_light,
        lookat::RESPOND | lookat::CONVERSATION => p.violet_pale,
        lookat::HOVER => p.teal,
        lookat::SELECT => p.rose,
        lookat::FOCUS => p.violet_light,
        lookat::MOUSELOOK => p.amber,
        _ => p.ink,
    }
}

/// Fade-in of a target that appears or changes kind or object (s).
const FADE: f32 = 0.25;

/// What each avatar's target was, and since when: the Aurora styles fade
/// in a new one.
#[derive(Default)]
pub struct Fades(HashMap<Uuid, (u8, Option<Uuid>, Instant)>);

impl Fades {
    /// 0..1, how far the target of `id` has faded in.
    fn alpha(&mut self, id: Uuid, t: &Target, now: Instant) -> f32 {
        let e = self.0.entry(id).or_insert((t.kind, t.object, now));
        if (e.0, e.1) != (t.kind, t.object) {
            *e = (t.kind, t.object, now);
        }
        (now.duration_since(e.2).as_secs_f32() / FADE).min(1.0)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(ctx: &egui::Context, p: &Palette, world: &World, scene: &Scene, proj: &Projector, eye: Vec3, s: &Settings, fades: &mut Fades) {
    if !s.show_targets {
        fades.0.clear();
        return;
    }
    let style = Style::from_env();
    let painter = world_painter(ctx);
    let now = Instant::now();
    let seg = |a: Vec3, b: Vec3, color: Color32, width: f32| {
        if let (Some((pa, _)), Some((pb, _))) = (proj.project(a), proj.project(b)) {
            painter.line_segment([pa, pb], Stroke::new(width, color));
        }
    };
    let shown = sources(world, s);
    fades.0.retain(|id, _| shown.iter().any(|(s, _)| s == id));
    for (id, t) in shown {
        let Some(target) = scene.look_target_pos(world, id, &t, now) else {
            continue;
        };
        let alpha = match style {
            Style::Cross => 1.0,
            _ => fades.alpha(id, &t, now),
        };
        let color = match style {
            Style::Cross => color(t.kind),
            _ => theme_color(p, t.kind),
        }
        .gamma_multiply(alpha);
        // the line leaves from between the animated eyes (Firestorm's
        // leaves the avatar's root, a little above it)
        if s.lines
            && lookat::has_line(t.kind)
            && let Some(head) = scene.eyes_world(world, &id, now)
        {
            // the dots of the ring style are short, whatever the length
            let (len, max) = if style == Style::Ring {
                (DOT, MAX_DOTS)
            } else {
                (PIECE, MAX_PIECES)
            };
            let all: Vec<_> = pieces(head, target, len, max).collect();
            let n = all.len() as f32;
            for (i, (a, b)) in all.into_iter().enumerate() {
                let (c, width) = match style {
                    Style::Cross => (color, 1.5),
                    // fades in from the head toward the target
                    Style::Halo => (color.gamma_multiply(0.15 + 0.7 * (i as f32 + 1.0) / n), 1.5),
                    // dotted
                    Style::Ring if i % 2 == 1 => continue,
                    Style::Ring => (color.gamma_multiply(0.8), 1.25),
                };
                aurora_render::world_label_at(ctx, Some(toward((a + b) * 0.5, eye)));
                seg(a, b, c, width);
            }
        }
        aurora_render::world_label_at(ctx, Some(toward(target, eye)));
        let center = proj.project(target).map(|(c, _)| c);
        match (style, center) {
            (Style::Cross, _) => {
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    seg(target - axis * ARM, target + axis * ARM, color, 1.5);
                }
            }
            (Style::Halo, Some(c)) => {
                for (r, a) in [(13.0, 0.06), (9.5, 0.12), (6.5, 0.25)] {
                    painter.circle_filled(c, r, color.gamma_multiply(a));
                }
                // a dark edge keeps the dot readable on a light background
                painter.circle_filled(c, 4.5, p.navy.gamma_multiply(0.7 * alpha));
                painter.circle_filled(c, 3.5, color);
            }
            (Style::Ring, Some(c)) => {
                painter.circle_stroke(c, 7.5, Stroke::new(1.0, p.navy.gamma_multiply(0.6 * alpha)));
                painter.circle_stroke(c, 6.0, Stroke::new(2.0, color));
                painter.circle_filled(c, 1.75, color);
            }
            _ => {}
        }
        let Some(text) = name(world, &id, s.show_names) else {
            continue;
        };
        match style {
            Style::Cross => {
                let at = target + Vec3::Z * NAME_ABOVE;
                if let Some((p, _)) = proj.project(at) {
                    aurora_render::world_label_at(ctx, Some(toward(at, eye)));
                    let font = FontId::proportional(11.0);
                    // Firestorm's drop shadow
                    // theme-guard: content — the drop shadow of world text
                    let shadow = Color32::from_black_alpha(200);
                    painter.text(p + vec2(1.0, 1.0), Align2::CENTER_BOTTOM, &text, font.clone(), shadow);
                    painter.text(p, Align2::CENTER_BOTTOM, &text, font, color);
                }
            }
            Style::Halo => {
                // a small pill above the dot, like a name plate
                let Some(c) = center else { continue };
                let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), p.ink.gamma_multiply(alpha));
                let size = galley.size() + vec2(14.0, 4.0);
                let rect = egui::Rect::from_center_size(c - vec2(0.0, 16.0 + size.y * 0.5), size);
                let round = size.y * 0.5;
                painter.rect_filled(rect, round, p.panel.gamma_multiply(0.85 * alpha));
                painter.rect_stroke(rect, round, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                painter.galley(rect.center() - galley.size() * 0.5, galley, p.ink);
            }
            Style::Ring => {
                let Some(c) = center else { continue };
                let font = FontId::proportional(10.5);
                let at = c - vec2(0.0, 12.0);
                painter.text(
                    at + vec2(0.0, 1.0),
                    Align2::CENTER_BOTTOM,
                    &text,
                    font.clone(),
                    p.navy.gamma_multiply(0.8 * alpha),
                );
                painter.text(at, Align2::CENTER_BOTTOM, &text, font, p.ink.gamma_multiply(0.9 * alpha));
            }
        }
    }
    aurora_render::world_label_at(ctx, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_cut_into_pieces_that_join() {
        let (a, b) = (Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let p: Vec<_> = pieces(a, b, PIECE, MAX_PIECES).collect();
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].0, a);
        assert_eq!(p[3].1, b);
        assert!(p.windows(2).all(|w| w[0].1 == w[1].0));
        assert_eq!(pieces(a, Vec3::new(500.0, 0.0, 0.0), PIECE, MAX_PIECES).count(), MAX_PIECES);
        assert_eq!(pieces(a, a, PIECE, MAX_PIECES).count(), 1);
    }

    #[test]
    fn pieces_stand_toward_the_camera_but_never_past_half_way() {
        let eye = Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(toward(Vec3::ZERO, eye), Vec3::new(PULL, 0.0, 0.0));
        assert!((toward(Vec3::new(9.9, 0.0, 0.0), eye).x - 9.95).abs() < 1e-5);
    }

    #[test]
    fn our_target_is_hidden_when_asked_or_private() {
        use crate::scene::avatar::AvatarLibrary;
        use std::sync::Arc;
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let focus = Target {
            kind: lookat::FOCUS,
            object: None,
            offset: glam::DVec3::new(1.0, 2.0, 3.0),
        };
        world.look_at.remote.insert(other, focus);
        world.look_at.remote.insert(Uuid::from_u128(3), Target::NONE);
        world.look_at.own.set(lookat::FOCUS, None, focus.offset, Instant::now());
        let ids = |s: &Settings| {
            let mut v: Vec<Uuid> = sources(&world, s).into_iter().map(|(id, _)| id).collect();
            v.sort();
            v
        };
        // the defaults hide our own; a "none" target is never drawn
        assert_eq!(ids(&Settings::default()), vec![other]);
        let shown = Settings {
            hide_own: false,
            ..Settings::default()
        };
        assert_eq!(ids(&shown), vec![world.agent_id, other]);
        let private = Settings {
            hide_own: false,
            private: true,
            ..Settings::default()
        };
        assert_eq!(ids(&private), vec![other]);
    }
}
