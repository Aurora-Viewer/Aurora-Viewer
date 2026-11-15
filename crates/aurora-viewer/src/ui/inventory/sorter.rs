//! The Auto-sort window: the rules of `world::inventory::sorter`, a preview of
//! where every entry goes, the user's own rules, and the sort itself, sent as
//! bounded inventory mutations one after another through the ordinary
//! pending edit (like Bulk permissions), so that nothing else races it.
//!
//! It opens three ways: the Auto-sort button of the inventory window (on the
//! folder shown or selected, Objects otherwise), a folder's right-click (its
//! contents) and an entry's right-click, which sorts the selected entries at
//! once. "Whole inventory" reads My Inventory and its system folders, and
//! asks twice before it moves anything. Pre-Trash rules collect entries in
//! a Pre-Trash folder; they reach the Trash only from its own confirmation,
//! through the ordinary delete path.

use super::*;
use crate::world::inventory::sorter::{self, Match, Move, Plan, Rule, Step};
use std::collections::VecDeque;

/// What the window sorts.
#[derive(Clone, PartialEq)]
enum Source {
    /// The direct contents of a folder.
    Folder(Uuid),
    /// Entries chosen in the inventory, sorted at once.
    Entries(Vec<Uuid>),
    /// My Inventory's loose items and its system folders
    /// (`sorter::whole_sources`), confirmed before it starts.
    Whole,
}

/// A confirmation on top of the window.
#[derive(Clone, PartialEq)]
enum Confirm {
    Whole,
    PreTrash(Vec<Uuid>),
}

/// The folder picker: for the folder to sort or for a rule's destination.
#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Source,
    Rule(usize),
    Exclude,
}

struct Picker {
    pick: Pick,
    filter: String,
    /// Every eligible folder and its path, built once per inventory change.
    rows: Vec<(Uuid, String)>,
    generation: u64,
}

#[derive(Default)]
pub(super) struct State {
    source: Option<Source>,
    open: bool,
    tab: usize,
    /// The preview, kept until the inventory or the options change.
    preview: Option<(u64, sorter::Preferences, Plan)>,
    job: Option<Job>,
    /// A batch is on its way; its reply comes through `finish_sorter`.
    submitted: bool,
    status: String,
    /// Start once the preview is ready: a right-click on entries, or
    /// `AURORA_DEMO_INVENTORY=sorter-run`.
    autostart: bool,
    /// The toolbar button: the folder is chosen where the inventory is known.
    requested: Option<Uuid>,
    picker: Option<Picker>,
    confirm: Option<Confirm>,
    /// The whole-inventory confirmation's "I understand" box.
    understood: bool,
}

struct Job {
    queue: VecDeque<Move>,
    base: String,
    total: usize,
    done: usize,
    sending: usize,
}

const PREVIEW: usize = 0;
const RULES: usize = 1;
const OPTIONS: usize = 2;

impl State {
    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted || self.requested.is_some()
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.submitted
    }

    fn set_source(&mut self, source: Source) {
        self.open = true;
        if self.busy() {
            return;
        }
        if self.source.as_ref() != Some(&source) {
            self.preview = None;
            self.status.clear();
        }
        self.source = Some(source);
    }
}

impl InventoryUi {
    /// A folder's right-click: sort its contents.
    pub(crate) fn open_sorter(&mut self, source: Uuid) {
        self.sorter.set_source(Source::Folder(source));
        self.sorter.tab = PREVIEW;
    }

    /// An entry's right-click: sort the selected entries at once.
    pub(crate) fn sort_entries(&mut self, ids: Vec<Uuid>) {
        if self.sorter.busy() {
            self.sorter.open = true;
            return;
        }
        self.sorter.set_source(Source::Entries(ids));
        self.sorter.tab = PREVIEW;
        self.sorter.autostart = true;
    }

