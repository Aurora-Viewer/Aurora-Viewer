//! The northern light on the plate of a member of the Aurora team
//! (`world::staff`): curtains of light rising from behind its top edge, a
//! border whose colors flow around it over a soft halo, and on the full and
//! compact plates the "Aurora Team" tag over its top left edge. Built every
//! frame, since it moves, for the few plates that have it.
//!
//! Aurora's own: the Legacy Viewer has no staff mark but the Linden color
//! of the names.

use crate::theme::Palette;
use egui::{Color32, Mesh, Pos2, Rect, Vec2};
use std::f32::consts::{PI, TAU};

/// Height of the curtains over the plate's height.
const CURTAIN_HEIGHT: f32 = 1.05;
/// Share of their light that covers what is behind them; the rest is added
/// to it (premultiplied with less alpha than color): the curtains glow over
/// the night and still show over a bright sky.
const CURTAIN_COVER: f32 = 0.55;
/// The halo around the border, in mockup pixels, and its brightness.
const HALO: f32 = 7.0;
const HALO_LIGHT: f32 = 0.42;

/// The tag, in mockup pixels: its height, padding, icon, the gap between
/// the icon and the text, and the text's size.
pub const TAG_HEIGHT: f32 = 17.0;
pub const TAG_PAD: f32 = 7.0;
pub const TAG_ICON: f32 = 11.0;
pub const TAG_GAP: f32 = 4.0;
pub const TAG_TEXT: f32 = 10.5;
/// How far the tag's middle stands above the plate's top edge: it floats
/// mostly outside, clear of the name.
pub const TAG_RISE: f32 = 3.0;

/// Width of the tag around a text `text` points wide, at `scale`.
pub fn tag_width(text: f32, scale: f32) -> f32 {
    (2.0 * TAG_PAD + TAG_ICON + TAG_GAP) * scale + text
}

/// Seconds of the light's motion at `now`; held still with "Reduce
/// animations".
pub fn time(now: f64, still: bool) -> f32 {
    if still { 7.0 } else { now.rem_euclid(3600.0) as f32 }
}

