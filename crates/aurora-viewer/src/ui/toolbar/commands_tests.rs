use super::*;
use crate::ui::{Panels, bars, chat::ChatUi, icons::Icons};
use egui::{Id, Pos2, Rect, Vec2};

fn block_controls_fixture(bar: Bar, classic: bool, locked: bool) -> Bench {
    let mut bench = Bench {
        native_top: true,
        menu_open: false,
        classic,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.top.clear();
    bench.settings.buttons = Some(Vec::new());
    bench.settings.set_bar(
        bar,
        vec![
            tests::button("inventory"),
            structure::divider("inside"),
            tests::button("people"),
            structure::separator("boundary"),
            tests::button("preferences"),
        ],
    );
    bench.settings.locked = locked;
    bench.state.open = true;
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    bench
}

/// A block's settings are in its buttons' right click, the bars edited or
/// not: the gear is gone (human request, 2026-11-26).
fn open_block_settings(bench: &mut Bench, bar: Bar, command: &str) {
    assert_eq!(bench.settings.location(command, &Layout::default()), Some(bar));
    let point = bench
        .ctx
        .read_response(bars::tool_button_id(command))
        .expect("its button")
        .rect
        .center();
    right_click_at(bench, point);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(!ToolbarUi::dragging(&bench.ctx), "the right click must not start a drag");
    // its section's heading, in capitals
    let heading = aurora_i18n::tr!("toolbar-block-settings").to_uppercase();
    assert!(bench.text.iter().any(|(text, _)| text.to_uppercase() == heading));
}

/// The grip of the block of `command` in `bar`, which shows with the
/// pointer over the block (human request, 2026-11-26).
fn block_grip(bench: &mut Bench, bar: Bar, command: &str) -> Rect {
    let over = bench
        .ctx
        .read_response(bars::tool_button_id(command))
        .expect("its button")
        .rect
        .center();
    for _ in 0..2 {
        bench.pointer(over, None);
    }
    bench
        .ctx
        .read_response(Id::unique(("toolbar_block_grip", bar, command)))
        .expect("its grip shows")
        .rect
}

/// Scrolls the block window until the row `label` shows, and returns its
/// centre.
fn scroll_window_to(bench: &mut Bench, label: &str) -> Pos2 {
    let window = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings")))
        .expect("the block window");
    // below the title bar, where the rows are not hidden under it
    let inside = Rect::from_min_max(window.min + Vec2::new(8.0, 56.0), window.max - Vec2::splat(8.0));
    let find = |bench: &Bench| {
        bench
            .text
            .iter()
            .rev()
            .find(|(text, rect)| text == label && inside.contains(rect.center()))
            .map(|(_, rect)| rect.center())
    };
    for delta in std::iter::repeat_n(-60.0, 30).chain(std::iter::repeat_n(60.0, 60)) {
        if let Some(mut point) = find(bench) {
            // the scroll eases in: wait until the row stops moving
            for _ in 0..60 {
                bench.frame(Vec::new());
                match find(bench) {
                    Some(next) if next == point => return point,
                    Some(next) => point = next,
                    None => break,
                }
            }
        }
        bench.frame(vec![
            egui::Event::PointerMoved(window.center()),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, delta),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Default::default(),
            },
        ]);
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
    }
    panic!("{label} not reached in the block window");
}

fn menu_point(bench: &Bench, label: &str) -> Pos2 {
    bench
        .text
        .iter()
        .rev()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("missing menu row {label}"))
        .1
        .center()
}

#[test]
fn nearby_chat_first_and_last_share_the_bottom_toolbar_layer_and_edit_zone() {
    for classic in [false, true] {
        for editing in [false, true] {
            for first in [false, true] {
                let mut bench = Bench {
                    native_top: true,
                    menu_open: false,
                    classic,
                    screen: Vec2::new(1536.0, 864.0),
                    ..Default::default()
                };
                bench.settings.composed = true;
                bench.settings.chat_composed = true;
                let mut buttons = vec![
                    tests::button("bar_chat"),
                    structure::separator("chat-tools"),
                    tests::button("inventory"),
                ];
                if !first {
                    buttons.reverse();
                }
                bench.settings.buttons = Some(buttons);
                bench.state.open = editing;
                for _ in 0..5 {
                    bench.frame(Vec::new());
                }
                let chat = bench.ctx.read_response(bars::tool_button_id("bar_chat")).expect("chat slot");
                let bar = bench.ctx.read_response(Id::unique("toolbar_background")).expect("bottom toolbar");
                assert_eq!(chat.layer_id, bar.layer_id, "first={first}, classic={classic}, editing={editing}");
                assert!(
                    bar.rect.contains_rect(chat.rect),
                    "chat stays inside the same bar: first={first}, classic={classic}, editing={editing}: {:?}, {:?}",
                    chat.rect,
                    bar.rect
                );
            }
        }
    }
}

#[test]
fn stationary_right_click_closes_and_reopens_bar_contexts_without_executing_actions() {
    fn right_click(bench: &mut Bench, point: Pos2) {
        for pressed in [true, false] {
            bench.frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }
    for classic in [false, true] {
        for editing in [false, true] {
            for source in [
                "inventory",
                "aurora_menu",
                "report_bug",
                "bar_clock",
                "bar_volume",
                "bar_notifications",
                "gap",
                "drawer",
                "background",
            ] {
                let mut bench = Bench {
                    native_top: true,
                    menu_open: false,
                    classic,
                    screen: Vec2::new(1920.0, 1080.0),
                    ..Default::default()
                };
                bench.settings.compose_top();
                let command = if source == "gap" {
                    let gap = structure::separator("context-toggle");
                    bench.settings.top = vec![tests::button("inventory"), gap.clone(), tests::button("preferences")];
                    gap.command
                } else if source == "drawer" {
                    let id = drawers::create(&mut bench.settings, Bar::Top, &Layout::default());
                    bench.settings.drawers.get_mut(&id).expect("drawer").buttons = vec![tests::button("preferences")];
                    id
                } else {
                    source.into()
                };
                bench.state.open = editing;
                for _ in 0..5 {
                    bench.frame(Vec::new());
                }
                let point = if source == "background" {
                    let rect = bench
                        .ctx
                        .read_response(Id::unique("top_toolbar_background"))
                        .expect("bar background")
                        .rect;
                    rect.left_bottom() + Vec2::new(1.0, -1.0)
                } else {
                    bench
                        .ctx
                        .read_response(bars::tool_button_id(&command))
                        .expect("control")
                        .rect
                        .center()
                };
                right_click(&mut bench, point);
                for _ in 0..3 {
                    bench.frame(Vec::new());
                }
                let popup = native_context::tracked_popup(&bench.ctx)
                    .unwrap_or_else(|| panic!("context for {source}, classic={classic}, editing={editing}"));
                assert!(egui::Popup::is_id_open(&bench.ctx, popup));
                assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-add-spacer")));
                right_click(&mut bench, point + Vec2::new(1.0, 1.0));
                for _ in 0..2 {
                    bench.frame(Vec::new());
                }
                assert!(
                    !egui::Popup::is_id_open(&bench.ctx, popup),
                    "second click closes {source}, classic={classic}, editing={editing}"
                );
                assert!(matches!(bench.action, bars::BarAction::None));
                assert_eq!(bench.audio_action, crate::ui::audio::AudioAction::None);
                right_click(&mut bench, point);
                for _ in 0..2 {
                    bench.frame(Vec::new());
                }
                assert!(
                    egui::Popup::is_id_open(&bench.ctx, popup),
                    "a later click reopens {source}, classic={classic}, editing={editing}"
                );
            }
        }
    }
}

/// "Edit button…" of a right click, the bars not edited (human request,
/// 2026-11-26): its window gives the button a name, a display and an icon,
/// the icon picker beside the window, without entering customization nor
/// moving the button.
#[test]
fn the_edit_button_window_names_shows_and_draws_a_button_without_customization() {
    let layout = Layout::default();
    for classic in [false, true] {
        for bar in [Bar::Left, Bar::Right] {
            let mut bench = Bench {
                native_top: true,
                menu_open: false,
                classic,
                screen: Vec2::new(1920.0, 1080.0),
                ..Default::default()
            };
            let _icons = Icons::load(&bench.ctx, None);
            bench.settings.composed = true;
            bench.settings.buttons = Some(Vec::new());
            // a button saved with no name of its own
            bench.settings.set_bar(
                bar,
                vec![ToolButton {
                    tooltip: String::new(),
                    ..tests::button("inventory")
                }],
            );
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let placement = bench.settings.command_placement("inventory", &Layout::default());
            let point = bench
                .ctx
                .read_response(bars::tool_button_id("inventory"))
                .expect("inventory")
                .rect
                .center();
            right_click_at(&mut bench, point);
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            // no display choice nor icon row in the right click any more
            for key in ["toolbar-button-text", "toolbar-icon-change"] {
                assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)), "{key}");
            }
            let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-button-edit"));
            bench.pointer(edit, Some(true));
            bench.pointer(edit, Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let title = aurora_i18n::tr!("toolbar-button-edit-title");
            assert!(bench.text.iter().any(|(text, _)| text == title), "its window");
            // its display, "Inherited" first
            let inherited = menu_point(&bench, aurora_i18n::tr!("toolbar-button-inherited"));
            let text = menu_point(&bench, aurora_i18n::tr!("toolbar-button-text"));
            assert!(inherited.y < text.y);
            bench.pointer(text, Some(true));
            bench.pointer(text, Some(false));
            assert_eq!(bench.settings.appearance["inventory"].mode, Some(ButtonMode::TextOnly));
            // its name
            let name = bench
                .ctx
                .read_response(Id::unique(("button_name", "inventory")))
                .expect("its name field")
                .rect
                .center();
            bench.pointer(name, Some(true));
            bench.pointer(name, Some(false));
            bench.frame(vec![egui::Event::Text("Bag".into())]);
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let placed = bench.settings.placed_button("inventory", &layout).cloned().expect("placed");
            assert_eq!(placed.tooltip, "Bag");
            assert_eq!(super::label(&placed), "Bag");
            // its icon, the picker beside the window
            let icon = bench
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_icon_button_rect", "inventory"))))
                .expect("icon button");
            bench.pointer(icon.center(), Some(true));
            bench.pointer(icon.center(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let window = bench
                .ctx
                .memory(|memory| memory.area_rect(Id::unique("toolbar_button_appearance")))
                .expect("the window");
            let picker = bench
                .ctx
                .memory(|memory| memory.area_rect(Id::unique(("toolbar_icon_side_picker", "inventory"))))
                .expect("side picker");
            assert!(picker.width() <= 365.0 && bench.ctx.content_rect().contains_rect(picker));
            assert!(
                picker.left() >= window.right() || picker.right() <= window.left(),
                "the picker beside the window: {window:?}, {picker:?}"
            );
            let search = menu_point(&bench, aurora_i18n::tr!("toolbar-icon-search"));
            bench.pointer(search, Some(true));
            bench.pointer(search, Some(false));
            bench.frame(vec![egui::Event::Text("star".into())]);
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let choice = bench
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_icon_choice", "inventory", "star-filled"))))
                .expect("filtered star icon");
            bench.pointer(choice.center(), Some(true));
            bench.pointer(choice.center(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            assert_eq!(bench.settings.appearance["inventory"].icon.as_deref(), Some("star-filled"));
            assert_eq!(bench.settings.appearance["inventory"].mode, Some(ButtonMode::TextOnly));
            assert_eq!(bench.settings.command_placement("inventory", &Layout::default()), placement);
            assert!(!bench.state.open);
            assert!(matches!(bench.action, bars::BarAction::None));
            // emptied, the name is the default one again
            let name = bench
                .ctx
                .read_response(Id::unique(("button_name", "inventory")))
                .expect("its name field")
                .rect
                .center();
            bench.pointer(name, Some(true));
            bench.pointer(name, Some(false));
            for _ in 0..3 {
                bench.frame(vec![egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }]);
            }
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let placed = bench.settings.placed_button("inventory", &layout).cloned().expect("placed");
            assert!(placed.tooltip.is_empty(), "{:?}", placed.tooltip);
        }
    }
}

#[test]
fn normal_native_contexts_share_one_customization_entry_and_select_the_clicked_control() {
    for classic in [false, true] {
        for locked in [false, true] {
            for command in [
                "inventory",
                "aurora_menu",
                "report_bug",
                "bar_navigation",
                "bar_status",
                "bar_balance",
                "bar_clock",
                "bar_ping",
                "bar_fps",
                "bar_network",
                "bar_media",
                "bar_music",
                "bar_volume",
                "bar_notifications",
            ] {
                let mut bench = Bench {
                    native_top: true,
                    menu_open: false,
                    classic,
                    screen: Vec2::new(1920.0, 1080.0),
                    net_stats: true,
                    ..Default::default()
                };
                bench.settings.compose_top();
                bench.settings.locked = locked;
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let point = bench
                    .ctx
                    .read_response(bars::tool_button_id(command))
                    .unwrap_or_else(|| panic!("visible native control {command}, classic={classic}, locked={locked}"))
                    .rect
                    .center();
                let before = bench.settings.clone();
                for pressed in [true, false] {
                    bench.frame(vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]);
                }
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                assert!(!bench.state.open, "right click opens options, not customization: {command}");
                if command == "bar_status" {
                    assert!(
                        !bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("currency-menu")),
                        "status must not offer currency actions"
                    );
                }
                let entries: Vec<_> = bench
                    .text
                    .iter()
                    .filter(|(text, _)| text == aurora_i18n::tr!("toolbar-interface-edit"))
                    .collect();
                assert_eq!(entries.len(), 1, "one shared entry: {command}, classic={classic}, locked={locked}");
                // every right click has the same sections (human request,
                // 2026-11-26): a button's "Edit button…", no display choice
                // nor icon row of its own
                let heading = aurora_i18n::tr!("toolbar-button-settings").to_uppercase();
                let button_heading = menu_point(&bench, &heading);
                for key in ["toolbar-button-text", "toolbar-icon-change"] {
                    assert!(
                        !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
                        "{command}: {key}"
                    );
                }
                if matches!(command, "inventory" | "aurora_menu" | "report_bug") {
                    let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-button-edit"));
                    assert!(edit.y > button_heading.y && entries[0].1.top() > edit.y, "{command}");
                }
                // a bar's button and a bar's own control: its removal, then
                // its block's settings (the main menu's and the bug
                // report's are islands of their own in the Aurora style)
                if command == "inventory" || command.starts_with("bar_") {
                    let remove = menu_point(&bench, aurora_i18n::tr!("toolbar-remove"));
                    let block = menu_point(&bench, &aurora_i18n::tr!("toolbar-block-settings").to_uppercase());
                    let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-block-edit"));
                    let removal = menu_point(&bench, aurora_i18n::tr!("toolbar-block-remove"));
                    assert!(
                        button_heading.y < remove.y && remove.y < block.y && block.y < edit.y && edit.y < removal.y,
                        "{command}, classic={classic}, locked={locked}"
                    );
                    assert!(entries[0].1.top() > removal.y, "the shared entry last: {command}");
                }
                if command == "report_bug" {
                    let copy = menu_point(&bench, aurora_i18n::tr!("bug-copy-version"));
                    assert!(copy.y < button_heading.y, "its own action first");
                }
                for key in [
                    "toolbar-locked",
                    "toolbar-add-spacer",
                    "ui-bars-show-in-island",
                    "ui-bars-show-clock",
                    "ui-bars-show-fps",
                    "ui-bars-show-media",
                    "bug-toolbar",
                ] {
                    assert!(
                        !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
                        "layout choices stay in customization: {command}, {key}"
                    );
                }
                assert!(
                    matches!(bench.action, bars::BarAction::None),
                    "right click must not execute {command}"
                );
                let entry = entries[0].1.center();
                bench.pointer(entry, Some(true));
                bench.pointer(entry, Some(false));
                assert!(bench.state.open, "entry opens customization: {command}");
                assert_eq!(bench.state.selected.as_deref(), Some(command));
                assert_eq!(bench.settings, before, "opening customization never writes the layout");
            }
        }
    }
}

#[test]
fn bug_and_notification_context_actions_remain_explicit_and_functional() {
    for classic in [false, true] {
        for (command, key) in [("report_bug", "bug-copy-version"), ("bar_notifications", "common-do-not-disturb")] {
            let mut bench = Bench {
                native_top: true,
                menu_open: false,
                classic,
                screen: Vec2::new(1920.0, 1080.0),
                ..Default::default()
            };
            bench.settings.compose_top();
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let point = bench
                .ctx
                .read_response(bars::tool_button_id(command))
                .expect("button")
                .rect
                .center();
            for pressed in [true, false] {
                bench.frame(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let choice = menu_point(&bench, aurora_i18n::text(key));
            bench.pointer(choice, Some(true));
            bench.pointer(choice, Some(false));
            assert!(
                match command {
                    "report_bug" => matches!(bench.action, bars::BarAction::CopyVersion),
                    _ => matches!(bench.action, bars::BarAction::SetStatus(status) if status.dnd),
                },
                "explicit {key} still runs"
            );
            assert!(!bench.state.open);
        }
    }
}

#[test]
fn block_controls_stay_outside_buttons_and_highlight_the_whole_block() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for locked in [false, true] {
                let mut bench = block_controls_fixture(bar, classic, locked);
                // away from the block, no grip
                for _ in 0..2 {
                    bench.pointer(bench.screen.to_pos2() / 2.0, None);
                }
                assert!(
                    bench
                        .ctx
                        .read_response(Id::unique(("toolbar_block_grip", bar, "inventory")))
                        .is_none_or(|response| !response.rect.is_positive()),
                    "{bar:?}: the grip waits for the pointer"
                );
                let grip = block_grip(&mut bench, bar, "inventory");
                let body = bench
                    .ctx
                    .read_response(bars::tool_button_id("inventory"))
                    .expect("inventory")
                    .rect
                    .union(bench.ctx.read_response(bars::tool_button_id("people")).expect("people").rect);
                let controls = grip;
                assert!(
                    match bar {
                        Bar::Top => grip.top() >= body.bottom(),
                        Bar::Bottom => grip.bottom() <= body.top(),
                        Bar::Left => grip.left() >= body.right(),
                        Bar::Right => grip.right() <= body.left(),
                    },
                    "{bar:?}: {body:?}, {grip:?}"
                );
                for _ in 0..4 {
                    bench.pointer(grip.center(), None);
                }
                let current = bench
                    .ctx
                    .read_response(Id::unique(("toolbar_block_grip", bar, "inventory")))
                    .expect("hovered grip");
                let visible_body = bench
                    .ctx
                    .read_response(bars::tool_button_id("inventory"))
                    .expect("visible inventory")
                    .interact_rect
                    .union(
                        bench
                            .ctx
                            .read_response(bars::tool_button_id("people"))
                            .expect("visible people")
                            .interact_rect,
                    );
                let accent = crate::theme::Theme::default().palette().violet_light;
                fn outline(shape: &egui::Shape, body: Rect, color: egui::Color32) -> Option<Rect> {
                    match shape {
                        egui::Shape::Rect(rect) => (rect.rect.contains_rect(body) && rect.stroke.color == color).then_some(rect.rect),
                        egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| outline(shape, body, color)),
                        _ => None,
                    }
                }
                let highlight = bench
                    .shapes
                    .iter()
                    .find_map(|shape| outline(&shape.shape, visible_body, accent))
                    .unwrap_or_else(|| {
                        panic!(
                            "hover highlights the visible block: {bar:?}, classic={classic}, locked={locked}, body={visible_body:?}, grip={:?}, hovered={}, contains={}",
                            current.rect,
                            current.hovered(),
                            current.contains_pointer()
                        )
                    });
                // The visible frame includes padding clipped by its edge bar;
                // the raw button hit boxes need not share that frame's center.
                let center_offset = if bar.vertical() {
                    controls.center().y - highlight.center().y
                } else {
                    controls.center().x - highlight.center().x
                };
                assert!(
                    center_offset.abs() <= 1.0,
                    "controls stay centered on {bar:?}: {controls:?}, {highlight:?}"
                );
            }
        }
    }
}

#[test]
fn block_settings_remove_only_the_selected_block_and_respect_locks() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for locked in [false, true] {
                let mut bench = block_controls_fixture(bar, classic, locked);
                let before = bench.settings.clone();
                open_block_settings(&mut bench, bar, "inventory");
                assert_eq!(bench.settings, before, "opening settings never saves");
                let footer = menu_point(&bench, aurora_i18n::tr!("toolbar-interface-edit"));
                assert!(footer.y > menu_point(&bench, aurora_i18n::tr!("toolbar-locked")).y);
                let remove = menu_point(&bench, aurora_i18n::tr!("toolbar-block-remove"));
                bench.pointer(remove, Some(true));
                bench.pointer(remove, Some(false));
                if locked {
                    assert_eq!(bench.settings, before);
                } else {
                    for command in ["inventory", "people", "divider:inside"] {
                        assert!(bench.settings.location(command, &Layout::default()).is_none());
                    }
                    assert_eq!(bench.settings.location("preferences", &Layout::default()), Some(bar));
                }
            }
        }
    }
}

