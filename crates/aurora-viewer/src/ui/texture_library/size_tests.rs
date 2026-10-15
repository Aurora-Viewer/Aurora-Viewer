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
            .memory(|m| m.area_rect(egui::Id::new(preview_window_id(Uuid::from_u128(item)))))
            .expect("preview window")
    }
    fn open(&mut self, item: u128) {
        self.resources.open_preview(&self.ctx, &self.world, Uuid::from_u128(item));
        self.settle();
    }
    fn front(&mut self, item: u128) {
        let id = preview_window_id(Uuid::from_u128(item));
        widgets::move_floater(&self.ctx, &id, egui::pos2(20.0, 60.0));
        self.ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(id)));
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
    same_size(f.rect(111).min - f.rect(110).min, Vec2::splat(16.0));
    let last = f.rect(111);
    f.front(110);
    f.open(110); // Raising an older item must not become the cascade anchor.
    f.open(112);
    same_size(f.rect(112).min - last.min, Vec2::splat(16.0));
    f.open(113);
    f.open(114);
    same_size(f.rect(114).min - f.rect(113).min, Vec2::splat(16.0));
    let material = f.rect(114);
    widgets::minimize_floater(&f.ctx, &preview_window_id(Uuid::from_u128(112)));
    f.settle();
    f.open(115);
    same_size(f.rect(115).min - last.min, Vec2::splat(16.0));
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
