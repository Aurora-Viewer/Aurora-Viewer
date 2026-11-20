//! LLFloaterBulkPermission: eleven type filters above three permission
//! columns and the request status (Firestorm llfloaterbulkpermission.cpp,
//! floater_bulk_perms.xml, originally LGPL 2.1). Aurora widgets and palette.

use super::common::{check, label, section, small};
use crate::build::BuildTool;
use crate::theme::Palette;
use crate::ui::widgets::{self, Floater};
use crate::world::World;
use aurora_net::build::{BuildCmd, TaskItem, perm};
use egui::Vec2;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

// Column-major, matching the type picker in floater_bulk_perms.xml.
const TYPES: [i8; 11] = [20, 13, 5, 21, 7, 6, 10, 1, 0, 56, 57];

#[cfg(test)]
#[path = "contents_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct BulkPerms {
    pub types: [bool; 11],
    pub share: bool,
    pub everyone_copy: bool,
    pub next_modify: bool,
    pub next_copy: bool,
    pub next_transfer: bool,
    /// "Include objects inside objects" (`build/deep_perms.rs`): object
    /// contents only, the inventory window never shows it.
    pub nested: bool,
}

impl Default for BulkPerms {
    fn default() -> Self {
        Self {
            types: [true; 11],
            share: false,
            everyone_copy: false,
            next_modify: false,
            next_copy: true,
            next_transfer: false,
            nested: false,
        }
    }
}

impl BulkPerms {
    pub(crate) fn apply(&self, item: &mut TaskItem) {
        let before = [item.group_mask, item.everyone_mask, item.next_owner_mask];
        item.group_mask = if self.share { perm::COPY | perm::MODIFY | perm::MOVE } else { 0 };
        item.everyone_mask = if self.everyone_copy { perm::COPY } else { 0 };
        item.next_owner_mask = perm::MOVE;
        if self.next_modify {
            item.next_owner_mask |= perm::MODIFY;
        }
        if self.next_copy || item.asset_type == 56 {
            item.next_owner_mask |= perm::COPY;
        }
        // LLFloaterBulkPermission::onCommitCopy: no-copy remains transferable.
        if self.next_transfer || !self.next_copy {
            item.next_owner_mask |= perm::TRANSFER;
        }
        if item.asset_type == 6 {
            item.flags = crate::world::inventory::actions::object_permission_flags(
                item.flags,
                before,
                [item.group_mask, item.everyone_mask, item.next_owner_mask],
            );
        }
    }

    pub(crate) fn includes_type(&self, asset_type: i32) -> bool {
        TYPES
            .iter()
            .position(|t| i32::from(*t) == asset_type)
            .is_some_and(|i| self.types[i])
    }

    pub(crate) fn includes(&self, item: &TaskItem) -> bool {
        !item.is_folder && self.includes_type(i32::from(item.asset_type))
    }
}

struct Job {
    options: BulkPerms,
    targets: VecDeque<Uuid>,
    failed: HashSet<Uuid>,
    requested: Option<Instant>,
    updated: usize,
    skipped: usize,
}

#[derive(Default)]
pub struct BulkState {
    pub options: BulkPerms,
    original: Option<BulkPerms>,
    job: Option<Job>,
    pub(crate) deep: Option<crate::build::deep_perms::Job>,
    pub(crate) status: String,
    /// `AURORA_DEMO_DEEP_PERMS=run`: started once.
    demo_started: bool,
}

