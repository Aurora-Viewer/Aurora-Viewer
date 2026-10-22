//! AO management floater: Firestorm AO::updateList / onDrop / onClick*.

use super::widgets::{self, Floater};
use crate::ao::{Account, Cycle, Set, State, inventory, runtime::Runtime};
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use aurora_net::inventory::operations::Mutation;
use egui::{RichText, Vec2};
use uuid::Uuid;

mod category;
mod combinations;
mod editor;
mod migration;
pub mod prototype;
pub mod quick;
#[cfg(test)]
mod tests;

pub enum Action {
    Edit(Mutation, Option<Uuid>),
    EditContainer(Mutation, EditorTarget),
    Create(Set),
    Import(Uuid),
    Migrate,
    ReloadMigration,
    Reload,
    Preferences,
    MovementPreferences,
}

#[derive(Clone, Copy)]
pub struct EditorTarget {
    pub set: Uuid,
    pub state: State,
    pub container: Option<Uuid>,
}

pub struct Pending {
    pub request: Uuid,
    pub refresh: Vec<Uuid>,
    pub select: Option<Uuid>,
    pub editor: Option<EditorTarget>,
}

#[derive(Default)]
pub struct AoUi {
    pub tab: usize,
    /// Only populated by the explicit offline AO design scenario.
    pub prototype: Option<prototype::Prototype>,
    pub pending: Option<Pending>,
    pub import: Option<Uuid>,
    pub import_folder: Option<Uuid>,
    pub migration: Option<crate::ao::migration::Migration>,
    pub message: String,
    pub warnings: Vec<String>,
    pub startup_loading: bool,
    pub state: State,
    pub container: Option<Uuid>,
    name: String,
    combination: Option<combinations::Dialog>,
    new_dialog: Option<SetDialog>,
    remove: Option<Uuid>,
    picker: Option<bool>,
    folder: Uuid,
    filter: String,
    rows: Vec<(Uuid, String)>,
    rows_key: Option<(u64, String, bool, super::inventory::view::Sort)>,
    cycle_edit: Option<(Uuid, Cycle)>,
    selected_set: Uuid,
    state_focus: Option<State>,
    rail_state: Option<State>,
    category_states: [Option<State>; category::Category::ALL.len()],
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

    pub fn finish_editor(&mut self, target: EditorTarget, created: &std::collections::HashMap<Uuid, Uuid>) -> bool {
        if self.selected_set != target.set || self.state != target.state {
            return false;
        }
        self.container = target.container.map(|id| created.get(&id).copied().unwrap_or(id));
        true
    }

