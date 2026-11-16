use super::restricted_text::*;
use crate::text_validation::Policy;
use egui::{
    Event, Id,
    text::{CCursor, CCursorRange},
};

fn frame(ctx: &egui::Context, text: &mut String, policy: Policy, events: Vec<Event>, multiline: bool) -> egui::text_edit::TextEditOutput {
    let mut field = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        },
        |ui| {
            field = Some(show(ui, true, text, policy, |b, _| {
                let edit = if multiline {
                    egui::TextEdit::multiline(b)
                } else {
                    egui::TextEdit::singleline(b)
                };
                edit.id(Id::unique("restricted_field")).desired_width(300.0).char_limit(63)
            }));
        },
    );
    output.textures_delta.clear();
    field.expect("field")
}

fn selected(ctx: &egui::Context, text: &mut String, policy: Policy) {
    let mut field = frame(ctx, text, policy, vec![], false);
    field
        .state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()))));
    field.state.store(ctx, field.response.id);
    ctx.memory_mut(|m| m.request_focus(field.response.id));
}

#[test]
fn restricted_text_rejects_typing_and_entire_pastes_without_losing_the_selection() {
    let ctx = egui::Context::default();
    for event in [
        Event::Text("桜".into()),
        Event::Text("龙".into()),
        Event::Paste("Prefix 桜 suffix".into()),
        Event::Paste(format!("{}桜", "a".repeat(70))),
    ] {
        let mut text = "Original".to_owned();
        selected(&ctx, &mut text, Policy::Ascii);
        let field = frame(&ctx, &mut text, Policy::Ascii, vec![event], false);
        assert_eq!(text, "Original");
        assert!(!field.response.changed());
        let range = field.state.cursor.char_range().expect("selection");
        assert_eq!(range.sorted_cursors()[0].index.0, 0);
        assert_eq!(range.sorted_cursors()[1].index.0, 8);
        assert!(
            ctx.data(|d| d.get_temp::<Feedback>(field.response.id.with("invalid_input")))
                .is_some()
        );
    }
}

#[test]
fn restricted_text_accepts_punctuation_and_keeps_multiline_rules_separate() {
    let ctx = egui::Context::default();
    let mut text = "Original".into();
    selected(&ctx, &mut text, Policy::PrintableNoPipe);
    frame(
        &ctx,
        &mut text,
        Policy::PrintableNoPipe,
        vec![Event::Paste("ASCII (test) #2!".into())],
        false,
    );
    assert_eq!(text, "ASCII (test) #2!");
    selected(&ctx, &mut text, Policy::MultilineNoPipe);
    frame(
        &ctx,
        &mut text,
        Policy::MultilineNoPipe,
        vec![Event::Paste("First\nSecond".into())],
        true,
    );
    assert_eq!(text, "First\nSecond");
    selected(&ctx, &mut text, Policy::MultilineNoPipe);
    frame(
        &ctx,
        &mut text,
        Policy::MultilineNoPipe,
        vec![Event::Paste("First|Second".into())],
        true,
    );
    assert_eq!(text, "First\nSecond");
}

#[test]
fn restricted_text_ime_commit_does_not_replace_selected_text_with_unsupported_characters() {
    let ctx = egui::Context::default();
    let mut text = "Original".into();
    selected(&ctx, &mut text, Policy::Ascii);
    for candidate in ["sa", "桜"] {
        let field = frame(
            &ctx,
            &mut text,
            Policy::Ascii,
            vec![Event::Ime(egui::ImeEvent::Preedit {
                text: candidate.into(),
                active_range_chars: None,
            })],
            false,
        );
        assert_eq!(text, "Original");
        assert!(!field.response.changed());
        assert!(
            ctx.data(|d| d.get_temp::<Feedback>(field.response.id.with("invalid_input")))
                .is_none()
        );
    }
    let field = frame(
        &ctx,
        &mut text,
        Policy::Ascii,
        vec![Event::Ime(egui::ImeEvent::Commit("桜".into()))],
        false,
    );
    assert_eq!(text, "Original");
    assert!(
        ctx.data(|d| d.get_temp::<Feedback>(field.response.id.with("invalid_input")))
            .is_some()
    );
    frame(
        &ctx,
        &mut text,
        Policy::Ascii,
        vec![Event::Ime(egui::ImeEvent::Commit("ASCII".into()))],
        false,
    );
    assert_eq!(text, "ASCII");
}

#[test]
fn restricted_text_never_rewrites_existing_grid_text_and_digits_reject_mixed_pastes() {
    let ctx = egui::Context::default();
    let mut text = "Existing 桜龙".into();
    frame(&ctx, &mut text, Policy::PrintableNoPipe, vec![], false);
    assert_eq!(text, "Existing 桜龙");
    selected(&ctx, &mut text, Policy::PrintableNoPipe);
    frame(
        &ctx,
        &mut text,
        Policy::PrintableNoPipe,
        vec![Event::Text("Repaired".into())],
        false,
    );
    assert_eq!(text, "Repaired");
    text = "100".into();
    selected(&ctx, &mut text, Policy::Digits);
    frame(&ctx, &mut text, Policy::Digits, vec![Event::Paste("10２0".into())], false);
    assert_eq!(text, "100");
}

