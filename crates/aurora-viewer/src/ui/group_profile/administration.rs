//! The Administration tab: everything that changes the group, each section
//! shown with its abilities (Firestorm's "Members & Roles" › Roles,
//! Abilities and Banned Residents sub-tabs and the editable "General"
//! panel: llpanelgrouproles.cpp, llpanelgroupgeneral.cpp,
//! llpanelgroupbulkban.cpp, originally LGPL 2.1):
//! - Roles: create (MAX_ROLES), copy, delete, name, title, description,
//!   members visible, abilities, the changes saved together
//!   (LLGroupMgrGroupData::setRoleData / sendRoleChanges), with the
//!   warnings of the dangerous abilities;
//! - Group identity: insignia, charter, show in search, content rating,
//!   open enrollment and fee (UpdateGroupInfo);
//! - Abilities: who has which ability, by role and member;
//! - Banned residents: the list (GroupAPIv1), unban, ban by name;
//! - Invitations: the invitation window.

use super::abilities::{self, AbilitySet};
use super::members::role_name;
use super::view::{self, View};
use super::{GroupAction, GroupProfileUi};
use crate::theme::Palette;
use crate::ui::avatar_picker::AvatarPicker;
use crate::ui::place_details::Ctx;
use crate::ui::texture_picker::TexturePicker;
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::group_data::GroupData;
use aurora_net::groups::{GroupInfo, GroupRole, RoleUpdate, powers, role_change};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// MAX_ROLES (roles_constants.h): "everyone" included.
pub const MAX_ROLES: usize = 10;

/// The abilities that open the tab.
pub const ADMIN: u64 = powers::ROLE_CREATE
    | powers::ROLE_DELETE
    | powers::ROLE_PROPERTIES
    | powers::ROLE_CHANGE_ACTIONS
    | powers::GROUP_CHANGE_IDENTITY
    | powers::MEMBER_OPTIONS
    | powers::GROUP_BAN_ACCESS
    | powers::MEMBER_INVITE;

const ROLES: u64 = powers::ROLE_CREATE | powers::ROLE_DELETE | powers::ROLE_PROPERTIES | powers::ROLE_CHANGE_ACTIONS;
const IDENTITY: u64 = powers::GROUP_CHANGE_IDENTITY | powers::MEMBER_OPTIONS;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Section {
    #[default]
    Roles,
    Identity,
    Abilities,
    Bans,
    Invitations,
}

/// A dangerous ability waiting for its confirmation, or a warning about
/// the ban ability (AssignDangerousActionWarning,
/// AssignDangerousAbilityWarning, AssignBanAbilityWarning,
/// RemoveBanAbilityWarning).
#[derive(Clone, Debug, PartialEq)]
enum Dialog {
    Dangerous { role: Uuid, bit: u32 },
    BanGiven { role: Uuid },
    BanTaken { role: Uuid },
}

#[derive(Default)]
pub struct AdminState {
    pub section: Section,
    pub role: Option<Uuid>,
    /// The role changes waiting for "Save" (mRoleChanges).
    pub changes: BTreeMap<Uuid, RoleUpdate>,
    dialog: Option<Dialog>,
    open_sets: HashSet<usize>,
    /// The group's identity being edited.
    pub identity: Option<GroupInfo>,
    /// The ability chosen in "Abilities".
    ability: Option<u32>,
    bans_asked: Option<Uuid>,
    bans_checked: Vec<Uuid>,
    picker: AvatarPicker,
    texture: TexturePicker,
    /// Residents chosen in the picker, banned at the next frame.
    to_ban: Vec<Uuid>,
}

/// The tab shows with one of the administration abilities.
pub fn visible(v: &View) -> bool {
    v.member.is_some() && v.can(ADMIN)
}

/// A role as it would be after the changes waiting.
pub fn role_now(d: &GroupData, changes: &BTreeMap<Uuid, RoleUpdate>, id: &Uuid) -> Option<GroupRole> {
    match changes.get(id) {
        Some(u) if u.kind == role_change::DELETE => None,
        Some(u) => Some(GroupRole {
            id: *id,
            name: u.name.clone(),
            title: u.title.clone(),
            description: u.description.clone(),
            powers: u.powers,
            members: d.roles.get(id).map_or(0, |r| r.members),
        }),
        None => d.roles.get(id).cloned(),
    }
}

/// The roles shown: the group's and the new ones, without the deleted;
/// "everyone" first, the owners' next.
pub fn roles_now(d: &GroupData, changes: &BTreeMap<Uuid, RoleUpdate>, owner_role: Uuid) -> Vec<GroupRole> {
    let mut ids: Vec<Uuid> = d.roles.keys().copied().collect();
    ids.extend(changes.keys().filter(|id| !d.roles.contains_key(id)));
    let mut v: Vec<GroupRole> = ids.iter().filter_map(|id| role_now(d, changes, id)).collect();
    v.sort_by_cached_key(|r| (!r.id.is_nil(), r.id != owner_role, role_name(&r.name).to_lowercase()));
    v
}

