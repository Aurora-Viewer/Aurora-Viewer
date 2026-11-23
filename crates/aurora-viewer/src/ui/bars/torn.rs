//! Torn-off menus: the sub-menus of the wolf menu taken out into windows of
//! their own, which stay open while their entries are used. Port of
//! LLTearOffMenu and LLMenuItemTearOffGL::onCommit (indra/llui/llmenugl.cpp,
//! originally LGPL 2.1): the window takes the menu's label as its title,
//! opens where the menu was and as wide, has no minimize button, and closes
//! with its cross or its own tear-off row. One window per sub-menu; a
//! sub-menu of a torn-off window can be torn off in turn. They last for the
//! session, as in the Legacy Viewer (nothing restores them at the next
//! login); `ui_layout.ron` remembers where each was.
//!
//! A window draws the whole menu again through `menu::scoped`, which keeps
//! only the rows of its sub-menu: every entry keeps its rules (greyed,
//! checked) and its action, given back as the bars' `BarAction`.

use super::{BarAction, StatusInfo};
use crate::theme::Palette;
use crate::ui::{Panels, menu, widgets};
use egui::{Pos2, Rect, Vec2};

/// Where a window's body starts from its top-left corner: its left margin,
/// and its title bar.
const BODY: Vec2 = Vec2::new(10.0, 40.0);

/// The torn-off windows of the session.
#[derive(Default)]
pub(crate) struct TornMenus {
    windows: Vec<Torn>,
}

/// One torn-off sub-menu: its path of labels, its window id, the width of
/// its body (the menu it came from, widened to its widest entry).
struct Torn {
    path: Vec<String>,
    id: String,
    width: f32,
}

impl TornMenus {
    /// Tear off the sub-menu `path`, whose tear-off row was at `row` (the
    /// menu's top and width); a sub-menu already torn off closes instead
    /// (its own tear-off row was clicked). Returns the new window's id.
    fn toggle(&mut self, path: Vec<String>, row: Rect) -> Option<String> {
        if let Some(i) = self.windows.iter().position(|w| w.path == path) {
            self.windows.remove(i);
            return None;
        }
        let id = menu::torn_window_id(&path);
        self.windows.push(Torn {
            path,
            id: id.clone(),
            width: row.width(),
        });
        Some(id)
    }

    fn paths(&self) -> Vec<Vec<String>> {
        self.windows.iter().map(|w| w.path.clone()).collect()
    }

    /// The windows of the session, when it ends.
    pub(crate) fn clear(&mut self) {
        self.windows.clear();
    }
}

/// Where the window of a menu whose tear-off row was at `row` opens: its
/// first row where the menu's was.
fn window_pos(row: Rect) -> Pos2 {
    row.min - BODY
}

/// The width a window's body takes when its entries, `drawn` wide, do not
/// fit in its `room` (a label longer than the menu it came from, a demo's
/// guess); `None` when they fit. The rows fill the room they are given, so
/// the window only ever widens to its widest entry.
fn widen(room: f32, drawn: f32) -> Option<f32> {
    (drawn > room + 0.5).then(|| drawn.ceil())
}

/// Take the tear-off row clicked this frame, then draw the torn-off
/// windows; returns what their entries asked of the bars.
pub(crate) fn show(ctx: &egui::Context, p: &Palette, menus: &mut TornMenus, panels: &mut Panels, st: &StatusInfo) -> BarAction {
    if let Some(request) = menu::take_tear_request(ctx)
        && let Some(id) = menus.toggle(request.path, request.rect)
    {
        // where the menu was, whatever place it had the last time
        widgets::move_floater(ctx, &id, window_pos(request.rect));
    }
    let mut action = BarAction::None;
    menus.windows.retain_mut(|w| {
        let mut open = true;
        let mut found = true;
        let mut wider = None;
        let title = w.path.last().cloned().unwrap_or_default();
        widgets::Floater::new(&w.id, title, Pos2::new(200.0, 120.0), Vec2::new(w.width, 120.0))
            .auto_height()
            .no_minimize()
            // its sub-menus open beside it, in the viewer's window
            .stays_inside()
            .show(ctx, p, &mut open, |ui| {
                let room = ui.available_width();
                let drawn = ui.scope(|ui| menu::scoped(ui, p, &w.path, |ui| super::main_menus(ui, p, panels, st, &mut action, true)));
                found = drawn.inner;
                wider = widen(room, drawn.response.rect.width());
            });
        if let Some(width) = wider {
            w.width = width;
        }
        open && found
    });
    menu::set_torn(ctx, menus.paths());
    action
}

/// `AURORA_DEMO_MENU_TEAROFF=<menus>`: the sub-menus named, by their labels
/// in the interface's language joined with `/` (`World/Sun position`) and
/// separated by `;`, torn off at frame 400, side by side; capture frame
/// 620.
pub(crate) fn demo(ctx: &egui::Context, menus: &mut TornMenus, frame: u64) {
    static PATHS: std::sync::OnceLock<Vec<Vec<String>>> = std::sync::OnceLock::new();
    if frame != 400 {
        return;
    }
    let paths = PATHS.get_or_init(|| {
        std::env::var("AURORA_DEMO_MENU_TEAROFF")
            .map(|v| demo_paths(&v))
            .unwrap_or_default()
    });
    let mut x = 300.0;
    for path in paths {
        let row = Rect::from_min_size(Pos2::new(x, 160.0), Vec2::new(220.0, 10.0));
        if let Some(id) = menus.toggle(path.clone(), row) {
            widgets::move_floater(ctx, &id, window_pos(row));
        }
        x += 320.0;
    }
}

/// The paths of `AURORA_DEMO_MENU_TEAROFF`.
fn demo_paths(value: &str) -> Vec<Vec<String>> {
    value
        .split(';')
        .map(|path| {
            path.split('/')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|path| !path.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|s| (*s).to_owned()).collect()
    }

    fn row() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 200.0), Vec2::new(200.0, 10.0))
    }

    #[test]
    fn a_sub_menu_is_torn_off_once_and_its_row_closes_it() {
        let mut menus = TornMenus::default();
        let id = menus.toggle(path(&["World"]), row()).expect("torn off");
        assert_eq!(id, menu::torn_window_id(&path(&["World"])));
        assert_eq!(menus.paths(), vec![path(&["World"])]);
        // a sub-menu of it is a window of its own
        assert!(menus.toggle(path(&["World", "Sun position"]), row()).is_some());
        assert_eq!(menus.paths().len(), 2);
        // the tear-off row of the window closes it, the other stays
        assert!(menus.toggle(path(&["World"]), row()).is_none());
        assert_eq!(menus.paths(), vec![path(&["World", "Sun position"])]);
        menus.clear();
        assert!(menus.paths().is_empty());
    }

    #[test]
    fn a_torn_off_window_opens_where_its_menu_was_and_as_wide() {
        let mut menus = TornMenus::default();
        menus.toggle(path(&["Me"]), row());
        assert_eq!(menus.windows[0].width, 200.0);
        assert_eq!(window_pos(row()), Pos2::new(100.0, 200.0) - BODY);
    }

    #[test]
    fn a_window_widens_to_its_widest_entry_only() {
        assert_eq!(widen(200.0, 200.0), None);
        assert_eq!(widen(200.0, 200.3), None);
        assert_eq!(widen(200.0, 263.4), Some(264.0));
    }

    #[test]
    fn demo_paths_are_labels_joined_with_slashes() {
        assert_eq!(
            demo_paths("World; Me/Avatar health ;;"),
            vec![path(&["World"]), path(&["Me", "Avatar health"])]
        );
        assert!(demo_paths("").is_empty());
    }
}
