//! Preferences › Language and updates: the interface's language, how the
//! new versions come, whether the `secondlife:` links of web pages open
//! in this viewer, the backup of the settings (`options/backup.rs`), and
//! the removal of Aurora from this computer (`uninstall.rs`).

use super::super::COMBO_W;
use super::super::cx::{Cx, Nothing, field, fields};
use super::super::{Folder, FolderChoice};
use crate::language_file::{Failure, Shown};
use egui::{RichText, Vec2};

fn language_choices() -> &'static [(&'static str, String)] {
    static CHOICES: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();
    CHOICES.get_or_init(|| {
        let mut languages: Vec<_> = aurora_i18n::available_languages().collect();
        aurora_i18n::language_menu_order(&mut languages, aurora_i18n::system_language());
        languages
            .into_iter()
            .map(|(code, name)| (code, format!("{name} ({code})")))
            .collect()
    })
}

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.group(tr!("language-choice"), |cx| {
        let choices = language_choices();
        let languages: Vec<_> = choices.iter().map(|(_, label)| label.as_str()).collect();
        cx.keywords(&languages.join(" "))
            .row(tr!("language-choice"), tr!("language-help"), &field!(language), |ui, s| {
                let selected = if s.language == "default" || s.language.is_empty() {
                    tr!("language-system")
                } else {
                    let codes: Vec<_> = choices.iter().map(|(code, _)| *code).collect();
                    let code = aurora_i18n::resolve_language(&s.language, "", &codes);
                    choices
                        .iter()
                        .find(|(key, _)| *key == code)
                        .map_or_else(|| aurora_i18n::text_in("en", "language-name"), |(_, label)| label.as_str())
                };
                let mut changed = false;
                // greyed while the language file is used instead
                ui.add_enabled_ui(!s.custom_language, |ui| {
                    egui::ComboBox::from_id_salt(ui.scope_id().with("ui-language"))
                        .width(COMBO_W)
                        .selected_text(selected)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(&mut s.language, "default".to_owned(), tr!("language-system"))
                                .changed();
                            for (code, label) in choices {
                                changed |= ui.selectable_value(&mut s.language, (*code).to_owned(), label).changed();
                            }
                        })
                })
                .response
                .on_disabled_hover_text(tr!("language-file-instead"));
                changed
            });
        // the user's own catalog file, read at startup (language_file.rs)
        let p = cx.p;
        cx.keywords("ftl fluent").row_with(
            tr!("language-file"),
            tr!("language-file-help"),
            &fields!(custom_language, custom_language_file),
            |ui, s, _, r| {
                use crate::ui::widgets::{self, ActionKind, action_button};
                let mut changed = false;
                ui.vertical(|ui| {
                    changed |= widgets::checkbox(ui, &p, &mut s.custom_language, tr!("language-file-use")).changed();
                    if s.custom_language {
                        let size = Vec2::new(0.0, 24.0);
                        if action_button(ui, &p, "folder-open-filled", tr!("language-file-browse"), ActionKind::Flat, size).clicked() {
                            r.folder = Some((Folder::LanguageFile, FolderChoice::Pick));
                        }
                        let shown = crate::language_file::shown(&s.custom_language_file, crate::language_file::loaded());
                        for (line, warning) in status_lines(&shown) {
                            let color = if warning { p.warn } else { p.muted };
                            ui.add(egui::Label::new(RichText::new(line).size(11.5).color(color)).wrap());
                        }
                    }
                });
                changed
            },
        );
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

/// The lines under the language file's button, each with whether it is a
/// warning: the file in use, the one for the next launch, or why it is not
/// used.
fn status_lines(shown: &Shown) -> Vec<(String, bool)> {
    use aurora_i18n::tr;
    match shown {
        Shown::NoFile => vec![(tr!("language-file-none").to_owned(), true)],
        Shown::InUse { name, info } => {
            let mut lines = vec![(tr!("language-file-in-use", name = name.as_str(), count = info.messages), false)];
            if info.errors > 0 {
                lines.push((tr!("language-file-errors", count = info.errors), true));
            }
            if !info.named {
                lines.push((tr!("language-file-unnamed").to_owned(), true));
            }
            lines
        }
        Shown::NextLaunch { name } => vec![(tr!("language-file-next", name = name.as_str()), false)],
        Shown::Failed { name, failure } => {
            let reason = match failure {
                Failure::Missing => tr!("language-file-missing"),
                Failure::Unreadable => tr!("language-file-unreadable"),
                Failure::TooLarge => tr!("language-file-too-large"),
                Failure::NotText => tr!("language-file-not-text"),
                Failure::Empty => tr!("language-file-empty"),
            };
            vec![(tr!("language-file-failed", name = name.as_str(), reason = reason), true)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_i18n::UserCatalog;

    #[test]
    fn the_language_file_s_lines_say_what_is_in_use_and_what_is_wrong() {
        let info = UserCatalog {
            code: "de",
            named: true,
            messages: 3,
            errors: 0,
        };
        let name = || "de.ftl".to_owned();
        let lines = |shown| status_lines(&shown);
        assert_eq!(lines(Shown::NoFile).len(), 1);
        assert!(lines(Shown::NoFile)[0].1, "a warning: nothing is used yet");
        let in_use = lines(Shown::InUse { name: name(), info });
        assert_eq!(in_use.len(), 1);
        assert!(in_use[0].0.contains("de.ftl") && in_use[0].0.contains('3') && !in_use[0].1);
        let partial = lines(Shown::InUse {
            name: name(),
            info: UserCatalog {
                errors: 2,
                named: false,
                ..info
            },
        });
        assert_eq!(partial.len(), 3);
        assert!(partial[1].1 && partial[2].1);
        assert!(!lines(Shown::NextLaunch { name: name() })[0].1);
        for failure in [
            Failure::Missing,
            Failure::Unreadable,
            Failure::TooLarge,
            Failure::NotText,
            Failure::Empty,
        ] {
            let failed = lines(Shown::Failed { name: name(), failure });
            assert!(failed[0].1 && failed[0].0.contains("de.ftl"), "{failure:?}");
        }
    }
}
