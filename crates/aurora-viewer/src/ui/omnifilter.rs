//! The Omnifilter Rules Editor (Communicate › Omnifilter, toolbar
//! `omnifilter`): port of the Omnifilter floater (indra/newview/
//! omnifilter.cpp, floater_omnifilter.xml, originally LGPL 2.1). The rule
//! sets with New, Clone and Remove (never the Default set), the switch of
//! the filter, the rules of the set in use with their checkbox, Add,
//! Remove, Up and Down, the log of matches in SLT, and the selected rule's
//! fields: name, sender and content with their case and matching mode,
//! region, owner key, the kinds of message, the replacement and the
//! dialog answers with their delay. Every change is saved a few seconds
//! later (`crate::omnifilter::Engine::update`).
//!
//! The Legacy Viewer asks for a new set's name in a notification; here it
//! is a field of the window. Its notecard export and import are not here
//! yet.

use super::widgets::{self, Floater};
use crate::omnifilter::{Engine, MatchType, NameError, Rule, Source};
use crate::settings::Settings;
use crate::theme::Palette;
use egui::Vec2;
use uuid::Uuid;

pub const WINDOW: &str = "omnifilter";

/// Asking for the name of a set to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prompt {
    New,
    Clone,
    Remove,
}

#[derive(Default)]
pub struct OmnifilterUi {
    /// The selected rule of the set in use.
    selected: Option<usize>,
    /// (set, rule, name) the fields below were filled from.
    filled: Option<(usize, usize, String)>,
    name: String,
    owner: String,
    prompt: Option<Prompt>,
    prompt_name: String,
    prompt_error: Option<NameError>,
    /// The set the selection belongs to (None: the window has not shown
    /// one yet).
    set: Option<usize>,
}

fn source_label(s: Source) -> (&'static str, &'static str) {
    match s {
        Source::NearbyChat => (
            aurora_i18n::tr!("omnifilter-source-nearby"),
            aurora_i18n::tr!("omnifilter-source-nearby-tip"),
        ),
        Source::InstantMessage => (
            aurora_i18n::tr!("omnifilter-source-im"),
            aurora_i18n::tr!("omnifilter-source-im-tip"),
        ),
        Source::GroupChat => (
            aurora_i18n::tr!("omnifilter-source-group"),
            aurora_i18n::tr!("omnifilter-source-group-tip"),
        ),
        Source::GroupNotice => (
            aurora_i18n::tr!("omnifilter-source-notice"),
            aurora_i18n::tr!("omnifilter-source-notice-tip"),
        ),
        Source::GroupInvite => (
            aurora_i18n::tr!("omnifilter-source-invite"),
            aurora_i18n::tr!("omnifilter-source-invite-tip"),
        ),
        Source::ObjectChat => (
            aurora_i18n::tr!("omnifilter-source-object-chat"),
            aurora_i18n::tr!("omnifilter-source-object-chat-tip"),
        ),
        Source::ObjectInstantMessage => (
            aurora_i18n::tr!("omnifilter-source-object-im"),
            aurora_i18n::tr!("omnifilter-source-object-im-tip"),
        ),
        Source::ScriptError => (
            aurora_i18n::tr!("omnifilter-source-error"),
            aurora_i18n::tr!("omnifilter-source-error-tip"),
        ),
        Source::ScriptDialog => (
            aurora_i18n::tr!("omnifilter-source-dialog"),
            aurora_i18n::tr!("omnifilter-source-dialog-tip"),
        ),
        Source::InventoryOffer => (
            aurora_i18n::tr!("omnifilter-source-offer"),
            aurora_i18n::tr!("omnifilter-source-offer-tip"),
        ),
        Source::FriendshipOffer => (
            aurora_i18n::tr!("omnifilter-source-friendship"),
            aurora_i18n::tr!("omnifilter-source-friendship-tip"),
        ),
        Source::Lure => (
            aurora_i18n::tr!("omnifilter-source-lure"),
            aurora_i18n::tr!("omnifilter-source-lure-tip"),
        ),
        Source::TeleportRequest => (
            aurora_i18n::tr!("omnifilter-source-tp-request"),
            aurora_i18n::tr!("omnifilter-source-tp-request-tip"),
        ),
        Source::UrlRequest => (
            aurora_i18n::tr!("omnifilter-source-url"),
            aurora_i18n::tr!("omnifilter-source-url-tip"),
        ),
    }
}

