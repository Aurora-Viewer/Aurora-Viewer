//! Firestorm-style building blocks: floaters with a slim title bar
//! (title, ?, —, ✕), flat tabs, column headers and status dots.

use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

#[cfg(test)]
mod check_tests;
#[cfg(test)]
mod collapse_tests;
pub mod color;
pub mod demo;
#[cfg(test)]
pub(crate) mod elide_tests;
pub(crate) mod keycap;
pub(crate) mod pills;
mod resize;
mod unread;
mod workspace;
pub use color::color_button;
pub use pills::{dot_pill, dot_pill_width, filter_pill, filter_pill_at, filter_pill_width, pill_tab, pill_tab_wide, search_box};
pub use unread::{CountSize, UnreadCount, unread_count};
pub use workspace::{Modal, collapsing, collapsing_forced, glyph, navigation, navigation_wrapped, section_header, stat, surface};
pub(crate) use workspace::{
    finish_bar_glass, inventory_item_label, inventory_spacing, island_shape, overlay, overlay_frame, paint_world_label,
};

/// Keep status suffixes at normal weight and slightly darker than the name.
pub(crate) fn name_with_suffix(name: &str, suffix: &str, font_id: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(name, 0.0, egui::TextFormat::simple(font_id.clone(), color));
    job.append(suffix, 0.0, egui::TextFormat::simple(font_id, color.gamma_multiply(0.85)));
    job
}

/// A one-line label at most `max_width` wide: a longer text ends with "…",
/// as the texts Firestorm declares with use_ellipses="true" do
/// (LLFontGL::render), and shows whole on hover (egui's tooltip of an elided
/// label; Firestorm adds it to its truncated tab labels,
/// LLTabContainer::addLabelToolTip). It takes the room of its text only, so
/// what follows in a row stays next to it. For names of residents, groups
/// and conversations, whose length is not ours.
///
/// The galley comes from egui's layout cache: an unchanged text costs a
/// lookup per frame, not a layout.
pub(crate) fn elided_label(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> egui::Label {
    egui::Label::new(elided_galley(ui, text, max_width))
}

/// The same for a row painted by hand: `text` starts at `left_center` and is
/// cut at `width`. Returns true when it was cut, for the caller to show the
/// whole text on hover (LLTabContainer::addLabelToolTip, FIRE-36603: only a
/// truncated label gets the tooltip).
pub(crate) fn paint_elided(ui: &egui::Ui, text: &str, left_center: egui::Pos2, width: f32, font: egui::FontId, color: Color32) -> bool {
    if text.is_empty() {
        return false;
    }
    // less room than the ellipsis itself: nothing readable to paint
    if width < font.size {
        return true;
    }
    let galley = elided_galley(ui, RichText::new(text).font(font).color(color), width);
    let pos = egui::pos2(left_center.x, left_center.y - galley.size().y * 0.5);
    let cut = galley.elided;
    ui.painter().galley(pos, galley, color);
    cut
}

fn elided_galley(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> std::sync::Arc<egui::Galley> {
    text.into()
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), max_width.max(0.0), egui::TextStyle::Body)
}

/// LLFontGL BOLD_OFFSET: egui's strong text changes color, not weight.
pub(crate) fn bold_text_offset(ui: &egui::Ui) -> Vec2 {
    let ppp = ui.ctx().pixels_per_point();
    Vec2::new(ppp.round().max(1.0) / ppp, 0.0)
}

/// What a floater has above and below its body besides its title: the
/// margins of its frame and its outline.
const BODY_CHROME: f32 = 17.0;
/// The least height the scrolled body of a floater keeps.
const MIN_SCROLLED_BODY: f32 = 80.0;

/// `pos` moved so that a window of `size` placed there stands inside `area`
/// (its top left corner first, when it is larger).
fn fit_in(area: egui::Rect, pos: egui::Pos2, size: Vec2) -> egui::Pos2 {
    let max = (area.max - size).max(area.min);
    pos.clamp(area.min, max)
}

/// Width of a minimized floater: its icon, a title cut short and its
/// buttons. Firestorm's is fixed (UIMinimizedWidth, 160) for a smaller
/// title font; a narrower window keeps its own width.
const MINIMIZED_W: f32 = 200.0;

fn minimized_width(expanded: f32) -> f32 {
    expanded.min(MINIMIZED_W)
}

/// Height of a floater's title bar, and the width of it that stays inside
/// the room at least: enough to grab it and bring the window back.
const TITLE_BAR_H: f32 = 38.0;
const TITLE_GRIP: f32 = 96.0;

/// `pos` of a window `width` wide, moved so that its title bar is in
/// `area`: whole in height, and by `TITLE_GRIP` at least in width.
fn keep_title_in(area: egui::Rect, pos: egui::Pos2, width: f32) -> egui::Pos2 {
    let grip = TITLE_GRIP.min(width);
    let left = area.left() - (width - grip);
    let right = (area.right() - grip).max(left);
    let bottom = (area.bottom() - TITLE_BAR_H).max(area.top());
    egui::pos2(pos.x.clamp(left, right), pos.y.clamp(area.top(), bottom))
}

/// The position to give window `id` this frame: `asked` (by the layout, a
/// placement or a resize) or where it stood, with its title bar kept in
/// `area`. None when it stands well and nothing asks for it elsewhere.
fn title_in_reach(ctx: &egui::Context, id: egui::Id, area: egui::Rect, asked: Option<egui::Pos2>) -> Option<egui::Pos2> {
    let state = egui::AreaState::load(ctx, id).filter(|s| s.size.is_some());
    let width = state.as_ref().and_then(|s| s.size).map_or(TITLE_GRIP, |s| s.x);
    let pos = asked.or_else(|| state.as_ref().map(|s| s.left_top_pos()))?;
    let kept = keep_title_in(area, pos, width);
    (asked.is_some() || kept != pos).then_some(kept)
}

pub struct Floater<'a> {
    pub id: &'a str,
    pub title: String,
    pub default_pos: egui::Pos2,
    pub default_size: Vec2,
    pub min_size: Vec2,
    pub resizable: bool,
    pub help: Option<&'a str>,
    /// Extra title bar button: (Tabler icon, tooltip, set when clicked).
    pub action: Option<(&'a str, &'a str, &'a mut bool)>,
    /// No open / close sounds (sound_flags="0", e.g. floater_tools).
    pub silent: bool,
    /// An inset dark title bar and edge-to-edge body for inspector layouts.
    pub framed_header: Option<&'a str>,
    /// Keep an inspector's width stable while its height follows the content.
    pub auto_height: bool,
    /// Put a toolbar directly beneath the title, without a separator layout row.
    pub compact_header: bool,
    /// Something new waits inside (unread messages): the minimized bar shows
    /// it, as Firestorm flashes the conversations button.
    pub attention: bool,
    /// Minimized, the window keeps a compact body under its title bar
    /// instead of the bar alone, at this width: the body is still drawn and
    /// asks `floater_minimized` what to show.
    pub compact_minimized: Option<f32>,
}

