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

/// A G / M / A toggle: its badge, and its name when `label` (a compact
/// window shows the badge alone, the name in its tooltip), lit on `raised`
/// while on; a rating the account may not see stays off and says why.
/// Shared by Aurora's Search and Destinations.
pub fn rating_toggle(ui: &mut egui::Ui, p: &Palette, on: &mut bool, name: &str, allowed: bool, label: bool) -> bool {
    let font = FontId::proportional(12.0);
    let lit = *on && allowed;
    let ink = if lit { p.ink } else { p.muted_dim };
    let galley = label.then(|| ui.painter().layout_no_wrap(name.to_owned(), font, ink));
    let text_w = galley.as_ref().map_or(0.0, |g| 6.0 + g.size().x + 3.0);
    let size = Vec2::new(6.0 + 16.0 + 6.0 + text_w, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, if allowed { Sense::click() } else { Sense::hover() });
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let fill = if lit {
        p.raised
    } else if allowed && resp.hovered() {
        p.raised.gamma_multiply(0.5)
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    let left = if label { rect.left() + 6.0 } else { rect.center().x - 8.0 };
    let badge = Rect::from_center_size(egui::pos2(left + 8.0, rect.center().y), Vec2::splat(16.0));
    let mut painter = ui.painter().clone();
    if !lit {
        painter.multiply_opacity(0.35);
    }
    super::paint_maturity_badge(&painter, p, name, badge, super::inner_radius(ui.visuals()));
    if let Some(g) = galley {
        super::paint_row_text(ui.painter(), g, badge.right() + 6.0, rect.center().y, ink);
    }
    let resp = if !allowed {
        resp.on_hover_text(aurora_i18n::tr!("search-rating-not-allowed"))
    } else if label {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(name)
    };
    if allowed && resp.clicked() {
        *on = !*on;
        return true;
    }
    false
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

/// The search field of a list: a magnifier then the text, on the field's face.
pub fn search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    icon_field(ui, p, "search-filled", text, hint, width)
}

/// `search_box` with an in-field clear button while filled.
pub fn clearable_search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    icon_field_inner(ui, p, "search-filled", text, hint, width, true)
}

/// A plain filter input with the same clear button as `clearable_search_box`.
pub fn filter_field(ui: &mut egui::Ui, p: &Palette, text: &mut String, id: egui::Id, width: f32) -> egui::Response {
    let mut margin = egui::Margin::symmetric(4, 2) + super::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    if !text.is_empty() {
        margin.right += CLEAR_WIDTH;
    }
    let mut response = ui.add_sized(
        [width, ui.spacing().interact_size.y],
        egui::TextEdit::singleline(text).id(id).margin(margin),
    );
    select_on_focus(ui, &response, text);
    clear_filter(ui, p, text, &mut response);
    response
}

/// A configurable search/filter field with the shared in-field clear action.
/// Keep the editor's id, keyboard handling and caller's change notifications.
pub fn search_text_edit(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &mut String,
    configure: impl for<'a> FnOnce(egui::TextEdit<'a>) -> egui::TextEdit<'a>,
) -> egui::Response {
    let margin = egui::Margin::symmetric(4, 2) + super::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
    search_text_edit_with_margin(ui, p, text, margin, configure)
}

/// The same editor on an existing custom field face, retaining its padding.
pub fn search_text_edit_with_margin(
    ui: &mut egui::Ui,
    p: &Palette,
    text: &mut String,
    mut margin: egui::Margin,
    configure: impl for<'a> FnOnce(egui::TextEdit<'a>) -> egui::TextEdit<'a>,
) -> egui::Response {
    let was_filled = !text.is_empty();
    // LLSearchEditor (indra/llui/llsearcheditor.cpp) reserves the clear
    // button's room. Keep it reserved while empty too, so typing cannot
    // hide the end of a query for one frame or move the surrounding layout.
    margin.right += CLEAR_WIDTH;
    let mut response = ui.add(configure(egui::TextEdit::singleline(text)).margin(margin));
    select_on_focus(ui, &response, text);
    clear_filter(ui, p, text, &mut response);
    note_search_clear(ui, &response, was_filled && text.is_empty());
    response
}