/// LLGroupMgrGroupData::setRoleData: a role's new values become a change of
/// the right kind, or no change when they are back to the saved ones; a new
/// role stays a creation.
pub fn set_role(d: &GroupData, changes: &mut BTreeMap<Uuid, RoleUpdate>, role: GroupRole) {
    if let Some(c) = changes.get(&role.id) {
        if c.kind == role_change::CREATE {
            changes.insert(role.id, update(&role, role_change::CREATE));
            return;
        }
        if c.kind == role_change::DELETE {
            return;
        }
    }
    let Some(old) = d.roles.get(&role.id) else { return };
    let data = old.name != role.name || old.title != role.title || old.description != role.description;
    let powers = old.powers != role.powers;
    let kind = match (data, powers) {
        (false, false) => {
            changes.remove(&role.id);
            return;
        }
        (true, true) => role_change::ALL,
        (true, false) => role_change::DATA,
        (false, true) => role_change::POWERS,
    };
    changes.insert(role.id, update(&role, kind));
}

fn update(r: &GroupRole, kind: u8) -> RoleUpdate {
    RoleUpdate {
        role: r.id,
        name: r.name.clone(),
        title: r.title.clone(),
        description: r.description.clone(),
        powers: r.powers,
        kind,
    }
}

/// handleCreateRole / handleCopyRole: a new role ("New Role", or the copy
/// with "(Copy)"), while there is room (MAX_ROLES).
pub fn create_role(d: &GroupData, changes: &mut BTreeMap<Uuid, RoleUpdate>, owner_role: Uuid, from: Option<&GroupRole>) -> Option<Uuid> {
    if roles_now(d, changes, owner_role).len() >= MAX_ROLES {
        return None;
    }
    let id = Uuid::new_v4();
    let role = match from {
        Some(r) => GroupRole {
            id,
            name: format!("{}(Copy)", r.name),
            members: 0,
            ..r.clone()
        },
        None => GroupRole {
            id,
            name: "New Role".into(),
            title: String::new(),
            description: String::new(),
            powers: 0,
            members: 0,
        },
    };
    changes.insert(id, update(&role, role_change::CREATE));
    Some(id)
}

/// handleDeleteRole: never "everyone" nor the owners'; a role created and
/// not saved simply goes.
pub fn delete_role(d: &GroupData, changes: &mut BTreeMap<Uuid, RoleUpdate>, owner_role: Uuid, role: Uuid) -> bool {
    if role.is_nil() || role == owner_role {
        return false;
    }
    if changes.get(&role).is_some_and(|c| c.kind == role_change::CREATE) {
        changes.remove(&role);
        return true;
    }
    let Some(r) = d.roles.get(&role) else { return false };
    changes.insert(role, update(r, role_change::DELETE));
    true
}

/// handleActionCheck: the ban ability brings "eject" and "remove from
/// roles" with it.
pub fn set_power(powers_now: u64, bit: u32, on: bool) -> u64 {
    let p = 1u64 << bit;
    if on {
        let mut v = powers_now | p;
        if p == powers::GROUP_BAN_ACCESS {
            v |= powers::ROLE_REMOVE_MEMBER | powers::MEMBER_EJECT;
        }
        v
    } else {
        powers_now & !p
    }
}

/// The abilities that need a confirmation before they are given.
fn dangerous(bit: u32) -> bool {
    let p = 1u64 << bit;
    p == powers::ROLE_ASSIGN_MEMBER || p == powers::ROLE_CHANGE_ACTIONS
}

pub fn show(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let Some(d) = v.data else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    // the residents chosen to ban (LLPanelGroupBulkBan)
    if !st.admin.to_ban.is_empty() {
        let ids = std::mem::take(&mut st.admin.to_ban);
        actions.push(GroupAction::BanResidents { group: v.id, ids });
    }
    let owner_role = v.profile.map(|g| g.owner_role).unwrap_or_default();
    let sections = [
        (
            Section::Roles,
            "crown",
            aurora_i18n::tr!("ui-group-admin-roles"),
            v.can(ROLES),
            Some(roles_now(d, &st.admin.changes, owner_role).len()),
        ),
        (
            Section::Identity,
            "id-filled",
            aurora_i18n::tr!("ui-group-admin-identity"),
            v.can(IDENTITY),
            None,
        ),
        (
            Section::Abilities,
            "key-filled",
            aurora_i18n::tr!("ui-group-admin-abilities"),
            true,
            None,
        ),
        (
            Section::Bans,
            "ban",
            aurora_i18n::tr!("ui-group-admin-bans"),
            v.can(powers::GROUP_BAN_ACCESS),
            d.bans.as_ref().map(Vec::len),
        ),
        (
            Section::Invitations,
            "user-plus",
            aurora_i18n::tr!("ui-group-admin-invitations"),
            v.can(powers::MEMBER_INVITE),
            None,
        ),
    ];
    // the first section we may open when the chosen one is not ours
    if !sections.iter().any(|s| s.0 == st.admin.section && s.3)
        && let Some(s) = sections.iter().find(|s| s.3)
    {
        st.admin.section = s.0;
    }
    let body = Rect::from_min_max(
        ui.cursor().min + Vec2::new(4.0, 0.0),
        ui.max_rect().right_bottom() - Vec2::new(4.0, 0.0),
    );
    ui.allocate_rect(body, Sense::hover());
    let nav = Rect::from_min_max(body.min, Pos2::new(body.left() + 190.0, body.bottom()));
    let main = Rect::from_min_max(Pos2::new(nav.right() + 12.0, body.top()), body.max);
    let mut n = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(nav)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    n.spacing_mut().item_spacing.y = 2.0;
    for (section, icon, label, shown, count) in sections {
        if shown && nav_item(&mut n, p, icon, label, count, st.admin.section == section).clicked() {
            st.admin.section = section;
        }
    }
    let mut m = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(main)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    m.set_clip_rect(main.intersect(ui.clip_rect()));
    match st.admin.section {
        Section::Roles => roles(st, &mut m, c, v, d, actions),
        Section::Identity => identity(st, &mut m, c, v, actions),
        Section::Abilities => abilities_section(st, &mut m, c, v, d),
        Section::Bans => bans(st, &mut m, c, v, d, actions),
        Section::Invitations => invitations(st, &mut m, c, v),
    }
    dialog(st, ui.ctx(), p, v, d);
}

