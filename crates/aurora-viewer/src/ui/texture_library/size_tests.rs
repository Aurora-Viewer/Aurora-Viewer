//! Real resize gestures and persistence across independent resource windows.
use super::*;
use std::sync::Arc;

struct Fixture {
    ctx: egui::Context,
    world: World,
    scene: Scene,
    images: HashMap<Uuid, egui::TextureHandle>,
    resources: LibraryUi,
    prefs: Preferences,
    screen: Vec2,
    time: f64,
}
impl Fixture {
    fn new(avatars: Arc<crate::scene::avatar::AvatarLibrary>, prefs: Preferences) -> Self {
        let ctx = egui::Context::default();
        let mut world = World::new(avatars.clone());
        world.agent_id = Uuid::from_u128(70);
        let texture = Uuid::from_u128(120);
        let material = Uuid::from_u128(121);
        world.inventory.add_items(
            (110..117)
                .map(|n| aurora_net::inventory::InvItem {
                    id: Uuid::from_u128(n),
                    asset_id: if matches!(n, 113 | 114 | 116) { material } else { texture },
                    asset_type: if matches!(n, 113 | 114 | 116) { 57 } else { 0 },
                    name: format!("Preview {n}"),
                    ..Default::default()
                })
                .collect(),
        );
        let mut scene = Scene::new(std::env::temp_dir(), avatars);
        scene.materials.insert(material, aurora_assets::PbrMaterial::default());
        let image = ctx.load_texture(
            "preview size regression",
            egui::ColorImage::filled([128, 64], egui::Color32::GRAY),
            Default::default(),
        );
        Self {
            ctx,
            world,
            scene,
            images: HashMap::from([(texture, image)]),
            resources: LibraryUi::default(),
            prefs,
            screen: Vec2::new(1600.0, 1200.0),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |host| {
                self.resources.show(
                    host.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut self.world,
                    &mut self.scene,
                    &self.images,
                    &mut self.prefs,
                    &mut false,
                );
            },
        );
        output.textures_delta.clear();
    }
    fn settle(&mut self) {
        for _ in 0..8 {
            self.frame(Vec::new());
        }
    }
    fn rect(&self, item: u128) -> egui::Rect {
        self.ctx
            .memory(|m| m.area_rect(egui::Id::unique(preview_window_id(Uuid::from_u128(item)))))
            .expect("preview window")
    }
    fn open(&mut self, item: u128) {
        self.resources.open_preview(&self.ctx, &self.world, Uuid::from_u128(item));
        self.settle();
    }
    fn front(&mut self, item: u128) {
        let id = preview_window_id(Uuid::from_u128(item));
        widgets::move_floater(&self.ctx, &id, egui::pos2(20.0, 60.0));
        self.ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)));
        self.settle();
    }
    fn button(&mut self, pos: egui::Pos2, pressed: bool) {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            },
        ]);
    }
    fn resize(&mut self, item: u128, delta: Vec2) -> Vec2 {
        self.front(item);
        let before = self.prefs.clone();
        let corner = self.rect(item).right_bottom() - Vec2::splat(2.0);
        self.frame(vec![egui::Event::PointerMoved(corner)]);
        self.button(corner, true);
        let target = corner + delta;
        for _ in 0..4 {
            self.frame(vec![egui::Event::PointerMoved(target)]);
        }
        assert_eq!(self.prefs, before, "do not save settings on every drag frame");
        self.button(target, false);
        self.settle();
        self.rect(item).size()
    }
    fn close(&mut self, item: u128) {
        self.front(item);
        let rect = self.rect(item);
        let close = egui::pos2(rect.right() - 18.0, rect.top() + 19.0);
        self.button(close, true);
        self.button(close, false);
        self.settle();
        assert!(self.resources.windows.iter().all(|w| w.id != Uuid::from_u128(item)));
    }
}
fn same_size(actual: Vec2, expected: Vec2) {
    assert!((actual - expected).length() < 1.0, "{actual:?} != {expected:?}");
}

