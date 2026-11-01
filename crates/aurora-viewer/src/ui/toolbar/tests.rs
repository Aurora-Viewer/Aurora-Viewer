use super::*;
use crate::ui::{Panels, bars, chat::ChatUi, icons::Icons};
use egui::Vec2;

fn button(command: &str) -> ToolButton {
    ToolButton {
        command: command.into(),
        icon: "test-icon".into(),
        tooltip: command.into(),
    }
}

fn layout() -> Layout {
    Layout {
        toolbar: vec![button("inventory"), button("people"), button("preferences")],
        ..Default::default()
    }
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
            vec!["inventory", "vr_view"]
        } else {
            vec!["inventory"]
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
    for _ in 0..4 {
        bench.frame(vec![]);
    }
    let window = bench
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("toolbar_editor")))
        .expect("editor");
    assert!(window.width() <= 900.0 - 16.0, "catalogue width: {window:?}");
    for button in catalogue(&layout()) {
        let rect = bench
            .ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", button.command))))
            .expect("catalogue button");
        assert!(
            rect.left() >= window.left() && rect.right() <= window.right(),
            "button outside editor: {rect:?} / {window:?}"
        );
    }
}

struct Bench {
    ctx: egui::Context,
    state: ToolbarUi,
    settings: ToolbarSettings,
    panels: Panels,
    saves: usize,
    /// The dock of the islands instead of Firestorm's bar.
    islands: bool,
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
            islands: false,
        }
    }
}

impl Bench {
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        // the dock is an egui area: shown from its second frame on
        if self.islands && self.ctx.cumulative_frame_nr() == 0 {
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
                bars::bottom_bar(
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
                    false,
                    false,
                    &mut 0.0,
                    &mut bars::MicButton::default(),
                    false,
                    self.islands,
                    None,
                    &bars::favorites::View::default(),
                );
                self.state.show(ui.ctx(), &p, &layout, &self.settings);
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
}

/// Firestorm's bar and the dock of the islands.
#[test]
fn toolbar_real_right_click_removes_one_button_without_opening_its_window() {
    for islands in [false, true] {
        toolbar_real_right_click_removes_one_button_without_opening_its_window_in(islands);
    }
}

fn toolbar_real_right_click_removes_one_button_without_opening_its_window_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.frame(vec![]);
    let pos = bench.pos("people");
    bench.pointer(pos, true, true);
    bench.pointer(pos, false, true);
    let remove = bench.text(aurora_i18n::tr!("toolbar-remove"));
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
    let to = bench.pos("preferences") + Vec2::new(30.0, 0.0);
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
    let target = bench
        .ctx
        .read_response(egui::Id::unique("toolbar_background"))
        .expect("empty bar")
        .rect
        .center();
    bench.pointer(source, true, false);
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
fn toolbar_empty_background_opens_editor_and_locked_buttons_stay_in_place() {
    for islands in [false, true] {
        toolbar_empty_background_opens_editor_and_locked_buttons_stay_in_place_in(islands);
    }
}

fn toolbar_empty_background_opens_editor_and_locked_buttons_stay_in_place_in(islands: bool) {
    let mut bench = Bench {
        islands,
        ..Bench::default()
    };
    bench.settings.buttons = Some(vec![]);
    bench.frame(vec![]);
    let background = bench
        .ctx
        .read_response(egui::Id::unique("toolbar_background"))
        .expect("empty background")
        .rect
        .center();
    bench.pointer(background, true, true);
    bench.pointer(background, false, true);
    let edit = bench.text(aurora_i18n::tr!("toolbar-edit"));
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
fn toolbar_drop_outside_cancels_and_drop_back_in_catalogue_removes() {
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
    let outside = egui::pos2(20.0, 100.0);
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

/// The window lists the Legacy Viewer commands and Aurora additions, each
/// once, with an embedded icon and localized labels;
/// what Aurora runs does something, what it cannot run yet does nothing.
#[test]
fn toolbar_catalogue_holds_firestorm_commands_and_runs_the_ready_ones() {
    let skin = Layout::default();
    let all = catalogue(&skin);
    assert_eq!(all.len(), 74);
    assert_eq!(COMMANDS.len(), 74);
    let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 74, "each command once");
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
        // Speak, Favorites and the AO's toggle have buttons of their own
        if matches!(command.id, "speak" | "favorites") {
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
