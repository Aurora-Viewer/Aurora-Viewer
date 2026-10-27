use super::*;

fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        })
        .next_back()
        .expect("menu item visible")
}

fn has_text(output: &egui::FullOutput, label: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == label))
}

fn click_events(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    ]
}

#[test]
fn import_menu_is_in_build_and_requests_the_chooser_toggle_without_a_type_submenu() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let mut panels = Panels::from_settings(&crate::settings::Settings::default());
    let mut time = 0.0;
    let mut frame = |events| {
        time += 0.2;
        let mut action = BarAction::None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button(aurora_i18n::tr!("common-build"), |ui| build_menu(ui, &p, &mut action, "", false));
                    ui.menu_button(aurora_i18n::tr!("common-contents"), |ui| {
                        contents_menu(ui, &p, &mut panels, &mut action, true, false)
                    });
                });
            },
        );
        output.textures_delta.clear();
        (output, action)
    };
    let (output, _) = frame(Vec::new());
    let contents = text_position(&output, aurora_i18n::tr!("common-contents"));
    let build = text_position(&output, aurora_i18n::tr!("common-build"));
    for pressed in [true, false] {
        frame(click_events(contents, pressed));
    }
    for _ in 0..4 {
        frame(Vec::new());
    }
    let (output, action) = frame(Vec::new());
    assert!(!has_text(&output, aurora_i18n::tr!("import-menu")));
    assert!(matches!(action, BarAction::None));
    for pressed in [true, false] {
        frame(vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
    }
    for _ in 0..2 {
        for pressed in [true, false] {
            frame(click_events(build, pressed));
        }
        for _ in 0..4 {
            frame(Vec::new());
        }
        let (output, _) = frame(Vec::new());
        let import = text_position(&output, aurora_i18n::tr!("import-menu"));
        for _ in 0..3 {
            let (output, action) = frame(vec![egui::Event::PointerMoved(import)]);
            assert!(matches!(action, BarAction::None));
            assert!(!has_text(&output, crate::imports::Choice::Texture.label()));
        }
        frame(click_events(import, true));
        let (_, action) = frame(click_events(import, false));
        assert!(matches!(action, BarAction::Import));
        assert!(!has_text(&frame(Vec::new()).0, aurora_i18n::tr!("import-menu")));
    }
}

/// The Legacy Viewer's Content entries (#1116, #1117, #1121): L$ market data
/// only on a grid with a Marketplace (GridCheck "secondlife"), the pages
/// opened through the links' usual path, and Message of the day writing the
/// login message again, greyed out when the grid sent none.
#[test]
fn contents_menu_opens_the_legacy_content_pages_and_the_message_of_the_day() {
    use crate::ui::context::{CtxAction, take_requests};
    for (marketplace, motd) in [(true, true), (false, false)] {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let mut panels = Panels::from_settings(&crate::settings::Settings::default());
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.2;
            let mut action = BarAction::None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::MenuBar::new().ui(ui, |ui| {
                        ui.menu_button(aurora_i18n::tr!("common-contents"), |ui| {
                            contents_menu(ui, &p, &mut panels, &mut action, marketplace, motd)
                        });
                    });
                },
            );
            output.textures_delta.clear();
            (output, action)
        };
        let contents = text_position(&frame(Vec::new()).0, aurora_i18n::tr!("common-contents"));
        // opens the menu, clicks `label` in it, and gives what it asked for
        let mut click = |label: &str| {
            for pressed in [true, false] {
                frame(click_events(contents, pressed));
            }
            for _ in 0..4 {
                frame(Vec::new());
            }
            let (output, _) = frame(Vec::new());
            assert_eq!(has_text(&output, aurora_i18n::tr!("ui-bars-market-data")), marketplace);
            let at = text_position(&output, label);
            frame(click_events(at, true));
            let (_, action) = frame(click_events(at, false));
            for _ in 0..2 {
                frame(vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            (action, take_requests(&ctx))
        };
        let (_, asked) = click(aurora_i18n::tr!("ui-bars-script-library"));
        assert!(matches!(&asked[..], [CtxAction::OpenUrl(url)] if url == SCRIPT_LIBRARY));
        if marketplace {
            let (_, asked) = click(aurora_i18n::tr!("ui-bars-market-data"));
            assert!(matches!(&asked[..], [CtxAction::OpenUrl(url)] if url == LINDEX_MARKET));
        }
        let (action, _) = click(aurora_i18n::tr!("ui-bars-message-of-the-day"));
        assert_eq!(matches!(action, BarAction::MessageOfTheDay), motd);
    }
}
