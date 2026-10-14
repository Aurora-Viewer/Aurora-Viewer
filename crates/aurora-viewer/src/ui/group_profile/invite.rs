//! "Invite to the group" (LLFloaterGroupInvite / LLPanelGroupInvite,
//! indra/newview/llpanelgroupinvite.cpp, panel_group_invite.xml, originally
//! LGPL 2.1): residents chosen with the resident picker, the role they are
//! invited into among those our abilities allow (`groups::invite_roles`),
//! then InviteGroupRequest. As submitInvitations does: the owners' role asks
//! to confirm first, the residents already in the group are left out and
//! said so, more than MAX_GROUP_INVITES at once is refused.

use super::GroupAction;
use super::members::role_name;
use super::view::View;
use crate::theme::Palette;
use crate::ui::avatar_picker::AvatarPicker;
use crate::ui::place_details::Ctx;
use crate::ui::widgets::{self, ActionKind, Floater};
use crate::world::World;
use aurora_net::groups::{self, MAX_GROUP_INVITES};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

pub const WINDOW: &str = "group_invite";
const PICKER: &str = "group_invite_picker";

/// What the window says once the invitations went (or did not).
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// "already_in_group".
    AlreadyInGroup,
    /// "invite_selection_too_large".
    TooMany,
}

#[derive(Default)]
pub struct InviteUi {
    group: Option<Uuid>,
    open: bool,
    raise: bool,
    pub invitees: Vec<Uuid>,
    /// The role chosen, "everyone" (nil) by default.
    pub role: Uuid,
    picker: AvatarPicker,
    /// The owners' role was chosen: confirm before sending.
    confirm_owner: bool,
    pub notice: Option<Notice>,
}

impl InviteUi {
    /// LLFloaterGroupInvite::showForGroup: a new list for this group.
    pub fn open(&mut self, group: Uuid, invitees: Vec<Uuid>) {
        if self.group != Some(group) || !self.open {
            self.invitees = invitees;
            self.role = Uuid::nil();
        }
        self.group = Some(group);
        self.open = true;
        self.raise = true;
        self.confirm_owner = false;
    }

    /// The resident picker (it needs the world to search), drawn by the app
    /// after the windows.
    pub fn picker(&mut self, ctx: &egui::Context, p: &Palette, world: &mut World) {
        if let Some(ids) = self.picker.show(ctx, p, world) {
            for id in ids {
                if !self.invitees.contains(&id) {
                    self.invitees.push(id);
                }
            }
        }
    }

