//! The widths of a toolbar's buttons, after LLToolBar::updateLayoutAsNeeded
//! (indra/llui/lltoolbar.cpp, originally LGPL 2.1) and Firestorm's layout
//! styles: each button its own width, all the width of the widest, or the
//! buttons sharing the whole bar. Unlike Firestorm, a toolbar keeps one row
//! (or column): when its buttons do not fit, their labels are cut with "…"
//! and they narrow (the human's decision of 2026-10-15).

use crate::ui::toolbar::{Align, ButtonMode, Sizing, Style};

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

/// Fixed gaps and live blocks keep their intrinsic sizes. Elastic gaps take
/// surplus before Fill expands ordinary buttons.
#[allow(clippy::too_many_arguments)]
pub fn toolbar(
    ctx: &egui::Context,
    buttons: &[crate::ui::skin::ToolButton],
    natural: &[f32],
    sizing: Sizing,
    labelled: bool,
    room: f32,
    gap: f32,
    min: f32,
) -> Vec<f32> {
    let fixed: Vec<_> = buttons
        .iter()
        .map(|button| crate::ui::toolbar::structure::fixed_width(&button.command))
        .collect();
    let stretch = crate::ui::toolbar::structure::stretch(ctx, buttons);
    let groups = block_styles(ctx, buttons, sizing, labelled);
    let mut sizes = fit_blocks(natural, &fixed, &groups, &stretch, room, gap, min);
    crate::ui::toolbar::structure::distribute(&mut sizes, &stretch, room, gap);
    sizes
}

pub(super) struct BlockSizing {
    range: std::ops::Range<usize>,
    sizing: Sizing,
    align: Option<Align>,
    pub(super) labelled: bool,
}

pub(super) fn block_styles(
    ctx: &egui::Context,
    buttons: &[crate::ui::skin::ToolButton],
    sizing: Sizing,
    labelled: bool,
) -> Vec<BlockSizing> {
    use crate::ui::toolbar::{ToolbarUi, appearance, structure};
    let settings = ToolbarUi::layout_configuration(ctx).unwrap_or_else(|| appearance::settings(ctx));
    let mut groups = Vec::new();
    let mut start = 0;
    for end in 0..=buttons.len() {
        if end != buttons.len() && !structure::is_separator(&buttons[end].command) {
            continue;
        }
        if let Some(first) = buttons[start..end].iter().find(|button| !structure::is_divider(&button.command)) {
            let style = settings.block_styles.get(&first.command).copied().unwrap_or_default();
            // A single labelled override keeps its own width; it must not
            // equalize neighbouring icons in an otherwise icon-only block.
            let labelled = style.mode.map_or(labelled, |mode| mode != ButtonMode::IconsOnly);
            groups.push(BlockSizing {
                range: start..end,
                sizing: style.sizing.unwrap_or(sizing),
                align: style.align,
                labelled,
            });
        }
        start = end + 1;
    }
    groups
}

pub(super) fn has_alignment(ctx: &egui::Context, buttons: &[crate::ui::skin::ToolButton]) -> bool {
    use crate::ui::toolbar::{ToolbarUi, appearance};
    let settings = ToolbarUi::layout_configuration(ctx).unwrap_or_else(|| appearance::settings(ctx));
    buttons.iter().any(|button| {
        settings.drawer_of(&button.command).is_none()
            && settings
                .block_styles
                .get(&button.command)
                .is_some_and(|style| style.align.is_some())
    })
}

pub(super) fn alignment(
    ctx: &egui::Context,
    buttons: &[crate::ui::skin::ToolButton],
    sizes: &mut [f32],
    room: f32,
    gap: f32,
    base: Style,
) -> Option<f32> {
    if !has_alignment(ctx, buttons) {
        return None;
    }
    align_blocks(
        sizes,
        &block_styles(ctx, buttons, base.sizing, base.mode != ButtonMode::IconsOnly),
        room,
        gap,
        base.align,
    )
}

