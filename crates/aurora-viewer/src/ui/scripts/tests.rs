//! The script window's tests: the linkset harness and the behaviours it drives
//! -- the tree, the tabs, the split, the docks, the search, the references and
//! the editor's own aids. They stand here, apart from the window they test, so
//! that `ui/scripts.rs` holds what the viewer does.

use super::*;
use crate::ui::inventory::permissions::Permissions;
use aurora_net::build::perm;

fn window(target: ScriptTarget) -> String {
    format!("script_editor_{}_{}", target.object, target.item)
}

fn source_id(target: ScriptTarget) -> egui::Id {
    egui::Id::unique((window(target), "source_text"))
}

/// The page of a new script watches the prim's listing for the item it
/// asked for: the first script that was not there when the page opened,
/// never one it already listed.
#[test]
fn a_page_finds_the_script_that_appeared_after_it_opened() {
    let script = |item: u128| linkset::Script {
        object: Uuid::from_u128(9),
        item: Uuid::from_u128(item),
        name: "a".into(),
        flags: 0,
        running: None,
        permissions: Permissions::default(),
        dirty: false,
        menu: None,
    };
    let (before, after) = (script(1), script(2));
    let listed = vec![before.clone(), after.clone()];
    assert_eq!(new_script_of(&listed, &[before.item; 1]).map(|s| s.item), Some(after.item));
    assert_eq!(
        new_script_of(&listed, &[before.item, after.item]).map(|s| s.item),
        None,
        "nothing new yet"
    );
    assert_eq!(new_script_of(&[], &[before.item]).map(|s| s.item), None, "an empty prim");
}

fn frame(ctx: &egui::Context, editors: &mut Editors, events: Vec<egui::Event>) -> egui::FullOutput {
    frame_settings(ctx, editors, &mut settings::ScriptSettings::default(), events)
}

fn frame_settings(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    frame_settings_extra(ctx, editors, settings, events, |_, _| {})
}

fn frame_settings_extra(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    events: Vec<egui::Event>,
    mut extra: impl FnMut(&egui::Context, &crate::theme::Palette),
) -> egui::FullOutput {
    let theme = crate::theme::Theme::default();
    theme.apply(ctx, 1.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            super::super::layout::frame(ui.ctx(), &theme.palette(), false);
            editors.show(
                ui.ctx(),
                &theme.palette(),
                settings,
                &mut super::super::color_picker::Preferences::default(),
                &[],
            );
            extra(ui.ctx(), &theme.palette());
        },
    );
    // These headless frames inspect shapes and responses without a renderer.
    output.textures_delta.clear();
    output
}

fn frame_linksets(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    events: Vec<egui::Event>,
    linksets: &[Linkset],
) -> egui::FullOutput {
    frame_linksets_actions(ctx, editors, settings, events, linksets).0
}

fn frame_linksets_actions(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    events: Vec<egui::Event>,
    linksets: &[Linkset],
) -> (egui::FullOutput, Vec<Action>) {
    let theme = crate::theme::Theme::default();
    theme.apply(ctx, 1.0);
    let mut actions = Vec::new();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            super::super::layout::frame(ui.ctx(), &theme.palette(), false);
            actions = editors.show(
                ui.ctx(),
                &theme.palette(),
                settings,
                &mut super::super::color_picker::Preferences::default(),
                linksets,
            );
        },
    );
    output.textures_delta.clear();
    (output, actions)
}

fn settle(ctx: &egui::Context, editors: &mut Editors) {
    for _ in 0..3 {
        frame(ctx, editors, vec![]).textures_delta.clear();
    }
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn click_label(ctx: &egui::Context, editors: &mut Editors, label: &str) {
    click_label_settings(ctx, editors, &mut settings::ScriptSettings::default(), label);
}

fn click_label_settings(ctx: &egui::Context, editors: &mut Editors, settings: &mut settings::ScriptSettings, label: &str) {
    let output = frame_settings(ctx, editors, settings, vec![]);
    // The lowest text of that label: the button, under the floating
    // window's field label of the same text ("Find" beside the query).
    let pos = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .expect("visible button");
    for pressed in [true, false] {
        frame_settings(
            ctx,
            editors,
            settings,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn ctrl_f_then_replace_changes_the_selected_word_whichever_way_it_was_selected() {
    // egui shrinks the editor's selection to its moving end once Find
    // takes the keyboard: the start of a word selected right to left.
    for (anchor, caret) in [(11, 16), (16, 11)] {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let t = target(1);
        let load = editors.open(t, "Replace selected".into(), true, true, 0).unwrap();
        editors.loaded(load, t, Ok("count = 1;\ncount += 2;\ncount_max = 3;".into()));
        settle(&ctx, &mut editors);
        let id = source_id(t);
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(anchor),
            egui::text::CCursor::new(caret),
        )));
        state.store(&ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        settle(&ctx, &mut editors);
        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        settle(&ctx, &mut editors);
        assert_eq!(editors.panels.search.query, "count");
        assert_eq!(
            editors.panels.search.active_range(&ctx, id, &editors.documents[0].source),
            Some(11..16),
            "{anchor}..{caret}: the selected word stays the active match"
        );
        editors.panels.search.replacement = "total".into();
        // The Replace button, not the replacement field's label of the same text.
        let output = frame(&ctx, &mut editors, vec![]);
        let pos = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-replace") => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .max_by(|a, b| a.y.total_cmp(&b.y))
            .expect("visible Replace button");
        for pressed in [true, false] {
            frame(
                &ctx,
                &mut editors,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert_eq!(
            editors.documents[0].source, "count = 1;\ntotal += 2;\ncount_max = 3;",
            "{anchor}..{caret}"
        );
    }
}

#[test]
fn ctrl_f_after_find_keeps_the_found_match_for_replace_inline_and_floating() {
    fn click_text(ctx: &egui::Context, editors: &mut Editors, settings: &mut settings::ScriptSettings, label: &str) {
        // The lowest text of that label: the button, not the field label
        // "Replace" beside the floating window's replacement field.
        let output = frame_settings(ctx, editors, settings, vec![]);
        let pos = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + t.galley.size() * 0.5),
                _ => None,
            })
            .max_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)))
            .expect("visible button");
        for pressed in [true, false] {
            frame_settings(
                ctx,
                editors,
                settings,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }
    for inline in [true, false] {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let t = ScriptTarget {
            object: Uuid::from_u128(77),
            item: Uuid::from_u128(5),
        };
        let load = editors.open(t, "Find then Ctrl+F".into(), true, true, 0).unwrap();
        editors.loaded(load, t, Ok("default\n{\n    count\n    llSay(0, \"x\");\n    count\n}\n".into()));
        let mut settings = settings::ScriptSettings {
            search_inline: inline,
            ..Default::default()
        };
        super::super::widgets::place_floater(&ctx, &window(t), egui::pos2(100.0, 100.0), Vec2::new(900.0, 650.0));
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        let id = source_id(t);
        editing::select(&ctx, id, 2..2, true);
        frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        for _ in 0..3 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        editors.panels.search.query = "count".into();
        editors.panels.search.replacement = "hi".into();
        for _ in 0..2 {
            click_text(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-find"));
        }
        let second = 43..48;
        assert_eq!(
            editors.panels.search.active_range(&ctx, id, &editors.documents[0].source),
            Some(second.clone()),
            "inline {inline}: Find went to the second count"
        );
        // Ctrl+F again: egui has shrunk the editor's selection to a caret,
        // which must not drop the match Find went to.
        frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        for _ in 0..3 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        assert_eq!(
            editors.panels.search.active_range(&ctx, id, &editors.documents[0].source),
            Some(second),
            "inline {inline}: Ctrl+F keeps the active match"
        );
        click_text(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-replace"));
        assert_eq!(
            editors.documents[0].source, "default\n{\n    count\n    llSay(0, \"x\");\n    hi\n}\n",
            "inline {inline}"
        );
    }
}

#[test]
fn opening_scripts_focuses_character_zero_and_late_loads_do_not_steal_focus() {
    for object in [Uuid::nil(), Uuid::from_u128(22)] {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let first = ScriptTarget {
            object,
            item: Uuid::from_u128(1),
        };
        let second = ScriptTarget {
            object,
            item: Uuid::from_u128(2),
        };
        let pending = editors.open(first, "First".into(), true, true, 0).unwrap();
        settle(&ctx, &mut editors);
        let load = editors.open(second, "Second".into(), true, true, 0).unwrap();
        editors.loaded(load, second, Ok("é source".into()));
        settle(&ctx, &mut editors);
        assert!(ctx.memory(|m| m.has_focus(source_id(second))));
        assert_eq!(editing::cursor(&ctx, source_id(second)).as_sorted_char_range(), 0.into()..0.into());
        frame(&ctx, &mut editors, vec![egui::Event::Text("//".into())]);
        assert_eq!(editors.documents[1].source, "//é source");
        editors.loaded(pending, first, Ok("late source".into()));
        settle(&ctx, &mut editors);
        assert!(ctx.memory(|m| m.has_focus(source_id(second))));

        editing::select(&ctx, source_id(first), 3..7, false);
        ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((window(first), "minimized")), true));
        settle(&ctx, &mut editors);
        editors.open(first, "First".into(), true, true, 0);
        settle(&ctx, &mut editors);
        assert!(!super::super::widgets::floater_minimized(&ctx, &window(first)));
        assert!(ctx.memory(|m| m.has_focus(source_id(first))));
        assert_eq!(editing::cursor(&ctx, source_id(first)).as_sorted_char_range(), 0.into()..0.into());
        assert_eq!(editors.documents[0].source, "late source");
    }
}

#[test]
fn script_search_reuses_its_window_and_options_and_finds_in_the_new_owner() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let first = target(1);
    let second = ScriptTarget {
        object: Uuid::from_u128(22),
        item: Uuid::from_u128(2),
    };
    for target in [first, second] {
        let load = editors.open(target, "Search owner".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("first needle\nsecond needle".into()));
    }
    settle(&ctx, &mut editors);
    editors.open(first, "Search owner".into(), true, true, 0);
    settle(&ctx, &mut editors);
    frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
    settle(&ctx, &mut editors);
    assert!(editors.panels.search.owns(source_id(first)));
    editors.panels.search.query = "needle".into();
    editors.panels.search.replacement = "replacement".into();
    editors.panels.search.insensitive = false;
    editors.panels.search.up = true;
    // Rebind the still-open search, then reopen it after closing/minimizing.
    for closed in [false, true] {
        if closed {
            editors.panels.search.open = false;
            ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((dialogs::Search::WINDOW, "minimized")), true));
        }
        editors.open(second, "Search owner".into(), true, true, 0);
        settle(&ctx, &mut editors);
        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        settle(&ctx, &mut editors);
        assert!(editors.panels.search.owns(source_id(second)));
        assert!(!editors.panels.search.owns(source_id(first)));
        assert_eq!(editors.panels.search.query, "needle");
        assert_eq!(editors.panels.search.replacement, "replacement");
        assert!(!editors.panels.search.insensitive);
        assert!(editors.panels.search.up);
        assert!(!super::super::widgets::floater_minimized(&ctx, dialogs::Search::WINDOW));
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));
        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        settle(&ctx, &mut editors);
        assert!(editor_focused(&ctx));
        assert!(
            editors.panels.search.owns(source_id(second)),
            "Ctrl+F keeps Find open for the same editor"
        );
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));

        frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        settle(&ctx, &mut editors);
        assert!(editors.panels.search.owns(source_id(second)), "repeated Ctrl+F keeps Find open");
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));
        frame(&ctx, &mut editors, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
        assert_eq!(
            editing::cursor(&ctx, source_id(second)).as_sorted_char_range(),
            20.into()..26.into()
        );
        assert!(editing::cursor(&ctx, source_id(first)).is_empty());
    }
    click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
    assert_eq!(editors.documents[0].source, "first needle\nsecond needle");
    assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
    editors.documents[1].can_edit = false;
    editors.panels.search.query = "replacement".into();
    editors.panels.search.replacement = "blocked".into();
    click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
    assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
    editors.documents[1].can_edit = true;
    editors.documents[1].tools.processed = true;
    editors.documents[1].tools.processed_source = "replacement".into();
    click_label(&ctx, &mut editors, aurora_i18n::tr!("script-search-replace-all"));
    assert_eq!(editors.documents[1].tools.processed_source, "replacement");
    assert_eq!(editors.documents[1].source, "first replacement\nsecond replacement");
}

#[test]
fn ctrl_f_selects_the_whole_existing_query_in_floating_and_inline_search() {
    for inline in [false, true] {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let target = target(1);
        let load = editors.open(target, "Select search query".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("source".into()));
        let mut settings = settings::ScriptSettings {
            search_inline: inline,
            ..Default::default()
        };
        super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(100.0, 100.0), Vec2::new(1040.0, 650.0));
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        for _ in 0..3 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        let search_id = egui::Id::unique(dialogs::Search::WINDOW);
        let query_id = search_id.with("query");
        let replacement_id = search_id.with("replacement");
        assert!(ctx.memory(|m| m.has_focus(query_id)));
        assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 0.into()..0.into());
        assert_eq!(
            ctx.read_response(query_id).unwrap().layer_id.id,
            if inline { search_id.with("popup") } else { search_id },
            "the bar in the window, or the panel beside the editor"
        );

        for field in [source_id(target), query_id, replacement_id] {
            editors.panels.search.query = "étoile 🌟".into();
            editors.panels.search.replacement = "replacement".into();
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            let at = if field == source_id(target) { 0 } else { 2 };
            editing::select(&ctx, field, at..at, true);
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            for _ in 0..3 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            assert!(editors.panels.search.owns(source_id(target)), "Ctrl+F keeps Find open");
            assert!(ctx.memory(|m| m.has_focus(query_id)));
            frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
            for _ in 0..3 {
                frame_settings(&ctx, &mut editors, &mut settings, vec![]);
            }
            assert!(editors.panels.search.owns(source_id(target)));
            assert!(ctx.memory(|m| m.has_focus(query_id)));
            assert_eq!(editors.panels.search.query, "étoile 🌟");
            assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 0.into()..8.into());
            frame_settings(&ctx, &mut editors, &mut settings, vec![egui::Event::Text("new".into())]);
            assert_eq!(editors.panels.search.query, "new");
            assert_eq!(editors.panels.search.replacement, "replacement");
            assert_eq!(editors.documents[0].source, "source");
        }
        // Finding with Enter must not reset a manually placed query caret.
        editing::select(&ctx, query_id, 1..1, true);
        frame_settings(
            &ctx,
            &mut editors,
            &mut settings,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        assert_eq!(editing::cursor(&ctx, query_id).as_sorted_char_range(), 1.into()..1.into());
    }
}

#[test]
fn script_search_tries_both_sides_then_overlays_its_owner_and_hides_with_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(1);
    let load = editors.open(target, "Anchored".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("source".into()));
    settle(&ctx, &mut editors);
    frame(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
    for (pos, size) in [
        (egui::pos2(100.0, 70.0), Vec2::new(660.0, 520.0)),
        (egui::pos2(430.0, 190.0), Vec2::new(660.0, 520.0)),
        (egui::pos2(260.0, 120.0), Vec2::new(900.0, 650.0)),
        (egui::pos2(850.0, 120.0), Vec2::new(660.0, 520.0)),
    ] {
        super::super::widgets::place_floater(&ctx, &window(target), pos, size);
        settle(&ctx, &mut editors);
        let parent = ctx.memory(|m| m.area_rect(egui::Id::unique(window(target)))).unwrap();
        let search = ctx.memory(|m| m.area_rect(egui::Id::unique(dialogs::Search::WINDOW))).unwrap();
        assert!(search.width() < 420.0, "compact default: {search:?}");
        let expected = if parent.right() + search.width() <= 1600.0 {
            parent.right_top()
        } else if parent.left() >= search.width() {
            egui::pos2(parent.left() - search.width(), parent.top())
        } else {
            egui::pos2(parent.right() - search.width(), editors.documents[0].tools.body_top)
        };
        assert!((search.left_top() - expected).length() < 1.5, "{parent:?} / {search:?}");
    }
    ctx.data_mut(|d| d.insert_persisted(egui::Id::unique((window(target), "minimized")), true));
    settle(&ctx, &mut editors);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(
        !output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title")))
    );
    editors.open(target, "Anchored".into(), true, true, 0);
    settle(&ctx, &mut editors);
    assert!(editors.panels.search.open);
    editors.demo_close_clean(target);
    settle(&ctx, &mut editors);
    assert!(!editors.panels.search.open);
}