fn type_name(t: i8) -> &'static str {
    match t {
        20 => aurora_i18n::tr!("common-animations"),
        13 => aurora_i18n::tr!("content-perms-body-parts"),
        5 => aurora_i18n::tr!("common-clothing"),
        21 => aurora_i18n::tr!("content-perms-gestures"),
        7 => aurora_i18n::tr!("common-notes"),
        6 => aurora_i18n::tr!("common-objects"),
        10 => aurora_i18n::tr!("common-scripts"),
        1 => aurora_i18n::tr!("common-sounds"),
        0 => aurora_i18n::tr!("common-textures"),
        56 => aurora_i18n::tr!("content-perms-environment"),
        _ => aurora_i18n::tr!("ui-inventory-view-materials"),
    }
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, tool: &mut BuildTool, world: &World) {
    if !tool.ui.bulk_perms_open {
        return;
    }
    let running = tool.ui.bulk.job.is_some() || tool.ui.bulk.deep.is_some();
    // Stop while a nested job runs and has not been asked to stop yet
    let stoppable = tool.ui.bulk.deep.as_ref().is_some_and(|j| !j.stop_requested());
    let mut stop = false;
    let preview = if tool.ui.bulk.options.nested && !running {
        tool.nested_preview(world)
    } else {
        String::new()
    };
    let state = &mut tool.ui.bulk;
    state.original.get_or_insert_with(|| state.options.clone());
    let mut open = true;
    let mut apply = false;
    let mut accept = false;
    let mut cancel = false;
    let b = &mut state.options;
    Floater::new(
        "bulk_perms",
        aurora_i18n::tr!("build-ui-floaters-content-permissions"),
        ctx.content_rect().center() - Vec2::new(180.0, 177.0),
        Vec2::new(360.0, 354.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        (apply, accept, cancel) = controls(
            ui,
            p,
            b,
            &state.status,
            running,
            aurora_i18n::tr!("content-perms-adjust"),
            Some(&preview),
            stoppable.then_some(&mut stop),
        );
    });
    if stop && let Some(job) = tool.ui.bulk.deep.as_mut() {
        log::info!("deep permissions: Stop pressed");
        job.request_stop();
    }
    if tool.demo_deep_start(world) {
        apply = true;
    }
    if apply || accept {
        tool.start_bulk_permissions(world);
    }
    if accept {
        open = false;
        tool.ui.bulk.original = None;
    }
    if cancel || !open && !accept {
        if let Some(original) = tool.ui.bulk.original.take() {
            tool.ui.bulk.options = original;
        }
        open = false;
    }
    tool.ui.bulk_perms_open = open;
}

