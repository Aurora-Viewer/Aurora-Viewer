//! How a tab strip shares its row (`tab_widths`).

use super::*;

/// Room the labels of Places' Legacy tabs need, in points: Favorites,
/// Landmarks, Teleport history.
const LABELS: [f32; 3] = [65.0, 75.0, 103.0];
const GAP: f32 = 4.0;

fn row(widths: &[f32]) -> f32 {
    widths.iter().sum::<f32>() + GAP * (widths.len() as f32 - 1.0)
}

#[test]
fn filled_tabs_span_the_row_at_every_width() {
    for room in [120.0, 236.0, 260.0, 317.0, 378.0, 700.0, 1400.0] {
        let w = tab_widths(&LABELS, room, GAP, TabWidths::Fill);
        assert!((row(&w) - room).abs() < 0.01, "room {room}: {w:?}");
    }
}

#[test]
fn filled_tabs_are_even_once_every_label_fits() {
    // 3 × 103 + 2 gaps: the longest label fits in an equal share
    for room in [317.0, 378.0, 700.0] {
        let w = tab_widths(&LABELS, room, GAP, TabWidths::Fill);
        assert!(w.iter().all(|x| (x - w[0]).abs() < 0.01), "room {room}: {w:?}");
        assert!(w[0] >= LABELS[2], "room {room}: {w:?}");
    }
}

#[test]
fn narrow_filled_tabs_cut_the_long_label_no_more_than_the_fitted_strip() {
    // too narrow for even shares: the labels' shares, as `tabs` gives them
    for room in [200.0, 236.0, 260.0] {
        let filled = tab_widths(&LABELS, room, GAP, TabWidths::Fill);
        let fitted = tab_widths(&LABELS, room, GAP, TabWidths::Fit);
        // never cut more than the fitted strip, and never below its own
        // label once the labels fit
        assert!(filled[2] >= fitted[2].min(LABELS[2]) - 0.01, "room {room}: {filled:?} / {fitted:?}");
        assert!(filled[2] > filled[0], "room {room}: {filled:?}");
    }
}

#[test]
fn fitted_and_even_tabs_keep_their_widths() {
    // the strip of `tabs` stops growing 40 points past its labels
    let fit = tab_widths(&LABELS, 700.0, GAP, TabWidths::Fit);
    assert_eq!(fit, vec![105.0, 115.0, 143.0]);
    // narrower than the labels: each in proportion to its label
    let fit = tab_widths(&LABELS, 200.0, GAP, TabWidths::Fit);
    let scale = (200.0 - 2.0 * GAP) / (LABELS.iter().sum::<f32>());
    assert!(fit.iter().zip(LABELS).all(|(w, l)| (w - l * scale).abs() < 0.01), "{fit:?}");
    let even = tab_widths(&LABELS, 700.0, GAP, TabWidths::Even);
    assert!(even.iter().all(|x| (x - 692.0 / 3.0).abs() < 0.01), "{even:?}");
}

/// #2805: widening the row a little never makes a tab jump or shrink, from
/// a row narrower than the labels to well past equal shares, across both
/// switches (the labels' total, 251, and equal shares, 317).
#[test]
fn filled_tabs_grow_smoothly_with_the_room() {
    let mut before = tab_widths(&LABELS, 150.0, GAP, TabWidths::Fill);
    let mut room = 150.0;
    while room < 450.0 {
        room += 0.1;
        let now = tab_widths(&LABELS, room, GAP, TabWidths::Fill);
        for (i, (a, b)) in before.iter().zip(&now).enumerate() {
            assert!(b - a >= -0.001 && b - a <= 0.101, "room {room}, tab {i}: {before:?} then {now:?}");
        }
        before = now;
    }
    // just past the labels: only the narrowest tab takes the extra room
    let w = tab_widths(&LABELS, 260.0, GAP, TabWidths::Fill);
    assert!((w[0] - 74.0).abs() < 0.01 && w[1] == 75.0 && w[2] == 103.0, "{w:?}");
}