impl<'a> Floater<'a> {
    pub fn new(id: &'a str, title: impl Into<String>, pos: egui::Pos2, size: Vec2) -> Floater<'a> {
        Floater {
            id,
            title: title.into(),
            default_pos: pos,
            default_size: size,
            min_size: Vec2::new(220.0, 120.0),
            resizable: true,
            help: None,
            action: None,
            silent: false,
            framed_header: None,
            auto_height: false,
            compact_header: false,
            attention: false,
            compact_minimized: None,
        }
    }

    pub fn fixed(mut self) -> Self {
        self.resizable = false;
        self
    }

    pub fn help(mut self, text: &'a str) -> Self {
        self.help = Some(text);
        self
    }

    /// Add a title bar button left of "Collapse"; `clicked` is set when pressed.
    pub fn action(mut self, icon: &'a str, tip: &'a str, clicked: &'a mut bool) -> Self {
        self.action = Some((icon, tip, clicked));
        self
    }

    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    pub fn framed_header(mut self, icon: &'a str) -> Self {
        self.framed_header = Some(icon);
        self
    }

    pub fn auto_height(mut self) -> Self {
        self.auto_height = true;
        self.resizable = false;
        self
    }

    pub fn compact_header(mut self) -> Self {
        self.compact_header = true;
        self
    }

    /// Minimized, keep a compact body `width` wide (see `compact_minimized`).
    pub fn compact_when_minimized(mut self, width: f32) -> Self {
        self.compact_minimized = Some(width);
        self
    }

    /// Mark the minimized bar while `waiting` (unread messages inside).
    pub fn attention(mut self, waiting: bool) -> Self {
        self.attention = waiting;
        self
    }

    /// Show the floater; `open` is cleared by the close button.
    pub fn show<R>(mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
        if !*open {
            return None;
        }
        let min_id = egui::Id::new((self.id, "minimized"));
        // persisted with the UI layout (ui_layout.ron)
        let minimized = ctx.data_mut(|d| d.get_persisted::<bool>(min_id)).unwrap_or(false);
        // minimized with its compact body: sized by that body
        let compact = minimized && self.compact_minimized.is_some();
        if !self.silent {
            super::sound_cues::floater_shown(ctx, egui::Id::new(self.id));
        }
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(
                p.panel.r(),
                p.panel.g(),
                p.panel.b(),
                p.panel_alpha,
            ))
            .stroke(Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6)))
            .corner_radius(ctx.global_style().visuals.window_corner_radius)
            .shadow(ctx.global_style().visuals.window_shadow)
            .inner_margin(if self.framed_header.is_some() {
                egui::Margin::ZERO
            } else {
                egui::Margin {
                    left: 10,
                    right: 10,
                    top: if minimized && !compact { 4 } else { 5 },
                    bottom: if minimized && !compact { 4 } else { 10 },
                }
            });
        let mut close = false;
        // the toolbar button, a menu entry or a shortcut asked to expand it
        let expand_asked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::new(self.id).with(EXPAND)));
        let mut toggle_min = minimized && expand_asked.unwrap_or(false);
        let mut out = None;
        let window_id = egui::Id::new(self.id);
        let expanded_id = window_id.with("expanded_size");
        let restore_id = window_id.with("restore_size");
        let expanded = ctx.data_mut(|d| d.get_persisted::<Vec2>(expanded_id)).unwrap_or(self.default_size);
        let restore = ctx.data_mut(|d| d.remove_temp::<bool>(restore_id)).unwrap_or(false);
        let resizable = self.resizable && !minimized;
        // The windows live between the bars (`layout::work_area`): none
        // opens larger than that room, nor outside it.
        let work = super::layout::work_area(ctx);
        self.min_size = self.min_size.min(work.size());
        self.default_size = self.default_size.min(work.size());
        self.default_pos = fit_in(work, self.default_pos, self.default_size);
        // A window as tall as its content (fixed, or of automatic height)
        // whose content is taller than the room scrolls it, from the frame
        // after the one that showed it too tall, and for as long as it is.
        let content_sized = !self.resizable && !minimized;
        let scroll_id = window_id.with("body_scrolls");
        let scrolls = content_sized && ctx.data(|d| d.get_temp::<bool>(scroll_id)).unwrap_or(false);
        let mut still_too_tall = false;
        let resize_drag = resizable.then(|| resize::Drag::prepare(ctx, window_id, self.min_size)).flatten();
        // A resizable window left larger than the room (the viewer window
        // was made smaller) is brought down to it, as Firestorm's floaters
        // are (LLFloaterView::adjustToFitScreen).
        let shrink = egui::AreaState::load(ctx, window_id)
            .and_then(|s| s.size)
            .filter(|s| resizable && resize_drag.is_none() && (s.x > work.width() + 0.5 || s.y > work.height() + 0.5))
            .map(|s| s.min(work.size()));
        // Our handles clamp both geometry and position at the minimum, while
        // Window retains its normal movement and persisted layout.
        let mut w = egui::Window::new(&self.title)
            .id(window_id)
            .title_bar(false)
            .fade_in(false)
            .fade_out(false)
            .frame(frame)
            .default_pos(self.default_pos)
            .resizable(false);
        if self.auto_height && !minimized {
            w = w.auto_sized().min_width(self.default_size.x).max_width(self.default_size.x);
        } else if self.resizable && !minimized {
            w = w.default_size(self.default_size).min_size(self.min_size);
        } else if !minimized {
            w = w.default_width(self.default_size.x);
        }
        // LLFloater::setMinimized keeps the expanded geometry. egui's Resize
        // otherwise retains the old height when the body disappears, and then
        // forgets it when clamped to the header. Restore that size once.
        // Aurora keeps the dragged bar's current position when restoring.
        if let Some(width) = self.compact_minimized.filter(|_| compact) {
            w = w.auto_sized().min_width(width).max_width(width);
        } else if minimized {
            w = w.fixed_size(Vec2::new(
                minimized_width(expanded.x),
                if self.framed_header.is_some() { 40.0 } else { 38.0 },
            ));
        } else if restore && !self.auto_height {
            w = w.fixed_size(expanded.min(work.size()));
        } else if let Some(size) = shrink {
            w = w.fixed_size(size);
        }
        // the viewer window changed size: the floater follows its edges;
        // a placement asked this frame and a resize in progress come after
        let slot = super::layout::slot(ctx, window_id);
        let mut asked = slot.pos;
        // (a window dragged with its group has its own limits; the others
        // stay off the bars, which egui's own limit, the screen, lets them
        // cover)
        w = w.constrain_to(slot.limits.unwrap_or(work));
        if let Some(pos) = ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(window_id.with(PLACE_POS))) {
            asked = Some(pos);
        }
        // the edge dragged snaps to the other windows and to the work area
        let resized = resize_drag.map(|drag| super::layout::resizing(ctx, p, window_id, drag.rect(), drag.edges(), drag.minimum()));
        if let Some(rect) = resized {
            w = w.fixed_size(rect.size());
            asked = Some(rect.min);
        }
        // Whatever places the window, its title bar stays in reach: egui
        // lets a window larger than its limits slide out of them by what it
        // has in excess, title first.
        if let Some(pos) = title_in_reach(ctx, window_id, work, asked) {
            w = w.current_pos(pos);
        }
        let mut glass_slot = None;
        let response = w.show(ctx, |ui| {
            glass_slot = Some((ui.painter().clone(), ui.painter().add(egui::Shape::Noop)));
            // Our custom resize handles replace egui's native resizability.
            // Reserve the requested content area as a resizable Window would:
            // height-filling scroll areas otherwise shave a few pixels off
            // each restoration even though Resize remembered the right size.
            if resizable || (restore && !self.auto_height) {
                ui.set_min_size(ui.available_size());
            }
            // title bar
            let mut buttons_left = f32::INFINITY;
            let framed_header = self.framed_header;
            if framed_header.is_some() {
                ui.spacing_mut().item_spacing.y = 0.0;
            }
            let mut header_content = |ui: &mut egui::Ui| {
                if framed_header.is_some() {
                    ui.spacing_mut().interact_size.y = 20.0;
                }
                ui.horizontal(|ui| {
                    ui.set_min_height(if framed_header.is_some() { 20.0 } else { 28.0 });
                    let title = RichText::new(&self.title).size(14.0).strong().color(p.ink);
                    // a fixed floater is as wide as its title; a resizable one
                    // cuts a long title short before the buttons (LLFloater
                    // titles end with an ellipsis)
                    let truncate = self.resizable || (minimized && !compact) || self.auto_height;
                    if !truncate {
                        glyph(ui, workspace::window_icon(self.id), p.violet_light, 18.0);
                        ui.label(title.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if title_button(ui, p, "x").on_hover_text(aurora_i18n::tr!("common-close")).clicked() {
                            close = true;
                        }
                        if title_button(ui, p, "minus")
                            .on_hover_text(if minimized {
                                aurora_i18n::tr!("common-expand")
                            } else {
                                aurora_i18n::tr!("common-collapse")
                            })
                            .clicked()
                        {
                            toggle_min = true;
                        }
                        if let Some((icon, tip, clicked)) = self.action.as_mut()
                            && title_button(ui, p, icon).on_hover_text(*tip).clicked()
                        {
                            **clicked = true;
                        }
                        if let Some(h) = self.help {
                            title_button(ui, p, "help-circle-filled").on_hover_text(h);
                        }
                        buttons_left = ui.cursor().right();
                        if truncate {
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                {
                                    let icon = framed_header.unwrap_or_else(|| workspace::window_icon(self.id));
                                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(17.0), Sense::hover());
                                    if let Some(t) = super::icons::global(icon) {
                                        ui.painter().image(
                                            t.id(),
                                            rect,
                                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                            p.violet_light,
                                        );
                                    }
                                }
                                ui.add(egui::Label::new(title).truncate());
                            });
                        }
                    });
                })
            };
            let header = if framed_header.is_some() {
                egui::Frame::new()
                    .fill(p.bar)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 10,
                        bottom: 10,
                    })
                    .show(ui, &mut header_content)
                    .response
            } else {
                header_content(ui).response
            };
            let mut title_rect = header.rect;
            if framed_header.is_some() {
                ui.painter().hline(header.rect.x_range(), header.rect.bottom(), divider_stroke(p));
            }
            title_rect.max.x = buttons_left;
            // Keep the title buttons outside the double-click target; Window
            // still owns dragging the bar, including when it is minimized.
            if ui
                .interact(title_rect, window_id.with("title_double_click"), Sense::click())
                .double_clicked()
            {
                toggle_min = true;
            }
            if compact {
                divider(ui, p);
                ui.add_space(5.0);
                out = Some(body(ui));
            } else if !minimized {
                if framed_header.is_none() {
                    if self.compact_header {
                        ui.painter()
                            .hline(header.rect.x_range(), header.rect.bottom() + 2.0, divider_stroke(p));
                    } else {
                        divider(ui, p);
                        ui.add_space(5.0);
                    }
                }
                if scrolls {
                    let above = ui.cursor().top() - ui.max_rect().top();
                    let room = (work.height() - above - BODY_CHROME).max(MIN_SCROLLED_BODY);
                    let shown = egui::ScrollArea::vertical().id_salt("floater_body").max_height(room).show(ui, body);
                    still_too_tall = shown.content_size.y > room + 0.5;
                    out = Some(shown.inner);
                } else {
                    out = Some(body(ui));
                }
            }
        });
        if let Some(response) = response {
            if content_sized {
                let too_tall = if scrolls {
                    still_too_tall
                } else {
                    response.response.rect.height() > work.height() + 0.5
                };
                ctx.data_mut(|d| d.insert_temp(scroll_id, too_tall));
            }
            slot.shown(ctx, response.response.rect, minimized);
            if let Some((painter, slot)) = glass_slot {
                let radius = f32::from(ctx.global_style().visuals.window_corner_radius.nw);
                workspace::finish_glass(
                    &painter.with_clip_rect(response.response.rect),
                    slot,
                    response.response.rect,
                    p,
                    radius,
                );
            }
            if !minimized {
                ctx.data_mut(|d| d.insert_persisted(expanded_id, response.response.rect.size()));
            } else if self.attention {
                // the bar lights up in the accent while something waits inside
                ctx.layer_painter(response.response.layer_id).rect_stroke(
                    response.response.rect,
                    ctx.global_style().visuals.window_corner_radius,
                    Stroke::new(1.5, p.violet_light),
                    egui::StrokeKind::Inside,
                );
            }
            if let (Some(drag), Some(requested)) = (resize_drag, resized) {
                drag.finish(ctx, window_id, requested, response.response.rect);
            }
            if resizable {
                resize::handles(ctx, p, &response.response);
            }
        }
        if close {
            *open = false;
            // LLFloater::closeFloater: "Always unminimize before trying to
            // close", the window opens again expanded
            unminimize_quietly(ctx, self.id);
        } else if toggle_min {
            ctx.data_mut(|d| d.insert_persisted(min_id, !minimized));
            if minimized {
                ctx.data_mut(|d| d.insert_temp(restore_id, true));
            }
            // LLFloater::setMinimized(false) plays UISndWindowClose
            if minimized && !self.silent {
                super::sound_cues::request(ctx, crate::ui_sound::UiSound::WindowClose);
            }
        }
        out
    }
}

