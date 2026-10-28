//! The "Land and L$" tab (LLPanelGroupLandMoney, indra/newview/
//! llpanelgrouplandmoney.cpp, panel_group_land_money.xml, originally LGPL
//! 2.1): the land the group uses, has left and was given, with our own
//! contribution (applyContribution: between 0 and what we hold plus what we
//! give already); the group's parcels (onMapButton, and the parcel's About
//! Land or place profile); the group's L$ by week, on three pages:
//! planning (this week's balance, credits and debits), details and sales,
//! eight weeks back at most. Members only (isVisibleByAgent), as in
//! Firestorm.

use super::view::{self, View};
use super::{GroupAction, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::place_details::Ctx;
use crate::ui::{menu, widgets};
use crate::world::group_money::GroupMoneyData;
use aurora_net::GroupMembership;
use aurora_net::group_money::{AccountBody, AccountLine, AccountSummary, GroupParcel, LandTotals, Ledger, Transaction, trans};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

const ROW_H: f32 = 30.0;
/// Width of the area column and of the action column.
const AREA_W: f32 = 120.0;
const ACTION_W: f32 = 44.0;

#[derive(Default)]
pub struct LandState {
    /// The group the tab was activated for; None when another tab shows
    /// (activate() runs again each time the tab is shown).
    pub active: Option<Uuid>,
    /// The page of the accounts shown.
    pub ledger: Ledger,
    /// Our contribution while it is typed.
    contribution: Option<String>,
    /// The most we may give, set when the tab is activated as Firestorm
    /// does (the land we hold changes only by buying more).
    max: i32,
    selected: Option<usize>,
}

/// The tab shows for a member (LLPanelGroupLandMoney::isVisibleByAgent).
pub fn visible(v: &View) -> bool {
    v.member.is_some()
}

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let Some(me) = v.member else { return };
    if st.land.active != Some(v.id) {
        st.land.active = Some(v.id);
        st.land.contribution = None;
        st.land.selected = None;
        // activate(): our contribution plus what we hold and give to nobody
        st.land.max = me
            .contribution
            .saturating_add(c.world.land_credit.saturating_sub(c.world.land_committed));
        actions.push(GroupAction::LoadLand {
            group: v.id,
            ledger: st.land.ledger,
        });
    }
    let data = c.world.group_money.get(&v.id);
    egui::ScrollArea::vertical()
        .id_salt("group_profile_land")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width() - 8.0;
            ui.horizontal_top(|ui| {
                ui.add_space(4.0);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 10.0;
                    let totals = data.and_then(|d| d.totals);
                    stats(st, ui, c, v, me, totals, actions);
                    if totals.is_some_and(|t| t.available() < 0) {
                        over_limit(ui, c.p);
                    }
                    parcels(st, ui, c, data, actions);
                    money(st, ui, c, v, data, actions);
                });
            });
        });
}

/// "4 096 m²".
fn area(n: i32) -> String {
    aurora_i18n::tr!("ui-group-profile-area", area = aurora_i18n::integer(i64::from(n)))
}

/// "L$ 1 250", "− L$ 300".
pub fn lindens(n: i32) -> String {
    let text = aurora_i18n::tr!("ui-group-land-lindens", amount = aurora_i18n::integer(i64::from(n).abs()));
    if n < 0 { format!("− {text}") } else { text }
}

/// A figure of the head: its value large, its name below.
fn tile(ui: &mut egui::Ui, p: &Palette, width: f32, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(p.bar)
        .corner_radius(ui.visuals().window_corner_radius)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.set_height(42.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                body(ui);
            });
        });
}

fn figure(ui: &mut egui::Ui, p: &Palette, value: &str, color: Color32, name: &str) {
    let font = FontId::new(17.0, crate::ui::fonts::semibold(ui.ctx()));
    ui.label(egui::RichText::new(value).font(font).color(color));
    ui.label(egui::RichText::new(name).size(11.5).color(p.muted));
}

