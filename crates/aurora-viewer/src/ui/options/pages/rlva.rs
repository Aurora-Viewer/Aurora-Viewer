//! Preferences › RLVa: the scripted restrictions (RestrainedLove API), their
//! global switch, their options and the restrictions active in the session.
//! The page opens on a notice: RLVa is still very experimental.
//! Rows participate in preference search, modified settings and page reset.

use super::super::cx::{Cx, Nothing, field};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    cx.experimental_notice(tr!("ui-options-rlva-experimental"));
    cx.group(tr!("rlva-title"), |cx| {
        cx.keywords("RLVa RLV RestrainedLove")
            .toggle(tr!("rlva-enable"), tr!("rlva-enable-hint"), &field!(rlva_enabled));
        cx.note(tr!("rlva-session-hint"));
        cx.keywords("RLVa RLV console locks modifiers").row_with(
            tr!("rlva-tools-title"),
            tr!("rlva-tools-hint"),
            &Nothing,
            |ui, _, state, _| {
                if ui.button(tr!("rlva-tools-open")).clicked() {
                    state.rlva_tools.open = true;
                }
                false
            },
        );
        cx.greyed(!cx.s.rlva_enabled);
        cx.toggle(tr!("rlva-ooc"), tr!("rlva-ooc-hint"), &field!(rlva.allow_ooc));
        cx.toggle(tr!("rlva-ellipsis"), tr!("rlva-ellipsis-hint"), &field!(rlva.show_ellipsis));
        cx.toggle(
            tr!("rlva-redirect-typing"),
            tr!("rlva-redirect-typing-hint"),
            &field!(rlva.show_redirect_typing),
        );
        cx.toggle(
            tr!("rlva-replace-unlocked"),
            tr!("rlva-replace-unlocked-hint"),
            &field!(rlva.replace_unlocked),
        );
        cx.toggle(
            tr!("rlva-experimental"),
            tr!("rlva-experimental-hint"),
            &field!(rlva.experimental_commands),
        );
        cx.toggle(
            tr!("rlva-debug-commands"),
            tr!("rlva-debug-commands-hint"),
            &field!(rlva.debug_commands),
        );
        cx.toggle(
            tr!("rlva-debug-hide"),
            tr!("rlva-debug-hide-hint"),
            &field!(rlva.debug_hide_unset_duplicate),
        );
        cx.toggle(
            tr!("rlva-split-redirect"),
            tr!("rlva-split-redirect-hint"),
            &field!(rlva.split_redirect_chat),
        );
        cx.row(
            tr!("rlva-compatibility"),
            tr!("rlva-compatibility-hint"),
            &field!(rlva.compatibility_list),
            |ui, settings| {
                ui.add(egui::TextEdit::singleline(&mut settings.rlva.compatibility_list).desired_width(180.0))
                    .changed()
            },
        );
        cx.toggle(
            tr!("rlva-env-locks"),
            tr!("rlva-env-locks-hint"),
            &field!(rlva.allow_environment_locks),
        );
        cx.toggle(
            tr!("rlva-temp-controllers"),
            tr!("rlva-temp-controllers-hint"),
            &field!(rlva.temporary_controllers),
        );
        cx.greyed(!cx.s.rlva_enabled || !cx.s.rlva.temporary_controllers);
        cx.combo(
            tr!("rlva-experience-maturity"),
            tr!("rlva-experience-maturity-hint"),
            &field!(rlva.experience_maturity),
            &[
                (0, tr!("common-none")),
                (1, tr!("common-general")),
                (2, tr!("common-moderate")),
                (3, tr!("common-adult")),
            ],
        );
        cx.row(
            tr!("rlva-blocked-experiences"),
            tr!("rlva-blocked-experiences-hint"),
            &field!(rlva.blocked_experiences),
            |ui, settings| {
                ui.add(egui::TextEdit::singleline(&mut settings.rlva.blocked_experiences).desired_width(180.0))
                    .changed()
            },
        );
        cx.greyed(!cx.s.rlva_enabled);
        cx.toggle(
            tr!("rlva-shared-offers"),
            tr!("rlva-shared-offers-hint"),
            &field!(rlva.allow_shared_offers),
        );
        cx.toggle(tr!("rlva-im-queries"), tr!("rlva-im-queries-hint"), &field!(rlva.im_queries));
        cx.toggle(
            tr!("rlva-im-auto-share"),
            tr!("rlva-im-auto-share-hint"),
            &field!(rlva.im_queries_auto_share),
        );
        cx.toggle(tr!("rlva-shared-wear"), tr!("rlva-shared-wear-hint"), &field!(rlva.shared_wear));
        cx.greyed(!cx.s.rlva_enabled || cx.s.rlva.shared_wear);
        cx.toggle(tr!("rlva-auto-rename"), tr!("rlva-auto-rename-hint"), &field!(rlva.auto_rename));
        cx.greyed(!cx.s.rlva_enabled);
        cx.toggle(
            tr!("rlva-legacy-naming"),
            tr!("rlva-legacy-naming-hint"),
            &field!(rlva.legacy_naming),
        );
        cx.toggle(
            tr!("rlva-hide-attachments"),
            tr!("rlva-hide-attachments-hint"),
            &field!(rlva.hide_locked_attachments),
        );
        cx.toggle(
            tr!("rlva-hide-layers"),
            tr!("rlva-hide-layers-hint"),
            &field!(rlva.hide_locked_layers),
        );
        for (label, field) in [
            (tr!("rlva-add-prefix"), field!(rlva.wear_add_prefix)),
            (tr!("rlva-replace-prefix"), field!(rlva.wear_replace_prefix)),
        ] {
            cx.row(label, tr!("rlva-prefix-hint"), &field, |ui, settings| {
                ui.add(egui::TextEdit::singleline((field.get_mut)(settings)).desired_width(100.0))
                    .changed()
            });
        }
        cx.greyed(false);
        cx.keywords("RLVa RLV RestrainedLove")
            .below()
            .row_with(tr!("rlva-active"), "", &Nothing, |ui, _, st, _| {
                ui.vertical(|ui| {
                    if st.rlva_restrictions.is_empty() {
                        ui.label(tr!("rlva-none"));
                    } else {
                        egui::ScrollArea::vertical().id_salt("rlva-active").max_height(140.0).show_rows(
                            ui,
                            ui.spacing().interact_size.y,
                            st.rlva_restrictions.len(),
                            |ui, rows| {
                                for (source, command) in &st.rlva_restrictions[rows] {
                                    ui.label(format!("@{command}")).on_hover_text(source.to_string());
                                }
                            },
                        );
                    }
                });
                false
            });
    });
}
