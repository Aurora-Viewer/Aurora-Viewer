//! Ban a resident from every group where we may manage bans, or lift the
//! ban in all of them. Port of LLGroupActions::canBanFromAnyGroup,
//! confirmBanFromAllGroups, callbackBanFromAllGroups and
//! banFromAllGroupsCoro (indra/newview/llgroupactions.cpp), with the
//! muting of the server's ejection alerts of process_alert_core
//! (indra/newview/llviewermessage.cpp).
//!
//! The network sends one group after the other (`NetCommand::BanInGroups`);
//! each answer comes back here, which keeps our copy of the group's bans,
//! ejects a banned member, and reports once every group has answered.

use super::{World, attention, notifications};
use aurora_net::NetCommand;
use aurora_net::groups::powers;
use aurora_net::social::GroupMembership;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long the ejection alerts stay muted after the last answer: the
/// ejection replies trail the last request (suspendUntilTimeout(5.f)).
const QUIET_AFTER: Duration = Duration::from_secs(5);

/// The groups where we may manage bans (GP_GROUP_BAN_ACCESS), by name in
/// dictionary order: (id, name).
pub fn ban_groups(groups: &[GroupMembership]) -> Vec<(Uuid, String)> {
    let mut v: Vec<(Uuid, String)> = groups
        .iter()
        .filter(|g| g.powers & powers::GROUP_BAN_ACCESS != 0)
        .map(|g| (g.id, g.name.clone()))
        .collect();
    v.sort_by_cached_key(|(_, name)| crate::natural_order::key(name));
    v
}

/// canBanFromAnyGroup: someone else, and one group at least where we may
/// manage bans.
pub fn can_ban_anywhere(groups: &[GroupMembership], agent: Uuid, me: Uuid) -> bool {
    !agent.is_nil() && agent != me && groups.iter().any(|g| g.powers & powers::GROUP_BAN_ACCESS != 0)
}

/// One ban (or lifting) sent to several groups.
#[derive(Debug, Clone)]
struct Run {
    agent: Uuid,
    ban: bool,
    groups: Vec<(Uuid, String)>,
    answered: Vec<Uuid>,
    succeeded: Vec<String>,
    failed: Vec<String>,
}

/// What a finished run tells (FSBanFromAllGroupsReport /
/// FSUnbanFromAllGroupsReport).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub agent: Uuid,
    pub ban: bool,
    pub count: usize,
    pub succeeded: Vec<String>,
    pub failed: Vec<String>,
}

/// The runs on their way, and the end of the alerts' muting.
#[derive(Debug, Default)]
pub struct BanAll {
    runs: Vec<Run>,
    quiet_until: Option<Instant>,
}

impl BanAll {
    /// A run starts; the command that sends it.
    pub fn start(&mut self, agent: Uuid, ban: bool, groups: Vec<(Uuid, String)>) -> NetCommand {
        let ids = groups.iter().map(|(id, _)| *id).collect();
        self.runs.push(Run {
            agent,
            ban,
            groups,
            answered: Vec::new(),
            succeeded: Vec::new(),
            failed: Vec::new(),
        });
        NetCommand::BanInGroups { agent, groups: ids, ban }
    }

    /// The answer of one group; the report once the run's last group
    /// answered. An answer no run waits for is dropped.
    pub fn answer(&mut self, group: Uuid, agent: Uuid, ban: bool, ok: bool, now: Instant) -> Option<Report> {
        let i = self
            .runs
            .iter()
            .position(|r| r.agent == agent && r.ban == ban && !r.answered.contains(&group) && r.groups.iter().any(|(g, _)| *g == group))?;
        let run = &mut self.runs[i];
        run.answered.push(group);
        let name = run
            .groups
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        if ok {
            run.succeeded.push(name);
        } else {
            run.failed.push(name);
        }
        if run.answered.len() < run.groups.len() {
            return None;
        }
        let run = self.runs.remove(i);
        if self.runs.is_empty() {
            self.quiet_until = Some(now + QUIET_AFTER);
        }
        Some(Report {
            agent: run.agent,
            ban: run.ban,
            count: run.groups.len(),
            succeeded: run.succeeded,
            failed: run.failed,
        })
    }

    /// process_alert_core while a run is on its way (and 5 s after): the
    /// server answers every ejection of a non-member with an alert.
    pub fn mutes_alert(&self, message: &str, now: Instant) -> bool {
        let running = !self.runs.is_empty() || self.quiet_until.is_some_and(|t| now < t);
        if !running {
            return false;
        }
        let m = message.to_lowercase();
        m.contains("unable to eject") || m.contains("ejectee")
    }
}

impl World {
    /// callbackBanFromAllGroups, once confirmed: the groups asked again (a
    /// power may have gone since the question), the notice, then the run.
    pub fn ban_in_all_groups(&mut self, agent: Uuid, ban: bool) {
        if !can_ban_anywhere(&self.groups.groups, agent, self.agent_id) {
            return;
        }
        let groups = ban_groups(&self.groups.groups);
        let name = self.social.name_of(&agent);
        self.notifications.push(
            notifications::Kind::Group,
            aurora_i18n::tr!("world-ban-all-title"),
            aurora_i18n::tr!("world-ban-all-started", name = name, count = groups.len()),
            notifications::Data::None,
        );
        let cmd = self.ban_all.start(agent, ban, groups);
        self.group_data.out.push(cmd);
    }