    pub fn interface_demo(&mut self, mode: &str, catalog: &inventory::Catalog) {
        let Some(set) = catalog.selected(self.selected_set) else { return };
        self.state = match mode {
            "interface-running" => State::Running,
            "interface-jumping" => State::Jumping,
            "interface-swim" | "interface-swim-missing" => State::SwimmingForward,
            "interface-flight" => State::Flying,
            "interface-sitting" => State::Sitting,
            _ => State::Standing,
        };
        self.container = set.states.get(&self.state).and_then(|c| {
            if mode == "interface-group" {
                c.steps.iter().find(|s| s.group).map(|s| s.id)
            } else if mode == "interface-track" {
                c.tracks.first().map(|t| t.id)
            } else {
                None
            }
        });
        if mode == "interface-combine" {
            let mut dialog = combinations::Dialog::new(set.id, self.state);
            dialog.name = "Body and hands".into();
            dialog.selected = set
                .states
                .get(&self.state)
                .into_iter()
                .flat_map(|c| c.steps.iter().take(2).map(|s| s.id))
                .collect();
            self.combination = Some(dialog);
        }
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

fn enabled(ui: &mut egui::Ui, p: &Palette, runtime: &mut Runtime, account: &mut Account) -> bool {
    let changed = toggle(ui, p, "ao-enabled", &mut account.enabled);
    if changed && account.enabled {
        runtime.stands = true;
        runtime.paused = false;
    }
    changed
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
    movement: &mut super::movement::MovementUi,
    movement_prefs: &mut crate::movement::Preferences,
    keys: &crate::keybinds::KeyBindings,
    movement_status: &str,
) -> Vec<Action> {
    let mut out = Vec::new();
    let selected = catalog.selected(account.selected);
    movement.swim_ready = catalog.swimming_ready(account.selected);
    movement.swim_slots = selected.map_or(0, Set::core_swimming_slots);
    let selected_id = selected.map_or(Uuid::nil(), |s| s.id);
    if state.selected_set != selected_id {
        state.selected_set = selected_id;
        state.picker = None;
        state.container = None;
        state.cycle_edit = None;
        state.combination = None;
    }
    let title = aurora_i18n::tr!("ao-title");
    prototype::workbench(ctx, p, state, open);
    Floater::new("ao", title, egui::pos2(220.0, 120.0), Vec2::new(560.0, 760.0))
        .help(aurora_i18n::tr!("ao-help"))
        .show(ctx, p, open, |ui| {
            if prototype::before(ui, p, state) {
                return;
            }
            if state.prototype.is_none() {
                widgets::tabs(
                    ui,
                    p,
                    &mut state.tab,
                    &[
                        (aurora_i18n::tr!("ao-tab-animations"), true),
                        (aurora_i18n::tr!("ao-tab-movement"), true),
                    ],
                );
                if state.tab == 1 {
                    let actions = egui::ScrollArea::vertical()
                        .id_salt("ao_movement")
                        .auto_shrink([false, false])
                        .show(ui, |ui| movement.show(ui, p, movement_prefs, keys, movement_status))
                        .inner;
                    if actions.changed {
                        out.push(Action::MovementPreferences);
                    }
                    return;
                }
            }
            ui.horizontal(|ui| {
                if enabled(ui, p, runtime, account) {
                    out.push(Action::Preferences);
                }
                ui.add_enabled_ui(account.enabled, |ui| {
                    toggle(ui, p, "ao-pause", &mut runtime.paused);
                });
                toggle(ui, p, "ao-stands", &mut runtime.stands);
            });
            if let Some(set) = selected {
                set_options(ui, p, inv, set, state, &mut out);
            }
            widgets::divider(ui, p);
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
                ui.horizontal_wrapped(|ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-new")).clicked() {
                        state.name.clear();
                        state.new_dialog = Some(SetDialog::New);
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-import")).clicked() {
                        state.picker = Some(true);
                        state.folder = inv.root;
                    }
                    if ui
                        .add_enabled(!state.startup_loading, egui::Button::new(aurora_i18n::tr!("ao-migrate")))
                        .clicked()
                    {
                        state.migration = Some(Default::default());
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
            } else if catalog.loading || state.startup_loading {
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
            widgets::divider(ui, p);
            ui.add_enabled_ui(!state.busy(), |ui| {
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
            widgets::divider(ui, p);
            editor::show(ui, p, inv, set, runtime, state, &mut out);
        });
    if *open {
        dialogs(ctx, p, inv, catalog, state, &mut out);
        migration::show(ctx, p, catalog, state, &mut out);
        combinations::show(ctx, p, inv, catalog, state, &mut out);
    }
    out
}

fn set_options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, set: &Set, state: &mut AoUi, out: &mut Vec<Action>) {
    ui.add_enabled_ui(!state.busy(), |ui| {
        let (mut sits, mut smart, mut mouselook) = (set.sits, set.smart, set.mouselook);
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= toggle(ui, p, "ao-sits", &mut sits);
            changed |= toggle(ui, p, "ao-smart", &mut smart);
        });
        changed |= toggle(ui, p, "ao-mouselook", &mut mouselook);
        if changed {
            let mut edit = set.clone();
            (edit.sits, edit.smart, edit.mouselook) = (sits, smart, mouselook);
            push(out, state, inventory::save_set(inv, &edit));
        }
    });
}

fn swimming_status(ui: &mut egui::Ui, p: &Palette, set: &Set, slot: State) {
    if set.swimming_ready() {
        ui.label(RichText::new(aurora_i18n::tr!("ao-swim-ready")).color(p.muted));
    } else {
        swimming_warning(ui, p);
    }
    if !State::CORE_SWIMMING.contains(&slot) {
        ui.label(RichText::new(aurora_i18n::tr!("ao-swim-optional")).color(p.muted));
    }
    if slot == State::UnderwaterSwimming && set.swimming_animation(slot).is_none() {
        ui.label(RichText::new(aurora_i18n::tr!("ao-underwater-swimming-fallback")).color(p.muted));
    }
}

pub(super) fn swimming_warning(ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal_top(|ui| {
        widgets::glyph(ui, "alert-triangle-filled", p.warn, 14.0);
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("ao-swim-required")).color(p.warn)).wrap());
    });
}

fn swimming_marker(ui: &mut egui::Ui, p: &Palette, set: &Set, slot: State) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(16.0, 28.0), egui::Sense::hover());
    if State::CORE_SWIMMING.contains(&slot) {
        let (icon, color, tip) = if set.swimming_animation(slot).is_some() {
            ("circle-check-filled", p.success, aurora_i18n::tr!("ao-swim-required-present"))
        } else {
            ("alert-triangle-filled", p.warn, aurora_i18n::tr!("ao-swim-required-missing"))
        };
        widgets::paint_glyph(ui.painter(), icon, rect.center(), 14.0, color);
        response.on_hover_text(tip);
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
        let order = super::inventory::view::picker_order(ctx, inv);
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
            let key = (
                inv.generation,
                state.filter.clone(),
                notecard,
                super::inventory::view::picker_sort(ctx),
            );
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
                state.rows.sort_by_key(|row| order.rank(row.0));
                state.rows_key = Some(key);
            }
            egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                for (id, name) in &state.rows {
                    if ui.selectable_label(false, name).clicked() {
                        picked = Some(*id);
                    }
                }
            });
            widgets::divider(ui, p);
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
                    for &id in order.children(state.folder) {
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
