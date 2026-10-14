//! Drawing of a name plate. What uses egui's own texture (the plate, its
//! lights, rings, discs and text) is grouped between the few shapes that
//! do not (the blur, the picture, the icons): each change of texture is a
//! draw of its own.

use super::mesh::{self, Border, Glow, Outline};
use super::{Badge, Badges, Content, Halo, Layout, Level, Metrics, Picture, Presence, Role, SCALE, SubLine, Tag, TextSize};
use crate::theme::Palette;
use egui::epaint::TextShape;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Galley, Mesh, Pos2, Rect, Shape, Vec2};
use std::sync::Arc;

/// Opacity of the plate's tint over the blurred world, and of the far pill's.
const TINT_GLASS: (f32, f32) = (0.52, 0.42);
/// The same without the blur behind: denser, so the text stays readable
/// over a busy scene.
const TINT_PLAIN: (f32, f32) = (0.74, 0.62);
/// Opacity of the light border, and of the far pill's.
const BORDER: (f32, f32) = (0.14, 0.10);
/// Brightness kept of the blurred world under the tint. The mockup's tint
/// alone turns light grey over a white wall or a bright sky, and the grey
/// lines of text disappear in it.
const GLASS_SHADE: f32 = 0.65;
/// A blocked resident's plate.
const BLOCKED_OPACITY: f32 = 0.62;
/// Opacity of a badge's disc. Denser than the mockup's 0.72, which has a
/// blur of its own behind it: the plate's border must not show through.
const BADGE_OPACITY: f32 = 0.9;

/// What every plate of a frame shares.
pub(super) struct Env<'a> {
    painter: &'a egui::Painter,
    p: &'a Palette,
    icons: &'a crate::ui::icons::Icons,
    now: f64,
    pixels_per_point: f32,
    semibold: egui::FontFamily,
    /// The plate's own color, opaque: halfway from the night blue of the
    /// brand to the bars' color.
    tint: Color32,
    /// Changes with the skin's colors: what was built with others is rebuilt.
    skin: u64,
}

impl<'a> Env<'a> {
    pub fn new(painter: &'a egui::Painter, p: &'a Palette, icons: &'a crate::ui::icons::Icons, now: f64) -> Self {
        Env {
            painter,
            p,
            icons,
            now,
            pixels_per_point: painter.pixels_per_point(),
            semibold: crate::ui::fonts::semibold(painter.ctx()),
            tint: p.navy.lerp_to_gamma(p.bar, 0.5),
            skin: {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                [
                    p.navy, p.bar, p.ink, p.success, p.amber, p.danger, p.raised, p.field, p.violet, p.teal,
                ]
                .hash(&mut hasher);
                hasher.finish()
            },
        }
    }
}

/// The geometry of a plate, which stays the same from frame to frame while
/// its avatar's state does not change: built around the origin, then moved
/// (and faded) to where the plate is.
#[derive(Default)]
pub(super) struct Cache {
    surface: Option<Built<SurfaceKey>>,
    marks: Option<Built<MarksKey>>,
}

struct Built<K> {
    key: K,
    /// Around the origin, at full opacity.
    mesh: Mesh,
    /// As last drawn (where, how opaque): a plate that has not moved is
    /// drawn again without a copy.
    shown: Option<(Vec2, f32, Arc<Mesh>)>,
}

impl<K: PartialEq> Built<K> {
    /// The geometry of `key` in `slot`, built again when the key changed.
    fn of(slot: &mut Option<Built<K>>, key: K, build: impl FnOnce(&K) -> Mesh) -> &mut Built<K> {
        if slot.as_ref().is_none_or(|built| built.key != key) {
            let mesh = build(&key);
            *slot = Some(Built { key, mesh, shown: None });
        }
        slot.as_mut().expect("the slot was just filled")
    }

    /// Draw it moved by `at`, at `alpha`.
    fn emit(&mut self, painter: &egui::Painter, at: Vec2, alpha: f32) {
        if self.mesh.is_empty() {
            return;
        }
        if let Some((_, _, mesh)) = self.shown.as_ref().filter(|(a, o, _)| *a == at && *o == alpha) {
            painter.add(Shape::Mesh(mesh.clone()));
            return;
        }
        let moved = |v: &egui::epaint::Vertex| egui::epaint::Vertex {
            pos: v.pos + at,
            uv: v.uv,
            color: if alpha >= 0.999 { v.color } else { v.color.gamma_multiply(alpha) },
        };
        let mesh = Arc::new(Mesh {
            indices: self.mesh.indices.clone(),
            vertices: self.mesh.vertices.iter().map(moved).collect(),
            texture_id: self.mesh.texture_id,
        });
        painter.add(Shape::Mesh(mesh.clone()));
        self.shown = Some((at, alpha, mesh));
    }
}

