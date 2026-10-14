//! Where the avatars look, shown in the world (Preferences › Privacy ›
//! "Show look-at targets"), port of LLHUDEffectLookAt::render
//! (indra/newview/llhudeffectlookat.cpp, Linden Research and Firestorm,
//! originally LGPL 2.1): a crosshair at each target in the colour of its
//! type, the avatar's name over it (DebugLookAtShowNames) and a line back to
//! the avatar (ExodusLookAtLines, FIRE-16912).
//!
//! Firestorm draws them in the world with the depth test and without
//! writing depth. Here they are painted on the world layer like the name
//! tags, each piece after the mark of where it stands
//! (`aurora_render::world_label_at`): the scene hides what is behind it.

use super::hud::{Projector, world_painter};
use crate::scene::Scene;
use crate::world::World;
use crate::world::lookat::{self, Settings, Target};
use egui::{Align2, Color32, FontId, Stroke, vec2};
use glam::Vec3;
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

/// Cut `a`–`b` into pieces of about `PIECE` metres.
fn pieces(a: Vec3, b: Vec3) -> impl Iterator<Item = (Vec3, Vec3)> {
    let n = ((a.distance(b) / PIECE).ceil() as usize).clamp(1, MAX_PIECES);
    (0..n).map(move |i| (a.lerp(b, i as f32 / n as f32), a.lerp(b, (i + 1) as f32 / n as f32)))
}

pub fn draw(ctx: &egui::Context, world: &World, scene: &Scene, proj: &Projector, eye: Vec3, s: &Settings) {
    if !s.show_targets {
        return;
    }
    let painter = world_painter(ctx);
    let now = Instant::now();
    let seg = |a: Vec3, b: Vec3, color: Color32| {
        if let (Some((pa, _)), Some((pb, _))) = (proj.project(a), proj.project(b)) {
            painter.line_segment([pa, pb], Stroke::new(1.5, color));
        }
    };
    for (id, t) in sources(world, s) {
        let Some(idx) = world.objects.index_of_uuid(&id) else {
            continue;
        };
        let Some(target) = scene.look_target_pos(world, id, &t, now) else {
            continue;
        };
        let color = color(t.kind);
        if s.lines && lookat::has_line(t.kind) {
            // to the avatar's position, a little above it
            if let Some((root, _, _)) = Scene::object_transform(world, idx, now, 0) {
                for (a, b) in pieces(target, root + Vec3::Z * 0.05) {
                    aurora_render::world_label_at(ctx, Some(toward((a + b) * 0.5, eye)));
                    seg(a, b, color);
                }
            }
        }
        aurora_render::world_label_at(ctx, Some(toward(target, eye)));
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            seg(target - axis * ARM, target + axis * ARM, color);
        }
        if let Some(text) = name(world, &id, s.show_names) {
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
    }
    aurora_render::world_label_at(ctx, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_cut_into_pieces_that_join() {
        let (a, b) = (Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
        let p: Vec<_> = pieces(a, b).collect();
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].0, a);
        assert_eq!(p[3].1, b);
        assert!(p.windows(2).all(|w| w[0].1 == w[1].0));
        assert_eq!(pieces(a, Vec3::new(500.0, 0.0, 0.0)).count(), MAX_PIECES);
        assert_eq!(pieces(a, a).count(), 1);
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