fn match_label(m: MatchType) -> &'static str {
    match m {
        MatchType::Exact => aurora_i18n::tr!("omnifilter-match-exact"),
        MatchType::Substring => aurora_i18n::tr!("omnifilter-match-substring"),
        MatchType::Regex => aurora_i18n::tr!("omnifilter-match-regex"),
        MatchType::Other(_) => "?",
    }
}

/// The set's name as listed: the Default set in the interface's language
/// (reloadRules).
fn set_label(index: usize, name: &str) -> String {
    if index == 0 {
        aurora_i18n::tr!("omnifilter-default-name").to_owned()
    } else {
        name.to_owned()
    }
}

/// "HH:MM:SS" in SLT, and the full date for its tooltip (onLogLine).
fn log_times(t: std::time::SystemTime) -> (String, String) {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (y, mo, d, _, h, mi, s) = super::land::slt_parts(secs);
    (
        format!("{h:02}:{mi:02}:{s:02}"),
        format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} SLT"),
    )
}

/// One text field of a rule with its case and matching mode.
fn match_row(ui: &mut egui::Ui, p: &Palette, id: &str, kind: &mut MatchType, case_insensitive: &mut bool) {
    let mut sensitive = !*case_insensitive;
    if widgets::checkbox(ui, p, &mut sensitive, "Aa")
        .on_hover_text(aurora_i18n::tr!("omnifilter-case-tip"))
        .changed()
    {
        *case_insensitive = !sensitive;
    }
    egui::ComboBox::from_id_salt(id)
        .selected_text(match_label(*kind))
        .show_ui(ui, |ui| {
            for m in [MatchType::Exact, MatchType::Substring, MatchType::Regex] {
                ui.selectable_value(kind, m, match_label(m));
            }
        })
        .response
        .on_hover_text(aurora_i18n::tr!("omnifilter-match-tip"));
}

