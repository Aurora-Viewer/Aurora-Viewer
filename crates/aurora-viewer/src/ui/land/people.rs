//! "People and objects" of the Aurora About Land (`aurora.rs`): the
//! allowed and banned residents of LLPanelLandAccess and the object owners
//! of LLPanelLandObjects as rows with a round picture, a status, the time
//! left and a box to select several. The lists go out as the panels send
//! them (`Land::send_lists`, ParcelReturnObjects with the owners' IDs), the
//! CSV export / import of FS:PP and LLFloaterBanDuration are the classic
//! window's.

use super::aurora::{Pictures, cards, semibold};
use super::dialogs::Confirm;
use super::manage::{card_begin, card_end, missing_power};
use super::{Dialog, LandUi, PickFor, View};
use crate::theme::Palette;
use crate::ui::sheet::{Person, ROW_H, list_title, matches, names, person_row};
use crate::ui::widgets;
use crate::world::World;
use crate::world::land::{OwnerList, powers};
use aurora_net::land::{AL_ACCESS, AL_BAN, AccessEntry, ObjectOwner, PARCEL_MAX_ACCESS_LIST};
use egui::{Rect, RichText, Sense, Vec2};
use uuid::Uuid;

/// The time an entry has left, "4 h" or "24 min"; None for always.
pub(super) fn time_left(time: i32, now: i64) -> Option<String> {
    if time == 0 {
        return None;
    }
    let s = (i64::from(time) - now).max(0);
    Some(if s >= 3600 {
        format!("{} h", s / 3600)
    } else if s >= 60 {
        format!("{} min", s / 60)
    } else {
        format!("{s} s")
    })
}

/// "today 09:12", "yesterday", a weekday, then the date: when an owner's
/// newest object came, in SLT.
pub(super) fn when(secs: i64, now: i64) -> String {
    let (y, m, d, w, h, mi, _) = super::slt_parts(secs);
    let (ny, nm, nd, ..) = super::slt_parts(now);
    let days = (now - secs).div_euclid(86_400);
    if (y, m, d) == (ny, nm, nd) {
        aurora_i18n::tr!("ui-land-people-today", time = format!("{h:02}:{mi:02}"))
    } else if (0..2).contains(&days) && super::slt_parts(now - 86_400).2 == d {
        aurora_i18n::tr!("ui-land-people-yesterday").to_owned()
    } else if (0..7).contains(&days) {
        super::weekdays()[w].to_owned()
    } else {
        super::overview::day(secs)
    }
}

/// The page. `pics`: the residents' pictures.
pub(super) fn show(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, pics: &Pictures) {
    if v.parcel.is_none() {
        return;
    }
    if std::mem::take(&mut s.refresh_owners_once) && world.land.sel.as_ref().is_some_and(|sel| matches!(sel.owners, OwnerList::Idle)) {
        world.land.refresh_owners();
    }
    egui::ScrollArea::vertical()
        .id_salt("about_land_people")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(ui.available_width() - 6.0);
            cards(
                ui,
                p,
                world,
                s,
                1.0,
                |ui, world, s| list_card(ui, p, s, v, world, pics, AL_ACCESS),
                |ui, world, s| list_card(ui, p, s, v, world, pics, AL_BAN),
            );
            ui.add_space(12.0);
            let rect = owners_card(ui, p, s, v, world, pics);
            if std::mem::take(&mut s.scroll_to_owners) {
                ui.scroll_to_rect(rect, Some(egui::Align::TOP));
            }
        });
    super::access::poll_files(s, world);
}

