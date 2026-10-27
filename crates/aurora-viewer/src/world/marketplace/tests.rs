use super::validate::{self, Level};
use super::*;
use crate::world::inventory::Folder;
use crate::world::inventory::actions::{COPY, TRANSFER};
use aurora_net::inventory::operations::Operation;
use aurora_net::inventory::{InvFolder, InvItem};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const ROOT: u128 = 1;
const LISTINGS_ROOT: u128 = 2;

#[test]
fn clipboard_copies_normalize_only_the_copies_and_moves_keep_stock_rules() {
    let inv = inventory();
    let worn = HashSet::new();
    let root = id(LISTINGS_ROOT);
    let change = validate::paste_plan(&inv, root, id(10), &[id(201)], true, &worn).expect("copy into listing");
    assert_eq!(change.copies.len(), 1);
    assert_eq!(change.copies[0].item, id(201));
    assert!(
        change.operations.iter().any(
            |op| matches!(op, Operation::Folder { id: folder, parent, .. } if *folder == change.copies[0].parent && *parent == id(10))
        )
    );
    assert!(
        !change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Patch { id: source, .. } if *source == id(201)))
    );
    assert_eq!(inv.items[&id(201)].parent, id(20));

    // A folder with loose copyable items becomes a listing/version pair,
    // without the fixing audit moving the source's children.
    let mut inv = inventory();
    inv.remove(&[id(202)]);
    let change = validate::paste_plan(&inv, root, root, &[id(20)], true, &worn).expect("copy normalized folder");
    assert_eq!(change.copies.len(), 1);
    let version = change.copies[0].parent;
    let listing = change
        .operations
        .iter()
        .find_map(|op| match op {
            Operation::Folder { id, parent, .. } if *id == version => Some(*parent),
            _ => None,
        })
        .expect("version");
    assert!(
        change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Folder { id, parent, .. } if *id == listing && *parent == root)),
        "{change:?}"
    );
    assert!(
        !change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Patch { id: item, .. } if [id(20), id(201)].contains(item)))
    );

    let inv = inventory();
    assert!(validate::paste_plan(&inv, root, id(12), &[id(202)], true, &worn).is_err());
    let copied = validate::paste_plan(&inv, root, root, &[id(20)], true, &worn).expect("copyable contents only");
    assert_eq!(copied.copies.len(), 1);
    assert_eq!(copied.copies[0].item, id(201));
    assert!(validate::paste_plan(&inv, root, id(13), &[id(201)], false, &worn).is_err());
    let change = validate::paste_plan(&inv, root, id(12), &[id(202)], false, &worn).expect("move stock item");
    assert!(
        change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Folder { type_default, .. } if *type_default == STOCK))
    );
    assert!(crate::world::inventory::actions::paste(&inv, &[id(201)], id(12), false, true, &HashSet::new(), &worn).is_err());

    // Copying an unfinished listing must normalize the clone alone, even
    // when its source is already below the Marketplace root.
    let mut inv = inventory();
    let mut folder = inv.folders[&id(20)].info.clone();
    folder.parent = root;
    inv.server_updated(vec![folder], Vec::new());
    let copied = validate::paste_plan(&inv, root, root, &[id(20)], true, &worn).expect("copy an unfinished listing");
    assert_eq!(copied.copies.len(), 1);
    assert!(
        !copied
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Patch { id: source, .. } if [id(20), id(201), id(202)].contains(source)))
    );
}

#[test]
fn pending_listing_updates_cover_items_and_stock_descendants() {
    let inv = inventory();
    let mut data = merchant_with(&inv, vec![listing(1, 10, 11, true, 3)]);
    data.activate_listing(&inv, id(LISTINGS_ROOT), id(10), false);
    for n in [10, 11, 12, 13, 101, 104] {
        assert!(data.is_updating(&inv, id(LISTINGS_ROOT), id(n)), "{n}");
    }
    assert!(!data.is_updating(&inv, id(LISTINGS_ROOT), id(201)));
}

