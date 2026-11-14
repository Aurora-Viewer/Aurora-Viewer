//! The aurora's colours of the update notice in Aurora's notification center
//! (the "edge" variant the human chose on 2026-11-06): a fine edge whose
//! teal, violet and indigo slowly flow around the card, a soft halo that
//! breathes outside it, and the title in the same colours. All of it stands
//! still with Reduce motion (`theme::apply_motion`: no animation time).
//!
//! Drawn as two vertex-coloured meshes, a few hundred vertices, only while
//! the card is out.

use crate::theme::Palette;
use egui::{Color32, Mesh, Pos2, Rect, Vec2, epaint::Shape, text::LayoutJob};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// One turn of the colours around the card (the mockup's 9 s).
const FLOW: f64 = 9.0;
/// The halo's breath (4.5 s).
const BREATH: f64 = 4.5;
/// The title's colours (12 s).
const TITLE_FLOW: f64 = 12.0;
/// The edge's width and the halo's reach outside the card.
const EDGE: f32 = 1.5;
const HALO: f32 = 7.0;
/// Longest straight piece of the outline, so the colours stay smooth.
const STEP: f32 = 8.0;

/// The colour `t` of the way along the looped gradient (teal, violet,
/// indigo, light violet, back to teal); `t` wraps.
pub fn color_at(p: &Palette, t: f32) -> Color32 {
    let stops = [p.teal, p.violet, p.indigo, p.violet_light];
    let t = t.rem_euclid(1.0) * stops.len() as f32;
    let i = (t as usize).min(stops.len() - 1);
    stops[i].lerp_to_gamma(stops[(i + 1) % stops.len()], t - i as f32)
}

/// The title's colour `x` of the way along it: teal to pale violet and back.
fn title_color(p: &Palette, x: f32) -> Color32 {
    p.teal.lerp_to_gamma(p.violet_pale, 0.5 - 0.5 * (TAU * x).cos())
}

/// Seconds of the animation: the frame's time, or a fixed moment with
/// Reduce motion.
fn clock(ui: &egui::Ui) -> f64 {
    if ui.style().animation_time > 0.0 {
        ui.ctx().request_repaint();
        ui.input(|i| i.time)
    } else {
        0.0
    }
}

/// Points around `rect` with rounded corners of `radius`, clockwise from the
/// top left corner's end, each with its outward normal and its distance
/// along the outline; then the outline's length.
pub fn outline(rect: Rect, radius: f32) -> (Vec<(Pos2, Vec2, f32)>, f32) {
    let r = radius.clamp(0.0, rect.width().min(rect.height()) / 2.0);
    // corner centres, clockwise from the top left, and their first angle
    let corners = [
        (Pos2::new(rect.left() + r, rect.top() + r), PI),
        (Pos2::new(rect.right() - r, rect.top() + r), -FRAC_PI_2),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), FRAC_PI_2),
    ];
    let arc = if r > 0.5 { 6 } else { 1 };
    let mut points: Vec<(Pos2, Vec2)> = Vec::new();
    for (i, (center, start)) in corners.iter().enumerate() {
        for k in 0..=arc {
            let a = start + FRAC_PI_2 * k as f32 / arc as f32;
            let n = Vec2::angled(a);
            points.push((*center + n * r, n));
        }
        // the straight side to the next corner, cut in short pieces
        let (next, next_start) = corners[(i + 1) % corners.len()];
        let from = *center + Vec2::angled(start + FRAC_PI_2) * r;
        let to = next + Vec2::angled(next_start) * r;
        let normal = Vec2::angled(start + FRAC_PI_2);
        let pieces = ((to - from).length() / STEP).ceil().max(1.0) as usize;
        for k in 1..pieces {
            points.push((from + (to - from) * (k as f32 / pieces as f32), normal));
        }
    }
    let mut out = Vec::with_capacity(points.len() + 1);
    let mut along = 0.0;
    for (i, (at, n)) in points.iter().enumerate() {
        if i > 0 {
            along += (*at - points[i - 1].0).length();
        }
        out.push((*at, *n, along));
    }
    let length = along + (points[0].0 - points[points.len() - 1].0).length();
    // closed: the first point again, a whole turn along
    out.push((points[0].0, points[0].1, length));
    (out, length)
}