#[test]
fn cascade_resource_previews_use_creation_order_and_separate_type_groups() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.open(110);
    f.open(111);
    same_size(f.rect(111).min - f.rect(110).min, Vec2::splat(32.0));
    let last = f.rect(111);
    f.front(110);
    f.open(110); // Raising an older item must not become the cascade anchor.
    f.open(112);
    same_size(f.rect(112).min - last.min, Vec2::splat(32.0));
    f.open(113);
    f.open(114);
    same_size(f.rect(114).min - f.rect(113).min, Vec2::splat(32.0));
    let material = f.rect(114);
    widgets::minimize_floater(&f.ctx, &preview_window_id(Uuid::from_u128(112)));
    f.settle();
    f.open(115);
    same_size(f.rect(115).min - last.min, Vec2::splat(32.0));
    assert_eq!(f.rect(114), material);
}

#[test]
fn preview_sizes_follow_the_last_manual_resize_of_each_type_and_survive_restart() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars.clone(), Preferences::default());
    f.open(110);
    let small_texture = f.resize(110, egui::vec2(-80.0, -120.0));
    same_size(f.prefs.preview_size(false).expect("saved texture size"), small_texture);
    assert_eq!(f.prefs.material_preview_size, None);
    f.open(111);
    same_size(f.rect(111).size(), small_texture);
    f.open(113);
    assert!(
        (f.rect(113).size() - small_texture).length() > 20.0,
        "materials do not inherit texture sizes"
    );
    let large_material = f.resize(113, egui::vec2(60.0, 30.0));
    same_size(f.prefs.preview_size(true).expect("saved material size"), large_material);
    f.open(114);
    same_size(f.rect(114).size(), large_material);
    let larger_texture = f.resize(110, egui::vec2(55.0, 75.0));
    same_size(f.rect(111).size(), small_texture);
    same_size(f.rect(114).size(), large_material);
    same_size(f.prefs.preview_size(false).expect("most recent texture resize"), larger_texture);
    f.open(112);
    same_size(f.rect(112).size(), larger_texture);
    f.open(111);
    same_size(f.rect(111).size(), small_texture); // Already open: focus only.
    f.close(111);
    f.open(111);
    same_size(f.rect(111).size(), larger_texture); // New window, even for an old item.
    let saved = serde_json::to_string(&f.prefs).expect("persist both sizes");
    let mut restarted = Fixture::new(avatars, serde_json::from_str(&saved).expect("reload sizes"));
    restarted.open(115);
    restarted.open(116);
    same_size(restarted.rect(115).size(), larger_texture);
    same_size(restarted.rect(116).size(), large_material);
}

#[test]
fn preview_size_memory_ignores_placement_minimization_and_automatic_screen_fitting() {
    let legacy: Preferences = serde_json::from_str("{}").expect("old preferences remain valid");
    assert_eq!(legacy.texture_preview_size, None);
    assert_eq!(legacy.material_preview_size, None);
    let mut prefs = Preferences {
        texture_preview_size: Some([440.0, 380.0]),
        material_preview_size: Some([1100.0, 850.0]),
        ..Default::default()
    };
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, prefs.clone());
    f.open(110);
    let id = preview_window_id(Uuid::from_u128(110));
    widgets::place_floater(&f.ctx, &id, egui::pos2(20.0, 60.0), egui::vec2(250.0, 280.0));
    f.settle();
    assert_eq!(f.prefs, prefs);
    widgets::minimize_floater(&f.ctx, &id);
    f.settle();
    assert_eq!(f.prefs, prefs);
    f.open(110);
    assert_eq!(f.prefs, prefs);
    f.screen = egui::vec2(800.0, 560.0);
    f.open(113);
    assert!(f.rect(113).width() < 1100.0 && f.rect(113).height() < 850.0);
    assert_eq!(f.prefs, prefs, "screen clamping must not erase the preferred size");
    prefs.texture_preview_size = Some([f32::NAN, 300.0]);
    prefs.material_preview_size = Some([300.0, -10.0]);
    assert_eq!(prefs.preview_size(false), None);
    assert_eq!(prefs.preview_size(true), None);
}

