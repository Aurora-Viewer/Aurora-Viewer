//! Rights given to friends, one grant in flight per friend.
//!
//! Each change sends GrantUserRights with the friend's full rights, and the
//! server answers with ChangeUserRights carrying what it applied. Two grants
//! in flight for one friend race: the first answer puts the friend back to
//! the first state, a click made then starts from it and drops the second
//! change, and a reliable resend may reach the server after a newer grant.
//! FSFloaterContacts::applyRightsToFriends (indra/newview/
//! fsfloatercontacts.cpp) disables the friend's row "until response from
//! server to avoid race conditions". Here the boxes stay usable instead
//! (visible difference, better for the user): while a grant waits, the
//! newest choice is kept and shown, answers that are not the latest no
//! longer overwrite it, and the newest choice leaves once the server has
//! answered. The Legacy Viewer has no timeout; one covers an answer that
//! never comes, so a lost reply cannot hold the friend's changes forever.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// How long a grant waits for the server's answer before the next one may
/// leave.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy)]
struct Pending {
    /// The rights of the grant in flight.
    sent: i32,
    at: Instant,
    /// The user's newest choice, made while `sent` waits.
    wanted: Option<i32>,
    /// The last answer that did not match `sent`: what the server holds,
    /// shown if the grant's own answer never comes.
    other_answer: Option<i32>,
}

/// What the world does with a ChangeUserRights answer about a friend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Show these rights (nothing of ours was waiting, or it is settled).
    Show(i32),
    /// Keep showing our own choice: the answer is not the latest grant.
    Keep,
    /// The grant was answered and a newer choice waits: send it.
    Send(i32),
}

#[derive(Debug, Default)]
pub struct RightsSync {
    pending: HashMap<Uuid, Pending>,
}

impl RightsSync {
    /// A grant about to leave: true when it may go now (it is then the
    /// friend's grant in flight), false when it waits behind the one in
    /// flight, as the friend's newest choice.
    pub fn send(&mut self, friend: Uuid, rights: i32, now: Instant) -> bool {
        match self.pending.get_mut(&friend) {
            Some(p) if now.duration_since(p.at) < ANSWER_TIMEOUT => {
                p.wanted = (rights != p.sent).then_some(rights);
                false
            }
            _ => {
                self.pending.insert(
                    friend,
                    Pending {
                        sent: rights,
                        at: now,
                        wanted: None,
                        other_answer: None,
                    },
                );
                true
            }
        }
    }

    /// The server's answer about the rights we give `friend`
    /// (LLAvatarTracker::processChange, our own agent id).
    pub fn answered(&mut self, friend: Uuid, rights: i32) -> Answer {
        let Some(p) = self.pending.get_mut(&friend) else {
            return Answer::Show(rights);
        };
        if rights != p.sent {
            p.other_answer = Some(rights);
            return Answer::Keep;
        }
        let wanted = p.wanted;
        self.pending.remove(&friend);
        match wanted {
            Some(w) => Answer::Send(w),
            None => Answer::Show(rights),
        }
    }

