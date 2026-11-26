use super::*;
use crate::ui::{Panels, bars, chat::ChatUi, icons::Icons};
use egui::Vec2;

pub(super) fn button(command: &str) -> ToolButton {
    ToolButton {
        command: command.into(),
        icon: "test-icon".into(),
        tooltip: command.into(),
    }
}

#[test]
fn restore_defaults_reunites_information_and_separates_audio_without_an_slurl_toggle() {
    let mut settings = ToolbarSettings {
        composed: true,
        top: vec![button("location_bar"), button("inventory")],
        ..Default::default()
    };
    let mut state = ToolbarUi {
        pending: Some(Edit::Restore),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    settings.compose_top();
    let groups: Vec<Vec<_>> = settings
        .top
        .split(|button| structure::is_separator(&button.command))
        .map(|group| group.iter().map(|button| button.command.as_str()).collect())
        .collect();
    assert_eq!(
        groups,
        [
            vec!["aurora_menu"],
            vec!["report_bug"],
            vec!["bar_navigation"],
            vec![
                "bar_conversations",
                "bar_status",
                "bar_balance",
                "bar_clock",
                "bar_ping",
                "bar_fps",
                "bar_network",
            ],
            vec!["bar_media", "bar_music", "bar_volume"],
            vec!["bar_notifications"],
        ]
    );
    assert!(!settings.top.iter().any(|button| button.command == "location_bar"));
    let customized = settings.top.clone();
    assert!(!settings.compose_top());
    assert_eq!(settings.top, customized, "composition never regroups a saved layout");
}

#[test]
fn nearby_chat_migrates_the_actual_skin_once_and_moves_between_edges_and_drawers() {
    let mut skin = layout();
    skin.toolbar = vec![button("inventory"), button("skin_custom")];
    let mut settings = ToolbarSettings::default();
    assert!(settings.compose_chat(&skin));
    assert_eq!(&settings.buttons(&skin)[2..], skin.toolbar);
    assert_eq!(settings.block_styles["bar_chat"].align, Some(Align::Start));
    assert!(!settings.compose_chat(&skin));
    let mut state = ToolbarUi::default();
    for bar in Bar::ALL {
        state.pending = Some(Edit::Insert(bar, button("bar_chat"), usize::MAX));
        state.commit(&skin, &mut settings);
        assert_eq!(settings.location("bar_chat", &skin), Some(bar));
        let count = Bar::ALL
            .into_iter()
            .flat_map(|bar| settings.bar(bar, &skin))
            .filter(|b| b.command == "bar_chat")
            .count();
        assert_eq!(count, 1);
    }
    let drawer = drawers::create(&mut settings, Bar::Left, &skin);
    state.pending = Some(Edit::DrawerInsert(drawer.clone(), button("bar_chat"), 0));
    assert!(state.commit(&skin, &mut settings));
    assert_eq!(settings.drawer_of("bar_chat"), Some(drawer.as_str()));
    let json = serde_json::to_string(&settings).expect("chat layout");
    settings = serde_json::from_str(&json).expect("restored chat layout");
    assert!(!settings.compose_chat(&skin));
    state.pending = Some(Edit::Insert(Bar::Right, button("bar_chat"), 0));
    assert!(state.commit(&skin, &mut settings));
    assert!(!settings.drawers.contains_key(&drawer), "moving the last child removes its drawer");
    settings.locked = true;
    state.pending = Some(Edit::Remove("bar_chat".into()));
    assert!(!state.commit(&skin, &mut settings));
    settings.locked = false;
    state.pending = Some(Edit::Remove("bar_chat".into()));
    assert!(state.commit(&skin, &mut settings));
    assert!(!settings.compose_chat(&skin));
    assert!(
        settings.location("bar_chat", &skin).is_none(),
        "intentional removal survives migration"
    );
    state.pending = Some(Edit::Restore);
    assert!(state.commit(&skin, &mut settings));
    assert!(settings.compose_chat(&skin));
    assert_eq!(&settings.buttons(&skin)[2..], skin.toolbar);
    assert_eq!(settings.block_styles["bar_chat"].align, Some(Align::Start));
    settings.block_styles.get_mut("bar_chat").expect("chat block").align = Some(Align::Center);
    assert!(!settings.compose_chat(&skin));
    assert_eq!(
        settings.block_styles["bar_chat"].align,
        Some(Align::Center),
        "an explicit alignment is retained"
    );
}

#[test]
fn editing_groups_split_and_merge_without_losing_order_or_separator_preferences() {
    let mut settings = ToolbarSettings {
        top: vec![
            button("inventory"),
            button("people"),
            structure::separator("a"),
            structure::separator("b"),
            button("preferences"),
            button("worldmap"),
        ],
        ..Default::default()
    };
    settings
        .separators
        .insert("separator:a".into(), structure::SeparatorStyle { size: 70.0, stretch: true });
    let mut state = ToolbarUi {
        pending: Some(Edit::Split(Bar::Top, "people".into(), true)),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    assert!(structure::is_separator(&settings.top[1].command));
    state.pending = Some(Edit::Split(Bar::Top, "people".into(), true));
    assert!(!state.commit(&layout(), &mut settings), "an existing boundary is not duplicated");
    state.pending = Some(Edit::Merge(Bar::Top, "people".into(), false));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.separators.contains_key("separator:a"));
    assert!(!settings.top.iter().any(|b| b.command == "separator:b"));
    state.pending = Some(Edit::Merge(Bar::Top, "people".into(), true));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(
        settings.top.iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
        ["inventory", "people", "preferences", "worldmap"]
    );
    settings.locked = true;
    let before = serde_json::to_string(&settings).expect("locked groups");
    state.pending = Some(Edit::Split(Bar::Top, "people".into(), true));
    assert!(!state.commit(&layout(), &mut settings));
    assert_eq!(serde_json::to_string(&settings).expect("unchanged"), before);
}

#[test]
fn proximity_drop_moves_once_and_brackets_a_new_group_atomically_on_every_edge() {
    let task_layout = layout();
    for bar in Bar::ALL {
        let drawer_bar = if bar == Bar::Right { Bar::Left } else { Bar::Right };
        let mut settings = ToolbarSettings {
            composed: true,
            buttons: Some(Vec::new()),
            top: Vec::new(),
            ..Default::default()
        };
        settings.set_bar(bar, vec![button("inventory"), button("people")]);
        settings.set_bar(drawer_bar, vec![button("drawer:tools")]);
        settings.drawers.insert(
            "drawer:tools".into(),
            drawers::Drawer {
                buttons: vec![button("radar")],
                ..Default::default()
            },
        );
        let mut state = ToolbarUi {
            pending: Some(Edit::InsertGroup(bar, button("radar"), 1)),
            ..Default::default()
        };
        assert!(state.commit(&layout(), &mut settings));
        let list = settings.bar(bar, &task_layout);
        assert_eq!(
            list.iter()
                .filter(|b| !structure::is_separator(&b.command))
                .map(|b| b.command.as_str())
                .collect::<Vec<_>>(),
            ["inventory", "radar", "people"]
        );
        assert_eq!(list.len(), 5);
        assert!(structure::is_separator(&list[1].command));
        assert!(structure::is_separator(&list[3].command));
        assert_ne!(list[1].command, list[3].command);
        assert!(!settings.drawers.contains_key("drawer:tools"));
        assert!(settings.location("drawer:tools", &layout()).is_none());
        assert_eq!(settings.location("radar", &layout()), Some(bar));
        settings.locked = true;
        let json = serde_json::to_string(&settings).expect("group save");
        state.pending = Some(Edit::InsertGroup(bar, button("preferences"), 1));
        assert!(!state.commit(&layout(), &mut settings));
        assert_eq!(serde_json::to_string(&settings).expect("locked layout"), json);
        let mut restored: ToolbarSettings = serde_json::from_str(&json).expect("reload");
        restored.sanitize();
        assert_eq!(commands(restored.bar(bar, &layout())), commands(settings.bar(bar, &layout())));
    }
}

#[test]
fn proximity_drop_reuses_an_existing_separator_and_keeps_its_size() {
    let mut settings = ToolbarSettings {
        top: vec![button("inventory"), structure::separator("kept"), button("people")],
        ..Default::default()
    };
    let style = structure::SeparatorStyle { size: 80.0, stretch: true };
    settings.separators.insert("separator:kept".into(), style);
    let mut state = ToolbarUi {
        pending: Some(Edit::InsertGroup(Bar::Top, button("radar"), 2)),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.top.len(), 5);
    assert_eq!(settings.top[1].command, "separator:kept");
    assert_eq!(settings.top[2].command, "radar");
    assert!(structure::is_separator(&settings.top[3].command));
    assert_eq!(settings.separators["separator:kept"], style);
}

#[test]
fn repeated_nearby_drops_of_an_isolated_group_do_not_grow_or_rewrite_its_separators() {
    let task_layout = layout();
    for bar in Bar::ALL {
        for alone in [false, true] {
            let mut settings = ToolbarSettings {
                composed: true,
                top: Vec::new(),
                buttons: Some(Vec::new()),
                ..Default::default()
            };
            let mut list = vec![structure::separator("outer"), button("radar")];
            if !alone {
                list.splice(0..0, [button("inventory"), structure::separator("boundary")]);
            }
            settings.set_bar(bar, list);
            settings
                .separators
                .insert("separator:outer".into(), structure::SeparatorStyle { size: 80.0, stretch: true });
            let original = settings.clone();
            let mut state = ToolbarUi::default();
            let first_rank = usize::from(!alone);
            for _ in 0..20 {
                for rank in first_rank..=settings.bar(bar, &task_layout).len() {
                    state.pending = Some(Edit::InsertGroup(bar, button("radar"), rank));
                    assert!(!state.commit(&task_layout, &mut settings), "same group on {bar:?}, rank={rank}");
                    assert_eq!(settings, original);
                }
            }
        }
    }
}

#[test]
fn moving_singleton_groups_reuses_the_existing_boundary_ids_and_size_across_edges() {
    let task_layout = layout();
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        top: vec![button("inventory"), structure::separator("carried"), button("radar")],
        left: vec![button("people")],
        ..Default::default()
    };
    let style = structure::SeparatorStyle { size: 64.0, stretch: true };
    settings.separators.insert("separator:carried".into(), style);
    let mut state = ToolbarUi::default();
    for _ in 0..12 {
        for bar in [Bar::Left, Bar::Top] {
            state.pending = Some(Edit::InsertGroup(bar, button("radar"), settings.bar(bar, &task_layout).len()));
            assert!(state.commit(&task_layout, &mut settings));
            let separators: Vec<_> = Bar::ALL
                .into_iter()
                .flat_map(|bar| settings.bar(bar, &task_layout))
                .filter(|b| structure::is_separator(&b.command))
                .collect();
            assert_eq!(separators.len(), 1);
            assert_eq!(separators[0].command, "separator:carried");
            assert_eq!(settings.separators["separator:carried"], style);
        }
    }
}

#[test]
fn repeated_round_trips_through_an_empty_bar_reuse_the_exposed_boundary() {
    let task_layout = layout();
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        top: vec![button("inventory"), structure::separator("kept"), button("radar")],
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    for _ in 0..20 {
        state.pending = Some(Edit::Insert(Bar::Left, button("radar"), 0));
        assert!(state.commit(&task_layout, &mut settings));
        state.pending = Some(Edit::InsertGroup(Bar::Top, button("radar"), 1));
        assert!(state.commit(&task_layout, &mut settings));
        assert_eq!(commands(&settings.top), ["inventory", "separator:kept", "radar"]);
        assert!(settings.left.is_empty());
    }
}