/// One entry of the section list.
fn nav_item(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, count: Option<usize>, on: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let ink = if on { p.ink } else { p.muted };
    if let Some(t) = crate::ui::icons::global(icon) {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(Pos2::new(rect.left() + 18.0, rect.center().y), Vec2::splat(16.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            ink,
        );
    }
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx()));
    let g = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, rect.left() + 35.0, rect.center().y);
    ui.painter().galley(at, g, ink);
    if let Some(n) = count {
        let g = ui.painter().layout_no_wrap(n.to_string(), FontId::proportional(11.0), p.muted_dim);
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, rect.right() - 10.0 - g.size().x, rect.center().y);
        ui.painter().galley(at, g, p.muted_dim);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A labelled text field of the role editor and the identity.
fn field(ui: &mut egui::Ui, p: &Palette, label: &str, text: &mut String, enabled: bool, limit: usize, multiline: bool) -> bool {
    let mut changed = false;
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(90.0, 24.0), Sense::hover());
        ui.painter().text(
            Pos2::new(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            p.muted,
        );
        ui.add_enabled_ui(enabled, |ui| {
            let edit = if multiline {
                egui::TextEdit::multiline(text).desired_rows(3)
            } else {
                egui::TextEdit::singleline(text)
            };
            changed = ui.add(edit.char_limit(limit).desired_width(f32::INFINITY)).changed();
        });
    });
    changed
}

/// A switch row with an icon.
fn switch_row(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: &mut bool, enabled: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        widgets::glyph(ui, icon, p.muted, 15.0);
        ui.label(egui::RichText::new(label).size(12.5).color(if enabled { p.ink } else { p.muted }));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| changed = widgets::switch(ui, p, on).changed());
        });
    });
    changed
}

