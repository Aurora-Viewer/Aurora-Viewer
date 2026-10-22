//! The Top scripts / Top colliders window of the Region / Estate window
//! (LLFloaterTopObjects, newview/llfloatertopobjects.cpp,
//! floater_top_objects.xml, originally LGPL 2.1): the simulator's report of
//! the objects that take the most script time (with their memory and URLs)
//! or that collide the most, asked by the Debug panel's estate message
//! (`scripts` "6", `colliders` "1") and refreshed or filtered by
//! LandStatRequest; the selected objects or all of them returned
//! (ParcelReturnObjects for the whole region), a beacon on one, a teleport
//! to it, and for a resident in the colliders their profile and the
//! estate's kick. The Legacy Viewer's "Script info" waits for the LSL
//! bridge.

use super::interventions::Dialog;
use super::{RegionUi, View};
use crate::theme::Palette;
use crate::ui::sheet::semibold;
use crate::ui::widgets::{self, ActionKind, DialogButton, Floater};
use crate::world::World;
use aurora_net::estate::{EstateCommand, TopObject, land_stat};
use egui::{RichText, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// Id of the window (its place and size are kept by the layout).
pub const WINDOW: &str = "region_top_objects";

/// The columns, in the floater's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    Score,
    Name,
    Owner,
    Location,
    Parcel,
    Time,
    Memory,
    Urls,
}

/// The filters of the floater's three "Filter" buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    Object,
    Owner,
    Parcel,
}

impl Filter {
    /// STAT_FILTER_BY_OBJECT, _OWNER, _PARCEL_NAME (onGetBy…).
    pub fn flag(self) -> u32 {
        match self {
            Filter::Object => land_stat::FILTER_BY_OBJECT,
            Filter::Owner => land_stat::FILTER_BY_OWNER,
            Filter::Parcel => land_stat::FILTER_BY_PARCEL_NAME,
        }
    }
}

/// What the window keeps between frames.
#[derive(Default)]
pub(super) struct TopState {
    pub open: bool,
    /// The report shown or asked: TOP_SCRIPTS or TOP_COLLIDERS.
    pub report: u32,
    text: String,
    by: Filter,
    selected: HashSet<Uuid>,
    /// (column, descending)
    sort: Option<(Column, bool)>,
    ask_return_all: bool,
    /// AURORA_DEMO_REGION's report, opened once the simulator answered.
    pub demo: Option<u32>,
}

impl TopState {
    /// Closed, or on another region; a demo's report waits for its region.
    pub fn reset(&mut self) {
        *self = TopState {
            demo: self.demo.take(),
            ..Default::default()
        };
    }
}

/// The window opens on `report`, asked as the Debug panel asks it.
pub(super) fn open(s: &mut RegionUi, v: &View, world: &mut World, report: u32) {
    s.top.open = true;
    s.top.report = report;
    s.top.selected.clear();
    s.top.sort = None;
    world.estate.ask_top(EstateCommand::TopObjects { handle: v.handle, report });
}

/// onRefresh: the same report again, with the filter when one is typed.
fn refresh(s: &mut TopState, v: &View, world: &mut World, filtered: bool) {
    s.selected.clear();
    let text = s.text.trim().to_owned();
    let (flags, filter) = if filtered && !text.is_empty() {
        (s.by.flag(), text)
    } else {
        (0, String::new())
    };
    world.estate.ask_top(EstateCommand::LandStat {
        handle: v.handle,
        report: s.report,
        flags,
        filter,
    });
}

/// The rows in the chosen order (the simulator's, highest score first, by
/// default).
pub(crate) fn sorted(objects: &[TopObject], sort: Option<(Column, bool)>) -> Vec<&TopObject> {
    let mut rows: Vec<&TopObject> = objects.iter().collect();
    if let Some((col, desc)) = sort {
        rows.sort_by(|a, b| {
            let o = match col {
                Column::Score => a.score.total_cmp(&b.score),
                Column::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                Column::Owner => a.owner.cmp(&b.owner),
                Column::Location => a.position[0]
                    .total_cmp(&b.position[0])
                    .then(a.position[1].total_cmp(&b.position[1])),
                Column::Parcel => a.parcel.to_lowercase().cmp(&b.parcel.to_lowercase()),
                Column::Time => a.time.cmp(&b.time),
                Column::Memory => a.memory.total_cmp(&b.memory),
                Column::Urls => a.urls.cmp(&b.urls),
            };
            if desc { o.reverse() } else { o }
        });
    }
    rows
}