#[test]
fn old_school_text_menus_open_the_native_groups_and_dispatch_build_import() {
    let settings = presets::Preset::OldSchool.layout(&Layout::default());
    let menus = settings
        .top
        .iter()
        .filter(|button| button.command.starts_with(commands::PREFIX))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(menus.len(), 6);
    for menu in menus {
        let mut bench = Bench {
            settings: settings.clone(),
            native_top: true,
            menu_open: false,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        for _ in 0..5 {
            bench.frame(Vec::new());
        }
        assert!(bench.text.iter().any(|(text, _)| text == label(&menu)));
        let source = bench
            .ctx
            .read_response(bars::tool_button_id(&menu.command))
            .expect("text menu button")
            .rect
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source, Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        let catalogue = commands::catalogue(&bench.ctx);
        let child = catalogue
            .entries
            .iter()
            .find(|entry| entry.parent == menu.command && !entry.separator && !entry.info)
            .expect("native menu child");
        let rect = bench
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique(("main_menu_entry", &child.button.command))))
            .expect("opened native menu child");
        assert!(rect.is_positive() && bench.ctx.content_rect().intersects(rect));
        assert!(
            matches!(bench.action, bars::BarAction::None),
            "opening a group executes no leaf action"
        );
        if menu.command == "menu:common-build" {
            let import = bench
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("main_menu_entry", "menu:common-build/blender-live-title"))))
                .expect("native import action")
                .center();
            bench.pointer(import, Some(true));
            bench.pointer(import, Some(false));
            let ctx = bench.ctx.clone();
            let mut action = bars::BarAction::None;
            let p = crate::theme::Theme::default().palette();
            // The app dispatches queued menu requests after drawing. This
            // focused toolbar fixture normally leaves them for its caller.
            let _ = ctx.run_ui(Default::default(), |ui| {
                action = bars::dispatch_menu(ui, &p, &mut bench.panels, &bars::islands::tests::status(true, false));
            });
            assert!(matches!(action, bars::BarAction::BlenderLive));
            assert!(commands::take_request(&ctx).is_none(), "the native request is consumed once");
        }
    }
}

#[test]
fn photographer_controls_render_and_personal_lighting_uses_the_native_handler() {
    let mut bench = Bench {
        settings: presets::Preset::Photographer.layout(&Layout::default()),
        native_top: true,
        menu_open: false,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    for _ in 0..5 {
        bench.frame(Vec::new());
    }
    for command in [
        "snapshot",
        "view",
        "photo_roll",
        "immersive",
        "graphics_preferences",
        "quickprefs",
        "myenvironments",
    ] {
        let rect = bench
            .ctx
            .read_response(bars::tool_button_id(command))
            .expect("photography control")
            .rect;
        assert!(
            rect.is_positive() && bench.ctx.content_rect().contains_rect(rect),
            "visible control {command}"
        );
    }
    let lighting = "menu:common-world/common-environment/common-personal-lighting";
    let point = bench
        .ctx
        .read_response(bars::tool_button_id(lighting))
        .expect("personal lighting control")
        .rect
        .center();
    assert!(!bench.panels.personal_lighting);
    bench.pointer(point, Some(true));
    bench.pointer(point, Some(false));
    let ctx = bench.ctx.clone();
    let p = crate::theme::Theme::default().palette();
    let _ = ctx.run_ui(Default::default(), |ui| {
        bars::dispatch_menu(ui, &p, &mut bench.panels, &bars::islands::tests::status(true, false));
    });
    assert!(bench.panels.personal_lighting);
    assert!(commands::take_request(&ctx).is_none());
}

#[test]
fn editor_save_opens_the_selector_and_replaces_notecard_actions_in_both_styles() {
    for classic in [false, true] {
        for locked in [false, true] {
            let mut bench = block_controls_fixture(Bar::Bottom, classic, locked);
            assert!(!bench.state.notecards.active());
            for key in ["toolbar-notecard-save", "toolbar-notecard-load"] {
                assert!(
                    !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
                    "old action removed: {key}"
                );
            }
            let point = menu_point(&bench, aurora_i18n::tr!("common-save"));
            bench.pointer(point, Some(true));
            bench.pointer(point, Some(false));
            assert!(
                bench.state.notecards.active(),
                "saving remains available for a locked layout, classic={classic}"
            );
        }
    }
}

#[test]
fn editor_presets_queue_all_choices_and_selection_keeps_the_catalogue_stable() {
    for classic in [false, true] {
        for locked in [false, true] {
            for preset in presets::Preset::ALL {
                let mut bench = block_controls_fixture(Bar::Bottom, classic, locked);
                let before = bench.settings.clone();
                let point = menu_point(&bench, preset.label());
                bench.pointer(point, Some(true));
                bench.pointer(point, Some(false));
                assert_eq!(bench.state.preset_request, (!locked).then_some(preset));
                assert_eq!(bench.settings, before, "the app applies a queued preset atomically after drawing");
            }
        }
        let mut bench = block_controls_fixture(Bar::Bottom, classic, false);
        let divider = Id::unique(("toolbar_catalogue", structure::DIVIDER));
        let before = bench.ctx.data(|data| data.get_temp::<Rect>(divider)).expect("structural tile");
        let source = bench
            .ctx
            .read_response(bars::tool_button_id("inventory"))
            .expect("placed button")
            .rect
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source, Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert_eq!(bench.state.selected.as_deref(), Some("inventory"));
        let after = bench.ctx.data(|data| data.get_temp::<Rect>(divider)).expect("structural tile");
        assert_eq!(before, after, "selecting a control does not insert an action panel");
        for key in [
            "toolbar-button-edit",
            "toolbar-remove",
            "toolbar-edit-split-before",
            "toolbar-edit-split-after",
        ] {
            assert!(
                !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
                "selection has no action strip: {key}"
            );
        }
    }
}

/// The microphone's "Edit button" has its display, never an icon editor:
/// its glyph shows the voice's state.
#[test]
fn the_microphones_edit_button_keeps_its_display_and_no_icon_editor() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            menu_open: false,
            classic,
            screen: Vec2::new(1536.0, 864.0),
            ..Default::default()
        };
        bench.settings.composed = true;
        bench.settings.chat_composed = true;
        bench.settings.buttons = Some(vec![tests::button("speak")]);
        bench.settings.bottom_style = BarStyle {
            sizing: Some(Sizing::Auto),
            align: Some(Align::Center),
            ..Default::default()
        };
        for _ in 0..5 {
            bench.frame(Vec::new());
        }
        let point = bench
            .ctx
            .read_response(Id::unique("toolbar_background"))
            .expect("microphone bar")
            .rect
            .center();
        right_click_at(&mut bench, point);
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-icon-change")));
        let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-button-edit"));
        bench.pointer(edit, Some(true));
        bench.pointer(edit, Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        let text = menu_point(&bench, aurora_i18n::tr!("toolbar-button-text"));
        bench.pointer(text, Some(true));
        bench.pointer(text, Some(false));
        assert_eq!(bench.settings.appearance["speak"].mode, Some(ButtonMode::TextOnly));
        for key in ["toolbar-icon-search", "toolbar-icon-default", "toolbar-icon-change"] {
            assert!(
                !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
                "no microphone icon editor: {key}"
            );
        }
    }
}

#[test]
fn block_alignment_anchors_independently_on_every_edge_and_keeps_canonical_rects() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            let mut bench = block_controls_fixture(bar, classic, false);
            bench.settings.chat_composed = true;
            bench.settings.style_mut(bar).sizing = Some(Sizing::Auto);
            let configure = |bench: &mut Bench, command: &str, align| {
                bench.state.pending = Some(Edit::BlockStyle(
                    bar,
                    command.into(),
                    BarStyle {
                        align: Some(align),
                        ..Default::default()
                    },
                ));
                assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
                for _ in 0..5 {
                    bench.frame(Vec::new());
                }
            };
            configure(&mut bench, "inventory", Align::Start);
            configure(&mut bench, "preferences", Align::End);
            let bounds = |bench: &Bench| {
                let id = match bar {
                    Bar::Top => Id::unique("top_toolbar_background"),
                    Bar::Bottom => Id::unique("toolbar_background"),
                    Bar::Left => Id::unique(("toolbar_background", "toolbar_left")),
                    Bar::Right => Id::unique(("toolbar_background", "toolbar_right")),
                };
                bench.ctx.read_response(id).expect("bar bounds").rect
            };
            let group = |bench: &Bench| {
                bench
                    .ctx
                    .read_response(bars::tool_button_id("inventory"))
                    .expect("first")
                    .rect
                    .union(bench.ctx.read_response(bars::tool_button_id("people")).expect("last").rect)
            };
            let zone = bounds(&bench);
            let first = group(&bench);
            let last = bench
                .ctx
                .read_response(bars::tool_button_id("preferences"))
                .expect("other block")
                .rect;
            let start = |rect: Rect| if bar.vertical() { rect.top() } else { rect.left() };
            let end = |rect: Rect| if bar.vertical() { rect.bottom() } else { rect.right() };
            assert!(
                (start(first) - start(zone)).abs() < 12.0,
                "start: {bar:?}, classic={classic}: {first:?}, {zone:?}"
            );
            assert!(
                (end(last) - end(zone)).abs() < 12.0,
                "end: {bar:?}, classic={classic}: {last:?}, {zone:?}"
            );
            configure(&mut bench, "inventory", Align::Center);
            let first = group(&bench);
            let zone = bounds(&bench);
            assert!(
                ((start(first) + end(first)) / 2.0 - (start(zone) + end(zone)) / 2.0).abs() < 8.0,
                "center: {bar:?}, classic={classic}: {first:?}, {zone:?}"
            );
            assert_eq!(bench.settings.block_styles["preferences"].align, Some(Align::End));
            let restored: ToolbarSettings = serde_json::from_str(&serde_json::to_string(&bench.settings).expect("save")).expect("load");
            assert_eq!(restored, bench.settings);
        }
    }
}

#[test]
/// A block's right click leaves its display, sizing and alignment to its
/// "Edit block…" window, which it opens (human request, 2026-11-26), and
/// splits it at the chosen boundary.
fn block_right_clicks_leave_display_to_the_block_window_and_split_at_the_chosen_boundary() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            let mut bench = block_controls_fixture(bar, classic, false);
            let before = bench.settings.clone();
            open_block_settings(&mut bench, bar, "inventory");
            assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-edit-move")));
            let labels = mode_rows()
                .map(|(_, _, label)| label)
                .into_iter()
                .chain(sizing_rows().map(|(_, _, label)| label))
                .chain(align_rows(bar).map(|(_, _, label)| label));
            for label in labels {
                assert!(!bench.text.iter().any(|(text, _)| text == label), "{label}: in the block's window");
            }
            let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-block-edit"));
            bench.pointer(edit, Some(true));
            bench.pointer(edit, Some(false));
            assert!(
                bench
                    .state
                    .block_editor
                    .as_ref()
                    .is_some_and(|(at, command, _)| *at == bar && command == "inventory"),
                "{bar:?}, classic={classic}"
            );
            assert_eq!(bench.settings, before);
        }
    }
    {
        let mut bench = block_controls_fixture(Bar::Top, false, false);
        open_block_settings(&mut bench, Bar::Top, "inventory");
        let point = menu_point(&bench, aurora_i18n::tr!("toolbar-block-split"));
        for _ in 0..4 {
            bench.pointer(point, None);
        }
        let split_button = tests::button("people");
        let label = aurora_i18n::tr!("toolbar-block-split-before", button = label(&split_button));
        let choice = menu_point(&bench, &label);
        bench.pointer(choice, Some(true));
        bench.pointer(choice, Some(false));
        assert_eq!(edit_mode::group(&bench.settings.top, "inventory"), Some(0..2));
        assert_ne!(
            edit_mode::group(&bench.settings.top, "inventory"),
            edit_mode::group(&bench.settings.top, "people")
        );
    }
}

#[test]
fn drawer_context_unfolds_a_separate_block_without_entering_edit_mode() {
    for classic in [false, true] {
        for locked in [false, true] {
            let mut bench = Bench {
                native_top: true,
                menu_open: false,
                classic,
                screen: Vec2::new(1600.0, 900.0),
                ..Default::default()
            };
            bench.settings.composed = true;
            bench.settings.top = vec![
                tests::button("preferences"),
                tests::button("drawer:tools"),
                tests::button("inventory"),
            ];
            bench.settings.buttons = Some(Vec::new());
            bench.settings.drawers.insert(
                "drawer:tools".into(),
                drawers::Drawer {
                    buttons: vec![tests::button("people"), tests::button("worldmap")],
                    mode: ButtonMode::TextOnly,
                    ..Default::default()
                },
            );
            bench.settings.locked = locked;
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let before = bench.settings.clone();
            let point = bench
                .ctx
                .read_response(bars::tool_button_id("drawer:tools"))
                .expect("drawer toggle")
                .rect
                .center();
            for pressed in [true, false] {
                bench.frame(vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]);
            }
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let unfold = menu_point(&bench, aurora_i18n::tr!("toolbar-block-kind-fixed"));
            bench.pointer(unfold, Some(true));
            bench.pointer(unfold, Some(false));
            assert!(!bench.state.open);
            if locked {
                assert_eq!(bench.settings, before);
            } else {
                assert!(!bench.settings.drawers.contains_key("drawer:tools"));
                let range = edit_mode::group(&bench.settings.top, "people").expect("unfolded block");
                assert_eq!(bench.settings.top[range], [tests::button("people"), tests::button("worldmap")]);
                assert_eq!(bench.settings.block_styles["people"].mode, Some(ButtonMode::TextOnly));
                assert!(!bench.settings.block_styles.contains_key("preferences"));
                assert!(!bench.settings.block_styles.contains_key("inventory"));
            }
        }
    }
}

#[test]
fn customization_contexts_share_one_final_edit_interface_action_and_preserve_settings() {
    for classic in [false, true] {
        for locked in [false, true] {
            for source in ["inventory", "aurora_menu", "drawer", "menu"] {
                let mut bench = Bench {
                    native_top: source != "menu",
                    menu_open: true,
                    classic,
                    screen: Vec2::new(1600.0, 900.0),
                    ..Default::default()
                };
                bench.settings.compose_top();
                bench.settings.locked = locked;
                let drawer = if source == "drawer" {
                    Some(drawers::create(&mut bench.settings, Bar::Left, &Layout::default()))
                } else {
                    None
                };
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let command = drawer.as_deref().unwrap_or(if source == "menu" {
                    "menu:common-build/blender-live-title"
                } else {
                    source
                });
                let point = if source == "menu" {
                    bench
                        .ctx
                        .data(|data| data.get_temp::<Rect>(Id::unique(("main_menu_entry", command))))
                        .expect("menu command")
                        .center()
                } else {
                    bench
                        .ctx
                        .read_response(bars::tool_button_id(command))
                        .expect("button")
                        .rect
                        .center()
                };
                let before = bench.settings.clone();
                for pressed in [true, false] {
                    bench.frame(vec![
                        egui::Event::PointerMoved(point),
                        egui::Event::PointerButton {
                            pos: point,
                            button: egui::PointerButton::Secondary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]);
                }
                for _ in 0..3 {
                    bench.frame(Vec::new());
                }
                let preceding = aurora_i18n::text(match source {
                    "drawer" => "toolbar-block-edit",
                    _ => "toolbar-button-edit",
                });
                let preceding = bench.text.iter().rev().find(|(text, _)| text == preceding).expect("context row").1;
                // The Aurora menu remains behind its context menu and also owns
                // an Edit interface command. Count only this popup's text column.
                let rows: Vec<_> = bench
                    .text
                    .iter()
                    .filter(|(text, rect)| {
                        text == aurora_i18n::tr!("toolbar-interface-edit") && (rect.left() - preceding.left()).abs() < 0.5
                    })
                    .collect();
                assert_eq!(rows.len(), 1, "{source}, classic={classic}, locked={locked}");
                let footer = rows[0].1;
                assert!(footer.top() >= preceding.bottom(), "the edit action must be last: {source}");
                bench.pointer(footer.center(), Some(true));
                bench.pointer(footer.center(), Some(false));
                assert!(bench.state.open);
                assert_eq!(bench.settings, before);
                assert!(commands::take_request(&bench.ctx).is_none());
            }
        }
    }
}

#[test]
fn block_handles_move_all_members_on_every_edge_in_both_styles() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            let mut bench = Bench {
                native_top: true,
                classic,
                screen: Vec2::new(1600.0, 900.0),
                ..Default::default()
            };
            bench.settings.composed = true;
            bench.settings.buttons = Some(Vec::new());
            bench.settings.top = vec![tests::button("inventory"), structure::divider("inside"), tests::button("people")];
            bench.state.open = true;
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let source = block_grip(&mut bench, Bar::Top, "inventory").center();
            let target = match bar {
                Bar::Top => egui::pos2(420.0, 20.0),
                Bar::Bottom => egui::pos2(1300.0, 880.0),
                Bar::Left => egui::pos2(16.0, 680.0),
                Bar::Right => egui::pos2(1584.0, 680.0),
            };
            let before = bench.settings.clone();
            bench.pointer(source, Some(true));
            bench.pointer(source + Vec2::new(12.0, 0.0), None);
            assert_eq!(
                egui::DragAndDrop::payload::<Drag>(&bench.ctx).expect("whole payload").members.len(),
                3
            );
            for _ in 0..4 {
                bench.pointer(target, None);
            }
            assert_eq!(bench.settings, before, "preview is not saved");
            assert_eq!(
                drop_preview::current(&bench.ctx)
                    .unwrap_or_else(|| panic!(
                        "missing block preview on {bar:?}, classic={classic}, target={target:?}, layer={:?}, dock={:?}",
                        bench.ctx.layer_id_at(target),
                        bench
                            .ctx
                            .read_response(egui::Id::unique("dock_drop"))
                            .map(|response| (response.rect, response.layer_id))
                    ))
                    .bar,
                bar
            );
            let snapped = bench
                .ctx
                .read_response(bars::tool_button_id("inventory"))
                .expect("snapped block")
                .rect;
            bench.pointer(target, Some(false));
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let placed = bench
                .ctx
                .read_response(bars::tool_button_id("inventory"))
                .expect("placed block")
                .rect;
            assert!(
                (snapped.min - placed.min).length() < 0.5 && (snapped.max - placed.max).length() < 0.5,
                "{bar:?}, classic={classic}: snapped={snapped:?}, placed={placed:?}"
            );
            let layout = Layout::default();
            let list = bench.settings.bar(bar, &layout);
            assert_eq!(
                list.iter().map(|button| button.command.as_str()).collect::<Vec<_>>(),
                ["inventory", "divider:inside", "people"],
                "{bar:?} classic={classic}"
            );
            let divider = bench
                .ctx
                .read_response(bars::tool_button_id("divider:inside"))
                .expect("divider")
                .rect;
            assert!((if bar.vertical() { divider.height() } else { divider.width() } - structure::DIVIDER_SIZE).abs() < 0.5);
            assert_eq!(
                edit_mode::group(list, "people"),
                Some(0..3),
                "the internal divider does not split the block"
            );
        }
    }
}

#[test]
fn placed_button_drag_out_shows_removal_and_removes_while_catalogue_copies_cancel() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            classic,
            ..Default::default()
        };
        bench.settings.composed = true;
        bench.settings.buttons = Some(Vec::new());
        bench.settings.top = vec![tests::button("inventory")];
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let source = bench
            .ctx
            .read_response(bars::tool_button_id("inventory"))
            .expect("source")
            .rect
            .center();
        let outside = egui::pos2(750.0, 400.0);
        bench.pointer(source, Some(true));
        bench.pointer(source + Vec2::new(12.0, 0.0), None);
        for _ in 0..3 {
            bench.pointer(outside, None);
        }
        assert!(
            bench
                .ctx
                .data(|data| data.get_temp::<bool>(egui::Id::unique("toolbar_drag_remove")))
                .unwrap_or(false)
        );
        assert!(bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-remove")));
        bench.pointer(outside, Some(false));
        assert_eq!(bench.settings.location("inventory", &Layout::default()), None);
        bench.state.open = true;
        bench.state.search = catalogue::find("inventory").expect("catalogue").label().into();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let tile = bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", "inventory"))))
            .expect("tile")
            .center();
        bench.pointer(tile, Some(true));
        bench.pointer(tile + Vec2::new(12.0, 0.0), None);
        for _ in 0..3 {
            bench.pointer(outside, None);
        }
        assert!(
            !bench
                .ctx
                .data(|data| data.get_temp::<bool>(egui::Id::unique("toolbar_drag_remove")))
                .unwrap_or(false)
        );
        bench.pointer(outside, Some(false));
        assert!(bench.settings.top.is_empty());
    }
}

