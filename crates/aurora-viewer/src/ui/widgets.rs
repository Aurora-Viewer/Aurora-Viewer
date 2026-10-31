//! Firestorm-style building blocks: floaters with a slim title bar
//! (title, ?, —, ✕), flat tabs, column headers and status dots.

use crate::theme::Palette;
use egui::{Color32, CornerRadius, RichText, Sense, Stroke, Vec2};

#[cfg(test)]
pub(crate) mod alignment_tests;
mod cascade;
#[cfg(test)]
mod cascade_tests;
#[cfg(test)]
mod check_tests;
#[cfg(test)]
mod collapse_tests;
#[cfg(test)]
mod field_centering_tests;
#[cfg(test)]
mod movement_tests;
pub(crate) use cascade::Group as CascadeGroup;
pub mod color;
mod copy_on_click;
pub mod demo;
#[cfg(test)]
pub(crate) mod elide_tests;
pub(crate) mod hero;
pub(crate) mod keycap;
mod modal_keys;
pub(crate) mod pills;
mod resize;
pub(crate) mod scroll_offset;
#[cfg(test)]
pub(crate) mod steady_tests;
mod text_field;
mod unread;
mod volume;
mod workspace;
pub use color::color_button;
pub use copy_on_click::copy_on_click;
pub(crate) use modal_keys::{default_button, enter_pressed};
pub use pills::{
    clearable_search_box, dot_pill, dot_pill_width, filter_field, filter_pill, filter_pill_at, filter_pill_width, pill_tab, pill_tab_wide,
    search_box,
};
pub use text_field::text_field;
pub use unread::{CountSize, UnreadCount, unread_count};
pub use volume::{VolumeScale, perceptual_override, set_perceptual_volume, volume_slider, volume_slider_with};
pub use workspace::{
    Modal, caret_room, collapsing, collapsing_forced, disclosure_caret, experimental_mark, glyph, navigation, navigation_beside,
    navigation_wrapped, paint_glyph, section_header, sort_caret, stat, surface,
};
pub(crate) use workspace::{
    finish_bar_glass, inventory_item_label, inventory_item_label_tagged, inventory_spacing, island_shape, organization_drag_preview,
    organization_header, organization_section, organization_typography, overlay, overlay_frame, paint_world_label,
};

/// Whether UI owns a world-pointer event at `pos` (logical points). Resize
/// grips consume the press outside their window too, as LLResizeHandle and
/// LLResizeBar::handleMouseDown do, and egui retains the subsequent drag.
pub(crate) fn pointer_over_ui(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input() || resize::contains_pointer(ctx, pos)
}

/// Any floater/bare window is moving or resizing, independently of drawing
/// order. The move ID starts dragging only after the window's click tolerance,
/// so pointer jitter, child controls and text selection keep their behavior.
pub(crate) fn window_dragging(ctx: &egui::Context) -> bool {
    let Some(dragged) = ctx.dragged_id() else { return false };
    resize::active(ctx)
        || ctx
            .memory(|memory| memory.areas().visible_layer_ids())
            .into_iter()
            .any(|layer| layer.id.with("move") == dragged)
}

/// Wait for egui's click tolerance before moving a window. Area's drag-only
/// background otherwise starts moving even while a button above it is clicked.
/// Keep its move ID and hit-test plane: children and resize handles stay on top,
/// and layout's snapping and grouped movement retain the same drag lifecycle.
/// This adds the requested tolerance to LLDragHandle::handleHover's immediate
/// movement (indra/llui/lldraghandle.cpp).
fn window_click_tolerance(response: &egui::Response) {
    let _ = response.interact(Sense::CLICK | Sense::DRAG);
}

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
    let pos = row_text_origin(ui.painter(), &galley, left_center.x, left_center.y);
    let cut = galley.elided;
    ui.painter().galley(pos, galley, color);
    cut
}

/// From the middle of a name to the middle of the line under it, both
/// painted by hand around a row's middle: `base` at the usual text size,
/// more once a large text size makes the two lines taller, for the row to
/// grow by the difference instead of overlapping them (#1227).
pub(crate) fn two_lines_gap(ui: &egui::Ui, top: &egui::FontId, bottom: &egui::FontId, base: f32) -> f32 {
    ui.fonts_mut(|f| (f.row_height(top) + f.row_height(bottom)) / 2.0 + 1.0).max(base)
}

fn elided_galley(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let text = text.into();
    if !super::bidi::has_rtl(text.text()) {
        return text.into_galley(ui, Some(egui::TextWrapMode::Truncate), max_width.max(0.0), egui::TextStyle::Body);
    }
    // a right-to-left name (an Arabic display name in a message header) in
    // reading order: its first word on the right, cut at its left end
    let job = text.into_layout_job(ui.style(), egui::TextStyle::Body.into(), ui.text_valign());
    let mut cut = std::sync::Arc::unwrap_or_clone(job);
    let whole = cut.clone();
    cut.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    let mut galley = super::bidi::galley(ui.ctx(), cut, super::bidi::Base::Auto);
    if galley.elided {
        // the tooltip of a cut label shows its galley's job: as egui's own
        // cut galleys do, it holds the whole text (here in display order)
        let whole = super::bidi::galley(ui.ctx(), whole, super::bidi::Base::Auto);
        std::sync::Arc::make_mut(&mut galley).job = whole.job.clone();
    }
    galley
}

/// The bold names of LLFontGL's BOLD style (worn items, the current
/// outfit, friends): the interface font's semi-bold face, egui's strong
/// text only changing the color.
pub(crate) fn embolden(ui: &egui::Ui, format: &mut egui::text::TextFormat) {
    let semibold = super::fonts::semibold_format(&super::fonts::semibold(ui.ctx()), format.font_id.size, format.color);
    format.font_id = semibold.font_id;
    format.coords = semibold.coords;
}

/// The text format of a bold name.
pub(crate) fn bold_format(ui: &egui::Ui, font_id: egui::FontId, color: Color32) -> egui::text::TextFormat {
    let mut format = egui::text::TextFormat::simple(font_id, color);
    embolden(ui, &mut format);
    format
}

