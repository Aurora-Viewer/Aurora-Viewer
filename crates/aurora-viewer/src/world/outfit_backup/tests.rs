use super::*;

fn fixture() -> (Inventory, Uuid, Backup) {
    let agent = Uuid::from_u128(1);
    let mut inv = Inventory {
        owner: agent,
        ..Default::default()
    };
    appearance::seed_demo(&mut inv, agent);
    let backup = Backup::capture(&inv, appearance::cof(&inv).expect("COF"), &HashMap::new()).expect("capture");
    (inv, agent, backup)
}

#[test]
fn outfit_json_round_trip_preserves_links_layers_gestures_and_points() {
    let (mut inv, agent, _) = fixture();
    appearance::seed_demo_layers(&mut inv, agent);
    let cof = appearance::cof(&inv).expect("COF");
    let gesture = aurora_net::inventory::InvItem {
        id: Uuid::from_u128(999),
        parent: inv.root,
        name: "Demo gesture".into(),
        asset_type: 21,
        inv_type: 21,
        owner: agent,
        ..Default::default()
    };
    inv.add_items(vec![gesture.clone()]);
    let mut links = appearance::folder_links(&inv, cof);
    links.push(appearance::item_link(&gesture));
    appearance::demo_mutate(
        &mut inv,
        agent,
        &aurora_net::outfits::OutfitMutation {
            folder: cof,
            create: None,
            links,
            cof: None,
        },
    )
    .expect("gesture");
    let object = Uuid::from_u128(717);
    let backup = Backup::capture(&inv, cof, &HashMap::from([(object, 31)])).expect("capture");
    let bytes = backup.bytes().expect("JSON");
    assert!(bytes.starts_with(b"{\n"));
    let restored = Backup::parse(&bytes).expect("parse");
    assert_eq!(restored.resolve(&inv, agent).links, backup.resolve(&inv, agent).links);
    assert_eq!(restored.points().iter().find(|p| p.item == object).expect("object").point, 31);
    assert!(restored.resolve(&inv, agent).links.iter().any(|link| link.desc.starts_with('@')));
    assert!(restored.resolve(&inv, agent).links.iter().any(|link| link.target == gesture.id));
}

#[test]
fn outfit_json_missing_items_are_explicit_and_partial_import_is_opt_in() {
    let (mut inv, agent, backup) = fixture();
    let removed = backup.items.iter().find(|it| it.asset_type == 6).expect("attachment").item_id;
    inv.items.remove(&removed);
    inv.missing_items.insert(removed);
    let resolved = backup.resolve(&inv, agent);
    assert_eq!(resolved.missing.len(), 1);
    assert_eq!(resolved.missing[0].item_id, removed);
    let action = |partial| appearance::Action::ImportJson {
        backup: Box::new(backup.clone()),
        name: "Imported".into(),
        partial,
    };
    assert!(appearance::plan(&inv, action(false), &HashMap::new()).is_err());
    let (change, sync) = appearance::plan(&inv, action(true), &HashMap::new()).expect("partial");
    assert!(sync);
    assert!(change.create.is_some());
    assert!(change.links.iter().all(|l| l.target != removed));
    appearance::demo_mutate(&mut inv, agent, &change).expect("apply");
    assert_eq!(appearance::base(&inv), Some(change.folder));
    assert!(!appearance::dirty(&inv));
}

#[test]
fn outfit_json_import_preserves_current_required_body_parts_when_missing() {
    let (inv, _, mut backup) = fixture();
    backup.items.retain(|it| it.asset_type != 13);
    let (change, _) = appearance::plan(
        &inv,
        appearance::Action::ImportJson {
            backup: Box::new(backup),
            name: "Partial".into(),
            partial: true,
        },
        &HashMap::new(),
    )
    .expect("import");
    assert_eq!(change.links.iter().filter(|l| inv.items[&l.target].asset_type == 13).count(), 4);
}

