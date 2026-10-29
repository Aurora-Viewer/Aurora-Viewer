use super::*;

#[test]
fn texture_size_switch_updates_both_axes_preview_and_price() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let pixels = crate::imports::tests::image(2048, 1024);
    let entry = Entry::new(
        "large.png".into(),
        Ok(Source::Texture(pixels.clone())),
        import_texture::Options {
            max_width: 2048,
            max_height: 2048,
            ..Default::default()
        },
    );
    let id = entry.id;
    let mut imports = Imports {
        open: true,
        choice: Choice::Texture,
        active: Some(id),
        entries: vec![entry],
        ..Default::default()
    };
    let prices = Prices {
        texture: Some(10),
        large_texture: Some(50),
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let frame = |imports: &mut Imports, prefs: &mut Preferences, events| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| actions = show(ui.ctx(), &theme.palette(), imports, prefs, &world, &Default::default()),
        );
        output.textures_delta.clear();
        (output, actions)
    };
    for _ in 0..8 {
        drop(frame(&mut imports, &mut prefs, Vec::new()));
    }
    for (limit, dimensions, cost) in [(true, (1024, 512), 10), (false, (2048, 1024), 50), (true, (1024, 512), 10)] {
        let (output, _) = frame(&mut imports, &mut prefs, Vec::new());
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-max-size") => {
                    Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center())
                }
                _ => None,
            })
            .expect("one size-limit switch");
        let generation = imports.entries[0].preview_generation;
        let mut preview_requested = false;
        for pressed in [true, false] {
            let (_, actions) = frame(
                &mut imports,
                &mut prefs,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            preview_requested |= actions
                .iter()
                .any(|action| matches!(action, Action::Preview(entry) if *entry == id));
        }
        let options = imports.entries[0].options;
        let maximum = if limit { 1024 } else { 2048 };
        assert_eq!((options.max_width, options.max_height), (maximum, maximum));
        assert_eq!(import_texture::dimensions(pixels.width, pixels.height, options), dimensions);
        assert_eq!(imports.total(&prices), Some(cost));
        assert_eq!(prefs.limit_texture_size, limit);
        assert!(preview_requested && imports.entries[0].preview_generation > generation);
    }
    for (width, height, visible) in [
        (240, 180, false),
        (1024, 1024, false),
        (1792, 800, false),
        (1793, 800, true),
        (2048, 512, true),
        (512, 2048, true),
    ] {
        imports.entries[0].source = Some(Source::Texture(crate::imports::tests::image(width, height)));
        let (output, _) = frame(&mut imports, &mut prefs, Vec::new());
        let shown = output.shapes.iter().any(|shape| {
            matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-max-size"))
        });
        assert_eq!(
            shown, visible,
            "visibility uses the unrestricted encoded size of {width} × {height}"
        );
        assert!(prefs.limit_texture_size, "viewing a file must not replace the remembered choice");
    }
}

#[test]
fn mesh_inspector_geometry_settles_without_resizing_in_a_short_work_area() {
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for (work_width, work_height, ppp) in [
        (960.0, 520.0, 1.125),
        (1020.0, 580.0, 1.125),
        (1040.0, 600.0, 1.0),
        (901.0, 540.0, 1.125),
        (1023.5, 580.0, 1.125),
        (1040.0, 650.0, 1.0),
        (1040.0, 480.0, 1.125),
    ] {
        let screen = Vec2::new(1280.0, 800.0);
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        ctx.set_pixels_per_point(ppp);
        theme.apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        let mut draft = crate::imports::tests::mesh_fixture();
        draft.prepared = Some(Arc::new(mesh::prepare(&draft).expect("prepared fixture")));
        let entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
        let id = entry.id;
        let mut imports = Imports {
            open: true,
            choice: Choice::Mesh,
            active: Some(id),
            entries: vec![entry],
            ..Default::default()
        };
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("mesh_section", id)), 1usize));
        let mut baseline = None;
        for frame in 0..240 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    crate::ui::layout::set_work_area(
                        ui.ctx(),
                        egui::Rect::from_min_size(egui::pos2(17.333, 40.444), Vec2::new(work_width, work_height)),
                    );
                    show(
                        ui.ctx(),
                        &theme.palette(),
                        &mut imports,
                        &mut Preferences::default(),
                        &world,
                        &Default::default(),
                    );
                },
            );
            output.textures_delta.clear();
            let size = ctx
                .data_mut(|d| d.get_persisted::<Vec2>(egui::Id::unique(Choice::Mesh.window_id()).with("expanded_size")))
                .expect("mesh window size");
            let positions: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && [
                            aurora_i18n::tr!("common-name"),
                            aurora_i18n::tr!("import-mesh"),
                            aurora_i18n::tr!("import-mesh-section-physics"),
                        ]
                        .contains(&text.galley.text())
                    {
                        Some((text.galley.text().to_owned(), text.pos))
                    } else {
                        None
                    }
                })
                .collect();
            if frame >= 120 {
                let viewport = ctx
                    .data(|d| d.get_temp::<(egui::Rect, Vec2)>(egui::Id::unique(("mesh_viewport", id))))
                    .expect("mesh viewport");
                assert!(
                    viewport.1.x <= viewport.0.width().ceil(),
                    "collision controls fit without horizontal scrolling: {viewport:?}"
                );
                let geometry = (size, positions, viewport);
                let expected = baseline.get_or_insert_with(|| geometry.clone());
                assert_eq!(
                    &geometry, expected,
                    "frame {frame}, work {work_width} × {work_height}, pixels per point {ppp}"
                );
            }
        }
    }
}

