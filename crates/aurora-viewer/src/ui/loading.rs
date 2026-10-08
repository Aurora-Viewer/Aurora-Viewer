//! Loading (after login) and teleport screens: one shared "splash" (logo
//! with a soft light, title, subtitle, gradient progress bar, detail line)
//! over the last view blurred, or the aurora background.

use crate::theme::Palette;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, RichText, Shape, Vec2};

pub struct LoadingInfo<'a> {
    pub stage: &'a str,
    pub fraction: f32,
    pub region: &'a str,
    pub objects: usize,
    pub textures: (usize, usize),
    pub terrain: f32,
    pub elapsed: f32,
}

/// Fade-out duration of the loading / teleport screens.
pub const FADE_SECONDS: f32 = 0.7;

/// Background of the loading screens: the last view (blurred) or the
/// aurora background.
pub fn draw_backdrop(ui: &egui::Ui, p: &Palette, backdrop: Option<&super::backdrop::Backdrop>) {
    let rect = ui.ctx().content_rect();
    let painter = ui.ctx().layer_painter(egui::LayerId::background());
    if !backdrop.is_some_and(|b| b.paint(&painter, p, rect, 1.0)) {
        super::login::draw_background(ui, p);
    }
}

fn additive(c: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied((c.r() as f32 * k) as u8, (c.g() as f32 * k) as u8, (c.b() as f32 * k) as u8, 0)
}

/// Horizontal gradient bar (violet → teal) with rounded look and a glow.
fn gradient_rect(painter: &egui::Painter, r: Rect, a: Color32, b: Color32) {
    let mut m = Mesh::default();
    let v = |pos: Pos2, color: Color32| Vertex { pos, uv: WHITE_UV, color };
    m.vertices.push(v(r.left_top(), a));
    m.vertices.push(v(r.right_top(), b));
    m.vertices.push(v(r.right_bottom(), b));
    m.vertices.push(v(r.left_bottom(), a));
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(m));
}

/// Progress bar (track, gradient fill, glow). `fraction` None = moving
/// segment (unknown duration).
pub fn paint_progress(painter: &egui::Painter, p: &Palette, r: Rect, fraction: Option<f32>, t: f32, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    painter.rect_filled(r, r.height() * 0.5, Color32::from_white_alpha((22.0 * a) as u8));
    let fill = match fraction {
        Some(f) => Rect::from_min_size(r.min, Vec2::new(r.width() * f.clamp(0.0, 1.0), r.height())),
        None => {
            let seg = r.width() * 0.28;
            let u = 0.5 - 0.5 * (t * 2.2).cos();
            Rect::from_min_size(Pos2::new(r.left() + (r.width() - seg) * u, r.top()), Vec2::new(seg, r.height()))
        }
    };
    if fill.width() < 1.0 {
        return;
    }
    // glow under the fill
    painter.rect_filled(fill.expand2(Vec2::new(4.0, 5.0)), 8.0, additive(p.violet, 0.22 * a));
    let (c0, c1) = (p.violet.gamma_multiply(a), p.teal.gamma_multiply(a));
    // rounded ends: caps + gradient body
    let rad = r.height() * 0.5;
    painter.circle_filled(Pos2::new(fill.left() + rad, fill.center().y), rad, c0);
    painter.circle_filled(Pos2::new(fill.right() - rad, fill.center().y), rad, c1);
    if fill.width() > r.height() {
        gradient_rect(painter, fill.shrink2(Vec2::new(rad, 0.0)), c0, c1);
    }
}

/// Progress bar laid out in a `Ui` (login card).
pub fn progress_bar(ui: &mut egui::Ui, p: &Palette, width: f32, fraction: Option<f32>, t: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), egui::Sense::hover());
    paint_progress(ui.painter(), p, r, fraction, t, 1.0);
    ui.ctx().request_repaint();
}

