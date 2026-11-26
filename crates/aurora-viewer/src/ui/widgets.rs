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
#[cfg(test)]
mod outside_tests;
pub(crate) use cascade::Group as CascadeGroup;
#[cfg(test)]
mod auto_height_tests;
pub mod color;
mod copy_on_click;
pub mod demo;
#[cfg(test)]
pub(crate) mod elide_tests;
pub(crate) mod hero;
pub(crate) mod keycap;
pub(crate) mod local_texture;
mod modal_keys;
pub(crate) mod pills;
pub(crate) mod reorder;
mod resize;
mod restricted_text;
#[cfg(test)]
mod restricted_text_tests;
#[cfg(test)]
mod solid_background_tests;
pub use crate::text_validation::Policy as TextPolicy;
pub use restricted_text::field as restricted_text_field;
pub use restricted_text::number as restricted_number;
pub use restricted_text::optional as text_field_with_policy;
pub use restricted_text::show as restricted_text_edit;
pub(crate) mod scroll_offset;
#[cfg(test)]
pub(crate) mod steady_tests;
mod text_field;
mod tracks;
mod unread;
mod volume;
mod workspace;
pub use color::color_button;
pub use copy_on_click::{copied_feedback, copy_on_click, show_copied};
pub(crate) use modal_keys::{default_button, enter_pressed};
pub use pills::{
    chip_row, clearable_search_box, dot_pill, dot_pill_width, filter_field, filter_pill, filter_pill_at, filter_pill_width, pill_tab,
    pill_tab_wide, reveal_search_selection, search_box, search_cleared, search_list_item, search_scroll_to_column, search_scroll_to_row,
    search_text_edit, search_text_edit_with_margin,
};
pub use text_field::text_field;
pub use tracks::{
    TRACK_INSET, TrackFace, TrackLit, button_track, help_tag, icon_track, labelled_choice, paint_tile_button, track_button,
    wide_button_track,
};
pub use unread::{CountSize, UnreadCount, unread_count};
pub use volume::{VolumeScale, perceptual_override, set_perceptual_volume, volume_slider, volume_slider_with};
pub use workspace::{
    FORM_LABEL_W, FORM_SLIDER_W, Modal, ModalGround, caret_room, collapsing, collapsing_forced, disclosure_caret, experimental_mark,
    form_row, glyph, navigation, navigation_beside, navigation_wrapped, paint_glyph, section_header, sort_caret, stat, surface,
};
pub(crate) use workspace::{
    INVENTORY_LABEL_PADDING, finish_bar_glass, inventory_item_label, inventory_item_label_tagged, inventory_spacing, island, island_shape,
    organization_drag_preview, organization_drop_target, organization_header, organization_section, organization_source,
    organization_typography, overlay, overlay_frame, paint_world_label,
};

/// Whether UI owns a world-pointer event at `pos` (logical points). Resize
/// grips consume the press outside their window too, as LLResizeHandle and
/// LLResizeBar::handleMouseDown do, and egui retains the subsequent drag.
pub(crate) fn pointer_over_ui(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    ctx.is_pointer_over_egui() || ctx.egui_wants_pointer_input() || resize::contains_pointer(ctx, pos) || solid_background_at(ctx, pos)
}

/// The background areas that are panels of their own, by layer id.
/// The body of a window of automatic height (`Floater::auto_height`) that
/// scrolls once it is taller than `room`: then exactly `room` tall. Without
/// the floor, a body taller than the room took the height egui's window
/// had last frame, so a window squeezed once (the viewer's window made tiny,
/// a tester's report of 2026-11-17) kept its small height for good.
pub(crate) fn auto_height_body(room: f32) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .max_height(room)
        .min_scrolled_height(room)
        .auto_shrink([false, true])
}

#[derive(Clone, Default)]
struct SolidAreas(std::collections::HashSet<egui::Id>);

fn solid_areas_id() -> egui::Id {
    egui::Id::unique("aurora_solid_background_areas")
}

/// The background area `id` (`Order::Background`, kept under the windows) is
/// a panel of its own, the Director's timeline or a keyframe of it in the
/// world: a press on it is its own, never the world's. egui leaves a press
/// on such an area to the world (`is_pointer_over_egui` only counts a
/// background area outside the root's free room), and the world reads the
/// press before egui's pass has seen it (`egui_wants_pointer_input` comes
/// too late): without this, a click on the strip between its buttons walked
/// the avatar there or stood it up.
pub(crate) fn solid_background(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<SolidAreas>(solid_areas_id()).0.insert(id);
    });
}

/// The topmost area at `pos` (shown last frame, interactable, a window above
/// it first) is a solid background area (`solid_background`).
fn solid_background_at(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    let Some(layer) = ctx.memory(|m| m.layer_id_at(pos)) else {
        return false;
    };
    layer.order == egui::Order::Background
        && ctx.data_mut(|d| d.get_temp_mut_or_default::<SolidAreas>(solid_areas_id()).0.contains(&layer.id))
}

/// A floater edge or corner grip contains `pos` (LLResizeBar / LLResizeHandle).
pub(crate) fn pointer_on_floater_edge(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    resize::contains_pointer(ctx, pos)
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

pub(crate) fn elided_galley(ui: &egui::Ui, text: impl Into<egui::WidgetText>, max_width: f32) -> std::sync::Arc<egui::Galley> {
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

/// A floater's inner margin around its body (neither `flush_body` nor
/// `framed_header`, not minimized), and the width of its outline.
const FLOATER_MARGIN: egui::Margin = egui::Margin {
    left: 10,
    right: 10,
    top: 5,
    bottom: 10,
};
const FLOATER_STROKE: f32 = 1.0;
/// The title row's least height, its title's size, and the room left
/// between the divider under it and the body.
const TITLE_ROW_MIN: f32 = 28.0;
const TITLE_SIZE: f32 = 14.0;
const UNDER_DIVIDER: f32 = 5.0;
/// What a floater has above and below its body besides its title: the
/// margins of its frame and its outline.
const BODY_CHROME: f32 = FLOATER_MARGIN.top as f32 + FLOATER_MARGIN.bottom as f32 + 2.0 * FLOATER_STROKE;
/// The least height the scrolled body of a floater keeps.
const MIN_SCROLLED_BODY: f32 = 80.0;
/// The outer height egui gives a window it opens without a default height
/// (`egui::Window::new`): the room a fixed floater first lays its body in.
const EGUI_WINDOW_HEIGHT: f32 = 420.0;

/// What a floater of the usual layout (title row and divider, no status
/// row, neither `flush_body` nor `framed_header`) adds around its body:
/// its margins and outline across; down, those with the title row and the
/// divider under it. A window whose minimum follows its body adds this.
pub fn floater_chrome(ctx: &egui::Context) -> Vec2 {
    let gap = ctx.global_style().spacing.item_spacing.y;
    let title = ctx
        .fonts_mut(|f| f.row_height(&egui::FontId::proportional(TITLE_SIZE)))
        .max(TITLE_ROW_MIN);
    let m = FLOATER_MARGIN;
    Vec2::new(
        m.leftf() + m.rightf() + 2.0 * FLOATER_STROKE,
        BODY_CHROME + title + gap + DIVIDER_ROOM + gap + UNDER_DIVIDER,
    )
}

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

/// LLSpinCtrl::onUpBtn / onDownBtn (indra/llui/llspinctrl.cpp): Alt ×10,
/// else Ctrl ×0.1, else Shift ×0.01.
pub(crate) fn spin_key_scale(modifiers: egui::Modifiers) -> f32 {
    if modifiers.alt {
        10.0
    } else if modifiers.ctrl {
        0.1
    } else if modifiers.shift {
        0.01
    } else {
        1.0
    }
}

/// clamp_precision (indra/llui/llspinctrl.cpp): an arrow or wheel step is
/// rounded to the field's decimals, so a step finer than the field shows
/// (Ctrl or Shift on a 0-decimal field) changes nothing instead of leaving
/// a hidden fraction. In f64, ten at a time like the Legacy Viewer, with
/// halves rounded up (ll_round: floor(x + 0.5)).
pub(crate) fn clamp_precision(value: f32, decimals: usize) -> f32 {
    let mut v = f64::from(value);
    for _ in 0..decimals {
        v *= 10.0;
    }
    v = (v + 0.5).floor();
    for _ in 0..decimals {
        v /= 10.0;
    }
    v as f32
}

/// Points of a trackpad flick that make one notch. Same grain as the shape
/// editor's category wheel.
const WHEEL_POINTS_PER_NOTCH: f32 = 55.0;

#[derive(Clone, Copy)]
struct WheelRemainder(f32);

pub(crate) struct WheelSteps {
    /// Modifier-scaled notches. Positive raises.
    pub scaled: f32,
    /// The same notches, each rounded to an integer. Ctrl and Shift become
    /// no step on an integer field.
    pub integer: i32,
    /// Whole notches, without the modifiers' scale (LLSlider has none).
    pub notches: f32,
}

impl WheelSteps {
    fn none() -> Self {
        Self {
            scaled: 0.0,
            integer: 0,
            notches: 0.0,
        }
    }
}

/// Notches of the wheel over `rect`. Positive raises. Shared by the
/// build spinners and the colour picker's number fields (both LLSpinCtrl).
///
/// LLSpinCtrl::handleScrollWheel (indra/llui/llspinctrl.cpp) calls onDownBtn
/// for a positive click count and onUpBtn for a negative one, and returns
/// true so the panel does not also scroll. The Legacy Viewer negates the
/// platform wheel (llwindowsdl2.cpp), so rolling it up raises the value.
/// winit reports that roll as a positive delta.y. Each whole notch is scaled
/// like onUpBtn. While the pointer stays on an enabled field the smoothed
/// scroll is eaten, including the frames after the notch, so the build panel
/// does not creep. A disabled field leaves the wheel to the panel, and so
/// does a field that does not `take` it (the build window's fields that are
/// not being typed in, unless its "wheel on hover" option is on). `apply`
/// is false while a typed draft or a drag owns the value: the wheel is still
/// eaten, and it does not change the number.
pub(crate) fn spin_wheel(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, take: bool, apply: bool) -> WheelSteps {
    let key = id.with("spinner_wheel_remainder");
    let hovered = take && ui.is_enabled() && ui.rect_contains_pointer(rect);
    if !hovered {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
        return WheelSteps::none();
    }
    ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
    if !apply {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
        return WheelSteps::none();
    }
    let mut remainder = ui.data(|d| d.get_temp::<WheelRemainder>(key)).map(|kept| kept.0).unwrap_or(0.0);
    let mut scaled = 0.0f32;
    let mut integer = 0i32;
    let mut notches_total = 0.0f32;
    ui.input(|input| {
        for event in &input.events {
            let egui::Event::MouseWheel {
                unit, delta, modifiers, ..
            } = event
            else {
                continue;
            };
            if delta.y == 0.0 || !delta.y.is_finite() {
                continue;
            }
            let notches = delta.y
                / match unit {
                    egui::MouseWheelUnit::Point => WHEEL_POINTS_PER_NOTCH,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 1.0,
                };
            if !notches.is_finite() {
                continue;
            }
            remainder += notches;
            let whole = remainder.trunc().clamp(-100.0, 100.0);
            remainder -= whole;
            if whole != 0.0 {
                notches_total += whole;
                let scale = spin_key_scale(*modifiers);
                scaled += whole * scale;
                integer = integer.saturating_add((whole as i32).saturating_mul(scale.round() as i32));
            }
        }
    });
    if remainder == 0.0 {
        ui.data_mut(|d| d.remove::<WheelRemainder>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, WheelRemainder(remainder)));
    }
    WheelSteps {
        scaled,
        integer,
        notches: notches_total,
    }
}

/// LLButton held_down_delay (widgets/button.xml), in seconds.
const ARROW_HOLD_DELAY: f64 = 0.5;
/// Repeats after the delay. LLButton::handleHover fires every frame; a
/// fixed interval is the same pace at any refresh rate.
const ARROW_HOLD_EVERY: f64 = 0.05;
/// A stalled frame must not dump a long hold as one jump.
const ARROW_HOLD_BURST: u32 = 20;

/// How many repeats a hold has earned. The first is at half a second, then
/// one more every 50 ms.
pub(crate) fn arrow_hold_repeats(held_for: f64) -> u32 {
    if !held_for.is_finite() || held_for < ARROW_HOLD_DELAY {
        return 0;
    }
    // Count in whole milliseconds. `(t - 0.5) / 0.05` is not exact in f64:
    // `0.10 / 0.05` is 1.999…, and floor would skip the repeat that just came due.
    let elapsed_ms = ((held_for - ARROW_HOLD_DELAY) * 1000.0).round();
    if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
        return 1;
    }
    let steps = (elapsed_ms / (ARROW_HOLD_EVERY * 1000.0)).floor();
    let steps = if steps >= u32::MAX as f64 { u32::MAX } else { steps as u32 };
    steps.saturating_add(1)
}

#[derive(Clone, Copy)]
struct ArrowHold {
    since: f64,
    emitted: u32,
}

