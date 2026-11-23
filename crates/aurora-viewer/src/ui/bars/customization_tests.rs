use super::*;
use crate::ui::toolbar::{self, Bar, ButtonMode, ToolbarSettings, ToolbarUi, commands};

#[test]
fn menu_catalogue_collection_has_no_actions_and_contains_stable_distinct_commands() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut st = super::islands::tests::status(true, true);
    st.marketplace = true;
    st.region_console = true;
    st.release_notes = true;
    let _ = ctx.run_ui(Default::default(), |ui| collect_menu(ui, &p, &mut panels, &st));
    let entries = commands::catalogue(&ctx);
    let ids: std::collections::HashSet<_> = entries.entries.iter().map(|entry| &entry.button.command).collect();
    assert_eq!(ids.len(), entries.entries.len());
    assert!(
        entries
            .entries
            .iter()
            .filter(|entry| entry.button.command.starts_with(commands::PREFIX))
            .count()
            > 100
    );
    assert!(entries.find("menu:common-build/import-menu").is_some());
    assert!(entries.find("menu:common-world/common-environment/time-of-day-2").is_some());
    assert!(!panels.inventory && !panels.settings && !panels.appearance);
    assert_eq!(panels.time_of_day, 0);
    assert!(crate::ui::context::take_requests(&ctx).is_empty());
    assert!(commands::take_request(&ctx).is_none());
    st.flying = true;
    let _ = ctx.run_ui(Default::default(), |ui| collect_menu(ui, &p, &mut panels, &st));
    let next = commands::catalogue(&ctx);
    let next_ids: std::collections::HashSet<_> = next.entries.iter().map(|entry| &entry.button.command).collect();
    assert_eq!(ids, next_ids, "flight changes a label, never a saved command identity");
}

#[test]
fn moved_menu_commands_use_their_native_handlers_and_current_enabled_guards() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut st = super::islands::tests::status(true, true);
    let mut action = BarAction::None;
    let mut invoke = |command: &str, st: &StatusInfo| {
        let _ = ctx.run_ui(Default::default(), |ui| {
            collect_menu(ui, &p, &mut panels, st);
            commands::request(ui.ctx(), command);
            action = dispatch_menu(ui, &p, &mut panels, st);
        });
        std::mem::take(&mut action)
    };
    assert!(matches!(invoke("menu:common-build/import-menu", &st), BarAction::Import));
    st.home_entry = false;
    assert!(matches!(invoke("menu:common-me/common-go-home", &st), BarAction::None));
    st.home_entry = true;
    assert!(matches!(invoke("menu:common-me/common-go-home", &st), BarAction::TeleportHome));
    st.hover_tips.enabled = false;
    let land_tips = "menu:common-world/ui-options-hover-tips/ui-options-hover-tips-land";
    assert!(matches!(invoke(land_tips, &st), BarAction::None));
    st.hover_tips.enabled = true;
    assert!(matches!(invoke(land_tips, &st), BarAction::HoverTips(_)));
    assert!(matches!(invoke("menu:removed-command", &st), BarAction::None));
    assert!(matches!(invoke("menu:common-advanced/debug-settings", &st), BarAction::None));
    assert!(panels.debug_settings, "the new menu command retains its native window handler");
}

#[test]
fn menu_layout_reparents_groups_preserves_new_entries_and_rejects_cycles() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let st = super::islands::tests::status(true, true);
    let _ = ctx.run_ui(Default::default(), |ui| collect_menu(ui, &p, &mut panels, &st));
    let catalogue = commands::catalogue(&ctx);
    let mut layout = std::collections::BTreeMap::new();
    let world = "menu:common-world";
    let environment = "menu:common-world/common-environment";
    assert!(!catalogue.can_place(world, environment, &layout));
    assert!(!catalogue.can_place(world, world, &layout));
    assert!(catalogue.can_place(environment, "", &layout));
    layout.insert(String::new(), vec![environment.into()]);
    assert_eq!(catalogue.children("", &layout)[0].button.command, environment);
    assert!(
        !catalogue
            .children(world, &layout)
            .iter()
            .any(|entry| entry.button.command == environment)
    );
    assert!(catalogue.children("", &layout).iter().any(|entry| entry.button.command == world));
    assert!(!catalogue.children(environment, &layout).is_empty());
    layout.insert(environment.into(), vec![world.into()]);
    assert!(!catalogue.can_place(environment, world, &layout));
}

