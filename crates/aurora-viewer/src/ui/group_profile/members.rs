//! The Members tab, read only: the list of LLPanelGroupMembersSubTab
//! (name, title, status, and for those who manage members their roles and
//! donation), its filter, the selected member's roles beside it, and their
//! profile or a conversation. Inviting, ejecting, banning and changing
//! roles belong to the members' management.

use super::view::{self, Chip, View};
use super::{Env, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind};
use aurora_net::groups::{BuiltinRole, GroupMember, LastSeen, powers};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// Which members the list shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Show {
    #[default]
    All,
    Online,
    Role(Uuid),
}

/// The columns one can sort on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Sort {
    Name,
    Title,
    #[default]
    Status,
    Donation,
}

#[derive(Default)]
pub struct MembersState {
    pub filter: String,
    pub show: Show,
    pub sort: Sort,
    /// Sorted the other way from the column's natural order.
    pub reversed: bool,
    pub selected: Option<Uuid>,
    /// The rows, filtered and sorted, with the names they were sorted by.
    rows: Vec<(Uuid, String)>,
    /// What the rows were made from, and when.
    made: Option<(String, Show, Sort, bool, usize, bool)>,
    made_at: f64,
}

/// Abilities that manage members: with one of them, the list also shows
/// the roles and the donations (as the management columns of the mockup).
const MANAGE: u64 = powers::MEMBER_INVITE
    | powers::MEMBER_EJECT
    | powers::ROLE_ASSIGN_MEMBER
    | powers::ROLE_ASSIGN_MEMBER_LIMITED
    | powers::ROLE_REMOVE_MEMBER;

/// What a member's chip says and how: the owners, the officers (the role
/// the server calls "Officers"), the others.
pub fn chip_of(v: &View, m: &GroupMember) -> Chip {
    if m.is_owner {
        return Chip::Owner;
    }
    let officer = v.data.is_some_and(|d| {
        d.member_roles.get(&m.id).is_some_and(|roles| {
            roles.iter().any(|r| {
                d.roles
                    .get(r)
                    .is_some_and(|r| BuiltinRole::of(&r.name) == Some(BuiltinRole::Officers))
            })
        })
    });
    if officer { Chip::Officer } else { Chip::Plain }
}

/// A role's name as LLGroupMgr shows it (the built-in ones translated).
pub fn role_name(name: &str) -> String {
    match BuiltinRole::of(name) {
        Some(BuiltinRole::Everyone) => aurora_i18n::tr!("ui-group-profile-role-everyone").to_owned(),
        Some(BuiltinRole::Officers) => aurora_i18n::tr!("ui-group-profile-role-officers").to_owned(),
        Some(BuiltinRole::Owners) => aurora_i18n::tr!("ui-group-profile-role-owners").to_owned(),
        None => name.to_owned(),
    }
}

/// "online", "today", "yesterday", "N days ago", or the date.
pub fn status_text(seen: &LastSeen, today: (i64, u32, u32)) -> String {
    match seen {
        LastSeen::Online => aurora_i18n::tr!("ui-group-profile-status-online").to_owned(),
        LastSeen::Date(y, m, d) => {
            let days = aurora_net::profile::days_from_civil(today.0, today.1, today.2)
                - aurora_net::profile::days_from_civil(i64::from(*y), u32::from(*m), u32::from(*d));
            match days {
                0 => aurora_i18n::tr!("ui-chat-today").to_lowercase(),
                1 => aurora_i18n::tr!("ui-chat-yesterday").to_lowercase(),
                2..=6 => aurora_i18n::tr!("ui-group-profile-days-ago", count = days),
                _ => aurora_i18n::tr!(
                    "profile-date",
                    month = format!("{m:02}"),
                    day = format!("{d:02}"),
                    year = y.to_string()
                ),
            }
        }
        LastSeen::Other(text) => text.clone(),
    }
}

/// Today in SLT, as the dates of the server are.
fn today() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (y, m, d) = aurora_net::profile::civil_from_days((secs - 8 * 3600).div_euclid(86_400));
    (y, m, d)
}

