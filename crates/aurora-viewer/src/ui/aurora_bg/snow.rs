//! December snow over the aurora sky (the login screen and the loading
//! screens that show the sky). Flakes fall in three depths: far ones, small
//! and slow, behind the far ridgeline; middle ones between the two ridges;
//! near ones, larger and quicker, in front of everything. Each flake sways
//! on its own while one slow wind, which rises and dies down, carries them
//! all (the near ones further: parallax). The crests take a light frosting.
//! Through the login cards' glass (`aurora_bg::frosted`) only the middle and
//! near flakes show, melted into soft blobs, at the same place as outside.
//!
//! On from the 1st to the 31st of December (local date), never with "Reduce
//! animations". `AURORA_LOGIN_SNOW=1|0` forces it on or off for a launch.
//! An Aurora addition: the Legacy Viewer's login page has no snow.
//!
//! Every flake is a pure function of its index and of the time, so the
//! background and the frosted panes agree without any state, and the snow
//! is already spread over the sky on the first frame.

use super::{hash, mix, v};
use crate::theme::Palette;
use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, Shape};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// How often the local date is read again (a login screen left open over
/// midnight on the 30th of November starts snowing within that time).
const RECHECK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
struct State {
    season: bool,
    checked: Instant,
    on: bool,
}

fn state_id() -> egui::Id {
    egui::Id::unique("aurora_bg_snow")
}

/// The snow season: the whole of December.
pub fn in_season(month: u32) -> bool {
    month == 12
}

/// `AURORA_LOGIN_SNOW`: `1`/`on` forces the snow, `0`/`off` stops it, any
/// other value leaves the date decide.
fn parse_forced(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "on" | "true" => Some(true),
        "0" | "off" | "false" => Some(false),
        _ => None,
    }
}

fn forced() -> Option<bool> {
    static FORCED: OnceLock<Option<bool>> = OnceLock::new();
    *FORCED.get_or_init(|| std::env::var("AURORA_LOGIN_SNOW").ok().as_deref().and_then(parse_forced))
}

fn season_now() -> bool {
    use chrono::Datelike;
    forced().unwrap_or_else(|| in_season(chrono::Local::now().month()))
}

/// Once a frame, from the app: whether the sky snows, read by `amount`.
pub fn update(ctx: &egui::Context, reduce_motion: bool) {
    let now = Instant::now();
    let last = ctx.data(|d| d.get_temp::<State>(state_id()));
    let (season, checked) = match last {
        Some(s) if now.duration_since(s.checked) < RECHECK => (s.season, s.checked),
        _ => (season_now(), now),
    };
    let on = season && !reduce_motion;
    if last.is_none_or(|s| s.season != season || s.on != on || s.checked != checked) {
        ctx.data_mut(|d| d.insert_temp(state_id(), State { season, checked, on }));
    }
}

/// How much it snows on this frame: 1 in December, 0 otherwise.
pub fn amount(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<State>(state_id())).is_some_and(|s| s.on) as u8 as f32
}

/// The depths, drawn at their place among the ridgelines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Depth {
    Far,
    Middle,
    Near,
}

struct Layer {
    /// Flakes on a 1920 × 1080 sky; scaled by the area.
    count: f32,
    /// Radius in points, smallest to largest.
    radius: (f32, f32),
    /// Fall speed in sky heights per second.
    fall: (f32, f32),
    /// Sway amplitude in points.
    sway: f32,
    /// How much of the wind reaches this depth.
    wind: f32,
    /// Opacity of the core.
    alpha: f32,
    /// Index offset, so the depths do not share their random numbers.
    seed: u32,
}

