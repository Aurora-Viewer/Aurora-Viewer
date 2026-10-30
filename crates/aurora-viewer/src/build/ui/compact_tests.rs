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
}

impl Inspector {
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
