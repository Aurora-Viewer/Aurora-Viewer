//! Choose a picture among the inventory's textures and snapshots, like
//! LLFloaterTexturePicker opened by the profiles (PICK_TEXTURE, inventory
//! filter IT_TEXTURE | IT_SNAPSHOT, no local textures nor bakes).

use super::widgets::{self, Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// IT_TEXTURE, IT_SNAPSHOT (llinventorytype.h).
const INV_TEXTURE: i32 = 0;
const INV_SNAPSHOT: i32 = 15;

#[derive(Default)]
pub struct TexturePicker {
    pub open: bool,
    title: String,
    filter: String,
    /// Asset of the selected item.
    selected: Option<Uuid>,
    /// Images to stream (selected texture preview).
    pub wanted_images: HashSet<Uuid>,
}

impl TexturePicker {
    pub fn open(&mut self, title: &str, current: Uuid) {
        self.open = true;
        self.title = title.to_owned();
        self.filter.clear();
        self.selected = (!current.is_nil()).then_some(current);
    }

    /// The asset chosen with "OK" (the picker closes).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, images: &HashMap<Uuid, egui::TextureHandle>) -> Option<Uuid> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new(420.0, 380.0);
        Floater::new("texture_picker", self.title.clone(), screen.center() - size * 0.5, size).show(ctx, p, &mut open, |ui| {
            let filter = self.filter.trim().to_lowercase();
            let mut items: Vec<_> = world
                .inventory
                .items
                .values()
                .filter(|i| (i.inv_type == INV_TEXTURE || i.inv_type == INV_SNAPSHOT) && !i.asset_id.is_nil())
                .filter(|i| filter.is_empty() || i.name.to_lowercase().contains(&filter))
                .collect();
            items.sort_by_key(|i| i.name.to_lowercase());
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    widgets::search_field(ui, &mut self.filter, "Filtrer les textures", 200.0);
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(290.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if items.is_empty() {
                                ui.label(RichText::new("Aucune texture dans l'inventaire chargé.").size(11.5).color(p.muted));
                            }
                            for i in items.iter().take(2000) {
                                let sel = self.selected == Some(i.asset_id);
                                let r = ui.selectable_label(sel, RichText::new(&i.name).size(12.0));
                                if r.clicked() {
                                    self.selected = Some(i.asset_id);
                                }
                                if r.double_clicked() {
                                    chosen = Some(i.asset_id);
                                }
                            }
                        });
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(180.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 2.0, p.field);
                    if let Some(a) = self.selected {
                        self.wanted_images.insert(a);
                        match images.get(&a) {
                            Some(t) => {
                                ui.painter().image(
                                    t.id(),
                                    rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE,
                                );
                            }
                            None => {
                                ui.painter().text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "Chargement…",
                                    egui::FontId::proportional(11.5),
                                    p.muted,
                                );
                            }
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(self.selected.is_some(), |ui| {
                            if flat_button(ui, p, "OK").clicked() {
                                chosen = self.selected;
                            }
                        });
                        if flat_button(ui, p, "Annuler").clicked() {
                            self.open = false;
                        }
                    });
                });
            });
        });
        if !open || chosen.is_some() {
            self.open = false;
        }
        chosen
    }
}