/// A window without a title bar, dragged by its whole surface (the
/// mini-map): `frame` around `body`, which fills it. Like a floater it is
/// placed by the layout (`layout::Slot`), snaps to the other windows when
/// moved or resized, and is resized by the same edge handles.
pub(crate) fn bare_window(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    title: &str,
    frame: egui::Frame,
    default: egui::Rect,
    min_size: Vec2,
    body: impl FnOnce(&mut egui::Ui),
) {
    let window_id = egui::Id::new(id);
    let drag = resize::Drag::prepare(ctx, window_id, min_size);
    let mut w = egui::Window::new(title)
        .id(window_id)
        .title_bar(false)
        .frame(frame)
        .fade_in(false)
        .fade_out(false)
        .default_pos(default.min)
        .default_size(default.size())
        .min_size(min_size)
        .resizable(false)
        .constrain(true);
    let slot = super::layout::slot(ctx, window_id);
    if let Some(pos) = slot.pos {
        w = w.current_pos(pos);
    }
    if let Some(limits) = slot.limits {
        w = w.constrain_to(limits);
    }
    let resized = drag.map(|drag| super::layout::resizing(ctx, p, window_id, drag.rect(), drag.edges(), drag.minimum()));
    if let Some(rect) = resized {
        w = w.fixed_size(rect.size()).current_pos(rect.min);
    }
    let Some(shown) = w.show(ctx, |ui| {
        // as a resizable egui::Window would: the body gets the whole size
        ui.set_min_size(ui.available_size());
        body(ui);
    }) else {
        return;
    };
    slot.shown(ctx, shown.response.rect, false);
    if let (Some(drag), Some(requested)) = (drag, resized) {
        drag.finish(ctx, window_id, requested, shown.response.rect);
    }
    resize::handles(ctx, p, &shown.response);
}

const PLACE_POS: &str = "place_pos";

/// Movable toolbar without a common face. The body supplies its individual
/// button surfaces and close action; layout placement remains shared.
pub(crate) fn toolstrip<R>(ctx: &egui::Context, id: &str, pos: egui::Pos2, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let window_id = egui::Id::new(id);
    super::sound_cues::floater_shown(ctx, window_id);
    let slot = super::layout::slot(ctx, window_id);
    let mut area = egui::Area::new(window_id)
        .order(egui::Order::Middle)
        .fade_in(false)
        .movable(true)
        .default_pos(pos)
        .constrain_to(slot.limits.unwrap_or_else(|| super::layout::work_area(ctx)));
    let asked = ctx
        .data_mut(|data| data.remove_temp::<egui::Pos2>(window_id.with(PLACE_POS)))
        .or(slot.pos);
    if let Some(pos) = asked {
        area = area.current_pos(pos);
    }
    let shown = area.show(ctx, body);
    slot.shown(ctx, shown.response.rect, false);
    shown.inner
}

/// Move a dependent floater without changing its size or minimized state.
pub fn move_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(id).with(PLACE_POS), pos));
}

/// Give the floater `id` this position and size on its next frame, expanded,
/// as if the user had moved and resized it: a demo scenario then shows the
/// same thing whatever layout the previous run left in ui_layout.ron.
pub fn place_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2, size: Vec2) {
    let window_id = egui::Id::new(id);
    ctx.data_mut(|d| {
        d.insert_persisted(egui::Id::new((id, "minimized")), false);
        d.insert_persisted(window_id.with("expanded_size"), size);
        d.insert_temp(window_id.with("restore_size"), true);
        d.insert_temp(window_id.with(PLACE_POS), pos);
    });
}

/// Minimize the floater `id` (LLFloater::setMinimized), as its title button
/// does.
pub fn minimize_floater(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_persisted(egui::Id::new((id, "minimized")), true));
}

/// Is the floater `id` reduced to its title bar?
pub fn floater_minimized(ctx: &egui::Context, id: &str) -> bool {
    ctx.data_mut(|d| d.get_persisted::<bool>(egui::Id::new((id, "minimized"))))
        .unwrap_or(false)
}

/// Expand the minimized floater `id` on its next frame and bring it to the
/// front, as its title button does (LLFloater::openFloater on a minimized
/// floater: setMinimized(false), setVisibleAndFrontmost).
pub fn expand_floater(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(id).with(EXPAND), true));
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id)));
}

/// The toolbar button, menu entry or shortcut of the floater `id`
/// (LLFloaterReg::toggleInstance): a floater shown closes, a minimized one
/// expands, a closed one opens expanded. LLFloater::openFloater and
/// closeFloater both unminimize, so a window closed while minimized does not
/// come back as a bar.
pub fn toggle_floater(ctx: &egui::Context, id: &str, open: &mut bool) {
    if *open && floater_minimized(ctx, id) {
        expand_floater(ctx, id);
        return;
    }
    *open = !*open;
    unminimize_quietly(ctx, id);
}

/// Open the floater `id` (LLFloaterReg::showInstance): expanded, whether it
/// was closed or minimized.
pub fn open_floater(ctx: &egui::Context, id: &str, open: &mut bool) {
    if *open {
        if floater_minimized(ctx, id) {
            expand_floater(ctx, id);
        }
    } else {
        *open = true;
        unminimize_quietly(ctx, id);
    }
}

/// Forget that a floater not on screen was minimized: it opens expanded, at
/// its expanded size, without the sound of the title button.
fn unminimize_quietly(ctx: &egui::Context, id: &str) {
    if floater_minimized(ctx, id) {
        let window_id = egui::Id::new(id);
        ctx.data_mut(|d| {
            d.insert_persisted(egui::Id::new((id, "minimized")), false);
            d.insert_temp(window_id.with("restore_size"), true);
        });
    }
}

/// Temporary flag of a floater asked to expand (`expand_floater`).
const EXPAND: &str = "expand";

/// Title bar button with a Tabler icon (drawn by hand if it is missing).
fn title_button(ui: &mut egui::Ui, p: &Palette, icon: &str) -> egui::Response {
    title_button_sized(ui, p, icon, Vec2::splat(16.0))
}

/// The standard plain close glyph, with a toolbar-sized hit target.
pub(crate) fn close_button(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let label = aurora_i18n::tr!("common-close");
    let response = title_button_sized(ui, p, "x", Vec2::new(24.0, 36.0));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    response.on_hover_text(label).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn title_button_sized(ui: &mut egui::Ui, p: &Palette, icon: &str, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let col = if resp.hovered() { p.ink } else { p.muted };
    let c = rect.center();
    let stroke = Stroke::new(1.4, col);
    let painter = ui.painter();
    if let Some(t) = super::icons::global(icon) {
        let r = egui::Rect::from_center_size(c, Vec2::splat(14.0));
        painter.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
        return resp;
    }
    match icon {
        "x" => {
            painter.line_segment([c + Vec2::new(-4.0, -4.0), c + Vec2::new(4.0, 4.0)], stroke);
            painter.line_segment([c + Vec2::new(4.0, -4.0), c + Vec2::new(-4.0, 4.0)], stroke);
        }
        "minus" => {
            painter.line_segment([c + Vec2::new(-5.0, 1.0), c + Vec2::new(5.0, 1.0)], stroke);
        }
        "help-circle-filled" => {
            painter.circle_stroke(c, 6.0, Stroke::new(1.2, col));
            painter.text(c, egui::Align2::CENTER_CENTER, "?", egui::FontId::proportional(10.0), col);
        }
        _ => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::splat(10.0)),
                inner_radius(ui.visuals()),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
    }
    resp
}

