//! The region restart alert, as the human chose it in the animated mockup
//! (variant B, "alert card"): Firestorm's box (floater_region_restarting.xml)
//! redrawn as a card in the top third of the view, big figures, a bar that
//! empties and a border that breathes faster as the time runs out; a pill
//! when minimized; Home, the landmarks, the neighbour region and the last
//! place to leave; "Cancel the restart" for the region's managers. Firestorm's
//! yellow box itself is the legacy look (`legacy_box`).
//!
//! Nothing flashes: the slowest breath is 5 s, the fastest one a second (the
//! limit is three flashes a second), and "Reduce animations" stops them all,
//! with the shakes, the drop and the spinner.
//!
//! The rules (steps, sounds, when it comes back) are in
//! `world/region_restart.rs`; the app drives both (`app/region_restart.rs`).

use crate::theme::Palette;
use crate::ui::{menu, widgets};
use crate::world::region_restart::{Level, Phase, Restart};
use egui::emath::easing;
use egui::epaint::{PathShape, PathStroke};
use egui::{Align2, Color32, FontId, Id, Margin, Pos2, Rect, Sense, Shape, Stroke, Vec2};
use uuid::Uuid;

/// The card's width and where it stands: centered, its top at 17 % of the
/// view (the mockup's `.rc` and `.alert-layer.vB`).
pub const WIDTH: f32 = 380.0;
const TOP: f32 = 0.17;
/// The landmark menu's width (`.lm`).
const MENU_W: f32 = 330.0;

/// The breath of the card's border by step: 5 s, 3 s, 1.6 s then 1 s
/// (`.rc.lvl-*::after`).
fn card_period(level: Level) -> Option<f64> {
    match level {
        Level::Calm => Some(5.0),
        Level::Near => Some(3.0),
        Level::Red => Some(1.6),
        Level::Final => Some(1.0),
        Level::Zero => None,
    }
}

/// The pill's: still at first, then 4 s, 2 s, 1 s (`.rpill.lvl-*::after`).
fn pill_period(level: Level) -> Option<f64> {
    match level {
        Level::Calm | Level::Zero => None,
        Level::Near => Some(4.0),
        Level::Red => Some(2.0),
        Level::Final => Some(1.0),
    }
}

/// The color of the alert: amber, orange, red, then grey at 0; green when
/// cancelled, teal once left (`.lvl-*`).
pub fn color(p: &Palette, a: &Restart) -> Color32 {
    match a.phase {
        Phase::Cancelled => p.success,
        Phase::Left { .. } => p.teal,
        Phase::Live => match a.level() {
            Level::Calm => p.amber,
            Level::Near => p.warn,
            Level::Red | Level::Final => p.danger,
            Level::Zero => p.muted,
        },
    }
}

/// The time shown: "4:59", "52 s", then the bare seconds of the final count
/// (the mockup's `timeTxt`).
pub fn time_text(seconds: i32) -> String {
    match seconds {
        s if s <= 0 => "0".to_owned(),
        s if s <= 10 => s.to_string(),
        s if s < 60 => aurora_i18n::tr!("region-restart-seconds-short", seconds = s.to_string()),
        s => format!("{}:{:02}", s / 60, s % 60),
    }
}

/// "until the restart", or "seconds" for the final count.
fn unit(seconds: i32) -> &'static str {
    if seconds <= 10 {
        aurora_i18n::tr!("region-restart-seconds")
    } else {
        aurora_i18n::tr!("region-restart-before")
    }
}

/// French elides "de" before a vowel ("d'Aurora", "de Lune"): the catalogs
/// select on it.
fn elide(region: &str) -> &'static str {
    let first = region.chars().next().map(|c| c.to_lowercase().next().unwrap_or(c));
    if first.is_some_and(|c| "aeiouyàâäéèêëîïôöùûüœæ".contains(c)) {
        "yes"
    } else {
        "no"
    }
}

/// The title and the text of the alert at its step (the mockup's `words`).
pub fn words(a: &Restart) -> (String, String) {
    let region = a.region.as_str();
    let e = elide(region);
    match &a.phase {
        Phase::Cancelled => (
            aurora_i18n::tr!("region-restart-cancelled-title").to_owned(),
            aurora_i18n::tr!("region-restart-cancelled-text", region = region, elide = e),
        ),
        Phase::Left { arrival } => (
            aurora_i18n::tr!("region-restart-left-title", region = region),
            aurora_i18n::tr!("region-restart-left-text", arrival = arrival.as_str()),
        ),
        Phase::Live => match a.level() {
            Level::Calm => (
                aurora_i18n::tr!("region-restart-calm-title", region = region),
                aurora_i18n::tr!("region-restart-calm-text").to_owned(),
            ),
            Level::Near => (
                aurora_i18n::tr!("region-restart-near-title").to_owned(),
                aurora_i18n::tr!("region-restart-near-text", region = region, elide = e),
            ),
            Level::Red => (
                aurora_i18n::tr!("region-restart-red-title").to_owned(),
                aurora_i18n::tr!("region-restart-red-text", region = region),
            ),
            Level::Final => (
                aurora_i18n::tr!("region-restart-final-title").to_owned(),
                aurora_i18n::tr!("region-restart-final-text", region = region),
            ),
            Level::Zero => (
                aurora_i18n::tr!("region-restart-zero-title", region = region),
                aurora_i18n::tr!("region-restart-zero-text").to_owned(),
            ),
        },
    }
}

