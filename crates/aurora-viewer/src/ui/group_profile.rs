//! Group profile window (LLPanelGroup, indra/newview/llpanelgroup.cpp,
//! llpanelgroupgeneral.cpp, llpanelgrouproles.cpp, llgroupactions.cpp,
//! originally LGPL 2.1). A group's name opens it like LLGroupActions::show
//! (the "group ... inspect" links of Firestorm): "About Land", the build
//! window, the search results, the People window. One window, on the last
//! group asked for.
//!
//! Two looks, chosen by the "Profiles" row of Preferences › Aurora:
//! - Aurora: a header with the insignia over its blurred banner, the
//!   group's identity, badges and actions, then tabs (`group_profile/`):
//!   Home (charter, latest notices, members online, "Me in this group")
//!   and Members (the list, read only for now);
//! - classic: the general information of LLPanelGroupGeneral drawn by the
//!   group pane of "Search" (`search/details.rs`), with the members below
//!   in a collapsing section, as Firestorm's accordions.
//!
//! "Create a group" (`group_profile/create.rs`) is a version of the same
//! window: the header and the identity cards, or Firestorm's creation panel.
//!
//! The data: GroupProfileReply (`world.search.groups`), our membership
//! (`world.groups`) and LLGroupMgr's members, roles, titles and notices
//! (`world.group_data`), the land and accounts of LLPanelGroupLandMoney
//! (`world.group_money`). Experiences are not ported yet.

mod abilities;
mod admin_members;
mod administration;
pub mod create;
mod experiences;
mod home;
mod invite;
mod land;
mod members;
mod notices;
mod view;

// a notice's attachment drawn the same on its notification card
pub(crate) use notices::{asset_label, attachment_icon};

use super::place_details::Ctx;
use super::search::SearchAction;
use super::widgets::{self, Floater};
use crate::world::World;
use egui::Vec2;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const WINDOW: &str = "group_profile";

/// Ask for the profile of a group from anywhere in the interface (a link, a
/// name); the app opens it at the end of the frame.
pub fn request_open(ctx: &egui::Context, id: Uuid) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("aurora_open_group_profile"), id));
}

pub fn take_open_request(ctx: &egui::Context) -> Option<Uuid> {
    ctx.data_mut(|d| d.remove_temp::<Uuid>(egui::Id::unique("aurora_open_group_profile")))
}

/// Ask for the "Create a group" window (LLGroupActions::createGroup) from
/// anywhere in the interface; the app opens it at the end of the frame.
pub fn request_create(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("aurora_create_group"), true));
}

pub fn take_create_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("aurora_create_group")))
        .unwrap_or(false)
}

