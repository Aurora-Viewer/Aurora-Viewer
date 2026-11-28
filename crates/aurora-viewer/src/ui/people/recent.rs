//! People › Recent in Aurora's look (the People mock-up v2): the residents
//! we dealt with lately (`world/recent.rs`, LLRecentPeople; the recent tab
//! of LLPanelPeople, panel_people.xml) as Near you's rows, newest first,
//! with how long ago and the region we were in. The menu is the avatar
//! list's (menu_people_recent.xml has the same entries); a plain click
//! opens the sheet, Ctrl / Shift choose several.

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleAction, PeopleUi};
use crate::ui::context::{self, AvatarList, CtxAction};
use crate::ui::{menu, widgets};
use crate::world::names::NameScope;
use crate::world::recent::Met;
use egui::{Rect, Vec2};
use uuid::Uuid;

/// The recent tab: the list, and the sheet of the person clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-recent-list", list, super::person_sheet);
}

/// LLAvatarListItem::formatSeconds (indra/newview/llavatarlistitem.cpp):
/// use the largest whole unit, with fixed 30-day months and 365-day years.
fn elapsed_text(seconds: u64) -> String {
    const DAY: u64 = 86_400;
    let days = seconds / DAY;
    if days >= 365 {
        aurora_i18n::tr!("profile-age-years", count = (days / 365) as i64)
    } else if days >= 30 {
        aurora_i18n::tr!("profile-age-months", count = (days / 30) as i64)
    } else if days >= 7 {
        aurora_i18n::tr!("profile-age-weeks", count = (days / 7) as i64)
    } else if days >= 1 {
        aurora_i18n::tr!("profile-age-days", count = days as i64)
    } else if seconds >= 3600 {
        aurora_i18n::tr!("people-recent-hours", count = (seconds / 3600) as i64)
    } else if seconds >= 60 {
        aurora_i18n::tr!("people-recent-minutes", count = (seconds / 60) as i64)
    } else {
        aurora_i18n::tr!("people-recent-seconds", count = seconds as i64)
    }
}

fn interaction_time(when: std::time::SystemTime, zone: crate::settings::ClockZone, twelve: bool) -> Option<String> {
    use crate::ui::bars::clock;
    let unix = i64::try_from(when.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let (year, month, day, _, hour, minute, second) = crate::ui::land::parts_at(unix, clock::offset(zone, unix));
    Some(aurora_i18n::tr!(
        "people-recent-last-interaction",
        date = format!("{year:04}-{month:02}-{day:02}"),
        time = clock::hour_text(hour, minute, &format!(":{second:02}"), twelve),
        zone = clock::zone_name(zone, unix)
    ))
}

/// "12 min ago · Ahern". Under RLVa @showloc a region the agent is
/// connected to is "(Hidden region)", as every list names it
/// (RlvUtil::isNearbyRegion, rlvcommon.cpp); one met there under the
/// restriction was kept without its name (`World::met`).
fn ago(met: &Met, world: &crate::world::World) -> String {
    let seconds = met.when.elapsed().map_or(0, |d| d.as_secs());
    let when = aurora_i18n::tr!("people-recent-ago", when = elapsed_text(seconds));
    if met.region.is_empty() {
        when
    } else if world.rlva_hides_region(&met.region) {
        format!("{when} · {}", aurora_i18n::tr!("rlva-hidden-region"))
    } else {
        format!("{when} · {}", met.region)
    }
}

fn list(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    let p = c.p;
    if beside_sheet {
        ui.set_max_width(ui.max_rect().width() - 10.0);
    }
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    ui.allocate_ui_with_layout(
        Vec2::new(w, rows::TOOLBAR_H),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            widgets::clearable_search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-filter-short"), w);
        },
    );
    ui.add_space(7.0);
    let world = c.world;
    let filter = st.filter.trim().to_lowercase();
    let people: Vec<(Met, String)> = world
        .recent
        .list()
        .iter()
        .map(|m| (m.clone(), rows::display_name(world, NameScope::Roster, &m.id)))
        .filter(|(met, name)| super::matches_name(world, NameScope::Roster, &met.id, name, &filter))
        .collect();
    let order: Vec<Uuid> = people.iter().map(|(m, _)| m.id).collect();
    st.sel.retain(|id| order.contains(id));
    egui::ScrollArea::vertical()
        .id_salt("people-recent-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if world.recent.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("people-recent-empty"));
            } else if people.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("ui-people-nobody-here-yet"));
            }
            for (met, name) in &people {
                row(ui, c, data, st, met, name, &order);
            }
        });
}

