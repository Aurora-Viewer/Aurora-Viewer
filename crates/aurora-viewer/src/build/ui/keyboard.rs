//! One command table for native input ownership and egui build actions.
//! LLViewerInput::scanKey (indra/newview/llviewerinput.cpp) leaves keys
//! handled by menu_viewer.xml accelerators out of held movement controls.

use super::{BuildSettings, BuildTool, Tool, World};
use crate::keybinds::{EditHistory, Input, Mods};
use egui::Key;
use glam::Vec3;
use std::collections::HashSet;
use winit::keyboard::{Key as NativeKey, KeyCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    History(EditHistory),
    ToggleBuild,
    Open(Tool),
    GridOptions,
    Close,
    Delete,
    Duplicate,
    Link,
    Unlink,
    EditLinked,
    Snap,
    SelectionGrid,
    SnapXy,
    SelectPart { next: bool, include: bool },
}

impl Shortcut {
    pub fn from_key(key: Key, mods: Mods, open: bool) -> Option<Self> {
        if let Some(history) = EditHistory::from_key(key, mods) {
            return Some(Self::History(history));
        }
        if mods.alt {
            return None;
        }
        Some(match (key, mods.ctrl, mods.shift) {
            (Key::B, true, false) => Self::ToggleBuild,
            (Key::Num1, true, false) => Self::Open(Tool::Focus),
            (Key::Num2, true, false) => Self::Open(Tool::Grab),
            (Key::Num3, true, false) => Self::Open(Tool::Edit),
            (Key::Num4, true, false) => Self::Open(Tool::Create),
            (Key::Num5, true, false) => Self::Open(Tool::Land),
            (Key::B, true, true) => Self::GridOptions,
            _ if !open => return None,
            (Key::Escape, false, false) => Self::Close,
            (Key::Delete, false, false) => Self::Delete,
            (Key::D, true, false) => Self::Duplicate,
            (Key::L, true, false) => Self::Link,
            (Key::L, true, true) => Self::Unlink,
            (Key::E, true, true) => Self::EditLinked,
            (Key::G, false, false) => Self::Snap,
            (Key::G, false, true) => Self::SelectionGrid,
            (Key::X, false, true) => Self::SnapXy,
            (Key::Period, true, include) => Self::SelectPart { next: true, include },
            (Key::Comma, true, include) => Self::SelectPart { next: false, include },
            _ => return None,
        })
    }

    /// Consume the physical hold before movement/gestures see the press.
    /// Text/media still receive their native events; only history remains
    /// reserved when another UI control owns the keyboard.
    pub fn take_native_key(
        key: &NativeKey,
        code: KeyCode,
        mods: Mods,
        open: bool,
        build_keyboard: bool,
        down: &mut HashSet<Input>,
    ) -> Option<Self> {
        let key = crate::keybinds::native_shortcut_key(key, code)?;
        let command = if build_keyboard {
            Self::from_key(key, mods, open)?
        } else {
            Self::History(EditHistory::from_key(key, mods)?)
        };
        down.remove(&Input::key(code));
        Some(command)
    }

    pub(super) fn run(self, tool: &mut BuildTool, world: &mut World, settings: &mut BuildSettings) {
        match self {
            Self::History(_) if !tool.open => {}
            Self::History(history) => {
                // LLToolBrushLand has UndoLand only: redo must not reach
                // an object selection left over from another tool.
                if tool.tool == Tool::Land {
                    if history == EditHistory::Undo {
                        let mut commands = Vec::new();
                        tool.land.undo(&mut commands);
                        for command in commands {
                            tool.send(command);
                        }
                    }
                } else {
                    tool.undo(world, history == EditHistory::Redo);
                }
            }
            Self::ToggleBuild => {
                if tool.open {
                    tool.close(world, settings);
                } else {
                    tool.open_build(Tool::Create);
                }
            }
            Self::Open(which) => tool.open_build(which),
            Self::GridOptions => tool.ui.grid_options_open = !tool.ui.grid_options_open,
            Self::Close => tool.close(world, settings),
            Self::Delete => tool.delete(world),
            Self::Duplicate => tool.duplicate(world, Vec3::new(0.5, 0.5, 0.0), true),
            Self::Link => tool.link(world),
            Self::Unlink => tool.unlink(world),
            Self::EditLinked => {
                let on = !settings.edit_linked;
                tool.set_edit_linked(world, settings, on);
            }
            Self::Snap => settings.snap = !settings.snap,
            Self::SelectionGrid => tool.use_selection_for_grid(world, settings),
            Self::SnapXy => tool.snap_xy_to_grid(world, settings),
            Self::SelectPart { next, include } => tool.select_next_part(world, settings, next, include),
        }
    }
}
