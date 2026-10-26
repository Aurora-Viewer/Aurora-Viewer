use super::*;

/// One frame of a spell-checked, auto-replaced field.
fn frame(ctx: &egui::Context, text: &mut String, events: Vec<egui::Event>) -> TextEditOutput {
    let p = crate::theme::Theme::default().palette();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 200.0))),
        events,
        ..Default::default()
    };
    let mut kept = None;
    let mut out = ctx.run_ui(raw, |ui| {
        let mut output = egui::TextEdit::multiline(text)
            .id(Id::unique("spelling_test"))
            .desired_width(120.0)
            .show(ui);
        field(ui, &p, &mut output, text, true);
        kept = Some(output);
    });
    out.textures_delta.clear();
    kept.expect("drawn")
}

#[test]
fn a_typed_space_replaces_the_keyword() {
    let ctx = egui::Context::default();
    let mut text = String::new();
    frame(&ctx, &mut text, Vec::new());
    ctx.memory_mut(|m| m.request_focus(Id::unique("spelling_test")));
    frame(&ctx, &mut text, Vec::new());
    crate::autoreplace::set_enabled(true);
    frame(&ctx, &mut text, vec![egui::Event::Text("ok brb".into())]);
    assert_eq!(text, "ok brb", "no space yet");
    let out = frame(&ctx, &mut text, vec![egui::Event::Text(" ".into())]);
    assert_eq!(text, "ok be right back ");
    assert_eq!(out.cursor_range.map(|r| r.primary.index.0), Some(17));
    // typing goes on after the replacement
    frame(&ctx, &mut text, vec![egui::Event::Text("now".into())]);
    assert_eq!(text, "ok be right back now");
    crate::autoreplace::set_enabled(false);
}

#[test]
fn a_word_on_two_rows_gets_two_pieces() {
    let ctx = egui::Context::default();
    let mut text = "short words then Pneumonoultramicroscopicsilicovolcanoconiosis".to_owned();
    let out = frame(&ctx, &mut text, Vec::new());
    let start = text.chars().position(|c| c == 'P').expect("long word");
    let pieces = segments(&out, start..text.chars().count());
    assert!(pieces.len() >= 2, "{pieces:?}");
    assert!(pieces.windows(2).all(|w| w[1].2 > w[0].2), "one row below the other");
    // a short word: one piece on its row
    let pieces = segments(&out, 0..5);
    assert_eq!(pieces.len(), 1);
    assert!(pieces[0].1 > pieces[0].0);
}
