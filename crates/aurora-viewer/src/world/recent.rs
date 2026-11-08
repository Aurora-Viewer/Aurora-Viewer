//! The residents we dealt with lately, for People › Recent: port of
//! LLRecentPeople (indra/newview/llrecentpeople.cpp, originally LGPL 2.1).
//! Someone joins it when they talk to us in the nearby chat or by IM, or
//! when we open a conversation with them (LLAvatarActions::startIM), with the
//! date and, Aurora's addition for the list's second line, the region we
//! were in. Aurora additionally persists this list across sessions (social_history.rs).

use std::time::SystemTime;
use uuid::Uuid;

/// One resident of the list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Met {
    pub id: Uuid,
    pub when: SystemTime,
    /// The region we were in then.
    pub region: String,
}

#[derive(Debug, Default)]
pub struct RecentPeople {
    /// Newest first (the Recent tab's order, LLAvatarItemRecentComparator).
    people: Vec<Met>,
    pub(super) revision: u64,
    /// Login-scoped discovery; these fields never enter the saved file.
    server_request: Option<Uuid>,
    server_people: Vec<aurora_net::persistent_chat::Participant>,
    server_deadline: Option<std::time::Instant>,
}

impl RecentPeople {
    /// LLRecentPeople::add: never ourselves nor a group (a group's chat
    /// session carries the group's id); someone already there comes back to
    /// the top with the new date.
    pub fn add(&mut self, me: Uuid, id: Uuid, is_group: bool, region: &str, when: SystemTime) {
        if id == me || id.is_nil() || is_group {
            return;
        }
        self.revision = self.revision.wrapping_add(1);
        self.people.retain(|m| m.id != id);
        self.people.insert(
            0,
            Met {
                id,
                when,
                region: region.to_owned(),
            },
        );
    }

    /// Merge saved/server timestamps without making old contacts look newly met.
    pub fn merge(&mut self, me: Uuid, entries: impl IntoIterator<Item = Met>) {
        let mut by_id: std::collections::HashMap<Uuid, Met> = self.people.iter().cloned().map(|m| (m.id, m)).collect();
        let mut changed = false;
        for entry in entries {
            if entry.id.is_nil() || entry.id == me {
                continue;
            }
            if by_id.get(&entry.id).is_none_or(|old| old.when < entry.when) {
                by_id.insert(entry.id, entry);
                changed = true;
            }
        }
        if changed {
            self.people = by_id.into_values().collect();
            self.people.sort_by(|a, b| b.when.cmp(&a.when).then_with(|| a.id.cmp(&b.id)));
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
    /// One read-only discovery per login when the simulator advertises support.
    /// Follow Chat Service's transcript-consent and mute-list gates. Missing
    /// capability, failed discovery or disabled logging leaves local data alone.
    pub(super) fn poll_recent_participants(&mut self) {
        if !self.chat_log.cfg.log_im || !self.mutes.is_loaded() {
            self.recent.server_people.clear();
            return;
        }
        if self.recent.server_request.is_none() && self.main().is_some_and(|r| r.caps.contains_key(aurora_net::persistent_chat::CAP)) {
            let request = Uuid::new_v4();
            self.recent.server_request = Some(request);
            self.groups.out.push(aurora_net::NetCommand::RequestRecentParticipants { request });
        }
        let expired = self.recent.server_deadline.is_some_and(|t| std::time::Instant::now() >= t);
        let mut ready = Vec::new();
        self.recent.server_people.retain(|p| {
            if p.id == self.agent_id || self.groups.is_member(&p.id) || self.mutes.is_muted_id(&p.id) {
                return false;
            }
            let name = self
                .social
                .avatar_names
                .get(&p.id)
                .map(|n| n.user_name())
                .or_else(|| self.social.names.get(&p.id).cloned());
            if let Some(name) = name {
                if !self.mutes.is_muted(&p.id, &name, 0) {
                    ready.push(Met {
                        id: p.id,
                        when: p.when,
                        region: String::new(),
                    });
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

    pub(super) fn on_recent_participants(&mut self, request: Uuid, people: Vec<aurora_net::persistent_chat::Participant>) {
        if self.recent.server_request != Some(request) || !self.chat_log.cfg.log_im {
            return;
        }
        for person in &people {
            if !self.mutes.is_muted_id(&person.id) {
                self.social.want_name(person.id);
            }
        }
        self.recent.server_people = people;
        self.recent.server_deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(30));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_chat_reply_is_login_scoped_and_does_not_open_tabs() {
        use crate::world::{
            World,
            mutes::{LoadState, MuteType},
        };
        use aurora_net::persistent_chat::Participant;
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let (me, a, b, c) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3), Uuid::from_u128(4));
        w.agent_id = me;
        w.mutes.state = LoadState::Loaded;
        w.social.names.insert(a, "Example One".into());
        w.social.names.insert(b, "Example Two".into());
        w.social.names.insert(c, "Example Three".into());
        w.block(b, "Example Two", MuteType::Agent).unwrap();
        w.block(Uuid::nil(), "Example Three", MuteType::ByName).unwrap();
        let now = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(100);
        w.recent.add(me, a, false, "", now);
        let request = Uuid::new_v4();
        w.recent.server_request = Some(request);
        let people = vec![
            Participant {
                id: a,
                when: now - std::time::Duration::from_secs(10),
            },
            Participant { id: b, when: now },
            Participant { id: c, when: now },
        ];
        w.on_recent_participants(Uuid::new_v4(), people.clone());
        assert!(w.recent.server_people.is_empty());
        w.on_recent_participants(request, people);
        w.poll_recent_participants();
        assert_eq!(w.recent.len(), 1);
        assert_eq!(w.recent.list()[0].when, now, "older server activity cannot replace local activity");
        assert!(w.social.ims.is_empty() && w.reopen_ims.is_empty());
        w.chat_log.cfg.log_im = false;
        w.on_recent_participants(request, vec![Participant { id: c, when: now }]);
        assert!(w.recent.server_people.is_empty());
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
