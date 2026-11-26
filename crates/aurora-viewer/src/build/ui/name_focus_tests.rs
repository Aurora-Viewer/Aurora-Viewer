//! Empty clicks on the build window focus Name or Description and select all.
use super::super::super::name_focus::{self, Which};
use super::*;
use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;

fn prepare(legacy_window: bool) -> Inspector {
    let mut inspector = Inspector::new();
    inspector.settings.legacy_window = legacy_window;
    inspector.tool.tab = 6;
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let id = inspector.world.objects.get(idx).unwrap().full_id;
    inspector.tool.props.get_mut(&id).unwrap().description = "A short note".into();
    if legacy_window {
        resize_inspector(&mut inspector, vec2(400.0, 820.0));
    }
    for _ in 0..4 {
        inspector.show(vec![]);
    }
    inspector
}

fn press(inspector: &mut Inspector, pos: egui::Pos2, button: egui::PointerButton) {
    inspector.show(vec![egui::Event::PointerMoved(pos)]);
    inspector.show(vec![egui::Event::PointerButton {
        pos,
        button,
        pressed: true,
        modifiers: Default::default(),
    }]);
    inspector.show(vec![egui::Event::PointerButton {
        pos,
        button,
        pressed: false,
        modifiers: Default::default(),
    }]);
}

fn blank_beside_name(inspector: &mut Inspector) -> egui::Pos2 {
    let (window, texts) = inspector.show(vec![]);
    let word = texts
        .iter()
        .find(|(text, _)| text == aurora_i18n::tr!("common-name-2"))
        .expect("Name label")
        .1;
    let field = inspector
        .ctx
        .read_response(name_focus::inspector_field(Which::Name))
        .expect("Name field");
    let x = (word.right() + field.rect.left()) * 0.5;
    let pos = egui::pos2(x, field.rect.center().y);
    assert!(
        window.contains(pos) && pos.x > word.right() + 2.0 && pos.x < field.rect.left() - 2.0,
        "blank beside Name must sit in the label column, not on the word or the field: word {word:?} field {:?} window {window:?}",
        field.rect
    );
    pos
}

fn selection(inspector: &Inspector, which: Which) -> Option<(usize, usize)> {
    selection_in(&inspector.ctx, which)
}

fn selection_in(ctx: &egui::Context, which: Which) -> Option<(usize, usize)> {
    let state = TextEditState::load(ctx, name_focus::inspector_field(which))?;
    let range = state.cursor.char_range()?;
    let [start, end] = range.sorted_cursors();
    Some((start.index.0, end.index.0))
}

fn focused(inspector: &Inspector) -> Option<egui::Id> {
    inspector.ctx.memory(|m| m.focused())
}

/// The other section: the painted Object tab, or the icon rail's Shape button.
fn other_section(inspector: &mut Inspector, legacy_window: bool) -> egui::Pos2 {
    if legacy_window {
        let (_, texts) = inspector.show(vec![]);
        let tab = aurora_i18n::tr!("build-legacy-object");
        return texts.iter().find(|(text, _)| text == tab).expect("object tab").1.center();
    }
    inspector.show(vec![]);
    inspector
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("build-rail", aurora_i18n::tr!("build-compact-shape")))))
        .expect("shape section")
        .center()
}

