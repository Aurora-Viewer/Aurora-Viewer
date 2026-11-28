//! The contactless terminal of the Aurora Pay window (`ui/pay.rs`): where
//! its Pay button was, a payment card to hold the click on. The card slides
//! onto a reader, four lights come on one after the other, and the payment
//! leaves when the fourth is lit; let go before that and nothing is paid.
//!
//! Aurora's own (the human's request and mock-up of 2026-11-28), a
//! deliberate difference from the Legacy Viewer, which pays at a click
//! (LLFloaterPay::onGive): `Settings::pay_contactless` off brings the Pay
//! button back, and the Legacy look (`Settings::classic_pay_window`) never
//! shows the terminal. It only stands for that click: the amount, the
//! confirmation above the threshold (FSConfirmPayments), the balance check
//! and the RLVa rule stay in `ui/pay.rs`, and apply after the hold as they
//! do after the click.
//!
//! The time is an argument of the hold (`Terminal::step`), so its tests need
//! no clock, and frames are asked for only while something moves. Each light
//! plays UISndClick, an interface sound of the Legacy Viewer, under the
//! settings of every interface sound.

use crate::theme::Palette;
use crate::ui::{fonts, icons, sheet, sound_cues, widgets};
use crate::ui_sound::UiSound;
use egui::epaint::{Mesh, Shadow, Vertex};
use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

/// How long the click is held before the payment leaves, in seconds.
pub const HOLD: f64 = 1.2;
/// How long the display says that a hold was cancelled, in seconds.
const CANCELLED: f64 = 2.5;
/// A hold not looked at for this long is over, in seconds.
const STALE: f64 = 0.5;
/// The lights of the terminal.
const LIGHTS: usize = 4;
/// The hold in equal steps: the card reaches the reader at the second, a
/// light comes on at each of the last four, the last one as the hold ends.
const STEPS: f32 = 6.0;

/// Width to height of a payment card (ISO/IEC 7810, ID-1).
const CARD_RATIO: f32 = 1.586;
/// The corners of the card and of the place it is held on: those of a
/// payment card, the drawing of a thing and not a surface of the interface
/// (docs/BRANDING.md, "Discreet rounding").
const CARD_RADIUS: f32 = 7.0;

// theme-guard: content — a black payment card, the same under every skin
const CARD: Color32 = Color32::from_rgb(12, 13, 18);
const CARD_EDGE: Color32 = Color32::from_rgb(58, 60, 72);
const CARD_INK: Color32 = Color32::from_rgb(232, 237, 251);
const ENGRAVING: Color32 = Color32::from_rgb(33, 35, 44);
const ENGRAVING_LIT: Color32 = Color32::from_rgb(150, 156, 178);

/// A hold on the card, from one frame to the next.
#[derive(Debug, Default)]
pub struct Terminal {
    /// When the hold began, in egui's time.
    since: Option<f64>,
    /// The hold reached its end: nothing more until the click is released.
    done: bool,
    /// The display says that the hold was cancelled until then.
    cancelled: Option<f64>,
    /// Lights that already played their tick in this hold.
    ticked: usize,
    /// The click went down on the card and is still down over the terminal.
    pressed: bool,
    /// The window's Pay button holds the card in our place (`play`), until
    /// the hold ends or is stopped.
    auto: bool,
    /// The last frame the hold was looked at.
    last: Option<f64>,
}

/// What a frame did to the hold.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Lights that came on.
    pub ticks: usize,
    /// The hold reached its end: the payment leaves.
    pub completed: bool,
}

/// What the window knows of the payment, for the display.
pub enum Status<'a> {
    /// A payment can leave.
    Ready,
    /// Something is missing first: what the display says of it, in the
    /// danger color for an `alert`.
    Blocked { label: &'a str, text: &'a str, alert: bool },
    /// The payment left and the server has not answered yet.
    Sending,
    /// The server took it: what the display says.
    Accepted(&'a str),
}

/// What the terminal shows this frame.
pub struct View<'a> {
    /// The amount on the display.
    pub amount: Option<i32>,
    /// The name on the card: ours.
    pub holder: &'a str,
    pub status: Status<'a>,
    /// Preferences › Accessibility › Reduce motion: the card is on the
    /// reader or off it, with no slide, wave or sweep of light.
    pub reduce_motion: bool,
    /// AURORA_DEMO_PAY=hold: a hold drawn at this fraction, for captures.
    pub frozen: Option<f32>,
}