#[test]
fn live_conversation_placement_stays_present_across_drawer_and_group_moves_until_removal() {
    let task_layout = layout();
    let mut settings = ToolbarSettings {
        composed: true,
        top: vec![button("bar_conversations")],
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    assert_eq!(
        settings.command_placement("bar_conversations", &task_layout),
        Some((Bar::Top, None, 0))
    );
    let drawer = drawers::create(&mut settings, Bar::Left, &task_layout);
    state.pending = Some(Edit::DrawerInsert(drawer.clone(), button("bar_conversations"), 0));
    assert!(state.commit(&task_layout, &mut settings));
    assert_eq!(
        settings.command_placement("bar_conversations", &task_layout),
        Some((Bar::Left, Some(drawer.clone()), 0))
    );
    state.pending = Some(Edit::Insert(Bar::Right, settings.left[0].clone(), 0));
    assert!(state.commit(&task_layout, &mut settings));
    assert_eq!(
        settings.command_placement("bar_conversations", &task_layout),
        Some((Bar::Right, Some(drawer), 0))
    );
    state.pending = Some(Edit::InsertGroup(Bar::Top, button("bar_conversations"), 0));
    assert!(state.commit(&task_layout, &mut settings));
    assert_eq!(
        settings.command_placement("bar_conversations", &task_layout),
        Some((Bar::Top, None, 0))
    );
    state.pending = Some(Edit::Remove("bar_conversations".into()));
    assert!(state.commit(&task_layout, &mut settings));
    assert!(settings.command_placement("bar_conversations", &task_layout).is_none());
}

#[test]
fn drawers_preserve_children_locations_and_support_move_insert_reorder_rename_and_unfold() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        top: vec![
            button("inventory"),
            button("people"),
            structure::separator("kept"),
            button("preferences"),
        ],
        ..Default::default()
    };
    let mut state = ToolbarUi {
        pending: Some(Edit::Fold(Bar::Top, "inventory".into())),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    let id = settings.top[0].command.clone();
    assert!(drawers::is_drawer(&id));
    assert_eq!(settings.location("people", &layout()), Some(Bar::Top));
    assert_eq!(settings.drawer_of("people"), Some(id.as_str()));
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(id.clone()),
        |o| {
            o.name = "Tools".into();
            o.mode = settings.drawers[&id].mode;
            o.look = Default::default();
        },
    ));
    assert!(state.commit(&layout(), &mut settings));
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(id.clone()),
        |o| {
            o.name = "Tools".into();
            o.mode = ButtonMode::TextOnly;
            o.look = Default::default();
        },
    ));
    assert!(state.commit(&layout(), &mut settings));
    state.pending = Some(Edit::Insert(Bar::Right, settings.top[0].clone(), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.location("inventory", &layout()), Some(Bar::Right));
    state.pending = Some(Edit::DrawerInsert(id.clone(), button("preferences"), 0));
    assert!(state.commit(&layout(), &mut settings));
    state.pending = Some(Edit::DrawerInsert(id.clone(), button("people"), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(
        settings.drawers[&id].buttons.iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
        ["people", "preferences", "inventory"]
    );
    state.pending = Some(Edit::Insert(Bar::Bottom, button("people"), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.drawer_of("people"), None);
    let json = serde_json::to_string(&settings).expect("drawer save");
    let mut loaded: ToolbarSettings = serde_json::from_str(&json).expect("drawer load");
    loaded.sanitize();
    assert_eq!(loaded.location("preferences", &layout()), Some(Bar::Right));
    assert_eq!(loaded.right[0].tooltip, "Tools");
    assert_eq!(loaded.drawers[&id].mode, ButtonMode::TextOnly);
    state.pending = Some(Edit::DrawerInsert(id.clone(), loaded.right[0].clone(), 0));
    assert!(!state.commit(&layout(), &mut loaded), "drawers cannot contain drawers");
    state.pending = Some(Edit::Unfold(Bar::Right, id.clone()));
    assert!(state.commit(&layout(), &mut loaded));
    assert!(!loaded.drawers.contains_key(&id));
    assert_eq!(
        loaded.right.iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
        ["preferences", "inventory"]
    );
    assert!(loaded.top.iter().any(|b| b.command == "separator:kept"));
}

#[test]
fn drawer_defaults_load_older_settings_and_mode_edits_respect_lock_and_identity() {
    let old: drawers::Drawer = serde_json::from_str(r#"{"buttons":[]}"#).expect("old drawer");
    assert_eq!(old.mode, ButtonMode::IconsWithText);
    assert!(old.closes, "a drawer saved before closes after a click");
    let kept: drawers::Drawer = serde_json::from_str(r#"{"buttons":[],"closes":false}"#).expect("drawer");
    assert!(!kept.closes);
    let mut settings = ToolbarSettings {
        top: Vec::new(),
        buttons: Some(Vec::new()),
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    let first = drawers::create(&mut settings, Bar::Left, &layout());
    let second = drawers::create(&mut settings, Bar::Right, &layout());
    for mode in [ButtonMode::IconsOnly, ButtonMode::TextOnly, ButtonMode::IconsWithText] {
        state.pending = Some(block_window::edit_of(
            &settings,
            &layout(),
            block_window::Target::Drawer(first.clone()),
            |o| {
                o.name = String::new();
                o.mode = mode;
                o.look = Default::default();
            },
        ));
        assert!(state.commit(&layout(), &mut settings));
        assert_eq!(settings.drawers[&first].mode, mode);
        assert_eq!(settings.drawers[&second].mode, ButtonMode::IconsWithText);
        state.pending = Some(block_window::edit_of(
            &settings,
            &layout(),
            block_window::Target::Drawer(first.clone()),
            |o| {
                o.name = String::new();
                o.mode = mode;
                o.look = Default::default();
            },
        ));
        assert!(!state.commit(&layout(), &mut settings));
    }
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(first.clone()),
        |o| {
            o.closes = false;
        },
    ));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.drawers[&first].closes && settings.drawers[&second].closes);
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(first.clone()),
        |o| {
            o.closes = false;
        },
    ));
    assert!(!state.commit(&layout(), &mut settings));
    settings.locked = true;
    let before = settings.clone();
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(first.clone()),
        |o| {
            o.closes = true;
        },
    ));
    assert!(!state.commit(&layout(), &mut settings));
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(first.clone()),
        |o| {
            o.name = String::new();
            o.mode = ButtonMode::IconsOnly;
            o.look = Default::default();
        },
    ));
    assert!(!state.commit(&layout(), &mut settings));
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(first),
        |o| {
            o.name = "Blocked".into();
            o.mode = ButtonMode::IconsOnly;
            o.look = Default::default();
        },
    ));
    assert!(!state.commit(&layout(), &mut settings));
    assert_eq!(settings, before);
}

#[test]
fn drawer_options_are_atomic_persisted_and_empty_drawers_are_pruned_on_removal() {
    for remove in [true, false] {
        let mut settings = ToolbarSettings {
            composed: true,
            buttons: Some(Vec::new()),
            top: Vec::new(),
            ..Default::default()
        };
        let id = drawers::create(&mut settings, Bar::Left, &layout());
        let empty = drawers::create(&mut settings, Bar::Right, &layout());
        let mut state = ToolbarUi {
            pending: Some(Edit::DrawerInsert(id.clone(), button("inventory"), 0)),
            ..Default::default()
        };
        assert!(state.commit(&layout(), &mut settings));
        let own = appearance::Appearance {
            mode: Some(ButtonMode::TextOnly),
            icon: Some("star-filled".into()),
        };
        state.pending = Some(block_window::edit_of(
            &settings,
            &layout(),
            block_window::Target::Drawer(id.clone()),
            |o| {
                o.name = "Tools".into();
                o.mode = ButtonMode::IconsOnly;
                o.look = own.clone();
            },
        ));
        assert!(state.commit(&layout(), &mut settings));
        assert_eq!(settings.left[0].tooltip, "Tools");
        assert_eq!(settings.drawers[&id].mode, ButtonMode::IconsOnly);
        assert_eq!(settings.appearance[&id], own);
        assert_eq!(
            serde_json::from_str::<ToolbarSettings>(&serde_json::to_string(&settings).expect("encode")).expect("decode"),
            settings
        );
        settings.locked = true;
        let before = settings.clone();
        state.pending = Some(block_window::edit_of(
            &settings,
            &layout(),
            block_window::Target::Drawer(id.clone()),
            |o| {
                o.name = "Blocked".into();
                o.mode = ButtonMode::IconsWithText;
                o.look = Default::default();
            },
        ));
        assert!(!state.commit(&layout(), &mut settings));
        assert_eq!(settings, before);
        settings.locked = false;
        state.pending = Some(Edit::DrawerInsert(id.clone(), structure::divider("decoration"), 0));
        assert!(state.commit(&layout(), &mut settings));
        state.pending = Some(if remove {
            Edit::Remove("inventory".into())
        } else {
            Edit::Insert(Bar::Top, button("inventory"), 0)
        });
        assert!(state.commit(&layout(), &mut settings));
        assert!(
            !settings.drawers.contains_key(&id),
            "decorations cannot keep an emptied drawer alive"
        );
        assert!(settings.location(&id, &layout()).is_none());
        assert!(
            settings.drawers.contains_key(&empty),
            "new empty placeholders remain available to fill"
        );
    }
}

#[test]
fn drawer_sanitization_rejects_cycles_duplicates_and_unplaced_containers() {
    let mut settings = ToolbarSettings {
        buttons: Some(Vec::new()),
        top: vec![button("drawer:a"), button("inventory")],
        ..Default::default()
    };
    settings.drawers.insert(
        "drawer:a".into(),
        drawers::Drawer {
            buttons: vec![button("drawer:a"), button("drawer:b"), button("inventory"), button("people")],
            ..Default::default()
        },
    );
    settings.drawers.insert(
        "drawer:b".into(),
        drawers::Drawer {
            buttons: vec![button("drawer:a")],
            ..Default::default()
        },
    );
    settings.sanitize();
    assert_eq!(settings.drawers.len(), 1);
    assert_eq!(settings.drawers["drawer:a"].buttons, [button("people")]);
    assert_eq!(settings.location("people", &layout()), Some(Bar::Top));
    settings.locked = true;
    let mut state = ToolbarUi {
        pending: Some(Edit::Fold(Bar::Top, "people".into())),
        ..Default::default()
    };
    assert!(!state.commit(&layout(), &mut settings));
}