/// A static system font has no semi-bold face: its bold names are then
/// painted a second time this far to the right, as LLFontGL's BOLD_OFFSET
/// does. None when `embolden` gives a heavier face.
pub(crate) fn faux_bold_offset(ui: &egui::Ui) -> Option<Vec2> {
    if super::fonts::has_semibold(ui.ctx()) {
        return None;
    }
    let ppp = ui.ctx().pixels_per_point();
    Some(Vec2::new(ppp.round().max(1.0) / ppp, 0.0))
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

/// The position to give window `id` this frame: `asked` (by the layout, a
/// placement or a resize) or where it stood, with its title bar kept in
/// `area` (`layout::keep_title_in`) and, once no button is held, clear of
/// the bars' islands (`layout::clear_of_islands`): a window saved in a
/// larger screen or another layout comes back within reach. None when it
/// stands well and nothing asks for it elsewhere.
fn title_in_reach(
    ctx: &egui::Context,
    id: egui::Id,
    area: egui::Rect,
    asked: Option<egui::Pos2>,
    sized: Option<Vec2>,
) -> Option<egui::Pos2> {
    use super::layout::{TITLE_BAR_H, TITLE_GRIP, clear_of_islands, edges_for, islands, keep_title_in};
    let state = egui::AreaState::load(ctx, id).filter(|s| s.size.is_some());
    // `sized`: the size it takes this frame (an edge dragged)
    let size = sized
        .or_else(|| state.as_ref().and_then(|s| s.size))
        .unwrap_or(egui::vec2(TITLE_GRIP, TITLE_BAR_H));
    let pos = asked.or_else(|| state.as_ref().map(|s| s.left_top_pos()))?;
    let edges = edges_for(ctx, id);
    let mut kept = keep_title_in(area, pos, size, edges);
    if !ctx.input(|i| i.pointer.any_down()) {
        kept = clear_of_islands(area, kept, size, edges, &islands(ctx));
    }
    (asked.is_some() || kept != pos).then_some(kept)
}

/// Two compact increment buttons beside numeric fields (LLSpinCtrl).
/// Uses theme faces and Tabler glyphs, and inherits disabled state.
pub(crate) fn step_buttons(ui: &mut egui::Ui, height: f32) -> i8 {
    let mut step = 0;
    ui.allocate_ui_with_layout(Vec2::new(13.0, height), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_max_width(13.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        for (direction, name, label) in [
            (1, "caret-up-filled", aurora_i18n::tr!("numeric-increase")),
            (-1, "caret-down-filled", aurora_i18n::tr!("numeric-decrease")),
        ] {
            let (rect, response) = ui.allocate_exact_size(Vec2::new(13.0, height * 0.5), Sense::click());
            let visuals = ui.style().interact(&response);
            ui.painter().rect_filled(rect, visuals.corner_radius, visuals.bg_fill);
            if let Some(icon) = super::icons::global(name) {
                ui.painter().image(
                    icon.id(),
                    egui::Rect::from_center_size(rect.center(), Vec2::splat(9.0)),
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    visuals.fg_stroke.color,
                );
            }
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
            if response.on_hover_text(label).clicked() {
                step = direction;
            }
        }
    });
    step
}

pub struct Floater<'a> {
    pub id: &'a str,
    pub title: String,
    /// Optional compact state row within the title bar: glyph, label, theme role.
    pub header_status: Vec<(&'static str, String, Color32)>,
    /// A text action at the right end of the secondary header row.
    pub header_status_action: Option<(&'a str, &'a str, &'a str, &'a mut bool)>,
    /// A short muted text right after the title (a count of what the
    /// window holds).
    pub title_note: Option<String>,
    /// A state at the right end of the title bar, before its buttons: a
    /// dot of its colour and its text in muted.
    pub title_state: Option<(Color32, String)>,
    pub default_pos: egui::Pos2,
    pub default_size: Vec2,
    pub min_size: Vec2,
    pub resizable: bool,
    pub help: Option<&'a str>,
    /// Extra title bar buttons: (Tabler icon, tooltip, set when clicked),
    /// the first one nearest "Collapse".
    pub actions: Vec<(&'a str, &'a str, &'a mut bool)>,
    /// No open / close sounds (sound_flags="0", e.g. floater_tools).
    pub silent: bool,
    /// An inset dark title bar and edge-to-edge body for inspector layouts.
    pub framed_header: Option<&'a str>,
    /// Keep an inspector's width stable while its height follows the content.
    pub auto_height: bool,
    /// Resized in height only: its width is the one it opens with
    /// (Performance's full view, lined up under the system island).
    pub height_only: bool,
    /// Put a toolbar directly beneath the title, without a separator layout row.
    pub compact_header: bool,
    /// Something new waits inside (unread messages): the minimized bar shows
    /// it, as Firestorm flashes the conversations button.
    pub attention: bool,
    /// The violet laid over the minimized bar while `attention` (its
    /// strength, and whether it moves: a flash under way).
    pub attention_flash: Option<(f32, bool)>,
    /// Minimized, the window keeps a compact body under its title bar
    /// instead of the bar alone, at this width: the body is still drawn and
    /// asks `floater_minimized` what to show.
    pub compact_minimized: Option<f32>,
    /// The body reaches the window's edges under its title bar (the
    /// Conversations workspace: its column and panes are the window).
    pub flush_body: bool,
    /// Once shown, only its edges resize it: its body is laid out in its
    /// exact rect and cut at its edges (see `steady_size`).
    pub steady_size: bool,
    /// Resized by its edges, its body keeps this width over height, the
    /// window adding the second value around it (see `keep_aspect`).
    pub aspect: Option<(f32, Vec2)>,
    /// Its layer: the windows' own (`Middle`), or `Foreground` over an
    /// overlay whose controls sit there (the photo viewfinder's poser).
    pub order: egui::Order,
    cascade: Option<CascadeGroup>,
    partial_outside: bool,
    /// A chat window: it stays in immersive mode (`hide_windows`).
    chat: bool,
}

impl<'a> Floater<'a> {
    pub fn new(id: &'a str, title: impl Into<String>, pos: egui::Pos2, size: Vec2) -> Floater<'a> {
        Floater {
            id,
            title: title.into(),
            header_status: Vec::new(),
            header_status_action: None,
            title_note: None,
            title_state: None,
            default_pos: pos,
            default_size: size,
            min_size: Vec2::new(220.0, 120.0),
            resizable: true,
            height_only: false,
            help: None,
            actions: Vec::new(),
            silent: false,
            framed_header: None,
            auto_height: false,
            compact_header: false,
            attention: false,
            attention_flash: None,
            compact_minimized: None,
            flush_body: false,
            steady_size: false,
            aspect: None,
            order: egui::Order::Middle,
            cascade: None,
            partial_outside: false,
            chat: false,
        }
    }

    pub fn fixed(mut self) -> Self {
        self.resizable = false;
        self
    }

    /// Let this window cross the side/bottom stops while its title stays
    /// reachable, as LLFloaterView::adjustToFitScreen(allow_partial_outside).
    pub(crate) fn allow_partial_outside(mut self) -> Self {
        self.partial_outside = true;
        self
    }

    /// Position new inventory windows like Firestorm's same-type floaters.
    pub(crate) fn cascade(mut self, group: CascadeGroup) -> Self {
        self.cascade = Some(group);
        self
    }

    pub fn help(mut self, text: &'a str) -> Self {
        self.help = Some(text);
        self
    }

    /// A muted text after the title (see `title_note`).
    pub fn title_note(mut self, note: impl Into<String>) -> Self {
        self.title_note = Some(note.into());
        self
    }

    /// A state before the title buttons (see `title_state`).
    pub fn title_state(mut self, dot: Color32, text: impl Into<String>) -> Self {
        self.title_state = Some((dot, text.into()));
        self
    }

    /// Add a title bar button left of "Collapse" (and of the buttons added
    /// before it); `clicked` is set when pressed.
    pub fn action(mut self, icon: &'a str, tip: &'a str, clicked: &'a mut bool) -> Self {
        self.actions.push((icon, tip, clicked));
        self
    }

    pub fn silent(mut self) -> Self {
        self.silent = true;
        self
    }

    /// A chat window: immersive mode keeps it (#859).
    pub fn chat(mut self) -> Self {
        self.chat = true;
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

    /// The body edge to edge under the title bar (see `flush_body`).
    pub fn flush_body(mut self) -> Self {
        self.flush_body = true;
        self
    }

    /// A resizable window whose content changes all the time (the
    /// Conversations window): egui's frame follows what its content
    /// measured, and its Resize keeps the widest it saw, so a long message
    /// or a tab opened or closed that measured a fraction of a point too
    /// wide made it grow a pixel or two at a time, for good. Once shown, its
    /// body is laid out in its exact rect instead, cut at its edges.
    pub fn steady_size(mut self) -> Self {
        self.steady_size = true;
        self
    }

    /// Resized by its edges, the body keeps `ratio` (width over height) at
    /// every frame of the drag, `chrome` being what the window adds around
    /// it (its title bar): a video player never stretches (#941).
    pub fn keep_aspect(mut self, ratio: f32, chrome: Vec2) -> Self {
        self.aspect = Some((ratio, chrome));
        self
    }

    /// Mark the minimized bar while `waiting` (unread messages inside).
    pub fn attention(mut self, waiting: bool) -> Self {
        self.attention = waiting;
        self
    }

    /// Tint the minimized bar's mark with `flash` (the violet's strength,
    /// and whether it moves), as `ButtonFlash::bar` gives it.
    pub fn attention_flash(mut self, flash: (f32, bool)) -> Self {
        self.attention_flash = Some(flash);
        self
    }

    /// Show the floater; `open` is cleared by the close button.
    pub fn show<R>(mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
        super::layout::set_partial_outside(ctx, egui::Id::unique(self.id), self.partial_outside);
        if window_hidden(ctx, egui::Id::unique(self.id), *open, self.chat) {
            if self.cascade.is_some() {
                cascade::hidden(ctx, egui::Id::unique(self.id));
            }
            // still open: no close sound, nor an open one when it comes back
            if !self.silent {
                super::sound_cues::floater_shown(ctx, egui::Id::unique(self.id));
            }
            return None;
        }
        if !*open {
            if self.cascade.is_some() {
                cascade::hidden(ctx, egui::Id::unique(self.id));
            }
            return None;
        }
        let min_id = egui::Id::unique((self.id, "minimized"));
        // persisted with the UI layout (ui_layout.ron)
        let minimized = ctx.data_mut(|d| d.get_persisted::<bool>(min_id)).unwrap_or(false);
        // minimized with its compact body: sized by that body
        let compact = minimized && self.compact_minimized.is_some();
        if !self.silent {
            super::sound_cues::floater_shown(ctx, egui::Id::unique(self.id));
        }
        let frame = egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(
                p.panel.r(),
                p.panel.g(),
                p.panel.b(),
                p.panel_alpha,
            ))
            .stroke(border_stroke(p, Stroke::new(1.0, p.muted_dim.gamma_multiply(0.6))))
            .corner_radius(ctx.global_style().visuals.window_corner_radius)
            .shadow(ctx.global_style().visuals.window_shadow)
            .inner_margin(if self.framed_header.is_some() {
                egui::Margin::ZERO
            } else if self.flush_body {
                egui::Margin {
                    left: 0,
                    right: 0,
                    top: if minimized && !compact { 4 } else { 5 },
                    bottom: if minimized && !compact { 4 } else { 0 },
                }
            } else {
                egui::Margin {
                    left: 10,
                    right: 10,
                    top: if minimized && !compact { 4 } else { 5 },
                    bottom: if minimized && !compact { 4 } else { 10 },
                }
            });
        // Ctrl+W closes it as its cross does, when it is the frontmost
        let mut close = super::close_key::take(ctx, egui::Id::unique(self.id));
        super::close_key::shown(ctx, egui::Id::unique(self.id));
        // the toolbar button, a menu entry or a shortcut asked to expand it
        let expand_asked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(self.id).with(EXPAND)));
        let mut toggle_min = minimized && expand_asked.unwrap_or(false);
        let mut out = None;
        let window_id = egui::Id::unique(self.id);
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
        let opening = self
            .cascade
            .and_then(|group| cascade::prepare(ctx, window_id, group, egui::Rect::from_min_size(self.default_pos, expanded), work));
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
            .order(self.order)
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
        } else if let Some(size) = opening.as_ref().and_then(|o| o.size).filter(|_| !self.auto_height) {
            w = w.fixed_size(size);
        } else if restore && !self.auto_height {
            w = w.fixed_size(expanded.min(work.size()));
        } else if let Some(size) = shrink {
            w = w.fixed_size(size);
        } else if self.height_only
            && !minimized
            && let Some(size) = egui::AreaState::load(ctx, window_id).and_then(|s| s.size)
        {
            // its width held at every frame: egui's first layout of a new
            // window could otherwise widen it to what it measured
            w = w.fixed_size(Vec2::new(self.default_size.x, size.y));
        }
        // the viewer window changed size: the floater follows its edges;
        // a placement asked this frame and a resize in progress come after
        let slot = super::layout::slot(ctx, window_id);
        if opening.is_some() {
            super::layout::reanchor(ctx, window_id);
        }
        let mut asked = opening.as_ref().map(|o| o.pos).or(slot.pos);
        // A window moves anywhere in the room the bars leave it
        // (`layout::move_area`: the whole screen with the islands, between
        // the Legacy Viewer's bars otherwise), past the edges the user lets
        // it cross, its title bar within reach (`layout::reach`); a window
        // dragged with its group has the group's limits.
        let room = super::layout::move_area(ctx);
        // the edge dragged snaps to the other windows and to the work area
        let resized = resize_drag.map(|drag| {
            let rect = on_pixels(
                ctx,
                super::layout::resizing(ctx, p, window_id, drag.rect(), drag.edges(), drag.minimum()),
            );
            // a window of fixed shape: the snapped rect brought back to it
            match self.aspect {
                Some((ratio, chrome)) => on_pixels(ctx, drag.keep_aspect(rect, ratio, chrome)),
                None => rect,
            }
        });
        let size = egui::AreaState::load(ctx, window_id)
            .and_then(|s| s.size)
            .unwrap_or(self.default_size);
        let edges = super::layout::edges_for(ctx, window_id);
        // While an edge is dragged the drag keeps the window in its bounds
        // and its title in reach (`title_in_reach` below): egui would
        // constrain the asked position with the last frame's size, and a
        // quick drag of the left or top edge pushed the whole window back
        // by what that size exceeded, its opposite edge moving for a frame
        // (the human's report of 2026-10-16).
        w = if resized.is_some() {
            w.constrain(false)
        } else {
            w.constrain_to(slot.limits.unwrap_or_else(|| super::layout::reach(room, size, edges)))
        };
        if let Some(pos) = ctx.data_mut(|d| d.remove_temp::<egui::Pos2>(window_id.with(PLACE_POS))) {
            asked = Some(pos);
            if self.cascade.is_some() {
                cascade::placed(ctx, window_id);
            }
        }
        if let Some(rect) = resized {
            w = w.fixed_size(rect.size());
            asked = Some(rect.min);
        }
        // Whatever places the window, its title bar stays in reach: egui
        // lets a window larger than its limits slide out of them by what it
        // has in excess, title first.
        if let Some(pos) = title_in_reach(ctx, window_id, room, asked, resized.map(|r| r.size())) {
            w = w.current_pos(pos);
        }
        // `steady_size`: once shown, the body in the window's exact rect
        let held = self.steady_size && resizable && egui::AreaState::load(ctx, window_id).is_some_and(|s| s.size.is_some());
        let mut glass_slot = None;
        let response = w.show(ctx, |outer| {
            glass_slot = Some((outer.painter().clone(), outer.painter().add(egui::Shape::Noop)));
            let held = held.then(|| outer.available_rect_before_wrap());
            let mut held_ui = held.map(|rect| {
                let mut child = outer.new_child(egui::UiBuilder::new().max_rect(rect).layout(*outer.layout()));
                child.set_clip_rect(rect.intersect(outer.clip_rect()));
                child
            });
            let ui: &mut egui::Ui = match held_ui.as_mut() {
                Some(child) => child,
                None => &mut *outer,
            };
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
                let title_row = ui.horizontal(|ui| {
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
                        // a window with a compact body when minimized says
                        // so with the Performance window's compact / full
                        // icons rather than a collapse (#1035)
                        let (icon, tip) = match (self.compact_minimized.is_some(), minimized) {
                            (true, true) => ("arrows-maximize", aurora_i18n::tr!("common-full-view")),
                            (true, false) => ("arrows-minimize", aurora_i18n::tr!("common-compact-view")),
                            (false, true) => ("minus", aurora_i18n::tr!("common-expand")),
                            (false, false) => ("minus", aurora_i18n::tr!("common-collapse")),
                        };
                        if title_button(ui, p, icon).on_hover_text(tip).clicked() {
                            toggle_min = true;
                        }
                        for (icon, tip, clicked) in &mut self.actions {
                            let response = title_button(ui, p, icon).on_hover_text(*tip);
                            #[cfg(test)]
                            ui.ctx()
                                .data_mut(|d| d.insert_temp(egui::Id::unique(("floater_action", self.id, *icon)), response.rect));
                            if response.clicked() {
                                **clicked = true;
                            }
                        }
                        if let Some(h) = self.help {
                            title_button(ui, p, "help-circle-filled").on_hover_text(h);
                        }
                        if let Some((dot, text)) = &self.title_state {
                            ui.add_space(4.0);
                            ui.label(RichText::new(text).size(12.0).color(p.muted));
                            ui.spacing_mut().item_spacing.x = 0.0;
                            status_dot(ui, *dot);
                            ui.spacing_mut().item_spacing.x = 6.0;
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
                                if let Some(note) = &self.title_note {
                                    ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.muted)).truncate());
                                }
                            });
                        }
                    });
                });
                if !self.header_status.is_empty() || self.header_status_action.is_some() {
                    let status = |ui: &mut egui::Ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(7.0, 2.0);
                            for (name, label, color) in &self.header_status {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    glyph(ui, name, *color, 11.0);
                                    ui.label(RichText::new(label).size(11.0).color(*color));
                                });
                            }
                        });
                    };
                    if let Some((icon, label, tip, clicked)) = &mut self.header_status_action {
                        ui.horizontal(|ui| {
                            let width = (ui.available_width() - 65.0).max(1.0);
                            ui.allocate_ui_with_layout(Vec2::new(width, 18.0), egui::Layout::top_down(egui::Align::Min), status);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add(egui::Label::new(RichText::new(*label).size(12.0).color(p.violet_light)).sense(Sense::click()))
                                    .on_hover_text(*tip)
                                    .clicked()
                                {
                                    **clicked = true;
                                }
                                glyph(ui, icon, p.violet_light, 13.0);
                            });
                        });
                    } else {
                        status(ui);
                    }
                }
                title_row
            };
            let flush = self.flush_body;
            let header = if flush {
                // the title keeps the margins the body gives up
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 0))
                    .show(ui, &mut header_content)
                    .response
            } else if framed_header.is_some() {
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
                if flush {
                    ui.add_space(5.0);
                } else if framed_header.is_none() {
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
            if let Some(rect) = held {
                outer.advance_cursor_after_rect(rect);
            }
        });
        if let Some(response) = response {
            window_click_tolerance(&response.response);
            if content_sized {
                let too_tall = if scrolls {
                    still_too_tall
                } else {
                    response.response.rect.height() > work.height() + 0.5
                };
                ctx.data_mut(|d| d.insert_temp(scroll_id, too_tall));
            }
            slot.shown(ctx, response.response.rect, minimized);
            if self.cascade.is_some() {
                cascade::shown(ctx, window_id, response.response.rect, minimized);
            }
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
                // the bar lights up in the accent while something waits
                // inside, tinted and flashing like the Conversations button
                // when asked for, its outline widening with the violet
                let painter = ctx.layer_painter(response.response.layer_id);
                let radius = ctx.global_style().visuals.window_corner_radius;
                let (wash, moving) = self.attention_flash.unwrap_or((0.0, false));
                if moving {
                    ctx.request_repaint();
                }
                attention_mark(&painter, p, response.response.rect, radius, wash);
            }
            if let (Some(drag), Some(requested)) = (resize_drag, resized) {
                drag.finish(ctx, window_id, requested, response.response.rect);
            }
            if resizable {
                resize::handles_of(ctx, p, &response.response, self.height_only);
            }
        }
        if close {
            *open = false;
            // LLFloater::closeFloater: "Always unminimize before trying to
            // close", the window opens again expanded
            unminimize_quietly(ctx, self.id);
            if self.cascade.is_some() {
                cascade::hidden(ctx, window_id);
            }
        } else if toggle_min {
            ctx.data_mut(|d| d.insert_persisted(min_id, !minimized));
            if self.cascade.is_some() {
                cascade::minimized(ctx, window_id, !minimized);
            }
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
/// moved with Alt or resized, and is resized by the same edge handles.
/// `locked`: the user can neither move nor resize it (it still follows the
/// viewer's edges and the group it is glued to).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bare_window(
    ctx: &egui::Context,
    p: &Palette,
    id: &str,
    title: &str,
    frame: egui::Frame,
    default: egui::Rect,
    min_size: Vec2,
    locked: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let window_id = egui::Id::unique(id);
    if window_hidden(ctx, window_id, true, false) {
        return;
    }
    let drag = if locked {
        None
    } else {
        resize::Drag::prepare(ctx, window_id, min_size)
    };
    let mut w = egui::Window::new(title)
        .id(window_id)
        .title_bar(false)
        .frame(frame)
        .fade_in(false)
        .fade_out(false)
        .default_pos(default.min)
        .default_size(default.size())
        .min_size(min_size)
        .movable(!locked)
        .resizable(false)
        .constrain(true);
    let slot = super::layout::slot(ctx, window_id);
    if let Some(pos) = slot.pos {
        w = w.current_pos(pos);
    }
    if let Some(limits) = slot.limits {
        w = w.constrain_to(limits);
    }
    let resized = drag.map(|drag| {
        on_pixels(
            ctx,
            super::layout::resizing(ctx, p, window_id, drag.rect(), drag.edges(), drag.minimum()),
        )
    });
    if let Some(rect) = resized {
        // (no constraint from the last frame's size: see `Floater::show`)
        w = w.fixed_size(rect.size()).current_pos(rect.min).constrain(false);
    }
    let Some(shown) = w.show(ctx, |ui| {
        // as a resizable egui::Window would: the body gets the whole size
        ui.set_min_size(ui.available_size());
        body(ui);
    }) else {
        return;
    };
    // locked: no drag on the move ID at all, or the layout would carry the
    // windows glued to it while it stays put
    if !locked {
        window_click_tolerance(&shown.response);
    }
    slot.shown(ctx, shown.response.rect, false);
    if let (Some(drag), Some(requested)) = (drag, resized) {
        drag.finish(ctx, window_id, requested, shown.response.rect);
    }
    if !locked {
        resize::handles(ctx, p, &shown.response);
    }
}