fn list_card(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, pics: &Pictures, kind: u32) {
    let ban = kind == AL_BAN;
    let parcel = v.parcel.clone();
    let override_ok = parcel.as_ref().is_some_and(|p| p.region_allow_access_override);
    // the estate owner may not let parcel owners manage access
    let power = if ban {
        powers::LAND_MANAGE_BANNED
    } else {
        powers::LAND_MANAGE_ALLOWED
    };
    let can = v.can(power) && (ban || override_ok);
    let list: Vec<AccessEntry> = world
        .land
        .sel
        .as_ref()
        .map(|sel| if ban { sel.bans.clone() } else { sel.access.clone() })
        .unwrap_or_default();
    let full = list.len() >= PARCEL_MAX_ACCESS_LIST;
    // head: title, count, add
    let title = if ban {
        aurora_i18n::tr!("ui-land-people-banned")
    } else {
        aurora_i18n::tr!("ui-land-people-allowed")
    };
    let count = format!("{} / {PARCEL_MAX_ACCESS_LIST}", list.len());
    list_title(ui, p, if ban { "ban" } else { "user-check" }, title, &count, |ui| {
        let label = if ban {
            aurora_i18n::tr!("ui-land-people-ban")
        } else {
            aurora_i18n::tr!("common-add")
        };
        if widgets::action_button_if(ui, p, "plus", label, widgets::ActionKind::Flat, Vec2::new(0.0, 26.0), can && !full).clicked() {
            s.pick_for = Some(if ban { PickFor::Ban } else { PickFor::Access });
            s.picker.open("land_avatar_picker", ban);
        }
    });
    if !can && let Some(m) = missing_power(v, power) {
        ui.label(RichText::new(m).size(11.5).color(p.amber));
    }
    // search, import, export
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let filter = if ban { &mut s.ban_filter } else { &mut s.access_filter };
        let width = ui.available_width() - 2.0 * 26.0 - 12.0;
        widgets::search_box(ui, p, filter, aurora_i18n::tr!("common-search"), width);
        if widgets::icon_button(
            ui,
            p,
            "file-import",
            aurora_i18n::tr!("ui-land-access-import-a-csv-file-containing-only-valid-uuids-one"),
            can && !full,
        ) {
            s.file_job = Some(super::files::open(kind));
        }
        if widgets::icon_button(
            ui,
            p,
            "file-export",
            aurora_i18n::tr!("ui-land-access-export"),
            can && !list.is_empty(),
        ) {
            let file = if ban { "land_banned_list.csv" } else { "land_access_list.csv" };
            let csv = super::access::export_csv(&list, |id| world.social.name_of(id));
            s.file_job = Some(super::files::save(file, csv));
        }
    });
    ui.add_space(2.0);
    let filter = if ban { s.ban_filter.clone() } else { s.access_filter.clone() };
    let mut rows: Vec<(AccessEntry, String, Option<String>)> = list
        .iter()
        .map(|e| {
            let (name, user) = names(world, &e.id);
            (*e, name, user)
        })
        .filter(|(_, name, user)| matches(&filter, name, user.as_deref()))
        .collect();
    rows.sort_by_key(|r| r.1.to_lowercase());
    let selected = if ban { &mut s.ban_sel } else { &mut s.access_sel };
    selected.retain(|id| list.iter().any(|e| e.id == *id));
    let width = widgets::scrolled_width(ui);
    egui::ScrollArea::vertical()
        .id_salt(("land_people", kind))
        .max_height(ROW_H * 5.5)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if rows.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(aurora_i18n::tr!("status-empty")).size(12.0).color(p.muted_dim));
            }
            for (e, name, user) in &rows {
                let left = time_left(e.time, v.now);
                let exp = match (&left, ban) {
                    (None, _) => aurora_i18n::tr!("common-always").to_owned(),
                    (Some(l), true) => l.clone(),
                    (Some(l), false) => aurora_i18n::tr!("ui-land-people-left", left = l.clone()),
                };
                let is_sel = selected.contains(&e.id);
                let row = Person {
                    id: e.id,
                    pic: pics.avatars.get(&e.id),
                    name,
                    user: user.as_deref(),
                    exp: &exp,
                    timed: left.is_some(),
                    mid: None,
                    group: false,
                    selected: is_sel,
                    can,
                };
                let r = person_row(ui, p, world, &row, width);
                if r.toggled {
                    if is_sel {
                        selected.remove(&e.id);
                    } else {
                        selected.insert(e.id);
                    }
                }
                if r.wants_pic {
                    s.wanted_pics.insert(e.id);
                }
            }
        });
    // foot: what is selected and what can be done with it
    let selected: Vec<Uuid> = selected.iter().copied().collect();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        if selected.is_empty() {
            let hint = if ban {
                aurora_i18n::tr!("ui-land-people-banned-hint")
            } else {
                aurora_i18n::tr!("ui-land-people-allowed-hint")
            };
            ui.add(egui::Label::new(RichText::new(hint).size(11.5).color(p.muted)).wrap());
            return;
        }
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-land-people-selected", count = selected.len() as i64))
                .size(11.5)
                .color(p.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ban
                && widgets::action_button_if(
                    ui,
                    p,
                    "",
                    aurora_i18n::tr!("ui-land-people-duration"),
                    widgets::ActionKind::Flat,
                    Vec2::new(0.0, 24.0),
                    can,
                )
                .clicked()
            {
                // LLFloaterBanDuration for residents already banned
                s.dialog = Some(Dialog::BanDuration {
                    ids: selected.clone(),
                    temporary: true,
                    hours: 1,
                    replace: true,
                });
            }
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("ui-land-people-remove"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                can,
            )
            .clicked()
                && let Some(sel) = world.land.sel.as_mut()
            {
                if ban {
                    sel.bans.retain(|e| !selected.contains(&e.id));
                    s.ban_sel.clear();
                } else {
                    sel.access.retain(|e| !selected.contains(&e.id));
                    s.access_sel.clear();
                }
                sel.revision += 1;
                world.land.send_lists(kind);
            }
        });
    });
}