#[test]
fn moving_navigation_into_a_drawer_keeps_the_address_visible_and_does_not_duplicate_it() {
    let mut settings = ToolbarSettings::default();
    settings.compose_top();
    let mut state = ToolbarUi::default();
    let id = drawers::create(&mut settings, Bar::Left, &layout());
    state.pending = Some(Edit::DrawerInsert(id, button("bar_navigation"), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_ne!(settings.address_visible, Some(false));
    settings.show_address_bar(true);
    assert_eq!(settings.location("bar_navigation", &layout()), Some(Bar::Left));
    assert!(!settings.top.iter().any(|b| b.command == "bar_navigation"));
}

#[test]
fn drawer_contents_participate_in_live_menu_refresh_and_bug_visibility() {
    let mut settings = ToolbarSettings {
        composed: true,
        top: vec![button("drawer:tools"), button("inventory")],
        ..Default::default()
    };
    settings.drawers.insert(
        "drawer:tools".into(),
        drawers::Drawer {
            buttons: vec![button("report_bug"), button("menu:common-build/import-menu")],
            ..Default::default()
        },
    );
    assert!(
        !edit_mode::fold_allowed(&settings.top, "inventory"),
        "a group cannot contain another drawer"
    );
    let ctx = egui::Context::default();
    commands::publish(
        &ctx,
        commands::Catalogue {
            entries: vec![commands::Entry {
                button: button("menu:common-build/import-menu"),
                parent: String::new(),
                group: false,
                info: false,
                separator: false,
                enabled: true,
                checked: None,
            }],
        },
    );
    assert!(!ToolbarUi::default().needs_commands(&ctx, Some(&ToolbarSettings::default())));
    assert!(ToolbarUi::default().needs_commands(&ctx, Some(&settings)));
    assert!(settings.set_bug_visible(&layout(), false));
    assert!(settings.location("report_bug", &layout()).is_none());
    assert!(!settings.set_bug_visible(&layout(), false));
    assert_eq!(settings.drawers["drawer:tools"].buttons.len(), 1);
}

#[test]
fn separator_styles_are_independent_persisted_and_protected_by_the_bar_lock() {
    let first = structure::separator("first");
    let second = structure::separator("second");
    let mut settings = ToolbarSettings {
        top: vec![first.clone(), second.clone()],
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    let style = structure::SeparatorStyle { size: 80.0, stretch: true };
    state.pending = Some(Edit::Separator(first.command.clone(), style));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.separators.contains_key(&second.command));
    state.pending = Some(Edit::Insert(Bar::Left, first.clone(), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.location(&first.command, &layout()), Some(Bar::Left));
    assert_eq!(settings.separators[&first.command], style);
    let json = serde_json::to_string(&settings).expect("separator settings");
    let restored: ToolbarSettings = serde_json::from_str(&json).expect("separator settings reload");
    assert_eq!(restored.separators[&first.command], style);
    state.pending = Some(Edit::Separator(first.command.clone(), style));
    assert!(!state.commit(&layout(), &mut settings));
    settings.locked = true;
    state.pending = Some(Edit::Separator(first.command.clone(), Default::default()));
    assert!(!state.commit(&layout(), &mut settings));
    settings.locked = false;
    state.pending = Some(Edit::Separator(first.command.clone(), Default::default()));
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.separators.is_empty());
}

#[test]
fn aggregate_chrome_layouts_expand_in_place_once_and_preserve_existing_placements() {
    for bar in Bar::ALL {
        let mut settings = ToolbarSettings {
            composed: true,
            top: Vec::new(),
            buttons: Some(Vec::new()),
            locked: true,
            ..Default::default()
        };
        let original = vec![
            button("bar_account"),
            structure::separator("kept"),
            button("bar_system"),
            button("inventory"),
        ];
        match bar {
            Bar::Top => settings.top = original,
            Bar::Left => settings.left = original,
            Bar::Right => settings.right = original,
            Bar::Bottom => settings.buttons = Some(original),
        }
        settings.appearance.insert(
            "bar_system".into(),
            appearance::Appearance {
                icon: Some("star-filled".into()),
                ..Default::default()
            },
        );
        assert!(settings.compose_top());
        assert_eq!(
            settings
                .bar(bar, &layout())
                .iter()
                .map(|b| {
                    if structure::is_separator(&b.command) && b.command != "separator:kept" {
                        structure::SEPARATOR
                    } else {
                        b.command.as_str()
                    }
                })
                .collect::<Vec<_>>(),
            [
                "bar_status",
                "toolbar_separator",
                "bar_balance",
                "separator:kept",
                "bar_clock",
                "bar_ping",
                "bar_network",
                "toolbar_separator",
                "bar_fps",
                "toolbar_separator",
                "bar_media",
                "bar_music",
                "bar_volume",
                "inventory"
            ]
        );
        assert!(settings.locked);
        assert_eq!(settings.appearance["bar_volume"].icon.as_deref(), Some("star-filled"));
        assert!(!settings.compose_top());
        let json = serde_json::to_string(&settings).expect("split layout");
        let mut restored: ToolbarSettings = serde_json::from_str(&json).expect("restored layout");
        assert!(!restored.compose_top());
        assert_eq!(serde_json::to_string(&restored).expect("unchanged layout"), json);
    }
    let mut settings = ToolbarSettings {
        composed: true,
        top: vec![button("bar_system")],
        right: vec![button("bar_fps")],
        ..Default::default()
    };
    assert!(settings.compose_top());
    assert_eq!(settings.location("bar_fps", &layout()), Some(Bar::Right));
    assert!(!settings.top.iter().any(|b| b.command == "bar_fps"));
    settings.top.retain(|b| b.command != "bar_clock");
    assert!(!settings.compose_top(), "a removed component stays removed");
}

#[test]
fn the_address_shortcut_restores_removed_navigation_and_keeps_existing_placements() {
    let layout = layout();
    let mut settings = ToolbarSettings::default();
    settings.compose_top();
    let mut state = ToolbarUi {
        pending: Some(Edit::Remove("bar_navigation".into())),
        ..Default::default()
    };
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(settings.address_visible, Some(false));
    settings.show_address_bar(true);
    assert_eq!(settings.location("bar_navigation", &layout), Some(Bar::Top));
    state.pending = Some(Edit::Insert(Bar::Bottom, button("bar_navigation"), 0));
    assert!(state.commit(&layout, &mut settings));
    settings.show_address_bar(false);
    settings.show_address_bar(true);
    assert_eq!(settings.location("bar_navigation", &layout), Some(Bar::Bottom));
}

fn layout() -> Layout {
    Layout {
        toolbar: vec![button("inventory"), button("people"), button("preferences")],
        ..Default::default()
    }
}

#[test]
fn repeated_separators_move_remove_and_round_trip_independently() {
    let layout = layout();
    let mut settings = ToolbarSettings::default();
    let mut state = ToolbarUi::default();
    // apart: gaps next to each other are one (`single_gaps`)
    for rank in [1, 3] {
        state.pending = Some(Edit::Insert(Bar::Bottom, button(structure::SEPARATOR), rank));
        assert!(state.commit(&layout, &mut settings));
    }
    let gaps: Vec<_> = settings
        .buttons(&layout)
        .iter()
        .filter(|b| structure::is_separator(&b.command))
        .cloned()
        .collect();
    assert_eq!(gaps.len(), 2);
    assert_ne!(gaps[0].command, gaps[1].command);
    state.pending = Some(Edit::Insert(Bar::Left, gaps[0].clone(), 0));
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(settings.location(&gaps[0].command, &layout), Some(Bar::Left));
    state.pending = Some(Edit::Remove(gaps[1].command.clone()));
    assert!(state.commit(&layout, &mut settings));
    let mut restored: ToolbarSettings = serde_json::from_str(&serde_json::to_string(&settings).expect("settings")).expect("round trip");
    restored.sanitize();
    assert_eq!(restored.left, settings.left);
    assert_eq!(restored.location(&gaps[1].command, &layout), None);
    restored.locked = true;
    state.pending = Some(Edit::Remove(gaps[0].command.clone()));
    assert!(!state.commit(&layout, &mut restored));
}

#[test]
fn top_composition_migrates_once_and_keeps_custom_buttons_and_appearance() {
    let mut settings = ToolbarSettings {
        top: vec![button("inventory"), button("people")],
        left: vec![button("preferences")],
        ..Default::default()
    };
    settings.appearance.insert(
        "inventory".into(),
        appearance::Appearance {
            mode: Some(ButtonMode::TextOnly),
            icon: Some("star-filled".into()),
        },
    );
    assert!(settings.compose_top());
    let custom: Vec<_> = settings
        .top
        .iter()
        .filter(|button| matches!(button.command.as_str(), "inventory" | "people"))
        .map(|b| b.command.as_str())
        .collect();
    assert_eq!(custom, ["inventory", "people"]);
    for command in [
        "aurora_menu",
        "report_bug",
        "bar_navigation",
        "bar_status",
        "bar_balance",
        "bar_clock",
        "bar_ping",
        "bar_network",
        "bar_fps",
        "bar_media",
        "bar_music",
        "bar_volume",
        "bar_notifications",
        "bar_conversations",
    ] {
        assert!(settings.top.iter().any(|button| button.command == command));
    }
    assert_eq!(settings.left, [button("preferences")]);
    assert_eq!(settings.appearance["inventory"].mode, Some(ButtonMode::TextOnly));
    let old = settings.top.clone();
    assert!(!settings.compose_top());
    assert_eq!(settings.top, old);
    settings.top.extend([button(structure::SEPARATOR), button(structure::SEPARATOR)]);
    settings.sanitize();
    assert_ne!(
        settings.top[settings.top.len() - 2].command,
        settings.top[settings.top.len() - 1].command
    );
}

#[test]
fn toolbar_moves_in_both_directions_without_duplicates_or_noop_saves() {
    let layout = layout();
    let mut settings = ToolbarSettings::default();
    let mut state = ToolbarUi::default();
    for (command, rank, changed, wanted) in [
        ("inventory", 1, false, vec!["inventory", "people", "preferences"]),
        ("inventory", 3, true, vec!["people", "preferences", "inventory"]),
        ("inventory", 0, true, vec!["inventory", "people", "preferences"]),
        ("friends", 1, true, vec!["inventory", "friends", "people", "preferences"]),
        ("friends", 2, false, vec!["inventory", "friends", "people", "preferences"]),
    ] {
        state.pending = Some(Edit::Insert(Bar::Bottom, button(command), rank));
        assert_eq!(state.commit(&layout, &mut settings), changed);
        assert_eq!(
            settings.buttons(&layout).iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
            wanted
        );
    }
}

#[test]
fn toolbar_persistence_preserves_custom_icons_empty_layout_and_skin_fallback() {
    let mut layout = layout();
    let mut settings = ToolbarSettings::default();
    let mut state = ToolbarUi {
        pending: Some(Edit::Remove("people".into())),
        ..Default::default()
    };
    assert!(state.commit(&layout, &mut settings));
    let json = serde_json::to_string(&settings).expect("serialize");
    settings = serde_json::from_str(&json).expect("round trip");
    assert_eq!(settings.buttons(&layout)[0].icon, "test-icon");
    state.pending = Some(Edit::Clear);
    assert!(state.commit(&layout, &mut settings));
    settings = serde_json::from_str(&serde_json::to_string(&settings).expect("serialize")).expect("round trip");
    assert!(settings.buttons(&layout).is_empty());
    state.pending = Some(Edit::Restore);
    assert!(state.commit(&layout, &mut settings));
    layout.toolbar.push(button("home"));
    assert_eq!(settings.buttons(&layout).len(), 4, "defaults follow skin hot reload");
    assert!(
        serde_json::from_str::<ToolbarSettings>("{}")
            .expect("old settings")
            .buttons
            .is_none()
    );
}

#[test]
fn toolbar_lock_blocks_all_layout_edits_and_catalogue_preserves_skin_icons() {
    let layout = layout();
    let mut settings = ToolbarSettings {
        locked: true,
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    for edit in [
        Edit::Clear,
        Edit::Restore,
        Edit::Remove("people".into()),
        Edit::Insert(Bar::Bottom, button("home"), 0),
        Edit::Insert(Bar::Left, button("home"), 0),
    ] {
        state.pending = Some(edit);
        assert!(!state.commit(&layout, &mut settings));
        assert!(settings.buttons.is_none());
    }
    let catalogue = catalogue(&layout);
    assert_eq!(catalogue.iter().filter(|b| b.command == "inventory").count(), 1);
    assert_eq!(
        catalogue.iter().find(|b| b.command == "inventory").expect("inventory").icon,
        "test-icon"
    );
    assert!(catalogue.windows(2).all(|w| sort_key(label(&w[0])) <= sort_key(label(&w[1]))));
    state.pending = Some(Edit::Lock);
    assert!(state.commit(&layout, &mut settings));
    assert!(!settings.locked);
}

#[test]
fn toolbar_insertion_uses_button_centres_and_scroll_offset() {
    let rects: Vec<_> = [-40.0, 10.0, 60.0]
        .into_iter()
        .map(|x| Rect::from_min_size(egui::pos2(x, 0.0), Vec2::new(40.0, 22.0)))
        .collect();
    let at = |x| egui::pos2(x, 0.0);
    assert_eq!(insertion(&rects, at(-25.0), false), 0);
    assert_eq!(insertion(&rects, at(25.0), false), 1);
    assert_eq!(insertion(&rects, at(110.0), false), 3);
    assert_eq!(insertion(&[], at(1.0), false), 0);
    // a side toolbar: by the buttons' vertical centres
    let column: Vec<_> = [0.0, 40.0]
        .into_iter()
        .map(|y| Rect::from_min_size(egui::pos2(0.0, y), Vec2::new(34.0, 34.0)))
        .collect();
    assert_eq!(insertion(&column, egui::pos2(500.0, 10.0), true), 0);
    assert_eq!(insertion(&column, egui::pos2(500.0, 30.0), true), 1);
    assert_eq!(insertion(&column, egui::pos2(500.0, 90.0), true), 2);
}

#[test]
fn toolbar_layout_and_lock_are_shared_across_accounts_after_restart() {
    let first = uuid::Uuid::from_u128(101);
    let second = uuid::Uuid::from_u128(102);
    let mut settings = crate::settings::Settings::default();
    assert!(settings.migrate_toolbar(first));
    let common = settings.toolbar.as_mut().expect("shared toolbar");
    common.buttons = Some(vec![button("inventory"), button("people")]);
    common.locked = true;
    let json = serde_json::to_string(&settings).expect("settings");
    let mut restored: crate::settings::Settings = serde_json::from_str(&json).expect("settings round trip");
    assert!(!restored.migrate_toolbar(second), "account switches keep the shared value");
    let common = restored.toolbar.as_ref().expect("shared toolbar");
    assert_eq!(
        common.buttons(&layout()).iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
        ["inventory", "people"]
    );
    assert!(common.locked);
    assert!(!restored.migrate_toolbar(first));
}

#[test]
fn water_mode_toolbar_buttons_are_opt_in_and_preserve_saved_placements() {
    let layout = Layout::default();
    let mut toolbar = ToolbarSettings {
        vr_button_migrated: true,
        director_button_migrated: true,
        ..Default::default()
    };
    let mut state = ToolbarUi::default();
    for command in ["movement", "water_walk"] {
        assert!(toolbar.location(command, &layout).is_none(), "absent from defaults: {command}");
        assert!(
            catalogue(&layout).iter().any(|b| b.command == command),
            "available to add: {command}"
        );
    }
    for (command, bar) in [("movement", Bar::Bottom), ("water_walk", Bar::Right)] {
        state.pending = Some(Edit::Insert(bar, button(command), 0));
        assert!(state.commit(&layout, &mut toolbar));
    }
    let mut saved = serde_json::to_value(crate::settings::Settings {
        toolbar: Some(toolbar),
        ..Default::default()
    })
    .expect("custom toolbar");
    saved["toolbar_common"]["water_walk_button_added"] = true.into();
    let mut restored: crate::settings::Settings = serde_json::from_value(saved).expect("retired upgrade flag is accepted");
    assert!(
        !restored.migrate_toolbar(uuid::Uuid::from_u128(101)),
        "login keeps custom placements"
    );
    let toolbar = restored.toolbar.as_mut().expect("common toolbar");
    assert_eq!(toolbar.location("movement", &layout), Some(Bar::Bottom));
    assert_eq!(toolbar.location("water_walk", &layout), Some(Bar::Right));
    state.pending = Some(Edit::Restore);
    assert!(state.commit(&layout, toolbar));
    for command in ["movement", "water_walk"] {
        assert!(
            toolbar.location(command, &layout).is_none(),
            "restore keeps water controls optional: {command}"
        );
    }
}

#[test]
fn toolbar_migration_uses_the_first_connected_accounts_layout_not_another_accounts() {
    let first = uuid::Uuid::from_u128(101);
    let second = uuid::Uuid::from_u128(102);
    let legacy = std::collections::HashMap::from([
        (
            first,
            ToolbarSettings {
                buttons: Some(vec![button("people")]),
                ..Default::default()
            },
        ),
        (
            second,
            ToolbarSettings {
                buttons: Some(vec![button("inventory")]),
                locked: true,
                ..Default::default()
            },
        ),
    ]);
    let old = serde_json::json!({ "toolbar": legacy });
    let settings: crate::settings::Settings = serde_json::from_value(old).expect("previous settings format");
    // A settings change at login must not lose the account's layout before it
    // enters the world and the migration knows which layout to adopt.
    let before_login = serde_json::to_string(&settings).expect("pre-login save");
    let mut restored: crate::settings::Settings = serde_json::from_str(&before_login).expect("pre-login restart");
    assert!(restored.migrate_toolbar(second));
    let common = restored.toolbar.as_ref().expect("migrated toolbar");
    assert_eq!(
        common.buttons(&layout()).iter().map(|b| b.command.as_str()).collect::<Vec<_>>(),
        if cfg!(any(target_os = "linux", target_os = "windows", target_os = "macos")) {
            vec!["inventory", "vr_view", "director"]
        } else {
            vec!["inventory", "director"]
        }
    );
    assert!(common.locked);
    assert!(!restored.migrate_toolbar(first));
    let saved = serde_json::to_value(&restored).expect("common settings");
    assert!(saved.get("toolbar").is_none(), "old account layouts are retired");
    assert!(saved["toolbar_common"]["locked"].as_bool().expect("lock"));
}

#[test]
fn toolbar_shared_empty_layout_wins_over_legacy_accounts_and_defaults_follow_the_skin() {
    let account = uuid::Uuid::from_u128(101);
    let legacy = std::collections::HashMap::from([(
        account,
        ToolbarSettings {
            buttons: Some(vec![button("inventory")]),
            ..Default::default()
        },
    )]);
    let saved = serde_json::json!({ "toolbar": legacy, "toolbar_common": { "buttons": [], "locked": true } });
    let mut settings: crate::settings::Settings = serde_json::from_value(saved).expect("mixed settings");
    assert!(settings.migrate_toolbar(account), "retire stale legacy entries");
    let common = settings.toolbar.as_ref().expect("common toolbar");
    assert!(common.buttons(&layout()).is_empty(), "empty is a user choice");
    assert!(common.locked);
    assert!(!settings.migrate_toolbar(account));

    let mut missing: crate::settings::Settings = serde_json::from_str("{}").expect("missing settings");
    assert!(missing.migrate_toolbar(account));
    assert_eq!(
        missing.toolbar.as_ref().expect("common defaults").buttons(&layout()),
        layout().toolbar
    );
}

#[test]
fn toolbar_custom_skin_labels_are_kept_and_built_in_labels_follow_the_language() {
    assert_eq!(label(&button("inventory")), "inventory", "skin's explicit custom label");
    let translated = ToolButton {
        tooltip: aurora_i18n::tr_in!("fr", "common-inventory").into(),
        ..button("inventory")
    };
    assert_eq!(label(&translated), aurora_i18n::tr!("common-inventory"));
}

#[test]
fn toolbar_catalogue_wraps_without_widening_the_floater() {
    let mut bench = Bench::default();
    bench.state.open = true;
    let tiles = catalogue_view::entries(&layout(), &bench.settings, &commands::Catalogue::default());
    for advanced in [false, true] {
        bench.state.advanced_catalogue = advanced;
        for _ in 0..4 {
            bench.frame(vec![]);
        }
        let window = bench
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("toolbar_editor")))
            .expect("editor");
        assert!(window.width() <= 900.0 - 16.0, "catalogue width: {window:?}");
        for tile in tiles.iter().filter(|tile| tile.in_view(advanced, "")) {
            let rect = bench
                .ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", &tile.button.command))))
                .expect("catalogue button");
            assert!(
                rect.left() >= window.left() && rect.right() <= window.right(),
                "button outside editor: {rect:?} / {window:?}"
            );
        }
    }
}

struct Bench {
    ctx: egui::Context,
    state: ToolbarUi,
    settings: ToolbarSettings,
    panels: Panels,
    saves: usize,
    action: bars::BarAction,
    /// The dock of the islands instead of Firestorm's bar.
    islands: bool,
    /// Our avatar sits, for the bars' Row.
    seated: bool,
}

#[test]
fn toolbar_editor_resizes_by_its_corner_reflows_and_keeps_its_size() {
    let mut bench = Bench::default();
    bench.state.open = true;
    for _ in 0..8 {
        bench.frame(vec![]);
    }
    let rect = |bench: &Bench| {
        bench
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::unique(editor::WINDOW)))
            .expect("editor")
    };
    let original = rect(&bench);
    let grip = original.right_bottom() - Vec2::splat(1.0);
    bench.frame(vec![egui::Event::PointerMoved(grip)]);
    bench.pointer(grip, true, false);
    let target = grip - Vec2::new(130.0, 100.0);
    bench.frame(vec![egui::Event::PointerMoved(target)]);
    bench.pointer(target, false, false);
    for _ in 0..8 {
        bench.frame(vec![]);
    }
    let resized = rect(&bench);
    assert!(resized.width() < original.width() - 80.0, "{original:?} -> {resized:?}");
    assert!(resized.height() < original.height() - 60.0, "{original:?} -> {resized:?}");
    assert!(editor::columns(resized.width() - 22.0) < editor::columns(original.width() - 22.0));
    bench.state.open = false;
    bench.frame(vec![]);
    bench.state.open = true;
    for _ in 0..8 {
        bench.frame(vec![]);
    }
    assert!((rect(&bench).width() - resized.width()).abs() < 1.0);
    assert!((rect(&bench).height() - resized.height()).abs() < 1.0);
}

