//! Linden Lab's mark on the plate of one of its employees
//! (`world::staff::Mark::Linden`): four-pointed stars rising and
//! twinkling above the plate, a shine running around its border over a
//! soft halo, and the "Linden Lab" tag with the verified rosette. All in
//! the Linden colour of Preferences › Colors (the colour of their names
//! when names are coloured by relation, shared with the classic tags), so
//! that it stays apart from the Aurora team's northern light.
//!
//! Aurora's own: the Legacy Viewer only colours Linden names.

use super::aurora::light;
use crate::theme::Palette;
use egui::{Color32, Mesh, Pos2, Rect, Vec2};
use std::f32::consts::{FRAC_PI_4, PI, TAU};

/// Share of a star's light that covers what is behind it (`aurora::light`).
const STAR_COVER: f32 = 0.8;
/// Seconds the shine takes to go once around the border.
const SHINE_LAP: f32 = 2.6;

/// The Linden colour, opaque.
pub fn color() -> Color32 {
    crate::ui::colors::c(crate::ui::colors::get().tag_linden).to_opaque()
}

/// A number in 0..1 that looks random, the same for the same `k` and `salt`.
fn hash(k: u32, salt: f32) -> f32 {
    ((k as f32 * 12.989_8 + salt * 78.233).sin() * 43_758.547).rem_euclid(1.0)
}

/// The border's color `s` of the way around (0..1) at `t`: the Linden
/// colour, with a bright shine running around and a fainter one opposite.
pub fn edge(p: &Palette, base: Color32, s: f32, t: f32) -> Color32 {
    let head = (t / SHINE_LAP).rem_euclid(1.0);
    let gap = |at: f32| ((s - at).rem_euclid(1.0) - 0.5).abs();
    // distance from the head, then from its opposite (0.5 at the most)
    let (near, far) = (0.5 - gap(head), 0.5 - gap(head + 0.5));
    let shine = (-(near / 0.05).powi(2)).exp() + 0.45 * (-(far / 0.035).powi(2)).exp();
    base.gamma_multiply(0.85).lerp_to_gamma(p.ink, (0.85 * shine).min(0.9))
}

/// The color of the `i`th of `n` letters of the tag at `t`: the Linden
/// colour lightened, a shine sweeping across the word with the border's.
pub fn letter_color(p: &Palette, base: Color32, i: usize, n: usize, t: f32) -> Color32 {
    let u = i as f32 / n.saturating_sub(1).max(1) as f32;
    let sweep = (t / SHINE_LAP).rem_euclid(1.0) * 1.6 - 0.3;
    let shine = (-((u - sweep) / 0.12).powi(2)).exp();
    base.lerp_to_gamma(p.ink, 0.3 + 0.55 * shine)
}

/// A four-pointed star of `size` points at `at`, turned by `turn`, of
/// `color` at `i` (0..1): long thin rays and a bright core.
fn star(mesh: &mut Mesh, at: Pos2, size: f32, turn: f32, color: Color32, i: f32) {
    if i <= 0.004 || size <= 0.0 {
        return;
    }
    // a soft glow under the rays
    let glow = mesh.vertices.len() as u32;
    mesh.colored_vertex(at, light(color, i * 0.45, STAR_COVER * 0.5));
    for j in 0..12u32 {
        let a = j as f32 * TAU / 12.0;
        mesh.colored_vertex(at + Vec2::new(a.cos(), a.sin()) * size * 0.55, light(color, 0.0, STAR_COVER));
    }
    for j in 0..12u32 {
        mesh.indices.extend_from_slice(&[glow, glow + 1 + j, glow + 1 + (j + 1) % 12]);
    }
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(at, light(color, i, STAR_COVER));
    for j in 0..8u32 {
        let a = turn + j as f32 * FRAC_PI_4;
        let r = if j % 2 == 0 { size } else { size * 0.26 };
        let tip = if j % 2 == 0 { 0.0 } else { i * 0.55 };
        mesh.colored_vertex(at + Vec2::new(a.cos(), a.sin()) * r, light(color, tip, STAR_COVER));
    }
    for j in 0..8u32 {
        mesh.indices.extend_from_slice(&[center, center + 1 + j, center + 1 + (j + 1) % 8]);
    }
}