/// A resized window's rect on whole physical pixels. The pointer moves by
/// fractions of a point (a scaled screen, a sub-pixel drag): egui rounds the
/// window's position but not its size, so its bottom, and the messages laid
/// against it, fell between pixels and hopped by one as the edge moved (the
/// human's report of 2026-10-16).
fn on_pixels(ctx: &egui::Context, rect: egui::Rect) -> egui::Rect {
    let ppp = ctx.pixels_per_point();
    let round = |v: f32| (v * ppp).round() / ppp;
    egui::Rect::from_min_max(
        egui::pos2(round(rect.min.x), round(rect.min.y)),
        egui::pos2(round(rect.max.x), round(rect.max.y)),
    )
}

const PLACE_POS: &str = "place_pos";

/// The windows the hidden interface keeps out of sight (Alt+Shift+U,
/// LLFloaterView::hideAllFloaters / showHiddenFloaters): those open on the
/// frame it was hidden. A window opened afterwards shows; one closed in the
/// meantime is forgotten. Immersive mode (#859) keeps the chat windows.
#[derive(Clone, Default)]
struct HiddenWindows {
    /// The frame whose open windows are hidden.
    capture: Option<u64>,
    ids: std::collections::HashSet<egui::Id>,
    /// The chat windows stay (immersive mode).
    keep_chat: bool,
}

