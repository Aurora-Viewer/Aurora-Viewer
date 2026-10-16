//! Text beside icons, badges and boxes, with every interface font.

use egui::{Color32, Pos2, Rect, Vec2};

/// The interface fonts the alignment is checked with: the embedded
/// ones, and an installed one when there is one (Segoe UI on Windows:
/// a tall line box, 0.11 em of vertical shift).
pub(crate) fn fonts() -> Vec<String> {
    let mut fonts: Vec<String> = crate::ui::fonts::BUILTINS.iter().map(|b| b.name.to_owned()).collect();
    let system = crate::ui::fonts::system_fonts();
    let pick = system.iter().find(|f| f.name == "Segoe UI").or_else(|| system.first());
    match pick {
        Some(f) => fonts.push(format!("system:{}", f.path.display())),
        None => eprintln!("no system font: checked with the embedded ones only"),
    }
    fonts
}

/// The capitals' ink of each text painted, in physical pixels, where
/// epaint draws it: the galley's origin and each glyph's corner on the
/// pixel (`tessellate_text`, `tessellate_glyphs`).
pub(crate) fn capitals_ink(shapes: &[egui::epaint::ClippedShape], ppp: f32) -> Vec<(String, Rect)> {
    fn walk(shape: &egui::Shape, ppp: f32, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                let origin = (t.pos.to_vec2() * ppp).round();
                let mut ink = Rect::NOTHING;
                for row in &t.galley.rows {
                    for g in row.row.glyphs.iter().filter(|g| g.chr.is_uppercase() && g.uv_rect.size.y > 0.0) {
                        let corner = ((g.pos + g.uv_rect.offset).to_vec2() * ppp).round();
                        let min = origin + row.pos.to_vec2() * ppp + corner;
                        ink = ink.union(Rect::from_min_size(min.to_pos2(), g.uv_rect.size * ppp));
                    }
                }
                if ink.is_positive() {
                    out.push((t.galley.text().to_owned(), ink));
                }
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, ppp, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, ppp, &mut out);
    }
    out
}

/// Where the letters of a one-line text painted in a frame stand, in
/// physical pixels: the top and the foot of its capitals as drawn, and its
/// font's x-height as drawn.
struct Letters {
    caps: Rect,
    x_height: f32,
}

impl Letters {
    /// The middle between the middle of the capitals and the middle of the
    /// x-height: where a row's text is centred on its icons.
    fn optical(&self) -> f32 {
        let foot = self.caps.bottom();
        (self.caps.center().y + foot - self.x_height / 2.0) / 2.0
    }
}

/// The widgets checked, each in a row of its own: (name, label, the
/// rectangle its letters are centred in, whether they centre their
/// capitals instead of their optical middle).
type Drawn = Vec<(&'static str, &'static str, Rect, bool)>;

fn draw_widgets(ui: &mut egui::Ui, p: &crate::theme::Palette, nudge: f32) -> Drawn {
    use super::*;
    let mut out: Drawn = Vec::new();
    ui.add_space(10.0 + nudge);
    let mut row = |ui: &mut egui::Ui, name, label, caps, add: &mut dyn FnMut(&mut egui::Ui) -> Rect| {
        let rect = add(ui);
        out.push((name, label, rect, caps));
    };
    row(ui, "action_button", "Home", false, &mut |ui| {
        action_button(ui, p, "home-filled", "Home", ActionKind::Flat, Vec2::new(0.0, 28.0)).rect
    });
    row(ui, "action_menu_button", "Teleport", false, &mut |ui| {
        action_menu_button(ui, p, "home-filled", "Teleport", false, 28.0).rect
    });
    row(ui, "menu::item", "Edit", false, &mut |ui| {
        let before = ui.cursor().top();
        crate::ui::menu::item(ui, p, "pencil-filled", "Edit");
        Rect::from_x_y_ranges(ui.max_rect().x_range(), before..=ui.min_rect().bottom())
    });
    row(ui, "menu::item_keys", "Fly", false, &mut |ui| {
        let before = ui.cursor().top();
        crate::ui::menu::item_keys(ui, p, "plane-filled", "Fly", "Alt+H");
        Rect::from_x_y_ranges(ui.max_rect().x_range(), before..=ui.min_rect().bottom())
    });
    row(ui, "checkbox", "Flying", false, &mut |ui| checkbox(ui, p, &mut true, "Flying").rect);
    row(ui, "radio", "Light", false, &mut |ui| radio(ui, p, true, "Light").rect);
    row(ui, "tabs", "Places", false, &mut |ui| {
        let r = ui.horizontal(|ui| tabs(ui, p, &mut 0, &[("Places", true), ("Teleports", true)]));
        r.response.rect
    });
    row(ui, "segmented", "Pie", true, &mut |ui| {
        let before = ui.cursor().top();
        segmented(ui, p, &mut 0, &[("Pie", true), ("Menu", false)]);
        Rect::from_x_y_ranges(ui.max_rect().x_range(), before..=ui.min_rect().bottom()).shrink2(Vec2::new(0.0, 2.0))
    });
    row(ui, "pill_tab", "People", false, &mut |ui| {
        pill_tab(ui, p, "users", "People", None, false, true, true).rect
    });
    row(ui, "filter_pill", "Friends", false, &mut |ui| {
        filter_pill(ui, p, "user-filled", "Friends", true).rect
    });
    row(ui, "navigation", "Landmarks", false, &mut |ui| {
        navigation(ui, p, "map-pin-filled", "Landmarks", true).rect
    });
    row(ui, "paint_elided", "Lunaria", false, &mut |ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 24.0), egui::Sense::hover());
        paint_elided(ui, "Lunaria", rect.left_center(), 200.0, egui::FontId::proportional(12.0), p.ink);
        rect
    });
    // a favourite with descenders and one without, in one row
    for label in ["Lounge", "Tower"] {
        row(ui, "paint_ink_left", label, false, &mut |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(200.0, 28.0), egui::Sense::hover());
            let font = egui::FontId::proportional(12.5);
            let galley = ui.painter().layout_no_wrap(label.to_owned(), font, p.ink);
            paint_ink_left(ui.painter(), galley, rect.left_center(), p.ink);
            rect
        });
    }
    out
}