    /// The toolbar button: the window opens on `shown` (the folder the
    /// inventory shows), the selected folder or Objects.
    pub(crate) fn request_sorter(&mut self, shown: Uuid) {
        if self.sorter.open {
            self.sorter.open = false;
        } else {
            self.sorter.requested = Some(shown);
        }
    }

    /// The demo's unattended run: Auto-sort opens and starts by itself.
    pub(crate) fn demo_run_sorter(&mut self, source: Uuid) {
        self.open_sorter(source);
        self.sorter.autostart = true;
    }

    pub(super) fn sorter_open(&self) -> bool {
        self.sorter.open
    }

    /// An Auto-sort batch is on its way (set in the frame it is pushed,
    /// before the app submits it): its new folders are neither shown nor
    /// renamed like a "New folder".
    pub(crate) fn sorter_sending(&self) -> bool {
        self.sorter.submitted || self.windows.iter().any(|w| w.state.sorter.submitted)
    }

    /// The ordinary inventory result completes the sorter's batch too.
    pub(crate) fn finish_sorter(&mut self, error: Option<&str>) {
        let state = &mut self.sorter;
        if state.submitted {
            state.submitted = false;
            state.preview = None;
            match (error, state.job.as_mut()) {
                (Some(error), _) => {
                    // the confirmed batches stay sorted; the rest stays put
                    state.job = None;
                    state.status = error.to_owned();
                }
                (None, Some(job)) => job.done += std::mem::take(&mut job.sending),
                (None, None) => {}
            }
        }
        for window in &mut self.windows {
            window.state.finish_sorter(error);
        }
    }
}

/// The folder the toolbar button sorts: the selected folder, else the folder
/// shown, else Objects, else My Inventory.
fn default_source(inv: &Inventory, st: &InventoryUi, shown: Uuid) -> Uuid {
    st.selected
        .filter(|id| inv.folders.contains_key(id))
        .into_iter()
        .chain([shown])
        .find(|id| *id != inv.root && sorter::source_allowed(inv, *id))
        .or_else(|| rules::system(inv, 6))
        .unwrap_or(inv.root)
}