/// The scroll bars of every scroll area, set once by `Theme::apply`: thin and
/// solid, in a strip of their own at the right of the content, like
/// Firestorm's LLScrollContainer, whose visible width loses the bar's
/// (indra/llui/llscrollcontainer.cpp, calcVisibleSize). egui's default
/// floating bar is drawn over the content when hovered, over the buttons and
/// values at the right edge. The strip is taken only when the content
/// overflows. 6 points of bar, 4 between it and the content, faintly shown
/// at rest. `ui/theme_guard.rs` keeps windows from setting another style.
pub fn reserved_scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        dormant_background_opacity: 0.2,
        dormant_handle_opacity: 0.6,
        ..egui::style::ScrollStyle::solid()
    }
}

/// The width left to the rows of a vertical scroll area opened next in `ui`
/// once its bar shows: for rows sized outside the area (a table whose
/// header sits above it), which would otherwise run under the bar's strip
/// and be clipped.
pub fn scrolled_width(ui: &egui::Ui) -> f32 {
    (ui.available_width() - scroll_strip(ui)).max(0.0)
}

/// The width of a header of columns drawn above the rows of a vertical
/// scroll area: less the bar's strip while the list showed its bar (as
/// `remember_scroll_bar` noted under the same `list` id), so the columns of
/// the header stay over those of the rows.
pub fn header_width(ui: &egui::Ui, list: egui::Id) -> f32 {
    let shown = ui.ctx().data(|d| d.get_temp::<bool>(list)).unwrap_or(false);
    ui.available_width() - if shown { scroll_strip(ui) } else { 0.0 }
}

/// Note whether the scroll area of `out` showed its vertical bar, for the
/// `header_width` of its list next frame.
pub fn remember_scroll_bar<R>(ui: &egui::Ui, list: egui::Id, out: &egui::scroll_area::ScrollAreaOutput<R>) {
    let shown = out.content_size.y > out.inner_rect.height() + 0.5;
    ui.ctx().data_mut(|d| d.insert_temp(list, shown));
}

/// Width of the strip a scroll bar takes at the right of what scrolls:
/// what rows of a known width give up to stay clear of it.
pub fn scroll_strip(ui: &egui::Ui) -> f32 {
    ui.spacing().scroll.allocated_width()
}

/// Stroke of every divider of the interface: a thin line of a neutral
/// hardly lighter than the panels.
pub fn divider_stroke(p: &Palette) -> Stroke {
    Stroke::new(1.0, p.raised)
}

/// Thin line between two zones of a window, across the layout (horizontal in
/// a column, vertical in a row), in the room of egui's separator: a window
/// keeps its layout. Not for buttons or toolbar items: their spacing and the
/// pill of the hovered one tell them apart. A menu uses `menu::separator`.
pub fn divider(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    const ROOM: f32 = 6.0;
    let across = !ui.layout().main_dir().is_horizontal();
    let available = if ui.is_sizing_pass() {
        Vec2::ZERO
    } else {
        ui.available_size_before_wrap()
    };
    let size = if across {
        Vec2::new(available.x, ROOM)
    } else {
        Vec2::new(ROOM, available.y)
    };
    let (rect, resp) = ui.allocate_at_least(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        if across {
            ui.painter().hline(rect.x_range(), rect.center().y, divider_stroke(p));
        } else {
            ui.painter().vline(rect.center().x, rect.y_range(), divider_stroke(p));
        }
    }
    resp
}

// theme-guard: content — a picture keeps its own colors
/// Tint of a picture drawn with its own colors (texture, snapshot, emoji,
/// web page): none.
pub const PICTURE_TINT: Color32 = Color32::WHITE;

// theme-guard: content — the same picture, darkened
/// Tint of a picture whose control is disabled.
pub const PICTURE_TINT_DISABLED: Color32 = Color32::from_gray(120);

/// Corners of a small mark inside a control or a line (the knob of a switch,
/// a key cap, a status square, a letter badge): one pixel tighter than the
/// windows', so that a mark of a few pixels stays a square.
pub fn inner_radius(visuals: &egui::Visuals) -> CornerRadius {
    CornerRadius::same(visuals.window_corner_radius.nw.saturating_sub(1).max(1))
}

/// Ends of a thin bar (the track of a progress bar or of a level meter, a
/// mark on a rail): the radius of the theme would turn a bar of a few pixels
/// into a pill.
pub const TRACK_RADIUS: f32 = 2.0;

/// Size of a `switch`.
pub const SWITCH_SIZE: Vec2 = Vec2::new(34.0, 18.0);

/// Flat on/off switch: a track with the corners of the theme, and a square
/// knob that slides in it.
pub fn switch(ui: &mut egui::Ui, p: &Palette, on: &mut bool) -> egui::Response {
    // a whole number of pixels around the knob keeps its edges crisp
    const KNOB: f32 = 14.0;
    let (rect, mut resp) = ui.allocate_exact_size(SWITCH_SIZE, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let track = if *on { p.violet } else { p.raised };
    let radius = ui.visuals().window_corner_radius;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, track);
    let half = rect.height() / 2.0;
    let x = egui::lerp(rect.left() + half..=rect.right() - half, t);
    let knob = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(KNOB));
    painter.rect_filled(knob, inner_radius(ui.visuals()), if *on { p.on_accent } else { p.muted });
    resp
}

/// Check box of the interface, with the contract of `ui.checkbox`: the
/// response is `changed()` when a click flipped `on`. A flat square with the
/// corners of the theme, violet and ticked when on; the square and its label
/// are one clickable row.
pub fn checkbox(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    check_row(ui, p, on, label.into(), false)
}

/// `checkbox` showing a dash instead of its tick while `mixed`: the objects
/// of a selection do not agree.
pub fn checkbox_mixed(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: impl Into<egui::WidgetText>, mixed: bool) -> egui::Response {
    check_row(ui, p, on, label.into(), mixed)
}

/// The row shared by the check box and the radio button: the room of the
/// mark, then the label, one clickable rectangle. The geometry of the stock
/// ones: a window keeps its layout.
fn mark_row(ui: &mut egui::Ui, label: egui::WidgetText) -> (egui::Rect, egui::Response, Option<std::sync::Arc<egui::Galley>>) {
    let (side, gap) = (ui.spacing().icon_width, ui.spacing().icon_spacing);
    let galley =
        (!label.is_empty()).then(|| label.into_galley(ui, None, (ui.available_width() - side - gap).max(0.0), egui::TextStyle::Button));
    let mut size = Vec2::new(side, 0.0);
    if let Some(galley) = &galley {
        size += Vec2::new(gap, 0.0) + galley.size();
    }
    let size = size.max(Vec2::splat(ui.spacing().interact_size.y));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    (rect, resp, galley)
}

/// Where the mark of a `mark_row` goes: a square at the left of the row.
fn mark_square(ui: &egui::Ui, row: egui::Rect) -> egui::Rect {
    let side = ui.spacing().icon_width;
    egui::Rect::from_center_size(egui::pos2(row.left() + side / 2.0, row.center().y), Vec2::splat(side))
}

/// Face of the mark: violet when on, the raised tone when off, both lighter
/// under the pointer or with the keyboard focus.
fn mark_fill(p: &Palette, on: bool, hot: bool) -> Color32 {
    match (on, hot) {
        (true, false) => p.violet,
        (true, true) => p.violet_light,
        (false, false) => p.raised,
        (false, true) => p.raised.lerp_to_gamma(p.ink, 0.16),
    }
}

fn mark_label(ui: &egui::Ui, p: &Palette, row: egui::Rect, mark: egui::Rect, galley: Option<std::sync::Arc<egui::Galley>>) {
    if let Some(galley) = galley {
        let text = egui::pos2(mark.right() + ui.spacing().icon_spacing, row.center().y - galley.size().y / 2.0);
        ui.painter().galley(text, galley, p.ink);
    }
}

fn check_row(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: egui::WidgetText, mixed: bool) -> egui::Response {
    let (rect, mut resp, galley) = mark_row(ui, label);
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let checked = *on;
    resp.widget_info(|| {
        let text = galley.as_ref().map_or("", |galley| galley.text());
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), checked, text)
    });
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered() || resp.has_focus();
        let square = mark_square(ui, rect);
        let painter = ui.painter();
        painter.rect_filled(square, ui.visuals().window_corner_radius, mark_fill(p, checked || mixed, hot));
        if !(checked || mixed) {
            // an empty box stays visible on a panel of nearly its own color
            painter.rect_stroke(
                square,
                ui.visuals().window_corner_radius,
                Stroke::new(1.0, p.muted_dim),
                egui::StrokeKind::Inside,
            );
        }
        let mark = Stroke::new(1.6, p.on_accent);
        if mixed {
            painter.hline(square.shrink(3.5).x_range(), square.center().y, mark);
        } else if checked {
            match super::icons::global("check") {
                Some(icon) => {
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    painter.image(icon.id(), square.shrink(1.0), uv, p.on_accent);
                }
                // no icon theme: the tick is drawn
                None => {
                    let c = square.center();
                    let tick = [c + Vec2::new(-3.5, 0.0), c + Vec2::new(-1.0, 2.5), c + Vec2::new(3.5, -2.5)];
                    painter.line_segment([tick[0], tick[1]], mark);
                    painter.line_segment([tick[1], tick[2]], mark);
                }
            }
        }
        mark_label(ui, p, rect, square, galley);
    }
    resp
}

