use super::*;

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
                    &Default::default(),
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