/// Send the next batch when the inventory is free, then draw the window.
pub(super) fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    worn: &HashSet<Uuid>,
    actions: &mut Vec<InvAction>,
) {
    if let Some(shown) = st.sorter.requested.take() {
        let source = default_source(inv, st, shown);
        st.sorter.set_source(Source::Folder(source));
    }
    let free = st.pending.is_none() && !actions.iter().any(|action| matches!(action, InvAction::Edit(_)));
    let state = &mut st.sorter;
    if free
        && !state.submitted
        && let Some(job) = state.job.as_mut()
    {
        match sorter::step(inv, &job.base, &mut job.queue, &prefs.protected, worn) {
            Step::Change(change, taken) => {
                job.sending = taken;
                state.submitted = true;
                actions.push(InvAction::Edit(*change));
            }
            Step::Wait => {}
            Step::Done => {
                state.status = aurora_i18n::tr!("inventory-sorter-done", count = job.done);
                state.job = None;
                state.preview = None;
            }
        }
    }
    if !state.open {
        return;
    }
    let Some(source) = state.source.clone() else {
        state.open = false;
        return;
    };
    let busy = state.busy();
    let loaded = match &source {
        Source::Folder(id) => {
            // Loading the folder is all a preview needs: its direct contents.
            let loaded = inv.folders.get(id).is_some_and(|f| f.state == FetchState::Fetched);
            if !loaded && inv.folders.contains_key(id) {
                inv.request(*id);
            }
            loaded
        }
        Source::Entries(_) => true,
        Source::Whole => {
            let mut loaded = true;
            for id in sorter::whole_sources(inv, &prefs.sorter) {
                if inv.folders.get(&id).is_some_and(|f| f.state != FetchState::Fetched) {
                    inv.request(id);
                    loaded = false;
                }
            }
            loaded
        }
    };
    if loaded
        && !busy
        && state
            .preview
            .as_ref()
            .is_none_or(|(generation, options, _)| *generation != inv.generation || *options != prefs.sorter)
    {
        let plan = match &source {
            Source::Folder(id) => sorter::plan(inv, *id, &prefs.sorter, &prefs.protected, worn),
            Source::Entries(ids) => sorter::plan_entries(inv, ids, &prefs.sorter, &prefs.protected, worn),
            Source::Whole => sorter::plan_whole(inv, &prefs.sorter, &prefs.protected, worn),
        };
        // A Pre-Trash rule reads the whole folder sorted at any depth (and
        // the sort folder's tree): what is not loaded yet is asked for, and
        // the preview fills in as it arrives (it follows the inventory's
        // generation). Nothing waits for it.
        for id in &plan.unfetched {
            if inv.folders.get(id).is_some_and(|f| f.state == FetchState::Unknown) {
                inv.request(*id);
            }
        }
        state.preview = Some((inv.generation, prefs.sorter.clone(), plan));
    }
    if state.autostart && state.preview.is_some() {
        state.autostart = false;
        start(state, prefs);
    }
    let mut open = true;
    let mut go = false;
    let mut stop = false;
    let waiting = sorter::pre_trash_entries(inv);
    // The tabs change the body's height all the time (rule cards, preview
    // groups opening): egui's frame followed it and kept the tallest it saw,
    // so the window grew down and its bottom edge snapped back when dragged.
    // `steady_size` lays the body out in the window's own rect, like the
    // World Map's side panel (#2815); the minimum keeps the tabs, a few rows
    // and the footer whole.
    let window_id = format!("inventory_sorter_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-sorter-title"),
        ctx.content_rect().center() - Vec2::new(250.0, 290.0),
        Vec2::new(500.0, 580.0),
    )
    .steady_size();
    window.min_size = Vec2::new(420.0, 360.0);
    window.show(ctx, p, &mut open, |ui| {
        let state = &mut st.sorter;
        source_row(ui, p, inv, state, &source, busy);
        ui.add_space(6.0);
        let rule_count = prefs.sorter.rules.len();
        let rules_label = if rule_count == 0 {
            aurora_i18n::tr!("inventory-sorter-rules").to_owned()
        } else {
            format!("{}  ({rule_count})", aurora_i18n::tr!("inventory-sorter-rules"))
        };
        crate::ui::widgets::tabs(
            ui,
            p,
            &mut state.tab,
            &[
                (aurora_i18n::tr!("inventory-sorter-preview"), true),
                (&rules_label, true),
                (aurora_i18n::tr!("inventory-sorter-options"), true),
            ],
        );
        ui.add_space(6.0);
        let footer = 46.0;
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_body")
            .max_height((ui.available_height() - footer).max(80.0))
            .auto_shrink([false, false])
            .show(ui, |ui| match state.tab {
                RULES => {
                    ui.add_enabled_ui(!busy, |ui| rules_editor(ui, p, inv, state, &mut prefs.sorter))
                        .inner
                }
                OPTIONS => ui.add_enabled_ui(!busy, |ui| options(ui, p, inv, state, &mut prefs.sorter)).inner,
                _ => {
                    if !waiting.is_empty() {
                        pre_trash_card(ui, p, state, &waiting, busy);
                        ui.add_space(6.0);
                    }
                    match state.preview.as_ref() {
                        _ if !loaded => {
                            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-loading")).color(p.muted));
                        }
                        Some((_, _, plan)) => preview(ui, p, plan),
                        None => {}
                    }
                }
            });
        crate::ui::widgets::divider(ui, p);
        footer_row(ui, p, state, st.pending.is_none(), &mut go, &mut stop);
    });
    let state = &mut st.sorter;
    if go && state.job.is_none() {
        if source == Source::Whole {
            // the second step: nothing moves before the confirmation
            state.understood = false;
            state.confirm = Some(Confirm::Whole);
        } else {
            start(state, prefs);
        }
    }
    if stop && let Some(job) = state.job.as_mut() {
        // the batch on its way finishes; nothing after it is sent
        job.queue.clear();
    }
    state.open = open;
    if !open {
        state.picker = None;
        state.confirm = None;
    }
    picker(ctx, p, inv, st, prefs);
    confirm(ctx, p, inv, st, prefs, worn, actions);
}

