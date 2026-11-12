//! Choose a picture among the inventory's textures and snapshots, like
//! LLFloaterTexturePicker (floater_texture_ctrl.xml): opened by the
//! profiles (PICK_TEXTURE, inventory filter IT_TEXTURE | IT_SNAPSHOT) and
//! by the build floater, which adds the "Default", "Blank",
//! "Transparent" and "None" buttons, the UUID field, and lists
//! materials for a PBR material (PICK_MATERIAL), plus published local previews.

use super::widgets::{self, Floater};
use crate::theme::Palette;
use crate::world::inventory::{Inventory, actions};
use crate::world::{LocalAssetRequest, World};
use aurora_net::inventory::InvItem;
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
pub(crate) mod pipette;
mod tree;

/// IT_TEXTURE, IT_SNAPSHOT, IT_MATERIAL (llinventorytype.h).
const INV_TEXTURE: i32 = 0;
const INV_SNAPSHOT: i32 = 15;
const INV_MATERIAL: i32 = 26;
/// IMG_WHITE: "Blank" of a texture.
const BLANK_TEXTURE: Uuid = aurora_prim::te::BLANK_TEXTURE;
/// UIImgTransparentUUID.
const TRANSPARENT_TEXTURE: Uuid = aurora_prim::te::TRANSPARENT_TEXTURE;
/// BLANK_MATERIAL_ASSET_ID.
const BLANK_MATERIAL: Uuid = Uuid::from_u128(0x968cbad0_4dad_d64e_71b5_72bf13ad051a);

