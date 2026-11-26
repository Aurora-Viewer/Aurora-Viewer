//! The keys of a binding drawn as keycaps (shortcuts list, microphone
//! panel): Tabler's filled key icon when the key has one, else a cap of the
//! same shape around the key's name (after the mouse icon for a mouse
//! button).

use crate::theme::Palette;
use egui::{Color32, Vec2};

/// Height of a keycap, and of the square of Tabler's key icons.
pub(crate) const HEIGHT: f32 = 14.0;
/// Height of the keys of a window's shortcut line (the toolbar's block and
/// shortcut windows): readable beside their 28 px buttons (human report,
/// 2026-11-26).
pub(crate) const LARGE: f32 = 20.0;
/// Room between two caps of a row.
pub(crate) const GAP: f32 = 3.0;

/// Width taken by the mouse icon of a mouse button's cap.
const MOUSE: f32 = 12.0;

/// A cap, with its height.
pub(crate) enum Keycap {
    Icon(egui::TextureHandle, f32),
    Text(Option<egui::TextureHandle>, std::sync::Arc<egui::Galley>, f32),
}

impl Keycap {
    fn height(&self) -> f32 {
        match self {
            Keycap::Icon(_, height) | Keycap::Text(_, _, height) => *height,
        }
    }

    fn width(&self) -> f32 {
        match self {
            Keycap::Icon(_, height) => *height,
            Keycap::Text(mouse, galley, height) => {
                let scale = height / HEIGHT;
                let mouse = if mouse.is_some() { MOUSE * scale } else { 0.0 };
                (mouse + galley.size().x + 8.0 * scale).max(*height)
            }
        }
    }

    fn paint(&self, ui: &egui::Ui, p: &Palette, rect: egui::Rect, behind: Color32) {
        match self {
            Keycap::Icon(texture, height) => {
                // the icon's square spans 20 of its 24 units
                let image = egui::Rect::from_center_size(rect.center(), Vec2::splat(height * 24.0 / 20.0));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture.id(), image, uv, p.ink);
            }
            Keycap::Text(mouse, galley, height) => {
                ui.painter().rect_filled(rect, super::inner_radius(ui.visuals()), p.ink);
                let mut text = rect;
                if let Some(texture) = mouse {
                    let size = MOUSE * height / HEIGHT;
                    let image =
                        egui::Rect::from_center_size(egui::pos2(rect.left() + 3.0 + size / 2.0, rect.center().y), Vec2::splat(size));
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(texture.id(), image, uv, behind);
                    text.min.x += size;
                }
                let mut pos = super::row_text_origin(ui.painter(), galley, text.center().x - galley.size().x / 2.0, text.center().y);
                // The accent of a capital (Ù on AZERTY, Ö on QWERTZ) rises
                // past the line box of this small font: lower the name until
                // its ink fits in the cap, written in the colour behind.
                let above = rect.top() + 1.0 - (pos.y + galley.mesh_bounds.top());
                let below = rect.bottom() - 1.0 - (pos.y + galley.mesh_bounds.bottom());
                pos.y += above.min(below).max(0.0);
                ui.painter().galley(pos, galley.clone(), behind);
            }
        }
    }
}

/// The caps of `binding`, modifiers first. `behind` is the color under them:
/// a key's name is written in it, like the cut-out letter of the icons.
pub(crate) fn caps(ui: &egui::Ui, binding: &crate::keybinds::Binding, behind: Color32) -> Vec<Keycap> {
    caps_sized(ui, binding, behind, HEIGHT)
}

/// `caps` of another `height` (`LARGE`), their names in proportion.
pub(crate) fn caps_sized(ui: &egui::Ui, binding: &crate::keybinds::Binding, behind: Color32, height: f32) -> Vec<Keycap> {
    binding
        .caps()
        .into_iter()
        .map(|cap| match cap.icon.as_deref().and_then(crate::ui::icons::global) {
            Some(texture) => Keycap::Icon(texture, height),
            None => Keycap::Text(
                cap.mouse.then(|| crate::ui::icons::global("mouse-filled")).flatten(),
                // semibold, like the mockups' keys: a regular 10.5 px cut-out
                // loses thin strokes, the accent of "Échap" first
                ui.painter().layout_no_wrap(
                    cap.label,
                    egui::FontId::new(10.5 * height / HEIGHT, crate::ui::fonts::semibold(ui.ctx())),
                    behind,
                ),
                height,
            ),
        })
        .collect()
}

/// Width of `caps` drawn side by side.
pub(crate) fn row_width(caps: &[Keycap]) -> f32 {
    caps.iter().map(Keycap::width).sum::<f32>() + GAP * caps.len().saturating_sub(1) as f32
}

/// A small legend, one line per binding: its caps in a column, then what it
/// does. Lines whose flag is set stand out while one is set.
pub(crate) fn legend(ui: &mut egui::Ui, p: &Palette, rows: &[(crate::keybinds::Binding, &str, bool)]) {
    const LINE: f32 = 20.0;
    const AFTER_CAPS: f32 = 8.0;
    let lines: Vec<_> = rows.iter().map(|(binding, _, _)| caps(ui, binding, p.panel)).collect();
    let column = lines.iter().map(|caps| row_width(caps)).fold(0.0, f32::max);
    let held = rows.iter().any(|(_, _, active)| *active);
    ui.spacing_mut().item_spacing.y = 0.0;
    for (caps, (_, text, active)) in lines.iter().zip(rows) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(column + AFTER_CAPS, LINE), egui::Sense::hover());
            paint_row(ui, p, caps, rect.left_center(), p.panel);
            let color = if held && !active { p.muted } else { p.ink };
            ui.label(egui::RichText::new(*text).color(color));
        });
    }
}

/// Draw `caps` side by side from `left_center`.
pub(crate) fn paint_row(ui: &egui::Ui, p: &Palette, caps: &[Keycap], left_center: egui::Pos2, behind: Color32) {
    let mut x = left_center.x;
    for cap in caps {
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, left_center.y - cap.height() / 2.0),
            Vec2::new(cap.width(), cap.height()),
        );
        cap.paint(ui, p, rect, behind);
        x += cap.width() + GAP;
    }
}
