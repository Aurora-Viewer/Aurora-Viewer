//! Item image editor, picker and scene snapshot (LLFloaterChangeItemThumbnail).

use super::*;
use crate::ui::widgets::{Floater, flat_button, search_field};
use std::{collections::HashMap, sync::Arc};

mod choices;
pub(crate) use choices::FileChoices;

pub enum Input {
    File,
    Capture,
    Pixels(Arc<Vec<u8>>),
    Crop { image: Arc<image::RgbaImage>, origin: [u32; 2] },
    Texture { asset: Uuid, resize: bool },
    Paste,
}

#[derive(Default)]
pub struct ThumbnailUi {
    pub item: Option<Uuid>,
    pub pending: Option<Uuid>,
    pub error: String,
    pub saved: bool,
    pub photo_open: bool,
    pub photo: Option<egui::TextureHandle>,
    pub pixels: Option<Arc<Vec<u8>>>,
    pub picker_open: bool,
    selected: Option<Uuid>,
    // Keep the inventory row as well as its asset: several items may share one image.
    selected_item: Option<Uuid>,
    filter: String,
    uuid_text: String,
    choices: Vec<choices::Choice>,
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
        self.saved = false;
        actions.push(InvAction::Thumbnail { request, item, input });
    }
    /// LLFloaterSimpleSnapshot::onSend closes the snapshot once upload starts.
    /// Firestorm indra/newview/llfloatersimplesnapshot.cpp (LGPL 2.1).
    pub(crate) fn save_photo(&mut self, actions: &mut Vec<InvAction>) {
        if let Some(item) = self.item
            && let Some(pixels) = self.pixels.clone()
        {
            self.request(item, Input::Pixels(pixels), actions);
            self.photo_open = false;
        }
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
            self.choices.clear();
            return;
        }
        let asset = image(&world.inventory, item);
        let ready = self.pending.is_none() && !mutation_pending && rules::mutable(&world.inventory, item);
        let position = ctx.content_rect().center() - egui::vec2(360.0, 200.0);
        let mut open = true;
        let mut caption = String::new();
        Floater::new(
            &format!("inventory_image_{ui_id}"),
            aurora_i18n::tr!("ui-inventory-thumbnail-edit-object-picture"),
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
                    "file-upload-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-upload-from-your-computer"),
                    ready,
                    &mut caption,
                ) {
                    self.request(item, Input::File, actions);
                }
                if toolbar_button(
                    ui,
                    p,
                    "camera-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-take-snapshot"),
                    ready,
                    &mut caption,
                ) {
                    self.photo_open = true;
                    self.request(item, Input::Capture, actions);
                }
                if toolbar_button(
                    ui,
                    p,
                    "photo-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-choose-image"),
                    ready,
                    &mut caption,
                ) {
                    self.picker_open = true;
                    self.selected = (!asset.is_nil()).then_some(asset);
                    self.selected_item = world
                        .inventory
                        .items
                        .values()
                        .filter(|it| it.asset_id == asset && usable(&world.inventory, it))
                        .map(|it| it.id)
                        .min();
                    // the picker's UUID field, as LLFloaterTexturePicker
                    // writes it: zeros unless the asset id rule allows it
                    let shown = crate::world::inventory::asset_ids::AssetIds::of(world).shown(&asset);
                    self.uuid_text = if shown { asset } else { Uuid::nil() }.to_string();
                    self.filter.clear();
                }
                if toolbar_button(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-copy-to-clipboard"),
                    !asset.is_nil(),
                    &mut caption,
                ) {
                    let clipboard = crate::world::inventory::asset_ids::AssetIds::of(world).shown(&asset);
                    if clipboard {
                        ctx.copy_text(asset.to_string());
                    }
                    actions.push(InvAction::ThumbnailCopied { asset, clipboard });
                }
                if toolbar_button(
                    ui,
                    p,
                    "clipboard-text-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-paste-from-clipboard"),
                    ready,
                    &mut caption,
                ) {
                    self.request(item, Input::Paste, actions);
                }
                if toolbar_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("ui-inventory-thumbnail-remove-image"),
                    ready && !asset.is_nil(),
                    &mut caption,
                ) {
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
                ui.label(RichText::new(aurora_i18n::tr!("ui-inventory-thumbnail-processing-image")).color(p.muted));
            } else if self.saved && self.error.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("inventory-thumbnail-upload-complete")).color(p.success));
            }
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(p.warn));
            }
        });
        if !open {
            self.item = None;
            self.choices.clear();
            self.picker_open = false;
            self.photo_open = false;
            return;
        }
        self.show_file_choices(ctx, p, ui_id, &rules::name(&world.inventory, item), ready, actions);
        if self.photo_open {
            let mut photo_open = true;
            let mut cancel = false;
            Floater::new(
                &format!("inventory_photo_{ui_id}"),
                aurora_i18n::tr!("ui-inventory-thumbnail-object-picture"),
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
                            aurora_i18n::tr!("ui-inventory-thumbnail-taking-snapshot"),
                            egui::FontId::proportional(13.0),
                            p.muted,
                        );
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(ready, egui::Button::new(aurora_i18n::tr!("ui-inventory-thumbnail-take-snapshot")))
                        .clicked()
                    {
                        self.request(item, Input::Capture, actions);
                    }
                    if ui
                        .add_enabled(ready && self.pixels.is_some(), egui::Button::new(aurora_i18n::tr!("common-save")))
                        .clicked()
                    {
                        self.save_photo(actions);
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
            self.photo_open &= photo_open && !cancel;
        }
        if self.picker_open {
            view::set_picker_sort(ctx, prefs.sort);
            let order = view::picker_order(ctx, &world.inventory);
            let mut picker_open = true;
            let mut chosen = None;
            let mut cancel = false;
            Floater::new(
                &format!("inventory_image_picker_{ui_id}"),
                aurora_i18n::tr!("ui-inventory-thumbnail-choose-object-picture"),
                position + egui::vec2(312.0, 0.0),
                Vec2::new(450.0, 392.0),
            )
            .fixed()
            .show(ctx, p, &mut picker_open, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        picture(ui, p, self.selected.unwrap_or_default(), 164.0, images, wanted);
                        ui.label(aurora_i18n::tr!("common-inventory"));
                        ui.label(aurora_i18n::tr!("ui-inventory-thumbnail-size-64-to-256-pixels"));
                        for (label, id) in [
                            (aurora_i18n::tr!("common-blank"), aurora_prim::te::BLANK_TEXTURE),
                            (aurora_i18n::tr!("common-transparent"), aurora_prim::te::TRANSPARENT_TEXTURE),
                        ] {
                            if flat_button(ui, p, label).clicked() {
                                self.selected = Some(id);
                                self.selected_item = None;
                                self.uuid_text = id.to_string();
                            }
                        }
                    });
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        let filter = search_field(ui, p, &mut self.filter, aurora_i18n::tr!("common-filter-textures"), 250.0);
                        #[cfg(test)]
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(egui::Id::unique("thumbnail_search"), (filter.id, filter.rect)));
                        let _ = filter;
                        let reveal: HashSet<_> = if widgets::search_cleared(ui) {
                            self.selected_item
                                .map(|id| rules::ancestors(&world.inventory, id).collect())
                                .unwrap_or_default()
                        } else {
                            HashSet::new()
                        };
                        egui::ScrollArea::both()
                            .id_salt("thumbnail_tree")
                            .max_height(255.0)
                            .min_scrolled_height(255.0)
                            .show(ui, |ui| {
                                let query = self.filter.trim().to_lowercase();
                                if query.is_empty() {
                                    let root = world.inventory.root;
                                    picker_folder(
                                        ui,
                                        p,
                                        icons,
                                        &mut world.inventory,
                                        &order,
                                        root,
                                        0,
                                        &mut self.selected,
                                        &mut self.selected_item,
                                        &reveal,
                                        prefs,
                                    );
                                } else {
                                    let mut items: Vec<_> = world
                                        .inventory
                                        .items
                                        .values()
                                        .filter(|it| usable(&world.inventory, it) && it.name.to_lowercase().contains(&query))
                                        .collect();
                                    items.sort_by_key(|it| order.rank(it.id));
                                    for it in items.into_iter().take(500) {
                                        ui.horizontal(|ui| {
                                            resolved_item_type_icon(ui, icons, &world.inventory, it, p);
                                            picker_texture(ui, it, &mut self.selected, &mut self.selected_item);
                                        });
                                    }
                                }
                            });
                    });
                });
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.uuid_text)
                            .hint_text(aurora_i18n::tr!("ui-inventory-thumbnail-image-uuid"))
                            .desired_width(320.0),
                    );
                    if flat_button(ui, p, aurora_i18n::tr!("common-apply-uuid")).clicked() {
                        match Uuid::parse_str(self.uuid_text.trim()) {
                            Ok(id) if !id.is_nil() => {
                                self.selected = Some(id);
                                self.selected_item = None;
                            }
                            _ => self.error = aurora_i18n::tr!("ui-inventory-thumbnail-invalid-image-uuid").into(),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(ready && self.selected.is_some(), egui::Button::new(aurora_i18n::tr!("common-ok")))
                        .clicked()
                    {
                        chosen = self.selected;
                    }
                    if flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
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
    match inv.items.get(&item) {
        Some(it) => resolved_item_type_icon(ui, icons, inv, it, p),
        None => icon(ui, icons, "Inv_FolderClosed", p),
    }
    ui.label(RichText::new(rules::name(inv, item)).color(p.ink));
}
fn paint_image(ui: &egui::Ui, rect: egui::Rect, texture: &egui::TextureHandle) {
    ui.painter().image(
        texture.id(),
        rect,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        crate::ui::widgets::PICTURE_TINT,
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
                aurora_i18n::tr!("common-loading-2"),
                egui::FontId::proportional(12.0),
                p.muted,
            );
        }
    } else if let Some(camera) = crate::ui::icons::global("camera-filled") {
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
        && (rules::library(inv, it.id) || rules::held(inv, it) & (rules::COPY | rules::TRANSFER) == rules::COPY | rules::TRANSFER)
}
/// A texture id that may become an item image (pasted, typed or picked):
/// a built-in or local texture, any id in demo mode, or the asset of a
/// texture in the inventory that allows copying and transfer (`usable`).
pub(crate) fn image_source_allowed(inv: &Inventory, asset: &Uuid, local: bool, demo: bool) -> bool {
    !asset.is_nil()
        && (crate::world::inventory::asset_ids::built_in(asset)
            || demo
            || local
            || inv.items.values().any(|it| it.asset_id == *asset && usable(inv, it)))
}
#[allow(clippy::too_many_arguments)]
fn picker_folder(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    order: &view::PickerOrder,
    id: Uuid,
    depth: usize,
    selected: &mut Option<Uuid>,
    selected_item: &mut Option<Uuid>,
    reveal: &HashSet<Uuid>,
    prefs: &InventoryPreferences,
) {
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
        aurora_i18n::tr!("common-my-inventory").to_owned()
    } else if id == inv.lib_root {
        aurora_i18n::tr!("common-library").to_owned()
    } else {
        folder.info.name.clone()
    };
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        ui.make_persistent_id(("thumbnail_folder", id)),
        depth == 0,
    );
    if reveal.contains(&id) {
        state.set_open(true);
        ui.ctx().animate_bool_with_time(state.id(), true, 0.0);
    }
    let folder_open = state.is_open();
    state
        .show_header(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let tint = super::folder_colors::tint(prefs, id);
                tree::icon(ui, icons, folder_icon(folder.info.type_default, folder_open), p, tint);
                ui.label(RichText::new(name).color(p.ink));
            });
        })
        .body(|ui| {
            inv.request(id);
            for &item in order.children(id) {
                if inv.folders.contains_key(&item) {
                    picker_folder(ui, p, icons, inv, order, item, depth + 1, selected, selected_item, reveal, prefs);
                } else if let Some(it) = inv.items.get(&item)
                    && usable(inv, it)
                {
                    ui.horizontal(|ui| {
                        resolved_item_type_icon(ui, icons, inv, it, p);
                        picker_texture(ui, it, selected, selected_item);
                    });
                }
            }
        });
}