impl Depth {
    fn layer(self) -> Layer {
        match self {
            Depth::Far => Layer {
                count: 190.0,
                radius: (0.7, 1.2),
                fall: (0.030, 0.045),
                sway: 5.0,
                wind: 0.45,
                alpha: 0.42,
                seed: 10_000,
            },
            Depth::Middle => Layer {
                count: 120.0,
                radius: (1.3, 2.1),
                fall: (0.055, 0.080),
                sway: 11.0,
                wind: 0.75,
                alpha: 0.66,
                seed: 20_000,
            },
            Depth::Near => Layer {
                count: 26.0,
                radius: (2.6, 4.2),
                fall: (0.10, 0.14),
                sway: 22.0,
                wind: 1.25,
                alpha: 0.55,
                seed: 30_000,
            },
        }
    }
}

/// How far the wind has carried a flake by `t`, in points on a 1920-point
/// wide sky: the integral of a light steady breeze plus two slow gusts, so
/// a change of the wind never makes the flakes jump.
pub(super) fn drift(t: f32) -> f32 {
    const BASE: f32 = 9.0;
    const GUSTS: [(f32, f32, f32); 2] = [
        // amplitude (points per second), angular speed, phase
        (16.0, 0.11, 0.0),
        (7.0, 0.37, 1.3),
    ];
    BASE * t
        + GUSTS
            .iter()
            .map(|(a, w, phase)| a / w * (phase.cos() - (w * t + phase).cos()))
            .sum::<f32>()
}

/// Position of flake `i` of a layer at `t`, its fall wrapped over the sky
/// and a margin so it enters and leaves out of sight.
fn position(rect: Rect, layer: &Layer, i: u32, t: f32, margin: f32) -> Pos2 {
    let (w, h) = (rect.width(), rect.height());
    let k = layer.seed + i * 7;
    let fall = layer.fall.0 + (layer.fall.1 - layer.fall.0) * hash(k + 1);
    let span_y = h + 2.0 * margin;
    let y = (hash(k + 2) + t * fall * h / span_y).fract() * span_y - margin;
    // a sway of its own, with a slower second swing so no two flakes beat
    // in step
    let phase = hash(k + 3) * std::f32::consts::TAU;
    let speed = 0.45 + hash(k + 4) * 0.55;
    let sway = (t * speed + phase).sin() + 0.35 * (t * speed * 0.43 + phase * 1.7).sin();
    let span_x = w + 2.0 * margin;
    let x = hash(k) * span_x + drift(t) * layer.wind * w / 1920.0 + sway * layer.sway;
    Pos2::new(rect.left() - margin + x.rem_euclid(span_x), rect.top() + y)
}

/// A soft round dot: a bright core and a halo that fades to nothing, so the
/// edge stays smooth at any size.
fn dot(mesh: &mut Mesh, center: Pos2, radius: f32, halo: f32, core: Color32) {
    const SEG: u32 = 10;
    let i = mesh.vertices.len() as u32;
    mesh.vertices.push(v(center, core));
    let rim = core.gamma_multiply(0.8);
    for ring in [(radius, rim), (radius * halo, Color32::TRANSPARENT)] {
        for s in 0..SEG {
            let a = s as f32 / SEG as f32 * std::f32::consts::TAU;
            mesh.vertices.push(v(center + egui::vec2(a.cos(), a.sin()) * ring.0, ring.1));
        }
    }
    for s in 0..SEG {
        let (a0, a1) = (i + 1 + s, i + 1 + (s + 1) % SEG);
        let (b0, b1) = (a0 + SEG, a1 + SEG);
        mesh.add_triangle(i, a0, a1);
        mesh.add_triangle(a0, b0, b1);
        mesh.add_triangle(a0, b1, a1);
    }
}

/// The colour of the snow under the aurora: white with a touch of its violet.
pub(super) fn color(p: &Palette) -> Color32 {
    mix(Color32::WHITE, p.violet_pale, 0.14)
}

