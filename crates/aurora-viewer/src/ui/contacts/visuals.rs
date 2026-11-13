//! A contact set's tile: the set as the interface shows it, in its look.

use super::*;
use crate::world::contact_sets::appearance::ICONS;

/// A contact set's tile, wherever the interface shows the set (People's
/// list of sets, its picker, the set's editor, Quick sort): its badge in
/// its colour, else its first letter, on a wash of its colour when its
/// plates have a background and ringed in it when they have a border, so
/// that the tile wears the set's look as its members' plates do. Aurora's
/// own: the Legacy Viewer shows a set by its name only.
pub(crate) fn set_tile(ui: &egui::Ui, p: &Palette, tile: egui::Rect, set: &crate::world::contact_sets::ContactSet) {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let color = crate::interface_colors::on_panel(crate::ui::colors::set_color(set.color));
    let fill = if set.visuals.background {
        color.gamma_multiply(0.2)
    } else {
        p.raised.gamma_multiply(0.6)
    };
    ui.painter().rect_filled(tile, radius, fill);
    if set.visuals.border {
        ui.painter()
            .rect_stroke(tile.shrink(0.75), radius, egui::Stroke::new(1.5, color), egui::StrokeKind::Inside);
    }
    match set.visuals.icon.and_then(|i| ICONS.get(i)) {
        Some(icon) => {
            if let Some(texture) = crate::ui::icons::global(icon) {
                let rect = egui::Rect::from_center_size(tile.center(), Vec2::splat((tile.height() * 0.48).round()));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture.id(), rect, uv, color);
            }
        }
        None => {
            let letter: String = set
                .name
                .chars()
                .find(|c| c.is_alphanumeric())
                .into_iter()
                .flat_map(char::to_uppercase)
                .collect();
            let g = ui.painter().layout_no_wrap(
                letter,
                crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), tile.height() * 0.42, color).font_id,
                color,
            );
            widgets::paint_ink_centered(ui.painter(), g, tile.center(), color);
        }
    }
}
