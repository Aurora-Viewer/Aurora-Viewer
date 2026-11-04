//! The tabs, filter pills and search box of the redrawn windows (group
//! profile, About Land, People): first written for one window each, shared
//! here once a second window needed them.

use crate::theme::Palette;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// One tab of a row of tabs in pills: an optional Tabler icon, its name, a
/// count after it (in a violet pill when `accent`, an unread count). The
/// selected tab sits on `raised`.
#[allow(clippy::too_many_arguments)]
pub fn pill_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<String>,
    accent: bool,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    tab(ui, p, icon, label, count, accent, selected, enabled, None, None)
}

/// A `pill_tab` of the given width, its icon and name in the middle (the
/// two halves of the Controls window), greyed while it does not apply,
/// `why` on hover (the Movement tab while the photo's free camera flies).
#[allow(clippy::too_many_arguments)]
pub fn pill_tab_wide(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    selected: bool,
    width: f32,
    enabled: bool,
    why: &str,
) -> egui::Response {
    tab(ui, p, icon, label, None, false, selected, enabled, Some(width), Some(why))
}

#[allow(clippy::too_many_arguments)]
fn tab(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    count: Option<String>,
    accent: bool,
    selected: bool,
    enabled: bool,
    width: Option<f32>,
    // what a greyed tab says on hover ("not yet" by default)
    disabled_tip: Option<&str>,
) -> egui::Response {
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if !enabled {
        p.muted_dim
    } else if selected {
        p.ink
    } else {
        p.muted
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let count_ink = if accent { p.on_accent } else { p.muted_dim };
    let count = count.map(|t| {
        ui.painter()
            .layout_no_wrap(t, FontId::new(11.0, crate::ui::fonts::semibold(ui.ctx())), count_ink)
    });
    let pad = if accent { 5.0 } else { 0.0 };
    let count_w = count.as_ref().map_or(0.0, |g| 5.0 + g.size().x + pad * 2.0);
    let icon_w = if icon.is_empty() { 0.0 } else { 15.0 + 5.0 };
    let content = icon_w + galley.size().x + count_w;
    let size = Vec2::new(width.unwrap_or(12.0 + content + 12.0), 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if selected {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let mut x = match width {
        Some(_) => rect.center().x - content / 2.0,
        None => rect.left() + 12.0,
    };
    if !icon.is_empty() {
        if let Some(t) = crate::ui::icons::global(icon) {
            let r = Rect::from_center_size(Pos2::new(x + 7.5, rect.center().y), Vec2::splat(15.0));
            ui.painter()
                .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
        }
        x += icon_w;
    }
    let w = galley.size().x;
    crate::ui::widgets::paint_row_text(ui.painter(), galley, x, rect.center().y, ink);
    if let Some(g) = count {
        if accent {
            // an unread count: its digits in the middle of their pill
            let pill = Rect::from_min_size(
                Pos2::new(x + w + 5.0, rect.center().y - 8.0),
                Vec2::new(g.size().x + pad * 2.0, 16.0),
            );
            ui.painter().rect_filled(pill, ui.visuals().window_corner_radius, p.violet);
            crate::ui::widgets::paint_ink_centered(ui.painter(), g, pill.center(), count_ink);
        } else {
            let pos = crate::ui::widgets::row_text_origin(ui.painter(), &g, x + w + 5.0, rect.center().y);
            ui.painter().galley(pos, g, count_ink);
        }
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, label));
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        let tip = match disabled_tip {
            Some(tip) => tip.to_owned(),
            None => crate::ui::menu::not_yet().to_owned(),
        };
        resp.on_hover_text(tip)
    }
}

/// A filter or sort pill of a list's bar: lit in the accent while `on`,
/// with an optional Tabler icon before its label.
pub fn filter_pill(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(filter_pill_width(ui, icon, label), 26.0), Sense::click());
    paint_filter_pill(ui, p, rect, &resp, icon, label, on);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `filter_pill` at a place of its own (a row's right end laid out apart).
pub fn filter_pill_at(ui: &mut egui::Ui, p: &Palette, rect: Rect, id: egui::Id, icon: &str, label: &str, on: bool) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    paint_filter_pill(ui, p, rect, &resp, icon, label, on);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_filter_pill(ui: &egui::Ui, p: &Palette, rect: Rect, resp: &egui::Response, icon: &str, label: &str, on: bool) {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if on { p.ink } else { p.muted };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let icon_w = if icon.is_empty() { 0.0 } else { 13.0 + 5.0 };
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 9.0;
    if !icon.is_empty() {
        if let Some(t) = crate::ui::icons::global(icon) {
            let r = Rect::from_center_size(Pos2::new(x + 6.5, rect.center().y), Vec2::splat(13.0));
            ui.painter()
                .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), ink);
        }
        x += icon_w;
    }
    crate::ui::widgets::paint_row_text(ui.painter(), galley, x, rect.center().y, ink);
}

