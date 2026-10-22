use super::*;

#[test]
fn import_launcher_defaults_to_the_top_midpoint_and_keeps_custom_placement() {
    let ctx = egui::Context::default();
    let mut imports = Imports {
        launcher_open: true,
        ..Default::default()
    };
    let run = |ctx: &egui::Context, imports: &mut Imports, size| {
        let theme = crate::theme::Theme::default();
        theme.apply(ctx, 1.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                let screen = ui.ctx().content_rect();
                crate::ui::layout::set_work_area(ui.ctx(), crate::ui::layout::default_work_area(screen));
                crate::ui::layout::set_move_area(ui.ctx(), screen);
                crate::ui::layout::frame(ui.ctx(), &theme.palette(), false);
                show(ui.ctx(), &theme.palette(), imports, &mut Preferences::default());
            },
        );
        output.textures_delta.clear();
    };
    for size in [Vec2::new(1280.0, 720.0), Vec2::new(1600.0, 900.0), Vec2::new(1000.0, 700.0)] {
        for _ in 0..4 {
            run(&ctx, &mut imports, size);
        }
        let bar = bounds(&ctx, WINDOW).expect("measured launcher");
        assert!((bar.center().x - size.x * 0.5).abs() < 0.5, "top midpoint: {bar:?}");
        assert_eq!(bar.top(), crate::ui::layout::work_area(&ctx).top());
    }
    let size = Vec2::new(1600.0, 900.0);
    let custom = egui::pos2(80.0, 180.0);
    widgets::move_floater(&ctx, WINDOW, custom);
    for _ in 0..4 {
        run(&ctx, &mut imports, size);
    }
    assert_eq!(bounds(&ctx, WINDOW).expect("moved launcher").min, custom);
    imports.launcher_open = false;
    for _ in 0..3 {
        run(&ctx, &mut imports, size);
    }
    imports.open_launcher(&ctx);
    for _ in 0..4 {
        run(&ctx, &mut imports, size);
    }
    assert_eq!(bounds(&ctx, WINDOW).expect("reopened launcher").min, custom);
    let saved = ron::to_string(&ctx.memory(|m| m.clone())).expect("saved custom placement");
    let next = egui::Context::default();
    next.memory_mut(|m| *m = ron::from_str(&saved).expect("restored placement"));
    for _ in 0..4 {
        run(&next, &mut imports, Vec2::new(1920.0, 1080.0));
    }
    assert_eq!(bounds(&next, WINDOW).expect("custom placement after restart").min, custom);
}

fn frame(ctx: &egui::Context, imports: &mut Imports, events: Vec<egui::Event>) -> egui::FullOutput {
    let theme = crate::theme::Theme::default();
    theme.apply(ctx, 1.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
            events,
            ..Default::default()
        },
        |ui| show(ui.ctx(), &theme.palette(), imports, &mut Preferences::default()),
    );
    output.textures_delta.clear();
    output
}