/// A landmark of the inventory offered to leave.
#[derive(Debug, Clone, PartialEq)]
pub struct Landmark {
    pub asset: Uuid,
    pub name: String,
    /// Where it leads, when known.
    pub detail: String,
    /// Creation date (the recent ones first).
    pub created: i64,
}

/// Where the alert offers to go.
#[derive(Debug, Clone, Default)]
pub struct Destinations {
    pub landmarks: Vec<Landmark>,
    /// The closest neighbour region one may enter.
    pub neighbour: Option<String>,
    /// The last place of the teleport history outside this region.
    pub last_place: Option<String>,
}

/// The landmarks the menu lists: the three most recent, or those whose name
/// holds `query` (eight at most).
pub fn listed<'a>(landmarks: &'a [Landmark], query: &str) -> Vec<&'a Landmark> {
    let q = query.trim().to_lowercase();
    let mut v: Vec<&Landmark> = landmarks
        .iter()
        .filter(|l| q.is_empty() || l.name.to_lowercase().contains(&q))
        .collect();
    if q.is_empty() {
        v.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
        v.truncate(3);
    } else {
        v.sort_by_key(|l| (!l.name.to_lowercase().starts_with(&q), l.name.to_lowercase()));
        v.truncate(8);
    }
    v
}

/// What a click on the alert asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Home,
    Landmark(Uuid),
    Neighbour,
    LastPlace,
    /// "All landmarks…": the Places window.
    AllLandmarks,
    Minimize,
    Expand,
    Close,
    /// "Cancel the restart", confirmed.
    CancelRestart,
    /// "Region / Estate ›".
    RegionEstate,
}

/// What the frame tells the alert.
pub struct View<'a> {
    pub alert: &'a Restart,
    /// We manage the region's estate (LLViewerRegion::canManageEstate).
    pub manager: bool,
    /// Do not disturb, mouselook, the photo's full frame: a pill only.
    pub pill_only: bool,
    /// "Reduce animations".
    pub still: bool,
    /// The demo's frozen pulse, for screenshots (the mockup's still states).
    pub frozen: bool,
    pub destinations: &'a Destinations,
}

/// What the alert remembers from one frame to the next.
#[derive(Default)]
pub struct RestartUi {
    /// When it dropped in (220 ms) and when it trembles (700 ms).
    enter_at: Option<f64>,
    shake_at: Option<f64>,
    query: String,
    confirm: bool,
    /// The landmark menu, opened by the demo for its screenshots.
    pub open_menu: bool,
    /// The landmark menu is open (its button stays lit).
    menu_open: bool,
    /// Expanded by a click on the pill in do not disturb.
    expanded_in_dnd: bool,
    /// The alert's layer, for the order of the notification layers.
    pub layer: Option<egui::LayerId>,
}

impl RestartUi {
    /// What the landmark menu searches.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The alert opens: it drops in, then trembles (the mockup's
    /// `dropIn 220ms, shakeX 700ms 220ms`).
    pub fn opened(&mut self, now: f64) {
        self.enter_at = Some(now);
        self.shake_at = Some(now + DROP);
        self.query.clear();
        self.expanded_in_dnd = false;
    }

    pub fn shake(&mut self, now: f64) {
        self.shake_at = Some(now);
    }

    /// Something moves: a repaint is needed next frame.
    fn moving(&self, now: f64) -> bool {
        self.enter_at.is_some_and(|t| now - t < DROP) || self.shake_at.is_some_and(|t| now - t < SHAKE)
    }
}

const DROP: f64 = 0.22;
const SHAKE: f64 = 0.7;

/// The tremble of the alert (`@keyframes shakeX`, each step eased out).
fn shake_x(t: f64) -> f32 {
    const KEYS: [(f32, f32); 8] = [
        (0.0, 0.0),
        (0.12, -7.0),
        (0.26, 6.0),
        (0.40, -4.0),
        (0.55, 2.5),
        (0.70, -1.2),
        (0.85, 0.6),
        (1.0, 0.0),
    ];
    let t = (t / SHAKE) as f32;
    if !(0.0..1.0).contains(&t) {
        return 0.0;
    }
    for w in KEYS.windows(2) {
        let ((t0, x0), (t1, x1)) = (w[0], w[1]);
        if t <= t1 {
            let f = easing::quadratic_out((t - t0) / (t1 - t0));
            return x0 + (x1 - x0) * f;
        }
    }
    0.0
}

/// The breath of a border (`@keyframes glow`): 0 at rest, 1 at its peak,
/// eased in and out.
fn breath(now: f64, period: f64) -> f32 {
    let t = (now / period).fract() as f32;
    let half = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    easing::quadratic_in_out(half)
}

