//! Build and its modes' toolbar buttons (Camera, Move, Edit objects, Create
//! prims, Edit land): which tool a click opens, and when it closes the
//! window.

use super::*;
use std::sync::Arc;

fn world() -> World {
    World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()))
}

/// A click as the app runs it: open or switch to the tool, or close.
fn click(tool: &mut BuildTool, world: &mut World, settings: &mut BuildSettings, mode: Option<Tool>, button: bool) {
    match tool.bar_click(mode, button) {
        Some(t) => tool.open_build(t),
        None => tool.close(world, settings),
    }
}

#[test]
fn build_first_opens_on_create_then_reopens_on_the_tool_last_shown() {
    let (mut world, mut settings) = (world(), BuildSettings::default());
    let mut tool = BuildTool::default();
    // LLToolMgr::enterBuildMode: the first opening is on Create
    click(&mut tool, &mut world, &mut settings, None, true);
    assert!(tool.open);
    assert_eq!(tool.shown_tool(), Some(Tool::Create));
    // the window switches to Edit land, then Build closes it
    click(&mut tool, &mut world, &mut settings, Some(Tool::Land), true);
    assert_eq!(tool.shown_tool(), Some(Tool::Land));
    click(&mut tool, &mut world, &mut settings, None, true);
    assert!(!tool.open);
    assert_eq!(tool.shown_tool(), None);
    // Build reopens on Edit land, not on Create
    click(&mut tool, &mut world, &mut settings, None, true);
    assert_eq!(tool.shown_tool(), Some(Tool::Land));
    // the same after the Edit tool, chosen from another path
    tool.set_tool(Tool::Edit);
    tool.close(&mut world, &mut settings);
    click(&mut tool, &mut world, &mut settings, None, false);
    assert_eq!(tool.shown_tool(), Some(Tool::Edit));
}

#[test]
fn a_lit_mode_button_closes_the_window_and_another_switches_it() {
    let (mut world, mut settings) = (world(), BuildSettings::default());
    let mut tool = BuildTool::default();
    for mode in [Tool::Focus, Tool::Grab, Tool::Edit, Tool::Create, Tool::Land] {
        // closed: the button opens on its tool
        click(&mut tool, &mut world, &mut settings, Some(mode), true);
        assert_eq!(tool.shown_tool(), Some(mode), "{mode:?}");
        // another mode's button switches the window, it does not close it
        let other = if mode == Tool::Edit { Tool::Create } else { Tool::Edit };
        click(&mut tool, &mut world, &mut settings, Some(other), true);
        assert_eq!(tool.shown_tool(), Some(other), "{mode:?}");
        click(&mut tool, &mut world, &mut settings, Some(mode), true);
        // lit: the same button again closes the window
        click(&mut tool, &mut world, &mut settings, Some(mode), true);
        assert!(!tool.open, "{mode:?}");
        // and Build reopens on that mode
        click(&mut tool, &mut world, &mut settings, None, true);
        assert_eq!(tool.shown_tool(), Some(mode), "{mode:?}");
        tool.close(&mut world, &mut settings);
    }
}

#[test]
fn a_mode_menu_entry_or_shortcut_only_selects_its_tool() {
    let (mut world, mut settings) = (world(), BuildSettings::default());
    let mut tool = BuildTool::default();
    // Tools.SelectTool (Ctrl+3, Ctrl+4, Ctrl+5): never a toggle
    click(&mut tool, &mut world, &mut settings, Some(Tool::Land), false);
    click(&mut tool, &mut world, &mut settings, Some(Tool::Land), false);
    assert_eq!(tool.shown_tool(), Some(Tool::Land));
    // Build in the menu (Ctrl+B) still toggles
    click(&mut tool, &mut world, &mut settings, None, false);
    assert!(!tool.open);
}
