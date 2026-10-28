//! Drawing of a name plate. What uses egui's own texture (the plate, its
//! lights, rings, discs and text) is grouped between the few shapes that
//! do not (the blur, the picture, the icons): each change of texture is a
//! draw of its own.

use super::aurora;
use super::mark::Look;
use super::mesh::{self, Border, Glow, Outline, Ring};
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
/// Under the pointer (#473): how far the surface goes toward the ink, and
/// the opacity the border gains. Faint: the plate answers, it does not flash.
const HOVER_LIGHT: f32 = 0.12;
const HOVER_BORDER: f32 = 0.18;
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
    pub painter: &'a egui::Painter,
    pub p: &'a Palette,
    icons: &'a crate::ui::icons::Icons,
    pub now: f64,
    pub pixels_per_point: f32,
    pub semibold: egui::FontFamily,
    /// The plate's own color, opaque: halfway from the night blue of the
    /// brand to the bars' color.
    pub tint: Color32,
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

    /// The icon `name`, from the skin or the viewer's own.
    pub fn icon(&self, name: &str) -> Option<egui::TextureHandle> {
        self.icons.get(name).cloned().or_else(|| crate::ui::icons::global(name))
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
    tint: Color32,
    border: Option<Color32>,
    size: Vec2,
    halo: Option<Halo>,
    glass: f32,
    far: f32,
    hover: f32,
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
    /// How far its size has gone: `progress`, or past 1 and back for a
    /// jiggly plate that grows (`fidget::overshoot`).
    pub size_progress: f32,
    pub halo: Option<Halo>,
    /// 1: over the blurred world; 0: tinted only.
    pub glass: f32,
    /// 1 under the pointer: a lighter surface and border.
    pub hover: f32,
    /// Points the surface grows on each side, across and up, in its bounce
    /// (`bounce_grow`) or its wobble (`fidget::Wobble`).
    pub grow: Vec2,
    /// Preferences › "Reduce animations": a team member's light holds still.
    pub still: bool,
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
    /// A team member's "Aurora Viewer" tag, and the time of its light.
    staff: Option<(Arc<Galley>, Look, f32)>,
}

fn lerp(range: (f32, f32), t: f32) -> f32 {
    range.0 + (range.1 - range.0) * t
}

fn line(env: &Env<'_>, text: &str, format: TextFormat, max_width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), format);
    job.break_on_newline = false;
    // a long name ends with "…"; the full one is everywhere else
    job.wrap = TextWrapping::truncate_at_width(max_width);
    crate::ui::bidi::galley(env.painter.ctx(), job, crate::ui::bidi::Base::Auto)
}

fn face(env: &Env<'_>, level: Level, c: &Content, t: f32) -> Face {
    let p = env.p;
    let m = Metrics::of(level, SCALE);
    let color = c
        .appearance
        .nameplate_name
        .map(crate::ui::colors::set_color)
        .unwrap_or_else(|| c.role.name_color(p, &crate::ui::colors::get()));
    let color = c
        .appearance
        .background
        .map(|bg| readable_name(color, plate_tint(env, bg), p.ink))
        .unwrap_or(color);
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
    // the staff's tag, its letters in the mark's colors; not on the far pill
    let staff = c.mark.filter(|_| level != Level::Far).map(|mark| {
        let look = Look::new(mark);
        let label = look.label();
        let n = label.chars().count();
        let mut job = LayoutJob {
            break_on_newline: false,
            ..Default::default()
        };
        for (i, ch) in label.chars().enumerate() {
            let format = crate::ui::fonts::semibold_format(&env.semibold, aurora::TAG_TEXT * m.scale, look.letter(p, i, n, t));
            job.append(ch.encode_utf8(&mut [0; 4]), 0.0, format);
        }
        (env.painter.layout_job(job), look, t)
    });
    let lead = staff.as_ref().map_or(0.0, |(g, ..)| aurora::tag_width(g.size().x, m.scale));
    Face {
        metrics: m,
        layout: super::layout_with(level, &m, &text, badges.len(), lead),
        name,
        subs,
        badges,
        dot: super::status_dot(level, c),
        staff,
    }
}