/// A search field that takes the keyboard without a click (a window that
/// reopens on its kept search, a Tab) selects its whole text, so the next
/// character replaces the old search instead of continuing it. A click puts
/// the cursor where it landed, like any field.
/// Port of LLLineEditor's `mSelectAllonFocusReceived` (indra/llui/lllineeditor.cpp),
/// which the Legacy Viewer's filter fields take on a keyboard focus.
pub fn select_on_focus(ui: &egui::Ui, response: &egui::Response, text: &str) {
    // egui's own `gained_focus` is not reported for a focus asked for after
    // the field was drawn, which is how the windows ask on opening: the field
    // remembers instead whether it held the keyboard in the frame before. A
    // field not drawn for a frame (its window closed) forgets it. Enter
    // lets a single-line field go for a frame, and the find bars take it back:
    // that is the same hold, so the caret they kept stays.
    // (a window's invisible sizing pass draws nothing the user sees: it
    // neither counts as a drawn frame nor takes the keyboard)
    if ui.is_sizing_pass() {
        return;
    }
    let key = response.id.with("select_on_focus");
    let frame = ui.ctx().cumulative_frame_nr();
    let focused = response.has_focus();
    let held = focused || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    let last = ui.ctx().data_mut(|d| d.get_temp::<(u64, bool)>(key));
    let drawn_last = last.is_some_and(|(seen, _)| seen + 1 >= frame);
    let had = last.is_some_and(|(seen, was)| was && seen + 1 >= frame);
    ui.ctx().data_mut(|d| d.insert_temp(key, (frame, held)));
    if text.is_empty() {
        return;
    }
    // A window that reopens on the search it kept takes the keyboard for it
    // (the field was not drawn last frame and its window was not shown
    // either: a tab change inside an open window does not count), unless
    // another field already holds it (typing in the chat bar).
    if !drawn_last && !focused && take_opening(ui) {
        response.request_focus();
        ui.ctx().request_repaint();
        return;
    }
    if !focused || had {
        return;
    }
    let by_pointer = response.contains_pointer() && ui.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
    if by_pointer {
        return;
    }
    let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(0),
        egui::text::CCursor::new(text.chars().count()),
    )));
    state.store(ui.ctx(), response.id);
    ui.ctx().request_repaint();
}

/// How long after it opens a window may hand the keyboard to its search
/// (frames): the field can be drawn a little later than the window, once the
/// list it filters has loaded.
const OPENING_FRAMES: u64 = 90;

fn opened_key(window: egui::Id) -> egui::Id {
    egui::Id::unique("search_window_opened").with(window)
}

/// Called by the windows (`Floater::show`, `bare_window`) on every frame they
/// are drawn: the first one after a gap is an opening, which lets the first
/// search field of the window with a kept text take the keyboard
/// (`select_on_focus`). A tab change inside an open window is not one.
pub(crate) fn note_window_drawn(ctx: &egui::Context, window: egui::Id) {
    let frame = ctx.cumulative_frame_nr();
    let key = egui::Id::unique("search_window_drawn").with(window);
    let last = ctx.data_mut(|d| d.get_temp::<u64>(key));
    ctx.data_mut(|d| d.insert_temp(key, frame));
    if last.is_none_or(|last| last + 1 < frame) {
        ctx.data_mut(|d| d.insert_temp(opened_key(window), frame));
    }
}

/// The window holding `ui` opened a moment ago, and nothing else holds the
/// keyboard. One field takes it: the opening is spent.
fn take_opening(ui: &egui::Ui) -> bool {
    if !ui.is_enabled() || ui.ctx().memory(|m| m.focused().is_some()) {
        return false;
    }
    let ctx = ui.ctx();
    let frame = ctx.cumulative_frame_nr();
    let key = opened_key(ui.layer_id().id);
    let recent = ctx
        .data_mut(|d| d.get_temp::<u64>(key))
        .is_some_and(|opened| frame.saturating_sub(opened) <= OPENING_FRAMES);
    if recent {
        ctx.data_mut(|d| d.remove_temp::<u64>(key));
    }
    recent
}

fn note_search_clear(ui: &egui::Ui, response: &egui::Response, emptied: bool) {
    if emptied && response.changed() {
        let key = egui::Id::unique("search_selection_reveal").with(ui.layer_id());
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx().data_mut(|data| data.insert_temp(key, frame));
        ui.ctx().request_repaint();
    }
}

