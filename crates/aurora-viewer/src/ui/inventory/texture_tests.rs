//! Pointer regressions for inventory texture previews and world cargo.
use super::*;
use std::{collections::HashMap, sync::Arc};

#[test]
fn double_click_opens_a_texture_preview_and_both_sources_drag_the_actual_item() {
    let ctx = egui::Context::default();
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(2);
    world.inventory = super::tests::view_inventory(false);
    let item = Uuid::from_u128(91001);
    let asset = Uuid::from_u128(91002);
    world.inventory.add_items(vec![aurora_net::inventory::InvItem {
        id: item,
        parent: world.inventory.root,
        asset_id: asset,
        asset_type: 0,
        name: "Texture preview regression".into(),
        owner: world.agent_id,
        owner_mask: rules::COPY | rules::TRANSFER,
        ..Default::default()
    }]);
    let image = ctx.load_texture(
        "inventory texture test",
        egui::ColorImage::filled([128, 64], egui::Color32::GRAY),
        Default::default(),
    );
    let images = HashMap::from([(asset, image)]);
    let mut state = InventoryUi::default();
    state.show_original(&world.inventory, item);
    let mut prefs = InventoryPreferences::default();
    let mut resources = crate::ui::texture_library::LibraryUi::default();
    let mut library_prefs = crate::ui::texture_library::Preferences::default();
    let mut scene = crate::scene::Scene::new(std::env::temp_dir(), world.avatar_lib.clone());
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>, state: &mut InventoryUi| {
        time += 1.0 / 60.0;
        let mut actions = Vec::new();
        let mut dropped = None;
        let output = headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            // every pass's actions, as the app handles them (a pass laid out
            // again has no input of its own)
            |host| {
                let pass = show(
                    host.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &Icons::default(),
                    &mut world,
                    state,
                    &mut prefs,
                    &mut true,
                    false,
                    &images,
                    &Default::default(),
                    &Default::default(),
                    &Default::default(),
                    true,
                );
                for action in &pass {
                    if let InvAction::Preview(item) = action {
                        resources.open_preview(host.ctx(), &world, *item);
                    }
                }
                actions.extend(pass);
                resources.show(
                    host.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut world,
                    &mut scene,
                    &images,
                    &mut library_prefs,
                    &mut false,
                );
                if let Some(drag) = world_drag(host.ctx())
                    && drag.released
                {
                    dropped = Some(drag.item);
                    egui::DragAndDrop::clear_payload(host.ctx());
                }
            },
        ));
        fn row(shape: &egui::Shape) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("Texture preview regression") => {
                    Some(text.pos + egui::vec2(5.0, 5.0))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(row),
                _ => None,
            }
        }
        (actions, dropped, output.shapes.iter().find_map(|shape| row(&shape.shape)))
    };
    let mut row = None;
    for _ in 0..8 {
        row = frame(Vec::new(), &mut state).2;
    }
    let row = row.expect("visible inventory texture row");
    let button = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            },
        ]
    };
    frame(button(row, true), &mut state);
    frame(button(row, false), &mut state);
    frame(button(row, true), &mut state);
    let actions = frame(button(row, false), &mut state).0;
    assert!(actions.iter().any(|action| matches!(action, InvAction::Preview(id) if *id == item)));
    assert_eq!(state.preview, None, "resource windows do not replace document editors");
    for _ in 0..8 {
        frame(Vec::new(), &mut state);
    }
    let rect = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", item))))
        .expect("preview image");
    assert!((rect.aspect_ratio() - 2.0).abs() < 0.01, "preview keeps the image aspect ratio");
    let destination = egui::pos2(100.0, 700.0);
    for source in [rect.center(), row] {
        if source == row {
            let window = ctx
                .memory(|m| m.area_rect(egui::Id::unique(crate::ui::texture_library::preview_window_id(item))))
                .expect("preview window");
            let close = egui::pos2(window.right() - 18.0, window.top() + 19.0);
            frame(button(close, true), &mut state);
            frame(button(close, false), &mut state);
            for _ in 0..4 {
                frame(Vec::new(), &mut state);
            }
        }
        frame(button(source, true), &mut state);
        frame(vec![egui::Event::PointerMoved(source + egui::vec2(-15.0, 0.0))], &mut state);
        assert_eq!(egui::DragAndDrop::payload::<InvDrag>(&ctx).expect("real drag payload").0, item);
        frame(vec![egui::Event::PointerMoved(destination)], &mut state);
        assert_eq!(frame(button(destination, false), &mut state).1, Some(item));
    }
}
