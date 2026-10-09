//! Choose a picture among the inventory's textures and snapshots, like
//! LLFloaterTexturePicker (floater_texture_ctrl.xml): opened by the
//! profiles (PICK_TEXTURE, inventory filter IT_TEXTURE | IT_SNAPSHOT) and
//! by the build floater, which adds the « Défaut », « Vierge »,
//! « Transparent » and « Aucune » buttons, the UUID field, and lists
//! materials for a PBR material (PICK_MATERIAL). No local textures nor bakes.

use super::widgets::{self, Floater, flat_button};
use crate::theme::Palette;
use crate::world::World;
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// IT_TEXTURE, IT_SNAPSHOT, IT_MATERIAL (llinventorytype.h).
const INV_TEXTURE: i32 = 0;
const INV_SNAPSHOT: i32 = 15;
const INV_MATERIAL: i32 = 26;
/// IMG_WHITE: « Vierge » of a texture.
const BLANK_TEXTURE: Uuid = aurora_prim::te::BLANK_TEXTURE;
/// UIImgTransparentUUID.
const TRANSPARENT_TEXTURE: Uuid = aurora_prim::te::TRANSPARENT_TEXTURE;
/// BLANK_MATERIAL_ASSET_ID.
const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

#[derive(Default)]
pub struct TexturePicker {
    pub open: bool,
    title: String,
    filter: String,
    /// Asset of the selected item.
    selected: Option<Uuid>,
    /// Images to stream (selected texture preview).
    pub wanted_images: HashSet<Uuid>,
    /// Build floater options: list materials, the default asset, « Aucune ».
    materials: bool,
    default: Option<Uuid>,
    allow_none: bool,
    extra_buttons: bool,
    uuid_text: String,
}

impl TexturePicker {
    pub fn open(&mut self, title: &str, current: Uuid) {
        self.open = true;
        self.title = title.to_owned();
        self.filter.clear();
        self.selected = (!current.is_nil()).then_some(current);
        self.materials = false;
        self.default = None;
        self.allow_none = false;
        self.extra_buttons = false;
        self.uuid_text.clear();
    }

    /// The build floater's picker: « Défaut » (`default`), « Vierge »,
    /// « Transparent », « Aucune » when `allow_none`.
    pub fn open_with_defaults(&mut self, title: &str, current: Uuid, default: Option<Uuid>, allow_none: bool) {
        self.open(title, current);
        self.default = default;
        self.allow_none = allow_none;
        self.extra_buttons = true;
    }

    /// PICK_MATERIAL: the inventory's materials, « Vierge » = blank material.
    pub fn open_materials(&mut self, title: &str, current: Uuid) {
        self.open(title, current);
        self.materials = true;
        self.allow_none = true;
        self.extra_buttons = true;
    }

    /// The asset chosen with "OK" (the picker closes). Nil: « Aucune ».
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, world: &World, images: &HashMap<Uuid, egui::TextureHandle>) -> Option<Uuid> {
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
        let size = Vec2::new(440.0, 420.0);
        Floater::new("texture_picker", self.title.clone(), screen.center() - size * 0.5, size).show(ctx, p, &mut open, |ui| {
            let filter = self.filter.trim().to_lowercase();
            let materials = self.materials;
            let mut items: Vec<_> = world
                .inventory
                .items
                .values()
                .filter(|i| {
                    if materials {
                        i.inv_type == INV_MATERIAL
                    } else {
                        i.inv_type == INV_TEXTURE || i.inv_type == INV_SNAPSHOT
                    }
                })
                .filter(|i| !i.asset_id.is_nil())
                .filter(|i| filter.is_empty() || i.name.to_lowercase().contains(&filter))
                .collect();
            items.sort_by_key(|i| i.name.to_lowercase());
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    let hint = if materials {
                        "Filtrer les matériaux"
                    } else {
                        "Filtrer les textures"
                    };
                    widgets::search_field(ui, &mut self.filter, hint, 200.0);
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(290.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if items.is_empty() {
                                let t = if materials {
                                    "Aucun matériau dans l'inventaire chargé."
                                } else {
                                    "Aucune texture dans l'inventaire chargé."
                                };
                                ui.label(RichText::new(t).size(11.5).color(p.muted));
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
                    if let Some(a) = self.selected.filter(|_| !materials) {
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
                    if self.extra_buttons {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.set_max_width(180.0);
                            if let Some(d) = self.default
                                && flat_button(ui, p, "Défaut").clicked()
                            {
                                self.selected = Some(d);
                            }
                            if flat_button(ui, p, "Vierge").clicked() {
                                self.selected = Some(if materials { BLANK_MATERIAL } else { BLANK_TEXTURE });
                            }
                            if !materials && flat_button(ui, p, "Transparent").clicked() {
                                self.selected = Some(TRANSPARENT_TEXTURE);
                            }
                            if self.allow_none && flat_button(ui, p, "Aucune").clicked() {
                                chosen = Some(Uuid::nil());
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.uuid_text)
                                    .hint_text("UUID")
                                    .desired_width(110.0),
                            );
                            if flat_button(ui, p, "Appl. UUID").clicked() {
                                match Uuid::parse_str(self.uuid_text.trim()) {
                                    Ok(id) => self.selected = Some(id),
                                    Err(_) => self.uuid_text = "UUID invalide".into(),
                                }
                            }
                        });
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
