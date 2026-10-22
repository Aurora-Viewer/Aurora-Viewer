//! Preferences › Protections: session-local anti-spam settings and blocks,
//! and the trust given to web links (warning, trusted sites).
//! Rows participate in preference search, modified settings and page reset.

use super::super::OptionsResult;
use super::super::cx::{Cx, F, Nothing, field};
use crate::theme::Palette;
use crate::ui::widgets;
use crate::world::anti_spam::Queue;
use uuid::Uuid;

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
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
    cx.group(tr!("anti-spam-title"), |cx| {
        cx.keywords(tr!("anti-spam-title"))
            .toggle(tr!("anti-spam-enable"), tr!("anti-spam-enable-hint"), &field!(anti_spam.enabled));
        cx.greyed(!cx.s.anti_spam.enabled);
        cx.keywords(tr!("anti-spam-title"))
            .toggle(tr!("anti-spam-own"), tr!("anti-spam-own-hint"), &field!(anti_spam.own_objects));
        cx.keywords(tr!("anti-spam-title"))
            .toggle(tr!("anti-spam-global"), tr!("anti-spam-global-hint"), &field!(anti_spam.global));
        slider(
            cx,
            tr!("anti-spam-amount"),
            tr!("anti-spam-amount-hint"),
            &field!(anti_spam.amount),
            1..=1000,
            "",
        );
        slider(
            cx,
            tr!("anti-spam-period"),
            tr!("anti-spam-period-hint"),
            &field!(anti_spam.period),
            1..=60,
            " s",
        );
        slider(
            cx,
            tr!("anti-spam-newlines"),
            tr!("anti-spam-newlines-hint"),
            &field!(anti_spam.newlines),
            5..=1000,
            "",
        );
        slider(
            cx,
            tr!("anti-spam-sound-multiplier"),
            tr!("anti-spam-multiplier-hint"),
            &field!(anti_spam.sound_multiplier),
            1..=100,
            "",
        );
        slider(
            cx,
            tr!("anti-spam-preload-multiplier"),
            tr!("anti-spam-multiplier-hint"),
            &field!(anti_spam.preload_multiplier),
            1..=100,
            "",
        );
        cx.greyed(false);
        cx.note(tr!("anti-spam-session-hint"));
        cx.keywords(tr!("anti-spam-title"))
            .below()
            .row_with(tr!("anti-spam-session-blocks"), "", &Nothing, |ui, s, st, r| {
                session(ui, &p, s.anti_spam.global, &st.anti_spam_blocks, r);
                false
            });
    });
    // The web links of chat and profiles (link_trust.rs): the warning, and
    // the trusted sites (Aurora choice, the Legacy Viewer's list is fixed).
    cx.group(tr!("ui-options-web-links"), |cx| {
        cx.toggle(
            tr!("ui-options-warn-before-external-links"),
            tr!("ui-options-warn-before-opening-a-link-to-a-site-that"),
            &field!(warn_external_links),
        );
        let words = format!("{} {}", cx.s.trusted_sites.added.join(" "), crate::link_trust::built_in().join(" "));
        cx.wide().below().keywords(&words).row_with(
            tr!("ui-options-trusted-sites"),
            tr!("ui-options-trusted-sites-hint"),
            &field!(trusted_sites),
            |ui, s, st, _| trusted_sites(ui, &p, &mut s.trusted_sites, &mut st.trusted_site_typed),
        );
    });
}

/// The sites the user added (each removable), a field to add one, then the
/// built-in sites, each of which can stop being trusted. True on a change.
fn trusted_sites(ui: &mut egui::Ui, p: &Palette, sites: &mut crate::link_trust::TrustedSites, typed: &mut String) -> bool {
    use aurora_i18n::tr;
    use egui::RichText;
    let mut changed = false;
    ui.vertical(|ui| {
        if sites.added.is_empty() {
            ui.label(RichText::new(tr!("ui-options-trusted-sites-none")).size(12.0).color(p.muted));
        }
        let mut removed = None;
        for (i, site) in sites.added.iter().enumerate() {
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, p, "trash-filled", tr!("ui-options-trusted-sites-remove"), true) {
                    removed = Some(i);
                }
                ui.label(RichText::new(site).size(12.5).color(p.ink));
            });
        }
        if let Some(i) = removed {
            sites.added.remove(i);
            changed = true;
        }
        ui.add_space(4.0);
        let domain = crate::link_trust::normalize(typed);
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(typed)
                    .desired_width(260.0)
                    .hint_text(tr!("ui-options-trusted-sites-add-hint")),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let add = ui
                .add_enabled_ui(domain.is_some(), |ui| {
                    let label = tr!("ui-options-trusted-sites-add");
                    widgets::action_button(
                        ui,
                        p,
                        "circle-plus-filled",
                        label,
                        widgets::ActionKind::Flat,
                        egui::vec2(90.0, 26.0),
                    )
                })
                .inner
                .clicked();
            if let Some(domain) = &domain
                && (add || entered)
            {
                changed |= sites.add(domain);
                typed.clear();
            }
        });
        if domain.is_none() && !typed.trim().is_empty() {
            ui.label(RichText::new(tr!("ui-options-trusted-sites-invalid")).size(12.0).color(p.muted));
        }
        ui.add_space(4.0);
        widgets::collapsing(
            ui,
            p,
            "trusted_sites_built_in",
            tr!("ui-options-trusted-sites-built-in"),
            false,
            |ui| {
                for &builtin in crate::link_trust::built_in() {
                    let was = !sites.removed.iter().any(|r| r == builtin);
                    let mut on = was;
                    widgets::checkbox(ui, p, &mut on, builtin);
                    if on != was {
                        sites.set_built_in(builtin, on);
                        changed = true;
                    }
                }
            },
        );
    });
    changed
}