/// Radio button of the interface, with the contract of `ui.radio`: the
/// response is `clicked()` when its row was clicked, selected or not. The row
/// of `checkbox` (same size, same label, same hover) around a round mark, so
/// that one choice among several is never taken for a check box: a thin ring
/// when empty, violet around a dot when selected. The one round control of
/// the interface, with the status dots (docs/BRANDING.md).
pub fn radio(ui: &mut egui::Ui, p: &Palette, selected: bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    let (rect, resp, galley) = mark_row(ui, label.into());
    resp.widget_info(|| {
        let text = galley.as_ref().map_or("", |galley| galley.text());
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected, text)
    });
    if ui.is_rect_visible(rect) {
        let hot = resp.hovered() || resp.has_focus();
        let square = mark_square(ui, rect);
        let (center, radius) = (square.center(), square.width() / 2.0);
        let painter = ui.painter();
        painter.circle_filled(center, radius, mark_fill(p, selected, hot));
        if selected {
            painter.circle_filled(center, radius * RADIO_DOT, p.on_accent);
        } else {
            // as the empty check box: visible on a panel of nearly its color
            painter.circle_stroke(center, radius - 0.5, Stroke::new(1.0, p.muted_dim));
        }
        mark_label(ui, p, rect, square, galley);
    }
    resp
}

/// The dot of a selected radio button, as a share of its radius.
const RADIO_DOT: f32 = 0.42;

/// `radio` for one alternative of a value, with the contract of
/// `ui.radio_value`: a click makes `current` this `alternative`, and the
/// response is `changed()` when that changed it.
pub fn radio_value<Value: PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    current: &mut Value,
    alternative: Value,
    label: impl Into<egui::WidgetText>,
) -> egui::Response {
    let mut resp = radio(ui, p, *current == alternative, label);
    if resp.clicked() && *current != alternative {
        *current = alternative;
        resp.mark_changed();
    }
    resp
}

/// A switch followed by its label, both clickable: a lone on / off setting
/// of a window. The response is `changed()` when either flipped `on`.
pub fn switch_labeled(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    ui.horizontal(|ui| {
        let mut resp = switch(ui, p, on);
        let text = ui.add(egui::Label::new(label).sense(Sense::click()));
        if text.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        resp.union(text)
    })
    .inner
}

/// Flat tab strip. Returns true when the selection changed.
pub fn tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = labels.len().max(1) as f32;
        let font = egui::FontId::proportional(12.0);
        let text_w: Vec<f32> = labels
            .iter()
            .map(|(l, _)| ui.fonts_mut(|f| f.layout_no_wrap((*l).to_owned(), font.clone(), Color32::PLACEHOLDER).size().x) + 18.0)
            .collect();
        let gaps = (n - 1.0) * ui.spacing().item_spacing.x;
        let total: f32 = text_w.iter().sum::<f32>() + gaps;
        let compact = ((ui.available_width() - gaps) / (total - gaps)).clamp(0.0, 1.0);
        let extra = ((ui.available_width() - total) / n).max(0.0);
        for (i, (label, enabled)) in labels.iter().enumerate() {
            let w = text_w[i] * compact + extra.min(40.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), if *enabled { Sense::click() } else { Sense::hover() });
            let sel = *selected == i;
            if sel || (resp.hovered() && *enabled) {
                let fill = if sel {
                    p.violet.gamma_multiply(0.18)
                } else {
                    p.raised.gamma_multiply(0.25)
                };
                ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
            }
            underline(ui, p, &resp, sel, *enabled);
            let col = if !*enabled {
                p.muted_dim
            } else if sel {
                p.ink
            } else {
                p.muted
            };
            let galley = egui::WidgetText::from(*label).into_galley(
                ui,
                Some(egui::TextWrapMode::Truncate),
                (rect.width() - 8.0).max(0.0),
                font.clone(),
            );
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, col);
            let resp = resp.on_hover_text(*label);
            if resp.clicked() && !sel {
                *selected = i;
                changed = true;
            }
        }
    });
    changed
}

/// Height of a `segmented` choice, room around its segments, and room
/// around a segment's label.
const SEGMENT_H: f32 = 26.0;
const SEGMENT_PAD: f32 = 2.0;
const SEGMENT_PAD_X: f32 = 20.0;

fn segment_galleys(ui: &egui::Ui, options: &[(&str, bool)]) -> Vec<std::sync::Arc<egui::Galley>> {
    options
        .iter()
        .map(|(label, _)| {
            ui.painter()
                .layout_no_wrap((*label).to_owned(), egui::FontId::proportional(12.0), Color32::PLACEHOLDER)
        })
        .collect()
}

/// The room a `segmented` choice takes, to lay it out before drawing it.
pub fn segmented_size(ui: &egui::Ui, options: &[(&str, bool)]) -> Vec2 {
    let labels: f32 = segment_galleys(ui, options).iter().map(|g| g.size().x + SEGMENT_PAD_X).sum();
    Vec2::new(labels + SEGMENT_PAD * 2.0, SEGMENT_H)
}

/// Choice between a few options side by side, in one track: the chosen
/// segment is lit, in the accent when its option is `accent` (Aurora's own
/// look), in `raised` otherwise (a legacy look). Returns true when the
/// selection changed.
pub fn segmented(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, options: &[(&str, bool)]) -> bool {
    let galleys = segment_galleys(ui, options);
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + SEGMENT_PAD_X).collect();
    let (track, _) = ui.allocate_exact_size(segmented_size(ui, options), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(track, radius, p.bar);
    let mut changed = false;
    let mut x = track.left() + SEGMENT_PAD;
    for (i, ((label, accent), galley)) in options.iter().zip(galleys).enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, track.top() + SEGMENT_PAD),
            Vec2::new(widths[i], track.height() - SEGMENT_PAD * 2.0),
        );
        x += widths[i];
        let resp = ui.interact(rect, ui.id().with(("segmented", i, *label)), Sense::click());
        let sel = *selected == i;
        let (face, text) = match (sel, *accent) {
            (true, true) => (Some(p.violet), p.on_accent),
            (true, false) => (Some(p.raised), p.ink),
            (false, _) if resp.hovered() => (Some(p.raised.gamma_multiply(0.5)), p.ink),
            (false, _) => (None, p.muted),
        };
        if let Some(face) = face {
            ui.painter().rect_filled(rect, inner_radius(ui.visuals()), face);
        }
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, text);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, sel, *label));
        if resp.clicked() && !sel {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// Column header row for list views.
pub fn header_row(ui: &mut egui::Ui, p: &Palette, cols: &[(&str, f32)]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.field);
    let mut x = rect.left() + 6.0;
    for (label, w) in cols {
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(12.0),
            p.muted,
        );
        x += if *w > 0.0 { *w } else { rect.width() * 0.5 };
    }
}

pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

pub fn search_field(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width))
}

/// Small flat button with a Tabler icon (copy / paste / flip...).
pub fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool) -> bool {
    small_icon_button(ui, p, icon, tip, enabled, false)
}

/// `icon_button` that stays lit while `on` (a mode that is armed); true
/// when clicked.
pub fn icon_toggle(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool) -> bool {
    small_icon_button(ui, p, icon, tip, true, on)
}

fn small_icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(20.0, 20.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if on {
        p.violet
    } else if resp.hovered() && enabled {
        p.raised
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    let col = match (enabled, on) {
        (false, _) => p.muted_dim,
        (true, true) => p.on_accent,
        (true, false) => p.ink,
    };
    match super::icons::global(icon) {
        Some(t) => {
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                col,
            );
        }
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(10.0),
                col,
            );
        }
    }
    resp.on_hover_text(tip).clicked() && enabled
}

/// Small flat text button.
pub fn flat_button(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    text_button(ui, p, label, None)
}

/// A light toolbar action using the same gray/active underline as the tabs.
pub(crate) fn underlined_button(ui: &mut egui::Ui, p: &Palette, label: &str, selected: bool) -> egui::Response {
    let response = ui.add(
        egui::Button::new(RichText::new(label).size(12.0).color(p.ink))
            .frame(false)
            .min_size(Vec2::new(0.0, ui.spacing().interact_size.y)),
    );
    underline(ui, p, &response, selected || response.is_pointer_button_down_on(), ui.is_enabled());
    response
}

fn underline(ui: &egui::Ui, p: &Palette, response: &egui::Response, selected: bool, enabled: bool) {
    let color = if selected {
        p.violet
    } else if response.hovered() && enabled {
        p.muted
    } else {
        p.muted_dim.gamma_multiply(0.7)
    };
    let height = if selected { 2.0 } else { 1.0 };
    let rect = response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.left_bottom() - Vec2::new(-6.0, height),
            Vec2::new((rect.width() - 12.0).max(1.0), height),
        ),
        0.0,
        color,
    );
}

/// Paint `galley` from `left_center`, centered vertically on its ink like
/// an icon beside it (a font's line box sits lower than its letters).
pub fn paint_ink_left(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, left_center: egui::Pos2, color: Color32) {
    let center = left_center + Vec2::new(galley.size().x / 2.0, 0.0);
    paint_ink_centered(painter, galley, center, color);
}

/// Equal-sized dialog buttons; long script labels keep their full hover text.
pub fn flat_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    text_button(ui, p, label, Some(size)).on_hover_text(label)
}

