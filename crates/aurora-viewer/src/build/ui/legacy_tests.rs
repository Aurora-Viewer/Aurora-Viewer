//! Pointer-driven regressions for the alternate native construction layout.
use super::*;

fn click(inspector: &mut Inspector, pos: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        inspector.show(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
}

fn legacy() -> Inspector {
    let mut inspector = Inspector::new();
    inspector.settings.legacy_window = true;
    inspector.tool.tab = 6;
    resize_inspector(&mut inspector, vec2(400.0, 820.0));
    inspector
}

#[test]
fn legacy_build_tabs_reuse_native_panels_and_ignore_hidden_aurora_sections() {
    use crate::build::layout::{Item, Panel};
    let mut inspector = legacy();
    inspector.settings.layout.place(Item::Shape, Panel::Hidden, None, false);
    inspector.settings.layout.place(Item::Features, Panel::Hidden, None, false);
    let layout = inspector.settings.layout.clone();
    let selection = inspector.tool.selection.clone();
    let mut size = None;
    for (label, tab, content) in [
        (
            aurora_i18n::tr!("build-legacy-object"),
            5,
            aurora_i18n::tr!("build-compact-position"),
        ),
        (
            aurora_i18n::tr!("build-compact-features"),
            2,
            aurora_i18n::tr!("build-ui-features-edit-object-attributes"),
        ),
        (
            aurora_i18n::tr!("build-legacy-texture"),
            3,
            aurora_i18n::tr!("build-ui-texture-blinn-phong"),
        ),
        (aurora_i18n::tr!("build-compact-contents"), 4, aurora_i18n::tr!("common-new-script")),
        (aurora_i18n::tr!("build-compact-general"), 6, "Presentation test"),
    ] {
        let (_, texts) = inspector.show(vec![]);
        let pos = texts
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("Missing tab: {label}"))
            .1
            .center();
        click(&mut inspector, pos);
        assert_eq!(inspector.tool.tab, tab);
        let (rect, texts) = inspector.show(vec![]);
        assert!(
            texts.iter().any(|(text, _)| text == content),
            "Missing shared panel content: {content}"
        );
        assert_eq!(inspector.tool.selection, selection);
        assert_eq!(inspector.settings.layout, layout);
        if let Some(old) = size {
            assert!((rect.size() - old).length() <= 2.0, "Switching tabs must not grow the window");
        }
        size = Some(rect.size());
    }
    inspector.tool.ui.layout.open = true;
    for _ in 0..4 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    for label in [
        aurora_i18n::tr!("rez-under-land-group"),
        aurora_i18n::tr!("build-ui-tools-follow-edit-object"),
    ] {
        assert!(
            texts.iter().any(|(text, _)| text == label),
            "The fixed settings must retain {label}"
        );
    }
    let before = inspector.settings.follow_edit_object;
    let pos = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-tools-follow-edit-object"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, pos);
    assert_eq!(inspector.settings.follow_edit_object, !before);
    inspector.settings.legacy_window = false;
    inspector.show(vec![]);
    assert_eq!(inspector.settings.layout, layout);
    assert_eq!(inspector.tool.selection, selection);
}

#[test]
fn legacy_build_radios_and_tool_buttons_follow_native_transitions() {
    let mut inspector = legacy();
    let (_, texts) = inspector.show(vec![]);
    let pos = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-select-face"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, pos);
    assert_eq!(inspector.tool.edit_mode, EditMode::Face);
    for mode in [Tool::Grab, Tool::Focus, Tool::Create, Tool::Land, Tool::Edit] {
        let rect = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("legacy-build-tool", mode as u8))))
            .unwrap();
        click(&mut inspector, rect.center());
        assert_eq!(inspector.tool.tool, mode);
    }
    let (_, texts) = inspector.show(vec![]);
    let pos = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-linked"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, pos);
    assert!(inspector.settings.edit_linked);
    let (_, texts) = inspector.show(vec![]);
    let pos = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-align"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, pos);
    assert_eq!(inspector.tool.edit_mode, EditMode::Align);
    assert!(
        !inspector.settings.edit_linked,
        "Align must use the native whole-linkset transition"
    );
}

#[test]
fn legacy_build_collapse_restores_geometry_and_scrolling_keeps_the_tabs_visible() {
    let mut inspector = legacy();
    inspector.tool.tab = 5;
    let (expanded, texts) = inspector.show(vec![]);
    let label = aurora_i18n::tr!("build-legacy-object");
    let tab = texts.iter().find(|(text, _)| text == label).unwrap().1;
    inspector.show(vec![
        egui::Event::PointerMoved(egui::pos2(expanded.center().x, expanded.bottom() - 80.0)),
        egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -800.0),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    let (_, texts) = inspector.show(vec![]);
    assert_eq!(texts.iter().find(|(text, _)| text == label).unwrap().1, tab);
    for (icon, is_expanded) in [("caret-up-filled", false), ("caret-down-filled", true)] {
        let rect = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("floater_action", super::super::super::INSPECTOR_ID, icon))))
            .unwrap();
        click(&mut inspector, rect.center());
        for _ in 0..5 {
            inspector.show(vec![]);
        }
        let (rect, _) = inspector.show(vec![]);
        assert_eq!(inspector.settings.expanded, is_expanded);
        if is_expanded {
            assert!((rect.size() - expanded.size()).length() <= 2.0);
        } else {
            assert!(rect.height() < expanded.height() * 0.65);
        }
    }
}

#[test]
fn legacy_build_setting_roundtrips_without_replacing_the_aurora_arrangement() {
    let mut settings = BuildSettings::default();
    assert!(!settings.legacy_window);
    settings.legacy_window = true;
    settings
        .layout
        .place(crate::build::layout::Item::Shape, crate::build::layout::Panel::Hidden, None, false);
    let json = serde_json::to_string(&settings).unwrap();
    let loaded: BuildSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(loaded, settings);
    let old: BuildSettings = serde_json::from_str("{}").unwrap();
    assert!(!old.legacy_window);
}
