//! Native presentation regressions: actual panel content and pointer selection.
use super::*;
use aurora_net::build::{ObjectProps, perm};
use std::sync::Arc;

#[path = "contents_keyboard_tests.rs"]
mod contents_keyboard_tests;

fn fixture() -> (World, BuildTool, BuildSettings) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(118)).unwrap();
    world.objects.get_mut(idx).unwrap().update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
    let id = world.objects.get(idx).unwrap().full_id;
    let mut tool = BuildTool::default();
    let settings = BuildSettings::default();
    tool.open_build(Tool::Edit);
    tool.click_select(&mut world, &settings, Some(idx), false);
    tool.props.insert(
        id,
        ObjectProps {
            name: "Presentation test".into(),
            owner_id: world.agent_id,
            creator_id: world.agent_id,
            owner_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
            base_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
            ..Default::default()
        },
    );
    (world, tool, settings)
}

fn text_shapes(shape: &egui::Shape, result: &mut Vec<(String, egui::Rect)>) {
    match shape {
        egui::Shape::Text(text) => result.push((text.galley.job.text.clone(), text.galley.rect.translate(text.pos.to_vec2()))),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, result);
            }
        }
        _ => {}
    }
}

fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut body: impl FnMut(&mut egui::Ui)) -> Vec<(String, egui::Rect)> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(298.0, 700.0))),
            events,
            focused: true,
            ..Default::default()
        },
        |ui| {
            ui.set_max_width(298.0);
            body(ui);
        },
    );
    output.textures_delta.clear();
    let mut texts = Vec::new();
    for shape in output.shapes {
        text_shapes(&shape.shape, &mut texts);
    }
    texts
}

#[test]
fn general_and_rights_expose_separate_content_with_the_same_selection() {
    let (mut world, mut tool, _) = fixture();
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let general = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, false));
    let rights = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, true));
    let has = |texts: &[(String, egui::Rect)], label: &str| texts.iter().any(|(text, _)| text == label);
    assert!(has(&general, aurora_i18n::tr!("common-name-2")));
    assert!(has(&general, aurora_i18n::tr!("common-share")));
    assert!(has(&general, aurora_i18n::tr!("common-show-in-search")));
    assert!(!has(&general, aurora_i18n::tr!("build-ui-general-next-owner")));
    assert!(has(&rights, aurora_i18n::tr!("build-ui-general-next-owner")));
    assert!(!has(&rights, aurora_i18n::tr!("common-name-2")));
    assert!(!has(&rights, aurora_i18n::tr!("common-share")));
    assert!(!has(&rights, aurora_i18n::tr!("common-show-in-search")));
}

#[test]
fn transform_actions_leave_the_three_vectors_visible_at_the_top() {
    let (mut world, mut tool, mut settings) = fixture();
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let texts = frame(&ctx, vec![], |ui| {
        object::show_transform(ui, &p, &mut tool, &mut world, &mut settings, Mods::default());
    });
    for title in [
        aurora_i18n::tr!("build-compact-position"),
        aurora_i18n::tr!("build-compact-size"),
        aurora_i18n::tr!("build-compact-rotation"),
    ] {
        let rect = texts
            .iter()
            .find(|(text, _)| text == title)
            .unwrap_or_else(|| panic!("Missing vector: {title}"))
            .1;
        assert!(rect.bottom() < 340.0, "Vector pushed below the action row: {title}: {rect:?}");
    }
}

