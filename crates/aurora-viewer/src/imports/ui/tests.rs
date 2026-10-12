use super::*;

#[test]
fn every_local_asset_exposes_the_same_three_actions_without_a_footer_close_button() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut prefs = Preferences::default();
    let sources = [
        Source::Texture(Arc::new(DecodedImage {
            width: 1,
            height: 1,
            components: 3,
            data: vec![255; 3],
            discard: 0,
        })),
        Source::Material {
            value: PbrMaterial::default(),
            images: Box::new(std::array::from_fn(|_| None)),
        },
        Source::Mesh(Box::new(crate::imports::tests::mesh_fixture())),
    ];
    for source in sources {
        let mut entry = Entry::new("preview".into(), Ok(source), Default::default());
        entry.local = true;
        let id = entry.id;
        let mut imports = Imports {
            open: true,
            choice: Choice::Local,
            entries: vec![entry],
            active: Some(id),
            ..Default::default()
        };
        let mut frame = |events| {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| actions = show(ui.ctx(), &theme.palette(), &mut imports, &mut prefs, &world, &Default::default()),
            );
            output.textures_delta.clear();
            (output, actions)
        };
        for _ in 0..3 {
            drop(frame(Vec::new()));
        }
        let (output, _) = frame(Vec::new());
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some((text.galley.text().to_owned(), text.pos + text.galley.size() * 0.5)),
                _ => None,
            })
            .collect();
        assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("common-close")));
        for (index, label) in [
            aurora_i18n::tr!("import-local-apply"),
            aurora_i18n::tr!("import-local-clear"),
            aurora_i18n::tr!("import-local-rez"),
        ]
        .into_iter()
        .enumerate()
        {
            let pos = texts.iter().find(|(text, _)| text == label).expect("local control visible").1;
            for pressed in [true, false] {
                let (_, actions) = frame(vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                if !pressed {
                    assert!(
                        match actions.as_slice() {
                            [Action::Local(entry)] => index == 0 && *entry == id,
                            [Action::ClearLocal(entry)] => index == 1 && *entry == id,
                            [Action::RezLocal(entry)] => index == 2 && *entry == id,
                            _ => false,
                        },
                        "local button dispatches its action"
                    );
                }
            }
        }
    }
}

#[test]
fn menu_file_request_is_emitted_once_and_tab_changes_do_not_reopen_the_picker() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut prefs = Preferences::default();
    let mut imports = Imports {
        open: true,
        choice: Choice::Mesh,
        choose_files_on_open: true,
        ..Default::default()
    };
    let mut frame = |imports: &mut Imports| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                actions = show(
                    ui.ctx(),
                    &theme.palette(),
                    imports,
                    &mut prefs,
                    &world,
                    &crate::ui::inventory::InventoryPreferences::default(),
                );
            },
        );
        output.textures_delta.clear();
        drop(output);
        actions
    };
    assert!(matches!(frame(&mut imports).as_slice(), [Action::Files]));
    assert!(frame(&mut imports).is_empty());
    imports.open(Choice::Animation);
    assert!(frame(&mut imports).is_empty());
}

#[test]
fn mesh_error_details_are_explained_and_uncertain_transfers_keep_the_inventory_warning() {
    let rejection = aurora_net::inventory::upload::Rejection {
        status: Some(400),
        identifier: "MAV_FOUND_DEGENERATE_TRIANGLES".into(),
        messages: vec!["MAV_FOUND_DEGENERATE_TRIANGLES".into()],
    };
    let refused = error(Error::Refused(rejection.clone()));
    assert!(refused.contains("400"));
    assert!(refused.contains(&rejection.identifier));
    let explanation = aurora_i18n::tr!("import-mesh-error-degenerate");
    assert_eq!(refused.matches(explanation).count(), 1, "validation reason explained once");
    let uncertain = error(Error::Uncertain(rejection));
    assert!(uncertain.starts_with(aurora_i18n::tr!("import-unknown-outcome")));
    assert!(uncertain.contains(explanation));
    let missing = error(Error::Refused(aurora_net::inventory::upload::Rejection {
        status: Some(403),
        identifier: String::new(),
        messages: Vec::new(),
    }));
    assert!(missing.contains(aurora_i18n::tr!("import-error-no-details")));
}

