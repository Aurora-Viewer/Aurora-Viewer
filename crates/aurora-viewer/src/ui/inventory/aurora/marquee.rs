//! Rubber-band (marquee) selection in the explorer, as in Windows Explorer
//! and Dolphin: a drag that starts in the list's empty space draws a band,
//! and every row or tile it touches is selected; Ctrl or Shift adds to the
//! selection already there; near the list's top or bottom edge the list
//! scrolls (the drag's own edge scrolling, `drag_navigation::scroll`).
//! Aurora difference: the Legacy Viewer's list has no marquee.
//!
//! The rows are virtualised (only those in sight are drawn), so each row
//! drawn while the band is out is remembered where it sits in the list's
//! content, not on screen: a row scrolled out of sight under the band stays
//! selected.

use super::super::*;
use egui::{Pos2, Rect, Sense};
use std::collections::HashMap;

/// The smallest drag that is a band rather than a slip of a click.
const THRESHOLD: f32 = 4.0;

#[derive(Debug, Default)]
pub(crate) struct Marquee {
    /// Where the band started, in the content's coordinates.
    start: Option<Pos2>,
    /// The selection before the band (kept with Ctrl or Shift).
    base: HashSet<Uuid>,
    /// The rows drawn since the band started, in the content's coordinates.
    seen: HashMap<Uuid, Rect>,
    /// The list's scroll area and its viewport's top, from its last frame.
    scroll: Option<egui::Id>,
    top: f32,
    /// The scroll offset the rows of this frame are laid out with.
    offset: f32,
}

/// The rows a band from `a` to `b` touches, among `seen`; all in the
/// content's coordinates.
pub(crate) fn touched(seen: &HashMap<Uuid, Rect>, a: Pos2, b: Pos2) -> HashSet<Uuid> {
    let band = Rect::from_two_pos(a, b);
    seen.iter().filter(|(_, rect)| rect.intersects(band)).map(|(id, _)| *id).collect()
}

fn active_id() -> egui::Id {
    egui::Id::unique("inventory_marquee_active")
}

/// A band is being drawn in some explorer (the edge scrolling follows it):
/// one was drawn in the last frame, so a band whose explorer went away
/// (closed, narrowed, another shortcut) stops at once.
pub(crate) fn active(ctx: &egui::Context) -> bool {
    let now = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<u64>(active_id())).is_some_and(|at| at + 1 >= now)
}

impl Marquee {
    fn content(&self, pos: Pos2) -> Pos2 {
        Pos2::new(pos.x, pos.y - self.top + self.offset)
    }
}

/// Before the list draws: the empty space under its rows takes a drag. The
/// rows, laid out after, keep their own clicks and drags.
pub(crate) fn begin(ui: &mut egui::Ui, st: &mut InventoryUi, list: Rect) {
    let m = &mut st.marquee;
    m.offset = m
        .scroll
        .and_then(|id| egui::scroll_area::State::load(ui.ctx(), id))
        .map_or(0.0, |state| state.unclamped_offset().y);
    // click as well as drag: a drag-only widget's drag would start at the
    // press and take the click from the row above it (egui)
    let r = ui.interact(list, ui.scope_id().with("inventory_marquee"), Sense::click_and_drag());
    if r.drag_started()
        && let Some(pos) = r.interact_pointer_pos()
    {
        let adding = ui.input(|i| i.modifiers.command || i.modifiers.ctrl || i.modifiers.shift);
        let m = &mut st.marquee;
        m.start = Some(m.content(pos));
        m.seen.clear();
        m.base = if adding { st.selection.clone() } else { HashSet::new() };
        clipboard::request_focus(ui, st);
    }
}

/// A row or a tile drawn at `rect` this frame.
pub(crate) fn record(st: &mut InventoryUi, id: Uuid, rect: Rect) {
    let m = &mut st.marquee;
    if m.start.is_some() {
        let at = rect.translate(egui::vec2(0.0, m.offset - m.top));
        m.seen.insert(id, at);
    }
}

/// After the list draws: the band follows the pointer, selects what it
/// touches, and is drawn over the list.
pub(crate) fn end(ui: &egui::Ui, p: &Palette, st: &mut InventoryUi, list: Rect, scroll: egui::Id, viewport: Rect) {
    let ctx = ui.ctx().clone();
    st.marquee.scroll = Some(scroll);
    st.marquee.top = viewport.top();
    let Some(start) = st.marquee.start else { return };
    let (pos, down) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.primary_down()));
    let Some(pos) = pos else { return };
    if !down {
        st.marquee.start = None;
        st.marquee.seen.clear();
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(active_id(), frame));
    let now = st.marquee.content(pos);
    if (now - start).length() < THRESHOLD {
        return;
    }
    let mut selection = st.marquee.base.clone();
    selection.extend(touched(&st.marquee.seen, start, now));
    if selection != st.selection {
        let order = keyboard::shown_order(st);
        st.selected = st
            .selected
            .filter(|id| selection.contains(id))
            .or_else(|| order.iter().find(|id| selection.contains(id)).copied());
        st.anchor = st.selected;
        st.selection = selection;
        st.focus = None;
    }
    // the band on screen, its start where the content has moved it
    let from = Pos2::new(start.x, start.y + st.marquee.top - st.marquee.offset);
    let band = Rect::from_two_pos(from, pos).intersect(list);
    let painter = ui.painter().with_clip_rect(list.intersect(ui.clip_rect()));
    let radius = ui.visuals().window_corner_radius;
    painter.rect_filled(band, radius, p.violet.gamma_multiply(0.14));
    painter.rect_stroke(band, radius, egui::Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_band_touches_the_rows_it_crosses_either_way() {
        let row = |n: u128, y: f32| (Uuid::from_u128(n), Rect::from_min_size(Pos2::new(0.0, y), egui::vec2(300.0, 24.0)));
        let seen: HashMap<Uuid, Rect> = [row(1, 0.0), row(2, 26.0), row(3, 52.0), row(4, 78.0)].into_iter().collect();
        let down = touched(&seen, Pos2::new(10.0, 30.0), Pos2::new(40.0, 60.0));
        assert_eq!(down, [Uuid::from_u128(2), Uuid::from_u128(3)].into_iter().collect::<HashSet<_>>());
        // drawn upward, the same rows
        assert_eq!(touched(&seen, Pos2::new(40.0, 60.0), Pos2::new(10.0, 30.0)), down);
        assert!(touched(&seen, Pos2::new(310.0, 0.0), Pos2::new(400.0, 100.0)).is_empty());
    }

    #[test]
    fn tiles_are_touched_in_two_dimensions() {
        let tile = |n: u128, x: f32, y: f32| (Uuid::from_u128(n), Rect::from_min_size(Pos2::new(x, y), egui::vec2(128.0, 160.0)));
        let seen: HashMap<Uuid, Rect> = [tile(1, 0.0, 0.0), tile(2, 140.0, 0.0), tile(3, 0.0, 170.0), tile(4, 140.0, 170.0)]
            .into_iter()
            .collect();
        let hit = touched(&seen, Pos2::new(150.0, 10.0), Pos2::new(200.0, 200.0));
        assert_eq!(hit, [Uuid::from_u128(2), Uuid::from_u128(4)].into_iter().collect::<HashSet<_>>());
    }
}