fn hidden_windows_id() -> egui::Id {
    egui::Id::unique("aurora_hidden_windows")
}

/// The interface is hidden (`hide_windows`).
pub fn interface_hidden(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<HiddenWindows>(hidden_windows_id()).is_some())
}

/// Hide the windows open now (on the next frame), or show them back; with
/// `keep_chat`, the chat windows stay. Called between two frames.
pub fn hide_windows(ctx: &egui::Context, hide: bool, keep_chat: bool) {
    let state = HiddenWindows {
        capture: hide.then(|| ctx.cumulative_frame_nr()),
        ids: Default::default(),
        keep_chat,
    };
    ctx.data_mut(|d| {
        if hide {
            d.insert_temp(hidden_windows_id(), state);
        } else {
            d.remove::<HiddenWindows>(hidden_windows_id());
        }
    });
}

/// The window `id` (`open` or not, a `chat` window or not) is kept hidden
/// this frame.
fn window_hidden(ctx: &egui::Context, id: egui::Id, open: bool, chat: bool) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let Some(mut state) = d.get_temp::<HiddenWindows>(hidden_windows_id()) else {
            return false;
        };
        if chat && state.keep_chat {
            return false;
        }
        let hidden = if !open {
            state.ids.remove(&id);
            false
        } else if state.capture == Some(frame) {
            state.ids.insert(id);
            true
        } else {
            state.ids.contains(&id)
        };
        d.insert_temp(hidden_windows_id(), state);
        hidden
    })
}

/// Movable toolbar without a common face. The body supplies its individual
/// button surfaces and close action. Its default point is positioned after
/// measurement; subsequent placement and movement remain shared.
pub(crate) fn toolstrip<R>(
    ctx: &egui::Context,
    id: &str,
    point: egui::Align2,
    pos: egui::Pos2,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let window_id = egui::Id::unique(id);
    super::sound_cues::floater_shown(ctx, window_id);
    let slot = super::layout::slot(ctx, window_id).default_anchor(ctx, point, pos);
    let mut area = egui::Area::new(window_id)
        .order(egui::Order::Middle)
        .fade_in(false)
        .movable(true)
        .default_pos(pos)
        .constrain_to(slot.limits.unwrap_or_else(|| super::layout::move_area(ctx)));
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

/// What is being dragged, drawn by `add` under the pointer above every
/// window, at `pos`, with a window's shadow under it (the favourites of the
/// bars, a conversation's entry). It takes no pointer: the drop is read
/// under it.
pub(crate) fn drag_ghost(ctx: &egui::Context, p: &Palette, id: &str, pos: egui::Pos2, add: impl FnOnce(&mut egui::Ui)) {
    egui::Area::new(egui::Id::unique(id))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .fade_in(false)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            let shadow = ui.painter().add(egui::Shape::Noop);
            add(ui);
            let shape = egui::epaint::Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: p.navy.gamma_multiply(0.5),
            }
            .as_shape(ui.min_rect(), ui.visuals().window_corner_radius);
            ui.painter().set(shadow, shape);
        });
}