/// Find / Replace hang in a popup under the window's menus and toolbar,
/// over the source, at any width, and keep their fields, their options and
/// the keyboard while switching between the popup and the panel beside the
/// window: no row of the editor is taken from the text.
#[test]
fn find_hangs_under_the_toolbar_and_keeps_its_fields_and_focus() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(1);
    let load = editors.open(target, "Responsive search".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("needle needle".into()));
    let mut settings = settings::ScriptSettings {
        search_prefer_left: true,
        search_inline: true,
        ..Default::default()
    };
    let parent_id = egui::Id::unique(window(target));
    let search_id = egui::Id::unique(dialogs::Search::WINDOW);
    let bar_id = search_id.with("popup");
    let query_id = search_id.with("query");
    super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(470.0, 120.0), Vec2::new(660.0, 650.0));
    for _ in 0..4 {
        frame_settings(&ctx, &mut editors, &mut settings, vec![]);
    }
    frame_settings(&ctx, &mut editors, &mut settings, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
    editors.panels.search.query = "needle".into();
    editors.panels.search.replacement = "thread".into();
    editors.panels.search.insensitive = false;
    editors.panels.search.up = true;
    for _ in 0..4 {
        frame_settings(&ctx, &mut editors, &mut settings, vec![]);
    }
    for width in [660.0, 840.0, 1250.0] {
        super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(100.0, 120.0), Vec2::new(width, 650.0));
        // the reader's caret in the query, in the bar under the toolbar
        editing::select(&ctx, query_id, 2..2, true);
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        let output = frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        let query = ctx.read_response(query_id).unwrap();
        assert_eq!(query.layer_id.id, bar_id, "the bar in the window, at {width}");
        let bar = ctx.memory(|m| m.area_rect(bar_id)).unwrap();
        let parent = ctx.memory(|m| m.area_rect(parent_id)).unwrap();
        let source = ctx.read_response(source_id(target)).unwrap().rect;
        assert!(bar.top() > parent.top(), "under the toolbar: {bar:?} in {parent:?}");
        assert!(
            bar.right() <= parent.right() + 1.5 && bar.left() >= parent.left() - 1.5,
            "the bar stays in the window's row: {bar:?} / {parent:?}"
        );
        assert!(
            bar.top() < source.top() + source.height() / 2.0 && bar.bottom() > source.top(),
            "over the text: {bar:?} / {source:?}"
        );
        assert!(ctx.memory(|m| m.has_focus(query_id)), "the bar keeps the keyboard");
        assert_eq!(editors.panels.search.query, "needle");
        assert_eq!(editors.panels.search.replacement, "thread");
        assert!(!editors.panels.search.insensitive && editors.panels.search.up);
        // the bar has no title of a window over its one row
        assert!(
            !output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title")))
        );
        let label_at = |label| {
            output
                .shapes
                .iter()
                .enumerate()
                .find_map(|(index, s)| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == label => Some((index, t.pos)),
                    _ => None,
                })
                .unwrap()
        };
        let (find, find_at) = label_at(aurora_i18n::tr!("script-search-find"));
        let (replace, replace_at) = label_at(aurora_i18n::tr!("script-search-replace"));
        let (replace_all, replace_all_at) = label_at(aurora_i18n::tr!("script-search-replace-all"));
        assert!(find < replace && replace < replace_all, "Find, then Replace, then Replace all");
        if width > 1000.0 {
            // the whole bar on one row where the window has the room
            assert!(
                (find_at.y - replace_all_at.y).abs() < 1.5 && find_at.x < replace_at.x && replace_at.x < replace_all_at.x,
                "the bar across one row: {find_at:?} / {replace_at:?} / {replace_all_at:?}"
            );
        }
        // the panel beside the window, as the setting off asks for, takes
        // no row either and keeps the fields
        settings.search_inline = false;
        for _ in 0..4 {
            frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        }
        assert_eq!(
            ctx.read_response(query_id).unwrap().layer_id.id,
            search_id,
            "the panel beside the editor"
        );
        if width < 1000.0 {
            // the screen has room beside the window: the panel takes it
            let parent = ctx.memory(|m| m.area_rect(parent_id)).unwrap();
            let beside = ctx.memory(|m| m.area_rect(search_id)).unwrap();
            assert!(
                (beside.right() - parent.left()).abs() < 1.5 || (beside.left() - parent.right()).abs() < 1.5,
                "a panel beside the editor: {parent:?} / {beside:?}"
            );
        }
        let source_beside = ctx.read_response(source_id(target)).unwrap().rect;
        assert!(
            (source.top() - source_beside.top()).abs() < 1.5 && (source.height() - source_beside.height()).abs() < 1.5,
            "the mode takes no row from the text: {source_beside:?} / {source:?}"
        );
        assert_eq!(editors.panels.search.query, "needle");
        assert_eq!(editors.panels.search.replacement, "thread");
        assert!(!editors.panels.search.insensitive && editors.panels.search.up);
        settings.search_inline = true;
    }
    editing::select(&ctx, source_id(target), 0..0, false);
    click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-find"));
    assert_eq!(editing::cursor(&ctx, source_id(target)).as_sorted_char_range(), 7.into()..13.into());
    editors.panels.search.query = "missing".into();
    editors.panels.search.find(&ctx, source_id(target), &editors.documents[0].source);
    let source_before_status = ctx.read_response(source_id(target)).unwrap().rect;
    for _ in 0..4 {
        frame_settings(&ctx, &mut editors, &mut settings, vec![]);
    }
    assert_eq!(
        ctx.read_response(source_id(target)).unwrap().rect,
        source_before_status,
        "the bar's word stays in the bar, over the text"
    );
    editors.panels.search.query = "needle".into();
    click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-replace-all"));
    assert_eq!(editors.documents[0].source, "thread thread");
    editors.documents[0].can_edit = false;
    editors.panels.search.query = "thread".into();
    editors.panels.search.replacement = "blocked".into();
    click_label_settings(&ctx, &mut editors, &mut settings, aurora_i18n::tr!("script-search-replace-all"));
    assert_eq!(editors.documents[0].source, "thread thread");
}

#[test]
fn overlapping_search_stays_above_its_owner_without_covering_unrelated_windows() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let first = target(1);
    let second = target(2);
    for target in [first, second] {
        let load = editors.open(target, "Layer ordering".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("source".into()));
    }
    settle(&ctx, &mut editors);
    super::super::widgets::place_floater(&ctx, &window(first), egui::pos2(260.0, 120.0), Vec2::new(1100.0, 650.0));
    editors.panels.search.begin(&ctx, source_id(first), source_id(first), "source");
    settle(&ctx, &mut editors);
    let owner = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window(first)));
    let other = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window(second)));
    let search = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(dialogs::Search::WINDOW));
    for top in [owner, other] {
        ctx.move_to_top(top);
        settle(&ctx, &mut editors);
        let layers = ctx.memory(|m| m.layer_ids().collect::<Vec<_>>());
        let at = |layer| layers.iter().position(|&l| l == layer).unwrap();
        assert!(at(search) > at(owner));
        if top == other {
            assert!(at(other) > at(search));
        }
    }
}

#[test]
fn dragging_scripts_can_cross_closed_edges_but_keeps_the_title_reachable() {
    for delta in [
        egui::vec2(-2000.0, 0.0),
        egui::vec2(2000.0, 0.0),
        egui::vec2(0.0, 2000.0),
        egui::vec2(0.0, -2000.0),
    ] {
        let ctx = egui::Context::default();
        let mut editors = Editors::default();
        let target = target(1);
        let load = editors.open(target, "Reachable".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("source".into()));
        settle(&ctx, &mut editors);
        let id = egui::Id::unique(window(target));
        let start = ctx.read_response(id.with("title_double_click")).unwrap().rect.center();
        frame(&ctx, &mut editors, vec![egui::Event::PointerMoved(start)]);
        frame(
            &ctx,
            &mut editors,
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let end = start + delta;
        frame(&ctx, &mut editors, vec![egui::Event::PointerMoved(end)]);
        frame(
            &ctx,
            &mut editors,
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        settle(&ctx, &mut editors);
        let rect = ctx.memory(|m| m.area_rect(id)).unwrap();
        let room = super::super::layout::move_area(&ctx);
        let visible = rect.intersect(room);
        assert!(visible.width() >= super::super::layout::TITLE_GRIP - 1.5, "{delta:?}: {rect:?}");
        assert!(visible.height() >= super::super::layout::TITLE_BAR_H - 1.5, "{delta:?}: {rect:?}");
        if delta.x < 0.0 {
            assert!(rect.left() < room.left());
        }
        if delta.x > 0.0 {
            assert!(rect.right() > room.right());
        }
        if delta.y > 0.0 {
            assert!(rect.bottom() > room.bottom());
        }
        assert!(rect.top() >= room.top());
        assert_eq!(
            super::super::layout::edges_for(&ctx, egui::Id::unique("ordinary_window")),
            super::super::layout::Edges::default()
        );
    }
}

#[test]
fn ctrl_f_follows_script_window_activation_without_needing_source_field_focus() {
    fn show(ctx: &egui::Context, editors: &mut Editors, events: Vec<egui::Event>) -> egui::FullOutput {
        frame_settings_extra(ctx, editors, &mut settings::ScriptSettings::default(), events, |ctx, p| {
            let mut other = String::from("Another window");
            super::super::widgets::Floater::new("script_shortcut_other", "Other", egui::pos2(1200.0, 720.0), Vec2::new(260.0, 200.0)).show(
                ctx,
                p,
                &mut true,
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut other).id(egui::Id::unique("script_shortcut_other_field")));
                },
            );
        })
    }
    fn click(ctx: &egui::Context, editors: &mut Editors, pos: egui::Pos2) {
        show(ctx, editors, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            show(
                ctx,
                editors,
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
    }
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let first = target(1);
    let second = target(2);
    for (target, x) in [(first, 50.0), (second, 780.0)] {
        let load = editors.open(target, "Activation".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("source".into()));
        super::super::widgets::place_floater(&ctx, &window(target), egui::pos2(x, 100.0), Vec2::new(660.0, 520.0));
    }
    for _ in 0..4 {
        show(&ctx, &mut editors, vec![]);
    }
    for (target, area) in [
        (first, "title"),
        (second, "header"),
        (first, "control"),
        (second, "footer"),
        (first, "menu"),
    ] {
        editors.panels.search.open = false;
        for _ in 0..2 {
            show(&ctx, &mut editors, vec![]);
        }
        let parent = ctx.memory(|m| m.area_rect(egui::Id::unique(window(target)))).unwrap();
        let editor = editors.documents.iter().find(|e| e.target == target).unwrap();
        let pos = match area {
            "title" => ctx
                .read_response(egui::Id::unique(window(target)).with("title_double_click"))
                .unwrap()
                .rect
                .center(),
            "header" => egui::pos2(parent.right() - 25.0, editor.tools.body_top + 8.0),
            "footer" => egui::pos2(parent.center().x, parent.bottom() - 25.0),
            _ => {
                let label = if area == "menu" {
                    aurora_i18n::tr!("script-menu-file")
                } else {
                    aurora_i18n::tr!("script-target-mono")
                };
                show(&ctx, &mut editors, vec![])
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.job.text == label && parent.contains(t.pos) => Some(t.pos + t.galley.size() * 0.5),
                        _ => None,
                    })
                    .unwrap()
            }
        };
        click(&ctx, &mut editors, pos);
        assert!(
            super::super::find_shortcut::window_takes_it(&ctx),
            "{area}: app must yield Ctrl+F to the script"
        );
        // Hovering another window does not move activation there.
        show(&ctx, &mut editors, vec![egui::Event::PointerMoved(egui::pos2(1330.0, 800.0))]);
        assert!(
            super::super::find_shortcut::window_takes_it(&ctx),
            "{area}: pointer hover must not steal it"
        );
        show(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        for _ in 0..3 {
            show(&ctx, &mut editors, vec![]);
        }
        assert!(
            editors.panels.search.owns(source_id(target)),
            "{area}: find must bind the clicked script"
        );
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(dialogs::Search::WINDOW).with("query"))));
    }
    for outside in [
        ctx.read_response(egui::Id::unique("script_shortcut_other").with("title_double_click"))
            .unwrap()
            .rect
            .center(),
        ctx.read_response(egui::Id::unique("script_shortcut_other_field"))
            .unwrap()
            .rect
            .center(),
        egui::pos2(1560.0, 950.0),
    ] {
        editors.panels.search.open = false;
        for _ in 0..2 {
            show(&ctx, &mut editors, vec![]);
        }
        let title = ctx
            .read_response(egui::Id::unique(window(first)).with("title_double_click"))
            .unwrap()
            .rect
            .center();
        click(&ctx, &mut editors, title);
        assert!(super::super::find_shortcut::window_takes_it(&ctx));
        click(&ctx, &mut editors, outside);
        assert!(
            !super::super::find_shortcut::window_takes_it(&ctx),
            "outside click must release script Ctrl+F"
        );
        show(&ctx, &mut editors, vec![key(egui::Key::F, egui::Modifiers::COMMAND)]);
        assert!(!editors.panels.search.open);
    }
    editors.documents.clear();
    show(&ctx, &mut editors, vec![]);
    assert!(!super::super::find_shortcut::window_takes_it(&ctx));
}

/// Escape closes Find while its field has the keyboard, and the script
/// takes the keyboard back: an IDE's find bar.
#[test]
fn escape_closes_the_find_bar_and_returns_the_keyboard() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(97);
    let load = editors.open(target, "Find".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("llSay(0, \"hi\");".into()));
    settle(&ctx, &mut editors);
    let owner = source_id(target);
    editors.panels.search.begin(&ctx, owner, owner, "llSay");
    let query = egui::Id::unique(super::dialogs::Search::WINDOW).with("query");
    // a window lays out a first, unmeasured frame before its fields take
    // the keyboard
    for _ in 0..3 {
        frame(&ctx, &mut editors, vec![]);
    }
    assert!(ctx.memory(|m| m.has_focus(query)), "the query field has the keyboard");
    frame(&ctx, &mut editors, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(!editors.panels.search.open, "Escape closes it");
    assert!(ctx.memory(|m| m.has_focus(owner)), "the script has the keyboard back");
}

fn target(n: u128) -> ScriptTarget {
    ScriptTarget {
        object: Uuid::nil(),
        item: Uuid::from_u128(n),
    }
}