#[test]
fn toolbar_editor_category_choice_filters_tiles_and_searches_within_the_category() {
    let mut bench = Bench::default();
    bench.state.open = true;
    for _ in 0..4 {
        bench.frame(vec![]);
    }
    let categories = bench.text(aurora_i18n::tr!("toolbar-category-all"));
    bench.pointer(categories, true, false);
    bench.pointer(categories, false, false);
    fn menu_text(shape: &egui::Shape, ctx: &egui::Context, wanted: &str) -> Option<egui::Pos2> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => {
                let pos = text.pos + Vec2::new(4.0, 7.0);
                ctx.layer_id_at(pos)
                    .is_some_and(|layer| layer.order == egui::Order::Foreground)
                    .then_some(pos)
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| menu_text(shape, ctx, wanted)),
            _ => None,
        }
    }
    bench.frame(vec![]);
    let output = bench.frame(vec![]);
    let world = output
        .shapes
        .iter()
        .find_map(|shape| menu_text(&shape.shape, &bench.ctx, catalogue_view::Category::World.label()))
        .expect("World category menu row");
    bench.pointer(world, true, false);
    bench.pointer(world, false, false);
    assert_eq!(bench.state.category, Some(catalogue_view::Category::World));
    bench.state.search = "aboutland".into();
    for _ in 0..3 {
        bench.frame(vec![]);
    }
    assert!(bench.ctx.read_response(egui::Id::unique(("toolbar_tile", "aboutland"))).is_some());
    assert!(bench.ctx.read_response(egui::Id::unique(("toolbar_tile", "inventory"))).is_none());
}

impl Default for Bench {
    fn default() -> Self {
        let settings = crate::settings::Settings {
            show_people: false,
            show_inventory: false,
            ..Default::default()
        };
        Self {
            ctx: Default::default(),
            state: Default::default(),
            settings: Default::default(),
            panels: Panels::from_settings(&settings),
            saves: 0,
            action: bars::BarAction::None,
            islands: false,
            seated: false,
        }
    }
}

impl Bench {
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        // the dock is an egui area: shown from its second frame on
        if self.islands && self.ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) == 0 {
            self.frame_once(Vec::new());
        }
        self.frame_once(events)
    }

    fn frame_once(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        let p = crate::theme::Theme::default().palette();
        let layout = layout();
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(900.0, 650.0))),
                events,
                ..Default::default()
            },
            |ui| {
                self.state.prepare(&self.ctx, &self.settings);
                bars::top::reserve(
                    ui,
                    &self.settings,
                    Rect::from_min_size(egui::pos2(8.0, 8.0), Vec2::new(884.0, 36.0)),
                    80.0,
                );
                let (_, action, _, _) = bars::bottom_bar(
                    ui,
                    &p,
                    &Icons::default(),
                    &layout,
                    &mut self.state,
                    &mut self.settings,
                    &mut self.panels,
                    &mut ChatUi::default(),
                    &crate::gestures::Manager::default(),
                    0,
                    None,
                    false,
                    self.seated,
                    false,
                    &mut 0.0,
                    &mut bars::MicButton::default(),
                    false,
                    self.islands,
                    None,
                    &bars::favorites::View::default(),
                );
                self.action = action;
                let favorites = bars::favorites::View::default();
                let row = bars::Row {
                    unread: 0,
                    latest_unread: None,
                    flash: 0.0,
                    flying: false,
                    seated: self.seated,
                    mouselook: false,
                    ao_active: false,
                    locked: self.settings.locked,
                    favorites: &favorites,
                    bar: Bar::Bottom,
                    style: Style::default(),
                    editing: self.state.open,
                    floating: false,
                };
                bars::top::show(
                    ui,
                    &p,
                    &Icons::default(),
                    &layout,
                    &mut self.state,
                    &self.settings,
                    &mut self.panels,
                    &mut bars::MicButton::default(),
                    &row,
                    !self.islands,
                );
                bars::side::bars(
                    ui,
                    &p,
                    &Icons::default(),
                    &layout,
                    &mut self.state,
                    &self.settings,
                    &mut self.panels,
                    &mut bars::MicButton::default(),
                    &row,
                    Rect::from_min_max(egui::pos2(0.0, 20.0), egui::pos2(900.0, 600.0)),
                    self.islands,
                    [true, true],
                );
                self.state.show(ui.ctx(), &p, &layout, &self.settings, None);
                if self.state.commit(&layout, &mut self.settings) {
                    self.saves += 1;
                }
            },
        );
        output.textures_delta.clear();
        output
    }

    /// The window's tile of `command`, filtered to the top of the grid.
    fn tile_rect(&mut self, command: &str) -> Rect {
        self.state.search = command.into();
        self.frame(vec![]);
        self.frame(vec![]);
        self.ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", command))))
            .expect("catalogue tile")
    }

    fn tile(&mut self, command: &str) -> egui::Pos2 {
        self.tile_rect(command).center()
    }

    fn pos(&self, command: &str) -> egui::Pos2 {
        self.ctx
            .read_response(bars::tool_button_id(command))
            .expect("real button")
            .rect
            .center()
    }

    fn pointer(&mut self, pos: egui::Pos2, pressed: bool, secondary: bool) {
        self.frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: if secondary {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                },
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }

    fn text(&mut self, wanted: &str) -> egui::Pos2 {
        fn find(shape: &egui::Shape, wanted: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == wanted => Some(t.pos + Vec2::new(4.0, 7.0)),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, wanted)),
                _ => None,
            }
        }
        self.frame(vec![]);
        self.frame(vec![])
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, wanted))
            .expect("visible text")
    }

    fn context_text(&mut self, command: &str, wanted: &str) -> egui::Pos2 {
        let layer = egui::LayerId::new(egui::Order::Foreground, bars::tool_button_id(command).with("context_menu"));
        // The inspector repeats labels behind the popup. Reverse paint order
        // avoids mistaking that obscured text for a row of the menu above it.
        fn find(shape: &egui::Shape, wanted: &str, ctx: &egui::Context, layer: egui::LayerId) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == wanted => {
                    let pos = text.pos + Vec2::new(4.0, 7.0);
                    (ctx.layer_id_at(pos) == Some(layer)).then_some(pos)
                }
                egui::Shape::Vec(shapes) => shapes.iter().rev().find_map(|shape| find(shape, wanted, ctx, layer)),
                _ => None,
            }
        }
        self.frame(vec![]);
        self.frame(vec![])
            .shapes
            .iter()
            .rev()
            .find_map(|shape| find(&shape.shape, wanted, &self.ctx, layer))
            .expect("visible context-menu row")
    }
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_real_right_click_removes_one_button_in_customization_without_opening_its_window() {
    for islands in [false, true] {
        toolbar_real_right_click_removes_one_button_without_opening_its_window_in(islands);
    }
}

