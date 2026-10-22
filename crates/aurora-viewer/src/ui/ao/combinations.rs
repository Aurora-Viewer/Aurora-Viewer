//! Guided group/track edits; immutable selection is revalidated before sending.
use super::*;
use std::collections::HashSet;

pub(super) struct Dialog {
    pub set: Uuid,
    pub state: State,
    pub track: bool,
    pub name: String,
    pub selected: HashSet<Uuid>,
    pub rename: Option<Uuid>,
}

impl Dialog {
    pub fn new(set: Uuid, state: State) -> Self {
        Self {
            set,
            state,
            track: false,
            name: aurora_i18n::tr!("ao-new-group").into(),
            selected: HashSet::new(),
            rename: None,
        }
    }

    pub fn rename(set: Uuid, state: State, id: Uuid, name: &str, track: bool) -> Self {
        Self {
            set,
            state,
            track,
            name: name.into(),
            selected: HashSet::new(),
            rename: Some(id),
        }
    }

    fn validate(&self, set: &Set) -> Result<(), String> {
        inventory::valid_name(&self.name)?;
        let c = set.states.get(&self.state);
        if set.id != self.set
            || self
                .rename
                .is_some_and(|id| c.is_none_or(|c| inventory::editor::container_members(c, id).is_err()))
            || self
                .selected
                .iter()
                .any(|id| c.is_none_or(|c| !c.steps.iter().any(|s| !s.group && s.id == *id && s.members.len() == 1)))
        {
            return Err(aurora_i18n::tr!("ao-not-found").into());
        }
        let name = self.name.trim();
        if c.is_some_and(|c| {
            c.steps.iter().any(|s| s.group && Some(s.id) != self.rename && s.name == name)
                || c.tracks.iter().any(|t| Some(t.id) != self.rename && t.name == name)
        }) {
            return Err(aurora_i18n::tr!("ao-combine-name-exists").into());
        }
        Ok(())
    }
}

pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    catalog: &inventory::Catalog,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    if state.busy() {
        return;
    }
    let Some(dialog) = state.combination.as_mut() else { return };
    let Some(set) = catalog.sets.iter().find(|s| s.id == dialog.set) else {
        state.combination = None;
        return;
    };
    if state.state != dialog.state || state.selected_set != dialog.set {
        state.combination = None;
        return;
    }
    let mut cancel = false;
    let mut submit = false;
    let modal = widgets::Modal::new(egui::Id::unique("ao_combination")).show(ctx, |ui| {
        ui.set_width(410.0);
        ui.label(
            RichText::new(aurora_i18n::text(if dialog.rename.is_some() {
                "ao-rename-combination"
            } else {
                "ao-combine-title"
            }))
            .strong(),
        );
        if dialog.rename.is_none() {
            let before = dialog.track;
            ui.columns(2, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    let key = if index == 0 {
                        "ao-combine-group-choice"
                    } else {
                        "ao-combine-track-choice"
                    };
                    let label = format!(
                        "{}\n{}",
                        aurora_i18n::text(if index == 0 { "ao-group" } else { "ao-track" }),
                        aurora_i18n::text(key)
                    );
                    if column
                        .add(
                            egui::Button::selectable(dialog.track == (index == 1), label)
                                .wrap()
                                .min_size(Vec2::new(column.available_width(), 70.0)),
                        )
                        .clicked()
                    {
                        dialog.track = index == 1;
                    }
                }
            });
            if dialog.track != before
                && (dialog.name == aurora_i18n::tr!("ao-new-group") || dialog.name == aurora_i18n::tr!("ao-new-track"))
            {
                dialog.name = aurora_i18n::text(if dialog.track { "ao-new-track" } else { "ao-new-group" }).into();
            }
            ui.label(
                RichText::new(aurora_i18n::text(if dialog.track {
                    "ao-track-example"
                } else {
                    "ao-group-example"
                }))
                .color(p.muted),
            );
        }
        ui.label(aurora_i18n::text(if dialog.track { "ao-track-name" } else { "ao-group-name" }));
        ui.add(egui::TextEdit::singleline(&mut dialog.name).desired_width(f32::INFINITY));
        if dialog.rename.is_none()
            && let Some(c) = set.states.get(&dialog.state)
            && c.steps.iter().any(|s| !s.group)
        {
            let title = aurora_i18n::tr!("ao-combine-existing", state = dialog.state.label());
            widgets::collapsing(ui, p, "ao_combine_existing", &title, !dialog.selected.is_empty(), |ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ao-combine-existing-help")).small().color(p.muted));
                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                    for step in c.steps.iter().filter(|s| !s.group && s.members.len() == 1) {
                        let mut checked = dialog.selected.contains(&step.id);
                        if widgets::checkbox(ui, p, &mut checked, &step.name).changed() {
                            if checked {
                                dialog.selected.insert(step.id);
                            } else {
                                dialog.selected.remove(&step.id);
                            }
                        }
                    }
                });
            });
        }
        let valid = dialog.validate(set);
        if let Err(reason) = &valid {
            ui.label(RichText::new(reason).color(p.warn));
        }
        ui.horizontal(|ui| {
            let label = if dialog.rename.is_some() {
                "common-save"
            } else if dialog.track {
                "ao-create-track"
            } else {
                "ao-create-group"
            };
            submit = ui.add_enabled(valid.is_ok(), egui::Button::new(aurora_i18n::text(label))).clicked();
            cancel = widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
        });
    });
    if submit {
        let result = if let Some(id) = dialog.rename {
            inventory::editor::rename_container(inv, set, dialog.state, id, &dialog.name).map(|change| (change, id))
        } else {
            inventory::editor::combine(inv, set, dialog.state, &dialog.name, dialog.track, &dialog.selected)
        };
        match result {
            Ok((change, id)) => {
                out.push(Action::EditContainer(
                    change,
                    EditorTarget {
                        set: set.id,
                        state: dialog.state,
                        container: Some(id),
                    },
                ));
                state.combination = None;
            }
            Err(reason) => state.message = reason,
        }
    } else if cancel || modal.should_close() {
        state.combination = None;
    }
}
