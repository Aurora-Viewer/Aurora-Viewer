use super::*;
use crate::scene::avatar::AvatarLibrary;
use std::sync::Arc;
use uuid::Uuid;

fn frame(ctx: &egui::Context, panel: &mut PortalUi, world: &World, events: Vec<egui::Event>) -> Actions {
    let mut actions = Actions::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            events,
            ..Default::default()
        },
        |ui| {
            super::super::close_key::begin(ui.ctx());
            actions = panel.show(ui.ctx(), &crate::theme::Theme::default().palette(), world);
        },
    );
    output.textures_delta.clear();
    actions
}
fn click(ctx: &egui::Context, panel: &mut PortalUi, world: &World, at: egui::Pos2) -> Actions {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(ctx, panel, world, vec![egui::Event::PointerMoved(at)]);
    frame(ctx, panel, world, vec![button(true)]);
    frame(ctx, panel, world, vec![button(false)])
}

/// No bridge is needed any more: a portal is drawn by this viewer only.
#[test]
fn portal_creator_needs_only_a_place_of_this_grid() {
    let ctx = egui::Context::default();
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    let mut panel = PortalUi {
        open: true,
        destination: "Demo destination/128/128/25".into(),
        ..Default::default()
    };
    for _ in 0..4 {
        frame(&ctx, &mut panel, &world, Vec::new());
    }
    let create = panel.create_rect.unwrap().center();
    assert!(click(&ctx, &mut panel, &world, create).create);
    world.teleporting = true;
    assert!(!click(&ctx, &mut panel, &world, create).create, "not while teleporting");
    world.teleporting = false;
    panel.destination.clear();
    frame(&ctx, &mut panel, &world, Vec::new());
    let create = panel.create_rect.unwrap().center();
    assert!(!click(&ctx, &mut panel, &world, create).create);
}

#[test]
fn portal_labels_show_the_destination_its_time_left_and_whether_to_step_away() {
    use portals::{LIFETIME, Portal};
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(1);
    world.main_region = Some(256);
    world.agent.position = Vec3::new(10.0, 10.0, 25.0);
    let mut state = portals::State::default();
    let unix = portals::unix_time();
    state.sync(&world, unix);
    let created = state.create(
        Portal {
            id: Uuid::from_u128(3),
            region: 256,
            center: [10.0, 10.0, 25.0],
            yaw: 0.0,
            destination: "Demo destination".into(),
            position: [987.0, 654.0, 321.0],
            expires: unix + LIFETIME,
        },
        crate::world::land::BuildRight::Allowed,
        world.agent.position,
        std::time::Instant::now(),
        unix,
    );
    assert!(created.is_ok());
    let entry = state.own().unwrap();
    for remaining in [LIFETIME, 1, 0] {
        let at = unix + LIFETIME - remaining;
        let timer = aurora_i18n::tr!("portal-expires", seconds = remaining.to_string());
        assert!(label_text(&world, entry, at).contains(timer.as_str()));
    }
    let expired = aurora_i18n::tr!("portal-expires", seconds = "0".to_string());
    assert!(label_text(&world, entry, unix + LIFETIME + 1).contains(expired.as_str()));
    let text = label_text(&world, entry, unix);
    assert!(text.contains("Demo destination (987, 654, 321)"), "{text}");
    assert!(text.contains(aurora_i18n::tr!("portal-step-away")), "opened on us: {text}");
}