/// The letters of the text `label` painted in `shapes`.
fn letters_of(ctx: &egui::Context, shapes: &[egui::epaint::ClippedShape], label: &str) -> Option<Letters> {
    let ppp = ctx.pixels_per_point();
    let caps = capitals_ink(shapes, ppp).into_iter().find(|(t, _)| t == label)?.1;
    let font = shapes.iter().find_map(|c| match &c.shape {
        egui::Shape::Text(t) if t.galley.text() == label => t.galley.job.sections.first().map(|s| s.format.font_id.clone()),
        _ => None,
    })?;
    let x = ctx.fonts_mut(|f| f.layout_no_wrap("x".into(), font, Color32::PLACEHOLDER));
    Some(Letters {
        caps,
        x_height: super::ink_rect(&x).height() * ppp,
    })
}

/// Every shared widget that writes a label beside an icon or in a box,
/// with each interface font, at the scales 1, 1.25 and 1.5, on whole and
/// half points: its letters are centred on its row within half a pixel,
/// the best the pixel grid allows (their optical middle, or their capitals
/// for the segmented choice). Centring egui's text row put them up to
/// 1.9 px off (action buttons, pill tabs with Roboto at 1.25), centring
/// each line's ink up to 1.5 px (a name with descenders in the
/// favourites).
#[test]
fn labels_are_centred_on_their_icons_with_every_font() {
    let p = crate::theme::Theme::default().palette();
    let mut worst: Vec<(String, f32)> = Vec::new();
    let mut failures = Vec::new();
    for font in fonts() {
        for zoom in [1.0f32, 1.25, 1.5] {
            for nudge in [0.0f32, 0.5] {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                crate::ui::fonts::apply(&ctx, &font);
                ctx.set_zoom_factor(zoom);
                let mut drawn = Vec::new();
                let mut shapes = Vec::new();
                for _ in 0..3 {
                    let input = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 900.0))),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(input, |ui| drawn = draw_widgets(ui, &p, nudge));
                    out.textures_delta.clear();
                    shapes = out.shapes;
                }
                let ppp = ctx.pixels_per_point();
                for (name, label, rect, caps) in &drawn {
                    let case = format!("{name} with {font} at {zoom}, nudged {nudge}");
                    let letters = letters_of(&ctx, &shapes, label).unwrap_or_else(|| panic!("{case}: no {label:?}"));
                    let middle = if *caps { letters.caps.center().y } else { letters.optical() };
                    let off = middle - rect.center().y * ppp;
                    let key = format!("{name} {}", font.rsplit(['\\', ':']).next().unwrap_or(&font));
                    match worst.iter_mut().find(|(n, _)| *n == key) {
                        Some((_, w)) if off.abs() > w.abs() => *w = off,
                        Some(_) => {}
                        None => worst.push((key, off)),
                    }
                    if off.abs() > 0.501 {
                        failures.push(format!("{case}: letters {off:+.2} px from the row's centre"));
                    }
                }
            }
        }
    }
    for (key, off) in &worst {
        eprintln!("worst {key}: {off:+.2} px");
    }
    assert!(failures.is_empty(), "{} off:\n{}", failures.len(), failures.join("\n"));
}
