//! Recent residents: LLRecentPeople (indra/newview/llrecentpeople.cpp,
//! LGPL 2.1). Aurora intentionally keeps the newest 1,000 across sessions
//! and merges the shared Chat Service manager's validated direct participants.
//! Neither reference viewer adds Chat Service participants to Recent; this
//! extension was requested to keep contacts recoverable after exits/crashes.

use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// Bound both the visible list and the saved snapshot, newest activity first.
pub const MAX_PEOPLE: usize = 1_000;
/// Region labels are metadata, not an unbounded part of the saved file.
const MAX_REGION_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Met {
    pub id: Uuid,
    pub when: SystemTime,
    pub region: String,
}

impl Met {
    fn bound_region(&mut self) {
        let mut end = self.region.len().min(MAX_REGION_BYTES);
        while !self.region.is_char_boundary(end) {
            end -= 1;
        }
        self.region.truncate(end);
    }
}

#[derive(Debug, Default)]
pub struct RecentPeople {
    people: Vec<Met>,
    pub(super) revision: u64,
    /// Names still being resolved for the accepted list, never serialized.
    pending: Vec<Met>,
    name_deadline: Option<Instant>,
}

impl RecentPeople {
    /// LLRecentPeople::add: exclude ourselves, nil ids and group sessions.
    pub fn add(&mut self, me: Uuid, id: Uuid, is_group: bool, region: &str, when: SystemTime) {
        if id == me || id.is_nil() || is_group {
            return;
        }
        let mut entry = Met {
            id,
            when,
            region: region.to_owned(),
        };
        entry.bound_region();
        if self.people.first() == Some(&entry) {
            return;
        }
        self.people.retain(|m| m.id != id);
        self.people.insert(0, entry);
        self.people.truncate(MAX_PEOPLE);
        self.revision = self.revision.wrapping_add(1);
    }

    /// Merge by identity and original time, then retain only the newest people.
    /// A legacy file or a service list cannot grow the persisted list unbounded.
    pub fn merge(&mut self, me: Uuid, entries: impl IntoIterator<Item = Met>) {
        let mut by_id: std::collections::HashMap<Uuid, Met> = self.people.iter().cloned().map(|m| (m.id, m)).collect();
        for mut entry in entries {
            if entry.id.is_nil() || entry.id == me {
                continue;
            }
            if by_id.get(&entry.id).is_none_or(|old| old.when < entry.when) {
                entry.bound_region();
                by_id.insert(entry.id, entry);
            }
        }
        let mut people: Vec<_> = by_id.into_values().collect();
        people.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.id.cmp(&b.id)));
        people.truncate(MAX_PEOPLE);
        if people != self.people {
            self.people = people;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn list(&self) -> &[Met] {
        &self.people
    }
    pub fn len(&self) -> usize {
        self.people.len()
    }
    pub fn is_empty(&self) -> bool {
        self.people.is_empty()
    }
}