/// What the surface, its border and its lights are built from.
#[derive(Clone, Copy, PartialEq)]
struct SurfaceKey {
    size: Vec2,
    halo: Option<Halo>,
    glass: f32,
    far: f32,
    pixels_per_point: f32,
    skin: u64,
}

/// What the rings, discs and bars on a plate are built from.
#[derive(Clone, Copy, PartialEq)]
struct MarksKey {
    layout: Layout,
    scale: f32,
    placeholder: bool,
    dot: Option<Presence>,
    badges: usize,
    loading: Option<f32>,
    pixels_per_point: f32,
    skin: u64,
}

/// Where a plate is between two levels, and its lights.
pub(super) struct State {
    pub level: Level,
    /// The level it is leaving, `progress` (0 to 1) of the way.
    pub from: Option<Level>,
    pub progress: f32,
    pub halo: Option<Halo>,
    /// 1: over the blurred world; 0: tinted only.
    pub glass: f32,
}

/// A sub line: its text, and the complexity written after a username.
type Line = (Arc<Galley>, Option<Arc<Galley>>);

/// A plate at one level, laid out.
struct Face {
    metrics: Metrics,
    layout: Layout,
    name: Arc<Galley>,
    subs: [Option<Line>; 2],
    badges: Badges,
    dot: Option<Presence>,
}

fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t
}

fn line(env: &Env<'_>, text: &str, format: TextFormat, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.break_on_newline = false;
    // a long name ends with "…"; the full one is everywhere else
    job.wrap = TextWrapping::truncate_at_width(max_width);
    env.painter.layout_job(job)
}

fn face(env: &Env<'_>, level: Level, c: &Content) -> Face {
    let p = env.p;
    let m = Metrics::of(level, SCALE);
    let color = c.role.name_color(p, &crate::ui::colors::get());
    let mut format = match level {
        // the far pill's name is a regular weight
        Level::Far => TextFormat::simple(FontId::proportional(m.name_pt), color),
        _ => crate::ui::fonts::semibold_format(&env.semibold, m.name_pt, color),
    };
    format.italics = c.role == Role::Blocked;
    let name = line(env, &c.name, format, m.max_text);
    let small = |color: Color32| TextFormat::simple(FontId::proportional(m.sub_pt), color);
    let detail_color = |d: &super::Detail| if d.alert { p.amber } else { p.muted };
    let subs = super::sub_lines(level, c).map(|sub| {
        Some(match sub? {
            SubLine::Username(user, None) => (line(env, user, small(p.muted), m.max_text), None),
            SubLine::Username(user, Some(d)) => {
                // the figure is never the part that gets elided
                let tail = line(env, &format!(" · {}", d.short), small(detail_color(d)), m.max_text * 0.6);
                let room = m.max_text - tail.size().x;
                (line(env, user, small(p.muted), room), Some(tail))
            }
            SubLine::Title(title) => (line(env, title, small(p.teal), m.max_text), None),
            SubLine::Detail(d) => (line(env, &d.full, small(detail_color(d)), m.max_text), None),
        })
    });
    let size = |l: &Line| {
        let tail = l.1.as_ref().map_or(Vec2::ZERO, |t| t.size());
        Vec2::new(l.0.size().x + tail.x, l.0.size().y.max(tail.y))
    };
    let text = TextSize {
        name: name.size(),
        subs: [subs[0].as_ref().map(size), subs[1].as_ref().map(size)],
    };
    let badges = super::badges(level, c);
    Face {
        metrics: m,
        layout: super::layout(level, &m, &text, badges.len()),
        name,
        subs,
        badges,
        dot: super::status_dot(level, c),
    }
}

/// Draw the plate of `tag` and return its rectangle.
pub(super) fn plate(env: &Env<'_>, tag: &Tag<'_>, st: &State, cache: &mut Cache) -> Rect {
    let c = &tag.content;
    let to = face(env, st.level, c);
    let from = st.from.map(|level| (level, face(env, level, c)));
    let far = |level: Level| if level == Level::Far { 1.0 } else { 0.0 };
    let (size, farness) = match &from {
        Some((level, f)) => (
            f.layout.size + (to.layout.size - f.layout.size) * st.progress,
            far(*level) + (far(st.level) - far(*level)) * st.progress,
        ),
        None => (to.layout.size, far(st.level)),
    };
    // on whole pixels: the plate and its text stay sharp as the avatar moves
    let ppp = env.pixels_per_point;
    let min = Pos2::new(tag.anchor.x - size.x * 0.5, tag.anchor.y - size.y);
    let rect = Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), size);
    let alpha = tag.fade * if c.blocked { BLOCKED_OPACITY } else { 1.0 };
    background(env, rect, st, alpha, farness, &mut cache.surface);
    match from {
        None => contents(env, env.painter, rect, &to, tag, alpha, &mut cache.marks),
        Some((_, from)) => {
            // both levels cross-fade inside the plate as it changes size
            let clip = Rect::from_min_max(Pos2::new(rect.left(), rect.top() - 16.0 * SCALE), rect.max);
            let painter = env.painter.with_clip_rect(clip.intersect(env.painter.clip_rect()));
            contents(env, &painter, rect, &from, tag, alpha * (1.0 - st.progress), &mut None);
            contents(env, &painter, rect, &to, tag, alpha * st.progress, &mut cache.marks);
        }
    }
    rect
}