#[test]
fn slurl_button_popup_is_anchored_and_keeps_integrated_navigation_placement() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            let mut bench = Bench {
                native_top: true,
                classic,
                screen: Vec2::new(1600.0, 900.0),
                ..Default::default()
            };
            bench.settings.compose_top();
            bench.state.pending = Some(Edit::Insert(bar, bars::top::button("location_bar", "map-pin-filled"), 0));
            bench.state.commit(&Layout::default(), &mut bench.settings);
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let before = bench.settings.clone();
            let anchor = bench.ctx.read_response(bars::tool_button_id("location_bar")).expect("toggle").rect;
            bench.pointer(anchor.center(), Some(true));
            bench.pointer(anchor.center(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            assert!(bars::location_popup::is_open(&bench.ctx), "{bar:?} classic={classic}");
            assert_eq!(bench.settings, before);
            assert!(
                bench.panels.nav_edit.is_none(),
                "popup editing does not activate the integrated field"
            );
            let field = bench
                .ctx
                .read_response(bars::location_popup::field_id())
                .expect("independent SLURL field")
                .rect;
            assert!(
                match bar {
                    Bar::Top => field.top() > anchor.bottom(),
                    Bar::Bottom => field.bottom() < anchor.top(),
                    Bar::Left => field.left() > anchor.right(),
                    Bar::Right => field.right() < anchor.left(),
                },
                "{bar:?}: {anchor:?}, {field:?}"
            );
            assert!(bench.ctx.content_rect().contains_rect(field));
            bench.pointer(anchor.center(), Some(true));
            bench.pointer(anchor.center(), Some(false));
            assert!(!bars::location_popup::is_open(&bench.ctx));
            assert_eq!(bench.settings, before);
            bench.location_hidden = true;
            bench.pointer(anchor.center(), Some(true));
            bench.pointer(anchor.center(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            // @showloc keeps the field, on the hidden SLURL, like the
            // navigation bar's
            assert!(bars::location_popup::is_open(&bench.ctx));
            assert!(bench.ctx.read_response(bars::location_popup::field_id()).is_some());
        }
    }
}

fn drawer_target(classic: bool, bar: Bar, editing: bool, open: bool) -> (Bench, String) {
    let mut bench = Bench {
        native_top: true,
        classic,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.buttons = Some(Vec::new());
    bench.settings.top = vec![tests::button("inventory")];
    let id = drawers::create(&mut bench.settings, bar, &Layout::default());
    bench.settings.drawers.get_mut(&id).expect("drawer").buttons = vec![tests::button("preferences")];
    bench.state.open = editing;
    drawers::set_open(&bench.ctx, &id, open);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    (bench, id)
}

fn start_toolbar_drag(bench: &mut Bench, command: &str) {
    let source = bench
        .ctx
        .read_response(bars::tool_button_id(command))
        .expect("source")
        .rect
        .center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    assert!(ToolbarUi::dragging(&bench.ctx), "drag {command}");
}

fn move_gradually(bench: &mut Bench, target: Pos2) {
    let start = bench.ctx.pointer_interact_pos().expect("drag pointer");
    let steps = ((target - start).length() / 8.0).ceil() as usize;
    for step in 1..=steps.max(1) {
        bench.pointer(start.lerp(target, step as f32 / steps.max(1) as f32), None);
    }
}

#[test]
fn button_faces_create_drawers_with_the_targets_effective_display_on_every_edge() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for override_mode in [None, Some(ButtonMode::TextOnly)] {
                let mut bench = Bench {
                    native_top: true,
                    classic,
                    screen: Vec2::new(1600.0, 900.0),
                    ..Default::default()
                };
                bench.settings.composed = true;
                bench.settings.buttons = Some(Vec::new());
                bench.settings.top = vec![tests::button("inventory")];
                let mut list = bench.settings.bar(bar, &Layout::default()).to_vec();
                list.push(tests::button("people"));
                bench.settings.set_bar(bar, list);
                bench.settings.style_mut(bar).mode = Some(ButtonMode::IconsWithText);
                bench.settings.appearance.insert(
                    "people".into(),
                    appearance::Appearance {
                        mode: override_mode,
                        icon: Some("star-filled".into()),
                    },
                );
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let target = bench
                    .ctx
                    .read_response(bars::tool_button_id("people"))
                    .expect("target")
                    .rect
                    .center();
                let before = bench.settings.clone();
                start_toolbar_drag(&mut bench, "inventory");
                move_gradually(&mut bench, target);
                for _ in 0..4 {
                    bench.pointer(target, None);
                }
                assert_eq!(bench.settings, before, "grouping hover never saves");
                assert!(drop_preview::current(&bench.ctx).is_none());
                bench.pointer(target, Some(false));
                let id = bench
                    .settings
                    .drawer_of("inventory")
                    .unwrap_or_else(|| panic!("missing group on {bar:?}, classic={classic}"));
                assert_eq!(bench.settings.drawer_of("people"), Some(id));
                assert_eq!(bench.settings.location(id, &Layout::default()), Some(bar));
                assert_eq!(bench.settings.drawers[id].mode, override_mode.unwrap_or(ButtonMode::IconsWithText));
                assert_eq!(
                    bench.settings.drawers[id].buttons,
                    [tests::button("people"), tests::button("inventory")]
                );
                assert_eq!(bench.settings.appearance["people"].icon.as_deref(), Some("star-filled"));
            }
        }
    }
}

#[test]
fn drawer_faces_and_grouping_accept_menu_sources_and_last_children() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    for group in [false, true] {
        for source in ["menu", "drawer", "catalogue"] {
            let (mut bench, id) = drawer_target(false, Bar::Right, false, false);
            bench.settings.top.clear();
            let target_command = if group {
                bench.settings.right = vec![tests::button("people")];
                bench.settings.drawers.remove(&id);
                "people".to_owned()
            } else {
                id.clone()
            };
            let from = if source == "drawer" {
                let from = drawers::create(&mut bench.settings, Bar::Left, &Layout::default());
                bench.settings.drawers.get_mut(&from).expect("source").buttons = vec![tests::button("inventory")];
                drawers::set_open(&bench.ctx, &from, true);
                Some(from)
            } else {
                None
            };
            if source == "menu" {
                bench.native_top = false;
            }
            if source == "catalogue" {
                bench.state.open = true;
                bench.state.search = catalogue::find("inventory").expect("command").label().into();
            }
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let target = bench
                .ctx
                .read_response(bars::tool_button_id(&target_command))
                .expect("target")
                .rect
                .center();
            let command = if source == "menu" { IMPORT } else { "inventory" };
            let start = match source {
                "menu" => bench
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique(("main_menu_entry", IMPORT))))
                    .expect("menu row")
                    .center(),
                "catalogue" => bench
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_catalogue", "inventory"))))
                    .expect("catalogue tile")
                    .center(),
                _ => bench
                    .ctx
                    .read_response(bars::tool_button_id("inventory"))
                    .expect("drawer child")
                    .rect
                    .center(),
            };
            bench.pointer(start, Some(true));
            bench.pointer(start + Vec2::new(12.0, 0.0), None);
            assert!(ToolbarUi::dragging(&bench.ctx));
            move_gradually(&mut bench, target);
            for _ in 0..6 {
                bench.pointer(target, None);
            }
            bench.pointer(target, Some(false));
            let destination = bench
                .settings
                .drawer_of(command)
                .unwrap_or_else(|| panic!("{source}, grouping={group}"));
            if group {
                assert_eq!(bench.settings.drawer_of("people"), Some(destination));
            } else {
                assert_eq!(destination, id);
            }
            if let Some(from) = from {
                assert!(!bench.settings.drawers.contains_key(&from));
                assert!(bench.settings.location(&from, &Layout::default()).is_none());
            }
        }
    }
}

#[test]
fn toolbar_reordering_has_wide_insertion_corridors_and_requires_a_pause_to_group() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for mode in [ButtonMode::IconsOnly, ButtonMode::TextOnly] {
                for gesture in [
                    "edge",
                    "trailing-edge",
                    "gap",
                    "quick-centre",
                    "release-at-deadline",
                    "leave-centre",
                    "return-to-centre",
                ] {
                    let mut bench = Bench {
                        native_top: true,
                        classic,
                        screen: Vec2::new(1600.0, 900.0),
                        ..Default::default()
                    };
                    bench.settings.composed = true;
                    bench.settings.buttons = Some(Vec::new());
                    bench
                        .settings
                        .set_bar(bar, ["inventory", "people", "preferences"].map(tests::button).to_vec());
                    bench.settings.style_mut(bar).mode = Some(mode);
                    for _ in 0..4 {
                        bench.frame(Vec::new());
                    }
                    let target = bench
                        .ctx
                        .read_response(bars::tool_button_id("preferences"))
                        .expect("last neighbour")
                        .rect;
                    let mut edge = if bar.vertical() {
                        egui::pos2(target.center().x, target.top() + target.height() * 0.2)
                    } else {
                        egui::pos2(target.left() + target.width() * 0.2, target.center().y)
                    };
                    if matches!(gesture, "trailing-edge" | "gap") {
                        let preceding = bench
                            .ctx
                            .read_response(bars::tool_button_id("people"))
                            .expect("first neighbour")
                            .rect;
                        edge = if bar.vertical() {
                            egui::pos2(
                                preceding.center().x,
                                if gesture == "gap" {
                                    (preceding.bottom() + target.top()) / 2.0
                                } else {
                                    preceding.top() + preceding.height() * 0.8
                                },
                            )
                        } else {
                            egui::pos2(
                                if gesture == "gap" {
                                    (preceding.right() + target.left()) / 2.0
                                } else {
                                    preceding.left() + preceding.width() * 0.8
                                },
                                preceding.center().y,
                            )
                        };
                    }
                    let before = bench.settings.clone();
                    start_toolbar_drag(&mut bench, "inventory");
                    let point = if matches!(gesture, "quick-centre" | "release-at-deadline") {
                        bench.pointer(target.center(), None);
                        target.center()
                    } else {
                        if matches!(gesture, "leave-centre" | "return-to-centre") {
                            for _ in 0..6 {
                                bench.pointer(target.center(), None);
                            }
                            assert!(drop_preview::current(&bench.ctx).is_none(), "grouping is armed");
                        }
                        for _ in 0..6 {
                            bench.pointer(edge, None);
                        }
                        if gesture == "return-to-centre" {
                            bench.pointer(target.center(), None);
                            target.center()
                        } else {
                            edge
                        }
                    };
                    assert_eq!(bench.settings, before, "hover never persists an edit");
                    assert!(
                        drop_preview::current(&bench.ctx).is_some(),
                        "insertion preview: {gesture}, {bar:?}, {classic}, {mode:?}"
                    );
                    if gesture == "release-at-deadline" {
                        bench.time += 1.0;
                    }
                    bench.pointer(point, Some(false));
                    assert!(
                        bench.settings.drawers.is_empty(),
                        "unexpected folder: {gesture}, {bar:?}, {classic}, {mode:?}"
                    );
                    let expected = if matches!(gesture, "quick-centre" | "release-at-deadline" | "return-to-centre") {
                        ["people", "preferences", "inventory"]
                    } else {
                        ["people", "inventory", "preferences"]
                    };
                    assert_eq!(
                        bench
                            .settings
                            .bar(bar, &Layout::default())
                            .iter()
                            .map(|button| button.command.as_str())
                            .collect::<Vec<_>>(),
                        expected,
                        "{gesture}, {bar:?}, {classic}, {mode:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn drawer_icons_accept_drops_on_every_edge_open_or_closed_in_both_styles_and_edit_modes() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for editing in [false, true] {
                for open in [false, true] {
                    let (mut bench, id) = drawer_target(classic, bar, editing, open);
                    let point = bench.ctx.read_response(bars::tool_button_id(&id)).expect("toggle").rect.center();
                    let before = bench.settings.clone();
                    start_toolbar_drag(&mut bench, "inventory");
                    move_gradually(&mut bench, point);
                    // its centre takes the drop after the wait a button's
                    // takes to group (human request, 2026-11-25)
                    for _ in 0..6 {
                        bench.pointer(point, None);
                    }
                    assert_eq!(bench.settings, before, "hover must not save");
                    assert_eq!(drawers::is_open(&bench.ctx, &id), open, "hover must not toggle the drawer");
                    assert!(
                        drop_preview::current(&bench.ctx).is_none(),
                        "icon drop takes priority over bar reordering"
                    );
                    let target = bench.ctx.read_response(bars::tool_button_id(&id)).expect("highlighted toggle").rect;
                    let accent = crate::theme::Theme::default().palette().violet_light;
                    assert!(bench.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == target && rect.stroke.width == 2.0 && rect.stroke.color == accent)), "visible drawer acceptance on {bar:?}, classic={classic}, editing={editing}, open={open}");
                    bench.pointer(point, Some(false));
                    assert_eq!(
                        bench.settings.drawer_of("inventory"),
                        Some(id.as_str()),
                        "{bar:?}, classic={classic}, editing={editing}, open={open}"
                    );
                    assert_eq!(
                        bench.settings.drawers[&id]
                            .buttons
                            .iter()
                            .map(|button| button.command.as_str())
                            .collect::<Vec<_>>(),
                        ["preferences", "inventory"]
                    );
                    assert!(!Bar::ALL.into_iter().any(|bar| {
                        bench
                            .settings
                            .bar(bar, &Layout::default())
                            .iter()
                            .any(|button| button.command == "inventory")
                    }));
                    assert_eq!(drawers::is_open(&bench.ctx, &id), open);
                }
            }
        }
    }
}

#[test]
fn drawer_icon_drops_accept_catalogue_commands_native_controls_and_independent_dividers() {
    for command in [
        "inventory",
        "menu:common-build/ui-bars-save-to-object-contents",
        "bar_fps",
        "bar_chat",
        structure::DIVIDER,
    ] {
        let (mut bench, id) = drawer_target(false, Bar::Right, true, false);
        bench.settings.top.clear();
        bench.state.search = commands::catalogue(&bench.ctx).find(command).map_or_else(
            || label(&bars::top::button(command, "")).to_owned(),
            |entry| label(&entry.button).to_owned(),
        );
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let source = bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", command))))
            .unwrap_or_else(|| panic!("catalogue source {command}, search={}", bench.state.search))
            .center();
        let target = bench
            .ctx
            .read_response(bars::tool_button_id(&id))
            .expect("drawer icon")
            .rect
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source + Vec2::new(12.0, 0.0), None);
        assert!(ToolbarUi::dragging(&bench.ctx), "{command}");
        let before = bench.settings.clone();
        for _ in 0..6 {
            bench.pointer(target, None);
        }
        assert_eq!(bench.settings, before);
        bench.pointer(target, Some(false));
        let added = bench.settings.drawers[&id].buttons.last().expect("inserted control");
        assert!(
            if command == structure::DIVIDER {
                added.command.starts_with(structure::DIVIDER_PREFIX)
            } else {
                added.command == command
            },
            "{command}"
        );
        assert!(!drawers::is_open(&bench.ctx, &id));
    }
}

#[test]
fn drawer_icon_drops_move_children_and_whole_blocks_without_losing_order_or_overrides() {
    let (mut bench, target) = drawer_target(false, Bar::Left, true, false);
    bench.settings.top.clear();
    let source = drawers::create(&mut bench.settings, Bar::Right, &Layout::default());
    bench.settings.drawers.get_mut(&source).expect("source drawer").buttons = vec![tests::button("inventory"), tests::button("people")];
    bench.settings.appearance.insert(
        "inventory".into(),
        appearance::Appearance {
            mode: Some(ButtonMode::TextOnly),
            icon: Some("star-filled".into()),
        },
    );
    drawers::set_open(&bench.ctx, &source, true);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let point = bench
        .ctx
        .read_response(bars::tool_button_id(&target))
        .expect("target")
        .rect
        .center();
    start_toolbar_drag(&mut bench, "inventory");
    for _ in 0..6 {
        bench.pointer(point, None);
    }
    bench.pointer(point, Some(false));
    assert_eq!(bench.settings.drawer_of("inventory"), Some(target.as_str()));
    assert_eq!(bench.settings.drawers[&source].buttons, vec![tests::button("people")]);
    assert_eq!(bench.settings.appearance["inventory"].mode, Some(ButtonMode::TextOnly));
    assert_eq!(bench.settings.appearance["inventory"].icon.as_deref(), Some("star-filled"));

    let (mut bench, id) = drawer_target(false, Bar::Right, true, true);
    bench.settings.top.clear();
    bench.settings.drawers.get_mut(&id).expect("drawer").buttons =
        vec![tests::button("inventory"), tests::button("people"), tests::button("preferences")];
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    start_toolbar_drag(&mut bench, "inventory");
    for _ in 0..4 {
        bench.pointer(egui::pos2(16.0, 680.0), None);
    }
    assert!(drop_preview::current(&bench.ctx).is_some());
    let target = bench.ctx.read_response(bars::tool_button_id(&id)).expect("toggle").rect.center();
    for _ in 0..6 {
        bench.pointer(target, None);
    }
    bench.pointer(target, Some(false));
    assert_eq!(
        bench.settings.drawers[&id]
            .buttons
            .iter()
            .map(|button| button.command.as_str())
            .collect::<Vec<_>>(),
        ["people", "preferences", "inventory"],
        "an existing child appends using the saved rank after a root preview"
    );

    let (mut bench, id) = drawer_target(true, Bar::Right, true, false);
    let members = vec![tests::button("inventory"), structure::divider("block"), tests::button("people")];
    bench.settings.top = members.clone();
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let handle = block_grip(&mut bench, Bar::Top, "inventory").center();
    let target = bench.ctx.read_response(bars::tool_button_id(&id)).expect("target").rect.center();
    bench.pointer(handle, Some(true));
    bench.pointer(handle + Vec2::new(12.0, 0.0), None);
    assert_eq!(egui::DragAndDrop::payload::<Drag>(&bench.ctx).expect("block drag").members, members);
    for _ in 0..6 {
        bench.pointer(target, None);
    }
    bench.pointer(target, Some(false));
    assert_eq!(bench.settings.drawers[&id].buttons[1..], members);
    assert!(bench.settings.top.is_empty());
}

#[test]
fn drawer_icon_drops_reject_locks_self_and_nested_drawers_without_reordering_or_removal() {
    for case in ["locked", "self", "nested"] {
        let (mut bench, target) = drawer_target(false, Bar::Right, true, false);
        let source = if case == "self" {
            target.clone()
        } else if case == "nested" {
            drawers::create(&mut bench.settings, Bar::Top, &Layout::default())
        } else {
            "inventory".into()
        };
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let point = bench
            .ctx
            .read_response(bars::tool_button_id(&target))
            .expect("toggle")
            .rect
            .center();
        start_toolbar_drag(&mut bench, &source);
        if case == "locked" {
            bench.settings.locked = true;
        }
        let before = bench.settings.clone();
        for _ in 0..3 {
            bench.pointer(point, None);
        }
        bench.pointer(point, Some(false));
        if case == "nested" {
            // a drawer goes into no drawer: on its centre it moves beside
            // it, as a button does (human request, 2026-11-25)
            assert_eq!(bench.settings.drawer_of(&source), None);
            assert_eq!(bench.settings.drawers[&target].buttons, before.drawers[&target].buttons);
            assert!(bench.settings.drawers.contains_key(&source));
            assert_eq!(bench.settings.location(&source, &Layout::default()), Some(Bar::Right));
        } else {
            assert_eq!(bench.settings, before, "{case}");
        }
        assert!(!drawers::is_open(&bench.ctx, &target));
    }
}

#[test]
fn drawer_icon_hover_survives_clearing_a_bar_preview_and_cancels_outside_the_viewport() {
    for cancel in [false, true] {
        let (mut bench, id) = drawer_target(true, Bar::Right, true, false);
        let neighbour = drawers::create(&mut bench.settings, Bar::Right, &Layout::default());
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        start_toolbar_drag(&mut bench, "inventory");
        let anchor = bench.ctx.read_response(bars::tool_button_id(&id)).expect("toggle").rect;
        for _ in 0..4 {
            bench.pointer(anchor.center() - Vec2::new(0.0, 60.0), None);
        }
        assert!(drop_preview::current(&bench.ctx).is_some(), "start with a root-bar preview");
        let target = anchor.center();
        let before = bench.settings.clone();
        for _ in 0..5 {
            bench.pointer(target, None);
        }
        assert!(drop_preview::current(&bench.ctx).is_none());
        assert_eq!(bench.settings, before);
        let accent = crate::theme::Theme::default().palette().violet_light;
        let neighbour = bench.ctx.read_response(bars::tool_button_id(&neighbour)).expect("neighbour").rect;
        assert!(!bench.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == neighbour && rect.stroke.width == 2.0 && rect.stroke.color == accent)), "a shifted neighbour must not advertise a second drawer target");
        let release = if cancel { egui::pos2(-20.0, 400.0) } else { target };
        bench.pointer(release, Some(false));
        if cancel {
            assert_eq!(bench.settings, before);
        } else {
            assert_eq!(bench.settings.drawer_of("inventory"), Some(id.as_str()));
        }
    }
}

#[test]
fn builder_save_back_uses_the_native_action_and_rechecks_selection_on_dispatch() {
    const COMMAND: &str = "menu:common-build/ui-bars-save-to-object-contents";
    for classic in [false, true] {
        for (enabled, dispatch_enabled) in [(false, false), (true, true), (true, false)] {
            let mut bench = Bench {
                settings: presets::Preset::Build.layout(&Layout::default()),
                native_top: true,
                menu_open: false,
                classic,
                screen: Vec2::new(1920.0, 1080.0),
                save_to_contents: enabled,
                ..Default::default()
            };
            for _ in 0..5 {
                bench.frame(Vec::new());
            }
            assert_eq!(commands::available(&bench.ctx, COMMAND), enabled);
            let button = bench
                .ctx
                .read_response(bars::tool_button_id(COMMAND))
                .expect("Builder Save back button");
            // Unavailable toolbar controls retain their drag target. Their
            // live availability gates execution rather than egui interaction.
            let point = button.rect.center();
            bench.pointer(point, Some(true));
            bench.pointer(point, Some(false));
            let ctx = bench.ctx.clone();
            let p = crate::theme::Theme::default().palette();
            let mut status = bars::islands::tests::status(true, false);
            status.save_to_contents = dispatch_enabled;
            let mut action = bars::BarAction::None;
            let _ = ctx.run_ui(Default::default(), |ui| {
                action = bars::dispatch_menu(ui, &p, &mut bench.panels, &status);
            });
            assert_eq!(matches!(action, bars::BarAction::SaveToObjectContents), enabled && dispatch_enabled);
            assert!(matches!(action, bars::BarAction::None | bars::BarAction::SaveToObjectContents));
        }
    }
}

struct Bench {
    ctx: egui::Context,
    state: ToolbarUi,
    settings: ToolbarSettings,
    panels: Panels,
    menu_open: bool,
    native_top: bool,
    classic: bool,
    time: f64,
    text: Vec<(String, Rect)>,
    shapes: Vec<egui::epaint::ClippedShape>,
    screen: Vec2,
    notif: crate::ui::notifications::NotifUi,
    conversation_count: usize,
    audio: crate::settings::AudioSettings,
    audio_ui: crate::ui::audio::AudioUi,
    media: crate::ui::audio::MediaState,
    audio_action: crate::ui::audio::AudioAction,
    action: bars::BarAction,
    location_hidden: bool,
    net_stats: bool,
    save_to_contents: bool,
}

impl Default for Bench {
    fn default() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let settings = ToolbarSettings {
            top: Vec::new(),
            menu_layout: [(String::new(), vec!["menu:common-build/blender-live-title".into()])].into(),
            ..Default::default()
        };
        Self {
            ctx,
            state: ToolbarUi::default(),
            settings,
            panels: Panels::from_settings(&crate::settings::Settings::default()),
            menu_open: true,
            native_top: false,
            classic: false,
            time: 0.0,
            text: Vec::new(),
            shapes: Vec::new(),
            screen: Vec2::new(1000.0, 700.0),
            notif: Default::default(),
            conversation_count: 0,
            audio: Default::default(),
            audio_ui: Default::default(),
            media: Default::default(),
            audio_action: crate::ui::audio::AudioAction::None,
            action: bars::BarAction::None,
            location_hidden: false,
            net_stats: false,
            save_to_contents: false,
        }
    }
}