#[test]
fn restricted_text_numeric_feedback_never_restricts_a_neighbouring_unicode_editor() {
    let ctx = egui::Context::default();
    let mut value = 100.0;
    let mut prose = String::new();
    let mut numeric = None;
    let draw = |events, value: &mut f64, prose: &mut String, numeric: &mut Option<egui::Response>| {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| {
                *numeric = Some(ui.add(number(egui::DragValue::new(value), Policy::Digits)));
                ui.add(egui::TextEdit::singleline(prose).id(Id::unique("unicode_prose")));
            },
        );
        out.textures_delta.clear();
    };
    draw(vec![], &mut value, &mut prose, &mut numeric);
    let id = numeric.as_ref().expect("number").id;
    ctx.memory_mut(|m| m.request_focus(id));
    draw(vec![], &mut value, &mut prose, &mut numeric);
    draw(vec![Event::Paste("10桜".into())], &mut value, &mut prose, &mut numeric);
    assert_eq!(value, 100.0);
    assert!(ctx.data(|d| d.get_temp::<Feedback>(id.with("invalid_input"))).is_some());
    ctx.memory_mut(|m| m.request_focus(Id::unique("unicode_prose")));
    draw(vec![], &mut value, &mut prose, &mut numeric);
    draw(vec![Event::Text("桜龙".into())], &mut value, &mut prose, &mut numeric);
    assert_eq!(prose, "桜龙");
}

#[test]
fn restricted_text_rejected_pastes_do_not_pollute_undo_or_redo() {
    let undo_key = |shift, pressed| Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            command: true,
            shift,
            ..Default::default()
        },
    };
    let run = |reject_paste| {
        let ctx = egui::Context::default();
        let mut text = "Original".into();
        selected(&ctx, &mut text, Policy::Ascii);
        for _ in 0..60 {
            frame(&ctx, &mut text, Policy::Ascii, vec![], false);
        }
        frame(&ctx, &mut text, Policy::Ascii, vec![Event::Text("Renamed".into())], false);
        for _ in 0..60 {
            frame(&ctx, &mut text, Policy::Ascii, vec![], false);
        }
        selected(&ctx, &mut text, Policy::Ascii);
        let events = if reject_paste {
            vec![Event::Paste("Bad 桜 name".into())]
        } else {
            vec![]
        };
        frame(&ctx, &mut text, Policy::Ascii, events, false);
        assert_eq!(text, "Renamed");
        let mut history = Vec::new();
        for shift in [false, false, false, true, true, true] {
            frame(&ctx, &mut text, Policy::Ascii, vec![undo_key(shift, true)], false);
            history.push(text.clone());
            frame(&ctx, &mut text, Policy::Ascii, vec![undo_key(shift, false)], false);
        }
        assert!(history.iter().any(|value| value == "Original"));
        assert_eq!(text, "Renamed");
        history
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn restricted_text_invalid_replacement_and_enter_cannot_submit_the_old_name() {
    let ctx = egui::Context::default();
    let mut text = "Original".into();
    selected(&ctx, &mut text, Policy::Ascii);
    let mut submitted = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![
                Event::Paste("Bad 桜".into()),
                Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| {
            field(ui, true, &mut text, Policy::Ascii, |b, _| {
                egui::TextEdit::singleline(b).id(Id::unique("restricted_field"))
            });
            submitted = super::modal_keys::enter_pressed(ui);
        },
    );
    output.textures_delta.clear();
    assert!(!submitted);
    assert_eq!(text, "Original");
    assert!(ctx.memory(|m| m.has_focus(Id::unique("restricted_field"))));
}

#[test]
fn restricted_text_empty_preedit_does_not_roll_back_typing_or_paste() {
    for paste in [false, true] {
        for empty_first in [false, true] {
            let ctx = egui::Context::default();
            let mut text = "Original".to_owned();
            selected(&ctx, &mut text, Policy::PrintableNoPipe);
            let input = if paste {
                Event::Paste("Portrait".into())
            } else {
                Event::Text("Portrait".into())
            };
            let empty = Event::Ime(egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            });
            let events = if empty_first { vec![empty, input] } else { vec![input, empty] };
            let output = frame(&ctx, &mut text, Policy::PrintableNoPipe, events, false);
            assert_eq!(text, "Portrait", "paste {paste}, empty_first {empty_first}");
            assert!(output.response.changed());
            frame(&ctx, &mut text, Policy::PrintableNoPipe, vec![Event::Text("2".into())], false);
            assert_eq!(text, "Portrait2");
        }
    }
}