fn slider(cx: &mut Cx, label: &str, hint: &str, field: &F<u32>, range: std::ops::RangeInclusive<u32>, suffix: &str) {
    cx.keywords(aurora_i18n::tr!("anti-spam-title")).row(label, hint, field, |ui, s| {
        ui.add(
            egui::Slider::new((field.get_mut)(s), range)
                .suffix(suffix)
                .custom_parser(aurora_i18n::parse_number)
                .trailing_fill(true),
        )
        .changed()
    });
}

fn session(ui: &mut egui::Ui, p: &Palette, global: bool, blocks: &[(Queue, Uuid, String)], result: &mut OptionsResult) {
    ui.vertical(|ui| {
        if widgets::flat_button(ui, p, aurora_i18n::tr!("anti-spam-reset")).clicked() {
            result.reset_anti_spam = true;
        }
        if !blocks.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("anti-spam-blocks")
                .max_height(160.0)
                .show_rows(ui, ui.spacing().interact_size.y, blocks.len(), |ui, rows| {
                    for &(queue, source, ref name) in &blocks[rows] {
                        ui.push_id((queue, source), |ui| {
                            ui.horizontal(|ui| {
                                let kind = if global { aurora_i18n::tr!("anti-spam-all") } else { queue.label() };
                                ui.add(widgets::elided_label(
                                    ui,
                                    egui::RichText::new(format!("{kind} · {name}")).color(p.ink),
                                    (ui.available_width() - 95.0).max(40.0),
                                ))
                                .on_hover_text(format!("{name}\n{source}"));
                                if widgets::flat_button(ui, p, aurora_i18n::tr!("anti-spam-unblock")).clicked() {
                                    result.unblock_anti_spam = Some((queue, source));
                                }
                            });
                        });
                    }
                });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::anti_spam::Settings;

    fn text_rect(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| text_rect(s, text)),
            _ => None,
        }
    }

    #[test]
    fn anti_spam_native_buttons_reset_all_or_unblock_only_the_chosen_source() {
        for reset in [true, false] {
            let ctx = egui::Context::default();
            let p = crate::theme::Theme::default().palette();
            let settings = Settings {
                enabled: true,
                ..Default::default()
            };
            let source = Uuid::from_u128(678);
            let blocks = vec![(Queue::Sound, source, "Test source".to_owned())];
            let label = if reset {
                aurora_i18n::tr!("anti-spam-reset")
            } else {
                aurora_i18n::tr!("anti-spam-unblock")
            };
            let draw = |events| {
                let mut result = OptionsResult::default();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(850.0, 1400.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.horizontal(|ui| session(ui, &p, settings.global, &blocks, &mut result));
                    },
                );
                // Headless input tests inspect shapes without uploading font textures.
                output.textures_delta.clear();
                let reset_rect = output
                    .shapes
                    .iter()
                    .find_map(|s| text_rect(&s.shape, aurora_i18n::tr!("anti-spam-reset")))
                    .expect("reset");
                let unblock_rect = output
                    .shapes
                    .iter()
                    .find_map(|s| text_rect(&s.shape, aurora_i18n::tr!("anti-spam-unblock")))
                    .expect("unblock");
                assert!(
                    unblock_rect.top() > reset_rect.bottom(),
                    "session rows must stack vertically inside a horizontal preference control"
                );
                let rect = output
                    .shapes
                    .iter()
                    .find_map(|s| text_rect(&s.shape, label))
                    .expect("action visible");
                (result, rect.center())
            };
            let mut at = egui::Pos2::ZERO;
            for _ in 0..4 {
                at = draw(Vec::new()).1;
            }
            let mut clicked = OptionsResult::default();
            for pressed in [true, false] {
                clicked = draw(vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ])
                .0;
            }
            assert_eq!(clicked.reset_anti_spam, reset);
            assert_eq!(clicked.unblock_anti_spam, (!reset).then_some((Queue::Sound, source)));
            assert!(!clicked.changed, "session actions must not save preferences");
        }
    }
}
