//! The text of a field sits in the middle of its face: as much room above
//! its capitals as below their baseline (descenders aside), within a point,
//! with every embedded font (`field_text_margin`).

use crate::theme::{Palette, Theme};

/// The room above the capitals of `HEH` typed in the field `add` shows with
/// `font`, and below their baseline, measured on the glyphs egui draws, on
/// the rectangle `add` covers.
fn room(font: &str, add: impl Fn(&mut egui::Ui, &Palette, &mut String)) -> (f32, f32) {
    let ctx = egui::Context::default();
    Theme::default().apply(&ctx, 1.0);
    crate::ui::fonts::apply(&ctx, font);
    let p = Theme::default().palette();
    let mut face = egui::Rect::NOTHING;
    let mut shapes = Vec::new();
    for _ in 0..3 {
        let mut text = "HEH".to_owned();
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                face = ui.horizontal(|ui| add(ui, &p, &mut text)).response.rect;
            },
        );
        out.textures_delta.clear();
        shapes = out.shapes;
    }
    let mut ink = egui::Rect::NOTHING;
    for clipped in &shapes {
        if let egui::Shape::Text(text) = &clipped.shape
            && text.galley.text() == "HEH"
        {
            ink = ink.union(super::ink_rect(&text.galley).translate(text.pos.to_vec2()));
        }
    }
    assert!(ink.is_positive(), "{font}: the field's text was drawn");
    (ink.top() - face.top(), face.bottom() - ink.bottom())
}

fn assert_centred(font: &str, what: &str, (above, below): (f32, f32)) {
    assert!(
        (above - below).abs() <= 1.0,
        "{font}, {what}: {above} above the capitals, {below} below"
    );
}

#[test]
fn text_fields_centre_their_capitals_with_every_font() {
    for font in crate::ui::fonts::BUILTINS.iter().map(|b| b.name) {
        let framed = room(font, |ui, _, text| {
            ui.add(egui::TextEdit::singleline(text).desired_width(200.0));
        });
        assert_centred(font, "theme field", framed);
        let icon = room(font, |ui, p, text| {
            super::pills::search_box(ui, p, text, "", 300.0);
        });
        assert_centred(font, "field with an icon", icon);
        let bare = room(font, |ui, p, text| {
            let small = egui::FontId::proportional(12.0);
            super::pills::field_frame(ui, p).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::singleline(text)
                        .font(small.clone())
                        .frame(egui::Frame::NONE.inner_margin(super::field_text_margin(ui.ctx(), &small)))
                        .desired_width(200.0),
                );
            });
        });
        assert_centred(font, "bare field on its own face", bare);
    }
}
