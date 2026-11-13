//! Real inspector selection, focus routing and task-inventory commands.
use super::*;
use aurora_net::{
    NetCommand,
    build::{BuildCmd, TaskItem},
};
use std::collections::HashSet;

fn fixture() -> (Inspector, uuid::Uuid, Vec<uuid::Uuid>) {
    let mut inspector = Inspector::new();
    inspector.settings.transform_expanded = false;
    inspector.tool.tab = 4;
    let object = inspector
        .world
        .objects
        .get(inspector.tool.contents_target(&inspector.world).unwrap())
        .unwrap()
        .full_id;
    let ids = (0..4).map(|i| uuid::Uuid::from_u128(97000 + i)).collect::<Vec<_>>();
    let items = ids
        .iter()
        .enumerate()
        .map(|(i, id)| TaskItem {
            item_id: *id,
            parent_id: object,
            owner_id: inspector.world.agent_id,
            name: format!("Entry {i:03}"),
            asset_type: 10,
            inv_type: 10,
            owner_mask: perm::COPY | perm::MODIFY | perm::TRANSFER,
            ..Default::default()
        })
        .collect();
    inspector.tool.on_task_inventory(&inspector.world, object, Some(1), items);
    for _ in 0..4 {
        inspector.show(vec![]);
    }
    inspector.tool.out.clear();
    (inspector, object, ids)
}

fn pointer(inspector: &mut Inspector, pos: egui::Pos2, mods: egui::Modifiers, button: egui::PointerButton) {
    inspector.show(vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        inspector.show(vec![egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: mods,
        }]);
    }
}

fn click_text(inspector: &mut Inspector, name: &str, mods: egui::Modifiers, button: egui::PointerButton) {
    let texts = inspector.show(vec![]).1;
    let point = texts
        .iter()
        .find(|(text, _)| text.starts_with(name))
        .unwrap_or_else(|| panic!("Missing {name}"))
        .1
        .center();
    pointer(inspector, point, mods, button);
}

fn click(inspector: &mut Inspector, i: usize, mods: egui::Modifiers) {
    click_text(inspector, &format!("Entry {i:03}"), mods, egui::PointerButton::Primary);
}

fn key(inspector: &mut Inspector, key: egui::Key, modifiers: egui::Modifiers) {
    inspector.show(
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            })
            .collect(),
    );
}

fn removed(inspector: &Inspector) -> Vec<uuid::Uuid> {
    inspector
        .tool
        .out
        .iter()
        .filter_map(|cmd| match cmd {
            NetCommand::Build(BuildCmd::RemoveTaskInventory { item_id, .. }) => Some(*item_id),
            _ => None,
        })
        .collect()
}

fn renamed(inspector: &Inspector) -> Vec<&TaskItem> {
    inspector
        .tool
        .out
        .iter()
        .filter_map(|cmd| match cmd {
            NetCommand::Build(BuildCmd::UpdateTaskInventory { item, .. }) => Some(&**item),
            _ => None,
        })
        .collect()
}

fn assert_no_object_delete(inspector: &Inspector) {
    assert!(
        !inspector
            .tool
            .out
            .iter()
            .any(|cmd| matches!(cmd, NetCommand::Build(BuildCmd::DeRez { .. }))),
        "Contents Delete reached the world object"
    );
}