/// The border of the alert: its inset line (0.35 → 0.75) and the glow around
/// it (0 → 0.34, 26 px), at `v` of the breath; without a breath, the still
/// line (0.45).
fn paint_border(painter: &egui::Painter, rect: Rect, radius: egui::CornerRadius, lv: Color32, v: Option<f32>) -> Vec<Shape> {
    let (line, glow) = match v {
        Some(v) => (0.35 + 0.4 * v, 0.34 * v),
        None => (0.45, 0.0),
    };
    let mut out = Vec::new();
    if glow > 0.0 {
        let shadow = egui::Shadow {
            offset: [0, 0],
            blur: 26,
            spread: 3,
            color: lv.gamma_multiply(glow),
        };
        out.push(Shape::Rect(shadow.as_shape(rect, radius)));
    }
    painter.rect_stroke(rect, radius, Stroke::new(1.0, lv.gamma_multiply(line)), egui::StrokeKind::Inside);
    out
}

/// The level's tint over the glass: strong at the top, faint from the middle
/// (`linear-gradient(lv .14, lv .02 55%)`).
fn tint(rect: Rect, lv: Color32, top: f32, bottom: f32) -> Shape {
    use egui::epaint::{Mesh, Vertex, WHITE_UV};
    let mut m = Mesh::default();
    let rows = [(0.0, top), (0.55, bottom), (1.0, bottom)];
    for (y, a) in rows {
        let c = lv.gamma_multiply(a);
        let yy = rect.top() + rect.height() * y;
        m.vertices.push(Vertex {
            pos: Pos2::new(rect.left(), yy),
            uv: WHITE_UV,
            color: c,
        });
        m.vertices.push(Vertex {
            pos: Pos2::new(rect.right(), yy),
            uv: WHITE_UV,
            color: c,
        });
    }
    for i in 0..2u32 {
        let k = i * 2;
        m.add_triangle(k, k + 1, k + 2);
        m.add_triangle(k + 1, k + 3, k + 2);
    }
    Shape::mesh(m)
}

/// The alert: the card, or its pill. Returns what was clicked.
pub fn show(ctx: &egui::Context, p: &Palette, st: &mut RestartUi, v: &View<'_>) -> Vec<Action> {
    let mut out = Vec::new();
    let a = v.alert;
    if a.closed {
        st.layer = None;
        return out;
    }
    let now = ctx.input(|i| i.time);
    if !v.pill_only {
        st.expanded_in_dnd = false;
    }
    let pill = a.live() && (a.minimized || (v.pill_only && !st.expanded_in_dnd));
    let screen = ctx.content_rect();
    let width = WIDTH.min(screen.width() - 16.0);
    let top = screen.top() + screen.height() * TOP;
    // dropped in, trembling, fading out
    let (mut dy, mut alpha, mut dx) = (0.0, 1.0, 0.0);
    if !v.still {
        if let Some(t) = st.enter_at.map(|t| now - t).filter(|t| (0.0..DROP).contains(t)) {
            let f = easing::quadratic_out((t / DROP) as f32);
            dy = -14.0 * (1.0 - f);
            alpha = f;
        }
        if !pill && let Some(t) = st.shake_at.map(|t| now - t) {
            dx = shake_x(t);
        }
        let fade = a.fade();
        if fade > 0.0 {
            let f = easing::quadratic_in(fade);
            alpha *= 1.0 - f;
            dy -= 8.0 * f;
        }
    } else if a.fade() > 0.0 {
        alpha = 1.0 - a.fade();
    }
    let lv = color(p, a);
    let level = a.level();
    let period = if pill { pill_period(level) } else { card_period(level) };
    let breathing = match a.phase {
        Phase::Live if !v.still => period.map(|t| if v.frozen { 0.875 } else { breath(now, t) }),
        _ => None,
    };
    // the mockup's still states leave the first step at rest
    let breathing = if v.frozen && level == Level::Calm { None } else { breathing };
    let area = egui::Area::new(Id::unique("region_restart_alert"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(Pos2::new(screen.center().x + dx, top + dy))
        .constrain(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(alpha);
            if pill {
                pill_view(ui, p, a, lv, breathing, &mut out);
            } else {
                card(ui, p, st, v, width, lv, breathing, now, &mut out);
            }
        });
    st.layer = Some(area.response.layer_id);
    if out.contains(&Action::Expand) && v.pill_only {
        st.expanded_in_dnd = true;
    }
    if st.confirm {
        confirm_cancel(ctx, p, st, a, &mut out);
    }
    if out.contains(&Action::Expand) {
        st.enter_at = Some(now);
    }
    if !v.still && (breathing.is_some() || st.moving(now) || a.fade() > 0.0 || (a.level() == Level::Zero && !pill)) {
        ctx.request_repaint();
    }
    out
}

/// The ring of the pill: it empties over the whole count, fills again for
/// the last minute, then turns into ten notches that go out one by one.
fn ring(painter: &egui::Painter, center: Pos2, size: f32, a: &Restart, p: &Palette, lv: Color32) {
    let sw = if size > 30.0 { 4.0 } else { 3.0 };
    let r = (size - sw) / 2.0;
    painter.circle_stroke(center, r, Stroke::new(sw, p.raised));
    let arc = |from: f32, to: f32, color: Color32| {
        let steps = ((to - from).abs() * 24.0).ceil().max(2.0) as usize;
        let points: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let t = from + (to - from) * i as f32 / steps as f32;
                // from the top, clockwise
                let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                center + Vec2::new(angle.cos(), angle.sin()) * r
            })
            .collect();
        Shape::Path(PathShape::line(points, PathStroke::new(sw, color)))
    };
    let secs = a.seconds();
    match a.level() {
        Level::Final => {
            let gap = 0.03;
            for i in 0..10 {
                let start = i as f32 / 10.0 + gap / 2.0;
                let color = if i < secs { lv } else { p.raised };
                painter.add(arc(start, start + 0.1 - gap, color));
            }
        }
        Level::Zero => {}
        _ => {
            let frac = if secs > 60 {
                a.fraction()
            } else {
                (a.seconds() as f32 / 60.0).clamp(0.0, 1.0)
            };
            if frac > 0.0 {
                painter.add(arc(0.0, frac, lv));
            }
        }
    }
}

