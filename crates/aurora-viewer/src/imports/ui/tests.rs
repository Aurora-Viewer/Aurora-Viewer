use super::*;

#[test]
fn an_empty_batch_cannot_be_confirmed_when_local_previews_are_selected() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.balance = Some(1000);
    let source = Source::Texture(Arc::new(aurora_assets::DecodedImage {
        width: 4,
        height: 4,
        components: 4,
        discard: 0,
        data: vec![255; 64],
    }));
    let mut local = Entry::new("local.png".into(), Ok(source.clone()), Default::default());
    local.local = true;
    let mut imports = Imports {
        open: true,
        choice: Choice::Bulk,
        demo_started: true,
        entries: vec![local],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = |events, imports: &mut Imports| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &theme.palette(), imports, &mut prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    for has_upload in [false, true] {
        if has_upload {
            imports
                .entries
                .push(Entry::new("upload.png".into(), Ok(source.clone()), Default::default()));
        }
        for _ in 0..4 {
            drop(frame(Vec::new(), &mut imports));
        }
        let output = frame(Vec::new(), &mut imports);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.text() == aurora_i18n::tr!("import-upload")
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("upload button");
        for pressed in [true, false] {
            drop(frame(
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
            ));
        }
        assert_eq!(imports.confirm, has_upload);
    }
}

#[test]
fn material_shape_selection_thumbnails_and_reset_work_with_real_clicks() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let pixels = aurora_assets::DecodedImage {
        width: 1,
        height: 1,
        components: 4,
        discard: 0,
        data: vec![90, 120, 160, 255],
    };
    let map = files::local_image(pixels, "map.png".into());
    let mut entry = Entry::new(
        "material.gltf".into(),
        Ok(Source::Material {
            value: PbrMaterial {
                alpha_mode: aurora_assets::AlphaMode::Mask,
                ..Default::default()
            },
            images: Box::new(std::array::from_fn(|_| Some(map.clone()))),
        }),
        Default::default(),
    );
    entry.model = Some(Arc::new(aurora_render::preview::Model {
        parts: Vec::new(),
        center: glam::Vec3::ZERO,
        radius: 1.0,
    }));
    entry.view.yaw = 1.8;
    entry.view.pitch = -0.7;
    entry.view.distance = 6.0;
    entry.view.pan = [0.3, -0.2];
    entry.material_previews = std::array::from_fn(|slot| {
        Some((
            map.key,
            ctx.load_texture(
                format!("map_{slot}"),
                egui::ColorImage::filled([2, 1], theme.palette().teal),
                egui::TextureOptions::LINEAR,
            ),
        ))
    });
    let texture_ids = entry
        .material_previews
        .each_ref()
        .map(|preview| preview.as_ref().expect("map").1.id());
    let mut imports = Imports {
        open: true,
        choice: Choice::Material,
        active: Some(entry.id),
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = |events: Vec<egui::Event>, imports: &mut Imports| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &theme.palette(), imports, &mut prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    let position = |output: &egui::FullOutput, label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape
                    && text.galley.text() == label
                {
                    Some(text.pos + text.galley.size() * 0.5)
                } else {
                    None
                }
            })
            .expect("visible control")
    };
    let click = |pos, pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    for _ in 0..4 {
        drop(frame(Vec::new(), &mut imports));
    }
    let mut thumbnails_output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
            ..Default::default()
        },
        |ui| {
            let entry = &mut imports.entries[0];
            let Some(Source::Material { value, images }) = &mut entry.source else {
                panic!("material")
            };
            material_panel::show(
                ui,
                &theme.palette(),
                entry.id,
                value,
                images,
                &mut Vec::new(),
                &mut entry.material_shape,
                &entry.material_previews,
                &mut Preferences::default(),
                entry.model.as_ref(),
                &mut entry.view,
            );
        },
    );
    thumbnails_output.textures_delta.clear();
    for texture in texture_ids {
        assert!(
            thumbnails_output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == texture && (mesh.calc_bounds().width() - 88.0).abs() < 0.1 && (mesh.calc_bounds().height() - 44.0).abs() < 0.1)),
            "a rectangular map keeps its aspect inside the square thumbnail"
        );
    }
    for _ in 0..3 {
        drop(frame(Vec::new(), &mut imports));
    }
    drop(frame(Vec::new(), &mut imports));
    let reset = ctx
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new(("material_reset", imports.entries[0].id))))
        .expect("reset button")
        .center();
    for pressed in [true, false] {
        drop(frame(click(reset, pressed), &mut imports));
    }
    let view = &imports.entries[0].view;
    let default = aurora_render::preview::View::default();
    assert_eq!(
        (view.yaw, view.pitch, view.distance, view.pan),
        (default.yaw, default.pitch, default.distance, default.pan)
    );
    let output = frame(Vec::new(), &mut imports);
    let original_canvas = ctx
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new(("material_canvas", imports.entries[0].id))))
        .expect("material canvas");
    let original_hint = position(&output, aurora_i18n::tr!("import-preview-gestures"));
    let sphere = position(&output, aurora_i18n::tr!("import-preview-sphere"));
    for pressed in [true, false] {
        drop(frame(click(sphere, pressed), &mut imports));
    }
    let output = frame(Vec::new(), &mut imports);
    let cube = position(&output, aurora_i18n::tr!("import-preview-cube"));
    for pressed in [true, false] {
        drop(frame(click(cube, pressed), &mut imports));
    }
    assert!(imports.entries[0].material_shape == preview::MaterialShape::Cube);
    assert_eq!(imports.entries[0].preview_generation, 1);
    assert!(imports.entries[0].model.is_none());
    let pending = frame(Vec::new(), &mut imports);
    let pending_canvas = ctx
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new(("material_canvas", imports.entries[0].id))))
        .expect("pending canvas");
    assert_eq!(pending_canvas, original_canvas, "loading a new shape retains the exact canvas");
    assert_eq!(
        position(&pending, aurora_i18n::tr!("import-preview-gestures")),
        original_hint,
        "gestures remain under the canvas while it is loading"
    );
    let cards = [0, 2, 3, 1].map(|slot| {
        ctx.data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::new(("material_card", imports.entries[0].id, slot))))
            .expect("map card")
    });
    assert_eq!(cards[0].top(), cards[1].top());
    assert_eq!(cards[2].top(), cards[3].top());
    assert!(cards[1].left() > cards[0].right() && cards[2].top() > cards[0].bottom());
    assert!(
        cards[3].bottom() <= 750.0,
        "all four maps fit below the shared metadata without scrolling"
    );
}

