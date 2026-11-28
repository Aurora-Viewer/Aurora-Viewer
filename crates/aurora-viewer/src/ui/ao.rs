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
    /// When the last edit was saved: "Set saved" shows over the preview a
    /// moment (`preview::saved_notice`).
    saved_at: Option<std::time::Instant>,
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
    /// The set open in the window, to edit; None: the one in use. Opening a
    /// set does not make it play: "Use this set" does (FloaterAO::onSelectSet
    /// and onClickActivate, indra/newview/ao.cpp).
    pub editing: Option<Uuid>,
    /// The set in use this frame, and a set to put in use (an entry of the
    /// open set played in the world, as FloaterAO::onAnimationListDoubleClick
    /// activates its set first).
    active_set: Uuid,
    use_set: Option<Uuid>,
    /// A list's order as just dropped, shown while it is saved, and the moves
    /// waiting for their turn to be saved (one edit at a time).
    held_order: Option<slot::HeldOrder>,
    queued_moves: Vec<slot::QueuedMove>,
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
    /// The slot unfolded on each group's page: none when the window first
    /// opens, then each as left while flipping through the groups; a click
    /// on a slot's title unfolds it, another folds it.
    open_slots: [Option<State>; category::Category::ALL.len()],
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
        self.open_slots[category::Category::of(self.state).index()] = Some(self.state);
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

    /// The set open in the window: the one opened, else the one in use.
    pub(super) fn edited<'a>(&self, catalog: &'a inventory::Catalog, account: &Account) -> Option<&'a Set> {
        self.editing
            .and_then(|id| catalog.sets.iter().find(|s| s.id == id))
            .or_else(|| catalog.selected(account.selected))
    }

    /// An edit was saved: said a moment over the preview, in place of a
    /// line above the page.
    pub fn note_saved(&mut self) {
        self.message.clear();
        self.saved_at = Some(std::time::Instant::now());
    }

    /// Opens a slot on its group's page, unfolded, its main list shown.
    fn select_state(&mut self, item: State) {
        self.point_at(item);
        self.open_slots[category::Category::of(item).index()] = Some(item);
    }

    /// Makes `item` the slot the window works on (and previews), what is
    /// unfolded left as it is.
    fn point_at(&mut self, item: State) {
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

/// The window's shades, as its approved board draws them: steps between the
/// theme's bar and its raised face, its muted ink and its violet, so that
/// any skin keeps the same relations.
#[derive(Clone, Copy)]
pub(super) struct Shades {
    /// The middle column, the summary and the take-turns card.
    pub card: egui::Color32,
    /// A slot that holds animations; a group row under the pointer.
    pub slot: egui::Color32,
    /// The open slot.
    pub open: egui::Color32,
    /// An entry's card and the order button.
    pub item: egui::Color32,
    pub item_hot: egui::Color32,
    /// A stack's card.
    pub stack: egui::Color32,
    /// The open slot's and the previewed entry's outline.
    pub edge: egui::Color32,
}

pub(super) fn shades(p: &Palette) -> Shades {
    let mix = |t: f32| p.bar.lerp_to_gamma(p.raised, t);
    Shades {
        card: mix(0.1),
        slot: mix(0.26),
        open: mix(0.4),
        item: mix(0.6),
        item_hot: mix(0.78),
        stack: p.bar.lerp_to_gamma(p.violet, 0.17),
        edge: p.bar.lerp_to_gamma(p.muted, 0.38),
    }
}

fn push(out: &mut Vec<Action>, state: &mut AoUi, result: Result<Mutation, String>) {
    match result {
        Ok(change) => out.push(Action::Edit(change, None)),
        Err(reason) => state.message = reason,
    }
}

/// Width of the left column, and of the preview on the right.
const SIDEBAR: f32 = 252.0;
const PREVIEW: f32 = 340.0;
/// The narrowest the preview gets as the window narrows: it never folds
/// away.
const PREVIEW_LEAST: f32 = 230.0;
/// The least the page keeps between the sidebar and the preview.
const PAGE_LEAST: f32 = 400.0;
/// How wide the grip on the preview's left edge is.
const HANDLE: f32 = 16.0;
/// A saved window narrower than this is the old one-column AO window's.
const OLD_WIDTH: f32 = 1000.0;
/// The window's first size, and the least it can be made.
const SIZE: Vec2 = Vec2::new(1240.0, 760.0);
/// (the sidebar, a middle column of 400 and the narrowest preview)
const MIN_SIZE: Vec2 = Vec2::new(SIDEBAR + PAGE_LEAST + PREVIEW_LEAST, 520.0);

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
    if size.x < OLD_WIDTH {
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
    movement_status: &str,
) -> Vec<Action> {
    let mut out = Vec::new();
    let selected = state.edited(catalog, account);
    state.active_set = catalog.selected(account.selected).map_or(Uuid::nil(), |s| s.id);
    movement.swim_ready = catalog.swimming_ready(account.selected);
    movement.swim_slots = catalog.selected(account.selected).map_or(0, Set::core_swimming_slots);
    let selected_id = selected.map_or(Uuid::nil(), |s| s.id);
    if state.selected_set != selected_id {
        state.selected_set = selected_id;
        state.held_order = None;
        state.queued_moves.clear();
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
    if *open {
        widen_once(ctx);
    }
    // the AO on or off, in the title bar (FloaterAO's "Enable AO")
    let mut on = account.enabled;
    let mut floater = Floater::new("ao", title, egui::pos2(160.0, 90.0), SIZE).title_switch(aurora_i18n::tr!("ao-switch"), &mut on);
    floater.min_size = MIN_SIZE;
    floater.flush_body().show(ctx, p, open, |ui| {
        let full = ui.available_rect_before_wrap();
        ui.allocate_rect(full, egui::Sense::hover());
        let preview_w = preview_width(ui, full);
        let side = egui::Rect::from_min_size(full.min, Vec2::new(SIDEBAR, full.height()));
        let right = egui::Rect::from_min_max(egui::pos2(full.right() - preview_w, full.top()), full.max);
        let middle = egui::Rect::from_min_max(egui::pos2(side.right(), full.top()), egui::pos2(right.left(), full.bottom()));
        // the board's columns: the darker bar, then the card shade
        let shade = shades(p);
        ui.painter().rect_filled(side, egui::CornerRadius::ZERO, p.bar);
        ui.painter().rect_filled(middle, egui::CornerRadius::ZERO, shade.card);
        column(ui, "ao_sidebar", side, egui::Margin::symmetric(10, 12), |ui| {
            sidebar::show(ui, p, inv, catalog, account, state, &mut out);
        });
        column(ui, "ao_page", middle, egui::Margin::symmetric(16, 12), |ui| {
            notices(ui, p, inv, catalog, state, &mut out);
            if state.tab == 1 {
                page::movement(ui, p, movement, movement_prefs, movement_status, &mut out);
            } else if let Some(set) = selected {
                let swim_ready = movement.swim_ready;
                page::show(ui, p, inv, set, runtime, state, movement_prefs, swim_ready, &mut out);
            } else {
                first_run::show(ui, p, inv, catalog, state, &mut out);
            }
        });
        {
            column(ui, "ao_preview", right, egui::Margin::ZERO, |ui| {
                preview::show(ui, p, selected, runtime, state);
            });
        }
        preview_edge(ui, p, full, preview_w);
    });
    if let Some(id) = state.use_set.take()
        && account.selected != id
    {
        account.selected = id;
        out.push(Action::Preferences);
    }
    if on != account.enabled {
        account.enabled = on;
        if on {
            runtime.stands = true;
            runtime.paused = false;
        }
        out.push(Action::Preferences);
    }
    if *open {
        dialogs(ctx, p, inv, catalog, state, &mut out);
        migration::show(ctx, p, catalog, state, &mut out);
        combinations::show(ctx, p, inv, catalog, state, &mut out);
    }
    out
}

const PREVIEW_WIDTH: &str = "ao_preview_width";

/// The preview column's width: the one dragged at its left edge (kept with
/// the window's layout), narrowed with the window but never folded away,
/// the page keeping its least.
fn preview_width(ui: &egui::Ui, full: egui::Rect) -> f32 {
    let most = (full.width() - SIDEBAR - PAGE_LEAST).max(PREVIEW_LEAST);
    let saved = ui.data_mut(|d| d.get_persisted::<f32>(egui::Id::unique(PREVIEW_WIDTH)));
    saved.unwrap_or(PREVIEW).clamp(PREVIEW_LEAST, most)
}

/// The grip on the preview's left edge, over the columns: a drag widens or
/// narrows the preview (from the next frame), a double-click goes back to
/// the default width.
fn preview_edge(ui: &mut egui::Ui, p: &Palette, full: egui::Rect, width: f32) {
    let id = egui::Id::unique(PREVIEW_WIDTH);
    let most = (full.width() - SIDEBAR - PAGE_LEAST).max(PREVIEW_LEAST);
    let x = full.right() - width;
    let edge = egui::Rect::from_center_size(egui::pos2(x, full.center().y), Vec2::new(HANDLE, full.height()));
    let response = ui
        .interact(edge, id, egui::Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.double_clicked() {
        ui.data_mut(|d| d.remove::<f32>(id));
    } else if response.dragged() && response.drag_delta().x != 0.0 {
        // (dragged left, the preview widens)
        let width = (width - response.drag_delta().x).clamp(PREVIEW_LEAST, most);
        ui.data_mut(|d| d.insert_persisted(id, width));
        ui.ctx().request_repaint();
    }
    let lit = ui.ctx().animate_bool(id.with("lit"), response.hovered() || response.dragged());
    if lit > 0.0 {
        let pointer = response.hover_pos().or(response.interact_pointer_pos()).map(|at| at.y);
        let held = ui.data_mut(|d| {
            let held = d.get_temp_mut_or(id.with("y"), full.center().y);
            if let Some(y) = pointer {
                *held = y;
            }
            *held
        });
        glow_line(ui, x, full.y_range(), held, p.violet_light.gamma_multiply(lit));
    }
}

/// A vertical line brightest at `at` and fainter away from it, never quite
/// gone: the grip under the pointer.
fn glow_line(ui: &egui::Ui, x: f32, span: egui::Rangef, at: f32, color: egui::Color32) {
    const REACH: f32 = 140.0;
    const FAINT: f32 = 0.12;
    let mut mesh = egui::Mesh::default();
    let steps = ((span.span() / 6.0).ceil() as u32).max(1);
    for i in 0..=steps {
        let y = span.min + span.span() * i as f32 / steps as f32;
        let near = (-((y - at) / REACH).powi(2)).exp();
        let c = color.gamma_multiply(FAINT + (1.0 - FAINT) * near);
        mesh.colored_vertex(egui::pos2(x - 1.0, y), c);
        mesh.colored_vertex(egui::pos2(x + 1.0, y), c);
        if i > 0 {
            let b = 2 * i;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b + 1, b);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
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
    // (saving is said over the preview, where "Set saved" follows it)
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
