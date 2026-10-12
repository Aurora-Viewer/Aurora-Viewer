//! Choose a picture among the inventory's textures and snapshots, like
//! LLFloaterTexturePicker (floater_texture_ctrl.xml): opened by the
//! profiles (PICK_TEXTURE, inventory filter IT_TEXTURE | IT_SNAPSHOT) and
//! by the build floater, which adds the « Défaut », « Vierge »,
//! « Transparent » and « Aucune » buttons, the UUID field, and lists
//! materials for a PBR material (PICK_MATERIAL), plus published local previews.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::inventory::{Inventory, actions};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
mod tree;

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

fn inventory_asset(inv: &Inventory, id: Uuid, materials: bool) -> Option<Uuid> {
    // LLTextureCtrl: linked rows use their original asset; a material with
    // a null inventory asset is the blank material, not "no material".
    let original = actions::original(inv, id).unwrap_or(id);
    let item = inv.items.get(&original)?;
    if materials {
        (item.inv_type == INV_MATERIAL).then_some(if item.asset_id.is_nil() { BLANK_MATERIAL } else { item.asset_id })
    } else {
        (matches!(item.inv_type, INV_TEXTURE | INV_SNAPSHOT) && !item.asset_id.is_nil()).then_some(item.asset_id)
    }
}

fn can_preview(inv: &Inventory, id: Uuid) -> bool {
    let Some(original) = actions::original(inv, id) else { return false };
    inv.items
        .get(&original)
        .is_some_and(|item| item.owner_mask & actions::COPY != 0 || actions::library(inv, original))
}

#[derive(Default)]
pub struct TexturePicker {
    pub open: bool,
    title: String,
    filter: String,
    /// Asset of the selected item.
    selected: Option<Uuid>,
    selected_item: Option<Uuid>,
    selected_lookup: Option<(u64, Option<Uuid>)>,
    tree: tree::Tree,
    list_fraction: f32,
    footer_height: f32,
    /// Images to stream (selected texture preview).
    pub wanted_images: HashSet<Uuid>,
    /// Build floater options: list materials, the default asset, « Aucune ».
    materials: bool,
    default: Option<Uuid>,
    allow_none: bool,
    extra_buttons: bool,
    /// Only callers with a client-only application path expose local assets.
    pub(crate) allow_local: bool,
    uuid_text: String,
    /// Intentional choices, separate from OK so callers can opt into preview.
    preview: Option<Uuid>,
    pub(crate) generation: u64,
}

