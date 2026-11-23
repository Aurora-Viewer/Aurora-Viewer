//! Empty clicks on the build window focus Name or Description and select all.
use super::super::super::name_focus;
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
    let field = inspector.ctx.read_response(name_focus::field_id("name")).expect("Name field");
    let x = (word.right() + field.rect.left()) * 0.5;
    let pos = egui::pos2(x, field.rect.center().y);
    assert!(
        window.contains(pos) && pos.x > word.right() + 2.0 && pos.x < field.rect.left() - 2.0,
        "blank beside Name must sit in the label column, not on the word or the field: word {word:?} field {:?} window {window:?}",
        field.rect
    );
    pos
}

fn selection(inspector: &Inspector, which: &str) -> Option<(usize, usize)> {
    let state = TextEditState::load(&inspector.ctx, name_focus::field_id(which))?;
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
        assert_eq!(focused(&inspector), Some(name_focus::field_id("name")), "legacy {legacy_window}");
        assert_eq!(
            selection(&inspector, "name"),
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
            .read_response(name_focus::field_id("desc"))
            .expect("Description field")
            .rect;
        press(&mut inspector, desc_rect.center(), egui::PointerButton::Primary);
        assert_eq!(focused(&inspector), Some(name_focus::field_id("desc")));
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
        assert_eq!(focused(&inspector), Some(name_focus::field_id("desc")), "legacy {legacy_window}");
        assert_eq!(
            selection(&inspector, "desc"),
            Some((0, desc.chars().count())),
            "legacy {legacy_window}"
        );

        let mut inspector = prepare(legacy_window);
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Secondary);
        assert_eq!(
            focused(&inspector),
            Some(name_focus::field_id("name")),
            "right click, legacy {legacy_window}"
        );
        assert_eq!(selection(&inspector, "name"), Some((0, name.chars().count())));

        let mut inspector = prepare(legacy_window);
        let section = other_section(&mut inspector, legacy_window);
        press(&mut inspector, section, egui::PointerButton::Primary);
        assert_ne!(focused(&inspector), Some(name_focus::field_id("name")));
        assert_ne!(selection(&inspector, "name"), Some((0, name.chars().count())));

        let mut inspector = prepare(legacy_window);
        let id = name_focus::field_id("name");
        inspector.show(vec![egui::Event::PointerMoved(egui::Pos2::ZERO)]);
        inspector.ctx.memory_mut(|m| m.request_focus(id));
        let mut state = TextEditState::load(&inspector.ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(4))));
        state.store(&inspector.ctx, id);
        inspector.show(vec![]);
        assert_eq!(selection(&inspector, "name"), Some((4, 4)));
        let blank = blank_beside_name(&mut inspector);
        press(&mut inspector, blank, egui::PointerButton::Primary);
        assert_eq!(
            selection(&inspector, "name"),
            Some((4, 4)),
            "already focused, legacy {legacy_window}"
        );

        let mut inspector = prepare(legacy_window);
        let field = inspector.ctx.read_response(name_focus::field_id("name")).unwrap().rect;
        press(&mut inspector, field.center(), egui::PointerButton::Primary);
        let (start, end) = selection(&inspector, "name").expect("caret");
        assert_eq!(start, end, "a direct click places a caret, legacy {legacy_window}");
        assert_ne!((start, end), (0, name.chars().count()));
    }
}

#[test]
fn empty_click_selects_a_read_only_name_without_editing_it() {
    let mut inspector = prepare(false);
    let idx = inspector.tool.sel_prims(&inspector.world)[0];
    inspector.world.objects.get_mut(idx).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
    for _ in 0..3 {
        inspector.show(vec![]);
    }
    let blank = blank_beside_name(&mut inspector);
    press(&mut inspector, blank, egui::PointerButton::Primary);
    assert_eq!(focused(&inspector), Some(name_focus::field_id("name")));
    assert_eq!(selection(&inspector, "name"), Some((0, "Presentation test".chars().count())));
    inspector.show(vec![egui::Event::Text("Q".into())]);
    let id = inspector.world.objects.get(idx).unwrap().full_id;
    assert_eq!(inspector.tool.props.get(&id).unwrap().name, "Presentation test");
    assert!(!inspector.tool.ui.edits.contains_key("name"));
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
        assert_ne!(focused(&inspector), Some(name_focus::field_id("name")));
        assert_ne!(focused(&inspector), Some(name_focus::field_id("desc")));
    }
}