/// sortByColumnIndex(count, descending), as the Objects tab.
const DEFAULT_SORT: (usize, bool) = (2, false);
/// Columns of the owners' list: box, name, count, newest, actions.
const COLS: [f32; 4] = [22.0, 70.0, 110.0, 150.0];

/// Sort the owners' rows like the Objects tab's columns (1 name, 2 count,
/// 3 newest; anything else by kind).
pub(super) fn sort_owners(rows: &mut [(ObjectOwner, String)], sort: (usize, bool)) {
    let (col, asc) = sort;
    rows.sort_by(|a, b| {
        let o = match col {
            1 => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
            2 => a.0.count.cmp(&b.0.count),
            3 => a.0.most_recent.cmp(&b.0.most_recent),
            _ => a.0.is_group.cmp(&b.0.is_group),
        };
        if asc { o } else { o.reverse() }
    });
}

/// The confirmation of "Return the selection": one owner as the Objects
/// tab's button, several in one message.
pub(super) fn return_selection(selected: Vec<ObjectOwner>) -> Option<Confirm> {
    match selected.len() {
        0 => None,
        1 => selected.into_iter().next().map(Confirm::ReturnList),
        _ => Some(Confirm::ReturnOwners(selected)),
    }
}

fn owners_card(ui: &mut egui::Ui, p: &Palette, s: &mut LandUi, v: &View, world: &mut World, pics: &Pictures) -> Rect {
    let any = v.can(powers::LAND_RETURN_GROUP_OWNED) || v.can(powers::LAND_RETURN_GROUP_SET) || v.can(powers::LAND_RETURN_NON_GROUP);
    let owners = world.land.sel.as_ref().map(|s| s.owners.clone()).unwrap_or_default();
    let mut rows: Vec<(ObjectOwner, String)> = match &owners {
        OwnerList::Rows(rows) => rows
            .iter()
            .map(|o| {
                let name = if o.is_group {
                    let groups = world.groups.groups.clone();
                    world
                        .land
                        .group_name(&o.id, &groups)
                        .unwrap_or_else(|| aurora_i18n::tr!("status-loading-4").into())
                } else {
                    names(world, &o.id).0
                };
                (o.clone(), name)
            })
            .collect(),
        _ => Vec::new(),
    };
    sort_owners(&mut rows, s.owner_sort.unwrap_or(DEFAULT_SORT));
    s.owners_sel.retain(|id| rows.iter().any(|r| r.0.id == *id));
    let total: i64 = rows.iter().map(|r| i64::from(r.0.count)).sum();
    let most = rows.iter().map(|r| r.0.count).max().unwrap_or(1).max(1);
    let searching = matches!(owners, OwnerList::Searching);
    let mut c = card_begin(ui);
    let ui2 = &mut c.child;
    ui2.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 7.0;
        widgets::glyph(ui, "box", p.violet_light, 15.0);
        ui.label(semibold(ui, aurora_i18n::tr!("ui-land-people-owners"), 13.0, p.ink));
        if !rows.is_empty() {
            ui.label(semibold(
                ui,
                &aurora_i18n::tr!("ui-land-people-owners-count", objects = total, owners = rows.len() as i64),
                11.0,
                p.muted_dim,
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let chosen: Vec<ObjectOwner> = rows
                .iter()
                .filter(|r| s.owners_sel.contains(&r.0.id))
                .map(|r| r.0.clone())
                .collect();
            if widgets::action_button_if(
                ui,
                p,
                "arrow-back-up",
                aurora_i18n::tr!("ui-land-people-return-selection"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 26.0),
                any && !chosen.is_empty(),
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-objects-return-objects-to-their-owners"))
            .clicked()
                && let Some(confirm) = return_selection(chosen)
            {
                s.dialog = Some(Dialog::Confirm(confirm));
            }
            if widgets::action_button_if(
                ui,
                p,
                "refresh",
                aurora_i18n::tr!("ui-land-people-refresh"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 26.0),
                any && !searching,
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-objects-refresh-object-list"))
            .clicked()
            {
                s.owners_sel.clear();
                world.land.refresh_owners();
            }
        });
    });
    if !any && let Some(m) = missing_power(v, powers::LAND_RETURN_NON_GROUP) {
        ui2.label(RichText::new(m).size(11.5).color(p.amber));
    }
    ui2.add_space(4.0);
    let width = ui2.available_width();
    let name_w = width - COLS.iter().sum::<f32>();
    // the head: click to sort
    ui2.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (sort_col, sort_asc) = s.owner_sort.unwrap_or(DEFAULT_SORT);
        for (i, (label, w, right)) in [
            ("", COLS[0], false),
            (aurora_i18n::tr!("ui-land-people-owner-head"), name_w, false),
            (aurora_i18n::tr!("ui-land-people-objects-head"), COLS[1], true),
            (aurora_i18n::tr!("ui-land-people-newest-head"), COLS[2], false),
            ("", COLS[3], false),
        ]
        .into_iter()
        .enumerate()
        {
            let (rect, r) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::click());
            if label.is_empty() {
                continue;
            }
            const CARET: f32 = 9.0;
            let sorted = sort_col == i;
            let caret_w = if sorted { widgets::caret_room(CARET) } else { 0.0 };
            let g = ui.painter().layout_job(semibold(ui, label, 11.0, p.muted_dim));
            let x = if right {
                rect.right() - 6.0 - g.size().x - caret_w
            } else {
                rect.left() + 6.0
            };
            if sorted {
                let center = egui::pos2(x + g.size().x + caret_w - CARET / 2.0, rect.center().y);
                widgets::paint_glyph(ui.painter(), widgets::sort_caret(sort_asc), center, CARET, p.muted);
            }
            let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
            ui.painter().galley(at, g, p.muted_dim);
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                s.owner_sort = Some(if sort_col == i { (i, !sort_asc) } else { (i, true) });
            }
        }
    });
    match &owners {
        OwnerList::Idle => {
            ui2.add_space(6.0);
            ui2.label(
                RichText::new(aurora_i18n::tr!("ui-land-people-owners-idle"))
                    .size(12.0)
                    .color(p.muted),
            );
        }
        OwnerList::Searching => {
            ui2.add_space(6.0);
            ui2.label(RichText::new(aurora_i18n::tr!("common-search")).size(12.0).color(p.muted));
        }
        OwnerList::Rows(_) if rows.is_empty() => {
            ui2.add_space(6.0);
            ui2.label(RichText::new(aurora_i18n::tr!("common-no-results")).size(12.0).color(p.muted));
        }
        OwnerList::Rows(_) => {
            for (o, name) in &rows {
                line(ui2, p);
                match owner_row(ui2, p, s, v, pics, o, name, most, any, name_w) {
                    Some(OwnerAction::Return(confirm)) => s.dialog = Some(Dialog::Confirm(confirm)),
                    Some(OwnerAction::Show(id)) => world.land.show_objects(aurora_net::land::RT_LIST, vec![id]),
                    None => {}
                }
            }
        }
    }
    card_end(ui, p, c)
}

