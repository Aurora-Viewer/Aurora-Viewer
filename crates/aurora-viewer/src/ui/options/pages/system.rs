//! Preferences › Language and updates: the interface's language, how the
//! new versions come, whether the `secondlife:` links of web pages open
//! in this viewer, the backup of the settings (`options/backup.rs`), and
//! the removal of Aurora from this computer (`uninstall.rs`).

use super::super::COMBO_W;
use super::super::cx::{Cx, Nothing, field};
use egui::{RichText, Vec2};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
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
        use crate::updater::Interval;
        cx.combo(
            tr!("update-interval"),
            tr!("update-interval-help"),
            &field!(update_check_interval),
            &[
                (Interval::Minutes10, tr!("update-interval-10-minutes")),
                (Interval::Minutes30, tr!("update-interval-30-minutes")),
                (Interval::Hour1, tr!("update-interval-1-hour")),
                (Interval::Hours6, tr!("update-interval-6-hours")),
                (Interval::Never, tr!("update-interval-never")),
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
    // the Legacy Viewer's uninstaller, from the viewer (uninstall.rs): what
    // it registered outside its folders, confirmed first
    cx.group(tr!("ui-options-remove-aurora"), |cx| {
        use crate::ui::widgets::{ActionKind, action_button};
        cx.row_with(
            tr!("ui-options-remove-aurora"),
            tr!("ui-options-remove-aurora-help"),
            &Nothing,
            |ui, _, _, r| {
                if action_button(
                    ui,
                    &p,
                    "trash-filled",
                    tr!("ui-options-remove-aurora-button"),
                    ActionKind::Danger,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    r.uninstall = true;
                }
                false
            },
        );
    });
}