#[test]
fn contents_cmt_menu_selects_each_display_without_editing_or_sending_commands() {
    use crate::ui::inventory::permissions::Rights;
    let (mut inspector, object, _) = fixture();
    let original = inspector.tool.contents.objects[&object].items.clone();
    let before = inspector.show(vec![]).1;
    assert!(
        before
            .iter()
            .any(|(text, _)| text.starts_with("Entry 000") && text.contains("CMT") && !text.contains('→'))
    );
    let texts = inspector.show(vec![]).1;
    let text_rect = |label| texts.iter().find(|(text, _)| text == aurora_i18n::text(label)).unwrap().1;
    let new_script = text_rect("common-new-script");
    let reset = text_rect("build-ui-contents-reset-scripts");
    let refresh = text_rect("build-ui-contents-refresh");
    assert!(new_script.right() < reset.left());
    assert!(reset.right() < refresh.left());
    assert!((new_script.center().y - reset.center().y).abs() < 1.0);
    assert!((refresh.center().y - reset.center().y).abs() < 1.0);
    for (choice, key, count) in [
        (Rights::Both, "inventory-rights-both", 2),
        (Rights::Next, "inventory-rights-next", 1),
        (Rights::Hidden, "inventory-rights-hidden", 0),
        (Rights::Mine, "inventory-rights-mine", 1),
    ] {
        let rect = inspector
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("contents-rights-button")))
            .unwrap();
        let rights = text_rect("common-rights");
        assert!(rights.right() < rect.left());
        assert!((rights.center().y - rect.center().y).abs() < 1.0);
        let search = inspector
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("contents-search-field")))
            .unwrap();
        assert!(rect.right() < search.left());
        assert!((search.center().y - rect.center().y).abs() < 1.0);
        pointer(&mut inspector, rect.center(), egui::Modifiers::NONE, egui::PointerButton::Primary);
        click_text(
            &mut inspector,
            aurora_i18n::text(key),
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert_eq!(inspector.tool.ui.contents_rights, choice);
        let shown = inspector.show(vec![]).1;
        let entry = &shown.iter().find(|(text, _)| text.starts_with("Entry 000")).unwrap().0;
        assert_eq!(entry.matches("CMT").count(), count);
        assert_eq!(entry.contains('→'), choice.shows_next());
    }
    assert!(
        inspector.tool.out.is_empty(),
        "Display must not emit task-inventory or permission commands"
    );
    for (before, after) in original.iter().zip(&inspector.tool.contents.objects[&object].items) {
        assert_eq!(
            (before.owner_mask, before.next_owner_mask),
            (after.owner_mask, after.next_owner_mask)
        );
    }
}

