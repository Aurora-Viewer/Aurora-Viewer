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
        let size = Vec2::new(500.0, 460.0);
        let mut floater = Floater::new("texture_picker", self.title.clone(), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(440.0, 330.0);
        floater.show(ctx, p, &mut open, |ui| {
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
            // Both columns receive a bounded share of the existing width.
            // Feeding a child's required width back into the next frame's
            // available width made this floater grow continuously.
            ui.columns(2, |columns| {
                {
                    let ui = &mut columns[0];
                    let hint = if materials {
                        aurora_i18n::tr!("ui-texture-picker-filter-materials")
                    } else {
                        aurora_i18n::tr!("common-filter-textures")
                    };
                    widgets::search_field(ui, &mut self.filter, hint, ui.available_width());
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height((ui.available_height() - 8.0).max(120.0))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if items.is_empty() {
                                let t = if materials {
                                    aurora_i18n::tr!("ui-texture-picker-no-materials-in-the-loaded-inventory")
                                } else {
                                    aurora_i18n::tr!("ui-texture-picker-no-textures-in-the-loaded-inventory")
                                };
                                ui.add(egui::Label::new(RichText::new(t).size(11.5).color(p.muted)).wrap());
                            }
                            for i in items.iter().take(2000) {
                                let sel = self.selected == Some(i.asset_id);
                                let r = ui
                                    .add_sized(
                                        [ui.available_width(), 22.0],
                                        egui::Button::selectable(sel, RichText::new(&i.name).size(12.0))
                                            .wrap_mode(egui::TextWrapMode::Truncate),
                                    )
                                    .on_hover_text(&i.name);
                                if r.clicked() {
                                    self.selected = Some(i.asset_id);
                                }
                                if r.double_clicked() {
                                    chosen = Some(i.asset_id);
                                }
                            }
                        });
                }
                {
                    let ui = &mut columns[1];
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(188.0), egui::Sense::hover());
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
                                    aurora_i18n::tr!("common-loading-2"),
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
                                && flat_button(ui, p, aurora_i18n::tr!("common-default")).clicked()
                            {
                                self.selected = Some(d);
                            }
                            if flat_button(ui, p, aurora_i18n::tr!("common-blank")).clicked() {
                                self.selected = Some(if materials { BLANK_MATERIAL } else { BLANK_TEXTURE });
                            }
                            if !materials && flat_button(ui, p, aurora_i18n::tr!("common-transparent")).clicked() {
                                self.selected = Some(TRANSPARENT_TEXTURE);
                            }
                            if self.allow_none && flat_button(ui, p, aurora_i18n::tr!("common-none-2")).clicked() {
                                chosen = Some(Uuid::nil());
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.uuid_text)
                                    .hint_text("UUID")
                                    .desired_width(110.0),
                            );
                            if flat_button(ui, p, aurora_i18n::tr!("common-apply-uuid")).clicked() {
                                match Uuid::parse_str(self.uuid_text.trim()) {
                                    Ok(id) => self.selected = Some(id),
                                    Err(_) => self.uuid_text = aurora_i18n::tr!("ui-texture-picker-invalid-uuid").into(),
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
                        if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                            self.open = false;
                        }
                    });
                }
            });
        });
        if !open || chosen.is_some() {
            self.open = false;
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_width_stays_bounded_with_long_names_empty_inventory_and_material_controls() {
        let p = crate::theme::Theme::default().palette();
        for mode in 0..3 {
            let ctx = egui::Context::default();
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let asset = Uuid::from_u128(1);
            if mode != 0 {
                world.inventory.items.insert(
                    asset,
                    aurora_net::inventory::InvItem {
                        id: asset,
                        asset_id: asset,
                        inv_type: if mode == 2 { INV_MATERIAL } else { INV_TEXTURE },
                        name: "A very long texture name with many details ".repeat(30),
                        ..Default::default()
                    },
                );
            }
            let mut picker = TexturePicker::default();
            if mode == 2 {
                picker.open_materials("Material", asset);
            } else {
                picker.open_with_defaults("Texture", asset, Some(asset), true);
            }
            let mut initial = None;
            for frame in 0..180 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |_| {
                        picker.show(&ctx, &p, &world, &HashMap::new());
                    },
                );
                output.textures_delta.clear();
                let rect = ctx.memory(|m| m.area_rect(egui::Id::new("texture_picker"))).unwrap();
                assert!(rect.width() < 550.0, "mode {mode}, frame {frame}: {rect:?}");
                if frame == 20 {
                    initial = Some(rect);
                }
                if frame > 20 {
                    assert!(
                        (rect.width() - initial.unwrap().width()).abs() < 0.5,
                        "mode {mode}, frame {frame}: {rect:?}"
                    );
                }
            }
        }
    }
}