#[test]
fn settings_only_show_permissions_and_payment_and_sharing_clicks_are_per_type() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut imports = Imports {
        settings_open: true,
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let frame = |imports: &mut Imports, prefs: &mut Preferences, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &theme.palette(), imports, prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    for _ in 0..8 {
        drop(frame(&mut imports, &mut prefs, Vec::new()));
    }
    let output = frame(&mut imports, &mut prefs, Vec::new());
    for label in [
        aurora_i18n::tr!("import-permissions-group"),
        aurora_i18n::tr!("import-permissions-everyone"),
        aurora_i18n::tr!("import-show-payment"),
    ] {
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.text() == label && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())))),
            "visible Settings field: {label}"
        );
    }
    for label in [
        aurora_i18n::tr!("import-max-size"),
        aurora_i18n::tr!("import-lossless"),
        aurora_i18n::tr!("import-preview-own-avatar"),
        aurora_i18n::tr!("import-auto-weights"),
        aurora_i18n::tr!("import-local-settings"),
        aurora_i18n::tr!("import-preview-colors"),
    ] {
        assert!(
            !output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label))
        );
    }
    for (kind, column) in [(Kind::Sound, 3usize), (Kind::Mesh, 4usize)] {
        let pos = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_permission", kind, column))))
            .expect("permission cell")
            .center();
        for pressed in [true, false] {
            drop(frame(
                &mut imports,
                &mut prefs,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
    }
    assert_ne!(prefs.permissions_for(Kind::Sound).group, 0);
    assert_eq!(prefs.permissions_for(Kind::Texture).group, 0);
    assert_ne!(prefs.permissions_for(Kind::Mesh).everyone, 0);
    assert_eq!(prefs.permissions_for(Kind::Sound).everyone, 0);
    assert!(!prefs.share_group && !prefs.everyone_copy, "legacy fallbacks remain unchanged");
}

#[test]
fn mesh_palette_uses_the_shared_picker_and_applies_every_color_without_rebuilding() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let mut draft = crate::imports::tests::mesh_fixture();
    draft.preview_physics = true;
    let mut entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
    entry.model = Some(Arc::new(aurora_render::preview::Model {
        parts: Vec::new(),
        center: glam::Vec3::ZERO,
        radius: 1.0,
    }));
    let id = entry.id;
    let mut imports = Imports {
        open: true,
        demo_started: true,
        choice: Choice::Mesh,
        active: Some(id),
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let frame = |imports: &mut Imports, prefs: &mut Preferences, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &theme.palette(), imports, prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    let events = |pos, pressed| {
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
    for _ in 0..6 {
        drop(frame(&mut imports, &mut prefs, Vec::new()));
    }
    let pos = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("import_preview_palette", id))))
        .expect("palette button")
        .center();
    for pressed in [true, false] {
        drop(frame(&mut imports, &mut prefs, events(pos, pressed)));
    }
    for label in [
        aurora_i18n::tr!("import-preview-background"),
        aurora_i18n::tr!("import-edges"),
        aurora_i18n::tr!("import-preview-physics-fill"),
        aurora_i18n::tr!("import-preview-physics-edges"),
        aurora_i18n::tr!("import-preview-degenerate-fill"),
        aurora_i18n::tr!("import-preview-degenerate-edges"),
    ] {
        for _ in 0..4 {
            drop(frame(&mut imports, &mut prefs, Vec::new()));
        }
        let (swatch, rect) = ctx
            .data(|data| data.get_temp::<(egui::Id, egui::Rect)>(egui::Id::unique(("import_preview_swatch", label))))
            .expect("shared swatch");
        for pressed in [true, false] {
            drop(frame(&mut imports, &mut prefs, events(rect.center(), pressed)));
        }
        for _ in 0..3 {
            drop(frame(&mut imports, &mut prefs, Vec::new()));
        }
        let picker = ctx
            .data(|data| data.get_temp::<widgets::color::ColorPickState>(swatch))
            .unwrap_or_else(|| panic!("common viewer picker remains open for {label}"));
        let sample = picker.field.lerp_inside(Vec2::new(0.65, 0.3));
        for pressed in [true, false] {
            drop(frame(&mut imports, &mut prefs, events(sample, pressed)));
        }
        // Close the current picker before clicking a swatch it may cover.
        let title = egui::AreaState::load(&ctx, egui::Id::unique("import_preview_colors"))
            .expect("palette window")
            .left_top_pos()
            + Vec2::new(80.0, 16.0);
        for pressed in [true, false] {
            drop(frame(&mut imports, &mut prefs, events(title, pressed)));
        }
    }
    let view = &imports.entries[0].view;
    assert_eq!(view.background, prefs.preview_background.expect("background"));
    assert_eq!(view.edge_color, prefs.preview_physics_edges.expect("physics edges"));
    assert_eq!(
        view.material.expect("physics fill").color,
        linear_color(prefs.preview_physics_fill.expect("physics color"))
    );
    assert_eq!(
        view.diagnostic_colors,
        [
            linear_color(prefs.preview_degenerate_fill.expect("diagnostic fill")),
            prefs.preview_degenerate_edges.expect("diagnostic edges")
        ]
    );
    assert!(prefs.preview_edges.is_some());
    assert_eq!(imports.entries[0].preview_generation, 0, "colors update render uniforms only");
}

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
                true,
            );
        },
    );
    thumbnails_output.textures_delta.clear();
    for texture in texture_ids {
        assert!(
            thumbnails_output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == texture && (mesh.calc_bounds().width() - 95.2).abs() < 0.1 && (mesh.calc_bounds().height() - 47.6).abs() < 0.1)),
            "a rectangular map keeps its aspect inside the square thumbnail"
        );
    }
    for _ in 0..3 {
        drop(frame(Vec::new(), &mut imports));
    }
    drop(frame(Vec::new(), &mut imports));
    let reset = ctx
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("material_reset", imports.entries[0].id))))
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
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("material_canvas", imports.entries[0].id))))
        .expect("material canvas");
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
    drop(frame(Vec::new(), &mut imports));
    let pending_canvas = ctx
        .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("material_canvas", imports.entries[0].id))))
        .expect("pending canvas");
    assert_eq!(pending_canvas, original_canvas, "loading a new shape retains the exact canvas");
    assert_eq!(pending_canvas.size(), Vec2::splat(176.0));
    let cards = [0, 2, 3, 1].map(|slot| {
        ctx.data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("material_card", imports.entries[0].id, slot))))
            .expect("map card")
    });
    for pair in cards.windows(2) {
        assert_eq!(pair[0].left(), pair[1].left(), "one column of maps");
        assert!(pair[1].top() > pair[0].bottom());
    }
    drop(frame(
        vec![
            egui::Event::PointerMoved(cards[1].center()),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -200.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        &mut imports,
    ));
    for _ in 0..60 {
        drop(frame(Vec::new(), &mut imports));
    }
    let output = frame(Vec::new(), &mut imports);
    for slot in [0, 2, 3, 1] {
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.text() == map_label(slot) && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())))),
            "all four map headings remain reachable with enlarged thumbnails");
    }
}