/// "<128, 64, 22>", as the floater writes a location.
pub(crate) fn location_text(p: [f32; 3]) -> String {
    format!("<{:.0}, {:.0}, {:.0}>", p[0], p[1], p[2])
}

/// The window, while open.
pub(super) fn show(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    if world.estate.region.is_some()
        && let Some(report) = s.top.demo.take()
    {
        open(s, v, world, report);
    }
    if !s.top.open {
        return;
    }
    let report = world.estate.top.as_ref().filter(|(h, _)| *h == v.handle).map(|(_, r)| r.clone());
    // the report says which kind it is (handleReply's mCurrentMode)
    if let Some(r) = &report {
        s.top.report = r.report;
    }
    let scripts = s.top.report != land_stat::TOP_COLLIDERS;
    let title = if scripts {
        aurora_i18n::tr!("region-top-scripts")
    } else {
        aurora_i18n::tr!("region-top-colliders")
    };
    let screen = ctx.content_rect();
    let size = Vec2::new(900.0, 540.0);
    let mut open = true;
    let mut floater = Floater::new(WINDOW, title, screen.center() - size * 0.5, size);
    floater.min_size = Vec2::new(620.0, 360.0);
    let mut kick = None;
    floater.show(ctx, p, &mut open, |ui| {
        let waiting = world.estate.top_waiting;
        // which report
        ui.horizontal(|ui| {
            let options = [
                (aurora_i18n::tr!("region-top-scripts"), true),
                (aurora_i18n::tr!("region-top-colliders"), true),
            ];
            let mut choice = usize::from(!scripts);
            if widgets::segmented(ui, p, &mut choice, &options) {
                let report = if choice == 1 {
                    land_stat::TOP_COLLIDERS
                } else {
                    land_stat::TOP_SCRIPTS
                };
                open_report(s, v, world, report);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let refresh_label = aurora_i18n::tr!("region-top-refresh");
                if widgets::action_button_if(ui, p, "refresh", refresh_label, ActionKind::Flat, Vec2::new(0.0, 26.0), !waiting).clicked() {
                    refresh(&mut s.top, v, world, false);
                }
            });
        });
        ui.add_space(6.0);
        // the summary (top_scripts_text / top_colliders_text)
        let summary = match &report {
            None if waiting => aurora_i18n::tr!("common-loading").to_owned(),
            None => String::new(),
            Some(r) if scripts => {
                let (time, memory) = r.totals();
                aurora_i18n::tr!(
                    "region-top-scripts-text",
                    count = i64::from(r.total),
                    time = aurora_i18n::number(f64::from(time), 3),
                    memory = aurora_i18n::integer(memory as i64)
                )
            }
            Some(r) => aurora_i18n::tr!("region-top-colliders-text", count = i64::from(r.total)),
        };
        ui.label(RichText::new(summary).size(12.5).color(p.muted));
        ui.add_space(6.0);
        // the filter
        ui.horizontal(|ui| {
            let options = [
                (aurora_i18n::tr!("region-top-by-object"), true),
                (aurora_i18n::tr!("region-top-by-owner"), true),
                (aurora_i18n::tr!("region-top-by-parcel"), true),
            ];
            let mut by = match s.top.by {
                Filter::Object => 0,
                Filter::Owner => 1,
                Filter::Parcel => 2,
            };
            if widgets::segmented(ui, p, &mut by, &options) {
                s.top.by = [Filter::Object, Filter::Owner, Filter::Parcel][by];
            }
            // the button first, from the right: the box takes what it leaves
            // (a guessed button width widened the window at every frame)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let filter_label = aurora_i18n::tr!("region-top-filter");
                let filter = widgets::action_button_if(
                    ui,
                    p,
                    "filter-filled",
                    filter_label,
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                    !waiting,
                );
                let w = ui.available_width().max(140.0);
                let r = widgets::search_box(ui, p, &mut s.top.text, aurora_i18n::tr!("region-top-filter-hint"), w);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if filter.clicked() || enter && !waiting {
                    refresh(&mut s.top, v, world, true);
                }
            });
        });
        ui.add_space(8.0);
        let rows = report.as_ref().map(|r| sorted(&r.objects, s.top.sort)).unwrap_or_default();
        let (mut list, foot) = widgets::above_footer(ui);
        table(&mut list, p, &mut s.top, &rows, scripts, waiting);
        foot.show(ui, |ui| {
            ui.add_space(8.0);
            // the actions
            let one = (s.top.selected.len() == 1)
                .then(|| rows.iter().find(|o| s.top.selected.contains(&o.task_id)).copied())
                .flatten();
            // a resident among the colliders: on the region's map
            let here = |id: &Uuid| world.coarse.get(&v.handle).is_some_and(|l| l.iter().any(|(a, _)| a == id));
            let avatar = one.filter(|o| here(&o.task_id)).map(|o| o.task_id);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let size = Vec2::new(0.0, 28.0);
                if widgets::action_button_if(
                    ui,
                    p,
                    "current-location-filled",
                    aurora_i18n::tr!("region-top-beacon"),
                    ActionKind::Flat,
                    size,
                    one.is_some(),
                )
                .clicked()
                    && let Some(o) = one
                {
                    beacon(world, v, o, false);
                }
                if widgets::action_button_if(
                    ui,
                    p,
                    "send-filled",
                    aurora_i18n::tr!("common-teleport"),
                    ActionKind::Flat,
                    size,
                    one.is_some(),
                )
                .clicked()
                    && let Some(o) = one
                {
                    beacon(world, v, o, true);
                }
                if widgets::action_button_if(
                    ui,
                    p,
                    "user-filled",
                    aurora_i18n::tr!("common-profile"),
                    ActionKind::Flat,
                    size,
                    avatar.is_some(),
                )
                .clicked()
                    && let Some(id) = avatar
                {
                    crate::ui::profile::request_open(ui.ctx(), id);
                }
                let kick_label = aurora_i18n::tr!("region-int-kick");
                if widgets::action_button_if(
                    ui,
                    p,
                    "door-exit",
                    kick_label,
                    ActionKind::Flat,
                    size,
                    avatar.is_some() && v.manages(),
                )
                .clicked()
                {
                    kick = avatar;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let can = v.manages() && !rows.is_empty();
                    if widgets::action_button_if(
                        ui,
                        p,
                        "",
                        aurora_i18n::tr!("region-top-return-all"),
                        ActionKind::SoftDanger,
                        size,
                        can,
                    )
                    .clicked()
                    {
                        s.top.ask_return_all = true;
                    }
                    let some = can && !s.top.selected.is_empty();
                    if widgets::action_button_if(
                        ui,
                        p,
                        "arrow-back-up",
                        aurora_i18n::tr!("region-top-return-selected"),
                        ActionKind::Flat,
                        size,
                        some,
                    )
                    .clicked()
                    {
                        let tasks: Vec<Uuid> = rows
                            .iter()
                            .filter(|o| s.top.selected.contains(&o.task_id))
                            .map(|o| o.task_id)
                            .collect();
                        world.estate.intervene(EstateCommand::ReturnTopObjects { handle: v.handle, tasks });
                        s.top.selected.clear();
                    }
                });
            });
        });
    });
    if let Some(target) = kick {
        s.interventions.dialog = Some(Dialog::Kick { target, ban: false });
    }
    if !open {
        s.top.open = false;
    }
    if s.top.ask_return_all {
        return_all(ctx, p, s, v, world);
    }
}