/// The reduced alert: the ring, the time, "<region> restarts", a chevron;
/// a click opens the card again (`.rpill`).
fn pill_view(ui: &mut egui::Ui, p: &Palette, a: &Restart, lv: Color32, breathing: Option<f32>, out: &mut Vec<Action>) {
    let glow_slot = ui.painter().add(Shape::Noop);
    let mut tint_slot = None;
    let frame = widgets::overlay(
        ui,
        p,
        Margin {
            left: 5,
            right: 10,
            top: 4,
            bottom: 4,
        },
        |ui| {
            tint_slot = Some(ui.painter().add(Shape::Noop));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
                ring(ui.painter(), r.center(), 24.0, a, p, lv);
                let time = time_text(a.seconds());
                let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
                let g = ui.painter().layout_no_wrap(time, font, lv);
                let (tr, _) = ui.allocate_exact_size(Vec2::new(g.size().x.max(30.0), 24.0), Sense::hover());
                let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, tr.left(), tr.center().y);
                ui.painter().galley(at, g, lv);
                let label = aurora_i18n::tr!("region-restart-zero-title", region = a.region.as_str());
                let g = ui.painter().layout_no_wrap(label, FontId::proportional(12.0), p.muted);
                let (lr, _) = ui.allocate_exact_size(Vec2::new(g.size().x, 24.0), Sense::hover());
                let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, lr.left(), lr.center().y);
                ui.painter().galley(at, g, p.muted);
                let (cr, _) = ui.allocate_exact_size(Vec2::new(13.0, 24.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    "chevron-down",
                    Rect::from_center_size(cr.center(), Vec2::splat(13.0)),
                    p.muted,
                );
            });
        },
    );
    let rect = frame.response.rect;
    let radius = ui.visuals().window_corner_radius;
    if let Some(slot) = tint_slot {
        ui.painter().set(slot, tint(rect.shrink(1.0), lv, 0.16, 0.05));
    }
    let glow = paint_border(ui.painter(), rect, radius, lv, breathing);
    ui.painter().set(glow_slot, Shape::Vec(glow));
    let click = ui
        .interact(rect, Id::unique("region_restart_pill"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(aurora_i18n::tr!("region-restart-show"));
    if click.clicked() {
        out.push(Action::Expand);
    }
}

fn paint_icon(painter: &egui::Painter, icon: &str, rect: Rect, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(icon) {
        painter.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    }
}

/// A button of the card's head: the minimize and close glyphs, a face when
/// hovered.
fn head_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 26.0), Sense::click());
    let hot = resp.hovered();
    if hot {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    paint_icon(
        ui.painter(),
        icon,
        Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        if hot { p.ink } else { p.muted },
    );
    resp.on_hover_text(tip).clicked()
}

/// Where the baseline sits in a line of the interface font, from its top.
const BASELINE: f32 = 0.78;

/// Big figures whose digits keep one width (`font-variant-numeric:
/// tabular-nums`): the unit after them does not move each second.
fn tabular(ui: &egui::Ui, text: &str, font: &FontId, color: Color32) -> (Vec<(f32, std::sync::Arc<egui::Galley>)>, f32) {
    let painter = ui.painter();
    let cell = ('0'..='9')
        .map(|d| painter.layout_no_wrap(d.to_string(), font.clone(), color).size().x)
        .fold(0.0_f32, f32::max);
    let mut x = 0.0;
    let mut parts = Vec::new();
    for c in text.chars() {
        let g = painter.layout_no_wrap(c.to_string(), font.clone(), color);
        let w = if c.is_ascii_digit() { cell } else { g.size().x };
        parts.push((x + (w - g.size().x) / 2.0, g));
        x += w;
    }
    (parts, x)
}