fn roles(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, d: &GroupData, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let owner_role = v.profile.map(|g| g.owner_role).unwrap_or_default();
    let list = roles_now(d, &st.admin.changes, owner_role);
    if st.admin.role.is_none_or(|r| !list.iter().any(|x| x.id == r)) {
        st.admin.role = list.get(1).or(list.first()).map(|r| r.id);
    }
    let full = ui.max_rect();
    let left = Rect::from_min_max(ui.cursor().min, Pos2::new(ui.cursor().min.x + 220.0, full.bottom()));
    let right = Rect::from_min_max(Pos2::new(left.right() + 12.0, left.top()), full.right_bottom());
    let mut l = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    l.spacing_mut().item_spacing.y = 2.0;
    for r in &list {
        let members = if r.id.is_nil() { d.member_total.max(0) as u32 } else { r.members };
        let (rect, resp) = l.allocate_exact_size(Vec2::new(l.available_width(), 40.0), Sense::click());
        let on = st.admin.role == Some(r.id);
        let radius = l.visuals().widgets.inactive.corner_radius;
        if on {
            l.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.18));
        } else if resp.hovered() {
            l.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.35));
        }
        let changed = st.admin.changes.contains_key(&r.id);
        let name = if changed {
            format!("{} •", role_name(&r.name))
        } else {
            role_name(&r.name)
        };
        widgets::paint_elided(
            &l,
            &name,
            Pos2::new(rect.left() + 8.0, rect.center().y - 8.0),
            rect.width() - 16.0,
            FontId::new(13.0, crate::ui::fonts::semibold(l.ctx())),
            p.ink,
        );
        let sub = aurora_i18n::tr!(
            "ui-group-profile-members",
            count = i64::from(members),
            shown = aurora_i18n::integer(i64::from(members))
        );
        widgets::paint_elided(
            &l,
            &sub,
            Pos2::new(rect.left() + 8.0, rect.center().y + 9.0),
            rect.width() - 16.0,
            FontId::proportional(11.5),
            p.muted,
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            st.admin.role = Some(r.id);
        }
    }
    l.add_space(6.0);
    l.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let size = Vec2::new(0.0, 26.0);
        let room = list.len() < MAX_ROLES;
        if widgets::action_button_if(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("ui-group-admin-new-role"),
            ActionKind::Flat,
            size,
            room && v.can(powers::ROLE_CREATE),
        )
        .clicked()
        {
            st.admin.role = create_role(d, &mut st.admin.changes, owner_role, None);
        }
        let chosen = st.admin.role.and_then(|r| list.iter().find(|x| x.id == r).cloned());
        let copyable = chosen.as_ref().is_some_and(|r| !r.id.is_nil());
        if widgets::action_button_if(
            ui,
            p,
            "copy-filled",
            aurora_i18n::tr!("ui-group-admin-copy-role"),
            ActionKind::Flat,
            size,
            room && copyable && v.can(powers::ROLE_CREATE),
        )
        .clicked()
        {
            st.admin.role = create_role(d, &mut st.admin.changes, owner_role, chosen.as_ref());
        }
        let deletable = chosen.as_ref().is_some_and(|r| !r.id.is_nil() && r.id != owner_role) && v.can(powers::ROLE_DELETE);
        if widgets::action_button_if(ui, p, "trash-filled", "", ActionKind::SoftDanger, Vec2::new(30.0, 26.0), deletable)
            .on_hover_text(aurora_i18n::tr!("ui-group-admin-delete-role"))
            .clicked()
            && let Some(r) = &chosen
            && delete_role(d, &mut st.admin.changes, owner_role, r.id)
        {
            st.admin.role = None;
        }
    });
    if list.len() >= MAX_ROLES {
        l.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-admin-max-roles", count = MAX_ROLES as i64))
                .size(11.0)
                .color(p.muted_dim),
        );
    }

    let mut r = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    r.set_clip_rect(right.intersect(ui.clip_rect()));
    let Some(role) = st.admin.role.and_then(|id| role_now(d, &st.admin.changes, &id)) else {
        return;
    };
    let is_owner_role = role.id == owner_role && !owner_role.is_nil();
    let props = v.can(powers::ROLE_PROPERTIES);
    let actions_ok = v.can(powers::ROLE_CHANGE_ACTIONS) && !is_owner_role;
    egui::ScrollArea::vertical()
        .id_salt("group_admin_role")
        .max_height((right.height() - 40.0).max(80.0))
        .auto_shrink([false, false])
        .show(&mut r, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let mut edited = role.clone();
            let mut changed = false;
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 6.0;
                    // handleRoleSelect: the owners' and everyone's names are fixed
                    let name_ok = props && !is_owner_role && !role.id.is_nil();
                    // the built-in roles show their translated names (read only)
                    if name_ok {
                        changed |= field(
                            ui,
                            p,
                            aurora_i18n::tr!("ui-group-admin-role-name"),
                            &mut edited.name,
                            true,
                            20,
                            false,
                        );
                    } else {
                        let mut shown = role_name(&edited.name);
                        field(ui, p, aurora_i18n::tr!("ui-group-admin-role-name"), &mut shown, false, 20, false);
                    }
                    changed |= field(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-group-admin-role-title"),
                        &mut edited.title,
                        props,
                        20,
                        false,
                    );
                    changed |= field(
                        ui,
                        p,
                        aurora_i18n::tr!("ui-group-admin-role-description"),
                        &mut edited.description,
                        props && !role.id.is_nil(),
                        254,
                        true,
                    );
                    let mut visible = edited.powers & powers::MEMBER_VISIBLE_IN_DIR != 0;
                    if switch_row(
                        ui,
                        p,
                        "eye-filled",
                        aurora_i18n::tr!("ui-group-admin-role-visible"),
                        &mut visible,
                        props && !is_owner_role,
                    ) {
                        edited.powers = set_power(edited.powers, powers::MEMBER_VISIBLE_IN_DIR.trailing_zeros(), visible);
                        changed = true;
                    }
                });
            // the abilities, by set
            let catalogue = abilities::catalogue();
            let total: usize = catalogue.iter().map(|s| s.abilities.len()).sum();
            let has: usize = catalogue
                .iter()
                .flat_map(|s| s.abilities.iter())
                .filter(|a| edited.powers & (1u64 << a.bit) != 0)
                .count();
            let title = aurora_i18n::tr!("ui-group-admin-role-abilities", count = has as i64, total = total as i64);
            view::card(ui, p, "key-filled", p.muted_dim, &title, None, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (i, set) in catalogue.iter().enumerate() {
                    let on_set = set.abilities.iter().filter(|a| edited.powers & (1u64 << a.bit) != 0).count();
                    let open = st.admin.open_sets.contains(&i);
                    if set_head(ui, p, set, on_set, open).clicked() {
                        if open {
                            st.admin.open_sets.remove(&i);
                        } else {
                            st.admin.open_sets.insert(i);
                        }
                    }
                    if open {
                        for a in &set.abilities {
                            let mut on = edited.powers & (1u64 << a.bit) != 0;
                            let resp = ui
                                .horizontal(|ui| {
                                    ui.set_min_height(24.0);
                                    ui.label(egui::RichText::new(a.name).size(12.0).color(p.muted))
                                        .on_hover_text(a.help);
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add_enabled_ui(actions_ok, |ui| widgets::switch(ui, p, &mut on).changed()).inner
                                    })
                                    .inner
                                })
                                .inner;
                            if resp {
                                if on && dangerous(a.bit) {
                                    st.admin.dialog = Some(Dialog::Dangerous { role: role.id, bit: a.bit });
                                } else {
                                    if 1u64 << a.bit == powers::GROUP_BAN_ACCESS {
                                        st.admin.dialog = Some(if on {
                                            Dialog::BanGiven { role: role.id }
                                        } else {
                                            Dialog::BanTaken { role: role.id }
                                        });
                                    }
                                    edited.powers = set_power(edited.powers, a.bit, on);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            });
            if changed {
                set_role(d, &mut st.admin.changes, edited);
            }
        });
    // Save / Cancel for every change waiting (apply / cancel)
    r.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let size = Vec2::new(0.0, 28.0);
            let waiting = !st.admin.changes.is_empty();
            if widgets::action_button_if(
                ui,
                p,
                "device-floppy-filled",
                aurora_i18n::tr!("ui-group-admin-save"),
                ActionKind::Primary,
                size,
                waiting,
            )
            .clicked()
            {
                let updates: Vec<RoleUpdate> = std::mem::take(&mut st.admin.changes).into_values().collect();
                actions.push(GroupAction::UpdateRoles { group: v.id, updates });
            }
            if widgets::action_button_if(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, waiting).clicked() {
                st.admin.changes.clear();
            }
        });
    });
}

