//! A group chat's moderator options, in the right-click menu of a member of
//! its members' panel: eject them from the group, allow or forbid their
//! text in the chat, and ban them. Port of FSParticipantList's menu
//! (indra/newview/fsparticipantlist.cpp: isGroupModerator, hasAbilityToBan,
//! canBanSelectedMember, menu_participant_list.xml) and
//! LLGroupActions::canEjectFromGroup (indra/newview/llgroupactions.cpp).
//!
//! The voice items of the Legacy Viewer's submenu are not here: Aurora's
//! group calls have no moderator controls yet.

use crate::theme::Palette;
use crate::ui::context::{CtxAction, request};
use crate::ui::menu;
use crate::ui::widgets::{self, ActionKind};
use crate::world::World;
use crate::world::group_data::GroupData;
use aurora_net::groups::powers;
use uuid::Uuid;

/// A moderator's choice that is confirmed first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    /// EjectGroupMemberWarning.
    Eject { group: Uuid, agent: Uuid },
    /// Aurora asks before a ban too, as its group profile does; the Legacy
    /// Viewer's chat menu bans at once.
    Ban { group: Uuid, agent: Uuid },
}

/// isGroupModerator: the chat of a group of ours, which we moderate.
pub fn is_group_moderator(world: &World, session: &Uuid) -> bool {
    world.groups.is_member(session)
        && world
            .groups
            .sessions
            .get(session)
            .is_some_and(|s| s.moderators.contains(&world.agent_id))
}

/// Our powers in the group (LLAgent::hasPowerInGroup).
pub fn powers_in(world: &World, group: &Uuid) -> u64 {
    world.groups.group(group).map_or(0, |g| g.powers)
}

/// LLGroupActions::canEjectFromGroup: the ejection power; while the
/// members are not all known, that is enough; a member in any role but
/// "everyone", an owner or a resident not in the list is not ejected. Never
/// ourselves (stricter: the Legacy Viewer's menu leaves it to the server).
pub fn can_eject(d: Option<&GroupData>, powers: u64, member: Uuid, me: Uuid) -> bool {
    if member == me || powers & powers::MEMBER_EJECT == 0 {
        return false;
    }
    let Some(d) = d.filter(|d| d.members_complete) else {
        return true;
    };
    let Some(m) = d.members.get(&member) else {
        return false;
    };
    if d.roles_complete && d.role_members_complete && d.member_roles.get(&member).is_some_and(|r| r.iter().any(|r| !r.is_nil())) {
        return false;
    }
    !m.is_owner
}

/// canBanSelectedMember: never ourselves, not while a ban waits, never an
/// owner once the roles are known; the removal from roles and the ban
/// powers. A group whose data was never asked is loaded by the ban itself
/// (`GroupMgr::ban_member`, banMemberById), which checks the owner again,
/// so the entry is not left greyed out until its profile is opened.
pub fn can_ban(d: Option<&GroupData>, powers: u64, member: Uuid, me: Uuid) -> bool {
    if member == me {
        return false;
    }
    if let Some(d) = d {
        if d.ban_pending() {
            return false;
        }
        if d.role_members_complete && d.members.get(&member).is_some_and(|m| m.is_owner) {
            return false;
        }
    }
    powers & powers::ROLE_REMOVE_MEMBER != 0 && powers & powers::GROUP_BAN_ACCESS != 0
}

