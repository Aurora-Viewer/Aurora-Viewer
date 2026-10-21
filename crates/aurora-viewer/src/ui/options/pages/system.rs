//! Preferences › Language and updates: the interface's language, how the
//! new versions come, and whether the `secondlife:` links of web pages open
//! in this viewer.

use super::super::COMBO_W;
use super::super::cx::{Cx, Nothing, field};

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
                egui::ComboBox::from_id_salt(ui.id().with("ui-language"))
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
}
