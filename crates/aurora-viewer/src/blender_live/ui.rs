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
        Vec2::new(560.0, 660.0),
    );
    floater.min_size = Vec2::new(460.0, 350.0);
    floater.show(ctx, p, &mut open, |ui| {
        let font = egui::FontId::proportional(16.0);
        ui.style_mut().override_font_id = Some(font.clone());
        let row_height = (ui.fonts_mut(|fonts| fonts.row_height(&font)) + 12.0).max(32.0);
        ui.spacing_mut().interact_size.y = row_height;
        let mut tab = usize::from(live.tab == Tab::Plugins);
        widgets::stretched_tabs(
            ui,
            p,
            &mut tab,
            &[
                (aurora_i18n::tr!("blender-live-connection"), true),
                (aurora_i18n::tr!("blender-live-plugins"), true),
            ],
        );
        live.tab = if tab == 0 { Tab::Connection } else { Tab::Plugins };
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt(("blender_live_tab", tab))
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| match live.tab {
                Tab::Connection => start = connection(live, ui, p, can_upload),
                Tab::Plugins => plugins(live, ui, p),
            });
    });
    live.open = open;
    start
}

fn connection(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette, can_upload: bool) -> bool {
    let mut start = false;
    let connected = live.server.as_ref().is_some_and(|server| server.shared.status.lock().connected);
    let label = if connected {
        aurora_i18n::tr!("blender-live-connected")
    } else if live.running() {
        aurora_i18n::tr!("blender-live-waiting")
    } else {
        aurora_i18n::tr!("blender-live-stopped")
    };
    ui.horizontal(|ui| {
        widgets::glyph(ui, "link-filled", p.violet_light, 18.0);
        ui.colored_label(if connected { p.violet_light } else { p.muted }, label);
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(aurora_i18n::tr!("blender-live-port"));
        ui.add_enabled(!live.running(), egui::DragValue::new(&mut live.port).range(1024..=65535));
        ui.monospace("127.0.0.1");
        if live.running() {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-stop")).clicked() {
                live.stop();
            }
        } else if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-start")).clicked() {
            start = true;
        }
    });
    shape_controls(live, ui, p);
    object_list(live, ui, p, can_upload);
    object_controls(live, ui, p);
    let error = live
        .error
        .clone()
        .or_else(|| live.server.as_ref().and_then(|s| s.shared.status.lock().error.clone()));
    if let Some(error) = error {
        ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-error", error = error));
    }
    ui.add_space(12.0);
    ui.label(aurora_i18n::tr!("blender-live-local-only"));
    start
}

fn plugins(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    installer(live, ui, p);
    ui.add_space(12.0);
    widgets::collapsing(
        ui,
        p,
        "blender_live_manual_zip",
        aurora_i18n::tr!("blender-live-manual-install"),
        false,
        |ui| {
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
        },
    );
}

fn installer(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    use super::installer::Action;
    let installer = &live.installer;
    ui.label(aurora_i18n::tr!("blender-live-bundled-version", version = super::addon::version()));
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!installer.busy, |ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-detect")).clicked() {
                live.installer_scan = true;
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("blender-live-choose-blender")).clicked() {
                live.installer_choose = true;
            }
        });
    });
    if installer.busy {
        use super::installer::Phase;
        ui.horizontal(|ui| {
            ui.spinner();
            ui.weak(match installer.phase() {
                Phase::Choosing => aurora_i18n::tr!("blender-live-choosing"),
                Phase::Scanning => aurora_i18n::tr!("blender-live-detecting"),
                Phase::Checking => aurora_i18n::tr!("blender-live-checking"),
                Phase::Installing => aurora_i18n::tr!("blender-live-installing"),
            });
        });
    } else if installer.scanned && installer.installations.is_empty() {
        ui.weak(aurora_i18n::tr!("blender-live-no-installations"));
    }
    if let Some(report) = &installer.success {
        let [major, minor, patch] = report.version;
        ui.colored_label(
            p.success,
            aurora_i18n::tr!("blender-live-install-success", version = format!("{major}.{minor}.{patch}")),
        );
        if let Some(backup) = &report.backup {
            ui.label(aurora_i18n::tr!("blender-live-backup", path = backup.display().to_string()));
        }
        ui.label(aurora_i18n::tr!(
            "blender-live-install-location",
            path = report.target.display().to_string()
        ))
        .on_hover_text(report.profile.display().to_string());
    }
    if let Some(error) = &installer.error {
        ui.colored_label(p.danger, aurora_i18n::tr!("blender-live-installer-error", error = error.as_str()));
    }
    egui::ScrollArea::vertical()
        .id_salt("blender_installations")
        .max_height(360.0)
        .show(ui, |ui| {
            for installation in &installer.installations {
                ui.push_id(&installation.executable, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(installation.label()).on_hover_text(installation.path_label());
                        let report = &installation.report;
                        let status = if let Some(version) = &report.plugin_version {
                            aurora_i18n::tr!("blender-live-installed-version", version = version.as_str())
                        } else {
                            aurora_i18n::tr!("blender-live-not-installed").to_owned()
                        };
                        ui.weak(status);
                        let action = installation.action();
                        let label = match action {
                            Action::Install => aurora_i18n::tr!("blender-live-install"),
                            Action::Update => aurora_i18n::tr!("blender-live-update"),
                            Action::Enable => aurora_i18n::tr!("blender-live-enable-addon"),
                            Action::Reinstall => aurora_i18n::tr!("blender-live-reinstall"),
                            Action::Newer => aurora_i18n::tr!("blender-live-newer"),
                            Action::Unsupported => aurora_i18n::tr!("blender-live-unsupported-version"),
                            Action::Failed => aurora_i18n::tr!("blender-live-detection-failed"),
                        };
                        let enabled =
                            !installer.busy && matches!(action, Action::Install | Action::Update | Action::Enable | Action::Reinstall);
                        if ui
                            .add_enabled_ui(enabled, |ui| widgets::flat_button(ui, p, label))
                            .inner
                            .on_hover_text(report.error.as_deref().unwrap_or(label))
                            .clicked()
                        {
                            live.installer_target = Some(installation.executable.clone());
                        }
                    });
                    ui.add(egui::Label::new(egui::RichText::new(installation.path_label()).color(p.muted)).truncate())
                        .on_hover_text(installation.path_label());
                });
            }
        });
}