impl TexturePicker {
    pub fn open(&mut self, title: &str, current: Uuid) {
        self.open = true;
        self.title = title.to_owned();
        self.filter.clear();
        self.selected = (!current.is_nil()).then_some(current);
        self.selected_item = None;
        self.selected_lookup = None;
        self.tree.opened(current);
        self.materials = false;
        self.default = None;
        self.allow_none = false;
        self.extra_buttons = false;
        self.allow_local = false;
        self.uuid_text.clear();
        self.footer_height = 0.0;
        self.preview = None;
        self.generation = self.generation.wrapping_add(1);
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

    pub(crate) fn take_preview(&mut self) -> Option<Uuid> {
        self.preview.take()
    }

    fn select(&mut self, asset: Uuid, item: Option<Uuid>, preview: bool) {
        self.selected = Some(asset);
        self.selected_item = item;
        self.preview = preview.then_some(asset);
    }

    /// The asset chosen with "OK" (the picker closes). Nil: « Aucune ».
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<Uuid> {
        self.preview = None;
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
        self.tree
            .prepare(&mut world.inventory, world.agent_id, self.materials, &self.filter);
        let lookup = (world.inventory.generation, self.selected);
        if self.selected_lookup != Some(lookup) {
            self.selected_lookup = Some(lookup);
            if !self
                .selected_item
                .is_some_and(|id| inventory_asset(&world.inventory, id, self.materials) == self.selected)
            {
                self.selected_item = self.selected.and_then(|asset| {
                    world
                        .inventory
                        .items
                        .values()
                        .filter(|it| inventory_asset(&world.inventory, it.id, self.materials) == Some(asset))
                        .map(|it| it.id)
                        .min()
                });
            }
        }
        let size = Vec2::new(680.0, 480.0);
        let mut floater = Floater::new("texture_picker", self.title.clone(), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(560.0, 400.0);
        floater.show(ctx, p, &mut open, |ui| {
            let hint = if self.materials {
                aurora_i18n::tr!("ui-texture-picker-filter-materials")
            } else {
                aurora_i18n::tr!("common-filter-textures")
            };
            widgets::search_field(ui, &mut self.filter, hint, ui.available_width());
            ui.add_space(8.0);
            // Reserve the footer's measured natural height. Theme spacing or
            // larger input fonts must never feed extra height back into the
            // next frame's filling list and make the window grow endlessly.
            let footer = if self.footer_height > 0.0 {
                self.footer_height
            } else {
                (if self.extra_buttons { 112.0 } else { 64.0 }) + 2.0 * ui.spacing().item_spacing.y
            };
            let (body, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), (ui.available_height() - footer).max(280.0)),
                egui::Sense::hover(),
            );
            let fraction = if self.list_fraction > 0.0 { self.list_fraction } else { 0.68 };
            let left_width = (body.width() * fraction).clamp(240.0, body.width() - 176.0);
            let divider = egui::Rect::from_min_max(
                egui::pos2(body.left() + left_width, body.top()),
                egui::pos2(body.left() + left_width + 8.0, body.bottom()),
            );
            let drag = ui
                .interact(divider, ui.id().with("split"), egui::Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text(aurora_i18n::tr!("ui-texture-picker-adjust-list-width"));
            if drag.dragged()
                && let Some(pointer) = drag.interact_pointer_pos()
            {
                self.list_fraction = ((pointer.x - body.left()).clamp(240.0, body.width() - 176.0)) / body.width();
            }
            ui.painter()
                .vline(divider.center().x, divider.y_range(), egui::Stroke::new(1.0, p.muted_dim));
            let mut list = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("texture_list")
                    .max_rect(egui::Rect::from_min_max(body.min, egui::pos2(divider.left(), body.bottom())))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            list.spacing_mut().item_spacing.y = 0.0;
            let rows = &self.tree.state.rows;
            let filter = self.filter.trim().to_lowercase();
            let mut locals: Vec<_> = world
                .local_assets
                .iter()
                .filter(|(_, (name, material))| {
                    self.allow_local && *material == self.materials && (filter.is_empty() || name.to_lowercase().contains(&filter))
                })
                .collect();
            locals.sort_by(|a, b| a.1.0.cmp(&b.1.0));
            let row_count = rows.len() + if locals.is_empty() { 0 } else { locals.len() + 1 };
            let tab = usize::from(!self.filter.trim().is_empty());
            let mut toggle = None;
            let radius = ui.visuals().window_corner_radius;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(body.height())
                .show_rows(&mut list, 22.0, row_count, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for index in range {
                        if index >= rows.len() {
                            if index == rows.len() {
                                ui.add_sized(
                                    Vec2::new(ui.available_width(), 22.0),
                                    egui::Label::new(RichText::new(aurora_i18n::tr!("import-local")).color(p.muted)),
                                );
                            } else {
                                let (id, (name, _)) = locals[index - rows.len() - 1];
                                let selected = self.selected == Some(*id);
                                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
                                if selected || response.hovered() {
                                    ui.painter().rect_filled(rect, radius, if selected { p.violet } else { p.raised });
                                }
                                widgets::paint_elided(
                                    ui,
                                    name,
                                    rect.left_center() + egui::vec2(8.0, 0.0),
                                    rect.width() - 16.0,
                                    egui::FontId::proportional(12.0),
                                    p.ink,
                                );
                                if response.clicked() {
                                    self.selected = Some(*id);
                                    self.selected_item = None;
                                    self.preview = Some(*id);
                                }
                                if response.double_clicked() {
                                    chosen = Some(*id);
                                }
                            }
                            continue;
                        }
                        let row = &rows[index];
                        let inv = &world.inventory;
                        let name = if row.id == inv.root {
                            aurora_i18n::tr!("common-my-inventory").to_owned()
                        } else if row.id == inv.lib_root {
                            aurora_i18n::tr!("common-library").to_owned()
                        } else {
                            crate::world::inventory::actions::name(inv, row.id)
                        };
                        let selected = !row.folder && self.selected_item == Some(row.id);
                        let asset = (!row.folder).then(|| inventory_asset(inv, row.id, self.materials)).flatten();
                        let enabled = row.folder || asset.is_some();
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 22.0),
                            if enabled { egui::Sense::click() } else { egui::Sense::hover() },
                        );
                        if selected || response.hovered() {
                            ui.painter().rect_filled(rect, radius, if selected { p.violet } else { p.raised });
                        }
                        let mut x = rect.left() + 4.0 + row.depth.min(16) as f32 * 12.0;
                        if row.folder {
                            let c = egui::pos2(x + 5.0, rect.center().y);
                            let points = if row.open {
                                vec![c + egui::vec2(-3.5, -2.0), c + egui::vec2(3.5, -2.0), c + egui::vec2(0.0, 2.5)]
                            } else {
                                vec![c + egui::vec2(-2.0, -3.5), c + egui::vec2(2.5, 0.0), c + egui::vec2(-2.0, 3.5)]
                            };
                            ui.painter().add(egui::Shape::convex_polygon(points, p.muted, egui::Stroke::NONE));
                        }
                        x += 12.0;
                        let icon = if row.folder {
                            if row.open { "folder-open-filled" } else { "folder-filled" }
                        } else if self.materials {
                            "droplet-filled"
                        } else {
                            "photo-filled"
                        };
                        if let Some(icon) = super::icons::global(icon) {
                            ui.painter().image(
                                icon.id(),
                                egui::Rect::from_center_size(egui::pos2(x + 7.0, rect.center().y), Vec2::splat(14.0)),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                if selected { p.ink } else { p.violet_light },
                            );
                        }
                        x += 20.0;
                        let cut = widgets::paint_elided(
                            ui,
                            &name,
                            egui::pos2(x, rect.center().y),
                            rect.right() - x - 4.0,
                            egui::FontId::proportional(12.0),
                            if enabled { p.ink } else { p.muted_dim },
                        );
                        let response = if cut { response.on_hover_text(&name) } else { response };
                        if response.clicked() {
                            if row.folder {
                                toggle = Some(row.id);
                            } else {
                                if let Some(asset) = asset {
                                    self.selected = Some(asset);
                                    self.selected_item = Some(row.id);
                                    self.preview = can_preview(&world.inventory, row.id).then_some(asset);
                                }
                            }
                        }
                        if !row.folder && response.double_clicked() {
                            chosen = self.selected;
                        }
                    }
                });
            if let Some(id) = toggle {
                self.tree.state.toggle(id, tab);
            }
            let mut preview = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("texture_preview")
                    .max_rect(egui::Rect::from_min_max(egui::pos2(divider.right() + 4.0, body.top()), body.max))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            {
                let ui = &mut preview;
                ui.label(RichText::new(aurora_i18n::tr!("ui-texture-picker-preview")).size(12.0).color(p.ink));
                let reserve = if self.extra_buttons {
                    if !self.materials && self.allow_none { 154.0 } else { 126.0 }
                } else {
                    60.0
                };
                let side = ui.available_width().min(188.0).min((body.height() - reserve).max(96.0));
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                ui.painter().rect_filled(rect, radius, p.field);
                if let Some(a) = self.selected.filter(|_| !self.materials) {
                    self.wanted_images.insert(a);
                    if let Some(t) = images.get(&a) {
                        ui.painter().image(
                            t.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        let size = t.size();
                        ui.label(RichText::new(format!("{} × {}", size[0], size[1])).size(11.0).color(p.muted));
                    } else if a == BLANK_TEXTURE {
                        // IMG_WHITE has known content while its metadata streams.
                        ui.painter().rect_filled(rect, radius, egui::Color32::WHITE);
                    } else if a == TRANSPARENT_TEXTURE {
                        let cell = 12.0;
                        for y in 0..(side / cell).ceil() as usize {
                            for x in 0..(side / cell).ceil() as usize {
                                let tile =
                                    egui::Rect::from_min_size(rect.min + egui::vec2(x as f32 * cell, y as f32 * cell), Vec2::splat(cell))
                                        .intersect(rect);
                                ui.painter()
                                    .rect_filled(tile, 0.0, if (x + y) % 2 == 0 { p.field } else { p.raised });
                            }
                        }
                    } else {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            aurora_i18n::tr!("common-loading-2"),
                            egui::FontId::proportional(11.5),
                            p.muted,
                        );
                    }
                }
                if let Some(item) = self.selected_item.and_then(|id| world.inventory.items.get(&id)) {
                    ui.add(widgets::elided_label(
                        ui,
                        RichText::new(&item.name).size(12.0).color(p.ink),
                        ui.available_width(),
                    ));
                }
                if self.extra_buttons {
                    ui.add_space(6.0);
                    let half = (ui.available_width() - ui.spacing().item_spacing.x) * 0.5;
                    ui.horizontal(|ui| {
                        if let Some(default) = self.default
                            && widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-default"), Vec2::new(half, 24.0)).clicked()
                        {
                            self.select(default, None, true);
                        }
                        let width = if self.default.is_some() { half } else { ui.available_width() };
                        if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-blank"), Vec2::new(width, 24.0)).clicked() {
                            self.select(if self.materials { BLANK_MATERIAL } else { BLANK_TEXTURE }, None, true);
                        }
                    });
                    if !self.materials
                        && widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-transparent"), Vec2::new(ui.available_width(), 24.0))
                            .clicked()
                    {
                        self.select(TRANSPARENT_TEXTURE, None, true);
                    }
                    if self.allow_none
                        && widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-none-2"), Vec2::new(ui.available_width(), 24.0))
                            .clicked()
                    {
                        chosen = Some(Uuid::nil());
                    }
                }
            }
            // All footer rows live outside the columns. Their widths depend
            // on the window only, never on texture names or preview content.
            ui.add_space(6.0);
            let message = if self.selected_item.is_some_and(|id| !can_preview(&world.inventory, id)) {
                aurora_i18n::tr!("ui-texture-picker-confirm-no-copy")
            } else if self.tree.loading {
                aurora_i18n::tr!("common-loading-2")
            } else if self.tree.view.results.is_empty() {
                if self.materials {
                    aurora_i18n::tr!("ui-texture-picker-no-materials-in-the-loaded-inventory")
                } else {
                    aurora_i18n::tr!("ui-texture-picker-no-textures-in-the-loaded-inventory")
                }
            } else {
                ""
            };
            let (status, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), egui::Sense::hover());
            widgets::paint_elided(
                ui,
                message,
                status.left_center(),
                status.width(),
                egui::FontId::proportional(11.0),
                p.muted,
            );
            if self.extra_buttons {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-texture-picker-uuid")).size(12.0).color(p.muted));
                    let width = (ui.available_width() - 104.0).max(120.0);
                    ui.add_sized([width, 24.0], egui::TextEdit::singleline(&mut self.uuid_text));
                    if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-apply"), Vec2::new(96.0, 24.0)).clicked() {
                        match Uuid::parse_str(self.uuid_text.trim()) {
                            Ok(id) => {
                                self.select(id, None, true);
                            }
                            Err(_) => self.uuid_text = aurora_i18n::tr!("ui-texture-picker-invalid-uuid").into(),
                        }
                    }
                });
            }
            ui.separator();
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 26.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.add_enabled_ui(self.selected.is_some(), |ui| {
                        if widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(72.0, 26.0)).clicked() {
                            chosen = self.selected;
                        }
                    });
                    if widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(84.0, 26.0)).clicked() {
                        self.open = false;
                    }
                },
            );
            self.footer_height = (ui.cursor().top() - body.bottom()).max(1.0);
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
    fn inventory_links_resolve_assets_and_a_null_material_is_blank() {
        let mut inv = Inventory::default();
        let texture = Uuid::from_u128(1);
        let asset = Uuid::from_u128(2);
        let link = Uuid::from_u128(3);
        let material = Uuid::from_u128(4);
        for item in [
            aurora_net::inventory::InvItem {
                id: texture,
                asset_id: asset,
                inv_type: INV_TEXTURE,
                ..Default::default()
            },
            aurora_net::inventory::InvItem {
                id: link,
                asset_id: texture,
                asset_type: 24,
                inv_type: INV_TEXTURE,
                ..Default::default()
            },
            aurora_net::inventory::InvItem {
                id: material,
                inv_type: INV_MATERIAL,
                ..Default::default()
            },
        ] {
            inv.items.insert(item.id, item);
        }
        assert_eq!(inventory_asset(&inv, link, false), Some(asset));
        assert_eq!(inventory_asset(&inv, material, true), Some(BLANK_MATERIAL));
        assert_eq!(inventory_asset(&inv, material, false), None);
        inv.items.get_mut(&texture).unwrap().asset_id = Uuid::nil();
        assert_eq!(inventory_asset(&inv, link, false), None);
        // Firestorm defers no-copy inventory textures until OK; links must
        // check the original item's rights, and library textures can preview.
        assert!(!can_preview(&inv, link));
        inv.items.get_mut(&texture).unwrap().owner_mask = actions::COPY;
        assert!(can_preview(&inv, link));
        inv.items.get_mut(&texture).unwrap().owner_mask = 0;
        let library = Uuid::from_u128(5);
        inv.items.get_mut(&texture).unwrap().parent = library;
        inv.folders.insert(
            library,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: library,
                    ..Default::default()
                },
                children: vec![],
                items: vec![texture],
                state: crate::world::inventory::FetchState::Fetched,
                library: true,
            },
        );
        assert!(can_preview(&inv, link));
    }

    #[test]
    fn picker_width_stays_bounded_with_long_names_empty_inventory_and_material_controls() {
        let p = crate::theme::Theme::default().palette();
        for (mode, font_scale) in (0..3).flat_map(|mode| [1.0, 2.0].map(move |scale| (mode, scale))) {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, font_scale);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let asset = Uuid::from_u128(1);
            let root = Uuid::from_u128(2);
            world.inventory.root = root;
            world.inventory.folders.insert(
                root,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: root,
                        ..Default::default()
                    },
                    children: Vec::new(),
                    items: vec![asset],
                    state: crate::world::inventory::FetchState::Fetched,
                    library: false,
                },
            );
            if mode != 0 {
                world.inventory.items.insert(
                    asset,
                    aurora_net::inventory::InvItem {
                        id: asset,
                        parent: root,
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
                if frame == 60 || frame == 120 {
                    world.inventory.folders.get_mut(&root).unwrap().state = if frame == 60 {
                        crate::world::inventory::FetchState::Fetching
                    } else {
                        crate::world::inventory::FetchState::Fetched
                    };
                    world.inventory.generation += 1;
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                        time: Some(frame as f64 / 60.0),
                        ..Default::default()
                    },
                    |_| {
                        picker.show(&ctx, &p, &mut world, &HashMap::new());
                    },
                );
                output.textures_delta.clear();
                let rect = ctx.memory(|m| m.area_rect(egui::Id::new("texture_picker"))).unwrap();
                assert!(rect.width() < 730.0, "mode {mode}, frame {frame}: {rect:?}");
                assert!(rect.height() < 600.0, "mode {mode}, frame {frame}: {rect:?}");
                if frame == 20 {
                    initial = Some(rect);
                }
                if frame > 20 {
                    assert!(
                        (rect.height() - initial.unwrap().height()).abs() < 0.5,
                        "unstable height: mode {mode}, frame {frame}: {rect:?}"
                    );
                    assert!(
                        (rect.width() - initial.unwrap().width()).abs() < 0.5,
                        "mode {mode}, frame {frame}: {rect:?}"
                    );
                }
            }
        }
    }
}