/// The rows of the list: filtered on the name or the title, on the online
/// members or a role, sorted on a column (then by name).
pub fn rows(v: &View, names: &dyn Fn(&Uuid) -> String, filter: &str, show: Show, sort: Sort, reversed: bool) -> Vec<(Uuid, String)> {
    let Some(d) = v.data else {
        return Vec::new();
    };
    let filter = filter.trim().to_lowercase();
    let mut rows: Vec<(&GroupMember, String)> = d
        .members
        .values()
        .filter(|m| match show {
            Show::All => true,
            Show::Online => m.last_seen == LastSeen::Online,
            Show::Role(r) => d.member_roles.get(&m.id).is_some_and(|roles| roles.contains(&r)),
        })
        .map(|m| (m, names(&m.id)))
        .filter(|(m, name)| filter.is_empty() || name.to_lowercase().contains(&filter) || m.title.to_lowercase().contains(&filter))
        .collect();
    rows.sort_by(|(a, an), (b, bn)| {
        let by_name = an.to_lowercase().cmp(&bn.to_lowercase());
        let order = match sort {
            Sort::Name => by_name,
            Sort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(by_name),
            // most recent first
            Sort::Status => b.last_seen.rank().cmp(&a.last_seen.rank()).then(by_name),
            Sort::Donation => b.contribution.cmp(&a.contribution).then(by_name),
        };
        if reversed { order.reverse() } else { order }
    });
    rows.into_iter().map(|(m, n)| (m.id, n)).collect()
}

/// A filter pill of the bar.
fn pill(ui: &mut egui::Ui, p: &Palette, label: &str, on: bool) -> egui::Response {
    let font = FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx()));
    let ink = if on { p.ink } else { p.muted };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 18.0, 26.0), Sense::click());
    let radius = ui.visuals().window_corner_radius;
    let fill = if on {
        p.violet.gamma_multiply(0.28)
    } else if resp.hovered() {
        p.raised
    } else {
        p.bar
    };
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            radius,
            egui::Stroke::new(1.0, p.violet_light.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );
    }
    ui.painter().galley(
        Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0).round(),
        galley,
        ink,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A round picture with its status dot.
fn avatar(owner: &mut GroupProfileUi, ui: &egui::Ui, c: &Ctx, env: &Env, rect: Rect, id: Uuid, name: &str, online: bool) {
    let pic = env.avatar_pics.get(&id);
    if pic.is_none() {
        owner.want_pic(id, env.now);
    }
    widgets::avatar_disc(ui, c.p, rect, pic, name, false);
    // what the scene or the friend list says, else what the group says
    let status = match crate::ui::presence::of(c.world, &id) {
        crate::ui::presence::Status::Unknown | crate::ui::presence::Status::Offline if online => crate::ui::presence::Status::Online,
        crate::ui::presence::Status::Unknown => crate::ui::presence::Status::Offline,
        s => s,
    };
    crate::ui::presence::paint_dot(ui.painter(), c.p, rect, status, c.p.panel);
}

const ROW_H: f32 = 30.0;

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View) {
    let p = c.p;
    let manage = v.can(MANAGE);
    let body = Rect::from_min_max(
        ui.cursor().min + Vec2::new(4.0, 0.0),
        ui.max_rect().right_bottom() - Vec2::new(4.0, 0.0),
    );
    let side_w = 290.0_f32.min(body.width() * 0.4);
    let main = Rect::from_min_max(body.min, Pos2::new(body.right() - side_w - 12.0, body.bottom()));
    let side = Rect::from_min_max(Pos2::new(main.right() + 12.0, body.top()), body.max);
    ui.allocate_rect(body, Sense::hover());

    let mut m = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(main)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    m.set_clip_rect(main.intersect(ui.clip_rect()));
    list(st, &mut m, c, env, v, manage);

    let mut s = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(side)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    s.set_clip_rect(side.intersect(ui.clip_rect()));
    roles_card(st, &mut s, c, v, p);
}