/// LLFloaterTexturePicker::onBakeTextureSelect: keep the magic bake UUID
/// in the texture entry and show its coloured reference image while editing.
fn bom_choices() -> [(&'static str, Uuid); 11] {
    let names = [
        aurora_i18n::tr!("picker-bom-head"),
        aurora_i18n::tr!("picker-bom-upper"),
        aurora_i18n::tr!("picker-bom-lower"),
        aurora_i18n::tr!("picker-bom-eyes"),
        aurora_i18n::tr!("picker-bom-skirt"),
        aurora_i18n::tr!("picker-bom-hair"),
        aurora_i18n::tr!("picker-bom-leftarm"),
        aurora_i18n::tr!("picker-bom-leftleg"),
        aurora_i18n::tr!("picker-bom-aux1"),
        aurora_i18n::tr!("picker-bom-aux2"),
        aurora_i18n::tr!("picker-bom-aux3"),
    ];
    std::array::from_fn(|index| (names[index], crate::scene::avatar::BAKES[index].2))
}

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

/// Whether the item misses one of the rights `required` (agent_mask: the
/// same reading as the Texture tab's slots).
fn lacks_rights(item: &InvItem, agent: Uuid, required: u32) -> bool {
    crate::world::inventory::actions::agent_mask(item, agent) & required != required
}

/// A drop that needs copy and transfer (`drop_target_copy_transfer`): a
/// library texture, or one whose owner may copy and transfer it.
fn copy_and_transfer(inv: &Inventory, item: &aurora_net::inventory::InvItem) -> bool {
    let both = actions::COPY | actions::TRANSFER;
    actions::library(inv, item.id) || actions::held(inv, item) & both == both
}

fn can_preview(inv: &Inventory, id: Uuid) -> bool {
    let Some(original) = actions::original(inv, id) else { return false };
    inv.items
        .get(&original)
        .is_some_and(|item| actions::held(inv, item) & actions::COPY != 0 || actions::library(inv, original))
}

#[derive(Default)]
pub struct TexturePicker {
    drop_target: Option<super::resource_drop::Target>,
    drop_slot: usize,
    /// The registered drop takes only copy and transfer textures.
    drop_copy_transfer: bool,
    dropped_slot: Option<usize>,
    pub open: bool,
    pipette_id: Option<egui::Id>,
    pipette_error: bool,
    pipette_lookup: Option<(u64, Uuid, Option<Uuid>)>,
    title: String,
    filter: String,
    /// Asset of the selected item.
    selected: Option<Uuid>,
    selected_item: Option<Uuid>,
    selected_lookup: Option<(u64, Option<Uuid>)>,
    reveal_selection: bool,
    tree: tree::Tree,
    local_open: [bool; 2],
    source: usize,
    library_lookup: Option<(u64, Uuid, Option<Uuid>)>,
    uuid_error: bool,
    footer_height: f32,
    local_row_height: f32,
    footer_font_scale: f32,
    material_previews: super::texture_library::previews::Previews,
    material_model: Option<std::sync::Arc<aurora_render::preview::Model>>,
    material_view: aurora_render::preview::View,
    /// Images to stream (selected texture preview).
    pub wanted_images: HashSet<Uuid>,
    /// Build floater options: list materials, the default asset, "None".
    materials: bool,
    default: Option<Uuid>,
    allow_none: bool,
    extra_buttons: bool,
    /// Only callers with a client-only application path expose local assets.
    pub(crate) allow_local: bool,
    /// Rights an inventory item needs here. LLTextureCtrl's immediate filter
    /// hides the others; they stay listed here, greyed out.
    pub(crate) required_permissions: u32,
    /// Rights an item dropped on the open picker needs (LLTextureCtrl's
    /// DnD filter), on top of `required_permissions`.
    pub(crate) drop_permissions: u32,
    /// Rights an item needs for a live preview; 0: copy, as for any picker
    /// (FSPanelFace::onTextureSelectionChanged / onPbrSelectionChanged).
    pub(crate) live_permissions: u32,
    /// The objects the choice goes on are the agent's and not for sale: a
    /// no-transfer item may be confirmed only then (setCanApply).
    pub(crate) owned_target: bool,
    uuid_text: String,
    uuid_from_user: bool,
    /// Selection, inventory permissions and local membership used by the UUID field.
    uuid_lookup: Option<(u64, Option<Uuid>, bool)>,
    /// Intentional choices, separate from OK so callers can opt into preview.
    preview: Option<Uuid>,
    /// Single-value callers share the same reversible live-edit session.
    original: Uuid,
    live_value: Uuid,
    cancelled: bool,
    pub(crate) generation: u64,
}

impl TexturePicker {
    /// Register an editable thumbnail for the same choice path as this picker.
    pub(crate) fn drop_target(&mut self, ui: &egui::Ui, rect: egui::Rect, materials: bool, enabled: bool) {
        self.drop_target_slot(ui, rect, materials, enabled, 0);
    }

    /// Keep a drop's target separate from an open picker's target. A refused
    /// drop must not redirect that picker's next choice or Cancel.
    pub(crate) fn drop_target_slot(&mut self, ui: &egui::Ui, rect: egui::Rect, materials: bool, enabled: bool, slot: usize) {
        let Some(target) = super::resource_drop::Target::hovered(ui, rect, materials, enabled) else {
            return;
        };
        self.drop_target = Some(target);
        self.drop_slot = slot;
        self.drop_copy_transfer = false;
    }

    /// `drop_target` for a texture that leaves this viewer in what it makes
    /// (the particle editor's script): copy and transfer textures only.
    pub(crate) fn drop_target_copy_transfer(&mut self, ui: &egui::Ui, rect: egui::Rect, enabled: bool) {
        self.drop_target_slot(ui, rect, false, enabled, 0);
        if self.drop_target.is_some() {
            self.drop_copy_transfer = true;
        }
    }

    pub(crate) fn dropped_slot(&self) -> Option<usize> {
        self.dropped_slot
    }

    /// The inventory item behind the asset just chosen, when there is one.
    pub(crate) fn chosen_item(&self, asset: Uuid) -> Option<Uuid> {
        self.selected_item.filter(|_| self.selected == Some(asset))
    }

    /// Whether choosing the item `id` previews it live: `live_permissions`
    /// when a caller sets them (a Library item always), else `can_preview`.
    fn previewable(&self, inv: &Inventory, agent: Uuid, id: Uuid) -> bool {
        Self::previewable_with(self.live_permissions, inv, agent, id)
    }

    fn previewable_with(live_permissions: u32, inv: &Inventory, agent: Uuid, id: Uuid) -> bool {
        if live_permissions == 0 {
            return can_preview(inv, id);
        }
        let Some(original) = actions::original(inv, id) else { return false };
        actions::library(inv, original)
            || inv
                .items
                .get(&original)
                .is_some_and(|item| !lacks_rights(item, agent, live_permissions))
    }

    /// OK is refused for a no-transfer item unless `owned_target`
    /// (LLTextureCtrl::setCanApply(false, is_object_owner && not_for_sale)).
    fn confirm_blocked(&self, inv: &Inventory, agent: Uuid) -> bool {
        Self::confirm_blocked_with(self.owned_target, self.selected_item, inv, agent)
    }

    fn confirm_blocked_with(owned_target: bool, item: Option<Uuid>, inv: &Inventory, agent: Uuid) -> bool {
        !owned_target
            && item
                .and_then(|id| actions::original(inv, id))
                .filter(|id| !actions::library(inv, *id))
                .and_then(|id| inv.items.get(&id))
                .is_some_and(|item| lacks_rights(item, agent, actions::TRANSFER))
    }

    fn preview_drop(&mut self, ui: &egui::Ui, p: &Palette, world: &World, rect: egui::Rect) {
        if let Some(target) = super::resource_drop::Target::hovered(ui, rect, self.materials, true)
            && let Some(resource) = target.receive(ui.ctx(), p, &world.inventory, |item| {
                !lacks_rights(item, world.agent_id, self.required_permissions | self.drop_permissions)
            })
        {
            self.select(
                resource.asset,
                Some(resource.item.id),
                self.previewable(&world.inventory, world.agent_id, resource.item.id),
            );
            self.tree.opened(resource.asset);
            self.reveal_selection = true;
        }
    }

    fn take_swatch_drop(&mut self, ctx: &egui::Context, p: &Palette, inv: &Inventory, agent: Uuid) -> Option<Uuid> {
        self.dropped_slot = None;
        let copy_transfer = self.drop_copy_transfer;
        let required = self.required_permissions | self.drop_permissions;
        let resource = self.drop_target.take()?.receive(ctx, p, inv, |item| {
            (!copy_transfer || copy_and_transfer(inv, item)) && !lacks_rights(item, agent, required)
        })?;
        self.selected = Some(resource.asset);
        self.selected_item = Some(resource.item.id);
        self.dropped_slot = Some(self.drop_slot);
        self.open = false;
        self.stop_pipette(ctx);
        self.cancelled = false;
        self.preview = None;
        Some(resource.asset)
    }

    pub fn open(&mut self, title: &str, current: Uuid) {
        self.open = true;
        self.pipette_id = Some(egui::Id::unique("texture_picker_session").with(Uuid::new_v4()));
        self.pipette_error = false;
        self.pipette_lookup = None;
        self.title = title.to_owned();
        self.filter.clear();
        self.selected = (!current.is_nil()).then_some(current);
        self.selected_item = None;
        self.selected_lookup = None;
        self.source = 0;
        self.reveal_selection = true;
        self.tree.opened(current);
        self.local_open = [true; 2];
        self.materials = false;
        self.default = None;
        self.allow_none = false;
        self.extra_buttons = false;
        self.allow_local = false;
        self.required_permissions = 0;
        self.drop_permissions = 0;
        self.live_permissions = 0;
        self.owned_target = true;
        self.uuid_text = current.to_string();
        self.uuid_from_user = false;
        self.uuid_lookup = None;
        self.uuid_error = false;
        self.footer_height = 0.0;
        self.local_row_height = 0.0;
        self.material_model = None;
        self.material_view.reset_camera();
        self.preview = None;
        self.original = current;
        self.live_value = current;
        self.cancelled = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// The build floater's picker: "Default" (`default`), "Blank",
    /// "Transparent", "None" when `allow_none`.
    pub fn open_with_defaults(&mut self, title: &str, current: Uuid, default: Option<Uuid>, allow_none: bool) {
        self.open(title, current);
        self.default = default;
        self.allow_none = allow_none;
        self.extra_buttons = true;
    }

    /// PICK_MATERIAL: the inventory's materials, "Blank" = blank material.
    pub fn open_materials(&mut self, title: &str, current: Uuid) {
        self.open(title, current);
        self.materials = true;
        self.allow_none = true;
        self.extra_buttons = true;
    }

    /// Terrain requires a material in every slot; Blank remains available.
    pub(crate) fn open_terrain_materials(&mut self, title: &str, current: Uuid) {
        self.open_materials(title, current);
        self.allow_none = false;
    }

    /// Demo: open on the Local tab (kept only where local assets are allowed).
    pub(crate) fn show_local_tab(&mut self) {
        self.source = 1;
    }

    pub(crate) fn take_preview(&mut self) -> Option<Uuid> {
        self.preview.take()
    }

    pub(crate) fn forget_local_asset(&mut self, id: Uuid) {
        if self.selected == Some(id) {
            self.selected = None;
            self.selected_item = None;
            self.preview = None;
            self.uuid_text.clear();
        }
    }

    /// LLTextureCtrl TEXTURE_CHANGE / SELECT / CANCEL (lltexturectrl.cpp):
    /// intentional choices apply while the picker stays open; Cancel restores
    /// the value captured on opening. Closing keeps applied choices, and OK
    /// confirms without a duplicate update (including restricted choices).
    /// Callers keep their target until `open` becomes false. Multi-face editors
    /// use `show` / `take_preview` to preserve each face's original separately.
    pub(crate) fn show_live(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<Uuid> {
        if let Some(asset) = self.take_swatch_drop(ctx, p, &world.inventory, world.agent_id) {
            self.live_value = asset;
            return Some(asset);
        }
        let accepted = self.show(ctx, p, world, images);
        let value = accepted.or_else(|| if self.cancelled { Some(self.original) } else { self.take_preview() })?;
        if value == self.live_value {
            return None;
        }
        self.live_value = value;
        Some(value)
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled
    }

    /// The open picker shows another current value without reopening, like
    /// LLTextureCtrl::setImageAssetID when the selection changes
    /// (LLFloaterTexturePicker::setImageID, indra/newview/lltexturectrl.cpp).
    pub(crate) fn follow(&mut self, current: Uuid) {
        let selected = (!current.is_nil()).then_some(current);
        if self.selected == selected {
            return;
        }
        self.select(current, None, false);
        self.selected = selected;
        self.selected_lookup = None;
        self.tree.opened(current);
        self.source = 0;
        self.reveal_selection = true;
    }

    #[cfg(test)]
    pub(crate) fn shown(&self) -> Option<Uuid> {
        self.selected
    }

    /// Reuse the library's worker-built GPU preview for this selection only.
    pub(crate) fn prepare_material_preview(&mut self, ctx: &egui::Context, scene: &mut crate::scene::Scene) {
        self.material_previews.poll();
        self.material_model = if self.open && self.materials {
            self.selected
                .filter(|asset| !asset.is_nil())
                .and_then(|asset| self.material_previews.model(asset, scene, &mut self.wanted_images, ctx))
        } else {
            None
        };
    }

    fn select(&mut self, asset: Uuid, item: Option<Uuid>, preview: bool) {
        self.reveal_selection = false;
        self.pipette_error = false;
        self.uuid_text = asset.to_string();
        self.uuid_from_user = false;
        self.uuid_lookup = None;
        self.uuid_error = false;
        self.selected = Some(asset);
        self.selected_item = item;
        self.preview = preview.then_some(asset);
        self.material_model = None;
    }

    fn refresh_uuid_text(&mut self, world: &World) {
        if self.uuid_from_user {
            return;
        }
        let local = self.selected.is_some_and(|asset| world.local_assets.contains_key(&asset));
        let lookup = (world.inventory.generation, self.selected, local);
        if self.uuid_lookup == Some(lookup) {
            return;
        }
        // The shared rule scans the inventory: only repeat it when the choice,
        // permissions or local status changes, never on every rendered frame.
        self.uuid_lookup = Some(lookup);
        let shown = self.selected.filter(|asset| {
            local
                || [BLANK_TEXTURE, TRANSPARENT_TEXTURE, BLANK_MATERIAL].contains(asset)
                || crate::scene::avatar::BAKES.iter().any(|(_, _, magic)| magic == asset)
                || actions::asset_uuid_shown(&world.inventory, *asset)
        });
        self.uuid_text = shown.unwrap_or_default().to_string();
    }

    /// Editing windows identify a BOM channel by its coloured BAKED_* image.
    /// Only scene rendering resolves it to the wearer's current bake.
    fn preview_texture(&self) -> Option<Uuid> {
        self.selected.filter(|asset| !self.materials && !asset.is_nil())
    }

    /// The asset chosen with "OK" (the picker closes). Nil: "None".
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<Uuid> {
        #[cfg(test)]
        ctx.data_mut(|data| data.remove::<(egui::Rect, egui::Rect)>(egui::Id::unique("picker_selected_row")));
        self.preview = None;
        self.cancelled = false;
        if let Some(asset) = self.take_swatch_drop(ctx, p, &world.inventory, world.agent_id) {
            return Some(asset);
        }
        self.pipette_input(ctx, &world.inventory);
        if !self.open {
            return None;
        }
        let mut chosen = None;
        let mut open = true;
        let screen = ctx.content_rect();
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
        // LLFloaterTexturePicker::setImageID reveals the current inventory
        // item. A previous Local/Library tab and a virtualized, scrolled list
        // must not hide that selection when another control is opened (#2362).
        if self.reveal_selection && self.source == 0 {
            if self.allow_local && self.selected.is_some_and(|asset| world.local_assets.contains_key(&asset)) {
                self.source = 1;
            } else if self
                .selected_item
                .is_some_and(|id| super::texture_library::catalogue::contains(&world.inventory, id))
            {
                self.source = 2;
            }
        }
        let size = Vec2::new(320.0, 720.0);
        // Adopt the portrait default once for layouts saved by the wider picker.
        // Later manual resizing remains part of the user's persisted layout.
        let layout_id = egui::Id::unique("texture_picker").with("portrait_default");
        let adopt_default = ctx.data_mut(|data| {
            let adopted = data.get_persisted::<bool>(layout_id).unwrap_or(false);
            data.insert_persisted(layout_id, true);
            !adopted
        });
        if adopt_default {
            widgets::size_floater(ctx, "texture_picker", size);
        }
        let mut floater = Floater::new("texture_picker", self.title.clone(), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(320.0, 480.0);
        let preview_texture = self.preview_texture();
        floater.show(ctx, p, &mut open, |ui| {
            let radius = ui.visuals().window_corner_radius;
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    let side = 132.0;
                    if self.materials
                        && let Some(model) = self.material_model.clone()
                    {
                        self.material_view.background = super::texture_library::previews::view(p).background;
                        let response = aurora_render::preview::paint(
                            ui,
                            ui.scope_id().with("selected_material_preview"),
                            model,
                            &mut self.material_view,
                            Vec2::splat(side),
                        );
                        #[cfg(test)]
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(egui::Id::unique("picker_material_scene"), response.rect));
                        if self.selected.is_some_and(|id| world.local_assets.contains_key(&id)) {
                            widgets::local_texture::paint(ui, p, response.rect);
                        }
                        self.preview_drop(ui, p, world, response.rect);
                        return;
                    }
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), egui::Sense::hover());
                    ui.painter().rect_filled(rect, radius, p.field);
                    if let Some(a) = preview_texture {
                        self.wanted_images.insert(a);
                        if let Some(t) = images.get(&a) {
                            ui.painter().image(
                                t.id(),
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                widgets::PICTURE_TINT,
                            );
                            let size = t.size();
                            ui.label(RichText::new(format!("{} × {}", size[0], size[1])).size(11.0).color(p.muted));
                        } else if a == BLANK_TEXTURE {
                            // IMG_WHITE has known content while its metadata streams.
                            // theme-guard: content — the blank texture is white
                            ui.painter().rect_filled(rect, radius, egui::Color32::WHITE);
                        } else if a == TRANSPARENT_TEXTURE {
                            let cell = 12.0;
                            for y in 0..(side / cell).ceil() as usize {
                                for x in 0..(side / cell).ceil() as usize {
                                    let tile = egui::Rect::from_min_size(
                                        rect.min + egui::vec2(x as f32 * cell, y as f32 * cell),
                                        Vec2::splat(cell),
                                    )
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

                    if self.materials || (self.selected.is_some() && preview_texture.is_none()) {
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if self.selected.is_some_and(|asset| !asset.is_nil()) {
                                aurora_i18n::tr!("common-loading-2")
                            } else {
                                aurora_i18n::tr!("common-none-2")
                            },
                            egui::FontId::proportional(11.5),
                            p.muted,
                        );
                    }
                    if self.selected.is_some_and(|id| world.local_assets.contains_key(&id)) {
                        widgets::local_texture::paint(ui, p, rect);
                    }
                    self.preview_drop(ui, p, world, rect);
                });
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("ui-texture-picker-preview"))
                            .size(12.0)
                            .color(p.muted),
                    );
                    let name = self
                        .selected_item
                        .and_then(|id| world.inventory.items.get(&id))
                        .map(|it| it.name.as_str())
                        .or_else(|| {
                            self.selected
                                .and_then(|id| world.local_assets.get(&id))
                                .map(|(name, _)| name.as_str())
                        })
                        .or_else(|| {
                            bom_choices()
                                .into_iter()
                                .find(|(_, asset)| Some(*asset) == self.selected)
                                .map(|(name, _)| name)
                        });
                    if let Some(name) = name {
                        ui.add(widgets::elided_label(
                            ui,
                            RichText::new(name).size(12.0).color(p.ink),
                            ui.available_width(),
                        ));
                    }
                    self.pipette_button(ui, p);
                });
            });
            ui.add_space(8.0);
            widgets::tabs(
                ui,
                p,
                &mut self.source,
                &[
                    (aurora_i18n::tr!("picker-inventory-tab"), true),
                    (aurora_i18n::tr!("picker-local-tab"), self.allow_local),
                    (aurora_i18n::tr!("picker-aurora-tab"), true),
                ],
            );
            if self.source == 1 && !self.allow_local {
                self.source = 0;
            }
            if self.source != 0 {
                self.stop_pipette(ctx);
            }
            let inv = &world.inventory;
            if self.library_lookup.map(|(generation, root, _)| (generation, root)) != Some((inv.generation, inv.root)) {
                self.library_lookup = Some((inv.generation, inv.root, super::texture_library::catalogue::root(inv)));
            }
            let library = self.library_lookup.and_then(|(_, _, root)| root);
            let root = if self.source == 2 {
                library.unwrap_or(Uuid::nil())
            } else {
                world.inventory.root
            };
            let hint = if self.materials {
                aurora_i18n::tr!("ui-texture-picker-filter-materials")
            } else {
                aurora_i18n::tr!("common-filter-textures")
            };
            let was_filtering = !self.filter.trim().is_empty();
            let width = ui.available_width();
            let search = ui
                .horizontal(|ui| widgets::clearable_search_box(ui, p, &mut self.filter, hint, width))
                .inner;
            let cleared = search.changed() && was_filtering && self.filter.trim().is_empty();
            #[cfg(test)]
            ctx.data_mut(|data| data.insert_temp(egui::Id::unique("picker_search_field"), (search.id, search.rect)));
            if cleared {
                // LLFloaterTexturePicker::onFilterEdit restores browsing and
                // opens the selected item's ancestors before scrolling to it.
                self.reveal_selection = true;
                if self.source == 1 {
                    self.local_open[0] = true;
                }
            }
            self.tree.prepare_root(
                &mut world.inventory,
                world.agent_id,
                self.materials,
                &self.filter,
                root,
                super::inventory::view::picker_sort(ctx),
            );
            if cleared
                && self.source != 1
                && let Some(item) = self.selected_item
            {
                self.tree.state.reveal(&world.inventory, item, 0);
                self.tree.state.prepare(&self.tree.view, root, 0, true);
            }
            ui.add_space(6.0);
            let font_scale = super::fonts::scale(ui.ctx());
            let footer = if self.footer_height > 0.0 && self.footer_font_scale == font_scale {
                self.footer_height
            } else {
                // Keep the first frame within its saved height before the
                // scaled controls' actual footer has been measured, including
                // reopening or a live text-size change.
                let base = if self.extra_buttons {
                    if self.materials { 200.0 } else { 240.0 }
                } else {
                    90.0
                };
                base * font_scale.max(1.0)
            };
            let local_tab = self.allow_local && self.source == 1;
            // The Local tab's buttons sit under its list, outside the footer
            // shared by every tab.
            let local_row = if !local_tab {
                0.0
            } else if self.local_row_height > 0.0 && self.footer_font_scale == font_scale {
                self.local_row_height
            } else {
                32.0 * font_scale.max(1.0)
            };
            let (body, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), (ui.available_height() - footer - local_row).max(48.0)),
                egui::Sense::hover(),
            );
            let mut list = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("texture_list", self.source))
                    .max_rect(body)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            list.spacing_mut().item_spacing.y = 0.0;
            let live_permissions = self.live_permissions;
            let owned_target = self.owned_target;
            let rows = &self.tree.state.rows;
            let filter = self.filter.trim().to_lowercase();
            let mut locals: Vec<_> = world
                .local_assets
                .iter()
                .filter(|(_, (name, material))| {
                    self.allow_local && *material == self.materials && (filter.is_empty() || name.to_lowercase().contains(&filter))
                })
                .collect();
            locals.sort_by(|a, b| super::inventory::view::picker_name_cmp(&a.1.0, &b.1.0).then_with(|| a.0.cmp(b.0)));
            let tab = usize::from(!self.filter.trim().is_empty());
            let local_count = if self.allow_local && self.source == 1 {
                1 + if self.local_open[tab] { locals.len() } else { 0 }
            } else {
                0
            };
            let row_count = local_count + if self.source == 1 { 0 } else { rows.len() };
            let mut toggle = None;
            let mut toggle_local = false;
            let radius = ui.visuals().window_corner_radius;
            let mut scroll = egui::ScrollArea::vertical();
            if self.reveal_selection {
                let row = if self.source == 1 {
                    locals.iter().position(|(id, _)| Some(**id) == self.selected).map(|index| index + 1)
                } else {
                    rows.iter().position(|row| Some(row.id) == self.selected_item)
                };
                if let Some(index) = row {
                    scroll = scroll.vertical_scroll_offset((index as f32 * 22.0 - (body.height() - 22.0) * 0.5).max(0.0));
                    self.reveal_selection = false;
                }
            }
            scroll
                .auto_shrink([false, false])
                .max_height(body.height())
                .show_rows(&mut list, 22.0, row_count, |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for index in range {
                        let inv = &world.inventory;
                        let agent = world.agent_id;
                        let local = index < local_count;
                        let (id, depth, folder, row_open, name, asset) = if local {
                            if index == 0 {
                                (
                                    Uuid::nil(),
                                    0,
                                    true,
                                    self.local_open[tab],
                                    aurora_i18n::tr!("ui-texture-picker-local-files").to_owned(),
                                    None,
                                )
                            } else {
                                let (id, (name, _)) = locals[index - 1];
                                (*id, 1, false, false, name.clone(), Some(*id))
                            }
                        } else {
                            let row = &rows[index - local_count];
                            let name = if row.id == inv.root {
                                aurora_i18n::tr!("common-my-inventory").to_owned()
                            } else if row.id == inv.lib_root {
                                aurora_i18n::tr!("common-library").to_owned()
                            } else {
                                actions::name(inv, row.id)
                            };
                            (
                                row.id,
                                row.depth,
                                row.folder,
                                row.open,
                                name,
                                (!row.folder).then(|| inventory_asset(inv, row.id, self.materials)).flatten(),
                            )
                        };
                        let selected = !folder
                            && if local {
                                self.selected == asset
                            } else {
                                self.selected_item == Some(id)
                            };
                        let locked = !local
                            && !folder
                            && actions::original(inv, id)
                                .and_then(|id| inv.items.get(&id))
                                .is_some_and(|item| lacks_rights(item, agent, self.required_permissions));
                        let enabled = folder || asset.is_some() && !locked;
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 22.0),
                            if enabled { egui::Sense::click() } else { egui::Sense::hover() },
                        );
                        #[cfg(test)]
                        if selected {
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(egui::Id::unique("picker_selected_row"), (rect, ui.clip_rect())));
                        }
                        if local && !folder && wearable_demo_input() {
                            ui.ctx()
                                .data_mut(|data| data.insert_temp(egui::Id::unique(("wearable_picker_row", id)), rect));
                        }
                        if selected || response.hovered() {
                            ui.painter().rect_filled(rect, radius, if selected { p.violet } else { p.raised });
                        }
                        let mut x = rect.left() + 4.0 + depth.min(16) as f32 * 12.0;
                        if folder {
                            let c = egui::pos2(x + 5.0, rect.center().y);
                            let points = if row_open {
                                vec![c + egui::vec2(-3.5, -2.0), c + egui::vec2(3.5, -2.0), c + egui::vec2(0.0, 2.5)]
                            } else {
                                vec![c + egui::vec2(-2.0, -3.5), c + egui::vec2(2.5, 0.0), c + egui::vec2(-2.0, 3.5)]
                            };
                            ui.painter().add(egui::Shape::convex_polygon(points, p.muted, egui::Stroke::NONE));
                        }
                        x += 12.0;
                        let icon = if folder {
                            if row_open { "folder-open-filled" } else { "folder-filled" }
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
                                match (selected, enabled) {
                                    (true, _) => p.on_accent,
                                    (false, true) => p.violet_light,
                                    (false, false) => p.muted_dim,
                                },
                            );
                        }
                        x += 20.0;
                        let cut = widgets::paint_elided(
                            ui,
                            &name,
                            egui::pos2(x, rect.center().y),
                            rect.right() - x - 4.0,
                            egui::FontId::proportional(12.0),
                            match (selected, enabled) {
                                (true, _) => p.on_accent,
                                (false, true) => p.ink,
                                (false, false) => p.muted_dim,
                            },
                        );
                        let response = if cut { response.on_hover_text(&name) } else { response };
                        let response = if locked {
                            response.on_hover_text(aurora_i18n::tr!("build-ui-texture-drop-restricted"))
                        } else {
                            response
                        };
                        if response.clicked() {
                            if folder {
                                if local {
                                    toggle_local = true;
                                } else {
                                    toggle = Some(id);
                                }
                            } else if let Some(asset) = asset {
                                self.reveal_selection = false;
                                self.selected = Some(asset);
                                self.selected_item = (!local).then_some(id);
                                self.uuid_text = asset.to_string();
                                self.uuid_from_user = false;
                                self.uuid_lookup = None;
                                self.uuid_error = false;
                                self.preview = (local || Self::previewable_with(live_permissions, &world.inventory, world.agent_id, id))
                                    .then_some(asset);
                            }
                        }
                        if !folder
                            && response.double_clicked()
                            && !Self::confirm_blocked_with(owned_target, self.selected_item, &world.inventory, world.agent_id)
                        {
                            chosen = self.selected;
                        }
                    }
                });
            if let Some(id) = toggle {
                self.tree.state.toggle(id, tab);
            }
            if toggle_local {
                self.local_open[tab] = !self.local_open[tab];
            }
            let local_top = ui.cursor().top();
            if local_tab {
                // LLFloaterTexturePicker's local mode: Add, Remove, Upload.
                ui.add_space(6.0);
                let chosen_local = self.selected.filter(|asset| world.local_assets.contains_key(asset));
                ui.horizontal(|ui| {
                    let gap = ui.spacing().item_spacing.x;
                    let size = Vec2::new((ui.available_width() - 2.0 * gap) / 3.0, 26.0);
                    let flat = widgets::ActionKind::Flat;
                    if widgets::action_button(ui, p, "plus", aurora_i18n::tr!("common-add"), flat, size)
                        .on_hover_text(aurora_i18n::tr!("ui-texture-picker-local-add-tip"))
                        .clicked()
                    {
                        world
                            .local_asset_requests
                            .push(LocalAssetRequest::Add { materials: self.materials });
                    }
                    let enabled = chosen_local.is_some();
                    if widgets::action_button_if(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), flat, size, enabled).clicked()
                        && let Some(id) = chosen_local
                    {
                        world.local_asset_requests.push(LocalAssetRequest::Remove(id));
                        self.selected = None;
                        self.uuid_text.clear();
                    }
                    if widgets::action_button_if(ui, p, "file-upload-filled", aurora_i18n::tr!("import-upload"), flat, size, enabled)
                        .on_hover_text(aurora_i18n::tr!("ui-texture-picker-local-upload-tip"))
                        .clicked()
                        && let Some(id) = chosen_local
                    {
                        world.local_asset_requests.push(LocalAssetRequest::Upload(id));
                    }
                });
            }
            let local_bottom = ui.cursor().top();

            ui.add_space(6.0);
            if self.extra_buttons {
                let presets = [
                    (aurora_i18n::tr!("common-default"), self.default),
                    (
                        aurora_i18n::tr!("common-blank"),
                        Some(if self.materials { BLANK_MATERIAL } else { BLANK_TEXTURE }),
                    ),
                    (
                        aurora_i18n::tr!("common-transparent"),
                        (!self.materials).then_some(TRANSPARENT_TEXTURE),
                    ),
                    (aurora_i18n::tr!("common-none-2"), self.allow_none.then_some(Uuid::nil())),
                ];
                for row in presets.chunks(2) {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        for &(label, asset) in row {
                            if ui
                                .add_enabled_ui(asset.is_some(), |ui| {
                                    widgets::flat_button_sized(ui, p, label, Vec2::new(width, 26.0))
                                })
                                .inner
                                .clicked()
                                && let Some(asset) = asset
                            {
                                self.select(asset, None, true);
                            }
                        }
                    });
                }
                if !self.materials {
                    let choices = bom_choices();
                    let current = choices.iter().find(|(_, asset)| Some(*asset) == self.selected);
                    let label = current.map_or(aurora_i18n::tr!("picker-bom"), |(name, _)| *name);
                    egui::ComboBox::from_id_salt("picker_bom")
                        .width(ui.available_width())
                        .selected_text(RichText::new(label).size(12.0))
                        .show_ui(ui, |ui| {
                            for (name, asset) in choices {
                                if ui
                                    .selectable_label(self.selected == Some(asset), RichText::new(name).size(12.0))
                                    .clicked()
                                {
                                    self.select(asset, None, true);
                                }
                            }
                        });
                }
                ui.add_space(5.0);
                self.refresh_uuid_text(world);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-texture-picker-uuid")).size(11.0).color(p.muted));
                    let response = ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::TextEdit::singleline(&mut self.uuid_text)
                            .font(egui::FontId::monospace(10.0))
                            .align(egui::Align2::LEFT_CENTER),
                    );
                    if response.changed() {
                        match Uuid::parse_str(self.uuid_text.trim()) {
                            Ok(asset) if !asset.is_nil() || self.allow_none => self.select(asset, None, true),
                            _ => self.uuid_error = true,
                        }
                        // Preserve only text the user supplied, including partial input.
                        self.uuid_from_user = true;
                    }
                });
            }
            let message = if self.pipette_error {
                if self.materials {
                    aurora_i18n::tr!("picker-pipette-no-material")
                } else {
                    aurora_i18n::tr!("picker-pipette-no-texture")
                }
            } else if self.uuid_error {
                aurora_i18n::tr!("ui-texture-picker-invalid-uuid")
            } else if self.confirm_blocked(&world.inventory, world.agent_id) {
                aurora_i18n::tr!("ui-texture-picker-owner-only")
            } else if self
                .selected_item
                .is_some_and(|id| !self.previewable(&world.inventory, world.agent_id, id))
            {
                if self.live_permissions == 0 {
                    aurora_i18n::tr!("ui-texture-picker-confirm-no-copy")
                } else {
                    aurora_i18n::tr!("ui-texture-picker-no-live-preview")
                }
            } else if self.source == 2 && library.is_none() {
                aurora_i18n::tr!("picker-empty-library")
            } else if self.tree.loading {
                aurora_i18n::tr!("common-loading-2")
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
                if self.uuid_error || self.pipette_error { p.danger } else { p.muted },
            );
            ui.interact(status, ui.scope_id().with("picker_status"), egui::Sense::hover())
                .on_hover_text(message);
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width(), 26.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add_enabled_ui(
                            self.selected.is_some() && !self.uuid_error && !self.confirm_blocked(&world.inventory, world.agent_id),
                            |ui| widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-ok"), Vec2::new(72.0, 26.0)),
                        )
                        .inner
                        .clicked()
                    {
                        chosen = self.selected;
                    }
                    let cancel = widgets::flat_button_sized(ui, p, aurora_i18n::tr!("common-cancel"), Vec2::new(84.0, 26.0));
                    if wearable_demo_input() {
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(egui::Id::unique("wearable_picker_cancel"), cancel.rect));
                    }
                    if cancel.clicked() {
                        self.open = false;
                        self.cancelled = true;
                    }
                },
            );
            if local_tab {
                self.local_row_height = (local_bottom - local_top).max(1.0);
            }
            self.footer_height = (ui.cursor().top() - body.bottom() - (local_bottom - local_top)).max(1.0);
            self.footer_font_scale = font_scale;
        });
        if !open || chosen.is_some() {
            self.open = false;
        }
        if !self.open {
            self.stop_pipette(ctx);
        }
        chosen
    }
}