impl Bench {
    fn palette_drag(&mut self, command: &str, bar: Bar) {
        self.state.open = true;
        self.state.search = catalogue::find(command).map_or_else(
            || label(&bars::top::button(command, "")).to_string(),
            |command| command.label().to_string(),
        );
        self.state.category = None;
        for _ in 0..4 {
            self.frame(Vec::new());
        }
        let source = self
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", command))))
            .expect("catalogue source")
            .center();
        self.pointer(source, Some(true));
        self.pointer(source + Vec2::new(12.0, 0.0), None);
        assert!(ToolbarUi::dragging(&self.ctx), "source {command}");
        let anchor = self
            .settings
            .bar(bar, &Layout::default())
            .iter()
            .filter(|button| !structure::is_separator(&button.command))
            .find_map(|button| self.ctx.read_response(bars::tool_button_id(&button.command)));
        let target = anchor.map_or_else(
            || match bar {
                Bar::Top => egui::pos2(20.0, 26.0),
                Bar::Left => egui::pos2(16.0, self.screen.y / 2.0),
                Bar::Right => egui::pos2(self.screen.x - 16.0, self.screen.y / 2.0),
                Bar::Bottom => egui::pos2(self.screen.x / 2.0, self.screen.y - 20.0),
            },
            |response| {
                if bar.vertical() {
                    egui::pos2(response.rect.center().x, response.rect.top() + 1.0)
                } else {
                    egui::pos2(response.rect.left() + 1.0, response.rect.center().y)
                }
            },
        );
        for _ in 0..4 {
            self.pointer(target, None);
        }
        self.pointer(target, Some(false));
        self.frame(Vec::new());
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let ctx = self.ctx.clone();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let layout = Layout::default();
        let mut status = bars::islands::tests::status(true, false);
        status.classic_bars = self.classic;
        status.location_hidden = self.location_hidden;
        status.conversation_thumbnails = self.conversation_count;
        status.net_stats = self.net_stats;
        status.save_to_contents = self.save_to_contents;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.screen)),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                bars::collect_menu(ui, &p, &mut self.panels, &status);
                self.state.prepare(&ctx, &self.settings);
                self.state.islands = !self.classic;
                if self.settings.composed {
                    bars::chrome::prepare(ui, &p, &status, &self.settings, self.state.open);
                }
                if self.native_top && !self.settings.composed {
                    bars::top_bars(
                        ui,
                        &p,
                        &icons,
                        &layout,
                        &mut self.panels,
                        &status,
                        &mut Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut self.state,
                        &self.settings,
                    );
                } else {
                    bars::top::reserve(
                        ui,
                        &self.settings,
                        Rect::from_min_size(egui::pos2(100.0, 8.0), Vec2::new(800.0, 36.0)),
                        80.0,
                    );
                }
                let favorites = bars::favorites::View::default();
                let row = bars::Row {
                    unread: 0,
                    latest_unread: None,
                    flash: 0.0,
                    flying: false,
                    seated: false,
                    mouselook: false,
                    ao_active: false,
                    locked: self.settings.locked,
                    favorites: &favorites,
                    bar: Bar::Bottom,
                    style: Style::default(),
                    editing: self.state.open,
                    floating: false,
                };
                let mut mic = bars::MicButton::default();
                bars::top::show(
                    ui,
                    &p,
                    &icons,
                    &layout,
                    &mut self.state,
                    &self.settings,
                    &mut self.panels,
                    &mut mic,
                    &row,
                    self.classic,
                );
                bars::bottom_bar(
                    ui,
                    &p,
                    &icons,
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
                    &mut mic,
                    false,
                    !self.classic,
                    None,
                    &favorites,
                );
                bars::side::bars(
                    ui,
                    &p,
                    &icons,
                    &layout,
                    &mut self.state,
                    &self.settings,
                    &mut self.panels,
                    &mut mic,
                    &row,
                    Rect::from_min_max(egui::pos2(0.0, 52.0), egui::pos2(self.screen.x, self.screen.y - 56.0)),
                    !self.classic,
                    [true, true],
                );
                let mut import_shown = false;
                let _ = floating::show(
                    &ctx,
                    &p,
                    &icons,
                    &mut self.state,
                    &mut self.panels,
                    &mut mic,
                    &row,
                    &mut import_shown,
                );
                if self.settings.composed {
                    (self.action, self.audio_action) = bars::chrome::show(
                        ui,
                        &p,
                        &icons,
                        &mut self.panels,
                        &status,
                        &mut self.audio,
                        &mut self.audio_ui,
                        self.media,
                        &mut self.notif,
                        Default::default(),
                        &mut self.state,
                        &self.settings,
                    );
                }
                if let Some(action) = bars::location_popup::show(ui, &p, &mut self.panels, &status) {
                    self.action = action;
                }
                if !self.native_top {
                    menu::popup_at(
                        &ctx,
                        egui::Id::unique("customization_test_menu"),
                        egui::pos2(40.0, 80.0),
                        &mut self.menu_open,
                        &p,
                        |ui| {
                            commands::show(ui, &p, &mut self.state, &self.settings, "");
                        },
                    );
                }
                crate::ui::layout::set_work_area(
                    &ctx,
                    Rect::from_min_max(egui::pos2(52.0, 52.0), self.screen.to_pos2() - Vec2::new(52.0, 56.0)),
                );
                self.state.show(&ctx, &p, &layout, &self.settings, None);
                self.state.commit_with_menus(&ctx, &layout, &mut self.settings);
            },
        );
        self.text.clear();
        fn gather(shape: &egui::Shape, text: &mut Vec<(String, Rect)>) {
            match shape {
                egui::Shape::Text(shape) => {
                    text.push((shape.galley.text().to_owned(), Rect::from_min_size(shape.pos, shape.galley.size())))
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        gather(shape, text);
                    }
                }
                _ => {}
            }
        }
        for shape in &output.shapes {
            gather(&shape.shape, &mut self.text);
        }
        output.textures_delta.clear();
        self.shapes = output.shapes;
    }

    fn pointer(&mut self, position: Pos2, pressed: Option<bool>) {
        let mut events = vec![egui::Event::PointerMoved(position)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        self.frame(events);
    }
}

#[test]
fn toolbar_group_glass_keeps_its_end_padding_around_the_last_native_button() {
    fn borders(shape: &egui::Shape, clip: Rect, result: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Rect(rect) if rect.stroke.width > 0.0 => result.push(rect.rect.intersect(clip)),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    borders(shape, clip, result);
                }
            }
            _ => {}
        }
    }
    for bar in Bar::ALL {
        for align in [Align::Start, Align::Center, Align::End] {
            for placement in [0, 1, 2] {
                let mut bench = Bench {
                    native_top: true,
                    menu_open: false,
                    ..Default::default()
                };
                bench.settings.composed = true;
                bench.settings.buttons = Some(Vec::new());
                let gap = |id| structure::separator(id);
                let button = |command| bars::top::button(command, "");
                let mut buttons = vec![button("bar_fps"), gap("first")];
                if placement == 0 {
                    buttons.extend([button("bar_notifications"), gap("middle")]);
                }
                buttons.extend([button("bar_media"), button("bar_music"), button("bar_volume")]);
                if placement != 0 {
                    if placement == 1 {
                        buttons.push(gap("last"));
                    }
                    buttons.push(button("bar_notifications"));
                }
                bench.settings.set_bar(bar, buttons);
                bench.settings.style_mut(bar).align = Some(align);
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let bell = bench
                    .ctx
                    .read_response(bars::tool_button_id("bar_notifications"))
                    .expect("native bell slot")
                    .rect;
                let mut painted = Vec::new();
                for shape in &bench.shapes {
                    borders(&shape.shape, shape.clip_rect, &mut painted);
                }
                let face = painted
                    .into_iter()
                    .filter(|rect| rect.contains_rect(bell))
                    .min_by(|a, b| a.area().total_cmp(&b.area()))
                    .expect("bell group glass");
                let (before, after) = if bar.vertical() {
                    (bell.top() - face.top(), face.bottom() - bell.bottom())
                } else {
                    (bell.left() - face.left(), face.right() - bell.right())
                };
                let case = format!("{bar:?}, {align:?}, placement={placement}");
                if placement != 2 {
                    assert!(before >= bars::islands::PAD - 0.5, "leading glass: {before}, {case}");
                }
                assert!((after - bars::islands::PAD).abs() < 0.5, "trailing glass: {after}, {case}");
            }
        }
    }
}

#[test]
fn native_readouts_keep_their_live_content_during_interface_customization() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            menu_open: false,
            classic,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let commands = ["bar_clock", "bar_fps", "bar_balance", "bar_ping"];
        let content: Vec<Vec<String>> = commands
            .iter()
            .map(|command| {
                let rect = bench.ctx.read_response(bars::tool_button_id(command)).expect("native readout").rect;
                let texts: Vec<_> = bench
                    .text
                    .iter()
                    .filter(|(_, text_rect)| rect.contains(text_rect.center()))
                    .map(|(text, _)| text.clone())
                    .collect();
                assert!(!texts.is_empty(), "live content: {command}");
                texts
            })
            .collect();
        bench.state.open = true;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        for (command, wanted) in commands.iter().zip(content) {
            let rect = bench
                .ctx
                .read_response(bars::tool_button_id(command))
                .expect("editable readout")
                .rect;
            for text in wanted {
                assert!(
                    bench
                        .text
                        .iter()
                        .any(|(actual, text_rect)| *actual == text && rect.contains(text_rect.center())),
                    "{command} retains {text}, classic={classic}"
                );
            }
        }
    }
}

#[test]
fn drawer_settings_put_own_properties_first_and_flip_the_icon_picker_beside_the_window() {
    let (mut bench, id) = drawer_target(false, Bar::Left, false, false);
    bench.screen = Vec2::new(1920.0, 1080.0);
    bench.state.drawer_editor = Some(id.clone());
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let own = menu_point(&bench, aurora_i18n::tr!("toolbar-block-button-mode"));
    let contents = menu_point(&bench, aurora_i18n::tr!("toolbar-drawer-default-mode"));
    let icon = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_icon_button_rect", &id))))
        .expect("icon button");
    // its icon, then its button's display, then its buttons' (human
    // request, 2026-11-26)
    assert!(
        icon.bottom() < own.y && own.y < contents.y,
        "icon, own display, then content defaults"
    );
    assert!(
        bench
            .ctx
            .memory(|memory| memory.area_rect(Id::unique(("toolbar_icon_side_picker", &id))))
            .is_none()
    );
    bench.pointer(icon.center(), Some(true));
    bench.pointer(icon.center(), Some(false));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let window = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings")))
        .expect("settings");
    let picker = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique(("toolbar_icon_side_picker", &id))))
        .expect("picker");
    assert!(picker.left() >= window.right(), "picker opens on the right: {window:?}, {picker:?}");
    let title = window.center_top() + Vec2::new(0.0, 18.0);
    let moved = title + Vec2::new(600.0, 0.0);
    bench.pointer(title, Some(true));
    bench.pointer(moved, None);
    bench.pointer(moved, Some(false));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let window = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique("toolbar_drawer_settings")))
        .expect("moved settings");
    let picker = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique(("toolbar_icon_side_picker", &id))))
        .expect("moved picker");
    assert!(picker.right() <= window.left(), "picker flips to the left: {window:?}, {picker:?}");
    assert!(bench.ctx.content_rect().contains_rect(picker));
}

#[test]
fn editing_intercepts_native_and_regular_buttons_without_executing_their_actions() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for command in [
                "bar_clock",
                "bar_fps",
                "bar_balance",
                "bar_volume",
                "report_bug",
                "inventory",
                "speak",
            ] {
                let mut bench = Bench {
                    native_top: true,
                    classic,
                    screen: Vec2::new(1920.0, 1080.0),
                    ..Default::default()
                };
                bench.settings.compose_top();
                bench.state.pending = Some(Edit::Insert(bar, bars::top::button(command, "folder-filled"), 0));
                bench.state.commit(&Layout::default(), &mut bench.settings);
                bench.state.open = true;
                if command == "bar_volume" {
                    bench.audio_ui.panel_open = true;
                }
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let before = format!("{:?}", bench.panels);
                let point = bench
                    .ctx
                    .read_response(bars::tool_button_id(command))
                    .expect("editable item")
                    .rect
                    .center();
                bench.pointer(point, Some(true));
                bench.pointer(point, Some(false));
                assert_eq!(
                    bench.state.selected.as_deref(),
                    Some(command),
                    "select {command} on {bar:?}, classic={classic}"
                );
                assert!(matches!(bench.action, bars::BarAction::None));
                assert_eq!(bench.audio_action, crate::ui::audio::AudioAction::None);
                assert!(!bench.audio.muted[0]);
                if command == "bar_volume" {
                    assert!(
                        bench.ctx.read_response(Id::unique("volume_panel")).is_none(),
                        "editing never exposes an interactive volume popup"
                    );
                }
                assert_eq!(format!("{:?}", bench.panels), before);
            }
        }
    }
}

#[test]
fn proximity_preview_matches_the_drop_and_hover_never_saves_on_any_edge_or_style() {
    let task_layout = Layout::default();
    for classic in [false, true] {
        for bar in Bar::ALL {
            for separate in [false, true] {
                let mut bench = Bench {
                    native_top: true,
                    classic,
                    screen: Vec2::new(1920.0, 1080.0),
                    ..Default::default()
                };
                bench.settings.composed = true;
                bench.settings.top.clear();
                bench.settings.buttons = Some(Vec::new());
                bench
                    .settings
                    .set_bar(bar, vec![bars::top::button("inventory", "briefcase-filled")]);
                bench.settings.style_mut(bar).sizing = Some(Sizing::Auto);
                bench.state.open = true;
                bench.state.search = catalogue::find("radar").expect("radar").label().into();
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let source = bench
                    .ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", "radar"))))
                    .expect("radar tile")
                    .center();
                let anchor = bench.ctx.read_response(bars::tool_button_id("inventory")).expect("anchor").rect;
                let axis = if bar.vertical() { Vec2::Y } else { Vec2::X };
                let point = anchor.center()
                    + axis
                        * if separate {
                            60.0
                        } else if bar.vertical() {
                            anchor.height() / 2.0 - 1.0
                        } else {
                            anchor.width() / 2.0 - 1.0
                        };
                let before = serde_json::to_string(&bench.settings).expect("layout before hover");
                bench.pointer(source, Some(true));
                bench.pointer(source + Vec2::new(12.0, 0.0), None);
                for _ in 0..4 {
                    bench.pointer(point, None);
                }
                let preview = drop_preview::current(&bench.ctx).expect("drop preview");
                assert_eq!(preview.bar, bar, "target on {bar:?}, classic={classic}, separate={separate}");
                assert_eq!(
                    preview.kind,
                    if separate {
                        drop_preview::Kind::Group
                    } else {
                        drop_preview::Kind::Join
                    }
                );
                assert_eq!(serde_json::to_string(&bench.settings).expect("hover"), before);
                let snapped = bench
                    .ctx
                    .read_response(bars::tool_button_id("radar"))
                    .expect("real provisional button")
                    .rect;
                assert!(
                    (preview.rect.min - snapped.min).length() < 0.5,
                    "preview={:?}, widget={snapped:?}, edge={bar:?}, classic={classic}, separate={separate}",
                    preview.rect
                );
                assert!(
                    bench
                        .ctx
                        .memory(|memory| memory.area_rect(egui::Id::unique("toolbar_drop_preview").with("caption")))
                        .is_none()
                );
                bench.pointer(point, Some(false));
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let final_rect = bench.ctx.read_response(bars::tool_button_id("radar")).expect("dropped button").rect;
                assert!(
                    (snapped.min - final_rect.min).length() < 0.5 && (snapped.max - final_rect.max).length() < 0.5,
                    "snapped={snapped:?}, final={final_rect:?}, edge={bar:?}, classic={classic}"
                );
                let list = bench.settings.bar(bar, &task_layout);
                assert_eq!(
                    list.iter().filter(|b| structure::is_separator(&b.command)).count(),
                    usize::from(separate)
                );
                assert_eq!(list.iter().filter(|b| b.command == "radar").count(), 1);
                assert_eq!(bench.settings.location("radar", &Layout::default()), Some(bar));
            }
        }
    }
}

#[test]
fn nearby_perpendicular_drops_isolate_the_whole_group_and_floaters_or_locks_block_the_preview() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    bench.settings.left = vec![
        bars::top::button("inventory", "briefcase-filled"),
        bars::top::button("people", "users-filled"),
    ];
    bench.settings.buttons = Some(Vec::new());
    bench.state.open = true;
    bench.state.search = catalogue::find("radar").expect("radar").label().into();
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", "radar"))))
        .expect("tile")
        .center();
    let anchor = bench
        .ctx
        .read_response(bars::tool_button_id("people"))
        .expect("second control")
        .rect;
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    let editor = bench
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(editor::WINDOW)))
        .expect("editor");
    let covered = editor.min + Vec2::new(200.0, 6.0);
    for _ in 0..4 {
        bench.pointer(covered, None);
    }
    assert!(drop_preview::current(&bench.ctx).is_none(), "an overlapping floater keeps the drop");
    let point = anchor.center() + Vec2::new(50.0, 0.0);
    for _ in 0..4 {
        bench.pointer(point, None);
    }
    let preview = drop_preview::current(&bench.ctx).expect("detached side preview");
    assert_eq!(preview.kind, drop_preview::Kind::Group);
    assert_eq!(preview.rank, 2, "do not split the middle of the existing group");
    bench.settings.locked = true;
    bench.pointer(point, None);
    assert!(drop_preview::current(&bench.ctx).is_none());
    let before = serde_json::to_string(&bench.settings).expect("locked");
    bench.pointer(point, Some(false));
    assert_eq!(serde_json::to_string(&bench.settings).expect("locked drop"), before);
}

#[test]
fn snapping_with_the_editor_closed_keeps_hidden_blocks_and_outer_gaps_out_of_the_final_geometry() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.top.clear();
    bench.settings.address_visible = Some(false);
    bench.settings.buttons = Some(vec![
        structure::separator("outer"),
        bars::top::button("inventory", "briefcase-filled"),
        structure::separator("hidden"),
        bars::top::button("bar_navigation", "map-pin-filled"),
    ]);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .read_response(bars::tool_button_id("inventory"))
        .expect("inventory")
        .rect
        .center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    let point = source + Vec2::new(50.0, 0.0);
    for _ in 0..6 {
        bench.pointer(point, None);
    }
    let preview = drop_preview::current(&bench.ctx).expect("snapped destination").rect;
    bench.pointer(point, Some(false));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let final_rect = bench
        .ctx
        .read_response(bars::tool_button_id("inventory"))
        .expect("dropped inventory")
        .rect;
    assert!(
        (preview.min - final_rect.min).length() < 0.5 && (preview.max - final_rect.max).length() < 0.5,
        "preview={preview:?}, final={final_rect:?}"
    );
    assert_eq!(bench.settings.address_visible, Some(false));
}

#[test]
fn opening_and_finishing_interface_editing_exposes_empty_targets_without_saving_a_layout() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.top.clear();
    bench.settings.buttons = Some(Vec::new());
    let before = serde_json::to_string(&bench.settings).expect("initial layout");
    bench.state.open = true;
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    for name in ["toolbar_left", "toolbar_right"] {
        assert!(
            bench
                .ctx
                .read_response(egui::Id::unique(("toolbar_background", name)))
                .expect("visible empty edge")
                .rect
                .height()
                > 300.0
        );
    }
    assert!(
        !bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-edit-done")),
        "the redundant Finish button is absent"
    );
    crate::ui::widgets::close_floater(&bench.ctx, editor::WINDOW);
    bench.frame(Vec::new());
    assert!(!bench.state.open);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    assert!(!ToolbarUi::editing(&bench.ctx));
    assert!(
        bench
            .ctx
            .read_response(egui::Id::unique(("toolbar_background", "toolbar_left")))
            .is_none()
    );
    assert_eq!(serde_json::to_string(&bench.settings).expect("unchanged layout"), before);
}