/// My Inventory (1) › Marketplace listings (2) › listing (10) › versions
/// (11, 12); the stock folder 13 in version 11 holds three no-copy items.
fn inventory() -> Inventory {
    let mut inv = Inventory {
        root: id(ROOT),
        ..Default::default()
    };
    inv.folders.insert(
        id(ROOT),
        Folder {
            info: InvFolder {
                id: id(ROOT),
                name: "My Inventory".into(),
                type_default: 8,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: FetchState::Fetched,
            library: false,
        },
    );
    let folder = |n: u128, parent: u128, name: &str, kind: i32| InvFolder {
        id: id(n),
        parent: id(parent),
        name: name.into(),
        type_default: kind,
        version: 1,
        ..Default::default()
    };
    inv.created_folders(vec![
        folder(LISTINGS_ROOT, ROOT, "Marketplace listings", LISTINGS),
        folder(14, ROOT, "Trash", 14),
        folder(10, LISTINGS_ROOT, "Hoodie", -1),
        folder(11, 10, "Hoodie v1", -1),
        folder(12, 10, "Hoodie v2", -1),
        folder(13, 11, "Hoodie stock", STOCK),
        folder(20, ROOT, "Products", -1),
    ]);
    inv.add_items(vec![
        item(101, 13, "Hoodie #1", false),
        item(102, 13, "Hoodie #2", false),
        item(103, 13, "Hoodie #3", false),
        item(104, 12, "Hoodie v2", true),
        item(201, 20, "Chair", true),
        item(202, 20, "Vase", false),
    ]);
    inv
}

fn item(n: u128, parent: u128, name: &str, copy: bool) -> InvItem {
    InvItem {
        id: id(n),
        parent: id(parent),
        name: name.into(),
        asset_type: 6,
        inv_type: 6,
        owner_mask: if copy { COPY | TRANSFER } else { TRANSFER },
        next_owner_mask: TRANSFER,
        ..Default::default()
    }
}

fn listing(n: i32, folder: u128, version: u128, is_listed: bool, count: i32) -> Listing {
    Listing {
        id: n,
        is_listed,
        edit_url: format!("https://m/{n}"),
        folder: id(folder),
        version: if version == 0 { Uuid::nil() } else { id(version) },
        count,
    }
}

fn merchant_with(inv: &Inventory, listings: Vec<Listing>) -> MarketplaceData {
    let mut data = MarketplaceData::default();
    assert!(data.initialize());
    assert!(!data.initialize(), "GET /merchant is sent once");
    data.set_status(MerchantStatus::Merchant);
    assert!(data.fetch_contents(id(LISTINGS_ROOT)));
    assert_eq!(data.take_requests(Instant::now()), vec![SlmRequest::Listings]);
    assert!(
        data.is_updating(inv, id(LISTINGS_ROOT), id(10)),
        "everything waits for GET /listings"
    );
    data.apply(
        inv,
        id(LISTINGS_ROOT),
        &SlmRequest::Listings,
        SlmReply::Listings(listings),
        Instant::now(),
    );
    assert_eq!(data.fetch(), Fetch::Done);
    assert!(!data.is_updating(inv, id(LISTINGS_ROOT), id(10)));
    data
}

#[test]
fn depth_and_listing_folder() {
    let inv = inventory();
    let root = id(LISTINGS_ROOT);
    assert_eq!(super::root(&inv), Some(root));
    assert_eq!(depth(&inv, root, root), 0);
    assert_eq!(depth(&inv, root, id(10)), 1);
    assert_eq!(depth(&inv, root, id(11)), 2);
    assert_eq!(depth(&inv, root, id(101)), 4);
    assert_eq!(depth(&inv, root, id(20)), -1);
    assert_eq!(nested_parent(&inv, id(101), 4), id(10));
    assert_eq!(nested_parent(&inv, id(10), 1), id(10));
    assert_eq!(nested_parent(&inv, id(20), -1), Uuid::nil());
}

#[test]
fn stock_counts_follow_compute_stock_count() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let data = merchant_with(&inv, vec![listing(7, 10, 11, true, 3)]);
    // the listing's stock is its version folder's
    assert_eq!(data.stock_count(&inv, root, id(10), false), 3);
    assert_eq!(data.stock_count(&inv, root, id(13), false), 3);
    // another version than the active one has no meaningful stock
    assert_eq!(data.stock_count(&inv, root, id(12), false), STOCK_INFINITE);
    // a listing without SLM record neither, unless forced
    let empty = MarketplaceData::default();
    assert_eq!(empty.stock_count(&inv, root, id(10), false), STOCK_INFINITE);
    assert_eq!(empty.stock_count(&inv, root, id(11), true), 3);
    // a stock folder not loaded cannot be counted
    inv.folders.get_mut(&id(13)).unwrap().state = FetchState::Unknown;
    assert_eq!(data.stock_count(&inv, root, id(10), false), STOCK_NOT_EVALUATED);
    assert!(data.activation_state(id(11)), "the version folder of a listed listing is active");
    assert!(data.is_version_folder(id(11)));
    assert_eq!(data.listing_url(&inv, root, id(101)), "https://m/7");
}

