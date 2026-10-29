//! The media filter's question (MediaAlert, MediaAlert2, MediaAlertSingle,
//! AudioAlert and EnableMediaFilter of notifications.xml) and its list in
//! the preferences (FloaterMediaLists, floater_media_lists.xml). The rules
//! are in `media::filter`.

use super::widgets;
use crate::media::filter::{Action, Edit, Question, Remember, Rule, Source};
use crate::theme::Palette;
use egui::RichText;

/// The height of the preferences' list before it scrolls.
const LIST_H: f32 = 180.0;

/// The question's state: the choice to remember, kept while the same URL
/// is asked about.
#[derive(Default)]
pub struct PromptUi {
    url: String,
    remember: Remember,
}

pub enum Answer {
    Allow(Remember),
    Deny(Remember),
    /// "Do not filter media", offered with the first question: the
    /// filter goes off and this media plays (callback_enable_media_filter).
    TurnOff,
}

/// The question about `q`, one at a time. Closing it (Escape) denies this
/// once, as the Legacy Viewer's alert leaves the media stopped.
pub fn prompt(ctx: &egui::Context, p: &Palette, q: &Question, first_use: bool, st: &mut PromptUi) -> Option<Answer> {
    if st.url != q.url {
        st.url = q.url.clone();
        st.remember = Remember::Once;
    }
    let mut answer = None;
    let modal = widgets::Modal::new(egui::Id::unique("media_filter_question")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            if let Some(t) = super::icons::global("shield-lock-filled") {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), rect, uv, p.ink);
            }
            ui.label(
                RichText::new(aurora_i18n::tr!("media-filter-title"))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
        });
        ui.add_space(6.0);
        if first_use {
            ui.label(RichText::new(aurora_i18n::tr!("media-filter-intro")).size(12.5).color(p.muted));
            ui.add_space(8.0);
        }
        let from = match q.source {
            Source::ParcelMedia => aurora_i18n::tr!("media-filter-parcel-media"),
            Source::PrimMedia => aurora_i18n::tr!("media-filter-prim-media"),
            Source::Music => aurora_i18n::tr!("media-filter-music"),
        };
        ui.label(RichText::new(from).size(12.5).color(p.muted));
        ui.add_space(2.0);
        ui.label(RichText::new(&q.domain).size(15.0).strong().color(p.ink));
        ui.add(
            egui::Label::new(RichText::new(&q.url).size(12.0).monospace().color(p.muted))
                .wrap()
                .selectable(true),
        );
        ui.add_space(10.0);
        let domain = aurora_i18n::tr!("media-filter-remember-domain", domain = q.domain.clone());
        for (value, label) in [
            (Remember::Once, aurora_i18n::tr!("media-filter-remember-once").to_string()),
            (Remember::Domain, domain),
            (Remember::Url, aurora_i18n::tr!("media-filter-remember-url").to_string()),
        ] {
            widgets::radio_value(ui, p, &mut st.remember, value, RichText::new(label).size(12.5).color(p.ink));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("media-filter-allow")).clicked() {
                answer = Some(Answer::Allow(st.remember));
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("media-filter-deny")).clicked() {
                answer = Some(Answer::Deny(st.remember));
            }
            if first_use {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("media-filter-turn-off")).clicked() {
                        answer = Some(Answer::TurnOff);
                    }
                });
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(Answer::Deny(Remember::Once));
    }
    answer
}

/// The account's list in Preferences › Sound and voice › Media: each site
/// with its answer (switched in place) and its removal, then a field to
/// add one. `rules` is `None` on the login screen. Changes go to `edits`
/// (applied by the app).
pub fn list(ui: &mut egui::Ui, p: &Palette, rules: Option<&[Rule]>, typed: &mut String, edits: &mut Vec<Edit>, width: f32) {
    let Some(rules) = rules else {
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-options-media-filter-logged-out"))
                .size(12.0)
                .color(p.muted),
        );
        return;
    };
    let options = [
        (aurora_i18n::tr!("ui-options-media-filter-allowed"), false),
        (aurora_i18n::tr!("ui-options-media-filter-blocked"), false),
    ];
    ui.vertical(|ui| {
        ui.set_width(width);
        if rules.is_empty() {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-options-media-filter-empty"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("media_filter_list")
            // a long list scrolls in its own height, the page does not grow
            .min_scrolled_height(LIST_H)
            .max_height(LIST_H)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for rule in rules {
                    ui.horizontal(|ui| {
                        if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("ui-options-media-filter-remove"), true) {
                            edits.push(Edit::Remove(rule.pattern.clone()));
                        }
                        let mut i = usize::from(rule.action == Action::Deny);
                        if widgets::segmented(ui, p, &mut i, &options) {
                            let action = if i == 0 { Action::Allow } else { Action::Deny };
                            edits.push(Edit::Set(rule.pattern.clone(), action));
                        }
                        let room = ui.available_width();
                        ui.add(widgets::elided_label(
                            ui,
                            RichText::new(&rule.pattern).size(12.5).color(p.ink),
                            room,
                        ));
                    });
                }
            });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(typed)
                    .desired_width((width - 2.0 * (90.0 + ui.spacing().item_spacing.x)).max(80.0))
                    .hint_text(aurora_i18n::tr!("ui-options-media-filter-add-hint")),
            );
            let entered = field.lost_focus() && widgets::enter_pressed(ui);
            let has_text = !typed.trim().is_empty();
            let size = egui::vec2(90.0, 26.0);
            let allow = ui
                .add_enabled_ui(has_text, |ui| {
                    widgets::action_button(
                        ui,
                        p,
                        "circle-check-filled",
                        aurora_i18n::tr!("media-filter-allow"),
                        widgets::ActionKind::Flat,
                        size,
                    )
                })
                .inner
                .clicked();
            let deny = ui
                .add_enabled_ui(has_text, |ui| {
                    widgets::action_button(
                        ui,
                        p,
                        "circle-x-filled",
                        aurora_i18n::tr!("media-filter-deny"),
                        widgets::ActionKind::Flat,
                        size,
                    )
                })
                .inner
                .clicked();
            if has_text && (allow || deny || entered) {
                edits.push(Edit::Set(std::mem::take(typed), if deny { Action::Deny } else { Action::Allow }));
            }
        });
    });
}
