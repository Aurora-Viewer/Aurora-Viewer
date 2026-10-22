//! The pieces of Aurora's redesigned windows ("About Land", and the windows
//! drawn after it): a header framed on the field's face, page tabs with
//! "View as", pills, cards with a title and a link, lines of facts, settings
//! rows with their explanation, the amber lock banner and line, folded
//! "Advanced" parts, a column of sections whose navigation follows the
//! scrolling, and the lists of residents (their search field is
//! `widgets::search_box`). Each
//! window keeps its own data and rules; these only draw.

use crate::theme::Palette;
use crate::ui::{presence, widgets};
use crate::world::World;
use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Vec2};
use uuid::Uuid;

/// The tint of a pill: allowed, for the group only, refused, or a plain fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Allow,
    Group,
    Deny,
    Neutral,
}

/// Text in the theme's semi-bold face.
pub fn semibold(ui: &egui::Ui, text: &str, size: f32, color: Color32) -> egui::text::LayoutJob {
    let family = crate::ui::fonts::semibold(ui.ctx());
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, crate::ui::fonts::semibold_format(&family, size, color));
    job
}

/// A small Tabler icon `size` wide.
pub fn icon_at(ui: &egui::Ui, name: &str, center: Pos2, size: f32, tint: Color32) {
    if let Some(t) = crate::ui::icons::global(name) {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(center, Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
}

/// A pill: an icon (or none) and a short text on a tinted face.
pub fn pill(ui: &mut egui::Ui, p: &Palette, tone: Tone, icon: &str, label: &str) -> egui::Response {
    const PAD: f32 = 9.0;
    const ICON: f32 = 13.0;
    let (fill, ink) = match tone {
        Tone::Allow => (p.success.gamma_multiply(0.12), p.success),
        Tone::Group => (p.violet_light.gamma_multiply(0.16), p.violet_pale),
        Tone::Deny => (p.danger.gamma_multiply(0.12), p.danger),
        Tone::Neutral => (p.bar, p.muted),
    };
    let galley = ui.painter().layout_job(semibold(ui, label, 12.0, ink));
    let lead = if icon.is_empty() { 0.0 } else { ICON + 5.0 };
    let size = Vec2::new(PAD * 2.0 + lead + galley.size().x, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    if !icon.is_empty() {
        icon_at(ui, icon, egui::pos2(rect.left() + PAD + ICON / 2.0, rect.center().y), ICON, ink);
    }
    let pos = crate::ui::widgets::row_text_origin(ui.painter(), &galley, rect.left() + PAD + lead, rect.center().y);
    ui.painter().galley(pos, galley, ink);
    resp
}

/// The title of a card: its icon, its name, and a link at the right end
/// (its response).
pub fn card_title(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, link: Option<&str>) -> Option<egui::Response> {
    let out = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            widgets::glyph(ui, icon, p.violet_light, 15.0);
            ui.label(semibold(ui, title, 13.0, p.ink));
            link.map(|text| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(semibold(ui, text, 12.0, p.violet_light)).sense(Sense::click()))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                })
                .inner
            })
        })
        .inner;
    ui.add_space(2.0);
    out
}

/// Width of the keys of `kv`.
const KEY_W: f32 = 140.0;

/// A line of facts: its key in the left column, then `value`.
pub fn kv(ui: &mut egui::Ui, p: &Palette, key: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.allocate_ui_with_layout(Vec2::new(KEY_W, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(KEY_W);
            ui.add(egui::Label::new(RichText::new(key).size(12.5).color(p.muted)).truncate());
        });
        ui.add_space(6.0);
        value(ui);
    });
}

