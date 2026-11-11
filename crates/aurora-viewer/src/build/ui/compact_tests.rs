//! Native presentation regressions: actual panel content and pointer selection.
use super::*;
use aurora_net::build::{ObjectProps, perm};
use egui::vec2;
use std::sync::Arc;

#[path = "contents_keyboard_tests.rs"]
mod contents_keyboard_tests;
#[path = "layout_tests.rs"]
mod layout_tests;
#[path = "legacy_tests.rs"]
mod legacy_tests;

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
            object_id: id,
            full: true,
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

#[test]
fn ownership_stays_left_of_every_build_permission_state() {
    let mut inspector = Inspector::new();
    let p = crate::theme::Theme::default().palette();
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let id = inspector.world.objects.get(idx).unwrap().full_id;
    let other = uuid::Uuid::from_u128(0x1234);
    inspector.world.social.names.insert(other, "Status Owner".into());
    for owner in [inspector.world.agent_id, other] {
        inspector.tool.props.get_mut(&id).unwrap().owner_id = owner;
        let name = if owner == other {
            "Status Owner"
        } else {
            aurora_i18n::tr!("build-status-self")
        };
        for (modifiable, locked) in [(true, false), (false, false), (true, true), (false, true)] {
            let flags = &mut inspector.world.objects.get_mut(idx).unwrap().update_flags;
            *flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            if modifiable {
                *flags |= crate::ui::context::flags::OBJECT_MODIFY;
            }
            inspector.tool.props.get_mut(&id).unwrap().owner_mask = if locked { !perm::MOVE } else { u32::MAX };
            let states = object_status(&inspector.tool, &mut inspector.world, &p);
            assert_eq!(states[0].1, name);
            assert_eq!(states.iter().any(|(icon, _, _)| *icon == "check"), modifiable && !locked);
            for _ in 0..3 {
                inspector.show(Vec::new());
            }
            let (window, texts) = inspector.show(Vec::new());
            let owner_rect = texts.iter().find(|(text, _)| text == name).unwrap().1;
            for (_, status, _) in &states[1..] {
                let status_rect = texts.iter().find(|(text, _)| text == status).unwrap().1;
                assert!(owner_rect.right() < status_rect.left(), "Owner must precede {status}");
                assert!((owner_rect.center().y - status_rect.center().y).abs() < 1.0);
                assert!(window.contains_rect(status_rect));
            }
        }
    }
}

#[test]
fn build_ownership_resolves_names_and_distinguishes_groups_and_multiple_owners() {
    let (mut world, mut tool, settings) = fixture();
    let p = crate::theme::Theme::default().palette();
    let idx = tool.sel_prims(&world)[0];
    let id = world.objects.get(idx).unwrap().full_id;
    let owner = uuid::Uuid::from_u128(0x1234);
    world.objects.get_mut(idx).unwrap().owner_id = owner;
    tool.props.remove(&id);
    assert_eq!(object_status(&tool, &mut world, &p)[0].1, "…");
    assert!(world.social.avatar_names.take_asks().contains(&owner));
    world.social.names.insert(owner, "Resolved Owner".into());
    assert_eq!(object_status(&tool, &mut world, &p)[0].1, "Resolved Owner");
    tool.props.insert(
        id,
        ObjectProps {
            owner_id: owner,
            group_id: owner,
            ..Default::default()
        },
    );
    world.land.group_names.insert(owner, "Owner Group".into());
    let status = object_status(&tool, &mut world, &p);
    assert_eq!((status[0].0, status[0].1.as_str()), ("users-group", "Owner Group"));
    let second = world.objects.index_of_uuid(&crate::demo::action_id(119)).unwrap();
    tool.click_select(&mut world, &settings, Some(second), true);
    assert_eq!(object_status(&tool, &mut world, &p)[0].1, aurora_i18n::tr!("status-multiple"));
    tool.deselect_all(&world);
    assert!(object_status(&tool, &mut world, &p).is_empty());
}

#[test]
fn reflection_probe_selection_toggle_is_available_without_selection_in_every_inspector_section() {
    let mut inspector = Inspector::new();
    inspector.tool.ui.layout.open = true;
    inspector.tool.deselect_all(&inspector.world);
    for tab in 0..7 {
        inspector.tool.tab = tab;
        for _ in 0..4 {
            inspector.show(Vec::new());
        }
        let (window, texts) = inspector.show(Vec::new());
        let label = aurora_i18n::tr!("build-compact-probes");
        let matches: Vec<_> = texts.iter().filter(|(text, _)| text == label).collect();
        assert_eq!(matches.len(), 1, "Exactly one probe selection toggle in section {tab}");
        let rect = matches[0].1;
        let settings = inspector
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("construction-settings")))
            .unwrap();
        assert!(settings.contains_rect(rect));
        assert!(!window.intersects(rect));
        let before = inspector.settings.select_probes;
        inspector.show(vec![egui::Event::PointerMoved(rect.center())]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        assert_eq!(inspector.settings.select_probes, !before, "Toggle in section {tab}");
        assert!(inspector.tool.selection.is_empty());
    }
}

#[test]
fn mesh_inspector_is_interactive_without_modify_and_selection_numbers_stay_in_footer() {
    let mut inspector = Inspector::new();
    inspector.settings.transform_expanded = false;
    resize_inspector(&mut inspector, vec2(584.0, 820.0));
    let idx = inspector.world.objects.index_of(inspector.tool.selection.first().unwrap()).unwrap();
    let object = inspector.world.objects.get_mut(idx).unwrap();
    object.volume.sculpt = Some(aurora_prim::params::SculptParams {
        texture: crate::demo::FACE_SELECTION_MESH,
        sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
    });
    object.extra.sculpt = object.volume.sculpt;
    object.update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
    let key = object.key;
    let id = object.full_id;
    inspector.tool.props.get_mut(&id).unwrap().owner_mask &= !perm::MODIFY;
    assert!(!inspector.tool.can_modify(&inspector.world, idx));
    inspector.tool.tab = 5;
    inspector.tool.face_counts.insert(id, 6);
    inspector.tool.sync_mesh_inspector(&mut inspector.world, &mut inspector.scene);
    for _ in 0..5 {
        inspector.show(Vec::new());
    }
    let (rect, texts) = inspector.show(Vec::new());
    assert!(texts.iter().any(|(t, _)| t == aurora_i18n::tr!("build-mesh-triangles")));
    for heading in [
        aurora_i18n::tr!("build-mesh-transition"),
        "LL",
        "Legacy",
        aurora_i18n::tr!("build-mesh-current"),
    ] {
        assert!(
            texts.iter().any(|(t, r)| t == heading && r.height() < 18.0),
            "Distance column must stay on one line: {heading}"
        );
    }
    assert!(
        texts
            .iter()
            .any(|(t, r)| t == aurora_i18n::tr!("build-mesh-distances-note") && r.bottom() < rect.bottom() - 150.0)
    );
    assert!(
        !texts
            .iter()
            .any(|(t, _)| t == aurora_i18n::tr!("build-ui-object-a-mesh-shape-cannot-be-edited-here"))
    );
    let point = texts
        .iter()
        .find(|(t, _)| t == aurora_i18n::tr!("build-mesh-default"))
        .unwrap()
        .1
        .center();
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
    click(&mut inspector, point);
    let (_, menu) = inspector.show(Vec::new());
    let low = menu
        .iter()
        .rev()
        .find(|(t, _)| t == aurora_i18n::tr!("build-mesh-low"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, low);
    assert_eq!(inspector.tool.inspected_mesh_lod(), Some(1));
    inspector.settings.edit_linked = true;
    inspector.tool.edit_mode = EditMode::Face;
    inspector.tool.faces.insert(key, [0, 2, 5].into_iter().collect());
    let (_, texts) = inspector.show(Vec::new());
    assert!(texts.iter().any(|(t, r)| t == "0, 2, 5" && r.top() > rect.center().y));
    assert!(texts.iter().any(|(t, r)| t == "0" && r.top() > rect.center().y));
    for _ in 0..12 {
        let (next, _) = inspector.show(Vec::new());
        // A changed row can snap to the next pixel once; repeated frames must
        // stay within that one point, rather than accumulating window growth.
        assert!(
            (next.width() - rect.width()).abs() <= 1.0 && (next.height() - rect.height()).abs() <= 1.0,
            "before={rect:?}, after={next:?}"
        );
    }
}

fn history_key(key: egui::Key, shift: bool, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            shift,
            ..Default::default()
        },
    }
}

fn history_frame(
    ctx: &egui::Context,
    world: &mut World,
    tool: &mut BuildTool,
    settings: &mut BuildSettings,
    mut events: Vec<egui::Event>,
) -> Vec<egui::Event> {
    // Ctrl can be released before the next rendered frame.
    events.push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let mut remaining = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            super::super::shortcuts(ui.ctx(), tool, world, settings);
            remaining = ui.input(|i| i.events.clone());
        },
    );
    output.textures_delta.clear();
    remaining
}

#[test]
fn duplicate_and_linked_edit_commands_own_their_presses_and_do_not_repeat() {
    let (mut world, mut tool, mut settings) = fixture();
    let ctx = egui::Context::default();
    tool.out.clear();
    let remaining = history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![
            history_key(egui::Key::D, false, false),
            history_key(egui::Key::D, false, true),
            history_key(egui::Key::E, true, false),
            history_key(egui::Key::E, true, true),
        ],
    );
    assert_eq!(
        tool.out
            .iter()
            .filter(|cmd| matches!(cmd, aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Duplicate { .. })))
            .count(),
        1
    );
    assert!(settings.edit_linked, "the repeated press must not toggle linked edit back off");
    assert!(
        !remaining
            .iter()
            .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
    );
}

#[test]
fn build_commands_reserve_logical_keys_and_preserve_text_alt_camera_and_closed_tools() {
    use crate::keybinds::{Action, Binding, Input, KeyBindings, Mods as KeyMods};
    use winit::keyboard::{Key as NativeKey, KeyCode};
    for (letter, code, mods) in [
        (
            "d",
            KeyCode::KeyD,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
        ),
        (
            "e",
            KeyCode::KeyE,
            KeyMods {
                ctrl: true,
                shift: true,
                alt: false,
            },
        ),
        (
            "l",
            KeyCode::KeyL,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
        ),
        (
            "l",
            KeyCode::KeyL,
            KeyMods {
                ctrl: true,
                shift: true,
                alt: false,
            },
        ),
        ("g", KeyCode::KeyG, KeyMods::default()),
        (
            "g",
            KeyCode::KeyG,
            KeyMods {
                shift: true,
                ..Default::default()
            },
        ),
        (
            "x",
            KeyCode::KeyX,
            KeyMods {
                shift: true,
                ..Default::default()
            },
        ),
        (
            "b",
            KeyCode::KeyB,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
        ),
        (
            "ю",
            KeyCode::KeyD,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
        ),
    ] {
        let key = NativeKey::Character(letter.into());
        let input = Input::key(code);
        let mut kb = KeyBindings::default();
        kb.set(Action::Forward, 0, Some(Binding::captured(input.clone(), KeyMods::default())));
        kb.set(Action::CamZoomIn, 0, Some(Binding::captured(input.clone(), KeyMods::default())));
        let mut down = std::collections::HashSet::from([input.clone()]);
        assert!(super::super::Shortcut::take_native_key(&key, code, mods, true, true, &mut down).is_some());
        assert!(down.is_empty());
        assert!(!kb.held(Action::Forward, &down, KeyMods::default()));
        assert!(!kb.held(Action::CamZoomIn, &down, KeyMods::default()));
        // A focused field owns its own editing events, even if build is open.
        assert!(super::super::Shortcut::take_native_key(&key, code, mods, true, false, &mut down).is_none());
        assert!(super::super::Shortcut::take_native_key(&key, code, KeyMods { alt: true, ..mods }, true, true, &mut down).is_none());
    }
    assert!(
        super::super::Shortcut::from_key(
            egui::Key::D,
            KeyMods {
                ctrl: true,
                ..Default::default()
            },
            false
        )
        .is_none()
    );
    assert!(super::super::Shortcut::from_key(egui::Key::G, KeyMods::default(), false).is_none());
}

