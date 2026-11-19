//! Saved expanded-inventory splitters. Like LLScrollListCtrl::updateColumnWidths
//! (indra/llui/llscrolllistctrl.cpp), the name absorbs remaining space. Adjacent
//! columns share a dragged boundary; shrinking the window only fits the display.
//! Aurora differences (Windows Explorer, Dolphin): the columns can be chosen
//! and reordered, and a double-click on a divider fits the column on its
//! left to what it shows.

use super::super::view::{COLUMNS, ColumnId, ExplorerPreferences};
use super::{Frame, Layout, rights};
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use egui::{Rangef, Rect, Sense};

pub(super) const GAP: f32 = 12.0;
pub(super) const PAD: f32 = 8.0;
const NAME_MIN: f32 = 80.0;
const MAX: f32 = 1600.0;

/// Where each shown column is drawn this frame, in their order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    pub name_end: f32,
    ranges: [Option<(ColumnId, Rangef)>; COLUMNS],
}

/// A column's width when none is saved, by the window's width.
fn default_width(id: ColumnId, frame: Frame) -> f32 {
    let wide = frame.layout == Layout::Wide;
    match id {
        ColumnId::Kind => {
            if wide {
                128.0
            } else {
                112.0
            }
        }
        ColumnId::Rights => minimum(id, frame),
        ColumnId::Date => {
            if wide {
                92.0
            } else {
                72.0
            }
        }
        ColumnId::Creator => {
            if wide {
                140.0
            } else {
                120.0
            }
        }
        ColumnId::Description => {
            if wide {
                180.0
            } else {
                140.0
            }
        }
        ColumnId::Worn => {
            if wide {
                104.0
            } else {
                80.0
            }
        }
        ColumnId::Count => 56.0,
    }
}

fn minimum(id: ColumnId, frame: Frame) -> f32 {
    match id {
        ColumnId::Rights => rights::width(frame.rights).max(56.0),
        ColumnId::Count => 40.0,
        ColumnId::Worn => 48.0,
        _ => 64.0,
    }
}

/// Whether `id` is drawn: the explorer's widths, the column switched on,
/// and for the rights a mode that shows them.
fn visible(id: ColumnId, frame: Frame) -> bool {
    frame.layout != Layout::Compact && frame.columns.columns.shown(id) && (id != ColumnId::Rights || frame.rights != super::Rights::Hidden)
}

impl Columns {
    pub fn new(frame: Frame, rect: Rect) -> Self {
        let slots = frame.columns.columns.0;
        let mut widths = [0.0; COLUMNS];
        let mut minimum_sum = 0.0;
        let mut extra = 0.0;
        let mut shown = 0;
        for (i, slot) in slots.iter().enumerate() {
            if visible(slot.id, frame) {
                let least = minimum(slot.id, frame);
                widths[i] = if slot.width == 0 {
                    default_width(slot.id, frame)
                } else {
                    f32::from(slot.width).clamp(least, MAX)
                };
                minimum_sum += least;
                extra += widths[i] - least;
                shown += 1;
            }
        }
        let gaps = GAP * shown as f32;
        let room = (rect.width() - 2.0 * PAD - NAME_MIN - gaps).max(0.0);
        let extra_scale = if extra > 0.0 {
            ((room - minimum_sum) / extra).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let minimum_scale = if minimum_sum > 0.0 { (room / minimum_sum).min(1.0) } else { 1.0 };
        let mut end = rect.right() - PAD;
        let mut ranges = [None; COLUMNS];
        for (i, slot) in slots.iter().enumerate().rev() {
            if visible(slot.id, frame) {
                let least = minimum(slot.id, frame);
                let width = least * minimum_scale + (widths[i] - least) * extra_scale;
                ranges[i] = Some((slot.id, Rangef::new(end - width, end)));
                end -= width + GAP;
            }
        }
        Self { name_end: end, ranges }
    }

    /// Where `id` is drawn, if it is.
    pub fn get(&self, id: ColumnId) -> Option<Rangef> {
        self.iter().find(|(shown, _)| *shown == id).map(|(_, range)| range)
    }

    /// The shown columns, left to right.
    pub fn iter(&self) -> impl Iterator<Item = (ColumnId, Rangef)> + '_ {
        self.ranges.iter().flatten().copied()
    }
}