#[test]
fn favorites_left_grid_and_normal_customization_menu_stay_separate_on_every_bar() {
    for islands in [false, true] {
        for bar in Bar::ALL {
            let mut bench = Bench {
                islands,
                ..Bench::default()
            };
            bench.settings.buttons = Some(Vec::new());
            match bar {
                Bar::Bottom => bench.settings.buttons = Some(vec![button("favorites")]),
                Bar::Left => bench.settings.left = vec![button("favorites")],
                Bar::Right => bench.settings.right = vec![button("favorites")],
                Bar::Top => bench.settings.top = vec![button("favorites")],
            }
            bench.frame(vec![]);
            bench.frame(vec![]);
            let before = bench.settings.clone();
            let saves = bench.saves;
            let pos = bench.pos("favorites");
            bench.pointer(pos, true, false);
            bench.pointer(pos, false, false);
            bench.frame(vec![]);
            assert!(
                bars::favorites::take_shown(&bench.ctx),
                "left click opens Favorites: {bar:?}, islands={islands}"
            );
            bench.pointer(pos, true, true);
            bench.pointer(pos, false, true);
            bars::favorites::take_shown(&bench.ctx);
            bench.frame(vec![]);
            assert!(
                !bars::favorites::take_shown(&bench.ctx),
                "right click only opens the toolbar menu: {bar:?}, islands={islands}"
            );
            let appearance = bench.text(aurora_i18n::tr!("toolbar-button-inherit"));
            let customize = bench.text(aurora_i18n::tr!("toolbar-interface-edit"));
            assert!(
                appearance.y < customize.y,
                "appearance precedes customization: {bar:?}, islands={islands}"
            );
            // the block's settings between (human request, 2026-11-25)
            let floating = bench.text(aurora_i18n::tr!("toolbar-block-kind-floating"));
            let edit = bench.text(aurora_i18n::tr!("toolbar-block-edit"));
            assert!(appearance.y < floating.y && floating.y < edit.y && edit.y < customize.y);
            assert!(customize.y - edit.y < 40.0, "shared customization group has no internal separator");
            bench.pointer(customize, true, false);
            bench.pointer(customize, false, false);
            assert!(bench.state.open);
            assert_eq!(bench.state.selected.as_deref(), Some("favorites"));
            assert_eq!(bench.settings, before, "entering customization preserves the layout");
            assert_eq!(bench.saves, saves);
        }
    }
}

fn toolbar_real_right_click_removes_one_button_without_opening_its_window_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.state.open = true;
    bench.frame(vec![]);
    let pos = bench.pos("people");
    bench.pointer(pos, true, true);
    bench.pointer(pos, false, true);
    let remove = bench.context_text("people", aurora_i18n::tr!("toolbar-remove"));
    bench.pointer(remove, true, false);
    bench.pointer(remove, false, false);
    assert_eq!(bench.settings.buttons(&layout()).len(), 2);
    assert!(!bench.panels.people);
    assert_eq!(bench.saves, 1);
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_real_drag_commits_once_on_release_and_escape_cancels() {
    for islands in [false, true] {
        toolbar_real_drag_commits_once_on_release_and_escape_cancels_in(islands);
    }
}

fn toolbar_real_drag_commits_once_on_release_and_escape_cancels_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.frame(vec![]);
    let from = bench.pos("inventory");
    let target = bench.ctx.read_response(bars::tool_button_id("preferences")).expect("target").rect;
    let to = egui::pos2(target.right() + 1.0, target.center().y);
    bench.pointer(from, true, false);
    bench.frame(vec![egui::Event::PointerMoved(to)]);
    bench.frame(vec![egui::Event::PointerMoved(to)]);
    assert_eq!(bench.saves, 0);
    assert!(egui::DragAndDrop::has_payload_of_type::<Drag>(&bench.ctx));
    bench.pointer(to, false, false);
    assert_eq!(bench.settings.buttons(&layout()).last().expect("last").command, "inventory");
    assert_eq!(bench.saves, 1);
    assert!(!bench.panels.inventory);
    let from = bench.pos("inventory");
    bench.pointer(from, true, false);
    bench.frame(vec![egui::Event::PointerMoved(from - Vec2::new(80.0, 0.0))]);
    bench.frame(vec![egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    }]);
    bench.pointer(bench.pos("people"), false, false);
    assert_eq!(bench.saves, 1);
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_catalogue_click_and_real_drop_into_empty_bar_add_single_buttons() {
    for islands in [false, true] {
        toolbar_catalogue_click_and_real_drop_into_empty_bar_add_single_buttons_in(islands);
    }
}

fn toolbar_catalogue_click_and_real_drop_into_empty_bar_add_single_buttons_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.state.open = true;
    bench.settings.buttons = Some(vec![]);
    bench.frame(vec![]);
    bench.frame(vec![]);
    let source = bench.tile("friends");
    bench.pointer(source, true, false);
    bench.frame(vec![egui::Event::PointerMoved(source + Vec2::new(0.0, 30.0))]);
    bench.frame(vec![]);
    let target = bench
        .ctx
        .read_response(egui::Id::unique("toolbar_background"))
        .expect("empty bar")
        .rect
        .center();
    bench.frame(vec![egui::Event::PointerMoved(target)]);
    bench.frame(vec![egui::Event::PointerMoved(target)]);
    bench.pointer(target, false, false);
    assert_eq!(
        bench
            .settings
            .buttons(&layout())
            .iter()
            .map(|b| b.command.as_str())
            .collect::<Vec<_>>(),
        ["friends"]
    );
    bench.frame(vec![]);
    let appearance = bench.tile("appearance");
    bench.pointer(appearance, true, false);
    bench.pointer(appearance, false, false);
    assert_eq!(bench.settings.buttons(&layout()).len(), 2);
    assert_eq!(bench.saves, 2);
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_background_opens_editor_and_locked_buttons_stay_in_place() {
    for islands in [false, true] {
        toolbar_background_opens_editor_and_locked_buttons_stay_in_place_in(islands);
    }
}

#[test]
fn toolbar_empty_dock_hides_after_last_button_moves_to_either_side_and_accepts_it_back() {
    for bar in [Bar::Left, Bar::Right] {
        let mut bench = Bench {
            islands: true,
            settings: ToolbarSettings {
                buttons: Some(vec![button("inventory")]),
                ..Default::default()
            },
            ..Default::default()
        };
        let background = egui::Id::unique("toolbar_background");
        bench.frame(vec![]);
        let from = bench.pos("inventory");
        bench.pointer(from, true, false);
        bench.frame(vec![egui::Event::PointerMoved(from - Vec2::new(0.0, 40.0))]);
        bench.frame(vec![]);
        let side = if bar == Bar::Left { "toolbar_left" } else { "toolbar_right" };
        let target = bench
            .ctx
            .read_response(egui::Id::unique(("toolbar_background", side)))
            .expect("empty side drop zone during a toolbar drag")
            .rect
            .center();
        bench.frame(vec![egui::Event::PointerMoved(target)]);
        bench.pointer(target, false, false);
        assert!(bench.settings.buttons(&layout()).is_empty());
        assert_eq!(commands(bench.settings.bar(bar, &layout())), ["inventory"]);
        for _ in 0..3 {
            bench.frame(vec![]);
        }
        assert!(bench.ctx.read_response(background).is_none(), "empty dock disappears after release");
        assert!(
            bench.ctx.read_response(crate::ui::chat::bar_input_id()).is_some(),
            "nearby chat stays available"
        );
        bench.state.open = true;
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(
            bench.ctx.read_response(background).is_some(),
            "the visual editor exposes an empty dock destination"
        );
        bench.state.open = false;
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(
            bench.ctx.read_response(background).is_none(),
            "closing the editor hides an unused dock"
        );
        let from = bench.pos("inventory");
        bench.pointer(from, true, false);
        bench.frame(vec![egui::Event::PointerMoved(from + Vec2::new(0.0, 40.0))]);
        bench.frame(vec![]);
        let target = bench
            .ctx
            .read_response(background)
            .expect("empty dock returns during a toolbar drag")
            .rect
            .center();
        bench.frame(vec![egui::Event::PointerMoved(target)]);
        bench.pointer(target, false, false);
        assert_eq!(commands(bench.settings.buttons(&layout())), ["inventory"]);
        assert!(bench.settings.bar(bar, &layout()).is_empty());
        bench.frame(vec![]);
        bench.frame(vec![]);
        assert!(bench.ctx.read_response(background).is_some(), "populated dock stays visible");
        assert_eq!(bench.saves, 2);
    }
}

fn toolbar_background_opens_editor_and_locked_buttons_stay_in_place_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    if !islands {
        bench.settings.buttons = Some(vec![]);
    }
    bench.frame(vec![]);
    let background = bench
        .ctx
        .read_response(egui::Id::unique("toolbar_background"))
        .expect("toolbar background")
        .rect
        .center();
    bench.pointer(background, true, true);
    bench.pointer(background, false, true);
    let edit = bench.text(aurora_i18n::tr!("toolbar-interface-edit"));
    bench.pointer(edit, true, false);
    bench.pointer(edit, false, false);
    assert!(bench.state.open);
    assert_eq!(bench.saves, 0);
    bench.state.open = false;
    bench.settings.buttons = None;
    bench.settings.locked = true;
    bench.frame(vec![]);
    let from = bench.pos("inventory");
    let to = bench.pos("preferences");
    bench.pointer(from, true, false);
    bench.frame(vec![egui::Event::PointerMoved(to)]);
    bench.pointer(to, false, false);
    assert_eq!(bench.saves, 0);
    assert!(!egui::DragAndDrop::has_payload_of_type::<Drag>(&bench.ctx));
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_drop_outside_the_viewport_cancels_and_drop_back_in_catalogue_removes() {
    for islands in [false, true] {
        toolbar_drop_outside_cancels_and_drop_back_in_catalogue_removes_in(islands);
    }
}

fn toolbar_drop_outside_cancels_and_drop_back_in_catalogue_removes_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.frame(vec![]);
    let from = bench.pos("inventory");
    let outside = egui::pos2(-20.0, 100.0);
    bench.pointer(from, true, false);
    bench.frame(vec![egui::Event::PointerMoved(outside)]);
    bench.pointer(outside, false, false);
    assert_eq!(bench.saves, 0);
    assert!(!egui::DragAndDrop::has_payload_of_type::<Drag>(&bench.ctx));
    bench.state.open = true;
    bench.frame(vec![]);
    bench.frame(vec![]);
    let target = bench.tile("friends");
    bench.pointer(from, true, false);
    bench.frame(vec![egui::Event::PointerMoved(target)]);
    bench.frame(vec![egui::Event::PointerMoved(target)]);
    bench.pointer(target, false, false);
    assert_eq!(bench.saves, 1);
    assert!(!bench.settings.buttons(&layout()).iter().any(|b| b.command == "inventory"));
}

fn commands(list: &[ToolButton]) -> Vec<&str> {
    list.iter().map(|b| b.command.as_str()).collect()
}

/// LLToolBarView keeps a command in one toolbar: dropped in another it
/// moves there; removed, it leaves the one it was in; Clear all empties
/// the three, Restore defaults brings the skin's bottom toolbar back with
/// empty side ones and the looks' defaults (loadDefaultToolbars).
#[test]
fn toolbar_commands_stand_in_one_of_three_toolbars() {
    let layout = layout();
    let mut settings = ToolbarSettings::default();
    let mut state = ToolbarUi {
        pending: Some(Edit::Insert(Bar::Left, button("people"), 0)),
        ..Default::default()
    };
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(commands(settings.buttons(&layout)), ["inventory", "preferences"]);
    assert_eq!(commands(&settings.left), ["people"]);
    assert_eq!(settings.location("people", &layout), Some(Bar::Left));
    state.pending = Some(Edit::Insert(Bar::Left, button("radar"), 0));
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(commands(&settings.left), ["radar", "people"]);
    // reordered within the left toolbar
    state.pending = Some(Edit::Insert(Bar::Left, button("radar"), 2));
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(commands(&settings.left), ["people", "radar"]);
    state.pending = Some(Edit::Insert(Bar::Right, button("people"), 0));
    assert!(state.commit(&layout, &mut settings));
    assert_eq!(commands(&settings.left), ["radar"]);
    assert_eq!(commands(&settings.right), ["people"]);
    state.pending = Some(Edit::Remove("people".into()));
    assert!(state.commit(&layout, &mut settings));
    assert!(settings.right.is_empty());
    assert_eq!(settings.location("people", &layout), None);
    // an untouched bottom toolbar follows the skin
    let mut fresh = ToolbarSettings::default();
    state.pending = Some(Edit::Insert(Bar::Right, button("radar"), 0));
    assert!(state.commit(&layout, &mut fresh));
    assert!(fresh.buttons.is_none());
    state.pending = Some(Edit::Clear);
    assert!(state.commit(&layout, &mut settings));
    assert!(settings.buttons(&layout).is_empty() && settings.left.is_empty() && settings.right.is_empty());
    settings.left_style.mode = Some(ButtonMode::TextOnly);
    settings.left.push(button("radar"));
    state.pending = Some(Edit::Restore);
    assert!(state.commit(&layout, &mut settings));
    assert!(settings.buttons.is_none() && settings.left.is_empty());
    assert_eq!(settings.left_style, BarStyle::default());
    state.pending = Some(Edit::Restore);
    assert!(!state.commit(&layout, &mut settings), "nothing to restore");
}

