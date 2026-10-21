//! Contents-property commits and bulk permissions through the real UI/protocol.
use super::*;
use crate::build::{BuildSettings, Tool};
use aurora_net::NetCommand;
use std::sync::Arc;

fn fixture() -> (World, BuildTool, Uuid, Vec<TaskItem>) {
    let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    for event in crate::demo::events() {
        world.apply(event);
    }
    let idx = world.objects.index_of_uuid(&crate::demo::action_id(118)).unwrap();
    world.objects.get_mut(idx).unwrap().update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
    let object = world.objects.get(idx).unwrap().full_id;
    let mut tool = BuildTool::default();
    tool.open_build(Tool::Edit);
    tool.click_select(&mut world, &BuildSettings::default(), Some(idx), false);
    tool.props.insert(
        object,
        aurora_net::build::ObjectProps {
            owner_id: world.agent_id,
            base_mask: u32::MAX,
            owner_mask: perm::MODIFY | perm::COPY | perm::TRANSFER | perm::MOVE,
            ..Default::default()
        },
    );
    let items = TYPES
        .iter()
        .enumerate()
        .map(|(i, t)| TaskItem {
            item_id: Uuid::from_u128(99000 + i as u128),
            parent_id: object,
            owner_id: world.agent_id,
            creator_id: world.agent_id,
            name: format!("Item {i}"),
            asset_type: *t,
            inv_type: *t,
            base_mask: perm::MODIFY | perm::COPY | perm::TRANSFER | perm::MOVE,
            owner_mask: perm::MODIFY | perm::COPY | perm::TRANSFER | perm::MOVE,
            next_owner_mask: perm::MOVE | perm::COPY | perm::TRANSFER,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    tool.on_task_inventory(&world, object, Some(1), items.clone());
    tool.out.clear();
    (world, tool, object, items)
}

fn updates(tool: &BuildTool) -> Vec<&TaskItem> {
    tool.out
        .iter()
        .filter_map(|cmd| match cmd {
            NetCommand::Build(BuildCmd::UpdateTaskInventory { item, .. }) => Some(&**item),
            _ => None,
        })
        .collect()
}

#[test]
fn all_eleven_bulk_filters_fetch_and_update_only_their_type_with_valid_masks() {
    let (world, mut tool, object, mut items) = fixture();
    items.push(TaskItem {
        item_id: object,
        is_folder: true,
        asset_type: 10,
        ..Default::default()
    });
    items.push(TaskItem {
        item_id: Uuid::from_u128(99999),
        asset_type: 49,
        ..Default::default()
    });
    for (i, t) in TYPES.iter().enumerate() {
        tool.out.clear();
        tool.ui.bulk.options.types.fill(false);
        tool.ui.bulk.options.types[i] = true;
        tool.ui.bulk.options.share = true;
        tool.ui.bulk.options.everyone_copy = true;
        tool.ui.bulk.options.next_copy = false;
        tool.ui.bulk.options.next_transfer = false;
        tool.start_bulk_permissions(&world);
        assert!(updates(&tool).is_empty(), "Wait for fresh contents, even when cached");
        assert!(
            tool.out
                .iter()
                .any(|cmd| matches!(cmd, NetCommand::Build(BuildCmd::RequestTaskInventory { object_id, .. }) if *object_id == object))
        );
        tool.on_task_inventory(&world, object, Some(2), items.clone());
        // An in-flight job keeps the applied choices, not subsequent UI changes.
        tool.ui.bulk.options.types.fill(false);
        tool.ui.bulk.options.share = false;
        tool.advance_bulk_permissions(&world, Instant::now());
        tool.advance_bulk_permissions(&world, Instant::now());
        let sent = updates(&tool);
        assert_eq!(sent.len(), 1, "Filter for type {t}");
        assert_eq!(sent[0].asset_type, *t);
        assert_eq!(sent[0].group_mask, perm::MOVE | perm::MODIFY | perm::COPY);
        assert_eq!(sent[0].everyone_mask, perm::COPY);
        assert_eq!(
            sent[0].next_owner_mask,
            perm::MOVE | perm::TRANSFER | if *t == 56 { perm::COPY } else { 0 }
        );
        assert!(tool.ui.bulk.job.is_none());
        tool.advance_bulk_permissions(&world, Instant::now());
        assert_eq!(updates(&tool).len(), 1, "No duplicate sends after completion");
    }
}

#[test]
fn bulk_skips_revoked_failed_or_timed_out_sources_without_updating_cached_items() {
    for failure in 0..3 {
        let (mut world, mut tool, object, _) = fixture();
        tool.start_bulk_permissions(&world);
        match failure {
            0 => {
                let idx = world.objects.index_of_uuid(&object).unwrap();
                world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            }
            1 => tool.replacement_task_inventory_error(object, "Offline test error".into()),
            _ => {}
        }
        let now = Instant::now() + if failure == 2 { Duration::from_secs(31) } else { Duration::ZERO };
        tool.advance_bulk_permissions(&world, now);
        assert_eq!(tool.ui.bulk.job.as_ref().unwrap().skipped, 1);
        tool.advance_bulk_permissions(&world, now);
        assert!(tool.ui.bulk.job.is_none());
        assert!(updates(&tool).is_empty());
    }
}

#[test]
fn task_property_save_keeps_the_source_and_uses_update_task_inventory() {
    let (world, mut tool, object, items) = fixture();
    let mut state = super::super::item_properties::TaskProperties::new(object, &items[0]).unwrap();
    state.edit.name = "Renamed animation".into();
    state.edit.desc = "Description".into();
    state.edit.next = perm::MOVE | perm::MODIFY | perm::TRANSFER;
    state.edit.group = perm::MODIFY | perm::COPY | perm::MOVE;
    state.edit.everyone = perm::COPY;
    state.edit.sale_type = 2;
    state.edit.price = 42;
    // Changing the build selection must not redirect an open item's save.
    tool.selection.clear();
    assert!(tool.save_item_properties(&world, &state));
    assert_eq!(updates(&tool).len(), 1);
    let sent = updates(&tool)[0];
    assert_eq!(sent.parent_id, object);
    assert_eq!(sent.item_id, items[0].item_id);
    assert_eq!(sent.name, state.edit.name);
    assert_eq!(sent.description, state.edit.desc);
    assert_eq!(sent.next_owner_mask, state.edit.next);
    assert_eq!(sent.group_mask, state.edit.group);
    assert_eq!(sent.everyone_mask, perm::COPY);
    assert_eq!((sent.sale_type, sent.sale_price), (2, 42));
    assert!(matches!(&tool.out[..], [NetCommand::Build(BuildCmd::UpdateTaskInventory { .. })]));
}

#[test]
fn task_property_save_rechecks_access_and_preserves_nonmodifiable_item_text() {
    for revoke_object in [false, true] {
        let (mut world, mut tool, object, items) = fixture();
        let mut state = super::super::item_properties::TaskProperties::new(object, &items[0]).unwrap();
        state.edit.name = "Changed".into();
        state.edit.desc = "Changed".into();
        if revoke_object {
            let idx = world.objects.index_of_uuid(&object).unwrap();
            world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
            assert!(!tool.save_item_properties(&world, &state));
            assert!(updates(&tool).is_empty());
        } else {
            tool.contents.objects.get_mut(&object).unwrap().items[0].owner_mask &= !perm::MODIFY;
            assert!(tool.save_item_properties(&world, &state));
            assert_eq!(updates(&tool)[0].name, items[0].name);
            assert_eq!(updates(&tool)[0].description, items[0].description);
        }
    }
}

fn frame(ctx: &egui::Context, tool: &mut BuildTool, world: &World, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            focused: true,
            ..Default::default()
        },
        |_| show(ctx, &crate::theme::Theme::default().palette(), tool, world),
    );
    output.textures_delta.clear();
    fn text(shape: &egui::Shape, result: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => result.push((t.galley.text().to_owned(), t.galley.rect.translate(t.pos.to_vec2()))),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    text(s, result);
                }
            }
            _ => {}
        }
    }
    let mut result = Vec::new();
    for s in output.shapes {
        text(&s.shape, &mut result);
    }
    result
}

