//! Toolbar AO controls, reusing the manager's account and movement preferences.
//! The Legacy Viewer's small AO panel also shares its set selector with the
//! manager (indra/newview/ao.cpp); Aurora adds the requested movement controls.
use super::*;
use crate::movement::Preferences;

#[derive(Default)]
pub struct Quick {
    pub open: bool,
    anchor: Option<(egui::Response, egui::RectAlign, u64)>,
    list_was_open: bool,
}

impl Quick {
    pub fn anchor(&mut self, response: &egui::Response, align: egui::RectAlign) {
        if response.clicked() {
            self.open = !self.open;
        }
        self.anchor = Some((response.clone(), align, response.ctx.cumulative_frame_nr()));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        catalog: &inventory::Catalog,
        runtime: &mut Runtime,
        account: &mut Account,
        prefs: &mut Preferences,
        manager_open: &mut bool,
        busy: bool,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let Some((anchor, align, frame)) = &self.anchor else { return out };
        if *frame != ctx.cumulative_frame_nr() {
            self.open = false;
            self.list_was_open = false;
            return out;
        }
        let before = prefs.clone();
        let was_open = self.open;
        let mut list_open = false;
        let mut manager_requested = false;
        let shown = egui::Popup::from_response(anchor)
            .open_bool(&mut self.open)
            .align(if *align == egui::RectAlign::TOP {
                egui::RectAlign::TOP_END
            } else {
                *align
            })
            .gap(7.0)
            .frame(egui::Frame::NONE)
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .show(|ui| {
                widgets::overlay(ui, p, egui::Margin::same(10), |ui| {
                    ui.set_width(310.0);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.label(RichText::new(aurora_i18n::tr!("ao-title")).strong());
                    ui.label(aurora_i18n::tr!("ao-set"));
                    ui.add_enabled_ui(!busy && !catalog.sets.is_empty(), |ui| {
                        let selected = catalog.selected(account.selected);
                        list_open = egui::ComboBox::from_id_salt("ao_quick_set")
                            .width(ui.available_width())
                            .selected_text(selected.map_or(aurora_i18n::tr!("common-none-2"), |s| &s.name))
                            .show_ui(ui, |ui| {
                                for set in &catalog.sets {
                                    if ui.selectable_label(selected.is_some_and(|s| s.id == set.id), &set.name).clicked()
                                        && account.selected != set.id
                                    {
                                        account.selected = set.id;
                                        out.push(Action::Preferences);
                                    }
                                }
                            })
                            .inner
                            .is_some();
                    });
                    if super::enabled(ui, p, runtime, account) {
                        out.push(Action::Preferences);
                    }
                    let ready = catalog.swimming_ready(account.selected);
                    if !ready {
                        prefs.swimming = false;
                    }
                    let mut swimming = prefs.swimming;
                    ui.add_enabled_ui(ready, |ui| {
                        widgets::switch_labeled(ui, p, &mut swimming, aurora_i18n::tr!("movement-swimming"));
                    });
                    if swimming != prefs.swimming {
                        prefs.set_water_mode(false, swimming, ready);
                    }
                    let mut walking = prefs.water_walk;
                    widgets::switch_labeled(ui, p, &mut walking, aurora_i18n::tr!("movement-water-walk"));
                    if walking != prefs.water_walk {
                        prefs.set_water_mode(true, walking, ready);
                    }
                    if !ready {
                        if catalog.failed {
                            ui.label(RichText::new(aurora_i18n::tr!("ao-fetch-failed")).color(p.warn));
                        } else if catalog.loading {
                            ui.label(RichText::new(aurora_i18n::tr!("ao-loading")).color(p.muted));
                        } else {
                            super::swimming_warning(ui, p);
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new(aurora_i18n::tr!("ao-quick-boosts")).strong());
                    // Leave room for the multiplier's value field in each half
                    // of the compact panel, so one row cannot widen the next.
                    ui.spacing_mut().slider_width = 80.0;
                    for (label, enabled, multiplier) in [
                        (aurora_i18n::tr!("ao-quick-run"), &mut prefs.running, &mut prefs.run_multiplier),
                        (aurora_i18n::tr!("ao-quick-fly"), &mut prefs.fly_assist, &mut prefs.fly_multiplier),
                    ] {
                        ui.columns(2, |columns| {
                            widgets::switch_labeled(&mut columns[0], p, enabled, label);
                            columns[1].add_enabled(*enabled, egui::Slider::new(multiplier, 1.0..=5.0).prefix("×").step_by(0.1));
                        });
                    }
                    ui.add_space(4.0);
                    if ui.button(aurora_i18n::tr!("ao-quick-manager")).clicked() {
                        manager_requested = true;
                        widgets::open_floater(ctx, "ao", manager_open);
                        ui.close();
                    }
                });
            });
        // A set combo opens a child popup. Keep the parent through selecting
        // a set or dismissing that child, as for the toolbar microphone panel.
        let in_list = list_open || self.list_was_open;
        if let Some(shown) = shown {
            if was_open && shown.response.clicked_elsewhere() && anchor.clicked_elsewhere() && !in_list {
                self.open = false;
            } else if !self.open && in_list && !manager_requested {
                self.open = true;
            }
        }
        self.list_was_open = self.open && list_open;
        prefs.sanitize();
        if *prefs != before {
            out.push(Action::MovementPreferences);
        }
        out
    }
}