/// Two cards side by side, `ratio` to 1 (1.3 for an overview's grid), as
/// tall as the taller one. `world` and `s` are lent to each side in turn.
pub fn cards<W, S>(
    ui: &mut egui::Ui,
    p: &Palette,
    world: &mut W,
    s: &mut S,
    ratio: f32,
    left: impl FnOnce(&mut egui::Ui, &mut W, &mut S),
    right: impl FnOnce(&mut egui::Ui, &mut W, &mut S),
) {
    const GAP: f32 = 12.0;
    const PAD: Vec2 = Vec2::new(12.0, 10.0);
    let width = ui.available_width();
    let left_w = ((width - GAP) * ratio / (ratio + 1.0)).floor();
    let right_w = width - GAP - left_w;
    let top = ui.cursor().min;
    let faces = [ui.painter().add(Shape::Noop), ui.painter().add(Shape::Noop)];
    let mut column = |x: f32, w: f32, body: &mut dyn FnMut(&mut egui::Ui)| {
        let inner = Rect::from_min_size(
            egui::pos2(x + PAD.x, top.y + PAD.y),
            Vec2::new((w - PAD.x * 2.0).max(1.0), 10_000.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        // the lines of facts as close as the mockup's (12.5 px text, 6 px apart)
        child.spacing_mut().item_spacing.y = 6.0;
        child.spacing_mut().interact_size.y = 16.0;
        body(&mut child);
        child.min_rect().height()
    };
    let mut left = Some(left);
    let mut right = Some(right);
    let h_left = column(top.x, left_w, &mut |ui| {
        if let Some(f) = left.take() {
            f(ui, world, s)
        }
    });
    let h_right = column(top.x + left_w + GAP, right_w, &mut |ui| {
        if let Some(f) = right.take() {
            f(ui, world, s)
        }
    });
    let h = h_left.max(h_right) + PAD.y * 2.0;
    let radius = ui.visuals().window_corner_radius;
    ui.painter().set(
        faces[0],
        Shape::rect_filled(Rect::from_min_size(top, Vec2::new(left_w, h)), radius, p.field),
    );
    ui.painter().set(
        faces[1],
        Shape::rect_filled(
            Rect::from_min_size(egui::pos2(top.x + left_w + GAP, top.y), Vec2::new(right_w, h)),
            radius,
            p.field,
        ),
    );
    ui.allocate_rect(Rect::from_min_size(top, Vec2::new(width, h)), Sense::hover());
}

/// A fact of a header: its small key over its value.
pub fn fact(ui: &mut egui::Ui, p: &Palette, key: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(key).size(11.0).color(p.muted));
        value(ui);
    });
}

/// Width of a header's gauge.
pub const GAUGE_W: f32 = 220.0;

/// A header's gauge under its fact: the accent, or `tint` (caution, danger,
/// good health).
pub fn gauge(ui: &mut egui::Ui, p: &Palette, id: &str, fraction: f32, width: f32, tint: Option<Color32>) {
    ui.add_space(3.0);
    widgets::progress(ui, p, id, fraction, width, "", tint);
}

/// The side of a header's picture.
pub const PICTURE: Vec2 = Vec2::new(220.0, 148.0);

/// The part of a `w` x `h` picture that covers a `size` frame, cut in its
/// middle.
pub fn cover_uv(w: usize, h: usize, size: Vec2) -> Rect {
    let picture = w.max(1) as f32 / h.max(1) as f32;
    let frame = size.x / size.y.max(1.0);
    let half = if picture > frame {
        Vec2::new(frame / picture, 1.0) * 0.5
    } else {
        Vec2::new(1.0, picture / frame) * 0.5
    };
    Rect::from_min_max(egui::pos2(0.5, 0.5) - half, egui::pos2(0.5, 0.5) + half)
}

/// The bottom `height` of `rect` fading into a card's face (an excerpt
/// cut before its "Read all").
pub fn fade(ui: &egui::Ui, p: &Palette, rect: Rect, height: f32) {
    let top = rect.bottom() - height;
    let clear = p.field.gamma_multiply(0.0);
    let v = |x: f32, y: f32, color: Color32| egui::epaint::Vertex {
        pos: egui::pos2(x, y),
        uv: egui::epaint::WHITE_UV,
        color,
    };
    let mut m = egui::epaint::Mesh::default();
    m.vertices.extend([
        v(rect.left(), top, clear),
        v(rect.right(), top, clear),
        v(rect.right(), rect.bottom(), p.field),
        v(rect.left(), rect.bottom(), p.field),
    ]);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    ui.painter().add(Shape::mesh(m));
}

/// A long text of a card (a covenant) cut at `max_h` with a fade unless
/// `open`. True when it is longer than that (its "Read all" link).
pub fn excerpt(ui: &mut egui::Ui, p: &Palette, text: &str, open: bool, max_h: f32) -> bool {
    let width = ui.available_width();
    let galley = ui.painter().layout(text.to_owned(), FontId::proportional(12.5), p.muted, width);
    let long = galley.size().y > max_h + 1.0;
    let shown = if long && !open { max_h } else { galley.size().y };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, shown), Sense::hover());
    ui.painter_at(rect).galley(rect.min, galley, p.muted);
    if long && !open {
        fade(ui, p, rect, 30.0);
    }
    long
}