/// The stars rising above a plate whose rectangle is `rect`, at `alpha`:
/// each comes up from the top edge, twinkles and fades out on its way, on
/// a cycle of its own.
pub fn stars(mesh: &mut Mesh, p: &Palette, base: Color32, rect: Rect, t: f32, alpha: f32) {
    let k = (rect.height() / 38.0).clamp(0.45, 1.0);
    let count = ((rect.width() / 13.0) as u32).clamp(6, 22);
    let rise = rect.height() * 1.05;
    let bright = base.lerp_to_gamma(p.ink, 0.55);
    for n in 0..count {
        let x = rect.left() + rect.width() * (-0.04 + 1.08 * (n as f32 + hash(n, 1.0)) / count as f32);
        let period = 2.4 + 1.4 * hash(n, 2.0);
        let life = (t / period + hash(n, 3.0)).rem_euclid(1.0);
        let fade = (PI * life).sin().powi(2);
        let twinkle = 0.55 + 0.45 * (t * (5.0 + 3.0 * hash(n, 4.0)) + TAU * hash(n, 5.0)).sin();
        let size = (5.0 + 5.0 * hash(n, 6.0)) * k * (0.7 + 0.3 * twinkle);
        let drift = 4.0 * k * (t * 0.8 + 6.0 * hash(n, 7.0)).sin();
        let at = Pos2::new(x + drift, rect.top() - 2.0 - life * rise);
        star(mesh, at, size, 0.4 * t + TAU * hash(n, 8.0), bright, fade * twinkle * alpha);
    }
}

/// Stars twinkling where they are, over a whole `rect` (a profile's
/// banner), about one for every `spacing` points squared.
pub fn starfield(mesh: &mut Mesh, p: &Palette, base: Color32, rect: Rect, t: f32, spacing: f32) {
    let count = (rect.area() / (spacing * spacing)) as u32;
    let bright = base.lerp_to_gamma(p.ink, 0.55);
    for n in 0..count.clamp(8, 120) {
        let at = Pos2::new(
            rect.left() + rect.width() * hash(n, 11.0),
            rect.top() + rect.height() * hash(n, 12.0),
        );
        let twinkle = (0.5 + 0.5 * (t * (1.2 + 2.0 * hash(n, 13.0)) + TAU * hash(n, 14.0)).sin()).powi(2);
        let size = 3.5 + 7.5 * hash(n, 15.0).powi(2);
        star(mesh, at, size * (0.6 + 0.4 * twinkle), TAU * hash(n, 16.0), bright, twinkle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(160.0, 50.0))
    }

    #[test]
    fn the_stars_rise_above_the_plate_and_fade_out() {
        let p = crate::theme::Theme::default().palette();
        let base = p.rose;
        let mut mesh = Mesh::default();
        for t in [0.0, 0.7, 3.3, 11.0] {
            stars(&mut mesh, &p, base, plate(), t, 1.0);
        }
        assert!(mesh.is_valid() && !mesh.indices.is_empty());
        // over the top edge (a star's rays may reach a little below it)
        assert!(
            mesh.vertices
                .iter()
                .all(|v| v.pos.y <= 52.0 + 10.0 && v.pos.y >= 50.0 - 50.0 * 1.05 - 14.0)
        );
        let mut faded = Mesh::default();
        stars(&mut faded, &p, base, plate(), 1.0, 0.0);
        assert!(faded.vertices.is_empty());
        let mut field = Mesh::default();
        starfield(
            &mut field,
            &p,
            base,
            Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 130.0)),
            2.0,
            26.0,
        );
        assert!(field.is_valid() && field.vertices.len() > 22 * 50);
    }

    #[test]
    fn the_shine_runs_around_the_border() {
        let p = crate::theme::Theme::default().palette();
        let base = p.rose;
        // brightest at the head, which moves on
        let at = |s: f32, t: f32| edge(&p, base, s, t);
        let head = 0.25 * SHINE_LAP;
        assert_ne!(at(0.25, head), at(0.75 + 0.1, head));
        assert_ne!(at(0.25, head), at(0.25, head + 0.5));
        assert_eq!(at(0.0, 0.0), at(1.0, 0.0));
        assert_ne!(
            letter_color(&p, base, 0, 10, 0.3 * SHINE_LAP),
            letter_color(&p, base, 9, 10, 0.3 * SHINE_LAP)
        );
    }
}