/// The head of an ability set: its icon, name, how many it has.
fn set_head(ui: &mut egui::Ui, p: &Palette, set: &AbilitySet, on: usize, open: bool) -> egui::Response {
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
                ui.label(
                    egui::RichText::new(format!("{on} / {}", set.abilities.len()))
                        .size(11.0)
                        .color(p.muted_dim),
                );
            });
        })
        .response
        .interact(Sense::click())
        .on_hover_text(set.help);
    ui.add_space(2.0);
    head.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The confirmations and warnings of the abilities (handleActionCheck).
fn dialog(st: &mut GroupProfileUi, ctx: &egui::Context, p: &Palette, v: &View, d: &GroupData) {
    let Some(dlg) = st.admin.dialog.clone() else { return };
    let role = match &dlg {
        Dialog::Dangerous { role, .. } | Dialog::BanGiven { role } | Dialog::BanTaken { role } => *role,
    };
    let role_label = role_now(d, &st.admin.changes, &role)
        .map(|r| role_name(&r.name))
        .unwrap_or_default();
    let catalogue = abilities::catalogue();
    let ability = |bit: u32| {
        catalogue
            .iter()
            .flat_map(|s| s.abilities.iter())
            .find(|a| a.bit == bit)
            .map_or("", |a| a.name)
    };
    let (text, confirm) = match &dlg {
        Dialog::Dangerous { bit, .. } if 1u64 << bit == powers::ROLE_CHANGE_ACTIONS => (
            aurora_i18n::tr!("ui-group-admin-dangerous-ability", action = ability(*bit), role = role_label),
            true,
        ),
        Dialog::Dangerous { bit, .. } => (
            aurora_i18n::tr!("ui-group-admin-dangerous-action", action = ability(*bit), role = role_label),
            true,
        ),
        Dialog::BanGiven { .. } => (
            aurora_i18n::tr!(
                "ui-group-admin-ban-ability-given",
                action = ability(powers::GROUP_BAN_ACCESS.trailing_zeros()),
                role = role_label,
                eject = ability(powers::MEMBER_EJECT.trailing_zeros()),
                remove = ability(powers::ROLE_REMOVE_MEMBER.trailing_zeros())
            ),
            false,
        ),
        Dialog::BanTaken { .. } => (
            aurora_i18n::tr!(
                "ui-group-admin-ban-ability-taken",
                action = ability(powers::GROUP_BAN_ACCESS.trailing_zeros()),
                role = role_label,
                eject = ability(powers::MEMBER_EJECT.trailing_zeros()),
                remove = ability(powers::ROLE_REMOVE_MEMBER.trailing_zeros())
            ),
            false,
        ),
    };
    let mut done: Option<bool> = None;
    let modal = widgets::Modal::new(egui::Id::unique("group_admin_dialog")).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.label(egui::RichText::new(&text).size(12.5).color(p.ink));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if confirm {
                if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-yes"), ActionKind::Danger, size).clicked() {
                    done = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    done = Some(false);
                }
            } else if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() {
                done = Some(false);
            }
        });
    });
    if modal.should_close() && done.is_none() {
        done = Some(false);
    }
    let Some(yes) = done else { return };
    st.admin.dialog = None;
    if let (true, Dialog::Dangerous { role, bit }) = (yes, dlg)
        && let Some(mut r) = role_now(d, &st.admin.changes, &role)
    {
        r.powers = set_power(r.powers, bit, true);
        set_role(d, &mut st.admin.changes, r);
    }
    let _ = v;
}

