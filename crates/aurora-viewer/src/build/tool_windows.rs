//! The Aurora style's tools in windows of their own (human request,
//! 2026-11-24): the main build window keeps the Edit tool and its
//! sections; Camera, Move, Create and Land each open a window of their
//! own, from the main window's rail, their toolbar buttons, their menu
//! entries and Ctrl+1, 2, 4, 5. Several can be open together: the tool in
//! use is the one of the window last opened or pressed (the main window's
//! is Edit). The build tools stay open while any of these windows is.
//!
//! The Legacy style keeps the Legacy Viewer's single window
//! (LLFloaterTools), whose tool buttons switch its panel.

use super::{BuildSettings, BuildTool, Tool};
use crate::world::World;

impl BuildTool {
    /// Whether the window of `tool` (Camera, Move, Create, Land) is shown.
    pub fn tool_window_open(&self, tool: Tool) -> bool {
        self.tool_windows.contains(&tool)
    }

    /// The tool windows shown, in the order they opened.
    /// The Create window's shape picker waits for a world click.
    pub fn capturing_shape(&self) -> bool {
        self.shape_capture && self.open && !self.ui.legacy() && self.tool_windows.contains(&Tool::Create)
    }

    pub fn tool_windows(&self) -> &[Tool] {
        &self.tool_windows
    }

    /// The tool the main window shows, while it is shown: the Aurora
    /// style's is always Edit (the others have their own windows).
    pub fn main_tool(&self) -> Option<Tool> {
        if !self.open || !self.main_open {
            None
        } else if self.ui.legacy() {
            Some(self.tool)
        } else {
            Some(Tool::Edit)
        }
    }

    /// The tool of a window just pressed becomes the one in use.
    pub fn activate(&mut self, tool: Tool) {
        if self.tool != tool {
            self.set_tool(tool);
        }
    }

    /// The tool in use once its window closed: the window opened last, but
    /// never Create, which comes back only from its shapes (it puts a shape
    /// under the pointer to rez: human request, 2026-11-24); else Edit.
    fn fallback_tool(&self) -> Tool {
        self.tool_windows
            .iter()
            .rev()
            .copied()
            .find(|t| *t != Tool::Create)
            .unwrap_or(Tool::Edit)
    }

    /// Create no longer the tool in use, its window staying open (its
    /// chosen shape clicked again): the tool goes to `fallback_tool`.
    pub fn deactivate_create(&mut self) {
        if self.tool == Tool::Create {
            self.set_tool(self.fallback_tool());
        }
    }

    /// Closes the window of `tool`. The tool in use then goes to
    /// `fallback_tool`; with no window left the build tools close
    /// (LLFloaterTools::onClose).
    pub fn close_tool_window(&mut self, tool: Tool, world: &mut World, s: &mut BuildSettings) {
        self.tool_windows.retain(|t| *t != tool);
        if tool == Tool::Create {
            self.shape_capture = false;
        }
        if !self.main_open && self.tool_windows.is_empty() {
            self.close_all(world, s);
        } else if self.tool == tool {
            self.set_tool(self.fallback_tool());
        }
    }

    /// Closes the Aurora main window; the tool windows stay, the last one
    /// opened (Create aside: `fallback_tool`) taking over from Edit.
    pub fn close_main(&mut self, world: &mut World, s: &mut BuildSettings) {
        if self.ui.legacy() || self.tool_windows.is_empty() {
            self.close_all(world, s);
            return;
        }
        self.main_open = false;
        if self.tool == Tool::Edit {
            let next = self.fallback_tool();
            if next != Tool::Edit {
                self.set_tool(next);
            }
        }
    }

    /// The whole build tools closed; a close the replacement holds keeps
    /// the main window, where its notice shows.
    fn close_all(&mut self, world: &mut World, s: &mut BuildSettings) {
        self.close(world, s);
        if self.open {
            self.main_open = true;
        }
    }

    /// What Build and its tools' toolbar buttons, menu entries and
    /// shortcuts do. `mode` `None` is Build (Ctrl+B), which opens or
    /// closes the main window on Edit. `toggle` is a toolbar button's or
    /// Build's click: lit, it closes its window.
    ///
    /// Aurora style: Camera, Move, Create and Land open or close their own
    /// window from every path (Ctrl+1, 2, 4, 5 too); Edit's menu entry and
    /// Ctrl+3 open the main window on Edit, give Edit back while another
    /// tool is in use, and close the window when it shows Edit (human
    /// request, 2026-11-26).
    ///
    /// Legacy style, its single window as the Legacy Viewer has it: Build
    /// opens it on Create (LLToolMgr::enterBuildMode) or closes it; a lit
    /// tool button closes it; a menu entry or shortcut selects the tool
    /// (Tools.SelectTool).
    ///
    /// `can_build` is false under RLVa's @edit and @rez
    /// (LLToolMgr::buildEnabledOrActive): the tools do not open, open ones
    /// still change and close.
    pub fn bar_click(&mut self, mode: Option<Tool>, toggle: bool, can_build: bool, world: &mut World, s: &mut BuildSettings) {
        let allowed = self.open || can_build;
        if self.ui.legacy() {
            match mode {
                None if self.open => self.close(world, s),
                Some(tool) if toggle && self.lit(Some(tool)) => self.close(world, s),
                _ if !allowed => {}
                None => self.open_build(Tool::Create),
                Some(tool) => self.open_build(tool),
            }
            return;
        }
        match mode {
            None | Some(Tool::Edit) if self.lit(mode) && (toggle || mode.is_none() || self.tool == Tool::Edit) => self.close_main(world, s),
            Some(tool) if tool != Tool::Edit && self.tool_window_open(tool) => self.close_tool_window(tool, world, s),
            _ if !allowed => {}
            None => self.open_build(Tool::Edit),
            Some(tool) => self.open_build(tool),
        }
    }

    /// Whether the button of Build (`None`) or of a tool lights: in the
    /// Aurora style while its window is shown (Build and Edit: the main
    /// window), in the Legacy style while the window shows the tool.
    pub fn lit(&self, mode: Option<Tool>) -> bool {
        if !self.open {
            return false;
        }
        match mode {
            None => self.ui.legacy() || self.main_open,
            Some(tool) if self.ui.legacy() => self.tool == tool,
            Some(Tool::Edit) => self.main_open,
            Some(tool) => self.tool_window_open(tool),
        }
    }
}
