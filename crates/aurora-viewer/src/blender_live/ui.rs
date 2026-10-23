use super::*;
use crate::ui::widgets::{self, Floater};
use egui::Vec2;

pub(super) fn show(live: &mut Live, ctx: &egui::Context, p: &crate::theme::Palette, can_upload: bool) -> bool {
    if !live.open {
        return false;
    }
    let mut open = live.open;
    let mut start = false;
    let mut floater = Floater::new(
        "blender_live",
        aurora_i18n::tr!("blender-live-title"),
        egui::pos2(70.0, 110.0),
        Vec2::new(500.0, 640.0),
    );
    floater.min_size = Vec2::new(400.0, 350.0);
    floater.show(ctx, p, &mut open, |ui| {
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, p, "link-filled", aurora_i18n::tr!("blender-live-connection"));
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
                });
                object_list(live, ui, p, can_upload);
                let error = live
                    .error
                    .clone()
                    .or_else(|| live.server.as_ref().and_then(|s| s.shared.status.lock().error.clone()));
                if let Some(error) = error {
                    ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-error", error = error));
                }
                section(ui, p, "download-filled", aurora_i18n::tr!("blender-live-plugins"));
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

fn object_list(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette, can_upload: bool) {
    section(ui, p, "cube", aurora_i18n::tr!("blender-live-objects"));
    let sources = live.snapshot.as_ref().map(|s| s.sources.clone()).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(aurora_i18n::tr!(
            "blender-live-counts",
            objects = sources.iter().filter(|s| live.state(s.id) == SourceState::Visible).count(),
            triangles = live.triangles
        ));
        if ui
            .add_enabled_ui(sources.iter().any(|s| live.state(s.id) != SourceState::Removed), |ui| {
                widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-clear"))
            })
            .inner
            .clicked()
        {
            for source in &sources {
                live.set_state(source.id, SourceState::Removed);
            }
        }
    });
    if let Some(revision) = live.revision {
        ui.label(aurora_i18n::tr!("blender-live-revision", revision = revision));
    }
    if sources.iter().all(|s| live.state(s.id) == SourceState::Removed) {
        ui.weak(aurora_i18n::tr!("blender-live-empty"));
    }
    egui::ScrollArea::vertical()
        .id_salt("blender_live_objects")
        .max_height(190.0)
        .show(ui, |ui| {
            for source in &sources {
                let state = live.state(source.id);
                if state == SourceState::Removed {
                    continue;
                }
                ui.push_id(source.id, |ui| {
                    ui.horizontal(|ui| {
                        let mut visible = state == SourceState::Visible;
                        if widgets::checkbox(ui, p, &mut visible, "")
                            .on_hover_text(aurora_i18n::tr!("blender-live-visible"))
                            .changed()
                        {
                            live.set_state(source.id, if visible { SourceState::Visible } else { SourceState::Hidden });
                        }
                        let import_label = aurora_i18n::tr!("blender-live-upload");
                        let import_width = widgets::action_button_fit(ui, "file-upload-filled", import_label);
                        let name_width = (ui.available_width() - import_width - ui.spacing().item_spacing.x).max(1.0);
                        if ui
                            .add_sized(
                                Vec2::new(name_width, 26.0),
                                egui::Button::selectable(live.selected == Some(source.id), "")
                                    .left_text(&source.name)
                                    .truncate(),
                            )
                            .on_hover_text(&source.name)
                            .clicked()
                        {
                            live.selected = Some(source.id);
                        }
                        if widgets::action_button_if(
                            ui,
                            p,
                            "file-upload-filled",
                            import_label,
                            widgets::ActionKind::Primary,
                            Vec2::new(import_width, 26.0),
                            can_upload,
                        )
                        .clicked()
                        {
                            live.selected = Some(source.id);
                            live.upload_requested = Some(source.id);
                        }
                    });
                });
            }
        });
    if sources.iter().any(|s| live.state(s.id) == SourceState::Removed)
        && widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-restore")).clicked()
    {
        for source in &sources {
            if live.state(source.id) == SourceState::Removed {
                live.set_state(source.id, SourceState::Visible);
            }
        }
    }
}

fn section(ui: &mut egui::Ui, p: &crate::theme::Palette, icon: &str, title: &str) {
    ui.add_space(12.0);
    widgets::surface(ui, p, |ui| {
        ui.set_min_width(ui.available_width());
        widgets::section_header(ui, p, icon, title);
    });
    ui.add_space(4.0);
}