/// The flakes of one depth. `amount` (0–1) thins and fades them; `soft` is
/// the view through a frosted pane: no far flakes, the others twice as
/// wide and faint.
pub(super) fn flakes(p: &Palette, rect: Rect, t: f32, amount: f32, depth: Depth, soft: bool) -> Option<Shape> {
    if amount <= 0.0 || (soft && depth == Depth::Far) {
        return None;
    }
    let layer = depth.layer();
    let area = (rect.width() * rect.height() / (1920.0 * 1080.0)).clamp(0.25, 2.5);
    let count = (layer.count * area * amount.min(1.0)).round() as u32;
    let (grow, alpha, halo) = if soft { (2.6, 0.30, 2.6) } else { (1.0, 1.0, 2.0) };
    let snow = color(p);
    let margin = layer.radius.1 * grow * halo + layer.sway * 1.4;
    let mut mesh = Mesh::default();
    for i in 0..count {
        let pos = position(rect, &layer, i, t, margin);
        let r = (layer.radius.0 + (layer.radius.1 - layer.radius.0) * hash(layer.seed + i * 7 + 5)) * grow;
        // a few flakes catch more light than the others
        let glint = 0.75 + 0.25 * hash(layer.seed + i * 7 + 6);
        let core = snow.gamma_multiply((layer.alpha * glint * alpha * amount).clamp(0.0, 1.0));
        dot(&mut mesh, pos, r, halo, core);
    }
    Some(Shape::mesh(mesh))
}

/// Strength of the frosting on a crest: only the parts of the ridgeline that
/// stand high (`height` 0–1 of its amplitude) keep snow.
pub(super) fn cap(height: f32) -> f32 {
    let x = ((height - 0.35) / 0.5).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_snows_in_december_only() {
        assert!(in_season(12));
        for month in 1..=11 {
            assert!(!in_season(month), "month {month}");
        }
    }

    #[test]
    fn the_variable_forces_the_snow_either_way() {
        assert_eq!(parse_forced("1"), Some(true));
        assert_eq!(parse_forced(" On "), Some(true));
        assert_eq!(parse_forced("0"), Some(false));
        assert_eq!(parse_forced("off"), Some(false));
        assert_eq!(parse_forced("december"), None);
        assert_eq!(parse_forced(""), None);
    }

    #[test]
    fn the_wind_carries_the_flakes_without_a_jump() {
        assert_eq!(drift(0.0), 0.0);
        let mut last = drift(0.0);
        for step in 1..20_000 {
            let now = drift(step as f32 * 0.05);
            // never more than a strong gust in one twentieth of a second
            assert!((now - last).abs() < 2.0, "jump at step {step}: {last} → {now}");
            last = now;
        }
    }

    #[test]
    fn every_flake_stays_over_the_sky_and_its_margin() {
        let rect = Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(1280.0, 720.0));
        for depth in [Depth::Far, Depth::Middle, Depth::Near] {
            let layer = depth.layer();
            let margin = 30.0;
            for i in 0..200 {
                for t in [0.0, 1.5, 60.0, 3600.0, 86_400.0] {
                    let pos = position(rect, &layer, i, t, margin);
                    assert!(
                        pos.x >= rect.left() - margin && pos.x <= rect.right() + margin,
                        "{depth:?} {i} {t}: {pos:?}"
                    );
                    assert!(
                        pos.y >= rect.top() - margin && pos.y <= rect.bottom() + margin,
                        "{depth:?} {i} {t}: {pos:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_frosted_view_has_no_far_flakes_and_no_snow_means_nothing_drawn() {
        let p = crate::theme::Theme::default().skin_palette();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(1920.0, 1080.0));
        assert!(flakes(&p, rect, 3.0, 1.0, Depth::Far, true).is_none());
        assert!(flakes(&p, rect, 3.0, 1.0, Depth::Near, true).is_some());
        for depth in [Depth::Far, Depth::Middle, Depth::Near] {
            assert!(flakes(&p, rect, 3.0, 0.0, depth, false).is_none());
        }
    }

    #[test]
    fn only_the_high_crests_keep_snow() {
        assert_eq!(cap(0.0), 0.0);
        assert_eq!(cap(0.3), 0.0);
        assert!(cap(0.6) > 0.0 && cap(0.6) < 1.0);
        assert_eq!(cap(1.0), 1.0);
    }
}