/// A header on the field's face, 12 px inside: `body` lays out its parts
/// left to right, 14 px apart (the picture, then a column of texts).
pub fn hero(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui)) {
    const PAD: f32 = 12.0;
    let width = ui.available_width();
    let top = ui.cursor().min;
    let face = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_size(top + Vec2::splat(PAD), Vec2::new(width - PAD * 2.0, 10_000.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.x = 14.0;
    body(&mut child);
    let h = child.min_rect().height() + PAD * 2.0;
    ui.painter().set(
        face,
        Shape::rect_filled(
            Rect::from_min_size(top, Vec2::new(width, h)),
            ui.visuals().window_corner_radius,
            p.field,
        ),
    );
    ui.allocate_rect(Rect::from_min_size(top, Vec2::new(width, h)), Sense::hover());
}

/// A column of texts beside a header's picture: rows as tall as their
/// text, 8 px apart, like the mockups.
pub fn hero_column(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.spacing_mut().interact_size.y = 16.0;
        body(ui);
    });
}

/// One page tab: its icon and name; a disabled tab says why on hover.
pub struct PageTab<'a, P> {
    pub page: P,
    pub icon: &'a str,
    pub label: &'a str,
    pub disabled: Option<&'a str>,
}

/// "View as": the name of the agent's own rights, of someone without
/// rights, what the choice does (its tooltip), and whether the lit choice
/// takes the accent.
pub struct ViewAs<'a> {
    pub mine: &'a str,
    pub other: &'a str,
    pub tip: &'a str,
    pub accent: bool,
}

/// The page tabs: a lit face for the open one; at the right end, for who
/// may change something, "View as" themselves or someone without rights.
pub fn page_tabs<P: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    tabs: &[PageTab<'_, P>],
    page: &mut P,
    role: Option<ViewAs<'_>>,
    as_visitor: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for it in tabs {
            let selected = *page == it.page;
            let enabled = it.disabled.is_none();
            let ink = if selected {
                p.ink
            } else if enabled {
                p.muted
            } else {
                p.muted_dim
            };
            let galley = ui.painter().layout_job(semibold(ui, it.label, 12.5, ink));
            let size = Vec2::new(12.0 * 2.0 + 14.0 + 6.0 + galley.size().x, 28.0);
            let sense = if enabled { Sense::click() } else { Sense::hover() };
            let (rect, resp) = ui.allocate_exact_size(size, sense);
            let face = if selected {
                Some(p.raised)
            } else if enabled && resp.hovered() {
                Some(p.raised.gamma_multiply(0.5))
            } else {
                None
            };
            if let Some(face) = face {
                ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
            }
            icon_at(ui, it.icon, egui::pos2(rect.left() + 12.0 + 7.0, rect.center().y), 14.0, ink);
            let pos = crate::ui::widgets::row_text_origin(ui.painter(), &galley, rect.left() + 12.0 + 14.0 + 6.0, rect.center().y);
            ui.painter().galley(pos, galley, ink);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, it.label));
            if let Some(why) = it.disabled {
                resp.on_hover_text(why);
            } else if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                *page = it.page;
            }
        }
        if let Some(r) = role {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut choice = usize::from(*as_visitor);
                let options = [(r.mine, r.accent), (r.other, r.accent)];
                if widgets::segmented(ui, p, &mut choice, &options) {
                    *as_visitor = choice == 1;
                }
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-land-overview-view-as"))
                        .size(12.0)
                        .color(p.muted),
                )
                .on_hover_text(r.tip);
            });
        }
    });
}

/// Controls greyed (and faded like the mockup's read-only rows) unless
/// `editable`.
pub fn editable<R>(ui: &mut egui::Ui, editable: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        if !editable {
            ui.disable();
            ui.multiply_opacity(0.6);
        }
        body(ui)
    })
    .inner
}

/// A card being drawn: its face is painted once its height is known.
pub struct Card {
    top: Pos2,
    width: f32,
    face: egui::layers::ShapeIdx,
    pub child: egui::Ui,
}

const CARD_PAD: Vec2 = Vec2::new(12.0, 10.0);

