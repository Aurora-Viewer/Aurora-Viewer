use super::*;

#[test]
fn mesh_physics_warning_precedes_quotes_and_convex_action_opens_the_collision_preview() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut draft = crate::imports::tests::invalid_physics_fixture();
    draft.prepared = Some(Arc::new(mesh::prepare(&draft).expect("physics validation")));
    let scene = draft.scene.clone();
    let entry = Entry::new("physics.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
    let id = entry.id;
    let mut imports = Imports {
        choice: Choice::Mesh,
        active: Some(id),
        entries: vec![entry],
        ..Default::default()
    };
    let frame = |imports: &mut Imports, events| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(820.0, 400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(
                    ui,
                    &theme.palette(),
                    imports,
                    &Preferences::default(),
                    &world,
                    &Prices::default(),
                    &mut actions,
                );
            },
        );
        output.textures_delta.clear();
        (output, actions)
    };
    for _ in 0..3 {
        frame(&mut imports, vec![]);
    }
    let (output, actions) = frame(&mut imports, vec![]);
    assert!(actions.is_empty());
    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().contains(&aurora_i18n::tr!("import-degenerate-physics", count = 1)))));
    let quote_state = || {
        ctx.data(|data| data.get_temp::<(egui::Rect, egui::Rect, bool)>(egui::Id::unique("import_mesh_quote_action")))
            .unwrap()
    };
    assert!(!quote_state().2, "invalid physics blocks the fee action before any request");
    let (rect, clip) = ctx
        .data(|data| data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::unique("import_mesh_repair_action")))
        .unwrap();
    assert!(clip.contains_rect(rect), "the repair action remains reachable");
    assert!(rect.height() >= 20.0, "the repair action needs a real pointer target");
    for pressed in [true, false] {
        let (_, actions) = frame(
            &mut imports,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert!(actions.is_empty(), "conversion neither quotes nor uploads");
    }
    let entry = &mut imports.entries[0];
    let Some(Source::Mesh(draft)) = &mut entry.source else {
        panic!("mesh")
    };
    assert!(draft.options.decompose && draft.preview_physics);
    assert!(draft.prepared.is_none() && draft.quote.is_none());
    assert!(Arc::ptr_eq(&scene, &draft.scene), "visual source remains intact");
    assert_eq!(
        ctx.data(|data| data.get_temp::<usize>(egui::Id::unique(("mesh_section", id)))),
        Some(1)
    );
    assert!(!quote_state().2, "wait for the replacement geometry");
    draft.prepared = Some(Arc::new(mesh::prepare(draft).expect("completed convex analysis")));
    frame(&mut imports, vec![]);
    assert!(quote_state().2, "a valid convex shape can be quoted");
    assert!(imports.queue.is_empty() && !imports.confirm && imports.spent == 0);
}

#[test]
fn a_free_upload_starts_without_the_payment_confirmation_and_a_paid_one_still_asks() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![aurora_net::inventory::InvFolder {
        id: world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    world.balance = Some(1000);
    let price = |cost| Prices {
        texture: Some(cost),
        large_texture: Some(cost),
        sound: Some(cost),
        animation: Some(cost),
    };
    for (cost, starts) in [(0, true), (10, false)] {
        let entry = Entry::new(
            "free.png".into(),
            Ok(Source::Texture(crate::imports::tests::image(16, 16))),
            Default::default(),
        );
        let mut imports = Imports {
            choice: Choice::Texture,
            active: Some(entry.id),
            entries: vec![entry],
            demo_started: true,
            ..Default::default()
        };
        let prices = price(cost);
        let frame = |imports: &mut Imports, events| {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(820.0, 300.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(
                        ui,
                        &theme.palette(),
                        imports,
                        &Preferences::default(),
                        &world,
                        &prices,
                        &mut actions,
                    );
                },
            );
            output.textures_delta.clear();
            actions
        };
        for _ in 0..3 {
            frame(&mut imports, vec![]);
        }
        let (rect, clip, enabled) = ctx
            .data(|data| data.get_temp::<(egui::Rect, egui::Rect, bool)>(egui::Id::unique("import_upload_action")))
            .expect("upload action");
        assert!(enabled && clip.contains_rect(rect), "L${cost}: the upload action is reachable");
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(frame(
                &mut imports,
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
        if starts {
            assert!(
                matches!(actions.as_slice(), [Action::StartFor(Choice::Texture)]),
                "L$0 starts at once"
            );
            assert!(!imports.confirm && imports.confirm_choice.is_none());
        } else {
            assert!(actions.is_empty(), "a paid upload waits for the confirmation");
            assert!(imports.confirm);
            assert_eq!(imports.confirm_choice, Some(Choice::Texture));
        }
    }
}

#[test]
fn local_upload_button_uses_checked_files_instead_of_the_active_item_and_disables_during_loading() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut texture = Entry::new(
        "unchecked.png".into(),
        Ok(Source::Texture(crate::imports::tests::image(16, 16))),
        Default::default(),
    );
    texture.local = true;
    texture.selected = false;
    let mut mesh = Entry::new(
        "checked.dae".into(),
        Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
        Default::default(),
    );
    mesh.local = true;
    let mut imports = Imports {
        choice: Choice::Local,
        active: Some(texture.id),
        entries: vec![texture, mesh],
        ..Default::default()
    };
    imports.type_filters[4] = false;
    let frame = |imports: &mut Imports, events| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(820.0, 300.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(
                    ui,
                    &theme.palette(),
                    imports,
                    &Preferences::default(),
                    &world,
                    &Prices::default(),
                    &mut actions,
                );
            },
        );
        output.textures_delta.clear();
        actions
    };
    let state = || {
        ctx.data(|data| data.get_temp::<(egui::Rect, egui::Rect, bool)>(egui::Id::unique("import_stage_local_uploads")))
            .unwrap()
    };
    for _ in 0..3 {
        frame(&mut imports, vec![]);
    }
    let (rect, clip, enabled) = state();
    assert!(
        enabled && clip.contains_rect(rect),
        "the new action remains reachable when footer actions wrap"
    );
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(
            &mut imports,
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        ));
    }
    assert_eq!(
        actions.iter().filter(|action| matches!(action, Action::StageLocalUploads)).count(),
        1
    );
    assert!(!imports.confirm && imports.queue.is_empty() && imports.spent == 0);
    imports.pending_files = Some(Uuid::new_v4());
    frame(&mut imports, vec![]);
    assert!(!state().2);
    imports.pending_files = None;
    imports.entries[1].selected = false;
    frame(&mut imports, vec![]);
    assert!(!state().2);
    imports.entries[1].selected = true;
    imports.confirm = true;
    frame(&mut imports, vec![]);
    assert!(!state().2);
}