/// The card (`.rc`).
#[allow(clippy::too_many_arguments)]
fn card(
    ui: &mut egui::Ui,
    p: &Palette,
    st: &mut RestartUi,
    v: &View<'_>,
    width: f32,
    lv: Color32,
    breathing: Option<f32>,
    now: f64,
    out: &mut Vec<Action>,
) {
    let a = v.alert;
    let live = a.live();
    let manager = v.manager && live;
    let glow_slot = ui.painter().add(Shape::Noop);
    let mut tint_slot = None;
    let margin = Margin {
        left: 14,
        right: 14,
        top: 12,
        bottom: if manager { 0 } else { 14 },
    };
    let frame = widgets::overlay(ui, p, margin, |ui| {
        tint_slot = Some(ui.painter().add(Shape::Noop));
        let inner = width - 28.0 - 2.0;
        ui.set_width(inner);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        // head: the step's icon, the title and the region, minimize and close
        ui.horizontal(|ui| {
            ui.set_height(34.0);
            let (icon, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
            ui.painter()
                .rect_filled(icon, ui.visuals().window_corner_radius, lv.gamma_multiply(0.14));
            let (name, spin) = match (&a.phase, a.level()) {
                (Phase::Live, Level::Zero) => ("loader-2", true),
                (Phase::Live, _) => ("alert-triangle-filled", false),
                _ => ("circle-check-filled", false),
            };
            if let Some(t) = crate::ui::icons::global(name) {
                let angle = if spin && !v.still {
                    (now / 2.4).fract() as f32 * std::f32::consts::TAU
                } else {
                    0.0
                };
                egui::Image::new(&t)
                    .tint(lv)
                    .rotate(angle, Vec2::splat(0.5))
                    .paint_at(ui, Rect::from_center_size(icon.center(), Vec2::splat(19.0)));
            }
            ui.add_space(10.0);
            let (title, _) = words(a);
            let title = if live {
                aurora_i18n::tr!("region-restart-title").to_owned()
            } else {
                title
            };
            let room = ui.available_width() - if live { 56.0 } else { 28.0 };
            ui.allocate_ui_with_layout(Vec2::new(room, 34.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add_space(1.0);
                let font = FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx()));
                ui.add(egui::Label::new(egui::RichText::new(title).font(font).color(p.ink)).truncate());
                ui.add(egui::Label::new(egui::RichText::new(&a.region).size(12.0).color(p.muted)).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if head_button(ui, p, "x", aurora_i18n::tr!("common-close")) {
                    out.push(Action::Close);
                }
                if live && head_button(ui, p, "minus", aurora_i18n::tr!("region-restart-minimize")) {
                    out.push(Action::Minimize);
                }
            });
        });
        let (title, text) = words(a);
        if live {
            // the count: big figures and their unit, on one baseline
            ui.add_space(12.0);
            let secs = a.seconds();
            let big = FontId::new(38.0, crate::ui::fonts::semibold(ui.ctx()));
            let (parts, w) = tabular(ui, &time_text(secs), &big, lv);
            let unit = ui
                .painter()
                .layout_no_wrap(unit(secs).to_owned(), FontId::proportional(12.5), p.muted);
            // a line as high as the figures (`line-height: 1`)
            let h = parts.first().map_or(38.0, |(_, g)| g.size().y);
            let (row, _) = ui.allocate_exact_size(Vec2::new(inner, 38.0), Sense::hover());
            let top = row.top() - (h - 38.0) / 2.0;
            for (x, g) in parts {
                ui.painter().galley(Pos2::new(row.left() + x, top), g, lv);
            }
            // the unit's baseline on the figures' one
            let base = top + h * BASELINE;
            ui.painter()
                .galley(Pos2::new(row.left() + w + 10.0, base - unit.size().y * BASELINE), unit, p.muted);
            ui.add_space(6.0);
            widgets::countdown_bar(ui, p, "region_restart_bar", a.fraction(), inner, lv);
        }
        // what it means
        ui.add_space(10.0);
        let line = if live { format!("{title}. {text}") } else { text };
        let mut job = egui::text::LayoutJob::single_section(
            line,
            egui::TextFormat {
                font_id: FontId::proportional(12.5),
                color: p.muted,
                line_height: Some(12.5 * 1.45 * super::fonts::scale(ui.ctx())),
                ..Default::default()
            },
        );
        job.wrap.max_width = inner;
        let g = ui.painter().layout_job(job);
        let (r, _) = ui.allocate_exact_size(Vec2::new(inner, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, p.muted);
        ui.add_space(12.0);
        if live {
            actions(ui, p, st, v, out);
        }
        if manager {
            manager_row(ui, p, st, inner, out);
        }
    });
    let rect = frame.response.rect;
    let radius = ui.visuals().window_corner_radius;
    if let Some(slot) = tint_slot {
        ui.painter().set(slot, tint(rect.shrink(1.0), lv, 0.14, 0.02));
    }
    let glow = paint_border(ui.painter(), rect, radius, lv, breathing);
    ui.painter().set(glow_slot, Shape::Vec(glow));
}

/// Home and the landmark menu (`.rc-acts`).
fn actions(ui: &mut egui::Ui, p: &Palette, st: &mut RestartUi, v: &View<'_>, out: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if widgets::action_button(
            ui,
            p,
            "home-filled",
            aurora_i18n::tr!("common-home"),
            widgets::ActionKind::Primary,
            Vec2::new(0.0, 27.0),
        )
        .clicked()
        {
            out.push(Action::Home);
        }
        let resp = widgets::action_menu_button(ui, p, "map-pin-filled", aurora_i18n::tr!("common-landmark"), st.menu_open, 27.0);
        let popup = egui::Popup::default_response_id(&resp);
        if st.open_menu {
            st.open_menu = false;
            egui::Popup::open_id(ui.ctx(), popup);
        }
        st.menu_open = menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            landmark_menu(ui, p, st, v, out);
        });
    });
}