#[test]
fn aurora_menu_hover_shows_one_tooltip() {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let p = crate::theme::Theme::default().palette();
    let settings = ToolbarSettings::default();
    let st = islands::tests::status(true, true);
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut state = ToolbarUi::default();
    let mut count = 0;
    fn occurrences(shape: &egui::Shape, label: &str) -> usize {
        match shape {
            egui::Shape::Text(text) => usize::from(text.galley.text() == label),
            egui::Shape::Vec(shapes) => shapes.iter().map(|shape| occurrences(shape, label)).sum(),
            _ => 0,
        }
    }
    for frame in 0..10 {
        let pointer = ctx
            .read_response(egui::Id::unique("bars_island_menu"))
            .map_or(egui::pos2(26.0, 26.0), |response| response.rect.center());
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(frame as f64 * 0.3),
                events: if frame < 3 {
                    vec![egui::Event::PointerMoved(pointer)]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
            |ui| {
                state.prepare(&ctx, &settings);
                islands::top(
                    ui,
                    &p,
                    &Icons::default(),
                    &mut panels,
                    &st,
                    &mut Default::default(),
                    &mut Default::default(),
                    Default::default(),
                    &mut Default::default(),
                    Default::default(),
                    &mut state,
                    &settings,
                );
            },
        );
        count = output
            .shapes
            .iter()
            .map(|shape| occurrences(&shape.shape, aurora_i18n::tr!("ui-bars-aurora-menu")))
            .sum();
    }
    assert!(
        ctx.read_response(egui::Id::unique("bars_island_menu"))
            .expect("Aurora button")
            .hovered()
    );
    assert_eq!(count, 1, "a single label in the tooltip");
}

#[test]
fn categorized_catalogue_merges_about_land_and_preserves_a_saved_menu_copy() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let layout = Layout::default();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let st = islands::tests::status(true, true);
    let _ = ctx.run_ui(Default::default(), |ui| collect_menu(ui, &p, &mut panels, &st));
    let catalogue = commands::catalogue(&ctx);
    let mut settings = ToolbarSettings::default();
    let about = "menu:common-world/common-about-land";
    let menu_button = catalogue.find(about).expect("About Land menu entry").button.clone();
    settings.top.push(menu_button.clone());
    let entries = toolbar::catalogue_view::entries(&layout, &settings, &catalogue);
    let about_entries: Vec<_> = entries
        .iter()
        .filter(|entry| matches!(entry.button.command.as_str(), "aboutland" | "menu:common-world/common-about-land"))
        .collect();
    assert_eq!(about_entries.len(), 1, "one feature, one catalogue tile");
    assert_eq!(about_entries[0].button, menu_button, "keep the placed menu command's ID and icon");
    assert_eq!(about_entries[0].category, toolbar::catalogue_view::Category::World);
    assert!(catalogue.find(about).is_some(), "the original menu is unchanged");
    let gesture_count = entries.iter().filter(|entry| entry.button.command == "gestures").count();
    assert_eq!(gesture_count, 1, "both menu locations share the gesture tile");
    let group = entries
        .iter()
        .find(|entry| entry.button.command == "menu:common-build")
        .expect("build submenu");
    assert!(group.group);
    assert_ne!(
        group.label(),
        aurora_i18n::text("common-build"),
        "submenus are distinguished from actions"
    );
}

#[test]
fn mixed_button_modes_inherit_only_when_requested_and_round_trip_with_icons() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let layout = Layout::default();
    let inventory = layout
        .toolbar
        .iter()
        .find(|button| button.command == "inventory")
        .expect("inventory");
    let mut settings = ToolbarSettings::default();
    settings.appearance.insert(
        "inventory".into(),
        toolbar::appearance::Appearance {
            mode: Some(ButtonMode::IconsWithText),
            icon: Some("star-filled".into()),
        },
    );
    let mut widths = (0.0, 0.0);
    let _ = ctx.run_ui(Default::default(), |ui| {
        let state = ToolbarUi::default();
        state.prepare(ui.ctx(), &settings);
        widths.0 = natural_width(ui.ctx(), inventory, ButtonMode::IconsOnly, 28.0, 16.0, 38.0);
        settings.appearance.get_mut("inventory").expect("appearance").mode = None;
        state.prepare(ui.ctx(), &settings);
        widths.1 = natural_width(ui.ctx(), inventory, ButtonMode::IconsOnly, 28.0, 16.0, 38.0);
        let _ = p;
    });
    assert!(widths.0 > widths.1);
    assert_eq!(widths.1, 28.0);
    let saved: ToolbarSettings = serde_json::from_str(&serde_json::to_string(&settings).expect("settings")).expect("round trip");
    assert_eq!(saved.appearance["inventory"].icon.as_deref(), Some("star-filled"));
    assert_eq!(saved.appearance["inventory"].mode, None);
    assert_eq!(saved.top, settings.top);
    assert!(!Bar::Top.vertical());
}

