//! LLFavoritesBarCtrl::handleHover / LLFolderView::startDrag expose the same
//! landmark from favorites and Places (indra/newview, indra/llui, LGPL 2.1).
//! This cargo keeps list reordering separate from inventory moves and gifts.

use crate::{theme::Palette, ui::widgets};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct LandmarkDrag {
    pub item: Uuid,
    pub favorite: Option<Uuid>,
    pub name: String,
    pub thumbnail: Option<egui::TextureId>,
}

pub(crate) fn source(response: &egui::Response, cargo: impl FnOnce() -> LandmarkDrag) {
    if response.drag_started() {
        response.dnd_set_drag_payload(cargo());
    }
}

/// Draw after all source windows, so closing a popup cannot hide the preview.
pub(crate) fn ghost(ctx: &egui::Context, p: &Palette) {
    let Some(cargo) = egui::DragAndDrop::payload::<LandmarkDrag>(ctx) else {
        return;
    };
    let Some(pointer) = ctx.pointer_interact_pos() else { return };
    widgets::drag_ghost(ctx, p, "place_drag_preview", pointer + egui::vec2(16.0, 20.0), |ui| {
        ui.set_max_width(280.0);
        widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
            ui.horizontal(|ui| {
                if let Some(texture) = cargo.thumbnail {
                    ui.add(egui::Image::new((texture, egui::vec2(28.0, 28.0))));
                } else if let Some(icon) = crate::ui::icons::global("map-pin-filled") {
                    ui.add(egui::Image::new((icon.id(), egui::vec2(24.0, 24.0))).tint(p.violet_light));
                }
                ui.add(egui::Label::new(egui::RichText::new(&cargo.name).color(p.ink)).truncate());
            });
        });
    });
}

#[cfg(test)]
mod tests;
