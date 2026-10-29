//! Inventory picture tooltips: LLInventoryPanel::handleToolTip,
//! LLViewerInventoryItem::getThumbnailUUID and LLInspectTextureUtil::createInventoryToolTip
//! (indra/newview, originally LGPL 2.1). Object pictures are metadata;
//! inspecting an object's contents would require rezzing it.

use super::*;
use std::collections::HashMap;
use std::time::Duration;

const DELAY: f64 = 0.7;
const SIZE: f32 = 256.0;

#[derive(Default)]
pub(super) struct State {
    row: Option<(egui::Response, Uuid, Option<permissions::Permissions>)>,
    since: Option<(egui::Id, f64)>,
}

impl State {
    pub fn record(&mut self, response: &egui::Response, id: Uuid, permissions: Option<permissions::Permissions>) {
        if response.hovered() && response.ctx.input(|i| i.pointer.has_pointer()) {
            self.row = Some((response.clone(), id, permissions));
        }
        demo_row(response, id);
    }

    /// Finish in the owning window, where the shared asynchronous image cache
    /// is available. No image handles or responses survive this frame.
    pub fn show(
        &mut self,
        p: &Palette,
        inv: &Inventory,
        prefs: &InventoryPreferences,
        images: &HashMap<Uuid, egui::TextureHandle>,
        wanted: &mut HashSet<Uuid>,
    ) {
        let Some((response, id, permissions)) = self.row.take() else {
            self.since = None;
            return;
        };
        if response.ctx.is_being_dragged(response.id) || response.ctx.input(|i| i.pointer.any_down()) {
            self.since = None;
            return;
        }
        let now = response.ctx.input(|i| i.time);
        let since = self.since.get_or_insert((response.id, now));
        if since.0 != response.id {
            *since = (response.id, now);
        }
        let picture = prefs.thumbnail_tooltips.then(|| asset(inv, id)).flatten();
        if picture.is_some() && now - since.1 < DELAY {
            response
                .ctx
                .request_repaint_after(Duration::from_secs_f64((DELAY - (now - since.1)).max(0.0)));
            return;
        }
        response.on_hover_ui(|ui| {
            if let Some(asset) = picture {
                wanted.insert(asset);
                if let Some(texture) = images.get(&asset) {
                    ui.add(
                        egui::Image::new(texture)
                            .fit_to_exact_size(Vec2::splat(SIZE))
                            .maintain_aspect_ratio(true),
                    );
                } else {
                    ui.add_sized(Vec2::splat(SIZE), egui::Label::new(aurora_i18n::tr!("common-loading-2")));
                }
            }
            if let Some(it) = inv.items.get(&id) {
                ui.label(&it.name);
                ui.label(inventory_path(inv, it.parent));
                ui.label(if it.created_at > 0 {
                    aurora_net::inventory::created_date(it.created_at)
                } else {
                    aurora_i18n::tr!("ui-inventory-unknown-date").into()
                });
                if !it.desc.is_empty() {
                    ui.label(&it.desc);
                }
                if let Some(permissions) = permissions {
                    crate::ui::widgets::divider(ui, p);
                    ui.label(permissions.tooltip());
                }
            } else if let Some(folder) = inv.folders.get(&id) {
                ui.label(folder_name(inv, id));
                ui.label(inventory_path(inv, folder.info.parent));
            }
        });
    }
}

fn asset(inv: &Inventory, id: Uuid) -> Option<Uuid> {
    if !inv.can_preview(0) {
        return None;
    }
    let thumbnail = thumbnail::image(inv, id);
    if !thumbnail.is_nil() {
        return Some(thumbnail);
    }
    let original = rules::original(inv, id)?;
    let thumbnail = thumbnail::image(inv, original);
    if !thumbnail.is_nil() {
        return Some(thumbnail);
    }
    if let Some(item) = inv.items.get(&original) {
        return (item.asset_type == 0 && !item.asset_id.is_nil()).then_some(item.asset_id);
    }
    let folder = inv.folders.get(&original)?;
    if folder.info.type_default != 47 || folder.state != FetchState::Fetched {
        return None;
    }
    // Legacy outfit pictures: exactly one direct texture, including links.
    // Never walk or fetch a folder tree just because its row is hovered.
    let mut textures = folder.items.iter().filter_map(|id| {
        let item = inv.items.get(&rules::original(inv, *id)?)?;
        (item.asset_type == 0).then_some(item.asset_id)
    });
    let texture = textures.next()?;
    (!texture.is_nil() && textures.next().is_none()).then_some(texture)
}

fn mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_INVENTORY")
            .ok()
            .filter(|mode| mode.contains("hover-photo"))
    })
    .as_deref()
}

