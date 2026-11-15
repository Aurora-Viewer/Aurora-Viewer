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
use crate::world::inventory::sorter::{self, Age, CleanupPlan, Kind, Match, Move, Plan, Rule, Step};
use std::collections::VecDeque;

/// Entries sent to the Trash in one delete request.
const TRASH_BATCH: usize = 200;

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
    /// Clean up's entries, straight to the Trash.
    Trash(Vec<Uuid>),
    /// Imported settings replacing the user's rules and store hints.
    Replace,
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
    /// Clean up's list, kept until the inventory, its choices, the source or
    /// the day change.
    cleanup: Option<(u64, sorter::Cleanup, Uuid, i64, CleanupPlan)>,
    /// Clean up's rows unchecked: they stay.
    skipped: HashSet<Uuid>,
    /// Clean up's entries on their way to the Trash, a batch at a time.
    trash: Option<TrashJob>,
    /// A Clean up button pressed this frame.
    cleanup_action: Option<CleanupAction>,
    /// The Share window and what is ticked in it.
    share: Option<sorter::share::Share>,
    /// The Import window.
    import: Option<ImportWindow>,
}

/// The Import window: the pasted text, what it holds and what is ticked.
#[derive(Default)]
struct ImportWindow {
    text: String,
    /// The text `found` was read from.
    read: String,
    found: sorter::share::Imported,
    pick: sorter::share::Pick,
    done: String,
}

#[derive(Clone, Copy, PartialEq)]
enum CleanupAction {
    PreTrash,
    Trash,
}

struct TrashJob {
    queue: VecDeque<Uuid>,
    total: usize,
    sent: usize,
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
const CLEANUP: usize = 3;

impl State {
    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted || self.requested.is_some() || self.trash.is_some()
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.submitted || self.trash.is_some()
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
                state.cleanup = None;
            }
        }
    }
    send_trash_batch(inv, st, prefs, worn, actions, free);
    let state = &mut st.sorter;
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
    // Clean up's list, read on its tab only: once per inventory change (a
    // folder arriving included), choice, source or day.
    let cleanup_source = match &source {
        Source::Folder(id) => *id,
        Source::Entries(_) | Source::Whole => inv.root,
    };
    if state.tab == CLEANUP && !busy {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let day = now / 86_400;
        let stale = state.cleanup.as_ref().is_none_or(|(generation, choices, from, at, _)| {
            *generation != inv.generation || *choices != prefs.sorter.cleanup || *from != cleanup_source || *at != day
        });
        if stale {
            let plan = sorter::plan_cleanup(
                inv,
                &prefs.sorter,
                &prefs.sorter.cleanup,
                cleanup_source,
                now,
                &prefs.protected,
                worn,
            );
            for id in &plan.unfetched {
                if inv.folders.get(id).is_some_and(|f| f.state == FetchState::Unknown) {
                    inv.request(*id);
                }
            }
            state.cleanup = Some((inv.generation, prefs.sorter.cleanup.clone(), cleanup_source, day, plan));
        }
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
    let free_now = st.pending.is_none() && st.pending_delete.is_none();
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
                (aurora_i18n::tr!("inventory-cleanup-tab"), true),
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
                CLEANUP => cleanup_tab(ui, p, state, &mut prefs.sorter, matches!(source, Source::Folder(_)), busy, free_now),
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
    // Clean up's buttons: Pre-Trash through the sort's own batches, the
    // Trash after its confirmation
    match st.sorter.cleanup_action.take() {
        Some(CleanupAction::PreTrash) => {
            let state = &mut st.sorter;
            if let Some((.., plan)) = state.cleanup.as_ref() {
                let moves = plan.moves(&state.skipped);
                if !moves.is_empty() && state.job.is_none() {
                    state.status.clear();
                    state.job = Some(Job {
                        total: moves.len(),
                        queue: moves.into(),
                        base: prefs.sorter.base_name(),
                        done: 0,
                        sending: 0,
                    });
                }
            }
        }
        Some(CleanupAction::Trash) => {
            let state = &mut st.sorter;
            if let Some((.., plan)) = state.cleanup.as_ref() {
                let ids: Vec<Uuid> = plan.rows.iter().map(|r| r.id).filter(|id| !state.skipped.contains(id)).collect();
                if !ids.is_empty() {
                    state.confirm = Some(Confirm::Trash(ids));
                }
            }
        }
        None => {}
    }
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
    if stop && let Some(trash) = state.trash.as_mut() {
        trash.queue.clear();
    }
    state.open = open;
    if !open {
        state.picker = None;
        state.confirm = None;
    }
    picker(ctx, p, inv, st, prefs);
    share_window(ctx, p, st, &prefs.sorter);
    import_window(ctx, p, st, &mut prefs.sorter);
    confirm(ctx, p, inv, st, prefs, worn, actions);
}

