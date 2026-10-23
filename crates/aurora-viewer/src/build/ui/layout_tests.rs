use super::*;
use crate::build::layout::{Item, Panel, Profile};

fn settle(inspector: &mut Inspector) {
    for _ in 0..4 {
        inspector.show(vec![]);
    }
}
fn row(inspector: &Inspector, item: Item, panel: Panel) -> egui::Rect {
    inspector
        .tool
        .ui
        .layout
        .targets
        .iter()
        .find(|(key, location, _)| *key == item && *location == panel)
        .unwrap()
        .2
}
fn button(inspector: &mut Inspector, point: egui::Pos2, pressed: bool) {
    inspector.show(vec![
        egui::Event::PointerMoved(point),
        egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]);
}
fn click(inspector: &mut Inspector, point: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(point)]);
    button(inspector, point, true);
    button(inspector, point, false);
}
fn drag(inspector: &mut Inspector, from: egui::Pos2, to: egui::Pos2) {
    inspector.show(vec![egui::Event::PointerMoved(from)]);
    button(inspector, from, true);
    inspector.show(vec![egui::Event::PointerMoved(from + vec2(10.0, 0.0))]);
    inspector.show(vec![egui::Event::PointerMoved(to)]);
    button(inspector, to, false);
    settle(inspector);
}

#[test]
fn organization_blocks_option_clicks_without_changing_row_geometry() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    settle(&mut i);
    for item in [Item::Face, Item::Linked] {
        let before = row(&i, item, Panel::Footer);
        click(&mut i, before.left_center() + vec2(8.0, 0.0));
        let checked = row(&i, item, Panel::Footer);
        assert_eq!(before, checked);
        let settings = i.settings.clone();
        let mode = i.tool.edit_mode;
        i.tool.ui.layout.organizing = true;
        settle(&mut i);
        assert_eq!(row(&i, item, Panel::Footer), checked);
        click(&mut i, checked.left_center() + vec2(8.0, 0.0));
        assert_eq!(i.settings, settings);
        assert_eq!(i.tool.edit_mode, mode);
        i.tool.ui.layout.organizing = false;
    }
}

#[test]
fn pointer_organization_moves_options_toolbars_and_navigation_to_hidden_and_back() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let before = i.settings.clone();
    let from = row(&i, Item::RootAxis, Panel::Footer).center();
    let to = row(&i, Item::Snap, Panel::Settings).left_top() + vec2(80.0, 2.0);
    drag(&mut i, from, to);
    assert_eq!(i.settings.layout.locations[&Item::RootAxis], Panel::Settings);
    let settings = i.settings.layout.in_panel(Panel::Settings);
    assert_eq!(settings[0], Item::RootAxis);
    for item in [Item::RootAxis, Item::Manipulators, Item::Actions, Item::Shape] {
        let panel = i.settings.layout.locations[&item];
        let from = row(&i, item, panel).center();
        let to = i
            .tool
            .ui
            .layout
            .zones
            .iter()
            .find(|(panel, _)| *panel == Panel::Hidden)
            .unwrap()
            .1
            .center();
        drag(&mut i, from, to);
        assert_eq!(i.settings.layout.locations[&item], Panel::Hidden, "{item:?}");
        let from = row(&i, item, Panel::Hidden).center();
        let dest = if item.navigation() { Panel::Navigation } else { Panel::Footer };
        let to = i.tool.ui.layout.zones.iter().find(|(panel, _)| *panel == dest).unwrap().1.center();
        drag(&mut i, from, to);
        assert_eq!(i.settings.layout.locations[&item], dest, "{item:?}");
    }
    let mut after = i.settings.clone();
    after.layout = before.layout.clone();
    assert_eq!(before, after, "Moving controls must not change option values");
    let json = serde_json::to_string(&i.settings).unwrap();
    let restored: BuildSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(i.settings.layout, restored.layout);
}

#[test]
fn settings_follow_the_free_side_and_bottom_of_the_inspector() {
    let mut i = Inspector::new();
    i.tool.ui.layout.open = true;
    // Floating bars use a smaller work area for initial placement while the
    // editor can move across the whole screen.
    crate::ui::layout::set_work_area(&i.ctx, egui::Rect::from_min_max(egui::pos2(8.0, 60.0), egui::pos2(1272.0, 800.0)));
    crate::ui::layout::set_move_area(&i.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, i.screen));
    for x in [12.0, 880.0] {
        crate::ui::widgets::place_floater(&i.ctx, super::super::super::INSPECTOR_ID, egui::pos2(x, 40.0), vec2(350.0, 820.0));
        settle(&mut i);
        let main = i.show(vec![]).0;
        let settings = i.ctx.memory(|m| m.area_rect(egui::Id::unique("construction-settings"))).unwrap();
        assert!((settings.bottom() - main.bottom()).abs() < 1.5, "{main:?} {settings:?}");
        if x < 500.0 {
            assert!(settings.left() >= main.right());
        } else {
            assert!(settings.right() <= main.left());
        }
    }
    i.tool.ui.layout.organizing = true;
    settle(&mut i);
    let (_, texts) = i.show(vec![]);
    let settings = i.ctx.memory(|m| m.area_rect(egui::Id::unique("construction-settings"))).unwrap();
    for key in ["build-layout-export-notecard", "build-layout-finish", "build-layout-ruler"] {
        let rect = texts.iter().find(|(text, _)| text == aurora_i18n::text(key)).unwrap().1;
        assert!(
            settings.contains_rect(rect),
            "Organization controls must use the available height: {key}"
        );
    }
}

