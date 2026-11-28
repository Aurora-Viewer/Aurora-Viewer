//! Region Tracker window (the Legacy Viewer's ALFloaterRegionTracker,
//! floater_region_tracker.xml): label, maturity, region and avatar count of
//! the regions the account watches, redrawn every 5 s while it is open.
//!
//! The list opens sorted by Label from A to Z and keeps the sort chosen
//! between sessions, as the Legacy Viewer's `region_list` does
//! (`sort_column="0"`, `persist_sort_order`; settings `region_tracker_sort`).
//! A click on a column header sorts on it, again for the other direction.
//! Deliberate differences from the Legacy Viewer's scroll list
//! (`LLScrollListCtrl::onClickColumn`, which sorts every column ascending on
//! the first click, only toggles the direction after, and compares the text
//! shown with `LLStringUtil::compareDict`):
//! - Avatars sorts the most first on its first click;
//! - a third click goes back to the order the regions were saved in, as in
//!   the Blacklist window;
//! - a count or a maturity not known yet, and an offline region's maturity,
//!   stay last in both directions (the Legacy Viewer sorts `"..."` before
//!   the counts);
//! - Maturity sorts by rating (General, Moderate, Adult), not by the text
//!   or icon shown;
//! - an empty label (Aurora allows one) sorts as its region's name.

use super::widgets::{self, Floater};
use crate::settings::{RegionTrackerColumn as Column, RegionTrackerSort};
use crate::theme::Palette;
use crate::world::{
    World,
    region_tracker::{RegionState, RegionStatus, status},
    worldmap::SIM_ACCESS_DOWN,
};
use egui::{Pos2, RichText, Sense, Vec2};
use std::time::{Duration, Instant};

pub const WINDOW: &str = "region_tracker";

/// ALFloaterRegionTracker's LLEventTimer(5.f): the rows are rebuilt (and the
/// throttled requests of `World::refresh_tracked_regions` checked) this often.
const REDRAW: Duration = Duration::from_secs(5);
const ROW_H: f32 = 24.0;
/// Label, maturity, region, avatars (the columns of region_list).
const WEIGHTS: [f32; 4] = [0.32, 0.18, 0.36, 0.14];

struct Row {
    name: String,
    label: String,
    status: RegionStatus,
}

impl Column {
    const ALL: [Column; 4] = [Column::Label, Column::Maturity, Column::Region, Column::Avatars];

    fn title(self) -> &'static str {
        match self {
            Column::Label => aurora_i18n::tr!("region-tracker-label"),
            Column::Maturity => aurora_i18n::tr!("region-rules-maturity"),
            Column::Region => aurora_i18n::tr!("region-tracker-region"),
            Column::Avatars => aurora_i18n::tr!("region-tracker-avatars"),
        }
    }

    /// The direction of a first click: the most avatars first, the other
    /// columns from A to Z.
    fn first_descending(self) -> bool {
        self == Column::Avatars
    }
}

/// The sort after a click on `column`: its first direction, then the other
/// one, then none (the saved order).
fn next_sort(sort: Option<(Column, bool)>, column: Column) -> Option<(Column, bool)> {
    match sort {
        Some((c, descending)) if c == column => (descending == column.first_descending()).then_some((column, !descending)),
        _ => Some((column, column.first_descending())),
    }
}

/// The sort kept in the settings, as the window holds it.
fn from_saved(saved: RegionTrackerSort) -> Option<(Column, bool)> {
    saved.column.map(|c| (c, saved.descending))
}

/// The window's sort, as the settings keep it.
fn to_saved(sort: Option<(Column, bool)>) -> RegionTrackerSort {
    RegionTrackerSort {
        column: sort.map(|(c, _)| c),
        descending: sort.is_some_and(|(_, d)| d),
    }
}

