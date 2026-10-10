//! Aurora convenience gestures requested by the user.
use super::{Capture, OptionsUi, group, row};
use crate::{keybinds::HoldKey, settings::Settings, theme::Palette};
use egui::{CornerRadius, RichText, Vec2};

pub(super) fn page(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut changed = false;
    group(ui, p, "HUDs", |ui| {
        changed |= row(
            ui,
            p,
            "Touche de déplacement",
            "Maintenez cette touche puis glissez un HUD avec le clic gauche. ALT par défaut.",
            |ui| {
                let capturing = st.capture == Some(Capture::HudDrag);
                let text = if capturing {
                    "Appuyez sur une touche…".into()
                } else {
                    s.hud_drag_key.label()
                };
                let button = ui.add(
                    egui::Button::new(RichText::new(text).size(12.0).color(p.ink))
                        .fill(if capturing { p.violet } else { p.raised })
                        .corner_radius(CornerRadius::same(2))
                        .min_size(Vec2::new(132.0, 20.0)),
                );
                if button.clicked() {
                    st.capture = if capturing { None } else { Some(Capture::HudDrag) };
                }
                button.on_hover_text("Cliquez puis appuyez sur une touche (ALT, Ctrl et Maj sont acceptés). Échap pour annuler.");
                if crate::ui::widgets::flat_button(ui, p, "Rétablir ALT").clicked() {
                    s.hud_drag_key = HoldKey::default();
                    st.capture = None;
                    return true;
                }
                false
            },
        );
        ui.label(
            RichText::new(format!(
                "Maintenez {} et glissez un élément visible du HUD avec le clic gauche. La position est enregistrée au relâchement du clic.",
                s.hud_drag_key.label()
            ))
            .size(12.0)
            .color(p.muted),
        );
    });
    changed
}