#[test]
fn a_lua_script_is_saved_for_luau_without_the_lsl_preprocessor() {
    let target = target(80);
    let mut editors = Editors::default();
    // SST_LUA in the item's low byte; other flag bits do not count
    let load = editors.open(target, "Lua".into(), true, true, 0x0100_0001).unwrap();
    editors.loaded(load, target, Ok("print(\"hi\")".into()));
    // A running reply that says Mono does not turn a Lua source into LSL.
    editors.running(target, true, Some(CompileTarget::Mono));
    let e = &mut editors.documents[0];
    e.processor_settings.preprocessor = true;
    e.source = "#include \"x\"
print(\"bye\")"
        .into();
    let Some(Action::Save { options, source, .. }) = e.save() else {
        panic!("a Lua source is saved at once, not preprocessed")
    };
    assert!(e.processing.is_none());
    assert_eq!(
        source,
        "#include \"x\"
print(\"bye\")"
    );
    assert_eq!(options.compile, CompileTarget::Luau);
}

#[test]
fn an_lsl_script_on_the_luau_vm_keeps_it_and_the_first_reply_wins() {
    let target = target(81);
    let mut editors = Editors::default();
    let load = editors.open(target, "LSL on Luau".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default {}".into()));
    editors.running(target, true, Some(CompileTarget::LslLuau));
    editors.running(target, false, None);
    let e = &mut editors.documents[0];
    e.source = "default { state_entry() {} }".into();
    let Some(Action::Save { options, .. }) = e.save() else {
        panic!("save")
    };
    assert_eq!(options.compile, CompileTarget::LslLuau);
    assert!(!options.running);
}

#[test]
fn asynchronous_preprocessing_loads_includes_and_saves_a_snapshot_that_reopens_as_original_source() {
    let target = target(70);
    let mut editors = Editors::default();
    let load = editors.open(target, "Main".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default {}".into()));
    let original = "#include \"library.lsl\"\ndefault { state_entry() { greet(); } }";
    let settings = settings::ScriptSettings {
        preprocessor: true,
        ..Default::default()
    };
    let e = &mut editors.documents[0];
    e.processor_settings = settings.clone();
    e.source = original.into();
    assert!(e.save().is_none());
    assert!(e.saving.is_some());
    e.close_after_save = true;
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let mut settings = settings;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut uploaded = None;
    while std::time::Instant::now() < until && uploaded.is_none() {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            actions = editors.show(
                ui.ctx(),
                &p,
                &mut settings,
                &mut crate::ui::color_picker::Preferences::default(),
                &[],
            )
        });
        // No renderer consumes texture updates in this headless test.
        output.textures_delta.clear();
        for action in actions {
            match action {
                Action::Include { request, name } => {
                    assert_eq!(name, "library.lsl");
                    editors.include_asset(request, Uuid::from_u128(71));
                    editors.loaded(request, target, Ok("greet() { llOwnerSay(__ASSETID__); }".into()));
                }
                Action::Save { request, source, .. } => uploaded = Some((request, source)),
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (request, uploaded) = uploaded.expect("worker produced upload");
    assert!(uploaded.contains(&Uuid::from_u128(71).to_string()));
    assert_eq!(preprocessor::decode(&uploaded), original);
    assert_eq!(editors.documents[0].source, original);
    editors.saved(
        request,
        target,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(72),
            compiled: true,
            checked: true,
            errors: vec![],
        }),
    );
    assert!(editors.documents[0].closed);
    editors.documents.clear();
    let load = editors.open(target, "Main".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(uploaded));
    assert_eq!(editors.documents[0].source, original);
    assert!(!editors.documents[0].dirty());
}

#[test]
fn failed_preprocessing_keeps_the_draft_and_emits_no_network_save() {
    let target = target(80);
    let mut editors = Editors::default();
    let load = editors.open(target, "Main".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default {}".into()));
    let mut settings = settings::ScriptSettings {
        preprocessor: true,
        ..Default::default()
    };
    editors.documents[0].processor_settings = settings.clone();
    editors.documents[0].source = "#error test".into();
    assert!(editors.documents[0].save().is_none());
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while editors.documents[0].processing.is_some() && std::time::Instant::now() < until {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(
                !editors
                    .show(
                        ui.ctx(),
                        &p,
                        &mut settings,
                        &mut crate::ui::color_picker::Preferences::default(),
                        &[],
                    )
                    .iter()
                    .any(|a| matches!(a, Action::Save { .. }))
            )
        });
        output.textures_delta.clear();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let e = &editors.documents[0];
    assert!(e.processing.is_none());
    assert!(e.saving.is_none());
    assert!(e.compile_failed);
    assert!(e.dirty());
    assert_eq!(e.source, "#error test");
    assert_eq!(e.baseline, "default {}");
    assert!(!e.tools.diagnostics_processed);
}

#[test]
fn experience_only_changes_and_removal_are_saved_with_their_snapshot() {
    use aurora_net::scripts::experiences::ScriptExperiences;
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(20),
        item: Uuid::from_u128(1),
    };
    let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
    editors.loaded(request, target, Ok("original".into()));
    editors.running(target, true, Some(CompileTarget::Mono));
    let Action::Experiences { request, .. } = editors.load_experiences(target).expect("experiences") else {
        panic!("load experiences")
    };
    editors.experiences_loaded(
        request,
        target,
        ScriptExperiences {
            associated: Ok(Uuid::nil()),
            available: Ok(vec![]),
        },
    );
    assert!(!editors.documents[0].dirty());
    for id in [Uuid::from_u128(45), Uuid::nil()] {
        editors.documents[0].experience.selected = Some(id);
        assert!(!editors.prepare_exit());
        let Action::Save {
            request, options, source, ..
        } = editors.documents[0].save().expect("save association")
        else {
            panic!("save")
        };
        assert_eq!(options.experience, id);
        assert_eq!(source, "original");
        editors.saved(
            request,
            target,
            &Ok(ScriptSaved {
                asset: Uuid::from_u128(99),
                compiled: true,
                checked: true,
                errors: vec![],
            }),
        );
        assert!(!editors.documents[0].dirty());
    }
}

#[test]
fn unknown_association_requires_a_choice_and_disconnect_rejects_metadata() {
    use aurora_net::scripts::experiences::ScriptExperiences;
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(20),
        item: Uuid::from_u128(1),
    };
    let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
    editors.loaded(request, target, Ok("original".into()));
    editors.running(target, true, Some(CompileTarget::Mono));
    editors.documents[0].source.push_str(" // edit");
    let Action::Experiences { request, .. } = editors.load_experiences(target).expect("experiences") else {
        panic!("load")
    };
    assert!(!editors.documents[0].can_save());
    editors.experiences_loaded(
        request,
        target,
        ScriptExperiences {
            associated: Err("unavailable".into()),
            available: Ok(vec![]),
        },
    );
    assert!(!editors.documents[0].can_save());
    editors.documents[0].experience.selected = Some(Uuid::nil());
    assert!(editors.documents[0].can_save());
    let Action::Experiences { request, .. } = editors.load_experiences(target).expect("retry") else {
        panic!("load")
    };
    editors.disconnected();
    editors.experiences_loaded(
        request,
        target,
        ScriptExperiences {
            associated: Ok(Uuid::from_u128(45)),
            available: Ok(vec![]),
        },
    );
    assert_eq!(editors.documents[0].experience.selected, Some(Uuid::nil()));
    assert!(!editors.documents[0].can_save());
}

#[test]
fn loaded_sources_are_clean_and_reopening_waits_for_the_server() {
    let mut editors = Editors::default();
    for object in [Uuid::nil(), Uuid::from_u128(10)] {
        let target = ScriptTarget {
            object,
            item: Uuid::from_u128(1),
        };
        let source = "default { state_entry() { llSay(0, \"Saved source\"); } }";
        let request = editors.open(target, "Test".into(), true, true, 0).expect("request existing script");
        assert!(editors.documents[0].source.is_empty());
        assert!(!editors.documents[0].can_save());
        editors.loaded(request, target, Ok(source.into()));
        assert_eq!(editors.documents[0].source, source);
        assert!(!editors.documents[0].dirty());
        assert!(editors.prepare_exit());
        assert!(editors.demo_close_clean(target));
        let next = editors.open(target, "Test".into(), true, true, 0).expect("request again");
        assert_ne!(request, next);
        assert!(editors.documents[0].source.is_empty());
        editors.loaded(request, target, Ok("stale".into()));
        assert!(editors.documents[0].source.is_empty());
        editors.loaded(next, target, Ok(source.into()));
        assert_eq!(editors.documents[0].source, source);
        assert!(!editors.documents[0].dirty());
        editors.documents[0].source.push_str("\n// Unsaved edit");
        assert!(!editors.demo_close_clean(target));
        assert!(!editors.prepare_exit());
        editors.documents.clear();
    }
}

#[test]
fn cascade_inventory_and_object_scripts_share_one_group_without_replacing_drafts() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let mut colors = super::super::color_picker::Preferences::default();
    let targets = [
        ScriptTarget {
            object: Uuid::nil(),
            item: Uuid::from_u128(1),
        },
        ScriptTarget {
            object: Uuid::from_u128(10),
            item: Uuid::from_u128(1),
        },
    ];
    let mut frame = 0;
    for target in targets {
        let request = editors.open(target, "Script".into(), true, true, 0).expect("load");
        editors.loaded(request, target, Ok("default { state_entry() {} }".into()));
        for _ in 0..8 {
            frame += 1;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1200.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    editors.show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut settings, &mut colors, &[]);
                },
            );
            output.textures_delta.clear();
        }
    }
    let rects: Vec<_> = targets
        .iter()
        .map(|t| {
            ctx.memory(|m| {
                m.area_rect(egui::Id::unique(format!("script_editor_{}_{}", t.object, t.item)))
                    .expect("script window")
            })
        })
        .collect();
    assert!(((rects[1].min - rects[0].min) - egui::Vec2::splat(32.0)).length() < 1.0);
    editors.documents[0].source.push_str("\n// Unsaved draft");
    assert!(editors.open(targets[0], "Script".into(), true, true, 0).is_none());
    assert_eq!(editors.documents.len(), 2);
    assert!(editors.documents[0].dirty());
    assert!(editors.documents[0].source.ends_with("// Unsaved draft"));
}

#[test]
fn load_and_save_replies_are_scoped_to_the_request_and_document() {
    let mut editors = Editors::default();
    let target = target(1);
    let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
    editors.loaded(Uuid::new_v4(), target, Ok("stale".into()));
    assert!(editors.documents[0].source.is_empty());
    editors.loaded(request, target, Ok("original".into()));
    editors.documents[0].source = "edited".into();
    let Action::Save { request, .. } = editors.documents[0].save().expect("save") else {
        panic!("save action")
    };
    assert!(!editors.saved(
        Uuid::new_v4(),
        target,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(3),
            compiled: true,
            checked: true,
            errors: vec![]
        })
    ));
    assert!(editors.documents[0].dirty());
    editors.saved(request, target, &Err("offline".into()));
    assert!(editors.documents[0].dirty());
    assert!(editors.documents[0].can_save());
}

#[test]
fn compiler_errors_keep_the_editor_open_but_track_the_saved_source() {
    let mut editors = Editors::default();
    let target = target(1);
    let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
    editors.loaded(request, target, Ok("original".into()));
    editors.documents[0].source = "invalid source".into();
    let Action::Save { request, .. } = editors.documents[0].save().expect("save script") else {
        panic!("save action")
    };
    editors.documents[0].close_after_save = true;
    editors.saved(
        request,
        target,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(3),
            compiled: false,
            checked: true,
            errors: vec!["(1, 1) Syntax error".into()],
        }),
    );
    let e = &editors.documents[0];
    assert!(!e.dirty());
    assert!(!e.closed);
    assert_eq!(e.errors, ["(1, 1) Syntax error"]);
    assert!(e.can_save());
}

#[test]
fn reopening_a_script_keeps_the_target_it_was_saved_with() {
    let mut editors = Editors::default();
    let slua = target(1);
    let request = editors.open(slua, "SLua".into(), true, true, 1).expect("load");
    editors.loaded(request, slua, Ok("ll.Say(0, 'hi')".into()));
    assert_eq!(editors.documents[0].compile, CompileTarget::Luau, "an SLua item opens as Lua");
    let lsl = target(3);
    let request = editors.open(lsl, "LSL".into(), true, true, 0).expect("load");
    editors.loaded(request, lsl, Ok("default {}".into()));
    editors.documents[1].compile = CompileTarget::Lsl2;
    let Some(Action::Save { request, options, .. }) = editors.documents[1].save() else {
        panic!("save action")
    };
    assert_eq!(options.compile, CompileTarget::Lsl2);
    editors.saved(
        request,
        lsl,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(4),
            compiled: true,
            checked: true,
            errors: vec![],
        }),
    );
    // raised, not created again: the item's flags do not reset the choice
    assert!(editors.open(lsl, "LSL".into(), true, true, 0).is_none());
    assert_eq!(editors.documents[1].compile, CompileTarget::Lsl2);
    assert!(!editors.documents[1].dirty());
    // an object's script: the running reply wins over the item
    let object = ScriptTarget {
        object: Uuid::from_u128(2),
        ..slua
    };
    editors.open(object, "LSL on Luau".into(), true, true, 0);
    assert_eq!(editors.documents[2].compile, CompileTarget::Mono);
    editors.running(object, true, Some(CompileTarget::LslLuau));
    editors.open(object, "LSL on Luau".into(), true, true, 0);
    assert_eq!(editors.documents[2].compile, CompileTarget::LslLuau);
}

#[test]
fn a_mono_line_does_not_move_lsl_on_lua_off_luau() {
    let target = target(80);
    let mut editors = Editors::default();
    let load = editors.open(target, "Main".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default {}".into()));
    let mut settings = settings::ScriptSettings {
        preprocessor: true,
        ..Default::default()
    };
    let e = &mut editors.documents[0];
    e.processor_settings = settings.clone();
    e.compile = CompileTarget::LslLuau;
    e.source = "//mono\ndefault { state_entry() { llOwnerSay(\"hi\"); } }".into();
    assert!(e.save().is_none(), "preprocessed first");
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut saved = None;
    while std::time::Instant::now() < until && saved.is_none() {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            actions = editors.show(
                ui.ctx(),
                &p,
                &mut settings,
                &mut crate::ui::color_picker::Preferences::default(),
                &[],
            )
        });
        output.textures_delta.clear();
        saved = actions.into_iter().find_map(|action| match action {
            Action::Save { options, .. } => Some(options.compile),
            _ => None,
        });
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(saved, Some(CompileTarget::LslLuau));
    assert_eq!(editors.documents[0].compile, CompileTarget::LslLuau);
}

#[test]
fn same_item_uuid_in_inventory_and_object_has_independent_editors_and_exit_guard() {
    let mut editors = Editors::default();
    let inventory = target(1);
    let object = ScriptTarget {
        object: Uuid::from_u128(2),
        ..inventory
    };
    for target in [inventory, object] {
        let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
        editors.loaded(request, target, Ok("original".into()));
    }
    editors.running(object, false, Some(CompileTarget::Lsl2));
    editors.documents[0].source = "inventory edit".into();
    editors.documents[1].source = "object edit".into();
    assert!(editors.open(inventory, "Again".into(), true, true, 0).is_none());
    assert_eq!(editors.documents.len(), 2);
    assert!(!editors.prepare_exit());
    assert!(editors.documents.iter().all(|e| e.close_prompt));
    let Action::Save { request, .. } = editors.documents[0].save().expect("save inventory") else {
        panic!("save action")
    };
    editors.saved(
        request,
        inventory,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(4),
            compiled: true,
            checked: true,
            errors: vec![],
        }),
    );
    assert!(!editors.documents[0].dirty());
    assert!(editors.documents[1].dirty());
    assert_eq!(editors.documents[1].compile, CompileTarget::Lsl2);
}

/// The scripts of one linkset: a root prim and a child, one script each,
/// as the app builds the tree.
/// The linkset fixture's two scripts open in one window, their text as
/// given, after the three frames their rows and docks measure themselves
/// over: the linkset and the two scripts, ready for more frames.
fn two_scripts_open(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    panel_source: &str,
    door_source: &str,
) -> (Linkset, ScriptTarget, ScriptTarget) {
    let (linkset, panel, door) = linkset_fixture();
    for (target, name, source) in [(panel, "Panel Script", panel_source), (door, "Door Script", door_source)] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok(source.into()));
        editors.documents.iter_mut().find(|e| e.target == target).unwrap().linkset = linkset.root;
    }
    for _ in 0..3 {
        frame_linksets(ctx, editors, settings, vec![], std::slice::from_ref(&linkset));
    }
    (linkset, panel, door)
}

/// A script of the tree's fixture, as the contents window lists one:
/// `flags` carries SST_LUA for a Lua script.
fn script_row(target: ScriptTarget, name: &str, flags: u32) -> LinksetScript {
    LinksetScript {
        object: target.object,
        item: target.item,
        name: name.into(),
        flags,
        running: Some(true),
        permissions: Permissions::new(perm::COPY | perm::MODIFY | perm::TRANSFER, 0),
        dirty: false,
        menu: Some(LinksetScriptMenu {
            moves: false,
            refused: None,
            tip: "Copy the script to the inventory",
            can_delete: true,
            can_control: true,
        }),
    }
}

fn linkset_fixture() -> (Linkset, ScriptTarget, ScriptTarget) {
    let panel = ScriptTarget {
        object: Uuid::from_u128(501),
        item: Uuid::from_u128(1),
    };
    let door = ScriptTarget {
        object: Uuid::from_u128(502),
        item: Uuid::from_u128(2),
    };
    let linkset = Linkset {
        root: Uuid::from_u128(500),
        name: "Linkset".into(),
        note: None,
        prims: vec![
            LinksetPrim {
                object: panel.object,
                kind: PrimKind::Prim,
                name: "Control Panel".into(),
                depth: 0,

                type_default: 0,
                loading: false,
                can_new_script: true,
                touch: true,
                menu: Some(LinksetPrimMenu {
                    take: true,
                    take_copy: true,
                    delete: true,
                }),
                scripts: vec![script_row(panel, "Panel Script", 0)],
            },
            LinksetPrim {
                object: door.object,
                kind: PrimKind::Prim,
                name: "Door".into(),
                depth: 0,

                type_default: 0,
                loading: false,
                can_new_script: true,
                touch: true,
                menu: None,
                scripts: vec![script_row(door, "Door Script", 0)],
            },
        ],
    };
    (linkset, panel, door)
}

/// Both scripts sit in one window, with the linkset's tree beside them;
/// choosing a row keeps every document and its text, and a row marks the
/// script whose text is unsaved.
#[test]
fn the_scripts_of_a_linkset_share_one_window_with_its_tree() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    let container_id = format!("script_linkset_{}", linkset.root);
    let container = ctx
        .memory(|m| m.area_rect(egui::Id::unique(&container_id)))
        .expect("the linkset's window");
    assert!(container.width() > 700.0, "wide enough for the tree and the source");
    // no script opened a window of its own
    assert!(ctx.memory(|m| m.area_rect(egui::Id::unique(window(panel)))).is_none());
    assert!(ctx.memory(|m| m.area_rect(egui::Id::unique(window(door)))).is_none());
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "the last opened is in front");
    assert!(
        editors
            .documents
            .iter()
            .all(|e| e.source == "panel source" || e.source == "door source")
    );

    // a click on the row of the first script brings its source to the front
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Panel Script");
    for pressed in [true, false] {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
    }
    assert_eq!(editors.active.get(&linkset.root), Some(&panel));
    assert!(ctx.memory(|m| m.has_focus(source_id(panel))));
    // both documents kept their text: nothing was lost in the switch
    assert_eq!(editors.documents[0].source, "panel source");
    assert_eq!(editors.documents[1].source, "door source");

    // the tree lists the linkset, a group per prim, even shut
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    for name in ["Filter scripts", "Control Panel", "Door", "Panel Script", "Door Script"] {
        assert!(
            output
                .shapes
                .iter()
                .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == name)),
            "{name} is drawn"
        );
    }
    // closing the tab in front keeps the other one open
    editors.documents[0].closed = true;
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.documents.len(), 1);
    assert!(editors.documents[0].is_open());
    assert_eq!(editors.active.get(&linkset.root), Some(&door));
}

/// The window keys of the script in front: Ctrl+Tab and Ctrl+Shift+Tab
/// cycle the tabs of its window, Ctrl+W closes the script in front,
/// straight away when it has nothing to save and through its prompt when it
/// has.
#[test]
fn the_window_keys_cycle_the_tabs_and_close_the_one_in_front() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    assert_eq!(editors.active.get(&linkset.root), Some(&door));
    assert!(ctx.memory(|m| m.has_focus(source_id(door))), "the script in front has the keyboard");
    assert_eq!(focus::active(&ctx), Some(source_id(door)), "the editor's commands know the script");

    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::Tab, egui::Modifiers::CTRL)],
        linksets,
    );
    assert_eq!(editors.active.get(&linkset.root), Some(&panel), "Ctrl+Tab goes to the next tab");
    assert!(ctx.memory(|m| m.has_focus(source_id(panel))));
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::Tab, egui::Modifiers::CTRL | egui::Modifiers::SHIFT)],
        linksets,
    );
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "Ctrl+Shift+Tab goes back");
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::Tab, egui::Modifiers::CTRL)],
        linksets,
    );
    // Ctrl+W on a script with nothing to save closes it, and the keyboard
    // goes to its neighbour
    assert_eq!(editors.active.get(&linkset.root), Some(&panel));
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::W, egui::Modifiers::CTRL)],
        linksets,
    );
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.documents.len(), 1);
    assert_eq!(editors.documents[0].target, door);
    assert!(ctx.memory(|m| m.has_focus(source_id(door))), "the neighbour takes the keyboard");

    // on unsaved text it asks first
    editors.documents[0].source = "edited".into();
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::W, egui::Modifiers::CTRL)],
        linksets,
    );
    assert!(editors.documents[0].close_prompt);
    assert!(editors.documents[0].is_open());
}

