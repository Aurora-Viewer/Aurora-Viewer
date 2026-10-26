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
use crate::world::recent::Met;
use egui::{Rect, Sense, Vec2};
use uuid::Uuid;

/// The recent tab: the list, and the sheet of the person clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-recent-list", list, super::person_sheet);
}

/// "12 min ago · Ahern".
fn ago(met: &Met) -> String {
    let seconds = met.when.elapsed().map_or(0.0, |d| d.as_secs_f64());
    let when = aurora_i18n::tr!("people-recent-ago", when = super::nearby::seen_text(seconds));
    if met.region.is_empty() {
        when
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
            widgets::search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-filter-short"), w);
        },
    );
    ui.add_space(7.0);
    let world = c.world;
    let filter = st.filter.trim().to_lowercase();
    let people: Vec<(Met, String)> = world
        .recent
        .list()
        .iter()
        .map(|m| (m.clone(), rows::display_name(world, &m.id)))
        .filter(|(met, name)| super::matches_name(world, &met.id, name, &filter))
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if !data.pics.contains_key(&id) {
        st.want_pic(id, ui.input(|i| i.time));
    }
    let selected = st.sel.contains(&id) || st.sheet == Some(id);
    let hot = (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(id) || st.demo_hover == Some(id);
    rows::face(ui, p, rect, selected, hot);
    let left = rows::picture(ui, p, world, rect, data.pics.get(&id), id, name, false);
    let friend = world.social.is_friend(&id);
    // Near you's quick actions: profile, IM, a friendship offer, the menu
    let buttons: Option<Vec<(&str, &str)>> = hot.then(|| {
        let mut b = vec![
            ("user-filled", aurora_i18n::tr!("common-view-profile")),
            ("message-circle-filled", aurora_i18n::tr!("ui-context-send-im")),
        ];
        if !friend {
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
        &ago(met),
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
    rows::person_menu(&resp, p, world, &st.sel, id);
    let detail = (cut && resp.hover_pos().is_some_and(|at| at.x <= end.names_right)).then_some(name);
    rows::contact_tip(resp, detail);
}

fn hover_actions(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, id: Uuid, slot: Rect, buttons: &[(&str, &str)]) {
    let p = c.p;
    let world = c.world;
    let mut menu_open = false;
    for ((icon, _), resp) in buttons.iter().zip(rows::actions(ui, p, slot, buttons, ("people-recent-act", id))) {
        match *icon {
            "user-filled" if resp.clicked() => c.actions.push(PeopleAction::Profile(id)),
            "message-circle-filled" if resp.clicked() => c.actions.push(PeopleAction::OpenIm(id)),
            "user-plus" if resp.clicked() => context::request(ui.ctx(), CtxAction::AddFriend(id)),
            "dots-filled" => {
                menu_open = menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_list_menu(ui, p, world, id, AvatarList::Nearby);
                });
            }
            _ => {}
        }
    }
    super::keep_row_menu(st, id, menu_open);
}