fn row(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, met: &Met, name: &str, order: &[Uuid]) {
    let p = c.p;
    let world = c.world;
    let id = met.id;
    let (rect, resp) = rows::allocate(ui, id, Vec2::new(ui.available_width(), RICH_H));
    widgets::reveal_search_selection(
        ui,
        &resp,
        st.anchor
            .filter(|id| st.sel.contains(id))
            .or_else(|| st.sel.last().copied())
            .or(st.sheet)
            == Some(id),
    );
    if !ui.is_rect_visible(rect) {
        rows::person_menu(&resp, p, world, NameScope::Roster, &st.sel, id);
        return;
    }
    if !data.pics.contains_key(&id) {
        st.want_pic(id, ui.input(|i| i.time));
    }
    let selected = st.sel.contains(&id) || st.sheet == Some(id);
    let hot = (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(id) || st.demo_hover == Some(id);
    rows::face(ui, p, rect, selected, hot);
    let left = rows::picture(ui, p, world, NameScope::Roster, rect, data.pics.get(&id), id, name, false);
    let friend = world.social.is_friend(&id);
    // Near you's quick actions: profile, IM, a friendship offer, the menu
    let buttons: Option<Vec<(&str, &str)>> = hot.then(|| {
        let mut b = vec![
            ("user-filled", aurora_i18n::tr!("common-view-profile")),
            ("message-circle-filled", aurora_i18n::tr!("ui-context-send-im")),
        ];
        // LLPanelPeople::updateButtons (llpanelpeople.cpp): no friendship
        // offer from this list to a resident `@shownames` hides
        if !friend && world.rlva_can_name(id) {
            b.push(("user-plus", aurora_i18n::tr!("radar-add-friend")));
        }
        b.push(("dots-filled", aurora_i18n::tr!("ui-chat-more")));
        b
    });
    let widths: Vec<f32> = match &buttons {
        Some(b) => vec![rows::actions_width(b.len())],
        None if friend => vec![13.0],
        None => Vec::new(),
    };
    let end = rows::end(rect, left, &widths, 8.0, false);
    match (&buttons, end.items.first()) {
        (Some(buttons), Some(slot)) => hover_actions(ui, c, st, id, *slot, buttons),
        (None, Some(cell)) => super::radar::icon(ui, "heart-filled", *cell, 13.0, p.success),
        _ => {}
    }
    let cut = rows::person_names(
        ui,
        p,
        rect,
        left,
        end.names_right,
        name,
        crate::ui::colors::resident_name(world, &id, p.ink),
        &ago(met, world),
        None,
        None,
        crate::world::staff::mark(world, &id),
    );
    let action = rows::select(ui, &resp, &mut st.sel, &mut st.anchor, order, id);
    if action.sheet {
        super::nearby::open_sheet(st, id);
    }
    if action.activate {
        c.actions.push(PeopleAction::OpenIm(id));
    }
    rows::person_menu(&resp, p, world, NameScope::Roster, &st.sel, id);
    let detail = (cut && resp.hover_pos().is_some_and(|at| at.x <= end.names_right)).then_some(name);
    rows::contact_tip(resp, detail, || {
        use crate::ui::bars::clock;
        interaction_time(met.when, clock::zone(), clock::chat_twelve_hours())
    });
}

fn hover_actions(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, id: Uuid, slot: Rect, buttons: &[(&str, &str)]) {
    let p = c.p;
    let world = c.world;
    let mut menu_open = false;
    for ((icon, _), resp) in buttons.iter().zip(rows::actions(ui, p, slot, buttons, ("people-recent-act", id))) {
        match *icon {
            "user-filled" if resp.clicked() => c.actions.push(PeopleAction::RosterProfile(id)),
            "message-circle-filled" if resp.clicked() => c.actions.push(PeopleAction::OpenIm(id)),
            "user-plus" if resp.clicked() => context::request(ui.ctx(), CtxAction::AddFriend(id)),
            "dots-filled" => {
                menu_open = menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_roster_menu(ui, p, world, id, AvatarList::Nearby);
                });
            }
            _ => {}
        }
    }
    super::keep_row_menu(st, id, menu_open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_age_uses_the_largest_complete_unit() {
        let day = 86_400;
        for (seconds, expected) in [
            (59, aurora_i18n::tr!("people-recent-seconds", count = 59)),
            (60, aurora_i18n::tr!("people-recent-minutes", count = 1)),
            (3600, aurora_i18n::tr!("people-recent-hours", count = 1)),
            (day - 1, aurora_i18n::tr!("people-recent-hours", count = 23)),
            (day, aurora_i18n::tr!("profile-age-days", count = 1)),
            (7 * day, aurora_i18n::tr!("profile-age-weeks", count = 1)),
            (30 * day - 1, aurora_i18n::tr!("profile-age-weeks", count = 4)),
            (30 * day, aurora_i18n::tr!("profile-age-months", count = 1)),
            (2_332 * 3600, aurora_i18n::tr!("profile-age-months", count = 3)),
            (365 * day - 1, aurora_i18n::tr!("profile-age-months", count = 12)),
            (365 * day, aurora_i18n::tr!("profile-age-years", count = 1)),
        ] {
            assert_eq!(elapsed_text(seconds), expected, "{seconds} seconds");
        }
    }

    #[test]
    fn recent_hover_uses_the_saved_instant_and_its_zone_date() {
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(3661);
        let stamp = interaction_time(when, crate::settings::ClockZone::Slt, false).expect("valid date");
        assert!(
            stamp.contains("1969-12-31") && stamp.contains("17:01:01") && stamp.contains("SLT"),
            "{stamp}"
        );
    }

    /// RLVa @showloc: a region the agent is connected to is "(Hidden
    /// region)" in the list, another keeps its name.
    #[test]
    fn showloc_hides_a_connected_region_of_the_list() {
        let world = crate::world::rlva_location_tests::showloc_world();
        let met = |region: &str| Met {
            id: Uuid::from_u128(5),
            when: std::time::SystemTime::now(),
            region: region.to_owned(),
        };
        let here = world.region_name();
        assert!(ago(&met(&here), &world).ends_with(aurora_i18n::tr!("rlva-hidden-region")));
        assert!(ago(&met("Far Away"), &world).ends_with("Far Away"));
    }
}