/// A card the width of the room left, its content in `Card::child`.
pub fn card_begin(ui: &mut egui::Ui) -> Card {
    let width = ui.available_width();
    let top = ui.cursor().min;
    let face = ui.painter().add(Shape::Noop);
    let inner = Rect::from_min_size(top + CARD_PAD, Vec2::new(width - CARD_PAD.x * 2.0, 100_000.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.spacing_mut().item_spacing.y = 0.0;
    child.spacing_mut().interact_size.y = 16.0;
    Card { top, width, face, child }
}

/// The card's face under what was drawn, and 10 px before the next one.
pub fn card_end(ui: &mut egui::Ui, p: &Palette, card: Card) -> Rect {
    let rect = Rect::from_min_size(card.top, Vec2::new(card.width, card.child.min_rect().height() + CARD_PAD.y * 2.0));
    ui.painter()
        .set(card.face, Shape::rect_filled(rect, ui.visuals().window_corner_radius, p.field));
    ui.allocate_rect(rect, Sense::hover());
    ui.add_space(10.0);
    rect
}

/// The title of a section: its icon and name, and `right` at the right end.
pub fn section_title(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        widgets::glyph(ui, icon, p.violet_light, 16.0);
        ui.label(semibold(ui, label, 13.5, p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// The sentence under a title.
pub fn why(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(3.0);
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
    ui.add_space(6.0);
}

/// An amber line with a lock: why the rows under it are greyed.
pub fn lock_line(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        widgets::glyph(ui, "lock-filled", p.amber, 13.0);
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(p.amber)).wrap());
    });
    ui.add_space(4.0);
}

/// `lock_line` ending on a link to where it is decided; true when the link
/// is clicked.
pub fn lock_line_link(ui: &mut egui::Ui, p: &Palette, text: &str, link: &str) -> bool {
    ui.add_space(2.0);
    let clicked = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            widgets::glyph(ui, "lock-filled", p.amber, 13.0);
            let width = ui.available_width();
            let (galley, at) = linked_text(ui, p, text, p.amber, 11.5, link, width);
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            paint_linked(ui, galley, rect.min, at, p.amber)
        })
        .inner;
    ui.add_space(4.0);
    clicked
}

/// `text` then a violet `link` in one wrapped paragraph, and where the
/// link starts (in characters).
fn linked_text(
    ui: &egui::Ui,
    p: &Palette,
    text: &str,
    color: Color32,
    size: f32,
    link: &str,
    width: f32,
) -> (std::sync::Arc<egui::Galley>, usize) {
    let mut job = egui::text::LayoutJob::default();
    let plain = format!("{text} ");
    job.append(&plain, 0.0, egui::TextFormat::simple(FontId::proportional(size), color));
    let family = crate::ui::fonts::semibold(ui.ctx());
    job.append(link, 0.0, crate::ui::fonts::semibold_format(&family, size, p.violet_light));
    job.wrap.max_width = width;
    (ui.painter().layout_job(job), plain.chars().count())
}

/// Paint a `linked_text` at `pos`; true when its link is clicked (the
/// pointer's hand over it).
fn paint_linked(ui: &mut egui::Ui, galley: std::sync::Arc<egui::Galley>, pos: Pos2, link_at: usize, color: Color32) -> bool {
    let rect = Rect::from_min_size(pos, galley.size());
    let id = ui.scope_id().with(("linked_text", pos.x as i32, pos.y as i32));
    let r = ui.interact(rect, id, Sense::click());
    let on_link = r.hover_pos().is_some_and(|at| {
        galley.cursor_from_pos(at - pos).index >= egui::text::CharIndex(link_at)
            && galley.rows.iter().any(|row| row.rect().translate(pos.to_vec2()).contains(at))
    });
    ui.painter().galley(pos, galley, color);
    if on_link {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    on_link && r.clicked()
}

/// The amber banner over a page.
pub fn lock_bar(ui: &mut egui::Ui, p: &Palette, text: &str) {
    egui::Frame::new()
        .fill(p.amber.gamma_multiply(0.08))
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::glyph(ui, "lock-filled", p.amber, 14.0);
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.amber)).wrap());
            });
        });
    ui.add_space(10.0);
}

/// The faint line between two rows.
pub fn line(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, p.raised.gamma_multiply(0.35)),
    );
}

/// A setting: its name over its explanation on the left, its controls at
/// the right end, centred on the text. `controls_w` is the room they take
/// (the text wraps before it); they are drawn right to left, the last one
/// first.
pub fn srow(ui: &mut egui::Ui, p: &Palette, name: &str, desc: &str, controls_w: f32, controls: impl FnOnce(&mut egui::Ui)) {
    let text = RowText {
        name,
        lock: None,
        desc,
        link: None,
    };
    srow_ext(ui, p, &text, controls_w, controls);
}

/// The texts of a setting's row.
pub struct RowText<'a> {
    pub name: &'a str,
    /// An amber lock and who may change it, after the name ("estate owner").
    pub lock: Option<&'a str>,
    pub desc: &'a str,
    /// A link ending the explanation ("See in Region / Estate ›").
    pub link: Option<&'a str>,
}