#[test]
fn history_dispatch_uses_each_event_modifiers_and_supports_repeats_and_both_redo_keys() {
    let (mut world, mut tool, mut settings) = fixture();
    let ctx = egui::Context::default();
    world.groups.active = uuid::Uuid::from_u128(8123);
    let root = tool.roots(&world)[0];
    let object = world.objects.get(root).unwrap();
    let (handle, id) = (object.key.region, object.full_id);
    tool.out.clear();
    let remaining = history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![
            history_key(egui::Key::Z, false, false),
            history_key(egui::Key::Z, false, true),
            history_key(egui::Key::Y, false, false),
            history_key(egui::Key::Y, false, true),
            history_key(egui::Key::Z, true, false),
        ],
    );
    assert!(
        !remaining
            .iter()
            .any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))
    );
    let commands = tool
        .out
        .iter()
        .filter_map(|cmd| match cmd {
            aurora_net::NetCommand::Build(cmd) => Some(cmd),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(commands.len(), 5);
    for (i, cmd) in commands.iter().enumerate() {
        let (h, ids, group_id) = match cmd {
            aurora_net::build::BuildCmd::Undo { handle, ids, group_id } if i < 2 => (handle, ids, group_id),
            aurora_net::build::BuildCmd::Redo { handle, ids, group_id } if i >= 2 => (handle, ids, group_id),
            other => panic!("Unexpected history command: {other:?}"),
        };
        assert_eq!((*h, ids.as_slice(), *group_id), (handle, [id].as_slice(), world.groups.active));
    }
}

#[test]
fn terrain_history_keeps_regions_for_further_undo_and_never_redoes_a_leftover_object() {
    let (mut world, mut tool, mut settings) = fixture();
    let ctx = egui::Context::default();
    tool.set_tool(Tool::Land);
    let regions = vec![123, 456];
    tool.land.last_regions = regions.clone();
    tool.out.clear();
    for _ in 0..2 {
        history_frame(
            &ctx,
            &mut world,
            &mut tool,
            &mut settings,
            vec![history_key(egui::Key::Z, false, false)],
        );
        assert_eq!(tool.land.last_regions, regions);
    }
    let undone = tool
        .out
        .iter()
        .filter_map(|cmd| match cmd {
            aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::UndoLand { handle }) => Some(*handle),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(undone, regions.repeat(2));
    tool.out.clear();
    history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![history_key(egui::Key::Y, false, false), history_key(egui::Key::Z, true, false)],
    );
    assert!(tool.out.is_empty());
}

#[test]
fn object_history_targets_linkset_roots_once_or_the_selected_linked_part() {
    let (mut world, mut tool, mut settings) = fixture();
    let ctx = egui::Context::default();
    let root_idx = tool.roots(&world)[0];
    let root = world.objects.get(root_idx).unwrap();
    let (root_key, root_id) = (root.key, root.full_id);
    let child_idx = world
        .objects
        .iter()
        .find(|(idx, object)| *idx != root_idx && !object.is_avatar())
        .unwrap()
        .0;
    let child = world.objects.get_mut(child_idx).unwrap();
    child.parent_id = root_key.local_id;
    child.parent_region = root_key.region;
    let (child_key, child_id) = (child.key, child.full_id);
    tool.selection = vec![child_key, root_key];
    tool.out.clear();
    history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![history_key(egui::Key::Z, false, false)],
    );
    assert!(matches!(
        tool.out.as_slice(),
        [aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Undo { ids, .. })] if ids == &[root_id]
    ));
    tool.selection_linked = true;
    tool.selection = vec![child_key];
    tool.out.clear();
    history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![history_key(egui::Key::Y, false, false)],
    );
    assert!(matches!(
        tool.out.as_slice(),
        [aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Redo { ids, .. })] if ids == &[child_id]
    ));
    tool.open = false;
    tool.out.clear();
    history_frame(
        &ctx,
        &mut world,
        &mut tool,
        &mut settings,
        vec![history_key(egui::Key::Z, false, false)],
    );
    assert!(tool.out.is_empty());
}

#[test]
fn focused_text_history_stays_in_singleline_and_multiline_fields_instead_of_the_object() {
    for multiline in [false, true] {
        let (mut world, mut tool, mut settings) = fixture();
        let ctx = egui::Context::default();
        let id = egui::Id::unique("history_text");
        let mut text = String::new();
        let mut frame = |time, events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::super::shortcuts(ui.ctx(), &mut tool, &mut world, &mut settings);
                    let edit = if multiline {
                        egui::TextEdit::multiline(&mut text)
                    } else {
                        egui::TextEdit::singleline(&mut text)
                    };
                    ui.add(edit.id(id)).request_focus();
                },
            );
            output.textures_delta.clear();
            text.clone()
        };
        frame(0.0, vec![]);
        frame(0.1, vec![]);
        frame(0.2, vec![egui::Event::Text("Original".into())]);
        // Native TextEdit groups typing until its one-second idle timeout.
        frame(1.3, vec![]);
        assert_eq!(frame(1.5, vec![egui::Event::Text(" changed".into())]), "Original changed");
        frame(2.6, vec![]);
        assert_eq!(frame(2.7, vec![history_key(egui::Key::Z, false, false)]), "Original");
        assert_eq!(frame(2.8, vec![history_key(egui::Key::Y, false, false)]), "Original changed");
        assert_eq!(frame(2.9, vec![history_key(egui::Key::Z, false, false)]), "Original");
        assert_eq!(frame(3.0, vec![history_key(egui::Key::Z, true, false)]), "Original changed");
        assert!(!tool.out.iter().any(|cmd| matches!(
            cmd,
            aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Undo { .. } | aurora_net::build::BuildCmd::Redo { .. })
        )));
    }
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
fn general_includes_ownership_permissions_before_click_action() {
    let (mut world, mut tool, _) = fixture();
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let general = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, false));
    let rights = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, true));
    let has = |texts: &[(String, egui::Rect)], label: &str| texts.iter().any(|(text, _)| text == label);
    assert!(has(&general, aurora_i18n::tr!("common-name-2")));
    assert!(has(&general, aurora_i18n::tr!("common-share")));
    assert!(has(&general, aurora_i18n::tr!("common-show-in-search")));
    assert!(has(&general, aurora_i18n::tr!("build-ui-general-next-owner")));
    let permissions = general
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-general-next-owner"))
        .unwrap()
        .1;
    let click = general
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-general-click-to"))
        .unwrap()
        .1;
    assert!(permissions.top() < click.top());
    assert!(has(&rights, aurora_i18n::tr!("build-ui-general-next-owner")));
    assert!(!has(&rights, aurora_i18n::tr!("common-name-2")));
    assert!(!has(&rights, aurora_i18n::tr!("common-share")));
    assert!(!has(&rights, aurora_i18n::tr!("common-show-in-search")));
}