/// The landmark menu (`.lm`): search, recent landmarks, the neighbour region,
/// the last place, all the landmarks.
fn landmark_menu(ui: &mut egui::Ui, p: &Palette, st: &mut RestartUi, v: &View<'_>, out: &mut Vec<Action>) {
    ui.set_width(MENU_W - 8.0);
    ui.add_space(1.0);
    // the search, its glass in the field's face (`.lm .srch`)
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(Vec2::new(13.0, 20.0), Sense::hover());
                paint_icon(
                    ui.painter(),
                    "search-filled",
                    Rect::from_center_size(r.center(), Vec2::splat(13.0)),
                    p.muted_dim,
                );
                ui.add(
                    egui::TextEdit::singleline(&mut st.query)
                        .frame(egui::Frame::NONE.inner_margin(crate::ui::widgets::field_text_margin(
                            ui.ctx(),
                            &egui::TextStyle::Body.resolve(ui.style()),
                        )))
                        .hint_text(aurora_i18n::tr!("region-restart-search"))
                        .desired_width(f32::INFINITY),
                );
            });
        });
    ui.add_space(4.0);
    let d = v.destinations;
    let found = listed(&d.landmarks, &st.query);
    if st.query.trim().is_empty() {
        menu::section(ui, p, aurora_i18n::tr!("region-restart-recent"));
    }
    if found.is_empty() {
        menu::info(ui, p, "map-pin-filled", aurora_i18n::tr!("region-restart-no-landmark"));
    }
    for l in found {
        if menu::item_detail(ui, p, "map-pin-filled", &l.name, &l.detail) {
            out.push(Action::Landmark(l.asset));
        }
    }
    menu::separator(ui, p);
    if let Some(n) = &d.neighbour
        && menu::item_tag(
            ui,
            p,
            "map-2",
            &aurora_i18n::tr!("region-restart-neighbour", region = n.as_str()),
            aurora_i18n::tr!("region-restart-suggestion"),
        )
    {
        out.push(Action::Neighbour);
    }
    if let Some(place) = &d.last_place
        && menu::item(
            ui,
            p,
            "history",
            &aurora_i18n::tr!("region-restart-last-place", place = place.as_str()),
        )
    {
        out.push(Action::LastPlace);
    }
    if menu::item(ui, p, "folder-filled", aurora_i18n::tr!("region-restart-all-landmarks")) {
        out.push(Action::AllLandmarks);
    }
}

/// The managers' line (`.rb-mgr`): "Cancel the restart" and the link to
/// Region / Estate.
fn manager_row(ui: &mut egui::Ui, p: &Palette, st: &mut RestartUi, inner: f32, out: &mut Vec<Action>) {
    ui.add_space(12.0);
    let (line, _) = ui.allocate_exact_size(Vec2::new(inner, 1.0), Sense::hover());
    // across the whole card, from edge to edge
    ui.painter().hline(
        (line.left() - 14.0)..=(line.right() + 14.0),
        line.center().y,
        widgets::divider_stroke(p),
    );
    ui.add_space(7.0);
    // who we are here, and the cancel button at the right end
    ui.allocate_ui_with_layout(Vec2::new(inner, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
        let (r, _) = ui.allocate_exact_size(Vec2::new(15.0, 22.0), Sense::hover());
        paint_icon(
            ui.painter(),
            "building-castle",
            Rect::from_center_size(r.center(), Vec2::splat(15.0)),
            p.violet_light,
        );
        let label = ui.painter().layout_no_wrap(
            aurora_i18n::tr!("region-restart-manager").to_owned(),
            FontId::proportional(12.0),
            p.muted,
        );
        let (r, _) = ui.allocate_exact_size(Vec2::new(label.size().x, 22.0), Sense::hover());
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &label, r.left(), r.center().y);
        ui.painter().galley(at, label, p.muted);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button(
                ui,
                p,
                "refresh-off",
                aurora_i18n::tr!("region-restart-cancel"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 22.0),
            )
            .clicked()
            {
                st.confirm = true;
            }
        });
    });
    ui.add_space(8.0);
    // the link to the window where restarts are scheduled
    ui.allocate_ui_with_layout(Vec2::new(inner, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        let link = aurora_i18n::tr!("region-restart-region-estate");
        let g = ui.painter().layout_no_wrap(
            link.to_owned(),
            FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx())),
            p.violet_light,
        );
        let (r, resp) = ui.allocate_exact_size(Vec2::new(g.size().x, 16.0), Sense::click());
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, r.left(), r.center().y);
        ui.painter().galley(at, g, p.violet_light);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            out.push(Action::RegionEstate);
        }
    });
    ui.add_space(8.0);
}

/// "Cancel the restart?": the manager confirms before the estate request
/// (Firestorm's button sends it at once).
fn confirm_cancel(ctx: &egui::Context, p: &Palette, st: &mut RestartUi, a: &Restart, out: &mut Vec<Action>) {
    let mut close = false;
    let response = widgets::Modal::new(Id::unique("region_restart_cancel")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("region-restart-cancel-confirm-title"))
                .size(14.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("region-restart-cancel-confirm-text", region = a.region.as_str()))
                .size(12.5)
                .color(p.muted),
        );
        ui.add_space(14.0);
        let buttons = [
            widgets::DialogButton {
                icon: "",
                label: aurora_i18n::tr!("region-restart-keep"),
                kind: widgets::ActionKind::Flat,
                enabled: true,
            },
            widgets::DialogButton {
                icon: "refresh-off",
                label: aurora_i18n::tr!("region-restart-cancel"),
                kind: widgets::ActionKind::Danger,
                enabled: true,
            },
        ];
        match widgets::dialog_buttons(ui, p, &buttons, 28.0) {
            Some(1) => {
                out.push(Action::CancelRestart);
                close = true;
            }
            Some(_) => close = true,
            None => {}
        }
    });
    if close || response.should_close() || !a.live() {
        st.confirm = false;
    }
}