#[test]
fn contents_ctrl_shift_and_context_selection_follow_the_visible_order() {
    let (mut inspector, _, ids) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    click(&mut inspector, 2, egui::Modifiers::CTRL);
    assert_eq!(inspector.tool.ui.contents_selection.items, HashSet::from([ids[0], ids[2]]));
    click(&mut inspector, 0, egui::Modifiers::CTRL);
    assert_eq!(inspector.tool.ui.contents_selection.items, HashSet::from([ids[2]]));
    click(&mut inspector, 1, egui::Modifiers::NONE);
    click(&mut inspector, 3, egui::Modifiers::SHIFT);
    assert_eq!(
        inspector.tool.ui.contents_selection.items,
        ids[1..].iter().copied().collect::<HashSet<_>>()
    );
    click(&mut inspector, 0, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
    assert_eq!(
        inspector.tool.ui.contents_selection.items,
        ids.iter().copied().collect::<HashSet<_>>()
    );
    click_text(&mut inspector, "Entry 002", egui::Modifiers::NONE, egui::PointerButton::Secondary);
    assert_eq!(
        inspector.tool.ui.contents_selection.items.len(),
        4,
        "Right click preserves the selected batch"
    );
}

#[test]
fn contents_context_copy_keeps_selected_mixed_types_in_one_request_in_both_layouts() {
    for legacy in [false, true] {
        for selected in [1, 2] {
            let (mut inspector, object, ids) = fixture();
            inspector.settings.legacy_window = legacy;
            let items = &mut inspector.tool.contents.objects.get_mut(&object).unwrap().items;
            items[0].asset_type = 7;
            items[0].inv_type = 7;
            items[2].asset_type = 6;
            items[2].inv_type = 6;
            resize_inspector(&mut inspector, vec2(400.0, 820.0));
            inspector.tool.ui.requests.clear();
            click(&mut inspector, 0, egui::Modifiers::NONE);
            if selected == 2 {
                click(&mut inspector, 2, egui::Modifiers::CTRL);
            }
            click_text(&mut inspector, "Entry 000", egui::Modifiers::NONE, egui::PointerButton::Secondary);
            click_text(
                &mut inspector,
                aurora_i18n::tr!("build-ui-contents-copy-to-inventory"),
                egui::Modifiers::NONE,
                egui::PointerButton::Primary,
            );
            let expected = if selected == 1 { vec![ids[0]] } else { vec![ids[0], ids[2]] };
            assert!(
                matches!(&inspector.tool.ui.requests[..],
                    [crate::build::ui::Request::CopyContents { object: source, items }]
                    if *source == object && items.iter().map(|item| item.item_id).collect::<Vec<_>>() == expected
                ),
                "legacy={legacy}, selected={selected}: one folder request, with only the selection"
            );
            assert!(
                !inspector
                    .tool
                    .out
                    .iter()
                    .any(|cmd| matches!(cmd, NetCommand::Build(BuildCmd::MoveTaskInventory { .. }))),
                "the menu must wait for the common folder and move confirmation"
            );
        }
    }
}

#[test]
fn legacy_contents_delete_targets_items_in_every_tool_mode_with_visible_tabs() {
    for mode in [Tool::Edit, Tool::Focus, Tool::Grab, Tool::Create] {
        let (mut inspector, _, ids) = fixture();
        inspector.settings.legacy_window = true;
        inspector.tool.set_tool(mode);
        resize_inspector(&mut inspector, vec2(400.0, 820.0));
        click(&mut inspector, 0, egui::Modifiers::NONE);
        key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(removed(&inspector), [ids[0]], "Tool: {mode:?}");
        assert_no_object_delete(&inspector);
    }
}

#[test]
fn contents_delete_removes_only_selected_items_and_repeated_keys_never_delete_the_object() {
    let (mut inspector, object, ids) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    click(&mut inspector, 2, egui::Modifiers::CTRL);
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(removed(&inspector), [ids[0], ids[2]]);
    assert_eq!(
        inspector.tool.contents.objects[&object]
            .items
            .iter()
            .map(|it| it.item_id)
            .collect::<Vec<_>>(),
        [ids[1], ids[3]]
    );
    inspector.show(vec![egui::Event::Key {
        key: egui::Key::Delete,
        physical_key: Some(egui::Key::Delete),
        pressed: true,
        repeat: true,
        modifiers: egui::Modifiers::NONE,
    }]);
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(removed(&inspector).len(), 2);
    assert_no_object_delete(&inspector);
}

#[test]
fn contents_readonly_and_filtered_selection_do_not_fall_back_to_object_delete() {
    let (mut inspector, object, _) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    key(&mut inspector, egui::Key::A, egui::Modifiers::CTRL);
    assert_eq!(inspector.tool.ui.contents_selection.items.len(), 4);
    inspector.tool.ui.contents_filter = "Entry 003".into();
    inspector.show(vec![]);
    assert_eq!(inspector.tool.ui.contents_selection.items.len(), 1);
    inspector.tool.props.get_mut(&object).unwrap().owner_mask &= !perm::MODIFY;
    let idx = inspector.world.objects.index_of_uuid(&object).unwrap();
    inspector.world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(removed(&inspector).is_empty());
    assert_no_object_delete(&inspector);
}

#[test]
fn contents_f2_renames_one_item_and_enter_commits_without_deleting_text_as_an_item() {
    let (mut inspector, object, ids) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    key(&mut inspector, egui::Key::F2, egui::Modifiers::NONE);
    assert_eq!(inspector.tool.ui.rename.as_ref().unwrap().item, ids[0]);
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(removed(&inspector).is_empty());
    inspector.show(vec![egui::Event::Text("Renamed script".into())]);
    key(&mut inspector, egui::Key::Enter, egui::Modifiers::NONE);
    assert!(inspector.tool.ui.rename.is_none());
    let sent = renamed(&inspector);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].item_id, ids[0]);
    assert_eq!(sent[0].name, "Renamed script");
    assert!(
        inspector
            .ctx
            .memory(|m| m.has_focus(super::super::super::contents_selection::focus_id(object))),
        "Enter returns focus to the list"
    );
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(removed(&inspector), [ids[0]]);
    assert_no_object_delete(&inspector);
}

