//! Movable structural toolbar entries. Each gap has its own identity, unlike
//! commands, so the catalogue can supply any number of independent separators.

use crate::ui::skin::ToolButton;

pub const SEPARATOR: &str = "toolbar_separator";
pub const PREFIX: &str = "separator:";
pub const DIVIDER: &str = "toolbar_divider";
pub const DIVIDER_PREFIX: &str = "divider:";
pub const DIVIDER_SIZE: f32 = 12.0;
/// Leaves an 8 px glass break after the neighbouring groups' padding.
pub const GAP: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SeparatorStyle {
    pub size: f32,
    pub stretch: bool,
}

impl Default for SeparatorStyle {
    fn default() -> Self {
        Self { size: GAP, stretch: false }
    }
}

impl SeparatorStyle {
    pub const MIN: f32 = 4.0;
    pub const MAX: f32 = 1024.0;

    pub fn sanitized(self) -> Self {
        Self {
            size: if self.size.is_finite() {
                self.size.clamp(Self::MIN, Self::MAX)
            } else {
                GAP
            },
            ..self
        }
    }
}

pub fn style(ctx: &egui::Context, command: &str) -> SeparatorStyle {
    super::appearance::settings(ctx)
        .separators
        .get(command)
        .copied()
        .unwrap_or_default()
        .sanitized()
}

pub fn stretch(ctx: &egui::Context, buttons: &[ToolButton]) -> Vec<bool> {
    let settings = super::appearance::settings(ctx);
    buttons
        .iter()
        .map(|button| is_separator(&button.command) && settings.separators.get(&button.command).is_some_and(|style| style.stretch))
        .collect()
}

/// Human-requested extension to LLToolBar::updateLayoutAsNeeded
/// (indra/llui/lltoolbar.cpp): elastic gaps take surplus before button Fill
/// or the navigation block. They never steal minimum space from controls.
pub fn distribute(sizes: &mut [f32], stretch: &[bool], room: f32, gap: f32) {
    let count = stretch.iter().take(sizes.len()).filter(|flag| **flag).count();
    if count == 0 || !room.is_finite() {
        return;
    }
    let extra = (room - sizes.iter().sum::<f32>() - gap * sizes.len().saturating_sub(1) as f32).max(0.0);
    let share = extra / count as f32;
    for (size, stretch) in sizes.iter_mut().zip(stretch) {
        if *stretch {
            *size += share;
        }
    }
}

/// Includes the two old aggregate IDs so saved layouts can still be read.
pub const BLOCKS: [&str; 15] = [
    "bar_navigation",
    "bar_account",
    "bar_system",
    "bar_notifications",
    "bar_conversations",
    "bar_status",
    "bar_balance",
    "bar_clock",
    "bar_ping",
    "bar_fps",
    "bar_network",
    "bar_media",
    "bar_music",
    "bar_volume",
    "bar_chat",
];

pub fn is_separator(command: &str) -> bool {
    command == SEPARATOR || command.starts_with(PREFIX)
}

pub fn separator(id: &str) -> ToolButton {
    ToolButton {
        command: format!("{PREFIX}{id}"),
        icon: "separator-vertical".into(),
        tooltip: String::new(),
    }
}

pub fn instantiate(mut button: ToolButton) -> ToolButton {
    if button.command == SEPARATOR {
        button.command = format!("{PREFIX}{}", uuid::Uuid::new_v4());
    } else if button.command == DIVIDER {
        button.command = format!("{DIVIDER_PREFIX}{}", uuid::Uuid::new_v4());
    }
    button
}

pub fn is_block(command: &str) -> bool {
    BLOCKS.contains(&command)
}

pub fn is_divider(command: &str) -> bool {
    command == DIVIDER || command.starts_with(DIVIDER_PREFIX)
}

/// Gaps that follow one another between two blocks are one (human request,
/// 2026-11-25): the first stays, with its look. The others' ids are returned.
pub fn single_gaps(buttons: &mut Vec<ToolButton>) -> Vec<String> {
    let mut removed = Vec::new();
    let mut after_gap = false;
    buttons.retain(|button| {
        let gap = is_separator(&button.command);
        if gap && after_gap {
            removed.push(button.command.clone());
            return false;
        }
        after_gap = gap;
        true
    });
    removed
}

pub fn divider(id: &str) -> ToolButton {
    ToolButton {
        command: format!("{DIVIDER_PREFIX}{id}"),
        icon: "separator-vertical".into(),
        tooltip: String::new(),
    }
}

pub fn fixed_width(command: &str) -> bool {
    is_separator(command) || is_divider(command) || is_block(command)
}

#[derive(Clone)]
pub struct Display {
    original: Vec<ToolButton>,
    drawn: Vec<ToolButton>,
    visible: Vec<ToolButton>,
    indices: Vec<usize>,
    editing: bool,
    visibility: [bool; BLOCKS.len()],
    separators: std::collections::BTreeMap<String, SeparatorStyle>,
}

