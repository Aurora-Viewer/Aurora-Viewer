//! AO window: one page per group of slots, after FloaterAO (indra/newview/ao.cpp,
//! floater_ao.xml) and AOEngine (indra/newview/aoengine.cpp).
//!
//! Three columns: the set and its groups on the left (`sidebar`), the slots of
//! the chosen group in the middle (`page`, each slot opening its editor in
//! place, `slot`), and the preview on the right (`preview`). An account
//! without a set sees the first-run choices instead (`first_run`).
//!
//! Every edit goes through the same inventory plans as before
//! (`crate::ao::inventory`): only the window changed. Differences with the
//! Legacy Viewer's floater, whose features are all kept:
//! - its state list is one combo box; here every slot of a group is shown,
//!   with what it holds, and the five groups and Essentials narrow the list;
//! - its cycle checkbox, random checkbox and seconds field become one order
//!   menu (In order / Shuffle / Manual) and a time slider whose first stop,
//!   "Each time" (0 s), is AOEngine's cycle without a timer: the next one at
//!   each entry into the state.

use super::widgets::{self, Floater};
use crate::ao::{Account, Cycle, Set, State, inventory, runtime::Runtime};
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use aurora_net::inventory::operations::Mutation;
use egui::{RichText, Vec2};
use uuid::Uuid;

mod category;
mod combinations;
mod first_run;
mod migration;
pub mod mini;
mod page;
mod preview;
pub mod quick;
mod sidebar;
mod slot;
#[cfg(test)]
mod tests;

pub enum Action {
    Edit(Mutation, Option<Uuid>),
    EditContainer(Mutation, EditorTarget),
    Create(Set),
    Import(Uuid),
    Migrate,
    ReloadMigration,
    FinishRecovery(inventory::Recovery),
    /// Opens the inventory on that folder: a set, or an unfinished import.
    ShowInInventory(Uuid),
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
    /// 0: the groups' pages, 1: Movement.
    pub tab: usize,
    pub pending: Option<Pending>,
    pub import: Option<Uuid>,
    pub import_folder: Option<Uuid>,
    pub migration: Option<crate::ao::migration::Migration>,
    pub message: String,
    pub warnings: Vec<String>,
    pub startup_loading: bool,
    /// The slot open on its page.
    pub state: State,
    /// The layer (track) shown in that slot, or the group the picker adds to;
    /// `None` for the slot's main list.
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
    /// Only the six slots everyone notices (`page::ESSENTIALS`).
    essentials: bool,
    /// The "Your sets" panel replaces the groups in the left column.
    sets_open: bool,
    /// The set renamed in place in that panel.
    renaming: Option<Uuid>,
    /// The set whose deletion waits for its confirmation in that panel.
    confirm_delete: Option<Uuid>,
    /// Where the animation picker adds: a stack or a layer of the open
    /// slot, `None` for its main list.
    pick_into: Option<Uuid>,
    preview: preview::Preview,
    /// The preview's 3D stage: our outfit, posed by the preview's skeleton
    /// (app/ao_preview.rs), drawn as Inspect draws an avatar.
    pub stage: super::object_stage::StageUi,
    /// The slot each group had open, to open it again (`page::show`).
    category_states: [Option<State>; category::Category::ALL.len()],
    /// The slot header that takes the keyboard focus next frame.
    state_focus: Option<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetDialog {
    New,
    /// A copy of that set.
    Clone(Uuid),
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
        self.tab = 0;
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
                if let Some(index) = c.steps.iter().position(|s| s.group) {
                    self.preview.show_entry(index);
                }
                None
            } else if mode == "interface-track" {
                c.tracks.first().map(|t| t.id)
            } else {
                None
            }
        });
        self.sets_open = mode == "interface-sets";
        self.essentials = mode == "interface-essentials";
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

    /// Opens a slot on its group's page, its main list shown.
    fn select_state(&mut self, item: State) {
        if self.state != item {
            self.preview = Default::default();
        }
        self.tab = 0;
        self.state = item;
        self.container = None;
        self.cycle_edit = None;
        self.picker = None;
        self.combination = None;
        self.pick_into = None;
    }
}