/// What the bell's card shows of the restart (`restartCard`).
#[derive(Debug, Clone)]
pub struct BellInfo {
    pub region: String,
    pub seconds: i32,
    pub color: Color32,
}

/// The restart's card in Aurora's center, pinned at the top of "To do": the
/// region and the live count, Home, "Show the alert". Accept goes home, Open
/// shows the alert.
pub fn bell_card(ui: &mut egui::Ui, p: &Palette, info: &BellInfo) -> Option<crate::world::notifications::Response> {
    use crate::world::notifications::Response;
    let mut out = None;
    let lv = info.color;
    let frame = egui::Frame::new()
        .fill(p.field)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(Margin {
            left: 10,
            right: 10,
            top: 9,
            bottom: 9,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                ui.painter()
                    .rect_filled(icon, ui.visuals().window_corner_radius, lv.gamma_multiply(0.16));
                paint_icon(
                    ui.painter(),
                    "alert-triangle-filled",
                    Rect::from_center_size(icon.center(), Vec2::splat(15.0)),
                    lv,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let font = FontId::new(13.0, crate::ui::fonts::semibold(ui.ctx()));
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("region-restart-title"))
                                .font(font)
                                .color(p.ink),
                        );
                        let (r, _) = ui.allocate_exact_size(Vec2::new(11.0, 16.0), Sense::hover());
                        paint_icon(
                            ui.painter(),
                            "pin-filled",
                            Rect::from_center_size(r.center(), Vec2::splat(10.5)),
                            p.muted_dim,
                        );
                        ui.add_space(-3.0);
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("region-restart-pinned"))
                                .size(10.5)
                                .strong()
                                .color(p.muted_dim),
                        );
                    });
                    let mut job = egui::text::LayoutJob::default();
                    let f = |c| egui::TextFormat::simple(FontId::proportional(12.0), c);
                    job.append(&format!("{} · ", info.region), 0.0, f(p.muted));
                    job.append(
                        &time_text(info.seconds),
                        0.0,
                        egui::TextFormat::simple(FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx())), lv),
                    );
                    job.append(&format!(" {}", unit(info.seconds)), 0.0, f(p.muted));
                    ui.label(job);
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if widgets::action_button(
                            ui,
                            p,
                            "home-filled",
                            aurora_i18n::tr!("common-home"),
                            widgets::ActionKind::Primary,
                            Vec2::new(0.0, 24.0),
                        )
                        .clicked()
                        {
                            out = Some(Response::Accept);
                        }
                        if widgets::action_button(
                            ui,
                            p,
                            "",
                            aurora_i18n::tr!("region-restart-show"),
                            widgets::ActionKind::Flat,
                            Vec2::new(0.0, 24.0),
                        )
                        .clicked()
                        {
                            out = Some(Response::Open);
                        }
                    });
                });
            });
        });
    // the step's color along the left edge (`.card.rs`)
    let r = frame.response.rect;
    ui.painter()
        .rect_filled(Rect::from_min_max(r.left_top(), Pos2::new(r.left() + 2.0, r.bottom())), 0.0, lv);
    out
}