/// The moderator's part of a member's menu, after the resident's entries:
/// "Moderator options" for a moderator of a group's chat, "Ban member" with
/// the ban power (hasAbilityToBan), as menu_participant_list.xml orders
/// them. Nothing in a conference or a resident's conversation.
pub fn menu(ui: &mut egui::Ui, p: &Palette, world: &World, session: Uuid, id: Uuid) {
    if !world.groups.is_member(&session) {
        return;
    }
    let powers = powers_in(world, &session);
    let moderator = is_group_moderator(world, &session);
    let ban = powers & powers::GROUP_BAN_ACCESS != 0;
    if !moderator && !ban {
        return;
    }
    let ctx = ui.ctx().clone();
    let me = world.agent_id;
    let data = world.group_data.get(&session);
    if moderator {
        menu::separator(ui, p);
        demo_mark(
            &ctx,
            DemoMark::Submenu,
            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 20.0)),
        );
        menu::submenu(ui, p, "shield-filled", aurora_i18n::tr!("ui-chat-moderator-options"), true, |ui| {
            if menu::item_if(
                ui,
                p,
                "user-x",
                aurora_i18n::tr!("ui-chat-eject-from-group"),
                can_eject(data, powers, id, me),
            ) {
                request(&ctx, CtxAction::GroupChatEject { group: session, agent: id });
            }
            menu::separator(ui, p);
            // two plain entries, not a toggle (the Legacy Viewer's choice,
            // against unmuting someone by accident)
            if menu::item(ui, p, "message-circle-filled", aurora_i18n::tr!("ui-chat-allow-text-chat")) {
                request(
                    &ctx,
                    CtxAction::ModerateGroupText {
                        session,
                        agent: id,
                        allow: true,
                    },
                );
            }
            if menu::item(ui, p, "message-off", aurora_i18n::tr!("ui-chat-forbid-text-chat")) {
                request(
                    &ctx,
                    CtxAction::ModerateGroupText {
                        session,
                        agent: id,
                        allow: false,
                    },
                );
            }
        });
    }
    if ban {
        menu::separator(ui, p);
        if menu::item_if(ui, p, "ban", aurora_i18n::tr!("ui-chat-ban-member"), can_ban(data, powers, id, me)) {
            request(&ctx, CtxAction::GroupChatBan { group: session, agent: id });
        }
    }
}

/// `AURORA_DEMO_CONV_MODERATE=menu|eject|ban|banned|error` (with
/// `AURORA_DEMO_CONV=aurora:group`): we moderate Aurora Builders' chat
/// with every ability; `menu` right-clicks the builder shown as "Tess" in
/// its members and opens "Moderator options", `eject` and `ban` ask about
/// her, `banned` bans her at once (the group's members and roles are asked
/// first, then the ban goes out), `error` forbids her text and the demo
/// refuses it.
pub fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::LazyLock<Option<String>> =
        std::sync::LazyLock::new(|| std::env::var("AURORA_DEMO_CONV_MODERATE").ok().map(|v| v.trim().to_owned()));
    MODE.as_deref()
}

#[derive(Clone, Copy, Debug, Hash)]
pub(super) enum DemoMark {
    /// The member's row in the panel.
    Row,
    /// The "Moderator options" heading of the menu.
    Submenu,
}

fn demo_id(mark: DemoMark) -> egui::Id {
    egui::Id::unique(("group_chat_moderation_demo", mark))
}

/// Where the demo's pointer goes: `menu` mode only.
pub(super) fn demo_mark(ctx: &egui::Context, mark: DemoMark, rect: egui::Rect) {
    if demo_mode() == Some("menu") {
        ctx.data_mut(|d| d.insert_temp(demo_id(mark), rect));
    }
}