/// Land in use, available, contributed (processGroupLand's totals), and our
/// contribution.
fn stats(
    st: &mut GroupProfileUi,
    ui: &mut egui::Ui,
    c: &mut Ctx,
    v: &View,
    me: &GroupMembership,
    totals: Option<LandTotals>,
    actions: &mut Vec<GroupAction>,
) {
    let p = c.p;
    const GAP: f32 = 6.0;
    let w = ((ui.available_width() - GAP * 3.0) / 4.0).floor();
    let value = |f: fn(&LandTotals) -> i32| totals.map_or_else(|| "…".to_owned(), |t| area(f(&t)));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        tile(ui, p, w, |ui| {
            figure(ui, p, &value(|t| t.in_use), p.ink, aurora_i18n::tr!("ui-group-land-in-use"))
        });
        let short = totals.is_some_and(|t| t.available() < 0);
        tile(ui, p, w, |ui| {
            let color = if short { p.danger } else { p.ink };
            figure(
                ui,
                p,
                &value(LandTotals::available),
                color,
                aurora_i18n::tr!("ui-group-land-available"),
            );
        });
        tile(ui, p, w, |ui| {
            figure(
                ui,
                p,
                &value(|t| t.contributed),
                p.ink,
                aurora_i18n::tr!("ui-group-land-contributed"),
            );
        });
        tile(ui, p, w, |ui| {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-land-my-contribution"))
                    .size(11.5)
                    .color(p.muted),
            );
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                contribution(st, ui, v, me, actions);
                let max = aurora_i18n::tr!("ui-group-land-of-max", max = aurora_i18n::integer(i64::from(st.land.max)));
                ui.label(egui::RichText::new(max).size(11.5).color(p.muted));
            });
        });
    });
}

/// Our contribution, applied when the field is left
/// (LLPanelGroupLandMoney::impl::applyContribution).
fn contribution(st: &mut GroupProfileUi, ui: &mut egui::Ui, v: &View, me: &GroupMembership, actions: &mut Vec<GroupAction>) {
    let stored = me.contribution;
    let mut text = st.land.contribution.clone().unwrap_or_else(|| stored.to_string());
    let resp = ui
        .add(egui::TextEdit::singleline(&mut text).char_limit(10).desired_width(64.0))
        .on_hover_text(aurora_i18n::tr!(
            "ui-group-land-contribution-tip",
            max = aurora_i18n::integer(i64::from(st.land.max))
        ));
    if resp.has_focus() {
        text.retain(|ch| ch.is_ascii_digit());
        st.land.contribution = Some(text.clone());
    }
    if resp.lost_focus() {
        st.land.contribution = None;
        match checked_contribution(&text, stored, st.land.max) {
            Some(contribution) => actions.push(GroupAction::SetContribution { group: v.id, contribution }),
            // "we just fail silently and force the previous value"
            None => log::debug!("group contribution unchanged ({text:?})"),
        }
    }
}

/// applyContribution's rule: a new value from 0 to the maximum, else none.
pub fn checked_contribution(text: &str, stored: i32, max: i32) -> Option<i32> {
    let new: i32 = text.trim().parse().ok()?;
    (new != stored && (0..=max).contains(&new)).then_some(new)
}

/// "More land credits are needed to support land in use"
/// (group_over_limit_text).
fn over_limit(ui: &mut egui::Ui, p: &Palette) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        widgets::glyph(ui, "alert-triangle-filled", p.danger, 14.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-land-over-limit"))
                .size(12.5)
                .color(p.danger),
        );
    });
}

/// A column title, in small capitals.
fn head_label(ui: &egui::Ui, p: &Palette, text: &str, cell: Rect, right: bool) {
    let mut job = egui::text::LayoutJob::default();
    let mut format = crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, p.muted_dim);
    format.extra_letter_spacing = 0.4;
    job.append(&text.to_uppercase(), 0.0, format);
    let galley = ui.painter().layout_job(job);
    let x = if right {
        cell.right() - 8.0 - galley.size().x
    } else {
        cell.left() + 8.0
    };
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &galley, x, cell.center().y);
    ui.painter().galley(at, galley, p.muted_dim);
}