#[test]
fn import_footer_distinguishes_connection_mesh_access_quotes_and_missing_prices() {
    let account = Uuid::from_u128(40);
    let mut mesh_entry = Entry::new(
        "mesh.dae".into(),
        Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
        Default::default(),
    );
    let request = Uuid::new_v4();
    let Some(Source::Mesh(draft)) = &mut mesh_entry.source else {
        panic!("mesh")
    };
    draft.pending = Some((request, 0));
    let texture = Entry::new(
        "image.png".into(),
        Ok(Source::Texture(Arc::new(DecodedImage {
            width: 1,
            height: 1,
            components: 3,
            data: vec![255; 3],
            discard: 0,
        }))),
        Default::default(),
    );
    let mut imports = Imports {
        choice: Choice::Bulk,
        entries: vec![mesh_entry, texture],
        mesh_permission_account: account,
        ..Default::default()
    };
    let prices = Prices {
        texture: Some(10),
        ..Default::default()
    };
    assert_eq!(
        price_label(&imports, &prices, false, account),
        aurora_i18n::tr!("import-price-unavailable")
    );
    assert_eq!(
        price_label(&imports, &prices, true, account),
        aurora_i18n::tr!("import-mesh-fee-required")
    );
    imports.mesh_quoted(request, Err(Error::MeshPermission));
    assert!(mesh_blocked(&imports, true, account));
    assert_eq!(
        price_label(&imports, &prices, true, account),
        aurora_i18n::tr!("import-mesh-account-denied")
    );
    assert!(
        !mesh_blocked(&imports, true, Uuid::from_u128(41)),
        "another account does not inherit a previous refusal"
    );
    imports.entries[0].selected = false;
    assert!(
        !mesh_blocked(&imports, true, account),
        "an unselected mesh cannot block texture imports"
    );
    assert_eq!(
        price_label(&imports, &prices, true, account),
        aurora_i18n::tr!("import-total", cost = 10)
    );
    assert_eq!(
        price_label(&imports, &Prices::default(), true, account),
        aurora_i18n::tr!("import-price-pending")
    );
    imports.entries[1].status = Status::Error("bad pixels".into());
    assert_eq!(
        price_label(&imports, &prices, true, account),
        aurora_i18n::tr!("import-fix-errors-before-upload")
    );
}

