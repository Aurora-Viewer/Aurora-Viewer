//! The keys of a binding drawn as keycaps (shortcuts list, microphone
//! panel): Tabler's filled key icon when the key has one, else a cap of the
//! same shape around the key's name (after the mouse icon for a mouse
//! button).

use crate::theme::Palette;
use egui::{Color32, Vec2};

/// Height of a keycap, and of the square of Tabler's key icons.
pub(crate) const HEIGHT: f32 = 14.0;
/// Room between two caps of a row.
pub(crate) const GAP: f32 = 3.0;

/// Width taken by the mouse icon of a mouse button's cap.
const MOUSE: f32 = 12.0;

pub(crate) enum Keycap {
    Icon(egui::TextureHandle),
    Text(Option<egui::TextureHandle>, std::sync::Arc<egui::Galley>),
}

impl Keycap {
    fn width(&self) -> f32 {
        match self {
            Keycap::Icon(_) => HEIGHT,
            Keycap::Text(mouse, galley) => {
                let mouse = if mouse.is_some() { MOUSE } else { 0.0 };
                (mouse + galley.size().x + 8.0).max(HEIGHT)
            }
        }
    }

    fn paint(&self, ui: &egui::Ui, p: &Palette, rect: egui::Rect, behind: Color32) {
        match self {
            Keycap::Icon(texture) => {
                // the icon's square spans 20 of its 24 units
                let image = egui::Rect::from_center_size(rect.center(), Vec2::splat(HEIGHT * 24.0 / 20.0));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture.id(), image, uv, p.ink);
            }
            Keycap::Text(mouse, galley) => {
                ui.painter().rect_filled(rect, super::inner_radius(ui.visuals()), p.ink);
                let mut text = rect;
                if let Some(texture) = mouse {
                    let image =
                        egui::Rect::from_center_size(egui::pos2(rect.left() + 3.0 + MOUSE / 2.0, rect.center().y), Vec2::splat(MOUSE));
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(texture.id(), image, uv, behind);
                    text.min.x += MOUSE;
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
    binding
        .caps()
        .into_iter()
        .map(|cap| match cap.icon.as_deref().and_then(crate::ui::icons::global) {
            Some(texture) => Keycap::Icon(texture),
            None => Keycap::Text(
                cap.mouse.then(|| crate::ui::icons::global("mouse-filled")).flatten(),
                // semibold, like the mockups' keys: a regular 10.5 px cut-out
                // loses thin strokes, the accent of "Échap" first
                ui.painter()
                    .layout_no_wrap(cap.label, egui::FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())), behind),
            ),
        })
        .collect()
}

/// Width of `caps` drawn side by side.
pub(crate) fn row_width(caps: &[Keycap]) -> f32 {
    caps.iter().map(Keycap::width).sum::<f32>() + GAP * caps.len().saturating_sub(1) as f32
}

/// Draw `caps` side by side from `left_center`.
pub(crate) fn paint_row(ui: &egui::Ui, p: &Palette, caps: &[Keycap], left_center: egui::Pos2, behind: Color32) {
    let mut x = left_center.x;
    for cap in caps {
        let rect = egui::Rect::from_min_size(egui::pos2(x, left_center.y - HEIGHT / 2.0), Vec2::new(cap.width(), HEIGHT));
        cap.paint(ui, p, rect, behind);
        x += cap.width() + GAP;
    }
}