/// Two compact increment buttons beside numeric fields (LLSpinCtrl).
/// Uses theme faces and Tabler glyphs, and inherits disabled state.
///
/// widgets/spinner.xml sets tab_stop="false" on both buttons. `Sense::CLICK`
/// is clickable and not focusable, so Tab moves from the field to the next
/// field. A hold repeats onUpBtn/onDownBtn: LLButton::handleHover
/// (indra/llui/llbutton.cpp) fires the held callback every frame after
/// held_down_delay, and handleMouseUp steps once more on release. Repeating
/// every frame is one step per rendered frame, so a high refresh rate runs
/// away with the value. The repeat here is every 50 ms: the same pace on
/// every machine, and still a quick walk. The first step stays the release,
/// so a short click is one step. Dragging off the button ends the repeat.
/// The release counts only when the press and the release are both on the
/// button. egui forgets a click-only widget after 0.8 s or 6 px of travel
/// (`is_pointer_button_down_on` and `clicked()` both go false while the
/// button is still held). A held arrow outlasts that window, so the repeat
/// is our own timer and the release step is the button coming up with the
/// pointer still on it, the same moment as handleMouseUp.
pub(crate) fn step_buttons(ui: &mut egui::Ui, height: f32) -> i32 {
    let mut step = 0i32;
    let now = ui.input(|i| i.time);
    let enabled = ui.is_enabled();
    ui.allocate_ui_with_layout(Vec2::new(13.0, height), egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_max_width(13.0);
        ui.spacing_mut().item_spacing.y = 0.0;
        for (direction, name, label) in [
            (1i32, "caret-up-filled", aurora_i18n::tr!("numeric-increase")),
            (-1, "caret-down-filled", aurora_i18n::tr!("numeric-decrease")),
        ] {
            // Not focusable: see the function comment and widgets/spinner.xml.
            let (rect, response) = ui.allocate_exact_size(Vec2::new(13.0, height * 0.5), Sense::CLICK);
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
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
            let response = response.on_hover_text(label);
            if !enabled {
                continue;
            }
            let id = response.id.with("spin_arrow_hold");
            let pointer_here = ui.input(|i| i.pointer.interact_pos()).is_some_and(|at| rect.contains(at));
            let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let tracked = ui.data(|d| d.get_temp::<ArrowHold>(id));
            // is_pointer_button_down_on stays true while a fresh press is
            // dragged outside the widget, and it goes false once egui's click
            // window ends. The hold stops when the pointer leaves. One we
            // already started keeps going for as long as the button is down.
            let holding = primary_down && pointer_here && (response.is_pointer_button_down_on() || tracked.is_some());
            let mut repeats = 0u32;
            let mut release_step = false;
            if holding {
                let mut hold = tracked.unwrap_or(ArrowHold { since: now, emitted: 0 });
                let due = arrow_hold_repeats(now - hold.since);
                repeats = due.saturating_sub(hold.emitted).min(ARROW_HOLD_BURST);
                // Drop repeats a stalled frame would have piled up, so the
                // value does not fast-forward when the window wakes.
                hold.emitted = due;
                let until = if due == 0 {
                    ARROW_HOLD_DELAY - (now - hold.since)
                } else {
                    let into = (now - hold.since - ARROW_HOLD_DELAY).rem_euclid(ARROW_HOLD_EVERY);
                    ARROW_HOLD_EVERY - into
                };
                ui.ctx().request_repaint_after_secs(until.max(0.0) as f32);
                ui.data_mut(|d| d.insert_temp(id, hold));
            } else if let Some(hold) = tracked {
                // Released on the button: the repeat that came due, then one
                // more step. Leaving while the button is still down just stops.
                if pointer_here && !primary_down {
                    let due = arrow_hold_repeats(now - hold.since);
                    repeats = due.saturating_sub(hold.emitted).min(ARROW_HOLD_BURST);
                    release_step = true;
                }
                ui.data_mut(|d| d.remove::<ArrowHold>(id));
            } else if response.clicked() {
                // Press and release in the same frame, before a hold existed.
                release_step = true;
            }
            if release_step {
                step += direction;
            }
            step += direction * repeats as i32;
        }
    });
    step
}

/// Draft for an optional in-title document name editor. Other floaters do not use it.
#[derive(Clone, Default)]
struct FloaterTitleDraft {
    text: String,
    focus: bool,
}

fn clean_floater_title_name(input: &str) -> Option<String> {
    crate::world::inventory::actions::clean_name(input).ok()
}

/// Label (or inline input) occupying only the visible document-name glyphs.
/// The rest of the header remains the ordinary minimize double-click target.
/// Decide title-edit completion without conflating a click in the field
/// with an actual click away. Escape has priority over focus loss or Enter.
fn floater_title_finish(focused: bool, escape: bool, enter: bool, lost_focus: bool) -> Option<bool> {
    if (focused || lost_focus) && escape {
        Some(false)
    } else if focused && enter || lost_focus {
        Some(true)
    } else {
        None
    }
}

fn floater_title_name(
    ui: &mut egui::Ui,
    title: RichText,
    displayed: &str,
    truncate: bool,
    draft: &mut Option<FloaterTitleDraft>,
    finished: &mut Option<bool>,
    field: egui::Id,
) -> egui::Rect {
    if let Some(edit) = draft.as_mut() {
        // Capture shortcuts before the TextEdit can use Escape to release focus.
        // That focus loss must never be mistaken for an accepted rename.
        let focused = ui.memory(|m| m.has_focus(field));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let response = restricted_text_field(ui, true, &mut edit.text, TextPolicy::PrintableNoPipe, |buffer, _| {
            egui::TextEdit::singleline(buffer).id(field).char_limit(63).desired_width(225.0)
        });
        if edit.focus {
            response.request_focus();
            edit.focus = false;
        }
        let click_outside = ui.input(|i| {
            i.pointer.button_pressed(egui::PointerButton::Primary)
                && i.pointer.interact_pos().is_some_and(|pos| !response.rect.contains(pos))
        });
        if let Some(result) = floater_title_finish(focused, escape, enter, response.lost_focus() || focused && click_outside) {
            *finished = Some(result);
        }
        response.rect
    } else {
        let label = egui::Label::new(title);
        let response = ui.add(if truncate { label.truncate() } else { label });
        // A truncating label may occupy a whole row; target only visible glyphs.
        let text_width = ui
            .painter()
            .layout_no_wrap(
                displayed.to_owned(),
                egui::FontId::proportional(TITLE_SIZE),
                ui.visuals().text_color(),
            )
            .size()
            .x
            + 8.0;
        egui::Rect::from_min_size(
            response.rect.min,
            Vec2::new(text_width.min(response.rect.width()), response.rect.height()),
        )
    }
}
/// The name being typed in the title of the window `window`, while its
/// rename is allowed: in the viewer's window and in its window outside.
fn title_draft_of(ctx: &egui::Context, window: egui::Id, rename: Option<&(String, bool)>) -> Option<FloaterTitleDraft> {
    let allowed = rename.is_some_and(|(_, allowed)| *allowed);
    ctx.data_mut(|d| d.get_temp::<FloaterTitleDraft>(window.with("title_rename_draft")))
        .filter(|_| allowed)
}

/// Ends the pass of a title that can be renamed: a finished edit (`Some`,
/// true to keep it) leaves its cleaned name for [`take_title_rename`] when
/// it changed, and the draft still open is kept for the next pass.
fn title_draft_end(
    ctx: &egui::Context,
    window: egui::Id,
    rename: Option<&(String, bool)>,
    mut draft: Option<FloaterTitleDraft>,
    finished: Option<bool>,
) {
    if let Some(save) = finished {
        if save
            && let Some(edit) = draft.as_ref()
            && let Some((original, true)) = rename
            && let Some(name) = clean_floater_title_name(&edit.text)
            && name != original.as_str()
        {
            ctx.data_mut(|d| d.insert_temp(window.with("title_rename_result"), name));
        }
        draft = None;
    }
    ctx.data_mut(|d| {
        if let Some(draft) = draft {
            d.insert_temp(window.with("title_rename_draft"), draft);
        } else {
            d.remove_temp::<FloaterTitleDraft>(window.with("title_rename_draft"));
        }
    });
}

/// A completed title rename, requested by the document editor's own floater.
/// Only the caller performs permission checks and sends a metadata update.
pub fn take_title_rename(ctx: &egui::Context, window: &str) -> Option<String> {
    ctx.data_mut(|d| d.remove_temp::<String>(egui::Id::unique(window).with("title_rename_result")))
}

