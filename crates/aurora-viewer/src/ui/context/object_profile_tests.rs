use super::*;
use crate::ui::widgets::elide_tests::{Frames, Text, painted};
use aurora_net::build::{BuildCmd, perm, perm_field};

#[test]
fn object_profile_state_stays_small_on_the_windows_startup_stack() {
    assert!(std::mem::size_of::<ObjectProfileUi>() <= 64);
}

struct Harness {
    frames: Frames,
    palette: Palette,
    world: World,
    profile: ObjectProfileUi,
    facts: crate::ui::context::ObjectFacts,
    id: Uuid,
    key: ObjKey,
}

impl Harness {
    fn new() -> Self {
        let frames = Frames::new();
        let theme = crate::theme::Theme::default();
        theme.apply(&frames.ctx, 1.0);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events()
            .into_iter()
            .chain(crate::demo::object_menu::events(glam::Vec3::splat(130.0), glam::Vec3::X))
        {
            world.apply(event);
        }
        world.agent_id = crate::demo::DEMO_AGENT;
        let object = world
            .objects
            .iter()
            .find(|(_, object)| object.key.local_id == crate::demo::object_menu::BELL)
            .unwrap()
            .1;
        let (id, key) = (object.full_id, object.key);
        world.main_region = Some(key.region);
        let props = ObjectProps {
            object_id: id,
            full: true,
            name: "Profile original".into(),
            description: "Original description".into(),
            owner_id: world.agent_id,
            base_mask: perm::MOVE | perm::COPY | perm::MODIFY | perm::TRANSFER,
            owner_mask: perm::MOVE | perm::COPY | perm::MODIFY | perm::TRANSFER,
            next_owner_mask: perm::COPY | perm::TRANSFER,
            sale_price: 25,
            ..Default::default()
        };
        let mut profile = ObjectProfileUi::default();
        profile.open(id, key, Some(props));
        let mut harness = Self {
            frames,
            palette: theme.palette(),
            world,
            profile,
            id,
            key,
            facts: Default::default(),
        };
        for _ in 0..4 {
            harness.draw(Vec::new());
        }
        harness
    }

    fn draw(&mut self, events: Vec<egui::Event>) -> Vec<Text> {
        self.frames.run(events, |ui| {
            self.profile.show(ui.ctx(), &self.palette, &mut self.world, &self.facts)
        })
    }