/// Each toolbar's mode, alignment and size are saved apart; one left at
/// its default follows each look's (the human's decision of 2026-10-15);
/// Firestorm keeps these menus while the toolbars are locked.
#[test]
fn toolbar_styles_follow_each_look_until_chosen_and_survive_a_restart() {
    let layout = layout();
    let style = BarStyle::default();
    assert_eq!(
        style.resolve(Bar::Bottom, true),
        Style {
            mode: ButtonMode::IconsOnly,
            align: Align::Center,
            sizing: Sizing::Fixed
        }
    );
    assert_eq!(style.resolve(Bar::Bottom, false).sizing, Sizing::Fill);
    assert_eq!(style.resolve(Bar::Left, false).align, Align::Center);
    let mut settings = ToolbarSettings {
        locked: true,
        ..Default::default()
    };
    let mut state = ToolbarUi {
        pending: Some(Edit::Style(
            Bar::Bottom,
            BarStyle {
                mode: Some(ButtonMode::IconsWithText),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    assert!(state.commit(&layout, &mut settings), "styles change while locked");
    state.pending = Some(Edit::Style(
        Bar::Bottom,
        BarStyle {
            align: Some(Align::End),
            ..Default::default()
        },
    ));
    assert!(state.commit(&layout, &mut settings));
    let both = settings.style(Bar::Bottom);
    assert_eq!(both.mode, Some(ButtonMode::IconsWithText));
    assert_eq!(both.align, Some(Align::End));
    assert_eq!(both.resolve(Bar::Bottom, false).sizing, Sizing::Fill, "unset: the look's own");
    assert_eq!(both.resolve(Bar::Bottom, true).sizing, Sizing::Fixed);
    let json = serde_json::to_string(&settings).expect("serialize");
    let restored: ToolbarSettings = serde_json::from_str(&json).expect("round trip");
    assert_eq!(restored.style(Bar::Bottom), both);
    assert_eq!(restored.style(Bar::Left), BarStyle::default());
    // a file of #366 keeps its bottom toolbar and gets empty side ones
    let old: ToolbarSettings =
        serde_json::from_str(r#"{"buttons":[{"icon":"x","command":"people","tooltip":""}],"locked":false}"#).expect("old file");
    assert_eq!(commands(old.buttons(&layout)), ["people"]);
    assert!(old.left.is_empty() && old.right.is_empty());
    assert_eq!(old.style(Bar::Right), BarStyle::default());
}

/// A file edited by hand with a command twice keeps its first place.
#[test]
fn toolbar_sanitize_keeps_each_command_once_over_the_three_toolbars() {
    let mut settings = ToolbarSettings {
        buttons: Some(vec![button("people"), button("people"), button("inventory")]),
        left: vec![button("inventory"), button("radar"), button("")],
        right: vec![button("radar"), button("search")],
        ..Default::default()
    };
    settings.sanitize();
    let layout = layout();
    assert_eq!(commands(settings.buttons(&layout)), ["people", "inventory"]);
    assert_eq!(commands(&settings.left), ["radar"]);
    assert_eq!(commands(&settings.right), ["search"]);
}

/// A saved or imported layout naming `howto` (dropped by the Legacy Viewer)
/// loads without it and keeps the rest.
#[test]
fn toolbar_sanitize_skips_the_retired_howto_command() {
    let mut settings = ToolbarSettings {
        buttons: Some(vec![button("people"), button("howto"), button("inventory")]),
        left: vec![button("howto"), button("radar")],
        right: vec![button("search"), button("howto")],
        ..Default::default()
    };
    settings.sanitize();
    let layout = layout();
    assert_eq!(commands(settings.buttons(&layout)), ["people", "inventory"]);
    assert_eq!(commands(&settings.left), ["radar"]);
    assert_eq!(commands(&settings.right), ["search"]);
    assert!(catalogue::find("howto").is_none());
}

/// The window lists the Legacy Viewer commands and Aurora additions, each
/// once, with an embedded icon and localized labels;
/// what Aurora runs does something, what it cannot run yet does nothing.
#[test]
fn toolbar_catalogue_holds_firestorm_commands_and_runs_the_ready_ones() {
    let skin = Layout::default();
    let all = catalogue(&skin);
    // every command but bar_account and bar_system, which stay out of the catalogue
    assert_eq!(all.len(), 129);
    assert!(
        !all.iter()
            .any(|button| matches!(button.command.as_str(), "bar_account" | "bar_system"))
    );
    assert_eq!(COMMANDS.len(), 131);
    let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), COMMANDS.len(), "each command once");
    assert!(ready("texture_library"));
    assert!(ready("movelock") && ready("move_lock") && ready("movement") && ready("water_walk"));
    assert_eq!(catalogue::find("move_lock").map(|c| c.id), Some("movelock"));
    assert_eq!(all.iter().filter(|b| b.command == "movelock").count(), 1);
    assert!(!all.iter().any(|b| b.command == "move_lock"));
    let ctx = egui::Context::default();
    for command in COMMANDS {
        assert!(
            command.icon == "AO" || crate::ui::icons::embedded(command.icon),
            "{}: icon {} is not embedded",
            command.id,
            command.icon
        );
        for (language, _) in aurora_i18n::available_languages() {
            assert_ne!(command.label_in(language), command.key, "{}: no label in {language}", command.id);
        }
        // Speak, Favorites and the AO's toggle have buttons of their own;
        // "Floating block" becomes a block where it is dropped
        if matches!(command.id, "speak" | "favorites" | "aurora_menu" | "vr_view" | drawers::FLOATING)
            || structure::is_separator(command.id)
            || structure::is_divider(command.id)
            || structure::is_block(command.id)
        {
            continue;
        }
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let before = format!("{panels:?}");
        let action = bars::run_command(&ctx, command.id, &mut panels);
        let did = !matches!(action, bars::BarAction::None) || format!("{panels:?}") != before;
        assert_eq!(did, command.ready, "{}", command.id);
    }
    assert!(!ready("flickr"));
    assert!(ready("a_skin_command"));
}

#[test]
fn a_placed_catalogue_tile_can_move_to_the_top_without_duplicating_its_button() {
    for locked in [false, true] {
        let mut bench = Bench {
            islands: true,
            ..Default::default()
        };
        bench.state.open = true;
        bench.settings.locked = locked;
        let source = bench.tile("inventory");
        assert_eq!(bench.settings.location("inventory", &layout()), Some(Bar::Bottom));
        bench.pointer(source, true, false);
        bench.frame(vec![egui::Event::PointerMoved(source + Vec2::new(0.0, 30.0))]);
        assert_eq!(ToolbarUi::dragging(&bench.ctx), !locked);
        if locked {
            continue;
        }
        bench.frame(vec![]);
        let zone = bench
            .ctx
            .read_response(egui::Id::unique("top_toolbar_drop"))
            .expect("top drop zone")
            .rect;
        let target = egui::pos2(zone.right() - 8.0, zone.center().y);
        for _ in 0..3 {
            bench.frame(vec![egui::Event::PointerMoved(target)]);
        }
        assert_eq!(bench.saves, 0);
        bench.pointer(target, false, false);
        assert_eq!(bench.saves, 1);
        assert_eq!(bench.settings.location("inventory", &layout()), Some(Bar::Top));
        assert_eq!(
            Bar::ALL
                .iter()
                .flat_map(|&bar| bench.settings.bar(bar, &layout()).to_vec())
                .filter(|button| button.command == "inventory")
                .count(),
            1
        );
    }
}

/// A saved button of a command Aurora does not have yet stays in its
/// toolbar, dimmed by its tooltip, and its click does nothing.
#[test]
fn toolbar_window_tiles_show_placed_and_missing_commands() {
    let mut bench = Bench::default();
    bench.state.open = true;
    bench.settings.left = vec![button("radar")];
    for _ in 0..4 {
        bench.frame(vec![]);
    }
    let radar = bench.tile_rect("radar");
    // the placed tile cannot be added again
    bench.pointer(radar.center(), true, false);
    bench.pointer(radar.center(), false, false);
    assert_eq!(bench.saves, 0);
    let flickr = bench.tile_rect("flickr");
    bench.pointer(flickr.center(), true, false);
    bench.pointer(flickr.center(), false, false);
    assert_eq!(bench.saves, 0, "not in Aurora yet");
    let build = bench.tile_rect("build");
    bench.pointer(build.center(), true, false);
    bench.pointer(build.center(), false, false);
    assert_eq!(bench.saves, 1);
    assert_eq!(bench.settings.location("build", &layout()), Some(Bar::Bottom));
}

/// Clear all asks first (ConfirmClearAllToolbars): Cancel keeps the three
/// toolbars, OK empties them in one save.
#[test]
fn toolbar_clear_all_asks_before_emptying_the_three_toolbars() {
    let mut bench = Bench::default();
    bench.state.open = true;
    bench.settings.left = vec![button("radar")];
    for _ in 0..3 {
        bench.frame(vec![]);
    }
    let click = |bench: &mut Bench, text: &str| {
        let at = bench.text(text);
        bench.pointer(at, true, false);
        bench.pointer(at, false, false);
    };
    click(&mut bench, aurora_i18n::tr!("toolbar-clear"));
    click(&mut bench, aurora_i18n::tr!("common-cancel"));
    assert_eq!(bench.saves, 0);
    assert_eq!(bench.settings.buttons(&layout()).len(), 3);
    click(&mut bench, aurora_i18n::tr!("toolbar-clear"));
    click(&mut bench, aurora_i18n::tr!("common-ok"));
    assert_eq!(bench.saves, 1);
    assert!(bench.settings.buttons(&layout()).is_empty() && bench.settings.left.is_empty());
}

/// A button dragged from the bottom toolbar onto the window goes back to it.
#[test]
fn toolbar_drop_anywhere_on_the_window_removes_the_button() {
    for islands in [false, true] {
        let mut bench = Bench {
            islands,
            ..Bench::default()
        };
        bench.state.open = true;
        bench.frame(vec![]);
        bench.frame(vec![]);
        let window = bench
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique("toolbar_editor")))
            .expect("editor");
        let from = bench.pos("people");
        let to = egui::pos2(window.center().x, window.bottom() - 12.0);
        bench.pointer(from, true, false);
        bench.frame(vec![egui::Event::PointerMoved(to)]);
        bench.frame(vec![egui::Event::PointerMoved(to)]);
        bench.pointer(to, false, false);
        assert_eq!(bench.saves, 1, "islands {islands}");
        assert_eq!(bench.settings.location("people", &layout()), None);
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
#[test]
fn vr_toolbar_migration_preserves_customization_and_manual_removal() {
    let mut settings = ToolbarSettings {
        buttons: Some(vec![button("inventory"), button("view"), button("preferences")]),
        ..Default::default()
    };
    assert!(settings.migrate_vr_button());
    let commands: Vec<_> = settings
        .buttons
        .as_ref()
        .expect("custom toolbar")
        .iter()
        .map(|b| b.command.as_str())
        .collect();
    assert_eq!(commands, ["inventory", "view", "vr_view", "preferences"]);
    settings
        .buttons
        .as_mut()
        .expect("custom toolbar")
        .retain(|b| b.command != "vr_view");
    assert!(!settings.migrate_vr_button());
    assert!(
        !settings
            .buttons
            .as_ref()
            .expect("custom toolbar")
            .iter()
            .any(|b| b.command == "vr_view")
    );
    let mut empty = ToolbarSettings {
        buttons: Some(vec![]),
        ..Default::default()
    };
    empty.migrate_vr_button();
    assert!(empty.buttons.expect("explicit empty toolbar").is_empty());
    let mut side = ToolbarSettings {
        buttons: Some(vec![button("view")]),
        left: vec![button("vr_view")],
        ..Default::default()
    };
    side.migrate_vr_button();
    assert_eq!(side.buttons.expect("custom toolbar").len(), 1);
}

#[test]
fn menu_toolbar_edits_recheck_lock_membership_and_preserve_other_pending_edits() {
    let ctx = egui::Context::default();
    let layout = layout();
    let mut state = ToolbarUi::default();
    let mut settings = ToolbarSettings::default();
    let queue = |remove| {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<MenuEdit>>(egui::Id::unique("menu_toolbar_edits"))
                .push(MenuEdit { command: "import", remove })
        })
    };
    queue(false);
    settings.locked = true;
    assert!(!state.commit_with_menus(&ctx, &layout, &mut settings));
    assert!(settings.location("import", &layout).is_none());
    settings.locked = false;
    state.pending = Some(Edit::Remove("inventory".into()));
    queue(false);
    assert!(state.commit_with_menus(&ctx, &layout, &mut settings));
    assert!(settings.location("inventory", &layout).is_none());
    assert_eq!(settings.bar(Bar::Bottom, &layout).last().unwrap().command, "import");
    queue(false);
    assert!(
        !state.commit_with_menus(&ctx, &layout, &mut settings),
        "stale Add does not move an existing button"
    );
    state.pending = Some(Edit::Insert(
        Bar::Right,
        settings.bar(Bar::Bottom, &layout).last().unwrap().clone(),
        0,
    ));
    state.commit_with_menus(&ctx, &layout, &mut settings);
    queue(true);
    assert!(state.commit_with_menus(&ctx, &layout, &mut settings));
    assert!(settings.location("import", &layout).is_none(), "menu removal finds side bars too");
}

#[test]
fn menu_toolbar_added_import_button_runs_import_in_both_bar_layouts() {
    for islands in [false, true] {
        let mut bench = Bench {
            islands,
            ..Bench::default()
        };
        bench.ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::unique("menu_toolbar_edits"),
                vec![MenuEdit {
                    command: "import",
                    remove: false,
                }],
            )
        });
        assert!(bench.state.commit_with_menus(&bench.ctx, &layout(), &mut bench.settings));
        bench.frame(vec![]);
        let pos = bench.pos("import");
        bench.pointer(pos, true, false);
        bench.pointer(pos, false, false);
        assert!(matches!(bench.action, bars::BarAction::Import));
    }
}