fn picker_texture(ui: &mut egui::Ui, it: &aurora_net::inventory::InvItem, selected: &mut Option<Uuid>, selected_item: &mut Option<Uuid>) {
    let on = selected_item.map_or(*selected == Some(it.asset_id), |id| id == it.id);
    let response = ui.selectable_label(on, &it.name);
    widgets::reveal_search_selection(ui, &response, *selected_item == Some(it.id));
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("thumbnail_texture", it.id)), (response.rect, ui.clip_rect())));
    if response.clicked() {
        *selected = Some(it.asset_id);
        *selected_item = Some(it.id);
    }
}

fn toolbar_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, enabled: bool, caption: &mut String) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    ui.painter().rect_filled(
        rect,
        ui.visuals().window_corner_radius,
        if enabled && response.hovered() { p.raised } else { p.field },
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_thumbnail_search_reopens_the_chosen_items_path_and_reveals_the_exact_duplicate() {
        for method in 0..3 {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
            let root = Uuid::from_u128(1);
            let target = Uuid::from_u128(500);
            let asset = Uuid::from_u128(1001);
            world.inventory.root = root;
            world.inventory.created_folders(
                (1..=3)
                    .map(|id| aurora_net::inventory::InvFolder {
                        id: Uuid::from_u128(id),
                        parent: Uuid::from_u128(id - 1),
                        name: format!("Folder {id}"),
                        type_default: -1,
                        ..Default::default()
                    })
                    .collect(),
            );
            for folder in world.inventory.folders.values_mut() {
                folder.state = FetchState::Fetched;
            }
            world.inventory.add_items(
                (0..101)
                    .map(|i| aurora_net::inventory::InvItem {
                        id: if i == 100 { target } else { Uuid::from_u128(100 + i) },
                        parent: Uuid::from_u128(3),
                        asset_id: if i == 100 { asset } else { Uuid::from_u128(1000 + i) },
                        name: if i == 100 { "Needle".into() } else { format!("A texture {i:03}") },
                        asset_type: 0,
                        inv_type: 0,
                        owner_mask: rules::COPY | rules::TRANSFER,
                        ..Default::default()
                    })
                    .collect(),
            );
            let mut picker = ThumbnailUi {
                item: Some(root),
                picker_open: true,
                filter: "Needle".into(),
                ..Default::default()
            };
            let row_key = egui::Id::unique(("thumbnail_texture", target));
            let mut frame = |picker: &mut ThumbnailUi, events| {
                ctx.data_mut(|data| data.remove::<(egui::Rect, egui::Rect)>(row_key));
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                        events,
                        ..Default::default()
                    },
                    |_| {
                        picker.show(
                            &ctx,
                            &theme.palette(),
                            &Icons::default(),
                            &mut world,
                            &InventoryPreferences::default(),
                            Uuid::nil(),
                            &HashMap::new(),
                            &mut HashSet::new(),
                            false,
                            &mut Vec::new(),
                        );
                    },
                );
                output.textures_delta.clear();
            };
            for _ in 0..4 {
                frame(&mut picker, Vec::new());
            }
            let pos = ctx
                .data(|data| data.get_temp::<(egui::Rect, egui::Rect)>(row_key))
                .expect("filtered texture")
                .0
                .center();
            let click = |pos, pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            for pressed in [true, false] {
                frame(&mut picker, click(pos, pressed));
            }
            assert_eq!(picker.selected_item, Some(target));
            assert_eq!(picker.selected, Some(asset));
            let (field, rect) = ctx
                .data(|data| data.get_temp::<(egui::Id, egui::Rect)>(egui::Id::unique("thumbnail_search")))
                .expect("search field");
            let pos = if method == 0 {
                ctx.read_response(field.with("clear")).expect("clear action").rect.center()
            } else {
                rect.center()
            };
            for pressed in [true, false] {
                frame(&mut picker, click(pos, pressed));
            }
            if method != 0 {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                };
                frame(
                    &mut picker,
                    vec![
                        egui::Event::Key {
                            key: egui::Key::A,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers,
                        },
                        egui::Event::Key {
                            key: if method == 1 { egui::Key::Delete } else { egui::Key::Backspace },
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(picker.filter.is_empty());
            for _ in 0..5 {
                frame(&mut picker, Vec::new());
            }
            assert_eq!(picker.selected_item, Some(target), "another item shares this asset");
            assert_eq!(picker.selected, Some(asset));
            let (rect, clip) = ctx
                .data(|data| data.get_temp::<(egui::Rect, egui::Rect)>(row_key))
                .expect("the closed ancestors were reopened");
            assert!(clip.contains(rect.center()), "method={method}: {rect:?} outside {clip:?}");
            assert!(ctx.memory(|memory| memory.has_focus(field)));
        }
    }

    #[test]
    fn an_item_image_comes_from_a_copy_and_transfer_texture_built_in_local_or_demo() {
        let mut inv = Inventory::default();
        let root = Uuid::from_u128(1);
        inv.root = root;
        let (open, no_transfer, unknown) = (Uuid::from_u128(0x61), Uuid::from_u128(0x62), Uuid::from_u128(0x63));
        inv.add_items(vec![
            aurora_net::inventory::InvItem {
                id: Uuid::from_u128(0x71),
                parent: root,
                asset_id: open,
                asset_type: 0,
                owner_mask: rules::COPY | rules::TRANSFER,
                ..Default::default()
            },
            aurora_net::inventory::InvItem {
                id: Uuid::from_u128(0x72),
                parent: root,
                asset_id: no_transfer,
                asset_type: 0,
                owner_mask: rules::COPY,
                ..Default::default()
            },
        ]);
        assert!(image_source_allowed(&inv, &open, false, false));
        assert!(!image_source_allowed(&inv, &no_transfer, false, false), "copy without transfer");
        assert!(!image_source_allowed(&inv, &unknown, false, false), "not in the inventory");
        assert!(image_source_allowed(&inv, &unknown, true, false), "a local texture");
        assert!(image_source_allowed(&inv, &unknown, false, true), "demo mode");
        assert!(!image_source_allowed(&inv, &Uuid::nil(), false, true));
    }

    #[test]
    fn open_thumbnail_picker_tracks_inventory_order_in_tree_and_search() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let root = Uuid::from_u128(1);
        world.inventory.root = root;
        world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
            id: root,
            name: "Inventory".into(),
            type_default: -1,
            ..Default::default()
        }]);
        world.inventory.folders.get_mut(&root).expect("root").state = FetchState::Fetched;
        world.inventory.add_items(
            [("Texture 10", 200), ("texture 2", 100)]
                .into_iter()
                .enumerate()
                .map(|(i, (name, date))| aurora_net::inventory::InvItem {
                    id: Uuid::from_u128(10 + i as u128),
                    parent: root,
                    asset_id: Uuid::from_u128(110 + i as u128),
                    name: name.into(),
                    created_at: date,
                    asset_type: 0,
                    inv_type: 0,
                    owner_mask: rules::COPY | rules::TRANSFER,
                    ..Default::default()
                })
                .collect(),
        );
        let mut picker = ThumbnailUi {
            item: Some(root),
            picker_open: true,
            ..Default::default()
        };
        for (by_date, query) in [(false, ""), (true, ""), (false, "texture"), (true, "texture")] {
            let prefs = InventoryPreferences {
                sort: view::Sort {
                    by_date,
                    ..Default::default()
                },
                ..Default::default()
            };
            picker.filter = query.into();
            let mut names = HashMap::new();
            for _ in 0..3 {
                let output = headless_output(ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                        ..Default::default()
                    },
                    |_| {
                        picker.show(
                            &ctx,
                            &crate::theme::Theme::default().palette(),
                            &Icons::default(),
                            &mut world,
                            &prefs,
                            Uuid::nil(),
                            &HashMap::new(),
                            &mut HashSet::new(),
                            false,
                            &mut Vec::new(),
                        )
                    },
                ));
                for shape in output.shapes {
                    if let egui::Shape::Text(text) = shape.shape {
                        names.insert(text.galley.text().to_owned(), text.pos.y);
                    }
                }
            }
            assert_eq!(names["Texture 10"] < names["texture 2"], by_date, "query={query}");
        }
    }

    #[test]
    fn clicking_save_uploads_the_photo_and_closes_only_the_snapshot_window() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let p = crate::theme::Theme::default().palette();
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        crate::world::inventory::demo::seed(&mut world.inventory, Uuid::from_u128(2));
        let item = Uuid::from_u128(8101);
        let pixels = Arc::new(vec![128; 256 * 256 * 4]);
        let mut st = ThumbnailUi {
            item: Some(item),
            photo_open: true,
            pixels: Some(pixels.clone()),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let mut frame = |events| {
            headless_output(ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                    events,
                    ..Default::default()
                },
                |_ui| {
                    st.show(
                        &ctx,
                        &p,
                        &Icons::default(),
                        &mut world,
                        &InventoryPreferences::default(),
                        Uuid::nil(),
                        &HashMap::new(),
                        &mut HashSet::new(),
                        false,
                        &mut actions,
                    );
                },
            ))
        };
        for _ in 0..4 {
            frame(Vec::new());
        }
        let output = frame(Vec::new());
        let pos = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("common-save") => Some(t.pos + egui::vec2(4.0, 4.0)),
                _ => None,
            })
            .expect("save button");
        for pressed in [true, false] {
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let output = frame(Vec::new());
        assert!(
            matches!(&actions[..], [InvAction::Thumbnail { item: target, input: Input::Pixels(photo), .. }]
            if *target == item && Arc::ptr_eq(photo, &pixels))
        );
        assert!(!st.photo_open);
        assert_eq!(st.item, Some(item));
        assert!(st.pending.is_some());
        assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t)
            if t.galley.text() == aurora_i18n::tr!("ui-inventory-thumbnail-edit-object-picture"))));
        assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::epaint::Shape::Text(t)
            if t.galley.text() == aurora_i18n::tr!("ui-inventory-thumbnail-object-picture"))));
    }
}