/// The bar says how much is selected, the engine the script compiles for
/// and whether it runs, beside the weight of its text.
#[test]
fn the_status_bar_says_what_is_selected_and_which_engine_runs_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel", "abcdef");
    let linksets = std::slice::from_ref(&linkset);
    editors.documents.iter_mut().find(|e| e.target == door).unwrap().running = Some(true);
    // three characters selected in the script in front
    let at = editing::line_cursor("abcdef", 0, 1);
    editing::select(&ctx, source_id(door), at..at + 3, true);
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the bar's labels carry their separators, so a word is found inside one
    let said = |needle: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text.contains(needle)))
    };
    let selection = aurora_i18n::tr!("script-selected-chars", count = 3);
    assert!(said(selection.as_ref()), "what a copy would take");
    assert!(said(aurora_i18n::tr!("script-state-running").as_ref()), "it runs");
    assert!(said(aurora_i18n::tr!("script-target-mono").as_ref()), "its engine");
}

/// The engine of a script is chosen from the bar's own word: a click opens
/// the menu there, and the choice reaches the script.
#[test]
fn the_engine_is_chosen_from_the_bar() {
    use aurora_net::scripts::CompileTarget;
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    // an object script chooses its engine once the region says it runs
    editors.documents.iter_mut().find(|e| e.target == door).unwrap().running = Some(true);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the bar's word carries the running state after the engine
    let name = aurora_i18n::tr!("script-target-mono").to_string();
    let engine = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.job.text.contains(&name) => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .expect("the bar's engine");
    click_at(&ctx, &mut editors, &mut settings, linksets, engine, egui::PointerButton::Primary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let lso = text_at(&output, aurora_i18n::tr!("script-target-lsl2").as_ref());
    click_at(&ctx, &mut editors, &mut settings, linksets, lso, egui::PointerButton::Primary);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(
        editors.documents.iter().find(|e| e.target == door).unwrap().compile,
        CompileTarget::Lsl2,
        "the engine chosen from the bar"
    );
}

/// Ctrl+1 to Ctrl+5 choose a tab of the dock, in the reader's order, and
/// unfold it.
/// Ctrl+Shift+S saves every script of the window, as the Save All button.
#[test]
fn ctrl_shift_s_asks_to_save_every_script_of_the_window() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let mut settings = settings::ScriptSettings::default();
    for (target, name) in [(panel, "Panel Script"), (door, "Door Script")] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("saved".into()));
        let e = editors.documents.iter_mut().find(|e| e.target == target).unwrap();
        e.linkset = linkset.root;
        e.source = "edited".into();
        // an object's script needs its reply and its experience to save
        e.running = Some(true);
        e.experience.selected = Some(Uuid::new_v4());
    }
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let (_, actions) = frame_linksets_actions(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::S, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT)],
        linksets,
    );
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::SaveAll { linkset: root, .. } if *root == linkset.root)),
        "Save All"
    );
}

/// F8 and Shift+F8 walk the diagnostics of the dock in its order, wrapping
/// at either end.
#[test]
fn f8_and_shift_f8_walk_the_diagnostics() {
    let rows = [(0usize, 3usize), (0, 10), (1, 5)];
    assert_eq!(view::problem_row(&rows, (0, 0), true), Some(0));
    assert_eq!(view::problem_row(&rows, (0, 3), true), Some(1));
    assert_eq!(view::problem_row(&rows, (0, 10), true), Some(2));
    assert_eq!(view::problem_row(&rows, (1, 5), true), Some(0), "wraps to the first");
    assert_eq!(view::problem_row(&rows, (0, 4), false), Some(0));
    assert_eq!(view::problem_row(&rows, (1, 5), false), Some(1));
    assert_eq!(view::problem_row(&rows, (0, 0), false), Some(2), "wraps to the last");
    assert_eq!(view::problem_row(&[], (0, 0), true), None);
}

/// The tab's menu closes the other scripts of the window, or all of them.
#[test]
fn the_tabs_menu_closes_the_other_scripts_or_all_of_them() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, _) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    // the tab of the panel's script (its tree row carries the letters)
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let close_others = text_at(&output, aurora_i18n::tr!("script-linkset-close-others"));
    click_at(
        &ctx,
        &mut editors,
        &mut settings,
        linksets,
        close_others,
        egui::PointerButton::Primary,
    );
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.documents.len(), 1, "the other script is closed");
    assert_eq!(editors.documents[0].target, panel);

    // and the last one, from its own menu
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let close_all = text_at(&output, aurora_i18n::tr!("script-linkset-close-all"));
    click_at(&ctx, &mut editors, &mut settings, linksets, close_all, egui::PointerButton::Primary);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(editors.documents.is_empty(), "every script is closed");
}

/// The dock's Outline tab lists the script's declarations, and a click
/// puts the caret on one; the window remembers the tab it shows.
/// The Problems dock folds to its header, and keeps its room: the source
/// takes the rest.
/// A diagnostic marks its line in the gutter, where the dock's row would
/// take the caret.
#[test]
fn a_diagnostic_marks_its_line_in_the_gutter() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(7),
        item: Uuid::from_u128(8),
    };
    let load = editors.open(target, "Broken".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default\n{\n    state_entry()\n    {\n    }\n}\n".into()));
    editors.documents[0].errors = vec!["(4, 5) : ERROR: syntax error".into()];
    settle(&ctx, &mut editors);
    let output = frame(&ctx, &mut editors, vec![]);
    let danger = crate::theme::Theme::default().palette().danger;
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Circle(circle) if circle.fill == danger)),
        "a mark for the error's line"
    );
    // and a squiggle under the word the error points at
    assert!(
        output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Path(path)
                    if path.stroke.color == egui::epaint::ColorMode::Solid(danger) && path.points.len() > 2)),
        "the error's span is underlined"
    );
}

/// The other occurrences of the word under the caret are lit behind the
/// text, the one the caret is in left alone.
#[test]
fn the_other_occurrences_of_the_word_under_the_caret_are_lit() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(13),
        item: Uuid::from_u128(14),
    };
    let load = editors.open(target, "Words".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("integer alpha = 0;\nalpha = alpha + 1;\n".into()));
    settle(&ctx, &mut editors);
    // the caret in the declaration's `alpha`
    editing::select(&ctx, source_id(target), 8..8, true);
    frame(&ctx, &mut editors, vec![]);
    let output = frame(&ctx, &mut editors, vec![]);
    let fill = crate::theme::Theme::default().palette().raised.gamma_multiply(0.38);
    let mut shapes = Vec::new();
    flatten_shapes(&output.shapes, &mut shapes);
    let boxes = shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::Rect(rect) if rect.fill == fill))
        .count();
    assert_eq!(boxes, 2, "the two other occurrences of alpha");
}

/// The tree says which scripts run, where the contents window shows its
/// C/M/T letters; its panel, its filter at the top, starts on the line of
/// the first tab and runs down beside the source.
#[test]
fn the_tree_shows_the_rights_under_tabs_that_span_the_window() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let texts: Vec<(&str, egui::Pos2)> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((t.galley.job.text.as_str(), t.pos)),
            _ => None,
        })
        .collect();
    // the highest text of a name: the tabs sit above the tree's rows
    let of = |text: &str| {
        texts
            .iter()
            .filter(|(drawn, _)| *drawn == text)
            .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .map(|(_, pos)| *pos)
    };
    // a running script says so at the right end of its row, in the tree
    let running = aurora_i18n::tr!("script-running").to_string();
    assert!(
        texts.iter().any(|(drawn, pos)| *drawn == running && pos.x < 560.0),
        "a running script says so: {texts:?}"
    );
    assert!(
        texts.iter().any(|(drawn, pos)| *drawn == "Door Script" && pos.x < 560.0),
        "the row's name: {texts:?}"
    );
    // the tabs run over the window, above the panel
    let first = of("Panel Script").expect("the first tab");
    let tab = of("Door Script").expect("the tab");
    let filter = of("Filter scripts").expect("the tree's filter");
    let prim = of("Control Panel").expect("the tree's first prim");
    let source = of("door source").expect("the source");
    assert!(
        (tab.y - filter.y).abs() <= 6.0,
        "the first tab and the tree's panel start on one line: {} and {}",
        tab.y,
        filter.y
    );
    assert!(
        first.x > filter.x + 150.0 && first.x < source.x,
        "the strip starts over the source, past the tree: {} and {}",
        first.x,
        filter.x
    );
    // the filter is the panel's first row, on the line of the first tab
    assert!(
        (filter.y - first.y).abs() <= 8.0,
        "the filter and the first tab start on one line: {} and {}",
        filter.y,
        first.y
    );
    assert!(
        prim.y > filter.y + 10.0,
        "the tree's rows come under its filter: {} and {}",
        prim.y,
        filter.y
    );
    // and the panel's rows start beside the source's first line
    assert!(
        (prim.y - source.y).abs() <= 26.0,
        "the tree's rows and the source start close together: {} and {}",
        prim.y,
        source.y
    );
}

/// The tree's panel and the status bar end on the same line: the window's
/// bottom, not a few points above it.
#[test]
fn the_trees_panel_reaches_the_status_bars_bottom() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let p = crate::theme::Theme::default().palette();
    let field = p.field.gamma_multiply(0.72);
    let mut tree = None;
    let mut bar = None;
    for shape in &output.shapes {
        let egui::Shape::Rect(rect) = &shape.shape else { continue };
        if rect.fill == field && rect.rect.width() > 100.0 && rect.rect.height() > 200.0 {
            tree = Some(rect.rect);
        }
        if rect.fill == p.bar {
            bar = Some(rect.rect);
        }
    }
    let tree = tree.expect("the tree's panel");
    let bar = bar.expect("the status bar");
    // the bar keeps the air of the columns between them above it
    let air = bar.top() - tree.bottom();
    assert!(
        air > 0.5 && air <= 12.0,
        "the tree ends at {} and the status bar starts at {}, {} apart",
        tree.bottom(),
        bar.top(),
        air
    );
}

/// The dock's tabs drag into another order, the last one too: the
/// References tab dropped on Problems takes its place, and no tab is lost on
/// the way.
/// The bar's first toggle is the window layout: the shared window, or one
/// per script as the preference keeps it.
#[test]
fn the_bar_flips_one_window_per_script() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the toggle stands before the readout: its own 18 points, the hairline
    // after them and the bar's five between each pair, so its middle is 26
    // points left of the readout's
    let front = editors.documents.iter().rev().find(|e| e.is_open()).expect("the script in front");
    let readout = aurora_i18n::tr!(
        "script-position",
        line = (front.tools.position.0 + front.first_line()) as i64,
        column = (front.tools.position.1 + front.first_line()) as i64
    )
    .to_string();
    let (left, y) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == readout => Some((text.pos.x, text.pos.y + text.galley.size().y * 0.5)),
            _ => None,
        })
        .expect("the bar's readout");
    let at = egui::pos2(left - 26.0, y);
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions.iter().any(|action| matches!(action, Action::BarChange(BarChange::Simple))),
        "the toggle asks the app for the other layout"
    );
}

/// The bar says when the script was last saved, and that it changed since.
#[test]
fn the_bar_says_when_the_script_was_saved() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let said = |output: &egui::FullOutput, needle: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text.contains(needle)))
    };
    // a script just loaded, never saved here: the bar says nothing of it
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(!said(&output, "Saved"), "nothing of a save it did not make");
    // an edit: unsaved
    let e = editors.documents.iter_mut().find(|e| e.target == door).unwrap();
    e.source.push_str("\n// edited\n");
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(said(&output, aurora_i18n::tr!("script-unsaved").as_ref()), "unsaved");
    // saved: the time of the save
    let e = editors.documents.iter_mut().find(|e| e.target == door).unwrap();
    e.baseline = e.source.clone();
    e.saved_at = Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000));
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let time = crate::ui::chat::hhmm(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000));
    assert!(
        said(&output, aurora_i18n::tr!("script-saved-at", time = time).as_ref()),
        "the time of the save"
    );
}

/// A script's row stands one step in under its prim's, where a prim
/// linked under that one puts its icons: the tree reads as objects
/// holding scripts, not as one flat list.
#[test]
fn a_scripts_row_stands_under_its_prim() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel", "door");
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], std::slice::from_ref(&linkset));
    // the prim's name is the tree's, the path under the source being
    // lower down; the script's is its tab's too, so its row is the one
    // "Running" stands on
    let prim = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Control Panel" => Some(text.pos),
            _ => None,
        })
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .expect("the prim's row")
        .x;
    let running = aurora_i18n::tr!("script-running").to_string();
    let badge = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == running => Some(text.pos.y),
            _ => None,
        })
        .min_by(f32::total_cmp)
        .expect("the row says it runs");
    let script = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Panel Script" && (text.pos.y - badge).abs() < 2.0 => Some(text.pos.x),
            _ => None,
        })
        .expect("the script's row");
    let step = script - prim;
    assert!(step > 8.0, "the script steps in under its prim ({step} points)");
    assert!(step < 24.0, "one step, not a column of its own ({step} points)");
}

/// The debug channel reaches the console too: a run-time error or a
/// warning of the object's script, as the chat shows it.
/// The old layout: with no shared window, each script of an object is in
/// a window of its own, as it was before the tree -- the window named after
/// the script alone, and no tab strip or tree beside it.
#[test]
fn the_simple_layout_gives_each_script_its_own_window() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let panel = ScriptTarget {
        object: Uuid::from_u128(501),
        item: Uuid::from_u128(1),
    };
    let door = ScriptTarget {
        object: Uuid::from_u128(502),
        item: Uuid::from_u128(2),
    };
    // as the app opens them with One window per script on: no linkset
    for (target, name) in [(panel, "Panel Script"), (door, "Door Script")] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok(format!("// {name}")));
    }
    let left = Editors::window_of(&editors.documents[0]);
    let right = Editors::window_of(&editors.documents[1]);
    assert_eq!(left, window(panel), "a window of its own, named after the script");
    assert_eq!(right, window(door));
    assert_ne!(left, right, "both scripts of the object, in two windows");
    let mut output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], &[]);
    for _ in 0..3 {
        output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], &[]);
    }
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    // each window shows its own script and names it in the title; a shared
    // window would have carried both in one tab strip
    for name in ["Panel Script", "Door Script"] {
        assert!(
            drawn(aurora_i18n::tr!("script-window-title", name = name).as_ref()),
            "{name}: its own window, titled after it"
        );
    }
}

/// The inventory's scripts share one window, with the folders that hold
/// them as its tree: two scripts of two folders are two tabs and two rows,
/// and the dock has no Console tab (no object says anything there).
#[test]
fn inventory_scripts_share_the_inventory_window() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let folder = Uuid::new_v4();
    let sword = ScriptTarget {
        object: Uuid::nil(),
        item: Uuid::from_u128(1),
    };
    let wrench = ScriptTarget {
        object: Uuid::nil(),
        item: Uuid::from_u128(2),
    };
    for (target, name) in [(sword, "Sword Script"), (wrench, "Wrench Script")] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("default { state_entry() { } }".into()));
        let document = editors.documents.iter_mut().find(|e| e.target == target).unwrap();
        document.linkset = INVENTORY_WINDOW;
    }
    assert_eq!(Editors::window_of(&editors.documents[0]), "script_inventory");
    assert_eq!(Editors::window_of(&editors.documents[1]), "script_inventory");
    let row = |target: ScriptTarget, name: &str| LinksetScript {
        object: Uuid::nil(),
        item: target.item,
        name: name.into(),
        flags: 0,
        running: None,
        permissions: Permissions::new(perm::COPY | perm::MODIFY | perm::TRANSFER, 0),
        dirty: false,
        menu: None,
    };
    let linkset = Linkset {
        root: INVENTORY_WINDOW,
        name: "Inventory".into(),
        note: None,
        prims: vec![LinksetPrim {
            object: folder,
            kind: PrimKind::Folder,
            name: "Weapons".into(),
            depth: 0,
            type_default: -1,
            loading: false,
            can_new_script: false,
            menu: None,
            touch: false,
            scripts: vec![row(sword, "Sword Script"), row(wrench, "Wrench Script")],
        }],
    };
    let mut output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], std::slice::from_ref(&linkset));
    for _ in 0..3 {
        output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], std::slice::from_ref(&linkset));
    }
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    let texts: Vec<&str> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    for label in ["Weapons", "Sword Script", "Wrench Script"] {
        assert!(drawn(label), "{label} is drawn; drawn: {texts:?}");
    }
}