/// Move a dependent floater without changing its size or minimized state.
pub fn move_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2) {
    super::layout::reanchor(ctx, egui::Id::unique(id));
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(id).with(PLACE_POS), pos));
}

/// Give the floater `id` this position and size on its next frame, expanded,
/// as if the user had moved and resized it: a demo scenario then shows the
/// same thing whatever layout the previous run left in ui_layout.ron.
pub fn place_floater(ctx: &egui::Context, id: &str, pos: egui::Pos2, size: Vec2) {
    let window_id = egui::Id::unique(id);
    super::layout::reanchor(ctx, window_id);
    ctx.data_mut(|d| {
        d.insert_persisted(egui::Id::unique((id, "minimized")), false);
        d.insert_persisted(window_id.with("expanded_size"), size);
        d.insert_temp(window_id.with("restore_size"), true);
        d.insert_temp(window_id.with(PLACE_POS), pos);
    });
}

/// Split what is left of `ui` between a list and the footer under it (its
/// buttons, a note): the list gets a child exactly as tall as the room the
/// footer leaves, and `Footer::show` lays the footer out under it. A list
/// of `available_height()` minus a guessed footer height made the window
/// taller at every frame once the footer measured more (a larger text, a
/// wrapped label, a message): egui's Resize grew to the body, the list took
/// the new room, and the body overflowed again. The list is sized with the
/// height the footer measured last, and the pass is laid out again when
/// that height changes; `ui` takes the whole rest and nothing more,
/// whatever the list and the footer measure.
pub fn above_footer(ui: &mut egui::Ui) -> (egui::Ui, Footer) {
    let rest = ui.available_rect_before_wrap();
    let id = ui.scope_id().with("above_footer");
    let height = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
    let top = rest.bottom() - height;
    let gap = if height > 0.0 { ui.spacing().item_spacing.y } else { 0.0 };
    let above = egui::Rect::from_min_max(rest.min, egui::pos2(rest.right(), (top - gap).max(rest.top())));
    // the room is taken without a widget: one over the footer would take
    // its buttons' pointer
    ui.advance_cursor_after_rect(rest);
    let mut list = ui.new_child(egui::UiBuilder::new().max_rect(above).layout(*ui.layout()));
    list.set_clip_rect(above.intersect(ui.clip_rect()));
    (list, Footer { id, rest, top, height })
}

/// The footer's place under a list (see `above_footer`).
pub struct Footer {
    id: egui::Id,
    rest: egui::Rect,
    top: f32,
    height: f32,
}

impl Footer {
    /// Lay the footer out, top down, at the bottom of the rest. A footer
    /// taller than the whole rest keeps its bottom (its buttons) in view,
    /// cut at the top. It is laid out with no height to fill, so that a
    /// row filling what it is given (a centered row) measures what it
    /// holds, not the room it was given.
    pub fn show(self, ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
        let rest = self.rest;
        let mut foot = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_min_max(
                    egui::pos2(rest.left(), self.top),
                    egui::pos2(rest.right(), self.top),
                ))
                .layout(*ui.layout()),
        );
        foot.set_clip_rect(rest.intersect(ui.clip_rect()));
        add(&mut foot);
        let measured = foot.min_rect().height();
        if (measured - self.height).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(self.id, measured));
            ui.ctx().request_discard("footer height");
        }
    }
}

/// Set an expanded window's size once, preserving its independent position.
pub fn size_floater(ctx: &egui::Context, id: &str, size: Vec2) {
    let window_id = egui::Id::unique(id);
    ctx.data_mut(|d| {
        d.insert_persisted(window_id.with("expanded_size"), size);
        d.insert_temp(window_id.with("restore_size"), true);
    });
}

/// Consume the actual size after a manual edge/corner resize is released.
/// Layout placement, minimization and automatic screen fitting do not emit it.
pub fn take_floater_resize(ctx: &egui::Context, id: &str) -> Option<Vec2> {
    ctx.data_mut(|d| d.remove_temp::<Vec2>(egui::Id::unique(id).with("floater_resized")))
}

/// Minimize the floater `id` (LLFloater::setMinimized), as its title button
/// does.
pub fn minimize_floater(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((id, "minimized")), true));
    cascade::minimized(ctx, egui::Id::unique(id), true);
}

/// The accent that says something waits behind `rect`: a violet wash of
/// strength `wash` (`ButtonFlash`) and a light outline that widens with it.
/// A minimized conversations bar and the Conversations button wear the same
/// one, so both flash alike.
pub fn attention_mark(painter: &egui::Painter, p: &Palette, rect: egui::Rect, radius: CornerRadius, wash: f32) {
    if wash > 0.0 {
        painter.rect_filled(rect, radius, p.violet.gamma_multiply(wash));
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.5 + 4.0 * wash, p.violet_light),
        egui::StrokeKind::Inside,
    );
}

/// Is the floater `id` shown in front of the floater `other`? A window not
/// shown is behind every other, whatever its layer (the poser's goes up
/// over the photo viewfinder).
pub fn floater_above(ctx: &egui::Context, id: &str, other: &str) -> bool {
    let (id, other) = (egui::Id::unique(id), egui::Id::unique(other));
    ctx.memory(|m| {
        let place = |want: egui::Id| m.layer_ids().filter(|l| m.areas().is_visible(l)).position(|l| l.id == want);
        place(id) > place(other)
    })
}

/// Is the floater `id` reduced to its title bar?
pub fn floater_minimized(ctx: &egui::Context, id: &str) -> bool {
    ctx.data_mut(|d| d.get_persisted::<bool>(egui::Id::unique((id, "minimized"))))
        .unwrap_or(false)
}

/// Expand the minimized floater `id` on its next frame and bring it to the
/// front, as its title button does (LLFloater::openFloater on a minimized
/// floater: setMinimized(false), setVisibleAndFrontmost).
pub fn expand_floater(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(id).with(EXPAND), true));
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)));
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
        cascade::forget(ctx, egui::Id::unique(id));
        *open = true;
        unminimize_quietly(ctx, id);
    }
}

/// Forget that a floater not on screen was minimized: it opens expanded, at
/// its expanded size, without the sound of the title button.
fn unminimize_quietly(ctx: &egui::Context, id: &str) {
    if floater_minimized(ctx, id) {
        let window_id = egui::Id::unique(id);
        ctx.data_mut(|d| {
            d.insert_persisted(egui::Id::unique((id, "minimized")), false);
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

fn title_button_sized(ui: &mut egui::Ui, p: &Palette, icon: &str, size: Vec2) -> egui::Response {
    let (_, resp) = ui.allocate_exact_size(size, Sense::click());
    paint_title_button(ui, p, icon, &resp);
    resp
}

fn paint_title_button(ui: &egui::Ui, p: &Palette, icon: &str, resp: &egui::Response) {
    let rect = resp.rect;
    let col = if resp.hovered() { p.ink } else { p.muted };
    let c = rect.center();
    let stroke = Stroke::new(1.4, col);
    let painter = ui.painter();
    if let Some(t) = super::icons::global(icon) {
        let r = egui::Rect::from_center_size(c, Vec2::splat(14.0));
        painter.image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), col);
        return;
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
}

/// The scroll bars of every scroll area, set once by `Theme::apply`: thin and
/// solid, in a strip of their own at the right of the content, like
/// Firestorm's LLScrollContainer, whose visible width loses the bar's
/// (indra/llui/llscrollcontainer.cpp, calcVisibleSize). egui's default
/// floating bar is drawn over the content when hovered, over the buttons and
/// values at the right edge. The strip is taken only when the content
/// overflows. 6 points of bar, 4 between it and the content, faintly shown
/// at rest. The content stops where the drag that scrolls it leaves it, at
/// the edge of the list (egui 0.37 lets it coast on and spring back past the
/// edge). `ui/theme_guard.rs` keeps windows from setting another style.
pub fn reserved_scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        dormant_background_opacity: 0.2,
        dormant_handle_opacity: 0.6,
        kinetic: egui::style::KineticScrollStyle {
            decay_time: 0.0,
            rubber_band: false,
            ..Default::default()
        },
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
/// hardly lighter than the panels (a light one in high contrast).
pub fn divider_stroke(p: &Palette) -> Stroke {
    border_stroke(p, Stroke::new(1.0, p.raised))
}

/// The border of a window, panel, island or divider: `usual`, or in high
/// contrast (`crate::high_contrast`) a light line that stands out 3:1 from
/// the black panels.
pub fn border_stroke(p: &Palette, usual: Stroke) -> Stroke {
    if p.high_contrast {
        Stroke::new(usual.width.max(1.0), p.muted_dim)
    } else {
        usual
    }
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
        let text = row_text_origin(ui.painter(), &galley, mark.right() + ui.spacing().icon_spacing, row.center().y);
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
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), checked, text)
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
        egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, text)
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

/// Custom-painted controls follow an explicit font override in their UI scope.
fn control_font(ui: &egui::Ui, default_size: f32) -> egui::FontId {
    ui.style()
        .override_font_id
        .clone()
        .unwrap_or_else(|| egui::FontId::proportional(default_size))
}

/// Flat tab strip. Returns true when the selection changed.
pub fn tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, false, false)
}