impl Display {
    /// Cache the derived visible list: hidden live blocks and redundant breaks
    /// do not leave holes, while edit mode exposes every saved separator.
    pub fn for_bar(ctx: &egui::Context, bar: super::Bar, buttons: &[ToolButton], editing: bool) -> std::sync::Arc<Self> {
        Self::derive(ctx, bar, buttons, editing, true)
    }

    pub fn for_drawer(ctx: &egui::Context, bar: super::Bar, buttons: &[ToolButton], editing: bool) -> std::sync::Arc<Self> {
        Self::derive(ctx, bar, buttons, editing, false)
    }

    fn derive(ctx: &egui::Context, bar: super::Bar, original: &[ToolButton], editing: bool, root: bool) -> std::sync::Arc<Self> {
        let shown = root.then(|| super::drop_preview::shown_buttons(ctx, bar)).flatten();
        let buttons = shown.as_deref().unwrap_or(original);
        let commands = BLOCKS;
        let visibility = commands.map(|command| crate::ui::bars::chrome::width(ctx, command) != Some(0.0));
        let settings = super::appearance::settings(ctx);
        let id = egui::Id::unique(("toolbar_display", bar, root));
        if let Some(cached) = ctx.data(|data| data.get_temp::<std::sync::Arc<Self>>(id))
            && cached.original == original
            && cached.drawn == buttons
            && cached.editing == editing
            && cached.visibility == visibility
            && cached.separators == settings.separators
        {
            return cached;
        }
        let mut indices = Vec::new();
        let mut gap = None;
        for (index, button) in buttons.iter().enumerate() {
            if !editing
                && commands
                    .iter()
                    .position(|command| *command == button.command)
                    .is_some_and(|i| !visibility[i])
            {
                continue;
            }
            if !editing
                && is_separator(&button.command)
                && settings
                    .separators
                    .get(&button.command)
                    .is_none_or(|style| *style == SeparatorStyle::default())
            {
                if !indices.is_empty() && gap.is_none() {
                    gap = Some(index);
                }
                continue;
            }
            if let Some(gap) = gap.take() {
                indices.push(gap);
            }
            indices.push(index);
        }
        let visible = indices.iter().map(|&index| buttons[index].clone()).collect();
        let display = std::sync::Arc::new(Self {
            original: original.to_vec(),
            drawn: buttons.to_vec(),
            visible,
            indices,
            editing,
            visibility,
            separators: settings.separators.clone(),
        });
        ctx.data_mut(|data| data.insert_temp(id, display.clone()));
        display
    }

    pub fn buttons(&self) -> &[ToolButton] {
        &self.visible
    }

    pub fn original(&self) -> &[ToolButton] {
        &self.original
    }

    pub fn full_rects<'a>(&self, rects: &'a [egui::Rect]) -> std::borrow::Cow<'a, [egui::Rect]> {
        if self.drawn == self.original && self.indices.len() == self.original.len() {
            return std::borrow::Cow::Borrowed(rects);
        }
        let mut full = vec![egui::Rect::NOTHING; self.original.len()];
        for (&index, &rect) in self.indices.iter().zip(rects) {
            if let Some(original) = self.original.iter().position(|button| button.command == self.drawn[index].command) {
                full[original] = rect;
            }
        }
        std::borrow::Cow::Owned(full)
    }
}

pub fn top_buttons(existing: &[ToolButton]) -> Vec<ToolButton> {
    let button = |command: &str| {
        let command = super::catalogue::find(command);
        command
            .filter(|c| !existing.iter().any(|button| button.command == c.id))
            .map(|c| ToolButton {
                command: c.id.into(),
                icon: c.icon.into(),
                tooltip: String::new(),
            })
    };
    let mut result = Vec::new();
    for (index, group) in [
        button("aurora_menu").into_iter().collect::<Vec<_>>(),
        button("report_bug").into_iter().collect(),
        existing.to_vec(),
        button("bar_navigation").into_iter().collect(),
        [
            "bar_conversations",
            "bar_status",
            "bar_balance",
            "bar_clock",
            "bar_ping",
            "bar_fps",
            "bar_network",
        ]
        .into_iter()
        .filter_map(button)
        .collect(),
        ["bar_media", "bar_music", "bar_volume"].into_iter().filter_map(button).collect(),
        button("bar_notifications").into_iter().collect(),
    ]
    .into_iter()
    .enumerate()
    {
        if group.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push(separator(&format!("top-{index}")));
        }
        result.extend(group);
    }
    result
}

