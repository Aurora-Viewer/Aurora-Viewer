//! HUD gestures in the shortcut preferences.
use super::{Capture, OptionsUi, group, row};
use crate::{keybinds::HoldKey, settings::Settings, theme::Palette};
use egui::{CornerRadius, RichText, Vec2};

pub(super) fn shortcuts_group(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, st: &mut OptionsUi) -> bool {
    let mut changed = false;
    group(ui, p, aurora_i18n::tr!("preferences-group-huds"), |ui| {
        changed |= row(
            ui,
            p,
            aurora_i18n::tr!("ui-options-practical-movement-key"),
            aurora_i18n::tr!("ui-options-practical-hold-this-key-then-left-drag-a-hud-alt"),
            |ui| {
                let capturing = st.capture == Some(Capture::HudDrag);
                let text = if capturing {
                    aurora_i18n::tr!("common-press-a-key").into()
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
                button.on_hover_text(aurora_i18n::tr!("ui-options-practical-click-then-press-a-key-alt-ctrl-and-shift"));
                if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("ui-options-practical-restore-alt")).clicked() {
                    s.hud_drag_key = HoldKey::default();
                    st.capture = None;
                    return true;
                }
                false
            },
        );
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "ui-options-practical-hold-and-left-drag-a-visible-hud-element-the",
                value0 = s.hud_drag_key.label().to_string()
            ))
            .size(12.0)
            .color(p.muted),
        );
    });
    changed
}
