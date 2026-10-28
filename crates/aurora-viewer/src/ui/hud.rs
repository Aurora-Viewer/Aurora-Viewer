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
    /// Construct inside the UI pass: egui applies pending zoom at its start.
    /// Sampling ppp before run_ui projects with the previous frame's zoom,
    /// then painting with the new zoom shifts every world-space label.
    pub fn for_ui(ctx: &egui::Context, view_proj: Mat4, size: [f32; 2]) -> Self {
        Self {
            view_proj,
            width: size[0],
            height: size[1],
            ppp: ctx.pixels_per_point(),
        }
    }

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
#[derive(Clone, Copy)]
pub struct NameTag {
    pub id: uuid::Uuid,
    pub rect: egui::Rect,
    /// The plane used by labels.wgsl: constant camera depth across the tag,
    /// rather than a sphere centred on its avatar's current position.
    pub at: Vec3,
    pub normal: Vec3,
}

impl NameTag {
    pub fn new(id: uuid::Uuid, rect: egui::Rect, at: Vec3, proj: &Projector) -> Self {
        Self {
            id,
            rect,
            at,
            normal: proj.view_proj.row(3).truncate().normalize_or_zero(),
        }
    }

    pub fn point(&self, (origin, direction): (Vec3, Vec3)) -> Option<Vec3> {
        let denominator = self.normal.dot(direction);
        if denominator.abs() < 1e-6 {
            return None;
        }
        let t = self.normal.dot(self.at - origin) / denominator;
        (t.is_finite() && t >= 0.0).then_some(origin + direction * t)
    }
}

/// Port of LLVOAvatar::idleUpdateNameTag/updateCharacter
/// (indra/newview/llvoavatar.cpp, originally LGPL 2.1): ten seconds by
/// default, fading over the final second, and a fresh interval after the
/// avatar leaves the view. Both tag styles share the clock and hit rules.
#[derive(Default)]
pub struct NameTagVisibility {
    visible: HashMap<uuid::Uuid, (f64, bool)>,
    config: Option<(uuid::Uuid, bool, bool, bool, bool, u32)>,
}

impl NameTagVisibility {
    pub fn clear(&mut self) {
        self.visible.clear();
    }

    fn begin(&mut self, settings: &crate::settings::Settings, agent: uuid::Uuid) {
        let config = (
            agent,
            settings.show_own_name_tag,
            settings.show_friend_name_tags,
            settings.show_other_name_tags,
            settings.temporary_name_tags,
            settings.name_tag_show_seconds.to_bits(),
        );
        if self.config != Some(config) {
            self.clear();
            self.config = Some(config);
        }
        for (_, seen) in self.visible.values_mut() {
            *seen = false;
        }
    }

    fn alpha(&mut self, id: uuid::Uuid, now: f64, settings: &crate::settings::Settings) -> f32 {
        if !settings.temporary_name_tags {
            return 1.0;
        }
        let (since, seen) = self.visible.entry(id).or_insert((now, true));
        *seen = true;
        let remaining = f64::from(settings.name_tag_show_seconds) - (now - *since).max(0.0);
        remaining.clamp(0.0, 1.0) as f32
    }

    fn finish(&mut self) {
        self.visible.retain(|_, (_, seen)| *seen);
    }
}

