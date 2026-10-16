//! "Objects" (LLPanelLandObjects): prim counts, returns, auto-return delay
//! and the object owners list (ParcelObjectOwnersRequest).

use super::dialogs::Confirm;
use super::{Dialog, LandUi, View, button, row, text};
use crate::theme::Palette;
use crate::world::World;
use crate::world::land::{OwnerList, PrimUsage, powers};
use aurora_net::land::{LandCommand, ObjectOwner, RT_GROUP, RT_LIST, RT_OTHER, RT_OWNER};
use egui::{RichText, Vec2};

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World) {
    let Some(parcel) = v.parcel.clone() else {
        super::no_selection(ui, p);
        return;
    };
    // the figures of the header's gauge
    let usage = PrimUsage::of(&parcel, v.region.as_ref().map_or(0, |r| r.max_tasks));
    let (sw_total, sw_max) = (usage.used, usage.max);
    let mut max = (parcel.max_prims as f32 * parcel.prim_bonus).round() as i32;
    // never more than the region's object capacity, whatever the bonus
    if let Some(r) = v.region.as_ref().filter(|r| r.max_tasks > 0) {
        max = max.min(r.max_tasks as i32);
    }
    if parcel.prim_bonus != 1.0 {
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "ui-land-objects-object-bonus-factor",
                value0 = aurora_i18n::number_text(&format!("{:.2}", parcel.prim_bonus))
            ))
            .size(12.0)
            .color(p.ink),
        );
    }
    let wide = 210.0;
    let line = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            super::fixed(ui, wide, |ui| {
                ui.label(RichText::new(label).size(12.0).color(p.muted));
            });
            text(ui, p, value);
        });
    };
    line(
        ui,
        aurora_i18n::tr!("ui-land-objects-region-capacity"),
        if sw_total > sw_max {
            aurora_i18n::tr!(
                "ui-land-objects-of-will-be-removed",
                sw_total = format!("{}", sw_total),
                sw_max = format!("{}", sw_max),
                value0 = format!("{}", sw_total - sw_max)
            )
        } else {
            aurora_i18n::tr!(
                "status-sw-total-of-sw-max-available",
                sw_total = format!("{}", sw_total),
                sw_max = format!("{}", sw_max),
                value0 = format!("{}", sw_max - sw_total)
            )
        },
    );
    line(ui, aurora_i18n::tr!("ui-land-objects-parcel-capacity"), max.to_string());
    line(
        ui,
        aurora_i18n::tr!("ui-land-objects-parcel-impact"),
        parcel.total_prims.to_string(),
    );

    let can_owned = v.can(powers::LAND_RETURN_GROUP_OWNED);
    let can_group = v.can(powers::LAND_RETURN_GROUP_SET);
    let can_other = v.can(powers::LAND_RETURN_NON_GROUP);
    let any = can_owned || can_group || can_other;
    for (label, count, can, confirm, kind) in [
        (
            aurora_i18n::tr!("ui-land-objects-owned-by-parcel-owner"),
            parcel.owner_prims,
            can_owned,
            Confirm::ReturnOwner,
            RT_OWNER,
        ),
        (
            aurora_i18n::tr!("ui-land-objects-deeded-to-group"),
            parcel.group_prims,
            can_group,
            Confirm::ReturnGroup,
            RT_GROUP,
        ),
        (
            aurora_i18n::tr!("ui-land-objects-owned-by-others"),
            parcel.other_prims,
            can_other,
            Confirm::ReturnOther,
            RT_OTHER,
        ),
    ] {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            super::fixed(ui, wide - 26.0, |ui| {
                ui.label(RichText::new(label).size(12.0).color(p.muted));
            });
            super::fixed(ui, 60.0, |ui| {
                text(ui, p, count.to_string());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let enabled = any && can && count > 0;
                if button(ui, p, aurora_i18n::tr!("common-back"), enabled)
                    .on_hover_text(aurora_i18n::tr!("ui-land-objects-return-objects-to-their-owners"))
                    .clicked()
                {
                    s.dialog = Some(Dialog::Confirm(confirm));
                }
                // onClickShow*Objects: the simulator highlights them
                if button(ui, p, aurora_i18n::tr!("common-show"), enabled).clicked() {
                    world.land.show_objects(kind, Vec::new());
                }
            });
        });
    }
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        super::fixed(ui, wide - 26.0, |ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("ui-land-objects-selected-sat-upon"))
                    .size(12.0)
                    .color(p.muted),
            );
        });
        text(ui, p, parcel.selected_prims.to_string());
    });
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "ui-land-objects-auto-return-other-residents-objects-minutes-0-to-disable"
            ))
            .size(12.0)
            .color(p.muted),
        );
        s.clean_time.sync(&parcel.other_clean_time.to_string());
        let r = ui.add_enabled(any, egui::TextEdit::singleline(&mut s.clean_time.text).desired_width(46.0));
        // validateNonNegativeS32
        s.clean_time.text.retain(|c| c.is_ascii_digit());
        if let Some(t) = s.clean_time.after(&r) {
            // onCommitClean: only a changed value is sent
            let minutes: i32 = t.parse().unwrap_or(0);
            if minutes != parcel.other_clean_time {
                world
                    .land
                    .parcel_command(|handle, local_id| LandCommand::SetOtherCleanTime { handle, local_id, minutes });
                world.land.update(|_| {});
            }
        }
    });
    ui.add_space(2.0);
    let owners = world.land.sel.as_ref().map(|s| s.owners.clone()).unwrap_or_default();
    let selected = match &owners {
        OwnerList::Rows(rows) => s.owner_sel.and_then(|id| rows.iter().find(|r| r.id == id).cloned()),
        _ => None,
    };
    row(ui, p, aurora_i18n::tr!("ui-land-objects-owners"), |ui| {
        let searching = matches!(owners, OwnerList::Searching);
        if super::icon_button(ui, p, "refresh", any && !searching)
            .on_hover_text(aurora_i18n::tr!("ui-land-objects-refresh-object-list"))
            .clicked()
        {
            s.owner_sel = None;
            world.land.refresh_owners();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if button(ui, p, aurora_i18n::tr!("ui-land-objects-object-return"), any && selected.is_some()).clicked()
                && let Some(o) = &selected
            {
                s.dialog = Some(Dialog::Confirm(Confirm::ReturnList(o.clone())));
            }
        });
    });
    owners_list(ui, p, s, world, &owners, v.now);
}