/// The shared splash, centered in `screen`. Returns the rect under the
/// detail line (for an optional button).
#[allow(clippy::too_many_arguments)]
fn splash(
    painter: &egui::Painter,
    p: &Palette,
    screen: Rect,
    logo: Option<&egui::TextureHandle>,
    title: &str,
    subtitle: &str,
    fraction: Option<f32>,
    detail: &str,
    t: f32,
    alpha: f32,
) -> Pos2 {
    let a = alpha.clamp(0.0, 1.0);
    let c = screen.center() - Vec2::new(0.0, 30.0);
    if let Some(l) = logo {
        let lc = c - Vec2::new(0.0, 80.0);
        // breathing light behind the logo
        let breathe = 0.85 + 0.15 * (t * 1.4).sin();
        super::aurora_bg::radial_glow(painter, lc, 160.0, p.violet, 0.45 * breathe * a);
        let lr = Rect::from_center_size(lc, Vec2::splat(140.0));
        painter.image(
            l.id(),
            lr,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(a),
        );
    }
    painter.text(
        c + Vec2::new(0.0, 22.0),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(26.0),
        p.ink.gamma_multiply(a),
    );
    if !subtitle.is_empty() {
        painter.text(
            c + Vec2::new(0.0, 52.0),
            egui::Align2::CENTER_CENTER,
            subtitle,
            egui::FontId::proportional(14.0),
            p.violet_pale.gamma_multiply(a),
        );
    }
    let bar = Rect::from_center_size(c + Vec2::new(0.0, 82.0), Vec2::new(380.0, 6.0));
    paint_progress(painter, p, bar, fraction, t, a);
    if !detail.is_empty() {
        painter.text(
            c + Vec2::new(0.0, 104.0),
            egui::Align2::CENTER_CENTER,
            detail,
            egui::FontId::proportional(12.0),
            p.muted.gamma_multiply(a),
        );
    }
    c + Vec2::new(0.0, 126.0)
}

/// Loading screen between login and the first rendered frames.
pub fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    logo: Option<&egui::TextureHandle>,
    backdrop: Option<&super::backdrop::Backdrop>,
    info: &LoadingInfo,
) -> bool {
    draw_backdrop(ui, p, backdrop);
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("loading_splash")));
    let t = ctx.input(|i| i.time) as f32;
    let subtitle = if info.region.is_empty() {
        info.stage.to_owned()
    } else {
        info.region.to_owned()
    };
    let detail = if info.region.is_empty() {
        String::new()
    } else {
        format!(
            "{}  ·  terrain {:.0} %  ·  {} objets  ·  textures {}/{}",
            info.stage,
            info.terrain * 100.0,
            info.objects,
            info.textures.0,
            info.textures.1
        )
    };
    let below = splash(
        &painter,
        p,
        screen,
        logo,
        "Chargement du monde",
        &subtitle,
        Some(info.fraction),
        &detail,
        t,
        1.0,
    );
    ctx.request_repaint();
    let mut cancel = false;
    if info.elapsed > 20.0 {
        egui::Area::new(egui::Id::new("loading_cancel"))
            .fixed_pos(below - Vec2::new(30.0, 0.0))
            .show(&ctx, |ui| {
                if ui.link(RichText::new("Annuler").color(p.muted)).clicked() {
                    cancel = true;
                }
            });
    }
    cancel
}

/// Teleport screen over the world (and the end of the loading screen fading
/// out): same splash, moving bar.
#[allow(clippy::too_many_arguments)]
pub fn teleport_overlay(
    ctx: &egui::Context,
    p: &Palette,
    logo: Option<&egui::TextureHandle>,
    backdrop: Option<&super::backdrop::Backdrop>,
    title: &str,
    dest: &str,
    detail: &str,
    fraction: Option<f32>,
    alpha: f32,
) {
    let a = alpha.clamp(0.0, 1.0);
    let rect = ctx.content_rect();
    let t = ctx.input(|i| i.time) as f32;
    egui::Area::new(egui::Id::new("teleport_overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min)
        .fade_in(false)
        .interactable(a > 0.5)
        .show(ctx, |ui| {
            // swallow clicks while it covers the screen
            let (r, _) = ui.allocate_exact_size(rect.size(), egui::Sense::click_and_drag());
            let painter = ui.painter_at(r);
            if !backdrop.is_some_and(|b| b.paint(&painter, p, r, a)) {
                super::aurora_bg::paint(&painter, p, r, t, a);
            }
            splash(&painter, p, r, logo, title, dest, fraction, detail, t, a);
        });
    ctx.request_repaint();
}
