//! "Rules" (LLPanelLandCovenant::refresh, process_covenant_reply).

use super::{View, dim, row, text};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::region_flags as rf;
use egui::RichText;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, v: &View, world: &mut World) {
    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    row(ui, p, aurora_i18n::tr!("ui-land-covenant-estate"), |ui| {
        text(ui, p, covenant.as_ref().map_or("", |c| c.estate_name.as_str()));
    });
    row(ui, p, aurora_i18n::tr!("common-owner-2"), |ui| match &covenant {
        Some(c) if !c.estate_owner.is_nil() => super::agent_link(ui, p, world, c.estate_owner),
        _ => {
            dim(ui, p, aurora_i18n::tr!("status-none"));
        }
    });
    let body = match &covenant {
        Some(c) => c.text.clone().unwrap_or_else(|| aurora_i18n::tr!("common-loading").into()),
        None => aurora_i18n::tr!("common-loading").into(),
    };
    egui::Frame::new()
        .fill(p.field)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("covenant_text")
                .max_height(190.0)
                .min_scrolled_height(190.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(body).size(12.0).color(p.ink))
                            .wrap()
                            .selectable(true),
                    );
                });
        });
    // covenant_timestamp_text, right-aligned
    if let Some(c) = &covenant {
        let w = ui.available_width();
        ui.allocate_ui_with_layout(egui::vec2(w, 18.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let when = if c.timestamp == 0 {
                aurora_i18n::tr!("status-never").to_owned()
            } else {
                super::covenant_date(c.timestamp as i64)
            };
            dim(ui, p, aurora_i18n::tr!("ui-land-covenant-last-modified", when = when.to_string()));
        });
    }
    let region = v.region.as_ref();
    row(ui, p, aurora_i18n::tr!("ui-land-covenant-region"), |ui| {
        text(ui, p, region.map_or(aurora_i18n::tr!("common-loading"), |r| r.name.as_str()));
    });
    row(ui, p, aurora_i18n::tr!("common-type-2"), |ui| {
        text(ui, p, super::product_name(region.map_or("", |r| r.product.as_str())));
    });
    row(ui, p, aurora_i18n::tr!("common-category"), |ui| super::maturity(ui, p, v));
    row(ui, p, aurora_i18n::tr!("ui-land-covenant-resell"), |ui| {
        text(
            ui,
            p,
            if v.region_flag(rf::BLOCK_LAND_RESELL) {
                aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-resold")
            } else {
                aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-resold")
            },
        );
    });
    row(ui, p, aurora_i18n::tr!("ui-land-covenant-subdivide"), |ui| {
        text(
            ui,
            p,
            if v.region_flag(rf::ALLOW_PARCEL_CHANGES) {
                aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-can-be-joined-or")
            } else {
                aurora_i18n::tr!("ui-land-covenant-land-bought-in-this-region-cannot-be-joined-or")
            },
        );
    });
}