#[test]
fn shared_editing_enables_general_edits_without_sale_or_permission_authority() {
    use aurora_net::{NetCommand, build::BuildCmd};
    let (mut world, mut tool, _) = fixture();
    let idx = world.objects.index_of(&tool.selection[0]).unwrap();
    let id = world.objects.get(idx).unwrap().full_id;
    world.objects.get_mut(idx).unwrap().owner_id = uuid::Uuid::from_u128(911);
    tool.props.get_mut(&id).unwrap().owner_id = uuid::Uuid::from_u128(911);
    tool.out.clear();
    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let mut show = |events| frame(&ctx, events, |ui| general::show(ui, &p, &mut tool, &mut world, false));
    let select_all = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let mut texts = show(vec![]);
    for _ in 0..3 {
        texts = show(vec![]);
    }
    let name = texts.iter().find(|(text, _)| text == "Presentation test").unwrap().1.center();
    show(vec![egui::Event::PointerMoved(name)]);
    for pressed in [true, false] {
        show(vec![egui::Event::PointerButton {
            pos: name,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    show(vec![
        egui::Event::ModifiersChanged(select_all),
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: select_all,
        },
    ]);
    show(vec![
        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        egui::Event::Text("Shared edit".into()),
    ]);
    texts = show(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    for label in [aurora_i18n::tr!("common-share"), aurora_i18n::tr!("common-for-sale")] {
        let pos = texts.iter().find(|(text, _)| text == label).unwrap().1.center();
        show(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            show(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
    }
    assert!(
        matches!(&tool.out[..], [NetCommand::Build(BuildCmd::SetName { name, .. })] if name == "Shared edit"),
        "{:?}",
        tool.out
    );
    let texts = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, true));
    assert!(
        texts
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("build-ui-general-you-can-edit-this-object"))
    );
    world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
    let texts = frame(&ctx, vec![], |ui| general::show(ui, &p, &mut tool, &mut world, true));
    assert!(
        texts
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("build-ui-general-you-cannot-edit-this-object"))
    );
}

#[test]
fn general_name_edit_commits_to_each_selected_object_root() {
    use aurora_net::{NetCommand, build::BuildCmd};

    let (mut world, mut tool, _) = fixture();
    let keys: Vec<_> = [118, 151]
        .into_iter()
        .map(|local_id| {
            let index = world.objects.iter().find(|(_, object)| object.key.local_id == local_id).unwrap().0;
            let object = world.objects.get_mut(index).unwrap();
            object.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
            let key = object.key;
            tool.props.insert(
                object.full_id,
                ObjectProps {
                    object_id: object.full_id,
                    full: true,
                    name: format!("Original {local_id}"),
                    owner_id: world.agent_id,
                    creator_id: world.agent_id,
                    owner_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    base_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    ..Default::default()
                },
            );
            key
        })
        .collect();
    tool.selection = keys.clone();
    tool.selection_linked = false;
    tool.out.clear();

    let p = crate::theme::Theme::default().palette();
    let ctx = egui::Context::default();
    let mut show = |events| frame(&ctx, events, |ui| general::show(ui, &p, &mut tool, &mut world, false));
    let mut texts = show(vec![]);
    for _ in 0..3 {
        texts = show(vec![]);
    }
    let name_field = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-general-multiple-selection"))
        .unwrap()
        .1
        .center();
    show(vec![egui::Event::PointerMoved(name_field)]);
    for pressed in [true, false] {
        show(vec![egui::Event::PointerButton {
            pos: name_field,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    let select_all = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    show(vec![
        egui::Event::ModifiersChanged(select_all),
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: select_all,
        },
    ]);
    show(vec![
        egui::Event::ModifiersChanged(egui::Modifiers::NONE),
        egui::Event::Text("Shared edit".into()),
    ]);
    show(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);

    let changed: Vec<_> = tool
        .out
        .iter()
        .filter_map(|command| match command {
            NetCommand::Build(BuildCmd::SetName { local_ids, name, .. }) => Some((local_ids, name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        changed.len(),
        1,
        "the selected roots should share one region packet: {:?}",
        tool.out
    );
    assert_eq!(changed[0].1, "Shared edit");
    assert_eq!(changed[0].0, &keys.iter().map(|key| key.local_id).collect::<Vec<_>>());
}

#[test]
fn megaprim_edit_keeps_server_dimensions_until_a_size_edit() {
    for legacy in [false, true] {
        for modifiable in [true, false] {
            let mut inspector = Inspector::new();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 5;
            let idx = inspector.world.objects.index_of(&inspector.tool.selection[0]).unwrap();
            let original = Vec3::new(128.125, 256.25, 1.5);
            let object = inspector.world.objects.get_mut(idx).unwrap();
            object.scale = original;
            if !modifiable {
                object.update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            }
            inspector.tool.out.clear();
            for _ in 0..6 {
                inspector.show(vec![]);
                assert_eq!(
                    inspector.world.objects.get(idx).unwrap().scale,
                    original,
                    "Opening Edit changed the megaprim (legacy={legacy}, modifiable={modifiable})"
                );
                assert!(!inspector.tool.out.iter().any(|command| matches!(
                    command,
                    aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Transform { .. })
                )));
            }
            if modifiable {
                // Moving or rotating a megaprim must not submit its size either.
                inspector.tool.set_transform(
                    &mut inspector.world,
                    &inspector.settings,
                    Some(Vec3::new(42.0, 43.0, 44.0)),
                    None,
                    Some(glam::Quat::from_rotation_z(0.5)),
                    true,
                );
                assert_eq!(inspector.world.objects.get(idx).unwrap().scale, original);
                assert!(matches!(
                    inspector.tool.out.last(),
                    Some(aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Transform { updates, .. }))
                        if updates.len() == 1 && updates[0].scale.is_none()
                ));
            }
        }
    }
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
    numeric_drag: Option<crate::ui::numeric_drag::NumericDrag>,
    native_pixels_per_point: f32,
    screen: Vec2,
    text_fonts: Vec<(String, f32)>,
    /// A window drawn before the build tools that takes Escape for itself.
    escape_taken_first: bool,
}

impl Inspector {
    /// The object's contents as the simulator lists them: the PBR controls
    /// can be edited once they are known.
    fn list_contents(&mut self, idx: usize) {
        let object = self.world.objects.get(idx).unwrap().full_id;
        self.tool.contents_ready(&self.world, idx);
        self.tool.on_task_inventory(&self.world, object, Some(1), Vec::new());
    }

    fn new() -> Self {
        let (world, mut tool, settings) = fixture();
        tool.tab = 1;
        let scene = crate::scene::Scene::new(std::env::temp_dir(), world.avatar_lib.clone());
        let ctx = egui::Context::default();
        crate::ui::widgets::place_floater(&ctx, super::super::INSPECTOR_ID, egui::pos2(12.0, 40.0), vec2(344.0, 684.0));
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
            numeric_drag: None,
            escape_taken_first: false,
            native_pixels_per_point: 1.0,
            screen: egui::vec2(1280.0, 900.0),
            text_fonts: Vec::new(),
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
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.screen)),
            time: Some(self.frame as f64 / 60.0),
            events,
            focused: true,
            ..Default::default()
        };
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().native_pixels_per_point = Some(self.native_pixels_per_point);
        if let Some(drag) = &mut self.numeric_drag {
            drag.before(&mut input, self.ctx.pixels_per_point());
        }
        let mut output = self.ctx.run_ui(input, |ui| {
            if self.escape_taken_first {
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
            super::super::shortcuts(ui.ctx(), &mut self.tool, &mut self.world, &mut self.settings);
            let mut env = Env {
                images: &Default::default(),
                media: &mut self.media,
                scene: &mut self.scene,
                zoom_fraction: 0.0,
                property_lines: &mut false,
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
        });
        if let Some(drag) = &mut self.numeric_drag {
            drag.after(&self.ctx, &output.platform_output.events, true);
        }
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
        fn fonts(shape: &egui::Shape, out: &mut Vec<(String, f32)>) {
            match shape {
                egui::Shape::Text(text) => {
                    if let Some(section) = text.galley.job.sections.first() {
                        out.push((text.galley.text().to_owned(), section.format.font_id.size));
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        fonts(shape, out);
                    }
                }
                _ => {}
            }
        }
        self.text_fonts.clear();
        for shape in output.shapes {
            fonts(&shape.shape, &mut self.text_fonts);
            text_shapes(&shape.shape, &mut texts);
        }
        let rect = self
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(super::super::INSPECTOR_ID)).unwrap());
        (rect, texts)
    }
}

fn resize_inspector(inspector: &mut Inspector, size: Vec2) -> (egui::Rect, Vec<(String, egui::Rect)>) {
    crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, size);
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    inspector.show(vec![])
}

#[test]
fn construction_presentations_follow_pointer_resizing_without_changing_selection_or_drafts() {
    let mut inspector = Inspector::new();
    inspector.tool.tab = 3;
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    inspector.tool.ui.edits.insert("workspace-draft", "Pending value".into());
    let selection = inspector.tool.selection.clone();
    let settings = inspector.settings.clone();
    for width in [584.0, 1000.0, 480.0, 584.0] {
        let before = inspector.show(vec![]).0;
        let start = before.right_bottom() - Vec2::splat(2.0);
        let target = egui::pos2(before.left() + width - 2.0, start.y);
        inspector.show(vec![egui::Event::PointerMoved(start)]);
        inspector.show(vec![egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        for _ in 0..2 {
            inspector.show(vec![egui::Event::PointerMoved(target)]);
        }
        inspector.show(vec![egui::Event::PointerButton {
            pos: target,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        for _ in 0..5 {
            inspector.show(vec![]);
        }
        let (rect, texts) = inspector.show(vec![]);
        assert!(
            (rect.width() - width).abs() < 4.0,
            "A presentation must not snap to another width: {width}, {rect:?}"
        );
        assert!((rect.height() - before.height()).abs() < 1.0);
        let rail = texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-workspace-object"));
        assert!(!rail, "Labeled navigation at width {width}");
        let vectors: Vec<_> = texts
            .iter()
            .filter(|(text, _)| text == aurora_i18n::tr!("build-compact-position"))
            .collect();
        assert_eq!(vectors.len(), 1, "Companion transform at width {width}");
        for (_, control) in vectors {
            assert!(rect.contains_rect(*control));
        }
        assert_eq!(inspector.tool.selection, selection);
        assert_eq!(inspector.tool.tab, 3);
        assert_eq!(inspector.tool.ui.edits["workspace-draft"], "Pending value");
        assert_eq!(inspector.settings, settings);
    }
}

#[test]
fn construction_large_keeps_each_section_and_tool_inside_a_stable_window() {
    let mut inspector = Inspector::new();
    let size = resize_inspector(&mut inspector, vec2(1000.0, 780.0)).0.size();
    for tab in 0..7 {
        inspector.tool.tab = tab;
        for _ in 0..4 {
            inspector.show(vec![]);
        }
        let (rect, texts) = inspector.show(vec![]);
        assert!((rect.size() - size).length() < 1.0, "Section {tab} changed the window size");
        for label in [
            aurora_i18n::tr!("build-compact-select-face"),
            aurora_i18n::tr!("build-compact-root-axis"),
        ] {
            let controls: Vec<_> = texts.iter().filter(|(text, _)| text == label).collect();
            assert_eq!(controls.len(), 1, "Shared command duplicated in section {tab}: {label}");
            assert!(rect.contains_rect(controls[0].1));
        }
        assert_eq!(
            texts
                .iter()
                .filter(|(text, _)| text == aurora_i18n::tr!("build-compact-position"))
                .count(),
            1
        );
    }
    for tool in [Tool::Focus, Tool::Grab, Tool::Create, Tool::Land] {
        inspector.tool.set_tool(tool);
        for _ in 0..4 {
            inspector.show(vec![]);
        }
        assert!(
            (inspector.show(vec![]).0.size() - size).length() < 1.0,
            "Tool {tool:?} changed the window size"
        );
    }
}

#[test]
fn create_panel_holds_still_at_every_width_around_its_scroll_bar() {
    // #1092: the shape grid sized itself from the width left by the scroll
    // bar, so at widths where only the wider grid overflowed the bar came
    // and went on every frame and the whole panel jittered.
    let mut inspector = Inspector::new();
    inspector.tool.set_tool(Tool::Create);
    let label = aurora_i18n::tr!("build-ui-tools-keep-selection-tool");
    let place = |texts: &[(String, egui::Rect)]| texts.iter().find(|(text, _)| text == label).map(|(_, rect)| rect.min);
    // Past about 650 points the grid pushes the options below the view.
    for width in (300..=600).step_by(3) {
        resize_inspector(&mut inspector, vec2(width as f32, 300.0));
        let first = place(&inspector.show(vec![]).1);
        assert!(first.is_some(), "Create options missing at width {width}");
        for _ in 0..4 {
            assert_eq!(place(&inspector.show(vec![]).1), first, "Create panel moved at width {width}");
        }
    }
}

#[test]
fn construction_reflows_at_fixed_sizes_and_uses_saved_geometry_after_layout_reload() {
    let mut inspector = Inspector::new();
    inspector.tool.tab = 3;
    for width in [518.0, 524.0, 918.0, 924.0, 640.0] {
        let (rect, texts) = resize_inspector(&mut inspector, vec2(width, 710.0));
        assert!((rect.width() - width).abs() < 1.0);
        assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-workspace-object")));
        assert!(texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-position")));
    }
    let custom = inspector.show(vec![]).0.size();
    let saved = ron::to_string(&inspector.ctx.memory(|m| m.clone())).unwrap();
    let mut reopened = Inspector::new();
    reopened.tool.tab = 3;
    reopened.ctx.memory_mut(|m| *m = ron::from_str(&saved).unwrap());
    for _ in 0..5 {
        reopened.show(vec![]);
    }
    let (rect, texts) = reopened.show(vec![]);
    assert!((rect.size() - custom).length() < 1.0);
    assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-workspace-object")));
    assert!(texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-position")));
    resize_inspector(&mut reopened, vec2(1000.0, 780.0));
    reopened.screen = vec2(800.0, 600.0);
    for _ in 0..5 {
        reopened.show(vec![]);
    }
    let (rect, texts) = reopened.show(vec![]);
    assert!(rect.right() <= 800.0 && rect.bottom() <= 600.0);
    assert!(texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-position")));
}

#[test]
fn construction_presentations_keep_replacement_exclusive_and_cancel_in_reach() {
    let mut inspector = Inspector::new();
    let target = crate::demo::action_id(118);
    inspector.tool.replacement.draft = Some(crate::build::replace::Draft {
        target,
        number: 1,
        source: None,
        picking: true,
    });
    let selection = inspector.tool.selection.clone();
    for size in [vec2(344.0, 684.0), vec2(584.0, 740.0), vec2(1000.0, 780.0), vec2(584.0, 520.0)] {
        let (rect, texts) = resize_inspector(&mut inspector, size);
        for label in [aurora_i18n::tr!("replace-choose-source"), aurora_i18n::tr!("common-cancel")] {
            let control = texts.iter().find(|(text, _)| text == label).unwrap().1;
            assert!(
                rect.contains_rect(control),
                "Replacement action outside {size:?}: {label}, {control:?}"
            );
        }
        for hidden in [
            aurora_i18n::tr!("build-workspace-object"),
            aurora_i18n::tr!("build-workspace-tools"),
            aurora_i18n::tr!("build-compact-duplicate"),
        ] {
            assert!(
                !texts.iter().any(|(text, _)| text == hidden),
                "Replacement must own the window: {hidden}"
            );
        }
        let draft = inspector.tool.replacement.draft.as_ref().unwrap();
        assert_eq!(draft.target, target);
        assert!(draft.picking);
        assert_eq!(inspector.tool.selection, selection);
        assert_eq!(inspector.tool.tool, Tool::Edit);
    }
}

#[test]
fn construction_fonts_match_monitor_body_secondary_and_section_sizes() {
    let mut inspector = Inspector::new();
    inspector.tool.tab = 3;
    inspector.tool.ui.layout.open = true;
    let original = inspector.ctx.style_of(inspector.ctx.theme());
    resize_inspector(&mut inspector, vec2(1000.0, 780.0));
    for (label, size) in [
        (aurora_i18n::tr!("build-compact-root-axis"), 12.0),
        (aurora_i18n::tr!("build-compact-position"), 12.0),
        (aurora_i18n::tr!("build-status-self"), 11.0),
        (aurora_i18n::tr!("build-compact-materials"), 14.0),
        (aurora_i18n::tr!("build-layout-ruler"), 13.0),
    ] {
        assert!(
            inspector
                .text_fonts
                .iter()
                .any(|(text, actual)| text == label && (*actual - size).abs() < 0.01),
            "Wrong font for {label}"
        );
    }
    assert!(
        inspector
            .text_fonts
            .iter()
            .any(|(text, size)| aurora_i18n::parse_number(text).is_some() && (*size - 12.0).abs() < 0.01)
    );
    assert_eq!(
        inspector.ctx.style_of(inspector.ctx.theme()).text_styles,
        original.text_styles,
        "Build typography must remain local to the floater"
    );
}

// Transaction tests keep their fields in view while retaining compact widths.
fn material_inspector() -> Inspector {
    let mut inspector = Inspector::new();
    inspector.screen.y = 1800.0;
    inspector.settings.transform_expanded = false;
    crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, vec2(350.0, 1500.0));
    inspector
}

#[test]
fn shifted_mesh_selection_edits_specular_values_across_different_maps() {
    use crate::scene::legacy_mat::LegacyMaterial;
    use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_MOVE};

    let mut inspector = material_inspector();
    inspector.tool.tab = 3;
    inspector.tool.ui.tex_tab = 0;
    let objects: Vec<_> = [118, 151]
        .into_iter()
        .map(|local_id| {
            inspector
                .world
                .objects
                .iter()
                .find(|(_, object)| object.key.local_id == local_id)
                .unwrap()
                .0
        })
        .collect();
    let material_ids = [uuid::Uuid::from_u128(0x5001), uuid::Uuid::from_u128(0x5002)];
    let specular_maps = [uuid::Uuid::from_u128(0x6001), uuid::Uuid::from_u128(0x6002)];
    for (index, (&object_idx, &material_id)) in objects.iter().zip(&material_ids).enumerate() {
        let object = inspector.world.objects.get_mut(object_idx).unwrap();
        object.update_flags |= OBJECT_MODIFY | OBJECT_MOVE;
        object.volume.sculpt = Some(aurora_prim::params::SculptParams {
            texture: uuid::Uuid::from_u128(0x7000 + index as u128),
            sculpt_type: aurora_prim::params::LL_SCULPT_TYPE_MESH,
        });
        let mut te = object.te.as_deref().unwrap().clone();
        for face in &mut te.faces {
            face.material_id = material_id;
        }
        object.te = Some(std::sync::Arc::new(te));
        inspector.scene.legacy_mats.insert(
            material_id,
            LegacyMaterial {
                specular_map: specular_maps[index],
                specular_exp: 70,
                env_intensity: 15,
                ..Default::default()
            },
        );
    }

    inspector
        .tool
        .click_select(&mut inspector.world, &inspector.settings, Some(objects[0]), false);
    inspector.tool.mouse_down(
        &mut inspector.world,
        &mut inspector.settings,
        None,
        Some(objects[1]),
        None,
        (10_000.0, 10_000.0),
        Mods {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(inspector.tool.selection.len(), 2, "Shift-click should select both mesh roots");

    for _ in 0..4 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let environment = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-environment"))
        .unwrap()
        .1;
    let field = texts
        .iter()
        .find(|(text, rect)| {
            aurora_i18n::parse_number(text) == Some(15.0) && rect.top() > environment.top() && rect.top() < environment.bottom() + 32.0
        })
        .unwrap()
        .1;
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
    click(&mut inspector, field.center());
    inspector.show(vec![egui::Event::Text("44".into())]);
    inspector.show(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);

    for (object_idx, _) in objects.iter().zip(&material_ids) {
        let id = inspector.world.objects.get(*object_idx).unwrap().full_id;
        assert!(
            inspector.tool.ui.legacy_pending.iter().any(|((object_id, _), (_, material))| {
                *object_id == id && material.as_ref().is_some_and(|material| material.env_intensity == 44)
            }),
            "the selected mesh {id} should receive the edited environment value"
        );
    }
}

#[test]
fn material_tab_clicks_switch_the_selected_objects_local_render_mode() {
    let mut inspector = material_inspector();
    inspector.tool.tab = 3;
    inspector.tool.ui.tex_tab = 1;
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let asset = uuid::Uuid::from_u128(0x7995);
    inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = (0..6).map(|face| (face, asset)).collect();
    inspector.scene.materials.insert(asset, Default::default());
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    // The frame's update has chosen the tab for this selection (PBR), so
    // the clicks below are the user's own choices.
    inspector.tool.update_material_preview(&mut inspector.world, &inspector.settings);
    assert_eq!(inspector.tool.ui.tex_tab, 1);
    inspector.tool.out.clear();
    for (tab, label) in [
        (0, aurora_i18n::tr!("build-ui-texture-blinn-phong")),
        (1, "PBR"),
        (2, aurora_i18n::tr!("extra-media")),
    ] {
        let texts = inspector.show(vec![]).1;
        let point = texts.iter().find(|(text, _)| text == label).unwrap().1.center();
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        assert_eq!(inspector.tool.ui.tex_tab, tab);
        inspector.tool.update_material_preview(&mut inspector.world, &inspector.settings);
        assert_eq!(inspector.world.objects.get(idx).unwrap().visual_material(0).is_none(), tab == 0);
        assert_eq!(inspector.world.objects.get(idx).unwrap().extra.render_materials[0].1, asset);
    }
    assert!(inspector.tool.out.is_empty());
}

#[test]
fn navigation_sets_the_resize_floor_and_settings_glyph_stays_square() {
    use crate::build::layout::Panel;
    let mut inspector = Inspector::new();
    inspector.screen.y = 1400.0;
    for expanded in [false, true] {
        inspector.settings.transform_expanded = expanded;
        let minimum = minimum_size(&inspector.tool, &inspector.settings);
        let (window, _) = resize_inspector(&mut inspector, vec2(350.0, 420.0));
        assert!((window.height() - minimum.y).abs() < 1.0, "{window:?}, {minimum:?}");
        let zone = inspector
            .tool
            .ui
            .layout
            .zones
            .iter()
            .find(|(panel, _)| *panel == Panel::Navigation)
            .unwrap()
            .1;
        let viewport = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("build_navigation_viewport")))
            .unwrap();
        for (_, _, row) in inspector
            .tool
            .ui
            .layout
            .targets
            .iter()
            .filter(|(_, panel, _)| *panel == Panel::Navigation)
        {
            assert!(zone.contains_rect(*row) && viewport.contains_rect(*row) && window.contains_rect(*row));
        }
        let glyph = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("compact_icon_glyph", "settings-filled"))))
            .unwrap();
        assert_eq!(glyph.width(), glyph.height());
        assert!(window.contains_rect(glyph));
    }
    let before = minimum_size(&inspector.tool, &inspector.settings).y;
    for item in inspector.settings.layout.in_panel(Panel::Navigation).into_iter().take(3) {
        inspector.settings.layout.place(item, Panel::Hidden, None, false);
    }
    let after = minimum_size(&inspector.tool, &inspector.settings).y;
    assert_eq!(before - after, 111.0);
    let (window, _) = resize_inspector(&mut inspector, vec2(350.0, 420.0));
    assert!((window.height() - after).abs() < 1.0);
}

/// An inventory texture released on a slot of the Texture tab applies to
/// the selected faces, like the picker (LLTextureCtrl::handleDragAndDrop,
/// #2296); a material or a release beside the slot changes nothing.
#[test]
fn a_texture_dropped_on_a_texture_slot_applies_to_the_selected_faces() {
    use super::super::PickTarget;
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let mut inspector = material_inspector();
    inspector.tool.tab = 3;
    inspector.tool.ui.tex_tab = 0;
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let agent = inspector.world.agent_id;
    let all = crate::build::contents_drop::UNRESTRICTED;
    let (texture, material, asset) = (
        uuid::Uuid::from_u128(0x0002_2961),
        uuid::Uuid::from_u128(0x0002_2962),
        uuid::Uuid::from_u128(0x0002_2963),
    );
    for (id, kind) in [(texture, 0), (material, 57)] {
        inspector.world.inventory.items.insert(
            id,
            aurora_net::inventory::InvItem {
                id,
                asset_type: kind,
                inv_type: kind,
                asset_id: asset,
                owner: agent,
                base_mask: all,
                owner_mask: all,
                ..Default::default()
            },
        );
    }
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let slot = |inspector: &Inspector| {
        inspector.ctx.data(|d| {
            d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", format!("{:?}", PickTarget::Diffuse))))
                .expect("diffuse slot")
        })
    };
    let diffuse = |inspector: &Inspector| inspector.world.objects.get(idx).unwrap().te.as_ref().unwrap().face(0).texture;
    let drop = |inspector: &mut Inspector, item: uuid::Uuid, pos: egui::Pos2| {
        egui::DragAndDrop::set_payload(&inspector.ctx, crate::ui::inventory::InvDrag(item));
        inspector.show(vec![egui::Event::PointerMoved(pos)]);
        inspector.show(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ]);
        egui::DragAndDrop::clear_payload(&inspector.ctx);
        inspector.show(vec![]);
    };
    let before = diffuse(&inspector);
    assert_ne!(before, asset);
    let rect = slot(&inspector);
    drop(&mut inspector, material, rect.center());
    drop(&mut inspector, texture, rect.right_bottom() + vec2(30.0, 30.0));
    assert_eq!(diffuse(&inspector), before, "a material, or a release beside the slot");
    assert!(inspector.tool.ui.slot_drop.is_none());
    drop(&mut inspector, texture, rect.center());
    assert_eq!(diffuse(&inspector), asset);
    assert!(!inspector.tool.ui.picker.open, "a drop does not open the picker");
}

#[test]
fn material_previews_are_larger_above_their_options_and_open_the_correct_picker() {
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    for pbr in [false, true] {
        let mut inspector = material_inspector();
        inspector.tool.tab = 3;
        inspector.tool.ui.tex_tab = usize::from(pbr);
        let idx = inspector.tool.sel_prims(&inspector.world)[0];
        let asset = uuid::Uuid::from_u128(0x7994);
        if pbr {
            inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(u8::MAX, asset)];
            inspector.scene.materials.insert(asset, Default::default());
            inspector.list_contents(idx);
        }
        for _ in 0..5 {
            inspector.show(vec![]);
        }
        let (window, texts) = inspector.show(vec![]);
        if pbr {
            assert!(
                inspector
                    .ctx
                    .data(|d| d.get_temp::<bool>(egui::Id::unique("build_material_model_ready")))
                    .unwrap()
            );
        }
        let targets = if pbr {
            vec![
                super::super::PickTarget::PbrMaterial,
                super::super::PickTarget::PbrMap(0),
                super::super::PickTarget::PbrMap(1),
                super::super::PickTarget::PbrMap(3),
                super::super::PickTarget::PbrMap(2),
            ]
        } else {
            vec![
                super::super::PickTarget::Diffuse,
                super::super::PickTarget::Normal,
                super::super::PickTarget::Specular,
            ]
        };
        let rects: Vec<_> = targets
            .iter()
            .map(|target| {
                inspector.ctx.data(|d| {
                    d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", format!("{target:?}"))))
                        .unwrap()
                })
            })
            .collect();
        for rect in &rects {
            assert!(window.contains_rect(*rect));
            assert!((78.0..=82.1).contains(&rect.width()));
            assert!((rect.width() - rect.height()).abs() < 0.1);
        }
        assert!((rects[0].top() - rects[1].top()).abs() < 0.1);
        assert!(rects[0].right() < rects[1].left());
        if pbr {
            assert!((rects[2].top() - rects[0].top()).abs() < 0.1);
            assert!(rects[3].top() > rects[0].bottom());
            assert!((rects[3].left() - rects[0].left()).abs() < 0.1);
            assert!((rects[4].top() - rects[3].top()).abs() < 0.1);
        } else {
            assert!((rects[2].top() - rects[0].top()).abs() < 0.1);
        }
        let surface = texts
            .iter()
            .find(|(text, _)| text == aurora_i18n::tr!("build-material-surface"))
            .unwrap()
            .1;
        assert!(rects.iter().all(|rect| rect.bottom() < surface.top()));
        let point = rects[1].center();
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            inspector.show(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            }]);
        }
        assert!(inspector.tool.ui.picker.open);
        assert_eq!(inspector.tool.ui.pick_target, Some(targets[1]));
        let (_, labels) = inspector.show(vec![]);
        assert!(
            labels.iter().any(|(label, _)| label == aurora_i18n::tr!("picker-bom")),
            "map pickers expose BOM choices"
        );
        assert!(
            !inspector.tool.ui.wanted_images.contains(&asset),
            "glTF assets are not texture images"
        );
    }
}