/// Lights on at `fraction` of the hold: none while the card travels, then
/// one per step, the last as the hold ends.
pub fn lights(fraction: f32) -> usize {
    let first = STEPS - LIGHTS as f32 + 1.0;
    (0..LIGHTS).filter(|i| fraction * STEPS >= first + *i as f32 - 1e-4).count()
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Terminal {
    /// The Pay button: the card goes to the reader by itself and the payment
    /// leaves at the end of the same hold, unless `stop` comes first.
    pub fn play(&mut self) {
        self.auto = true;
    }

    /// Is the Pay button's hold on its way?
    pub fn playing(&self) -> bool {
        self.auto
    }

    /// Takes the card back before the end of the Pay button's hold: nothing
    /// is paid.
    pub fn stop(&mut self) {
        self.auto = false;
    }

    /// One frame of the hold at the time `now`: `pressed` while the click is
    /// held on the card, `ready` while a payment can leave.
    pub fn step(&mut self, pressed: bool, ready: bool, now: f64) -> Step {
        let mut step = Step::default();
        // a hold nobody saw the end of pays nothing: the window was hidden
        // or the viewer stalled, and its time ran out meanwhile
        let stale = self.last.is_some_and(|last| now - last > STALE);
        self.last = Some(now);
        if stale || !ready {
            self.auto = false;
        }
        let down = (pressed || self.auto) && !stale;
        if !down {
            // let go before the end: nothing was paid, and the display says so
            if self.since.take().is_some() {
                self.cancelled = Some(now + CANCELLED);
            }
            // a click still down after its payment starts no other one
            if !pressed {
                self.done = false;
            }
            self.ticked = 0;
            return step;
        }
        if self.done {
            return step;
        }
        if !ready {
            self.since = None;
            self.ticked = 0;
            return step;
        }
        let since = *self.since.get_or_insert(now);
        self.cancelled = None;
        let fraction = Self::fraction_at(since, now);
        let lit = lights(fraction);
        step.ticks = lit.saturating_sub(self.ticked);
        self.ticked = lit;
        if fraction >= 1.0 {
            self.since = None;
            self.done = true;
            self.auto = false;
            step.completed = true;
        }
        step
    }

    fn fraction_at(since: f64, now: f64) -> f32 {
        (((now - since) / HOLD) as f32).clamp(0.0, 1.0)
    }

    /// Draws the terminal over the width of `ui`; true on the frame the hold
    /// reaches its end.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View<'_>) -> bool {
        const MARGIN: f32 = 12.0;
        const GAP: f32 = 16.0;
        const LIGHT: f32 = 4.0;
        const DISPLAY: f32 = 54.0;
        let now = ui.input(|i| i.time);
        let width = ui.available_width();
        let inner = width - 2.0 * MARGIN;
        let card_w = ((inner - GAP) / 2.0).floor();
        let card = vec2(card_w, (card_w / CARD_RATIO).round());
        let height = 10.0 + 2.0 * LIGHT + 10.0 + DISPLAY + 10.0 + card.y + 12.0;
        let (zone, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let display = Rect::from_min_size(pos2(zone.left() + MARGIN, zone.top() + 20.0 + 2.0 * LIGHT), vec2(inner, DISPLAY));
        // where the card is held, and where it waits
        let place = Rect::from_min_size(pos2(display.left(), display.bottom() + 10.0), card);
        let rest = place.translate(vec2(card.x + GAP, 0.0));
        let pad = place.union(rest).expand2(vec2(0.0, 4.0));
        let id = ui.scope_id().with("pay_terminal");
        // a drag: a click's own window (egui's max_click_duration) ends
        // before the hold does
        let response = ui.interact(pad, id, Sense::drag());

        let ready = matches!(view.status, Status::Ready);
        let (primary, over) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.interact_pos().is_some_and(|at| pad.contains(at)),
            )
        });
        // taking the card off the terminal lets go of it, like the release
        self.pressed = primary && over && (self.pressed || response.is_pointer_button_down_on());
        let step = match view.frozen {
            Some(_) => Step::default(),
            None => self.step(self.pressed, ready, now),
        };
        for _ in 0..step.ticks {
            sound_cues::request(ui.ctx(), UiSound::Click);
        }

        let hold = view.frozen.or_else(|| self.since.map(|since| Self::fraction_at(since, now)));
        let sending = matches!(view.status, Status::Sending);
        let accepted = matches!(view.status, Status::Accepted(_));
        // read, and still on the reader
        let read = hold.is_none() && (self.done || sending);
        let on_reader = hold.is_some() || read;
        let lit = match hold {
            Some(fraction) => lights(fraction),
            None if read || accepted => LIGHTS,
            None => 0,
        };
        let cancelled = self.cancelled.filter(|until| hold.is_none() && now < *until);
        if (hold.is_some() && view.frozen.is_none()) || sending {
            ui.ctx().request_repaint();
        } else if let Some(until) = cancelled {
            ui.ctx().request_repaint_after_secs((until - now) as f32);
        }

        let target = if on_reader { 1.0 } else { 0.0 };
        let travel = if view.reduce_motion {
            target
        } else {
            let seconds = (HOLD * 2.0 / f64::from(STEPS)) as f32;
            ease(ui.ctx().animate_value_with_time(id.with("travel"), target, seconds))
        };
        let pop = if view.reduce_motion {
            f32::from(u8::from(accepted))
        } else {
            ui.ctx().animate_bool_with_time(id.with("accepted"), accepted, 0.25)
        };

        let painter = ui.painter().with_clip_rect(zone.intersect(ui.clip_rect()));
        painter.rect_filled(zone, ui.visuals().window_corner_radius, p.bar);
        for i in 0..LIGHTS {
            let x = zone.center().x + (i as f32 - (LIGHTS as f32 - 1.0) / 2.0) * 18.0;
            let center = pos2(x, zone.top() + 10.0 + LIGHT);
            if i < lit {
                painter.circle_filled(center, LIGHT + 3.0, p.success.gamma_multiply(0.22));
                painter.circle_filled(center, LIGHT, p.success);
            } else {
                painter.circle_filled(center, LIGHT, p.raised);
            }
        }

        let (label, text, color) = if hold.is_some() {
            let help = if self.auto {
                aurora_i18n::tr!("ui-pay-terminal-playing-help")
            } else {
                aurora_i18n::tr!("ui-pay-terminal-reading-help")
            };
            (aurora_i18n::tr!("ui-pay-terminal-reading"), help, p.violet_light)
        } else {
            match &view.status {
                Status::Sending => (
                    aurora_i18n::tr!("ui-pay-terminal-sending"),
                    aurora_i18n::tr!("ui-pay-terminal-sending-help"),
                    p.violet_light,
                ),
                Status::Accepted(text) => (aurora_i18n::tr!("ui-pay-terminal-accepted"), *text, p.success),
                _ if cancelled.is_some() => (
                    aurora_i18n::tr!("ui-pay-terminal-cancelled"),
                    aurora_i18n::tr!("ui-pay-terminal-cancelled-help"),
                    p.amber,
                ),
                Status::Blocked { label, text, alert } => (*label, *text, if *alert { p.danger } else { p.muted }),
                Status::Ready => (
                    aurora_i18n::tr!("ui-pay-terminal-ready"),
                    aurora_i18n::tr!("ui-pay-terminal-ready-help"),
                    p.muted,
                ),
            }
        };
        painter.rect_filled(display, widgets::inner_radius(ui.visuals()), p.field);
        let family = fonts::semibold(ui.ctx());
        let mut figures = LayoutJob::default();
        figures.append(aurora_i18n::currency(), 0.0, fonts::semibold_format(&family, 12.0, p.muted));
        figures.append(
            &aurora_i18n::integer(i64::from(view.amount.unwrap_or(0).max(0))),
            5.0,
            fonts::semibold_format(&family, 22.0, p.ink),
        );
        let figures = painter.layout_job(figures);
        let text_width = (display.width() - 36.0 - figures.size().x).max(40.0);
        let mut label_job = sheet::semibold(ui, label, 10.5, color);
        label_job.wrap = TextWrapping::truncate_at_width(text_width);
        let label_galley = painter.layout_job(label_job);
        let mut text_job = LayoutJob::simple(text.to_owned(), FontId::proportional(12.5), p.ink, text_width);
        text_job.wrap.max_rows = 2;
        text_job.wrap.overflow_character = Some('…');
        let text_galley = painter.layout_job(text_job);
        let top = display.center().y - (label_galley.size().y + 2.0 + text_galley.size().y) / 2.0;
        let below = top + label_galley.size().y + 2.0;
        painter.galley(pos2(display.left() + 12.0, top), label_galley, color);
        painter.galley(pos2(display.left() + 12.0, below), text_galley, p.ink);
        let figures_at = pos2(
            display.right() - 12.0 - figures.size().x,
            display.center().y - figures.size().y / 2.0,
        );
        painter.galley(figures_at, figures, p.ink);
        let progress = match hold {
            Some(fraction) => fraction,
            None if read || accepted => 1.0,
            None => 0.0,
        };
        if progress > 0.0 {
            let line = Rect::from_min_max(
                pos2(display.left(), display.bottom() - 2.0),
                pos2(display.left() + display.width() * progress, display.bottom()),
            );
            painter.rect_filled(line, 0.0, if accepted { p.success } else { p.violet_light });
        }

        let (edge, wash) = if accepted {
            (p.success.gamma_multiply(0.6), p.success.gamma_multiply(0.08))
        } else if on_reader {
            (p.violet_light.gamma_multiply(0.9), p.violet.gamma_multiply(0.16))
        } else if ready {
            (p.violet_light.gamma_multiply(0.45), p.violet.gamma_multiply(0.07))
        } else {
            (p.muted_dim.gamma_multiply(0.4), p.violet.gamma_multiply(0.04))
        };
        painter.rect_filled(place, CARD_RADIUS, wash);
        painter.rect_stroke(place, CARD_RADIUS, Stroke::new(1.5, edge), StrokeKind::Inside);
        if on_reader && !view.reduce_motion {
            // the waves of the reading, kept under the display
            let waves = painter.with_clip_rect(pad.intersect(painter.clip_rect()));
            for wave in 0..2 {
                let phase = ((now / 1.2 + f64::from(wave) * 0.5) % 1.0) as f32;
                let stroke = Stroke::new(1.5, p.violet_light.gamma_multiply(0.7 * (1.0 - phase)));
                waves.circle_stroke(place.center(), card.y * (0.35 + 1.3 * phase), stroke);
            }
        }
        if pop < 1.0 {
            let strength = if ready || on_reader { 0.9 } else { 0.35 };
            symbol(
                &painter,
                place.center(),
                30.0,
                p.violet_light.gamma_multiply(strength * (1.0 - pop)),
            );
        }
        if pop > 0.0 {
            let size = 44.0 * (0.6 + 0.4 * ease(pop));
            painter.circle_filled(place.center(), size / 2.0, p.success.gamma_multiply(0.16 * pop));
            icon(&painter, "check", place.center(), size * 0.6, p.success.gamma_multiply(pop));
        }

        let mut card_painter = painter.clone();
        if !(ready || on_reader || accepted) {
            card_painter.set_opacity(0.45);
        }
        let at = rest.translate(vec2(-(card.x + GAP) * travel, 0.0));
        paint_card(&card_painter, ui, p, at, view.holder, travel, on_reader);

        if self.pressed {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        let response = if ready {
            response.on_hover_cursor(egui::CursorIcon::Grab)
        } else {
            response
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, text));
        step.completed
    }
}