pub struct Floater<'a> {
    pub id: &'a str,
    pub title: String,
    /// Document name and permission gate for title double-click rename. Other
    /// floaters leave this unset and retain ordinary double-click minimizing.
    title_rename: Option<(String, bool)>,
    /// Optional compact state row within the title bar: glyph, label, theme role.
    pub header_status: Vec<(&'static str, String, Color32)>,
    /// A text action at the right end of the secondary header row; a click
    /// writes the action's rect, for feedback drawn beside it.
    pub header_status_action: Option<(&'a str, &'a str, &'a str, &'a mut Option<egui::Rect>)>,
    /// A single-line detail row inside the expanded title section.
    pub header_summary: Option<egui::WidgetText>,
    pub header_summary_action: Option<&'a mut bool>,
    /// An icon button at the right end of the detail row, under the status
    /// action: its glyph, tooltip, and whether it was clicked.
    pub header_summary_icon: Option<(&'a str, &'a str, &'a mut bool)>,
    /// The width of a panel that sits beside the window in the viewer's
    /// (the build window's settings), 0 for none. Outside the viewer's
    /// nothing is beside a window: its window there is that much wider,
    /// and the panel is drawn in that room (`side_room_outside`), in the
    /// window's own pass; over the window's right end where the system
    /// gives no room.
    pub side_outside: f32,
    /// A short muted text right after the title (a count of what the
    /// window holds).
    pub title_note: Option<String>,
    pub title_note_tooltip: Option<String>,
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
    /// No title bar: its close cross in a small box against its right edge
    /// (see `headless`).
    pub headless: bool,
    /// Its body's margins in place of the floater's own (`margin`).
    pub margin: Option<egui::Margin>,
    /// A window without a title bar shows no close cross, and Ctrl+W passes
    /// over it (`without_close`).
    pub closable: bool,
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
            title_rename: None,
            header_status: Vec::new(),
            header_status_action: None,
            header_summary: None,
            header_summary_action: None,
            header_summary_icon: None,
            side_outside: 0.0,
            title_note: None,
            title_note_tooltip: None,
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
            headless: false,
            margin: None,
            closable: true,
            cascade: None,
            partial_outside: false,
            chat: false,
        }
    }

    /// Only document editors opt in. A read-only document still reserves its
    /// name as a non-minimizing double-click target, but cannot rename.
    pub fn rename_title(mut self, name: impl Into<String>, can_rename: bool) -> Self {
        self.title_rename = Some((name.into(), can_rename));
        self
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

    /// Details shown when hovering the muted text after the title.
    pub fn title_note_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.title_note_tooltip = Some(tooltip.into());
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

    /// The user may send this window to a window of the system, outside
    /// the viewer's, and bring it back (outside.rs): its title bar has the
    /// button for it. Not in the Legacy Viewer. Every window may, but those
    /// of `STAYS_INSIDE` and one whose sizing the window outside does not
    /// follow (`show_outside`): a height the user sets at a held width. A
    /// window drawn over an overlay of the viewer's (`order`) is a window
    /// of its own outside, over nothing.
    fn goes_outside(&self) -> bool {
        !STAYS_INSIDE.contains(&self.id) && !self.height_only
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

    /// No title bar, only a close cross in a small box against the window's
    /// right edge, at its top, outside it (the build window's transform
    /// block taken out): the window is moved by its background, as any
    /// window by the room its widgets leave, and it cannot be minimized.
    pub fn headless(mut self) -> Self {
        self.headless = true;
        self
    }

    /// Its body's margins, in place of the floater's own: a small window
    /// without a title bar laid out to the pixel (the Classic style's small
    /// camera window).
    pub fn margin(mut self, margin: egui::Margin) -> Self {
        self.margin = Some(margin);
        self
    }

    /// Without a title bar, and without the close cross beside it either:
    /// the window closes by what opened it (its toolbar button, its menu
    /// entry), as the Legacy Viewer's small camera window
    /// (floater_fs_camera_small.xml, `can_close="false"`). Ctrl+W passes
    /// over it to the window behind, and Ctrl+Shift+W still closes it
    /// (LLFloaterView::getFrontmostClosableFloater skips a floater that is
    /// not closeable; closeAllChildren does not).
    pub fn without_close(mut self) -> Self {
        self.headless = true;
        self.closable = false;
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
        let role = if self.chat { HideKeep::Chat } else { HideKeep::None };
        let hidden = window_hidden(ctx, egui::Id::unique(self.id), *open, role);
        // A window outside the viewer's stays up while the interface is
        // hidden (mouselook, immersive mode, "Hide the interface"): it
        // covers nothing of the world. It is still counted with the hidden
        // ones (`window_hidden`, above): brought back inside meanwhile, it
        // is hidden with them and comes back with the interface.
        let stays_outside = *open && self.goes_outside() && {
            let window_id = egui::Id::unique(self.id);
            crate::outside::wanted(ctx, window_id) && !super::layout::reset_pending(ctx, window_id)
        };
        if hidden && !stays_outside {
            // its toolbar button still closes it with the interface hidden,
            // as it does the windows it toggles itself
            if close_asked(ctx, self.id) {
                *open = false;
            }
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
        if self.goes_outside() && crate::outside::wanted(ctx, egui::Id::unique(self.id)) {
            let window_id = egui::Id::unique(self.id);
            if !super::layout::reset_pending(ctx, window_id) {
                return self.show_outside(ctx, p, open, body);
            }
            // "Reset window positions and sizes": back inside the viewer's,
            // where it takes its default place and size below
            crate::outside::want(ctx, window_id, false);
            ctx.data_mut(|d| d.remove::<Vec2>(window_id.with(OUTSIDE_SIZE)));
        }
        let min_id = egui::Id::unique((self.id, "minimized"));
        // "Reset window positions and sizes" (`reset_windows`): back to its
        // default place and size, expanded, at this frame
        let reset = super::layout::take_reset(ctx, egui::Id::unique(self.id));
        if reset {
            ctx.data_mut(|d| {
                d.remove::<bool>(min_id);
                d.remove::<Vec2>(egui::Id::unique(self.id).with("expanded_size"));
            });
            if self.cascade.is_some() {
                // it opens again as the first of its group
                cascade::forget(ctx, egui::Id::unique(self.id));
            }
        }
        // persisted with the UI layout (ui_layout.ron)
        // a window without a title bar has no bar to be minimized to
        let minimized = !self.headless && ctx.data_mut(|d| d.get_persisted::<bool>(min_id)).unwrap_or(false);
        // minimized with its compact body: sized by that body
        let compact = minimized && self.compact_minimized.is_some();
        if !self.silent {
            super::sound_cues::floater_shown(ctx, egui::Id::unique(self.id));
        }
        let opacity = super::surface_opacity::window(ctx, self.id, self.chat);
        let shadow = ctx.global_style().visuals.window_shadow;
        let fill = super::surface_opacity::frame_fill(
            p,
            Color32::from_rgba_unmultiplied(p.panel.r(), p.panel.g(), p.panel.b(), p.panel_alpha),
            opacity,
        );
        let frame = egui::Frame::new()
            .fill(fill)
            .stroke(border_stroke(p, Stroke::new(FLOATER_STROKE, p.muted_dim.gamma_multiply(0.6))))
            .corner_radius(ctx.global_style().visuals.window_corner_radius)
            .shadow(egui::Shadow {
                color: shadow.color.gamma_multiply(opacity),
                ..shadow
            })
            .inner_margin(if let Some(margin) = self.margin {
                margin
            } else if self.framed_header.is_some() {
                egui::Margin::ZERO
            } else if self.headless {
                egui::Margin {
                    top: 8,
                    bottom: 8,
                    ..FLOATER_MARGIN
                }
            } else if self.flush_body {
                egui::Margin {
                    left: 0,
                    right: 0,
                    top: if minimized && !compact { 4 } else { 5 },
                    bottom: if minimized && !compact { 4 } else { 0 },
                }
            } else if minimized && !compact {
                egui::Margin {
                    top: 4,
                    bottom: 4,
                    ..FLOATER_MARGIN
                }
            } else {
                FLOATER_MARGIN
            });
        // Ctrl+W closes it as its cross does, when it is the frontmost.
        // Ctrl+Shift+W closes every window, one without a close cross too,
        // which Ctrl+W passes over (`without_close`).
        let mut close = super::close_key::take(ctx, egui::Id::unique(self.id)) || close_asked(ctx, self.id);
        if self.closable {
            super::close_key::shown(ctx, egui::Id::unique(self.id));
        } else {
            super::close_key::shown_extra(ctx, egui::Id::unique(self.id));
        }
        // the toolbar button, a menu entry or a shortcut asked to expand it
        let expand_asked = ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(self.id).with(EXPAND)));
        // asked to the front from the body of a window outside, whose
        // `move_to_top` orders that window's own layers (`bring_floater_front`)
        if ctx
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(self.id).with(FRONT)))
            .unwrap_or(false)
        {
            ctx.move_to_top(egui::LayerId::new(self.order, egui::Id::unique(self.id)));
        }
        let mut toggle_min = minimized && expand_asked.unwrap_or(false);
        let mut go_outside = false;
        let mut out = None;
        let window_id = egui::Id::unique(self.id);
        let title_field_id = window_id.with("title_rename_field");
        let mut title_draft = title_draft_of(ctx, window_id, self.title_rename.as_ref());
        let mut title_finished: Option<bool> = None;
        let expanded_id = window_id.with("expanded_size");
        let restore_id = window_id.with("restore_size");
        let expanded = ctx.data_mut(|d| d.get_persisted::<Vec2>(expanded_id)).unwrap_or(self.default_size);
        let restore = ctx.data_mut(|d| d.remove_temp::<bool>(restore_id)).unwrap_or(false) || (reset && self.resizable);
        let resizable = self.resizable && !minimized;
        // The windows live between the bars (`layout::work_area`): none
        // opens larger than that room, nor outside it.
        let work = super::layout::work_area(ctx);
        self.min_size = self.min_size.min(work.size());
        self.default_size = self.default_size.min(work.size());
        self.default_pos = fit_in(work, self.default_pos, self.default_size);
        if reset && self.cascade.is_none() {
            super::layout::reanchor(ctx, window_id);
            ctx.data_mut(|d| d.insert_temp(window_id.with(PLACE_POS), self.default_pos));
        }
        let opening = self.cascade.and_then(|group| {
            cascade::prepare(
                ctx,
                window_id,
                group,
                egui::Rect::from_min_size(self.default_pos, expanded),
                work,
                self.min_size,
            )
        });
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
        // A window sized by its content lays its body out in the room egui's
        // window had the frame before, and only ever grows past it: squeezed
        // once (the viewer's window made tiny), that room stayed small, and a
        // body that fits it (a scroll area, wrapped text) kept the window
        // small for good (the Director's, Aurora Weather's: the human's
        // reports of 2026-11-17 and 2026-11-21). It gets at least the room
        // egui gives a window it opens, which changes nothing until a squeeze.
        if self.auto_height && !minimized {
            w = w
                .auto_sized()
                .min_width(self.default_size.x)
                .max_width(self.default_size.x)
                .min_height(work.height());
        } else if self.resizable && !minimized {
            w = w.default_size(self.default_size).min_size(self.min_size);
        } else if !minimized {
            w = w
                .default_width(self.default_size.x)
                .min_width(self.default_size.x)
                .min_height(EGUI_WINDOW_HEIGHT.min(work.height()));
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
            // a stacked window made smaller to keep its step
            // (`cascade::Group::keeps_step`) stops at its least size
            w = w.fixed_size(size.max(self.min_size));
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
            if self.headless {
                // the body alone: the cross is in its box outside the window
                out = Some(body(ui));
                if let Some(rect) = held {
                    outer.advance_cursor_after_rect(rect);
                }
                return;
            }
            // title bar
            let mut buttons_left = f32::INFINITY;
            let mut note_rect = None;
            let mut title_name_rect = None;
            let mut primary_title_rect = None;
            let framed_header = self.framed_header;
            if framed_header.is_some() {
                ui.spacing_mut().item_spacing.y = 0.0;
            }
            let goes_outside = self.goes_outside();
            let mut header_content = |ui: &mut egui::Ui| {
                if framed_header.is_some() {
                    ui.spacing_mut().interact_size.y = 20.0;
                }
                let title_row = ui.horizontal(|ui| {
                    ui.set_min_height(if framed_header.is_some() { 20.0 } else { TITLE_ROW_MIN });
                    let title = RichText::new(&self.title).size(TITLE_SIZE).strong().color(p.ink);
                    // a fixed floater is as wide as its title; a resizable one
                    // cuts a long title short before the buttons (LLFloater
                    // titles end with an ellipsis)
                    let truncate = self.resizable || (minimized && !compact) || self.auto_height;
                    if !truncate {
                        glyph(ui, workspace::window_icon(self.id), p.violet_light, 18.0);
                        title_name_rect = Some(floater_title_name(
                            ui,
                            title.clone(),
                            &self.title,
                            false,
                            &mut title_draft,
                            &mut title_finished,
                            title_field_id,
                        ));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if title_button(ui, p, "x").on_hover_text(aurora_i18n::tr!("common-close")).clicked() {
                            close = true;
                        }
                        // a window with a compact body when minimized says
                        // so with the Performance window's compact / full
                        // icons rather than a collapse (#1035)
                        let icon = minimize_title_icon(self.compact_minimized.is_some(), minimized);
                        let tip = match (self.compact_minimized.is_some(), minimized) {
                            (true, true) => aurora_i18n::tr!("common-full-view"),
                            (true, false) => aurora_i18n::tr!("common-compact-view"),
                            (false, true) => aurora_i18n::tr!("common-expand"),
                            (false, false) => aurora_i18n::tr!("common-collapse"),
                        };
                        let response = title_button(ui, p, icon).on_hover_text(tip);
                        #[cfg(test)]
                        ui.ctx()
                            .data_mut(|d| d.insert_temp(egui::Id::unique(("floater_minimize", self.id)), response.rect));
                        if response.clicked() {
                            toggle_min = true;
                        }
                        if goes_outside
                            && title_button(ui, p, OUTSIDE_ICON)
                                .on_hover_text(aurora_i18n::tr!("common-window-outside"))
                                .clicked()
                        {
                            go_outside = true;
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
                                title_name_rect = Some(floater_title_name(
                                    ui,
                                    title,
                                    &self.title,
                                    true,
                                    &mut title_draft,
                                    &mut title_finished,
                                    title_field_id,
                                ));
                                if let Some(note) = &self.title_note {
                                    let response = ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.muted)).truncate());
                                    note_rect = Some(response.rect);
                                }
                            });
                        }
                    });
                });
                primary_title_rect = Some(title_row.response.rect);
                if !minimized || compact {
                    header_rows(
                        ui,
                        p,
                        &self.header_status,
                        &mut self.header_status_action,
                        self.header_summary.as_ref(),
                        &mut self.header_summary_action,
                        &mut self.header_summary_icon,
                        None,
                    );
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
            // The header layout closure is no longer used; release its
            // mutable borrow of the inline name editor before hit-testing.
            drop(header_content);
            // Secondary header controls own their clicks; only the title row
            // is the window's double-click target.
            let mut title_rect = primary_title_rect.unwrap_or(header.rect);
            if framed_header.is_some() {
                ui.painter().hline(header.rect.x_range(), header.rect.bottom(), divider_stroke(p));
            }
            title_rect.max.x = buttons_left;
            // Keep the title buttons outside the double-click target; Window
            // still owns dragging the bar, including when it is minimized.
            // While renaming, do not place the title-bar's invisible click
            // surface over the TextEdit: it steals caret-placement clicks.
            // Both remaining bar segments keep double-click minimize/restore.
            if let Some(field_rect) = title_name_rect.filter(|_| title_draft.is_some()) {
                let before = egui::Rect::from_min_max(
                    title_rect.min,
                    egui::pos2(field_rect.left().clamp(title_rect.left(), title_rect.right()), title_rect.bottom()),
                );
                let after = egui::Rect::from_min_max(
                    egui::pos2(field_rect.right().clamp(title_rect.left(), title_rect.right()), title_rect.top()),
                    title_rect.max,
                );
                for (part, rect) in [
                    ("title_double_click_before_rename", before),
                    ("title_double_click_after_rename", after),
                ] {
                    if rect.width() > 1.0 && ui.interact(rect, window_id.with(part), Sense::click()).double_clicked() {
                        toggle_min = true;
                    }
                }
            } else {
                let title_response = ui.interact(title_rect, window_id.with("title_double_click"), Sense::click());
                if title_response.double_clicked() {
                    let on_name = title_response
                        .interact_pointer_pos()
                        .is_some_and(|pos| title_name_rect.is_some_and(|rect: egui::Rect| rect.contains(pos)));
                    if on_name && self.title_rename.is_some() {
                        // A minimized document restores on double-click,
                        // whether or not the item has Modify permission.
                        if minimized || self.title_rename.as_ref().is_some_and(|(_, allowed)| !*allowed) {
                            toggle_min = true;
                        } else if title_draft.is_none()
                            && let Some((name, true)) = &self.title_rename
                        {
                            title_draft = Some(FloaterTitleDraft {
                                text: name.clone(),
                                focus: true,
                            });
                            ctx.request_repaint();
                        }
                    } else {
                        toggle_min = true;
                    }
                }
                if let Some(rect) = note_rect
                    && let Some(tooltip) = &self.title_note_tooltip
                    && ui.input(|input| input.pointer.hover_pos().is_some_and(|pos| rect.contains(pos)))
                {
                    title_response.with_new_rect(rect).on_hover_text(tooltip);
                }
            }
            if compact {
                divider(ui, p);
                ui.add_space(UNDER_DIVIDER);
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
                        ui.add_space(UNDER_DIVIDER);
                    }
                }
                if scrolls {
                    let above = ui.cursor().top() - ui.max_rect().top();
                    let room = (work.height() - above - BODY_CHROME).max(MIN_SCROLLED_BODY);
                    // exactly the room tall while it scrolls, not the
                    // smaller room a squeeze left (`auto_height_body`)
                    let shown = egui::ScrollArea::vertical()
                        .id_salt("floater_body")
                        .max_height(room)
                        .min_scrolled_height(room)
                        .show(ui, body);
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
            if self.headless && self.closable && headless_close(ctx, p, window_id, &response.response, fill, opacity, self.order) {
                close = true;
            }
            if self.cascade.is_some() {
                cascade::shown(ctx, window_id, response.response.rect, minimized);
            }
            if let Some((painter, slot)) = glass_slot {
                let radius = f32::from(ctx.global_style().visuals.window_corner_radius.nw);
                super::surface_opacity::fade_after(ctx, painter.layer_id(), slot, p, opacity);
                workspace::finish_glass_faded(
                    &painter.with_clip_rect(response.response.rect),
                    slot,
                    response.response.rect,
                    p,
                    radius,
                    opacity,
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
        title_draft_end(ctx, window_id, self.title_rename.as_ref(), title_draft, title_finished);
        if close {
            *open = false;
            // LLFloater::closeFloater: "Always unminimize before trying to
            // close", the window opens again expanded
            unminimize_quietly(ctx, self.id);
            if self.cascade.is_some() {
                cascade::hidden(ctx, window_id);
            }
        } else if go_outside {
            // (it comes back expanded, where and as large as it was; a
            // compact view stays the view it has, outside too)
            if self.compact_minimized.is_none() {
                unminimize_quietly(ctx, self.id);
            }
            crate::outside::want(ctx, window_id, true);
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

    /// The window in a window of the system, outside the viewer's
    /// (outside.rs): the same body on the palette's solid panel, under one
    /// row of the title bar's buttons. The system's title bar names it,
    /// moves it and closes it; the first button brings it back inside.
    fn show_outside<R>(mut self, ctx: &egui::Context, p: &Palette, open: &mut bool, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
        let window_id = egui::Id::unique(self.id);
        if !self.silent {
            super::sound_cues::floater_shown(ctx, window_id);
        }
        // Ctrl+Shift+W closes it with the windows inside. Ctrl+W in the
        // viewer's window goes to the front window there, never to this
        // one; pressed in this one, it closes it (`pressed_outside`, below)
        let close_key = super::close_key::take(ctx, window_id);
        super::close_key::shown_extra(ctx, window_id);
        if close_key || close_asked(ctx, self.id) {
            *open = false;
            return None;
        }
        // (no longer at its place among the windows inside)
        if self.cascade.is_some() {
            cascade::hidden(ctx, window_id);
        }
        let asked_id = window_id.with("outside content size");
        if !crate::outside::native(ctx, window_id) {
            // (a new window outside: no size was asked of it yet)
            ctx.data_mut(|d| {
                d.remove::<(Vec2, f32)>(asked_id);
                d.remove::<SideRoom>(window_id.with(SIDE_ROOM));
                d.remove::<egui::Rect>(window_id.with(SIDE_RECT));
            });
            // asked at this pass, opened before the next: as large as it
            // was outside last, or as it is inside
            let inside = egui::AreaState::load(ctx, window_id);
            let size = ctx
                .data_mut(|d| d.get_persisted::<Vec2>(window_id.with(OUTSIDE_SIZE)))
                .or_else(|| inside.and_then(|s| s.size))
                .unwrap_or(self.default_size);
            let min = inside.map_or(self.default_pos, |s| s.left_top_pos());
            crate::outside::ask(
                ctx,
                crate::outside::Ask {
                    window: window_id,
                    title: self.title.clone(),
                    rect: egui::Rect::from_min_size(min, size),
                    min_size: self.min_size,
                    content_sized: !self.resizable,
                },
            );
            return None;
        }
        crate::outside::seen(ctx, window_id);
        // An expand asked for (`expand_floater`: its toolbar button, a tab
        // that shows in its full form) is taken here as inside: the compact
        // view gives way to the full one, and nothing is left pending
        // (`floater_expand_pending`).
        if ctx.data_mut(|d| d.remove_temp::<bool>(window_id.with(EXPAND))).unwrap_or(false) && floater_minimized(ctx, self.id) {
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((self.id, "minimized")), false));
        }
        // its own window is in front of the viewer's layers: an ask to the
        // front among them (`to_front`) is not kept for its return inside
        ctx.data_mut(|d| d.remove_temp::<bool>(window_id.with(FRONT)));
        let flush = self.flush_body;
        let framed = self.framed_header.is_some();
        // as tall as its content (fixed, or of automatic height), and for a
        // fixed one as wide when the content is wider than its width
        let content_sized = !self.resizable;
        // its compact view (`compact_minimized`), which its body draws from
        // the same state as inside, as wide as inside
        let compact = self.compact_minimized.filter(|_| floater_minimized(ctx, self.id));
        let (width, auto_height) = (compact.unwrap_or(self.default_size.x), self.auto_height);
        let mut toggle_compact = false;
        let mut content = None;
        // a picture of a kept shape (`keep_aspect`): its ratio, and once
        // drawn the height of the window's head and the window's width
        let ratio = self.aspect.map(|(ratio, _)| ratio);
        let mut shape = None;
        let mut body = Some(body);
        let mut back = false;
        // a document's name, renamed by a double click on it as in the
        // title inside: the title of this window is the system's
        let title_field_id = window_id.with("title_rename_field");
        let mut title_draft = title_draft_of(ctx, window_id, self.title_rename.as_ref());
        let mut title_finished: Option<bool> = None;
        let builder = egui::ViewportBuilder::default().with_title(self.title.clone());
        let out = ctx.show_viewport_immediate(crate::outside::viewport(window_id), builder, |ui, _| {
            super::close_key::pressed_outside(ui.ctx(), window_id);
            // the window's own width, and the room at its right for what
            // sits beside it inside (`side_outside`)
            let whole = ui.max_rect();
            let wanted = if self.side_outside > 0.0 {
                self.side_outside + 2.0 * SIDE_GAP
            } else {
                0.0
            };
            let was = ui.ctx().data(|d| d.get_temp::<SideRoom>(window_id.with(SIDE_ROOM)));
            // (the viewer's clock: a window outside has its own input)
            let now = ui.ctx().input_for(egui::ViewportId::ROOT, |i| i.time);
            let (side, ask) = side_room(was, whole.width(), wanted, self.min_size.x, ui.ctx().zoom_factor(), now);
            if let Some(width) = ask {
                crate::outside::resize(ui.ctx(), window_id, Vec2::new(width, whole.height()), content_sized);
            }
            let own = side.map_or(whole.width(), |side| side.own.min(whole.width()));
            let room = side.and_then(|side| side_rect(whole, side));
            ui.ctx().data_mut(|d| {
                d.remove::<SideRoom>(window_id.with(SIDE_ROOM));
                d.remove::<egui::Rect>(window_id.with(SIDE_RECT));
                if let Some(side) = side {
                    d.insert_temp(window_id.with(SIDE_ROOM), side);
                }
                if let Some(room) = room {
                    d.insert_temp(window_id.with(SIDE_RECT), room);
                }
            });
            let size = Vec2::new(own, whole.height());
            ui.ctx().data_mut(|d| d.insert_persisted(window_id.with(OUTSIDE_SIZE), size));
            // what follows the pointer during a drag, as the viewer's pass
            // draws it in its own window (`App::inventory_drag_input`)
            super::inventory::drag::badge(ui.ctx(), p);
            super::places::drag::ghost(ui.ctx(), p);
            crate::outside::carry::hint(ui.ctx(), p);
            let margin = if framed {
                egui::Margin::ZERO
            } else if flush {
                egui::Margin {
                    left: 0,
                    right: 0,
                    top: 5,
                    bottom: 0,
                }
            } else {
                FLOATER_MARGIN
            };
            // (the window keeps its own width, at the left of the room: the
            // same region narrowed, not one inside it, whose scroll places
            // and focus would be other ones while the room is there)
            if own < whole.width() {
                ui.set_max_width(own);
                ui.set_clip_rect(ui.clip_rect().intersect(egui::Rect::from_min_size(whole.min, size)));
            }
            egui::Frame::new()
                .inner_margin(margin)
                .show(ui, |ui| {
                    if !content_sized {
                        ui.set_min_size(ui.available_size());
                    }
                    // An inspector's header (`framed_header`) has a row of
                    // states under its title inside. Outside the title is
                    // the system's: its buttons end the row of the states,
                    // where a row of their own is a dark band that holds
                    // nothing else.
                    let on_states_row = framed
                        && (!self.header_status.is_empty() || self.header_status_action.is_some())
                        && self.title_state.is_none()
                        && self.title_rename.is_none()
                        && self.title_note.is_none();
                    // (from the right, in a row laid out that way)
                    let mut buttons = |ui: &mut egui::Ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if title_button(ui, p, INSIDE_ICON)
                            .on_hover_text(aurora_i18n::tr!("common-window-inside"))
                            .clicked()
                        {
                            back = true;
                        }
                        if self.compact_minimized.is_some() {
                            let tip = if compact.is_some() {
                                aurora_i18n::tr!("common-full-view")
                            } else {
                                aurora_i18n::tr!("common-compact-view")
                            };
                            if title_button(ui, p, minimize_title_icon(true, compact.is_some()))
                                .on_hover_text(tip)
                                .clicked()
                            {
                                toggle_compact = true;
                            }
                        }
                        for (icon, tip, clicked) in &mut self.actions {
                            if title_button(ui, p, icon).on_hover_text(*tip).clicked() {
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
                        if self.title_rename.is_none() && self.title_note.is_none() {
                            return;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            if let Some((name, allowed)) = &self.title_rename {
                                let rect = floater_title_name(
                                    ui,
                                    RichText::new(name).size(TITLE_SIZE).strong().color(p.ink),
                                    name,
                                    true,
                                    &mut title_draft,
                                    &mut title_finished,
                                    title_field_id,
                                );
                                if *allowed
                                    && title_draft.is_none()
                                    && ui
                                        .interact(rect, window_id.with("title_double_click"), Sense::click())
                                        // (the title bar above holds the name too, and is the system's)
                                        .on_hover_text(aurora_i18n::tr!("common-window-rename-name"))
                                        .double_clicked()
                                {
                                    title_draft = Some(FloaterTitleDraft {
                                        text: name.clone(),
                                        focus: true,
                                    });
                                    ui.ctx().request_repaint();
                                }
                            }
                            if let Some(note) = &self.title_note {
                                ui.add(egui::Label::new(RichText::new(note).size(12.0).color(p.muted)).truncate());
                            }
                        });
                    };
                    let mut header = |ui: &mut egui::Ui| {
                        if !on_states_row {
                            ui.horizontal(|ui| {
                                ui.set_min_height(if framed { 20.0 } else { TITLE_ROW_MIN });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), &mut buttons);
                            });
                        }
                        header_rows(
                            ui,
                            p,
                            &self.header_status,
                            &mut self.header_status_action,
                            self.header_summary.as_ref(),
                            &mut self.header_summary_action,
                            &mut self.header_summary_icon,
                            on_states_row.then_some(&mut buttons as &mut dyn FnMut(&mut egui::Ui)),
                        );
                    };
                    if framed {
                        // an inspector's dark bar, from edge to edge as inside
                        let bar = egui::Frame::new()
                            .fill(p.bar)
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: 10,
                                bottom: 10,
                            })
                            .show(ui, header)
                            .response;
                        ui.painter().hline(bar.rect.x_range(), bar.rect.bottom(), divider_stroke(p));
                    } else if flush {
                        // the row keeps the margins the body gives up
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, header);
                        ui.add_space(5.0);
                    } else {
                        header(ui);
                        divider(ui, p);
                        ui.add_space(UNDER_DIVIDER);
                    }
                    if let Some(ratio) = ratio {
                        // the system sizes the window as the user likes,
                        // to the whole screen too, and no system holds a
                        // window to a ratio the same way: the picture
                        // keeps its shape in the room it has, as in any
                        // player, where the edges held it inside
                        let room = ui.available_rect_before_wrap();
                        shape = Some((room.top(), ui.ctx().content_rect().width()));
                        ui.allocate_rect(room, Sense::hover());
                        let picture = egui::Rect::from_center_size(room.center(), letterboxed(room.size(), ratio));
                        return ui
                            .scope_builder(egui::UiBuilder::new().max_rect(picture), |ui| body.take().map(|body| body(ui)))
                            .inner;
                    }
                    if !content_sized {
                        return body.take().map(|body| body(ui));
                    }
                    // its window takes the size of what it shows, and it
                    // scrolls where the screen is too small for that
                    // (`outside::fitting`), as inside a small viewer
                    let above = ui.cursor().top();
                    let shown = sized_body(ui, |ui| body.take().map(|body| body(ui)));
                    let sides = f32::from(margin.left) + f32::from(margin.right);
                    content = Some(Vec2::new(
                        if auto_height {
                            width
                        } else {
                            width.max(shown.content_size.x + sides)
                        },
                        above + shown.content_size.y + f32::from(margin.bottom),
                    ));
                    shown.inner
                })
                .inner
        });
        title_draft_end(ctx, window_id, self.title_rename.as_ref(), title_draft, title_finished);
        if toggle_compact {
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((self.id, "minimized")), compact.is_none()));
        }
        if back {
            crate::outside::want(ctx, window_id, false);
        }
        if let (Some(ratio), Some((head, width))) = (ratio, shape) {
            // the window takes the picture's shape once, at its width,
            // when the picture changes shape (another video): the user
            // sizes it freely after that
            let ratio_id = window_id.with("outside ratio");
            if ctx.data(|d| d.get_temp::<f32>(ratio_id)) != Some(ratio) {
                crate::outside::resize(ctx, window_id, Vec2::new(width, head + width / ratio), false);
                ctx.data_mut(|d| d.insert_temp(ratio_id, ratio));
            }
        }
        if let Some(content) = content {
            let size = ctx
                .data_mut(|d| d.get_persisted::<Vec2>(window_id.with(OUTSIDE_SIZE)))
                .unwrap_or(content);
            // (asked in points: at another zoom the same points are
            // another size of the window, to ask again)
            let zoom = ctx.zoom_factor();
            let asked = asked_at(ctx.data(|d| d.get_temp::<(Vec2, f32)>(asked_id)), zoom);
            if follows_content(size, content, asked) {
                crate::outside::resize(ctx, window_id, content + Vec2::new(side_extra(ctx, window_id), 0.0), true);
                ctx.data_mut(|d| d.insert_temp(asked_id, (content, zoom)));
            }
        }
        out
    }
}