#[test]
fn pbr_number_scrubbing_keeps_the_chosen_inspector_width() {
    for (native, zoom, text_scale) in [
        (1.0, 1.0, 1.0),
        (1.25, 1.0, 1.0),
        (1.0, 1.125, 1.0),
        (1.25, 1.125, 1.0),
        (1.0, 1.25, 1.0),
        (1.0, 1.0, 1.4),
        (1.0, 1.0, 2.0),
    ] {
        for width in [344.0, 390.0] {
            let mut inspector = material_inspector();
            inspector.settings.transform_expanded = false;
            inspector.numeric_drag = Some(Default::default());
            inspector.native_pixels_per_point = native;
            crate::theme::Theme::default().apply(&inspector.ctx, text_scale);
            inspector.ctx.set_zoom_factor(zoom);
            inspector.tool.tab = 3;
            inspector.tool.ui.tex_tab = 1;
            inspector.tool.edit_mode = EditMode::Face;
            let key = inspector.tool.selection[0];
            inspector.tool.faces.insert(key, vec![0]);
            let idx = inspector.world.objects.index_of(&key).unwrap();
            let asset = uuid::Uuid::from_u128(0x7992);
            inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(u8::MAX, asset)];
            inspector.list_contents(idx);
            inspector.scene.materials.insert(
                asset,
                aurora_assets::PbrMaterial {
                    metallic_factor: 0.5,
                    roughness_factor: 0.5,
                    ..Default::default()
                },
            );
            inspector.show(vec![]);
            crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(width, 1500.0));
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let initial = inspector.show(vec![]).0;
            for label in [
                aurora_i18n::tr!("build-ui-texture-metallic"),
                aurora_i18n::tr!("build-ui-texture-roughness"),
            ] {
                let (_, texts) = inspector.show(vec![]);
                let caption = texts.iter().find(|(text, _)| text == label).unwrap().1;
                let point = texts
                    .iter()
                    .find(|(text, rect)| {
                        aurora_i18n::parse_number(text).is_some()
                            && rect.left() > caption.right()
                            && (rect.center().y - caption.center().y).abs() < 8.0
                    })
                    .unwrap()
                    .1
                    .center();
                inspector.show(vec![egui::Event::PointerMoved(point)]);
                inspector.show(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }]);
                let mut end = point;
                inspector.tool.out.clear();
                for frame in 0..60 {
                    if frame < 20 {
                        end = point + egui::vec2(frame as f32 + 4.0, 0.0);
                    } else if frame >= 40 {
                        end = point + egui::vec2(71.0 - frame as f32, 0.0);
                    }
                    let delta = if frame < 20 {
                        20.0
                    } else if frame >= 40 {
                        -20.0
                    } else {
                        0.0
                    };
                    let (rect, _) = inspector.show(vec![
                        egui::Event::PointerMoved(end),
                        egui::Event::MouseMoved(egui::vec2(delta, 0.0)),
                    ]);
                    if frame == 15 {
                        assert!(
                            inspector.numeric_drag.as_ref().unwrap().active(),
                            "The real numeric drag must be active: native={native}, zoom={zoom}, text={text_scale}"
                        );
                    }
                    assert!(
                        (rect.size() - initial.size()).length() < 1.0,
                        "Scrubbing {label} changed the window at frame {frame}, native={native}, zoom={zoom}, text={text_scale}, width={width}: {initial:?} -> {rect:?}"
                    );
                }
                inspector.show(vec![egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                }]);
                assert!(!inspector.numeric_drag.as_ref().unwrap().active());
                assert_eq!(
                    inspector
                        .tool
                        .out
                        .iter()
                        .filter(|command| matches!(command,
                            aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Cap { cap, .. }) if cap == "ModifyMaterialParams"
                        ))
                        .count(),
                    1,
                    "The scrub must commit its PBR value once"
                );
                for _ in 0..180 {
                    let rect = inspector.show(vec![]).0;
                    assert!(
                        (rect.size() - initial.size()).length() < 1.0,
                        "The released inspector must retain its chosen size"
                    );
                }
            }
        }
    }
}

#[test]
fn material_numbers_commit_typed_values_once_without_resetting_the_inspector() {
    for label in [
        aurora_i18n::tr!("build-ui-texture-shininess"),
        aurora_i18n::tr!("common-environment"),
    ] {
        for finish in [egui::Key::Enter, egui::Key::Tab, egui::Key::Escape] {
            let mut inspector = material_inspector();
            inspector.settings.transform_expanded = false;
            inspector.tool.tab = 3;
            inspector.tool.edit_mode = EditMode::Face;
            inspector.tool.faces.insert(inspector.tool.selection[0], vec![0]);
            let idx = inspector.tool.sel_prims(&inspector.world)[0];
            let object = inspector.world.objects.get_mut(idx).unwrap();
            let id = object.full_id;
            let material_id = uuid::Uuid::from_u128(0x7990);
            Arc::make_mut(object.te.as_mut().unwrap()).faces[0].material_id = material_id;
            inspector.scene.legacy_mats.insert(
                material_id,
                crate::scene::legacy_mat::LegacyMaterial {
                    specular_map: texture::DEFAULT_SPECULAR,
                    ..Default::default()
                },
            );
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (window, texts) = inspector.show(vec![]);
            let caption = texts
                .iter()
                .filter(|(text, _)| text == label)
                .max_by(|a, b| a.1.top().total_cmp(&b.1.top()))
                .unwrap()
                .1;
            let field = texts
                .iter()
                .find(|(text, rect)| {
                    aurora_i18n::parse_number(text).is_some()
                        && (rect.center().y - caption.center().y).abs() < 8.0
                        && rect.left() > caption.right()
                })
                .unwrap()
                .1;
            let point = field.center();
            inspector.show(vec![egui::Event::PointerMoved(point)]);
            for pressed in [true, false] {
                inspector.show(vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }]);
            }
            let focus = inspector.ctx.memory(|m| m.focused());
            assert!(focus.is_some(), "Number entry must have focus");
            inspector.tool.out.clear();
            for digit in ["1", "2", "3"] {
                let (rect, _) = inspector.show(vec![egui::Event::Text(digit.into())]);
                assert!(
                    inspector.tool.ui.legacy_pending.is_empty(),
                    "Typing {label} must not submit a material"
                );
                assert_eq!(inspector.ctx.memory(|m| m.focused()), focus);
                assert!((rect.size() - window.size()).length() < 1.0);
            }
            inspector.show(vec![egui::Event::Key {
                key: finish,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]);
            inspector.show(vec![]);
            if finish == egui::Key::Escape {
                assert!(inspector.tool.ui.legacy_pending.is_empty());
                continue;
            }
            let material = inspector.tool.ui.legacy_pending[&(id, 0)].1.as_ref().unwrap();
            if label == aurora_i18n::tr!("common-environment") {
                assert_eq!((material.specular_exp, material.env_intensity), (51, 123));
            } else {
                assert_eq!((material.specular_exp, material.env_intensity), (123, 0));
            }
            let (rect, _) = inspector.show(vec![]);
            assert!((rect.size() - window.size()).length() < 1.0);
            assert_eq!(
                inspector
                    .tool
                    .out
                    .iter()
                    .filter(|command| matches!(command,
                        aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Cap { cap, .. }) if cap == "RenderMaterials"
                    ))
                    .count(),
                1,
                "One material submission per completed edit"
            );
        }
    }
}

