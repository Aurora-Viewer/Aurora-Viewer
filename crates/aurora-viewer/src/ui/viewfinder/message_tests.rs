use super::*;

/// The rows of the shown text `wanted` in this frame's shapes.
fn rows(output: &egui::FullOutput, wanted: &str) -> usize {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.galley.rows.len()),
            _ => None,
        })
        .expect("message visible")
}

#[test]
fn a_long_message_after_a_short_one_stays_on_one_line() {
    let ctx = egui::Context::default();
    let theme = crate::theme::Theme::default();
    theme.apply(&ctx, 1.0);
    let p = theme.palette();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0));
    let mut time = 0.0;
    let mut frame = |text: &str| {
        time += 0.1;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            },
            |ui| message(ui.ctx(), &p, screen, text, false),
        );
        output.textures_delta.clear();
        output
    };
    // the area learns the short message's width first
    for _ in 0..3 {
        frame("AF-S");
    }
    let long = "Interface hidden · I or the button at the bottom right shows it again";
    frame(long);
    assert_eq!(rows(&frame(long), long), 1);
}