/// `srow` with a lock after the name and a link after the explanation; true
/// when the link is clicked.
pub fn srow_ext(ui: &mut egui::Ui, p: &Palette, text: &RowText<'_>, controls_w: f32, controls: impl FnOnce(&mut egui::Ui)) -> bool {
    const PAD: f32 = 7.0;
    const GAP: f32 = 14.0;
    let width = ui.available_width();
    let text_w = (width - controls_w - GAP).max(120.0);
    let mut job = semibold(ui, text.name, 12.5, p.ink);
    job.wrap.max_width = text_w;
    let name_g = ui.painter().layout_job(job);
    let desc_g = match text.link {
        Some(link) => Some(linked_text(ui, p, text.desc, p.muted, 11.5, link, text_w)),
        None => (!text.desc.is_empty()).then(|| {
            let g = ui
                .painter()
                .layout(text.desc.to_owned(), FontId::proportional(11.5), p.muted, text_w);
            (g, usize::MAX)
        }),
    };
    let text_h = name_g.size().y + desc_g.as_ref().map_or(0.0, |(g, _)| 1.0 + g.size().y);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, text_h.max(26.0) + PAD * 2.0), Sense::hover());
    let top = rect.center().y - text_h / 2.0;
    let name_h = name_g.size().y;
    let name_end = name_g.rows.last().map_or(0.0, |r| r.rect().right());
    let first_row_h = name_g.rows.first().map_or(name_h, |r| r.rect().height());
    ui.painter().galley(egui::pos2(rect.left(), top), name_g, p.ink);
    if let Some(who) = text.lock {
        // the mockup's amber "owner only" tag after the name
        let at = egui::pos2(rect.left() + name_end + 6.0, top + first_row_h / 2.0);
        icon_at(ui, "lock-filled", egui::pos2(at.x + 6.0, at.y), 11.0, p.amber);
        let tag = ui.painter().layout_job(semibold(ui, who, 11.0, p.amber));
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &tag, at.x + 16.0, at.y);
        ui.painter().galley(at, tag, p.amber);
    }
    let mut clicked = false;
    if let Some((g, link_at)) = desc_g {
        let pos = egui::pos2(rect.left(), top + name_h + 1.0);
        if link_at == usize::MAX {
            ui.painter().galley(pos, g, p.muted);
        } else {
            clicked = paint_linked(ui, g, pos, link_at, p.muted);
        }
    }
    let slot = Rect::from_min_max(egui::pos2(rect.left() + text_w + GAP, rect.top()), rect.max);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(slot)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 8.0;
    controls(&mut child);
    clicked
}

/// The bar under a section whose settings wait for "Apply", like
/// Firestorm's tabs that send all their settings at once: "N changes not
/// sent yet", Cancel and Apply. Some(true) for Apply, Some(false) for
/// Cancel.
pub fn pending_bar(ui: &mut egui::Ui, p: &Palette, count: usize) -> Option<bool> {
    const H: f32 = 24.0;
    ui.add_space(8.0);
    let mut out = None;
    egui::Frame::new()
        .fill(p.violet.gamma_multiply(0.12))
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let row = Vec2::new(ui.available_width(), H);
            ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                widgets::glyph(ui, "pencil-filled", p.violet_pale, 13.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("sheet-pending-changes", n = count as i64))
                        .size(12.0)
                        .color(p.violet_pale),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let apply = aurora_i18n::tr!("common-apply");
                    let w = ui.painter().layout_job(semibold(ui, apply, 12.0, p.on_accent)).size().x + 20.0;
                    if widgets::primary_button_sized(ui, p, apply, Vec2::new(w, H)).clicked() {
                        out = Some(true);
                    }
                    let cancel = aurora_i18n::tr!("common-cancel");
                    if widgets::action_button(ui, p, "", cancel, widgets::ActionKind::Flat, Vec2::new(0.0, H)).clicked() {
                        out = Some(false);
                    }
                });
            });
        });
    out
}

/// A compact navigation entry: its icon, and its name only while it is
/// the lit one (the mockup's row of icons above the sections).
pub fn nav_compact(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, lit: bool) -> egui::Response {
    const H: f32 = 31.0;
    const ICON: f32 = 15.0;
    let ink = if lit { p.ink } else { p.muted };
    let galley = lit.then(|| ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(12.5), ink));
    let w = 16.0 + ICON + galley.as_ref().map_or(0.0, |g| 9.0 + g.size().x);
    let (rect, r) = ui.allocate_exact_size(Vec2::new(w, H), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if lit {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
        ui.painter().vline(
            rect.left() + 1.0,
            rect.top() + 6.0..=rect.bottom() - 6.0,
            egui::Stroke::new(2.0, p.violet),
        );
    } else if r.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    let icon_tint = if lit { p.violet_pale } else { p.muted };
    icon_at(
        ui,
        icon,
        egui::pos2(rect.left() + 8.0 + ICON / 2.0, rect.center().y),
        ICON,
        icon_tint,
    );
    if let Some(g) = galley {
        let pos = crate::ui::widgets::row_text_origin(ui.painter(), &g, rect.left() + 8.0 + ICON + 9.0, rect.center().y);
        ui.painter().galley(pos, g, ink);
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, lit, label));
    r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(label)
}