#[test]
fn drawers_open_toggle_and_run_the_original_commands_on_each_edge_in_both_styles() {
    // "Close after a click on a button" (on by default) folds the drawer
    // once its button ran; off, it stays open until its icon is pressed
    for (classic, bar, closes) in [false, true].into_iter().flat_map(|classic| {
        Bar::ALL
            .into_iter()
            .flat_map(move |bar| [(classic, bar, true), (classic, bar, false)])
    }) {
        {
            let mut bench = Bench {
                native_top: true,
                classic,
                screen: Vec2::new(1920.0, 1080.0),
                ..Default::default()
            };
            bench.settings.compose_top();
            let id = drawers::create(&mut bench.settings, bar, &Layout::default());
            assert!(bench.settings.drawers[&id].closes, "on by default");
            bench.state.pending = Some(Edit::DrawerInsert(id.clone(), bars::top::button("inventory", "folder-filled"), 0));
            assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
            bench.state.pending = Some(block_window::edit_of(
                &bench.settings,
                &Layout::default(),
                block_window::Target::Drawer(id.clone()),
                |o| {
                    o.closes = closes;
                },
            ));
            assert_eq!(bench.state.commit(&Layout::default(), &mut bench.settings), !closes);
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let point = bench
                .ctx
                .read_response(bars::tool_button_id(&id))
                .expect("drawer button")
                .rect
                .center();
            bench.pointer(point, Some(true));
            bench.pointer(point, Some(false));
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let target = bench
                .ctx
                .read_response(egui::Id::unique(("drawer_drop", &id)))
                .expect("expanded drawer")
                .rect;
            assert!(
                Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(target),
                "drawer on {bar:?}: {target:?}"
            );
            let child = bench
                .ctx
                .read_response(bars::tool_button_id("inventory"))
                .expect("drawer contents")
                .rect
                .center();
            let before = bench.panels.inventory;
            bench.pointer(child, Some(true));
            bench.pointer(child, Some(false));
            assert_ne!(bench.panels.inventory, before, "native action on {bar:?}, classic={classic}");
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let open = bench.ctx.read_response(egui::Id::unique(("drawer_drop", &id))).is_some();
            assert_eq!(open, !closes, "after a click on {bar:?}, classic={classic}");
            let shown = |bench: &Bench| bench.ctx.read_response(egui::Id::unique(("drawer_drop", &id))).is_some();
            if closes {
                // open again: a click outside it (here the world) folds it
                bench.pointer(point, Some(true));
                bench.pointer(point, Some(false));
                for _ in 0..3 {
                    bench.frame(Vec::new());
                }
                assert!(shown(&bench), "reopened on {bar:?}, classic={classic}");
            }
            let world = bench.screen / 2.0;
            bench.pointer(world.to_pos2(), Some(true));
            bench.pointer(world.to_pos2(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            assert_eq!(shown(&bench), !closes, "after a click outside on {bar:?}, classic={classic}");
            if !closes {
                bench.pointer(point, Some(true));
                bench.pointer(point, Some(false));
                for _ in 0..3 {
                    bench.frame(Vec::new());
                }
                assert!(!shown(&bench));
            }
        }
    }
}

#[test]
fn four_drawer_options_fit_without_scrolling_and_overflow_recovers_after_shrinking() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for screen in [Vec2::new(1000.0, 700.0), Vec2::new(640.0, 360.0)] {
                for editing in [false, true] {
                    let mut bench = Bench {
                        native_top: true,
                        menu_open: false,
                        classic,
                        screen,
                        ..Default::default()
                    };
                    bench.settings.composed = true;
                    bench.settings.buttons = Some(Vec::new());
                    bench.settings.style_mut(bar).align = Some(Align::End);
                    bench.state.open = editing;
                    let id = drawers::create(&mut bench.settings, bar, &Layout::default());
                    let small: Vec<_> = ["inventory", "worldmap", "preferences", "search"]
                        .map(|command| bars::top::button(command, "folder-filled"))
                        .into();
                    drawers::set_open(&bench.ctx, &id, true);
                    for count in [0, 4, 32, 4] {
                        bench.settings.drawers.get_mut(&id).expect("drawer").mode = if count == 0 {
                            ButtonMode::IconsOnly
                        } else {
                            ButtonMode::IconsWithText
                        };
                        bench.settings.drawers.get_mut(&id).expect("drawer").buttons = if count == 0 {
                            Vec::new()
                        } else if count == 4 {
                            small.clone()
                        } else {
                            catalogue::COMMANDS
                                .iter()
                                .filter(|command| !structure::is_block(command.id) && !structure::is_separator(command.id))
                                .take(count)
                                .map(|command| bars::top::button(command.id, command.icon))
                                .collect()
                        };
                        for _ in 0..4 {
                            bench.frame(Vec::new());
                        }
                        let (content, viewport) = bench
                            .ctx
                            .data(|data| data.get_temp::<(Vec2, Rect)>(egui::Id::unique(("drawer_scroll_geometry", &id))))
                            .expect("drawer viewport");
                        let target = bench
                            .ctx
                            .read_response(egui::Id::unique(("drawer_drop", &id)))
                            .expect("drawer surface")
                            .rect;
                        let case = format!("{bar:?}, classic={classic}, {screen:?}, editing={editing}, count={count}");
                        assert!(
                            Rect::from_min_size(Pos2::ZERO, screen).contains_rect(target),
                            "in viewport: {target:?}, {case}"
                        );
                        assert!(
                            content.x <= viewport.width() + 0.5,
                            "horizontal fit: {content:?}, {viewport:?}, {case}"
                        );
                        if count <= 4 {
                            assert!(
                                content.y <= viewport.height() + 0.5,
                                "four buttons need no scroll: {content:?}, {viewport:?}, {case}"
                            );
                            for button in small.iter().take(count) {
                                let response = bench
                                    .ctx
                                    .read_response(bars::tool_button_id(&button.command))
                                    .expect("drawer option");
                                assert!(
                                    viewport.expand(0.5).contains_rect(response.rect),
                                    "visible option: {}, {case}",
                                    button.command
                                );
                            }
                        } else {
                            assert!(content.y > viewport.height() + 1.0, "overflow scrolls: {case}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn drawer_geometry_follows_effective_modes_and_recovers_after_label_changes() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for editing in [false, true] {
                let (mut bench, id) = drawer_target(classic, bar, editing, true);
                bench.settings.top.retain(|button| button.command != "inventory");
                let commands = ["inventory", "worldmap", "preferences"];
                bench.settings.drawers.get_mut(&id).expect("drawer").buttons = commands.map(tests::button).into();
                for phase in 0..4 {
                    let drawer = bench.settings.drawers.get_mut(&id).expect("drawer");
                    drawer.mode = if phase == 1 {
                        ButtonMode::IconsWithText
                    } else {
                        ButtonMode::IconsOnly
                    };
                    bench.settings.appearance.clear();
                    if phase == 1 {
                        for command in commands {
                            bench.settings.appearance.insert(
                                command.into(),
                                appearance::Appearance {
                                    mode: Some(ButtonMode::IconsOnly),
                                    ..Default::default()
                                },
                            );
                        }
                    } else if phase == 2 {
                        bench.settings.appearance.insert(
                            "worldmap".into(),
                            appearance::Appearance {
                                mode: Some(ButtonMode::TextOnly),
                                ..Default::default()
                            },
                        );
                    }
                    for _ in 0..4 {
                        bench.frame(Vec::new());
                    }
                    let rects = commands.map(|command| bench.ctx.read_response(bars::tool_button_id(command)).expect("drawer child").rect);
                    let vertical = !bar.vertical() || phase == 2;
                    for pair in rects.windows(2) {
                        assert!(
                            if vertical {
                                pair[1].top() >= pair[0].bottom()
                            } else {
                                pair[1].left() >= pair[0].right()
                            },
                            "{bar:?}, classic={classic}, editing={editing}, phase={phase}: {rects:?}"
                        );
                    }
                    let (content, viewport) = bench
                        .ctx
                        .data(|data| data.get_temp::<(Vec2, Rect)>(Id::unique(("drawer_scroll_geometry", &id))))
                        .expect("scroll geometry");
                    assert!(
                        content.x <= viewport.width() + 0.5 && content.y <= viewport.height() + 0.5,
                        "unnecessary scrolling: {content:?}, {viewport:?}"
                    );
                    if phase != 2 {
                        assert!(
                            (if vertical { viewport.width() } else { viewport.height() }) < 40.0,
                            "icon-only cross-axis must be compact: {viewport:?}"
                        );
                    }
                    assert!(bench.ctx.content_rect().contains_rect(viewport));
                    let toggle = bench.ctx.read_response(bars::tool_button_id(&id)).expect("toggle").rect;
                    assert!(!toggle.intersects(viewport));
                    if bar.vertical() && phase != 2 {
                        let outer = bench
                            .ctx
                            .memory(|memory| memory.area_rect(Id::unique(("toolbar_drawer", &id))))
                            .expect("drawer frame");
                        assert!(
                            (rects[0].center().y - toggle.center().y).abs() < 0.5,
                            "aligned icon row: {rects:?}, {toggle:?}"
                        );
                        assert!(
                            (outer.height() - toggle.height() - bars::islands::PAD * 2.0).abs() < 0.5,
                            "same block height: {outer:?}, {toggle:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn horizontal_drawers_insert_along_x_and_only_scroll_when_content_overflows() {
    for classic in [false, true] {
        for bar in [Bar::Left, Bar::Right] {
            let (mut bench, id) = drawer_target(classic, bar, false, true);
            bench.settings.drawers.get_mut(&id).expect("drawer").mode = ButtonMode::IconsOnly;
            bench.settings.drawers.get_mut(&id).expect("drawer").buttons = ["preferences", "worldmap", "search"].map(tests::button).into();
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let first = bench.ctx.read_response(bars::tool_button_id("preferences")).expect("first").rect;
            let second = bench.ctx.read_response(bars::tool_button_id("worldmap")).expect("second").rect;
            let target = egui::pos2((first.right() + second.left()) / 2.0, first.center().y);
            start_toolbar_drag(&mut bench, "inventory");
            for _ in 0..3 {
                bench.pointer(target, None);
            }
            bench.pointer(target, Some(false));
            assert_eq!(
                bench.settings.drawers[&id]
                    .buttons
                    .iter()
                    .map(|button| button.command.as_str())
                    .collect::<Vec<_>>(),
                ["preferences", "inventory", "worldmap", "search"]
            );
            assert!(!bench.panels.inventory, "dropping does not activate Inventory");
            bench.screen = Vec2::new(640.0, 360.0);
            let small = bench.settings.drawers[&id].buttons.clone();
            for overflow in [true, false] {
                bench.settings.drawers.get_mut(&id).expect("drawer").buttons = if overflow {
                    catalogue::COMMANDS
                        .iter()
                        .filter(|command| !structure::is_block(command.id) && !structure::is_separator(command.id))
                        .take(32)
                        .map(|command| bars::top::button(command.id, command.icon))
                        .collect()
                } else {
                    small.clone()
                };
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let (content, viewport) = bench
                    .ctx
                    .data(|data| data.get_temp::<(Vec2, Rect)>(Id::unique(("drawer_scroll_geometry", &id))))
                    .expect("scroll geometry");
                assert_eq!(
                    content.x > viewport.width() + 0.5,
                    overflow,
                    "{bar:?}, classic={classic}: {content:?}, {viewport:?}"
                );
                assert!(
                    content.y <= viewport.height() + 0.5,
                    "clipped row on {bar:?}, classic={classic}, overflow={overflow}: {content:?}, {viewport:?}"
                );
                assert!(bench.ctx.content_rect().contains_rect(viewport));
            }
        }
    }
}

#[test]
fn native_icon_blocks_follow_drawer_rows_and_labels_restore_columns() {
    for classic in [false, true] {
        for bar in [Bar::Left, Bar::Right] {
            let (mut bench, id) = drawer_target(classic, bar, false, true);
            bench.conversation_count = 4;
            bench.settings.drawers.get_mut(&id).expect("drawer").buttons =
                ["bar_conversations", "bar_volume", "preferences"].map(tests::button).into();
            bench.settings.drawers.get_mut(&id).expect("drawer").mode = ButtonMode::IconsOnly;
            for label in [false, true, false] {
                if label {
                    bench.settings.appearance.insert(
                        "preferences".into(),
                        appearance::Appearance {
                            mode: Some(ButtonMode::IconsWithText),
                            ..Default::default()
                        },
                    );
                } else {
                    bench.settings.appearance.remove("preferences");
                }
                for _ in 0..4 {
                    bench.frame(Vec::new());
                }
                let conversations = bench
                    .ctx
                    .read_response(bars::tool_button_id("bar_conversations"))
                    .expect("conversation icons")
                    .rect;
                let volume = bench.ctx.read_response(bars::tool_button_id("bar_volume")).expect("volume").rect;
                assert_eq!(
                    conversations.height() > conversations.width(),
                    label,
                    "{bar:?}, classic={classic}: {conversations:?}"
                );
                assert!(if label {
                    volume.top() >= conversations.bottom()
                } else {
                    volume.left() >= conversations.right()
                });
            }
        }
    }
}

#[test]
fn overflowing_horizontal_drawers_leave_their_toggle_accessible() {
    for classic in [false, true] {
        for bar in [Bar::Top, Bar::Bottom] {
            let mut bench = Bench {
                native_top: true,
                menu_open: false,
                classic,
                screen: Vec2::new(640.0, 360.0),
                ..Default::default()
            };
            bench.settings.composed = true;
            bench.settings.buttons = Some(Vec::new());
            let id = drawers::create(&mut bench.settings, bar, &Layout::default());
            bench.settings.drawers.get_mut(&id).expect("drawer").buttons = catalogue::COMMANDS
                .iter()
                .filter(|command| !structure::is_block(command.id) && !structure::is_separator(command.id))
                .take(32)
                .map(|command| bars::top::button(command.id, command.icon))
                .collect();
            drawers::set_open(&bench.ctx, &id, true);
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let anchor = bench.ctx.read_response(bars::tool_button_id(&id)).expect("drawer toggle").rect;
            let panel = bench
                .ctx
                .read_response(egui::Id::unique(("drawer_drop", &id)))
                .expect("open drawer")
                .rect;
            assert!(
                !anchor.intersects(panel),
                "toggle covered on {bar:?}, classic={classic}: {anchor:?}, {panel:?}"
            );
            bench.pointer(anchor.center(), Some(true));
            bench.pointer(anchor.center(), Some(false));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            assert!(!drawers::is_open(&bench.ctx, &id));
        }
    }
}

#[test]
fn compact_drawer_settings_stay_above_the_panel_and_edit_name_and_inherited_display() {
    let mut bench = Bench {
        native_top: true,
        menu_open: false,
        screen: Vec2::new(640.0, 360.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.buttons = Some(Vec::new());
    let id = drawers::create(&mut bench.settings, Bar::Left, &Layout::default());
    bench.settings.drawers.get_mut(&id).expect("drawer").buttons = ["inventory", "worldmap", "preferences", "search"]
        .map(|command| bars::top::button(command, "folder-filled"))
        .into();
    drawers::set_open(&bench.ctx, &id, true);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-drawer")));
    assert!(
        bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("drawer_settings_button", &id))))
            .is_none()
    );
    let toggle = bench.ctx.read_response(bars::tool_button_id(&id)).expect("toggle").rect.center();
    for pressed in [true, false] {
        bench.frame(vec![
            egui::Event::PointerMoved(toggle),
            egui::Event::PointerButton {
                pos: toggle,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    for key in ["toolbar-block-edit", "toolbar-interface-edit"] {
        assert!(
            bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
            "context entry {key}"
        );
    }
    let edit = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-block-edit"))
        .expect("edit drawer")
        .1
        .center();
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.state.drawer_editor.as_deref(), Some(id.as_str()));
    let name = bench
        .ctx
        .read_response(egui::Id::unique(("drawer_name", &id)))
        .expect("name field")
        .rect
        .center();
    bench.pointer(name, Some(true));
    bench.pointer(name, Some(false));
    bench.frame(vec![egui::Event::Text("Tools".into())]);
    assert_eq!(bench.settings.left[0].tooltip, "Tools");
    let mode_label = super::mode_rows()[1].2;
    scroll_window_to(&mut bench, mode_label);
    let contents = menu_point(&bench, aurora_i18n::tr!("toolbar-drawer-default-mode"));
    let choice = bench
        .text
        .iter()
        .find(|(text, rect)| text == mode_label && rect.center().y > contents.y)
        .expect("icon-only radio")
        .1
        .center();
    bench.pointer(choice, Some(true));
    bench.pointer(choice, Some(false));
    assert_eq!(bench.settings.drawers[&id].mode, ButtonMode::IconsOnly);
    bench.state.drawer_editor = None;
    bench.settings.appearance.insert(
        "inventory".into(),
        appearance::Appearance {
            mode: Some(ButtonMode::IconsWithText),
            ..Default::default()
        },
    );
    for mode in [ButtonMode::IconsOnly, ButtonMode::TextOnly, ButtonMode::IconsWithText] {
        bench.state.pending = Some(block_window::edit_of(
            &bench.settings,
            &Layout::default(),
            block_window::Target::Drawer(id.clone()),
            |o| {
                o.name = "Tools".into();
                o.mode = mode;
                o.look = appearance::get(&bench.ctx, &id);
            },
        ));
        bench.state.commit(&Layout::default(), &mut bench.settings);
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert!(
            bench
                .text
                .iter()
                .any(|(text, _)| text == label(&bars::top::button("inventory", ""))),
            "explicit override survives {mode:?}"
        );
        assert_eq!(
            bench.text.iter().any(|(text, _)| text == label(&bars::top::button("worldmap", ""))),
            mode != ButtonMode::IconsOnly,
            "drawer inheritance {mode:?}"
        );
    }
}

#[test]
fn editing_can_drag_a_catalogue_button_into_a_drawer_and_back_to_a_toolbar() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    let id = drawers::create(&mut bench.settings, Bar::Right, &Layout::default());
    bench.state.open = true;
    bench.state.search = aurora_i18n::tr!("common-inventory").into();
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let drawer = bench
        .ctx
        .read_response(bars::tool_button_id(&id))
        .expect("drawer button")
        .rect
        .center();
    bench.pointer(drawer, Some(true));
    bench.pointer(drawer, Some(false));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("toolbar_catalogue", "inventory"))))
        .expect("inventory tile")
        .center();
    let target = bench
        .ctx
        .read_response(egui::Id::unique(("drawer_drop", &id)))
        .expect("drawer drop target")
        .rect
        .center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    assert!(ToolbarUi::dragging(&bench.ctx));
    for _ in 0..4 {
        bench.pointer(target, None);
    }
    bench.pointer(target, Some(false));
    assert_eq!(bench.settings.drawer_of("inventory"), Some(id.as_str()));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .read_response(bars::tool_button_id("inventory"))
        .expect("drawer item")
        .rect
        .center();
    let target = bench
        .ctx
        .read_response(bars::tool_button_id("people"))
        .expect("bottom destination")
        .rect;
    let target = egui::pos2(target.right() - 1.0, target.center().y);
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    assert!(ToolbarUi::dragging(&bench.ctx));
    for _ in 0..4 {
        bench.pointer(target, None);
    }
    bench.pointer(target, Some(false));
    assert_eq!(bench.settings.location("inventory", &Layout::default()), Some(Bar::Bottom));
    assert_eq!(bench.settings.drawer_of("inventory"), None);
    assert!(!bench.settings.drawers.contains_key(&id), "the emptied source drawer disappears");
}

#[test]
fn separator_sizes_and_springs_follow_every_edge_and_window_resize_in_both_styles() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            let mut bench = Bench {
                native_top: true,
                classic,
                screen: Vec2::new(1600.0, 900.0),
                ..Default::default()
            };
            bench.settings.composed = true;
            bench.settings.top.clear();
            bench.settings.buttons = Some(Vec::new());
            let buttons = vec![
                tests::button("inventory"),
                structure::separator("fixed"),
                tests::button("people"),
                structure::separator("spring"),
                tests::button("preferences"),
            ];
            match bar {
                Bar::Top => bench.settings.top = buttons,
                Bar::Bottom => bench.settings.buttons = Some(buttons),
                Bar::Left => bench.settings.left = buttons,
                Bar::Right => bench.settings.right = buttons,
            }
            for (id, size, stretch) in [("separator:fixed", 64.0, false), ("separator:spring", 12.0, true)] {
                bench.state.pending = Some(Edit::Separator(id.into(), structure::SeparatorStyle { size, stretch }));
                assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
            }
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let extent = |bench: &Bench, command: &str| {
                let rect = bench.ctx.read_response(bars::tool_button_id(command)).expect("separator slot").rect;
                assert!(
                    Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(rect),
                    "{command} on {bar:?}, classic={classic}: {rect:?}"
                );
                if bar.vertical() { rect.height() } else { rect.width() }
            };
            assert!(
                (extent(&bench, "separator:fixed") - 64.0).abs() < 0.5,
                "fixed on {bar:?}, classic={classic}"
            );
            let spring = extent(&bench, "separator:spring");
            assert!(spring > 100.0, "spring on {bar:?}, classic={classic}: {spring}");
            bench.screen += Vec2::new(240.0, 180.0);
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            assert!((extent(&bench, "separator:fixed") - 64.0).abs() < 0.5);
            assert!(extent(&bench, "separator:spring") > spring + 100.0);
        }
    }
}

#[test]
fn a_separators_context_opens_its_settings_and_the_switch_commits_only_that_instance() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    bench.settings.top.insert(1, structure::separator("configured"));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let point = bench
        .ctx
        .read_response(bars::tool_button_id("separator:configured"))
        .expect("separator")
        .rect
        .center();
    for pressed in [true, false] {
        bench.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let entry = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-separator-settings"))
        .expect("separator settings entry")
        .1
        .center();
    bench.pointer(entry, Some(true));
    bench.pointer(entry, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(
        bench
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::unique("toolbar_separator_settings")))
            .is_some()
    );
    let toggle = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-separator-stretch"))
        .expect("stretch switch")
        .1
        .center();
    bench.pointer(toggle, Some(true));
    bench.pointer(toggle, Some(false));
    assert!(bench.settings.separators["separator:configured"].stretch);
    assert_eq!(bench.settings.separators.len(), 1);
}

#[test]
fn separate_chrome_controls_keep_their_native_actions_on_every_edge() {
    use crate::ui::audio::AudioAction;
    for classic in [false, true] {
        for bar in Bar::ALL {
            for in_drawer in [false, true] {
                for command in [
                    "bar_status",
                    "bar_balance",
                    "bar_clock",
                    "bar_fps",
                    "bar_media",
                    "bar_music",
                    "bar_volume",
                ] {
                    let mut bench = Bench {
                        native_top: true,
                        classic,
                        screen: Vec2::new(1920.0, 1080.0),
                        ..Default::default()
                    };
                    bench.media.media_available = true;
                    bench.media.music_available = true;
                    bench.settings.compose_top();
                    let button = bars::top::button(command, "");
                    if in_drawer {
                        let id = drawers::create(&mut bench.settings, bar, &Layout::default());
                        drawers::set_open(&bench.ctx, &id, true);
                        bench.state.pending = Some(Edit::DrawerInsert(id, button, 0));
                    } else {
                        bench.state.pending = Some(Edit::Insert(bar, button, 0));
                    }
                    bench.state.commit(&Layout::default(), &mut bench.settings);
                    for _ in 0..4 {
                        bench.frame(Vec::new());
                    }
                    let point = bench
                        .ctx
                        .read_response(bars::tool_button_id(command))
                        .expect("native control")
                        .rect
                        .center();
                    bench.pointer(point, Some(true));
                    bench.pointer(point, Some(false));
                    match command {
                        "bar_status" => assert!(
                            egui::Popup::is_id_open(&bench.ctx, egui::Id::unique("bars_island_status").with("popup")),
                            "status on {bar:?}, classic={classic}, drawer={in_drawer}, target={point:?}"
                        ),
                        "bar_balance" => assert!(matches!(bench.action, bars::BarAction::BuyCurrency)),
                        "bar_clock" => assert!(matches!(bench.action, bars::BarAction::SystemIsland(_))),
                        "bar_fps" => assert!(bench.panels.perf),
                        "bar_media" => assert_eq!(bench.audio_action, AudioAction::ToggleMedia),
                        "bar_music" => assert_eq!(bench.audio_action, AudioAction::ToggleMusic),
                        "bar_volume" => {
                            assert!(bench.audio.muted[0]);
                            assert_eq!(bench.audio_action, AudioAction::Changed);
                            for _ in 0..3 {
                                bench.frame(Vec::new());
                            }
                            let panel = bench
                                .ctx
                                .memory(|memory| memory.area_rect(egui::Id::unique("volume_panel")))
                                .expect("volume panel");
                            assert!(
                                Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(panel),
                                "volume on {bar:?}, classic={classic}: {panel:?}"
                            );
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}

#[test]
fn conversation_blocks_switch_between_column_and_row_on_every_edge() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            classic,
            conversation_count: 6,
            screen: Vec2::new(1600.0, 900.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        for bar in [Bar::Left, Bar::Right, Bar::Bottom, Bar::Top] {
            bench.state.pending = Some(Edit::Insert(bar, bars::top::button("bar_conversations", "messages-filled"), 0));
            bench.state.commit(&Layout::default(), &mut bench.settings);
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let slot = bench
                .ctx
                .read_response(bars::tool_button_id("bar_conversations"))
                .expect("conversation block")
                .rect;
            if bar.vertical() {
                assert!(slot.height() > slot.width() * 2.0, "column on {bar:?}, classic={classic}: {slot:?}");
                assert!(slot.width() < 50.0);
            } else {
                assert!(slot.width() > slot.height() * 2.0, "row on {bar:?}, classic={classic}: {slot:?}");
            }
            assert!(
                Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(slot),
                "visible conversation block on {bar:?}, classic={classic}: {slot:?}"
            );
        }
    }
}

#[test]
fn catalogue_level_changes_do_not_edit_bars_and_search_finds_advanced_commands_from_main() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.state.open = true;
    let before = serde_json::to_string(&bench.settings).expect("settings");
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    assert!(!bench.state.advanced_catalogue);
    let advanced = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-section-advanced"))
        .expect("advanced section")
        .1
        .center();
    bench.pointer(advanced, Some(true));
    bench.pointer(advanced, Some(false));
    assert!(bench.state.advanced_catalogue);
    bench.state.advanced_catalogue = false;
    bench.state.search = "settings_debug".into();
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let found = bench
        .ctx
        .read_response(egui::Id::unique(("toolbar_tile", "settings_debug")))
        .expect("advanced search result");
    assert!(found.interact_rect.is_positive());
    assert_eq!(serde_json::to_string(&bench.settings).expect("settings"), before);
}

#[test]
fn native_blocks_and_repeatable_separators_drag_to_every_edge_in_both_layouts() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for command in [
                structure::SEPARATOR,
                "bar_navigation",
                "bar_status",
                "bar_balance",
                "bar_clock",
                "bar_ping",
                "bar_fps",
                "bar_network",
                "bar_media",
                "bar_music",
                "bar_volume",
                "bar_notifications",
                "bar_conversations",
                "bar_chat",
            ] {
                let mut bench = Bench {
                    native_top: true,
                    classic,
                    screen: Vec2::new(1600.0, 900.0),
                    ..Default::default()
                };
                bench.settings.compose_top();
                let separators_before = bench
                    .settings
                    .bar(bar, &Layout::default())
                    .iter()
                    .filter(|button| structure::is_separator(&button.command))
                    .count();
                bench.palette_drag(command, bar);
                if command == structure::SEPARATOR {
                    let count = bench
                        .settings
                        .bar(bar, &Layout::default())
                        .iter()
                        .filter(|b| structure::is_separator(&b.command))
                        .count();
                    assert_eq!(count, separators_before + 1, "separator on {bar:?}, classic={classic}");
                    // dropped beside it again, it is one with it (human
                    // request, 2026-11-25)
                    bench.palette_drag(command, bar);
                    let next = bench
                        .settings
                        .bar(bar, &Layout::default())
                        .iter()
                        .filter(|b| structure::is_separator(&b.command))
                        .count();
                    assert_eq!(next, count);
                } else {
                    assert_eq!(
                        bench.settings.location(command, &Layout::default()),
                        Some(bar),
                        "{command} on {bar:?}, classic={classic}"
                    );
                    let rect = bench.ctx.read_response(bars::tool_button_id(command)).expect("placed block").rect;
                    assert!(
                        Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(rect),
                        "visible block {command}: {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn moved_notification_bells_still_toggle_the_native_list() {
    for bar in Bar::ALL {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1600.0, 900.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        bench.state.pending = Some(Edit::Insert(bar, bars::top::button("bar_notifications", "bell-filled"), 0));
        bench.state.commit(&Layout::default(), &mut bench.settings);
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let point = bench
            .ctx
            .read_response(bars::tool_button_id("bar_notifications"))
            .expect("bell slot")
            .rect
            .center();
        for expected in [true, false] {
            bench.pointer(point, Some(true));
            bench.pointer(point, Some(false));
            assert_eq!(bench.notif.open, expected, "native bell on {bar:?}");
        }
        let bell = bench.notif.bell.expect("native bell anchor");
        assert!(Rect::from_min_size(Pos2::ZERO, bench.screen).contains_rect(bell));
    }
}

#[test]
fn composed_native_buttons_can_be_removed_by_dragging_back_to_the_editor() {
    for (command, id) in [("aurora_menu", "bars_island_menu"), ("report_bug", "bars_island_bug")] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1600.0, 900.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        bench.state.open = true;
        bench.state.search = aurora_i18n::tr!("toolbar-separator").into();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let source = bench.ctx.read_response(egui::Id::unique(id)).expect("native button").rect.center();
        let target = bench
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::unique(editor::WINDOW)))
            .expect("editor")
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source + Vec2::new(12.0, 0.0), None);
        assert!(ToolbarUi::dragging(&bench.ctx));
        for _ in 0..3 {
            bench.pointer(target, None);
        }
        bench.pointer(target, Some(false));
        assert_eq!(bench.settings.location(command, &Layout::default()), None, "remove {command}");
    }
}

#[test]
fn the_composed_top_accepts_a_command_dragged_from_the_native_aurora_menu() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            classic,
            screen: Vec2::new(1600.0, 900.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let opener = bench
            .ctx
            .read_response(egui::Id::unique("bars_island_menu"))
            .expect("Aurora button")
            .rect
            .center();
        bench.pointer(opener, Some(true));
        bench.pointer(opener, Some(false));
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let source = bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", IMPORT))))
            .expect("Aurora menu command")
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source + Vec2::new(12.0, 0.0), None);
        assert!(ToolbarUi::dragging(&bench.ctx));
        let target = egui::pos2(140.0, 26.0);
        for _ in 0..4 {
            bench.pointer(target, None);
        }
        bench.pointer(target, Some(false));
        assert_eq!(bench.settings.location(IMPORT, &Layout::default()), Some(Bar::Top));
    }
}

#[test]
fn folding_the_address_keeps_the_default_aurora_button_at_the_left() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            classic,
            screen: Vec2::new(1600.0, 900.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let left = bench
            .ctx
            .read_response(egui::Id::unique("bars_island_menu"))
            .expect("Aurora button")
            .rect
            .left();
        bench.settings.show_address_bar(false);
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let folded = bench
            .ctx
            .read_response(egui::Id::unique("bars_island_menu"))
            .expect("Aurora button")
            .rect
            .left();
        assert!((folded - left).abs() < 0.5);
        assert!(folded < 16.0);
        bench.settings.top_style.align = Some(Align::End);
        assert_eq!(bench.settings.top_style.resolve(Bar::Top, !classic).align, Align::End);
    }
}

#[test]
fn overflowing_top_blocks_keep_their_full_height_and_click_area() {
    for classic in [false, true] {
        let mut bench = Bench {
            native_top: true,
            classic,
            screen: Vec2::new(1000.0, 700.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let full_height = bench
            .ctx
            .read_response(bars::tool_button_id("bar_navigation"))
            .expect("navigation before overflow")
            .interact_rect
            .height();
        bench.conversation_count = 32;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let navigation = bench
            .ctx
            .read_response(bars::tool_button_id("bar_navigation"))
            .expect("navigation block");
        assert_eq!(navigation.rect.height(), bars::islands::HEIGHT);
        assert_eq!(
            navigation.interact_rect.height(),
            full_height,
            "overflow must not reserve a scrollbar through the controls"
        );
    }
}

#[test]
fn a_real_menu_drag_places_the_command_on_each_edge_and_keeps_its_menu_source() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    for bar in Bar::ALL {
        let mut bench = Bench::default();
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        let source = bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", IMPORT))))
            .expect("menu row")
            .center();
        bench.pointer(source, Some(true));
        bench.pointer(source + Vec2::new(12.0, 0.0), None);
        assert!(ToolbarUi::dragging(&bench.ctx), "source on {bar:?}");
        let target = match bar {
            Bar::Top => egui::pos2(118.0, 26.0),
            Bar::Left => egui::pos2(24.0, 348.0),
            Bar::Right => egui::pos2(976.0, 348.0),
            Bar::Bottom => bench
                .ctx
                .read_response(bars::tool_button_id("inventory"))
                .expect("bottom button")
                .rect
                .center(),
        };
        for _ in 0..3 {
            bench.pointer(target, None);
        }
        bench.pointer(target, Some(false));
        bench.frame(Vec::new());
        assert_eq!(bench.settings.location(IMPORT, &Layout::default()), Some(bar), "drop on {bar:?}");
        assert!(
            commands::catalogue(&bench.ctx)
                .children("", &bench.settings.menu_layout)
                .iter()
                .any(|entry| entry.button.command == IMPORT)
        );
        let json = serde_json::to_string(&bench.settings).expect("settings");
        let restored: ToolbarSettings = serde_json::from_str(&json).expect("round trip");
        assert_eq!(restored.location(IMPORT, &Layout::default()), Some(bar));
    }
}

#[test]
fn native_aurora_menu_drags_append_beyond_existing_top_buttons() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    for classic in [false, true] {
        for (parent, command) in [("", IMPORT), ("menu:common-build", IMPORT), ("", "menu:common-world")] {
            let mut bench = Bench {
                native_top: true,
                classic,
                ..Default::default()
            };
            bench.settings.top = vec![bars::top::button("location_bar", "map-pin-filled")];
            if !parent.is_empty() {
                bench.settings.menu_layout.clear();
            }
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let bar = bench
                .ctx
                .read_response(egui::Id::unique("top_toolbar_background"))
                .expect("top bar")
                .rect;
            let original_button = bench
                .ctx
                .read_response(bars::tool_button_id("location_bar"))
                .expect("existing top button")
                .rect;
            let target = if classic {
                bar.center()
            } else {
                egui::pos2(bar.right() + 24.0, bar.center().y)
            };
            let menu_id = if classic { "classic_main_menu" } else { "bars_island_menu" };
            let opener = bench
                .ctx
                .read_response(egui::Id::unique(menu_id))
                .expect("Aurora button")
                .rect
                .center();
            bench.pointer(opener, Some(true));
            bench.pointer(opener, Some(false));
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            if !parent.is_empty() {
                let heading = bench
                    .ctx
                    .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", parent))))
                    .expect("submenu heading")
                    .center();
                for _ in 0..6 {
                    bench.pointer(heading, None);
                }
            }
            let source = bench
                .ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", command))))
                .expect("native menu row")
                .center();
            bench.pointer(source, Some(true));
            bench.pointer(source + Vec2::new(12.0, 0.0), None);
            assert!(ToolbarUi::dragging(&bench.ctx), "source: {command}, classic={classic}");
            for _ in 0..4 {
                bench.pointer(target, None);
            }
            let drop = bench
                .ctx
                .read_response(egui::Id::unique("top_toolbar_drop"))
                .expect("top destination");
            assert!(drop.rect.contains(target), "top destination must include the append position");
            let existing_button = bench
                .ctx
                .read_response(bars::tool_button_id("location_bar"))
                .expect("existing top button")
                .rect;
            assert!(
                (existing_button.left() - original_button.left()).abs() < 0.5,
                "expanding the destination must preserve the existing buttons' anchor"
            );
            assert_eq!(bench.settings.top.len(), 1, "hover must not save");
            bench.pointer(target, Some(false));
            bench.frame(Vec::new());
            assert_eq!(
                bench.settings.location(command, &Layout::default()),
                Some(Bar::Top),
                "drop {command}, classic={classic}"
            );
            assert!(
                commands::catalogue(&bench.ctx).find(command).is_some(),
                "menu source remains available"
            );
        }
    }
}

#[test]
fn locked_menu_rows_do_not_start_a_layout_drag() {
    let mut bench = Bench::default();
    bench.settings.locked = true;
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", "menu:common-build/blender-live-title"))))
        .expect("menu row")
        .center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(20.0, 0.0), None);
    assert!(!ToolbarUi::dragging(&bench.ctx));
}

#[test]
fn a_menu_entry_right_click_has_no_display_choices_and_no_label_tooltip() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    let mut bench = Bench::default();
    // a mode saved before the choice was removed is ignored
    bench.settings.appearance.insert(
        appearance::menu_key(IMPORT),
        appearance::Appearance {
            mode: Some(ButtonMode::IconsOnly),
            icon: None,
        },
    );
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", IMPORT))))
        .expect("menu row")
        .center();
    // hovering long enough for a tooltip shows the label once, on the row
    for _ in 0..12 {
        bench.frame(vec![egui::Event::PointerMoved(source)]);
    }
    let labels = bench
        .text
        .iter()
        .filter(|(text, _)| text == aurora_i18n::tr!("blender-live-title"))
        .count();
    assert_eq!(labels, 1, "the row shows its label and no tooltip repeats it");
    for pressed in [true, false] {
        bench.frame(vec![
            egui::Event::PointerMoved(source),
            egui::Event::PointerButton {
                pos: source,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(
        bench
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::unique("toolbar_button_appearance")))
            .is_none(),
        "normal context choices do not open an appearance floater"
    );
    // the Aurora menu's rows always show icon and label: no display choice
    for key in ["toolbar-button-icons", "toolbar-button-text", "toolbar-button-icons-text"] {
        assert!(
            !bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key)),
            "menu row offers {key}"
        );
    }
    assert!(
        bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-button-edit")),
        "its icon stays editable, in its window"
    );
    assert!(commands::take_request(&bench.ctx).is_none(), "customizing never activates LiveMesh");
}

