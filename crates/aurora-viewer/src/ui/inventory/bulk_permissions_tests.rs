use super::*;
use aurora_net::inventory::operations::Operation;

fn fixture() -> (Inventory, InventoryPreferences, Uuid) {
    let agent = Uuid::from_u128(2);
    let mut inv = Inventory::default();
    crate::world::inventory::demo::seed(&mut inv, agent);
    (inv, InventoryPreferences::default(), agent)
}
fn job(roots: &[u128], options: BulkPerms) -> Job {
    let mut state = State {
        options,
        ..Default::default()
    };
    state.open(roots.iter().copied().map(Uuid::from_u128).collect());
    state.start();
    state.job.take().unwrap()
}
fn finish(job: &mut Job, inv: &mut Inventory, prefs: &InventoryPreferences, agent: Uuid) -> Mutation {
    for _ in 0..30 {
        if let Some(change) = job.advance(inv, prefs, agent).unwrap() {
            return change;
        }
    }
    panic!("Bulk planning did not finish");
}
fn targets(change: &Mutation) -> HashSet<Uuid> {
    change
        .operations
        .iter()
        .map(|op| match op {
            Operation::Patch { id, folder: false, body } => {
                assert!(
                    body.as_map()
                        .unwrap()
                        .keys()
                        .all(|key| matches!(key.as_str(), "permissions" | "flags")),
                    "Only permissions and their object overrides may change"
                );
                assert!(body.has("permissions"));
                *id
            }
            _ => panic!("Expected an item permission PATCH"),
        })
        .collect()
}

#[test]
fn inventory_bulk_menu_supports_multiple_items_and_folders_with_pinned_targets() {
    let (inv, mut prefs, agent) = fixture();
    let facts = Facts {
        agent,
        ..Default::default()
    };
    let mut st = InventoryUi::default();
    let ids = [8100, 8102].map(Uuid::from_u128);
    st.selection.extend(ids);
    let catalog = super::super::context_catalog::build(&inv, ids[0], &st, &prefs, &facts);
    let entry = catalog.entries.iter().find(|e| e.id == "bulk-permissions").unwrap();
    assert!(entry.enabled);
    let super::super::menu_layout::Kind::Action(index) = entry.kind else {
        panic!()
    };
    let Some(super::super::context_catalog::Command::BulkPermissions(roots)) = &catalog.commands[index] else {
        panic!()
    };
    assert_eq!(roots.iter().copied().collect::<HashSet<_>>(), HashSet::from(ids));
    st.selection.clear();
    let folder = Uuid::from_u128(8000);
    let catalog = super::super::context_catalog::build(&inv, folder, &st, &prefs, &facts);
    assert!(catalog.entries.iter().any(|e| e.id == "bulk-permissions" && e.enabled));
    prefs.protected.insert(folder);
    let catalog = super::super::context_catalog::build(&inv, folder, &st, &prefs, &facts);
    assert!(catalog.entries.iter().any(|e| e.id == "bulk-permissions" && !e.enabled));
    let catalog = super::super::context_catalog::build(&inv, inv.lib_root, &st, &prefs, &facts);
    assert!(!catalog.entries.iter().any(|e| e.id == "bulk-permissions" && e.enabled));
}

#[test]
fn inventory_bulk_descendants_are_deduplicated_and_restricted_items_and_links_are_skipped() {
    let (mut inv, mut prefs, agent) = fixture();
    let nested = Uuid::from_u128(8001);
    let script = Uuid::from_u128(8102);
    let mut item = inv.items[&script].clone();
    item.parent = nested;
    item.owner_mask &= !rules::MODIFY;
    inv.add_items(vec![item]);
    inv.folders
        .get_mut(&Uuid::from_u128(8000))
        .unwrap()
        .items
        .retain(|id| *id != script);
    prefs.protected.insert(Uuid::from_u128(8103));
    inv.items.get_mut(&Uuid::from_u128(8104)).unwrap().owner = Uuid::from_u128(900);
    let mut plan = job(&[8000, 8001, 8102, 8000], BulkPerms::default());
    let change = finish(&mut plan, &mut inv, &prefs, agent);
    let ids = targets(&change);
    assert!(ids.contains(&script), "No Modify asset still allows next-owner permissions");
    assert_eq!(change.operations.len(), ids.len(), "No duplicate PATCH");
    assert!(!ids.contains(&Uuid::from_u128(8103)));
    assert!(!ids.contains(&Uuid::from_u128(8104)));
    assert!(ids.iter().all(|id| !matches!(inv.items[id].asset_type, 24 | 25)));
    assert_eq!(change.refresh.iter().copied().collect::<HashSet<_>>().len(), change.refresh.len());
}