#[test]
fn create_listing_takes_the_only_version_folder() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, Vec::new());
    // two versions: none chosen, and the merchant is told
    assert!(data.create_listing(&inv, root, id(10)));
    assert_eq!(data.take_alerts(), vec![Alert::ActivateRequired]);
    assert_eq!(
        data.take_requests(Instant::now()),
        vec![SlmRequest::Create {
            folder: id(10),
            name: "Hoodie".into(),
            version: Uuid::nil(),
            count: STOCK_INFINITE
        }]
    );
    assert!(data.is_updating(&inv, root, id(12)));
    // one version: chosen, with its stock (a stock folder not loaded is 0)
    inv.remove(&[id(12)]);
    inv.folders.get_mut(&id(13)).unwrap().state = FetchState::Fetching;
    let mut data = merchant_with(&inv, Vec::new());
    assert!(data.create_listing(&inv, root, id(10)));
    assert!(data.take_alerts().is_empty());
    let request = data.take_requests(Instant::now()).remove(0);
    assert_eq!(
        request,
        SlmRequest::Create {
            folder: id(10),
            name: "Hoodie".into(),
            version: id(11),
            count: 0
        }
    );
    data.apply(
        &inv,
        root,
        &request,
        SlmReply::Listings(vec![listing(9, 10, 11, false, 0)]),
        Instant::now(),
    );
    assert_eq!(data.listing_id(id(10)), 9);
    assert!(!data.is_updating(&inv, root, id(10)));
    assert!(!data.create_listing(&inv, root, id(10)), "a folder has one listing");
}

#[test]
fn list_unlist_and_answers_that_differ() {
    let inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, vec![listing(7, 10, 11, false, 3)]);
    assert!(data.activate_listing(&inv, root, id(10), false), "unchanged: nothing sent");
    assert!(data.take_requests(Instant::now()).is_empty());
    assert!(data.activate_listing(&inv, root, id(10), true));
    let request = data.take_requests(Instant::now()).remove(0);
    assert_eq!(
        request,
        SlmRequest::Update {
            folder: id(10),
            id: 7,
            version: id(11),
            is_listed: true,
            count: 3
        }
    );
    // SLM kept it unlisted: AlertMerchantListingNotUpdated with its page
    data.apply(
        &inv,
        root,
        &request,
        SlmReply::Listings(vec![listing(7, 10, 11, false, 3)]),
        Instant::now(),
    );
    assert_eq!(data.take_alerts(), vec![Alert::NotUpdated { url: "https://m/7".into() }]);
    // an empty answer: the listing is read back five seconds later
    let now = Instant::now();
    data.apply(&inv, root, &request, SlmReply::Listings(Vec::new()), now);
    assert!(data.take_requests(now).is_empty());
    assert_eq!(
        data.take_requests(now + Duration::from_secs(5)),
        vec![SlmRequest::Listing { id: 7 }]
    );
    assert!(data.is_updating(&inv, root, id(10)));
    data.apply(
        &inv,
        root,
        &SlmRequest::Listing { id: 7 },
        SlmReply::Listings(vec![listing(7, 10, 11, true, 3)]),
        now,
    );
    assert!(data.activation_state(id(10)));
    // a failure becomes the transaction alert
    let error = SlmError::Transaction {
        reason: "'Bad'\n".into(),
        description: String::new(),
    };
    data.apply(&inv, root, &request, SlmReply::Failed(error.clone()), now);
    assert_eq!(data.take_alerts(), vec![Alert::Failed(error)]);
}