/// What waits in Pre-Trash, and the step that sends it to the Trash.
fn pre_trash_card(ui: &mut egui::Ui, p: &Palette, state: &mut State, waiting: &[Uuid], busy: bool) {
    crate::ui::widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "trash-filled", p.warn, 16.0);
            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-waiting", count = waiting.len())).strong());
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-hint"))
                .small()
                .color(p.muted),
        );
        ui.add_space(4.0);
        let send = ui.add_enabled_ui(!busy, |ui| {
            crate::ui::widgets::action_button(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("inventory-sorter-pre-trash-empty"),
                crate::ui::widgets::ActionKind::Danger,
                Vec2::new(0.0, 26.0),
            )
        });
        if send.inner.clicked() {
            state.confirm = Some(Confirm::PreTrash(waiting.to_vec()));
        }
    });
}

/// The confirmations: a whole-inventory sort (with its "I understand" box)
/// and Pre-Trash to Trash. Escape or Cancel closes them, nothing moved.
fn confirm(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    worn: &HashSet<Uuid>,
    actions: &mut Vec<InvAction>,
) {
    let Some(asked) = st.sorter.confirm.clone() else { return };
    let moves = st.sorter.preview.as_ref().map_or(0, |(_, _, plan)| plan.moves.len());
    let mut answer = None;
    crate::ui::widgets::Modal::new(egui::Id::unique(("inventory_sorter_confirm", st.ui_id))).show(ctx, |ui| {
        use crate::ui::widgets::{ActionKind, action_button};
        ui.set_max_width(400.0);
        let size = Vec2::new(0.0, 26.0);
        match &asked {
            Confirm::Whole => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-whole-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("inventory-sorter-whole-text", count = moves));
                ui.add_space(8.0);
                crate::ui::widgets::checkbox(ui, p, &mut st.sorter.understood, aurora_i18n::tr!("inventory-sorter-whole-check"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = aurora_i18n::tr!("inventory-sorter-start", count = moves);
                    let go = ui.add_enabled_ui(st.sorter.understood, |ui| {
                        action_button(ui, p, "check", &label, ActionKind::Primary, size)
                    });
                    if go.inner.clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
            Confirm::PreTrash(ids) => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-sorter-pre-trash-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("inventory-sorter-pre-trash-text", count = ids.len()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let free = st.pending.is_none() && st.pending_delete.is_none();
                    let go = ui.add_enabled_ui(free, |ui| {
                        action_button(
                            ui,
                            p,
                            "trash-filled",
                            aurora_i18n::tr!("inventory-sorter-pre-trash-empty"),
                            ActionKind::Danger,
                            size,
                        )
                    });
                    if go.inner.clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(false);
        }
    });
    let Some(answer) = answer else { return };
    st.sorter.confirm = None;
    if !answer {
        return;
    }
    match asked {
        Confirm::Whole => start(&mut st.sorter, prefs),
        Confirm::PreTrash(ids) => {
            // the ordinary delete: the Trash's own rules decide what goes
            let change = rules::system(inv, 14)
                .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
                .and_then(|trash| rules::move_selection(inv, &ids, trash, &prefs.protected, worn));
            match change {
                Ok(change) => actions.push(InvAction::Delete(Box::new(DeleteRequest::new(inv, st, &ids, change)))),
                Err(reason) => st.sorter.status = reason,
            }
        }
    }
}

fn start(state: &mut State, prefs: &InventoryPreferences) {
    let Some((_, _, plan)) = state.preview.as_ref() else { return };
    state.status.clear();
    if plan.moves.is_empty() {
        state.status = aurora_i18n::tr!("inventory-sorter-nothing").into();
        return;
    }
    state.job = Some(Job {
        queue: plan.moves.iter().cloned().collect(),
        base: prefs.sorter.base_name(),
        total: plan.moves.len(),
        done: 0,
        sending: 0,
    });
}

/// What is sorted: a folder (with Change…) or the selected entries.
fn source_row(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, source: &Source, busy: bool) {
    crate::ui::widgets::surface(ui, p, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "folder-filled", p.violet_light, 16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-source")).small().color(p.muted));
                let text = match source {
                    Source::Folder(id) => sorter::folder_path(inv, *id),
                    Source::Entries(ids) => aurora_i18n::tr!("inventory-sorter-selection", count = ids.len()),
                    Source::Whole => aurora_i18n::tr!("inventory-sorter-whole").to_owned(),
                };
                ui.label(RichText::new(text).strong());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-change")).clicked() {
                        state.picker = Some(Picker::new(Pick::Source));
                    }
                    if *source != Source::Whole
                        && crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-whole"))
                            .on_hover_text(aurora_i18n::tr!("inventory-sorter-whole-tip"))
                            .clicked()
                    {
                        state.set_source(Source::Whole);
                    }
                });
            });
        });
        let intro = if *source == Source::Whole {
            aurora_i18n::tr!("inventory-sorter-whole-intro")
        } else {
            aurora_i18n::tr!("inventory-sorter-intro")
        };
        ui.label(RichText::new(intro).small().color(p.muted));
    });
}

