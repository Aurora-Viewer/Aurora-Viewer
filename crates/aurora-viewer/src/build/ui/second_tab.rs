//! The Aurora build window's second section (the human's request,
//! 2026-11-23): two of General, Object, Features, Materials and Contents
//! side by side. An Aurora control: the Legacy Viewer shows one panel at a
//! time (LLFloaterTools' tab container), and its style here keeps doing so.
//!
//! - Shift+click on a section's button shows it beside the active one, the
//!   window widened to hold both; on the second's button, or on the active
//!   one's, it closes that section and gives the window back the width
//!   chosen with one section.
//! - A click replaces the left section (on the second's button, the two
//!   swap).
//! - Narrowed below two sections' width, the window closes the second;
//!   widened past it, it opens one by itself.
use super::{BuildSettings, BuildTool, Tool, detach, layout};

/// Under this width two sections do not fit side by side: twice the build
/// window's narrowest.
pub(super) const TWO_TABS_WIDTH: f32 = 2.0 * super::workspace::MIN_SIZE.x;

/// Frames a width asked of the window may take to be in place before the
/// second section follows the width it has.
const RESIZE_FRAMES: u8 = 10;

#[derive(Default)]
pub struct State {
    /// The second section, at the right.
    pub tab: Option<usize>,
    /// The second section drawn this frame (`compact::body`).
    pub shown: Option<usize>,
    /// The last second section, opened again by a window widened.
    last: Option<usize>,
    /// The window's width with one section, given back when the second
    /// closes by Shift+click.
    single_width: Option<f32>,
    /// The window's width this frame and the last.
    width: f32,
    last_width: Option<f32>,
    /// A width asked of the window, applied after it is drawn, and the
    /// frames it still has to get there.
    request: Option<f32>,
    pending: Option<(f32, u8)>,
}

/// The sections of the build window, in their bar's order: those not in a
/// window of their own.
fn sections(s: &BuildSettings) -> Vec<usize> {
    s.layout
        .in_bars()
        .iter()
        .filter_map(|item| layout::section(*item))
        .filter(|tab| !detach::tab_detached(s, *tab))
        .collect()
}

/// Once a frame before the window is drawn, with its `width`: the second
/// section drops when it is no longer in the window or the window is too
/// narrow, and opens when the window is widened past two sections' width.
pub(super) fn update(tool: &mut BuildTool, s: &BuildSettings, width: f32) {
    let sections = sections(s);
    let active = tool.tab;
    let st = &mut tool.ui.second;
    st.shown = None;
    st.width = width;
    let was = st.last_width.replace(width).unwrap_or(0.0);
    if s.legacy_window {
        st.tab = None;
        st.pending = None;
        return;
    }
    if st.tab.is_some_and(|tab| tab == active || !sections.contains(&tab)) {
        st.tab = None;
    }
    if let Some((target, frames)) = st.pending {
        // the width asked is on its way: nothing follows the old one
        st.pending = ((width - target).abs() >= 1.0 && frames > 0).then_some((target, frames - 1));
        if st.pending.is_some() {
            return;
        }
    }
    if width < TWO_TABS_WIDTH {
        if let Some(tab) = st.tab.take() {
            st.last = Some(tab);
        }
    } else if was < TWO_TABS_WIDTH && st.tab.is_none() {
        let next = sections
            .iter()
            .position(|tab| *tab == active)
            .map(|at| sections[(at + 1) % sections.len()]);
        st.tab = st
            .last
            .filter(|tab| *tab != active && sections.contains(tab))
            .or(next.filter(|tab| *tab != active));
    }
    if st.tab.is_none() && st.single_width.is_none() {
        st.single_width = Some(width);
    }
}

/// The window's size after a resize by its edge (`take_floater_resize`):
/// with one section, the width it gives back after two.
pub(super) fn resized(tool: &mut BuildTool, width: f32) {
    let st = &mut tool.ui.second;
    if st.tab.is_none() || width < TWO_TABS_WIDTH {
        st.single_width = Some(width);
    }
}

/// The width the window is asked to take this frame, if any.
pub(super) fn take_request(tool: &mut BuildTool) -> Option<f32> {
    tool.ui.second.request.take()
}

/// Ask the window for `width`; the second section waits for it.
fn ask(st: &mut State, width: f32) {
    st.request = Some(width);
    st.pending = Some((width, RESIZE_FRAMES));
}

/// The second section to draw this frame.
pub(super) fn shown(tool: &BuildTool, s: &BuildSettings) -> Option<usize> {
    tool.ui
        .second
        .tab
        .filter(|tab| !s.legacy_window && tool.tool == Tool::Edit && *tab != tool.tab)
}