#[test]
fn version_changes_and_stock_updates() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, vec![listing(7, 10, 11, true, 3)]);
    // clearing the version unlists
    assert!(data.set_version_folder(&inv, root, id(11), Uuid::nil()));
    let request = data.take_requests(Instant::now()).remove(0);
    assert!(matches!(request, SlmRequest::Update { is_listed: false, version, count: STOCK_INFINITE, .. } if version.is_nil()));
    // another version keeps the listed state and takes its stock
    assert!(data.set_version_folder(&inv, root, id(12), id(12)));
    assert!(matches!(
        data.take_requests(Instant::now()).remove(0),
        SlmRequest::Update { is_listed: true, version, .. } if version == id(12)
    ));
    // one item sold: the local count changes at once, one update only
    inv.remove(&[id(103)]);
    assert!(data.update_count_on_hand(&inv, root, id(10)));
    assert_eq!(data.count_on_hand(id(10)), 2);
    assert_eq!(data.take_requests(Instant::now()).len(), 1);
    assert!(data.update_count_on_hand(&inv, root, id(10)));
    assert!(data.take_requests(Instant::now()).is_empty());
}

#[test]
fn associate_delete_and_missing_listings() {
    let inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, vec![listing(7, 12, 0, false, -1)]);
    assert!(data.associate_listing(&inv, id(10), id(20), 7));
    assert_eq!(data.take_alerts(), vec![Alert::ActivateRequired]);
    let request = data.take_requests(Instant::now()).remove(0);
    assert!(data.is_updating(&inv, root, id(10)));
    data.apply(
        &inv,
        root,
        &request,
        SlmReply::Listings(vec![listing(7, 10, 0, false, -1)]),
        Instant::now(),
    );
    assert!(!data.is_listed(id(12)), "an id belongs to one folder");
    assert_eq!(data.listing_id(id(10)), 7);
    // a listing SLM no longer has is forgotten
    data.apply(&inv, root, &SlmRequest::Listing { id: 7 }, SlmReply::NotFound, Instant::now());
    assert!(!data.is_listed(id(10)));
    // DELETE removes the record of the listing it answers with
    let mut data = merchant_with(&inv, vec![listing(8, 10, 11, false, 3)]);
    assert!(data.clear_listing(&inv, root, id(101)));
    let request = data.take_requests(Instant::now()).remove(0);
    assert_eq!(request, SlmRequest::Delete { id: 8 });
    assert!(data.is_updating(&inv, root, id(10)));
    data.apply(
        &inv,
        root,
        &request,
        SlmReply::Listings(vec![listing(8, 10, 11, false, 3)]),
        Instant::now(),
    );
    assert!(!data.is_listed(id(10)));
    assert!(!data.is_updating(&inv, root, id(10)));
    assert!(!data.clear_listing(&inv, root, id(10)), "no listing left to clear");
}

#[test]
fn reconcile_keeps_slm_in_step_with_the_inventory() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, vec![listing(7, 10, 12, true, -1)]);
    data.reconcile(&inv, root);
    assert!(data.take_requests(Instant::now()).is_empty(), "nothing to change");
    // the active version emptied: unlisted, and the merchant is told
    inv.remove(&[id(104)]);
    data.reconcile(&inv, root);
    assert_eq!(data.take_alerts(), vec![Alert::VersionEmpty]);
    assert!(matches!(
        data.take_requests(Instant::now())[..],
        [SlmRequest::Update { is_listed: false, .. }]
    ));
    data.reconcile(&inv, root);
    assert!(data.take_requests(Instant::now()).is_empty(), "not sent again while it runs");
    // the listing folder taken out of the listings: disassociated
    let mut data = merchant_with(&inv, vec![listing(7, 10, 11, false, 3)]);
    inv.server_updated(
        vec![InvFolder {
            id: id(10),
            parent: id(14),
            name: "Hoodie".into(),
            type_default: -1,
            version: 2,
            ..Default::default()
        }],
        Vec::new(),
    );
    data.reconcile(&inv, root);
    assert_eq!(data.take_requests(Instant::now()), vec![SlmRequest::Delete { id: 7 }]);
}

