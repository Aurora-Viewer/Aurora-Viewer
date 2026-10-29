//! The same quality choices in the FPS hover card and the system menu.

use super::{BarAction, StatusInfo};
use crate::graphics_presets::Preset;
use crate::theme::Palette;
use crate::ui::menu;
use aurora_i18n::tr;

pub(super) fn choices(ui: &mut egui::Ui, p: &Palette, st: &StatusInfo, action: &mut BarAction) {
    menu::section(ui, p, tr!("ui-bars-graphics-presets"));
    let custom = st.graphics_saved.iter().any(|(_, active)| *active);
    for preset in Preset::ALL {
        if menu::check(
            ui,
            p,
            "device-desktop-filled",
            preset.label(),
            !custom && st.graphics_preset == Some(preset),
            true,
        ) {
            *action = BarAction::GraphicsPreset(preset);
        }
    }
    if !st.graphics_saved.is_empty() {
        menu::section(ui, p, tr!("graphics-saved-title"));
        egui::ScrollArea::vertical()
            .id_salt("graphics_saved_list")
            .max_height(132.0)
            .show(ui, |ui| {
                for (name, active) in &st.graphics_saved {
                    if menu::check(ui, p, "device-floppy-filled", name, *active, true) {
                        *action = BarAction::SavedGraphicsPreset(name.clone());
                    }
                }
            });
    }
    menu::separator(ui, p);
    let cap = tr!("graphics-fps-cap", limit = st.fps_limit);
    if menu::check(ui, p, "gauge-filled", &cap, st.fps_cap, true) {
        *action = BarAction::FpsCap(!st.fps_cap);
    }
    if menu::item(ui, p, "adjustments-filled", tr!("ui-bars-open-graphics-preferences")) {
        *action = BarAction::GraphicsPreferences;
    }
}

pub(super) fn hover(response: &egui::Response, p: &Palette, st: &StatusInfo, action: &mut BarAction) {
    // An ordinary egui tooltip closes on wheel input. This card is a
    // hover panel, like the system tip, so a long profile list can scroll.
    let ctx = &response.ctx;
    let id = egui::Id::unique("bars_graphics_hover");
    let now = ctx.input(|i| i.time);
    let was_open = is_open(ctx);
    let hovered = response.hovered();
    let keep = was_open
        && ctx.memory(|m| m.area_rect(id)).is_some_and(|card| {
            let bridge = egui::Rect::from_min_max(
                egui::pos2(card.left(), response.rect.bottom()),
                egui::pos2(card.right(), card.top() + 1.0),
            );
            ctx.pointer_latest_pos()
                .is_some_and(|pos| card.contains(pos) || bridge.contains(pos))
        });
    let forced = super::demo::get().fps_presets && ctx.cumulative_frame_nr() >= 300;
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let dismissed = ctx.data(|d| d.get_temp::<bool>(id.with("dismissed"))).unwrap_or(false) && (hovered || keep);
    ctx.data_mut(|d| d.insert_temp(id.with("dismissed"), escape || dismissed));
    let blocked = egui::Popup::is_any_open(ctx) || escape || dismissed;
    let open = if blocked {
        false
    } else if forced || keep {
        true
    } else if hovered {
        let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id.with("since"), || now));
        let delay = f64::from(ctx.global_style().interaction.tooltip_delay);
        ctx.request_repaint_after_secs((delay - (now - since)).max(0.0) as f32);
        was_open || now - since >= delay
    } else {
        false
    };
    ctx.data_mut(|d| {
        d.insert_temp(id.with("open"), open);
        if blocked || (!open && !hovered) {
            d.remove_temp::<f64>(id.with("since"));
        }
    });
    if !open {
        return;
    }
    let mut picked = BarAction::None;
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(response.rect.right() - 268.0, response.rect.bottom() + 4.0))
        .constrain(true)
        .fade_in(false)
        .show(ctx, |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.set_width(260.0);
                choices(ui, p, st, &mut picked);
            });
        });
    if !matches!(picked, BarAction::None) {
        *action = picked;
        ctx.data_mut(|d| {
            d.insert_temp(id.with("open"), false);
            d.remove_temp::<f64>(id.with("since"));
        });
    }
}

pub(super) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("bars_graphics_hover").with("open")))
        .unwrap_or(false)
}