/// The console's first look at the lines is the frame, not the first
/// line: a script's first `llOwnerSay` of a session is not taken for
/// history.
/// A line names the prim its script said it with, not the window: the
/// Console of a linkset's window takes every prim of it, even one no tab
/// holds, and nothing of an object no window shows.
/// The console's scope leads it from the whole object to the script in
/// front, as the other tabs' scopes do.
/// The console of a script's own window keeps its scope in the dock's header,
/// inside the window: the row it takes never pushes the tabs past the edge.
/// A very large script -- 128 KB, far past any ordinary one, the editor taking
/// up to a megabyte -- comes up and settles: from then on its frames cost what
/// the ones before them cost, nothing is read or laid out again for a text that
/// did not change, and the window keeps its size.
///
/// The check is a ratio between frames, not a clock: a shared PC's timings mean
/// little. The first frames are the expensive ones (they lay the whole text out
/// and read it once); bringing that first cost down is a task of its own
/// (TASKS.md).

#[test]
fn a_very_large_script_keeps_its_cost_steady() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let target = target(1);
    let mut source = String::with_capacity(140 * 1024);
    source.push_str("integer counter = 0;\n\n");
    let mut n = 0;
    while source.len() < 128 * 1024 {
        source.push_str(&format!(
            "integer helper_{n}(integer value)\n{{\n    return value + counter + {n};\n}}\n\n"
        ));
        n += 1;
    }
    source.push_str("default\n{\n    state_entry()\n    {\n        counter = 0;\n    }\n}\n");
    let load = editors.open(target, "Large script".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source.clone()));
    // the window opens, then the frames a writer spends in it: the first two
    // lay the text out and read it once, the rest are the steady state
    let window = egui::Id::unique(window(target));
    let mut times = Vec::new();
    for _ in 0..10 {
        let started = std::time::Instant::now();
        frame_settings(&ctx, &mut editors, &mut settings, vec![]);
        times.push(started.elapsed());
    }
    let opening: std::time::Duration = times[..2].iter().sum();
    let times = &times[2..];
    let opened = egui::AreaState::load(&ctx, window).and_then(|state| state.size);
    let key = editors.documents[0].tools.analysis.0;
    assert!(key.is_some(), "the text was read once");
    // the frames of a shared PC are noisy: the median says what one costs, and
    // a frame much past it would mean work made again
    let mut sorted = times.to_vec();
    sorted.sort();
    let typical = sorted[sorted.len() / 2];
    let worst = sorted[sorted.len() - 1];
    assert!(
        worst <= typical.mul_f64(8.0) + std::time::Duration::from_millis(100),
        "a frame of a {}-byte script costs {worst:?} against a usual {typical:?}: the text is not read or laid out again",
        source.len()
    );
    assert!(
        typical < std::time::Duration::from_millis(1500),
        "a frame of a {}-byte script usually costs {typical:?}",
        source.len()
    );
    // the opening frames read the text once and lay it out once: a pass that
    // ran over the whole script for every line made them cost far more than
    // the steady ones (the minimap's did, the square of the script's size)
    assert!(
        opening <= typical.mul_f64(12.0) + std::time::Duration::from_millis(300),
        "the first frames of a {}-byte script cost {opening:?} together, against a usual {typical:?}",
        source.len()
    );
    assert_eq!(
        egui::AreaState::load(&ctx, window).and_then(|state| state.size),
        opened,
        "the window keeps its size"
    );
    assert_eq!(
        editors.documents[0].tools.analysis.0, key,
        "its outline and lints are the text's own"
    );
    assert_eq!(editors.documents[0].source.len(), source.len(), "and the text is the one loaded");
}

/// The object-wide check reads a Lua script as Lua: the apostrophe of a
/// `--` comment is not a string left open, its `llSleep` is Lua's own lint
/// and none of LSL's.
/// A Lua pane offers no Source / Output switch, even with the preprocessor
/// on: the preprocessor is LSL's, so there is no second view to show.
#[test]
fn a_lua_pane_has_no_source_or_output_switch() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings {
        preprocessor: true,
        ..Default::default()
    };
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    editors.documents.iter_mut().find(|e| e.target == door).unwrap().compile = CompileTarget::Luau;
    let linksets = std::slice::from_ref(&linkset);
    // the door's Lua script is in front: no switch
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(
        !drawn(&output, aurora_i18n::tr!("script-tab-processed").as_ref()),
        "a Lua pane has no Output view"
    );
    // the panel's LSL script, with the preprocessor on, has the choice
    let index = editors.documents.iter().position(|e| e.target == panel).unwrap();
    editors.select_script(linkset.root, index);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(drawn(&output, aurora_i18n::tr!("script-tab-processed").as_ref()));
}

/// The linkset's window hangs Find / Replace in a popup under its menus and
/// toolbar, over the source, for the script that opened it, when the
/// setting keeps Find in the window: no row of the window is taken from
/// the text.
#[test]
fn the_linksets_window_hangs_find_under_its_toolbar() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings {
        search_inline: true,
        ..Default::default()
    };
    let (linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let window = format!("script_linkset_{}", linkset.root);
    let window_id = egui::Id::unique(&window);
    super::super::widgets::place_floater(&ctx, &window, egui::pos2(100.0, 100.0), Vec2::new(1200.0, 660.0));
    for _ in 0..4 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    // the text's place without the popup: no row is taken from it
    let alone = ctx.read_response(source_id(door)).expect("the source").rect;
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::F, egui::Modifiers::COMMAND)],
        linksets,
    );
    for _ in 0..4 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let search_id = egui::Id::unique(dialogs::Search::WINDOW);
    let response = ctx.read_response(search_id.with("query")).expect("the Find field");
    assert_eq!(
        response.layer_id.id,
        search_id.with("popup"),
        "the bar under the toolbar, not a panel beside the window"
    );
    let parent = ctx.memory(|m| m.area_rect(window_id)).expect("the window");
    let source = ctx.read_response(source_id(door)).expect("the source").rect;
    assert!(
        response.rect.top() > parent.top() && response.rect.top() < source.top(),
        "under the menus and the toolbar, above the sources: {:?} / {source:?}",
        response.rect
    );
    assert!(
        (source.top() - alone.top()).abs() < 1.5 && (source.height() - alone.height()).abs() < 1.5,
        "the bar takes no row from the text: {alone:?} / {source:?}"
    );
    // the script in front owns it: the panel's query draws on its text
    assert!(editors.panels.search.owns(source_id(door)));
}

/// A Lua script of a prim sits in the tree beside its LSL neighbour, at
/// its depth: neither writes its language -- its icon is its own, and a
/// running script says so where the contents window shows its rights.
#[test]
fn a_lua_script_sits_in_the_tree_beside_its_lsl_neighbour() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (mut linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    linkset.prims[1].scripts.push(script_row(
        ScriptTarget {
            object: door.object,
            item: Uuid::from_u128(91),
        },
        "Lua Script",
        1,
    ));
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let running = aurora_i18n::tr!("script-running").to_string();
    let lua = aurora_i18n::tr!("script-type-lua").to_string();
    let lsl = aurora_i18n::tr!("script-type-lsl").to_string();
    let row = |name: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.starts_with(name) => Some((text.galley.job.text.as_str(), text.pos.x)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the row of {name}"))
    };
    // both scripts of the door's prim at one depth, and neither says its
    // language: a running one says so, at the right end of its row
    let (lua_row, lua_left) = row("Lua Script");
    let (lsl_row, lsl_left) = row("Door Script");
    assert!((lua_left - lsl_left).abs() < 1.0, "both scripts of the door's prim at one depth");
    let badges = output
        .shapes
        .iter()
        .filter(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == running && t.pos.x < 560.0))
        .count();
    assert_eq!(badges, 3, "the panel's script and both of the door's say that they run");
    assert!(
        !lua_row.ends_with(&lua) && !lsl_row.ends_with(&lsl),
        "the language is its icon's, not the row's"
    );
}

/// The Console tab lists what the window's object said with `llOwnerSay`,
/// with the time it said it, and Clear empties it.
/// The dock's Search tab lists a word in every script of the window, and a
/// click opens that script on the line.
/// A click in the pane that is not in front brings its script to the
/// front (the tree and the status bar follow it), and the panes stay.
#[test]
fn a_click_in_the_other_pane_brings_it_to_the_front() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    editors.splits.insert(linkset.root, (door, panel));
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "the door's script in front");
    // a click in the right pane's source
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let (galley, pos) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "panel source" => Some((text.galley.clone(), text.pos)),
            _ => None,
        })
        .expect("the panel's source beside the door's");
    let at = pos + galley.pos_from_cursor(egui::text::CCursor::new(2)).center().to_vec2();
    for pressed in [true, false] {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
    }
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(
        editors.active.get(&linkset.root),
        Some(&panel),
        "the pane clicked is the one in front, for the tree and the status bar"
    );
    assert_eq!(
        editors.splits.get(&linkset.root),
        Some(&(door, panel)),
        "the panes stay where they are"
    );
}

/// Choosing a script that is not one of the split's two gives a single
/// pane again.
#[test]
fn choosing_another_script_leaves_a_single_pane() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let third = ScriptTarget {
        object: Uuid::from_u128(503),
        item: Uuid::from_u128(3),
    };
    let mut settings = settings::ScriptSettings::default();
    for (target, name, source) in [
        (panel, "Panel Script", "panel source"),
        (door, "Door Script", "door source"),
        (third, "Third Script", "third source"),
    ] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok(source.into()));
        editors.documents.iter_mut().find(|e| e.target == target).unwrap().linkset = linkset.root;
    }
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    editors.splits.insert(linkset.root, (door, panel));
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    // the third script, from the strip
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let tab = text_at(&output, "Third Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, tab, egui::PointerButton::Primary);
    assert_eq!(
        editors.splits.get(&linkset.root),
        None,
        "the split is gone as soon as the choice is made, not a frame later"
    );
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&third));
    assert_eq!(editors.splits.get(&linkset.root), None, "one script again");
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    assert!(drawn("third source"), "its own source");
    assert!(!drawn("panel source") && !drawn("door source"), "and nothing beside it");
}

/// Ctrl+\ gives the script in front a pane beside it, and Ctrl+\ again
/// one script at a time.
#[test]
fn ctrl_backslash_splits_the_window_and_joins_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let front = *editors.active.get(&linkset.root).expect("a script in front");
    let press = || vec![key(egui::Key::Backslash, egui::Modifiers::CTRL)];
    frame_linksets(&ctx, &mut editors, &mut settings, press(), linksets);
    let split = editors.splits.get(&linkset.root).copied().expect("a split");
    assert_eq!(split.0, front, "the script in front keeps the left pane");
    assert_eq!(split.1, if front == panel { door } else { panel }, "the other beside it");
    frame_linksets(&ctx, &mut editors, &mut settings, press(), linksets);
    assert_eq!(editors.splits.get(&linkset.root), None, "one script again");
}

/// The split's panes share its width, and neither may be dragged away.
#[test]
fn the_splits_panes_share_the_width_and_keep_a_readable_least() {
    assert_eq!(view::split_widths(1000.0, 0.5), (496.0, 496.0));
    assert_eq!(view::split_widths(1000.0, 0.0), (160.0, 832.0));
    assert_eq!(view::split_widths(1000.0, 1.0), (832.0, 160.0));
    assert_eq!(view::split_widths(300.0, 0.5), (146.0, 146.0));
}

/// The bar says what the region reports of the object's scripts: their
/// memory and how many of them run.
#[test]
fn the_bar_says_the_objects_script_figure() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    editors.script_stats(&aurora_net::bridge::query::ScriptInfo {
        object: linkset.root,
        name: "Panel Script".into(),
        running: 2,
        total: 3,
        memory: 3 * 65_536,
        cpu_seconds: 0.0,
        character_seconds: 0.0,
    });
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the bar says the memory and the running count, in two cells
    let memory = aurora_i18n::tr!("script-bar-scripts", memory = "192.0 KB").to_string();
    let running = aurora_i18n::tr!("script-bar-scripts-running", running = 2, total = 3).to_string();
    assert!(drawn(&output, &memory), "the object's memory");
    assert!(drawn(&output, &running), "and how many of its scripts run");
}

/// The split's tab shows the two names in the order of the panes whichever
/// one is in front, each once: focusing the right pane used to draw its
/// name twice and the other script as a tab of its own.
#[test]
fn the_split_tab_keeps_the_panes_order_whichever_is_in_front() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    editors.splits.insert(linkset.root, (door, panel));
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    // a click in the right pane's source focuses it
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let (galley, pos) = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "panel source" => Some((text.galley.clone(), text.pos)),
            _ => None,
        })
        .expect("the panel's source beside the door's");
    let at = pos + galley.pos_from_cursor(egui::text::CCursor::new(2)).center().to_vec2();
    for pressed in [true, false] {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&panel), "the right pane in front");
    // each name once, in the panes' order, the one in front in ink; the
    // strip is the topmost row that names a script (the crumb bar under
    // the source names it too)
    let palette = crate::theme::Theme::default().palette();
    let mut names: Vec<(String, egui::Pos2, Option<egui::Color32>)> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "Door Script" || text.galley.job.text == "Panel Script" => Some((
                text.galley.job.text.clone(),
                text.pos,
                text.galley.job.sections.first().map(|section| section.format.color),
            )),
            _ => None,
        })
        .collect();
    let top = names.iter().map(|name| name.1.y).fold(f32::MAX, f32::min);
    names.retain(|name| (name.1.y - top).abs() < 1.0);
    assert_eq!(names.len(), 2, "the two names of the split, once each: {names:?}");
    let door_name = names.iter().find(|(label, ..)| label == "Door Script").unwrap();
    let panel_name = names.iter().find(|(label, ..)| label == "Panel Script").unwrap();
    assert!(door_name.1.x < panel_name.1.x, "the left pane's name first");
    assert_eq!(door_name.2, Some(palette.muted), "the pane not in front is dimmed");
    assert_eq!(panel_name.2, Some(palette.ink), "the pane in front is in ink");
    // its name in the strip brings the left pane back without moving either
    let door_at = text_at(&output, "Door Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, door_at, egui::PointerButton::Primary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&door));
    assert_eq!(editors.splits.get(&linkset.root), Some(&(door, panel)), "the panes stay");
    assert!(
        text_at(&output, "Door Script").x < text_at(&output, "Panel Script").x,
        "and so does their order in the strip"
    );
}

/// The notice over a script's source says what it cannot do, and the
/// reader can put it away; another one comes back on its own.
#[test]
fn the_notice_over_the_source_says_what_the_script_cannot_do() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(7);
    let load = editors.open(target, "Locked".into(), true, false, 0).unwrap();
    editors.loaded(load, target, Ok("default\n{\n}\n".into()));
    let drawn = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == text))
    };
    let read_only = aurora_i18n::tr!("script-read-only");
    settle(&ctx, &mut editors);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(drawn(&output, read_only), "a read-only script says so over its source");
    // put away, it stays away
    editors.documents[0].tools.notice_hidden = Some(view::Notice::ReadOnly);
    frame(&ctx, &mut editors, vec![]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(!drawn(&output, read_only), "the reader hid it");
    // a different one is not hidden with it; the save's word is in the
    // console, not in `status`, and the failure stays readable until the
    // reader hides it or a load or a save settles it
    editors.documents[0].can_edit = true;
    editors.documents[0].compile_failed = true;
    let failed = aurora_i18n::tr!("script-compile-failed");
    frame(&ctx, &mut editors, vec![]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(drawn(&output, failed), "the failure takes the notice's place");
    assert!(!drawn(&output, read_only));
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(drawn(&output, failed), "it stays over the source, not a flash");
    // a save that compiles takes it away
    editors.documents[0].compile_failed = false;
    frame(&ctx, &mut editors, vec![]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(!drawn(&output, failed), "a successful compile clears it");
}

/// A Lua script that writes `#define` or `#include` says over its source
/// that the preprocessor is LSL's, where an LSL script says nothing.
#[test]
fn a_lua_script_with_a_directive_says_the_preprocessor_is_lsls() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(9);
    let load = editors.open(target, "Lua with a macro".into(), true, true, 1).unwrap();
    editors.loaded(
        load,
        target,
        Ok("#define SPEED 2\nlocal function run()\n    ll.OwnerSay(SPEED)\nend\nrun()\n".into()),
    );
    let drawn = |output: &egui::FullOutput, text: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == text))
    };
    let notice = aurora_i18n::tr!("script-notice-lsl-only");
    settle(&ctx, &mut editors);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(drawn(&output, notice), "the hint over a Lua script");
    // put away, it stays away
    editors.documents[0].tools.notice_hidden = Some(view::Notice::LslOnly);
    frame(&ctx, &mut editors, vec![]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(!drawn(&output, notice), "the reader hid it");
    // an LSL script with the same lines keeps the preprocessor it has
    let mut lsl = Editors::default();
    let load = lsl.open(target, "LSL with a macro".into(), true, true, 0).unwrap();
    lsl.loaded(
        load,
        target,
        Ok("#define SPEED 2\ndefault { state_entry() { llSay(0, (string)SPEED); } }\n".into()),
    );
    settle(&ctx, &mut lsl);
    let output = frame(&ctx, &mut lsl, vec![]);
    assert!(!drawn(&output, notice), "no hint on LSL");
}