/// The largest size of the ratio `ratio` (width over height) in `room`: a
/// picture of a kept shape in a window outside of any shape.
fn letterboxed(room: Vec2, ratio: f32) -> Vec2 {
    if room.x > room.y * ratio {
        Vec2::new(room.y * ratio, room.y)
    } else {
        Vec2::new(room.x, room.x / ratio)
    }
}

/// The size last asked of a window outside, in points, when it was asked
/// at the zoom `zoom`: at another the same points are another size.
fn asked_at(asked: Option<(Vec2, f32)>, zoom: f32) -> Option<Vec2> {
    asked.and_then(|(size, at)| (at == zoom).then_some(size))
}

/// How much taller than its room the content of a window outside may be
/// before it scrolls, in points.
const OUTSIDE_SLACK: f32 = 1.0;

/// The body of a window outside sized by its content: it scrolls where the
/// screen is too small for it, and only there. The system gives a window
/// whole pixels, a fraction of a point under or over the size asked: a
/// content that fraction taller than its room does not scroll, since the
/// scroll bar would take its strip from the content's width, a content
/// that wraps would be taller for it, and its window would follow it up
/// and down at every frame.
fn sized_body<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
    // (more room below, what is above staying where it is)
    ui.expand_to_include_y(ui.max_rect().bottom() + OUTSIDE_SLACK);
    egui::ScrollArea::vertical()
        .id_salt("floater_body")
        .auto_shrink([false, true])
        .show(ui, body)
}