impl super::World {
    /// A side effect of the existing manager's accepted response, never a
    /// second HTTP client. Its tickets reject old-login and old-region replies.
    pub(super) fn on_recent_conversations(&mut self, entries: Vec<aurora_net::chat_service::ListEntry>) {
        if !self.recent_service_allowed() {
            return;
        }
        self.recent.pending = entries
            .into_iter()
            .filter_map(|entry| {
                let seconds = aurora_net::chat_service::parse_time_uuid(&entry.last_msg_id)?.unix_seconds()?;
                let when = SystemTime::UNIX_EPOCH.checked_add(Duration::try_from_secs_f64(seconds).ok()?)?;
                (!self.mutes.is_muted_id(&entry.resident_id)).then_some(Met {
                    id: entry.resident_id,
                    when,
                    region: String::new(),
                })
            })
            .collect();
        self.recent
            .pending
            .sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.id.cmp(&b.id)));
        self.recent.pending.truncate(MAX_PEOPLE);
        for person in &self.recent.pending {
            self.social.want_name(person.id);
        }
        self.recent.name_deadline = Some(Instant::now() + super::chat_service::NAME_TIMEOUT);
        self.poll_recent_participants();
    }

    fn recent_service_allowed(&self) -> bool {
        // UI settings may have changed before this frame feeds the manager's
        // gates; never publish a response during that small consent gap.
        self.chat_log.cfg.log_im
            && self.chat_service.running()
            && self.chat_service.manager.service_presentation_allowed()
            && self.mutes.is_loaded_from_server()
    }

    /// Pending names use the same authoritative name/mute rules as the manager.
    /// Revoked consent or a degraded mute list cannot publish pending entries.
    pub(super) fn poll_recent_participants(&mut self) {
        if !self.recent_service_allowed() {
            self.recent.pending.clear();
            return;
        }
        let expired = self.recent.name_deadline.is_some_and(|t| Instant::now() >= t);
        let mut ready = Vec::new();
        self.recent.pending.retain(|p| {
            if p.id == self.agent_id || self.groups.is_member(&p.id) || self.mutes.is_muted_id(&p.id) {
                return false;
            }
            if let Some(name) = self.social.avatar_names.raw(&p.id).filter(|n| !n.temporary) {
                if !self.mutes.is_muted(&p.id, &name.user_name(), 0) {
                    ready.push(p.clone());
                }
                false
            } else {
                !expired
            }
        });
        if !ready.is_empty() {
            self.recent.merge(self.agent_id, ready);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_retention_caps_live_and_restored_entries_by_newest_activity() {
        let me = Uuid::from_u128(1);
        let mut recent = RecentPeople::default();
        for i in 2..MAX_PEOPLE + 202 {
            recent.add(
                me,
                Uuid::from_u128(i as u128),
                false,
                "",
                SystemTime::UNIX_EPOCH + Duration::from_secs(i as u64),
            );
        }
        assert_eq!(recent.len(), MAX_PEOPLE);
        assert_eq!(recent.list()[0].id, Uuid::from_u128((MAX_PEOPLE + 201) as u128));
        let original = recent.list().to_vec();
        recent.merge(
            me,
            (2..202).map(|i| Met {
                id: Uuid::from_u128(i),
                when: SystemTime::UNIX_EPOCH,
                region: "".into(),
            }),
        );
        assert_eq!(recent.list(), original);
        recent.add(
            me,
            Uuid::from_u128(2),
            false,
            "",
            SystemTime::UNIX_EPOCH + Duration::from_secs(100_000),
        );
        assert_eq!(recent.list()[0].id, Uuid::from_u128(2));
        assert_eq!(recent.len(), MAX_PEOPLE);
        let mut restored = RecentPeople::default();
        restored.merge(me, original.iter().cloned().chain(recent.list().iter().cloned()));
        assert_eq!(restored.list(), recent.list());
    }

    #[test]
    fn recent_metadata_stays_bounded_and_has_one_capability_provider() {
        let mut recent = RecentPeople::default();
        let me = Uuid::from_u128(1);
        recent.merge(
            me,
            (2..MAX_PEOPLE + 2).map(|i| Met {
                id: Uuid::from_u128(i as u128),
                when: SystemTime::UNIX_EPOCH,
                region: "界".repeat(10_000),
            }),
        );
        assert!(recent.list().iter().all(|m| m.region.len() <= MAX_REGION_BYTES));
        assert!(serde_json::to_vec(recent.list()).unwrap().len() < 2 * 1024 * 1024);
        assert_eq!(
            aurora_net::caps::REQUESTED_CAPS
                .iter()
                .filter(|&&c| c == super::super::direct_history::LIST_CAP)
                .count(),
            1
        );
    }

    #[test]
    fn newest_first_without_us_nor_groups() {
        let (me, a, b, g) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        let t = SystemTime::UNIX_EPOCH;
        let mut r = RecentPeople::default();
        r.add(me, a, false, "Ahern", t);
        r.add(me, b, false, "Ahern", t);
        r.add(me, me, false, "Ahern", t);
        r.add(me, g, true, "Ahern", t);
        assert_eq!(r.list().iter().map(|m| m.id).collect::<Vec<_>>(), vec![b, a]);
        // met again: back on top, with where we were then
        r.add(me, a, false, "Morris", t);
        assert_eq!(
            r.list()[0],
            Met {
                id: a,
                when: t,
                region: "Morris".into()
            }
        );
        assert_eq!(r.len(), 2);
    }
}
