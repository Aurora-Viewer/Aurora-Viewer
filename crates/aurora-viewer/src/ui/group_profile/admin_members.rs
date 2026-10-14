//! The members' management of the Members tab (LLPanelGroupMembersSubTab,
//! indra/newview/llpanelgrouproles.cpp, originally LGPL 2.1): which of the
//! chosen members may be ejected or banned, which of their roles we may add
//! or remove (agentCanAddToRole, agentCanRemoveFromRole, handleMemberSelect),
//! the role changes waiting for "Save" (handleRoleCheck, applyMemberChanges),
//! what they can do (buildActionsList) and the export of the list
//! (onExportMembersToXMLCallback, Firestorm's FIRE-12276).

use super::abilities::AbilitySet;
use crate::world::group_data::GroupData;
use aurora_net::groups::{RoleChange, powers};
use uuid::Uuid;

/// Who we are in the group.
#[derive(Clone, Copy, Debug)]
pub struct Me {
    pub id: Uuid,
    pub powers: u64,
    pub owner_role: Uuid,
}

impl Me {
    fn can(&self, p: u64) -> bool {
        self.powers & p != 0
    }

    fn is_owner(&self, d: &GroupData) -> bool {
        in_role(d, &[], &self.id, &self.owner_role)
    }
}

/// Whether `member` is in `role` once `staged` is applied.
pub fn in_role(d: &GroupData, staged: &[RoleChange], member: &Uuid, role: &Uuid) -> bool {
    if role.is_nil() {
        return d.members.contains_key(member);
    }
    let mut has = d.member_roles.get(member).is_some_and(|r| r.contains(role));
    for c in staged.iter().filter(|c| c.member == *member && c.role == *role) {
        has = c.add;
    }
    has
}

/// agentCanAddToRole: owners to any role, "assign members to our roles" to
/// the roles we have, "assign members" to any role but the owners'.
pub fn can_add_to_role(d: &GroupData, me: &Me, role: &Uuid) -> bool {
    if !d.members.contains_key(&me.id) {
        return false;
    }
    me.is_owner(d)
        || (me.can(powers::ROLE_ASSIGN_MEMBER_LIMITED) && in_role(d, &[], &me.id, role))
        || (me.can(powers::ROLE_ASSIGN_MEMBER) && *role != me.owner_role)
}

/// What the eject and ban buttons may do for the members chosen
/// (handleMemberSelect).
pub fn eject_and_ban(d: &GroupData, me: &Me, selected: &[Uuid]) -> (bool, bool) {
    if selected.is_empty() {
        return (false, false);
    }
    let mut eject = me.can(powers::MEMBER_EJECT);
    let mut ban = me.can(powers::GROUP_BAN_ACCESS);
    let mut member_is_owner = false;
    let other_owner = selected.iter().any(|m| *m != me.id && in_role(d, &[], m, &me.owner_role));
    // removing someone from the owners' role is how the owners' checkbox is
    // enabled; another owner chosen forbids it, and the ban with it
    if me.can(powers::ROLE_REMOVE_MEMBER) && other_owner {
        ban = false;
    }
    for m in selected {
        let roles = d.member_roles.get(m).map(Vec::as_slice).unwrap_or_default();
        // in any role besides "everyone": not ejected that way
        if roles.iter().any(|r| !r.is_nil()) {
            eject = false;
        }
        if roles.contains(&me.owner_role) && !me.owner_role.is_nil() {
            member_is_owner = true;
        }
    }
    // an owner may eject anyone but another owner
    if !eject && !member_is_owner && me.is_owner(d) {
        eject = true;
    }
    // ...as may who has both the removal from roles and the ejection
    if me.can(powers::ROLE_REMOVE_MEMBER) && !member_is_owner {
        if me.can(powers::MEMBER_EJECT) {
            eject = true;
        }
        if me.can(powers::GROUP_BAN_ACCESS) {
            ban = true;
        }
    }
    if selected.contains(&me.id) {
        return (false, false);
    }
    (eject, ban)
}

/// One role of the chosen members' roles card.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleCheck {
    pub role: Uuid,
    /// How many of the chosen are in it (the changes waiting included).
    pub count: usize,
    pub enabled: bool,
}