    /// One group's answer: our copy of its bans (createBanEntry /
    /// removeBanEntry), the ejection of a banned member (a ban alone does
    /// not remove them; not when the members' list says they are not
    /// one), then the report once all have answered.
    pub(super) fn on_group_ban_in_group(&mut self, group: Uuid, agent: Uuid, ban: bool, ok: bool) {
        if ok {
            self.group_data.note_ban(group, agent, ban);
            let known_non_member = self
                .group_data
                .get(&group)
                .is_some_and(|g| g.members_complete && !g.members.contains_key(&agent));
            let can_eject = self.groups.group(&group).is_some_and(|g| g.powers & powers::MEMBER_EJECT != 0);
            if ban && !known_non_member && can_eject {
                self.group_data.eject(group, vec![agent], self.agent_id);
            }
        }
        let Some(r) = self.ban_all.answer(group, agent, ban, ok, Instant::now()) else {
            return;
        };
        let list = |v: &[String]| if v.is_empty() { "-".to_owned() } else { v.join(", ") };
        let (name, count, done) = (self.social.name_of(&r.agent), r.count, r.succeeded.len());
        let (succeeded, failed) = (list(&r.succeeded), list(&r.failed));
        let text = if r.ban {
            aurora_i18n::tr!(
                "world-ban-all-report",
                name = name,
                done = done,
                count = count,
                succeeded = succeeded,
                failed = failed
            )
        } else {
            aurora_i18n::tr!(
                "world-unban-all-report",
                name = name,
                done = done,
                count = count,
                succeeded = succeeded,
                failed = failed
            )
        };
        log::info!("group bans of {}: {done} of {count} groups", r.agent);
        let id = self.notifications.push(
            notifications::Kind::Group,
            aurora_i18n::tr!("world-ban-all-title"),
            text,
            notifications::Data::None,
        );
        // the Legacy report is an alert: it asks for attention
        let notice = self.notice_of(id, attention::Open::Notifications);
        self.ask_attention(attention::Cause::Notice, notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Uuid = Uuid::from_u128(1);
    const BOB: Uuid = Uuid::from_u128(2);
    const G1: Uuid = Uuid::from_u128(10);
    const G2: Uuid = Uuid::from_u128(11);
    const G3: Uuid = Uuid::from_u128(12);

    fn group(id: Uuid, name: &str, powers: u64) -> GroupMembership {
        GroupMembership {
            id,
            name: name.into(),
            insignia: Uuid::nil(),
            powers,
            accept_notices: true,
            list_in_profile: true,
            contribution: 0,
        }
    }

    fn mine() -> Vec<GroupMembership> {
        vec![
            group(G1, "zebra club", powers::GROUP_BAN_ACCESS),
            group(G2, "Builders 10", powers::GROUP_BAN_ACCESS | powers::MEMBER_EJECT),
            group(G3, "builders 2", 0),
        ]
    }

    #[test]
    fn only_the_groups_where_we_manage_bans_in_dictionary_order() {
        let v = ban_groups(&mine());
        assert_eq!(v, vec![(G2, "Builders 10".into()), (G1, "zebra club".into())]);
        let mut all = mine();
        all.push(group(Uuid::from_u128(13), "Builders 9", powers::GROUP_BAN_ACCESS));
        let names: Vec<String> = ban_groups(&all).into_iter().map(|(_, n)| n).collect();
        assert_eq!(names, ["Builders 9", "Builders 10", "zebra club"]);
    }

    #[test]
    fn never_ourselves_nobody_or_without_a_ban_power() {
        assert!(can_ban_anywhere(&mine(), BOB, ME));
        assert!(!can_ban_anywhere(&mine(), ME, ME));
        assert!(!can_ban_anywhere(&mine(), Uuid::nil(), ME));
        assert!(!can_ban_anywhere(&[group(G3, "builders 2", powers::MEMBER_EJECT)], BOB, ME));
        assert!(!can_ban_anywhere(&[], BOB, ME));
    }

    #[test]
    fn the_report_comes_once_every_group_answered() {
        let mut b = BanAll::default();
        let cmd = b.start(BOB, true, ban_groups(&mine()));
        assert!(matches!(cmd, NetCommand::BanInGroups { agent: BOB, ref groups, ban: true } if *groups == [G2, G1]));
        let now = Instant::now();
        assert_eq!(b.answer(G2, BOB, true, true, now), None);
        // an answer no run waits for, twice the same group, the other way
        assert_eq!(b.answer(G3, BOB, true, true, now), None);
        assert_eq!(b.answer(G2, BOB, true, true, now), None);
        assert_eq!(b.answer(G1, BOB, false, true, now), None);
        let r = b.answer(G1, BOB, true, false, now).expect("the report");
        assert_eq!(r.count, 2);
        assert_eq!(r.succeeded, ["Builders 10"]);
        assert_eq!(r.failed, ["zebra club"]);
        assert_eq!(b.answer(G1, BOB, true, true, now), None);
    }

    #[test]
    fn ejection_alerts_are_muted_while_it_runs_and_five_seconds_after() {
        let mut b = BanAll::default();
        let now = Instant::now();
        let alert = "Unable to eject the ejectee: not a member";
        assert!(!b.mutes_alert(alert, now));
        b.start(BOB, true, vec![(G1, "zebra club".into())]);
        assert!(b.mutes_alert(alert, now));
        assert!(!b.mutes_alert("The region is restarting", now));
        b.answer(G1, BOB, true, true, now);
        assert!(b.mutes_alert(alert, now + Duration::from_secs(4)));
        assert!(!b.mutes_alert(alert, now + Duration::from_secs(6)));
    }
}