#[test]
fn transform_copy_and_paste_target_each_vector_independently_of_the_manipulator() {
    for mode in [EditMode::Move, EditMode::Rotate, EditMode::Stretch] {
        let (mut world, mut tool, mut settings) = fixture();
        tool.edit_mode = mode;
        let idx = world.objects.index_of(&tool.selection[0]).unwrap();
        let p = crate::theme::Theme::default().palette();
        let ctx = egui::Context::default();
        let titles = [
            aurora_i18n::tr!("build-compact-position"),
            aurora_i18n::tr!("build-compact-size"),
            aurora_i18n::tr!("build-compact-rotation"),
        ];
        let mut show = |events| {
            let mut right = 0.0;
            let texts = frame(&ctx, events, |ui| {
                right = ui.max_rect().right();
                object::show_transform(ui, &p, &mut tool, &mut world, &mut settings, Mods::default());
            });
            (texts, right)
        };
        let mut rows = show(vec![]);
        for _ in 0..3 {
            rows = show(vec![]);
        }
        // Visible buttons share each vector caption's baseline. The copy
        // button is immediately before Paste at the panel's right edge.
        let points: Vec<_> = titles
            .iter()
            .map(|title| {
                let caption = rows.0.iter().find(|(text, _)| text == title).unwrap().1;
                egui::pos2(rows.1 - 35.0, caption.center().y)
            })
            .collect();
        for point in &points {
            show(vec![egui::Event::PointerMoved(*point)]);
            for pressed in [true, false] {
                show(vec![egui::Event::PointerButton {
                    pos: *point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
        }
        let object = world.objects.get(idx).unwrap();
        assert_eq!(tool.ui.vec_clipboard[0], Some(object.position), "Position copy in {mode:?}");
        assert_eq!(tool.ui.vec_clipboard[1], Some(object.scale), "Size copy in {mode:?}");
        assert!(tool.ui.vec_clipboard[2].is_some(), "Rotation copy in {mode:?}");

        let values = [Vec3::new(14.0, 23.0, 42.0), Vec3::new(0.7, 0.8, 0.9), Vec3::new(15.0, 25.0, 35.0)];
        tool.ui.vec_clipboard = values.map(Some);
        for point in points {
            let point = point + egui::vec2(24.0, 0.0);
            let mut show = |events| {
                frame(&ctx, events, |ui| {
                    object::show_transform(ui, &p, &mut tool, &mut world, &mut settings, Mods::default());
                });
            };
            show(vec![egui::Event::PointerMoved(point)]);
            for pressed in [true, false] {
                show(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
        }
        let object = world.objects.get(idx).unwrap();
        assert!(object.position.abs_diff_eq(values[0], 0.001), "Position paste in {mode:?}");
        assert!(object.scale.abs_diff_eq(values[1], 0.001), "Size paste in {mode:?}");
        let expected = glam::Quat::from_euler(
            glam::EulerRot::ZYX,
            values[2].z.to_radians(),
            values[2].y.to_radians(),
            values[2].x.to_radians(),
        );
        assert!(object.rotation.abs_diff_eq(expected, 0.001), "Rotation paste in {mode:?}");
        assert!(
            !tool
                .ui
                .requests
                .iter()
                .any(|request| matches!(request, super::super::Request::PasteVector(_)))
        );
    }
}

struct Inspector {
    ctx: egui::Context,
    world: World,
    tool: BuildTool,
    settings: BuildSettings,
    scene: crate::scene::Scene,
    media: crate::media::MediaManager,
    frame: u32,
    inventory_surface: Option<egui::Rect>,
}

impl Inspector {
    fn new() -> Self {
        let (world, mut tool, settings) = fixture();
        tool.tab = 1;
        let scene = crate::scene::Scene::new(std::env::temp_dir(), world.avatar_lib.clone());
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        Self {
            ctx,
            world,
            tool,
            settings,
            scene,
            media: Default::default(),
            frame: 0,
            inventory_surface: None,
        }
    }

    fn show(&mut self, mut events: Vec<egui::Event>) -> (egui::Rect, Vec<(String, egui::Rect)>) {
        self.frame += 1;
        let modifiers = events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerButton { modifiers, .. } | egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0))),
                time: Some(self.frame as f64 / 60.0),
                events,
                focused: true,
                ..Default::default()
            },
            |ui| {
                super::super::shortcuts(ui.ctx(), &mut self.tool, &mut self.world, &mut self.settings);
                let mut env = Env {
                    images: &Default::default(),
                    media: &mut self.media,
                    scene: &mut self.scene,
                    zoom_fraction: 0.0,
                    demo: false,
                };
                super::super::show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    &mut self.tool,
                    &mut self.world,
                    &mut self.settings,
                    Mods::default(),
                    &mut env,
                );
            },
        );
        output.textures_delta.clear();
        fn inventory_surface(shape: &egui::Shape) -> Option<egui::Rect> {
            match shape {
                egui::Shape::Rect(rect) if rect.fill == crate::theme::Theme::default().palette().field.gamma_multiply(0.72) => {
                    Some(rect.rect)
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(inventory_surface),
                _ => None,
            }
        }
        self.inventory_surface = output.shapes.iter().find_map(|shape| inventory_surface(&shape.shape));
        let mut texts = Vec::new();
        for shape in output.shapes {
            text_shapes(&shape.shape, &mut texts);
        }
        let rect = self.ctx.memory(|m| m.area_rect(egui::Id::new(super::super::INSPECTOR_ID)).unwrap());
        (rect, texts)
    }
}

#[test]
fn coordinate_menu_is_available_beside_linked_parts_in_every_edit_section() {
    let mut inspector = Inspector::new();
    let click = |inspector: &mut Inspector, point| {
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
    };
    for section in [6, 1, 5, 3, 2, 0, 4] {
        inspector.tool.tab = section;
        for _ in 0..3 {
            inspector.show(vec![]);
        }
        for (mode, label) in [
            (1, aurora_i18n::tr!("build-compact-local")),
            (2, aurora_i18n::tr!("build-compact-reference")),
            (0, aurora_i18n::tr!("build-compact-world")),
        ] {
            let (rect, texts) = inspector.show(vec![]);
            let linked = texts
                .iter()
                .find(|(text, _)| text == aurora_i18n::tr!("build-compact-linked"))
                .unwrap()
                .1;
            let point = egui::pos2(rect.right() - 19.0, linked.center().y);
            let face = texts
                .iter()
                .find(|(text, _)| text == aurora_i18n::tr!("build-compact-select-face"))
                .unwrap()
                .1;
            assert!(
                face.top() > linked.bottom(),
                "Face selection must leave room for the coordinate icon"
            );
            assert!(linked.right() < point.x - 11.0, "Linked editing overlaps the coordinate icon");
            click(&mut inspector, point);
            let (_, texts) = inspector.show(vec![]);
            for choice in [
                aurora_i18n::tr!("build-compact-world"),
                aurora_i18n::tr!("build-compact-local"),
                aurora_i18n::tr!("build-compact-reference"),
            ] {
                assert!(
                    texts.iter().any(|(text, _)| text == choice),
                    "Missing coordinate choice in section {section}"
                );
            }
            let item = texts.iter().find(|(text, _)| text == label).unwrap().1;
            click(&mut inspector, item.center());
            assert_eq!(inspector.settings.grid_mode, mode);
            if mode == 2 {
                assert_eq!(inspector.tool.grid_objects, inspector.tool.selection);
            }
            let (_, texts) = inspector.show(vec![]);
            assert!(!texts.iter().any(|(text, _)| text == label), "Coordinate menu did not close");
        }
    }
}

#[test]
fn empty_legacy_texture_channels_can_be_selected_and_their_scale_edited() {
    let mut inspector = Inspector::new();
    inspector.tool.tab = 3;
    inspector.tool.ui.tex_tab = 0;
    for _ in 0..4 {
        inspector.show(vec![]);
    }
    let click = |inspector: &mut Inspector, point| {
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
    };
    for (channel, label) in [
        (1, aurora_i18n::tr!("build-ui-texture-normal")),
        (2, aurora_i18n::tr!("build-ui-texture-specular")),
    ] {
        let (_, texts) = inspector.show(vec![]);
        assert!(texts.iter().any(|(text, _)| text == "Blinn-Phong"));
        let tab = texts
            .iter()
            .filter(|(text, _)| text == label)
            .max_by(|a, b| a.1.top().total_cmp(&b.1.top()))
            .unwrap()
            .1;
        click(&mut inspector, tab.center());
        assert_eq!(inspector.tool.ui.bp_channel, channel);
        let (_, texts) = inspector.show(vec![]);
        let scale = texts.iter().find(|(text, _)| text == aurora_i18n::tr!("common-scale")).unwrap().1;
        let field = texts
            .iter()
            .find(|(text, rect)| {
                aurora_i18n::parse_number(text) == Some(1.0)
                    && rect.top() > scale.bottom()
                    && rect.top() < scale.bottom() + 30.0
                    && rect.left() >= scale.left()
                    && rect.left() < scale.left() + 94.0
            })
            .unwrap()
            .1;
        click(&mut inspector, field.center());
        inspector.show(vec![egui::Event::Text("2.500".into())]);
        inspector.show(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        let material = inspector
            .tool
            .ui
            .legacy_pending
            .values()
            .find_map(|(_, material)| material.as_ref())
            .unwrap();
        assert!(material.normal_map.is_nil() && material.specular_map.is_nil());
        assert_eq!(
            if channel == 1 {
                material.normal_st[0]
            } else {
                material.specular_st[0]
            },
            2.5
        );
    }
}

#[test]
fn narrow_grab_panel_keeps_its_height_across_frames_and_wrapped_status_messages() {
    let mut inspector = Inspector::new();
    let mut original = inspector.show(vec![]).0;
    for _ in 0..4 {
        original = inspector.show(vec![]).0;
    }
    for status in [
        "",
        "A longer tool hint that wraps onto several lines in the narrow inspector. "
            .repeat(4)
            .as_str(),
    ] {
        inspector.tool.set_tool(Tool::Grab);
        inspector.tool.status = status.into();
        for frame in 0..90 {
            let (rect, _) = inspector.show(vec![]);
            assert!(
                (rect.size() - original.size()).length() < 1.0,
                "Grab changed the chosen geometry on frame {frame}: {original:?} -> {rect:?}; status={status:?}"
            );
        }
    }
}

#[test]
fn object_contents_inventory_fills_the_panel_and_follows_height_changes_and_filtering() {
    let mut inspector = Inspector::new();
    inspector.tool.tab = 4;
    let object_id = inspector
        .world
        .objects
        .get(inspector.tool.contents_target(&inspector.world).unwrap())
        .unwrap()
        .full_id;
    let items = (0..64)
        .map(|i| aurora_net::build::TaskItem {
            item_id: uuid::Uuid::from_u128(90000 + i),
            name: format!("Entry {i:03}"),
            asset_type: crate::build::contents::AT_LSL_TEXT,
            inv_type: crate::build::contents::IT_LSL,
            owner_id: inspector.world.agent_id,
            owner_mask: perm::COPY | perm::MODIFY | perm::TRANSFER,
            ..Default::default()
        })
        .collect();
    inspector.tool.on_task_inventory(&inspector.world, object_id, Some(0), items);
    let mut original = inspector.show(vec![]);
    for _ in 0..4 {
        original = inspector.show(vec![]);
    }
    let gap_to_footer = |surface: egui::Rect, texts: &[(String, egui::Rect)]| {
        let footer = texts
            .iter()
            .find(|(text, _)| text == aurora_i18n::tr!("build-compact-linked"))
            .unwrap()
            .1;
        footer.top() - surface.bottom()
    };
    let surface = inspector.inventory_surface.expect("Native object inventory frame");
    assert!(
        gap_to_footer(surface, &original.1) < 30.0,
        "Contents leaves unused space below its list"
    );
    let visible_before = original.1.iter().filter(|(text, _)| text.starts_with("Entry ")).count();
    crate::ui::widgets::size_floater(
        &inspector.ctx,
        super::super::INSPECTOR_ID,
        original.0.size() + egui::vec2(0.0, 140.0),
    );
    let mut enlarged = inspector.show(vec![]);
    for _ in 0..4 {
        enlarged = inspector.show(vec![]);
    }
    let enlarged_surface = inspector.inventory_surface.unwrap();
    assert!(
        enlarged_surface.height() > surface.height() + 120.0,
        "Inventory must use the additional height"
    );
    assert!(gap_to_footer(enlarged_surface, &enlarged.1) < 30.0);
    assert!(enlarged.1.iter().filter(|(text, _)| text.starts_with("Entry ")).count() > visible_before);
    inspector.tool.ui.contents_filter = "Entry 063".into();
    let (rect, texts) = inspector.show(vec![]);
    assert!((rect.size() - enlarged.0.size()).length() < 1.0);
    assert!((inspector.inventory_surface.unwrap().height() - enlarged_surface.height()).abs() < 1.0);
    assert_eq!(
        texts
            .iter()
            .filter(|(text, rect)| text.starts_with("Entry ") && enlarged_surface.contains(rect.center()))
            .count(),
        1
    );
    inspector.tool.props.get_mut(&object_id).unwrap().owner_mask &= !perm::MODIFY;
    let (readonly_rect, texts) = inspector.show(vec![]);
    assert!((readonly_rect.size() - enlarged.0.size()).length() < 1.0);
    assert!(
        !texts
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("build-ui-contents-drag-inventory-items-here-to-add-them-modifiable-object")),
        "The removed inventory hint must not appear for read-only objects"
    );
    let readonly_surface = inspector.inventory_surface.unwrap();
    assert!((readonly_surface.height() - enlarged_surface.height()).abs() < 1.0);
    assert!(
        gap_to_footer(readonly_surface, &texts) < 30.0,
        "Read-only contents must also fill the space above the edit footer"
    );
}

#[test]
fn inspector_resize_fills_the_chosen_size_and_preserves_it_across_sections_and_tools() {
    let mut inspector = Inspector::new();
    let mut original = inspector.show(vec![]).0;
    for _ in 0..4 {
        original = inspector.show(vec![]).0;
    }
    let start = original.right_bottom() - Vec2::splat(2.0);
    let target = start + egui::vec2(160.0, 180.0);
    inspector.show(vec![egui::Event::PointerMoved(start)]);
    inspector.show(vec![egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    }]);
    inspector.show(vec![egui::Event::PointerMoved(target)]);
    inspector.show(vec![egui::Event::PointerMoved(target)]);
    inspector.show(vec![egui::Event::PointerButton {
        pos: target,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    }]);
    let (expanded, texts) = inspector.show(vec![]);
    assert!(
        expanded.width() > original.width() + 140.0,
        "Width must follow the corner drag: {original:?} -> {expanded:?}"
    );
    assert!(
        expanded.height() > original.height() + 160.0,
        "Height must follow the corner drag: {original:?} -> {expanded:?}"
    );
    let duplicate = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-duplicate"))
        .unwrap()
        .1;
    assert!(expanded.bottom() - duplicate.bottom() < 30.0, "Footer must follow the bottom edge");
    let uuid = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-uuid"))
        .unwrap()
        .1;
    assert!(expanded.right() - uuid.right() < 30.0, "UUID must follow the right edge");
    for tab in [6, 5, 3, 2, 0, 4, 1] {
        inspector.tool.tab = tab;
        let (rect, texts) = inspector.show(vec![]);
        assert!(
            (rect.size() - expanded.size()).length() < 1.0,
            "Switching section changed the chosen size: {tab}"
        );
        let title = sections().into_iter().find(|(id, _, _)| *id == tab).unwrap().2;
        assert_eq!(
            texts.iter().filter(|(text, _)| text == title).count(),
            1,
            "Duplicate section title: {title}"
        );
        for label in [
            aurora_i18n::tr!("build-compact-both-sides"),
            aurora_i18n::tr!("build-compact-root-axis"),
        ] {
            let matches: Vec<_> = texts.iter().filter(|(text, _)| text == label).collect();
            assert_eq!(matches.len(), 1, "Shared option missing or duplicated: {label}");
            assert!(
                expanded.bottom() - matches[0].1.bottom() < 65.0,
                "Shared option must be in the footer: {label}"
            );
        }
    }
    // Folding the sections must restore the user's size rather than leaving
    // the newly resizable body at its title-bar height.
    for expanding in [false, true] {
        let rect = inspector.show(vec![]).0;
        let point = egui::pos2(rect.right() - 64.0, rect.top() + 20.0);
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        let rect = inspector.show(vec![]).0;
        assert_eq!(inspector.settings.expanded, expanding);
        if expanding {
            assert!((rect.size() - expanded.size()).length() < 1.0);
        } else {
            assert!(rect.height() < 60.0);
            assert!((rect.width() - expanded.width()).abs() < 1.0);
        }
    }
    for tool in [Tool::Focus, Tool::Grab, Tool::Create, Tool::Land] {
        inspector.tool.set_tool(tool);
        let (rect, texts) = inspector.show(vec![]);
        assert!(
            (rect.size() - expanded.size()).length() < 1.0,
            "Switching tool changed the chosen size: {tool:?}"
        );
        assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-duplicate")));
        assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-linked")));
    }
}

