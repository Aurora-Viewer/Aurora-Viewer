//! AO management floater: Firestorm AO::updateList / onDrop / onClick*.

use super::widgets::{self, Floater};
use crate::ao::{Account, Cycle, Set, State, inventory, runtime::Runtime};
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use aurora_net::inventory::operations::Mutation;
use egui::{RichText, Vec2};
use uuid::Uuid;

#[cfg(test)]
mod tests;

pub enum Action {
    Edit(Mutation, Option<Uuid>),
    Create(Set),
    Import(Uuid),
    Reload,
    Preferences,
}

pub struct Pending {
    pub request: Uuid,
    pub refresh: Vec<Uuid>,
    pub select: Option<Uuid>,
}

#[derive(Default)]
pub struct AoUi {
    pub pending: Option<Pending>,
    pub import: Option<Uuid>,
    pub import_folder: Option<Uuid>,
    pub message: String,
    pub warnings: Vec<String>,
    pub state: State,
    pub container: Option<Uuid>,
    name: String,
    container_name: String,
    new_dialog: Option<SetDialog>,
    remove: Option<Uuid>,
    picker: Option<bool>,
    folder: Uuid,
    filter: String,
    rows: Vec<(Uuid, String)>,
    rows_key: Option<(u64, String, bool)>,
    cycle_edit: Option<(Uuid, Cycle)>,
    selected_set: Uuid,
}

#[derive(Clone, Copy)]
enum SetDialog {
    New,
    Clone,
    Rename,
}

impl AoUi {
    pub fn busy(&self) -> bool {
        self.pending.is_some() || self.import.is_some() || self.import_folder.is_some()
    }

    pub fn reset_cycle(&mut self) {
        self.cycle_edit = None;
    }
}

fn push(out: &mut Vec<Action>, state: &mut AoUi, result: Result<Mutation, String>) {
    match result {
        Ok(change) => out.push(Action::Edit(change, None)),
        Err(reason) => state.message = reason,
    }
}