/// The widest thing each column showed: the rows drawn and its title, the
/// last frame's for the fit of a divider's double-click. Only the rows in
/// sight are measured (Dolphin's fit).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColumnFit {
    last: [f32; COLUMNS],
    now: [f32; COLUMNS],
}

impl ColumnFit {
    /// A new frame of the list starts: what was measured becomes the fit.
    pub fn start(&mut self) {
        self.last = self.now;
        self.now = [0.0; COLUMNS];
    }

    pub fn note(&mut self, id: ColumnId, width: f32) {
        let slot = &mut self.now[id as usize];
        *slot = slot.max(width);
    }

    /// The width that shows all of `id`, if it was drawn.
    pub fn width(&self, id: ColumnId) -> Option<f32> {
        let width = self.last[id as usize].max(self.now[id as usize]);
        (width > 0.0).then(|| width.ceil() + 2.0)
    }
}

/// Saves `id` at the width that shows all of it.
fn fit(prefs: &mut ExplorerPreferences, fit: &ColumnFit, id: ColumnId, frame: Frame) {
    if let Some(width) = fit.width(id) {
        prefs.columns.slot_mut(id).width = width.clamp(minimum(id, frame), MAX).round() as u16;
    }
}

/// Every shown column at the width that shows all of it.
pub(super) fn fit_all(prefs: &mut ExplorerPreferences, measured: &ColumnFit, frame: Frame) {
    for id in ColumnId::ALL {
        if visible(id, Frame { columns: *prefs, ..frame }) {
            fit(prefs, measured, id, frame);
        }
    }
}

pub(super) fn resize(
    ui: &mut egui::Ui,
    p: &Palette,
    mut frame: Frame,
    rect: Rect,
    prefs: &mut ExplorerPreferences,
    measured: &ColumnFit,
) -> Columns {
    frame.columns = *prefs;
    let cols = Columns::new(frame, rect);
    let mut previous: Option<(ColumnId, f32)> = None;
    let mut left_width = cols.name_end - rect.left() - PAD;
    for (index, (id, range)) in cols.iter().enumerate() {
        let x = range.min - GAP * 0.5;
        let hit = Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(8.0, rect.height()));
        let response = ui
            .interact(hit, ui.scope_id().with(("inventory_column_resize", id)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(aurora_i18n::tr!("inventory-resize-columns"));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_column_handle", index)), hit));
        let _ = index;
        if response.hovered() || response.dragged() {
            ui.painter().vline(
                x,
                rect.y_range().shrink(3.0),
                widgets::border_stroke(p, egui::Stroke::new(1.0, p.violet_light)),
            );
        }
        if response.double_clicked() {
            match previous {
                // the column on the divider's left fits what it shows
                Some((left, _)) => fit(prefs, measured, left, frame),
                // the name takes what is left: its divider resets the column
                None => prefs.columns.slot_mut(id).width = 0,
            }
        } else if response.dragged() {
            let left_min = previous.map_or(NAME_MIN, |(left, _)| minimum(left, frame));
            let right_min = minimum(id, frame);
            let delta = response
                .drag_delta()
                .x
                .clamp(-(left_width - left_min).max(0.0), (range.span() - right_min).max(0.0));
            if delta != 0.0 {
                prefs.columns.slot_mut(id).width = (range.span() - delta).round() as u16;
                if let Some((left, _)) = previous {
                    prefs.columns.slot_mut(left).width = (left_width + delta).round() as u16;
                }
            }
        }
        previous = Some((id, range.span()));
        left_width = range.span();
    }
    frame.columns = *prefs;
    Columns::new(frame, rect)
}

pub(super) fn sidebar(ui: &mut egui::Ui, p: &Palette, area: Rect, pane: f32, layout: Layout, prefs: &mut ExplorerPreferences) -> f32 {
    let default = if layout == Layout::Wide { 220.0 } else { 196.0 };
    let max = (area.width() - pane - if pane > 0.0 { 16.0 } else { 8.0 } - 360.0).max(140.0);
    let width = if prefs.sidebar_width == 0 {
        default
    } else {
        f32::from(prefs.sidebar_width)
    }
    .clamp(140.0, max);
    let hit = Rect::from_min_max(
        egui::pos2(area.left() + width, area.top()),
        egui::pos2(area.left() + width + 8.0, area.bottom()),
    );
    let response = ui
        .interact(hit, ui.scope_id().with("inventory_sidebar_resize"), Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text(aurora_i18n::tr!("inventory-resize-sidebar"));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique("inventory_sidebar_handle"), hit));
    if response.hovered() || response.dragged() {
        ui.painter().vline(
            hit.center().x,
            hit.y_range(),
            widgets::border_stroke(p, egui::Stroke::new(1.0, p.violet_light)),
        );
    }
    if response.double_clicked() {
        prefs.sidebar_width = 0;
        default.clamp(140.0, max)
    } else if response.dragged() && response.drag_delta().x != 0.0 {
        let width = (width + response.drag_delta().x).clamp(140.0, max);
        prefs.sidebar_width = width.round() as u16;
        width
    } else {
        width
    }
}