#[test]
fn reconcile_waits_for_a_complete_parent_chain() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let mut data = merchant_with(&inv, vec![listing(7, 10, 11, false, 3)]);
    // a parent the inventory does not know is no proof the listing moved
    inv.folders.get_mut(&id(10)).unwrap().info.parent = id(999);
    data.reconcile(&inv, root);
    assert!(data.take_requests(Instant::now()).is_empty());
}

#[test]
fn audit_reports_without_changing_anything() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let worn = HashSet::new();
    let report = validate::validate(&inv, root, id(10), false, -1, &worn);
    assert!(report.ok, "{:?}", report.messages);
    assert!(report.shown().is_empty(), "a clean listing is not shown");
    // loose items in a listing folder, a no-copy one outside a stock folder
    inv.add_items(vec![item(105, 10, "Loose", true), item(106, 12, "No copy", false)]);
    let report = validate::validate(&inv, root, id(10), false, -1, &worn);
    assert!(!report.ok);
    assert!(report.change.operations.is_empty());
    let errors: Vec<_> = report.messages.iter().filter(|m| m.level == Level::Error).collect();
    assert_eq!(errors.len(), 2, "{:?}", report.messages);
    assert!(errors[0].text.contains("Loose") && errors[1].text.contains("No copy"));
    assert!(!report.shown().is_empty());
    // a non-transferable item can never be sold
    let mut bad = item(107, 12, "Gift", true);
    bad.owner_mask = COPY;
    inv.add_items(vec![bad]);
    let report = validate::validate(&inv, root, id(12), false, -1, &worn);
    assert!(report.errors().contains("Gift"));
}

#[test]
fn audit_fixes_the_hierarchy_like_firestorm() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let worn = HashSet::new();
    // items right under the listing folder are wrapped in a version
    // folder, the no-copy one in a stock folder
    inv.add_items(vec![item(105, 10, "Loose", true), item(106, 10, "Loose no copy", false)]);
    let report = validate::validate(&inv, root, id(10), true, -1, &worn);
    let created: Vec<i32> = report
        .change
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Folder { parent, type_default, .. } if *parent == id(10) => Some(*type_default),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![STOCK, -1], "{:?}", report.change.operations);
    let moved = report
        .change
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Patch { folder: false, .. }))
        .count();
    assert_eq!(moved, 2);
    // a stock folder right under a listing is nested in a version folder
    let mut inv = inventory();
    inv.server_updated(
        vec![InvFolder {
            id: id(13),
            parent: id(10),
            name: "Hoodie stock".into(),
            type_default: STOCK,
            version: 2,
            ..Default::default()
        }],
        Vec::new(),
    );
    let report = validate::validate(&inv, root, id(10), true, -1, &worn);
    assert!(!report.ok, "Firestorm fails the pass that nests a stock folder");
    let wrapper = report.change.operations.iter().find_map(|op| match op {
        Operation::Folder {
            id: new,
            parent,
            type_default: -1,
            ..
        } if *parent == id(10) => Some(*new),
        _ => None,
    });
    let wrapper = wrapper.expect("a version folder for the stock folder");
    assert!(report.change.operations.iter().any(|op| matches!(
        op,
        Operation::Patch { id: moved, folder: true, body } if *moved == id(13) && body["parent_id"].as_uuid() == wrapper
    )));
    // the fixed hierarchy passes the next audit
    crate::world::inventory::actions::demo_mutate(&mut inv, id(ROOT), report.change).expect("fix applies");
    let report = validate::validate(&inv, root, id(10), false, -1, &worn);
    assert!(report.ok, "{:?}", report.messages);
}