fn footer_row(ui: &mut egui::Ui, p: &Palette, state: &State, inventory_free: bool, go: &mut bool, stop: &mut bool) {
    ui.add_space(4.0);
    if let Some(job) = &state.job {
        let fraction = if job.total == 0 { 1.0 } else { job.done as f32 / job.total as f32 };
        let text = aurora_i18n::tr!("inventory-sorter-progress", done = job.done, total = job.total);
        ui.horizontal(|ui| {
            crate::ui::widgets::progress(
                ui,
                p,
                "inventory_sorter_progress",
                fraction,
                (ui.available_width() - 110.0).max(60.0),
                &text,
                None,
            );
            *stop = crate::ui::widgets::action_button(
                ui,
                p,
                "player-stop-filled",
                aurora_i18n::tr!("common-stop"),
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(96.0, 26.0),
            )
            .clicked();
        });
        return;
    }
    let moves = state.preview.as_ref().map_or(0, |(_, _, plan)| plan.moves.len());
    ui.horizontal(|ui| {
        let label = aurora_i18n::tr!("inventory-sorter-start", count = moves);
        *go = ui
            .add_enabled_ui(moves > 0 && !state.submitted && inventory_free, |ui| {
                crate::ui::widgets::primary_button_sized(ui, p, &label, Vec2::new(170.0, 28.0))
            })
            .inner
            .clicked();
        if !state.status.is_empty() {
            ui.label(RichText::new(&state.status).color(p.muted));
        }
    });
}