/// A rule as one line: its words and where it sends them.
fn rule_line(rule: &Rule) -> String {
    let to = if rule.pre_trash {
        sorter::pre_trash_name()
    } else if rule.target.is_some() {
        aurora_i18n::tr!("inventory-share-my-folder").to_owned()
    } else {
        rule.folder.trim().to_owned()
    };
    format!("{}  →  {to}", rule.words.trim())
}

/// Share…: what will be shared, ticked, then copied as text.
fn share_window(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &sorter::Preferences) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox};
    let Some(share) = st.sorter.share.as_mut() else { return };
    let mut open = true;
    let mut copy = false;
    let window_id = format!("inventory_sorter_share_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-share-title"),
        ctx.content_rect().center() - Vec2::new(200.0, 240.0),
        Vec2::new(400.0, 480.0),
    )
    .steady_size();
    window.min_size = Vec2::new(320.0, 300.0);
    window.show(ctx, p, &mut open, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-share-hint")).small().color(p.muted));
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_share_list")
            .max_height((ui.available_height() - 44.0).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !prefs.rules.is_empty() {
                    let shareable: Vec<usize> = (0..prefs.rules.len())
                        .filter(|i| sorter::share::shareable(&prefs.rules[*i]))
                        .collect();
                    let mut all = !shareable.is_empty() && shareable.iter().all(|i| share.rules[*i]);
                    let before = all;
                    checkbox(ui, p, &mut all, RichText::new(aurora_i18n::tr!("inventory-sorter-rules")).strong());
                    if all != before {
                        for i in &shareable {
                            share.rules[*i] = all;
                        }
                    }
                    for (index, rule) in prefs.rules.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            if sorter::share::shareable(rule) {
                                checkbox(ui, p, &mut share.rules[index], rule_line(rule));
                            } else {
                                let mut never = false;
                                ui.add_enabled_ui(false, |ui| checkbox(ui, p, &mut never, rule_line(rule)));
                                ui.label(RichText::new(aurora_i18n::tr!("inventory-share-private")).small().color(p.muted));
                            }
                        });
                    }
                    ui.add_space(6.0);
                }
                if !prefs.store_hints.is_empty() {
                    let mut all = share.hints.iter().all(|t| *t);
                    let before = all;
                    checkbox(ui, p, &mut all, RichText::new(aurora_i18n::tr!("inventory-sorter-hints")).strong());
                    if all != before {
                        share.hints.iter_mut().for_each(|t| *t = all);
                    }
                    for (index, hint) in prefs.store_hints.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            checkbox(
                                ui,
                                p,
                                &mut share.hints[index],
                                format!("{}  →  {}", hint.store.trim(), hint.category.folder()),
                            );
                        });
                    }
                    ui.add_space(6.0);
                }
                ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-options")).strong());
                if !prefs.base.trim().is_empty() {
                    checkbox(
                        ui,
                        p,
                        &mut share.base,
                        aurora_i18n::tr!("inventory-share-base", name = prefs.base.trim()),
                    );
                }
                checkbox(ui, p, &mut share.stores, aurora_i18n::tr!("inventory-sorter-by-store"));
                checkbox(ui, p, &mut share.folders, aurora_i18n::tr!("inventory-sorter-folders"));
                checkbox(ui, p, &mut share.kinds, aurora_i18n::tr!("inventory-sorter-kinds"));
                if !prefs.excluded.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let mut never = false;
                        let label = format!("{} ({})", aurora_i18n::tr!("inventory-sorter-excluded"), prefs.excluded.len());
                        ui.add_enabled_ui(false, |ui| checkbox(ui, p, &mut never, label));
                        ui.label(RichText::new(aurora_i18n::tr!("inventory-share-private")).small().color(p.muted));
                    });
                }
            });
        crate::ui::widgets::divider(ui, p);
        ui.horizontal(|ui| {
            copy = action_button(
                ui,
                p,
                "copy-filled",
                aurora_i18n::tr!("inventory-share-copy"),
                ActionKind::Primary,
                Vec2::new(0.0, 28.0),
            )
            .clicked();
            if !st.sorter.status.is_empty() {
                ui.label(RichText::new(&st.sorter.status).small().color(p.muted));
            }
        });
    });
    if copy && let Some(share) = st.sorter.share.as_ref() {
        ctx.copy_text(sorter::share::export(prefs, share));
        st.sorter.status = aurora_i18n::tr!("inventory-share-copied").to_owned();
    }
    if !open {
        st.sorter.share = None;
    }
}