/// The same tab strip at the density of fixed inspector controls.
pub fn compact_tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, false, true)
}

/// Equal-width tabs spanning a data inspector, with the usual theme and input handling.
pub fn stretched_tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, true, false)
}

fn tabs_row(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)], stretched: bool, compact: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let n = labels.len().max(1) as f32;
        let font = control_font(ui, 12.0);
        let minimum_height = if compact { 22.0 } else { 30.0 };
        let height = if ui.style().override_font_id.is_some() {
            (ui.fonts_mut(|f| f.row_height(&font)) + if compact { 8.0 } else { 12.0 }).max(minimum_height)
        } else {
            minimum_height
        };
        let text_w: Vec<f32> = labels
            .iter()
            .map(|(l, _)| ui.fonts_mut(|f| f.layout_no_wrap((*l).to_owned(), font.clone(), Color32::PLACEHOLDER).size().x) + 18.0)
            .collect();
        let gaps = (n - 1.0) * ui.spacing().item_spacing.x;
        let total: f32 = text_w.iter().sum::<f32>() + gaps;
        let compact = ((ui.available_width() - gaps) / (total - gaps)).clamp(0.0, 1.0);
        let extra = ((ui.available_width() - total) / n).max(0.0);
        let even_width = ((ui.available_width() - gaps) / n).max(0.0);
        for (i, (label, enabled)) in labels.iter().enumerate() {
            let w = if stretched {
                even_width
            } else {
                text_w[i] * compact + extra.min(40.0)
            };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, height), if *enabled { Sense::click() } else { Sense::hover() });
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
            let left = rect.center().x - galley.size().x / 2.0;
            paint_row_text(ui.painter(), galley, left, rect.center().y, col);
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
        let resp = ui.interact(rect, ui.scope_id().with(("segmented", i, *label)), Sense::click());
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
        let at = letters_in_middle(ui.painter(), &galley, rect);
        ui.painter().galley(at, galley, text);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, sel, *label));
        if resp.clicked() && !sel {
            *selected = i;
            changed = true;
        }
    }
    changed
}

/// Where to paint a one-line galley so that its letters, from the baseline
/// up to the capitals, sit in the middle of `rect` — not its whole line box,
/// whose room for the descenders leaves a word without any ("Chuchoter",
/// "Dire") visibly high in a short segment. The capitals as drawn
/// (`centered_baseline`): taking them as three quarters of the ascent held
/// for Inter only (Noto Sans and Segoe UI: two thirds, a pixel high).
fn letters_in_middle(painter: &egui::Painter, galley: &egui::Galley, rect: egui::Rect) -> egui::Pos2 {
    let left = rect.center().x - galley.size().x / 2.0;
    let Some(font) = galley.job.sections.first().map(|s| s.format.font_id.clone()) else {
        return egui::pos2(left, rect.center().y - galley.size().y / 2.0);
    };
    let baseline = centered_baseline(painter, font, rect.center().y);
    baseline_origin(painter, galley, left, baseline)
}

/// Tabs sharing one track as wide as the space left, each with its count
/// after its label (Aurora's Places, the money tracker): the chosen one is
/// lit in `raised`. Returns true when the selection changed.
pub fn count_tabs(ui: &mut egui::Ui, p: &Palette, id_salt: &str, selected: &mut usize, tabs: &[(&str, usize)]) -> bool {
    let (bar, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(bar, radius, p.field);
    let n = tabs.len().max(1) as f32;
    let tab_w = (bar.width() - 4.0 - 2.0 * (n - 1.0)) / n;
    let id = egui::Id::unique(id_salt);
    let line = |s: &str, size: f32, strong: bool, width: f32| {
        let family = if strong {
            super::fonts::semibold(ui.ctx())
        } else {
            egui::FontFamily::Proportional
        };
        let mut job = egui::text::LayoutJob::single_section(
            s.to_owned(),
            egui::TextFormat::simple(egui::FontId::new(size, family), Color32::PLACEHOLDER),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: width.max(1.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        ui.painter().layout_job(job)
    };
    let mut changed = false;
    for (i, (label, count)) in tabs.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            egui::pos2(bar.left() + 2.0 + i as f32 * (tab_w + 2.0), bar.top() + 2.0),
            Vec2::new(tab_w, bar.height() - 4.0),
        );
        let resp = ui.interact(r, id.with(i), Sense::click());
        let on = *selected == i;
        if on {
            ui.painter().rect_filled(r, radius, p.raised);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, radius, p.raised.gamma_multiply(0.5));
        }
        let ink = if on { p.ink } else { p.muted };
        let name = line(label, 12.5, on, tab_w - 30.0);
        let num = line(&count.to_string(), 11.0, false, 30.0);
        let (nw, total) = (name.size().x, name.size().x + 6.0 + num.size().x);
        let x = r.center().x - total * 0.5;
        paint_row_text(ui.painter(), name, x, r.center().y, ink);
        paint_row_text(ui.painter(), num, x + nw + 6.0, r.center().y, p.muted);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, *label));
        if resp.clicked() && !on {
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

/// Flat icon toggle with a larger glyph and hit target, in logical points.
pub fn icon_toggle_sized(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool, side: f32) -> bool {
    sized_icon_button(ui, p, icon, tip, true, on, side)
}

fn small_icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool, on: bool) -> bool {
    sized_icon_button(ui, p, icon, tip, enabled, on, 20.0)
}

fn sized_icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool, on: bool, side: f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), if enabled { Sense::click() } else { Sense::hover() });
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
                egui::Rect::from_center_size(rect.center(), Vec2::splat(side * 0.7)),
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

/// Paint `galley` from `left_center`, its letters centred on the icon
/// beside it (`row_text_origin`; a font's line box sits lower than its
/// letters), its ink from `left_center.x`. A line used to centre its own
/// ink: a name with descenders ("Shopping") then stood a pixel or two
/// above one without ("Home") in the same row of favourites.
pub fn paint_ink_left(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, left_center: egui::Pos2, color: Color32) {
    if galley.rows.len() != 1 {
        let center = left_center + Vec2::new(galley.size().x / 2.0, 0.0);
        paint_ink_centered(painter, galley, center, color);
        return;
    }
    // the ink's left edge where the whole ink's centring put it
    let left = left_center.x + galley.size().x / 2.0 - ink_rect(&galley).center().x;
    paint_row_text(painter, galley, left, left_center.y, color);
}

/// Equal-sized dialog buttons; long script labels keep their full hover text.
pub fn flat_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    text_button(ui, p, label, Some(size)).on_hover_text(label)
}