/// Firestorm's box, the legacy look (floater_region_restarting.xml, 290 ×
/// 200): the yellow panel, the caution icon, the region's sentence, the
/// seconds left in large type, "My Landmarks" and the button that teleports
/// there. A window like any other, centered when it opens.
pub fn legacy_box(
    ctx: &egui::Context,
    p: &Palette,
    a: &Restart,
    landmarks: &[Landmark],
    pick: &mut Option<Uuid>,
    open: &mut bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let size = Vec2::new(290.0, 200.0);
    let pos = ctx.content_rect().center() - size / 2.0;
    widgets::Floater::new("region_restarting", aurora_i18n::tr!("region-restart-title"), pos, size)
        .fixed()
        .show(ctx, p, open, |ui| {
            // the panel under the title: 181 px high in Firestorm
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 181.0), Sense::hover());
            // theme-guard: content — Firestorm's "Orange" panel (colors.xml, 1 .82 .46) and its black text
            let (orange, black) = (Color32::from_rgb(255, 209, 117), Color32::from_rgb(0, 0, 0));
            ui.painter().rect_filled(rect, 0.0, orange);
            // notify_caution_icon.tga: a yellow triangle, its outline and its
            // mark dark (the dark triangle shows through the yellow one's mark)
            // theme-guard: content — the yellow and the dark brown of Firestorm's caution icon
            let (caution, edge) = (Color32::from_rgb(255, 225, 77), Color32::from_rgb(58, 46, 0));
            let icon = Rect::from_min_size(rect.min + Vec2::new(4.0, 4.0), Vec2::splat(32.0));
            paint_icon(ui.painter(), "alert-triangle-filled", icon, edge);
            paint_icon(
                ui.painter(),
                "alert-triangle-filled",
                icon.shrink(2.5).translate(Vec2::new(0.0, 0.8)),
                caution,
            );
            let text = aurora_i18n::tr!("region-restart-legacy-text", region = a.region.as_str());
            let g = ui.painter().layout(
                text,
                FontId::new(11.5, crate::ui::fonts::semibold(ui.ctx())),
                black,
                (rect.width() - 44.0).min(236.0),
            );
            ui.painter().galley(rect.min + Vec2::new(40.0, 5.0), g, black);
            // "Seconds until restart" and the count, centered in large type
            for (i, line) in [
                aurora_i18n::tr!("region-restart-legacy-seconds").to_owned(),
                a.seconds().to_string(),
            ]
            .into_iter()
            .enumerate()
            {
                let g = ui
                    .painter()
                    .layout_no_wrap(line, FontId::new(14.0, crate::ui::fonts::semibold(ui.ctx())), black);
                let x = rect.center().x - g.size().x / 2.0;
                ui.painter().galley(Pos2::new(x, rect.top() + 86.0 + 17.5 * i as f32), g, black);
            }
            let row = Rect::from_min_size(rect.min + Vec2::new(30.0, 136.0), Vec2::new(214.0, 23.0));
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            child.spacing_mut().item_spacing.x = 1.0;
            let selected = pick.and_then(|id| landmarks.iter().find(|l| l.asset == id)).map_or_else(
                || aurora_i18n::tr!("region-restart-legacy-landmarks").to_owned(),
                |l| l.name.clone(),
            );
            egui::ComboBox::from_id_salt("region_restarting_landmarks")
                .width(182.0)
                .selected_text(selected)
                .show_ui(&mut child, |ui| {
                    let mut sorted: Vec<&Landmark> = landmarks.iter().collect();
                    sorted.sort_by_key(|l| l.name.to_lowercase());
                    for l in sorted {
                        ui.selectable_value(pick, Some(l.asset), &l.name);
                    }
                });
            if widgets::action_button(&mut child, p, "arrow-right", "", widgets::ActionKind::Flat, Vec2::new(28.0, 23.0))
                .on_hover_text(aurora_i18n::tr!("region-restart-legacy-teleport"))
                .clicked()
                && let Some(asset) = *pick
            {
                out.push(Action::Landmark(asset));
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::region_restart::Tracker;
    use aurora_net::region_restart::Notice;

    fn alert(seconds: i32) -> Restart {
        let mut t = Tracker::default();
        t.notice(
            &Notice {
                seconds,
                region: Some("Aurora Démo".into()),
                opensim: false,
            },
            "",
            Some(1),
            false,
        );
        t.alert.expect("alert")
    }

    #[test]
    fn the_time_is_written_like_the_mockup() {
        assert_eq!(time_text(300), "5:00");
        assert_eq!(time_text(61), "1:01");
        assert_eq!(time_text(60), "1:00");
        assert_eq!(time_text(10), "10");
        assert_eq!(time_text(7), "7");
        assert_eq!(time_text(0), "0");
        assert!(time_text(52).starts_with("52"));
    }

    #[test]
    fn the_colors_rise_with_the_steps() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(color(&p, &alert(300)), p.amber);
        assert_eq!(color(&p, &alert(52)), p.warn);
        assert_eq!(color(&p, &alert(24)), p.danger);
        assert_eq!(color(&p, &alert(7)), p.danger);
        assert_eq!(color(&p, &alert(0)), p.muted);
    }

    /// Never more than one breath a second: far under three flashes.
    #[test]
    fn nothing_breathes_faster_than_once_a_second() {
        for level in [Level::Calm, Level::Near, Level::Red, Level::Final, Level::Zero] {
            for period in [card_period(level), pill_period(level)].into_iter().flatten() {
                assert!(period >= 1.0, "{level:?}: {period}");
            }
        }
        assert_eq!(pill_period(Level::Calm), None, "the pill is still at first");
    }

    #[test]
    fn the_shake_is_damped_and_ends_at_rest() {
        assert_eq!(shake_x(0.0), 0.0);
        assert!((shake_x(SHAKE * 0.12) + 7.0).abs() < 0.01);
        assert!(shake_x(SHAKE * 0.85).abs() < 1.0);
        assert_eq!(shake_x(SHAKE), 0.0);
    }

    #[test]
    fn the_menu_lists_recent_landmarks_or_the_search() {
        let l = |name: &str, created| Landmark {
            asset: Uuid::new_v4(),
            name: name.into(),
            detail: String::new(),
            created,
        };
        let all = vec![l("Maison", 30), l("Café", 20), l("Plage", 10), l("Vieux port", 5)];
        let names = |v: Vec<&Landmark>| v.into_iter().map(|l| l.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(listed(&all, "")), ["Maison", "Café", "Plage"]);
        assert_eq!(names(listed(&all, "po")), ["Vieux port"]);
        assert_eq!(names(listed(&all, "P")), ["Plage", "Vieux port"]);
    }

    #[test]
    fn french_elides_before_a_vowel() {
        assert_eq!(elide("Aurora Démo"), "yes");
        assert_eq!(elide("Écume"), "yes");
        assert_eq!(elide("Lune Bleue"), "no");
        assert_eq!(elide(""), "no");
    }
}