fn wearable_demo_input() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("AURORA_DEMO").as_deref() == Ok("1")
            && std::env::var("AURORA_DEMO_WEARABLE").is_ok_and(|mode| mode.contains(":picker-"))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn a_worn_object_greys_out_a_texture_without_copy_and_transfer() {
        let agent = Uuid::from_u128(0xd0);
        let both = actions::COPY | actions::TRANSFER;
        let item = |owner_mask| aurora_net::inventory::InvItem {
            owner: agent,
            owner_mask,
            ..Default::default()
        };
        assert!(!lacks_rights(&item(both), agent, both), "no-modify is enough");
        assert!(lacks_rights(&item(actions::COPY), agent, both));
        assert!(lacks_rights(&item(actions::TRANSFER), agent, both));
        assert!(!lacks_rights(&item(0), agent, 0), "nothing required off a worn object");
    }

    #[test]
    fn a_build_picker_previews_copy_and_transfer_items_and_keeps_ok_for_the_owner() {
        let mut inv = Inventory::default();
        let agent = Uuid::from_u128(0xd0);
        let both = actions::COPY | actions::TRANSFER;
        for (n, owner_mask) in [(1u128, both), (2, actions::COPY | actions::MODIFY), (3, actions::FULL)] {
            let id = Uuid::from_u128(n);
            inv.items.insert(
                id,
                aurora_net::inventory::InvItem {
                    id,
                    owner: agent,
                    owner_mask,
                    ..Default::default()
                },
            );
        }
        let (both_item, no_transfer, full) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let mut picker = TexturePicker::default();
        // any picker: copy is enough for a live preview
        assert!(picker.previewable(&inv, agent, no_transfer));
        picker.live_permissions = both;
        assert!(picker.previewable(&inv, agent, both_item));
        assert!(!picker.previewable(&inv, agent, no_transfer), "no live preview without transfer");
        picker.live_permissions = actions::FULL;
        assert!(!picker.previewable(&inv, agent, both_item), "a material needs modify too");
        assert!(picker.previewable(&inv, agent, full));
        // OK: a no-transfer item only on an object the agent owns, not for sale
        picker.selected_item = Some(no_transfer);
        picker.owned_target = true;
        assert!(!picker.confirm_blocked(&inv, agent));
        picker.owned_target = false;
        assert!(picker.confirm_blocked(&inv, agent));
        picker.selected_item = Some(both_item);
        assert!(!picker.confirm_blocked(&inv, agent));
    }

    #[test]
    fn direct_thumbnail_drops_apply_the_same_asset_to_each_slot_without_opening_a_picker() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let item = Uuid::from_u128(0xd001);
        let asset = Uuid::from_u128(0xd002);
        world.inventory.items.insert(
            item,
            aurora_net::inventory::InvItem {
                id: item,
                asset_id: asset,
                asset_type: 0,
                inv_type: 0,
                owner_mask: actions::COPY,
                ..Default::default()
            },
        );
        let mut picker = TexturePicker::default();
        let mut ui = PickerUi::new();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(100.0, 100.0));
        let mut choices = Vec::new();
        let draw = |picker: &mut TexturePicker, world: &mut World, choices: &mut Vec<Uuid>, ctx: &egui::Context, p: &Palette| {
            egui::Area::new(egui::Id::unique("thumbnail_drop_test"))
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
                    picker.drop_target(ui, rect, false, true);
                });
            if let Some(asset) = picker.show_live(ctx, p, world, &HashMap::new()) {
                choices.push(asset);
            }
        };
        ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut choices, ctx, p));
        for _ in 0..2 {
            egui::DragAndDrop::set_payload(&ui.ctx, super::super::inventory::InvDrag(item));
            ui.frame(vec![egui::Event::PointerMoved(rect.center())], |ctx, p| {
                draw(&mut picker, &mut world, &mut choices, ctx, p)
            });
            ui.frame(
                vec![egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                |ctx, p| draw(&mut picker, &mut world, &mut choices, ctx, p),
            );
            assert!(!picker.open);
        }
        assert_eq!(
            choices,
            vec![asset, asset],
            "each slot gets its own commit, even when a previous slot chose the same asset"
        );
    }

    #[test]
    fn a_copy_and_transfer_drop_target_takes_only_such_textures() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (copy_only, open) = (Uuid::from_u128(0xd011), Uuid::from_u128(0xd012));
        for (item, mask) in [(copy_only, actions::COPY), (open, actions::COPY | actions::TRANSFER)] {
            world.inventory.items.insert(
                item,
                aurora_net::inventory::InvItem {
                    id: item,
                    asset_id: Uuid::from_u128(item.as_u128() + 0x100),
                    asset_type: 0,
                    inv_type: 0,
                    owner_mask: mask,
                    ..Default::default()
                },
            );
        }
        let mut picker = TexturePicker::default();
        let mut ui = PickerUi::new();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(100.0, 100.0));
        let mut choices = Vec::new();
        let draw = |picker: &mut TexturePicker, world: &mut World, choices: &mut Vec<Uuid>, ctx: &egui::Context, p: &Palette| {
            egui::Area::new(egui::Id::unique("copy_transfer_drop_test"))
                .fixed_pos(rect.min)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
                    picker.drop_target_copy_transfer(ui, rect, true);
                });
            if let Some(asset) = picker.show_live(ctx, p, world, &HashMap::new()) {
                choices.push(asset);
            }
        };
        ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut choices, ctx, p));
        for item in [copy_only, open] {
            egui::DragAndDrop::set_payload(&ui.ctx, super::super::inventory::InvDrag(item));
            ui.frame(vec![egui::Event::PointerMoved(rect.center())], |ctx, p| {
                draw(&mut picker, &mut world, &mut choices, ctx, p)
            });
            ui.frame(
                vec![egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
                |ctx, p| draw(&mut picker, &mut world, &mut choices, ctx, p),
            );
        }
        assert_eq!(
            choices,
            vec![Uuid::from_u128(open.as_u128() + 0x100)],
            "copy without transfer is refused"
        );
    }

    use super::*;

    #[test]
    fn pipette_is_shared_by_plain_build_and_material_pickers_and_restores_on_cancel() {
        for mode in 0..3 {
            let mut ui = PickerUi::new();
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut picker = TexturePicker::default();
            let original = Uuid::from_u128(70);
            let asset = Uuid::from_u128(71);
            let item = Uuid::from_u128(72);
            world.inventory.items.insert(
                item,
                aurora_net::inventory::InvItem {
                    id: item,
                    asset_id: asset,
                    inv_type: if mode == 2 { INV_MATERIAL } else { INV_TEXTURE },
                    owner_mask: actions::COPY,
                    name: "Sampled asset".into(),
                    ..Default::default()
                },
            );
            match mode {
                0 => picker.open("Profile texture", original),
                1 => picker.open_with_defaults("Object texture", original, None, true),
                _ => picker.open_materials("Material", original),
            }
            let mut changes = Vec::new();
            let draw = |picker: &mut TexturePicker, world: &mut World, changes: &mut Vec<Uuid>, ctx: &egui::Context, p: &Palette| {
                if let Some(value) = picker.show_live(ctx, p, world, &HashMap::new()) {
                    changes.push(value);
                }
            };
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert!(pipette::armed(&ui.ctx));
            assert_eq!(pipette::needs_sample(&ui.ctx, (10.0, 20.0)), Some(mode == 2));
            assert_eq!(pipette::needs_sample(&ui.ctx, (10.0, 20.0)), None);
            pipette::sample(&ui.ctx, Some((Uuid::from_u128(80), 3, asset)));
            // A fast press/release must survive until the next UI frame.
            pipette::release(&ui.ctx);
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            assert_eq!(picker.selected, Some(asset));
            assert_eq!(picker.selected_item, Some(item));
            assert_eq!(changes, [asset]);
            assert!(!pipette::armed(&ui.ctx));
            assert!(picker.open);
            ui.click(aurora_i18n::tr!("common-cancel"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert_eq!(changes, [asset, original]);
        }
    }

    #[test]
    fn pipette_rejects_missing_and_no_copy_assets_and_stops_on_escape_close_or_source_change() {
        let mut ui = PickerUi::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut picker = TexturePicker::default();
        let original = Uuid::from_u128(90);
        let asset = Uuid::from_u128(91);
        let item = Uuid::from_u128(92);
        picker.open("Texture", original);
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        for present in [false, true] {
            if present {
                world.inventory.items.insert(
                    item,
                    aurora_net::inventory::InvItem {
                        id: item,
                        asset_id: asset,
                        inv_type: INV_TEXTURE,
                        ..Default::default()
                    },
                );
                world.inventory.generation += 1;
            }
            pipette::sample(&ui.ctx, Some((Uuid::from_u128(100), 0, asset)));
            ui.settle(|ctx, p| {
                picker.show(ctx, p, &mut world, &HashMap::new());
            });
            assert!(picker.pipette_error);
            assert!(pipette::failed(&ui.ctx));
            assert_eq!(picker.selected, Some(original));
            assert_eq!(picker.take_preview(), None);
        }
        ui.frame(
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            |ctx, p| {
                picker.show(ctx, p, &mut world, &HashMap::new());
            },
        );
        assert!(!pipette::armed(&ui.ctx));
        assert!(picker.open);
        ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        picker.allow_local = true;
        picker.show_local_tab();
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        assert!(!pipette::armed(&ui.ctx));
        picker.source = 0;
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        ui.close(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        assert!(!pipette::armed(&ui.ctx));
    }

    #[test]
    fn pipette_accepts_blank_material_and_does_not_deliver_to_a_reopened_picker() {
        let mut picker = TexturePicker::default();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        picker.open_materials("Material", Uuid::from_u128(110));
        // Exercise the same owner routing with a sample after release.
        let mut ui = PickerUi::new();
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        pipette::sample(&ui.ctx, Some((Uuid::nil(), 0, BLANK_MATERIAL)));
        pipette::release(&ui.ctx);
        picker.pipette_input(&ui.ctx, &world.inventory);
        assert_eq!(picker.take_preview(), Some(BLANK_MATERIAL));
        assert!(!picker.pipette_error);
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("color-picker-pipette"), |ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        pipette::sample(&ui.ctx, Some((Uuid::nil(), 0, BLANK_MATERIAL)));
        picker.open("Other caller", Uuid::nil());
        ui.settle(|ctx, p| {
            picker.show(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(picker.selected, None);
        assert!(!pipette::armed(&ui.ctx));
    }

    pub(crate) struct PickerUi {
        ctx: egui::Context,
        time: f64,
        labels: HashMap<String, egui::Pos2>,
        copied: Vec<String>,
    }

    impl PickerUi {
        pub(crate) fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            Self {
                ctx,
                time: 0.0,
                labels: HashMap::new(),
                copied: Vec::new(),
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>, mut draw: impl FnMut(&egui::Context, &Palette)) {
            self.time += 1.0 / 60.0;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |_| draw(&self.ctx, &crate::theme::Theme::default().palette()),
            );
            for command in &output.platform_output.commands {
                if let egui::OutputCommand::CopyText(text) = command {
                    self.copied.push(text.clone());
                }
            }
            output.textures_delta.clear();
            self.labels.clear();
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape {
                    self.labels
                        .insert(text.galley.text().to_owned(), text.pos + text.galley.size() * 0.5);
                }
            }
        }

        pub(crate) fn settle(&mut self, mut draw: impl FnMut(&egui::Context, &Palette)) {
            for _ in 0..5 {
                self.frame(vec![], &mut draw);
            }
        }

        pub(crate) fn close(&mut self, draw: impl FnMut(&egui::Context, &Palette)) {
            let rect = self
                .ctx
                .memory(|memory| memory.area_rect(egui::Id::unique("texture_picker")))
                .expect("picker");
            self.labels
                .insert("picker close".into(), egui::pos2(rect.right() - 18.0, rect.top() + 18.0));
            self.click("picker close", draw);
        }

        pub(crate) fn click(&mut self, label: &str, mut draw: impl FnMut(&egui::Context, &Palette)) {
            let pos = *self
                .labels
                .get(label)
                .unwrap_or_else(|| panic!("missing picker label {label:?}: {:?}", self.labels.keys()));
            self.time += 1.0;
            for pressed in [true, false] {
                self.frame(
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    &mut draw,
                );
            }
        }
    }

    #[test]
    fn live_picker_applies_repeated_choices_restores_on_cancel_and_confirms_once() {
        for materials in [false, true] {
            let mut ui = PickerUi::new();
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let mut picker = TexturePicker::default();
            let original = Uuid::from_u128(77);
            let open = |picker: &mut TexturePicker, current| {
                if materials {
                    picker.open_materials("Material", current);
                } else {
                    picker.open_with_defaults("Texture", current, None, true);
                }
            };
            let blank = if materials { BLANK_MATERIAL } else { BLANK_TEXTURE };
            let mut changes = Vec::new();
            open(&mut picker, original);
            let draw = |picker: &mut TexturePicker, world: &mut World, changes: &mut Vec<Uuid>, ctx: &egui::Context, p: &Palette| {
                if let Some(value) = picker.show_live(ctx, p, world, &HashMap::new()) {
                    changes.push(value);
                }
            };
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            assert!(changes.is_empty(), "opening is not an intentional edit");
            ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert_eq!(changes, [blank]);
            assert!(picker.open);
            ui.click(aurora_i18n::tr!("common-none-2"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert_eq!(changes, [blank, Uuid::nil()]);
            assert!(picker.open);
            ui.click(aurora_i18n::tr!("common-cancel"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert_eq!(changes, [blank, Uuid::nil(), original]);
            assert!(!picker.open);
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            assert_eq!(changes.len(), 3, "a closed picker cannot repeat the rollback");

            // Opening another control captures its own baseline.
            changes.clear();
            open(&mut picker, Uuid::nil());
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            ui.click(aurora_i18n::tr!("common-ok"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            assert_eq!(changes, [blank], "OK does not resend the live edit");
            assert!(!picker.open);
            assert!(!picker.cancelled());

            changes.clear();
            open(&mut picker, original);
            ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
                draw(&mut picker, &mut world, &mut changes, ctx, p)
            });
            ui.close(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
            assert_eq!(changes, [blank], "closing keeps the live choice without another edit");
            assert!(!picker.open);
            assert!(!picker.cancelled());
        }
    }

    #[test]
    fn live_picker_requires_confirmation_for_a_noncopyable_inventory_choice() {
        let mut ui = PickerUi::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let root = Uuid::from_u128(1);
        let item = Uuid::from_u128(2);
        let asset = Uuid::from_u128(3);
        world.inventory.root = root;
        world.inventory.folders.insert(
            root,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: root,
                    ..Default::default()
                },
                state: crate::world::inventory::FetchState::Fetched,
                children: Vec::new(),
                items: vec![item],
                library: false,
            },
        );
        world.inventory.items.insert(
            item,
            aurora_net::inventory::InvItem {
                id: item,
                parent: root,
                asset_id: asset,
                name: "Restricted texture".into(),
                inv_type: INV_TEXTURE,
                ..Default::default()
            },
        );
        let mut picker = TexturePicker::default();
        picker.open("Texture", Uuid::nil());
        let mut changes = Vec::new();
        let draw = |picker: &mut TexturePicker, world: &mut World, changes: &mut Vec<Uuid>, ctx: &egui::Context, p: &Palette| {
            if let Some(asset) = picker.show_live(ctx, p, world, &HashMap::new()) {
                changes.push(asset);
            }
        };
        ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
        ui.click("Restricted texture", |ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
        assert!(changes.is_empty(), "a noncopyable choice must not preview");
        assert!(picker.open);
        ui.close(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
        assert!(changes.is_empty(), "closing does not apply a restricted unconfirmed choice");
        picker.open("Texture", Uuid::nil());
        ui.settle(|ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
        ui.click("Restricted texture", |ctx, p| draw(&mut picker, &mut world, &mut changes, ctx, p));
        ui.click(aurora_i18n::tr!("common-ok"), |ctx, p| {
            draw(&mut picker, &mut world, &mut changes, ctx, p)
        });
        assert_eq!(changes, [asset]);
        assert!(!picker.open);
    }

    fn frame(
        ctx: &egui::Context,
        picker: &mut TexturePicker,
        world: &mut World,
        time: f64,
        events: Vec<egui::Event>,
    ) -> HashMap<String, egui::Pos2> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |_| {
                picker.show(ctx, &crate::theme::Theme::default().palette(), world, &HashMap::new());
            },
        );
        output.textures_delta.clear();
        let mut labels = HashMap::new();
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                labels
                    .entry(text.galley.text().to_owned())
                    .or_insert(text.pos + text.galley.size() * 0.5);
            }
        }
        labels
    }

    fn click(ctx: &egui::Context, picker: &mut TexturePicker, world: &mut World, time: f64, pos: egui::Pos2) {
        for (step, pressed) in [true, false].into_iter().enumerate() {
            frame(
                ctx,
                picker,
                world,
                time + step as f64 / 60.0,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    #[test]
    fn construction_material_picker_previews_only_the_selected_material() {
        let ctx = egui::Context::default();
        let avatars = std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load());
        let mut world = World::new(avatars.clone());
        let mut scene = crate::scene::Scene::new(std::env::temp_dir(), avatars);
        let assets = [Uuid::from_u128(101), Uuid::from_u128(102)];
        let maps = [Uuid::from_u128(201), Uuid::from_u128(202)];
        for (index, asset) in assets.into_iter().enumerate() {
            scene.materials.insert(
                asset,
                aurora_assets::PbrMaterial {
                    base_color_texture: Some(maps[index]),
                    metallic_factor: index as f32,
                    ..Default::default()
                },
            );
        }
        // the preview job runs on this thread: the model is there on the
        // next poll, whatever the load of the machine
        let _inline = crate::scene::jobs::execute_inline_on_this_thread();
        let mut picker = TexturePicker::default();
        picker.open_materials("Material", assets[0]);
        for (index, asset) in assets.into_iter().enumerate() {
            picker.wanted_images.clear();
            picker.select(asset, None, false);
            let ready = (0..3).any(|_| {
                picker.prepare_material_preview(&ctx, &mut scene);
                picker.material_model.is_some()
            });
            assert!(ready, "material preview not built");
            let model = picker.material_model.as_ref().unwrap();
            assert_eq!(model.parts[0].material.metallic, index as f32);
            assert_eq!(picker.wanted_images, HashSet::from([maps[index]]));
            for frame in 0..3 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 800.0))),
                        ..Default::default()
                    },
                    |_| {
                        picker.show(&ctx, &crate::theme::Theme::default().palette(), &mut world, &HashMap::new());
                    },
                );
                output.textures_delta.clear();
                if frame == 2 {
                    assert!(
                        output.shapes.iter().any(|shape| matches!(shape.shape, egui::Shape::Callback(_))),
                        "GPU material preview missing"
                    );
                }
            }
            let image = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("picker_material_scene")))
                .unwrap();
            let window = ctx.memory(|memory| memory.area_rect(egui::Id::unique("texture_picker"))).unwrap();
            let camera = picker.material_view;
            let start = image.center();
            let target = start + egui::vec2(35.0, 18.0);
            let time = 1.0 + index as f64 * 2.0;
            frame(&ctx, &mut picker, &mut world, time, vec![egui::Event::PointerMoved(start)]);
            frame(
                &ctx,
                &mut picker,
                &mut world,
                time + 0.02,
                vec![egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
            );
            frame(&ctx, &mut picker, &mut world, time + 0.04, vec![egui::Event::PointerMoved(target)]);
            frame(
                &ctx,
                &mut picker,
                &mut world,
                time + 0.06,
                vec![egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }],
            );
            assert_ne!(picker.material_view.yaw, camera.yaw, "dragging the material must orbit the preview");
            assert_ne!(picker.material_view.pitch, camera.pitch);
            let after = ctx.memory(|memory| memory.area_rect(egui::Id::unique("texture_picker"))).unwrap();
            assert!(
                (after.min - window.min).length() < 0.5,
                "orbit moved the picker: {window:?} -> {after:?}"
            );
            assert!((after.size() - window.size()).length() < 0.5, "orbit resized the picker");
            assert_eq!(picker.selected, Some(asset));
            assert_eq!(picker.take_preview(), None, "orbiting must not apply the material");
        }
    }

    #[test]
    fn construction_bom_selector_keeps_the_magic_uuid_and_resets_for_an_ordinary_texture() {
        let ctx = egui::Context::default();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let mut picker = TexturePicker::default();
        picker.open_with_defaults("Texture", BLANK_TEXTURE, None, true);
        let mut labels = HashMap::new();
        for n in 0..5 {
            labels = frame(&ctx, &mut picker, &mut world, n as f64 / 60.0, vec![]);
        }
        click(&ctx, &mut picker, &mut world, 1.0, labels[aurora_i18n::tr!("picker-bom")]);
        labels = frame(&ctx, &mut picker, &mut world, 1.1, vec![]);
        let (name, magic) = bom_choices()[0];
        click(&ctx, &mut picker, &mut world, 1.2, labels[name]);
        assert_eq!(picker.selected, Some(magic));
        assert_eq!(picker.take_preview(), Some(magic));
        assert_eq!(picker.uuid_text, magic.to_string());
        assert_eq!(
            picker.preview_texture(),
            Some(magic),
            "editing previews show the coloured BAKED_* reference image"
        );
        labels = frame(&ctx, &mut picker, &mut world, 1.3, vec![]);
        assert!(labels.contains_key(name));
        assert!(
            picker.wanted_images.contains(&magic),
            "request the BAKED_* image, including without an avatar bake"
        );
        click(&ctx, &mut picker, &mut world, 1.4, labels[aurora_i18n::tr!("common-blank")]);
        labels = frame(&ctx, &mut picker, &mut world, 1.5, vec![]);
        assert!(labels.contains_key(aurora_i18n::tr!("picker-bom")));
        assert_eq!(picker.selected, Some(BLANK_TEXTURE));
    }

    #[test]
    fn local_tab_filters_by_type_collapses_and_previews_a_real_click() {
        for materials in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let root = Uuid::from_u128(1);
            world.inventory.root = root;
            world.inventory.folders.insert(
                root,
                crate::world::inventory::Folder {
                    info: aurora_net::inventory::InvFolder {
                        id: root,
                        ..Default::default()
                    },
                    state: crate::world::inventory::FetchState::Fetched,
                    children: Vec::new(),
                    items: vec![Uuid::from_u128(20)],
                    library: false,
                },
            );
            world.inventory.items.insert(
                Uuid::from_u128(20),
                aurora_net::inventory::InvItem {
                    id: Uuid::from_u128(20),
                    parent: root,
                    asset_id: Uuid::from_u128(21),
                    inv_type: if materials { INV_MATERIAL } else { INV_TEXTURE },
                    name: "Inventory asset".into(),
                    owner_mask: actions::COPY,
                    ..Default::default()
                },
            );
            for (id, name, material) in [
                (10, "A texture", false),
                (11, "B texture", false),
                (12, "A material", true),
                (13, "B material", true),
            ] {
                world.local_assets.insert(Uuid::from_u128(id), (name.into(), material));
            }
            let mut picker = TexturePicker::default();
            if materials {
                picker.open_materials("Material", Uuid::nil());
            } else {
                picker.open_with_defaults("Texture", Uuid::nil(), None, false);
            }
            picker.allow_local = true;
            picker.source = 1;
            let folder = aurora_i18n::tr!("ui-texture-picker-local-files");
            let inventory = aurora_i18n::tr!("common-my-inventory");
            let (first, second, excluded, asset) = if materials {
                ("A material", "B material", "A texture", 12)
            } else {
                ("A texture", "B texture", "A material", 10)
            };
            let mut labels = HashMap::new();
            for n in 0..5 {
                labels = frame(&ctx, &mut picker, &mut world, n as f64 / 60.0, vec![]);
            }
            assert!(labels[folder].y < labels[first].y);
            assert!(labels[first].y < labels[second].y);
            assert!(!labels.contains_key(inventory));
            assert!(!labels.contains_key(excluded));
            click(&ctx, &mut picker, &mut world, 1.0, labels[first]);
            assert_eq!(picker.selected, Some(Uuid::from_u128(asset)));
            assert_eq!(picker.selected_item, None);
            assert_eq!(picker.take_preview(), Some(Uuid::from_u128(asset)));
            assert!(picker.open);
            click(&ctx, &mut picker, &mut world, 2.0, labels[folder]);
            labels = frame(&ctx, &mut picker, &mut world, 2.1, vec![]);
            assert!(!labels.contains_key(second));
            assert!(!labels.contains_key(inventory));
            picker.filter = "B".into();
            labels = frame(&ctx, &mut picker, &mut world, 3.0, vec![]);
            assert!(labels.contains_key(second));
            assert!(!labels.contains_key(excluded));
            picker.filter.clear();
            labels = frame(&ctx, &mut picker, &mut world, 4.0, vec![]);
            assert!(!labels.contains_key(second));
            world.local_assets.clear();
            labels = frame(&ctx, &mut picker, &mut world, 5.0, vec![]);
            assert!(!labels.contains_key(inventory));
            picker.allow_local = false;
            labels = frame(&ctx, &mut picker, &mut world, 6.0, vec![]);
            assert!(!labels.contains_key(folder));
        }
    }

    #[test]
    fn local_tab_buttons_add_remove_and_upload_local_files() {
        for materials in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let local = Uuid::from_u128(10);
            world.local_assets.insert(local, ("Local file".into(), materials));
            let mut picker = TexturePicker::default();
            if materials {
                picker.open_materials("Material", Uuid::nil());
            } else {
                picker.open_with_defaults("Texture", Uuid::nil(), None, false);
            }
            picker.allow_local = true;
            picker.source = 1;
            let (add, remove, upload) = (
                aurora_i18n::tr!("common-add"),
                aurora_i18n::tr!("common-remove"),
                aurora_i18n::tr!("import-upload"),
            );
            let mut labels = HashMap::new();
            for n in 0..5 {
                labels = frame(&ctx, &mut picker, &mut world, n as f64 / 60.0, vec![]);
            }
            // Remove and Upload wait for a local file to be chosen.
            assert!(labels["Local file"].y < labels[add].y);
            click(&ctx, &mut picker, &mut world, 1.0, labels[remove]);
            click(&ctx, &mut picker, &mut world, 1.1, labels[upload]);
            assert!(world.local_asset_requests.is_empty());
            click(&ctx, &mut picker, &mut world, 1.2, labels[add]);
            assert_eq!(world.local_asset_requests, vec![LocalAssetRequest::Add { materials }]);
            world.local_asset_requests.clear();
            click(&ctx, &mut picker, &mut world, 2.0, labels["Local file"]);
            assert_eq!(picker.selected, Some(local));
            click(&ctx, &mut picker, &mut world, 3.0, labels[upload]);
            assert_eq!(world.local_asset_requests, vec![LocalAssetRequest::Upload(local)]);
            world.local_asset_requests.clear();
            click(&ctx, &mut picker, &mut world, 4.0, labels[remove]);
            assert_eq!(world.local_asset_requests, vec![LocalAssetRequest::Remove(local)]);
            assert_eq!(picker.selected, None);
            // The other tabs keep the list's full height.
            picker.source = 0;
            labels = frame(&ctx, &mut picker, &mut world, 5.0, vec![]);
            assert!(!labels.contains_key(add));
        }
    }

    #[test]
    fn texture_uuid_uses_the_shared_asset_rule_and_resolves_links() {
        let mut inv = Inventory::default();
        let agent = Uuid::from_u128(91);
        let item = Uuid::from_u128(92);
        let link = Uuid::from_u128(93);
        let asset = Uuid::from_u128(96);
        let full = actions::COPY | actions::MODIFY | actions::TRANSFER;
        inv.items.insert(
            item,
            aurora_net::inventory::InvItem {
                id: item,
                owner: agent,
                asset_id: asset,
                owner_mask: full,
                ..Default::default()
            },
        );
        inv.items.insert(
            link,
            aurora_net::inventory::InvItem {
                id: link,
                asset_type: 24,
                asset_id: item,
                owner_mask: full,
                ..Default::default()
            },
        );
        assert_eq!(inventory_asset(&inv, link, false), Some(asset));
        assert!(actions::asset_uuid_shown(&inv, asset));
        for missing in [actions::COPY, actions::MODIFY, actions::TRANSFER] {
            inv.items.get_mut(&item).unwrap().owner_mask = full & !missing;
            assert!(!actions::asset_uuid_shown(&inv, asset));
        }
        let original = inv.items.get_mut(&item).unwrap();
        original.owner_mask = full;
        original.owner = Uuid::from_u128(94);
        assert!(actions::asset_uuid_shown(&inv, asset));
        inv.items.get_mut(&item).unwrap().everyone_mask = full;
        assert!(actions::asset_uuid_shown(&inv, asset));
        inv.items.remove(&item);
        assert!(!actions::asset_uuid_shown(&inv, asset));
    }

    #[test]
    fn texture_uuid_masks_unknown_assets_without_changing_selection_or_user_input() {
        let world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let hidden = Uuid::from_u128(95);
        let mut picker = TexturePicker::default();
        picker.open_with_defaults("test", hidden, None, true);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string());
        assert_eq!(picker.selected, Some(hidden));
        picker.uuid_text = "user supplied".into();
        picker.uuid_from_user = true;
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, "user supplied");
        picker.select(hidden, None, false);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string());
        picker.select(BLANK_TEXTURE, None, true);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, BLANK_TEXTURE.to_string());
    }

    #[test]
    fn texture_uuid_refreshes_permissions_without_applying_or_replacing_the_selection() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let asset = Uuid::from_u128(0x1943);
        let mut item = aurora_net::inventory::InvItem {
            id: Uuid::from_u128(0x1944),
            asset_id: asset,
            owner_mask: actions::FULL,
            ..Default::default()
        };
        world.inventory.add_items(vec![item.clone()]);
        let mut picker = TexturePicker::default();
        picker.open_with_defaults("Texture", asset, None, true);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, asset.to_string());
        for missing in [actions::COPY, actions::MODIFY, actions::TRANSFER] {
            item.owner_mask = actions::FULL & !missing;
            world.inventory.add_items(vec![item.clone()]);
            picker.refresh_uuid_text(&world);
            assert_eq!(picker.uuid_text, Uuid::nil().to_string());
            assert_eq!(picker.selected, Some(asset));
            assert_eq!(picker.take_preview(), None, "masking cannot apply a different texture");
            item.owner_mask = actions::FULL;
            world.inventory.add_items(vec![item.clone()]);
            picker.refresh_uuid_text(&world);
            assert_eq!(picker.uuid_text, asset.to_string());
        }
        world.inventory.remove(&[item.id]);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string());
        assert_eq!(picker.selected, Some(asset));
    }

    #[test]
    fn texture_uuid_builtin_and_local_exceptions_do_not_authorize_other_assets() {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let local = Uuid::from_u128(0x1945);
        let unknown = Uuid::from_u128(0x1946);
        let mut picker = TexturePicker::default();
        picker.open_materials("Material", local);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string());
        world.local_assets.insert(local, ("Local preview".into(), true));
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, local.to_string());
        world.local_assets.remove(&local);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string(), "local removal needs no inventory change");
        for asset in [BLANK_TEXTURE, TRANSPARENT_TEXTURE, BLANK_MATERIAL, aurora_prim::te::DEFAULT_TEXTURE]
            .into_iter()
            .chain(crate::scene::avatar::BAKES.iter().map(|(_, _, id)| *id))
        {
            picker.select(asset, None, true);
            picker.refresh_uuid_text(&world);
            assert_eq!(picker.uuid_text, asset.to_string());
            assert_eq!(picker.take_preview(), Some(asset));
        }
        picker.select(unknown, None, false);
        picker.refresh_uuid_text(&world);
        assert_eq!(picker.uuid_text, Uuid::nil().to_string());
        assert_eq!(picker.selected, Some(unknown));
    }

    #[test]
    fn texture_uuid_field_copies_zeros_and_preserves_user_input_until_a_new_choice() {
        for materials in [false, true] {
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let hidden = Uuid::from_u128(0x1947);
            let pasted = Uuid::from_u128(0x1948);
            let mut picker = TexturePicker::default();
            if materials {
                picker.open_materials("Material", hidden);
            } else {
                picker.open_with_defaults("Texture", hidden, None, true);
            }
            let mut ui = PickerUi::new();
            let draw = |picker: &mut TexturePicker, world: &mut World, ctx: &egui::Context, p: &Palette| {
                picker.show(ctx, p, world, &HashMap::new());
            };
            ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
            let zeros = Uuid::nil().to_string();
            assert!(ui.labels.contains_key(&zeros));
            assert!(!ui.labels.contains_key(&hidden.to_string()));
            assert_eq!(picker.selected, Some(hidden));
            ui.click(&zeros, |ctx, p| draw(&mut picker, &mut world, ctx, p));
            let select_all = || egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            };
            ui.frame(vec![select_all(), egui::Event::Copy], |ctx, p| {
                draw(&mut picker, &mut world, ctx, p)
            });
            assert_eq!(ui.copied.last(), Some(&zeros));
            ui.frame(vec![egui::Event::Paste(pasted.to_string())], |ctx, p| {
                draw(&mut picker, &mut world, ctx, p)
            });
            ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
            assert_eq!(picker.uuid_text, pasted.to_string());
            assert_eq!(picker.selected, Some(pasted));
            ui.frame(vec![select_all(), egui::Event::Paste("partial uuid".into())], |ctx, p| {
                draw(&mut picker, &mut world, ctx, p)
            });
            ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
            assert_eq!(picker.uuid_text, "partial uuid");
            assert!(picker.uuid_error);
            assert_eq!(picker.selected, Some(pasted));
            picker.select(pasted, None, false);
            ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
            assert_eq!(picker.uuid_text, zeros, "reselecting the same asset clears the manual exception");
            picker.open_with_defaults("Reopened", hidden, None, true);
            ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
            assert_eq!(picker.uuid_text, zeros);
        }
    }

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
    fn current_picker_item_is_visible_after_reopening_from_another_source() {
        for materials in [false, true] {
            for source in [0, 2] {
                let mut ui = PickerUi::new();
                let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                world.inventory.root = Uuid::from_u128(1);
                world.inventory.created_folders(
                    [(1, 0, "Inventory"), (2, 1, "#Aurora"), (3, 2, "Aurora Library")]
                        .into_iter()
                        .map(|(id, parent, name)| aurora_net::inventory::InvFolder {
                            id: Uuid::from_u128(id),
                            parent: Uuid::from_u128(parent),
                            name: name.into(),
                            type_default: -1,
                            ..Default::default()
                        })
                        .collect(),
                );
                for folder in world.inventory.folders.values_mut() {
                    folder.state = crate::world::inventory::FetchState::Fetched;
                }
                let parent = Uuid::from_u128(if source == 0 { 1 } else { 3 });
                world.inventory.add_items(
                    (0..100)
                        .map(|i| aurora_net::inventory::InvItem {
                            id: Uuid::from_u128(100 + i),
                            asset_id: Uuid::from_u128(1000 + i),
                            parent,
                            inv_type: if materials { INV_MATERIAL } else { INV_TEXTURE },
                            asset_type: if materials { 57 } else { 0 },
                            name: format!("Resource {i:03}"),
                            owner_mask: actions::COPY,
                            ..Default::default()
                        })
                        .collect(),
                );
                let mut picker = TexturePicker::default();
                for i in [99, 0] {
                    // Reopening must reveal a row outside the old viewport,
                    // even after the previous session used the Local tab.
                    picker.source = 1;
                    let asset = Uuid::from_u128(1000 + i);
                    if materials {
                        picker.open_materials("Material", asset);
                    } else {
                        picker.open("Texture", asset);
                    }
                    ui.settle(|ctx, p| {
                        assert_eq!(picker.show_live(ctx, p, &mut world, &HashMap::new()), None);
                    });
                    assert_eq!(picker.source, source);
                    assert_eq!(picker.selected_item, Some(Uuid::from_u128(100 + i)));
                    let (row, clip) = ui
                        .ctx
                        .data(|data| data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::unique("picker_selected_row")))
                        .expect("current item must be drawn and selected");
                    assert!(clip.contains(row.center()), "current item must be inside the picker viewport");
                    assert!(picker.take_preview().is_none(), "revealing a value must not edit the face");
                }
            }
        }
    }

    #[test]
    fn clearing_picker_search_keeps_the_clicked_resource_visible() {
        for materials in [false, true] {
            for source in [0, 1, 2] {
                for key in [Some(egui::Key::Delete), Some(egui::Key::Backspace), None] {
                    let mut ui = PickerUi::new();
                    let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                    world.inventory.root = Uuid::from_u128(1);
                    let parent = if source == 2 { 3 } else { 1 };
                    world.inventory.created_folders(
                        [
                            (1, 0, "Inventory"),
                            (2, 1, "#Aurora"),
                            (3, 2, "Aurora Library"),
                            (4, parent, "Resources"),
                            (5, parent, "Unrelated"),
                            (6, 4, "Nested"),
                        ]
                        .into_iter()
                        .map(|(id, parent, name)| aurora_net::inventory::InvFolder {
                            id: Uuid::from_u128(id),
                            parent: Uuid::from_u128(parent),
                            name: name.into(),
                            type_default: -1,
                            ..Default::default()
                        })
                        .collect(),
                    );
                    for folder in world.inventory.folders.values_mut() {
                        folder.state = crate::world::inventory::FetchState::Fetched;
                    }
                    world.inventory.add_items(
                        (0..100)
                            .map(|i| aurora_net::inventory::InvItem {
                                id: Uuid::from_u128(100 + i),
                                asset_id: Uuid::from_u128(1000 + i),
                                parent: Uuid::from_u128(6),
                                inv_type: if materials { INV_MATERIAL } else { INV_TEXTURE },
                                asset_type: if materials { 57 } else { 0 },
                                name: format!("Resource {i:03}"),
                                owner_mask: actions::COPY,
                                ..Default::default()
                            })
                            .collect(),
                    );
                    // A second inventory item for the same asset must not
                    // replace the exact row chosen in the search results.
                    let mut duplicate = world.inventory.items[&Uuid::from_u128(199)].clone();
                    duplicate.id = Uuid::from_u128(10);
                    duplicate.parent = Uuid::from_u128(parent);
                    duplicate.name = "Duplicate asset".into();
                    world.inventory.add_items(vec![duplicate]);
                    if source == 1 {
                        world
                            .local_assets
                            .extend((0..100).map(|i| (Uuid::from_u128(2000 + i), (format!("Resource {i:03}"), materials))));
                    }
                    let mut picker = TexturePicker::default();
                    if materials {
                        picker.open_materials("Material", Uuid::nil());
                    } else {
                        picker.open("Texture", Uuid::nil());
                    }
                    picker.allow_local = true;
                    picker.source = source;
                    picker.local_open[0] = false;
                    let applied = std::cell::RefCell::new(Vec::new());
                    let draw = |picker: &mut TexturePicker, world: &mut World, ctx: &egui::Context, p: &Palette| {
                        if let Some(asset) = picker.show_live(ctx, p, world, &HashMap::new()) {
                            applied.borrow_mut().push(asset);
                        }
                    };
                    ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
                    let (field, rect) = ui.ctx.data(|data| {
                        data.get_temp::<(egui::Id, egui::Rect)>(egui::Id::unique("picker_search_field"))
                            .unwrap()
                    });
                    ui.ctx.memory_mut(|memory| memory.request_focus(field));
                    ui.frame(vec![egui::Event::Text("Resource 099".into())], |ctx, p| {
                        draw(&mut picker, &mut world, ctx, p)
                    });
                    ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
                    assert_eq!(picker.filter, "Resource 099");
                    assert!(!picker.tree.state.is_open(Uuid::from_u128(4), 0));
                    ui.click("Resource 099", |ctx, p| draw(&mut picker, &mut world, ctx, p));
                    ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
                    let selected = Uuid::from_u128(if source == 1 { 2099 } else { 1099 });
                    assert_eq!(picker.selected, Some(selected));
                    assert_eq!(*applied.borrow(), vec![selected]);
                    if let Some(key) = key {
                        ui.ctx.memory_mut(|memory| memory.request_focus(field));
                        ui.frame(
                            vec![
                                egui::Event::Key {
                                    key: egui::Key::A,
                                    physical_key: None,
                                    pressed: true,
                                    repeat: false,
                                    modifiers: egui::Modifiers {
                                        ctrl: true,
                                        command: true,
                                        ..Default::default()
                                    },
                                },
                                egui::Event::Key {
                                    key,
                                    physical_key: None,
                                    pressed: true,
                                    repeat: false,
                                    modifiers: egui::Modifiers::NONE,
                                },
                            ],
                            |ctx, p| draw(&mut picker, &mut world, ctx, p),
                        );
                    } else {
                        assert!(
                            ui.ctx.read_response(field.with("clear")).is_some(),
                            "filled search has a clear button"
                        );
                        ui.labels.insert("clear search".into(), rect.right_center() - Vec2::new(10.0, 0.0));
                        ui.click("clear search", |ctx, p| draw(&mut picker, &mut world, ctx, p));
                    }
                    ui.settle(|ctx, p| draw(&mut picker, &mut world, ctx, p));
                    assert!(picker.filter.is_empty(), "source={source}, key={key:?}");
                    assert!(
                        ui.ctx.read_response(field.with("clear")).is_none(),
                        "empty search has no clear button"
                    );
                    assert_eq!(picker.source, source);
                    assert_eq!(picker.selected, Some(selected));
                    assert_eq!(picker.selected_item, (source != 1).then_some(Uuid::from_u128(199)));
                    let (row, clip) = ui.ctx.data(|data| {
                        data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::unique("picker_selected_row"))
                            .expect("the selected resource must still be drawn")
                    });
                    assert!(
                        clip.contains(row.center()),
                        "source={source}, key={key:?}: {row:?} outside {clip:?}"
                    );
                    assert!(
                        !picker.tree.state.is_open(Uuid::from_u128(5), 0),
                        "unrelated folders stay collapsed"
                    );
                    assert_eq!(
                        *applied.borrow(),
                        vec![selected],
                        "clearing the filter must not apply another resource"
                    );
                }
            }
        }
    }

    #[test]
    fn open_texture_and_material_selectors_follow_live_inventory_sort_in_search_and_aurora() {
        use super::super::inventory::view::{Sort, set_picker_sort};
        for (materials, source) in [false, true]
            .into_iter()
            .flat_map(|materials| [0, 2].map(move |source| (materials, source)))
        {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let root = Uuid::from_u128(1);
            world.inventory.root = root;
            let parent = if source == 0 { root } else { Uuid::from_u128(3) };
            world.inventory.created_folders(
                [(1, 0, "Inventory"), (2, 1, "#Aurora"), (3, 2, "Aurora Library")]
                    .into_iter()
                    .map(|(id, parent, name)| aurora_net::inventory::InvFolder {
                        id: Uuid::from_u128(id),
                        parent: Uuid::from_u128(parent),
                        name: name.into(),
                        type_default: -1,
                        ..Default::default()
                    })
                    .collect(),
            );
            for folder in world.inventory.folders.values_mut() {
                folder.state = crate::world::inventory::FetchState::Fetched;
            }
            world.inventory.add_items(
                [("Texture 10", 200), ("texture 2", 100)]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, date))| aurora_net::inventory::InvItem {
                        id: Uuid::from_u128(10 + i as u128),
                        asset_id: Uuid::from_u128(110 + i as u128),
                        parent,
                        name: name.into(),
                        created_at: date,
                        inv_type: if materials { INV_MATERIAL } else { INV_TEXTURE },
                        ..Default::default()
                    })
                    .collect(),
            );
            let mut picker = TexturePicker::default();
            if materials {
                picker.open_materials("Material", Uuid::nil());
            } else {
                picker.open("Texture", Uuid::nil());
            }
            picker.source = source;
            // Exercise the same picker while open, both filtered and browsing.
            for (i, (by_date, query)) in [(false, ""), (true, ""), (true, "texture"), (false, "texture"), (true, "")]
                .into_iter()
                .enumerate()
            {
                set_picker_sort(
                    &ctx,
                    Sort {
                        by_date,
                        ..Default::default()
                    },
                );
                picker.filter = query.into();
                let mut labels = HashMap::new();
                for pass in 0..3 {
                    labels = frame(&ctx, &mut picker, &mut world, (i * 3 + pass) as f64 / 60.0, vec![]);
                }
                assert_eq!(
                    labels["Texture 10"].y < labels["texture 2"].y,
                    by_date,
                    "materials={materials}, query={query}"
                );
            }
        }
    }

    #[test]
    fn portrait_picker_stays_bounded_with_long_names_empty_inventory_and_material_controls() {
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
            picker.allow_local = true;
            if mode != 0 {
                world
                    .local_assets
                    .insert(Uuid::from_u128(3), ("A very long local file name ".repeat(30), mode == 2));
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
                let rect = ctx.memory(|m| m.area_rect(egui::Id::unique("texture_picker"))).unwrap();
                assert!(rect.width() < 350.0, "mode {mode}, frame {frame}: {rect:?}");
                assert!(rect.height() < 790.0, "mode {mode}, frame {frame}: {rect:?}");
                if frame == 20 {
                    assert!(rect.height() >= 720.0, "portrait default: mode {mode}: {rect:?}");
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

            // A user can still resize the portrait picker and reopen it without
            // having the new default applied over their choice again.
            widgets::size_floater(&ctx, "texture_picker", Vec2::new(400.0, 600.0));
            for n in 180..190 {
                frame(&ctx, &mut picker, &mut world, n as f64 / 60.0, vec![]);
            }
            let resized = ctx.memory(|m| m.area_rect(egui::Id::unique("texture_picker"))).unwrap();
            assert!(resized.width() > initial.unwrap().width());
            assert!(resized.height() < initial.unwrap().height());
            if mode == 2 {
                picker.open_materials("Material", asset);
            } else {
                picker.open_with_defaults("Texture", asset, Some(asset), true);
            }
            for n in 190..200 {
                frame(&ctx, &mut picker, &mut world, n as f64 / 60.0, vec![]);
            }
            let reopened = ctx.memory(|m| m.area_rect(egui::Id::unique("texture_picker"))).unwrap();
            assert!((reopened.width() - resized.width()).abs() < 0.5, "resized width lost: {reopened:?}");
            assert!(
                (reopened.height() - resized.height()).abs() < 0.5,
                "resized height lost: {reopened:?}"
            );
        }
    }
}
