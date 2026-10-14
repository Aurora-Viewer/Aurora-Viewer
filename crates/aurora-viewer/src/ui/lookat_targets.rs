//! Where the avatars look, shown in the world (Preferences › Privacy ›
//! "Show look-at targets"), after LLHUDEffectLookAt::render
//! (indra/newview/llhudeffectlookat.cpp, Linden Research and Firestorm,
//! originally LGPL 2.1): a mark at each target in the colour of its type,
//! the avatar's name over it (DebugLookAtShowNames) and a line back to the
//! avatar (ExodusLookAtLines, FIRE-16912).
//!
//! Aurora's look, chosen by the human: a dot with a soft halo in the theme
//! colour of the type, the name in a small pill like the name plates, a
//! line that fades in from the avatar toward its target, all fading in when
//! a target appears. Firestorm's (`lookat::Style::Legacy`): a wire
//! crosshair in the colours of its attention sets, the name as text, a
//! solid line. In both the line leaves from between the eyes, where
//! Firestorm's leaves the avatar's root.
//!
//! Firestorm draws them in the world with the depth test and without
//! writing depth. Here they are painted on the world layer like the name
//! tags, each piece after the mark of where it stands
//! (`aurora_render::world_label_at`): the scene hides what is behind it.

use super::hud::{Projector, world_painter};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use crate::world::lookat::{self, Settings, Style, Target};
use egui::{Align2, Color32, FontId, Stroke, vec2};
use glam::Vec3;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// How far a piece stands toward the camera when the scene is asked what
/// hides it: a target lies on the surface looked at, which must not cut it.
const PULL: f32 = 0.15;
/// A line is cut into pieces of about this many points on screen, each
/// hidden on its own: a far line, short on screen, is one or two pieces.
const SCREEN_PIECE: f32 = 60.0;
/// The same in metres, for a line one end of which is off screen.
const PIECE: f32 = 0.5;
const MAX_PIECES: usize = 16;
/// Fade-in of a target that appears or changes kind or object (s).
const FADE: f32 = 0.25;
/// Legacy look: half the size of the crosshair (m, Firestorm's lines of ±1
/// scaled by 0.1) and the height of the name above it (m).
const ARM: f32 = 0.1;
const NAME_ABOVE: f32 = 0.3;

/// The name over a target, in the format of DebugLookAtShowNames (1
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