/// Queues a menu's Add of each command and applies it: their buttons go to
/// the end of the bottom toolbar.
fn add_from_menu(bench: &mut Bench, commands: &[&'static str]) {
    bench.ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::unique("menu_toolbar_edits"),
            commands
                .iter()
                .map(|&command| MenuEdit { command, remove: false })
                .collect::<Vec<_>>(),
        )
    });
    assert!(bench.state.commit_with_menus(&bench.ctx, &layout(), &mut bench.settings));
}

/// Clicks the toolbar button of `command` and gives the bottom bar's action.
fn click(bench: &mut Bench, command: &str) -> bars::BarAction {
    let pos = bench.pos(command);
    bench.pointer(pos, true, false);
    bench.pointer(pos, false, false);
    std::mem::replace(&mut bench.action, bars::BarAction::None)
}

/// The button of `command` has the violet face of a lit button (hovered
/// or not) in `output`.
fn lit(bench: &Bench, output: &egui::FullOutput, command: &str) -> bool {
    fn face(shape: &egui::Shape, rect: Rect, faces: &[egui::Color32]) -> bool {
        match shape {
            egui::Shape::Rect(r) => r.rect == rect && faces.contains(&r.fill),
            egui::Shape::Vec(v) => v.iter().any(|s| face(s, rect, faces)),
            _ => false,
        }
    }
    let p = crate::theme::Theme::default().palette();
    let faces = [p.violet.gamma_multiply(0.40), p.violet.gamma_multiply(0.55)];
    let rect = bench.ctx.read_response(bars::tool_button_id(command)).expect("real button").rect;
    output.shapes.iter().any(|s| face(&s.shape, rect, &faces))
}

/// The bench draws the bottom and side toolbars and never the top bar, as
/// with the interface hidden or chatting in mouselook: the buttons still
/// follow the selection, Fly override and Quickjump given by the app.
#[test]
fn menu_toolbar_buttons_follow_the_app_state_without_the_top_bar() {
    let mut bench = Bench::default();
    add_from_menu(&mut bench, &["edit_particles", "fly_override", "quickjump"]);
    bench.frame(vec![]);
    assert!(matches!(click(&mut bench, "edit_particles"), bars::BarAction::None));
    let output = bench.frame(vec![]);
    assert!(!lit(&bench, &output, "fly_override") && !lit(&bench, &output, "quickjump"));
    bench.state.edit_particles = true;
    bench.state.fly_override_active = true;
    bench.frame(vec![]);
    assert!(matches!(click(&mut bench, "edit_particles"), bars::BarAction::EditParticles));
    let output = bench.frame(vec![]);
    assert!(lit(&bench, &output, "fly_override"), "Fly override lights while on");
    assert!(!lit(&bench, &output, "quickjump"));
    bench.state.fly_override_active = false;
    bench.state.quickjump_active = true;
    let output = bench.frame(vec![]);
    assert!(!lit(&bench, &output, "fly_override"), "a toggled Fly override goes dark");
    assert!(lit(&bench, &output, "quickjump"), "Quickjump lights while on");
}

/// enable_standup_self (indra/newview/llviewermenu.cpp): the Stand up
/// button does nothing while standing, and stands while seated.
#[test]
fn menu_toolbar_stand_up_button_is_greyed_while_standing() {
    let mut bench = Bench::default();
    add_from_menu(&mut bench, &["stand_up"]);
    bench.frame(vec![]);
    assert!(matches!(click(&mut bench, "stand_up"), bars::BarAction::None));
    bench.seated = true;
    bench.frame(vec![]);
    assert!(matches!(click(&mut bench, "stand_up"), bars::BarAction::StandUp));
}

/// The Build modes' buttons carry short labels, "Build - " and the build
/// window's tool, like every other button; the Build menu names them without
/// the prefix and shows their Ctrl+3/4/5 in its keys column.
#[test]
fn menu_toolbar_build_buttons_have_short_labels() {
    for (id, label) in [
        ("build_focus", "Build - Camera"),
        ("build_grab", "Build - Move"),
        ("build_edit", "Build - Edit"),
        ("build_create", "Build - Create"),
        ("build_land", "Build - Land"),
    ] {
        let command = catalogue::find(id).expect("menu command");
        assert_eq!(command.label_in("en"), label);
        let label = command.label();
        assert!(!label.contains("Ctrl") && !label.contains('('), "{id}: {label}");
    }
}

/// One function, one icon: the menu commands' icons are not another
/// command's of the catalogue.
#[test]
fn menu_toolbar_commands_have_icons_of_their_own() {
    for id in ["movement_enhancer", "movelock_options"] {
        let icon = catalogue::find(id).expect("menu command").icon;
        let others: Vec<_> = COMMANDS.iter().filter(|c| c.id != id && c.icon == icon).map(|c| c.id).collect();
        assert!(others.is_empty(), "{id}'s {icon} is also {others:?}'s");
    }
}

/// Build - Edit and Build - Create saved with their former icons take the
/// build window's (pointer, wand) on every bar; another icon stays.
#[test]
fn saved_build_buttons_take_the_build_window_icons() {
    let saved = |command: &str, icon: &str| ToolButton {
        command: command.into(),
        icon: icon.into(),
        tooltip: String::new(),
    };
    let mut settings = ToolbarSettings {
        buttons: Some(vec![saved("build_edit", "pencil-filled"), saved("inventory", "pencil-filled")]),
        left: vec![saved("build_create", "circle-plus-filled")],
        top: vec![saved("build_edit", "star-filled")],
        ..Default::default()
    };
    assert!(settings.migrate_build_icons());
    let icons = |bar: &[ToolButton]| bar.iter().map(|b| b.icon.clone()).collect::<Vec<_>>();
    assert_eq!(
        icons(settings.buttons.as_deref().expect("bottom bar")),
        ["pointer-filled", "pencil-filled"]
    );
    assert_eq!(icons(&settings.left), ["wand"]);
    assert_eq!(icons(&settings.top), ["star-filled"]);
    assert!(!settings.migrate_build_icons(), "once");
}

#[test]
fn director_button_is_added_next_to_the_photo_button_once() {
    let commands = |bar: &[ToolButton]| bar.iter().map(|b| b.command.clone()).collect::<Vec<_>>();
    // the default bar already has it, right after the photo button
    let default = commands(&Layout::default().toolbar);
    let photo = default.iter().position(|c| c == "snapshot").expect("photo button");
    assert_eq!(default[photo + 1], "director");
    // a bar that follows the skin is left as it is
    let mut skin = ToolbarSettings::default();
    assert!(skin.migrate_director_button());
    assert!(skin.buttons.is_none());
    // an arranged bottom bar: after the photo button, once
    let mut settings = ToolbarSettings {
        buttons: Some(vec![button("inventory"), button("snapshot"), button("preferences")]),
        ..Default::default()
    };
    assert!(settings.migrate_director_button());
    let bottom = settings.buttons.as_mut().expect("custom toolbar");
    assert_eq!(commands(bottom), ["inventory", "snapshot", "director", "preferences"]);
    bottom.retain(|b| b.command != "director");
    assert!(!settings.migrate_director_button(), "a removed Director stays removed");
    assert_eq!(
        commands(settings.buttons.as_ref().expect("custom toolbar")),
        ["inventory", "snapshot", "preferences"]
    );
    // the photo button on a side bar: the Director joins it there
    let mut side = ToolbarSettings {
        buttons: Some(vec![button("inventory")]),
        right: vec![button("snapshot"), button("worldmap")],
        ..Default::default()
    };
    side.migrate_director_button();
    assert_eq!(commands(&side.right), ["snapshot", "director", "worldmap"]);
    assert_eq!(commands(side.buttons.as_ref().expect("custom toolbar")), ["inventory"]);
    // no photo button: at the end of the bottom bar
    let mut none = ToolbarSettings {
        buttons: Some(vec![button("inventory")]),
        ..Default::default()
    };
    none.migrate_director_button();
    assert_eq!(commands(none.buttons.as_ref().expect("custom toolbar")), ["inventory", "director"]);
    // an emptied bottom bar and an already placed Director are left alone
    let mut empty = ToolbarSettings {
        buttons: Some(vec![]),
        ..Default::default()
    };
    empty.migrate_director_button();
    assert!(empty.buttons.expect("explicit empty toolbar").is_empty());
    let mut placed = ToolbarSettings {
        buttons: Some(vec![button("snapshot")]),
        left: vec![button("director")],
        ..Default::default()
    };
    placed.migrate_director_button();
    assert_eq!(commands(placed.buttons.as_ref().expect("custom toolbar")), ["snapshot"]);
    assert_eq!(commands(&placed.left), ["director"]);
}

/// Floating blocks (human request, 2026-11-25): the catalogue's entry makes
/// one with its button, its buttons follow the one-place rule, it outlives
/// its button, and only the user's own can be deleted.
#[test]
fn floating_blocks_keep_one_place_per_command_and_only_the_users_are_deleted() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(vec![button("inventory"), button("import_texture")]),
        top: Vec::new(),
        ..Default::default()
    };
    assert!(settings.ensure_import_bar(&layout()));
    assert!(!settings.ensure_import_bar(&layout()), "once");
    let import = &settings.drawers[drawers::IMPORT_BAR];
    let import_commands: Vec<_> = import.buttons.iter().map(|b| b.command.as_str()).collect();
    assert_eq!(
        import_commands,
        [
            "import_texture",
            "import_animation",
            "import_sound",
            "import_material",
            "import_mesh",
            "divider:import-bar-files",
            "import_advanced",
            "import_local",
            "import_settings"
        ],
        "every import button, taken from where it stood"
    );
    assert_eq!(commands(settings.buttons.as_ref().expect("bottom")), ["inventory"]);
    assert_eq!(import.mode, ButtonMode::IconsOnly);
    assert!(settings.placed("import_mesh", &layout()) && settings.location("import_mesh", &layout()).is_none());

    let mut state = ToolbarUi {
        pending: Some(Edit::Insert(Bar::Left, button(drawers::FLOATING), 0)),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    let id = settings.left[0].command.clone();
    assert!(drawers::is_floating(&settings, &id) && id != drawers::IMPORT_BAR);
    assert_eq!(settings.left[0].tooltip, aurora_i18n::tr!("toolbar-floating-new"));
    assert!(settings.drawers[&id].floating.as_ref().is_some_and(|floating| floating.shown));

    state.pending = Some(Edit::DrawerInsert(id.clone(), button("inventory"), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.buttons(&layout()).iter().any(|b| b.command == "inventory"));
    assert_eq!(settings.drawer_of("inventory"), Some(id.as_str()));
    assert!(settings.placed("inventory", &layout()));
    for refused in [drawers::FLOATING, "bar_clock", drawers::IMPORT_BAR] {
        state.pending = Some(Edit::DrawerInsert(id.clone(), button(refused), 0));
        assert!(!state.commit(&layout(), &mut settings), "{refused}");
    }

    // its button removed, the block stays, and so do its buttons
    state.pending = Some(Edit::Remove(id.clone()));
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.left.is_empty());
    settings.sanitize();
    assert_eq!(settings.drawer_of("inventory"), Some(id.as_str()));

    // shown or hidden even locked, as a window
    settings.locked = true;
    state.pending = Some(Edit::FloatingShown(id.clone(), false));
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.drawers[&id].floating.as_ref().is_some_and(|floating| !floating.shown));
    state.pending = Some(Edit::FloatingDelete(id.clone()));
    assert!(!state.commit(&layout(), &mut settings));
    settings.locked = false;

    // a button taken out goes where it is dropped
    state.pending = Some(Edit::Insert(Bar::Bottom, button("import_mesh"), 0));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.location("import_mesh", &layout()), Some(Bar::Bottom));
    assert!(
        !settings.drawers[drawers::IMPORT_BAR]
            .buttons
            .iter()
            .any(|b| b.command == "import_mesh")
    );

    state.pending = Some(Edit::FloatingDelete(id.clone()));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.drawers.contains_key(&id));
    assert!(!settings.placed("inventory", &layout()), "its buttons go back to the catalogue");

    // the import block is deleted like the others, and not made again
    // (human request, 2026-11-25)
    state.pending = Some(Edit::FloatingDelete(drawers::IMPORT_BAR.into()));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.drawers.contains_key(drawers::IMPORT_BAR));
    assert!(!settings.ensure_import_bar(&layout()));
    assert!(!settings.drawers.contains_key(drawers::IMPORT_BAR));

    state.pending = Some(Edit::Insert(
        Bar::Top,
        drawers::floating_toggle(&drawers::create_floating(&mut settings, "Tools"), "Tools"),
        0,
    ));
    assert!(state.commit(&layout(), &mut settings));
    state.pending = Some(Edit::Clear);
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.drawers.is_empty(), "every block goes");
}

