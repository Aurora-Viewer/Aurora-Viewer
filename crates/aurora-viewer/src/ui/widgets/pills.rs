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

/// A `pill_tab` of its own width, without icon or count, that says `why`
/// on hover while it is greyed (a preview tab behind a waiting question or
/// a viewing restriction).
pub fn pill_tab_why(ui: &mut egui::Ui, p: &Palette, label: &str, selected: bool, enabled: bool, why: &str) -> egui::Response {
    tab(ui, p, "", label, None, false, selected, enabled, None, Some(why))
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
    let height = galley.size().y.max(count.as_ref().map_or(0.0, |g| g.size().y)) + 8.0;
    let size = Vec2::new(width.unwrap_or(12.0 + content + 12.0), height.max(28.0));
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

/// A row of chips that may be wider than its window (the folders of Places,
/// the categories of Destinations). It has no scroll bar, which would take a
/// line of a narrow window, so the row has to move by other means (#3872: with
/// a mouse there was none, the folders past the edge could not be reached):
/// - the wheel scrolls it without Shift, the row moving one way only;
/// - the chosen chip, which `add` returns with a key of its own, is brought
///   to the middle of the row when the choice changes: a click on a chip at
///   the edge shows the next ones, and a choice made elsewhere is in view.
pub fn chip_row(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui) -> Option<(egui::Id, Rect)>) {
    let either_axis = std::mem::replace(&mut ui.style_mut().always_scroll_the_only_direction, true);
    let mut area = egui::ScrollArea::horizontal()
        .id_salt(salt)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    // the place worked out at the end of the last frame, once the row was laid out
    let wanted = ui.scope_id().with((salt, "wanted"));
    if let Some(offset) = ui.ctx().data_mut(|d| d.remove_temp::<f32>(wanted)) {
        area = area.horizontal_scroll_offset(offset);
    }
    let out = area.show(ui, |ui| ui.horizontal(add).inner);
    ui.style_mut().always_scroll_the_only_direction = either_axis;
    let shown = out.id.with("chosen");
    let chosen = out.inner.map(|(key, _)| key);
    if ui.ctx().data(|d| d.get_temp::<Option<egui::Id>>(shown)) != Some(chosen) {
        ui.ctx().data_mut(|d| d.insert_temp(shown, chosen));
        if let Some((_, chip)) = out.inner {
            let view = out.inner_rect;
            let most = (out.content_size.x - view.width()).max(0.0);
            // the chip's rectangle is on screen, so already moved by the offset
            let now = out.state.clamped_offset().x;
            let offset = (now + chip.center().x - view.center().x).clamp(0.0, most);
            if (offset - now).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(wanted, offset));
                ui.ctx().request_repaint();
            }
        }
    }
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
    icon_field_inner(ui, p, "search-filled", text, hint, width, Field::Search)
}

