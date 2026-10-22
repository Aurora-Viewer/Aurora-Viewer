//! The Members tab, read only: the list of LLPanelGroupMembersSubTab
//! (name, title, status, and for those who manage members their roles and
//! donation), its filter, the selected member's roles beside it, and their
//! profile or a conversation. Inviting, ejecting, banning and changing
//! roles belong to the members' management.

use super::GroupAction;
use super::admin_members::{self, Me};
use super::view::{self, Chip, View};
use super::{Env, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::context::{self, CtxAction};
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind};
use aurora_net::groups::{BuiltinRole, GroupMember, LastSeen, RoleChange, powers};
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
    /// The members ticked for the management's actions (with the abilities).
    pub checked: Vec<Uuid>,
    /// Role changes waiting for "Save".
    pub staged: Vec<RoleChange>,
    /// A management action waiting for its confirmation.
    pub confirm: Option<Confirm>,
    /// The ability sets unfolded in "What they can do".
    pub open_sets: std::collections::HashSet<usize>,
    /// The rows, filtered and sorted, with the names they were sorted by.
    rows: Vec<(Uuid, String)>,
    /// What the rows were made from, and when.
    made: Option<(String, Show, Sort, bool, usize, bool)>,
    made_at: f64,
}

/// A management action to confirm (EjectGroupMember(s)Warning,
/// BanGroupMember(s)Warning, AddGroupOwnerWarning).
#[derive(Clone, Debug, PartialEq)]
pub enum Confirm {
    Eject(Vec<Uuid>),
    Ban(Vec<Uuid>),
    AddOwners,
}

impl MembersState {
    /// The members the actions apply to: those ticked, else the one chosen.
    pub fn targets(&self) -> Vec<Uuid> {
        if self.checked.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.checked.clone()
        }
    }
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
pub(super) fn pill(ui: &mut egui::Ui, p: &Palette, label: &str, on: bool) -> egui::Response {
    widgets::filter_pill(ui, p, "", label, on)
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

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View, actions: &mut Vec<GroupAction>) {
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
    egui::ScrollArea::vertical()
        .id_salt("group_members_side")
        .auto_shrink([false, false])
        .show(&mut s, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            roles_card(st, ui, c, v, p, actions);
            abilities_card(st, ui, c, v, p);
        });
    confirm_dialog(st, ui.ctx(), c, v, actions);
}

/// Who we are, for the management's rules.
fn me(c: &Ctx, v: &View) -> Me {
    Me {
        id: c.world.agent_id,
        powers: v.powers(),
        owner_role: v.profile.map(|g| g.owner_role).unwrap_or_default(),
    }
}

/// The confirmations of Firestorm, then the action.
fn confirm_dialog(st: &mut GroupProfileUi, ctx: &egui::Context, c: &Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let Some(confirm) = st.members.confirm.clone() else {
        return;
    };
    let p = c.p;
    let name = |ids: &[Uuid]| c.world.social.name_of(&ids[0]);
    let (text, ok, icon) = match &confirm {
        Confirm::Eject(ids) if ids.len() == 1 => (
            aurora_i18n::tr!("ui-group-admin-eject-one", name = name(ids)),
            aurora_i18n::tr!("ui-group-admin-eject"),
            "user-x",
        ),
        Confirm::Eject(ids) => (
            aurora_i18n::tr!("ui-group-admin-eject-many", count = ids.len() as i64),
            aurora_i18n::tr!("ui-group-admin-eject"),
            "user-x",
        ),
        Confirm::Ban(ids) if ids.len() == 1 => (
            aurora_i18n::tr!("ui-group-admin-ban-one", name = name(ids)),
            aurora_i18n::tr!("ui-group-admin-ban"),
            "ban",
        ),
        Confirm::Ban(ids) => (
            aurora_i18n::tr!("ui-group-admin-ban-many", count = ids.len() as i64),
            aurora_i18n::tr!("ui-group-admin-ban"),
            "ban",
        ),
        Confirm::AddOwners => {
            let owners = v
                .data
                .and_then(|d| v.profile.and_then(|g| d.roles.get(&g.owner_role)))
                .map(|r| role_name(&r.name))
                .unwrap_or_default();
            (
                aurora_i18n::tr!("ui-group-admin-add-owners", role = owners),
                aurora_i18n::tr!("common-yes"),
                "crown",
            )
        }
    };
    let mut done: Option<bool> = None;
    let modal = widgets::Modal::new(egui::Id::unique("group_members_confirm")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(&text).size(12.5).color(p.ink));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if widgets::action_button(ui, p, icon, ok, ActionKind::Danger, size).clicked() {
                done = Some(true);
            }
            if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                done = Some(false);
            }
        });
    });
    if modal.should_close() && done.is_none() {
        done = Some(false);
    }
    let Some(yes) = done else { return };
    st.members.confirm = None;
    if !yes {
        return;
    }
    match confirm {
        Confirm::Eject(members) => {
            st.members.checked.clear();
            actions.push(GroupAction::Eject { group: v.id, members });
        }
        Confirm::Ban(members) => {
            st.members.checked.clear();
            actions.push(GroupAction::Ban { group: v.id, members });
        }
        Confirm::AddOwners => {
            let changes = std::mem::take(&mut st.members.staged);
            actions.push(GroupAction::ChangeRoles { group: v.id, changes });
        }
    }
}