/// Orders `rows` on `sort` (None keeps them as given). A count or a
/// maturity not known yet, and an offline region's maturity, go last in
/// either direction; an empty label sorts as its region's name; equal rows
/// keep the region's name order.
fn sort_rows(rows: &mut [Row], sort: Option<(Column, bool)>) {
    let Some((column, descending)) = sort else {
        return;
    };
    let text = crate::natural_order::cmp;
    fn label(r: &Row) -> &str {
        if r.label.is_empty() { &r.name } else { &r.label }
    }
    rows.sort_by(|a, b| {
        let known = |r: &Row| match column {
            Column::Maturity => r.status.access.is_some_and(|a| a != SIM_ACCESS_DOWN),
            Column::Avatars => r.status.agents.is_some(),
            Column::Label | Column::Region => true,
        };
        let order = match column {
            Column::Label => text(label(a), label(b)),
            Column::Maturity => a.status.access.cmp(&b.status.access),
            Column::Region => text(&a.name, &b.name),
            Column::Avatars => a.status.agents.cmp(&b.status.agents),
        };
        let order = if descending { order.reverse() } else { order };
        known(b).cmp(&known(a)).then(order).then_with(|| text(&a.name, &b.name))
    });
}

#[derive(Default)]
pub struct RegionTrackerUi {
    name: String,
    label: String,
    selected: Option<String>,
    rows: Vec<Row>,
    /// The column sorted on and whether descending; None: the saved order.
    sort: Option<(Column, bool)>,
    /// When the rows were last rebuilt; None while closed, so that opening
    /// asks the regions at once (ALFloaterRegionTracker::onOpen).
    refreshed: Option<Instant>,
}

impl RegionTrackerUi {
    /// The World Map's Track Region (LLFloaterWorldMap::onTrackRegion): the
    /// region's name, its label offered with its current one or its name
    /// (notification RegionTrackerAdd).
    pub fn track_region(&mut self, world: &World, name: String) {
        self.label = world
            .region_tracker
            .regions()
            .iter()
            .find(|r| r.name.eq_ignore_ascii_case(&name))
            .map(|r| r.label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| name.clone());
        self.name = name;
    }

    fn rebuild(&mut self, world: &mut World) {
        world.refresh_tracked_regions();
        let own = world.main_origin().map(|(x, y)| (x / 256, y / 256));
        self.rows = world
            .region_tracker
            .regions()
            .iter()
            .map(|r| Row {
                name: r.name.clone(),
                label: r.label.clone(),
                status: status(&world.map, &r.name, own),
            })
            .collect();
        sort_rows(&mut self.rows, self.sort);
        // the name as the list keeps it: "ahern" added again selects the
        // tracked "Ahern"
        self.selected = self
            .selected
            .take()
            .and_then(|sel| self.rows.iter().find(|r| r.name.eq_ignore_ascii_case(&sel)).map(|r| r.name.clone()));
        self.refreshed = Some(Instant::now());
    }

    /// ALFloaterRegionTracker::openMap: the world map on the region, or
    /// simply shown without one: opened, or brought to the front, expanded
    /// when it is minimized (LLFloaterReg::showInstance →
    /// LLFloater::openFloater).
    fn open_map(ctx: &egui::Context, world: &mut World, region: Option<&str>, map_open: &mut bool) {
        if let Some(name) = region {
            world.map.track_region(name, glam::Vec3::new(128.0, 128.0, 0.0));
            world.map.focus_track = true;
        }
        widgets::show_floater(ctx, "worldmap", map_open);
    }

