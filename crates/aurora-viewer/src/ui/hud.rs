//! World-space overlays: avatar name tags and prim hover text.

use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use egui::{Align2, Color32, FontId, Pos2};
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::time::Instant;

/// Format complexity with the selected thousands separator.
pub fn group_digits(n: u32) -> String {
    aurora_i18n::integer(i64::from(n))
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

/// How far an avatar's name tag stands in front of its anchor, toward the
/// camera, when the scene is asked what it hides of it: the avatar's own
/// head, hair or hat never cut its name. LLHUDNameTag::updateVisibility
/// pulls the tag by the radius of the whole avatar, which lets the name of
/// someone standing a metre behind a wall show through it; half a metre
/// clears a head and keeps more names behind their walls.
pub const TAG_PULL: f32 = 0.5;

/// Where the name tag anchored at `head` stands for the scene's depth:
/// `TAG_PULL` toward `eye`, and never more than half way.
pub fn tag_point(head: Vec3, eye: Vec3) -> Vec3 {
    let to = eye - head;
    let d = to.length();
    if d < 1e-3 { head } else { head + to / d * TAG_PULL.min(d * 0.5) }
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
        self.screen_point(ndc.x, ndc.y).map(|point| (point, c.w))
    }

    /// Centre on the avatar's projected root; use its stable height only
    /// for the vertical placement. Projecting the elevated anchor's x as
    /// well makes perspective shift the tag sideways with a pitched camera.
    /// Deliberate difference from LLVOAvatar::idleCalcNameTagPosition
    /// (indra/newview/llvoavatar.cpp): head motion never sets the centre.
    pub fn project_name_tag(&self, root: Vec3, anchor: Vec3) -> Option<Pos2> {
        let root = self.view_proj * root.extend(1.0);
        let anchor = self.view_proj * anchor.extend(1.0);
        if root.w <= 0.05 || anchor.w <= 0.05 {
            return None;
        }
        self.screen_point(root.x / root.w, anchor.y / anchor.w)
    }

    fn screen_point(&self, x: f32, y: f32) -> Option<Pos2> {
        if x.abs() > 1.2 || y.abs() > 1.2 {
            return None;
        }
        Some(Pos2::new(
            (x * 0.5 + 0.5) * self.width / self.ppp,
            (0.5 - y * 0.5) * self.height / self.ppp,
        ))
    }
}

/// An avatar name tag on screen (egui points): it takes the clicks like its
/// avatar (LLPipeline::lineSegmentIntersectInWorld picks the tags).
pub struct NameTag {
    pub id: uuid::Uuid,
    pub rect: egui::Rect,
}

/// Painter of what belongs to the world (name tags, floating texts, the
/// tracking beacon). Nothing of it may pass in front of an element of the
/// interface: it is painted on the layer egui draws first, before the bars
/// are laid out on that layer, and `aurora_render::end_world_labels` ends
/// it, so that the renderer draws it before the HUD attachments too.
pub fn world_painter(ctx: &egui::Context) -> egui::Painter {
    ctx.layer_painter(egui::LayerId::background())
}

/// llSetText on our HUD attachments, without world-distance fading. Painted
/// after the world's labels and before the bars: over the HUD attachments,
/// under the interface.
pub fn draw_attachments(ctx: &egui::Context, world: &World, scene: &Scene) {
    let Some(view) = scene.lists.hud_view else { return };
    let own = world.objects.index_of_uuid(&world.agent_id);
    let ppp = ctx.pixels_per_point();
    let proj = Projector {
        view_proj: view.matrix(),
        width: view.width,
        height: view.height,
        ppp,
    };
    let painter = ctx.layer_painter(egui::LayerId::background());
    for (idx, o) in world.objects.texted() {
        if o.text_color[3] == 0 || !scene.gpu.get(idx).is_some_and(|g| g.hud && g.owner_avatar == own) {
            continue;
        }
        let Some((pos, _, _)) = Scene::object_transform(world, idx, Instant::now(), 0) else {
            continue;
        };
        let scale = world.hud_scale(idx);
        let Some((point, _)) = proj.project(pos + Vec3::Z * o.scale.z * scale * 0.6) else {
            continue;
        };
        let c = o.text_color;
        painter.text(
            point,
            egui::Align2::CENTER_BOTTOM,
            &o.text,
            egui::FontId::proportional(12.0 * scale),
            Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
        );
    }
}