#[test]
fn outfit_json_rejects_untrusted_format_versions_duplicates_and_oversized_files() {
    let (_, _, mut backup) = fixture();
    let bytes = backup.bytes().expect("bytes");
    assert!(Backup::parse(b"{}").is_err());
    assert!(Backup::parse(&vec![b' '; MAX_BYTES as usize + 1]).is_err());
    let mut text: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    text["version"] = 2.into();
    assert!(Backup::parse(&serde_json::to_vec(&text).expect("JSON")).is_err());
    backup.items.push(backup.items[0].clone());
    assert!(backup.bytes().is_err());
}

#[test]
fn outfit_json_portable_filenames_keep_unicode_and_reject_devices_and_paths() {
    for (input, expected) in [
        ("Evening dress", "Evening dress.json"),
        ("CON", "_CON.json"),
        ("LPT1.txt", "_LPT1.txt.json"),
        ("COM¹", "_COM¹.json"),
        ("...", "outfit.json"),
        (" ../a:b?* ", ".._a_b__.json"),
        ("夜の服", "夜の服.json"),
    ] {
        assert_eq!(file_name(input), expected);
    }
    assert!(file_name(&"服".repeat(300)).len() <= 185);
}

#[test]
fn outfit_json_partial_import_never_rezzes_trashed_or_foreign_items() {
    let (mut inv, agent, backup) = fixture();
    let objects: Vec<_> = backup.items.iter().filter(|it| it.asset_type == 6).map(|it| it.item_id).collect();
    let trash = appearance::system_folder(&inv, 14).expect("Trash");
    inv.items.get_mut(&objects[0]).expect("object").parent = trash;
    inv.items.get_mut(&objects[1]).expect("object").owner = Uuid::from_u128(222);
    let action = appearance::Action::ImportJson {
        backup: Box::new(backup),
        name: "Available only".into(),
        partial: true,
    };
    let rez = appearance::attachments_for_action(&inv, agent, &action);
    assert!(!rez.iter().any(|request| objects[..2].contains(&request.item_id)));
    let (change, _) = appearance::plan(&inv, action, &HashMap::new()).expect("partial");
    assert!(!change.links.iter().any(|link| objects[..2].contains(&link.target)));
}

#[test]
fn outfit_json_import_applies_saved_layer_order_even_after_inventory_order_changes() {
    let (mut inv, agent, _) = fixture();
    appearance::seed_demo_layers(&mut inv, agent);
    let cof = appearance::cof(&inv).expect("COF");
    let mut original = appearance::folder_links(&inv, cof);
    for (id, order) in [(714, 1402), (724, 1400), (725, 1401)] {
        original
            .iter_mut()
            .find(|link| link.target == Uuid::from_u128(id))
            .expect("tattoo")
            .desc = format!("@{order}");
    }
    let mut mutation = appearance::plan(&inv, appearance::Action::Save(None), &HashMap::new())
        .expect("save")
        .0;
    mutation.folder = cof;
    mutation.links = original;
    mutation.cof = None;
    appearance::demo_mutate(&mut inv, agent, &mutation).expect("layers");
    let backup = Backup::parse(&Backup::capture(&inv, cof, &HashMap::new()).expect("backup").bytes().expect("bytes")).expect("parse");
    let (change, _) = appearance::plan(
        &inv,
        appearance::Action::ImportJson {
            backup: Box::new(backup),
            name: "Restored layers".into(),
            partial: false,
        },
        &HashMap::new(),
    )
    .expect("import");
    for (id, order) in [(714, 1402), (724, 1400), (725, 1401)] {
        assert_eq!(
            change
                .links
                .iter()
                .find(|link| link.target == Uuid::from_u128(id))
                .expect("tattoo")
                .desc,
            format!("@{order}")
        );
    }
}

