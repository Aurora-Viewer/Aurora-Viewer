use super::*;
use aurora_net::inventory::operations::Operation;
fn id(id: u128) -> Uuid {
    Uuid::from_u128(id)
}
fn inventory() -> Inventory {
    let mut inv = Inventory {
        owner: id(2),
        root: id(1),
        ..Default::default()
    };
    crate::world::inventory::demo::seed(&mut inv, id(2));
    inv
}
fn deny_modify() -> Options {
    Options {
        modify: Choice::Deny,
        ..Default::default()
    }
}
fn review(inv: &mut Inventory, roots: &[Uuid], opts: Options) -> Review {
    let mut review = Review::new(roots);
    for _ in 0..30 {
        review.scan(inv, id(2), opts);
        if review.complete {
            return review;
        }
    }
    panic!("folder traversal stuck");
}
#[test]
fn explorer_bulk_next_owner_respects_mask_limits_and_fair_use() {
    let inv = inventory();
    let mut item = inv.items[&id(8100)].clone();
    item.next_owner_mask = rules::COPY | rules::MODIFY | rules::MOVE;
    let opts = Options {
        copy: Choice::Deny,
        ..Default::default()
    };
    assert_eq!(opts.mask(&inv, &item), None, "never implicitly grant transfer");
    let opts = Options {
        transfer: Choice::Allow,
        ..opts
    };
    assert_eq!(opts.mask(&inv, &item), Some(rules::MODIFY | rules::MOVE | rules::TRANSFER));
    item.asset_type = 56;
    assert_eq!(opts.mask(&inv, &item), None, "settings remain copyable");
    item.asset_type = 6;
    item.base_mask &= !rules::TRANSFER;
    assert_eq!(opts.mask(&inv, &item), None);
    item.base_mask |= rules::TRANSFER;
    item.owner_mask &= !rules::TRANSFER;
    assert_eq!(opts.mask(&inv, &item), None);
}
#[test]
fn explorer_bulk_next_owner_recurses_deduplicates_and_only_patches_eligible_owned_items() {
    let mut inv = inventory();
    let original = inv.items[&id(8100)].clone();
    let mut inner = original.clone();
    inner.id = id(30001);
    inner.parent = id(8001);
    inner.owner_mask &= !rules::MODIFY;
    inv.add_items(vec![inner]);
    let mut review = review(&mut inv, &[id(8100), id(8000), id(8001)], deny_modify());
    let (mut change, mut expected) = review.batch(&inv, id(2), deny_modify());
    while !review.ready.is_empty() {
        let (sibling_change, more) = review.batch(&inv, id(2), deny_modify());
        assert!(sibling_change.refresh.len() <= 1, "read back only one parent per batch");
        change.operations.extend(sibling_change.operations);
        change.refresh.extend(sibling_change.refresh);
        expected.extend(more);
    }
    assert!(expected.contains_key(&id(30001)) && expected.contains_key(&id(8100)));
    assert_eq!(change.operations.len(), expected.len());
    assert!(!expected.contains_key(&id(8110)), "owner has no transfer");
    assert!(review.counts.skipped > 0);
    let Operation::Patch { body, .. } = change
        .operations
        .iter()
        .find(|p| matches!(p, Operation::Patch {id: target, ..} if *target == id(8100)))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(body["flags"].as_u32(), original.flags | SLAM_NEXT_OWNER);
    let mask = &body["permissions"];
    assert_eq!(mask.as_map().unwrap().len(), 1);
    assert_eq!(mask["next_owner_mask"].as_u32(), original.next_owner_mask & !rules::MODIFY);
    let reply = rules::demo_mutate(&mut inv, id(2), change).unwrap();
    let previous = expected.len();
    let mut st = State {
        review: Some(review),
        sent: expected,
        ..Default::default()
    };
    st.bind(id(777));
    st.finish(id(8), Some(&reply));
    assert_eq!(st.sent.len(), previous, "unrelated request cannot complete a batch");
    st.finish(id(777), Some(&reply));
    assert_eq!(st.review.unwrap().counts.verified, previous);
    let saved = &inv.items[&id(8100)];
    assert_eq!(
        (
            saved.owner_mask,
            saved.base_mask,
            saved.everyone_mask,
            saved.group_mask,
            saved.asset_id
        ),
        (
            original.owner_mask,
            original.base_mask,
            original.everyone_mask,
            original.group_mask,
            original.asset_id
        )
    );
}
#[test]
fn explorer_bulk_next_owner_reports_unread_branches_and_revalidates_moved_items() {
    let mut inv = inventory();
    let inner = id(8001);
    inv.folders.get_mut(&inner).unwrap().state = FetchState::Unknown;
    let mut plan = Review::new(&[id(8000)]);
    plan.scan(&mut inv, id(2), deny_modify());
    plan.scan(&mut inv, id(2), deny_modify());
    assert!(inv.queue.iter().any(|(folder, _)| *folder == inner));
    assert!(!plan.complete);
    inv.folders.get_mut(&inner).unwrap().state = FetchState::Failed;
    plan.scan(&mut inv, id(2), deny_modify());
    assert!(plan.complete && plan.counts.unread == 1);
    inv.items.get_mut(&id(8100)).unwrap().parent = inv.root;
    inv.items.get_mut(&id(8102)).unwrap().owner_mask &= !rules::TRANSFER;
    let (mut change, mut expected) = plan.batch(&inv, id(2), deny_modify());
    while !plan.ready.is_empty() {
        let (additional, more) = plan.batch(&inv, id(2), deny_modify());
        change.operations.extend(additional.operations);
        expected.extend(more);
    }
    assert!(!expected.contains_key(&id(8100)) && !expected.contains_key(&id(8102)));
    assert!(!change.operations.is_empty());
    let mut state = State {
        review: Some(plan),
        sent: expected.clone(),
        ..Default::default()
    };
    state.bind(id(333));
    state.finish(id(333), None);
    assert_eq!(state.review.unwrap().counts.unconfirmed, expected.len());
}

#[test]
fn explorer_bulk_next_owner_blocks_links_managed_locations_and_foreign_owners() {
    let mut inv = inventory();
    for (n, kind) in [(99901, 24), (99902, 25), (99903, 2), (99904, 3)] {
        let mut item = inv.items[&id(8100)].clone();
        item.id = id(n);
        item.asset_type = kind;
        inv.add_items(vec![item]);
        assert!(!selectable(&inv, id(n), id(2)));
    }
    for target in [inv.root, inv.lib_root, id(701), id(706)] {
        assert!(!selectable(&inv, target, id(2)));
    }
    assert!(!selectable(&inv, id(8100), id(4)));
    inv.items.get_mut(&id(8100)).unwrap().group_owned = true;
    assert!(!selectable(&inv, id(8100), id(2)));
}

#[test]
fn explorer_bulk_next_owner_rebinds_review_but_keeps_an_active_jobs_selection() {
    let mut st = State::default();
    st.open(id(1), vec![id(3)]);
    st.review = Some(Review::new(&st.roots));
    st.open(id(1), vec![id(4)]);
    assert_eq!(st.roots, [id(4)]);
    assert!(st.review.is_none(), "must review the newly selected folder");
    st.sent.insert(id(4), (rules::COPY, true));
    st.bind(id(99));
    st.open(id(1), vec![id(5)]);
    assert_eq!(st.roots, [id(4)], "must keep the exact submitted scope until its readback");
    st.finish(id(99), None);
    st.open(id(1), vec![id(5)]);
    assert_eq!(st.roots, [id(5)]);
}