fn text_button(ui: &mut egui::Ui, p: &Palette, label: &str, size: Option<Vec2>) -> egui::Response {
    // Keep the button in the parent's layout so wrapped toolbars can break
    // before it. A child UI would wrap internally and enlarge narrow windows.
    // The theme gives every button its plain filled face.
    let button = egui::Button::new(RichText::new(label).size(12.0).color(p.ink));
    match size {
        Some(size) => ui.add_sized(size, button.truncate()),
        None => ui.add(button.min_size(Vec2::new(0.0, 20.0))),
    }
}

/// Signed height adjustment: keep egui's pointer/keyboard behaviour, with a
/// visible Aurora rail, zero marker, accent fill and solid rectangular knob.
/// The surrounding floater supplies the shared glass, no opaque card here.
pub fn height_slider(
    ui: &mut egui::Ui,
    p: &Palette,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    subdued: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        let aspect = 0.6;
        let slider = ui
            .scope(|ui| {
                // Paint only the slider ourselves; the numeric field keeps the
                // theme's normal input face. The stock slider still owns input.
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.bg_fill = Color32::TRANSPARENT;
                widgets.inactive.fg_stroke = Stroke::NONE;
                widgets.hovered.bg_fill = Color32::TRANSPARENT;
                widgets.hovered.fg_stroke = Stroke::NONE;
                widgets.active.bg_fill = Color32::TRANSPARENT;
                widgets.active.fg_stroke = Stroke::NONE;
                ui.add(
                    egui::Slider::new(&mut *value, range.clone())
                        .step_by(0.001)
                        .clamping(egui::SliderClamping::Edits)
                        .smart_aim(false)
                        .show_value(false)
                        .trailing_fill(false)
                        .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: aspect }),
                )
            })
            .inner;
        let rect = slider.rect;
        let half_width = rect.height() / 2.5 * aspect; // egui Slider::position_range
        let left = rect.left() + half_width;
        let right = rect.right() - half_width;
        let at = |v: f32| egui::lerp(left..=right, ((v - range.start()) / (range.end() - range.start())).clamp(0.0, 1.0));
        let x = at(*value);
        let zero = at(0.0);
        let fade = if ui.is_enabled() { 1.0 } else { 0.45 };
        let rail = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 8.0));
        let painter = ui.painter();
        painter.rect(
            rail,
            TRACK_RADIUS,
            p.muted_dim.lerp_to_gamma(p.raised, 0.25).gamma_multiply(fade),
            Stroke::new(1.0, p.muted.gamma_multiply(0.5 * fade)),
            egui::StrokeKind::Inside,
        );
        let fill = egui::Rect::from_min_max(egui::pos2(zero.min(x), rail.top()), egui::pos2(zero.max(x), rail.bottom()));
        if fill.width() > 0.5 {
            painter.rect_filled(
                fill,
                TRACK_RADIUS,
                (if subdued { p.muted_dim } else { p.violet }).gamma_multiply(fade),
            );
        }
        painter.line_segment(
            [egui::pos2(zero, rail.top() - 2.0), egui::pos2(zero, rail.bottom() + 2.0)],
            Stroke::new(1.0, p.muted.gamma_multiply(fade)),
        );
        let hot = slider.hovered() || slider.dragged() || slider.has_focus();
        let knob = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::new(half_width * 2.0, rect.height() / 1.25));
        painter.rect(
            knob,
            inner_radius(ui.visuals()),
            (if subdued {
                p.muted
            } else if hot {
                p.on_accent
            } else {
                p.violet_light
            })
            .gamma_multiply(fade),
            Stroke::new(1.0, p.on_accent.gamma_multiply(0.75 * fade)),
            egui::StrokeKind::Inside,
        );
        let numeric = ui.add_sized(
            Vec2::new(78.0, rect.height()),
            egui::DragValue::new(value)
                .range(range)
                .clamp_existing_to_range(false)
                .speed(0.001)
                .fixed_decimals(3)
                .suffix(" m")
                .custom_formatter(|value, _| aurora_i18n::number(value, 3))
                .custom_parser(|text| aurora_i18n::parse_number(text).filter(|v| v.is_finite())),
        );
        if numeric.gained_focus() || numeric.has_focus() || numeric.lost_focus() {
            numeric.union(slider)
        } else {
            slider.union(numeric)
        }
    })
    .inner
}

/// Progress bar of the interface: a flat track filled with the brand
/// gradient (or `tint`), which glides to its value instead of jumping.
/// `text` is written under it; `{}` in it is replaced by the percentage
/// shown, so the figures glide with the bar.
pub fn progress(ui: &mut egui::Ui, p: &Palette, id: &str, fraction: f32, width: f32, text: &str, tint: Option<Color32>) {
    let shown = ui.ctx().animate_value_with_time(ui.id().with(id), fraction.clamp(0.0, 1.0), 0.45);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
        // the track of the sliders: it shows on a window as on a group
        ui.painter().rect_filled(rect, TRACK_RADIUS, p.raised);
        let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * shown, rect.height()));
        if fill.width() >= 1.0 {
            match tint {
                Some(color) => {
                    ui.painter().rect_filled(fill, TRACK_RADIUS, color);
                }
                None => super::loading::gradient_rect(ui.painter(), fill, p.violet, p.teal, true),
            }
        }
        if !text.is_empty() {
            let percent = aurora_i18n::number_text(&format!("{:.0} %", shown * 100.0));
            ui.label(RichText::new(text.replace("{}", &percent)).size(11.5).color(p.muted));
        }
    });
}

/// Level meter (microphone): the track of `progress` with a plain fill that
/// follows its value closely, only smoothing the jitter between two samples.
pub fn level(ui: &mut egui::Ui, p: &Palette, id: &str, value: f32, width: f32, color: Color32) {
    let shown = ui.ctx().animate_value_with_time(ui.id().with(id), value.clamp(0.0, 1.0), 0.08);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, TRACK_RADIUS, p.raised);
    let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * shown, rect.height()));
    ui.painter().rect_filled(fill, TRACK_RADIUS, color);
}

/// Face of an `action_button`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// A neutral filled face, without a border.
    Flat,
    /// The accent: the action the screen is for.
    Primary,
    /// The danger color: a destructive action.
    Danger,
    /// The caution color: something to report or to look after.
    Caution,
    /// Flat until hovered, then the danger color.
    FlatDanger,
    /// A wash of the danger color with its text in it, full when hovered:
    /// a destructive action that stays in view among flat ones (Leave).
    SoftDanger,
}

/// Filled button with a Tabler icon before its label: every kind is a plain
/// face without a border, like the login button (the human's choice). A zero
/// width in `size` fits the content.
pub fn action_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, kind: ActionKind, size: Vec2) -> egui::Response {
    action_button_if(ui, p, icon, label, kind, size, true)
}

/// `action_button` greyed out when `enabled` is false. It stays in the
/// parent's layout: the child UI of `add_enabled_ui` would wrap on its own in
/// a wrapping row.
pub fn action_button_if(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    kind: ActionKind,
    size: Vec2,
    enabled: bool,
) -> egui::Response {
    action_button_keyed(ui, p, icon, label, kind, size, enabled, None)
}

const KEYED_ICON: f32 = 15.0;
const KEYED_GAP: f32 = 7.0;
/// Room around the content of a button that takes its content's width.
const KEYED_PAD: f32 = 24.0;

/// An `action_button_keyed`'s icon with its gap, label and keycaps widths.
fn keyed_parts(ui: &egui::Ui, icon: &str, label: &str, key: Option<&crate::keybinds::Binding>) -> (f32, f32, f32) {
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), Color32::PLACEHOLDER)
        .size()
        .x;
    // an icon alone (empty label) sits in the middle, without the gap
    let lead = match (icon.is_empty(), label.is_empty()) {
        (true, _) => 0.0,
        (false, true) => KEYED_ICON,
        (false, false) => KEYED_ICON + KEYED_GAP,
    };
    let caps = key.map_or(0.0, |b| KEYED_GAP + keycap::row_width(&keycap::caps(ui, b, Color32::PLACEHOLDER)));
    (lead, text, caps)
}

/// The width of an `action_button_keyed`'s content (icon, label, keys):
/// to give the button its own padding, or lay out what sits beside it
/// before it is drawn.
pub fn action_button_keyed_content(ui: &egui::Ui, icon: &str, label: &str, key: Option<&crate::keybinds::Binding>) -> f32 {
    let (lead, text, caps) = keyed_parts(ui, icon, label, key);
    lead + text + caps
}

