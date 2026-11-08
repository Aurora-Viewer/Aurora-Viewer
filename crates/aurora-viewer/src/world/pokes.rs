//! Pokes (Preferences › Chat › Conversations, on by default; feedback board
//! post #255): a friendly "hey" sent with one click, and poke wars with a
//! counter. Aurora's own: the Legacy Viewer has nothing of the kind.
//!
//! A poke needs no server support: it is the private IM `POKE`, an emote,
//! so a viewer that does not know pokes shows "Name pokes you 👉". This
//! viewer recognises the exact text in a resident's online 1-to-1 IM: the
//! line stays in the conversation and its transcript (the history matches
//! what the sender sees) and a poke card (`ui/poke.rs`) takes the place of
//! the message's toast, one per resident, counting the pokes of a streak.
//! With the option off a poke is an ordinary IM, and no Poke button shows.
//!
//! A poke goes only to a resident known to be online (a friend showing
//! their status, or someone in the scene): to anyone else it would wait as
//! an offline IM. Never to a blocked resident (an IM unblocks them), and
//! under RLVa only where a typed IM may go. After each poke the buttons for
//! that resident rest `COOLDOWN`, which keeps a poke war under the other
//! viewer's anti-spam and the simulator's IM throttle.

use super::World;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The text of a poke, sent and recognised as is: a protocol constant, never
/// translated (every viewer must read it the same).
pub const POKE: &str = "/me pokes you 👉";

/// Rest of the Poke buttons for a resident after a poke to them.
pub const COOLDOWN: Duration = Duration::from_secs(2);

/// Without a poke from the same resident for this long, their count starts
/// again at one.
pub const STREAK: Duration = Duration::from_secs(60);

/// `text` is a poke: exactly `POKE` (a hand-typed "/me pokes you with a
/// stick" stays a message).
pub fn is_poke(text: &str) -> bool {
    text == POKE
}

/// Why a poke cannot go to a resident now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The option is off (no button shows).
    Off,
    /// Ourselves, or no resident.
    NotAResident,
    /// Not known to be online.
    Offline,
    /// Blocked: an IM would unblock them.
    Blocked,
    /// RLVa keeps IMs to them from going.
    Restricted,
    /// A poke to them just went.
    Cooling,
}

impl Refusal {
    /// The Poke button is hidden (rather than greyed out) for this reason.
    pub fn hides(self) -> bool {
        matches!(self, Refusal::Off | Refusal::NotAResident | Refusal::Blocked | Refusal::Restricted)
    }
}

/// A resident's pokes in a row.
#[derive(Debug, Clone, Copy)]
struct Streak {
    count: usize,
    last: Instant,
}

#[derive(Debug)]
pub struct Pokes {
    /// The option (Preferences › Chat › Conversations), set by the app each
    /// frame.
    pub enabled: bool,
    /// When our last poke to each resident went (the cooldown).
    sent: HashMap<Uuid, Instant>,
    /// The streak of each resident poking us.
    received: HashMap<Uuid, Streak>,
}

impl Default for Pokes {
    fn default() -> Self {
        Pokes {
            enabled: true,
            sent: HashMap::new(),
            received: HashMap::new(),
        }
    }
}

impl Pokes {
    /// A poke to `to` went less than `COOLDOWN` ago.
    pub fn cooling(&self, to: &Uuid, now: Instant) -> bool {
        self.sent.get(to).is_some_and(|t| now.saturating_duration_since(*t) < COOLDOWN)
    }

    /// The cooldown left for `to` (zero once over).
    pub fn cooldown_left(&self, to: &Uuid, now: Instant) -> Duration {
        self.sent
            .get(to)
            .map_or(Duration::ZERO, |t| COOLDOWN.saturating_sub(now.saturating_duration_since(*t)))
    }

    fn mark_sent(&mut self, to: Uuid, now: Instant) {
        self.sent.retain(|_, t| now.saturating_duration_since(*t) < COOLDOWN);
        self.sent.insert(to, now);
    }

