//! Explorer pane and Details geometry. Splitters preview against the saved widths,
//! then commit once on release so reflowing a large selection does not repeatedly save the layout.
use super::*;
use aurora_i18n::tr;
use model::{DirectoryView, SortKey};

const DIVIDER: f32 = 8.0;
const MIN_COLUMN: [f32; 4] = [62.0, 56.0, 64.0, 56.0];

/// The persisted widths remain the drag origin. Move the hit target and draw a single
/// divider at the clamped preview position. Some platforms report the final motion
/// and button-up separately, so retain the press coordinate until release.
fn divider(ui: &Ui, id: egui::Id, grip: Rect, label: &str, min_dx: f32, max_dx: f32) -> (f32, bool) {
    let memory = id.with("drag-origin");
    let last = ui.data(|d| d.get_temp::<(f32, f32)>(memory));
    let preview = last.map_or(0.0, |(_, x)| x.clamp(min_dx, max_dx));
    let response = ui
        .interact(grip.translate(vec2(preview, 0.0)), id, Sense::drag())
        .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, label));
    let commit = response.drag_stopped();
    let dx = if let Some(delta) = response.total_drag_delta() {
        if let Some(pt) = response.interact_pointer_pos() {
            ui.data_mut(|d| d.insert_temp(memory, (pt.x - delta.x, delta.x)));
        }
        delta.x
    } else if commit {
        last.map_or(0.0, |(origin, delta)| {
            response.interact_pointer_pos().map_or(delta, |p| p.x - origin)
        })
    } else {
        0.0
    }
    .clamp(min_dx, max_dx);
    if commit || !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| d.remove::<(f32, f32)>(memory));
    }
    let stroke = if response.hovered() || response.dragged() {
        egui::Stroke::new(2.0, ui.visuals().selection.stroke.color)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke
    };
    ui.painter().vline(grip.center().x + dx, grip.y_range(), stroke);
    (dx, commit)
}

fn pin_width(width: f32, preferred: Option<u16>) -> (f32, f32, f32) {
    let room = (width - DIVIDER).max(0.0);
    let min = 96.0_f32.min(room * 0.30);
    let max = (room - 156.0_f32.min(room * 0.53)).max(min);
    let desired = preferred.map(f32::from).unwrap_or_else(|| (width * 0.25).clamp(110.0, 210.0));
    (desired.clamp(min, max), min, max)
}

pub(super) fn panes(ui: &Ui, rect: Rect, prefs: &mut Preferences) -> (Rect, Rect, Rect) {
    let line = ui.text_style_height(&egui::TextStyle::Small).ceil();
    let foot_top = (rect.bottom() - (line + 12.0).max(25.0)).max(rect.top());
    let footer = Rect::from_min_max(pos2(rect.left(), foot_top), rect.max);
    let panes = Rect::from_min_max(rect.min, pos2(rect.right(), (foot_top - 2.0).max(rect.top())));
    let (mut width, min, max) = pin_width(rect.width(), prefs.pin_width());
    let grip = Rect::from_min_size(pos2(rect.left() + width, panes.top()), vec2(DIVIDER, panes.height()));
    let (dx, commit) = divider(
        ui,
        ui.scope_id().with("pin-width"),
        grip,
        tr!("explorer-resize-pins"),
        min - width,
        max - width,
    );
    width += dx;
    if commit {
        prefs.set_pin_width(width.round() as u16);
    }
    let right = (rect.left() + width).min(rect.right());
    let side = Rect::from_min_max(panes.min, pos2(right, panes.bottom()));
    let main = Rect::from_min_max(pos2((right + DIVIDER).min(rect.right()), panes.top()), panes.max);
    (side, main, footer)
}

