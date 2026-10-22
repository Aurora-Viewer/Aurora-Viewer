//! RlvHandler::checkActiveGroupThrottle, setActiveGroupRole and
//! onActiveGroupChanged (rlvhandler.cpp, originally LGPL 2.1).
use super::{super::World, Command};
use aurora_net::NetCommand;
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const ACTOR: Uuid = Uuid::from_u128(0x726c_7661_6772_6f75_7072_6573_746f_7265);

#[derive(Default)]
pub struct Groups {
    seen: Option<Uuid>,
    expected: Option<Uuid>,
    changed: Option<Instant>,
    throttle_owner: Option<Uuid>,
    pending: Option<(Uuid, Uuid, String)>,
    restore: Option<Uuid>,
}

impl Groups {
    pub fn restore_allowed(&self, group: Uuid) -> bool {
        self.restore == Some(group) && self.expected == Some(group)
    }
    fn allow(&mut self, source: Uuid, owner: bool, now: Instant) -> bool {
        if self
            .changed
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(60))
        {
            self.changed = Some(now);
            self.throttle_owner = None;
            true
        } else if owner && self.throttle_owner.is_none() {
            self.changed = Some(now);
            self.throttle_owner = Some(source);
            true
        } else {
            false
        }
    }
}

impl World {
    /// UUIDs must identify a membership; names prefer an exact match, then
    /// the last case-insensitive prefix match in the native membership list.
    fn rlva_group_id(&self, name: &str) -> Option<Uuid> {
        if name == "none" {
            return Some(Uuid::nil());
        }
        if let Ok(id) = Uuid::parse_str(name) {
            return (id.is_nil() || self.groups.is_member(&id)).then_some(id);
        }
        if name.is_empty() {
            return None;
        }
        let name = name.to_lowercase();
        let mut found = None;
        for group in &self.groups.groups {
            let candidate = group.name.to_lowercase();
            if candidate.starts_with(&name) {
                found = Some(group.id);
                if candidate == name {
                    break;
                }
            }
        }
        found
    }

    pub(super) fn rlva_group_force(&mut self, source: Uuid, c: &Command, now: Instant) -> Vec<(Uuid, NetCommand)> {
        if !self.rlva.enabled() || self.rlva.has_except("setgroup", source) {
            return Vec::new();
        }
        let mut parts = c.option.split(';');
        let Some(group) = parts.next().and_then(|name| self.rlva_group_id(name)) else {
            return Vec::new();
        };
        let role = parts.next();
        if parts.next().is_some() {
            return Vec::new();
        }
        let owner = self.rlva.rules_for("setgroup", Uuid::nil()).any(|(id, _)| id == source);
        if !self.rlva.groups.allow(source, owner, now) {
            return Vec::new();
        }
        if self.rlva.groups.seen.is_none() {
            self.rlva.groups.seen = Some(self.groups.active);
        }
        self.rlva.groups.expected = Some(group);
        self.rlva.groups.restore = None;
        self.rlva.groups.pending = role.filter(|_| !group.is_nil()).map(|role| (source, group, role.to_owned()));
        let mut commands = Vec::new();
        if self.groups.active != group {
            commands.push((source, NetCommand::ActivateGroup(group)));
        }
        commands.extend(self.rlva_group_tick());
        commands
    }