/// Clearing a search keeps its list's selection in view. The next frame is
/// included for lists whose projection was computed before their search bar.
/// Extends LLFloaterTexturePicker::onFilterEdit / LLFolderView::scrollToShowSelection
/// (Legacy Viewer, indra/newview/lltexturectrl.cpp and indra/llui/llfolderview.cpp)
/// to the other filtered lists. Window/popup layers isolate unrelated filters.
pub fn search_cleared(ui: &egui::Ui) -> bool {
    let key = egui::Id::unique("search_selection_reveal").with(ui.layer_id());
    ui.ctx()
        .data(|data| data.get_temp::<u64>(key))
        .is_some_and(|frame| ui.ctx().cumulative_frame_nr().saturating_sub(frame) <= 1)
}

/// For ordinary lists, reveal a selected row without moving keyboard focus.
pub fn reveal_search_selection(ui: &egui::Ui, response: &egui::Response, selected: bool) {
    if selected && search_cleared(ui) {
        response.scroll_to_me_animation(Some(egui::Align::Center), egui::style::ScrollAnimation::none());
    }
}

pub fn search_list_item(ui: &mut egui::Ui, selected: bool, text: impl Into<egui::WidgetText>) -> egui::Response {
    let response = ui.selectable_label(selected, text);
    reveal_search_selection(ui, &response, selected);
    response
}

/// A virtual row must be brought into the viewport before it can be drawn.
pub fn search_scroll_to_row(ui: &egui::Ui, scroll: egui::ScrollArea, row: Option<usize>, height: f32) -> egui::ScrollArea {
    if search_cleared(ui)
        && let Some(row) = row
    {
        scroll
            .animated(false)
            .vertical_scroll_offset(row as f32 * (height + ui.spacing().item_spacing.y))
    } else {
        scroll
    }
}

pub fn search_scroll_to_column(ui: &egui::Ui, scroll: egui::ScrollArea, column: Option<usize>, width: f32) -> egui::ScrollArea {
    if search_cleared(ui)
        && let Some(column) = column
    {
        scroll.animated(false).horizontal_scroll_offset(column as f32 * width)
    } else {
        scroll
    }
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
    icon_field_inner(ui, p, icon, text, hint, width, false)
}

fn icon_field_inner(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &mut String, hint: &str, width: f32, clear: bool) -> egui::Response {
    let was_filled = !text.is_empty();
    let response = field_frame(ui, p)
        .show(ui, |ui| {
            ui.set_width(width - 18.0);
            // left to right even in a row laid out from the right
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                super::glyph(ui, icon, p.muted, 13.0);
                let mut centre = super::field_text_margin(ui.ctx(), &egui::TextStyle::Body.resolve(ui.style()));
                if clear && !text.is_empty() {
                    centre.right += CLEAR_WIDTH;
                }
                let mut response = ui.add(
                    egui::TextEdit::singleline(text)
                        .frame(egui::Frame::NONE.inner_margin(centre))
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                );
                select_on_focus(ui, &response, text);
                if clear {
                    clear_filter(ui, p, text, &mut response);
                }
                response
            })
            .inner
        })
        .inner;
    note_search_clear(ui, &response, clear && was_filled && text.is_empty());
    response
}

const CLEAR_WIDTH: i8 = 20;