/// Whether a window outside of the size `size` is asked to take the size
/// `content` of what it shows: when they differ, and once per size, since
/// the system may give less (a small screen) or nothing.
fn follows_content(size: Vec2, content: Vec2, asked: Option<Vec2>) -> bool {
    ((size.x - content.x).abs() > 1.0 || (size.y - content.y).abs() > 1.0) && asked != Some(content)
}

/// The room a window outside keeps at its right (`Floater::side_outside`):
/// the system's window is that much wider than the window, which keeps a
/// width of its own. The system sizes its window when it likes, some
/// frames after it is asked, less than asked (a small screen) or not at
/// all (a tiled or maximized window): the window's own width is the rest
/// of the system's only once that one has answered, and where the rest is
/// too narrow for the window, the window keeps the whole width and the
/// panel is drawn over its right end, as it is inside a viewer too small
/// for both.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SideRoom {
    /// The window's own width at this pass.
    own: f32,
    /// The window's own width without the room: what the system is asked
    /// for when the room goes.
    back: f32,
    /// The room's width in the size last asked, its gaps included.
    room: f32,
    /// The width asked of the system, until it has given it or the wait
    /// is over.
    asked: Option<SideAsk>,
    /// The system's width at the last pass.
    seen: f32,
    /// The zoom the widths are in: at another, the same points are
    /// another size.
    zoom: f32,
}

/// A width asked of the system for a window outside and its room.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SideAsk {
    width: f32,
    /// The system's width when it was asked: while it is still that one,
    /// the system has not answered.
    from: f32,
    since: f64,
}

impl SideRoom {
    /// The system has not answered the width asked: the window keeps the
    /// width it had, and its content does not jump to the width of both
    /// and back.
    fn waits(&self) -> bool {
        self.asked.is_some_and(|ask| (self.seen - ask.from).abs() <= 1.0)
    }
}

const SIDE_ROOM: &str = "outside side room";
const SIDE_RECT: &str = "outside side rect";
/// The gap between the window and its side panel, and around that one.
const SIDE_GAP: f32 = 8.0;
/// How long the system has to answer a width asked, in seconds: past it,
/// the window makes do with the width it has.
const SIDE_WAIT: f64 = 0.5;

/// One pass of a window outside that the system shows `native` wide and
/// that wants `room` at its right (0 for none), `min` being its own least
/// width: what to keep for the next pass, and the width to ask of the
/// system, once, when the room or the zoom changes.
fn side_room(was: Option<SideRoom>, native: f32, room: f32, min: f32, zoom: f32, now: f64) -> (Option<SideRoom>, Option<f32>) {
    let mut side = match was {
        Some(side) => side,
        None if room <= 0.0 => return (None, None),
        None => SideRoom {
            own: native,
            back: native,
            room: 0.0,
            asked: None,
            seen: native,
            zoom,
        },
    };
    let mut again = false;
    if side.zoom != zoom {
        // the same window, counted in other points: the room, as wide in
        // points as before, is asked again
        let scale = side.zoom / zoom;
        side.own *= scale;
        side.back *= scale;
        side.seen *= scale;
        side.asked = side.asked.map(|ask| SideAsk {
            width: ask.width * scale,
            from: ask.from * scale,
            since: ask.since,
        });
        side.zoom = zoom;
        again = side.room > 0.0;
    }
    let mut ask = None;
    if side.room != room || again {
        side.room = room;
        let width = if room > 0.0 { side.back.max(min) + room } else { side.back };
        side.asked = Some(SideAsk {
            width,
            from: native,
            since: now,
        });
        ask = Some(width);
    }
    let moved = (native - side.seen).abs() > 1.0;
    side.seen = native;
    // (a width that changes while one is asked is the system's answer,
    // in one step or several; otherwise the user's hand on the edge)
    let answer = side.asked.is_some_and(|ask| now - ask.since <= SIDE_WAIT);
    if side
        .asked
        .is_some_and(|ask| (native - ask.width).abs() <= 1.0 || now - ask.since > SIDE_WAIT)
    {
        side.asked = None;
    }
    if side.waits() {
        return (Some(side), ask);
    }
    if side.room <= 0.0 {
        return (None, ask);
    }
    side.own = if native - side.room >= min { native - side.room } else { native };
    if moved && !answer {
        side.back = side.own;
    }
    (Some(side), ask)
}

/// Where the side panel of a window outside is drawn in `whole`, the
/// system's window: beside the window where the room is there, over its
/// right end where it is not. None while the system has not answered.
fn side_rect(whole: egui::Rect, side: SideRoom) -> Option<egui::Rect> {
    if side.room <= 0.0 || side.waits() {
        return None;
    }
    let width = (side.room - 2.0 * SIDE_GAP).min(whole.width() - 2.0 * SIDE_GAP);
    (width > 0.0).then(|| {
        egui::Rect::from_min_max(
            egui::pos2(whole.right() - SIDE_GAP - width, whole.top() + SIDE_GAP),
            egui::pos2(whole.right() - SIDE_GAP, whole.bottom() - SIDE_GAP),
        )
    })
}

/// What the system's window of `window_id` has, or is asked to have,
/// beside the window's own width: a size asked of the window by its own
/// measure (`size_floater`, its content's) is asked of the system with it.
fn side_extra(ctx: &egui::Context, window_id: egui::Id) -> f32 {
    ctx.data(|d| d.get_temp::<SideRoom>(window_id.with(SIDE_ROOM)))
        .filter(|side| side.waits() || side.own < side.seen - 0.5)
        .map_or(0.0, |side| side.room)
}

/// The room the window `id` has at its right in its window outside
/// (`Floater::side_outside`), during its own pass there. None in the
/// viewer's window, where a panel sits beside the window itself, and
/// until the system has made the room.
pub(crate) fn side_room_outside(ctx: &egui::Context, id: &str) -> Option<egui::Rect> {
    let window_id = egui::Id::unique(id);
    floater_outside(ctx, window_id)
        .then(|| ctx.data(|d| d.get_temp::<egui::Rect>(window_id.with(SIDE_RECT))))
        .flatten()
}

/// The rows under a window's title: its states, with the text action at
/// their right end, then its one-line summary, with the icon button at its
/// right end. In the viewer's window and in the window outside alike.
#[allow(clippy::too_many_arguments)]
fn header_rows<'a>(
    ui: &mut egui::Ui,
    p: &Palette,
    states: &[(&'static str, String, Color32)],
    action: &mut Option<(&'a str, &'a str, &'a str, &'a mut Option<egui::Rect>)>,
    summary: Option<&egui::WidgetText>,
    summary_action: &mut Option<&'a mut bool>,
    summary_icon: &mut Option<(&'a str, &'a str, &'a mut bool)>,
    buttons: Option<&mut dyn FnMut(&mut egui::Ui)>,
) {
    let act = |ui: &mut egui::Ui, (icon, label, tip, clicked): &mut (&str, &str, &str, &mut Option<egui::Rect>)| {
        let action = ui
            .add(egui::Label::new(RichText::new(*label).size(12.0).color(p.violet_light)).sense(Sense::click()))
            .on_hover_text(*tip);
        if action.clicked() {
            **clicked = Some(action.rect);
        }
        glyph(ui, icon, p.violet_light, 13.0);
    };
    if !states.is_empty() || action.is_some() || buttons.is_some() {
        let status = |ui: &mut egui::Ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(7.0, 2.0);
                for (name, label, color) in states {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        glyph(ui, name, *color, 11.0);
                        ui.label(RichText::new(label).size(11.0).color(*color));
                    });
                }
            });
        };
        if let Some(buttons) = buttons {
            // a window outside the viewer's (`Floater::show_outside`): its
            // title bar's buttons end the row, then the action, and the
            // states take the room left
            const ROW: f32 = 20.0;
            ui.horizontal(|ui| {
                ui.set_min_height(ROW);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // (the buttons set their own spacing)
                    let spacing = ui.spacing().item_spacing;
                    buttons(ui);
                    ui.spacing_mut().item_spacing = spacing;
                    if let Some(action) = action {
                        ui.add_space(4.0);
                        act(ui, action);
                    }
                    // (as tall as the row: centred in it at a lesser height,
                    // a line of states taller than asked would grow under it)
                    let size = Vec2::new(ui.available_width().max(1.0), ROW);
                    ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), status);
                });
            });
        } else if let Some(action) = action {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 65.0).max(1.0);
                ui.allocate_ui_with_layout(Vec2::new(width, 18.0), egui::Layout::top_down(egui::Align::Min), status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| act(ui, action));
            });
        } else {
            status(ui);
        }
    }
    if let Some(summary) = summary {
        let full_text = summary.text().to_owned();
        let sense = if summary_action.is_some() { Sense::click() } else { Sense::hover() };
        let mut summary_row = |ui: &mut egui::Ui| {
            if ui
                .add(egui::Label::new(summary.clone()).truncate().sense(sense))
                .on_hover_text(&full_text)
                .clicked()
                && let Some(clicked) = summary_action
            {
                **clicked = true;
            }
        };
        if let Some((icon, tip, clicked)) = summary_icon {
            ui.horizontal(|ui| {
                let size = Vec2::splat(18.0);
                let width = (ui.available_width() - size.x - ui.spacing().item_spacing.x).max(1.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(width, size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    &mut summary_row,
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let glyph_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0));
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(egui::Id::unique(("header_summary_icon", *icon)), glyph_rect));
                    if let Some(texture) = crate::ui::icons::global(icon) {
                        ui.painter().image(
                            texture.id(),
                            glyph_rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            p.violet_light,
                        );
                    }
                    let response = response.on_hover_text(*tip);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, *tip));
                    if response.clicked() {
                        **clicked = true;
                    }
                });
            });
        } else {
            summary_row(ui);
        }
    }
}

/// The title bar's button that sends a window outside the viewer's, and
/// the one that brings it back.
const OUTSIDE_ICON: &str = "app-window-filled";
const INSIDE_ICON: &str = "picture-in-picture-filled";
/// The size a window had outside the viewer's, kept with the layout.
pub(crate) const OUTSIDE_SIZE: &str = "outside_size";
/// The windows that never leave the viewer's. The Dev Toolbox: its tools
/// read the world under the pointer of the viewer's window and draw over
/// it.
const STAYS_INSIDE: &[&str] = &[super::dev_toolbox::WINDOW];

/// The window `id` is drawn in its window outside the viewer's.
pub(crate) fn floater_outside(ctx: &egui::Context, window_id: egui::Id) -> bool {
    crate::outside::wanted(ctx, window_id) && crate::outside::native(ctx, window_id)
}

/// The size the window `id` was last drawn at, in the viewer's window or
/// in its own outside.
pub fn floater_size(ctx: &egui::Context, id: &str) -> Option<Vec2> {
    let window_id = egui::Id::unique(id);
    if floater_outside(ctx, window_id) {
        return ctx.data_mut(|d| d.get_persisted::<Vec2>(window_id.with(OUTSIDE_SIZE)));
    }
    egui::AreaState::load(ctx, window_id).and_then(|state| state.size)
}

/// The size of the floater `id` expanded: while it is minimized, the size
/// it comes back to (LLFloater::setMinimized keeps the expanded geometry),
/// not its title bar's.
pub fn floater_expanded_size(ctx: &egui::Context, id: &str) -> Option<Vec2> {
    let window_id = egui::Id::unique(id);
    if !floater_outside(ctx, window_id) && floater_minimized(ctx, id) {
        return ctx.data_mut(|d| d.get_persisted::<Vec2>(window_id.with("expanded_size")));
    }
    floater_size(ctx, id)
}

/// A window without a title bar, dragged by its whole surface (the
/// mini-map): `frame` around `body`, which fills it. Like a floater it is
/// placed by the layout (`layout::Slot`), snaps to the other windows when
/// moved with Alt or resized, and is resized by the same edge handles.
/// `locked`: the user can neither move nor resize it (it still follows the
/// viewer's edges and the group it is glued to). `keep_in_mouselook`: stays
/// visible when entering mouselook (the mini-map, `LLAgent::endAnimationUpdateUI`).
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
    keep_in_mouselook: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let window_id = egui::Id::unique(id);
    let role = if keep_in_mouselook { HideKeep::Mouselook } else { HideKeep::None };
    if window_hidden(ctx, window_id, true, role) {
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
    if super::layout::take_reset(ctx, window_id) {
        // "Reset window positions and sizes": back to its default rect
        super::layout::reanchor(ctx, window_id);
        w = w.current_pos(default.min).fixed_size(default.size());
    } else if let Some(pos) = slot.pos {
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
/// Which windows stay shown when [`hide_windows`] hides the interface:
/// immersive mode (#859) keeps the chat windows; mouselook keeps the mini-map
/// (`LLAgent::endAnimationUpdateUI`, indra/newview/llagent.cpp).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum HideKeep {
    #[default]
    None,
    Chat,
    Mouselook,
}

/// LLFloaterView::hideAllFloaters / showHiddenFloaters): those open on the
/// frame it was hidden. A window opened afterwards shows; one closed in the
/// meantime is forgotten.
#[derive(Clone, Default)]
struct HiddenWindows {
    /// The frame whose open windows are hidden.
    capture: Option<u64>,
    ids: std::collections::HashSet<egui::Id>,
    keep: HideKeep,
}

fn hidden_windows_id() -> egui::Id {
    egui::Id::unique("aurora_hidden_windows")
}

/// The interface is hidden (`hide_windows`).
pub fn interface_hidden(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<HiddenWindows>(hidden_windows_id()).is_some())
}

/// Hide the windows open now (on the next frame), or show them back; `keep`
/// leaves the chat windows (immersive mode) or the mini-map (mouselook).
/// Called between two frames.
pub fn hide_windows(ctx: &egui::Context, hide: bool, keep: HideKeep) {
    let state = HiddenWindows {
        capture: hide.then(|| ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT)),
        ids: Default::default(),
        keep,
    };
    ctx.data_mut(|d| {
        if hide {
            d.insert_temp(hidden_windows_id(), state);
        } else {
            d.remove::<HiddenWindows>(hidden_windows_id());
        }
    });
}