/// The bar, the table, the selection's buttons.
fn list(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View, manage: bool) {
    let p = c.p;
    let mut invite = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let w = (ui.available_width() - 260.0).max(120.0);
        widgets::search_field(ui, &mut st.members.filter, aurora_i18n::tr!("ui-group-profile-filter-members"), w);
        if pill(ui, p, aurora_i18n::tr!("common-all"), st.members.show == Show::All).clicked() {
            st.members.show = Show::All;
        }
        if pill(
            ui,
            p,
            aurora_i18n::tr!("ui-group-profile-online-filter"),
            st.members.show == Show::Online,
        )
        .clicked()
        {
            st.members.show = Show::Online;
        }
        let role_label = match st.members.show {
            Show::Role(r) => v
                .data
                .and_then(|d| d.roles.get(&r))
                .map_or_else(String::new, |r| role_name(&r.name)),
            _ => aurora_i18n::tr!("ui-group-profile-by-role").to_owned(),
        } + " ▾";
        let by_role = pill(ui, p, &role_label, matches!(st.members.show, Show::Role(_)));
        // "Invite" with the ability, as in the head
        if v.can(powers::MEMBER_INVITE)
            && widgets::action_button(
                ui,
                p,
                "user-plus",
                aurora_i18n::tr!("ui-group-profile-invite"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
        {
            invite = true;
        }
        crate::ui::menu::dropdown(&by_role, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            let mut roles: Vec<_> = v.data.map(|d| d.roles.values().collect()).unwrap_or_default();
            roles.sort_by_cached_key(|r| (!r.id.is_nil(), role_name(&r.name).to_lowercase()));
            if roles.is_empty() {
                crate::ui::menu::info(ui, p, "loader-2", aurora_i18n::tr!("common-loading-2"));
            }
            for r in roles {
                if crate::ui::menu::item(ui, p, "users", &role_name(&r.name)) {
                    st.members.show = Show::Role(r.id);
                }
            }
        });
    });
    if invite {
        st.open_invite(v.id);
    }
    ui.add_space(8.0);

    // the rows again when what they come from changed, and once a second
    // for the names that arrived
    let d = v.data;
    let key = (
        st.members.filter.clone(),
        st.members.show,
        st.members.sort,
        st.members.reversed,
        d.map_or(0, |d| d.members.len() + d.member_roles.len()),
        d.is_some_and(|d| d.members_complete),
    );
    if st.members.made.as_ref() != Some(&key) || env.now - st.members.made_at > 1.0 || env.now < st.members.made_at {
        let world = c.world;
        st.members.rows = rows(
            v,
            &|id| world.social.display_name_of(id),
            &st.members.filter,
            st.members.show,
            st.members.sort,
            st.members.reversed,
        );
        // the names still unknown are asked (all of them: the filter and
        // the order need them)
        if let Some(d) = d {
            c.want_names
                .extend(d.members.keys().filter(|id| world.social.display_name_of(id) == "…"));
        }
        st.members.made = Some(key);
        st.members.made_at = env.now;
    }

    // the columns: name, title, (roles), status, (donation)
    let width = ui.available_width();
    // (label, sort, fixed width, or share of the rest when the width is 0)
    let mut cols: Vec<(&str, Option<Sort>, f32, f32)> = vec![
        (aurora_i18n::tr!("ui-group-profile-col-member"), Some(Sort::Name), 0.0, 1.5),
        (aurora_i18n::tr!("ui-group-profile-col-title"), Some(Sort::Title), 0.0, 1.0),
    ];
    if manage {
        cols.push((aurora_i18n::tr!("ui-group-profile-col-roles"), None, 0.0, 1.3));
    }
    cols.push((aurora_i18n::tr!("ui-group-profile-col-status"), Some(Sort::Status), 110.0, 0.0));
    if manage {
        cols.push((aurora_i18n::tr!("ui-group-profile-col-donation"), Some(Sort::Donation), 80.0, 0.0));
    }
    let fixed: f32 = cols.iter().map(|c| c.2).sum();
    let shares: f32 = cols.iter().map(|c| c.3).sum();
    let widths: Vec<f32> = cols
        .iter()
        .map(|c| {
            if c.2 > 0.0 {
                c.2
            } else {
                ((width - fixed) * c.3 / shares).max(60.0)
            }
        })
        .collect();

    // the header
    let (head, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
    let mut x = head.left();
    let last = cols.len() - 1;
    for (i, (label, sort, ..)) in cols.iter().enumerate() {
        let w = widths[i];
        let cell = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(w, head.height()));
        let mut job = egui::text::LayoutJob::default();
        let on = *sort == Some(st.members.sort);
        let mut format =
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, if on { p.muted } else { p.muted_dim });
        format.extra_letter_spacing = 0.4;
        let arrow = if on {
            if st.members.reversed { " ▴" } else { " ▾" }
        } else {
            ""
        };
        job.append(&(label.to_uppercase() + arrow), 0.0, format);
        let galley = ui.painter().layout_job(job);
        let right = i == last && manage;
        let pos = if right {
            Pos2::new(cell.right() - 8.0 - galley.size().x, cell.center().y - galley.size().y / 2.0)
        } else {
            Pos2::new(cell.left() + 8.0, cell.center().y - galley.size().y / 2.0)
        };
        ui.painter().galley(pos.round(), galley, p.muted_dim);
        if let Some(sort) = sort {
            let r = ui.interact(cell, ui.id().with(("group_members_head", i)), Sense::click());
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if st.members.sort == *sort {
                    st.members.reversed = !st.members.reversed;
                } else {
                    st.members.sort = *sort;
                    st.members.reversed = false;
                }
            }
        }
        x += w;
    }

    let Some(d) = d else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    let footer_h = 44.0;
    let list_h = (ui.available_height() - footer_h).max(ROW_H * 2.0);
    let today = today();
    let count = st.members.rows.len();
    let mut open_profile = None;
    egui::ScrollArea::vertical()
        .id_salt("group_members_rows")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let (id, name) = st.members.rows[i].clone();
                let Some(m) = d.members.get(&id) else { continue };
                let (row, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                let selected = st.members.selected == Some(id);
                if selected {
                    ui.painter().rect_filled(row, 0.0, p.violet.gamma_multiply(0.16));
                } else if resp.hovered() {
                    ui.painter().rect_filled(row, 0.0, p.raised.gamma_multiply(0.35));
                }
                ui.painter()
                    .hline(row.x_range(), row.top(), egui::Stroke::new(1.0, p.raised.gamma_multiply(0.3)));
                if resp.clicked() {
                    st.members.selected = Some(id);
                }
                if resp.double_clicked() {
                    open_profile = Some(id);
                }
                crate::ui::menu::context_menu(&resp, p, |ui| {
                    context::avatar_list_menu(ui, p, c.world, id, context::AvatarList::Name);
                });
                let mut x = row.left();
                let mut col = 0;
                // member
                let pic = Rect::from_center_size(Pos2::new(x + 8.0 + 10.0, row.center().y), Vec2::splat(20.0));
                avatar(st, ui, c, env, pic, id, &name, m.last_seen == LastSeen::Online);
                widgets::paint_elided(
                    ui,
                    &name,
                    Pos2::new(pic.right() + 8.0, row.center().y),
                    widths[col] - 44.0,
                    FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                    p.ink,
                );
                x += widths[col];
                col += 1;
                // title
                widgets::paint_elided(
                    ui,
                    &m.title,
                    Pos2::new(x + 8.0, row.center().y),
                    widths[col] - 16.0,
                    FontId::proportional(12.5),
                    p.muted,
                );
                x += widths[col];
                col += 1;
                if manage {
                    let cell = Rect::from_min_max(Pos2::new(x + 8.0, row.top()), Pos2::new(x + widths[col] - 4.0, row.bottom()));
                    let mut chips = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(cell)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    chips.set_clip_rect(cell.intersect(ui.clip_rect()));
                    chips.spacing_mut().item_spacing.x = 3.0;
                    let roles = d.roles_of(&id);
                    for r in roles.iter().filter(|r| !r.id.is_nil() || roles.len() == 1) {
                        let chip = match BuiltinRole::of(&r.name) {
                            Some(BuiltinRole::Owners) => Chip::Owner,
                            Some(BuiltinRole::Officers) => Chip::Officer,
                            _ => Chip::Plain,
                        };
                        view::role_chip(&mut chips, p, &role_name(&r.name), chip);
                    }
                    x += widths[col];
                    col += 1;
                }
                let online = m.last_seen == LastSeen::Online;
                let status = status_text(&m.last_seen, today);
                widgets::paint_elided(
                    ui,
                    &status,
                    Pos2::new(x + 8.0, row.center().y),
                    widths[col] - 16.0,
                    FontId::proportional(12.5),
                    if online { p.success } else { p.muted },
                );
                x += widths[col];
                col += 1;
                if manage {
                    let text = if m.contribution > 0 {
                        aurora_i18n::tr!("ui-group-profile-area", area = aurora_i18n::integer(i64::from(m.contribution)))
                    } else {
                        "0".to_owned()
                    };
                    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(12.5), p.muted);
                    let pos = Pos2::new(x + widths[col] - 8.0 - galley.size().x, row.center().y - galley.size().y / 2.0);
                    ui.painter().galley(pos.round(), galley, p.muted);
                }
            }
        });
    if count == 0 {
        let text = if d.members_complete {
            aurora_i18n::tr!("ui-group-profile-no-member-found")
        } else {
            aurora_i18n::tr!("common-loading-2")
        };
        ui.label(egui::RichText::new(text).size(12.5).color(p.muted));
    }
    if let Some(id) = open_profile {
        context::request(ui.ctx(), CtxAction::Profile(id));
    }

    // the selection's buttons
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let selected = st.members.selected.filter(|id| d.members.contains_key(id));
        let text = if selected.is_some() {
            aurora_i18n::tr!("ui-group-profile-one-selected").to_owned()
        } else {
            aurora_i18n::tr!(
                "ui-group-profile-shown",
                count = count as i64,
                shown = aurora_i18n::integer(count as i64)
            )
        };
        ui.label(egui::RichText::new(text).size(12.0).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let size = Vec2::new(0.0, 28.0);
            let me = selected == Some(c.world.agent_id);
            if widgets::action_button_if(
                ui,
                p,
                "user-filled",
                aurora_i18n::tr!("common-profile"),
                ActionKind::Flat,
                size,
                selected.is_some(),
            )
            .clicked()
                && let Some(id) = selected
            {
                context::request(ui.ctx(), CtxAction::Profile(id));
            }
            if widgets::action_button_if(
                ui,
                p,
                "message-filled",
                aurora_i18n::tr!("ui-group-profile-conversation"),
                ActionKind::Flat,
                size,
                selected.is_some() && !me,
            )
            .clicked()
                && let Some(id) = selected
            {
                context::request(ui.ctx(), CtxAction::Im(id));
            }
        });
    });
}