/// Import…: paste a shared text, tick what to take, then add or replace.
fn import_window(ctx: &egui::Context, p: &Palette, st: &mut InventoryUi, prefs: &mut sorter::Preferences) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox};
    let replace_open = st.sorter.confirm == Some(Confirm::Replace);
    let Some(import) = st.sorter.import.as_mut() else { return };
    if import.text != import.read {
        import.found = sorter::share::import(&import.text);
        import.pick = sorter::share::Pick::new(&import.found, prefs);
        import.read.clone_from(&import.text);
        import.done.clear();
    }
    let mut open = true;
    let (mut add, mut replace) = (false, false);
    let window_id = format!("inventory_sorter_import_{}", st.ui_id);
    let mut window = crate::ui::widgets::Floater::new(
        &window_id,
        aurora_i18n::tr!("inventory-import-title"),
        ctx.content_rect().center() - Vec2::new(210.0, 260.0),
        Vec2::new(420.0, 520.0),
    )
    .steady_size();
    window.min_size = Vec2::new(320.0, 340.0);
    window.show(ctx, p, &mut open, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-import-hint")).small().color(p.muted));
        ui.add(
            egui::TextEdit::multiline(&mut import.text)
                .char_limit(sorter::share::MAX_TEXT)
                .hint_text(sorter::share::HEADER)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let (dup_rules, dup_hints) = import.found.duplicates(prefs);
        egui::ScrollArea::vertical()
            .id_salt("inventory_sorter_import_list")
            .max_height((ui.available_height() - 64.0).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let found = &import.found;
                if !import.text.trim().is_empty() && found.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-import-nothing")).color(p.muted));
                }
                for (index, rule) in found.rules.iter().enumerate() {
                    row(ui, p, &mut import.pick.rules[index], rule_line(rule), dup_rules[index]);
                }
                for (index, hint) in found.hints.iter().enumerate() {
                    let label = format!("{}  →  {}", hint.store.trim(), hint.category.folder());
                    row(ui, p, &mut import.pick.hints[index], label, dup_hints[index]);
                }
                if found.base.is_some() || found.stores.is_some() || found.folders.is_some() || found.kinds.is_some() {
                    checkbox(ui, p, &mut import.pick.options, aurora_i18n::tr!("inventory-import-options"));
                }
                if found.skipped > 0 {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("inventory-import-skipped", count = found.skipped))
                            .small()
                            .color(p.warn),
                    );
                }
            });
        crate::ui::widgets::divider(ui, p);
        let any = import.pick.rules.iter().chain(&import.pick.hints).any(|t| *t) || import.pick.options && !import.found.is_empty();
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 28.0);
            add = ui
                .add_enabled_ui(any && !replace_open, |ui| {
                    action_button(ui, p, "plus", aurora_i18n::tr!("inventory-import-add"), ActionKind::Primary, size)
                })
                .inner
                .clicked();
            replace = ui
                .add_enabled_ui(any && !replace_open, |ui| {
                    action_button(
                        ui,
                        p,
                        "replace",
                        aurora_i18n::tr!("inventory-import-replace"),
                        ActionKind::Flat,
                        size,
                    )
                })
                .inner
                .clicked();
        });
        if !import.done.is_empty() {
            ui.label(RichText::new(&import.done).small().color(p.muted));
        }
    });
    if add {
        import.found.apply(prefs, &import.pick, false);
        import.done = aurora_i18n::tr!("inventory-import-done").to_owned();
        import.read.clear();
    }
    if replace {
        st.sorter.confirm = Some(Confirm::Replace);
    }
    if !open {
        st.sorter.import = None;
    }
}

