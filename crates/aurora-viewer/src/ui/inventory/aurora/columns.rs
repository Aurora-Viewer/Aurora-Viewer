//! Saved expanded-inventory splitters. Like LLScrollListCtrl::updateColumnWidths
//! (indra/llui/llscrolllistctrl.cpp), the name absorbs remaining space. Adjacent
//! columns share a dragged boundary; shrinking the window only fits the display.

use super::super::view::ExplorerPreferences;
use super::{Frame, Layout, rights};
use crate::theme::Palette;
use crate::ui::{menu, widgets};
use egui::{Rangef, Rect, Sense};

pub(super) const GAP: f32 = 12.0;
pub(super) const PAD: f32 = 8.0;
const NAME_MIN: f32 = 80.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    pub name_end: f32,
    pub kind: Option<Rangef>,
    pub rights: Option<Rangef>,
    pub date: Option<Rangef>,
}

impl Columns {
    pub fn new(frame: Frame, rect: Rect) -> Self {
        let prefs = frame.columns;
        let expanded = frame.layout != Layout::Compact;
        let wide = frame.layout == Layout::Wide;
        let visible = [
            expanded && prefs.show_kind,
            expanded && prefs.show_rights && frame.rights != super::Rights::Hidden,
            expanded && prefs.show_date,
        ];
        let defaults = [
            if wide { 128.0 } else { 112.0 },
            rights::width(frame.rights).max(56.0),
            if wide { 92.0 } else { 72.0 },
        ];
        let saved = [prefs.kind_width, prefs.rights_width, prefs.date_width];
        let minima = [64.0, defaults[1], 64.0];
        let mut widths = [0.0; 3];
        let mut minimum = 0.0;
        let mut extra = 0.0;
        for (i, width) in widths.iter_mut().enumerate() {
            if visible[i] {
                *width = if saved[i] == 0 {
                    defaults[i]
                } else {
                    f32::from(saved[i]).clamp(minima[i], 1600.0)
                };
                minimum += minima[i];
                extra += *width - minima[i];
            }
        }
        let gaps = GAP * visible.iter().filter(|v| **v).count() as f32;
        let room = (rect.width() - 2.0 * PAD - NAME_MIN - gaps).max(0.0);
        let extra_scale = if extra > 0.0 {
            ((room - minimum) / extra).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let minimum_scale = if minimum > 0.0 { (room / minimum).min(1.0) } else { 1.0 };
        let mut end = rect.right() - PAD;
        let mut ranges = [None; 3];
        for (i, range) in ranges.iter_mut().enumerate().rev() {
            if visible[i] {
                let width = minima[i] * minimum_scale + (widths[i] - minima[i]) * extra_scale;
                *range = Some(Rangef::new(end - width, end));
                end -= width + GAP;
            }
        }
        Self {
            name_end: end,
            kind: ranges[0],
            rights: ranges[1],
            date: ranges[2],
        }
    }

    fn ranges(self) -> [Option<Rangef>; 3] {
        [self.kind, self.rights, self.date]
    }
}

fn saved_width(prefs: &mut ExplorerPreferences, index: usize) -> &mut u16 {
    match index {
        0 => &mut prefs.kind_width,
        1 => &mut prefs.rights_width,
        _ => &mut prefs.date_width,
    }
}

pub(super) fn resize(ui: &mut egui::Ui, p: &Palette, mut frame: Frame, rect: Rect, prefs: &mut ExplorerPreferences) -> Columns {
    frame.columns = *prefs;
    let cols = Columns::new(frame, rect);
    let mut previous = None;
    let mut left_width = cols.name_end - rect.left() - PAD;
    for (index, range) in cols.ranges().into_iter().enumerate() {
        let Some(range) = range else { continue };
        let x = range.min - GAP * 0.5;
        let hit = Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(8.0, rect.height()));
        let response = ui
            .interact(hit, ui.scope_id().with(("inventory_column_resize", index)), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            .on_hover_text(aurora_i18n::tr!("inventory-resize-columns"));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::unique(("inventory_column_handle", index)), hit));
        if response.hovered() || response.dragged() {
            ui.painter().vline(
                x,
                rect.y_range().shrink(3.0),
                widgets::border_stroke(p, egui::Stroke::new(1.0, p.violet_light)),
            );
        }
        if response.double_clicked() {
            *saved_width(prefs, index) = 0;
            if let Some(previous) = previous {
                *saved_width(prefs, previous) = 0;
            }
        } else if response.dragged() {
            let left_min = previous.map_or(NAME_MIN, |i| if i == 1 { rights::width(frame.rights).max(56.0) } else { 64.0 });
            let right_min = if index == 1 { rights::width(frame.rights).max(56.0) } else { 64.0 };
            let delta = response
                .drag_delta()
                .x
                .clamp(-(left_width - left_min).max(0.0), (range.span() - right_min).max(0.0));
            if delta != 0.0 {
                *saved_width(prefs, index) = (range.span() - delta).round() as u16;
                if let Some(previous) = previous {
                    *saved_width(prefs, previous) = (left_width + delta).round() as u16;
                }
            }
        }
        previous = Some(index);
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

pub(super) fn options(ui: &mut egui::Ui, p: &Palette, prefs: &mut ExplorerPreferences, mode: &mut super::Rights) {
    menu::check(ui, p, "", aurora_i18n::tr!("common-name"), true, false);
    menu::toggle(ui, p, "", aurora_i18n::tr!("common-type"), &mut prefs.show_kind);
    let mut show_rights = prefs.show_rights && *mode != super::Rights::Hidden;
    if menu::toggle(ui, p, "", aurora_i18n::tr!("inventory-column-permissions"), &mut show_rights) {
        prefs.show_rights = show_rights;
        if show_rights && *mode == super::Rights::Hidden {
            *mode = super::Rights::Mine;
        }
    }
    menu::toggle(ui, p, "", aurora_i18n::tr!("inventory-column-modified"), &mut prefs.show_date);
    menu::separator(ui, p);
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
    fn saved_columns_fit_all_visibility_and_rights_choices_without_losing_preferences() {
        for layout in [Layout::Compact, Layout::Medium, Layout::Wide] {
            for rights in [Rights::Hidden, Rights::Mine, Rights::Next, Rights::Both] {
                for mask in 0..8 {
                    let prefs = ExplorerPreferences {
                        kind_width: u16::MAX,
                        rights_width: 1,
                        date_width: 420,
                        show_kind: mask & 1 != 0,
                        show_rights: mask & 2 != 0,
                        show_date: mask & 4 != 0,
                        ..Default::default()
                    };
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
                        assert_eq!(cols.kind.is_some(), layout != Layout::Compact && prefs.show_kind);
                        assert_eq!(
                            cols.rights.is_some(),
                            layout != Layout::Compact && prefs.show_rights && rights != Rights::Hidden
                        );
                        assert_eq!(cols.date.is_some(), layout != Layout::Compact && prefs.show_date);
                        let mut end = cols.name_end;
                        for range in cols.ranges().into_iter().flatten() {
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
        changed.explorer = ExplorerPreferences {
            sidebar_width: 314,
            kind_width: 230,
            rights_width: 128,
            date_width: 164,
            show_date: false,
            ..Default::default()
        };
        let saved = serde_json::to_string(&changed).unwrap();
        let reloaded: InventoryPreferences = serde_json::from_str(&saved).unwrap();
        assert_eq!(reloaded, changed);
        assert_eq!(reloaded.rights, old.rights);
        let partial: InventoryPreferences = serde_json::from_str(r#"{"explorer":{"sidebar_width":280,"show_kind":false}}"#).unwrap();
        assert!(!partial.explorer.show_kind);
        assert!(partial.explorer.show_date && partial.explorer.show_rights);
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
                kind_width: 240,
                ..Default::default()
            };
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
                    |ui| options(ui, &palette, prefs, &mut mode),
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
                    assert!(!prefs.show_kind);
                }
                if label == aurora_i18n::tr!("inventory-column-permissions") {
                    assert!(!prefs.show_rights);
                }
                if label == aurora_i18n::tr!("inventory-column-modified") {
                    assert!(!prefs.show_date);
                }
            }
            assert_eq!(prefs, ExplorerPreferences::default());
            assert_eq!(mode, Rights::Next);
        }
    }
}