/// Shared bulk-permission controls for object contents and agent inventory.
/// `nested` (object contents only) shows "Include objects inside objects"
/// with the preview of what it would open; `stop`, while such a job runs,
/// a Stop button in place of OK and Apply.
#[allow(clippy::too_many_arguments)]
pub(crate) fn controls(
    ui: &mut egui::Ui,
    p: &Palette,
    b: &mut BulkPerms,
    status: &str,
    running: bool,
    heading: &str,
    nested: Option<&str>,
    stop: Option<&mut bool>,
) -> (bool, bool, bool) {
    let (mut apply, mut accept, mut cancel) = (false, false, false);
    // Keep the actions reachable when enlarged text, a nested preview or a
    // status needs more room. Measure the footer and scroll only the options
    // above it, so neither can overlap the other.
    let (mut body, footer) = widgets::above_footer(ui);
    egui::ScrollArea::vertical()
        .id_salt("bulk_permission_options")
        .show(&mut body, |ui| {
            let filter_width = [aurora_i18n::tr!("common-all"), aurora_i18n::tr!("common-clear")]
                .into_iter()
                .map(|text| {
                    ui.painter()
                        .layout_no_wrap(text.to_owned(), egui::TextStyle::Button.resolve(ui.style()), p.ink)
                        .size()
                        .x
                        + 2.0 * ui.spacing().button_padding.x
                })
                .fold(98.0, f32::max);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                egui::Grid::new("content_permission_types")
                    .spacing(Vec2::new(12.0, 3.0))
                    .show(ui, |ui| {
                        for row in 0..3 {
                            for col in 0..4 {
                                let i = col * 3 + row;
                                if let Some(t) = TYPES.get(i) {
                                    ui.push_id(t, |ui| {
                                        ui.horizontal(|ui| {
                                            widgets::checkbox(ui, p, &mut b.types[i], "").on_hover_text(type_name(*t));
                                            if let Some(icon) =
                                                crate::ui::icons::global(crate::ui::inventory::item_icon(i32::from(*t), i32::from(*t), 0))
                                            {
                                                let color = crate::ui::inventory::item_icon_color(i32::from(*t), i32::from(*t), p);
                                                ui.add(egui::Image::new(&icon).fit_to_exact_size(Vec2::splat(18.0)).tint(color))
                                                    .on_hover_text(type_name(*t));
                                            } else {
                                                ui.allocate_space(Vec2::splat(18.0));
                                            }
                                        });
                                    });
                                } else {
                                    ui.label("");
                                }
                            }
                            ui.end_row();
                        }
                    });
                ui.allocate_ui_with_layout(Vec2::new(filter_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    if ui
                        .add_sized(
                            [filter_width, 25.0],
                            egui::Button::new(aurora_i18n::tr!("common-all")).wrap_mode(egui::TextWrapMode::Extend),
                        )
                        .clicked()
                    {
                        b.types.fill(true);
                    }
                    if ui
                        .add_sized(
                            [filter_width, 25.0],
                            egui::Button::new(aurora_i18n::tr!("common-clear")).wrap_mode(egui::TextWrapMode::Extend),
                        )
                        .clicked()
                    {
                        b.types.fill(false);
                    }
                });
            });
            section(ui, p, heading);
            // the columns' widths, wider than the layout's when a larger text size
            // would wrap their words ("Shar/e")
            let group_width = column_width(ui, p, 82.0, aurora_i18n::tr!("common-group-2"), aurora_i18n::tr!("common-share"));
            let anyone_width = column_width(
                ui,
                p,
                90.0,
                aurora_i18n::tr!("content-perms-anyone"),
                aurora_i18n::tr!("common-copy-2"),
            );
            let next_width = column_width(
                ui,
                p,
                100.0,
                aurora_i18n::tr!("content-perms-next-owner"),
                aurora_i18n::tr!("common-transfer"),
            )
            .max(column_width(ui, p, 0.0, "", aurora_i18n::tr!("common-edit")))
            .max(column_width(ui, p, 0.0, "", aurora_i18n::tr!("common-copy-2")));
            // Preserve complete words in every permission column; larger text
            // moves a column to the next row instead of squeezing the last one.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                ui.allocate_ui_with_layout(Vec2::new(group_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(group_width);
                    label(ui, p, aurora_i18n::tr!("common-group-2"));
                    if let Some(v) = check(ui, p, b.share, false, aurora_i18n::tr!("common-share"), true) {
                        b.share = v;
                    }
                });
                ui.allocate_ui_with_layout(Vec2::new(anyone_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(anyone_width);
                    label(ui, p, aurora_i18n::tr!("content-perms-anyone"));
                    if let Some(v) = check(ui, p, b.everyone_copy, false, aurora_i18n::tr!("common-copy-2"), true) {
                        b.everyone_copy = v;
                    }
                });
                ui.allocate_ui_with_layout(Vec2::new(next_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(next_width);
                    label(ui, p, aurora_i18n::tr!("content-perms-next-owner"));
                    if let Some(v) = check(ui, p, b.next_modify, false, aurora_i18n::tr!("common-edit"), true) {
                        b.next_modify = v;
                    }
                    if let Some(v) = check(ui, p, b.next_copy, false, aurora_i18n::tr!("common-copy-2"), true) {
                        b.next_copy = v;
                    }
                    if !b.next_copy {
                        b.next_transfer = true;
                    }
                    if let Some(v) = check(ui, p, b.next_transfer, false, aurora_i18n::tr!("common-transfer"), b.next_copy) {
                        b.next_transfer = v;
                    }
                });
            });
            ui.add_space(8.0);
            if let Some(preview) = nested {
                ui.add_enabled_ui(!running, |ui| {
                    widgets::checkbox(ui, p, &mut b.nested, aurora_i18n::tr!("content-perms-nested"))
                        .on_hover_text(aurora_i18n::tr!("content-perms-nested-help"));
                });
                if b.nested && !running && !preview.is_empty() {
                    small(ui, p, preview);
                }
                // scripts react to their own rez on the simulator, before any viewer
                // can stop them: RezObject has no way to rez with scripts off
                if b.nested && !running {
                    ui.label(
                        egui::RichText::new(aurora_i18n::tr!("content-perms-nested-scripts-warning"))
                            .size(11.0)
                            .color(p.warn),
                    );
                }
                ui.add_space(4.0);
            }
            if !status.is_empty() {
                small(ui, p, status);
            }
        });
    footer.show(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(stop) = stop {
                *stop = ui
                    .button(aurora_i18n::tr!("common-stop"))
                    .on_hover_text(aurora_i18n::tr!("content-perms-nested-stop-help"))
                    .clicked();
            } else {
                let enabled = !running && b.types.iter().any(|on| *on);
                accept = ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-ok"))).clicked();
                apply = ui
                    .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-apply")))
                    .clicked();
            }
            cancel = ui.button(aurora_i18n::tr!("common-cancel")).clicked();
        });
    });
    (apply, accept, cancel)
}