#[test]
fn clicking_an_animation_selects_and_restarts_its_preview() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut prefs = Preferences {
        preview_on_avatar: true,
        ..Default::default()
    };
    let mut imports = Imports {
        open: true,
        choice: Choice::Animation,
        ..Default::default()
    };
    let animation = Arc::new(Animation {
        base_priority: 2,
        duration: 2.0,
        emote_name: String::new(),
        loop_in: 0.0,
        loop_out: 2.0,
        looping: false,
        ease_in: 0.3,
        ease_out: 0.3,
        hand_pose: 0,
        joints: Vec::new(),
    });
    for name in ["first", "second"] {
        let mut entry = Entry::new(
            PathBuf::from(format!("{name}.anim")),
            Ok(Source::Animation {
                bytes: Arc::new(Vec::new()),
                animation: animation.clone(),
                bvh: None,
            }),
            prefs.texture(),
        );
        entry.player.started = true;
        entry.player.position = 2.0;
        imports.entries.push(entry);
    }
    imports.active = Some(imports.entries[0].id);
    let target = imports.entries[1].id;
    let mut position = None;
    let mut frame = |events: Vec<egui::Event>, imports: &mut Imports| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                actions = show(
                    ui.ctx(),
                    &theme.palette(),
                    imports,
                    &mut prefs,
                    &world,
                    &crate::ui::inventory::InventoryPreferences::default(),
                );
            },
        );
        output.textures_delta.clear();
        (output, actions)
    };
    for frame_index in 0..3 {
        let (output, actions) = frame(Vec::new(), &mut imports);
        assert!(actions.is_empty());
        if frame_index == 2 {
            let location = |label: &str| {
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                        _ => None,
                    })
                    .next_back()
                    .expect("transport control visible")
            };
            let pause = location(aurora_i18n::tr!("import-pause"));
            let avatar = location(aurora_i18n::tr!("import-preview-own-avatar"));
            assert!(
                avatar.x > pause.x && (avatar.y - pause.y).abs() < 2.0,
                "common avatar option belongs to the playback row"
            );
        }
        position = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == "second" => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .or(position);
    }
    let pos = position.expect("second animation row visible");
    // Both switching to a finished preview and clicking its selected row replay it.
    for _ in 0..2 {
        let mut actions = Vec::new();
        for pressed in [true, false] {
            let (output, next) = frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut imports,
            );
            drop(output);
            actions.extend(next);
        }
        assert_eq!(imports.active, Some(target));
        assert!(matches!(actions.as_slice(), [Action::Play(id, true)] if *id == target));
    }
    drop(frame);
    assert!(prefs.preview_on_avatar, "selecting another animation keeps the common preference");
}

#[test]
fn popup_rows_keep_their_rectangles_when_hover_changes() {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    let mut selected = 0;
    let mut rectangles: Vec<egui::Rect> = Vec::new();
    for hover in [None, Some(0), Some(1), Some(2), None] {
        for _ in 0..3 {
            let pointer = hover.map(|index| rectangles[index].center());
            let mut current = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                    events: pointer.map_or_else(Vec::new, |pos| vec![egui::Event::PointerMoved(pos)]),
                    ..Default::default()
                },
                |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    ui.set_width(250.0);
                    for (index, label) in ["Convex hull only", "Medium", "Bounding box"].into_iter().enumerate() {
                        current.push(selection(ui, &mut selected, index, label).rect);
                    }
                },
            );
            output.textures_delta.clear();
            drop(output);
            if !rectangles.is_empty() {
                assert_eq!(current, rectangles);
            }
            rectangles = current;
        }
    }
    assert_eq!(selected, 0);
}
