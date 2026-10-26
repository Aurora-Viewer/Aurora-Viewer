//! The head of the Aurora profiles, avatars' and groups' alike (#1089): a
//! banner as tall as what it holds, the picture on its left, who it is
//! beside it and the actions on its right, centred on one line, the tabs
//! along its bottom. Who it is is measured once laid out; the banner,
//! painted first, takes the height measured last, and the pass is laid out
//! again when that height changes.

use crate::theme::Palette;
use egui::{Id, Painter, Rect};

/// Above and below the line of the picture, who it is and the actions.
pub const PAD: f32 = 18.0;
/// The row of the tabs, and the room under it.
pub const TABS_H: f32 = 28.0;
pub const TABS_BOTTOM: f32 = 8.0;
/// The least room of who it is beside the named buttons: narrower, the
/// secondary buttons show their icons alone (a long language, a narrow
/// window), so the badges keep to a line or two.
pub const WHO_MIN_W: f32 = 440.0;

/// The height of who it is, as measured last under `id` (`least` before
/// the first measure).
pub fn measured(ui: &egui::Ui, id: Id, least: f32) -> f32 {
    ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(least)
}

/// Keep the height of who it is for the next pass; a new one lays this
/// pass out again, so the banner never shows the wrong height.
pub fn measure(ui: &egui::Ui, id: Id, height: f32) {
    let last = ui.ctx().data(|d| d.get_temp::<f32>(id));
    if last.is_none_or(|h| (h - height).abs() > 0.5) {
        ui.ctx().data_mut(|d| d.insert_temp(id, height));
        ui.ctx().request_discard("a profile's banner changed height");
    }
}

/// The banner's rows: the line of the picture centred under `PAD`, the tabs
/// under it, from the line's `height`.
pub fn rows(full: Rect, top: f32, height: f32) -> (Rect, Rect, Rect) {
    let line = Rect::from_min_size(egui::pos2(full.left(), top + PAD), egui::vec2(full.width(), height));
    let tabs = Rect::from_min_size(egui::pos2(full.left(), line.bottom() + PAD), egui::vec2(full.width(), TABS_H));
    let banner = Rect::from_min_max(egui::pos2(full.left(), top), egui::pos2(full.right(), tabs.bottom() + TABS_BOTTOM));
    (banner, line, tabs)
}

/// The window's panel laid thin over the whole banner: the names, the
/// badges and the buttons read over a bright picture, in both themes.
pub fn veil(ui: &egui::Ui, painter: &Painter, p: &Palette, banner: Rect) {
    painter.rect_filled(banner, ui.visuals().window_corner_radius, p.panel.gamma_multiply(0.35));
}
