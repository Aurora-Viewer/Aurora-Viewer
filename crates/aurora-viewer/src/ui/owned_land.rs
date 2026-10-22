//! LLFloaterLandHoldings (indra/newview/llfloaterlandholdings.cpp,
//! originally LGPL 2.1): personal parcels and contributions to groups.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use uuid::Uuid;

pub const WINDOW: &str = "owned_land";

/// Returns true when the world map should open on a selected parcel.
pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World, open: &mut bool) -> bool {
    if !*open {
        return false;
    }
    world.search.want_land_types();
    let mut show_map = false;
    let mut map_destination = None;
    let agent_z = world.agent_global().map_or(0.0, |g| g.z as f32);
    let mut refresh = false;
    let mut window = Floater::new(
        WINDOW,
        aurora_i18n::tr!("owned-land-title"),
        egui::pos2(340.0, 140.0),
        egui::vec2(660.0, 440.0),
    );
    window.min_size = egui::vec2(460.0, 240.0);
    window.show(ctx, p, open, |ui| {
        if widgets::action_button(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("owned-land-refresh"),
            widgets::ActionKind::Flat,
            egui::vec2(0.0, 28.0),
        )
        .clicked()
        {
            refresh = true;
        }
        widgets::divider(ui, p);
        egui::ScrollArea::vertical().show(ui, |ui| {
            let list = world.group_money.get(&Uuid::nil()).and_then(|g| g.parcels.as_ref());
            if let Some(list) = list {
                let mut order: Vec<_> = list.iter().collect();
                order.sort_by_cached_key(|v| v.name.to_lowercase());
                for parcel in order {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 72.0).max(120.0));
                            ui.label(egui::RichText::new(&parcel.name).strong());
                            let x = parcel.global_x.rem_euclid(256.0);
                            let y = parcel.global_y.rem_euclid(256.0);
                            ui.label(format!("{} ({x:.0}, {y:.0})", parcel.sim_name));
                            ui.label(aurora_i18n::tr!(
                                "owned-land-area",
                                actual = parcel.actual_area,
                                billable = parcel.billable_area
                            ));
                            if let Some(kind) = world.search.land_types.get(&parcel.sku) {
                                ui.label(kind);
                            }
                        });
                        if widgets::icon_button(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2"), true) {
                            map_destination = Some((f64::from(parcel.global_x), f64::from(parcel.global_y)));
                            show_map = true;
                        }
                        if widgets::icon_button(ui, p, "world-filled", aurora_i18n::tr!("common-teleport"), true) {
                            let pos = glam::Vec3::new(
                                parcel.global_x.rem_euclid(256.0),
                                parcel.global_y.rem_euclid(256.0),
                                parcel.global_z,
                            );
                            super::context::request(ctx, super::context::CtxAction::TeleportToPlace(parcel.sim_name.clone(), pos));
                        }
                    });
                    ui.add_space(8.0);
                }
                if list.is_empty() {
                    ui.label(aurora_i18n::tr!("owned-land-empty"));
                }
                let actual: i64 = list.iter().map(|p| i64::from(p.actual_area)).sum();
                let billable: i64 = list.iter().map(|p| i64::from(p.billable_area)).sum();
                ui.label(aurora_i18n::tr!("owned-land-total", actual = actual, billable = billable));
            } else {
                ui.label(aurora_i18n::tr!("common-loading-2"));
            }
            widgets::divider(ui, p);
            ui.label(egui::RichText::new(aurora_i18n::tr!("owned-land-contributions")).strong());
            for group in world.groups.sorted() {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Label::new(egui::RichText::new(&group.name).color(p.violet_light)).sense(egui::Sense::click()))
                        .clicked()
                    {
                        super::group_profile::request_open(ctx, group.id);
                    }
                    ui.label(aurora_i18n::tr!("owned-land-contribution", area = group.contribution));
                });
            }
        });
    });
    if let Some((x, y)) = map_destination {
        world.map.track_location(x, y, agent_z, false);
    }
    if refresh {
        world.group_money.request_owned_land();
    }
    show_map
}