/// User-requested extension of LLToolBar::updateLayoutAsNeeded: anchor each
/// block along the bar, keeping canonical order and minimum inter-block gaps.
/// Extra room belongs to existing gap slots, so drag/drop and its preview use
/// exactly the same rectangles as the rendered blocks.
pub(super) fn align_blocks(sizes: &mut [f32], groups: &[BlockSizing], room: f32, gap: f32, base: Align) -> Option<f32> {
    if !groups.iter().any(|group| group.align.is_some()) {
        return None;
    }
    let mut offsets = Vec::with_capacity(sizes.len() + 1);
    offsets.push(0.0);
    for size in sizes.iter() {
        offsets.push(offsets.last().copied().unwrap_or_default() + size + gap);
    }
    let extent = (offsets.last().copied().unwrap_or_default() - gap).max(0.0);
    if extent >= room {
        return Some(0.0);
    }
    let mut starts = vec![0.0; groups.len()];
    let mut first = 0;
    while first < groups.len() {
        let align = groups[first].align.unwrap_or(base);
        let mut end = first + 1;
        while end < groups.len() && groups[end].align.unwrap_or(base) == align {
            end += 1;
        }
        let from = if first == 0 { 0.0 } else { offsets[groups[first].range.start] };
        let to = if end == groups.len() {
            extent
        } else {
            offsets[groups[end - 1].range.end] - gap
        };
        let lead = place(align, 0.0, room, to - from);
        for index in first..end {
            starts[index] = lead + offsets[groups[index].range.start] - from;
        }
        first = end;
    }
    for index in 1..groups.len() {
        let minimum = offsets[groups[index].range.start] - offsets[groups[index - 1].range.start];
        starts[index] = starts[index].max(starts[index - 1] + minimum);
    }
    let last = groups.len() - 1;
    starts[last] = starts[last].min(room - (extent - offsets[groups[last].range.start]));
    for index in (0..last).rev() {
        let minimum = offsets[groups[index + 1].range.start] - offsets[groups[index].range.start];
        starts[index] = starts[index].min(starts[index + 1] - minimum);
    }
    for index in 1..groups.len() {
        let separator = groups[index - 1].range.end;
        let minimum = offsets[groups[index].range.start] - offsets[groups[index - 1].range.start];
        sizes[separator] += (starts[index] - starts[index - 1] - minimum).max(0.0);
    }
    Some((starts[0] - offsets[groups[0].range.start]).max(0.0))
}

/// Equal widths and Fill are local to a block. Only Fill blocks share surplus;
/// fixed native controls and elastic inter-block gaps keep their existing rules.
pub(super) fn fit_blocks(
    natural: &[f32],
    fixed: &[bool],
    groups: &[BlockSizing],
    stretch: &[bool],
    room: f32,
    gap: f32,
    min: f32,
) -> Vec<f32> {
    let mut sizes = natural.to_vec();
    let mut fill = vec![false; natural.len()];
    for group in groups {
        let sizing = if group.sizing == Sizing::Fixed {
            Sizing::Fixed
        } else {
            Sizing::Auto
        };
        let widths = fit_fixed(
            &natural[group.range.clone()],
            &fixed[group.range.clone()],
            sizing,
            group.labelled,
            f32::INFINITY,
            gap,
            min,
        );
        sizes[group.range.clone()].copy_from_slice(&widths);
        if group.sizing == Sizing::Fill {
            for index in group.range.clone() {
                fill[index] = !fixed[index];
            }
        }
    }
    let count = fill.iter().filter(|flag| **flag).count();
    if count > 0 && !stretch.iter().any(|flag| *flag) {
        let surplus = (room - sizes.iter().sum::<f32>() - gap * sizes.len().saturating_sub(1) as f32).max(0.0);
        let share = (surplus / count as f32).floor();
        for (size, fill) in sizes.iter_mut().zip(fill) {
            if fill {
                *size += share;
            }
        }
    }
    fit_fixed(
        &sizes,
        fixed,
        Sizing::Auto,
        groups.iter().any(|group| group.labelled),
        room,
        gap,
        min,
    )
}

