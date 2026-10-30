//! Preferences › Protections: session-local anti-spam settings and blocks,
//! and the trust given to web links (warning, trusted sites). RLVa has its
//! own page (`rlva.rs`).
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