fn luminance(c: Color32) -> f32 {
    let v = egui::Rgba::from(c).to_array();
    0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2]
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn plate_tint(env: &Env<'_>, color: [f32; 4]) -> Color32 {
    let color = crate::ui::colors::set_color(color).to_opaque();
    // Derive the hue from the set, while keeping the ordinary surface alpha.
    // Check both the main ink and the secondary line against the tint.
    let mut tint = color.lerp_to_gamma(env.tint, 0.65);
    let target = if contrast(env.p.ink, env.p.navy) >= contrast(env.p.ink, env.p.field) {
        env.p.navy
    } else {
        env.p.field
    };
    for _ in 0..12 {
        if contrast(env.p.ink, tint) >= 4.5 && contrast(env.p.muted, tint) >= 3.0 {
            break;
        }
        tint = tint.lerp_to_gamma(target, 0.2);
    }
    tint
}

fn readable_name(mut name: Color32, background: Color32, ink: Color32) -> Color32 {
    for _ in 0..12 {
        if contrast(name, background) >= 4.5 {
            break;
        }
        name = name.lerp_to_gamma(ink, 0.2);
    }
    name
}

/// Draw the plate of `tag` and return its rectangle, and its surface as
/// drawn (moved by its bounce).
pub(super) fn plate(env: &Env<'_>, tag: &Tag<'_>, st: &State, cache: &mut Cache) -> (Rect, Rect) {
    let c = &tag.content;
    let t = aurora::time(env.now, st.still);
    let to = face(env, st.level, c, t);
    let from = st.from.map(|level| (level, face(env, level, c, t)));
    let far = |level: Level| if level == Level::Far { 1.0 } else { 0.0 };
    let (size, farness) = match &from {
        Some((level, f)) => (
            f.layout.size + (to.layout.size - f.layout.size) * st.size_progress,
            far(*level) + (far(st.level) - far(*level)) * st.progress,
        ),
        None => (to.layout.size, far(st.level)),
    };
    // on whole pixels: the plate and its text stay sharp as the avatar moves
    let ppp = env.pixels_per_point;
    let min = Pos2::new(tag.anchor.x - size.x * 0.5, tag.anchor.y - size.y);
    let rect = Rect::from_min_size(Pos2::new((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), size);
    let alpha = tag.fade * if c.blocked { BLOCKED_OPACITY } else { 1.0 };
    // the bounce moves the surface alone, on whole pixels too; the plate
    // keeps its rectangle for the pointer, which does not chase the bounce
    let grow = (st.grow * ppp).round() / ppp;
    let surface = rect.expand2(grow);
    // a staff member's light rises from behind the plate, its border over it
    let look = c.mark.map(Look::new);
    let light = |build: &dyn Fn(&mut Mesh)| {
        let mut mesh = Mesh::default();
        build(&mut mesh);
        env.painter.add(Shape::mesh(mesh));
    };
    if let Some(look) = look {
        light(&|mesh| look.above(mesh, env.p, surface, ppp, t, alpha));
    }
    background(env, surface, st, alpha, farness, c, &mut cache.surface);
    if let Some(look) = look {
        let width = (2.0 * SCALE).max(1.0 / ppp);
        light(&|mesh| look.border(mesh, env.p, surface, ppp, t, alpha, width, SCALE));
    }
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
    (rect, surface)
}

/// The blur of the world, then the surface, its border, its shadow and
/// the voice halo in one mesh.
#[allow(clippy::too_many_arguments)]
fn background(env: &Env<'_>, rect: Rect, st: &State, alpha: f32, far: f32, c: &Content, cache: &mut Option<Built<SurfaceKey>>) {
    let p = env.p;
    let s = SCALE;
    if st.glass > 0.0 {
        // inside the antialiased edge of the surface, which hides the blur's own
        let radius = rect.height() * 0.5 - 0.5;
        env.painter
            .add(aurora_render::glass_region(rect.shrink(0.5), radius, st.glass * alpha, GLASS_SHADE));
    }
    let key = SurfaceKey {
        tint: c.appearance.background.map(|bg| plate_tint(env, bg)).unwrap_or(env.tint),
        border: c.appearance.border.map(crate::ui::colors::set_color),
        size: rect.size(),
        halo: st.halo,
        glass: st.glass,
        far,
        hover: st.hover,
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
        let opacity = lerp((lerp(TINT_PLAIN, far), lerp(TINT_GLASS, far)), st.glass);
        let plain = key
            .border
            .unwrap_or_else(|| p.ink.gamma_multiply(lerp(BORDER, far) + HOVER_BORDER * key.hover));
        let voice = key.halo.map(|halo| voice_light(p, halo, plain, key.size.y));
        let mut plate = Mesh::default();
        let at_origin = Rect::from_min_size(Pos2::ZERO, rect.size());
        let fill = key.tint.lerp_to_gamma(p.ink, HOVER_LIGHT * key.hover).gamma_multiply(opacity);
        let ppp = env.pixels_per_point;
        match voice {
            Some(v) => mesh::plate(&mut plate, at_origin, ppp, fill, v.border, &[shadow, v.glow], &[v.ring]),
            // no outline for a silent participant (the human's choice)
            None => mesh::plate(&mut plate, at_origin, ppp, fill, Border::Inside(plain, 1.0), &[shadow], &[]),
        }
        plate
    });
    surface.emit(env.painter, rect.min.to_vec2(), alpha);
}

