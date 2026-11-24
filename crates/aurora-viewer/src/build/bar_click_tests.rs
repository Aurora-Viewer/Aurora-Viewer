//! Build and its tools' buttons, menu entries and shortcuts: which windows
//! they open or close, the tool in use and the lit buttons, in the Aurora
//! style (a window per tool) and the Legacy style (one window).

use super::*;
use std::sync::Arc;

struct Bench {
    tool: BuildTool,
    world: World,
    settings: BuildSettings,
}

impl Bench {
    fn new(legacy: bool) -> Self {
        let mut tool = BuildTool::default();
        tool.ui.set_legacy(legacy);
        Self {
            tool,
            world: World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load())),
            settings: BuildSettings::default(),
        }
    }

    /// A toolbar button (or Build, Ctrl+B): `toggle`.
    fn button(&mut self, mode: Option<Tool>) {
        self.tool.bar_click(mode, true, true, &mut self.world, &mut self.settings);
    }

    /// A menu entry or its shortcut (Ctrl+1 to Ctrl+5).
    fn key(&mut self, mode: Tool) {
        self.tool.bar_click(Some(mode), false, true, &mut self.world, &mut self.settings);
    }

    fn lit(&self) -> Vec<Option<Tool>> {
        [
            None,
            Some(Tool::Focus),
            Some(Tool::Grab),
            Some(Tool::Edit),
            Some(Tool::Create),
            Some(Tool::Land),
        ]
        .into_iter()
        .filter(|mode| self.tool.lit(*mode))
        .collect()
    }
}

#[test]
fn aurora_build_opens_and_closes_the_main_window_on_edit() {
    let mut b = Bench::new(false);
    b.button(None);
    assert!(b.tool.open && b.tool.main_open);
    assert_eq!(b.tool.tool, Tool::Edit);
    assert_eq!(b.tool.main_tool(), Some(Tool::Edit));
    assert!(b.tool.tool_windows().is_empty());
    assert_eq!(b.lit(), [None, Some(Tool::Edit)]);
    b.button(None);
    assert!(!b.tool.open && !b.tool.main_open);
    assert!(b.lit().is_empty());
    // Build - Edit's button does the same; Ctrl+3 opens and stays
    b.button(Some(Tool::Edit));
    assert!(b.tool.main_open);
    b.key(Tool::Edit);
    assert!(b.tool.main_open);
    b.button(Some(Tool::Edit));
    assert!(!b.tool.open);
}

#[test]
fn aurora_each_tool_opens_and_closes_its_own_window_alone() {
    for which in [Tool::Focus, Tool::Grab, Tool::Create, Tool::Land] {
        let mut b = Bench::new(false);
        b.button(Some(which));
        assert!(b.tool.open, "{which:?}");
        assert!(!b.tool.main_open, "{which:?}: the main window stays closed");
        assert_eq!(b.tool.tool_windows(), [which]);
        assert_eq!(b.tool.tool, which);
        assert_eq!(b.tool.main_tool(), None);
        assert_eq!(b.lit(), [Some(which)], "{which:?}");
        // lit: the button closes it, and the build tools with it
        b.button(Some(which));
        assert!(!b.tool.open, "{which:?}");
        assert!(b.lit().is_empty());
        // its shortcut (Ctrl+1, 2, 4, 5) and menu entry also open and close it
        b.key(which);
        assert_eq!(b.tool.tool_windows(), [which]);
        b.key(which);
        assert!(!b.tool.open, "{which:?}");
    }
}

#[test]
fn aurora_tool_windows_stay_open_together_and_the_last_one_holds_the_tool() {
    let mut b = Bench::new(false);
    b.button(None);
    b.button(Some(Tool::Create));
    b.button(Some(Tool::Land));
    assert_eq!(b.tool.tool_windows(), [Tool::Create, Tool::Land]);
    assert_eq!(b.tool.tool, Tool::Land, "the window opened last");
    assert_eq!(b.lit(), [None, Some(Tool::Edit), Some(Tool::Create), Some(Tool::Land)]);
    // a press in a window: its tool is the one in use, the windows stay
    b.tool.activate(Tool::Create);
    assert_eq!(b.tool.tool, Tool::Create);
    b.tool.activate(Tool::Edit);
    assert_eq!(b.tool.tool, Tool::Edit);
    assert_eq!(b.tool.tool_windows(), [Tool::Create, Tool::Land]);
    // closing the window of the tool in use gives it to the last one left
    b.tool.activate(Tool::Land);
    b.button(Some(Tool::Land));
    assert_eq!(b.tool.tool, Tool::Create);
    // then to Edit, the main window's
    b.button(Some(Tool::Create));
    assert_eq!(b.tool.tool, Tool::Edit);
    assert!(b.tool.open && b.tool.main_open);
}

#[test]
fn aurora_closing_the_main_window_keeps_the_tool_windows() {
    let mut b = Bench::new(false);
    b.button(None);
    b.button(Some(Tool::Focus));
    b.tool.activate(Tool::Edit);
    // its cross, Build or Build - Edit
    b.tool.close_main(&mut b.world, &mut b.settings);
    assert!(b.tool.open && !b.tool.main_open);
    assert_eq!(b.tool.tool, Tool::Focus, "the tool windows take over from Edit");
    assert_eq!(b.tool.main_tool(), None);
    assert_eq!(b.lit(), [Some(Tool::Focus)]);
    // Build brings it back on Edit; the camera window stays
    b.button(None);
    assert!(b.tool.main_open);
    assert_eq!(b.tool.tool, Tool::Edit);
    assert_eq!(b.tool.tool_windows(), [Tool::Focus]);
    // the last window closed: the build tools close
    b.button(None);
    b.button(Some(Tool::Focus));
    assert!(!b.tool.open);
}

#[test]
fn aurora_under_rlva_no_window_opens_and_an_open_one_still_closes() {
    let mut b = Bench::new(false);
    b.tool.bar_click(Some(Tool::Create), true, false, &mut b.world, &mut b.settings);
    b.tool.bar_click(None, true, false, &mut b.world, &mut b.settings);
    assert!(!b.tool.open);
    b.button(Some(Tool::Land));
    b.tool.bar_click(Some(Tool::Land), true, false, &mut b.world, &mut b.settings);
    assert!(!b.tool.open);
}

#[test]
fn legacy_keeps_its_single_window() {
    let mut b = Bench::new(true);
    // LLToolMgr::enterBuildMode: Build opens it on Create
    b.button(None);
    assert!(b.tool.open && b.tool.main_open);
    assert_eq!(b.tool.tool, Tool::Create);
    assert!(b.tool.tool_windows().is_empty());
    assert_eq!(b.tool.main_tool(), Some(Tool::Create));
    assert_eq!(b.lit(), [None, Some(Tool::Create)]);
    // another tool's button switches the window to it, no window of its own
    b.button(Some(Tool::Land));
    assert_eq!(b.tool.tool, Tool::Land);
    assert!(b.tool.tool_windows().is_empty());
    // lit: it closes the window
    b.button(Some(Tool::Land));
    assert!(!b.tool.open);
    // a menu entry or shortcut only selects its tool (Tools.SelectTool)
    b.key(Tool::Focus);
    b.key(Tool::Focus);
    assert!(b.tool.open);
    assert_eq!(b.tool.tool, Tool::Focus);
    b.button(None);
    assert!(!b.tool.open);
}