/// `action_button_if` with the keycaps of its shortcut after the label
/// ("Stop flying F"); in a button too narrow for all three, the icon goes.
#[allow(clippy::too_many_arguments)]
pub fn action_button_keyed(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    kind: ActionKind,
    size: Vec2,
    enabled: bool,
    key: Option<&crate::keybinds::Binding>,
) -> egui::Response {
    const ICON: f32 = KEYED_ICON;
    const GAP: f32 = KEYED_GAP;
    let enabled = enabled && ui.is_enabled();
    let font = egui::FontId::proportional(12.5);
    let (lead, text_w, caps_w) = keyed_parts(ui, icon, label, key);
    let icon = if key.is_some() && size.x > 0.0 && lead + text_w + caps_w + 2.0 * GAP > size.x {
        ""
    } else {
        icon
    };
    let lead = if icon.is_empty() { 0.0 } else { lead };
    let content = lead + text_w + caps_w;
    let desired = Vec2::new(if size.x > 0.0 { size.x } else { content + KEYED_PAD }, size.y);
    let (rect, resp) = ui.allocate_exact_size(desired, if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && (resp.hovered() || resp.has_focus());
    let (fill, ink) = match kind {
        ActionKind::Primary => (if hot { p.violet_light } else { p.violet }, p.on_accent),
        ActionKind::Danger => (if hot { p.danger } else { p.danger.gamma_multiply(0.88) }, p.navy),
        ActionKind::Caution => (if hot { p.amber } else { p.amber.gamma_multiply(0.88) }, p.navy),
        ActionKind::FlatDanger if hot => (p.danger.gamma_multiply(0.88), p.navy),
        ActionKind::SoftDanger if hot => (p.danger.gamma_multiply(0.88), p.navy),
        ActionKind::SoftDanger => (p.danger.gamma_multiply(0.14), p.danger),
        ActionKind::Flat | ActionKind::FlatDanger => (if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised }, p.ink),
    };
    let fade = if enabled { 1.0 } else { 0.45 };
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(rect, radius, fill.gamma_multiply(fade));
    let ink = ink.gamma_multiply(fade);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let left = rect.center().x - content / 2.0;
    if let Some(texture) = super::icons::global(icon) {
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(egui::pos2(left + ICON / 2.0, rect.center().y), Vec2::splat(ICON)),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            ink,
        );
    }
    // centered on the line, not on the ink: a row of buttons shares a baseline
    let text = egui::pos2(left + lead, rect.center().y - galley.size().y / 2.0).round();
    let text_w = galley.size().x;
    ui.painter().galley(text, galley, ink);
    if let Some(b) = key {
        let behind = fill.gamma_multiply(fade);
        let caps = keycap::caps(ui, b, behind);
        keycap::paint_row(ui, p, &caps, egui::pos2(left + lead + text_w + GAP, rect.center().y), behind);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    resp
}

/// `action_button` that opens a menu: the chevron after its label says so,
/// and the face stays lit while the menu is `open`. Its height is `height`,
/// its width fits the content.
pub fn action_menu_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, open: bool, height: f32) -> egui::Response {
    const ICON: f32 = 15.0;
    const GAP: f32 = 7.0;
    const CHEVRON: f32 = 13.0;
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, p.ink);
    let lead = if label.is_empty() { ICON } else { ICON + GAP };
    let content = lead + galley.size().x + 5.0 + CHEVRON;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(content + 22.0, height), Sense::click());
    let hot = resp.hovered() || resp.has_focus();
    let fill = if open {
        p.violet.gamma_multiply(0.28)
    } else if hot {
        p.raised.lerp_to_gamma(p.ink, 0.16)
    } else {
        p.raised
    };
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(rect, radius, fill);
    if open {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let left = rect.center().x - content / 2.0;
    if let Some(texture) = super::icons::global(icon) {
        let r = egui::Rect::from_center_size(egui::pos2(left + ICON / 2.0, rect.center().y), Vec2::splat(ICON));
        ui.painter().image(texture.id(), r, uv, p.ink);
    }
    let width = galley.size().x;
    let text = egui::pos2(left + lead, rect.center().y - galley.size().y / 2.0).round();
    ui.painter().galley(text, galley, p.ink);
    if let Some(texture) = super::icons::global("chevron-down") {
        let x = left + lead + width + 5.0 + CHEVRON / 2.0;
        let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(CHEVRON));
        ui.painter().image(texture.id(), r, uv, p.ink);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp
}

/// A thin countdown bar (3 px): the track of `progress`, filled with
/// `color` from the left, gliding to its value like `progress`.
pub fn countdown_bar(ui: &mut egui::Ui, p: &Palette, id: &str, fraction: f32, width: f32, color: Color32) {
    let shown = ui.ctx().animate_value_with_time(ui.id().with(id), fraction.clamp(0.0, 1.0), 0.35);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 3.0), Sense::hover());
    ui.painter().rect_filled(rect, TRACK_RADIUS, p.raised);
    let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * shown, rect.height()));
    if fill.width() >= 0.5 {
        ui.painter().rect_filled(fill, TRACK_RADIUS, color);
    }
}

/// One button of `dialog_buttons`.
#[derive(Clone, Copy, Debug)]
pub struct DialogButton<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub kind: ActionKind,
    pub enabled: bool,
}

/// The width `action_button` gives `label` with `icon` when it fits its content.
pub fn action_button_fit(ui: &egui::Ui, icon: &str, label: &str) -> f32 {
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.5), Color32::PLACEHOLDER)
        .size()
        .x;
    let lead = match (icon.is_empty(), label.is_empty()) {
        (true, _) => 0.0,
        (false, true) => 15.0,
        (false, false) => 22.0,
    };
    lead + text + 24.0
}

