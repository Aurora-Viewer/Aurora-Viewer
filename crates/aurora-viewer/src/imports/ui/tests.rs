use super::*;

#[test]
fn popup_rows_keep_their_rectangles_when_hover_changes() {
    let ctx = egui::Context::default();
    crate::theme::Theme::default().apply(&ctx, 1.0);
    let mut selected = 0;
    let mut rectangles: Vec<egui::Rect> = Vec::new();
    for hover in [None, Some(0), Some(1), Some(2), None] {
        for _ in 0..3 {
            let pointer = hover.map(|index| rectangles[index].center());
            let mut current = Vec::new();
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 500.0))),
                    events: pointer.map_or_else(Vec::new, |pos| vec![egui::Event::PointerMoved(pos)]),
                    ..Default::default()
                },
                |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    ui.set_width(250.0);
                    for (index, label) in ["Convex hull only", "Medium", "Bounding box"].into_iter().enumerate() {
                        current.push(selection(ui, &mut selected, index, label).rect);
                    }
                },
            );
            drop(output);
            if !rectangles.is_empty() {
                assert_eq!(current, rectangles);
            }
            rectangles = current;
        }
    }
    assert_eq!(selected, 0);
}