/// The selected member's roles, read only: "everyone" for all, the owners'
/// locked.
fn roles_card(st: &GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, p: &Palette) {
    let selected = st.members.selected.filter(|id| v.data.is_some_and(|d| d.members.contains_key(id)));
    let title = match selected {
        Some(id) => aurora_i18n::tr!("ui-group-profile-roles-of", name = c.world.social.display_name_of(&id)),
        None => aurora_i18n::tr!("ui-group-profile-roles").to_owned(),
    };
    view::card(ui, p, "user-filled", p.muted_dim, &title, None, |ui| {
        let (Some(id), Some(d)) = (selected, v.data) else {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-profile-pick-a-member"))
                    .size(12.0)
                    .color(p.muted),
            );
            return;
        };
        if !d.role_members_complete {
            ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.0).color(p.muted));
            return;
        }
        let mine: Vec<Uuid> = d.member_roles.get(&id).cloned().unwrap_or_default();
        let mut roles: Vec<_> = d.roles.values().collect();
        roles.sort_by_cached_key(|r| (!r.id.is_nil(), role_name(&r.name).to_lowercase()));
        for r in roles {
            let has = mine.contains(&r.id);
            let owners = v.profile.is_some_and(|g| g.owner_role == r.id && !r.id.is_nil());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.set_min_height(24.0);
                let (icon, color) = if has {
                    ("square-check-filled", p.violet_light)
                } else {
                    ("square", p.muted_dim)
                };
                widgets::glyph(ui, icon, color, 15.0);
                let ink = if has { p.ink } else { p.muted };
                ui.label(egui::RichText::new(role_name(&r.name)).size(12.5).color(ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if r.id.is_nil() {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("ui-group-profile-fixed"))
                                .size(11.0)
                                .color(p.muted_dim),
                        );
                    } else if owners {
                        widgets::glyph(ui, "lock-filled", p.muted_dim, 12.0);
                    }
                });
            });
        }
    });
}