/// The buttons that end a dialog, against the right edge in the order
/// given, all as wide as the widest of them (Cancel as wide as "Pay L$
/// 1,500"). Returns the index of the one clicked.
pub fn dialog_buttons(ui: &mut egui::Ui, p: &Palette, buttons: &[DialogButton<'_>], height: f32) -> Option<usize> {
    let width = buttons
        .iter()
        .map(|b| action_button_fit(ui, b.icon, b.label))
        .fold(0.0_f32, f32::max);
    let mut clicked = None;
    let row = Vec2::new(ui.available_width(), height);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        for (i, b) in buttons.iter().enumerate().rev() {
            if action_button_if(ui, p, b.icon, b.label, b.kind, Vec2::new(width, height), b.enabled).clicked() && b.enabled {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Individual frosted glass with the theme's tint, border, shadow and radius.
/// The native button retains input and accessibility without painting a face.
pub fn glass_action_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, size: Vec2, icon_color: Color32) -> egui::Response {
    let response = ui.add_sized(size, egui::Button::new("").frame(false));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label));
    if ui.is_rect_visible(response.rect) {
        let rect = response.rect;
        let pressed = response.is_pointer_button_down_on();
        let hot = response.hovered() || response.has_focus();
        let base = if pressed {
            p.indigo
        } else if hot {
            p.raised
        } else {
            p.panel
        };
        let alpha = if p.panel_alpha == 255 {
            255
        } else {
            (f32::from(p.panel_alpha)
                * if pressed {
                    0.88
                } else if hot {
                    0.72
                } else {
                    0.55
                }) as u8
        };
        let tint = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        let mut frame = overlay_frame(ui.ctx(), p);
        frame.fill = tint;
        frame.corner_radius = ui.visuals().widgets.inactive.corner_radius;
        frame.stroke = Stroke::new(
            1.0,
            if hot {
                p.violet_light.gamma_multiply(0.65)
            } else {
                p.ink.gamma_multiply(0.18)
            },
        );
        ui.painter().add(frame.paint(rect.shrink(frame.stroke.width)));
        if p.panel_alpha < 255 {
            ui.painter().add(aurora_render::glass_panel(
                rect.shrink(1.0),
                f32::from(frame.corner_radius.nw),
                tint,
            ));
        }
        if let Some(texture) = super::icons::global(icon) {
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(rect.center(), Vec2::splat(22.0)),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                if pressed { p.on_accent } else { icon_color },
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Primary action using the Aurora accent and the flat button geometry.
pub fn primary_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    ui.add_sized(
        size,
        egui::Button::new(RichText::new(label).size(12.0).strong().color(p.on_accent))
            .fill(p.violet)
            .stroke(Stroke::new(1.0, p.violet))
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .truncate(),
    )
    .on_hover_text(label)
}

/// Green online / grey offline.
pub fn online_color(p: &Palette, online: bool) -> Color32 {
    if online { p.success } else { p.muted_dim }
}

/// Region maturity badge (G / M / A), flat vector square in the Aurora style.
/// Letter, background and letter color of a classification label.
/// Fixed-meaning colors (kept from the official viewer): G violet, M white,
/// A red.
pub fn maturity_style(p: &Palette, maturity: &str) -> Option<(&'static str, Color32, Color32)> {
    // theme-guard: content — the white and the red of the official maturity badges
    const WHITE: Color32 = Color32::WHITE;
    const ADULT: Color32 = Color32::from_rgb(0xD9, 0x2D, 0x2D);
    match maturity {
        "" => None,
        value if value == aurora_i18n::text("common-general") => Some(("G", p.violet, WHITE)),
        value if value == aurora_i18n::text("common-moderate") => Some(("M", WHITE, p.navy)),
        value if value == aurora_i18n::text("common-adult") => Some(("A", ADULT, WHITE)),
        _ => None,
    }
}

pub fn maturity_badge(ui: &mut egui::Ui, p: &Palette, maturity: &str, size: f32) -> Option<egui::Response> {
    let (letter, ..) = maturity_style(p, maturity)?;
    let tip = match letter {
        "G" => aurora_i18n::tr!("ui-widgets-general-region-all-ages"),
        "M" => aurora_i18n::tr!("ui-widgets-moderate-region"),
        _ => aurora_i18n::tr!("ui-widgets-adult-region"),
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_maturity_badge(ui.painter(), p, maturity, rect, inner_radius(ui.visuals()));
    Some(resp.on_hover_text(tip))
}

/// The same badge painted in a given square (labels drawn on a map).
pub fn paint_maturity_badge(
    painter: &egui::Painter,
    p: &Palette,
    maturity: &str,
    rect: egui::Rect,
    radius: impl Into<egui::CornerRadius>,
) -> bool {
    let Some((letter, bg, fg)) = maturity_style(p, maturity) else {
        return false;
    };
    painter.rect_filled(rect, radius, bg);
    // centered on the letter itself, not on the text row
    let galley = painter.layout_no_wrap(letter.to_owned(), egui::FontId::proportional(rect.height() * 0.72), fg);
    paint_ink_centered(painter, galley, rect.center(), fg);
    true
}

/// The badge of a rating given as SIM_ACCESS_* (the navigation bar's).
pub fn maturity_badge_of(ui: &mut egui::Ui, p: &Palette, sim_access: u8, size: f32) -> Option<egui::Response> {
    maturity_badge(ui, p, super::bars::maturity_name(sim_access), size)
}

/// The badge of a region named by a place link, once its rating is known.
/// An unknown region is handed to `take_wanted_regions` (only the links
/// actually drawn are asked) and gets its badge when the answer arrives.
pub fn region_badge(
    ui: &mut egui::Ui,
    p: &Palette,
    ratings: &crate::world::worldmap::RegionRatings,
    region: &str,
    size: f32,
) -> Option<egui::Response> {
    maturity_badge_of(ui, p, region_rating(ui.ctx(), ratings, region)?, size)
}

/// A region's rating, for a badge drawn now; an unknown one is asked
/// (`take_wanted_regions`).
pub fn region_rating(ctx: &egui::Context, ratings: &crate::world::worldmap::RegionRatings, region: &str) -> Option<u8> {
    let r = ratings.get(region);
    if r.is_none() {
        want_region(ctx, region);
    }
    r
}

fn wanted_regions_id() -> egui::Id {
    egui::Id::new("aurora-wanted-region-ratings")
}

fn want_region(ctx: &egui::Context, region: &str) {
    if region.trim().is_empty() {
        return;
    }
    ctx.data_mut(|d| {
        let v = d.get_temp_mut_or_default::<Vec<String>>(wanted_regions_id());
        if !v.iter().any(|n| n == region) {
            v.push(region.to_owned());
        }
    });
}

/// The regions of the links drawn without a rating since the last call.
pub fn take_wanted_regions(ctx: &egui::Context) -> Vec<String> {
    ctx.data_mut(|d| d.remove_temp::<Vec<String>>(wanted_regions_id()))
        .unwrap_or_default()
}

/// The ratings a content preference includes, General first (the rating
/// icons next to Firestorm's maturity list, LLFloaterPreference::onChangeMaturity).
pub fn ratings_up_to(preferred: u8) -> Vec<u8> {
    use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};
    [SIM_ACCESS_PG, SIM_ACCESS_MATURE, SIM_ACCESS_ADULT]
        .into_iter()
        .filter(|r| *r <= preferred.max(SIM_ACCESS_PG))
        .collect()
}

/// Their badges, side by side.
pub fn maturity_badges_up_to(ui: &mut egui::Ui, p: &Palette, preferred: u8, size: f32) {
    ui.spacing_mut().item_spacing.x = 3.0;
    for r in ratings_up_to(preferred) {
        maturity_badge_of(ui, p, r, size);
    }
}

/// The rectangle the glyphs of `galley` cover, relative to its origin (its
/// layout rectangle when nothing is drawn).
pub fn ink_rect(galley: &egui::Galley) -> egui::Rect {
    let mut ink = egui::Rect::NOTHING;
    for row in &galley.rows {
        for g in &row.row.glyphs {
            if g.uv_rect.size.y <= 0.0 {
                continue;
            }
            let min = row.pos + g.pos.to_vec2() + g.uv_rect.offset;
            ink = ink.union(egui::Rect::from_min_size(min, g.uv_rect.size));
        }
    }
    if ink.is_positive() { ink } else { galley.rect }
}

/// Paint a galley so that its visible ink (not egui's text row, which
/// includes ascender / descender room) is centered on `center`: digits in
/// round badges then sit exactly in the middle.
pub fn paint_ink_centered(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, center: egui::Pos2, color: Color32) {
    let ink_center = ink_rect(&galley).center();
    // whole pixels keep the glyphs crisp
    let pos = (center - ink_center.to_vec2()).round();
    painter.galley(pos, galley, color);
}

/// A picture fitted in `size` keeping its aspect, or a placeholder (a
/// Tabler `icon` over a caption) while there is none.
pub fn picture(
    ui: &mut egui::Ui,
    p: &Palette,
    tex: Option<&egui::TextureHandle>,
    size: Vec2,
    icon: &str,
    placeholder: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            let aspect = w.max(1) as f32 / h.max(1) as f32;
            let fit = if aspect >= size.x / size.y {
                Vec2::new(size.x, size.x / aspect)
            } else {
                Vec2::new(size.y * aspect, size.y)
            };
            ui.painter().image(
                t.id(),
                egui::Rect::from_center_size(rect.center(), fit),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                PICTURE_TINT,
            );
        }
        None => {
            if let Some(t) = super::icons::global(icon) {
                let ir = egui::Rect::from_center_size(rect.center(), Vec2::splat(size.min_elem() * 0.45));
                ui.painter().image(
                    t.id(),
                    ir,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    p.muted_dim,
                );
            }
            ui.painter().text(
                rect.center_bottom() - Vec2::new(0.0, 12.0),
                egui::Align2::CENTER_CENTER,
                placeholder,
                egui::FontId::proportional(10.5),
                p.muted_dim,
            );
        }
    }
    ui.painter().rect_stroke(
        rect,
        ui.visuals().window_corner_radius,
        egui::Stroke::new(1.0, p.raised),
        egui::StrokeKind::Inside,
    );
    resp
}

/// First letters of the two first words of a name.
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

/// Part of a `w` x `h` picture a square shows: its center, the longer side
/// cut (profile pictures are often 4:3).
fn square_crop(w: usize, h: usize) -> egui::Rect {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (u, v) = if w > h { (h / w, 1.0) } else { (1.0, w / h) };
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(u, v))
}

/// `avatar_picture` in a disc: the residents of the Conversations window,
/// round like the pictures of the name plates (docs/BRANDING.md: only these
/// and the name plates are round).
pub fn avatar_disc(ui: &egui::Ui, p: &Palette, rect: egui::Rect, pic: Option<&egui::TextureHandle>, name: &str, dim: bool) {
    let radius = rect.width().min(rect.height()) * 0.5;
    match pic {
        Some(t) => {
            let [w, h] = t.size();
            let tint = if dim { PICTURE_TINT_DISABLED } else { PICTURE_TINT };
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
                .uv(square_crop(w, h))
                .tint(tint)
                .corner_radius(radius)
                .paint_at(ui, rect);
        }
        None => {
            ui.painter().circle_filled(rect.center(), radius, p.raised);
            let color = if dim { p.muted_dim } else { p.violet_pale };
            let font = egui::FontId::proportional(rect.height() * 0.42);
            ui.painter()
                .text(rect.center(), egui::Align2::CENTER_CENTER, initials(name), font, color);
        }
    }
}

/// Profile picture of a resident over the square `rect` of a list row or a
/// chat line, or the initials of `name` on a tile while there is none.
/// `dim` fades it with its row.
pub fn avatar_picture(ui: &egui::Ui, p: &Palette, rect: egui::Rect, pic: Option<&egui::TextureHandle>, name: &str, dim: bool) {
    match pic {
        Some(t) => {
            let [w, h] = t.size();
            let tint = if dim { PICTURE_TINT_DISABLED } else { PICTURE_TINT };
            ui.painter().image(t.id(), rect, square_crop(w, h), tint);
        }
        None => {
            ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.raised);
            let color = if dim { p.muted_dim } else { p.violet_pale };
            let font = egui::FontId::proportional(rect.height() * 0.45);
            ui.painter()
                .text(rect.center(), egui::Align2::CENTER_CENTER, initials(name), font, color);
        }
    }
}

/// A menu opened by a Tabler icon button, with the rows and the frame of the
/// right-click menus (`menu::button`).
pub fn icon_menu(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, content: impl FnOnce(&mut egui::Ui)) {
    let button = match super::icons::global(name) {
        Some(t) => {
            let img = egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.muted);
            egui::Button::image(img).frame(false)
        }
        None => egui::Button::new(tip),
    };
    super::menu::button(ui, p, button, content).on_hover_text(tip);
}

#[cfg(test)]
mod maturity_tests {
    use super::*;
    use aurora_net::search::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};

    #[test]
    fn a_preference_shows_the_ratings_it_includes() {
        assert_eq!(ratings_up_to(SIM_ACCESS_PG), [SIM_ACCESS_PG]);
        assert_eq!(ratings_up_to(SIM_ACCESS_MATURE), [SIM_ACCESS_PG, SIM_ACCESS_MATURE]);
        assert_eq!(
            ratings_up_to(SIM_ACCESS_ADULT),
            [SIM_ACCESS_PG, SIM_ACCESS_MATURE, SIM_ACCESS_ADULT]
        );
        // an unknown preference still shows General
        assert_eq!(ratings_up_to(0), [SIM_ACCESS_PG]);
        // each rating has the navigation bar's badge
        let p = crate::theme::Theme::default().palette();
        for (r, letter) in [(SIM_ACCESS_PG, "G"), (SIM_ACCESS_MATURE, "M"), (SIM_ACCESS_ADULT, "A")] {
            let (got, _, _) = maturity_style(&p, super::super::bars::maturity_name(r)).unwrap();
            assert_eq!(got, letter);
        }
    }
}