/// The region and the position in it, as Firestorm's "location" column
/// ("Sim (x, y)").
fn location(parcel: &GroupParcel) -> String {
    let x = (parcel.global_x.round() as i64).rem_euclid(256);
    let y = (parcel.global_y.round() as i64).rem_euclid(256);
    format!("{} ({x}, {y})", parcel.sim_name)
}

/// "4 096 m²", or "billable / actual m²" when they differ.
fn parcel_area(parcel: &GroupParcel) -> String {
    if parcel.billable_area == parcel.actual_area {
        area(parcel.billable_area)
    } else {
        aurora_i18n::tr!(
            "ui-group-land-billable-of",
            billable = aurora_i18n::integer(i64::from(parcel.billable_area)),
            area = area(parcel.actual_area)
        )
    }
}

/// What a parcel's row asks.
enum ParcelAct {
    Map,
    AboutLand,
    PlaceProfile,
}

/// The group's parcels (group_parcel_list).
fn parcels(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, data: Option<&GroupMoneyData>, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let width = ui.available_width();
    let shares = [1.1_f32, 1.5, 1.1];
    let free = width - AREA_W - ACTION_W;
    let total: f32 = shares.iter().sum();
    let widths: Vec<f32> = shares.iter().map(|s| free * s / total).collect();
    let (head, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
    let mut x = head.left();
    for (i, label) in [
        aurora_i18n::tr!("ui-group-land-col-parcel"),
        aurora_i18n::tr!("ui-group-land-col-region"),
        aurora_i18n::tr!("ui-group-land-col-type"),
    ]
    .into_iter()
    .enumerate()
    {
        head_label(
            ui,
            p,
            label,
            Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(widths[i], head.height())),
            false,
        );
        x += widths[i];
    }
    let area_cell = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(AREA_W, head.height()));
    head_label(ui, p, aurora_i18n::tr!("ui-group-land-col-area"), area_cell, true);

    let Some(list) = data.and_then(|d| d.parcels.as_ref()) else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    if list.is_empty() {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-land-no-parcels"))
                .size(12.5)
                .color(p.muted),
        );
        return;
    }
    let mut asked = None;
    let prev_spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 0.0;
    for (i, parcel) in list.iter().enumerate() {
        let (row, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        let selected = st.land.selected == Some(i);
        if selected {
            ui.painter().rect_filled(row, 0.0, p.violet.gamma_multiply(0.16));
        } else if resp.hovered() {
            ui.painter().rect_filled(row, 0.0, p.raised.gamma_multiply(0.35));
        }
        ui.painter()
            .hline(row.x_range(), row.top(), egui::Stroke::new(1.0, p.raised.gamma_multiply(0.3)));
        if resp.clicked() {
            st.land.selected = Some(i);
        }
        if resp.double_clicked() {
            asked = Some((i, ParcelAct::PlaceProfile));
        }
        let mid = row.center().y;
        let mut x = row.left();
        widgets::paint_elided(
            ui,
            &parcel.name,
            Pos2::new(x + 8.0, mid),
            widths[0] - 16.0,
            FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
            p.ink,
        );
        x += widths[0];
        // the region's rating, once the map knows it
        let mut text_x = x + 8.0;
        if let Some(r) = widgets::region_rating(ui.ctx(), &c.world.map.ratings, &parcel.sim_name) {
            let badge = Rect::from_min_size(Pos2::new(text_x, mid - 7.0), Vec2::splat(14.0));
            widgets::paint_maturity_badge(
                ui.painter(),
                p,
                crate::ui::bars::maturity_name(r),
                badge,
                widgets::inner_radius(ui.visuals()),
            );
            text_x += 20.0;
        }
        widgets::paint_elided(
            ui,
            &location(parcel),
            Pos2::new(text_x, mid),
            x + widths[1] - 8.0 - text_x,
            FontId::proportional(12.5),
            p.muted,
        );
        x += widths[1];
        let kind = c
            .world
            .search
            .land_types
            .get(&parcel.sku)
            .cloned()
            .unwrap_or_else(|| aurora_i18n::tr!("ui-group-land-type-unknown").to_owned());
        widgets::paint_elided(
            ui,
            &kind,
            Pos2::new(x + 8.0, mid),
            widths[2] - 16.0,
            FontId::proportional(12.5),
            p.muted,
        );
        x += widths[2];
        let galley = ui
            .painter()
            .layout_no_wrap(parcel_area(parcel), FontId::proportional(12.5), p.muted);
        let pos = crate::ui::widgets::row_text_origin(ui.painter(), &galley, x + AREA_W - 8.0 - galley.size().x, mid);
        ui.painter().galley(pos, galley, p.muted);
        // "Map" at the end of the row
        let cell = Rect::from_min_max(Pos2::new(row.right() - ACTION_W, row.top()), row.right_bottom());
        let mut b = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(cell)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        if widgets::icon_button(&mut b, p, "map", aurora_i18n::tr!("common-show-on-map-2"), true) {
            asked = Some((i, ParcelAct::Map));
        }
        let nearby = crate::ui::land::LandUi::region_at_global(c.world, f64::from(parcel.global_x), f64::from(parcel.global_y)).is_some();
        menu::context_menu(&resp, p, |ui| {
            if menu::item(ui, p, "map", aurora_i18n::tr!("common-show-on-map-2")) {
                asked = Some((i, ParcelAct::Map));
            }
            if menu::item_if(ui, p, "info-circle-filled", aurora_i18n::tr!("common-about-land"), nearby) {
                asked = Some((i, ParcelAct::AboutLand));
            }
            if menu::item(ui, p, "map-pin-filled", aurora_i18n::tr!("common-place-profile")) {
                asked = Some((i, ParcelAct::PlaceProfile));
            }
        });
    }
    ui.spacing_mut().item_spacing.y = prev_spacing;
    if let Some((i, act)) = asked
        && let Some(parcel) = list.get(i)
    {
        st.land.selected = Some(i);
        let (x, y) = (f64::from(parcel.global_x), f64::from(parcel.global_y));
        match act {
            ParcelAct::Map => actions.push(GroupAction::ShowOnMap { x, y }),
            ParcelAct::AboutLand => actions.push(GroupAction::AboutLand { x, y }),
            ParcelAct::PlaceProfile => {
                let pos = glam::Vec3::new(x.rem_euclid(256.0) as f32, y.rem_euclid(256.0) as f32, parcel.global_z);
                context::request(ui.ctx(), CtxAction::ShowPlaceInfo(parcel.sim_name.clone(), pos));
            }
        }
    }
}