/// A permission column: at least `min`, or its heading and its check box's
/// label (with the box) on one line each, as drawn (12-point text).
fn column_width(ui: &egui::Ui, p: &Palette, min: f32, heading: &str, check: &str) -> f32 {
    let width = |t: &str| {
        ui.painter()
            .layout_no_wrap(t.to_owned(), egui::FontId::proportional(12.0), p.ink)
            .size()
            .x
    };
    min.max(width(heading) + 6.0).max(width(check) + 30.0)
}

impl BuildTool {
    pub(crate) fn start_bulk_permissions(&mut self, world: &World) {
        if self.ui.bulk.job.is_some() || self.ui.bulk.deep.is_some() {
            return;
        }
        if self.ui.bulk.options.nested {
            let prims = self
                .sel_prims(world)
                .into_iter()
                .filter(|i| self.contents_editable(world, *i))
                .filter_map(|i| world.objects.get(i).map(|o| o.full_id))
                .collect();
            log::info!("deep permissions: started");
            self.ui.bulk.deep = Some(crate::build::deep_perms::Job::new(
                self.ui.bulk.options.clone(),
                prims,
                Instant::now(),
            ));
            self.ui.bulk.status = aurora_i18n::tr!("content-perms-starting").into();
            return;
        }
        let targets = self
            .sel_prims(world)
            .into_iter()
            .filter(|i| self.contents_editable(world, *i))
            .filter_map(|i| world.objects.get(i).map(|o| o.full_id))
            .collect();
        self.ui.bulk.job = Some(Job {
            options: self.ui.bulk.options.clone(),
            targets,
            failed: HashSet::new(),
            requested: None,
            updated: 0,
            skipped: 0,
        });
        self.ui.bulk.status = aurora_i18n::tr!("content-perms-starting").into();
        self.advance_bulk_permissions(world, Instant::now());
    }