fn options(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    crate::ui::widgets::section_header(ui, p, "folder-filled", aurora_i18n::tr!("inventory-sorter-base"));
    ui.add(
        egui::TextEdit::singleline(&mut options.base)
            .char_limit(63)
            .hint_text(aurora_i18n::tr!("inventory-sorter-default-base"))
            .desired_width(240.0),
    );
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-base-hint")).small().color(p.muted));
    ui.add_space(10.0);
    crate::ui::widgets::switch_labeled(ui, p, &mut options.folders, aurora_i18n::tr!("inventory-sorter-folders"));
    ui.add_space(4.0);
    crate::ui::widgets::switch_labeled(ui, p, &mut options.stores, aurora_i18n::tr!("inventory-sorter-by-store"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-by-store-hint"))
            .small()
            .color(p.muted),
    );
    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "folder-off", aurora_i18n::tr!("inventory-sorter-excluded"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-excluded-hint"))
            .small()
            .color(p.muted),
    );
    ui.add_space(4.0);
    let mut remove = None;
    for (index, id) in options.excluded.iter().enumerate() {
        ui.horizontal(|ui| {
            crate::ui::widgets::glyph(ui, "folder-off", p.muted, 14.0);
            let text = if inv.folders.contains_key(id) {
                sorter::folder_path(inv, *id)
            } else {
                aurora_i18n::tr!("inventory-sorter-folder-missing").to_owned()
            };
            ui.label(text);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if crate::ui::widgets::icon_button(ui, p, "x", aurora_i18n::tr!("inventory-sorter-exclude-remove"), true) {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        options.excluded.remove(index);
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-exclude-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .clicked()
    {
        state.picker = Some(Picker::new(Pick::Exclude));
    }
}

/// The user's rules as cards: when, how the words meet the name, and where
/// to: a new folder under the sort folder or one of the user's own.
fn rules_editor(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, state: &mut State, options: &mut sorter::Preferences) {
    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rules-hint")).color(p.muted));
    ui.add_space(6.0);
    let mut remove = None;
    let mut swap = None;
    let count = options.rules.len();
    let base = options.base_name();
    for (index, rule) in options.rules.iter_mut().enumerate() {
        ui.push_id(("inventory_sorter_rule", index), |ui| {
            crate::ui::widgets::surface(ui, p, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::ui::widgets::switch(ui, p, &mut rule.enabled).on_hover_text(aurora_i18n::tr!("inventory-sorter-rule-enabled"));
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rule-number", n = index + 1)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-sorter-rule-remove"), true) {
                            remove = Some(index);
                        }
                        if crate::ui::widgets::icon_button(
                            ui,
                            p,
                            "arrow-down",
                            aurora_i18n::tr!("inventory-sorter-rule-down"),
                            index + 1 < count,
                        ) {
                            swap = Some((index, index + 1));
                        }
                        if crate::ui::widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("inventory-sorter-rule-up"), index > 0) {
                            swap = Some((index - 1, index));
                        }
                    });
                });
                ui.add_enabled_ui(rule.enabled, |ui| {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("inventory-sorter-rule-words"))
                            .small()
                            .color(p.muted),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.words)
                            .char_limit(120)
                            .hint_text(aurora_i18n::tr!("inventory-sorter-rule-words-hint"))
                            .desired_width(f32::INFINITY),
                    );
                    let mut mode = match rule.mode {
                        Match::All => 0,
                        Match::Any => 1,
                        Match::Phrase => 2,
                    };
                    if crate::ui::widgets::segmented(
                        ui,
                        p,
                        &mut mode,
                        &[
                            (aurora_i18n::tr!("inventory-sorter-match-all"), true),
                            (aurora_i18n::tr!("inventory-sorter-match-any"), true),
                            (aurora_i18n::tr!("inventory-sorter-match-phrase"), true),
                        ],
                    ) {
                        rule.mode = [Match::All, Match::Any, Match::Phrase][mode.min(2)];
                    }
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        crate::ui::widgets::glyph(ui, "calendar-filled", p.muted, 14.0);
                        ui.label(
                            RichText::new(aurora_i18n::tr!("inventory-sorter-rule-before"))
                                .small()
                                .color(p.muted),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut rule.before)
                                .char_limit(10)
                                .hint_text(aurora_i18n::tr!("inventory-sorter-rule-before-hint"))
                                .desired_width(110.0),
                        );
                    });
                    ui.add_space(4.0);
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rule-to")).small().color(p.muted));
                    let mut kind = if rule.pre_trash { 2 } else { usize::from(rule.target.is_some()) };
                    if crate::ui::widgets::segmented(
                        ui,
                        p,
                        &mut kind,
                        &[
                            (aurora_i18n::tr!("inventory-sorter-to-new"), true),
                            (aurora_i18n::tr!("inventory-sorter-to-mine"), true),
                            (aurora_i18n::tr!("inventory-sorter-to-pre-trash"), true),
                        ],
                    ) {
                        rule.pre_trash = kind == 2;
                        match kind {
                            1 => state.picker = Some(Picker::new(Pick::Rule(index))),
                            0 => rule.target = None,
                            _ => {}
                        }
                    }
                    match rule.target {
                        _ if rule.pre_trash => {
                            // where it looks: the whole folder sorted, at any
                            // depth, with or without the sort folder's tree
                            ui.add_space(4.0);
                            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-look-in")).small().color(p.muted));
                            let mut look = usize::from(rule.skip_sorted);
                            if crate::ui::widgets::segmented(
                                ui,
                                p,
                                &mut look,
                                &[
                                    (aurora_i18n::tr!("inventory-sorter-look-all"), true),
                                    (aurora_i18n::tr!("inventory-sorter-look-unsorted"), true),
                                ],
                            ) {
                                rule.skip_sorted = look == 1;
                            }
                            let hint = if rule.skip_sorted {
                                aurora_i18n::tr!("inventory-sorter-rule-pre-trash-unsorted-hint", folder = base.as_str())
                            } else {
                                aurora_i18n::tr!("inventory-sorter-rule-pre-trash-hint").into()
                            };
                            ui.horizontal(|ui| {
                                crate::ui::widgets::glyph(ui, "trash-filled", p.warn, 14.0);
                                ui.add(egui::Label::new(RichText::new(hint).small().color(p.muted)).wrap());
                            });
                        }
                        Some(target) => {
                            ui.horizontal(|ui| {
                                let text = if inv.folders.contains_key(&target) {
                                    sorter::folder_path(inv, target)
                                } else {
                                    aurora_i18n::tr!("inventory-sorter-folder-missing").to_owned()
                                };
                                crate::ui::widgets::glyph(ui, "folder-filled", p.violet_light, 14.0);
                                ui.label(RichText::new(text).strong());
                                if crate::ui::widgets::flat_button(ui, p, aurora_i18n::tr!("inventory-sorter-change")).clicked() {
                                    state.picker = Some(Picker::new(Pick::Rule(index)));
                                }
                            });
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{base} /")).color(p.muted));
                                ui.add(
                                    egui::TextEdit::singleline(&mut rule.folder)
                                        .char_limit(200)
                                        .hint_text(aurora_i18n::tr!("inventory-sorter-rule-folder"))
                                        .desired_width(f32::INFINITY),
                                );
                            });
                        }
                    }
                    if rule.enabled && !rule.usable() {
                        ui.label(
                            RichText::new(aurora_i18n::tr!("inventory-sorter-rule-incomplete"))
                                .small()
                                .color(p.warn),
                        );
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
    if let Some(index) = remove {
        options.rules.remove(index);
        state.picker = None;
    }
    if let Some((a, b)) = swap {
        options.rules.swap(a, b);
        state.picker = None;
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-rule-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(150.0, 26.0),
    )
    .clicked()
    {
        options.rules.push(Rule::default());
    }
}