/// sortByColumnIndex(count, descending).
const DEFAULT_SORT: (usize, bool) = (2, false);

/// The owners list: Type | Name | Count | Newest, sorted by count first.
fn owners_list(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, world: &mut World, owners: &OwnerList, _now: i64) {
    let cols: [(&str, f32); 4] = [
        (aurora_i18n::tr!("common-type"), 46.0),
        (aurora_i18n::tr!("common-name"), 0.0),
        (aurora_i18n::tr!("ui-land-objects-count"), 60.0),
        (aurora_i18n::tr!("ui-land-objects-newest"), 150.0),
    ];
    let width = ui.available_width();
    let name_w = width - cols.iter().map(|c| c.1).sum::<f32>() - 12.0;
    // header: click to sort (LLScrollListCtrl columns)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, (label, w)) in cols.iter().enumerate() {
            let w = if *w == 0.0 { name_w } else { *w };
            let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 20.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 0.0, p.field);
            let (sort_col, sort_asc) = s.owner_sort.unwrap_or(DEFAULT_SORT);
            let g = ui.painter().text(
                rect.left_center() + Vec2::new(6.0, 0.0),
                egui::Align2::LEFT_CENTER,
                *label,
                egui::FontId::proportional(12.0),
                p.muted,
            );
            if sort_col == i {
                // sort arrow, drawn (the fonts have no triangles)
                let c = egui::pos2(g.right() + 8.0, rect.center().y);
                let (a, b) = if sort_asc { (-3.0, 2.0) } else { (3.0, -2.0) };
                let pts = vec![c + Vec2::new(0.0, a), c + Vec2::new(-3.5, b), c + Vec2::new(3.5, b)];
                ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
            }
            if r.clicked() {
                s.owner_sort = Some(if sort_col == i { (i, !sort_asc) } else { (i, true) });
            }
        }
    });
    let list_h = ui.available_height().max(60.0);
    egui::Frame::new().fill(p.field).show(ui, |ui| {
        ui.set_width(width);
        ui.set_min_height(list_h);
        egui::ScrollArea::vertical()
            .id_salt("land_owners")
            .max_height(list_h)
            .auto_shrink([false, false])
            .show(ui, |ui| match owners {
                OwnerList::Idle => {}
                OwnerList::Searching => {
                    ui.label(RichText::new(aurora_i18n::tr!("common-search")).size(12.0).color(p.muted));
                }
                OwnerList::Rows(rows) if rows.is_empty() => {
                    ui.label(RichText::new(aurora_i18n::tr!("common-no-results")).size(12.0).color(p.muted));
                }
                OwnerList::Rows(rows) => {
                    let mut rows: Vec<(ObjectOwner, String)> = rows
                        .iter()
                        .map(|o| {
                            let name = if o.is_group {
                                let groups = world.groups.groups.clone();
                                world
                                    .land
                                    .group_name(&o.id, &groups)
                                    .unwrap_or_else(|| aurora_i18n::tr!("status-loading-4").into())
                            } else {
                                world.social.want_name(o.id);
                                world.social.name_of(&o.id)
                            };
                            (o.clone(), name)
                        })
                        .collect();
                    let (col, asc) = s.owner_sort.unwrap_or(DEFAULT_SORT);
                    rows.sort_by(|a, b| {
                        let o = match col {
                            0 => a.0.is_group.cmp(&b.0.is_group),
                            1 => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
                            2 => a.0.count.cmp(&b.0.count),
                            _ => a.0.most_recent.cmp(&b.0.most_recent),
                        };
                        if asc { o } else { o.reverse() }
                    });
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (o, name) in rows {
                        let online = online(world, &o);
                        let sel = s.owner_sel == Some(o.id);
                        let (rect, r) = ui.allocate_exact_size(Vec2::new(width - 12.0, 20.0), egui::Sense::click());
                        let fill = if sel {
                            p.violet.gamma_multiply(0.6)
                        } else if r.hovered() {
                            p.raised
                        } else {
                            p.field
                        };
                        ui.painter().rect_filled(rect, 0.0, fill);
                        let mut x = rect.left() + 6.0;
                        let icon = if o.is_group { "users-group" } else { "user" };
                        let tint = if o.is_group || online { p.ink } else { p.muted_dim };
                        if let Some(t) = super::super::icons::global(icon) {
                            let ir = egui::Rect::from_center_size(egui::pos2(x + 8.0, rect.center().y), Vec2::splat(14.0));
                            ui.painter().image(
                                t.id(),
                                ir,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                tint,
                            );
                        }
                        if !o.is_group {
                            // FIRE-1292: in our region (or us) = online
                            let dot = super::super::widgets::online_color(p, online);
                            ui.painter().circle_filled(egui::pos2(x + 22.0, rect.center().y), 3.0, dot);
                        }
                        x += cols[0].1;
                        let mut job = egui::text::LayoutJob::simple_singleline(name, egui::FontId::proportional(12.0), p.ink);
                        job.wrap = egui::text::TextWrapping::truncate_at_width(name_w - 8.0);
                        let g = ui.painter().layout_job(job);
                        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
                        ui.painter().galley(at, g, p.ink);
                        x += name_w;
                        ui.painter().text(
                            egui::pos2(x, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            o.count.to_string(),
                            egui::FontId::proportional(12.0),
                            p.ink,
                        );
                        x += cols[2].1;
                        if o.most_recent > 0 {
                            ui.painter().text(
                                egui::pos2(x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                super::claim_date(o.most_recent as i64),
                                egui::FontId::proportional(11.5),
                                p.muted,
                            );
                        }
                        if r.clicked() {
                            s.owner_sel = Some(o.id);
                            // onCommitList: highlight this owner's objects
                            world.land.show_objects(RT_LIST, vec![o.id]);
                        }
                        if r.double_clicked() && !o.is_group {
                            // onDoubleClickOwner: the profile (groups have no window yet)
                            super::super::profile::request_open(ui.ctx(), o.id);
                        }
                    }
                }
            });
    });
}

/// FIRE-1292: the server no longer says who is online; owners in our
/// region are shown online (and ourselves).
fn online(world: &World, o: &ObjectOwner) -> bool {
    if o.id == world.agent_id {
        return true;
    }
    world
        .main_region
        .and_then(|h| world.coarse.get(&h))
        .is_some_and(|list| list.iter().any(|(id, _)| *id == o.id))
}