/// The bar under the source names the blocks the caret is in, and each
/// step goes back to its line.
#[test]
fn the_crumbs_under_the_source_name_the_blocks_and_go_to_them() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let _ = panel;
    let mut settings = settings::ScriptSettings::default();
    let source = "default\n{\n    state_entry()\n    {\n        llSay(0, \"hi\");\n    }\n}\n";
    let load = editors.open(door, "Door Script".into(), true, true, 0).unwrap();
    editors.loaded(load, door, Ok(source.into()));
    editors.documents.iter_mut().find(|e| e.target == door).unwrap().linkset = linkset.root;
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    // the caret inside the event's body
    let at = editing::line_cursor(source, 4, 8);
    editing::select(&ctx, source_id(door), at..at, true);
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let drawn = |text: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == text))
    };
    // the bar opens on the prim that holds the script
    assert_eq!(editors.documents.iter().find(|e| e.target == door).unwrap().tools.prim, "Door");
    assert!(drawn("Door Script"), "the script the bar belongs to");
    assert!(drawn("default"), "the state");
    assert!(drawn("state_entry()"), "the event");
    // a step goes to its line: the lowest `state_entry()` is the bar's (the
    // source has one of its own, above it)
    let at = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(t) if t.galley.job.text == "state_entry()" => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .max_by(|a, b| a.y.total_cmp(&b.y))
        .expect("the step");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(
        editors.documents.iter().find(|e| e.target == door).unwrap().tools.position.0,
        2,
        "the caret is on the event's line"
    );
}

/// What is typed under the caret is completed: the catalog's names are
/// offered, Enter takes the one typed whole and a click takes another, both
/// with their bracket and the caret inside it.
#[test]
fn typing_a_name_offers_the_catalog_and_enter_writes_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(21);
    let source = "default\n{\n    state_entry()\n    {\n    }\n}\n";
    let load = editors.open(target, "Complete".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source.into()));
    settle(&ctx, &mut editors);
    // the caret inside the event's body
    let at = editing::line_cursor(source, 4, 0);
    editing::select(&ctx, source_id(target), at..at, true);
    frame(&ctx, &mut editors, vec![]);
    // a name typed whole is the first of the list: Enter takes it
    frame(&ctx, &mut editors, vec![egui::Event::Text("llSay".into())]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "llSay")),
        "the list offers the catalog's names"
    );
    frame(&ctx, &mut editors, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    frame(&ctx, &mut editors, vec![]);
    let source = &editors.documents[0].source;
    assert!(source.contains("llSay("), "{source}");
    let caret = editing::cursor(&ctx, source_id(target)).primary.index.0;
    assert_eq!(source.chars().nth(caret - 1), Some('('), "the caret is inside the call");
    // another name, taken with the mouse under the list
    frame(&ctx, &mut editors, vec![egui::Event::Text("0, llSetT".into())]);
    let output = frame(&ctx, &mut editors, vec![]);
    let row = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == "llSetText" => Some(text.pos + text.galley.size() * 0.5),
            _ => None,
        })
        .expect("the row of llSetText");
    click_at(
        &ctx,
        &mut editors,
        &mut settings::ScriptSettings::default(),
        &[],
        row,
        egui::PointerButton::Primary,
    );
    frame(&ctx, &mut editors, vec![]);
    assert!(
        editors.documents[0].source.contains("llSetText("),
        "{}",
        editors.documents[0].source
    );
}

/// A snippet writes its body with its placeholders, the first selected, and
/// Tab walks them until the last, which ends the filling.
#[test]
fn a_snippet_writes_its_body_and_tab_walks_its_placeholders() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(23);
    let source = "default\n{\n    state_entry()\n    {\n    }\n}\n";
    let load = editors.open(target, "Snippet".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source.into()));
    settle(&ctx, &mut editors);
    // the caret at the start of the empty line of the event's body
    let at = editing::line_cursor(source, 4, 0);
    editing::select(&ctx, source_id(target), at..at, true);
    frame(&ctx, &mut editors, vec![]);
    frame(&ctx, &mut editors, vec![egui::Event::Text("if".into())]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "if")),
        "the list offers the snippet by its name"
    );
    frame(&ctx, &mut editors, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    frame(&ctx, &mut editors, vec![]);
    let written = &editors.documents[0].source;
    assert!(written.contains("if (condition)"), "{written}");
    assert!(written.contains("{\n    \n}"), "its body, at its own indentation: {written}");
    // the first placeholder is selected, ready to be replaced
    let range = editing::bytes(written, editing::cursor(&ctx, source_id(target)));
    assert_eq!(&written[range], "condition");
    // Tab takes the empty one inside the braces
    frame(&ctx, &mut editors, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    frame(&ctx, &mut editors, vec![]);
    let written = &editors.documents[0].source;
    let caret = editing::cursor(&ctx, source_id(target)).primary.index.0;
    assert!(editing::cursor(&ctx, source_id(target)).is_empty(), "an empty placeholder");
    let (line, _) = editing::position(written, caret);
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(lines[line].trim(), "", "inside the body: {written}");
    assert_eq!(lines[line - 1].trim(), "{", "under the brace: {written}");
    assert!(editors.documents[0].tools.snippet.is_some(), "still filling it");
    // past the last placeholder, the filling is done
    frame(&ctx, &mut editors, vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
    frame(&ctx, &mut editors, vec![]);
    assert!(editors.documents[0].tools.snippet.is_none(), "the filling ends");
}

/// Escape closes the list until the name changes, and a name inside a
/// comment is not completed at all.
#[test]
fn escape_closes_the_list_and_a_comment_is_not_completed() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(22);
    let source = "default\n{\n    state_entry()\n    {\n        // llS\n    }\n}\n";
    let load = editors.open(target, "Complete".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source.into()));
    settle(&ctx, &mut editors);
    let at = editing::line_cursor(source, 4, 11);
    editing::select(&ctx, source_id(target), at..at, true);
    frame(&ctx, &mut editors, vec![]);
    frame(&ctx, &mut editors, vec![egui::Event::Text("a".into())]);
    let output = frame(&ctx, &mut editors, vec![]);
    let listed = |output: &egui::FullOutput| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("llSa")))
    };
    assert!(!listed(&output), "a name in a comment is not completed");
    // a real name: the list shows, Escape closes it, the text stays
    let at = editing::line_cursor(source, 4, 8);
    editing::select(&ctx, source_id(target), at..at, true);
    frame(&ctx, &mut editors, vec![]);
    frame(&ctx, &mut editors, vec![egui::Event::Text("llS".into())]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(listed(&output), "a name in the code is");
    frame(&ctx, &mut editors, vec![key(egui::Key::Escape, egui::Modifiers::NONE)]);
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(!listed(&output), "Escape closes it");
    assert!(editors.documents[0].source.contains("llS"), "and the letters typed stay");
}

/// Ctrl and a click on a name takes the window to where the name is
/// declared, in another script of the linkset.
#[test]
fn ctrl_click_goes_to_the_declaration_in_another_script() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let mut settings = settings::ScriptSettings::default();
    let panel_source = "default\n{\n    state_entry()\n    {\n        helper(1);\n    }\n}\n";
    for (target, name, source) in [
        (panel, "Panel Script", panel_source),
        (door, "Door Script", "integer helper(integer n)\n{\n    return n;\n}\n"),
    ] {
        let load = editors.open(target, name.into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok(source.into()));
        editors.documents.iter_mut().find(|e| e.target == target).unwrap().linkset = linkset.root;
    }
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    // the panel's script in front, its source on screen
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let row = row_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, row, egui::PointerButton::Primary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&panel));
    // the place of `helper` in it
    let source = panel_source;
    let found = source.find("helper").expect("helper is in the source");
    let (galley, pos) = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(text) if text.galley.job.text == source => Some((text.galley.clone(), text.pos)),
            _ => None,
        })
        .expect("the panel's source on screen");
    let at = pos + galley.pos_from_cursor(egui::text::CCursor::new(found)).center().to_vec2();
    for pressed in [true, false] {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::CTRL),
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::CTRL,
                },
            ],
            linksets,
        );
    }
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "the door declares it");
    assert_eq!(
        editors.documents.iter().find(|e| e.target == door).unwrap().tools.position.0,
        0,
        "the caret is on the declaration"
    );
}

/// The sticky line above the source names the state and the event the top
/// of the viewport is in.
#[test]
fn the_sticky_line_names_the_event_holding_the_top() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(19),
        item: Uuid::from_u128(20),
    };
    let source = "default\n{\n    state_entry()\n    {\n        llSay(0, \"hi\");\n    }\n}\n";
    let load = editors.open(target, "Sticky".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source.into()));
    settle(&ctx, &mut editors);
    // the top of the viewport inside the event
    editors.documents[0].tools.first_visible = 3;
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "default \u{203a} state_entry")),
        "the state and the event above the text"
    );
    // at the very top of the state, the state alone
    editors.documents[0].tools.first_visible = 0;
    let output = frame(&ctx, &mut editors, vec![]);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "default")),
        "the state alone"
    );
}

/// F2 and Shift+F2 walk the reader's bookmarks, wrapping at either end.
#[test]
fn the_bookmarks_walk_in_both_directions() {
    let lines: std::collections::BTreeSet<usize> = [3, 10].into_iter().collect();
    assert_eq!(view::bookmark_step(&lines, 0, true), Some(3));
    assert_eq!(view::bookmark_step(&lines, 3, true), Some(10));
    assert_eq!(view::bookmark_step(&lines, 10, true), Some(3), "wraps to the first");
    assert_eq!(view::bookmark_step(&lines, 0, false), Some(10), "wraps to the last");
    assert_eq!(view::bookmark_step(&lines, 10, false), Some(3));
    assert_eq!(view::bookmark_step(&lines, 5, true), Some(10));
    assert_eq!(view::bookmark_step(&std::collections::BTreeSet::new(), 0, true), None);
}

/// Ctrl+F2 marks the caret's line (or unmarks it), the gutter shows a mark,
/// and F2 walks to it.
#[test]
fn ctrl_f2_marks_the_line_and_f2_goes_to_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(17),
        item: Uuid::from_u128(18),
    };
    let source: String = (0..6).map(|line| format!("line {line}\n")).collect();
    let load = editors.open(target, "Marked".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source));
    settle(&ctx, &mut editors);
    // the caret on the fourth line
    editing::select(&ctx, source_id(target), 0..0, true);
    for _ in 0..2 {
        frame(&ctx, &mut editors, vec![]);
    }
    editors.documents[0].tools.position = (3, 0);
    frame(&ctx, &mut editors, vec![key(egui::Key::F2, egui::Modifiers::CTRL)]);
    assert!(editors.documents[0].bookmarks.contains(&3), "the caret's line is marked");
    // a mark in the gutter, and one in the overview strip
    let output = frame(&ctx, &mut editors, vec![]);
    let indigo = crate::theme::Theme::default().palette().indigo_light;
    let mut shapes = Vec::new();
    flatten_shapes(&output.shapes, &mut shapes);
    assert!(
        shapes
            .iter()
            .any(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill == indigo)),
        "the bookmark's mark"
    );
    // the caret goes back to the start, then F2 finds the mark
    editing::select(&ctx, source_id(target), 0..0, true);
    for _ in 0..2 {
        frame(&ctx, &mut editors, vec![]);
    }
    frame(&ctx, &mut editors, vec![key(egui::Key::F2, egui::Modifiers::NONE)]);
    frame(&ctx, &mut editors, vec![]);
    assert_eq!(editors.documents[0].tools.position.0, 3, "F2 goes to the bookmark");
    // and Ctrl+F2 takes the mark off
    frame(&ctx, &mut editors, vec![key(egui::Key::F2, egui::Modifiers::CTRL)]);
    assert!(!editors.documents[0].bookmarks.contains(&3));
}

/// A script opened beside another is drawn in a second pane, and choosing
/// it from the strip brings it to the front, the other taking its place.
#[test]
fn a_script_opened_beside_another_is_drawn_in_its_own_pane() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    let linksets = std::slice::from_ref(&linkset);
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "the door's script in front");
    let before = ctx
        .memory(|m| m.area_rect(egui::Id::unique(format!("script_linkset_{}", linkset.root))))
        .expect("the window")
        .width();
    editors.splits.insert(linkset.root, (door, panel));
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let after = ctx
        .memory(|m| m.area_rect(egui::Id::unique(format!("script_linkset_{}", linkset.root))))
        .expect("the window")
        .width();
    assert!(
        (after - before).abs() < 1.0,
        "opening the split keeps the window's width: {before} -> {after}"
    );
    let text_pos = |label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} on screen"))
    };
    let left = text_pos("door source");
    let right = text_pos("panel source");
    assert!(right.x > left.x + 100.0, "a pane each, side by side: {left:?} {right:?}");
    assert!((right.y - left.y).abs() < 4.0, "their tops are level");
    // one tab holds both names, a separator between them, and the front
    // script's name comes first (documents order would put the panel first)
    let label_rect = |label: &str| -> egui::Rect {
        // the tab strip is above the tree's rows, which carry the names too
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                _ => None,
            })
            .min_by(|a, b| a.pos.y.total_cmp(&b.pos.y))
            .unwrap_or_else(|| panic!("the {label} tab"));
        egui::Rect::from_min_size(text.pos, text.galley.size())
    };
    let front = label_rect("Door Script");
    let beside = label_rect("Panel Script");
    assert!(front.left() < beside.left(), "the front script's name comes first");
    let background = settings.color(settings::ColorRole::Background, &crate::theme::Theme::default().palette());
    let shared = output.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Rect(rect) => rect.fill == background && rect.rect.contains_rect(front) && rect.rect.contains_rect(beside),
        _ => false,
    });
    assert!(shared, "the two names share one tab");
    // and opening it does not widen the window
    let window = ctx
        .memory(|m| m.area_rect(egui::Id::unique(format!("script_linkset_{}", linkset.root))))
        .expect("the window");
    // the default window is 960 wide, and the documentation pane's room
    // (288 and the air beside it) opens it at 1272: no wider than that
    assert!(window.width() <= 1273.0, "the split keeps the window's width: {window:?}");
    // choosing the script shown beside brings it to the front, the other
    // taking its place beside
    let tab = text_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, tab, egui::PointerButton::Primary);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&panel));
    assert_eq!(editors.splits.get(&linkset.root), Some(&(door, panel)));
}

/// The overview strip maps the whole script: a click near its bottom puts
/// the caret on a line near the end, and the strip stays in the window
/// while the source scrolls.
#[test]
fn a_click_on_the_overview_strip_goes_to_a_line_near_the_end() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = ScriptTarget {
        object: Uuid::from_u128(9),
        item: Uuid::from_u128(10),
    };
    let source: String = (0..80).map(|line| format!("// line {line}\n")).collect();
    let load = editors.open(target, "Long".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source));
    settle(&ctx, &mut editors);
    let strip = editors.documents[0].tools.overview.expect("the overview strip");
    assert!(strip.width() >= 5.0 && strip.height() > 40.0, "{strip:?}");
    let at = egui::pos2(strip.center().x, strip.bottom() - 2.0);
    for pressed in [true, false] {
        frame(
            &ctx,
            &mut editors,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    frame(&ctx, &mut editors, vec![]);
    let line = editors.documents[0].tools.position.0;
    assert!(line >= 70, "the caret is near the end, not on line {line}");
}

/// The minimap takes the strip's place: a column wide enough for the
/// script's bars, a click on it near the end goes there, a drag moves the
/// view without moving the caret, and the setting gives the thin strip back.
#[test]
fn the_minimap_shows_the_script_and_the_setting_gives_the_strip_back() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(31);
    let source: String = (0..80).map(|line| format!("    llSay(0, \"line {line}\");\n")).collect();
    let load = editors.open(target, "Map".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok(source));
    settle(&ctx, &mut editors);
    let map = editors.documents[0].tools.overview.expect("the rail");
    assert!(map.width() > 40.0 && map.height() > 40.0, "the minimap: {map:?}");
    // a drag through the map goes where the pointer is
    let start = egui::pos2(map.center().x, map.top() + map.height() * 0.1);
    let end = egui::pos2(map.center().x, map.top() + map.height() * 0.9);
    for events in [
        vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        vec![egui::Event::PointerMoved(end)],
        vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        frame(&ctx, &mut editors, events);
    }
    frame(&ctx, &mut editors, vec![]);
    let line = editors.documents[0].tools.position.0;
    assert!(line > 60, "the caret went where the pointer did: line {line}");
    // off, the thin overview strip comes back to the rail
    let mut settings = settings::ScriptSettings {
        minimap: false,
        ..Default::default()
    };
    settle(&ctx, &mut editors);
    frame_settings(&ctx, &mut editors, &mut settings, vec![]);
    let strip = editors.documents[0].tools.overview.expect("the rail");
    assert!(strip.width() < 12.0, "the overview strip: {strip:?}");
}