/// The window `id` (`open` or not, with its `role`) is kept hidden this frame.
fn window_hidden(ctx: &egui::Context, id: egui::Id, open: bool, role: HideKeep) -> bool {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| {
        let Some(mut state) = d.get_temp::<HiddenWindows>(hidden_windows_id()) else {
            return false;
        };
        if role != HideKeep::None && state.keep == role {
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

/// "Reset window positions and sizes" (Preferences › Layout): every window
/// back to its default place and size, expanded, without touching a
/// setting (`layout::reset_windows`). The groups of preview windows forget
/// the rect they open at, and the Performance window the one of its full
/// view.
pub fn reset_windows(ctx: &egui::Context) {
    super::layout::reset_windows(ctx);
    cascade::forget_saved(ctx);
    super::perf::forget_monitor_rect(ctx);
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
    if floater_outside(ctx, window_id) {
        // (its own size: the room its window keeps at its right stays)
        crate::outside::resize(ctx, window_id, size + Vec2::new(side_extra(ctx, window_id), 0.0), false);
        return;
    }
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

/// The floater `id` is the front window (LLFloater::isFrontmost): the topmost
/// visible layer that is not the background.
pub fn floater_is_front(ctx: &egui::Context, id: &str) -> bool {
    let want = egui::Id::unique(id);
    // in its window outside the viewer's: in front with the system's focus
    if crate::outside::native(ctx, want) {
        return crate::outside::in_focus(ctx, want);
    }
    ctx.memory(|m| {
        m.layer_ids()
            .filter(|layer| layer.order != egui::Order::Background && m.areas().is_visible(layer))
            .last()
            .is_some_and(|layer| layer.id == want)
    })
}

/// The keyboard is inside the floater `id` (LLFloater::hasFocus).
pub fn floater_holds_keyboard(ctx: &egui::Context, id: &str) -> bool {
    let want = egui::Id::unique(id);
    // (the keys of a window outside with the focus are its own)
    if crate::outside::native(ctx, want) {
        return crate::outside::in_focus(ctx, want);
    }
    let Some(focus) = ctx.memory(|m| m.focused()) else {
        return false;
    };
    ctx.read_response(focus).is_some_and(|response| response.layer_id.id == want)
}

/// Bring the floater `id` to the front, expanding it when it is minimized
/// (LLFloater::setVisibleAndFrontmost).
pub fn bring_floater_front(ctx: &egui::Context, id: &str) {
    if crate::outside::native(ctx, egui::Id::unique(id)) {
        crate::outside::raise(ctx, egui::Id::unique(id));
    } else if floater_minimized(ctx, id) {
        expand_floater(ctx, id);
    } else {
        to_front(ctx, id);
    }
}

/// The floater `id` inside comes to the front: now, and again when it is
/// drawn. A button in a window outside the viewer's runs in that window's
/// pass, where `move_to_top` orders that window's layers, not the viewer's.
fn to_front(ctx: &egui::Context, id: &str) {
    ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)));
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(id).with(FRONT), true));
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
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique(id).with(EXPAND), true);
        d.remove_temp::<bool>(egui::Id::unique(id).with(QUIET));
    });
    to_front(ctx, id);
}

/// Will the floater `id` change its minimized state at its next frame for a
/// reason other than its title button: an expand asked for
/// (`expand_floater`) or a "Reset window positions and sizes" not yet
/// applied to it.
pub fn floater_expand_pending(ctx: &egui::Context, id: &str) -> bool {
    floater_expand_asked(ctx, id) || super::layout::reset_pending(ctx, egui::Id::unique(id))
}

/// A button, link or shortcut asked the floater `id` to expand
/// (`expand_floater`) and it has not drawn since: not its title button, not
/// a window reset.
pub fn floater_expand_asked(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(id).with(EXPAND))).unwrap_or(false)
}

/// Drops an expand asked for by `expand_floater` that the floater `id` has
/// not shown yet (a window reset's is left).
pub fn cancel_expand(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(id).with(EXPAND)));
}

/// The floater `id` is in its window outside the viewer's and not in front
/// there: another program's window may cover it, which no window inside
/// the viewer's can be. Its toolbar button, menu entry or shortcut then
/// brings it in front instead of closing it unseen, and closes it at the
/// next press: the rule of LLFloaterReg::toggleInstanceOrBringToFront
/// (indra/llui/llfloaterreg.cpp) for a floater without the focus. Not in
/// the Legacy Viewer, which has no window outside; a window inside keeps
/// its toggle.
fn outside_behind(ctx: &egui::Context, id: &str) -> bool {
    let window = egui::Id::unique(id);
    crate::outside::native(ctx, window) && !crate::outside::shows_in_front(ctx, window)
}

/// The toolbar button, menu entry or shortcut of the floater `id`
/// (LLFloaterReg::toggleInstance): a floater shown closes, a minimized one
/// expands, a closed one opens expanded. LLFloater::openFloater and
/// closeFloater both unminimize, so a window closed while minimized does not
/// come back as a bar. In its window outside and behind, it comes in front
/// ([`outside_behind`]).
pub fn toggle_floater(ctx: &egui::Context, id: &str, open: &mut bool) {
    shown_with_keyboard(ctx, id);
    if *open && outside_behind(ctx, id) {
        bring_floater_front(ctx, id);
        return;
    }
    if *open && floater_minimized(ctx, id) {
        expand_floater(ctx, id);
        return;
    }
    *open = !*open;
    if !*open {
        forget_keyboard(ctx, id);
    }
    unminimize_quietly(ctx, id);
}

/// The toolbar button of a floater the app opens itself (its opening loads
/// or resets something), as LLFloaterReg::toggleInstance: a minimized one
/// expands, a shown one closes as its cross would ([`close_floater`]), and
/// true asks the caller to open a closed one.
#[must_use]
pub fn toggle_floater_closing(ctx: &egui::Context, id: &str, open: bool) -> bool {
    shown_with_keyboard(ctx, id);
    if open && outside_behind(ctx, id) {
        bring_floater_front(ctx, id);
    } else if open && floater_minimized(ctx, id) {
        expand_floater(ctx, id);
    } else if open {
        close_floater(ctx, id);
        forget_keyboard(ctx, id);
    }
    !open
}

/// Close the floater `id` as its cross does, so that the window's own close
/// runs (its questions, its cleanup): on its next frame drawn, this one or
/// the next. A request for a window not drawn by then is dropped.
pub fn close_floater(ctx: &egui::Context, id: &str) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(id).with(CLOSE), frame));
}

/// The window `window` (a floater's `egui::Id`) is asked to close, as
/// [`close_floater`] asks by name: the cross of its window outside the
/// viewer's.
pub(crate) fn close_window(ctx: &egui::Context, window: egui::Id) {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| d.insert_temp(window.with(CLOSE), frame));
}

/// A close asked of the window `window` still waits when its window is
/// drawn again: for one whose code does not run for a while
/// (`outside::keep`), where a request is otherwise dropped after a frame.
/// True while one waits.
pub(crate) fn keep_close_asked(ctx: &egui::Context, window: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| {
        let waits = d.get_temp::<u64>(window.with(CLOSE)).is_some();
        if waits {
            d.insert_temp(window.with(CLOSE), frame);
        }
        waits
    })
}

/// The floater `id` was asked to close ([`close_floater`]), once.
fn close_asked(ctx: &egui::Context, id: &str) -> bool {
    let frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    ctx.data_mut(|d| d.remove_temp::<u64>(egui::Id::unique(id).with(CLOSE)))
        .is_some_and(|at| at + 1 >= frame)
}

/// Open the floater `id` (LLFloaterReg::showInstance): expanded, whether it
/// was closed or minimized.
pub fn open_floater(ctx: &egui::Context, id: &str, open: &mut bool) {
    shown_with_keyboard(ctx, id);
    open_expanded(ctx, id, open);
}

/// [`open_floater`] without a word on the keyboard.
fn open_expanded(ctx: &egui::Context, id: &str, open: &mut bool) {
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

/// Show the floater `id` from a button or a link, as
/// LLFloaterReg::showInstance → LLFloater::openFloater does: a closed one
/// opens, an open one comes to the front, expanded when it is minimized
/// (setMinimized(false), setVisibleAndFrontmost). Its own code learns that
/// it was shown ([`take_floater_shown`]): LLFloater::onOpen runs on every
/// show, the floater open or not.
pub fn show_floater(ctx: &egui::Context, id: &str, open: &mut bool) {
    let was_open = *open;
    open_floater(ctx, id, open);
    if was_open {
        bring_floater_front(ctx, id);
    }
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(id).with(SHOWN), true));
}

/// Show the floater `id` for something that must not take the keyboard
/// (llMapDestination without BEACON_FOCUS_MAP: process_script_teleport_request
/// sets the floater's auto-focus off before LLFloater::openFloater, whose
/// setVisibleAndFrontmost then leaves the focus where it is): as
/// [`show_floater`], but a window outside the viewer's is not raised, the
/// system giving the focus to the window it raises; it only expands when
/// minimized. Its own code learns it ([`take_floater_quiet`]) until a show,
/// open, toggle or expand that is not quiet replaces it.
pub fn show_floater_quietly(ctx: &egui::Context, id: &str, open: &mut bool) {
    let was_open = *open;
    open_expanded(ctx, id, open);
    // (`open_expanded` has expanded it when minimized)
    if was_open && !crate::outside::native(ctx, egui::Id::unique(id)) {
        bring_floater_front(ctx, id);
    }
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::unique(id).with(SHOWN), true);
        d.insert_temp(egui::Id::unique(id).with(QUIET), true);
    });
}

/// The last show of the floater `id` was quiet ([`show_floater_quietly`]):
/// true once.
pub fn take_floater_quiet(ctx: &egui::Context, id: &str) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(id).with(QUIET)))
        .unwrap_or(false)
}

/// The last show of the floater `id` was quiet, and it stays known: for an
/// expand that waits.
pub fn floater_shown_quietly(ctx: &egui::Context, id: &str) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(id).with(QUIET))).unwrap_or(false)
}

/// A show, open or toggle that may take the keyboard replaces a quiet one,
/// and its own code learns of it ([`take_floater_keyboard`]).
fn shown_with_keyboard(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| {
        d.remove_temp::<bool>(egui::Id::unique(id).with(QUIET));
        d.insert_temp(egui::Id::unique(id).with(KEYBOARD), true);
    });
}

/// A toggle that closes the floater gives it no keyboard: a quiet show
/// after it stays quiet.
fn forget_keyboard(ctx: &egui::Context, id: &str) {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(id).with(KEYBOARD)));
}

/// A show, open or toggle of the floater `id` that may take the keyboard
/// came since the last call, a quiet show after it or not: true once. Its
/// keyboard is given at once (LLFloater::openFloater, onOpen), and a quiet
/// show after it takes nothing back.
pub fn take_floater_keyboard(ctx: &egui::Context, id: &str) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(id).with(KEYBOARD)))
        .unwrap_or(false)
}

/// The floater `id` was shown by [`show_floater`] or
/// [`show_floater_quietly`] since the last call: true once.
pub fn take_floater_shown(ctx: &egui::Context, id: &str) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(id).with(SHOWN)))
        .unwrap_or(false)
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
pub(crate) const EXPAND: &str = "expand";

/// Temporary frame of a floater asked to close (`close_floater`).
const CLOSE: &str = "close";

/// Temporary flag of a floater shown by a button or a link (`show_floater`).
const SHOWN: &str = "shown";

/// Temporary flag of a floater shown without the keyboard
/// (`show_floater_quietly`).
const QUIET: &str = "quiet";

/// Temporary flag of a floater shown, opened or toggled by the user
/// (`shown_with_keyboard`).
const KEYBOARD: &str = "keyboard";

/// Temporary flag of a floater asked to the front (`bring_floater_front`).
const FRONT: &str = "front";

/// Title-bar minimize / expand icon (#3084). Ordinary floaters: minus while
/// expanded, square while minimized. Compact bodies (Performance) keep the
/// arrows-maximize / arrows-minimize pair.
fn minimize_title_icon(compact: bool, minimized: bool) -> &'static str {
    match (compact, minimized) {
        (true, true) => "arrows-maximize",
        (true, false) => "arrows-minimize",
        (false, true) => "square",
        (false, false) => "minus",
    }
}

/// Title bar button with a Tabler icon (drawn by hand if it is missing).
/// The close cross of a window without a title bar (`Floater::headless`):
/// a small box of the window's surface against its right edge, at its top,
/// drawn just above the window; true when it is clicked.
fn headless_close(
    ctx: &egui::Context,
    p: &Palette,
    window: egui::Id,
    shown: &egui::Response,
    fill: Color32,
    opacity: f32,
    order: egui::Order,
) -> bool {
    const SIZE: f32 = 22.0;
    // its left border lies on the window's right border
    let pos = egui::pos2(shown.rect.right() - FLOATER_STROKE, shown.rect.top());
    let area = egui::Area::new(window.with("headless_close"))
        .order(order)
        .fixed_pos(pos)
        .movable(false)
        .constrain(false)
        .fade_in(false)
        .show(ctx, |ui| {
            let glass = ui.painter().add(egui::Shape::Noop);
            let (rect, response) = ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click());
            let radius = ctx.global_style().visuals.window_corner_radius;
            // square against the window, rounded like it outside
            let corners = egui::CornerRadius { nw: 0, sw: 0, ..radius };
            ui.painter().rect(
                rect,
                corners,
                fill,
                border_stroke(p, Stroke::new(FLOATER_STROKE, p.muted_dim.gamma_multiply(0.6))),
                egui::StrokeKind::Inside,
            );
            workspace::finish_glass_faded(ui.painter(), glass, rect, p, f32::from(radius.ne), opacity);
            paint_title_button(ui, p, "x", &response);
            response.on_hover_text(aurora_i18n::tr!("common-close")).clicked()
        });
    ctx.set_sublayer(shown.layer_id, area.response.layer_id);
    area.inner
}

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