fn preview(ui: &mut egui::Ui, p: &Palette, plan: &Plan) {
    if plan.moves.is_empty() && plan.unknown.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-nothing")).color(p.muted));
    }
    if !plan.moves.is_empty() {
        let mut summary = aurora_i18n::tr!("inventory-sorter-moves", count = plan.moves.len()).to_owned();
        let unpackers = plan.unpackers();
        if unpackers > 0 {
            summary.push_str(", ");
            summary.push_str(&aurora_i18n::tr!("inventory-sorter-unpackers", count = unpackers));
        }
        ui.label(RichText::new(summary).strong());
    }
    if plan.kept > 0 {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-kept", count = plan.kept)).color(p.muted));
    }
    ui.add_space(4.0);
    for (label, count) in plan.groups() {
        let title = format!("{label}  ({count})");
        crate::ui::widgets::collapsing(ui, p, ("inventory_sorter_group", &label), title, false, |ui| {
            for m in plan.moves.iter().filter(|m| m.label == label) {
                ui.label(RichText::new(&m.name).color(if m.unpacker { p.violet_light } else { p.ink }));
            }
        });
    }
    if !plan.unknown.is_empty() {
        let title = format!("{}  ({})", aurora_i18n::tr!("inventory-sorter-not-recognised"), plan.unknown.len());
        crate::ui::widgets::collapsing(ui, p, "inventory_sorter_unknown", title, false, |ui| {
            for name in &plan.unknown {
                ui.label(RichText::new(name).color(p.muted));
            }
        });
    }
}