fn toggle(ui: &mut egui::Ui, p: &Palette, key: &'static str, value: &mut bool) -> bool {
    ui.horizontal(|ui| {
        let changed = widgets::switch(ui, p, value).changed();
        let label = ui.add(egui::Label::new(aurora_i18n::text(key)).sense(egui::Sense::click()));
        if label.clicked() {
            *value = !*value;
            true
        } else {
            changed
        }
    })
    .inner
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &mut Inventory,
    catalog: &inventory::Catalog,
    runtime: &mut Runtime,
    account: &mut Account,
    state: &mut AoUi,
    open: &mut bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    let selected = catalog.selected(account.selected);
    let selected_id = selected.map_or(Uuid::nil(), |s| s.id);
    if state.selected_set != selected_id {
        state.selected_set = selected_id;
        state.container = None;
        state.cycle_edit = None;
    }
    let title = aurora_i18n::tr!("ao-title");
    Floater::new("ao", title, egui::pos2(220.0, 120.0), Vec2::new(560.0, 550.0))
        .help(aurora_i18n::tr!("ao-help"))
        .show(ctx, p, open, |ui| {
            ui.horizontal(|ui| {
                if toggle(ui, p, "ao-enabled", &mut account.enabled) {
                    if account.enabled {
                        runtime.stands = true;
                        runtime.paused = false;
                    }
                    out.push(Action::Preferences);
                }
                ui.add_enabled_ui(account.enabled, |ui| {
                    toggle(ui, p, "ao-pause", &mut runtime.paused);
                });
                toggle(ui, p, "ao-stands", &mut runtime.stands);
            });
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("ao-set"));
                egui::ComboBox::from_id_salt("ao_set")
                    .selected_text(selected.map_or(aurora_i18n::tr!("common-none-2"), |s| &s.name))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for set in &catalog.sets {
                            if ui.selectable_label(selected_id == set.id, &set.name).clicked() {
                                account.selected = set.id;
                                out.push(Action::Preferences);
                            }
                        }
                    });
                if widgets::icon_button(ui, p, "reload", aurora_i18n::tr!("ao-reload"), !state.busy()) {
                    out.push(Action::Reload);
                }
            });
            ui.add_enabled_ui(!state.busy() && !catalog.loading, |ui| {
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-new")).clicked() {
                        state.name.clear();
                        state.new_dialog = Some(SetDialog::New);
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-import")).clicked() {
                        state.picker = Some(true);
                        state.folder = inv.root;
                    }
                    if let Some(set) = selected {
                        if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-duplicate")).clicked() {
                            state.name = set.name.clone();
                            state.new_dialog = Some(SetDialog::Clone);
                        }
                        if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), true) {
                            state.remove = Some(set.id);
                        }
                    }
                });
            });
            if catalog.failed {
                ui.label(RichText::new(aurora_i18n::tr!("ao-fetch-failed")).color(p.warn));
            } else if catalog.loading {
                ui.label(RichText::new(aurora_i18n::tr!("ao-loading")).color(p.muted));
            }
            if state.busy() {
                ui.label(RichText::new(aurora_i18n::tr!("ao-saving")).color(p.muted));
            }
            if !state.message.is_empty() {
                ui.label(RichText::new(&state.message).color(p.warn));
            }
            if !state.warnings.is_empty() {
                // the title is also the id, as it was for the stock header
                let warnings = aurora_i18n::tr!("ao-import-warnings");
                widgets::collapsing(ui, p, warnings, warnings, false, |ui| {
                    egui::ScrollArea::vertical().max_height(85.0).show(ui, |ui| {
                        for warning in &state.warnings {
                            ui.label(warning);
                        }
                    });
                });
            }
            let Some(set) = selected else {
                ui.label(aurora_i18n::tr!("ao-empty"));
                return;
            };
            ui.separator();
            ui.add_enabled_ui(!state.busy(), |ui| {
                ui.horizontal(|ui| {
                    let mut edit = set.clone();
                    if toggle(ui, p, "ao-sits", &mut edit.sits) {
                        push(&mut out, state, inventory::save_set(inv, &edit));
                    }
                    if toggle(ui, p, "ao-smart", &mut edit.smart) {
                        push(&mut out, state, inventory::save_set(inv, &edit));
                    }
                });
                let mut edit = set.clone();
                if toggle(ui, p, "ao-mouselook", &mut edit.mouselook) {
                    push(&mut out, state, inventory::save_set(inv, &edit));
                }
                ui.horizontal(|ui| {
                    let mut is_default = set.default;
                    if toggle(ui, p, "ao-default", &mut is_default) {
                        let mut change = Mutation::default();
                        let result: Result<(), String> = (|| {
                            for old in catalog.sets.iter().filter(|s| s.id == set.id || (is_default && s.default)) {
                                let mut edit = old.clone();
                                edit.default = is_default && old.id == set.id;
                                let saved = inventory::save_set(inv, &edit)?;
                                change.operations.extend(saved.operations);
                                change.refresh.extend(saved.refresh);
                            }
                            Ok(())
                        })();
                        push(&mut out, state, result.map(|()| change));
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("common-rename")).clicked() {
                        state.name = set.name.clone();
                        state.new_dialog = Some(SetDialog::Rename);
                    }
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("ao_state")
                    .selected_text(state.state.label())
                    .width(210.0)
                    .show_ui(ui, |ui| {
                        for s in State::ALL {
                            if ui.selectable_value(&mut state.state, s, s.label()).changed() {
                                state.container = None;
                                state.cycle_edit = None;
                            }
                        }
                    });
                if let Some(current) = runtime.current {
                    ui.label(RichText::new(aurora_i18n::tr!("ao-current", state = current.label())).color(p.muted));
                }
            });
            let config = set.states.get(&state.state).cloned().unwrap_or_default();
            if let Some(container) = state.container
                && !config.steps.iter().any(|s| s.id == container)
                && !config.tracks.iter().any(|t| t.id == container)
            {
                state.container = None;
            }
            ui.horizontal(|ui| {
                if widgets::icon_button(
                    ui,
                    p,
                    "caret-left-filled",
                    aurora_i18n::tr!("ao-previous"),
                    !config.steps.is_empty(),
                ) {
                    runtime.cycle(state.state, &config, true);
                }
                if widgets::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("ao-next"), !config.steps.is_empty()) {
                    runtime.cycle(state.state, &config, false);
                }
                ui.add_enabled_ui(!state.busy(), |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-add")).clicked() {
                        state.picker = Some(false);
                        state.folder = crate::world::inventory::actions::system(inv, 20).unwrap_or(inv.root);
                    }
                });
            });
            if let Some(name) = state.container.and_then(|id| {
                config
                    .steps
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.as_str())
                    .or_else(|| config.tracks.iter().find(|t| t.id == id).map(|t| t.name.as_str()))
            }) {
                ui.label(RichText::new(aurora_i18n::tr!("ao-target", name = name.to_owned())).color(p.muted));
            }
            let drop = egui::Frame::new()
                .fill(p.field)
                .inner_margin(6.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("ao_animations")
                        .max_height(190.0)
                        .show(ui, |ui| {
                            for (index, step) in config.steps.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let label = if step.group {
                                        format!("{} ({})", step.name, step.members.len())
                                    } else {
                                        step.name.clone()
                                    };
                                    let playing = runtime.index(state.state, config.folder) == index;
                                    let response = ui.selectable_label(playing, label).on_hover_text(aurora_i18n::tr!("ao-play-tip"));
                                    if response.clicked() {
                                        state.container = step.group.then_some(step.id);
                                    }
                                    if response.double_clicked() {
                                        runtime.choose(state.state, config.folder, index);
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !state.busy()) {
                                            state.remove = Some(step.id);
                                        }
                                        if widgets::icon_button(
                                            ui,
                                            p,
                                            "caret-down-filled",
                                            aurora_i18n::tr!("ao-down"),
                                            !state.busy() && index + 1 < config.steps.len(),
                                        ) {
                                            push(&mut out, state, inventory::order(inv, &config, index, true));
                                        }
                                        if widgets::icon_button(
                                            ui,
                                            p,
                                            "caret-up-filled",
                                            aurora_i18n::tr!("ao-up"),
                                            !state.busy() && index > 0,
                                        ) {
                                            push(&mut out, state, inventory::order(inv, &config, index, false));
                                        }
                                    });
                                });
                                if step.group {
                                    ui.indent(step.id, |ui| {
                                        member_rows(ui, p, inv, &step.members, state, &mut out);
                                    });
                                }
                            }
                            for track in &config.tracks {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(
                                            state.container == Some(track.id),
                                            format!("{} · {}", aurora_i18n::tr!("ao-track"), track.name),
                                        )
                                        .clicked()
                                    {
                                        state.container = Some(track.id);
                                        state.cycle_edit = None;
                                    }
                                    if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !state.busy()) {
                                        state.remove = Some(track.id);
                                    }
                                });
                                ui.indent(track.id, |ui| {
                                    for (index, a) in track.members.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            if ui
                                                .selectable_label(runtime.index(state.state, track.id) == index, &a.name)
                                                .double_clicked()
                                            {
                                                runtime.choose(state.state, track.id, index);
                                            }
                                            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !state.busy())
                                            {
                                                push(&mut out, state, inventory::remove(inv, a.link));
                                            }
                                            if widgets::icon_button(
                                                ui,
                                                p,
                                                "caret-up-filled",
                                                aurora_i18n::tr!("ao-up"),
                                                !state.busy() && index > 0,
                                            ) {
                                                push(&mut out, state, inventory::member_order(inv, &track.members, index, false));
                                            }
                                            if widgets::icon_button(
                                                ui,
                                                p,
                                                "caret-down-filled",
                                                aurora_i18n::tr!("ao-down"),
                                                !state.busy() && index + 1 < track.members.len(),
                                            ) {
                                                push(&mut out, state, inventory::member_order(inv, &track.members, index, true));
                                            }
                                        });
                                    }
                                });
                            }
                            if config.steps.is_empty() && config.tracks.is_empty() {
                                ui.label(RichText::new(aurora_i18n::tr!("ao-drop")).color(p.muted));
                            }
                        });
                })
                .response;
            if let Some(payload) = drop.dnd_release_payload::<super::inventory::InvDrag>()
                && !state.busy()
            {
                push(&mut out, state, inventory::add(inv, set, state.state, payload.0, state.container));
            }
            ui.add_enabled_ui(!state.busy(), |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.container_name)
                            .desired_width(150.0)
                            .hint_text(aurora_i18n::tr!("ao-container-name")),
                    );
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-group")).clicked() {
                        push(
                            &mut out,
                            state,
                            inventory::new_container(inv, set, state.state, &state.container_name.clone(), false),
                        );
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-track")).clicked() {
                        push(
                            &mut out,
                            state,
                            inventory::new_container(inv, set, state.state, &state.container_name.clone(), true),
                        );
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-state-root")).clicked() {
                        state.container = None;
                        state.cycle_edit = None;
                    }
                });
                let track = state.container.and_then(|id| config.tracks.iter().find(|t| t.id == id));
                let (id, cycle) = track.map_or((config.folder, &config.cycle), |t| (t.id, &t.cycle));
                if !id.is_nil() {
                    if state.cycle_edit.as_ref().is_none_or(|(old, _)| *old != id) {
                        state.cycle_edit = Some((id, cycle.clone()));
                    }
                    let edit = &mut state.cycle_edit.as_mut().expect("cycle editor initialized").1;
                    let apply = ui
                        .horizontal(|ui| {
                            toggle(ui, p, "ao-cycle", &mut edit.cycle);
                            toggle(ui, p, "ao-random", &mut edit.random);
                            ui.add(egui::DragValue::new(&mut edit.seconds).range(0.0..=86400.0).suffix(" s").speed(1.0));
                            widgets::flat_button(ui, p, aurora_i18n::tr!("common-apply")).clicked()
                        })
                        .inner;
                    if apply {
                        let edit = edit.clone();
                        let result = if let Some(track) = track {
                            crate::world::inventory::actions::patch(
                                inv,
                                track.id,
                                aurora_llsd::llsd_map! { "name" => format!("{}:TR{}",track.name,edit.suffix()) },
                            )
                        } else {
                            let mut config = config.clone();
                            config.cycle = edit;
                            inventory::save_cycle(inv, state.state, &config)
                        };
                        push(&mut out, state, result);
                    }
                }
            });
        });
    if *open {
        dialogs(ctx, p, inv, catalog, state, &mut out);
    }
    out
}