/// The tree's edge gives it more room, and a double click takes the default
/// back (the inventory tree's own side, reused).
#[test]
fn the_trees_edge_resizes_it_and_a_double_click_restores_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let panel = |output: &egui::FullOutput| {
        let p = crate::theme::Theme::default().palette();
        let field = p.field.gamma_multiply(0.72);
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == field && rect.rect.width() > 100.0 => Some(rect.rect),
                _ => None,
            })
            .expect("the tree's panel")
    };
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let tree = panel(&output);
    let before = tree.width();
    // the strip lies in the middle of the room between the two panels
    let handle = egui::pos2(tree.right() + 8.0, tree.center().y);
    let wider = egui::pos2(tree.right() + 44.0, tree.center().y);
    for events in [
        vec![
            egui::Event::PointerMoved(handle),
            egui::Event::PointerButton {
                pos: handle,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        vec![egui::Event::PointerMoved(wider)],
        vec![egui::Event::PointerButton {
            pos: wider,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        frame_linksets(&ctx, &mut editors, &mut settings, events, linksets);
    }
    assert!(settings.tree_width > 0, "the reader's width is kept");
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(panel(&output).width() > before, "the list is wider");
    // a double click gives the default back
    let tree = panel(&output);
    let at = egui::pos2(tree.right() + 4.0, tree.center().y);
    for pressed in [true, false, true, false] {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
    }
    assert_eq!(settings.tree_width, 0, "the default again");
}

/// The foot of the tree opens the script editor's preferences, as the
/// inventory tree's foot opens its Trash.
#[test]
fn the_trees_foot_opens_the_script_preferences() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let row = text_at(&output, aurora_i18n::tr!("script-tree-settings").as_ref());
    click_at(&ctx, &mut editors, &mut settings, linksets, row, egui::PointerButton::Primary);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(editors.panels.preferences.open, "the preferences open");
}

/// A script chosen from its tab shows in the tree too: the prim that holds
/// it opens, where the reader had shut it.
#[test]
fn choosing_a_script_opens_the_prim_that_holds_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    // the panel's script in front, then the door's prim shut by the reader
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let tab = text_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, tab, egui::PointerButton::Primary);
    ctx.data_mut(|data| {
        data.insert_persisted(linkset::collapsed_id(linkset.root), vec![door.object]);
    });
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let running = aurora_i18n::tr!("script-running").to_string();
    let row_drawn = |output: &egui::FullOutput| {
        let badges: Vec<f32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == running && text.pos.x < 560.0 => Some(text.pos.y),
                _ => None,
            })
            .collect();
        output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text)
                    if text.galley.job.text == "Door Script" && badges.iter().any(|y| (y - text.pos.y).abs() < 2.0))
        })
    };
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(!row_drawn(&output), "the shut prim keeps its scripts to itself");
    // its script, from the tab
    let tab = text_at(&output, "Door Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, tab, egui::PointerButton::Primary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.active.get(&linkset.root), Some(&door));
    assert!(row_drawn(&output), "the prim opened to show the script chosen");
    // and stays opened afterwards
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(row_drawn(&output), "the reader's shut group is open now");
}

/// A tab dragged over another takes its place: before it when it comes
/// from its right, after it when it comes from its left.
#[test]
fn a_script_tab_dragged_over_another_takes_its_place() {
    let (a, b, c) = (target(61), target(62), target(63));
    let mut order = vec![a, b, c];
    reorder_targets(&mut order, c, a);
    assert_eq!(order, vec![c, a, b], "from the right, before it");
    reorder_targets(&mut order, c, b);
    assert_eq!(order, vec![a, b, c], "from the left, after it");
    reorder_targets(&mut order, a, a);
    assert_eq!(order, vec![a, b, c], "dropped on itself");
}

/// Help › Shortcuts opens a window that tells what each key does in a
/// script's window, in groups.
#[test]
fn the_shortcuts_window_lists_what_the_keys_do() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let target = target(41);
    let load = editors.open(target, "Keys".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default { }".into()));
    settle(&ctx, &mut editors);
    // the Help menu's item: the window, open (a floater lays itself out on
    // the frame that opens it)
    editors.panels.shortcuts = true;
    let mut output = frame(&ctx, &mut editors, vec![]);
    for _ in 0..2 {
        output = frame(&ctx, &mut editors, vec![]);
    }
    let drawn = |text: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(galley) if galley.galley.job.text == text))
    };
    assert!(drawn(aurora_i18n::tr!("script-help-shortcuts").as_ref()), "its title");
    assert!(drawn(aurora_i18n::tr!("script-shortcuts-files").as_ref()), "the first group");
    assert!(drawn(aurora_i18n::tr!("script-shortcuts-dock-tabs").as_ref()), "a row of it");
    assert!(
        drawn(aurora_i18n::tr!("script-shortcuts-definition").as_ref()),
        "a row of the second group"
    );
    assert!(
        drawn(aurora_i18n::tr!("script-shortcuts-code").as_ref()) && drawn(aurora_i18n::tr!("script-shortcuts-dismiss").as_ref()),
        "the third group and its last row, the window tall enough for all of them"
    );
}

/// The tree of a linkset with a prim under another: a group starts one
/// step deeper than the prim that holds it, its scripts with it.
#[test]
fn the_tree_indents_a_prim_under_the_one_that_holds_it() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (mut linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    // the door hangs from the panel, and a handle from the door
    linkset.prims[1].depth = 1;
    let handle = ScriptTarget {
        object: Uuid::from_u128(77),
        item: Uuid::from_u128(78),
    };
    linkset.prims.push(LinksetPrim {
        object: handle.object,
        kind: PrimKind::Prim,
        name: "Handle".into(),
        depth: 2,
        type_default: 0,
        loading: false,
        can_new_script: true,
        menu: None,
        touch: true,
        scripts: Vec::new(),
    });
    // a prim with no script in front, so the tree foot's header does not
    // repeat a script's name above its row: this measures the rows alone
    editors.active.insert(linkset.root, handle);
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the left edge of a row's words, where the indent shows (a longer
    // name has a longer centre, not a further left edge)
    let left_of = |label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos.x),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} is drawn"))
    };
    // the name's row in the tree, left of the tabs above the source; its
    // `Running` badge is a word of its own at the row's right end
    let row_left = |name: &str| {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == name => Some(t.pos.x),
                _ => None,
            })
            .min_by(f32::total_cmp)
            .unwrap_or_else(|| panic!("the row of {name}"))
    };
    let top = left_of("Control Panel");
    let under = left_of("Door");
    let deepest = left_of("Handle");
    assert!(under > top + 10.0, "the door under the panel: {under} against {top}");
    assert!(deepest > under + 10.0, "the handle under the door: {deepest} against {under}");
    // a script's row keeps the step its prim takes
    let panel_script = row_left("Panel Script");
    let door_script = row_left("Door Script");
    assert!(
        door_script > panel_script + 10.0,
        "the door's script under the panel's: {door_script} against {panel_script}"
    );
    let _ = (panel, door);
}

/// A linkset is the root and the prims linked to it: the tree lists them
/// at one level, as the grid sends them, never a prim under a prim.
#[test]
fn the_tree_lists_the_linksets_prims_at_one_level() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let left_of = |label: &str| {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos.x),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{label} is drawn"))
    };
    assert!(
        (left_of("Door") - left_of("Control Panel")).abs() < 1.0,
        "the root's children stand at its level"
    );
}

/// Folding a prim takes the prims linked under it away with its own
/// scripts: the group is a group.
#[test]
fn a_folded_prim_takes_the_prims_linked_under_it_away() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (mut linkset, _panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "panel source", "door source");
    linkset.prims[1].depth = 1;
    linkset.prims.push(LinksetPrim {
        object: Uuid::from_u128(79),
        kind: PrimKind::Prim,
        name: "Handle".into(),
        depth: 2,
        type_default: 0,
        loading: false,
        can_new_script: true,
        menu: None,
        touch: true,
        scripts: Vec::new(),
    });
    // as a click on the door's caret leaves it
    ctx.data_mut(|d| d.insert_persisted(linkset::collapsed_id(linkset.root), vec![door.object]));
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let lsl = aurora_i18n::tr!("script-type-lsl").to_string();
    let row = |name: &str| {
        output.shapes.iter().any(|s| match &s.shape {
            egui::Shape::Text(t) => t.galley.job.text.starts_with(name) && t.galley.job.text.ends_with(&lsl),
            _ => false,
        })
    };
    assert!(drawn(&output, "Control Panel"), "the prim that holds the door keeps its row");
    assert!(drawn(&output, "Door"), "the folded prim keeps its own row");
    assert!(drawn(&output, "Panel Script"), "the panel's script is not the door's");
    assert!(!drawn(&output, "Handle"), "the prim linked under the door goes with it");
    assert!(!row("Door Script"), "and the door's own script too");
}

/// A script of the tree with no tab: the click asks the app for an editor
/// of its own, with the rights and the source the app reads.
#[test]
fn a_tree_row_without_a_tab_asks_the_app_to_open_its_script() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (mut linkset, panel, _door) = linkset_fixture();
    let spare = ScriptTarget {
        object: panel.object,
        item: Uuid::from_u128(9),
    };
    linkset.prims[0].scripts.push(LinksetScript {
        object: spare.object,
        item: spare.item,
        name: "Spare Script".into(),
        flags: 0,
        running: None,
        permissions: Permissions::new(perm::COPY | perm::MODIFY, 0),
        dirty: false,
        menu: Some(LinksetScriptMenu {
            moves: false,
            refused: None,
            tip: "Copy the script to the inventory",
            can_delete: true,
            can_control: true,
        }),
    });
    let mut settings = settings::ScriptSettings::default();
    let load = editors.open(panel, "Panel Script".into(), true, true, 0).unwrap();
    editors.loaded(load, panel, Ok("panel source".into()));
    editors.documents[0].linkset = linkset.root;
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text.starts_with("Spare Script") => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .expect("the row of the script that has no tab");
    let mut opened = Vec::new();
    for pressed in [true, false] {
        let (_, actions) = frame_linksets_actions(
            &ctx,
            &mut editors,
            &mut settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
        opened.extend(actions);
    }
    assert!(
        opened
            .iter()
            .any(|action| matches!(action, Action::OpenScript(target) if *target == spare)),
        "the app opens the script of the tree row"
    );
    // the click did not replace the document already open
    assert_eq!(editors.documents.len(), 1);
    assert_eq!(editors.active.get(&linkset.root), Some(&panel));
}

/// A quit waits while the open scripts ask about their text: it goes on
/// once the last one is saved or discarded, and gives up on a cancel.
#[test]
fn a_quit_waits_for_the_scripts_and_goes_on_after_the_last_one() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let mut settings = settings::ScriptSettings::default();
    for target in [panel, door] {
        let load = editors.open(target, "Script".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok("saved".into()));
        let e = editors.documents.iter_mut().find(|e| e.target == target).unwrap();
        e.linkset = linkset.root;
        e.source = "edited".into();
    }
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert!(!editors.prepare_exit(), "both scripts have unsaved text");
    assert!(!editors.exit_ready());
    // discard the script in front: the other one asks next
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let discard = text_at(&output, aurora_i18n::tr!("script-discard"));
    click_at(&ctx, &mut editors, &mut settings, linksets, discard, egui::PointerButton::Primary);
    assert!(!editors.exit_ready(), "the second script still asks");
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let discard = text_at(&output, aurora_i18n::tr!("script-discard"));
    click_at(&ctx, &mut editors, &mut settings, linksets, discard, egui::PointerButton::Primary);
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert!(editors.documents.is_empty());
    assert!(editors.exit_ready(), "the quit can go on");

    // a cancel leaves the script open, and says so for the quit
    let load = editors.open(panel, "Script".into(), true, true, 0).unwrap();
    editors.loaded(load, panel, Ok("saved".into()));
    editors.documents[0].linkset = linkset.root;
    editors.documents[0].source = "edited again".into();
    assert!(!editors.prepare_exit());
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let cancel = text_at(&output, aurora_i18n::tr!("common-cancel"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, cancel, egui::PointerButton::Primary);
    assert!(
        actions.iter().any(|action| matches!(action, Action::CloseCancelled)),
        "a cancelled prompt abandons the quit"
    );
    assert!(!editors.exit_ready());
    assert_eq!(editors.documents.len(), 1);
}

/// Find and the preferences work in a window of a linkset's scripts: the
/// panel belongs to the script's own source, and the preferences come up
/// over the window that hid them.
#[test]
fn find_and_the_preferences_work_in_a_linkset_window() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, door) = linkset_fixture();
    let mut settings = settings::ScriptSettings::default();
    for (target, source) in [(panel, "panel source"), (door, "door source")] {
        let load = editors.open(target, "Script".into(), true, true, 0).unwrap();
        editors.loaded(load, target, Ok(source.into()));
        editors.documents.iter_mut().find(|e| e.target == target).unwrap().linkset = linkset.root;
    }
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }

    // Ctrl+F opens Find for the script in front
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![key(egui::Key::F, egui::Modifiers::COMMAND)],
        linksets,
    );
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert!(editors.panels.search.open, "Ctrl+F opens Find");
    assert!(editors.panels.search.owns(source_id(door)), "Find follows the tab in front");
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(
        output
            .shapes
            .iter()
            .any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == aurora_i18n::tr!("script-search-title"))),
        "the Find panel is drawn"
    );

    // File › Editor preferences… opens the panel over the window
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let file = text_at(&output, aurora_i18n::tr!("script-menu-file"));
    click_at(&ctx, &mut editors, &mut settings, linksets, file, egui::PointerButton::Primary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let item = text_at(&output, aurora_i18n::tr!("script-menu-preferences"));
    click_at(&ctx, &mut editors, &mut settings, linksets, item, egui::PointerButton::Primary);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert!(editors.panels.preferences.open, "the menu opens the preferences");
    let window = egui::Id::unique(super::dialogs::Preferences::WINDOW);
    let rect = ctx.memory(|m| m.area_rect(window)).expect("the preferences window");
    assert_eq!(
        ctx.layer_id_at(rect.center()).map(|layer| layer.id),
        Some(window),
        "the preferences sit over the script window"
    );
}

/// Ctrl and Shift choose several script rows; the menu of a selected row
/// acts on all of them, and a plain click leaves one alone.
#[test]
fn ctrl_and_shift_choose_several_scripts_and_the_menu_acts_on_them() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..2 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    assert_eq!(editors.active.get(&linkset.root), Some(&door));
    assert_eq!(
        editors.tree_selection.get(&linkset.root),
        Some(&vec![door]),
        "the script in front is the selection"
    );
    // Ctrl+click the other row: both selected, the script in front kept
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Panel Script");
    click_at_mods(
        &ctx,
        &mut editors,
        &mut settings,
        linksets,
        at,
        egui::PointerButton::Primary,
        egui::Modifiers::COMMAND,
    );
    assert_eq!(
        editors.tree_selection.get(&linkset.root),
        Some(&vec![door, panel]),
        "Ctrl+click adds the row"
    );
    assert_eq!(editors.active.get(&linkset.root), Some(&door), "and does not open it");
    // Shift+click the row in front: the range between the two
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Door Script");
    click_at_mods(
        &ctx,
        &mut editors,
        &mut settings,
        linksets,
        at,
        egui::PointerButton::Primary,
        egui::Modifiers::SHIFT,
    );
    assert_eq!(
        editors.tree_selection.get(&linkset.root),
        Some(&vec![panel, door]),
        "the range between the anchor and the click"
    );
    // the menu of a selected row deletes both
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, aurora_i18n::tr!("common-delete"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    let deleted: Vec<(Uuid, Uuid)> = actions
        .iter()
        .filter_map(|action| match action {
            Action::DeleteScript { object, item } => Some((*object, *item)),
            _ => None,
        })
        .collect();
    assert_eq!(
        deleted,
        vec![(panel.object, panel.item), (door.object, door.item)],
        "Delete on the selection deletes both scripts"
    );
}

/// A console row that names a script brings it to the front.
/// The Properties section keeps its room while the page of a new script is
/// in front: the tree's list and its Settings row do not move.
#[test]
fn the_trees_foot_keeps_its_room_for_the_new_script_page() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    editors.active.insert(linkset.root, panel);
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let y = |output: &egui::FullOutput, label: &str| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.pos.y),
            _ => None,
        })
    };
    let before = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let settings_y = y(&before, aurora_i18n::tr!("script-tree-settings").as_ref()).expect("the Settings row");
    editors.open_draft(linkset.root, panel.object, "Panel".into(), Lang::Lua, Vec::new());
    let draft = editors.documents.iter().find(|e| e.draft.is_some()).expect("the page").target;
    editors.active.insert(linkset.root, draft);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let after = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert!(
        y(&after, aurora_i18n::tr!("script-properties-none").as_ref()).is_some(),
        "the section says there is no script in front"
    );
    assert_eq!(
        y(&after, aurora_i18n::tr!("script-tree-settings").as_ref()),
        Some(settings_y),
        "the foot keeps its room"
    );
}

/// The documentation pane follows the settings: open, it shows the name
/// the window read; closed, it is gone and the source takes its room.
/// The menus write the keys of the commands the source listens to, at the
/// right end of their entries, as the top bar's menus do.
#[test]
fn the_menus_write_the_keys_of_their_commands() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let t = target(1);
    let load = editors.open(t, "Keys".into(), true, true, 0).unwrap();
    editors.loaded(load, t, Ok("default { state_entry() { } }".into()));
    settle(&ctx, &mut editors);
    // the modifiers of this platform, as `keybinds::Binding::label` writes them
    let keys = |key: &str, ctrl: bool, shift: bool, alt: bool| {
        let prefix = crate::keybinds::Binding {
            input: crate::keybinds::Input::Key(String::new()),
            ctrl,
            shift,
            alt,
        }
        .label();
        format!("{prefix}{key}")
    };
    click_label(&ctx, &mut editors, aurora_i18n::tr!("script-menu-file").as_ref());
    let mut output = frame(&ctx, &mut editors, vec![]);
    for _ in 0..2 {
        output = frame(&ctx, &mut editors, vec![]);
    }
    for expected in [keys("S", true, false, false), keys("O", true, false, false)] {
        assert!(drawn(&output, &expected), "the File menu writes {expected}");
    }
    // no key for a command the source has none for
    assert!(drawn(&output, aurora_i18n::tr!("script-menu-revert").as_ref()));
}