/// `menu` mode: the right click on the member at frames 700 to 702, then
/// the pointer on "Moderator options" from frame 730 (capture at 800 with
/// `AURORA_FPS_LIMIT=60`).
pub(super) fn demo_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if demo_mode() != Some("menu") || !(700..=760).contains(&frame) {
        return Vec::new();
    }
    let at = |mark| ctx.data(|d| d.get_temp::<egui::Rect>(demo_id(mark)));
    match frame {
        700..=702 => {
            let Some(r) = at(DemoMark::Row) else { return Vec::new() };
            let pos = egui::pos2(r.left() + r.width() * 0.5, r.center().y);
            if frame == 700 {
                return vec![egui::Event::PointerMoved(pos)];
            }
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: frame == 701,
                modifiers: Default::default(),
            }]
        }
        730..=760 => match at(DemoMark::Submenu) {
            Some(r) => vec![egui::Event::PointerMoved(egui::pos2(
                r.left() + 40.0 + (frame - 730) as f32 * 0.1,
                r.center().y,
            ))],
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The confirmation of an ejection or a ban; Some(true) once confirmed,
/// Some(false) when cancelled.
pub fn confirm(ctx: &egui::Context, p: &Palette, world: &World, c: &Confirm) -> Option<bool> {
    let (agent, text, ok, icon) = match *c {
        Confirm::Eject { agent, .. } => (
            agent,
            aurora_i18n::tr!("ui-group-admin-eject-one", name = world.social.name_of(&agent)),
            aurora_i18n::tr!("ui-group-admin-eject"),
            "user-x",
        ),
        Confirm::Ban { agent, .. } => (
            agent,
            aurora_i18n::tr!("ui-group-admin-ban-one", name = world.social.name_of(&agent)),
            aurora_i18n::tr!("ui-group-admin-ban"),
            "ban",
        ),
    };
    let mut done = None;
    let modal = widgets::Modal::new(egui::Id::unique(("group_chat_moderation_confirm", agent))).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(egui::RichText::new(&text).size(12.5).color(p.ink));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = egui::Vec2::new(0.0, 26.0);
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
    done
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::groups::GroupMember;

    const ME: Uuid = Uuid::from_u128(1);
    const BOB: Uuid = Uuid::from_u128(2);
    const OWNER: Uuid = Uuid::from_u128(3);
    const OFFICER_ROLE: Uuid = Uuid::from_u128(40);
    const ALL: u64 = powers::MEMBER_EJECT | powers::ROLE_REMOVE_MEMBER | powers::GROUP_BAN_ACCESS;

    fn member(id: Uuid, is_owner: bool) -> GroupMember {
        GroupMember {
            id,
            contribution: 0,
            powers: 0,
            title: String::new(),
            last_seen: aurora_net::groups::LastSeen::Online,
            is_owner,
        }
    }

    /// A group whose members and roles are all known: Bob in "everyone"
    /// only, an owner.
    fn loaded() -> GroupData {
        let mut d = GroupData::default();
        d.members_complete = true;
        d.roles_complete = true;
        d.role_members_complete = true;
        for m in [member(ME, false), member(BOB, false), member(OWNER, true)] {
            d.member_roles.insert(m.id, vec![Uuid::nil()]);
            d.members.insert(m.id, m);
        }
        d
    }

    #[test]
    fn the_ejection_power_is_enough_while_the_members_are_not_known() {
        assert!(can_eject(None, powers::MEMBER_EJECT, BOB, ME));
        assert!(can_eject(Some(&GroupData::default()), powers::MEMBER_EJECT, BOB, ME));
        assert!(!can_eject(None, powers::GROUP_BAN_ACCESS, BOB, ME));
        assert!(!can_eject(None, ALL, ME, ME));
    }

    #[test]
    fn a_member_with_a_role_an_owner_or_a_stranger_is_not_ejected() {
        let mut d = loaded();
        assert!(can_eject(Some(&d), powers::MEMBER_EJECT, BOB, ME));
        assert!(!can_eject(Some(&d), powers::MEMBER_EJECT, OWNER, ME));
        assert!(!can_eject(Some(&d), powers::MEMBER_EJECT, Uuid::from_u128(9), ME));
        d.member_roles.get_mut(&BOB).expect("Bob").push(OFFICER_ROLE);
        assert!(!can_eject(Some(&d), powers::MEMBER_EJECT, BOB, ME));
        // the roles not all known yet: the role is not looked at
        d.role_members_complete = false;
        assert!(can_eject(Some(&d), powers::MEMBER_EJECT, BOB, ME));
    }

    #[test]
    fn a_ban_needs_both_powers_and_never_reaches_an_owner_or_ourselves() {
        let d = loaded();
        assert!(can_ban(Some(&d), ALL, BOB, ME));
        assert!(!can_ban(Some(&d), powers::GROUP_BAN_ACCESS, BOB, ME));
        assert!(!can_ban(Some(&d), powers::ROLE_REMOVE_MEMBER, BOB, ME));
        assert!(!can_ban(Some(&d), ALL, OWNER, ME));
        assert!(!can_ban(Some(&d), ALL, ME, ME));
        // a group never loaded: the ban loads it, and checks the owner then
        assert!(can_ban(None, ALL, BOB, ME));
    }
}