    /// The roles the list offers, "everyone" first.
    fn roles<'a>(v: &'a View, me: Uuid) -> Vec<&'a groups::GroupRole> {
        let Some(d) = v.data.filter(|d| d.roles_complete) else {
            return Vec::new();
        };
        let roles: Vec<groups::GroupRole> = d.roles.values().cloned().collect();
        let mine = d.member_roles.get(&me).cloned().unwrap_or_default();
        let owner = d.members.get(&me).is_some_and(|m| m.is_owner);
        let owner_role = v.profile.map(|g| g.owner_role).unwrap_or_default();
        let allowed: Vec<Uuid> = groups::invite_roles(&roles, &mine, v.powers(), owner_role, owner)
            .iter()
            .map(|r| r.id)
            .collect();
        let mut out: Vec<&groups::GroupRole> = d.roles.values().filter(|r| allowed.contains(&r.id)).collect();
        out.sort_by_cached_key(|r| (!r.id.is_nil(), role_name(&r.name).to_lowercase()));
        out
    }

    /// LLPanelGroupInvite::impl::submitInvitations: the pairs to send and
    /// what to tell, or None while the owners' role waits for a yes.
    pub fn submit(&mut self, v: &View) -> Option<Vec<(Uuid, Uuid)>> {
        let owner_role = v.profile.map(|g| g.owner_role).unwrap_or_default();
        if self.role == owner_role && !owner_role.is_nil() && !self.confirm_owner {
            self.confirm_owner = true;
            return None;
        }
        let members = v.data.map(|d| &d.members);
        let (already, pairs): (Vec<Uuid>, Vec<Uuid>) = self.invitees.iter().partition(|id| members.is_some_and(|m| m.contains_key(id)));
        self.open = false;
        self.confirm_owner = false;
        if pairs.len() > MAX_GROUP_INVITES {
            self.notice = Some(Notice::TooMany);
            return Some(Vec::new());
        }
        self.notice = (!already.is_empty()).then_some(Notice::AlreadyInGroup);
        Some(pairs.into_iter().map(|id| (id, self.role)).collect())
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        c: &mut Ctx,
        pics: &HashMap<Uuid, egui::TextureHandle>,
        v: &View,
        actions: &mut Vec<GroupAction>,
    ) {
        self.notice_dialog(ctx, c.p);
        if !self.open || self.group != Some(v.id) {
            return;
        }
        let p = c.p;
        if std::mem::take(&mut self.raise) {
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::new(WINDOW)));
        }
        let size = Vec2::new(380.0, 460.0);
        let screen = ctx.content_rect();
        let title = aurora_i18n::tr!("ui-group-invite-title", group = v.name.clone());
        let mut open = self.open;
        let mut send = false;
        let mut floater = Floater::new(WINDOW, title, screen.center() - size * 0.5 + Vec2::new(40.0, 40.0), size);
        floater.min_size = Vec2::new(320.0, 360.0);
        floater.show(ctx, p, &mut open, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-invite-help"))
                    .size(12.5)
                    .color(p.muted),
            );
            if widgets::action_button(
                ui,
                p,
                "user-plus",
                aurora_i18n::tr!("ui-group-invite-choose"),
                ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
            {
                self.picker.open(PICKER, true);
            }
            // the invitees
            let list_h = (ui.available_height() - 110.0).max(80.0);
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(ui.visuals().window_corner_radius)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt("group_invitees")
                        .max_height(list_h)
                        .min_scrolled_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.invitees.is_empty() {
                                ui.label(
                                    egui::RichText::new(aurora_i18n::tr!("ui-group-invite-empty"))
                                        .size(12.5)
                                        .color(p.muted_dim),
                                );
                            }
                            let mut remove = None;
                            for id in &self.invitees {
                                let name = c.world.social.display_name_of(id);
                                if name == "…" {
                                    c.want_names.insert(*id);
                                }
                                let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
                                let pic = Rect::from_center_size(Pos2::new(row.left() + 12.0, row.center().y), Vec2::splat(22.0));
                                widgets::avatar_disc(ui, p, pic, pics.get(id), &name, false);
                                widgets::paint_elided(
                                    ui,
                                    &name,
                                    Pos2::new(pic.right() + 8.0, row.center().y),
                                    row.width() - 70.0,
                                    FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
                                    p.ink,
                                );
                                let x = Rect::from_center_size(Pos2::new(row.right() - 14.0, row.center().y), Vec2::splat(22.0));
                                let mut slot = ui.new_child(egui::UiBuilder::new().max_rect(x));
                                if widgets::icon_button(&mut slot, p, "x", aurora_i18n::tr!("ui-group-invite-remove"), true) {
                                    remove = Some(*id);
                                }
                            }
                            if let Some(id) = remove {
                                self.invitees.retain(|i| *i != id);
                            }
                        });
                });
            // the role
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-invite-role"))
                    .size(12.5)
                    .color(p.ink),
            );
            let roles = Self::roles(v, c.world.agent_id);
            if !roles.iter().any(|r| r.id == self.role) {
                self.role = Uuid::nil();
            }
            let current = roles
                .iter()
                .find(|r| r.id == self.role)
                .map_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned(), |r| role_name(&r.name));
            ui.add_enabled_ui(!roles.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(("group_invite_role", v.id))
                    .width(ui.available_width())
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for r in &roles {
                            if ui.selectable_label(r.id == self.role, role_name(&r.name)).clicked() {
                                self.role = r.id;
                            }
                        }
                    });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let ready = !self.invitees.is_empty() && !roles.is_empty();
                let label = aurora_i18n::tr!("ui-group-invite-send");
                if widgets::action_button_if(ui, p, "send-filled", label, ActionKind::Primary, Vec2::new(0.0, 28.0), ready).clicked() {
                    send = true;
                }
                if widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                )
                .clicked()
                {
                    self.open = false;
                }
            });
        });
        if !open {
            self.open = false;
        }
        if self.confirm_owner {
            self.owner_dialog(ctx, p, &mut send);
        }
        if send
            && let Some(pairs) = self.submit(v)
            && !pairs.is_empty()
        {
            actions.push(GroupAction::Invite {
                group: v.id,
                invites: pairs,
            });
        }
    }

    /// "confirm_invite_owner_str", Yes / Cancel.
    fn owner_dialog(&mut self, ctx: &egui::Context, p: &Palette, send: &mut bool) {
        let modal = widgets::Modal::new(egui::Id::new("group_invite_owner")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-group-invite-owner"))
                    .size(12.5)
                    .color(p.ink),
            );
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = Vec2::new(0.0, 26.0);
                if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-yes"), ActionKind::Danger, size).clicked() {
                    *send = true;
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    self.confirm_owner = false;
                }
            });
        });
        if modal.should_close() && !*send {
            self.confirm_owner = false;
        }
    }

    /// The GenericAlert of "already_in_group" / "invite_selection_too_large".
    fn notice_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(notice) = &self.notice else {
            return;
        };
        let text = match notice {
            Notice::AlreadyInGroup => aurora_i18n::tr!("ui-group-invite-already"),
            Notice::TooMany => aurora_i18n::tr!("ui-group-invite-too-many"),
        };
        let mut done = false;
        let modal = widgets::Modal::new(egui::Id::new("group_invite_notice")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(egui::RichText::new(text).size(12.5).color(p.ink));
            ui.add_space(10.0);
            if widgets::action_button(
                ui,
                p,
                "check",
                aurora_i18n::tr!("common-ok"),
                ActionKind::Primary,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
            {
                done = true;
            }
        });
        if done || modal.should_close() {
            self.notice = None;
        }
    }
}
