//! Item image editor, picker and scene snapshot (LLFloaterChangeItemThumbnail).

use super::*;
use crate::ui::widgets::{Floater, flat_button, search_field};
use std::{collections::HashMap, sync::Arc};

pub enum Input {
    File,
    Capture,
    Photo(Arc<Vec<u8>>),
    Texture { asset: Uuid, resize: bool },
    Paste,
}

#[derive(Default)]
pub struct ThumbnailUi {
    pub item: Option<Uuid>,
    pub pending: Option<Uuid>,
    pub error: String,
    pub photo_open: bool,
    pub photo: Option<egui::TextureHandle>,
    pub pixels: Option<Arc<Vec<u8>>>,
    pub picker_open: bool,
    selected: Option<Uuid>,
    filter: String,
    uuid_text: String,
}

impl ThumbnailUi {
    pub fn open(&mut self, item: Uuid) {
        if self.item != Some(item) {
            *self = Self {
                item: Some(item),
                ..Default::default()
            };
        }
    }
    fn request(&mut self, item: Uuid, input: Input, actions: &mut Vec<InvAction>) {
        let request = Uuid::new_v4();
        self.pending = Some(request);
        self.error.clear();
        actions.push(InvAction::Thumbnail { request, item, input });
    }
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        icons: &Icons,
        world: &mut World,
        prefs: &InventoryPreferences,
        ui_id: Uuid,
        images: &HashMap<Uuid, egui::TextureHandle>,
        wanted: &mut HashSet<Uuid>,
        mutation_pending: bool,
        actions: &mut Vec<InvAction>,
    ) {
        let Some(item) = self.item else {
            return;
        };
        if !world.inventory.items.contains_key(&item) && !world.inventory.folders.contains_key(&item) {
            self.item = None;
            return;
        }
        let asset = image(&world.inventory, item);
        let ready = self.pending.is_none() && !mutation_pending && rules::mutable(&world.inventory, item, &prefs.protected);
        let position = ctx.content_rect().center() - egui::vec2(360.0, 200.0);
        let mut open = true;
        let mut caption = String::new();
        Floater::new(
            &format!("inventory_image_{ui_id}"),
            "Modifier l’image de l’objet",
            position,
            Vec2::new(300.0, 398.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.horizontal(|ui| {
                type_name(ui, p, icons, &world.inventory, item);
            });
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                picture(ui, p, asset, 256.0, images, wanted);
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if toolbar_button(
                    ui,
                    p,
                    "arrow-square-out",
                    "Télécharger depuis votre ordinateur",
                    ready,
                    &mut caption,
                ) {
                    self.request(item, Input::File, actions);
                }
                if toolbar_button(ui, p, "camera", "Prendre une photo", ready, &mut caption) {
                    self.photo_open = true;
                    self.request(item, Input::Capture, actions);
                }
                if toolbar_button(ui, p, "image", "Choisir une image", ready, &mut caption) {
                    self.picker_open = true;
                    self.selected = (!asset.is_nil()).then_some(asset);
                    self.uuid_text = asset.to_string();
                    self.filter.clear();
                }
                if toolbar_button(ui, p, "copy", "Copier dans le presse-papiers", !asset.is_nil(), &mut caption) {
                    ctx.copy_text(asset.to_string());
                    actions.push(InvAction::ThumbnailCopied(asset));
                }
                if toolbar_button(ui, p, "clipboard-text", "Coller depuis le presse-papiers", ready, &mut caption) {
                    self.request(item, Input::Paste, actions);
                }
                if toolbar_button(ui, p, "trash", "Supprimer l’image", ready && !asset.is_nil(), &mut caption) {
                    match rules::patch(
                        &world.inventory,
                        item,
                        aurora_net::inventory::operations::metadata_thumbnail(Uuid::nil()),
                    ) {
                        Ok(change) => actions.push(InvAction::Edit(change)),
                        Err(reason) => self.error = reason,
                    }
                }
            });
            ui.label(RichText::new(&caption).size(11.0).color(p.muted));
            if self.pending.is_some() || mutation_pending {
                ui.label(RichText::new("Traitement de l’image…").color(p.muted));
            }
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(p.warn));
            }
        });
        if !open {
            self.item = None;
            self.picker_open = false;
            self.photo_open = false;
            return;
        }
        if self.photo_open {
            let mut photo_open = true;
            let mut cancel = false;
            Floater::new(
                &format!("inventory_photo_{ui_id}"),
                "Photo de l’objet",
                position + egui::vec2(312.0, 0.0),
                Vec2::new(300.0, 326.0),
            )
            .fixed()
            .show(ctx, p, &mut photo_open, |ui| {
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(256.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, p.field);
                    if let Some(texture) = &self.photo {
                        paint_image(ui, rect, texture);
                    } else {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "Prise de photo…",
                            egui::FontId::proportional(13.0),
                            p.muted,
                        );
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready, egui::Button::new("Prendre une photo")).clicked() {
                        self.request(item, Input::Capture, actions);
                    }
                    if ui
                        .add_enabled(ready && self.pixels.is_some(), egui::Button::new("Enregistrer"))
                        .clicked()
                        && let Some(pixels) = &self.pixels
                    {
                        self.request(item, Input::Photo(pixels.clone()), actions);
                    }
                    if flat_button(ui, p, "Annuler").clicked() {
                        cancel = true;
                    }
                });
            });
            self.photo_open = photo_open && !cancel;
        }
        if self.picker_open {
            let mut picker_open = true;
            let mut chosen = None;
            let mut cancel = false;
            Floater::new(
                &format!("inventory_image_picker_{ui_id}"),
                "Choisir : Image pour l’objet",
                position + egui::vec2(312.0, 0.0),
                Vec2::new(450.0, 392.0),
            )
            .fixed()
            .show(ctx, p, &mut picker_open, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        picture(ui, p, self.selected.unwrap_or_default(), 164.0, images, wanted);
                        ui.label("Inventaire");
                        ui.label("Taille : 64 à 256 pixels");
                        for (label, id) in [
                            ("Vierge", aurora_prim::te::BLANK_TEXTURE),
                            ("Transparent", aurora_prim::te::TRANSPARENT_TEXTURE),
                        ] {
                            if flat_button(ui, p, label).clicked() {
                                self.selected = Some(id);
                                self.uuid_text = id.to_string();
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        search_field(ui, &mut self.filter, "Filtrer les textures", 250.0);
                        egui::ScrollArea::both()
                            .id_salt("thumbnail_tree")
                            .max_height(255.0)
                            .min_scrolled_height(255.0)
                            .show(ui, |ui| {
                                let query = self.filter.trim().to_lowercase();
                                if query.is_empty() {
                                    let root = world.inventory.root;
                                    picker_folder(ui, p, icons, &mut world.inventory, root, 0, &mut self.selected);
                                } else {
                                    let mut items: Vec<_> = world
                                        .inventory
                                        .items
                                        .values()
                                        .filter(|it| usable(&world.inventory, it) && it.name.to_lowercase().contains(&query))
                                        .collect();
                                    items.sort_by_key(|it| it.name.to_lowercase());
                                    for it in items.into_iter().take(500) {
                                        ui.horizontal(|ui| {
                                            icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
                                            if ui.selectable_label(self.selected == Some(it.asset_id), &it.name).clicked() {
                                                self.selected = Some(it.asset_id);
                                            }
                                        });
                                    }
                                }
                            });
                    });
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.uuid_text)
                            .hint_text("UUID de l’image")
                            .desired_width(320.0),
                    );
                    if flat_button(ui, p, "Appl. UUID").clicked() {
                        match Uuid::parse_str(self.uuid_text.trim()) {
                            Ok(id) if !id.is_nil() => self.selected = Some(id),
                            _ => self.error = "UUID d’image invalide.".into(),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(ready && self.selected.is_some(), egui::Button::new("OK")).clicked() {
                        chosen = self.selected;
                    }
                    if flat_button(ui, p, "Annuler").clicked() {
                        cancel = true;
                    }
                });
            });
            self.picker_open = picker_open && !cancel && chosen.is_none();
            if let Some(asset) = chosen {
                self.request(item, Input::Texture { asset, resize: true }, actions);
            }
        }
    }
}