/// A click on the button of section `tab`, Shift held or not.
pub(super) fn click(tool: &mut BuildTool, tab: usize, shift: bool) {
    let active = tool.tab;
    let st = &mut tool.ui.second;
    if !shift {
        if st.tab == Some(tab) {
            st.tab = Some(active);
        }
        tool.tab = tab;
        return;
    }
    match st.tab {
        Some(second) if second == tab => close(st),
        Some(second) if tab == active => {
            tool.tab = second;
            close(st);
        }
        Some(_) => st.tab = Some(tab),
        None if tab == active => {}
        None => {
            st.single_width = Some(st.width);
            st.tab = Some(tab);
            if st.width < TWO_TABS_WIDTH {
                ask(st, (2.0 * st.width).max(TWO_TABS_WIDTH));
            }
        }
    }
}

/// The second section closed, the window back to its width with one.
fn close(st: &mut State) {
    st.last = st.tab.take();
    if let Some(width) = st.single_width.filter(|width| *width < st.width) {
        ask(st, width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> (BuildTool, BuildSettings) {
        let tool = BuildTool {
            tool: Tool::Edit,
            tab: 6,
            ..Default::default()
        };
        (tool, BuildSettings::default())
    }

    /// One frame at `width`, with the width asked applied as the window
    /// would.
    fn frame(tool: &mut BuildTool, s: &BuildSettings, width: &mut f32) {
        update(tool, s, *width);
        if let Some(asked) = take_request(tool) {
            *width = asked;
        }
    }

    #[test]
    fn shift_click_opens_a_second_section_and_widens_then_gives_the_width_back() {
        let (mut tool, s) = tool();
        let mut width = 350.0;
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, None);
        click(&mut tool, 3, true);
        frame(&mut tool, &s, &mut width);
        assert_eq!((tool.tab, tool.ui.second.tab, width), (6, Some(3), 700.0));
        assert_eq!(shown(&tool, &s), Some(3));
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, Some(3), "the width in place keeps it");
        // a click replaces the left one; on the second's button they swap
        click(&mut tool, 2, false);
        assert_eq!((tool.tab, tool.ui.second.tab), (2, Some(3)));
        click(&mut tool, 3, false);
        assert_eq!((tool.tab, tool.ui.second.tab), (3, Some(2)));
        // Shift+click on a third replaces the second
        click(&mut tool, 4, true);
        assert_eq!((tool.tab, tool.ui.second.tab), (3, Some(4)));
        // Shift+click on the second closes it, the width given back
        click(&mut tool, 4, true);
        frame(&mut tool, &s, &mut width);
        assert_eq!((tool.tab, tool.ui.second.tab, width), (3, None, 350.0));
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, None);
    }

    #[test]
    fn shift_click_on_the_active_section_closes_it_and_keeps_the_second() {
        let (mut tool, s) = tool();
        let mut width = 400.0;
        frame(&mut tool, &s, &mut width);
        click(&mut tool, 5, true);
        frame(&mut tool, &s, &mut width);
        frame(&mut tool, &s, &mut width);
        assert_eq!(width, 800.0);
        click(&mut tool, 6, true);
        frame(&mut tool, &s, &mut width);
        assert_eq!((tool.tab, tool.ui.second.tab, width), (5, None, 400.0));
        // alone, Shift+click on the active section does nothing
        click(&mut tool, 5, true);
        assert_eq!((tool.tab, tool.ui.second.tab), (5, None));
    }

    #[test]
    fn the_window_narrowed_closes_the_second_and_widened_opens_one() {
        let (mut tool, s) = tool();
        let mut width = 350.0;
        frame(&mut tool, &s, &mut width);
        click(&mut tool, 2, true);
        frame(&mut tool, &s, &mut width);
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, Some(2));
        width = 600.0;
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, None, "too narrow for two");
        width = 720.0;
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, Some(2), "the last one comes back");
        // without a last one, the next section in the bar
        tool.ui.second.last = None;
        width = 500.0;
        frame(&mut tool, &s, &mut width);
        tool.ui.second.last = None;
        width = 900.0;
        frame(&mut tool, &s, &mut width);
        let bar = sections(&s);
        let after = bar[(bar.iter().position(|tab| *tab == 6).unwrap() + 1) % bar.len()];
        assert_eq!(tool.ui.second.tab, Some(after));
        // a window left wide with one section stays so: only a widening
        // opens the second
        tool.ui.second.single_width = Some(900.0);
        click(&mut tool, after, true);
        frame(&mut tool, &s, &mut width);
        frame(&mut tool, &s, &mut width);
        assert_eq!((tool.ui.second.tab, width), (None, 900.0));
    }

    #[test]
    fn the_legacy_style_another_tool_or_a_detached_section_show_no_second() {
        let (mut tool, mut s) = tool();
        let mut width = 350.0;
        frame(&mut tool, &s, &mut width);
        click(&mut tool, 3, true);
        frame(&mut tool, &s, &mut width);
        tool.tool = Tool::Create;
        assert_eq!(shown(&tool, &s), None);
        tool.tool = Tool::Edit;
        assert_eq!(shown(&tool, &s), Some(3));
        s.legacy_window = true;
        frame(&mut tool, &s, &mut width);
        assert_eq!(tool.ui.second.tab, None);
    }
}