/// The name plate style of the avatar tags (`ui::nameplate`): what the
/// plates remember and what they read besides the world.
pub struct PlateFrame<'a> {
    pub plates: &'a mut super::nameplate::Plates,
    /// Profile pictures already loaded, by avatar.
    pub pictures: &'a HashMap<uuid::Uuid, egui::TextureHandle>,
    /// Avatars shown as silhouettes (object indices).
    pub too_complex: &'a std::collections::HashSet<usize>,
    /// Demo: an avatar whose plate is shown as too complex.
    pub demo_too_complex: Option<uuid::Uuid>,
}

/// What a name plate says about the avatar `o`. The states are those of
/// LLVOAvatar::idleUpdateNameTagText (away, do not disturb, typing, muted,
/// friend); the names and the complexity are those of the classic tag.
fn plate_content(
    world: &World,
    idx: usize,
    o: &crate::world::objects::Object,
    in_voice: bool,
    complexity: Option<&TagComplexity<'_>>,
    too_complex: bool,
) -> super::nameplate::Content {
    use super::nameplate::{Content, Detail, Presence, Role};
    use super::presence::Status;
    let id = o.full_id;
    let own = id == world.agent_id;
    let anims = world.animations_of(&id);
    let playing = |anim: uuid::Uuid| anims.iter().any(|a| a.id == anim);
    // the source of the Conversations window's dot too (ui/presence.rs)
    let presence = match super::presence::of_avatar(world, &id) {
        Status::Busy => Presence::Busy,
        Status::Away => Presence::Away,
        _ => Presence::Online,
    };
    let names = &world.social.avatar_names;
    let (name, username, legacy) = match names.get(&id) {
        Some(n) => {
            let (main, user) = n.tag_lines(&names.options);
            (main, user, n.user_name_for_display(&names.options))
        }
        None => {
            let legacy = o.display_name().unwrap_or_else(|| aurora_i18n::tr!("status-loading-2").into());
            (legacy.clone(), None, legacy)
        }
    };
    let blocked = !own && world.is_avatar_blocked(&id);
    let friend = !own && world.social.is_friend(&id);
    // the same rule as the classic tag (FSTagShowARW and its two options)
    let detail = complexity.and_then(|tc| {
        let shown = (own && tc.own) || (!own && (!tc.only_too_complex || tc.too_complex.contains(&idx)));
        let c = *tc.values.get(&id).filter(|_| shown)?;
        let figure = group_digits(c);
        let mut full: String = aurora_i18n::tr!("common-complexity", value0 = figure.clone());
        let mut short = figure;
        let area = tc.areas.get(&id).copied().unwrap_or(0.0);
        let over_area = tc.max > 0 && tc.max_area > 0.0 && area > tc.max_area;
        if over_area {
            let area: String = aurora_i18n::tr!("ui-hud-area-m2", value0 = group_digits(area as u32));
            full = format!("{full} · {area}");
            short = format!("{short} · {area}");
        }
        Some(Detail {
            full,
            short,
            alert: !own && tc.max > 0 && (c > tc.max || over_area),
        })
    });
    Content {
        name,
        username,
        title: o.group_title(),
        detail,
        role: Role::of(own, blocked, super::colors::is_linden(&legacy), friend),
        presence,
        in_voice,
        // FSShowTypingStateInNameTag: never on our own tag
        typing: !own && playing(crate::world::radar::ANIM_AGENT_TYPE),
        friend,
        too_complex: !own && too_complex,
        blocked,
    }
}