/// Default optional columns yield to the name and search location on a narrow window.
/// Apply saved sizes to the visible columns only; shrinking a window does not rewrite them.
pub(super) fn columns(width: f32, search: bool, preferred: [u16; 3]) -> [f32; 4] {
    let width = width.max(1.0);
    let ty = if search {
        if width >= 420.0 { (width * 0.16).clamp(65.0, 140.0) } else { 0.0 }
    } else if width >= 154.0 {
        (width * 0.25).clamp(58.0, 160.0)
    } else {
        0.0
    };
    let date = if width >= if search { 620.0 } else { 450.0 } {
        if search {
            (width * 0.14).clamp(80.0, 130.0)
        } else {
            (width * 0.19).clamp(80.0, 145.0)
        }
    } else {
        0.0
    };
    let location = if search && width >= 130.0 {
        (width - ty - date) * 0.51
    } else {
        0.0
    };
    let mut widths = [0.0, ty, date, location];
    let mut minima = 0.0;
    let mut extra = 0.0;
    for i in 1..4 {
        if widths[i] > 0.0 {
            if preferred[i - 1] != 0 {
                widths[i] = f32::from(preferred[i - 1]).clamp(MIN_COLUMN[i], 32_000.0);
            }
            minima += MIN_COLUMN[i];
            extra += widths[i] - MIN_COLUMN[i];
        }
    }
    let scale = if extra > 0.0 {
        ((width - MIN_COLUMN[0] - minima) / extra).clamp(0.0, 1.0)
    } else {
        1.0
    };
    for i in 1..4 {
        if widths[i] > 0.0 {
            widths[i] = MIN_COLUMN[i] + (widths[i] - MIN_COLUMN[i]) * scale;
        }
    }
    widths[0] = width - widths[1..].iter().sum::<f32>();
    widths
}

