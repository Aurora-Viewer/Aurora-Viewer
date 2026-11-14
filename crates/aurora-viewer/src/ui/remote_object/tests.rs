use super::*;
use crate::scene::avatar::AvatarLibrary;
use crate::ui::widgets::elide_tests::Frames;
use std::sync::Arc;
use uuid::Uuid;

fn fixture() -> (World, RemoteObject) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(1);
    let object = RemoteObject {
        id: Uuid::from_u128(2),
        owner: Uuid::from_u128(3),
        name: "A talking object".into(),
        group_owned: false,
        slurl: "Demo/14/40/22".into(),
    };
    world.social.names.insert(object.owner, "Demo Owner".into());
    world.land.group_names.insert(object.owner, "Demo Group".into());
    (world, object)
}

fn click(frames: &mut Frames, at: egui::Pos2, mut draw: impl FnMut(&mut egui::Ui)) {
    for pressed in [true, false] {
        frames.run(
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
            &mut draw,
        );
    }
}

#[test]
fn object_information_buttons_target_the_object_and_its_owner_separately() {
    for group_owned in [false, true] {
        for block_owner in [false, true] {
            let (world, mut object) = fixture();
            object.group_owned = group_owned;
            let mut frames = Frames::new();
            let theme = crate::theme::Theme::default();
            theme.apply(&frames.ctx, 1.0);
            let palette = theme.palette();
            let mut shown = Some(object.clone());
            let mut draw = |ui: &mut egui::Ui| show(ui.ctx(), &palette, &world, &mut shown);
            frames.run(Vec::new(), &mut draw);
            let texts = frames.run(Vec::new(), &mut draw);
            let label = if block_owner {
                "ui-remote-object-block-owner"
            } else {
                "ui-remote-object-block-object"
            };
            let at = texts
                .iter()
                .find(|t| t.source == aurora_i18n::text(label))
                .expect("block button")
                .rect
                .center();
            click(&mut frames, at, &mut draw);
            let actions = super::super::context::take_requests(&frames.ctx);
            if block_owner {
                assert!(
                    matches!(&actions[..], [CtxAction::BlockObjectOwner { owner, group_owned: group }]
                    if *owner == object.owner && *group == group_owned),
                    "{actions:?}"
                );
            } else {
                assert!(
                    matches!(&actions[..], [CtxAction::ToggleBlockObject { id, .. }]
                    if *id == object.id),
                    "{actions:?}"
                );
            }
        }
    }
}

#[test]
fn object_information_never_offers_to_block_ourselves_or_an_unknown_owner() {
    for unknown in [false, true] {
        let (world, mut object) = fixture();
        object.owner = if unknown { Uuid::nil() } else { world.agent_id };
        let mut frames = Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        let palette = theme.palette();
        let mut shown = Some(object);
        let mut draw = |ui: &mut egui::Ui| show(ui.ctx(), &palette, &world, &mut shown);
        frames.run(Vec::new(), &mut draw);
        let texts = frames.run(Vec::new(), &mut draw);
        assert!(!texts.iter().any(|t| t.source == aurora_i18n::tr!("ui-remote-object-block-owner")));
    }
}