/// "Show" / "Hide" (or `label`) with its chevron: folds a part of a section.
pub fn fold_link(ui: &mut egui::Ui, p: &Palette, open: &mut bool, label: &str) {
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let text = ui.add(egui::Label::new(semibold(ui, label, 12.0, p.violet_light)).sense(Sense::click()));
            let icon = if *open { "chevron-down" } else { "chevron-right" };
            widgets::glyph(ui, icon, p.violet_light, 13.0);
            text
        })
        .inner;
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        *open = !*open;
    }
}

/// A fold link at the left of a row (the "Advanced" of a section).
pub fn fold_row(ui: &mut egui::Ui, p: &Palette, open: &mut bool, label: &str) {
    ui.add_space(6.0);
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let icon = if *open { "chevron-down" } else { "chevron-right" };
            widgets::glyph(ui, icon, p.violet_light, 13.0);
            ui.add(egui::Label::new(semibold(ui, label, 12.0, p.violet_light)).sense(Sense::click()))
        })
        .inner;
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        *open = !*open;
    }
    ui.add_space(2.0);
}

/// Width of the navigation of a column of sections.
pub const NAV_W: f32 = 200.0;

/// Which section of a column is in view, and the scrolling that follows it.
#[derive(Debug, Default)]
pub struct SectionScroll<S> {
    /// The section lit in the navigation.
    pub current: S,
    /// The section to bring to the top on the next frame.
    pub jump: Option<S>,
    /// Bring its end into view instead (a demo shows its bottom bar).
    pub jump_to_end: bool,
    /// Where the column was scrolled.
    scroll: f32,
    /// When a navigation click scrolled it (its animation is not the user's).
    jumped_at: f64,
}

impl<S: Copy> SectionScroll<S> {
    /// Light `sec` and bring it to the top.
    pub fn go(&mut self, sec: S) {
        self.current = sec;
        self.jump = Some(sec);
    }
}

/// A column of sections, each on its card, beside a navigation that
/// scrolls to them and follows the scrolling. `nav` draws a section's
/// entry (lit or not) and returns its response, `body` a section's card.
pub fn sections<S: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    list: &[S],
    state: &mut SectionScroll<S>,
    nav: impl FnMut(&mut egui::Ui, S, bool) -> egui::Response,
    body: impl FnMut(&mut egui::Ui, S),
) {
    sections_with(ui, p, id, list, state, false, nav, |_| {}, body);
}

/// `sections` with `tail` drawn after the navigation's entries (entries
/// that are not sections yet), and in a `compact` window the navigation as
/// a row above the column (`nav_compact`).
#[allow(clippy::too_many_arguments)]
pub fn sections_with<S: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    list: &[S],
    state: &mut SectionScroll<S>,
    compact: bool,
    mut nav: impl FnMut(&mut egui::Ui, S, bool) -> egui::Response,
    tail: impl FnOnce(&mut egui::Ui),
    body: impl FnMut(&mut egui::Ui, S),
) {
    if compact {
        ui.vertical(|ui| {
            // the mockup's 10 px between the row and the column, exactly
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // a long row is cut at the column's sides, never in height
                let (mut clip, max) = (ui.clip_rect(), ui.max_rect());
                clip.min.x = clip.min.x.max(max.min.x);
                clip.max.x = clip.max.x.min(max.max.x);
                ui.set_clip_rect(clip);
                for sec in list {
                    if nav(ui, *sec, state.current == *sec).clicked() {
                        state.go(*sec);
                    }
                }
                tail(ui);
            });
            ui.add_space(10.0);
            section_column(ui, p, id, list, state, body);
        });
        return;
    }
    let height = ui.available_height();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.allocate_ui_with_layout(Vec2::new(NAV_W, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(NAV_W);
            ui.spacing_mut().item_spacing.y = 2.0;
            for sec in list {
                if nav(ui, *sec, state.current == *sec).clicked() {
                    state.go(*sec);
                }
            }
            tail(ui);
        });
        ui.vertical(|ui| section_column(ui, p, id, list, state, body));
    });
}

