//! World-space overlays: avatar name tags and prim hover text.

use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use egui::{Align2, Color32, FontId, Pos2};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::time::Instant;

/// 12345 -> "12 345" (French thousands separator).
pub fn group_digits(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out
}

/// Complexity lines of the name tags.
pub struct TagComplexity<'a> {
    pub values: &'a HashMap<uuid::Uuid, u32>,
    pub areas: &'a HashMap<uuid::Uuid, f32>,
    /// Avatars shown as silhouettes (object indices).
    pub too_complex: &'a std::collections::HashSet<usize>,
    pub max: u32,
    pub max_area: f32,
    /// Only on avatars over the limit (FSTagShowTooComplexOnlyARW).
    pub only_too_complex: bool,
    /// On our own tag too (FSTagShowOwnARW).
    pub own: bool,
}

pub struct Projector {
    pub view_proj: Mat4,
    pub width: f32,
    pub height: f32,
    pub ppp: f32,
}

impl Projector {
    /// World -> egui points (None if behind the camera / off-screen).
    pub fn project(&self, p: Vec3) -> Option<(Pos2, f32)> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 0.05 {
            return None;
        }
        let ndc = c.truncate() / c.w;
        if ndc.x.abs() > 1.2 || ndc.y.abs() > 1.2 {
            return None;
        }
        let x = (ndc.x * 0.5 + 0.5) * self.width / self.ppp;
        let y = (0.5 - ndc.y * 0.5) * self.height / self.ppp;
        Some((Pos2::new(x, y), c.w))
    }
}