#[test]
fn texture_preview_grows_past_320_pixels_and_keeps_each_windows_display_options() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.open(110);
    let image = |f: &Fixture| {
        f.ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", Uuid::from_u128(110)))))
            .expect("image")
    };
    let before = image(&f).size();
    assert!(before.x > 320.0, "preview must use the space available: {before:?}");
    f.resize(110, Vec2::new(260.0, 200.0));
    let after = image(&f).size();
    assert!(after.x > before.x + 100.0, "{before:?} -> {after:?}");
    assert!((after.x / after.y - 2.0).abs() < 0.01);
    f.resources.windows[0].texture.display.keep_ratio = false;
    f.settle();
    let stretched = image(&f).size();
    assert!((stretched.x / stretched.y - 2.0).abs() > 0.1);
    f.resources.windows[0].texture.display.actual_size = true;
    f.settle();
    same_size(image(&f).size(), Vec2::new(128.0, 64.0));
    f.open(111);
    assert!(!f.resources.windows[1].texture.display.actual_size);
    assert!(f.resources.windows[0].texture.display.actual_size);
}

#[test]
fn material_save_callbacks_are_scoped_to_their_window_and_preserve_failed_drafts() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.open(113);
    f.open(114);
    let value = f.resources.windows[0].editor.as_mut().expect("loaded");
    Arc::make_mut(&mut value.draft).metallic_factor = 0.2;
    let request = Uuid::from_u128(300);
    assert!(f.resources.begin_material_save(Uuid::from_u128(113), request));
    assert!(!f.resources.begin_material_save(Uuid::from_u128(113), Uuid::from_u128(301)));
    assert!(f.resources.material_saved(Uuid::from_u128(302), &Err("unrelated".into())).is_none());
    f.resources.material_saved(request, &Err("rejected".into()));
    assert!(f.resources.windows[0].editor.as_ref().expect("editor").dirty());
    assert!(!f.resources.windows[1].editor.as_ref().expect("editor").dirty());
    assert_eq!(f.scene.materials.get(&Uuid::from_u128(121)).expect("asset").metallic_factor, 1.0);
}

#[test]
fn a_new_material_with_no_asset_opens_its_default_parameters_immediately() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.world.inventory.items.get_mut(&Uuid::from_u128(113)).expect("item").asset_id = Uuid::nil();
    f.open(113);
    let editor = f.resources.windows[0].editor.as_ref().expect("new material defaults");
    assert_eq!(*editor.draft, aurora_assets::PbrMaterial::default());
    assert!(!editor.dirty());
}

#[test]
fn viewer_exit_waits_for_a_material_draft_or_pending_save() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.open(113);
    assert!(f.resources.prepare_exit(&f.ctx));
    let editor = f.resources.windows[0].editor.as_mut().expect("editor");
    Arc::make_mut(&mut editor.draft).roughness_factor = 0.3;
    assert!(!f.resources.prepare_exit(&f.ctx));
    let editor = f.resources.windows[0].editor.as_mut().expect("editor");
    assert!(editor.confirm_close);
    editor.draft = editor.original.clone();
    editor.begin_save(Uuid::from_u128(310));
    assert!(!f.resources.prepare_exit(&f.ctx));
    f.resources.material_saved(Uuid::from_u128(310), &Err("Rejected".into()));
    assert!(f.resources.prepare_exit(&f.ctx));
}

#[test]
fn material_maps_have_thumbnails_without_a_new_scene_upload_and_shapes_are_selectable() {
    use crate::imports::preview::MaterialShape;
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    f.images.clear(); // No prior UI image or pending GPU upload.
    f.open(113);
    f.front(113);
    let maps = [
        aurora_prim::te::BLANK_TEXTURE,
        aurora_prim::te::TRANSPARENT_TEXTURE,
        Uuid::from_u128(0xde40_a170_0000_0000_0000_0000_0000_0064),
        Uuid::from_u128(0xde40_a170_0000_0000_0000_0000_0000_0065),
    ];
    for (slot, map) in maps.into_iter().enumerate() {
        f.resources.windows[0].editor.as_mut().expect("editor").set_map(slot, map);
    }
    f.settle();
    let thumbnails = |f: &Fixture| {
        std::array::from_fn::<_, 4, _>(|slot| {
            f.ctx
                .data(|data| {
                    data.get_temp::<(Option<Uuid>, Option<egui::TextureId>, egui::Rect)>(egui::Id::unique((
                        "resource_material_map",
                        Uuid::from_u128(113),
                        slot,
                    )))
                })
                .expect("map row")
        })
    };
    let initial = thumbnails(&f);
    for (slot, (map, texture, _)) in initial.iter().enumerate() {
        assert_eq!(*map, Some(maps[slot]));
        assert!(texture.is_some(), "map {slot} needs actual pixels");
    }
    for shape in [MaterialShape::Cube, MaterialShape::Aurora, MaterialShape::Sphere] {
        let choice = f
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_shape_choice", Uuid::from_u128(113)))))
            .expect("shape selector")
            .center();
        f.button(choice, true);
        f.button(choice, false);
        f.settle();
        let option = f
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_shape", Uuid::from_u128(113), shape))))
            .expect("shape option")
            .center();
        f.button(option, true);
        f.button(option, false);
        f.settle();
        assert!(f.resources.windows[0].editor.as_ref().expect("editor").shape == shape);
        for (slot, (_, texture, _)) in thumbnails(&f).iter().enumerate() {
            assert_eq!(*texture, initial[slot].1, "shape changes reuse map thumbnails");
        }
    }
    let replacement = Uuid::from_u128(0xde40_a170_0000_0000_0000_0000_0000_0066);
    f.resources.windows[0].editor.as_mut().expect("editor").set_map(0, replacement);
    f.settle();
    let updated = thumbnails(&f);
    assert_eq!(updated[0].0, Some(replacement));
    assert!(updated[0].1.is_some() && updated[0].1 != initial[0].1);
}