impl OmnifilterUi {
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, engine: &mut Engine, settings: &mut Settings, open: &mut bool) {
        if !*open {
            // closed while a name was being typed: it is kept
            self.commit_rename(engine);
            return;
        }
        let before = settings.omnifilter;
        let mut window = Floater::new(
            WINDOW,
            aurora_i18n::tr!("omnifilter-title"),
            egui::pos2(260.0, 120.0),
            Vec2::new(860.0, 560.0),
        );
        window.min_size = Vec2::new(800.0, 440.0);
        window.show(ctx, p, open, |ui| {
            self.sets_row(ui, p, engine, settings);
            widgets::divider(ui, p);
            // reloadRule: a set shown selects its first rule
            if self.set != Some(engine.current()) {
                self.set = Some(engine.current());
                self.selected = (!engine.current_set().rules.is_empty()).then_some(0);
            }
            if self.selected.is_some_and(|i| i >= engine.current_set().rules.len()) {
                self.selected = engine.current_set().rules.len().checked_sub(1);
            }
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(Vec2::new(250.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(250.0);
                    self.rules_column(ui, p, engine, settings, height);
                });
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("omnifilter_details")
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.details(ui, p, engine));
                });
            });
        });
        if settings.omnifilter != before {
            // another set chosen: the name typed in this one is kept first
            if settings.omnifilter.rule_set != before.rule_set {
                self.commit_rename(engine);
            }
            engine.enabled = settings.omnifilter.enabled;
            engine.select(settings.omnifilter.rule_set);
            settings.save();
        }
    }

    /// onNeedleNameChanged for a name typed and not committed yet (the field
    /// commits when it loses focus, which a click elsewhere in the same
    /// frame would not report): it goes to the rule it was typed for, before
    /// the selection or the set moves. A blank or taken name is dropped.
    fn commit_rename(&mut self, engine: &mut Engine) {
        let Some((set, index, name)) = &self.filled else {
            return;
        };
        let (set, index) = (*set, *index);
        if self.name == *name {
            return; // nothing typed: no allocation while the window is closed
        }
        let still_there = set == engine.current() && engine.current_set().rules.get(index).is_some_and(|r| r.name == *name);
        if still_there && engine.rename_rule(index, &self.name) {
            self.filled = Some((set, index, self.name.clone()));
        }
    }

    fn sets_row(&mut self, ui: &mut egui::Ui, p: &Palette, engine: &mut Engine, settings: &mut Settings) {
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("omnifilter-rule-sets"));
            let current = engine.current();
            let mut chosen = current;
            egui::ComboBox::from_id_salt("omnifilter_sets")
                .selected_text(set_label(current, &engine.current_set().name))
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (i, s) in engine.sets().iter().enumerate() {
                        ui.selectable_value(&mut chosen, i, set_label(i, &s.name));
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("omnifilter-rule-sets-tip"));
            if chosen != current {
                settings.omnifilter.rule_set = chosen;
                engine.select(chosen);
                self.prompt = None;
            }
            let button = |ui: &mut egui::Ui, icon, label, tip, enabled| {
                widgets::action_button_if(ui, p, icon, label, widgets::ActionKind::Flat, Vec2::new(0.0, 26.0), enabled)
                    .on_hover_text(tip)
                    .on_disabled_hover_text(tip)
                    .clicked()
            };
            if button(
                ui,
                "plus",
                aurora_i18n::tr!("omnifilter-new-set"),
                aurora_i18n::tr!("omnifilter-new-set-tip"),
                true,
            ) {
                self.ask(Prompt::New, aurora_i18n::tr!("omnifilter-new-set-name"));
            }
            if button(
                ui,
                "copy-filled",
                aurora_i18n::tr!("omnifilter-clone-set"),
                aurora_i18n::tr!("omnifilter-clone-set-tip"),
                true,
            ) {
                self.ask(Prompt::Clone, aurora_i18n::tr!("omnifilter-clone-set-name"));
            }
            let tip = if current == 0 {
                aurora_i18n::tr!("omnifilter-remove-default")
            } else {
                aurora_i18n::tr!("omnifilter-remove-set-tip")
            };
            if button(ui, "trash-filled", aurora_i18n::tr!("omnifilter-remove-set"), tip, current != 0) {
                self.prompt = Some(Prompt::Remove);
            }
        });
        let Some(prompt) = self.prompt else { return };
        ui.horizontal(|ui| {
            if prompt == Prompt::Remove {
                let name = engine.current_set().name.clone();
                ui.label(aurora_i18n::tr!("omnifilter-remove-set-confirm", name = name));
            } else {
                ui.label(if prompt == Prompt::New {
                    aurora_i18n::tr!("omnifilter-new-set-prompt")
                } else {
                    aurora_i18n::tr!("omnifilter-clone-set-prompt")
                });
                let r = ui.add(egui::TextEdit::singleline(&mut self.prompt_name).desired_width(200.0));
                if r.changed() {
                    self.prompt_error = None;
                }
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-ok")).clicked() || (enter && prompt != Prompt::Remove) {
                self.answer(prompt, engine, settings);
            }
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                self.prompt = None;
            }
        });
        if let Some(e) = self.prompt_error {
            ui.colored_label(
                p.danger,
                match e {
                    NameError::Blank => aurora_i18n::tr!("omnifilter-name-blank"),
                    NameError::Duplicate => aurora_i18n::tr!("omnifilter-name-duplicate"),
                },
            );
        }
    }

    fn ask(&mut self, prompt: Prompt, name: &str) {
        self.prompt = Some(prompt);
        self.prompt_name = name.to_owned();
        self.prompt_error = None;
    }

    fn answer(&mut self, prompt: Prompt, engine: &mut Engine, settings: &mut Settings) {
        let added = match prompt {
            Prompt::Remove => {
                if engine.remove_current_set() {
                    settings.omnifilter.rule_set = 0;
                }
                self.prompt = None;
                return;
            }
            Prompt::New | Prompt::Clone => {
                // a clone carries the name typed, a new set leaves this one
                self.commit_rename(engine);
                if prompt == Prompt::New {
                    engine.add_set(&self.prompt_name)
                } else {
                    engine.clone_set(&self.prompt_name)
                }
            }
        };
        match added {
            Ok(index) => {
                settings.omnifilter.rule_set = index;
                self.prompt = None;
                self.set = Some(index);
                // onNewRuleSetNameSelectedCallback: a new set starts with
                // one new rule
                if prompt == Prompt::New {
                    self.selected = Some(engine.add_rule(aurora_i18n::tr!("omnifilter-new-rule")));
                } else {
                    self.selected = (!engine.current_set().rules.is_empty()).then_some(0);
                }
            }
            Err(e) => self.prompt_error = Some(e),
        }
    }

    fn rules_column(&mut self, ui: &mut egui::Ui, p: &Palette, engine: &mut Engine, settings: &mut Settings, height: f32) {
        widgets::checkbox(ui, p, &mut settings.omnifilter.enabled, aurora_i18n::tr!("omnifilter-enable"));
        ui.add_space(4.0);
        // the switch, the buttons and the log's header take the rest: the
        // rules get half of what is left, the log the other half
        let lists = (height - 110.0).max(80.0);
        let list_height = lists * 0.5;
        let mut toggled = None;
        let mut picked = None;
        egui::ScrollArea::vertical()
            .id_salt("omnifilter_rules")
            .max_height(list_height)
            .min_scrolled_height(list_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, rule) in engine.current_set().rules.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let mut on = rule.enabled;
                        if widgets::checkbox(ui, p, &mut on, "").changed() {
                            toggled = Some((i, on));
                        }
                        let selected = self.selected == Some(i);
                        if ui.selectable_label(selected, rule.name.as_str()).clicked() {
                            picked = Some(i);
                        }
                    });
                }
            });
        if let Some((i, on)) = toggled {
            engine.rules_mut()[i].enabled = on;
        }
        if let Some(i) = picked {
            // the name typed for the rule left is kept, for that rule
            self.commit_rename(engine);
            self.selected = Some(i);
        }
        ui.horizontal(|ui| {
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-add")).clicked() {
                self.commit_rename(engine);
                self.selected = Some(engine.add_rule(aurora_i18n::tr!("omnifilter-new-rule")));
                self.filled = None;
            }
            let chosen = self.selected;
            ui.add_enabled_ui(chosen.is_some(), |ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-remove")).clicked()
                    && let Some(i) = chosen
                {
                    engine.remove_rule(i);
                    // selectPrevItem, else the first
                    let len = engine.current_set().rules.len();
                    self.selected = if len == 0 { None } else { Some(i.saturating_sub(1).min(len - 1)) };
                    self.filled = None;
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("omnifilter-up"))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-up-tip"))
                    .clicked()
                    && let Some(i) = chosen
                {
                    self.commit_rename(engine);
                    self.selected = Some(engine.move_rule(i, true));
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("omnifilter-down"))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-down-tip"))
                    .clicked()
                    && let Some(i) = chosen
                {
                    self.commit_rename(engine);
                    self.selected = Some(engine.move_rule(i, false));
                }
            });
        });
        widgets::divider(ui, p);
        widgets::header_row(
            ui,
            p,
            &[
                (aurora_i18n::tr!("omnifilter-log-time"), 80.0),
                (aurora_i18n::tr!("omnifilter-rule"), 160.0),
            ],
        );
        let log_height = (lists * 0.5).min(ui.available_height()).max(0.0);
        egui::ScrollArea::vertical()
            .id_salt("omnifilter_log")
            .max_height(log_height)
            .min_scrolled_height(log_height)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (t, name) in &engine.log {
                    let (short, full) = log_times(*t);
                    ui.horizontal(|ui| {
                        ui.add_sized(Vec2::new(80.0, 18.0), egui::Label::new(short)).on_hover_text(full);
                        ui.add(egui::Label::new(name.as_str()).truncate());
                    });
                }
            });
    }

    fn details(&mut self, ui: &mut egui::Ui, p: &Palette, engine: &mut Engine) {
        let Some(index) = self.selected else { return };
        let Some(original) = engine.current_set().rules.get(index) else {
            return;
        };
        let key = (engine.current(), index, original.name.clone());
        if self.filled.as_ref() != Some(&key) {
            self.name = original.name.clone();
            self.owner = if original.owner_id.is_nil() {
                String::new()
            } else {
                original.owner_id.to_string()
            };
            self.filled = Some(key);
        }
        let mut rule: Rule = original.clone();
        let mut rename = None;
        let width = ui.available_width() - 120.0;
        egui::Grid::new("omnifilter_rule")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(aurora_i18n::tr!("omnifilter-rule-name"));
                let r = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(width));
                if r.lost_focus() {
                    rename = Some(self.name.clone());
                }
                ui.end_row();

                ui.label(aurora_i18n::tr!("omnifilter-sender"));
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut rule.sender_name).desired_width(width))
                        .on_hover_text(aurora_i18n::tr!("omnifilter-sender-tip"));
                    ui.horizontal(|ui| {
                        match_row(
                            ui,
                            p,
                            "omnifilter_sender_match",
                            &mut rule.sender_match,
                            &mut rule.sender_case_insensitive,
                        )
                    });
                });
                ui.end_row();

                ui.label(aurora_i18n::tr!("omnifilter-content"));
                ui.vertical(|ui| {
                    ui.add(egui::TextEdit::multiline(&mut rule.content).desired_width(width).desired_rows(3))
                        .on_hover_text(aurora_i18n::tr!("omnifilter-content-tip"));
                    ui.horizontal(|ui| {
                        match_row(
                            ui,
                            p,
                            "omnifilter_content_match",
                            &mut rule.content_match,
                            &mut rule.content_case_insensitive,
                        );
                        // onMatchDialogButtonLabelClicked
                        if widgets::flat_button(ui, p, aurora_i18n::tr!("omnifilter-dialog-button"))
                            .on_hover_text(aurora_i18n::tr!("omnifilter-dialog-button-tip"))
                            .clicked()
                        {
                            if !rule.content.is_empty() {
                                rule.content.push('\n');
                            }
                            rule.content.push_str("button_name=BUTTON_LABEL");
                        }
                    });
                });
                ui.end_row();

                ui.label(aurora_i18n::tr!("omnifilter-region"));
                ui.add(egui::TextEdit::singleline(&mut rule.region_name).desired_width(width))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-region-tip"));
                ui.end_row();

                ui.label(aurora_i18n::tr!("omnifilter-owner"));
                // onOwnerChanged: a key that is not one shows in the emphasis colour
                let valid = self.owner.is_empty() || Uuid::parse_str(self.owner.trim()).is_ok();
                let mut field = egui::TextEdit::singleline(&mut self.owner)
                    .desired_width(width)
                    .hint_text("00000000-0000-0000-0000-000000000000");
                if !valid {
                    field = field.text_color(p.warn);
                }
                if ui.add(field).on_hover_text(aurora_i18n::tr!("omnifilter-owner-tip")).changed() {
                    rule.owner_id = Uuid::parse_str(self.owner.trim()).unwrap_or(Uuid::nil());
                }
                ui.end_row();

                ui.label(aurora_i18n::tr!("omnifilter-sources"));
                egui::Grid::new("omnifilter_sources")
                    .num_columns(3)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (n, s) in Source::ALL.into_iter().enumerate() {
                            let (label, tip) = source_label(s);
                            let mut on = rule.has_source(s);
                            if widgets::checkbox(ui, p, &mut on, label).on_hover_text(tip).changed() {
                                rule.set_source(s, on);
                            }
                            if n % 3 == 2 {
                                ui.end_row();
                            }
                        }
                    });
                ui.end_row();
            });
        widgets::divider(ui, p);
        egui::Grid::new("omnifilter_actions")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(aurora_i18n::tr!("omnifilter-chat-replace"));
                ui.add(egui::TextEdit::singleline(&mut rule.chat_replace).desired_width(width))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-chat-replace-tip"));
                ui.end_row();
                ui.label(aurora_i18n::tr!("omnifilter-button-reply"));
                ui.add(egui::TextEdit::singleline(&mut rule.button_reply).desired_width(width))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-button-reply-tip"));
                ui.end_row();
                ui.label(aurora_i18n::tr!("omnifilter-textbox-reply"));
                ui.add(
                    egui::TextEdit::multiline(&mut rule.textbox_reply)
                        .desired_width(width)
                        .desired_rows(2),
                )
                .on_hover_text(aurora_i18n::tr!("omnifilter-textbox-reply-tip"));
                ui.end_row();
                ui.label(aurora_i18n::tr!("omnifilter-reply-delay"));
                ui.horizontal(|ui| {
                    ui.add(widgets::restricted_number(
                        egui::DragValue::new(&mut rule.reply_delay)
                            .range(0.0..=3.0)
                            .speed(0.1)
                            .fixed_decimals(1),
                        widgets::TextPolicy::Decimal,
                    ))
                    .on_hover_text(aurora_i18n::tr!("omnifilter-reply-delay-tip"));
                    ui.label(aurora_i18n::tr!("omnifilter-seconds"));
                });
                ui.end_row();
            });
        if rule != *original {
            engine.rules_mut()[index] = rule;
        }
        // onNeedleNameChanged: a blank or taken name goes back
        if let Some(name) = rename {
            if engine.rename_rule(index, &name) {
                self.filled = Some((engine.current(), index, name));
            } else {
                self.name = engine.current_set().rules[index].name.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_typed_then_another_rule_picked_goes_to_the_rule_it_was_typed_for() {
        let mut engine = Engine::with_sets(vec![crate::omnifilter::RuleSet {
            name: crate::omnifilter::DEFAULT_SET.into(),
            rules: vec![Rule::new("First"), Rule::new("Second")],
        }]);
        let mut ui = OmnifilterUi {
            filled: Some((0, 0, "First".into())),
            name: "Renamed".into(),
            ..Default::default()
        };
        ui.commit_rename(&mut engine);
        assert_eq!(engine.current_set().rules[0].name, "Renamed");
        assert_eq!(engine.current_set().rules[1].name, "Second");
        // nothing left to commit the second time
        ui.commit_rename(&mut engine);
        assert_eq!(engine.current_set().rules[0].name, "Renamed");
        // a taken or blank name is dropped, as the field's own commit does
        ui.name = "Second".into();
        ui.commit_rename(&mut engine);
        ui.name = "  ".into();
        ui.commit_rename(&mut engine);
        assert_eq!(engine.current_set().rules[0].name, "Renamed");
        // the rule it was typed for is gone (moved, removed): nothing else is renamed
        ui.filled = Some((0, 1, "Elsewhere".into()));
        ui.name = "Other".into();
        ui.commit_rename(&mut engine);
        assert_eq!(engine.current_set().rules[1].name, "Second");
    }

    #[test]
    fn the_window_keeps_its_size() {
        let mut engine = Engine::with_sets(vec![crate::omnifilter::RuleSet {
            name: crate::omnifilter::DEFAULT_SET.into(),
            rules: vec![Rule::new("Shout spam"), Rule::new("Dialog")],
        }]);
        let mut settings = Settings::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(860.0, 560.0), Vec2::new(800.0, 440.0)],
            aurora_i18n::tr!("omnifilter-log-time"),
            OmnifilterUi::default,
            |ui, ctx, p| ui.show(ctx, p, &mut engine, &mut settings, &mut true),
        );
    }

    #[test]
    fn log_times_are_slt() {
        // 2026-01-15 20:00:00 UTC is 12:00:00 PST
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_768_507_200);
        let (short, full) = log_times(t);
        assert_eq!(short, "12:00:00");
        assert_eq!(full, "2026-01-15 12:00:00 SLT");
    }
}
