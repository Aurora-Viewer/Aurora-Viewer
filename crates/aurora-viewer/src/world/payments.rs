//! L$ a resident paid us directly (TRANS_GIFT, the Pay window's payment) as
//! a card in the conversation with the payer, like their inventory offers'
//! (#1198). The Legacy Viewer says it in a PaymentReceived toast, or in
//! nearby chat with FSPaymentInfoInChat (process_money_balance_reply_extended
//! and reason_from_transaction_type, indra/newview/llviewermessage.cpp,
//! originally LGPL 2.1): same rule for what counts and for the payer's
//! message, which the simulator gives as "Payment" when there is none.
//!
//! Only direct payments: an object paid, a sale, a group's money and our
//! own payments stay in the money tracker. The card goes into the payer's
//! conversation when one is open; a friend's payment opens it with
//! "Open a conversation for a friend's payment" (Preferences › Chat, on by
//! default). The payment is always written to the payer's transcript, with
//! or without a card, as the offers' are (LLHandlerUtil::logToIMP2P).

use super::{ChatKind, ChatLine, Origin, World};
use std::collections::VecDeque;
use std::time::SystemTime;
use uuid::Uuid;

/// TRANS_GIFT (lltransactiontypes.h): L$ given to a resident.
const GIFT: i32 = 5001;

/// Payments kept for the conversation cards; an older card shows its text.
const MAX_PAYMENTS: usize = 100;

/// Transaction ids remembered: the simulator may send a reply twice
/// (process_money_balance_reply's MAX_LOOKBACK).
const MAX_RECENT: usize = 30;

#[derive(Debug, Clone)]
pub struct Payment {
    pub id: u64,
    pub from: Uuid,
    pub amount: i32,
    /// The payer's message, empty when they wrote none.
    pub message: String,
}

#[derive(Default)]
pub struct Payments {
    list: VecDeque<Payment>,
    next: u64,
    recent: VecDeque<Uuid>,
    /// A friend's payment opens their conversation, from the preferences.
    pub open_for_friends: bool,
}

impl Payments {
    pub fn get(&self, id: u64) -> Option<&Payment> {
        self.list.iter().find(|v| v.id == id)
    }

    fn push(&mut self, from: Uuid, amount: i32, message: String) -> u64 {
        self.next += 1;
        self.list.push_back(Payment {
            id: self.next,
            from,
            amount,
            message,
        });
        if self.list.len() > MAX_PAYMENTS {
            self.list.pop_front();
        }
        self.next
    }

    /// Whether this transaction was seen already, remembering it.
    fn seen(&mut self, transaction: Uuid) -> bool {
        if transaction.is_nil() {
            return false;
        }
        if self.recent.contains(&transaction) {
            return true;
        }
        if self.recent.len() >= MAX_RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back(transaction);
        false
    }
}

/// reason_from_transaction_type for TRANS_GIFT: "Payment" is the simulator's
/// word for no message.
fn message_of(item: &str) -> &str {
    if item == "Payment" { "" } else { item.trim() }
}

/// "L$1,000" as the amount reads in the card and the transcript.
pub fn amount_label(amount: i32) -> String {
    aurora_i18n::integer(i64::from(amount.max(0)))
}

/// The sentence of a payment, with the payer as `name`.
pub fn sentence(name: String, amount: i32, message: &str) -> String {
    if message.is_empty() {
        aurora_i18n::tr!("world-payments-paid-you", name = name, amount = amount_label(amount))
    } else {
        aurora_i18n::tr!(
            "world-payments-paid-you-message",
            name = name,
            amount = amount_label(amount),
            message = message.to_owned()
        )
    }
}