#[test]
fn material_update_keeps_controls_and_focus_until_the_new_material_loads() {
    let mut inspector = material_inspector();
    inspector.settings.transform_expanded = false;
    inspector.tool.tab = 3;
    inspector.tool.edit_mode = EditMode::Face;
    inspector.tool.faces.insert(inspector.tool.selection[0], vec![0]);
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let object = inspector.world.objects.get_mut(idx).unwrap();
    let id = object.full_id;
    let old_id = uuid::Uuid::from_u128(0x7990);
    let new_id = uuid::Uuid::from_u128(0x7991);
    Arc::make_mut(object.te.as_mut().unwrap()).faces[0].material_id = old_id;
    let material = crate::scene::legacy_mat::LegacyMaterial {
        specular_map: texture::DEFAULT_SPECULAR,
        specular_exp: 123,
        env_intensity: 45,
        ..Default::default()
    };
    inspector.scene.legacy_mats.insert(old_id, material.clone());
    inspector.tool.ui.legacy_pending.insert((id, 0), (old_id, Some(material.clone())));
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (window, texts) = inspector.show(vec![]);
    let field = texts.iter().find(|(text, _)| text == "123").unwrap().1;
    let point = field.center();
    inspector.show(vec![egui::Event::PointerMoved(point)]);
    for pressed in [true, false] {
        inspector.show(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    let focus = inspector.ctx.memory(|m| m.focused());
    assert!(focus.is_some());
    Arc::make_mut(inspector.world.objects.get_mut(idx).unwrap().te.as_mut().unwrap()).faces[0].material_id = new_id;
    for loaded in [false, true] {
        if loaded {
            let mut confirmed = material.clone();
            confirmed.env_intensity = 46;
            inspector.scene.legacy_mats.insert(new_id, confirmed);
        }
        for _ in 0..4 {
            let (rect, texts) = inspector.show(vec![]);
            assert!(
                texts.iter().any(|(text, _)| text == aurora_i18n::tr!("common-environment")),
                "Material controls disappeared during the simulator update (loaded={loaded})"
            );
            assert_eq!(inspector.ctx.memory(|m| m.focused()), focus);
            assert!((rect.size() - window.size()).length() < 1.0);
            let environment = texts.iter().find(|(text, _)| text == if loaded { "46" } else { "45" }).unwrap().1;
            assert!(environment.top() > field.top());
        }
    }
    assert!(
        inspector.tool.ui.legacy_pending.is_empty(),
        "A loaded server material supersedes the pending edit"
    );
    Arc::make_mut(inspector.world.objects.get_mut(idx).unwrap().te.as_mut().unwrap()).faces[0].material_id = uuid::Uuid::nil();
    let (_, texts) = inspector.show(vec![]);
    assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("common-environment")));
}

#[test]
fn construction_coordinate_icons_select_one_mode_and_keep_the_right_edge() {
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
    for width in [344.0, 520.0] {
        crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(width, 684.0));
        for section in [6, 1, 5, 3, 2, 0, 4] {
            inspector.tool.tab = section;
            for _ in 0..3 {
                inspector.show(vec![]);
            }
            for mode in [1, 2, 0] {
                let (rect, texts) = inspector.show(vec![]);
                let linked = texts
                    .iter()
                    .find(|(text, _)| text == aurora_i18n::tr!("build-compact-linked"))
                    .unwrap()
                    .1;
                let target = inspector
                    .ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_grid_mode", mode))))
                    .unwrap();
                assert!(
                    rect.contains_rect(target),
                    "Coordinate control outside inspector: {rect:?}, {target:?}"
                );
                let point = target.center();
                assert!(
                    rect.right() - target.right() < 24.0,
                    "coordinate column must stay against the window's right edge: {rect:?}, {target:?}"
                );
                let face = texts
                    .iter()
                    .find(|(text, _)| text == aurora_i18n::tr!("build-compact-select-face"))
                    .unwrap()
                    .1;
                assert!(face.bottom() < linked.top(), "Face selection comes before linked editing");
                assert!(target.bottom() < face.top(), "References belong above the shared footer");
                click(&mut inspector, point);
                assert_eq!(inspector.settings.grid_mode, mode, "Coordinate icon in section {section}");
                if mode == 2 {
                    assert_eq!(inspector.tool.grid_objects, inspector.tool.selection);
                }
                click(&mut inspector, point);
                assert_eq!(inspector.settings.grid_mode, mode, "Clicking the active mode must keep it selected");
            }
        }
    }
}