fn demo_row(response: &egui::Response, id: Uuid) {
    if mode().is_some() {
        response
            .ctx
            .data_mut(|d| d.insert_temp(egui::Id::unique(("inventory_hover_demo", id)), response.rect.center()));
    }
}

pub fn demo_events(ctx: &egui::Context, frame: u64, inv: &Inventory, st: &mut InventoryUi) -> Vec<egui::Event> {
    let Some(mode) = mode() else { return Vec::new() };
    let item = Uuid::from_u128(if mode.contains("object") { 8100 } else { 8104 });
    if frame == 2 {
        st.reveal_item(inv, item);
    }
    // Repeating PointerMoved, even at the same point, restarts egui's
    // stationary-pointer tooltip timer. Move once, then leave it still.
    if frame != 600 {
        return Vec::new();
    }
    ctx.data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("inventory_hover_demo", item))))
        .map(|pos| vec![egui::Event::PointerMoved(pos)])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::Frames;
    use aurora_net::inventory::InvItem;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn inventory_hover_picture_resolves_photos_metadata_links_and_outfits() {
        let mut inv = super::super::tests::view_inventory(false);
        let photo = id(8104);
        let texture = inv.items[&photo].asset_id;
        inv.items.get_mut(&photo).unwrap().inv_type = 15;
        assert_eq!(asset(&inv, photo), Some(texture));
        assert_eq!(asset(&inv, id(8100)), None, "object asset IDs are never texture IDs");
        inv.items.get_mut(&id(8100)).unwrap().thumbnail = id(200);
        assert_eq!(asset(&inv, id(8113)), Some(id(200)), "object link uses its target's picture");
        inv.items.get_mut(&id(8113)).unwrap().thumbnail = id(201);
        assert_eq!(asset(&inv, id(8113)), Some(id(201)), "explicit link picture wins");
        inv.items.get_mut(&photo).unwrap().thumbnail = id(202);
        assert_eq!(asset(&inv, photo), Some(id(202)), "explicit picture wins over the photo asset");
        inv.add_items(vec![InvItem {
            id: id(300),
            asset_type: 24,
            asset_id: photo,
            ..Default::default()
        }]);
        let folder = inv.folders.get_mut(&id(8000)).unwrap();
        folder.info.type_default = 47;
        folder.items = vec![id(300), id(8100)];
        assert_eq!(asset(&inv, id(8000)), Some(texture), "one direct texture through a link");
        inv.folders.get_mut(&id(8000)).unwrap().items.push(photo);
        assert_eq!(asset(&inv, id(8000)), None, "two texture items are ambiguous");
        inv.folders.get_mut(&id(8000)).unwrap().info.thumbnail = id(203);
        assert_eq!(asset(&inv, id(8000)), Some(id(203)));
        inv.add_items(vec![InvItem {
            id: id(301),
            asset_type: 25,
            asset_id: id(8000),
            ..Default::default()
        }]);
        assert_eq!(asset(&inv, id(301)), Some(id(203)));
        inv.items.get_mut(&id(300)).unwrap().asset_id = id(300);
        assert_eq!(asset(&inv, id(300)), None, "cyclic link");
        inv.items.get_mut(&id(300)).unwrap().asset_id = id(999);
        assert_eq!(asset(&inv, id(300)), None, "broken link");
        inv.rlva_preview_mask = 1;
        for item in [photo, id(8100), id(8113), id(8000), id(301)] {
            assert_eq!(asset(&inv, item), None, "viewtexture also covers thumbnails");
        }
        assert!(inv.queue.is_empty(), "hover never fetches inventory contents");
    }

    #[test]
    fn inventory_hover_picture_is_delayed_and_keeps_the_image_aspect_and_text() {
        let mut frames = Frames::new();
        let ctx = frames.ctx.clone();
        let texture = ctx.load_texture(
            "hover landscape",
            egui::ColorImage::filled([64, 32], egui::Color32::WHITE),
            Default::default(),
        );
        let mut inv = super::super::tests::view_inventory(false);
        let item = id(8104);
        inv.items.get_mut(&item).unwrap().desc = "Photo description".into();
        let picture = inv.items[&item].asset_id;
        let images = HashMap::from([(picture, texture.clone())]);
        let prefs = InventoryPreferences::default();
        let p = crate::theme::Theme::default().palette();
        let mut state = State::default();
        let mut wanted = HashSet::new();
        let pos = egui::pos2(40.0, 20.0);
        let mut draw = |ui: &mut egui::Ui| {
            let response = ui.add_sized([180.0, 24.0], egui::Label::new("Photo row"));
            state.record(&response, item, None);
            state.show(&p, &inv, &prefs, &images, &mut wanted);
        };
        frames.run(vec![], &mut draw);
        frames.run(vec![egui::Event::PointerMoved(pos)], &mut draw);
        for _ in 0..20 {
            frames.run(vec![], &mut draw);
        }
        assert!(wanted.is_empty());
        let texts = frames.hover(pos, |ui| {
            let response = ui.add_sized([180.0, 24.0], egui::Label::new("Photo row"));
            state.record(&response, item, None);
            state.show(&p, &inv, &prefs, &images, &mut wanted);
        });
        assert_eq!(wanted, HashSet::from([picture]));
        assert!(texts.iter().any(|text| text.source == "Photo description"));
        let shapes = frames.shapes(vec![], |ui| {
            let response = ui.add_sized([180.0, 24.0], egui::Label::new("Photo row"));
            state.record(&response, item, None);
            state.show(&p, &inv, &prefs, &images, &mut wanted);
        });
        let rect = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Rect(rect) if rect.fill_texture_id() == texture.id() => Some(rect.rect),
                _ => None,
            })
            .expect("photo actually painted in tooltip");
        assert!((rect.width() - 256.0).abs() < 1.0);
        assert!((rect.height() - 128.0).abs() < 1.0);
        wanted.clear();
        frames.run(vec![egui::Event::PointerGone], |ui| {
            let response = ui.add_sized([180.0, 24.0], egui::Label::new("Photo row"));
            state.record(&response, item, None);
            state.show(&p, &inv, &prefs, &images, &mut wanted);
        });
        assert!(wanted.is_empty());
        assert!(state.since.is_none(), "leaving the row resets its hover delay");
    }

    #[test]
    fn inventory_hover_picture_disabled_or_restricted_preserves_text_without_requests() {
        for restricted in [false, true] {
            let mut frames = Frames::new();
            let mut inv = super::super::tests::view_inventory(false);
            inv.rlva_preview_mask = u8::from(restricted);
            let item = id(8104);
            inv.items.get_mut(&item).unwrap().desc = "Photo description".into();
            let prefs = InventoryPreferences {
                thumbnail_tooltips: restricted,
                ..Default::default()
            };
            let p = crate::theme::Theme::default().palette();
            let mut state = State::default();
            let mut wanted = HashSet::new();
            let texts = frames.hover(egui::pos2(40.0, 20.0), |ui| {
                let response = ui.add_sized([180.0, 24.0], egui::Label::new("Photo row"));
                state.record(&response, item, None);
                state.show(&p, &inv, &prefs, &Default::default(), &mut wanted);
            });
            assert!(wanted.is_empty());
            assert!(texts.iter().any(|text| text.source == "Photo description"));
        }
    }

    #[test]
    fn inventory_hover_picture_uses_both_inventory_presentations_without_selecting() {
        for width in [0.0, 440.0, 800.0, 1180.0] {
            let mut frames = Frames::new();
            let ctx = frames.ctx.clone();
            let mut inv = super::super::tests::view_inventory(false);
            let item = id(8104);
            let mut st = InventoryUi {
                search: inv.items[&item].name.clone(),
                ..Default::default()
            };
            let mut prefs = InventoryPreferences {
                show_details: false,
                ..Default::default()
            };
            let facts = Facts::default();
            let p = crate::theme::Theme::default().palette();
            let mut actions = Vec::new();
            let mut draw = |ui: &mut egui::Ui| {
                st.visible.clear();
                ui.set_width(if width == 0.0 { 440.0 } else { width });
                if width == 0.0 {
                    inventory_contents(ui, &p, &Icons::default(), &mut inv, &mut st, &mut prefs, &facts, &mut actions);
                } else {
                    aurora::contents(
                        ui,
                        &p,
                        &Icons::default(),
                        &mut inv,
                        &mut st,
                        &mut prefs,
                        &facts,
                        &Default::default(),
                        &mut actions,
                    );
                }
                st.hover.show(&p, &inv, &prefs, &Default::default(), &mut st.wanted_hover_images);
            };
            for _ in 0..4 {
                frames.run(vec![], &mut draw);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("inventory_drag_row", Uuid::nil(), item))))
                .expect("actual photo row visible");
            frames.hover(rect.center(), &mut draw);
            assert!(st.wanted_hover_images.contains(&inv.items[&item].asset_id), "width {width}");
            assert!(st.wanted_images.is_empty(), "hover does not populate the persistent UI cache");
            assert!(st.selection.is_empty());
            assert!(actions.is_empty());
        }
    }
}