/// What the window asks the app to do (the rest goes through the context
/// actions: chat, leave, profiles, block the chat).
pub enum GroupAction {
    /// The pane of "Search" asked something (the classic look).
    Search(SearchAction),
    /// LLGroupMgr's data of the group, asked when the window opens on it or
    /// when our membership changed (`world.group_data.open`).
    Load { group: Uuid, member: bool },
    /// "Refresh": the profile and the data again (LLGroupActions::refresh).
    Refresh { group: Uuid, member: bool },
    /// JoinGroupRequest, confirmed.
    Join(Uuid),
    /// The notice list again (onClickRefreshNotices).
    RefreshNotices(Uuid),
    /// GroupNoticeRequest (onSelectNotice).
    OpenNotice { group: Uuid, notice: Uuid },
    /// The notice's attachment, accepted into the inventory.
    SaveAttachment(Uuid),
    /// send_group_notice: subject, message, (item, owner) attached.
    SendNotice {
        group: Uuid,
        subject: String,
        message: String,
        item: Option<(Uuid, Uuid)>,
    },
    /// GroupRoleUpdate: the role changes saved.
    UpdateRoles {
        group: Uuid,
        updates: Vec<aurora_net::groups::RoleUpdate>,
    },
    /// UpdateGroupInfo: the identity saved.
    UpdateInfo { group: Uuid, info: aurora_net::groups::GroupInfo },
    /// CreateGroupRequest, confirmed: the trimmed name and the identity.
    Create { name: String, info: aurora_net::groups::GroupInfo },
    /// The ban list asked (GroupAPIv1).
    RequestBans(Uuid),
    /// Bans lifted.
    Unban { group: Uuid, ids: Vec<Uuid> },
    /// Residents banned by name (LLPanelGroupBulkBan).
    BanResidents { group: Uuid, ids: Vec<Uuid> },
    /// EjectGroupMemberRequest, confirmed.
    Eject { group: Uuid, members: Vec<Uuid> },
    /// The ban (GroupAPIv1), then the ejection, confirmed.
    Ban { group: Uuid, members: Vec<Uuid> },
    /// GroupRoleChanges.
    ChangeRoles {
        group: Uuid,
        changes: Vec<aurora_net::groups::RoleChange>,
    },
    /// InviteGroupRequest: (invitee, role) pairs.
    Invite { group: Uuid, invites: Vec<(Uuid, Uuid)> },
    /// The invitation opened from a resident's menu
    /// (LLFloaterGroupInvite::showForGroup, request_update): our groups and
    /// this group's data asked again.
    ReloadForInvite(Uuid),
    /// "Call" (LLGroupActions::startCall), once group voice is there
    /// (`Env::group_calls`).
    Call(Uuid),
    /// ActivateGroup (nil: none).
    Activate(Uuid),
    /// GroupTitleUpdate.
    SetTitle { group: Uuid, role: Uuid },
    /// SetGroupAcceptNotices (LLAgent::setUserGroupFlags).
    SetFlags {
        group: Uuid,
        accept_notices: bool,
        list_in_profile: bool,
    },
    /// "See in People": the groups tab, filtered on this name.
    ShowInPeople(String),
    /// The "Land and L$" tab shown (LLPanelGroupLandMoney::activate): the
    /// parcels and the page shown asked.
    LoadLand {
        group: Uuid,
        ledger: aurora_net::group_money::Ledger,
    },
    /// Another page of the accounts shown (onClickTab).
    LandPage {
        group: Uuid,
        ledger: aurora_net::group_money::Ledger,
    },
    /// A week earlier or later (onClickEarlier / onClickLater).
    LandStep {
        group: Uuid,
        ledger: aurora_net::group_money::Ledger,
        earlier: bool,
    },
    /// applyContribution: our land given to the group, checked.
    SetContribution { group: Uuid, contribution: i32 },
    /// "Map" of a parcel (onMapButton): its global position.
    ShowOnMap { x: f64, y: f64 },
    /// "About Land" on a parcel of a region nearby.
    AboutLand { x: f64, y: f64 },
    /// The experiences the group owns asked (LLPanelGroupExperiences::activate).
    LoadExperiences(Uuid),
    /// "Profile" of an experience: the experience profile window.
    ExperienceProfile(Uuid),
}

/// The tabs of the Aurora look.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Home,
    Members,
    Notices,
    Land,
    Experiences,
    Admin,
}

/// The confirmation of "Join" (LLGroupActions::join's notifications).
#[derive(Clone, Debug, PartialEq)]
enum Confirm {
    /// JoinGroupNoCost / JoinGroupCanAfford.
    Join { group: Uuid, name: String, fee: i32 },
    /// JoinGroupCannotAfford.
    CannotAfford { name: String, fee: i32 },
    /// JoinedTooManyGroups.
    TooManyGroups,
}

#[derive(Default)]
pub struct GroupProfileUi {
    group: Option<Uuid>,
    open: bool,
    raise: bool,
    pub(super) tab: Tab,
    /// The data was asked for this group, as member or not: asked again
    /// when the membership changes or the data was forgotten.
    loaded: Option<(Uuid, bool)>,
    pub(super) members: members::MembersState,
    pub(super) notices: notices::NoticesState,
    pub(super) land: land::LandState,
    pub(super) experiences: experiences::ExperiencesState,
    pub(super) admin: administration::AdminState,
    confirm: Option<Confirm>,
    /// "Invite to the group".
    pub invite: invite::InviteUi,
    /// "Create a group".
    pub create: create::GroupCreateUi,
    /// Profile pictures of the residents shown (drained by the app).
    pub wanted_pics: HashSet<Uuid>,
    pic_budget: super::nameplate::PictureBudget,
    pics_asked: HashSet<Uuid>,
}

/// What the window draws with, besides the world.
pub struct Env<'a> {
    pub avatar_pics: &'a HashMap<Uuid, egui::TextureHandle>,
    /// "Profiles" of Preferences › Aurora set to the classic window.
    pub classic: bool,
    /// Seconds, for the pace of the picture requests.
    pub now: f64,
    /// Group voice calls can be started ("Call" is greyed until then).
    pub group_calls: bool,
}