/// The room a `divider` takes across its line.
const DIVIDER_ROOM: f32 = 6.0;

/// Thin line between two zones of a window, across the layout (horizontal in
/// a column, vertical in a row), in the room of egui's separator: a window
/// keeps its layout. Not for buttons or toolbar items: their spacing and the
/// pill of the hovered one tell them apart. A menu uses `menu::separator`.
pub fn divider(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let across = !ui.layout().main_dir().is_horizontal();
    let available = if ui.is_sizing_pass() {
        Vec2::ZERO
    } else {
        ui.available_size_before_wrap()
    };
    let size = if across {
        Vec2::new(available.x, DIVIDER_ROOM)
    } else {
        Vec2::new(DIVIDER_ROOM, available.y)
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

/// A state written over a picture ("Multiple"), on a backing of its own:
/// the picture under it may be as light as the text.
pub(crate) fn picture_label(ui: &egui::Ui, p: &Palette, rect: egui::Rect, text: &str, size: f32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), p.ink);
    let at = egui::Rect::from_center_size(rect.center(), galley.size());
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    painter.rect_filled(at.expand2(egui::vec2(5.0, 2.0)), inner_radius(ui.visuals()), p.bar);
    painter.galley(at.min, galley, p.ink);
}

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

pub const TEXT_BUTTON_HEIGHT: f32 = 18.0;

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

/// [`switch_labeled`] in a narrow column: the label wraps onto more lines
/// instead of running past the column's edge (a long translation, a larger
/// font), the switch beside its first line. It wraps within what is shown:
/// a wider row above can stretch the column past its clip.
pub fn switch_labeled_wrapped(ui: &mut egui::Ui, p: &Palette, on: &mut bool, label: impl Into<egui::WidgetText>) -> egui::Response {
    let room = ui.available_width().min(ui.clip_rect().right() - ui.cursor().left()).max(0.0);
    ui.horizontal_top(|ui| {
        ui.set_max_width(room);
        let mut resp = switch(ui, p, on);
        let text = ui.add(egui::Label::new(label).wrap().sense(Sense::click()));
        if text.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        resp.union(text)
    })
    .inner
}

/// Custom-painted controls follow an explicit font override in their UI scope.
pub(crate) fn control_font(ui: &egui::Ui, default_size: f32) -> egui::FontId {
    ui.style()
        .override_font_id
        .clone()
        .unwrap_or_else(|| egui::FontId::proportional(default_size))
}

/// Flat tab strip. Returns true when the selection changed.
pub fn tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, false, false).0
}

/// Keep tab labels whole by starting another row when their measured
/// widths do not fit. Selection still uses the original tab index.
pub fn wrapping_tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    let widths = tab_widths(ui, labels, &control_font(ui, 12.0));
    let available = ui.available_width();
    let mut changed = false;
    let mut start = 0;
    while start < labels.len() {
        let mut end = start + 1;
        let mut width = widths[start];
        while end < labels.len() && width + TAB_GAP + widths[end] <= available {
            width += TAB_GAP + widths[end];
            end += 1;
        }
        let mut index = if (start..end).contains(selected) {
            *selected - start
        } else {
            usize::MAX
        };
        if tabs(ui, p, &mut index, &labels[start..end]) {
            *selected = start + index;
            changed = true;
        }
        start = end;
    }
    changed
}

/// The flat tab strip, with the rect of each tab (a tab that opens when
/// something is dragged over it).
pub fn tabs_at(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> (bool, Vec<egui::Rect>) {
    tabs_row(ui, p, selected, labels, false, false)
}

/// The same tab strip at the density of fixed inspector controls.
pub fn compact_tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, false, true).0
}

/// The compact tab strip, with the rect of each tab.
pub fn compact_tabs_at(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> (bool, Vec<egui::Rect>) {
    tabs_row(ui, p, selected, labels, false, true)
}

/// Equal-width tabs spanning a data inspector, with the usual theme and input handling.
pub fn stretched_tabs(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, labels: &[(&str, bool)]) -> bool {
    tabs_row(ui, p, selected, labels, true, false).0
}

const TAB_GAP: f32 = 4.0;

fn tab_widths(ui: &egui::Ui, labels: &[(&str, bool)], font: &egui::FontId) -> Vec<f32> {
    labels
        .iter()
        .map(|(label, _)| ui.fonts_mut(|f| f.layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER).size().x) + 18.0)
        .collect()
}

fn tabs_row(
    ui: &mut egui::Ui,
    p: &Palette,
    selected: &mut usize,
    labels: &[(&str, bool)],
    stretched: bool,
    compact: bool,
) -> (bool, Vec<egui::Rect>) {
    let mut changed = false;
    let mut rects = Vec::with_capacity(labels.len());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = TAB_GAP;
        let n = labels.len().max(1) as f32;
        let font = control_font(ui, 12.0);
        let minimum_height = if compact { 22.0 } else { 30.0 };
        let height = if ui.style().override_font_id.is_some() {
            (ui.fonts_mut(|f| f.row_height(&font)) + if compact { 8.0 } else { 12.0 }).max(minimum_height)
        } else {
            minimum_height
        };
        let text_w = tab_widths(ui, labels, &font);
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
            rects.push(rect);
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
    (changed, rects)
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
/// look), in `raised` otherwise (a legacy look). The lit face slides from
/// the segment chosen before, as the poser's tracks do (`tracks::icon_track`;
/// none under Reduce animations). Returns true when the selection changed.
pub fn segmented(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, options: &[(&str, bool)]) -> bool {
    segmented_track(ui, p, selected, options, &[], SEGMENT_H)
}

/// `segmented` with a track `height` tall, to line it up with the fields
/// and buttons of a taller row.
pub fn segmented_tall(ui: &mut egui::Ui, p: &Palette, selected: &mut usize, options: &[(&str, bool)], height: f32) -> bool {
    segmented_track(ui, p, selected, options, &[], height)
}

/// `segmented` with some options out of reach: `disabled[i]` is the tooltip
/// that says why option `i` cannot be chosen (None, or past the end of the
/// slice: it can). A disabled option is drawn dimmed, takes no hover or click.
pub fn segmented_with_disabled(
    ui: &mut egui::Ui,
    p: &Palette,
    selected: &mut usize,
    options: &[(&str, bool)],
    disabled: &[Option<&str>],
) -> bool {
    segmented_track(ui, p, selected, options, disabled, SEGMENT_H)
}

fn segmented_track(
    ui: &mut egui::Ui,
    p: &Palette,
    selected: &mut usize,
    options: &[(&str, bool)],
    disabled: &[Option<&str>],
    height: f32,
) -> bool {
    let galleys = segment_galleys(ui, options);
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + SEGMENT_PAD_X).collect();
    let size = Vec2::new(segmented_size(ui, options).x, height);
    let (track, _) = ui.allocate_exact_size(size, Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(track, radius, p.bar);
    let off = |i: usize| disabled.get(i).copied().flatten();
    let mut rects = Vec::with_capacity(widths.len());
    let mut x = track.left() + SEGMENT_PAD;
    for w in &widths {
        rects.push(egui::Rect::from_min_size(
            egui::pos2(x, track.top() + SEGMENT_PAD),
            Vec2::new(*w, track.height() - SEGMENT_PAD * 2.0),
        ));
        x += w;
    }
    // the lit face, its edges sliding from the segment chosen before (kept
    // from the track's left, so that moving the window does not slide it);
    // none on a chosen option that is out of reach
    if let Some(chosen) = rects.get(*selected).filter(|_| off(*selected).is_none()) {
        let id = ui.scope_id().with(("segmented-slide", options.len(), options.first().map(|o| o.0)));
        let time = ui.style().animation_time;
        let left = ui
            .ctx()
            .animate_value_with_time(id.with("left"), chosen.left() - track.left(), time);
        let right = ui
            .ctx()
            .animate_value_with_time(id.with("right"), chosen.right() - track.left(), time);
        let face = if options[*selected].1 { p.violet } else { p.raised };
        let lit = egui::Rect::from_x_y_ranges(track.left() + left..=track.left() + right, chosen.y_range());
        ui.painter().rect_filled(lit, inner_radius(ui.visuals()), face);
    }
    let mut changed = false;
    for (i, ((label, accent), galley)) in options.iter().zip(galleys).enumerate() {
        let rect = rects[i];
        let off = off(i);
        let resp = ui.interact(
            rect,
            ui.scope_id().with(("segmented", i, *label)),
            if off.is_some() { Sense::hover() } else { Sense::click() },
        );
        let sel = *selected == i;
        let text = match (sel, *accent) {
            _ if off.is_some() => p.muted_dim,
            (true, true) => p.on_accent,
            (true, false) => p.ink,
            (false, _) if resp.hovered() => p.ink,
            (false, _) => p.muted,
        };
        if !sel && off.is_none() && resp.hovered() {
            ui.painter()
                .rect_filled(rect, inner_radius(ui.visuals()), p.raised.gamma_multiply(0.5));
        }
        let at = letters_in_middle(ui.painter(), &galley, rect);
        ui.painter().galley(at, galley, text);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, off.is_none(), sel, *label));
        if let Some(tip) = off {
            resp.on_hover_text(tip);
        } else if resp.clicked() && !sel {
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

pub fn search_field(ui: &mut egui::Ui, p: &Palette, text: &mut String, hint: &str, width: f32) -> egui::Response {
    search_text_edit(ui, p, text, |edit| edit.hint_text(hint).desired_width(width))
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

/// The A–Z switch of a list drawn from the inventory: lit while `by_name`,
/// off for the list's own order, `tip` saying which and what a click does;
/// flips `by_name` and returns true when clicked.
pub fn sort_by_name_toggle(ui: &mut egui::Ui, p: &Palette, by_name: &mut bool, tip: &str) -> bool {
    let clicked = icon_toggle(ui, p, "sort-ascending-letters", tip, *by_name);
    if clicked {
        *by_name = !*by_name;
    }
    clicked
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
    text_button(ui, p.ink, label, None)
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
    text_button(ui, p.ink, label, Some(size)).on_hover_text(label)
}

/// The initial Enter action of a confirmation with no text field. Give its
/// visible default real focus so the first Tab advances from that button.
pub fn default_button_sized(ui: &mut egui::Ui, p: &Palette, label: &str, size: Vec2) -> egui::Response {
    let hint = modal_keys::show_default_hint(ui);
    ui.scope(|ui| {
        if hint {
            let active = ui.visuals().widgets.active;
            ui.visuals_mut().widgets.inactive = active;
            ui.visuals_mut().widgets.hovered = active;
        }
        let ink = if hint { p.on_accent } else { p.ink };
        let mut response = text_button(ui, ink, label, Some(size)).on_hover_text(label);
        // Keep the modal's opening-Enter guard when its default gains focus:
        // egui otherwise turns held-key repeats into focused-button clicks.
        if ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        repeat: true,
                        ..
                    }
                )
            })
        }) {
            response.flags.remove(egui::response::Flags::FAKE_PRIMARY_CLICKED);
        }
        if hint && response.enabled() && ui.memory(|m| m.allows_interaction(response.layer_id)) {
            response.request_focus();
        }
        modal_keys::default_button(response)
    })
    .inner
}

fn text_button(ui: &mut egui::Ui, ink: Color32, label: &str, size: Option<Vec2>) -> egui::Response {
    // Keep the button in the parent's layout so wrapped toolbars can break
    // before it. A child UI would wrap internally and enlarge narrow windows.
    // The theme gives every button its plain filled face.
    let button = egui::Button::new(RichText::new(label).font(control_font(ui, 12.0)).color(ink));
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

/// One button of an `index_buttons` row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexState {
    /// Greyed: nothing to pick there.
    Disabled,
    Off,
    /// Picked for part of what it stands for: a wash of the accent.
    Mixed,
    On,
}

/// A row of numbered buttons 0, 1, 2… sharing `size`, each with its state,
/// as tight as the row: the flat face of `action_button`, the accent when
/// on. Returns the index clicked; `tip` gives each one's tooltip.
pub fn index_buttons(
    ui: &mut egui::Ui,
    p: &Palette,
    size: Vec2,
    states: &[IndexState],
    tip: impl Fn(usize) -> String,
) -> (Option<usize>, Vec<egui::Rect>) {
    let base = ui.next_auto_id();
    let (row, _) = ui.allocate_exact_size(size, Sense::hover());
    let count = states.len().max(1) as f32;
    let gap = if row.width() / count >= 12.0 { 2.0 } else { 1.0 };
    let cell = ((row.width() - gap * (count - 1.0)) / count).max(1.0);
    let font = control_font(ui, (cell * 1.45).clamp(7.0, 12.5).min(row.height() * 0.7));
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let enabled_ui = ui.is_enabled();
    let mut clicked = None;
    let mut rects = Vec::with_capacity(states.len());
    for (index, state) in states.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(row.left() + index as f32 * (cell + gap), row.top()),
            Vec2::new(cell, row.height()),
        );
        rects.push(rect);
        let enabled = enabled_ui && *state != IndexState::Disabled;
        let resp = ui.interact(
            rect,
            base.with(("index-button", index)),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        let hot = enabled && resp.hovered();
        let (fill, ink) = match state {
            IndexState::On => (if hot { p.violet_light } else { p.violet }, p.on_accent),
            IndexState::Mixed => (p.violet.gamma_multiply(if hot { 0.5 } else { 0.32 }), p.ink),
            IndexState::Off | IndexState::Disabled => (if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised }, p.ink),
        };
        let fade = if enabled { 1.0 } else { 0.45 };
        ui.painter().rect_filled(rect, radius, fill.gamma_multiply(fade));
        if p.high_contrast && matches!(state, IndexState::Off | IndexState::Disabled) {
            let edge = if hot { p.ink } else { p.muted_dim };
            ui.painter()
                .rect_stroke(rect, radius, Stroke::new(1.0, edge.gamma_multiply(fade)), egui::StrokeKind::Inside);
        }
        let label = index.to_string();
        let galley = ui.painter().layout_no_wrap(label.clone(), font.clone(), ink.gamma_multiply(fade));
        let origin = row_text_origin(ui.painter(), &galley, rect.center().x - galley.size().x / 2.0, rect.center().y);
        ui.painter().galley(origin, galley, ink.gamma_multiply(fade));
        if hot {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let on = *state == IndexState::On;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, &label));
        if resp.on_hover_text(tip(index)).clicked() {
            clicked = Some(index);
        }
    }
    (clicked, rects)
}