/// The members in the classic look: name, title, status, as the members
/// list of Firestorm's "Members and roles".
pub fn classic_list(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View) {
    let p = c.p;
    widgets::search_field(
        ui,
        &mut st.members.filter,
        aurora_i18n::tr!("ui-group-profile-filter-members"),
        ui.available_width(),
    );
    ui.add_space(4.0);
    let Some(d) = v.data else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.0).color(p.muted));
        return;
    };
    let world = c.world;
    let rows = rows(
        v,
        &|id| world.social.display_name_of(id),
        &st.members.filter,
        Show::All,
        Sort::Status,
        false,
    );
    for id in rows.iter().map(|(id, _)| id).filter(|id| world.social.display_name_of(id) == "…") {
        c.want_names.insert(*id);
    }
    let today = today();
    let width = ui.available_width();
    egui::ScrollArea::vertical()
        .id_salt("group_members_classic")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show_rows(ui, 22.0, rows.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (id, name) in &rows[range] {
                let Some(m) = d.members.get(id) else { continue };
                let (row, resp) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(row, 0.0, p.raised.gamma_multiply(0.35));
                }
                if resp.double_clicked() {
                    context::request(ui.ctx(), CtxAction::Profile(*id));
                }
                let third = row.width() / 3.0;
                widgets::paint_elided(
                    ui,
                    name,
                    Pos2::new(row.left() + 4.0, row.center().y),
                    third - 8.0,
                    FontId::proportional(12.0),
                    p.ink,
                );
                widgets::paint_elided(
                    ui,
                    &m.title,
                    Pos2::new(row.left() + third, row.center().y),
                    third - 8.0,
                    FontId::proportional(12.0),
                    p.muted,
                );
                let online = m.last_seen == LastSeen::Online;
                let color = if online { p.success } else { p.muted };
                widgets::paint_elided(
                    ui,
                    &status_text(&m.last_seen, today),
                    Pos2::new(row.left() + 2.0 * third, row.center().y),
                    third - 8.0,
                    FontId::proportional(12.0),
                    color,
                );
            }
        });
}