#[test]
fn profile_roundtrip_restores_settings_without_changing_object_transforms() {
    let (mut world, mut tool, mut settings) = fixture();
    settings.layout.place(Item::Shape, Panel::Hidden, None, false);
    settings.layout.place(Item::Actions, Panel::Settings, Some(Item::Snap), false);
    settings.grid_resolution = 1.25;
    settings.scale_uniform = true;
    let text = serde_json::to_string(&Profile::capture(&tool, &settings)).unwrap();
    let expected = settings.clone();
    let idx = tool.sel_prims(&world)[0];
    let object = world.objects.get(idx).unwrap();
    let transform = (object.position, object.scale, object.rotation);
    settings.layout = Default::default();
    settings.grid_resolution = 0.5;
    settings.scale_uniform = false;
    Profile::read(&text).unwrap().apply(&mut tool, &mut world, &mut settings);
    assert_eq!(settings, expected);
    let object = world.objects.get(idx).unwrap();
    assert_eq!(transform, (object.position, object.scale, object.rotation));
    assert!(!text.contains("position"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    for (field, bad) in [("version", serde_json::json!(20)), ("format", serde_json::json!("other"))] {
        let mut changed = value.clone();
        changed[field] = bad;
        assert!(Profile::read(&changed.to_string()).is_err());
    }
    let mut changed = value.clone();
    changed["layout"]["order"] = serde_json::json!(["face", "face"]);
    assert!(Profile::read(&changed.to_string()).is_err());
    let mut changed = value.clone();
    changed["layout"]["locations"]["nav-section-general"] = serde_json::json!("footer");
    assert!(Profile::read(&changed.to_string()).is_err());
    let mut changed = value;
    changed["ruler"]["gridUnit"] = serde_json::json!(-1.0);
    assert!(Profile::read(&changed.to_string()).is_err());
    let before = settings.clone();
    tool.ui.notecards.pending_load = Some(uuid::Uuid::from_u128(4));
    assert!(super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        uuid::Uuid::from_u128(4),
        &Ok(changed.to_string().into_bytes())
    ));
    assert_eq!(settings, before);
}

#[test]
fn folded_references_stay_horizontal_and_do_not_open_the_values() {
    let mut i = Inspector::new();
    for expanded in [true, false] {
        i.settings.transform_expanded = expanded;
        settle(&mut i);
        let references: Vec<_> = (0u8..3)
            .map(|mode| {
                i.ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build_grid_mode", mode))))
                    .unwrap()
            })
            .collect();
        if expanded {
            assert!((references[0].center().x - references[2].center().x).abs() < 1.0);
            assert!(references[0].bottom() < references[1].top());
        } else {
            assert!((references[0].center().y - references[2].center().y).abs() < 1.0);
            assert!(references[0].right() < references[1].left());
        }
        click(&mut i, references[1].center());
        assert_eq!(i.settings.grid_mode, 1);
        assert_eq!(i.settings.transform_expanded, expanded);
    }
}

#[test]
fn notecard_callbacks_require_their_own_request_and_keep_settings_on_failure() {
    let (mut world, mut tool, mut settings) = fixture();
    let request = uuid::Uuid::from_u128(40);
    let unrelated = uuid::Uuid::from_u128(41);
    tool.ui.notecards.pending_save = Some(request);
    let result = Err("Synthetic upload failure".into());
    assert!(!super::super::super::layout_notecard_saved(&mut tool, unrelated, &result));
    assert_eq!(tool.ui.notecards.pending_save, Some(request));
    assert!(super::super::super::layout_notecard_saved(&mut tool, request, &result));
    assert!(tool.ui.notecards.pending_save.is_none());
    let before = settings.clone();
    tool.ui.notecards.pending_load = Some(request);
    assert!(!super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        unrelated,
        &Ok(Vec::new())
    ));
    assert!(super::super::super::layout_notecard_loaded(
        &mut tool,
        &mut world,
        &mut settings,
        request,
        &Err("Synthetic download failure".into())
    ));
    assert_eq!(settings, before);
}
