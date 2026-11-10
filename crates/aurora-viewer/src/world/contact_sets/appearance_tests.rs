use super::appearance::ICONS;
use super::*;

#[test]
fn contact_visuals_cascade_per_attribute_and_keep_non_friends() {
    let mut sets = ContactSets::default();
    let id = Uuid::from_u128(1);
    for name in ["Staff", "Friends"] {
        sets.add_set(name);
        sets.add_to_set(&[id], name, |_| false);
    }
    sets.set_color("Staff", [1.0, 0.0, 0.0, 1.0]);
    sets.set_color("Friends", [0.0, 0.0, 1.0, 1.0]);
    sets.set_visuals(
        "Staff",
        Visuals {
            border: true,
            icon: Some(0),
            ..Default::default()
        },
    );
    sets.set_visuals(
        "Friends",
        Visuals {
            border: true,
            ui_name: true,
            icon: Some(1),
            ..Default::default()
        },
    );
    sets.move_priority("Staff", true);
    assert_eq!(sets.appearance(&id).border, Some([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(sets.appearance(&id).ui_name, Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(sets.appearance(&id).icons.len(), 1);
    assert_eq!(sets.appearance(&id).icons[0].set, "Staff");
    let mut friends = sets.set("Friends").unwrap().visuals.clone();
    friends.replace_friend = true;
    sets.set_visuals("Friends", friends);
    assert!(sets.appearance(&id).replace_friend);
    let mut staff = sets.set("Staff").unwrap().visuals.clone();
    staff.border = false;
    sets.set_visuals("Staff", staff);
    assert_eq!(sets.appearance(&id).border, Some([0.0, 0.0, 1.0, 1.0]));
    sets.set_stack_icons(true);
    assert_eq!(sets.appearance(&id).icons.len(), 2);
    sets.rename_set("Staff", "Team");
    assert_eq!(sets.appearance(&id).icons[0].set, "Team");
    sets.remove_from_set(&[id], "Team", |_| false);
    assert_eq!(sets.appearance(&id).icons.len(), 1);
    sets.remove_set("Friends", |_| false);
    assert_eq!(**sets.appearance(&id), Appearance::default());
}

#[test]
fn contact_visuals_round_trip_and_old_files_inherit() {
    let mut sets = ContactSets::default();
    let id = Uuid::from_u128(2);
    sets.add_set("Example");
    sets.add_to_set(&[id], "Example", |_| true);
    let mut old = Llsd::new_map();
    let mut old_set = Llsd::new_map();
    let mut members = Llsd::new_map();
    members.insert(id.to_string(), "");
    old_set.insert("friends", members);
    old.insert("Example", old_set);
    let mut imported = ContactSets::default();
    imported.import(&old);
    assert_eq!(**imported.appearance(&id), Appearance::default());
    let v = Visuals {
        border: true,
        background: true,
        nameplate_name: true,
        ui_name: true,
        chat_name: true,
        chat_text: true,
        minimap: true,
        icon: Some(3),
        replace_friend: true,
        size: PlateSize::Reduced,
        marker: MarkerScope::Off,
        marker_shape: MarkerShape::Circle,
        marker_fade: None,
    };
    sets.set_visuals("Example", v.clone());
    sets.set_stack_icons(true);
    let xml = aurora_llsd::to_xml(&sets.export());
    let llsd = aurora_llsd::from_xml(&xml).unwrap();
    let mut restored = ContactSets::default();
    restored.import(&llsd);
    assert_eq!(restored.set("Example").unwrap().visuals, v);
    assert!(restored.stack_icons);
    assert_eq!(restored.appearance(&id), sets.appearance(&id));
    let mut invalid = llsd;
    let mut unsupported = Llsd::new_map();
    unsupported.insert("icon", "not-an-icon");
    unsupported.insert("size", "unexpected");
    let mut set = Llsd::new_map();
    set.insert("aurora_visuals", unsupported);
    invalid.insert("Example", set);
    let mut restored = ContactSets::default();
    restored.import(&invalid);
    assert_eq!(restored.set("Example").unwrap().visuals.icon, None);
    assert_eq!(restored.set("Example").unwrap().visuals.size, PlateSize::Inherit);
}

#[test]
fn contact_visuals_picker_assets_and_labels_exist() {
    assert!((30..=50).contains(&ICONS.len()));
    for (i, icon) in ICONS.iter().enumerate() {
        assert!(crate::ui::icons::embedded(icon), "missing icon {icon}");
        let key = appearance::ICON_LABELS[i];
        assert_ne!(aurora_i18n::text(key), key);
    }
}

#[test]
fn contact_markers_cascade_by_priority_and_round_trip() {
    let mut sets = ContactSets::default();
    let id = Uuid::from_u128(1);
    for name in ["Staff", "Friends", "Plain"] {
        sets.add_set(name);
        sets.add_to_set(&[id], name, |_| false);
    }
    sets.set_color("Staff", [1.0, 0.0, 0.0, 1.0]);
    sets.set_color("Friends", [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(sets.appearance(&id).marker, None, "off by default");
    assert!(!sets.has_markers(), "nobody to look for");
    sets.set_visuals(
        "Friends",
        Visuals {
            marker: MarkerScope::Region,
            marker_shape: MarkerShape::Diamond,
            marker_fade: Some(9000),
            icon: Some(2),
            ..Default::default()
        },
    );
    let friends = sets.appearance(&id).marker.expect("marker");
    assert!(sets.has_markers());
    assert_eq!(
        friends,
        Marker {
            scope: MarkerScope::Region,
            shape: MarkerShape::Diamond,
            color: [0.0, 0.0, 1.0, 1.0],
            border: None,
            background: None,
            icon: Some(2),
            fade: Some(*appearance::MARKER_FADE_RANGE.end()),
        }
    );
    // the set on top wins as a whole, its lack of a symbol included
    sets.set_visuals(
        "Staff",
        Visuals {
            marker: MarkerScope::Parcel,
            ..Default::default()
        },
    );
    sets.move_priority("Staff", true);
    sets.move_priority("Staff", true);
    let staff = sets.appearance(&id).marker.expect("marker");
    assert_eq!(
        (staff.scope, staff.color, staff.icon, staff.fade),
        (MarkerScope::Parcel, [1.0, 0.0, 0.0, 1.0], None, None)
    );

    let visuals = sets.set("Friends").unwrap().visuals.clone();
    assert_eq!(Visuals::read(&visuals.write()), visuals);
    let visuals = sets.set("Staff").unwrap().visuals.clone();
    assert_eq!(Visuals::read(&visuals.write()), visuals);
    assert_eq!(staff.shape, MarkerShape::Circle, "the set's own shape, not the other's");
    // the border is the plate's, even from a set that gives no marker
    assert_eq!(staff.border, None);
    sets.set_color("Plain", [0.0, 1.0, 0.0, 1.0]);
    sets.set_visuals(
        "Plain",
        Visuals {
            border: true,
            background: true,
            ..Default::default()
        },
    );
    let plate = sets.appearance(&id).marker.expect("marker");
    assert_eq!(plate.border, Some([0.0, 1.0, 0.0, 1.0]));
    assert_eq!(plate.background, Some([0.0, 1.0, 0.0, 1.0]));
    assert_eq!(Visuals::read(&Llsd::new_map()).marker, MarkerScope::Off, "older files");
    assert_eq!(Visuals::read(&Llsd::new_map()).marker_shape, MarkerShape::Circle);
    for name in ["Staff", "Friends"] {
        let mut visuals = sets.set(name).unwrap().visuals.clone();
        visuals.marker = MarkerScope::Off;
        sets.set_visuals(name, visuals);
    }
    assert!(!sets.has_markers(), "every marker off again");
}