/// The label of a column, as its title and its menu entry say it.
pub(super) fn title(id: ColumnId, frame: Frame) -> &'static str {
    match id {
        ColumnId::Kind => aurora_i18n::tr!("common-type"),
        ColumnId::Rights => rights::heading(frame.rights),
        ColumnId::Date => aurora_i18n::tr!("inventory-column-modified"),
        ColumnId::Creator => aurora_i18n::tr!("inventory-column-creator"),
        ColumnId::Description => aurora_i18n::tr!("inventory-sort-description"),
        ColumnId::Worn => aurora_i18n::tr!("common-worn"),
        ColumnId::Count => aurora_i18n::tr!("inventory-column-count"),
    }
}

/// The columns' menu, of the toolbar's Columns button and of the header's
/// right-click: a switch per column in their order, the fit and the resets.
pub(super) fn options(
    ui: &mut egui::Ui,
    p: &Palette,
    prefs: &mut ExplorerPreferences,
    mode: &mut super::Rights,
    measured: &ColumnFit,
    layout: Layout,
) {
    menu::check(ui, p, "", aurora_i18n::tr!("common-name"), true, false);
    for slot in prefs.columns.0 {
        let id = slot.id;
        if id == ColumnId::Rights {
            let mut show_rights = slot.show && *mode != super::Rights::Hidden;
            if menu::toggle(ui, p, "", aurora_i18n::tr!("inventory-column-permissions"), &mut show_rights) {
                prefs.columns.slot_mut(id).show = show_rights;
                if show_rights && *mode == super::Rights::Hidden {
                    *mode = super::Rights::Mine;
                }
            }
        } else {
            let frame = Frame {
                layout,
                rights: *mode,
                columns: *prefs,
                explorer: true,
            };
            let mut show = slot.show;
            if menu::toggle(ui, p, "", title(id, frame), &mut show) {
                prefs.columns.slot_mut(id).show = show;
            }
        }
    }
    menu::separator(ui, p);
    let frame = Frame {
        layout,
        rights: *mode,
        columns: *prefs,
        explorer: true,
    };
    if menu::item(ui, p, "arrows-horizontal", aurora_i18n::tr!("inventory-columns-fit")) {
        fit_all(prefs, measured, frame);
    }
    if menu::item(ui, p, "restore", aurora_i18n::tr!("inventory-columns-reset-widths")) {
        prefs.columns.reset_widths();
    }
    if menu::item(ui, p, "restore", aurora_i18n::tr!("inventory-columns-reset")) {
        *prefs = ExplorerPreferences::default();
        if *mode == super::Rights::Hidden {
            *mode = super::Rights::Mine;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Rights;
    use super::*;

    #[test]
    fn saved_columns_fit_all_visibility_order_and_rights_choices_without_losing_preferences() {
        for layout in [Layout::Compact, Layout::Medium, Layout::Wide] {
            for rights in [Rights::Hidden, Rights::Mine, Rights::Next, Rights::Both] {
                for mask in 0..(1u32 << COLUMNS) {
                    let mut prefs = ExplorerPreferences::default();
                    for (i, id) in ColumnId::ALL.into_iter().enumerate() {
                        let slot = prefs.columns.slot_mut(id);
                        slot.show = mask & (1 << i) != 0;
                        slot.width = [u16::MAX, 1, 420, 0, 90, 30, 2000][i];
                    }
                    // a reordered list draws in its own order
                    if mask % 3 == 0 {
                        prefs.columns.move_to(ColumnId::Date, 0);
                    }
                    let frame = Frame {
                        layout,
                        rights,
                        columns: prefs,
                        explorer: false,
                    };
                    for width in [360.0, 620.0, 1400.0] {
                        let rect = Rect::from_min_size(egui::pos2(124.0, 70.0), egui::vec2(width, 24.0));
                        let cols = Columns::new(frame, rect);
                        assert!(cols.name_end >= rect.left() + PAD + NAME_MIN - 0.1);
                        for id in ColumnId::ALL {
                            let shown = layout != Layout::Compact
                                && prefs.columns.shown(id)
                                && (id != ColumnId::Rights || rights != Rights::Hidden);
                            assert_eq!(cols.get(id).is_some(), shown, "{id:?}");
                        }
                        let order: Vec<_> = cols.iter().map(|(id, _)| id).collect();
                        let expected: Vec<_> = prefs.columns.0.iter().map(|s| s.id).filter(|id| order.contains(id)).collect();
                        assert_eq!(order, expected);
                        let mut end = cols.name_end;
                        for (_, range) in cols.iter() {
                            assert!(range.min >= end + GAP - 0.1 && range.max > range.min);
                            assert!(range.max <= rect.right() - PAD + 0.1);
                            end = range.max;
                        }
                    }
                    assert_eq!(frame.columns, prefs);
                }
            }
        }
    }

    #[test]
    fn expanded_inventory_preferences_migrate_and_round_trip() {
        use super::super::super::InventoryPreferences;
        let old: InventoryPreferences = serde_json::from_str(r#"{"rights":"Next"}"#).expect("old preferences");
        assert_eq!(old.explorer, ExplorerPreferences::default());
        let mut changed = old.clone();
        changed.explorer.sidebar_width = 314;
        changed.explorer.columns.slot_mut(ColumnId::Kind).width = 230;
        changed.explorer.columns.slot_mut(ColumnId::Date).show = false;
        changed.explorer.columns.slot_mut(ColumnId::Creator).show = true;
        changed.explorer.columns.move_to(ColumnId::Creator, 0);
        let saved = serde_json::to_string(&changed).unwrap();
        let reloaded: InventoryPreferences = serde_json::from_str(&saved).unwrap();
        assert_eq!(reloaded, changed);
        assert_eq!(reloaded.rights, old.rights);
        // the three columns' settings of before the list
        let partial: InventoryPreferences =
            serde_json::from_str(r#"{"explorer":{"sidebar_width":280,"show_kind":false,"date_width":150}}"#).unwrap();
        let columns = partial.explorer.columns;
        assert_eq!(partial.explorer.sidebar_width, 280);
        assert!(!columns.shown(ColumnId::Kind));
        assert!(columns.shown(ColumnId::Date) && columns.shown(ColumnId::Rights));
        assert!(!columns.shown(ColumnId::Creator));
        assert_eq!(columns.slot(ColumnId::Date).width, 150);
        // an unknown, repeated or missing column never loses the others
        let odd: ExplorerPreferences = serde_json::from_str(
            r#"{"columns":[{"id":"worn","show":true},{"id":"size","show":true},{"id":"worn","show":false},{"id":"kind","show":false,"width":99},{"show":true}]}"#,
        )
        .unwrap();
        let ids: Vec<_> = odd.columns.0.iter().map(|s| s.id).collect();
        assert_eq!(ids[..2], [ColumnId::Worn, ColumnId::Kind]);
        assert_eq!(ids.len(), COLUMNS);
        assert!(ColumnId::ALL.iter().all(|id| ids.contains(id)));
        assert!(odd.columns.shown(ColumnId::Worn) && !odd.columns.shown(ColumnId::Kind));
        assert_eq!(odd.columns.slot(ColumnId::Kind).width, 99);
        assert!(!odd.columns.shown(ColumnId::Date), "a column the list lacks comes back hidden");
    }

    #[test]
    fn a_fit_takes_the_widest_text_the_last_frame_measured() {
        let mut fit = ColumnFit::default();
        assert_eq!(fit.width(ColumnId::Kind), None, "nothing drawn, nothing to fit");
        fit.note(ColumnId::Kind, 40.0);
        fit.note(ColumnId::Kind, 70.4);
        fit.note(ColumnId::Kind, 20.0);
        fit.start();
        assert_eq!(fit.width(ColumnId::Kind), Some(73.0));
        fit.start();
        assert_eq!(fit.width(ColumnId::Kind), None, "a column hidden since is not fitted");
        let frame = Frame {
            layout: Layout::Wide,
            rights: Rights::Mine,
            columns: ExplorerPreferences::default(),
            explorer: true,
        };
        let mut prefs = ExplorerPreferences::default();
        fit.note(ColumnId::Kind, 30.0);
        fit.note(ColumnId::Date, 9000.0);
        fit.note(ColumnId::Creator, 200.0);
        fit_all(&mut prefs, &fit, frame);
        assert_eq!(prefs.columns.slot(ColumnId::Kind).width, 64, "never under the column's least width");
        assert_eq!(prefs.columns.slot(ColumnId::Date).width, 1600, "nor over the most");
        assert_eq!(prefs.columns.slot(ColumnId::Creator).width, 0, "a hidden column keeps its width");
    }

    #[test]
    fn column_menu_clicks_hide_columns_and_reset_without_changing_permission_mode() {
        fn text_rect(shape: &egui::Shape, label: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(Rect::from_min_size(text.pos, text.galley.size())),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, label)),
                _ => None,
            }
        }
        for scale in [1.0, 1.5] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let palette = crate::theme::Theme::default().palette();
            let mut prefs = ExplorerPreferences {
                sidebar_width: 312,
                ..Default::default()
            };
            prefs.columns.slot_mut(ColumnId::Kind).width = 240;
            let mut tick = 0;
            let mut mode = Rights::Next;
            let mut draw = |prefs: &mut ExplorerPreferences, events| {
                tick += 1;
                super::super::super::headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                        time: Some(f64::from(tick) / 60.0),
                        events,
                        ..Default::default()
                    },
                    |ui| options(ui, &palette, prefs, &mut mode, &ColumnFit::default(), Layout::Wide),
                ))
            };
            for label in [
                aurora_i18n::tr!("common-type"),
                aurora_i18n::tr!("inventory-column-permissions"),
                aurora_i18n::tr!("inventory-column-modified"),
                aurora_i18n::tr!("inventory-columns-reset"),
            ] {
                drop(draw(&mut prefs, vec![]));
                let output = draw(&mut prefs, vec![]);
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|shape| text_rect(&shape.shape, label))
                    .expect("column menu entry")
                    .center();
                drop(draw(&mut prefs, vec![egui::Event::PointerMoved(pos)]));
                for pressed in [true, false] {
                    drop(draw(
                        &mut prefs,
                        vec![egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        }],
                    ));
                }
                if label == aurora_i18n::tr!("common-type") {
                    assert!(!prefs.columns.shown(ColumnId::Kind));
                }
                if label == aurora_i18n::tr!("inventory-column-permissions") {
                    assert!(!prefs.columns.shown(ColumnId::Rights));
                }
                if label == aurora_i18n::tr!("inventory-column-modified") {
                    assert!(!prefs.columns.shown(ColumnId::Date));
                }
            }
            assert_eq!(prefs, ExplorerPreferences::default());
            assert_eq!(mode, Rights::Next);
        }
    }
}