fn push(out: &mut Vec<Action>, state: &mut AoUi, result: Result<Mutation, String>) {
    match result {
        Ok(change) => out.push(Action::Edit(change, None)),
        Err(reason) => state.message = reason,
    }
}

pub(super) fn toggle(ui: &mut egui::Ui, p: &Palette, key: &'static str, value: &mut bool) -> bool {
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

pub(super) fn enabled(ui: &mut egui::Ui, p: &Palette, runtime: &mut Runtime, account: &mut Account) -> bool {
    let changed = toggle(ui, p, "ao-enabled", &mut account.enabled);
    if changed && account.enabled {
        runtime.stands = true;
        runtime.paused = false;
    }
    changed
}

/// Width of the left column, and of the preview on the right.
const SIDEBAR: f32 = 252.0;
const PREVIEW: f32 = 340.0;
/// Below this width the preview column folds away.
const PREVIEW_ROOM: f32 = 1000.0;
/// The window's first size, and the least it can be made.
const SIZE: Vec2 = Vec2::new(1240.0, 760.0);
const MIN_SIZE: Vec2 = Vec2::new(760.0, 520.0);

/// A window saved at the size of the old one-column AO window (560 wide)
/// opens once at the size of the three columns; a size chosen after that
/// stays.
fn widen_once(ctx: &egui::Context) {
    let done = egui::Id::unique("ao_three_columns");
    if ctx.data_mut(|d| d.get_persisted::<bool>(done)).unwrap_or(false) {
        return;
    }
    // Until the window has been laid out once, its saved size is unknown.
    let Some(size) = widgets::floater_size(ctx, "ao") else { return };
    if size.x < PREVIEW_ROOM {
        widgets::size_floater(ctx, "ao", SIZE);
    }
    ctx.data_mut(|d| d.insert_persisted(done, true));
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
        state.pick_into = None;
        state.renaming = None;
        state.confirm_delete = None;
        state.preview = Default::default();
    }
    let title = aurora_i18n::tr!("ao-title");
    let (dot, status) = if !account.enabled {
        (p.muted, aurora_i18n::tr!("ao-status-off"))
    } else if runtime.paused {
        (p.amber, aurora_i18n::tr!("ao-status-paused"))
    } else {
        (p.success, aurora_i18n::tr!("ao-status-on"))
    };
    let (mut power, mut pause) = (false, false);
    let power_tip = aurora_i18n::text(if account.enabled { "ao-turn-off" } else { "ao-turn-on" });
    let (pause_icon, pause_tip) = if runtime.paused {
        ("player-play-filled", aurora_i18n::tr!("ao-resume"))
    } else {
        ("player-pause-filled", aurora_i18n::tr!("ao-pause"))
    };
    if *open {
        widen_once(ctx);
    }
    let mut floater = Floater::new("ao", title, egui::pos2(160.0, 90.0), SIZE)
        .help(aurora_i18n::tr!("ao-help-pages"))
        .title_state(dot, status)
        .action("power", power_tip, &mut power);
    if account.enabled {
        floater = floater.action(pause_icon, pause_tip, &mut pause);
    }
    floater.min_size = MIN_SIZE;
    floater.flush_body().show(ctx, p, open, |ui| {
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, egui::Sense::hover());
        let preview_w = if full.width() >= PREVIEW_ROOM { PREVIEW } else { 0.0 };
        let side = egui::Rect::from_min_size(full.min, Vec2::new(SIDEBAR, full.height()));
        let right = egui::Rect::from_min_max(egui::pos2(full.right() - preview_w, full.top()), full.max);
        let middle = egui::Rect::from_min_max(egui::pos2(side.right(), full.top()), egui::pos2(right.left(), full.bottom()));
        ui.painter().rect_filled(side, egui::CornerRadius::ZERO, p.bar);
        column(ui, "ao_sidebar", side, egui::Margin::symmetric(10, 12), |ui| {
            sidebar::show(ui, p, inv, catalog, runtime, account, state, &mut out);
        });
        column(ui, "ao_page", middle, egui::Margin::symmetric(16, 12), |ui| {
            notices(ui, p, inv, catalog, state, &mut out);
            if state.tab == 1 {
                page::movement(ui, p, movement, movement_prefs, keys, movement_status, &mut out);
            } else if let Some(set) = selected {
                let swim_ready = movement.swim_ready;
                page::show(ui, p, inv, set, runtime, state, movement_prefs, swim_ready, &mut out);
            } else {
                first_run::show(ui, p, inv, catalog, state, &mut out);
            }
        });
        if preview_w > 0.0 {
            ui.painter().rect_filled(right, egui::CornerRadius::ZERO, p.panel);
            column(ui, "ao_preview", right, egui::Margin::ZERO, |ui| {
                preview::show(ui, p, selected, runtime, state);
            });
        }
    });
    if power {
        account.enabled = !account.enabled;
        if account.enabled {
            runtime.stands = true;
            runtime.paused = false;
        }
        out.push(Action::Preferences);
    }
    if pause && account.enabled {
        runtime.paused = !runtime.paused;
    }
    if *open {
        dialogs(ctx, p, inv, catalog, state, &mut out);
        migration::show(ctx, p, catalog, state, &mut out);
        combinations::show(ctx, p, inv, catalog, state, &mut out);
    }
    out
}

