//! Resizable windows keep their size: a body that measures more than the
//! window (a list of `available_height() - <guessed footer>` above a footer
//! that measured more) made egui grow the window a little every frame.

use super::elide_tests::Frames;
use crate::theme::Palette;

/// Run `show` on fresh state from `new` for 30 frames, at the interface
/// scales 1 / 1.25 / 1.5, with the text 1.4 and 2 times larger, and at each of
/// `sizes`, the floater `id` placed at that size: its rect never changes
/// after the first two frames, and every text `bottom` (its bottom-most
/// button) is drawn inside it.
pub(crate) fn assert_window_steady<S>(
    id: &str,
    sizes: &[egui::Vec2],
    bottom: &str,
    mut new: impl FnMut() -> S,
    mut show: impl FnMut(&mut S, &egui::Context, &Palette),
) {
    let p = crate::theme::Theme::default().palette();
    for (zoom, text) in [(1.0f32, 1.0f32), (1.25, 1.0), (1.5, 1.0), (1.0, 1.4), (1.0, 2.0)] {
        for &size in sizes {
            let mut frames = Frames::new();
            crate::theme::Theme::default().apply(&frames.ctx, text);
            crate::ui::fonts::apply(&frames.ctx, crate::ui::fonts::DEFAULT);
            frames.ctx.set_zoom_factor(zoom);
            super::place_floater(&frames.ctx, id, egui::pos2(40.0, 40.0), size);
            let mut state = new();
            let case = format!("{id}, zoom {zoom}, text {text}, {size:?}");
            let mut rects = Vec::new();
            let mut texts = Vec::new();
            for _ in 0..30 {
                texts = frames.run(Vec::new(), |ui| show(&mut state, ui.ctx(), &p));
                rects.push(
                    frames
                        .ctx
                        .memory(|m| m.area_rect(egui::Id::unique(id)))
                        .expect("the window is shown"),
                );
            }
            // its height from the start; its width once a list's scroll bar
            // has faded in (egui widens the window by a fraction of a point
            // during that fade, once)
            let first = rects[2];
            assert!(
                rects[2..].iter().all(|r| r.y_range() == first.y_range()),
                "{case}: the window grew: {:?}",
                changes(&rects)
            );
            let last = rects[rects.len() - 1];
            assert!(
                rects[15..].iter().all(|r| *r == last),
                "{case}: the window moved or grew: {:?}",
                changes(&rects)
            );
            let first = last;
            let found: Vec<_> = texts.iter().filter(|t| t.source == bottom).collect();
            assert!(
                !found.is_empty(),
                "{case}: {bottom:?} is drawn, among {:?}",
                texts.iter().map(|t| t.source.as_str()).collect::<Vec<_>>()
            );
            for t in found {
                assert!(t.rect.bottom() <= first.bottom(), "{case}: {bottom:?} at {:?} in {first:?}", t.rect);
            }
        }
    }
}

/// The frames where the rect changed, with its new size.
fn changes(rects: &[egui::Rect]) -> Vec<(usize, egui::Rect)> {
    rects
        .iter()
        .enumerate()
        .filter(|(i, r)| *i == 0 || rects[i - 1] != **r)
        .map(|(i, r)| (i, *r))
        .collect()
}

const SIZES: [egui::Vec2; 2] = [egui::vec2(360.0, 520.0), egui::vec2(300.0, 260.0)];

/// A list over a footer of three rows, the footer's height guessed (the
/// defect) or measured by `above_footer` (the fix).
fn window(ctx: &egui::Context, p: &Palette, id: &str, guessed: bool) {
    let mut open = true;
    super::Floater::new(id, "Steady", egui::pos2(40.0, 40.0), SIZES[0]).show(ctx, p, &mut open, |ui| {
        let footer = |ui: &mut egui::Ui| {
            for label in ["Third", "Second", "Last"] {
                let _ = ui.button(label);
            }
        };
        if guessed {
            let h = (ui.available_height() - 30.0).max(40.0);
            ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::hover());
            footer(ui);
        } else {
            let (mut list, foot) = super::above_footer(ui);
            let h = list.available_height();
            list.allocate_exact_size(egui::vec2(list.available_width(), h), egui::Sense::hover());
            foot.show(ui, footer);
        }
    });
}

#[test]
fn a_measured_footer_keeps_the_window_still() {
    assert_window_steady(
        "steady-test-fixed",
        &SIZES,
        "Last",
        || (),
        |_, ctx, p| window(ctx, p, "steady-test-fixed", false),
    );
}

#[test]
#[should_panic(expected = "the window grew")]
fn a_guessed_footer_is_caught() {
    assert_window_steady(
        "steady-test-guessed",
        &SIZES,
        "Last",
        || (),
        |_, ctx, p| window(ctx, p, "steady-test-guessed", true),
    );
}