/// The blur of the world, then the surface, its border, its shadow and
/// the voice halo in one mesh.
fn background(env: &Env<'_>, rect: Rect, st: &State, alpha: f32, far: f32, cache: &mut Option<Built<SurfaceKey>>) {
    let p = env.p;
    let s = SCALE;
    if st.glass > 0.0 {
        // inside the antialiased edge of the surface, which hides the blur's own
        let radius = rect.height() * 0.5 - 0.5;
        env.painter
            .add(aurora_render::glass_region(rect.shrink(0.5), radius, st.glass * alpha, GLASS_SHADE));
    }
    let key = SurfaceKey {
        size: rect.size(),
        halo: st.halo,
        glass: st.glass,
        far,
        pixels_per_point: env.pixels_per_point,
        skin: env.skin,
    };
    let surface = Built::of(cache, key, |key| {
        let shadow = Glow {
            offset: Vec2::new(0.0, 6.0 * s),
            blur: 18.0 * s,
            spread: 0.0,
            color: p.navy.gamma_multiply(0.35),
        };
        let mut lights = [shadow; 2];
        let mut count = 1;
        if let Some(halo) = key.halo {
            lights[1] = Glow {
                offset: Vec2::ZERO,
                blur: halo.blur,
                spread: halo.spread,
                color: p.success.gamma_multiply(halo.alpha),
            };
            count = 2;
        }
        let opacity = lerp((lerp(TINT_PLAIN, far), lerp(TINT_GLASS, far)), st.glass);
        let plain = p.ink.gamma_multiply(lerp(BORDER, far));
        let border = match key.halo {
            // only while the avatar speaks (the human's choice: no outline
            // for a silent participant): the border and one more pixel
            // around it, turning green with the halo's gate
            Some(halo) => Border::Around(plain.lerp_to_gamma(p.success, halo.gate), s),
            None => Border::Inside(plain, 1.0),
        };
        let mut plate = Mesh::default();
        let at_origin = Rect::from_min_size(Pos2::ZERO, rect.size());
        let fill = env.tint.gamma_multiply(opacity);
        mesh::plate(&mut plate, at_origin, env.pixels_per_point, fill, border, &lights[..count]);
        plate
    });
    surface.emit(env.painter, rect.min.to_vec2(), alpha);
}

const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

/// Part of a picture that fills a square without stretching it.
pub(super) fn cover(size: [usize; 2]) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    if w >= h {
        let half = h / w * 0.5;
        Rect::from_min_max(Pos2::new(0.5 - half, 0.0), Pos2::new(0.5 + half, 1.0))
    } else {
        let half = w / h * 0.5;
        Rect::from_min_max(Pos2::new(0.0, 0.5 - half), Pos2::new(1.0, 0.5 + half))
    }
}

fn badge_icon(p: &Palette, badge: Badge) -> Option<(&'static str, Color32)> {
    Some(match badge {
        Badge::Voice => ("microphone-filled", p.success),
        Badge::Friend => ("heart-filled", p.success),
        Badge::TooComplex => ("alert-triangle-filled", p.amber),
        Badge::Blocked => ("forbid-filled", p.danger),
        Badge::Typing => return None,
    })
}