/// The bar, the table, the selection's buttons.
fn list(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &View, manage: bool) {
    let p = c.p;
    let mut invite = false;
    let mut export = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        // room for the pills, Invite and Export
        let w = (ui.available_width() - 350.0).max(120.0);
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
        };
        let by_role = widgets::filter_pill(ui, p, "crown", &role_label, matches!(st.members.show, Show::Role(_)));
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
        // Firestorm's "Export List" (FIRE-12276), with "visible in the directory"
        if widgets::icon_button(
            ui,
            p,
            "download-filled",
            aurora_i18n::tr!("ui-group-admin-export"),
            v.can(powers::MEMBER_VISIBLE_IN_DIR),
        ) {
            export = true;
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
    if export && let Some(d) = v.data {
        let world = c.world;
        let csv = admin_members::export_csv(&v.name, d, &|id| world.social.name_of(id));
        let file = format!(
            "{}_members.csv",
            v.name.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
        );
        let title = aurora_i18n::tr!("ui-group-admin-export");
        // the save dialog off the frame, as the other file pickers
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(title)
                .set_file_name(&file)
                .add_filter("CSV", &["csv"])
                .save_file()
            {
                match std::fs::write(&path, csv) {
                    Ok(()) => log::info!("group members exported to {}", path.display()),
                    Err(e) => log::warn!("group members export: {e}"),
                }
            }
        });
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

    // the columns: name, title, (roles), status, (donation), left of the
    // list's scroll bar
    let width = widgets::scrolled_width(ui);
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

    // the ticks of the management, then the columns
    let tick_w = if manage { 26.0 } else { 0.0 };
    let width = width - tick_w;
    let widths: Vec<f32> = widths.iter().map(|w| w * width / (width + tick_w)).collect();
    let (head, _) = ui.allocate_exact_size(Vec2::new(width + tick_w, 22.0), Sense::hover());
    let mut x = head.left() + tick_w;
    let last = cols.len() - 1;
    for (i, (label, sort, ..)) in cols.iter().enumerate() {
        let w = widths[i];
        let cell = Rect::from_min_size(Pos2::new(x, head.top()), Vec2::new(w, head.height()));
        let mut job = egui::text::LayoutJob::default();
        let on = *sort == Some(st.members.sort);
        let mut format =
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, if on { p.muted } else { p.muted_dim });
        format.extra_letter_spacing = 0.4;
        job.append(&label.to_uppercase(), 0.0, format);
        let galley = ui.painter().layout_job(job);
        const CARET: f32 = 9.0;
        let caret_w = if on { widgets::caret_room(CARET) } else { 0.0 };
        let right = i == last && manage;
        let pos = if right {
            Pos2::new(
                cell.right() - 8.0 - galley.size().x - caret_w,
                cell.center().y - galley.size().y / 2.0,
            )
        } else {
            Pos2::new(cell.left() + 8.0, cell.center().y - galley.size().y / 2.0)
        };
        if on {
            let center = Pos2::new(pos.x + galley.size().x + caret_w - CARET / 2.0, cell.center().y);
            widgets::paint_glyph(ui.painter(), widgets::sort_caret(st.members.reversed), center, CARET, p.muted);
        }
        ui.painter().galley(pos.round(), galley, p.muted_dim);
        if let Some(sort) = sort {
            let r = ui.interact(cell, ui.scope_id().with(("group_members_head", i)), Sense::click());
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
                let (row, resp) = ui.allocate_exact_size(Vec2::new(width + tick_w, ROW_H), Sense::click());
                let selected = st.members.selected == Some(id) || st.members.checked.contains(&id);
                if selected {
                    ui.painter().rect_filled(row, 0.0, p.violet.gamma_multiply(0.16));
                } else if resp.hovered() {
                    ui.painter().rect_filled(row, 0.0, p.raised.gamma_multiply(0.35));
                }
                ui.painter()
                    .hline(row.x_range(), row.top(), egui::Stroke::new(1.0, p.raised.gamma_multiply(0.3)));
                if resp.clicked() {
                    st.members.selected = Some(id);
                    // Ctrl adds to the ticks, a plain click starts them again
                    if manage {
                        if ui.input(|i| i.modifiers.command) {
                            toggle(&mut st.members.checked, id);
                        } else {
                            st.members.checked = vec![id];
                        }
                    }
                }
                if manage {
                    let tick = Rect::from_min_size(row.min, Vec2::new(tick_w, ROW_H));
                    let on = st.members.checked.contains(&id);
                    let (icon, color) = if on {
                        ("square-check-filled", p.violet_light)
                    } else {
                        ("square", p.muted_dim)
                    };
                    if let Some(t) = crate::ui::icons::global(icon) {
                        ui.painter().image(
                            t.id(),
                            Rect::from_center_size(tick.center(), Vec2::splat(15.0)),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            color,
                        );
                    }
                    if ui
                        .interact(tick, ui.scope_id().with(("group_member_tick", id)), Sense::click())
                        .clicked()
                    {
                        toggle(&mut st.members.checked, id);
                        st.members.selected = Some(id);
                    }
                }
                if resp.double_clicked() {
                    open_profile = Some(id);
                }
                crate::ui::menu::context_menu(&resp, p, |ui| {
                    context::avatar_list_menu(ui, p, c.world, id, context::AvatarList::Name);
                });
                let mut x = row.left() + tick_w;
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
                    let pos =
                        crate::ui::widgets::row_text_origin(ui.painter(), &galley, x + widths[col] - 8.0 - galley.size().x, row.center().y);
                    ui.painter().galley(pos, galley, p.muted);
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
    st.members.checked.retain(|id| d.members.contains_key(id));
    let targets: Vec<Uuid> = st.members.targets().into_iter().filter(|id| d.members.contains_key(id)).collect();
    let (can_eject, can_ban) = admin_members::eject_and_ban(d, &me(c, v), &targets);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let selected = (targets.len() == 1).then(|| targets[0]);
        let text = if !targets.is_empty() {
            aurora_i18n::tr!("ui-group-admin-selected", count = targets.len() as i64)
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
            // Eject / Ban with the abilities (handleMemberSelect's rules)
            if v.can(powers::GROUP_BAN_ACCESS)
                && widgets::action_button_if(
                    ui,
                    p,
                    "ban",
                    aurora_i18n::tr!("ui-group-admin-ban"),
                    ActionKind::SoftDanger,
                    size,
                    can_ban,
                )
                .clicked()
            {
                st.members.confirm = Some(Confirm::Ban(targets.clone()));
            }
            if (v.can(powers::MEMBER_EJECT) || v.can(powers::ROLE_REMOVE_MEMBER))
                && widgets::action_button_if(
                    ui,
                    p,
                    "user-x",
                    aurora_i18n::tr!("ui-group-admin-eject"),
                    ActionKind::SoftDanger,
                    size,
                    can_eject,
                )
                .clicked()
            {
                st.members.confirm = Some(Confirm::Eject(targets.clone()));
            }
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
                "message-circle-filled",
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

/// The chosen members' roles: ticked as they are (and as the waiting
/// changes make them), changed with the abilities, saved together;
/// "everyone" for all, the owners' locked when we may not change it.
fn roles_card(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, p: &Palette, actions: &mut Vec<GroupAction>) {
    let targets: Vec<Uuid> = st
        .members
        .targets()
        .into_iter()
        .filter(|id| v.data.is_some_and(|d| d.members.contains_key(id)))
        .collect();
    if targets.len() > 1 {
        let title = aurora_i18n::tr!("ui-group-admin-roles-of-many", count = targets.len() as i64);
        roles_body(st, ui, c, v, p, &targets, &title, actions);
        return;
    }
    let selected = targets.first().copied();
    let title = match selected {
        Some(id) => aurora_i18n::tr!("ui-group-profile-roles-of", name = c.world.social.display_name_of(&id)),
        None => aurora_i18n::tr!("ui-group-profile-roles").to_owned(),
    };
    if selected.is_some() {
        roles_body(st, ui, c, v, p, &targets, &title, actions);
        return;
    }
    view::card(ui, p, "user-filled", p.muted_dim, &title, None, |ui| {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-profile-pick-a-member"))
                .size(12.0)
                .color(p.muted),
        );
    });
}

fn toggle(list: &mut Vec<Uuid>, id: Uuid) {
    if let Some(i) = list.iter().position(|x| *x == id) {
        list.remove(i);
    } else {
        list.push(id);
    }
}

/// The roles of the chosen, with their ticks, and Save / Cancel while
/// changes wait.
#[allow(clippy::too_many_arguments)]
fn roles_body(
    st: &mut GroupProfileUi,
    ui: &mut egui::Ui,
    c: &mut Ctx,
    v: &View,
    p: &Palette,
    targets: &[Uuid],
    title: &str,
    actions: &mut Vec<GroupAction>,
) {
    let me = me(c, v);
    view::card(ui, p, "user-filled", p.muted_dim, title, None, |ui| {
        let Some(d) = v.data.filter(|d| d.role_members_complete) else {
            ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.0).color(p.muted));
            return;
        };
        // changes for members no longer chosen are forgotten
        st.members.staged.retain(|ch| targets.contains(&ch.member));
        let checks = admin_members::role_checks(d, &me, targets, &st.members.staged);
        let mut clicked = None;
        for ch in &checks {
            let Some(r) = d.roles.get(&ch.role) else { continue };
            let all = ch.count == targets.len() && ch.count > 0;
            let some = ch.count > 0 && !all;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.set_min_height(24.0);
                let (icon, color) = if all {
                    ("square-check-filled", p.violet_light)
                } else if some {
                    ("square-minus-filled", p.violet_light)
                } else {
                    ("square", p.muted_dim)
                };
                let alpha = if ch.enabled || r.id.is_nil() { 1.0 } else { 0.45 };
                let (rect, resp) = ui.allocate_exact_size(Vec2::splat(15.0), if ch.enabled { Sense::click() } else { Sense::hover() });
                if let Some(t) = crate::ui::icons::global(icon) {
                    ui.painter().image(
                        t.id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        color.gamma_multiply(alpha),
                    );
                }
                let ink = if ch.count > 0 { p.ink } else { p.muted };
                let mut label = role_name(&r.name);
                // "Don't bother showing a count, if there is only 0 or 1."
                if ch.count > 1 {
                    label = format!("{label}: {}", ch.count);
                }
                let text = ui.add(
                    egui::Label::new(egui::RichText::new(label).size(12.5).color(ink.gamma_multiply(alpha))).sense(if ch.enabled {
                        Sense::click()
                    } else {
                        Sense::hover()
                    }),
                );
                if ch.enabled && (resp.clicked() || text.clicked()) {
                    clicked = Some(r.id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if r.id.is_nil() {
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("ui-group-profile-fixed"))
                                .size(11.0)
                                .color(p.muted_dim),
                        );
                    } else if !ch.enabled {
                        widgets::glyph(ui, "lock-filled", p.muted_dim, 12.0);
                    }
                });
            });
        }
        if let Some(role) = clicked {
            admin_members::toggle_role(d, targets, &mut st.members.staged, role);
        }
        if !st.members.staged.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let size = Vec2::new(0.0, 26.0);
                    if widgets::action_button(
                        ui,
                        p,
                        "device-floppy-filled",
                        aurora_i18n::tr!("ui-group-admin-save"),
                        ActionKind::Primary,
                        size,
                    )
                    .clicked()
                    {
                        // AddGroupOwnerWarning first when owners are added
                        let owners = me.owner_role;
                        if st.members.staged.iter().any(|ch| ch.add && ch.role == owners && !owners.is_nil()) {
                            st.members.confirm = Some(Confirm::AddOwners);
                        } else {
                            let changes = std::mem::take(&mut st.members.staged);
                            actions.push(GroupAction::ChangeRoles { group: v.id, changes });
                        }
                    }
                    if widgets::action_button(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        st.members.staged.clear();
                    }
                });
            });
        }
    });
}