#[test]
fn empty_legacy_texture_channels_can_be_selected_and_their_scale_edited() {
    let mut inspector = material_inspector();
    inspector.settings.transform_expanded = false;
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
fn material_spinner_drags_commit_all_uv_channels_and_glow_only_on_release() {
    fn values(inspector: &Inspector, channel: usize) -> [f32; 5] {
        let object = inspector
            .world
            .objects
            .get(inspector.world.objects.index_of(&inspector.tool.selection[0]).unwrap())
            .unwrap();
        if inspector.tool.ui.tex_tab == 1 {
            let over = BuildTool::gltf_override(&inspector.world, inspector.world.objects.index_of(&object.key).unwrap(), 0);
            let t = over.transforms[channel.saturating_sub(1)];
            return [t.scale[0], t.scale[1], t.offset[0], t.offset[1], t.rotation];
        }
        let material = inspector
            .tool
            .ui
            .legacy_pending
            .get(&(object.full_id, 0))
            .and_then(|(_, m)| m.clone())
            .unwrap_or_default();
        let te = object.te.as_ref().unwrap().face(0);
        match channel {
            1 => [
                material.normal_st[0],
                material.normal_st[1],
                material.normal_st[2],
                material.normal_st[3],
                material.normal_rot,
            ],
            2 => [
                material.specular_st[0],
                material.specular_st[1],
                material.specular_st[2],
                material.specular_st[3],
                material.specular_rot,
            ],
            _ => [te.scale_s, te.scale_t, te.offset_s, te.offset_t, te.rotation],
        }
    }
    fn drag(inspector: &mut Inspector, point: egui::Pos2) {
        inspector.tool.out.clear();
        inspector.show(vec![egui::Event::PointerMoved(point)]);
        inspector.show(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        let mut end = point;
        for delta in [10.0, 20.0, 30.0] {
            end = point + egui::vec2(delta, 0.0);
            inspector.show(vec![egui::Event::PointerMoved(end)]);
            assert!(inspector.tool.out.is_empty(), "Dragging must not send intermediate edits");
        }
        for _ in 0..3 {
            inspector.show(vec![]);
            assert!(inspector.tool.out.is_empty());
        }
        inspector.show(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        assert_eq!(inspector.tool.out.len(), 1, "Release must submit the retained value once");
        inspector.show(vec![]);
        assert_eq!(inspector.tool.out.len(), 1, "An idle frame must not submit again");
    }
    for pbr in [false, true] {
        for channel in 0..if pbr { 5 } else { 3 } {
            let mut inspector = material_inspector();
            inspector.settings.transform_expanded = false;
            inspector.tool.tab = 3;
            inspector.tool.edit_mode = EditMode::Face;
            inspector.tool.ui.tex_tab = usize::from(pbr);
            inspector.tool.ui.bp_channel = channel;
            inspector.tool.ui.pbr_channel = channel;
            let key = inspector.tool.selection[0];
            inspector.tool.faces.insert(key, vec![0]);
            let idx = inspector.world.objects.index_of(&key).unwrap();
            if pbr {
                inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(u8::MAX, uuid::Uuid::from_u128(9100))];
                // the UV fields wait for the material (face_server_pbr_transforms)
                inspector.scene.materials.insert(uuid::Uuid::from_u128(9100), Default::default());
                inspector.list_contents(idx);
            }
            inspector.show(vec![]);
            crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(390.0, 1500.0));
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            for property in 0..5 {
                let before = values(&inspector, channel);
                let (_, texts) = inspector.show(vec![]);
                let (label, width, row) = match property {
                    0 | 1 => (aurora_i18n::tr!("common-scale"), 103.0, property),
                    2 | 3 => (aurora_i18n::tr!("build-ui-texture-offset"), 75.0, property - 2),
                    _ => (aurora_i18n::tr!("build-ui-texture-rotation"), 71.0, 0),
                };
                let title = texts.iter().find(|(text, _)| text == label).unwrap().1;
                let mut fields: Vec<_> = texts
                    .iter()
                    .filter(|(text, rect)| {
                        aurora_i18n::parse_number(text).is_some()
                            && rect.top() > title.bottom()
                            && rect.top() < title.bottom() + 70.0
                            && rect.left() >= title.left()
                            && rect.right() < title.left() + width
                    })
                    .collect();
                fields.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
                let point = fields
                    .get(row)
                    .unwrap_or_else(|| panic!("Missing UV field {property}, PBR={pbr}, channel={channel}"))
                    .1
                    .center();
                drag(&mut inspector, point);
                let after = values(&inspector, channel);
                assert!(
                    after[property] > before[property],
                    "UV drag lost its value: PBR={pbr}, channel={channel}, field={property}: {before:?} -> {after:?}"
                );
                for other in (0..5).filter(|other| *other != property) {
                    assert!((after[other] - before[other]).abs() < 1e-5, "Dragging one UV field changed another");
                }
            }
            let (_, texts) = inspector.show(vec![]);
            let glow = texts
                .iter()
                .find(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-glow"))
                .unwrap()
                .1;
            let point = texts
                .iter()
                .find(|(text, rect)| {
                    aurora_i18n::parse_number(text).is_some()
                        && rect.left() > glow.right()
                        && (rect.center().y - glow.center().y).abs() < 10.0
                })
                .unwrap()
                .1
                .center();
            drag(&mut inspector, point);
            assert!(inspector.world.objects.get(idx).unwrap().te.as_ref().unwrap().face(0).glow > 0.0);
        }
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
    inspector.settings.transform_expanded = false;
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
            .find(|(text, _)| text == aurora_i18n::tr!("build-layout-transform"))
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
    let idx = inspector.world.objects.index_of_uuid(&object_id).unwrap();
    inspector.world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
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
    inspector.screen.y = 1300.0;
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
    let actions = inspector
        .tool
        .ui
        .layout
        .targets
        .iter()
        .find(|(item, panel, _)| *item == crate::build::layout::Item::Actions && *panel == crate::build::layout::Panel::Footer)
        .unwrap()
        .2;
    assert!(expanded.bottom() - actions.bottom() < 30.0);
    let uuid = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-uuid"))
        .unwrap()
        .1;
    assert!(expanded.right() - uuid.right() < 30.0, "UUID must follow the right edge");
    for tab in [6, 5, 3, 2, 4] {
        inspector.tool.tab = tab;
        let (rect, texts) = inspector.show(vec![]);
        assert!(
            (rect.size() - expanded.size()).length() < 1.0,
            "Switching section changed the chosen size: {tab}; {expanded:?} -> {rect:?}"
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
            aurora_i18n::tr!("build-compact-stretch-textures"),
        ] {
            let matches: Vec<_> = texts.iter().filter(|(text, _)| text == label).collect();
            assert_eq!(matches.len(), 1, "Shared option missing or duplicated: {label}");
            assert!(
                expanded.bottom() - matches[0].1.bottom() < 195.0,
                "Shared option must be in the footer: {label}; window={expanded:?}, control={:?}",
                matches[0].1
            );
        }
    }
    // Folding the sections must restore the user's size rather than leaving
    // the newly resizable body at its title-bar height.
    for expanding in [false, true] {
        // Let auto-height settle and separate these deliberate clicks from
        // the preceding resize/section gesture's double-click interval.
        for _ in 0..30 {
            inspector.show(vec![]);
        }
        let icon = if inspector.settings.expanded {
            "caret-up-filled"
        } else {
            "caret-down-filled"
        };
        let point = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("floater_action", super::super::INSPECTOR_ID, icon))))
            .unwrap()
            .center();
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
            assert!(rect.height() < 85.0);
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
        assert_eq!(
            inspector
                .tool
                .ui
                .layout
                .targets
                .iter()
                .any(|(item, panel, _)| { *item == crate::build::layout::Item::Actions && *panel == crate::build::layout::Panel::Footer }),
            tool == Tool::Create
        );
        assert_eq!(
            texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-compact-linked")),
            tool == Tool::Create
        );
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

/// Selects the transparency field of the texture section and types `text`,
/// then finishes with `finish`.
fn type_transparency(inspector: &mut Inspector, text: &str, finish: egui::Key) -> String {
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let caption = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-transparency"))
        .unwrap()
        .1;
    let (shown, field) = texts
        .iter()
        .find(|(_, rect)| (rect.center().y - caption.center().y).abs() < 8.0 && rect.left() > caption.right())
        .cloned()
        .unwrap();
    inspector.show(vec![egui::Event::PointerMoved(field.center())]);
    for pressed in [true, false] {
        inspector.show(vec![egui::Event::PointerButton {
            pos: field.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    assert!(inspector.ctx.memory(|m| m.focused()).is_some(), "The field must take the keyboard");
    inspector.tool.out.clear();
    if !text.is_empty() {
        inspector.show(vec![egui::Event::Text(text.into())]);
    }
    inspector.show(vec![egui::Event::Key {
        key: finish,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    inspector.show(vec![]);
    shown
}

/// #1956: a linked part selected whole after one of its faces was made
/// transparent shows a mixed transparency. Typing the value hidden under the
/// "—" (the first face's 0 %) applies it to every face, as LLSpinCtrl::
/// onEditorCommit + LLPanelFace::sendAlpha (selectionSetAlphaOnly) do; a
/// face picked alone takes the value alone. Both window styles.
#[test]
fn typed_transparency_reaches_every_selected_face_even_over_a_mixed_value() {
    let sent = |inspector: &Inspector| {
        inspector
            .tool
            .out
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetTextures { .. })
                )
            })
            .count()
    };
    for legacy in [false, true] {
        let style = if legacy { "Legacy" } else { "Aurora" };
        for (text, finish, applied) in [
            ("0", egui::Key::Enter, true),
            ("0", egui::Key::Tab, true),
            ("0", egui::Key::Escape, false),
            ("", egui::Key::Enter, false),
        ] {
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 3;
            let idx = inspector.tool.sel_prims(&inspector.world)[0];
            let n = inspector.tool.num_faces(&inspector.world, idx);
            let object = inspector.world.objects.get_mut(idx).unwrap();
            Arc::make_mut(object.te.as_mut().unwrap()).faces[2].color[3] = 0.0;
            let shown = type_transparency(&mut inspector, text, finish);
            assert_eq!(shown, "—", "{style}: a whole prim with one transparent face is mixed");
            let te = inspector.world.objects.get(idx).unwrap().te.clone().unwrap();
            let alphas: Vec<f32> = te.faces[..n].iter().map(|f| f.color[3]).collect();
            let all_opaque = alphas.iter().all(|a| *a == 1.0);
            assert_eq!(all_opaque, applied, "{style} {text:?} {finish:?}: {alphas:?}");
            assert_eq!(
                sent(&inspector),
                usize::from(applied),
                "{style} {text:?} {finish:?}: one ObjectImage"
            );
        }

        // One face picked alone: only that face changes.
        let mut inspector = material_inspector();
        inspector.settings.legacy_window = legacy;
        inspector.tool.tab = 3;
        inspector.tool.edit_mode = EditMode::Face;
        inspector.tool.faces.insert(inspector.tool.selection[0], vec![2]);
        let idx = inspector.tool.sel_prims(&inspector.world)[0];
        let n = inspector.tool.num_faces(&inspector.world, idx);
        let shown = type_transparency(&mut inspector, "100", egui::Key::Enter);
        assert_eq!(shown, "0", "{style}: one opaque face");
        let te = inspector.world.objects.get(idx).unwrap().te.clone().unwrap();
        for (face, f) in te.faces[..n].iter().enumerate() {
            assert_eq!(f.color[3], if face == 2 { 0.0 } else { 1.0 }, "{style}: face {face}");
        }
        assert_eq!(sent(&inspector), 1);
    }
}

fn press(inspector: &mut Inspector, point: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(point)]);
    for pressed in [true, false] {
        inspector.show(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
}

fn key(inspector: &mut Inspector, key: egui::Key) {
    inspector.show(vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
}

/// The UV field `property` (scale U / V, offset U / V, rotation) with its
/// column title: the title's rect, then the field's shown text and rect.
fn uv_field(texts: &[(String, egui::Rect)], property: usize) -> Option<(egui::Rect, (String, egui::Rect))> {
    let (label, width, row) = match property {
        0 | 1 => (aurora_i18n::tr!("common-scale"), 103.0, property),
        2 | 3 => (aurora_i18n::tr!("build-ui-texture-offset"), 75.0, property - 2),
        _ => (aurora_i18n::tr!("build-ui-texture-rotation"), 71.0, 0),
    };
    let title = texts.iter().find(|(text, _)| text == label)?.1;
    let mut fields: Vec<_> = texts
        .iter()
        .filter(|(text, rect)| {
            (text == "—" || aurora_i18n::parse_number(text).is_some())
                && rect.top() > title.bottom()
                && rect.top() < title.bottom() + 70.0
                && rect.left() >= title.left()
                && rect.right() < title.left() + width
        })
        .cloned()
        .collect();
    fields.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    Some((title, fields.get(row)?.clone()))
}

/// How a typed UV entry ends in `typed_uv_value_*`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Finish {
    Enter,
    Tab,
    ClickAway,
    Escape,
    /// Enter with nothing typed: the "—" stays untouched.
    Untouched,
}

/// #1956, UV half: every UV component (scale U / V, offset U / V, rotation)
/// is mixed over a whole prim whose face 1 differs in all five. Typing the
/// first face's hidden value into one of them applies that component alone
/// to every face, as LLPanelFace sends each spinner's commit on its own
/// (onCommitTextureScaleX… for Blinn-Phong, onCommitGLTFTextureScaleU… and
/// updateGLTFTextureTransform for PBR); face 1 keeps its other components.
/// Escape or an untouched "—" sends nothing. Both window styles.
#[test]
fn typed_uv_value_over_a_mixed_field_reaches_every_face_and_only_that_component() {
    const ODD: [f32; 5] = [2.0, 3.0, 0.25, -0.25, 0.5];
    const FIRST: [f32; 5] = [1.0, 1.0, 0.0, 0.0, 0.0];
    const TYPED: [&str; 5] = ["1", "1", "0", "0", "0"];
    let asset = uuid::Uuid::from_u128(0x1956);
    fn values(inspector: &Inspector, pbr: bool, idx: usize, face: u8) -> [[f32; 5]; 4] {
        if pbr {
            let over = BuildTool::gltf_override(&inspector.world, idx, face);
            over.transforms
                .map(|t| [t.scale[0], t.scale[1], t.offset[0], t.offset[1], t.rotation])
        } else {
            let f = inspector.world.objects.get(idx).unwrap().te.as_ref().unwrap().face(face as usize);
            [[f.scale_s, f.scale_t, f.offset_s, f.offset_t, f.rotation]; 4]
        }
    }
    for legacy in [false, true] {
        for pbr in [false, true] {
            for (property, typed) in TYPED.into_iter().enumerate() {
                for finish in [Finish::Enter, Finish::Tab, Finish::ClickAway, Finish::Escape, Finish::Untouched] {
                    let case = format!("legacy={legacy} pbr={pbr} field={property} {finish:?}");
                    let mut inspector = material_inspector();
                    inspector.settings.legacy_window = legacy;
                    inspector.tool.tab = 3;
                    inspector.tool.ui.tex_tab = usize::from(pbr);
                    let key_ = inspector.tool.selection[0];
                    let idx = inspector.world.objects.index_of(&key_).unwrap();
                    let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
                    if pbr {
                        inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(u8::MAX, asset)];
                        inspector.scene.materials.insert(asset, Default::default());
                        let odd = aurora_assets::material::TextureTransform {
                            scale: [ODD[0], ODD[1]],
                            offset: [ODD[2], ODD[3]],
                            rotation: ODD[4],
                        };
                        let over = aurora_assets::material::PbrOverride {
                            transforms: [odd; 4],
                            ..Default::default()
                        };
                        inspector.world.gltf_overrides.insert(key_, Arc::from(vec![(1u8, over)]));
                        inspector.list_contents(idx);
                    } else {
                        let object = inspector.world.objects.get_mut(idx).unwrap();
                        let te = Arc::make_mut(object.te.as_mut().unwrap());
                        let f = &mut te.faces[1];
                        [f.scale_s, f.scale_t, f.offset_s, f.offset_t, f.rotation] = ODD;
                    }
                    inspector.show(vec![]);
                    crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(390.0, 1500.0));
                    for _ in 0..5 {
                        inspector.show(vec![]);
                    }
                    let (_, texts) = inspector.show(vec![]);
                    let (title, (shown, field)) = uv_field(&texts, property).unwrap_or_else(|| panic!("{case}: missing UV field"));
                    assert_eq!(shown, "—", "{case}: the field is mixed");
                    press(&mut inspector, field.center());
                    assert!(
                        inspector.ctx.memory(|m| m.focused()).is_some(),
                        "{case}: the field must take the keyboard"
                    );
                    inspector.tool.out.clear();
                    if finish != Finish::Untouched {
                        inspector.show(vec![egui::Event::Text(typed.into())]);
                    }
                    match finish {
                        Finish::Enter | Finish::Untouched => key(&mut inspector, egui::Key::Enter),
                        Finish::Tab => key(&mut inspector, egui::Key::Tab),
                        Finish::Escape => key(&mut inspector, egui::Key::Escape),
                        Finish::ClickAway => press(&mut inspector, title.center()),
                    }
                    inspector.show(vec![]);
                    let applied = matches!(finish, Finish::Enter | Finish::Tab | Finish::ClickAway);
                    // Escape also closes the selection (Deselect), which is no edit.
                    let edits: Vec<_> = inspector
                        .tool
                        .out
                        .iter()
                        .filter(|command| !matches!(command, aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Deselect { .. })))
                        .collect();
                    assert_eq!(edits.len(), usize::from(applied), "{case}: one edit, or none: {edits:?}");
                    for face in 0..n {
                        let expected = if face == 1 { ODD } else { FIRST };
                        for (channel, got) in values(&inspector, pbr, idx, face).iter().enumerate() {
                            for component in 0..5 {
                                // A face without an override keeps the identity
                                // until a committed component lands on it.
                                let want = if applied && component == property {
                                    FIRST[component]
                                } else {
                                    expected[component]
                                };
                                assert!(
                                    (got[component] - want).abs() < 1e-5,
                                    "{case}: face {face} channel {channel} component {component}: {got:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// #1956, review of PR #2814: a PBR material's own UV transform is
/// inherited by a face with no override. `PbrMaterial::apply_override`
/// takes a scale or an offset as a whole pair, so editing U alone must keep
/// the V that face shows (the base material's), on one channel and under
/// "All", in what is applied and in what is sent. Both window styles.
#[test]
fn a_pbr_uv_edit_keeps_the_other_component_inherited_from_the_material() {
    let asset = uuid::Uuid::from_u128(0x2814);
    let base = aurora_assets::material::TextureTransform {
        scale: [2.0, 3.0],
        offset: [0.2, 0.3],
        rotation: 0.0,
    };
    for legacy in [false, true] {
        for channel in [0usize, 1] {
            for (property, typed, expected) in [(0, "4", ([4.0, 3.0], [0.2, 0.3])), (2, "0.4", ([2.0, 3.0], [0.4, 0.3]))] {
                let case = format!("legacy={legacy} channel={channel} field={property}");
                let mut inspector = material_inspector();
                inspector.settings.legacy_window = legacy;
                inspector.tool.tab = 3;
                inspector.tool.ui.tex_tab = 1;
                inspector.tool.ui.pbr_channel = channel;
                let key_ = inspector.tool.selection[0];
                let idx = inspector.world.objects.index_of(&key_).unwrap();
                let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
                inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(u8::MAX, asset)];
                inspector.scene.materials.insert(
                    asset,
                    aurora_assets::PbrMaterial {
                        transforms: [base; 4],
                        ..Default::default()
                    },
                );
                inspector.list_contents(idx);
                inspector.show(vec![]);
                crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(390.0, 1500.0));
                for _ in 0..5 {
                    inspector.show(vec![]);
                }
                let (_, texts) = inspector.show(vec![]);
                let (_, (shown, field)) = uv_field(&texts, property).unwrap_or_else(|| panic!("{case}: missing UV field"));
                let material_value = if property == 0 { base.scale[0] } else { base.offset[0] };
                assert!(
                    aurora_i18n::parse_number(&shown).is_some_and(|v| (v as f32 - material_value).abs() < 1e-5),
                    "{case}: the field shows the material's value, not {shown}"
                );
                press(&mut inspector, field.center());
                inspector.tool.out.clear();
                inspector.show(vec![egui::Event::Text(typed.into())]);
                key(&mut inspector, egui::Key::Enter);
                inspector.show(vec![]);
                let edited: Vec<usize> = if channel == 0 { (0..4).collect() } else { vec![channel - 1] };
                // applied: the material under each face's new override
                for face in 0..n {
                    let mut material = aurora_assets::PbrMaterial {
                        transforms: [base; 4],
                        ..Default::default()
                    };
                    material.apply_override(&BuildTool::gltf_override(&inspector.world, idx, face));
                    for (c, t) in material.transforms.iter().enumerate() {
                        let want = if edited.contains(&c) { expected } else { (base.scale, base.offset) };
                        assert_eq!((t.scale, t.offset), want, "{case}: face {face} channel {c}");
                        assert_eq!(t.rotation, 0.0);
                    }
                }
                // sent: the KHR_texture_transform of each face's override
                let sent: Vec<serde_json::Value> = inspector
                    .tool
                    .out
                    .iter()
                    .filter_map(|command| match command {
                        aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Cap { cap, body, .. })
                            if cap == "ModifyMaterialParams" =>
                        {
                            Some(body.as_array().to_vec())
                        }
                        _ => None,
                    })
                    .flatten()
                    .map(|entry| serde_json::from_str(entry.get("gltf_json").as_str()).expect("override JSON"))
                    .collect();
                assert_eq!(sent.len(), usize::from(n), "{case}: one override per face");
                // the edited pair whole, the untouched one left to the material
                let sent_pairs = if property == 0 {
                    (expected.0, [0.0, 0.0])
                } else {
                    ([1.0, 1.0], expected.1)
                };
                for json in &sent {
                    let transform =
                        &json["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["extensions"]["KHR_texture_transform"];
                    let pair = |name: &str| -> Option<[f32; 2]> {
                        Some([transform[name][0].as_f64()? as f32, transform[name][1].as_f64()? as f32])
                    };
                    assert_eq!(
                        (pair("scale"), pair("offset")),
                        (Some(sent_pairs.0), Some(sent_pairs.1)),
                        "{case}: sent {json}"
                    );
                }
            }
        }
    }
}

/// The value under a row's caption: the first number or "—" to its right,
/// on the same line.
fn row_field(texts: &[(String, egui::Rect)], caption: &str) -> Option<(String, egui::Rect)> {
    let label = texts.iter().find(|(text, _)| text == caption)?.1;
    texts
        .iter()
        .filter(|(text, rect)| {
            (text == "—" || aurora_i18n::parse_number(text).is_some())
                && (rect.center().y - label.center().y).abs() < 8.0
                && rect.left() > label.right()
        })
        .min_by(|a, b| a.1.left().total_cmp(&b.1.left()))
        .cloned()
}

/// #1956: the other mixed texture spinners take a value typed over "—" too,
/// even when it equals the first face's hidden value (LLSpinCtrl::
/// onEditorCommit, then LLPanelFace's per-field commits). Face 1 differs,
/// the first face's value is typed and Enter pressed: every face takes it.
/// Both window styles; the Legacy style's mask cutoff needs a loaded
/// texture with alpha and is checked by hand in the demo.
#[test]
fn typed_value_over_the_other_mixed_texture_fields_reaches_every_face() {
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Field {
        Glow,
        RepeatsPerMeter,
        Glossiness,
        Environment,
        MaskCutoff,
    }
    use crate::scene::legacy_mat::LegacyMaterial;
    fn material(inspector: &Inspector, idx: usize, face: u8) -> LegacyMaterial {
        let object = inspector.world.objects.get(idx).unwrap();
        let te = object.te.as_ref().unwrap().face(face as usize);
        match inspector.tool.ui.legacy_pending.get(&(object.full_id, face)) {
            Some((id, m)) if *id == te.material_id => m.clone().unwrap_or_default(),
            _ => inspector
                .scene
                .legacy_mats
                .peek(&te.material_id)
                .map(|m| (*m).clone())
                .unwrap_or_default(),
        }
    }
    fn value(inspector: &Inspector, field: Field, idx: usize, face: u8) -> f32 {
        let object = inspector.world.objects.get(idx).unwrap();
        let te = object.te.as_ref().unwrap().face(face as usize);
        let m = material(inspector, idx, face);
        match field {
            Field::Glow => te.glow,
            Field::RepeatsPerMeter => {
                let (sa, ta) = texture::te_st_axes(face);
                let rpm = (te.scale_s / object.scale[sa].max(0.001)).max(te.scale_t / object.scale[ta].max(0.001));
                (rpm * 10.0).round() / 10.0
            }
            Field::Glossiness => f32::from(m.specular_exp),
            Field::Environment => f32::from(m.env_intensity),
            Field::MaskCutoff => f32::from(m.alpha_cutoff),
        }
    }
    let fields = [
        Field::Glow,
        Field::RepeatsPerMeter,
        Field::Glossiness,
        Field::Environment,
        Field::MaskCutoff,
    ];
    for legacy in [false, true] {
        for field in fields {
            if legacy && field == Field::MaskCutoff {
                continue;
            }
            let case = format!("legacy={legacy} {field:?}");
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 3;
            inspector.tool.ui.tex_tab = 0;
            let idx = inspector.tool.sel_prims(&inspector.world)[0];
            let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
            let full_id = inspector.world.objects.get(idx).unwrap().full_id;
            // a specular map (glossiness, environment) or Mask mode (cutoff)
            // on every face, face 1 with its own values
            let first = LegacyMaterial {
                specular_map: texture::DEFAULT_SPECULAR,
                specular_exp: 51,
                env_intensity: 10,
                diffuse_alpha_mode: crate::scene::legacy_mat::alpha_mode::MASK,
                alpha_cutoff: 127,
                ..Default::default()
            };
            let odd = LegacyMaterial {
                specular_exp: 200,
                env_intensity: 90,
                alpha_cutoff: 30,
                ..first.clone()
            };
            let object = inspector.world.objects.get_mut(idx).unwrap();
            let te = Arc::make_mut(object.te.as_mut().unwrap());
            for face in 0..n {
                let (id, m) = if face == 1 {
                    (uuid::Uuid::from_u128(0x1957), odd.clone())
                } else {
                    (uuid::Uuid::from_u128(0x1956), first.clone())
                };
                te.faces[face as usize].material_id = id;
                inspector.scene.legacy_mats.insert(id, m.clone());
                inspector.tool.ui.legacy_pending.insert((full_id, face), (id, Some(m)));
            }
            match field {
                Field::Glow => te.faces[1].glow = 0.5,
                Field::RepeatsPerMeter => {
                    te.faces[1].scale_s *= 3.0;
                    te.faces[1].scale_t *= 3.0;
                }
                _ => {}
            }
            let typed = value(&inspector, field, idx, 0);
            assert_ne!(typed, value(&inspector, field, idx, 1), "{case}: face 1 differs");
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            let caption = match field {
                Field::Glow => aurora_i18n::tr!("build-ui-texture-glow"),
                Field::RepeatsPerMeter => aurora_i18n::tr!("build-ui-texture-repeats-meter"),
                Field::Glossiness if legacy => aurora_i18n::tr!("build-legacy-glossiness"),
                Field::Glossiness => aurora_i18n::tr!("build-ui-texture-shininess"),
                Field::Environment => aurora_i18n::tr!("common-environment"),
                Field::MaskCutoff => aurora_i18n::tr!("build-ui-texture-mask-cutoff"),
            };
            let (shown, rect) = row_field(&texts, caption).unwrap_or_else(|| panic!("{case}: no field beside {caption}"));
            assert_eq!(shown, "—", "{case}: the field is mixed");
            press(&mut inspector, rect.center());
            assert!(
                inspector.ctx.memory(|m| m.focused()).is_some(),
                "{case}: the field must take the keyboard"
            );
            inspector.show(vec![egui::Event::Text(aurora_i18n::number_text(&typed.to_string()))]);
            key(&mut inspector, egui::Key::Enter);
            inspector.show(vec![]);
            for face in 0..n {
                let got = value(&inspector, field, idx, face);
                assert!((got - typed).abs() < 1e-4, "{case}: face {face} has {got}, typed {typed}");
            }
        }
    }
}

/// #1956: unlike the texture fields, the object's position, size and
/// rotation still send only a value that differs from the object's
/// (LLPanelObject::sendPosition, indra/newview/llpanelobject.cpp): typing
/// the primary object's X (the last selected) over a mixed X sends nothing,
/// another value moves the selection. Both window styles.
#[test]
fn a_typed_position_over_a_mixed_axis_is_sent_only_when_it_changes() {
    for legacy in [false, true] {
        // the last object selected is the primary one the fields show
        for (typed, moves) in [("102", false), ("101", true)] {
            let case = format!("legacy={legacy} typed={typed}");
            let mut inspector = Inspector::new();
            inspector.settings.legacy_window = legacy;
            let first = inspector.tool.sel_prims(&inspector.world)[0];
            let second = inspector.world.objects.index_of_uuid(&crate::demo::action_id(119)).unwrap();
            let base = inspector.world.objects.get(first).unwrap().position;
            inspector.world.objects.get_mut(first).unwrap().position = glam::Vec3::new(100.0, base.y, base.z);
            let other = inspector.world.objects.get_mut(second).unwrap();
            other.position = glam::Vec3::new(102.0, base.y, base.z);
            other.update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
            let id = other.full_id;
            let owner = inspector.world.agent_id;
            inspector.tool.props.insert(
                id,
                ObjectProps {
                    object_id: id,
                    full: true,
                    owner_id: owner,
                    creator_id: owner,
                    owner_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    base_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    ..Default::default()
                },
            );
            inspector
                .tool
                .click_select(&mut inspector.world, &inspector.settings, Some(second), true);
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            // a mixed axis field: "—" with its axis letter just to its left
            let mixed: Vec<_> = texts
                .iter()
                .filter(|(text, rect)| {
                    text == "—"
                        && texts.iter().any(|(letter, l)| {
                            ["X", "Y", "Z"].contains(&letter.as_str())
                                && (l.center().y - rect.center().y).abs() < 10.0
                                && l.right() <= rect.left()
                                && rect.left() - l.right() < 40.0
                        })
                })
                .collect();
            // the top row is the position; the demo objects' sizes differ too
            let top = mixed.iter().map(|(_, r)| r.top()).fold(f32::INFINITY, f32::min);
            let position: Vec<_> = mixed.iter().filter(|(_, r)| (r.top() - top).abs() < 4.0).collect();
            assert_eq!(position.len(), 1, "{case}: only the position's X is mixed: {mixed:?}");
            press(&mut inspector, position[0].1.center());
            assert!(
                inspector.ctx.memory(|m| m.focused()).is_some(),
                "{case}: the field must take the keyboard"
            );
            inspector.tool.out.clear();
            inspector.show(vec![egui::Event::Text(typed.into())]);
            key(&mut inspector, egui::Key::Enter);
            inspector.show(vec![]);
            let sent = inspector.tool.out.iter().any(|command| {
                matches!(
                    command,
                    aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Transform { .. })
                )
            });
            assert_eq!(sent, moves, "{case}: {:?}", inspector.tool.out);
        }
    }
}

/// Escape over a typed mixed transparency, with the selection kept (Escape
/// only turns "Place on surface" off), also when a window drawn earlier
/// takes the key. Nothing may change on that frame nor on the frames after.
#[test]
fn escape_over_a_typed_mixed_value_changes_nothing_while_the_selection_stays() {
    for legacy in [false, true] {
        for (typed, taken) in [("0", false), ("50", false), ("0", true), ("50", true)] {
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.escape_taken_first = taken;
            inspector.tool.tab = 3;
            let idx = inspector.tool.sel_prims(&inspector.world)[0];
            let n = inspector.tool.num_faces(&inspector.world, idx);
            let object = inspector.world.objects.get_mut(idx).unwrap();
            Arc::make_mut(object.te.as_mut().unwrap()).faces[2].color[3] = 0.0;
            inspector.tool.place.active = true;
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            let (shown, field) = row_field(&texts, aurora_i18n::tr!("build-ui-texture-transparency")).expect("transparency field");
            assert_eq!(shown, "—");
            press(&mut inspector, field.center());
            inspector.tool.out.clear();
            inspector.show(vec![egui::Event::Text(typed.into())]);
            key(&mut inspector, egui::Key::Escape);
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            assert!(!inspector.tool.selection.is_empty(), "legacy={legacy}: the selection stays");
            let te = inspector.world.objects.get(idx).unwrap().te.clone().unwrap();
            let alphas: Vec<f32> = te.faces[..n].iter().map(|f| f.color[3]).collect();
            assert_eq!(
                alphas[2], 0.0,
                "legacy={legacy} typed={typed} taken={taken}: Escape changed {alphas:?}"
            );
            assert!(
                !inspector
                    .tool
                    .out
                    .iter()
                    .any(|c| matches!(c, aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetTextures { .. }))),
                "legacy={legacy} typed={typed} taken={taken}: {:?}",
                inspector.tool.out
            );
        }
    }
}

/// Escape in a build text entry drops what was typed and sends nothing, as
/// LLLineEditor and LLSpinCtrl::handleKeyHere revert it, and is not also
/// the build tool's Close. Name and sale price (a spin_i), both window
/// styles, with Escape seen by the field and with Escape taken first by a
/// window drawn before the build tools.
#[test]
fn escape_reverts_every_build_text_entry_and_keeps_the_tool_open() {
    use aurora_net::{NetCommand, build::BuildCmd};
    let select_all = egui::Modifiers {
        ctrl: true,
        command: true,
        ..Default::default()
    };
    let type_over = |inspector: &mut Inspector, at: egui::Pos2, text: &str| {
        press(inspector, at);
        inspector.show(vec![
            egui::Event::ModifiersChanged(select_all),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: select_all,
            },
        ]);
        inspector.show(vec![
            egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            egui::Event::Text(text.into()),
        ]);
    };
    for legacy in [false, true] {
        for taken in [false, true] {
            let case = format!("legacy={legacy} escape taken first={taken}");
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 0;
            inspector.escape_taken_first = taken;
            let idx = inspector.tool.sel_prims(&inspector.world)[0];
            let id = inspector.world.objects.get(idx).unwrap().full_id;
            {
                let props = inspector.tool.props.get_mut(&id).unwrap();
                props.sale_type = 2;
                props.sale_price = 10;
            }
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            let name = texts.iter().find(|(text, _)| text == "Presentation test").expect("name field").1;
            inspector.tool.out.clear();
            type_over(&mut inspector, name.center(), "Changed");
            key(&mut inspector, egui::Key::Escape);
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            assert!(inspector.tool.open, "{case}: Escape in Name closed the build tool");
            assert_eq!(inspector.tool.props[&id].name, "Presentation test", "{case}");
            assert!(
                !inspector
                    .tool
                    .out
                    .iter()
                    .any(|c| matches!(c, NetCommand::Build(BuildCmd::SetName { .. }))),
                "{case}: {:?}",
                inspector.tool.out
            );
            let (_, texts) = inspector.show(vec![]);
            assert!(
                texts.iter().any(|(text, _)| text == "Presentation test"),
                "{case}: the name shows again"
            );

            let (shown, price) = row_field(&texts, aurora_i18n::currency()).expect("sale price field");
            assert_eq!(shown, "10", "{case}");
            type_over(&mut inspector, price.center(), "99");
            key(&mut inspector, egui::Key::Escape);
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            assert!(inspector.tool.open, "{case}: Escape in the price closed the build tool");
            assert_eq!(inspector.tool.props[&id].sale_price, 10, "{case}");
            assert!(inspector.tool.ui.sale_draft.is_none(), "{case}: {:?}", inspector.tool.ui.sale_draft);
            assert!(
                !inspector
                    .tool
                    .out
                    .iter()
                    .any(|c| matches!(c, NetCommand::Build(BuildCmd::SetSaleInfo { .. }))),
                "{case}: {:?}",
                inspector.tool.out
            );

            // The same entries with Enter are sent, once.
            type_over(&mut inspector, price.center(), "99");
            key(&mut inspector, egui::Key::Enter);
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            let sales = inspector
                .tool
                .out
                .iter()
                .filter(|c| matches!(c, NetCommand::Build(BuildCmd::SetSaleInfo { .. })))
                .count();
            assert_eq!(sales, 1, "{case}: {:?}", inspector.tool.out);
            assert_eq!(inspector.tool.props[&id].sale_price, 99, "{case}");
        }
    }
}

/// A value typed for part A, then a press on part B in the world. As
/// LLTool::handleAnyMouseClick, the press takes the keyboard first
/// (app.rs drops egui's focus on the raw press) and waits for one
/// interface pass (ClickWait::hold_for_ui, app/click_pick.rs) before the
/// tool selects: the field commits to A in that pass, then B is selected
/// and keeps its own value. Both styles, whole objects and the Face tool
/// (which selects on the press itself).
#[test]
fn a_value_typed_for_one_selection_lands_on_it_not_on_the_next() {
    for legacy in [false, true] {
        for face_mode in [false, true] {
            let case = format!("legacy={legacy} face mode={face_mode}");
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 3;
            let a = inspector.tool.sel_prims(&inspector.world)[0];
            let b = inspector.world.objects.index_of_uuid(&crate::demo::action_id(119)).unwrap();
            for idx in [a, b] {
                let object = inspector.world.objects.get_mut(idx).unwrap();
                object.update_flags |= crate::ui::context::flags::OBJECT_MOVE | crate::ui::context::flags::OBJECT_MODIFY;
                Arc::make_mut(object.te.as_mut().unwrap()).faces[2].color[3] = 0.5;
            }
            let id = inspector.world.objects.get(b).unwrap().full_id;
            let owner = inspector.world.agent_id;
            inspector.tool.props.insert(
                id,
                ObjectProps {
                    object_id: id,
                    full: true,
                    owner_id: owner,
                    creator_id: owner,
                    owner_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    base_mask: perm::COPY | perm::MODIFY | perm::MOVE | perm::TRANSFER,
                    ..Default::default()
                },
            );
            if face_mode {
                inspector.tool.edit_mode = EditMode::Face;
                inspector.tool.faces.insert(inspector.tool.selection[0], vec![2]);
            }
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            let (_, field) = row_field(&texts, aurora_i18n::tr!("build-ui-texture-transparency")).expect("transparency field");
            press(&mut inspector, field.center());
            inspector.show(vec![egui::Event::Text("0".into())]);
            // the raw press in the world: the field lets the keyboard go,
            // the press waits for the next pass
            inspector.ctx.memory_mut(|m| {
                if let Some(focused) = m.focused() {
                    m.surrender_focus(focused);
                }
            });
            inspector.show(vec![]);
            let a_te = inspector.world.objects.get(a).unwrap().te.clone().unwrap();
            assert_eq!(a_te.faces[2].color[3], 1.0, "{case}: A takes the value typed for it");
            // then the held press selects B
            inspector
                .tool
                .click_select(&mut inspector.world, &inspector.settings, Some(b), false);
            if face_mode {
                let b_id = inspector.tool.selection[0];
                inspector.tool.faces.insert(b_id, vec![2]);
            }
            inspector.tool.out.clear();
            for _ in 0..4 {
                inspector.show(vec![]);
            }
            let b_te = inspector.world.objects.get(b).unwrap().te.clone().unwrap();
            assert_eq!(b_te.faces[2].color[3], 0.5, "{case}: B took the value typed for A");
            assert!(
                !inspector
                    .tool
                    .out
                    .iter()
                    .any(|c| matches!(c, aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetTextures { .. }))),
                "{case}: {:?}",
                inspector.tool.out
            );
        }
    }
}

/// A press on the build window outside the typing field takes its focus at
/// once, as LLButton::handleMouseDown does: the field commits to the face it
/// was typed for before the release, where the next part / face button, the
/// edit mode or Edit linked change the selection. Face 2 picked alone, a
/// transparency typed, a press on the window's own surface (a label), then
/// the selection moves to face 3 on the release: face 2 takes the value,
/// face 3 keeps its own. Both styles.
#[test]
fn a_press_on_the_build_window_commits_the_field_before_the_release() {
    for legacy in [false, true] {
        let mut inspector = material_inspector();
        inspector.settings.legacy_window = legacy;
        inspector.tool.tab = 3;
        inspector.tool.edit_mode = EditMode::Face;
        let key_ = inspector.tool.selection[0];
        inspector.tool.faces.insert(key_, vec![2]);
        let idx = inspector.tool.sel_prims(&inspector.world)[0];
        for face in [2, 3] {
            let object = inspector.world.objects.get_mut(idx).unwrap();
            Arc::make_mut(object.te.as_mut().unwrap()).faces[face].color[3] = 0.5;
        }
        for _ in 0..5 {
            inspector.show(vec![]);
        }
        let caption = aurora_i18n::tr!("build-ui-texture-transparency");
        let (_, texts) = inspector.show(vec![]);
        let (shown, field) = row_field(&texts, caption).expect("transparency field");
        assert_eq!(shown, "50", "legacy={legacy}");
        let label = texts.iter().find(|(text, _)| text == caption).expect("transparency label").1;
        press(&mut inspector, field.center());
        inspector.show(vec![egui::Event::Text("0".into())]);
        inspector.show(vec![egui::Event::PointerMoved(label.center())]);
        inspector.show(vec![egui::Event::PointerButton {
            pos: label.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        let te = inspector.world.objects.get(idx).unwrap().te.clone().unwrap();
        assert_eq!(te.faces[2].color[3], 1.0, "legacy={legacy}: the press commits to face 2");
        // the release: a next-face button would select face 3 here
        inspector.tool.faces.insert(key_, vec![3]);
        inspector.show(vec![egui::Event::PointerButton {
            pos: label.center(),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        for _ in 0..3 {
            inspector.show(vec![]);
        }
        let te = inspector.world.objects.get(idx).unwrap().te.clone().unwrap();
        assert_eq!(te.faces[3].color[3], 0.5, "legacy={legacy}: face 3 took the value typed for face 2");
    }
}

/// The pair a PBR UV edit keeps comes from what the face inherits on the
/// server. While a face's material still loads the UV fields wait (typing
/// sends nothing), then the edit keeps that material's V. A face showing a
/// local material keeps its server material's V, never the preview's,
/// which only this viewer has. Both window styles.
#[test]
fn a_pbr_uv_edit_waits_for_the_materials_and_ignores_a_local_preview() {
    use aurora_assets::material::TextureTransform;
    let t = |s: [f32; 2], o: [f32; 2]| TextureTransform {
        scale: s,
        offset: o,
        rotation: 0.0,
    };
    let first = uuid::Uuid::from_u128(0x2814);
    let second = uuid::Uuid::from_u128(0x2815);
    let local = uuid::Uuid::from_u128(0x2816);
    let first_transforms = [t([2.0, 3.0], [0.2, 0.3]); 4];
    let second_transforms = [t([6.0, 7.0], [0.7, 0.8]); 4];
    let local_transforms = [t([9.0, 11.0], [0.9, 0.1]); 4];
    let material = |transforms| aurora_assets::PbrMaterial {
        transforms,
        ..Default::default()
    };
    for legacy in [false, true] {
        let setup = |with_local: bool| {
            let mut inspector = material_inspector();
            inspector.settings.legacy_window = legacy;
            inspector.tool.tab = 3;
            inspector.tool.ui.tex_tab = 1;
            inspector.tool.ui.pbr_channel = 0;
            let key_ = inspector.tool.selection[0];
            let idx = inspector.world.objects.index_of(&key_).unwrap();
            let object = inspector.world.objects.get_mut(idx).unwrap();
            object.extra.render_materials = vec![(1, second), (u8::MAX, first)];
            if with_local {
                object
                    .render
                    .local_appearance
                    .insert(Some(0), crate::world::objects::LocalAppearance::Material(local));
                inspector.world.local_assets.insert(local, ("local".into(), false));
                inspector.scene.materials.insert(local, material(local_transforms));
            }
            inspector.scene.materials.insert(first, material(first_transforms));
            inspector.list_contents(idx);
            inspector.show(vec![]);
            crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(390.0, 1500.0));
            (inspector, idx)
        };
        let type_scale_u = |inspector: &mut Inspector| {
            for _ in 0..5 {
                inspector.show(vec![]);
            }
            let (_, texts) = inspector.show(vec![]);
            let (_, (_, field)) = uv_field(&texts, 0).expect("scale U field");
            press(inspector, field.center());
            inspector.show(vec![egui::Event::Text(aurora_i18n::number_text("4"))]);
            key(inspector, egui::Key::Enter);
            inspector.show(vec![]);
        };
        let overrides = |inspector: &Inspector, idx: usize| {
            let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
            (0..n)
                .map(|face| BuildTool::gltf_override(&inspector.world, idx, face).transforms)
                .collect::<Vec<_>>()
        };

        // face 1's material is not there yet: nothing is written
        let (mut inspector, idx) = setup(false);
        let before = overrides(&inspector, idx);
        type_scale_u(&mut inspector);
        assert_eq!(
            overrides(&inspector, idx),
            before,
            "legacy={legacy}: written while a material loads"
        );
        // it arrives: the edit keeps each face's own V
        inspector.scene.materials.insert(second, material(second_transforms));
        type_scale_u(&mut inspector);
        let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
        for face in 0..n {
            let base = if face == 1 { second_transforms } else { first_transforms };
            let mut rendered = material(base);
            rendered.apply_override(&BuildTool::gltf_override(&inspector.world, idx, face));
            for (c, (shown, inherited)) in rendered.transforms.iter().zip(&base).enumerate() {
                assert_eq!(shown.scale, [4.0, inherited.scale[1]], "legacy={legacy}: face {face} channel {c}");
            }
        }

        // a local material on face 0: its server override keeps the server V
        let (mut inspector, idx) = setup(true);
        inspector.scene.materials.insert(second, material(second_transforms));
        type_scale_u(&mut inspector);
        let mut rendered = material(first_transforms);
        rendered.apply_override(&BuildTool::gltf_override(&inspector.world, idx, 0));
        for (c, (shown, inherited)) in rendered.transforms.iter().zip(&first_transforms).enumerate() {
            assert_eq!(
                shown.scale,
                [4.0, inherited.scale[1]],
                "legacy={legacy}: the local preview's V reached the server, channel {c}"
            );
        }
    }
}

/// #1956, review of PR #2814: the inherited pair is kept per face and per
/// channel, never copied from the first face or channel. Face 1 has its own
/// material, the four channels of each material differ, and face 2 already
/// overrides its scale and offset. Each U and V field of scale and offset,
/// typed over "—" on "All", Base color and Emissive: every face and channel
/// renders what it rendered before with only the typed component changed
/// (the material after `apply_override`). Both window styles.
#[test]
fn a_pbr_uv_edit_keeps_each_faces_and_channels_own_pair() {
    use aurora_assets::material::{PbrOverride, TextureTransform};
    let t = |s: [f32; 2], o: [f32; 2]| TextureTransform {
        scale: s,
        offset: o,
        rotation: 0.0,
    };
    let first = uuid::Uuid::from_u128(0x2814);
    let second = uuid::Uuid::from_u128(0x2815);
    let first_transforms = [
        t([2.0, 3.0], [0.2, 0.3]),
        t([1.5, 2.5], [0.1, 0.25]),
        t([3.0, 0.5], [0.35, 0.05]),
        t([0.75, 1.25], [0.15, 0.6]),
    ];
    let second_transforms = [
        t([6.0, 7.0], [0.7, 0.8]),
        t([2.25, 0.25], [0.55, 0.65]),
        t([1.75, 3.5], [0.45, 0.15]),
        t([2.75, 0.6], [0.05, 0.95]),
    ];
    let own_pairs = [t([5.0, 6.0], [0.5, 0.6]); 4];
    let typed = [("4", 4.0f32), ("4.5", 4.5), ("0.4", 0.4), ("0.45", 0.45)];
    for legacy in [false, true] {
        for channel in [0usize, 1, 4] {
            for (property, (text, value)) in typed.into_iter().enumerate() {
                let case = format!("legacy={legacy} channel={channel} field={property}");
                let mut inspector = material_inspector();
                inspector.settings.legacy_window = legacy;
                inspector.tool.tab = 3;
                inspector.tool.ui.tex_tab = 1;
                inspector.tool.ui.pbr_channel = channel;
                let key_ = inspector.tool.selection[0];
                let idx = inspector.world.objects.index_of(&key_).unwrap();
                let n = inspector.tool.num_faces(&inspector.world, idx) as u8;
                inspector.world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(1, second), (u8::MAX, first)];
                for (asset, transforms) in [(first, first_transforms), (second, second_transforms)] {
                    inspector.scene.materials.insert(
                        asset,
                        aurora_assets::PbrMaterial {
                            transforms,
                            ..Default::default()
                        },
                    );
                }
                inspector.world.gltf_overrides.insert(
                    key_,
                    vec![(
                        2u8,
                        PbrOverride {
                            transforms: own_pairs,
                            ..Default::default()
                        },
                    )]
                    .into(),
                );
                let rendered = |inspector: &Inspector, face: u8| {
                    let mut material = aurora_assets::PbrMaterial {
                        transforms: if face == 1 { second_transforms } else { first_transforms },
                        ..Default::default()
                    };
                    material.apply_override(&BuildTool::gltf_override(&inspector.world, idx, face));
                    material.transforms
                };
                let before: Vec<[TextureTransform; 4]> = (0..n).map(|face| rendered(&inspector, face)).collect();
                inspector.list_contents(idx);
                inspector.show(vec![]);
                crate::ui::widgets::size_floater(&inspector.ctx, super::super::INSPECTOR_ID, egui::vec2(390.0, 1500.0));
                for _ in 0..5 {
                    inspector.show(vec![]);
                }
                let (_, texts) = inspector.show(vec![]);
                let (_, (shown, field)) = uv_field(&texts, property).unwrap_or_else(|| panic!("{case}: missing UV field"));
                assert_eq!(shown, "—", "{case}: the faces differ");
                press(&mut inspector, field.center());
                inspector.show(vec![egui::Event::Text(aurora_i18n::number_text(text))]);
                key(&mut inspector, egui::Key::Enter);
                inspector.show(vec![]);
                let edited: Vec<usize> = if channel == 0 { (0..4).collect() } else { vec![channel - 1] };
                for face in 0..n {
                    let after = rendered(&inspector, face);
                    for c in 0..4 {
                        let mut want = before[face as usize][c];
                        if edited.contains(&c) {
                            match property {
                                0 => want.scale[0] = value,
                                1 => want.scale[1] = value,
                                2 => want.offset[0] = value,
                                _ => want.offset[1] = value,
                            }
                        }
                        assert_eq!(after[c], want, "{case}: face {face} channel {c}");
                    }
                }
            }
        }
    }
}