#[test]
fn face_selection_can_be_enabled_in_general_and_disabled_in_shape() {
    let (mut world, mut tool, mut settings) = fixture();
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let label = aurora_i18n::tr!("build-compact-select-face");
    for (tab, on) in [(6, true), (5, false)] {
        tool.tab = tab;
        let mut texts = frame(&ctx, vec![], |ui| {
            footer(ui, &p, &mut tool, &mut world, &mut settings, Mods::default())
        });
        if texts.is_empty() {
            texts = frame(&ctx, vec![], |ui| {
                footer(ui, &p, &mut tool, &mut world, &mut settings, Mods::default())
            });
        }
        let point = texts.iter().find(|(text, _)| text == label).unwrap().1.center();
        frame(&ctx, vec![egui::Event::PointerMoved(point)], |ui| {
            footer(ui, &p, &mut tool, &mut world, &mut settings, Mods::default())
        });
        for pressed in [true, false] {
            frame(
                &ctx,
                vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                }],
                |ui| footer(ui, &p, &mut tool, &mut world, &mut settings, Mods::default()),
            );
        }
        assert_eq!(tool.edit_mode == EditMode::Face, on);
        if on {
            tool.faces.insert(tool.selection[0], vec![0]);
        } else {
            assert!(tool.faces.is_empty());
        }
    }
}