/// A light of `color` at `i` (0..1): mostly added to what is behind it.
pub(super) fn light(color: Color32, i: f32, cover: f32) -> Color32 {
    let i = i.clamp(0.0, 1.0);
    let [r, g, b, _] = color.to_opaque().to_array();
    let k = |c: u8| (c as f32 * i).round() as u8;
    Color32::from_rgba_premultiplied(k(r), k(g), k(b), (255.0 * i * cover).round() as u8)
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The color where the border is `s` of the way around (0..1) at `t`: the
/// borealis twice around the plate, turning slowly, with brighter glints
/// running faster.
pub fn border_color(p: &Palette, s: f32, t: f32) -> Color32 {
    let u = 0.5 + 0.5 * (TAU * (2.0 * s - 0.07 * t)).sin();
    let glint = 0.78 + 0.22 * (TAU * (5.0 * s + 0.21 * t)).sin();
    p.borealis_at(u).to_opaque().gamma_multiply(glint)
}

/// The curtains behind a plate whose rectangle is `rect`, at `alpha`. Two
/// layers: a bright green one, folded, with fine rays, and a fainter one
/// higher up turning violet, slower, like the top of a real aurora.
pub fn curtains(mesh: &mut Mesh, p: &Palette, rect: Rect, ppp: f32, t: f32, alpha: f32) {
    let r = rect.height() * 0.5;
    // the whole width of the plate, the foot following its round ends
    let (left, right) = (rect.left(), rect.right());
    let w = (right - left).max(1.0);
    let foot_y = |x: f32| {
        let off = (rect.left() + r - x).max(x - (rect.right() - r)).max(0.0);
        rect.center().y - (r * r - off * off).max(0.0).sqrt() + 1.0
    };
    let full = rect.height() * CURTAIN_HEIGHT;
    let columns = ((w * ppp / 3.0).ceil() as usize).clamp(8, 180);
    // only the very tips fade out
    let end = (0.25 * r / w).clamp(0.03, 0.12);
    // (phase, speed, height, brightness, colors of the foot, middle and top)
    let layers = [(0.0, 1.0, 1.0, 1.0, [0.0, 0.22, 0.5]), (2.1, 0.68, 1.3, 0.65, [0.35, 0.68, 1.0])];
    for (phase, speed, high, bright, colors) in layers {
        let [foot, middle, top] = colors.map(|c| p.borealis_at(c));
        let first = mesh.vertices.len() as u32;
        for i in 0..=columns {
            let u = i as f32 / columns as f32;
            let x = left + u * w;
            let ts = t * speed;
            let fold = (u * 6.3 + 0.8 * ts + phase + 1.2 * (u * 2.7 - 0.45 * ts).sin()).sin();
            // folds that brighten and dim, never quite dark
            let band = 0.3 + 0.7 * (0.5 + 0.5 * fold).powf(1.3);
            let rays = 0.6 + 0.4 * (u * 38.0 + 1.9 * ts + 2.5 * (u * 11.0 + 0.6 * ts + phase).sin()).sin();
            let fade = smooth(u / end) * smooth((1.0 - u) / end);
            let i = (band * rays).powf(0.8) * fade * bright * alpha;
            let h = full * high * (0.55 + 0.45 * (0.5 + 0.5 * (u * 4.1 - 0.5 * ts + phase).sin())) * (0.7 + 0.3 * band);
            // the rays lean a little as the curtain sways
            let lean = 0.12 * h * (u * 3.0 + 0.3 * ts + phase).sin();
            let base = foot_y(x);
            mesh.colored_vertex(Pos2::new(x, base), light(foot, i, CURTAIN_COVER));
            mesh.colored_vertex(Pos2::new(x + lean * 0.3, base - 0.32 * h), light(middle, i * 0.8, CURTAIN_COVER));
            mesh.colored_vertex(Pos2::new(x + lean, base - h), light(top, 0.0, CURTAIN_COVER));
        }
        for i in 0..columns as u32 {
            let (a, b) = (first + 3 * i, first + 3 * (i + 1));
            for k in 0..2 {
                mesh.indices
                    .extend_from_slice(&[a + k, b + k, b + k + 1, a + k, b + k + 1, a + k + 1]);
            }
        }
    }
}

/// Points around a rounded rectangle, clockwise from the left end of its
/// top edge, a few pixels apart even along its straight edges (the colors
/// flow along them): position, outward normal, and how far around (0..1).
struct Perimeter {
    points: Vec<(Pos2, Vec2, f32)>,
    center: Pos2,
}

impl Perimeter {
    /// A pill: round ends.
    fn of(rect: Rect, ppp: f32) -> Perimeter {
        Perimeter::rounded(rect, rect.height() * 0.5, ppp)
    }

    fn rounded(rect: Rect, radius: f32, ppp: f32) -> Perimeter {
        let r = radius.clamp(0.0, rect.height().min(rect.width()) * 0.5);
        let (w, h) = ((rect.width() - 2.0 * r).max(0.0), (rect.height() - 2.0 * r).max(0.0));
        let (l, t) = (rect.left() + r, rect.top() + r);
        let arc = ((0.5 * PI * r * ppp / 3.0).ceil() as usize).clamp(2, 16);
        let mut points = Vec::new();
        // each side then its corner: (start, direction, length, outward normal, corner center)
        let sides = [
            (Pos2::new(l, rect.top()), Vec2::X, w, -Vec2::Y, Pos2::new(l + w, t)),
            (Pos2::new(rect.right(), t), Vec2::Y, h, Vec2::X, Pos2::new(l + w, t + h)),
            (Pos2::new(l + w, rect.bottom()), -Vec2::X, w, Vec2::Y, Pos2::new(l, t + h)),
            (Pos2::new(rect.left(), t + h), -Vec2::Y, h, -Vec2::X, Pos2::new(l, t)),
        ];
        for (k, (start, dir, length, normal, corner)) in sides.into_iter().enumerate() {
            let steps = if length > 0.01 {
                ((length * ppp / 4.0).ceil() as usize).max(1)
            } else {
                0
            };
            for i in 0..steps {
                points.push((start + dir * (length * i as f32 / steps as f32), normal));
            }
            if r > 0.0 {
                let from = -PI * 0.5 + k as f32 * PI * 0.5;
                for i in 0..arc {
                    let a = from + 0.5 * PI * i as f32 / arc as f32;
                    let n = Vec2::new(a.cos(), a.sin());
                    points.push((corner + n * r, n));
                }
            }
        }
        let total = (2.0 * (w + h) + TAU * r).max(f32::EPSILON);
        let mut along = 0.0;
        let mut last = points[0].0;
        let points = points
            .into_iter()
            .map(|(pos, n)| {
                along += (pos - last).length();
                last = pos;
                (pos, n, along / total)
            })
            .collect();
        Perimeter {
            points,
            center: rect.center(),
        }
    }

    /// A ring `d` outside the pill (inside when negative), each vertex in
    /// the color `color` gives for how far around it is.
    fn ring(&self, mesh: &mut Mesh, d: f32, mut color: impl FnMut(f32) -> Color32) -> u32 {
        let first = mesh.vertices.len() as u32;
        for &(pos, n, s) in &self.points {
            mesh.colored_vertex(pos + n * d, color(s));
        }
        first
    }

    fn band(&self, mesh: &mut Mesh, inner: u32, outer: u32) {
        let n = self.points.len() as u32;
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.indices
                .extend_from_slice(&[inner + i, inner + j, outer + j, inner + i, outer + j, outer + i]);
        }
    }

    fn fill(&self, mesh: &mut Mesh, ring: u32, color: Color32) {
        let center = mesh.vertices.len() as u32;
        mesh.colored_vertex(self.center, color);
        let n = self.points.len() as u32;
        for i in 0..n {
            mesh.indices.extend_from_slice(&[center, ring + i, ring + (i + 1) % n]);
        }
    }
}

