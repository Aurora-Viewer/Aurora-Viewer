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
            aurora_i18n::tr!("build-ui-object-position-meters"),
        ),
        (
            aurora_i18n::tr!("build-compact-features"),
            2,
            aurora_i18n::tr!("build-ui-features-edit-object-attributes"),
        ),
        (
            aurora_i18n::tr!("build-legacy-texture"),
            3,
            aurora_i18n::tr!("build-legacy-textures"),
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
        .find(|(text, _)| text == aurora_i18n::tr!("common-align"))
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
    for is_expanded in [false, true] {
        let rect = inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("legacy-build-collapse")))
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

#[test]
fn legacy_object_keeps_vertical_vectors_left_of_the_shape_and_native_copy_paste() {
    let mut inspector = legacy();
    inspector.tool.tab = 5;
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let shape = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-compact-path-cut"))
        .unwrap()
        .1;
    for which in 0..3usize {
        let rows: Vec<_> = (0..3usize)
            .map(|axis| {
                inspector
                    .ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("legacy-vector-row", which, axis))))
                    .unwrap()
            })
            .collect();
        assert!((rows[0].left() - rows[1].left()).abs() < 1.0);
        assert!((rows[1].left() - rows[2].left()).abs() < 1.0);
        assert!(rows[0].bottom() <= rows[1].top() + 1.0 && rows[1].bottom() <= rows[2].top() + 1.0);
        assert!(
            rows.iter().all(|row| row.right() < shape.left()),
            "XYZ must stay in the reference left column"
        );
    }
    let key = inspector.tool.selection[0];
    let index = inspector.world.objects.index_of(&key).unwrap();
    let original = inspector.world.objects.get(index).unwrap().position;
    let copy = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-legacy-vector-copy"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, copy);
    assert!(inspector.tool.ui.vec_clipboard[0].is_some());
    inspector.world.objects.get_mut(index).unwrap().position += glam::Vec3::X;
    inspector.tool.out.clear();
    let (_, texts) = inspector.show(vec![]);
    let paste = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-legacy-vector-paste"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, paste);
    assert_eq!(inspector.world.objects.get(index).unwrap().position, original);
    assert!(inspector.tool.out.iter().any(|command| matches!(
        command,
        aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::Transform { .. })
    )));
}

#[test]
fn legacy_general_places_sale_before_permissions_and_features_exposes_both_columns() {
    let mut inspector = legacy();
    let (_, texts) = inspector.show(vec![]);
    let sale = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-for-sale"))
        .unwrap()
        .1;
    let permissions = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-general-anyone"))
        .unwrap()
        .1;
    assert!(sale.bottom() < permissions.top());
    assert!(texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-legacy-copy-keys")));
    inspector.tool.tab = 2;
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let flexible = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-features-flexibility"))
        .unwrap()
        .1;
    let physics = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-features-physics-shape-type"))
        .unwrap()
        .1;
    assert!(flexible.right() < physics.left());
    for label in [
        aurora_i18n::tr!("build-ui-features-force-z"),
        aurora_i18n::tr!("common-restitution"),
        aurora_i18n::tr!("common-light"),
        aurora_i18n::tr!("common-reflection-probe"),
    ] {
        assert!(
            texts.iter().any(|(text, _)| text == label),
            "The reference fields must be exposed without opening cards: {label}"
        );
    }
}

#[test]
fn legacy_texture_uses_a_single_channel_preview_and_vertical_full_width_uv_rows() {
    let mut inspector = legacy();
    inspector.tool.tab = 3;
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let rows: Vec<_> = [
        aurora_i18n::tr!("build-legacy-scale-horizontal"),
        aurora_i18n::tr!("build-legacy-scale-vertical"),
        aurora_i18n::tr!("build-ui-object-rotation-degrees"),
        aurora_i18n::tr!("build-legacy-offset-horizontal"),
        aurora_i18n::tr!("build-legacy-offset-vertical"),
    ]
    .map(|label| texts.iter().find(|(text, _)| text == label).unwrap().1)
    .into();
    assert!(rows.windows(2).all(|pair| pair[0].bottom() <= pair[1].top() + 1.0));
    assert!(rows.iter().all(|row| (row.left() - rows[0].left()).abs() < 1.0));
    let preview = inspector
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", "Diffuse".to_string()))))
        .unwrap();
    assert!(
        inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", "Normal".to_string()))))
            .is_none()
    );
    let radio = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-bumpiness"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, radio);
    assert_eq!(inspector.tool.ui.bp_channel, 1);
    let normal = inspector
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", "Normal".to_string()))))
        .unwrap();
    assert_eq!(normal, preview, "Changing the channel must reuse the single reference texture slot");
    click(&mut inspector, normal.center());
    assert_eq!(inspector.tool.ui.pick_target, Some(super::super::super::PickTarget::Normal));
}

#[test]
fn legacy_pbr_edit_opens_live_factors_without_inline_cards() {
    let _inline = crate::scene::jobs::execute_inline_on_this_thread();
    let mut inspector = legacy();
    inspector.tool.tab = 3;
    inspector.tool.ui.tex_tab = 1;
    let index = inspector.tool.sel_prims(&inspector.world)[0];
    let asset = uuid::Uuid::from_u128(0x7993);
    inspector.world.objects.get_mut(index).unwrap().extra.render_materials = vec![(u8::MAX, asset)];
    inspector.scene.materials.insert(asset, Default::default());
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    assert!(!texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-metallic")));
    let edit = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-edit"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, edit);
    assert!(inspector.tool.ui.legacy_pbr_editor);
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    assert!(texts.iter().any(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-metallic")));
    let double_sided = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-double-sided"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, double_sided);
    assert_eq!(BuildTool::gltf_override(&inspector.world, index, 0).double_sided, Some(true));
}