/// Another report from the window's switch: the Debug panel's message.
fn open_report(s: &mut RegionUi, v: &View, world: &mut World, report: u32) {
    s.top.report = report;
    s.top.selected.clear();
    s.top.sort = None;
    world.estate.ask_top(EstateCommand::TopObjects { handle: v.handle, report });
}

/// showBeacon / onTeleportToObject: the object's place in the region, as
/// a tracked location or a teleport.
fn beacon(world: &mut World, v: &View, o: &TopObject, teleport: bool) {
    let (x, y) = aurora_net::handle_to_origin(v.handle);
    let gx = f64::from(x) + f64::from(o.position[0]);
    let gy = f64::from(y) + f64::from(o.position[1]);
    world.map.track_location(gx, gy, o.position[2], teleport);
}

/// The list: a head that sorts, a row per object with a box to select it.
fn table(ui: &mut egui::Ui, p: &Palette, s: &mut TopState, rows: &[&TopObject], scripts: bool, waiting: bool) {
    let mut columns: Vec<(Column, &str, f32)> = vec![
        (
            Column::Score,
            if scripts {
                aurora_i18n::tr!("region-top-time")
            } else {
                aurora_i18n::tr!("region-top-score")
            },
            80.0,
        ),
        (Column::Name, aurora_i18n::tr!("common-name"), 0.0),
        (Column::Owner, aurora_i18n::tr!("common-owner"), 150.0),
        (Column::Location, aurora_i18n::tr!("region-top-location"), 120.0),
        (Column::Parcel, aurora_i18n::tr!("region-top-parcel"), 130.0),
        (Column::Time, aurora_i18n::tr!("region-top-date"), 90.0),
    ];
    if scripts {
        columns.push((Column::Memory, aurora_i18n::tr!("region-top-memory"), 90.0));
        columns.push((Column::Urls, aurora_i18n::tr!("region-top-urls"), 50.0));
    }
    const CHECK: f32 = 26.0;
    let fixed: f32 = columns.iter().map(|c| c.2).sum::<f32>() + CHECK;
    let full = ui.available_width();
    let name_w = (full - fixed).max(120.0);
    let width_of = |c: &(Column, &str, f32)| if c.0 == Column::Name { name_w } else { c.2 };
    // head
    let (head, _) = ui.allocate_exact_size(Vec2::new(full, 22.0), Sense::hover());
    let mut x = head.left() + CHECK;
    for c in &columns {
        let w = width_of(c);
        let rect = egui::Rect::from_min_size(egui::pos2(x, head.top()), Vec2::new(w, head.height()));
        let r = ui.interact(rect, ui.scope_id().with(("top_head", c.0 as u8)), Sense::click());
        let lit = s.sort.is_some_and(|(col, _)| col == c.0);
        let mut text = c.1.to_owned();
        if let Some((col, desc)) = s.sort
            && col == c.0
        {
            text.push_str(if desc { " ▾" } else { " ▴" });
        }
        let galley = ui
            .painter()
            .layout_job(semibold(ui, &text, 11.5, if lit { p.ink } else { p.muted }));
        let at = widgets::row_text_origin(ui.painter(), &galley, rect.left() + 4.0, rect.center().y);
        ui.painter().galley(at, galley, p.muted);
        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            s.sort = match s.sort {
                Some((col, desc)) if col == c.0 => Some((col, !desc)),
                // numbers start with the highest
                _ => Some((c.0, matches!(c.0, Column::Score | Column::Time | Column::Memory | Column::Urls))),
            };
        }
        x += w;
    }
    let height = ui.available_height();
    egui::ScrollArea::vertical()
        .id_salt("region_top_objects_list")
        .max_height(height)
        .min_scrolled_height(height)
        .auto_shrink([false, false])
        .show_rows(ui, 26.0, rows.len().max(1), |ui, range| {
            if rows.is_empty() {
                let text = if waiting {
                    aurora_i18n::tr!("common-loading")
                } else {
                    aurora_i18n::tr!("region-top-none")
                };
                ui.label(RichText::new(text).size(12.0).color(p.muted));
                return;
            }
            for o in &rows[range] {
                let (rect, r) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                let selected = s.selected.contains(&o.task_id);
                let radius = ui.visuals().widgets.inactive.corner_radius;
                if selected {
                    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
                } else if r.hovered() {
                    ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
                }
                // the box
                let bx = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), Vec2::splat(14.0));
                ui.painter()
                    .rect_stroke(bx, radius, egui::Stroke::new(1.0, p.muted), egui::StrokeKind::Inside);
                if selected {
                    crate::ui::sheet::icon_at(ui, "check", bx.center(), 12.0, p.violet_light);
                }
                let cells = [
                    format!("{:.3}", o.score),
                    o.name.clone(),
                    o.owner.clone(),
                    location_text(o.position),
                    o.parcel.clone(),
                    date_text(o.time),
                    if o.extended {
                        aurora_i18n::integer((o.memory / 1024.0).round() as i64)
                    } else {
                        String::new()
                    },
                    if o.extended { o.urls.to_string() } else { String::new() },
                ];
                let mut x = rect.left() + CHECK;
                for (c, text) in columns.iter().zip(cells.iter()) {
                    let w = width_of(c);
                    let mut job = egui::text::LayoutJob::simple_singleline(text.clone(), egui::FontId::proportional(12.0), p.ink);
                    job.wrap = egui::text::TextWrapping::truncate_at_width(w - 8.0);
                    let galley = ui.painter().layout_job(job);
                    let at = widgets::row_text_origin(ui.painter(), &galley, x + 4.0, rect.center().y);
                    ui.painter().galley(at, galley, p.ink);
                    x += w;
                }
                if r.clicked() {
                    // a click selects that row, Ctrl or the box adds to the selection
                    let toggle =
                        ui.input(|i| i.modifiers.command) || r.interact_pointer_pos().is_some_and(|pos| pos.x < rect.left() + CHECK);
                    if toggle {
                        if !s.selected.remove(&o.task_id) {
                            s.selected.insert(o.task_id);
                        }
                    } else {
                        s.selected.clear();
                        s.selected.insert(o.task_id);
                    }
                }
            }
        });
}