/// "week of 6 Oct." from a reply's StartDate.
fn week_label(start: &str) -> Option<String> {
    let (y, m, d) = aurora_net::group_money::parse_date(start)?;
    Some(aurora_i18n::tr!("ui-group-land-week-of", date = day_month(y, m, d)))
}

/// "6 Oct.", the year added when it is not this one.
fn day_month(y: i32, m: u32, d: u32) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let this_year = aurora_net::profile::civil_from_days(now.div_euclid(86_400)).0;
    let y = i64::from(y);
    crate::ui::land::day_month((y != this_year).then_some(y), m as usize, i64::from(d))
}

/// A small chevron of the card's title: a week earlier or later.
fn chevron(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, enabled: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let color = if enabled { p.violet_light } else { p.muted_dim };
    if let Some(t) = crate::ui::icons::global(icon) {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            color,
        );
    }
    let resp = resp.on_hover_text(tip);
    enabled && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The group's L$: the three pages, the week and its arrows.
fn money(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, data: Option<&GroupMoneyData>, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let ledger = st.land.ledger;
    let page = data.and_then(|d| d.page(ledger));
    let mut step = None;
    view::card_with(
        ui,
        p,
        "coin-filled",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-land-money"),
        |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            // the planning page has no other week (no buttons in Firestorm)
            if ledger != Ledger::Planning {
                let later = page.is_some_and(|p| p.can_go_later());
                if chevron(ui, p, "chevron-right", aurora_i18n::tr!("ui-group-land-later"), later) {
                    step = Some(false);
                }
                let earlier = page.is_some_and(|p| p.can_go_earlier());
                if chevron(ui, p, "chevron-left", aurora_i18n::tr!("ui-group-land-earlier"), earlier) {
                    step = Some(true);
                }
                ui.add_space(4.0);
            }
            if let Some(label) = page.filter(|p| p.body.is_some()).and_then(|p| week_label(&p.start_date)) {
                ui.label(egui::RichText::new(label).size(11.5).color(p.muted));
            }
        },
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                for (l, label) in [
                    (Ledger::Planning, aurora_i18n::tr!("ui-group-land-planning")),
                    (Ledger::Details, aurora_i18n::tr!("ui-group-land-details")),
                    (Ledger::Sales, aurora_i18n::tr!("ui-group-land-sales")),
                ] {
                    if super::members::pill(ui, p, label, ledger == l).clicked() && ledger != l {
                        st.land.ledger = l;
                        actions.push(GroupAction::LandPage { group: v.id, ledger: l });
                    }
                }
            });
            ui.add_space(2.0);
            ui.spacing_mut().item_spacing.y = 7.0;
            match page.and_then(|p| p.body.as_ref().map(|b| (p.interval, b))) {
                None => {
                    ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
                }
                Some((interval, AccountBody::Summary(s))) => summary(ui, p, s, interval == 0),
                Some((_, AccountBody::Details(lines))) => details(ui, p, lines),
                Some((_, AccountBody::Sales(list))) => sales(ui, p, list),
            }
        },
    );
    if let Some(earlier) = step {
        actions.push(GroupAction::LandStep {
            group: v.id,
            ledger,
            earlier,
        });
    }
}