/// `search_box` with an in-field clear button while filled.
pub fn clearable_search_box(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    icon_field_inner(ui, p, "search-filled", text, hint, width, Field::ClearableSearch)
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

/// A search or filter field that takes the keyboard selects its whole
/// text, so the next character replaces the old search instead of
/// continuing it: when a window hands it the keyboard (the Inventory,
/// Preferences and Outfits on opening, Ctrl+F, Tab), and on the first click
/// into it, whose drag selects nothing else. A second click, into the field
/// that holds the keyboard, places the caret, and its drag selects.
/// Port of LLLineEditor's `mSelectAllonFocusReceived`
/// (indra/llui/lllineeditor.cpp: `setFocus` selects all, `handleMouseDown`
/// skips the caret placement and the mouse capture while the field does not
/// hold the keyboard),
/// which the Legacy Viewer's search and filter fields set
/// (`select_on_focus` in search_editor.xml and filter_editor.xml).
pub fn select_on_focus(ui: &egui::Ui, response: &egui::Response, text: &str) {
    // egui's own `gained_focus` is not reported for a focus asked for after
    // the field was drawn, which is how the windows ask on opening: the field
    // remembers instead whether it held the keyboard in the frame before. A
    // field not drawn for a frame (its window closed or hidden) forgets it.
    // Enter lets a single-line field go for a frame, and the find bars take
    // it back: that is the same hold, so the caret they kept stays.
    // (a window's invisible sizing pass draws nothing the user sees)
    if ui.is_sizing_pass() {
        return;
    }
    let ctx = ui.ctx();
    let key = response.id.with("select_on_focus");
    // The viewer's own window's frame, also in a window drawn outside it,
    // whose pass runs inside the viewer's and shares its numbers (outside.rs).
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    let last = ctx.data(|d| d.get_temp::<Hold>(key)).filter(|last| last.frame + 1 >= frame);
    // The hold is egui's keyboard focus, as LLLineEditor::hasFocus is the
    // keyboard focus alone (`Response::has_focus` also asks for the
    // system's): another program in front and gone again (Alt+Tab, a click
    // on the title bar) leaves the hold and the caret, as
    // LLFocusMgr::setAppHasFocus leaves the keyboard focus
    // (indra/llui/llfocusmgr.cpp). Another window of the viewer that takes
    // the system's focus (the viewer's own, for a field in a window outside)
    // ends the hold until the field's window has the focus back, as a click
    // into another floater takes the Legacy Viewer's keyboard focus: egui
    // keeps each window's focus apart, and the field selects again when its
    // window comes back.
    let away = match crate::outside::focused_window(ctx) {
        Some(window) => window != ctx.viewport_id(),
        None => last.is_some_and(|last| last.away),
    };
    let focused = !away && ui.memory(|m| m.has_focus(response.id));
    let held = focused || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    let had = last.is_some_and(|last| last.held);
    let gained = focused && !had;
    ctx.data_mut(|d| d.insert_temp(key, Hold { frame, held, away }));
    if !gained {
        return;
    }
    // The press that gave the field the keyboard: egui put the caret at the
    // pointer, and its drag would move the selection's end with the pointer
    // where the Legacy Viewer leaves the whole text selected
    // (`LLLineEditor::setFocus` ends `mIsSelecting`). egui's drag of that
    // press is let go, so nothing moves the selection until the release.
    // Only the field's own: under a press of this pass on the field, nothing
    // else is dragged yet, and a field that takes no drag (egui's on a touch
    // screen, before it has the keyboard) leaves the press to what scrolls
    // under it. Once past egui's click distance, the rest of the press is
    // nobody's, as `handleMouseDown` takes no mouse capture for it: what is
    // under the pointer shows its hover, and nothing takes a click, a drag
    // or a drop of it.
    if response.sense.senses_drag()
        && response.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.any_pressed())
        && ctx.dragged_id().is_none()
    {
        ctx.stop_dragging();
    }
    if text.is_empty() {
        return;
    }
    let mut state = egui::TextEdit::load_state(ctx, response.id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(text)));
    state.store(ctx, response.id);
    ctx.request_repaint();
}

/// What `select_on_focus` keeps of a field's last frame.
#[derive(Clone, Copy)]
struct Hold {
    frame: u64,
    /// It held the keyboard (or let it go to Enter, for a find bar).
    held: bool,
    /// Another window of the viewer had the system's focus.
    away: bool,
}

fn note_search_clear(ui: &egui::Ui, response: &egui::Response, emptied: bool) {
    if emptied && response.changed() {
        let key = egui::Id::unique("search_selection_reveal").with(ui.layer_id());
        let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
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
        .is_some_and(|frame| ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT).saturating_sub(frame) <= 1)
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

/// A text field on that face, a Tabler `icon` before the text. Not a
/// search: it keeps the caret where a click puts it (the place menu's
/// destination, as the Legacy Viewer's location_input.xml sets
/// `select_on_focus="false"`).
pub fn icon_field(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &mut String, hint: &str, width: f32) -> egui::Response {
    icon_field_inner(ui, p, icon, text, hint, width, Field::Plain)
}

/// What an `icon_field_inner` is.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Plain,
    /// It selects its text when it takes the keyboard (`select_on_focus`).
    Search,
    /// A search with the clear action while filled.
    ClearableSearch,
}