/// `voice`: avatars in voice chat (level, speaking) for the voice dots.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    proj: &Projector,
    eye: Vec3,
    show_own: bool,
    tag_distance: f32,
    voice: &HashMap<uuid::Uuid, (f32, bool)>,
    dots: &mut super::voice_dot::VoiceDots,
    complexity: Option<TagComplexity<'_>>,
    loading: Option<&HashMap<uuid::Uuid, f32>>,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("hud")));
    let now = Instant::now();
    // (distance, anchor, lines, voice: (id, level, speaking))
    #[allow(clippy::type_complexity)]
    let mut tags: Vec<(f32, Pos2, Vec<(String, Color32, f32)>, Option<(uuid::Uuid, f32, bool)>, Option<f32>)> = Vec::new();
    for (idx, o) in world.objects.iter() {
        if o.is_avatar() {
            if o.full_id == world.agent_id && !show_own {
                continue;
            }
            let Some((pos, _, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let d = pos.distance(eye);
            if d > tag_distance {
                continue;
            }
            let head = pos + Vec3::new(0.0, 0.0, 1.05);
            let Some((sp, _)) = proj.project(head) else {
                continue;
            };
            let mut lines = Vec::new();
            // status line first, as LLVOAvatar::idleUpdateNameTagText: "(Absent, Bloqué(e))"
            {
                use crate::world::status::{ANIM_AWAY, ANIM_DO_NOT_DISTURB};
                let anims = world.animations_of(&o.full_id);
                let playing = |id: uuid::Uuid| anims.iter().any(|a| a.id == id);
                let mut status = Vec::new();
                if playing(ANIM_AWAY) {
                    status.push("Absent");
                }
                if playing(ANIM_DO_NOT_DISTURB) {
                    status.push("Ne pas déranger");
                }
                if world.is_avatar_blocked(&o.full_id) {
                    status.push("Bloqué(e)");
                }
                if !status.is_empty() {
                    lines.push((format!("({})", status.join(", ")), p.muted, 11.0));
                }
            }
            if let Some(t) = o.group_title() {
                lines.push((t, p.muted, 11.0));
            }
            // LLVOAvatar::idleUpdateNameTagText: the display name, then the
            // username in small when one was chosen; getNameTagColor: match /
            // mismatch, Lindens by their legacy name (display names can lie)
            let names = &world.social.avatar_names;
            let (name, username, custom, legacy) = match names.get(&o.full_id) {
                Some(n) => {
                    let (main, user) = n.tag_lines(&names.options);
                    let custom = names.options.use_display_names && !n.is_default;
                    (main, user, custom, n.user_name_for_display(&names.options))
                }
                None => {
                    let legacy = o.display_name().unwrap_or_else(|| "(chargement…)".into());
                    (legacy.clone(), None, false, legacy)
                }
            };
            let col = super::colors::tag_color(world, &o.full_id, &legacy, custom);
            lines.push((name, col, 13.0));
            if let Some(u) = username {
                let dim = |v: u8| (v as f32 * 0.83) as u8;
                lines.push((
                    u,
                    Color32::from_rgba_unmultiplied(dim(col.r()), dim(col.g()), dim(col.b()), col.a()),
                    11.0,
                ));
            }
            // complexity under the name (Firestorm FSTagShowARW): red as it
            // nears the limit, green fading past it; grey for ourselves or
            // without a limit; the surface area too when it is the reason
            if let Some(tc) = &complexity {
                let own = o.full_id == world.agent_id;
                let too = tc.too_complex.contains(&idx);
                if ((own && tc.own) || (!own && (!tc.only_too_complex || too)))
                    && let Some(&c) = tc.values.get(&o.full_id)
                {
                    let ratio_color = |v: f32, m: f32| {
                        let green = 1.0 - ((v - m) / m).clamp(0.0, 1.0);
                        let red = (v / m).min(1.0);
                        Color32::from_rgb((red * 255.0) as u8, (green * 255.0) as u8, 0)
                    };
                    let col = if tc.max == 0 || own {
                        Color32::from_gray(190)
                    } else {
                        ratio_color(c as f32, tc.max as f32)
                    };
                    lines.push((format!("Complexité : {}", group_digits(c)), col, 11.0));
                    let area = tc.areas.get(&o.full_id).copied().unwrap_or(0.0);
                    if tc.max > 0 && tc.max_area > 0.0 && area > tc.max_area {
                        lines.push((
                            format!("Surface : {} m²", group_digits(area as u32)),
                            ratio_color(area, tc.max_area),
                            11.0,
                        ));
                    }
                }
            }
            let in_voice = voice.get(&o.full_id).map(|(level, speaking)| (o.full_id, *level, *speaking));
            let progress = loading.and_then(|l| l.get(&o.full_id).copied());
            tags.push((d, sp, lines, in_voice, progress));
        } else if !o.text.is_empty() {
            let Some((pos, _, hud)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            if hud {
                continue;
            }
            // LLHUDText: fade from 8 m over 4 m, anchored 0.6 x height above the
            // object and pulled towards the camera by its radius
            const FADE_DISTANCE: f32 = 8.0;
            const FADE_RANGE: f32 = 4.0;
            let c = o.text_color;
            if c[3] == 0 {
                continue;
            }
            let top = pos + Vec3::new(0.0, 0.0, o.scale.z * 0.6);
            let radius = (o.scale * 0.5).length();
            let to = top - eye;
            let anchor = if to.length() > radius + 0.3 {
                top - to.normalize() * radius
            } else {
                top
            };
            let d = anchor.distance(eye);
            if d > FADE_DISTANCE + FADE_RANGE {
                continue;
            }
            let fade = if d > FADE_DISTANCE {
                (1.0 - (d - FADE_DISTANCE) / FADE_RANGE).max(0.0)
            } else {
                1.0
            };
            let Some((sp, _)) = proj.project(anchor) else {
                continue;
            };
            let col = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], (c[3] as f32 * fade) as u8);
            let lines = o.text.lines().map(|l| (l.to_owned(), col, 12.0)).collect();
            tags.push((d + 1000.0, sp, lines, None, None));
        }
    }
    let now_s = ctx.input(|i| i.time);
    // far first so near tags draw on top
    tags.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (d, sp, lines, in_voice, progress) in tags {
        let is_avatar = d < 1000.0;
        let fade = if is_avatar {
            (1.0 - (d - 40.0) / 24.0).clamp(0.35, 1.0)
        } else {
            1.0
        };
        let mut y = sp.y;
        // measure
        let galleys: Vec<_> = lines
            .iter()
            .map(|(t, c, s)| painter.layout_no_wrap(t.clone(), FontId::proportional(*s), c.gamma_multiply(fade)))
            .collect();
        let total_h: f32 = galleys.iter().map(|g| g.size().y).sum();
        let max_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
        y -= total_h;
        // in voice chat: the voice dot sits in the tag, left of the name
        let voice_w = if in_voice.is_some() { super::voice_dot::TAG_WIDTH } else { 0.0 };
        let text_x = sp.x + voice_w * 0.5;
        if is_avatar {
            let w = max_w + 12.0 + voice_w;
            let r = egui::Rect::from_min_size(Pos2::new(sp.x - w * 0.5, y - 3.0), egui::vec2(w, total_h + 6.0));
            painter.rect_filled(
                r,
                3.0,
                Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), (190.0 * fade) as u8),
            );
            // still loading: thin progress bar along the bottom of the tag
            // (violet -> turquoise, like the loading screens)
            if let Some(f) = progress {
                let track = egui::Rect::from_min_max(
                    Pos2::new(r.left() + 4.0, r.bottom() - 3.0),
                    Pos2::new(r.right() - 4.0, r.bottom() - 1.0),
                );
                painter.rect_filled(track, 1.0, p.field.gamma_multiply(fade));
                let fill = egui::Rect::from_min_max(
                    track.min,
                    Pos2::new(track.left() + track.width() * f.clamp(0.02, 1.0), track.bottom()),
                );
                let (a, b) = (p.violet.gamma_multiply(fade), p.teal.gamma_multiply(fade));
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(fill.left_top(), a);
                mesh.colored_vertex(fill.right_top(), b);
                mesh.colored_vertex(fill.right_bottom(), b);
                mesh.colored_vertex(fill.left_bottom(), a);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                painter.add(mesh);
            }
            if let Some((id, level, speaking)) = in_voice {
                // centered on the name (last line)
                let name_h = galleys.last().map(|g| g.size().y).unwrap_or(total_h);
                let c = Pos2::new(r.left() + 4.0 + voice_w * 0.5, r.bottom() - 3.0 - name_h * 0.5);
                dots.paint(&painter, p, id, c, level, speaking, now_s, fade);
            }
        }
        for g in galleys {
            let h = g.size().y;
            let pos = Pos2::new(text_x, y);
            let rect = Align2::CENTER_TOP.anchor_size(pos, g.size());
            if !is_avatar {
                // drop shadow (LLFontGL::DROP_SHADOW)
                let a = g.job.sections.first().map(|s| s.format.color.a()).unwrap_or(255);
                let shadow = painter.layout_no_wrap(
                    g.job.text.clone(),
                    g.job.sections.first().map(|s| s.format.font_id.clone()).unwrap_or_default(),
                    Color32::from_black_alpha((a as f32 * 0.7) as u8),
                );
                painter.galley(rect.min + egui::vec2(1.0, 1.0), shadow, Color32::BLACK);
            }
            painter.galley(rect.min, g, Color32::WHITE);
            y += h;
        }
    }
    dots.prune(now_s);
}