/// The payment card at `rect`: black, our name on it, the Aurora wolf
/// engraved in it (the brand's white silhouette under a tint; the light that
/// crosses it is the one exception docs/BRANDING.md makes to "no effect on
/// the logo"). `travel`: how far the card is on its way to the reader, which
/// moves that light; `lit`: on the reader, its glow.
fn paint_card(painter: &egui::Painter, ui: &egui::Ui, p: &Palette, rect: Rect, holder: &str, travel: f32, lit: bool) {
    let shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: p.navy.gamma_multiply(0.7),
    };
    painter.add(shadow.as_shape(rect, CARD_RADIUS));
    if lit {
        let glow = Shadow {
            offset: [0, 0],
            blur: 22,
            spread: 1,
            color: p.violet.gamma_multiply(0.5 * travel),
        };
        painter.add(glow.as_shape(rect, CARD_RADIUS));
    }
    painter.rect_filled(rect, CARD_RADIUS, CARD);
    if let Some(wolf) = icons::global("wolf-large") {
        let size = rect.height() - 12.0;
        let logo = Rect::from_center_size(pos2(rect.right() - size / 2.0 - 8.0, rect.center().y), Vec2::splat(size));
        painter.add(Shape::mesh(engraving(wolf.id(), logo, -0.2 + 1.4 * travel)));
    }
    let edge = if lit { p.violet_light.gamma_multiply(0.6) } else { CARD_EDGE };
    painter.rect_stroke(rect, CARD_RADIUS, Stroke::new(1.0, edge), StrokeKind::Inside);

    let chip = Rect::from_min_size(
        pos2(rect.left() + 14.0, rect.top() + (rect.height() * 0.3).round()),
        vec2(26.0, 19.0),
    );
    painter.rect_filled(chip, widgets::inner_radius(ui.visuals()), p.violet_pale);
    let contact = Stroke::new(1.0, CARD.gamma_multiply(0.5));
    for x in [chip.left() + 8.0, chip.right() - 8.0] {
        painter.line_segment([pos2(x, chip.top()), pos2(x, chip.bottom())], contact);
    }
    for y in [chip.top() + 6.0, chip.bottom() - 6.0] {
        painter.line_segment([pos2(chip.left(), y), pos2(chip.left() + 8.0, y)], contact);
        painter.line_segment([pos2(chip.right() - 8.0, y), pos2(chip.right(), y)], contact);
    }
    symbol(
        painter,
        pos2(chip.right() + 15.0, chip.center().y),
        16.0,
        CARD_INK.gamma_multiply(0.8),
    );

    let currency = painter.layout_job(sheet::semibold(ui, aurora_i18n::currency(), 13.0, p.violet_light));
    let mut name = sheet::semibold(ui, &holder.to_uppercase(), 10.5, CARD_INK);
    name.wrap = TextWrapping::truncate_at_width(rect.width() - 36.0 - currency.size().x);
    let name = painter.layout_job(name);
    painter.galley(pos2(rect.left() + 14.0, rect.bottom() - 12.0 - name.size().y), name, CARD_INK);
    let currency_at = pos2(rect.right() - 13.0 - currency.size().x, rect.bottom() - 11.0 - currency.size().y);
    painter.galley(currency_at, currency, p.violet_light);
    let ribbon = Rect::from_min_max(
        pos2(rect.left() + 2.0, rect.bottom() - 6.0),
        pos2(rect.right() - 2.0, rect.bottom() - 4.0),
    );
    painter.add(Shape::mesh(aurora_ribbon(p, ribbon)));
}