/// An inactive action face with a live spinner and readable work status.
pub fn busy_button(ui: &mut egui::Ui, p: &Palette, label: &str) -> egui::Response {
    const SPINNER: f32 = 14.0;
    const GAP: f32 = 7.0;
    let galley = ui.painter().layout_no_wrap(label.to_owned(), control_font(ui, 12.0), p.ink);
    let content = SPINNER + GAP + galley.size().x;
    let size = Vec2::new(content + KEYED_PAD, ui.spacing().interact_size.y);
    let response = action_button_if(ui, p, "", "", ActionKind::Flat, size, false);
    let left = response.rect.center().x - content / 2.0;
    egui::Spinner::new().color(p.violet_light).paint_at(
        ui,
        egui::Rect::from_center_size(egui::pos2(left + SPINNER / 2.0, response.rect.center().y), Vec2::splat(SPINNER)),
    );
    let origin = row_text_origin(ui.painter(), &galley, left + SPINNER + GAP, response.rect.center().y);
    ui.painter().galley(origin, galley, p.ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, false, label));
    response
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

/// A dashed outline in `rect`: a place to drop something or a card that
/// adds one (the profile's Give zone, the photo filters' drop zones).
pub fn dashed_outline(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let r = rect.shrink(0.75);
    let path = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    painter.extend(egui::Shape::dashed_line(&path, egui::Stroke::new(1.5, color), 5.0, 4.0));
}

/// A place number that turns over in `rect` (the contact sets' tiles and
/// the photo filters' cards while one is held): `before` slides up and
/// fades as `now` rises in, `turn` going from 0 to 1, eased out with a
/// touch of overshoot as it lands.
pub fn turning_number(ui: &egui::Ui, rect: egui::Rect, now: usize, before: Option<usize>, turn: f32, color: Color32) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let font = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), rect.height() * 0.5, color).font_id;
    let k = turn.clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - k).powi(3);
    let travel = rect.height() * 0.6;
    if let Some(before) = before.filter(|_| k < 1.0) {
        let g = painter.layout_no_wrap(aurora_i18n::integer(before as i64), font.clone(), color);
        let at = rect.center() - Vec2::new(0.0, travel * e);
        paint_ink_centered(&painter, g, at, color.gamma_multiply(1.0 - e));
    }
    let g = painter.layout_no_wrap(aurora_i18n::integer(now as i64), font, color);
    let at = rect.center() + Vec2::new(0.0, travel * (1.0 - e));
    paint_ink_centered(&painter, g, at, color.gamma_multiply(e.max(0.15)));
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

/// A row of a window's rail (Places, Appearance's showcase): icon, name,
/// count; `depth` indents a sub-folder. The chosen row takes the accent's
/// wash, a hovered one the raised face.
#[allow(clippy::too_many_arguments)]
pub fn rail_row(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    tint: Color32,
    label: &str,
    count: Option<usize>,
    on: bool,
    depth: usize,
) -> egui::Response {
    rail_tree_row(ui, ui.next_auto_id(), p, icon, tint, label, count, on, depth, None).0
}

/// Width the disclosure caret of `rail_tree_row` adds before the icon.
const RAIL_CARET_ROOM: f32 = 18.0;
/// Size of that caret, the one of the Inventory sidebar.
const RAIL_CARET: f32 = 13.0;

/// `rail_row` for a folder of a tree (the Places rail): when `open` is
/// `Some`, the folder has sub-folders and a `disclosure_caret` comes before
/// its icon, as in the Inventory sidebar. The caret is its own click target,
/// returned beside the row: a click on it folds or unfolds, a click on the
/// rest of the row chooses the folder.
#[allow(clippy::too_many_arguments)]
pub fn rail_tree_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    p: &Palette,
    icon: &str,
    tint: Color32,
    label: &str,
    count: Option<usize>,
    on: bool,
    depth: usize,
    open: Option<bool>,
) -> (egui::Response, Option<egui::Response>) {
    let size = Vec2::new(ui.available_width(), 28.0);
    let (_, bounds) = ui.allocate_space(size);
    let mut resp = ui.interact(bounds, id, Sense::click());
    resp.set_intrinsic_size(size);
    let rect = ui.layout().align_size_within_rect(size, bounds);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, label));
    // Even the deepest branch keeps its caret, icon and some of its name
    // inside the rail, clear of the count (up to 40 points wide + margin).
    let count_room = if count.is_some() { 50.0 } else { 0.0 };
    let caret_room = if open.is_some() { RAIL_CARET_ROOM } else { 0.0 };
    let max_indent = (rect.width() - 10.0 - caret_room - 25.0 - 32.0 - count_room).max(0.0);
    let x = rect.left() + 10.0 + (depth as f32 * 14.0).min(max_indent);
    let caret_center = egui::pos2(x + 8.0, rect.center().y);
    // Registered after the row, so it is the one a click on the caret reaches.
    let caret = open.map(|open| {
        let caret = ui.interact(
            egui::Rect::from_center_size(caret_center, Vec2::splat(RAIL_CARET + 6.0)),
            resp.id.with("rail_caret"),
            Sense::click(),
        );
        caret.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, ui.is_enabled(), label));
        ui.ctx().accesskit_node_builder(caret.id, |node| node.set_expanded(open));
        caret
    });
    if resp.gained_focus() || caret.as_ref().is_some_and(|c| c.gained_focus()) {
        resp.scroll_to_me(None);
    }
    if !ui.is_rect_visible(rect) {
        return (resp, caret);
    }
    let radius = ui.visuals().window_corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.28));
    } else if resp.hovered() || resp.has_focus() || caret.as_ref().is_some_and(|c| c.hovered()) {
        ui.painter().rect_filled(rect, radius, p.raised);
    }
    let x = match (open, &caret) {
        (Some(open), Some(c)) => {
            let color = if c.hovered() || c.has_focus() {
                p.ink
            } else if on {
                p.violet_pale
            } else {
                p.muted_dim
            };
            paint_glyph(ui.painter(), disclosure_caret(open), caret_center, RAIL_CARET, color);
            x + RAIL_CARET_ROOM
        }
        _ => x,
    };
    if let Some(t) = super::icons::global(icon) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if on && tint == p.muted { p.violet_pale } else { tint },
        );
    }
    let one_line = |text: &str, size: f32, color: Color32, width: f32| {
        let mut job =
            egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(egui::FontId::proportional(size), color));
        job.wrap = egui::text::TextWrapping {
            max_width: width.max(1.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('…'),
        };
        ui.painter().layout_job(job)
    };
    let num = count.map(|n| one_line(&n.to_string(), 11.5, p.muted, 40.0));
    let num_w = num.as_ref().map_or(0.0, |g| g.size().x + 10.0);
    let ink = if tint == p.violet_light { tint } else { p.ink };
    let g = one_line(label, 13.0, ink, rect.right() - num_w - x - 25.0);
    if g.elided {
        resp = resp.on_hover_text(label);
    }
    paint_row_text(ui.painter(), g, x + 25.0, rect.center().y, ink);
    if let Some(g) = num {
        let w = g.size().x;
        paint_row_text(ui.painter(), g, rect.right() - 10.0 - w, rect.center().y, p.muted);
    }
    (resp, caret)
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

/// Colors `(ink, fill)` for the active group tag pill.
pub fn active_tag_colors(p: &Palette) -> (Color32, Color32) {
    (p.teal, p.teal.gamma_multiply(0.14))
}

/// Measures a small row tag pill (such as "active" next to a group). Returns `(galley, total_width)`.
pub fn tag_pill_layout(ui: &egui::Ui, text: &str, ink: Color32) -> (std::sync::Arc<egui::Galley>, f32) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_owned(), egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())), ink);
    let w = g.size().x + 12.0;
    (g, w)
}

/// Paints a small row tag pill (such as "active" next to a group) into `rect`.
pub fn paint_tag_pill(ui: &egui::Ui, g: std::sync::Arc<egui::Galley>, rect: egui::Rect, ink: Color32, fill: Color32) {
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, fill);
    let left = rect.center().x - g.size().x / 2.0;
    paint_row_text(ui.painter(), g, left, rect.center().y, ink);
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
    if let Some(texture) = tex {
        local_texture::image(ui, p, rect, texture.id());
    }
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
mod floater_title_rename_tests {
    use super::{clean_floater_title_name, floater_title_finish};

    #[test]
    fn title_rename_rejects_blank_pipes_controls_and_limits_name_length() {
        assert_eq!(clean_floater_title_name("  "), None);
        assert_eq!(clean_floater_title_name("  Hello|World  "), None);
        assert_eq!(clean_floater_title_name("Bad\nName"), None);
        assert_eq!(clean_floater_title_name("  Hello  ").as_deref(), Some("Hello"));
        assert_eq!(clean_floater_title_name(&"x".repeat(70)).unwrap().len(), 63);
    }

    #[test]
    fn title_escape_cancels_only_when_the_field_was_focused() {
        assert_eq!(floater_title_finish(true, true, false, false), Some(false));
        assert_eq!(floater_title_finish(true, true, true, true), Some(false));
        assert_eq!(
            floater_title_finish(false, true, false, true),
            Some(false),
            "Escape cancels even when TextEdit has already surrendered focus"
        );
        assert_eq!(floater_title_finish(false, true, false, false), None);
    }

    #[test]
    fn title_rename_commits_on_focused_enter_or_lost_focus() {
        assert_eq!(floater_title_finish(true, false, false, false), None);
        assert_eq!(floater_title_finish(true, false, true, false), Some(true));
        assert_eq!(floater_title_finish(false, false, true, false), None);
        assert_eq!(floater_title_finish(true, false, false, true), Some(true));
    }
}

#[cfg(test)]
mod spin_arrow_tests {
    use super::arrow_hold_repeats;

    #[test]
    fn arrow_hold_repeats_lands_on_the_50_ms_marks() {
        assert_eq!(arrow_hold_repeats(0.49), 0);
        assert_eq!(arrow_hold_repeats(0.50), 1);
        assert_eq!(arrow_hold_repeats(0.54), 1);
        assert_eq!(arrow_hold_repeats(0.55), 2);
        // 0.10 / 0.05 is 1.999… in f64. The repeat at 0.60 s must still count.
        assert_eq!(arrow_hold_repeats(0.60), 3);
        assert_eq!(arrow_hold_repeats(0.90), 9);
        assert_eq!(arrow_hold_repeats(f64::NAN), 0);
    }
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

#[cfg(test)]
mod segmented_disabled_tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;

    /// A disabled option takes no click, whatever else is clicked changes
    /// the choice.
    #[test]
    fn a_disabled_segment_cannot_be_chosen() {
        let p = crate::theme::Theme::default().palette();
        let mut chosen = Vec::new();
        for (disabled, clicks_work) in [(Some("why"), false), (None, true)] {
            let mut frames = Frames::new();
            let mut selected = 0usize;
            let draw = |frames: &mut Frames, events: Vec<egui::Event>, selected: &mut usize| {
                frames.run(events, |ui| {
                    segmented_with_disabled(
                        ui,
                        &p,
                        selected,
                        &[("Aaaa", true), ("Bbbb", true), ("Cccc", true)],
                        &[None, disabled],
                    );
                })
            };
            draw(&mut frames, Vec::new(), &mut selected);
            // the second segment's middle: after the first, in the track
            let texts = draw(&mut frames, Vec::new(), &mut selected);
            let b = texts.iter().find(|t| t.source == "Bbbb").expect("the label is drawn").rect.center();
            draw(&mut frames, vec![egui::Event::PointerMoved(b)], &mut selected);
            for pressed in [true, false] {
                draw(
                    &mut frames,
                    vec![egui::Event::PointerButton {
                        pos: b,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    }],
                    &mut selected,
                );
            }
            chosen.push((selected, clicks_work));
        }
        assert_eq!(chosen, vec![(0, false), (1, true)]);
    }
}

#[cfg(test)]
mod wrapping_tabs_tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, painted};

    #[test]
    fn wrapped_tab_labels_stay_whole_and_clicks_keep_their_index() {
        let labels = ["Track", "Camera", "Look", "Output", "Filters", "Library"].map(|label| (label, true));
        let p = crate::theme::Theme::default().palette();
        for width in [220.0, 340.0] {
            for font_size in [12.0, 24.0] {
                let mut frames = Frames::new();
                let mut selected = 0;
                let draw = |frames: &mut Frames, events, selected: &mut usize| {
                    frames.run(events, |ui| {
                        ui.set_width(width);
                        ui.style_mut().override_font_id = Some(egui::FontId::proportional(font_size));
                        wrapping_tabs(ui, &p, selected, &labels);
                    })
                };
                draw(&mut frames, Vec::new(), &mut selected);
                let texts = draw(&mut frames, Vec::new(), &mut selected);
                for (label, _) in &labels {
                    assert!(!painted(&texts, label).cut, "{label} at width {width}, font {font_size}");
                }
                let last = painted(&texts, "Library").rect.center();
                if width == 220.0 || font_size == 24.0 {
                    assert!(last.y > painted(&texts, "Track").rect.center().y);
                }
                draw(&mut frames, vec![egui::Event::PointerMoved(last)], &mut selected);
                for pressed in [true, false] {
                    draw(
                        &mut frames,
                        vec![egui::Event::PointerButton {
                            pos: last,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        }],
                        &mut selected,
                    );
                }
                assert_eq!(selected, 5);
            }
        }
    }
}
