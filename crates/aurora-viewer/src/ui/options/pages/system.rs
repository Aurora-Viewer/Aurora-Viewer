//! Preferences › Language and updates: the interface's language, how the
//! new versions come, whether the `secondlife:` links of web pages open
//! in this viewer, and the backup of the settings (`options/backup.rs`).

use super::super::COMBO_W;
use super::super::cx::{Cx, Nothing, field};
use egui::{RichText, Vec2};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("deep-diagnostics-title"), |cx| {
        cx.toggle(
            tr!("deep-diagnostics-next-session"),
            tr!("deep-diagnostics-next-session-help"),
            &field!(deep_diagnostics_next_session),
        );
        if crate::deep_diagnostics::active() {
            cx.note(tr!("deep-diagnostics-active"));
        }
        cx.toggle(
            tr!("deep-diagnostics-use-preferences"),
            tr!("deep-diagnostics-use-preferences-help"),
            &field!(diagnostic_environment.use_preferences),
        );
        let use_preferences = cx.s.diagnostic_environment.use_preferences;
        cx.subs(!use_preferences, |cx| {
            cx.toggle(
                tr!("deep-diagnostics-gpu-validation"),
                tr!("deep-diagnostics-gpu-validation-help"),
                &field!(diagnostic_environment.gpu_validation),
            );
            cx.toggle(
                tr!("deep-diagnostics-gpu-recovery"),
                tr!("deep-diagnostics-gpu-recovery-help"),
                &field!(diagnostic_environment.gpu_recovery),
            );
            cx.toggle(
                tr!("deep-diagnostics-cpu-cull"),
                tr!("deep-diagnostics-cpu-cull-help"),
                &field!(diagnostic_environment.cpu_cull),
            );
            cx.row(
                tr!("deep-diagnostics-contact-variant"),
                tr!("deep-diagnostics-contact-variant-help"),
                &field!(diagnostic_environment.contact_variant),
                |ui, settings| {
                    let values = [
                        ("", tr!("common-default")),
                        ("no-early-out", tr!("deep-diagnostics-contact-no-early-out")),
                        ("no-shared", tr!("deep-diagnostics-contact-no-shared")),
                        ("short", tr!("deep-diagnostics-contact-short")),
                        ("skip", tr!("deep-diagnostics-contact-skip")),
                    ];
                    let selected = values
                        .iter()
                        .find(|(v, _)| *v == settings.diagnostic_environment.contact_variant)
                        .map_or(tr!("common-default"), |(_, name)| *name);
                    let mut changed = false;
                    egui::ComboBox::from_id_salt(ui.scope_id().with("diagnostic-contact-variant"))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (value, label) in values {
                                changed |= ui
                                    .selectable_value(&mut settings.diagnostic_environment.contact_variant, value.to_owned(), label)
                                    .changed();
                            }
                        });
                    changed
                },
            );
            cx.row(
                tr!("deep-diagnostics-vram-budget"),
                tr!("deep-diagnostics-vram-budget-help"),
                &field!(diagnostic_environment.vram_budget_mb),
                |ui, settings| {
                    ui.add(egui::TextEdit::singleline(&mut settings.diagnostic_environment.vram_budget_mb).desired_width(90.0))
                        .changed()
                },
            );
            cx.row(
                tr!("deep-diagnostics-present-mode"),
                tr!("deep-diagnostics-present-mode-help"),
                &field!(diagnostic_environment.present_mode),
                |ui, settings| {
                    let values = [
                        ("", tr!("common-default")),
                        ("immediate", tr!("deep-diagnostics-present-immediate")),
                        ("mailbox", tr!("deep-diagnostics-present-mailbox")),
                        ("fifo", tr!("deep-diagnostics-present-fifo")),
                        ("fifo-relaxed", tr!("deep-diagnostics-present-fifo-relaxed")),
                    ];
                    let selected = values
                        .iter()
                        .find(|(v, _)| *v == settings.diagnostic_environment.present_mode)
                        .map_or(tr!("common-default"), |(_, name)| *name);
                    let mut changed = false;
                    egui::ComboBox::from_id_salt(ui.scope_id().with("diagnostic-present-mode"))
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            for (value, label) in values {
                                changed |= ui
                                    .selectable_value(&mut settings.diagnostic_environment.present_mode, value.to_owned(), label)
                                    .changed();
                            }
                        });
                    changed
                },
            );
            cx.note(tr!("deep-diagnostics-applies-next-start"));
        });
        let p = cx.p;
        cx.row_with(
            tr!("deep-diagnostics-bundle"),
            tr!("deep-diagnostics-bundle-help"),
            &Nothing,
            |ui, _, st, result| {
                use crate::ui::widgets::{ActionKind, action_button};
                if action_button(
                    ui,
                    &p,
                    "file-download-filled",
                    tr!("deep-diagnostics-bundle-save"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    result.save_repro_bundle = true;
                }
                if !st.repro_bundle_status.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&st.repro_bundle_status).size(11.5).color(p.muted)).wrap());
                }
                false
            },
        );
    });
    cx.group(tr!("language-choice"), |cx| {
        let languages: Vec<&str> = aurora_i18n::available_languages().map(|(_, name)| name).collect();
        cx.keywords(&languages.join(" "))
            .row(tr!("language-choice"), tr!("language-help"), &field!(language), |ui, s| {
                let selected = if s.language == "default" {
                    tr!("language-system")
                } else {
                    aurora_i18n::available_languages()
                        .find(|&(code, _)| code == s.language)
                        .map_or_else(|| aurora_i18n::text_in("en", "language-name"), |(_, name)| name)
                };
                let mut changed = false;
                egui::ComboBox::from_id_salt(ui.scope_id().with("ui-language"))
                    .width(COMBO_W)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut s.language, "default".to_owned(), tr!("language-system"))
                            .changed();
                        for (code, name) in aurora_i18n::available_languages() {
                            changed |= ui.selectable_value(&mut s.language, code.to_owned(), name).changed();
                        }
                    });
                changed
            });
    });
    cx.group(tr!("update-settings"), |cx| {
        use crate::updater::Mode;
        cx.combo(
            tr!("update-settings"),
            tr!("update-settings-help"),
            &field!(update_mode),
            &[
                (Mode::Disabled, tr!("update-mode-disabled")),
                (Mode::Ask, tr!("update-mode-ask")),
                (Mode::Automatic, tr!("update-mode-auto")),
            ],
        );
        use aurora_updater::feed::Channel;
        cx.combo(
            tr!("update-channel"),
            tr!("update-channel-help"),
            &field!(update_channel),
            &[
                (Channel::Stable, tr!("update-channel-stable")),
                (Channel::Beta, tr!("update-channel-beta")),
            ],
        );
    });
    // the system's state, not a saved setting, so outside the page's reset;
    // the Legacy Viewer's installer asks it once
    if crate::url_scheme::supported() {
        let p = cx.p;
        cx.group(tr!("ui-options-slurl-links"), |cx| {
            use crate::url_scheme::Handler;
            let handler = crate::url_scheme::handler();
            if let Handler::Other(program) = &handler {
                cx.note(&tr!("ui-options-slurl-links-other", program = program.clone()));
            }
            cx.row(
                tr!("ui-options-slurl-links-open"),
                tr!("ui-options-slurl-links-open-help"),
                &Nothing,
                |ui, _| {
                    let mut on = handler == Handler::This;
                    if super::super::toggle(ui, &p, &mut on) {
                        let done = if on {
                            crate::url_scheme::register()
                        } else {
                            crate::url_scheme::unregister()
                        };
                        if let Err(e) = done {
                            log::warn!("secondlife links: {e}");
                        }
                        crate::url_scheme::refresh();
                    }
                    false
                },
            );
        });
    }
    // Legacy Viewer › Backup (panel_preferences_backup.xml): files, not
    // settings, so outside the page's reset
    let p = cx.p;
    cx.group(tr!("ui-options-backup"), |cx| {
        use crate::ui::widgets::{ActionKind, action_button};
        let size = Vec2::new(0.0, 26.0);
        cx.row_with(
            tr!("ui-options-backup-save"),
            tr!("ui-options-backup-save-help"),
            &Nothing,
            |ui, _, st, r| {
                ui.vertical(|ui| {
                    if action_button(
                        ui,
                        &p,
                        "file-download-filled",
                        tr!("ui-options-backup-save-button"),
                        ActionKind::Flat,
                        size,
                    )
                    .clicked()
                    {
                        r.back_up = true;
                    }
                    if !st.backup_status.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&st.backup_status).size(11.5).color(p.muted)).wrap());
                    }
                });
                false
            },
        );
        cx.row_with(
            tr!("ui-options-backup-restore"),
            tr!("ui-options-backup-restore-help"),
            &Nothing,
            |ui, _, st, r| {
                ui.vertical(|ui| {
                    if action_button(
                        ui,
                        &p,
                        "file-upload-filled",
                        tr!("ui-options-backup-restore-button"),
                        ActionKind::Flat,
                        size,
                    )
                    .clicked()
                    {
                        r.pick_backup = true;
                    }
                    if !st.restore_status.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&st.restore_status).size(11.5).color(p.muted)).wrap());
                    }
                });
                false
            },
        );
    });
}