fn icon_field_inner(ui: &mut egui::Ui, p: &Palette, icon: &str, text: &mut String, hint: &str, width: f32, kind: Field) -> egui::Response {
    let clear = kind == Field::ClearableSearch;
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
                if kind != Field::Plain {
                    select_on_focus(ui, &response, text);
                }
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
    // Not a Tab stop: LLSearchEditor builds its clear button with
    // `tab_stop(false)` (indra/llui/llsearcheditor.cpp), so Tab leaves the
    // field for the next control. `Sense::click()` would make it focusable.
    let clear = ui.interact(rect, response.id.with("clear"), Sense::CLICK);
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
mod chip_row_tests {
    use super::*;

    /// One frame of a 200-point window holding 20 chips; returns the chips'
    /// rectangles.
    fn frame(ctx: &egui::Context, chosen: &mut usize, events: Vec<egui::Event>) -> Vec<Rect> {
        let p = crate::theme::Theme::default().palette();
        let mut rects = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(200.0, 120.0))),
                events,
                ..Default::default()
            },
            |ui| {
                chip_row(ui, "chips", |ui| {
                    let mut shown = None;
                    for i in 0..20 {
                        let on = *chosen == i;
                        let response = filter_pill(ui, &p, "", &format!("Folder {i:02}"), on);
                        rects.push(response.rect);
                        if on {
                            shown = Some((egui::Id::unique(i), response.rect));
                        }
                        if response.clicked() {
                            *chosen = i;
                        }
                    }
                    shown
                });
            },
        );
        // no renderer takes the font atlas here
        output.textures_delta.clear();
        rects
    }

    fn wheel(delta: Vec2) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta,
            modifiers: Default::default(),
        }
    }

    /// #3872: a mouse wheel turns up and down; the row of folders has no bar
    /// and did not move with it, so the folders past the edge stayed there.
    #[test]
    fn the_wheel_scrolls_a_row_of_chips_without_shift() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut chosen = 0;
        let start = frame(&ctx, &mut chosen, Vec::new());
        assert!(start[19].left() > 200.0, "the last chips are past the edge: {:?}", start[19]);
        let over = start[1].center();
        frame(&ctx, &mut chosen, vec![egui::Event::PointerMoved(over)]);
        let mut rects = Vec::new();
        for _ in 0..40 {
            frame(&ctx, &mut chosen, vec![wheel(egui::vec2(0.0, -120.0))]);
        }
        for _ in 0..30 {
            rects = frame(&ctx, &mut chosen, Vec::new());
        }
        assert!(rects[19].right() <= 200.5, "the wheel reaches the last chip: {:?}", rects[19]);
        assert_eq!(chosen, 0, "scrolling chooses nothing");
        // and back
        for _ in 0..40 {
            frame(&ctx, &mut chosen, vec![wheel(egui::vec2(0.0, 120.0))]);
        }
        for _ in 0..30 {
            rects = frame(&ctx, &mut chosen, Vec::new());
        }
        assert!((rects[0].left() - start[0].left()).abs() < 0.5, "back to the start: {:?}", rects[0]);
    }

    #[test]
    fn the_chosen_chip_comes_to_the_middle_once() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        // a choice made elsewhere, out of view when the row opens
        let mut chosen = 12;
        frame(&ctx, &mut chosen, Vec::new());
        let rects = frame(&ctx, &mut chosen, Vec::new());
        assert!(
            (rects[12].center().x - 100.0).abs() < 1.0,
            "the chosen chip is in the middle: {:?}",
            rects[12]
        );

        // a click on the chip cut by the edge shows the next ones
        let edge = rects.iter().position(|r| r.right() > 200.0).expect("a chip at the edge");
        let pos = egui::pos2(rects[edge].left() + 4.0, rects[edge].center().y);
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut chosen, vec![egui::Event::PointerMoved(pos), press(true)]);
        frame(&ctx, &mut chosen, vec![press(false)]);
        assert_eq!(chosen, edge);
        frame(&ctx, &mut chosen, vec![egui::Event::PointerGone]);
        let rects = frame(&ctx, &mut chosen, Vec::new());
        assert!(
            (rects[edge].center().x - 100.0).abs() < 1.0,
            "the clicked chip is in the middle: {:?}",
            rects[edge]
        );

        // then the row is the user's: the wheel is not undone
        frame(
            &ctx,
            &mut chosen,
            vec![egui::Event::PointerMoved(egui::pos2(100.0, rects[edge].center().y))],
        );
        for _ in 0..10 {
            frame(&ctx, &mut chosen, vec![wheel(egui::vec2(0.0, 120.0))]);
        }
        let mut after = Vec::new();
        for _ in 0..30 {
            after = frame(&ctx, &mut chosen, Vec::new());
        }
        assert!(
            after[edge].center().x > 150.0,
            "the row stays where the wheel left it: {:?}",
            after[edge]
        );
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

    /// A lone search field drawn frame after frame, a sixtieth of a second
    /// apart: egui takes two clicks closer than 0.3 s for a double click.
    struct Lone {
        ctx: egui::Context,
        time: f64,
        /// The viewer's window has the system's focus.
        in_front: bool,
        /// What the last frame painted.
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    impl Lone {
        fn id() -> egui::Id {
            egui::Id::unique("lone_search")
        }

        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            Self {
                ctx,
                time: 0.0,
                in_front: true,
                shapes: Vec::new(),
            }
        }

        /// One frame; `after` runs after the field, with its response, as
        /// the window that holds it does.
        fn frame(&mut self, text: &mut String, events: Vec<egui::Event>, after: impl Fn(&mut egui::Ui, &egui::Response)) -> egui::Response {
            let p = crate::theme::Theme::default().palette();
            self.time += 1.0 / 60.0;
            let mut field = None;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0))),
                    events,
                    time: Some(self.time),
                    focused: self.in_front,
                    ..Default::default()
                },
                |ui| {
                    let response = search_text_edit(ui, &p, text, |edit| edit.id(Self::id()).desired_width(240.0));
                    after(ui, &response);
                    field = Some(response);
                },
            );
            output.textures_delta.clear();
            self.shapes = output.shapes;
            field.expect("the field is drawn every frame")
        }

        /// The part of `text` the last frame painted selected.
        fn painted(&self, text: &str) -> Painted {
            painted_selection(&self.ctx, &self.shapes, text)
        }

        fn idle(&mut self, text: &mut String) -> egui::Response {
            self.frame(text, Vec::new(), |_, _| {})
        }

        /// The field's selection, as the next frame starts from it.
        fn range(&self) -> Option<egui::text::CCursorRange> {
            egui::TextEdit::load_state(&self.ctx, Self::id()).and_then(|s| s.cursor.char_range())
        }

        fn focused(&self) -> bool {
            self.ctx.memory(|m| m.has_focus(Self::id()))
        }

        /// Puts the field's caret after its `at`th character, as the user's
        /// arrows or a click would.
        fn place_caret(&self, at: usize) -> egui::text::CCursorRange {
            let caret = egui::text::CCursorRange::one(egui::text::CCursor::new(at));
            let mut state = egui::TextEdit::load_state(&self.ctx, Self::id()).expect("the field's state");
            state.cursor.set_char_range(Some(caret));
            state.store(&self.ctx, Self::id());
            caret
        }
    }

    /// What a frame painted of a field's text: the span of its selection, in
    /// points from the text's start (None: no selection), the text's width,
    /// and where the text starts on the screen.
    #[derive(Debug)]
    struct Painted {
        selected: Option<(f32, f32)>,
        width: f32,
        at: egui::Pos2,
    }

    impl Painted {
        fn whole(&self) -> bool {
            self.selected.is_some_and(|(from, to)| from <= 0.5 && to >= self.width - 0.5)
        }
    }

    fn painted_selection(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape], text: &str) -> Painted {
        let color = ctx.global_style().visuals.selection.bg_fill;
        let shape = shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(shape) if shape.galley.job.text == text => Some(shape),
                _ => None,
            })
            .expect("the field's text is painted");
        let row = &shape.galley.rows[0];
        // (TextEdit paints the selection as a flat quad of that colour in the row)
        let selected = row
            .row
            .visuals
            .mesh
            .vertices
            .iter()
            .filter(|v| v.color == color && v.uv == egui::epaint::WHITE_UV)
            .fold(None, |span: Option<(f32, f32)>, v| {
                Some(span.map_or((v.pos.x, v.pos.x), |(from, to)| (from.min(v.pos.x), to.max(v.pos.x))))
            });
        Painted {
            selected,
            width: row.row.size.x,
            at: shape.pos + row.pos.to_vec2(),
        }
    }

    #[test]
    fn a_search_that_takes_the_keyboard_selects_its_text_so_typing_replaces_it() {
        // also while another program is in front (a demo's window launched
        // behind it): the keyboard is egui's, the next key typed in the viewer
        // goes there
        for in_front in [true, false] {
            let mut f = Lone::new();
            f.in_front = in_front;
            let mut text = "old search".to_owned();
            f.idle(&mut text);
            // The window asks for the keyboard after drawing it, like the Inventory.
            f.frame(&mut text, Vec::new(), |_, field| field.request_focus());
            f.idle(&mut text);
            let all = Some(egui::text::CCursorRange::select_all_str("old search"));
            assert_eq!(f.range(), all, "the whole old search is selected (in front: {in_front})");
            f.in_front = true;
            f.frame(&mut text, vec![egui::Event::Text("n".into())], |_, _| {});
            assert_eq!(text, "n", "the first character replaces the selection (in front: {in_front})");
        }
    }

    #[test]
    fn a_first_click_selects_the_whole_search_and_a_second_places_the_caret() {
        // LLLineEditor::handleMouseDown with select_on_focus (indra/llui/lllineeditor.cpp)
        let mut f = Lone::new();
        let mut text = "old search".to_owned();
        let all = Some(egui::text::CCursorRange::select_all_str("old search"));
        let field = f.idle(&mut text);
        let start = field.rect.left_center() + egui::vec2(12.0, 0.0);
        // the middle of the text: a drag selection from its start would stop there
        let painted = f.painted("old search");
        let middle = egui::pos2(painted.at.x + painted.width / 2.0, field.rect.center().y);
        // pressed near the start of the text, dragged to its middle and on,
        // released there (the press's own frame still shows the caret egui
        // put at the pointer: the field paints before select_on_focus runs)
        f.frame(&mut text, click(start, true), |_, _| {});
        assert!(f.focused(), "the click gives the field the keyboard");
        assert_eq!(f.range(), all, "the press selects the whole search");
        for step in 1..=4 {
            let at = start + (middle - start) * (step as f32 / 4.0);
            f.frame(&mut text, vec![egui::Event::PointerMoved(at)], |_, _| {});
            let shown = f.painted("old search");
            assert!(shown.whole(), "its drag shows the whole search selected: {shown:?}");
            assert_eq!(f.range(), all, "and keeps it");
            assert!(!f.ctx.is_being_dragged(Lone::id()), "that press drags no selection");
        }
        f.frame(&mut text, click(middle, false), |_, _| {});
        assert!(f.painted("old search").whole(), "so does its release");
        f.idle(&mut text);
        assert_eq!(f.range(), all);
        assert!(f.painted("old search").whole());
        // A second click, past a double click's delay, into the field that
        // holds the keyboard: the caret goes where it lands.
        f.time += 1.0;
        f.frame(&mut text, click(start, true), |_, _| {});
        f.frame(&mut text, click(start, false), |_, _| {});
        f.idle(&mut text);
        let caret = f.range().expect("the field has a caret");
        assert!(caret.is_empty(), "the second click selects nothing: {caret:?}");
        f.frame(&mut text, vec![egui::Event::Text("x".into())], |_, _| {});
        assert_eq!(text.chars().count(), 11, "typing goes in at the caret: {text}");
        assert!(!text.ends_with('x'), "the caret is where the click landed, not at the end: {text}");
    }

    #[test]
    fn a_drag_in_a_search_that_holds_the_keyboard_selects_as_it_goes() {
        // egui's drag selection, as LLLineEditor::handleHover's once the
        // field has the keyboard
        let mut f = Lone::new();
        let mut text = "old search".to_owned();
        let field = f.idle(&mut text);
        let start = field.rect.left_center() + egui::vec2(12.0, 0.0);
        let painted = f.painted("old search");
        let middle = egui::pos2(painted.at.x + painted.width / 2.0, field.rect.center().y);
        f.frame(&mut text, click(start, true), |_, _| {});
        f.frame(&mut text, click(start, false), |_, _| {});
        f.idle(&mut text);
        assert!(f.focused());
        f.time += 1.0;
        f.frame(&mut text, click(start, true), |_, _| {});
        for step in 1..=4 {
            let at = start + (middle - start) * (step as f32 / 4.0);
            f.frame(&mut text, vec![egui::Event::PointerMoved(at)], |_, _| {});
        }
        let shown = f.painted("old search");
        assert!(
            shown.selected.is_some() && !shown.whole(),
            "the drag shows part of the search: {shown:?}"
        );
        f.frame(&mut text, click(middle, false), |_, _| {});
        f.idle(&mut text);
        let range = f.range().expect("the field has a selection");
        assert!(!range.is_empty(), "the drag selected something");
        assert_ne!(
            Some(range),
            Some(egui::text::CCursorRange::select_all_str("old search")),
            "but not all of it"
        );
    }

    #[test]
    fn coming_back_from_another_program_keeps_the_caret_of_a_search() {
        // LLLineEditor::hasFocus is the keyboard focus alone, which
        // LLFocusMgr::setAppHasFocus leaves as it is (indra/llui/llfocusmgr.cpp)
        let mut f = Lone::new();
        let mut text = "old search".to_owned();
        f.idle(&mut text);
        f.frame(&mut text, Vec::new(), |_, field| field.request_focus());
        f.idle(&mut text);
        let caret = f.place_caret(3);
        f.idle(&mut text);
        // Alt+Tab to another program: the viewer goes on drawing behind it
        f.in_front = false;
        for _ in 0..3 {
            f.idle(&mut text);
        }
        // and back, by Alt+Tab or a click on the title bar
        f.in_front = true;
        f.idle(&mut text);
        f.idle(&mut text);
        assert!(f.focused(), "the search kept the keyboard");
        let range = f.range().expect("the field has a caret");
        assert_eq!(range.as_sorted_char_range(), caret.as_sorted_char_range(), "the caret stays");
        f.frame(&mut text, vec![egui::Event::Text("-".into())], |_, _| {});
        assert_eq!(text, "old- search", "the next character goes in at the caret");
    }

    /// Which window has the system's focus.
    #[derive(Clone, Copy, PartialEq)]
    enum Front {
        /// The window outside that holds the search.
        Outside,
        /// The viewer's own window.
        Viewer,
        /// Another program.
        Other,
    }

    /// A search in a window outside the viewer's (outside.rs): its pass runs
    /// inside each of the viewer's, with its own input and system focus.
    struct Apart {
        ctx: egui::Context,
        time: f64,
        /// What the window outside reads at its next pass, and its focus.
        input: std::rc::Rc<std::cell::RefCell<(Vec<egui::Event>, bool)>>,
    }

    impl Apart {
        fn window() -> egui::Id {
            egui::Id::unique("apart_window")
        }

        fn id() -> egui::Id {
            egui::Id::unique("apart_search")
        }

        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let input: std::rc::Rc<std::cell::RefCell<(Vec<egui::Event>, bool)>> = std::rc::Rc::default();
            let for_pass = input.clone();
            // (no native window in a test: the pass is run as outside.rs runs it)
            ctx.set_embed_viewports(false);
            egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| {
                let egui::ImmediateViewport {
                    ids, mut viewport_ui_cb, ..
                } = viewport;
                let (events, focused) = {
                    let mut input = for_pass.borrow_mut();
                    (std::mem::take(&mut input.0), input.1)
                };
                let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0));
                let info = egui::ViewportInfo {
                    parent: Some(ids.parent),
                    native_pixels_per_point: Some(1.0),
                    inner_rect: Some(screen),
                    focused: Some(focused),
                    ..Default::default()
                };
                let raw = egui::RawInput {
                    viewport_id: ids.this,
                    viewports: std::iter::once((ids.this, info)).collect(),
                    screen_rect: Some(screen),
                    time: Some(ctx.input(|i| i.time)),
                    focused,
                    events,
                    ..Default::default()
                };
                let mut output = ctx.run_ui(raw, |ui| viewport_ui_cb(ui));
                output.textures_delta.clear();
            });
            Self { ctx, time: 0.0, input }
        }

        /// One frame of the viewer, with `front` in front and `events` for
        /// the window outside; whether the search held egui's keyboard.
        fn frame(&mut self, text: &mut String, front: Front, events: Vec<egui::Event>) -> (egui::Response, bool) {
            let p = crate::theme::Theme::default().palette();
            self.time += 1.0 / 60.0;
            crate::outside::focus_for_test(&self.ctx, (front == Front::Outside).then(Self::window));
            *self.input.borrow_mut() = (events, front == Front::Outside);
            let mut field = None;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    time: Some(self.time),
                    focused: front == Front::Viewer,
                    ..Default::default()
                },
                |ui| {
                    let outside = crate::outside::viewport(Self::window());
                    ui.ctx()
                        .show_viewport_immediate(outside, egui::ViewportBuilder::default(), |ui, _| {
                            let response = search_text_edit(ui, &p, text, |edit| edit.id(Self::id()).desired_width(240.0));
                            let held = ui.memory(|m| m.has_focus(Self::id()));
                            field = Some((response, held));
                        });
                },
            );
            output.textures_delta.clear();
            field.expect("the search is drawn in its window outside")
        }

        fn range(&self) -> Option<egui::text::CCursorRange> {
            egui::TextEdit::load_state(&self.ctx, Self::id()).and_then(|s| s.cursor.char_range())
        }

        fn click_at(&mut self, text: &mut String, front: Front, at: egui::Pos2) {
            self.time += 1.0;
            self.frame(text, front, click(at, true));
            self.frame(text, front, click(at, false));
            self.frame(text, front, Vec::new());
        }
    }

    #[test]
    fn a_search_outside_selects_again_once_another_window_of_the_viewer_had_the_focus() {
        let mut f = Apart::new();
        let mut text = "old search".to_owned();
        let all = Some(egui::text::CCursorRange::select_all_str("old search"));
        let (field, _) = f.frame(&mut text, Front::Outside, Vec::new());
        let start = field.rect.left_center() + egui::vec2(12.0, 0.0);
        f.click_at(&mut text, Front::Outside, start);
        assert_eq!(f.range(), all, "the first click selects the whole search");
        f.click_at(&mut text, Front::Outside, start);
        let caret = f.range().expect("the search has a caret");
        assert!(caret.is_empty(), "a second click places the caret: {caret:?}");
        // another program comes in front and goes: the caret stays
        for _ in 0..3 {
            f.frame(&mut text, Front::Other, Vec::new());
        }
        let (_, held) = f.frame(&mut text, Front::Outside, Vec::new());
        f.frame(&mut text, Front::Outside, Vec::new());
        assert!(held, "the search kept the keyboard");
        assert_eq!(f.range(), Some(caret), "coming back from another program keeps the caret");
        // a click in the viewer's own window takes the system's focus; the
        // search outside keeps egui's keyboard, which nothing there takes
        for _ in 0..3 {
            let (_, held) = f.frame(&mut text, Front::Viewer, Vec::new());
            assert!(held);
        }
        // as a click into another floater moves the Legacy Viewer's keyboard
        // focus (LLFloater::setFocus gives it back to the field on return)
        f.click_at(&mut text, Front::Outside, start);
        assert_eq!(f.range(), all, "the next click into the search selects it again");
    }

    #[test]
    fn enter_in_a_find_bar_keeps_its_caret() {
        // The script editor's find bar (scripts/dialogs.rs): Enter finds the
        // next match and takes the keyboard back.
        let find_bar = |ui: &mut egui::Ui, field: &egui::Response| {
            if (field.has_focus() || field.lost_focus()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                field.request_focus();
            }
        };
        let mut f = Lone::new();
        let mut text = "old search".to_owned();
        f.idle(&mut text);
        f.frame(&mut text, Vec::new(), |_, field| field.request_focus());
        f.idle(&mut text);
        // the caret put after "old"
        let caret = egui::text::CCursorRange::one(egui::text::CCursor::new(3));
        let mut state = egui::TextEdit::load_state(&f.ctx, Lone::id()).expect("the field's state");
        state.cursor.set_char_range(Some(caret));
        state.store(&f.ctx, Lone::id());
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        f.frame(&mut text, vec![enter], find_bar);
        f.frame(&mut text, Vec::new(), find_bar);
        f.frame(&mut text, Vec::new(), find_bar);
        assert!(f.focused(), "the find bar keeps the keyboard");
        let range = f.range().expect("the field has a caret");
        assert_eq!(range.as_sorted_char_range(), caret.as_sorted_char_range(), "the caret stays");
        f.frame(&mut text, vec![egui::Event::Text("-".into())], find_bar);
        assert_eq!(text, "old- search");
    }

    /// One frame of a window holding a kept search and not asking for the
    /// keyboard, as Debug Settings, Experiences or a group's members.
    fn kept_search_window(ctx: &egui::Context, text: &mut String, mut open: bool, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::Floater::new(
                    "kept_search_window",
                    "Kept search",
                    egui::pos2(20.0, 20.0),
                    egui::vec2(320.0, 200.0),
                )
                .show(ui.ctx(), &p, &mut open, |ui| {
                    search_text_edit(ui, &p, text, |edit| edit.id(egui::Id::unique("kept_search")).desired_width(200.0));
                });
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn a_window_that_does_not_ask_leaves_the_keyboard_alone_when_it_opens_or_shows_again() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let id = egui::Id::unique("kept_search");
        let mut text = "kept search".to_owned();
        for _ in 0..3 {
            kept_search_window(&ctx, &mut text, true, Vec::new());
        }
        assert!(!ctx.memory(|m| m.has_focus(id)), "opening on a kept search");
        for _ in 0..3 {
            kept_search_window(&ctx, &mut text, false, Vec::new());
        }
        for _ in 0..3 {
            kept_search_window(&ctx, &mut text, true, Vec::new());
        }
        assert!(!ctx.memory(|m| m.has_focus(id)), "reopening on a kept search");
        // mouselook, or Alt+Shift+U twice
        super::super::hide_windows(&ctx, true, super::super::HideKeep::None);
        for _ in 0..3 {
            kept_search_window(&ctx, &mut text, true, Vec::new());
        }
        super::super::hide_windows(&ctx, false, super::super::HideKeep::None);
        for _ in 0..3 {
            kept_search_window(&ctx, &mut text, true, Vec::new());
        }
        assert!(!ctx.memory(|m| m.has_focus(id)), "shown again with the interface");
        kept_search_window(&ctx, &mut text, true, vec![egui::Event::Text("w".into())]);
        assert_eq!(text, "kept search", "a movement key does not type into the search");
    }

    #[test]
    fn an_empty_search_that_takes_the_keyboard_has_nothing_to_select() {
        let mut f = Lone::new();
        let mut text = String::new();
        f.idle(&mut text);
        f.frame(&mut text, Vec::new(), |_, field| field.request_focus());
        f.frame(&mut text, vec![egui::Event::Text("n".into())], |_, _| {});
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
    fn tab_skips_the_search_clear_action() {
        let tab = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for kind in 0..4 {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut text = "query".into();
            let mut field = frame(&ctx, kind, &mut text, vec![], true);
            for _ in 0..3 {
                field = frame(&ctx, kind, &mut text, vec![], true);
            }
            let clear = field.id.with("clear");
            assert!(ctx.read_response(clear).is_some(), "kind={kind}: a filled field has a clear action");
            ctx.memory_mut(|m| m.request_focus(field.id));
            frame(&ctx, kind, &mut text, vec![], true);
            assert_eq!(
                ctx.memory(|m| m.focused()),
                Some(field.id),
                "kind={kind}: the field holds the focus before Tab"
            );
            frame(&ctx, kind, &mut text, vec![tab.clone()], true);
            for _ in 0..2 {
                frame(&ctx, kind, &mut text, vec![], true);
            }
            assert_ne!(
                ctx.memory(|m| m.focused()),
                Some(clear),
                "kind={kind}: Tab must not stop on the cross"
            );
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