#[test]
fn mixed_batch_exposes_the_mesh_fee_button() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let entry = Entry::new(
        "mesh.dae".into(),
        Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
        Default::default(),
    );
    let id = entry.id;
    let mut imports = Imports {
        open: true,
        choice: Choice::Bulk,
        active: Some(id),
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = |events| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                actions = show(ui.ctx(), &theme.palette(), &mut imports, &mut prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        (output, actions)
    };
    for _ in 0..4 {
        drop(frame(Vec::new()));
    }
    let (output, _) = frame(Vec::new());
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::epaint::Shape::Text(text) = &shape.shape
                && text.galley.text() == aurora_i18n::tr!("import-calculate-fee")
            {
                Some(text.pos + text.galley.size() * 0.5)
            } else {
                None
            }
        })
        .expect("mesh fee button visible in batch mode");
    let (_, actions) = frame(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    assert!(matches!(actions.as_slice(), [Action::Quote(entry)] if *entry == id));
}

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
            let (current, _) = frame(Vec::new());
            let pos = current
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                })
                .expect("local control visible");
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
fn compact_importers_do_not_grow_over_time_and_permissions_keep_the_file_list() {
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut metadata_positions = None;
    for choice in [Choice::Texture, Choice::Mesh, Choice::Material, Choice::Bulk] {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let mut prefs = Preferences::default();
        let mesh = Entry::new(
            "mesh.dae".into(),
            Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
            Default::default(),
        );
        let material = Entry::new(
            "pbr.gltf".into(),
            Ok(Source::Material {
                value: Default::default(),
                images: Box::new(std::array::from_fn(|_| None)),
            }),
            Default::default(),
        );
        let texture = Entry::new(
            "texture.png".into(),
            Ok(Source::Texture(Arc::new(DecodedImage {
                width: 1,
                height: 1,
                components: 3,
                data: vec![255; 3],
                discard: 0,
            }))),
            Default::default(),
        );
        let active = match choice {
            Choice::Texture => texture.id,
            Choice::Material => material.id,
            _ => mesh.id,
        };
        let mut imports = Imports {
            open: true,
            choice,
            active: Some(active),
            entries: vec![mesh, material, texture],
            ..Default::default()
        };
        let mut frame = |imports: &mut Imports, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(ui.ctx(), &theme.palette(), imports, &mut prefs, &world, &Default::default());
                },
            );
            output.textures_delta.clear();
            output
        };
        for _ in 0..60 {
            drop(frame(&mut imports, Vec::new()));
        }
        let size = ctx
            .data_mut(|data| data.get_persisted::<Vec2>(egui::Id::new("asset_imports_v5").with("expanded_size")))
            .expect("window size");
        assert!(
            size.x <= 902.0 && size.y <= 722.0,
            "compact window grew for {}: {size:?}",
            choice.label()
        );
        let output = frame(&mut imports, Vec::new());
        let positions = [
            aurora_i18n::tr!("common-name"),
            aurora_i18n::tr!("common-description"),
            aurora_i18n::tr!("import-destination"),
        ]
        .map(|label| {
            let positions: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .collect();
            assert_eq!(positions.len(), 1, "one shared metadata row for {label}");
            positions[0]
        });
        if let Some(previous) = metadata_positions {
            assert_eq!(positions, previous, "metadata keeps the same layout across asset panels");
        }
        metadata_positions = Some(positions);
        assert!(positions[0].y < positions[1].y && positions[1].y < positions[2].y);
        if matches!(choice, Choice::Mesh | Choice::Bulk) {
            for section in [2, 1, 0] {
                ctx.data_mut(|data| data.insert_temp(egui::Id::new(("mesh_section", active)), section));
                for _ in 0..20 {
                    drop(frame(&mut imports, Vec::new()));
                }
                let size = ctx
                    .data_mut(|data| data.get_persisted::<Vec2>(egui::Id::new("asset_imports_v5").with("expanded_size")))
                    .expect("mesh section size");
                assert!(size.x <= 902.0 && size.y <= 722.0, "mesh section {section} grew: {size:?}");
                let output = frame(&mut imports, Vec::new());
                for (index, label) in [
                    aurora_i18n::tr!("common-name"),
                    aurora_i18n::tr!("common-description"),
                    aurora_i18n::tr!("import-destination"),
                ]
                .into_iter()
                .enumerate()
                {
                    let rows: Vec<_> = output
                        .shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::epaint::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(rows, [positions[index]], "metadata stays above every mesh section");
                }
            }
        }
        let permissions = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-permissions-tab") => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .expect("main Permissions tab");
        for pressed in [true, false] {
            drop(frame(
                &mut imports,
                vec![
                    egui::Event::PointerMoved(permissions),
                    egui::Event::PointerButton {
                        pos: permissions,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
        assert!(imports.permissions_open);
        assert!(imports.choice == choice, "permissions are independent of the asset type");
        assert_eq!(imports.active, Some(active));
        for _ in 0..20 {
            drop(frame(&mut imports, Vec::new()));
        }
        let size = ctx
            .data_mut(|data| data.get_persisted::<Vec2>(egui::Id::new("asset_imports_v5").with("expanded_size")))
            .expect("permissions size");
        assert!(size.x <= 902.0 && size.y <= 722.0, "permissions table grew: {size:?}");
        let output = frame(&mut imports, Vec::new());
        for name in ["mesh", "pbr"] {
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text().starts_with(name))),
                "all files remain selectable in Permissions"
            );
        }
        imports.open(choice);
        assert!(!imports.permissions_open);
        assert_eq!(imports.active, Some(active));
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
    imports.open_file_picker(Choice::Bulk);
    assert_eq!(imports.choice, Choice::Bulk);
    assert!(matches!(frame(&mut imports).as_slice(), [Action::Files]));
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