fn rectangles(output: &egui::FullOutput) -> Vec<egui::epaint::RectShape> {
    fn collect(shape: &egui::Shape, out: &mut Vec<egui::epaint::RectShape>) {
        match shape {
            egui::Shape::Rect(rect) => out.push(rect.clone()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    output.shapes.iter().for_each(|shape| collect(&shape.shape, &mut out));
    out
}

fn action_positions(output: &egui::FullOutput) -> Vec<egui::Pos2> {
    let mut positions: Vec<_> = rectangles(output)
        .iter()
        .filter(|shape| shape.stroke.width > 0.0 && (shape.rect.width() - 36.0).abs() < 0.5 && (shape.rect.height() - 36.0).abs() < 0.5)
        .map(|shape| shape.rect.center())
        .collect();
    positions.sort_by(|left, right| left.x.total_cmp(&right.x));
    positions.dedup_by(|left, right| left.distance(*right) < 0.5);
    positions
}

fn click(ctx: &egui::Context, imports: &mut Imports, pos: egui::Pos2) {
    for pressed in [true, false] {
        drop(frame(
            ctx,
            imports,
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

#[test]
fn import_launcher_keeps_all_types_on_one_row_and_defers_the_picker_until_click() {
    for (index, choice) in [
        Choice::Texture,
        Choice::Sound,
        Choice::Animation,
        Choice::Material,
        Choice::Mesh,
        Choice::Bulk,
        Choice::Local,
    ]
    .into_iter()
    .enumerate()
    {
        let ctx = egui::Context::default();
        let mut imports = Imports::default();
        imports.open_launcher(&ctx);
        for _ in 0..4 {
            drop(frame(&ctx, &mut imports, Vec::new()));
        }
        assert!(!imports.open);
        assert!(!imports.choose_files_on_open);
        let output = frame(&ctx, &mut imports, Vec::new());
        let positions = action_positions(&output);
        assert_eq!(positions.len(), 8);
        let first = positions[0];
        let settings = positions[7];
        assert!((first.y - settings.y).abs() < 1.0);
        assert!(settings.x > first.x);
        assert!(settings.x - first.x < 350.0);
        click(&ctx, &mut imports, positions[index]);
        assert!(imports.launcher_open);
        assert!(imports.open);
        assert!(imports.choose_files_on_open);
        assert_eq!(imports.choice, choice);
        click(&ctx, &mut imports, positions[index]);
        assert!(!imports.open, "a second click closes the displayed type");
        assert!(!imports.choose_files_on_open);
        assert!(imports.launcher_open);
    }
}

#[test]
fn a_listed_file_opens_without_a_picker_and_the_same_type_toggles_closed() {
    let ctx = egui::Context::default();
    let mut imports = Imports::default();
    imports.entries.push(Entry::new(
        "mesh.dae".into(),
        Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
        Default::default(),
    ));
    imports.open_launcher(&ctx);
    for _ in 0..4 {
        drop(frame(&ctx, &mut imports, Vec::new()));
    }
    let positions = action_positions(&frame(&ctx, &mut imports, Vec::new()));
    for open in [true, false, true] {
        click(&ctx, &mut imports, positions[4]);
        assert_eq!(imports.open, open);
        assert!(!imports.choose_files_on_open);
        assert!(imports.launcher_open);
    }
    click(&ctx, &mut imports, positions[1]);
    assert!(
        imports.open && imports.choose_files_on_open,
        "a mesh draft does not suppress the empty Sound picker"
    );
}

#[test]
fn import_launcher_shortcut_toggles_without_opening_a_file_picker() {
    let ctx = egui::Context::default();
    let mut imports = Imports::default();
    for expected in [true, false, true, false] {
        imports.toggle_launcher(&ctx);
        assert_eq!(imports.launcher_open, expected);
        assert!(!imports.open);
        assert!(!imports.choose_files_on_open);
    }
}

#[test]
fn import_launcher_stays_clickable_when_switching_types_with_an_inspector_open() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    widgets::move_floater(&ctx, WINDOW, egui::pos2(240.0, 180.0));
    let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    let mut imports = Imports {
        launcher_open: true,
        demo_mode: "inspectors".into(),
        ..Default::default()
    };
    let mut prefs = Preferences::default();
    let mut frame = |imports: &mut Imports, events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                super::super::show(ui.ctx(), &theme.palette(), imports, &mut prefs, &world, &Default::default());
            },
        );
        output.textures_delta.clear();
        output
    };
    for _ in 0..4 {
        drop(frame(&mut imports, Vec::new()));
    }
    for (index, choice) in [(4, Choice::Mesh), (1, Choice::Sound)] {
        let output = frame(&mut imports, Vec::new());
        let bar = bounds(&ctx, WINDOW).expect("visible launcher");
        let positions: Vec<_> = action_positions(&output).into_iter().filter(|pos| bar.contains(*pos)).collect();
        let pos = positions[index];
        assert_eq!(
            ctx.layer_id_at(pos),
            Some(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)))
        );
        for pressed in [true, false] {
            drop(frame(
                &mut imports,
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
        for _ in 0..4 {
            drop(frame(&mut imports, Vec::new()));
        }
        assert!(imports.launcher_open);
        assert!(imports.open);
        assert_eq!(imports.window_choice(), choice);
        let bar = bounds(&ctx, WINDOW).expect("launcher still displayed");
        let settings = bar.right_center() - Vec2::new(44.0, 0.0);
        assert_eq!(
            ctx.layer_id_at(settings),
            Some(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW))),
            "the last icons stay reachable on a narrow viewport"
        );
    }
    imports.toggle_launcher(&ctx);
    assert!(!imports.launcher_open);
    assert!(imports.open);
}

#[test]
fn import_launcher_settings_and_escape_do_not_open_a_file_picker() {
    let ctx = egui::Context::default();
    let mut imports = Imports::default();
    imports.open_launcher(&ctx);
    for _ in 0..4 {
        drop(frame(&ctx, &mut imports, Vec::new()));
    }
    let output = frame(&ctx, &mut imports, Vec::new());
    let settings_pos = action_positions(&output)[7];
    click(&ctx, &mut imports, settings_pos);
    assert!(imports.settings_open);
    assert!(imports.launcher_open);
    assert!(!imports.open);
    assert!(!imports.choose_files_on_open);
    for _ in 0..4 {
        drop(frame(&ctx, &mut imports, Vec::new()));
    }
    click(&ctx, &mut imports, settings_pos);
    assert!(!imports.settings_open);
    assert!(imports.launcher_open);
    click(&ctx, &mut imports, settings_pos);
    assert!(imports.settings_open);
    widgets::minimize_floater(&ctx, SETTINGS);
    drop(frame(&ctx, &mut imports, Vec::new()));
    click(&ctx, &mut imports, settings_pos);
    assert!(!imports.settings_open);
    click(&ctx, &mut imports, settings_pos);
    assert!(imports.settings_open);
    assert!(!widgets::floater_minimized(&ctx, SETTINGS));
    for settings_was_open in [true, false] {
        drop(frame(
            &ctx,
            &mut imports,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ));
        assert!(!imports.settings_open);
        assert_eq!(imports.launcher_open, settings_was_open);
        drop(frame(
            &ctx,
            &mut imports,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ));
    }
    assert!(!imports.choose_files_on_open);
}

#[test]
fn import_launcher_has_no_title_or_shortcut_and_its_right_cross_closes_it() {
    let ctx = egui::Context::default();
    let mut imports = Imports::default();
    imports.open_launcher(&ctx);
    for _ in 0..4 {
        drop(frame(&ctx, &mut imports, Vec::new()));
    }
    let output = frame(&ctx, &mut imports, Vec::new());
    let visible_text: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                (!text.galley.text().is_empty()).then_some(text.galley.text())
            } else {
                None
            }
        })
        .collect();
    assert!(visible_text.is_empty(), "unexpected toolbar text: {visible_text:?}");
    let icons = action_positions(&output);
    assert_eq!(icons.len(), 8);
    let close = output
        .shapes
        .iter()
        .find_map(|shape| {
            if let egui::Shape::LineSegment { points, .. } = &shape.shape
                && (points[0].x - points[1].x).abs() == 8.0
                && (points[0].y - points[1].y).abs() == 8.0
            {
                Some(egui::pos2((points[0].x + points[1].x) * 0.5, (points[0].y + points[1].y) * 0.5))
            } else {
                None
            }
        })
        .expect("plain close cross");
    assert!(close.x > icons[7].x);
    assert!((close.y - icons[7].y).abs() < 1.0);
    let bar = egui::AreaState::load(&ctx, egui::Id::unique(WINDOW)).expect("toolbar area");
    assert!(bar.size.expect("measured size").y <= 40.0);
    assert!(
        !rectangles(&output)
            .iter()
            .any(|shape| shape.rect.width() > 90.0 && shape.rect.height() <= 56.0),
        "no common bar background"
    );
    click(&ctx, &mut imports, close);
    assert!(!imports.launcher_open);
    assert!(!imports.choose_files_on_open);
}