pub(super) fn header(ui: &mut Ui, prefs: &mut Preferences, state: &ExplorerState, mut view: DirectoryView, width: f32) -> [f32; 4] {
    let location = state.here.location;
    let search = !state.here.query.trim().is_empty();
    let slot = usize::from(search);
    let mut sizes = columns(width, search, view.column_widths[slot]);
    let height = (ui.fonts_mut(|fonts| fonts.row_height(&FontId::proportional(12.0))).ceil() + 6.0).max(24.0);
    let (band, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let header_id = ui.scope_id().with(("details-heading", location, search));
    let mut previous = 0;
    let mut x = band.left() + sizes[0];
    for i in 1..4 {
        if sizes[i] <= 0.0 {
            continue;
        }
        let grip = Rect::from_center_size(pos2(x, band.center().y), vec2(DIVIDER, band.height()));
        let (dx, commit) = divider(
            ui,
            header_id.with(("divider", i)),
            grip,
            tr!("explorer-resize-columns"),
            (MIN_COLUMN[previous] - sizes[previous]).min(0.0),
            (sizes[i] - MIN_COLUMN[i]).max(0.0),
        );
        sizes[previous] += dx;
        sizes[i] -= dx;
        if commit {
            if previous != 0 {
                view.column_widths[slot][previous - 1] = sizes[previous].round() as u16;
            }
            view.column_widths[slot][i - 1] = sizes[i].round() as u16;
            prefs.set_view(location, view);
        }
        x += dx;
        previous = i;
        x += sizes[i];
    }
    let mut x = band.left();
    for (i, size) in sizes.iter().copied().enumerate().filter(|(_, w)| *w > 0.0) {
        let rect = Rect::from_min_size(pos2(x, band.top()), vec2(size, band.height()));
        x += size;
        let key = [Some(SortKey::Name), Some(SortKey::Type), Some(SortKey::Date), None][i];
        let label = key.map_or_else(|| tr!("explorer-panel-location"), views::sort_label);
        let sortable = key.is_some() && !(state.on_dashboard() && key == Some(SortKey::Date));
        // Keep the column button and its resize grips disjoint.
        let hit = rect.shrink2(vec2(DIVIDER * 0.5, 0.0));
        let r = ui.interact(hit, header_id.with(i), if sortable { Sense::click() } else { Sense::hover() });
        if sortable {
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            if r.hovered() {
                ui.painter().rect_filled(
                    hit,
                    ui.visuals().widgets.inactive.corner_radius,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
        }
        let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let color = ui.visuals().text_color();
        let title = p.layout_no_wrap(label.to_owned(), FontId::proportional(12.0), color);
        let title_end = rect.left() + 5.0 + title.size().x;
        crate::ui::widgets::paint_row_text(&p, title, rect.left() + 5.0, rect.center().y, color);
        let visible_sort = if state.on_dashboard() {
            view.for_dashboard().sort
        } else {
            view.sort
        };
        if key == Some(visible_sort) {
            super::item_art::paint_tabler(
                &p,
                if view.descending { "arrow-down" } else { "arrow-up" },
                Rect::from_center_size(pos2(title_end + 10.0, rect.center().y), vec2(12.0, 12.0)),
                color,
            );
        }
        if let Some(key) = key
            && sortable
            && r.clicked()
        {
            view.descending = if visible_sort == key {
                !view.descending
            } else {
                key == SortKey::Date
            };
            view.sort = key;
            prefs.set_view(location, view);
        }
    }
    sizes
}

#[allow(clippy::too_many_arguments)]
pub(super) fn status(
    ui: &mut Ui,
    rect: Rect,
    inv: &Inventory,
    prefs: &Preferences,
    state: &mut ExplorerState,
    avatar: &str,
    facts: &Facts,
    message: &str,
) {
    let selected = state.here.selected.len() as i64;
    let count = state.total_rows as i64;
    let mut parts = Vec::with_capacity(5);
    if state.native_feedback && !message.is_empty() {
        parts.push(message.to_owned());
    } else if !state.status.is_empty() {
        parts.push(state.status.clone());
    }
    if selected > 0 {
        parts.push(tr!("explorer-panel-selection-count", selected = selected, count = count));
        if selected == 1
            && let Some(&row) = state.here.selected.iter().next()
        {
            parts.push(row_name(inv, prefs, row, state.here.raw, avatar));
            parts.push(views::kind(inv, row));
            if let Some(activity) = row.id().and_then(|id| facts.worn_label(inv, id)) {
                parts.push(activity.to_owned());
            }
        }
        if selected > 1 {
            let folders = state.here.selection_folders() as i64;
            if folders > 0 && folders < selected {
                parts.push(tr!("explorer-status-folders", count = folders));
                let items = selected - folders;
                parts.push(tr!("inventory-items-count", count = items, number = items));
            }
        }
        if state.incomplete {
            parts.push(tr!("explorer-status-loading").into());
        }
    } else if state.incomplete {
        parts.push(tr!("explorer-panel-incomplete-count", count = count));
    } else if count == 0 {
        parts.push(
            if state.here.query.is_empty() {
                tr!("explorer-panel-empty")
            } else {
                tr!("explorer-panel-no-matches")
            }
            .into(),
        );
    } else {
        parts.push(tr!("explorer-panel-entry-count", count = count));
    }
    ui.painter()
        .hline(rect.x_range(), rect.top(), ui.visuals().widgets.noninteractive.bg_stroke);
    // Leave the lower-right window resize handle unobstructed.
    let inner = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + 1.0),
        pos2((rect.right() - 16.0).max(rect.left() + 6.0), rect.bottom()),
    );
    let mut footer = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("explorer-footer")
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    footer.set_clip_rect(inner.intersect(ui.clip_rect()));
    let text = parts.join("  ·  ");
    footer
        .add(egui::Label::new(egui::RichText::new(&text).small()).truncate())
        .on_hover_text(text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

    fn click(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn organized_dividers_drag_without_sorting_and_store_only_completed_adjustments() {
        let ctx = egui::Context::default();
        let mut prefs = Preferences::default();
        let mut state = ExplorerState::at(Location::Library);
        state.here.query = "dress".into();
        let draw = |prefs: &mut Preferences, events: Vec<Event>| {
            let mut regions = None;
            let mut out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 540.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let rect = ui.available_rect_before_wrap();
                    let (left, right, footer) = panes(ui, rect, prefs);
                    let mut main = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(right)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    let width = right.width();
                    let view = prefs.view(state.here.location);
                    let widths = header(&mut main, prefs, &state, view, width);
                    regions = Some((left, right, footer, widths));
                },
            );
            out.textures_delta.clear();
            let (left, right, footer, widths) = regions.unwrap();
            let painted: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::LineSegment { points, .. } if (points[0].y - points[1].y).abs() >= left.height() - 1.0 => {
                        Some(points[0].x)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(painted.len(), 1, "one full-height divider per pane");
            assert!((painted[0] - (left.right() + DIVIDER * 0.5)).abs() <= 0.5);
            (left, right, footer, widths)
        };
        let _ = draw(&mut prefs, vec![]);
        let (left, right, footer, _) = draw(&mut prefs, vec![]);
        assert!(footer.top() > left.bottom() && footer.top() > right.bottom());
        assert_eq!(footer.left(), left.left());
        assert_eq!(footer.right(), right.right());
        let a = pos2(left.right() + DIVIDER * 0.5, left.top() + 90.0);
        let b = a + vec2(54.0, 0.0);
        draw(&mut prefs, vec![Event::PointerMoved(a)]);
        draw(&mut prefs, vec![click(a, true)]);
        let (preview, _, _, _) = draw(&mut prefs, vec![Event::PointerMoved(b)]);
        assert!((preview.width() - left.width() - 54.0).abs() <= 1.0);
        assert_eq!(prefs.pin_width(), None);
        let (_, right, _, old) = draw(&mut prefs, vec![click(b, false)]);
        assert_eq!(prefs.pin_width(), Some(preview.width().round() as u16));

        // Move Location's left boundary toward Date; only these two columns change.
        let a = pos2(right.right() - old[3], right.top() + 12.0);
        let b = a + vec2(-12.0, 0.0);
        draw(&mut prefs, vec![Event::PointerMoved(a)]);
        draw(&mut prefs, vec![click(a, true)]);
        let (_, _, _, preview) = draw(&mut prefs, vec![Event::PointerMoved(b)]);
        let unchanged = prefs.view(state.here.location);
        assert_eq!(unchanged.column_widths, [[0; 3]; 2]);
        for i in 0..2 {
            assert_eq!(old[i], preview[i]);
        }
        assert!((preview[3] - old[3] - 12.0).abs() < 0.1);
        draw(&mut prefs, vec![click(b, false)]);
        let saved: Preferences = serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(saved.pin_width(), prefs.pin_width());
        let view = saved.view(Location::Library);
        assert_eq!(view.sort, unchanged.sort);
        assert_eq!(view.descending, unchanged.descending);
        assert_eq!(view.column_widths[0], [0; 3]);
        assert_eq!(view.column_widths[1][2], preview[3].round() as u16);
        assert_eq!(saved.view(Location::FullTree).column_widths, [[0; 3]; 2]);
        assert_eq!(columns(118.0, true, view.column_widths[1]), [118.0, 0.0, 0.0, 0.0]);
        let (narrow, _, _) = pin_width(320.0, saved.pin_width());
        assert!(narrow < left.width());
        let (wide, _, _) = pin_width(1000.0, saved.pin_width());
        assert_eq!(wide, f32::from(saved.pin_width().unwrap()));
    }

    #[test]
    fn organized_columns_stay_inside_the_pane_for_saved_extremes_and_hidden_headers() {
        for width in [82.0, 130.0, 154.0, 180.0, 419.0, 420.0, 449.0, 450.0, 619.0, 620.0, 1800.0] {
            for search in [true, false] {
                let cols = columns(width, search, [65535, 65535, 65535]);
                assert!((cols.iter().sum::<f32>() - width).abs() < 0.1);
                assert!(cols[0] >= MIN_COLUMN[0] - 0.1 && cols.iter().all(|v| *v >= 0.0));
            }
        }
    }
}