    /// A poke from `from` arrived: its place in their streak (1 for the
    /// first, or after `STREAK` without one).
    fn count(&mut self, from: Uuid, now: Instant) -> usize {
        self.received.retain(|_, s| now.saturating_duration_since(s.last) < STREAK);
        let s = self.received.entry(from).or_insert(Streak { count: 0, last: now });
        s.count += 1;
        s.last = now;
        s.count
    }

    /// Their card was closed or opened: the next poke starts a new count.
    pub fn forget(&mut self, from: &Uuid) {
        self.received.remove(from);
    }
}

impl World {
    /// Whether we may poke `to` now, and why not.
    pub fn poke_refusal(&self, to: &Uuid, now: Instant) -> Option<Refusal> {
        let status = crate::ui::presence::of(self, to);
        if !self.pokes.enabled {
            Some(Refusal::Off)
        } else if to.is_nil() || *to == self.agent_id || self.is_group_session(to) {
            Some(Refusal::NotAResident)
        } else if self.mutes.is_muted_id(to) {
            Some(Refusal::Blocked)
        } else if !self.rlva_allows_im("sendim", *to) || !self.rlva_can_start_im(*to) {
            Some(Refusal::Restricted)
        } else if matches!(status, crate::ui::presence::Status::Offline | crate::ui::presence::Status::Unknown) {
            Some(Refusal::Offline)
        } else if self.pokes.cooling(to, now) {
            Some(Refusal::Cooling)
        } else {
            None
        }
    }

    /// Poke `to`: our line in their conversation and the IM to send, or
    /// nothing when the poke may not go (`poke_refusal`).
    pub fn poke(&mut self, to: Uuid, now: Instant) -> Option<aurora_net::NetCommand> {
        if let Some(why) = self.poke_refusal(&to, now) {
            log::info!("poke to {to} not sent: {why:?}");
            return None;
        }
        self.pokes.mark_sent(to, now);
        self.own_im(to, POKE);
        Some(aurora_net::NetCommand::SendIm {
            to,
            message: POKE.to_owned(),
        })
    }

    /// A resident's IM `text` received live is a poke this viewer shows as
    /// one: the option on, the exact text.
    pub(super) fn takes_poke(&self, text: &str, offline: bool) -> bool {
        self.pokes.enabled && !offline && is_poke(text)
    }