#[test]
fn a_floating_block_dropped_on_the_screen_takes_its_place_and_its_options() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(vec![button("inventory")]),
        top: Vec::new(),
        ..Default::default()
    };
    let mut state = ToolbarUi {
        pending: Some(Edit::FloatingNew(egui::pos2(300.0, 200.0))),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    let (id, pos) = state.floating_placed.clone().expect("where it was dropped");
    assert_eq!(pos, egui::pos2(300.0, 200.0));
    assert!(drawers::is_floating(&settings, &id));
    assert!(settings.location(&id, &layout()).is_none(), "its button is placed nowhere yet");
    let tiles = catalogue_view::entries(&layout(), &settings, &Default::default());
    assert!(
        tiles.iter().any(|tile| tile.button.command == id),
        "the catalogue offers its button"
    );
    assert!(
        !tiles.iter().any(|tile| tile.button.command == drawers::FLOATING),
        "it stands beside the separator, out of the grid"
    );
    assert!(
        !tiles.iter().any(|tile| tile.button.command == drawers::IMPORT_BAR),
        "no import block yet"
    );

    state.pending = Some(Edit::Insert(Bar::Top, drawers::floating_toggle(&id, "x"), 0));
    assert!(state.commit(&layout(), &mut settings));
    let options = block_window::Options {
        name: "Tools".into(),
        mode: ButtonMode::TextOnly,
        vertical: true,
        closes: true,
        hides_button: true,
        look: appearance::Appearance {
            mode: None,
            icon: Some("star-filled".into()),
        },
        align: Align::Center,
        sizing: Sizing::Fixed,
    };
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(id.clone()),
        |o| {
            o.name = options.clone().name.clone();
            o.mode = options.clone().mode;
            o.vertical = options.clone().vertical;
            o.closes = options.clone().closes;
            o.hides_button = options.clone().hides_button;
            o.look = options.clone().look.clone();
        },
    ));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.top[0].tooltip, "Tools", "its placed button takes the name");
    let floating = settings.drawers[&id].floating.clone().expect("floating");
    assert!(floating.vertical && floating.closes && floating.name == "Tools");
    assert_eq!(settings.drawers[&id].mode, ButtonMode::TextOnly);
    assert_eq!(settings.appearance[&id], options.look);
    state.pending = Some(block_window::edit_of(
        &settings,
        &layout(),
        block_window::Target::Drawer(id.clone()),
        |o| {
            o.name = options.name.clone();
            o.mode = options.mode;
            o.vertical = options.vertical;
            o.closes = options.closes;
            o.hides_button = options.hides_button;
            o.look = options.look.clone();
        },
    ));
    assert!(!state.commit(&layout(), &mut settings), "no change, no save");

    // the catalogue's entry never stays on a bar; the import block's button
    // does, and the catalogue offers it (human request, 2026-11-25)
    assert!(settings.ensure_import_bar(&layout()));
    let tiles = catalogue_view::entries(&layout(), &settings, &Default::default());
    assert!(tiles.iter().any(|tile| tile.button.command == drawers::IMPORT_BAR));
    settings.left = vec![button(drawers::FLOATING), button(drawers::IMPORT_BAR)];
    settings.sanitize();
    assert_eq!(commands(&settings.left), [drawers::IMPORT_BAR]);
}

/// The import bar takes its buttons once, out of a drawer included (the
/// human's had them all, 2026-11-25), and a drawer left with gaps only goes;
/// made again later it takes only those placed nowhere.
#[test]
fn the_import_bar_takes_its_buttons_once() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(vec![button("inventory")]),
        top: vec![button("drawer:imports"), button("people")],
        ..Default::default()
    };
    settings.drawers.insert(
        "drawer:imports".into(),
        drawers::Drawer {
            buttons: vec![button("import_texture"), structure::divider("d"), button("import_mesh")],
            ..Default::default()
        },
    );
    assert!(settings.ensure_import_bar(&layout()));
    assert!(!settings.drawers.contains_key("drawer:imports"), "emptied, it goes with its button");
    assert_eq!(commands(&settings.top), ["people"]);
    assert!(
        settings.drawers[drawers::IMPORT_BAR]
            .buttons
            .iter()
            .any(|b| b.command == "import_texture")
    );

    // made a fixed block or deleted, it is not made again (human request,
    // 2026-11-25: a preset that has it brings it back)
    settings.drawers.remove(drawers::IMPORT_BAR);
    assert!(!settings.ensure_import_bar(&layout()));
    assert!(!settings.drawers.contains_key(drawers::IMPORT_BAR));
}

/// A block, a drawer and a floating block become one another (human
/// request, 2026-11-25).
#[test]
fn blocks_drawers_and_floating_blocks_switch_into_one_another() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        top: vec![button("inventory"), button("people"), structure::separator("gap"), button("search")],
        ..Default::default()
    };
    settings.appearance.insert(
        "inventory".into(),
        appearance::Appearance {
            mode: None,
            icon: Some("star-filled".into()),
        },
    );
    let pos = egui::pos2(10.0, 60.0);
    let mut state = ToolbarUi {
        pending: Some(Edit::Float(Bar::Top, "people".into(), pos)),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    let id = settings.top[0].command.clone();
    assert_eq!(state.floating_placed, Some((id.clone(), pos)));
    assert_eq!(
        (settings.top[0].icon.as_str(), settings.top[0].tooltip.as_str()),
        ("star-filled", "inventory")
    );
    let floating = settings.drawers[&id].floating.clone().expect("floating");
    assert_eq!((floating.name.as_str(), floating.icon.as_str()), ("inventory", "star-filled"));
    assert_eq!(commands(&settings.drawers[&id].buttons), ["inventory", "people"]);
    assert_eq!(commands(&settings.top[1..]), ["separator:gap", "search"]);

    // a drawer again, its button kept
    state.pending = Some(Edit::Dock(id.clone(), true));
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.drawers[&id].floating.is_none());
    assert_eq!(settings.top[0].command, id);
    // floating again, its name and icon kept
    state.pending = Some(Edit::Float(Bar::Top, id.clone(), pos));
    assert!(state.commit(&layout(), &mut settings));
    assert!(drawers::is_floating(&settings, &id));
    assert_eq!(
        settings.drawers[&id].floating.as_ref().map(|f| f.icon.as_str()),
        Some("star-filled")
    );
    // a block of its bar again, its button removed first: back at the bottom
    state.pending = Some(Edit::Remove(id.clone()));
    assert!(state.commit(&layout(), &mut settings));
    state.pending = Some(Edit::Dock(id.clone(), false));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.drawers.contains_key(&id));
    assert_eq!(commands(settings.buttons.as_ref().expect("bottom")), ["inventory", "people"]);

    // a bar's block cannot float; the import block docks like the others
    // (human request, 2026-11-25)
    settings.top.insert(0, button("bar_clock"));
    state.pending = Some(Edit::Float(Bar::Top, "bar_clock".into(), pos));
    assert!(!state.commit(&layout(), &mut settings));
    assert!(settings.ensure_import_bar(&layout()));
    state.pending = Some(Edit::Dock(drawers::IMPORT_BAR.into(), true));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!drawers::is_floating(&settings, drawers::IMPORT_BAR));
    assert!(
        settings.location(drawers::IMPORT_BAR, &layout()).is_some(),
        "a drawer, its button in a bar"
    );
    // a drawer like the others: its button removed, it goes
    state.pending = Some(Edit::Remove(drawers::IMPORT_BAR.into()));
    assert!(state.commit(&layout(), &mut settings));
    assert!(!settings.drawers.contains_key(drawers::IMPORT_BAR));
}

/// A plain block's shortcut follows it into a drawer, a floating block and
/// back (human request, 2026-11-25).
#[test]
fn a_blocks_shortcut_follows_it_through_drawers_and_floating_blocks() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        top: vec![button("inventory"), button("people"), structure::separator("gap"), button("search")],
        ..Default::default()
    };
    let binding = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyK".into()),
        ctrl: true,
        shift: false,
        alt: false,
    };
    // saved under its second button: the block's all the same
    settings.shortcuts.insert(shortcuts::block_key("people"), binding.clone());
    let top = settings.top.clone();
    assert_eq!(shortcuts::block_of(&settings, &top, "inventory").as_deref(), Some("block:people"));
    assert_eq!(shortcuts::block_of(&settings, &top, "search").as_deref(), Some("block:search"));
    let mut state = ToolbarUi {
        pending: Some(Edit::Fold(Bar::Top, "inventory".into())),
        ..Default::default()
    };
    assert!(state.commit(&layout(), &mut settings));
    let id = settings.top[0].command.clone();
    assert_eq!(settings.shortcuts.get(&id), Some(&binding));
    assert_eq!(shortcuts::target(&settings, &layout(), &binding).as_deref(), Some(id.as_str()));
    state.pending = Some(Edit::Unfold(Bar::Top, id.clone()));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.shortcuts.keys().collect::<Vec<_>>(), ["block:inventory"]);
    assert_eq!(shortcuts::target(&settings, &layout(), &binding), None, "a plain block's waits");
    state.pending = Some(Edit::Float(Bar::Top, "people".into(), egui::pos2(10.0, 60.0)));
    assert!(state.commit(&layout(), &mut settings));
    let floating = settings.top[0].command.clone();
    assert!(drawers::is_floating(&settings, &floating));
    assert_eq!(settings.shortcuts.get(&floating), Some(&binding));
    state.pending = Some(Edit::Dock(floating.clone(), false));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.shortcuts.keys().collect::<Vec<_>>(), ["block:inventory"]);
    // a floating block deleted takes its shortcut with it
    state.pending = Some(Edit::Float(Bar::Top, "inventory".into(), egui::pos2(10.0, 60.0)));
    assert!(state.commit(&layout(), &mut settings));
    let floating = settings.top[0].command.clone();
    state.pending = Some(Edit::FloatingDelete(floating));
    assert!(state.commit(&layout(), &mut settings));
    assert!(settings.shortcuts.is_empty());
    // the shortcut window's edit, locked bars included
    settings.locked = true;
    state.pending = Some(Edit::Shortcut("search".into(), Some(binding.clone())));
    assert!(state.commit(&layout(), &mut settings));
    assert_eq!(settings.shortcuts.get("search"), Some(&binding));
}

/// The import bar's first fill had a separator: it becomes a block's
/// divider; Ctrl+U shows the bar, given once (human requests, 2026-11-25).
#[test]
fn the_import_bar_gets_its_divider_and_ctrl_u_once() {
    let mut settings = ToolbarSettings {
        composed: true,
        buttons: Some(Vec::new()),
        import_bar_filled: true,
        ..Default::default()
    };
    settings.drawers.insert(
        drawers::IMPORT_BAR.into(),
        drawers::Drawer {
            buttons: vec![
                button("import_mesh"),
                structure::separator("import-bar-files"),
                button("import_local"),
            ],
            floating: Some(Default::default()),
            ..Default::default()
        },
    );
    assert!(settings.ensure_import_bar(&layout()));
    assert_eq!(
        commands(&settings.drawers[drawers::IMPORT_BAR].buttons),
        ["import_mesh", "divider:import-bar-files", "import_local"]
    );
    let ctrl_u = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyU".into()),
        ctrl: true,
        shift: false,
        alt: false,
    };
    assert_eq!(settings.shortcuts.get(drawers::IMPORT_BAR), Some(&ctrl_u));
    assert_eq!(
        shortcuts::target(&settings, &layout(), &ctrl_u).as_deref(),
        Some(drawers::IMPORT_BAR)
    );
    settings.shortcuts.clear();
    assert!(!settings.ensure_import_bar(&layout()), "once: a removed shortcut stays removed");
    assert!(settings.shortcuts.is_empty());
}

/// Gaps that follow one another between two blocks are one, on the bars and
/// in the drawers, the first keeping its look (human request, 2026-11-25).
#[test]
fn gaps_that_follow_one_another_are_one() {
    let mut settings = ToolbarSettings {
        buttons: Some(vec![
            button("inventory"),
            structure::separator("a"),
            structure::separator("b"),
            structure::separator("c"),
            button("people"),
        ]),
        top: vec![structure::separator("d"), button("worldmap")],
        ..Default::default()
    };
    settings.separators.insert(
        "separator:b".into(),
        structure::SeparatorStyle {
            size: 30.0,
            ..Default::default()
        },
    );
    settings.drawers.insert(
        "drawer:shelf".into(),
        drawers::Drawer {
            buttons: vec![
                button("search"),
                structure::separator("e"),
                structure::separator("f"),
                button("map"),
            ],
            ..Default::default()
        },
    );
    assert!(settings.single_gaps());
    assert_eq!(
        commands(settings.buttons.as_deref().unwrap_or_default()),
        ["inventory", "separator:a", "people"]
    );
    assert!(!settings.separators.contains_key("separator:b"));
    assert_eq!(commands(&settings.top), ["separator:d", "worldmap"], "a lone gap stays");
    assert_eq!(
        commands(&settings.drawers["drawer:shelf"].buttons),
        ["search", "separator:e", "map"]
    );
    assert!(!settings.single_gaps(), "nothing left to merge");
}