fn shape_controls(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    ui.horizontal_wrapped(|ui| {
        for (action, label, tooltip, blocked) in [
            (
                shape::Action::Create,
                aurora_i18n::tr!("blender-shape-send"),
                aurora_i18n::tr!("blender-shape-tooltip"),
                &live.shape.blocked,
            ),
            (
                shape::Action::Apply,
                aurora_i18n::tr!("blender-shape-apply"),
                aurora_i18n::tr!("blender-shape-apply-tooltip"),
                &live.shape.apply_blocked,
            ),
        ] {
            if ui
                .add_enabled_ui(blocked.is_none() && !live.shape.busy(), |ui| widgets::flat_button(ui, p, label))
                .inner
                .on_hover_text(tooltip)
                .on_disabled_hover_text(blocked.as_deref().unwrap_or(aurora_i18n::tr!("blender-shape-sending")))
                .clicked()
            {
                live.shape.requested = Some(action);
            }
        }
        if live.shape.busy() {
            ui.weak(aurora_i18n::tr!("blender-shape-sending"));
        }
    });
    if !live.shape.message.is_empty() {
        ui.colored_label(if live.shape.error { p.danger } else { p.success }, &live.shape.message);
    }
}

fn object_list(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette, can_upload: bool) {
    section(ui, p, "cube", aurora_i18n::tr!("blender-live-objects"));
    if !live.import.message.is_empty() {
        ui.colored_label(if live.import.error { p.danger } else { p.muted }, &live.import.message);
    }
    let sources = live.snapshot.as_ref().map(|s| s.sources.clone()).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.label(aurora_i18n::tr!(
            "blender-live-linkset-counts",
            objects = sources
                .iter()
                .filter(|s| s.link.is_none() && live.state(s.id) == SourceState::Visible)
                .count(),
            prims = live.snapshot.as_ref().map_or(0, |s| s
                .meshes
                .iter()
                .filter(|m| live.state(m.source) == SourceState::Visible && live.state(live.linked_root(m.source)) == SourceState::Visible)
                .count()),
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
            for row in live.linkset_rows() {
                let source = &row.source;
                let state = live.state(source.id);
                if state == SourceState::Removed {
                    continue;
                }
                ui.push_id(source.id, |ui| {
                    ui.horizontal(|ui| {
                        if source.link.is_some() {
                            ui.add_space(22.0);
                        }
                        let mut visible = state == SourceState::Visible;
                        if widgets::checkbox(ui, p, &mut visible, "")
                            .on_hover_text(aurora_i18n::tr!("blender-live-visible"))
                            .changed()
                        {
                            live.set_state(source.id, if visible { SourceState::Visible } else { SourceState::Hidden });
                        }
                        let import_label = aurora_i18n::tr!("blender-live-upload");
                        let import_width = widgets::action_button_fit(ui, "file-upload-filled", import_label);
                        let row_height = ui.spacing().interact_size.y;
                        let name_width = (ui.available_width() - import_width - ui.spacing().item_spacing.x).max(1.0);
                        let name = if source.link.is_some() {
                            aurora_i18n::tr!("blender-object-linked-prim", number = row.number, name = source.name.as_str())
                        } else if row.prims > 1 {
                            aurora_i18n::tr!("blender-object-linkset", name = source.name.as_str(), prims = row.prims)
                        } else {
                            source.name.clone()
                        };
                        if ui
                            .add_sized(
                                Vec2::new(name_width, row_height),
                                egui::Button::selectable(live.selected == Some(source.id), "")
                                    .left_text(&name)
                                    .truncate(),
                            )
                            .on_hover_text(&name)
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
                            Vec2::new(import_width, row_height),
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
}

fn object_controls(live: &mut Live, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    use super::objects::{Action, Location};
    use crate::ui::menu;
    use glam::{EulerRot, Quat, Vec3};
    let Some(source) = live
        .selected
        .and_then(|id| live.snapshot.as_ref()?.sources.iter().find(|source| source.id == id))
        .cloned()
    else {
        return;
    };
    if source.rigged() {
        return;
    }
    let Some(placement) = live.placements.get(&source.id).copied() else {
        return;
    };
    ui.add_space(8.0);
    ui.label(&source.name);
    if let Some(link) = source.link {
        let root = live
            .snapshot
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| s.id == link.parent))
            .map_or("", |s| s.name.as_str());
        ui.weak(aurora_i18n::tr!("blender-object-child-frame", root = root));
    }
    let location = live
        .placements
        .get(&live.linked_root(source.id))
        .map_or(placement.location, |p| p.location);
    let attached = matches!(location, Location::Attachment(_));
    let present = live.source_present(source.id);
    let status = match location {
        Location::World => aurora_i18n::tr!("blender-object-world").to_owned(),
        Location::Detached => aurora_i18n::tr!("blender-object-detached").to_owned(),
        Location::Attachment(point) => live
            .attach_choices
            .iter()
            .find(|choice| choice.0 == point)
            .map_or_else(String::new, |choice| {
                aurora_i18n::tr!("blender-object-attached", point = choice.2.as_str())
            }),
    };
    ui.weak(status);
    ui.horizontal_wrapped(|ui| {
        for (action, label, enabled) in [
            (
                Action::Edit,
                aurora_i18n::tr!("blender-object-edit"),
                present && live.root(source.id).is_some(),
            ),
            (Action::Rez, aurora_i18n::tr!("blender-object-rez"), !present),
            (Action::Drop, aurora_i18n::tr!("blender-object-drop"), present),
            (Action::Derez, aurora_i18n::tr!("blender-object-derez"), present && !attached),
            (Action::Detach, aurora_i18n::tr!("common-detach"), attached),
        ] {
            if ui.add_enabled_ui(enabled, |ui| widgets::flat_button(ui, p, label)).inner.clicked() {
                live.object_action = Some((source.id, action));
            }
        }
        for hud in [false, true] {
            let label = if hud {
                aurora_i18n::tr!("common-attach-to-hud")
            } else {
                aurora_i18n::tr!("common-attach-to")
            };
            menu::button(ui, p, egui::Button::new(label), |ui| {
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (point, is_hud, name) in &live.attach_choices {
                        if *is_hud == hud && menu::item(ui, p, if hud { "device-desktop-filled" } else { "man-filled" }, name) {
                            live.object_action = Some((source.id, Action::Attach(*point)));
                        }
                    }
                });
            });
        }
    });
    ui.add_enabled_ui(present && (source.link.is_none() || live.edit_linked), |ui| {
        let mut position = placement.position.to_array();
        let (z, y, x) = placement.rotation.to_euler(EulerRot::ZYX);
        let mut rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
        let mut size = (placement.size * placement.scale).to_array();
        let mut changed = false;
        egui::Grid::new("blender_object_transform").num_columns(4).show(ui, |ui| {
            for (label, values, speed, range) in [
                (
                    aurora_i18n::tr!("build-ui-object-position-meters"),
                    &mut position,
                    0.01,
                    -65536.0..=65536.0,
                ),
                (
                    aurora_i18n::tr!("build-ui-object-rotation-degrees"),
                    &mut rotation,
                    1.0,
                    -360.0..=360.0,
                ),
                (aurora_i18n::tr!("build-ui-object-size-meters"), &mut size, 0.01, 0.01..=64.0),
            ] {
                ui.label(label);
                for (axis, value) in ["X ", "Y ", "Z "].into_iter().zip(values.iter_mut()) {
                    changed |= ui
                        .add(
                            egui::DragValue::new(value)
                                .prefix(axis)
                                .speed(speed)
                                .range(range.clone())
                                .max_decimals(3),
                        )
                        .changed();
                }
                ui.end_row();
            }
        });
        if changed {
            let mut position = Vec3::from_array(position);
            if placement.parent.is_none()
                && let Location::Attachment(point) = placement.location
                && live.attach_choices.iter().any(|choice| choice.0 == point && !choice.1)
            {
                position = position.clamp_length_max(crate::build::MAX_ATTACHMENT_DIST);
            }
            live.object_action = Some((
                source.id,
                Action::Transform {
                    position,
                    rotation: Quat::from_euler(
                        EulerRot::ZYX,
                        rotation[2].to_radians(),
                        rotation[1].to_radians(),
                        rotation[0].to_radians(),
                    ),
                    scale: Vec3::from_array(size) / placement.size,
                },
            ));
        }
    });
}

fn section(ui: &mut egui::Ui, p: &crate::theme::Palette, icon: &str, title: &str) {
    section_divider(ui, p);
    widgets::section_header(ui, p, icon, title);
}

fn section_divider(ui: &mut egui::Ui, p: &crate::theme::Palette) {
    ui.add_space(12.0);
    let divider_palette = crate::theme::Palette { raised: p.muted_dim, ..*p };
    widgets::divider(ui, &divider_palette);
}
