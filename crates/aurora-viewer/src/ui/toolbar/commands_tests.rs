use super::*;
use crate::ui::{Panels, bars, chat::ChatUi, icons::Icons};
use egui::{Pos2, Rect, Vec2};

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
    screen: Vec2,
    notif: crate::ui::notifications::NotifUi,
    conversation_count: usize,
}

impl Default for Bench {
    fn default() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let settings = ToolbarSettings {
            top: Vec::new(),
            menu_layout: [(String::new(), vec!["menu:common-build/import-menu".into()])].into(),
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
            screen: Vec2::new(1000.0, 700.0),
            notif: Default::default(),
            conversation_count: 0,
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
        let target = match bar {
            Bar::Top => egui::pos2(20.0, 26.0),
            Bar::Left => egui::pos2(16.0, self.screen.y / 2.0),
            Bar::Right => egui::pos2(self.screen.x - 16.0, self.screen.y / 2.0),
            Bar::Bottom => egui::pos2(self.screen.x / 2.0, self.screen.y - 20.0),
        };
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
        status.conversation_thumbnails = self.conversation_count;
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
                    bars::chrome::prepare(ui, &p, &status, &self.settings, self.state.open && !self.settings.locked);
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
                    editing: self.state.open && !self.settings.locked,
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
                if self.settings.composed {
                    bars::chrome::show(
                        ui,
                        &p,
                        &icons,
                        &mut self.panels,
                        &status,
                        &mut Default::default(),
                        &mut Default::default(),
                        Default::default(),
                        &mut self.notif,
                        Default::default(),
                        &mut self.state,
                        &self.settings,
                    );
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
                self.state.show(&ctx, &p, &layout, &self.settings);
                self.state.commit(&layout, &mut self.settings);
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
fn native_blocks_and_repeatable_separators_drag_to_every_edge_in_both_layouts() {
    for classic in [false, true] {
        for bar in Bar::ALL {
            for command in [
                structure::SEPARATOR,
                "bar_navigation",
                "bar_account",
                "bar_system",
                "bar_notifications",
                "bar_conversations",
            ] {
                let mut bench = Bench {
                    native_top: true,
                    classic,
                    screen: Vec2::new(1600.0, 900.0),
                    ..Default::default()
                };
                bench.settings.compose_top();
                bench.palette_drag(command, bar);
                if command == structure::SEPARATOR {
                    let count = bench
                        .settings
                        .bar(bar, &Layout::default())
                        .iter()
                        .filter(|b| structure::is_separator(&b.command))
                        .count();
                    assert!(count > usize::from(bar == Bar::Top) * 4, "separator on {bar:?}, classic={classic}");
                    bench.palette_drag(command, bar);
                    let next = bench
                        .settings
                        .bar(bar, &Layout::default())
                        .iter()
                        .filter(|b| structure::is_separator(&b.command))
                        .count();
                    assert_eq!(next, count + 1);
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
    const IMPORT: &str = "menu:common-build/import-menu";
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
    const IMPORT: &str = "menu:common-build/import-menu";
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
    const IMPORT: &str = "menu:common-build/import-menu";
    for classic in [false, true] {
        for (parent, command) in [("", IMPORT), ("menu:common-build", IMPORT), ("", "menu:common-world")] {
            let mut bench = Bench {
                native_top: true,
                classic,
                ..Default::default()
            };
            bench.settings.top = ToolbarSettings::default().top;
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
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", "menu:common-build/import-menu"))))
        .expect("menu row")
        .center();
    bench.pointer(source, Some(true));
    bench.pointer(source + Vec2::new(20.0, 0.0), None);
    assert!(!ToolbarUi::dragging(&bench.ctx));
}

#[test]
fn a_menu_entry_right_click_opens_its_appearance_editor_and_saves_its_own_mode() {
    const IMPORT: &str = "menu:common-build/import-menu";
    let mut bench = Bench::default();
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let source = bench
        .ctx
        .data(|data| data.get_temp::<Rect>(egui::Id::unique(("main_menu_entry", IMPORT))))
        .expect("menu row")
        .center();
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
    let edit = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-button-appearance"))
        .expect("appearance context action")
        .1
        .center();
    bench.pointer(edit, Some(true));
    bench.pointer(edit, Some(false));
    for _ in 0..3 {
        bench.frame(Vec::new());
    }
    let editor = bench
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique("toolbar_button_appearance")))
        .expect("appearance editor");
    assert!(editor.height() <= 520.0, "the search field must not stretch the editor: {editor:?}");
    let text_mode = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-mode-text"))
        .expect("text-only radio")
        .1
        .center();
    bench.pointer(text_mode, Some(true));
    bench.pointer(text_mode, Some(false));
    assert_eq!(
        bench.settings.appearance[&appearance::menu_key(IMPORT)].mode,
        Some(ButtonMode::TextOnly)
    );
    assert!(
        !bench.settings.appearance.contains_key(IMPORT),
        "the toolbar copy keeps its own bar default"
    );
    assert!(commands::take_request(&bench.ctx).is_none(), "customizing never activates Import");
}

#[test]
fn a_real_menu_drag_can_return_an_entry_to_another_submenu() {
    const IMPORT: &str = "menu:common-build/import-menu";
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
    assert_eq!(
        catalogue.children("menu:common-build", &bench.settings.menu_layout)[0]
            .button
            .command,
        IMPORT
    );
}