fn clear_filter(ui: &mut egui::Ui, p: &Palette, text: &mut String, response: &mut egui::Response) {
    if text.is_empty() {
        return;
    }
    let rect = Rect::from_min_max(
        egui::pos2(response.rect.right() - f32::from(CLEAR_WIDTH), response.rect.top()),
        response.rect.right_bottom(),
    );
    let clear = ui.interact(rect, response.id.with("clear"), Sense::click());
    super::paint_title_button(ui, p, "x", &clear);
    let label = aurora_i18n::tr!("common-clear");
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if clear.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        text.clear();
        response.mark_changed();
        response.request_focus();
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    struct ListFrame {
        field: egui::Response,
        selected_rect: Option<Rect>,
        clip: Rect,
        cleared: bool,
    }

    fn list_frame(
        ctx: &egui::Context,
        kind: usize,
        query: &mut String,
        selected: &mut Option<usize>,
        events: Vec<egui::Event>,
    ) -> ListFrame {
        let p = crate::theme::Theme::default().palette();
        let mut result = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let before: Vec<_> = (0..150).filter(|i| format!("Item {i:03}").contains(query.as_str())).collect();
                let field = search_text_edit(ui, &p, query, |edit| edit.id(egui::Id::unique("list_search")).desired_width(240.0));
                let rows: Vec<_> = if kind >= 4 {
                    before
                } else {
                    (0..150).filter(|i| format!("Item {i:03}").contains(query.as_str())).collect()
                };
                let hierarchy = kind == 8;
                let unfiltered = query.is_empty();
                let kind = kind % 4;
                let index = rows.iter().position(|i| Some(*i) == *selected);
                let mut selected_rect = None;
                let mut clip = Rect::NOTHING;
                let mut row = |ui: &mut egui::Ui, i: usize| {
                    let (_, response) = ui.allocate_exact_size(Vec2::new(48.0, 24.0), Sense::click());
                    if response.clicked() {
                        *selected = Some(i);
                    }
                    if *selected == Some(i) || rows.len() == 1 {
                        selected_rect = Some(response.rect);
                    }
                    clip = ui.clip_rect();
                    if kind == 0 {
                        reveal_search_selection(ui, &response, *selected == Some(i));
                    }
                };
                match if hierarchy { 4 } else { kind } {
                    0 => {
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            for &i in &rows {
                                row(ui, i);
                            }
                        });
                    }
                    1 => {
                        search_scroll_to_row(ui, egui::ScrollArea::vertical(), index, 24.0)
                            .max_height(120.0)
                            .show_rows(ui, 24.0, rows.len(), |ui, range| {
                                for &i in &rows[range] {
                                    row(ui, i);
                                }
                            });
                    }
                    2 => {
                        search_scroll_to_row(ui, egui::ScrollArea::vertical(), index.map(|i| i / 2), 24.0)
                            .max_height(120.0)
                            .show_rows(ui, 24.0, rows.len().div_ceil(2), |ui, range| {
                                for index in range {
                                    ui.horizontal(|ui| {
                                        for &i in rows.iter().skip(index * 2).take(2) {
                                            row(ui, i);
                                        }
                                    });
                                }
                            });
                    }
                    4 => {
                        let open = search_cleared(ui).then_some(true);
                        egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                            if unfiltered {
                                super::super::collapsing_forced(ui, &p, "parent", "Parent", false, open, |ui| {
                                    super::super::collapsing_forced(ui, &p, "child", "Child", false, open, |ui| {
                                        for &i in &rows {
                                            row(ui, i);
                                        }
                                    });
                                });
                            } else {
                                for &i in &rows {
                                    row(ui, i);
                                }
                            }
                        });
                    }
                    _ => {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        search_scroll_to_column(ui, egui::ScrollArea::horizontal(), index, 48.0).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for &i in &rows {
                                    row(ui, i);
                                }
                            });
                        });
                    }
                }
                result = Some(ListFrame {
                    field,
                    selected_rect,
                    clip,
                    cleared: search_cleared(ui),
                });
            },
        );
        output.textures_delta.clear();
        result.expect("list drawn")
    }

    #[test]
    fn clearing_search_reveals_the_chosen_row_in_lists_virtual_rows_grids_and_shelves() {
        for kind in 0..9 {
            for method in 0..3 {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                let mut query = "099".to_owned();
                let mut selected = None;
                let mut frame = list_frame(&ctx, kind, &mut query, &mut selected, Vec::new());
                for _ in 0..3 {
                    frame = list_frame(&ctx, kind, &mut query, &mut selected, Vec::new());
                }
                let pos = frame.selected_rect.expect("filtered candidate").center();
                for pressed in [true, false] {
                    list_frame(&ctx, kind, &mut query, &mut selected, click(pos, pressed));
                }
                assert_eq!(selected, Some(99));
                let pos = if method == 0 {
                    ctx.read_response(frame.field.id.with("clear")).expect("clear action").rect.center()
                } else {
                    frame.field.rect.center()
                };
                for pressed in [true, false] {
                    frame = list_frame(&ctx, kind, &mut query, &mut selected, click(pos, pressed));
                }
                if method != 0 {
                    let modifiers = egui::Modifiers {
                        ctrl: true,
                        command: true,
                        ..Default::default()
                    };
                    frame = list_frame(
                        &ctx,
                        kind,
                        &mut query,
                        &mut selected,
                        vec![
                            egui::Event::Key {
                                key: egui::Key::A,
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers,
                            },
                            egui::Event::Key {
                                key: if method == 1 { egui::Key::Delete } else { egui::Key::Backspace },
                                physical_key: None,
                                pressed: true,
                                repeat: false,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                    );
                }
                assert!(query.is_empty(), "kind={kind}, method={method}");
                assert!(frame.cleared);
                for _ in 0..4 {
                    frame = list_frame(&ctx, kind, &mut query, &mut selected, Vec::new());
                }
                assert_eq!(selected, Some(99), "clearing keeps the same row");
                let rect = frame.selected_rect.expect("the selected virtual row must be laid out");
                assert!(
                    frame.clip.contains(rect.center()),
                    "kind={kind}, method={method}: {rect:?} outside {:?}",
                    frame.clip
                );
                assert!(frame.field.has_focus(), "revealing must leave focus on the search field");
                assert!(!frame.cleared, "the reveal expires so manual scrolling remains possible");
            }
        }
    }

    #[test]
    fn clearing_one_window_does_not_reveal_another_windows_selection() {
        let ctx = egui::Context::default();
        for frame in 0..4 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
                for window in 0..2 {
                    egui::Window::new(format!("List {window}")).show(&ctx, |ui| {
                        if frame == 0 && window == 0 {
                            let mut response = ui.label("Cleared search");
                            response.mark_changed();
                            note_search_clear(ui, &response, true);
                        }
                        assert_eq!(search_cleared(ui), window == 0 && frame <= 1);
                    });
                }
            });
            output.textures_delta.clear();
        }
    }

    /// One frame of a lone search field, with the id the tests share.
    fn lone_field(ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>, focus: bool) {
        let p = crate::theme::Theme::default().palette();
        let id = egui::Id::unique("lone_search");
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0))),
                events,
                ..Default::default()
            },
            |ui| {
                search_text_edit(ui, &p, text, |edit| edit.id(id).desired_width(240.0));
                if focus {
                    ui.ctx().memory_mut(|m| m.request_focus(id));
                }
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn a_search_that_takes_the_keyboard_selects_its_text_so_typing_replaces_it() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut text = String::new();
        lone_field(&ctx, &mut text, Vec::new(), false);
        text = "old search".to_owned();
        // The window asks for the keyboard, like the inventory.
        lone_field(&ctx, &mut text, Vec::new(), true);
        lone_field(&ctx, &mut text, Vec::new(), false);
        let range = egui::TextEdit::load_state(&ctx, egui::Id::unique("lone_search")).and_then(|s| s.cursor.char_range());
        let all = Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(10),
        ));
        assert_eq!(range, all, "the whole old search is selected");
        lone_field(&ctx, &mut text, vec![egui::Event::Text("n".into())], false);
        assert_eq!(text, "n", "the first character replaces the selection");
    }

    /// One frame of a window whose search field is drawn or not, with the id
    /// the window tests share.
    fn window_field(ctx: &egui::Context, text: &mut String, window: bool, field: bool, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0))),
                events,
                ..Default::default()
            },
            |_| {
                if window {
                    // what `Floater::show` does for every window it draws
                    let id = egui::Id::unique("search_win");
                    note_window_drawn(ctx, id);
                    egui::Window::new("Search window").id(id).show(ctx, |ui| {
                        if field {
                            search_text_edit(ui, &p, text, |edit| edit.id(egui::Id::unique("window_search")).desired_width(200.0));
                        } else {
                            ui.label("Another tab");
                        }
                    });
                }
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn a_window_that_reopens_on_its_kept_search_takes_the_keyboard_with_it_selected() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let id = egui::Id::unique("window_search");
        let mut text = String::new();
        for _ in 0..3 {
            window_field(&ctx, &mut text, true, true, Vec::new());
        }
        assert!(!ctx.memory(|m| m.has_focus(id)), "an empty search does not take the keyboard");
        text = "kept search".to_owned();
        // The window is closed for a while, then opens again.
        for _ in 0..3 {
            window_field(&ctx, &mut text, false, false, Vec::new());
        }
        for _ in 0..4 {
            window_field(&ctx, &mut text, true, true, Vec::new());
        }
        assert!(ctx.memory(|m| m.has_focus(id)), "the reopened window takes the keyboard");
        window_field(&ctx, &mut text, true, true, vec![egui::Event::Text("n".into())]);
        assert_eq!(text, "n", "the first character replaces the kept search");
    }

    #[test]
    fn a_tab_change_inside_an_open_window_does_not_take_the_keyboard() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let id = egui::Id::unique("window_search");
        let mut text = "kept search".to_owned();
        // (past the moments after the opening, when its search may still come)
        for _ in 0..(OPENING_FRAMES + 10) {
            window_field(&ctx, &mut text, true, false, Vec::new());
        }
        for _ in 0..4 {
            window_field(&ctx, &mut text, true, true, Vec::new());
        }
        // The window was open all along: only its field came back.
        assert!(!ctx.memory(|m| m.has_focus(id)));
    }

    #[test]
    fn an_empty_search_that_takes_the_keyboard_has_nothing_to_select() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut text = String::new();
        lone_field(&ctx, &mut text, Vec::new(), false);
        lone_field(&ctx, &mut text, Vec::new(), true);
        lone_field(&ctx, &mut text, vec![egui::Event::Text("n".into())], false);
        assert_eq!(text, "n");
    }

    fn frame(ctx: &egui::Context, kind: usize, text: &mut String, events: Vec<egui::Event>, enabled: bool) -> egui::Response {
        let p = crate::theme::Theme::default().palette();
        let mut field = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 150.0))),
                events,
                ..Default::default()
            },
            |ui| {
                field = Some(
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.horizontal(|ui| match kind {
                            0 => super::super::search_field(ui, &p, text, "Search", 240.0),
                            1 => search_text_edit(ui, &p, text, |edit| {
                                edit.id(egui::Id::unique("custom_search")).char_limit(128).desired_width(240.0)
                            }),
                            2 => {
                                let font = egui::FontId::proportional(12.0);
                                let margin = super::super::field_text_margin(ui.ctx(), &font);
                                search_text_edit_with_margin(ui, &p, text, margin, |edit| {
                                    edit.font(font).frame(egui::Frame::NONE).desired_width(240.0)
                                })
                            }
                            _ => clearable_search_box(ui, &p, text, "Search", 240.0),
                        })
                        .inner
                    })
                    .inner,
                );
            },
        );
        output.textures_delta.clear();
        field.expect("the search field was drawn")
    }

    fn click(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn search_clear_preserves_id_focus_layout_and_changed_notifications() {
        for kind in 0..4 {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut text = "A long query with spaces and 日本語".repeat(4);
            let mut field = frame(&ctx, kind, &mut text, vec![], true);
            for _ in 0..3 {
                field = frame(&ctx, kind, &mut text, vec![], true);
            }
            let id = field.id;
            let rect = field.rect;
            assert!(rect.width() < 270.0 && rect.height() < 40.0, "kind={kind}: {rect:?}");
            let clear = ctx.read_response(id.with("clear")).expect("a filled field has a clear action");
            assert!(rect.contains_rect(clear.rect));
            frame(&ctx, kind, &mut text, click(clear.rect.center(), true), true);
            field = frame(&ctx, kind, &mut text, click(clear.rect.center(), false), true);
            assert!(text.is_empty(), "kind={kind}");
            assert!(field.changed(), "the caller must refresh its filtered results");
            assert_eq!(field.id, id, "clearing must keep custom editor ids");
            assert!(field.has_focus(), "the next keystroke returns to the search");
            field = frame(&ctx, kind, &mut text, vec![egui::Event::Text("next".into())], true);
            assert_eq!(text, "next");
            assert!(field.changed());
            let modifiers = egui::Modifiers {
                ctrl: true,
                command: true,
                ..Default::default()
            };
            field = frame(
                &ctx,
                kind,
                &mut text,
                vec![
                    egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                true,
            );
            assert!(text.is_empty());
            assert!(field.changed());
            for _ in 0..3 {
                field = frame(&ctx, kind, &mut text, vec![], true);
            }
            assert!(ctx.read_response(id.with("clear")).is_none(), "empty searches have no clear action");
            assert_eq!(field.rect.size(), rect.size(), "the field must not resize when cleared");
        }
    }

    #[test]
    fn disabled_search_clear_cannot_change_the_query() {
        for kind in 0..4 {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut text = "unchanged".into();
            let mut field = frame(&ctx, kind, &mut text, vec![], false);
            for _ in 0..3 {
                field = frame(&ctx, kind, &mut text, vec![], false);
            }
            let clear = ctx.read_response(field.id.with("clear")).expect("clear action");
            assert!(!clear.enabled());
            frame(&ctx, kind, &mut text, click(clear.rect.center(), true), false);
            field = frame(&ctx, kind, &mut text, click(clear.rect.center(), false), false);
            assert_eq!(text, "unchanged");
            assert!(!field.changed());
        }
    }
}