fn text_button(ui: &mut egui::Ui, p: &Palette, label: &str, size: Option<Vec2>) -> egui::Response {
    // Keep the button in the parent's layout so wrapped toolbars can break
    // before it. A child UI would wrap internally and enlarge narrow windows.
    // The theme gives every button its plain filled face.
    let button = egui::Button::new(RichText::new(label).font(control_font(ui, 12.0)).color(p.ink));
    match size {
        Some(size) => ui.add_sized(size, button.truncate()),
        None => {
            let height = if ui.style().override_font_id.is_some() {
                ui.spacing().interact_size.y
            } else {
                20.0
            };
            ui.add(button.min_size(Vec2::new(0.0, height)))
        }
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
    let shown = ui
        .ctx()
        .animate_value_with_time(ui.scope_id().with(id), fraction.clamp(0.0, 1.0), 0.45);
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
    let shown = ui
        .ctx()
        .animate_value_with_time(ui.scope_id().with(id), value.clamp(0.0, 1.0), 0.08);
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
pub(crate) const KEYED_PAD: f32 = 24.0;

/// An `action_button_keyed`'s icon with its gap, label and keycaps widths.
fn keyed_parts(ui: &egui::Ui, icon: &str, label: &str, key: Option<&crate::keybinds::Binding>) -> (f32, f32, f32) {
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), control_font(ui, 12.5), Color32::PLACEHOLDER)
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
    let font = control_font(ui, 12.5);
    let (lead, text_w, caps_w) = keyed_parts(ui, icon, label, key);
    let icon = if key.is_some() && size.x > 0.0 && lead + text_w + caps_w + 2.0 * GAP > size.x {
        ""
    } else {
        icon
    };
    let lead = if icon.is_empty() { 0.0 } else { lead };
    let content = lead + text_w + caps_w;
    let desired = Vec2::new(if size.x > 0.0 { size.x } else { content + KEYED_PAD }, size.y);
    let (rect, mut resp) = ui.allocate_exact_size(desired, if enabled { Sense::click() } else { Sense::hover() });
    match kind {
        ActionKind::Primary => modal_keys::offer(&mut resp, modal_keys::Answer::Primary),
        ActionKind::Danger => modal_keys::offer(&mut resp, modal_keys::Answer::Danger),
        _ => {}
    }
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
    // high contrast: the dark faces get the buttons' light border, white
    // when hovered (the accent's and the states' faces stand out already)
    if p.high_contrast && matches!(kind, ActionKind::Flat | ActionKind::FlatDanger | ActionKind::SoftDanger) {
        let edge = if hot { p.ink } else { p.muted_dim };
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(1.0, edge.gamma_multiply(fade)), egui::StrokeKind::Inside);
    }
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
    // centred like the icon (`row_text_origin`): a row of buttons of one
    // height shares a baseline
    let text = row_text_origin(ui.painter(), &galley, left + lead, rect.center().y);
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
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
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
    let text = row_text_origin(ui.painter(), &galley, left + lead, rect.center().y);
    ui.painter().galley(text, galley, p.ink);
    if let Some(texture) = super::icons::global("chevron-down") {
        let x = left + lead + width + 5.0 + CHEVRON / 2.0;
        let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(CHEVRON));
        ui.painter().image(texture.id(), r, uv, p.ink);
    }
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    resp
}

/// A thin countdown bar (3 px): the track of `progress`, filled with
/// `color` from the left, gliding to its value like `progress`.
pub fn countdown_bar(ui: &mut egui::Ui, p: &Palette, id: &str, fraction: f32, width: f32, color: Color32) {
    let shown = ui
        .ctx()
        .animate_value_with_time(ui.scope_id().with(id), fraction.clamp(0.0, 1.0), 0.35);
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
        .layout_no_wrap(label.to_owned(), control_font(ui, 12.5), Color32::PLACEHOLDER)
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
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
    let mut response = ui.add_sized(
        size,
        egui::Button::new(RichText::new(label).size(12.0).strong().color(p.on_accent))
            .fill(p.violet)
            .stroke(Stroke::new(1.0, p.violet))
            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
            .truncate(),
    );
    modal_keys::offer(&mut response, modal_keys::Answer::Primary);
    response.on_hover_text(label)
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

fn maturity_tip(letter: &str) -> &'static str {
    match letter {
        "G" => aurora_i18n::tr!("ui-widgets-general-region-all-ages"),
        "M" => aurora_i18n::tr!("ui-widgets-moderate-region"),
        _ => aurora_i18n::tr!("ui-widgets-adult-region"),
    }
}

pub fn maturity_badge(ui: &mut egui::Ui, p: &Palette, maturity: &str, size: f32) -> Option<egui::Response> {
    let (letter, ..) = maturity_style(p, maturity)?;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_maturity_badge(ui.painter(), p, maturity, rect, inner_radius(ui.visuals()));
    Some(resp.on_hover_text(maturity_tip(letter)))
}