/// A plate's border, `width` points along the inside of its edge, in the
/// color `edge` gives for each point around it (0..1), over a halo of the
/// same colors `scale` mockup pixels to points.
pub(super) fn border(mesh: &mut Mesh, rect: Rect, ppp: f32, alpha: f32, width: f32, scale: f32, edge: impl Fn(f32) -> Color32) {
    let o = Perimeter::of(rect, ppp);
    let f = 0.5 / ppp;
    let color = |s: f32| edge(s).gamma_multiply(alpha);
    let halo = |s: f32| light(edge(s), HALO_LIGHT * alpha, 0.5);
    let a = o.ring(mesh, -width - f, |_| Color32::TRANSPARENT);
    let b = o.ring(mesh, -width + f, color);
    let c = o.ring(mesh, -f, color);
    let d = o.ring(mesh, f, halo);
    let e = o.ring(mesh, HALO * scale, |_| Color32::TRANSPARENT);
    o.band(mesh, a, b);
    o.band(mesh, b, c);
    o.band(mesh, c, d);
    o.band(mesh, d, e);
}

/// The tag at `rect`, with corners of `radius` (half its height on a
/// plate): `fill` inside a border `width` wide, in the color `color` gives
/// for each point around it (0..1).
#[allow(clippy::too_many_arguments)]
pub(super) fn tag(
    mesh: &mut Mesh,
    rect: Rect,
    radius: f32,
    ppp: f32,
    fill: Color32,
    alpha: f32,
    width: f32,
    color: impl Fn(f32) -> Color32,
) {
    let o = Perimeter::rounded(rect, radius, ppp);
    let f = 0.5 / ppp;
    let fill = fill.gamma_multiply(alpha);
    let edge = |s: f32| super::mesh::over(color(s).gamma_multiply(alpha), fill);
    let inner = o.ring(mesh, -width - f, |_| fill);
    o.fill(mesh, inner, fill);
    let b = o.ring(mesh, -width + f, edge);
    let c = o.ring(mesh, -f, edge);
    let d = o.ring(mesh, f, |_| Color32::TRANSPARENT);
    o.band(mesh, inner, b);
    o.band(mesh, b, c);
    o.band(mesh, c, d);
}