fn name_tag_enabled(settings: &crate::settings::Settings, own: bool, friend: bool) -> bool {
    if own {
        settings.show_own_name_tag
    } else if friend {
        settings.show_friend_name_tags
    } else {
        settings.show_other_name_tags
    }
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
        if !world.rlva.hover_visible(o.full_id, true) {
            continue;
        }
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
        let color = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
        let text = world.rlva_hover_privacy.apply(&o.text).into_owned();
        let galley = super::bidi::no_wrap(ctx, text, egui::FontId::proportional(12.0 * scale), color);
        let rect = egui::Align2::CENTER_BOTTOM.anchor_size(point, galley.size());
        painter.galley(rect.min, galley, color);
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
    /// We type in the nearby chat (`ChatUi::local_typing`).
    pub own_typing: bool,
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
    settings: &crate::settings::Settings,
    own_typing: bool,
) -> super::nameplate::Content {
    use super::nameplate::{Content, Detail, Presence, Role};
    use super::presence::Status;
    let id = o.full_id;
    let own = id == world.agent_id;
    // the source of the Conversations window's dot too (ui/presence.rs)
    // our own plate says "Invisible" first, as the top bar's dot does:
    // nothing in the world tells it, the residents around still see us
    let presence = match super::presence::of_avatar(world, &id) {
        _ if own && world.status.invisible.on => Presence::Invisible,
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
        appearance: world.contact_sets.appearance(&id).clone(),
        name,
        username,
        title: o.group_title().filter(|_| world.group_titles.shows(own)),
        detail,
        role: Role::of(own, blocked, super::colors::is_linden(&legacy), friend),
        presence,
        in_voice,
        // FSShowTypingStateInNameTag. Our own plate too, from our typing
        // rather than the animation (it may be off), like its microphone:
        // the Legacy Viewer's tag never shows ours (the human's choice,
        // 2026-10-19)
        typing: settings.typing_in_name_tags && if own { own_typing } else { world.is_typing_nearby(&id) },
        friend,
        too_complex: !own && too_complex,
        // LLVOAvatar::getVisualMuteSettings (indra/newview/llvoavatar.cpp)
        // distinguishes an explicit rendering choice from an automatic limit.
        render_never: !own && too_complex && settings.render_exception(&id) == 1,
        blocked,
        mark: crate::world::staff::mark_named(&id, &legacy),
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
    avatar_tags: bool,
    settings: &crate::settings::Settings,
    visibility: &mut NameTagVisibility,
    voice: &HashMap<uuid::Uuid, (f32, bool)>,
    audio: &crate::settings::AudioSettings,
    voice_tags: &mut super::voice_tag::VoiceTags,
    complexity: Option<TagComplexity<'_>>,
    loading: Option<&HashMap<uuid::Uuid, f32>>,
    plates: Option<PlateFrame<'_>>,
    bubbles: Option<super::chat_bubbles::Frame<'_>>,
) -> Vec<NameTag> {
    let painter = world_painter(ctx);
    let now = Instant::now();
    let now_s = ctx.input(|i| i.time);
    visibility.begin(settings, world.agent_id);
    // (distance, anchor, lines, voice: (level, speaking), loading, avatar,
    // where it stands in the world, temporary fade, chat bubble)
    #[allow(clippy::type_complexity)]
    let mut tags: Vec<(
        f32,
        Pos2,
        Vec<(String, Color32, f32)>,
        Option<(f32, bool)>,
        Option<f32>,
        Option<uuid::Uuid>,
        Vec3,
        f32,
        Option<super::chat_bubbles::Bubble>,
    )> = Vec::new();
    // the bubbles under the plates, painted right after their plate
    let mut plate_bubbles: HashMap<uuid::Uuid, super::chat_bubbles::Bubble> = HashMap::new();
    let mut name_tags = Vec::new();
    let (plates, plate_frame) = match plates {
        Some(f) => (Some(f.plates), Some((f.pictures, f.too_complex, f.demo_too_complex, f.own_typing))),
        None => (None, None),
    };
    let mut plate_tags: Vec<super::nameplate::Tag<'_>> = Vec::new();
    // only the avatars and the objects with a text: a shop has thousands of
    // objects for a few dozen of these
    for (idx, o) in world.objects.avatars().chain(world.objects.texted()) {
        if !world.parcel_object_visible(idx) || !crate::snapshot::isolate::object_visible(world, idx) {
            continue;
        }
        if !o.is_avatar() && !world.rlva.hover_visible(o.full_id, scene.gpu.get(idx).is_some_and(|g| g.hud)) {
            continue;
        }
        if o.is_avatar() {
            // immersive mode without name tags: no plate, no bubble under it
            if !avatar_tags {
                continue;
            }
            if world.rlva.has("shownearby") && o.full_id != world.agent_id {
                continue;
            }
            let own = o.full_id == world.agent_id;
            if own && !show_own {
                continue;
            }
            let Some((pos, _, _)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let d = pos.distance(eye);
            if !world
                .rlva
                .nametag_visible(o.full_id, world.agent_id, pos.distance(world.agent.position))
            {
                continue;
            }
            // a resident with a bubble shows it under their name whatever
            // the name tags' settings, their distance and their timer
            // (LLVOAvatar::idleUpdateNameTag: visible_chat)
            let chatting = bubbles.as_ref().is_some_and(|b| super::chat_bubbles::shows(world, b, o.full_id, d));
            if !chatting && (!name_tag_enabled(settings, own, world.social.is_friend(&o.full_id)) || d > settings.name_tag_distance) {
                continue;
            }
            let head = scene.name_tag_anchor(&o.full_id, pos);
            if !scene.portal_keeps_point(idx, head) {
                continue;
            }
            let Some(sp) = proj.project_name_tag(pos, head) else {
                continue;
            };
            let alpha = visibility.alpha(o.full_id, now_s, settings);
            let alpha = if chatting { 1.0 } else { alpha };
            if alpha <= 0.0 {
                continue;
            }
            let bubble = bubbles
                .as_ref()
                .filter(|_| chatting)
                .and_then(|b| super::chat_bubbles::layout(&painter, p, world, b, o.full_id, d));
            // the name tag rises above its bubble
            let lift = bubble.as_ref().map_or(0.0, |b| b.size.y + super::chat_bubbles::GAP);
            let at = tag_point(head, eye);
            if let Some((pictures, too_complex, demo_too_complex, own_typing)) = plate_frame {
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
                if let Some(bubble) = bubble {
                    plate_bubbles.insert(id, bubble);
                }
                plate_tags.push(super::nameplate::Tag {
                    id,
                    anchor: sp - egui::vec2(0.0, lift),
                    at,
                    distance: d,
                    // the distance fade of the classic tags
                    fade: (1.0 - (d - 40.0) / 24.0).clamp(0.35, 1.0) * alpha,
                    content: plate_content(
                        world,
                        idx,
                        o,
                        in_voice.is_some()
                            && audio.voice_visualizer_dot
                            && audio.world_voice_visualizer(id == world.agent_id, world.calls.active.is_some(), false),
                        complexity.as_ref(),
                        too,
                        settings,
                        own_typing,
                    ),
                    voice: in_voice.filter(|_| audio.world_voice_visualizer(id == world.agent_id, world.calls.active.is_some(), false)),
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
                // FSShowTypingStateInNameTag, last, never on our own tag
                if settings.typing_in_name_tags && o.full_id != world.agent_id && world.is_typing_nearby(&o.full_id) {
                    status.push(aurora_i18n::tr!("radar-col-typing"));
                }
                if !status.is_empty() {
                    lines.push((format!("({})", status.join(", ")), p.muted, 11.0));
                }
            }
            if let Some(t) = o.group_title().filter(|_| world.group_titles.shows(o.full_id == world.agent_id)) {
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
                        // the red–green ramp, daltonised for a colour blindness
                        crate::colorblind::adapt(ratio_color(c as f32, tc.max as f32), crate::colorblind::current())
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
            tags.push((
                d,
                sp - egui::vec2(0.0, lift),
                lines,
                in_voice,
                progress,
                Some(o.full_id),
                at,
                alpha,
                bubble,
            ));
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
            let text = world.rlva_hover_privacy.apply(&o.text);
            let lines = text.lines().map(|l| (l.to_owned(), col, 12.0)).collect();
            tags.push((d + 1000.0, sp, lines, None, None, None, anchor, 1.0, None));
        }
    }
    visibility.finish();
    // the classic tag is a small card of the theme
    let tag_radius = ctx.global_style().visuals.window_corner_radius;
    // far first so near tags draw on top
    tags.sort_by(|a, b| b.0.total_cmp(&a.0));
    let opacity = bubbles.as_ref().map_or(0.0, |b| b.opacity);
    for (d, sp, lines, in_voice, progress, avatar, at, alpha, bubble) in tags {
        // the scene hides what it has in front of this point
        aurora_render::world_label_at(ctx, Some(at));
        let is_avatar = d < 1000.0;
        let fade = if is_avatar {
            (1.0 - (d - 40.0) / 24.0).clamp(0.35, 1.0) * alpha
        } else {
            1.0
        };
        let mut y = sp.y;
        // measure
        let galleys: Vec<_> = lines
            .iter()
            .map(|(t, c, s)| super::bidi::no_wrap(painter.ctx(), t.clone(), FontId::proportional(*s), c.gamma_multiply(fade)))
            .collect();
        let total_h: f32 = galleys.iter().map(|g| g.size().y).sum();
        let max_w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
        y -= total_h;
        let visualizer = audio.world_voice_visualizer(avatar == Some(world.agent_id), world.calls.active.is_some(), false);
        let no_voice = visualizer && is_avatar && in_voice.is_none();
        let voice_w = if no_voice { super::voice_tag::ICON_WIDTH } else { 0.0 };
        let text_x = sp.x + voice_w * 0.5;
        if is_avatar {
            let w = max_w + 12.0 + super::voice_tag::TAG_EXTRA_WIDTH;
            let r = egui::Rect::from_min_size(Pos2::new(sp.x - w * 0.5, y - 3.0), egui::vec2(w, total_h + 6.0));
            if let Some(id) = avatar {
                name_tags.push(NameTag::new(id, r, at, proj));
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
            if let Some(id) = avatar.filter(|_| visualizer) {
                voice_tags.paint(&painter, p, id, r, in_voice, now_s, fade);
                if audio.voice_visualizer_dot && in_voice.is_some() {
                    painter.circle_filled(Pos2::new(r.left() + 12.0, r.center().y), 3.0, p.success.gamma_multiply(fade));
                }
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
            if let Some(b) = &bubble {
                let top = Pos2::new(sp.x, r.bottom() + super::chat_bubbles::GAP);
                super::chat_bubbles::paint(&painter, p, b, top, fade, opacity, false, now_s);
            }
        }
        for g in galleys {
            let h = g.size().y;
            let pos = Pos2::new(text_x, y);
            let rect = Align2::CENTER_TOP.anchor_size(pos, g.size());
            if !is_avatar {
                // drop shadow (LLFontGL::DROP_SHADOW)
                let a = g.job.sections.first().map(|s| s.format.color.a()).unwrap_or(255);
                // the same galley in black: a right-to-left line is in
                // sections that a plain copy of its text would lose
                painter.galley_with_override_text_color(
                    rect.min + egui::vec2(1.0, 1.0),
                    g.clone(),
                    Color32::from_black_alpha((a as f32 * 0.7) as u8),
                );
            }
            // every line was laid out with its own color: nothing to fall back on
            painter.galley(rect.min, g, Color32::PLACEHOLDER);
            y += h;
        }
    }
    if let Some(plates) = plates {
        plate_tags.sort_by(|a, b| b.distance.total_cmp(&a.distance));
        let fades: HashMap<uuid::Uuid, f32> = if plate_bubbles.is_empty() {
            HashMap::new()
        } else {
            plate_tags.iter().map(|t| (t.id, t.fade)).collect()
        };
        plates.paint(&painter, p, icons, &plate_tags, now_s, |id, rect, at| {
            // right after its plate, before the nearer ones
            if let Some(b) = plate_bubbles.get(&id) {
                let top = Pos2::new(rect.center().x, rect.bottom() + super::chat_bubbles::GAP);
                let fade = fades.get(&id).copied().unwrap_or(1.0);
                super::chat_bubbles::paint(&painter, p, b, top, fade, opacity, true, now_s);
            }
            name_tags.push(NameTag::new(id, rect, at, proj))
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
    fn plate_render_badges_distinguish_manual_limits_and_follow_setting_changes() {
        use super::super::nameplate::{Badge, Level, badges};
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::tags::events()) {
            world.apply(event);
        }
        let (idx, avatar) = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id != world.agent_id)
            .expect("a demo avatar");
        let mut settings = crate::settings::Settings::default();
        for mode in [0, 1, 2, 1, 0] {
            settings.set_render_exception(avatar.full_id, mode);
            for silhouette in [false, true] {
                let content = plate_content(&world, idx, avatar, false, None, silhouette, &settings, false);
                let expected = if mode == 1 { Badge::RenderNever } else { Badge::TooComplex };
                for level in [Level::Full, Level::Compact] {
                    let shown: Vec<_> = badges(level, &content)
                        .iter()
                        .filter(|b| matches!(b, Badge::RenderNever | Badge::TooComplex))
                        .collect();
                    assert_eq!(shown, if silhouette { vec![expected] } else { vec![] });
                }
                assert_eq!(badges(Level::Far, &content).len(), 0);
            }
        }
        let (idx, avatar) = world
            .objects
            .avatars()
            .find(|(_, o)| o.full_id == world.agent_id)
            .expect("our demo avatar");
        settings.set_render_exception(avatar.full_id, 1);
        let own = plate_content(&world, idx, avatar, false, None, true, &settings, false);
        assert!(!own.too_complex && !own.render_never, "our own avatar has no rendering-limit badge");
    }

    #[test]
    fn name_tag_categories_are_independent_and_self_takes_priority() {
        for own in [false, true] {
            for friends in [false, true] {
                for others in [false, true] {
                    let settings = crate::settings::Settings {
                        show_own_name_tag: own,
                        show_friend_name_tags: friends,
                        show_other_name_tags: others,
                        ..Default::default()
                    };
                    assert_eq!(name_tag_enabled(&settings, true, false), own);
                    assert_eq!(name_tag_enabled(&settings, true, true), own);
                    assert_eq!(name_tag_enabled(&settings, false, true), friends);
                    assert_eq!(name_tag_enabled(&settings, false, false), others);
                }
            }
        }
    }

    #[test]
    fn temporary_name_tags_fade_once_and_restart_only_after_leaving_view() {
        let settings = crate::settings::Settings {
            temporary_name_tags: true,
            ..Default::default()
        };
        let agent = uuid::Uuid::from_u128(1);
        let avatar = uuid::Uuid::from_u128(2);
        let mut visibility = NameTagVisibility::default();
        for (now, expected) in [(100.0, 1.0), (109.0, 1.0), (109.5, 0.5), (110.0, 0.0), (130.0, 0.0)] {
            visibility.begin(&settings, agent);
            assert_eq!(visibility.alpha(avatar, now, &settings), expected);
            visibility.finish();
        }
        visibility.begin(&settings, agent);
        visibility.finish();
        assert!(visibility.visible.is_empty(), "off-screen and departed avatars are pruned");
        visibility.begin(&settings, agent);
        assert_eq!(visibility.alpha(avatar, 140.0, &settings), 1.0);
        visibility.finish();
    }

    #[test]
    fn temporary_name_tags_have_individual_clocks_and_live_setting_changes_restart_them() {
        let mut settings = crate::settings::Settings {
            temporary_name_tags: true,
            name_tag_show_seconds: 1.0,
            ..Default::default()
        };
        let agent = uuid::Uuid::from_u128(1);
        let a = uuid::Uuid::from_u128(2);
        let b = uuid::Uuid::from_u128(3);
        let mut visibility = NameTagVisibility::default();
        visibility.begin(&settings, agent);
        assert_eq!(visibility.alpha(a, 0.0, &settings), 1.0);
        visibility.finish();
        visibility.begin(&settings, agent);
        assert_eq!(visibility.alpha(a, 0.5, &settings), 0.5);
        assert_eq!(visibility.alpha(b, 0.5, &settings), 1.0);
        visibility.finish();
        settings.name_tag_show_seconds = 60.0;
        visibility.begin(&settings, agent);
        assert_eq!(visibility.alpha(a, 20.0, &settings), 1.0);
        visibility.finish();
        settings.temporary_name_tags = false;
        visibility.begin(&settings, agent);
        assert_eq!(visibility.alpha(a, 1000.0, &settings), 1.0);
        visibility.finish();
        assert!(visibility.visible.is_empty());
    }

    #[test]
    fn hidden_and_expired_name_tags_have_no_hit_rectangles_in_either_style() {
        use std::collections::HashSet;
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::tags::events()) {
            world.apply(event);
        }
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = super::super::icons::Icons::default();
        let proj = Projector {
            view_proj: Mat4::from_scale(Vec3::splat(1.0 / 512.0)),
            width: 1600.0,
            height: 900.0,
            ppp: 1.0,
        };
        let empty = HashMap::new();
        let pictures = HashMap::new();
        let too_complex = HashSet::new();
        for classic in [false, true] {
            let mut visibility = NameTagVisibility::default();
            let mut plates = super::super::nameplate::Plates::default();
            let mut voice_tags = super::super::voice_tag::VoiceTags::default();
            let mut frame = |settings: &crate::settings::Settings, time| {
                let mut hits = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        hits = draw(
                            ui.ctx(),
                            &p,
                            &icons,
                            &world,
                            &scene,
                            &proj,
                            Vec3::new(128.0, 128.0, 5.0),
                            true,
                            true,
                            settings,
                            &mut visibility,
                            &empty,
                            &settings.audio,
                            &mut voice_tags,
                            None,
                            None,
                            (!classic).then_some(PlateFrame {
                                plates: &mut plates,
                                pictures: &pictures,
                                too_complex: &too_complex,
                                demo_too_complex: None,
                                own_typing: false,
                            }),
                            None,
                        );
                    },
                );
                output.textures_delta.clear();
                hits.into_iter().map(|tag| tag.id).collect::<HashSet<_>>()
            };
            let mut settings = crate::settings::Settings {
                name_tag_distance: 128.0,
                ..Default::default()
            };
            let all = frame(&settings, 0.0);
            assert!(all.contains(&world.agent_id), "fixture includes our tag");
            assert!(
                all.iter().any(|id| *id != world.agent_id && world.social.is_friend(id)),
                "fixture includes a friend"
            );
            assert!(
                all.iter().any(|id| *id != world.agent_id && !world.social.is_friend(id)),
                "fixture includes another resident"
            );
            for (own, friends, others) in [(false, true, true), (true, false, true), (true, true, false), (false, false, false)] {
                settings.show_own_name_tag = own;
                settings.show_friend_name_tags = friends;
                settings.show_other_name_tags = others;
                let expected: HashSet<_> = all
                    .iter()
                    .copied()
                    .filter(|id| name_tag_enabled(&settings, *id == world.agent_id, world.social.is_friend(id)))
                    .collect();
                assert_eq!(frame(&settings, 1.0), expected, "classic={classic}");
            }
            settings.show_own_name_tag = true;
            settings.show_friend_name_tags = true;
            settings.show_other_name_tags = true;
            settings.temporary_name_tags = true;
            assert_eq!(frame(&settings, 2.0), all);
            assert!(
                frame(&settings, 12.0).is_empty(),
                "expired tags cannot receive clicks, classic={classic}"
            );
        }
    }

    /// LLVOAvatar::idleUpdateNameTag's `visible_chat`: a resident who
    /// speaks shows their tag, raised above the bubble, with the name tags
    /// off, expired or out of their distance; up to 45 m only.
    #[test]
    fn a_bubble_shows_its_tag_whatever_the_name_tag_settings_and_raises_it() {
        use crate::world::bubble_chat::{BubbleChat, Utterance, Voice};
        use std::collections::HashSet;
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::tags::events()) {
            world.apply(event);
        }
        let scene = Scene::new(std::path::PathBuf::new(), world.avatar_lib.clone());
        let ctx = egui::Context::default();
        let p = crate::theme::Theme::default().palette();
        let icons = super::super::icons::Icons::default();
        let proj = Projector {
            view_proj: Mat4::from_scale(Vec3::splat(1.0 / 512.0)),
            width: 1600.0,
            height: 900.0,
            ppp: 1.0,
        };
        let eye = Vec3::new(128.0, 128.0, 5.0);
        let empty = HashMap::new();
        let pictures = HashMap::new();
        let too_complex = HashSet::new();
        let now = Instant::now();
        let distance = |id: &uuid::Uuid| {
            let idx = world.objects.index_of_uuid(id).expect("avatar");
            Scene::object_transform(&world, idx, now, 0).expect("placed").0.distance(eye)
        };
        for classic in [false, true] {
            let mut visibility = NameTagVisibility::default();
            let mut plates = super::super::nameplate::Plates::default();
            let mut voice_tags = super::super::voice_tag::VoiceTags::default();
            let mut frame = |settings: &crate::settings::Settings, chat: Option<&BubbleChat>, time| {
                let mut hits = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        hits = draw(
                            ui.ctx(),
                            &p,
                            &icons,
                            &world,
                            &scene,
                            &proj,
                            eye,
                            true,
                            true,
                            settings,
                            &mut visibility,
                            &empty,
                            &settings.audio,
                            &mut voice_tags,
                            None,
                            None,
                            (!classic).then_some(PlateFrame {
                                plates: &mut plates,
                                pictures: &pictures,
                                too_complex: &too_complex,
                                demo_too_complex: None,
                                own_typing: false,
                            }),
                            chat.map(|chat| crate::ui::chat_bubbles::Frame {
                                chat,
                                typing: false,
                                own_typing: false,
                                opacity: 0.5,
                                now,
                            }),
                        );
                    },
                );
                output.textures_delta.clear();
                hits.into_iter().map(|tag| (tag.id, tag.rect)).collect::<HashMap<_, _>>()
            };
            let mut settings = crate::settings::Settings {
                name_tag_distance: 128.0,
                ..Default::default()
            };
            let all = frame(&settings, None, 0.0);
            let mut chat = BubbleChat::default();
            for id in all.keys() {
                chat.add(
                    *id,
                    Utterance {
                        text: "Hello".into(),
                        from: "Someone".into(),
                        voice: Voice::Normal,
                        said: now,
                    },
                );
            }
            let near: HashSet<_> = all.keys().copied().filter(|id| distance(id) < 45.0).collect();
            assert!(!near.is_empty() && near.len() < all.len(), "fixture: near and far residents");
            // the tags rise above their bubbles
            let raised = frame(&settings, Some(&chat), 1.0);
            for id in &near {
                assert!(raised[id].bottom() < all[id].bottom() - 10.0, "classic={classic}");
            }
            // the name tags off, then near enough only, then expired
            settings.show_own_name_tag = false;
            settings.show_friend_name_tags = false;
            settings.show_other_name_tags = false;
            assert!(frame(&settings, None, 2.0).is_empty());
            let shown: HashSet<_> = frame(&settings, Some(&chat), 3.0).into_keys().collect();
            assert_eq!(shown, near, "classic={classic}");
            settings = crate::settings::Settings {
                name_tag_distance: 1.0,
                temporary_name_tags: true,
                ..Default::default()
            };
            let shown: HashSet<_> = frame(&settings, Some(&chat), 30.0).into_keys().collect();
            assert_eq!(shown, near, "classic={classic}");
        }
    }

    #[test]
    fn pending_ui_zoom_keeps_world_text_and_avatar_anchors_at_the_same_physical_pixels() {
        let mut old_drift: f32 = 0.0;
        for native in [1.0, 1.5, 2.0] {
            let ctx = egui::Context::default();
            let size = [1920.0, 1080.0];
            // Separate avatar root/height, plus text off-center on both sides.
            let root = Vec3::new(0.4, -0.2, 0.0);
            let head = Vec3::new(0.7, 0.6, 0.0);
            for zoom in [1.0, 1.01, 1.384, 1.39, 1.0, 0.75, 2.0, 1.0] {
                let previous_ppp = ctx.pixels_per_point();
                ctx.set_zoom_factor(zoom);
                let mut raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(size[0], size[1]) / previous_ppp)),
                    ..Default::default()
                };
                raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
                let mut output = ctx.run_ui(raw, |ui| {
                    let proj = Projector::for_ui(ui.ctx(), Mat4::IDENTITY, size);
                    let ppp = ui.ctx().pixels_per_point();
                    assert!((ppp - native * zoom).abs() < 0.00001);
                    let physical = proj.project_name_tag(root, head).expect("visible avatar") * ppp;
                    assert!(physical.distance(Pos2::new(1344.0, 216.0)) < 0.001);
                    for (point, expected) in [
                        (Vec3::new(-0.6, 0.3, 0.0), Pos2::new(384.0, 378.0)),
                        (Vec3::new(0.8, -0.5, 0.0), Pos2::new(1728.0, 810.0)),
                    ] {
                        let physical = proj.project(point).expect("visible floating text").0 * ppp;
                        assert!(
                            physical.distance(expected) < 0.001,
                            "world text stays anchored at {native} DPI / {zoom} zoom"
                        );
                        let old = Projector { ppp: previous_ppp, ..proj };
                        old_drift = old_drift.max((old.project(point).expect("visible point").0 * ppp).distance(expected));
                    }
                });
                output.textures_delta.clear();
            }
        }
        assert!(old_drift > 10.0, "the pre-pass scale reproduces the reported jump");
    }

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