#[test]
fn inventory_bulk_waits_for_all_folders_and_sends_nothing_on_read_failure_or_timeout() {
    let (mut inv, prefs, agent) = fixture();
    let child = Uuid::from_u128(8001);
    inv.folders.get_mut(&child).unwrap().state = FetchState::Unknown;
    let mut plan = job(&[8000], BulkPerms::default());
    assert!(plan.advance(&mut inv, &prefs, agent).unwrap().is_none());
    assert!(plan.change.operations.is_empty());
    assert_eq!(inv.queue, vec![(child, false)]);
    inv.folders.get_mut(&child).unwrap().state = FetchState::Failed;
    assert!(plan.advance(&mut inv, &prefs, agent).is_err());
    inv.folders.get_mut(&child).unwrap().state = FetchState::Fetched;
    let change = finish(&mut plan, &mut inv, &prefs, agent);
    assert!(!change.operations.is_empty());
    let mut plan = job(&[8000], BulkPerms::default());
    plan.started = Instant::now() - Duration::from_secs(121);
    assert!(plan.advance(&mut inv, &prefs, agent).is_err());
}

#[test]
fn inventory_bulk_type_filters_and_fair_use_respect_base_and_sharing_rights() {
    let (mut inv, prefs, agent) = fixture();
    for (index, kind) in [20, 13, 5, 21, 7, 6, 10, 1, 0, 56, 57].into_iter().enumerate() {
        let mut options = BulkPerms::default();
        options.types.fill(false);
        options.types[index] = true;
        let change = finish(&mut job(&[8000], options), &mut inv, &prefs, agent);
        let ids = targets(&change);
        assert!(!ids.is_empty());
        assert!(ids.iter().all(|id| inv.items[id].asset_type == kind));
    }
    let mut item = inv.items[&Uuid::from_u128(8100)].clone();
    let options = BulkPerms {
        share: true,
        everyone_copy: true,
        next_modify: true,
        next_copy: false,
        next_transfer: false,
        ..Default::default()
    };
    for mask in [rules::COPY | rules::MODIFY | rules::TRANSFER, rules::COPY, rules::TRANSFER] {
        item.base_mask = mask;
        item.owner_mask = mask;
        let (group, everyone, next) = permissions(&item, &options);
        assert_eq!(next & (rules::COPY | rules::MODIFY | rules::TRANSFER) & !mask, 0);
        assert_ne!(next & (rules::COPY | rules::TRANSFER), 0);
        if mask != rules::COPY | rules::MODIFY | rules::TRANSFER {
            assert_eq!((group, everyone), (item.group_mask, item.everyone_mask));
        }
    }
    item.base_mask = u32::MAX;
    item.owner_mask = u32::MAX;
    item.asset_type = 56;
    assert_ne!(permissions(&item, &options).2 & rules::COPY, 0);
}

fn frame(
    ctx: &egui::Context,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    agent: Uuid,
    events: Vec<egui::Event>,
) -> (Vec<(String, egui::Rect)>, Vec<InvAction>) {
    let mut actions = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0))),
            focused: true,
            events,
            ..Default::default()
        },
        |_| show(ctx, &crate::theme::Theme::default().palette(), inv, st, prefs, agent, &mut actions),
    );
    output.textures_delta.clear();
    fn text(shape: &egui::Shape, result: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => result.push((t.galley.text().to_owned(), t.galley.rect.translate(t.pos.to_vec2()))),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    text(shape, result);
                }
            }
            _ => {}
        }
    }
    let mut labels = Vec::new();
    for shape in output.shapes {
        text(&shape.shape, &mut labels);
    }
    (labels, actions)
}

#[test]
fn inventory_bulk_apply_keeps_selection_and_options_and_ok_outlives_the_window() {
    let (mut inv, prefs, agent) = fixture();
    let ctx = egui::Context::default();
    let mut st = InventoryUi::default();
    st.open_bulk_permissions(vec![Uuid::from_u128(8102)]);
    st.selection.insert(Uuid::from_u128(8103));
    for _ in 0..4 {
        frame(&ctx, &mut inv, &mut st, &prefs, agent, vec![]);
    }
    for (label, closes) in [(aurora_i18n::tr!("common-apply"), false), (aurora_i18n::tr!("common-ok"), true)] {
        let (labels, _) = frame(&ctx, &mut inv, &mut st, &prefs, agent, vec![]);
        let point = labels.iter().find(|(text, _)| text == label).unwrap().1.center();
        frame(&ctx, &mut inv, &mut st, &prefs, agent, vec![egui::Event::PointerMoved(point)]);
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut inv,
                &mut st,
                &prefs,
                agent,
                vec![egui::Event::PointerButton {
                    pos: point,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                }],
            );
        }
        assert_eq!(st.bulk_permissions.open, !closes);
        assert!(st.bulk_permissions.job.is_some());
        st.bulk_permissions.options.types.fill(false);
        let (_, actions) = frame(&ctx, &mut inv, &mut st, &prefs, agent, vec![]);
        let [InvAction::Edit(change)] = &actions[..] else {
            panic!("Expected one bulk mutation")
        };
        assert_eq!(targets(change), HashSet::from([Uuid::from_u128(8102)]));
        st.finish_bulk_permissions(None);
        st.bulk_permissions.options.types.fill(true);
        frame(&ctx, &mut inv, &mut st, &prefs, agent, vec![]);
    }
}

