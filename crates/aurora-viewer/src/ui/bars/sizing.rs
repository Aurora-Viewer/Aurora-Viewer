//! The widths of a toolbar's buttons, after LLToolBar::updateLayoutAsNeeded
//! (indra/llui/lltoolbar.cpp, originally LGPL 2.1) and Firestorm's layout
//! styles: each button its own width, all the width of the widest, or the
//! buttons sharing the whole bar. Unlike Firestorm, a toolbar keeps one row
//! (or column): when its buttons do not fit, their labels are cut with "…"
//! and they narrow (the human's decision of 2026-10-15).

use crate::ui::toolbar::{ButtonMode, Sizing};

/// Room before and after a label.
pub const LABEL_PAD: f32 = 8.0;
/// Between an icon and its label.
pub const ICON_GAP: f32 = 5.0;
pub const LABEL_FONT: f32 = 12.0;
/// The widest button with a label (floater_toybox.xml
/// `button_icon_and_text.button_width.max`).
pub const MAX_LABELLED: f32 = 172.0;

/// A button's own width: `square` with its icon only; with a label, the
/// label's `text` width, its icon and their margins (`extra`: the Speak
/// button's arrow), never under `square` nor over `MAX_LABELLED`.
pub fn natural(mode: ButtonMode, square: f32, icon: f32, text: f32, extra: f32) -> f32 {
    let wanted = match mode {
        ButtonMode::IconsOnly => return square,
        ButtonMode::IconsWithText => LABEL_PAD * 2.0 + icon + ICON_GAP + text + extra,
        ButtonMode::TextOnly => LABEL_PAD * 2.0 + text + extra,
    };
    wanted.ceil().clamp(square, MAX_LABELLED.max(square))
}

/// The buttons' widths in a row of `room` with `gap` between them: their
/// own (`Auto`), the widest one's for all when they have labels (`Fixed`;
/// icons keep their square), or theirs with the rest of the room shared
/// equally (`Fill`). Too long, labelled buttons are held under one common
/// width, the widest first, so that the longest labels are cut and the
/// short ones stay whole, down to `min`.
pub fn fit(natural: &[f32], sizing: Sizing, labelled: bool, room: f32, gap: f32, min: f32) -> Vec<f32> {
    let n = natural.len();
    if n == 0 {
        return Vec::new();
    }
    let gaps = gap * (n - 1) as f32;
    let widest = natural.iter().copied().fold(0.0, f32::max);
    let mut widths: Vec<f32> = match sizing {
        Sizing::Fixed if labelled => vec![widest; n],
        Sizing::Fill => {
            let share = ((room - gaps - natural.iter().sum::<f32>()) / n as f32).max(0.0).floor();
            natural.iter().map(|w| w + share).collect()
        }
        _ => natural.to_vec(),
    };
    let total = widths.iter().sum::<f32>() + gaps;
    if total > room && labelled {
        // the common width that fills the room: the narrower buttons keep
        // theirs, the others share what is left
        let mut sorted = widths.clone();
        sorted.sort_by(f32::total_cmp);
        let mut left = room - gaps;
        let mut cap = min;
        for (i, w) in sorted.iter().enumerate() {
            let rest = (n - i) as f32;
            if w * rest > left {
                cap = (left / rest).floor();
                break;
            }
            left -= w;
        }
        let cap = cap.max(min);
        for w in &mut widths {
            *w = w.min(cap);
        }
    }
    widths
}

/// Where a row `width` wide starts in `[start, end]`, by its alignment
/// (never before `start`).
pub fn place(align: crate::ui::toolbar::Align, start: f32, end: f32, width: f32) -> f32 {
    use crate::ui::toolbar::Align;
    let x = match align {
        Align::Start => start,
        Align::Center => start + ((end - start - width) / 2.0).round(),
        Align::End => end - width,
    };
    x.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_keep_their_square_and_labels_their_text() {
        assert_eq!(natural(ButtonMode::IconsOnly, 34.0, 18.0, 80.0, 0.0), 34.0);
        assert_eq!(
            natural(ButtonMode::IconsWithText, 34.0, 18.0, 50.0, 0.0),
            8.0 + 18.0 + 5.0 + 50.0 + 8.0
        );
        assert_eq!(natural(ButtonMode::TextOnly, 34.0, 18.0, 50.0, 0.0), 66.0);
        assert_eq!(natural(ButtonMode::TextOnly, 34.0, 18.0, 5.0, 0.0), 34.0, "never under the square");
        assert_eq!(natural(ButtonMode::IconsWithText, 34.0, 18.0, 500.0, 0.0), MAX_LABELLED);
    }

    #[test]
    fn layout_styles_share_the_room_like_firestorm() {
        let own = [40.0, 60.0, 80.0];
        assert_eq!(fit(&own, Sizing::Auto, true, 1000.0, 2.0, 24.0), own);
        assert_eq!(fit(&own, Sizing::Fixed, true, 1000.0, 2.0, 24.0), [80.0; 3]);
        assert_eq!(
            fit(&[34.0, 44.0], Sizing::Fixed, false, 1000.0, 2.0, 24.0),
            [34.0, 44.0],
            "icons keep their size"
        );
        let filled = fit(&own, Sizing::Fill, true, 304.0, 2.0, 24.0);
        assert_eq!(filled, [80.0, 100.0, 120.0]);
        assert!(filled.iter().sum::<f32>() + 4.0 <= 304.0);
    }

    #[test]
    fn a_row_too_long_cuts_its_labels_and_keeps_one_row() {
        let own = [120.0; 10];
        let w = fit(&own, Sizing::Auto, true, 600.0, 2.0, 24.0);
        assert!(w.iter().sum::<f32>() + 18.0 <= 600.0, "{w:?}");
        assert!(w.iter().all(|x| *x >= 24.0));
        // down to the least width, then over the room
        let w = fit(&own, Sizing::Auto, true, 100.0, 2.0, 24.0);
        assert!(w.iter().all(|x| *x == 24.0));
        // the long labels are cut first, the short ones stay whole
        let w = fit(&[50.0, 150.0, 150.0], Sizing::Auto, true, 254.0, 2.0, 24.0);
        assert_eq!(w, [50.0, 100.0, 100.0]);
    }

    #[test]
    fn alignment_places_the_row() {
        use crate::ui::toolbar::Align;
        assert_eq!(place(Align::Start, 100.0, 500.0, 100.0), 100.0);
        assert_eq!(place(Align::Center, 100.0, 500.0, 100.0), 250.0);
        assert_eq!(place(Align::End, 100.0, 500.0, 100.0), 400.0);
        assert_eq!(place(Align::End, 100.0, 150.0, 100.0), 100.0);
    }
}