fn click(ctx: &egui::Context, tool: &mut BuildTool, world: &World, label: &str) {
    let texts = frame(ctx, tool, world, vec![]);
    let point = texts
        .iter()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("Missing button {label}"))
        .1
        .center();
    frame(ctx, tool, world, vec![egui::Event::PointerMoved(point)]);
    for pressed in [true, false] {
        frame(
            ctx,
            tool,
            world,
            vec![egui::Event::PointerButton {
                pos: point,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            }],
        );
    }
}

#[test]
fn bulk_buttons_select_types_cancel_drafts_and_distinguish_apply_from_ok() {
    let (world, mut tool, object, items) = fixture();
    let ctx = egui::Context::default();
    tool.ui.bulk_perms_open = true;
    for _ in 0..4 {
        frame(&ctx, &mut tool, &world, vec![]);
    }
    click(&ctx, &mut tool, &world, aurora_i18n::tr!("common-clear"));
    assert!(tool.ui.bulk.options.types.iter().all(|on| !on));
    click(&ctx, &mut tool, &world, aurora_i18n::tr!("common-all"));
    assert!(tool.ui.bulk.options.types.iter().all(|on| *on));
    tool.ui.bulk.options.share = true;
    click(&ctx, &mut tool, &world, aurora_i18n::tr!("common-cancel"));
    assert!(!tool.ui.bulk_perms_open);
    assert!(!tool.ui.bulk.options.share);
    assert!(updates(&tool).is_empty());
    tool.ui.bulk_perms_open = true;
    click(&ctx, &mut tool, &world, aurora_i18n::tr!("common-apply"));
    assert!(tool.ui.bulk_perms_open);
    assert!(tool.ui.bulk.job.is_some());
    tool.on_task_inventory(&world, object, Some(2), items);
    for _ in 0..3 {
        tool.advance_bulk_permissions(&world, Instant::now());
    }
    assert_eq!(updates(&tool).len(), 11);
    click(&ctx, &mut tool, &world, aurora_i18n::tr!("common-ok"));
    assert!(!tool.ui.bulk_perms_open);
    assert!(tool.ui.bulk.job.is_some(), "OK closes the panel, not the pending request");
}

/// The bulk permissions window keeps its size, its buttons inside it.
#[test]
fn bulk_permissions_keep_their_size() {
    crate::ui::widgets::steady_tests::assert_window_steady(
        "bulk_perms",
        &[Vec2::new(360.0, 354.0)],
        aurora_i18n::tr!("common-cancel"),
        || {
            let (world, mut tool, _, _) = fixture();
            tool.ui.bulk_perms_open = true;
            (world, tool)
        },
        |(world, tool), ctx, p| show(ctx, p, tool, world),
    );
}