/// The group's identity (LLPanelGroupGeneral's editable part).
fn identity(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    let Some(g) = v.profile else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    let saved = GroupInfo {
        charter: g.charter.clone(),
        show_in_list: g.show_in_list,
        insignia: g.insignia,
        membership_fee: g.membership_fee,
        open_enrollment: g.open_enrollment,
        allow_publish: g.allow_publish,
        mature_publish: g.mature_publish,
    };
    let info = st.admin.identity.get_or_insert_with(|| saved.clone());
    let ident = v.can(powers::GROUP_CHANGE_IDENTITY);
    let options = v.can(powers::MEMBER_OPTIONS);
    egui::ScrollArea::vertical()
        .id_salt("group_admin_identity")
        .max_height((ui.available_height() - 40.0).max(80.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let fields = IdentityFields {
                ident,
                options,
                charter_limit: 1023,
                insignia_card: true,
            };
            if identity_cards(ui, c, info, Rating::Switch, fields) {
                st.admin.texture.open(aurora_i18n::tr!("ui-group-admin-insignia"), info.insignia);
            }
        });
    let dirty = *info != saved;
    let (mut save, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let size = Vec2::new(0.0, 28.0);
            save = widgets::action_button_if(
                ui,
                p,
                "device-floppy-filled",
                aurora_i18n::tr!("ui-group-admin-save"),
                ActionKind::Primary,
                size,
                dirty,
            )
            .clicked();
            cancel = widgets::action_button_if(ui, p, "", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, dirty).clicked();
        });
    });
    if save {
        actions.push(GroupAction::UpdateInfo {
            group: v.id,
            info: info.clone(),
        });
    }
    if save || cancel {
        st.admin.identity = None;
    }
}

/// How the identity cards ask the content rating.
pub(super) enum Rating<'a> {
    /// A "Moderate content" switch on `mature_publish` (the identity of a
    /// group).
    Switch,
    /// General or Moderate, neither chosen at first (the group being
    /// created: LLPanelGroupCreate's "- Select maturity rating -").
    Choice(&'a mut Option<bool>),
}

/// Which parts of the identity the cards let us change.
pub(super) struct IdentityFields {
    /// GP_GROUP_CHANGE_IDENTITY: insignia, charter, search, rating.
    pub ident: bool,
    /// GP_MEMBER_OPTIONS: enrollment and fee.
    pub options: bool,
    /// Characters of the charter.
    pub charter_limit: usize,
    /// The insignia's card (the creation picks it on the header instead).
    pub insignia_card: bool,
}

/// The cards of a group's identity (LLPanelGroupGeneral's editable part,
/// the creation panel's fields): the insignia, the charter, then search,
/// rating, enrollment and fee. True when "Change the insignia" was clicked.
pub(super) fn identity_cards(ui: &mut egui::Ui, c: &mut Ctx, info: &mut GroupInfo, rating: Rating, fields: IdentityFields) -> bool {
    let p = c.p;
    let IdentityFields {
        ident,
        options,
        charter_limit,
        insignia_card,
    } = fields;
    let mut change = false;
    ui.spacing_mut().item_spacing.y = 10.0;
    if insignia_card {
        view::card(
            ui,
            p,
            "photo-filled",
            p.muted_dim,
            aurora_i18n::tr!("ui-group-admin-insignia"),
            None,
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if !info.insignia.is_nil() {
                        c.wanted_images.insert(info.insignia);
                    }
                    widgets::picture(ui, p, c.images.get(&info.insignia), Vec2::splat(96.0), "users-group", "");
                    ui.vertical(|ui| {
                        change = widgets::action_button_if(
                            ui,
                            p,
                            "photo-filled",
                            aurora_i18n::tr!("ui-group-admin-insignia-change"),
                            ActionKind::Flat,
                            Vec2::new(0.0, 26.0),
                            ident,
                        )
                        .clicked();
                    });
                });
            },
        );
    }
    view::card(
        ui,
        p,
        "file-text-filled",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-profile-charter"),
        None,
        |ui| {
            ui.add_enabled_ui(ident, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut info.charter)
                        .char_limit(charter_limit)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
            });
        },
    );
    view::card(
        ui,
        p,
        "adjustments-filled",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-admin-settings"),
        None,
        |ui| {
            switch_row(
                ui,
                p,
                "search-filled",
                aurora_i18n::tr!("ui-group-admin-show-in-search"),
                &mut info.show_in_list,
                ident,
            );
            match rating {
                Rating::Switch => {
                    switch_row(
                        ui,
                        p,
                        "alert-triangle-filled",
                        aurora_i18n::tr!("ui-group-admin-moderate"),
                        &mut info.mature_publish,
                        ident,
                    );
                }
                Rating::Choice(chosen) => rating_choice(ui, p, chosen, info, ident),
            }
            switch_row(
                ui,
                p,
                "door-enter",
                aurora_i18n::tr!("ui-group-profile-open-enrollment"),
                &mut info.open_enrollment,
                options,
            );
            // the fee: on with an amount, as Firestorm's check box and field
            let mut paid = info.membership_fee > 0;
            if switch_row(ui, p, "coin-filled", aurora_i18n::tr!("ui-group-admin-fee"), &mut paid, options) {
                info.membership_fee = if paid { 1.max(info.membership_fee) } else { 0 };
            }
            if paid {
                ui.horizontal(|ui| {
                    ui.add_space(23.0);
                    ui.label(egui::RichText::new("L$").size(12.5).color(p.muted));
                    ui.add_enabled(options, egui::DragValue::new(&mut info.membership_fee).range(1..=99_999));
                });
            }
        },
    );
    change
}