#[test]
fn empty_click_selects_name_then_description_and_leaves_controls_and_carets() {
    for legacy_window in [false, true] {
        let mut inspector = prepare(legacy_window);
        let name = "Presentation test";
        let desc = "A short note";
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Primary);
        assert_eq!(
            focused(&inspector),
            Some(name_focus::inspector_field(Which::Name)),
            "legacy {legacy_window}"
        );
        assert_eq!(
            selection(&inspector, Which::Name),
            Some((0, name.chars().count())),
            "legacy {legacy_window}"
        );

        inspector.show(vec![egui::Event::Text("Box".into())]);
        assert_eq!(
            inspector.tool.ui.edits.get("name").map(String::as_str),
            Some("Box"),
            "legacy {legacy_window}"
        );

        let mut inspector = prepare(legacy_window);
        let desc_rect = inspector
            .ctx
            .read_response(name_focus::inspector_field(Which::Desc))
            .expect("Description field")
            .rect;
        press(&mut inspector, desc_rect.center(), egui::PointerButton::Primary);
        assert_eq!(focused(&inspector), Some(name_focus::inspector_field(Which::Desc)));
        let (window, _) = inspector.show(vec![]);
        press(
            &mut inspector,
            egui::pos2(window.right() + 30.0, window.center().y),
            egui::PointerButton::Primary,
        );
        assert!(
            !crate::ui::widgets::floater_holds_keyboard(&inspector.ctx, super::super::super::INSPECTOR_ID),
            "a click outside the window drops its keyboard"
        );
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Primary);
        assert_eq!(
            focused(&inspector),
            Some(name_focus::inspector_field(Which::Desc)),
            "legacy {legacy_window}"
        );
        assert_eq!(
            selection(&inspector, Which::Desc),
            Some((0, desc.chars().count())),
            "legacy {legacy_window}"
        );

        let mut inspector = prepare(legacy_window);
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Secondary);
        assert_eq!(
            focused(&inspector),
            Some(name_focus::inspector_field(Which::Name)),
            "right click, legacy {legacy_window}"
        );
        assert_eq!(selection(&inspector, Which::Name), Some((0, name.chars().count())));

        let mut inspector = prepare(legacy_window);
        let section = other_section(&mut inspector, legacy_window);
        press(&mut inspector, section, egui::PointerButton::Primary);
        assert_ne!(focused(&inspector), Some(name_focus::inspector_field(Which::Name)));
        assert_ne!(selection(&inspector, Which::Name), Some((0, name.chars().count())));

        let mut inspector = prepare(legacy_window);
        let id = name_focus::inspector_field(Which::Name);
        inspector.show(vec![egui::Event::PointerMoved(egui::Pos2::ZERO)]);
        inspector.ctx.memory_mut(|m| m.request_focus(id));
        let mut state = TextEditState::load(&inspector.ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(4))));
        state.store(&inspector.ctx, id);
        inspector.show(vec![]);
        assert_eq!(selection(&inspector, Which::Name), Some((4, 4)));
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Primary);
        assert_eq!(
            selection(&inspector, Which::Name),
            Some((4, 4)),
            "already focused, legacy {legacy_window}"
        );

        let mut inspector = prepare(legacy_window);
        let field = inspector.ctx.read_response(name_focus::inspector_field(Which::Name)).unwrap().rect;
        press(&mut inspector, field.center(), egui::PointerButton::Primary);
        let (start, end) = selection(&inspector, Which::Name).expect("caret");
        assert_eq!(start, end, "a direct click places a caret, legacy {legacy_window}");
        assert_ne!((start, end), (0, name.chars().count()));
    }
}

/// Without the modify right the Legacy Viewer disables Name and Description
/// (`LLPanelPermissions::refresh`), and `LLFloater::setFocus` does not give
/// a disabled control the focus.
#[test]
fn empty_click_leaves_a_read_only_name_alone() {
    for legacy_window in [false, true] {
        let mut inspector = prepare(legacy_window);
        let idx = inspector.tool.sel_prims(&inspector.world)[0];
        inspector.world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
        for _ in 0..3 {
            inspector.show(vec![]);
        }
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Primary);
        assert_ne!(
            focused(&inspector),
            Some(name_focus::inspector_field(Which::Name)),
            "legacy {legacy_window}"
        );
        assert_ne!(
            selection(&inspector, Which::Name),
            Some((0, "Presentation test".chars().count())),
            "legacy {legacy_window}"
        );
        inspector.show(vec![egui::Event::Text("Q".into())]);
        let id = inspector.world.objects.get(idx).unwrap().full_id;
        assert_eq!(inspector.tool.props.get(&id).unwrap().name, "Presentation test");
        assert!(!inspector.tool.ui.edits.contains_key("name"));
    }
}

/// Object > Profile draws General in a window of its own: its rows have
/// their own ids, typing there edits nothing in the build window, and the
/// build window does not remember them as its last field.
#[test]
fn object_profile_rows_are_not_the_build_windows_rows() {
    let mut inspector = prepare(false);
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    let object = inspector.world.objects.get(idx).unwrap();
    let (id, key) = (object.full_id, object.key);
    let mut profile = crate::ui::context::ObjectProfileUi::default();
    profile.open(id, key, inspector.tool.props.get(&id).cloned());
    inspector.profile = Some(profile);
    for _ in 0..4 {
        inspector.show(vec![]);
    }
    let profile_layer = egui::Id::unique("object_profile");
    for which in [Which::Name, Which::Desc] {
        let ours = name_focus::inspector_field(which);
        let theirs = name_focus::field_id_on(profile_layer, egui::ViewportId::ROOT, which);
        assert_ne!(ours, theirs);
        let ours = inspector.ctx.read_response(ours).expect("build window field");
        let theirs = inspector.ctx.read_response(theirs).expect("profile field");
        assert_eq!(ours.layer_id.id, egui::Id::unique(super::super::super::INSPECTOR_ID));
        assert_eq!(theirs.layer_id.id, profile_layer);
    }

    let theirs = name_focus::field_id_on(profile_layer, egui::ViewportId::ROOT, Which::Desc);
    let rect = inspector.ctx.read_response(theirs).unwrap().rect;
    press(&mut inspector, rect.center(), egui::PointerButton::Primary);
    assert_eq!(focused(&inspector), Some(theirs));
    inspector.show(vec![egui::Event::Text("Q".into())]);
    assert!(
        !inspector.tool.ui.edits.contains_key("desc"),
        "typing in the profile edits the build window"
    );
    assert!(!inspector.ctx.memory(|m| m.has_focus(name_focus::inspector_field(Which::Desc))));

    let blank = blank_beside_name(&mut inspector);
    press(&mut inspector, blank, egui::PointerButton::Primary);
    assert_eq!(
        focused(&inspector),
        Some(name_focus::inspector_field(Which::Name)),
        "the profile's Description is not the build window's last field"
    );
    assert_eq!(selection(&inspector, Which::Name), Some((0, "Presentation test".chars().count())));
}

