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
fn legacy_selection_shows_the_link_or_the_faces_of_one_prim() {
    let mut inspector = legacy();
    for event in crate::demo::selection_cycle_events() {
        inspector.world.apply(event);
    }
    let prim = |inspector: &Inspector, id| inspector.world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap();
    let (root, child) = (prim(&inspector, 99610), prim(&inspector, 99611));
    let link = |n: usize| aurora_i18n::tr!("build-ui-tools-link-number", n = n);
    let faces = |list: &str| aurora_i18n::tr!("build-ui-tools-faces", list = list);
    let shown = |inspector: &mut Inspector, text: &str| inspector.show(vec![]).1.iter().any(|(t, _)| t == text);
    let line = |inspector: &Inspector| {
        inspector
            .tool
            .legacy_part_line(&inspector.world, &inspector.settings, Mods::default())
    };

    // A whole linkset: several prims, no line (enable_link_count = false).
    inspector.settings.edit_linked = false;
    inspector
        .tool
        .click_select(&mut inspector.world, &inspector.settings, Some(child), false);
    assert!(inspector.tool.sel_prims(&inspector.world).len() > 1);
    assert_eq!(line(&inspector), None);

    // One linked prim: its link number, on screen too.
    inspector.settings.edit_linked = true;
    inspector
        .tool
        .click_select(&mut inspector.world, &inspector.settings, Some(child), false);
    assert_eq!(line(&inspector), Some(link(2)));
    assert!(shown(&mut inspector, &link(2)));

    // Several linked prims: no number, though the first still has one.
    inspector
        .tool
        .click_select(&mut inspector.world, &inspector.settings, Some(root), true);
    assert_eq!(line(&inspector), None);
    assert!(!shown(&mut inspector, &link(1)) && !shown(&mut inspector, &link(2)));

    // The root is link 1.
    inspector
        .tool
        .click_select(&mut inspector.world, &inspector.settings, Some(root), false);
    assert_eq!(line(&inspector), Some(link(1)));

    // The face tool wins over the link number.
    inspector.tool.edit_mode = EditMode::Face;
    assert_eq!(line(&inspector), Some(faces("ALL_SIDES")));
    let key = inspector.tool.selection[0];
    let count = inspector.tool.num_faces(&inspector.world, root);
    assert!(count >= 3, "a demo root with several faces");
    inspector.tool.faces.insert(key, vec![2, 0]);
    assert_eq!(line(&inspector), Some(faces("0, 2")));
    assert!(shown(&mut inspector, &faces("0, 2")));
    inspector.tool.faces.insert(key, (0..count as u8).collect());
    assert_eq!(line(&inspector), Some(faces("ALL_SIDES")));

    // Outside both, nothing.
    inspector.tool.edit_mode = EditMode::Move;
    inspector.settings.edit_linked = false;
    assert_eq!(line(&inspector), None);
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
fn legacy_texture_exposes_material_tabs_vertical_maps_and_three_uv_columns() {
    let mut inspector = legacy();
    inspector.tool.tab = 3;
    for _ in 0..5 {
        inspector.show(vec![]);
    }
    let (_, texts) = inspector.show(vec![]);
    let modes = [
        "PBR",
        aurora_i18n::tr!("build-ui-texture-blinn-phong"),
        aurora_i18n::tr!("extra-media"),
    ]
    .map(|label| texts.iter().find(|(text, _)| text == label).unwrap().1);
    assert!(modes.windows(2).all(|pair| pair[0].right() < pair[1].left()));
    assert!(modes.iter().all(|rect| (rect.top() - modes[0].top()).abs() < 1.0));
    let maps = ["Diffuse", "Normal", "Specular"].map(|target| {
        inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", target.to_string()))))
            .unwrap()
    });
    assert!(maps.windows(2).all(|pair| pair[0].bottom() < pair[1].top()));
    assert!(
        maps.iter()
            .all(|rect| (rect.left() - maps[0].left()).abs() < 1.0 && (rect.width() - 60.0).abs() < 1.0)
    );
    let uv = [
        aurora_i18n::tr!("common-scale"),
        aurora_i18n::tr!("build-ui-texture-offset"),
        aurora_i18n::tr!("build-ui-texture-rotation"),
    ]
    .map(|label| texts.iter().find(|(text, _)| text == label).unwrap().1);
    assert!(uv.windows(2).all(|pair| pair[0].right() < pair[1].left()));
    assert!(uv.iter().all(|rect| (rect.top() - uv[0].top()).abs() < 1.0));
    assert!(uv[0].top() > maps[2].bottom());
    let normal_tab = texts
        .iter()
        .rev()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-normal"))
        .unwrap()
        .1;
    click(&mut inspector, normal_tab.center());
    assert_eq!(inspector.tool.ui.bp_channel, 1);
    click(&mut inspector, maps[1].center());
    assert_eq!(inspector.tool.ui.pick_target, Some(super::super::super::PickTarget::Normal));
}

#[test]
fn legacy_texture_material_choices_remain_visible_without_editable_faces_and_switch_native_modes() {
    let mut inspector = legacy();
    inspector.tool.tab = 3;
    let index = inspector.tool.sel_prims(&inspector.world)[0];
    let asset = uuid::Uuid::from_u128(0x7992);
    inspector.world.objects.get_mut(index).unwrap().extra.render_materials = (0..6).map(|face| (face, asset)).collect();
    inspector.scene.materials.insert(asset, Default::default());
    for mode in [1, 2, 0] {
        let (_, texts) = inspector.show(vec![]);
        let labels = [
            aurora_i18n::tr!("build-ui-texture-blinn-phong"),
            "PBR",
            aurora_i18n::tr!("extra-media"),
        ];
        let target = texts.iter().find(|(text, _)| text == labels[mode]).unwrap().1.center();
        click(&mut inspector, target);
        assert_eq!(inspector.tool.ui.tex_tab, mode);
        inspector.tool.update_material_preview(&mut inspector.world, &inspector.settings);
        assert_eq!(inspector.world.objects.get(index).unwrap().visual_material(0).is_none(), mode == 0);
    }
    inspector.tool.selection.clear();
    let (_, texts) = inspector.show(vec![]);
    for label in [
        aurora_i18n::tr!("build-ui-texture-blinn-phong"),
        "PBR",
        aurora_i18n::tr!("extra-media"),
    ] {
        assert!(texts.iter().any(|(text, _)| text == label));
    }
}

#[test]
fn legacy_pbr_keeps_all_maps_left_of_live_inline_factors() {
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
    let maps = ["PbrMaterial", "PbrMap(0)", "PbrMap(1)", "PbrMap(3)", "PbrMap(2)"].map(|target| {
        inspector
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("material_preview", target.to_string()))))
            .unwrap()
    });
    assert!((maps[0].top() - maps[1].top()).abs() < 1.0);
    assert!(maps[0].right() < maps[1].left());
    assert!((maps[2].top() - maps[3].top()).abs() < 1.0);
    assert!(maps[2].top() > maps[0].bottom() && maps[4].top() > maps[2].bottom());
    let metallic = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("build-ui-texture-metallic"))
        .unwrap()
        .1;
    assert!(maps.iter().all(|rect| rect.right() < metallic.left()));
    let double_sided = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-double-sided"))
        .unwrap()
        .1
        .center();
    click(&mut inspector, double_sided);
    assert_eq!(BuildTool::gltf_override(&inspector.world, index, 0).double_sided, Some(true));
    click(&mut inspector, maps[1].center());
    assert_eq!(inspector.tool.ui.pick_target, Some(super::super::super::PickTarget::PbrMap(0)));
}