/// The scrolling column of `sections`, each section on its card.
fn section_column<S: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    list: &[S],
    state: &mut SectionScroll<S>,
    mut body: impl FnMut(&mut egui::Ui, S),
) {
    ui.set_width(ui.available_width());
    let out = egui::ScrollArea::vertical().id_salt(id).auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width() - 6.0);
        let view_top = ui.clip_rect().top();
        let mut current = None;
        for sec in list {
            let mut c = card_begin(ui);
            body(&mut c.child, *sec);
            let rect = card_end(ui, p, c);
            if state.jump == Some(*sec) {
                let align = if state.jump_to_end { egui::Align::BOTTOM } else { egui::Align::TOP };
                ui.scroll_to_rect(rect, Some(align));
                state.jump = None;
                state.jump_to_end = false;
                state.jumped_at = ui.input(|i| i.time);
            } else if rect.top() <= view_top + 24.0 {
                current = Some(*sec);
            }
        }
        current
    });
    // the section in view follows the scrolling, not a jump that could not
    // bring the last sections to the top
    let offset = out.state.unclamped_offset().y;
    let settling = ui.input(|i| i.time) - state.jumped_at < 0.8;
    if (offset - state.scroll).abs() > 0.5 {
        state.scroll = offset;
        if state.jump.is_none()
            && !settling
            && let Some(sec) = out.inner
        {
            state.current = sec;
        }
    }
}

/// The head of a list card: its icon, title and count (`count` as "2 / 20"),
/// and `right` at the right end (its buttons).
pub fn list_title(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str, count: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 7.0;
        widgets::glyph(ui, icon, p.violet_light, 15.0);
        ui.label(semibold(ui, title, 13.0, p.ink));
        if !count.is_empty() {
            ui.label(semibold(ui, count, 11.0, p.muted_dim));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), right);
    });
}

/// A resident's display name and user name (when the settings show both).
pub fn names(world: &mut World, id: &Uuid) -> (String, Option<String>) {
    world.social.want_name(*id);
    let names = &world.social.avatar_names;
    match names.get(id) {
        Some(n) => (n.display(&names.options), n.chat_username(&names.options)),
        None => (world.social.name_of(id), None),
    }
}

/// A list row filtered by the search field: its name or user name.
pub fn matches(filter: &str, name: &str, username: Option<&str>) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || name.to_lowercase().contains(&f) || username.is_some_and(|u| u.to_lowercase().contains(&f))
}

/// Height of a resident's row.
pub const ROW_H: f32 = 40.0;

/// A resident's row of a list.
pub struct Person<'a> {
    pub id: Uuid,
    pub pic: Option<&'a egui::TextureHandle>,
    pub name: &'a str,
    pub user: Option<&'a str>,
    /// At the right end (time left, a date), amber when `timed`.
    pub exp: &'a str,
    pub timed: bool,
    /// A column before `exp`: `exp` and it then keep the widths of
    /// `person_head`'s columns (the estate's banned residents).
    pub mid: Option<&'a str>,
    /// A group: a square picture, no status, its profile on double click.
    pub group: bool,
    pub selected: bool,
    /// The box can be ticked.
    pub can: bool,
}

/// The widths of `Person::mid` and of `exp` when there is a `mid`.
const MID_W: f32 = 116.0;
const EXP_W: f32 = 96.0;
/// Where a row's names start: box, picture and their gaps.
const NAME_X: f32 = 26.0 + 9.0 + 30.0 + 9.0;

/// The x of a row's `mid` and `exp` columns, from its right end.
fn columns(right: f32) -> (f32, f32) {
    let exp = right - 8.0 - EXP_W;
    (exp - 8.0 - MID_W, exp)
}

/// The head of a list with `Person::mid` columns: the names', the middle
/// one's and the right one's titles, over the rows' columns.
pub fn person_head(ui: &mut egui::Ui, p: &Palette, width: f32, name: &str, mid: &str, exp: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let (mid_x, exp_x) = columns(rect.right());
    for (text, x) in [(name, rect.left() + NAME_X), (mid, mid_x), (exp, exp_x)] {
        let g = ui.painter().layout_job(semibold(ui, &text.to_uppercase(), 10.5, p.muted_dim));
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
        ui.painter().galley(at, g, p.muted_dim);
    }
}

/// What a click on a row asks for.
pub struct RowAnswer {
    pub toggled: bool,
    pub wants_pic: bool,
}

