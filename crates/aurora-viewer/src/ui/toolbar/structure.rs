//! Movable structural toolbar entries. Each gap has its own identity, unlike
//! commands, so the catalogue can supply any number of independent separators.

use crate::ui::skin::ToolButton;

pub const SEPARATOR: &str = "toolbar_separator";
pub const PREFIX: &str = "separator:";
/// Leaves an 8 px glass break after the neighbouring groups' padding.
pub const GAP: f32 = 12.0;

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
    }
    button
}

pub fn is_block(command: &str) -> bool {
    matches!(
        command,
        "bar_navigation" | "bar_account" | "bar_system" | "bar_notifications" | "bar_conversations"
    )
}

pub fn fixed_width(command: &str) -> bool {
    is_separator(command) || is_block(command)
}

#[derive(Clone)]
pub struct Display {
    original: Vec<ToolButton>,
    visible: Vec<ToolButton>,
    indices: Vec<usize>,
    editing: bool,
    visibility: [bool; 5],
}

impl Display {
    /// Cache the derived visible list: hidden live blocks and redundant breaks
    /// do not leave holes, while edit mode exposes every saved separator.
    pub fn for_bar(ctx: &egui::Context, bar: super::Bar, buttons: &[ToolButton], editing: bool) -> std::sync::Arc<Self> {
        let commands = [
            "bar_navigation",
            "bar_account",
            "bar_system",
            "bar_notifications",
            "bar_conversations",
        ];
        let visibility = commands.map(|command| crate::ui::bars::chrome::width(ctx, command) != Some(0.0));
        let id = egui::Id::unique(("toolbar_display", bar));
        if let Some(cached) = ctx.data(|data| data.get_temp::<std::sync::Arc<Self>>(id))
            && cached.original == buttons
            && cached.editing == editing
            && cached.visibility == visibility
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
            if !editing && is_separator(&button.command) {
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
            original: buttons.to_vec(),
            visible,
            indices,
            editing,
            visibility,
        });
        ctx.data_mut(|data| data.insert_temp(id, display.clone()));
        display
    }

    pub fn buttons(&self) -> &[ToolButton] {
        &self.visible
    }

    pub fn full_rects<'a>(&self, rects: &'a [egui::Rect]) -> std::borrow::Cow<'a, [egui::Rect]> {
        if self.indices.len() == self.original.len() {
            return std::borrow::Cow::Borrowed(rects);
        }
        let mut full = vec![egui::Rect::NOTHING; self.original.len()];
        for (&index, &rect) in self.indices.iter().zip(rects) {
            full[index] = rect;
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
        button("bar_conversations").into_iter().collect(),
        button("bar_account").into_iter().collect(),
        button("bar_system").into_iter().collect(),
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
                rect.min.x = outer.left();
                rect.max.x = outer.right();
                rect = rect.expand2(egui::vec2(0.0, 4.0));
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
            let rects = [rect(4.0, 28.0), rect(34.0, GAP), rect(48.0, 28.0)];
            let groups = groups(
                &buttons,
                &rects,
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0)),
                vertical,
            );
            assert_eq!(groups.len(), 2);
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
