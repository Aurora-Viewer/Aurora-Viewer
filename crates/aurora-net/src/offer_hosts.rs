//! The circuit each inventory offer came in on, so its answer goes back to
//! the same simulator: LLOfferInfo::mHost, set from the message's sender in
//! process_improved_im (indra/newview/llimprocessing.cpp) and used by
//! LLOfferInfo::send_auto_receive_response and send_decline_response
//! (`msg->sendReliable(mHost)`, indra/newview/llviewermessage.cpp),
//! originally LGPL 2.1.
//!
//! An object in a neighbouring region (a vendor touched while the camera
//! shops there) offers through that region's circuit, and that simulator
//! holds the offer: an IM_TASK_INVENTORY_ACCEPTED sent to the agent's own
//! region names a transaction it does not know, and nothing is delivered.

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use uuid::Uuid;

/// IM_INVENTORY_OFFERED, IM_TASK_INVENTORY_OFFERED, IM_GROUP_NOTICE and
/// IM_GROUP_NOTICE_REQUESTED (llinstantmessage.h): the offers that keep
/// their sender.
const OFFERS: [u8; 4] = [4, 9, 32, 37];

/// Their answers: the offer's dialog + 1 to accept, + 2 to decline
/// (IM_INVENTORY_ACCEPTED / _DECLINED, IM_TASK_INVENTORY_ACCEPTED /
/// _DECLINED, IM_GROUP_NOTICE_INVENTORY_ACCEPTED / _DECLINED).
const ANSWERS: [u8; 6] = [5, 6, 10, 11, 33, 34];

/// Offers left unanswered are forgotten oldest first past this many.
const LIMIT: usize = 256;

#[derive(Default)]
pub(crate) struct OfferHosts {
    hosts: HashMap<Uuid, SocketAddr>,
    order: VecDeque<Uuid>,
}

impl OfferHosts {
    /// An ImprovedInstantMessage received from `from`: an offer keeps the
    /// circuit under its transaction id (the IM's session id, which every
    /// answer carries back). Returns whether it was kept.
    pub(crate) fn received(&mut self, dialog: u8, transaction: Uuid, from: SocketAddr) -> bool {
        if !OFFERS.contains(&dialog) || transaction.is_nil() {
            return false;
        }
        if self.hosts.insert(transaction, from).is_none() {
            self.order.push_back(transaction);
        }
        while self.order.len() > LIMIT {
            if let Some(old) = self.order.pop_front() {
                self.hosts.remove(&old);
            }
        }
        true
    }

    /// The circuit an answer goes to: the one its offer came in on, if it
    /// is known and `alive`, else None (the agent's region). An offer can be
    /// answered more than once (a card and its notification), so the entry
    /// stays until it ages out.
    pub(crate) fn answer_to(&self, dialog: u8, transaction: Uuid, alive: impl Fn(SocketAddr) -> bool) -> Option<SocketAddr> {
        if !ANSWERS.contains(&dialog) {
            return None;
        }
        self.hosts.get(&transaction).copied().filter(|host| alive(*host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "127.0.0.1:13001";
    const NEIGHBOUR: &str = "127.0.0.1:13002";

    fn addr(a: &str) -> SocketAddr {
        a.parse().expect("address")
    }

    #[test]
    fn every_answer_goes_back_to_the_circuit_its_offer_came_in_on() {
        let mut hosts = OfferHosts::default();
        let task = Uuid::from_u128(1);
        let resident = Uuid::from_u128(2);
        let notice = Uuid::from_u128(3);
        hosts.received(9, task, addr(NEIGHBOUR));
        hosts.received(4, resident, addr(MAIN));
        hosts.received(32, notice, addr(NEIGHBOUR));
        for dialog in [10, 11] {
            assert_eq!(hosts.answer_to(dialog, task, |_| true), Some(addr(NEIGHBOUR)));
        }
        for dialog in [5, 6] {
            assert_eq!(hosts.answer_to(dialog, resident, |_| true), Some(addr(MAIN)));
        }
        for dialog in [33, 34] {
            assert_eq!(hosts.answer_to(dialog, notice, |_| true), Some(addr(NEIGHBOUR)));
        }
        // answered twice (card and notification): still the same circuit
        assert_eq!(hosts.answer_to(10, task, |_| true), Some(addr(NEIGHBOUR)));
    }

    #[test]
    fn other_dialogs_unknown_offers_and_closed_circuits_use_the_agents_region() {
        let mut hosts = OfferHosts::default();
        let task = Uuid::from_u128(1);
        hosts.received(9, task, addr(NEIGHBOUR));
        // a lure decline or a plain IM with the same id is not an answer
        assert_eq!(hosts.answer_to(24, task, |_| true), None);
        assert_eq!(hosts.answer_to(0, task, |_| true), None);
        assert_eq!(hosts.answer_to(10, Uuid::from_u128(9), |_| true), None);
        // the neighbour's circuit closed since (teleport, region out of range)
        assert_eq!(hosts.answer_to(10, task, |host| host != addr(NEIGHBOUR)), None);
        // neither a teleport offer nor an offer without a transaction is kept
        hosts.received(22, Uuid::from_u128(5), addr(NEIGHBOUR));
        hosts.received(9, Uuid::nil(), addr(NEIGHBOUR));
        assert_eq!(hosts.hosts.len(), 1);
    }

    #[test]
    fn unanswered_offers_age_out_oldest_first() {
        let mut hosts = OfferHosts::default();
        for n in 0..=LIMIT as u128 {
            hosts.received(9, Uuid::from_u128(n + 1), addr(NEIGHBOUR));
        }
        assert_eq!(hosts.hosts.len(), LIMIT);
        assert_eq!(hosts.answer_to(10, Uuid::from_u128(1), |_| true), None);
        assert_eq!(
            hosts.answer_to(10, Uuid::from_u128(LIMIT as u128 + 1), |_| true),
            Some(addr(NEIGHBOUR))
        );
        // the same transaction offered again takes one place, not two
        hosts.received(9, Uuid::from_u128(LIMIT as u128 + 1), addr(MAIN));
        assert_eq!(hosts.order.len(), LIMIT);
        assert_eq!(hosts.answer_to(10, Uuid::from_u128(LIMIT as u128 + 1), |_| true), Some(addr(MAIN)));
    }
}
