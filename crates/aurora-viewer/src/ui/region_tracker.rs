//! Region Tracker window.

use super::widgets::Floater;
use crate::theme::Palette;
use crate::world::{
    World,
    region_tracker::{RegionState, status},
};
use egui::{Pos2, RichText, Vec2};

pub const WINDOW: &str = "region_tracker";

#[derive(Default)]
pub struct RegionTrackerUi {
    name: String,
    label: String,
    selected: Option<String>,
}

impl RegionTrackerUi {
    pub fn track_region(&mut self, name: String) {
        self.name = name;
        self.label.clear();
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World, open: &mut bool, map_open: &mut bool) {
        if !*open {
            return;
        }
        world.refresh_tracked_regions();
        let screen = ctx.content_rect();
        let pos = Pos2::new((screen.width() - 620.0).max(0.0) * 0.5, 100.0);
        Floater::new(
            WINDOW,
            aurora_i18n::tr!("toolbar-command-region-tracker"),
            pos,
            Vec2::new(620.0, 420.0),
        )
        .show(ctx, p, open, |ui| {
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("region-tracker-region"));
                ui.text_edit_singleline(&mut self.name);
                ui.label(aurora_i18n::tr!("region-tracker-label"));
                ui.text_edit_singleline(&mut self.label);
                if ui.button(aurora_i18n::tr!("common-add")).clicked() {
                    world.add_tracked_region(&self.name, &self.label);
                    self.name.clear();
                    self.label.clear();
                }
                if ui.button(aurora_i18n::tr!("region-tracker-add-current")).clicked() {
                    let name = world.region_name();
                    world.add_tracked_region(&name, "");
                }
            });
            super::widgets::divider(ui, p);
            ui.horizontal(|ui| {
                ui.strong(aurora_i18n::tr!("region-tracker-label"));
                ui.add_space(18.0);
                ui.strong(aurora_i18n::tr!("region-rules-maturity"));
                ui.add_space(20.0);
                ui.strong(aurora_i18n::tr!("region-tracker-region"));
                ui.add_space(20.0);
                ui.strong(aurora_i18n::tr!("region-tracker-avatars"));
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for region in world.region_tracker.regions() {
                    let s = status(&world.map, &region.name, world.main_origin().map(|(x, y)| (x / 256, y / 256)));
                    let color = match s.state {
                        RegionState::Unknown => p.muted,
                        RegionState::Down => p.danger,
                        RegionState::Online => p.ink,
                    };
                    let access = s
                        .access
                        .map(crate::world::worldmap::access_label)
                        .unwrap_or(aurora_i18n::tr!("common-unknown"));
                    let count = s.agents.map(|n| n.to_string()).unwrap_or_else(|| "—".into());
                    let response = ui
                        .horizontal(|ui| {
                            ui.label(&region.label);
                            ui.add_space(18.0);
                            ui.label(access);
                            ui.add_space(20.0);
                            ui.label(RichText::new(&region.name).color(color));
                            ui.add_space(20.0);
                            ui.label(count);
                        })
                        .response;
                    if response.clicked() {
                        self.selected = Some(region.name.clone());
                    }
                    if response.double_clicked() {
                        world.map.track_region(&region.name, glam::Vec3::new(128.0, 128.0, 30.0));
                        *map_open = true;
                    }
                }
            });
            ui.horizontal(|ui| {
                if ui.button(aurora_i18n::tr!("common-remove")).clicked()
                    && let Some(name) = self.selected.take()
                {
                    world.remove_tracked_region(&name);
                }
                if ui.button(aurora_i18n::tr!("common-world-map")).clicked() {
                    *map_open = true;
                }
            });
        });
    }
}