/// The wolf of `texture` engraved over `rect`, tone on tone, with a leaning
/// band of light centered on `light` (0 at its left edge, 1 at its right).
fn engraving(texture: egui::TextureId, rect: Rect, light: f32) -> Mesh {
    const COLUMNS: u32 = 16;
    // the band reaches the top of a column before its foot
    const LEAN: f32 = 0.18;
    let shade = |at: f32| {
        let off = ((at - light) / 0.24).abs().min(1.0);
        ENGRAVING.lerp_to_gamma(ENGRAVING_LIT, (1.0 - off) * (1.0 - off))
    };
    let mut mesh = Mesh::with_texture(texture);
    for column in 0..=COLUMNS {
        let u = column as f32 / COLUMNS as f32;
        let x = rect.left() + rect.width() * u;
        mesh.vertices.push(Vertex {
            pos: pos2(x, rect.top()),
            uv: pos2(u, 0.0),
            color: shade(u + LEAN),
        });
        mesh.vertices.push(Vertex {
            pos: pos2(x, rect.bottom()),
            uv: pos2(u, 1.0),
            color: shade(u - LEAN),
        });
        if column > 0 {
            let k = column * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

/// The aurora gradient along `rect`, from its left to its right.
fn aurora_ribbon(p: &Palette, rect: Rect) -> Mesh {
    const STEPS: u32 = 12;
    let mut mesh = Mesh::default();
    for step in 0..=STEPS {
        let t = step as f32 / STEPS as f32;
        let color = crate::ui::options::aurora_color(p, t);
        let x = rect.left() + rect.width() * t;
        mesh.colored_vertex(pos2(x, rect.top()), color);
        mesh.colored_vertex(pos2(x, rect.bottom()), color);
        if step > 0 {
            let k = step * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k + 1, k);
        }
    }
    mesh
}

fn icon(painter: &egui::Painter, name: &str, center: Pos2, size: f32, tint: Color32) {
    if let Some(texture) = icons::global(name) {
        let rect = Rect::from_center_size(center, Vec2::splat(size));
        painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
}

/// The contactless symbol: Tabler's `wifi` turned a quarter, its waves
/// leaving to the right.
fn symbol(painter: &egui::Painter, center: Pos2, size: f32, tint: Color32) {
    let Some(texture) = icons::global("wifi") else {
        return;
    };
    let rect = Rect::from_center_size(center, Vec2::splat(size));
    let mut mesh = Mesh::with_texture(texture.id());
    mesh.add_rect_with_uv(rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    mesh.rotate(egui::emath::Rot2::from_angle(std::f32::consts::FRAC_PI_2), center);
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hold of `seconds` at 60 frames a second: the ticks and the payments.
    fn hold(terminal: &mut Terminal, from: f64, seconds: f64) -> (usize, usize) {
        let frames = (seconds * 60.0).round() as u32;
        let (mut ticks, mut paid) = (0, 0);
        for frame in 0..=frames {
            let step = terminal.step(true, true, from + f64::from(frame) / 60.0);
            ticks += step.ticks;
            paid += usize::from(step.completed);
        }
        (ticks, paid)
    }

    #[test]
    fn the_lights_follow_the_card_and_the_last_ends_the_hold() {
        assert_eq!(lights(0.0), 0);
        assert_eq!(lights(0.49), 0, "none while the card travels");
        assert_eq!(lights(0.5), 1);
        assert_eq!(lights(0.67), 2);
        assert_eq!(lights(0.84), 3);
        assert_eq!(lights(0.99), 3);
        assert_eq!(lights(1.0), LIGHTS);
    }

    #[test]
    fn a_full_hold_pays_once_with_a_tick_per_light() {
        let mut terminal = Terminal::default();
        assert_eq!(hold(&mut terminal, 10.0, HOLD + 0.1), (LIGHTS, 1));
        // still held: nothing more, however long
        assert_eq!(hold(&mut terminal, 12.0, 3.0), (0, 0));
        assert!(terminal.cancelled.is_none());
        // released after the end: not a cancellation
        assert_eq!(terminal.step(false, true, 16.0), Step::default());
        assert!(terminal.cancelled.is_none());
        // a new hold pays again
        assert_eq!(hold(&mut terminal, 17.0, HOLD + 0.1), (LIGHTS, 1));
    }

    #[test]
    fn letting_go_before_the_end_pays_nothing_and_says_so() {
        let mut terminal = Terminal::default();
        let (ticks, paid) = hold(&mut terminal, 5.0, HOLD * 0.7);
        assert_eq!((ticks, paid), (2, 0));
        assert_eq!(terminal.step(false, true, 5.9), Step::default());
        assert_eq!(terminal.cancelled, Some(5.9 + CANCELLED));
        // the next hold starts from nothing: the whole time again
        let (ticks, paid) = hold(&mut terminal, 6.0, HOLD * 0.7);
        assert_eq!((ticks, paid), (2, 0));
        assert!(terminal.cancelled.is_none(), "a new hold clears the notice");
    }

    #[test]
    fn nothing_is_held_while_no_payment_can_leave() {
        let mut terminal = Terminal::default();
        for frame in 0..200 {
            assert_eq!(terminal.step(true, false, f64::from(frame) / 60.0), Step::default());
        }
        // released: no hold was lost, so nothing to say
        terminal.step(false, false, 4.0);
        assert!(terminal.cancelled.is_none());
        // a hold that stops being possible half-way is dropped without paying
        hold(&mut terminal, 5.0, HOLD * 0.5);
        assert_eq!(terminal.step(true, false, 5.7), Step::default());
        assert_eq!(hold(&mut terminal, 5.8, HOLD * 0.5), (1, 0), "the time starts again");
    }

    /// The frames of `seconds` with nothing pressed: the ticks and the payments.
    fn watch(terminal: &mut Terminal, from: f64, seconds: f64) -> (usize, usize) {
        let frames = (seconds * 60.0).round() as u32;
        let (mut ticks, mut paid) = (0, 0);
        for frame in 0..=frames {
            let step = terminal.step(false, true, from + f64::from(frame) / 60.0);
            ticks += step.ticks;
            paid += usize::from(step.completed);
        }
        (ticks, paid)
    }

    #[test]
    fn the_pay_button_holds_the_card_to_the_end_and_pays_once() {
        let mut terminal = Terminal::default();
        assert_eq!(watch(&mut terminal, 1.0, 1.0), (0, 0), "nothing by itself");
        terminal.play();
        assert!(terminal.playing());
        assert_eq!(watch(&mut terminal, 2.0, HOLD + 0.1), (LIGHTS, 1));
        assert!(!terminal.playing());
        assert_eq!(watch(&mut terminal, 4.0, 3.0), (0, 0), "once");
        assert!(terminal.cancelled.is_none());
    }

    #[test]
    fn the_pay_button_hold_stopped_pays_nothing() {
        let mut terminal = Terminal::default();
        terminal.play();
        assert_eq!(watch(&mut terminal, 2.0, HOLD * 0.7), (2, 0));
        terminal.stop();
        assert_eq!(watch(&mut terminal, 2.9, 3.0), (0, 0));
        assert!(terminal.cancelled.is_some(), "the display says so");
        // nor when a payment stops being possible on the way
        terminal.play();
        watch(&mut terminal, 6.0, HOLD * 0.5);
        assert_eq!(terminal.step(false, false, 6.7), Step::default());
        assert!(!terminal.playing());
        assert_eq!(watch(&mut terminal, 6.8, 3.0), (0, 0));
    }

    #[test]
    fn a_hold_nobody_saw_the_end_of_pays_nothing() {
        // the window hidden, or the viewer stalled, while the time ran out
        let mut terminal = Terminal::default();
        terminal.play();
        watch(&mut terminal, 2.0, HOLD * 0.5);
        assert_eq!(terminal.step(false, true, 60.0), Step::default());
        assert!(!terminal.playing());
        assert_eq!(watch(&mut terminal, 60.1, 3.0), (0, 0));
        // the same for a click still held
        hold(&mut terminal, 70.0, HOLD * 0.5);
        assert_eq!(terminal.step(true, true, 90.0), Step::default());
        assert_eq!(hold(&mut terminal, 90.1, HOLD * 0.5), (1, 0), "its time starts again");
    }

    #[test]
    fn the_click_held_on_the_card_pays_and_a_short_one_does_not() {
        let mut frames = widgets::elide_tests::Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        let p = theme.palette();
        for (held, paid) in [(90, 1), (30, 0)] {
            let mut terminal = Terminal::default();
            let mut payments = 0;
            let mut draw = |ui: &mut egui::Ui| {
                ui.set_width(380.0);
                let view = View {
                    amount: Some(25),
                    holder: "Nova Resident",
                    status: Status::Ready,
                    reduce_motion: false,
                    frozen: None,
                };
                payments += usize::from(terminal.show(ui, &p, &view));
            };
            frames.run(Vec::new(), &mut draw);
            let texts = frames.run(Vec::new(), &mut draw);
            let pos = texts
                .iter()
                .find(|t| t.source == "NOVA RESIDENT")
                .expect("the name on the card")
                .rect
                .center();
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            frames.run(vec![egui::Event::PointerMoved(pos), button(true)], &mut draw);
            for _ in 0..held {
                frames.run(Vec::new(), &mut draw);
            }
            frames.run(vec![button(false)], &mut draw);
            frames.run(Vec::new(), &mut draw);
            assert_eq!(payments, paid, "held for {held} frames");
        }
    }
}
