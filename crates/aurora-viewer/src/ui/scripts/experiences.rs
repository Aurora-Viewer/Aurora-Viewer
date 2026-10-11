//! Object-script association selector, following LLLiveLSLEditor (Firestorm
//! indra/newview/llpreviewscript.cpp, originally LGPL 2.1). A single selector
//! includes "None" so removing an association is explicit and easy to find.

use crate::{theme::Palette, ui::widgets::flat_button};
use aurora_net::scripts::experiences::{ScriptExperience, ScriptExperiences};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct State {
    pub selected: Option<Uuid>,
    pub baseline: Option<Uuid>,
    pub loading: Option<Uuid>,
    choices: Vec<ScriptExperience>,
    errors: Vec<String>,
}

impl State {
    pub fn dirty(&self) -> bool {
        self.selected != self.baseline
    }

    pub fn begin(&mut self) -> Option<Uuid> {
        if self.loading.is_some() {
            return None;
        }
        let request = Uuid::new_v4();
        self.loading = Some(request);
        self.errors.clear();
        Some(request)
    }

    pub fn loaded(&mut self, request: Uuid, result: ScriptExperiences) {
        if self.loading != Some(request) {
            return;
        }
        self.loading = None;
        match result.associated {
            Ok(id) => {
                if !self.dirty() {
                    self.selected = Some(id);
                }
                self.baseline = Some(id);
            }
            Err(error) => self.errors.push(error),
        }
        match result.available {
            Ok(choices) => self.choices = choices,
            Err(error) => self.errors.push(error),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, editable: bool) -> bool {
        let name = match self.selected {
            Some(id) if id.is_nil() => aurora_i18n::tr!("script-experience-none").to_owned(),
            Some(id) => self
                .choices
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| id.to_string()),
            None if self.loading.is_some() => aurora_i18n::tr!("common-loading-2").to_owned(),
            None => aurora_i18n::tr!("script-experience-choose").to_owned(),
        };
        let mut retry = false;
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("script-experience"));
            ui.add_enabled_ui(editable, |ui| {
                egui::ComboBox::from_id_salt("experience")
                    .width(270.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.selected, Some(Uuid::nil()), aurora_i18n::tr!("script-experience-none"));
                        for choice in &self.choices {
                            ui.add_enabled_ui(choice.selectable, |ui| {
                                ui.selectable_value(&mut self.selected, Some(choice.id), &choice.name);
                            });
                        }
                    });
            });
            if !self.errors.is_empty() && self.loading.is_none() {
                retry = flat_button(ui, p, aurora_i18n::tr!("script-retry")).clicked();
            }
        });
        for error in &self.errors {
            ui.label(egui::RichText::new(error).color(p.warn));
        }
        retry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: Uuid) -> ScriptExperiences {
        ScriptExperiences {
            associated: Ok(id),
            available: Err("list unavailable".into()),
        }
    }

    #[test]
    fn replies_preserve_user_choice_and_ignore_stale_requests() {
        let mut state = State::default();
        let request = state.begin().expect("request");
        state.loaded(Uuid::new_v4(), reply(Uuid::from_u128(1)));
        assert_eq!(state.selected, None);
        state.selected = Some(Uuid::nil());
        state.loaded(request, reply(Uuid::from_u128(2)));
        assert_eq!(state.selected, Some(Uuid::nil()));
        assert_eq!(state.baseline, Some(Uuid::from_u128(2)));
        assert!(state.dirty());
    }

    #[test]
    fn list_failure_preserves_loaded_association_and_can_be_retried() {
        let mut state = State::default();
        let request = state.begin().expect("request");
        state.loaded(request, reply(Uuid::from_u128(2)));
        assert_eq!(state.selected, Some(Uuid::from_u128(2)));
        assert!(!state.dirty());
        assert!(state.begin().is_some());
        assert!(state.begin().is_none());
    }
}