/// "2026-10-17", or nothing for an object without a date.
pub(crate) fn date_text(time: u32) -> String {
    if time == 0 {
        return String::new();
    }
    let (y, m, d, ..) = crate::ui::land::slt_parts(i64::from(time));
    format!("{y:04}-{m:02}-{d:02}")
}

/// ReturnAllTopObjects: every listed object back to its owner.
fn return_all(ctx: &egui::Context, p: &Palette, s: &mut RegionUi, v: &View, world: &mut World) {
    let mut answer = None;
    let modal = widgets::Modal::new(egui::Id::unique("region_top_return_all")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::glyph(ui, "arrow-back-up", p.danger, 16.0);
            ui.label(semibold(ui, aurora_i18n::tr!("region-top-return-all-title"), 13.5, p.ink));
        });
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                RichText::new(aurora_i18n::tr!("region-top-return-all-text"))
                    .size(12.5)
                    .color(p.ink),
            )
            .wrap(),
        );
        ui.add_space(14.0);
        let buttons = [
            DialogButton {
                icon: "",
                label: aurora_i18n::tr!("common-cancel"),
                kind: ActionKind::Flat,
                enabled: true,
            },
            DialogButton {
                icon: "",
                label: aurora_i18n::tr!("region-top-return-all"),
                kind: ActionKind::Danger,
                enabled: true,
            },
        ];
        if let Some(i) = widgets::dialog_buttons(ui, p, &buttons, 28.0) {
            answer = Some(i == 1);
        }
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    let Some(ok) = answer else {
        return;
    };
    s.top.ask_return_all = false;
    if ok && let Some((_, r)) = world.estate.top.as_ref().filter(|(h, _)| *h == v.handle) {
        let tasks: Vec<Uuid> = r.objects.iter().map(|o| o.task_id).collect();
        world.estate.intervene(EstateCommand::ReturnTopObjects { handle: v.handle, tasks });
        s.top.selected.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(n: u128, score: f32, name: &str) -> TopObject {
        TopObject {
            task_id: Uuid::from_u128(n),
            score,
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_rows_keep_the_simulator_s_order_until_a_column_is_chosen() {
        let list = [obj(1, 0.5, "b"), obj(2, 2.0, "A"), obj(3, 1.0, "c")];
        let ids = |rows: Vec<&TopObject>| rows.iter().map(|o| o.task_id.as_u128()).collect::<Vec<_>>();
        assert_eq!(ids(sorted(&list, None)), [1, 2, 3]);
        assert_eq!(ids(sorted(&list, Some((Column::Score, true)))), [2, 3, 1]);
        assert_eq!(ids(sorted(&list, Some((Column::Name, false)))), [2, 1, 3], "names ignore the case");
    }

    #[test]
    fn the_window_keeps_its_size() {
        use crate::world::estate::Role;
        let v = View {
            handle: aurora_net::origin_to_handle(1012 * 256, 998 * 256),
            role: Role::Owner,
            shown: Role::Owner,
            name: "Aurora".into(),
            sim_access: 13,
            flags: 0,
            owner: Uuid::from_u128(1),
            product: String::new(),
            same_region: true,
        };
        let report = std::sync::Arc::new(aurora_net::estate::TopObjects {
            report: land_stat::TOP_SCRIPTS,
            flags: 0,
            total: 40,
            objects: (0..40).map(|i| obj(i, i as f32, &format!("Object {i}"))).collect(),
        });
        widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(900.0, 540.0), Vec2::new(620.0, 360.0)],
            aurora_i18n::tr!("region-top-return-all"),
            || {
                let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
                world.estate.top = Some((v.handle, report.clone()));
                let mut s = RegionUi::default();
                s.top.open = true;
                (s, world)
            },
            |(s, world), ctx, p| show(ctx, p, s, &v, world),
        );
    }

    #[test]
    fn locations_dates_and_filters() {
        assert_eq!(location_text([128.4, 64.5, 22.0]), "<128, 64, 22>");
        assert_eq!(date_text(0), "");
        assert_eq!(Filter::Owner.flag(), land_stat::FILTER_BY_OWNER);
        assert_eq!(Filter::Parcel.flag(), land_stat::FILTER_BY_PARCEL_NAME);
    }
}