#[test]
fn audit_limits_and_mixed_stock() {
    let mut inv = inventory();
    let root = id(LISTINGS_ROOT);
    let worn = HashSet::new();
    // a no-copy item of another type in the stock folder
    let mut other = item(108, 13, "Hoodie notecard", false);
    other.inv_type = 7;
    other.asset_type = 7;
    inv.add_items(vec![other]);
    let report = validate::validate(&inv, root, id(10), false, -1, &worn);
    assert!(!report.ok);
    assert!(
        report
            .messages
            .iter()
            .any(|m| m.level == Level::Error && m.text.contains("Hoodie stock"))
    );
    // more than 20 folders in a version folder
    let mut inv = inventory();
    let folders = (0..21)
        .map(|n| InvFolder {
            id: id(300 + n),
            parent: id(12),
            name: format!("Box {n}"),
            type_default: -1,
            version: 1,
            ..Default::default()
        })
        .collect();
    inv.created_folders(folders);
    let report = validate::validate(&inv, root, id(12), false, -1, &worn);
    assert!(!report.ok);
    assert!(report.errors().contains("20"), "{}", report.errors());
}

#[test]
fn dropping_inventory_builds_the_listing_folders() {
    let inv = inventory();
    let root = id(LISTINGS_ROOT);
    let worn = HashSet::new();
    // a no-copy item on the root: listing, version and stock folders
    let change = validate::drop_plan(&inv, root, root, &[id(202)], &worn).expect("drop accepted");
    let kinds: Vec<i32> = change
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Folder { type_default, .. } => Some(*type_default),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![-1, -1, STOCK]);
    // a copyable item on a listing: one version folder
    let change = validate::drop_plan(&inv, root, id(10), &[id(201)], &worn).expect("drop accepted");
    assert_eq!(
        change.operations.iter().filter(|op| matches!(op, Operation::Folder { .. })).count(),
        1
    );
    // a worn item is refused, and so is a copyable item in a stock folder
    let worn: HashSet<Uuid> = [id(201)].into();
    assert!(validate::drop_plan(&inv, root, root, &[id(201)], &worn).is_err());
    assert!(validate::drop_plan(&inv, root, id(13), &[id(201)], &HashSet::new()).is_err());
    // a folder is moved, then the destination is fixed: Products holds a
    // no-copy vase, which gets its stock folder
    let change = validate::drop_plan(&inv, root, root, &[id(20)], &HashSet::new()).expect("drop accepted");
    assert!(
        change
            .operations
            .iter()
            .any(|op| matches!(op, Operation::Folder { type_default: STOCK, .. }))
    );
}

#[test]
fn the_demo_server_answers_like_slm() {
    let inv = Inventory {
        root: id(ROOT),
        ..Default::default()
    };
    let mut slm = demo::DemoSlm::new(demo::Mode::Merchant);
    assert_eq!(slm.merchant(), MerchantStatus::Merchant);
    let SlmReply::Listings(all) = slm.answer(&SlmRequest::Listings) else {
        panic!("listings")
    };
    assert_eq!(all.len(), 3);
    // the draft cannot be listed (an incomplete listing)
    let draft = all.iter().find(|l| l.folder == demo::DRAFT).unwrap();
    let list = SlmRequest::Update {
        folder: draft.folder,
        id: draft.id,
        version: draft.version,
        is_listed: true,
        count: -1,
    };
    assert_eq!(slm.answer(&list), SlmReply::Failed(SlmError::Unprocessable));
    // out of stock stays unlisted, which the store reports
    let poster = all.iter().find(|l| l.folder == demo::POSTER).unwrap().clone();
    let list = SlmRequest::Update {
        folder: poster.folder,
        id: poster.id,
        version: poster.version,
        is_listed: true,
        count: 0,
    };
    let reply = slm.answer(&list);
    let mut data = MarketplaceData::default();
    data.set_status(MerchantStatus::Merchant);
    data.apply(&inv, Uuid::nil(), &SlmRequest::Listings, SlmReply::Listings(all), Instant::now());
    data.apply(&inv, Uuid::nil(), &list, reply, Instant::now());
    assert!(matches!(data.take_alerts()[..], [Alert::NotUpdated { .. }]));
    assert_eq!(demo::DemoSlm::new(demo::Mode::NotMerchant).merchant(), MerchantStatus::NotMerchant);
    assert!(matches!(
        demo::DemoSlm::new(demo::Mode::Error).merchant(),
        MerchantStatus::ConnectionFailure(_)
    ));
}
