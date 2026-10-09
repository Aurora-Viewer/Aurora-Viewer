//! "Règlement" (LLPanelLandCovenant::refresh, process_covenant_reply).

use super::{View, dim, row, text};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::region_flags as rf;
use egui::RichText;

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, v: &View, world: &mut World) {
    let covenant = world.land.covenant.clone().filter(|c| c.handle == v.handle);
    row(ui, p, "Domaine :", |ui| {
        text(ui, p, covenant.as_ref().map_or("", |c| c.estate_name.as_str()));
    });
    row(ui, p, "Propriétaire :", |ui| match &covenant {
        Some(c) if !c.estate_owner.is_nil() => super::agent_link(ui, p, world, c.estate_owner),
        _ => {
            dim(ui, p, "(aucun)");
        }
    });
    let body = match &covenant {
        Some(c) => c.text.clone().unwrap_or_else(|| "Chargement...".into()),
        None => "Chargement...".into(),
    };
    egui::Frame::new()
        .fill(p.field)
        .stroke(egui::Stroke::new(1.0, p.raised))
        .corner_radius(egui::CornerRadius::same(2))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("covenant_text")
                .max_height(190.0)
                .min_scrolled_height(190.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(body).size(12.0).color(p.ink)).wrap().selectable(true));
                });
        });
    // covenant_timestamp_text, right-aligned
    if let Some(c) = &covenant {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let when = if c.timestamp == 0 {
                " (jamais)".to_owned()
            } else {
                super::covenant_date(c.timestamp as i64)
            };
            dim(ui, p, format!("Dernière modification :{when}"));
        });
    }
    let region = v.region.as_ref();
    row(ui, p, "Région :", |ui| {
        text(ui, p, region.map_or("Chargement...", |r| r.name.as_str()));
    });
    row(ui, p, "Type :", |ui| {
        text(ui, p, super::product_name(region.map_or("", |r| r.product.as_str())));
    });
    row(ui, p, "Catégorie :", |ui| super::maturity(ui, p, v));
    row(ui, p, "Revendre :", |ui| {
        text(
            ui,
            p,
            if v.region_flag(rf::BLOCK_LAND_RESELL) {
                "Le terrain acheté dans cette région ne peut pas être revendu."
            } else {
                "Le terrain acheté dans cette région peut être revendu."
            },
        );
    });
    row(ui, p, "Sous-diviser :", |ui| {
        text(
            ui,
            p,
            if v.region_flag(rf::ALLOW_PARCEL_CHANGES) {
                "Le terrain acheté dans cette région peut être fusionné ou divisé."
            } else {
                "Le terrain acheté dans cette région ne peut pas être fusionné ou divisé."
            },
        );
    });
}