/// The color of the `i`th of `n` letters of the tag at `t`: the borealis
/// from green to violet across the word, a wave of light running along it.
pub fn letter_color(p: &Palette, i: usize, n: usize, t: f32) -> Color32 {
    let u = i as f32 / n.saturating_sub(1).max(1) as f32;
    let shine = (TAU * (u * 0.8 - 0.25 * t)).sin().max(0.0).powi(3);
    p.borealis_at(0.1 + 0.85 * u).lerp_to_gamma(p.ink, 0.25 + 0.45 * shine)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pill() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(160.0, 50.0))
    }

    #[test]
    fn the_perimeter_goes_once_around_the_pill() {
        let o = Perimeter::of(pill(), 1.0);
        assert!(o.points.len() > 60);
        let r = 25.0;
        for &(pos, n, s) in &o.points {
            assert!((0.0..=1.0).contains(&s));
            assert!((n.length() - 1.0).abs() < 1e-4);
            // every point is on the edge
            let cx = pos.x.clamp(125.0, 235.0);
            assert!(((pos.x - cx).hypot(pos.y - 75.0) - r).abs() < 0.01, "{pos:?}");
        }
        assert!(o.points.windows(2).all(|w| w[1].2 > w[0].2));
        // the straight edges are cut too: the colors flow along them
        assert!(o.points.iter().filter(|(pos, ..)| pos.y == 50.0).count() > 20);
    }

    #[test]
    fn the_light_stays_inside_its_meshes() {
        let p = crate::theme::Theme::default().palette();
        let mut mesh = Mesh::default();
        curtains(&mut mesh, &p, pill(), 1.0, 3.0, 1.0);
        assert!(mesh.is_valid() && !mesh.indices.is_empty());
        // above the plate, never below its top edge
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.pos.y <= 76.0 && v.pos.y >= 50.0 - 50.0 * CURTAIN_HEIGHT * 1.31)
        );
        // over the whole width, the feet on the plate's outline (its round
        // ends included), never under it
        let feet: Vec<_> = mesh.vertices.iter().step_by(3).map(|v| v.pos).collect();
        assert!(feet.iter().any(|f| f.x < 101.0) && feet.iter().any(|f| f.x > 259.0));
        for f in &feet {
            let cx = f.x.clamp(125.0, 235.0);
            assert!(((f.x - cx).hypot(f.y - 1.0 - 75.0) - 25.0).abs() < 0.05, "{f:?}");
        }
        // the tops are dark, the feet lit somewhere
        assert!(mesh.vertices.iter().any(|v| v.color.g() > 40));
        let mut ring = Mesh::default();
        let edge = |s: f32| border_color(&p, s, 3.0);
        border(&mut ring, pill(), 1.0, 1.0, 1.5, 0.76, edge);
        tag(&mut ring, pill(), 25.0, 2.0, p.navy, 0.9, 1.0, edge);
        tag(&mut ring, pill(), 3.0, 1.5, p.navy, 1.0, 1.0, edge);
        assert!(ring.is_valid());
        // the theme's small corners: every point within the rectangle's edge
        let o = Perimeter::rounded(pill(), 3.0, 1.0);
        assert!(o.points.iter().all(|(pos, ..)| pill().expand(0.01).contains(*pos)));
        assert!(o.points.iter().any(|(pos, ..)| pos.x == 260.0));
        // faded out: nothing
        let mut faded = Mesh::default();
        curtains(&mut faded, &p, pill(), 1.0, 3.0, 0.0);
        assert!(faded.vertices.iter().all(|v| v.color == Color32::TRANSPARENT));
    }

    #[test]
    fn the_light_moves_unless_held_still() {
        let p = crate::theme::Theme::default().palette();
        assert_ne!(border_color(&p, 0.1, 0.0), border_color(&p, 0.1, 2.0));
        assert_eq!(time(10.0, true), time(500.0, true));
        assert!(time(10.0, false) < time(11.0, false));
        // the word goes from green to violet
        assert_ne!(letter_color(&p, 0, 12, 0.0), letter_color(&p, 11, 12, 0.0));
        assert!(tag_width(40.0, 1.0) > 40.0);
    }
}