/// A resident's row: box, round picture with status, names, and `exp` at
/// the right end. A double click opens the profile.
pub fn person_row(ui: &mut egui::Ui, p: &Palette, world: &World, row: &Person<'_>, width: f32) -> RowAnswer {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if row.selected {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
    } else if r.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
    }
    // the box
    let mut on = row.selected;
    let boxed = Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 0.0), Vec2::new(20.0, ROW_H));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(boxed)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    if !row.can {
        child.disable();
    }
    let toggled = widgets::checkbox(&mut child, p, &mut on, "").changed() || (row.can && r.clicked());
    // picture and status
    let disc = Rect::from_center_size(egui::pos2(rect.left() + 26.0 + 9.0 + 15.0, rect.center().y), Vec2::splat(30.0));
    if row.group {
        widgets::avatar_picture(ui, p, disc, row.pic, row.name, false);
    } else {
        widgets::avatar_disc(ui, p, disc, row.pic, row.name, false);
        let status = if row.id == world.agent_id {
            presence::Status::Online
        } else {
            presence::of(world, &row.id)
        };
        if status != presence::Status::Unknown {
            presence::paint_dot(ui.painter(), p, disc, status, p.field);
        }
    }
    // the right end, then the names in the room before it
    let small = |text: &str, width: f32, color| {
        let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(11.5), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        ui.painter().layout_job(job)
    };
    let exp_color = if row.timed { p.amber } else { p.muted };
    let names_end = match row.mid {
        // two columns of the head's widths
        Some(mid) => {
            let (mid_x, exp_x) = columns(rect.right());
            for (g, x) in [(small(mid, MID_W, p.muted), mid_x), (small(row.exp, EXP_W, exp_color), exp_x)] {
                let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
                ui.painter().galley(at, g, p.muted);
            }
            mid_x - 8.0
        }
        None => {
            let exp_g = small(row.exp, f32::INFINITY, exp_color);
            let exp_w = exp_g.size().x;
            let at = crate::ui::widgets::row_text_origin(ui.painter(), &exp_g, rect.right() - 8.0 - exp_w, rect.center().y);
            ui.painter().galley(at, exp_g, p.muted);
            rect.right() - 16.0 - exp_w
        }
    };
    let text_left = disc.right() + 9.0;
    let text_w = (names_end - text_left).max(20.0);
    let mut job = semibold(ui, row.name, 12.5, p.ink);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
    let name_g = ui.painter().layout_job(job);
    let user_g = row.user.map(|u| {
        let mut job = egui::text::LayoutJob::simple_singleline(u.to_owned(), FontId::proportional(11.5), p.muted);
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_w);
        ui.painter().layout_job(job)
    });
    let h = name_g.size().y + user_g.as_ref().map_or(0.0, |g| g.size().y);
    let top = rect.center().y - h / 2.0;
    let name_h = name_g.size().y;
    ui.painter().galley(egui::pos2(text_left, top), name_g, p.ink);
    if let Some(g) = user_g {
        ui.painter().galley(egui::pos2(text_left, top + name_h), g, p.muted);
    }
    if r.double_clicked() {
        if row.group {
            crate::ui::group_profile::request_open(ui.ctx(), row.id);
        } else {
            crate::ui::profile::request_open(ui.ctx(), row.id);
        }
    }
    RowAnswer {
        toggled,
        wants_pic: row.pic.is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header's picture keeps its aspect: a square picture fills the
    /// width and loses its top and bottom.
    #[test]
    fn a_picture_covers_its_frame() {
        let frame = Vec2::new(220.0, 148.0);
        let full = cover_uv(220, 148, frame);
        assert!((full.width() - 1.0).abs() < 1e-5 && (full.height() - 1.0).abs() < 1e-5);
        let square = cover_uv(512, 512, frame);
        assert!((square.width() - 1.0).abs() < 1e-5);
        assert!((square.height() - 148.0 / 220.0).abs() < 1e-5);
        assert!((square.center().y - 0.5).abs() < 1e-5);
        let wide = cover_uv(1024, 256, frame);
        assert!((wide.height() - 1.0).abs() < 1e-5 && wide.width() < 1.0);
    }

    #[test]
    fn the_search_finds_a_name_or_a_user_name() {
        assert!(matches("", "Nova", None));
        assert!(matches(" nov ", "Nova ♪", None));
        assert!(matches("example", "Nova ♪", Some("nova.example")));
        assert!(!matches("tess", "Nova ♪", Some("nova.example")));
    }

    /// A navigation click lights its section and asks for the jump.
    #[test]
    fn going_to_a_section_lights_it() {
        let mut s = SectionScroll::<u8>::default();
        s.go(3);
        assert_eq!((s.current, s.jump), (3, Some(3)));
    }
}