/// The content rating to choose: General or Moderate, with the maturity
/// badges, in an amber line until one is chosen.
fn rating_choice(ui: &mut egui::Ui, p: &Palette, chosen: &mut Option<bool>, info: &mut GroupInfo, enabled: bool) {
    ui.horizontal(|ui| {
        ui.set_min_height(26.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        let ink = if chosen.is_none() { p.amber } else { p.muted };
        widgets::glyph(ui, "alert-triangle-filled", ink, 15.0);
        let label = egui::RichText::new(aurora_i18n::tr!("ui-group-create-rating")).size(12.5);
        ui.label(if chosen.is_none() {
            label.color(p.amber)
        } else {
            label.color(p.ink)
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (moderate, text) in [
                    (true, aurora_i18n::tr!("ui-group-profile-moderate-content")),
                    (false, aurora_i18n::tr!("ui-group-profile-general-content")),
                ] {
                    if widgets::radio(ui, p, *chosen == Some(moderate), text).clicked() {
                        *chosen = Some(moderate);
                    }
                }
            });
        });
    });
    info.mature_publish = *chosen == Some(true);
}

/// Who has which ability (LLPanelGroupActionsSubTab): the abilities by
/// set, the roles and members of the one chosen.
fn abilities_section(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, d: &GroupData) {
    let p = c.p;
    let catalogue = abilities::catalogue();
    let full = ui.max_rect();
    let left = Rect::from_min_max(
        ui.cursor().min,
        Pos2::new(ui.cursor().min.x + (full.width() * 0.5).min(320.0), full.bottom()),
    );
    let right = Rect::from_min_max(Pos2::new(left.right() + 12.0, left.top()), full.right_bottom());
    let mut l = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    l.set_clip_rect(left.intersect(ui.clip_rect()));
    egui::ScrollArea::vertical()
        .id_salt("group_admin_abilities")
        .auto_shrink([false, false])
        .show(&mut l, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for set in &catalogue {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    widgets::glyph(ui, set.icon, p.violet_pale, 13.0);
                    ui.label(
                        egui::RichText::new(set.name)
                            .font(FontId::new(12.0, crate::ui::fonts::semibold(ui.ctx())))
                            .color(p.violet_pale),
                    );
                });
                for a in &set.abilities {
                    let on = st.admin.ability == Some(a.bit);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
                    if on {
                        ui.painter()
                            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.violet.gamma_multiply(0.18));
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.35));
                    }
                    widgets::paint_elided(
                        ui,
                        a.name,
                        Pos2::new(rect.left() + 19.0, rect.center().y),
                        rect.width() - 24.0,
                        FontId::proportional(12.0),
                        if on { p.ink } else { p.muted },
                    );
                    if resp.on_hover_text(a.help).clicked() {
                        st.admin.ability = Some(a.bit);
                    }
                }
            }
        });
    let mut r = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(right)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    r.spacing_mut().item_spacing.y = 10.0;
    let Some(bit) = st.admin.ability else {
        view::card(
            &mut r,
            p,
            "key-filled",
            p.muted_dim,
            aurora_i18n::tr!("ui-group-admin-abilities"),
            None,
            |ui| {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-admin-pick-ability"))
                        .size(12.0)
                        .color(p.muted),
                );
            },
        );
        return;
    };
    let power = 1u64 << bit;
    let ability = catalogue.iter().flat_map(|s| s.abilities.iter()).find(|a| a.bit == bit);
    let roles: Vec<&GroupRole> = {
        let mut v: Vec<&GroupRole> = d.roles.values().filter(|r| r.powers & power != 0).collect();
        v.sort_by_cached_key(|r| role_name(&r.name).to_lowercase());
        v
    };
    view::card(
        &mut r,
        p,
        "crown",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-admin-roles-with"),
        None,
        |ui| {
            if let Some(a) = ability {
                ui.label(egui::RichText::new(a.help).size(12.0).color(p.muted));
            }
            if roles.is_empty() {
                ui.label(
                    egui::RichText::new(aurora_i18n::tr!("ui-group-admin-nobody"))
                        .size(12.0)
                        .color(p.muted_dim),
                );
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for r in &roles {
                    view::role_chip(ui, p, &role_name(&r.name), view::Chip::Plain);
                }
            });
        },
    );
    // the members through those roles (owners have every ability)
    let mut members: Vec<(Uuid, String)> = d
        .members
        .values()
        .filter(|m| {
            m.is_owner
                || d.member_roles
                    .get(&m.id)
                    .is_some_and(|rs| rs.iter().any(|r| roles.iter().any(|x| x.id == *r)))
        })
        .map(|m| (m.id, c.world.social.display_name_of(&m.id)))
        .collect();
    members.sort_by_cached_key(|m| m.1.to_lowercase());
    let title = aurora_i18n::tr!("ui-group-admin-members-with", count = members.len() as i64);
    view::card(&mut r, p, "users", p.muted_dim, &title, None, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("group_admin_ability_members")
            .max_height(260.0)
            .show(ui, |ui| {
                for (id, name) in members.iter().take(200) {
                    if name == "…" {
                        c.want_names.insert(*id);
                    }
                    ui.label(egui::RichText::new(name).size(12.5).color(p.ink));
                }
            });
    });
    let _ = v;
}