/// An imported item and its box; one the user already has is shown
/// unticked and greyed, "already in yours".
fn row(ui: &mut egui::Ui, p: &Palette, ticked: &mut bool, label: String, mine: bool) {
    ui.horizontal(|ui| {
        if mine {
            let mut never = false;
            ui.add_enabled_ui(false, |ui| crate::ui::widgets::checkbox(ui, p, &mut never, label));
            ui.label(RichText::new(aurora_i18n::tr!("inventory-import-already")).small().color(p.muted));
        } else {
            crate::ui::widgets::checkbox(ui, p, ticked, label);
        }
    });
}

/// Clean up's Trash: one delete request of at most [`TRASH_BATCH`] entries
/// at a time, through the ordinary delete path (the Trash's own rules decide
/// what goes), the next one once the inventory is free again.
fn send_trash_batch(
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &InventoryPreferences,
    worn: &HashSet<Uuid>,
    actions: &mut Vec<InvAction>,
    free: bool,
) {
    let free = free && st.pending_delete.is_none() && !actions.iter().any(|a| matches!(a, InvAction::Delete(_)));
    let Some(trash) = st.sorter.trash.as_mut() else { return };
    if !free {
        return;
    }
    if trash.queue.is_empty() {
        st.sorter.status = aurora_i18n::tr!("inventory-cleanup-trash-done", count = trash.sent);
        st.sorter.trash = None;
        st.sorter.cleanup = None;
        st.sorter.skipped.clear();
        return;
    }
    let take = trash.queue.len().min(TRASH_BATCH);
    let batch: Vec<Uuid> = trash.queue.drain(..take).collect();
    trash.sent += batch.len();
    // what left or can no longer go is dropped, like a sort's moves
    let batch: Vec<Uuid> = batch
        .into_iter()
        .filter(|id| inv.items.contains_key(id) || inv.folders.contains_key(id))
        .collect();
    if batch.is_empty() {
        return;
    }
    let change = rules::system(inv, 14)
        .ok_or_else(|| aurora_i18n::tr!("common-trash-folder-not-found").to_owned())
        .and_then(|trash| rules::move_selection(inv, &batch, trash, &prefs.protected, worn));
    match change {
        Ok(change) => actions.push(InvAction::Delete(Box::new(DeleteRequest::new(inv, st, &batch, change)))),
        Err(reason) => {
            st.sorter.status = reason;
            st.sorter.trash = None;
        }
    }
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
    prefs: &mut InventoryPreferences,
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
            Confirm::PreTrash(ids) | Confirm::Trash(ids) => {
                let (title, text) = if matches!(asked, Confirm::Trash(_)) {
                    (
                        aurora_i18n::tr!("inventory-cleanup-trash-title"),
                        aurora_i18n::tr!("inventory-cleanup-trash-text", count = ids.len()),
                    )
                } else {
                    (
                        aurora_i18n::tr!("inventory-sorter-pre-trash-title"),
                        aurora_i18n::tr!("inventory-sorter-pre-trash-text", count = ids.len()),
                    )
                };
                ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
                ui.add_space(8.0);
                ui.label(text);
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
            Confirm::Replace => {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-import-replace-title"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!(
                    "inventory-import-replace-text",
                    rules = prefs.sorter.rules.len(),
                    hints = prefs.sorter.store_hints.len()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        p,
                        "replace",
                        aurora_i18n::tr!("inventory-import-replace"),
                        ActionKind::Danger,
                        size,
                    )
                    .clicked()
                    {
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
        Confirm::Replace => {
            if let Some(import) = st.sorter.import.as_mut() {
                import.found.apply(&mut prefs.sorter, &import.pick, true);
                import.done = aurora_i18n::tr!("inventory-import-done").to_owned();
                import.read.clear();
            }
        }
        Confirm::Trash(ids) => {
            st.sorter.status.clear();
            st.sorter.trash = Some(TrashJob {
                total: ids.len(),
                queue: ids.into(),
                sent: 0,
            });
        }
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
    if let Some(trash) = &state.trash {
        let fraction = if trash.total == 0 {
            1.0
        } else {
            trash.sent as f32 / trash.total as f32
        };
        let text = aurora_i18n::tr!("inventory-cleanup-trash-progress", done = trash.sent, total = trash.total);
        ui.horizontal(|ui| {
            crate::ui::widgets::progress(
                ui,
                p,
                "inventory_cleanup_trash",
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
    if state.tab == CLEANUP && state.job.is_none() {
        // Clean up has its own buttons in the tab
        if !state.status.is_empty() {
            ui.label(RichText::new(&state.status).color(p.muted));
        }
        return;
    }
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

fn month_name(month: u32) -> &'static str {
    match month {
        1 => aurora_i18n::tr!("inventory-cleanup-month-1"),
        2 => aurora_i18n::tr!("inventory-cleanup-month-2"),
        3 => aurora_i18n::tr!("inventory-cleanup-month-3"),
        4 => aurora_i18n::tr!("inventory-cleanup-month-4"),
        5 => aurora_i18n::tr!("inventory-cleanup-month-5"),
        6 => aurora_i18n::tr!("inventory-cleanup-month-6"),
        7 => aurora_i18n::tr!("inventory-cleanup-month-7"),
        8 => aurora_i18n::tr!("inventory-cleanup-month-8"),
        9 => aurora_i18n::tr!("inventory-cleanup-month-9"),
        10 => aurora_i18n::tr!("inventory-cleanup-month-10"),
        11 => aurora_i18n::tr!("inventory-cleanup-month-11"),
        _ => aurora_i18n::tr!("inventory-cleanup-month-12"),
    }
}

/// "1 March 2020", in the catalogue's order (UTC, like the cutoff).
fn date_text(at: i64) -> String {
    use chrono::Datelike;
    let Some(day) = chrono::DateTime::from_timestamp(at, 0).map(|t| t.date_naive()) else {
        return String::new();
    };
    // as text, so no number formatting groups the year's digits
    let (number, year) = (day.day().to_string(), day.year().to_string());
    aurora_i18n::tr!(
        "inventory-cleanup-date",
        day = number.as_str(),
        month = month_name(day.month()),
        year = year.as_str()
    )
}

/// A row's acquired date, `2014-06-03`.
fn short_date(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0).map_or_else(String::new, |t| t.date_naive().format("%Y-%m-%d").to_string())
}

/// The item kinds as two columns of boxes, with All and None: Clean up's
/// filter and the sort's.
fn kinds_grid(ui: &mut egui::Ui, p: &Palette, id: &str, kinds: &mut Vec<Kind>) {
    use crate::ui::widgets::{checkbox, flat_button};
    ui.horizontal(|ui| {
        if flat_button(ui, p, aurora_i18n::tr!("inventory-cleanup-all")).clicked() {
            *kinds = Kind::ALL.to_vec();
        }
        if flat_button(ui, p, aurora_i18n::tr!("inventory-cleanup-none")).clicked() {
            kinds.clear();
        }
    });
    egui::Grid::new(id).num_columns(2).spacing([18.0, 2.0]).show(ui, |ui| {
        for (index, kind) in Kind::ALL.iter().enumerate() {
            let was = kinds.contains(kind);
            let mut on = was;
            checkbox(ui, p, &mut on, kind.label());
            if on != was {
                if on {
                    kinds.push(*kind);
                    kinds.sort();
                } else {
                    kinds.retain(|k| k != kind);
                }
            }
            if index % 2 == 1 {
                ui.end_row();
            }
        }
    });
}

/// Rows drawn per group: the rest is counted, so a large cleanup stays
/// smooth (the groups open one at a time).
const CLEANUP_ROWS: usize = 300;

/// The Clean up tab: how old (chips, or a month and a year), which kinds,
/// where, then the list by kind with a box per row (unchecked rows stay) and
/// the two ways out: Pre-Trash, or the Trash after a confirmation.
fn cleanup_tab(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &mut State,
    options: &mut sorter::Preferences,
    has_folder: bool,
    busy: bool,
    free: bool,
) {
    use crate::ui::widgets::{ActionKind, action_button, checkbox, collapsing, section_header, segmented, switch_labeled};
    let base = options.base_name();
    let cleanup = &mut options.cleanup;
    ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-intro")).small().color(p.muted));
    ui.add_space(6.0);
    ui.add_enabled_ui(!busy, |ui| {
        section_header(ui, p, "calendar-filled", aurora_i18n::tr!("inventory-cleanup-when"));
        const AGES: [Age; 5] = [Age::OneYear, Age::TwoYears, Age::ThreeYears, Age::FiveYears, Age::Custom];
        let mut age = AGES.iter().position(|a| *a == cleanup.age).unwrap_or(0);
        if segmented(
            ui,
            p,
            &mut age,
            &[
                (aurora_i18n::tr!("inventory-cleanup-age-1"), true),
                (aurora_i18n::tr!("inventory-cleanup-age-2"), true),
                (aurora_i18n::tr!("inventory-cleanup-age-3"), true),
                (aurora_i18n::tr!("inventory-cleanup-age-5"), true),
                (aurora_i18n::tr!("inventory-cleanup-age-custom"), true),
            ],
        ) {
            cleanup.age = AGES[age.min(AGES.len() - 1)];
        }
        let now = chrono::Utc::now();
        if cleanup.age == Age::Custom {
            use chrono::Datelike;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("inventory_cleanup_month")
                    .selected_text(month_name(cleanup.month))
                    .show_ui(ui, |ui| {
                        for month in 1..=12 {
                            ui.selectable_value(&mut cleanup.month, month, month_name(month));
                        }
                    });
                egui::ComboBox::from_id_salt("inventory_cleanup_year")
                    .selected_text(cleanup.year.to_string())
                    .show_ui(ui, |ui| {
                        for year in (2003..=now.year()).rev() {
                            ui.selectable_value(&mut cleanup.year, year, year.to_string());
                        }
                    });
            });
        }
        if let Some(cutoff) = cleanup.cutoff(now.timestamp()) {
            ui.add_space(2.0);
            let date = date_text(cutoff);
            ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-before", date = date.as_str())).strong());
        }
        ui.add_space(8.0);
        section_header(ui, p, "category-filled", aurora_i18n::tr!("inventory-cleanup-types"));
        kinds_grid(ui, p, "inventory_cleanup_kinds", &mut cleanup.kinds);
        ui.add_space(8.0);
        section_header(ui, p, "folder-filled", aurora_i18n::tr!("inventory-cleanup-where"));
        let mut place = usize::from(!cleanup.whole && has_folder);
        if segmented(
            ui,
            p,
            &mut place,
            &[
                (aurora_i18n::tr!("inventory-cleanup-whole"), true),
                (aurora_i18n::tr!("inventory-cleanup-this-folder"), has_folder),
            ],
        ) {
            cleanup.whole = place == 0;
        }
        switch_labeled(
            ui,
            p,
            &mut cleanup.include_sorted,
            aurora_i18n::tr!("inventory-cleanup-include-sorted", folder = base.as_str()),
        );
        switch_labeled(
            ui,
            p,
            &mut cleanup.empty_folders,
            aurora_i18n::tr!("inventory-cleanup-empty-folders"),
        );
        ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-never")).small().color(p.muted));
    });
    crate::ui::widgets::divider(ui, p);
    let State {
        cleanup: list,
        skipped,
        cleanup_action,
        ..
    } = state;
    let Some((.., plan)) = list.as_ref() else { return };
    if !plan.unfetched.is_empty() {
        ui.label(
            RichText::new(aurora_i18n::tr!("inventory-cleanup-loading", count = plan.unfetched.len()))
                .small()
                .color(p.muted),
        );
    }
    let date = date_text(plan.cutoff);
    if plan.rows.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-nothing", date = date.as_str())).color(p.muted));
        return;
    }
    let going: Vec<_> = plan.rows.iter().filter(|r| !skipped.contains(&r.id)).collect();
    let items: usize = going.iter().map(|r| r.items).sum();
    ui.label(RichText::new(aurora_i18n::tr!("inventory-cleanup-summary", count = items, date = date.as_str())).strong());
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let size = Vec2::new(0.0, 26.0);
        let enabled = !going.is_empty() && !busy && free;
        let label = aurora_i18n::tr!("inventory-cleanup-to-pre-trash", count = going.len());
        if ui
            .add_enabled_ui(enabled, |ui| {
                action_button(ui, p, "trash-filled", &label, ActionKind::Primary, size)
            })
            .inner
            .clicked()
        {
            *cleanup_action = Some(CleanupAction::PreTrash);
        }
        let label = aurora_i18n::tr!("inventory-cleanup-to-trash", count = going.len());
        if ui
            .add_enabled_ui(enabled, |ui| {
                action_button(ui, p, "trash-x-filled", &label, ActionKind::Danger, size)
            })
            .inner
            .clicked()
        {
            *cleanup_action = Some(CleanupAction::Trash);
        }
    });
    ui.add_space(6.0);
    // folders first (they go whole), then each kind in the filter's order
    let mut groups: Vec<(String, Vec<&sorter::CleanupRow>)> = Vec::new();
    let folders: Vec<_> = plan.rows.iter().filter(|r| r.folder && r.items > 0).collect();
    if !folders.is_empty() {
        groups.push((aurora_i18n::tr!("inventory-cleanup-folders").to_owned(), folders));
    }
    let empty: Vec<_> = plan.rows.iter().filter(|r| r.folder && r.items == 0).collect();
    if !empty.is_empty() {
        groups.push((aurora_i18n::tr!("inventory-cleanup-empty-group").to_owned(), empty));
    }
    for kind in Kind::ALL {
        let rows: Vec<_> = plan.rows.iter().filter(|r| r.kind == Some(kind)).collect();
        if !rows.is_empty() {
            groups.push((kind.label().to_owned(), rows));
        }
    }
    for (label, rows) in groups {
        let title = format!("{label}  ({})", rows.len());
        collapsing(ui, p, ("inventory_cleanup_group", &label), title, false, |ui| {
            for row in rows.iter().take(CLEANUP_ROWS) {
                ui.horizontal(|ui| {
                    let mut on = !skipped.contains(&row.id);
                    checkbox(ui, p, &mut on, &row.name).on_hover_text(aurora_i18n::tr!("inventory-cleanup-keep-tip"));
                    if on {
                        skipped.remove(&row.id);
                    } else {
                        skipped.insert(row.id);
                    }
                    let mut when = short_date(row.created);
                    if row.folder {
                        when = format!("{when} · {}", aurora_i18n::tr!("inventory-cleanup-folder-items", count = row.items));
                    }
                    ui.label(RichText::new(when).small().color(p.muted));
                });
            }
            if rows.len() > CLEANUP_ROWS {
                ui.label(
                    RichText::new(aurora_i18n::tr!("inventory-cleanup-more", count = rows.len() - CLEANUP_ROWS))
                        .small()
                        .color(p.muted),
                );
            }
        });
    }
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
    crate::ui::widgets::section_header(ui, p, "category-filled", aurora_i18n::tr!("inventory-sorter-kinds"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-kinds-hint"))
            .small()
            .color(p.muted),
    );
    kinds_grid(ui, p, "inventory_sorter_kinds", &mut options.kinds);
    ui.add_space(10.0);
    store_hints(ui, p, options);
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
    ui.add_space(10.0);
    crate::ui::widgets::section_header(ui, p, "share", aurora_i18n::tr!("inventory-share-section"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-share-section-hint"))
            .small()
            .color(p.muted),
    );
    ui.horizontal(|ui| {
        use crate::ui::widgets::{ActionKind, action_button};
        let size = Vec2::new(0.0, 26.0);
        if action_button(ui, p, "share", aurora_i18n::tr!("inventory-share-open"), ActionKind::Flat, size).clicked() {
            state.status.clear();
            state.share = Some(sorter::share::Share::all(options));
        }
        if action_button(
            ui,
            p,
            "file-import",
            aurora_i18n::tr!("inventory-import-open"),
            ActionKind::Flat,
            size,
        )
        .clicked()
        {
            state.import = Some(ImportWindow::default());
        }
    });
}

/// Store hints: a store whose names say nothing of what it sells, and the
/// category its items go to.
fn store_hints(ui: &mut egui::Ui, p: &Palette, options: &mut sorter::Preferences) {
    use sorter::{Category, StoreHint};
    crate::ui::widgets::section_header(ui, p, "building-store", aurora_i18n::tr!("inventory-sorter-hints"));
    ui.label(
        RichText::new(aurora_i18n::tr!("inventory-sorter-hints-hint"))
            .small()
            .color(p.muted),
    );
    let mut remove = None;
    for (index, hint) in options.store_hints.iter_mut().enumerate() {
        ui.push_id(("inventory_sorter_hint", index), |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut hint.store)
                        .char_limit(64)
                        .hint_text(aurora_i18n::tr!("inventory-sorter-hint-store"))
                        .desired_width(150.0),
                );
                ui.label(RichText::new("→").color(p.muted));
                egui::ComboBox::from_id_salt("category")
                    .selected_text(hint.category.folder())
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for category in Category::ALL {
                            ui.selectable_value(&mut hint.category, category, category.folder());
                        }
                    });
                if crate::ui::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-sorter-hint-remove"), true) {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        options.store_hints.remove(index);
    }
    if crate::ui::widgets::action_button(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("inventory-sorter-hint-add"),
        crate::ui::widgets::ActionKind::Flat,
        Vec2::new(0.0, 26.0),
    )
    .clicked()
    {
        options.store_hints.push(StoreHint::default());
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