    /// A newly joined group can become active on the simulator without a
    /// local ActivateGroup action. Restore the last permitted group once per
    /// actual change, and retain the actor while role data loads asynchronously.
    pub fn rlva_group_tick(&mut self) -> Vec<(Uuid, NetCommand)> {
        if !self.rlva.enabled() {
            return Vec::new();
        }
        let current = self.groups.active;
        let changed = self.rlva.groups.seen != Some(current);
        self.rlva.groups.seen = Some(current);
        let mut commands = Vec::new();
        if changed {
            if self.rlva.has("setgroup") && self.rlva.groups.expected.is_some_and(|expected| expected != current) {
                let expected = self
                    .rlva
                    .groups
                    .expected
                    .filter(|id| id.is_nil() || self.groups.is_member(id))
                    .unwrap_or_default();
                self.rlva.groups.expected = Some(expected);
                // This precise restoration bypasses the user action guard;
                // it never grants a script another group's permissions.
                self.rlva.groups.restore = Some(expected);
                commands.push((ACTOR, NetCommand::ActivateGroup(expected)));
            } else {
                self.rlva.groups.expected = Some(current);
                self.rlva.groups.restore = None;
            }
        }
        let Some((source, group, role)) = self.rlva.groups.pending.clone() else {
            return commands;
        };
        if self.rlva.has_except("setgroup", source) || !self.groups.is_member(&group) {
            self.rlva.groups.pending = None;
        } else if current == group {
            if self.group_data.get(&group).is_none_or(|g| !g.roles_complete) {
                self.group_data.request_roles(group);
            } else {
                let role = role.to_lowercase();
                let data = &self.group_data.groups[&group];
                let mut roles: Vec<_> = data.roles.values().collect();
                roles.sort_by_key(|r| r.id);
                let mut found = None;
                for candidate in roles {
                    let name = candidate.name.to_lowercase();
                    if name.starts_with(&role) {
                        found = Some(candidate.id);
                        if name == role {
                            break;
                        }
                    }
                }
                if let Some(role) = found {
                    commands.push((source, NetCommand::GroupTitleUpdate { group, role }));
                }
                self.rlva.groups.pending = None;
            }
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const G1: Uuid = Uuid::from_u128(10);
    const G2: Uuid = Uuid::from_u128(11);

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.rlva.configure(true);
        for (id, name) in [(G1, "Example"), (G2, "Example Extended")] {
            w.groups.groups.push(aurora_net::social::GroupMembership {
                id,
                name: name.into(),
                insignia: Uuid::nil(),
                powers: 0,
                accept_notices: true,
                list_in_profile: true,
                contribution: 0,
            });
        }
        w.groups.active = G1;
        w.rlva_group_tick();
        w
    }

    fn command(option: &str) -> Command {
        Command::parse(&format!("setgroup:{option}=force")).unwrap()
    }

    #[test]
    fn rlva_group_names_membership_throttle_and_lock_owner_exception() {
        let mut w = world();
        assert_eq!(w.rlva_group_id("example"), Some(G1));
        assert_eq!(w.rlva_group_id("exam"), Some(G2), "last partial match wins");
        assert_eq!(w.rlva_group_id("none"), Some(Uuid::nil()));
        assert_eq!(w.rlva_group_id(&Uuid::nil().to_string()), Some(Uuid::nil()));
        assert_eq!(w.rlva_group_id(&B.to_string()), None);
        assert_eq!(w.rlva_group_id(""), None);
        let now = Instant::now();
        assert!(matches!(&w.rlva_group_force(A, &command("exam"), now)[..], [(id, NetCommand::ActivateGroup(g))] if *id == A && *g == G2));
        assert!(w.rlva_group_force(B, &command("none"), now).is_empty());
        w.rlva.process(A, Command::parse("setgroup=n").unwrap());
        assert!(
            matches!(&w.rlva_group_force(A, &command("none"), now)[..], [(id, NetCommand::ActivateGroup(g))] if *id == A && g.is_nil())
        );
        assert!(
            w.rlva_group_force(A, &command("exam"), now).is_empty(),
            "owner only gets one early change"
        );
        assert!(!w.rlva_group_force(A, &command("exam"), now + Duration::from_secs(60)).is_empty());
        assert!(
            w.rlva_group_force(B, &command("example"), now + Duration::from_secs(120))
                .is_empty(),
            "other issuers obey setgroup"
        );
        assert!(
            w.rlva_group_force(A, &command("example;role;extra"), now + Duration::from_secs(120))
                .is_empty()
        );
    }

    #[test]
    fn rlva_group_roles_wait_for_native_data_and_retain_the_authorized_actor() {
        let mut w = world();
        let now = Instant::now();
        w.rlva.process(A, Command::parse("setgroup=n").unwrap());
        assert!(w.rlva_group_force(A, &command("example;Officer"), now).is_empty());
        assert!(matches!(&w.group_data.take_commands()[..], [NetCommand::GroupRoleDataRequest { group, .. }] if *group == G1));
        assert!(w.rlva_group_tick().is_empty());
        assert!(w.group_data.take_commands().is_empty(), "one request while role data is pending");
        let data = w.group_data.groups.get_mut(&G1).unwrap();
        data.roles_complete = true;
        for (id, name) in [(A, "Officer"), (B, "Officers Extended")] {
            data.roles.insert(
                id,
                aurora_net::groups::GroupRole {
                    id,
                    name: name.into(),
                    title: String::new(),
                    description: String::new(),
                    powers: 0,
                    members: 1,
                },
            );
        }
        let commands = w.rlva_group_tick();
        assert!(
            matches!(&commands[..], [(actor, NetCommand::GroupTitleUpdate { group, role })] if *actor == A && *group == G1 && *role == A)
        );
        assert!(w.rlva_allows_command(&commands[0].1, A));
        assert!(!w.rlva_allows_command(&commands[0].1, Uuid::nil()));
        assert!(w.rlva_group_tick().is_empty());
        assert!(
            matches!(&w.rlva_group_force(A, &command("example;Off"), now)[..], [(_, NetCommand::GroupTitleUpdate { role, .. })] if *role == B),
            "last role prefix wins"
        );
        w.rlva_group_force(A, &command("Example Extended;Officer"), now + Duration::from_secs(60));
        w.groups.active = G2;
        w.rlva_group_tick();
        w.rlva.process(B, Command::parse("setgroup=n").unwrap());
        assert!(w.rlva_group_tick().is_empty());
        assert!(w.rlva.groups.pending.is_none(), "a newly added lock cancels the delayed write");
        w.rlva.configure(false);
        assert!(w.rlva_group_tick().is_empty());
        assert!(w.rlva_allows_command(&commands[0].1, Uuid::nil()));
    }

    #[test]
    fn rlva_group_restoration_only_authorizes_the_precise_last_permitted_group() {
        let mut w = world();
        w.rlva.process(A, Command::parse("setgroup=n").unwrap());
        w.groups.active = G2;
        let restored = w.rlva_group_tick();
        assert!(matches!(&restored[..], [(actor, NetCommand::ActivateGroup(group))] if *actor == ACTOR && *group == G1));
        assert!(w.rlva_allows_command(&restored[0].1, ACTOR));
        assert!(!w.rlva_allows_command(&NetCommand::ActivateGroup(G2), ACTOR));
        assert!(
            w.rlva_group_tick().is_empty(),
            "no per-frame activation flood while awaiting the acknowledgement"
        );
        w.groups.active = G1;
        w.rlva_group_tick();
        assert!(
            !w.rlva_allows_command(&NetCommand::ActivateGroup(G1), ACTOR),
            "acknowledgement releases the internal bypass"
        );
        w.groups.groups.retain(|g| g.id != G1);
        w.groups.active = G2;
        assert!(matches!(&w.rlva_group_tick()[..], [(_, NetCommand::ActivateGroup(group))] if group.is_nil()));
        w.rlva.configure(false);
        assert!(w.rlva_group_tick().is_empty());
    }
}