/// `voice`: avatars in voice chat (level, speaking); absence means no voice.
/// `plates`: the avatars get name plates instead of the classic tags.
/// Each name tag and floating text is painted after the mark of where it
/// stands in the world (`aurora_render::world_label_at`): the scene hides
/// what is behind it, as in Firestorm.
/// Returns the avatar name tags, far first as drawn (the last one on top).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    p: &Palette,
    icons: &super::icons::Icons,
    world: &World,
    scene: &Scene,
    proj: &Projector,
    eye: Vec3,
    show_own: bool,
    tag_distance: f32,
    voice: &HashMap<uuid::Uuid, (f32, bool)>,
    voice_tags: &mut super::voice_tag::VoiceTags,
    complexity: Option<TagComplexity<'_>>,
    loading: Option<&HashMap<uuid::Uuid, f32>>,
    plates: Option<PlateFrame<'_>>,
) -> Vec<NameTag> {
    let painter = world_painter(ctx);
    let now = Instant::now();
    // (distance, anchor, lines, voice: (level, speaking), loading, avatar,
    // where it stands in the world)
    #[allow(clippy::type_complexity)]
    let mut tags: Vec<(
        f32,
        Pos2,
        Vec<(String, Color32, f32)>,
        Option<(f32, bool)>,
        Option<f32>,
        Option<uuid::Uuid>,
        Vec3,
    )> = Vec::new();
    let mut name_tags = Vec::new();
    let (plates, plate_frame) = match plates {
        Some(f) => (Some(f.plates), Some((f.pictures, f.too_complex, f.demo_too_complex))),
        None => (None, None),
    };
    let mut plate_tags: Vec<super::nameplate::Tag<'_>> = Vec::new();
    // only the avatars and the objects with a text: a shop has thousands of
    // objects for a few dozen of these
    for (idx, o) in world.objects.avatars().chain(world.objects.texted()) {
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
            let head = scene.name_tag_anchor(&o.full_id, pos);
            let Some(sp) = proj.project_name_tag(pos, head) else {
                continue;
            };
            let at = tag_point(head, eye);
            if let Some((pictures, too_complex, demo_too_complex)) = plate_frame {
                use super::nameplate::Picture;
                let id = o.full_id;
                let in_voice = voice.get(&id).copied();
                let too = too_complex.contains(&idx) || demo_too_complex == Some(id);
                // a picture is asked for until the profile says there is none
                let picture = match pictures.get(&id) {
                    Some(texture) => Picture::Loaded(texture),
                    None if world.profiles.get(&id).is_some_and(|pr| pr.sl_image.is_nil()) => Picture::None,
                    None => Picture::Wanted,
                };
                plate_tags.push(super::nameplate::Tag {
                    id,
                    anchor: sp,
                    at,
                    distance: d,
                    // the distance fade of the classic tags
                    fade: (1.0 - (d - 40.0) / 24.0).clamp(0.35, 1.0),
                    content: plate_content(world, idx, o, in_voice.is_some(), complexity.as_ref(), too),
                    voice: in_voice,
                    loading: loading.and_then(|l| l.get(&id).copied()),
                    picture,
                });
                continue;
            }
            let mut lines = Vec::new();
            // status line first, as LLVOAvatar::idleUpdateNameTagText: "(Away, Blocked)"
            {
                use crate::world::status::{ANIM_AWAY, ANIM_DO_NOT_DISTURB};
                let anims = world.animations_of(&o.full_id);
                let playing = |id: uuid::Uuid| anims.iter().any(|a| a.id == id);
                let mut status = Vec::new();
                if playing(ANIM_AWAY) {
                    status.push(aurora_i18n::tr!("common-away"));
                }
                if playing(ANIM_DO_NOT_DISTURB) {
                    status.push(aurora_i18n::tr!("common-do-not-disturb"));
                }
                if world.is_avatar_blocked(&o.full_id) {
                    status.push(aurora_i18n::tr!("ui-hud-blocked"));
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
                    let legacy = o.display_name().unwrap_or_else(|| aurora_i18n::tr!("status-loading-2").into());
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
                        // theme-guard: content — Firestorm's complexity ramp (complexity_color: red and green levels)
                        Color32::from_rgb((red * 255.0) as u8, (green * 255.0) as u8, 0)
                    };
                    let col = if tc.max == 0 || own {
                        // theme-guard: content — Firestorm's complexity without a limit (LLColor4::grey1)
                        Color32::from_gray(190)
                    } else {
                        ratio_color(c as f32, tc.max as f32)
                    };
                    lines.push((
                        aurora_i18n::tr!("common-complexity", value0 = group_digits(c).to_string()),
                        col,
                        11.0,
                    ));
                    let area = tc.areas.get(&o.full_id).copied().unwrap_or(0.0);
                    if tc.max > 0 && tc.max_area > 0.0 && area > tc.max_area {
                        lines.push((
                            aurora_i18n::tr!("ui-hud-area-m2", value0 = group_digits(area as u32).to_string()),
                            ratio_color(area, tc.max_area),
                            11.0,
                        ));
                    }
                }
            }
            let in_voice = voice.get(&o.full_id).copied();
            let progress = loading.and_then(|l| l.get(&o.full_id).copied());
            tags.push((d, sp, lines, in_voice, progress, Some(o.full_id), at));
        } else {
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
            tags.push((d + 1000.0, sp, lines, None, None, None, anchor));
        }
    }
    let now_s = ctx.input(|i| i.time);
    // the classic tag is a small card of the theme
    let tag_radius = ctx.global_style().visuals.window_corner_radius;
    // far first so near tags draw on top
    tags.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (d, sp, lines, in_voice, progress, avatar, at) in tags {
        // the scene hides what it has in front of this point
        aurora_render::world_label_at(ctx, Some(at));
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
        let no_voice = is_avatar && in_voice.is_none();
        let voice_w = if no_voice { super::voice_tag::ICON_WIDTH } else { 0.0 };
        let text_x = sp.x + voice_w * 0.5;
        if is_avatar {
            let w = max_w + 12.0 + super::voice_tag::TAG_EXTRA_WIDTH;
            let r = egui::Rect::from_min_size(Pos2::new(sp.x - w * 0.5, y - 3.0), egui::vec2(w, total_h + 6.0));
            if let Some(id) = avatar {
                name_tags.push(NameTag { id, rect: r });
            }
            painter.rect_filled(
                r,
                tag_radius,
                Color32::from_rgba_unmultiplied(p.bar.r(), p.bar.g(), p.bar.b(), (190.0 * fade) as u8),
            );
            // still loading: thin progress bar along the bottom of the tag
            // (violet -> turquoise, like the loading screens)
            if let Some(f) = progress {
                let track = egui::Rect::from_min_max(
                    Pos2::new(r.left() + 4.0, r.bottom() - 3.0),
                    Pos2::new(r.right() - 4.0, r.bottom() - 1.0),
                );
                painter.rect_filled(track, super::widgets::TRACK_RADIUS, p.field.gamma_multiply(fade));
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
            if let Some(id) = avatar {
                voice_tags.paint(&painter, p, id, r, in_voice, now_s, fade);
            }
            if no_voice && let Some(icon) = icons.get("microphone-off") {
                let center = Pos2::new(r.left() + 6.0 + super::voice_tag::ICON_SIZE * 0.5, r.center().y);
                let icon_rect = egui::Rect::from_center_size(center, egui::Vec2::splat(super::voice_tag::ICON_SIZE));
                painter.image(
                    icon.id(),
                    icon_rect,
                    egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    p.danger.gamma_multiply(fade),
                );
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
                painter.galley(rect.min + egui::vec2(1.0, 1.0), shadow, Color32::PLACEHOLDER);
            }
            // every line was laid out with its own color: nothing to fall back on
            painter.galley(rect.min, g, Color32::PLACEHOLDER);
            y += h;
        }
    }
    if let Some(plates) = plates {
        plate_tags.sort_by(|a, b| b.distance.total_cmp(&a.distance));
        plates.paint(&painter, p, icons, &plate_tags, now_s, |id, rect| {
            name_tags.push(NameTag { id, rect })
        });
    }
    // what else is painted for the world (the tracking beacon) stands nowhere
    aurora_render::world_label_at(ctx, None);
    voice_tags.prune(now_s);
    name_tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_tags_stay_centred_on_the_root_from_both_sides_and_at_any_height() {
        let root = Vec3::new(0.0, 0.8, 1.0);
        let mut old_offset: f32 = 0.0;
        for side in [-1.0, 1.0] {
            for distance in [3.0, 6.0, 12.0] {
                let eye = Vec3::new(side * distance, 0.0, 2.2);
                let view = glam::camera::rh::view::look_to_mat4(eye, Vec3::new(0.0, 0.0, 1.2) - eye, Vec3::Z);
                let projection = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1);
                for ppp in [1.0, 1.5, 2.0] {
                    let proj = Projector {
                        view_proj: projection * view,
                        width: 1280.0,
                        height: 800.0,
                        ppp,
                    };
                    let root_screen = proj.project(root).expect("visible root").0;
                    for height in [0.5, 1.05, 1.6] {
                        let anchor = root + Vec3::Z * height;
                        let elevated = proj.project(anchor).expect("visible height").0;
                        let tag = proj.project_name_tag(root, anchor).expect("visible tag");
                        assert!((tag.x - root_screen.x).abs() < 1e-4);
                        assert_eq!(tag.y, elevated.y);
                        old_offset = old_offset.max((elevated.x - root_screen.x).abs());
                    }
                }
            }
        }
        assert!(old_offset > 10.0, "the old projection must reproduce a visible horizontal drift");
    }

    #[test]
    fn name_tag_visibility_uses_the_combined_screen_position() {
        let proj = Projector {
            view_proj: Mat4::IDENTITY,
            width: 1280.0,
            height: 800.0,
            ppp: 1.5,
        };
        // The root may be below the view while the label above it is visible.
        let root = Vec3::new(0.5, -2.0, 0.0);
        let anchor = Vec3::new(2.0, 0.5, 0.0);
        assert!(proj.project(root).is_none());
        assert!(proj.project(anchor).is_none());
        assert!(proj.project_name_tag(root, anchor).is_some());
        assert!(proj.project_name_tag(Vec3::new(2.0, 0.0, 0.0), anchor).is_none());
        assert!(proj.project_name_tag(root, Vec3::new(0.0, 2.0, 0.0)).is_none());

        let perspective = Projector {
            view_proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 1.6, 0.1),
            ..proj
        };
        assert!(perspective.project_name_tag(Vec3::Z, -Vec3::Z).is_none());
        assert!(perspective.project_name_tag(-Vec3::Z, Vec3::Z).is_none());
    }

    #[test]
    fn a_name_tag_stands_in_front_of_its_anchor_but_never_past_the_camera() {
        let eye = Vec3::new(0.0, 0.0, 2.0);
        let head = Vec3::new(10.0, 0.0, 2.0);
        assert!(tag_point(head, eye).distance(Vec3::new(10.0 - TAG_PULL, 0.0, 2.0)) < 1e-5);
        // the camera against the avatar: half way at most
        let near = Vec3::new(0.4, 0.0, 2.0);
        assert!(tag_point(near, eye).distance(Vec3::new(0.2, 0.0, 2.0)) < 1e-5);
        assert_eq!(tag_point(eye, eye), eye);
        // on the line of sight: the tag stays where it is on screen
        let up = Vec3::new(6.0, 3.0, 9.0);
        let pulled = tag_point(up, eye);
        assert!((pulled - eye).normalize().distance((up - eye).normalize()) < 1e-5);
    }
}