#[test]
fn outfit_json_with_no_items_never_wears_an_empty_outfit() {
    let (inv, agent, mut backup) = fixture();
    backup.items.clear();
    let backup = Backup::parse(&backup.bytes().expect("bytes")).expect("an empty list is still a readable file");
    for partial in [false, true] {
        let action = appearance::Action::ImportJson {
            backup: Box::new(backup.clone()),
            name: "Empty".into(),
            partial,
        };
        assert!(appearance::attachments_for_action(&inv, agent, &action).is_empty());
        assert_eq!(
            appearance::plan(&inv, action, &HashMap::new()).err().as_deref(),
            Some(aurora_i18n::tr!("outfit-json-empty"))
        );
    }
}

#[test]
fn outfit_json_link_descriptions_fit_the_server_limit_on_a_char_boundary() {
    let (inv, agent, mut backup) = fixture();
    let long = format!("ab{}", "é".repeat(100));
    for item in &mut backup.items {
        item.description = long.clone();
    }
    let links = backup.resolve(&inv, agent).links;
    assert!(!links.is_empty());
    for link in links {
        // Byte 127 falls inside a two-byte "é": the cut keeps the one before.
        assert_eq!(link.desc.len(), 126);
        assert!(long.starts_with(&link.desc));
    }
    backup.items[0].description = "@1402".into();
    assert!(backup.resolve(&inv, agent).links.iter().any(|link| link.desc == "@1402"));
}

/// #4865: the worn bridge never goes into a backup, and one an older backup
/// holds is not imported into the new outfit.
#[test]
fn outfit_json_leaves_the_bridge_out() {
    let (mut inv, agent, _) = fixture();
    let cof = appearance::cof(&inv).expect("COF");
    let bridge = Uuid::from_u128(0xB41D);
    inv.add_items(vec![aurora_net::inventory::InvItem {
        id: bridge,
        parent: inv.root,
        name: "Old bridge".into(),
        asset_type: 6,
        inv_type: 6,
        owner: agent,
        ..Default::default()
    }]);
    let worn = HashMap::from([(bridge, 31u8)]);
    // a backup from before the fix: the worn bridge (then by any name) in it
    let old = Backup::capture(&inv, cof, &worn).expect("capture");
    assert!(old.items.iter().any(|item| item.item_id == bridge));
    inv.items.get_mut(&bridge).expect("bridge").name = crate::world::bridge::NAME.into();
    let backup = Backup::capture(&inv, cof, &worn).expect("capture");
    assert!(!backup.items.iter().any(|item| item.item_id == bridge), "no bridge in a new backup");
    let (change, _) = appearance::plan(
        &inv,
        appearance::Action::ImportJson {
            backup: Box::new(old.clone()),
            name: "Imported".into(),
            partial: false,
        },
        &HashMap::new(),
    )
    .expect("import");
    assert!(change.create.is_some());
    assert!(!change.links.iter().any(|l| l.target == bridge), "not imported into the outfit");
    // review: neither attached again from the backup (the viewer wears its
    // own) nor reported missing, even once that bridge is gone
    let resolved = old.resolve(&inv, agent);
    assert!(!resolved.links.iter().any(|l| l.target == bridge));
    assert!(resolved.missing.is_empty(), "not missing");
    let import = |backup: &Backup| appearance::Action::ImportJson {
        backup: Box::new(backup.clone()),
        name: "Imported".into(),
        partial: false,
    };
    let attached = appearance::attachments_for_action(&inv, agent, &import(&old));
    assert!(!attached.is_empty(), "the other objects are attached");
    assert!(!attached.iter().any(|request| request.item_id == bridge), "not the bridge");
    let mut named = old.clone();
    for item in named.items.iter_mut().filter(|item| item.item_id == bridge) {
        item.name = crate::world::bridge::NAME.into();
    }
    inv.items.remove(&bridge);
    let resolved = named.resolve(&inv, agent);
    assert!(resolved.missing.is_empty(), "a bridge since replaced is not missing");
    assert!(!resolved.links.is_empty());
    // an ordinary object that is gone still is
    assert_eq!(old.resolve(&inv, agent).missing.len(), 1);
}
