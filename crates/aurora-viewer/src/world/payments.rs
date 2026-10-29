//! L$ a resident paid us directly (TRANS_GIFT, the Pay window's payment) as
//! a card in the conversation with the payer, like their inventory offers'
//! (#1198). The Legacy Viewer says it in a PaymentReceived toast, or in
//! nearby chat with FSPaymentInfoInChat (process_money_balance_reply_extended
//! and reason_from_transaction_type, indra/newview/llviewermessage.cpp,
//! originally LGPL 2.1): same rule for what counts and for the payer's
//! message, which the simulator gives as "Payment" when there is none.
//!
//! Only direct payments: an object paid, a sale and a group's money stay in
//! the money tracker. The card goes into the payer's conversation when one
//! is open; a friend's payment opens it with "Open a conversation for a
//! friend's payment" (Preferences › Chat, on by default). The payment is
//! always written to the payer's transcript, with or without a card, as the
//! offers' are (LLHandlerUtil::logToIMP2P).
//!
//! Our own direct payment to a resident (the Legacy Viewer's PaymentSent,
//! you_paid_ldollars_gift) is the same the other way: its card in the
//! payee's conversation when it is open, never opening one (we just paid
//! them), and its line in their transcript, so that it is still there when
//! the conversation is opened again, as a received payment is (#1198).

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
    /// The resident of the conversation: the payer, or the payee of ours.
    pub other: Uuid,
    /// We paid them.
    pub sent: bool,
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

    fn push(&mut self, other: Uuid, sent: bool, amount: i32, message: String) -> u64 {
        self.next += 1;
        self.list.push_back(Payment {
            id: self.next,
            other,
            sent,
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

/// The card's title: "Jane paid you L$75", or "You paid Jane L$75" for ours.
pub fn title(name: String, sent: bool, amount: i32) -> String {
    if sent {
        aurora_i18n::tr!("world-payments-you-paid", name = name, amount = amount_label(amount))
    } else {
        aurora_i18n::tr!("world-payments-paid-you", name = name, amount = amount_label(amount))
    }
}

/// The sentence of a payment, with the other resident as `name`.
pub fn sentence(name: String, sent: bool, amount: i32, message: &str) -> String {
    match (sent, message.is_empty()) {
        (_, true) => title(name, sent, amount),
        (false, false) => aurora_i18n::tr!(
            "world-payments-paid-you-message",
            name = name,
            amount = amount_label(amount),
            message = message.to_owned()
        ),
        (true, false) => aurora_i18n::tr!(
            "world-payments-you-paid-message",
            name = name,
            amount = amount_label(amount),
            message = message.to_owned()
        ),
    }
}

impl World {
    /// A MoneyBalanceReply naming a transaction: a direct payment between a
    /// resident and us gets its card and its line in their transcript.
    pub fn payment(&mut self, ev: &aurora_net::NetEvent) {
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
        let sent = *source == self.agent_id;
        let other = if sent { *dest } else { *source };
        if !success
            || *kind != GIFT
            || *amount <= 0
            || *source_group
            || *dest_group
            || (!sent && *dest != self.agent_id)
            || other.is_nil()
            || other == self.agent_id
            || self.payments.seen(*id)
            || (!sent && self.mutes.text_muted(&other, &self.social.name_of(&other)))
        {
            return;
        }
        let message = message_of(item).to_owned();
        let text = sentence(super::teleport_invites::agent_link(other), sent, *amount, &message);
        let note = self.notice_line(text.clone());
        self.transcript_only(other, &note);
        let open = self.social.has_session(&other);
        let friend = !sent
            && self.payments.open_for_friends
            && self.social.is_friend(&other)
            && self.rlva_can_start_im(other);
        if !open && !friend {
            return;
        }
        self.social.session_mut(other);
        let payment = self.payments.push(other, sent, *amount, message);
        let line = ChatLine {
            time: SystemTime::now(),
            from: self.social.name_of(&other),
            text,
            kind: ChatKind::Payment(payment),
            source: other,
            origin: Origin::Live,
        };
        self.conversation_card(other, line);
        if sent {
            // our own doing: nothing new to read
            let s = self.session_lines(other);
            s.unread = s.unread.saturating_sub(1);
        }
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
        w.payment(&paid(10, GIFT, JANE, AGENT, 75, "Thanks for the set!"));
        assert!(w.social.ims.is_empty(), "no conversation opened for a stranger");
        w.social.session_mut(JANE);
        w.payment(&paid(11, GIFT, JANE, AGENT, 1200, "Payment"));
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
            w.payment(&paid(10, GIFT, JANE, AGENT, 50, "Rent"));
            assert_eq!(cards(&w, JANE).len(), usize::from(open), "{open}");
        }
    }

    #[test]
    fn only_direct_payments_to_us_once() {
        let mut w = world();
        w.social.session_mut(JANE);
        // an object paid, a sale, a failure: none
        w.payment(&paid(1, 5008, JANE, AGENT, 10, "Tip jar"));
        w.payment(&paid(2, 5000, JANE, AGENT, 10, "Lamp"));
        let mut failed = paid(4, GIFT, JANE, AGENT, 10, "Payment");
        if let NetEvent::MoneyTransaction { success, .. } = &mut failed {
            *success = false;
        }
        w.payment(&failed);
        assert!(cards(&w, JANE).is_empty());
        // the simulator's reply twice: one card
        w.payment(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        w.payment(&paid(5, GIFT, JANE, AGENT, 10, "Payment"));
        assert_eq!(cards(&w, JANE).len(), 1);
    }

    #[test]
    fn the_sentence_names_the_message_when_there_is_one() {
        assert_eq!(message_of("Payment"), "");
        let plain = sentence("Jane".into(), false, 1000, "");
        let with = sentence("Jane".into(), false, 75, "Thanks");
        assert!(plain.contains("Jane") && plain.contains("1"), "{plain}");
        assert!(with.contains("75") && with.ends_with("Thanks"), "{with}");
        let ours = sentence("Jane".into(), true, 75, "Thanks");
        assert!(ours != with && ours.contains("Jane") && ours.ends_with("Thanks"), "{ours}");
    }

    #[test]
    fn our_payment_shows_in_the_open_conversation_without_opening_one() {
        let mut w = world();
        w.social.form_friendship(JANE);
        // no conversation: not opened, even for a friend (we just paid them)
        w.payment(&paid(20, GIFT, AGENT, JANE, 300, "Rent"));
        assert!(w.social.ims.is_empty());
        w.social.session_mut(JANE);
        w.payment(&paid(21, GIFT, AGENT, JANE, 40, "Payment"));
        let got = cards(&w, JANE);
        assert_eq!(got.len(), 1);
        assert!(got[0].sent && got[0].other == JANE && got[0].amount == 40);
        assert_eq!(w.social.ims[0].unread, 0, "our own doing is not unread");
        // to a group, or failed: none
        let mut group = paid(22, GIFT, AGENT, JANE, 10, "Payment");
        if let NetEvent::MoneyTransaction { dest_group, .. } = &mut group {
            *dest_group = true;
        }
        w.payment(&group);
        assert_eq!(cards(&w, JANE).len(), 1);
    }
}