/// A window moved outside the viewer is drawn on its viewport's background
/// layer, the same layer id in every viewport: the Build window and an
/// Object > Profile both outside must still not share a field's edit state.
#[test]
fn windows_outside_the_viewer_keep_their_own_field_ids() {
    let ctx = egui::Context::default();
    let mut seen = Vec::new();
    for window in ["aurora build outside", "aurora profile outside"] {
        let viewport = crate::outside::viewport(egui::Id::unique(window));
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0))),
            ..Default::default()
        };
        input.viewports.insert(viewport, Default::default());
        let _ = ctx.run_ui(input, |ui| {
            assert_eq!(ui.ctx().viewport_id(), viewport);
            assert_eq!(ui.layer_id(), egui::LayerId::background());
            seen.push([name_focus::field_id(ui, Which::Name), name_focus::field_id(ui, Which::Desc)]);
        });
    }
    let [build, profile] = [seen[0], seen[1]];
    for i in 0..2 {
        assert_ne!(build[i], profile[i], "two windows outside share a field id");
    }
    for ids in [build, profile] {
        for (id, which) in ids.into_iter().zip([Which::Name, Which::Desc]) {
            assert_ne!(id, name_focus::inspector_field(which));
        }
    }
}

/// One pass of a build-window layer holding Name in a short scroll area.
fn lone_name(ctx: &egui::Context, offset: f32, editable: bool, click: bool) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0))),
        ..Default::default()
    };
    let _ = ctx.run_ui(input, |ui| {
        egui::Area::new(egui::Id::unique(super::super::super::INSPECTOR_ID)).show(ui.ctx(), |ui| {
            egui::ScrollArea::vertical()
                .max_height(40.0)
                .vertical_scroll_offset(offset)
                .show(ui, |ui| {
                    let mut text = String::from("Presentation test");
                    let id = name_focus::field_id(ui, Which::Name);
                    let response = ui.add(egui::TextEdit::singleline(&mut text).id(id));
                    name_focus::shown(ui, Which::Name, &response, editable, &text);
                    ui.add_space(400.0);
                });
        });
        if click {
            name_focus::apply(ui.ctx());
        }
    });
}

#[test]
fn empty_click_skips_a_name_scrolled_out_of_view_or_read_only() {
    for (offset, editable, focus) in [(0.0, true, true), (200.0, true, false), (0.0, false, false)] {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            lone_name(&ctx, offset, editable, false);
        }
        lone_name(&ctx, offset, editable, true);
        let id = name_focus::inspector_field(Which::Name);
        assert_eq!(
            ctx.memory(|m| m.focused()) == Some(id),
            focus,
            "offset {offset} editable {editable}"
        );
        let all = Some((0, "Presentation test".chars().count()));
        assert_eq!(selection_in(&ctx, Which::Name) == all, focus, "offset {offset} editable {editable}");
    }
}

#[test]
fn empty_click_on_another_tab_does_not_focus_a_hidden_name() {
    for legacy_window in [false, true] {
        let mut inspector = prepare(legacy_window);
        let section = other_section(&mut inspector, legacy_window);
        press(&mut inspector, section, egui::PointerButton::Primary);
        assert_eq!(inspector.tool.tab, 5, "the other section opened, legacy {legacy_window}");
        for _ in 0..3 {
            inspector.show(vec![]);
        }
        let (window, _) = inspector.show(vec![]);
        // Past the icon rail and the resize edge, in the other section's body.
        press(
            &mut inspector,
            egui::pos2(window.left() + 80.0, window.center().y),
            egui::PointerButton::Primary,
        );
        assert_ne!(focused(&inspector), Some(name_focus::inspector_field(Which::Name)));
        assert_ne!(focused(&inspector), Some(name_focus::inspector_field(Which::Desc)));
    }
}