/// The width `dot_pill` takes.
pub fn dot_pill_width(ui: &egui::Ui, label: &str, count: Option<&str>, close: bool) -> f32 {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let w = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    9.0 + 9.0 + 6.0 + w(label) + count.map_or(0.0, |c| 5.0 + w(c)) + if close { 5.0 + 12.0 } else { 0.0 } + 9.0
}

/// A filter pill led by a dot of `dot` (a contact set's color), with a
/// count after its label, or a cross that takes it away (`close`). Returns
/// the pill's response and whether its cross was clicked.
pub fn dot_pill(
    ui: &mut egui::Ui,
    p: &Palette,
    dot: egui::Color32,
    label: &str,
    count: Option<&str>,
    on: bool,
    close: bool,
) -> (egui::Response, bool) {
    let width = dot_pill_width(ui, label, count, close);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if on || close { p.ink } else { p.muted };
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let mut x = rect.left() + 9.0;
    ui.painter().circle_filled(Pos2::new(x + 4.5, rect.center().y), 4.5, dot);
    x += 9.0 + 6.0;
    let g = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), ink);
    let w = g.size().x;
    crate::ui::widgets::paint_row_text(ui.painter(), g, x, rect.center().y, ink);
    x += w;
    if let Some(c) = count {
        let g = ui.painter().layout_no_wrap(c.to_owned(), font, ink);
        crate::ui::widgets::paint_row_text(ui.painter(), g, x + 5.0, rect.center().y, ink);
    }
    let mut closed = false;
    if close {
        let cross = Rect::from_center_size(Pos2::new(rect.right() - 9.0 - 6.0, rect.center().y), Vec2::splat(16.0));
        let c = ui.interact(cross, resp.id.with("close"), Sense::click());
        let tint = if c.hovered() { p.ink } else { p.muted };
        if let Some(t) = crate::ui::icons::global("x") {
            ui.painter().image(
                t.id(),
                Rect::from_center_size(cross.center(), Vec2::splat(12.0)),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                tint,
            );
        }
        closed = c.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
    }
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), closed)
}

/// The width `filter_pill` takes.
pub fn filter_pill_width(ui: &egui::Ui, icon: &str, label: &str) -> f32 {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    (if icon.is_empty() { 0.0 } else { 13.0 + 5.0 }) + w + 18.0
}

/// The search field of a list: a magnifier then the text, on the field's
/// face, with the clear cross of `clear_button` while it holds text. Every
/// search and filter goes through it, in any layout: it takes the height of
/// its line.
pub fn search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    search_box_with(ui, p, text, hint, width, |edit| edit)
}

/// `search_box` whose `style` finishes the edit (an id, an event filter…).
pub fn search_box_with(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &mut String,
    hint: &str,
    width: f32,
    style: impl for<'a> FnOnce(egui::TextEdit<'a>) -> egui::TextEdit<'a>,
) -> egui::Response {
    search_face(ui, p, "search-filled", width, |ui| {
        let margin = face_clear_margin(ui);
        let edit = egui::TextEdit::singleline(&mut *text)
            .frame(egui::Frame::NONE.inner_margin(margin))
            .hint_text(hint)
            .desired_width(f32::INFINITY);
        let mut response = ui.add(style(edit));
        clear_button(ui, p, text, &mut response);
        response
    })
}

/// `AURORA_DEMO_FIELD_TEXT`: a field with the clear cross starts holding this
/// text, once, so a capture of any window shows the cross and the filter's
/// effect, in demo mode only.
pub fn demo_field_text(ui: &egui::Ui, text: &mut String, response: &mut egui::Response) {
    static DEMO: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let Some(demo) = DEMO.get_or_init(|| {
        std::env::var("AURORA_DEMO_FIELD_TEXT")
            .ok()
            .filter(|_| std::env::var_os("AURORA_DEMO").is_some())
    }) else {
        return;
    };
    let seen = response.id.with("demo_field_text");
    if !ui.data(|d| d.get_temp::<bool>(seen).unwrap_or(false)) {
        ui.data_mut(|d| d.insert_temp(seen, true));
        if text.is_empty() {
            text.clone_from(demo);
            response.mark_changed();
        }
    }
}

