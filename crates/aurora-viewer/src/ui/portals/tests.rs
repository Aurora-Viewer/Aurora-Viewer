use super::*;
use crate::scene::avatar::AvatarLibrary;
use std::sync::Arc;

fn frame(ctx: &egui::Context, panel: &mut PortalUi, world: &World, ready: bool, events: Vec<egui::Event>) -> Actions {
    let mut actions = Actions::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            events,
            ..Default::default()
        },
        |ui| {
            super::super::close_key::begin(ui.ctx());
            actions = panel.show(ui.ctx(), &crate::theme::Theme::default().palette(), world, ready);
        },
    );
    output.textures_delta.clear();
    actions
}
fn click(ctx: &egui::Context, panel: &mut PortalUi, world: &World, ready: bool, at: egui::Pos2) -> Actions {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(ctx, panel, world, ready, vec![egui::Event::PointerMoved(at)]);
    frame(ctx, panel, world, ready, vec![button(true)]);
    frame(ctx, panel, world, ready, vec![button(false)])
}
#[test]
fn portal_creator_requires_ready_bridge_and_destination_and_selects_an_actual_member_group() {
    let ctx = egui::Context::default();
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    let group = Uuid::from_u128(7);
    world.groups.groups.push(aurora_net::GroupMembership {
        id: group,
        name: "Demo group".into(),
        insignia: Uuid::nil(),
        powers: 0,
        accept_notices: true,
        list_in_profile: true,
        contribution: 0,
    });
    let mut panel = PortalUi {
        open: true,
        destination: "Demo destination/128/128/25".into(),
        ..Default::default()
    };
    for _ in 0..4 {
        frame(&ctx, &mut panel, &world, false, Vec::new());
    }
    let create = panel.rects[1].unwrap().center();
    assert!(!click(&ctx, &mut panel, &world, false, create).create);
    let radio = panel.rects[0].unwrap().center();
    click(&ctx, &mut panel, &world, true, radio);
    assert_eq!(panel.access, Access::Group(group));
    for _ in 0..3 {
        frame(&ctx, &mut panel, &world, true, Vec::new());
    }
    let create = panel.rects[1].unwrap().center();
    assert!(click(&ctx, &mut panel, &world, true, create).create);
    panel.destination.clear();
    frame(&ctx, &mut panel, &world, true, Vec::new());
    let create = panel.rects[1].unwrap().center();
    assert!(!click(&ctx, &mut panel, &world, true, create).create);
}
