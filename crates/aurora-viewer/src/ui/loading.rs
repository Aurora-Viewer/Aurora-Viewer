//! Loading (after login) and teleport screens: one shared "splash" (logo
//! with a soft light above a card with the windows' surface: title, place,
//! flat gradient progress bar, detail line) over the last view blurred, or
//! the aurora background.

use crate::theme::Palette;
use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Pos2, Rect, Shape, Vec2};

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

/// Progress bar: a flat track with the brand gradient as fill. `fraction`
/// None = moving segment (unknown duration).
pub fn paint_progress(painter: &egui::Painter, p: &Palette, r: Rect, fraction: Option<f32>, t: f32, alpha: f32) {
    let a = alpha.clamp(0.0, 1.0);
    painter.rect_filled(r, 2.0, p.field.gamma_multiply(a));
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
    gradient_rect(painter, fill, p.violet.gamma_multiply(a), p.teal.gamma_multiply(a));
}

/// Progress bar laid out in a `Ui` (login card).
pub fn progress_bar(ui: &mut egui::Ui, p: &Palette, width: f32, fraction: Option<f32>, t: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 4.0), egui::Sense::hover());
    paint_progress(ui.painter(), p, r, fraction, t, 1.0);
    ui.ctx().request_repaint();
}

const CARD_WIDTH: f32 = 460.0;
const CARD_HEIGHT: f32 = 124.0;
const CARD_MARGIN: f32 = 20.0;

/// The shared splash, centered in `screen`: the brand block of the login
/// screen (logo and its light) above a card with the windows' surface, which
/// names what is loading, where, and how far it is. Painted by hand so the
/// whole splash fades with `alpha`. Returns the point under the card (for
/// an optional button).
#[allow(clippy::too_many_arguments)]
fn splash(
    ctx: &egui::Context,
    painter: &egui::Painter,
    p: &Palette,
    screen: Rect,
    logo: Option<&egui::TextureHandle>,
    title: &str,
    place: &str,
    fraction: Option<f32>,
    detail: &str,
    t: f32,
    alpha: f32,
) -> Pos2 {
    let a = alpha.clamp(0.0, 1.0);
    let c = screen.center();
    if let Some(l) = logo {
        let lc = c - Vec2::new(0.0, 112.0);
        // breathing light behind the logo
        let breathe = 0.85 + 0.15 * (t * 1.4).sin();
        super::aurora_bg::radial_glow(painter, lc, 150.0, p.violet, 0.42 * breathe * a);
        let lr = Rect::from_center_size(lc, Vec2::splat(128.0));
        painter.image(
            l.id(),
            lr,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(a),
        );
    }
    let width = CARD_WIDTH.min(screen.width() - 32.0);
    let card = Rect::from_center_size(c + Vec2::new(0.0, 34.0), Vec2::new(width, CARD_HEIGHT));
    let visuals = &ctx.global_style().visuals;
    let radius = visuals.window_corner_radius;
    let mut shadow = visuals.window_shadow;
    shadow.color = shadow.color.gamma_multiply(a);
    painter.add(shadow.as_shape(card, radius));
    // What is behind is already blurred (the last view) or soft (the aurora):
    // a lighter tint than the windows' lets it show through, the windows' glass
    // without their blur pass. An opaque skin stays opaque.
    let tint = if p.panel_alpha < 255 {
        (f32::from(p.panel_alpha) * 0.78) as u8
    } else {
        255
    };
    let fill = Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), tint);
    painter.rect_filled(card, radius, fill.gamma_multiply(a));
    painter.rect_stroke(
        card,
        radius,
        egui::Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6 * a)),
        egui::StrokeKind::Inside,
    );
    let inner = card.shrink(CARD_MARGIN);
    let text = painter.with_clip_rect(inner.expand(2.0));
    // title, with the progress in figures at the right
    let mut title_right = inner.right();
    if let Some(f) = fraction {
        let percent = aurora_i18n::number_text(&format!("{:.0}", f.clamp(0.0, 1.0) * 100.0));
        let r = text.text(
            Pos2::new(inner.right(), inner.top() + 11.0),
            egui::Align2::RIGHT_CENTER,
            format!("{percent} %"),
            egui::FontId::proportional(13.0),
            p.muted.gamma_multiply(a),
        );
        title_right = r.left() - 10.0;
    }
    let clip = Rect::from_min_max(card.left_top(), Pos2::new(title_right, card.bottom()));
    painter.with_clip_rect(clip).text(
        Pos2::new(inner.left(), inner.top() + 11.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(17.0),
        p.ink.gamma_multiply(a),
    );
    // the place, with its pin
    let line = inner.top() + 36.0;
    let mut x = inner.left();
    if !place.is_empty() {
        if let Some(pin) = super::icons::global("map-pin-filled") {
            let r = Rect::from_center_size(Pos2::new(x + 7.0, line), Vec2::splat(14.0));
            painter.image(
                pin.id(),
                r,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                p.violet_light.gamma_multiply(a),
            );
            x += 20.0;
        }
        text.text(
            Pos2::new(x, line),
            egui::Align2::LEFT_CENTER,
            place,
            egui::FontId::proportional(13.0),
            p.violet_light.gamma_multiply(a),
        );
    }
    let bar = Rect::from_min_size(Pos2::new(inner.left(), inner.top() + 56.0), Vec2::new(inner.width(), 4.0));
    paint_progress(painter, p, bar, fraction, t, a);
    if !detail.is_empty() {
        text.text(
            Pos2::new(inner.left(), bar.bottom() + 15.0),
            egui::Align2::LEFT_CENTER,
            detail,
            egui::FontId::proportional(12.0),
            p.muted.gamma_multiply(a),
        );
    }
    Pos2::new(c.x, card.bottom() + 14.0)
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
        aurora_i18n::tr!(
            "status-terrain-objects-textures",
            value0 = info.stage.to_string(),
            value1 = aurora_i18n::number_text(&format!("{:.0}", info.terrain * 100.0)),
            value2 = format!("{}", info.objects),
            value3 = format!("{}", info.textures.0),
            value4 = format!("{}", info.textures.1)
        )
    };
    let below = splash(
        &ctx,
        &painter,
        p,
        screen,
        logo,
        aurora_i18n::tr!("common-loading-world"),
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
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(below)
            .show(&ctx, |ui| {
                let label = aurora_i18n::tr!("common-cancel");
                let kind = super::widgets::ActionKind::Flat;
                if super::widgets::action_button(ui, p, "x", label, kind, Vec2::new(0.0, 28.0)).clicked() {
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
            splash(ctx, &painter, p, r, logo, title, dest, fraction, detail, t, a);
        });
    ctx.request_repaint();
}