impl GroupProfileUi {
    /// "Invite" of a group's sheet in People (LLGroupActions::invite): the
    /// profile on that group, its invitation open.
    pub fn open_with_invite(&mut self, world: &mut World, id: Uuid) {
        self.open(world, id);
        if self.group == Some(id) {
            self.open_invite(id);
        }
    }

    /// LLGroupActions::show: the window on this group, brought to the front,
    /// its profile asked again.
    pub fn open(&mut self, world: &mut World, id: Uuid) {
        if id.is_nil() {
            return;
        }
        world.search.request_group(id);
        if self.group != Some(id) {
            self.tab = Tab::Home;
            self.members = Default::default();
            self.land = Default::default();
            self.experiences = Default::default();
            self.loaded = None;
        }
        self.group = Some(id);
        self.open = true;
        self.raise = true;
    }

    /// Open the Administration tab, on Group identity when `identity` (the
    /// demo's captures).
    pub fn show_admin(&mut self, identity: bool) {
        self.tab = Tab::Admin;
        if identity {
            self.admin.section = administration::Section::Identity;
        }
    }

    /// Open the Experiences tab (the demo's captures).
    pub fn show_experiences(&mut self) {
        self.tab = Tab::Experiences;
    }

    /// Open the "Land and L$" tab on a page of the accounts (the demo's
    /// captures).
    pub fn show_land(&mut self, ledger: aurora_net::group_money::Ledger) {
        self.tab = Tab::Land;
        self.land.ledger = ledger;
    }

    /// Open the Notices tab (the demo's captures), on a new notice when
    /// `compose`.
    pub fn show_notices(&mut self, compose: bool) {
        self.tab = Tab::Notices;
        if compose {
            self.notices.compose = Some(Default::default());
        }
    }

    /// Open the Members tab with a member selected (the demo's captures).
    pub fn show_member(&mut self, id: Uuid) {
        self.tab = Tab::Members;
        self.members.selected = Some(id);
    }

    /// The group shown, while the window is open.
    pub fn shown(&self) -> Option<Uuid> {
        self.group.filter(|_| self.open)
    }

    /// Join from search or the profile through the same confirmation,
    /// without needing to open the profile (LLGroupActions::join).
    pub fn join(&mut self, world: &World, group: Uuid) {
        let v = view::View::of(world, group);
        if v.member.is_none() && v.profile.is_some_and(|g| g.open_enrollment) {
            self.ask_join(world, &v, group);
        }
    }

    /// A row on screen has no picture yet: asked at a few new residents a
    /// second, as the People window does.
    pub(super) fn want_pic(&mut self, id: Uuid, now: f64) {
        if self.pics_asked.contains(&id) || self.pic_budget.take(now) {
            self.pics_asked.insert(id);
            self.wanted_pics.insert(id);
        }
    }