    /// `saved`: the sort kept between sessions, written back when a header
    /// is clicked.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        open: &mut bool,
        map_open: &mut bool,
        saved: &mut RegionTrackerSort,
    ) {
        if !*open {
            self.refreshed = None;
            return;
        }
        self.sort = from_saved(*saved);
        if self.refreshed.is_none_or(|t| t.elapsed() >= REDRAW) {
            self.rebuild(world);
        }
        let screen = ctx.content_rect();
        let pos = Pos2::new((screen.width() - 620.0).max(0.0) * 0.5, 100.0);
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-region-tracker"),
            pos,
            Vec2::new(620.0, 420.0),
        );
        floater.min_size = Vec2::new(420.0, 260.0);
        let mut changed = false;
        floater.show(ctx, p, open, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("region-tracker-region"));
                ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(160.0));
                ui.label(aurora_i18n::tr!("region-tracker-label"));
                ui.add(egui::TextEdit::singleline(&mut self.label).desired_width(140.0));
                // a tracked name again: its label is replaced
                if ui
                    .add_enabled(!self.name.trim().is_empty(), egui::Button::new(aurora_i18n::tr!("common-add")))
                    .clicked()
                {
                    world.add_tracked_region(&self.name, &self.label);
                    self.selected = Some(self.name.trim().to_owned());
                    self.name.clear();
                    self.label.clear();
                    changed = true;
                }
                // the region's name is hidden under RLVa @showloc
                let here = world.region_name();
                if ui
                    .add_enabled(
                        !here.is_empty() && !world.rlva.has("showloc"),
                        egui::Button::new(aurora_i18n::tr!("region-tracker-add-current")),
                    )
                    .clicked()
                {
                    // keeps the label of a region already tracked
                    let label = world
                        .region_tracker
                        .regions()
                        .iter()
                        .find(|r| r.name.eq_ignore_ascii_case(&here))
                        .map(|r| r.label.clone())
                        .unwrap_or_default();
                    world.add_tracked_region(&here, &label);
                    changed = true;
                }
            });
            ui.add_space(4.0);
            let list = ui.scope_id().with("region_tracker_rows");
            let width = widgets::header_width(ui, list);
            let mut resort = false;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (i, column) in Column::ALL.into_iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width * WEIGHTS[i], 22.0), Sense::click());
                    let sorted = self.sort.filter(|(c, _)| *c == column);
                    let color = if sorted.is_some() || resp.hovered() { p.ink } else { p.muted };
                    let room = rect.width() - if sorted.is_some() { 22.0 } else { 8.0 };
                    widgets::paint_elided(
                        ui,
                        column.title(),
                        egui::pos2(rect.left() + 4.0, rect.center().y),
                        room,
                        egui::FontId::proportional(12.0),
                        color,
                    );
                    if let Some((_, descending)) = sorted
                        && let Some(texture) = crate::ui::icons::global(if descending { "caret-down-filled" } else { "caret-up-filled" })
                    {
                        let at = egui::Rect::from_center_size(egui::pos2(rect.right() - 10.0, rect.center().y), Vec2::splat(8.0));
                        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                        ui.painter().image(texture.id(), at, uv, p.violet_light);
                    }
                    if resp.clicked() {
                        self.sort = next_sort(self.sort, column);
                        *saved = to_saved(self.sort);
                        resort = true;
                    }
                }
            });
            if resort {
                if self.sort.is_some() {
                    sort_rows(&mut self.rows, self.sort);
                } else {
                    changed = true;
                }
            }
            // the list takes the room the buttons leave, measured, not guessed
            let (mut rows_ui, footer) = widgets::above_footer(ui);
            let mut open_region = None;
            let out = egui::ScrollArea::vertical()
                .id_salt("region_tracker_rows")
                .auto_shrink([false, false])
                .show(&mut rows_ui, |ui| {
                    for row in &self.rows {
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                        let selected = self.selected.as_deref() == Some(row.name.as_str());
                        if selected || response.hovered() {
                            let fill = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
                            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
                        }
                        // alfloaterregiontracker.cpp: grey while unknown, red
                        // with 0 avatars once down
                        let color = match row.status.state {
                            RegionState::Unknown => p.muted,
                            RegionState::Down => p.danger,
                            RegionState::Online => p.ink,
                        };
                        let maturity = match row.status.state {
                            RegionState::Unknown => "",
                            _ => row.status.access.map(crate::world::worldmap::access_label).unwrap_or_default(),
                        };
                        let count = row.status.agents.map_or_else(|| "…".to_owned(), |n| n.to_string());
                        let texts = [row.label.as_str(), maturity, row.name.as_str(), count.as_str()];
                        let mut x = rect.left();
                        for (column, text) in texts.into_iter().enumerate() {
                            let w = rect.width() * WEIGHTS[column];
                            widgets::paint_elided(
                                ui,
                                text,
                                egui::pos2(x + 4.0, rect.center().y),
                                w - 8.0,
                                egui::FontId::proportional(13.0),
                                color,
                            );
                            x += w;
                        }
                        if response.clicked() {
                            // the row's label goes to the fields: Add then
                            // changes it
                            self.selected = Some(row.name.clone());
                            self.name = row.name.clone();
                            self.label = row.label.clone();
                        }
                        if response.double_clicked() {
                            open_region = Some(row.name.clone());
                        }
                    }
                    if self.rows.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("common-none")).color(p.muted));
                    }
                });
            widgets::remember_scroll_bar(&rows_ui, list, &out);
            footer.show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new(aurora_i18n::tr!("common-remove")))
                        .clicked()
                        && let Some(name) = self.selected.take()
                    {
                        world.remove_tracked_region(&name);
                        changed = true;
                    }
                    if ui.button(aurora_i18n::tr!("common-world-map")).clicked() {
                        open_region = Some(self.selected.clone().unwrap_or_default());
                    }
                });
            });
            if let Some(name) = open_region {
                Self::open_map(ctx, world, Some(name.as_str()).filter(|n| !n.is_empty()), map_open);
            }
        });
        if changed {
            self.rebuild(world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for i in 0..30 {
            world.region_tracker.add(&format!("Region {i}"), &format!("Label {i}"));
        }
        world
    }

    /// The list takes the room the buttons leave: the window keeps its size
    /// at every interface scale and text size, its buttons in view.
    #[test]
    fn the_window_keeps_its_size() {
        let mut world = world();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(620.0, 420.0), Vec2::new(420.0, 260.0)],
            aurora_i18n::tr!("common-world-map"),
            RegionTrackerUi::default,
            |ui, ctx, p| ui.show(ctx, p, &mut world, &mut true, &mut false, &mut RegionTrackerSort::default()),
        );
    }

    /// World map, or a double-clicked row, expands a minimized map instead
    /// of leaving it on its title bar (LLFloater::openFloater).
    #[test]
    fn world_map_expands_a_minimized_map() {
        let ctx = egui::Context::default();
        let mut world = world();
        crate::ui::widgets::minimize_floater(&ctx, "worldmap");
        let mut map_open = true;
        RegionTrackerUi::open_map(&ctx, &mut world, Some("Region 3"), &mut map_open);
        assert!(map_open);
        assert!(crate::ui::widgets::floater_expand_pending(&ctx, "worldmap"));
        assert!(
            crate::ui::widgets::take_floater_shown(&ctx, "worldmap"),
            "its search takes the keyboard"
        );
        assert!(world.map.focus_track);
    }

    fn row(name: &str, label: &str, agents: Option<u32>, access: Option<u8>) -> Row {
        Row {
            name: name.to_owned(),
            label: label.to_owned(),
            status: RegionStatus {
                state: if agents.is_some() {
                    RegionState::Online
                } else {
                    RegionState::Unknown
                },
                agents,
                access,
            },
        }
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    /// Avatars sort the most first, then the fewest first, then back to the
    /// saved order; a count not known yet stays last.
    #[test]
    fn avatars_sort_the_most_first_and_unknown_counts_last() {
        let mut rows = vec![
            row("Region 1", "b", Some(3), Some(13)),
            row("Region 2", "a", None, None),
            row("Region 3", "c", Some(12), Some(21)),
            row("Region 4", "", Some(0), Some(42)),
        ];
        let first = next_sort(None, Column::Avatars);
        assert_eq!(first, Some((Column::Avatars, true)));
        sort_rows(&mut rows, first);
        assert_eq!(names(&rows), ["Region 3", "Region 1", "Region 4", "Region 2"]);
        let second = next_sort(first, Column::Avatars);
        sort_rows(&mut rows, second);
        assert_eq!(names(&rows), ["Region 4", "Region 1", "Region 3", "Region 2"]);
        assert_eq!(
            next_sort(second, Column::Avatars),
            None,
            "a third click goes back to the saved order"
        );
    }

    /// Names sort from A to Z first, in natural order and whatever the case.
    #[test]
    fn regions_and_labels_sort_from_a_to_z_first() {
        let mut rows = vec![
            row("region 10", "Zed", Some(1), None),
            row("Region 9", "alpha", Some(1), None),
            row("Abbot", "Mid", Some(1), None),
        ];
        let by_region = next_sort(None, Column::Region);
        assert_eq!(by_region, Some((Column::Region, false)));
        sort_rows(&mut rows, by_region);
        assert_eq!(names(&rows), ["Abbot", "Region 9", "region 10"]);
        sort_rows(&mut rows, next_sort(by_region, Column::Region));
        assert_eq!(names(&rows), ["region 10", "Region 9", "Abbot"]);
        sort_rows(&mut rows, next_sort(by_region, Column::Label));
        assert_eq!(names(&rows), ["Region 9", "Abbot", "region 10"]);
    }

    /// An empty label sorts as its region's name, not ahead of every label.
    #[test]
    fn an_empty_label_sorts_as_its_region_name() {
        let mut rows = vec![
            row("Region 1", "Zed", Some(1), None),
            row("Moor", "", Some(1), None),
            row("Region 2", "Alpha", Some(1), None),
        ];
        sort_rows(&mut rows, Some((Column::Label, false)));
        assert_eq!(names(&rows), ["Region 2", "Moor", "Region 1"]);
        sort_rows(&mut rows, Some((Column::Label, true)));
        assert_eq!(names(&rows), ["Region 1", "Moor", "Region 2"]);
    }

    /// Maturity sorts by rating (General, Moderate, Adult); a rating not
    /// known yet and an offline region stay last in both directions.
    #[test]
    fn maturity_sorts_by_rating_and_unknown_or_offline_last() {
        use crate::world::worldmap::{SIM_ACCESS_ADULT, SIM_ACCESS_MATURE, SIM_ACCESS_PG};
        let mut rows = vec![
            row("Region 1", "", Some(1), Some(SIM_ACCESS_ADULT)),
            row("Region 2", "", None, None),
            row("Region 3", "", Some(1), Some(SIM_ACCESS_PG)),
            row("Region 4", "", Some(0), Some(SIM_ACCESS_DOWN)),
            row("Region 5", "", Some(1), Some(SIM_ACCESS_MATURE)),
        ];
        let up = next_sort(None, Column::Maturity);
        assert_eq!(up, Some((Column::Maturity, false)));
        sort_rows(&mut rows, up);
        assert_eq!(names(&rows), ["Region 3", "Region 5", "Region 1", "Region 2", "Region 4"]);
        sort_rows(&mut rows, next_sort(up, Column::Maturity));
        assert_eq!(names(&rows)[..3], ["Region 1", "Region 5", "Region 3"]);
        let mut last = names(&rows)[3..].to_vec();
        last.sort_unstable();
        assert_eq!(last, ["Region 2", "Region 4"], "unknown and offline stay last going down too");
    }

    /// The sort holds when the list is rebuilt every 5 s.
    #[test]
    fn the_sort_holds_across_rebuilds() {
        let mut world = world();
        let mut ui = RegionTrackerUi {
            sort: Some((Column::Region, true)),
            ..Default::default()
        };
        ui.rebuild(&mut world);
        assert_eq!(ui.rows.first().map(|r| r.name.as_str()), Some("Region 29"));
        assert_eq!(ui.rows.last().map(|r| r.name.as_str()), Some("Region 0"));
    }

    /// The window opens on the kept sort (Label from A to Z by default), and
    /// a header click writes the new one back, the saved order included.
    #[test]
    fn the_window_opens_on_the_kept_sort_and_writes_clicks_back() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for (name, label) in [("Region 1", "Shop"), ("Region 2", "Beach"), ("Region 3", "Club")] {
            world.region_tracker.add(name, label);
        }
        let p = crate::theme::Theme::default().palette();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let mut ui = RegionTrackerUi::default();
        let mut saved = RegionTrackerSort::default();
        frames.run(Vec::new(), |ctx_ui| {
            ui.show(ctx_ui.ctx(), &p, &mut world, &mut true, &mut false, &mut saved)
        });
        assert_eq!(names(&ui.rows), ["Region 2", "Region 3", "Region 1"]);
        // two more clicks on Label: Z to A, then the saved order
        let mut sort = ui.sort;
        assert_eq!(sort, Some((Column::Label, false)));
        for expected in [Some((Column::Label, true)), None] {
            sort = next_sort(sort, Column::Label);
            assert_eq!(sort, expected);
            saved = to_saved(sort);
            assert_eq!(from_saved(saved), expected);
        }
        assert_eq!(saved.column, None, "the saved order is kept as no column");
    }

    /// A region added again under another case keeps the list's spelling
    /// and stays selected.
    #[test]
    fn a_region_added_again_in_another_case_stays_selected() {
        let mut world = world();
        let mut ui = RegionTrackerUi {
            selected: Some("region 3".to_owned()),
            ..Default::default()
        };
        ui.rebuild(&mut world);
        assert_eq!(ui.selected.as_deref(), Some("Region 3"));
        ui.selected = Some("Nowhere".to_owned());
        ui.rebuild(&mut world);
        assert_eq!(ui.selected, None, "a region no longer tracked is unselected");
    }
}