#[test]
fn sound_split_and_amplification_controls_fit_inside_the_first_opened_window() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut entry = Entry::new(
        "Track.wav".into(),
        Ok(Source::Sound(Arc::new(Sound {
            source_channels: 1,
            source_bits: 16,
            mono: vec![0.8; 44100 * 6],
        }))),
        Default::default(),
    );
    entry.sound.options = sound::Options {
        gain: 2.0,
        start: 0.25,
        end: Some(5.5),
        segment: Some(2.0),
        ..Default::default()
    };
    let mut imports = Imports {
        open: true,
        choice: Choice::Sound,
        active: Some(entry.id),
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = || {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
                ..Default::default()
            },
            |ui| {
                show(ui.ctx(), &theme.palette(), &mut imports, &mut prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    for _ in 0..60 {
        drop(frame());
    }
    let output = frame();
    for label in [
        aurora_i18n::tr!("import-sound-volume"),
        aurora_i18n::tr!("import-sound-split"),
        aurora_i18n::tr!("import-sound-clipping"),
    ] {
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text)
            if text.galley.text() == label && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())))),
            "sound control must be visible without scrolling: {label}"
        );
    }
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
fn compact_importers_keep_their_size_and_filtered_files_without_permissions() {
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
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
        let window_id = egui::Id::unique(choice.window_id());
        assert_eq!(
            egui::AreaState::load(&ctx, window_id).expect("dedicated window").left_top_pos(),
            egui::pos2(48.0, 80.0),
            "every first-opened importer has the same top-left corner"
        );
        let maximum = inspector::dimensions(choice).0 + Vec2::splat(2.0);
        let size = ctx
            .data_mut(|data| data.get_persisted::<Vec2>(window_id.with("expanded_size")))
            .expect("window size");
        assert!(
            size.x <= maximum.x && size.y <= maximum.y,
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
        assert!(positions[0].y < positions[1].y && positions[1].y < positions[2].y);
        if matches!(choice, Choice::Mesh | Choice::Bulk) {
            for section in [2, 1, 0] {
                ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("mesh_section", active)), section));
                for _ in 0..20 {
                    drop(frame(&mut imports, Vec::new()));
                }
                let size = ctx
                    .data_mut(|data| data.get_persisted::<Vec2>(window_id.with("expanded_size")))
                    .expect("mesh section size");
                assert!(size.x <= maximum.x && size.y <= maximum.y, "mesh section {section} grew: {size:?}");
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
        assert!(
            !output.shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("import-permissions-tab"))
            ),
            "rights are edited only in Settings"
        );
        for (name, file_type) in [("mesh", Choice::Mesh), ("pbr", Choice::Material), ("texture", Choice::Texture)] {
            assert_eq!(
                output
                    .shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == name)),
                choice == Choice::Bulk || choice == file_type,
                "the file list stays scoped in dedicated windows and complete in Multiple"
            );
        }
        imports.open(choice);
        assert_eq!(imports.active, Some(active));
        if choice == Choice::Bulk {
            let output = frame(&mut imports, Vec::new());
            let tab = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == Choice::Texture.label() => Some(text.pos + text.galley.size() * 0.5),
                    _ => None,
                })
                .expect("Texture tab in Multiple");
            for pressed in [true, false] {
                drop(frame(
                    &mut imports,
                    vec![
                        egui::Event::PointerMoved(tab),
                        egui::Event::PointerButton {
                            pos: tab,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                ));
            }
            assert_eq!(imports.choice, Choice::Texture);
            assert_eq!(
                imports.window_choice(),
                Choice::Bulk,
                "Multiple remains the complete window after changing a tab"
            );
            assert!(!imports.choose_files_on_open);
            let output = frame(&mut imports, Vec::new());
            assert!(
                output
                    .shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == Choice::Bulk.label()))
            );
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
            let active = imports.active.expect("active animation");
            let canvas = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_canvas", active))))
                .expect("canvas");
            let reset = ctx
                .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_reset", active))))
                .expect("reset");
            assert!(reset.top() >= canvas.bottom() && reset.height() >= 26.0);
            assert!(
                location(aurora_i18n::tr!("common-play")).y > reset.bottom(),
                "transport stays below the complete reset row"
            );
            let avatar = location(aurora_i18n::tr!("import-preview-own-avatar"));
            assert!(
                avatar.y > pause.y && (avatar.x - pause.x).abs() < 230.0,
                "common avatar option stays under the preview's transport controls"
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
fn custom_animation_model_buttons_choose_and_restore_without_playing_or_uploading() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let animation = super::super::demo::tail_animation();
    let entry = Entry::new(
        "tail.anim".into(),
        Ok(Source::Animation {
            bytes: Arc::new(Vec::new()),
            animation: Arc::new(animation),
            bvh: None,
        }),
        Default::default(),
    );
    let id = entry.id;
    let mut imports = Imports {
        open: true,
        choice: Choice::Animation,
        active: Some(id),
        entries: vec![entry],
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = |events, imports: &mut Imports| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| actions = show(ui.ctx(), &theme.palette(), imports, &mut prefs, &world, &Default::default()),
        );
        output.textures_delta.clear();
        actions
    };
    for _ in 0..3 {
        assert!(frame(Vec::new(), &mut imports).is_empty());
    }
    for (key, enabled) in [("animation_model_choose", true), ("animation_model_reset", false)] {
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique((key, id))))
            .expect("model button");
        let mut actions = Vec::new();
        for pressed in [true, false] {
            actions.extend(frame(
                vec![
                    egui::Event::PointerMoved(rect.center()),
                    egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                &mut imports,
            ));
        }
        if enabled {
            assert!(matches!(actions.as_slice(), [Action::AnimationModel(target)] if *target == id));
        } else {
            assert!(actions.is_empty(), "Standard is disabled until a model is selected or loading");
        }
    }
    imports.entries[0].animation_model.pending = true;
    drop(frame(Vec::new(), &mut imports));
    let rect = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_model_reset", id))))
        .expect("Standard");
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(
            vec![
                egui::Event::PointerMoved(rect.center()),
                egui::Event::PointerButton {
                    pos: rect.center(),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            &mut imports,
        ));
    }
    assert!(matches!(actions.as_slice(), [Action::DefaultAnimationModel(target)] if *target == id));
    assert!(!imports.entries[0].player.started);
    assert!(matches!(imports.entries[0].status, Status::Ready));
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

