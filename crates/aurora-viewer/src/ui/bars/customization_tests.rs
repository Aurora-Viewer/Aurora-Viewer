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
    st.can_go_home = false;
    assert!(matches!(invoke("menu:common-me/common-go-home", &st), BarAction::None));
    st.can_go_home = true;
    assert!(matches!(invoke("menu:common-me/common-go-home", &st), BarAction::TeleportHome));
    st.hover_tips.enabled = false;
    let land_tips = "menu:common-world/ui-options-hover-tips/ui-options-hover-tips-land";
    assert!(matches!(invoke(land_tips, &st), BarAction::None));
    st.hover_tips.enabled = true;
    assert!(matches!(invoke(land_tips, &st), BarAction::HoverTips(_)));
    assert!(matches!(invoke("menu:removed-command", &st), BarAction::None));
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
    assert!(saved.top.iter().any(|button| button.command == "location_bar"));
    assert!(!Bar::Top.vertical());
}
