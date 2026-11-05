use super::*;
use crate::ui::{Panels, bars, chat::ChatUi, icons::Icons};
use egui::{Pos2, Rect, Vec2};

struct Bench {
    ctx: egui::Context,
    state: ToolbarUi,
    settings: ToolbarSettings,
    panels: Panels,
    menu_open: bool,
    time: f64,
    text: Vec<(String, Rect)>,
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
            time: 0.0,
            text: Vec::new(),
        }
    }
}

impl Bench {
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let ctx = self.ctx.clone();
        let p = crate::theme::Theme::default().palette();
        let icons = Icons::default();
        let layout = Layout::default();
        let status = bars::islands::tests::status(true, false);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ui| {
                bars::collect_menu(ui, &p, &mut self.panels, &status);
                self.state.prepare(&ctx, &self.settings);
                bars::top::reserve(
                    ui,
                    &self.settings,
                    Rect::from_min_size(egui::pos2(100.0, 8.0), Vec2::new(800.0, 36.0)),
                );
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
                    editing: false,
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
                    false,
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
                    true,
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
                    Rect::from_min_max(egui::pos2(0.0, 52.0), egui::pos2(1000.0, 644.0)),
                    true,
                    [true, true],
                );
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
    assert!(
        bench
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::unique("toolbar_button_appearance")))
            .is_some()
    );
    let text_mode = bench
        .text
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("toolbar-mode-text"))
        .expect("text-only radio")
        .1
        .center();
    bench.pointer(text_mode, Some(true));
    bench.pointer(text_mode, Some(false));
    assert_eq!(bench.settings.appearance[IMPORT].mode, Some(ButtonMode::TextOnly));
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