pub(super) fn block_widths(buttons: &[crate::ui::skin::ToolButton], natural: &[f32], min: f32, groups: &[BlockSizing]) -> Vec<f32> {
    let mut widths = vec![min; buttons.len()];
    let mut start = 0;
    for end in 0..=buttons.len() {
        if end != buttons.len() && !crate::ui::toolbar::structure::is_separator(&buttons[end].command) {
            continue;
        }
        let width = natural[start..end].iter().copied().fold(min, f32::max);
        widths[start..end].fill(width);
        start = end + 1;
    }
    for group in groups.iter().filter(|group| group.sizing == Sizing::Auto) {
        for index in group.range.clone() {
            if !crate::ui::toolbar::structure::is_divider(&buttons[index].command) {
                widths[index] = natural[index].max(min);
            }
        }
    }
    widths
}

pub fn fit_fixed(natural: &[f32], fixed: &[bool], sizing: Sizing, labelled: bool, room: f32, gap: f32, min: f32) -> Vec<f32> {
    let fixed_at = |i: usize| fixed.get(i).copied().unwrap_or(false);
    let variable: Vec<_> = natural
        .iter()
        .enumerate()
        .filter(|(i, _)| !fixed_at(*i))
        .map(|(_, width)| *width)
        .collect();
    let reserved: f32 = natural
        .iter()
        .enumerate()
        .filter(|(i, _)| fixed_at(*i))
        .map(|(_, width)| width)
        .sum();
    let extra_gaps = natural.len().saturating_sub(1).saturating_sub(variable.len().saturating_sub(1)) as f32 * gap;
    let mut widths = fit(&variable, sizing, labelled, room - reserved - extra_gaps, gap, min).into_iter();
    natural
        .iter()
        .enumerate()
        .map(|(i, width)| if fixed_at(i) { *width } else { widths.next().unwrap_or(*width) })
        .collect()
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
    fn independent_block_anchors_expand_only_the_inter_block_slots() {
        let groups = [Align::Start, Align::Center, Align::End]
            .into_iter()
            .enumerate()
            .map(|(index, align)| BlockSizing {
                range: index * 2..index * 2 + 1,
                sizing: Sizing::Auto,
                align: Some(align),
                labelled: false,
            })
            .collect::<Vec<_>>();
        let mut sizes = [40.0, 12.0, 60.0, 12.0, 50.0];
        let lead = align_blocks(&mut sizes, &groups, 600.0, 2.0, Align::Center).expect("explicit anchors");
        assert_eq!(lead, 0.0);
        assert_eq!([sizes[0], sizes[2], sizes[4]], [40.0, 60.0, 50.0]);
        assert_eq!(sizes[0] + sizes[1] + 4.0 + sizes[2] / 2.0, 300.0);
        assert_eq!(sizes.iter().sum::<f32>() + 8.0, 600.0);
        let mut cramped = [40.0, 12.0, 60.0, 12.0, 50.0];
        assert_eq!(align_blocks(&mut cramped, &groups, 180.0, 2.0, Align::Center), Some(0.0));
        assert_eq!(cramped, [40.0, 12.0, 60.0, 12.0, 50.0]);
    }

    #[test]
    fn conflicting_anchors_keep_order_minimum_gaps_and_outer_separators() {
        for first in [Align::Start, Align::Center, Align::End] {
            for last in [Align::Start, Align::Center, Align::End] {
                let groups = [first, last]
                    .into_iter()
                    .enumerate()
                    .map(|(index, align)| BlockSizing {
                        range: index * 2 + 1..index * 2 + 2,
                        sizing: Sizing::Auto,
                        align: Some(align),
                        labelled: false,
                    })
                    .collect::<Vec<_>>();
                let mut sizes = [12.0, 40.0, 12.0, 50.0, 12.0];
                let lead = align_blocks(&mut sizes, &groups, 500.0, 2.0, Align::Center).expect("anchors");
                assert!(lead >= 0.0 && lead + sizes.iter().sum::<f32>() + 8.0 <= 500.01);
                assert_eq!([sizes[0], sizes[1], sizes[3], sizes[4]], [12.0, 40.0, 50.0, 12.0]);
                assert!(sizes[2] >= 12.0);
            }
        }
        let mut sizes = [40.0, 12.0, 50.0];
        assert_eq!(align_blocks(&mut sizes, &[], 500.0, 2.0, Align::End), None);
        assert_eq!(sizes, [40.0, 12.0, 50.0]);
    }

    #[test]
    fn block_sizing_changes_only_its_members_and_respects_elastic_gaps() {
        let natural = [40.0, 80.0, 12.0, 30.0, 70.0];
        let fixed = [false, false, true, false, false];
        let mut groups = [
            BlockSizing {
                range: 0..2,
                sizing: Sizing::Auto,
                align: None,
                labelled: true,
            },
            BlockSizing {
                range: 3..5,
                sizing: Sizing::Fixed,
                align: None,
                labelled: true,
            },
        ];
        let auto = fit_blocks(&natural, &fixed, &groups, &[], 600.0, 2.0, 24.0);
        assert_eq!(auto, [40.0, 80.0, 12.0, 70.0, 70.0]);
        groups[0].sizing = Sizing::Fixed;
        assert_eq!(
            fit_blocks(&natural, &fixed, &groups, &[], 600.0, 2.0, 24.0),
            [80.0, 80.0, 12.0, 70.0, 70.0]
        );
        groups[0].sizing = Sizing::Fill;
        let fill = fit_blocks(&natural, &fixed, &groups, &[], 600.0, 2.0, 24.0);
        assert_eq!(fill, [200.0, 240.0, 12.0, 70.0, 70.0]);
        assert_eq!(
            fit_blocks(&natural, &fixed, &groups, &[false, false, true, false, false], 600.0, 2.0, 24.0),
            auto
        );
        let button = |command: &str| crate::ui::skin::ToolButton {
            command: command.into(),
            icon: "box-filled".into(),
            tooltip: String::new(),
        };
        let buttons = [
            button("inventory"),
            button("people"),
            crate::ui::toolbar::structure::separator("scope"),
            button("worldmap"),
            button("build"),
        ];
        assert_eq!(
            block_widths(&buttons, &[40.0, 80.0, 12.0, 24.0, 24.0], 24.0, &groups),
            [80.0, 80.0, 24.0, 24.0, 24.0]
        );
        groups[0].sizing = Sizing::Auto;
        assert_eq!(
            block_widths(&buttons, &[40.0, 80.0, 12.0, 24.0, 24.0], 24.0, &groups),
            [40.0, 80.0, 24.0, 24.0, 24.0]
        );
    }

    #[test]
    fn separators_and_live_blocks_keep_intrinsic_sizes_in_every_mode() {
        for sizing in [Sizing::Auto, Sizing::Fixed, Sizing::Fill] {
            let widths = fit_fixed(
                &[40.0, 12.0, 220.0, 80.0],
                &[false, true, true, false],
                sizing,
                true,
                600.0,
                2.0,
                28.0,
            );
            assert_eq!(widths[1..3], [12.0, 220.0]);
            assert!(widths.iter().sum::<f32>() + 6.0 <= 600.0);
        }
        let crowded = fit_fixed(
            &[40.0, 12.0, 220.0, 80.0],
            &[false, true, true, false],
            Sizing::Fill,
            true,
            100.0,
            2.0,
            28.0,
        );
        assert_eq!(crowded, [28.0, 12.0, 220.0, 28.0]);
    }

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