    /// processCreateGroupReply: the new group's profile opened, or the
    /// refusal shown by the creation window.
    pub fn on_created(&mut self, world: &mut World, group: Uuid, success: bool, message: &str) {
        if let Some(id) = self.create.on_reply(group, success, message) {
            self.open(world, id);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, c: &mut Ctx, env: &Env) -> Vec<GroupAction> {
        let mut actions = self.create.show(ctx, c, env);
        // "Invite to group" of a resident's menu: the group picker, and the
        // chosen group's data asked again
        self.invite.group_picker(ctx, c.p, c.world);
        if let Some(group) = self.invite.take_reload() {
            actions.push(GroupAction::ReloadForInvite(group));
        }
        if let Some(id) = self.shown() {
            self.profile_window(ctx, c, env, id, &mut actions);
        }
        // the invitation is a window of its own (LLFloaterGroupInvite), with
        // or without the group's profile
        if let Some(group) = self.invite.target() {
            let v = view::View::of(c.world, group);
            self.invite.show(ctx, c, env.avatar_pics, &v, &mut actions);
        }
        self.confirm_dialog(ctx, c.p, &mut actions);
        actions
    }

    /// The profile window on group `id`.
    fn profile_window(&mut self, ctx: &egui::Context, c: &mut Ctx, env: &Env, id: Uuid, actions: &mut Vec<GroupAction>) {
        if std::mem::take(&mut self.raise) {
            widgets::expand_floater(ctx, WINDOW);
        }
        let member = c.world.groups.is_member(&id);
        let forgotten = c.world.group_data.get(&id).is_none();
        if self.loaded != Some((id, member)) || forgotten {
            self.loaded = Some((id, member));
            actions.push(GroupAction::Load { group: id, member });
        }
        let v = view::View::of(c.world, id);
        let screen = ctx.content_rect();
        let mut open = self.open;
        if env.classic {
            let size = Vec2::new(430.0, 640.0);
            let mut floater = Floater::new(
                WINDOW,
                aurora_i18n::tr!("ui-group-profile-title"),
                screen.center() - size * 0.5 + Vec2::new(60.0, 20.0),
                size,
            );
            floater.min_size = Vec2::new(320.0, 300.0);
            floater.show(ctx, c.p, &mut open, |ui| self.classic(ui, c, &v, actions));
        } else {
            let size = Vec2::new(960.0, 840.0);
            let mut floater = Floater::new(WINDOW, v.name.clone(), screen.center() - size * 0.5, size);
            floater.min_size = Vec2::new(640.0, 460.0);
            floater.show(ctx, c.p, &mut open, |ui| self.aurora(ui, c, env, &v, actions));
        }
        self.open = open;
    }

    /// The resident picker of the invitation window (it searches the
    /// world), after the windows.
    pub fn dialogs(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> HashSet<Uuid> {
        self.invite.picker(ctx, p, world);
        let mut wanted = self.admin.dialogs(ctx, p, world, images);
        wanted.extend(self.create.picker(ctx, p, world, images));
        wanted
    }

    /// "Invite" of the head and of the Members tab (LLGroupActions::invite).
    pub(super) fn open_invite(&mut self, group: Uuid) {
        self.invite.open(group, Vec::new());
    }

    /// The Aurora look: the header, the tabs, the tab.
    fn aurora(&mut self, ui: &mut egui::Ui, c: &mut Ctx, env: &Env, v: &view::View, actions: &mut Vec<GroupAction>) {
        match view::header(ui, c, v, env, &mut self.tab, actions) {
            Some(view::Header::Join) => self.join(c.world, v.id),
            Some(view::Header::Invite) => self.open_invite(v.id),
            None => {}
        }
        // members only: back home after leaving the group
        if (self.tab == Tab::Land && !land::visible(v)) || (self.tab == Tab::Experiences && !experiences::visible(v)) {
            self.tab = Tab::Home;
        }
        // the tabs are activated again each time they are shown
        if self.tab != Tab::Land {
            self.land.active = None;
        }
        if self.tab != Tab::Experiences {
            self.experiences.active = None;
        }
        match self.tab {
            Tab::Home => home::show(self, ui, c, env, v, actions),
            Tab::Members => members::show(self, ui, c, env, v, actions),
            Tab::Notices => notices::show(self, ui, c, v, actions),
            Tab::Land => land::show(self, ui, c, v, actions),
            Tab::Experiences => experiences::show(self, ui, c, v, actions),
            Tab::Admin => administration::show(self, ui, c, v, actions),
        }
    }

    /// The classic look: the pane of "Search", then the members in a
    /// collapsing section (Firestorm's "Members and roles" accordion).
    fn classic(&mut self, ui: &mut egui::Ui, c: &mut Ctx, v: &view::View, actions: &mut Vec<GroupAction>) {
        let mut pane_actions = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("group_profile_classic")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let body = egui::Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), 420.0));
                ui.allocate_rect(body, egui::Sense::hover());
                let mut pane = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(body)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                pane.set_clip_rect(body.intersect(ui.clip_rect()));
                super::search::group_details(&mut pane, c, v.id, &v.name, true, &mut pane_actions);
                ui.add_space(6.0);
                let title = aurora_i18n::tr!("ui-group-profile-members-and-roles");
                widgets::collapsing(ui, c.p, "group_profile_classic_members", title, false, |ui| {
                    members::classic_list(self, ui, c, v);
                });
                // Firestorm's "Notices" accordion: the same list, reading and
                // new notice, in a part of the window's height
                if notices::visible(v) {
                    let title = aurora_i18n::tr!("ui-group-profile-notices");
                    widgets::collapsing(ui, c.p, "group_profile_classic_notices", title, false, |ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 420.0), egui::Sense::hover());
                        let mut part = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        notices::show(self, &mut part, c, v, actions);
                    });
                }
                // Firestorm's "Land & L$" accordion
                if land::visible(v) {
                    let title = aurora_i18n::tr!("ui-group-profile-land");
                    widgets::collapsing(ui, c.p, "group_profile_classic_land", title, false, |ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 460.0), egui::Sense::hover());
                        let mut part = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        land::show(self, &mut part, c, v, actions);
                    });
                }
                // and its "Experiences" accordion
                if experiences::visible(v) {
                    let title = aurora_i18n::tr!("ui-group-profile-experiences");
                    widgets::collapsing(ui, c.p, "group_profile_classic_experiences", title, false, |ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 220.0), egui::Sense::hover());
                        let mut part = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect)
                                .layout(egui::Layout::top_down(egui::Align::Min)),
                        );
                        experiences::show(self, &mut part, c, v, actions);
                    });
                }
            });
        actions.extend(pane_actions.into_iter().map(|a| match a {
            SearchAction::RefreshGroup(group) => GroupAction::Refresh {
                group,
                member: v.member.is_some(),
            },
            other => GroupAction::Search(other),
        }));
    }

    /// LLGroupActions::join: the group limit, then the fee.
    fn ask_join(&mut self, world: &World, v: &view::View, group: Uuid) {
        let limit = super::contacts::group_limit(world);
        if limit > 0 && world.groups.groups.len() >= limit {
            self.confirm = Some(Confirm::TooManyGroups);
            return;
        }
        let fee = v.profile.map_or(0, |g| g.membership_fee);
        let affordable = fee <= 0 || world.balance.is_some_and(|b| b >= fee);
        self.confirm = Some(if affordable {
            Confirm::Join {
                group,
                name: v.name.clone(),
                fee,
            }
        } else {
            Confirm::CannotAfford { name: v.name.clone(), fee }
        });
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, actions: &mut Vec<GroupAction>) {
        let Some(confirm) = self.confirm.clone() else {
            return;
        };
        let (title, text, ok) = match &confirm {
            Confirm::Join { fee, .. } if *fee > 0 => (
                aurora_i18n::tr!("ui-group-profile-join-title"),
                aurora_i18n::tr!("ui-group-profile-join-cost", cost = aurora_i18n::integer(i64::from(*fee))),
                Some(aurora_i18n::tr!("ui-group-profile-join")),
            ),
            Confirm::Join { name, .. } => (
                aurora_i18n::tr!("ui-group-profile-join-title"),
                aurora_i18n::tr!("ui-group-profile-join-free", name = name.clone()),
                Some(aurora_i18n::tr!("ui-group-profile-join")),
            ),
            Confirm::CannotAfford { fee, .. } => (
                aurora_i18n::tr!("ui-group-profile-join-title"),
                aurora_i18n::tr!("ui-group-profile-join-cannot-afford", cost = aurora_i18n::integer(i64::from(*fee))),
                None,
            ),
            Confirm::TooManyGroups => (
                aurora_i18n::tr!("ui-group-profile-join-title"),
                aurora_i18n::tr!("ui-group-profile-too-many-groups").to_owned(),
                None,
            ),
        };
        let mut done: Option<bool> = None;
        let modal = widgets::Modal::new(egui::Id::unique("group_profile_confirm")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(egui::RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&text).size(12.5).color(p.muted));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = Vec2::new(0.0, 26.0);
                match ok {
                    Some(ok) => {
                        if widgets::action_button(ui, p, "door-enter", ok, widgets::ActionKind::Primary, size).clicked() {
                            done = Some(true);
                        }
                        let cancel = aurora_i18n::tr!("common-cancel");
                        if widgets::action_button(ui, p, "x", cancel, widgets::ActionKind::Flat, size).clicked() {
                            done = Some(false);
                        }
                    }
                    None => {
                        if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), widgets::ActionKind::Primary, size)
                            .clicked()
                        {
                            done = Some(false);
                        }
                    }
                }
            });
        });
        if modal.should_close() && done.is_none() {
            done = Some(false);
        }
        if let Some(yes) = done {
            self.confirm = None;
            if let (true, Confirm::Join { group, .. }) = (yes, confirm) {
                actions.push(GroupAction::Join(group));
            }
        }
    }
}

#[cfg(test)]
mod tests;