    /// Grants whose answer never came: the friend's newest choice to send
    /// (`Answer::Send`), or what the server last said to show
    /// (`Answer::Show`).
    pub fn expired(&mut self, now: Instant) -> Vec<(Uuid, Answer)> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.pending.retain(|id, p| {
            if now.duration_since(p.at) < ANSWER_TIMEOUT {
                return true;
            }
            log::warn!("friend rights: no answer to the grant for {id} after {ANSWER_TIMEOUT:?}");
            match (p.wanted, p.other_answer) {
                (Some(w), _) => out.push((*id, Answer::Send(w))),
                (None, Some(a)) => out.push((*id, Answer::Show(a))),
                (None, None) => {}
            }
            false
        });
        out
    }

    /// The friendship ended: nothing waits for them any more.
    pub fn forget(&mut self, friend: &Uuid) {
        self.pending.remove(friend);
    }

    /// A grant waits for its answer.
    #[cfg(test)]
    pub fn waiting(&self, friend: &Uuid) -> bool {
        self.pending.contains_key(friend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn first_grant_leaves_and_its_answer_is_shown() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 3, t));
        assert_eq!(s.answered(id(1), 3), Answer::Show(3));
        assert!(!s.waiting(&id(1)));
        // nothing of ours waits: a change from the server is shown as is
        assert_eq!(s.answered(id(1), 1), Answer::Show(1));
    }

    #[test]
    fn quick_clicks_keep_the_newest_choice_and_send_it_after_the_answer() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(!s.send(id(1), 3, t), "a second grant waits behind the first");
        assert!(!s.send(id(1), 7, t), "the newest choice replaces the waiting one");
        assert_eq!(s.answered(id(1), 1), Answer::Send(7));
        assert!(!s.waiting(&id(1)), "the app records the newest grant when it leaves");
        assert!(s.send(id(1), 7, t));
        assert_eq!(s.answered(id(1), 7), Answer::Show(7));
    }

    #[test]
    fn choice_back_to_the_grant_in_flight_sends_nothing_more() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(!s.send(id(1), 3, t));
        assert!(!s.send(id(1), 1, t));
        assert_eq!(s.answered(id(1), 1), Answer::Show(1));
    }

    #[test]
    fn an_answer_that_is_not_the_grant_in_flight_never_overwrites_the_choice() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 3, t));
        assert_eq!(s.answered(id(1), 1), Answer::Keep);
        assert!(s.waiting(&id(1)));
        assert_eq!(s.answered(id(1), 3), Answer::Show(3));
    }

    #[test]
    fn friends_do_not_wait_for_each_other() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(s.send(id(2), 1, t));
        assert!(s.send(id(3), 0, t));
        assert_eq!(s.answered(id(2), 1), Answer::Show(1));
        assert!(s.waiting(&id(1)) && s.waiting(&id(3)));
    }

    #[test]
    fn a_lost_answer_lets_the_newest_choice_leave_after_the_timeout() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(!s.send(id(1), 3, t));
        assert!(s.expired(t + ANSWER_TIMEOUT / 2).is_empty());
        assert_eq!(s.expired(t + ANSWER_TIMEOUT), vec![(id(1), Answer::Send(3))]);
        assert!(!s.waiting(&id(1)));
    }

    #[test]
    fn a_lost_answer_with_nothing_newer_shows_what_the_server_said() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 3, t));
        assert_eq!(s.answered(id(1), 2), Answer::Keep);
        assert_eq!(s.expired(t + ANSWER_TIMEOUT), vec![(id(1), Answer::Show(2))]);
        // a lone lost answer leaves the choice shown as it is
        assert!(s.send(id(2), 1, t));
        assert!(s.expired(t + ANSWER_TIMEOUT).is_empty());
    }

    #[test]
    fn a_grant_after_the_timeout_leaves_at_once() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(s.send(id(1), 3, t + ANSWER_TIMEOUT));
        assert_eq!(s.answered(id(1), 3), Answer::Show(3));
    }

    #[test]
    fn an_ended_friendship_forgets_its_grant() {
        let mut s = RightsSync::default();
        let t = Instant::now();
        assert!(s.send(id(1), 1, t));
        assert!(!s.send(id(1), 3, t));
        s.forget(&id(1));
        assert!(s.expired(t + ANSWER_TIMEOUT).is_empty());
        assert!(s.send(id(1), 0, t));
    }

    /// The world end to end, as the app's send path drives it: the click
    /// (`profile_command`), the gate (`send`), the server's answers
    /// (`World::apply`) and the timeout pass.
    #[test]
    fn late_answers_never_undo_a_quick_second_click() {
        use crate::world::World;
        use crate::world::social::Friend;
        use aurora_net::{NetCommand, NetEvent};
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let f = id(9);
        w.social.friends = vec![Friend {
            id: f,
            online: true,
            rights_given: 3,
            rights_has: 0,
        }];
        let t = Instant::now();
        let click = |w: &mut World, rights: i32| {
            w.profile_command(&NetCommand::GrantUserRights { friend: f, rights });
            w.social.rights_sync.send(f, rights, t)
        };
        let answer = |w: &mut World, rights: i32| {
            w.apply(NetEvent::UserRights {
                agent: w.agent_id,
                rights: vec![(f, rights)],
            });
        };
        let given = |w: &World| w.social.friends[0].rights_given;
        let sent = |w: &mut World| -> Vec<i32> {
            w.take_social_commands()
                .into_iter()
                .filter_map(|c| match c {
                    NetCommand::GrantUserRights { rights, .. } => Some(rights),
                    _ => None,
                })
                .collect()
        };
        // map off, then online off, before the server answers
        assert!(click(&mut w, 1));
        assert!(!click(&mut w, 0));
        assert_eq!(given(&w), 0);
        // the first answer no longer puts the map box back...
        answer(&mut w, 1);
        assert_eq!(given(&w), 0, "the newest choice stays shown");
        // ...and the newest choice leaves after it
        assert_eq!(sent(&mut w), vec![0]);
        assert!(w.social.rights_sync.send(f, 0, t));
        answer(&mut w, 0);
        assert_eq!(given(&w), 0);
        assert!(sent(&mut w).is_empty());
        // a change the server makes with nothing of ours waiting is shown
        answer(&mut w, 1);
        assert_eq!(given(&w), 1);
    }
}