impl World {
    /// A MoneyBalanceReply naming a transaction: a resident's direct payment
    /// to us gets its card and its line in their transcript.
    pub fn payment_received(&mut self, ev: &aurora_net::NetEvent) {
        let aurora_net::NetEvent::MoneyTransaction {
            id,
            success,
            kind,
            source,
            source_group,
            dest,
            dest_group,
            amount,
            item,
            ..
        } = ev
        else {
            return;
        };
        let from = *source;
        if !success
            || *kind != GIFT
            || *amount <= 0
            || *source_group
            || *dest_group
            || *dest != self.agent_id
            || from.is_nil()
            || from == self.agent_id
            || self.payments.seen(*id)
            || self.mutes.text_muted(&from, &self.social.name_of(&from))
        {
            return;
        }
        let message = message_of(item).to_owned();
        let text = sentence(super::teleport_invites::agent_link(from), *amount, &message);
        let note = self.notice_line(text.clone());
        self.transcript_only(from, &note);
        let open = self.social.has_session(&from);
        let friend = self.payments.open_for_friends && self.social.is_friend(&from) && self.rlva_can_start_im(from);
        if !open && !friend {
            return;
        }
        self.social.session_mut(from);
        let payment = self.payments.push(from, *amount, message);
        let line = ChatLine {
            time: SystemTime::now(),
            from: self.social.name_of(&from),
            text,
            kind: ChatKind::Payment(payment),
            source: from,
            origin: Origin::Live,
        };
        self.conversation_card(from, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::NetEvent;

    const AGENT: Uuid = Uuid::from_u128(1);
    const JANE: Uuid = Uuid::from_u128(2);

    fn world() -> World {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        w.agent_id = AGENT;
        w.payments.open_for_friends = true;
        w
    }

    fn paid(id: u128, kind: i32, source: Uuid, dest: Uuid, amount: i32, item: &str) -> NetEvent {
        NetEvent::MoneyTransaction {
            id: Uuid::from_u128(id),
            success: true,
            kind,
            source,
            source_group: false,
            dest,
            dest_group: false,
            amount,
            item: item.into(),
            description: String::new(),
        }
    }

    fn cards(w: &World, with: Uuid) -> Vec<Payment> {
        w.social
            .ims
            .iter()
            .filter(|s| s.other == with)
            .flat_map(|s| s.lines.iter())
            .filter_map(|l| match l.kind {
                ChatKind::Payment(id) => w.payments.get(id).cloned(),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_payment_shows_in_the_open_conversation_only() {
        let mut w = world();
        w.payment_received(&paid(10, GIFT, JANE, AGENT, 75, "Thanks for the set!"));
        assert!(w.social.ims.is_empty(), "no conversation opened for a stranger");
        w.social.session_mut(JANE);
        w.payment_received(&paid(11, GIFT, JANE, AGENT, 1200, "Payment"));
        let got = cards(&w, JANE);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].amount, got[0].message.as_str()), (1200, ""));
        assert_eq!(w.social.ims[0].unread, 1);
    }

    #[test]
    fn a_friend_s_payment_opens_the_conversation_when_asked() {
        for open in [true, false] {
            let mut w = world();
            w.payments.open_for_friends = open;
            w.social.form_friendship(JANE);
            w.payment_received(&paid(10, GIFT, JANE, AGENT, 50, "Rent"));
            assert_eq!(cards(&w, JANE).len(), usize::from(open), "{open}");
        }
    }

    #[test]
    fn only_direct_payments_to_us_once() {
        let mut w = world();
        w.social.session_mut(JANE);
        // an object paid, a sale, our own payment, a failure: none
        w.payment_received(&paid(1, 5008, JANE, AGENT, 10, "Tip jar"));
        w.payment_received(&paid(2, 5000, JANE, AGENT, 10, "Lamp"));
        w.payment_received(&paid(3, GIFT, AGENT, JANE, 10, "Payment"));
        let mut failed = paid(4, GIFT, JANE, AGENT, 10, "Payment");
        if let NetEvent::MoneyTransaction { success, .. } = &mut failed {
            *success = false;
        }
        w.payment_received(&failed);
        assert!(cards(&w, JANE).is_empty());
        // the simulator's reply twice: one card
        w.payment_received(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        w.payment_received(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        assert_eq!(cards(&w, JANE).len(), 1);
    }

    #[test]
    fn the_sentence_names_the_message_when_there_is_one() {
        assert_eq!(message_of("Payment"), "");
        let plain = sentence("Jane".into(), 1000, "");
        let with = sentence("Jane".into(), 75, "Thanks");
        assert!(plain.contains("Jane") && plain.contains("1"), "{plain}");
        assert!(with.contains("75") && with.ends_with("Thanks"), "{with}");
    }
}