#[test]
fn contents_context_rename_commits_on_blur_or_tab_change_and_escape_cancels() {
    for action in 0..4 {
        let (mut inspector, _, ids) = fixture();
        click_text(&mut inspector, "Entry 000", egui::Modifiers::NONE, egui::PointerButton::Secondary);
        click_text(
            &mut inspector,
            aurora_i18n::tr!("common-rename"),
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        inspector.show(vec![]);
        assert!(inspector.tool.ui.rename.is_some());
        inspector.show(vec![egui::Event::Text("New name".into())]);
        assert!(
            inspector.tool.ui.rename.is_some(),
            "Typing must keep rename active for action {action}"
        );
        match action {
            0 => click(&mut inspector, 2, egui::Modifiers::NONE),
            1 => {
                inspector.tool.tab = 6;
                inspector.show(vec![]);
            }
            2 => key(&mut inspector, egui::Key::Escape, egui::Modifiers::NONE),
            _ => click_text(
                &mut inspector,
                aurora_i18n::tr!("build-ui-contents-enter-filter-text"),
                egui::Modifiers::NONE,
                egui::PointerButton::Primary,
            ),
        }
        inspector.show(vec![]);
        assert!(inspector.tool.ui.rename.is_none(), "Renamer retained focus for action {action}");
        let sent = renamed(&inspector);
        if action == 2 {
            assert!(sent.is_empty());
        } else {
            assert_eq!(sent.len(), 1, "Commit for action {action}");
            assert_eq!((sent[0].item_id, sent[0].name.as_str()), (ids[0], "New name"));
        }
    }
}

#[test]
fn contents_f2_requires_one_modifiable_item_and_world_click_releases_the_list_focus() {
    let (mut inspector, object, _) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    click(&mut inspector, 1, egui::Modifiers::CTRL);
    key(&mut inspector, egui::Key::F2, egui::Modifiers::NONE);
    assert!(inspector.tool.ui.rename.is_none());
    click(&mut inspector, 3, egui::Modifiers::NONE);
    inspector.tool.contents.objects.get_mut(&object).unwrap().items[3].owner_mask &= !perm::MODIFY;
    key(&mut inspector, egui::Key::F2, egui::Modifiers::NONE);
    assert!(inspector.tool.ui.rename.is_none());
    key(&mut inspector, egui::Key::Num1, egui::Modifiers::CTRL);
    assert_eq!(inspector.tool.tool, Tool::Focus, "List focus must retain Ctrl+1 tool navigation");
    key(&mut inspector, egui::Key::Num3, egui::Modifiers::CTRL);
    assert_eq!(inspector.tool.tool, Tool::Edit);
    click(&mut inspector, 0, egui::Modifiers::NONE);
    pointer(
        &mut inspector,
        egui::pos2(1000.0, 800.0),
        egui::Modifiers::NONE,
        egui::PointerButton::Primary,
    );
    inspector.tool.out.clear();
    key(&mut inspector, egui::Key::Delete, egui::Modifiers::NONE);
    assert!(removed(&inspector).is_empty());
    assert!(
        inspector
            .tool
            .out
            .iter()
            .any(|cmd| matches!(cmd, NetCommand::Build(BuildCmd::DeRez { .. }))),
        "World focus must retain object Delete"
    );
}

/// AZERTY Ctrl+4 is logical Quote with physical Num4. Contents must still
/// give that key to the build tools (menu_viewer.xml control|4).
#[test]
fn azerty_ctrl_4_leaves_contents_focus_and_opens_create() {
    let (mut inspector, _, _) = fixture();
    click(&mut inspector, 0, egui::Modifiers::NONE);
    assert_eq!(inspector.tool.tool, Tool::Edit);
    let modifiers = egui::Modifiers {
        ctrl: true,
        ..Default::default()
    };
    inspector.show(vec![egui::Event::Key {
        key: egui::Key::Quote,
        physical_key: Some(egui::Key::Num4),
        pressed: true,
        repeat: false,
        modifiers,
    }]);
    assert_eq!(inspector.tool.tool, Tool::Create);
    assert!(inspector.tool.open);
}