fn scroll_shape(inspector: &mut Inspector, delta: f32) {
    let (window, _) = inspector.show(vec![]);
    inspector.show(vec![
        egui::Event::PointerMoved(egui::pos2(window.center().x, window.bottom() - 80.0)),
        egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    for _ in 0..20 {
        inspector.show(vec![]);
    }
}

#[test]
fn legacy_shape_exposes_all_parametric_controls_at_minimum_size() {
    let mut inspector = legacy();
    inspector.tool.tab = 5;
    resize_inspector(&mut inspector, crate::build::ui::legacy::MIN_SIZE);
    let index = inspector.tool.sel_prims(&inspector.world)[0];
    for (ty, (name, path, profile)) in super::super::object::types().into_iter().enumerate() {
        if ty == 7 {
            continue; // Sculpties have their own texture/stitching controls.
        }
        let volume = &mut inspector.world.objects.get_mut(index).unwrap().volume;
        *volume = aurora_prim::params::VolumeParams::default();
        volume.path.curve_type = path;
        volume.profile.curve_type = profile;
        let before = *volume;
        inspector.tool.out.clear();
        scroll_shape(&mut inspector, 2000.0);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..7 {
            let (window, texts) = inspector.show(vec![]);
            let tab = texts
                .iter()
                .find(|(text, _)| text == aurora_i18n::tr!("build-legacy-object"))
                .unwrap()
                .1;
            for (text, rect) in texts {
                if window.contains_rect(rect) && rect.top() > tab.bottom() && rect.bottom() < window.bottom() - 25.0 {
                    seen.insert(text);
                }
            }
            scroll_shape(&mut inspector, -100.0);
        }
        for label in [
            aurora_i18n::tr!("build-compact-path-cut"),
            aurora_i18n::tr!("build-ui-object-hollow"),
            aurora_i18n::tr!("build-ui-object-hollow-shape"),
            aurora_i18n::tr!("build-ui-object-bias"),
            aurora_i18n::tr!("build-compact-twist"),
            aurora_i18n::tr!("build-ui-object-tilt"),
            aurora_i18n::tr!("build-ui-object-radius"),
            aurora_i18n::tr!("build-ui-object-revolutions"),
        ] {
            assert!(seen.contains(label), "{name}: {label} must be reachable by scrolling");
        }
        assert_eq!(inspector.world.objects.get(index).unwrap().volume, before);
        assert!(
            !inspector
                .tool
                .out
                .iter()
                .any(|cmd| matches!(cmd, aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetShape { .. })))
        );
    }
}

#[test]
fn legacy_shape_restored_fields_commit_typed_values_for_extended_prims() {
    let mut inspector = legacy();
    inspector.tool.tab = 5;
    resize_inspector(&mut inspector, crate::build::ui::legacy::MIN_SIZE);
    let index = inspector.tool.sel_prims(&inspector.world)[0];
    let volume = &mut inspector.world.objects.get_mut(index).unwrap().volume;
    volume.path.curve_type = aurora_prim::params::LL_PCODE_PATH_CIRCLE2;
    volume.profile.curve_type = aurora_prim::params::LL_PCODE_PROFILE_SQUARE;
    for (label, value) in [
        (aurora_i18n::tr!("build-ui-object-bias"), "0.25"),
        (aurora_i18n::tr!("build-ui-object-radius"), "0.30"),
        (aurora_i18n::tr!("build-ui-object-revolutions"), "2.50"),
    ] {
        scroll_shape(&mut inspector, 2000.0);
        let mut field = None;
        for _ in 0..7 {
            let (window, texts) = inspector.show(vec![]);
            if let Some((_, caption)) = texts.iter().find(|(text, rect)| text == label && window.contains_rect(*rect)) {
                field = texts
                    .iter()
                    .find(|(text, rect)| {
                        aurora_i18n::parse_number(text).is_some()
                            && rect.top() >= caption.bottom()
                            && rect.top() < caption.bottom() + 30.0
                            && rect.left() >= caption.left()
                            && rect.left() < caption.left() + 160.0
                            && rect.bottom() < window.bottom() - 30.0
                    })
                    .map(|(_, rect)| *rect);
            }
            if field.is_some() {
                break;
            }
            scroll_shape(&mut inspector, -100.0);
        }
        let field = field.unwrap_or_else(|| panic!("Missing editable field: {label}"));
        inspector.tool.out.clear();
        click(&mut inspector, field.center());
        inspector.show(vec![egui::Event::Text(value.into())]);
        inspector.show(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }]);
        let sent = inspector
            .tool
            .out
            .iter()
            .filter_map(|cmd| match cmd {
                aurora_net::NetCommand::Build(aurora_net::build::BuildCmd::SetShape { shape, .. }) => Some(shape.to_params()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sent.len(), 1, "{label} must send exactly one committed shape");
        let actual = super::super::object::ui_from_volume(&sent[0]);
        let got = if label == aurora_i18n::tr!("build-ui-object-bias") {
            actual.skew
        } else if label == aurora_i18n::tr!("build-ui-object-radius") {
            actual.radius
        } else {
            actual.revolutions
        };
        assert!((got - value.parse::<f32>().unwrap()).abs() < 0.001, "{label}: {got}");
    }
}