#[test]
fn material_window_keeps_its_geometry_with_wrapped_settings_messages_and_late_maps() {
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let mut f = Fixture::new(avatars, Preferences::default());
    let item = Uuid::from_u128(113);
    let source = f.world.inventory.items.get_mut(&item).expect("item");
    source.owner = f.world.agent_id;
    source.owner_mask = rules::MODIFY | rules::COPY | rules::TRANSFER;
    f.open(113);
    for delta in [Vec2::new(-230.0, -380.0), Vec2::new(180.0, 250.0), Vec2::new(280.0, 240.0)] {
        f.resize(113, delta);
        let expected = f.rect(113);
        for frame in 0..120 {
            let editor = f.resources.windows[0].editor.as_mut().expect("editor");
            if frame == 10 {
                let value = Arc::make_mut(&mut editor.draft);
                value.alpha_mode = aurora_assets::AlphaMode::Mask;
                value.base_color_texture = Some(aurora_prim::te::BLANK_TEXTURE);
                value.normal_texture = Some(aurora_prim::te::TRANSPARENT_TEXTURE);
            }
            if frame == 30 {
                editor.message = "A long response that must wrap inside the footer without enlarging the preview window. ".repeat(3);
            }
            if frame == 60 {
                f.world.inventory.items.get_mut(&item).expect("item").owner_mask = rules::COPY;
            }
            f.frame(Vec::new());
            same_size(f.rect(113).size(), expected.size());
        }
        let viewport = f
            .ctx
            .data(|data| data.get_temp::<(egui::Rect, Vec2)>(egui::Id::unique(("resource_material_viewport", item))))
            .expect("scroll viewport");
        assert!(f.rect(113).contains_rect(viewport.0));
        assert!(viewport.1.y > viewport.0.height(), "settings stay scrollable");
    }
}

/// A texture preview keeps its size with every line under its picture: an
/// export under way and a long message.
#[test]
fn a_texture_preview_keeps_its_size_with_its_messages() {
    let avatars = Arc::new(crate::scene::avatar::AvatarLibrary::load());
    let item = Uuid::from_u128(110);
    let images = {
        let f = Fixture::new(avatars.clone(), Preferences::default());
        f.images
    };
    widgets::steady_tests::assert_window_steady(
        &preview_window_id(item),
        &[Vec2::new(460.0, 520.0), Vec2::new(260.0, 300.0)],
        aurora_i18n::tr!("texture-library-no-copy"),
        || {
            let mut f = Fixture::new(avatars.clone(), Preferences::default());
            f.resources.open_preview(&f.ctx, &f.world, item);
            let window = f.resources.windows.iter_mut().find(|w| w.id == item).expect("preview");
            window.texture.export = Some(false);
            window.texture.message = "The picture could not be saved: the folder chosen cannot be written.".into();
            (f.resources, f.world, f.scene, f.prefs)
        },
        |(resources, world, scene, prefs), ctx, p| {
            resources.show(ctx, p, world, scene, &images, prefs, &mut false);
        },
    );
}