/// A plain filter input with the same clear button as `search_box`, for a
/// column of a table of filters (Area Search), where a face per cell would
/// be noise.
pub fn filter_field(ui: &mut egui::Ui, p: &Palette, text: &mut String, id: egui::Id, width: f32) -> egui::Response {
    let margin = egui::Margin::symmetric(4, 2) + bare_clear_margin(ui);
    let mut response = ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::TextEdit::singleline(text).id(id).margin(margin),
    );
    clear_button(ui, p, text, &mut response);
    response
}

/// `bare_clear_margin` of a field in `search_face`.
pub fn face_clear_margin(ui: &egui::Ui) -> egui::Margin {
    let mut margin = bare_clear_margin(ui);
    margin.left = FACE_TEXT_LEFT;
    margin
}

/// The margin of a field with no frame of its own, laid on a face, with the
/// room of `clear_button`, kept even while empty so the first letter moves
/// nothing.
pub fn bare_clear_margin(ui: &egui::Ui) -> egui::Margin {
    let mut margin = super::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    margin.right += CLEAR_WIDTH;
    margin
}

/// The face of a field of the redrawn windows and menus: the search box's,
/// the SLURL of the place menu.
pub fn field_frame(ui: &egui::Ui, p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(9, 4))
}

/// A text field on that face, a Tabler `icon` before the text.
pub fn icon_field(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &mut String, hint: &str, width: f32) -> egui::Response {
    search_face(ui, p, icon, width, |ui| {
        let mut margin = super::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
        margin.left = FACE_TEXT_LEFT;
        ui.add(
            egui::TextEdit::singleline(text)
                .frame(egui::Frame::NONE.inner_margin(margin))
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        )
    })
}

/// The search box's face around `body`, a Tabler `icon` before it, laid
/// under the field (`face_clear_margin` leaves it room) so that a click on it
/// reaches the field. Left to right even in a row laid out from the right,
/// and one line high even in a column, whose full height a plain
/// `with_layout` would take.
pub fn search_face<R>(ui: &mut egui::Ui, p: &Palette, icon: &str, width: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    search_face_sized(ui, p, icon, width, 0.0, body)
}

/// `search_face` at least `height` high, for a toolbar's row of that height.
pub fn search_face_sized<R>(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    width: f32,
    height: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    field_frame(ui, p)
        .show(ui, |ui| {
            ui.set_width((width - 18.0).max(0.0));
            // from the top: a row's centring would push the line down the
            // face and stretch it, and a tree window's taller rows do not
            // reach the line, as high as its text unless `height` says more
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                let line = egui::vec2(ui.available_width(), (height - 8.0).max(0.0));
                let shown = ui.allocate_ui_with_layout(line, egui::Layout::left_to_right(egui::Align::Center), body);
                let at = egui::pos2(shown.response.rect.left() + GLYPH / 2.0, shown.response.rect.center().y);
                super::paint_glyph(ui.painter(), icon, at, GLYPH, p.muted);
                shown.inner
            })
            .inner
        })
        .inner
}

const CLEAR_WIDTH: i8 = 20;
/// The side of `search_face`'s icon.
const GLYPH: f32 = 13.0;
/// The room of that icon before the text, its gap included.
const FACE_TEXT_LEFT: i8 = 20;

/// The cross at the right end of a field whose margin keeps its room
/// (`bare_clear_margin`): empties `text`, reports the field changed and gives
/// it the keyboard back. Only while the field holds text and can be typed
/// in. As the Legacy Viewer's filter editors (LLSearchEditor, `clear_button`).
pub fn clear_button(ui: &mut egui::Ui, p: &Palette, text: &mut String, response: &mut egui::Response) {
    demo_field_text(ui, text, response);
    if text.is_empty() || !ui.is_enabled() {
        return;
    }
    if clear_cross(ui, p, response, aurora_i18n::tr!("common-clear")) {
        text.clear();
        response.mark_changed();
        response.request_focus();
    }
}

/// The cross alone, true when clicked, for a field that clears more than its
/// text (the inventory's filters). Clicks only, so that Tab goes from field
/// to field.
pub fn clear_cross(ui: &mut egui::Ui, p: &Palette, field: &egui::Response, label: &str) -> bool {
    let rect = Rect::from_min_max(
        egui::pos2(field.rect.right() - f32::from(CLEAR_WIDTH), field.rect.top()),
        field.rect.right_bottom(),
    );
    let clear = ui.interact(rect, field.id.with("clear"), Sense::CLICK);
    super::paint_title_button(ui, p, "x", &clear);
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    clear.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
