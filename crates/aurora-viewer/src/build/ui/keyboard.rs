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

    /// Whether the shortcut keeps its key from the gestures. Menu
    /// accelerators go before gestures, but a disabled menu item lets its
    /// key through (LLViewerWindow::handleKey, LLMenuItemCallGL::
    /// handleAcceleratorKey): Undo / Redo only take Ctrl+Z / Ctrl+Y while
    /// there is an edit to undo, here while the build tools are open.
    pub fn takes_gesture_key(self, open: bool) -> bool {
        open || !matches!(self, Self::History(_))
    }

    /// The build shortcut that keeps a gesture's key (Second Life KEY and
    /// MASK) from playing it in the world: `always`, or only while the
    /// build tools are open. For the gesture editor's warning.
    pub fn for_gesture_key(key: u8, mask: u32, open: bool) -> Option<Self> {
        let name = match key {
            0x86 => "Escape".to_owned(),
            0x88 => "Delete".to_owned(),
            // a press always gives the capital letter (keys::from_event)
            b'a'..=b'z' => return None,
            33..=126 => char::from(key).to_string(),
            _ => return None,
        };
        let mods = Mods {
            ctrl: mask == 1,
            shift: mask == 4,
            alt: false,
        };
        Self::from_key(Key::from_name(&name)?, mods, open).filter(|s| s.takes_gesture_key(open))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gestures::keys;

    /// Every gesture shortcut the editor offers that a build shortcut keeps,
    /// as the Legacy Viewer's menus keep them (#915): Undo / Redo only while
    /// the build tools are open, Build and its tools always.
    #[test]
    fn the_build_shortcuts_keep_these_gesture_keys() {
        let taken = |open: bool| -> Vec<String> {
            keys::choices()
                .flat_map(|key| [0, 4, 1].map(|mask| (key, mask)))
                .filter(|&(key, mask)| Shortcut::for_gesture_key(key, mask, open).is_some())
                .map(|(key, mask)| keys::shortcut(key, mask))
                .collect()
        };
        let ctrl = |key: &str| format!("{}{key}", keys::shortcut_prefix(1));
        assert_eq!(taken(false), [ctrl("1"), ctrl("2"), ctrl("3"), ctrl("4"), ctrl("5"), ctrl("B")]);
        let mut building = taken(true);
        building.sort();
        let shift = aurora_i18n::tr!("keybinds-shift-2");
        let mut expected: Vec<String> = [
            ctrl(","),
            ctrl("."),
            ctrl("1"),
            ctrl("2"),
            ctrl("3"),
            ctrl("4"),
            ctrl("5"),
            ctrl("B"),
            ctrl("D"),
            ctrl("L"),
            ctrl("Y"),
            ctrl("Z"),
            "G".into(),
        ]
        .into_iter()
        .chain([format!("{shift}G"), format!("{shift}X"), keys::label(0x86), keys::label(0x88)])
        .collect();
        expected.sort();
        assert_eq!(building, expected);
    }

    /// The real key path agrees: Ctrl+Z is an editing shortcut in any
    /// layout, but it keeps its key from the gestures only while the build
    /// tools are open; Ctrl+B always opens them.
    #[test]
    fn undo_leaves_its_key_to_the_gestures_while_the_build_tools_are_closed() {
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        for (letter, code) in [("z", KeyCode::KeyZ), ("z", KeyCode::KeyW), ("y", KeyCode::KeyY)] {
            let key = NativeKey::Character(letter.into());
            let mut down = HashSet::new();
            let shortcut = Shortcut::take_native_key(&key, code, ctrl, false, true, &mut down).expect("editing shortcut");
            assert!(matches!(shortcut, Shortcut::History(_)));
            assert!(!shortcut.takes_gesture_key(false), "Ctrl+{letter} with the build tools closed");
            assert!(shortcut.takes_gesture_key(true), "Ctrl+{letter} while building");
        }
        let mut down = HashSet::new();
        let build = Shortcut::take_native_key(&NativeKey::Character("b".into()), KeyCode::KeyB, ctrl, false, true, &mut down);
        assert_eq!(build, Some(Shortcut::ToggleBuild));
        assert!(Shortcut::ToggleBuild.takes_gesture_key(false));
    }
}