    fn click(&mut self, label: &str) {
        let texts = self.draw(Vec::new());
        let position = painted(&texts, label).rect.center();
        for pressed in [true, false] {
            self.draw(vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }

    fn type_and_commit(&mut self, text: &str) {
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        self.draw(vec![
            key(
                egui::Key::A,
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..Default::default()
                },
            ),
            egui::Event::Text(text.into()),
            key(egui::Key::Enter, Default::default()),
        ]);
    }
}

#[test]
fn object_profile_edits_name_and_description_directly_through_native_messages() {
    let mut h = Harness::new();
    h.click("Profile original");
    h.type_and_commit("Profile renamed");
    let (commands, _, _) = h.profile.take_output();
    assert!(commands.iter().any(
        |command| matches!(command, NetCommand::Build(BuildCmd::SetName { handle, local_id, name })
        if *handle == h.key.region && *local_id == h.key.local_id && name == "Profile renamed")
    ));
    h.click("Original description");
    h.type_and_commit("Changed description");
    assert!(h.profile.take_output().0.iter().any(
        |command| matches!(command, NetCommand::Build(BuildCmd::SetDescription { local_id, description, .. })
        if *local_id == h.key.local_id && description == "Changed description")
    ));
}

#[test]
fn object_profile_permissions_are_editable_without_modify_and_cannot_follow_an_unrelated_selection() {
    let mut h = Harness::new();
    let index = h.world.objects.index_of_uuid(&h.id).unwrap();
    h.world.objects.get_mut(index).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
    let other = h
        .world
        .objects
        .iter()
        .find(|(_, object)| object.key.local_id == crate::demo::object_menu::CRATE)
        .unwrap()
        .1
        .key;
    h.profile.editor.selection = vec![other];
    h.click(aurora_i18n::tr!("common-move"));
    let commands = h.profile.take_output().0;
    assert!(commands.iter().any(
        |command| matches!(command, NetCommand::Build(BuildCmd::SetPermissions { local_ids, field, set: true, mask, .. })
        if local_ids == &[h.key.local_id] && *field == perm_field::EVERYONE && *mask == perm::MOVE)
    ));
    h.profile.editor.props.get_mut(&h.id).unwrap().owner_id = Uuid::from_u128(500);
    h.click(aurora_i18n::tr!("common-move"));
    assert!(h.profile.take_output().0.is_empty());
    h.click("Profile original");
    h.type_and_commit("Denied rename");
    assert!(h.profile.take_output().0.is_empty());
}

#[test]
fn object_profile_sale_apply_uses_the_pinned_root_and_rechecks_revoked_ownership() {
    let mut h = Harness::new();
    h.click(aurora_i18n::tr!("common-for-sale"));
    assert!(h.profile.take_output().0.is_empty(), "sale checkbox stages the native draft");
    h.click(aurora_i18n::tr!("common-apply"));
    assert!(h.profile.take_output().0.iter().any(
        |command| matches!(command, NetCommand::Build(BuildCmd::SetSaleInfo { local_ids, sale_type: 2, price: 25, .. })
        if local_ids == &[h.key.local_id])
    ));
    h.profile.editor.ui.sale_draft = Some((true, 2, 150));
    h.profile.editor.props.get_mut(&h.id).unwrap().owner_id = Uuid::from_u128(500);
    h.click(aurora_i18n::tr!("common-apply"));
    assert!(h.profile.take_output().0.is_empty(), "old drafts never bypass changed rights");
}

#[test]
fn object_profile_rejects_unrelated_properties_resets_drafts_on_target_change_and_disables_departed_objects() {
    let mut h = Harness::new();
    h.profile.on_properties(&[ObjectProps {
        object_id: Uuid::from_u128(500),
        full: true,
        name: "Unrelated name".into(),
        ..Default::default()
    }]);
    assert_eq!(h.profile.editor.props.len(), 1);
    h.profile.on_properties(&[ObjectProps {
        object_id: h.id,
        full: false,
        name: "Partial response".into(),
        ..Default::default()
    }]);
    assert_eq!(h.profile.editor.props[&h.id].name, "Profile original");
    h.profile.editor.ui.edits.insert("name", "Old draft".into());
    h.profile.editor.ui.sale_draft = Some((true, 2, 999));
    h.profile.editor.ui.group_picker_open = true;
    let other = h
        .world
        .objects
        .iter()
        .find(|(_, object)| object.key.local_id == crate::demo::object_menu::CRATE)
        .unwrap()
        .1;
    let (id, key) = (other.full_id, other.key);
    h.profile.open(id, key, None);
    assert!(h.profile.editor.ui.edits.is_empty());
    assert!(h.profile.editor.ui.sale_draft.is_none());
    assert!(!h.profile.editor.ui.group_picker_open);
    assert!(h.profile.editor.props.is_empty());
    h.world.apply(aurora_net::NetEvent::ObjectsKilled {
        handle: key.region,
        local_ids: vec![key.local_id],
    });
    let texts = h.draw(Vec::new());
    assert!(
        texts
            .iter()
            .any(|text| text.source == aurora_i18n::tr!("scene-object-no-longer-visible"))
    );
    assert!(h.profile.take_output().0.is_empty());
}

#[test]
fn object_profile_details_and_open_use_native_actions_and_missing_rights_disable_buy_and_pay() {
    let mut h = Harness::new();
    h.click(aurora_i18n::tr!("common-pay"));
    h.click(aurora_i18n::tr!("common-buy"));
    assert!(super::super::take_requests(&h.frames.ctx).is_empty());
    h.facts.open = true;
    h.click(aurora_i18n::tr!("common-open"));
    h.click(aurora_i18n::tr!("scene-object-details"));
    let actions = super::super::take_requests(&h.frames.ctx);
    assert!(
        matches!(&actions[..], [super::super::CtxAction::Open(open), super::super::CtxAction::Inspect(inspect)] if *open == h.id && *inspect == h.id)
    );
}

#[test]
fn object_profile_hud_edits_only_its_root_and_resolves_the_group_name() {
    let mut h = Harness::new();
    for event in crate::demo::hud::events() {
        h.world.apply(event);
    }
    let object = h
        .world
        .objects
        .iter()
        .find(|(_, object)| object.key.local_id == crate::demo::hud::FIRST)
        .unwrap()
        .1;
    let (id, key) = (object.full_id, object.key);
    let mut props = h.profile.editor.props[&h.id].clone();
    props.object_id = id;
    props.name = "HUD root".into();
    props.group_id = Uuid::from_u128(0xfeed_0101);
    h.world.land.group_names.insert(props.group_id, "Resolved group".into());
    h.profile.open(id, key, Some(props));
    let texts = h.draw(Vec::new());
    assert!(texts.iter().any(|text| text.source == "Resolved group"));
    assert!(
        !texts
            .iter()
            .any(|text| text.source == aurora_i18n::tr!("build-ui-general-waiting-for-simulator-properties"))
    );
    h.click("HUD root");
    h.type_and_commit("Edited HUD");
    assert!(h.profile.take_output().0.iter().any(
        |command| matches!(command, NetCommand::Build(BuildCmd::SetName { local_id, name, .. })
        if *local_id == key.local_id && name == "Edited HUD")
    ));
}

#[test]
fn object_profile_keeps_its_live_properties_subscription_without_holding_other_selections() {
    let mut h = Harness::new();
    let mut command = NetCommand::Build(BuildCmd::Deselect {
        handle: h.key.region,
        local_ids: vec![h.key.local_id, 9999],
    });
    assert!(h.profile.retain_selection(&h.world, &mut command));
    assert!(matches!(command, NetCommand::Build(BuildCmd::Deselect { local_ids, .. }) if local_ids == [9999]));
    let mut command = NetCommand::Build(BuildCmd::Deselect {
        handle: h.key.region,
        local_ids: vec![h.key.local_id],
    });
    assert!(!h.profile.retain_selection(&h.world, &mut command));
    h.profile.object = None;
    let mut command = NetCommand::Build(BuildCmd::Deselect {
        handle: h.key.region,
        local_ids: vec![h.key.local_id],
    });
    assert!(h.profile.retain_selection(&h.world, &mut command));
}