/// What the voice adds to a plate, in points: a ring around it over a glow,
/// and the plate's own border taking the ring's color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct VoiceLight {
    pub glow: Glow,
    pub ring: Ring,
    pub border: Border,
}

/// The speaking ring for a smoothed level `level` and an overdrive `heat`
/// (both 0..1), in mockup pixels, before the gate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LoudRing {
    /// From the plate's edge to the ring, and its width.
    pub gap: f32,
    pub width: f32,
    /// Opacity of the ring, of its glow, and how far the plate's own edge
    /// takes the ring's color.
    pub alpha: f32,
    pub glow: f32,
    pub edge: f32,
    /// Blur of the glow around the ring.
    pub blur: f32,
    pub color: Color32,
}

/// Everything grows with the level, so that a whisper, a voice and a shout
/// look different at a glance: a whisper (just above the speaking level) is
/// a faint hairline hugging the plate with almost no glow, a normal voice a
/// ring a few points away over a soft glow, a shout a thick bright ring
/// pushed out with a wide glow. The glow grows with the square of the level:
/// it stays out of the way until the voice is loud. The color is the
/// microphone badge's green, a little brighter as the voice rises, sliding
/// to the danger red only past the overdriven level (`overdrive`), through
/// the amber: halfway between the green and the red directly is a muddy
/// olive.
pub(super) fn loud_ring(p: &Palette, level: f32, heat: f32) -> LoudRing {
    let (l, h) = (level.clamp(0.0, 1.0), heat.clamp(0.0, 1.0));
    let green = p.success.lerp_to_gamma(p.ink, 0.3 * l);
    let color = if h < 0.5 {
        green.lerp_to_gamma(p.amber, 2.0 * h)
    } else {
        p.amber.lerp_to_gamma(p.danger, 2.0 * h - 1.0)
    };
    LoudRing {
        gap: 1.0 + 7.0 * l,
        width: 0.9 + 2.7 * l,
        alpha: 0.4 + 0.6 * l,
        glow: 0.03 + 0.5 * l * l,
        edge: 0.25 + 0.35 * l,
        blur: 2.0 + 24.0 * l * l,
        color,
    }
}

/// The voice's light on a plate `height` points high, whose border is
/// `plain` when silent: the `loud_ring`, a little tighter around the
/// smaller plates, closing in on the plate as the voice starts (the gate).
pub(super) fn voice_light(p: &Palette, halo: Halo, plain: Color32, height: f32) -> VoiceLight {
    let g = halo.gate.clamp(0.0, 1.0);
    // mockup pixels to points
    let k = SCALE * (height / Metrics::of(Level::Full, SCALE).height).clamp(0.55, 1.0);
    let r = loud_ring(p, halo.level, halo.heat);
    let gap = r.gap + 4.0 * (1.0 - g);
    VoiceLight {
        glow: Glow {
            offset: Vec2::ZERO,
            blur: r.blur * k,
            spread: (gap + r.width) * k,
            color: r.color.gamma_multiply(g * r.glow),
        },
        ring: Ring {
            gap: gap * k,
            width: r.width * k,
            color: r.color.gamma_multiply(g * r.alpha),
        },
        // the border and one more pixel around it
        border: Border::Around(plain.lerp_to_gamma(r.color, g * r.edge), SCALE),
    }
}