/// A band along the outline, from `inner` to `outer` (offsets along the
/// normal), each edge of it coloured by `color(fraction of the outline)`.
fn band(outline: &[(Pos2, Vec2, f32)], length: f32, inner: f32, outer: f32, color: impl Fn(f32) -> (Color32, Color32)) -> Mesh {
    let mut mesh = Mesh::default();
    for (i, (at, n, along)) in outline.iter().enumerate() {
        let (a, b) = color(*along / length.max(1.0));
        mesh.colored_vertex(*at + *n * inner, a);
        mesh.colored_vertex(*at + *n * outer, b);
        if i > 0 {
            let v = (2 * i) as u32;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    mesh
}

/// The edge and the halo around the card drawn in `rect`.
pub fn paint_edge(ui: &egui::Ui, p: &Palette, rect: Rect) {
    let t = clock(ui);
    let phase = (t / FLOW).fract() as f32;
    let breath = 0.65 + 0.2 * (TAU * (t / BREATH) as f32).sin();
    let radius = f32::from(ui.visuals().window_corner_radius.nw);
    let (outline, length) = outline(rect, radius);
    let halo = band(&outline, length, 0.0, HALO, |x| {
        let c = color_at(p, x - phase);
        (c.gamma_multiply(0.6 * breath), Color32::TRANSPARENT)
    });
    let edge = band(&outline, length, -EDGE / 2.0, EDGE / 2.0, |x| {
        let c = color_at(p, x - phase);
        (c, c)
    });
    // the halo reaches over the panel's margin, never further than the card's
    // own reach past what is in view (a list scrolled)
    let painter = ui
        .ctx()
        .layer_painter(ui.layer_id())
        .with_clip_rect(ui.clip_rect().expand(HALO + 1.0).intersect(rect.expand(HALO)));
    painter.add(Shape::mesh(halo));
    ui.painter().add(Shape::mesh(edge));
}

/// The card's title in the aurora's colours, flowing slowly along it.
pub fn title(ui: &egui::Ui, p: &Palette, text: &str, size: f32) -> LayoutJob {
    let phase = (clock(ui) / TITLE_FLOW).fract() as f32;
    let count = text.chars().count().max(1) as f32;
    let mut job = LayoutJob::default();
    for (i, c) in text.chars().enumerate() {
        let mut buf = [0; 4];
        job.append(
            c.encode_utf8(&mut buf),
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(size), title_color(p, i as f32 / count - phase)),
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outline_goes_once_around_the_card_close_to_it() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(300.0, 90.0));
        let (points, length) = outline(rect, 4.0);
        // a rounded rectangle's perimeter: the sides less the corners'
        // squares, plus a circle
        let expected = 2.0 * (300.0 + 90.0) - 8.0 * 4.0 + TAU * 4.0;
        assert!((length - expected).abs() < 1.0, "{length} vs {expected}");
        assert_eq!(points.first().map(|p| p.0), points.last().map(|p| p.0));
        assert!(points.windows(2).all(|w| (w[1].0 - w[0].0).length() <= STEP + 0.01));
        // every point on the edge, its normal pointing out
        for (at, n, _) in &points {
            assert!(rect.expand(0.01).contains(*at));
            assert!(!rect.shrink(0.5).contains(*at + *n));
        }
    }

    #[test]
    fn the_gradient_loops_through_the_brand_colours() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(color_at(&p, 0.0), p.teal);
        assert_eq!(color_at(&p, 0.25), p.violet);
        assert_eq!(color_at(&p, 0.5), p.indigo);
        assert_eq!(color_at(&p, 1.0), p.teal);
        assert_eq!(color_at(&p, -0.75), p.violet);
    }
}