#[test]
fn a_real_menu_drag_can_return_an_entry_to_another_submenu() {
    const IMPORT: &str = "menu:common-build/blender-live-title";
    let mut bench = Bench::default();
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let rect = |bench: &Bench, id: &str| {
        bench
            .ctx
            .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", id))))
            .expect("menu entry")
    };
    let source = rect(&bench, IMPORT).center();
    let build = rect(&bench, "menu:common-build").center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(12.0, 0.0), None);
    for _ in 0..6 {
        bench.pointer(build, None);
    }
    let child = rect(&bench, "menu:common-build/poser-title").center_top() + Vec2::new(0.0, 3.0);
    for _ in 0..3 {
        bench.pointer(child, None);
    }
    bench.pointer(child, Some(false));
    let catalogue = commands::catalogue(&bench.ctx);
    assert!(
        !catalogue
            .children("", &bench.settings.menu_layout)
            .iter()
            .any(|entry| entry.button.command == IMPORT)
    );
    let children = catalogue.children("menu:common-build", &bench.settings.menu_layout);
    let inserted = children
        .iter()
        .position(|entry| entry.button.command == IMPORT)
        .expect("moved Import");
    assert_eq!(children[inserted + 1].button.command, "menu:common-build/poser-title");
}

#[test]
fn menu_row_right_click_adds_and_removes_its_toolbar_command_without_running_it() {
    fn click(bench: &mut Bench, point: Pos2, button: egui::PointerButton) {
        for pressed in [true, false] {
            bench.frame(vec![
                egui::Event::PointerMoved(point),
                egui::Event::PointerButton {
                    pos: point,
                    button,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
    }
    // the islands' wolf menu, a remembered dropdown like in the viewer: a
    // context menu nested in it used to replace it and vanish with it
    fn open_wolf(bench: &mut Bench) {
        let wolf = bench
            .ctx
            .read_response(Id::unique("bars_island_menu"))
            .expect("wolf button")
            .rect
            .center();
        click(bench, wolf, egui::PointerButton::Primary);
        assert!(egui::Popup::is_any_open(&bench.ctx), "the wolf menu opens");
    }
    let shown = |bench: &Bench, key: &'static str| bench.text.iter().any(|(text, _)| text == aurora_i18n::text(key));
    let layout = Layout::default();
    let mut bench = Bench {
        native_top: true,
        menu_open: false,
        ..Default::default()
    };
    bench.settings.locked = false;
    // a row with no toolbar command of its own has no Add entry
    let plain = "menu:common-me/avatar-health-menu/avatar-health-stop-animations";
    bench
        .settings
        .menu_layout
        .insert(String::new(), vec!["menu:common-build/blender-live-title".into(), plain.into()]);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.settings.location("blender_live", &layout), None);
    for remove in [false, true] {
        open_wolf(&mut bench);
        let import = menu_point(&bench, aurora_i18n::tr!("blender-live-title"));
        click(&mut bench, import, egui::PointerButton::Secondary);
        let (key, other) = if remove {
            ("toolbar-remove-from-menu", "toolbar-add-from-menu")
        } else {
            ("toolbar-add-from-menu", "toolbar-remove-from-menu")
        };
        assert!(shown(&bench, key), "remove={remove}");
        assert!(!shown(&bench, other), "remove={remove}");
        assert!(
            shown(&bench, "blender-live-title"),
            "the wolf menu stays open behind its row's context, remove={remove}"
        );
        let entry = menu_point(&bench, aurora_i18n::text(key));
        click(&mut bench, entry, egui::PointerButton::Primary);
        assert!(!shown(&bench, key), "the context closes after its choice");
        assert!(!shown(&bench, "blender-live-title"), "and the wolf menu with it");
        assert_eq!(commands::take_request(&bench.ctx), None, "a toolbar edit never runs LiveMesh");
        assert!(matches!(bench.action, bars::BarAction::None));
        assert_eq!(
            bench.settings.location("blender_live", &layout),
            if remove { None } else { Some(Bar::Bottom) }
        );
        if !remove {
            let button = bench.settings.bar(Bar::Bottom, &layout).last().expect("added button");
            assert_eq!(button.command, "blender_live", "the command, never its menu path");
            assert!(button.tooltip.is_empty());
        }
    }
    open_wolf(&mut bench);
    let row = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(Id::unique(("main_menu_entry", plain))))
        .expect("plain row")
        .center();
    click(&mut bench, row, egui::PointerButton::Secondary);
    assert!(!shown(&bench, "toolbar-add-from-menu"));
    assert!(shown(&bench, "toolbar-button-edit"), "its context still opens");
    // hovering another row of the menu closes the context, the menu stays
    let import = menu_point(&bench, aurora_i18n::tr!("blender-live-title"));
    for _ in 0..3 {
        bench.frame(vec![egui::Event::PointerMoved(import)]);
    }
    assert!(!shown(&bench, "toolbar-button-edit"), "the context closes on another row");
    assert!(shown(&bench, "blender-live-title"), "the wolf menu stays open");
    // locked bars keep the entry, greyed out, and ignore it
    bench.settings.locked = true;
    let import = menu_point(&bench, aurora_i18n::tr!("blender-live-title"));
    click(&mut bench, import, egui::PointerButton::Secondary);
    let entry = menu_point(&bench, aurora_i18n::tr!("toolbar-add-from-menu"));
    click(&mut bench, entry, egui::PointerButton::Primary);
    assert_eq!(bench.settings.location("blender_live", &layout), None);
}

/// A toolbar shortcut clicks its button where it is drawn: a drawer's
/// button opens it, a button in it runs its command; a button not drawn (in
/// the closed drawer) runs like its menu entry (human request, 2026-11-25).
#[test]
fn a_toolbar_shortcut_clicks_its_button_or_runs_its_menu_entry() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    let id = drawers::create(&mut bench.settings, Bar::Bottom, &Layout::default());
    bench.state.pending = Some(Edit::DrawerInsert(id.clone(), bars::top::button("inventory", "folder-filled"), 0));
    assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    assert!(!drawers::is_open(&bench.ctx, &id));
    shortcuts::press(&bench.ctx, "inventory".into());
    bench.frame(Vec::new());
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::None);
    assert_eq!(
        commands::take_request(&bench.ctx).as_deref(),
        Some("inventory"),
        "not drawn: the menu's way"
    );
    shortcuts::press(&bench.ctx, id.clone());
    bench.frame(Vec::new());
    assert!(drawers::is_open(&bench.ctx, &id), "its button clicked");
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let before = bench.panels.inventory;
    shortcuts::press(&bench.ctx, "inventory".into());
    bench.frame(Vec::new());
    assert_ne!(bench.panels.inventory, before, "drawn: clicked");
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::None);
    assert_eq!(commands::take_request(&bench.ctx), None);
}