pub(super) const WHOLE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

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
        Badge::Contact { icon, color } => (
            crate::world::contact_sets::appearance::ICONS[icon],
            Color32::from_rgba_premultiplied(color[0], color[1], color[2], color[3]),
        ),
        Badge::More(_) => return None,
        Badge::Voice => ("microphone-filled", p.success),
        Badge::Friend => ("heart-filled", p.success),
        Badge::TooComplex => ("alert-triangle-filled", p.amber),
        Badge::RenderNever => ("eye-off", p.amber),
        Badge::Blocked => ("ban", p.danger),
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
        use crate::ui::presence::Shape as Mark;
        let status = presence.status();
        let color = crate::ui::presence::color(p, status);
        Outline::circle(dot.center, dot.radius, ppp).dot(&mut mesh, env.tint);
        // the status's shape (ui/presence.rs), its cuts in the plate's tint
        let r = dot.radius - 3.0 * s;
        let mark = Outline::circle(dot.center, r, ppp);
        match status.shape() {
            Mark::Ring => mark.stroke(&mut mesh, r * Mark::RING, color),
            shape => {
                mark.dot(&mut mesh, color);
                match shape {
                    Mark::Crescent => {
                        Outline::circle(dot.center + Vec2::splat(r * Mark::CUT_OFFSET), r * Mark::CUT_RADIUS, ppp).dot(&mut mesh, env.tint)
                    }
                    Mark::Bar => Outline::pill(Rect::from_center_size(dot.center, Vec2::new(r * Mark::BAR.0, r * Mark::BAR.1)), ppp)
                        .dot(&mut mesh, env.tint),
                    _ => {}
                }
            }
        }
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
        && let Some(icon) = env
            .icons
            .get("user-filled")
            .cloned()
            .or_else(|| crate::ui::icons::global("user-filled"))
    {
        let size = Vec2::splat(photo.radius * 2.0 * 30.0 / 54.0);
        painter.image(icon.id(), Rect::from_center_size(photo.center + origin, size), WHOLE, ink(p.muted));
    }
    let places = || face.badges.iter().zip(l.badges).map(|(b, r)| (b, r.translate(origin)));
    for (badge, r) in places() {
        if let Badge::More(n) = badge {
            painter.text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("+{n}"),
                FontId::proportional(9.0),
                ink(p.ink),
            );
        }
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
            && let Some(icon) = env.icons.get(name).cloned().or_else(|| crate::ui::icons::global(name))
        {
            let size = Vec2::splat(m.badge_icon);
            painter.image(icon.id(), Rect::from_center_size(r.center(), size), WHOLE, ink(color));
        }
    }

    // the staff's tag over the top edge, from the name's left
    if let Some((label, look, t)) = &face.staff {
        let size = Vec2::new(aurora::tag_width(label.size().x, s), aurora::TAG_HEIGHT * s);
        let at = Pos2::new(origin.x + l.text_x.0 - 2.0 * s, rect.top() - aurora::TAG_RISE * s - size.y * 0.5);
        let pill = Rect::from_min_size(Pos2::new((at.x * ppp).round() / ppp, (at.y * ppp).round() / ppp), size);
        let mut mesh = Mesh::default();
        let fill = env.tint.gamma_multiply(BADGE_OPACITY);
        look.tag(&mut mesh, p, pill, pill.height() * 0.5, ppp, *t, fill, alpha, 1.2 * s);
        painter.add(Shape::mesh(mesh));
        let icon_at = Pos2::new(pill.left() + aurora::TAG_PAD * s + aurora::TAG_ICON * s * 0.5, pill.center().y);
        if let Some(icon) = env
            .icons
            .get(look.icon())
            .cloned()
            .or_else(|| crate::ui::icons::global(look.icon()))
        {
            let size = Vec2::splat(aurora::TAG_ICON * s);
            painter.image(icon.id(), Rect::from_center_size(icon_at, size), WHOLE, ink(look.icon_color(p, *t)));
        }
        let x = pill.left() + (aurora::TAG_PAD + aurora::TAG_ICON + aurora::TAG_GAP) * s;
        let mut shape = TextShape::new(Pos2::new(x, pill.center().y - label.size().y * 0.5), label.clone(), p.ink);
        shape.opacity_factor = alpha;
        painter.add(shape);
    }
}