fn member_rows(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, members: &[crate::ao::Animation], state: &mut AoUi, out: &mut Vec<Action>) {
    for (index, a) in members.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(&a.name);
            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-remove"), !state.busy()) {
                push(out, state, inventory::remove(inv, a.link));
            }
            if widgets::icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ao-up"), !state.busy() && index > 0) {
                push(out, state, inventory::member_order(inv, members, index, false));
            }
            if widgets::icon_button(
                ui,
                p,
                "caret-down-filled",
                aurora_i18n::tr!("ao-down"),
                !state.busy() && index + 1 < members.len(),
            ) {
                push(out, state, inventory::member_order(inv, members, index, true));
            }
        });
    }
}

fn dialogs(ctx: &egui::Context, p: &Palette, inv: &mut Inventory, catalog: &inventory::Catalog, state: &mut AoUi, out: &mut Vec<Action>) {
    if state.busy() {
        return;
    }
    if let Some(mode) = state.new_dialog {
        let mut open = true;
        let mut done = false;
        Floater::new(
            "ao_new",
            aurora_i18n::tr!("ao-set-name"),
            egui::pos2(290.0, 200.0),
            Vec2::new(330.0, 120.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.text_edit_singleline(&mut state.name);
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-save")).clicked() {
                match inventory::valid_name(&state.name) {
                    Ok(name) => {
                        if matches!(mode, SetDialog::Rename) {
                            if let Some(mut set) = catalog.selected(state.selected_set).cloned() {
                                if catalog.sets.iter().any(|s| s.id != set.id && s.name == name) {
                                    state.message = aurora_i18n::tr!("ao-name-or-loading").into();
                                } else {
                                    set.name = name;
                                    push(out, state, inventory::save_set(inv, &set));
                                    done = true;
                                }
                            }
                        } else {
                            let mut set = if matches!(mode, SetDialog::Clone) {
                                catalog
                                    .selected(state.selected_set)
                                    .cloned()
                                    .unwrap_or_else(|| Set::new(name.clone()))
                            } else {
                                Set::new(name.clone())
                            };
                            set.name = name;
                            set.default = catalog.sets.is_empty();
                            out.push(Action::Create(set));
                            done = true;
                        }
                    }
                    Err(reason) => state.message = reason,
                }
            }
        });
        if done || !open {
            state.new_dialog = None;
        }
    }
    if let Some(id) = state.remove {
        let mut open = true;
        let mut done = false;
        Floater::new(
            "ao_remove",
            aurora_i18n::tr!("common-remove"),
            egui::pos2(290.0, 220.0),
            Vec2::new(330.0, 120.0),
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("ao-remove-confirm"));
            ui.horizontal(|ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-remove")).clicked() {
                    push(out, state, inventory::remove(inv, id));
                    done = true;
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    done = true;
                }
            });
        });
        if done || !open {
            state.remove = None;
        }
    }
    if let Some(notecard) = state.picker {
        let mut open = true;
        let mut picked = None;
        Floater::new(
            "ao_picker",
            aurora_i18n::tr!("ao-choose"),
            egui::pos2(310.0, 160.0),
            Vec2::new(390.0, 420.0),
        )
        .show(ctx, p, &mut open, |ui| {
            ui.label(aurora_i18n::tr!("ao-picker-help"));
            ui.text_edit_singleline(&mut state.filter);
            let key = (inv.generation, state.filter.clone(), notecard);
            if state.rows_key.as_ref() != Some(&key) {
                state.rows = inv
                    .items
                    .values()
                    .filter(|it| {
                        (if notecard { it.asset_type == 7 } else { it.asset_type == 20 })
                            && it.name.to_lowercase().contains(&state.filter.to_lowercase())
                            && !crate::world::inventory::actions::in_type(inv, it.id, 14)
                    })
                    .map(|it| (it.id, it.name.clone()))
                    .collect();
                state.rows.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
                state.rows_key = Some(key);
            }
            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for (id, name) in &state.rows {
                    if ui.selectable_label(false, name).clicked() {
                        picked = Some(*id);
                    }
                }
            });
            ui.separator();
            ui.label(aurora_i18n::tr!("ao-browse"));
            if state.folder.is_nil() {
                state.folder = inv.root;
            }
            inv.request(state.folder);
            if let Some(folder) = inv.folders.get(&state.folder).cloned() {
                ui.horizontal(|ui| {
                    if widgets::icon_button(ui, p, "caret-up-filled", aurora_i18n::tr!("ao-up"), state.folder != inv.root) {
                        state.folder = folder.info.parent;
                    }
                    ui.label(&folder.info.name);
                });
                egui::ScrollArea::vertical().id_salt("ao_folders").max_height(160.0).show(ui, |ui| {
                    for id in folder.children {
                        if let Some(f) = inv.folders.get(&id)
                            && !f.library
                        {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                widgets::glyph(ui, "folder-filled", p.muted, 14.0);
                                if ui.selectable_label(false, &f.info.name).clicked() {
                                    state.folder = id;
                                }
                            });
                        }
                    }
                    if folder.state != FetchState::Fetched {
                        ui.label(aurora_i18n::tr!("ao-loading"));
                    }
                });
            }
        });
        if let Some(id) = picked {
            if notecard {
                out.push(Action::Import(id));
            } else if let Some(set) = catalog.selected(state.selected_set) {
                push(out, state, inventory::add(inv, set, state.state, id, state.container));
            }
            state.picker = None;
        } else if !open {
            state.picker = None;
        }
    }
}