/// The roles card: each role, how many of the chosen have it, and whether
/// we may change that.
pub fn role_checks(d: &GroupData, me: &Me, selected: &[Uuid], staged: &[RoleChange]) -> Vec<RoleCheck> {
    let mut roles: Vec<&aurora_net::groups::GroupRole> = d.roles.values().collect();
    roles.sort_by_cached_key(|r| {
        (
            !r.id.is_nil(),
            r.id != me.owner_role,
            super::members::role_name(&r.name).to_lowercase(),
        )
    });
    roles
        .into_iter()
        .map(|r| {
            let saved = selected.iter().filter(|m| in_role(d, &[], m, &r.id)).count();
            let count = selected.iter().filter(|m| in_role(d, staged, m, &r.id)).count();
            // the ability goes by the saved state (handleMemberSelect)
            let mut enabled = if saved > 0 {
                me.can(powers::ROLE_REMOVE_MEMBER)
            } else {
                can_add_to_role(d, me, &r.id)
            };
            // other owners cannot be removed from the owners' role
            if enabled && saved > 0 && r.id == me.owner_role {
                enabled = !selected.iter().any(|m| *m != me.id && in_role(d, &[], m, &me.owner_role));
            }
            // nobody leaves "everyone"
            if r.id.is_nil() {
                enabled = false;
            }
            RoleCheck {
                role: r.id,
                count,
                enabled,
            }
        })
        .collect()
}

/// handleRoleCheck: a click adds the role to all the chosen when some lack
/// it, removes it from all when all have it; a change back cancels the
/// waiting one.
pub fn toggle_role(d: &GroupData, selected: &[Uuid], staged: &mut Vec<RoleChange>, role: Uuid) {
    let all = selected.iter().all(|m| in_role(d, staged, m, &role));
    let add = !all;
    for m in selected {
        if in_role(d, staged, m, &role) == add {
            continue;
        }
        let saved = in_role(d, &[], m, &role);
        staged.retain(|c| !(c.member == *m && c.role == role));
        if saved != add {
            staged.push(RoleChange { role, member: *m, add });
        }
    }
}

/// The powers the chosen have through their roles, the waiting changes
/// included (getAgentPowersBasedOnRoleChanges): by some, by all.
pub fn powers_of(d: &GroupData, selected: &[Uuid], staged: &[RoleChange]) -> (u64, u64) {
    let mut some = 0;
    let mut all = u64::MAX;
    for m in selected {
        let p = if d.members.get(m).is_some_and(|x| x.is_owner) {
            u64::MAX
        } else {
            d.roles
                .values()
                .filter(|r| in_role(d, staged, m, &r.id))
                .fold(0, |acc, r| acc | r.powers)
        };
        some |= p;
        all &= p;
    }
    if selected.is_empty() {
        all = 0;
    }
    (some, all)
}

/// The abilities of a set the chosen have (by some), and how many it has.
pub fn set_count(set: &AbilitySet, some: u64) -> (usize, usize) {
    let has = set.abilities.iter().filter(|a| some & (1u64 << a.bit) != 0).count();
    (has, set.abilities.len())
}

/// The list as Firestorm exports it: a title line, then key, name, last
/// online, land contribution.
pub fn export_csv(group_name: &str, d: &GroupData, name: &dyn Fn(&Uuid) -> String) -> String {
    let mut out = format!("Group membership record for {group_name} (avatar key, avatar name, last online, land contribution)");
    let mut ids: Vec<&Uuid> = d.members.keys().collect();
    ids.sort();
    for id in ids {
        let m = &d.members[id];
        let seen = match &m.last_seen {
            aurora_net::groups::LastSeen::Online => "Online".to_owned(),
            aurora_net::groups::LastSeen::Date(y, mo, da) => format!("{mo:02}/{da:02}/{y}"),
            aurora_net::groups::LastSeen::Other(t) => t.clone(),
        };
        out.push_str(&format!("\n{id},{},{seen},{}", name(id), m.contribution));
    }
    out.push('\n');
    out
}