/// The badge inside a line of text of `size` (`emoji::text_rows`): it takes
/// a text row's height and its square is centred on the capitals of that
/// text as epaint draws them. Centred on egui's row instead, whose middle
/// sits under the letters (room for the descenders), the square stood up
/// to 1.5 px lower than the place name beside it.
pub fn maturity_badge_in_text(ui: &mut egui::Ui, p: &Palette, maturity: &str, size: f32) -> Option<egui::Response> {
    use egui::emath::GuiRounding as _;
    let (letter, ..) = maturity_style(p, maturity)?;
    let font = egui::FontId::proportional(size);
    let row = ui.fonts_mut(|f| f.row_height(&font));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(size, row), Sense::hover());
    // the capitals of a label of this text, laid out as egui lays a label
    // out (its rows sit apart from a bare layout's), drawn from the top of
    // its row on the pixel
    let probe = egui::WidgetText::from(egui::RichText::new("H").size(size)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let caps = ink_rect(&probe);
    let painter = ui.painter();
    let top = rect.left_top().round_to_pixels(painter.pixels_per_point()).y;
    let square = egui::Rect::from_center_size(egui::pos2(rect.center().x, top + caps.center().y), Vec2::splat(size));
    paint_maturity_badge(painter, p, maturity, square, inner_radius(ui.visuals()));
    Some(resp.on_hover_text(maturity_tip(letter)))
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
    // the square where epaint draws it (a 15 px badge centred on a whole
    // pixel covers the next one down), the letter centred on that
    let rect = {
        use egui::emath::GuiRounding as _;
        rect.round_to_pixels(painter.pixels_per_point())
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

/// The badge of a region named by a place link in a line of text
/// (`maturity_badge_in_text`: the chat, the notifications, the script
/// dialogs), once its rating is known. An unknown region is handed to
/// `take_wanted_regions` (only the links actually drawn are asked) and gets
/// its badge when the answer arrives.
pub fn region_badge_in_text(
    ui: &mut egui::Ui,
    p: &Palette,
    ratings: &crate::world::worldmap::RegionRatings,
    region: &str,
    size: f32,
) -> Option<egui::Response> {
    let rating = region_rating(ui.ctx(), ratings, region)?;
    maturity_badge_in_text(ui, p, super::bars::maturity_name(rating), size)
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
    egui::Id::unique("aurora-wanted-region-ratings")
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

/// `v` (points) on the physical pixel grid, as epaint rounds each glyph's
/// corner (`tessellate_glyphs`), a galley's position and a filled
/// rectangle: half a pixel goes down and right.
fn snap(v: f32, pixels_per_point: f32) -> f32 {
    (v * pixels_per_point).round() / pixels_per_point
}

/// The rectangle the glyphs of `galley` cover where epaint draws them,
/// relative to its origin (its
/// layout rectangle when nothing is drawn). Each glyph's corner goes to the
/// pixel there: the face's vertical shift (`fonts::optical_center_offset`,
/// 0.59 px for Inter at 13 px, 0.98 for Noto Sans) is drawn rounded, and
/// the ink of the layout alone stood up to half a pixel from the screen's.
pub fn ink_rect(galley: &egui::Galley) -> egui::Rect {
    let ppp = galley.pixels_per_point;
    let mut ink = egui::Rect::NOTHING;
    for row in &galley.rows {
        for g in &row.row.glyphs {
            if g.uv_rect.size.y <= 0.0 {
                continue;
            }
            let corner = g.pos + g.uv_rect.offset;
            let min = row.pos + Vec2::new(snap(corner.x, ppp), snap(corner.y, ppp));
            ink = ink.union(egui::Rect::from_min_size(min, g.uv_rect.size));
        }
    }
    if ink.is_positive() { ink } else { galley.rect }
}

/// Paint a galley so that its visible ink (not egui's text row, which
/// includes ascender / descender room) is centered on `center`: digits in
/// round badges then sit exactly in the middle.
pub fn paint_ink_centered(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, center: egui::Pos2, color: Color32) {
    use egui::emath::GuiRounding as _;
    let ink_center = ink_rect(&galley).center();
    // on the physical pixel, where epaint draws it (rounding in points left
    // a fraction of a pixel at the scales above 1)
    let pos = (center - ink_center.to_vec2()).round_to_pixels(painter.pixels_per_point());
    painter.galley(pos, galley, color);
}

/// The baseline of `galley`'s first line where its letters stand on
/// screen, from its origin: the same for any text of one font and size (a
/// glyph from a fallback font sits on it too), unlike its ink. egui's
/// layout baseline, moved by the face's vertical shift as epaint draws it
/// (`drawn_shift`): two fonts whose shifts round differently (Inter at 13
/// and at 10 px) stood a pixel apart on one layout baseline.
pub fn galley_baseline(painter: &egui::Painter, galley: &egui::Galley) -> f32 {
    let Some(row) = galley.rows.first() else {
        return galley.size().y * 0.8;
    };
    // a glyph of the font's own face: one taken from a fallback face (other
    // scripts, symbols) sits on that face's baseline, lower or higher
    let own = row.row.glyphs.iter().find(|g| (g.font_face_ascent - g.font_ascent).abs() < 0.01);
    let Some(g) = own.or(row.row.glyphs.first()) else {
        return galley.size().y * 0.8;
    };
    // the font of the first section: the texts set on a baseline have one
    let font = galley.job.sections.first().map(|s| s.format.font_id.clone());
    row.pos.y + g.pos.y + font.map_or(0.0, |f| drawn_shift(painter, f))
}

/// How far below egui's layout baseline the flat letters of `font` stand
/// on screen: the foot of its "H" as drawn (`ink_rect`).
fn drawn_shift(painter: &egui::Painter, font: egui::FontId) -> f32 {
    let probe = painter.layout_no_wrap("H".into(), font, Color32::PLACEHOLDER);
    let Some((row, g)) = probe.rows.first().and_then(|row| row.row.glyphs.first().map(|g| (row, g))) else {
        return 0.0;
    };
    if g.uv_rect.size.y <= 0.0 {
        return 0.0;
    }
    ink_rect(&probe).bottom() - (row.pos.y + g.pos.y)
}

/// The baseline (where the letters stand, `galley_baseline`) that centres
/// the capitals of `font` on `center_y`, on the pixel: texts side by side
/// placed on it (`paint_on_baseline`) line up whatever their letters.
/// Centring each text on its own ink set a name without descenders higher
/// than its neighbour with commas and brackets.
pub fn centered_baseline(painter: &egui::Painter, font: egui::FontId, center_y: f32) -> f32 {
    let probe = painter.layout_no_wrap("H".into(), font, Color32::PLACEHOLDER);
    // the capitals as drawn: the shift of the face is in both ends
    let cap = ink_rect(&probe).height().max(0.0);
    snap(center_y + cap / 2.0, painter.pixels_per_point())
}

/// Where to paint `galley` so that its first line stands on `baseline`, its
/// left edge at `left`, on the pixel.
pub fn baseline_origin(painter: &egui::Painter, galley: &egui::Galley, left: f32, baseline: f32) -> egui::Pos2 {
    use egui::emath::GuiRounding as _;
    egui::Pos2::new(left, baseline - galley_baseline(painter, galley)).round_to_pixels(painter.pixels_per_point())
}

/// Paint `galley` from `left` with its first line on `baseline`.
pub fn paint_on_baseline(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, left: f32, baseline: f32, color: Color32) {
    let pos = baseline_origin(painter, &galley, left, baseline);
    painter.galley(pos, galley, color);
}

/// Where to paint `galley` from `left` so that its letters are centred on
/// `center_y` like an icon or a box beside them, on the pixel: the middle
/// between the middle of its capitals and the middle of its x-height (what
/// `fonts::optical_center_offset` aims egui's text rows at), as epaint
/// draws it. Centring egui's text row instead put the letters up to 1.9 px
/// from there, with every font: the face's shift and the row's position
/// are each rounded on their own (`ink_rect`), the position in points at
/// the scales 1.25 and 1.5. A text of several lines keeps its block
/// centred.
pub fn row_text_origin(painter: &egui::Painter, galley: &egui::Galley, left: f32, center_y: f32) -> egui::Pos2 {
    use egui::emath::GuiRounding as _;
    let ppp = painter.pixels_per_point();
    let font = galley.job.sections.first().map(|s| s.format.font_id.clone());
    let middle = match font {
        Some(font) if galley.rows.len() == 1 => {
            let height = |s: &str| ink_rect(&painter.layout_no_wrap(s.into(), font.clone(), Color32::PLACEHOLDER)).height();
            galley_baseline(painter, galley) - (height("H") + height("x")) / 4.0
        }
        _ => galley.size().y / 2.0,
    };
    egui::Pos2::new(left, center_y - middle).round_to_pixels(ppp)
}

/// Paint `galley` from `left`, its letters centred on `center_y`
/// (`row_text_origin`).
pub fn paint_row_text(painter: &egui::Painter, galley: std::sync::Arc<egui::Galley>, left: f32, center_y: f32, color: Color32) {
    let pos = row_text_origin(painter, &galley, left, center_y);
    painter.galley(pos, galley, color);
}

/// The margin that puts the capitals of a text field in `font` in the middle
/// of the field (the human's choice of 2026-10-22): egui centres the text's
/// row, whose room for descenders leaves the letters high (Inter at 13
/// points: 1.1 points). What it adds above it takes below, so the field keeps
/// its height. Every field gets it: the theme's (`theme::FieldSize`) and the
/// bare ones laid on their own face (`pills::icon_field`…).
pub fn field_text_margin(ctx: &egui::Context, font: &egui::FontId) -> egui::Margin {
    let probe = ctx.fonts_mut(|f| f.layout_no_wrap("H".into(), font.clone(), Color32::PLACEHOLDER));
    let ink = ink_rect(&probe);
    let lower = ((probe.size().y - ink.bottom() - ink.top()) / 2.0).round().clamp(0.0, 4.0) as i8;
    egui::Margin {
        top: lower,
        bottom: -lower,
        ..egui::Margin::ZERO
    }
}

const ROW_TAG_PAD: f32 = 6.0;
/// The room on each side of a tag laid in a text (`reserve_row_tag`).
const ROW_TAG_GAP: f32 = 6.0;

/// Leave room in `job` for a row tag reading `text`, before what is
/// appended next; returns where the room ends, for
/// `inventory_item_label_tagged`.
pub fn reserve_row_tag(ui: &egui::Ui, job: &mut egui::text::LayoutJob, text: &str, p: &Palette) -> usize {
    let at = job.text.chars().count();
    let room = ROW_TAG_GAP + row_tag_width(ui, text);
    job.append(" ", room, egui::TextFormat::simple(egui::FontId::proportional(12.0), p.muted));
    at
}

/// The width of a row's tag (`paint_row_tag`).
pub fn row_tag_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_job(super::sheet::semibold(ui, text, 11.0, Color32::PLACEHOLDER))
        .size()
        .x
        + ROW_TAG_PAD * 2.0
}

/// Text colour shared by row tags and the names that follow their state.
pub fn row_tag_ink(p: &Palette, active: bool) -> Color32 {
    if active { p.teal } else { p.muted }
}

/// A small tag after an item's name, the same in every list: teal when
/// `active` (worn, an active gesture), neutral for a plain fact (a link).
/// Returns its width.
pub fn paint_row_tag(ui: &egui::Ui, p: &Palette, text: &str, active: bool, left: f32, center_y: f32) -> f32 {
    let fill = if active { p.teal.gamma_multiply(0.14) } else { p.raised };
    let ink = row_tag_ink(p, active);
    let galley = ui.painter().layout_job(super::sheet::semibold(ui, text, 11.0, ink));
    let rect = egui::Rect::from_min_size(
        egui::pos2(left, center_y - 9.0),
        Vec2::new(galley.size().x + ROW_TAG_PAD * 2.0, 18.0),
    );
    ui.painter().rect_filled(rect, inner_radius(ui.visuals()), fill);
    paint_row_text(ui.painter(), galley, rect.left() + ROW_TAG_PAD, rect.center().y, ink);
    rect.width()
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

/// A picture cropped square over `rect`, its corners the theme's (a
/// group's insignia in the Conversations window).
pub fn square_picture(ui: &egui::Ui, rect: egui::Rect, t: &egui::TextureHandle) {
    let [w, h] = t.size();
    egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), rect.size()))
        .uv(square_crop(w, h))
        .tint(PICTURE_TINT)
        .corner_radius(ui.visuals().window_corner_radius)
        .paint_at(ui, rect);
}

/// The initials stand for a picture: they keep to its tile, `share` of its
/// height, whatever the text size (which scales every font).
fn initials_font(ui: &egui::Ui, rect: egui::Rect, share: f32) -> egui::FontId {
    egui::FontId::proportional(rect.height() * share / super::fonts::scale(ui.ctx()))
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
            let font = initials_font(ui, rect, 0.42);
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
            let font = initials_font(ui, rect, 0.45);
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