/// The banned residents (GroupAPIv1), unban, ban by name.
fn bans(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View, d: &GroupData, actions: &mut Vec<GroupAction>) {
    let p = c.p;
    if st.admin.bans_asked != Some(v.id) {
        st.admin.bans_asked = Some(v.id);
        actions.push(GroupAction::RequestBans(v.id));
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let count = d.bans.as_ref().map_or(0, Vec::len);
        ui.label(
            egui::RichText::new(aurora_i18n::tr!(
                "ui-group-admin-bans-count",
                count = count as i64,
                max = crate::world::group_data::MAX_BANNED as i64
            ))
            .size(12.0)
            .color(p.muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("search-refresh"), true) {
                actions.push(GroupAction::RequestBans(v.id));
            }
            let room = count < crate::world::group_data::MAX_BANNED;
            if widgets::action_button_if(
                ui,
                p,
                "ban",
                aurora_i18n::tr!("ui-group-admin-ban-residents"),
                ActionKind::SoftDanger,
                Vec2::new(0.0, 26.0),
                room,
            )
            .clicked()
            {
                st.admin.picker.open("group_ban_picker", true);
            }
        });
    });
    ui.add_space(6.0);
    let Some(list) = &d.bans else {
        ui.label(egui::RichText::new(aurora_i18n::tr!("common-loading-2")).size(12.5).color(p.muted));
        return;
    };
    if list.is_empty() {
        ui.label(
            egui::RichText::new(aurora_i18n::tr!("ui-group-admin-no-bans"))
                .size(12.5)
                .color(p.muted),
        );
    }
    st.admin.bans_checked.retain(|id| list.iter().any(|(b, _)| b == id));
    egui::ScrollArea::vertical()
        .id_salt("group_admin_bans")
        .max_height((ui.available_height() - 40.0).max(60.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (id, date) in list {
                let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
                let on = st.admin.bans_checked.contains(id);
                if on {
                    ui.painter().rect_filled(row, 0.0, p.violet.gamma_multiply(0.16));
                } else if resp.hovered() {
                    ui.painter().rect_filled(row, 0.0, p.raised.gamma_multiply(0.35));
                }
                if resp.clicked() {
                    if let Some(i) = st.admin.bans_checked.iter().position(|x| x == id) {
                        st.admin.bans_checked.remove(i);
                    } else {
                        st.admin.bans_checked.push(*id);
                    }
                }
                let (icon, color) = if on {
                    ("square-check-filled", p.violet_light)
                } else {
                    ("square", p.muted_dim)
                };
                if let Some(t) = crate::ui::icons::global(icon) {
                    ui.painter().image(
                        t.id(),
                        Rect::from_center_size(Pos2::new(row.left() + 13.0, row.center().y), Vec2::splat(15.0)),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        color,
                    );
                }
                let name = c.world.social.display_name_of(id);
                if name == "…" {
                    c.want_names.insert(*id);
                }
                widgets::paint_elided(
                    ui,
                    &name,
                    Pos2::new(row.left() + 32.0, row.center().y),
                    row.width() * 0.6,
                    FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                    p.ink,
                );
                if *date > 0.0 {
                    let text = crate::world::profiles::slt_date(*date as u32);
                    let g = ui.painter().layout_no_wrap(text, FontId::proportional(12.0), p.muted);
                    let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, row.right() - 8.0 - g.size().x, row.center().y);
                    ui.painter().galley(at, g, p.muted);
                }
            }
        });
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let n = st.admin.bans_checked.len();
            if widgets::action_button_if(
                ui,
                p,
                "check",
                aurora_i18n::tr!("ui-group-admin-unban"),
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
                n > 0,
            )
            .clicked()
            {
                let ids = std::mem::take(&mut st.admin.bans_checked);
                actions.push(GroupAction::Unban { group: v.id, ids });
            }
        });
    });
}

/// "Invitations": invite by name, with a role.
fn invitations(st: &mut GroupProfileUi, ui: &mut egui::Ui, c: &mut Ctx, v: &View) {
    let p = c.p;
    let mut open = false;
    view::card(
        ui,
        p,
        "user-plus",
        p.muted_dim,
        aurora_i18n::tr!("ui-group-admin-invitations"),
        None,
        |ui| {
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-invite-help"))
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            if widgets::action_button(
                ui,
                p,
                "user-plus",
                aurora_i18n::tr!("ui-group-profile-invite"),
                ActionKind::Primary,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
            {
                open = true;
            }
        },
    );
    if open {
        st.open_invite(v.id);
    }
}

impl AdminState {
    /// The pickers (they search the world), after the windows; the images
    /// the texture picker wants are returned.
    pub fn dialogs(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> HashSet<Uuid> {
        if let Some(ids) = self.picker.show(ctx, p, world) {
            self.to_ban = ids;
        }
        if let Some(texture) = self.texture.show(ctx, p, world, images)
            && let Some(info) = self.identity.as_mut()
        {
            info.insignia = texture;
        }
        std::mem::take(&mut self.texture.wanted_images)
    }
}