#[test]
fn the_saved_animation_model_list_removes_each_row_without_selecting_it() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let id = Uuid::new_v4();
    let saved = vec![
        super::super::animation_model::Saved {
            id: Uuid::new_v4(),
            name: "Tail fixture with a long name that must leave room for its removal icon".into(),
            path: "tail.dae".into(),
        },
        super::super::animation_model::Saved {
            id: Uuid::new_v4(),
            name: "Wing fixture".into(),
            path: "wings.glb".into(),
        },
    ];
    let mut model = super::super::animation_model::Selection::default();
    let frame = |events, model: &super::super::animation_model::Selection| {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.set_width(330.0);
                animation_model_controls(ui, &theme.palette(), id, model, &saved, &mut actions);
            },
        );
        output.textures_delta.clear();
        (output, actions)
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
    for _ in 0..3 {
        drop(frame(Vec::new(), &model));
    }
    let list = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_model_list", id))))
        .expect("model list");
    for pressed in [true, false] {
        drop(frame(click(list.center(), pressed), &model));
    }
    let (output, _) = frame(Vec::new(), &model);
    let wing = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "Wing fixture" => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        })
        .expect("saved model visible in the dropdown");
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(click(wing, pressed), &model).1);
    }
    assert!(matches!(actions.as_slice(), [Action::SelectAnimationModel(target, selected)] if *target == id && *selected == saved[1].id));
    model.saved = Some(saved[1].id);
    drop(frame(Vec::new(), &model));
    for pressed in [true, false] {
        drop(frame(click(list.center(), pressed), &model));
    }
    drop(frame(Vec::new(), &model));
    let remove_tail = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_model_remove", id, saved[0].id))))
        .expect("unselected model's removal icon");
    let remove_wing = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_model_remove", id, saved[1].id))))
        .expect("selected model's removal icon");
    let tail_row = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("animation_model_row", id, saved[0].id))))
        .expect("long model name");
    assert!(
        (remove_tail.right() - remove_wing.right()).abs() < 0.1,
        "icons align regardless of name length"
    );
    assert!(tail_row.right() < remove_tail.left(), "names and removal buttons cannot overlap");
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(click(remove_tail.center(), pressed), &model).1);
    }
    assert!(matches!(actions.as_slice(), [Action::RemoveAnimationModel(removed)] if *removed == saved[0].id));
    assert_eq!(model.saved, Some(saved[1].id), "removing another row must not select it");
    let (output, _) = frame(Vec::new(), &model);
    assert!(
        output
            .shapes
            .iter()
            .filter(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Wing fixture"))
            .count()
            >= 2,
        "the list stays open after removing a row"
    );
    let mut actions = Vec::new();
    for pressed in [true, false] {
        actions.extend(frame(click(remove_wing.center(), pressed), &model).1);
    }
    assert!(matches!(actions.as_slice(), [Action::RemoveAnimationModel(selected)] if *selected == saved[1].id));
}

/// The enlarged preview keeps its size, its camera hint wrapped in a narrow
/// window.
#[test]
fn the_enlarged_preview_keeps_its_size() {
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    crate::ui::widgets::steady_tests::assert_window_steady(
        "import_large_preview",
        &[Vec2::new(760.0, 590.0), Vec2::new(420.0, 360.0)],
        aurora_i18n::tr!("import-preview-reset-view"),
        || {
            let draft = crate::imports::tests::mesh_fixture();
            let entry = Entry::new("mesh.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
            Imports {
                open: true,
                preview_zoom: Some((entry.id, None)),
                entries: vec![entry],
                ..Default::default()
            }
        },
        |imports, ctx, p| {
            show(ctx, p, imports, &mut Preferences::default(), &world, &Default::default());
        },
    );
}