/// What lies on a plate besides its text, pictures and icons, around the
/// origin: the placeholder's disc, the picture's ring, the status dot, the
/// loading bar and the discs of the badges.
fn marks(env: &Env<'_>, key: &MarksKey) -> Mesh {
    let p = env.p;
    let (l, s, ppp) = (&key.layout, key.scale, key.pixels_per_point);
    let mut mesh = Mesh::default();
    if let Some(photo) = l.photo {
        let disc = Outline::circle(photo.center, photo.radius, ppp);
        if key.placeholder {
            disc.dot(&mut mesh, p.raised);
        }
        disc.stroke(&mut mesh, 2.0 * s, p.ink.gamma_multiply(0.22));
    }
    if let (Some(dot), Some(presence)) = (l.dot, key.dot) {
        let color = crate::ui::presence::color(p, presence.status());
        Outline::circle(dot.center, dot.radius, ppp).dot(&mut mesh, env.tint);
        Outline::circle(dot.center, dot.radius - 3.0 * s, ppp).dot(&mut mesh, color);
    }
    // still loading: a thin bar under the text, violet to turquoise like
    // the loading screens
    if let Some(progress) = key.loading {
        let y = l.size.y - 5.0 * s;
        let track = Rect::from_min_max(Pos2::new(l.text_x.0, y), Pos2::new(l.text_x.1, y + 2.0 * s));
        mesh.add_colored_rect(track, p.field);
        let end = track.left() + track.width() * progress.clamp(0.02, 1.0);
        let first = mesh.vertices.len() as u32;
        mesh.colored_vertex(track.left_top(), p.violet);
        mesh.colored_vertex(Pos2::new(end, track.top()), p.teal);
        mesh.colored_vertex(Pos2::new(end, track.bottom()), p.teal);
        mesh.colored_vertex(track.left_bottom(), p.violet);
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first, first + 2, first + 3);
    }
    let fill = env.tint.gamma_multiply(BADGE_OPACITY);
    let edge = mesh::over(p.ink.gamma_multiply(0.16), fill);
    for r in &l.badges[..key.badges] {
        Outline::circle(r.center(), r.width() * 0.5, ppp).disc(&mut mesh, fill, edge, 1.0);
    }
    mesh
}

/// Picture, status dot, lines, loading bar and badges of one level, placed
/// in `rect` (which is the level's own rectangle unless the plate is
/// changing level).
fn contents(
    env: &Env<'_>,
    painter: &egui::Painter,
    rect: Rect,
    face: &Face,
    tag: &Tag<'_>,
    alpha: f32,
    cache: &mut Option<Built<MarksKey>>,
) {
    if alpha <= 0.004 {
        return;
    }
    let p = env.p;
    let ppp = env.pixels_per_point;
    let (l, m) = (&face.layout, &face.metrics);
    let s = m.scale;
    // left aligned, centered in the height while the plate changes size
    let origin = Vec2::new(rect.left(), rect.center().y - l.size.y * 0.5);
    let ink = |color: Color32| color.gamma_multiply(alpha);

    let picture = match tag.picture {
        Picture::Loaded(texture) => Some(texture),
        Picture::Wanted | Picture::None => None,
    };
    if let (Some(photo), Some(texture)) = (l.photo, picture) {
        let mut mesh = Mesh::with_texture(texture.id());
        let tint = ink(crate::ui::widgets::PICTURE_TINT);
        Outline::circle(photo.center + origin, photo.radius, ppp).picture(&mut mesh, cover(texture.size()), tint);
        painter.add(Shape::mesh(mesh));
    }
    // the placeholder's disc, then the rings, discs and bars that lie over
    // the picture, in one mesh
    let key = MarksKey {
        layout: *l,
        scale: s,
        placeholder: picture.is_none(),
        dot: face.dot,
        badges: face.badges.len(),
        loading: tag.loading,
        pixels_per_point: ppp,
        skin: env.skin,
    };
    Built::of(cache, key, |key| marks(env, key)).emit(painter, origin, alpha);
    if let Some(photo) = l.photo.filter(|_| picture.is_none())
        && let Some(icon) = env.icons.get("user-filled")
    {
        let size = Vec2::splat(photo.radius * 2.0 * 30.0 / 54.0);
        painter.image(icon.id(), Rect::from_center_size(photo.center + origin, size), WHOLE, ink(p.muted));
    }
    let places = || face.badges.iter().zip(l.badges).map(|(b, r)| (b, r.translate(origin)));
    for (badge, r) in places() {
        if badge == Badge::Typing {
            let mut dots = Mesh::default();
            for i in 0..3 {
                let (opacity, lift) = super::typing_dot(env.now, i);
                let at = r.center() + Vec2::new((i as f32 - 1.0) * 5.5 * s, lift * s);
                Outline::circle(at, 1.75 * s, ppp).dot(&mut dots, ink(p.ink.gamma_multiply(opacity)));
            }
            painter.add(Shape::mesh(dots));
        }
    }

    let text = |galley: &Arc<Galley>, pos: Pos2| {
        let mut shape = TextShape::new(pos + origin, galley.clone(), p.ink);
        shape.opacity_factor = alpha;
        painter.add(shape);
    };
    text(&face.name, l.name);
    for (sub, pos) in face.subs.iter().zip(l.subs) {
        if let (Some((galley, tail)), Some(pos)) = (sub, pos) {
            text(galley, pos);
            if let Some(tail) = tail {
                text(tail, pos + Vec2::new(galley.size().x, 0.0));
            }
        }
    }

    // the badges' icons last: one texture each
    for (badge, r) in places() {
        if let Some((name, color)) = badge_icon(p, badge)
            && let Some(icon) = env.icons.get(name)
        {
            let size = Vec2::splat(m.badge_icon);
            painter.image(icon.id(), Rect::from_center_size(r.center(), size), WHOLE, ink(color));
        }
    }
}
