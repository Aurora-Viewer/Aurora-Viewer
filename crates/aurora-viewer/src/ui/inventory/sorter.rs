//! The Auto-sort window: the rules of `world::inventory::sorter`, a preview of
//! where every entry of a folder goes, and the sort itself, sent as bounded
//! inventory mutations one after another through the ordinary pending edit
//! (like Bulk permissions), so that nothing else races it.

use super::*;
use crate::world::inventory::sorter::{self, Move, Plan, Rule, Step};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct State {
    source: Option<Uuid>,
    open: bool,
    /// The preview, kept until the inventory or the options change.
    preview: Option<(u64, sorter::Preferences, Plan)>,
    job: Option<Job>,
    /// A batch is on its way; its reply comes through `finish_sorter`.
    submitted: bool,
    status: String,
    /// `AURORA_DEMO_INVENTORY=sorter-run`: start once the preview is ready.
    autostart: bool,
}

struct Job {
    queue: VecDeque<Move>,
    base: String,
    total: usize,
    done: usize,
    sending: usize,
}

impl State {
    pub(super) fn active(&self) -> bool {
        self.open || self.job.is_some() || self.submitted
    }
}

impl InventoryUi {
    pub(crate) fn open_sorter(&mut self, source: Uuid) {
        let state = &mut self.sorter;
        state.open = true;
        if state.job.is_some() || state.submitted {
            return;
        }
        if state.source != Some(source) {
            state.preview = None;
            state.status.clear();
        }
        state.source = Some(source);
    }

    /// The demo's unattended run: Auto-sort opens and starts by itself.
    pub(crate) fn demo_run_sorter(&mut self, source: Uuid) {
        self.open_sorter(source);
        self.sorter.autostart = true;
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
    let free = st.pending.is_none() && !actions.iter().any(|action| matches!(action, InvAction::Edit(_)));
    let state = &mut st.sorter;
    if free
        && !state.submitted
        && let Some(source) = state.source
        && let Some(job) = state.job.as_mut()
    {
        match sorter::step(inv, source, &job.base, &mut job.queue, &prefs.protected, worn) {
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
    let Some(source) = state.source.filter(|id| inv.folders.contains_key(id)) else {
        state.open = false;
        return;
    };
    let busy = state.job.is_some() || state.submitted;
    // Loading the folder is all a preview needs: its direct contents.
    let loaded = inv.folders.get(&source).is_some_and(|f| f.state == FetchState::Fetched);
    if !loaded {
        inv.request(source);
    }
    if loaded
        && !busy
        && state
            .preview
            .as_ref()
            .is_none_or(|(generation, options, _)| *generation != inv.generation || *options != prefs.sorter)
    {
        let plan = sorter::plan(inv, source, &prefs.sorter, &prefs.protected, worn);
        state.preview = Some((inv.generation, prefs.sorter.clone(), plan));
    }
    let folder_name = rules::name(inv, source);
    let mut open = true;
    let mut start = false;
    let mut stop = false;
    crate::ui::widgets::Floater::new(
        &format!("inventory_sorter_{}", st.ui_id),
        aurora_i18n::tr!("inventory-sorter-title"),
        ctx.content_rect().center() - Vec2::new(230.0, 280.0),
        Vec2::new(460.0, 560.0),
    )
    .show(ctx, p, &mut open, |ui| {
        let state = &mut st.sorter;
        ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-intro", folder = folder_name.as_str())).color(p.muted));
        ui.add_space(6.0);
        ui.add_enabled_ui(!busy, |ui| options(ui, p, &mut prefs.sorter));
        crate::ui::widgets::divider(ui, p);
        let plan = state.preview.as_ref().map(|(_, _, plan)| plan);
        let footer = 64.0;
        egui::ScrollArea::vertical()
            .max_height((ui.available_height() - footer).max(80.0))
            .auto_shrink([false, true])
            .show(ui, |ui| match plan {
                _ if !loaded => {
                    ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-loading")).color(p.muted));
                }
                Some(plan) => preview(ui, p, plan),
                None => {}
            });
        crate::ui::widgets::divider(ui, p);
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
                stop = crate::ui::widgets::action_button(
                    ui,
                    p,
                    "player-stop-filled",
                    aurora_i18n::tr!("common-stop"),
                    crate::ui::widgets::ActionKind::Flat,
                    Vec2::new(96.0, 26.0),
                )
                .clicked();
            });
        } else {
            let moves = plan.map_or(0, |plan| plan.moves.len());
            ui.horizontal(|ui| {
                let label = aurora_i18n::tr!("inventory-sorter-start", count = moves);
                start = ui
                    .add_enabled_ui(moves > 0 && !busy && st.pending.is_none(), |ui| {
                        crate::ui::widgets::primary_button_sized(ui, p, &label, Vec2::new(170.0, 28.0))
                    })
                    .inner
                    .clicked();
                if !state.status.is_empty() {
                    ui.label(RichText::new(&state.status).color(p.muted));
                }
            });
        }
    });
    let state = &mut st.sorter;
    if state.autostart && state.preview.is_some() {
        state.autostart = false;
        start = true;
    }
    if start
        && let Some((_, _, plan)) = state.preview.as_ref()
        && state.job.is_none()
    {
        state.status.clear();
        state.job = Some(Job {
            queue: plan.moves.iter().cloned().collect(),
            base: prefs.sorter.base_name(),
            total: plan.moves.len(),
            done: 0,
            sending: 0,
        });
    }
    if stop && let Some(job) = state.job.as_mut() {
        // the batch on its way finishes; nothing after it is sent
        job.queue.clear();
    }
    state.open = open;
}