#[test]
fn top_toolbar_keeps_a_labelled_override_wide_beside_icon_only_buttons() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut settings = ToolbarSettings {
        address_visible: Some(false),
        ..Default::default()
    };
    settings.top = ["location_bar", "preferences", "home"]
        .into_iter()
        .map(|id| {
            let command = toolbar::COMMANDS.iter().find(|command| command.id == id).expect("command");
            crate::ui::skin::ToolButton {
                command: id.into(),
                icon: command.icon.into(),
                tooltip: String::new(),
            }
        })
        .collect();
    settings.appearance.insert(
        "preferences".into(),
        toolbar::appearance::Appearance {
            mode: Some(ButtonMode::IconsWithText),
            icon: None,
        },
    );
    let mut state = ToolbarUi::default();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut expected = 0.0;
    let _ = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            ..Default::default()
        },
        |ui| {
            state.prepare(ui.ctx(), &settings);
            expected = natural_width(
                ui.ctx(),
                &settings.top[1],
                ButtonMode::IconsOnly,
                islands::SLOT,
                islands::ICON,
                38.0,
            );
            top::reserve(
                ui,
                &settings,
                egui::Rect::from_min_size(egui::pos2(80.0, 8.0), Vec2::new(1000.0, 36.0)),
                80.0,
            );
            let favorites = favorites::View::default();
            let row = Row {
                unread: 0,
                latest_unread: None,
                flash: 0.0,
                flying: false,
                seated: false,
                mouselook: false,
                ao_active: false,
                locked: false,
                favorites: &favorites,
                bar: Bar::Top,
                style: Default::default(),
                editing: false,
            };
            top::show(
                ui,
                &p,
                &Icons::default(),
                &Layout::default(),
                &mut state,
                &settings,
                &mut panels,
                &mut MicButton::default(),
                &row,
                false,
            );
        },
    );
    let response = ctx.read_response(tool_button_id("preferences")).expect("top preferences");
    assert!(expected > islands::SLOT);
    assert!(response.rect.width() >= expected - 0.5, "{} < {expected}", response.rect.width());
    assert_eq!(
        ctx.read_response(tool_button_id("location_bar"))
            .expect("location button")
            .rect
            .width(),
        islands::SLOT
    );
}

#[test]
fn a_crowded_top_toolbar_preserves_the_address_and_status_islands() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut settings = ToolbarSettings {
        address_visible: Some(true),
        ..Default::default()
    };
    settings.top_style.mode = Some(ButtonMode::IconsWithText);
    settings.top = toolbar::COMMANDS
        .iter()
        .take(12)
        .map(|command| crate::ui::skin::ToolButton {
            command: command.id.into(),
            icon: command.icon.into(),
            tooltip: String::new(),
        })
        .collect();
    let st = islands::tests::status(true, true);
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut toolbar = ToolbarUi::default();
    let mut minimum = 0.0;
    for _ in 0..4 {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                ..Default::default()
            },
            |ui| {
                toolbar.prepare(ui.ctx(), &settings);
                minimum = super::place::minimum_width(ui, &st);
                islands::top(
                    ui,
                    &p,
                    &Icons::default(),
                    &mut panels,
                    &st,
                    &mut Default::default(),
                    &mut Default::default(),
                    Default::default(),
                    &mut Default::default(),
                    Default::default(),
                    &mut toolbar,
                    &settings,
                );
            },
        );
    }
    let rect = |name| {
        ctx.memory(|memory| memory.area_rect(egui::Id::unique(("bars_island", name))))
            .expect("island")
    };
    let top = ctx
        .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("custom_top_toolbar_rect")))
        .expect("toolbar");
    assert!(rect("bug").right() + islands::MARGIN <= top.left() + 0.5);
    assert!(top.right() + islands::MARGIN <= rect("place").left() + 0.5);
    assert!(rect("place").right() + islands::MARGIN <= rect("account").left() + 0.5);
    assert!(rect("place").width() + 0.5 >= minimum);
}