/// A menu at the pointer is never under the help of the word under it: a
/// tooltip is drawn above every menu (`Order::Tooltip` over
/// `Order::Foreground`), so the help goes while one is open.
#[test]
fn a_menu_at_the_pointer_is_not_under_the_helps_tooltip() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "llSay(0, \"hi\");\n", "source");
    editors.active.insert(linkset.root, panel);
    let linksets = std::slice::from_ref(&linkset);
    let drawn = |output: &egui::FullOutput, needle: &str| {
        output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) => text.galley.job.text.contains(needle),
            _ => false,
        })
    };
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.starts_with("llSay(0,") => Some(text.pos + egui::vec2(4.0, 6.0)),
            _ => None,
        })
        .expect("the source's first line");
    // the pointer on the word, still: its help arrives
    frame_linksets(&ctx, &mut editors, &mut settings, vec![egui::Event::PointerMoved(at)], linksets);
    let mut output = None;
    for _ in 0..60 {
        let frame = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
        let help = drawn(&frame, "integer channel");
        output = Some(frame);
        if help {
            break;
        }
    }
    assert!(drawn(&output.expect("a frame"), "integer channel"), "the help of the word under it");
    // a right click: the menu opens, and the help is not drawn over it
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let mut output = None;
    for _ in 0..4 {
        let frame = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
        let menu = drawn(&frame, aurora_i18n::tr!("script-goto-definition").as_ref());
        output = Some(frame);
        if menu {
            break;
        }
    }
    let output = output.expect("a frame");
    assert!(
        drawn(&output, aurora_i18n::tr!("script-goto-definition").as_ref()),
        "the menu is open"
    );
    assert!(!drawn(&output, "integer channel"), "the help is not drawn over its menu");
    // the pointer leaves the text to reach the menu's own rows: the word it was
    // opened on stays, and its declaration and references rows with it
    let references_row = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == aurora_i18n::tr!("script-find-references") => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .expect("the references row");
    for _ in 0..3 {
        frame_linksets(
            &ctx,
            &mut editors,
            &mut settings,
            vec![egui::Event::PointerMoved(references_row)],
            linksets,
        );
    }
    let over = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    for row in [
        aurora_i18n::tr!("script-goto-definition"),
        aurora_i18n::tr!("script-find-references"),
    ] {
        assert!(drawn(&over, row), "the menu keeps {row} once the pointer is on it");
    }
}

/// The page of a new script is no row of the tree: while it is in front no
/// row keeps the light, and once its script opens that script alone is lit.
#[test]
fn a_new_scripts_page_takes_the_selection_off_the_tree() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    editors.active.insert(linkset.root, door);
    editors.tree_selection.insert(linkset.root, vec![door]);
    let linksets = std::slice::from_ref(&linkset);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(
        editors.tree_selection.get(&linkset.root),
        Some(&vec![door]),
        "the script in front is the one row lit"
    );
    // the panel's New script: its page takes the front
    editors.open_draft(linkset.root, panel.object, "Panel".into(), Lang::Lsl, Vec::new());
    let page = editors.documents.iter().find(|e| e.draft.is_some()).expect("the page").target;
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let selection = editors.tree_selection.get(&linkset.root).cloned().unwrap_or_default();
    assert!(selection.is_empty(), "no row of the tree is lit beside the page: {selection:?}");
    // the language chosen: the script appears, the app opens it, and it is
    // the one row lit
    let new = ScriptTarget {
        object: panel.object,
        item: Uuid::from_u128(999),
    };
    let load = editors.open(new, "New Script".into(), true, false, 0).expect("a read");
    editors.loaded(load, new, Ok("default { }".into()));
    editors
        .documents
        .iter_mut()
        .find(|e| e.target == new)
        .expect("the new script")
        .linkset = linkset.root;
    editors.documents.iter_mut().find(|e| e.target == page).expect("the page").closed = true;
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(
        editors.tree_selection.get(&linkset.root),
        Some(&vec![new]),
        "the new script alone is lit"
    );
}

/// The foot of the tree gives the script in front's language, rights and
/// state, and folds to its title.
#[test]
fn the_trees_foot_shows_the_script_in_front() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    editors.active.insert(linkset.root, panel);
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    assert!(drawn("Panel Script"), "the header names the script in front");
    assert!(drawn("LSL"), "the script's language");
    assert!(drawn("CMT"), "its rights");
    assert!(drawn(aurora_i18n::tr!("script-state-running").as_ref()), "its state");
}

/// A row's context menu offers the entries of the contents window, and a
/// prim's offers a new script; each runs the app's own action.
#[test]
fn a_rows_context_menu_offers_the_contents_actions() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (linkset, panel, _door) = linkset_fixture();
    let mut settings = settings::ScriptSettings::default();
    // the tab's name differs from the tree's, to click the tree row
    let load = editors.open(panel, "Panel Doc".into(), true, true, 0).unwrap();
    editors.loaded(load, panel, Ok("panel source".into()));
    editors.documents[0].linkset = linkset.root;
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }

    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    for entry in [
        aurora_i18n::tr!("common-open"),
        aurora_i18n::tr!("common-properties"),
        aurora_i18n::tr!("build-ui-contents-copy-to-inventory"),
        aurora_i18n::tr!("common-delete"),
        aurora_i18n::tr!("common-stop"),
    ] {
        text_at(&output, entry);
    }
    let at = text_at(&output, aurora_i18n::tr!("build-ui-contents-copy-to-inventory"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::TakeScript { object, item } if *object == panel.object && *item == panel.item
        )),
        "Copy to inventory takes the script"
    );

    // the item's own window, and its running state, as the contents menu
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, aurora_i18n::tr!("common-properties"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Properties(target) if *target == panel)),
        "Properties opens the item's window"
    );
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = row_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, aurora_i18n::tr!("common-stop"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Running { target, running: false } if *target == panel)),
        "Stop asks the script to stop"
    );

    // the prim's own menu: a new script in its contents, the page that
    // asks which language it will be written in
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Control Panel");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, aurora_i18n::tr!("common-new-script"));
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        editors
            .documents
            .iter()
            .any(|e| e.draft.as_ref().is_some_and(|draft| draft.object == panel.object)),
        "New script asks which language, for that prim"
    );

    // and the object menu's own entries for the linkset's root
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Control Panel");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    for entry in [
        aurora_i18n::tr!("ui-context-take"),
        aurora_i18n::tr!("ui-context-take-copy"),
        aurora_i18n::tr!("common-delete"),
    ] {
        text_at(&output, entry);
    }
    let at = text_at(&output, aurora_i18n::tr!("ui-context-take-copy"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::TakeObject { object, copy: true } if *object == panel.object)),
        "Take copy takes a copy of that object"
    );
}

/// A stopped script's row offers Run, and the click asks to run it.
#[test]
fn a_stopped_scripts_row_offers_run() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let (mut linkset, panel, _door) = linkset_fixture();
    linkset.prims[0].scripts[0].running = Some(false);
    let mut settings = settings::ScriptSettings::default();
    let load = editors.open(panel, "Panel Doc".into(), true, true, 0).unwrap();
    editors.loaded(load, panel, Ok("panel source".into()));
    editors.documents[0].linkset = linkset.root;
    let linksets = std::slice::from_ref(&linkset);
    for _ in 0..3 {
        frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, "Panel Script");
    click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Secondary);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let at = text_at(&output, aurora_i18n::tr!("build-ui-contents-run"));
    let actions = click_at(&ctx, &mut editors, &mut settings, linksets, at, egui::PointerButton::Primary);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Running { target, running: true } if *target == panel)),
        "Run asks the script to run"
    );
}

/// The shapes a frame painted, with the batches of a reserved slot
/// (`Shape::Vec`: the text's row, its guides, its boxes) taken out.
fn flatten_shapes<'a>(painted: &'a [egui::epaint::ClippedShape], out: &mut Vec<&'a egui::Shape>) {
    fn collect<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
        match shape {
            egui::Shape::Vec(inner) => {
                for shape in inner {
                    collect(shape, out);
                }
            }
            other => out.push(other),
        }
    }
    for clipped in painted {
        collect(&clipped.shape, out);
    }
}

/// Is this text drawn, anywhere?
fn drawn(output: &egui::FullOutput, label: &str) -> bool {
    output
        .shapes
        .iter()
        .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
}

/// Where an exact label is drawn: the highest one, since a script's name is
/// both its tab's (above the tree) and its row's.
fn text_at(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        })
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .unwrap_or_else(|| panic!("{label} is drawn"))
}

/// The tree row of a script: its name, beside the "Running" word at the
/// right end of its line (the tree's scripts run; their language is their
/// icon's and their tooltip's).
fn row_at(output: &egui::FullOutput, name: &str) -> egui::Pos2 {
    let running = aurora_i18n::tr!("script-running").to_string();
    let badges: Vec<f32> = output
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == running => Some(t.pos.y),
            _ => None,
        })
        .collect();
    output
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == name && badges.iter().any(|y| (y - t.pos.y).abs() < 2.0) => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the row of {name}"))
}

/// A drag of the pointer from one point to another, over four frames, as
/// a reader drags a tree row onto the source.
fn drag(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    linksets: &[Linkset],
    from: egui::Pos2,
    to: egui::Pos2,
) -> Vec<Action> {
    let mut actions = Vec::new();
    let (_, taken) = frame_linksets_actions(ctx, editors, settings, vec![egui::Event::PointerMoved(from)], linksets);
    actions.extend(taken);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (_, taken) = frame_linksets_actions(ctx, editors, settings, vec![button(from, true)], linksets);
    actions.extend(taken);
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        let (_, taken) = frame_linksets_actions(ctx, editors, settings, vec![egui::Event::PointerMoved(at)], linksets);
        actions.extend(taken);
    }
    let (_, taken) = frame_linksets_actions(ctx, editors, settings, vec![button(to, false)], linksets);
    actions.extend(taken);
    actions
}

/// A script dragged from the tree and let go over a pane opens beside the
/// one in front, on the side it was dropped on.
#[test]
fn a_script_dragged_from_the_tree_opens_beside_the_one_in_front() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the door is the one in front: the panel is dragged beside it
    let from = row_at(&output, "Panel Script");
    let actions = drag(&ctx, &mut editors, &mut settings, linksets, from, egui::pos2(1100.0, 400.0));
    assert!(actions.is_empty(), "the script was already open");
    assert_eq!(
        editors.splits.get(&linkset.root),
        Some(&(door, panel)),
        "the dropped script beside the one in front"
    );
}

/// While a script is dragged over the source, the pane it would take is
/// lit, so the drop has no surprise.
#[test]
fn the_pane_a_dragged_script_would_take_is_lit() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    // the panel, while the door is in front: a drop beside it is allowed
    let from = row_at(&output, "Panel Script");
    let to = egui::pos2(1100.0, 400.0);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame_linksets(&ctx, &mut editors, &mut settings, vec![egui::Event::PointerMoved(from)], linksets);
    frame_linksets(&ctx, &mut editors, &mut settings, vec![button(from, true)], linksets);
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        frame_linksets(&ctx, &mut editors, &mut settings, vec![egui::Event::PointerMoved(at)], linksets);
    }
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let lit = crate::theme::Theme::default().palette().violet.gamma_multiply(0.12);
    assert!(
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == lit)),
        "the pane under the pointer is lit"
    );
    // let go, and the split is made
    frame_linksets(&ctx, &mut editors, &mut settings, vec![button(to, false)], linksets);
    assert_eq!(editors.splits.get(&linkset.root), Some(&(door, panel)));
}

/// The preferences' settings are pages behind a row of tabs: the pages'
/// names share the top row, and a click on one shows it.
#[test]
fn the_preferences_pages_are_tabs() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, _panel, _door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    editors.panels.preferences.open = true;
    // a window lays out a first, unmeasured frame before its body shows
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    let look = text_at(&output, "Look");
    let editing = text_at(&output, "Editing");
    assert!((editing.y - look.y).abs() < 20.0, "the pages' tabs share a row");
    assert!(editing.x - look.x < 160.0, "side by side, not two panes");
    assert!(drawn(aurora_i18n::tr!("script-preferences-reset-colors").as_ref()), "the Look page");
    assert!(
        !drawn(aurora_i18n::tr!("script-preferences-word-wrap").as_ref()),
        "the Editing page is not drawn yet"
    );
    // a click on the Editing tab shows its page
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame_linksets(
        &ctx,
        &mut editors,
        &mut settings,
        vec![egui::Event::PointerMoved(editing)],
        linksets,
    );
    frame_linksets(&ctx, &mut editors, &mut settings, vec![button(editing, true)], linksets);
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![button(editing, false)], linksets);
    let drawn = |label: &str| {
        output
            .shapes
            .iter()
            .any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == label))
    };
    assert!(
        drawn(aurora_i18n::tr!("script-preferences-word-wrap").as_ref()),
        "the Editing page after the click"
    );
    assert!(
        !drawn(aurora_i18n::tr!("script-preferences-reset-colors").as_ref()),
        "the Look page has gone"
    );
}

/// A script with no tab is opened for the drop, and takes its pane once
/// the app has given it a document.
#[test]
fn a_script_with_no_tab_is_opened_and_takes_its_pane() {
    let ctx = egui::Context::default();
    let mut editors = Editors::default();
    let mut settings = settings::ScriptSettings::default();
    let (linkset, panel, door) = two_scripts_open(&ctx, &mut editors, &mut settings, "source", "source");
    let linksets = std::slice::from_ref(&linkset);
    // a script of the object with no tab: close the door's
    editors.documents.iter_mut().find(|e| e.target == door).unwrap().closed = true;
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let target = door;
    let output = frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    let from = row_at(&output, "Door Script");
    let actions = drag(&ctx, &mut editors, &mut settings, linksets, from, egui::pos2(1100.0, 400.0));
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::OpenScript(open) if *open == target)),
        "the app is asked to open it"
    );
    assert_eq!(
        editors.pending_split.get(&linkset.root),
        Some(&(panel, target)),
        "its pane waits for the document"
    );
    // the app opens it, and the next frame gives it its side
    let load = editors.open(target, "Door Script".into(), true, true, 0).unwrap();
    editors.loaded(load, target, Ok("default { }".into()));
    editors.documents.iter_mut().find(|e| e.target == target).unwrap().linkset = linkset.root;
    frame_linksets(&ctx, &mut editors, &mut settings, vec![], linksets);
    assert_eq!(editors.splits.get(&linkset.root), Some(&(panel, target)));
    assert_eq!(editors.pending_split.get(&linkset.root), None, "the pane is taken");
}

/// A frame of the script windows with the modifiers a click carries (the
/// plain `frame_linksets_actions` keeps none).
fn frame_linksets_mods(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    events: Vec<egui::Event>,
    linksets: &[Linkset],
    modifiers: egui::Modifiers,
) -> (egui::FullOutput, Vec<Action>) {
    let theme = crate::theme::Theme::default();
    theme.apply(ctx, 1.0);
    let mut actions = Vec::new();
    // egui 0.37 carries the modifiers as an event of the frame
    let mut events = events;
    events.insert(0, egui::Event::ModifiersChanged(modifiers));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            focused: true,
            events,
            ..Default::default()
        },
        |ui| {
            super::super::layout::frame(ui.ctx(), &theme.palette(), false);
            actions = editors.show(
                ui.ctx(),
                &theme.palette(),
                settings,
                &mut super::super::color_picker::Preferences::default(),
                linksets,
            );
        },
    );
    output.textures_delta.clear();
    (output, actions)
}

fn click_at_mods(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    linksets: &[Linkset],
    at: egui::Pos2,
    button: egui::PointerButton,
    modifiers: egui::Modifiers,
) -> Vec<Action> {
    let mut actions = Vec::new();
    for pressed in [true, false] {
        let (_, taken) = frame_linksets_mods(
            ctx,
            editors,
            settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers,
                },
            ],
            linksets,
            modifiers,
        );
        actions.extend(taken);
    }
    // the modifiers are not held past the click
    let _ = frame_linksets_mods(ctx, editors, settings, vec![], linksets, egui::Modifiers::NONE);
    actions
}

fn click_at(
    ctx: &egui::Context,
    editors: &mut Editors,
    settings: &mut settings::ScriptSettings,
    linksets: &[Linkset],
    at: egui::Pos2,
    button: egui::PointerButton,
) -> Vec<Action> {
    let mut actions = Vec::new();
    for pressed in [true, false] {
        let (_, taken) = frame_linksets_actions(
            ctx,
            editors,
            settings,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            linksets,
        );
        actions.extend(taken);
    }
    actions
}

#[test]
fn disconnect_keeps_unsaved_source_and_rejects_the_previous_sessions_reply() {
    let mut editors = Editors::default();
    let target = target(1);
    let request = editors.open(target, "Test".into(), true, true, 0).expect("load");
    editors.loaded(request, target, Ok("original".into()));
    editors.documents[0].source = "unsaved".into();
    let Action::Save { request, .. } = editors.documents[0].save().expect("save") else {
        panic!("save action")
    };
    editors.disconnected();
    assert_eq!(editors.documents[0].source, "unsaved");
    assert!(editors.documents[0].dirty());
    assert!(!editors.documents[0].can_save());
    assert!(!editors.saved(
        request,
        target,
        &Ok(ScriptSaved {
            asset: Uuid::from_u128(3),
            compiled: true,
            checked: true,
            errors: vec![]
        })
    ));
}