/// The colour of a type: the palette of the theme, one hue per kind of
/// attention (Firestorm gives each its own colour too).
fn color(p: &Palette, kind: u8) -> Color32 {
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

/// The colour of a type in Firestorm's look.
fn legacy_color(kind: u8) -> Color32 {
    let [r, g, b] = lookat::legacy_color(kind);
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

/// How many pieces a line gets: one per `SCREEN_PIECE` points of its length
/// on screen (`screen`), or per `PIECE` metres when an end is off screen.
fn piece_count(screen: Option<f32>, metres: f32) -> usize {
    let n = match screen {
        Some(points) => points / SCREEN_PIECE,
        None => metres / PIECE,
    };
    (n.ceil() as usize).clamp(1, MAX_PIECES)
}

/// Cut `a`–`b` into `n` pieces.
fn pieces(a: Vec3, b: Vec3, n: usize) -> impl Iterator<Item = (Vec3, Vec3)> {
    (0..n).map(move |i| (a.lerp(b, i as f32 / n as f32), a.lerp(b, (i + 1) as f32 / n as f32)))
}

/// What each avatar's target was, and since when: a new one fades in.
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
    let painter = world_painter(ctx);
    let now = Instant::now();
    let legacy = s.style == Style::Legacy;
    let shown = sources(world, s);
    fades.0.retain(|id, _| shown.iter().any(|(s, _)| s == id));
    for (id, t) in shown {
        let Some(target) = scene.look_target_pos(world, id, &t, now) else {
            continue;
        };
        let (alpha, color) = if legacy {
            (1.0, legacy_color(t.kind))
        } else {
            let alpha = fades.alpha(id, &t, now);
            (alpha, color(p, t.kind).gamma_multiply(alpha))
        };
        let center = proj.project(target).map(|(c, _)| c);
        // the line, from between the animated eyes, fading in toward the
        // target
        if s.lines
            && lookat::has_line(t.kind)
            && let Some(eyes) = scene.eyes_world(world, &id, now)
        {
            let from = proj.project(eyes).map(|(c, _)| c);
            let screen = from.zip(center).map(|(a, b)| a.distance(b));
            let n = piece_count(screen, eyes.distance(target));
            for (i, (a, b)) in pieces(eyes, target, n).enumerate() {
                let (Some((pa, _)), Some((pb, _))) = (proj.project(a), proj.project(b)) else {
                    continue;
                };
                aurora_render::world_label_at(ctx, Some(toward((a + b) * 0.5, eye)));
                let fade = if legacy { 1.0 } else { 0.15 + 0.7 * (i as f32 + 1.0) / n as f32 };
                painter.line_segment([pa, pb], Stroke::new(1.5, color.gamma_multiply(fade)));
            }
        }
        if legacy {
            aurora_render::world_label_at(ctx, Some(toward(target, eye)));
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                if let (Some((a, _)), Some((b, _))) = (proj.project(target - axis * ARM), proj.project(target + axis * ARM)) {
                    painter.line_segment([a, b], Stroke::new(1.5, color));
                }
            }
            let at = target + Vec3::Z * NAME_ABOVE;
            if let (Some(text), Some((pos, _))) = (name(world, &id, s.show_names), proj.project(at)) {
                aurora_render::world_label_at(ctx, Some(toward(at, eye)));
                let font = FontId::proportional(11.0);
                // Firestorm's drop shadow
                // theme-guard: content — the drop shadow of world text
                let shadow = Color32::from_black_alpha(200);
                painter.text(pos + vec2(1.0, 1.0), Align2::CENTER_BOTTOM, &text, font.clone(), shadow);
                painter.text(pos, Align2::CENTER_BOTTOM, &text, font, color);
            }
            continue;
        }
        let Some(c) = center else {
            continue;
        };
        aurora_render::world_label_at(ctx, Some(toward(target, eye)));
        // a soft halo, then the dot on a dark edge that keeps it readable
        // on a light background
        painter.circle_filled(c, 11.0, color.gamma_multiply(0.08));
        painter.circle_filled(c, 7.0, color.gamma_multiply(0.2));
        painter.circle_filled(c, 4.5, p.navy.gamma_multiply(0.7 * alpha));
        painter.circle_filled(c, 3.5, color);
        // the name in a small pill above the dot, like a name plate
        if let Some(text) = name(world, &id, s.show_names) {
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), p.ink.gamma_multiply(alpha));
            let size = galley.size() + vec2(14.0, 4.0);
            let rect = egui::Rect::from_center_size(c - vec2(0.0, 16.0 + size.y * 0.5), size);
            let round = size.y * 0.5;
            painter.rect_filled(rect, round, p.panel.gamma_multiply(0.85 * alpha));
            painter.rect_stroke(rect, round, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
            painter.galley(rect.center() - galley.size() * 0.5, galley, p.ink);
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
        let p: Vec<_> = pieces(a, b, 4).collect();
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].0, a);
        assert_eq!(p[3].1, b);
        assert!(p.windows(2).all(|w| w[0].1 == w[1].0));
    }

    #[test]
    fn a_line_short_on_screen_gets_few_pieces() {
        // a far line: 20 m long, 50 points on screen
        assert_eq!(piece_count(Some(50.0), 20.0), 1);
        assert_eq!(piece_count(Some(300.0), 3.0), 5);
        assert_eq!(piece_count(Some(5000.0), 3.0), MAX_PIECES);
        // an end off screen: by its length in the world
        assert_eq!(piece_count(None, 2.0), 4);
        assert_eq!(piece_count(None, 500.0), MAX_PIECES);
        assert_eq!(piece_count(Some(0.0), 0.0), 1);
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