impl Picker {
    fn new(pick: Pick) -> Self {
        Self {
            pick,
            filter: String::new(),
            rows: Vec::new(),
            generation: u64::MAX,
        }
    }
}

/// A searchable list of the user's folders, by path: the folder to sort, or
/// the folder a rule sends entries to.
fn picker(ctx: &egui::Context, p: &Palette, inv: &Inventory, st: &mut InventoryUi, prefs: &mut InventoryPreferences) {
    let state = &mut st.sorter;
    let Some(picker) = state.picker.as_mut() else { return };
    if picker.generation != inv.generation {
        let pick = picker.pick;
        let mut rows: Vec<(Uuid, String)> = inv
            .folders
            .values()
            .filter(|f| !f.library)
            .map(|f| f.info.id)
            .filter(|id| match pick {
                Pick::Source => sorter::source_allowed(inv, *id),
                Pick::Rule(_) => sorter::target_allowed(inv, *id),
                Pick::Exclude => *id != inv.root && !sorter::untouchable(inv, *id),
            })
            .map(|id| (id, sorter::folder_path(inv, id)))
            .collect();
        rows.sort_by_cached_key(|(_, path)| crate::natural_order::key(path));
        picker.rows = rows;
        picker.generation = inv.generation;
    }
    let mut open = true;
    let mut chosen = None;
    // the same steady size as the Auto-sort window: the list's length
    // follows the search, the window does not
    let window_id = format!("inventory_sorter_picker_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-sorter-pick-title"),
        ctx.content_rect().center() - Vec2::new(170.0, 220.0),
        Vec2::new(340.0, 440.0),
    )
    .steady_size();
    window.min_size = Vec2::new(260.0, 240.0);
    window.show(ctx, p, &mut open, |ui| {
        crate::ui::widgets::search_field(
            ui,
            p,
            &mut picker.filter,
            aurora_i18n::tr!("inventory-sorter-pick-search"),
            ui.available_width(),
        );
        ui.add_space(4.0);
        let query = picker.filter.trim().to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (id, path) in picker
                .rows
                .iter()
                .filter(|(_, path)| query.is_empty() || path.to_lowercase().contains(&query))
                .take(400)
            {
                if crate::ui::widgets::rail_row(ui, p, "folder-filled", p.violet_light, path, None, false, 0).clicked() {
                    chosen = Some(*id);
                }
            }
        });
    });
    let pick = picker.pick;
    if let Some(id) = chosen {
        match pick {
            Pick::Source => state.set_source(Source::Folder(id)),
            Pick::Rule(index) => {
                if let Some(rule) = prefs.sorter.rules.get_mut(index) {
                    rule.target = Some(id);
                    rule.pre_trash = false;
                }
            }
            Pick::Exclude => {
                if !prefs.sorter.excluded.contains(&id) {
                    prefs.sorter.excluded.push(id);
                }
            }
        }
        open = false;
    }
    if !open {
        state.picker = None;
    }
}

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