/// One line of the accounts: its name at the left, its amount at the right.
fn kv(ui: &mut egui::Ui, p: &Palette, key: &str, value: &str, color: Color32, strong: bool) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
    let font = if strong {
        FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()))
    } else {
        FontId::proportional(12.5)
    };
    let value = ui.painter().layout_no_wrap(value.to_owned(), font.clone(), color);
    let value_w = value.size().x;
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &value, row.right() - value_w, row.center().y);
    ui.painter().galley(at, value, color);
    widgets::paint_elided(
        ui,
        key,
        Pos2::new(row.left(), row.center().y),
        row.width() - value_w - 12.0,
        font,
        if strong { p.ink } else { p.muted },
    );
}

fn amount_color(p: &Palette, n: i32) -> Color32 {
    if n < 0 { p.danger } else { p.ink }
}

/// LLGroupMoneyPlanningTabEventHandler::processReply: the balance (this
/// week), the group's credits, debits and total, the next stipend day.
fn summary(ui: &mut egui::Ui, p: &Palette, s: &AccountSummary, this_week: bool) {
    if this_week {
        kv(ui, p, aurora_i18n::tr!("ui-group-land-balance"), &lindens(s.balance), p.ink, false);
    }
    kv(
        ui,
        p,
        aurora_i18n::tr!("ui-group-land-credits"),
        &lindens(s.total_credits),
        p.ink,
        false,
    );
    kv(
        ui,
        p,
        aurora_i18n::tr!("ui-group-land-debits"),
        &lindens(s.total_debits),
        amount_color(p, s.total_debits),
        false,
    );
    let total = s.total_credits.saturating_add(s.total_debits);
    kv(
        ui,
        p,
        aurora_i18n::tr!("ui-group-land-total"),
        &lindens(total),
        amount_color(p, total),
        true,
    );
    if this_week && let Some((y, m, d)) = aurora_net::group_money::parse_date(&s.next_stipend) {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-land-next-stipend", date = day_month(y, m, d)))
                .size(11.5)
                .color(p.muted),
        );
    }
}