    /// Show the poke card of `from` (named `name`), the count of their
    /// streak on it: it replaces their card still out. Not in do not
    /// disturb, like a message's toast; the line is in the conversation.
    pub(super) fn notify_poke(&mut self, from: Uuid, name: String) {
        let count = self.pokes.count(from, Instant::now());
        if self.status.dnd {
            return;
        }
        let id = self.notifications.push_poke(from, name.clone(), count);
        self.notifications
            .link_name(id, name, super::notifications::NameTarget::Avatar(from));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_text_is_a_poke() {
        assert!(is_poke(POKE));
        assert!(!is_poke("/me pokes you with a stick"));
        assert!(!is_poke("/me pokes"));
        assert!(!is_poke(" /me pokes you 👉"));
        assert!(!is_poke("/me pokes you 👉 "));
        assert!(!is_poke("/ME POKES YOU 👉"));
    }

    #[test]
    fn the_cooldown_ends_after_its_time() {
        let mut p = Pokes::default();
        let to = Uuid::from_u128(7);
        let t = Instant::now();
        assert!(!p.cooling(&to, t));
        p.mark_sent(to, t);
        assert!(p.cooling(&to, t + Duration::from_millis(500)));
        assert!(!p.cooling(&Uuid::from_u128(8), t), "another resident is free");
        assert!(!p.cooling(&to, t + COOLDOWN));
        assert_eq!(p.cooldown_left(&to, t + COOLDOWN), Duration::ZERO);
    }

    #[test]
    fn a_streak_counts_up_and_starts_again_after_a_quiet_spell_or_a_close() {
        let mut p = Pokes::default();
        let from = Uuid::from_u128(9);
        let t = Instant::now();
        assert_eq!(p.count(from, t), 1);
        assert_eq!(p.count(from, t + Duration::from_secs(5)), 2);
        assert_eq!(p.count(from, t + Duration::from_secs(30)), 3);
        // a quiet minute
        assert_eq!(p.count(from, t + Duration::from_secs(30) + STREAK), 1);
        assert_eq!(p.count(from, t + Duration::from_secs(100)), 2);
        p.forget(&from);
        assert_eq!(p.count(from, t + Duration::from_secs(101)), 1);
        // each resident has their own
        assert_eq!(p.count(Uuid::from_u128(10), t + Duration::from_secs(101)), 1);
    }

    const FRIEND: Uuid = Uuid::from_u128(77);

    fn world() -> World {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = Uuid::from_u128(1);
        world.social.friends.push(crate::world::social::Friend {
            id: FRIEND,
            online: true,
            rights_given: crate::world::social::rights::ONLINE_STATUS,
            rights_has: crate::world::social::rights::ONLINE_STATUS,
        });
        world
    }

    fn im(world: &World, text: &str, offline: bool) -> aurora_net::NetEvent {
        aurora_net::NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: FRIEND,
            from_name: "Poker Resident".into(),
            to_agent_id: world.agent_id,
            dialog: 0,
            session_id: Uuid::from_u128(world.agent_id.as_u128() ^ FRIEND.as_u128()),
            message: text.into(),
            offline,
            from_group: false,
            binary_bucket: vec![0],
            timestamp: 0,
        })
    }

    /// Receive `text` from the friend, as the app then shows it (a message
    /// out of sight): the toasts of the notification list.
    fn receive(world: &mut World, text: &str, offline: bool) {
        world.apply(im(world, text, offline));
        for m in std::mem::take(&mut world.im_messages) {
            world.notify_message(&m, false);
        }
    }

    fn pokes_cards(world: &World) -> Vec<&crate::world::notifications::Notification> {
        use crate::world::notifications::Data;
        world
            .notifications
            .list
            .iter()
            .filter(|n| matches!(n.data, Data::Poke { from } if from == FRIEND))
            .collect()
    }

    fn messages_cards(world: &World) -> usize {
        use crate::world::notifications::Data;
        world
            .notifications
            .list
            .iter()
            .filter(|n| matches!(n.data, Data::Message { .. }))
            .count()
    }

    fn lines(world: &World) -> Vec<String> {
        world
            .social
            .ims
            .iter()
            .find(|s| s.other == FRIEND)
            .map(|s| s.lines.iter().map(|l| l.text.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_poke_keeps_its_line_and_shows_one_counting_card() {
        use crate::world::notifications::poke_body;
        let mut w = world();
        receive(&mut w, POKE, false);
        assert_eq!(lines(&w), vec![POKE.to_owned()], "the line stays in the conversation");
        let cards = pokes_cards(&w);
        assert_eq!(cards.len(), 1);
        assert!(cards[0].transient, "a toast only, never under the bell");
        assert!(cards[0].opens());
        assert!(!cards[0].interactive());
        assert_eq!(cards[0].body, poke_body(1));
        assert_eq!(messages_cards(&w), 0, "no message toast besides");
        receive(&mut w, POKE, false);
        receive(&mut w, POKE, false);
        let cards = pokes_cards(&w);
        assert_eq!(cards.len(), 1, "one card per resident");
        assert_eq!(cards[0].body, poke_body(3));
        assert_eq!(lines(&w).len(), 3);
    }

    #[test]
    fn closing_the_card_starts_a_new_count() {
        use crate::world::notifications::{Response, poke_body};
        let mut w = world();
        receive(&mut w, POKE, false);
        receive(&mut w, POKE, false);
        let id = pokes_cards(&w)[0].id;
        w.respond_notification(id, Response::Dismiss);
        assert!(pokes_cards(&w).is_empty());
        receive(&mut w, POKE, false);
        assert_eq!(pokes_cards(&w)[0].body, poke_body(1));
    }

    #[test]
    fn send_im_on_the_card_opens_the_conversation() {
        let mut w = world();
        receive(&mut w, POKE, false);
        let id = pokes_cards(&w)[0].id;
        assert_eq!(w.open_from_notification(id), Some(FRIEND));
        assert!(pokes_cards(&w).is_empty());
    }

    #[test]
    fn with_pokes_off_a_poke_is_an_ordinary_message() {
        let mut w = world();
        w.pokes.enabled = false;
        receive(&mut w, POKE, false);
        assert!(pokes_cards(&w).is_empty());
        assert_eq!(messages_cards(&w), 1);
        assert_eq!(lines(&w), vec![POKE.to_owned()]);
    }

    #[test]
    fn other_text_and_offline_pokes_are_messages() {
        let mut w = world();
        receive(&mut w, "/me pokes you with a stick", false);
        assert!(pokes_cards(&w).is_empty());
        assert_eq!(messages_cards(&w), 1);
        let mut w = world();
        receive(&mut w, POKE, true);
        assert!(pokes_cards(&w).is_empty(), "a poke kept while we were offline is a line");
    }

    #[test]
    fn no_card_in_do_not_disturb() {
        let mut w = world();
        w.status.dnd = true;
        receive(&mut w, POKE, false);
        assert!(pokes_cards(&w).is_empty());
        // (followed by our busy reply)
        assert_eq!(lines(&w).first().map(String::as_str), Some(POKE), "the line is still there");
    }

    #[test]
    fn a_blocked_resident_pokes_nothing() {
        let mut w = world();
        let _ = w.mutes.add(
            crate::world::mutes::Mute {
                id: FRIEND,
                name: "Poker Resident".into(),
                kind: crate::world::mutes::MuteType::Agent,
                flags: 0,
            },
            0,
        );
        receive(&mut w, POKE, false);
        assert!(pokes_cards(&w).is_empty());
        assert_eq!(w.poke_refusal(&FRIEND, Instant::now()), Some(Refusal::Blocked));
    }

    #[test]
    fn who_may_be_poked() {
        let t = Instant::now();
        let mut w = world();
        assert_eq!(w.poke_refusal(&FRIEND, t), None, "a friend online");
        assert_eq!(w.poke_refusal(&w.agent_id.clone(), t), Some(Refusal::NotAResident));
        assert_eq!(w.poke_refusal(&Uuid::nil(), t), Some(Refusal::NotAResident));
        // a stranger far away: their status is unknown
        assert_eq!(w.poke_refusal(&Uuid::from_u128(500), t), Some(Refusal::Offline));
        w.social.friends[0].online = false;
        assert_eq!(w.poke_refusal(&FRIEND, t), Some(Refusal::Offline));
        w.social.friends[0].online = true;
        w.pokes.enabled = false;
        assert_eq!(w.poke_refusal(&FRIEND, t), Some(Refusal::Off));
    }

    #[test]
    fn a_poke_sends_the_im_writes_our_line_and_cools_down() {
        let t = Instant::now();
        let mut w = world();
        let cmd = w.poke(FRIEND, t);
        assert!(
            matches!(&cmd, Some(aurora_net::NetCommand::SendIm { to, message }) if *to == FRIEND && message == POKE),
            "{cmd:?}"
        );
        assert_eq!(lines(&w), vec![POKE.to_owned()], "our line in their conversation");
        assert_eq!(w.poke_refusal(&FRIEND, t + Duration::from_millis(500)), Some(Refusal::Cooling));
        assert!(
            w.poke(FRIEND, t + Duration::from_millis(500)).is_none(),
            "not twice in the cooldown"
        );
        assert_eq!(lines(&w).len(), 1);
        assert!(w.poke(FRIEND, t + COOLDOWN).is_some(), "again once it is over");
    }

    #[test]
    fn only_some_refusals_hide_the_button() {
        assert!(Refusal::Off.hides());
        assert!(Refusal::Blocked.hides());
        assert!(Refusal::Restricted.hides());
        assert!(!Refusal::Offline.hides(), "greyed out: they may come online");
        assert!(!Refusal::Cooling.hides());
    }
}
