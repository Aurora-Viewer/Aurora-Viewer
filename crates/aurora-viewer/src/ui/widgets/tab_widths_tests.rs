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
        assert!(filled[2] >= fitted[2] - 0.01, "room {room}: {filled:?} / {fitted:?}");
        assert!(filled[2] > filled[0], "room {room}: {filled:?}");
    }
}

#[test]
fn fitted_and_even_tabs_keep_their_widths() {
    // the strip of `tabs` stops growing 40 points past its labels
    let fit = tab_widths(&LABELS, 700.0, GAP, TabWidths::Fit);
    assert_eq!(fit, vec![105.0, 115.0, 143.0]);
    let even = tab_widths(&LABELS, 700.0, GAP, TabWidths::Even);
    assert!(even.iter().all(|x| (x - 692.0 / 3.0).abs() < 0.01), "{even:?}");
}