/// LLGroupMoneyDetailsTabEventHandler::processReply: each line with an
/// amount, then the total of all.
fn details(ui: &mut egui::Ui, p: &Palette, lines: &[AccountLine]) {
    let mut any = false;
    for l in lines.iter().filter(|l| l.amount != 0) {
        any = true;
        kv(ui, p, &l.description, &lindens(l.amount), amount_color(p, l.amount), false);
    }
    if !any {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-land-none"))
                .size(12.5)
                .color(p.muted),
        );
    }
    let total: i32 = lines.iter().map(|l| l.amount).fold(0, i32::saturating_add);
    kv(
        ui,
        p,
        aurora_i18n::tr!("ui-group-land-total"),
        &lindens(total),
        amount_color(p, total),
        true,
    );
}

/// The verb of a sale (GroupMoneyBought, GroupMoneyPaidYou…).
fn verb(kind: i32) -> &'static str {
    match kind {
        trans::OBJECT_SALE => aurora_i18n::tr!("ui-group-land-bought"),
        trans::GIFT => aurora_i18n::tr!("ui-group-land-paid-you"),
        trans::PAY_OBJECT => aurora_i18n::tr!("ui-group-land-paid-into"),
        trans::LAND_PASS_SALE => aurora_i18n::tr!("ui-group-land-bought-pass"),
        trans::EVENT_FEE => aurora_i18n::tr!("ui-group-land-paid-event-fee"),
        trans::EVENT_PRIZE => aurora_i18n::tr!("ui-group-land-paid-event-prize"),
        _ => "",
    }
}

/// LLGroupMoneySalesTabEventHandler::processReply: "time amount - user
/// verb item" for each sale with an amount, "(none)" without any.
fn sales(ui: &mut egui::Ui, p: &Palette, list: &[Transaction]) {
    let shown: Vec<&Transaction> = list.iter().filter(|t| t.amount != 0).collect();
    if list.is_empty() {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-land-none"))
                .size(12.5)
                .color(p.muted),
        );
        return;
    }
    const TIME_W: f32 = 128.0;
    for t in shown {
        let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
        let mid = row.center().y;
        widgets::paint_elided(
            ui,
            &t.time,
            Pos2::new(row.left(), mid),
            TIME_W - 10.0,
            FontId::proportional(11.5),
            p.muted_dim,
        );
        let color = amount_color(p, t.amount);
        let amount = ui.painter().layout_no_wrap(lindens(t.amount), FontId::proportional(12.5), color);
        let amount_w = amount.size().x;
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &amount, row.right() - amount_w, mid);
        ui.painter().galley(at, amount, color);
        let what = [t.user.as_str(), verb(t.kind), t.item.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        widgets::paint_elided(
            ui,
            &what,
            Pos2::new(row.left() + TIME_W, mid),
            row.width() - TIME_W - amount_w - 12.0,
            FontId::proportional(12.5),
            p.ink,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contribution_is_applied_only_when_new_and_within_what_we_hold() {
        assert_eq!(checked_contribution("1024", 512, 1024), Some(1024));
        assert_eq!(checked_contribution("0", 512, 1024), Some(0));
        assert_eq!(checked_contribution("512", 512, 1024), None, "unchanged");
        assert_eq!(checked_contribution("2048", 512, 1024), None, "more than we hold");
        assert_eq!(checked_contribution("", 512, 1024), None);
    }

    #[test]
    fn amounts_and_locations_read_like_firestorm() {
        let parcel = GroupParcel {
            name: "Atelier".into(),
            desc: String::new(),
            actual_area: 1024,
            billable_area: 512,
            global_x: 255_744.0 + 30.4,
            global_y: 255_744.0 + 255.6,
            global_z: 0.0,
            sim_name: "Sablière".into(),
            sku: String::new(),
        };
        // ll_round(global) % REGION_WIDTH_UNITS
        assert_eq!(location(&parcel), "Sablière (30, 0)");
        assert!(parcel_area(&parcel).contains(&aurora_i18n::integer(512)));
        assert!(lindens(-300).starts_with('−'));
        assert!(!lindens(300).starts_with('−'));
    }
}