    /// What "Include objects inside objects" would open, from the selected
    /// prims' listings (asked for when missing): their own object items.
    fn nested_preview(&mut self, world: &World) -> String {
        let mut objects = 0;
        let mut skipped: Vec<(String, crate::build::deep_perms::Skip)> = Vec::new();
        let mut reading = false;
        for idx in self.sel_prims(world) {
            if !self.contents_editable(world, idx) {
                continue;
            }
            let Some(id) = world.objects.get(idx).map(|o| o.full_id) else {
                continue;
            };
            match self.contents.objects.get(&id).filter(|inv| inv.requested.is_some()) {
                Some(inv) => {
                    let items: Vec<TaskItem> = inv.items.iter().filter(|i| !i.is_folder).cloned().collect();
                    let found = crate::build::deep_perms::scan(&items, world.agent_id, world.groups.active, 0);
                    objects += found.eligible.len();
                    skipped.extend(found.skipped);
                    reading |= inv.loading;
                }
                None => {
                    self.request_contents(world, idx, false);
                    reading = true;
                }
            }
        }
        if reading && objects == 0 && skipped.is_empty() {
            return aurora_i18n::tr!("content-perms-reading").into();
        }
        let mut text = aurora_i18n::tr!("content-perms-nested-preview", objects = objects, skipped = skipped.len());
        if !skipped.is_empty() {
            let list: Vec<String> = skipped
                .iter()
                .take(4)
                .map(|(name, why)| aurora_i18n::tr!("content-perms-nested-entry", name = name.as_str(), reason = why.label()))
                .collect();
            text.push(' ');
            text.push_str(&list.join(aurora_i18n::tr!("content-perms-nested-separator")));
        }
        text
    }

    /// `AURORA_DEMO_DEEP_PERMS=run…`: ticks the option and presses Apply
    /// once the selected prim's properties are known.
    fn demo_deep_start(&mut self, world: &World) -> bool {
        if self.ui.bulk.demo_started || !crate::build::deep_perms::demo::run() {
            return false;
        }
        let prims = self.sel_prims(world);
        if prims.is_empty() || !prims.iter().all(|i| self.contents_editable(world, *i)) {
            return false;
        }
        self.ui.bulk.demo_started = true;
        self.ui.bulk.options.nested = true;
        log::info!("deep permissions: demo run starts");
        true
    }

    pub(crate) fn bulk_inventory_error(&mut self, object: Uuid) {
        if let Some(job) = self.ui.bulk.job.as_mut() {
            job.failed.insert(object);
        }
    }

    /// Fetch each selected object's complete inventory, then send updates. The
    /// pinned selection and options survive closing the floater with OK.
    pub(crate) fn advance_bulk_permissions(&mut self, world: &World, now: Instant) {
        let Some(mut job) = self.ui.bulk.job.take() else { return };
        let Some(id) = job.targets.front().copied() else {
            self.ui.bulk.status = aurora_i18n::tr!("content-perms-finished", count = job.updated, skipped = job.skipped);
            self.status.clone_from(&self.ui.bulk.status);
            return;
        };
        let idx = world.objects.index_of_uuid(&id);
        if idx.is_none_or(|idx| !self.contents_editable(world, idx))
            || job.failed.remove(&id)
            || job
                .requested
                .is_some_and(|start| now.saturating_duration_since(start) > Duration::from_secs(30))
        {
            job.skipped += 1;
        } else if let Some(idx) = idx {
            if job.requested.is_none() {
                self.request_contents(world, idx, true);
                job.requested = Some(now);
                self.ui.bulk.status = aurora_i18n::tr!("content-perms-reading").into();
                self.ui.bulk.job = Some(job);
                return;
            }
            let inv = self.contents.objects.get(&id);
            if inv.is_none_or(|inv| inv.loading || inv.requested.is_none()) {
                self.ui.bulk.job = Some(job);
                return;
            }
            let items: Vec<_> = inv
                .into_iter()
                .flat_map(|inv| &inv.items)
                .filter(|it| job.options.includes(it))
                .cloned()
                .collect();
            if let Some(o) = world.objects.get(idx) {
                let (handle, local_id) = (o.key.region, o.key.local_id);
                for mut item in items {
                    job.options.apply(&mut item);
                    self.send(BuildCmd::UpdateTaskInventory {
                        handle,
                        local_id,
                        item: Box::new(item),
                    });
                    job.updated += 1;
                }
                self.contents_changed(id);
            }
        }
        job.targets.pop_front();
        job.requested = None;
        self.ui.bulk.job = Some(job);
    }
}
