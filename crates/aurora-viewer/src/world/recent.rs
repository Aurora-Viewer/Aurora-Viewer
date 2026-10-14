//! The residents we dealt with lately, for People › Recent: port of
//! LLRecentPeople (indra/newview/llrecentpeople.cpp, originally LGPL 2.1).
//! Someone joins it when they talk to us in the nearby chat or by IM, or
//! when we open a conversation with them (LLAvatarActions::startIM), with the
//! date and, Aurora's addition for the list's second line, the region we
//! were in. Kept for the session, as Firestorm does.

use std::time::SystemTime;
use uuid::Uuid;

/// One resident of the list.
#[derive(Debug, Clone, PartialEq)]
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
}

impl RecentPeople {
    /// LLRecentPeople::add: never ourselves nor a group (a group's chat
    /// session carries the group's id); someone already there comes back to
    /// the top with the new date.
    pub fn add(&mut self, me: Uuid, id: Uuid, is_group: bool, region: &str, when: SystemTime) {
        if id == me || id.is_nil() || is_group {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