/// "What they can do": the abilities of the chosen through their roles, by
/// set (buildActionsList with the allowed-by-some powers).
fn abilities_card(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, p: &Palette) {
    let Some(d) = v.data.filter(|d| d.role_members_complete) else {
        return;
    };
    let targets: Vec<Uuid> = st.members.targets().into_iter().filter(|id| d.members.contains_key(id)).collect();
    if targets.is_empty() {
        return;
    }
    let (some, all) = admin_members::powers_of(d, &targets, &st.members.staged);
    let catalogue = super::abilities::catalogue();
    let total: usize = catalogue.iter().map(|s| admin_members::set_count(s, some).0).sum();
    let title = aurora_i18n::tr!(
        "ui-group-admin-can-do",
        count = targets.len() as i64,
        shown = aurora_i18n::integer(total as i64)
    );
    let _ = c;
    view::card(ui, p, "key-filled", p.muted_dim, &title, None, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, set) in catalogue.iter().enumerate() {
            let (has, of) = admin_members::set_count(set, some);
            if has == 0 {
                continue;
            }
            let open = st.members.open_sets.contains(&i);
            ui.painter().hline(
                ui.available_rect_before_wrap().x_range(),
                ui.cursor().top(),
                egui::Stroke::new(1.0, p.raised.gamma_multiply(0.3)),
            );
            ui.add_space(4.0);
            let head = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    widgets::glyph(ui, set.icon, p.violet_pale, 13.0);
                    ui.label(
                        egui::RichText::new(set.name)
                            .font(FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx())))
                            .color(p.violet_pale),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        widgets::glyph(ui, widgets::disclosure_caret(open), p.muted_dim, 11.0);
                        ui.label(egui::RichText::new(format!("{has} / {of}")).size(11.0).color(p.muted_dim));
                    });
                })
                .response
                .interact(Sense::click())
                .on_hover_text(set.help);
            if head.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if open {
                    st.members.open_sets.remove(&i);
                } else {
                    st.members.open_sets.insert(i);
                }
            }
            if open {
                for a in set.abilities.iter().filter(|a| some & (1u64 << a.bit) != 0) {
                    // an ability only some of the chosen have is dimmed
                    let ink = if all & (1u64 << a.bit) != 0 { p.muted } else { p.muted_dim };
                    ui.add(egui::Label::new(egui::RichText::new(a.name).size(12.0).color(ink)).wrap())
                        .on_hover_text(a.help);
                }
            }
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-admin-abilities-note"))
                .size(11.5)
                .color(p.muted_dim),
        );
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
    let width = widgets::scrolled_width(ui);
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
