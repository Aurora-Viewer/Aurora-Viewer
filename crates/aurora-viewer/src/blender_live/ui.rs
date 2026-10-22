use super::*;
use crate::ui::widgets::{self, Floater};
use egui::Vec2;

pub(super) fn show(live: &mut Live, ctx: &egui::Context, p: &crate::theme::Palette) -> bool {
    if !live.open {
        return false;
    }
    let mut open = live.open;
    let mut start = false;
    let mut floater = Floater::new(
        "blender_live",
        aurora_i18n::tr!("blender-live-title"),
        egui::pos2(70.0, 110.0),
        Vec2::new(460.0, 440.0),
    );
    floater.min_size = Vec2::new(400.0, 350.0);
    floater.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(aurora_i18n::tr!("blender-live-description"));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("blender-live-port"));
                    ui.add_enabled(!live.running(), egui::DragValue::new(&mut live.port).range(1024..=65535));
                    ui.monospace("127.0.0.1");
                });
                let connected = live.server.as_ref().is_some_and(|server| server.shared.status.lock().connected);
                let label = if connected {
                    aurora_i18n::tr!("blender-live-connected")
                } else if live.running() {
                    aurora_i18n::tr!("blender-live-waiting")
                } else {
                    aurora_i18n::tr!("blender-live-stopped")
                };
                ui.colored_label(if connected { p.violet_light } else { p.muted }, label);
                ui.horizontal(|ui| {
                    if live.running() {
                        if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-stop")).clicked() {
                            live.stop();
                        }
                    } else if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-start")).clicked() {
                        start = true;
                    }
                    if ui
                        .add_enabled_ui(!live.objects.is_empty(), |ui| {
                            widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-clear"))
                        })
                        .inner
                        .clicked()
                    {
                        live.clear = true;
                    }
                });
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!(
                    "blender-live-counts",
                    objects = live.objects.len(),
                    triangles = live.triangles
                ));
                if let Some(revision) = live.revision {
                    ui.label(aurora_i18n::tr!("blender-live-revision", revision = revision));
                }
                let error = live
                    .error
                    .clone()
                    .or_else(|| live.server.as_ref().and_then(|s| s.shared.status.lock().error.clone()));
                if let Some(error) = error {
                    ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-error", error = error));
                }
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("blender-live-instructions"));
                ui.add_enabled_ui(!live.addon.busy, |ui| {
                    widgets::radio_value(
                        ui,
                        p,
                        &mut live.addon.legacy,
                        false,
                        aurora_i18n::tr!("blender-live-package-extension"),
                    );
                    widgets::radio_value(ui, p, &mut live.addon.legacy, true, aurora_i18n::tr!("blender-live-package-legacy"));
                });
                if ui
                    .add_enabled_ui(!live.addon.busy, |ui| {
                        widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-save-addon"))
                    })
                    .inner
                    .clicked()
                {
                    live.save_addon = true;
                }
                if live.addon.saved {
                    ui.colored_label(p.violet_light, aurora_i18n::tr!("blender-live-addon-saved"));
                }
                if let Some(error) = &live.addon.error {
                    ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-addon-error", error = error.as_str()));
                }
                ui.small(aurora_i18n::tr!("blender-live-local-only"));
            });
    });
    live.open = open;
    start
}