/// One column of the window, laid out in its own rect and cut at its edges,
/// so that a long name never widens the window (#1600).
fn column(ui: &mut egui::Ui, salt: &str, rect: egui::Rect, margin: egui::Margin, body: impl FnOnce(&mut egui::Ui)) {
    let inner = rect - margin;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    body(&mut child);
}

/// Loading, saving, a refused edit, import warnings and unfinished imports,
/// above the page.
fn notices(ui: &mut egui::Ui, p: &Palette, inv: &mut Inventory, catalog: &inventory::Catalog, state: &mut AoUi, out: &mut Vec<Action>) {
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
    for recovery in inventory::recoveries(inv) {
        let name = inv
            .folders
            .get(&recovery.source)
            .map_or("Aurora AO".to_owned(), |f| f.info.name.clone());
        widgets::surface(ui, p, |ui| {
            let notice = if recovery.duplicate {
                aurora_i18n::tr!("ao-duplicate-native", folder = name.as_str(), count = recovery.sets)
            } else {
                aurora_i18n::tr!("ao-unfinished-import", folder = name.as_str(), count = recovery.sets)
            };
            ui.label(RichText::new(notice).color(p.warn));
            ui.horizontal(|ui| {
                // Like the other AO buttons: an edit already running
                // would drop the click.
                ui.add_enabled_ui(!state.busy() && !catalog.loading, |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-finish-import")).clicked() {
                        out.push(Action::FinishRecovery(recovery));
                    }
                });
                if widgets::flat_button(ui, p, aurora_i18n::tr!("ao-show-in-inventory")).clicked() {
                    out.push(Action::ShowInInventory(recovery.source));
                }
            });
        });
    }
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
            widgets::restricted_text_field(ui, true, &mut state.name, widgets::TextPolicy::AoName, |b, _| {
                egui::TextEdit::singleline(b)
            });
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-save")).clicked() {
                match inventory::valid_name(&state.name) {
                    Ok(name) => {
                        let mut set = match mode {
                            SetDialog::Clone(source) => catalog
                                .sets
                                .iter()
                                .find(|s| s.id == source)
                                .cloned()
                                .unwrap_or_else(|| Set::new(name.clone())),
                            SetDialog::New => Set::new(name.clone()),
                        };
                        set.name = name;
                        set.default = catalog.sets.is_empty();
                        out.push(Action::Create(set));
                        done = true;
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
            crate::ui::widgets::search_text_edit(ui, p, &mut state.filter, |edit| edit);
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
                push(out, state, inventory::add(inv, set, state.state, id, state.pick_into));
            }
            state.picker = None;
            state.pick_into = None;
        } else if !open {
            state.picker = None;
            state.pick_into = None;
        }
    }
}