/// A drag on a floating block's button moves the whole block, never the
/// button (human request, 2026-11-25).
#[test]
fn a_drag_on_a_floating_blocks_button_moves_the_block() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    let id = drawers::create_floating(&mut bench.settings, "Tools");
    bench.state.pending = Some(Edit::DrawerInsert(id.clone(), bars::top::button("inventory", "folder-filled"), 0));
    assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let area = |bench: &Bench| {
        bench
            .ctx
            .memory(|memory| memory.area_rect(Id::unique(floating::window_id(&id))))
            .expect("floating block drawn")
    };
    let before = area(&bench);
    // a row is as tall as its buttons, so that it goes to the screen's top
    // and bottom (human report, 2026-11-25)
    assert!(before.height() < 80.0, "a row's height: {before:?}");
    let start = bench
        .ctx
        .read_response(bars::tool_button_id("inventory"))
        .expect("its button")
        .rect
        .center();
    let inventory = bench.panels.inventory;
    bench.pointer(start, Some(true));
    for step in 1..=6 {
        bench.pointer(start + Vec2::new(20.0, 10.0) * step as f32, None);
        assert!(!ToolbarUi::dragging(&bench.ctx), "the button is not taken");
    }
    bench.pointer(start + Vec2::new(120.0, 60.0), Some(false));
    bench.frame(Vec::new());
    let moved = area(&bench).min - before.min;
    assert!(moved.x > 80.0 && moved.y > 40.0, "moved by {moved:?}");
    assert_eq!(bench.panels.inventory, inventory, "a drag is no click");
    assert_eq!(bench.settings.drawers[&id].buttons.len(), 1);
}

/// While the bars are edited, a floating block's separator and divider move
/// like its buttons, never the block (human request, 2026-11-25).
#[test]
fn an_edited_floating_blocks_separators_move_like_its_buttons() {
    for gap in ["separator", "divider"] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        let id = drawers::create_floating(&mut bench.settings, "Tools");
        let middle = if gap == "separator" {
            structure::separator("gap")
        } else {
            structure::divider("gap")
        };
        bench.settings.drawers.get_mut(&id).expect("floating block").buttons =
            vec![tests::button("inventory"), middle.clone(), tests::button("people")];
        bench.state.open = true;
        // beside the editor's window, which stands in the middle
        bench.state.floating_placed = Some((id.clone(), egui::pos2(80.0, 200.0)));
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let before = bench
            .ctx
            .memory(|memory| memory.area_rect(Id::unique(floating::window_id(&id))))
            .expect("floating block drawn");
        start_toolbar_drag(&mut bench, &middle.command);
        let payload = egui::DragAndDrop::payload::<Drag>(&bench.ctx).expect("its drag");
        assert_eq!(payload.button.command, middle.command, "{gap}");
        for step in 1..=4 {
            bench.pointer(before.center() + Vec2::new(0.0, 20.0) * step as f32, None);
        }
        let after = bench
            .ctx
            .memory(|memory| memory.area_rect(Id::unique(floating::window_id(&id))))
            .expect("floating block drawn");
        assert_eq!(after.min, before.min, "the block stays: {gap}");
    }
}

/// A floating block that closes hides once one of its buttons is used, as a
/// drawer folds; one that does not stays (human request, 2026-11-25).
#[test]
fn a_floating_block_that_closes_hides_after_a_click_on_its_button() {
    for closes in [false, true] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        let id = drawers::create_floating(&mut bench.settings, "Tools");
        let drawer = bench.settings.drawers.get_mut(&id).expect("floating block");
        drawer.buttons = vec![tests::button("inventory")];
        drawer.floating.as_mut().expect("floating").closes = closes;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let point = bench
            .ctx
            .read_response(bars::tool_button_id("inventory"))
            .expect("its button")
            .rect
            .center();
        bench.pointer(point, Some(true));
        bench.pointer(point, Some(false));
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        assert_eq!(
            bench.settings.drawers[&id].floating.as_ref().map(|floating| floating.shown),
            Some(!closes),
            "closes={closes}"
        );
    }
}

/// A floating block reaches the top and the bottom of the room the windows
/// move in, in a row as in a column (human report, 2026-11-25).
#[test]
fn a_floating_block_reaches_the_rooms_top_and_bottom_in_both_orientations() {
    for vertical in [true, false, true] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        let id = drawers::create_floating(&mut bench.settings, "Tools");
        let drawer = bench.settings.drawers.get_mut(&id).expect("floating block");
        drawer.buttons = vec![tests::button("inventory"), tests::button("people"), tests::button("preferences")];
        drawer.floating.as_mut().expect("floating").vertical = vertical;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        // turned the other way, as in its settings
        bench
            .settings
            .drawers
            .get_mut(&id)
            .expect("floating block")
            .floating
            .as_mut()
            .expect("floating")
            .vertical = !vertical;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let vertical = !vertical;
        let window = floating::window_id(&id);
        let rect = |bench: &Bench| {
            bench
                .ctx
                .memory(|memory| memory.area_rect(Id::unique(&window)))
                .expect("floating block drawn")
        };
        let room = crate::ui::layout::move_area(&bench.ctx);
        for y in [-500.0, 3000.0] {
            crate::ui::widgets::move_floater(&bench.ctx, &window, egui::pos2(600.0, y));
            for _ in 0..3 {
                bench.frame(Vec::new());
            }
            let at = rect(&bench);
            if vertical {
                assert!(at.height() > at.width(), "a column: {at:?}");
            } else {
                assert!(at.height() < 80.0, "a row as tall as its buttons: {at:?}");
            }
            if y < 0.0 {
                assert_eq!(at.top(), room.top(), "vertical={vertical}");
            } else {
                assert_eq!(at.bottom(), room.bottom(), "vertical={vertical}");
            }
        }
    }
}

fn right_click_at(bench: &mut Bench, point: Pos2) {
    for pressed in [true, false] {
        bench.frame(vec![
            egui::Event::PointerMoved(point),
            egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Default::default(),
            },
        ]);
    }
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
}

/// "Edit block…" opens a fixed block's settings too, as a drawer's: its
/// display, its alignment and its size (human request, 2026-11-26).
#[test]
fn a_fixed_blocks_edit_entry_opens_its_settings_window() {
    let mut bench = Bench {
        native_top: true,
        menu_open: false,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let point = bench
        .ctx
        .read_response(bars::tool_button_id("inventory"))
        .expect("inventory button")
        .rect
        .center();
    right_click_at(&mut bench, point);
    let kinds = menu_point(&bench, aurora_i18n::tr!("toolbar-block-kind-floating"));
    let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-block-edit"));
    assert!(edit.y > kinds.y, "after the block's type");
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let (_, command, base) = bench.state.block_editor.clone().expect("the block's window");
    assert_eq!(command, "inventory");
    assert!(bench.state.drawer_editor.is_none());
    // the same fields as a drawer's: name, icon, shortcut, then the fixed
    // and the floating modes' options (human request, 2026-11-26)
    for key in [
        "toolbar-block-name",
        "toolbar-icon-change",
        "toolbar-block-shortcut",
        "toolbar-drawer-default-mode",
        "toolbar-block-fixed-options",
        "toolbar-block-alignment",
        "toolbar-block-sizing",
        "toolbar-block-floating-options",
        "toolbar-floating-orientation",
        "toolbar-drawer-hides-button",
    ] {
        scroll_window_to(&mut bench, aurora_i18n::text(key));
    }
    let current = blocks::style(&bench.ctx, "inventory", base).sizing;
    let (value, _, label) = super::sizing_rows()
        .into_iter()
        .find(|(value, _, _)| *value != current)
        .expect("another size");
    scroll_window_to(&mut bench, label);
    let heading = menu_point(&bench, aurora_i18n::tr!("toolbar-block-sizing"));
    let choice = bench
        .text
        .iter()
        .find(|(text, rect)| text == label && rect.center().y > heading.y)
        .expect("size radio")
        .1
        .center();
    bench.pointer(choice, Some(true));
    bench.pointer(choice, Some(false));
    for _ in 0..2 {
        bench.frame(Vec::new());
    }
    let saved = bench.settings.block_styles["inventory"];
    assert_eq!(saved.sizing, Some(value));
    assert!(saved.mode.is_none() && saved.align.is_none(), "only what changed: {saved:?}");
    // the drawer's entry takes the window over
    bench.state.edit_drawer("drawer:none");
    assert!(bench.state.block_editor.is_none());
}

/// A floating block's right click has "Edit block…" and no Delete: its
/// window keeps it (human request, 2026-11-26).
#[test]
fn a_floating_blocks_right_click_edits_it_and_does_not_delete_it() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1920.0, 1080.0),
        ..Default::default()
    };
    bench.settings.compose_top();
    let id = drawers::create_floating(&mut bench.settings, "Tools");
    bench.settings.drawers.get_mut(&id).expect("floating block").buttons = vec![tests::button("inventory")];
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let glass = bench
        .ctx
        .memory(|memory| memory.area_rect(Id::unique(floating::window_id(&id))))
        .expect("floating block drawn");
    let button = bench.ctx.read_response(bars::tool_button_id("inventory")).expect("its button").rect;
    right_click_at(&mut bench, egui::pos2((glass.left() + button.left()) / 2.0, button.center().y));
    assert!(
        bench
            .text
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("toolbar-block-kind-floating"))
    );
    let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-block-edit"));
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.state.drawer_editor.as_deref(), Some(id.as_str()));
    assert!(
        bench
            .text
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("toolbar-block-edit-title"))
    );
}

/// "Hide the button while floating" leaves a floating block's button out of
/// its bar outside interface editing, once the block has a shortcut (human
/// request, 2026-11-26).
#[test]
fn a_floating_blocks_button_hides_once_asked_and_its_block_has_a_shortcut() {
    for (hides, shortcut, editing) in [(true, true, false), (true, true, true), (true, false, false), (false, true, false)] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        bench
            .settings
            .set_bar(Bar::Top, vec![tests::button("inventory"), tests::button("people")]);
        let id = drawers::float(&mut bench.settings, &crate::ui::skin::Layout::default(), Bar::Top, "inventory").expect("floated");
        bench.settings.drawers.get_mut(&id).expect("floating block").hides_button = hides;
        if shortcut {
            bench.settings.shortcuts.insert(
                id.clone(),
                crate::keybinds::Binding {
                    input: crate::keybinds::Input::Key("KeyJ".into()),
                    ctrl: true,
                    shift: false,
                    alt: false,
                },
            );
        }
        bench.state.open = editing;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let shown = bench
            .ctx
            .read_response(bars::tool_button_id(&id))
            .is_some_and(|response| response.rect.is_positive());
        assert_eq!(
            shown,
            !(hides && shortcut && !editing),
            "hides={hides}, shortcut={shortcut}, editing={editing}"
        );
    }
}

/// "Add a close button" ends a floating block with a Close button, an X,
/// "Close" or both as its buttons show, that hides the block; not while the
/// bars are edited (human request, 2026-11-26).
#[test]
fn a_floating_blocks_close_button_ends_its_buttons_and_hides_it() {
    let close = aurora_i18n::tr!("common-close");
    for (mode, editing) in [
        (ButtonMode::IconsOnly, false),
        (ButtonMode::TextOnly, false),
        (ButtonMode::IconsWithText, false),
        (ButtonMode::IconsWithText, true),
    ] {
        let mut bench = Bench {
            native_top: true,
            screen: Vec2::new(1920.0, 1080.0),
            ..Default::default()
        };
        bench.settings.compose_top();
        bench
            .settings
            .set_bar(Bar::Top, vec![tests::button("inventory"), tests::button("people")]);
        let id = drawers::float(&mut bench.settings, &crate::ui::skin::Layout::default(), Bar::Top, "inventory").expect("floated");
        let drawer = bench.settings.drawers.get_mut(&id).expect("floating block");
        drawer.close_button = true;
        drawer.mode = mode;
        bench.state.open = editing;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let command = drawers::close_button(&id).command;
        let button = bench.ctx.read_response(bars::tool_button_id(&command));
        assert_eq!(button.is_some(), !editing, "{mode:?}, editing={editing}");
        let Some(button) = button else { continue };
        let last = bench
            .ctx
            .read_response(bars::tool_button_id("inventory"))
            .expect("the block's button")
            .rect;
        assert!(button.rect.left() >= last.right(), "after the others: {:?} {last:?}", button.rect);
        let labelled = bench
            .text
            .iter()
            .any(|(text, rect)| text == close && button.rect.contains(rect.center()));
        assert_eq!(labelled, mode != ButtonMode::IconsOnly, "{mode:?}");
        assert!(bench.settings.drawers[&id].floating.as_ref().is_some_and(|floating| floating.shown));
        bench.pointer(button.rect.center(), Some(true));
        bench.pointer(button.rect.center(), Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert!(
            bench.settings.drawers[&id]
                .floating
                .as_ref()
                .is_some_and(|floating| !floating.shown),
            "{mode:?}: the click hides the block"
        );
    }
}

/// The catalogue's "Interfaces" becomes a drawer whose buttons are the
/// presets its eyes keep; a click applies one, and nothing can be dropped
/// into it (human request, 2026-11-26).
#[test]
fn the_interfaces_block_is_a_drawer_of_the_presets_its_eyes_keep() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.buttons = Some(Vec::new());
    bench.settings.top = vec![tests::button("inventory")];
    let entry = catalogue::find(drawers::INTERFACES).expect("in the catalogue");
    assert_eq!(entry.label(), aurora_i18n::tr!("toolbar-interfaces"));
    let toggle = ToolButton {
        command: drawers::INTERFACES.into(),
        icon: entry.icon.into(),
        tooltip: String::new(),
    };
    bench.state.pending = Some(Edit::Insert(Bar::Left, toggle.clone(), 0));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(bench.settings.drawers.contains_key(drawers::INTERFACES), "a drawer");
    assert_eq!(bench.settings.location(drawers::INTERFACES, &Layout::default()), Some(Bar::Left));
    assert_eq!(label(&toggle), aurora_i18n::tr!("toolbar-interfaces"));
    bench.settings.hidden_interfaces.insert(presets::Preset::Build.key().into());
    drawers::set_open(&bench.ctx, drawers::INTERFACES, true);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let shown = |bench: &Bench, preset: presets::Preset| {
        bench
            .ctx
            .read_response(bars::tool_button_id(&format!("{}{}", drawers::INTERFACE_PREFIX, preset.key())))
            .is_some_and(|response| response.rect.is_positive())
    };
    for preset in presets::Preset::ALL {
        assert_eq!(shown(&bench, preset), preset != presets::Preset::Build, "{preset:?}");
    }
    let old_school = bench
        .ctx
        .read_response(bars::tool_button_id(&format!(
            "{}{}",
            drawers::INTERFACE_PREFIX,
            presets::Preset::OldSchool.key()
        )))
        .expect("Old School")
        .rect
        .center();
    bench.pointer(old_school, Some(true));
    bench.pointer(old_school, Some(false));
    assert_eq!(bench.state.preset_request, Some(presets::Preset::OldSchool), "a click applies it");
    // it keeps no buttons and takes no drop
    let drag = Drag {
        button: tests::button("people"),
        from: Some(Bar::Top),
        members: Vec::new(),
    };
    assert!(drawers::insertion_edit(drawers::INTERFACES, 0, &drag).is_none());
    let mut settings = bench.settings.clone();
    let mut state = ToolbarUi {
        pending: Some(Edit::DrawerInsert(drawers::INTERFACES.into(), tests::button("people"), 0)),
        ..Default::default()
    };
    assert!(!state.commit(&Layout::default(), &mut settings));
    settings.drawers.get_mut(drawers::INTERFACES).expect("drawer").buttons = vec![tests::button("people")];
    settings.sanitize();
    assert!(settings.drawers[drawers::INTERFACES].buttons.is_empty());
}

/// "Interfaces" sits before "Separator" in the window, out of the searched
/// catalogue, and rests once placed (human request, 2026-11-26).
#[test]
fn the_interfaces_tile_sits_before_the_separator_and_rests_once_placed() {
    let mut bench = Bench {
        screen: Vec2::new(1600.0, 1000.0),
        ..Default::default()
    };
    bench.state.open = true;
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let tile = |bench: &Bench, command: &str| {
        bench
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_catalogue", command))))
            .unwrap_or_else(|| panic!("{command}"))
    };
    let interfaces = tile(&bench, drawers::INTERFACES);
    let separator = tile(&bench, structure::DIVIDER);
    assert!(interfaces.right() <= separator.left(), "{interfaces:?} before {separator:?}");
    assert!((interfaces.center().y - separator.center().y).abs() < 1.0, "on its row");
    let entries = catalogue_view::entries(&Layout::default(), &bench.settings, &Default::default());
    assert!(!entries.iter().any(|entry| entry.button.command == drawers::INTERFACES));
    // a click places it, in the bottom bar as the separator's does
    bench.pointer(interfaces.center(), Some(true));
    bench.pointer(interfaces.center(), Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(bench.settings.drawers.contains_key(drawers::INTERFACES));
    assert!(bench.settings.location(drawers::INTERFACES, &Layout::default()).is_some());
    let placed = bench.settings.clone();
    let interfaces = tile(&bench, drawers::INTERFACES);
    bench.pointer(interfaces.center(), Some(true));
    bench.pointer(interfaces.center(), Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert_eq!(bench.settings, placed, "placed, its tile does nothing");
}

/// The Interfaces block stays, a drawer: neither removed nor made a fixed
/// or a floating block, and its "Edit block…" changes only its name and
/// its icon (human request, 2026-11-26).
#[test]
fn the_interfaces_block_stays_and_only_its_name_and_icon_change() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.buttons = Some(Vec::new());
    bench.settings.top = vec![tests::button("inventory")];
    let toggle = ToolButton {
        command: drawers::INTERFACES.into(),
        icon: drawers::INTERFACES_ICON.into(),
        tooltip: String::new(),
    };
    bench.state.pending = Some(Edit::Insert(Bar::Top, toggle, 1));
    assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
    let layout = Layout::default();
    for edit in [
        Edit::Remove(drawers::INTERFACES.into()),
        Edit::Unfold(Bar::Top, drawers::INTERFACES.into()),
        Edit::Float(Bar::Top, drawers::INTERFACES.into(), Pos2::new(400.0, 400.0)),
    ] {
        bench.state.pending = Some(edit);
        assert!(!bench.state.commit(&layout, &mut bench.settings));
    }
    // a block's removal keeps it
    bench.state.pending = Some(Edit::RemoveBlock(vec!["inventory".into(), drawers::INTERFACES.into()]));
    assert!(bench.state.commit(&layout, &mut bench.settings));
    assert_eq!(bench.settings.location(drawers::INTERFACES, &layout), Some(Bar::Top));
    assert_eq!(bench.settings.location("inventory", &layout), None);
    let before = bench.settings.drawers[drawers::INTERFACES].clone();
    bench.state.pending = Some(block_window::edit_of(
        &bench.settings,
        &layout,
        block_window::Target::Drawer(drawers::INTERFACES.into()),
        |o| {
            o.name = "Looks".into();
            o.look.icon = Some("star-filled".into());
            o.look.mode = Some(ButtonMode::TextOnly);
            o.mode = ButtonMode::TextOnly;
            o.closes = !o.closes;
            o.vertical = !o.vertical;
            o.hides_button = !o.hides_button;
            o.close_button = !o.close_button;
        },
    ));
    assert!(bench.state.commit(&layout, &mut bench.settings));
    assert_eq!(bench.settings.drawers[drawers::INTERFACES], before, "nothing else");
    let look = &bench.settings.appearance[drawers::INTERFACES];
    assert_eq!((look.icon.as_deref(), look.mode), (Some("star-filled"), None));
    let placed = bench
        .settings
        .top
        .iter()
        .find(|button| button.command == drawers::INTERFACES)
        .expect("placed");
    assert_eq!(placed.tooltip, "Looks");
    // its right click has no type to choose
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let point = bench
        .ctx
        .read_response(bars::tool_button_id(drawers::INTERFACES))
        .expect("its button")
        .rect
        .center();
    right_click_at(&mut bench, point);
    assert!(bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-block-edit")));
    assert!(
        !bench
            .text
            .iter()
            .any(|(text, _)| text == aurora_i18n::text("toolbar-block-kind-fixed"))
    );
}

/// Each interface takes a shortcut from its right click's "Edit button…",
/// which applies it even with the block closed, while the block is placed
/// and its eye open; its right click has the sections of any button's, its
/// block's included (human request, 2026-11-26).
#[test]
fn an_interfaces_shortcut_applies_it_from_its_right_click_even_closed() {
    let mut bench = Bench {
        native_top: true,
        screen: Vec2::new(1600.0, 900.0),
        ..Default::default()
    };
    bench.settings.composed = true;
    bench.settings.buttons = Some(Vec::new());
    bench.settings.top = vec![ToolButton {
        command: drawers::INTERFACES.into(),
        icon: drawers::INTERFACES_ICON.into(),
        tooltip: String::new(),
    }];
    bench.settings.drawers.insert(drawers::INTERFACES.into(), Default::default());
    let key = format!("{}{}", drawers::INTERFACE_PREFIX, presets::Preset::OldSchool.key());
    drawers::set_open(&bench.ctx, drawers::INTERFACES, true);
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let point = bench
        .ctx
        .read_response(bars::tool_button_id(&key))
        .expect("Old School")
        .rect
        .center();
    right_click_at(&mut bench, point);
    assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-remove")));
    let block = aurora_i18n::tr!("toolbar-block-settings").to_uppercase();
    assert!(bench.text.iter().any(|(text, _)| *text == block), "its block's settings");
    let row = menu_point(&bench, aurora_i18n::tr!("toolbar-button-edit"));
    bench.pointer(row, Some(true));
    bench.pointer(row, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    // its window: its shortcut only, its name and icon its interface's
    assert!(bench.ctx.read_response(Id::unique(("button_name", key.as_str()))).is_none());
    let edit = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(Id::unique(("drawer_shortcut_edit", key.as_str()))))
        .expect("its shortcut row")
        .center();
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    assert_eq!(
        bench.state.shortcut_editor.as_ref().map(|editor| editor.key.as_str()),
        Some(key.as_str())
    );
    assert_eq!(bench.state.preset_request, None, "the right click does not apply it");
    bench.state.shortcut_editor = None;
    let binding = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyO".into()),
        ctrl: true,
        shift: true,
        alt: false,
    };
    bench.state.pending = Some(Edit::Shortcut(key.clone(), Some(binding.clone())));
    assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
    let layout = Layout::default();
    assert_eq!(shortcuts::target(&bench.settings, &layout, &binding), Some(key.clone()));
    assert_eq!(shortcuts::name(&bench.settings, &layout, &key), presets::Preset::OldSchool.label());
    // drawn, its button takes it
    bench.pointer(point, None);
    drawers::set_open(&bench.ctx, drawers::INTERFACES, true);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(
        bench
            .ctx
            .read_response(bars::tool_button_id(&key))
            .is_some_and(|response| response.rect.is_positive())
    );
    shortcuts::press(&bench.ctx, key.clone());
    bench.frame(Vec::new());
    assert_eq!(bench.state.preset_request.take(), Some(presets::Preset::OldSchool));
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::None);
    // closed, the frame hands it on
    drawers::set_open(&bench.ctx, drawers::INTERFACES, false);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    shortcuts::press(&bench.ctx, key.clone());
    bench.frame(Vec::new());
    assert_eq!(
        shortcuts::fallback(&bench.ctx, &mut bench.settings),
        shortcuts::Fallback::Interface(presets::Preset::OldSchool.key().into())
    );
    bench.state.interface_clicked(presets::Preset::OldSchool.key());
    assert_eq!(bench.state.preset_request, Some(presets::Preset::OldSchool));
    // its eye closed, or the block gone, it does nothing
    bench.settings.hidden_interfaces.insert(presets::Preset::OldSchool.key().into());
    assert_eq!(shortcuts::target(&bench.settings, &layout, &binding), None);
    bench.settings.hidden_interfaces.clear();
    bench.settings.top.clear();
    assert_eq!(shortcuts::target(&bench.settings, &layout, &binding), None);
}