/// Bound each visible group by its actual controls; a gap cuts the shared glass
/// surface. Empty leading/trailing gaps retain their drag targets without faces.
pub fn groups(buttons: &[ToolButton], rects: &[egui::Rect], outer: egui::Rect, vertical: bool) -> Vec<egui::Rect> {
    let mut result = Vec::new();
    let mut current: Option<egui::Rect> = None;
    let finish = |current: &mut Option<egui::Rect>, result: &mut Vec<egui::Rect>| {
        if let Some(mut rect) = current.take() {
            if vertical {
                // A labelled neighbour must not widen an icon-only block.
                rect = rect.expand(4.0);
            } else {
                rect.min.y = outer.top();
                rect.max.y = outer.bottom();
                rect = rect.expand2(egui::vec2(4.0, 0.0));
            }
            result.push(rect.intersect(outer));
        }
    };
    for (button, rect) in buttons.iter().zip(rects) {
        if is_separator(&button.command) {
            finish(&mut current, &mut result);
        } else if rect.is_positive() {
            current = Some(current.map_or(*rect, |previous| previous.union(*rect)));
        }
    }
    finish(&mut current, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elastic_gaps_share_only_surplus_and_keep_their_minimum_in_a_crowded_bar() {
        let flags = [false, true, false, true];
        for (room, expected) in [(400.0, [34.0, 156.0, 34.0, 170.0]), (100.0, [34.0, 12.0, 34.0, 26.0])] {
            let mut sizes = [34.0, 12.0, 34.0, 26.0];
            distribute(&mut sizes, &flags, room, 2.0);
            assert_eq!(sizes, expected);
        }
        assert_eq!(
            SeparatorStyle {
                size: f32::NAN,
                stretch: true
            }
            .sanitized()
            .size,
            GAP
        );
        assert_eq!(
            SeparatorStyle {
                size: -100.0,
                stretch: false
            }
            .sanitized()
            .size,
            SeparatorStyle::MIN
        );
        assert_eq!(
            SeparatorStyle {
                size: f32::INFINITY,
                stretch: false
            }
            .sanitized()
            .size,
            GAP
        );
    }

    #[test]
    fn explicit_outer_gaps_remain_visible_and_style_changes_invalidate_the_display() {
        use super::super::{Bar, ToolbarSettings, tests::button};
        let ctx = egui::Context::default();
        let buttons = [separator("start"), button("inventory"), separator("end")];
        let mut settings = ToolbarSettings::default();
        super::super::appearance::publish(&ctx, &settings);
        assert_eq!(Display::for_bar(&ctx, Bar::Top, &buttons, false).buttons().len(), 1);
        settings.separators.insert(
            buttons[0].command.clone(),
            SeparatorStyle {
                stretch: true,
                ..Default::default()
            },
        );
        settings.separators.insert(
            buttons[2].command.clone(),
            SeparatorStyle {
                size: 64.0,
                ..Default::default()
            },
        );
        super::super::appearance::publish(&ctx, &settings);
        assert_eq!(Display::for_bar(&ctx, Bar::Top, &buttons, false).buttons(), buttons);
    }

    #[test]
    fn glass_groups_leave_the_same_break_on_both_axes() {
        for vertical in [false, true] {
            let buttons = [
                super::super::tests::button("inventory"),
                separator("gap"),
                super::super::tests::button("people"),
            ];
            let rect = |start, length| {
                if vertical {
                    egui::Rect::from_min_size(egui::pos2(4.0, start), egui::vec2(28.0, length))
                } else {
                    egui::Rect::from_min_size(egui::pos2(start, 4.0), egui::vec2(length, 28.0))
                }
            };
            let mut rects = [rect(4.0, 28.0), rect(34.0, GAP), rect(48.0, 28.0)];
            if vertical {
                rects[2].max.x -= 8.0;
            }
            let groups = groups(
                &buttons,
                &rects,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0)),
                vertical,
            );
            assert_eq!(groups.len(), 2);
            if vertical {
                assert_eq!([groups[0].width(), groups[1].width()], [36.0, 28.0]);
            }
            let gap = if vertical {
                groups[1].top() - groups[0].bottom()
            } else {
                groups[1].left() - groups[0].right()
            };
            assert_eq!(gap, 8.0);
        }
    }

    #[test]
    fn display_collapses_outer_and_adjacent_gaps_but_preserves_saved_drop_ranks() {
        use super::super::{Bar, tests::button};
        let ctx = egui::Context::default();
        let buttons = [
            separator("a"),
            button("inventory"),
            separator("b"),
            separator("c"),
            button("people"),
            separator("d"),
        ];
        let display = Display::for_bar(&ctx, Bar::Top, &buttons, false);
        assert_eq!(
            display.buttons().iter().map(|button| button.command.as_str()).collect::<Vec<_>>(),
            ["inventory", "separator:b", "people"]
        );
        let rects = [egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::splat(28.0)); 3];
        let full = display.full_rects(&rects);
        assert_eq!(full.len(), buttons.len());
        assert_eq!(full[0], egui::Rect::NOTHING);
        assert_eq!(full[4], rects[2]);
        assert_eq!(Display::for_bar(&ctx, Bar::Top, &buttons, true).buttons(), buttons);
    }
    #[test]
    fn catalogue_gaps_create_independent_movable_instances() {
        let template = ToolButton {
            command: SEPARATOR.into(),
            icon: "separator-vertical".into(),
            tooltip: String::new(),
        };
        let first = instantiate(template.clone());
        let second = instantiate(template);
        assert_ne!(first.command, second.command);
        assert_eq!(instantiate(first.clone()), first);
        assert!(is_separator(&second.command));
    }
}