#[test]
fn inventory_bulk_large_plans_remain_incremental_and_recheck_protection_before_patching() {
    let (mut inv, mut prefs, agent) = fixture();
    let template = inv.items[&Uuid::from_u128(8100)].clone();
    inv.add_items(
        (0..1000)
            .map(|index| InvItem {
                id: Uuid::from_u128(100_000 + index),
                ..template.clone()
            })
            .collect(),
    );
    let mut plan = job(&[8000], BulkPerms::default());
    assert!(plan.advance(&mut inv, &prefs, agent).unwrap().is_none());
    assert!(plan.change.operations.is_empty(), "Discovery must precede every PATCH");
    let excluded = Uuid::from_u128(100_000);
    prefs.protected.insert(excluded);
    let change = finish(&mut plan, &mut inv, &prefs, agent);
    assert!(change.operations.len() >= 999);
    assert!(!targets(&change).contains(&excluded));
    let old = inv.items[&Uuid::from_u128(8100)].clone();
    let reply = rules::demo_mutate(&mut inv, agent, change).unwrap();
    inv.apply(reply.contents);
    let updated = &inv.items[&old.id];
    assert_eq!(
        (updated.name.as_str(), updated.asset_id, updated.owner_mask),
        (old.name.as_str(), old.asset_id, old.owner_mask)
    );
    let unchanged = finish(&mut job(&[8000], BulkPerms::default()), &mut inv, &prefs, agent);
    assert!(unchanged.operations.is_empty(), "Applying the same permissions twice is a no-op");
}

#[test]
fn inventory_bulk_completion_updates_closed_auxiliary_windows_and_reports_errors() {
    let mut st = InventoryUi::default();
    st.open_folder_window(Uuid::from_u128(8000));
    let window = &mut st.windows[0];
    window.open = false;
    window.state.bulk_permissions.submitted = true;
    assert!(window.state.bulk_permissions.active());
    st.finish_bulk_permissions(Some("Rejected permission update"));
    assert!(!st.windows[0].state.bulk_permissions.active());
    assert_eq!(st.windows[0].state.bulk_permissions.status, "Rejected permission update");
}

#[test]
fn inventory_bulk_window_keeps_its_footer_inside_at_larger_text_sizes() {
    crate::ui::widgets::steady_tests::assert_window_steady(
        "inventory_bulk_permissions_00000000-0000-0000-0000-000000000000",
        &[Vec2::new(360.0, 354.0)],
        aurora_i18n::tr!("common-cancel"),
        || {
            let (inv, prefs, agent) = fixture();
            let mut st = InventoryUi::default();
            st.open_bulk_permissions(vec![Uuid::from_u128(8000)]);
            (inv, prefs, agent, st)
        },
        |(inv, prefs, agent, st), ctx, p| show(ctx, p, inv, st, prefs, *agent, &mut Vec::new()),
    );
}

#[test]
fn inventory_bulk_object_permissions_carry_rez_overrides_and_preserve_existing_flags() {
    let (mut inv, prefs, agent) = fixture();
    let id = Uuid::from_u128(8100);
    inv.items.get_mut(&id).unwrap().flags = 0x20;
    let options = BulkPerms {
        share: true,
        everyone_copy: true,
        ..Default::default()
    };
    let change = finish(&mut job(&[8100], options), &mut inv, &prefs, agent);
    let [Operation::Patch { body, .. }] = &change.operations[..] else {
        panic!()
    };
    assert_eq!(body["flags"].as_u32(), 0x20 | 0x100 | 0x040000 | 0x080000);
    rules::demo_mutate(&mut inv, agent, change).unwrap();
    assert_eq!(inv.items[&id].flags, 0x20 | 0x100 | 0x040000 | 0x080000);
}