pub(super) fn image(inv: &Inventory, item: Uuid) -> Uuid {
    inv.items
        .get(&item)
        .map(|it| it.thumbnail)
        .or_else(|| inv.folders.get(&item).map(|f| f.info.thumbnail))
        .unwrap_or_default()
}
pub(super) fn type_name(ui: &mut egui::Ui, p: &Palette, icons: &Icons, inv: &Inventory, item: Uuid) {
    let name = inv
        .items
        .get(&item)
        .map(|it| item_icon(it.asset_type, it.inv_type))
        .unwrap_or("Inv_FolderClosed");
    icon(ui, icons, name, p);
    ui.label(RichText::new(rules::name(inv, item)).color(p.ink));
}
fn paint_image(ui: &egui::Ui, rect: egui::Rect, texture: &egui::TextureHandle) {
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}
pub(super) fn picture(
    ui: &mut egui::Ui,
    p: &Palette,
    asset: Uuid,
    size: f32,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut HashSet<Uuid>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.field);
    ui.painter()
        .rect_stroke(rect, 0.0, egui::Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
    if !asset.is_nil() {
        wanted.insert(asset);
        if let Some(texture) = images.get(&asset) {
            paint_image(ui, rect, texture);
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Chargement…",
                egui::FontId::proportional(12.0),
                p.muted,
            );
        }
    } else if let Some(camera) = crate::ui::icons::global("camera") {
        let camera_rect = egui::Rect::from_center_size(rect.center(), Vec2::splat(size * 0.28));
        ui.painter().image(
            camera.id(),
            camera_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            p.muted_dim,
        );
        ui.painter().line_segment(
            [camera_rect.left_bottom(), camera_rect.right_top()],
            egui::Stroke::new(2.0, p.muted_dim),
        );
    }
}
pub(crate) fn usable(inv: &Inventory, it: &aurora_net::inventory::InvItem) -> bool {
    it.asset_type == 0
        && !it.asset_id.is_nil()
        && !rules::in_type(inv, it.id, 14)
        && (rules::library(inv, it.id) || it.owner_mask & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER)
}
fn picker_folder(ui: &mut egui::Ui, p: &Palette, icons: &Icons, inv: &mut Inventory, id: Uuid, depth: usize, selected: &mut Option<Uuid>) {
    if depth > 24 {
        return;
    }
    let Some(folder) = inv.folders.get(&id) else {
        return;
    };
    if folder.info.type_default == 14 {
        return;
    }
    super::tree_spacing(ui);
    let name = if id == inv.root {
        "Mon inventaire".to_owned()
    } else if id == inv.lib_root {
        "Bibliothèque".to_owned()
    } else {
        folder.info.name.clone()
    };
    let mut children = folder.children.clone();
    if id == inv.root
        && !inv.lib_root.is_nil()
        && inv.lib_root != id
        && inv.folders.contains_key(&inv.lib_root)
        && !children.contains(&inv.lib_root)
    {
        children.push(inv.lib_root);
        children.sort_by_cached_key(|id| {
            if *id == inv.lib_root {
                "bibliothèque".to_owned()
            } else {
                rules::name(inv, *id).to_lowercase()
            }
        });
    }
    let items = folder.items.clone();
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("thumbnail_folder", id)),
        depth == 0,
    );
    let folder_open = state.is_open();
    state
        .show_header(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                icon(ui, icons, folder_icon(folder.info.type_default, folder_open), p);
                ui.label(RichText::new(name).color(p.ink));
            });
        })
        .body(|ui| {
            inv.request(id);
            for child in children {
                picker_folder(ui, p, icons, inv, child, depth + 1, selected);
            }
            for item in items {
                if let Some(it) = inv.items.get(&item)
                    && usable(inv, it)
                {
                    ui.horizontal(|ui| {
                        icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
                        if ui.selectable_label(*selected == Some(it.asset_id), &it.name).clicked() {
                            *selected = Some(it.asset_id);
                        }
                    });
                }
            }
        });
}

fn toolbar_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, enabled: bool, caption: &mut String) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    ui.painter()
        .rect_filled(rect, 2.0, if enabled && response.hovered() { p.raised } else { p.field });
    if let Some(texture) = crate::ui::icons::global(name) {
        ui.painter().image(
            texture.id(),
            egui::Rect::from_center_size(rect.center(), Vec2::splat(22.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            if enabled { p.ink } else { p.muted_dim },
        );
    }
    if response.hovered() {
        caption.clone_from(&tip.to_owned());
    }
    response.on_hover_text(tip).clicked() && enabled
}