fn options(ui: &mut egui::Ui, p: &Palette, options: &mut sorter::Preferences) {
    ui.horizontal(|ui| {
        ui.label(aurora_i18n::tr!("inventory-sorter-base"));
        ui.add(
            egui::TextEdit::singleline(&mut options.base)
                .char_limit(63)
                .hint_text(aurora_i18n::tr!("inventory-sorter-default-base"))
                .desired_width(200.0),
        )
        .on_hover_text(aurora_i18n::tr!("inventory-sorter-base-hint"));
    });
    crate::ui::widgets::checkbox(ui, p, &mut options.folders, aurora_i18n::tr!("inventory-sorter-folders"));
    crate::ui::widgets::collapsing(
        ui,
        p,
        "inventory_sorter_rules",
        aurora_i18n::tr!("inventory-sorter-rules"),
        false,
        |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("inventory-sorter-rules-hint")).color(p.muted));
            let mut remove = None;
            for (index, rule) in options.rules.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.words)
                            .char_limit(80)
                            .hint_text(aurora_i18n::tr!("inventory-sorter-rule-words"))
                            .desired_width(150.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut rule.folder)
                            .char_limit(200)
                            .hint_text(aurora_i18n::tr!("inventory-sorter-rule-folder"))
                            .desired_width(180.0),
                    );
                    if crate::ui::widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("inventory-sorter-rule-remove"), true) {
                        remove = Some(index);
                    }
                });
            }
            if let Some(index) = remove {
                options.rules.remove(index);
            }
            if crate::ui::widgets::action_button(
                ui,
                p,
                "plus",
                aurora_i18n::tr!("inventory-sorter-rule-add"),
                crate::ui::widgets::ActionKind::Flat,
                Vec2::new(140.0, 24.0),
            )
            .clicked()
            {
                options.rules.push(Rule::default());
            }
        },
    );
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
    for (path, count) in plan.groups() {
        let title = format!("{path}  ({count})");
        crate::ui::widgets::collapsing(ui, p, ("inventory_sorter_group", &path), title, false, |ui| {
            for m in plan.moves.iter().filter(|m| m.path.join(" / ") == path) {
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

#[cfg(test)]
#[path = "sorter_tests.rs"]
mod tests;
