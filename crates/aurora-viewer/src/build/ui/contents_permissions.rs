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
        }
    }
}

impl BulkPerms {
    fn apply(&self, item: &mut TaskItem) {
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
    }

    pub(crate) fn includes_type(&self, asset_type: i32) -> bool {
        TYPES
            .iter()
            .position(|t| i32::from(*t) == asset_type)
            .is_some_and(|i| self.types[i])
    }

    fn includes(&self, item: &TaskItem) -> bool {
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
    status: String,
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
    let state = &mut tool.ui.bulk;
    state.original.get_or_insert_with(|| state.options.clone());
    let mut open = true;
    let mut apply = false;
    let mut accept = false;
    let mut cancel = false;
    let running = state.job.is_some();
    let b = &mut state.options;
    Floater::new(
        "bulk_perms",
        aurora_i18n::tr!("build-ui-floaters-content-permissions"),
        ctx.content_rect().center() - Vec2::new(180.0, 177.0),
        Vec2::new(360.0, 354.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        (apply, accept, cancel) = controls(ui, p, b, &state.status, running, aurora_i18n::tr!("content-perms-adjust"));
    });
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
pub(crate) fn controls(
    ui: &mut egui::Ui,
    p: &Palette,
    b: &mut BulkPerms,
    status: &str,
    running: bool,
    heading: &str,
) -> (bool, bool, bool) {
    let (mut apply, mut accept, mut cancel) = (false, false, false);
    ui.horizontal_top(|ui| {
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
        ui.vertical(|ui| {
            if ui
                .add_sized([98.0, 25.0], egui::Button::new(aurora_i18n::tr!("common-all")))
                .clicked()
            {
                b.types.fill(true);
            }
            if ui
                .add_sized([98.0, 25.0], egui::Button::new(aurora_i18n::tr!("common-clear")))
                .clicked()
            {
                b.types.fill(false);
            }
        });
    });
    section(ui, p, heading);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(82.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(82.0);
            label(ui, p, aurora_i18n::tr!("common-group-2"));
            if let Some(v) = check(ui, p, b.share, false, aurora_i18n::tr!("common-share"), true) {
                b.share = v;
            }
        });
        ui.allocate_ui_with_layout(Vec2::new(90.0, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(90.0);
            label(ui, p, aurora_i18n::tr!("content-perms-anyone"));
            if let Some(v) = check(ui, p, b.everyone_copy, false, aurora_i18n::tr!("common-copy-2"), true) {
                b.everyone_copy = v;
            }
        });
        ui.vertical(|ui| {
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
    if !status.is_empty() {
        small(ui, p, status);
    }
    // the buttons at the bottom, from the height they measure (a
    // guessed 28 points grew the window at every frame with a larger
    // text)
    widgets::above_footer(ui).1.show(ui, |ui| {
        ui.horizontal(|ui| {
            let enabled = !running && b.types.iter().any(|on| *on);
            accept = ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-ok"))).clicked();
            apply = ui
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("common-apply")))
                .clicked();
            cancel = ui.button(aurora_i18n::tr!("common-cancel")).clicked();
        });
    });
    (apply, accept, cancel)
}

impl BuildTool {
    fn start_bulk_permissions(&mut self, world: &World) {
        if self.ui.bulk.job.is_some() {
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