fn line(ui: &mut egui::Ui, p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(1.0, p.raised.gamma_multiply(0.3)),
    );
}

/// What a button of an owner's row asks for.
enum OwnerAction {
    Return(Confirm),
    /// Highlight their objects.
    Show(Uuid),
}

/// An owner's row; Some when one of its buttons was clicked.
#[allow(clippy::too_many_arguments)]
fn owner_row(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &mut LandUi,
    v: &View,
    pics: &Pictures,
    o: &ObjectOwner,
    name: &str,
    most: i32,
    any: bool,
    name_w: f32,
) -> Option<OwnerAction> {
    const H: f32 = 34.0;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.set_min_height(H);
        ui.spacing_mut().item_spacing.x = 0.0;
        let cell = |ui: &mut egui::Ui, w: f32, layout: egui::Layout, body: &mut dyn FnMut(&mut egui::Ui)| {
            ui.allocate_ui_with_layout(Vec2::new(w, H), layout, |ui| {
                ui.set_min_width(w);
                body(ui);
            });
        };
        let left = egui::Layout::left_to_right(egui::Align::Center);
        cell(ui, COLS[0], left, &mut |ui| {
            ui.add_space(4.0);
            let mut on = s.owners_sel.contains(&o.id);
            if widgets::checkbox(ui, p, &mut on, "").changed() {
                if on {
                    s.owners_sel.insert(o.id);
                } else {
                    s.owners_sel.remove(&o.id);
                }
            }
        });
        cell(ui, name_w, left, &mut |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(6.0);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
            if o.is_group {
                // a group: the square of the group lists
                widgets::avatar_picture(ui, p, rect, None, name, false);
            } else {
                let pic = pics.avatars.get(&o.id);
                if pic.is_none() {
                    s.wanted_pics.insert(o.id);
                }
                widgets::avatar_disc(ui, p, rect, pic, name, false);
            }
            let r = ui
                .add(egui::Label::new(semibold(ui, name, 12.5, p.ink)).truncate().sense(Sense::click()))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if r.double_clicked() || r.clicked() {
                // onDoubleClickOwner: the profile
                if o.is_group {
                    crate::ui::group_profile::request_open(ui.ctx(), o.id);
                } else {
                    crate::ui::profile::request_open(ui.ctx(), o.id);
                }
            }
            let tag = if o.id == v.agent {
                Some((aurora_i18n::tr!("ui-land-people-you"), p.teal))
            } else if o.is_group {
                Some((aurora_i18n::tr!("ui-land-people-group-tag"), p.muted))
            } else {
                None
            };
            if let Some((text, color)) = tag {
                let g = ui.painter().layout_job(semibold(ui, text, 10.5, color));
                let (rect, _) = ui.allocate_exact_size(g.size() + Vec2::new(12.0, 2.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, ui.visuals().window_corner_radius, color.gamma_multiply(0.14));
                ui.painter().galley(rect.min + Vec2::new(6.0, 1.0), g, color);
            }
        });
        cell(ui, COLS[1], egui::Layout::top_down(egui::Align::Max), &mut |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add_space(8.0);
            ui.label(semibold(ui, &aurora_i18n::integer(i64::from(o.count)), 12.5, p.ink));
            let (bar, _) = ui.allocate_exact_size(Vec2::new(56.0, 4.0), Sense::hover());
            ui.painter().rect_filled(bar, widgets::TRACK_RADIUS, p.bar);
            let fill = Rect::from_min_size(bar.min, Vec2::new(bar.width() * o.count as f32 / most as f32, bar.height()));
            ui.painter().rect_filled(fill, widgets::TRACK_RADIUS, p.violet_light);
        });
        cell(ui, COLS[2], left, &mut |ui| {
            ui.add_space(12.0);
            if o.most_recent > 0 {
                ui.label(RichText::new(when(i64::from(o.most_recent), v.now)).size(11.5).color(p.muted))
                    .on_hover_text(super::claim_date(i64::from(o.most_recent)));
            }
        });
        cell(ui, COLS[3], egui::Layout::right_to_left(egui::Align::Center), &mut |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(4.0);
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("ui-land-people-return"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                any,
            )
            .clicked()
            {
                out = Some(OwnerAction::Return(Confirm::ReturnList(o.clone())));
            }
            // clickShowCore(RT_LIST): the simulator highlights their objects
            if widgets::action_button_if(
                ui,
                p,
                "",
                aurora_i18n::tr!("common-show"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                any,
            )
            .on_hover_text(aurora_i18n::tr!("ui-land-people-show-tip"))
            .clicked()
            {
                out = Some(OwnerAction::Show(o.id));
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left_of_the_entries() {
        assert_eq!(time_left(0, 100), None);
        assert_eq!(time_left(4 * 3600 + 100, 0).as_deref(), Some("4 h"));
        assert_eq!(time_left(24 * 60 + 30, 0).as_deref(), Some("24 min"));
        assert_eq!(time_left(30, 0).as_deref(), Some("30 s"));
        assert_eq!(time_left(10, 100).as_deref(), Some("0 s"));
    }

    #[test]
    fn the_search_looks_at_both_names() {
        assert!(matches("", "Nova ♪", None));
        assert!(matches("nova", "Nova ♪", Some("nova.exemple")));
        assert!(matches("EXEMPLE", "Nova ♪", Some("nova.exemple")));
        assert!(!matches("tess", "Nova ♪", Some("nova.exemple")));
    }

    #[test]
    fn owners_sort_and_return() {
        let o = |n: u128, count: i32, recent: u32, is_group: bool| ObjectOwner {
            id: Uuid::from_u128(n),
            is_group,
            count,
            online: false,
            most_recent: recent,
        };
        let mut rows = vec![
            (o(1, 47, 30, false), "Aurora Demo".to_owned()),
            (o(2, 12, 20, true), "Aurora Builders".to_owned()),
            (o(3, 2, 10, false), "Orion".to_owned()),
        ];
        sort_owners(&mut rows, DEFAULT_SORT);
        assert_eq!(rows.iter().map(|r| r.0.count).collect::<Vec<_>>(), [47, 12, 2]);
        sort_owners(&mut rows, (1, true));
        assert_eq!(rows[0].1, "Aurora Builders");
        sort_owners(&mut rows, (3, true));
        assert_eq!(rows[0].1, "Orion");
        assert!(return_selection(Vec::new()).is_none());
        assert!(matches!(return_selection(vec![o(3, 2, 0, false)]), Some(Confirm::ReturnList(_))));
        assert!(matches!(
            return_selection(vec![o(3, 2, 0, false), o(2, 12, 0, true)]),
            Some(Confirm::ReturnOwners(v)) if v.len() == 2
        ));
    }

    #[test]
    fn newest_object_in_words() {
        // 2025-03-12 18:00 UTC = 11:00 PDT
        let now = 1_741_802_400;
        assert_eq!(
            when(now - 3600, now),
            aurora_i18n::tr!("ui-land-people-today", time = "10:00".to_string())
        );
        assert_eq!(when(now - 86_400, now), aurora_i18n::tr!("ui-land-people-yesterday"));
        assert_eq!(when(now - 40 * 86_400, now), super::super::overview::day(now - 40 * 86_400));
    }
}