/// A right click shows "Button settings", then "Block settings", then
/// "Parent block settings" for a block's button standing in a block, the
/// bars edited or not (human request, 2026-11-26).
#[test]
fn a_right_click_shows_the_button_its_block_and_the_parent_block_edited_or_not() {
    let headings = |bench: &Bench| {
        ["toolbar-button-settings", "toolbar-block-settings", "toolbar-parent-block-settings"].map(|key| {
            let heading = aurora_i18n::text(key).to_uppercase();
            bench
                .text
                .iter()
                .rev()
                .find(|(text, _)| text.to_uppercase() == heading)
                .map(|(_, rect)| rect.center().y)
        })
    };
    for editing in [false, true] {
        for inside in [false, true] {
            let mut bench = Bench {
                native_top: true,
                screen: Vec2::new(1600.0, 900.0),
                ..Default::default()
            };
            bench.settings.compose_top();
            let id = drawers::create(&mut bench.settings, Bar::Bottom, &Layout::default());
            bench.state.pending = Some(Edit::DrawerInsert(id.clone(), bars::top::button("inventory", "folder-filled"), 0));
            assert!(bench.state.commit(&Layout::default(), &mut bench.settings));
            bench.state.open = editing;
            if inside {
                drawers::set_open(&bench.ctx, &id, true);
            }
            for _ in 0..4 {
                bench.frame(Vec::new());
            }
            let clicked = if inside { "inventory" } else { id.as_str() };
            let point = bench
                .ctx
                .read_response(bars::tool_button_id(clicked))
                .expect("its button")
                .rect
                .center();
            right_click_at(&mut bench, point);
            let [button, block, parent] = headings(&bench);
            let (block, parent) = (block.expect("block settings"), parent.expect("parent block settings"));
            assert!(block < parent, "editing={editing}, inside={inside}");
            if inside {
                assert!(button.expect("button settings") < block, "editing={editing}");
            } else {
                assert_eq!(button, None, "a drawer's button is its block");
            }
        }
    }
}

/// A bar's own control (the clock, the address bar…) has a button's right
/// click, its own actions first (human request, 2026-11-26): its code draws
/// its menu, whose choices the bars take on the next frame.
#[test]
fn a_native_controls_right_click_hands_its_choices_to_the_bars() {
    let layout = Layout::default();
    for editing in [false, true] {
        let mut bench = Bench {
            native_top: true,
            menu_open: false,
            screen: Vec2::new(1920.0, 1080.0),
            net_stats: true,
            ..Default::default()
        };
        bench.settings.compose_top();
        bench.settings.locked = false;
        bench.state.open = editing;
        for _ in 0..4 {
            bench.frame(Vec::new());
        }
        let point = bench
            .ctx
            .read_response(bars::tool_button_id("bar_clock"))
            .expect("the clock")
            .rect
            .center();
        right_click_at(&mut bench, point);
        let edit = menu_point(&bench, aurora_i18n::tr!("toolbar-block-edit"));
        bench.pointer(edit, Some(true));
        bench.pointer(edit, Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert!(
            bench
                .state
                .block_editor
                .as_ref()
                .is_some_and(|(_, command, _)| command == "bar_clock")
                || bench.state.drawer_editor.is_some(),
            "editing={editing}: its block's window"
        );
        bench.state.block_editor = None;
        bench.state.drawer_editor = None;
        for _ in 0..2 {
            bench.frame(Vec::new());
        }
        right_click_at(&mut bench, point);
        let remove = menu_point(&bench, aurora_i18n::tr!("toolbar-remove"));
        bench.pointer(remove, Some(true));
        bench.pointer(remove, Some(false));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
        assert_eq!(bench.settings.location("bar_clock", &layout), None, "editing={editing}: removed");
    }
}

/// A fixed block's shortcut shows and hides it in its bar; editing the
/// interface shows it, and it shows again once its shortcut is gone (human
/// request, 2026-11-26).
#[test]
fn a_fixed_blocks_shortcut_hides_and_shows_it_outside_interface_editing() {
    let layout = Layout::default();
    let mut bench = block_controls_fixture(Bar::Bottom, false, false);
    bench.state.open = false;
    let drawn = |bench: &Bench, command: &str| {
        bench
            .ctx
            .read_response(bars::tool_button_id(command))
            .is_some_and(|response| response.rect.is_positive())
    };
    let key = shortcuts::block_of(&bench.settings, bench.settings.bar(Bar::Bottom, &layout), "people").expect("its key");
    let binding = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyJ".into()),
        ctrl: true,
        shift: true,
        alt: false,
    };
    bench.state.pending = Some(Edit::Shortcut(key.clone(), Some(binding.clone())));
    assert!(bench.state.commit(&layout, &mut bench.settings));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(drawn(&bench, "inventory") && drawn(&bench, "people"));
    assert_eq!(shortcuts::target(&bench.settings, &layout, &binding), Some(key.clone()));
    shortcuts::press(&bench.ctx, key.clone());
    bench.frame(Vec::new());
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::Changed);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(!drawn(&bench, "inventory") && !drawn(&bench, "people"), "the whole block hidden");
    assert!(drawn(&bench, "preferences"), "the next block stays");
    // editing the interface shows it
    bench.state.open = true;
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(drawn(&bench, "inventory") && drawn(&bench, "people"), "shown while editing");
    bench.state.open = false;
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(!drawn(&bench, "inventory"), "hidden again after editing");
    // again: shown
    shortcuts::press(&bench.ctx, key.clone());
    bench.frame(Vec::new());
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::Changed);
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(drawn(&bench, "inventory") && drawn(&bench, "people"));
    // hidden, then its shortcut removed: shown
    shortcuts::press(&bench.ctx, key.clone());
    bench.frame(Vec::new());
    shortcuts::fallback(&bench.ctx, &mut bench.settings);
    assert!(bench.settings.hidden_blocks.contains(&key));
    bench.state.pending = Some(Edit::Shortcut(key.clone(), None));
    assert!(bench.state.commit(&layout, &mut bench.settings));
    assert!(bench.settings.hidden_blocks.is_empty());
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(drawn(&bench, "inventory") && drawn(&bench, "people"));
}

/// A shortcut another has: a block can replace it or join other blocks
/// that have it, a button or a block a button's key only replace it, and
/// Cancel leaves it (human request, 2026-11-26).
#[test]
fn the_shortcut_window_replaces_adds_or_cancels_a_taken_shortcut() {
    let layout = Layout::default();
    let mut bench = block_controls_fixture(Bar::Bottom, false, false);
    bench.state.open = false;
    let bottom = bench.settings.bar(Bar::Bottom, &layout).to_vec();
    let one = shortcuts::block_of(&bench.settings, &bottom, "inventory").expect("a block");
    let two = shortcuts::block_of(&bench.settings, &bottom, "preferences").expect("another block");
    assert_ne!(one, two);
    let k = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyJ".into()),
        ctrl: true,
        shift: true,
        alt: false,
    };
    let open = |bench: &mut Bench, key: &str| {
        bench
            .ctx
            .data_mut(|data| ["shortcut_add", "shortcut_replace"].map(|id| data.remove::<Rect>(Id::unique(id))));
        bench.state.edit_shortcut(key, Some(&k));
        for _ in 0..3 {
            bench.frame(Vec::new());
        }
    };
    let rect = |bench: &Bench, id: &str| bench.ctx.data(|data| data.get_temp::<Rect>(Id::unique(id)));
    bench.state.pending = Some(Edit::Shortcut(one.clone(), Some(k.clone())));
    assert!(bench.state.commit(&layout, &mut bench.settings));
    // a block beside a block: Add
    open(&mut bench, &two);
    let add = rect(&bench, "shortcut_add").expect("Add").center();
    assert!(rect(&bench, "shortcut_replace").is_some());
    bench.pointer(add, Some(true));
    bench.pointer(add, Some(false));
    assert_eq!(bench.settings.shortcuts.get(&one), Some(&k));
    assert_eq!(bench.settings.shortcuts.get(&two), Some(&k));
    let mut both = shortcuts::targets(&bench.settings, &layout, &k);
    both.sort();
    let mut expected = vec![one.clone(), two.clone()];
    expected.sort();
    assert_eq!(both, expected, "one press for both");
    shortcuts::press_all(&bench.ctx, both);
    bench.frame(Vec::new());
    assert_eq!(shortcuts::fallback(&bench.ctx, &mut bench.settings), shortcuts::Fallback::Changed);
    assert!(bench.settings.hidden_blocks.contains(&one) && bench.settings.hidden_blocks.contains(&two));
    bench.settings.hidden_blocks.clear();
    // a button: Replace only, the blocks lose it
    bench.state.shortcut_editor = None;
    open(&mut bench, "people");
    assert!(rect(&bench, "shortcut_add").is_none(), "a button never shares");
    let replace = rect(&bench, "shortcut_replace").expect("Replace").center();
    bench.pointer(replace, Some(true));
    bench.pointer(replace, Some(false));
    assert_eq!(bench.settings.shortcuts.keys().collect::<Vec<_>>(), ["people"]);
    // a block on a button's key: Replace or Cancel
    bench.state.shortcut_editor = None;
    open(&mut bench, &one);
    assert!(rect(&bench, "shortcut_add").is_none());
    assert!(rect(&bench, "shortcut_replace").is_some());
    let cancel = menu_point(&bench, aurora_i18n::tr!("common-cancel"));
    bench.pointer(cancel, Some(true));
    bench.pointer(cancel, Some(false));
    assert!(bench.state.shortcut_editor.is_none(), "Cancel closes the window");
    assert_eq!(bench.settings.shortcuts.keys().collect::<Vec<_>>(), ["people"], "unchanged");
}

/// Build › Import: the quick imports in the human's order, then the
/// advanced import, the local files and the import options, each its
/// toolbar command (human request, 2026-11-26).
#[test]
fn build_import_lists_the_quick_imports_then_the_advanced_import_local_files_and_options() {
    let mut bench = Bench::default();
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let catalogue = commands::catalogue(&bench.ctx);
    let parent = "menu:common-build/import-types";
    assert!(catalogue.find(parent).is_some(), "the Import submenu");
    let children: Vec<String> = catalogue
        .entries
        .iter()
        .filter(|entry| entry.parent == parent && !entry.separator && !entry.info)
        .map(|entry| entry.button.command.clone())
        .collect();
    let expected = [
        ("import-menu-textures", "import_texture"),
        ("import-menu-animations", "import_animation"),
        ("import-menu-sounds", "import_sound"),
        ("import-menu-materials", "import_material"),
        ("import-menu-meshes", "import_mesh"),
        ("toolbar-import-advanced", "import_advanced"),
        ("toolbar-import-local", "import_local"),
        ("import-menu-options", "import_settings"),
    ];
    assert_eq!(
        children,
        expected.map(|(key, _)| format!("{parent}/{key}")).to_vec(),
        "its entries in order"
    );
    for (key, command) in expected {
        assert_eq!(catalogue_view::alias(&format!("{parent}/{key}")), Some(command), "{key}");
    }
}

/// An interface's name too long for its button beside its eye ends with
/// "…", the whole name on hover; a short one is kept whole (human request,
/// 2026-11-26).
#[test]
fn a_long_interface_name_ends_with_an_ellipsis_before_its_eye() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let long = "Interface Aurora Builder with a very long name";
    let mut rects = Vec::new();
    let output = ctx.run_ui(Default::default(), |ui| {
        for label in [long, "Mine"] {
            let response = crate::ui::widgets::action_button_trailing(
                ui,
                &p,
                "file-text-filled",
                label,
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(190.0, 30.0),
                editor::EYE_ROOM,
            );
            rects.push(response.rect);
        }
    });
    let mut texts = Vec::new();
    fn gather(shape: &egui::Shape, texts: &mut Vec<(String, Rect, bool)>) {
        match shape {
            egui::Shape::Text(shape) => texts.push((shape.galley.text().to_owned(), shape.visual_bounding_rect(), shape.galley.elided)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| gather(shape, texts)),
            _ => {}
        }
    }
    for shape in &output.shapes {
        gather(&shape.shape, &mut texts);
    }
    let (_, rect, cut) = texts.iter().find(|(text, ..)| text == long).expect("the long name");
    assert!(*cut, "cut with its ellipsis");
    assert!(rect.right() <= rects[0].right() - editor::EYE_ROOM, "{rect:?} clear of the eye");
    assert!(texts.iter().any(|(text, _, cut)| text == "Mine" && !cut), "a short name whole");
}

/// The eyes and the interfaces' shortcuts belong to no interface: choosing
/// a preset, a personal interface or the defaults keeps them (human
/// request, 2026-11-26).
#[test]
fn choosing_an_interface_keeps_the_eyes_and_the_interfaces_shortcuts() {
    let layout = Layout::default();
    let binding = crate::keybinds::Binding {
        input: crate::keybinds::Input::Key("KeyO".into()),
        ctrl: true,
        shift: true,
        alt: false,
    };
    let key = format!("{}{}", drawers::INTERFACE_PREFIX, presets::Preset::Build.key());
    let mark = |toolbar: &mut ToolbarSettings| {
        toolbar.hidden_interfaces.insert(presets::Preset::Legacy.key().into());
        toolbar.shortcuts.insert(key.clone(), binding.clone());
        toolbar.shortcuts.insert("inventory".into(), binding.clone());
    };
    for preset in presets::Preset::ALL {
        let mut settings = crate::settings::Settings::default();
        let mut toolbar = presets::Preset::Light.layout(&layout);
        mark(&mut toolbar);
        settings.toolbar = Some(toolbar);
        assert!(preset.apply(&mut settings, &layout));
        let toolbar = settings.toolbar.as_ref().expect("its bars");
        assert!(toolbar.hidden_interfaces.contains(presets::Preset::Legacy.key()), "{preset:?}");
        assert_eq!(toolbar.shortcuts.get(&key), Some(&binding), "{preset:?}");
        assert!(!toolbar.shortcuts.contains_key("inventory"), "a button's goes with its interface");
    }
    // a personal interface
    let mut settings = crate::settings::Settings::default();
    let mut toolbar = presets::Preset::Light.layout(&layout);
    mark(&mut toolbar);
    settings.toolbar = Some(toolbar);
    let profile = profile::Profile {
        base: profile::Base::Aurora,
        toolbar: presets::Preset::Photographer.layout(&layout),
    };
    profile::apply(profile, &mut settings);
    let toolbar = settings.toolbar.as_ref().expect("its bars");
    assert!(toolbar.hidden_interfaces.contains(presets::Preset::Legacy.key()));
    assert_eq!(toolbar.shortcuts.get(&key), Some(&binding));
    // the defaults
    let mut toolbar = presets::Preset::Light.layout(&layout);
    mark(&mut toolbar);
    let mut state = ToolbarUi {
        pending: Some(Edit::Restore),
        ..Default::default()
    };
    assert!(state.commit(&layout, &mut toolbar));
    assert!(toolbar.hidden_interfaces.contains(presets::Preset::Legacy.key()));
    assert_eq!(toolbar.shortcuts.get(&key), Some(&binding));
}

/// Each preset's button in the editor has an eye, open by default: a click
/// on it hides the preset from the Interfaces block, or shows it again,
/// without applying it (human request, 2026-11-26).
#[test]
fn a_presets_eye_in_the_editor_shows_or_hides_it_in_the_interfaces_block() {
    let mut bench = Bench {
        screen: Vec2::new(1600.0, 1000.0),
        ..Default::default()
    };
    bench.state.open = true;
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let key = presets::Preset::Photographer.key();
    let eye = || {
        bench
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique(("toolbar_interface_eye_rect", key))))
            .expect("the eye")
    };
    let rect = eye();
    assert!(bench.settings.hidden_interfaces.is_empty(), "open by default");
    for hidden in [true, false] {
        bench.pointer(rect.center(), Some(true));
        bench.pointer(rect.center(), Some(false));
        for _ in 0..2 {
            bench.frame(Vec::new());
        }
        assert_eq!(bench.settings.hidden_interfaces.contains(key), hidden);
        assert_eq!(bench.state.preset_request, None, "the eye does not apply it");
    }
}

/// A drawer's window shows its shortcut with Edit and Delete before "Hide
/// the button while floating", greyed without a shortcut, a click on it
/// saying why; the shortcut window draws Ctrl, Alt and Shift as keys (human
/// request, 2026-11-26).
#[test]
fn a_blocks_window_edits_its_shortcut_and_greys_hiding_without_one() {
    let (mut bench, id) = drawer_target(false, Bar::Left, false, false);
    bench.screen = Vec2::new(1920.0, 1080.0);
    bench.state.drawer_editor = Some(id.clone());
    for _ in 0..4 {
        bench.frame(Vec::new());
    }
    let rect = |bench: &Bench, key: &str| {
        bench
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique((key, &id))))
            .unwrap_or_else(|| panic!("{key}"))
    };
    let closes = rect(&bench, "drawer_closes_check");
    let edit = rect(&bench, "drawer_shortcut_edit");
    let hides = rect(&bench, "drawer_hides_check");
    assert!(
        closes.bottom() <= edit.top() && edit.bottom() <= hides.top(),
        "closes, shortcut, then hiding"
    );
    assert!(bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-shortcut-none")));
    // greyed: a click changes nothing and says why
    let hides = scroll_window_to(&mut bench, aurora_i18n::tr!("toolbar-drawer-hides-button"));
    bench.pointer(hides, Some(true));
    bench.pointer(hides, Some(false));
    for _ in 0..2 {
        bench.frame(Vec::new());
    }
    assert!(!bench.settings.drawers[&id].hides_button);
    assert!(
        bench
            .text
            .iter()
            .any(|(text, _)| text == aurora_i18n::tr!("toolbar-drawer-hides-needs-shortcut"))
    );
    // Edit opens the shortcut window, its modifiers drawn as keys
    let edit = scroll_window_to(&mut bench, aurora_i18n::tr!("common-edit"));
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let editor = bench.state.shortcut_editor.clone().expect("the shortcut window");
    assert_eq!(editor.key, id);
    assert!(!editor.capturing, "it waits only once asked");
    assert!(!bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-shortcut-esc")));
    let choose = menu_point(&bench, aurora_i18n::tr!("toolbar-shortcut-choose-key"));
    bench.pointer(choose, Some(true));
    bench.pointer(choose, Some(false));
    for _ in 0..2 {
        bench.frame(Vec::new());
    }
    assert!(bench.state.shortcut_capturing(), "Choose a key waits for it");
    assert!(bench.text.iter().any(|(text, _)| text == aurora_i18n::tr!("toolbar-shortcut-esc")));
    let alt = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(Id::unique(("shortcut_modifier", "AltLeft"))))
        .expect("Alt key");
    bench.pointer(alt.center(), Some(true));
    bench.pointer(alt.center(), Some(false));
    assert!(bench.state.shortcut_editor.as_ref().is_some_and(|editor| editor.alt));
    bench.state.shortcut_editor = None;
    bench.state.pending = Some(Edit::Shortcut(
        id.clone(),
        Some(crate::keybinds::Binding {
            input: crate::keybinds::Input::Key("KeyJ".into()),
            ctrl: true,
            shift: false,
            alt: false,
        }),
    ));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    assert!(bench.settings.shortcuts.contains_key(&id));
    scroll_window_to(&mut bench, aurora_i18n::tr!("toolbar-drawer-hides-button"));
    let hides = rect(&bench, "drawer_hides_check");
    bench.pointer(hides.center(), Some(true));
    bench.pointer(hides.center(), Some(false));
    for _ in 0..2 {
        bench.frame(Vec::new());
    }
    assert!(bench.settings.drawers[&id].hides_button, "with a shortcut, it is set");
    let delete = scroll_window_to(&mut bench, aurora_i18n::tr!("common-delete"));
    bench.pointer(delete, Some(true));
    bench.pointer(delete, Some(false));
    for _ in 0..2 {
        bench.frame(Vec::new());
    }
    assert!(!bench.settings.shortcuts.contains_key(&id));
}
