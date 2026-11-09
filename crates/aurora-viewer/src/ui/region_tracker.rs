//! Region Tracker window (the Legacy Viewer's ALFloaterRegionTracker,
//! floater_region_tracker.xml): label, maturity, region and avatar count of
//! the regions the account watches, redrawn every 5 s while it is open.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::{
    World,
    region_tracker::{RegionState, RegionStatus, status},
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

#[derive(Default)]
pub struct RegionTrackerUi {
    name: String,
    label: String,
    selected: Option<String>,
    rows: Vec<Row>,
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
        // the name as the list keeps it: "ahern" added again selects the
        // tracked "Ahern"
        self.selected = self
            .selected
            .take()
            .and_then(|sel| self.rows.iter().find(|r| r.name.eq_ignore_ascii_case(&sel)).map(|r| r.name.clone()));
        self.refreshed = Some(Instant::now());
    }

    /// ALFloaterRegionTracker::openMap: the world map on the region, or
    /// simply opened without one.
    fn open_map(world: &mut World, region: Option<&str>, map_open: &mut bool) {
        if let Some(name) = region {
            world.map.track_region(name, glam::Vec3::new(128.0, 128.0, 0.0));
            world.map.focus_track = true;
        }
        *map_open = true;
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, open: &mut bool, map_open: &mut bool) {
        if !*open {
            self.refreshed = None;
            return;
        }
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
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (column, label) in [
                    aurora_i18n::tr!("region-tracker-label"),
                    aurora_i18n::tr!("region-rules-maturity"),
                    aurora_i18n::tr!("region-tracker-region"),
                    aurora_i18n::tr!("region-tracker-avatars"),
                ]
                .into_iter()
                .enumerate()
                {
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(width * WEIGHTS[column], 22.0), Sense::hover());
                    widgets::paint_elided(
                        ui,
                        label,
                        egui::pos2(rect.left() + 4.0, rect.center().y),
                        rect.width() - 8.0,
                        egui::FontId::proportional(12.0),
                        p.muted,
                    );
                }
            });
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
                Self::open_map(world, Some(name.as_str()).filter(|n| !n.is_empty()), map_open);
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
            |ui, ctx, p| ui.show(ctx, p, &mut world, &mut true, &mut false),
        );
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
